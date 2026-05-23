//! Timestamp, UTXO, block limit, and chain-work integration tests.

mod common;

use bitrst_core::{ChainError, UtxoError};

use common::{
    assert_msb_first_ordering, assert_unchanged_on_connect_err, build_block_spending_fake_utxo,
    build_block_with_coinbase_at_position, build_block_with_double_spend,
    build_block_without_coinbase, build_oversized_block, mine_block_on, setup_chain_of_length,
    snapshot, NETWORK_TIME,
};

#[test]
fn block_below_median_past_time_is_rejected() {
    let mut chain = setup_chain_of_length(1);
    let genesis = chain.active_block_at(0).expect("genesis").clone();

    let mut parent = genesis.clone();
    let mut blocks = Vec::new();
    for (index, offset) in [100u32, 200, 300, 400, 500].into_iter().enumerate() {
        parent = mine_block_on(&parent, NETWORK_TIME + offset, index as u32 + 1);
        chain.connect_block(parent.clone()).expect("extend for mtp");
        blocks.push(parent.clone());
    }

    let before = snapshot(&chain);
    let stale = mine_block_on(blocks.last().expect("tip"), 50, 6);
    let result = chain.connect_block(stale);
    assert!(matches!(result, Err(ChainError::InvalidTimestamp { .. })));
    assert_unchanged_on_connect_err(&chain, before, result);
}

#[test]
fn block_too_far_in_future_is_rejected() {
    let mut chain = setup_chain_of_length(1);
    let genesis = chain.active_block_at(0).expect("genesis").clone();
    let before = snapshot(&chain);

    let future_time = NETWORK_TIME + 7201;
    let block = mine_block_on(&genesis, future_time, 1);
    let result = chain.connect_block(block);
    assert!(matches!(result, Err(ChainError::InvalidTimestamp { .. })));
    assert_unchanged_on_connect_err(&chain, before, result);
}

#[test]
fn double_spend_within_same_block_is_rejected() {
    let mut chain = setup_chain_of_length(2);
    let block1 = chain.active_block_at(1).expect("block 1").clone();
    let before = snapshot(&chain);

    let result = chain.connect_block(build_block_with_double_spend(&block1));
    assert!(matches!(
        result,
        Err(ChainError::Utxo(UtxoError::DuplicateSpend { .. }))
    ));
    assert_unchanged_on_connect_err(&chain, before, result);
}

#[test]
fn spending_nonexistent_utxo_is_rejected() {
    let mut chain = setup_chain_of_length(1);
    let genesis = chain.active_block_at(0).expect("genesis").clone();
    let before = snapshot(&chain);

    let result = chain.connect_block(build_block_spending_fake_utxo(&genesis));
    assert!(matches!(
        result,
        Err(ChainError::Utxo(UtxoError::MissingInput { .. }))
    ));
    assert_unchanged_on_connect_err(&chain, before, result);
}

#[test]
fn oversized_block_is_rejected_before_validation() {
    let mut chain = setup_chain_of_length(1);
    let genesis = chain.active_block_at(0).expect("genesis").clone();
    let before = snapshot(&chain);

    let result = chain.connect_block(build_oversized_block(&genesis));
    assert!(matches!(result, Err(ChainError::BlockTooLarge)));
    assert_unchanged_on_connect_err(&chain, before, result);
}

#[test]
fn block_with_no_coinbase_is_rejected() {
    let mut chain = setup_chain_of_length(1);
    let genesis = chain.active_block_at(0).expect("genesis").clone();
    let before = snapshot(&chain);

    let result = chain.connect_block(build_block_without_coinbase(&genesis));
    assert!(matches!(
        result,
        Err(ChainError::InvalidCoinbaseCount { count: 0 })
    ));
    assert_unchanged_on_connect_err(&chain, before, result);
}

#[test]
fn block_with_coinbase_not_first_is_rejected() {
    let mut chain = setup_chain_of_length(1);
    let genesis = chain.active_block_at(0).expect("genesis").clone();
    let before = snapshot(&chain);

    let result = chain.connect_block(build_block_with_coinbase_at_position(&genesis, 1));
    assert!(matches!(
        result,
        Err(ChainError::InvalidCoinbaseCount { count: 1 })
    ));
    assert_unchanged_on_connect_err(&chain, before, result);
}

#[test]
fn chain_work_comparison_is_msb_first() {
    assert_msb_first_ordering();
}
