use sha2::{Digest, Sha256};

/// Returns Bitcoin's SHA-256d hash for the provided bytes.
pub fn sha256d(data: &[u8]) -> [u8; 32] {
    let first = Sha256::digest(data);
    let second = Sha256::digest(first);
    let mut out = [0u8; 32];
    out.copy_from_slice(&second);
    out
}

#[cfg(test)]
mod tests {
    use super::sha256d;

    fn to_bitcoin_hex(bytes: [u8; 32]) -> String {
        let mut reversed = bytes;
        reversed.reverse();
        hex::encode(reversed)
    }

    #[test]
    fn hashes_genesis_header_bytes() {
        let header = hex::decode(
            "010000000000000000000000000000000000000000000000000000000000000000000000\
             3ba3edfd7a7b12b27ac72c3e67768f617fc81bc3888a51323a9fb8aa4b1e5e4a\
             29ab5f49ffff001d1dac2b7c",
        )
        .expect("genesis header hex should decode");

        let hash = sha256d(&header);
        assert_eq!(
            to_bitcoin_hex(hash),
            "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f"
        );
    }
}
