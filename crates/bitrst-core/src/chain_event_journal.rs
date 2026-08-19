//! Bounded multi-consumer journal for chain observability events.

use std::collections::VecDeque;

use thiserror::Error;

use crate::chain_events::ChainEvent;
use crate::limits::DEFAULT_CHAIN_EVENT_JOURNAL_CAPACITY;

/// A retained chain event tagged with a monotonic sequence number.
#[derive(Debug, Clone, PartialEq, Eq)]
struct JournalEntry {
    seq: u64,
    event: ChainEvent,
}

/// Cursor position for non-destructive event collection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChainEventCursor {
    pub(crate) last_seq: u64,
}

/// Errors when a cursor has fallen behind the retained journal window.
#[derive(Debug, Error, PartialEq, Eq)]
#[error(
    "chain event cursor at seq {cursor_seq} lags retained journal starting at {oldest_available}"
)]
pub struct ChainEventCursorError {
    /// Last sequence successfully collected by the cursor.
    pub cursor_seq: u64,
    /// Oldest sequence still retained in the journal.
    pub oldest_available: u64,
}

/// Bounded ring journal of chain events with independent wallet and cursor consumers.
#[derive(Debug, Clone)]
pub struct ChainEventJournal {
    next_seq: u64,
    entries: VecDeque<JournalEntry>,
    capacity: usize,
    wallet_high_water: u64,
}

impl ChainEventJournal {
    /// Creates an empty journal with the given retention capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            next_seq: 1,
            entries: VecDeque::new(),
            capacity: capacity.max(1),
            wallet_high_water: 0,
        }
    }

    /// Appends `event` and returns its assigned sequence number.
    pub fn push(&mut self, event: ChainEvent) -> u64 {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(JournalEntry { seq, event });
        seq
    }

    /// Returns the oldest sequence still retained, if any.
    pub fn oldest_seq(&self) -> Option<u64> {
        self.entries.front().map(|entry| entry.seq)
    }

    /// Returns the highest assigned sequence, or zero when empty.
    pub fn latest_seq(&self) -> u64 {
        self.entries.back().map(|entry| entry.seq).unwrap_or(0)
    }

    /// Returns a cursor positioned at the current journal tail.
    pub fn event_cursor(&self) -> ChainEventCursor {
        ChainEventCursor {
            last_seq: self.latest_seq(),
        }
    }

    /// Drains wallet-visible events without removing them from the journal.
    pub fn take_events(&mut self) -> Vec<ChainEvent> {
        let events: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| entry.seq > self.wallet_high_water)
            .map(|entry| entry.event.clone())
            .collect();
        if let Some(last) = self.entries.back() {
            self.wallet_high_water = last.seq;
        }
        events
    }

    /// Collects events after `cursor`, advancing the cursor on success.
    pub fn collect_events(
        &self,
        cursor: &mut ChainEventCursor,
    ) -> Result<Vec<ChainEvent>, ChainEventCursorError> {
        if let Some(oldest) = self.oldest_seq() {
            if cursor.last_seq < oldest.saturating_sub(1) {
                return Err(ChainEventCursorError {
                    cursor_seq: cursor.last_seq,
                    oldest_available: oldest,
                });
            }
        }

        let events: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| entry.seq > cursor.last_seq)
            .map(|entry| entry.event.clone())
            .collect();
        if let Some(last) = self
            .entries
            .iter()
            .rfind(|entry| entry.seq > cursor.last_seq)
        {
            cursor.last_seq = last.seq;
        }
        Ok(events)
    }
}

impl Default for ChainEventJournal {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_CHAIN_EVENT_JOURNAL_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::{ChainEventCursor, ChainEventJournal};
    use crate::chain_events::ChainEvent;

    fn connected(height: u32) -> ChainEvent {
        ChainEvent::BlockConnected {
            height,
            hash: [height as u8; 32],
            tx_count: 1,
        }
    }

    #[test]
    fn wallet_take_before_mempool_cursor_still_replays() {
        let mut journal = ChainEventJournal::with_capacity(8);
        journal.push(connected(0));

        let wallet = journal.take_events();
        assert_eq!(wallet.len(), 1);

        let mut cursor = ChainEventCursor::default();
        let mempool = journal.collect_events(&mut cursor).expect("collect");
        assert_eq!(mempool.len(), 1);
    }

    #[test]
    fn two_cursors_collect_independently() {
        let mut journal = ChainEventJournal::with_capacity(8);
        journal.push(connected(1));

        let mut first = ChainEventCursor::default();
        assert_eq!(journal.collect_events(&mut first).expect("first").len(), 1);

        let mut second = journal.event_cursor();
        assert!(journal
            .collect_events(&mut second)
            .expect("second")
            .is_empty());

        journal.push(connected(2));
        assert_eq!(
            journal.collect_events(&mut second).expect("second").len(),
            1
        );
        assert_eq!(journal.collect_events(&mut first).expect("first").len(), 1);
    }

    #[test]
    fn overrun_returns_lag_error() {
        let mut journal = ChainEventJournal::with_capacity(2);
        let mut cursor = ChainEventCursor::default();
        journal.push(connected(1));
        journal.push(connected(2));
        journal.push(connected(3));

        let err = journal.collect_events(&mut cursor).expect_err("lag");
        assert_eq!(err.cursor_seq, 0);
        assert_eq!(err.oldest_available, 2);
    }

    #[test]
    fn caught_up_cursor_advances_after_journal_wrap() {
        let mut journal = ChainEventJournal::with_capacity(2);
        let mut cursor = ChainEventCursor::default();
        journal.push(connected(1));
        journal.collect_events(&mut cursor).expect("first");

        journal.push(connected(2));
        journal.push(connected(3));
        let collected = journal.collect_events(&mut cursor).expect("after wrap");
        assert_eq!(collected.len(), 2);
        assert_eq!(cursor.last_seq, 3);
    }

    #[test]
    fn take_events_is_idempotent_until_new_events() {
        let mut journal = ChainEventJournal::with_capacity(4);
        journal.push(connected(1));
        assert_eq!(journal.take_events().len(), 1);
        assert!(journal.take_events().is_empty());
        journal.push(connected(2));
        assert_eq!(journal.take_events().len(), 1);
    }

    #[test]
    fn rollback_events_remain_available_to_lagging_cursor() {
        let mut journal = ChainEventJournal::with_capacity(8);
        journal.push(connected(1));
        journal.push(connected(2));

        let mut cursor = ChainEventCursor::default();
        journal.collect_events(&mut cursor).expect("initial");

        journal.push(ChainEvent::BlockDisconnected {
            height: 2,
            hash: [0x02; 32],
        });
        journal.push(ChainEvent::ChainReorg {
            depth: 1,
            old_tip: [0xaa; 32],
            new_tip: [0xbb; 32],
        });
        journal.push(connected(2));

        let collected = journal.collect_events(&mut cursor).expect("rollback");
        assert_eq!(collected.len(), 3);
        assert!(matches!(
            collected[0],
            ChainEvent::BlockDisconnected { height: 2, .. }
        ));
        assert!(matches!(collected[1], ChainEvent::ChainReorg { .. }));
        assert!(matches!(
            collected[2],
            ChainEvent::BlockConnected { height: 2, .. }
        ));
    }
}
