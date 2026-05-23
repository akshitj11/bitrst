//! Proof-of-work target decoding and comparison.
//!
//! Bitcoin stores difficulty as a 4-byte compact `bits` field in each block header.
//! Miners must produce a header hash numerically below the decoded 256-bit target.

use std::cmp::Ordering;

/// A proof-of-work target decoded from Bitcoin's compact `bits` format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    threshold: [u8; 32],
}

impl Target {
    /// Wraps a decoded 32-byte target threshold in internal byte order.
    pub fn from_threshold(threshold: [u8; 32]) -> Self {
        Self { threshold }
    }

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

    /// Returns the per-block proof-of-work value used for cumulative chain work.
    ///
    /// Matches Bitcoin Core `GetBlockProof`: `(~target / (target + 1)) + 1`.
    /// Reference: <https://github.com/bitcoin/bitcoin/blob/master/src/chain.cpp>
    pub fn to_work(&self) -> Option<[u8; 32]> {
        crate::uint256::work_from_target(self.threshold)
    }

    /// Encodes this target as Bitcoin's compact `bits` representation.
    ///
    /// Returns `None` when the target is zero or cannot be represented in the
    /// compact floating-point format used by block headers.
    pub fn to_bits(&self) -> Option<u32> {
        let mut size = significant_size(&self.threshold)? as u32;
        let mut mantissa = if size <= 3 {
            let mut value = 0u64;
            for index in 0..size as usize {
                value |= u64::from(self.threshold[index]) << (8 * index);
            }
            (value << (8 * (3 - size))) as u32
        } else {
            let offset = (size - 3) as usize;
            u32::from(self.threshold[offset])
                | (u32::from(self.threshold[offset + 1]) << 8)
                | (u32::from(self.threshold[offset + 2]) << 16)
        };

        // Bitcoin shifts the mantissa and bumps the exponent when bit 23 is set.
        if mantissa & 0x0080_0000 != 0 {
            mantissa >>= 8;
            size += 1;
        }

        if mantissa > 0x007f_ffff {
            return None;
        }

        Some((size << 24) | mantissa)
    }
}

fn significant_size(threshold: &[u8; 32]) -> Option<usize> {
    for index in (0..32).rev() {
        if threshold[index] != 0 {
            return Some(index + 1);
        }
    }

    None
}

fn compare_internal_hash(left: &[u8; 32], right: &[u8; 32]) -> Ordering {
    left.iter().rev().cmp(right.iter().rev())
}

#[cfg(test)]
mod tests {
    use super::Target;
    use crate::BlockHeader;

    #[test]
    fn genesis_compact_bits_roundtrip() {
        let bits = 0x1d00_ffff;
        let target = Target::from_bits(bits).expect("genesis bits should decode");

        assert_eq!(target.to_bits(), Some(bits));
    }

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
    fn genesis_header_meets_decoded_target() {
        let header = BlockHeader {
            version: 1,
            prev_blockhash: [0u8; 32],
            merkle_root: [
                0x3b, 0xa3, 0xed, 0xfd, 0x7a, 0x7b, 0x12, 0xb2, 0x7a, 0xc7, 0x2c, 0x3e, 0x67, 0x76,
                0x8f, 0x61, 0x7f, 0xc8, 0x1b, 0xc3, 0x88, 0x8a, 0x51, 0x32, 0x3a, 0x9f, 0xb8, 0xaa,
                0x4b, 0x1e, 0x5e, 0x4a,
            ],
            time: 1231006505,
            bits: 0x1d00_ffff,
            nonce: 2083236893,
        };

        let target = Target::from_bits(header.bits).expect("genesis bits should decode");

        assert!(target.meets(&header.hash()));
    }
}
