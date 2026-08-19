//! Shared helpers for unit and integration tests.

use bitrst_core::{Block, BlockHeader, ChainHandle, Target, Transaction, TxInput, TxOutput};
use bitrst_crypto::hash160::hash160;
use bitrst_script::{p2pkh_script_pubkey, p2pkh_script_sig};
use secp256k1::{Message, Secp256k1, SecretKey};

/// Network time used by mined test blocks.
pub const NETWORK_TIME: u32 = 1_231_006_505;

/// Easy difficulty bits for fast test mining.
pub const TEST_BITS: u32 = 0x1f00_ffff;

/// Returns a mined genesis block suitable for networking tests.
pub fn genesis_block() -> Block {
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

/// Mines a valid child block extending `parent`.
pub fn child_block(parent: &Block, height: u32, time_offset: u32) -> Block {
    let header = BlockHeader {
        version: 1,
        prev_blockhash: parent.hash(),
        merkle_root: [0u8; 32],
        time: NETWORK_TIME + time_offset,
        bits: TEST_BITS,
        nonce: 0,
    };
    let mut block = Block::coinbase(header, height, 50_0000_0000);
    block.header.merkle_root = block.merkle_root().expect("merkle");
    let target = Target::from_bits(TEST_BITS).expect("bits");
    while !target.meets(&block.header.hash()) {
        block.header.nonce = block.header.nonce.wrapping_add(1);
    }
    block
}

/// Builds a block whose parent hash is unknown to a genesis-only chain.
pub fn orphan_block(unknown_parent: [u8; 32], height: u32) -> Block {
    let header = BlockHeader {
        version: 1,
        prev_blockhash: unknown_parent,
        merkle_root: [0u8; 32],
        time: NETWORK_TIME + 600,
        bits: TEST_BITS,
        nonce: 0,
    };
    let mut block = Block::coinbase(header, height, 50_0000_0000);
    block.header.merkle_root = block.merkle_root().expect("merkle");
    let target = Target::from_bits(TEST_BITS).expect("bits");
    while !target.meets(&block.header.hash()) {
        block.header.nonce = block.header.nonce.wrapping_add(1);
    }
    block
}

/// Funds the chain with a P2PKH output and returns a signed spend transaction.
pub fn funded_p2pkh_spend(chain: &ChainHandle) -> (Transaction, [u8; 32]) {
    let secp = Secp256k1::new();
    let sk = SecretKey::from_slice(&[0x44; 32]).expect("secret");
    let pk = secp256k1::PublicKey::from_secret_key(&secp, &sk);
    let pubkey_bytes = pk.serialize();
    let lock_script = p2pkh_script_pubkey(&hash160(&pubkey_bytes));

    let mut fund_block = child_block(&genesis_block(), 1, 100);
    fund_block.transactions[0].outputs[0].script_pubkey = lock_script.clone();
    fund_block.header.merkle_root = fund_block.merkle_root().expect("merkle");
    let target = Target::from_bits(TEST_BITS).expect("bits");
    while !target.meets(&fund_block.header.hash()) {
        fund_block.header.nonce = fund_block.header.nonce.wrapping_add(1);
    }
    chain.connect_block(fund_block).expect("fund");
    let funding_txid = chain
        .get_block(&chain.tip_hash().expect("tip"))
        .expect("get")
        .expect("block")
        .transactions[0]
        .txid();

    let mut spend = Transaction {
        version: 1,
        inputs: vec![TxInput {
            previous_output: funding_txid,
            index: 0,
            script_sig: Vec::new(),
            sequence: u32::MAX,
        }],
        outputs: vec![TxOutput {
            value: 49_0000_0000,
            script_pubkey: Vec::new(),
        }],
        lock_time: 0,
    };
    let prev_scripts = vec![lock_script];
    let sighash = bitrst_core::sighash_all(&spend, 0, &prev_scripts).expect("sighash");
    let sig = secp.sign_ecdsa(&Message::from_digest(sighash), &sk);
    let mut sig_bytes = sig.serialize_der().to_vec();
    sig_bytes.push(0x01);
    spend.inputs[0].script_sig = p2pkh_script_sig(&sig_bytes, &pubkey_bytes);

    let spend_txid = spend.txid();
    (spend, spend_txid)
}
