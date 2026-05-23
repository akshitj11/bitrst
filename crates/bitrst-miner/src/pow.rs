//! Nonce search loop for proof-of-work mining.

use bitrst_core::pow::Target;
use bitrst_core::BlockHeader;

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

/// Mines a block header using the compact target encoded in `header.bits`.
///
/// Returns `None` when `header.bits` does not decode to a valid compact target.
pub fn mine_with_header_bits(header: &mut BlockHeader) -> Option<[u8; 32]> {
    let target = Target::from_bits(header.bits)?;
    Some(mine(header, target))
}

#[cfg(test)]
mod tests {
    use super::{mine, mine_with_header_bits};
    use bitrst_core::pow::Target;
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

    #[test]
    fn mine_with_header_bits_rejects_invalid_compact_targets() {
        let mut header = BlockHeader {
            version: 1,
            prev_blockhash: [0u8; 32],
            merkle_root: [0u8; 32],
            time: 0,
            bits: 0,
            nonce: 0,
        };

        assert_eq!(mine_with_header_bits(&mut header), None);

        header.bits = 0x0180_0000;
        assert_eq!(mine_with_header_bits(&mut header), None);
    }

    #[test]
    fn mine_with_header_bits_mines_with_valid_compact_target() {
        let mut header = BlockHeader {
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

        let target = Target::from_bits(header.bits).expect("test bits should decode");
        let hash = mine_with_header_bits(&mut header).expect("valid bits should mine");

        assert!(target.meets(&hash));
        assert_eq!(header.nonce, 2083236893);
    }
}
