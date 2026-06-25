//! Private key handling and public key derivation.

use bitrst_crypto::hash160::hash160;
use secp256k1::{PublicKey, Secp256k1, SecretKey};

use crate::WalletError;

/// A secp256k1 private key used to control P2PKH outputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateKey {
    inner: SecretKey,
}

impl PrivateKey {
    /// Creates a private key from a 32-byte secp256k1 scalar.
    ///
    /// # Errors
    ///
    /// Returns [`WalletError::InvalidPrivateKey`] when the bytes are zero or outside the curve order.
    pub fn from_bytes(bytes: [u8; 32]) -> Result<Self, WalletError> {
        let inner = SecretKey::from_slice(&bytes).map_err(|_| WalletError::InvalidPrivateKey)?;
        Ok(Self { inner })
    }

    /// Generates a random secp256k1 private key.
    pub fn generate() -> Self {
        let mut rng = secp256k1::rand::thread_rng();
        Self {
            inner: SecretKey::new(&mut rng),
        }
    }

    /// Returns the compressed 33-byte public key for this private key.
    pub fn public_key(&self) -> [u8; 33] {
        let secp = Secp256k1::signing_only();
        PublicKey::from_secret_key(&secp, &self.inner).serialize()
    }

    /// Returns HASH160(compressed public key), the payload used by P2PKH addresses.
    pub fn pubkey_hash(&self) -> [u8; 20] {
        hash160(&self.public_key())
    }

    /// Returns the underlying secp256k1 secret key for signing.
    pub fn secret_key(&self) -> &SecretKey {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::PrivateKey;

    #[test]
    fn private_key_one_derives_generator_compressed_pubkey() {
        let private_key = PrivateKey::from_bytes([
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 1,
        ])
        .expect("valid key");

        assert_eq!(
            hex::encode(private_key.public_key()),
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
        );
    }

    #[test]
    fn rejects_zero_private_key() {
        assert!(PrivateKey::from_bytes([0u8; 32]).is_err());
    }
}
