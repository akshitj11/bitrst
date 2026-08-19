//! Observability events emitted during chain updates.

pub use crate::chain_event_journal::{ChainEventCursor, ChainEventCursorError};

/// Reason an orphan block was removed from the pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvictionReason {
    /// Pool reached capacity and this was the oldest entry.
    PoolFull,
}

/// Significant chain events for logging or monitoring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainEvent {
    /// A block was connected to the active chain.
    BlockConnected {
        /// Height on the active chain.
        height: u32,
        /// Block hash.
        hash: [u8; 32],
        /// Number of transactions in the block.
        tx_count: usize,
    },
    /// A block was disconnected during a reorg.
    BlockDisconnected {
        /// Height that was removed.
        height: u32,
        /// Block hash.
        hash: [u8; 32],
    },
    /// The active tip changed to a heavier fork.
    ChainReorg {
        /// Number of blocks disconnected from the old tip.
        depth: u32,
        /// Previous tip hash.
        old_tip: [u8; 32],
        /// New tip hash.
        new_tip: [u8; 32],
    },
    /// An orphan block was stored.
    OrphanAdded {
        /// Orphan block hash.
        hash: [u8; 32],
        /// Current orphan pool size.
        pool_size: usize,
    },
    /// An orphan was evicted from the pool.
    OrphanEvicted {
        /// Evicted block hash.
        hash: [u8; 32],
        /// Why the orphan left the pool.
        reason: EvictionReason,
    },
}
