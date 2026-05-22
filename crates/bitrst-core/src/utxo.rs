//! UTXO set primitives.

use std::collections::HashMap;

use crate::Transaction;

/// Identifies one spendable output from a previous transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutPoint {
    /// Transaction ID containing the output, stored in internal byte order.
    pub txid: [u8; 32],
    /// Output index inside the transaction.
    pub index: u32,
}

/// In-memory set of currently unspent transaction outputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UtxoSet {
    /// Map from outpoints to their spendable value in satoshis.
    pub entries: HashMap<OutPoint, u64>,
}

impl UtxoSet {
    /// Creates an empty UTXO set.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Removes spent inputs and inserts all outputs created by a transaction.
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

    /// Returns the number of unspent outputs currently tracked.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true when the UTXO set has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for UtxoSet {
    fn default() -> Self {
        Self::new()
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
