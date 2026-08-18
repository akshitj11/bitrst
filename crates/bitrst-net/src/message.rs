//! Message payload types and inventory vectors.

use bitrst_core::{Block, Transaction};

/// Inventory object type on the Bitcoin P2P wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum InvType {
    /// Transaction inventory (`MSG_TX`).
    Transaction = 1,
    /// Block inventory (`MSG_BLOCK`).
    Block = 2,
    /// Filtered block inventory (`MSG_FILTERED_BLOCK`).
    FilteredBlock = 3,
}

impl InvType {
    /// Decodes a wire inventory type.
    ///
    /// # Errors
    ///
    /// Returns `None` for unknown type values.
    #[must_use]
    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            1 => Some(Self::Transaction),
            2 => Some(Self::Block),
            3 => Some(Self::FilteredBlock),
            _ => None,
        }
    }

    /// Encodes the inventory type for the wire.
    #[must_use]
    pub const fn to_u32(self) -> u32 {
        self as u32
    }
}

/// One `inv` / `getdata` entry referencing a block or transaction hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryVector {
    /// Inventory type (`MSG_TX`, `MSG_BLOCK`, ...).
    pub inv_type: InvType,
    /// Object hash in internal byte order.
    pub hash: [u8; 32],
}

/// Decoded P2P message payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessagePayload {
    /// Empty `verack` acknowledgement.
    Verack,
    /// Protocol `version` announcement.
    Version(VersionMessage),
    /// Inventory announcement.
    Inv(Vec<InventoryVector>),
    /// Request for known objects.
    GetData(Vec<InventoryVector>),
    /// Full block payload.
    Block(Block),
    /// Legacy transaction payload.
    Tx(Transaction),
}

/// Fields carried in the `version` message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionMessage {
    /// Protocol version.
    pub version: i32,
    /// Service flags.
    pub services: u64,
    /// Peer timestamp.
    pub timestamp: i64,
    /// Random nonce used to detect self-connections.
    pub nonce: u64,
    /// User agent string.
    pub user_agent: String,
    /// Best known block height at send time.
    pub start_height: i32,
    /// Whether the peer relays transactions (BIP37-era field).
    pub relay: bool,
}

impl VersionMessage {
    /// Builds a version message for outbound handshakes.
    #[must_use]
    pub fn new(
        version: i32,
        services: u64,
        timestamp: i64,
        nonce: u64,
        user_agent: impl Into<String>,
        start_height: i32,
        relay: bool,
    ) -> Self {
        Self {
            version,
            services,
            timestamp,
            nonce,
            user_agent: user_agent.into(),
            start_height,
            relay,
        }
    }
}

/// A fully decoded P2P message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Command name (`version`, `block`, ...).
    pub command: String,
    /// Decoded payload.
    pub payload: MessagePayload,
}

impl Message {
    /// Creates a `verack` message.
    #[must_use]
    pub fn verack() -> Self {
        Self {
            command: "verack".to_owned(),
            payload: MessagePayload::Verack,
        }
    }

    /// Creates a `version` message.
    #[must_use]
    pub fn version(version: VersionMessage) -> Self {
        Self {
            command: "version".to_owned(),
            payload: MessagePayload::Version(version),
        }
    }

    /// Creates an `inv` message.
    #[must_use]
    pub fn inv(items: Vec<InventoryVector>) -> Self {
        Self {
            command: "inv".to_owned(),
            payload: MessagePayload::Inv(items),
        }
    }

    /// Creates a `getdata` message.
    #[must_use]
    pub fn getdata(items: Vec<InventoryVector>) -> Self {
        Self {
            command: "getdata".to_owned(),
            payload: MessagePayload::GetData(items),
        }
    }

    /// Creates a `block` message.
    #[must_use]
    pub fn block(block: Block) -> Self {
        Self {
            command: "block".to_owned(),
            payload: MessagePayload::Block(block),
        }
    }

    /// Creates a `tx` message.
    #[must_use]
    pub fn tx(transaction: Transaction) -> Self {
        Self {
            command: "tx".to_owned(),
            payload: MessagePayload::Tx(transaction),
        }
    }
}
