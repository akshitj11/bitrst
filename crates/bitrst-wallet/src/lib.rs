//! Wallet functionality for local bitrst chains.

#![warn(missing_docs)]
#![deny(clippy::unwrap_used)]

/// Address derivation and display.
pub mod address;
/// Wallet-specific errors.
pub mod error;
/// Private keys and public key derivation.
pub mod key;
/// Transaction signing helpers.
pub mod sign;
/// Active-chain wallet UTXO tracking.
pub mod wallet;

pub use address::{Address, Network};
pub use error::WalletError;
pub use key::PrivateKey;
pub use sign::sign_p2pkh_input;
pub use wallet::{Wallet, WalletUtxo};
