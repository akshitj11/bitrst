//! Base58Check address encoding for Bitcoin payloads.

use crate::sha256d::sha256d;

/// Errors while decoding Base58Check data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Base58Error {
    /// The string contains a byte outside the Bitcoin Base58 alphabet.
    InvalidCharacter,
    /// Decoded bytes were too short to contain version, payload, and checksum.
    InvalidLength,
    /// The four-byte checksum did not match the payload.
    InvalidChecksum,
}

/// Encodes a version byte and payload with Bitcoin's Base58Check checksum.
///
/// Base58Check appends the first four bytes of SHA256d(version || payload) before
/// encoding, which lets address parsers detect common transcription mistakes.
pub fn encode_check(version: u8, payload: &[u8]) -> String {
    let mut data = Vec::with_capacity(1 + payload.len() + 4);
    data.push(version);
    data.extend_from_slice(payload);
    data.extend_from_slice(&checksum(&data));
    bs58::encode(data).into_string()
}

/// Decodes a Base58Check string into `(version, payload)`.
///
/// # Errors
///
/// Returns [`Base58Error`] when the string is not valid Base58Check data.
pub fn decode_check(s: &str) -> Result<(u8, Vec<u8>), Base58Error> {
    let data = bs58::decode(s)
        .into_vec()
        .map_err(|_| Base58Error::InvalidCharacter)?;

    if data.len() < 5 {
        return Err(Base58Error::InvalidLength);
    }

    let (body, got_checksum) = data.split_at(data.len() - 4);
    if checksum(body) != got_checksum {
        return Err(Base58Error::InvalidChecksum);
    }

    Ok((body[0], body[1..].to_vec()))
}

fn checksum(data: &[u8]) -> [u8; 4] {
    let digest = sha256d(data);
    [digest[0], digest[1], digest[2], digest[3]]
}

#[cfg(test)]
mod tests {
    use super::{decode_check, encode_check, Base58Error};

    /// Bitcoin Wiki Base58Check example payload (version `0x00`).
    /// Reference: <https://en.bitcoin.it/wiki/Base58Check_encoding>
    #[test]
    fn encodes_mainnet_p2pkh_known_payload() {
        let payload = [
            0xd3, 0x0c, 0x70, 0xf7, 0xd1, 0xe2, 0x08, 0x12, 0x0e, 0x1e, 0x5e, 0x55, 0xb5, 0x34,
            0x1f, 0xa3, 0x21, 0xa6, 0x0f, 0xf2,
        ];

        assert_eq!(
            encode_check(0x00, &payload),
            "1LEvUuseTCgKTPfqB1d9xWUqJRZuxDhnCA"
        );
    }

    #[test]
    fn decodes_roundtrip_payload() {
        let payload = [0x42; 20];
        let encoded = encode_check(0x6f, &payload);

        assert_eq!(decode_check(&encoded), Ok((0x6f, payload.to_vec())));
    }

    #[test]
    fn rejects_bad_checksum() {
        let payload = [0x42; 20];
        let mut encoded = encode_check(0x00, &payload);
        encoded.replace_range(encoded.len() - 1.., "2");

        assert_eq!(decode_check(&encoded), Err(Base58Error::InvalidChecksum));
    }

    #[test]
    fn rejects_invalid_character() {
        assert_eq!(decode_check("0"), Err(Base58Error::InvalidCharacter));
    }

    #[test]
    fn rejects_too_short_payload() {
        assert_eq!(decode_check("1"), Err(Base58Error::InvalidLength));
    }
}
