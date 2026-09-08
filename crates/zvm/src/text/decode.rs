//! ZSCII text decoding — ZMSD §3.2–§3.8.
//!
//! Each Z-string word packs three 5-bit Z-characters; the high bit (0x8000)
//! marks the last word of the string. Three alphabets A0/A1/A2 cover
//! lowercase, uppercase, and punctuation/digits. Shift Z-chars 4/5 (v3+)
//! temporarily switch to A1/A2 for the next character only.

use crate::memory::Memory;
use super::{A0, A1, A2};

/// Decode a Z-encoded string starting at `addr`.
///
/// Returns `(decoded_string, address_past_end)`.
/// Abbreviation strings are decoded recursively (one level in practice).
/// Custom alphabet tables (header word 0x34, v5+) override the default A0/A1/A2
/// glyphs when present; otherwise the default table is used.
pub fn decode_string(mem: &Memory, addr: u32) -> (String, u32) {
    let mut out = String::new();
    let end = decode_into(mem, addr, 0, &mut out);
    (out, end)
}

/// Decode a Z-encoded string at `addr`, keeping the output split by the source
/// **word** that produced it.
///
/// Returns `(fragments, address_past_end)`, where `fragments[k]` is the text
/// completed while consuming the 2-byte word at `addr + 2*k`. The whole string
/// is `fragments.concat()` — which is precisely what [`decode_string`] returns.
///
/// Three 5-bit Z-characters ride in every word (§3.2), so no output character
/// ever sits under one *byte*, and a fragment is usually 0–3 characters. It can
/// also be arbitrarily long: the word carrying an abbreviation's index Z-char
/// (§3.3) is credited with the whole expansion. A character assembled from
/// Z-chars spanning two words — one past an alphabet shift (§3.2.3), or the two
/// Z-chars of a 10-bit ZSCII escape (§3.4) — is credited to the word holding its
/// LAST Z-char, the one that completed it.
///
/// This exists because the decoder is a state machine with no resync point: the
/// alphabet shift and a pending abbreviation both carry across words, so
/// restarting the decode part-way into a string produces text that is wrong
/// rather than merely offset. Attributing a whole-string decode after the fact
/// is the only honest way to say which bytes produced which text.
pub fn decode_string_words(mem: &Memory, addr: u32) -> (Vec<String>, u32) {
    let mut out: Vec<String> = Vec::new();
    let end = decode_into(mem, addr, 0, &mut out);
    (out, end)
}

/// Where one decode's output goes: either a single flat string, or one fragment
/// per source word.
///
/// The point is that `decode_string` — the VM's own text path, run for every
/// `print` — keeps its ONE allocation, while `decode_string_words` gets per-word
/// attribution out of the very same state machine. A shared implementation that
/// always built the per-word vector and joined it would put a `Vec` and a
/// `String` per word on the hot path to serve a debug view.
trait Sink {
    /// Called once with the string's word count before any `emit`.
    fn prepare(&mut self, _words: usize) {}
    /// Append `text`, produced while consuming source word `word`.
    fn emit(&mut self, word: usize, text: &str);
}

impl Sink for String {
    fn emit(&mut self, _word: usize, text: &str) {
        self.push_str(text);
    }
}

impl Sink for Vec<String> {
    fn prepare(&mut self, words: usize) {
        self.resize(words, String::new());
    }
    /// Clamped to the last word: the two Z-chars of a 10-bit escape (§3.4) can
    /// be missing off the end of a malformed string, which leaves the caller's
    /// index past the collected Z-chars.
    fn emit(&mut self, word: usize, text: &str) {
        if let Some(last) = self.len().checked_sub(1) {
            self[word.min(last)].push_str(text);
        }
    }
}

/// The decoder proper, writing into `out`; returns the address past the string's
/// end. ZMSD §3.3: "an abbreviation string must not itself use abbreviations" —
/// so at `depth` ≥ 1 (i.e. while already expanding an abbreviation) further
/// abbreviation Z-chars are NOT expanded. Without this rule a crafted story
/// whose abbreviation table points an abbreviation at itself recurses until the
/// process aborts with a stack overflow.
fn decode_into<S: Sink>(mem: &Memory, addr: u32, depth: u8, out: &mut S) -> u32 {
    // Collect all Z-chars first, recording where the string ends.
    let mut zchars: Vec<u8> = Vec::new();
    let mut pos = addr;
    loop {
        let word = mem.read_word(pos);
        let last = (word & 0x8000) != 0;
        pos += 2;
        zchars.push(((word >> 10) & 0x1F) as u8);
        zchars.push(((word >> 5) & 0x1F) as u8);
        zchars.push((word & 0x1F) as u8);
        // Stop at the end-of-string marker, or at end of memory: an
        // unterminated string (e.g. garbage decoded as a `print` inline
        // string) would otherwise read out of bounds forever until `pos`
        // overflows.
        if last || pos as usize >= mem.len() {
            break;
        }
    }

    let end_addr = pos;
    // Every `emit` below credits the source word holding the Z-char that
    // COMPLETED the character — `(i - 1) / 3`, since `i` has already advanced
    // past that Z-char in all four branches. A flat sink ignores the index.
    out.prepare(zchars.len() / 3);
    let word_of = |i: usize| i.saturating_sub(1) / 3;
    let mut i = 0;
    let mut alphabet: u8 = 0; // 0=A0, 1=A1, 2=A2
    let mut abbrev_pending: u8 = 0; // non-zero: waiting for abbrev index char

    // v5+ custom alphabet table (header 0x34). When present, glyph rows come from
    // the table (78 ZSCII bytes: A0 0..26, A1 26..52, A2 52..78). The A2 specials
    // (position 0 = escape, 1 = newline) keep their built-in handling.
    let custom = if mem.version() >= 5 {
        let p = mem.read_word(0x34) as u32;
        (p != 0).then_some(p)
    } else {
        None
    };

    while i < zchars.len() {
        let zc = zchars[i];
        i += 1;

        // Handle abbreviation index byte
        if abbrev_pending > 0 && depth > 0 {
            // Inside an abbreviation already — §3.3 forbids nested
            // abbreviations, so consume the index char without expanding.
            abbrev_pending = 0;
            continue;
        }
        if abbrev_pending > 0 {
            let z = abbrev_pending;
            abbrev_pending = 0;
            let index = 32 * (z as u32 - 1) + zc as u32;
            let abbrev_base = mem.abbrev_table() as u32;
            let entry_addr = abbrev_base + 2 * index;
            let word_addr = mem.read_word(entry_addr) as u32;
            let byte_addr = word_addr * 2;
            // An expansion is arbitrarily long but belongs entirely to the word
            // carrying its index Z-char, so it is decoded flat and emitted whole.
            let mut expansion = String::new();
            decode_into(mem, byte_addr, depth + 1, &mut expansion);
            out.emit(word_of(i), &expansion);
            // alphabet stays A0 after abbreviation
            continue;
        }

        match zc {
            0 => {
                out.emit(word_of(i), " ");
                // space; any pending shift is consumed
                alphabet = 0;
            }
            1..=3 => {
                abbrev_pending = zc;
                alphabet = 0;
            }
            4 => {
                // Shift to A1 for next character only (v3+)
                alphabet = 1;
            }
            5 => {
                // Shift to A2 for next character only (v3+)
                alphabet = 2;
            }
            zc => {
                if alphabet == 2 && zc == 6 {
                    // 10-bit ZSCII escape: next two Z-chars form the code
                    let hi = if i < zchars.len() { zchars[i] } else { 0 };
                    let lo = if i + 1 < zchars.len() { zchars[i + 1] } else { 0 };
                    i += 2;
                    let zscii = ((hi as u16) << 5) | (lo as u16);
                    // Consult the story's custom Unicode table first (ZMSD §3.8.5.4);
                    // fall back to the default ZSCII table.
                    let ch = mem.unicode_char(zscii).unwrap_or_else(|| zscii_to_char(zscii));
                    out.emit(word_of(i), ch.encode_utf8(&mut [0u8; 4]));
                    alphabet = 0;
                } else {
                    // Normal lookup: Z-char 6 → table index 0.
                    let idx = (zc - 6) as usize;
                    // A2 newline (idx 1) is special and is never taken from a custom
                    // table; A2 escape (idx 0) is already handled above.
                    match custom {
                        Some(tbl) if !(alphabet == 2 && idx == 1) => {
                            // The table holds ZSCII codes (ZMSD §3.5.5), which
                            // take the same §3.8 output translation as the
                            // 10-bit escape above — a raw char cast leaks
                            // control chars (Shogun places ZSCII 11, the
                            // sentence gap, in an alphabet slot).
                            let row = alphabet as u32;
                            let zscii = mem.read_byte(tbl + row * 26 + idx as u32) as u16;
                            let ch = mem.unicode_char(zscii).unwrap_or_else(|| zscii_to_char(zscii));
                            out.emit(word_of(i), ch.encode_utf8(&mut [0u8; 4]));
                        }
                        _ => {
                            let ch = match alphabet {
                                0 => A0[idx],
                                1 => A1[idx],
                                _ => A2[idx],
                            };
                            out.emit(word_of(i), (ch as char).encode_utf8(&mut [0u8; 4]));
                        }
                    }
                    alphabet = 0;
                }
            }
        }
    }

    end_addr
}

/// Default Unicode translation table for ZSCII 155–223 (ZMSD §3.8.5).
const UNICODE_TABLE: [char; 69] = [
    'ä', 'ö', 'ü', 'Ä', 'Ö', 'Ü', 'ß', '»', '«', // 155–163
    'ë', 'ï', 'ÿ', 'Ë', 'Ï', 'á', 'é', 'í', 'ó',  // 164–172
    'ú', 'ý', 'Á', 'É', 'Í', 'Ó', 'Ú', 'Ý', 'à',  // 173–181
    'è', 'ì', 'ò', 'ù', 'À', 'È', 'Ì', 'Ò', 'Ù',  // 182–190
    'â', 'ê', 'î', 'ô', 'û', 'Â', 'Ê', 'Î', 'Ô',  // 191–199
    'Û', 'å', 'Å', 'ø', 'Ø', 'ã', 'ñ', 'õ', 'Ã',  // 200–208
    'Ñ', 'Õ', 'æ', 'Æ', 'ç', 'Ç', 'þ', 'ð', 'Þ',  // 209–217
    'Ð', '£', 'œ', 'Œ', '¡', '¿',                  // 218–223
];

/// Map a ZSCII value to a Rust `char` (ZMSD §3.8).
///
/// ZSCII 13 → '\n'. ASCII 32–126 are identity.
/// ZSCII 155–223 → default Unicode translation table (ZMSD §3.8.5).
/// ZSCII 10 → ' ': it has no printed form. The reference interpreter (Frotz)
/// draws nothing for it but still advances the cursor one column
/// (`os_char_width` returns 1), so it acts as an invisible spacer — Beyond Zork
/// emits it between sentences in its boxed description. Rendering it as a space
/// matches that (a blank column showing the current background) instead of the
/// stray '?' the generic fallback would print.
/// ZSCII 9 (tab) and 11 (sentence gap) → ' ': printable in v6 output
/// (ZMSD §3.8.2.3-4); Frotz displays both as spacing (Shogun's prose uses 11
/// between sentences). Never the raw control char — that panics terminal
/// rendering downstream.
/// Everything else maps to '?'.
pub fn zscii_to_char(zscii: u16) -> char {
    match zscii {
        9..=11 => ' ',
        13 => '\n',
        32..=126 => zscii as u8 as char,
        155..=223 => UNICODE_TABLE[(zscii - 155) as usize],
        _ => '?',
    }
}

/// Map a Rust `char` to a ZSCII byte using the default translation table
/// (ZMSD §3.8), the inverse of `zscii_to_char`.
///
/// `'\n'` → 13. ASCII space–`~` (0x20–0x7E) are identity. Chars in the default
/// Unicode translation table (ZSCII 155–223) map back to their code. Anything
/// else falls back to `'?'` (63).
pub(crate) fn char_to_zscii_default(ch: char) -> u8 {
    match ch {
        '\n' => 13,
        ' '..='~' => ch as u8,
        _ => match UNICODE_TABLE.iter().position(|&c| c == ch) {
            Some(i) => 155 + i as u8,
            None => b'?',
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::tests_support::sample_story;
    use crate::memory::Memory;

    #[test]
    fn decodes_simple_lowercase_word() {
        // "abc" in A0: Z-chars 6,7,8. Word = (6<<10)|(7<<5)|8, top bit set (end).
        let bytes = sample_story(3);
        let w: u16 = 0x8000 | (6 << 10) | (7 << 5) | 8;
        let mut m = Memory::new(bytes).unwrap();
        m.write_word(0x100, w);
        let (s, end) = decode_string(&m, 0x100);
        assert_eq!(s, "abc");
        assert_eq!(end, 0x102);
    }

    #[test]
    fn decodes_space() {
        // Z-char 0 → ' '. Three spaces packed.
        let bytes = sample_story(3);
        let w: u16 = 0x8000; // all-zero Z-chars → three spaces
        let mut m = Memory::new(bytes).unwrap();
        m.write_word(0x100, w);
        let (s, _end) = decode_string(&m, 0x100);
        assert_eq!(s, "   ");
    }

    #[test]
    fn decodes_uppercase_shift() {
        // "A" via shift-to-A1 (Z-char 4) followed by Z-char 6 (A1 index 0 = 'A').
        // Third Z-char in word is 5 (shift to A2) — since string ends there, no output.
        let bytes = sample_story(3);
        let w: u16 = 0x8000 | (4 << 10) | (6 << 5) | 5;
        let mut m = Memory::new(bytes).unwrap();
        m.write_word(0x100, w);
        let (s, end) = decode_string(&m, 0x100);
        assert_eq!(s, "A");
        assert_eq!(end, 0x102);
    }

    #[test]
    fn decodes_a2_digit() {
        // '0' via shift-to-A2 (Z-char 5) followed by Z-char 8 (A2 index 2 = '0').
        // A2: idx 0=escape, 1=\n, 2='0', 3='1', ...
        // Third Z-char is 5 (another shift) — string ends, no output.
        let bytes = sample_story(3);
        let w: u16 = 0x8000 | (5 << 10) | (8 << 5) | 5;
        let mut m = Memory::new(bytes).unwrap();
        m.write_word(0x100, w);
        let (s, end) = decode_string(&m, 0x100);
        assert_eq!(s, "0");
        assert_eq!(end, 0x102);
    }

    /// A three-word string at 0x0100 exercising both things that make a
    /// Z-string undecodable from anywhere but its start, and returns the
    /// fragments plus the whole decode.
    ///
    /// - word 0 `[6, 7, 4]` — 'a', 'b', then Z-char 4: a shift to A1 that
    ///   produces nothing and takes effect on the NEXT word's first Z-char.
    /// - word 1 `[6, 1, 0]` — Z-char 6 under that shift is 'A'; then Z-char 1
    ///   opens abbreviation set 1 and Z-char 0 indexes it — entry 0, which this
    ///   fixture points at "abc".
    /// - word 2 `[5, 8, 5]` (end bit) — shift to A2, Z-char 8 = A2 index 2 =
    ///   '0', and a trailing shift with nothing after it.
    fn shift_and_abbrev_fixture() -> (Vec<String>, String) {
        let bytes = sample_story(3);
        let mut m = Memory::new(bytes).unwrap();
        // Abbreviation string "abc" at byte 0x0080, and set-1 entry 0 (at the
        // 0x0040 table base) holding its WORD address, 0x0040.
        m.write_word(0x0080, 0x8000 | (6 << 10) | (7 << 5) | 8);
        m.write_word(0x0040, 0x0040);
        m.write_word(0x0100, (6 << 10) | (7 << 5) | 4);
        m.write_word(0x0102, (6 << 10) | (1 << 5)); // [6, 1, 0]
        m.write_word(0x0104, 0x8000 | (5 << 10) | (8 << 5) | 5);
        let (frags, end) = decode_string_words(&m, 0x0100);
        assert_eq!(end, 0x0106);
        let (whole, _) = decode_string(&m, 0x0100);
        (frags, whole)
    }

    #[test]
    fn decode_string_words_splits_the_output_by_the_word_that_produced_it() {
        let (frags, whole) = shift_and_abbrev_fixture();
        assert_eq!(frags.len(), 3, "one fragment per source word");
        assert_eq!(frags[0], "ab", "the shift Z-char itself prints nothing");
        assert_eq!(
            frags[1], "Aabc",
            "the shifted 'A' is credited to the word that COMPLETED it, not the \
             one that shifted; and the whole abbreviation expansion lands on the \
             word carrying its index Z-char",
        );
        assert_eq!(frags[2], "0");
        assert_eq!(frags.concat(), whole, "and the fragments rebuild decode_string exactly");
        assert_eq!(whole, "abAabc0");
    }

    #[test]
    fn a_decode_started_mid_string_is_wrong_rather_than_merely_offset() {
        // Why the fragments exist at all: the decoder carries the alphabet shift
        // across the word boundary, so restarting at word 1 loses it and prints
        // a lowercase 'a' where the string says 'A' — text that looks entirely
        // plausible and is not what those bytes mean.
        let (frags, _) = shift_and_abbrev_fixture();
        let bytes = sample_story(3);
        let mut m = Memory::new(bytes).unwrap();
        m.write_word(0x0080, 0x8000 | (6 << 10) | (7 << 5) | 8);
        m.write_word(0x0040, 0x0040);
        m.write_word(0x0102, (6 << 10) | (1 << 5)); // [6, 1, 0]
        m.write_word(0x0104, 0x8000 | (5 << 10) | (8 << 5) | 5);
        let (restarted, _) = decode_string(&m, 0x0102);
        assert_eq!(restarted, "aabc0");
        assert_eq!(frags[1..].concat(), "Aabc0");
        assert_ne!(restarted, frags[1..].concat(), "same bytes, different text");
    }

    #[test]
    fn decodes_two_words() {
        // "abcdef" — two words, first without high bit, second with.
        let bytes = sample_story(3);
        let w1: u16 = (6 << 10) | (7 << 5) | 8; // a, b, c
        let w2: u16 = 0x8000 | (9 << 10) | (10 << 5) | 11; // d, e, f
        let mut m = Memory::new(bytes).unwrap();
        m.write_word(0x100, w1);
        m.write_word(0x102, w2);
        let (s, end) = decode_string(&m, 0x100);
        assert_eq!(s, "abcdef");
        assert_eq!(end, 0x104);
    }

    #[test]
    fn decodes_abbreviation() {
        // Abbreviation set 1 (Z-char 1), index 0 → expands to "abc".
        //
        // Abbreviation string at byte 0x0080: Z-chars [6,7,8] = "abc", terminated.
        //   word = 0x8000 | (6<<10) | (7<<5) | 8
        //
        // Abbreviation table base = 0x0040 (confirmed from sample_story header).
        // Entry for set 1, index 0: table_base + 2*( 32*(1-1)+0 ) = 0x0040.
        // Entry holds a WORD-address; decoder multiplies by 2 to get byte address.
        // To point at 0x0080, write word value 0x0040 at 0x0040.
        //
        // Main string at 0x0100: Z-chars [1, 0, 5], terminated.
        //   Z-char 1 = abbreviation trigger, Z-char 0 = index, Z-char 5 = shift (no output).
        //   word = 0x8000 | (1<<10) | (0<<5) | 5
        let bytes = sample_story(3);
        let mut m = Memory::new(bytes).unwrap();

        // Write abbreviation string "abc" at 0x0080.
        let abbrev_word: u16 = 0x8000 | (6 << 10) | (7 << 5) | 8;
        m.write_word(0x0080, abbrev_word);

        // Write abbreviation table entry at 0x0040: word-address 0x0040 → byte 0x0080.
        m.write_word(0x0040, 0x0040);

        // Write main string at 0x0100: [1, 0, 5] with terminator.
        let main_word: u16 = (0x8000 | (1 << 10)) | 5;
        m.write_word(0x0100, main_word);

        let (s, end) = decode_string(&m, 0x0100);
        assert_eq!(s, "abc");
        assert_eq!(end, 0x0102);
    }

    #[test]
    fn self_referencing_abbreviation_does_not_recurse_forever() {
        // ZMSD §3.3: "an abbreviation string must not itself use
        // abbreviations". A crafted story whose abbreviation 0 expands to a
        // string that invokes abbreviation 0 again used to recurse until the
        // process aborted with a stack overflow. Nested abbreviation Z-chars
        // are now consumed without expanding. (SQ-0620)
        let bytes = sample_story(3);
        let mut m = Memory::new(bytes).unwrap();
        // Abbrev table entry 0 (at 0x0040): word-address 0x0040 → byte 0x0080.
        m.write_word(0x0040, 0x0040);
        // The abbreviation string at 0x0080 triggers abbreviation 0 (itself),
        // then prints 'a': Z-chars [1, 0, 6], terminated.
        let abbrev_word: u16 = 0x8000 | (1 << 10) | 6;
        m.write_word(0x0080, abbrev_word);
        // Main string: [1, 0, 5] — expand abbreviation 0.
        let main_word: u16 = (0x8000 | (1 << 10)) | 5;
        m.write_word(0x0100, main_word);
        let (s, _) = decode_string(&m, 0x0100);
        assert_eq!(s, "a", "nested abbreviation ignored; outer text still decodes");
    }

    #[test]
    fn custom_unicode_table_overrides_default_zscii_155() {
        // Build a story with a header-extension table (at 0x0200) that points
        // to a Unicode translation table (at 0x0210).
        // Header word 0x36 = address of header-extension table (0x0200).
        // Header-ext word 0 = number of words in table = 3.
        // Header-ext word 3 (byte offset 6) = address of Unicode table (0x0210).
        // Unicode table: byte 0 = N=1, then 1 word = U+00A9 (©).
        // This maps ZSCII 155 → '©' (overriding default 'ä').
        //
        // Encode ZSCII 155 via the 10-bit escape: A2 shift (5), Z-char 6 (escape),
        // hi = 155 >> 5 = 4, lo = 155 & 31 = 27.
        // Pack into two words:
        //   w1 = [5, 6, 4]
        //   w2 (end) = [27, 5, 5]
        let mut bytes = sample_story(5); // v5 has header-extension table
        // header-ext table address at byte 0x36
        bytes[0x36] = 0x02; bytes[0x37] = 0x00; // 0x0200
        // header-ext table at 0x0200: word 0 = 3 (table has 3 words)
        bytes[0x0200] = 0x00; bytes[0x0201] = 0x03; // word 0: length=3
        bytes[0x0202] = 0x00; bytes[0x0203] = 0x00; // word 1: unused
        bytes[0x0204] = 0x00; bytes[0x0205] = 0x00; // word 2: unused
        bytes[0x0206] = 0x02; bytes[0x0207] = 0x10; // word 3: Unicode table at 0x0210
        // Unicode translation table at 0x0210: N=1, then word U+00A9 (©)
        bytes[0x0210] = 0x01;                        // N=1
        bytes[0x0211] = 0x00; bytes[0x0212] = 0xA9; // U+00A9 = ©
        let mut m = Memory::new(bytes).unwrap();
        // Encode ZSCII 155 via 10-bit escape
        let w1: u16 = (5 << 10) | (6 << 5) | 4;
        let w2: u16 = 0x8000 | (27 << 10) | (5 << 5) | 5;
        m.write_word(0x0100, w1);
        m.write_word(0x0102, w2);
        let (s, _end) = decode_string(&m, 0x0100);
        assert_eq!(s, "©", "custom Unicode table should map ZSCII 155 to © not ä");
    }

    #[test]
    fn custom_alphabet_table_overrides_default_glyphs() {
        // v5 story with header 0x34 -> a custom 78-byte alphabet table whose A0[0]='z'.
        let mut bytes = sample_story(5);
        let tbl: usize = 0x0200;
        bytes[0x34] = (tbl >> 8) as u8;
        bytes[0x35] = (tbl & 0xFF) as u8;
        let a0 = b"zbcdefghijklmnopqrstuvwxyz";
        let a1 = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let a2 = b"\x00\n0123456789.,!?_#'\"/\\-:()";
        for (i, &c) in a0.iter().enumerate() { bytes[tbl + i] = c; }
        for (i, &c) in a1.iter().enumerate() { bytes[tbl + 26 + i] = c; }
        for (i, &c) in a2.iter().enumerate() { bytes[tbl + 52 + i] = c; }
        let mut m = Memory::new(bytes).unwrap();
        // One A0 Z-char 6 (the first A0 letter), padded with shifts (no output).
        let w: u16 = 0x8000 | (6 << 10) | (5 << 5) | 5;
        m.write_word(0x0100, w);
        let (s, _end) = decode_string(&m, 0x0100);
        assert_eq!(s, "z", "custom A0[0] should decode to 'z' not the default 'a'");
    }

    #[test]
    fn custom_alphabet_bytes_are_zscii_codes_not_raw_chars() {
        // ZMSD §3.5.5: a custom alphabet table holds ZSCII codes, which must go
        // through the §3.8 output translation like any other printed ZSCII —
        // NOT be cast to a char raw. Shogun's table puts ZSCII 11 (sentence
        // gap) in an alphabet slot; the raw cast leaked '\u{b}' into every
        // prose string and panicked ratatui's cell_width debug assert (SQ-0456).
        // A code ≥155 cast raw is a C1 control char instead of its accent.
        let mut bytes = sample_story(6);
        let tbl: usize = 0x0200;
        bytes[0x34] = (tbl >> 8) as u8;
        bytes[0x35] = (tbl & 0xFF) as u8;
        let mut a0 = *b"abcdefghijklmnopqrstuvwxyz";
        a0[0] = 11; // ZSCII 11 = sentence gap
        a0[1] = 155; // ZSCII 155 = 'ä' via the default Unicode table
        let a1 = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let a2 = b"\x00\n0123456789.,!?_#'\"/\\-:()";
        for (i, &c) in a0.iter().enumerate() { bytes[tbl + i] = c; }
        for (i, &c) in a1.iter().enumerate() { bytes[tbl + 26 + i] = c; }
        for (i, &c) in a2.iter().enumerate() { bytes[tbl + 52 + i] = c; }
        let mut m = Memory::new(bytes).unwrap();
        // Z-chars: 6 (A0[0] = ZSCII 11), 7 (A0[1] = ZSCII 155), pad 5.
        let w: u16 = 0x8000 | (6 << 10) | (7 << 5) | 5;
        m.write_word(0x0100, w);
        let (s, _end) = decode_string(&m, 0x0100);
        assert_eq!(s, " ä", "table bytes must be ZSCII-translated: 11 → space, 155 → ä");
    }

    #[test]
    fn zscii_tab_and_sentence_gap_render_as_spaces() {
        // ZMSD §3.8.2.3-4: ZSCII 9 (tab) and 11 (sentence gap) are printable in
        // v6 output. Frotz displays both as spacing (its curses layer expands
        // them to runs of spaces); a space, never '?', and never the raw
        // control char.
        assert_eq!(zscii_to_char(9), ' ');
        assert_eq!(zscii_to_char(11), ' ');
    }

    #[test]
    fn zscii_extended_chars_decode() {
        assert_eq!(zscii_to_char(155), 'ä');
        assert_eq!(zscii_to_char(161), 'ß');
        assert_eq!(zscii_to_char(219), '£');
        assert_eq!(zscii_to_char(220), 'œ');
        assert_eq!(zscii_to_char(223), '¿');
        // boundary: just outside the table
        assert_eq!(zscii_to_char(154), '?');
        assert_eq!(zscii_to_char(224), '?');
    }

    #[test]
    fn char_to_zscii_default_inverts_default_table() {
        // 'û' is at UNICODE_TABLE index 40 (191..199 = â,ê,î,ô,û,...) → 155+40=195.
        assert_eq!(char_to_zscii_default('û'), 195);
        assert_eq!(char_to_zscii_default('A'), 65);
        assert_eq!(char_to_zscii_default('\n'), 13);
        // '€' is not in the default table or ASCII range → fallback '?'.
        assert_eq!(char_to_zscii_default('€'), b'?');
    }

    #[test]
    fn zscii_line_feed_renders_as_space() {
        // ZSCII 10 has no printed form; Frotz draws nothing but advances one
        // column, so it is an invisible spacer, not a newline. Render it as a
        // space (blank column over the current background), never the stray '?'
        // the generic fallback would print. ZSCII 13 stays the real newline.
        assert_eq!(zscii_to_char(10), ' ');
        assert_eq!(zscii_to_char(13), '\n');
    }

    #[test]
    fn decodes_digits_and_punctuation() {
        // Regression: A2 table must NOT have a space at index 2.
        // Per ZMSD §3.5.3, fixed A2: idx 2='0', idx 11='9', idx 12='.', idx 18='\''.
        // Z-char 5 = A2 shift; the following Z-char selects the character.
        // Period ('.') = A2 idx 12 = Z-char 18.
        // '9'         = A2 idx 11 = Z-char 17.
        // Apostrophe  = A2 idx 18 = Z-char 24.
        //
        // Word 1: [5, 18, 5]  → '.' then shift-A2
        // Word 2: [17, 5, 24] → '9' then shift-A2 then '\''? No: after shift+char,
        //   alphabet resets. So: [5, 18, 5] → '.', shift pending; [17] → wrong.
        //
        // Use three words for clarity:
        // Word 1 (no-end): Z-chars [5, 18, 5]  → period, then shift-A2 starts
        // Word 2 (no-end): Z-chars [17, 5, 24] → '9', shift-A2, '\''
        // Word 3 (end):    Z-chars [5, 5, 5]   → three unused shifts (no chars)
        //
        // Actually pack more carefully — each Z-char needs its own shift:
        // Word 1: [5, 18, 5]   → '.', A2-shift for next char
        // Word 2: [17, 5, 24]  → '9', A2-shift, '\''  ← shift is consumed by 24
        // Wait: after outputting '.', alphabet resets to 0.
        // [5, 18, 5]:  5=A2-shift, 18='.' output, 5=A2-shift again
        // [17, ...]    but alphabet=2 so 17 → A2 idx 11 = '9'; alphabet resets
        // [5, 24, 5]   5=A2-shift, 24='\'' output, 5=unused-shift
        // So three words:
        //   w1 = [5, 18, 5]
        //   w2 = [17, 5, 24]
        //   w3 (end) = [5, 5, 5]
        // Result: '.', '9', '\''
        let bytes = sample_story(3);
        let w1: u16 = (5 << 10) | (18 << 5) | 5;
        let w2: u16 = (17 << 10) | (5 << 5) | 24;
        let w3: u16 = 0x8000 | (5 << 10) | (5 << 5) | 5;
        let mut m = Memory::new(bytes).unwrap();
        m.write_word(0x100, w1);
        m.write_word(0x102, w2);
        m.write_word(0x104, w3);
        let (s, end) = decode_string(&m, 0x100);
        assert_eq!(s, ".9'", "period should be '.' not '9', apostrophe should be '\\'' not '#'");
        assert_eq!(end, 0x106);
    }

    #[test]
    fn decodes_zscii_10bit_escape() {
        // ZSCII escape: A2 shift (5), then Z-char 6 (escape trigger),
        // then hi=1, lo=0 → ZSCII = (1<<5)|0 = 32 → ' ' (space via 10-bit path).
        // Pack Z-chars as: [5, 6, 1, 0, 5, 5] across two words.
        // Word 1: zchars [5, 6, 1]
        // Word 2: zchars [0, 5, 5] — the 0 and next are the hi/lo bytes consumed
        //         by the escape; trailing [5, 5] are unused shifts.
        // So total output: one space (' ', ZSCII 32).
        let bytes = sample_story(3);
        let w1: u16 = (5 << 10) | (6 << 5) | 1;
        let w2: u16 = 0x8000 | (5 << 5) | 5;
        let mut m = Memory::new(bytes).unwrap();
        m.write_word(0x100, w1);
        m.write_word(0x102, w2);
        let (s, end) = decode_string(&m, 0x100);
        assert_eq!(s, " "); // ZSCII 32 = space
        assert_eq!(end, 0x104);
    }
}
