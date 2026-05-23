//! Thread-safe access to chain state.

use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::block::Block;
use crate::chain::{Chain, ChainError, ConnectResult};

/// Shared handle to a [`Chain`] for concurrent readers and exclusive writers.
#[derive(Debug, Clone)]
pub struct ChainHandle {
    inner: Arc<RwLock<Chain>>,
}

impl ChainHandle {
    /// Wraps a new in-memory chain.
    pub fn new(chain: Chain) -> Self {
        Self {
            inner: Arc::new(RwLock::new(chain)),
        }
    }

    /// Creates a genesis chain and wraps it.
    ///
    /// # Errors
    ///
    /// Returns [`ChainError`] when genesis validation fails.
    pub fn new_genesis(genesis: Block, network_time: u32) -> Result<Self, ChainError> {
        Ok(Self::new(Chain::new_genesis(genesis, network_time)?))
    }

    /// Returns the active tip hash (shared read lock).
    pub fn tip_hash(&self) -> Result<[u8; 32], ChainError> {
        Ok(self.read()?.tip_hash())
    }

    /// Returns the active chain height.
    pub fn height(&self) -> Result<u32, ChainError> {
        Ok(self.read()?.height())
    }

    /// Connects a block exclusively.
    pub fn connect_block(&self, block: Block) -> Result<ConnectResult, ChainError> {
        self.write()?.connect_block(block)
    }

    /// Updates network-adjusted time used for future-drift checks.
    pub fn set_network_time(&self, network_time: u32) -> Result<(), ChainError> {
        self.write()?.set_network_time(network_time);
        Ok(())
    }

    fn read(&self) -> Result<RwLockReadGuard<'_, Chain>, ChainError> {
        self.inner.read().map_err(|_| ChainError::LockPoisoned)
    }

    fn write(&self) -> Result<RwLockWriteGuard<'_, Chain>, ChainError> {
        self.inner.write().map_err(|_| ChainError::LockPoisoned)
    }
}
