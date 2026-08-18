//! Bitcoin P2P protocol constants and network identifiers.

use std::time::Duration;

use bitrst_core::limits::MAX_BLOCK_SERIALIZED_SIZE;

/// Size of the fixed Bitcoin P2P message header in bytes.
pub const MESSAGE_HEADER_SIZE: usize = 24;

/// Length of the NUL-padded ASCII command field in a message header.
pub const COMMAND_LENGTH: usize = 12;

/// Protocol version advertised during handshake.
pub const PROTOCOL_VERSION: i32 = 70016;

/// Maximum accepted payload size for one P2P message.
pub const MAX_PAYLOAD_SIZE: usize = MAX_BLOCK_SERIALIZED_SIZE;

/// Maximum inventory entries accepted in one `inv` or `getdata` message.
pub const MAX_INV_COUNT: usize = 50_000;

/// Maximum user-agent length in the `version` message.
pub const MAX_USER_AGENT_LEN: usize = 256;

/// Maximum queued outbound messages per peer before back-pressure applies.
pub const MAX_OUTBOUND_QUEUE: usize = 128;

/// Default maximum simultaneous inbound peers.
pub const DEFAULT_MAX_INBOUND: usize = 8;

/// Default maximum simultaneous outbound peers.
pub const DEFAULT_MAX_OUTBOUND: usize = 8;

/// Maximum outstanding block `getdata` requests tracked per peer.
pub const MAX_PENDING_BLOCK_REQUESTS: usize = 256;

/// Time after which an outstanding block request may be retried.
#[cfg(feature = "test-short-period")]
pub const BLOCK_REQUEST_TTL: Duration = Duration::from_secs(30);

/// Time after which an outstanding block request may be retried.
#[cfg(not(feature = "test-short-period"))]
pub const BLOCK_REQUEST_TTL: Duration = Duration::from_secs(300);

/// Maximum pending peer events buffered by the manager.
pub const MAX_PEER_EVENTS: usize = 256;

/// Maximum pending inbound peer registrations buffered by the manager.
pub const MAX_PEER_REGISTRATIONS: usize = 32;

/// Handshake timeout for version/verack exchange.
#[cfg(feature = "test-short-period")]
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);

/// Handshake timeout for version/verack exchange.
#[cfg(not(feature = "test-short-period"))]
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);

/// Bitcoin network selection for magic bytes and seeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Network {
    /// Bitcoin mainnet.
    Mainnet,
    /// Bitcoin public testnet.
    Testnet,
}

impl Network {
    /// Returns the four-byte little-endian network magic used in message headers.
    #[must_use]
    pub const fn magic(self) -> [u8; 4] {
        match self {
            Self::Mainnet => [0xf9, 0xbe, 0xb4, 0xd9],
            Self::Testnet => [0x0b, 0x11, 0x09, 0x07],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Network, MAX_PAYLOAD_SIZE, MESSAGE_HEADER_SIZE};
    use bitrst_core::limits::MAX_BLOCK_SERIALIZED_SIZE;

    #[test]
    fn mainnet_magic_matches_bitcoin_core() {
        assert_eq!(Network::Mainnet.magic(), [0xf9, 0xbe, 0xb4, 0xd9]);
    }

    #[test]
    fn testnet_magic_matches_bitcoin_core() {
        assert_eq!(Network::Testnet.magic(), [0x0b, 0x11, 0x09, 0x07]);
    }

    #[test]
    fn message_header_size_is_twenty_four_bytes() {
        assert_eq!(MESSAGE_HEADER_SIZE, 24);
    }

    #[test]
    fn payload_limit_matches_block_ceiling() {
        assert_eq!(MAX_PAYLOAD_SIZE, MAX_BLOCK_SERIALIZED_SIZE);
    }
}
