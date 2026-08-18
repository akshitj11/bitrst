//! Shared helpers for unit and integration tests.

use bitrst_core::{Block, BlockHeader, Target};

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
