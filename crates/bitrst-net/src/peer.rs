//! Per-peer async connection task.

use std::net::SocketAddr;
use std::sync::Arc;

use bitrst_core::{ChainHandle, MempoolHandle};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Notify};
use tokio::time::{timeout_at, Instant};

use crate::codec::default_version_message;
use crate::constants::{Network, MAX_OUTBOUND_QUEUE};
use crate::error::NetError;
use crate::framing::{FramedReader, MessageWriter};
use crate::handshake::{ConnectionDirection, HandshakeConfig, HandshakePhase, HandshakeState};
use crate::inbound_capacity::InboundGuard;
use crate::message::{Message, MessagePayload};
use crate::relay::{handle_peer_message, PeerRelayState, RelayAction, RelayError};

/// Configuration and shared handles for a peer connection task.
#[derive(Debug)]
pub struct PeerContext {
    /// Remote socket address.
    pub addr: SocketAddr,
    /// Whether this connection was initiated locally.
    pub direction: ConnectionDirection,
    /// Network magic and protocol parameters.
    pub network: Network,
    /// Shared chain state consulted by relay logic.
    pub chain: ChainHandle,
    /// Shared mempool consulted for transaction relay.
    pub mempool: MempoolHandle,
    /// Handshake timing and nonce configuration.
    pub handshake: HandshakeConfig,
    /// Chain height advertised in the local `version` message.
    pub start_height: i32,
    /// Bounded channel for peer lifecycle and relay events.
    pub event_tx: mpsc::Sender<PeerEvent>,
    /// Inbound slot reservation released when the peer task exits.
    pub inbound_guard: Option<InboundGuard>,
}

impl PeerContext {
    /// Creates a peer context for `addr` using default inbound-guard behaviour.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        addr: SocketAddr,
        direction: ConnectionDirection,
        network: Network,
        chain: ChainHandle,
        mempool: MempoolHandle,
        handshake: HandshakeConfig,
        start_height: i32,
        event_tx: mpsc::Sender<PeerEvent>,
    ) -> Self {
        Self {
            addr,
            direction,
            network,
            chain,
            mempool,
            handshake,
            start_height,
            event_tx,
            inbound_guard: None,
        }
    }

    /// Attaches an inbound capacity guard that releases on task exit.
    #[must_use]
    pub fn with_inbound_guard(mut self, guard: InboundGuard) -> Self {
        self.inbound_guard = Some(guard);
        self
    }
}

/// Events emitted by a connected peer task.
#[derive(Debug)]
pub enum PeerEvent {
    /// Handshake completed and application messages may flow.
    Ready {
        /// Remote socket address.
        addr: SocketAddr,
    },
    /// Inventory that should be relayed to other peers.
    Announce {
        /// Remote socket address.
        addr: SocketAddr,
        /// Block inventory to relay.
        items: Vec<crate::message::InventoryVector>,
    },
    /// The peer disconnected or failed.
    Disconnected {
        /// Remote socket address.
        addr: SocketAddr,
        /// Disconnect reason.
        error: NetError,
    },
    /// A peer registration was rejected by local policy.
    RegistrationRejected {
        /// Remote socket address.
        addr: SocketAddr,
        /// Rejection reason.
        error: NetError,
    },
}

/// Commands sent to a peer task.
#[derive(Debug)]
pub enum PeerCommand {
    /// Send a fully encoded message.
    Send(Message),
    /// Shut down the connection gracefully.
    Shutdown,
}

/// Spawns a peer task over an established TCP stream.
#[must_use]
pub fn spawn_peer(
    stream: TcpStream,
    ctx: PeerContext,
) -> (mpsc::Sender<PeerCommand>, tokio::task::JoinHandle<()>) {
    let (command_tx, command_rx) = mpsc::channel(32);
    let shutdown = Arc::new(Notify::new());
    let shutdown_for_task = Arc::clone(&shutdown);
    let addr = ctx.addr;
    let event_tx = ctx.event_tx.clone();
    let handle = tokio::spawn(async move {
        let result = run_peer(stream, ctx, command_rx, shutdown_for_task).await;
        if let Err(error) = result {
            let _ = emit_event(
                &event_tx,
                PeerEvent::Disconnected { addr, error },
                &shutdown,
            )
            .await;
        }
    });
    (command_tx, handle)
}

async fn run_peer(
    stream: TcpStream,
    ctx: PeerContext,
    mut commands: mpsc::Receiver<PeerCommand>,
    shutdown: Arc<Notify>,
) -> Result<(), NetError> {
    let PeerContext {
        addr,
        direction,
        network,
        chain,
        mempool,
        handshake: handshake_config,
        start_height,
        event_tx,
        inbound_guard: _inbound_guard,
    } = ctx;
    let (mut read_half, write_half) = stream.into_split();
    let (writer, writer_handle) = MessageWriter::spawn(write_half, network, MAX_OUTBOUND_QUEUE);
    let mut framed = FramedReader::new();
    let mut relay = PeerRelayState::with_event_cursor(chain.event_cursor()?);

    let mut handshake = HandshakeState::new(direction, handshake_config.clone());
    let local_version =
        default_version_message(handshake_config.local_nonce, unix_timestamp(), start_height);

    if direction == ConnectionDirection::Outbound {
        for message in handshake.initial_outbound_messages(local_version.clone()) {
            writer.send(message).await?;
        }
    }

    let handshake_deadline = Instant::now() + handshake_config.timeout;
    while handshake.phase() != HandshakePhase::Established {
        let message = timeout_at(
            handshake_deadline,
            framed.read_message(&mut read_half, network),
        )
        .await
        .map_err(|_| NetError::HandshakeTimeout(handshake_config.timeout))??;

        let replies = handshake.on_message(
            &message,
            if direction == ConnectionDirection::Inbound {
                Some(local_version.clone())
            } else {
                None
            },
        )?;
        for reply in replies {
            writer.send(reply).await?;
        }
    }

    emit_event(&event_tx, PeerEvent::Ready { addr }, &shutdown).await?;

    loop {
        tokio::select! {
            command = commands.recv() => {
                match command {
                    Some(PeerCommand::Send(message)) => writer.send(message).await?,
                    Some(PeerCommand::Shutdown) | None => {
                        shutdown.notify_waiters();
                        break;
                    }
                }
            }
            read_result = framed.read_message(&mut read_half, network) => {
                let message = read_result?;
                match message.payload {
                    MessagePayload::Version(_) | MessagePayload::Verack => {
                        return Err(NetError::HandshakeViolation(
                            "version/verack after handshake",
                        ));
                    }
                    _ => {
                        let chain = chain.clone();
                        let mempool = mempool.clone();
                        let now = std::time::Instant::now();
                        let (action, relay_state) = tokio::task::spawn_blocking(move || {
                            let mut relay_state = relay;
                            let result = handle_peer_message(
                                &chain,
                                &mempool,
                                &mut relay_state,
                                message,
                                now,
                            );
                            (result, relay_state)
                        })
                        .await
                        .map_err(|_| NetError::TaskJoinFailed)?;
                        relay = relay_state;
                        let action = action.map_err(relay_error_to_net)?;

                        match action {
                            RelayAction::None => {}
                            RelayAction::Reply(messages) => {
                                for reply in messages {
                                    writer.send(reply).await?;
                                }
                            }
                            RelayAction::Announce(items) => {
                                emit_event(
                                    &event_tx,
                                    PeerEvent::Announce { addr, items },
                                    &shutdown,
                                )
                                .await?;
                            }
                        }
                    }
                }
            }
        }
    }

    drop(writer);
    let _ = writer_handle.await;
    Ok(())
}

async fn emit_event(
    tx: &mpsc::Sender<PeerEvent>,
    event: PeerEvent,
    shutdown: &Arc<Notify>,
) -> Result<(), NetError> {
    tokio::select! {
        result = tx.send(event) => result.map_err(|_| NetError::EventQueueFull),
        _ = shutdown.notified() => Err(NetError::ConnectionClosed),
    }
}

fn relay_error_to_net(error: RelayError) -> NetError {
    match error {
        RelayError::Chain(chain_error) => chain_error.into(),
        RelayError::Mempool(_) => NetError::Io("mempool relay"),
    }
}

fn unix_timestamp() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{spawn_peer, PeerCommand, PeerContext, PeerEvent};
    use crate::constants::Network;
    use crate::handshake::{ConnectionDirection, HandshakeConfig};
    use crate::message::{InvType, InventoryVector, Message};
    use crate::testutil::{child_block, genesis_block, NETWORK_TIME};
    use bitrst_core::{ChainHandle, MempoolHandle};
    use tokio::net::TcpListener;
    use tokio::sync::{mpsc, Notify};
    use tokio::time::{sleep, Duration};

    fn event_channel() -> (mpsc::Sender<PeerEvent>, mpsc::Receiver<PeerEvent>) {
        mpsc::channel(crate::constants::MAX_PEER_EVENTS)
    }

    fn peer_context(
        addr: std::net::SocketAddr,
        direction: ConnectionDirection,
        chain: ChainHandle,
        handshake: HandshakeConfig,
        event_tx: mpsc::Sender<PeerEvent>,
    ) -> PeerContext {
        PeerContext::new(
            addr,
            direction,
            Network::Testnet,
            chain,
            MempoolHandle::new(),
            handshake,
            0,
            event_tx,
        )
    }

    #[tokio::test]
    async fn two_localhost_peers_complete_handshake() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let chain_in = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        let chain_out = chain_in.clone();

        let (event_tx, mut event_rx) = event_channel();

        let server = tokio::spawn(async move {
            let (stream, peer_addr) = listener.accept().await.expect("accept");
            spawn_peer(
                stream,
                peer_context(
                    peer_addr,
                    ConnectionDirection::Inbound,
                    chain_in,
                    HandshakeConfig {
                        local_nonce: 10,
                        timeout: Duration::from_secs(5),
                    },
                    event_tx,
                ),
            )
        });

        sleep(Duration::from_millis(50)).await;
        let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        let (event_tx2, mut event_rx2) = event_channel();
        let (_cmd_tx, handle) = spawn_peer(
            stream,
            peer_context(
                addr,
                ConnectionDirection::Outbound,
                chain_out,
                HandshakeConfig {
                    local_nonce: 20,
                    timeout: Duration::from_secs(5),
                },
                event_tx2,
            ),
        );

        let inbound_event = tokio::time::timeout(Duration::from_secs(5), event_rx.recv())
            .await
            .expect("timeout")
            .expect("event");
        let outbound_event = tokio::time::timeout(Duration::from_secs(5), event_rx2.recv())
            .await
            .expect("timeout")
            .expect("event");

        assert!(matches!(inbound_event, PeerEvent::Ready { .. }));
        assert!(matches!(outbound_event, PeerEvent::Ready { .. }));

        handle.abort();
        let (_cmd_tx, server_handle) = server.await.expect("server");
        server_handle.abort();
    }

    #[tokio::test]
    async fn handshake_times_out_without_version() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let chain = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        let (event_tx, mut event_rx) = event_channel();

        let server = tokio::spawn(async move {
            let (stream, peer_addr) = listener.accept().await.expect("accept");
            spawn_peer(
                stream,
                peer_context(
                    peer_addr,
                    ConnectionDirection::Inbound,
                    chain,
                    HandshakeConfig {
                        local_nonce: 1,
                        timeout: Duration::from_millis(200),
                    },
                    event_tx,
                ),
            )
        });

        sleep(Duration::from_millis(20)).await;
        let _silent = tokio::net::TcpStream::connect(addr).await.expect("connect");

        let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .expect("timeout")
            .expect("event");
        assert!(matches!(event, PeerEvent::Disconnected { .. }));

        let (_cmd, server_handle) = server.await.expect("server");
        server_handle.abort();
    }

    #[tokio::test]
    async fn duplicate_block_does_not_disconnect_peer() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let genesis = genesis_block();
        let chain_in = ChainHandle::new_genesis(genesis.clone(), NETWORK_TIME).expect("genesis");
        let chain_out = ChainHandle::new_genesis(genesis, NETWORK_TIME).expect("genesis");

        let (event_tx, mut event_rx) = event_channel();
        let server = tokio::spawn(async move {
            let (stream, peer_addr) = listener.accept().await.expect("accept");
            let (cmd_tx, handle) = spawn_peer(
                stream,
                peer_context(
                    peer_addr,
                    ConnectionDirection::Inbound,
                    chain_in,
                    HandshakeConfig {
                        local_nonce: 30,
                        timeout: Duration::from_secs(5),
                    },
                    event_tx,
                ),
            );
            (cmd_tx, handle)
        });

        sleep(Duration::from_millis(50)).await;
        let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        let (event_tx2, mut event_rx2) = event_channel();
        let (cmd_tx, handle) = spawn_peer(
            stream,
            peer_context(
                addr,
                ConnectionDirection::Outbound,
                chain_out,
                HandshakeConfig {
                    local_nonce: 40,
                    timeout: Duration::from_secs(5),
                },
                event_tx2,
            ),
        );

        let _ = tokio::time::timeout(Duration::from_secs(5), event_rx.recv())
            .await
            .expect("timeout");
        let _ = tokio::time::timeout(Duration::from_secs(5), event_rx2.recv())
            .await
            .expect("timeout");

        let (server_cmd, server_handle) = server.await.expect("server");
        let duplicate = genesis_block();
        cmd_tx
            .send(PeerCommand::Send(Message::block(duplicate.clone())))
            .await
            .expect("send duplicate");
        cmd_tx
            .send(PeerCommand::Send(Message::block(duplicate)))
            .await
            .expect("send duplicate again");

        sleep(Duration::from_millis(200)).await;
        assert!(
            tokio::time::timeout(Duration::from_secs(2), event_rx2.recv())
                .await
                .is_err(),
            "duplicate blocks should not disconnect the peer"
        );
        assert!(server_cmd.send(PeerCommand::Shutdown).await.is_ok());

        drop(cmd_tx);
        handle.abort();
        drop(server_cmd);
        server_handle.abort();
    }

    #[tokio::test]
    async fn announce_event_waits_on_bounded_queue() {
        let (event_tx, mut event_rx) = mpsc::channel(1);
        let shutdown = Arc::new(Notify::new());
        event_tx
            .send(PeerEvent::Ready {
                addr: "127.0.0.1:9".parse().expect("addr"),
            })
            .await
            .expect("seed ready event");

        let announce = PeerEvent::Announce {
            addr: "127.0.0.1:9".parse().expect("addr"),
            items: vec![InventoryVector {
                inv_type: InvType::Block,
                hash: [7u8; 32],
            }],
        };
        let shutdown_for_task = Arc::clone(&shutdown);
        let sender = event_tx.clone();
        let blocked =
            tokio::spawn(
                async move { super::emit_event(&sender, announce, &shutdown_for_task).await },
            );

        sleep(Duration::from_millis(50)).await;
        assert!(!blocked.is_finished());

        let _ = event_rx.recv().await;
        tokio::time::timeout(Duration::from_secs(1), blocked)
            .await
            .expect("announce should be delivered")
            .expect("join")
            .expect("emit");
    }

    #[tokio::test]
    async fn announce_emit_cancels_on_shutdown_notify() {
        let (event_tx, _event_rx) = mpsc::channel(1);
        let shutdown = Arc::new(Notify::new());
        event_tx
            .send(PeerEvent::Ready {
                addr: "127.0.0.1:9".parse().expect("addr"),
            })
            .await
            .expect("seed ready event");

        let announce = PeerEvent::Announce {
            addr: "127.0.0.1:9".parse().expect("addr"),
            items: vec![InventoryVector {
                inv_type: InvType::Block,
                hash: [8u8; 32],
            }],
        };
        let shutdown_for_task = Arc::clone(&shutdown);
        let sender = event_tx.clone();
        let blocked =
            tokio::spawn(
                async move { super::emit_event(&sender, announce, &shutdown_for_task).await },
            );

        sleep(Duration::from_millis(50)).await;
        shutdown.notify_waiters();

        let result = tokio::time::timeout(Duration::from_secs(1), blocked)
            .await
            .expect("shutdown cancels emit")
            .expect("join");
        assert_eq!(result, Err(crate::error::NetError::ConnectionClosed));
    }

    #[tokio::test]
    async fn peer_remains_responsive_while_chain_work_runs_on_blocking_pool() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let genesis = genesis_block();
        let chain_in = ChainHandle::new_genesis(genesis.clone(), NETWORK_TIME).expect("genesis");
        let chain_out = ChainHandle::new_genesis(genesis, NETWORK_TIME).expect("genesis");

        let (event_tx, mut event_rx) = event_channel();
        let server = tokio::spawn(async move {
            let (stream, peer_addr) = listener.accept().await.expect("accept");
            let (cmd_tx, handle) = spawn_peer(
                stream,
                peer_context(
                    peer_addr,
                    ConnectionDirection::Inbound,
                    chain_in,
                    HandshakeConfig {
                        local_nonce: 50,
                        timeout: Duration::from_secs(5),
                    },
                    event_tx,
                ),
            );
            (cmd_tx, handle)
        });

        sleep(Duration::from_millis(50)).await;
        let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        let (event_tx2, mut event_rx2) = event_channel();
        let (cmd_tx, handle) = spawn_peer(
            stream,
            peer_context(
                addr,
                ConnectionDirection::Outbound,
                chain_out,
                HandshakeConfig {
                    local_nonce: 60,
                    timeout: Duration::from_secs(5),
                },
                event_tx2,
            ),
        );

        let _ = tokio::time::timeout(Duration::from_secs(5), event_rx.recv())
            .await
            .expect("timeout");
        let _ = tokio::time::timeout(Duration::from_secs(5), event_rx2.recv())
            .await
            .expect("timeout");

        let child = child_block(&genesis_block(), 1, 600);
        cmd_tx
            .send(PeerCommand::Send(Message::block(child)))
            .await
            .expect("send block");
        cmd_tx
            .send(PeerCommand::Shutdown)
            .await
            .expect("peer should remain responsive during chain work");

        let (server_cmd, server_handle) = server.await.expect("server");
        let _ = server_cmd.send(PeerCommand::Shutdown).await;
        drop(cmd_tx);
        handle.abort();
        server_handle.abort();
    }
}
