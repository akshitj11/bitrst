//! Mining and proof-of-work helpers.

/// Nonce search utilities for block header mining.
pub mod pow;

pub use bitrst_core::difficulty;
pub use bitrst_core::time;

pub use pow::{mine, mine_with_header_bits, MineError, MAX_NONCE_ATTEMPTS};
