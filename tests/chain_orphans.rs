//! Orphan pool depth, deduplication, and eviction integration tests.

mod common;

use bitrst_core::{Chain, ChainError, ChainEvent, ConnectResult, EvictionReason};

use common::{
    build_linear_chain_blocks, build_orphan_block, fill_orphan_pool, genesis_block,
    setup_chain_of_length, NETWORK_TIME,
};

#[test]
fn long_orphan_chain_does_not_stack_overflow() {
    let genesis = genesis_block();
    let mut chain = Chain::new_genesis(genesis.clone(), NETWORK_TIME).expect("genesis");

    let mut blocks = build_linear_chain_blocks(&genesis, 200);
    for block in blocks.iter().skip(1) {
        chain
            .set_network_time(block.header.time)
            .expect("network time for orphan");
        assert!(matches!(
            chain.connect_block(block.clone()).expect("store orphan"),
            ConnectResult::Orphaned { .. }
        ));
    }
    assert_eq!(chain.height(), 0);

    let root = blocks.remove(0);
    chain
        .set_network_time(root.header.time)
        .expect("network time for root orphan");
    chain.connect_block(root).expect("promote orphan chain");
    assert_eq!(chain.height(), 200);
    assert_eq!(chain.utxo().len(), 201);
}

#[test]
fn orphan_with_duplicate_parent_is_rejected() {
    let mut chain = setup_chain_of_length(1);
    let orphan = build_orphan_block(1usize);

    assert!(matches!(
        chain.connect_block(orphan.clone()).expect("first orphan"),
        ConnectResult::Orphaned { .. }
    ));

    let result = chain.connect_block(orphan);
    assert!(matches!(result, Err(ChainError::BlockAlreadyKnown)));
}

#[test]
fn orphan_pool_evicts_oldest_when_full() {
    let mut chain = setup_chain_of_length(1);
    let first_hash = fill_orphan_pool(&mut chain);

    let events = chain.take_events();
    let evicted = events.iter().find_map(|event| match event {
        ChainEvent::OrphanEvicted { hash, reason, .. } => {
            (*reason == EvictionReason::PoolFull).then_some(*hash)
        }
        _ => None,
    });

    assert_eq!(evicted, Some(first_hash), "oldest orphan must be evicted");
}

#[test]
fn chain_of_three_orphans_promotes_to_height_three() {
    let genesis = genesis_block();
    let mut chain = Chain::new_genesis(genesis.clone(), NETWORK_TIME).expect("genesis");

    let blocks = build_linear_chain_blocks(&genesis, 3);
    assert!(matches!(
        chain.connect_block(blocks[2].clone()).expect("orphan b3"),
        ConnectResult::Orphaned { .. }
    ));
    assert!(matches!(
        chain.connect_block(blocks[1].clone()).expect("orphan b2"),
        ConnectResult::Orphaned { .. }
    ));

    chain.connect_block(blocks[0].clone()).expect("connect b1");
    assert_eq!(chain.height(), 3);
    assert_eq!(chain.utxo().len(), 4);
}
