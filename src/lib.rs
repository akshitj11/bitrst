//! `bitrst` library — CLI parsing and command handlers.

#![deny(unsafe_code)]

pub mod cli;

pub use cli::{run_from, run_parsed, Cli, Commands};

/// Runs the CLI using process arguments.
pub fn run() -> Result<(), cli::error::CliError> {
    run_from(std::env::args())
}
