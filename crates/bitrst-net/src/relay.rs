//! Block and transaction relay, inventory handling, and chain integration.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use bitrst_core::{
    Block, ChainError, ChainEventCursor, ChainHandle, ConnectResult, MempoolHandle,
    MempoolHandleError, Transaction,
};

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

/// Errors from relay handling against shared chain and mempool state.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RelayError {
    /// Chain access or validation failed.
    #[error(transparent)]
    Chain(#[from] ChainError),

    /// Mempool access or admission failed unexpectedly.
    #[error(transparent)]
    Mempool(#[from] MempoolHandleError),
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

/// Tracks outstanding transaction `getdata` requests.
#[derive(Debug, Clone)]
pub struct TxRequestTracker {
    pending: HashMap<[u8; 32], PendingRequest>,
    max_pending: usize,
    request_ttl: Duration,
}

impl Default for TxRequestTracker {
    fn default() -> Self {
        Self::new(MAX_PENDING_BLOCK_REQUESTS, BLOCK_REQUEST_TTL)
    }
}

impl TxRequestTracker {
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

    /// Records `txid` as requested. Returns `false` when already pending or at capacity.
    #[must_use]
    pub fn mark_requested(&mut self, txid: &[u8; 32], now: Instant) -> bool {
        self.expire_before(now);
        if self.pending.contains_key(txid) {
            return false;
        }
        if self.pending.len() >= self.max_pending {
            return false;
        }
        self.pending
            .insert(*txid, PendingRequest { requested_at: now });
        true
    }

    /// Clears a completed or abandoned request.
    pub fn clear(&mut self, txid: &[u8; 32]) {
        self.pending.remove(txid);
    }
}

/// Mutable relay state for one peer connection.
#[derive(Debug, Default)]
pub struct PeerRelayState {
    /// Outstanding block requests.
    pub block_requests: BlockRequestTracker,
    /// Outstanding transaction requests.
    pub tx_requests: TxRequestTracker,
    /// Non-destructive chain event read position for mempool synchronization.
    pub chain_events: ChainEventCursor,
}

impl PeerRelayState {
    /// Creates relay state with a chain event cursor positioned at the current log end.
    pub fn with_event_cursor(cursor: ChainEventCursor) -> Self {
        Self {
            block_requests: BlockRequestTracker::default(),
            tx_requests: TxRequestTracker::default(),
            chain_events: cursor,
        }
    }
}

/// Handles an inbound P2P message against local chain and mempool state.
///
/// # Errors
///
/// Returns [`RelayError`] when block connection or mempool synchronization fails unexpectedly.
#[allow(clippy::too_many_arguments)]
pub fn handle_peer_message(
    chain: &ChainHandle,
    mempool: &MempoolHandle,
    relay: &mut PeerRelayState,
    message: Message,
    now: Instant,
) -> Result<RelayAction, RelayError> {
    match message.payload {
        MessagePayload::Block(block) => handle_block(chain, mempool, relay, block, now),
        MessagePayload::Inv(items) => handle_inv(chain, mempool, relay, items, now),
        MessagePayload::GetData(items) => Ok(handle_getdata(chain, mempool, items)),
        MessagePayload::Tx(tx) => handle_tx(chain, mempool, relay, tx),
        MessagePayload::Version(_) | MessagePayload::Verack => Ok(RelayAction::None),
    }
}

fn sync_mempool(
    chain: &ChainHandle,
    mempool: &MempoolHandle,
    relay: &mut PeerRelayState,
) -> Result<(), RelayError> {
    let events = chain.collect_events(&mut relay.chain_events)?;
    if events.is_empty() {
        return Ok(());
    }
    match chain.with_chain(|active| mempool.apply_chain_events(&events, active)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.into()),
        Err(error) => Err(error.into()),
    }
}

fn handle_tx(
    chain: &ChainHandle,
    mempool: &MempoolHandle,
    relay: &mut PeerRelayState,
    tx: Transaction,
) -> Result<RelayAction, RelayError> {
    let txid = tx.txid();
    relay.tx_requests.clear(&txid);

    let accepted = match chain.with_chain(|active| mempool.accept_tx(tx, active.utxo())) {
        Ok(Ok(accepted)) => accepted,
        Ok(Err(MempoolHandleError::Admission(_))) => return Ok(RelayAction::None),
        Ok(Err(error)) => return Err(error.into()),
        Err(error) => return Err(error.into()),
    };

    Ok(RelayAction::Announce(vec![InventoryVector {
        inv_type: InvType::Transaction,
        hash: accepted.txid,
    }]))
}

fn handle_block(
    chain: &ChainHandle,
    mempool: &MempoolHandle,
    relay: &mut PeerRelayState,
    block: Block,
    now: Instant,
) -> Result<RelayAction, RelayError> {
    let hash = block.hash();
    let parent_hash = block.header.prev_blockhash;
    match chain.connect_block(block) {
        Ok(result) => match result {
            ConnectResult::Connected { .. } | ConnectResult::Reorganized { .. } => {
                relay.block_requests.clear(&hash);
                sync_mempool(chain, mempool, relay)?;
                Ok(RelayAction::Announce(vec![InventoryVector {
                    inv_type: InvType::Block,
                    hash,
                }]))
            }
            ConnectResult::Orphaned { .. } => {
                request_missing_block(chain, relay, parent_hash, now).map_err(RelayError::from)
            }
            ConnectResult::SideChain { .. } => Ok(RelayAction::None),
        },
        Err(ChainError::BlockAlreadyKnown) => Ok(RelayAction::None),
        Err(error) => {
            relay.block_requests.clear(&hash);
            relay.block_requests.clear(&parent_hash);
            Err(error.into())
        }
    }
}

fn request_missing_block(
    chain: &ChainHandle,
    relay: &mut PeerRelayState,
    hash: [u8; 32],
    now: Instant,
) -> Result<RelayAction, ChainError> {
    if chain.has_block(&hash)? || !relay.block_requests.mark_requested(&hash, now) {
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
    mempool: &MempoolHandle,
    relay: &mut PeerRelayState,
    items: Vec<InventoryVector>,
    now: Instant,
) -> Result<RelayAction, RelayError> {
    let mut requests = Vec::new();
    for item in items {
        match item.inv_type {
            InvType::Block => {
                if chain.has_block(&item.hash)?
                    || !relay.block_requests.mark_requested(&item.hash, now)
                {
                    continue;
                }
                requests.push(item);
            }
            InvType::Transaction => {
                if mempool.contains(&item.hash)?
                    || !relay.tx_requests.mark_requested(&item.hash, now)
                {
                    continue;
                }
                requests.push(item);
            }
            InvType::FilteredBlock => {}
        }
    }
    if requests.is_empty() {
        Ok(RelayAction::None)
    } else {
        Ok(RelayAction::Reply(vec![Message::getdata(requests)]))
    }
}

fn handle_getdata(
    chain: &ChainHandle,
    mempool: &MempoolHandle,
    items: Vec<InventoryVector>,
) -> RelayAction {
    let mut replies = Vec::new();
    for item in items {
        match item.inv_type {
            InvType::Block => {
                if let Ok(Some(block)) = chain.get_block(&item.hash) {
                    replies.push(Message::block(block));
                }
            }
            InvType::Transaction => {
                if let Ok(Some(tx)) = mempool.get_transaction(&item.hash) {
                    replies.push(Message::tx(tx));
                }
            }
            InvType::FilteredBlock => {}
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

    use super::{handle_peer_message, BlockRequestTracker, PeerRelayState, TxRequestTracker};
    use crate::message::{InvType, InventoryVector, Message};
    use crate::testutil::{
        child_block, funded_p2pkh_spend, genesis_block, orphan_block, NETWORK_TIME,
    };
    use crate::RelayAction;
    use bitrst_core::{ChainHandle, MempoolHandle};

    fn now() -> std::time::Instant {
        std::time::Instant::now()
    }

    fn relay_state(chain: &ChainHandle) -> PeerRelayState {
        let mut relay = PeerRelayState::default();
        relay.chain_events = chain.event_cursor().expect("cursor");
        relay
    }

    #[test]
    fn inv_requests_unknown_blocks_only() {
        let chain = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        let mempool = MempoolHandle::new();
        let mut relay = relay_state(&chain);
        let unknown = [9u8; 32];
        let action = handle_peer_message(
            &chain,
            &mempool,
            &mut relay,
            Message::inv(vec![InventoryVector {
                inv_type: InvType::Block,
                hash: unknown,
            }]),
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
    fn inv_requests_unknown_transactions_only() {
        let chain = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        let mempool = MempoolHandle::new();
        let mut relay = relay_state(&chain);
        let unknown = [8u8; 32];
        let action = handle_peer_message(
            &chain,
            &mempool,
            &mut relay,
            Message::inv(vec![InventoryVector {
                inv_type: InvType::Transaction,
                hash: unknown,
            }]),
            now(),
        )
        .expect("handle inv");
        assert_eq!(
            action,
            RelayAction::Reply(vec![Message::getdata(vec![InventoryVector {
                inv_type: InvType::Transaction,
                hash: unknown,
            }])])
        );
    }

    #[test]
    fn inv_suppresses_duplicate_requests() {
        let chain = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        let mempool = MempoolHandle::new();
        let mut relay = relay_state(&chain);
        let unknown = [9u8; 32];
        let requested_at = now();
        assert!(matches!(
            handle_peer_message(
                &chain,
                &mempool,
                &mut relay,
                Message::inv(vec![InventoryVector {
                    inv_type: InvType::Block,
                    hash: unknown,
                }]),
                requested_at,
            ),
            Ok(RelayAction::Reply(_))
        ));
        assert_eq!(
            handle_peer_message(
                &chain,
                &mempool,
                &mut relay,
                Message::inv(vec![InventoryVector {
                    inv_type: InvType::Block,
                    hash: unknown,
                }]),
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
    fn tx_tracker_respects_capacity() {
        let mut tracker = TxRequestTracker::new(1, Duration::from_secs(60));
        let first = [1u8; 32];
        let second = [2u8; 32];
        let requested_at = now();
        assert!(tracker.mark_requested(&first, requested_at));
        assert!(!tracker.mark_requested(&second, requested_at));
    }

    #[test]
    fn getdata_serves_known_block() {
        let genesis = genesis_block();
        let chain = ChainHandle::new_genesis(genesis.clone(), NETWORK_TIME).expect("genesis");
        let mempool = MempoolHandle::new();
        let mut relay = relay_state(&chain);
        let hash = genesis.hash();
        let action = handle_peer_message(
            &chain,
            &mempool,
            &mut relay,
            Message::getdata(vec![InventoryVector {
                inv_type: InvType::Block,
                hash,
            }]),
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
    fn getdata_serves_mempool_transaction() {
        let chain = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        let mempool = MempoolHandle::new();
        let mut relay = relay_state(&chain);
        let (spend, _) = funded_p2pkh_spend(&chain);
        let txid = spend.txid();
        chain
            .with_chain(|active| mempool.accept_tx(spend, active.utxo()))
            .expect("chain")
            .expect("accept");

        let action = handle_peer_message(
            &chain,
            &mempool,
            &mut relay,
            Message::getdata(vec![InventoryVector {
                inv_type: InvType::Transaction,
                hash: txid,
            }]),
            now(),
        )
        .expect("handle getdata");
        match action {
            RelayAction::Reply(messages) => {
                assert_eq!(messages.len(), 1);
                assert_eq!(messages[0].command, "tx");
            }
            other => panic!("expected reply, got {other:?}"),
        }
    }

    #[test]
    fn accepted_tx_announces_inventory() {
        let chain = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        let mempool = MempoolHandle::new();
        let mut relay = relay_state(&chain);
        let (spend, _) = funded_p2pkh_spend(&chain);
        let txid = spend.txid();
        let action = handle_peer_message(&chain, &mempool, &mut relay, Message::tx(spend), now())
            .expect("handle tx");
        assert_eq!(
            action,
            RelayAction::Announce(vec![InventoryVector {
                inv_type: InvType::Transaction,
                hash: txid,
            }])
        );
    }

    #[test]
    fn invalid_tx_is_ignored_without_disconnect() {
        let chain = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        let mempool = MempoolHandle::new();
        let mut relay = relay_state(&chain);
        let (mut spend, _) = funded_p2pkh_spend(&chain);
        spend.inputs[0].script_sig = vec![0x01];
        let action = handle_peer_message(&chain, &mempool, &mut relay, Message::tx(spend), now())
            .expect("handle tx");
        assert_eq!(action, RelayAction::None);
    }

    #[test]
    fn connected_block_triggers_inv_announcement() {
        let genesis = genesis_block();
        let chain = ChainHandle::new_genesis(genesis.clone(), NETWORK_TIME).expect("genesis");
        let mempool = MempoolHandle::new();
        let mut relay = relay_state(&chain);
        let child = child_block(&genesis, 1, 600);
        let hash = child.hash();
        let action =
            handle_peer_message(&chain, &mempool, &mut relay, Message::block(child), now())
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
        let mempool = MempoolHandle::new();
        let mut relay = relay_state(&chain);
        let action =
            handle_peer_message(&chain, &mempool, &mut relay, Message::block(genesis), now())
                .expect("duplicate");
        assert_eq!(action, RelayAction::None);
    }

    #[test]
    fn orphaned_block_requests_missing_parent() {
        let chain = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        let mempool = MempoolHandle::new();
        let mut relay = relay_state(&chain);
        let missing_parent = [0xab; 32];
        let orphan = orphan_block(missing_parent, 1);
        let action =
            handle_peer_message(&chain, &mempool, &mut relay, Message::block(orphan), now())
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
        let mempool = MempoolHandle::new();
        let mut relay = relay_state(&chain);
        let missing_parent = [0xab; 32];
        let orphan = orphan_block(missing_parent, 1);
        let requested_at = now();
        assert!(matches!(
            handle_peer_message(
                &chain,
                &mempool,
                &mut relay,
                Message::block(orphan.clone()),
                requested_at,
            ),
            Ok(RelayAction::Reply(_))
        ));
        assert_eq!(
            handle_peer_message(
                &chain,
                &mempool,
                &mut relay,
                Message::block(orphan),
                requested_at
            ),
            Ok(RelayAction::None)
        );
    }
}
