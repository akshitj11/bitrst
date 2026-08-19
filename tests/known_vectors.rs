//! Authoritative legacy Bitcoin vectors for implemented features.
//!
//! Sources are limited to Bitcoin Core test data and stable public documentation.
//! SegWit (BIP141) and BIP143 sighash vectors are intentionally omitted.

use bitrst_core::pow::Target;
use bitrst_core::{block_work, uint256};
use bitrst_crypto::base58;
use bitrst_wallet::{Address, Network};

/// Bitcoin Wiki P2PKH example payload (version `0x00`).
/// Reference: <https://en.bitcoin.it/wiki/Base58Check_encoding>
const WIKI_P2PKH_ADDRESS: &str = "1LEvUuseTCgKTPfqB1d9xWUqJRZuxDhnCA";
const WIKI_P2PKH_HASH: [u8; 20] = [
    0xd3, 0x0c, 0x70, 0xf7, 0xd1, 0xe2, 0x08, 0x12, 0x0e, 0x1e, 0x5e, 0x55, 0xb5, 0x34, 0x1f, 0xa3,
    0x21, 0xa6, 0x0f, 0xf2,
];

/// Bitcoin Core `src/test/data/key_io_valid.json` mainnet P2PKH entry.
/// Reference: <https://github.com/bitcoin/bitcoin/blob/master/src/test/data/key_io_valid.json>
const CORE_MAINNET_P2PKH: &str = "1FsSia9rv4NeEwvJ2GvXrX7LyxYspbN2mo";
const CORE_MAINNET_P2PKH_HASH: [u8; 20] = [
    0xa3, 0x1c, 0x06, 0xbd, 0x46, 0x3e, 0x39, 0x23, 0xbc, 0x1a, 0xad, 0xbd, 0xe4, 0x8b, 0x16, 0x97,
    0x6c, 0x08, 0x07, 0x17,
];

/// Genesis compact `bits` from the mainnet block 0 header.
/// Reference: <https://en.bitcoin.it/wiki/Block_hashing_algorithm>
const GENESIS_BITS: u32 = 0x1d00_ffff;

/// Genesis per-block work from Bitcoin Core `GetBlockProof`.
/// Reference: <https://github.com/bitcoin/bitcoin/blob/master/src/chain.cpp>
const GENESIS_BLOCK_WORK: [u8; 32] = [
    0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

#[test]
fn wiki_p2pkh_address_roundtrips_base58check() {
    assert_eq!(
        base58::encode_check(0x00, &WIKI_P2PKH_HASH),
        WIKI_P2PKH_ADDRESS
    );
    let (version, payload) =
        base58::decode_check(WIKI_P2PKH_ADDRESS).expect("wiki p2pkh should decode");
    assert_eq!(version, 0x00);
    assert_eq!(payload, WIKI_P2PKH_HASH);
}

#[test]
fn mainnet_p2pkh_address_matches_bitcoin_core_vector() {
    let address = Address::p2pkh(CORE_MAINNET_P2PKH_HASH, Network::Mainnet);

    assert_eq!(address.to_string(), CORE_MAINNET_P2PKH);

    let (version, payload) =
        base58::decode_check(CORE_MAINNET_P2PKH).expect("core p2pkh should decode");
    assert_eq!(version, 0x00);
    assert_eq!(payload, CORE_MAINNET_P2PKH_HASH);
}

#[test]
fn genesis_compact_bits_decode_matches_wiki_target() {
    let target = Target::from_bits(GENESIS_BITS).expect("genesis bits should decode");
    let mut expected = [0u8; 32];
    expected[26] = 0xff;
    expected[27] = 0xff;

    assert_eq!(target.threshold(), expected);
    assert_eq!(target.to_bits(), Some(GENESIS_BITS));
}

#[test]
fn genesis_block_work_matches_bitcoin_core_getblockproof() {
    let target = Target::from_bits(GENESIS_BITS).expect("genesis bits should decode");
    let from_target = target.to_work().expect("work from target");
    let from_bits = block_work(GENESIS_BITS)
        .expect("block_work should succeed")
        .0;

    assert_eq!(from_target, GENESIS_BLOCK_WORK);
    assert_eq!(from_bits, GENESIS_BLOCK_WORK);
    assert_eq!(
        uint256::work_from_target(target.threshold()).expect("direct work"),
        GENESIS_BLOCK_WORK
    );
}
