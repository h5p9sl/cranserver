use crate::CranPacket;
use bytes::BytesMut;
use futures::TryStreamExt;
use log::*;
use std::error::Error;
use std::net::SocketAddr;
use tokio::net::{tcp::OwnedReadHalf, TcpListener};
use tokio::sync::mpsc;
use tokio_util::codec::{FramedRead, LengthDelimitedCodec};

use crate::packets::{ClientInfo, ClientType};
use crate::{PacketChannel, SocketList};

pub async fn process(raw_packet: &BytesMut) -> anyhow::Result<CranPacket> {
    use bincode::Options;
    let serializer = crate::serde::get_serializer();
    let packet: CranPacket = serializer.deserialize(&raw_packet)?;
    {
        let hex = std::process::Command::new("hexdump")
            .arg("-C")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();

        use std::fmt::Write as _;
        use std::io::{Read, Write};

        let mut msg = String::new();

        writeln!(
            msg,
            "Packet of type {:?} and size {}.",
            &packet.packet_type,
            &packet.data.len()
        )?;
        writeln!(msg, "Hexdump of packet:")?;

        hex.stdin.unwrap().write_all(&raw_packet)?;
        hex.stdout.unwrap().read_to_string(&mut msg)?;

        debug!("{}", msg);
    }

    Ok(packet)
}

pub async fn listen_and_accept(
    client_type: ClientType,
    listen_addr: SocketAddr,
    tx: mpsc::Sender<PacketChannel>,
    sockets: SocketList,
) {
    let listener = TcpListener::bind(listen_addr).await.unwrap();

    loop {
        match listener.accept().await {
            Ok((s, ip)) => {
                let channel = tx.clone();
                let sockets = sockets.clone();
                let (s_read, s_write) = s.into_split();
                let info = ClientInfo::new(ip, client_type);

                sockets.lock().await.insert(info, s_write);

                // Add this socket to the socketlist and begin recieving packets
                tokio::spawn(async move {
                    handle(info, channel, s_read).await.unwrap();
                });
            }
            Err(e) => error!("Error accepting socket: {:?}", e),
        }
    }
}

async fn handle(
    client: ClientInfo,
    tx: mpsc::Sender<PacketChannel>,
    s: OwnedReadHalf,
) -> Result<(), Box<dyn Error>> {
    let client_type = client.client_type;
    let ip = client.address;
    let client_info = ClientInfo::new(ip, client_type);
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
                    let client_info = ClientInfo::new(ip, client_type);
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
