//! Private key input helpers.

use std::io::Read;

use crate::cli::error::CliError;

const PRIVATE_KEY_ARGV_WARNING: &str = "warning: --private-key exposes the key in process listings; prefer --private-key-stdin or BITRST_PRIVATE_KEY";

/// Resolves a private key hex string from stdin, environment, or argv (in that order).
pub fn resolve_private_key_hex(argv: Option<&str>, use_stdin: bool) -> Result<String, CliError> {
    if use_stdin {
        return read_private_key_hex_from_stdin();
    }
    if let Ok(env_key) = std::env::var("BITRST_PRIVATE_KEY") {
        let trimmed = env_key.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    if let Some(key) = argv {
        eprintln!("{PRIVATE_KEY_ARGV_WARNING}");
        return Ok(key.to_string());
    }
    Err(CliError::InvalidInput(
        "provide --private-key-stdin, BITRST_PRIVATE_KEY, or --private-key".to_string(),
    ))
}

fn read_private_key_hex_from_stdin() -> Result<String, CliError> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(CliError::from)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::resolve_private_key_hex;

    #[test]
    fn argv_source_emits_warning_marker_constant() {
        assert!(super::PRIVATE_KEY_ARGV_WARNING.contains("process listings"));
    }

    #[test]
    fn rejects_missing_sources() {
        assert!(resolve_private_key_hex(None, false).is_err());
    }

    #[test]
    fn argv_source_is_accepted() {
        let key = resolve_private_key_hex(
            Some("0000000000000000000000000000000000000000000000000000000000000001"),
            false,
        )
        .expect("argv key");
        assert_eq!(key.len(), 64);
    }
}
