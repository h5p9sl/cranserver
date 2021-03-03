#[macro_use]
use log::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt, Interest};
use tokio::net::*;

use std::error::Error;
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

#[tokio::main]
async fn main() -> tokio::io::Result<()> {
    init_logging();
    let mut ping_interval = Instant::now();
    let mut connect_interval = tokio::time::interval(Duration::from_secs(15));

    // Loop this at an interval upon failure
    debug!("Attempting connection to client.");

    connect_interval.tick().await;
    if let Ok(mut cran) = CranProcessor::new().await {
        info!("Successfully connected to client.");

        //while cran.connected().await.is_ok() {
        loop {
            // Measure ping every 3 seconds
            if ping_interval.elapsed() > Duration::from_secs(3) {
                ping_interval = Instant::now();
                info!(
                    "Ping to client: {}",
                    cran.ping_client().await.unwrap().as_millis()
                );
            }
        }
    }
    Ok(())
}
