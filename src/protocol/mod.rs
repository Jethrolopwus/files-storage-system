use crate::core::{BlockLength, BlockOffset, PieceIndex};
use bytes::{Buf, BufMut, BytesMut};
use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub mod handshake;
pub mod messages;

pub use handshake::*;
pub use messages::*;

//==== protocol constants ====//
pub const PROTOCOL_IDENTIFIER: &[u8] = b"BitTorrent protocol";
pub const PROTOCOL_VERSION: u8 = 1;

//==== Protocol message types ===//
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    Choke = 0,
    Unchoke = 1,
    Interested = 2,
    NotInterested = 3,
    Have = 4,
    Bitfield = 5,
    Request = 6,
    Piece = 7,
    Cancel = 8,
    Port = 9,
    KeepAlive = 255,
}

impl From<u8> for MessageType {
    fn from(value: u8) -> Self {
        match value {
            0 => MessageType::Choke,
            1 => MessageType::Unchoke,
            2 => MessageType::Interested,
            3 => MessageType::NotInterested,
            4 => MessageType::Have,
            5 => MessageType::Bitfield,
            6 => MessageType::Request,
            7 => MessageType::Piece,
            8 => MessageType::Cancel,
            9 => MessageType::Port,
            _ => MessageType::KeepAlive,
        }
    }
}

impl From<MessageType> for u8 {
    fn from(msg_type: MessageType) -> Self {
        msg_type as u8
    }
}

#[derive(Debug, Clone)]
pub struct Message {
    pub message_type: MessageType,
    pub payload: Vec<u8>,
}

impl Message {
    pub fn new(message_type: MessageType, payload: Vec<u8>) -> Self {
        Self {
            message_type,
            payload,
        }
    }

    pub fn keep_alive() -> Self {
        Self {
            message_type: MessageType::KeepAlive,
            payload: Vec::new(),
        }
    }

    //==== Serialize keep-alive message ====//
    pub fn serialize_keep_alive() -> Vec<u8> {
        vec![0, 0, 0, 0]
    }

    pub fn choke() -> Self {
        Self {
            message_type: MessageType::Choke,
            payload: Vec::new(),
        }
    }

    pub fn unchoke() -> Self {
        Self {
            message_type: MessageType::Unchoke,
            payload: Vec::new(),
        }
    }

    pub fn interested() -> Self {
        Self {
            message_type: MessageType::Interested,
            payload: Vec::new(),
        }
    }

    pub fn not_interested() -> Self {
        Self {
            message_type: MessageType::NotInterested,
            payload: Vec::new(),
        }
    }

    pub fn have(piece_index: PieceIndex) -> Self {
        let mut payload = Vec::new();
        payload.put_u32(piece_index);
        Self {
            message_type: MessageType::Have,
            payload,
        }
    }

    pub fn bitfield(bitfield: &[u8]) -> Self {
        Self {
            message_type: MessageType::Bitfield,
            payload: bitfield.to_vec(),
        }
    }

    pub fn request(piece_index: PieceIndex, offset: BlockOffset, length: BlockLength) -> Self {
        let mut payload = Vec::new();
        payload.put_u32(piece_index);
        payload.put_u32(offset);
        payload.put_u32(length);
        Self {
            message_type: MessageType::Request,
            payload,
        }
    }

    pub fn piece(piece_index: PieceIndex, offset: BlockOffset, data: Vec<u8>) -> Self {
        let mut payload = Vec::new();
        payload.put_u32(piece_index);
        payload.put_u32(offset);
        payload.extend_from_slice(&data);
        Self {
            message_type: MessageType::Piece,
            payload,
        }
    }

    pub fn cancel(piece_index: PieceIndex, offset: BlockOffset, length: BlockLength) -> Self {
        let mut payload = Vec::new();
        payload.put_u32(piece_index);
        payload.put_u32(offset);
        payload.put_u32(length);
        Self {
            message_type: MessageType::Cancel,
            payload,
        }
    }

    //=== Serialize message to bytes  ===//
    pub fn serialize(&self) -> Vec<u8> {
        let mut buffer = Vec::new();

        let total_length = 1 + self.payload.len();
        buffer.put_u32(total_length as u32);

        buffer.put_u8(self.message_type.into());

        buffer.extend_from_slice(&self.payload);

        buffer
    }

    /// Deserialize message from bytes
    pub fn deserialize(data: &[u8]) -> io::Result<Self> {
        if data.len() < 5 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Message too short",
            ));
        }

        let mut buffer = BytesMut::from(data);

        let message_length = buffer.get_u32() as usize;

        if message_length == 0 {
            return Ok(Message::keep_alive());
        }

        if data.len() < 5 + message_length - 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Incomplete message",
            ));
        }

        //=== Read message type, payload ===//
        let message_type = MessageType::from(buffer.get_u8());

        let payload = buffer[..message_length - 1].to_vec();

        Ok(Message {
            message_type,
            payload,
        })
    }
}

//==== Protocol handler for peer connections ====//
pub struct ProtocolHandler {
    stream: TcpStream,
    buffer: BytesMut,
}

impl ProtocolHandler {
    pub fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            buffer: BytesMut::new(),
        }
    }

    //==== Send and Recieve a message to the peer ===//
    pub async fn send_message(&mut self, message: &Message) -> io::Result<()> {
        let data = message.serialize();
        self.stream.write_all(&data).await?;
        self.stream.flush().await?;
        Ok(())
    }

    pub async fn receive_message(&mut self) -> io::Result<Message> {
        loop {
            if let Some(message) = self.try_parse_message()? {
                return Ok(message);
            }

            let mut chunk = vec![0u8; 1024];
            let n = self.stream.read(&mut chunk).await?;

            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Connection closed by peer",
                ));
            }

            self.buffer.extend_from_slice(&chunk[..n]);
        }
    }

    //=== parse a complete message from the buffer ===//
    fn try_parse_message(&mut self) -> io::Result<Option<Message>> {
        if self.buffer.len() < 4 {
            return Ok(None);
        }

        let message_length = u32::from_be_bytes([
            self.buffer[0],
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
        ]) as usize;

        if message_length == 0 {
            self.buffer.advance(4);
            return Ok(Some(Message::keep_alive()));
        }

        let total_length = 4 + message_length;
        if self.buffer.len() < total_length {
            return Ok(None);
        }

        //==== Extract the complete message ====//
        let message_data = self.buffer[..total_length].to_vec();
        self.buffer.advance(total_length);

        Message::deserialize(&message_data).map(Some)
    }

    pub fn into_stream(self) -> TcpStream {
        self.stream
    }

    pub fn stream(&self) -> &TcpStream {
        &self.stream
    }
    pub fn stream_mut(&mut self) -> &mut TcpStream {
        &mut self.stream
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_serialization() {
        let message = Message::have(123);
        let serialized = message.serialize();
        let deserialized = Message::deserialize(&serialized).unwrap();

        assert_eq!(message.message_type, deserialized.message_type);
        assert_eq!(message.payload, deserialized.payload);
    }

    #[test]
    fn test_keep_alive_message() {
        let message = Message::keep_alive();
        let serialized = Message::serialize_keep_alive();
        assert_eq!(serialized, vec![0, 0, 0, 0]);

        // Test that the message itself has the correct type
        assert_eq!(message.message_type, MessageType::KeepAlive);
        assert_eq!(message.payload, vec![] as Vec<u8>);
    }

    #[test]
    fn test_request_message() {
        let message = Message::request(1, 1024, 16384);
        let serialized = message.serialize();
        let deserialized = Message::deserialize(&serialized).unwrap();

        assert_eq!(message.message_type, deserialized.message_type);
        assert_eq!(message.payload, deserialized.payload);
    }
}
