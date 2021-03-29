use log::*;
use bytes::BytesMut;
use futures::{SinkExt, StreamExt, TryStreamExt};
use std::error::Error;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tokio::net::{
    tcp::{OwnedReadHalf, OwnedWriteHalf},
    TcpListener, TcpStream,
};
use tokio::sync::{mpsc, Mutex};
use tokio_util::codec::{Framed, FramedRead, FramedWrite, LengthDelimitedCodec};

mod config;
mod packets;
use packets::{CranPacket, PacketType, StatusPacket};

type SocketList = Arc<Mutex<std::collections::HashMap<packets::ClientInfo, OwnedWriteHalf>>>;
type PacketChannel = (packets::ClientInfo, BytesMut);

fn init_logging() {
    env_logger::builder()
        .filter_level(LevelFilter::Debug)
        .format_timestamp_millis()
        .init();
    debug!("Logger initialized");
}

async fn act_as_client(of_type: packets::ClientType) -> Result<(), Box<dyn Error>> {
    use bincode::Options;
    use packets::*;
    let serializer = bincode::options().with_big_endian().with_fixint_encoding();
    let stream = TcpStream::connect("127.0.0.1:8080").await?;

    match of_type {
        ClientType::Client => {
            let packet = CranPacket::new(PacketType::Status, []);
            let packet = serializer.serialize(&packet).unwrap().into();
            let mut stream = Framed::new(stream, LengthDelimitedCodec::new());

            stream.send(packet).await?;
            info!("Sent packet!");

            let buf: BytesMut = stream.next().await.unwrap()?;
            info!("Recieved packet of length {}", buf.len());

            let packet = packets::process(&buf).await;
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
                    let (s_read, s_write) = s.into_split();
                    sockets.lock().await.insert(info, s_write);
                    handle(info, channel.clone(), s_read).await.unwrap();
                });
            }
            Err(e) => error!("Error accepting socket: {:?}", e),
        }
    }
}

async fn handle(
    client: packets::ClientInfo,
    tx: mpsc::Sender<PacketChannel>,
    s: OwnedReadHalf,
) -> Result<(), Box<dyn Error>> {
    let client_type = client.client_type;
    let ip = client.address;
    let client_info = packets::ClientInfo::new(ip, client_type);
    let mut stream = FramedRead::new(s, LengthDelimitedCodec::new());
    info!("{:?} connection {} accepted.", client_type, ip);
    tokio::spawn(async move {
        loop {
            let buf = stream.try_next().await;
            match buf {
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    continue;
                }
                Err(e) => {
                    return Err(e);
                }
                Ok(None) => {
                    break;
                }
                Ok(Some(bytes)) => {
                    let client_info = packets::ClientInfo::new(ip, client_type);
                    if let Err(e) = tx.send((client_info, bytes.clone())).await {
                        error!("Reciever dropped ({})", e);
                        break;
                    }
                }
            }
        }
        info!("Connection with {} terminated.", &client.address);
        // Send an empty message through the channel to signal for socket closure
        if let Err(e) = tx.send((client_info, BytesMut::new())).await {
            error!("Reciever dropped ({})", e);
        }
        Ok(())
    });
    Ok(())
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
    //      1 green thread per queued packet
    //      deserialize packets, pass inner data down to a function
    //      return, serialize & send a response packet
    let t_commands = tokio::spawn(async move {
        while let Some((client_info, packet)) = rx.recv().await {
            debug!("Packet recieved through channel: {} bytes", packet.len());
            if packet.is_empty() {
                warn!("Empty packet recieved: closing socket");
                sockets.lock().await.remove(&client_info);
                continue;
            }

            // dereferenced borrow of `packet`, slightly unusual!
            let packet = packets::process(&packet)
                .await
                .expect("Packet processing failed.");

            let response: Option<CranPacket> = {
                use bincode::Options;
                let serializer = serializer!();
                match packet.packet_type {
                    PacketType::Status => {
                        let s = sockets.lock().await;
                        Some(CranPacket::new(
                            PacketType::Status,
                            serializer
                                .serialize(&StatusPacket {
                                    connected_clients: s.keys().copied().collect(),
                                })
                                .unwrap(),
                        ))
                    }
                    _ => None,
                }
            };

            if let Some(response) = response {
                use bincode::Options;
                let packet = BytesMut::from(serializer!().serialize(&response).unwrap().as_slice());

                if let Some(stream) = sockets.lock().await.get_mut(&client_info) {
                    let mut sink = FramedWrite::new(stream, LengthDelimitedCodec::new());
                    debug!(
                        "Sending response packet to {}...",
                        &client_info.address
                    );
                    // TODO: Remove or replace this with hexdump function
                    let _ = packets::process(&packet).await;
                    sink.send(packet.into()).await.unwrap();
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
