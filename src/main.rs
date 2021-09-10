use bytes::BytesMut;
use futures::{SinkExt, StreamExt};
use log::*;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tokio::net::{tcp::OwnedWriteHalf, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tokio_util::codec::{Framed, FramedWrite, LengthDelimitedCodec};

/// Server configuration
mod config;

/// Shared packet types
mod packets;
use packets::{CranPacket, CranPacketBuilder, PacketType, StatusPacket};

/// Client-Server packet processing
mod processing;

mod serde {
    use bincode::{DefaultOptions, Options};
    pub fn get_serializer() -> impl Options {
        DefaultOptions::new()
            .with_big_endian()
            .with_fixint_encoding()
    }
}

type SocketList = Arc<Mutex<std::collections::HashMap<packets::ClientInfo, OwnedWriteHalf>>>;
type PacketChannel = (packets::ClientInfo, BytesMut);

#[inline(always)]
fn init_logging() {
    env_logger::builder()
        .filter_level(LevelFilter::Debug)
        .format_timestamp_millis()
        .init();
    debug!("Logger initialized");
}

async fn act_as_client(of_type: packets::ClientType) -> anyhow::Result<()> {
    use bincode::Options;
    use packets::*;
    let stream = TcpStream::connect("127.0.0.1:8080").await?;

    match of_type {
        ClientType::Client => {
            let packet = CranPacketBuilder::new(PacketType::Status).build();
            let packet = crate::serde::get_serializer().serialize(&packet)?.into();
            let mut stream = Framed::new(stream, LengthDelimitedCodec::new());

            stream.send(packet).await?;
            info!("Sent packet!");

            let buf: BytesMut = stream.next().await.unwrap()?;
            info!("Recieved packet of length {}", buf.len());

            let packet = processing::process(&buf).await;
            if let Ok(packet) = packet {
                match packet.packet_type {
                    packets::PacketType::Status => {
                        let inner: StatusPacket =
                            crate::serde::get_serializer().deserialize(&packet.data)?;
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

#[tokio::main]
async fn main() -> tokio::io::Result<()> {
    init_logging();

    if std::env::args().rfind(|s| s == "-client").is_some() {
        act_as_client(packets::ClientType::Client).await.unwrap();
        return Ok(());
    }

    let cfg = config::get_config().unwrap_or_else(|e| {
        error!("Failed to parse config. Using default settings. ({})", e);
        Default::default()
    });

    let client_addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), cfg.client_port);
    let ctrl_addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), cfg.control_port);

    let sockets: SocketList = Default::default();
    let (tx, mut rx) = mpsc::channel::<PacketChannel>(8);

    let new_socket_task = |client_type, listen_addr| {
        let tx = tx.clone();
        let sockets = sockets.clone();
        info!(
            "Listening for new {:?} connections at {}",
            client_type, listen_addr
        );
        tokio::spawn(async move {
            processing::listen_and_accept(client_type, listen_addr, tx, sockets).await;
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

            let packet = processing::process(&packet).await;
            if let Err(e) = packet {
                error!("Packet processing failed: {}", e);
                continue;
            }
            let packet = packet.unwrap();

            let response: Option<CranPacket> = {
                match packet.packet_type {
                    PacketType::Status => {
                        let s = sockets.lock().await;
                        Some(
                            CranPacketBuilder::new(PacketType::Status)
                                .with_data_serialize(StatusPacket {
                                    connected_clients: s.keys().copied().collect(),
                                })
                                .unwrap()
                                .build(),
                        )
                    }
                    _ => None,
                }
            };

            if let Some(response) = response {
                use bincode::Options;
                let serializer = crate::serde::get_serializer();
                let packet = BytesMut::from(serializer.serialize(&response).unwrap().as_slice());

                if let Some(stream) = sockets.lock().await.get_mut(&client_info) {
                    let mut sink = FramedWrite::new(stream, LengthDelimitedCodec::new());
                    debug!("Sending response packet to {}...", &client_info.address);
                    // TODO: Remove or replace this with hexdump function
                    let _ = processing::process(&packet).await;
                    sink.send(packet.into())
                        .await
                        .expect("Failed to send response packet");
                } else {
                    error!("Failed to send response: socket no longer exists.");
                }
            }
        }
    });

    info!("Initialization done. Ready!");
    tokio::try_join!(t_clients, t_ctrl, t_commands,)?;
    Ok(())
}
