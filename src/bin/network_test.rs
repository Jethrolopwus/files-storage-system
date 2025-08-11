//! Network layer test binary

use file_storage_system::prelude::*;
use file_storage_system::network::{ConnectionInfo, ConnectionState, TrackerEvent, TrackerRequest};
use file_storage_system::protocol::{MessageParser, MessageValidator};
use log::info;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    env_logger::init();
    
    info!("Starting network layer test");
    
    // Test 1: Network Manager Creation and Configuration
    test_network_manager_creation().await?;
    
    // Test 2: Protocol Message Handling
    test_protocol_messages().await?;
    
    // Test 3: Handshake Protocol
    test_handshake_protocol().await?;
    
    // Test 4: Connection Management
    test_connection_management().await?;
    
    // Test 5: Tracker Communication (simulated)
    test_tracker_communication().await?;
    
    info!("All network tests completed successfully!");
    Ok(())
}

async fn test_network_manager_creation() -> Result<()> {
    info!("Testing network manager creation...");
    
    let config = Config::default();
    let network_manager = NetworkManager::new(config);
    
    assert_eq!(network_manager.config().listen_port, 6881);
    assert_eq!(network_manager.config().max_connections, 50);
    
    info!("✓ Network manager creation test passed");
    Ok(())
}

async fn test_protocol_messages() -> Result<()> {
    info!("Testing protocol message handling...");
    
    // Test message creation
    let have_message = Message::have(123);
    assert_eq!(have_message.message_type, MessageType::Have);
    
    let request_message = Message::request(1, 1024, 16384);
    assert_eq!(request_message.message_type, MessageType::Request);
    
    let piece_message = Message::piece(1, 1024, vec![1, 2, 3, 4, 5]);
    assert_eq!(piece_message.message_type, MessageType::Piece);
    
    // Test message serialization/deserialization
    let serialized = have_message.serialize();
    let deserialized = Message::deserialize(&serialized)?;
    assert_eq!(have_message.message_type, deserialized.message_type);
    assert_eq!(have_message.payload, deserialized.payload);
    
    // Test message parsing
    let piece_index = have_message.parse_have()?;
    assert_eq!(piece_index, 123);
    
    let (req_piece, req_offset, req_length) = request_message.parse_request()?;
    assert_eq!(req_piece, 1);
    assert_eq!(req_offset, 1024);
    assert_eq!(req_length, 16384);
    
    let (piece_piece, piece_offset, piece_data) = piece_message.parse_piece()?;
    assert_eq!(piece_piece, 1);
    assert_eq!(piece_offset, 1024);
    assert_eq!(piece_data, vec![1, 2, 3, 4, 5]);
    
    // Test message validation
    assert!(have_message.is_valid());
    assert!(request_message.is_valid());
    assert!(piece_message.is_valid());
    
    let invalid_message = Message::new(MessageType::Have, vec![1, 2, 3]); // Wrong length
    assert!(!invalid_message.is_valid());
    
    info!("✓ Protocol message handling test passed");
    Ok(())
}

async fn test_handshake_protocol() -> Result<()> {
    info!("Testing handshake protocol...");
    
    let info_hash = [1u8; 20];
    let peer_id = [2u8; 20];
    
    let handshake = Handshake::new(info_hash, peer_id);
    assert_eq!(handshake.info_hash, info_hash);
    assert_eq!(handshake.peer_id, peer_id);
    assert_eq!(handshake.protocol_identifier, *b"BitTorrent protocol");
    
    // Test serialization/deserialization
    let serialized = handshake.serialize();
    assert_eq!(serialized.len(), 68); // 1 + 19 + 8 + 20 + 20
    
    let deserialized = Handshake::deserialize(&serialized)?;
    assert_eq!(handshake.protocol_identifier, deserialized.protocol_identifier);
    assert_eq!(handshake.info_hash, deserialized.info_hash);
    assert_eq!(handshake.peer_id, deserialized.peer_id);
    
    info!("✓ Handshake protocol test passed");
    Ok(())
}

async fn test_connection_management() -> Result<()> {
    info!("Testing connection management...");
    
    let config = Config::default();
    let pool = ConnectionPool::new(config);
    
    assert_eq!(pool.connection_count().await, 0);
    
    // Test connection info creation
    let addr: SocketAddr = "127.0.0.1:6881".parse()?;
    let peer_id = [1u8; 20];
    let info_hash = [2u8; 20];
    
    let connection_info = ConnectionInfo::new(addr, peer_id, info_hash);
    assert_eq!(connection_info.addr, addr);
    assert_eq!(connection_info.peer_id, peer_id);
    assert_eq!(connection_info.info_hash, info_hash);
    assert_eq!(connection_info.state, ConnectionState::Connecting);
    
    // Test activity tracking
    let original_activity = connection_info.last_activity;
    sleep(Duration::from_millis(10)).await;
    
    let mut updated_info = ConnectionInfo::new(addr, peer_id, info_hash);
    updated_info.update_activity();
    assert!(updated_info.last_activity > original_activity);
    
    info!("✓ Connection management test passed");
    Ok(())
}

async fn test_tracker_communication() -> Result<()> {
    info!("Testing tracker communication...");
    
    let config = Config::default();
    let trackers = vec![
        "http://tracker.example.com/announce".to_string(),
        "http://tracker2.example.com/announce".to_string(),
    ];
    
    let mut tracker_manager = TrackerManager::new(config, trackers);
    assert_eq!(tracker_manager.trackers().len(), 2);
    
    // Test tracker event conversion
    assert_eq!(TrackerEvent::from("started"), TrackerEvent::Started);
    assert_eq!(TrackerEvent::from("stopped"), TrackerEvent::Stopped);
    assert_eq!(TrackerEvent::from("completed"), TrackerEvent::Completed);
    assert_eq!(TrackerEvent::from("unknown"), TrackerEvent::None);
    
    assert_eq!(<&str>::from(TrackerEvent::Started), "started");
    assert_eq!(<&str>::from(TrackerEvent::Stopped), "stopped");
    assert_eq!(<&str>::from(TrackerEvent::Completed), "completed");
    assert_eq!(<&str>::from(TrackerEvent::None), "");
    
    // Test tracker request creation
    let info_hash = [1u8; 20];
    let peer_id = [2u8; 20];
    let request = TrackerRequest::new(
        info_hash,
        peer_id,
        6881,
        1000,
        2000,
        3000,
        TrackerEvent::Started,
    );
    
    assert_eq!(request.info_hash, info_hash);
    assert_eq!(request.peer_id, peer_id);
    assert_eq!(request.port, 6881);
    assert_eq!(request.uploaded, 1000);
    assert_eq!(request.downloaded, 2000);
    assert_eq!(request.left, 3000);
    assert_eq!(request.event, TrackerEvent::Started);
    
    // Test query parameter generation
    let params = request.to_query_params();
    assert!(params.contains("info_hash="));
    assert!(params.contains("peer_id="));
    assert!(params.contains("port=6881"));
    assert!(params.contains("uploaded=1000"));
    assert!(params.contains("downloaded=2000"));
    assert!(params.contains("left=3000"));
    assert!(params.contains("event=started"));
    
    // Test tracker management
    tracker_manager.add_tracker("http://tracker3.example.com/announce".to_string());
    assert_eq!(tracker_manager.trackers().len(), 3);
    
    tracker_manager.remove_tracker("http://tracker2.example.com/announce");
    assert_eq!(tracker_manager.trackers().len(), 2);
    
    info!("✓ Tracker communication test passed");
    Ok(())
}

// Integration test: Simulate a complete network interaction
async fn test_network_integration() -> Result<()> {
    info!("Testing network integration...");
    
    // Create network manager
    let config = Config::default();
    let mut network_manager = NetworkManager::new(config);
    
    // Add torrent info
    let info_hash = [1u8; 20];
    let torrent_info = TorrentInfo::new(
        "test_torrent".to_string(),
        16384,
        vec![[0u8; 20], [1u8; 20], [2u8; 20]],
        vec![FileInfo::new(vec!["test.txt".to_string()], 49152)],
    );
    
    network_manager.add_torrent_info(info_hash, torrent_info).await?;
    
    // Create peer manager
    let peer_manager = network_manager.peer_manager();
    let peer_manager_guard = peer_manager.read().await;
            assert_eq!(peer_manager_guard.connected_peer_count(), 0);
    drop(peer_manager_guard);
    
    info!("✓ Network integration test passed");
    Ok(())
}

// Performance test: Test message throughput
async fn test_message_throughput() -> Result<()> {
    info!("Testing message throughput...");
    
    let start = std::time::Instant::now();
    let num_messages = 10000;
    
    for i in 0..num_messages {
        let message = Message::have(i);
        let serialized = message.serialize();
        let _deserialized = Message::deserialize(&serialized)?;
    }
    
    let duration = start.elapsed();
    let throughput = num_messages as f64 / duration.as_secs_f64();
    
    info!("Processed {} messages in {:?} ({:.2} msg/sec)", 
          num_messages, duration, throughput);
    
    assert!(throughput > 1000.0, "Throughput too low: {:.2} msg/sec", throughput);
    
    info!("✓ Message throughput test passed");
    Ok(())
}

// Error handling test
async fn test_error_handling() -> Result<()> {
    info!("Testing error handling...");
    
    // Test invalid message deserialization
    let invalid_data = vec![1, 2, 3, 4]; // Too short
    let result = Message::deserialize(&invalid_data);
    assert!(result.is_err());
    
    // Test invalid handshake deserialization
    let invalid_handshake = vec![1, 2, 3, 4]; // Wrong length
    let result = Handshake::deserialize(&invalid_handshake);
    assert!(result.is_err());
    
    // Test message parsing with wrong type
    let have_message = Message::have(123);
    let result = have_message.parse_request();
    assert!(result.is_err());
    
    info!("✓ Error handling test passed");
    Ok(())
} 