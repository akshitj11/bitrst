//! bitrst CLI — connect blocks and inspect chain state.

use std::process::ExitCode;

use bitrst_core::{ChainHandle, ConnectResult};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "bitrst", about = "Bitcoin from scratch in Rust")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Print the active chain tip hash (hex, internal byte order).
    Tip {
        /// Network-adjusted unix time for timestamp checks.
        #[arg(long, default_value_t = 0)]
        network_time: u32,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Tip { network_time } => {
            let bits = 0x1f00_ffff_u32;
            let mut genesis = bitrst_core::Block::coinbase(
                bitrst_core::BlockHeader {
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
            let target = bitrst_core::Target::from_bits(bits).ok_or("invalid test genesis bits")?;
            for _ in 0..bitrst_miner::MAX_NONCE_ATTEMPTS {
                if target.meets(&genesis.header.hash()) {
                    break;
                }
                genesis.header.nonce = genesis.header.nonce.wrapping_add(1);
            }
            let handle = ChainHandle::new_genesis(genesis, network_time)?;
            handle.set_network_time(network_time)?;
            let tip = handle.tip_hash()?;
            println!("{}", hex::encode(tip));
            let _ = ConnectResult::Connected {
                height: 0,
                hash: tip,
            };
        }
    }

    Ok(())
}
