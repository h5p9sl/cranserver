use log::*;
use serde::{self, Deserialize, Serialize};
use std::error::Error;
use std::net::SocketAddr;

// TODO Find a better method for defining serialization config
#[macro_export]
macro_rules! serializer {
    () => {{
        bincode::options().with_big_endian().with_fixint_encoding()
    }};
}

/*
pub fn serializer() -> impl bincode::Options {
    use bincode::Options;
    bincode::options().with_big_endian().with_fixint_encoding()
}
*/

#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
pub enum ClientType {
    Client,
    Controller,
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
pub struct ClientInfo {
    pub address: SocketAddr,
    pub client_type: ClientType,
}

impl ClientInfo {
    pub fn new(address: SocketAddr, client_type: ClientType) -> Self {
        ClientInfo {
            address,
            client_type,
        }
    }
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
pub enum PacketType {
    Ack,
    Status,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CranPacket {
    pub packet_type: PacketType,
    pub data: Vec<u8>,
}

impl CranPacket {
    pub fn new<T>(packet_type: PacketType, data: T) -> CranPacket
    where
        T: Into<Vec<u8>>,
    {
        let data = data.into();
        CranPacket { packet_type, data }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct StatusPacket {
    pub connected_clients: Vec<ClientInfo>,
}

impl Into<Vec<u8>> for StatusPacket {
    fn into(self) -> Vec<u8> {
        use bincode::Options;
        serializer!().serialize(&self).unwrap()
    }
}

pub async fn process<T>(client_type: ClientType, raw_packet: T) -> Result<CranPacket, Box<dyn Error>>
where
    T: Into<Vec<u8>>,
{
    let raw_packet = raw_packet.into();

    use bincode::Options;
    let packet: CranPacket = serializer!().deserialize(&raw_packet)?;
    debug!("Deserialized packet: {:#?}", &packet);
    {
        let hex = std::process::Command::new("hexdump")
            .arg("-C")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();

        use std::io::{Read, Write};
        hex.stdin.unwrap().write_all(&raw_packet)?;
        let mut s = String::new();
        hex.stdout.unwrap().read_to_string(&mut s)?;
        debug!("Hexdump of deserialized packet:\n{}", s);
    }

    Ok(packet)
}
