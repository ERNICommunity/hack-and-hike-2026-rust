//! The format of a record in flash: a small header, then the data.
//!
//! The storage capability keeps one record. Different applications can run
//! on the same board one after the other, so the header names the
//! application's data, and a checksum shows whether the data is complete:
//!
//! ```text
//! offset  size  field
//!      0     4  magic "HNHS"
//!      4     1  format version (1)
//!      5     3  reserved, 0
//!      8     4  kind: 32-bit FNV-1a hash of the record name, little endian
//!     12     4  data length in bytes, little endian
//!     16     4  CRC-32 of the data, little endian
//!     20     …  the data
//! ```
//!
//! Erased flash reads as `0xFF` bytes, which is not a valid header. The
//! capability writes the header last, so a save that stops half way (for
//! example because the power goes off) leaves no valid record.

use crate::network::message::message_kind;

/// Bytes in a [`Header`].
pub const HEADER_LEN: usize = 20;
/// The first bytes of every header.
pub const MAGIC: [u8; 4] = *b"HNHS";
/// The version of this format.
pub const FORMAT_VERSION: u8 = 1;

/// The header in front of the data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// The hash of the record name; see [`kind`].
    pub kind: u32,
    /// Length of the data in bytes.
    pub length: u32,
    /// The [`Crc32`] of the data.
    pub checksum: u32,
}

/// The kind of a record name: its 32-bit FNV-1a hash, like the kind of a
/// network message. Put the application name and a version in the name,
/// for example `"face_unlock/enrolment/v1"`, and change the version when the
/// data changes its meaning.
pub const fn kind(name: &str) -> u32 {
    message_kind(name)
}

impl Header {
    /// The header for `data` saved under `name`.
    ///
    /// # Panics
    ///
    /// When `data` is longer than `u32::MAX` bytes.
    pub fn new(name: &str, data: &[u8]) -> Self {
        let mut crc = Crc32::new();
        crc.update(data);
        Self {
            kind: kind(name),
            length: u32::try_from(data.len()).expect("record data fits in u32"),
            checksum: crc.finish(),
        }
    }

    /// The header as bytes, ready to write.
    pub fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let mut bytes = [0; HEADER_LEN];
        bytes[0..4].copy_from_slice(&MAGIC);
        bytes[4] = FORMAT_VERSION;
        bytes[8..12].copy_from_slice(&self.kind.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.length.to_le_bytes());
        bytes[16..20].copy_from_slice(&self.checksum.to_le_bytes());
        bytes
    }

    /// Read a header. `None` when the bytes are not a header of this
    /// format: erased flash, or data of an older format.
    pub fn parse(bytes: &[u8; HEADER_LEN]) -> Option<Self> {
        if bytes[0..4] != MAGIC || bytes[4] != FORMAT_VERSION {
            return None;
        }
        let word = |start: usize| {
            u32::from_le_bytes([
                bytes[start],
                bytes[start + 1],
                bytes[start + 2],
                bytes[start + 3],
            ])
        };
        Some(Self {
            kind: word(8),
            length: word(12),
            checksum: word(16),
        })
    }
}

/// The CRC-32 checksum (the one of zip and Ethernet), computed piece by
/// piece.
///
/// It detects changed and missing bytes, not deliberate changes.
#[derive(Clone, Copy, Debug)]
pub struct Crc32 {
    /// The inverted remainder so far.
    value: u32,
}

impl Crc32 {
    /// A checksum of no bytes yet.
    pub const fn new() -> Self {
        Self { value: u32::MAX }
    }

    /// Add `bytes` to the checksum.
    pub fn update(&mut self, bytes: &[u8]) {
        /// The reversed CRC-32 polynomial.
        const POLYNOMIAL: u32 = 0xEDB8_8320;
        for &byte in bytes {
            self.value ^= u32::from(byte);
            for _ in 0..8 {
                let mask = (self.value & 1).wrapping_neg();
                self.value = (self.value >> 1) ^ (POLYNOMIAL & mask);
            }
        }
    }

    /// The checksum of all bytes added so far.
    pub const fn finish(&self) -> u32 {
        !self.value
    }
}

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_standard_check_value() {
        let mut crc = Crc32::new();
        crc.update(b"123456789");
        assert_eq!(crc.finish(), 0xCBF4_3926);
        assert_eq!(Crc32::new().finish(), 0);
    }

    #[test]
    fn crc32_can_be_computed_in_pieces() {
        let mut whole = Crc32::new();
        whole.update(b"hello, flash");
        let mut pieces = Crc32::new();
        pieces.update(b"hello");
        pieces.update(b", ");
        pieces.update(b"flash");
        assert_eq!(whole.finish(), pieces.finish());
    }

    #[test]
    fn a_header_survives_the_round_trip() {
        let header = Header::new("face_unlock/enrolment/v1", &[1, 2, 3, 4, 5]);
        assert_eq!(header.kind, kind("face_unlock/enrolment/v1"));
        assert_eq!(header.length, 5);
        assert_eq!(Header::parse(&header.to_bytes()), Some(header));
    }

    #[test]
    fn erased_flash_is_not_a_header() {
        assert_eq!(Header::parse(&[0xFF; HEADER_LEN]), None);
        assert_eq!(Header::parse(&[0; HEADER_LEN]), None);
    }

    #[test]
    fn another_format_version_is_not_a_header() {
        let mut bytes = Header::new("a", b"data").to_bytes();
        bytes[4] = FORMAT_VERSION + 1;
        assert_eq!(Header::parse(&bytes), None);
    }

    #[test]
    fn different_names_have_different_kinds() {
        assert_ne!(
            kind("face_unlock/enrolment/v1"),
            kind("face_unlock/enrolment/v2")
        );
    }

    #[test]
    fn the_checksum_changes_with_the_data() {
        assert_ne!(
            Header::new("a", b"data").checksum,
            Header::new("a", b"date").checksum
        );
    }
}
