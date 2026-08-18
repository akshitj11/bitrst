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
    event_tx: mpsc::UnboundedSender<PeerEvent>,
    event_rx: mpsc::UnboundedReceiver<PeerEvent>,
    register_tx: mpsc::UnboundedSender<PeerRegistration>,
    register_rx: mpsc::UnboundedReceiver<PeerRegistration>,
}

impl PeerManager {
    /// Creates a peer manager bound to `config` and sharing `chain`.
    #[must_use]
    pub fn new(chain: ChainHandle, config: PeerManagerConfig) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (register_tx, register_rx) = mpsc::unbounded_channel();
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
        let accept_handle = tokio::spawn(async move {
            loop {
                let Ok((stream, addr)) = listener.accept().await else {
                    break;
                };
                let config = HandshakeConfig {
                    local_nonce: next_nonce(),
                    ..HandshakeConfig::default()
                };
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
                );
                let _ = register_tx.send(PeerRegistration {
                    addr,
                    direction: ConnectionDirection::Inbound,
                    command_tx,
                    task,
                });
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
            let allowed = match registration.direction {
                ConnectionDirection::Inbound => self.inbound_count() < self.config.max_inbound,
                ConnectionDirection::Outbound => self.outbound_count < self.config.max_outbound,
            };
            if allowed {
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
            } else {
                registration.task.abort();
            }
        }
    }

    fn inbound_count(&self) -> usize {
        self.peers
            .values()
            .filter(|entry| entry.direction == ConnectionDirection::Inbound)
            .count()
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bitrst_core::{Block, BlockHeader, ChainHandle, Target};
    use tokio::net::TcpStream;
    use tokio::time::sleep;

    use super::{PeerManager, PeerManagerConfig};
    use crate::constants::Network;
    use crate::handshake::{ConnectionDirection, HandshakeConfig};
    use crate::message::{InvType, InventoryVector};
    use crate::peer::spawn_peer;
    use crate::seeds::SeedStrategy;

    const NETWORK_TIME: u32 = 1_231_006_505;
    const TEST_BITS: u32 = 0x1f00_ffff;

    fn genesis_block() -> Block {
        let header = BlockHeader {
            version: 1,
            prev_blockhash: [0u8; 32],
            merkle_root: [0u8; 32],
            time: NETWORK_TIME,
            bits: TEST_BITS,
            nonce: 0,
        };
        let mut block = Block::coinbase(header, 0, 50_0000_0000);
        let target = Target::from_bits(TEST_BITS).expect("bits");
        while !target.meets(&block.header.hash()) {
            block.header.nonce = block.header.nonce.wrapping_add(1);
        }
        block
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
        let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel();
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
        );

        for _ in 0..40 {
            manager.process_events().await.expect("events");
            if manager.peer_count() > 0 {
                break;
            }
            sleep(Duration::from_millis(25)).await;
        }

        assert_eq!(manager.peer_count(), 1);
        handle.abort();
    }

    #[tokio::test]
    async fn peer_manager_relays_inv_to_connected_peer() {
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
        let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (_cmd, handle) = spawn_peer(
            stream,
            format!("127.0.0.1:{port}").parse().expect("addr"),
            ConnectionDirection::Outbound,
            Network::Testnet,
            chain,
            HandshakeConfig {
                local_nonce: 77,
                timeout: Duration::from_secs(5),
            },
            0,
            event_tx,
        );

        for _ in 0..40 {
            manager.process_events().await.expect("events");
            if manager.peer_count() > 0 {
                break;
            }
            sleep(Duration::from_millis(25)).await;
        }

        let hash = [0x42u8; 32];
        manager
            .relay_inventory(
                None,
                vec![InventoryVector {
                    inv_type: InvType::Block,
                    hash,
                }],
            )
            .await
            .expect("relay");

        handle.abort();
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
}
