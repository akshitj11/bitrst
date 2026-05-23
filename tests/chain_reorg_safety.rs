//! Reorg rollback and UTXO consistency integration tests.

mod common;

use bitrst_core::{Chain, ChainError, ConnectResult, UtxoError};

use common::{
    assert_chain_invariants, assert_unchanged_on_connect_err, build_block_spending_fake_utxo,
    build_linear_chain_blocks, genesis_block, mine_block_on, setup_chain_of_length, snapshot,
    NETWORK_TIME,
};

#[test]
fn reorg_rolls_back_on_invalid_fork_block() {
    let mut chain = setup_chain_of_length(3);
    let before = snapshot(&chain);

    let genesis = chain.active_block_at(0).expect("genesis").clone();
    let b1 = mine_block_on(&genesis, NETWORK_TIME + 9000, 1);
    let b2 = mine_block_on(&b1, NETWORK_TIME + 9100, 2);
    let b3_bad = build_block_spending_fake_utxo(&b2);

    chain
        .set_network_time(NETWORK_TIME + 9000)
        .expect("network time");
    chain.connect_block(b1).expect("store fork branch");
    chain
        .set_network_time(NETWORK_TIME + 9100)
        .expect("network time");
    chain.connect_block(b2).expect("store fork branch");
    chain
        .set_network_time(NETWORK_TIME + 9200)
        .expect("network time");
    let result = chain.connect_block(b3_bad);
    assert!(
        matches!(
            result,
            Err(ChainError::Utxo(UtxoError::MissingInput { .. }))
        ),
        "expected utxo rejection during reorg, got {result:?}"
    );
    assert_unchanged_on_connect_err(&chain, before, result);
    assert_chain_invariants(&chain);
}

#[test]
fn reorg_utxo_is_consistent_after_success() {
    let genesis = genesis_block();
    let mut chain_a = Chain::new_genesis(genesis.clone(), NETWORK_TIME).expect("genesis");
    let mut chain_b = Chain::new_genesis(genesis.clone(), NETWORK_TIME).expect("genesis");

    let a1 = mine_block_on(&genesis, NETWORK_TIME + 600, 1);
    let b1 = mine_block_on(&genesis, NETWORK_TIME + 700, 1);
    let b2 = mine_block_on(&b1, NETWORK_TIME + 800, 2);

    chain_a.connect_block(a1).expect("main fork a");
    chain_a.connect_block(b1.clone()).expect("side fork");
    chain_a
        .connect_block(b2.clone())
        .expect("heavier fork reorg");

    chain_b.connect_block(b1).expect("direct fork");
    chain_b.connect_block(b2).expect("direct fork tip");

    assert_eq!(chain_a.utxo(), chain_b.utxo());
    assert_eq!(chain_a.utxo().len(), chain_b.utxo().len());
    assert_chain_invariants(&chain_a);
    assert_chain_invariants(&chain_b);
}

#[test]
fn deep_reorg_restores_utxo_correctly() {
    let genesis = genesis_block();
    let mut chain = Chain::new_genesis(genesis.clone(), NETWORK_TIME).expect("genesis");

    let main_blocks = build_linear_chain_blocks(&genesis, 5);
    for block in &main_blocks {
        chain
            .set_network_time(block.header.time)
            .expect("network time");
        chain.connect_block(block.clone()).expect("main chain");
    }
    assert_eq!(chain.height(), 5);

    let mut fork_parent = genesis;
    for height in 1..=6 {
        let time = NETWORK_TIME.saturating_add(10_000 + height);
        let block = mine_block_on(&fork_parent, time, height);
        fork_parent = block.clone();
        chain.set_network_time(time).expect("network time");
        let result = chain.connect_block(block).expect("fork connect");
        if height < 6 {
            assert!(matches!(
                result,
                ConnectResult::SideChain { .. } | ConnectResult::Connected { .. }
            ));
        }
    }
    assert_eq!(chain.height(), 6);
    assert_eq!(chain.utxo().len(), 7);
    assert_chain_invariants(&chain);
}
