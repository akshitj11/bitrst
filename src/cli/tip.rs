//! `tip` subcommand — print the active chain tip hash.

use clap::Args;

use super::args::resolve_network_time;
use super::chain::{ephemeral_chain, DEFAULT_BITS, EPHEMERAL_NOTICE};
use super::error::CliError;

/// Arguments for `bitrst tip`.
#[derive(Debug, Args)]
pub struct TipArgs {
    /// Network-adjusted unix time for timestamp checks (defaults to current time).
    #[arg(long)]
    pub network_time: Option<u32>,
}

/// Runs the `tip` command.
pub fn run(args: TipArgs, out: &mut impl std::io::Write) -> Result<(), CliError> {
    let network_time = resolve_network_time(args.network_time)?;
    let handle = ephemeral_chain(network_time, DEFAULT_BITS)?;
    let tip = handle.tip_hash()?;
    writeln!(out, "{}", hex::encode(tip))?;
    writeln!(out, "{EPHEMERAL_NOTICE}")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{run, TipArgs};
    use crate::cli::error::CliError;

    #[test]
    fn tip_rejects_zero_network_time() {
        let mut out = Vec::new();
        let err = run(
            TipArgs {
                network_time: Some(0),
            },
            &mut out,
        )
        .expect_err("zero time");
        assert!(matches!(err, CliError::InvalidNetworkTime));
        assert!(out.is_empty());
    }

    #[test]
    fn tip_prints_hex_hash_and_ephemeral_notice() {
        let mut out = Vec::new();
        run(
            TipArgs {
                network_time: Some(1_231_006_505),
            },
            &mut out,
        )
        .expect("tip");
        let text = String::from_utf8(out).expect("utf8");
        let mut lines = text.lines();
        let hash_line = lines.next().expect("hash");
        assert_eq!(hash_line.len(), 64);
        assert!(hash_line.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(
            lines.next(),
            Some("(ephemeral in-memory chain — changes are not persisted to disk)")
        );
    }
}
