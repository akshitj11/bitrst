//! Difficulty adjustment and compact-bits validation integration tests.

mod common;

use bitrst_core::difficulty::{adjust_bits, difficulty_adjustment_interval};
use bitrst_core::{Chain, ChainError, Target};

use common::{
    assert_unchanged_on_connect_err, build_chain_of_length, genesis_block, mine_block_on,
    mine_header, setup_chain_of_length, snapshot, NETWORK_TIME,
};

#[test]
fn difficulty_adjusts_at_interval_boundary() {
    let interval = difficulty_adjustment_interval();
    let genesis = genesis_block();
    let mut chain = Chain::new_genesis(genesis.clone(), NETWORK_TIME).expect("genesis");
    let mut parent = genesis.clone();

    let pre_boundary_height = interval - 1;
    for height in 1..=pre_boundary_height {
        let time = NETWORK_TIME.saturating_add(height);
        parent = mine_block_on(&parent, time, height);
        chain
            .set_network_time(time)
            .expect("advance network time for drift check");
        chain.connect_block(parent.clone()).expect("extend chain");
    }

    let tip_bits = chain
        .active_block_at(chain.height())
        .expect("tip")
        .header
        .bits;
    let timespan = parent.header.time.saturating_sub(genesis.header.time);
    let expected_bits = adjust_bits(tip_bits, timespan).expect("adjust bits");
    assert_ne!(
        expected_bits, tip_bits,
        "compressed period must change bits at the adjustment boundary"
    );

    let boundary_time = parent.header.time.saturating_add(1);
    let wrong_boundary = mine_block_on(&parent, boundary_time, interval);
    let before = common::snapshot(&chain);
    let result = chain.connect_block(wrong_boundary);
    assert!(matches!(
        result,
        Err(ChainError::UnexpectedBits {
            height,
            expected,
            actual
        }) if height == interval && expected == expected_bits && actual == tip_bits
    ));
    assert_unchanged_on_connect_err(&chain, before, result);
}

#[test]
fn difficulty_unchanged_between_intervals() {
    let chain = build_chain_of_length(
        Chain::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis"),
        6,
    );
    let expected_bits = chain.active_block_at(1).expect("block 1").header.bits;

    for height in 2..=chain.height() {
        assert_eq!(
            chain.active_block_at(height).expect("block").header.bits,
            expected_bits,
            "bits must not change between adjustment intervals"
        );
    }
}

#[test]
fn block_with_wrong_bits_is_rejected() {
    let mut chain = setup_chain_of_length(1);
    let genesis = chain.active_block_at(0).expect("genesis").clone();
    let before = snapshot(&chain);

    let mut block = mine_block_on(&genesis, NETWORK_TIME + 600, 1);
    block.header.bits = 0x1f00_aaaa;
    let target = Target::from_bits(block.header.bits).expect("test bits");
    mine_header(&mut block.header, target);

    let result = chain.connect_block(block);
    assert!(matches!(result, Err(ChainError::UnexpectedBits { .. })));
    assert_unchanged_on_connect_err(&chain, before, result);
}
