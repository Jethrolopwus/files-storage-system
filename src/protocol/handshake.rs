use crate::core::{Hash, PeerId};
use bytes::{Buf, BufMut, BytesMut};
use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[derive(Debug, Clone)]
pub struct Handshake {
    pub protocol_identifier: [u8; 19],
    pub reserved: [u8; 8],
    pub info_hash: Hash,
    pub peer_id: PeerId,
}

impl Handshake {
    pub fn new(info_hash: Hash, peer_id: PeerId) -> Self {
        Self {
            protocol_identifier: *b"BitTorrent protocol",
            reserved: [0; 8],
            info_hash,
            peer_id,
        }
    }

    //=== Serialize handshake to bytes ===//
    pub fn serialize(&self) -> Vec<u8> {
        let mut buffer = Vec::new();

        //=== Protocol identifier length (1 byte) ===//
        buffer.put_u8(self.protocol_identifier.len() as u8);

        buffer.extend_from_slice(&self.protocol_identifier);

        buffer.extend_from_slice(&self.reserved);

        buffer.extend_from_slice(&self.info_hash);

        buffer.extend_from_slice(&self.peer_id);

        buffer
    }

    //=== Deserialize handshake from bytes ==//
    pub fn deserialize(data: &[u8]) -> io::Result<Self> {
        if data.len() != 68 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid handshake length: {}", data.len()),
            ));
        }

        let mut buffer = BytesMut::from(data);

        //=== Read protocol identifier length ===//
        let protocol_length = buffer.get_u8() as usize;

        if protocol_length != 19 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid protocol length: {}", protocol_length),
            ));
        }

        //===== Read protocol identifier ======//
        let mut protocol_identifier = [0u8; 19];
        buffer.copy_to_slice(&mut protocol_identifier);

        if protocol_identifier != *b"BitTorrent protocol" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid protocol identifier",
            ));
        }

        let mut reserved = [0u8; 8];
        buffer.copy_to_slice(&mut reserved);

        let mut info_hash = [0u8; 20];
        buffer.copy_to_slice(&mut info_hash);

        let mut peer_id = [0u8; 20];
        buffer.copy_to_slice(&mut peer_id);

        Ok(Handshake {
            protocol_identifier,
            reserved,
            info_hash,
            peer_id,
        })
    }
}

//=== Handshake  for managing peer handshakes ===//
pub struct HandshakeHandler {
    stream: TcpStream,
}

impl HandshakeHandler {
    pub fn new(stream: TcpStream) -> Self {
        Self { stream }
    }

    //==== Send a handshake to the peer ====//
    pub async fn send_handshake(&mut self, handshake: &Handshake) -> io::Result<()> {
        let data = handshake.serialize();
        self.stream.write_all(&data).await?;
        self.stream.flush().await?;
        Ok(())
    }

    pub async fn receive_handshake(&mut self) -> io::Result<Handshake> {
        let mut buffer = [0u8; 68];
        self.stream.read_exact(&mut buffer).await?;
        Handshake::deserialize(&buffer)
    }

    //==== Perform a complete handshake  ====//
    pub async fn perform_handshake(
        &mut self,
        info_hash: Hash,
        peer_id: PeerId,
    ) -> io::Result<(Handshake, Handshake)> {
        let our_handshake = Handshake::new(info_hash, peer_id);
        self.send_handshake(&our_handshake).await?;
        let their_handshake = self.receive_handshake().await?;

        if their_handshake.info_hash != info_hash {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Info hash mismatch in handshake",
            ));
        }

        Ok((our_handshake, their_handshake))
    }

    //=== Get the TCP stream ===//
    pub fn into_stream(self) -> TcpStream {
        self.stream
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handshake_serialization() {
        let info_hash = [1u8; 20];
        let peer_id = [2u8; 20];
        let handshake = Handshake::new(info_hash, peer_id);

        let serialized = handshake.serialize();
        let deserialized = Handshake::deserialize(&serialized).unwrap();

        assert_eq!(
            handshake.protocol_identifier,
            deserialized.protocol_identifier
        );
        assert_eq!(handshake.info_hash, deserialized.info_hash);
        assert_eq!(handshake.peer_id, deserialized.peer_id);
    }

    #[test]
    fn test_handshake_length() {
        let info_hash = [1u8; 20];
        let peer_id = [2u8; 20];
        let handshake = Handshake::new(info_hash, peer_id);

        let serialized = handshake.serialize();
        assert_eq!(serialized.len(), 68);
    }
}
