//! `version` message codec.

use bitrst_core::wire::WireReader;

use crate::constants::{MAX_USER_AGENT_LEN, PROTOCOL_VERSION};
use crate::error::NetError;
use crate::message::VersionMessage;

use super::wire_helpers::{read_limited_string, write_compact_size};

const LEGACY_ADDRESS_SIZE: usize = 26;

/// Decodes a `version` message payload.
///
/// # Errors
///
/// Returns [`NetError`] when fields are truncated or out of bounds.
pub fn decode_version(payload: &[u8]) -> Result<VersionMessage, NetError> {
    let mut reader = WireReader::new(payload);
    let version = reader.read_i32("version").map_err(NetError::Decode)?;
    let services = reader.read_u64("services").map_err(NetError::Decode)?;
    let timestamp = {
        let bytes = reader
            .read_bytes(8, "timestamp")
            .map_err(NetError::Decode)?;
        i64::from_le_bytes(bytes.try_into().expect("eight bytes"))
    };
    reader
        .read_bytes(LEGACY_ADDRESS_SIZE, "addr_recv")
        .map_err(NetError::Decode)?;
    reader
        .read_bytes(LEGACY_ADDRESS_SIZE, "addr_from")
        .map_err(NetError::Decode)?;
    let nonce = reader.read_u64("nonce").map_err(NetError::Decode)?;
    let user_agent = read_limited_string(&mut reader, "user agent", MAX_USER_AGENT_LEN)
        .map_err(NetError::Decode)?;
    let start_height = reader.read_i32("start height").map_err(NetError::Decode)?;

    let relay = if reader.remaining() == 0 {
        true
    } else if reader.remaining() == 1 {
        reader.read_bytes(1, "relay").map_err(NetError::Decode)?[0] != 0
    } else {
        return Err(NetError::Decode(
            bitrst_core::wire::DecodeError::TrailingBytes {
                context: "version",
                remaining: reader.remaining(),
            },
        ));
    };

    reader.finish("version").map_err(NetError::Decode)?;

    Ok(VersionMessage {
        version,
        services,
        timestamp,
        nonce,
        user_agent,
        start_height,
        relay,
    })
}

/// Encodes a `version` message payload.
///
/// # Errors
///
/// Returns [`NetError`] when the user agent exceeds limits.
pub fn encode_version(message: &VersionMessage) -> Result<Vec<u8>, NetError> {
    if message.user_agent.len() > MAX_USER_AGENT_LEN {
        return Err(NetError::PayloadTooLarge {
            length: message.user_agent.len() as u32,
            limit: MAX_USER_AGENT_LEN,
        });
    }

    let mut out = Vec::new();
    out.extend_from_slice(&message.version.to_le_bytes());
    out.extend_from_slice(&message.services.to_le_bytes());
    out.extend_from_slice(&message.timestamp.to_le_bytes());
    out.extend_from_slice(&[0u8; LEGACY_ADDRESS_SIZE]);
    out.extend_from_slice(&[0u8; LEGACY_ADDRESS_SIZE]);
    out.extend_from_slice(&message.nonce.to_le_bytes());
    write_compact_size(message.user_agent.len() as u64, &mut out);
    out.extend_from_slice(message.user_agent.as_bytes());
    out.extend_from_slice(&message.start_height.to_le_bytes());
    if message.version >= 60_002 {
        out.push(u8::from(message.relay));
    }
    Ok(out)
}

/// Builds the default outbound version message for this crate.
#[must_use]
pub fn default_version_message(nonce: u64, timestamp: i64, start_height: i32) -> VersionMessage {
    VersionMessage::new(
        PROTOCOL_VERSION,
        0,
        timestamp,
        nonce,
        "/bitrst:0.1.0/",
        start_height,
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::{decode_version, default_version_message, encode_version};
    use crate::constants::MAX_USER_AGENT_LEN;

    #[test]
    fn version_roundtrip_includes_relay_flag() {
        let message = default_version_message(0xdead_beef_cafe_babe, 1_700_000_000, 42);
        let encoded = encode_version(&message).expect("encode version");
        let decoded = decode_version(&encoded).expect("decode version");
        assert_eq!(decoded, message);
    }

    #[test]
    fn version_rejects_oversized_user_agent() {
        let mut message = default_version_message(1, 2, 0);
        message.user_agent = "x".repeat(MAX_USER_AGENT_LEN + 1);
        assert!(encode_version(&message).is_err());
    }
}
