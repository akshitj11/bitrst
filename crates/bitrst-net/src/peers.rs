//! Peer manager with connection limits and inventory relay.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use bitrst_core::ChainHandle;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::constants::{
    Network, DEFAULT_MAX_INBOUND, DEFAULT_MAX_OUTBOUND, MAX_PEER_EVENTS, MAX_PEER_REGISTRATIONS,
};
use crate::error::NetError;
use crate::handshake::{ConnectionDirection, HandshakeConfig};
use crate::inbound_capacity::InboundCapacity;
use crate::message::{InvType, InventoryVector, Message};
use crate::peer::{spawn_peer, PeerCommand, PeerEvent};
use crate::seeds::SeedStrategy;

/// Configuration for the peer manager.
#[derive(Debug, Clone)]
pub struct PeerManagerConfig {
    /// Network magic and handshake parameters.
    pub network: Network,
    /// Address to bind for inbound connections.
    pub listen_addr: SocketAddr,
    /// Maximum inbound peers.
    pub max_inbound: usize,
    /// Maximum outbound peers.
    pub max_outbound: usize,
    /// Seed selection strategy.
    pub seeds: SeedStrategy,
}

impl PeerManagerConfig {
    /// Builds a localhost test configuration.
    #[must_use]
    pub fn localhost_test(port: u16) -> Self {
        Self {
            network: Network::Testnet,
            listen_addr: SocketAddr::from(([127, 0, 0, 1], port)),
            max_inbound: DEFAULT_MAX_INBOUND,
            max_outbound: DEFAULT_MAX_OUTBOUND,
            seeds: SeedStrategy::localhost(port.saturating_add(1)),
        }
    }
}

struct PeerEntry {
    command_tx: mpsc::Sender<PeerCommand>,
    direction: ConnectionDirection,
    _task: JoinHandle<()>,
}

struct PeerRegistration {
    addr: SocketAddr,
    direction: ConnectionDirection,
    command_tx: mpsc::Sender<PeerCommand>,
    task: JoinHandle<()>,
}

/// Manages peer connections and relays inventory announcements.
pub struct PeerManager {
    chain: ChainHandle,
    config: PeerManagerConfig,
    outbound_count: usize,
    peers: HashMap<SocketAddr, PeerEntry>,
    listener: Option<TcpListener>,
    accept_handle: Option<JoinHandle<()>>,
    event_tx: mpsc::Sender<PeerEvent>,
    event_rx: mpsc::Receiver<PeerEvent>,
    register_tx: mpsc::Sender<PeerRegistration>,
    register_rx: mpsc::Receiver<PeerRegistration>,
    inbound_capacity: Arc<InboundCapacity>,
}

impl PeerManager {
    /// Creates a peer manager bound to `config` and sharing `chain`.
    #[must_use]
    pub fn new(chain: ChainHandle, config: PeerManagerConfig) -> Self {
        let (event_tx, event_rx) = mpsc::channel(MAX_PEER_EVENTS);
        let (register_tx, register_rx) = mpsc::channel(MAX_PEER_REGISTRATIONS);
        let inbound_capacity = InboundCapacity::new(config.max_inbound);
        Self {
            chain,
            config,
            outbound_count: 0,
            peers: HashMap::new(),
            listener: None,
            accept_handle: None,
            event_tx,
            event_rx,
            register_tx,
            register_rx,
            inbound_capacity,
        }
    }

    /// Starts listening for inbound peers.
    ///
    /// # Errors
    ///
    /// Returns [`NetError::Io`] when binding fails.
    pub async fn start_listener(&mut self) -> Result<(), NetError> {
        let listener = TcpListener::bind(self.config.listen_addr)
            .await
            .map_err(|_| NetError::Io("bind listener"))?;
        self.listener = Some(listener);
        Ok(())
    }

    /// Spawns the accept loop for inbound connections.
    pub fn spawn_acceptor(&mut self) {
        let Some(listener) = self.listener.take() else {
            return;
        };
        let event_tx = self.event_tx.clone();
        let register_tx = self.register_tx.clone();
        let chain = self.chain.clone();
        let network = self.config.network;
        let inbound_capacity = Arc::clone(&self.inbound_capacity);
        let accept_handle = tokio::spawn(async move {
            loop {
                let Ok((stream, addr)) = listener.accept().await else {
                    break;
                };
                let Some(guard) = inbound_capacity.try_acquire() else {
                    drop(stream);
                    continue;
                };
                let config = HandshakeConfig::default();
                let height = chain.height().unwrap_or(0) as i32;
                let (command_tx, task) = spawn_peer(
                    stream,
                    addr,
                    ConnectionDirection::Inbound,
                    network,
                    chain.clone(),
                    config,
                    height,
                    event_tx.clone(),
                    Some(guard),
                );
                let registration = PeerRegistration {
                    addr,
                    direction: ConnectionDirection::Inbound,
                    command_tx,
                    task,
                };
                if let Err(error) = register_tx.try_send(registration) {
                    error.into_inner().task.abort();
                }
            }
        });
        self.accept_handle = Some(accept_handle);
    }

    /// Connects to one outbound seed when under the outbound limit.
    ///
    /// # Errors
    ///
    /// Returns [`NetError`] on connection failure or limit exhaustion.
    pub async fn connect_seed(&mut self, addr: SocketAddr) -> Result<(), NetError> {
        if self.outbound_count >= self.config.max_outbound {
            return Err(NetError::ConnectionLimitReached);
        }
        if self.peers.contains_key(&addr) {
            return Ok(());
        }

        let stream = tokio::net::TcpStream::connect(addr)
            .await
            .map_err(|_| NetError::Io("connect"))?;
        let config = HandshakeConfig::default();
        let height = self.chain.height().unwrap_or(0) as i32;
        let (command_tx, task) = spawn_peer(
            stream,
            addr,
            ConnectionDirection::Outbound,
            self.config.network,
            self.chain.clone(),
            config,
            height,
            self.event_tx.clone(),
            None,
        );
        self.peers.insert(
            addr,
            PeerEntry {
                command_tx,
                direction: ConnectionDirection::Outbound,
                _task: task,
            },
        );
        self.outbound_count += 1;
        Ok(())
    }

    /// Connects to configured seeds sequentially.
    ///
    /// # Errors
    ///
    /// Returns [`NetError`] when no seed address connects successfully.
    pub async fn connect_seeds(&mut self) -> Result<(), NetError> {
        for addr in self.config.seeds.addresses(self.config.network) {
            if self.connect_seed(addr).await.is_ok() {
                return Ok(());
            }
        }
        Err(NetError::Io("no seeds connected"))
    }

    /// Broadcasts inventory to all connected peers except `origin`.
    ///
    /// # Errors
    ///
    /// Returns [`NetError`] when a peer send channel is closed.
    pub async fn relay_inventory(
        &self,
        origin: Option<SocketAddr>,
        items: Vec<InventoryVector>,
    ) -> Result<(), NetError> {
        if items.is_empty() {
            return Ok(());
        }
        let message = Message::inv(items);
        for (addr, entry) in &self.peers {
            if origin == Some(*addr) {
                continue;
            }
            entry
                .command_tx
                .send(PeerCommand::Send(message.clone()))
                .await
                .map_err(|_| NetError::ConnectionClosed)?;
        }
        Ok(())
    }

    /// Drains registrations and events until `condition` holds or `timeout` elapses.
    ///
    /// # Errors
    ///
    /// Returns [`NetError`] when inventory relay fails.
    pub async fn drive_until(
        &mut self,
        mut condition: impl FnMut(&Self) -> bool,
        timeout: std::time::Duration,
    ) -> Result<(), NetError> {
        let deadline = tokio::time::Instant::now() + timeout;
        while !condition(self) {
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            self.process_events().await?;
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        Ok(())
    }

    /// Drains pending peer events and relays block inventory.
    ///
    /// # Errors
    ///
    /// Returns [`NetError`] when inventory relay fails.
    pub async fn process_events(&mut self) -> Result<(), NetError> {
        self.process_registrations();
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                PeerEvent::Announce { addr, items } => {
                    let filtered: Vec<_> = items
                        .into_iter()
                        .filter(|item| item.inv_type == InvType::Block)
                        .collect();
                    self.relay_inventory(Some(addr), filtered).await?;
                }
                PeerEvent::HandshakeComplete { .. } => {}
                PeerEvent::Disconnected { addr, .. } => {
                    if let Some(entry) = self.peers.remove(&addr) {
                        if entry.direction == ConnectionDirection::Outbound {
                            self.outbound_count = self.outbound_count.saturating_sub(1);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn process_registrations(&mut self) {
        while let Ok(registration) = self.register_rx.try_recv() {
            if registration.direction == ConnectionDirection::Outbound
                && self.outbound_count >= self.config.max_outbound
            {
                registration.task.abort();
                continue;
            }
            if registration.direction == ConnectionDirection::Outbound {
                self.outbound_count += 1;
            }
            self.peers.insert(
                registration.addr,
                PeerEntry {
                    command_tx: registration.command_tx,
                    direction: registration.direction,
                    _task: registration.task,
                },
            );
        }
    }

    /// Returns the number of tracked peers.
    #[must_use]
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Returns the number of reserved inbound connection slots.
    #[must_use]
    pub fn inbound_reserved(&self) -> usize {
        self.inbound_capacity.reserved()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bitrst_core::ChainHandle;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpStream;
    use tokio::time::sleep;

    use super::{PeerManager, PeerManagerConfig};
    use crate::codec::{decode_inv, encode_inv};
    use crate::constants::Network;
    use crate::framing::read_message;
    use crate::handshake::{ConnectionDirection, HandshakeConfig};
    use crate::message::{InvType, InventoryVector};
    use crate::peer::spawn_peer;
    use crate::seeds::SeedStrategy;
    use crate::testutil::{genesis_block, NETWORK_TIME};

    fn event_channel() -> (
        tokio::sync::mpsc::Sender<crate::peer::PeerEvent>,
        tokio::sync::mpsc::Receiver<crate::peer::PeerEvent>,
    ) {
        tokio::sync::mpsc::channel(crate::constants::MAX_PEER_EVENTS)
    }

    #[tokio::test]
    async fn peer_manager_registers_inbound_peer() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);

        let chain = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        let mut manager = PeerManager::new(
            chain.clone(),
            PeerManagerConfig {
                network: Network::Testnet,
                listen_addr: format!("127.0.0.1:{port}").parse().expect("addr"),
                max_inbound: 4,
                max_outbound: 4,
                seeds: SeedStrategy::localhost(port.saturating_add(1)),
            },
        );
        manager.start_listener().await.expect("listen");
        manager.spawn_acceptor();

        sleep(Duration::from_millis(20)).await;
        let stream = TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .expect("connect");
        let (event_tx, _event_rx) = event_channel();
        let (_cmd, handle) = spawn_peer(
            stream,
            format!("127.0.0.1:{port}").parse().expect("addr"),
            ConnectionDirection::Outbound,
            Network::Testnet,
            chain,
            HandshakeConfig {
                local_nonce: 99,
                timeout: Duration::from_secs(5),
            },
            0,
            event_tx,
            None,
        );

        manager
            .drive_until(|manager| manager.peer_count() > 0, Duration::from_secs(5))
            .await
            .expect("drive");

        assert_eq!(manager.peer_count(), 1);
        handle.abort();
    }

    #[tokio::test]
    async fn peer_manager_relays_inv_bytes_to_connected_peer() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);

        let chain = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        let mut manager = PeerManager::new(
            chain,
            PeerManagerConfig {
                network: Network::Testnet,
                listen_addr: format!("127.0.0.1:{port}").parse().expect("addr"),
                max_inbound: 4,
                max_outbound: 4,
                seeds: SeedStrategy::localhost(port.saturating_add(1)),
            },
        );
        manager.start_listener().await.expect("listen");
        manager.spawn_acceptor();

        sleep(Duration::from_millis(20)).await;
        let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .expect("connect");
        client_handshake(&mut stream, 77).await;

        manager
            .drive_until(|manager| manager.peer_count() > 0, Duration::from_secs(5))
            .await
            .expect("drive");

        let hash = [0x42u8; 32];
        let expected_items = vec![InventoryVector {
            inv_type: InvType::Block,
            hash,
        }];
        manager
            .relay_inventory(None, expected_items.clone())
            .await
            .expect("relay");

        let message = tokio::time::timeout(
            Duration::from_secs(5),
            read_message(&mut stream, Network::Testnet),
        )
        .await
        .expect("read timeout")
        .expect("read inv");
        assert_eq!(message.command, "inv");
        match message.payload {
            crate::message::MessagePayload::Inv(items) => {
                assert_eq!(items, expected_items);
                let payload = encode_inv(&items).expect("encode inv");
                assert_eq!(decode_inv(&payload).expect("decode inv"), items);
            }
            other => panic!("expected inv payload, got {other:?}"),
        }
    }

    async fn client_handshake(stream: &mut TcpStream, nonce: u64) {
        use crate::codec::default_version_message;
        use crate::framing::{read_message, write_message};
        use crate::message::{Message, MessagePayload};

        write_message(
            stream,
            Network::Testnet,
            &Message::version(default_version_message(nonce, 1, 0)),
        )
        .await
        .expect("write version");
        match read_message(stream, Network::Testnet)
            .await
            .expect("peer version")
            .payload
        {
            MessagePayload::Version(_) => {}
            other => panic!("expected version, got {other:?}"),
        }
        assert_eq!(
            read_message(stream, Network::Testnet)
                .await
                .expect("verack")
                .command,
            "verack"
        );
        write_message(stream, Network::Testnet, &Message::verack())
            .await
            .expect("write verack");
    }

    #[tokio::test]
    async fn connect_seed_respects_outbound_limit() {
        let chain = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        let mut manager = PeerManager::new(
            chain,
            PeerManagerConfig {
                network: Network::Testnet,
                listen_addr: "127.0.0.1:0".parse().expect("addr"),
                max_inbound: 0,
                max_outbound: 0,
                seeds: SeedStrategy::Fixed(vec![]),
            },
        );
        assert_eq!(
            manager
                .connect_seed("127.0.0.1:1".parse().expect("addr"))
                .await,
            Err(crate::error::NetError::ConnectionLimitReached)
        );
    }

    #[tokio::test]
    async fn inbound_flood_respects_capacity_before_handshake() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);

        const MAX_INBOUND: usize = 2;
        let chain = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        let mut manager = PeerManager::new(
            chain,
            PeerManagerConfig {
                network: Network::Testnet,
                listen_addr: format!("127.0.0.1:{port}").parse().expect("addr"),
                max_inbound: MAX_INBOUND,
                max_outbound: 0,
                seeds: SeedStrategy::Fixed(vec![]),
            },
        );
        manager.start_listener().await.expect("listen");
        manager.spawn_acceptor();

        sleep(Duration::from_millis(20)).await;

        let mut clients = Vec::new();
        for _ in 0..8 {
            if let Ok(stream) = TcpStream::connect(format!("127.0.0.1:{port}")).await {
                clients.push(stream);
            }
        }

        manager
            .drive_until(
                |manager| {
                    manager.peer_count() == MAX_INBOUND && manager.inbound_reserved() <= MAX_INBOUND
                },
                Duration::from_secs(5),
            )
            .await
            .expect("drive");

        assert_eq!(manager.peer_count(), MAX_INBOUND);
        assert!(manager.inbound_reserved() <= MAX_INBOUND);

        for mut stream in clients {
            let _ = stream.shutdown().await;
        }
    }
}
