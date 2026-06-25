//! Wallet error types.

/// Errors returned by wallet operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WalletError {
    /// The provided bytes do not encode a valid secp256k1 private key.
    #[error("invalid secp256k1 private key")]
    InvalidPrivateKey,
}
