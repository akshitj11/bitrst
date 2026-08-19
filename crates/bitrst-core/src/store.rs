//! Block storage abstraction for persistence (in-memory and disk backends).

use std::io;

use thiserror::Error;

use crate::block::Block;

mod disk;

pub use disk::FileBlockStore;

/// Errors from block storage backends.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum StoreError {
    /// The requested block was not found.
    #[error("block not found")]
    NotFound,

    /// Storage failed to commit atomically.
    #[error("storage commit failed")]
    CommitFailed,

    /// A storage path was rejected as unsafe.
    #[error("invalid storage path")]
    InvalidPath,

    /// Stored bytes could not be decoded as a valid block.
    #[error("corrupt block file: {context}")]
    Corrupt {
        /// Human-readable corruption context.
        context: String,
    },

    /// Stored block hash does not match its filename key.
    #[error("block hash mismatch")]
    HashMismatch {
        /// Hash requested by the caller.
        expected: [u8; 32],
        /// Hash recomputed from file contents.
        actual: [u8; 32],
    },

    /// An I/O operation failed.
    #[error("{context}: {message}")]
    Io {
        /// Operation that failed.
        context: String,
        /// Underlying OS error message.
        message: String,
    },
}

impl StoreError {
    pub(crate) fn io(context: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            context: context.into(),
            message: source.to_string(),
        }
    }

    pub(crate) fn corrupt(context: impl Into<String>) -> Self {
        Self::Corrupt {
            context: context.into(),
        }
    }
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
