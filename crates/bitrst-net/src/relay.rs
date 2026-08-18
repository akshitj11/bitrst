//! Block relay, inventory handling, and chain integration.

use std::collections::HashSet;

use bitrst_core::{Block, ChainError, ChainHandle, ConnectResult};

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

/// Tracks outstanding block `getdata` requests to suppress loops and duplicates.
#[derive(Debug, Default, Clone)]
pub struct BlockRequestTracker {
    pending: HashSet<[u8; 32]>,
}

impl BlockRequestTracker {
    /// Records `hash` as requested. Returns `false` when already pending.
    #[must_use]
    pub fn mark_requested(&mut self, hash: &[u8; 32]) -> bool {
        self.pending.insert(*hash)
    }

    /// Clears a completed or abandoned request.
    pub fn clear(&mut self, hash: &[u8; 32]) {
        self.pending.remove(hash);
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
) -> Result<RelayAction, ChainError> {
    match message.payload {
        MessagePayload::Block(block) => handle_block(chain, block, tracker),
        MessagePayload::Inv(items) => handle_inv(chain, items, tracker),
        MessagePayload::GetData(items) => Ok(handle_getdata(chain, items)),
        MessagePayload::Tx(_) => Ok(RelayAction::None),
        MessagePayload::Version(_) | MessagePayload::Verack => Ok(RelayAction::None),
    }
}

fn handle_block(
    chain: &ChainHandle,
    block: Block,
    tracker: &mut BlockRequestTracker,
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
            ConnectResult::Orphaned { .. } => request_missing_block(chain, parent_hash, tracker),
            ConnectResult::SideChain { .. } => Ok(RelayAction::None),
        },
        Err(ChainError::BlockAlreadyKnown) => Ok(RelayAction::None),
        Err(error) => Err(error),
    }
}

fn request_missing_block(
    chain: &ChainHandle,
    hash: [u8; 32],
    tracker: &mut BlockRequestTracker,
) -> Result<RelayAction, ChainError> {
    if chain.has_block(&hash)? || !tracker.mark_requested(&hash) {
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
) -> Result<RelayAction, ChainError> {
    let mut requests = Vec::new();
    for item in items {
        if item.inv_type != InvType::Block {
            continue;
        }
        if chain.has_block(&item.hash)? || !tracker.mark_requested(&item.hash) {
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
    use super::{handle_peer_message, BlockRequestTracker};
    use crate::message::{InvType, InventoryVector, Message};
    use crate::testutil::{child_block, genesis_block, orphan_block, NETWORK_TIME};
    use crate::RelayAction;
    use bitrst_core::ChainHandle;

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
        assert!(matches!(
            handle_peer_message(
                &chain,
                Message::inv(vec![InventoryVector {
                    inv_type: InvType::Block,
                    hash: unknown,
                }]),
                &mut tracker,
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
            ),
            Ok(RelayAction::None)
        );
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
        assert!(matches!(
            handle_peer_message(&chain, Message::block(orphan.clone()), &mut tracker),
            Ok(RelayAction::Reply(_))
        ));
        assert_eq!(
            handle_peer_message(&chain, Message::block(orphan), &mut tracker),
            Ok(RelayAction::None)
        );
    }
}
