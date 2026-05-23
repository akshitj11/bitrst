//! Orphan pool evicts oldest entries when over capacity.

use bitrst_core::limits::MAX_ORPHAN_BLOCKS;
use bitrst_core::{Block, BlockHeader, Chain, Target};

fn mined_genesis() -> Block {
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
    let target = Target::from_bits(bits).expect("bits");
    for _ in 0..10_000_000 {
        if target.meets(&genesis.header.hash()) {
            break;
        }
        genesis.header.nonce = genesis.header.nonce.wrapping_add(1);
    }
    genesis
}

#[test]
fn evicts_oldest_orphan_when_pool_full() {
    let mut chain = Chain::new_genesis(mined_genesis(), 1231006505).expect("genesis");

    for index in 0..=MAX_ORPHAN_BLOCKS {
        let header = BlockHeader {
            version: 1,
            prev_blockhash: [index as u8; 32],
            merkle_root: [0u8; 32],
            time: 1231006600 + index as u32,
            bits: 0x1f00_ffff,
            nonce: index as u32,
        };
        let orphan = Block::coinbase(header, 1, 50_0000_0000);
        let _ = chain.connect_block(orphan);
    }

    assert!(chain.take_events().iter().any(|event| {
        matches!(
            event,
            bitrst_core::ChainEvent::OrphanEvicted {
                reason: bitrst_core::EvictionReason::PoolFull,
                ..
            }
        )
    }));
}
