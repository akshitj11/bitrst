//! Integration tests for mempool admission and disk-backed block storage.

mod common;

use bitrst_core::{
    BlockStore, FileBlockStore, Mempool, MempoolError, MempoolLimits, Target, Transaction, TxInput,
    TxOutput,
};
use bitrst_crypto::hash160::hash160;
use bitrst_script::{p2pkh_script_pubkey, p2pkh_script_sig};
use secp256k1::{Message, Secp256k1, SecretKey};
use tempfile::tempdir;

use common::{genesis_block, mine_block_on, setup_chain_of_length, NETWORK_TIME, TEST_BITS};

#[test]
fn file_block_store_persists_across_reopen_in_workspace() {
    let dir = tempdir().expect("tempdir");
    let genesis = genesis_block();
    let hash = genesis.hash();

    {
        let mut store = FileBlockStore::new(dir.path()).expect("open");
        store.put_block(&genesis).expect("put");
        store.commit().expect("commit");
    }

    let store = FileBlockStore::new(dir.path()).expect("reopen");
    let loaded = store.get_block(&hash).expect("get").expect("block");
    assert_eq!(loaded.hash(), hash);
}

#[test]
fn mempool_accepts_signed_spend_against_chain_utxo() {
    let mut chain = setup_chain_of_length(2);
    let parent = chain.active_block_at(1).expect("parent").clone();

    let secp = Secp256k1::new();
    let sk = SecretKey::from_slice(&[0x77; 32]).expect("secret");
    let pk = secp256k1::PublicKey::from_secret_key(&secp, &sk);
    let pubkey_bytes = pk.serialize();
    let lock_script = p2pkh_script_pubkey(&hash160(&pubkey_bytes));

    let fund = mine_block_on(&parent, NETWORK_TIME + 900, 2);
    let mut fund_block = fund;
    fund_block.transactions[0].outputs[0].script_pubkey = lock_script.clone();
    fund_block.header.merkle_root = fund_block.merkle_root().expect("merkle");
    let target = Target::from_bits(TEST_BITS).expect("bits");
    common::mine_header(&mut fund_block.header, target);
    chain.connect_block(fund_block).expect("fund");
    let funding_txid = chain.active_block_at(2).expect("fund").transactions[0].txid();

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
    let prev_scripts = vec![lock_script.clone()];
    let sighash = bitrst_core::sighash_all(&spend, 0, &prev_scripts).expect("sighash");
    let sig = secp.sign_ecdsa(&Message::from_digest(sighash), &sk);
    let mut sig_bytes = sig.serialize_der().to_vec();
    sig_bytes.push(0x01);
    spend.inputs[0].script_sig = p2pkh_script_sig(&sig_bytes, &pubkey_bytes);

    let mut pool = Mempool::new(MempoolLimits::default());
    let accepted = pool.accept_tx(spend, chain.utxo()).expect("accept");
    assert_eq!(accepted.fee, 1_0000_0000);
    assert!(pool.contains(&accepted.txid));
}

#[test]
fn mempool_rejects_missing_inputs_without_chain_mutation() {
    let chain = setup_chain_of_length(2);
    let mut pool = Mempool::new(MempoolLimits::default());
    let before_utxo = chain.utxo().len();

    let tx = Transaction {
        version: 1,
        inputs: vec![TxInput {
            previous_output: [8u8; 32],
            index: 0,
            script_sig: vec![],
            sequence: u32::MAX,
        }],
        outputs: vec![TxOutput {
            value: 1,
            script_pubkey: vec![],
        }],
        lock_time: 0,
    };

    assert!(matches!(
        pool.accept_tx(tx, chain.utxo()),
        Err(MempoolError::Utxo(_))
    ));
    assert_eq!(chain.utxo().len(), before_utxo);
}
