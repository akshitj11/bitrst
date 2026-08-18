//! Race-free inbound connection slot reservation.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Tracks and limits concurrent inbound peer handshakes.
#[derive(Debug)]
pub struct InboundCapacity {
    active: AtomicUsize,
    max: usize,
}

impl InboundCapacity {
    /// Creates a tracker allowing up to `max` simultaneous inbound peers.
    #[must_use]
    pub fn new(max: usize) -> Arc<Self> {
        Arc::new(Self {
            active: AtomicUsize::new(0),
            max,
        })
    }

    /// Reserves one inbound slot when capacity remains.
    pub fn try_acquire(self: &Arc<Self>) -> Option<InboundGuard> {
        loop {
            let current = self.active.load(Ordering::Acquire);
            if current >= self.max {
                return None;
            }
            if self
                .active
                .compare_exchange_weak(current, current + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Some(InboundGuard {
                    capacity: Arc::clone(self),
                });
            }
        }
    }

    /// Returns the number of reserved inbound slots.
    #[must_use]
    pub fn reserved(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }

    fn release(&self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Releases an inbound slot when dropped.
#[derive(Debug)]
pub struct InboundGuard {
    capacity: Arc<InboundCapacity>,
}

impl Drop for InboundGuard {
    fn drop(&mut self) {
        self.capacity.release();
    }
}

#[cfg(test)]
mod tests {
    use super::InboundCapacity;
    use std::sync::Arc;

    #[test]
    fn acquire_respects_max_and_releases_on_drop() {
        let capacity = InboundCapacity::new(2);
        let first = capacity.try_acquire().expect("first slot");
        let second = capacity.try_acquire().expect("second slot");
        assert!(capacity.try_acquire().is_none());
        assert_eq!(capacity.reserved(), 2);
        drop(first);
        assert_eq!(capacity.reserved(), 1);
        drop(second);
        assert_eq!(capacity.reserved(), 0);
        assert!(capacity.try_acquire().is_some());
    }

    #[test]
    fn concurrent_acquire_never_exceeds_max() {
        let capacity = InboundCapacity::new(4);
        let (tx, rx) = std::sync::mpsc::channel();
        let mut handles = Vec::new();
        for _ in 0..32 {
            let capacity = Arc::clone(&capacity);
            let tx = tx.clone();
            handles.push(std::thread::spawn(move || {
                if let Some(guard) = capacity.try_acquire() {
                    let _ = tx.send(guard);
                }
            }));
        }
        drop(tx);
        let guards: Vec<_> = rx.iter().collect();
        for handle in handles {
            handle.join().expect("join");
        }
        assert_eq!(guards.len(), 4);
        assert_eq!(capacity.reserved(), 4);
    }
}
