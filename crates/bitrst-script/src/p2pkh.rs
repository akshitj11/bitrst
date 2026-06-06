//! P2PKH script templates.

use crate::opcodes::{OP_CHECKSIG, OP_DUP, OP_EQUALVERIFY, OP_HASH160};

/// Encodes a push of `data` into script bytes.
pub fn push_data(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + data.len());
    match data.len() {
        0 => out.push(0x00),
        1..=75 => {
            out.push(data.len() as u8);
            out.extend_from_slice(data);
        }
        76..=255 => {
            out.push(0x4c);
            out.push(data.len() as u8);
            out.extend_from_slice(data);
        }
        256..=65535 => {
            out.push(0x4d);
            out.extend_from_slice(&(data.len() as u16).to_le_bytes());
            out.extend_from_slice(data);
        }
        _ => {
            out.push(0x4e);
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(data);
        }
    }
    out
}

/// Builds a standard P2PKH `scriptPubKey` for a 20-byte pubkey hash.
pub fn p2pkh_script_pubkey(pubkey_hash: &[u8; 20]) -> Vec<u8> {
    let mut script = Vec::with_capacity(25);
    script.push(OP_DUP);
    script.push(OP_HASH160);
    script.push(20);
    script.extend_from_slice(pubkey_hash);
    script.push(OP_EQUALVERIFY);
    script.push(OP_CHECKSIG);
    script
}

/// Builds a P2PKH `scriptSig` from signature (with sighash byte) and pubkey.
pub fn p2pkh_script_sig(signature_with_hashtype: &[u8], pubkey: &[u8]) -> Vec<u8> {
    let mut script = Vec::new();
    script.extend(push_data(signature_with_hashtype));
    script.extend(push_data(pubkey));
    script
}
