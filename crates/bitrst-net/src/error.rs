//! Networking error types with contextual variants for untrusted peers.

use std::time::Duration;

use bitrst_core::{ChainError, DecodeError};
use thiserror::Error;

/// Errors raised while encoding, decoding, or transporting P2P messages.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum NetError {
    /// The remote peer closed the connection.
    #[error("connection closed by peer")]
    ConnectionClosed,

    /// An I/O error occurred on the socket.
    #[error("io error: {0}")]
    Io(&'static str),

    /// The message header magic did not match the configured network.
    #[error("unexpected network magic")]
    InvalidMagic,

    /// The command field contained invalid characters or padding.
    #[error("invalid command field")]
    InvalidCommand,

    /// The declared payload length exceeds protocol limits.
    #[error("payload length {length} exceeds limit {limit}")]
    PayloadTooLarge {
        /// Declared payload length from the header.
        length: u32,
        /// Configured maximum payload size.
        limit: usize,
    },

    /// The payload checksum did not match SHA256d(payload).
    #[error("checksum mismatch for command {command}")]
    ChecksumMismatch {
        /// Command associated with the rejected message.
        command: String,
    },

    /// A message payload failed structural decoding.
    #[error("decode error: {0}")]
    Decode(#[from] DecodeError),

    /// A handshake step arrived out of order.
    #[error("handshake protocol violation: {0}")]
    HandshakeViolation(&'static str),

    /// The handshake did not complete before the timeout elapsed.
    #[error("handshake timed out after {0:?}")]
    HandshakeTimeout(Duration),

    /// The peer appears to be this node (matching version nonce).
    #[error("self-connection detected")]
    SelfConnection,

    /// Connecting to the peer was rejected by local policy.
    #[error("connection limit reached")]
    ConnectionLimitReached,

    /// The outbound message queue for a peer is full.
    #[error("outbound queue full")]
    OutboundQueueFull,

    /// The manager event queue is full.
    #[error("peer event queue full")]
    EventQueueFull,

    /// The manager registration queue is full.
    #[error("peer registration queue full")]
    RegistrationQueueFull,

    /// A post-handshake command name is not supported.
    #[error("unsupported command {command}")]
    UnsupportedCommand {
        /// Wire command string from the message header.
        command: String,
    },

    /// An inventory vector used an unknown type value.
    #[error("unknown inventory type {inv_type}")]
    UnknownInventoryType {
        /// Raw inventory type from the wire.
        inv_type: u32,
    },

    /// Chain validation failed while connecting a block.
    #[error("chain error: {0}")]
    Chain(#[from] ChainError),

    /// An internal task failed to join.
    #[error("task join failed")]
    TaskJoinFailed,
}
