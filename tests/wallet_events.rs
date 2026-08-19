//! Wallet event application integration tests.

mod common;

use bitrst_core::{Block, Chain, ChainEvent, ConnectResult};
use bitrst_script::p2pkh_script_pubkey;
use bitrst_wallet::{Address, Network, PrivateKey, Wallet};

use common::{genesis_block, mine_block_on, mine_header_for_bits, NETWORK_TIME, REWARD, TEST_BITS};

#[test]
fn wallet_ignores_side_chain_blocks() {
    let key = fixed_key();
    let address = Address::p2pkh(key.pubkey_hash(), Network::Mainnet);
    let mut wallet = Wallet::new();
    wallet.watch_address(address.clone());
    let mut chain = Chain::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
    let main_block = mine_block_on(
        chain.active_block_at(0).expect("genesis"),
        NETWORK_TIME + 600,
        1,
    );
    chain.connect_block(main_block).expect("main block");
    chain.take_events().expect("events");

    let side_block = coinbase_paying(
        chain.active_block_at(0).expect("genesis"),
        1,
        NETWORK_TIME + 700,
        address.pubkey_hash(),
    );
    chain
        .set_network_time(side_block.header.time)
        .expect("network time");
    let result = chain.connect_block(side_block).expect("side block");
    assert!(matches!(result, ConnectResult::SideChain { .. }));
    let events = chain.take_events().expect("events");

    wallet.apply_events(&events, &chain).expect("events");

    assert_eq!(wallet.balance(), 0);
}

#[test]
fn wallet_balance_correct_after_reorg() {
    let key = fixed_key();
    let address = Address::p2pkh(key.pubkey_hash(), Network::Mainnet);
    let mut wallet = Wallet::new();
    wallet.watch_address(address.clone());
    let genesis = genesis_block();
    let mut chain = Chain::new_genesis(genesis.clone(), NETWORK_TIME).expect("genesis");
    chain.take_events().expect("events");

    let paying_a1 = coinbase_paying(&genesis, 1, NETWORK_TIME + 600, address.pubkey_hash());
    chain
        .set_network_time(paying_a1.header.time)
        .expect("network time");
    chain.connect_block(paying_a1).expect("main block");
    wallet
        .apply_events(&chain.take_events().expect("events"), &chain)
        .expect("wallet main");
    assert_eq!(wallet.balance(), REWARD);

    let b1 = mine_block_on(&genesis, NETWORK_TIME + 5000, 1);
    let b2 = mine_block_on(&b1, NETWORK_TIME + 5100, 2);
    chain
        .set_network_time(b1.header.time)
        .expect("network time");
    chain.connect_block(b1).expect("side b1");
    chain.take_events().expect("events");
    chain
        .set_network_time(b2.header.time)
        .expect("network time");
    let result = chain.connect_block(b2).expect("reorg");
    assert!(matches!(result, ConnectResult::Reorganized { .. }));
    let events = chain.take_events().expect("events");
    assert!(events
        .iter()
        .any(|event| matches!(event, ChainEvent::BlockDisconnected { .. })));

    wallet.apply_events(&events, &chain).expect("wallet reorg");

    assert_eq!(wallet.balance(), 0);
}

fn fixed_key() -> PrivateKey {
    PrivateKey::from_bytes([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 1,
    ])
    .expect("valid key")
}

fn coinbase_paying(parent: &Block, height: u32, time: u32, pubkey_hash: [u8; 20]) -> Block {
    let mut block = mine_block_on(parent, time, height);
    block.transactions[0].outputs[0].script_pubkey = p2pkh_script_pubkey(&pubkey_hash);
    block.header.merkle_root = block.merkle_root().expect("merkle root");
    block.header.nonce = 0;
    mine_header_for_bits(&mut block.header, TEST_BITS);
    block
}
