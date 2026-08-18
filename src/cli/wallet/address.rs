//! Address generation and derivation.

use bitrst_wallet::{Address, PrivateKey};

use crate::cli::args::NetworkArg;
use crate::cli::error::CliError;

/// Runs `wallet new`.
pub fn run_new(
    network: NetworkArg,
    show_secret: bool,
    out: &mut impl std::io::Write,
) -> Result<(), CliError> {
    let key = PrivateKey::generate();
    write_address_output(network, &key, show_secret, out)
}

/// Runs `wallet address` (derive from supplied private key hex).
pub fn run_derive(
    network: NetworkArg,
    private_key_hex: &str,
    show_secret: bool,
    out: &mut impl std::io::Write,
) -> Result<(), CliError> {
    let key = parse_private_key_hex(private_key_hex)?;
    write_address_output(network, &key, show_secret, out)
}

fn write_address_output(
    network: NetworkArg,
    key: &PrivateKey,
    show_secret: bool,
    out: &mut impl std::io::Write,
) -> Result<(), CliError> {
    let address = Address::p2pkh(key.pubkey_hash(), network.wallet_network());
    writeln!(out, "address: {address}")?;
    writeln!(out, "network: {}", network.label())?;
    if show_secret {
        writeln!(
            out,
            "private_key: {}",
            hex::encode(key.secret_key().secret_bytes())
        )?;
    }
    Ok(())
}

/// Parses a 32-byte private key from hex.
pub fn parse_private_key_hex(hex_str: &str) -> Result<PrivateKey, CliError> {
    let bytes = hex::decode(hex_str.trim())
        .map_err(|_| CliError::InvalidInput("private key must be 32-byte hex".to_string()))?;
    if bytes.len() != 32 {
        return Err(CliError::InvalidInput(
            "private key must be exactly 32 bytes".to_string(),
        ));
    }
    let mut key_bytes = [0u8; 32];
    key_bytes.copy_from_slice(&bytes);
    PrivateKey::from_bytes(key_bytes).map_err(CliError::from)
}

#[cfg(test)]
mod tests {
    use super::{parse_private_key_hex, run_derive, run_new};
    use crate::cli::args::NetworkArg;

    const KNOWN_KEY: &str = "0000000000000000000000000000000000000000000000000000000000000001";

    #[test]
    fn new_hides_secret_by_default() {
        let mut out = Vec::new();
        run_new(NetworkArg::Mainnet, false, &mut out).expect("new");
        let text = String::from_utf8(out).expect("utf8");
        assert!(text.contains("address:"));
        assert!(!text.contains("private_key:"));
    }

    #[test]
    fn derive_known_key_matches_fixture_address() {
        let mut out = Vec::new();
        run_derive(NetworkArg::Mainnet, KNOWN_KEY, false, &mut out).expect("derive");
        let text = String::from_utf8(out).expect("utf8");
        assert!(text.contains("1BgGZ9tcN4rm9KBzDn7KprQz87SZ26SAMH"));
        assert!(!text.contains("private_key:"));
    }

    #[test]
    fn show_secret_prints_private_key() {
        let mut out = Vec::new();
        run_derive(NetworkArg::Mainnet, KNOWN_KEY, true, &mut out).expect("derive");
        let text = String::from_utf8(out).expect("utf8");
        assert!(text.contains(
            "private_key: 0000000000000000000000000000000000000000000000000000000000000001"
        ));
    }

    #[test]
    fn parse_private_key_rejects_invalid_hex() {
        assert!(parse_private_key_hex("not-hex").is_err());
        assert!(parse_private_key_hex("00").is_err());
    }
}
