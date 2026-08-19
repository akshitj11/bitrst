//! Cross-check per-block work against Bitcoin Core's GetBlockProof formula.
//!
//! Reference: <https://github.com/bitcoin/bitcoin/blob/master/src/chain.cpp>

use bitrst_core::pow::Target;
use bitrst_core::uint256;

#[test]
fn genesis_bits_work_matches_formula() {
    let bits = 0x1d00_ffff_u32;
    let target = Target::from_bits(bits).expect("genesis bits decode");
    let from_target = target.to_work().expect("work");
    let direct = uint256::work_from_target(target.threshold()).expect("work");
    assert_eq!(from_target, direct);
}

#[test]
fn harder_bits_yield_greater_work() {
    let easy = Target::from_bits(0x1f00_ffff).expect("easy");
    let harder = Target::from_bits(0x1f00_ff00).expect("harder");
    let easy_work = easy.to_work().expect("easy work");
    let harder_work = harder.to_work().expect("harder work");
    assert!(uint256::cmp_le(&harder_work, &easy_work) == std::cmp::Ordering::Greater);
}
