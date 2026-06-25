//! Wallet signing integration test.

mod common;

use bitrst_core::{sighash_all, Block, BlockHeader, Chain, Transaction, TxInput, TxOutput};
use bitrst_script::{p2pkh_script_pubkey, verify_script};
use bitrst_wallet::{sign_p2pkh_input, Address, Network, PrivateKey, Wallet};

use common::{genesis_block, mine_header_for_bits, NETWORK_TIME, REWARD, TEST_BITS};

#[test]
fn wallet_signs_and_spends_received_coinbase_on_local_chain() {
    let key = fixed_key();
    let address = Address::p2pkh(key.pubkey_hash(), Network::Mainnet);
    let mut wallet = Wallet::new();
    wallet.watch_address(address.clone());
    let mut chain = Chain::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
    chain.take_events();

    let funding_block = coinbase_paying(&chain, 1, address.pubkey_hash(), REWARD);
    let funding_txid = funding_block.transactions[0].txid();
    chain
        .set_network_time(funding_block.header.time)
        .expect("network time");
    chain.connect_block(funding_block).expect("funding block");
    wallet
        .apply_events(&chain.take_events(), &chain)
        .expect("wallet funding events");
    assert_eq!(wallet.balance(), REWARD);

    let prev_script = p2pkh_script_pubkey(&address.pubkey_hash());
    let prev_scripts = vec![prev_script.clone()];
    let change_value = 40_0000_0000;
    let mut spend = Transaction {
        version: 1,
        inputs: vec![TxInput {
            previous_output: funding_txid,
            index: 0,
            script_sig: Vec::new(),
            sequence: u32::MAX,
        }],
        outputs: vec![
            TxOutput {
                value: 9_0000_0000,
                script_pubkey: vec![0x51],
            },
            TxOutput {
                value: change_value,
                script_pubkey: prev_script.clone(),
            },
        ],
        lock_time: 0,
    };

    sign_p2pkh_input(&mut spend, 0, &prev_scripts, &key).expect("sign spend");
    let sighash = sighash_all(&spend, 0, &prev_scripts).expect("sighash");
    verify_script(&spend.inputs[0].script_sig, &prev_script, &sighash).expect("signed spend");

    let spend_block =
        block_with_transactions(&chain, 2, vec![Transaction::coinbase(2, REWARD), spend]);
    chain
        .set_network_time(spend_block.header.time)
        .expect("network time");
    chain.connect_block(spend_block).expect("spend block");
    wallet
        .apply_events(&chain.take_events(), &chain)
        .expect("wallet spend events");

    assert_eq!(wallet.balance(), change_value);
}

fn fixed_key() -> PrivateKey {
    PrivateKey::from_bytes([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 1,
    ])
    .expect("valid key")
}

fn coinbase_paying(chain: &Chain, height: u32, pubkey_hash: [u8; 20], value: u64) -> Block {
    let mut block = coinbase_block(chain, height, value);
    block.transactions[0].outputs[0].script_pubkey = p2pkh_script_pubkey(&pubkey_hash);
    block.header.merkle_root = block.merkle_root().expect("merkle root");
    mine_header_for_bits(&mut block.header, TEST_BITS);
    block
}

fn coinbase_block(chain: &Chain, height: u32, value: u64) -> Block {
    let header = BlockHeader {
        version: 1,
        prev_blockhash: chain.tip_hash(),
        merkle_root: [0u8; 32],
        time: NETWORK_TIME + height,
        bits: TEST_BITS,
        nonce: 0,
    };
    let mut block = Block::coinbase(header, height, value);
    block.header.merkle_root = block.merkle_root().expect("merkle root");
    mine_header_for_bits(&mut block.header, TEST_BITS);
    block
}

fn block_with_transactions(chain: &Chain, height: u32, transactions: Vec<Transaction>) -> Block {
    let header = BlockHeader {
        version: 1,
        prev_blockhash: chain.tip_hash(),
        merkle_root: [0u8; 32],
        time: NETWORK_TIME + height,
        bits: TEST_BITS,
        nonce: 0,
    };
    let mut block = Block::new(header, transactions);
    block.header.merkle_root = block.merkle_root().expect("merkle root");
    mine_header_for_bits(&mut block.header, TEST_BITS);
    block
}
