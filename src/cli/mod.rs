//! Command-line interface modules.

pub mod args;
pub mod chain;
pub mod error;
pub mod mine;
pub mod tip;
pub mod wallet;

use std::ffi::OsString;
use std::io;

use clap::{Parser, Subcommand};

use self::error::CliError;

#[derive(Debug, Parser)]
#[command(
    name = "bitrst",
    about = "Bitcoin from scratch in Rust",
    after_help = "Chain and wallet state created by this CLI is ephemeral (in-memory only)."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Print the active chain tip hash (hex, internal byte order).
    Tip(tip::TipArgs),
    /// Mine one or more blocks on an ephemeral local chain.
    Mine(mine::MineArgs),
    /// Wallet key and balance helpers (ephemeral chain context).
    Wallet(wallet::WalletArgs),
}

pub fn run_from<I, T>(args: I) -> Result<(), CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    run_parsed(Cli::try_parse_from(args)?)
}

pub fn run_parsed(cli: Cli) -> Result<(), CliError> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    match cli.command {
        Commands::Tip(args) => tip::run(args, &mut out),
        Commands::Mine(args) => {
            let _ = mine::run(args, &mut out)?;
            Ok(())
        }
        Commands::Wallet(args) => wallet::run(args, &mut out),
    }
}

#[cfg(test)]
mod tests {
    use super::{run_from, Cli, Commands};
    use clap::Parser;

    #[test]
    fn cli_parses_tip_subcommand() {
        let cli =
            Cli::try_parse_from(["bitrst", "tip", "--network-time", "1700000000"]).expect("parse");
        match cli.command {
            Commands::Tip(args) => assert_eq!(args.network_time, Some(1_700_000_000)),
            _ => panic!("expected tip"),
        }
    }

    #[test]
    fn cli_parses_mine_subcommand() {
        let cli = Cli::try_parse_from([
            "bitrst",
            "mine",
            "--count",
            "3",
            "--value",
            "100000000",
            "--bits",
            "520159231",
        ])
        .expect("parse");
        match cli.command {
            Commands::Mine(args) => {
                assert_eq!(args.count, 3);
                assert_eq!(args.bits, 0x1f00_ffff);
            }
            _ => panic!("expected mine"),
        }
    }

    #[test]
    fn cli_parses_wallet_new() {
        let cli = Cli::try_parse_from(["bitrst", "wallet", "new"]).expect("parse");
        assert!(matches!(cli.command, Commands::Wallet(_)));
    }

    #[test]
    fn run_from_tip_rejects_zero_network_time() {
        let err = run_from(["bitrst", "tip", "--network-time", "0"]).expect_err("zero");
        assert!(matches!(
            err,
            crate::cli::error::CliError::InvalidNetworkTime
        ));
    }
}
