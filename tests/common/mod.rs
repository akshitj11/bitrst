//! Shared helpers for workspace integration tests.

#![allow(dead_code)]

use std::cmp::Ordering;

use bitrst_core::difficulty::{adjust_bits, difficulty_adjustment_interval};
use bitrst_core::limits::{MAX_BLOCK_SERIALIZED_SIZE, MAX_ORPHAN_BLOCKS};
use bitrst_core::{
    Block, BlockHeader, Chain, ChainError, ChainWork, ConnectResult, Target, Transaction, TxInput,
    TxOutput,
};

pub const NETWORK_TIME: u32 = 1231006505;
pub const TEST_BITS: u32 = 0x1f00_ffff;
pub const REWARD: u64 = 50_0000_0000;

#[derive(Clone, Copy)]
pub struct ChainSnapshot {
    pub tip: [u8; 32],
    pub height: u32,
    pub utxo_len: usize,
    pub block_count: usize,
}

pub fn snapshot(chain: &Chain) -> ChainSnapshot {
    ChainSnapshot {
        tip: chain.tip_hash(),
        height: chain.height(),
        utxo_len: chain.utxo().len(),
        block_count: chain.active_block_count(),
    }
}

pub fn assert_chain_invariants(chain: &Chain) {
    assert_eq!(
        chain.active_block_count(),
        chain.height() as usize + 1,
        "active block count must match height"
    );
}

pub fn assert_unchanged_on_err(
    chain: &Chain,
    before: ChainSnapshot,
    result: Result<(), ChainError>,
) {
    assert!(result.is_err());
    assert_eq!(chain.tip_hash(), before.tip, "tip must not change");
    assert_eq!(chain.height(), before.height, "height must not change");
    assert_eq!(chain.utxo().len(), before.utxo_len, "utxo must not change");
    assert_eq!(
        chain.active_block_count(),
        before.block_count,
        "block count must not change"
    );
}

pub fn assert_unchanged_on_connect_err(
    chain: &Chain,
    before: ChainSnapshot,
    result: Result<ConnectResult, ChainError>,
) {
    assert_unchanged_on_err(chain, before, result.map(|_| ()));
}

pub fn mine_header(header: &mut BlockHeader, target: Target) {
    mine_header_with_attempts(header, target, 50_000_000);
}

pub fn mine_header_for_bits(header: &mut BlockHeader, bits: u32) {
    let target = Target::from_bits(bits).expect("valid compact bits for test mining");
    mine_header(header, target);
}

pub fn mine_header_with_attempts(header: &mut BlockHeader, target: Target, attempts: u64) {
    for _ in 0..attempts {
        if target.meets(&header.hash()) {
            return;
        }
        header.nonce = header.nonce.wrapping_add(1);
    }
    panic!("test mining exceeded attempt limit");
}

pub fn genesis_block() -> Block {
    let header = BlockHeader {
        version: 1,
        prev_blockhash: [0u8; 32],
        merkle_root: [0u8; 32],
        time: NETWORK_TIME,
        bits: TEST_BITS,
        nonce: 0,
    };
    let mut block = Block::coinbase(header, 0, REWARD);
    let target = Target::from_bits(TEST_BITS).expect("test bits");
    mine_header(&mut block.header, target);
    block
}

pub fn mine_block_on(parent: &Block, time: u32, height: u32) -> Block {
    let header = BlockHeader {
        version: 1,
        prev_blockhash: parent.hash(),
        merkle_root: [0u8; 32],
        time,
        bits: parent.header.bits,
        nonce: 0,
    };
    let mut block = Block::coinbase(header, height, REWARD);
    block.header.merkle_root = block.merkle_root().expect("merkle");
    let target = Target::from_bits(block.header.bits).expect("test bits");
    mine_header(&mut block.header, target);
    block
}

pub fn mine_block_with_bits(parent: &Block, time: u32, height: u32, bits: u32) -> Block {
    let header = BlockHeader {
        version: 1,
        prev_blockhash: parent.hash(),
        merkle_root: [0u8; 32],
        time,
        bits,
        nonce: 0,
    };
    let mut block = Block::coinbase(header, height, REWARD);
    block.header.merkle_root = block.merkle_root().expect("merkle");
    mine_header_for_bits(&mut block.header, bits);
    block
}

pub fn setup_chain_of_length(total_blocks: u32) -> Chain {
    let genesis = genesis_block();
    let chain = Chain::new_genesis(genesis, NETWORK_TIME).expect("genesis");
    build_chain_of_length(chain, total_blocks)
}

pub fn build_chain_of_length(mut chain: Chain, total_blocks: u32) -> Chain {
    let mut parent = chain.blocks_last_hash();
    let mut time = NETWORK_TIME;
    for height in 1..total_blocks {
        time = time.saturating_add(600);
        let block = mine_block_on_by_hash(parent, time, height);
        parent = block.hash();
        chain.connect_block(block).expect("connect block");
    }
    chain
}

fn mine_block_on_by_hash(prev_hash: [u8; 32], time: u32, height: u32) -> Block {
    let header = BlockHeader {
        version: 1,
        prev_blockhash: prev_hash,
        merkle_root: [0u8; 32],
        time,
        bits: TEST_BITS,
        nonce: 0,
    };
    let mut block = Block::coinbase(header, height, REWARD);
    block.header.merkle_root = block.merkle_root().expect("merkle");
    let target = Target::from_bits(TEST_BITS).expect("test bits");
    mine_header(&mut block.header, target);
    block
}

trait ChainBlocksLast {
    fn blocks_last_hash(&self) -> [u8; 32];
}

impl ChainBlocksLast for Chain {
    fn blocks_last_hash(&self) -> [u8; 32] {
        self.tip_hash()
    }
}

pub fn build_linear_chain_blocks(genesis: &Block, count: u32) -> Vec<Block> {
    let interval = difficulty_adjustment_interval();
    let mut blocks = Vec::with_capacity(count as usize);
    let mut parent = genesis.clone();
    for height in 1..=count {
        let time = NETWORK_TIME.saturating_add(height);
        let bits = if height.is_multiple_of(interval) {
            let timespan = time.saturating_sub(genesis.header.time);
            adjust_bits(parent.header.bits, timespan).expect("adjust bits for test chain")
        } else {
            parent.header.bits
        };
        let mut block = {
            let header = BlockHeader {
                version: 1,
                prev_blockhash: parent.hash(),
                merkle_root: [0u8; 32],
                time,
                bits,
                nonce: 0,
            };
            let mut block = Block::coinbase(header, height, REWARD);
            block.header.merkle_root = block.merkle_root().expect("merkle");
            block
        };
        let target = Target::from_bits(bits).expect("test bits");
        if height.is_multiple_of(interval) {
            mine_header_with_attempts(&mut block.header, target, 200_000_000);
        } else {
            mine_header(&mut block.header, target);
        }
        parent = block.clone();
        blocks.push(block);
    }
    let _ = interval;
    blocks
}

pub fn build_orphan_block(index: usize) -> Block {
    let byte = (index % 256) as u8;
    let header = BlockHeader {
        version: 1,
        prev_blockhash: [byte; 32],
        merkle_root: [0u8; 32],
        time: NETWORK_TIME + index as u32,
        bits: TEST_BITS,
        nonce: index as u32,
    };
    Block::coinbase(header, 1, REWARD)
}

pub fn build_block_with_bad_merkle(parent: &Block, time: u32, height: u32) -> Block {
    let header = BlockHeader {
        version: 1,
        prev_blockhash: parent.hash(),
        merkle_root: [0x01; 32],
        time,
        bits: parent.header.bits,
        nonce: 0,
    };
    let mut block = Block::coinbase(header, height, REWARD);
    let target = Target::from_bits(block.header.bits).expect("test bits");
    mine_header(&mut block.header, target);
    block
}

pub fn build_block_with_double_spend(parent: &Block) -> Block {
    let funding = parent.transactions[0].txid();
    let spend_a = Transaction {
        version: 1,
        inputs: vec![TxInput {
            previous_output: funding,
            index: 0,
            script_sig: vec![],
            sequence: u32::MAX,
        }],
        outputs: vec![TxOutput {
            value: REWARD / 2,
            script_pubkey: vec![],
        }],
        lock_time: 0,
    };
    let spend_b = Transaction {
        version: 1,
        inputs: vec![TxInput {
            previous_output: funding,
            index: 0,
            script_sig: vec![],
            sequence: u32::MAX,
        }],
        outputs: vec![TxOutput {
            value: REWARD / 2,
            script_pubkey: vec![],
        }],
        lock_time: 0,
    };
    let coinbase_tx = Transaction::coinbase(2, REWARD);
    let header = BlockHeader {
        version: 1,
        prev_blockhash: parent.hash(),
        merkle_root: [0u8; 32],
        time: parent.header.time + 600,
        bits: parent.header.bits,
        nonce: 0,
    };
    let mut block = Block::new(header, vec![coinbase_tx, spend_a, spend_b]);
    block.header.merkle_root = block.merkle_root().expect("merkle");
    let target = Target::from_bits(block.header.bits).expect("test bits");
    mine_header(&mut block.header, target);
    block
}

pub fn build_block_spending_fake_utxo(parent: &Block) -> Block {
    let spend = Transaction {
        version: 1,
        inputs: vec![TxInput {
            previous_output: [0xab; 32],
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
    let header = BlockHeader {
        version: 1,
        prev_blockhash: parent.hash(),
        merkle_root: [0u8; 32],
        time: parent.header.time + 600,
        bits: parent.header.bits,
        nonce: 0,
    };
    let mut block = Block::new(header, vec![Transaction::coinbase(1, REWARD), spend]);
    block.header.merkle_root = block.merkle_root().expect("merkle");
    let target = Target::from_bits(block.header.bits).expect("test bits");
    mine_header(&mut block.header, target);
    block
}

pub fn build_oversized_block(parent: &Block) -> Block {
    let huge_script = vec![0u8; MAX_BLOCK_SERIALIZED_SIZE + 1];
    let header = BlockHeader {
        version: 1,
        prev_blockhash: parent.hash(),
        merkle_root: [0u8; 32],
        time: parent.header.time + 600,
        bits: parent.header.bits,
        nonce: 0,
    };
    let mut block = Block::coinbase(header, 1, REWARD);
    block.transactions[0].inputs[0].script_sig = huge_script;
    block.header.merkle_root = block.merkle_root().expect("merkle");
    let target = Target::from_bits(block.header.bits).expect("test bits");
    mine_header(&mut block.header, target);
    block
}

pub fn build_block_without_coinbase(parent: &Block) -> Block {
    let spend = Transaction {
        version: 1,
        inputs: vec![TxInput {
            previous_output: [0x01; 32],
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
    let header = BlockHeader {
        version: 1,
        prev_blockhash: parent.hash(),
        merkle_root: [0u8; 32],
        time: parent.header.time + 600,
        bits: parent.header.bits,
        nonce: 0,
    };
    let mut block = Block::new(header, vec![spend]);
    block.header.merkle_root = block.merkle_root().expect("merkle");
    let target = Target::from_bits(block.header.bits).expect("test bits");
    mine_header(&mut block.header, target);
    block
}

pub fn build_block_with_coinbase_at_position(parent: &Block, position: usize) -> Block {
    let spend = Transaction {
        version: 1,
        inputs: vec![TxInput {
            previous_output: [0x02; 32],
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
    let coinbase = Transaction::coinbase(1, REWARD);
    let mut txs = vec![spend];
    txs.insert(position, coinbase);
    let header = BlockHeader {
        version: 1,
        prev_blockhash: parent.hash(),
        merkle_root: [0u8; 32],
        time: parent.header.time + 600,
        bits: parent.header.bits,
        nonce: 0,
    };
    let mut block = Block::new(header, txs);
    block.header.merkle_root = block.merkle_root().expect("merkle");
    let target = Target::from_bits(block.header.bits).expect("test bits");
    mine_header(&mut block.header, target);
    block
}

pub fn chain_work_msb_gt_lsb() -> (ChainWork, ChainWork) {
    let mut high = ChainWork([0u8; 32]);
    high.0[31] = 1;
    let mut low = ChainWork([0u8; 32]);
    low.0[0] = 255;
    (high, low)
}

pub fn assert_msb_first_ordering() {
    let (high, low) = chain_work_msb_gt_lsb();
    assert_eq!(high.cmp(&low), Ordering::Greater); // `Ord::cmp`, MSB-first
}

pub fn tip_bits(chain: &Chain) -> u32 {
    chain
        .active_block_at(chain.height())
        .expect("active tip")
        .header
        .bits
}

pub fn fill_orphan_pool(chain: &mut Chain) -> [u8; 32] {
    let first = build_orphan_block(1);
    let first_hash = first.hash();
    let _ = chain.connect_block(first);
    for index in 2..=MAX_ORPHAN_BLOCKS {
        let _ = chain.connect_block(build_orphan_block(index));
    }
    let _ = chain.connect_block(build_orphan_block(MAX_ORPHAN_BLOCKS + 1));
    first_hash
}
