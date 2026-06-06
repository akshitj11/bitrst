//! ECDSA signature verification for Bitcoin script checks.

use secp256k1::{ecdsa::Signature, Message, PublicKey, Secp256k1};

/// Errors from ECDSA verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EcdsaVerifyError {
    /// The public key bytes are not a valid secp256k1 key.
    InvalidPublicKey,
    /// The signature bytes are not valid DER (+ optional sighash type suffix).
    InvalidSignature,
    /// Signature verification failed.
    VerificationFailed,
}

/// Verifies a Bitcoin DER-encoded ECDSA signature over a 32-byte sighash.
///
/// `signature_with_hashtype` must include a trailing sighash-type byte (e.g. `0x01` for ALL).
pub fn verify_der_signature(
    signature_with_hashtype: &[u8],
    pubkey_bytes: &[u8],
    sighash: &[u8; 32],
) -> Result<(), EcdsaVerifyError> {
    if signature_with_hashtype.len() < 2 {
        return Err(EcdsaVerifyError::InvalidSignature);
    }

    let (der, _hashtype) = signature_with_hashtype.split_at(signature_with_hashtype.len() - 1);

    let secp = Secp256k1::verification_only();
    let message = Message::from_digest(*sighash);
    let signature = Signature::from_der(der).map_err(|_| EcdsaVerifyError::InvalidSignature)?;
    let pubkey =
        PublicKey::from_slice(pubkey_bytes).map_err(|_| EcdsaVerifyError::InvalidPublicKey)?;

    secp.verify_ecdsa(&message, &signature, &pubkey)
        .map_err(|_| EcdsaVerifyError::VerificationFailed)
}

#[cfg(test)]
mod tests {
    use super::verify_der_signature;
    use secp256k1::{Message, Secp256k1, SecretKey};

    #[test]
    fn verifies_known_signature_roundtrip() {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[0x01; 32]).expect("secret key");
        let pk = secp256k1::PublicKey::from_secret_key(&secp, &sk);
        let sighash = [0xab; 32];
        let message = Message::from_digest(sighash);
        let sig = secp.sign_ecdsa(&message, &sk);
        let mut der_with_type = sig.serialize_der().to_vec();
        der_with_type.push(0x01);

        verify_der_signature(&der_with_type, &pk.serialize(), &sighash).expect("valid sig");
    }

    #[test]
    fn rejects_tampered_signature() {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[0x02; 32]).expect("secret key");
        let pk = secp256k1::PublicKey::from_secret_key(&secp, &sk);
        let sighash = [0xcd; 32];
        let message = Message::from_digest(sighash);
        let sig = secp.sign_ecdsa(&message, &sk);
        let mut der_with_type = sig.serialize_der().to_vec();
        der_with_type.push(0x01);
        der_with_type[10] ^= 0xff;

        assert!(verify_der_signature(&der_with_type, &pk.serialize(), &sighash).is_err());
    }
}
