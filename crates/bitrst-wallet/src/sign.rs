//! P2PKH transaction signing helpers.

use bitrst_core::{sighash_all, Transaction, SIGHASH_ALL};
use bitrst_crypto::ecdsa::sign_der_with_hashtype;
use bitrst_script::p2pkh_script_sig;

use crate::{PrivateKey, WalletError};

/// Signs one P2PKH transaction input and writes its `scriptSig`.
///
/// # Errors
///
/// Returns [`WalletError`] if the input index or previous script list is invalid.
pub fn sign_p2pkh_input(
    tx: &mut Transaction,
    input_index: usize,
    prev_script_pubkeys: &[Vec<u8>],
    key: &PrivateKey,
) -> Result<(), WalletError> {
    if input_index >= tx.inputs.len() {
        return Err(WalletError::InputIndexOutOfRange);
    }

    let sighash = sighash_all(tx, input_index, prev_script_pubkeys)?;
    let signature = sign_der_with_hashtype(key.secret_key(), &sighash, SIGHASH_ALL as u8);
    tx.inputs[input_index].script_sig = p2pkh_script_sig(&signature, &key.public_key());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::sign_p2pkh_input;
    use crate::{Address, Network, PrivateKey};
    use bitrst_core::{sighash_all, Transaction, TxInput, TxOutput};
    use bitrst_script::{p2pkh_script_pubkey, verify_script};

    #[test]
    fn sign_p2pkh_input_builds_script_sig_that_verifies() {
        let key = PrivateKey::from_bytes([
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 1,
        ])
        .expect("valid key");
        let address = Address::p2pkh(key.pubkey_hash(), Network::Mainnet);
        let prev_script = p2pkh_script_pubkey(&address.pubkey_hash());
        let prev_scripts = vec![prev_script.clone()];
        let mut tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_output: [0x42; 32],
                index: 0,
                script_sig: Vec::new(),
                sequence: u32::MAX,
            }],
            outputs: vec![TxOutput {
                value: 49_0000_0000,
                script_pubkey: vec![0x51],
            }],
            lock_time: 0,
        };

        sign_p2pkh_input(&mut tx, 0, &prev_scripts, &key).expect("sign");

        let sighash = sighash_all(&tx, 0, &prev_scripts).expect("sighash");
        verify_script(&tx.inputs[0].script_sig, &prev_script, &sighash).expect("valid script");
    }
}
