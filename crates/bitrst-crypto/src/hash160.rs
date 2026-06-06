//! HASH160: RIPEMD160(SHA256(data)).

use ripemd::Ripemd160;
use sha2::{Digest, Sha256};

/// Computes Bitcoin HASH160 of `data`.
pub fn hash160(data: &[u8]) -> [u8; 20] {
    let sha = Sha256::digest(data);
    let ripemd = Ripemd160::digest(sha);
    let mut out = [0u8; 20];
    out.copy_from_slice(&ripemd);
    out
}

#[cfg(test)]
mod tests {
    use super::hash160;
    use hex::FromHex;

    #[test]
    fn hash160_matches_compressed_pubkey_fixture() {
        let pubkey = <[u8; 33]>::from_hex(
            "0279be667ef9dcbbac55b06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        )
        .expect("hex");
        let digest = hash160(&pubkey);
        let expected =
            <[u8; 20]>::from_hex("d30c70f7d1e208120e1e5e55b5341fa321a60ff2").expect("hex");
        assert_eq!(digest, expected);
    }

    #[test]
    fn hash160_is_deterministic() {
        let digest = hash160(b"bitrst-hash160-fixture");
        assert_eq!(digest, hash160(b"bitrst-hash160-fixture"));
        assert_ne!(digest, [0u8; 20]);
    }
}
