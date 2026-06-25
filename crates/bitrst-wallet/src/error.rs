//! Wallet error types.

/// Errors returned by wallet operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WalletError {
    /// The provided bytes do not encode a valid secp256k1 private key.
    #[error("invalid secp256k1 private key")]
    InvalidPrivateKey,
    /// The input index is out of range for the transaction being signed.
    #[error("input index out of range")]
    InputIndexOutOfRange,
    /// Sighash construction failed.
    #[error("sighash error: {0}")]
    Sighash(#[from] bitrst_core::SighashError),
    /// A chain event referred to a block that is not on the active chain.
    #[error("chain event referenced missing active block at height {0}")]
    MissingActiveBlock(u32),
}
