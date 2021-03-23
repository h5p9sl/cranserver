use log::*;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};

use std::error::Error;
use std::net::{Ipv4Addr, SocketAddr};

#[macro_use]
use serde::{self, Deserialize, Serialize};

mod config;
#[macro_use]
mod packets;

fn init_logging() {
    env_logger::builder()
        .filter_level(LevelFilter::Debug)
        .format_timestamp_millis()
        .init();
    debug!("Logger initialized");
}

type SocketList = Arc<Mutex<std::collections::HashMap<SocketAddr, TcpStream>>>;
type PacketChannel = (packets::ClientInfo, Vec<u8>);

/// Accept and handle new connections from `ip`
async fn handle(
    client_type: packets::ClientType,
    tx: mpsc::Sender<PacketChannel>,
    ip: std::net::SocketAddr,
    sockets: SocketList,
) -> Result<(), Box<dyn Error>> {
    info!("{:?} connection {} accepted.", client_type, ip);
    tokio::spawn(async move {
        // wait until socket is readable
        let mut sockets = sockets.lock().await;
        let s = sockets.get_mut(&ip).unwrap();
        s.readable().await.unwrap();

        // read packets into buf
        let mut buf = Vec::new();
        while s.read_buf(&mut buf).await.unwrap_or_else(|e| {
            error!("Socket with {} read failure: {}", ip, e);
            0
        }) > 0
        {
            let client_info = packets::ClientInfo::new(ip.into(), client_type);
            if let Err(e) = tx.send((client_info, buf.clone())).await {
                error!("Reciever dropped ({})", e);
                break;
            }
        }
        info!("Connection with {} terminated.", ip);
    });
    Ok(())
}

async fn act_as_client(of_type: packets::ClientType) -> Result<(), Box<dyn Error>> {
    use bincode::Options;
    use packets::*;
    let serializer = bincode::options().with_big_endian().with_fixint_encoding();
    let mut stream = TcpStream::connect("127.0.0.1:8080").await?;

    match of_type {
        ClientType::Client => {
            let packet = CranPacket::new(PacketType::Status, []);
            let packet = serializer.serialize(&packet).unwrap();

            stream.write_all(&packet).await?;

            tokio::time::sleep(std::time::Duration::from_secs_f32(1.0)).await;
        }
        ClientType::Controller => unimplemented!(),
    }
    Ok(())
}

#[tokio::main]
async fn main() -> tokio::io::Result<()> {
    // 1. Packet formats
    // --> 2. Commands / Operations
    // 3. Command executor / Cran Processor

    init_logging();

    if std::env::args().rfind(|s| s == "-client").is_some() {
        act_as_client(packets::ClientType::Client).await.unwrap();
        return Ok(());
    }

    let cfg = config::get_config()
        .unwrap_or_else(|e| {
            warn!("Failed to get config. Using default settings. ({})", e);
            Some(Default::default())
        })
        .expect("No configuration found and no default fallback.");

    let client_addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), cfg.client_port);
    let ctrl_addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), cfg.control_port);

    let sockets: SocketList = Default::default();

    let (tx, mut rx) = mpsc::channel::<PacketChannel>(8);

    let accept_and_handle_connections = |client_type, listen_addr| {
        let channel = tx.clone();
        let sockets = sockets.clone();
        tokio::spawn(async move {
            let listener = TcpListener::bind(listen_addr).await.unwrap();
            debug!(
                "Listening for new {:?} connections on port {}",
                client_type,
                listener.local_addr().unwrap().port()
            );
            while let Ok((s, ip)) = listener.accept().await {
                sockets.lock().await.insert(ip, s);
                handle(client_type, channel.clone(), ip, sockets.clone())
                    .await
                    .unwrap();
            }
        })
    };
    let t_clients = accept_and_handle_connections(packets::ClientType::Client, client_addr);
    let t_ctrl = accept_and_handle_connections(packets::ClientType::Controller, ctrl_addr);

    // Packet processing & response thread
    // EXPECTED BEHAVIOUR:
    //      aproximately 1 thread per queued packet
    //      deserialize packets, pass inner data down to a function
    //      return, serialize & send response
    let t_commands = tokio::spawn(async move {
        while let Some((client_info, packet)) = rx.recv().await {
            let ci = client_info;

            // dereferenced borrow of `packet`, slightly unusual!
            let packet = packets::process(ci.client_type, &*packet).await;
            let packet = {
                if let Err(e) = &packet {
                    error!("Failed to process packet: {}", e);
                    debug!("{:#?}", packet);
                    continue;
                }
                packet.unwrap()
            };

            use packets::{CranPacket, PacketType, StatusPacket};
            let response: Option<CranPacket> = match packet.packet_type {
                PacketType::Status => {
                    // TODO Put data in response packet
                    Some(CranPacket::new(
                        PacketType::Status,
                        Vec::new(), /*
                                    StatusPacket {
                                    }.into()
                                    */
                    ))
                }
                _ => None,
            };

            if let Some(response) = response {
                use bincode::Options;
                let packet = serializer!().serialize(&response).unwrap();
                // TODO send response to client
            }
        }
    });

    info!("Initialization done. Ready!");
    let result = tokio::try_join!(t_clients, t_ctrl, t_commands,);
    result.unwrap();
    Ok(())
}
