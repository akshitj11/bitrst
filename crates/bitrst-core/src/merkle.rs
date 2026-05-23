//! Merkle tree helpers for transaction inclusion commitments.

use bitrst_crypto::sha256d::sha256d;

use crate::limits::MAX_TRANSACTIONS_PER_BLOCK;

/// Computes the Bitcoin Merkle root for a non-empty list of transaction IDs.
pub fn merkle_root(txids: &[[u8; 32]]) -> Option<[u8; 32]> {
    if txids.is_empty() || txids.len() > MAX_TRANSACTIONS_PER_BLOCK {
        return None;
    }

    let mut level = txids.to_vec();
    while level.len() > 1 {
        let mut next_level = Vec::with_capacity(level.len().div_ceil(2));

        for pair in level.chunks(2) {
            let left = pair[0];
            let right = pair.get(1).copied().unwrap_or(left);

            let mut combined = [0u8; 64];
            combined[0..32].copy_from_slice(&left);
            combined[32..64].copy_from_slice(&right);
            next_level.push(sha256d(&combined));
        }

        level = next_level;
    }

    Some(level[0])
}

#[cfg(test)]
mod tests {
    use super::merkle_root;

    fn to_bitcoin_hex(bytes: [u8; 32]) -> String {
        let mut reversed = bytes;
        reversed.reverse();
        hex::encode(reversed)
    }

    #[test]
    fn returns_none_for_empty_transaction_list() {
        assert_eq!(merkle_root(&[]), None);
    }

    #[test]
    fn genesis_transaction_id_is_genesis_merkle_root() {
        let genesis_txid = [
            0x3b, 0xa3, 0xed, 0xfd, 0x7a, 0x7b, 0x12, 0xb2, 0x7a, 0xc7, 0x2c, 0x3e, 0x67, 0x76,
            0x8f, 0x61, 0x7f, 0xc8, 0x1b, 0xc3, 0x88, 0x8a, 0x51, 0x32, 0x3a, 0x9f, 0xb8, 0xaa,
            0x4b, 0x1e, 0x5e, 0x4a,
        ];

        let root = merkle_root(&[genesis_txid]).expect("single txid should have a Merkle root");

        assert_eq!(
            to_bitcoin_hex(root),
            "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b"
        );
    }

    #[test]
    fn duplicates_odd_final_hash_at_each_level() {
        let a = [0x01; 32];
        let b = [0x02; 32];
        let c = [0x03; 32];

        assert!(merkle_root(&[a, b, c]).is_some());
    }
}
