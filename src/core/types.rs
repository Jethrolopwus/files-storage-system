//! Core types and data structures

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use bitvec::prelude::*;

/// SHA-1 hash type (20 bytes)
pub type Hash = [u8; 20];

/// Peer ID type (20 bytes)
pub type PeerId = [u8; 20];

/// Piece index type
pub type PieceIndex = u32;

/// Block offset within a piece
pub type BlockOffset = u32;

/// Block length
pub type BlockLength = u32;

/// Configuration for the torrent system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Network settings
    pub listen_port: u16,
    pub max_connections: usize,
    pub connection_timeout: Duration,
    
    /// File settings
    pub download_path: PathBuf,
    pub piece_cache_size: usize,
    
    /// Choking settings
    pub upload_limit: Option<u64>,
    pub download_limit: Option<u64>,
    pub unchoke_interval: Duration,
    
    /// Tracker settings
    pub tracker_timeout: Duration,
    pub announce_interval: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_port: 6881,
            max_connections: 50,
            connection_timeout: Duration::from_secs(30),
            download_path: PathBuf::from("./downloads"),
            piece_cache_size: 100,
            upload_limit: None,
            download_limit: None,
            unchoke_interval: Duration::from_secs(10),
            tracker_timeout: Duration::from_secs(30),
            announce_interval: Duration::from_secs(1800),
        }
    }
}

/// Information about a single file in a torrent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    /// Path components (directory structure)
    pub path: Vec<String>,
    /// File length in bytes
    pub length: u64,
    /// Optional MD5 hash for verification
    pub md5sum: Option<String>,
}

impl FileInfo {
    pub fn new(path: Vec<String>, length: u64) -> Self {
        Self {
            path,
            length,
            md5sum: None,
        }
    }
    
    /// Get the full file path as a PathBuf
    pub fn full_path(&self) -> PathBuf {
        self.path.iter().collect()
    }
    
    /// Get the filename
    pub fn filename(&self) -> Option<&String> {
        self.path.last()
    }
}

/// Complete torrent metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentInfo {
    /// Name of the torrent
    pub name: String,
    /// Length of each piece in bytes (except possibly the last piece)
    pub piece_length: u32,
    /// SHA-1 hashes of all pieces
    pub pieces: Vec<Hash>,
    /// List of files in the torrent
    pub files: Vec<FileInfo>,
    /// Whether this is a private torrent
    pub private: bool,
    /// Optional comment
    pub comment: Option<String>,
    /// Optional creation date (Unix timestamp)
    pub creation_date: Option<u64>,
    /// Optional created by field
    pub created_by: Option<String>,
}

impl TorrentInfo {
    pub fn new(name: String, piece_length: u32, pieces: Vec<Hash>, files: Vec<FileInfo>) -> Self {
        Self {
            name,
            piece_length,
            pieces,
            files,
            private: false,
            comment: None,
            creation_date: None,
            created_by: None,
        }
    }
    
    /// Get the total number of pieces
    pub fn num_pieces(&self) -> usize {
        self.pieces.len()
    }
    
    /// Get the total size of all files
    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|f| f.length).sum()
    }
    
    /// Get the size of a specific piece
    pub fn piece_size(&self, piece_index: PieceIndex) -> u32 {
        let total_size = self.total_size();
        let piece_index = piece_index as u64;
        let piece_length = self.piece_length as u64;
        
        if piece_index < (self.num_pieces() as u64 - 1) {
            self.piece_length
        } else {
            // Last piece might be smaller
            (total_size - piece_index * piece_length) as u32
        }
    }
    
    /// Check if a piece index is valid
    pub fn is_valid_piece_index(&self, piece_index: PieceIndex) -> bool {
        (piece_index as usize) < self.num_pieces()
    }
}

/// Represents a single piece of a file
#[derive(Debug, Clone)]
pub struct Piece {
    /// Piece index
    pub index: PieceIndex,
    /// Piece data (None if not downloaded)
    pub data: Option<Vec<u8>>,
    /// Expected SHA-1 hash
    pub hash: Hash,
    /// Whether the piece has been verified
    pub verified: bool,
    /// Whether requests for this piece are in flight
    pub in_flight: bool,
    /// Timestamp of last request
    pub last_requested: Option<Instant>,
}

impl Piece {
    pub fn new(index: PieceIndex, hash: Hash) -> Self {
        Self {
            index,
            data: None,
            hash,
            verified: false,
            in_flight: false,
            last_requested: None,
        }
    }
    
    /// Check if the piece is complete and verified
    pub fn is_complete(&self) -> bool {
        self.data.is_some() && self.verified
    }
    
    /// Verify the piece data against its hash
    pub fn verify(&mut self) -> bool {
        if let Some(data) = &self.data {
            use sha1::{Sha1, Digest};
            let mut hasher = Sha1::new();
            hasher.update(data);
            let result = hasher.finalize();
            let computed_hash: Hash = result.into();
            
            self.verified = computed_hash == self.hash;
            self.verified
        } else {
            false
        }
    }
    
    /// Set piece data and verify it
    pub fn set_data(&mut self, data: Vec<u8>) -> bool {
        self.data = Some(data);
        self.verify()
    }
}

/// Bitfield for tracking piece availability
#[derive(Debug, Clone)]
pub struct Bitfield {
    bits: BitVec,
    num_pieces: usize,
}

impl Bitfield {
    /// Create a new bitfield with all pieces set to false
    pub fn new(num_pieces: usize) -> Self {
        Self {
            bits: bitvec![0; num_pieces],
            num_pieces,
        }
    }
    
    /// Create a bitfield from raw bytes
    pub fn from_bytes(bytes: &[u8], num_pieces: usize) -> Self {
        let mut bits: BitVec = BitVec::new();
        for byte in bytes {
            for i in 0..8 {
                if bits.len() >= num_pieces {
                    break;
                }
                bits.push((byte >> (7 - i)) & 1 == 1);
            }
            if bits.len() >= num_pieces {
                break;
            }
        }
        bits.resize(num_pieces, false);
        Self { bits, num_pieces }
    }
    
    /// Convert to raw bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut current_byte = 0u8;
        
        for (i, bit) in self.bits.iter().enumerate() {
            if *bit {
                current_byte |= 1 << (7 - (i % 8));
            }
            
            if (i + 1) % 8 == 0 || i == self.bits.len() - 1 {
                bytes.push(current_byte);
                current_byte = 0;
            }
        }
        
        bytes
    }
    
    /// Set a piece as available
    pub fn set_piece(&mut self, piece_index: PieceIndex) {
        if let Some(mut bit) = self.bits.get_mut(piece_index as usize) {
            *bit = true;
        }
    }
    
    /// Unset a piece
    pub fn unset_piece(&mut self, piece_index: PieceIndex) {
        if let Some(mut bit) = self.bits.get_mut(piece_index as usize) {
            *bit = false;
        }
    }
    
    /// Check if a piece is available
    pub fn has_piece(&self, piece_index: PieceIndex) -> bool {
        self.bits.get(piece_index as usize).map(|b| *b).unwrap_or(false)
    }
    
    /// Get the number of pieces we have
    pub fn count_pieces(&self) -> usize {
        self.bits.count_ones()
    }
    
    /// Get the total number of pieces
    pub fn total_pieces(&self) -> usize {
        self.num_pieces
    }
    
    /// Check if we have all pieces
    pub fn is_complete(&self) -> bool {
        self.count_pieces() == self.num_pieces
    }
    
    /// Get completion percentage
    pub fn completion_percentage(&self) -> f64 {
        if self.num_pieces == 0 {
            return 100.0;
        }
        (self.count_pieces() as f64 / self.num_pieces as f64) * 100.0
    }
    
    /// Find missing pieces
    pub fn missing_pieces(&self) -> Vec<PieceIndex> {
        self.bits
            .iter()
            .enumerate()
            .filter_map(|(i, bit)| if !*bit { Some(i as PieceIndex) } else { None })
            .collect()
    }
    
    /// Find available pieces
    pub fn available_pieces(&self) -> Vec<PieceIndex> {
        self.bits
            .iter()
            .enumerate()
            .filter_map(|(i, bit)| if *bit { Some(i as PieceIndex) } else { None })
            .collect()
    }
}

/// Statistics for tracking download/upload progress
#[derive(Debug, Clone, Default)]
pub struct Statistics {
    pub downloaded: u64,
    pub uploaded: u64,
    pub left: u64,
    pub corrupt: u64,
    pub download_rate: u64,
    pub upload_rate: u64,
    pub num_peers: usize,
    pub num_seeds: usize,
    pub num_leechers: usize,
}

impl Statistics {
    pub fn new(total_size: u64) -> Self {
        Self {
            left: total_size,
            ..Default::default()
        }
    }
    
    pub fn update_downloaded(&mut self, bytes: u64) {
        self.downloaded += bytes;
        self.left = self.left.saturating_sub(bytes);
    }
    
    pub fn update_uploaded(&mut self, bytes: u64) {
        self.uploaded += bytes;
    }
    
    pub fn completion_percentage(&self) -> f64 {
        let total = self.downloaded + self.left;
        if total == 0 {
            return 100.0;
        }
        (self.downloaded as f64 / total as f64) * 100.0
    }
}