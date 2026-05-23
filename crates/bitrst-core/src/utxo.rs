//! UTXO set primitives.

use std::collections::HashMap;

use crate::transaction::Transaction;
use thiserror::Error;

/// Identifies one spendable output from a previous transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutPoint {
    /// Transaction ID containing the output, stored in internal byte order.
    pub txid: [u8; 32],
    /// Output index inside the transaction.
    pub index: u32,
}

/// Undo data for reverting a single transaction on the UTXO set during reorg.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxUndo {
    /// Outputs removed from the UTXO set because this transaction spent them.
    pub removed: Vec<(OutPoint, u64)>,
    /// Outputs created by this transaction that must be removed on disconnect.
    pub created: Vec<OutPoint>,
}

/// Errors raised while validating transactions against the UTXO set.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum UtxoError {
    /// A transaction input referenced an outpoint that is not in the UTXO set.
    #[error("missing UTXO input {txid:?} index {index}")]
    MissingInput {
        /// Transaction ID of the missing output.
        txid: [u8; 32],
        /// Output index of the missing output.
        index: u32,
    },

    /// A transaction output had a negative value.
    #[error("transaction output value must not be negative")]
    NegativeOutputValue,

    /// Satoshi arithmetic overflowed while summing inputs or outputs.
    #[error("transaction value arithmetic overflowed")]
    ValueOverflow,

    /// Total output value exceeded total input value.
    #[error("transaction outputs exceed inputs: inputs={inputs} outputs={outputs}")]
    InputOutputMismatch {
        /// Total satoshis from inputs.
        inputs: u64,
        /// Total satoshis sent to outputs.
        outputs: u64,
    },

    /// The same outpoint was spent twice in one block.
    #[error("duplicate spend of UTXO {txid:?} index {index}")]
    DuplicateSpend {
        /// Transaction ID of the double-spent output.
        txid: [u8; 32],
        /// Output index of the double-spent output.
        index: u32,
    },
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

    /// Returns true when the outpoint is currently unspent.
    pub fn contains(&self, outpoint: &OutPoint) -> bool {
        self.entries.contains_key(outpoint)
    }

    /// Returns the satoshi value locked at an outpoint, if it is unspent.
    pub fn get(&self, outpoint: &OutPoint) -> Option<u64> {
        self.entries.get(outpoint).copied()
    }

    /// Returns true when the transaction is a coinbase reward transaction.
    pub fn is_coinbase(tx: &Transaction) -> bool {
        tx.inputs.len() == 1
            && tx.inputs[0].previous_output == [0u8; 32]
            && tx.inputs[0].index == u32::MAX
    }

    /// Validates that a non-coinbase transaction only spends existing UTXOs and
    /// does not create more value than it consumes.
    ///
    /// Coinbase transactions skip input checks because they mint new coins.
    ///
    /// # Errors
    ///
    /// Returns [`UtxoError`] when inputs are missing, values overflow, or outputs
    /// exceed inputs.
    pub fn validate_transaction(&self, tx: &Transaction) -> Result<(), UtxoError> {
        if Self::is_coinbase(tx) {
            return Self::validate_output_sum(tx);
        }

        let mut sum_in = 0u64;
        for input in &tx.inputs {
            let outpoint = OutPoint {
                txid: input.previous_output,
                index: input.index,
            };
            let value = self.get(&outpoint).ok_or(UtxoError::MissingInput {
                txid: outpoint.txid,
                index: outpoint.index,
            })?;
            sum_in = sum_in.checked_add(value).ok_or(UtxoError::ValueOverflow)?;
        }

        let sum_out = Self::output_sum(tx)?;

        if sum_in < sum_out {
            return Err(UtxoError::InputOutputMismatch {
                inputs: sum_in,
                outputs: sum_out,
            });
        }

        Ok(())
    }

    /// Applies a transaction to the UTXO set and returns undo data for reorg.
    ///
    /// Call [`Self::validate_transaction`] before this function.
    pub fn apply_transaction(&mut self, tx: &Transaction) -> TxUndo {
        let mut removed = Vec::new();

        if !Self::is_coinbase(tx) {
            for input in &tx.inputs {
                let outpoint = OutPoint {
                    txid: input.previous_output,
                    index: input.index,
                };
                if let Some(value) = self.entries.remove(&outpoint) {
                    removed.push((outpoint, value));
                }
            }
        }

        let txid = tx.txid();
        let mut created = Vec::new();
        for (index, output) in tx.outputs.iter().enumerate() {
            let outpoint = OutPoint {
                txid,
                index: index as u32,
            };
            self.entries.insert(outpoint, output.value);
            created.push(outpoint);
        }

        TxUndo { removed, created }
    }

    /// Reverts a transaction using undo data produced by [`Self::apply_transaction`].
    pub fn disconnect_undo(&mut self, undo: &TxUndo) {
        for outpoint in &undo.created {
            self.entries.remove(outpoint);
        }
        for (outpoint, value) in &undo.removed {
            self.entries.insert(*outpoint, *value);
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

    fn validate_output_sum(tx: &Transaction) -> Result<(), UtxoError> {
        Self::output_sum(tx)?;
        Ok(())
    }

    fn output_sum(tx: &Transaction) -> Result<u64, UtxoError> {
        let mut sum_out = 0u64;
        for output in &tx.outputs {
            sum_out = sum_out
                .checked_add(output.value)
                .ok_or(UtxoError::ValueOverflow)?;
        }
        Ok(sum_out)
    }
}

impl Default for UtxoSet {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{UtxoError, UtxoSet};
    use crate::Transaction;

    #[test]
    fn adds_outputs_from_coinbase() {
        let mut utxo = UtxoSet::new();
        let tx = Transaction::coinbase(1, 50_0000_0000);

        utxo.apply_transaction(&tx);

        assert_eq!(utxo.len(), 1);
    }

    #[test]
    fn rejects_spending_missing_inputs() {
        let utxo = UtxoSet::new();
        let tx = Transaction {
            version: 1,
            inputs: vec![crate::TxInput {
                previous_output: [1u8; 32],
                index: 0,
                script_sig: vec![],
                sequence: u32::MAX,
            }],
            outputs: vec![crate::TxOutput {
                value: 1,
                script_pubkey: vec![],
            }],
            lock_time: 0,
        };

        assert!(matches!(
            utxo.validate_transaction(&tx),
            Err(UtxoError::MissingInput { .. })
        ));
    }

    #[test]
    fn rejects_outputs_exceeding_inputs() {
        let mut utxo = UtxoSet::new();
        let funding = Transaction::coinbase(0, 10);
        let funding_txid = funding.txid();
        utxo.apply_transaction(&funding);

        let spend = Transaction {
            version: 1,
            inputs: vec![crate::TxInput {
                previous_output: funding_txid,
                index: 0,
                script_sig: vec![],
                sequence: u32::MAX,
            }],
            outputs: vec![crate::TxOutput {
                value: 11,
                script_pubkey: vec![],
            }],
            lock_time: 0,
        };

        assert!(matches!(
            utxo.validate_transaction(&spend),
            Err(UtxoError::InputOutputMismatch { .. })
        ));
    }

    #[test]
    fn disconnect_undo_restores_prior_state() {
        let mut utxo = UtxoSet::new();
        let coinbase = Transaction::coinbase(0, 50_0000_0000);
        let undo = utxo.apply_transaction(&coinbase);
        assert_eq!(utxo.len(), 1);

        utxo.disconnect_undo(&undo);
        assert!(utxo.is_empty());
    }
}
