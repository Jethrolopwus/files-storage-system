//! Peer representation and state management

use crate::core::{PeerId, Bitfield, PieceIndex};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Instant;

/// Possible states for a peer connection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeerState {
    /// Initial state - not connected
    Disconnected,
    /// Currently connecting
    Connecting,
    /// Connected but not handshaking
    Connected,
    /// Performing handshake
    Handshaking,
    /// Handshake complete, ready for protocol messages
    Ready,
    /// Connection is being closed
    Closing,
}

/// Choking state for upload/download
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChokingState {
    /// We are choking this peer (not uploading to them)
    Choked,
    /// We are not choking this peer (uploading to them)
    Unchoked,
}

/// Interest state for download
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterestState {
    /// We are not interested in this peer's pieces
    NotInterested,
    /// We are interested in this peer's pieces
    Interested,
}

/// Represents a peer connection and its state
#[derive(Debug, Clone)]
pub struct Peer {
    /// Unique peer identifier
    pub id: PeerId,
    /// Network address
    pub address: SocketAddr,
    /// Current connection state
    pub state: PeerState,
    /// Pieces this peer has available
    pub bitfield: Bitfield,
    /// Our choking state towards this peer
    pub am_choking: ChokingState,
    /// This peer's choking state towards us
    pub peer_choking: ChokingState,
    /// Our interest state in this peer
    pub am_interested: InterestState,
    /// This peer's interest state in us
    pub peer_interested: InterestState,
    /// Download rate from this peer (bytes per second)
    pub download_rate: f64,
    /// Upload rate to this peer (bytes per second)
    pub upload_rate: f64,
    /// Last time we heard from this peer
    pub last_seen: Instant,
    /// Last time we sent a message to this peer
    pub last_sent: Instant,
    /// Number of bytes downloaded from this peer
    pub downloaded: u64,
    /// Number of bytes uploaded to this peer
    pub uploaded: u64,
    /// Pieces currently being requested from this peer
    pub pending_requests: HashMap<PieceIndex, Instant>,
    /// Maximum number of concurrent requests to this peer
    pub max_requests: usize,
    /// Whether this peer supports fast extension
    pub supports_fast: bool,
    /// Whether this peer supports extended messaging
    pub supports_extended: bool,
}

impl Peer {
    /// Create a new peer with the given ID and address
    pub fn new(id: PeerId, address: SocketAddr, num_pieces: usize) -> Self {
        let now = Instant::now();
        Self {
            id,
            address,
            state: PeerState::Disconnected,
            bitfield: Bitfield::new(num_pieces),
            am_choking: ChokingState::Choked,
            peer_choking: ChokingState::Choked,
            am_interested: InterestState::NotInterested,
            peer_interested: InterestState::NotInterested,
            download_rate: 0.0,
            upload_rate: 0.0,
            last_seen: now,
            last_sent: now,
            downloaded: 0,
            uploaded: 0,
            pending_requests: HashMap::new(),
            max_requests: 5,
            supports_fast: false,
            supports_extended: false,
        }
    }
    
    /// Check if we can request pieces from this peer
    pub fn can_request(&self) -> bool {
        matches!(self.state, PeerState::Ready) &&
        matches!(self.peer_choking, ChokingState::Unchoked) &&
        matches!(self.am_interested, InterestState::Interested) &&
        self.pending_requests.len() < self.max_requests
    }
    
    /// Check if we can upload to this peer
    pub fn can_upload(&self) -> bool {
        matches!(self.state, PeerState::Ready) &&
        matches!(self.am_choking, ChokingState::Unchoked) &&
        matches!(self.peer_interested, InterestState::Interested)
    }
    
    /// Add a pending request for a piece
    pub fn add_request(&mut self, piece_index: PieceIndex) {
        self.pending_requests.insert(piece_index, Instant::now());
    }
    
    /// Remove a pending request
    pub fn remove_request(&mut self, piece_index: PieceIndex) {
        self.pending_requests.remove(&piece_index);
    }
    
    /// Check if a piece is already requested
    pub fn has_request(&self, piece_index: PieceIndex) -> bool {
        self.pending_requests.contains_key(&piece_index)
    }
    
    /// Get the number of pending requests
    pub fn pending_request_count(&self) -> usize {
        self.pending_requests.len()
    }
    
    /// Update download statistics
    pub fn update_download_stats(&mut self, bytes: u64) {
        self.downloaded += bytes;
        self.last_seen = Instant::now();
        // TODO: Implement rate calculation with moving average
    }
    
    /// Update upload statistics
    pub fn update_upload_stats(&mut self, bytes: u64) {
        self.uploaded += bytes;
        self.last_sent = Instant::now();
        // TODO: Implement rate calculation with moving average
    }
    
    /// Set the peer's bitfield
    pub fn set_bitfield(&mut self, bitfield: Bitfield) {
        self.bitfield = bitfield;
    }
    
    /// Mark that the peer has a specific piece
    pub fn has_piece(&mut self, piece_index: PieceIndex) {
        self.bitfield.set_piece(piece_index);
    }
    
    /// Check if the peer has a specific piece
    pub fn peer_has_piece(&self, piece_index: PieceIndex) -> bool {
        self.bitfield.has_piece(piece_index)
    }
    
    /// Get pieces that this peer has but we don't
    pub fn interesting_pieces(&self, our_bitfield: &Bitfield) -> Vec<PieceIndex> {
        let mut interesting = Vec::new();
        for piece_index in 0..self.bitfield.total_pieces() as PieceIndex {
            if self.peer_has_piece(piece_index) && !our_bitfield.has_piece(piece_index) {
                interesting.push(piece_index);
            }
        }
        interesting
    }
    
    /// Update interest state based on available pieces
    pub fn update_interest(&mut self, our_bitfield: &Bitfield) {
        let interesting = self.interesting_pieces(our_bitfield);
        self.am_interested = if interesting.is_empty() {
            InterestState::NotInterested
        } else {
            InterestState::Interested
        };
    }
    
    /// Check if this peer is a seeder (has all pieces)
    pub fn is_seeder(&self) -> bool {
        self.bitfield.is_complete()
    }
    
    /// Get completion percentage for this peer
    pub fn completion_percentage(&self) -> f64 {
        self.bitfield.completion_percentage()
    }
    
    /// Check if the peer connection is stale
    pub fn is_stale(&self, timeout: std::time::Duration) -> bool {
        self.last_seen.elapsed() > timeout
    }
    
    /// Get the peer's reputation score (simple calculation)
    pub fn reputation_score(&self) -> f64 {
        // Simple reputation based on upload/download ratio
        if self.downloaded == 0 {
            return 1.0; // Neutral for new peers
        }
        
        let ratio = self.uploaded as f64 / self.downloaded as f64;
        ratio.min(2.0) // Cap at 2.0 to prevent extreme values
    }
}

impl PartialEq for Peer {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Peer {}

impl std::hash::Hash for Peer {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}