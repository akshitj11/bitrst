use sha2::{Digest, Sha256};
pub fn sha256d(data: &[u8]->[u8;32]){
    let first = Sha256::digest(data);
    let second=Sha256::digest(first); //hashing the input 2x
    let mut out=[0u8;32];
    out.copy_from_slice(&second);
    out 
}
#[cfg(test)]  //performing this during cargo test , to actually match with the real sha256 outputs
mod tests {
    use super::sha256d;
     #[test]
    fn hashes_genesis_header() {
        let header = hex::decode(
            "010000000000000000000000000000000000000000000000000000000000000000000000\
             4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b\
             29ab5f49ffff001d1dac2b7c",
        )
        .unwrap(); //took btc genesis blockheader bytes then converted it from hex text to bytes

        let hash = sha256d(&header);
        assert_eq!(
            hex::encode(hash),
            "6fe28c0ab6f1b372c1a6a246ae63f74f931e8356655e16d9d6d8fdd3f0f0f19d"
        );//run double hash to check if the output matched to btc's gene hash
    }
}