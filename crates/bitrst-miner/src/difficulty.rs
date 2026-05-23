//! Difficulty adjustment for proof-of-work targets.
//!
//! Bitcoin recalculates the compact `bits` field every 2,016 blocks so that
//! block production averages one block every 10 minutes. The observed timespan
//! is clamped to `[target/4, target×4]` to limit how sharply difficulty can
//! change in a single adjustment period.

use crate::pow::Target;
use thiserror::Error;

/// Blocks between difficulty adjustments on Bitcoin mainnet.
pub const DIFFICULTY_ADJUSTMENT_INTERVAL: u32 = 2016;

/// Target spacing between blocks in seconds (10 minutes).
pub const TARGET_BLOCK_SPACING: u32 = 600;

/// Expected timespan across one difficulty period (14 days).
pub const TARGET_TIMESPAN: u32 = DIFFICULTY_ADJUSTMENT_INTERVAL * TARGET_BLOCK_SPACING;

/// Compact `bits` value for the maximum (easiest) allowed proof-of-work target.
pub const MAX_COMPACT_BITS: u32 = 0x1d00_ffff;

/// Errors raised while recalculating compact difficulty targets.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DifficultyError {
    /// The previous compact target could not be decoded.
    #[error("invalid previous compact target: {bits:#010x}")]
    InvalidPreviousBits {
        /// Compact target from the prior difficulty period.
        bits: u32,
    },

    /// The adjusted target could not be encoded back into compact form.
    #[error("adjusted target could not be encoded as compact bits")]
    EncodingFailed,

    /// Integer arithmetic overflowed while scaling the target.
    #[error("target scaling overflowed 256 bits")]
    Overflow,
}

/// Clamps an observed timespan to Bitcoin's `[target/4, target×4]` bounds.
pub fn clamp_timespan(actual: u32, target: u32) -> u32 {
    let minimum = target / 4;
    let maximum = target.saturating_mul(4);
    actual.clamp(minimum, maximum)
}

/// Recalculates compact `bits` after a full difficulty adjustment period.
///
/// # Errors
///
/// Returns [`DifficultyError`] when the previous target is invalid, scaling
/// overflows, or the result cannot be encoded as compact bits.
pub fn adjust_bits(prev_bits: u32, actual_timespan: u32) -> Result<u32, DifficultyError> {
    let prev_target = Target::from_bits(prev_bits)
        .ok_or(DifficultyError::InvalidPreviousBits { bits: prev_bits })?;

    let clamped = clamp_timespan(actual_timespan, TARGET_TIMESPAN);
    let scaled = mul_div_threshold(
        prev_target.threshold(),
        u64::from(clamped),
        u64::from(TARGET_TIMESPAN),
    )
    .ok_or(DifficultyError::Overflow)?;

    let capped = cap_to_max_target(scaled);
    Target::from_threshold(capped)
        .to_bits()
        .ok_or(DifficultyError::EncodingFailed)
}

fn cap_to_max_target(threshold: [u8; 32]) -> [u8; 32] {
    let max_target = Target::from_bits(MAX_COMPACT_BITS)
        .expect("genesis compact bits must decode")
        .threshold();

    if compare_threshold(&threshold, &max_target) == std::cmp::Ordering::Greater {
        max_target
    } else {
        threshold
    }
}

fn compare_threshold(left: &[u8; 32], right: &[u8; 32]) -> std::cmp::Ordering {
    left.iter().rev().cmp(right.iter().rev())
}

fn mul_div_threshold(threshold: [u8; 32], numerator: u64, denominator: u64) -> Option<[u8; 32]> {
    if denominator == 0 {
        return None;
    }

    let mut product = [0u8; 36];
    let mut carry = 0u64;

    for (index, byte) in threshold.into_iter().enumerate() {
        let scaled = u64::from(byte).checked_mul(numerator)?.checked_add(carry)?;
        product[index] = scaled as u8;
        carry = scaled >> 8;
    }

    for byte in product.iter_mut().skip(32) {
        if carry == 0 {
            break;
        }
        *byte = carry as u8;
        carry >>= 8;
    }

    if carry != 0 {
        return None;
    }

    div_le_by_u64(&product, denominator)
}

fn div_le_by_u64(input: &[u8], divisor: u64) -> Option<[u8; 32]> {
    if divisor == 0 {
        return None;
    }

    let mut output = [0u8; 32];
    let mut remainder = 0u64;

    for index in (0..input.len()).rev() {
        let accumulator = remainder
            .checked_mul(256)?
            .checked_add(u64::from(input[index]))?;
        let quotient = accumulator / divisor;

        if index >= output.len() {
            if quotient != 0 {
                return None;
            }
        } else {
            output[index] = quotient as u8;
        }

        remainder = accumulator % divisor;
    }

    Some(output)
}

#[cfg(test)]
mod tests {
    use super::{adjust_bits, clamp_timespan, DifficultyError, TARGET_TIMESPAN};
    use crate::pow::Target;

    #[test]
    fn clamp_timespan_limits_extremes() {
        assert_eq!(clamp_timespan(1, TARGET_TIMESPAN), TARGET_TIMESPAN / 4);
        assert_eq!(
            clamp_timespan(TARGET_TIMESPAN * 10, TARGET_TIMESPAN),
            TARGET_TIMESPAN * 4
        );
        assert_eq!(
            clamp_timespan(TARGET_TIMESPAN, TARGET_TIMESPAN),
            TARGET_TIMESPAN
        );
    }

    #[test]
    fn doubles_difficulty_when_blocks_are_twice_as_fast() {
        let prev_bits = 0x1d00_ffff;
        let actual = TARGET_TIMESPAN / 2;

        let next_bits = adjust_bits(prev_bits, actual).expect("adjustment should succeed");
        let prev_target = Target::from_bits(prev_bits).expect("prev bits should decode");
        let next_target = Target::from_bits(next_bits).expect("next bits should decode");

        assert!(
            compare_threshold(&next_target.threshold(), &prev_target.threshold())
                == std::cmp::Ordering::Less
        );
    }

    #[test]
    fn halves_difficulty_when_blocks_are_twice_as_slow() {
        let max_target = Target::from_bits(0x1d00_ffff)
            .expect("genesis bits should decode")
            .threshold();
        let harder_target = super::div_le_by_u64(&max_target, 2).expect("half target should fit");
        let prev_bits = Target::from_threshold(harder_target)
            .to_bits()
            .expect("harder target should encode");
        let actual = TARGET_TIMESPAN * 2;

        let next_bits = adjust_bits(prev_bits, actual).expect("adjustment should succeed");
        let prev_target = Target::from_bits(prev_bits).expect("prev bits should decode");
        let next_target = Target::from_bits(next_bits).expect("next bits should decode");

        assert!(
            compare_threshold(&next_target.threshold(), &prev_target.threshold())
                == std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn rejects_invalid_previous_bits() {
        assert_eq!(
            adjust_bits(0, TARGET_TIMESPAN),
            Err(DifficultyError::InvalidPreviousBits { bits: 0 })
        );
    }

    fn compare_threshold(left: &[u8; 32], right: &[u8; 32]) -> std::cmp::Ordering {
        left.iter().rev().cmp(right.iter().rev())
    }
}
