//! Torrent file parsing and creation

use crate::core::{TorrentInfo, FileInfo, Hash, Result, TorrentError, ValidationError, FileError};
use serde::{Deserialize, Serialize};
use std::path::Path;


/// Raw torrent file structure as it appears in .torrent files
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawTorrent {
    /// Info dictionary
    info: RawTorrentInfo,
    /// Announce URL (tracker)
    announce: Option<String>,
    /// List of announce URLs (multiple trackers)
    #[serde(rename = "announce-list")]
    announce_list: Option<Vec<Vec<String>>>,
    /// Comment
    comment: Option<String>,
    /// Created by
    #[serde(rename = "created by")]
    created_by: Option<String>,
    /// Creation date (Unix timestamp)
    #[serde(rename = "creation date")]
    creation_date: Option<u64>,
}

/// Raw info dictionary from torrent file
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawTorrentInfo {
    /// Name of the torrent
    name: String,
    /// Piece length in bytes
    #[serde(rename = "piece length")]
    piece_length: u32,
    /// Concatenated SHA-1 hashes of all pieces
    pieces: serde_bytes::ByteBuf,
    /// Private flag
    #[serde(default)]
    private: u8,
    /// Single file mode
    length: Option<u64>,
    /// Multi-file mode
    files: Option<Vec<RawFileInfo>>,
    /// MD5 hash for single file
    md5sum: Option<String>,
}

/// Raw file info from torrent file
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawFileInfo {
    /// File length
    length: u64,
    /// Path components
    path: Vec<String>,
    /// MD5 hash
    md5sum: Option<String>,
}

/// Torrent parser for reading and writing .torrent files
#[derive(Debug)]
pub struct TorrentParser;

impl TorrentParser {
    /// Parse a torrent file from bytes
    pub fn parse_bytes(data: &[u8]) -> Result<TorrentInfo> {
        // Note: This is a simplified implementation
        // Real torrent files use bencode format, not JSON
        let raw: RawTorrent = serde_json::from_slice(data)
            .map_err(|_e| TorrentError::Validation(ValidationError::InvalidTorrentInfo))?;
        
        Self::convert_raw_torrent(raw)
    }
    
    /// Parse a torrent file from a file path
    pub async fn parse_file<P: AsRef<Path>>(path: P) -> Result<TorrentInfo> {
        let data = tokio::fs::read(path).await
            .map_err(|_| TorrentError::File(FileError::NotFound { 
                path: "torrent file".to_string() 
            }))?;
        
        Self::parse_bytes(&data)
    }
    
    /// Convert raw torrent data to TorrentInfo
    fn convert_raw_torrent(raw: RawTorrent) -> Result<TorrentInfo> {
        let info = raw.info;
        
        // Validate piece length
        if info.piece_length == 0 {
            return Err(TorrentError::Validation(ValidationError::InvalidPieceSize));
        }
        
        // Parse piece hashes
        let pieces = Self::parse_pieces(&info.pieces)?;
        
        // Parse files
        let files = if let Some(files) = info.files {
            // Multi-file torrent
            files.into_iter().map(|f| FileInfo {
                path: f.path,
                length: f.length,
                md5sum: f.md5sum,
            }).collect()
        } else if let Some(length) = info.length {
            // Single file torrent
            vec![FileInfo {
                path: vec![info.name.clone()],
                length,
                md5sum: info.md5sum,
            }]
        } else {
            return Err(TorrentError::Validation(ValidationError::MissingField { 
                field: "files or length".to_string() 
            }));
        };
        
        Ok(TorrentInfo {
            name: info.name,
            piece_length: info.piece_length,
            pieces,
            files,
            private: info.private != 0,
            comment: raw.comment,
            creation_date: raw.creation_date,
            created_by: raw.created_by,
        })
    }
    
    /// Parse piece hashes from raw bytes
    fn parse_pieces(pieces_data: &[u8]) -> Result<Vec<Hash>> {
        if pieces_data.len() % 20 != 0 {
            return Err(TorrentError::Validation(ValidationError::InvalidHash));
        }
        
        let mut pieces = Vec::new();
        for chunk in pieces_data.chunks_exact(20) {
            let mut hash = [0u8; 20];
            hash.copy_from_slice(chunk);
            pieces.push(hash);
        }
        
        Ok(pieces)
    }
    
    /// Create a torrent file for a set of files
    pub async fn create_torrent<P: AsRef<Path>>(
        files: Vec<P>,
        piece_length: u32,
        name: String,
        comment: Option<String>,
    ) -> Result<TorrentInfo> {
        let mut file_infos = Vec::new();
        let mut all_data = Vec::new();
        
        for file_path in files {
            let path = file_path.as_ref();
            let metadata = tokio::fs::metadata(path).await
                .map_err(|_| TorrentError::File(FileError::NotFound { 
                    path: path.to_string_lossy().to_string() 
                }))?;
            
            let data = tokio::fs::read(path).await?;
            all_data.extend_from_slice(&data);
            
            let file_info = FileInfo {
                path: vec![path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()],
                length: metadata.len(),
                md5sum: None,
            };
            
            file_infos.push(file_info);
        }
        
        // Generate piece hashes
        let pieces = Self::generate_pieces(&all_data, piece_length)?;
        
        Ok(TorrentInfo {
            name,
            piece_length,
            pieces,
            files: file_infos,
            private: false,
            comment,
            creation_date: Some(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()),
            created_by: Some("file-storage-system".to_string()),
        })
    }
    
    /// Generate piece hashes for data
    fn generate_pieces(data: &[u8], piece_length: u32) -> Result<Vec<Hash>> {
        use sha1::{Sha1, Digest};
        
        let mut pieces = Vec::new();
        
        for chunk in data.chunks(piece_length as usize) {
            let mut hasher = Sha1::new();
            hasher.update(chunk);
            let result = hasher.finalize();
            let hash: Hash = result.into();
            pieces.push(hash);
        }
        
        Ok(pieces)
    }
    
    /// Serialize torrent info to bytes
    pub fn serialize_torrent(info: &TorrentInfo) -> Result<Vec<u8>> {
        let files = if info.files.len() == 1 {
            // Single file mode
            None
        } else {
            // Multi-file mode
            Some(info.files.iter().map(|f| RawFileInfo {
                length: f.length,
                path: f.path.clone(),
                md5sum: f.md5sum.clone(),
            }).collect())
        };
        
        let (length, md5sum) = if info.files.len() == 1 {
            (Some(info.files[0].length), info.files[0].md5sum.clone())
        } else {
            (None, None)
        };
        
        // Concatenate piece hashes
        let mut pieces_bytes = Vec::new();
        for piece in &info.pieces {
            pieces_bytes.extend_from_slice(piece);
        }
        
        let raw = RawTorrent {
            info: RawTorrentInfo {
                name: info.name.clone(),
                piece_length: info.piece_length,
                pieces: serde_bytes::ByteBuf::from(pieces_bytes),
                private: if info.private { 1 } else { 0 },
                length,
                files,
                md5sum,
            },
            announce: None,
            announce_list: None,
            comment: info.comment.clone(),
            created_by: info.created_by.clone(),
            creation_date: info.creation_date,
        };
        
        serde_json::to_vec(&raw)
            .map_err(|e| TorrentError::Serialization(e))
    }
    
    /// Write torrent info to a file
    pub async fn write_torrent_file<P: AsRef<Path>>(info: &TorrentInfo, path: P) -> Result<()> {
        let data = Self::serialize_torrent(info)?;
        tokio::fs::write(path, data).await?;
        Ok(())
    }
    
    /// Calculate info hash for a torrent
    pub fn calculate_info_hash(info: &TorrentInfo) -> Result<Hash> {
        use sha1::{Sha1, Digest};
        
        let serialized = Self::serialize_torrent(info)?;
        let mut hasher = Sha1::new();
        hasher.update(&serialized);
        let result = hasher.finalize();
        Ok(result.into())
    }
}