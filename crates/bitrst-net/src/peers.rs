//! Peer manager with connection limits and inventory relay.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};

use bitrst_core::ChainHandle;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::constants::{Network, DEFAULT_MAX_INBOUND, DEFAULT_MAX_OUTBOUND};
use crate::error::NetError;
use crate::handshake::{ConnectionDirection, HandshakeConfig};
use crate::message::{InvType, InventoryVector, Message};
use crate::peer::{spawn_peer, PeerCommand, PeerEvent};
use crate::seeds::SeedStrategy;

static NONCE_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    _task: JoinHandle<()>,
}

/// Manages peer connections and relays inventory announcements.
pub struct PeerManager {
    chain: ChainHandle,
    config: PeerManagerConfig,
    outbound_count: usize,
    peers: HashMap<SocketAddr, PeerEntry>,
    listener: Option<TcpListener>,
    accept_handle: Option<JoinHandle<()>>,
    event_tx: mpsc::UnboundedSender<PeerEvent>,
    event_rx: mpsc::UnboundedReceiver<PeerEvent>,
}

impl PeerManager {
    /// Creates a peer manager bound to `config` and sharing `chain`.
    #[must_use]
    pub fn new(chain: ChainHandle, config: PeerManagerConfig) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        Self {
            chain,
            config,
            outbound_count: 0,
            peers: HashMap::new(),
            listener: None,
            accept_handle: None,
            event_tx,
            event_rx,
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
        let chain = self.chain.clone();
        let network = self.config.network;
        let max_inbound = self.config.max_inbound;
        let accept_handle = tokio::spawn(async move {
            let mut inbound = 0usize;
            loop {
                let Ok((stream, addr)) = listener.accept().await else {
                    break;
                };
                if inbound >= max_inbound {
                    continue;
                }
                inbound += 1;
                let config = HandshakeConfig {
                    local_nonce: next_nonce(),
                    ..HandshakeConfig::default()
                };
                let height = chain.height().unwrap_or(0) as i32;
                let (_cmd_tx, _handle) = spawn_peer(
                    stream,
                    addr,
                    ConnectionDirection::Inbound,
                    network,
                    chain.clone(),
                    config,
                    height,
                    event_tx.clone(),
                );
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
        let config = HandshakeConfig {
            local_nonce: next_nonce(),
            ..HandshakeConfig::default()
        };
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
        );
        self.peers.insert(
            addr,
            PeerEntry {
                command_tx,
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

    /// Drains pending peer events and relays block inventory.
    ///
    /// # Errors
    ///
    /// Returns [`NetError`] when inventory relay fails.
    pub async fn process_events(&mut self) -> Result<(), NetError> {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                PeerEvent::Announce { addr, items } => {
                    let filtered: Vec<_> = items
                        .into_iter()
                        .filter(|item| item.inv_type == InvType::Block)
                        .collect();
                    self.relay_inventory(Some(addr), filtered).await?;
                }
                PeerEvent::HandshakeComplete { addr } => {
                    if !self.peers.contains_key(&addr) {
                        // Inbound peers are not inserted yet; ignore for now.
                    }
                }
                PeerEvent::Disconnected { addr, .. } => {
                    if self.peers.remove(&addr).is_some() {
                        self.outbound_count = self.outbound_count.saturating_sub(1);
                    }
                }
            }
        }
        Ok(())
    }

    /// Returns the number of tracked outbound peers.
    #[must_use]
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }
}

fn next_nonce() -> u64 {
    NONCE_COUNTER.fetch_add(1, Ordering::Relaxed)
}
