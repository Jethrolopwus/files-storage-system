use clap::{Parser, Subcommand};
use file_storage_system::file::{FileManager, PieceManager, TorrentParser};
use file_storage_system::peer::PeerManager;
use file_storage_system::prelude::*;
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
//== Create a new torrent file ==//
enum Commands {
    Create {
        #[arg(required = true)]
        files: Vec<PathBuf>,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(short, long)]
        name: String,
        #[arg(short, long, default_value = "262144")]
        piece_size: u32,
        #[arg(short, long)]
        comment: Option<String>,
    },
    //===  information about a torrent file ===//
    Info {
        torrent: PathBuf,
    },

    Download {
        torrent: PathBuf,

        #[arg(short, long, default_value = "./downloads")]
        output_dir: PathBuf,
    },
    //=== Verify integrity of downloaded files==//
    Verify {
        torrent: PathBuf,

        #[arg(short, long)]
        data_dir: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Create {
            files,
            output,
            name,
            piece_size,
            comment,
        } => {
            create_torrent(files, output, name, piece_size, comment).await?;
        }
        Commands::Info { torrent } => {
            show_torrent_info(torrent).await?;
        }
        Commands::Download {
            torrent,
            output_dir,
        } => {
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
        println!(
            "  {}: {} ({} bytes)",
            i + 1,
            file.full_path().display(),
            file.length
        );
    }

    Ok(())
}

async fn download_torrent(torrent: PathBuf, output_dir: PathBuf) -> Result<()> {
    println!("Loading torrent: {}", torrent.display());

    let torrent_info = TorrentParser::parse_file(torrent).await?;
    let mut file_manager = FileManager::new(torrent_info.clone(), output_dir.clone(), 100);

    //=== Initialize file structure ==//
    file_manager.initialize().await?;

    println!("Initialized download in: {}", output_dir.display());
    println!("Note: Actual downloading requires networking implementation (Phase 2)");
    println!("For now, this just sets up the file structure.");

    file_manager.allocate_files().await?;
    println!("Allocated space for {} files", torrent_info.files.len());

    file_manager.scan_existing_files().await?;

    let completion = file_manager.completion_percentage();
    println!("Current completion: {:.2}%", completion);

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

    file_manager.initialize().await?;
    file_manager.scan_existing_files().await?;

    println!("Scanning and verifying pieces...");

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
