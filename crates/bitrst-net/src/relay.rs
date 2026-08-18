//! Block relay, inventory handling, and chain integration.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use bitrst_core::{Block, ChainError, ChainHandle, ConnectResult};

use crate::constants::{BLOCK_REQUEST_TTL, MAX_PENDING_BLOCK_REQUESTS};
use crate::message::{InvType, InventoryVector, Message, MessagePayload};

/// Result of handling one post-handshake message from a peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayAction {
    /// No response required.
    None,
    /// Send these messages back to the peer.
    Reply(Vec<Message>),
    /// Broadcast inventory to other peers.
    Announce(Vec<InventoryVector>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingRequest {
    requested_at: Instant,
}

/// Tracks outstanding block `getdata` requests to suppress loops and duplicates.
#[derive(Debug, Clone)]
pub struct BlockRequestTracker {
    pending: HashMap<[u8; 32], PendingRequest>,
    max_pending: usize,
    request_ttl: Duration,
}

impl Default for BlockRequestTracker {
    fn default() -> Self {
        Self::new(MAX_PENDING_BLOCK_REQUESTS, BLOCK_REQUEST_TTL)
    }
}

impl BlockRequestTracker {
    /// Creates a tracker with explicit capacity and expiry policy.
    #[must_use]
    pub fn new(max_pending: usize, request_ttl: Duration) -> Self {
        Self {
            pending: HashMap::new(),
            max_pending,
            request_ttl,
        }
    }

    /// Drops expired entries and returns the number of live requests.
    pub fn expire_before(&mut self, now: Instant) -> usize {
        self.pending.retain(|_, request| {
            now.saturating_duration_since(request.requested_at) < self.request_ttl
        });
        self.pending.len()
    }

    /// Records `hash` as requested. Returns `false` when already pending or at capacity.
    #[must_use]
    pub fn mark_requested(&mut self, hash: &[u8; 32], now: Instant) -> bool {
        self.expire_before(now);
        if self.pending.contains_key(hash) {
            return false;
        }
        if self.pending.len() >= self.max_pending {
            return false;
        }
        self.pending
            .insert(*hash, PendingRequest { requested_at: now });
        true
    }

    /// Clears a completed or abandoned request.
    pub fn clear(&mut self, hash: &[u8; 32]) {
        self.pending.remove(hash);
    }

    /// Returns the number of outstanding requests.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

/// Handles an inbound P2P message against local chain state.
///
/// # Errors
///
/// Returns [`ChainError`] when block connection fails unexpectedly.
pub fn handle_peer_message(
    chain: &ChainHandle,
    message: Message,
    tracker: &mut BlockRequestTracker,
    now: Instant,
) -> Result<RelayAction, ChainError> {
    match message.payload {
        MessagePayload::Block(block) => handle_block(chain, block, tracker, now),
        MessagePayload::Inv(items) => handle_inv(chain, items, tracker, now),
        MessagePayload::GetData(items) => Ok(handle_getdata(chain, items)),
        MessagePayload::Tx(_) => Ok(RelayAction::None),
        MessagePayload::Version(_) | MessagePayload::Verack => Ok(RelayAction::None),
    }
}

fn handle_block(
    chain: &ChainHandle,
    block: Block,
    tracker: &mut BlockRequestTracker,
    now: Instant,
) -> Result<RelayAction, ChainError> {
    let hash = block.hash();
    let parent_hash = block.header.prev_blockhash;
    match chain.connect_block(block) {
        Ok(result) => match result {
            ConnectResult::Connected { .. } | ConnectResult::Reorganized { .. } => {
                tracker.clear(&hash);
                Ok(RelayAction::Announce(vec![InventoryVector {
                    inv_type: InvType::Block,
                    hash,
                }]))
            }
            ConnectResult::Orphaned { .. } => {
                request_missing_block(chain, parent_hash, tracker, now)
            }
            ConnectResult::SideChain { .. } => Ok(RelayAction::None),
        },
        Err(ChainError::BlockAlreadyKnown) => Ok(RelayAction::None),
        Err(error) => {
            tracker.clear(&hash);
            tracker.clear(&parent_hash);
            Err(error)
        }
    }
}

fn request_missing_block(
    chain: &ChainHandle,
    hash: [u8; 32],
    tracker: &mut BlockRequestTracker,
    now: Instant,
) -> Result<RelayAction, ChainError> {
    if chain.has_block(&hash)? || !tracker.mark_requested(&hash, now) {
        return Ok(RelayAction::None);
    }
    Ok(RelayAction::Reply(vec![Message::getdata(vec![
        InventoryVector {
            inv_type: InvType::Block,
            hash,
        },
    ])]))
}

fn handle_inv(
    chain: &ChainHandle,
    items: Vec<InventoryVector>,
    tracker: &mut BlockRequestTracker,
    now: Instant,
) -> Result<RelayAction, ChainError> {
    let mut requests = Vec::new();
    for item in items {
        if item.inv_type != InvType::Block {
            continue;
        }
        if chain.has_block(&item.hash)? || !tracker.mark_requested(&item.hash, now) {
            continue;
        }
        requests.push(item);
    }
    if requests.is_empty() {
        Ok(RelayAction::None)
    } else {
        Ok(RelayAction::Reply(vec![Message::getdata(requests)]))
    }
}

fn handle_getdata(chain: &ChainHandle, items: Vec<InventoryVector>) -> RelayAction {
    let mut replies = Vec::new();
    for item in items {
        if item.inv_type != InvType::Block {
            continue;
        }
        if let Ok(Some(block)) = chain.get_block(&item.hash) {
            replies.push(Message::block(block));
        }
    }
    if replies.is_empty() {
        RelayAction::None
    } else {
        RelayAction::Reply(replies)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{handle_peer_message, BlockRequestTracker};
    use crate::message::{InvType, InventoryVector, Message};
    use crate::testutil::{child_block, genesis_block, orphan_block, NETWORK_TIME};
    use crate::RelayAction;
    use bitrst_core::ChainHandle;

    fn now() -> std::time::Instant {
        std::time::Instant::now()
    }

    #[test]
    fn inv_requests_unknown_blocks_only() {
        let chain = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        let mut tracker = BlockRequestTracker::default();
        let unknown = [9u8; 32];
        let action = handle_peer_message(
            &chain,
            Message::inv(vec![InventoryVector {
                inv_type: InvType::Block,
                hash: unknown,
            }]),
            &mut tracker,
            now(),
        )
        .expect("handle inv");
        assert_eq!(
            action,
            RelayAction::Reply(vec![Message::getdata(vec![InventoryVector {
                inv_type: InvType::Block,
                hash: unknown,
            }])])
        );
    }

    #[test]
    fn inv_suppresses_duplicate_requests() {
        let chain = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        let mut tracker = BlockRequestTracker::default();
        let unknown = [9u8; 32];
        let requested_at = now();
        assert!(matches!(
            handle_peer_message(
                &chain,
                Message::inv(vec![InventoryVector {
                    inv_type: InvType::Block,
                    hash: unknown,
                }]),
                &mut tracker,
                requested_at,
            ),
            Ok(RelayAction::Reply(_))
        ));
        assert_eq!(
            handle_peer_message(
                &chain,
                Message::inv(vec![InventoryVector {
                    inv_type: InvType::Block,
                    hash: unknown,
                }]),
                &mut tracker,
                requested_at,
            ),
            Ok(RelayAction::None)
        );
    }

    #[test]
    fn expired_requests_can_be_retried() {
        let mut tracker = BlockRequestTracker::new(8, Duration::from_secs(60));
        let hash = [1u8; 32];
        let requested_at = now();
        assert!(tracker.mark_requested(&hash, requested_at));
        assert!(!tracker.mark_requested(&hash, requested_at));

        let retry_at = requested_at + Duration::from_secs(61);
        assert_eq!(tracker.expire_before(retry_at), 0);
        assert!(tracker.mark_requested(&hash, retry_at));
    }

    #[test]
    fn tracker_respects_capacity() {
        let mut tracker = BlockRequestTracker::new(2, Duration::from_secs(60));
        let first = [1u8; 32];
        let second = [2u8; 32];
        let third = [3u8; 32];
        let requested_at = now();
        assert!(tracker.mark_requested(&first, requested_at));
        assert!(tracker.mark_requested(&second, requested_at));
        assert!(!tracker.mark_requested(&third, requested_at));
        assert_eq!(tracker.pending_count(), 2);
    }

    #[test]
    fn getdata_serves_known_block() {
        let genesis = genesis_block();
        let chain = ChainHandle::new_genesis(genesis.clone(), NETWORK_TIME).expect("genesis");
        let hash = genesis.hash();
        let action = handle_peer_message(
            &chain,
            Message::getdata(vec![InventoryVector {
                inv_type: InvType::Block,
                hash,
            }]),
            &mut BlockRequestTracker::default(),
            now(),
        )
        .expect("handle getdata");
        match action {
            RelayAction::Reply(messages) => {
                assert_eq!(messages.len(), 1);
                assert_eq!(messages[0].command, "block");
            }
            other => panic!("expected reply, got {other:?}"),
        }
    }

    #[test]
    fn connected_block_triggers_inv_announcement() {
        let genesis = genesis_block();
        let chain = ChainHandle::new_genesis(genesis.clone(), NETWORK_TIME).expect("genesis");
        let child = child_block(&genesis, 1, 600);
        let hash = child.hash();
        let action = handle_peer_message(
            &chain,
            Message::block(child),
            &mut BlockRequestTracker::default(),
            now(),
        )
        .expect("connect");
        assert_eq!(
            action,
            RelayAction::Announce(vec![InventoryVector {
                inv_type: InvType::Block,
                hash,
            }])
        );
    }

    #[test]
    fn duplicate_known_block_is_ignored() {
        let genesis = genesis_block();
        let chain = ChainHandle::new_genesis(genesis.clone(), NETWORK_TIME).expect("genesis");
        let action = handle_peer_message(
            &chain,
            Message::block(genesis),
            &mut BlockRequestTracker::default(),
            now(),
        )
        .expect("duplicate");
        assert_eq!(action, RelayAction::None);
    }

    #[test]
    fn orphaned_block_requests_missing_parent() {
        let chain = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        let missing_parent = [0xab; 32];
        let orphan = orphan_block(missing_parent, 1);
        let action = handle_peer_message(
            &chain,
            Message::block(orphan),
            &mut BlockRequestTracker::default(),
            now(),
        )
        .expect("orphan");
        assert_eq!(
            action,
            RelayAction::Reply(vec![Message::getdata(vec![InventoryVector {
                inv_type: InvType::Block,
                hash: missing_parent,
            }])])
        );
    }

    #[test]
    fn orphaned_parent_request_is_not_repeated() {
        let chain = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        let missing_parent = [0xab; 32];
        let mut tracker = BlockRequestTracker::default();
        let orphan = orphan_block(missing_parent, 1);
        let requested_at = now();
        assert!(matches!(
            handle_peer_message(
                &chain,
                Message::block(orphan.clone()),
                &mut tracker,
                requested_at,
            ),
            Ok(RelayAction::Reply(_))
        ));
        assert_eq!(
            handle_peer_message(&chain, Message::block(orphan), &mut tracker, requested_at),
            Ok(RelayAction::None)
        );
    }
}
