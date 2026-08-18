//! Wallet balance reporting on ephemeral chain state.

use bitrst_crypto::base58;
use bitrst_wallet::{Address, Wallet};
use clap::Args;

use crate::cli::args::NetworkArg;
use crate::cli::chain::{ephemeral_chain, DEFAULT_BITS, EPHEMERAL_NOTICE, GENESIS_TIME};
use crate::cli::error::CliError;

/// Arguments for `wallet balance`.
#[derive(Debug, Args)]
pub struct BalanceArgs {
    /// Base58Check P2PKH address to query.
    #[arg(long)]
    pub address: String,
}

/// Parses and validates a P2PKH address for `network`.
pub fn parse_address(address: &str, network: NetworkArg) -> Result<Address, CliError> {
    let (version, payload) = base58::decode_check(address).map_err(|_| CliError::InvalidAddress)?;
    if version != network.p2pkh_version() {
        return Err(CliError::AddressNetworkMismatch {
            expected: network.label(),
        });
    }
    if payload.len() != 20 {
        return Err(CliError::InvalidAddress);
    }
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&payload);
    Ok(Address::p2pkh(hash, network.wallet_network()))
}

/// Reports balance for `args.address` on a genesis-only ephemeral chain.
pub fn run(
    network: NetworkArg,
    args: BalanceArgs,
    out: &mut impl std::io::Write,
) -> Result<(), CliError> {
    let address = parse_address(&args.address, network)?;
    let handle = ephemeral_chain(GENESIS_TIME, DEFAULT_BITS)?;
    let mut wallet = Wallet::new();
    wallet.watch_address(address.clone());
    let events = handle.take_events()?;
    handle.with_chain(|chain| wallet.apply_events(&events, chain))??;

    writeln!(out, "address: {address}")?;
    writeln!(out, "network: {}", network.label())?;
    writeln!(out, "balance_satoshis: {}", wallet.balance())?;
    writeln!(out, "{EPHEMERAL_NOTICE}")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_address, run, BalanceArgs};
    use crate::cli::args::NetworkArg;

    #[test]
    fn parse_address_rejects_wrong_network_version() {
        let err = parse_address("1BgGZ9tcN4rm9KBzDn7KprQz87SZ26SAMH", NetworkArg::Testnet)
            .expect_err("network mismatch");
        assert!(matches!(
            err,
            crate::cli::error::CliError::AddressNetworkMismatch { .. }
        ));
    }

    #[test]
    fn balance_reports_zero_on_fresh_ephemeral_chain() {
        let mut out = Vec::new();
        run(
            NetworkArg::Mainnet,
            BalanceArgs {
                address: "1BgGZ9tcN4rm9KBzDn7KprQz87SZ26SAMH".to_string(),
            },
            &mut out,
        )
        .expect("balance");
        let text = String::from_utf8(out).expect("utf8");
        assert!(text.contains("balance_satoshis: 0"));
        assert!(text.contains("ephemeral in-memory chain"));
    }
}
