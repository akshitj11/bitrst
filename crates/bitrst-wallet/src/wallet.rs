//! Active-chain wallet UTXO tracking.

use std::collections::{HashMap, HashSet};

use bitrst_core::{Chain, ChainEvent, OutPoint};

use crate::{Address, WalletError};

/// A spendable output controlled by this wallet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletUtxo {
    /// Outpoint that identifies this UTXO.
    pub outpoint: OutPoint,
    /// Satoshis locked by this output.
    pub value: u64,
    /// Locking script for this output.
    pub script_pubkey: Vec<u8>,
}

/// Tracks active-chain UTXOs for watched P2PKH addresses.
#[derive(Debug, Default, Clone)]
pub struct Wallet {
    watched: HashSet<[u8; 20]>,
    utxos: HashMap<OutPoint, WalletUtxo>,
    block_undo: HashMap<[u8; 32], WalletBlockUndo>,
}

#[derive(Debug, Default, Clone)]
struct WalletBlockUndo {
    created: Vec<OutPoint>,
    removed: Vec<WalletUtxo>,
}

impl Wallet {
    /// Creates an empty wallet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a P2PKH address to the watch list.
    pub fn watch_address(&mut self, address: Address) {
        self.watched.insert(address.pubkey_hash());
    }

    /// Applies active-chain events to the wallet's UTXO view.
    ///
    /// # Errors
    ///
    /// Returns [`WalletError::MissingActiveBlock`] if a connect event references a missing block.
    pub fn apply_events(
        &mut self,
        events: &[ChainEvent],
        chain: &Chain,
    ) -> Result<(), WalletError> {
        for event in events {
            match *event {
                ChainEvent::BlockConnected { height, .. } => {
                    let block = chain
                        .active_block_at(height)
                        .ok_or(WalletError::MissingActiveBlock(height))?;
                    let mut undo = WalletBlockUndo::default();

                    for tx in &block.transactions {
                        for input in &tx.inputs {
                            let outpoint = OutPoint {
                                txid: input.previous_output,
                                index: input.index,
                            };
                            if let Some(utxo) = self.utxos.remove(&outpoint) {
                                undo.removed.push(utxo);
                            }
                        }

                        let txid = tx.txid();
                        for (index, output) in tx.outputs.iter().enumerate() {
                            if let Some(pubkey_hash) = p2pkh_pubkey_hash(&output.script_pubkey) {
                                if self.watched.contains(&pubkey_hash) {
                                    let outpoint = OutPoint {
                                        txid,
                                        index: index as u32,
                                    };
                                    self.utxos.insert(
                                        outpoint,
                                        WalletUtxo {
                                            outpoint,
                                            value: output.value,
                                            script_pubkey: output.script_pubkey.clone(),
                                        },
                                    );
                                    undo.created.push(outpoint);
                                }
                            }
                        }
                    }
                    self.block_undo.insert(block.hash(), undo);
                }
                ChainEvent::BlockDisconnected { hash, .. } => {
                    if let Some(undo) = self.block_undo.remove(&hash) {
                        for outpoint in undo.created {
                            self.utxos.remove(&outpoint);
                        }
                        for utxo in undo.removed {
                            self.utxos.insert(utxo.outpoint, utxo);
                        }
                    }
                }
                ChainEvent::ChainReorg { .. }
                | ChainEvent::OrphanAdded { .. }
                | ChainEvent::OrphanEvicted { .. } => {}
            }
        }

        Ok(())
    }

    /// Returns total spendable satoshis currently tracked by the wallet.
    pub fn balance(&self) -> u64 {
        self.utxos.values().map(|utxo| utxo.value).sum()
    }

    /// Returns all currently tracked UTXOs.
    pub fn utxos(&self) -> impl Iterator<Item = &WalletUtxo> {
        self.utxos.values()
    }
}

fn p2pkh_pubkey_hash(script: &[u8]) -> Option<[u8; 20]> {
    if script.len() != 25
        || script[0] != 0x76
        || script[1] != 0xa9
        || script[2] != 20
        || script[23] != 0x88
        || script[24] != 0xac
    {
        return None;
    }

    let mut hash = [0u8; 20];
    hash.copy_from_slice(&script[3..23]);
    Some(hash)
}

#[cfg(test)]
mod tests {
    use super::Wallet;
    use crate::{Address, Network, PrivateKey};
    use bitrst_core::{Block, BlockHeader, Chain, ChainEvent, Target};
    use bitrst_script::p2pkh_script_pubkey;

    const TEST_BITS: u32 = 0x1f00_ffff;
    const NETWORK_TIME: u32 = 1_231_006_505;

    #[test]
    fn wallet_credits_matching_outputs_from_connected_blocks() {
        let key = fixed_key();
        let address = Address::p2pkh(key.pubkey_hash(), Network::Mainnet);
        let mut wallet = Wallet::new();
        wallet.watch_address(address.clone());
        let mut chain = Chain::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        chain.take_events().expect("events");

        let block = block_paying(&chain, 1, address.pubkey_hash(), 25_0000_0000);
        chain.connect_block(block).expect("connect");
        let events = chain.take_events().expect("events");

        wallet.apply_events(&events, &chain).expect("apply");

        assert_eq!(wallet.balance(), 25_0000_0000);
    }

    #[test]
    fn wallet_ignores_orphan_events() {
        let key = fixed_key();
        let address = Address::p2pkh(key.pubkey_hash(), Network::Mainnet);
        let mut wallet = Wallet::new();
        wallet.watch_address(address);

        wallet
            .apply_events(
                &[ChainEvent::OrphanAdded {
                    hash: [1u8; 32],
                    pool_size: 1,
                }],
                &Chain::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis"),
            )
            .expect("apply");

        assert_eq!(wallet.balance(), 0);
    }

    #[test]
    fn disconnected_block_removes_utxos_even_after_reorg() {
        let key = fixed_key();
        let address = Address::p2pkh(key.pubkey_hash(), Network::Mainnet);
        let mut wallet = Wallet::new();
        wallet.watch_address(address.clone());
        let mut chain = Chain::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        chain.take_events().expect("events");

        let block = block_paying(&chain, 1, address.pubkey_hash(), 25_0000_0000);
        let block_hash = block.hash();
        chain.connect_block(block).expect("connect");
        wallet
            .apply_events(&chain.take_events().expect("events"), &chain)
            .expect("connect event");
        assert_eq!(wallet.balance(), 25_0000_0000);

        let active_genesis_only =
            Chain::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
        wallet
            .apply_events(
                &[ChainEvent::BlockDisconnected {
                    height: 1,
                    hash: block_hash,
                }],
                &active_genesis_only,
            )
            .expect("disconnect event");

        assert_eq!(wallet.balance(), 0);
    }

    fn fixed_key() -> PrivateKey {
        PrivateKey::from_bytes([
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 1,
        ])
        .expect("valid key")
    }

    fn genesis_block() -> Block {
        let header = BlockHeader {
            version: 1,
            prev_blockhash: [0u8; 32],
            merkle_root: [0u8; 32],
            time: NETWORK_TIME,
            bits: TEST_BITS,
            nonce: 0,
        };
        let mut block = Block::coinbase(header, 0, 50_0000_0000);
        mine(&mut block);
        block
    }

    fn block_paying(chain: &Chain, height: u32, pubkey_hash: [u8; 20], value: u64) -> Block {
        let header = BlockHeader {
            version: 1,
            prev_blockhash: chain.tip_hash(),
            merkle_root: [0u8; 32],
            time: NETWORK_TIME + height,
            bits: TEST_BITS,
            nonce: 0,
        };
        let mut block = Block::coinbase(header, height, value);
        block.transactions[0].outputs[0].script_pubkey = p2pkh_script_pubkey(&pubkey_hash);
        block.header.merkle_root = block.merkle_root().expect("merkle root");
        mine(&mut block);
        block
    }

    fn mine(block: &mut Block) {
        let target = Target::from_bits(block.header.bits).expect("test bits");
        while !target.meets(&block.header.hash()) {
            block.header.nonce = block.header.nonce.wrapping_add(1);
        }
    }
}
