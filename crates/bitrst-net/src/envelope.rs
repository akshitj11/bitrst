//! Bitcoin P2P message header encoding and validation.

use bitrst_crypto::sha256d::sha256d;

use crate::constants::{COMMAND_LENGTH, MESSAGE_HEADER_SIZE};
use crate::error::NetError;

/// A validated 12-byte NUL-padded ASCII command name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Command(String);

impl Command {
    /// Parses and validates a command from a fixed 12-byte header field.
    ///
    /// # Errors
    ///
    /// Returns [`NetError::InvalidCommand`] when bytes are not valid NUL-padded ASCII.
    pub fn from_header_bytes(bytes: &[u8; COMMAND_LENGTH]) -> Result<Self, NetError> {
        let end = bytes
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(COMMAND_LENGTH);
        let command_bytes = &bytes[..end];
        if command_bytes.is_empty() {
            return Err(NetError::InvalidCommand);
        }
        if !command_bytes
            .iter()
            .all(|byte| (0x20..=0x7e).contains(byte))
        {
            return Err(NetError::InvalidCommand);
        }
        if bytes[end..].iter().any(|&byte| byte != 0) {
            return Err(NetError::InvalidCommand);
        }
        Ok(Self(
            std::str::from_utf8(command_bytes)
                .map_err(|_| NetError::InvalidCommand)?
                .to_owned(),
        ))
    }

    /// Encodes the command into a fixed 12-byte NUL-padded field.
    ///
    /// # Errors
    ///
    /// Returns [`NetError::InvalidCommand`] when the name is empty, too long, or non-ASCII.
    pub fn encode(name: &str) -> Result<[u8; COMMAND_LENGTH], NetError> {
        if name.is_empty() || name.len() > COMMAND_LENGTH {
            return Err(NetError::InvalidCommand);
        }
        if !name.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
            return Err(NetError::InvalidCommand);
        }
        let mut out = [0u8; COMMAND_LENGTH];
        out[..name.len()].copy_from_slice(name.as_bytes());
        Ok(out)
    }

    /// Returns the command string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A decoded 24-byte Bitcoin P2P message header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageHeader {
    /// Network magic bytes.
    pub magic: [u8; 4],
    /// NUL-padded ASCII command.
    pub command: Command,
    /// Payload length in bytes (little-endian on the wire).
    pub payload_len: u32,
    /// First four bytes of SHA256d(payload).
    pub checksum: [u8; 4],
}

impl MessageHeader {
    /// Decodes a header and validates magic, command padding, and payload bounds.
    ///
    /// # Errors
    ///
    /// Returns [`NetError`] when magic, command, or payload length checks fail.
    pub fn decode(
        bytes: &[u8; MESSAGE_HEADER_SIZE],
        expected_magic: [u8; 4],
        max_payload: usize,
    ) -> Result<Self, NetError> {
        let mut magic = [0u8; 4];
        magic.copy_from_slice(&bytes[0..4]);
        if magic != expected_magic {
            return Err(NetError::InvalidMagic);
        }

        let mut command_bytes = [0u8; COMMAND_LENGTH];
        command_bytes.copy_from_slice(&bytes[4..16]);
        let command = Command::from_header_bytes(&command_bytes)?;

        let payload_len = u32::from_le_bytes(bytes[16..20].try_into().expect("four bytes"));
        if payload_len as usize > max_payload {
            return Err(NetError::PayloadTooLarge {
                length: payload_len,
                limit: max_payload,
            });
        }

        let mut checksum = [0u8; 4];
        checksum.copy_from_slice(&bytes[20..24]);

        Ok(Self {
            magic,
            command,
            payload_len,
            checksum,
        })
    }

    /// Encodes a header for `command` and `payload`.
    ///
    /// # Errors
    ///
    /// Returns [`NetError`] when the command or payload length is invalid.
    pub fn encode(
        command: &str,
        payload: &[u8],
        magic: [u8; 4],
        max_payload: usize,
    ) -> Result<[u8; MESSAGE_HEADER_SIZE], NetError> {
        if payload.len() > max_payload {
            return Err(NetError::PayloadTooLarge {
                length: payload.len() as u32,
                limit: max_payload,
            });
        }
        let command_bytes = Command::encode(command)?;
        let checksum = checksum(payload);
        let mut out = [0u8; MESSAGE_HEADER_SIZE];
        out[0..4].copy_from_slice(&magic);
        out[4..16].copy_from_slice(&command_bytes);
        out[16..20].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        out[20..24].copy_from_slice(&checksum);
        Ok(out)
    }

    /// Verifies the checksum against `payload`.
    ///
    /// # Errors
    ///
    /// Returns [`NetError::ChecksumMismatch`] when bytes do not match.
    pub fn verify_checksum(&self, payload: &[u8]) -> Result<(), NetError> {
        if checksum(payload) != self.checksum {
            return Err(NetError::ChecksumMismatch {
                command: self.command.as_str().to_owned(),
            });
        }
        Ok(())
    }
}

/// Returns the first four bytes of SHA256d(payload).
#[must_use]
pub fn checksum(payload: &[u8]) -> [u8; 4] {
    let hash = sha256d(payload);
    let mut out = [0u8; 4];
    out.copy_from_slice(&hash[..4]);
    out
}

#[cfg(test)]
mod tests {
    use super::{checksum, Command, MessageHeader};
    use crate::constants::{Network, COMMAND_LENGTH, MAX_PAYLOAD_SIZE};

    #[test]
    fn command_rejects_interior_nul() {
        let mut bytes = [0u8; COMMAND_LENGTH];
        bytes[0] = b'v';
        bytes[1] = 0;
        bytes[2] = b'e';
        assert_eq!(
            Command::from_header_bytes(&bytes),
            Err(crate::error::NetError::InvalidCommand)
        );
    }

    #[test]
    fn command_rejects_non_ascii() {
        let mut bytes = [0u8; COMMAND_LENGTH];
        bytes[0] = 0xff;
        assert_eq!(
            Command::from_header_bytes(&bytes),
            Err(crate::error::NetError::InvalidCommand)
        );
    }

    #[test]
    fn command_accepts_nul_padded_version() {
        let encoded = Command::encode("version").expect("valid command");
        let command = Command::from_header_bytes(&encoded).expect("valid command");
        assert_eq!(command.as_str(), "version");
    }

    #[test]
    fn header_roundtrip_matches_payload_checksum() {
        let payload = b"payload-bytes";
        let header_bytes = MessageHeader::encode(
            "verack",
            payload,
            Network::Mainnet.magic(),
            MAX_PAYLOAD_SIZE,
        )
        .expect("encode header");
        let header =
            MessageHeader::decode(&header_bytes, Network::Mainnet.magic(), MAX_PAYLOAD_SIZE)
                .expect("decode header");
        header.verify_checksum(payload).expect("checksum ok");
        assert_eq!(header.command.as_str(), "verack");
        assert_eq!(header.payload_len as usize, payload.len());
    }

    #[test]
    fn header_rejects_wrong_magic() {
        let payload = [];
        let header_bytes =
            MessageHeader::encode("ping", &payload, Network::Mainnet.magic(), MAX_PAYLOAD_SIZE)
                .expect("encode");
        assert_eq!(
            MessageHeader::decode(&header_bytes, Network::Testnet.magic(), MAX_PAYLOAD_SIZE),
            Err(crate::error::NetError::InvalidMagic)
        );
    }

    #[test]
    fn header_rejects_oversized_payload_length() {
        let mut header_bytes =
            MessageHeader::encode("block", &[], Network::Mainnet.magic(), MAX_PAYLOAD_SIZE)
                .expect("encode");
        header_bytes[16..20].copy_from_slice(&(MAX_PAYLOAD_SIZE as u32 + 1).to_le_bytes());
        assert!(matches!(
            MessageHeader::decode(&header_bytes, Network::Mainnet.magic(), MAX_PAYLOAD_SIZE),
            Err(crate::error::NetError::PayloadTooLarge { .. })
        ));
    }

    #[test]
    fn checksum_matches_sha256d_prefix() {
        let payload = b"1234567890";
        let hash = bitrst_crypto::sha256d::sha256d(payload);
        assert_eq!(checksum(payload), hash[..4]);
    }
}
