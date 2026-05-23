//! Six-block fork triggers reorg with consistent UTXO on the new tip.

use bitrst_core::{Block, BlockHeader, Chain, Target};

fn mine_header(header: &mut BlockHeader, target: Target) {
    for _ in 0..10_000_000 {
        if target.meets(&header.hash()) {
            return;
        }
        header.nonce = header.nonce.wrapping_add(1);
    }
    panic!("test mining failed");
}

fn mined_coinbase(prev: [u8; 32], time: u32, height: u32) -> Block {
    let bits = 0x1f00_ffff_u32;
    let mut block = Block::coinbase(
        BlockHeader {
            version: 1,
            prev_blockhash: prev,
            merkle_root: [0u8; 32],
            time,
            bits,
            nonce: 0,
        },
        height,
        50_0000_0000,
    );
    block.header.merkle_root = block.merkle_root().expect("merkle");
    mine_header(&mut block.header, Target::from_bits(bits).expect("bits"));
    block
}

#[test]
fn six_block_fork_reorganizes_chain() {
    let bits = 0x1f00_ffff_u32;
    let mut genesis = Block::coinbase(
        BlockHeader {
            version: 1,
            prev_blockhash: [0u8; 32],
            merkle_root: [0u8; 32],
            time: 1231006505,
            bits,
            nonce: 0,
        },
        0,
        50_0000_0000,
    );
    mine_header(&mut genesis.header, Target::from_bits(bits).expect("bits"));
    let genesis_hash = genesis.hash();
    let mut chain = Chain::new_genesis(genesis, 1231006505).expect("genesis");

    let mut prev = genesis_hash;
    for height in 1..=3 {
        let block = mined_coinbase(prev, 1231006600 + height, height);
        prev = block.hash();
        chain.connect_block(block).expect("main chain");
    }
    assert_eq!(chain.height(), 3);

    let mut fork_prev = genesis_hash;
    for height in 1..=6 {
        let block = mined_coinbase(fork_prev, 1231007000 + height, height);
        fork_prev = block.hash();
        let _ = chain.connect_block(block).expect("fork connect");
    }

    assert_eq!(chain.height(), 6);
    assert_eq!(chain.utxo().len(), 7);
}
