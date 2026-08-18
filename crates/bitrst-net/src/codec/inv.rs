//! `inv` and `getdata` payload codecs.

use crate::error::NetError;
use crate::message::InventoryVector;

use super::wire_helpers::{decode_inventory_list, encode_inventory_list};

/// Decodes an `inv` payload.
///
/// # Errors
///
/// Returns [`NetError`] when counts or fields are invalid.
pub fn decode_inv(payload: &[u8]) -> Result<Vec<InventoryVector>, NetError> {
    decode_inventory_list(payload, "inv count")
}

/// Encodes an `inv` payload.
///
/// # Errors
///
/// Returns [`NetError`] when the inventory list exceeds limits.
pub fn encode_inv(items: &[InventoryVector]) -> Result<Vec<u8>, NetError> {
    encode_inventory_list(items)
}

/// Decodes a `getdata` payload.
///
/// # Errors
///
/// Returns [`NetError`] when counts or fields are invalid.
pub fn decode_getdata(payload: &[u8]) -> Result<Vec<InventoryVector>, NetError> {
    decode_inventory_list(payload, "getdata count")
}

/// Encodes a `getdata` payload.
///
/// # Errors
///
/// Returns [`NetError`] when the inventory list exceeds limits.
pub fn encode_getdata(items: &[InventoryVector]) -> Result<Vec<u8>, NetError> {
    encode_inventory_list(items)
}

#[cfg(test)]
mod tests {
    use super::{decode_getdata, decode_inv, encode_getdata, encode_inv};
    use crate::message::{InvType, InventoryVector};

    fn sample_hash(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    #[test]
    fn inv_roundtrips_single_block_entry() {
        let items = vec![InventoryVector {
            inv_type: InvType::Block,
            hash: sample_hash(0xab),
        }];
        let encoded = encode_inv(&items).expect("encode inv");
        assert_eq!(decode_inv(&encoded), Ok(items));
    }

    #[test]
    fn getdata_roundtrips_transaction_entry() {
        let items = vec![InventoryVector {
            inv_type: InvType::Transaction,
            hash: sample_hash(0x01),
        }];
        let encoded = encode_getdata(&items).expect("encode getdata");
        assert_eq!(decode_getdata(&encoded), Ok(items));
    }

    #[test]
    fn inv_rejects_unknown_type() {
        let mut encoded = encode_inv(&[InventoryVector {
            inv_type: InvType::Block,
            hash: sample_hash(1),
        }])
        .expect("encode");
        encoded[1] = 99;
        encoded[2] = 0;
        encoded[3] = 0;
        encoded[4] = 0;
        assert!(decode_inv(&encoded).is_err());
    }
}
