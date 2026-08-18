//! Safe, bounded decoding helpers for Bitcoin's legacy wire format.

use thiserror::Error;

/// Error returned when untrusted Bitcoin wire bytes cannot be decoded safely.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DecodeError {
    /// The input ended before the named field was complete.
    #[error("truncated {context}: needed {needed} bytes, only {remaining} remain")]
    Truncated {
        /// Name of the field being decoded.
        context: &'static str,
        /// Number of bytes required.
        needed: usize,
        /// Number of bytes still available.
        remaining: usize,
    },
}

pub(crate) struct WireReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> WireReader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    pub(crate) fn read_u32(&mut self, context: &'static str) -> Result<u32, DecodeError> {
        let bytes = self.read_array::<4>(context)?;
        Ok(u32::from_le_bytes(bytes))
    }

    pub(crate) fn read_compact_size(
        &mut self,
        context: &'static str,
    ) -> Result<u64, DecodeError> {
        match self.read_array::<1>(context)?[0] {
            value @ 0..=0xfc => Ok(u64::from(value)),
            0xfd => Ok(u64::from(u16::from_le_bytes(self.read_array(context)?))),
            0xfe => Ok(u64::from(u32::from_le_bytes(self.read_array(context)?))),
            0xff => Ok(u64::from_le_bytes(self.read_array(context)?)),
        }
    }

    fn read_array<const N: usize>(
        &mut self,
        context: &'static str,
    ) -> Result<[u8; N], DecodeError> {
        let remaining = self.bytes.len().saturating_sub(self.position);
        if remaining < N {
            return Err(DecodeError::Truncated {
                context,
                needed: N,
                remaining,
            });
        }
        let mut out = [0; N];
        out.copy_from_slice(&self.bytes[self.position..self.position + N]);
        self.position += N;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::WireReader;

    #[test]
    fn bounded_reader_rejects_truncated_values() {
        let mut reader = WireReader::new(&[1, 2, 3]);
        assert!(reader.read_u32("version").is_err());
    }

    #[test]
    fn compact_size_decodes_canonical_boundaries() {
        for (bytes, expected) in [
            (&[0xfc][..], 0xfc),
            (&[0xfd, 0xfd, 0x00], 0xfd),
            (&[0xfe, 0x00, 0x00, 0x01, 0x00], 0x1_0000),
            (&[0xff, 0, 0, 0, 0, 1, 0, 0, 0], 0x1_0000_0000),
        ] {
            assert_eq!(
                WireReader::new(bytes).read_compact_size("count"),
                Ok(expected)
            );
        }
    }
}
