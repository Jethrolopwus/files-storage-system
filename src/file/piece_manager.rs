use crate::core::{
    Bitfield, FileError, Hash, Piece, PieceIndex, Result, TorrentError, ValidationError,
};
use std::collections::HashMap;

use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom};

//== Manages file pieces for a torrent ==//
#[derive(Debug)]
pub struct PieceManager {
    //=== All pieces for the torrent ===//
    pieces: HashMap<PieceIndex, Piece>,
    bitfield: Bitfield,
    piece_length: u32,
    num_pieces: usize,
    piece_cache: HashMap<PieceIndex, Vec<u8>>,
    cache_size: usize,
}

impl PieceManager {
    //=== Create a new piece manager ===//
    pub fn new(piece_hashes: Vec<Hash>, piece_length: u32, cache_size: usize) -> Self {
        let num_pieces = piece_hashes.len();
        let mut pieces = HashMap::new();

        for (index, hash) in piece_hashes.into_iter().enumerate() {
            pieces.insert(index as PieceIndex, Piece::new(index as PieceIndex, hash));
        }

        Self {
            pieces,
            bitfield: Bitfield::new(num_pieces),
            piece_length,
            num_pieces,
            piece_cache: HashMap::new(),
            cache_size,
        }
    }

    //=== Get the bitfield representing completed pieces ===//
    pub fn bitfield(&self) -> &Bitfield {
        &self.bitfield
    }
    pub fn num_pieces(&self) -> usize {
        self.num_pieces
    }
    pub fn piece_length(&self) -> u32 {
        self.piece_length
    }

    pub fn is_valid_piece(&self, piece_index: PieceIndex) -> bool {
        (piece_index as usize) < self.num_pieces
    }
    pub fn has_piece(&self, piece_index: PieceIndex) -> bool {
        self.bitfield.has_piece(piece_index)
    }

    pub fn get_piece(&self, piece_index: PieceIndex) -> Option<&Piece> {
        self.pieces.get(&piece_index)
    }
    pub fn get_piece_mut(&mut self, piece_index: PieceIndex) -> Option<&mut Piece> {
        self.pieces.get_mut(&piece_index)
    }

    //==  Add piece data and verify it ==//
    pub fn add_piece_data(&mut self, piece_index: PieceIndex, data: Vec<u8>) -> Result<bool> {
        if !self.is_valid_piece(piece_index) {
            return Err(TorrentError::Validation(ValidationError::InvalidHash));
        }

        let piece =
            self.pieces
                .get_mut(&piece_index)
                .ok_or(TorrentError::File(FileError::NotFound {
                    path: format!("piece {}", piece_index),
                }))?;

        let verified = piece.set_data(data.clone());

        if verified {
            self.bitfield.set_piece(piece_index);
            if self.piece_cache.len() >= self.cache_size {
                if let Some(oldest_key) = self.piece_cache.keys().next().copied() {
                    self.piece_cache.remove(&oldest_key);
                }
            }
            self.piece_cache.insert(piece_index, data);
        }

        Ok(verified)
    }

    //=== Get piece data from cache or piece storage ===//
    pub fn get_piece_data(&self, piece_index: PieceIndex) -> Option<&Vec<u8>> {
        if let Some(data) = self.piece_cache.get(&piece_index) {
            return Some(data);
        }

        if let Some(piece) = self.pieces.get(&piece_index) {
            piece.data.as_ref()
        } else {
            None
        }
    }

    //== Remove piece from cache ==//
    pub fn evict_from_cache(&mut self, piece_index: PieceIndex) {
        self.piece_cache.remove(&piece_index);
    }
    pub fn missing_pieces(&self) -> Vec<PieceIndex> {
        self.bitfield.missing_pieces()
    }
    pub fn completed_pieces(&self) -> Vec<PieceIndex> {
        self.bitfield.available_pieces()
    }
    pub fn completion_percentage(&self) -> f64 {
        self.bitfield.completion_percentage()
    }
    pub fn is_complete(&self) -> bool {
        self.bitfield.is_complete()
    }

    //== Verify all completed pieces ==//
    pub fn verify_all_pieces(&mut self) -> Result<Vec<PieceIndex>> {
        let mut failed_pieces = Vec::new();

        for piece_index in self.completed_pieces() {
            if let Some(piece) = self.pieces.get_mut(&piece_index) {
                if !piece.verify() {
                    failed_pieces.push(piece_index);
                    self.bitfield.unset_piece(piece_index);
                    piece.data = None;
                    piece.verified = false;
                    self.piece_cache.remove(&piece_index);
                }
            }
        }

        Ok(failed_pieces)
    }

    //== Load pieces from file system ==//
    pub async fn load_from_files(
        &mut self,
        file_paths: &[String],
        file_sizes: &[u64],
    ) -> Result<()> {
        let mut current_offset = 0u64;

        for piece_index in 0..self.num_pieces as PieceIndex {
            let piece_size = if piece_index == (self.num_pieces - 1) as PieceIndex {
                let total_size: u64 = file_sizes.iter().sum();
                (total_size - (piece_index as u64 * self.piece_length as u64)) as u32
            } else {
                self.piece_length
            };

            let mut piece_data = vec![0u8; piece_size as usize];
            let mut bytes_read = 0;
            let mut file_index = 0;
            let mut file_offset = current_offset;

            while bytes_read < piece_size as usize && file_index < file_paths.len() {
                let file_path = &file_paths[file_index];
                let file_size = file_sizes[file_index];

                if file_offset >= file_size {
                    file_offset -= file_size;
                    file_index += 1;
                    continue;
                }

                let mut file = File::open(file_path).await.map_err(|_e| {
                    TorrentError::File(FileError::NotFound {
                        path: file_path.clone(),
                    })
                })?;

                file.seek(SeekFrom::Start(file_offset)).await?;

                let to_read = std::cmp::min(
                    piece_size as usize - bytes_read,
                    (file_size - file_offset) as usize,
                );

                let read = file
                    .read(&mut piece_data[bytes_read..bytes_read + to_read])
                    .await?;
                bytes_read += read;

                if read == to_read {
                    file_offset = 0;
                    file_index += 1;
                } else {
                    file_offset += read as u64;
                }
            }

            if bytes_read == piece_size as usize {
                self.add_piece_data(piece_index, piece_data)?;
            }

            current_offset += piece_size as u64;
        }

        Ok(())
    }

    //=== Write pieces to file system ===//
    pub async fn write_to_files(&self, file_paths: &[String], file_sizes: &[u64]) -> Result<()> {
        let mut current_offset = 0u64;

        for piece_index in 0..self.num_pieces as PieceIndex {
            if !self.has_piece(piece_index) {
                continue;
            }

            let piece_data = self.get_piece_data(piece_index).ok_or(TorrentError::File(
                FileError::NotFound {
                    path: format!("piece {}", piece_index),
                },
            ))?;

            let mut bytes_written = 0;
            let mut file_index = 0;
            let mut file_offset = current_offset;

            //=== Write piece data to multiple files if necessary ===/
            while bytes_written < piece_data.len() && file_index < file_paths.len() {
                let file_path = &file_paths[file_index];
                let file_size = file_sizes[file_index];

                if file_offset >= file_size {
                    file_offset -= file_size;
                    file_index += 1;
                    continue;
                }

                let mut file = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .open(file_path)
                    .await
                    .map_err(|_| {
                        TorrentError::File(FileError::PermissionDenied {
                            path: file_path.clone(),
                        })
                    })?;

                file.seek(SeekFrom::Start(file_offset)).await?;

                let to_write = std::cmp::min(
                    piece_data.len() - bytes_written,
                    (file_size - file_offset) as usize,
                );

                let written = file
                    .write(&piece_data[bytes_written..bytes_written + to_write])
                    .await?;
                bytes_written += written;

                if written == to_write {
                    file_offset = 0;
                    file_index += 1;
                } else {
                    file_offset += written as u64;
                }
            }

            current_offset += piece_data.len() as u64;
        }

        Ok(())
    }

    //=== Get cache statistics ===//
    pub fn cache_stats(&self) -> (usize, usize, f64) {
        let cache_used = self.piece_cache.len();
        let cache_total = self.cache_size;
        let hit_rate = if cache_used > 0 {
            cache_used as f64 / cache_total as f64
        } else {
            0.0
        };

        (cache_used, cache_total, hit_rate)
    }

    //=== Clear the piece cache ===//
    pub fn clear_cache(&mut self) {
        self.piece_cache.clear();
    }
}
