//! Bounded journal of blocks disconnected from the active chain during reorgs.

use std::collections::VecDeque;

use thiserror::Error;

use crate::block::Block;
use crate::limits::DEFAULT_DISCONNECTED_BLOCK_JOURNAL_CAPACITY;

/// A block disconnected from the active chain, keyed by its event sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
struct JournalEntry {
    event_seq: u64,
    block: Block,
}

/// Errors when recovery falls behind the retained disconnected-block window.
#[derive(Debug, Error, PartialEq, Eq)]
#[error(
    "disconnected block recovery at event seq {cursor_seq} lags retained journal starting at {oldest_available}"
)]
pub struct DisconnectedBlockRecoveryError {
    /// Last event sequence the consumer successfully replayed.
    pub cursor_seq: u64,
    /// Oldest event sequence still retained in the journal.
    pub oldest_available: u64,
}

/// Ring journal of recently disconnected active-chain blocks for mempool recovery.
#[derive(Debug, Clone)]
pub struct DisconnectedBlockJournal {
    entries: VecDeque<JournalEntry>,
    capacity: usize,
}

impl DisconnectedBlockJournal {
    /// Creates an empty journal with the given retention capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    /// Records `block` as disconnected at `event_seq`.
    pub fn record(&mut self, event_seq: u64, block: Block) {
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(JournalEntry { event_seq, block });
    }

    /// Returns disconnected blocks with event sequence greater than `since_event_seq`.
    ///
    /// # Errors
    ///
    /// Returns [`DisconnectedBlockRecoveryError`] when `since_event_seq` is older than
    /// the retained window, so exact recovery is impossible.
    pub fn blocks_since(
        &self,
        since_event_seq: u64,
    ) -> Result<Vec<Block>, DisconnectedBlockRecoveryError> {
        if let Some(oldest) = self.entries.front() {
            if since_event_seq < oldest.event_seq.saturating_sub(1) {
                return Err(DisconnectedBlockRecoveryError {
                    cursor_seq: since_event_seq,
                    oldest_available: oldest.event_seq,
                });
            }
        }

        Ok(self
            .entries
            .iter()
            .filter(|entry| entry.event_seq > since_event_seq)
            .map(|entry| entry.block.clone())
            .collect())
    }
}

impl Default for DisconnectedBlockJournal {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_DISCONNECTED_BLOCK_JOURNAL_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::{DisconnectedBlockJournal, DisconnectedBlockRecoveryError};
    use crate::block::{Block, BlockHeader};
    use crate::pow::Target;

    const TEST_BITS: u32 = 0x1f00_ffff;

    fn block_at(height: u32) -> Block {
        let header = BlockHeader {
            version: 1,
            prev_blockhash: [height as u8; 32],
            merkle_root: [0u8; 32],
            time: 1_231_006_505 + height,
            bits: TEST_BITS,
            nonce: 0,
        };
        let mut block = Block::coinbase(header, height, 50_0000_0000);
        block.header.merkle_root = block.merkle_root().expect("merkle");
        let target = Target::from_bits(TEST_BITS).expect("bits");
        while !target.meets(&block.header.hash()) {
            block.header.nonce = block.header.nonce.wrapping_add(1);
        }
        block
    }

    #[test]
    fn blocks_since_returns_entries_after_cursor() {
        let mut journal = DisconnectedBlockJournal::with_capacity(4);
        journal.record(2, block_at(1));
        journal.record(5, block_at(2));

        let blocks = journal.blocks_since(3).expect("recover");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].hash(), block_at(2).hash());
    }

    #[test]
    fn overrun_returns_recovery_error() {
        let mut journal = DisconnectedBlockJournal::with_capacity(2);
        journal.record(1, block_at(1));
        journal.record(2, block_at(2));
        journal.record(3, block_at(3));

        let err = journal.blocks_since(0).expect_err("lag");
        assert_eq!(
            err,
            DisconnectedBlockRecoveryError {
                cursor_seq: 0,
                oldest_available: 2,
            }
        );
    }
}
