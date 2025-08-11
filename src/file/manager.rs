use crate::core::{FileError, FileInfo, Result, TorrentError, TorrentInfo};
use crate::file::PieceManager;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs::create_dir_all;

#[derive(Debug)]
pub struct FileManager {
    torrent_info: TorrentInfo,
    piece_manager: PieceManager,
    download_path: PathBuf,
    file_paths: HashMap<String, PathBuf>,
    files_allocated: bool,
}

impl FileManager {
    //=== Create a new file manager ===//
    pub fn new(torrent_info: TorrentInfo, download_path: PathBuf, cache_size: usize) -> Self {
        let piece_manager = PieceManager::new(
            torrent_info.pieces.clone(),
            torrent_info.piece_length,
            cache_size,
        );

        Self {
            torrent_info,
            piece_manager,
            download_path,
            file_paths: HashMap::new(),
            files_allocated: false,
        }
    }

    pub fn torrent_info(&self) -> &TorrentInfo {
        &self.torrent_info
    }

    pub fn piece_manager(&self) -> &PieceManager {
        &self.piece_manager
    }

    pub fn piece_manager_mut(&mut self) -> &mut PieceManager {
        &mut self.piece_manager
    }

    //== Initialize file paths and create directory structure ===//
    pub async fn initialize(&mut self) -> Result<()> {
        create_dir_all(&self.download_path).await?;

        for file_info in &self.torrent_info.files {
            let file_path = self.download_path.join(file_info.full_path());

            if let Some(parent) = file_path.parent() {
                create_dir_all(parent).await?;
            }
            let key = file_info.full_path().to_string_lossy().to_string();
            self.file_paths.insert(key, file_path);
        }

        Ok(())
    }

    pub async fn allocate_files(&mut self) -> Result<()> {
        if self.files_allocated {
            return Ok(());
        }

        for file_info in &self.torrent_info.files {
            let key = file_info.full_path().to_string_lossy().to_string();
            if let Some(file_path) = self.file_paths.get(&key) {
                let file = tokio::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .open(file_path)
                    .await
                    .map_err(|_| {
                        TorrentError::File(FileError::PermissionDenied {
                            path: file_path.to_string_lossy().to_string(),
                        })
                    })?;
                file.set_len(file_info.length).await?;
            }
        }

        self.files_allocated = true;
        Ok(())
    }

    //== Check which pieces are already present on disk ==//
    pub async fn scan_existing_files(&mut self) -> Result<()> {
        let file_paths: Vec<String> = self
            .file_paths
            .values()
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        let file_sizes: Vec<u64> = self.torrent_info.files.iter().map(|f| f.length).collect();

        //== Check if all files exist ==//
        for path in &file_paths {
            if !Path::new(path).exists() {
                return Ok(());
            }
        }

        //== Load existing pieces ==//
        self.piece_manager
            .load_from_files(&file_paths, &file_sizes)
            .await?;

        Ok(())
    }

    //== Write completed pieces to disk ==//
    pub async fn flush_to_disk(&mut self) -> Result<()> {
        let file_paths: Vec<String> = self
            .file_paths
            .values()
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        let file_sizes: Vec<u64> = self.torrent_info.files.iter().map(|f| f.length).collect();

        self.piece_manager
            .write_to_files(&file_paths, &file_sizes)
            .await?;

        Ok(())
    }

    //== Get file path for a specific file ==//
    pub fn get_file_path(&self, file_info: &FileInfo) -> Option<&PathBuf> {
        let key = file_info.full_path().to_string_lossy().to_string();
        self.file_paths.get(&key)
    }
    //== Get all file paths ==//
    pub fn file_paths(&self) -> &HashMap<String, PathBuf> {
        &self.file_paths
    }

    pub fn files_allocated(&self) -> bool {
        self.files_allocated
    }

    //=== Get total download size ===//
    pub fn total_size(&self) -> u64 {
        self.torrent_info.total_size()
    }
    pub fn downloaded_size(&self) -> u64 {
        let completed_pieces = self.piece_manager.completed_pieces();
        let mut total = 0u64;

        for piece_index in completed_pieces {
            total += self.torrent_info.piece_size(piece_index) as u64;
        }

        total
    }

    //== Get completion percentage ==//
    pub fn completion_percentage(&self) -> f64 {
        self.piece_manager.completion_percentage()
    }
    pub fn is_complete(&self) -> bool {
        self.piece_manager.is_complete()
    }

    //== Verify integrity of all downloaded pieces ==//
    pub async fn verify_integrity(&mut self) -> Result<Vec<u32>> {
        let failed_pieces = self.piece_manager.verify_all_pieces()?;

        if !failed_pieces.is_empty() {
            log::warn!("Found {} corrupted pieces", failed_pieces.len());
        }

        Ok(failed_pieces)
    }

    //== Get file download progress ==//
    pub fn file_progress(&self) -> HashMap<String, f64> {
        let mut progress = HashMap::new();
        let mut current_offset = 0u64;

        for file_info in &self.torrent_info.files {
            let file_start = current_offset;
            let file_end = current_offset + file_info.length;
            let mut downloaded_bytes = 0u64;

            //=== Calculate how much of this file is downloaded ===//
            for piece_index in 0..self.torrent_info.num_pieces() as u32 {
                if !self.piece_manager.has_piece(piece_index) {
                    continue;
                }

                let piece_start = piece_index as u64 * self.torrent_info.piece_length as u64;
                let piece_end = piece_start + self.torrent_info.piece_size(piece_index) as u64;

                //== Check if piece overlaps with file ==//
                let overlap_start = std::cmp::max(piece_start, file_start);
                let overlap_end = std::cmp::min(piece_end, file_end);

                if overlap_start < overlap_end {
                    downloaded_bytes += overlap_end - overlap_start;
                }
            }

            let file_progress = if file_info.length > 0 {
                (downloaded_bytes as f64 / file_info.length as f64) * 100.0
            } else {
                100.0
            };

            let key = file_info.full_path().to_string_lossy().to_string();
            progress.insert(key, file_progress);

            current_offset += file_info.length;
        }

        progress
    }

    //== Get storage statistics ==//
    pub fn storage_stats(&self) -> Result<(u64, u64, u64)> {
        let total_size = self.total_size();
        let downloaded_size = self.downloaded_size();

        //== Get available disk space ==//
        let available_space = if let Some(_first_path) = self.file_paths.values().next() {
            0u64
        } else {
            0u64
        };

        Ok((total_size, downloaded_size, available_space))
    }
}
