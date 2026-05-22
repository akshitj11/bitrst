//! Bitcoin block header primitives.

use bitrst_crypto::sha256d::sha256d;

/// A Bitcoin block header in wire-serialization field order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockHeader {
    /// Block version, serialized as a little-endian signed integer.
    pub version: i32,
    /// Hash of the previous block header, stored in internal byte order.
    pub prev_blockhash: [u8; 32],
    /// Merkle root of this block's transactions, stored in internal byte order.
    pub merkle_root: [u8; 32],
    /// Unix timestamp claimed by the miner.
    pub time: u32,
    /// Compact target representation used for proof of work.
    pub bits: u32,
    /// Nonce adjusted by miners while searching for a valid proof of work.
    pub nonce: u32,
}

impl BlockHeader {
    /// Serializes the header into Bitcoin's fixed 80-byte wire format.
    pub fn serialize(&self) -> [u8; 80] {
        let mut out = [0u8; 80];
        out[0..4].copy_from_slice(&self.version.to_le_bytes());
        out[4..36].copy_from_slice(&self.prev_blockhash);
        out[36..68].copy_from_slice(&self.merkle_root);
        out[68..72].copy_from_slice(&self.time.to_le_bytes());
        out[72..76].copy_from_slice(&self.bits.to_le_bytes());
        out[76..80].copy_from_slice(&self.nonce.to_le_bytes());
        out
    }

    /// Returns the SHA-256d hash of this serialized block header.
    pub fn hash(&self) -> [u8; 32] {
        sha256d(&self.serialize())
    }
}

#[cfg(test)]
mod tests {
    use super::BlockHeader;

    fn to_bitcoin_hex(bytes: [u8; 32]) -> String {
        let mut reversed = bytes;
        reversed.reverse();
        hex::encode(reversed)
    }

    #[test]
    fn hashes_genesis_header() {
        let header = BlockHeader {
            version: 1,
            prev_blockhash: [0u8; 32],
            merkle_root: [
                0x3b, 0xa3, 0xed, 0xfd, 0x7a, 0x7b, 0x12, 0xb2, 0x7a, 0xc7, 0x2c, 0x3e, 0x67, 0x76,
                0x8f, 0x61, 0x7f, 0xc8, 0x1b, 0xc3, 0x88, 0x8a, 0x51, 0x32, 0x3a, 0x9f, 0xb8, 0xaa,
                0x4b, 0x1e, 0x5e, 0x4a,
            ],
            time: 1231006505,
            bits: 0x1d00ffff,
            nonce: 2083236893,
        };

        assert_eq!(
            to_bitcoin_hex(header.hash()),
            "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f"
        );
    }
}
