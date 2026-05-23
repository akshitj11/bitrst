//! Block storage abstraction for persistence (in-memory implementation for M4.5).

use thiserror::Error;

use crate::block::Block;

/// Errors from block storage backends.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum StoreError {
    /// The requested block was not found.
    #[error("block not found")]
    NotFound,

    /// Storage failed to commit atomically.
    #[error("storage commit failed")]
    CommitFailed,
}

/// Persistent or in-memory block storage.
pub trait BlockStore: Send + Sync {
    /// Stores a block keyed by its hash.
    fn put_block(&mut self, block: &Block) -> Result<(), StoreError>;

    /// Loads a block by hash.
    fn get_block(&self, hash: &[u8; 32]) -> Result<Option<Block>, StoreError>;

    /// Flushes pending writes (no-op for in-memory store).
    fn commit(&mut self) -> Result<(), StoreError>;
}

/// In-memory block store used for tests and prototyping.
#[derive(Debug, Default)]
pub struct MemoryBlockStore {
    blocks: std::collections::HashMap<[u8; 32], Block>,
}

impl MemoryBlockStore {
    /// Creates an empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl BlockStore for MemoryBlockStore {
    fn put_block(&mut self, block: &Block) -> Result<(), StoreError> {
        self.blocks.insert(block.hash(), block.clone());
        Ok(())
    }

    fn get_block(&self, hash: &[u8; 32]) -> Result<Option<Block>, StoreError> {
        Ok(self.blocks.get(hash).cloned())
    }

    fn commit(&mut self) -> Result<(), StoreError> {
        Ok(())
    }
}
