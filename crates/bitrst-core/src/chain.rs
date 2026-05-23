//! Block chain validation, orphan handling, and reorg logic.
//!
//! A [`Chain`] tracks the active proof-of-work chain, the UTXO set for that chain,
//! and orphan blocks waiting for unknown parents. The active tip is chosen by
//! cumulative proof-of-work, not block height alone.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use thiserror::Error;

use crate::block::Block;
use crate::difficulty::{
    adjust_bits, DifficultyError, DIFFICULTY_ADJUSTMENT_INTERVAL, MAX_COMPACT_BITS,
};
use crate::pow::Target;
use crate::time::valid_block_time;
use crate::utxo::{TxUndo, UtxoError, UtxoSet};

/// Total cumulative proof-of-work on a chain branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChainWork(u128);

impl ChainWork {
    fn add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
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
}

#[derive(Debug, Clone)]
struct BlockMeta {
    block: Block,
    height: u32,
    work: ChainWork,
    undo: Vec<TxUndo>,
}

/// In-memory block chain with UTXO state and orphan handling.
#[derive(Debug)]
pub struct Chain {
    /// Active chain blocks from genesis (height 0) to tip.
    blocks: Vec<Block>,
    /// All blocks this node knows, including side-chain headers without UTXO application.
    known: HashMap<[u8; 32], BlockMeta>,
    /// Active chain cumulative proof-of-work.
    total_work: ChainWork,
    /// Current UTXO set for the active chain.
    utxo: UtxoSet,
    /// Blocks waiting for a parent to arrive first.
    orphans: HashMap<[u8; 32], Block>,
    /// Network-adjusted time used for future-drift checks.
    network_time: u32,
}

impl Chain {
    /// Creates a new chain with a valid genesis block at height 0.
    ///
    /// # Errors
    ///
    /// Returns [`ChainError`] when the genesis block fails validation.
    pub fn new_genesis(genesis: Block, network_time: u32) -> Result<Self, ChainError> {
        let mut chain = Self {
            blocks: Vec::new(),
            known: HashMap::new(),
            total_work: ChainWork::default(),
            utxo: UtxoSet::new(),
            orphans: HashMap::new(),
            network_time,
        };

        let hash = genesis.hash();
        chain.validate_block_for_parent(&genesis, None, 0)?;
        let undo = chain.apply_block_transactions(&genesis)?;
        let work = block_work(genesis.header.bits);

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
        chain.total_work = work;

        Ok(chain)
    }

    /// Returns the active chain height (genesis is height 0).
    pub fn height(&self) -> u32 {
        self.blocks.len().saturating_sub(1) as u32
    }

    /// Returns the hash of the active chain tip.
    pub fn tip_hash(&self) -> [u8; 32] {
        self.blocks.last().map(Block::hash).unwrap_or([0u8; 32])
    }

    /// Returns a reference to the active UTXO set.
    pub fn utxo(&self) -> &UtxoSet {
        &self.utxo
    }

    /// Attempts to connect a block to the chain.
    ///
    /// # Errors
    ///
    /// Returns [`ChainError`] when the block fails validation. Returns
    /// [`ConnectResult::Orphaned`] without error when the parent is unknown.
    pub fn connect_block(&mut self, block: Block) -> Result<ConnectResult, ChainError> {
        let hash = block.hash();
        if self.known.contains_key(&hash) {
            return Err(ChainError::BlockAlreadyKnown);
        }

        let parent_hash = block.header.prev_blockhash;
        let Some(parent_meta) = self.known.get(&parent_hash).cloned() else {
            self.orphans.insert(hash, block);
            return Ok(ConnectResult::Orphaned { hash });
        };

        let height = parent_meta.height + 1;
        self.validate_block_for_parent(&block, Some(&parent_meta.block), height)?;

        let block_work = block_work(block.header.bits);
        let candidate_work = parent_meta.work.add(block_work);

        let tip_hash = self.tip_hash();
        let extends_tip = parent_hash == tip_hash;

        if extends_tip {
            let undo = self.apply_block_transactions(&block)?;
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
            self.total_work = candidate_work;
            self.process_orphans()?;
            return Ok(ConnectResult::Connected { height, hash });
        }

        if candidate_work.cmp(&self.total_work) != Ordering::Greater {
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
        self.reorg_to_block(block, parent_meta, candidate_work)?;
        let new_tip = self.tip_hash();

        Ok(ConnectResult::Reorganized { old_tip, new_tip })
    }

    fn reorg_to_block(
        &mut self,
        new_block: Block,
        _parent_meta: BlockMeta,
        new_tip_work: ChainWork,
    ) -> Result<(), ChainError> {
        let (fork_path, fork_hash) = self.collect_fork_path(new_block);

        while self.tip_hash() != fork_hash {
            self.disconnect_tip_block()?;
        }

        for block in fork_path {
            let hash = block.hash();
            let height = self.height() + 1;
            let parent = self.blocks.last();
            self.validate_block_for_parent(&block, parent, height)?;
            let undo = self.apply_block_transactions(&block)?;
            let parent_work = self
                .known
                .get(&block.header.prev_blockhash)
                .map(|meta| meta.work)
                .unwrap_or_default();
            let work = parent_work.add(block_work(block.header.bits));

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
        }

        self.total_work = new_tip_work;
        self.process_orphans()?;
        Ok(())
    }

    fn collect_fork_path(&self, block: Block) -> (Vec<Block>, [u8; 32]) {
        let mut path = vec![block];
        let active_hashes: HashSet<[u8; 32]> = self.blocks.iter().map(Block::hash).collect();

        while !active_hashes.contains(&path.last().expect("path not empty").header.prev_blockhash) {
            let parent_hash = path.last().expect("path not empty").header.prev_blockhash;
            let parent = self
                .known
                .get(&parent_hash)
                .expect("fork parent must be known");
            path.push(parent.block.clone());
        }

        let fork_hash = path.last().expect("path not empty").header.prev_blockhash;
        path.reverse();
        (path, fork_hash)
    }

    fn disconnect_tip_block(&mut self) -> Result<(), ChainError> {
        let block = self.blocks.pop().expect("tip block must exist");
        let hash = block.hash();
        let meta = self
            .known
            .get(&hash)
            .cloned()
            .expect("known block must exist");

        for undo in meta.undo.iter().rev() {
            self.utxo.disconnect_undo(undo);
        }

        self.total_work = self
            .blocks
            .last()
            .and_then(|b| self.known.get(&b.hash()).map(|m| m.work))
            .unwrap_or_default();

        Ok(())
    }

    fn process_orphans(&mut self) -> Result<(), ChainError> {
        loop {
            let ready: Vec<[u8; 32]> = self
                .orphans
                .iter()
                .filter(|(_, block)| self.known.contains_key(&block.header.prev_blockhash))
                .map(|(hash, _)| *hash)
                .collect();

            if ready.is_empty() {
                break;
            }

            for hash in ready {
                let block = self.orphans.remove(&hash).expect("orphan must exist");
                let _ = self.connect_block(block)?;
            }
        }
        Ok(())
    }

    fn validate_block_for_parent(
        &self,
        block: &Block,
        parent: Option<&Block>,
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

        if let Some(parent_block) = parent {
            let median_past = self.median_past_time();
            if !valid_block_time(block.header.time, median_past, self.network_time) {
                return Err(ChainError::InvalidTimestamp {
                    block_time: block.header.time,
                    median_past,
                    network_time: self.network_time,
                });
            }

            let _parent = parent_block;
        }

        let expected_bits = self.expected_bits(height, block.header.bits)?;
        if block.header.bits != expected_bits {
            return Err(ChainError::UnexpectedBits {
                height,
                expected: expected_bits,
                actual: block.header.bits,
            });
        }

        let mut spent = HashSet::new();
        for tx in &block.transactions {
            self.utxo.validate_transaction(tx)?;
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

    fn median_past_time(&self) -> u32 {
        let count = self.blocks.len();
        if count == 0 {
            return 0;
        }

        let start = count.saturating_sub(11);
        let mut times: Vec<u32> = self.blocks[start..]
            .iter()
            .map(|block| block.header.time)
            .collect();
        times.sort_unstable();
        times[times.len() / 2]
    }

    fn expected_bits(&self, height: u32, block_bits: u32) -> Result<u32, ChainError> {
        if height == 0 {
            return Ok(block_bits);
        }

        if !height.is_multiple_of(DIFFICULTY_ADJUSTMENT_INTERVAL) {
            return Ok(self
                .blocks
                .last()
                .expect("non-genesis has parent")
                .header
                .bits);
        }

        let period_start = height.saturating_sub(DIFFICULTY_ADJUSTMENT_INTERVAL) as usize;
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
}

pub(crate) fn block_work(bits: u32) -> ChainWork {
    let threshold = Target::from_bits(bits)
        .unwrap_or_else(Target::easy)
        .threshold();
    // Higher scalar when the target is smaller (more leading zero bytes from the MSB).
    let leading_zeros = threshold.iter().rev().take_while(|&&b| b == 0).count() as u128;
    let remainder = threshold
        .iter()
        .rev()
        .nth(leading_zeros as usize)
        .copied()
        .unwrap_or(0);
    let work = leading_zeros
        .saturating_mul(1_000_000)
        .saturating_add(u128::from(0xff - remainder));
    ChainWork(work.max(1))
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::{block_work, Chain, ChainError, ConnectResult};
    use crate::pow::Target;
    use crate::{Block, BlockHeader, Transaction};

    #[test]
    fn longer_fork_has_more_cumulative_work() {
        let bits = 0x1f00_ffff_u32;
        let one = block_work(bits);
        let two = one.add(one);
        assert_eq!(two.cmp(&one), Ordering::Greater);
    }

    fn mine_header(header: &mut BlockHeader, target: Target) {
        loop {
            if target.meets(&header.hash()) {
                return;
            }
            header.nonce = header.nonce.wrapping_add(1);
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

        let mut bad_header = genesis.header.clone();
        bad_header.prev_blockhash = genesis.hash();
        bad_header.bits = 0;
        bad_header.nonce = 0;
        let bad = Block::coinbase(bad_header, 1, 50_0000_0000);

        assert!(matches!(
            chain.connect_block(bad),
            Err(ChainError::InvalidProofOfWork)
        ));
    }

    #[test]
    fn rejects_block_with_mismatched_merkle_root() {
        let genesis = genesis_block();
        let mut chain = Chain::new_genesis(genesis.clone(), 1231006505).expect("genesis ok");

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
}
