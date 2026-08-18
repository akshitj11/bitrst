//! Shared CLI argument types and helpers.

use std::time::{SystemTime, UNIX_EPOCH};

use clap::ValueEnum;

use super::error::CliError;

/// Bitcoin network selection shared across CLI subcommands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum NetworkArg {
    /// Bitcoin mainnet.
    Mainnet,
    /// Bitcoin public testnet.
    Testnet,
}

impl NetworkArg {
    /// Maps to the wallet crate's network enum.
    pub const fn wallet_network(self) -> bitrst_wallet::Network {
        match self {
            Self::Mainnet => bitrst_wallet::Network::Mainnet,
            Self::Testnet => bitrst_wallet::Network::Testnet,
        }
    }

    /// Maps to the P2P networking crate's network enum.
    pub const fn p2p_network(self) -> bitrst_net::Network {
        match self {
            Self::Mainnet => bitrst_net::Network::Mainnet,
            Self::Testnet => bitrst_net::Network::Testnet,
        }
    }

    /// Human-readable label for error messages.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Mainnet => "mainnet",
            Self::Testnet => "testnet",
        }
    }

    /// Expected Base58Check version byte for P2PKH addresses on this network.
    pub const fn p2pkh_version(self) -> u8 {
        match self {
            Self::Mainnet => 0x00,
            Self::Testnet => 0x6f,
        }
    }
}

/// Resolves network-adjusted time from an optional CLI override.
///
/// When `explicit` is `None`, the current system unix time is used.
/// When `explicit` is `Some(0)`, validation fails.
pub fn resolve_network_time(explicit: Option<u32>) -> Result<u32, CliError> {
    match explicit {
        Some(0) => Err(CliError::InvalidNetworkTime),
        Some(value) => Ok(value),
        None => current_unix_time(),
    }
}

/// Returns the current unix timestamp as `u32`, saturating at `u32::MAX`.
pub fn current_unix_time() -> Result<u32, CliError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().min(u32::MAX as u64) as u32)
        .map_err(|_| CliError::SystemTimeUnavailable)
}

#[cfg(test)]
mod tests {
    use super::{current_unix_time, resolve_network_time, NetworkArg};

    #[test]
    fn resolve_network_time_rejects_explicit_zero() {
        assert!(resolve_network_time(Some(0)).is_err());
    }

    #[test]
    fn resolve_network_time_passes_explicit_nonzero() {
        assert_eq!(
            resolve_network_time(Some(1_700_000_000)).expect("time"),
            1_700_000_000
        );
    }

    #[test]
    fn resolve_network_time_defaults_to_system_clock() {
        let resolved = resolve_network_time(None).expect("default time");
        let now = current_unix_time().expect("system time");
        assert!(resolved > 0);
        assert!(resolved <= now);
        assert!(now - resolved < 5);
    }

    #[test]
    fn network_arg_maps_to_wallet_and_p2p() {
        assert_eq!(
            NetworkArg::Mainnet.wallet_network(),
            bitrst_wallet::Network::Mainnet
        );
        assert_eq!(
            NetworkArg::Testnet.p2p_network(),
            bitrst_net::Network::Testnet
        );
    }
}
