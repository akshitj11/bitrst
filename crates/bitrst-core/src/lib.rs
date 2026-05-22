//! Core Bitcoin domain types for blocks, transactions, and UTXO state.

/// Block header types and hashing helpers.
pub mod block;
/// Transaction input, output, and serialization types.
pub mod transaction;
/// UTXO set types and mutation helpers.
pub mod utxo;

pub use block::BlockHeader;
pub use transaction::{Transaction, TxInput, TxOutput};
pub use utxo::{OutPoint, UtxoSet};
