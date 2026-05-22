//! Core Bitcoin domain types for blocks, transactions, and UTXO state.

/// Block header types and hashing helpers.
pub mod block;
/// Merkle tree helpers for transaction inclusion commitments.
pub mod merkle;
/// Transaction input, output, and serialization types.
pub mod transaction;
/// UTXO set types and mutation helpers.
pub mod utxo;

pub use block::BlockHeader;
pub use merkle::merkle_root;
pub use transaction::{Transaction, TxInput, TxOutput};
pub use utxo::{OutPoint, UtxoSet};
