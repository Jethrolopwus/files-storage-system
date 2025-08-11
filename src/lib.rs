pub mod core;
pub mod file;
pub mod network;
pub mod peer;
pub mod protocol;

pub use core::*;

//== Re-exports for common types ==//
pub mod prelude {
    pub use crate::core::*;
    pub use crate::file::{FileManager, PieceManager, TorrentParser};
    pub use crate::peer::{ChokingState, InterestState, Peer, PeerManager, PeerState};
    pub use crate::network::{NetworkManager, ConnectionManager, ConnectionPool, TrackerManager};
    pub use crate::protocol::{Message, MessageType, ProtocolHandler, Handshake, HandshakeHandler};
    pub use anyhow::{Error, Result};
}
