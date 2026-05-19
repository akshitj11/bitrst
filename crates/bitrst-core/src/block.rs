use bitrst_crypto::sha256d::sha256d;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockHeader{
    pub version: i32,
    pub prev_blockhash:[u8; 32],
    pub merkle_root:[u8; 32],
    pub time: u32,
    pub bits:u32,
    pub nonce: u32,
}

impl BlockHeader{
    pub fn serialize(&self) -> [u8; 80] {           //converting our header into 80 byte btc header format
        let mut out = [0u8; 80];
        out[0..4].copy_from_slice(&self.version.to_le_bytes());        // this is little endian(kinda reading about this more)
        out[4..36].copy_from_slice(&self.prev_blockhash);
        out[36..68].copy_from_slice(&self.merkle_root);
        out[68..72].copy_from_slice(&self.time.to_le_bytes());
        out[72..76].copy_from_slice(&self.bits.to_le_bytes());
        out[76..80].copy_from_slice(&self.nonce.to_le_bytes());
        out
    }

    pub fn hash(&self) -> [u8; 32] {
     sha256d(&self.serialize())
    }
}

#[cfg(test)]  // builds btc genesis block header > hashes >serializes>matches the output
mod tests {
    use super::BlockHeader;

    fn to_bitcoin_hex(bytes: [u8; 32]) -> String {
        let mut reversed = bytes;
        reversed.reverse();
        hex::encode(reversed)
    }

    #[test]
    fn hashes_genesis_header() {
        let header = BlockHeader {
            version: 1,
            prev_blockhash: [0u8; 32],
            merkle_root: [
                0x3b, 0xa3, 0xed, 0xfd, 0x7a, 0x7b, 0x12, 0xb2,
                0x7a, 0xc7, 0x2c, 0x3e, 0x67, 0x76, 0x8f, 0x61,
                0x7f, 0xc8, 0x1b, 0xc3, 0x88, 0x8a, 0x51, 0x32,
                0x3a, 0x9f, 0xb8, 0xaa, 0x4b, 0x1e, 0x5e, 0x4a,
            ],
            time: 1231006505,
            bits: 0x1d00ffff,
            nonce: 2083236893,
        };

        assert_eq!(
            to_bitcoin_hex(header.hash()),
            "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f"
        );
    }
}
