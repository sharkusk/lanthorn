//! ZX Spectrum `.z80` snapshot container: decompresses a v1/v2/v3 `.z80`
//! file into the plain 48K memory image a Spectrum-hosted Scott Adams game
//! keeps its tables in.
//!
//! This is a container-format step, not a Scott Adams dialect: a `.z80`
//! snapshot's tables are not at any fixed offset *within the compressed
//! file* (SQ-1414 measured this directly — Golden Baton's ScottFree `.dat`
//! header cannot be found anywhere in a raw `m1goldba.z80`), so any loader
//! for the Spectrum dialect has to run on the output of
//! [`decompress_z80`], never on the file's raw bytes.
//!
//! # Source
//!
//! Written from Gerton Lunter's `.z80` file format description as
//! published by the comp.sys.sinclair FAQ / World of Spectrum
//! (<https://worldofspectrum.org/faq/reference/z80format.htm>, mirrored at
//! <https://worldofspectrum.net/features/faq/reference/z80format.htm>) —
//! a format description, not code, published for exactly this purpose
//! ("Please read the Copyright Notice for distribution policies"; the
//! byte layout itself is a factual specification, and only that layout is
//! used here). No GPL interpreter source (garglk's `ai_uk`/`decompressz80.c`
//! or any ScottFree/Spatterlight fork) was read to write this module — see
//! SQ-1452.
//!
//! # What's implemented
//!
//! Versions 1, 2 and 3 of the container, 48K memory only (hardware mode 0
//! "48k" or 1 "48k + Interface 1" — both use the same page layout). 128K,
//! SamRAM, and every other hardware mode [`decompress_z80`] recognises but
//! refuses, naming the mode byte, since this crate has no 128K-banked
//! memory model to decompress into.
//!
//! # What isn't here
//!
//! C64 snapshots are a different container on a different disk format
//! (D64), and extracting one is squarely a disk-image job, not a
//! decompression job — `lanthorn-blorb` already reads D64, but `scott`
//! takes zero dependencies (see the crate's hard rule), so that extraction
//! has to live in a host (the app, or a CLI) that hands `scott` a raw
//! memory image, the same shape [`decompress_z80`] produces here. No C64
//! loader lives in this crate.

use std::fmt;

/// One 48K Spectrum memory image: bytes `0x4000..=0xFFFF`, i.e. 49,152
/// bytes, indexed from `0x4000` (`image[0]` is address `0x4000`).
pub const IMAGE_LEN: usize = 0xFFFF - 0x4000 + 1;

/// Everything [`decompress_z80`] can fail with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Z80Error {
    /// The file ran out of bytes before the 30-byte v1 header, the
    /// version 2/3 additional header, or a page block's declared length
    /// was fully read.
    Truncated,
    /// The version 2/3 additional-header length word (file offset 30)
    /// isn't 23 (version 2) or 54/55 (version 3) — the file claims a
    /// format version this decoder doesn't recognise.
    UnknownHeaderLength(u16),
    /// The hardware-mode byte (file offset 34, version 2/3 only) names a
    /// machine this decoder doesn't implement — anything but 48K (0) or
    /// 48K + Interface 1 (1), both of which share the same page layout.
    /// The raw byte value is kept so a caller can say which mode it was.
    UnsupportedHardwareMode(u8),
    /// A memory-page block named a page number the 48K/+Interface-1 page
    /// layout doesn't use (only 4, 5 and 8 are valid there).
    BadPageNumber(u8),
    /// RLE-decoding a version 2/3 page produced a byte count other than
    /// the 16,384 every page must be — either the input ran out before
    /// filling it, or a run's repeat count overran the page boundary
    /// before the block's declared input was consumed.
    BadPageSize {
        /// The page number the malformed block named.
        page: u8,
        /// The byte count decoding actually produced.
        got: usize,
    },
    /// The decompressed version 1 body isn't exactly [`IMAGE_LEN`] bytes —
    /// the whole 48K address space, which version 1 always saves as one
    /// contiguous block.
    BadImageSize(usize),
}

impl fmt::Display for Z80Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Z80Error::Truncated => write!(f, "z80 file truncated"),
            Z80Error::UnknownHeaderLength(n) => {
                write!(f, "unrecognised z80 additional-header length {n} (want 23, 54 or 55)")
            }
            Z80Error::UnsupportedHardwareMode(m) => {
                write!(f, "unsupported z80 hardware mode {m} (only 48K/+IF1 are decoded)")
            }
            Z80Error::BadPageNumber(p) => write!(f, "z80 page number {p} is not valid in 48K mode"),
            Z80Error::BadPageSize { page, got } => {
                write!(f, "z80 page {page} decoded to {got} bytes, not 16384")
            }
            Z80Error::BadImageSize(n) => {
                write!(f, "z80 v1 body decoded to {n} bytes, not {IMAGE_LEN}")
            }
        }
    }
}

impl std::error::Error for Z80Error {}

/// Cheap sniff for whether `bytes` is plausibly a `.z80` snapshot: checks
/// the header is long enough to read and that the version 2/3 marker (the
/// program-counter word at offset 6 being zero) is followed by a
/// self-consistent additional-header length. Does not decompress anything,
/// so it can be run on an untrusted or partial buffer to decide whether a
/// multi-engine host should try [`decompress_z80`] at all.
#[must_use]
pub fn looks_like_z80(bytes: &[u8]) -> bool {
    if bytes.len() < 30 {
        return false;
    }
    let pc = u16::from_le_bytes([bytes[6], bytes[7]]);
    if pc != 0 {
        // Version 1: no further structure to check.
        return true;
    }
    if bytes.len() < 32 {
        return false;
    }
    let extra_len = u16::from_le_bytes([bytes[30], bytes[31]]) as usize;
    matches!(extra_len, 23 | 54 | 55) && bytes.len() >= 30 + 2 + extra_len
}

/// Decompresses a version 1, 2 or 3 `.z80` snapshot of a 48K (or 48K +
/// Interface 1) Spectrum into its plain 48K memory image: [`IMAGE_LEN`]
/// bytes covering addresses `0x4000..=0xFFFF`, `image[0]` being `0x4000`.
///
/// # Errors
///
/// See [`Z80Error`]: a truncated file, an unrecognised header-length or
/// hardware-mode byte, a page number outside `{4, 5, 8}`, or an RLE run
/// that over- or under-fills a 16K page.
pub fn decompress_z80(bytes: &[u8]) -> Result<Vec<u8>, Z80Error> {
    if bytes.len() < 30 {
        return Err(Z80Error::Truncated);
    }
    let pc = u16::from_le_bytes([bytes[6], bytes[7]]);
    // Byte 12's compression bit; per the spec, byte 12 == 255 is a known
    // compatibility quirk and must be treated as if it were 1 (no bits set
    // beyond bit 0), i.e. NOT compressed.
    let byte12 = if bytes[12] == 0xFF { 1 } else { bytes[12] };
    let compressed = byte12 & 0x20 != 0;

    if pc != 0 {
        // Version 1: one contiguous 48K block follows the 30-byte header,
        // RLE-compressed (with the `00 ED ED 00` end marker) if `compressed`,
        // otherwise 49,152 raw bytes.
        let body = &bytes[30..];
        let image = if compressed {
            rle_decode(body, true)
        } else {
            body.to_vec()
        };
        if image.len() != IMAGE_LEN {
            return Err(Z80Error::BadImageSize(image.len()));
        }
        return Ok(image);
    }

    // Version 2 or 3: read the additional header, then the hardware mode.
    if bytes.len() < 32 {
        return Err(Z80Error::Truncated);
    }
    let extra_len = u16::from_le_bytes([bytes[30], bytes[31]]);
    if !matches!(extra_len, 23 | 54 | 55) {
        return Err(Z80Error::UnknownHeaderLength(extra_len));
    }
    let header_end = 30 + 2 + extra_len as usize;
    if bytes.len() < header_end {
        return Err(Z80Error::Truncated);
    }
    let hw_mode = bytes[34];
    // 0 = 48k, 1 = 48k + Interface 1 — the only two hardware modes that use
    // the "48 mode" page column (pages 4, 5, 8) this decoder implements.
    if hw_mode != 0 && hw_mode != 1 {
        return Err(Z80Error::UnsupportedHardwareMode(hw_mode));
    }

    let mut image = vec![0u8; IMAGE_LEN];
    let mut off = header_end;
    while off < bytes.len() {
        if off + 3 > bytes.len() {
            return Err(Z80Error::Truncated);
        }
        let block_len = u16::from_le_bytes([bytes[off], bytes[off + 1]]);
        let page = bytes[off + 2];
        off += 3;

        let page_offset = match page {
            4 => 0x4000, // 0x8000-0xbfff
            5 => 0x8000, // 0xc000-0xffff
            8 => 0x0000, // 0x4000-0x7fff
            other => return Err(Z80Error::BadPageNumber(other)),
        };

        let page_data: Vec<u8> = if block_len == 0xFFFF {
            // Uncompressed: exactly 16,384 raw bytes follow.
            let end = off
                .checked_add(16384)
                .ok_or(Z80Error::Truncated)?;
            let slice = bytes.get(off..end).ok_or(Z80Error::Truncated)?;
            off = end;
            slice.to_vec()
        } else {
            let len = block_len as usize;
            let end = off.checked_add(len).ok_or(Z80Error::Truncated)?;
            let slice = bytes.get(off..end).ok_or(Z80Error::Truncated)?;
            off = end;
            // Version 2/3 page blocks have no end marker — decode the
            // whole declared length.
            let decoded = rle_decode(slice, false);
            if decoded.len() != 16384 {
                return Err(Z80Error::BadPageSize {
                    page,
                    got: decoded.len(),
                });
            }
            decoded
        };

        image[page_offset..page_offset + 16384].copy_from_slice(&page_data);
    }

    Ok(image)
}

/// The `.z80` RLE scheme: `ED ED xx yy` means byte `yy` repeated `xx`
/// times; any other byte — including a lone `0xED` not followed by another
/// `0xED` — is copied through literally. When `stop_at_end_marker` is set
/// (version 1 bodies only), the literal 4-byte sequence `00 ED ED 00`
/// terminates decoding without being emitted, matching the spec's stated
/// end marker; version 2/3 page blocks have no such marker and decode
/// their entire declared length.
///
/// A run whose `xx yy` pair is cut off by the end of `data` stops the scan
/// early (matching a truncated file) rather than panicking; the caller
/// checks the resulting length against what the format requires.
fn rle_decode(data: &[u8], stop_at_end_marker: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if stop_at_end_marker
            && i + 4 <= data.len()
            && data[i] == 0x00
            && data[i + 1] == 0xED
            && data[i + 2] == 0xED
            && data[i + 3] == 0x00
        {
            break;
        }
        if data[i] == 0xED && i + 1 < data.len() && data[i + 1] == 0xED {
            if i + 3 >= data.len() {
                break;
            }
            let count = data[i + 2] as usize;
            let value = data[i + 3];
            out.resize(out.len() + count, value);
            i += 4;
        } else {
            out.push(data[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a v1 30-byte header: PC (bytes 6-7) nonzero so it's read as
    /// version 1, byte 12's bit 5 set/clear per `compressed`.
    fn v1_header(compressed: bool) -> Vec<u8> {
        let mut h = vec![0u8; 30];
        h[6] = 0x34; // PC = nonzero => version 1
        h[7] = 0x12;
        h[12] = if compressed { 0x20 } else { 0x00 };
        h
    }

    #[test]
    fn rle_decodes_literals_a_run_and_a_lone_ed() {
        // Literal bytes, then a run (ED ED 03 41 => "A" x3), then a lone ED
        // (not followed by another ED, so passed through), then one more
        // literal, then the v1 end marker.
        let mut body = vec![0x01, 0x02];
        body.extend([0xED, 0xED, 0x03, 0x41]);
        body.push(0xED); // lone ED
        body.push(0x99);
        body.extend([0x00, 0xED, 0xED, 0x00]); // end marker
        body.extend([0xFF, 0xFF, 0xFF]); // must NOT be decoded (past marker)

        let decoded = rle_decode(&body, true);
        assert_eq!(decoded, vec![0x01, 0x02, 0x41, 0x41, 0x41, 0xED, 0x99]);
    }

    #[test]
    fn rle_without_end_marker_decodes_everything() {
        // Same body minus the trailing end marker, decoded as a v2/v3 page
        // (no end marker): everything is consumed.
        let mut body = vec![0x01, 0x02];
        body.extend([0xED, 0xED, 0x03, 0x41]);
        body.push(0xED);
        body.push(0x99);
        let decoded = rle_decode(&body, false);
        assert_eq!(decoded, vec![0x01, 0x02, 0x41, 0x41, 0x41, 0xED, 0x99]);
    }

    #[test]
    fn v1_uncompressed_round_trips() {
        let mut file = v1_header(false);
        let body: Vec<u8> = (0..IMAGE_LEN).map(|i| (i % 251) as u8).collect();
        file.extend(&body);
        let image = decompress_z80(&file).expect("decodes");
        assert_eq!(image, body);
    }

    #[test]
    fn v1_compressed_round_trips() {
        let mut file = v1_header(true);
        // Build a 49,152-byte image with one long run so the compressed
        // form is smaller than the image, then RLE-encode it by hand and
        // append the end marker.
        let mut image = vec![0x07u8; 20_000];
        image.extend((0..(IMAGE_LEN - 20_000)).map(|i| (i % 200) as u8));
        assert_eq!(image.len(), IMAGE_LEN);

        let mut body = vec![0xED, 0xED, 0xFF, 0x07]; // 255 x 0x07
        body.extend(std::iter::repeat_n(0x07u8, 20_000 - 255));
        body.extend(&image[20_000..]);
        body.extend([0x00, 0xED, 0xED, 0x00]);
        file.extend(&body);

        let decoded = decompress_z80(&file).expect("decodes");
        assert_eq!(decoded, image);
    }

    #[test]
    fn v1_wrong_image_size_is_an_error() {
        let mut file = v1_header(false);
        file.extend(vec![0u8; 100]); // far short of IMAGE_LEN
        assert_eq!(decompress_z80(&file), Err(Z80Error::BadImageSize(100)));
    }

    fn v23_header(extra_len: u16, hw_mode: u8) -> Vec<u8> {
        let mut h = vec![0u8; 30];
        // PC = 0 signals version 2/3.
        h.extend(extra_len.to_le_bytes());
        h.extend(vec![0u8; extra_len as usize]);
        h[34] = hw_mode;
        h
    }

    fn page_block(page: u8, compressed_data: &[u8]) -> Vec<u8> {
        let mut b = (compressed_data.len() as u16).to_le_bytes().to_vec();
        b.push(page);
        b.extend(compressed_data);
        b
    }

    fn uncompressed_page_block(page: u8, data: &[u8; 16384]) -> Vec<u8> {
        let mut b = 0xFFFFu16.to_le_bytes().to_vec();
        b.push(page);
        b.extend(data);
        b
    }

    #[test]
    fn v2_three_pages_including_one_uncompressed_round_trips() {
        let mut file = v23_header(23, 0);
        assert_eq!(u16::from_le_bytes([file[30], file[31]]), 23);

        let page4: Vec<u8> = (0..16384).map(|i| (i % 7) as u8).collect();
        let page5: [u8; 16384] = {
            let mut a = [0u8; 16384];
            for (i, b) in a.iter_mut().enumerate() {
                *b = (i % 251) as u8;
            }
            a
        };
        let page8: Vec<u8> = (0..16384).map(|i| ((i * 3) % 5) as u8).collect();

        // page4: RLE-encode as one big run so it round-trips through our
        // own encoder-shaped test data.
        let page4_rle: Vec<u8> = {
            let mut out = Vec::new();
            let mut i = 0;
            while i < page4.len() {
                let start = i;
                while i < page4.len() && page4[i] == page4[start] && i - start < 255 {
                    i += 1;
                }
                let run = i - start;
                if run >= 5 {
                    out.extend([0xED, 0xED, run as u8, page4[start]]);
                } else {
                    out.extend(&page4[start..i]);
                }
            }
            out
        };
        assert_eq!(rle_decode(&page4_rle, false), page4);

        let page8_rle: Vec<u8> = {
            let mut out = Vec::new();
            let mut i = 0;
            while i < page8.len() {
                let start = i;
                while i < page8.len() && page8[i] == page8[start] && i - start < 255 {
                    i += 1;
                }
                let run = i - start;
                if run >= 5 {
                    out.extend([0xED, 0xED, run as u8, page8[start]]);
                } else {
                    out.extend(&page8[start..i]);
                }
            }
            out
        };
        assert_eq!(rle_decode(&page8_rle, false), page8);

        file.extend(page_block(4, &page4_rle));
        file.extend(uncompressed_page_block(5, &page5));
        file.extend(page_block(8, &page8_rle));

        let image = decompress_z80(&file).expect("decodes");
        assert_eq!(&image[0x0000..0x4000], &page8[..]);
        assert_eq!(&image[0x4000..0x8000], &page4[..]);
        assert_eq!(&image[0x8000..0xC000], &page5[..]);
    }

    #[test]
    fn v3_extra_length_54_is_accepted() {
        let mut file = v23_header(54, 0);
        let page4: [u8; 16384] = [0x11; 16384];
        let page5: [u8; 16384] = [0x22; 16384];
        let page8: [u8; 16384] = [0x33; 16384];
        file.extend(uncompressed_page_block(4, &page4));
        file.extend(uncompressed_page_block(5, &page5));
        file.extend(uncompressed_page_block(8, &page8));
        let image = decompress_z80(&file).expect("decodes");
        assert_eq!(&image[0x4000..0x8000], &page4[..]);
    }

    #[test]
    fn unknown_header_length_is_an_error() {
        let file = v23_header(99, 0);
        assert_eq!(decompress_z80(&file), Err(Z80Error::UnknownHeaderLength(99)));
    }

    #[test]
    fn hardware_mode_128k_is_unsupported() {
        let file = v23_header(23, 4); // 4 = 128k in v2
        assert_eq!(decompress_z80(&file), Err(Z80Error::UnsupportedHardwareMode(4)));
    }

    #[test]
    fn bad_page_number_is_an_error() {
        let mut file = v23_header(23, 0);
        file.extend(uncompressed_page_block(9, &[0u8; 16384]));
        assert_eq!(decompress_z80(&file), Err(Z80Error::BadPageNumber(9)));
    }

    #[test]
    fn truncated_v1_header_is_an_error() {
        assert_eq!(decompress_z80(&[0u8; 10]), Err(Z80Error::Truncated));
    }

    #[test]
    fn truncated_page_block_is_an_error() {
        let mut file = v23_header(23, 0);
        // Declares 16384 uncompressed bytes but the file ends early.
        file.extend(0xFFFFu16.to_le_bytes());
        file.push(4);
        file.extend(vec![0u8; 100]);
        assert_eq!(decompress_z80(&file), Err(Z80Error::Truncated));
    }

    #[test]
    fn rle_run_overrunning_the_page_is_an_error() {
        let mut file = v23_header(23, 0);
        // A single run of 255 bytes plus more data totalling well over
        // 16,384 decoded bytes, all declared as page 4's block.
        let mut over = Vec::new();
        for _ in 0..70 {
            over.extend([0xED, 0xED, 0xFF, 0x01]); // 70 * 255 = 17,850
        }
        file.extend(page_block(4, &over));
        match decompress_z80(&file) {
            Err(Z80Error::BadPageSize { page: 4, got }) => assert!(got > 16384),
            other => panic!("expected BadPageSize, got {other:?}"),
        }
    }

    #[test]
    fn looks_like_z80_sniffs_versions() {
        assert!(looks_like_z80(&v1_header(true)));
        assert!(looks_like_z80(&v23_header(23, 0)));
        assert!(looks_like_z80(&v23_header(54, 0)));
        assert!(!looks_like_z80(&[0u8; 5]));
        assert!(!looks_like_z80(&v23_header(99, 0)));
    }
}
