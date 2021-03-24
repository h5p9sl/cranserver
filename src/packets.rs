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

#[derive(Serialize, Deserialize, PartialEq, Eq, Hash, Copy, Clone, Debug)]
pub enum ClientType {
    Client,
    Controller,
}

#[derive(Serialize, Deserialize, Eq, Copy, Clone, Debug)]
pub struct ClientInfo {
    pub address: SocketAddr,
    pub client_type: ClientType,
}

use std::hash::{Hash, Hasher};
impl Hash for ClientInfo {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.address.hash(state);
        self.client_type.hash(state);
    }
}

impl std::fmt::Display for ClientInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        use std::collections::hash_map::DefaultHasher;
        let mut h = DefaultHasher::new();
        self.hash(&mut h);
        write!(f, "{}", h.finish())?;
        Ok(())
    }
}

impl PartialEq for ClientInfo {
    fn eq(&self, other: &Self) -> bool {
        self.address == other.address
    }
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

pub async fn process<T>(raw_packet: T) -> Result<CranPacket, Box<dyn Error>>
where
    T: Into<Vec<u8>>,
{
    let raw_packet = raw_packet.into();

    use bincode::Options;
    let packet: CranPacket = serializer!().deserialize(&raw_packet)?;
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
