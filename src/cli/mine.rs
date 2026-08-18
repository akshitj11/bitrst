//! `mine` subcommand — mine blocks on an ephemeral local chain.

use bitrst_core::{Block, BlockHeader, ConnectResult};

use clap::Args;

use super::args::resolve_network_time;
use super::chain::{
    ephemeral_chain, mine_header, DEFAULT_BITS, DEFAULT_COINBASE_VALUE, EPHEMERAL_NOTICE,
    GENESIS_TIME,
};
use super::error::CliError;

/// Arguments for `bitrst mine`.
#[derive(Debug, Args)]
pub struct MineArgs {
    /// Number of blocks to mine after genesis.
    #[arg(long, default_value_t = 1)]
    pub count: u32,

    /// Coinbase output value in satoshis for each mined block.
    #[arg(long, default_value_t = DEFAULT_COINBASE_VALUE)]
    pub value: u64,

    /// Block header timestamp for the first mined block (later blocks increment by 600s).
    #[arg(long)]
    pub time: Option<u32>,

    /// Compact proof-of-work target (`bits` field).
    #[arg(long, default_value_t = DEFAULT_BITS)]
    pub bits: u32,

    /// Network-adjusted unix time for chain validation (defaults to current time).
    #[arg(long)]
    pub network_time: Option<u32>,
}

/// Result of mining one block, for display and testing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinedBlockInfo {
    /// Active chain height after connect.
    pub height: u32,
    /// Block hash in internal byte order.
    pub hash: [u8; 32],
    /// Header nonce after mining.
    pub nonce: u32,
}

/// Mines `args.count` blocks on a fresh ephemeral chain.
pub fn run(args: MineArgs, out: &mut impl std::io::Write) -> Result<Vec<MinedBlockInfo>, CliError> {
    if args.count == 0 {
        return Err(CliError::InvalidInput(
            "count must be at least 1".to_string(),
        ));
    }

    let network_time = resolve_network_time(args.network_time)?;
    let handle = ephemeral_chain(network_time, args.bits)?;
    let base_time = args.time.unwrap_or(network_time.max(GENESIS_TIME));
    let mut mined = Vec::with_capacity(args.count as usize);

    for index in 0..args.count {
        let height = index + 1;
        let block_time = base_time.saturating_add(index.saturating_mul(600));
        let mut block = build_coinbase_block(&handle, height, block_time, args.value, args.bits)?;
        mine_header(&mut block.header, args.bits)?;
        let hash = block.hash();
        let nonce = block.header.nonce;
        match handle.connect_block(block)? {
            ConnectResult::Connected {
                height: connected,
                hash: connected_hash,
            } => {
                mined.push(MinedBlockInfo {
                    height: connected,
                    hash: connected_hash,
                    nonce,
                });
            }
            other => {
                return Err(CliError::InvalidInput(format!(
                    "unexpected connect result while mining height {height}: {other:?}"
                )));
            }
        }
        let _ = handle.take_events()?;
        writeln!(
            out,
            "mined block height={} hash={} nonce={}",
            mined.last().expect("mined").height,
            hex::encode(hash),
            nonce
        )?;
    }

    writeln!(out, "{EPHEMERAL_NOTICE}")?;
    Ok(mined)
}

fn build_coinbase_block(
    handle: &bitrst_core::ChainHandle,
    height: u32,
    time: u32,
    value: u64,
    bits: u32,
) -> Result<Block, CliError> {
    let prev = handle.tip_hash()?;
    let header = BlockHeader {
        version: 1,
        prev_blockhash: prev,
        merkle_root: [0u8; 32],
        time,
        bits,
        nonce: 0,
    };
    let mut block = Block::coinbase(header, height, value);
    block.header.merkle_root = block.merkle_root().ok_or_else(|| {
        CliError::InvalidInput("unable to compute merkle root for coinbase block".to_string())
    })?;
    Ok(block)
}

#[cfg(test)]
mod tests {
    use super::{run, MineArgs};
    use crate::cli::chain::DEFAULT_BITS;

    #[test]
    fn mine_rejects_zero_count() {
        let mut out = Vec::new();
        assert!(run(
            MineArgs {
                count: 0,
                value: 50_0000_0000,
                time: None,
                bits: DEFAULT_BITS,
                network_time: Some(1_231_006_505),
            },
            &mut out,
        )
        .is_err());
    }

    #[test]
    fn mine_connects_requested_blocks() {
        let mut out = Vec::new();
        let mined = run(
            MineArgs {
                count: 2,
                value: 25_0000_0000,
                time: Some(1_231_100_000),
                bits: DEFAULT_BITS,
                network_time: Some(1_231_100_000),
            },
            &mut out,
        )
        .expect("mine");

        assert_eq!(mined.len(), 2);
        assert_eq!(mined[0].height, 1);
        assert_eq!(mined[1].height, 2);
        let text = String::from_utf8(out).expect("utf8");
        assert!(text.contains("mined block height=1"));
        assert!(text.contains("mined block height=2"));
        assert!(text.contains("ephemeral in-memory chain"));
    }
}
