use bitrst_crypto::sha256d::sha256d;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxInput {
    pub previous_output: [u8; 32],
    pub index: u32,
    pub script_sig: Vec<u8>,
    pub sequence: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxOutput {
    pub value: u64,
    pub script_pubkey: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    pub version: i32,
    pub inputs: Vec<TxInput>,
    pub outputs: Vec<TxOutput>,
    pub lock_time: u32,
}

impl Transaction {
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

    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();

        out.extend_from_slice(&self.version.to_le_bytes());
        out.push(self.inputs.len() as u8);

        for input in &self.inputs {
            out.extend_from_slice(&input.previous_output);
            out.extend_from_slice(&input.index.to_le_bytes());
            out.push(input.script_sig.len() as u8);
            out.extend_from_slice(&input.script_sig);
            out.extend_from_slice(&input.sequence.to_le_bytes());
        }

        out.push(self.outputs.len() as u8);

        for output in &self.outputs {
            out.extend_from_slice(&output.value.to_le_bytes());
            out.push(output.script_pubkey.len() as u8);
            out.extend_from_slice(&output.script_pubkey);
        }

        out.extend_from_slice(&self.lock_time.to_le_bytes());
        out
    }

    pub fn txid(&self) -> [u8; 32] {
        sha256d(&self.serialize())
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
}
