// Glulx image header — GLULX_NOTES.md §1, §2.
//
// The first 36 bytes are nine big-endian 32-bit fields. We validate the magic,
// the version (major 2 or 3), and the memory-map invariants (256-byte aligned,
// RAMSTART ≤ EXTSTART ≤ ENDMEM).

use crate::error::GError;

/// Ceiling on ENDMEM, shared with the run-time malloc/restore cap
/// (`Machine::MAX_MEMSIZE`). Applied at load so a 36-byte file cannot demand a
/// ~4 GiB zeroed allocation before the first opcode runs. (SQ-0624)
pub(crate) const MAX_MEMSIZE: u32 = 0x1000_0000; // 256 MiB

/// Ceiling on the header-requested stack. Real games ask for kilobytes (glulxe
/// defaults in the same range); 16 MiB is orders of magnitude of headroom
/// while keeping a hostile header un-allocatable. (SQ-0624)
pub(crate) const MAX_STACK_SIZE: u32 = 0x0100_0000; // 16 MiB

/// The parsed Glulx header fields.
#[derive(Debug, Clone, Copy)]
pub struct Header {
    /// Raw 32-bit version word (major<<16 | minor<<8 | sub).
    pub version: u32,
    /// First writable address (end of ROM).
    pub ramstart: u32,
    /// End of the stored initial memory in the image file.
    pub extstart: u32,
    /// End of the memory map at startup.
    pub endmem: u32,
    /// Stack size the program requests, in bytes.
    pub stack_size: u32,
    /// Address of the first function to execute.
    pub start_func: u32,
    /// String-decoding-table address (0 = none; used in phase 2b).
    pub decode_table: u32,
    /// Whole-image checksum field as stored.
    pub checksum: u32,
}

fn be32(b: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

/// Parse and validate the 36-byte Glulx header from `image`.
pub fn parse_header(image: &[u8]) -> Result<Header, GError> {
    if image.len() < 36 {
        return Err(GError::TooShort);
    }
    if &image[0..4] != b"Glul" {
        return Err(GError::BadMagic);
    }
    let version = be32(image, 0x04);
    let major = version >> 16;
    if major != 2 && major != 3 {
        return Err(GError::UnsupportedVersion(version));
    }
    let h = Header {
        version,
        ramstart: be32(image, 0x08),
        extstart: be32(image, 0x0C),
        endmem: be32(image, 0x10),
        stack_size: be32(image, 0x14),
        start_func: be32(image, 0x18),
        decode_table: be32(image, 0x1C),
        checksum: be32(image, 0x20),
    };

    // Memory-map invariants (GLULX_NOTES §2): all three boundaries are multiples
    // of 256 and ordered RAMSTART ≤ EXTSTART ≤ ENDMEM.
    let aligned = |v: u32| v.is_multiple_of(256);
    if !aligned(h.ramstart) || !aligned(h.extstart) || !aligned(h.endmem) {
        return Err(GError::BadMemoryMap);
    }
    if h.ramstart > h.extstart || h.extstart > h.endmem {
        return Err(GError::BadMemoryMap);
    }

    // Resource sanity caps (SQ-0624): both fields size an upfront allocation.
    if h.endmem > MAX_MEMSIZE {
        return Err(GError::LimitExceeded("ENDMEM exceeds the 256 MiB interpreter cap"));
    }
    if h.stack_size > MAX_STACK_SIZE {
        return Err(GError::LimitExceeded("stack size exceeds the 16 MiB interpreter cap"));
    }

    // The stored initial memory runs [0, EXTSTART); the file must be at least
    // that long.
    if (image.len() as u64) < h.extstart as u64 {
        return Err(GError::Truncated);
    }
    Ok(h)
}

/// The Inform compiler's own identification block (SQ-1306): not part of the
/// Glulx VM spec, but placed by every Inform-generated Glulx image (both
/// Inform 6 and Inform 7 builds) immediately after the 36-byte header — i.e.
/// at the fixed absolute offset 0x24. Glulx-Inform-Tech.html §1 "Static Data":
/// `long 'Info'`, `long` memory-layout word, two 4-byte ASCII version strings
/// (Inform, then the Glulx back-end), then a `short` release number and a
/// `byte[6]` serial number.
#[derive(Debug, Clone)]
pub struct InformInfo {
    pub release: u16,
    pub serial: String,
}

/// Read the Inform `Info` block at 0x24, or `None` when the magic there does
/// not match — a non-Inform Glulx image, or one built too old/differently to
/// carry the stamp. Only ever read after the magic is confirmed, per the
/// Static Data section's own layout: reading release/serial past a
/// non-matching magic would be reading whatever the compiler put there instead.
pub fn parse_inform_info(image: &[u8]) -> Option<InformInfo> {
    if image.len() < 0x3C || &image[0x24..0x28] != b"Info" {
        return None;
    }
    let release = be16(image, 0x34);
    let serial = String::from_utf8_lossy(&image[0x36..0x3C]).into_owned();
    Some(InformInfo { release, serial })
}

fn be16(b: &[u8], off: usize) -> u16 {
    ((b[off] as u16) << 8) | b[off + 1] as u16
}

#[cfg(test)]
mod inform_info_tests {
    use super::*;

    /// The header+Info bytes read directly out of
    /// `stories/CounterfeitMonkey-11.gblorb`'s embedded Glulx chunk (SQ-1306):
    /// release 11, serial "230220" — ground truth, not a guess.
    fn counterfeit_monkey_header_and_info() -> Vec<u8> {
        vec![
            0x47, 0x6c, 0x75, 0x6c, 0x00, 0x03, 0x01, 0x02, 0x00, 0x36, 0x87, 0x00, 0x00, 0x78,
            0xa1, 0x00, 0x00, 0x78, 0xa1, 0x00, 0x00, 0x09, 0x28, 0x00, 0x00, 0x00, 0x00, 0x3c,
            0x00, 0x26, 0x4a, 0xc9, 0xc6, 0xdf, 0x0f, 0x91, 0x49, 0x6e, 0x66, 0x6f, 0x00, 0x01,
            0x00, 0x00, 0x36, 0x2e, 0x34, 0x31, 0x30, 0x2e, 0x33, 0x38, 0x00, 0x0b, 0x32, 0x33,
            0x30, 0x32, 0x32, 0x30,
        ]
    }

    fn synthetic_info(release: u16, serial: &[u8; 6]) -> Vec<u8> {
        let mut b = vec![0u8; 0x3C];
        b[0x24..0x28].copy_from_slice(b"Info");
        b[0x34] = (release >> 8) as u8;
        b[0x35] = release as u8;
        b[0x36..0x3C].copy_from_slice(serial);
        b
    }

    #[test]
    fn reads_counterfeit_monkeys_real_release_and_serial() {
        let info = parse_inform_info(&counterfeit_monkey_header_and_info())
            .expect("CM's Info magic must match");
        assert_eq!(info.release, 11);
        assert_eq!(info.serial, "230220");
    }

    #[test]
    fn reads_release_and_serial_from_a_synthetic_info_block() {
        let img = synthetic_info(52, b"871125");
        let info = parse_inform_info(&img).expect("Info magic present");
        assert_eq!(info.release, 52);
        assert_eq!(info.serial, "871125");
    }

    #[test]
    fn none_when_magic_does_not_match() {
        let mut img = synthetic_info(11, b"230220");
        img[0x24] = b'X'; // corrupt magic: not an Inform-stamped image
        assert!(parse_inform_info(&img).is_none());
    }

    #[test]
    fn none_when_too_short_to_hold_the_block() {
        assert!(parse_inform_info(&[0u8; 10]).is_none());
        assert!(parse_inform_info(&synthetic_info(1, b"000000")[..0x3B]).is_none());
    }
}
