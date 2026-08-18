//! Block chain validation, orphan handling, and reorg logic.
//!
//! A [`Chain`] tracks the active proof-of-work chain, the UTXO set for that chain,
//! and orphan blocks waiting for unknown parents. The active tip is chosen by
//! cumulative proof-of-work, not block height alone.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};

use thiserror::Error;

use crate::block::Block;
use crate::chain_events::{ChainEvent, EvictionReason};
use crate::difficulty::{
    adjust_bits, difficulty_adjustment_interval, DifficultyError, MAX_COMPACT_BITS,
};
use crate::limits::{
    MAX_BLOCK_SERIALIZED_SIZE, MAX_ORPHAN_BLOCKS, MAX_SCRIPT_SIZE, MAX_TRANSACTIONS_PER_BLOCK,
};
use crate::pow::Target;
use crate::sighash::{sighash_all, SighashError};
use crate::store::{BlockStore, MemoryBlockStore, StoreError};
use crate::time::valid_block_time;
use crate::uint256::cmp_le;
use crate::utxo::{OutPoint, TxUndo, UtxoError, UtxoSet};

/// Total cumulative proof-of-work on a chain branch (256-bit, compared MSB-first).
#[derive(Debug, Clone, Copy, Default)]
pub struct ChainWork(pub [u8; 32]);

impl ChainWork {
    #[allow(clippy::needless_range_loop)]
    fn add(self, other: Self) -> Self {
        let mut out = [0u8; 32];
        let mut carry = 0u16;
        for index in 0..32 {
            let sum = u16::from(self.0[index]) + u16::from(other.0[index]) + carry;
            out[index] = sum as u8;
            carry = sum >> 8;
        }
        debug_assert_eq!(carry, 0, "ChainWork addition overflowed 256 bits");
        Self(out)
    }
}

impl PartialEq for ChainWork {
    fn eq(&self, other: &Self) -> bool {
        cmp_le(&self.0, &other.0) == Ordering::Equal
    }
}

impl Eq for ChainWork {}

impl PartialOrd for ChainWork {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ChainWork {
    fn cmp(&self, other: &Self) -> Ordering {
        cmp_le(&self.0, &other.0)
    }
}

/// Result of attempting to connect a block to the chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectResult {
    /// The block became part of the active chain.
    Connected {
        /// Height of the connected block.
        height: u32,
        /// Hash of the connected block header.
        hash: [u8; 32],
    },
    /// The block was stored until its parent arrives.
    Orphaned {
        /// Hash of the stored orphan block.
        hash: [u8; 32],
    },
    /// The active tip changed because a heavier fork was connected.
    Reorganized {
        /// Previous active tip hash.
        old_tip: [u8; 32],
        /// New active tip hash.
        new_tip: [u8; 32],
    },
    /// The block was valid but stored on a side chain with less cumulative work.
    SideChain {
        /// Height the block would have had on its fork.
        height: u32,
        /// Hash of the side-chain block.
        hash: [u8; 32],
    },
}

/// Errors raised while validating or connecting blocks.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ChainError {
    /// The block header failed the proof-of-work check.
    #[error("block header hash does not meet proof-of-work target")]
    InvalidProofOfWork,

    /// The header Merkle root did not match the transaction list.
    #[error("block merkle root does not match transaction list")]
    MerkleRootMismatch,

    /// The block did not contain exactly one coinbase transaction first.
    #[error("expected exactly one coinbase transaction, found {count}")]
    InvalidCoinbaseCount {
        /// Number of coinbase-like transactions detected.
        count: usize,
    },

    /// The block timestamp violated median-time-past or future-drift rules.
    #[error("block timestamp {block_time} is invalid (mtp={median_past}, network={network_time})")]
    InvalidTimestamp {
        /// Timestamp from the block header.
        block_time: u32,
        /// Median time past of prior blocks.
        median_past: u32,
        /// Network-adjusted clock used for the future-drift check.
        network_time: u32,
    },

    /// The compact `bits` field did not match the expected difficulty at this height.
    #[error(
        "unexpected compact bits {actual:#010x} at height {height}, expected {expected:#010x}"
    )]
    UnexpectedBits {
        /// Height of the block being connected.
        height: u32,
        /// Expected compact target.
        expected: u32,
        /// Actual compact target in the header.
        actual: u32,
    },

    /// A transaction in the block failed UTXO validation.
    #[error("transaction validation failed")]
    Utxo(#[from] UtxoError),

    /// Difficulty adjustment calculation failed.
    #[error("difficulty adjustment failed")]
    Difficulty(#[from] DifficultyError),

    /// The block hash is already known to this node.
    #[error("block is already known")]
    BlockAlreadyKnown,

    /// The genesis block was invalid.
    #[error("invalid genesis block")]
    InvalidGenesis,

    /// The block exceeds protocol size limits.
    #[error("block exceeds maximum serialized size")]
    BlockTooLarge,

    /// Too many transactions in the block.
    #[error("block exceeds maximum transaction count")]
    TooManyTransactions,

    /// A script in the block exceeds the size limit.
    #[error("transaction script exceeds maximum size")]
    ScriptTooLarge,

    /// An ancestor required for fork walking was missing from the block index.
    #[error("missing ancestor block in index")]
    MissingAncestor,

    /// The active chain has no tip to disconnect.
    #[error("no active tip to disconnect")]
    NoActiveTip,

    /// Block storage failed.
    #[error("block storage error")]
    Store(#[from] StoreError),

    /// An internal lock was poisoned.
    #[error("chain lock poisoned")]
    LockPoisoned,

    /// Network-adjusted time must be non-zero for future-drift checks.
    #[error("network time must be greater than zero")]
    InvalidNetworkTime,

    /// Script verification failed for a transaction input.
    #[error("invalid script")]
    InvalidScript,

    /// Sighash computation failed during script verification.
    #[error("sighash error")]
    Sighash(#[from] SighashError),
}

#[derive(Debug, Clone)]
struct BlockMeta {
    block: Block,
    height: u32,
    work: ChainWork,
    undo: Vec<TxUndo>,
}

#[derive(Debug, Clone)]
struct OrphanEntry {
    block: Block,
    received_at: u64,
}

/// In-memory block chain with UTXO state and orphan handling.
#[derive(Debug)]
pub struct Chain {
    blocks: Vec<Block>,
    active_hashes: HashSet<[u8; 32]>,
    known: HashMap<[u8; 32], BlockMeta>,
    total_work: ChainWork,
    utxo: UtxoSet,
    orphans: HashMap<[u8; 32], OrphanEntry>,
    orphan_receive_seq: u64,
    network_time: u32,
    store: MemoryBlockStore,
    events: Vec<ChainEvent>,
}

impl Chain {
    /// Creates a new chain with a valid genesis block at height 0.
    pub fn new_genesis(genesis: Block, network_time: u32) -> Result<Self, ChainError> {
        if network_time == 0 {
            return Err(ChainError::InvalidNetworkTime);
        }

        let mut chain = Self {
            blocks: Vec::new(),
            active_hashes: HashSet::new(),
            known: HashMap::new(),
            total_work: ChainWork::default(),
            utxo: UtxoSet::new(),
            orphans: HashMap::new(),
            orphan_receive_seq: 0,
            network_time,
            store: MemoryBlockStore::new(),
            events: Vec::new(),
        };

        let hash = genesis.hash();
        chain.validate_block_limits(&genesis)?;
        chain.validate_block_for_parent(&genesis, None, None, 0)?;
        let undo = chain.apply_block_transactions(&genesis)?;
        let work = block_work(genesis.header.bits)?;

        chain.store.put_block(&genesis)?;
        chain.known.insert(
            hash,
            BlockMeta {
                block: genesis.clone(),
                height: 0,
                work,
                undo,
            },
        );
        chain.blocks.push(genesis);
        chain.active_hashes.insert(hash);
        chain.total_work = work;
        chain.events.push(ChainEvent::BlockConnected {
            height: 0,
            hash,
            tx_count: chain.blocks[0].transactions.len(),
        });

        Ok(chain)
    }

    /// Returns the active chain height (genesis is height 0).
    pub fn height(&self) -> u32 {
        self.blocks.len().saturating_sub(1) as u32
    }

    /// Returns the number of blocks on the active chain (genesis included).
    pub fn active_block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Returns the hash of the active chain tip.
    pub fn tip_hash(&self) -> [u8; 32] {
        self.blocks.last().map(Block::hash).unwrap_or([0u8; 32])
    }

    /// Returns the block at `height` on the active chain (genesis is height 0).
    pub fn active_block_at(&self, height: u32) -> Option<&Block> {
        self.blocks.get(height as usize)
    }

    /// Returns a reference to the active UTXO set.
    pub fn utxo(&self) -> &UtxoSet {
        &self.utxo
    }

    /// Updates network-adjusted time used for future-drift checks.
    ///
    /// # Errors
    ///
    /// Returns [`ChainError::InvalidNetworkTime`] when `network_time` is zero.
    pub fn set_network_time(&mut self, network_time: u32) -> Result<(), ChainError> {
        if network_time == 0 {
            return Err(ChainError::InvalidNetworkTime);
        }
        self.network_time = network_time;
        Ok(())
    }

    /// Returns and clears pending chain events.
    pub fn take_events(&mut self) -> Vec<ChainEvent> {
        std::mem::take(&mut self.events)
    }

    /// Returns a reference to the block store.
    pub fn block_store(&self) -> &MemoryBlockStore {
        &self.store
    }

    /// Returns true when a block hash is known (active chain, side chain, or orphan pool).
    pub fn has_block_hash(&self, hash: &[u8; 32]) -> bool {
        self.known.contains_key(hash) || self.orphans.contains_key(hash)
    }

    /// Returns a cloned block for `hash` when it is known locally.
    pub fn block_by_hash(&self, hash: &[u8; 32]) -> Option<Block> {
        self.known
            .get(hash)
            .map(|meta| meta.block.clone())
            .or_else(|| self.orphans.get(hash).map(|entry| entry.block.clone()))
    }

    /// Attempts to connect a block to the chain.
    pub fn connect_block(&mut self, block: Block) -> Result<ConnectResult, ChainError> {
        self.connect_block_inner(block, true)
    }

    fn connect_block_inner(
        &mut self,
        block: Block,
        process_orphans: bool,
    ) -> Result<ConnectResult, ChainError> {
        self.validate_block_limits(&block)?;

        let hash = block.hash();
        if self.known.contains_key(&hash) || self.orphans.contains_key(&hash) {
            return Err(ChainError::BlockAlreadyKnown);
        }

        let parent_hash = block.header.prev_blockhash;
        let Some(parent_meta) = self.known.get(&parent_hash).cloned() else {
            self.insert_orphan(block)?;
            return Ok(ConnectResult::Orphaned { hash });
        };

        let height = parent_meta.height + 1;
        self.validate_block_for_parent(
            &block,
            Some(&parent_meta.block),
            Some(parent_hash),
            height,
        )?;

        let block_work = block_work(block.header.bits)?;
        let candidate_work = parent_meta.work.add(block_work);

        let tip_hash = self.tip_hash();
        let extends_tip = parent_hash == tip_hash;

        if extends_tip {
            let undo = self.apply_block_transactions(&block)?;
            self.store.put_block(&block)?;
            self.known.insert(
                hash,
                BlockMeta {
                    block: block.clone(),
                    height,
                    work: candidate_work,
                    undo,
                },
            );
            self.blocks.push(block);
            self.active_hashes.insert(hash);
            self.total_work = candidate_work;
            self.events.push(ChainEvent::BlockConnected {
                height,
                hash,
                tx_count: self
                    .blocks
                    .last()
                    .map(|b| b.transactions.len())
                    .unwrap_or(0),
            });
            if process_orphans {
                self.process_orphans()?;
            }
            return Ok(ConnectResult::Connected { height, hash });
        }

        if candidate_work <= self.total_work {
            self.known.insert(
                hash,
                BlockMeta {
                    block,
                    height,
                    work: candidate_work,
                    undo: Vec::new(),
                },
            );
            return Ok(ConnectResult::SideChain { height, hash });
        }

        let old_tip = tip_hash;
        let depth = self.reorg_to_block(block, candidate_work)?;
        let new_tip = self.tip_hash();
        self.events.push(ChainEvent::ChainReorg {
            depth,
            old_tip,
            new_tip,
        });

        if process_orphans {
            self.process_orphans()?;
        }

        Ok(ConnectResult::Reorganized { old_tip, new_tip })
    }

    fn insert_orphan(&mut self, block: Block) -> Result<(), ChainError> {
        if self.orphans.len() >= MAX_ORPHAN_BLOCKS {
            self.evict_oldest_orphan();
        }

        let hash = block.hash();
        self.orphan_receive_seq = self.orphan_receive_seq.wrapping_add(1);
        self.orphans.insert(
            hash,
            OrphanEntry {
                block,
                received_at: self.orphan_receive_seq,
            },
        );
        self.events.push(ChainEvent::OrphanAdded {
            hash,
            pool_size: self.orphans.len(),
        });
        Ok(())
    }

    fn evict_oldest_orphan(&mut self) {
        let oldest = self
            .orphans
            .iter()
            .min_by_key(|(_, entry)| entry.received_at)
            .map(|(hash, _)| *hash);

        if let Some(hash) = oldest {
            self.orphans.remove(&hash);
            self.events.push(ChainEvent::OrphanEvicted {
                hash,
                reason: EvictionReason::PoolFull,
            });
        }
    }

    /// Reorganizes to a heavier fork; snapshots active state and restores on failure.
    fn reorg_to_block(
        &mut self,
        new_block: Block,
        new_tip_work: ChainWork,
    ) -> Result<u32, ChainError> {
        let (fork_path, fork_hash) = self.collect_fork_path(new_block)?;

        let blocks_snapshot = self.blocks.clone();
        let utxo_snapshot = self.utxo.clone();
        let work_snapshot = self.total_work;
        let active_hashes_snapshot = self.active_hashes.clone();

        let result = self.try_reorg_to_block(fork_path, fork_hash, new_tip_work);
        if result.is_err() {
            self.blocks = blocks_snapshot;
            self.utxo = utxo_snapshot;
            self.total_work = work_snapshot;
            self.active_hashes = active_hashes_snapshot;
        }

        result
    }

    fn try_reorg_to_block(
        &mut self,
        fork_path: Vec<Block>,
        fork_hash: [u8; 32],
        new_tip_work: ChainWork,
    ) -> Result<u32, ChainError> {
        let _ = self
            .find_height(&fork_hash)
            .ok_or(ChainError::MissingAncestor)?;
        let mut disconnected = 0u32;

        while self.tip_hash() != fork_hash {
            self.disconnect_tip_block()?;
            disconnected += 1;
        }

        for block in fork_path {
            let hash = block.hash();
            let height = self.height() + 1;
            let parent = self.blocks.last();
            let parent_hash = block.header.prev_blockhash;
            self.validate_block_for_parent(&block, parent, Some(parent_hash), height)?;
            let undo = self.apply_block_transactions(&block)?;
            let parent_work = self
                .known
                .get(&block.header.prev_blockhash)
                .map(|meta| meta.work)
                .unwrap_or_default();
            let work = parent_work.add(block_work(block.header.bits)?);

            self.store.put_block(&block)?;
            self.known.insert(
                hash,
                BlockMeta {
                    block: block.clone(),
                    height,
                    work,
                    undo,
                },
            );
            self.blocks.push(block);
            self.active_hashes.insert(hash);
            self.events.push(ChainEvent::BlockConnected {
                height,
                hash,
                tx_count: self
                    .blocks
                    .last()
                    .map(|b| b.transactions.len())
                    .unwrap_or(0),
            });
        }

        self.total_work = new_tip_work;
        Ok(disconnected)
    }

    fn collect_fork_path(&self, block: Block) -> Result<(Vec<Block>, [u8; 32]), ChainError> {
        let mut path = vec![block];

        while let Some(tip) = path.last() {
            let parent_hash = tip.header.prev_blockhash;
            if self.active_hashes.contains(&parent_hash) {
                break;
            }
            let parent = self
                .known
                .get(&parent_hash)
                .ok_or(ChainError::MissingAncestor)?
                .block
                .clone();
            path.push(parent);
        }

        let fork_hash = path
            .last()
            .ok_or(ChainError::MissingAncestor)?
            .header
            .prev_blockhash;
        path.reverse();
        Ok((path, fork_hash))
    }

    fn disconnect_tip_block(&mut self) -> Result<(), ChainError> {
        let block = self.blocks.pop().ok_or(ChainError::NoActiveTip)?;
        let hash = block.hash();
        self.active_hashes.remove(&hash);
        let height = self.height().saturating_add(1);
        let meta = self
            .known
            .get(&hash)
            .ok_or(ChainError::MissingAncestor)?
            .clone();

        for undo in meta.undo.iter().rev() {
            self.utxo.disconnect_undo(undo);
        }

        self.total_work = self
            .blocks
            .last()
            .and_then(|b| self.known.get(&b.hash()).map(|m| m.work))
            .unwrap_or_default();

        self.events
            .push(ChainEvent::BlockDisconnected { height, hash });
        Ok(())
    }

    fn process_orphans(&mut self) -> Result<(), ChainError> {
        let mut queue = VecDeque::new();
        self.enqueue_ready_orphans(&mut queue);

        while let Some(block) = queue.pop_front() {
            self.connect_block_inner(block, false)?;
            self.enqueue_ready_orphans(&mut queue);
        }

        Ok(())
    }

    fn enqueue_ready_orphans(&mut self, queue: &mut VecDeque<Block>) {
        let ready: Vec<[u8; 32]> = self
            .orphans
            .iter()
            .filter(|(_, entry)| self.known.contains_key(&entry.block.header.prev_blockhash))
            .map(|(hash, _)| *hash)
            .collect();

        for hash in ready {
            if let Some(entry) = self.orphans.remove(&hash) {
                queue.push_back(entry.block);
            }
        }
    }

    fn validate_block_limits(&self, block: &Block) -> Result<(), ChainError> {
        if block.serialized_size() > MAX_BLOCK_SERIALIZED_SIZE {
            return Err(ChainError::BlockTooLarge);
        }

        if block.transactions.len() > MAX_TRANSACTIONS_PER_BLOCK {
            return Err(ChainError::TooManyTransactions);
        }

        for tx in &block.transactions {
            for input in &tx.inputs {
                if input.script_sig.len() > MAX_SCRIPT_SIZE {
                    return Err(ChainError::ScriptTooLarge);
                }
            }
            for output in &tx.outputs {
                if output.script_pubkey.len() > MAX_SCRIPT_SIZE {
                    return Err(ChainError::ScriptTooLarge);
                }
            }
        }

        Ok(())
    }

    fn validate_block_for_parent(
        &self,
        block: &Block,
        parent: Option<&Block>,
        parent_hash: Option<[u8; 32]>,
        height: u32,
    ) -> Result<(), ChainError> {
        self.validate_block_header_for_parent(block, parent, parent_hash, height)?;
        self.validate_block_transactions_for_utxo(block, &self.utxo)
    }

    fn validate_block_header_for_parent(
        &self,
        block: &Block,
        parent: Option<&Block>,
        parent_hash: Option<[u8; 32]>,
        height: u32,
    ) -> Result<(), ChainError> {
        let target = Target::from_bits(block.header.bits).ok_or(ChainError::InvalidProofOfWork)?;
        if !target.meets(&block.hash()) {
            return Err(ChainError::InvalidProofOfWork);
        }

        if !block.header_merkle_root_matches() {
            return Err(ChainError::MerkleRootMismatch);
        }

        let coinbase_count = block
            .transactions
            .iter()
            .filter(|tx| UtxoSet::is_coinbase(tx))
            .count();
        if coinbase_count != 1 || !UtxoSet::is_coinbase(&block.transactions[0]) {
            return Err(ChainError::InvalidCoinbaseCount {
                count: coinbase_count,
            });
        }

        if let (Some(_parent_block), Some(parent_hash)) = (parent, parent_hash) {
            let median_past = self.median_past_time_before(parent_hash);
            if !valid_block_time(block.header.time, median_past, self.network_time) {
                return Err(ChainError::InvalidTimestamp {
                    block_time: block.header.time,
                    median_past,
                    network_time: self.network_time,
                });
            }
        }

        let expected_bits = self.expected_bits(height, block.header.bits)?;
        if block.header.bits != expected_bits {
            return Err(ChainError::UnexpectedBits {
                height,
                expected: expected_bits,
                actual: block.header.bits,
            });
        }

        Ok(())
    }

    fn validate_block_transactions_for_utxo(
        &self,
        block: &Block,
        utxo: &UtxoSet,
    ) -> Result<(), ChainError> {
        let mut view = utxo.clone();
        let mut spent = HashSet::new();

        for tx in &block.transactions {
            if !UtxoSet::is_coinbase(tx) {
                for input in &tx.inputs {
                    let key = (input.previous_output, input.index);
                    if !spent.insert(key) {
                        return Err(ChainError::Utxo(UtxoError::DuplicateSpend {
                            txid: input.previous_output,
                            index: input.index,
                        }));
                    }
                }
            }

            view.validate_transaction(tx)?;

            if !UtxoSet::is_coinbase(tx) {
                let mut prev_scripts = Vec::with_capacity(tx.inputs.len());
                for input in &tx.inputs {
                    let outpoint = OutPoint {
                        txid: input.previous_output,
                        index: input.index,
                    };
                    let entry =
                        view.get(&outpoint)
                            .ok_or(ChainError::Utxo(UtxoError::MissingInput {
                                txid: outpoint.txid,
                                index: outpoint.index,
                            }))?;
                    prev_scripts.push(entry.script_pubkey.clone());
                }

                for (input_index, input) in tx.inputs.iter().enumerate() {
                    let script_pubkey = &prev_scripts[input_index];
                    if script_pubkey.is_empty() {
                        continue;
                    }

                    let sighash = sighash_all(tx, input_index, &prev_scripts)?;
                    bitrst_script::verify_script(&input.script_sig, script_pubkey, &sighash)
                        .map_err(|_| ChainError::InvalidScript)?;
                }
            }

            view.apply_transaction(tx);
        }

        Ok(())
    }

    fn apply_block_transactions(&mut self, block: &Block) -> Result<Vec<TxUndo>, ChainError> {
        let mut undos = Vec::with_capacity(block.transactions.len());
        for tx in &block.transactions {
            undos.push(self.utxo.apply_transaction(tx));
        }
        Ok(undos)
    }

    fn median_past_time_before(&self, mut block_hash: [u8; 32]) -> u32 {
        let mut times = Vec::new();
        for _ in 0..11 {
            let Some(meta) = self.known.get(&block_hash) else {
                break;
            };
            times.push(meta.block.header.time);
            block_hash = meta.block.header.prev_blockhash;
        }

        if times.is_empty() {
            return 0;
        }

        times.sort_unstable();
        times[times.len() / 2]
    }

    fn expected_bits(&self, height: u32, block_bits: u32) -> Result<u32, ChainError> {
        if height == 0 {
            return Ok(block_bits);
        }

        if !height.is_multiple_of(difficulty_adjustment_interval()) {
            let Some(tip) = self.blocks.last() else {
                return Err(ChainError::NoActiveTip);
            };
            return Ok(tip.header.bits);
        }

        debug_assert_eq!(
            self.blocks.len(),
            height as usize,
            "expected_bits must be called before the block is pushed"
        );

        let period_start = height.saturating_sub(difficulty_adjustment_interval()) as usize;
        let period_end = height as usize;
        if period_start >= self.blocks.len() {
            return Ok(MAX_COMPACT_BITS);
        }

        let start_time = self.blocks[period_start].header.time;
        let end_time = self.blocks[period_end - 1].header.time;
        let actual_timespan = end_time.saturating_sub(start_time);
        let prev_bits = self.blocks[period_end - 1].header.bits;

        Ok(adjust_bits(prev_bits, actual_timespan)?)
    }

    fn find_height(&self, hash: &[u8; 32]) -> Option<u32> {
        self.known.get(hash).map(|m| m.height)
    }
}

/// Per-block work from compact `bits` (Bitcoin Core `GetBlockProof`).
pub fn block_work(bits: u32) -> Result<ChainWork, ChainError> {
    let target = Target::from_bits(bits).ok_or(ChainError::InvalidProofOfWork)?;
    let work = target.to_work().ok_or(ChainError::InvalidProofOfWork)?;
    Ok(ChainWork(work))
}

#[cfg(test)]
mod tests {
    use super::{block_work, Chain, ChainError, ConnectResult};
    use crate::pow::Target;
    use crate::{Block, BlockHeader, Transaction};

    fn mine_header(header: &mut BlockHeader, target: Target) {
        let mut attempts = 0u64;
        loop {
            if target.meets(&header.hash()) {
                return;
            }
            header.nonce = header.nonce.wrapping_add(1);
            attempts += 1;
            if attempts > 10_000_000 {
                panic!("test mining exceeded attempt limit");
            }
        }
    }

    fn genesis_block() -> Block {
        let bits = 0x1f00_ffff_u32;
        let header = BlockHeader {
            version: 1,
            prev_blockhash: [0u8; 32],
            merkle_root: [0u8; 32],
            time: 1231006505,
            bits,
            nonce: 0,
        };
        let mut block = Block::coinbase(header, 0, 50_0000_0000);
        let target = Target::from_bits(bits).expect("test genesis bits should decode");
        mine_header(&mut block.header, target);
        block
    }

    #[test]
    fn cumulative_work_adds_for_same_bits() {
        let bits = 0x1f00_ffff_u32;
        let one = block_work(bits).expect("work");
        let two = one.add(one);
        assert!(two > one);
    }

    #[test]
    fn connects_genesis_block() {
        let genesis = genesis_block();
        let chain = Chain::new_genesis(genesis, 1231006505).expect("genesis should connect");

        assert_eq!(chain.height(), 0);
        assert_eq!(chain.utxo().len(), 1);
    }

    #[test]
    fn rejects_block_with_bad_proof_of_work() {
        let genesis = genesis_block();
        let mut chain = Chain::new_genesis(genesis.clone(), 1231006505).expect("genesis ok");
        let before_tip = chain.tip_hash();
        let before_height = chain.height();
        let before_utxo = chain.utxo().len();

        let mut bad_header = genesis.header.clone();
        bad_header.prev_blockhash = genesis.hash();
        bad_header.bits = 0;
        bad_header.nonce = 0;
        let bad = Block::coinbase(bad_header, 1, 50_0000_0000);

        assert!(matches!(
            chain.connect_block(bad),
            Err(ChainError::InvalidProofOfWork)
        ));
        assert_eq!(chain.tip_hash(), before_tip);
        assert_eq!(chain.height(), before_height);
        assert_eq!(chain.utxo().len(), before_utxo);
    }

    #[test]
    fn rejects_block_with_mismatched_merkle_root() {
        let genesis = genesis_block();
        let mut chain = Chain::new_genesis(genesis.clone(), 1231006505).expect("genesis ok");
        let before_tip = chain.tip_hash();
        let before_height = chain.height();
        let before_utxo = chain.utxo().len();

        let header = BlockHeader {
            version: 1,
            prev_blockhash: genesis.hash(),
            merkle_root: [0x01; 32],
            time: 1231006600,
            bits: 0x1f00_ffff,
            nonce: 0,
        };
        let mut bad = Block::new(header, vec![Transaction::coinbase(1, 50_0000_0000)]);
        let bad_target = Target::from_bits(bad.header.bits).expect("test bits should decode");
        mine_header(&mut bad.header, bad_target);
        assert!(matches!(
            chain.connect_block(bad),
            Err(ChainError::MerkleRootMismatch)
        ));
        assert_eq!(chain.tip_hash(), before_tip);
        assert_eq!(chain.height(), before_height);
        assert_eq!(chain.utxo().len(), before_utxo);
    }

    #[test]
    fn stores_orphan_when_parent_missing() {
        let genesis = genesis_block();
        let mut chain = Chain::new_genesis(genesis, 1231006505).expect("genesis ok");

        let header = BlockHeader {
            version: 1,
            prev_blockhash: [0x01; 32],
            merkle_root: [0u8; 32],
            time: 1231006600,
            bits: 0x1f00_ffff,
            nonce: 0,
        };
        let orphan = Block::coinbase(header, 1, 50_0000_0000);

        let result = chain
            .connect_block(orphan)
            .expect("orphan storage should succeed");
        assert!(matches!(result, ConnectResult::Orphaned { .. }));
        assert_eq!(chain.height(), 0);
    }

    #[test]
    fn promotes_orphan_after_parent_connects() {
        let genesis = genesis_block();
        let genesis_hash = genesis.hash();
        let mut chain = Chain::new_genesis(genesis, 1231006505).expect("genesis ok");

        let parent_header = BlockHeader {
            version: 1,
            prev_blockhash: genesis_hash,
            merkle_root: [0u8; 32],
            time: 1231006550,
            bits: 0x1f00_ffff,
            nonce: 0,
        };
        let mut parent = Block::coinbase(parent_header, 1, 50_0000_0000);
        parent.header.merkle_root = parent.merkle_root().expect("merkle");
        let parent_target = Target::from_bits(parent.header.bits).expect("test bits should decode");
        mine_header(&mut parent.header, parent_target);
        let parent_hash = parent.hash();

        let child_header = BlockHeader {
            version: 1,
            prev_blockhash: parent_hash,
            merkle_root: [0u8; 32],
            time: 1231006600,
            bits: 0x1f00_ffff,
            nonce: 0,
        };
        let mut child = Block::coinbase(child_header, 2, 50_0000_0000);
        child.header.merkle_root = child.merkle_root().expect("merkle");
        let child_target = Target::from_bits(child.header.bits).expect("test bits should decode");
        mine_header(&mut child.header, child_target);
        assert!(matches!(
            chain.connect_block(child.clone()).expect("store orphan"),
            ConnectResult::Orphaned { .. }
        ));

        assert!(matches!(
            chain.connect_block(parent).expect("parent should connect"),
            ConnectResult::Connected { height: 1, .. }
        ));
        assert_eq!(chain.height(), 2);
        assert_eq!(chain.utxo().len(), 3);
    }

    #[test]
    fn connects_mined_block_one() {
        let genesis = genesis_block();
        let genesis_hash = genesis.hash();
        let mut chain = Chain::new_genesis(genesis, 1231006505).expect("genesis ok");

        let header = BlockHeader {
            version: 1,
            prev_blockhash: genesis_hash,
            merkle_root: [0u8; 32],
            time: 1231006600,
            bits: 0x1f00_ffff,
            nonce: 0,
        };
        let block_one = Block::coinbase(header, 1, 50_0000_0000);
        let mut block_one = block_one;
        block_one.header.merkle_root = block_one.merkle_root().expect("merkle root should exist");
        let block_target =
            Target::from_bits(block_one.header.bits).expect("test bits should decode");
        mine_header(&mut block_one.header, block_target);

        let result = chain
            .connect_block(block_one)
            .expect("block one should connect");
        assert!(matches!(result, ConnectResult::Connected { height: 1, .. }));
        assert_eq!(chain.utxo().len(), 2);
    }

    #[test]
    fn reorgs_to_heavier_fork() {
        let genesis = genesis_block();
        let genesis_hash = genesis.hash();
        let mut chain = Chain::new_genesis(genesis, 1231006505).expect("genesis ok");

        let fork_a_header = BlockHeader {
            version: 1,
            prev_blockhash: genesis_hash,
            merkle_root: [0u8; 32],
            time: 1231006600,
            bits: 0x1f00_ffff,
            nonce: 0,
        };
        let mut fork_a = Block::coinbase(fork_a_header, 1, 50_0000_0000);
        fork_a.header.merkle_root = fork_a.merkle_root().expect("merkle");
        let fork_a_target = Target::from_bits(fork_a.header.bits).expect("test bits should decode");
        mine_header(&mut fork_a.header, fork_a_target);
        let fork_a_hash = fork_a.hash();
        chain.connect_block(fork_a).expect("connect fork a");
        assert_eq!(chain.height(), 1);

        let fork_b1_header = BlockHeader {
            version: 1,
            prev_blockhash: genesis_hash,
            merkle_root: [0u8; 32],
            time: 1231006700,
            bits: 0x1f00_ffff,
            nonce: 0,
        };
        let mut fork_b1 = Block::coinbase(fork_b1_header, 1, 50_0000_0000);
        fork_b1.header.merkle_root = fork_b1.merkle_root().expect("merkle");
        let fork_b1_target =
            Target::from_bits(fork_b1.header.bits).expect("test bits should decode");
        mine_header(&mut fork_b1.header, fork_b1_target);
        let fork_b1_hash = fork_b1.hash();

        assert!(matches!(
            chain.connect_block(fork_b1).expect("store competing fork"),
            ConnectResult::SideChain { height: 1, .. }
        ));
        assert_eq!(chain.tip_hash(), fork_a_hash);

        let fork_b2_header = BlockHeader {
            version: 1,
            prev_blockhash: fork_b1_hash,
            merkle_root: [0u8; 32],
            time: 1231006800,
            bits: 0x1f00_ffff,
            nonce: 0,
        };
        let mut fork_b2 = Block::coinbase(fork_b2_header, 2, 50_0000_0000);
        fork_b2.header.merkle_root = fork_b2.merkle_root().expect("merkle");
        let fork_b2_target =
            Target::from_bits(fork_b2.header.bits).expect("test bits should decode");
        mine_header(&mut fork_b2.header, fork_b2_target);
        let fork_b2_hash = fork_b2.hash();

        let result = chain
            .connect_block(fork_b2)
            .expect("longer fork should trigger reorg");
        assert!(matches!(result, ConnectResult::Reorganized { .. }));
        assert_eq!(chain.height(), 2);
        assert_eq!(chain.tip_hash(), fork_b2_hash);
        assert_eq!(chain.utxo().len(), 3);
    }

    #[test]
    fn rejects_zero_network_time_at_genesis() {
        let genesis = genesis_block();
        assert!(matches!(
            Chain::new_genesis(genesis, 0),
            Err(ChainError::InvalidNetworkTime)
        ));
    }

    #[test]
    fn promotes_deep_orphan_chain_iteratively() {
        let genesis = genesis_block();
        let genesis_hash = genesis.hash();
        let mut chain = Chain::new_genesis(genesis, 1231006505).expect("genesis ok");

        let mut blocks = Vec::new();
        let mut prev_hash = genesis_hash;

        for height in 1..=12 {
            let header = BlockHeader {
                version: 1,
                prev_blockhash: prev_hash,
                merkle_root: [0u8; 32],
                time: 1231006505 + height,
                bits: 0x1f00_ffff,
                nonce: 0,
            };
            let mut block = Block::coinbase(header, height, 50_0000_0000);
            block.header.merkle_root = block.merkle_root().expect("merkle");
            let target = Target::from_bits(block.header.bits).expect("bits");
            mine_header(&mut block.header, target);
            prev_hash = block.hash();
            blocks.push(block);
        }

        for block in blocks.iter().skip(1) {
            assert!(matches!(
                chain.connect_block(block.clone()).expect("store orphan"),
                ConnectResult::Orphaned { .. }
            ));
        }
        assert_eq!(chain.height(), 0);

        assert!(matches!(
            chain
                .connect_block(blocks[0].clone())
                .expect("connect first block"),
            ConnectResult::Connected { height: 1, .. }
        ));
        assert_eq!(chain.height(), 12);
        assert_eq!(chain.utxo().len(), 13);
    }

    #[test]
    fn connects_block_with_valid_p2pkh_spend() {
        use bitrst_crypto::hash160::hash160;
        use bitrst_script::{p2pkh_script_pubkey, p2pkh_script_sig};
        use secp256k1::{Message, Secp256k1, SecretKey};

        use crate::sighash::sighash_all;
        use crate::{Transaction, TxInput, TxOutput};

        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[0x33; 32]).expect("secret key");
        let pk = secp256k1::PublicKey::from_secret_key(&secp, &sk);
        let pubkey_bytes = pk.serialize();
        let lock_script = p2pkh_script_pubkey(&hash160(&pubkey_bytes));

        let genesis = genesis_block();
        let genesis_hash = genesis.hash();
        let mut chain = Chain::new_genesis(genesis, 1231006505).expect("genesis");

        let header1 = BlockHeader {
            version: 1,
            prev_blockhash: genesis_hash,
            merkle_root: [0u8; 32],
            time: 1231006600,
            bits: 0x1f00_ffff,
            nonce: 0,
        };
        let mut block1 = Block::coinbase(header1, 1, 50_0000_0000);
        block1.transactions[0].outputs[0].script_pubkey = lock_script.clone();
        block1.header.merkle_root = block1.merkle_root().expect("merkle");
        let block1_target = Target::from_bits(block1.header.bits).expect("bits");
        mine_header(&mut block1.header, block1_target);
        chain.connect_block(block1.clone()).expect("fund p2pkh");
        let funding_txid = block1.transactions[0].txid();

        let mut spend_tx = Transaction {
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
        let sighash =
            sighash_all(&spend_tx, 0, std::slice::from_ref(&lock_script)).expect("sighash");
        let sig = secp.sign_ecdsa(&Message::from_digest(sighash), &sk);
        let mut sig_bytes = sig.serialize_der().to_vec();
        sig_bytes.push(0x01);
        spend_tx.inputs[0].script_sig = p2pkh_script_sig(&sig_bytes, &pubkey_bytes);

        let header2 = BlockHeader {
            version: 1,
            prev_blockhash: block1.hash(),
            merkle_root: [0u8; 32],
            time: 1231006700,
            bits: 0x1f00_ffff,
            nonce: 0,
        };
        let mut block2 = Block::coinbase(header2, 2, 50_0000_0000);
        block2.transactions.push(spend_tx);
        block2.header.merkle_root = block2.merkle_root().expect("merkle");
        let block2_target = Target::from_bits(block2.header.bits).expect("bits");
        mine_header(&mut block2.header, block2_target);

        chain.connect_block(block2).expect("spend p2pkh");
        assert_eq!(chain.height(), 2);
        assert_eq!(chain.utxo().len(), 3);
    }

    #[test]
    fn rejects_block_with_invalid_p2pkh_script_sig() {
        use bitrst_crypto::hash160::hash160;
        use bitrst_script::p2pkh_script_pubkey;
        use secp256k1::{Secp256k1, SecretKey};

        use crate::{Transaction, TxInput, TxOutput};

        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[0x44; 32]).expect("secret key");
        let pk = secp256k1::PublicKey::from_secret_key(&secp, &sk);
        let pubkey_bytes = pk.serialize();
        let lock_script = p2pkh_script_pubkey(&hash160(&pubkey_bytes));

        let genesis = genesis_block();
        let genesis_hash = genesis.hash();
        let mut chain = Chain::new_genesis(genesis, 1231006505).expect("genesis");

        let header1 = BlockHeader {
            version: 1,
            prev_blockhash: genesis_hash,
            merkle_root: [0u8; 32],
            time: 1231006600,
            bits: 0x1f00_ffff,
            nonce: 0,
        };
        let mut block1 = Block::coinbase(header1, 1, 50_0000_0000);
        block1.transactions[0].outputs[0].script_pubkey = lock_script;
        block1.header.merkle_root = block1.merkle_root().expect("merkle");
        let block1_target = Target::from_bits(block1.header.bits).expect("bits");
        mine_header(&mut block1.header, block1_target);
        chain.connect_block(block1.clone()).expect("fund p2pkh");
        let funding_txid = block1.transactions[0].txid();

        let spend_tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_output: funding_txid,
                index: 0,
                script_sig: vec![0x00],
                sequence: u32::MAX,
            }],
            outputs: vec![TxOutput {
                value: 49_0000_0000,
                script_pubkey: Vec::new(),
            }],
            lock_time: 0,
        };

        let header2 = BlockHeader {
            version: 1,
            prev_blockhash: block1.hash(),
            merkle_root: [0u8; 32],
            time: 1231006700,
            bits: 0x1f00_ffff,
            nonce: 0,
        };
        let mut block2 = Block::coinbase(header2, 2, 50_0000_0000);
        block2.transactions.push(spend_tx);
        block2.header.merkle_root = block2.merkle_root().expect("merkle");
        let block2_target = Target::from_bits(block2.header.bits).expect("bits");
        mine_header(&mut block2.header, block2_target);

        let before_height = chain.height();
        let before_tip = chain.tip_hash();
        let before_utxo = chain.utxo().len();
        assert!(matches!(
            chain.connect_block(block2),
            Err(ChainError::InvalidScript)
        ));
        assert_eq!(chain.height(), before_height);
        assert_eq!(chain.tip_hash(), before_tip);
        assert_eq!(chain.utxo().len(), before_utxo);
    }
}
