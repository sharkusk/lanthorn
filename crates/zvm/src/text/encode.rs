//! Z-character text encoding — ZMSD §3.7.
//!
//! Encodes a (lower-cased, truncated) Rust string into the dictionary-resolution
//! form: 4 bytes (6 Z-chars) for v1–v3, 6 bytes (9 Z-chars) for v4+.
//! Z-chars are packed three per 16-bit word, big-endian, with the terminator
//! high bit (0x8000) set on the final word only.
//!
//! **The version decides which shift Z-chars are spelled**, so a word encoded
//! by v3's rules will not match a Version 1 or 2 dictionary: §3.2.2 puts the
//! one-shot shifts at Z-chars 2/3 rather than 4/5 there, and §3.7.1 requires a
//! shift-LOCK (4/5) wherever two consecutive characters share an alphabet.
//!
//! Scope: letters (A0) + A2 characters (shift-5 then A2 position), plus a
//! 10-bit ZSCII escape (shift-5, Z-char 6, hi/lo halves) for characters outside
//! A0/A2 (e.g. accented letters), mirroring the decode side.

use super::{A0, A1};
use crate::memory::Memory;

/// Encode `text` to its dictionary-resolution Z-character form using the
/// **default** alphabets.
///
/// Returns 4 bytes (6 Z-chars) for v1–v3, 6 bytes (9 Z-chars) for v4+
/// (ZMSD §3.7). The input is lower-cased before encoding. Characters longer
/// than the Z-char limit are truncated; shorter strings are padded with
/// Z-char 5.
pub fn encode_word(text: &str, version: u8) -> Vec<u8> {
    encode_word_impl(text, version, None)
}

/// Encode `text` honouring the story's custom alphabet table (v5+, header
/// word 0x34) when one is present; otherwise identical to `encode_word`.
///
/// Used on the dictionary-resolution and `encode_text` paths so player input is
/// encoded with the same alphabet the story's dictionary was built with — a
/// game that remaps A0/A2 would otherwise never match its own dictionary.
pub fn encode_word_mem(text: &str, mem: &Memory) -> Vec<u8> {
    let custom = read_custom_alphabet(mem);
    encode_word_impl(text, mem.version(), custom.as_ref())
}

/// Read the 78-byte custom alphabet table (three 26-byte rows: A0, A1, A2) when
/// the story defines one (v5+, header 0x34 nonzero). Mirrors the decode side.
fn read_custom_alphabet(mem: &Memory) -> Option<[u8; 78]> {
    if mem.version() < 5 {
        return None;
    }
    let p = mem.read_word(0x34) as u32;
    if p == 0 {
        return None;
    }
    let mut rows = [0u8; 78];
    for (i, slot) in rows.iter_mut().enumerate() {
        *slot = mem.read_byte(p + i as u32);
    }
    Some(rows)
}

/// Which alphabet a character lives in and the Z-chars that spell it there,
/// with the leading shift left to the caller (who alone knows the alphabet it
/// is shifting FROM). `len` is 1 for a glyph and 3 for the §3.4 escape.
struct Classified {
    alphabet: u8,
    body: [u8; 3],
    len: usize,
}

/// Shared encoder. `custom`, when present, supplies the A0 (rows 0..26) and A2
/// (rows 52..78) glyphs in place of the defaults; A2 position 0 (the §3.4
/// escape) is never matched, nor position 1 (the newline) from Version 2 on —
/// matching the decode side. In Version 1 position 1 is the digit `0`
/// (§3.5.4), a perfectly ordinary glyph.
fn encode_word_impl(text: &str, version: u8, custom: Option<&[u8; 78]>) -> Vec<u8> {
    let zchar_limit: usize = if version <= 3 { 6 } else { 9 };

    // Lower-case the input and build Z-char sequence.
    let lower = text.to_lowercase();
    let mut zchars: Vec<u8> = Vec::with_capacity(zchar_limit);

    // Alphabet glyph rows: the story's custom table, or the defaults — A2's
    // default row being Version 1's own where that is the version (§3.5.4).
    let a0: &[u8] = custom.map(|r| &r[0..26]).unwrap_or(&A0[..]);
    let a1: &[u8] = custom.map(|r| &r[26..52]).unwrap_or(&A1[..]);
    let a2: &[u8] = custom.map(|r| &r[52..78]).unwrap_or(&crate::text::a2_row(version)[..]);
    // First A2 slot holding a real glyph: 0 is always the escape; 1 is the
    // newline from Version 2 on, but the digit `0` in Version 1.
    let a2_first_glyph = if version == 1 { 1 } else { 2 };

    // §3.2.2 vs §3.2.3: the Z-char that moves one alphabet along is 2 in
    // Versions 1–2 and 4 in Versions 3+, so a shift is `shift_base + delta`
    // and — in Versions 1 and 2 only — a shift-LOCK is two higher again.
    let shift_base: u8 = if version <= 2 { 1 } else { 3 };

    let classify = |ch: char| -> Classified {
        let byte = ch as u8;
        // A0 (lowercase letters a–z by default).
        if let Some(pos) = a0.iter().position(|&b| b == byte) {
            return Classified { alphabet: 0, body: [(pos + 6) as u8, 0, 0], len: 1 };
        }
        // A1. The default A1 is uppercase and is never matched after
        // lower-casing, so standard stories are unaffected — but a custom
        // alphabet table may relocate lowercase letters into A1 (Shogun moves
        // j/q/v/x/z here), so this row must be searched or those words encode
        // via the 10-bit escape and never match the game's own dictionary.
        if let Some(pos) = a1.iter().position(|&b| b == byte) {
            return Classified { alphabet: 1, body: [(pos + 6) as u8, 0, 0], len: 1 };
        }
        // A2 (digits, punctuation, …).
        if let Some(pos) = a2.iter().skip(a2_first_glyph).position(|&b| b == byte) {
            let pos = pos + a2_first_glyph;
            return Classified { alphabet: 2, body: [(pos + 6) as u8, 0, 0], len: 1 };
        }
        // Not in any row — the 10-bit ZSCII escape (§3.4), which lives in A2:
        // Z-char 6 then the high/low 5-bit halves of the ZSCII code.
        let z = crate::text::decode::char_to_zscii_default(ch);
        Classified { alphabet: 2, body: [6, (z >> 5) & 0x1F, z & 0x1F], len: 3 }
    };

    // §3.7.1: "In Versions 1 and 2 only, when encoding text for dictionary
    // words, shift-lock Z-characters 4 and 5 are used instead of the
    // single-shift Z-characters 2 and 3 when the next two characters come from
    // the same alphabet." This is not cosmetic — Bocfel's `dict.cpp` records
    // the case that proves it: Zork I's PDP-10 has the dictionary word
    // "pdp10", whose `1` and `0` are both in A2 and were therefore compiled
    // with a lock; encode them as two single shifts and the word never matches,
    // so the machine can only be referred to by its synonyms. (Frotz's
    // `text.c` does NOT implement this rule — its `encode_text` always emits
    // the single shift — so Bocfel is the reference followed here.)
    let chars: Vec<char> = lower.chars().collect();
    let mut current: u8 = 0; // standing alphabet; always A0 from Version 3 on
    for (n, &ch) in chars.iter().enumerate() {
        if zchars.len() >= zchar_limit {
            break;
        }
        let cur = classify(ch);
        let lock = version <= 2
            && cur.alphabet != current
            && chars.get(n + 1).map(|&next| classify(next).alphabet) == Some(cur.alphabet);
        let delta = (cur.alphabet + 3 - current) % 3;
        // Only emit if the shift AND the whole body fit; a partially written
        // escape would decode as something else entirely.
        let need = cur.len + usize::from(delta != 0);
        if zchars.len() + need > zchar_limit {
            break;
        }
        if delta != 0 {
            zchars.push(shift_base + delta + if lock { 2 } else { 0 });
        }
        if lock {
            current = cur.alphabet;
        }
        zchars.extend_from_slice(&cur.body[..cur.len]);
    }

    // Pad to zchar_limit with Z-char 5.
    while zchars.len() < zchar_limit {
        zchars.push(5);
    }

    // Pack three Z-chars per 16-bit word.
    let word_count = zchar_limit / 3;
    let mut result = Vec::with_capacity(word_count * 2);

    for w in 0..word_count {
        let a = zchars[w * 3] as u16;
        let b = zchars[w * 3 + 1] as u16;
        let c = zchars[w * 3 + 2] as u16;
        let mut word: u16 = (a << 10) | (b << 5) | c;
        if w == word_count - 1 {
            word |= 0x8000; // terminator bit on final word only
        }
        result.push((word >> 8) as u8);
        result.push((word & 0xFF) as u8);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::tests_support::sample_story;
    use crate::memory::Memory;
    use crate::text::decode::decode_string;

    #[test]
    fn encodes_v3_word_to_four_bytes() {
        let enc = encode_word("sword", 3);
        assert_eq!(enc.len(), 4);
        // Terminator 0x8000 is the high bit of the MSB of the final 16-bit word,
        // which is enc[2] (the high byte of the second/last word).
        // Note: the brief specified enc[3] but that is the low byte; corrected here.
        assert_eq!(enc[2] & 0x80, 0x80); // terminator high bit on last word
    }

    #[test]
    fn encodes_v5_word_to_six_bytes() {
        let enc = encode_word("sword", 5);
        assert_eq!(enc.len(), 6);
        // Final word is bytes [4,5]; terminator high bit is enc[4] & 0x80.
        assert_eq!(enc[4] & 0x80, 0x80); // terminator high bit on last word
    }

    #[test]
    fn round_trip_v3() {
        // Encode "sword", write into a sample Memory, decode back, assert prefix.
        let enc = encode_word("sword", 3);
        let bytes = sample_story(3);
        let mut m = Memory::new(bytes).unwrap();
        // Write the 4-byte encoded form at 0x100.
        m.write_word(0x100, ((enc[0] as u16) << 8) | enc[1] as u16);
        m.write_word(0x102, ((enc[2] as u16) << 8) | enc[3] as u16);
        let (decoded, _) = decode_string(&m, 0x100);
        // Decoded text must start with "sword" (padding Z-char 5 may decode as A2 shift, no output).
        assert!(decoded.starts_with("sword"), "decoded: {:?}", decoded);
    }

    #[test]
    fn round_trip_v5() {
        let enc = encode_word("sword", 5);
        let bytes = sample_story(5);
        let mut m = Memory::new(bytes).unwrap();
        m.write_word(0x100, ((enc[0] as u16) << 8) | enc[1] as u16);
        m.write_word(0x102, ((enc[2] as u16) << 8) | enc[3] as u16);
        m.write_word(0x104, ((enc[4] as u16) << 8) | enc[5] as u16);
        let (decoded, _) = decode_string(&m, 0x100);
        assert!(decoded.starts_with("sword"), "decoded: {:?}", decoded);
    }

    #[test]
    fn lowercases_input() {
        assert_eq!(encode_word("SWORD", 3), encode_word("sword", 3));
    }

    #[test]
    fn truncates_long_input() {
        // A word longer than 6 Z-chars for v3 must still produce 4 bytes.
        let enc = encode_word("abcdefghij", 3);
        assert_eq!(enc.len(), 4);
        assert_eq!(encode_word("abcdefghij", 3), encode_word("abcdef", 3));
    }

    #[test]
    fn custom_alphabet_table_is_honoured_on_encode() {
        // v5 story with a custom alphabet table (header 0x34) whose A0 row swaps
        // 'a' and 'b' (A0[0]='b', A0[1]='a'); A1/A2 stay default.
        let tbl: usize = 0x0200;
        let a0 = b"bacdefghijklmnopqrstuvwxyz";
        let a1 = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let a2 = b"\x00\n0123456789.,!?_#'\"/\\-:()";
        let with_table = |word_bytes: &[u8]| {
            let mut bytes = sample_story(5);
            bytes[0x34] = (tbl >> 8) as u8;
            bytes[0x35] = (tbl & 0xFF) as u8;
            for (i, &c) in a0.iter().enumerate() { bytes[tbl + i] = c; }
            for (i, &c) in a1.iter().enumerate() { bytes[tbl + 26 + i] = c; }
            for (i, &c) in a2.iter().enumerate() { bytes[tbl + 52 + i] = c; }
            // Place the encoded word at 0x0100 for a decode round-trip.
            for (i, &b) in word_bytes.iter().enumerate() { bytes[0x0100 + i] = b; }
            Memory::new(bytes).unwrap()
        };

        // With the swap, "ab" must encode differently than the default alphabet…
        let m = with_table(&[]);
        let custom = encode_word_mem("ab", &m);
        assert_ne!(custom, encode_word("ab", 5), "custom A0 swap must change the encoding");

        // …and it must round-trip through the custom-aware decoder back to "ab".
        let m2 = with_table(&custom);
        let (decoded, _) = decode_string(&m2, 0x0100);
        assert!(decoded.starts_with("ab"), "custom round-trip decoded: {:?}", decoded);
    }

    #[test]
    fn custom_alphabet_absent_matches_default() {
        // A v5 story with no custom table (header 0x34 == 0) encodes identically
        // to the default-alphabet path.
        let m = Memory::new(sample_story(5)).unwrap();
        assert_eq!(m.read_word(0x34), 0, "fixture has no custom alphabet table");
        assert_eq!(encode_word_mem("north", &m), encode_word("north", 5));
    }

    #[test]
    fn encodes_accented_char_as_10bit_escape() {
        // 'é' is not in A0/A2, so it must be emitted as a 10-bit ZSCII escape
        // (shift-5, Z-char 6, hi/lo halves) and round-trip back through decode.
        let enc = encode_word("é", 3);
        assert_eq!(enc.len(), 4);
        let bytes = sample_story(3);
        let mut m = Memory::new(bytes).unwrap();
        m.write_word(0x100, ((enc[0] as u16) << 8) | enc[1] as u16);
        m.write_word(0x102, ((enc[2] as u16) << 8) | enc[3] as u16);
        let (decoded, _) = decode_string(&m, 0x100);
        assert_eq!(decoded, "é", "decoded: {:?}", decoded);
    }

    #[test]
    fn encodes_umlaut_round_trips() {
        // Same escape path for 'ü'.
        let enc = encode_word("ü", 3);
        assert_eq!(enc.len(), 4);
        let bytes = sample_story(3);
        let mut m = Memory::new(bytes).unwrap();
        m.write_word(0x100, ((enc[0] as u16) << 8) | enc[1] as u16);
        m.write_word(0x102, ((enc[2] as u16) << 8) | enc[3] as u16);
        let (decoded, _) = decode_string(&m, 0x100);
        assert_eq!(decoded, "ü", "decoded: {:?}", decoded);
    }

    #[test]
    fn ascii_word_encoding_unchanged() {
        // Plain ASCII words never hit the escape path — encoding is unaffected
        // by this change; round-trips exactly as before.
        let enc = encode_word("sword", 3);
        assert_eq!(enc.len(), 4);
        let bytes = sample_story(3);
        let mut m = Memory::new(bytes).unwrap();
        m.write_word(0x100, ((enc[0] as u16) << 8) | enc[1] as u16);
        m.write_word(0x102, ((enc[2] as u16) << 8) | enc[3] as u16);
        let (decoded, _) = decode_string(&m, 0x100);
        assert_eq!(decoded, "sword");
    }

    #[test]
    fn escape_truncates_when_budget_exhausted() {
        // v3 zchar_limit is 6. "aaa" consumes 3 Z-chars, leaving only 3 — not
        // enough for the 4-Z-char escape needed for 'é'. The word must
        // truncate cleanly (no partial escape, no panic), decoding to "aaa".
        let enc = encode_word("aaaé", 3);
        assert_eq!(enc.len(), 4);
        let bytes = sample_story(3);
        let mut m = Memory::new(bytes).unwrap();
        m.write_word(0x100, ((enc[0] as u16) << 8) | enc[1] as u16);
        m.write_word(0x102, ((enc[2] as u16) << 8) | enc[3] as u16);
        let (decoded, _) = decode_string(&m, 0x100);
        assert_eq!(decoded, "aaa", "decoded: {:?}", decoded);
    }

    #[test]
    fn pads_short_input() {
        // A very short word pads to the right length.
        let enc = encode_word("a", 3);
        assert_eq!(enc.len(), 4);
        // High byte of final word has the terminator bit set.
        assert_eq!(enc[2] & 0x80, 0x80);
    }
}
