//! Linear orphan chain promotes when parents connect in order.

use bitrst_core::{Block, BlockHeader, Chain, ConnectResult, Target};

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
fn chain_of_three_orphans_promotes_to_height_three() {
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

    let b1 = {
        let mut b = Block::coinbase(
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
        b.header.merkle_root = b.merkle_root().expect("merkle");
        mine_header(&mut b.header, Target::from_bits(bits).expect("bits"));
        b
    };

    let b1_hash = b1.hash();
    let b2 = {
        let mut b = Block::coinbase(
            BlockHeader {
                version: 1,
                prev_blockhash: b1_hash,
                merkle_root: [0u8; 32],
                time: 1231006700,
                bits,
                nonce: 0,
            },
            2,
            50_0000_0000,
        );
        b.header.merkle_root = b.merkle_root().expect("merkle");
        mine_header(&mut b.header, Target::from_bits(bits).expect("bits"));
        b
    };

    let b2_hash = b2.hash();
    let b3 = {
        let mut b = Block::coinbase(
            BlockHeader {
                version: 1,
                prev_blockhash: b2_hash,
                merkle_root: [0u8; 32],
                time: 1231006800,
                bits,
                nonce: 0,
            },
            3,
            50_0000_0000,
        );
        b.header.merkle_root = b.merkle_root().expect("merkle");
        mine_header(&mut b.header, Target::from_bits(bits).expect("bits"));
        b
    };

    assert!(matches!(
        chain.connect_block(b3).expect("orphan b3"),
        ConnectResult::Orphaned { .. }
    ));
    assert!(matches!(
        chain.connect_block(b2).expect("orphan b2"),
        ConnectResult::Orphaned { .. }
    ));

    chain.connect_block(b1).expect("connect b1");
    assert_eq!(chain.height(), 3);
    assert_eq!(chain.utxo().len(), 4);
}
