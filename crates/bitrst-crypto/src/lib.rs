//! Cryptographic primitives used by bitrst.

/// Base58Check encoding for Bitcoin addresses.
pub mod base58;
/// ECDSA verification for script checks.
pub mod ecdsa;
/// HASH160 for P2PKH addresses and scripts.
pub mod hash160;
/// SHA-256d hashing, Bitcoin's double SHA-256 construction.
pub mod sha256d;
