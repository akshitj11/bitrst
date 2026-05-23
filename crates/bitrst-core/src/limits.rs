//! Protocol and DoS limits for block and transaction validation.

/// Maximum serialized block size accepted from the network (bytes).
pub const MAX_BLOCK_SERIALIZED_SIZE: usize = 4_000_000;

/// Maximum orphan blocks held while waiting for parents.
pub const MAX_ORPHAN_BLOCKS: usize = 256;

/// Maximum transactions per block (protocol upper bound).
pub const MAX_TRANSACTIONS_PER_BLOCK: usize = 25_000;

/// Maximum script length per push (simplified bound for M4.5).
pub const MAX_SCRIPT_SIZE: usize = 10_000;

/// Maximum satoshis per output (21M BTC).
pub const MAX_MONEY: u64 = 21_000_000 * 100_000_000;
