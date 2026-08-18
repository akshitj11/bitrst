//! Block relay, inventory handling, and chain integration.

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

/// Handles an inbound P2P message against local chain state.
///
/// # Errors
///
/// Returns [`ChainError`] when block connection fails unexpectedly.
pub fn handle_peer_message(
    chain: &ChainHandle,
    message: Message,
) -> Result<RelayAction, ChainError> {
    match message.payload {
        MessagePayload::Block(block) => handle_block(chain, block),
        MessagePayload::Inv(items) => handle_inv(chain, items),
        MessagePayload::GetData(items) => Ok(handle_getdata(chain, items)),
        MessagePayload::Tx(_) => Ok(RelayAction::None),
        MessagePayload::Version(_) | MessagePayload::Verack => Ok(RelayAction::None),
    }
}

fn handle_block(chain: &ChainHandle, block: Block) -> Result<RelayAction, ChainError> {
    let hash = block.hash();
    let result = chain.connect_block(block)?;
    match result {
        ConnectResult::Connected { .. } | ConnectResult::Reorganized { .. } => {
            Ok(RelayAction::Announce(vec![InventoryVector {
                inv_type: InvType::Block,
                hash,
            }]))
        }
        ConnectResult::Orphaned { .. } | ConnectResult::SideChain { .. } => Ok(RelayAction::None),
    }
}

fn handle_inv(chain: &ChainHandle, items: Vec<InventoryVector>) -> Result<RelayAction, ChainError> {
    let mut requests = Vec::new();
    for item in items {
        if item.inv_type != InvType::Block {
            continue;
        }
        if chain.has_block(&item.hash)? {
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
    use super::handle_peer_message;
    use crate::message::{InvType, InventoryVector, Message};
    use crate::RelayAction;
    use bitrst_core::{Block, BlockHeader, ChainHandle, Target};

    const NETWORK_TIME: u32 = 1_231_006_505;
    const TEST_BITS: u32 = 0x1f00_ffff;

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
        let target = Target::from_bits(TEST_BITS).expect("bits");
        while !target.meets(&block.header.hash()) {
            block.header.nonce = block.header.nonce.wrapping_add(1);
        }
        block
    }

    fn child_block(parent: &Block, nonce: u32) -> Block {
        let header = BlockHeader {
            version: 1,
            prev_blockhash: parent.hash(),
            merkle_root: [0u8; 32],
            time: NETWORK_TIME + 600,
            bits: TEST_BITS,
            nonce,
        };
        let mut block = Block::coinbase(header, 1, 50_0000_0000);
        block.header.merkle_root = block.merkle_root().expect("merkle");
        let target = Target::from_bits(TEST_BITS).expect("bits");
        while !target.meets(&block.header.hash()) {
            block.header.nonce = block.header.nonce.wrapping_add(1);
        }
        block
    }

    #[test]
    fn inv_requests_unknown_blocks_only() {
        let chain = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        let unknown = [9u8; 32];
        let action = handle_peer_message(
            &chain,
            Message::inv(vec![InventoryVector {
                inv_type: InvType::Block,
                hash: unknown,
            }]),
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
        let child = child_block(&genesis, 1);
        let hash = child.hash();
        let action = handle_peer_message(&chain, Message::block(child)).expect("connect");
        assert_eq!(
            action,
            RelayAction::Announce(vec![InventoryVector {
                inv_type: InvType::Block,
                hash,
            }])
        );
    }
}
