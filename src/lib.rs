//! BitTorrent-like File Storage System
//! 
//! A distributed file storage system implementing BitTorrent protocol concepts
//! with peer-to-peer file sharing, distributed hash tables, and efficient
//! piece-based file transfer.

pub mod core;
pub mod network;
pub mod protocol;
pub mod file;
pub mod peer;

pub use core::*;

/// Re-exports for common types
pub mod prelude {
    pub use crate::core::*;
    pub use crate::file::{FileManager, PieceManager, TorrentParser};
    pub use crate::peer::{Peer, PeerState, PeerManager, ChokingState, InterestState};
    pub use anyhow::{Result, Error};
}