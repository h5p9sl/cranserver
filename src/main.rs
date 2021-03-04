#[macro_use]
use log::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::*;

use std::error::Error;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

struct CranProcessor {
    client_stream: Box<TcpStream>,
}

impl CranProcessor {
    async fn new() -> Result<Self, Box<dyn Error>> {
        let client_stream = Box::new(TcpStream::connect("127.0.0.1:8080").await?);
        Ok(Self { client_stream })
    }

    async fn ping_client(&mut self) -> Result<Duration, Box<dyn Error>> {
        let mut buf = Vec::new();
        let t_send = Instant::now();
        debug!("Ping packet being sent...");
        self.client_stream.write_all(b"hello world").await?;
        self.client_stream.read_buf(&mut buf).await?;
        Ok(t_send.elapsed())
    }
}

fn init_logging() {
    env_logger::builder()
        .filter_level(LevelFilter::Debug)
        .format_timestamp_millis()
        .init();
    debug!("Logger initialized");
}

use serde::Deserialize;
#[derive(Deserialize)]
struct Config {
    client_port: u16,
    control_port: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            client_port: 8080,
            control_port: 8090,
        }
    }
}

fn find_cfg() -> Option<std::path::PathBuf> {
    let locations = vec!["cranserver.toml", "/etc/cranserver/cranserver.toml"];

    for l in &locations {
        let p = std::path::PathBuf::from(l);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn get_config() -> Result<Option<Config>, Box<dyn std::error::Error>> {
    if let Some(cfg) = find_cfg() {
        Ok(Some(toml::from_str(&std::fs::read_to_string(cfg)?)?))
    } else {
        use std::io::{Error, ErrorKind};
        Err(Box::new(Error::new(
            ErrorKind::NotFound,
            "Config file was not found.",
        )))
    }
}

async fn accept_clients(address: std::net::SocketAddr) -> Result<(), Box<dyn Error>> {
    let listener = TcpListener::bind(address).await?;
    let (socket, ip) = listener.accept().await?;

    info!("Client connection {} accepted.", ip);
    tokio::spawn(async move {
        // process & await
    });
    Ok(())
}

async fn accept_controls(address: std::net::SocketAddr) -> Result<(), Box<dyn Error>> {
    let listener = TcpListener::bind(address).await?;
    let (mut socket, ip) = listener.accept().await?;

    info!("Controller connection {} accepted.", ip);
    tokio::spawn(async move {
        // process & await
        while socket.readable().await.is_ok() {
            let mut msg = Vec::new();
            socket.read_buf(&mut msg).await.unwrap();
            info!(
                "Message recieved from {}: {}",
                ip,
                std::str::from_utf8(&msg).unwrap()
            );
            if msg.len() == 0 {
                error!("Connection closed");
                break;
            }
        }
    });
    Ok(())
}

#[tokio::main]
async fn main() -> tokio::io::Result<()> {
    init_logging();

    let cfg = get_config()
        .unwrap_or_else(|e| {
            warn!("Failed to get config. Using default settings. ({})", e);
            Some(Default::default())
        })
        .unwrap_or_else(|| {
            panic!("No configuration found and no default fallback.");
        });

    let client_addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), cfg.client_port);
    let ctrl_addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), cfg.control_port);

    let task_clients = tokio::spawn(async move {
        loop {
            debug!(
                "Listening for new client connections on port {}",
                client_addr.port()
            );
            accept_clients(client_addr).await.unwrap();
        }
    });
    let task_ctrl = tokio::spawn(async move {
        loop {
            debug!(
                "Listening for new controller connections on port {}",
                ctrl_addr.port()
            );
            accept_controls(ctrl_addr).await.unwrap();
        }
    });

    info!("Initialization done. Ready!");
    task_clients.await?;
    task_ctrl.await?;

    Ok(())
}
