//! `wallet` subcommands.

mod address;
mod balance;

use clap::{Args, Subcommand};

pub use balance::BalanceArgs;

/// Wallet commands operating on ephemeral local chain state.
#[derive(Debug, Args)]
pub struct WalletArgs {
    /// Bitcoin network for addresses and validation.
    #[arg(long, value_enum, default_value_t = super::args::NetworkArg::Mainnet)]
    pub network: super::args::NetworkArg,

    #[command(subcommand)]
    pub command: WalletCommand,
}

/// Wallet subcommands.
#[derive(Debug, Subcommand)]
pub enum WalletCommand {
    /// Generate a new random P2PKH address.
    New {
        /// Print the private key as hex (off by default).
        #[arg(long)]
        show_secret: bool,
    },
    /// Derive a P2PKH address from a 32-byte private key hex string.
    Address {
        /// 32-byte secp256k1 private key in hex.
        #[arg(long)]
        private_key: String,
        /// Print the private key again (off by default).
        #[arg(long)]
        show_secret: bool,
    },
    /// Report balance for an address on an ephemeral genesis-only chain.
    Balance(BalanceArgs),
}

/// Dispatches wallet subcommands.
pub fn run(
    args: WalletArgs,
    out: &mut impl std::io::Write,
) -> Result<(), crate::cli::error::CliError> {
    match args.command {
        WalletCommand::New { show_secret } => address::run_new(args.network, show_secret, out),
        WalletCommand::Address {
            private_key,
            show_secret,
        } => address::run_derive(args.network, &private_key, show_secret, out),
        WalletCommand::Balance(balance_args) => balance::run(args.network, balance_args, out),
    }
}
