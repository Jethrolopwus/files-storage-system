//! Peer management and coordination

use crate::core::{PeerId, PieceIndex, Bitfield, Result, TorrentError, PeerError};
use crate::peer::{Peer, PeerState, ChokingState, InterestState};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

/// Manages all peer connections for a torrent
#[derive(Debug)]
pub struct PeerManager {
    /// All connected and known peers
    peers: HashMap<PeerId, Peer>,
    /// Our bitfield (pieces we have)
    our_bitfield: Bitfield,
    /// Maximum number of peers to maintain
    max_peers: usize,
    /// Peer connection timeout
    connection_timeout: Duration,
    /// Last time we performed choking algorithm
    last_choke_time: Instant,
    /// Interval for choking algorithm
    choke_interval: Duration,
    /// Peers we're currently uploading to
    unchoked_peers: HashSet<PeerId>,
    /// Maximum number of unchoked peers
    max_unchoked: usize,
    /// Optimistic unchoke peer
    optimistic_unchoke: Option<PeerId>,
}

impl PeerManager {
    /// Create a new peer manager
    pub fn new(num_pieces: usize, max_peers: usize) -> Self {
        Self {
            peers: HashMap::new(),
            our_bitfield: Bitfield::new(num_pieces),
            max_peers,
            connection_timeout: Duration::from_secs(30),
            last_choke_time: Instant::now(),
            choke_interval: Duration::from_secs(10),
            unchoked_peers: HashSet::new(),
            max_unchoked: 4,
            optimistic_unchoke: None,
        }
    }
    
    /// Add a new peer
    pub fn add_peer(&mut self, peer_id: PeerId, address: SocketAddr) -> Result<()> {
        if self.peers.len() >= self.max_peers {
            return Err(TorrentError::Peer(PeerError::NotFound {
                peer_id: format!("{:?}", peer_id)
            }));
        }
        
        if !self.peers.contains_key(&peer_id) {
            let peer = Peer::new(peer_id, address, self.our_bitfield.total_pieces());
            self.peers.insert(peer_id, peer);
        }
        
        Ok(())
    }
    
    /// Remove a peer
    pub fn remove_peer(&mut self, peer_id: &PeerId) -> Option<Peer> {
        self.unchoked_peers.remove(peer_id);
        if Some(*peer_id) == self.optimistic_unchoke {
            self.optimistic_unchoke = None;
        }
        self.peers.remove(peer_id)
    }
    
    /// Get a peer by ID
    pub fn get_peer(&self, peer_id: &PeerId) -> Option<&Peer> {
        self.peers.get(peer_id)
    }
    
    /// Get a mutable reference to a peer
    pub fn get_peer_mut(&mut self, peer_id: &PeerId) -> Option<&mut Peer> {
        self.peers.get_mut(peer_id)
    }
    
    /// Get all peers
    pub fn peers(&self) -> &HashMap<PeerId, Peer> {
        &self.peers
    }
    
    /// Get all peer IDs
    pub fn peer_ids(&self) -> Vec<PeerId> {
        self.peers.keys().copied().collect()
    }
    
    /// Get peers in a specific state
    pub fn peers_in_state(&self, state: PeerState) -> Vec<&Peer> {
        self.peers.values().filter(|p| p.state == state).collect()
    }
    
    /// Get connected peer count
    pub fn connected_peer_count(&self) -> usize {
        self.peers.values().filter(|p| matches!(p.state, PeerState::Ready)).count()
    }
    
    /// Get seeder count
    pub fn seeder_count(&self) -> usize {
        self.peers.values().filter(|p| p.is_seeder()).count()
    }
    
    /// Get leecher count
    pub fn leecher_count(&self) -> usize {
        self.connected_peer_count() - self.seeder_count()
    }
    
    /// Update our bitfield when we complete a piece
    pub fn completed_piece(&mut self, piece_index: PieceIndex) {
        self.our_bitfield.set_piece(piece_index);
        
        // Update interest states for all peers
        for peer in self.peers.values_mut() {
            peer.update_interest(&self.our_bitfield);
        }
    }
    
    /// Find peers that have a specific piece
    pub fn peers_with_piece(&self, piece_index: PieceIndex) -> Vec<PeerId> {
        self.peers
            .iter()
            .filter(|(_, peer)| peer.peer_has_piece(piece_index))
            .map(|(id, _)| *id)
            .collect()
    }
    
    /// Find the rarest pieces among connected peers
    pub fn rarest_pieces(&self) -> Vec<(PieceIndex, usize)> {
        let mut piece_counts: HashMap<PieceIndex, usize> = HashMap::new();
        
        // Count how many peers have each piece
        for peer in self.peers.values() {
            if matches!(peer.state, PeerState::Ready) {
                for piece_index in peer.bitfield.available_pieces() {
                    *piece_counts.entry(piece_index).or_insert(0) += 1;
                }
            }
        }
        
        // Convert to sorted vec (rarest first)
        let mut pieces: Vec<(PieceIndex, usize)> = piece_counts.into_iter().collect();
        pieces.sort_by_key(|(_, count)| *count);
        pieces
    }
    
    /// Get pieces we're missing that at least one peer has
    pub fn missing_pieces_available(&self) -> Vec<PieceIndex> {
        let missing = self.our_bitfield.missing_pieces();
        missing
            .into_iter()
            .filter(|&piece_index| {
                self.peers.values().any(|peer| peer.peer_has_piece(piece_index))
            })
            .collect()
    }
    
    /// Find best peers to request a piece from
    pub fn best_peers_for_piece(&self, piece_index: PieceIndex) -> Vec<PeerId> {
        let mut candidates: Vec<_> = self.peers
            .iter()
            .filter(|(_, peer)| {
                peer.can_request() && 
                peer.peer_has_piece(piece_index) &&
                !peer.has_request(piece_index)
            })
            .collect();
        
        // Sort by reputation and download rate
        candidates.sort_by(|(_, a), (_, b)| {
            let score_a = a.reputation_score() + a.download_rate / 1000.0;
            let score_b = b.reputation_score() + b.download_rate / 1000.0;
            score_b.partial_cmp(&score_a).unwrap_or(std::cmp::Ordering::Equal)
        });
        
        candidates.into_iter().map(|(id, _)| *id).collect()
    }
    
    /// Perform choking algorithm (tit-for-tat)
    pub fn update_choking(&mut self) {
        if self.last_choke_time.elapsed() < self.choke_interval {
            return;
        }
        
        self.last_choke_time = Instant::now();
        
        // Get interested peers sorted by upload rate
        let mut interested_peers: Vec<_> = self.peers
            .iter()
            .filter(|(_, peer)| matches!(peer.peer_interested, InterestState::Interested))
            .collect();
        
        interested_peers.sort_by(|(_, a), (_, b)| {
            b.upload_rate.partial_cmp(&a.upload_rate).unwrap_or(std::cmp::Ordering::Equal)
        });
        
        // Unchoke top uploaders
        let mut new_unchoked = HashSet::new();
        for (peer_id, _) in interested_peers.iter().take(self.max_unchoked.saturating_sub(1)) {
            new_unchoked.insert(**peer_id);
        }
        
        // Optimistic unchoke
        if self.optimistic_unchoke.is_none() || rand::random::<f32>() < 0.1 {
            // Select random interested peer that's not already unchoked
            let choked_interested: Vec<_> = interested_peers
                .iter()
                .filter(|(id, _)| !new_unchoked.contains(*id))
                .collect();
            
            if let Some((peer_id, _)) = choked_interested.get(0) {
                self.optimistic_unchoke = Some(**peer_id);
            }
        }
        
        if let Some(opt_peer) = self.optimistic_unchoke {
            new_unchoked.insert(opt_peer);
        }
        
        // Update choking states
        for (peer_id, peer) in self.peers.iter_mut() {
            let should_unchoke = new_unchoked.contains(peer_id);
            peer.am_choking = if should_unchoke {
                ChokingState::Unchoked
            } else {
                ChokingState::Choked
            };
        }
        
        self.unchoked_peers = new_unchoked;
    }
    
    /// Clean up stale peer connections
    pub fn cleanup_stale_peers(&mut self) {
        let stale_peers: Vec<PeerId> = self.peers
            .iter()
            .filter(|(_, peer)| peer.is_stale(self.connection_timeout))
            .map(|(id, _)| *id)
            .collect();
        
        for peer_id in stale_peers {
            self.remove_peer(&peer_id);
        }
    }
    
    /// Get our completion percentage
    pub fn completion_percentage(&self) -> f64 {
        self.our_bitfield.completion_percentage()
    }
    
    /// Check if we have completed the download
    pub fn is_complete(&self) -> bool {
        self.our_bitfield.is_complete()
    }
    
    /// Get download statistics
    pub fn download_stats(&self) -> (u64, u64, f64, f64) {
        let total_downloaded: u64 = self.peers.values().map(|p| p.downloaded).sum();
        let total_uploaded: u64 = self.peers.values().map(|p| p.uploaded).sum();
        let avg_download_rate: f64 = self.peers.values().map(|p| p.download_rate).sum::<f64>() / self.peers.len() as f64;
        let avg_upload_rate: f64 = self.peers.values().map(|p| p.upload_rate).sum::<f64>() / self.peers.len() as f64;
        
        (total_downloaded, total_uploaded, avg_download_rate, avg_upload_rate)
    }
}