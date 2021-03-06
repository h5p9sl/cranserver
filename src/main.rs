use log::*;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};

use std::error::Error;
use std::net::{Ipv4Addr, SocketAddr};

mod config;

fn init_logging() {
    env_logger::builder()
        .filter_level(LevelFilter::Debug)
        .format_timestamp_millis()
        .init();
    debug!("Logger initialized");
}

#[derive(Debug, Copy, Clone)]
enum ClientType {
    Client,
    Controller,
}

type SocketList = Arc<Mutex<std::collections::HashMap<SocketAddr, TcpStream>>>;

async fn handle<T: 'static + Send + From<String>>(
    client_type: ClientType,
    tx: mpsc::Sender<T>,
    ip: std::net::SocketAddr,
    sockets: SocketList,
) -> Result<(), Box<dyn Error>> {
    info!("{:?} connection {} accepted.", client_type, ip);
    let mut buf = Vec::new();
    tokio::spawn(async move {

        // wait until socket is readable
        let mut sockets = sockets.lock().await;
        let s = sockets.get_mut(&ip).unwrap();
        s.readable().await.unwrap();

        // read packets into buf
        while s.read_buf(&mut buf).await.unwrap_or_else(|e| {
            error!("Socket with {} read failure: {}", ip, e);
            0
        }) > 0
        {
            let msg = std::str::from_utf8(&buf).unwrap();
            info!("Message recieved from {} \"{}\"", ip, msg);
            if let Err(e) = tx.send(msg.to_string().into()).await {
                error!("Reciever dropped ({})", e);
                break;
            }
        }
        debug!("Connection with {} terminated.", ip);
    });
    Ok(())
}

pub enum PacketType {
    GetPing = 1,
    FileTransfer = 2,
}

pub struct CranPacket {
    data_len: usize,
    packet_type: PacketType,
    data: Vec<u8>,
}

#[tokio::main]
async fn main() -> tokio::io::Result<()> {
    // TODO:
    // 0. Somehow access sockets from the main function
    // 1. Packet formats
    // 2. Commands / Operations
    // 3. Command executor / Cran Processor

    init_logging();

    let cfg = config::get_config()
        .unwrap_or_else(|e| {
            warn!("Failed to get config. Using default settings. ({})", e);
            Some(Default::default())
        })
        .expect("No configuration found and no default fallback.");

    let client_addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), cfg.client_port);
    let ctrl_addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), cfg.control_port);

    let sockets: SocketList = Default::default();

    let (tx, mut rx) = mpsc::channel::<String>(8);

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
    let t_clients = accept_and_handle_connections(ClientType::Client, client_addr);
    let t_ctrl = accept_and_handle_connections(ClientType::Controller, ctrl_addr);

    let t_commands = tokio::spawn(async move {
        while let Some(i) = rx.recv().await {
            println!("got = {}", i);
        }
    });

    info!("Initialization done. Ready!");
    let result = tokio::try_join!(t_clients, t_ctrl, t_commands,);
    result.unwrap();
    Ok(())
}
