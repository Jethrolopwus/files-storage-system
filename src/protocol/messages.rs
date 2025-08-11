use crate::core::{BlockLength, BlockOffset, PieceIndex};
use crate::protocol::{Message, MessageType};
use bytes::{Buf, BufMut, BytesMut};
use std::io;

//=== Message parsing utilities ===//
pub trait MessageParser {
    fn parse_have(&self) -> io::Result<PieceIndex>;
    fn parse_bitfield(&self) -> io::Result<Vec<u8>>;
    fn parse_request(&self) -> io::Result<(PieceIndex, BlockOffset, BlockLength)>;
    fn parse_piece(&self) -> io::Result<(PieceIndex, BlockOffset, Vec<u8>)>;
    fn parse_cancel(&self) -> io::Result<(PieceIndex, BlockOffset, BlockLength)>;
    fn parse_port(&self) -> io::Result<u16>;
}

impl MessageParser for Message {
    fn parse_have(&self) -> io::Result<PieceIndex> {
        if self.message_type != MessageType::Have {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a have message",
            ));
        }

        if self.payload.len() != 4 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid have message payload length",
            ));
        }

        let mut buffer = BytesMut::from(&self.payload[..]);
        Ok(buffer.get_u32())
    }

    fn parse_bitfield(&self) -> io::Result<Vec<u8>> {
        if self.message_type != MessageType::Bitfield {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a bitfield message",
            ));
        }

        Ok(self.payload.clone())
    }

    fn parse_request(&self) -> io::Result<(PieceIndex, BlockOffset, BlockLength)> {
        if self.message_type != MessageType::Request {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a request message",
            ));
        }

        if self.payload.len() != 12 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid request message payload length",
            ));
        }

        let mut buffer = BytesMut::from(&self.payload[..]);
        let piece_index = buffer.get_u32();
        let offset = buffer.get_u32();
        let length = buffer.get_u32();

        Ok((piece_index, offset, length))
    }

    fn parse_piece(&self) -> io::Result<(PieceIndex, BlockOffset, Vec<u8>)> {
        if self.message_type != MessageType::Piece {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a piece message",
            ));
        }

        if self.payload.len() < 8 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid piece message payload length",
            ));
        }

        let mut buffer = BytesMut::from(&self.payload[..]);
        let piece_index = buffer.get_u32();
        let offset = buffer.get_u32();
        let data = buffer.to_vec();

        Ok((piece_index, offset, data))
    }

    fn parse_cancel(&self) -> io::Result<(PieceIndex, BlockOffset, BlockLength)> {
        if self.message_type != MessageType::Cancel {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a cancel message",
            ));
        }

        if self.payload.len() != 12 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid cancel message payload length",
            ));
        }

        let mut buffer = BytesMut::from(&self.payload[..]);
        let piece_index = buffer.get_u32();
        let offset = buffer.get_u32();
        let length = buffer.get_u32();

        Ok((piece_index, offset, length))
    }

    fn parse_port(&self) -> io::Result<u16> {
        if self.message_type != MessageType::Port {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a port message",
            ));
        }

        if self.payload.len() != 2 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid port message payload length",
            ));
        }

        let mut buffer = BytesMut::from(&self.payload[..]);
        Ok(buffer.get_u16())
    }
}

//=== Message builder utilities ===//
pub trait MessageBuilder {
    fn build_have(piece_index: PieceIndex) -> Message;
    fn build_bitfield(bitfield: &[u8]) -> Message;
    fn build_request(piece_index: PieceIndex, offset: BlockOffset, length: BlockLength) -> Message;
    fn build_piece(piece_index: PieceIndex, offset: BlockOffset, data: Vec<u8>) -> Message;
    fn build_cancel(piece_index: PieceIndex, offset: BlockOffset, length: BlockLength) -> Message;
    fn build_port(port: u16) -> Message;
}

impl MessageBuilder for Message {
    fn build_have(piece_index: PieceIndex) -> Message {
        Message::have(piece_index)
    }

    fn build_bitfield(bitfield: &[u8]) -> Message {
        Message::bitfield(bitfield)
    }

    fn build_request(piece_index: PieceIndex, offset: BlockOffset, length: BlockLength) -> Message {
        Message::request(piece_index, offset, length)
    }

    fn build_piece(piece_index: PieceIndex, offset: BlockOffset, data: Vec<u8>) -> Message {
        Message::piece(piece_index, offset, data)
    }

    fn build_cancel(piece_index: PieceIndex, offset: BlockOffset, length: BlockLength) -> Message {
        Message::cancel(piece_index, offset, length)
    }

    fn build_port(port: u16) -> Message {
        let mut payload = Vec::new();
        payload.put_u16(port);
        Message::new(MessageType::Port, payload)
    }
}

//=== Message validation utilities ===//
pub trait MessageValidator {
    fn is_valid(&self) -> bool;
    fn validate_request(&self, max_piece_size: u32) -> bool;
    fn validate_piece(&self, max_piece_size: u32) -> bool;
}

impl MessageValidator for Message {
    fn is_valid(&self) -> bool {
        match self.message_type {
            MessageType::Choke
            | MessageType::Unchoke
            | MessageType::Interested
            | MessageType::NotInterested => self.payload.is_empty(),
            MessageType::Have => self.payload.len() == 4,
            MessageType::Bitfield => !self.payload.is_empty(),
            MessageType::Request | MessageType::Cancel => self.payload.len() == 12,
            MessageType::Piece => self.payload.len() >= 8,
            MessageType::Port => self.payload.len() == 2,
            MessageType::KeepAlive => self.payload.is_empty(),
        }
    }

    fn validate_request(&self, max_piece_size: u32) -> bool {
        if self.message_type != MessageType::Request {
            return false;
        }

        if let Ok((_piece_index, offset, length)) = self.parse_request() {
            offset + length <= max_piece_size && length > 0 && length <= 16384
        } else {
            false
        }
    }

    fn validate_piece(&self, max_piece_size: u32) -> bool {
        if self.message_type != MessageType::Piece {
            return false;
        }

        if let Ok((_piece_index, offset, data)) = self.parse_piece() {
            offset + data.len() as u32 <= max_piece_size
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_have_message_parsing() {
        let message = Message::have(123);
        let piece_index = message.parse_have().unwrap();
        assert_eq!(piece_index, 123);
    }

    #[test]
    fn test_request_message_parsing() {
        let message = Message::request(1, 1024, 16384);
        let (piece_index, offset, length) = message.parse_request().unwrap();
        assert_eq!(piece_index, 1);
        assert_eq!(offset, 1024);
        assert_eq!(length, 16384);
    }

    #[test]
    fn test_piece_message_parsing() {
        let data = vec![1, 2, 3, 4, 5];
        let message = Message::piece(1, 1024, data.clone());
        let (piece_index, offset, received_data) = message.parse_piece().unwrap();
        assert_eq!(piece_index, 1);
        assert_eq!(offset, 1024);
        assert_eq!(received_data, data);
    }

    #[test]
    fn test_message_validation() {
        let valid_message = Message::have(123);
        assert!(valid_message.is_valid());

        let invalid_message = Message::new(MessageType::Have, vec![1, 2, 3]); // Wrong length
        assert!(!invalid_message.is_valid());
    }

    #[test]
    fn test_request_validation() {
        let valid_request = Message::request(1, 0, 16384);
        assert!(valid_request.validate_request(65536));

        let invalid_request = Message::request(1, 0, 0); // Zero length
        assert!(!invalid_request.validate_request(65536));
    }
}
