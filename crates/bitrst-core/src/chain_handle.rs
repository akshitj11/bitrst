//! Thread-safe access to chain state.

use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::block::Block;
use crate::chain::{Chain, ChainError, ConnectResult};
use crate::chain_events::{ChainEvent, ChainEventCursor};

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

    /// Returns a cursor positioned at the end of the current event log.
    pub fn event_cursor(&self) -> Result<ChainEventCursor, ChainError> {
        Ok(self.read()?.event_cursor())
    }

    /// Returns events appended since `cursor` and advances the cursor.
    pub fn collect_events(
        &self,
        cursor: &mut ChainEventCursor,
    ) -> Result<Vec<ChainEvent>, ChainError> {
        Ok(self.write()?.collect_events(cursor)?)
    }

    /// Returns and clears pending chain events.
    ///
    /// # Errors
    ///
    /// Returns [`ChainError::LockPoisoned`] if another thread panicked while holding the lock.
    pub fn take_events(&self) -> Result<Vec<ChainEvent>, ChainError> {
        Ok(self.write()?.take_events())
    }

    /// Returns true when the node already knows `hash`.
    pub fn has_block(&self, hash: &[u8; 32]) -> Result<bool, ChainError> {
        Ok(self.read()?.has_block_hash(hash))
    }

    /// Returns a known block by hash, if present.
    pub fn get_block(&self, hash: &[u8; 32]) -> Result<Option<Block>, ChainError> {
        Ok(self.read()?.block_by_hash(hash))
    }

    /// Invokes `f` with shared read access to the active chain state.
    pub fn with_chain<R>(&self, f: impl FnOnce(&Chain) -> R) -> Result<R, ChainError> {
        let guard = self.read()?;
        Ok(f(&guard))
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
    use crate::chain_events::ChainEventCursor;
    use crate::{Block, BlockHeader, ChainEvent, Target};

    const TEST_BITS: u32 = 0x1f00_ffff;
    const NETWORK_TIME: u32 = 1_231_006_505;

    #[test]
    fn take_events_returns_and_clears_genesis_event() {
        let handle = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");

        let events = handle.take_events().expect("events");

        assert_eq!(events.len(), 1);
        assert!(handle.take_events().expect("events").is_empty());
    }

    #[test]
    fn get_block_returns_genesis_by_hash() {
        let genesis = genesis_block();
        let hash = genesis.hash();
        let handle = ChainHandle::new_genesis(genesis, NETWORK_TIME).expect("genesis");

        assert!(handle.has_block(&hash).expect("has"));
        assert_eq!(
            handle.get_block(&hash).expect("get").expect("some").hash(),
            hash
        );
    }

    #[test]
    fn collect_events_is_non_destructive_and_survives_take_events() {
        let genesis = genesis_block();
        let handle = ChainHandle::new_genesis(genesis.clone(), NETWORK_TIME).expect("genesis");
        let mut cursor = ChainEventCursor::default();

        let collected = handle.collect_events(&mut cursor).expect("collect genesis");
        assert_eq!(collected.len(), 1);
        assert!(handle
            .collect_events(&mut cursor)
            .expect("again")
            .is_empty());
        assert_eq!(handle.take_events().expect("wallet").len(), 1);

        handle
            .connect_block(child_block(&genesis))
            .expect("connect");

        let collected = handle.collect_events(&mut cursor).expect("collect connect");
        assert_eq!(collected.len(), 1);
        assert!(matches!(
            collected[0],
            ChainEvent::BlockConnected { height: 1, .. }
        ));
    }

    fn child_block(parent: &Block) -> Block {
        let header = BlockHeader {
            version: 1,
            prev_blockhash: parent.hash(),
            merkle_root: [0u8; 32],
            time: NETWORK_TIME + 600,
            bits: TEST_BITS,
            nonce: 0,
        };
        let mut block = Block::coinbase(header, 1, 50_0000_0000);
        block.header.merkle_root = block.merkle_root().expect("merkle");
        let target = Target::from_bits(TEST_BITS).expect("test bits");
        while !target.meets(&block.header.hash()) {
            block.header.nonce = block.header.nonce.wrapping_add(1);
        }
        block
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
