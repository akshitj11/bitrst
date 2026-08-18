//! Peer manager with connection limits and inventory relay.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use bitrst_core::ChainHandle;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::constants::{
    Network, DEFAULT_MAX_INBOUND, DEFAULT_MAX_OUTBOUND, MAX_PEER_EVENTS, MAX_PEER_REGISTRATIONS,
    PEER_SHUTDOWN_TIMEOUT,
};
use crate::error::NetError;
use crate::handshake::{ConnectionDirection, HandshakeConfig};
use crate::inbound_capacity::InboundCapacity;
use crate::message::{InvType, InventoryVector, Message};
use crate::peer::{spawn_peer, PeerCommand, PeerContext, PeerEvent};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PeerLifecycle {
    Handshaking,
    Ready,
}

struct PeerEntry {
    command_tx: mpsc::Sender<PeerCommand>,
    direction: ConnectionDirection,
    state: PeerLifecycle,
    _task: JoinHandle<()>,
}

struct PeerRegistration {
    addr: SocketAddr,
    direction: ConnectionDirection,
    command_tx: mpsc::Sender<PeerCommand>,
    task: JoinHandle<()>,
}

/// Outcome of attempting configured seed connections.
#[derive(Debug, PartialEq, Eq)]
pub struct SeedConnectReport {
    /// First seed that connected successfully, if any.
    pub connected: Option<SocketAddr>,
    /// Per-seed connection failures in attempt order.
    pub failures: Vec<(SocketAddr, NetError)>,
}

impl SeedConnectReport {
    /// Returns the connected seed address when one succeeded.
    ///
    /// # Errors
    ///
    /// Returns [`NetError::SeedsExhausted`] when every seed attempt failed.
    pub fn into_result(self) -> Result<SocketAddr, NetError> {
        if let Some(addr) = self.connected {
            return Ok(addr);
        }
        Err(NetError::SeedsExhausted {
            attempted: self.failures.len(),
            details: format_seed_failures(&self.failures),
        })
    }
}

fn format_seed_failures(failures: &[(SocketAddr, NetError)]) -> String {
    failures
        .iter()
        .map(|(addr, error)| format!("{addr}: {error}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Manages peer connections and relays inventory announcements.
pub struct PeerManager {
    chain: ChainHandle,
    config: PeerManagerConfig,
    outbound_count: usize,
    peers: HashMap<SocketAddr, PeerEntry>,
    listener: Option<TcpListener>,
    bound_listen_addr: Option<SocketAddr>,
    accept_handle: Option<JoinHandle<()>>,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
    shutting_down: bool,
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
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let inbound_capacity = InboundCapacity::new(config.max_inbound);
        Self {
            chain,
            config,
            outbound_count: 0,
            peers: HashMap::new(),
            listener: None,
            bound_listen_addr: None,
            accept_handle: None,
            shutdown_tx,
            shutdown_rx,
            shutting_down: false,
            event_tx,
            event_rx,
            register_tx,
            register_rx,
            inbound_capacity,
        }
    }

    /// Returns the bound listen address after [`Self::start_listener`].
    #[must_use]
    pub fn listen_addr(&self) -> Option<SocketAddr> {
        self.bound_listen_addr
    }

    /// Starts listening for inbound connections.
    ///
    /// # Errors
    ///
    /// Returns [`NetError::Io`] when binding fails.
    pub async fn start_listener(&mut self) -> Result<(), NetError> {
        let listener = TcpListener::bind(self.config.listen_addr)
            .await
            .map_err(|_| NetError::Io("bind listener"))?;
        self.bound_listen_addr = listener.local_addr().ok();
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
        let mut shutdown_rx = self.shutdown_rx.clone();
        let accept_handle = tokio::spawn(async move {
            loop {
                if *shutdown_rx.borrow() {
                    break;
                }
                let accept = tokio::select! {
                    changed = shutdown_rx.changed() => {
                        if changed.is_ok() && *shutdown_rx.borrow() {
                            break;
                        }
                        continue;
                    }
                    accept = listener.accept() => accept,
                };
                let Ok((stream, addr)) = accept else {
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
                    PeerContext::new(
                        addr,
                        ConnectionDirection::Inbound,
                        network,
                        chain.clone(),
                        config,
                        height,
                        event_tx.clone(),
                    )
                    .with_inbound_guard(guard),
                );
                let registration = PeerRegistration {
                    addr,
                    direction: ConnectionDirection::Inbound,
                    command_tx,
                    task,
                };
                if register_tx.send(registration).await.is_err() {
                    break;
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
            PeerContext::new(
                addr,
                ConnectionDirection::Outbound,
                self.config.network,
                self.chain.clone(),
                config,
                height,
                self.event_tx.clone(),
            ),
        );
        self.peers.insert(
            addr,
            PeerEntry {
                command_tx,
                direction: ConnectionDirection::Outbound,
                state: PeerLifecycle::Handshaking,
                _task: task,
            },
        );
        self.outbound_count += 1;
        Ok(())
    }

    /// Attempts each configured seed and returns a per-attempt report.
    pub async fn connect_seeds_report(&mut self) -> SeedConnectReport {
        let mut failures = Vec::new();
        for addr in self.config.seeds.addresses(self.config.network) {
            match self.connect_seed(addr).await {
                Ok(()) => {
                    return SeedConnectReport {
                        connected: Some(addr),
                        failures,
                    };
                }
                Err(error) => failures.push((addr, error)),
            }
        }
        SeedConnectReport {
            connected: None,
            failures,
        }
    }

    /// Connects to configured seeds sequentially.
    ///
    /// # Errors
    ///
    /// Returns [`NetError::SeedsExhausted`] when no seed address connects successfully.
    pub async fn connect_seeds(&mut self) -> Result<SocketAddr, NetError> {
        self.connect_seeds_report().await.into_result()
    }

    /// Broadcasts inventory to ready peers except `origin`.
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
            if origin == Some(*addr) || entry.state != PeerLifecycle::Ready {
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
            self.poll().await?;
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        Ok(())
    }

    /// Drains one pass of registrations and all currently queued peer events.
    ///
    /// # Errors
    ///
    /// Returns [`NetError`] when inventory relay fails.
    pub async fn poll(&mut self) -> Result<(), NetError> {
        self.process_registrations();
        while let Ok(event) = self.event_rx.try_recv() {
            self.handle_event(event).await?;
        }
        Ok(())
    }

    /// Drains pending peer events and relays block inventory.
    ///
    /// # Errors
    ///
    /// Returns [`NetError`] when inventory relay fails.
    pub async fn process_events(&mut self) -> Result<(), NetError> {
        self.poll().await
    }

    async fn handle_event(&mut self, event: PeerEvent) -> Result<(), NetError> {
        match event {
            PeerEvent::Announce { addr, items } => {
                let filtered: Vec<_> = items
                    .into_iter()
                    .filter(|item| item.inv_type == InvType::Block)
                    .collect();
                self.relay_inventory(Some(addr), filtered).await?;
            }
            PeerEvent::Ready { addr } => {
                if let Some(entry) = self.peers.get_mut(&addr) {
                    entry.state = PeerLifecycle::Ready;
                }
            }
            PeerEvent::Disconnected { addr, .. } => {
                if let Some(entry) = self.peers.remove(&addr) {
                    if entry.direction == ConnectionDirection::Outbound {
                        self.outbound_count = self.outbound_count.saturating_sub(1);
                    }
                }
            }
            PeerEvent::RegistrationRejected { .. } => {}
        }
        Ok(())
    }

    fn process_registrations(&mut self) {
        while let Ok(registration) = self.register_rx.try_recv() {
            if registration.direction == ConnectionDirection::Outbound
                && self.outbound_count >= self.config.max_outbound
            {
                registration.task.abort();
                let _ = self.event_tx.try_send(PeerEvent::RegistrationRejected {
                    addr: registration.addr,
                    error: NetError::ConnectionLimitReached,
                });
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
                    state: PeerLifecycle::Handshaking,
                    _task: registration.task,
                },
            );
        }
    }

    /// Returns the number of tracked peers in any lifecycle state.
    #[must_use]
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Returns the number of peers that completed handshake and can receive relays.
    #[must_use]
    pub fn ready_peer_count(&self) -> usize {
        self.peers
            .values()
            .filter(|entry| entry.state == PeerLifecycle::Ready)
            .count()
    }

    /// Returns the number of reserved inbound connection slots.
    #[must_use]
    pub fn inbound_reserved(&self) -> usize {
        self.inbound_capacity.reserved()
    }

    /// Stops the accept loop, signals peers to shut down, and awaits tasks.
    ///
    /// This method is idempotent.
    ///
    /// # Errors
    ///
    /// Returns [`NetError::TaskJoinFailed`] when a peer task does not finish in time.
    pub async fn shutdown(&mut self) -> Result<(), NetError> {
        if self.shutting_down {
            return Ok(());
        }
        self.shutting_down = true;
        let _ = self.shutdown_tx.send(true);

        if let Some(handle) = self.accept_handle.take() {
            let mut handle = handle;
            if timeout(PEER_SHUTDOWN_TIMEOUT, &mut handle).await.is_err() {
                handle.abort();
            }
        }

        let peer_tasks: Vec<_> = self
            .peers
            .drain()
            .map(|(_, entry)| (entry.command_tx, entry._task))
            .collect();

        for (command_tx, _) in &peer_tasks {
            let _ = command_tx.send(PeerCommand::Shutdown).await;
        }

        for (_, task) in peer_tasks {
            let mut task = task;
            match timeout(PEER_SHUTDOWN_TIMEOUT, &mut task).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => return Err(NetError::TaskJoinFailed),
                Err(_) => task.abort(),
            }
        }

        self.outbound_count = 0;
        Ok(())
    }
}

impl Drop for PeerManager {
    fn drop(&mut self) {
        if self.shutting_down {
            return;
        }
        let _ = self.shutdown_tx.send(true);
        if let Some(handle) = self.accept_handle.take() {
            handle.abort();
        }
        for (_, entry) in self.peers.drain() {
            entry._task.abort();
        }
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
    use crate::peer::{spawn_peer, PeerContext};
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
            PeerContext::new(
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
            ),
        );

        manager
            .drive_until(
                |manager| manager.ready_peer_count() > 0,
                Duration::from_secs(5),
            )
            .await
            .expect("drive");

        assert_eq!(manager.ready_peer_count(), 1);
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
            .drive_until(
                |manager| manager.ready_peer_count() > 0,
                Duration::from_secs(5),
            )
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

    #[tokio::test]
    async fn relay_skips_peers_until_ready() {
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
                seeds: SeedStrategy::Fixed(vec![]),
            },
        );
        manager.start_listener().await.expect("listen");
        manager.spawn_acceptor();

        sleep(Duration::from_millis(20)).await;
        let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .expect("connect");
        write_message_version_only(&mut stream, 88).await;

        manager
            .drive_until(|manager| manager.peer_count() > 0, Duration::from_secs(5))
            .await
            .expect("drive");
        assert_eq!(manager.ready_peer_count(), 0);

        manager
            .relay_inventory(
                None,
                vec![InventoryVector {
                    inv_type: InvType::Block,
                    hash: [0x11; 32],
                }],
            )
            .await
            .expect("relay");

        let read_result = tokio::time::timeout(
            Duration::from_millis(300),
            read_message(&mut stream, Network::Testnet),
        )
        .await;
        match read_result {
            Err(_) => {}
            Ok(Ok(message)) => assert_ne!(
                message.command, "inv",
                "inv must not be sent before handshake completes"
            ),
            Ok(Err(_)) => {}
        }
    }

    async fn write_message_version_only(stream: &mut TcpStream, nonce: u64) {
        use crate::codec::default_version_message;
        use crate::framing::write_message;
        use crate::message::Message;

        write_message(
            stream,
            Network::Testnet,
            &Message::version(default_version_message(nonce, 1, 0)),
        )
        .await
        .expect("write version");
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
    async fn start_listener_records_bound_addr_for_ephemeral_port() {
        let chain = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        let mut manager = PeerManager::new(
            chain,
            PeerManagerConfig {
                network: Network::Testnet,
                listen_addr: "127.0.0.1:0".parse().expect("addr"),
                max_inbound: 1,
                max_outbound: 0,
                seeds: SeedStrategy::Fixed(vec![]),
            },
        );
        manager.start_listener().await.expect("listen");
        let bound = manager.listen_addr().expect("bound addr");
        assert_ne!(bound.port(), 0);
        manager.shutdown().await.expect("shutdown");
    }

    #[tokio::test]
    async fn shutdown_is_idempotent_and_clears_peers() {
        let chain = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        let mut manager = PeerManager::new(
            chain,
            PeerManagerConfig {
                network: Network::Testnet,
                listen_addr: "127.0.0.1:0".parse().expect("addr"),
                max_inbound: 2,
                max_outbound: 0,
                seeds: SeedStrategy::Fixed(vec![]),
            },
        );
        manager.start_listener().await.expect("listen");
        manager.spawn_acceptor();
        manager.shutdown().await.expect("shutdown");
        assert_eq!(manager.peer_count(), 0);
        manager.shutdown().await.expect("idempotent");
    }

    #[tokio::test]
    async fn connect_seeds_report_collects_failures() {
        let chain = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        let mut manager = PeerManager::new(
            chain,
            PeerManagerConfig {
                network: Network::Testnet,
                listen_addr: "127.0.0.1:0".parse().expect("addr"),
                max_inbound: 0,
                max_outbound: 1,
                seeds: SeedStrategy::Fixed(vec!["127.0.0.1:1".parse().expect("addr")]),
            },
        );
        let report = manager.connect_seeds_report().await;
        assert!(report.connected.is_none());
        assert_eq!(report.failures.len(), 1);
        assert!(matches!(
            report.into_result(),
            Err(crate::error::NetError::SeedsExhausted { .. })
        ));
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
