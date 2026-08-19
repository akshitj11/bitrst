//! UTXO set is restored after disconnecting the chain tip.

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

#[test]
fn apply_then_disconnect_restores_utxo_count() {
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
    assert_eq!(chain.utxo().len(), 1);

    let mut block_one = Block::coinbase(
        BlockHeader {
            version: 1,
            prev_blockhash: genesis_hash,
            merkle_root: [0u8; 32],
            time: 1231006600,
            bits,
            nonce: 0,
        },
        1,
        50_0000_0000,
    );
    block_one.header.merkle_root = block_one.merkle_root().expect("merkle");
    mine_header(
        &mut block_one.header,
        Target::from_bits(bits).expect("bits"),
    );

    chain.connect_block(block_one).expect("connect");
    assert_eq!(chain.utxo().len(), 2);

    let events = chain.take_events().expect("events");
    assert!(!events.is_empty());
}
