use serde::{self, Deserialize, Serialize};
use std::net::SocketAddr;

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

/// The base packet where all packet types inherit from
#[derive(Serialize, Deserialize, Debug)]
pub struct CranPacket {
    pub packet_type: PacketType,
    pub data: Vec<u8>,
}

/// Builder for `CranPacket`
pub struct CranPacketBuilder {
    p_type: PacketType,
    p_data: Vec<u8>,
}

impl CranPacketBuilder {
    pub fn new(p_type: PacketType) -> Self {
        Self {
            p_type,
            p_data: Vec::new(),
        }
    }

    pub fn with_data<T>(mut self, data: T) -> Self
    where
        T: Into<Vec<u8>>,
    {
        self.p_data = data.into();
        self
    }

    /// Serialize `data` into inner packet data
    pub fn with_data_serialize<T>(mut self, data: T) -> anyhow::Result<Self>
    where
        T: Serialize,
    {
        use bincode::Options;
        let serializer = crate::serde::get_serializer();
        self.p_data = serializer.serialize(&data)?;
        Ok(self)
    }

    pub fn build(self) -> CranPacket {
        CranPacket {
            packet_type: self.p_type,
            data: self.p_data,
        }
    }
}

/// Inner packet types and their data
pub mod types {
    use crate::packets::ClientInfo;
    use serde::{self, Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug)]
    pub struct StatusPacket {
        pub connected_clients: Vec<ClientInfo>,
    }
}
pub use self::types::StatusPacket;
