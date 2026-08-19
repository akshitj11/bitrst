//! Mainnet genesis block replay from a checked-in raw wire fixture.
//!
//! Fixture source: Bitcoin mainnet block 0 (hash
//! `000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f`).
//! See <https://en.bitcoin.it/wiki/Genesis_block>.

use bitrst_core::{block_work, Block, Chain, OutPoint};

const MAINNET_GENESIS_TIME: u32 = 1_231_006_505;
const MAINNET_GENESIS_HASH: &str =
    "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f";
const MAINNET_GENESIS_BITS: u32 = 0x1d00_ffff;
const MAINNET_GENESIS_REWARD: u64 = 50_0000_0000;

/// Genesis per-block work from Bitcoin Core `GetBlockProof`.
/// Reference: <https://github.com/bitcoin/bitcoin/blob/master/src/chain.cpp>
const MAINNET_GENESIS_WORK: [u8; 32] = [
    0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

fn load_mainnet_genesis_fixture() -> Vec<u8> {
    let hex = include_str!("fixtures/mainnet_genesis_block.hex");
    hex::decode(hex.trim()).expect("fixture hex should decode")
}

fn to_bitcoin_hex(bytes: [u8; 32]) -> String {
    let mut reversed = bytes;
    reversed.reverse();
    hex::encode(reversed)
}

#[test]
fn mainnet_genesis_fixture_deserializes_and_roundtrips() {
    let raw = load_mainnet_genesis_fixture();
    let block = Block::deserialize(&raw).expect("mainnet genesis should deserialize");
    let reserialized = block.serialize();
    assert_eq!(
        reserialized, raw,
        "exact wire reserialization must match fixture"
    );

    assert_eq!(
        to_bitcoin_hex(block.hash()),
        MAINNET_GENESIS_HASH,
        "block hash must match mainnet genesis"
    );
    assert!(
        block.header_merkle_root_matches(),
        "header merkle root must commit to the coinbase transaction"
    );
}

#[test]
fn mainnet_genesis_connects_as_chain_genesis() {
    let raw = load_mainnet_genesis_fixture();
    let block = Block::deserialize(&raw).expect("mainnet genesis should deserialize");
    let coinbase = block
        .transactions
        .first()
        .expect("genesis block must include coinbase");
    let coinbase_output = coinbase
        .outputs
        .first()
        .expect("genesis coinbase must fund one output");
    let coinbase_txid = coinbase.txid();
    let coinbase_script = coinbase_output.script_pubkey.clone();

    let chain = Chain::new_genesis(block, MAINNET_GENESIS_TIME).expect("genesis should connect");

    assert_eq!(chain.height(), 0);
    assert_eq!(chain.active_block_count(), 1);
    assert_eq!(to_bitcoin_hex(chain.tip_hash()), MAINNET_GENESIS_HASH);
    assert_eq!(chain.total_work().0, MAINNET_GENESIS_WORK);
    assert_eq!(
        block_work(MAINNET_GENESIS_BITS)
            .expect("genesis bits should decode to work")
            .0,
        MAINNET_GENESIS_WORK
    );

    let coinbase_outpoint = OutPoint {
        txid: coinbase_txid,
        index: 0,
    };
    assert_eq!(chain.utxo().len(), 1);
    assert!(chain.utxo().contains(&coinbase_outpoint));
    assert_eq!(
        chain.utxo().get_value(&coinbase_outpoint),
        Some(MAINNET_GENESIS_REWARD)
    );
    let utxo_entry = chain
        .utxo()
        .get(&coinbase_outpoint)
        .expect("genesis coinbase output should be spendable");
    assert_eq!(utxo_entry.value, MAINNET_GENESIS_REWARD);
    assert_eq!(utxo_entry.script_pubkey, coinbase_script);
}
