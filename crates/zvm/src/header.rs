//! Z-machine header parsing — ZMSD §11.
//!
//! All multi-byte values are big-endian. A "word" is an unsigned 16-bit value.

use crate::error::ZError;

/// Parsed representation of the Z-machine story file header (ZMSD §11).
#[derive(Debug)]
#[non_exhaustive]
pub struct Header {
    /// Z-machine version this story targets, header byte `$00`. Every value
    /// 1–8 is a published version this crate loads; [`parse_header`] rejects
    /// anything else.
    pub version: u8,
    /// Base address of high (paged/read-only) memory, header word `$04`.
    /// Packed routine and string addresses resolve relative to this boundary
    /// on the versions that scale them from it.
    pub high_mem_base: u16,
    /// Byte address of the story's first instruction, header word `$06`. In
    /// every version but 6 this is where execution starts; Version 6 instead
    /// starts at its packed `main` routine (ZMSD §5.4), so a v6 host must not
    /// read this field as the entry point.
    pub initial_pc: u16,
    /// Byte address of the parse dictionary, header word `$08` (ZMSD §13). The
    /// base [`crate::dictionary::load`] parses from.
    pub dictionary: u16,
    /// Byte address of the object table, header word `$0A` — property
    /// defaults followed by the object tree itself.
    pub object_table: u16,
    /// Byte address of the global variables table, header word `$0C`: 240
    /// consecutive words, each two bytes.
    pub global_vars: u16,
    /// Base address of static (read-only, non-paged) memory, header word
    /// `$0E`. Marks the end of dynamic memory — everything the story or a
    /// save file may write lives below this address.
    pub static_mem_base: u16,
    /// Abbreviations table base address, header word `$18`. ZMSD §11.1 marks
    /// the field "2" — **Version 1 has no abbreviations table**, and no
    /// abbreviation Z-character to reach it with (§3.3, §3.5.2).
    pub abbrev_table: u16,
    /// Byte length of the story image, from header word `$1A` scaled per ZMSD
    /// §11.1.6. **0 in Versions 1 and 2**, which have no such field (§11.1
    /// marks it "3+"), and in the early Version 3 files that shipped without
    /// one — see `parse_header`.
    pub file_length: u32,
    /// Story checksum, header word `$1C`. "3+" like `file_length` (ZMSD
    /// §11.1): it reads 0 on a Version 1 or 2 image, where `verify` (0OP:189,
    /// itself a Version 3 opcode per §14) cannot be asked for it anyway.
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
/// Returns `Err(ZError::UnsupportedVersion(v))` for any version outside
/// {1, 2, 3, 4, 5, 6, 7, 8} — every published Z-machine version.
pub fn parse_header(bytes: &[u8]) -> Result<Header, ZError> {
    if bytes.len() < 64 {
        return Err(ZError::NotAStoryFile);
    }

    let version = bytes[0x00];

    match version {
        1..=8 => {}
        v => return Err(ZError::UnsupportedVersion(v)),
    }

    // file_length is a packed value: the raw word is multiplied by 2 (v3),
    // 4 (v4/v5), or 8 (v6/v7/v8) to get the byte length (ZMSD §11.1.6).
    //
    // The FIELD only exists from Version 3 — §11.1's header table marks both
    // $1A and $1C "3+", with the note "Some early Version 3 files do not
    // contain length and checksum data". On a Version 1 or 2 image those two
    // bytes are not a length, so scaling them would manufacture a number out
    // of undefined header space; 0 is this struct's "absent" value, and
    // `Machine::story_checksum` falls back to the image's own size when it
    // reads one — which is what Frotz does (`fastmem.c`, `init_memory`: "some
    // old games lack the file size entry" → seek to end of file).
    let raw_len = be16(bytes, 0x1A) as u32;
    let file_length = match version {
        1 | 2 => 0,
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

    /// ZMSD §11.1: byte 0 is the version number, and every published version
    /// 1–8 is a story file this crate loads. Versions 1 and 2 in particular
    /// parse — the gate used to stop at 3 (SQ-1422).
    #[test]
    fn accepts_every_published_version() {
        for v in 1..=8u8 {
            let h = parse_header(&sample_header_bytes(v)).unwrap_or_else(|e| panic!("v{v}: {e:?}"));
            assert_eq!(h.version, v);
        }
    }

    /// §11.1's header table marks $1A "3+". A Version 1 or 2 image has no
    /// length word, so `file_length` reads 0 (the "absent" value) rather than
    /// whatever those undefined bytes happen to hold, doubled.
    #[test]
    fn v1_v2_have_no_file_length_word() {
        for v in [1u8, 2] {
            let mut b = sample_header_bytes(v);
            b[0x1A] = 0x12;
            b[0x1B] = 0x34;
            let h = parse_header(&b).unwrap();
            assert_eq!(h.file_length, 0, "v{v} must not scale $1A into a length");
        }
        // v3 onwards does scale it: 0x1234 * 2.
        let mut b = sample_header_bytes(3);
        b[0x1A] = 0x12;
        b[0x1B] = 0x34;
        assert_eq!(parse_header(&b).unwrap().file_length, 0x1234 * 2);
    }

    #[test]
    fn rejects_versions_outside_one_to_eight() {
        assert!(matches!(parse_header(&sample_header_bytes(0)).unwrap_err(), ZError::UnsupportedVersion(0)));
        assert!(matches!(parse_header(&sample_header_bytes(9)).unwrap_err(), ZError::UnsupportedVersion(9)));
    }

    #[test]
    fn rejects_truncated() {
        assert!(matches!(parse_header(&[0u8; 10]).unwrap_err(), ZError::NotAStoryFile));
    }
}
