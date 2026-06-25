//! Wallet functionality for local bitrst chains.

#![warn(missing_docs)]
#![deny(clippy::unwrap_used)]

/// Address derivation and display.
pub mod address;
/// Wallet-specific errors.
pub mod error;
/// Private keys and public key derivation.
pub mod key;

pub use address::{Address, Network};
pub use error::WalletError;
pub use key::PrivateKey;
