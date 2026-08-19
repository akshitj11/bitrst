//! Integration tests for mempool reorg synchronization.

mod common;

use bitrst_core::{ConnectResult, Mempool, MempoolLimits};

use common::{mine_block_on, setup_chain_of_length, NETWORK_TIME};

#[test]
fn mempool_restores_disconnected_tx_after_heavier_reorg() {
    let (mut chain, lock_script, funding_txid) = {
        let mut chain = setup_chain_of_length(2);
        let parent = chain.active_block_at(1).expect("parent").clone();

        let secp = secp256k1::Secp256k1::new();
        let sk = secp256k1::SecretKey::from_slice(&[0x55; 32]).expect("secret");
        let pk = secp256k1::PublicKey::from_secret_key(&secp, &sk);
        let pubkey_bytes = pk.serialize();
        let lock_script =
            bitrst_script::p2pkh_script_pubkey(&bitrst_crypto::hash160::hash160(&pubkey_bytes));

        let mut fund = mine_block_on(&parent, NETWORK_TIME + 800, 2);
        fund.transactions[0].outputs[0].script_pubkey = lock_script.clone();
        fund.header.merkle_root = fund.merkle_root().expect("merkle");
        let fund_bits = fund.header.bits;
        common::mine_header_for_bits(&mut fund.header, fund_bits);
        chain.connect_block(fund).expect("fund");
        let funding_txid = chain.active_block_at(2).expect("fund").transactions[0].txid();

        (chain, lock_script, funding_txid)
    };

    let fund_parent = chain.active_block_at(2).expect("fund block").clone();
    let spend = sign_spend(funding_txid, &lock_script);
    let spend_txid = spend.txid();

    let mut pool = Mempool::new(MempoolLimits::default());
    pool.accept_tx(spend.clone(), chain.utxo()).expect("accept");

    let mut confirm = mine_block_on(&fund_parent, NETWORK_TIME + 900, 3);
    confirm.transactions.push(spend.clone());
    confirm.header.merkle_root = confirm.merkle_root().expect("merkle");
    let confirm_bits = confirm.header.bits;
    common::mine_header_for_bits(&mut confirm.header, confirm_bits);
    chain.connect_block(confirm).expect("confirm");
    pool.synchronize_to_chain(
        chain.utxo(),
        &[chain.active_block_at(3).expect("confirm").clone()],
        &[],
    );
    assert!(!pool.contains(&spend_txid));

    let alt_b4 = mine_block_on(&fund_parent, NETWORK_TIME + 1000, 3);
    let alt_b5 = mine_block_on(&alt_b4, NETWORK_TIME + 1100, 4);
    chain.connect_block(alt_b4).expect("alt b4");
    chain.take_events().expect("events");
    let result = chain.connect_block(alt_b5).expect("reorg");
    assert!(matches!(result, ConnectResult::Reorganized { .. }));

    pool.apply_chain_events(&chain.take_events().expect("events"), &chain);
    assert!(pool.contains(&spend_txid));
}

#[test]
fn resync_after_event_cursor_overrun_restores_valid_tx_not_stale() {
    use bitrst_core::chain::Chain;

    let (mut chain, lock_script, funding_txid) = {
        let genesis = common::genesis_block();
        let mut chain =
            Chain::with_journal_capacities(genesis.clone(), NETWORK_TIME, 8).expect("genesis");
        let parent = chain.active_block_at(0).expect("genesis").clone();

        let secp = secp256k1::Secp256k1::new();
        let sk = secp256k1::SecretKey::from_slice(&[0x55; 32]).expect("secret");
        let pk = secp256k1::PublicKey::from_secret_key(&secp, &sk);
        let pubkey_bytes = pk.serialize();
        let lock_script =
            bitrst_script::p2pkh_script_pubkey(&bitrst_crypto::hash160::hash160(&pubkey_bytes));

        let mut fund = mine_block_on(&parent, NETWORK_TIME + 800, 1);
        fund.transactions[0].outputs[0].script_pubkey = lock_script.clone();
        fund.header.merkle_root = fund.merkle_root().expect("merkle");
        let fund_bits = fund.header.bits;
        common::mine_header_for_bits(&mut fund.header, fund_bits);
        chain.connect_block(fund).expect("fund");
        let funding_txid = chain.active_block_at(1).expect("fund").transactions[0].txid();

        (chain, lock_script, funding_txid)
    };

    let fund_parent = chain.active_block_at(1).expect("fund block").clone();
    let spend = sign_spend(funding_txid, &lock_script);
    let spend_txid = spend.txid();
    let mut invalid = spend.clone();
    invalid.inputs[0].script_sig = vec![0x01];

    let mut pool = Mempool::new(MempoolLimits::default());
    pool.accept_tx(spend.clone(), chain.utxo()).expect("accept");

    let mut confirm = mine_block_on(&fund_parent, NETWORK_TIME + 900, 2);
    confirm.transactions.push(spend.clone());
    confirm.header.merkle_root = confirm.merkle_root().expect("merkle");
    let confirm_bits = confirm.header.bits;
    common::mine_header_for_bits(&mut confirm.header, confirm_bits);
    chain.connect_block(confirm).expect("confirm");
    pool.synchronize_to_chain(
        chain.utxo(),
        &[chain.active_block_at(2).expect("confirm").clone()],
        &[],
    );
    assert!(!pool.contains(&spend_txid));

    let since_seq = chain.event_cursor().last_seq;
    let alt_b3 = mine_block_on(&fund_parent, NETWORK_TIME + 1000, 2);
    let alt_b4 = mine_block_on(&alt_b3, NETWORK_TIME + 1100, 3);
    chain.connect_block(alt_b3).expect("alt b3");
    chain.take_events().expect("events");
    let result = chain.connect_block(alt_b4).expect("reorg");
    assert!(matches!(result, ConnectResult::Reorganized { .. }));

    pool.resync_to_active_chain(&chain, since_seq)
        .expect("resync after lag");
    assert!(pool.contains(&spend_txid));
    assert!(!pool.contains(&invalid.txid()));
    assert!(!pool.is_valid_tx(&invalid, chain.utxo()));
    for txid in pool.txids() {
        let tx = pool.get_transaction(&txid).expect("tx");
        assert!(pool.is_valid_tx(&tx, chain.utxo()));
    }
}

fn sign_spend(funding_txid: [u8; 32], lock_script: &[u8]) -> bitrst_core::Transaction {
    use bitrst_core::{Transaction, TxInput, TxOutput};
    use bitrst_script::p2pkh_script_sig;
    use secp256k1::{Message, Secp256k1, SecretKey};

    let secp = Secp256k1::new();
    let sk = SecretKey::from_slice(&[0x55; 32]).expect("secret");
    let pk = secp256k1::PublicKey::from_secret_key(&secp, &sk);
    let pubkey_bytes = pk.serialize();

    let mut tx = Transaction {
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
    let prev_scripts = vec![lock_script.to_vec()];
    let sighash = bitrst_core::sighash_all(&tx, 0, &prev_scripts).expect("sighash");
    let sig = secp.sign_ecdsa(&Message::from_digest(sighash), &sk);
    let mut sig_bytes = sig.serialize_der().to_vec();
    sig_bytes.push(0x01);
    tx.inputs[0].script_sig = p2pkh_script_sig(&sig_bytes, &pubkey_bytes);
    tx
}
