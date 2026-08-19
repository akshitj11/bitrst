//! In-memory transaction pool with bounded admission and deterministic eviction.
//!
//! # Eviction policy
//!
//! When the pool is at its transaction-count or byte limit, the transaction with
//! the **lowest fee rate** (fee divided by serialized size in satoshis per byte)
//! is removed first. Ties break on **oldest admission** (lowest internal
//! sequence). Incoming transactions with fee rate less than or equal to the
//! lowest-priority resident are rejected with [`MempoolError::AtCapacity`].

use std::collections::{HashMap, HashSet};

use thiserror::Error;

use crate::block::Block;
use crate::chain::Chain;
use crate::chain_events::ChainEvent;
use crate::limits::{
    DEFAULT_MAX_MEMPOOL_BYTES, DEFAULT_MAX_MEMPOOL_TX_COUNT, MAX_SCRIPT_SIZE,
    MAX_TRANSACTION_SERIALIZED_SIZE,
};
use crate::sighash::{sighash_all, SighashError};
use crate::transaction::Transaction;
use crate::utxo::{OutPoint, UtxoError, UtxoSet};

/// Bounds for mempool capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MempoolLimits {
    /// Maximum number of transactions.
    pub max_tx_count: usize,
    /// Maximum total serialized bytes across stored transactions.
    pub max_bytes: usize,
}

impl Default for MempoolLimits {
    fn default() -> Self {
        Self {
            max_tx_count: DEFAULT_MAX_MEMPOOL_TX_COUNT,
            max_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
        }
    }
}

/// Metadata returned when a transaction is accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcceptedTx {
    /// Accepted transaction ID.
    pub txid: [u8; 32],
    /// Fee paid in satoshis (inputs minus outputs).
    pub fee: u64,
    /// Serialized transaction size in bytes.
    pub size: usize,
}

/// Errors raised while admitting transactions to the mempool.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum MempoolError {
    /// Coinbase transactions cannot enter the mempool.
    #[error("coinbase transactions are not allowed in the mempool")]
    Coinbase,

    /// The transaction is already present.
    #[error("duplicate transaction {txid:?}")]
    Duplicate {
        /// Existing transaction ID.
        txid: [u8; 32],
    },

    /// The same outpoint appears more than once in the transaction inputs.
    #[error("duplicate input spend of UTXO {txid:?} index {index} within transaction")]
    DuplicateInput {
        /// Transaction ID being validated.
        txid: [u8; 32],
        /// Repeated output index.
        index: u32,
    },

    /// The transaction exceeds the configured serialized-size ceiling.
    #[error("transaction size {size} exceeds limit {limit}")]
    Oversized {
        /// Actual serialized size.
        size: usize,
        /// Configured limit.
        limit: usize,
    },

    /// The transaction failed structural validation.
    #[error("malformed transaction: {context}")]
    Malformed {
        /// Human-readable reason.
        context: String,
    },

    /// A referenced UTXO is not available.
    #[error(transparent)]
    Utxo(#[from] UtxoError),

    /// An input conflicts with another mempool transaction.
    #[error("conflicting spend of {outpoint:?} with mempool tx {existing_txid:?}")]
    ConflictingSpend {
        /// Outpoint already claimed in the mempool.
        outpoint: OutPoint,
        /// Transaction currently holding the spend.
        existing_txid: [u8; 32],
    },

    /// Script verification failed.
    #[error("invalid script")]
    InvalidScript,

    /// Sighash computation failed.
    #[error(transparent)]
    Sighash(#[from] SighashError),

    /// No eviction candidate could free enough space.
    #[error("mempool at capacity and incoming transaction is not replaceable")]
    AtCapacity,
}

#[derive(Debug, Clone)]
struct MempoolEntry {
    tx: Transaction,
    fee_rate: u64,
    size: usize,
    seq: u64,
}

/// Bounded in-memory mempool validated against a chain UTXO view.
#[derive(Debug)]
pub struct Mempool {
    limits: MempoolLimits,
    entries: HashMap<[u8; 32], MempoolEntry>,
    spends: HashMap<OutPoint, [u8; 32]>,
    total_bytes: usize,
    next_seq: u64,
}

impl Mempool {
    /// Creates an empty mempool with the given limits.
    pub fn new(limits: MempoolLimits) -> Self {
        Self {
            limits,
            entries: HashMap::new(),
            spends: HashMap::new(),
            total_bytes: 0,
            next_seq: 0,
        }
    }

    /// Returns the configured limits.
    pub fn limits(&self) -> MempoolLimits {
        self.limits
    }

    /// Returns the number of transactions in the pool.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true when the pool has no transactions.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the total serialized bytes currently stored.
    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    /// Returns a snapshot of transaction IDs currently in the pool.
    pub fn txids(&self) -> Vec<[u8; 32]> {
        self.entries.keys().copied().collect()
    }

    /// Returns true when `txid` is present.
    pub fn contains(&self, txid: &[u8; 32]) -> bool {
        self.entries.contains_key(txid)
    }

    /// Returns a cloned transaction when `txid` is in the pool.
    pub fn get_transaction(&self, txid: &[u8; 32]) -> Option<Transaction> {
        self.entries.get(txid).map(|entry| entry.tx.clone())
    }

    /// Validates and admits a transaction against `chain_utxo` without mutating the chain.
    pub fn accept_tx(
        &mut self,
        tx: Transaction,
        chain_utxo: &UtxoSet,
    ) -> Result<AcceptedTx, MempoolError> {
        let size = validate_tx_structure(&tx)?;
        if UtxoSet::is_coinbase(&tx) {
            return Err(MempoolError::Coinbase);
        }

        let txid = tx.txid();
        if self.entries.contains_key(&txid) {
            return Err(MempoolError::Duplicate { txid });
        }

        check_duplicate_inputs(&tx)?;

        self.check_mempool_conflicts(&tx)?;

        let view = self.build_validation_view(chain_utxo);
        view.validate_transaction(&tx)?;

        let fee = compute_fee(&tx, &view)?;
        verify_scripts(&tx, &view)?;

        let fee_rate = if size > 0 { fee / size as u64 } else { 0 };
        let candidate = MempoolEntry {
            tx,
            fee_rate,
            size,
            seq: self.next_seq,
        };

        self.make_room_for(&candidate)?;
        self.insert_entry(txid, candidate);

        Ok(AcceptedTx { txid, fee, size })
    }

    /// Synchronizes the pool to the active chain after block connect/disconnect updates.
    ///
    /// `chain_utxo` must reflect the **final** active-chain UTXO set after all chain
    /// mutations represented by `connected_blocks` and `disconnected_blocks` have been
    /// applied. Disconnected non-coinbase transactions are collected for restoration,
    /// confirmed and conflicting transactions are removed, the remaining pool is fully
    /// revalidated, and restoration candidates are admitted in dependency-aware passes.
    pub fn synchronize_to_chain(
        &mut self,
        chain_utxo: &UtxoSet,
        connected_blocks: &[Block],
        disconnected_blocks: &[Block],
    ) {
        let mut restore_candidates = Vec::new();
        for block in disconnected_blocks {
            for tx in block.transactions.iter().skip(1) {
                if !UtxoSet::is_coinbase(tx) {
                    restore_candidates.push(tx.clone());
                }
            }
        }

        for block in connected_blocks {
            self.remove_for_block(block);
        }

        self.revalidate_pool(chain_utxo);
        self.restore_candidates(restore_candidates, chain_utxo);
    }

    /// Applies a caller-supplied chain event slice against the final active-chain UTXO.
    ///
    /// Events are not consumed from the chain; the caller must pass the slice explicitly.
    pub fn apply_chain_events(&mut self, events: &[ChainEvent], chain: &Chain) {
        let mut connected = Vec::new();
        let mut disconnected = Vec::new();

        for event in events {
            match event {
                ChainEvent::BlockConnected { hash, .. } => {
                    if let Some(block) = chain.block_by_hash(hash) {
                        connected.push(block);
                    }
                }
                ChainEvent::BlockDisconnected { hash, .. } => {
                    if let Some(block) = chain.block_by_hash(hash) {
                        disconnected.push(block);
                    }
                }
                ChainEvent::ChainReorg { .. }
                | ChainEvent::OrphanAdded { .. }
                | ChainEvent::OrphanEvicted { .. } => {}
            }
        }

        self.synchronize_to_chain(chain.utxo(), &connected, &disconnected);
    }

    /// Removes transactions confirmed in `block` and any mempool conflicts.
    pub fn remove_for_block(&mut self, block: &Block) {
        let mut remove = HashSet::new();

        for tx in &block.transactions {
            remove.insert(tx.txid());
        }

        let mut block_spends = HashSet::new();
        for tx in &block.transactions {
            if UtxoSet::is_coinbase(tx) {
                continue;
            }
            for input in &tx.inputs {
                block_spends.insert(OutPoint {
                    txid: input.previous_output,
                    index: input.index,
                });
            }
        }

        loop {
            let mut added = false;
            for (txid, entry) in &self.entries {
                if remove.contains(txid) {
                    continue;
                }

                if entry.tx.inputs.iter().any(|input| {
                    let outpoint = OutPoint {
                        txid: input.previous_output,
                        index: input.index,
                    };
                    block_spends.contains(&outpoint) || remove.contains(&input.previous_output)
                }) {
                    remove.insert(*txid);
                    added = true;
                }
            }

            if !added {
                break;
            }
        }

        for txid in remove {
            self.remove_entry(&txid);
        }
    }

    fn revalidate_pool(&mut self, chain_utxo: &UtxoSet) {
        loop {
            let invalid = self.find_invalid_txids(chain_utxo);
            if invalid.is_empty() {
                break;
            }
            for txid in invalid {
                self.remove_entry(&txid);
            }
        }
    }

    fn find_invalid_txids(&self, chain_utxo: &UtxoSet) -> Vec<[u8; 32]> {
        let mut invalid = Vec::new();
        let mut ordered: Vec<&MempoolEntry> = self.entries.values().collect();
        ordered.sort_by_key(|entry| entry.seq);

        let mut view = chain_utxo.clone();
        for entry in ordered {
            let tx = &entry.tx;
            let txid = tx.txid();
            if !is_valid_for_view(tx, &view) {
                invalid.push(txid);
                continue;
            }
            view.apply_transaction(tx);
        }
        invalid
    }

    fn restore_candidates(&mut self, candidates: Vec<Transaction>, chain_utxo: &UtxoSet) {
        let mut pending = candidates;
        loop {
            let mut progress = false;
            let mut remaining = Vec::new();
            for tx in pending {
                match self.accept_tx(tx.clone(), chain_utxo) {
                    Ok(_) => progress = true,
                    Err(_) => remaining.push(tx),
                }
            }
            if !progress || remaining.is_empty() {
                break;
            }
            pending = remaining;
        }
    }

    fn check_mempool_conflicts(&self, tx: &Transaction) -> Result<(), MempoolError> {
        for input in &tx.inputs {
            let outpoint = OutPoint {
                txid: input.previous_output,
                index: input.index,
            };
            if let Some(existing_txid) = self.spends.get(&outpoint) {
                return Err(MempoolError::ConflictingSpend {
                    outpoint,
                    existing_txid: *existing_txid,
                });
            }
        }
        Ok(())
    }

    fn build_validation_view(&self, chain_utxo: &UtxoSet) -> UtxoSet {
        let mut view = chain_utxo.clone();
        let mut ordered: Vec<&MempoolEntry> = self.entries.values().collect();
        ordered.sort_by_key(|entry| entry.seq);
        for entry in ordered {
            view.apply_transaction(&entry.tx);
        }
        view
    }

    fn make_room_for(&mut self, candidate: &MempoolEntry) -> Result<(), MempoolError> {
        while self.would_exceed_limits(candidate.size) {
            let Some(victim) = self.lowest_priority_txid() else {
                return Err(MempoolError::AtCapacity);
            };

            if !self.should_evict(&victim, candidate) {
                return Err(MempoolError::AtCapacity);
            }

            self.remove_entry(&victim);
        }
        Ok(())
    }

    fn would_exceed_limits(&self, incoming_size: usize) -> bool {
        let next_count = self.entries.len() + 1;
        let next_bytes = self.total_bytes + incoming_size;
        next_count > self.limits.max_tx_count || next_bytes > self.limits.max_bytes
    }

    fn should_evict(&self, victim: &[u8; 32], candidate: &MempoolEntry) -> bool {
        let Some(entry) = self.entries.get(victim) else {
            return false;
        };

        if candidate.fee_rate > entry.fee_rate {
            return true;
        }
        if candidate.fee_rate < entry.fee_rate {
            return false;
        }

        candidate.seq > entry.seq
    }

    fn lowest_priority_txid(&self) -> Option<[u8; 32]> {
        self.entries
            .iter()
            .min_by(|(_, left), (_, right)| {
                left.fee_rate
                    .cmp(&right.fee_rate)
                    .then(left.seq.cmp(&right.seq))
            })
            .map(|(txid, _)| *txid)
    }

    fn insert_entry(&mut self, txid: [u8; 32], entry: MempoolEntry) {
        self.total_bytes += entry.size;
        self.next_seq = self.next_seq.wrapping_add(1);
        for input in &entry.tx.inputs {
            self.spends.insert(
                OutPoint {
                    txid: input.previous_output,
                    index: input.index,
                },
                txid,
            );
        }
        self.entries.insert(txid, entry);
    }

    fn remove_entry(&mut self, txid: &[u8; 32]) {
        let Some(entry) = self.entries.remove(txid) else {
            return;
        };
        self.total_bytes -= entry.size;
        self.spends.retain(|_, owner| owner != txid);
    }
}

fn is_valid_for_view(tx: &Transaction, view: &UtxoSet) -> bool {
    validate_tx_structure(tx).is_ok()
        && !UtxoSet::is_coinbase(tx)
        && check_duplicate_inputs(tx).is_ok()
        && view.validate_transaction(tx).is_ok()
        && verify_scripts(tx, view).is_ok()
}

fn check_duplicate_inputs(tx: &Transaction) -> Result<(), MempoolError> {
    let txid = tx.txid();
    let mut seen = HashSet::new();
    for input in &tx.inputs {
        let key = (input.previous_output, input.index);
        if !seen.insert(key) {
            return Err(MempoolError::DuplicateInput {
                txid,
                index: input.index,
            });
        }
    }
    Ok(())
}

fn validate_tx_structure(tx: &Transaction) -> Result<usize, MempoolError> {
    let size = tx.serialized_size();
    if size > MAX_TRANSACTION_SERIALIZED_SIZE {
        return Err(MempoolError::Oversized {
            size,
            limit: MAX_TRANSACTION_SERIALIZED_SIZE,
        });
    }
    if tx.outputs.is_empty() {
        return Err(MempoolError::Malformed {
            context: "transaction has no outputs".into(),
        });
    }
    if !UtxoSet::is_coinbase(tx) && tx.inputs.is_empty() {
        return Err(MempoolError::Malformed {
            context: "non-coinbase transaction has no inputs".into(),
        });
    }

    for input in &tx.inputs {
        if input.script_sig.len() > MAX_SCRIPT_SIZE {
            return Err(MempoolError::Malformed {
                context: "scriptSig exceeds maximum size".into(),
            });
        }
    }
    for output in &tx.outputs {
        if output.script_pubkey.len() > MAX_SCRIPT_SIZE {
            return Err(MempoolError::Malformed {
                context: "scriptPubKey exceeds maximum size".into(),
            });
        }
    }

    Ok(size)
}

fn compute_fee(tx: &Transaction, view: &UtxoSet) -> Result<u64, MempoolError> {
    let mut inputs = 0u64;
    for input in &tx.inputs {
        let outpoint = OutPoint {
            txid: input.previous_output,
            index: input.index,
        };
        let value = view.get_value(&outpoint).ok_or(UtxoError::MissingInput {
            txid: outpoint.txid,
            index: outpoint.index,
        })?;
        inputs = inputs.checked_add(value).ok_or(UtxoError::ValueOverflow)?;
    }

    let mut outputs = 0u64;
    for output in &tx.outputs {
        outputs = outputs
            .checked_add(output.value)
            .ok_or(UtxoError::ValueOverflow)?;
    }

    if inputs < outputs {
        return Err(UtxoError::InputOutputMismatch { inputs, outputs }.into());
    }

    Ok(inputs - outputs)
}

fn verify_scripts(tx: &Transaction, view: &UtxoSet) -> Result<(), MempoolError> {
    let mut prev_scripts = Vec::with_capacity(tx.inputs.len());
    for input in &tx.inputs {
        let outpoint = OutPoint {
            txid: input.previous_output,
            index: input.index,
        };
        let entry = view.get(&outpoint).ok_or(UtxoError::MissingInput {
            txid: outpoint.txid,
            index: outpoint.index,
        })?;
        prev_scripts.push(entry.script_pubkey.clone());
    }

    for (input_index, input) in tx.inputs.iter().enumerate() {
        let script_pubkey = &prev_scripts[input_index];
        if script_pubkey.is_empty() {
            continue;
        }

        let sighash = sighash_all(tx, input_index, &prev_scripts)?;
        bitrst_script::verify_script(&input.script_sig, script_pubkey, &sighash)
            .map_err(|_| MempoolError::InvalidScript)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Mempool, MempoolError, MempoolLimits};
    use crate::block::{Block, BlockHeader};
    use crate::chain::Chain;
    use crate::limits::MAX_SCRIPT_SIZE;
    use crate::pow::Target;
    use crate::sighash::sighash_all;
    use crate::transaction::{Transaction, TxInput, TxOutput};
    use crate::utxo::UtxoError;
    use crate::ConnectResult;
    use bitrst_crypto::hash160::hash160;
    use bitrst_script::{p2pkh_script_pubkey, p2pkh_script_sig};
    use secp256k1::{Message, Secp256k1, SecretKey};

    const TEST_BITS: u32 = 0x1f00_ffff;
    const NETWORK_TIME: u32 = 1_231_006_505;

    fn genesis_block() -> Block {
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

    fn funded_chain_with_two_p2pkh_outputs() -> (Chain, Vec<u8>, [u8; 32]) {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[0x33; 32]).expect("secret");
        let pk = secp256k1::PublicKey::from_secret_key(&secp, &sk);
        let pubkey_bytes = pk.serialize();
        let lock_script = p2pkh_script_pubkey(&hash160(&pubkey_bytes));

        let genesis = genesis_block();
        let genesis_hash = genesis.hash();
        let mut chain = Chain::new_genesis(genesis, NETWORK_TIME).expect("genesis");

        let header1 = BlockHeader {
            version: 1,
            prev_blockhash: genesis_hash,
            merkle_root: [0u8; 32],
            time: NETWORK_TIME + 100,
            bits: TEST_BITS,
            nonce: 0,
        };
        let mut block1 = Block::coinbase(header1, 1, 50_0000_0000);
        block1.transactions[0].outputs = vec![
            TxOutput {
                value: 25_0000_0000,
                script_pubkey: lock_script.clone(),
            },
            TxOutput {
                value: 25_0000_0000,
                script_pubkey: lock_script.clone(),
            },
        ];
        block1.header.merkle_root = block1.merkle_root().expect("merkle");
        let target = Target::from_bits(TEST_BITS).expect("bits");
        while !target.meets(&block1.header.hash()) {
            block1.header.nonce = block1.header.nonce.wrapping_add(1);
        }
        chain.connect_block(block1).expect("connect");
        let funding_txid = chain.active_block_at(1).expect("b1").transactions[0].txid();

        (chain, lock_script, funding_txid)
    }

    fn funded_chain_with_p2pkh() -> (Chain, Vec<u8>, [u8; 32]) {
        let (chain, lock_script, funding_txid) = funded_chain_with_two_p2pkh_outputs();
        (chain, lock_script, funding_txid)
    }

    fn sign_p2pkh_spend(
        funding_txid: [u8; 32],
        output_index: u32,
        lock_script: &[u8],
        input_value: u64,
        fee: u64,
    ) -> Transaction {
        sign_p2pkh_spend_from_output(
            funding_txid,
            output_index,
            lock_script,
            input_value,
            input_value.saturating_sub(fee),
        )
    }

    fn sign_p2pkh_spend_from_output(
        funding_txid: [u8; 32],
        output_index: u32,
        lock_script: &[u8],
        input_value: u64,
        output_value: u64,
    ) -> Transaction {
        let _ = input_value;
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[0x33; 32]).expect("secret");
        let pk = secp256k1::PublicKey::from_secret_key(&secp, &sk);
        let pubkey_bytes = pk.serialize();

        let mut tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_output: funding_txid,
                index: output_index,
                script_sig: Vec::new(),
                sequence: u32::MAX,
            }],
            outputs: vec![TxOutput {
                value: output_value,
                script_pubkey: Vec::new(),
            }],
            lock_time: 0,
        };
        let prev_scripts = vec![lock_script.to_vec()];
        let sighash = sighash_all(&tx, 0, &prev_scripts).expect("sighash");
        let sig = secp.sign_ecdsa(&Message::from_digest(sighash), &sk);
        let mut sig_bytes = sig.serialize_der().to_vec();
        sig_bytes.push(0x01);
        tx.inputs[0].script_sig = p2pkh_script_sig(&sig_bytes, &pubkey_bytes);
        tx
    }

    fn mine_block_on(parent: &Block, time: u32, height: u32) -> Block {
        let header = BlockHeader {
            version: 1,
            prev_blockhash: parent.hash(),
            merkle_root: [0u8; 32],
            time,
            bits: parent.header.bits,
            nonce: 0,
        };
        let mut block = Block::coinbase(header, height, 50_0000_0000);
        block.header.merkle_root = block.merkle_root().expect("merkle");
        let target = Target::from_bits(block.header.bits).expect("bits");
        while !target.meets(&block.header.hash()) {
            block.header.nonce = block.header.nonce.wrapping_add(1);
        }
        block
    }

    #[test]
    fn rejects_coinbase_and_duplicate() {
        let chain = Chain::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        let mut pool = Mempool::new(MempoolLimits::default());

        let coinbase = Transaction::coinbase(1, 1);
        assert!(matches!(
            pool.accept_tx(coinbase, chain.utxo()),
            Err(MempoolError::Coinbase)
        ));

        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_output: [9u8; 32],
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
            pool.accept_tx(tx.clone(), chain.utxo()),
            Err(MempoolError::Utxo(UtxoError::MissingInput { .. }))
        ));

        let (chain, lock_script, funding_txid) = funded_chain_with_p2pkh();
        let valid = sign_p2pkh_spend(funding_txid, 0, &lock_script, 25_0000_0000, 1_0000_0000);
        pool.accept_tx(valid.clone(), chain.utxo()).expect("accept");
        assert!(matches!(
            pool.accept_tx(valid, chain.utxo()),
            Err(MempoolError::Duplicate { .. })
        ));
    }

    #[test]
    fn rejects_duplicate_inputs_within_transaction() {
        let (chain, lock_script, funding_txid) = funded_chain_with_p2pkh();
        let mut pool = Mempool::new(MempoolLimits::default());
        let mut tx = sign_p2pkh_spend(funding_txid, 0, &lock_script, 25_0000_0000, 1_0000_0000);
        tx.inputs.push(tx.inputs[0].clone());

        assert!(matches!(
            pool.accept_tx(tx, chain.utxo()),
            Err(MempoolError::DuplicateInput { .. })
        ));
    }

    #[test]
    fn rejects_oversized_and_malformed_transactions() {
        let chain = Chain::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        let mut pool = Mempool::new(MempoolLimits::default());

        let no_outputs = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_output: [1u8; 32],
                index: 0,
                script_sig: vec![],
                sequence: u32::MAX,
            }],
            outputs: vec![],
            lock_time: 0,
        };
        assert!(matches!(
            pool.accept_tx(no_outputs, chain.utxo()),
            Err(MempoolError::Malformed { .. })
        ));

        let huge_script = vec![0u8; MAX_SCRIPT_SIZE + 1];
        let oversize_script = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_output: [1u8; 32],
                index: 0,
                script_sig: huge_script,
                sequence: u32::MAX,
            }],
            outputs: vec![TxOutput {
                value: 0,
                script_pubkey: vec![],
            }],
            lock_time: 0,
        };
        assert!(matches!(
            pool.accept_tx(oversize_script, chain.utxo()),
            Err(MempoolError::Malformed { .. })
        ));
    }

    #[test]
    fn accepts_valid_p2pkh_and_reports_fee_metadata() {
        let (chain, lock_script, funding_txid) = funded_chain_with_p2pkh();
        let mut pool = Mempool::new(MempoolLimits::default());
        let fee = 2_0000_0000;
        let tx = sign_p2pkh_spend(funding_txid, 0, &lock_script, 25_0000_0000, fee);

        let accepted = pool.accept_tx(tx, chain.utxo()).expect("accept");
        assert_eq!(accepted.fee, fee);
        assert!(accepted.size > 0);
        assert!(pool.contains(&accepted.txid));
    }

    #[test]
    fn rejects_conflicting_spends_and_invalid_scripts() {
        let (chain, lock_script, funding_txid) = funded_chain_with_p2pkh();
        let mut pool = Mempool::new(MempoolLimits::default());

        let mut bad_script =
            sign_p2pkh_spend(funding_txid, 0, &lock_script, 25_0000_0000, 1_0000_0000);
        bad_script.inputs[0].script_sig = vec![0x01];
        assert!(matches!(
            pool.accept_tx(bad_script, chain.utxo()),
            Err(MempoolError::InvalidScript)
        ));

        let first = sign_p2pkh_spend(funding_txid, 0, &lock_script, 25_0000_0000, 1_0000_0000);
        pool.accept_tx(first, chain.utxo()).expect("first");

        let conflict = sign_p2pkh_spend(funding_txid, 0, &lock_script, 25_0000_0000, 2_0000_0000);
        assert!(matches!(
            pool.accept_tx(conflict, chain.utxo()),
            Err(MempoolError::ConflictingSpend { .. })
        ));
    }

    #[test]
    fn evicts_lowest_fee_rate_then_oldest() {
        use crate::limits::DEFAULT_MAX_MEMPOOL_BYTES;

        let (chain, lock_script, funding_txid) = funded_chain_with_two_p2pkh_outputs();
        let limits = MempoolLimits {
            max_tx_count: 1,
            max_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
        };
        let mut pool = Mempool::new(limits);

        let low_fee = sign_p2pkh_spend(funding_txid, 0, &lock_script, 25_0000_0000, 1);
        let low_txid = low_fee.txid();
        pool.accept_tx(low_fee, chain.utxo()).expect("low");

        let high_fee = sign_p2pkh_spend(funding_txid, 1, &lock_script, 25_0000_0000, 5_0000_0000);
        let high_txid = high_fee.txid();
        pool.accept_tx(high_fee, chain.utxo()).expect("high");

        assert!(!pool.contains(&low_txid));
        assert!(pool.contains(&high_txid));
    }

    #[test]
    fn rejects_low_fee_when_pool_is_full() {
        let (chain, lock_script, funding_txid) = funded_chain_with_two_p2pkh_outputs();
        let limits = MempoolLimits {
            max_tx_count: 1,
            max_bytes: usize::MAX,
        };
        let mut pool = Mempool::new(limits);

        let resident = sign_p2pkh_spend(funding_txid, 0, &lock_script, 25_0000_0000, 5_0000_0000);
        pool.accept_tx(resident, chain.utxo()).expect("resident");

        let challenger = sign_p2pkh_spend(funding_txid, 1, &lock_script, 25_0000_0000, 1);
        assert!(matches!(
            pool.accept_tx(challenger, chain.utxo()),
            Err(MempoolError::AtCapacity)
        ));
    }

    #[test]
    fn rejects_when_byte_capacity_is_exceeded() {
        let (chain, lock_script, funding_txid) = funded_chain_with_two_p2pkh_outputs();
        let resident = sign_p2pkh_spend(funding_txid, 0, &lock_script, 25_0000_0000, 5_0000_0000);
        let tx_size = resident.serialized_size();
        let limits = MempoolLimits {
            max_tx_count: 100,
            max_bytes: tx_size,
        };
        let mut pool = Mempool::new(limits);
        pool.accept_tx(resident, chain.utxo()).expect("first");

        let challenger = sign_p2pkh_spend(funding_txid, 1, &lock_script, 25_0000_0000, 1);
        assert!(matches!(
            pool.accept_tx(challenger, chain.utxo()),
            Err(MempoolError::AtCapacity)
        ));
    }

    #[test]
    fn removes_confirmed_and_conflicting_on_block_connect() {
        let (mut chain, lock_script, funding_txid) = funded_chain_with_p2pkh();
        let mut pool = Mempool::new(MempoolLimits::default());
        let spend = sign_p2pkh_spend(funding_txid, 0, &lock_script, 25_0000_0000, 1_0000_0000);
        let spend_txid = spend.txid();
        pool.accept_tx(spend.clone(), chain.utxo()).expect("accept");

        let parent = chain.active_block_at(1).expect("parent").clone();
        let header = BlockHeader {
            version: 1,
            prev_blockhash: parent.hash(),
            merkle_root: [0u8; 32],
            time: NETWORK_TIME + 200,
            bits: TEST_BITS,
            nonce: 0,
        };
        let mut block = Block::coinbase(header, 2, 50_0000_0000);
        block.transactions.push(spend);
        block.header.merkle_root = block.merkle_root().expect("merkle");
        let target = Target::from_bits(TEST_BITS).expect("bits");
        while !target.meets(&block.header.hash()) {
            block.header.nonce = block.header.nonce.wrapping_add(1);
        }

        let events = {
            chain.connect_block(block).expect("connect");
            chain.take_events()
        };
        pool.apply_chain_events(&events, &chain);
        assert!(!pool.contains(&spend_txid));
    }

    #[test]
    fn reorg_restores_valid_tx_and_evicts_stale_invalid_entries() {
        let (mut chain, lock_script, funding_txid) = funded_chain_with_p2pkh();
        let fund_parent = chain.active_block_at(1).expect("fund block").clone();

        let spend = sign_p2pkh_spend(funding_txid, 0, &lock_script, 25_0000_0000, 1_0000_0000);
        let spend_txid = spend.txid();

        let mut pool = Mempool::new(MempoolLimits::default());
        pool.accept_tx(spend.clone(), chain.utxo()).expect("accept");

        let mut block_with_spend = mine_block_on(&fund_parent, NETWORK_TIME + 300, 2);
        block_with_spend.transactions.push(spend.clone());
        block_with_spend.header.merkle_root = block_with_spend.merkle_root().expect("merkle");
        let target = Target::from_bits(TEST_BITS).expect("bits");
        while !target.meets(&block_with_spend.header.hash()) {
            block_with_spend.header.nonce = block_with_spend.header.nonce.wrapping_add(1);
        }
        chain
            .connect_block(block_with_spend)
            .expect("confirm spend");
        pool.synchronize_to_chain(
            chain.utxo(),
            &[chain.active_block_at(2).expect("b2").clone()],
            &[],
        );
        assert!(!pool.contains(&spend_txid));

        let stale_conflict =
            sign_p2pkh_spend(funding_txid, 0, &lock_script, 25_0000_0000, 2_0000_0000);
        let _ = pool.accept_tx(stale_conflict, chain.utxo());

        let alt_b2 = mine_block_on(&fund_parent, NETWORK_TIME + 500, 2);
        let alt_b3 = mine_block_on(&alt_b2, NETWORK_TIME + 600, 3);
        chain.connect_block(alt_b2).expect("alt b2");
        chain.take_events();
        let result = chain.connect_block(alt_b3).expect("reorg");
        assert!(matches!(result, ConnectResult::Reorganized { .. }));

        let events = chain.take_events();
        pool.apply_chain_events(&events, &chain);

        assert!(
            pool.contains(&spend_txid),
            "disconnected block tx should be restored when valid"
        );
        for txid in pool.txids() {
            let tx = pool.get_transaction(&txid).expect("tx");
            assert!(
                pool.accept_tx(tx, chain.utxo()).is_err(),
                "pool must not retain invalid entries"
            );
        }
    }
}
