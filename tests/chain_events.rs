//! Chain event log correctness around reorgs and side chains.

mod common;

use bitrst_core::{ChainEvent, ConnectResult};

use common::{mine_block_on, setup_chain_of_length, NETWORK_TIME};

#[test]
fn events_are_correct_after_reorg() {
    let mut chain = setup_chain_of_length(3);
    let genesis = chain.active_block_at(0).expect("genesis").clone();

    let b1 = mine_block_on(&genesis, NETWORK_TIME + 5000, 1);
    let b2 = mine_block_on(&b1, NETWORK_TIME + 5100, 2);
    let b3 = mine_block_on(&b2, NETWORK_TIME + 5200, 3);

    chain.connect_block(b1).expect("side fork b1");
    chain.take_events();

    chain.connect_block(b2).expect("side fork b2");
    let result = chain.connect_block(b3).expect("reorg to fork");
    assert!(matches!(result, ConnectResult::Reorganized { .. }));

    let events = chain.take_events();
    let disconnected = events
        .iter()
        .filter(|event| matches!(event, ChainEvent::BlockDisconnected { .. }))
        .count();
    let connected = events
        .iter()
        .filter(|event| matches!(event, ChainEvent::BlockConnected { .. }))
        .count();
    let reorgs = events
        .iter()
        .filter(|event| matches!(event, ChainEvent::ChainReorg { .. }))
        .count();

    assert_eq!(disconnected, 2, "a1 and a2 disconnected");
    assert_eq!(connected, 3, "b1, b2, b3 connected during reorg");
    assert_eq!(reorgs, 1);
}

#[test]
fn no_events_emitted_for_side_chain_block() {
    let mut chain = setup_chain_of_length(2);
    chain.take_events();

    let genesis = chain.active_block_at(0).expect("genesis").clone();
    let side_time = NETWORK_TIME + 700;
    let side_block = mine_block_on(&genesis, side_time, 1);
    chain
        .set_network_time(side_time)
        .expect("network time for side-chain block");

    let result = chain.connect_block(side_block).expect("side chain block");
    assert!(matches!(result, ConnectResult::SideChain { .. }));

    let events = chain.take_events();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ChainEvent::BlockConnected { .. })),
        "side chain must not emit BlockConnected"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ChainEvent::BlockDisconnected { .. })),
        "side chain must not emit BlockDisconnected"
    );
}
