//! Core Bitcoin domain types for blocks, transactions, and UTXO state.

/// Block header types and hashing helpers.
pub mod block;
/// Block chain validation and fork choice.
pub mod chain;
/// Difficulty adjustment over 2,016-block periods.
pub mod difficulty;
/// Merkle tree helpers for transaction inclusion commitments.
pub mod merkle;
/// Proof-of-work target decoding and comparison.
pub mod pow;
/// Block timestamp validation helpers.
pub mod time;
/// Transaction input, output, and serialization types.
pub mod transaction;
/// UTXO set types and mutation helpers.
pub mod utxo;

pub use block::{Block, BlockHeader};
pub use chain::{Chain, ChainError, ConnectResult};
pub use merkle::merkle_root;
pub use pow::Target;
pub use transaction::{Transaction, TxInput, TxOutput};
pub use utxo::{OutPoint, TxUndo, UtxoError, UtxoSet};
