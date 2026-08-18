//! Per-peer async connection task.

use std::net::SocketAddr;

use bitrst_core::ChainHandle;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::{timeout_at, Instant};

use crate::codec::default_version_message;
use crate::constants::{Network, MAX_OUTBOUND_QUEUE};
use crate::error::NetError;
use crate::framing::{FramedReader, MessageWriter};
use crate::handshake::{ConnectionDirection, HandshakeConfig, HandshakePhase, HandshakeState};
use crate::inbound_capacity::InboundGuard;
use crate::message::{Message, MessagePayload};
use crate::relay::{handle_peer_message, BlockRequestTracker, RelayAction};

/// Events emitted by a connected peer task.
#[derive(Debug)]
pub enum PeerEvent {
    /// Handshake completed and application messages may flow.
    HandshakeComplete {
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
#[allow(clippy::too_many_arguments)]
pub fn spawn_peer(
    stream: TcpStream,
    addr: SocketAddr,
    direction: ConnectionDirection,
    network: Network,
    chain: ChainHandle,
    handshake_config: HandshakeConfig,
    start_height: i32,
    event_tx: mpsc::Sender<PeerEvent>,
    inbound_guard: Option<InboundGuard>,
) -> (mpsc::Sender<PeerCommand>, tokio::task::JoinHandle<()>) {
    let (command_tx, command_rx) = mpsc::channel(32);
    let handle = tokio::spawn(async move {
        let result = run_peer(
            stream,
            addr,
            direction,
            network,
            chain,
            handshake_config,
            start_height,
            command_rx,
            &event_tx,
            inbound_guard,
        )
        .await;
        if let Err(error) = result {
            let _ = emit_event(&event_tx, PeerEvent::Disconnected { addr, error }).await;
        }
    });
    (command_tx, handle)
}

#[allow(clippy::too_many_arguments)]
async fn run_peer(
    stream: TcpStream,
    addr: SocketAddr,
    direction: ConnectionDirection,
    network: Network,
    chain: ChainHandle,
    handshake_config: HandshakeConfig,
    start_height: i32,
    mut commands: mpsc::Receiver<PeerCommand>,
    event_tx: &mpsc::Sender<PeerEvent>,
    _inbound_guard: Option<InboundGuard>,
) -> Result<(), NetError> {
    let (mut read_half, write_half) = stream.into_split();
    let (writer, writer_handle) = MessageWriter::spawn(write_half, network, MAX_OUTBOUND_QUEUE);
    let mut framed = FramedReader::new();
    let mut block_requests = BlockRequestTracker::default();

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

    emit_event(event_tx, PeerEvent::HandshakeComplete { addr }).await?;

    loop {
        tokio::select! {
            command = commands.recv() => {
                match command {
                    Some(PeerCommand::Send(message)) => writer.send(message).await?,
                    Some(PeerCommand::Shutdown) | None => break,
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
                        let action = handle_peer_message(&chain, message, &mut block_requests)?;

                        match action {
                            RelayAction::None => {}
                            RelayAction::Reply(messages) => {
                                for reply in messages {
                                    writer.send(reply).await?;
                                }
                            }
                            RelayAction::Announce(items) => {
                                let _ = emit_event(
                                    event_tx,
                                    PeerEvent::Announce { addr, items },
                                )
                                .await;
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

async fn emit_event(tx: &mpsc::Sender<PeerEvent>, event: PeerEvent) -> Result<(), NetError> {
    match event {
        PeerEvent::Disconnected { .. } | PeerEvent::HandshakeComplete { .. } => {
            tx.send(event).await.map_err(|_| NetError::EventQueueFull)
        }
        PeerEvent::Announce { .. } => tx.try_send(event).map_err(|_| NetError::EventQueueFull),
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
    use super::{spawn_peer, PeerEvent};
    use crate::constants::Network;
    use crate::handshake::{ConnectionDirection, HandshakeConfig};
    use crate::testutil::{genesis_block, NETWORK_TIME};
    use bitrst_core::ChainHandle;
    use tokio::net::TcpListener;
    use tokio::sync::mpsc;
    use tokio::time::{sleep, Duration};

    fn event_channel() -> (mpsc::Sender<PeerEvent>, mpsc::Receiver<PeerEvent>) {
        mpsc::channel(crate::constants::MAX_PEER_EVENTS)
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
                peer_addr,
                ConnectionDirection::Inbound,
                Network::Testnet,
                chain_in,
                HandshakeConfig {
                    local_nonce: 10,
                    timeout: Duration::from_secs(5),
                },
                0,
                event_tx,
                None,
            )
        });

        sleep(Duration::from_millis(50)).await;
        let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        let (event_tx2, mut event_rx2) = event_channel();
        let (_cmd_tx, handle) = spawn_peer(
            stream,
            addr,
            ConnectionDirection::Outbound,
            Network::Testnet,
            chain_out,
            HandshakeConfig {
                local_nonce: 20,
                timeout: Duration::from_secs(5),
            },
            0,
            event_tx2,
            None,
        );

        let inbound_event = tokio::time::timeout(Duration::from_secs(5), event_rx.recv())
            .await
            .expect("timeout")
            .expect("event");
        let outbound_event = tokio::time::timeout(Duration::from_secs(5), event_rx2.recv())
            .await
            .expect("timeout")
            .expect("event");

        assert!(matches!(inbound_event, PeerEvent::HandshakeComplete { .. }));
        assert!(matches!(
            outbound_event,
            PeerEvent::HandshakeComplete { .. }
        ));

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
                peer_addr,
                ConnectionDirection::Inbound,
                Network::Testnet,
                chain,
                HandshakeConfig {
                    local_nonce: 1,
                    timeout: Duration::from_millis(200),
                },
                0,
                event_tx,
                None,
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
                peer_addr,
                ConnectionDirection::Inbound,
                Network::Testnet,
                chain_in,
                HandshakeConfig {
                    local_nonce: 30,
                    timeout: Duration::from_secs(5),
                },
                0,
                event_tx,
                None,
            );
            (cmd_tx, handle)
        });

        sleep(Duration::from_millis(50)).await;
        let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        let (event_tx2, mut event_rx2) = event_channel();
        let (cmd_tx, handle) = spawn_peer(
            stream,
            addr,
            ConnectionDirection::Outbound,
            Network::Testnet,
            chain_out,
            HandshakeConfig {
                local_nonce: 40,
                timeout: Duration::from_secs(5),
            },
            0,
            event_tx2,
            None,
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
            .send(super::PeerCommand::Send(crate::message::Message::block(
                duplicate.clone(),
            )))
            .await
            .expect("send duplicate");
        cmd_tx
            .send(super::PeerCommand::Send(crate::message::Message::block(
                duplicate,
            )))
            .await
            .expect("send duplicate again");

        sleep(Duration::from_millis(200)).await;
        assert!(
            tokio::time::timeout(Duration::from_secs(2), event_rx2.recv())
                .await
                .is_err(),
            "duplicate blocks should not disconnect the peer"
        );
        assert!(server_cmd.send(super::PeerCommand::Shutdown).await.is_ok());

        drop(cmd_tx);
        handle.abort();
        drop(server_cmd);
        server_handle.abort();
    }
}
