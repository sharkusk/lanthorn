// Z-machine header parsing — ZMSD §11.
//
// All multi-byte values are big-endian. A "word" is an unsigned 16-bit value.

use crate::error::ZError;

/// Parsed representation of the Z-machine story file header (ZMSD §11).
#[derive(Debug)]
#[non_exhaustive]
pub struct Header {
    pub version: u8,
    pub high_mem_base: u16,
    pub initial_pc: u16,
    pub dictionary: u16,
    pub object_table: u16,
    pub global_vars: u16,
    pub static_mem_base: u16,
    pub abbrev_table: u16,
    pub file_length: u32,
    pub checksum: u16,
    /// Routines offset (packed address), meaningful only for v7 (byte 0x28).
    pub routines_offset: u16,
    /// Strings offset (packed address), meaningful only for v7 (byte 0x2A).
    pub strings_offset: u16,
}

/// Read a big-endian unsigned 16-bit word from `b` at byte offset `at`.
fn be16(b: &[u8], at: usize) -> u16 {
    ((b[at] as u16) << 8) | b[at + 1] as u16
}

/// Parse the Z-machine header from the first 64 bytes of `bytes`.
///
/// Returns `Err(ZError::NotAStoryFile)` if the slice is shorter than 64 bytes.
/// Returns `Err(ZError::UnsupportedVersion(v))` for versions 1, 2, or any
/// version not in {3, 4, 5, 6, 7, 8}.
pub fn parse_header(bytes: &[u8]) -> Result<Header, ZError> {
    if bytes.len() < 64 {
        return Err(ZError::NotAStoryFile);
    }

    let version = bytes[0x00];

    match version {
        3..=8 => {}
        v => return Err(ZError::UnsupportedVersion(v)),
    }

    // file_length is a packed value: the raw word is multiplied by 2 (v3),
    // 4 (v4/v5), or 8 (v6/v7/v8) to get the byte length. We store the raw
    // decoded byte length. For now we just store the scaled value.
    let raw_len = be16(bytes, 0x1A) as u32;
    let file_length = match version {
        3 => raw_len * 2,
        4 | 5 => raw_len * 4,
        6..=8 => raw_len * 8,
        _ => raw_len,
    };

    // routines_offset / strings_offset are meaningful only for v6 and v7.
    let (routines_offset, strings_offset) = if version == 6 || version == 7 {
        (be16(bytes, 0x28), be16(bytes, 0x2A))
    } else {
        (0, 0)
    };

    Ok(Header {
        version,
        high_mem_base: be16(bytes, 0x04),
        initial_pc: be16(bytes, 0x06),
        dictionary: be16(bytes, 0x08),
        object_table: be16(bytes, 0x0A),
        global_vars: be16(bytes, 0x0C),
        static_mem_base: be16(bytes, 0x0E),
        abbrev_table: be16(bytes, 0x18),
        file_length,
        checksum: be16(bytes, 0x1C),
        routines_offset,
        strings_offset,
    })
}

// Tests_support is reachable as crate::header::tests_support::sample_story,
// the canonical path shared across all tasks in this crate.
#[cfg(test)]
pub(crate) mod tests_support {
    /// Build a minimal but structurally valid story buffer of at least 0x400
    /// bytes for the given Z-machine version. The header fields are set to
    /// well-known values; dynamic memory occupies bytes 0x0040–0x03FF.
    #[allow(dead_code)]
    pub fn sample_story(version: u8) -> Vec<u8> {
        let mut buf = vec![0u8; 0x400];
        // Version byte
        buf[0x00] = version;
        // high_mem_base = 0x0400 (no high memory in this stub)
        buf[0x04] = 0x04;
        buf[0x05] = 0x00;
        // initial_pc = 0x0040 (just past abbreviations)
        buf[0x06] = 0x00;
        buf[0x07] = 0x40;
        // dictionary = 0x0200
        buf[0x08] = 0x02;
        buf[0x09] = 0x00;
        // object_table = 0x0100
        buf[0x0A] = 0x01;
        buf[0x0B] = 0x00;
        // global_vars = 0x0300
        buf[0x0C] = 0x03;
        buf[0x0D] = 0x00;
        // static_mem_base = 0x0400 → dynamic memory is 0x0000–0x03FF
        buf[0x0E] = 0x04;
        buf[0x0F] = 0x00;
        // abbrev_table = 0x0040
        buf[0x18] = 0x00;
        buf[0x19] = 0x40;
        buf
    }

    /// Build a minimal 64-byte header buffer for unit tests of parse_header.
    pub fn sample_header_bytes(version: u8) -> Vec<u8> {
        let mut b = vec![0u8; 64];
        b[0x00] = version;
        b[0x04] = 0x12; b[0x05] = 0x34; // high_mem_base = 0x1234
        b[0x06] = 0x00; b[0x07] = 0x10; // initial_pc   = 0x0010
        b[0x08] = 0x02; b[0x09] = 0x00; // dictionary   = 0x0200
        b[0x0A] = 0x01; b[0x0B] = 0x00; // object_table = 0x0100
        b[0x0C] = 0x03; b[0x0D] = 0x00; // global_vars  = 0x0300
        b[0x0E] = 0x04; b[0x0F] = 0x00; // static_mem_base = 0x0400
        b[0x18] = 0x00; b[0x19] = 0x40; // abbrev_table = 0x0040
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::tests_support::sample_header_bytes;

    #[test]
    fn parses_v3_header_fields() {
        let h = parse_header(&sample_header_bytes(3)).unwrap();
        assert_eq!(h.version, 3);
        assert_eq!(h.high_mem_base, 0x1234);
        assert_eq!(h.initial_pc, 0x0010);
        assert_eq!(h.object_table, 0x0100);
        assert_eq!(h.global_vars, 0x0300);
        assert_eq!(h.static_mem_base, 0x0400);
        assert_eq!(h.abbrev_table, 0x0040);
    }

    #[test]
    fn parses_v6_header_with_offsets() {
        let mut b = sample_header_bytes(6);
        b[0x28] = 0x00; b[0x29] = 0x11; // routines_offset = 0x0011
        b[0x2A] = 0x00; b[0x2B] = 0x22; // strings_offset  = 0x0022
        let h = parse_header(&b).unwrap();
        assert_eq!(h.version, 6);
        assert_eq!(h.routines_offset, 0x0011);
        assert_eq!(h.strings_offset, 0x0022);
    }

    #[test]
    fn rejects_v1_v2_as_unsupported() {
        assert!(matches!(parse_header(&sample_header_bytes(1)).unwrap_err(), ZError::UnsupportedVersion(1)));
        assert!(matches!(parse_header(&sample_header_bytes(2)).unwrap_err(), ZError::UnsupportedVersion(2)));
    }

    #[test]
    fn rejects_truncated() {
        assert!(matches!(parse_header(&[0u8; 10]).unwrap_err(), ZError::NotAStoryFile));
    }
}
