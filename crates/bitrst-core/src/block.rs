//! Bitcoin block header primitives.

use bitrst_crypto::sha256d::sha256d;
use serde::{Deserialize, Serialize};

use crate::merkle::merkle_root;
use crate::transaction::{write_compact_size, Transaction};

/// A Bitcoin block header in wire-serialization field order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// A complete Bitcoin block: an 80-byte header followed by ordered transactions.
///
/// The header's `merkle_root` commits to the block's transaction list via a binary
/// Merkle tree. Bitcoin duplicates the final hash at each level when a level has
/// an odd number of nodes, so inclusion proofs remain well-defined.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    /// Block header including proof-of-work fields and the Merkle root commitment.
    pub header: BlockHeader,
    /// Transactions included in this block, in the order they appear on the wire.
    pub transactions: Vec<Transaction>,
}

impl Block {
    /// Creates a block from a header and its ordered transaction list.
    pub fn new(header: BlockHeader, transactions: Vec<Transaction>) -> Self {
        Self {
            header,
            transactions,
        }
    }

    /// Builds a single-transaction coinbase block and sets the header Merkle root.
    ///
    /// The Merkle root is recomputed from the coinbase transaction ID so the header
    /// commitment matches the transaction list.
    pub fn coinbase(mut header: BlockHeader, height: u32, reward: u64) -> Self {
        let transactions = vec![Transaction::coinbase(height, reward)];
        if let Some(root) = Self::merkle_root_from_transactions(&transactions) {
            header.merkle_root = root;
        }

        Self {
            header,
            transactions,
        }
    }

    /// Returns the serialized block size in bytes (wire format).
    ///
    /// Computed analytically without allocating a serialization buffer.
    pub fn serialized_size(&self) -> usize {
        use crate::transaction::compact_size_encoded_len;

        80 + compact_size_encoded_len(self.transactions.len() as u64)
            + self
                .transactions
                .iter()
                .map(Transaction::serialized_size)
                .sum::<usize>()
    }

    /// Serializes the full block in Bitcoin P2P wire format.
    ///
    /// Wire layout: 80-byte header, transaction count as compact-size, then each
    /// transaction in order.
    pub fn serialize(&self) -> Vec<u8> {
        let mut out = self.header.serialize().to_vec();
        write_compact_size(self.transactions.len() as u64, &mut out);

        for transaction in &self.transactions {
            out.extend_from_slice(&transaction.serialize());
        }

        out
    }

    /// Returns the block hash (SHA-256d of the header only).
    pub fn hash(&self) -> [u8; 32] {
        self.header.hash()
    }

    /// Computes the Merkle root commitment over this block's transaction IDs.
    pub fn merkle_root(&self) -> Option<[u8; 32]> {
        Self::merkle_root_from_transactions(&self.transactions)
    }

    /// Returns true when the header's Merkle root matches the transaction list.
    pub fn header_merkle_root_matches(&self) -> bool {
        self.merkle_root()
            .is_some_and(|root| root == self.header.merkle_root)
    }

    fn merkle_root_from_transactions(transactions: &[Transaction]) -> Option<[u8; 32]> {
        if transactions.is_empty() {
            return None;
        }

        let txids: Vec<[u8; 32]> = transactions.iter().map(Transaction::txid).collect();
        merkle_root(&txids)
    }
}

#[cfg(test)]
mod tests {
    use super::{Block, BlockHeader};
    use crate::Transaction;

    fn to_bitcoin_hex(bytes: [u8; 32]) -> String {
        let mut reversed = bytes;
        reversed.reverse();
        hex::encode(reversed)
    }

    fn sample_header() -> BlockHeader {
        BlockHeader {
            version: 1,
            prev_blockhash: [0u8; 32],
            merkle_root: [0u8; 32],
            time: 1231006505,
            bits: 0x1d00_ffff,
            nonce: 0,
        }
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

    #[test]
    fn coinbase_block_sets_matching_merkle_root() {
        let block = Block::coinbase(sample_header(), 1, 50_0000_0000);

        assert_eq!(block.transactions.len(), 1);
        assert!(block.header_merkle_root_matches());
    }

    #[test]
    fn single_transaction_block_uses_txid_as_merkle_root() {
        let tx = Transaction::coinbase(1, 50_0000_0000);
        let txid = tx.txid();
        let mut header = sample_header();
        header.merkle_root = txid;

        let block = Block::new(header, vec![tx]);

        assert!(block.header_merkle_root_matches());
        assert_eq!(
            block
                .merkle_root()
                .expect("single-tx block should have a root"),
            txid
        );
    }

    #[test]
    fn serializes_header_then_transactions() {
        let block = Block::coinbase(sample_header(), 1, 50_0000_0000);
        let serialized = block.serialize();

        assert_eq!(serialized.len(), block.serialized_size());
        assert_eq!(
            serialized.len(),
            80 + 1 + block.transactions[0].serialize().len()
        );
        assert_eq!(&serialized[..80], block.header.serialize());
    }

    #[test]
    fn block_roundtrips_through_serde_json() {
        let block = Block::coinbase(sample_header(), 1, 50_0000_0000);
        let json = serde_json::to_string(&block).expect("block should serialize to json");
        let decoded: Block = serde_json::from_str(&json).expect("block should deserialize");

        assert_eq!(decoded, block);
    }
}
