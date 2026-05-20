use std::collections::HashMap;

use crate::Transaction;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutPoint {
    pub txid: [u8; 32],
    pub index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UtxoSet {
    pub entries: HashMap<OutPoint, u64>,
}

impl UtxoSet {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn apply_transaction(&mut self, tx: &Transaction) {
        for input in &tx.inputs {
            let outpoint = OutPoint {
                txid: input.previous_output,
                index: input.index,
            };
            self.entries.remove(&outpoint);
        }

        let txid = tx.txid();
        for (index, output) in tx.outputs.iter().enumerate() {
            self.entries.insert(
                OutPoint {
                    txid,
                    index: index as u32,
                },
                output.value,
            );
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::UtxoSet;
    use crate::Transaction;

    #[test]
    fn adds_outputs_from_coinbase() {
        let mut utxo = UtxoSet::new();
        let tx = Transaction::coinbase(1, 50_0000_0000);

        utxo.apply_transaction(&tx);

        assert_eq!(utxo.len(), 1);
    }
}
