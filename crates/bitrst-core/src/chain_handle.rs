//! Thread-safe access to chain state.

use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::block::Block;
use crate::chain::{Chain, ChainError, ConnectResult};
use crate::chain_events::ChainEvent;

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
        self.write()?.set_network_time(network_time)
    }

    /// Returns and clears pending chain events.
    ///
    /// # Errors
    ///
    /// Returns [`ChainError::LockPoisoned`] if another thread panicked while holding the lock.
    pub fn take_events(&self) -> Result<Vec<ChainEvent>, ChainError> {
        Ok(self.write()?.take_events())
    }

    fn read(&self) -> Result<RwLockReadGuard<'_, Chain>, ChainError> {
        self.inner.read().map_err(|_| ChainError::LockPoisoned)
    }

    fn write(&self) -> Result<RwLockWriteGuard<'_, Chain>, ChainError> {
        self.inner.write().map_err(|_| ChainError::LockPoisoned)
    }
}

#[cfg(test)]
mod tests {
    use super::ChainHandle;
    use crate::{Block, BlockHeader, Target};

    const TEST_BITS: u32 = 0x1f00_ffff;
    const NETWORK_TIME: u32 = 1_231_006_505;

    #[test]
    fn take_events_returns_and_clears_genesis_event() {
        let handle = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");

        let events = handle.take_events().expect("events");

        assert_eq!(events.len(), 1);
        assert!(handle.take_events().expect("events").is_empty());
    }

    fn genesis_block() -> Block {
        let header = BlockHeader {
            version: 1,
            prev_blockhash: [0u8; 32],
            merkle_root: [0u8; 32],
            time: NETWORK_TIME,
            bits: TEST_BITS,
            nonce: 0,
        };
        let mut block = Block::coinbase(header, 0, 50_0000_0000);
        let target = Target::from_bits(TEST_BITS).expect("test bits");
        while !target.meets(&block.header.hash()) {
            block.header.nonce = block.header.nonce.wrapping_add(1);
        }
        block
    }
}
