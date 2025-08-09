//! Command-line client for the file storage system

use file_storage_system::prelude::*;
use file_storage_system::file::{TorrentParser, FileManager, PieceManager};
use file_storage_system::peer::PeerManager;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tokio;

#[derive(Parser)]
#[command(name = "file-storage-client")]
#[command(about = "A BitTorrent-like file storage system client")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new torrent file
    Create {
        /// Files to include in the torrent
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Output torrent file path
        #[arg(short, long)]
        output: PathBuf,
        /// Torrent name
        #[arg(short, long)]
        name: String,
        /// Piece size in bytes
        #[arg(short, long, default_value = "262144")]
        piece_size: u32,
        /// Optional comment
        #[arg(short, long)]
        comment: Option<String>,
    },
    /// Show information about a torrent file
    Info {
        /// Path to the torrent file
        torrent: PathBuf,
    },
    /// Download a torrent (placeholder - networking not implemented yet)
    Download {
        /// Path to the torrent file
        torrent: PathBuf,
        /// Download directory
        #[arg(short, long, default_value = "./downloads")]
        output_dir: PathBuf,
    },
    /// Verify integrity of downloaded files
    Verify {
        /// Path to the torrent file
        torrent: PathBuf,
        /// Directory containing the files
        #[arg(short, long)]
        data_dir: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    
    let cli = Cli::parse();
    
    match cli.command {
        Commands::Create { files, output, name, piece_size, comment } => {
            create_torrent(files, output, name, piece_size, comment).await?;
        }
        Commands::Info { torrent } => {
            show_torrent_info(torrent).await?;
        }
        Commands::Download { torrent, output_dir } => {
            download_torrent(torrent, output_dir).await?;
        }
        Commands::Verify { torrent, data_dir } => {
            verify_torrent(torrent, data_dir).await?;
        }
    }
    
    Ok(())
}

async fn create_torrent(
    files: Vec<PathBuf>,
    output: PathBuf,
    name: String,
    piece_size: u32,
    comment: Option<String>,
) -> Result<()> {
    println!("Creating torrent '{}'...", name);
    
    let torrent_info = TorrentParser::create_torrent(files, piece_size, name, comment).await?;
    
    TorrentParser::write_torrent_file(&torrent_info, &output).await?;
    
    println!("Torrent created successfully: {}", output.display());
    println!("  Name: {}", torrent_info.name);
    println!("  Files: {}", torrent_info.files.len());
    println!("  Pieces: {}", torrent_info.num_pieces());
    println!("  Piece size: {} bytes", torrent_info.piece_length);
    println!("  Total size: {} bytes", torrent_info.total_size());
    
    Ok(())
}

async fn show_torrent_info(torrent: PathBuf) -> Result<()> {
    println!("Loading torrent file: {}", torrent.display());
    
    let torrent_info = TorrentParser::parse_file(torrent).await?;
    
    println!("\nTorrent Information:");
    println!("  Name: {}", torrent_info.name);
    println!("  Private: {}", torrent_info.private);
    println!("  Piece length: {} bytes", torrent_info.piece_length);
    println!("  Number of pieces: {}", torrent_info.num_pieces());
    println!("  Total size: {} bytes", torrent_info.total_size());
    
    if let Some(comment) = &torrent_info.comment {
        println!("  Comment: {}", comment);
    }
    
    if let Some(created_by) = &torrent_info.created_by {
        println!("  Created by: {}", created_by);
    }
    
    if let Some(creation_date) = torrent_info.creation_date {
        println!("  Creation date: {}", creation_date);
    }
    
    println!("\nFiles:");
    for (i, file) in torrent_info.files.iter().enumerate() {
        println!("  {}: {} ({} bytes)", 
                 i + 1, 
                 file.full_path().display(), 
                 file.length);
    }
    
    Ok(())
}

async fn download_torrent(torrent: PathBuf, output_dir: PathBuf) -> Result<()> {
    println!("Loading torrent: {}", torrent.display());
    
    let torrent_info = TorrentParser::parse_file(torrent).await?;
    let mut file_manager = FileManager::new(torrent_info.clone(), output_dir.clone(), 100);
    
    // Initialize file structure
    file_manager.initialize().await?;
    
    println!("Initialized download in: {}", output_dir.display());
    println!("Note: Actual downloading requires networking implementation (Phase 2)");
    println!("For now, this just sets up the file structure.");
    
    // Allocate space for files
    file_manager.allocate_files().await?;
    println!("Allocated space for {} files", torrent_info.files.len());
    
    // Scan for existing files
    file_manager.scan_existing_files().await?;
    
    let completion = file_manager.completion_percentage();
    println!("Current completion: {:.2}%", completion);
    
    // Show file progress
    let file_progress = file_manager.file_progress();
    for (file_path, progress) in file_progress {
        println!("  {}: {:.2}%", file_path, progress);
    }
    
    Ok(())
}

async fn verify_torrent(torrent: PathBuf, data_dir: PathBuf) -> Result<()> {
    println!("Verifying torrent data in: {}", data_dir.display());
    
    let torrent_info = TorrentParser::parse_file(torrent).await?;
    let mut file_manager = FileManager::new(torrent_info.clone(), data_dir, 100);
    
    // Initialize and scan existing files
    file_manager.initialize().await?;
    file_manager.scan_existing_files().await?;
    
    println!("Scanning and verifying pieces...");
    
    // Verify integrity
    let failed_pieces = file_manager.verify_integrity().await?;
    
    if failed_pieces.is_empty() {
        println!("✓ All pieces verified successfully!");
    } else {
        println!("✗ Found {} corrupted pieces:", failed_pieces.len());
        for piece in failed_pieces {
            println!("  Piece {}", piece);
        }
    }
    
    let completion = file_manager.completion_percentage();
    println!("Overall completion: {:.2}%", completion);
    
    Ok(())
}