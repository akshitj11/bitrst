//! Legacy Bitcoin sighash (pre-SegWit) for script verification.

use bitrst_crypto::sha256d::sha256d;

use crate::transaction::{write_compact_size, Transaction};

/// Sighash type flag: sign all inputs and outputs.
pub const SIGHASH_ALL: u32 = 1;

/// Computes legacy `SIGHASH_ALL` for `input_index` using the spent output scripts.
///
/// `prev_script_pubkeys` must have one entry per input (the `scriptPubKey` of each spent output).
pub fn sighash_all(
    tx: &Transaction,
    input_index: usize,
    prev_script_pubkeys: &[Vec<u8>],
) -> Result<[u8; 32], SighashError> {
    if input_index >= tx.inputs.len() {
        return Err(SighashError::InputIndexOutOfRange);
    }
    if prev_script_pubkeys.len() != tx.inputs.len() {
        return Err(SighashError::PrevScriptCountMismatch);
    }

    let mut data = Vec::new();
    data.extend_from_slice(&tx.version.to_le_bytes());
    write_compact_size(tx.inputs.len() as u64, &mut data);

    for (i, input) in tx.inputs.iter().enumerate() {
        data.extend_from_slice(&input.previous_output);
        data.extend_from_slice(&input.index.to_le_bytes());
        let script = if i == input_index {
            &prev_script_pubkeys[i]
        } else {
            &[][..]
        };
        write_compact_size(script.len() as u64, &mut data);
        data.extend_from_slice(script);
        data.extend_from_slice(&input.sequence.to_le_bytes());
    }

    write_compact_size(tx.outputs.len() as u64, &mut data);
    for output in &tx.outputs {
        data.extend_from_slice(&output.value.to_le_bytes());
        write_compact_size(output.script_pubkey.len() as u64, &mut data);
        data.extend_from_slice(&output.script_pubkey);
    }

    data.extend_from_slice(&tx.lock_time.to_le_bytes());
    data.extend_from_slice(&SIGHASH_ALL.to_le_bytes());
    Ok(sha256d(&data))
}

/// Errors while computing sighash.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SighashError {
    /// The input index is out of range for the transaction.
    #[error("input index out of range")]
    InputIndexOutOfRange,
    /// `prev_script_pubkeys` length did not match input count.
    #[error("previous script pubkey count mismatch")]
    PrevScriptCountMismatch,
}

#[cfg(test)]
mod tests {
    use super::{sighash_all, SIGHASH_ALL};
    use crate::{Transaction, TxInput, TxOutput};
    use bitrst_crypto::sha256d::sha256d;

    #[test]
    fn sighash_all_matches_hand_computed_fixture() {
        let funding_script = {
            let mut s = vec![0x76, 0xa9, 0x14];
            s.extend_from_slice(&[0xab; 20]);
            s.extend_from_slice(&[0x88, 0xac]);
            s
        };

        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_output: [0x01; 32],
                index: 0,
                script_sig: vec![0x51],
                sequence: 0xffff_ffff,
            }],
            outputs: vec![TxOutput {
                value: 49_0000_0000,
                script_pubkey: vec![0x51],
            }],
            lock_time: 0,
        };

        let digest = sighash_all(&tx, 0, std::slice::from_ref(&funding_script)).expect("sighash");
        let mut manual = Vec::new();
        manual.extend_from_slice(&1i32.to_le_bytes());
        manual.push(1);
        manual.extend_from_slice(&[0x01; 32]);
        manual.extend_from_slice(&0u32.to_le_bytes());
        manual.push(funding_script.len() as u8);
        manual.extend_from_slice(&funding_script);
        manual.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
        manual.push(1);
        manual.extend_from_slice(&49_0000_0000u64.to_le_bytes());
        manual.push(1);
        manual.push(0x51);
        manual.extend_from_slice(&0u32.to_le_bytes());
        manual.extend_from_slice(&SIGHASH_ALL.to_le_bytes());
        assert_eq!(digest, sha256d(&manual));
    }
}
