//! Core Bitcoin domain types for blocks, transactions, and UTXO state.

/// Block header types and hashing helpers.
pub mod block;
/// Block chain validation and fork choice.
pub mod chain;
/// Chain observability events.
pub mod chain_events;
/// Thread-safe chain access.
pub mod chain_handle;
/// Difficulty adjustment over 2,016-block periods.
pub mod difficulty;
/// Protocol size and DoS limits.
pub mod limits;
/// Merkle tree helpers for transaction inclusion commitments.
pub mod merkle;
/// Proof-of-work target decoding and comparison.
pub mod pow;
/// Block storage trait and in-memory implementation.
pub mod store;
/// Block timestamp validation helpers.
pub mod time;
/// Transaction input, output, and serialization types.
pub mod transaction;
/// 256-bit integer helpers for chain work.
pub mod uint256;
/// UTXO set types and mutation helpers.
pub mod utxo;

pub use block::{Block, BlockHeader};
pub use chain::{block_work, Chain, ChainError, ChainWork, ConnectResult};
pub use chain_events::{ChainEvent, EvictionReason};
pub use chain_handle::ChainHandle;
pub use merkle::merkle_root;
pub use pow::Target;
pub use store::{BlockStore, MemoryBlockStore, StoreError};
pub use transaction::{Transaction, TxInput, TxOutput};
pub use utxo::{OutPoint, TxUndo, UtxoError, UtxoSet};
