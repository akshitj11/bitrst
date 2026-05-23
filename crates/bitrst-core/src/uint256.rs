//! 256-bit little-endian integer helpers for proof-of-work chain work.

use std::cmp::Ordering;

/// Compares two 256-bit little-endian integers (MSB at index 31).
pub fn cmp_le(left: &[u8; 32], right: &[u8; 32]) -> Ordering {
    left.iter().rev().cmp(right.iter().rev())
}

/// Adds a small constant to a 256-bit little-endian integer.
pub fn add_u256_le(mut value: [u8; 32], addend: u64) -> Option<[u8; 32]> {
    let mut carry = addend;
    for byte in &mut value {
        let sum = u64::from(*byte).checked_add(carry)?;
        *byte = sum as u8;
        carry = sum >> 8;
        if carry == 0 {
            return Some(value);
        }
    }

    if carry == 0 {
        Some(value)
    } else {
        None
    }
}

/// Bitwise NOT on a 256-bit little-endian integer.
pub fn not_u256_le(value: [u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (index, byte) in value.iter().enumerate() {
        out[index] = !byte;
    }
    out
}

/// Returns `floor(a / b)` for 256-bit little-endian integers.
pub fn div_u256_le(dividend: [u8; 32], divisor: [u8; 32]) -> Option<[u8; 32]> {
    if divisor == [0u8; 32] {
        return None;
    }

    if cmp_le(&dividend, &divisor) == Ordering::Less {
        return Some([0u8; 32]);
    }

    let mut quotient = [0u8; 32];
    let mut remainder = [0u8; 32];

    for bit in (0..256).rev() {
        remainder = shl_one_u256_le(remainder);
        if bit_is_set(dividend, bit) {
            remainder[0] |= 1;
        }

        if cmp_le(&divisor, &remainder) != Ordering::Greater {
            remainder = sub_u256_le(remainder, divisor)?;
            set_bit(&mut quotient, bit);
        }
    }

    Some(quotient)
}

/// Bitcoin Core `GetBlockProof`: `(~target / (target + 1)) + 1`.
///
/// Reference: <https://github.com/bitcoin/bitcoin/blob/master/src/chain.cpp>
pub fn work_from_target(threshold: [u8; 32]) -> Option<[u8; 32]> {
    if threshold == [0u8; 32] {
        return None;
    }

    let target_plus_one = add_u256_le(threshold, 1)?;
    let inverted = not_u256_le(threshold);
    let quotient = div_u256_le(inverted, target_plus_one)?;
    add_u256_le(quotient, 1)
}

fn shl_one_u256_le(value: [u8; 32]) -> [u8; 32] {
    let mut carry = 0u8;
    let mut out = [0u8; 32];
    for index in 0..32 {
        let byte = value[index];
        out[index] = (byte << 1) | carry;
        carry = byte >> 7;
    }
    out
}

fn sub_u256_le(left: [u8; 32], right: [u8; 32]) -> Option<[u8; 32]> {
    let mut out = [0u8; 32];
    let mut borrow = 0u8;

    for index in 0..32 {
        let (diff, borrowed) = left[index].overflowing_sub(right[index] + borrow);
        out[index] = diff;
        borrow = if borrowed { 1 } else { 0 };
    }

    if borrow != 0 {
        None
    } else {
        Some(out)
    }
}

fn bit_is_set(value: [u8; 32], bit: usize) -> bool {
    let byte_index = bit / 8;
    let bit_index = bit % 8;
    value[byte_index] & (1 << bit_index) != 0
}

fn set_bit(value: &mut [u8; 32], bit: usize) {
    let byte_index = bit / 8;
    let bit_index = bit % 8;
    value[byte_index] |= 1 << bit_index;
}

#[cfg(test)]
mod tests {
    use super::{add_u256_le, cmp_le, div_u256_le, work_from_target};
    use std::cmp::Ordering;

    #[test]
    fn div_by_larger_returns_zero() {
        let mut small = [0u8; 32];
        small[0] = 1;
        let mut large = [0u8; 32];
        large[1] = 1;
        let q = div_u256_le(small, large).expect("div");
        assert_eq!(q, [0u8; 32]);
    }

    #[test]
    fn add_one_carries_across_bytes() {
        let mut v = [0u8; 32];
        v[0] = 0xff;
        let out = add_u256_le(v, 1).expect("add");
        assert_eq!(out[0], 0);
        assert_eq!(out[1], 1);
    }
}
