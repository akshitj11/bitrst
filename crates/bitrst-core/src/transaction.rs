//! Bitcoin transaction primitives.

use bitrst_crypto::sha256d::sha256d;
use serde::{Deserialize, Serialize};

use crate::limits::{
    MAX_SCRIPT_SIZE, MAX_TRANSACTION_INPUTS, MAX_TRANSACTION_OUTPUTS,
    MAX_TRANSACTION_SERIALIZED_SIZE,
};
use crate::wire::{DecodeError, WireReader};

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

    /// Decodes one complete legacy (non-SegWit) Bitcoin transaction.
    ///
    /// Counts and scripts are bounded before allocation, non-canonical
    /// CompactSize values are rejected, and trailing bytes are not accepted.
    pub fn deserialize(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.len() > MAX_TRANSACTION_SERIALIZED_SIZE {
            return Err(DecodeError::LimitExceeded {
                context: "transaction size",
                actual: bytes.len() as u64,
                limit: MAX_TRANSACTION_SERIALIZED_SIZE,
            });
        }
        let mut reader = WireReader::new(bytes);
        let transaction = Self::decode_from(&mut reader)?;
        reader.finish("transaction")?;
        Ok(transaction)
    }

    pub(crate) fn decode_from(reader: &mut WireReader<'_>) -> Result<Self, DecodeError> {
        let version = reader.read_i32("transaction version")?;
        let input_count =
            reader.read_limited_len("transaction input count", MAX_TRANSACTION_INPUTS)?;
        let mut inputs = Vec::with_capacity(input_count);
        for _ in 0..input_count {
            let mut previous_output = [0; 32];
            previous_output.copy_from_slice(reader.read_bytes(32, "previous output hash")?);
            let index = reader.read_u32("previous output index")?;
            let script_len = reader.read_limited_len("scriptSig", MAX_SCRIPT_SIZE)?;
            let script_sig = reader.read_bytes(script_len, "scriptSig")?.to_vec();
            let sequence = reader.read_u32("input sequence")?;
            inputs.push(TxInput {
                previous_output,
                index,
                script_sig,
                sequence,
            });
        }
        let output_count =
            reader.read_limited_len("transaction output count", MAX_TRANSACTION_OUTPUTS)?;
        let mut outputs = Vec::with_capacity(output_count);
        for _ in 0..output_count {
            let value = reader.read_u64("output value")?;
            let script_len = reader.read_limited_len("scriptPubKey", MAX_SCRIPT_SIZE)?;
            let script_pubkey = reader.read_bytes(script_len, "scriptPubKey")?.to_vec();
            outputs.push(TxOutput {
                value,
                script_pubkey,
            });
        }
        let lock_time = reader.read_u32("transaction locktime")?;
        Ok(Self {
            version,
            inputs,
            outputs,
            lock_time,
        })
    }

    /// Returns the serialized transaction size in bytes (wire format).
    pub fn serialized_size(&self) -> usize {
        use crate::transaction::compact_size_encoded_len;

        4 + compact_size_encoded_len(self.inputs.len() as u64)
            + self
                .inputs
                .iter()
                .map(|input| {
                    32 + 4
                        + compact_size_encoded_len(input.script_sig.len() as u64)
                        + input.script_sig.len()
                        + 4
                })
                .sum::<usize>()
            + compact_size_encoded_len(self.outputs.len() as u64)
            + self
                .outputs
                .iter()
                .map(|output| {
                    8 + compact_size_encoded_len(output.script_pubkey.len() as u64)
                        + output.script_pubkey.len()
                })
                .sum::<usize>()
            + 4
    }

    /// Returns this transaction's SHA-256d transaction ID.
    pub fn txid(&self) -> [u8; 32] {
        sha256d(&self.serialize())
    }
}

/// Returns the byte length of a Bitcoin compact-size encoding for `value`.
pub(crate) fn compact_size_encoded_len(value: u64) -> usize {
    match value {
        0..=0xfc => 1,
        0xfd..=0xffff => 3,
        0x1_0000..=0xffff_ffff => 5,
        _ => 9,
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
    use crate::limits::{
        MAX_TRANSACTION_INPUTS, MAX_TRANSACTION_OUTPUTS, MAX_TRANSACTION_SERIALIZED_SIZE,
    };
    use crate::wire::DecodeError;

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

    #[test]
    fn legacy_transaction_roundtrips_wire_encoding() {
        let tx = Transaction::coinbase(1, 50_0000_0000);
        assert_eq!(Transaction::deserialize(&tx.serialize()), Ok(tx));
    }

    #[test]
    fn rejects_transaction_truncation_at_every_byte() {
        let encoded = Transaction::coinbase(1, 50_0000_0000).serialize();
        for length in 0..encoded.len() {
            assert!(
                Transaction::deserialize(&encoded[..length]).is_err(),
                "accepted prefix of length {length}"
            );
        }
    }

    #[test]
    fn rejects_transaction_trailing_bytes() {
        let mut encoded = Transaction::coinbase(1, 50_0000_0000).serialize();
        encoded.push(0);
        assert!(matches!(
            Transaction::deserialize(&encoded),
            Err(DecodeError::TrailingBytes { .. })
        ));
    }

    #[test]
    fn rejects_oversized_script_before_allocation() {
        let mut encoded = vec![1, 0, 0, 0, 1];
        encoded.extend_from_slice(&[0; 32]);
        encoded.extend_from_slice(&u32::MAX.to_le_bytes());
        encoded.extend_from_slice(&[0xfd, 0x11, 0x27]);
        assert!(matches!(
            Transaction::deserialize(&encoded),
            Err(DecodeError::LimitExceeded {
                context: "scriptSig",
                ..
            })
        ));
    }

    #[test]
    fn rejects_oversized_input_count_before_allocation() {
        let encoded = [1, 0, 0, 0, 0xfd, 0xa9, 0x61];
        assert!(matches!(
            Transaction::deserialize(&encoded),
            Err(DecodeError::LimitExceeded {
                context: "transaction input count",
                ..
            })
        ));
    }

    #[test]
    fn rejects_non_canonical_transaction_counts() {
        let encoded = [1, 0, 0, 0, 0xfd, 0, 0];
        assert!(matches!(
            Transaction::deserialize(&encoded),
            Err(DecodeError::NonCanonicalCompactSize { .. })
        ));
    }

    #[test]
    fn rejects_oversized_transaction_before_parsing() {
        let encoded = vec![0; MAX_TRANSACTION_SERIALIZED_SIZE + 1];
        assert!(matches!(
            Transaction::deserialize(&encoded),
            Err(DecodeError::LimitExceeded {
                context: "transaction size",
                ..
            })
        ));
    }

    #[test]
    fn rejects_oversized_output_count_before_allocation() {
        let mut encoded = vec![1, 0, 0, 0, 0];
        encoded.push(0xfd);
        encoded.extend_from_slice(&((MAX_TRANSACTION_OUTPUTS as u16) + 1).to_le_bytes());
        assert!(matches!(
            Transaction::deserialize(&encoded),
            Err(DecodeError::LimitExceeded {
                context: "transaction output count",
                limit: MAX_TRANSACTION_OUTPUTS,
                ..
            })
        ));
        assert_ne!(MAX_TRANSACTION_INPUTS, 0);
    }
}
