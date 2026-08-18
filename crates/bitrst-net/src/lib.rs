//! Bitcoin P2P networking for bitrst.

#![deny(unsafe_code)]

/// Protocol constants and network identifiers.
pub mod constants;

pub use constants::Network;
/// Networking error types.
pub mod error;
pub use error::NetError;
/// 24-byte message header handling.
pub mod envelope;
pub use envelope::{checksum, Command, MessageHeader};
/// Message and inventory types.
pub mod message;
pub use message::{InventoryVector, InvType, Message, MessagePayload, VersionMessage};
