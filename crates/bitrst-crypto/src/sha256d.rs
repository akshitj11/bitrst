use sha2::{Digest, Sha256};

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
            "0100000000000000000000000000000000000000000000000000000000000000000000004a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b29ab5f49ffff001d1dac2b7c",
        )
        .expect("valid hex");

        let hash = sha256d(&header);
        assert_eq!(to_bitcoin_hex(hash), "f3554f2f2af964264669e106f2367c27fe48e49b767f48ca6f0166d0393dc6f2");
    }
}
