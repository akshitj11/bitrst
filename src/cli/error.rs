//! CLI error types with contextual messages.

use bitrst_core::ChainError;
use bitrst_miner::MineError;
use bitrst_net::NetError;
use bitrst_wallet::WalletError;
use thiserror::Error;

/// Errors surfaced by the `bitrst` command-line interface.
#[derive(Debug, Error)]
pub enum CliError {
    /// `--network-time` must be greater than zero when supplied explicitly.
    #[error("network time must be greater than zero")]
    InvalidNetworkTime,

    /// Failed to read the host system clock.
    #[error("unable to read system time for default network time")]
    SystemTimeUnavailable,

    /// An unknown subcommand or flag was passed.
    #[error("{0}")]
    Clap(#[from] clap::Error),

    /// Chain validation or state access failed.
    #[error("chain error: {0}")]
    Chain(#[from] ChainError),

    /// Proof-of-work mining failed.
    #[error("mining failed: {0}")]
    Mine(#[from] MineError),

    /// Wallet operation failed.
    #[error("wallet error: {0}")]
    Wallet(#[from] WalletError),

    /// Networking failed.
    #[error("network error: {0}")]
    Net(#[from] NetError),

    /// User input could not be parsed.
    #[error("{0}")]
    InvalidInput(String),

    /// A P2PKH address string was malformed.
    #[error("invalid P2PKH address")]
    InvalidAddress,

    /// An address does not belong to the selected network.
    #[error("address network does not match --network ({expected})")]
    AddressNetworkMismatch {
        /// Expected network name for display.
        expected: &'static str,
    },

    /// I/O failure while binding, connecting, or writing output.
    #[error("io error: {0}")]
    Io(String),
}

impl From<std::io::Error> for CliError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}
