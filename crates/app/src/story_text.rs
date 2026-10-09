//! Every word a Z-machine story can print, read once from the story file
//! (SQ-1553).
//!
//! A Version 1–3 dictionary keeps six Z-characters of a word (ZMSD 1.1 §13.3:
//! the encoded text is 4 bytes holding 6 Z-characters) and Version 4+ nine
//! (§13.4: 6 bytes, 9 Z-characters), so Zork I stores `lanter`, `mailbo` and
//! `brandi`.
//! The parser only ever needs the key, but a player reading `lanter` in a
//! completion list reads our bug. The whole word is almost always somewhere in
//! the story's own text, and this is where it is recovered from.
//!
//! # Why a scan, and not a walk from the code
//!
//! The obvious route — disassemble from the entry point and collect every string
//! a `print` or `print_paddr` reaches — finds very little: Infocom dispatches
//! most of its text through tables (object descriptions, room routines reached
//! through properties, message vectors), so a recursive descent from constant
//! call targets reaches a few hundred strings in a game that holds thousands.
//! The call graph cannot bound the pool of printable text.
//!
//! High memory can. Everything the story prints is either a Z-string in high
//! memory (§1.1.3 — packed-address strings in the string area, and inline
//! `print`/`print_ret` strings inside routines) or an object's short name in the
//! property tables (§12.4). Abbreviations (§3.3) are expanded wherever they are
//! used, so they are covered by the strings that use them. So high memory is
//! decoded end to end:
//!
//! - a decode that reads as text is taken, and the scan jumps to its end — in
//!   the string area, where strings lie back to back, that keeps every later
//!   decode on a true string boundary;
//! - one that does not read as text (code, tables) is dropped and the scan steps
//!   one byte on.
//!
//! A decode that did NOT start on a known boundary may have started part-way
//! into a word (`xamined` out of `examined`), so its first token is discarded.
//! The known boundaries are: the end of the previous string, and the byte after
//! a `print` or `print_ret` opcode byte, whose operand IS an inline string
//! (§14's table: 0OP:178 = `$B2` and 0OP:179 = `$B3`).
//!
//! # Why a false word here is cheap
//!
//! Garbage that happens to read as text contributes random letter strings, and
//! the only use made of these words is to spell out a dictionary key: a garbage
//! word is harmful only if its first six letters exactly equal a key AND it is
//! the only such word. The caller also refuses any key that more than one word
//! reaches, so an extra candidate can only make a spelling less likely to be
//! offered, never make a wrong one more likely. And whatever is shown, typing it
//! reaches the same dictionary entry, because the parser truncates the player's
//! word exactly as it truncated its own.

use std::collections::BTreeSet;

use zvm::memory::Memory;

/// `print` and `print_ret`: ZMSD §14's 0OP:178 and 0OP:179.
const PRINT: u8 = 0xB2;
const PRINT_RET: u8 = 0xB3;

/// Every word the story's own text holds, lowercased — letters only, two or
/// more of them.
///
/// Reads a private copy of the image: `Memory` latches an out-of-bounds read as
/// a fault for the CPU to raise, and a scan that decodes code as text will read
/// wild abbreviation addresses. That must never reach the live machine.
pub fn zmachine_words(live: &Memory) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    scan_text(live, |text, at_boundary| harvest(text, !at_boundary, &mut out));
    out
}

/// Does the story's own text carry Infocom's stock InvisiClues booklet advert
/// (`[If you're really stuck, maps and InvisiClues(TM) Hint Booklets are
/// available...]`)? That is all the `HINT` verb prints in such a story, so a
/// `hint` dictionary entry there is no evidence of built-in hints (SQ-1745).
///
/// Matched as a phrase inside one decoded string, not as loose words, so a story
/// with real hints that merely mentions the booklets elsewhere is not caught
/// unless one string holds both names.
pub fn advertises_hint_booklet(live: &Memory) -> bool {
    let mut found = false;
    scan_text(live, |text, _| {
        let lower = text.to_ascii_lowercase();
        if lower.contains("invisiclues") && lower.contains("hint booklet") {
            found = true;
        }
    });
    found
}

/// Walk every string the story can print — high memory decoded end to end, then
/// object short names — calling `f(text, at_boundary)`; `at_boundary` is false
/// when the decode may have begun part-way into a word.
fn scan_text(live: &Memory, mut f: impl FnMut(&str, bool)) {
    let Ok(mem) = Memory::new(live.raw_bytes().to_vec()) else {
        return;
    };
    let high = (mem.read_word(0x04) as u32).min(mem.len() as u32);
    let end = mem.len() as u32;
    let mut p = high;
    let mut synced = false;
    while p + 1 < end {
        let (text, next) = zvm::text::decode_string(&mem, p);
        if reads_as_text(&text) {
            let at_boundary = synced || matches!(mem.read_byte(p.saturating_sub(1)), PRINT | PRINT_RET);
            f(&text, at_boundary);
            p = next.max(p + 1);
            synced = true;
        } else {
            p += 1;
            synced = false;
        }
    }
    for obj in 1..=zvm::objects::object_count(&mem) {
        f(&zvm::objects::short_name(&mem, obj), true);
    }
}

/// Add `text`'s words to `out`, skipping the first when the decode may have
/// begun mid-word.
fn harvest(text: &str, skip_first: bool, out: &mut BTreeSet<String>) {
    let mut tokens = text.split(|c: char| !c.is_ascii_alphabetic());
    if skip_first {
        tokens.next();
    }
    for t in tokens {
        if t.len() >= 2 {
            out.insert(t.to_ascii_lowercase());
        }
    }
}

/// Does a decode look like prose rather than code or a table read as Z-text?
///
/// Decoded code is mostly lowercase letters too (alphabet A0 is Z-characters
/// 6–31), so "is it letters" is not enough. Three tests, each cheap and each
/// failed by random letters far more often than by any sentence:
///
/// - at least 90% of characters are letters, digits, spaces or common
///   punctuation (a ZSCII escape or a control code is rare in real text);
/// - vowels (with `y`) make up between a quarter and two thirds of the letters —
///   English runs near 40%, uniform random letters near 23%;
/// - no word holds five consonants in a row.
fn reads_as_text(s: &str) -> bool {
    let n = s.chars().count();
    if n < 2 {
        return false;
    }
    let ordinary = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || " .,!?'\"-:;()\n".contains(*c))
        .count();
    if ordinary * 10 < n * 9 {
        return false;
    }
    let is_vowel = |c: char| "aeiouyAEIOUY".contains(c);
    let (mut letters, mut vowels, mut run) = (0usize, 0usize, 0usize);
    for c in s.chars() {
        if c.is_ascii_alphabetic() {
            letters += 1;
            if is_vowel(c) {
                vowels += 1;
                run = 0;
            } else {
                run += 1;
                if run >= 5 {
                    return false;
                }
            }
        } else {
            run = 0;
        }
    }
    letters > 0 && vowels * 4 >= letters && vowels * 3 <= letters * 2
}

#[cfg(all(test, feature = "t-guidance"))]
mod tests {
    use super::*;

    #[test]
    fn prose_reads_as_text() {
        assert!(reads_as_text("You are standing in an open field west of a white house."));
        assert!(reads_as_text("lantern"));
    }

    #[test]
    fn random_letters_and_codes_do_not() {
        assert!(!reads_as_text("qkzvbx mptr"));
        assert!(!reads_as_text("x"));
        assert!(!reads_as_text("\u{1}\u{2}ab\u{3}\u{4}"));
        assert!(!reads_as_text("ddddde"));
    }

    #[test]
    fn an_unanchored_decode_loses_its_first_word() {
        let mut out = BTreeSet::new();
        harvest("xamined the lantern", true, &mut out);
        assert!(!out.contains("xamined"));
        assert!(out.contains("lantern") && out.contains("the"));
        harvest("brass lantern", false, &mut out);
        assert!(out.contains("brass"));
    }

    fn load(rel: &str) -> Option<Memory> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(rel);
        Memory::new(std::fs::read(p).ok()?).ok()
    }

    fn has_hint_word(mem: &Memory) -> bool {
        zvm::dictionary::load(mem)
            .words(mem)
            .iter()
            .any(|w| w.eq_ignore_ascii_case("hint") || w.eq_ignore_ascii_case("hints"))
    }

    /// Beyond Zork r57: `hint` is in the dictionary but only prints the booklet advert.
    #[test]
    fn beyond_zork_hint_is_only_the_booklet_advert() {
        let Some(mem) = load("beyondzork-r57-s871221.z5") else {
            eprintln!("SKIP: stories/beyondzork-r57-s871221.z5 absent");
            return;
        };
        assert!(has_hint_word(&mem), "non-vacuity: the dictionary has `hint`");
        assert!(advertises_hint_booklet(&mem));
    }

    /// Lost Pig has a real HINT verb and no booklet advert: still counts as built-in hints.
    #[test]
    fn a_story_with_real_hints_is_not_flagged() {
        let Some(mem) = load("LostPig.z8") else {
            eprintln!("SKIP: stories/LostPig.z8 absent");
            return;
        };
        assert!(has_hint_word(&mem));
        assert!(!advertises_hint_booklet(&mem));
    }
}
