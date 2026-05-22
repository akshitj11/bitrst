//! Proof-of-work mining primitives.

use bitrst_core::BlockHeader;

/// A compact proof-of-work target used by the first miner pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    threshold: [u8; 32],
}

impl Target {
    /// Creates an easy target for learning and testing.
    pub fn easy() -> Self {
        Self {
            threshold: [0xff; 32],
        }
    }

    /// Returns true when the hash is at or below the target threshold.
    pub fn meets(&self, hash: &[u8; 32]) -> bool {
        hash <= &self.threshold
    }
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
