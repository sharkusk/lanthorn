// Versions 1 and 2 (SQ-1422). No redistributable Version 1 or 2 story exists —
// the whole corpus is a handful of Infocom releases nobody may ship — so every
// fixture here is a hand-built image and every EXPECTED string below was worked
// out from the Z-Machine Standards Document 1.1's own tables by hand, never
// from this crate's output. The sections each case reads are named beside it.
//
// The four rules that make Versions 1 and 2 a different text format:
//
//   §3.2.2  Z-chars 2 and 3 shift for one character; 4 and 5 shift-LOCK. Both
//           pairs step the current alphabet cyclically — 2/4 by one alphabet,
//           3/5 by two — where Versions 3+ (§3.2.3) have only 4 and 5, meaning
//           a flat "next character is in A1/A2" with no lock at all.
//   §3.3    Version 2 spends Z-char 1 on abbreviations and has ONE table of 32.
//           Version 1 has no abbreviation Z-character whatsoever.
//   §3.5.2  In Version 1, Z-char 1 prints a new-line.
//   §3.5.4  Which is why Version 1's A2 row differs: with no newline to carry
//           at Z-char 7, the row slides one place left and gains `<`.
//
// And the encoder half, which is what a v1/v2 DICTIONARY is built with:
//
//   §3.7.1  "In Versions 1 and 2 only, when encoding text for dictionary words,
//           shift-lock Z-characters 4 and 5 are used instead of the
//           single-shift Z-characters 2 and 3 when the next two characters come
//           from the same alphabet."

use std::cell::RefCell;
use std::rc::Rc;

use zvm::cpu::boot::BootConfig;
use zvm::cpu::exec::{Machine, StepResult};
use zvm::dictionary::load;
use zvm::header::parse_header;
use zvm::io::Output;
use zvm::memory::Memory;
use zvm::text::decode::decode_string;
use zvm::text::encode_word;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Where each case puts what. Code and strings live in STATIC memory, above
/// `static_mem_base` — the header's own tail looks like free space and is not
/// (`init_caps` rewrites Flags 2 at $10, which silently ate a `print` opcode
/// while this suite was being written).
const CODE: usize = 0x0400;
const STRING: usize = 0x0500;
const EXPANSION: usize = 0x0600;
/// The `read` buffers, which the GAME writes, so these must stay dynamic.
const TEXT_BUF: usize = 0x0250;
const PARSE_BUF: usize = 0x0280;

/// A minimal but structurally valid story image for `version`, mirroring the
/// crate-internal `header::tests_support::sample_story` (not reachable from an
/// integration test). Dynamic memory is 0x0000–0x03FF: header, then the
/// abbreviations table at 0x0040 (32 words, §3.3), the object table at 0x0100,
/// the dictionary at 0x0200 and the globals at 0x0300. Everything from 0x0400
/// is static, and execution begins there.
fn sample_story(version: u8) -> Vec<u8> {
    let mut buf = vec![0u8; 0x800];
    buf[0x00] = version;
    buf[0x04] = 0x04; // high_mem_base   = 0x0400
    buf[0x06] = 0x04;
    buf[0x07] = 0x00; // initial_pc      = 0x0400
    buf[0x08] = 0x02; // dictionary      = 0x0200
    buf[0x0A] = 0x01; // object_table    = 0x0100
    buf[0x0C] = 0x03; // global_vars     = 0x0300
    buf[0x0E] = 0x04; // static_mem_base = 0x0400
    buf[0x18] = 0x00;
    buf[0x19] = 0x40; // abbrev_table    = 0x0040
    buf
}

/// Write `zchars` at `addr` as packed Z-string words, three per 16-bit word,
/// terminator high bit on the last. `zchars.len()` must be a multiple of 3.
fn put_zstring(buf: &mut [u8], addr: usize, zchars: &[u8]) {
    assert_eq!(zchars.len() % 3, 0, "a Z-string is whole words of three Z-chars");
    let words = zchars.len() / 3;
    for w in 0..words {
        let mut word = ((zchars[w * 3] as u16) << 10)
            | ((zchars[w * 3 + 1] as u16) << 5)
            | zchars[w * 3 + 2] as u16;
        if w == words - 1 {
            word |= 0x8000;
        }
        buf[addr + w * 2] = (word >> 8) as u8;
        buf[addr + w * 2 + 1] = (word & 0xFF) as u8;
    }
}

/// Decode `zchars` as a string sitting at `STRING` in a fresh `version` image.
fn decode_zchars(version: u8, zchars: &[u8]) -> String {
    let mut buf = sample_story(version);
    put_zstring(&mut buf, STRING, zchars);
    let m = Memory::new(buf).unwrap();
    decode_string(&m, STRING as u32).0
}

/// A sink that shares its buffer with the test, per `zvm::io`'s documented
/// pattern — no downcast needed to read what the story printed.
struct SharedSink(Rc<RefCell<String>>);

impl Output for SharedSink {
    fn print(&mut self, s: &str) {
        self.0.borrow_mut().push_str(s);
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ---------------------------------------------------------------------------
// The gate itself
// ---------------------------------------------------------------------------

/// ZMSD §11.1: byte 0 is the version number, and this crate now loads every
/// published one. The gate rejected 1 and 2 outright until SQ-1422.
#[test]
fn versions_one_and_two_load() {
    for v in 1..=8u8 {
        let m = Memory::new(sample_story(v)).unwrap_or_else(|e| panic!("v{v} refused: {e:?}"));
        assert_eq!(m.version(), v);
    }
}

/// §1.2.3: packed addresses are doubled in Versions 1 to 3 alike. A v1 image
/// whose `print_paddr` resolved by a v4 rule would read four bytes off target.
#[test]
fn v1_v2_unpack_packed_addresses_by_two() {
    for v in [1u8, 2] {
        let m = Memory::new(sample_story(v)).unwrap();
        assert_eq!(m.unpack_routine(0x0080), 0x0100, "v{v} routine");
        assert_eq!(m.unpack_string(0x0080), 0x0100, "v{v} string");
    }
}

// ---------------------------------------------------------------------------
// §3.2.2 — one-shot shift (Z-chars 2, 3)
// ---------------------------------------------------------------------------

/// Z-chars `[6, 3, 8] [7, 5, 5]`, read three ways.
///
/// **Versions 1 and 2** (§3.2.2): 6 → A0 index 0 = `a`. 3 → shift the next
/// character two alphabets on from A0, i.e. A2. 8 → A2 index 2, which is `0` in
/// the §3.5.3 row and `1` in Version 1's §3.5.4 row. 7 → the shift is spent, so
/// back in A0: index 1 = `b`. The trailing 5s are shift-locks, and §3.2.4 says a
/// sequence of shifts "prints nothing".
///
/// **Version 3** (§3.2.3): 3 is an abbreviation Z-char, not a shift, so the
/// same bytes mean something else entirely — which is the point.
#[test]
fn v1_v2_single_shift_reaches_a2_for_one_character() {
    assert_eq!(decode_zchars(1, &[6, 3, 8, 7, 5, 5]), "a1b");
    assert_eq!(decode_zchars(2, &[6, 3, 8, 7, 5, 5]), "a0b");
}

/// §3.2.2's table is cyclic, and a shift out of A2 is what proves it: Z-char 2
/// steps ONE alphabet on from wherever we are. From A2 that is A0, not A1.
///
/// `[5, 8, 2] [6, 5, 5]`: 5 locks to A2 (A0 + 2); 8 prints A2 index 2 (`1` in
/// v1, `0` in v2); 2 shifts one on from A2 → A0 for one character; 6 prints A0
/// index 0 = `a`; and the trailing locks print nothing.
#[test]
fn v1_v2_shift_from_a2_wraps_to_a0() {
    assert_eq!(decode_zchars(1, &[5, 8, 2, 6, 5, 5]), "1a");
    assert_eq!(decode_zchars(2, &[5, 8, 2, 6, 5, 5]), "0a");
}

// ---------------------------------------------------------------------------
// §3.2.2 — shift LOCK (Z-chars 4, 5)
// ---------------------------------------------------------------------------

/// The lock is the rule with no counterpart above Version 2, so this is the
/// case that separates a correct v1/v2 decoder from a v3 one wearing its
/// alphabet rows.
///
/// `[5, 8, 9] [10, 4, 6]`: 5 locks A0 + 2 = A2 *permanently*; 8, 9, 10 are then
/// A2 indices 2, 3, 4 — three digits in a row, with no shift between them; 4
/// locks one alphabet on from A2, i.e. back to A0; 6 is A0 index 0 = `a`.
///
/// Under §3.2.3's rules the same bytes read "0deA": one shift, then plain A0
/// letters. The two answers share no character but the count.
#[test]
fn v1_v2_shift_lock_holds_across_characters() {
    assert_eq!(decode_zchars(1, &[5, 8, 9, 10, 4, 6]), "123a");
    assert_eq!(decode_zchars(2, &[5, 8, 9, 10, 4, 6]), "012a");
    // The contrast, spelled out: Version 3 reads these bytes as a single shift
    // (§3.2.3) and then three A0 letters and an A1 one.
    assert_eq!(decode_zchars(3, &[5, 8, 9, 10, 4, 6]), "0deA");
}

/// A lock survives an intervening space and an intervening one-shot shift —
/// §3.2.2 makes 4/5 "permanent", and §3.2.3's "reverts after one character"
/// applies to 2/3 only, so the standing alphabet is what everything falls back
/// to.
///
/// `[5, 8, 0] [2, 6, 9]`: lock to A2; A2 index 2 (`1`/`0`); Z-char 0 is a space
/// in every version (§3.5.1) and consumes nothing; 2 shifts one on from the
/// LOCKED A2 → A0 for one character; 6 = `a`; then back to the lock, so 9 is A2
/// index 3 (`2` in v1, `1` in v2).
#[test]
fn v1_v2_lock_is_what_a_one_shot_shift_reverts_to() {
    assert_eq!(decode_zchars(1, &[5, 8, 0, 2, 6, 9]), "1 a2");
    assert_eq!(decode_zchars(2, &[5, 8, 0, 2, 6, 9]), "0 a1");
}

// ---------------------------------------------------------------------------
// §3.5.4 — the Version 1 A2 row
// ---------------------------------------------------------------------------

/// §3.5.4 gives Version 1 its own A2 row: "new-line is not needed, making room
/// for the `<` character". Reading the two rows off §3.5.3 and §3.5.4:
///
/// ```text
///  Z-char    6789abcdef0123456789abcdef
///  A2 (v2+)   ^0123456789.,!?_#'"/\-:()
///  A2 (v1)    0123456789.,!?_#'"/\<-:()
/// ```
///
/// So every glyph from the digits on sits one Z-char EARLIER in Version 1, and
/// Z-char 27 — `-` in every later version — is `<`. This walks the whole row
/// rather than sampling it, because an off-by-one here is exactly the shape of
/// defect that a spot check passes.
#[test]
fn v1_a2_row_is_the_shifted_one_with_an_angle_bracket() {
    // Z-chars 7..=31 in A2, four per string (a lock plus three glyphs, twice).
    let row_v1: String = (7u8..=31).map(|zc| decode_zchars(1, &[5, zc, 5, 5, 5, 5])).collect();
    let row_v2: String = (7u8..=31).map(|zc| decode_zchars(2, &[5, zc, 5, 5, 5, 5])).collect();
    assert_eq!(row_v1, r#"0123456789.,!?_#'"/\<-:()"#);
    // Version 2's Z-char 7 is the newline (§3.5.3's `^`), and the row then runs
    // one place behind Version 1's, ending at `)` on Z-char 31.
    assert_eq!(row_v2, "\n0123456789.,!?_#'\"/\\-:()");
}

/// §3.4 is not qualified by version: "Z-character 6 from A2 means that the two
/// subsequent Z-characters specify a ten-bit ZSCII character code". Version 1's
/// row shows a blank at that position for exactly this reason, and both Frotz
/// and Bocfel take the escape in every version, checking it ahead of any
/// version-specific handling rather than folding it into A2's per-version row.
///
/// `[5, 6, 2] [1, 5, 5]`: lock to A2; Z-char 6 opens the escape; the next two
/// Z-chars are its halves, 2 and 1, giving ZSCII (2 << 5) | 1 = 65 = `A`.
#[test]
fn v1_keeps_the_ten_bit_zscii_escape_at_a2_six() {
    assert_eq!(decode_zchars(1, &[5, 6, 2, 1, 5, 5]), "A");
    assert_eq!(decode_zchars(2, &[5, 6, 2, 1, 5, 5]), "A");
}

// ---------------------------------------------------------------------------
// §3.3 / §3.5.2 — abbreviations, and Version 1's newline
// ---------------------------------------------------------------------------

/// Build an image whose abbreviation entry `index` expands to the Z-chars
/// `expansion`. The table is 32 words at 0x0040 (§3.3: entry 32(z−1)+x, and
/// Version 2 has only z = 1, so 32 entries); each holds a WORD address, which
/// is why `EXPANSION` is even — the entry stores `EXPANSION / 2`.
fn story_with_abbreviation(version: u8, index: usize, expansion: &[u8]) -> Vec<u8> {
    let mut buf = sample_story(version);
    let entry = 0x0040 + index * 2;
    let word_addr = (EXPANSION / 2) as u16;
    buf[entry] = (word_addr >> 8) as u8;
    buf[entry + 1] = word_addr as u8;
    put_zstring(&mut buf, EXPANSION, expansion);
    buf
}

/// §3.5.2: "In Version 1, Z-character 1 is printed as a new-line (ZSCII 13)."
/// §3.3: "In Version 2, Z-character 1 has this effect" — the abbreviation
/// effect — "(but 2 and 3 do not, so there are only 32 abbreviations)."
///
/// One string, `[6, 1, 7]`, therefore means three different things. In Version 1
/// it is `a`, a newline, `b`. In Versions 2 and 3 the 1 opens an abbreviation
/// whose index is the following 7, i.e. entry 32(1−1)+7 = 7.
#[test]
fn zchar_one_is_a_newline_in_v1_and_an_abbreviation_from_v2() {
    // Entry 7 expands to "cd" (A0 indices 2 and 3 → Z-chars 8 and 9).
    let expansion = [8u8, 9, 5];

    let m = Memory::new(story_with_abbreviation(1, 7, &expansion)).unwrap();
    let mut buf = story_with_abbreviation(1, 7, &expansion);
    put_zstring(&mut buf, STRING, &[6, 1, 7]);
    let m1 = Memory::new(buf).unwrap();
    assert_eq!(decode_string(&m1, STRING as u32).0, "a\nb", "§3.5.2");
    drop(m);

    for v in [2u8, 3] {
        let mut buf = story_with_abbreviation(v, 7, &expansion);
        put_zstring(&mut buf, STRING, &[6, 1, 7]);
        let m = Memory::new(buf).unwrap();
        assert_eq!(decode_string(&m, STRING as u32).0, "acd", "v{v} §3.3");
    }
}

/// §3.3 again, the other half: in Version 2, Z-chars **2 and 3 are not**
/// abbreviations — they are §3.2.2's one-shot shifts, and only Version 3 turns
/// them into the second and third abbreviation banks.
///
/// `[6, 2, 6]` is therefore `a` then (v2) a shift into A1 and `A`, or (v3) the
/// abbreviation at entry 32(2−1)+6 = 38.
#[test]
fn v2_zchars_two_and_three_are_shifts_not_abbreviations() {
    let expansion = [8u8, 9, 5]; // "cd"

    let mut buf = story_with_abbreviation(2, 38, &expansion);
    put_zstring(&mut buf, STRING, &[6, 2, 6]);
    let m = Memory::new(buf).unwrap();
    assert_eq!(decode_string(&m, STRING as u32).0, "aA", "v2: Z-char 2 shifts A0 → A1");

    let mut buf = story_with_abbreviation(3, 38, &expansion);
    put_zstring(&mut buf, STRING, &[6, 2, 6]);
    let m = Memory::new(buf).unwrap();
    assert_eq!(decode_string(&m, STRING as u32).0, "acd", "v3: Z-char 2 opens bank 2");
}

/// Version 1 has no abbreviation Z-character at all, so a table pointed at a
/// string can never be reached — Z-char 1 is the newline and 2/3 are shifts.
/// This asserts the absence directly: the same bytes that expand in Version 2
/// print no part of the expansion in Version 1.
#[test]
fn v1_has_no_abbreviations() {
    let expansion = [8u8, 9, 5]; // "cd"
    for zc in [1u8, 2, 3] {
        let mut buf = story_with_abbreviation(1, 32 * (zc as usize - 1) + 7, &expansion);
        put_zstring(&mut buf, STRING, &[6, zc, 7]);
        let m = Memory::new(buf).unwrap();
        let got = decode_string(&m, STRING as u32).0;
        assert!(!got.contains("cd"), "v1 Z-char {zc} must not expand an abbreviation, got {got:?}");
    }
}

// ---------------------------------------------------------------------------
// §3.7 / §3.7.1 — dictionary encoding
// ---------------------------------------------------------------------------

/// §3.7: "The total string length must be 6 Z-characters (in Versions 1 to 3)",
/// which is 4 bytes, in Versions 1 and 2 as much as in 3.
#[test]
fn v1_v2_dictionary_resolution_is_six_zchars() {
    for v in [1u8, 2, 3] {
        assert_eq!(encode_word("sword", v).len(), 4, "v{v}");
    }
}

/// §3.7.1's rule, and the case that proves it is not cosmetic. Bocfel's
/// `dict.cpp` records it: Zork I's PDP-10 answers to the dictionary word
/// "pdp10", "the 1 and 0 are both in A2, and thus must be encoded with a lock,
/// not two shifts", or the machine can only be referred to by its synonyms.
///
/// Encoding "pdp10" by hand. `p` is A0 index 15 → Z-char 21; `d` is A0 index 3 →
/// Z-char 9. Then `1` and `0` are both A2 — two characters in a row from the
/// same, different alphabet — so §3.7.1 wants a LOCK. §3.2.2's table puts the
/// A0 → A2 lock at Z-char 5, and after it neither digit needs a shift:
///
/// ```text
///   v2:  21  9 21   5  9  8      (A2 indices 3 and 2 = '1' and '0')
///   v1:  21  9 21   5  8  7      (A2 indices 2 and 1 = '1' and '0', §3.5.4)
/// ```
///
/// Exactly six Z-chars — it only fits BECAUSE of the lock. Two single shifts
/// would need seven and lose the final digit, which is the bug.
#[test]
fn v1_v2_lock_two_same_alphabet_characters_when_encoding() {
    fn pack6(z: [u8; 6]) -> Vec<u8> {
        let w0 = ((z[0] as u16) << 10) | ((z[1] as u16) << 5) | z[2] as u16;
        let w1 = 0x8000 | ((z[3] as u16) << 10) | ((z[4] as u16) << 5) | z[5] as u16;
        vec![(w0 >> 8) as u8, w0 as u8, (w1 >> 8) as u8, w1 as u8]
    }
    assert_eq!(encode_word("pdp10", 1), pack6([21, 9, 21, 5, 8, 7]));
    assert_eq!(encode_word("pdp10", 2), pack6([21, 9, 21, 5, 9, 8]));
    // Version 3 has no lock (§3.7.1 is "Versions 1 and 2 only"), so the same
    // word costs a shift per digit, and the second one does not fit.
    assert_eq!(encode_word("pdp10", 3), pack6([21, 9, 21, 5, 9, 5]));
}

/// A shift is still a SHIFT where only one character needs it — §3.7.1 asks for
/// the lock only "when the next two characters come from the same alphabet".
///
/// "a1b": `a` is A0 index 0 → Z-char 6; `1` is A2 alone, so a one-shot A0 → A2
/// shift, which §3.2.2 puts at Z-char 3; `b` is A0 index 1 → Z-char 7, needing
/// no shift back because a §3.2.2 single shift lasts one character.
#[test]
fn v1_v2_use_a_single_shift_for_a_lone_character() {
    fn pack6(z: [u8; 6]) -> Vec<u8> {
        let w0 = ((z[0] as u16) << 10) | ((z[1] as u16) << 5) | z[2] as u16;
        let w1 = 0x8000 | ((z[3] as u16) << 10) | ((z[4] as u16) << 5) | z[5] as u16;
        vec![(w0 >> 8) as u8, w0 as u8, (w1 >> 8) as u8, w1 as u8]
    }
    assert_eq!(encode_word("a1b", 1), pack6([6, 3, 8, 7, 5, 5]));
    assert_eq!(encode_word("a1b", 2), pack6([6, 3, 9, 7, 5, 5]));
}

/// The encoder and the decoder must be the same rulebook read twice — a word
/// this crate encodes has to read back as itself, per version. The pairs below
/// each exercise a different branch: plain A0, a lone shift, a lock, and a
/// character (`<`) that exists in Version 1's A2 row and nowhere else.
#[test]
fn v1_v2_encode_decode_round_trip() {
    fn round_trip(v: u8, word: &str) -> String {
        let enc = encode_word(word, v);
        let mut buf = sample_story(v);
        buf[STRING..STRING + enc.len()].copy_from_slice(&enc);
        let m = Memory::new(buf).unwrap();
        decode_string(&m, STRING as u32).0
    }

    for v in [1u8, 2, 3] {
        for word in ["sword", "a1b", "x.y"] {
            assert_eq!(round_trip(v, word), word, "v{v} round trip of {word:?}");
        }
    }
    // "pdp10" survives six Z-chars only WITH §3.7.1's lock, so it round-trips
    // in Versions 1 and 2 and is truncated in Version 3 — which is the whole
    // point of the rule, and the reason Zork I's PDP-10 answers to it.
    assert_eq!(round_trip(1, "pdp10"), "pdp10");
    assert_eq!(round_trip(2, "pdp10"), "pdp10");
    assert_eq!(round_trip(3, "pdp10"), "pdp1");
    // `<` is reachable only through §3.5.4's Version 1 row.
    let enc = encode_word("a<b", 1);
    let mut buf = sample_story(1);
    buf[STRING..STRING + enc.len()].copy_from_slice(&enc);
    let m = Memory::new(buf).unwrap();
    assert_eq!(decode_string(&m, STRING as u32).0, "a<b");
}

/// The whole reason the encoder's version matters: a real v1/v2 dictionary was
/// built by a compiler following §3.2.2 and §3.7.1, so player input encoded by
/// Version 3's rules cannot match it. Here the dictionary entries are packed by
/// hand from the standard's tables, and lookup has to find them.
#[test]
fn v1_v2_dictionary_lookup_matches_a_hand_packed_entry() {
    fn pack6(z: [u8; 6]) -> [u8; 4] {
        let w0 = ((z[0] as u16) << 10) | ((z[1] as u16) << 5) | z[2] as u16;
        let w1 = 0x8000 | ((z[3] as u16) << 10) | ((z[4] as u16) << 5) | z[5] as u16;
        [(w0 >> 8) as u8, w0 as u8, (w1 >> 8) as u8, w1 as u8]
    }

    // Keys for "pdp10" and "sword", per version.
    //   sword: s=A0[18]→24, w=A0[22]→28, o=A0[14]→20, r=A0[17]→23, d=A0[3]→9,
    //          then one pad (§3.7: "The pad character, if needed, must be 5").
    let sword = pack6([24, 28, 20, 23, 9, 5]);
    let cases: [(u8, [u8; 4]); 2] = [
        (1, pack6([21, 9, 21, 5, 8, 7])),
        (2, pack6([21, 9, 21, 5, 9, 8])),
    ];

    for (v, pdp10) in cases {
        let mut buf = sample_story(v);
        let dict: usize = 0x0200;
        let mut keys = [pdp10, sword];
        keys.sort(); // a positive count declares the table sorted
        buf[dict] = 0; // no word separators
        buf[dict + 1] = 4; // entry_length: 4 bytes, the v1–v3 resolution
        buf[dict + 2] = 0;
        buf[dict + 3] = 2; // two entries, sorted
        for (i, key) in keys.iter().enumerate() {
            buf[dict + 4 + i * 4..dict + 8 + i * 4].copy_from_slice(key);
        }

        let m = Memory::new(buf).unwrap();
        let d = load(&m);
        assert_ne!(d.lookup(&m, "pdp10"), 0, "v{v}: the §3.7.1 lock is what makes this match");
        assert_ne!(d.lookup(&m, "sword"), 0, "v{v}: a plain A0 word");
        assert_eq!(d.lookup(&m, "xyzzy"), 0, "v{v}: an absent word still misses");
    }
}

/// SQ-1442. §3.7: "any multi-Z-character constructions should be left
/// incomplete (rather than omitted) if there's no room to finish them."
/// "aa11a" in Version 2 is a boundary case: 'a','a' consume two Z-chars, the
/// two consecutive '1's lock into A2 (§3.7.1) for three more, and the final
/// 'a' needs to shift back out of the lock with only ONE Z-char slot left —
/// room for the shift alone (Z-char 2), not its body. A dictionary compiled
/// correctly per §3.7 has that shift in the last slot; the bug this fixes
/// omitted the whole construction and padded with Z-char 5 there instead
/// (see `encode.rs`'s `shift_lock_left_incomplete_when_budget_exhausted_v2`
/// for the full by-hand derivation of `[6,6, 5,9,9, 2]`). The two encodings
/// differ only in that last byte, so this is the case that actually proves
/// the fix matters: `encode_word_mem` (what `lookup` uses) has to produce the
/// SAME left-incomplete bytes the dictionary was compiled with, or the word
/// never resolves.
#[test]
fn v1_v2_lookup_matches_boundary_truncated_shift() {
    fn pack6(z: [u8; 6]) -> [u8; 4] {
        let w0 = ((z[0] as u16) << 10) | ((z[1] as u16) << 5) | z[2] as u16;
        let w1 = 0x8000 | ((z[3] as u16) << 10) | ((z[4] as u16) << 5) | z[5] as u16;
        [(w0 >> 8) as u8, w0 as u8, (w1 >> 8) as u8, w1 as u8]
    }
    let aa11a = pack6([6, 6, 5, 9, 9, 2]);
    assert_eq!(
        encode_word("aa11a", 2),
        aa11a,
        "encoder must produce the left-incomplete bytes, not omit-and-pad"
    );

    let mut buf = sample_story(2);
    let dict: usize = 0x0200;
    buf[dict] = 0; // no word separators
    buf[dict + 1] = 4; // entry_length: 4 bytes, the v1–v3 resolution
    buf[dict + 2] = 0;
    buf[dict + 3] = 1; // one entry
    buf[dict + 4..dict + 8].copy_from_slice(&aa11a);

    let m = Memory::new(buf).unwrap();
    let d = load(&m);
    assert_ne!(
        d.lookup(&m, "aa11a"),
        0,
        "the §3.7 left-incomplete encoding is what makes this match"
    );
}

// ---------------------------------------------------------------------------
// §11.1 — the header fields Versions 1 and 2 do not have
// ---------------------------------------------------------------------------

/// §11.1's header table marks $1A (file length) and $1C (checksum) "3+", with
/// the note "Some early Version 3 files do not contain length and checksum
/// data". A Version 1 or 2 image has neither, so the parser must not read a
/// length out of bytes the format leaves undefined.
#[test]
fn v1_v2_have_no_length_or_checksum_words() {
    for v in [1u8, 2] {
        let mut buf = sample_story(v);
        buf[0x1A] = 0x12;
        buf[0x1B] = 0x34;
        buf[0x1C] = 0x56;
        buf[0x1D] = 0x78;
        let h = parse_header(&buf).unwrap();
        assert_eq!(h.file_length, 0, "v{v}: $1A is not a length here");
    }
}

/// And what `verify` (§15) then counts. With no declared length the sum has to
/// run to the end of the IMAGE — Frotz's `init_memory` substitutes the file's
/// own size when the header word reads 0 ("some old games lack the file size
/// entry"), and `z_verify` sums to that. Summing to $0040 instead would make
/// `verify` answer for an empty region and match a zero checksum by accident.
#[test]
fn story_checksum_falls_back_to_the_image_length() {
    let mut buf = sample_story(1);
    buf[0x0040] = 0xAB;
    buf[0x0041] = 0x02;
    let m = Machine::new(Memory::new(buf).unwrap());
    assert_eq!(
        m.story_checksum(),
        0xAB + 0x02,
        "every byte from $0040 to the end of a 0x400-byte image, and only those two are set"
    );
}

/// §11.1's "Flags 1 (in Versions 1 to 3)" table carries a `3` in its V column on
/// every documented bit, so none of them is the interpreter's to write below
/// Version 3. Bit 5 in particular ("Screen-splitting available?") would be a
/// lie: §14 puts `split_window` and `set_window` at Version 3, and §8.5's
/// Version 1/2 screen "can only be printed to (like a teletype)". Frotz writes
/// no Flags 1 bit below V3 either (`src/dumb/dumb_init.c`, `os_init_screen`).
#[test]
fn v1_v2_header_capability_bits_are_left_alone() {
    for v in [1u8, 2] {
        let mut buf = sample_story(v);
        buf[0x01] = 0b0101_0010; // an arbitrary shipped value
        let mut m = Machine::new(Memory::new(buf).unwrap());
        m.init_caps();
        assert_eq!(m.mem.read_byte(0x01), 0b0101_0010, "v{v} Flags 1 untouched");
    }
    // v3, by contrast, clears bits 4 and 6 and sets bit 5.
    let mut buf = sample_story(3);
    buf[0x01] = 0b0101_0010;
    let mut m = Machine::new(Memory::new(buf).unwrap());
    m.init_caps();
    assert_eq!(m.mem.read_byte(0x01), 0b0010_0010);
}

/// §8.2.1: "In Versions 1 and 2, all games are 'score games'. In Version 3, if
/// bit 1 of 'Flags 1' is clear then the game is a 'score game'; if it is set,
/// then the game is a 'time game'." So a Version 1 or 2 story that happens to
/// carry that bit still shows score and turns.
#[test]
fn v1_v2_are_always_score_games() {
    use zvm::screen::{compute_status_line, StatusRight};
    for v in [1u8, 2] {
        let mut buf = sample_story(v);
        buf[0x01] = 1 << 1; // the v3 "time game" bit, meaningless here
        let m = Memory::new(buf).unwrap();
        assert!(
            matches!(compute_status_line(&m).right, StatusRight::ScoreTurns { .. }),
            "v{v} must be a score game whatever bit 1 holds"
        );
    }
}

// ---------------------------------------------------------------------------
// Running a Version 1 and a Version 2 program
// ---------------------------------------------------------------------------

/// Boot each version through the public `Machine::boot` and run a real program:
/// `print` (0OP:178, ZMSD §14) with an inline Z-string, `new_line` (0OP:187),
/// then `quit` (0OP:186). The point is the whole chain — header gate, boot
/// config, instruction decode, the v1/v2 text decoder on an INLINE string —
/// not any one of them.
#[test]
fn v1_and_v2_boot_and_run_to_quit() {
    for v in [1u8, 2] {
        let mut buf = sample_story(v);
        // print "abc" — Z-chars 6, 7, 8 in A0, one word inline after the opcode.
        buf[CODE] = 0xB2;
        put_zstring(&mut buf, CODE + 1, &[6, 7, 8]);
        buf[CODE + 3] = 0xBB; // new_line
        buf[CODE + 4] = 0xBA; // quit

        let sink = Rc::new(RefCell::new(String::new()));
        let mut m = Machine::boot(
            Memory::new(buf).unwrap(),
            Box::new(SharedSink(Rc::clone(&sink))),
            BootConfig::new(),
        );
        let mut steps = 0;
        loop {
            match m.step() {
                StepResult::Quit => break,
                StepResult::Continue => {}
                other => panic!("v{v}: unexpected {other:?}"),
            }
            steps += 1;
            assert!(steps < 100, "v{v} did not reach quit");
        }
        assert_eq!(sink.borrow().as_str(), "abc\n", "v{v}");
    }
}

/// `save` is a BRANCH instruction in Versions 1 to 3 (§14, 0OP:181 "1 save
/// ?(label)"; it becomes a store only at Version 4). A Version 1 program must
/// therefore decode it with a branch byte, and the machine must suspend for the
/// host rather than mis-read the following byte as an operand.
#[test]
fn v1_save_is_a_branch_instruction() {
    let mut buf = sample_story(1);
    buf[CODE] = 0xB5; // save
    buf[CODE + 1] = 0xC2; // branch: on_true, short form, offset 2 (skip nothing)
    buf[CODE + 2] = 0xBA; // quit
    // `sample_story` puts the initial PC at CODE, so the machine boots here.
    let mut m = Machine::new(Memory::new(buf).unwrap());
    assert!(
        matches!(m.step(), StepResult::SaveRequest),
        "v1 save must suspend for the host, having consumed exactly the branch byte"
    );
}

/// §15 `read`: "In Versions 1 to 3, the status line is automatically
/// redisplayed first", and §15's buffer rules put the maximum length minus one
/// in byte 0 for Versions 1 to 4. Reaching `NeedLine` at all is what this pins —
/// a `sread` (VAR:228, Version 1) decoded with the wrong operand count would
/// never get here.
#[test]
fn v1_v2_sread_asks_the_host_for_a_line() {
    for v in [1u8, 2] {
        let mut buf = sample_story(v);
        // A one-entry dictionary so tokenising has somewhere to look.
        buf[0x0200] = 0;
        buf[0x0201] = 4;
        buf[0x0202] = 0;
        buf[0x0203] = 0;
        // Text buffer byte 0 = max letters minus 1 (§15, v1–v4);
        // parse buffer byte 0 = max words.
        buf[TEXT_BUF] = 20;
        buf[PARSE_BUF] = 4;
        // sread (VAR:228 = 0xE4), operand types byte: two large constants.
        buf[CODE] = 0xE4;
        buf[CODE + 1] = 0b0000_1111; // large, large, omitted, omitted
        buf[CODE + 2] = (TEXT_BUF >> 8) as u8;
        buf[CODE + 3] = TEXT_BUF as u8;
        buf[CODE + 4] = (PARSE_BUF >> 8) as u8;
        buf[CODE + 5] = PARSE_BUF as u8;
        buf[CODE + 6] = 0xBA; // quit

        let mut m = Machine::new(Memory::new(buf).unwrap());
        assert!(matches!(m.step(), StepResult::NeedLine { .. }), "v{v} sread must ask for a line");
    }
}
