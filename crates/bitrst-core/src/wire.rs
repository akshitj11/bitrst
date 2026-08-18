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
    /// A CompactSize integer used a longer encoding than required.
    #[error("non-canonical CompactSize for {context}: {value}")]
    NonCanonicalCompactSize {
        /// Name of the field being decoded.
        context: &'static str,
        /// Decoded integer value.
        value: u64,
    },
    /// A decoded length exceeded its configured safety limit.
    #[error("{context} length {actual} exceeds limit {limit}")]
    LimitExceeded {
        /// Name of the field being decoded.
        context: &'static str,
        /// Decoded length.
        actual: u64,
        /// Maximum accepted length.
        limit: usize,
    },
    /// Bytes remained after a complete top-level value.
    #[error("trailing bytes after {context}: {remaining}")]
    TrailingBytes {
        /// Name of the top-level value.
        context: &'static str,
        /// Number of unconsumed bytes.
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

    pub(crate) fn read_i32(&mut self, context: &'static str) -> Result<i32, DecodeError> {
        Ok(i32::from_le_bytes(self.read_array(context)?))
    }

    pub(crate) fn read_u64(&mut self, context: &'static str) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.read_array(context)?))
    }

    pub(crate) fn read_bytes(
        &mut self,
        length: usize,
        context: &'static str,
    ) -> Result<&'a [u8], DecodeError> {
        let remaining = self.remaining();
        if remaining < length {
            return Err(DecodeError::Truncated {
                context,
                needed: length,
                remaining,
            });
        }
        let start = self.position;
        self.position += length;
        Ok(&self.bytes[start..self.position])
    }

    pub(crate) fn read_limited_len(
        &mut self,
        context: &'static str,
        limit: usize,
    ) -> Result<usize, DecodeError> {
        let value = self.read_compact_size(context)?;
        let length = usize::try_from(value).map_err(|_| DecodeError::LimitExceeded {
            context,
            actual: value,
            limit,
        })?;
        if length > limit {
            return Err(DecodeError::LimitExceeded {
                context,
                actual: value,
                limit,
            });
        }
        Ok(length)
    }

    pub(crate) fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }

    pub(crate) fn finish(self, context: &'static str) -> Result<(), DecodeError> {
        let remaining = self.remaining();
        if remaining != 0 {
            return Err(DecodeError::TrailingBytes { context, remaining });
        }
        Ok(())
    }

    pub(crate) fn read_compact_size(&mut self, context: &'static str) -> Result<u64, DecodeError> {
        let (value, minimum) = match self.read_array::<1>(context)?[0] {
            value @ 0..=0xfc => return Ok(u64::from(value)),
            0xfd => (
                u64::from(u16::from_le_bytes(self.read_array(context)?)),
                0xfd,
            ),
            0xfe => (
                u64::from(u32::from_le_bytes(self.read_array(context)?)),
                0x1_0000,
            ),
            0xff => (u64::from_le_bytes(self.read_array(context)?), 0x1_0000_0000),
        };
        if value < minimum {
            return Err(DecodeError::NonCanonicalCompactSize { context, value });
        }
        Ok(value)
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

    #[test]
    fn compact_size_rejects_non_canonical_encodings() {
        for bytes in [
            &[0xfd, 0xfc, 0x00][..],
            &[0xfe, 0xff, 0xff, 0x00, 0x00],
            &[0xff, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0],
        ] {
            assert!(matches!(
                WireReader::new(bytes).read_compact_size("count"),
                Err(super::DecodeError::NonCanonicalCompactSize { .. })
            ));
        }
    }

    #[test]
    fn compact_size_rejects_truncated_prefixed_payloads() {
        for bytes in [
            &[0xfd][..],
            &[0xfd, 0xfd],
            &[0xfe, 0, 0, 1],
            &[0xff, 0, 0, 0, 0, 1, 0, 0],
        ] {
            assert!(matches!(
                WireReader::new(bytes).read_compact_size("count"),
                Err(super::DecodeError::Truncated {
                    context: "count",
                    ..
                })
            ));
        }
    }
}
