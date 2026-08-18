//! Bitcoin P2P networking for bitrst.

#![deny(unsafe_code)]

/// Message payload codecs.
pub mod codec;
/// Protocol constants and network identifiers.
pub mod constants;
/// 24-byte message header handling.
pub mod envelope;
/// Networking error types.
pub mod error;
/// Async framed reader and writer helpers.
pub mod framing;
/// Version/verack handshake state machine.
pub mod handshake;
/// Message and inventory types.
pub mod message;
/// Per-peer connection task.
pub mod peer;
/// Peer manager and connection limits.
pub mod peers;
/// Block relay and chain integration.
pub mod relay;
/// Offline-friendly seed selection.
pub mod seeds;

#[cfg(test)]
mod testutil;

mod inbound_capacity;

pub use constants::Network;
pub use envelope::{checksum, Command, MessageHeader};
pub use error::NetError;
pub use handshake::{ConnectionDirection, HandshakeConfig, HandshakePhase, HandshakeState};
pub use message::{InvType, InventoryVector, Message, MessagePayload, VersionMessage};
pub use peer::{spawn_peer, PeerCommand, PeerEvent};
pub use peers::{PeerManager, PeerManagerConfig};
pub use relay::{handle_peer_message, BlockRequestTracker, RelayAction};
pub use seeds::SeedStrategy;
