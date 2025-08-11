use crate::core::{Config, Hash, PeerId, TorrentInfo};
use crate::peer::{Peer, PeerManager};
use crate::protocol::{
    messages::MessageParser, Handshake, HandshakeHandler, Message, ProtocolHandler,
};
use anyhow::{Context, Result};
use log::{debug, error, info, warn};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, RwLock};
use tokio::time::{timeout, Duration};

pub mod connection;
pub mod tracker;

pub use connection::*;
pub use tracker::*;

//=== Network manager for handling all network operations ===//
pub struct NetworkManager {
    config: Config,
    peer_manager: Arc<RwLock<PeerManager>>,
    torrent_info: Arc<RwLock<HashMap<Hash, TorrentInfo>>>,
    listener: Option<TcpListener>,
    shutdown_tx: mpsc::Sender<()>,
    shutdown_rx: mpsc::Receiver<()>,
}

impl NetworkManager {
    pub fn new(config: Config) -> Self {
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

        Self {
            config,
            peer_manager: Arc::new(RwLock::new(PeerManager::new(100, 50))),
            torrent_info: Arc::new(RwLock::new(HashMap::new())),
            listener: None,
            shutdown_tx,
            shutdown_rx,
        }
    }
    pub async fn start(&mut self) -> Result<()> {
        info!(
            "Starting network manager on port {}",
            self.config.listen_port
        );

        //=== Bind to the listening port ===//
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), self.config.listen_port);
        let listener = TcpListener::bind(addr)
            .await
            .with_context(|| format!("Failed to bind to port {}", self.config.listen_port))?;

        self.listener = Some(listener);

        self.accept_connections().await?;

        Ok(())
    }
    pub async fn stop(&mut self) -> Result<()> {
        info!("Stopping network manager");

        //=== Send shutdown signal ===//
        if let Err(e) = self.shutdown_tx.send(()).await {
            warn!("Failed to send shutdown signal: {}", e);
        }
        self.listener = None;

        Ok(())
    }

    //=== Accept incoming connections ===//
    async fn accept_connections(&mut self) -> Result<()> {
        let listener = self
            .listener
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Listener not initialized"))?;

        let peer_manager = Arc::clone(&self.peer_manager);
        let torrent_info = Arc::clone(&self.torrent_info);
        let config = self.config.clone();

        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((socket, addr)) => {
                            debug!("New connection from {}", addr);

                            //=== Spawn a task to handle the connection ===//
                            let peer_manager_clone = Arc::clone(&peer_manager);
                            let torrent_info_clone = Arc::clone(&torrent_info);
                            let config_clone = config.clone();

                            tokio::spawn(async move {
                                if let Err(e) = Self::handle_incoming_connection(
                                    socket,
                                    addr,
                                    peer_manager_clone,
                                    torrent_info_clone,
                                    config_clone
                                ).await {
                                    error!("Error handling connection from {}: {}", addr, e);
                                }
                            });
                        }
                        Err(e) => {
                            error!("Error accepting connection: {}", e);
                        }
                    }
                }

                _ = self.shutdown_rx.recv() => {
                    info!("Received shutdown signal");
                    break;
                }
            }
        }

        Ok(())
    }
    async fn handle_incoming_connection(
        socket: TcpStream,
        addr: SocketAddr,
        peer_manager: Arc<RwLock<PeerManager>>,
        torrent_info: Arc<RwLock<HashMap<Hash, TorrentInfo>>>,
        config: Config,
    ) -> Result<()> {
        let mut handshake_handler = HandshakeHandler::new(socket);

        let handshake_result = timeout(
            config.connection_timeout,
            Self::perform_handshake(&mut handshake_handler),
        )
        .await;

        let (_our_handshake, their_handshake) = match handshake_result {
            Ok(Ok(result)) => result,
            Ok(Err(e)) => {
                error!("Handshake failed with {}: {}", addr, e);
                return Err(e.into());
            }
            Err(_) => {
                error!("Handshake timeout with {}", addr);
                return Err(anyhow::anyhow!("Handshake timeout"));
            }
        };

        //=== Verify the  torrent info ===//
        let torrent_info_guard = torrent_info.read().await;
        if !torrent_info_guard.contains_key(&their_handshake.info_hash) {
            error!("Unknown torrent info hash from {}", addr);
            return Err(anyhow::anyhow!("Unknown torrent"));
        }
        let torrent_info = torrent_info_guard[&their_handshake.info_hash].clone();
        drop(torrent_info_guard);

        //=== Create peer connection ===//
        let stream = handshake_handler.into_stream();
        let protocol_handler = ProtocolHandler::new(stream);

        //=== Add peer to manager ===//
        let mut peer_manager_guard = peer_manager.write().await;
        let _peer = Peer::new(their_handshake.peer_id, addr, torrent_info.num_pieces());

        peer_manager_guard.add_peer(their_handshake.peer_id, addr)?;
        drop(peer_manager_guard);

        Self::handle_peer_connection(
            protocol_handler,
            format!("{:?}", their_handshake.peer_id),
            peer_manager,
            config,
        )
        .await?;

        Ok(())
    }

    //=== Perform handshake with a peer ===//
    async fn perform_handshake(
        handshake_handler: &mut HandshakeHandler,
    ) -> Result<(Handshake, Handshake)> {
        // make these info hash and peer ID would come from the torrent info
        let info_hash = [0u8; 20];
        let peer_id = [0u8; 20];

        handshake_handler
            .perform_handshake(info_hash, peer_id)
            .await
            .map_err(|e| anyhow::anyhow!("Handshake failed: {}", e))
    }

    //=== Handle an established peer connection ===//
    async fn handle_peer_connection(
        mut protocol_handler: ProtocolHandler,
        peer_id: String,
        peer_manager: Arc<RwLock<PeerManager>>,
        _config: Config,
    ) -> Result<()> {
        info!("Handling peer connection: {}", peer_id);

        loop {
            let message_result =
                timeout(Duration::from_secs(30), protocol_handler.receive_message()).await;

            match message_result {
                Ok(Ok(message)) => {
                    debug!(
                        "Received message from {}: {:?}",
                        peer_id, message.message_type
                    );

                    if let Err(e) = Self::handle_message(
                        &message,
                        &mut protocol_handler,
                        &peer_id,
                        &peer_manager,
                    )
                    .await
                    {
                        error!("Error handling message from {}: {}", peer_id, e);
                        break;
                    }
                }
                Ok(Err(e)) => {
                    error!("Error receiving message from {}: {}", peer_id, e);
                    break;
                }
                Err(_) => {
                    debug!("Keep-alive timeout for peer {}", peer_id);
                    if let Err(e) = protocol_handler.send_message(&Message::keep_alive()).await {
                        error!("Error sending keep-alive to {}: {}", peer_id, e);
                        break;
                    }
                }
            }
        }

        //== Remove peer from manager ==//
        info!("Peer connection closed: {}", peer_id);
        Ok(())
    }

    //== Handle a protocol message ==//
    async fn handle_message(
        message: &Message,
        protocol_handler: &mut ProtocolHandler,
        peer_id: &str,
        peer_manager: &Arc<RwLock<PeerManager>>,
    ) -> Result<()> {
        use crate::protocol::MessageType;

        match message.message_type {
            MessageType::Choke => {
                debug!("Peer {} choked us", peer_id);
            }

            MessageType::Unchoke => {
                debug!("Peer {} unchoked us", peer_id);
            }

            MessageType::Interested => {
                debug!("Peer {} is interested", peer_id);
            }

            MessageType::NotInterested => {
                debug!("Peer {} is not interested", peer_id);
            }

            MessageType::Have => {
                if let Ok(piece_index) = message.parse_have() {
                    debug!("Peer {} has piece {}", peer_id, piece_index);
                }
            }

            MessageType::Bitfield => {
                if let Ok(_bitfield_data) = message.parse_bitfield() {
                    debug!("Peer {} sent bitfield", peer_id);
                }
            }

            MessageType::Request => {
                if let Ok((piece_index, offset, length)) = message.parse_request() {
                    debug!(
                        "Peer {} requested piece {} offset {} length {}",
                        peer_id, piece_index, offset, length
                    );
                    //=== Handle piece request ===//
                    Self::handle_piece_request(protocol_handler, piece_index, offset, length)
                        .await?;
                }
            }

            MessageType::Piece => {
                if let Ok((piece_index, offset, data)) = message.parse_piece() {
                    debug!(
                        "Peer {} sent piece {} offset {} length {}",
                        peer_id,
                        piece_index,
                        offset,
                        data.len()
                    );
                    //=== Handle received piece data ===//
                    Self::handle_piece_data(peer_id, piece_index, offset, data, peer_manager)
                        .await?;
                }
            }

            MessageType::Cancel => {
                debug!("Peer {} cancelled request", peer_id);
            }

            MessageType::Port => {
                if let Ok(port) = message.parse_port() {
                    debug!("Peer {} announced port {}", peer_id, port);
                }
            }

            MessageType::KeepAlive => {}
        }

        Ok(())
    }

    async fn handle_piece_request(
        protocol_handler: &mut ProtocolHandler,
        piece_index: crate::core::PieceIndex,
        offset: crate::core::BlockOffset,
        length: crate::core::BlockLength,
    ) -> Result<()> {
        //=== this data would read from the actual file ===//
        let dummy_data = vec![0u8; length as usize];

        let piece_message = Message::piece(piece_index, offset, dummy_data);
        protocol_handler
            .send_message(&piece_message)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send piece: {}", e))?;

        Ok(())
    }

    //=== Handle received piece data ===//
    async fn handle_piece_data(
        _peer_id: &str,
        piece_index: crate::core::PieceIndex,
        offset: crate::core::BlockOffset,
        data: Vec<u8>,
        _peer_manager: &Arc<RwLock<PeerManager>>,
    ) -> Result<()> {
        //==  this received data to be stored and verified ==//
        debug!(
            "Received {} bytes for piece {} offset {} from peer",
            data.len(),
            piece_index,
            offset
        );

        Ok(())
    }

    //== Connect to a peer ==//
    pub async fn connect_to_peer(
        &self,
        addr: SocketAddr,
        info_hash: Hash,
        peer_id: PeerId,
    ) -> Result<()> {
        info!("Connecting to peer at {}", addr);

        //=== Connect to the peer ===//
        let stream = TcpStream::connect(addr)
            .await
            .with_context(|| format!("Failed to connect to {}", addr))?;

        let mut handshake_handler = HandshakeHandler::new(stream);

        let (_our_handshake, their_handshake) = handshake_handler
            .perform_handshake(info_hash, peer_id)
            .await
            .with_context(|| format!("Handshake failed with {}", addr))?;

        //=== Create protocol handler ===//
        let stream = handshake_handler.into_stream();
        let protocol_handler = ProtocolHandler::new(stream);

        let mut peer_manager_guard = self.peer_manager.write().await;

        //=== Get torrent info ===//
        let torrent_info_guard = self.torrent_info.read().await;
        let torrent_info = torrent_info_guard
            .get(&info_hash)
            .ok_or_else(|| anyhow::anyhow!("Unknown torrent info hash"))?
            .clone();
        drop(torrent_info_guard);

        let _peer = Peer::new(their_handshake.peer_id, addr, torrent_info.num_pieces());

        peer_manager_guard.add_peer(their_handshake.peer_id, addr)?;
        drop(peer_manager_guard);

        //==== Handle the connection ====//
        let peer_manager_clone = Arc::clone(&self.peer_manager);
        let config_clone = self.config.clone();

        tokio::spawn(async move {
            if let Err(e) = Self::handle_peer_connection(
                protocol_handler,
                format!("{:?}", their_handshake.peer_id),
                peer_manager_clone,
                config_clone,
            )
            .await
            {
                error!("Error handling outgoing connection: {}", e);
            }
        });

        Ok(())
    }

    pub async fn add_torrent_info(&self, info_hash: Hash, torrent_info: TorrentInfo) -> Result<()> {
        let mut torrent_info_guard = self.torrent_info.write().await;
        torrent_info_guard.insert(info_hash, torrent_info);
        Ok(())
    }
    pub fn peer_manager(&self) -> Arc<RwLock<PeerManager>> {
        Arc::clone(&self.peer_manager)
    }

    //=== Get configuration ===//
    pub fn config(&self) -> &Config {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::TorrentInfo;

    #[tokio::test]
    async fn test_network_manager_creation() {
        let config = Config::default();
        let network_manager = NetworkManager::new(config);

        assert_eq!(network_manager.config().listen_port, 6881);
    }

    #[tokio::test]
    async fn test_add_torrent_info() {
        let config = Config::default();
        let network_manager = NetworkManager::new(config);

        let info_hash = [1u8; 20];
        let torrent_info = TorrentInfo::new("test".to_string(), 16384, vec![[0u8; 20]], vec![]);

        network_manager
            .add_torrent_info(info_hash, torrent_info)
            .await
            .unwrap();

        let torrent_info_guard = network_manager.torrent_info.read().await;
        assert!(torrent_info_guard.contains_key(&info_hash));
    }
}
