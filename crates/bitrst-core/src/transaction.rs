//! Bitcoin transaction primitives.

use bitrst_crypto::sha256d::sha256d;
use serde::{Deserialize, Serialize};

/// A reference to a previous transaction output plus an unlocking script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxInput {
    /// Transaction ID being spent, stored in internal byte order.
    pub previous_output: [u8; 32],
    /// Output index within the previous transaction.
    pub index: u32,
    /// Unlocking script proving this input can spend the previous output.
    pub script_sig: Vec<u8>,
    /// Input sequence number.
    pub sequence: u32,
}

/// A transaction output containing value and locking script bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxOutput {
    /// Amount locked by this output, in satoshis.
    pub value: u64,
    /// Locking script that defines the spending conditions.
    pub script_pubkey: Vec<u8>,
}

/// A Bitcoin transaction with inputs, outputs, and lock-time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    /// Transaction version, serialized as a little-endian signed integer.
    pub version: i32,
    /// Inputs consumed by this transaction.
    pub inputs: Vec<TxInput>,
    /// Outputs created by this transaction.
    pub outputs: Vec<TxOutput>,
    /// Earliest time or block height when this transaction may be included.
    pub lock_time: u32,
}

impl Transaction {
    /// Builds a minimal coinbase-style transaction for a block reward.
    pub fn coinbase(height: u32, reward: u64) -> Self {
        Self {
            version: 1,
            inputs: vec![TxInput {
                previous_output: [0u8; 32],
                index: u32::MAX,
                script_sig: height.to_le_bytes().to_vec(),
                sequence: u32::MAX,
            }],
            outputs: vec![TxOutput {
                value: reward,
                script_pubkey: vec![],
            }],
            lock_time: 0,
        }
    }

    /// Serializes the transaction using Bitcoin's transaction wire format.
    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();

        out.extend_from_slice(&self.version.to_le_bytes());
        write_compact_size(self.inputs.len() as u64, &mut out);

        for input in &self.inputs {
            out.extend_from_slice(&input.previous_output);
            out.extend_from_slice(&input.index.to_le_bytes());
            write_compact_size(input.script_sig.len() as u64, &mut out);
            out.extend_from_slice(&input.script_sig);
            out.extend_from_slice(&input.sequence.to_le_bytes());
        }

        write_compact_size(self.outputs.len() as u64, &mut out);

        for output in &self.outputs {
            out.extend_from_slice(&output.value.to_le_bytes());
            write_compact_size(output.script_pubkey.len() as u64, &mut out);
            out.extend_from_slice(&output.script_pubkey);
        }

        out.extend_from_slice(&self.lock_time.to_le_bytes());
        out
    }

    /// Returns this transaction's SHA-256d transaction ID.
    pub fn txid(&self) -> [u8; 32] {
        sha256d(&self.serialize())
    }
}

/// Writes a Bitcoin compact-size prefix for a vector length or script length.
pub(crate) fn write_compact_size(value: u64, out: &mut Vec<u8>) {
    match value {
        0..=0xfc => out.push(value as u8),
        0xfd..=0xffff => {
            out.push(0xfd);
            out.extend_from_slice(&(value as u16).to_le_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(0xfe);
            out.extend_from_slice(&(value as u32).to_le_bytes());
        }
        _ => {
            out.push(0xff);
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Transaction, TxInput, TxOutput};

    #[test]
    fn serializes_and_hashes_coinbase_like_tx() {
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_output: [0u8; 32],
                index: u32::MAX,
                script_sig: vec![0x51],
                sequence: u32::MAX,
            }],
            outputs: vec![TxOutput {
                value: 50_0000_0000,
                script_pubkey: vec![0x51],
            }],
            lock_time: 0,
        };

        assert!(!tx.serialize().is_empty());
        assert_eq!(tx.txid().len(), 32);
    }

    #[test]
    fn builds_coinbase_tx() {
        let tx = Transaction::coinbase(1, 50_0000_0000);

        assert_eq!(tx.inputs.len(), 1);
        assert_eq!(tx.outputs.len(), 1);
        assert_eq!(tx.outputs[0].value, 50_0000_0000);
    }

    #[test]
    fn uses_compact_size_for_large_scripts() {
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_output: [0u8; 32],
                index: u32::MAX,
                script_sig: vec![0x51; 253],
                sequence: u32::MAX,
            }],
            outputs: vec![],
            lock_time: 0,
        };

        let serialized = tx.serialize();
        assert_eq!(&serialized[41..44], &[0xfd, 0xfd, 0x00]);
    }
}
