//! Shared bounded wire helpers for P2P codecs.

use bitrst_core::wire::{DecodeError, WireReader};

use crate::constants::MAX_INV_COUNT;
use crate::error::NetError;
use crate::message::{InvType, InventoryVector};

pub(crate) fn decode_inventory_list(
    bytes: &[u8],
    context: &'static str,
) -> Result<Vec<InventoryVector>, NetError> {
    let mut reader = WireReader::new(bytes);
    let count = reader
        .read_limited_len(context, MAX_INV_COUNT)
        .map_err(NetError::Decode)?;
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        let inv_type = reader
            .read_u32("inv entry type")
            .map_err(NetError::Decode)?;
        let inv_type = InvType::from_u32(inv_type)
            .ok_or(NetError::HandshakeViolation("unknown inventory type"))?;
        let mut hash = [0u8; 32];
        hash.copy_from_slice(
            reader
                .read_bytes(32, "inv entry hash")
                .map_err(NetError::Decode)?,
        );
        items.push(InventoryVector { inv_type, hash });
    }
    reader.finish(context).map_err(NetError::Decode)?;
    Ok(items)
}

pub(crate) fn encode_inventory_list(items: &[InventoryVector]) -> Result<Vec<u8>, NetError> {
    if items.len() > MAX_INV_COUNT {
        return Err(NetError::PayloadTooLarge {
            length: items.len() as u32,
            limit: MAX_INV_COUNT,
        });
    }
    let mut out = Vec::new();
    write_compact_size(items.len() as u64, &mut out);
    for item in items {
        out.extend_from_slice(&item.inv_type.to_u32().to_le_bytes());
        out.extend_from_slice(&item.hash);
    }
    Ok(out)
}

pub(crate) fn write_compact_size(value: u64, out: &mut Vec<u8>) {
    match value {
        0..=0xfc => out.push(value as u8),
        0xfd..=0xffff => {
            out.push(0xfd);
            out.extend_from_slice(&(value as u16).to_le_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(0xfe);
            out.extend_from_slice(&(value as u32).to_le_bytes());
        }
        _ => {
            out.push(0xff);
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
}

pub(crate) fn read_limited_string(
    reader: &mut WireReader<'_>,
    context: &'static str,
    max_len: usize,
) -> Result<String, DecodeError> {
    let length = reader.read_limited_len(context, max_len)?;
    let bytes = reader.read_bytes(length, context)?;
    String::from_utf8(bytes.to_vec()).map_err(|_| DecodeError::Truncated {
        context,
        needed: 0,
        remaining: 0,
    })
}

#[allow(dead_code)]
pub(crate) fn decode_error(_error: DecodeError) -> NetError {
    NetError::Decode(_error)
}
