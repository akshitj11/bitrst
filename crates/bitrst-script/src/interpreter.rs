//! Script interpreter for P2PKH verification.

use bitrst_crypto::ecdsa::verify_der_signature;
use bitrst_crypto::hash160::hash160;
use thiserror::Error;

use crate::opcodes::{
    OP_CHECKSIG, OP_DUP, OP_EQUAL, OP_EQUALVERIFY, OP_HASH160, OP_PUSHDATA1, OP_PUSHDATA2,
    OP_PUSHDATA4,
};
use crate::stack::Stack;

/// Errors raised while executing or verifying scripts.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ScriptError {
    /// Script ended before all opcodes were consumed.
    #[error("unexpected end of script")]
    UnexpectedEnd,

    /// An opcode required a stack item that was not present.
    #[error("stack underflow")]
    StackUnderflow,

    /// An unknown opcode was encountered.
    #[error("unknown opcode {0:#04x}")]
    UnknownOpcode(u8),

    /// `OP_EQUALVERIFY` failed.
    #[error("equal verify failed")]
    EqualVerifyFailed,

    /// `OP_CHECKSIG` failed.
    #[error("checksig failed")]
    CheckSigFailed,

    /// Script finished without a true value on the stack.
    #[error("script evaluated to false")]
    EvalFalse,
}

/// Verifies `script_sig` and `script_pubkey` against a precomputed sighash.
pub fn verify_script(
    script_sig: &[u8],
    script_pubkey: &[u8],
    sighash: &[u8; 32],
) -> Result<(), ScriptError> {
    let mut stack = Stack::new();
    execute_script(script_sig, &mut stack, sighash)?;
    execute_script(script_pubkey, &mut stack, sighash)?;
    if stack.top_is_true()? {
        Ok(())
    } else {
        Err(ScriptError::EvalFalse)
    }
}

fn execute_script(script: &[u8], stack: &mut Stack, sighash: &[u8; 32]) -> Result<(), ScriptError> {
    let mut pc = 0usize;
    while pc < script.len() {
        let opcode = script[pc];
        pc += 1;

        if (1..=75).contains(&opcode) {
            let len = opcode as usize;
            let data = read_push(script, &mut pc, len)?;
            stack.push(data);
            continue;
        }

        match opcode {
            OP_PUSHDATA1 => {
                let len = read_u8(script, &mut pc)? as usize;
                stack.push(read_push(script, &mut pc, len)?);
            }
            OP_PUSHDATA2 => {
                let len = read_u16(script, &mut pc)? as usize;
                stack.push(read_push(script, &mut pc, len)?);
            }
            OP_PUSHDATA4 => {
                let len = read_u32(script, &mut pc)? as usize;
                stack.push(read_push(script, &mut pc, len)?);
            }
            OP_DUP => stack.dup()?,
            OP_HASH160 => {
                let item = stack.pop()?;
                stack.push(hash160(&item).to_vec());
            }
            OP_EQUAL => {
                let right = stack.pop()?;
                let left = stack.pop()?;
                stack.push(if left == right { vec![1] } else { Vec::new() });
            }
            OP_EQUALVERIFY => {
                let right = stack.pop()?;
                let left = stack.pop()?;
                if left != right {
                    return Err(ScriptError::EqualVerifyFailed);
                }
            }
            OP_CHECKSIG => {
                let pubkey = stack.pop()?;
                let signature = stack.pop()?;
                verify_der_signature(&signature, &pubkey, sighash)
                    .map_err(|_| ScriptError::CheckSigFailed)?;
                stack.push(vec![1]);
            }
            0x00 => stack.push(Vec::new()),
            0x51 => stack.push(vec![1]),
            _ => return Err(ScriptError::UnknownOpcode(opcode)),
        }
    }
    Ok(())
}

fn read_push(script: &[u8], pc: &mut usize, len: usize) -> Result<Vec<u8>, ScriptError> {
    let end = pc
        .checked_add(len)
        .filter(|end| *end <= script.len())
        .ok_or(ScriptError::UnexpectedEnd)?;
    let data = script[*pc..end].to_vec();
    *pc = end;
    Ok(data)
}

fn read_u8(script: &[u8], pc: &mut usize) -> Result<u8, ScriptError> {
    if *pc >= script.len() {
        return Err(ScriptError::UnexpectedEnd);
    }
    let value = script[*pc];
    *pc += 1;
    Ok(value)
}

fn read_u16(script: &[u8], pc: &mut usize) -> Result<u16, ScriptError> {
    if *pc + 2 > script.len() {
        return Err(ScriptError::UnexpectedEnd);
    }
    let value = u16::from_le_bytes([script[*pc], script[*pc + 1]]);
    *pc += 2;
    Ok(value)
}

fn read_u32(script: &[u8], pc: &mut usize) -> Result<u32, ScriptError> {
    if *pc + 4 > script.len() {
        return Err(ScriptError::UnexpectedEnd);
    }
    let value = u32::from_le_bytes([
        script[*pc],
        script[*pc + 1],
        script[*pc + 2],
        script[*pc + 3],
    ]);
    *pc += 4;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::verify_script;
    use crate::interpreter::ScriptError;
    use crate::p2pkh::{p2pkh_script_pubkey, p2pkh_script_sig};
    use bitrst_crypto::hash160::hash160;
    use secp256k1::{Message, Secp256k1, SecretKey};

    #[test]
    fn p2pkh_script_validates_known_signature() {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[0x11; 32]).expect("secret key");
        let pk = secp256k1::PublicKey::from_secret_key(&secp, &sk);
        let pubkey_bytes = pk.serialize();
        let pubkey_hash = hash160(&pubkey_bytes);
        let script_pubkey = p2pkh_script_pubkey(&pubkey_hash);

        let sighash = [0x42; 32];
        let message = Message::from_digest(sighash);
        let sig = secp.sign_ecdsa(&message, &sk);
        let mut sig_bytes = sig.serialize_der().to_vec();
        sig_bytes.push(0x01);
        let script_sig = p2pkh_script_sig(&sig_bytes, &pubkey_bytes);

        verify_script(&script_sig, &script_pubkey, &sighash).expect("valid p2pkh");
    }

    #[test]
    fn p2pkh_rejects_bad_signature() {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[0x22; 32]).expect("secret key");
        let pk = secp256k1::PublicKey::from_secret_key(&secp, &sk);
        let pubkey_bytes = pk.serialize();
        let pubkey_hash = hash160(&pubkey_bytes);
        let script_pubkey = p2pkh_script_pubkey(&pubkey_hash);

        let sighash = [0x99; 32];
        let message = Message::from_digest(sighash);
        let sig = secp.sign_ecdsa(&message, &sk);
        let mut sig_bytes = sig.serialize_der().to_vec();
        sig_bytes.push(0x01);
        sig_bytes[5] ^= 0xff;
        let script_sig = p2pkh_script_sig(&sig_bytes, &pubkey_bytes);

        assert_eq!(
            verify_script(&script_sig, &script_pubkey, &sighash),
            Err(ScriptError::CheckSigFailed)
        );
    }
}
