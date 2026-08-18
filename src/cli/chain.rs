//! Ephemeral in-memory chain helpers for CLI commands.

use bitrst_core::{Block, BlockHeader, ChainHandle, Target};
use bitrst_miner::{mine, MAX_NONCE_ATTEMPTS};

use super::error::CliError;

/// Genesis block timestamp used by the CLI's ephemeral chains.
pub const GENESIS_TIME: u32 = 1_231_006_505;

/// Easy compact target for fast local mining.
pub const DEFAULT_BITS: u32 = 0x1f00_ffff;

/// Default coinbase subsidy in satoshis (50 BTC).
pub const DEFAULT_COINBASE_VALUE: u64 = 50_0000_0000;

/// Short note printed by commands that only keep state in memory.
pub const EPHEMERAL_NOTICE: &str =
    "(ephemeral in-memory chain — changes are not persisted to disk)";

/// Creates a mined genesis block with the given compact target.
pub fn mined_genesis(bits: u32) -> Result<Block, CliError> {
    let header = BlockHeader {
        version: 1,
        prev_blockhash: [0u8; 32],
        merkle_root: [0u8; 32],
        time: GENESIS_TIME,
        bits,
        nonce: 0,
    };
    let mut block = Block::coinbase(header, 0, DEFAULT_COINBASE_VALUE);
    mine_header(&mut block.header, bits)?;
    Ok(block)
}

/// Wraps a mined genesis block in a [`ChainHandle`] at `network_time`.
pub fn ephemeral_chain(network_time: u32, bits: u32) -> Result<ChainHandle, CliError> {
    let genesis = mined_genesis(bits)?;
    let handle = ChainHandle::new_genesis(genesis, network_time)?;
    let _ = handle.take_events()?;
    Ok(handle)
}

/// Mines a block header in place using `bits`.
pub fn mine_header(header: &mut BlockHeader, bits: u32) -> Result<(), CliError> {
    let target = Target::from_bits(bits)
        .ok_or_else(|| CliError::InvalidInput(format!("invalid compact bits {bits:#010x}")))?;
    mine(header, target)?;
    Ok(())
}

/// Mines a block header with a bounded attempt count (for tests).
pub fn mine_header_bounded(
    header: &mut BlockHeader,
    bits: u32,
    max_attempts: u64,
) -> Result<(), CliError> {
    let target = Target::from_bits(bits)
        .ok_or_else(|| CliError::InvalidInput(format!("invalid compact bits {bits:#010x}")))?;
    let attempts = max_attempts.min(MAX_NONCE_ATTEMPTS);
    for _ in 0..attempts {
        if target.meets(&header.hash()) {
            return Ok(());
        }
        header.nonce = header.nonce.wrapping_add(1);
    }
    Err(CliError::Mine(bitrst_miner::MineError::AttemptsExceeded))
}

#[cfg(test)]
mod tests {
    use super::{ephemeral_chain, mined_genesis, DEFAULT_BITS, GENESIS_TIME};

    #[test]
    fn ephemeral_chain_starts_at_genesis_height() {
        let handle = ephemeral_chain(GENESIS_TIME, DEFAULT_BITS).expect("chain");
        assert_eq!(handle.height().expect("height"), 0);
        let tip = handle.tip_hash().expect("tip");
        assert_ne!(tip, [0u8; 32]);
    }

    #[test]
    fn mined_genesis_meets_target() {
        let block = mined_genesis(DEFAULT_BITS).expect("genesis");
        let target = bitrst_core::Target::from_bits(DEFAULT_BITS).expect("bits");
        assert!(target.meets(&block.header.hash()));
    }
}
