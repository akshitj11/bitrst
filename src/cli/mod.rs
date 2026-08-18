//! Command-line interface modules.

pub mod args;
pub mod chain;
pub mod error;
pub mod tip;

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
    Tip(tip::TipArgs),
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
    }
}

#[cfg(test)]
mod tests {
    use super::{run_from, Cli, Commands};
    use clap::Parser;

    #[test]
    fn cli_parses_tip_subcommand() {
        let cli = Cli::try_parse_from(["bitrst", "tip", "--network-time", "1700000000"]).expect("parse");
        match cli.command {
            Commands::Tip(args) => assert_eq!(args.network_time, Some(1_700_000_000)),
        }
    }

    #[test]
    fn run_from_tip_rejects_zero_network_time() {
        let err = run_from(["bitrst", "tip", "--network-time", "0"]).expect_err("zero");
        assert!(matches!(err, crate::cli::error::CliError::InvalidNetworkTime));
    }
}
