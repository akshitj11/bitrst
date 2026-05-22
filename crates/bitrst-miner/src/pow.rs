//! Proof-of-work mining primitives.

use bitrst_core::BlockHeader;
use std::cmp::Ordering;

/// A proof-of-work target decoded from Bitcoin's compact `bits` format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    threshold: [u8; 32],
}

impl Target {
    /// Decodes a compact Bitcoin proof-of-work target.
    pub fn from_bits(bits: u32) -> Option<Self> {
        let exponent = (bits >> 24) as usize;
        let mantissa = bits & 0x007f_ffff;

        if mantissa == 0 || bits & 0x0080_0000 != 0 {
            return None;
        }

        let mut threshold = [0u8; 32];

        if exponent <= 3 {
            let value = mantissa >> (8 * (3 - exponent));
            let bytes = value.to_le_bytes();
            threshold[0..3].copy_from_slice(&bytes[0..3]);
        } else {
            let offset = exponent - 3;
            if offset + 3 > threshold.len() {
                return None;
            }

            let bytes = mantissa.to_le_bytes();
            threshold[offset..offset + 3].copy_from_slice(&bytes[0..3]);
        }

        Some(Self { threshold })
    }

    /// Creates an easy target for learning and testing.
    pub fn easy() -> Self {
        Self {
            threshold: [0xff; 32],
        }
    }

    /// Returns the decoded 32-byte target in internal little-endian byte order.
    pub fn threshold(&self) -> [u8; 32] {
        self.threshold
    }

    /// Returns true when the hash is at or below the target threshold.
    pub fn meets(&self, hash: &[u8; 32]) -> bool {
        compare_internal_hash(hash, &self.threshold) != Ordering::Greater
    }
}

fn compare_internal_hash(left: &[u8; 32], right: &[u8; 32]) -> Ordering {
    left.iter().rev().cmp(right.iter().rev())
}

/// Mines a block by incrementing the nonce until the header hash meets the target.
pub fn mine(header: &mut BlockHeader, target: Target) -> [u8; 32] {
    loop {
        let hash = header.hash();
        if target.meets(&hash) {
            return hash;
        }

        header.nonce = header.nonce.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{mine, Target};
    use bitrst_core::BlockHeader;

    #[test]
    fn decodes_genesis_bits_target() {
        let target = Target::from_bits(0x1d00ffff).expect("genesis bits should decode");
        let mut expected = [0u8; 32];
        expected[26] = 0xff;
        expected[27] = 0xff;

        assert_eq!(target.threshold(), expected);
    }

    #[test]
    fn rejects_invalid_compact_targets() {
        assert_eq!(Target::from_bits(0), None);
        assert_eq!(Target::from_bits(0x01800000), None);
        assert_eq!(Target::from_bits(0x2100ffff), None);
    }

    #[test]
    fn compares_internal_hashes_as_little_endian_numbers() {
        let target = Target::from_bits(0x03000100).expect("target should decode");
        let equal = target.threshold();
        let below = [
            0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
        ];
        let above = [
            0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
        ];

        assert!(target.meets(&equal));
        assert!(target.meets(&below));
        assert!(!target.meets(&above));
    }

    #[test]
    fn mines_with_easy_target() {
        let mut header = BlockHeader {
            version: 1,
            prev_blockhash: [0u8; 32],
            merkle_root: [0u8; 32],
            time: 0,
            bits: 0,
            nonce: 0,
        };

        let hash = mine(&mut header, Target::easy());

        assert_eq!(hash.len(), 32);
    }
}
