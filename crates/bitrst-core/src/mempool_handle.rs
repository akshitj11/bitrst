//! Thread-safe access to the transaction mempool.

use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::chain::Chain;
use crate::chain_events::ChainEvent;
use crate::mempool::{AcceptedTx, Mempool, MempoolError, MempoolLimits};
use crate::transaction::Transaction;
use crate::utxo::UtxoSet;

/// Errors from acquiring the mempool lock or admitting transactions.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MempoolHandleError {
    /// Another thread panicked while holding the lock.
    #[error("mempool lock poisoned")]
    LockPoisoned,

    /// Admission failed validation or policy checks.
    #[error(transparent)]
    Admission(#[from] MempoolError),
}

/// Shared handle to a [`Mempool`] for concurrent readers and exclusive writers.
#[derive(Debug, Clone)]
pub struct MempoolHandle {
    inner: Arc<RwLock<Mempool>>,
}

impl MempoolHandle {
    /// Wraps a new mempool with default limits.
    pub fn new() -> Self {
        Self::with_limits(MempoolLimits::default())
    }

    /// Wraps a new mempool with explicit limits.
    pub fn with_limits(limits: MempoolLimits) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Mempool::new(limits))),
        }
    }

    /// Wraps an existing mempool instance.
    pub fn from_mempool(mempool: Mempool) -> Self {
        Self {
            inner: Arc::new(RwLock::new(mempool)),
        }
    }

    /// Returns the number of transactions in the pool.
    pub fn len(&self) -> Result<usize, MempoolHandleError> {
        Ok(self.read()?.len())
    }

    /// Returns true when the pool is empty.
    pub fn is_empty(&self) -> Result<bool, MempoolHandleError> {
        Ok(self.read()?.is_empty())
    }

    /// Returns transaction IDs currently in the pool.
    pub fn txids(&self) -> Result<Vec<[u8; 32]>, MempoolHandleError> {
        Ok(self.read()?.txids())
    }

    /// Returns true when `txid` is present.
    pub fn contains(&self, txid: &[u8; 32]) -> Result<bool, MempoolHandleError> {
        Ok(self.read()?.contains(txid))
    }

    /// Returns a cloned transaction when `txid` is in the pool.
    pub fn get_transaction(
        &self,
        txid: &[u8; 32],
    ) -> Result<Option<Transaction>, MempoolHandleError> {
        Ok(self.read()?.get_transaction(txid))
    }

    /// Validates and admits a transaction against the active chain UTXO set.
    pub fn accept_tx(
        &self,
        tx: Transaction,
        chain_utxo: &UtxoSet,
    ) -> Result<AcceptedTx, MempoolHandleError> {
        self.write()?
            .accept_tx(tx, chain_utxo)
            .map_err(MempoolHandleError::from)
    }

    /// Applies chain events to keep the pool consistent with the active chain.
    pub fn apply_chain_events(
        &self,
        events: &[ChainEvent],
        chain: &Chain,
    ) -> Result<(), MempoolHandleError> {
        self.write()?.apply_chain_events(events, chain);
        Ok(())
    }

    fn read(&self) -> Result<RwLockReadGuard<'_, Mempool>, MempoolHandleError> {
        self.inner
            .read()
            .map_err(|_| MempoolHandleError::LockPoisoned)
    }

    fn write(&self) -> Result<RwLockWriteGuard<'_, Mempool>, MempoolHandleError> {
        self.inner
            .write()
            .map_err(|_| MempoolHandleError::LockPoisoned)
    }
}

impl Default for MempoolHandle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{MempoolHandle, MempoolHandleError};
    use crate::block::BlockHeader;
    use crate::chain::Chain;
    use crate::mempool::MempoolLimits;
    use crate::pow::Target;
    use crate::transaction::{Transaction, TxInput, TxOutput};

    const TEST_BITS: u32 = 0x1f00_ffff;
    const NETWORK_TIME: u32 = 1_231_006_505;

    #[test]
    fn concurrent_reads_and_exclusive_accept() {
        let genesis_header = BlockHeader {
            version: 1,
            prev_blockhash: [0u8; 32],
            merkle_root: [0u8; 32],
            time: NETWORK_TIME,
            bits: TEST_BITS,
            nonce: 0,
        };
        let mut genesis = crate::Block::coinbase(genesis_header, 0, 50_0000_0000);
        let target = Target::from_bits(TEST_BITS).expect("bits");
        while !target.meets(&genesis.header.hash()) {
            genesis.header.nonce = genesis.header.nonce.wrapping_add(1);
        }
        let chain = Chain::new_genesis(genesis, NETWORK_TIME).expect("genesis");
        let handle = MempoolHandle::with_limits(MempoolLimits {
            max_tx_count: 10,
            max_bytes: 1_000_000,
        });

        assert!(handle.is_empty().expect("empty"));
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_output: [1u8; 32],
                index: 0,
                script_sig: vec![],
                sequence: u32::MAX,
            }],
            outputs: vec![TxOutput {
                value: 1,
                script_pubkey: vec![],
            }],
            lock_time: 0,
        };
        assert!(matches!(
            handle.accept_tx(tx, chain.utxo()),
            Err(MempoolHandleError::Admission(_))
        ));
        assert_eq!(handle.len().expect("len"), 0);
    }
}
