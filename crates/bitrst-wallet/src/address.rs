//! P2PKH address derivation.

use std::fmt;

use bitrst_crypto::base58;

/// Bitcoin network selector for address version bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Network {
    /// Bitcoin mainnet P2PKH addresses start with `1`.
    Mainnet,
    /// Bitcoin testnet P2PKH addresses usually start with `m` or `n`.
    Testnet,
}

impl Network {
    fn p2pkh_version(self) -> u8 {
        match self {
            Self::Mainnet => 0x00,
            Self::Testnet => 0x6f,
        }
    }
}

/// A Base58Check-encoded P2PKH address.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Address {
    network: Network,
    pubkey_hash: [u8; 20],
    encoded: String,
}

impl Address {
    /// Builds a P2PKH address from a 20-byte public key hash.
    pub fn p2pkh(pubkey_hash: [u8; 20], network: Network) -> Self {
        let encoded = base58::encode_check(network.p2pkh_version(), &pubkey_hash);
        Self {
            network,
            pubkey_hash,
            encoded,
        }
    }

    /// Returns the network this address belongs to.
    pub fn network(&self) -> Network {
        self.network
    }

    /// Returns the 20-byte public key hash payload.
    pub fn pubkey_hash(&self) -> [u8; 20] {
        self.pubkey_hash
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.encoded)
    }
}

#[cfg(test)]
mod tests {
    use super::{Address, Network};
    use crate::PrivateKey;

    #[test]
    fn known_private_key_derives_mainnet_p2pkh_address() {
        let private_key = PrivateKey::from_bytes([
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 1,
        ])
        .expect("valid key");

        let address = Address::p2pkh(private_key.pubkey_hash(), Network::Mainnet);

        assert_eq!(address.to_string(), "1BgGZ9tcN4rm9KBzDn7KprQz87SZ26SAMH");
    }

    #[test]
    fn testnet_address_uses_different_version_byte() {
        let private_key = PrivateKey::from_bytes([
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 1,
        ])
        .expect("valid key");

        let mainnet = Address::p2pkh(private_key.pubkey_hash(), Network::Mainnet);
        let testnet = Address::p2pkh(private_key.pubkey_hash(), Network::Testnet);

        assert_ne!(mainnet.to_string(), testnet.to_string());
        assert!(testnet.to_string().starts_with('m') || testnet.to_string().starts_with('n'));
    }
}
