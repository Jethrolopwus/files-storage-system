use std::io;
use thiserror::Error;

//=== Main error types ===//
#[derive(Error, Debug)]
pub enum TorrentError {
    #[error("Network error: {0}")]
    Network(#[from] NetworkError),

    #[error("File error: {0}")]
    File(#[from] FileError),

    #[error("Protocol error: {0}")]
    Protocol(#[from] ProtocolError),

    #[error("Peer error: {0}")]
    Peer(#[from] PeerError),

    #[error("Validation error: {0}")]
    Validation(#[from] ValidationError),

    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Error, Debug)]
pub enum NetworkError {
    #[error("Connection failed")]
    ConnectionFailed,

    #[error("Connection timeout")]
    Timeout,

    #[error("Invalid message format")]
    InvalidMessage,

    #[error("Peer disconnected")]
    PeerDisconnected,

    #[error("Address resolution failed")]
    AddressResolution,

    #[error("Network bind failed")]
    BindFailed,
}

#[derive(Error, Debug)]
pub enum FileError {
    #[error("File not found: {path}")]
    NotFound { path: String },

    #[error("Permission denied: {path}")]
    PermissionDenied { path: String },

    #[error("Disk space insufficient")]
    InsufficientSpace,

    #[error("File corruption detected")]
    Corruption,

    #[error("Invalid file format")]
    InvalidFormat,

    #[error("Piece verification failed")]
    PieceVerificationFailed,
}

#[derive(Error, Debug)]
pub enum ProtocolError {
    #[error("Invalid handshake")]
    InvalidHandshake,

    #[error("Unsupported protocol version")]
    UnsupportedVersion,

    #[error("Invalid message type: {message_type}")]
    InvalidMessageType { message_type: u8 },

    #[error("Message too large: {size} bytes")]
    MessageTooLarge { size: usize },

    #[error("Invalid piece index: {index}")]
    InvalidPieceIndex { index: u32 },

    #[error("Invalid block request")]
    InvalidBlockRequest,
}

#[derive(Error, Debug)]
pub enum PeerError {
    #[error("Peer not found: {peer_id}")]
    NotFound { peer_id: String },

    #[error("Peer is choked")]
    Choked,

    #[error("Peer is not interested")]
    NotInterested,

    #[error("Invalid peer state transition")]
    InvalidStateTransition,

    #[error("Peer handshake failed")]
    HandshakeFailed,

    #[error("Peer timeout")]
    Timeout,
}

#[derive(Error, Debug)]
pub enum ValidationError {
    #[error("Invalid hash")]
    InvalidHash,

    #[error("Invalid torrent info")]
    InvalidTorrentInfo,

    #[error("Invalid piece size")]
    InvalidPieceSize,

    #[error("Missing required field: {field}")]
    MissingField { field: String },

    #[error("Invalid configuration: {message}")]
    InvalidConfig { message: String },
}

pub type Result<T> = std::result::Result<T, TorrentError>;
