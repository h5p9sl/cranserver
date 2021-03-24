#![feature(map_into_keys_values)]
use log::*;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};

use std::error::Error;
use std::net::{Ipv4Addr, SocketAddr};

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

type SocketList = Arc<Mutex<std::collections::HashMap<packets::ClientInfo, TcpStream>>>;
type PacketChannel = (packets::ClientInfo, Vec<u8>);

/// Accept and handle new connections from `ip`
async fn handle(
    client: packets::ClientInfo,
    tx: mpsc::Sender<PacketChannel>,
    sockets: SocketList,
) -> Result<(), Box<dyn Error>> {
    let client_type = client.client_type;
    let ip = client.address;
    info!("{:?} connection {} accepted.", client_type, ip);
    tokio::spawn(async move {
        loop {
            // wait until socket is readable
            let mut sockets = sockets.lock().await;
            let s = sockets.get_mut(&client);
            if s.is_none() {
                break;
            }

            // read packets into buf
            let mut buf = Vec::new();
            match s.unwrap().try_read_buf(&mut buf) {
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    continue;
                }
                Err(e) => {
                    return Err(e);
                }
                Ok(0) => {
                    break;
                }
                Ok(_n) => {
                    let client_info = packets::ClientInfo::new(ip, client_type);
                    if let Err(e) = tx.send((client_info, buf.clone())).await {
                        error!("Reciever dropped ({})", e);
                        break;
                    }
                }
            }
        }
        info!("Connection with {} terminated.", &client.address);
        sockets.lock().await.remove(&client);
        Ok(())
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
            let mut buf = Vec::new();

            debug!("Sending packet...");
            stream.write_all(&packet).await?;
            info!("Sent packet!");

            stream.readable().await?;
            debug!("Awaiting packet...");
            stream.read_buf(&mut buf).await?;
            info!("Recieved packet of length {}", buf.len());

            let packet = packets::process(&*buf).await;
            if let Ok(packet) = packet {
                match packet.packet_type {
                    packets::PacketType::Status => {
                        let inner: StatusPacket = serializer.deserialize(&packet.data).unwrap();
                        info!("Status retrieved: {:#?}", inner);
                    }
                    _ => {}
                }
            } else {
                error!("Packet failed to process: {:?}", packet.err());
            }

            info!("Closing stream...");
        }
        ClientType::Controller => unimplemented!(),
    }
    Ok(())
}

async fn accept_and_handle_connections(
    client_type: packets::ClientType,
    listen_addr: SocketAddr,
    tx: mpsc::Sender<PacketChannel>,
    sockets: SocketList,
) {
    let listener = TcpListener::bind(listen_addr).await.unwrap();
    debug!(
        "Listening for new {:?} connections on port {}",
        client_type,
        listen_addr.port(),
    );

    loop {
        match listener.accept().await {
            Ok((s, ip)) => {
                // Clone references
                let channel = tx.clone();
                let sockets = sockets.clone();

                tokio::spawn(async move {
                    // Add this socket to the socketlist and begin recieving packets
                    let info = packets::ClientInfo::new(ip, client_type);
                    sockets.lock().await.insert(info, s);
                    handle(info, channel.clone(), sockets.clone())
                        .await
                        .unwrap();
                });
            }
            Err(e) => error!("Error accepting socket: {:?}", e),
        }
    }
}

#[tokio::main]
async fn main() -> tokio::io::Result<()> {
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

    // Creates a new tokio task for the specified socket address
    let new_socket_task = |client_type, listen_addr| {
        let tx = tx.clone();
        let sockets = sockets.clone();
        tokio::spawn(async move {
            accept_and_handle_connections(client_type, listen_addr, tx, sockets).await;
        })
    };
    let t_clients = new_socket_task(packets::ClientType::Client, client_addr);
    let t_ctrl = new_socket_task(packets::ClientType::Controller, ctrl_addr);

    // Packet processing & response task
    // EXPECTED BEHAVIOUR:
    //      aproximately 1 thread per queued packet
    //      deserialize packets, pass inner data down to a function
    //      return, serialize & send response
    let t_commands = tokio::spawn(async move {
        while let Some((client_info, packet)) = rx.recv().await {
            debug!("Packet recieved through channel: {} bytes", packet.len());
            if packet.is_empty() {
                error!("Empty packet recieved: closing socket");
                sockets.lock().await.remove(&client_info);
                continue;
            }

            // dereferenced borrow of `packet`, slightly unusual!
            debug!("Processing packet (1/3)");
            let packet = packets::process(&*packet).await.unwrap();

            use bincode::Options;
            let serializer = serializer!();

            // TODO (later) move this code into packets::process()
            use packets::{CranPacket, PacketType, StatusPacket};
            let response: Option<CranPacket> = match packet.packet_type {
                PacketType::Status => {
                    debug!("Status packet recieved");
                    dbg!(sockets.lock().await);
                    Some(CranPacket::new(
                        PacketType::Status,
                        serializer
                            .serialize(&StatusPacket {
                                connected_clients: sockets.lock().await.keys().copied().collect(),
                            })
                            .unwrap(),
                    ))
                }
                _ => None,
            };

            if let Some(response) = response {
                use bincode::Options;
                let packet = serializer!().serialize(&response).unwrap();

                debug!("Awaiting socket-list lock... (2/3)");
                let mut sockets = sockets.lock().await;
                if let Some(stream) = sockets.get_mut(&client_info) {
                debug!(
                    "Sending response packet to {}... (3/3)",
                    &client_info.address
                );
                let _ = packets::process(&*packet).await;
                stream.writable().await.unwrap();
                stream.write_all(&packet).await.unwrap();
                } else {
                    error!("Failed to send response: socket no longer exists.");
                }
            }
        }
    });

    info!("Initialization done. Ready!");
    let result = tokio::try_join!(t_clients, t_ctrl, t_commands,);
    result.unwrap();
    Ok(())
}
