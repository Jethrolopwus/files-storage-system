use crate::core::{Config, Hash, PeerId};
use crate::protocol::{HandshakeHandler, Message, ProtocolHandler};
use anyhow::{Context, Result};
use log::{error, info, warn};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tokio::time::{timeout, Duration};

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionState {
    Connecting,
    Handshaking,
    Connected,
    Disconnected,
    Failed,
}

#[derive(Debug)]
pub struct ConnectionInfo {
    pub addr: SocketAddr,
    pub peer_id: PeerId,
    pub info_hash: Hash,
    pub state: ConnectionState,
    pub connected_at: std::time::Instant,
    pub last_activity: std::time::Instant,
}

impl ConnectionInfo {
    pub fn new(addr: SocketAddr, peer_id: PeerId, info_hash: Hash) -> Self {
        let now = std::time::Instant::now();
        Self {
            addr,
            peer_id,
            info_hash,
            state: ConnectionState::Connecting,
            connected_at: now,
            last_activity: now,
        }
    }

    pub fn update_activity(&mut self) {
        self.last_activity = std::time::Instant::now();
    }

    pub fn is_stale(&self, timeout: Duration) -> bool {
        self.last_activity.elapsed() > timeout
    }
}

//=== Connection manager for  individual peer connections ===//
pub struct ConnectionManager {
    config: Config,
    connection_info: Arc<RwLock<ConnectionInfo>>,
    protocol_handler: Option<ProtocolHandler>,
}

impl ConnectionManager {
    pub fn new(config: Config, connection_info: ConnectionInfo) -> Self {
        Self {
            config,
            connection_info: Arc::new(RwLock::new(connection_info)),
            protocol_handler: None,
        }
    }
    pub async fn connect(&mut self) -> Result<()> {
        let mut info_guard = self.connection_info.write().await;
        info_guard.state = ConnectionState::Connecting;
        drop(info_guard);

        let addr = self.connection_info.read().await.addr;
        let stream = TcpStream::connect(addr)
            .await
            .with_context(|| format!("Failed to connect to {}", addr))?;

        self.perform_handshake(stream).await?;

        Ok(())
    }

    //=== Perform handshake with the peer ===//
    async fn perform_handshake(&mut self, stream: TcpStream) -> Result<()> {
        let mut info_guard = self.connection_info.write().await;
        info_guard.state = ConnectionState::Handshaking;
        drop(info_guard);

        let mut handshake_handler = HandshakeHandler::new(stream);

        //=== Perform handshake with timeout ===//
        let handshake_result = timeout(
            self.config.connection_timeout,
            handshake_handler.perform_handshake(
                self.connection_info.read().await.info_hash,
                self.connection_info.read().await.peer_id,
            ),
        )
        .await;

        let (_our_handshake, their_handshake) = match handshake_result {
            Ok(Ok(result)) => result,
            Ok(Err(e)) => {
                error!("Handshake failed: {}", e);
                self.set_state(ConnectionState::Failed).await;
                return Err(e.into());
            }
            Err(_) => {
                error!("Handshake timeout");
                self.set_state(ConnectionState::Failed).await;
                return Err(anyhow::anyhow!("Handshake timeout"));
            }
        };

        //=== Verify handshake ===//
        if their_handshake.info_hash != self.connection_info.read().await.info_hash {
            error!("Info hash mismatch in handshake");
            self.set_state(ConnectionState::Failed).await;
            return Err(anyhow::anyhow!("Info hash mismatch"));
        }

        //=== Create protocol handler ===//
        let stream = handshake_handler.into_stream();
        self.protocol_handler = Some(ProtocolHandler::new(stream));

        self.set_state(ConnectionState::Connected).await;

        info!(
            "Successfully connected to peer at {}",
            self.connection_info.read().await.addr
        );

        Ok(())
    }

    //==== Send a message to the peer ====//
    pub async fn send_message(&mut self, message: &Message) -> Result<()> {
        if let Some(protocol_handler) = &mut self.protocol_handler {
            protocol_handler.send_message(message).await?;
            self.update_activity().await;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Not connected"))
        }
    }

    //=== Receive a message from the peer ===//
    pub async fn receive_message(&mut self) -> Result<Message> {
        if let Some(protocol_handler) = &mut self.protocol_handler {
            let message = protocol_handler.receive_message().await?;
            self.update_activity().await;
            Ok(message)
        } else {
            Err(anyhow::anyhow!("Not connected"))
        }
    }

    //==== Receive a message with timeout ====//
    pub async fn receive_message_timeout(&mut self, timeout_duration: Duration) -> Result<Message> {
        if let Some(protocol_handler) = &mut self.protocol_handler {
            let message_result =
                timeout(timeout_duration, protocol_handler.receive_message()).await;

            match message_result {
                Ok(Ok(message)) => {
                    self.update_activity().await;
                    Ok(message)
                }
                Ok(Err(e)) => {
                    error!("Error receiving message: {}", e);
                    self.set_state(ConnectionState::Failed).await;
                    Err(e.into())
                }
                Err(_) => {
                    error!("Receive timeout");
                    self.set_state(ConnectionState::Failed).await;
                    Err(anyhow::anyhow!("Receive timeout"))
                }
            }
        } else {
            Err(anyhow::anyhow!("Not connected"))
        }
    }

    pub async fn disconnect(&mut self) -> Result<()> {
        info!(
            "Disconnecting from peer at {}",
            self.connection_info.read().await.addr
        );

        self.set_state(ConnectionState::Disconnected).await;
        self.protocol_handler = None;

        Ok(())
    }

    //==== Check for active connection ====//
    pub async fn is_active(&self) -> bool {
        let info_guard = self.connection_info.read().await;
        info_guard.state == ConnectionState::Connected
            && !info_guard.is_stale(self.config.connection_timeout)
    }

    pub async fn connection_info(&self) -> ConnectionInfo {
        let guard = self.connection_info.read().await;
        ConnectionInfo {
            addr: guard.addr,
            peer_id: guard.peer_id,
            info_hash: guard.info_hash,
            state: guard.state.clone(),
            connected_at: guard.connected_at,
            last_activity: guard.last_activity,
        }
    }

    async fn update_activity(&self) {
        let mut info_guard = self.connection_info.write().await;
        info_guard.update_activity();
    }

    async fn set_state(&self, state: ConnectionState) {
        let mut info_guard = self.connection_info.write().await;
        info_guard.state = state;
    }

    //==== Get  peer address ====//
    pub async fn peer_addr(&self) -> SocketAddr {
        self.connection_info.read().await.addr
    }

    //==== Get  peer ID ====//
    pub async fn peer_id(&self) -> PeerId {
        self.connection_info.read().await.peer_id
    }

    //==== Get the info hash  ====//
    pub async fn info_hash(&self) -> Hash {
        self.connection_info.read().await.info_hash
    }
}

//=== pool of connections for managing multiple connections ===//
pub struct ConnectionPool {
    config: Config,
    connections: Arc<RwLock<std::collections::HashMap<SocketAddr, ConnectionManager>>>,
}

impl ConnectionPool {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            connections: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Add a connection to the pool
    pub async fn add_connection(
        &self,
        addr: SocketAddr,
        peer_id: PeerId,
        info_hash: Hash,
    ) -> Result<()> {
        let connection_info = ConnectionInfo::new(addr, peer_id, info_hash);
        let mut connection_manager = ConnectionManager::new(self.config.clone(), connection_info);

        //=== set  connections ===//
        connection_manager.connect().await?;

        //=== Add to pool ===//
        let mut connections_guard = self.connections.write().await;
        connections_guard.insert(addr, connection_manager);

        info!("Added connection to pool: {}", addr);
        Ok(())
    }
    pub async fn remove_connection(&self, addr: &SocketAddr) -> Result<()> {
        let mut connections_guard = self.connections.write().await;

        if let Some(mut connection) = connections_guard.remove(addr) {
            connection.disconnect().await?;
            info!("Removed connection from pool: {}", addr);
        }

        Ok(())
    }

    //==== Get a connection from the pool ===//
    pub async fn get_connection(&self, addr: &SocketAddr) -> bool {
        let connections_guard = self.connections.read().await;
        connections_guard.contains_key(addr)
    }

    //==== Get all active connections count ====//
    pub async fn get_active_connections_count(&self) -> usize {
        let connections_guard = self.connections.read().await;
        connections_guard.len()
    }

    //=== Clean up stale connections ===//
    pub async fn cleanup_stale_connections(&self) -> Result<()> {
        let mut connections_guard = self.connections.write().await;
        let stale_addrs: Vec<SocketAddr> = connections_guard
            .iter()
            .filter(|(_, conn)| {
                let _info = conn.connection_info();
                false
            })
            .map(|(addr, _)| *addr)
            .collect();

        for addr in stale_addrs {
            if let Some(mut connection) = connections_guard.remove(&addr) {
                connection.disconnect().await?;
                info!("Removed stale connection: {}", addr);
            }
        }

        Ok(())
    }

    //=== Get connection count ===//
    pub async fn connection_count(&self) -> usize {
        self.connections.read().await.len()
    }

    //==== Close all connections ====//
    pub async fn close_all(&self) -> Result<()> {
        let mut connections_guard = self.connections.write().await;

        for (addr, mut connection) in connections_guard.drain() {
            if let Err(e) = connection.disconnect().await {
                warn!("Error disconnecting from {}: {}", addr, e);
            }
        }

        info!("Closed all connections");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_connection_info_creation() {
        let addr = "127.0.0.1:6881".parse().unwrap();
        let peer_id = [1u8; 20];
        let info_hash = [2u8; 20];

        let info = ConnectionInfo::new(addr, peer_id, info_hash);

        assert_eq!(info.addr, addr);
        assert_eq!(info.peer_id, peer_id);
        assert_eq!(info.info_hash, info_hash);
        assert_eq!(info.state, ConnectionState::Connecting);
    }

    #[tokio::test]
    async fn test_connection_info_activity() {
        let addr = "127.0.0.1:6881".parse().unwrap();
        let peer_id = [1u8; 20];
        let info_hash = [2u8; 20];

        let mut info = ConnectionInfo::new(addr, peer_id, info_hash);
        let original_activity = info.last_activity;

        std::thread::sleep(std::time::Duration::from_millis(10));
        info.update_activity();

        assert!(info.last_activity > original_activity);
    }

    #[tokio::test]
    async fn test_connection_pool_creation() {
        let config = Config::default();
        let pool = ConnectionPool::new(config);

        assert_eq!(pool.connection_count().await, 0);
    }
}
