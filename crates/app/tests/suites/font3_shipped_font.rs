//! The font-3 translation table, checked against the font Infocom shipped (SQ-0915).
//!
//! `zvm::cpu::exec::font3_translate` turns the Z-machine's character-graphics font
//! (ZMSD §16) into Unicode, because a terminal draws characters and not bitmaps. The
//! table came from bocfel (MIT-licensed as of v2.2.3, the version read), which is a
//! faithful reading of the *standard* — but the standard describes the glyphs in
//! prose, and what a player actually saw is what their machine drew.
//!
//! *Beyond Zork* shipped that font on the floppy. `Graphic.Data` on *Lost Treasures*
//! Amiga disk 5 is an 8×8 Amiga disk font — not artwork, despite the name, and not a
//! program despite the `$3F3` hunk header, which is the four-byte `moveq/rts` stub
//! every Amiga font begins with. It carries the whole of font 3: arrows, diagonals,
//! rules, the box-drawing set, blocks, meter bars, and the 26 runes.
//!
//! **This is the only oracle here that can falsify the QUESTION rather than the
//! answer.** Every other test asks whether lanthorn does what the table says; this
//! one asks whether the table is right, and it immediately found that codes 71–74
//! put `U+FFFD` on screen where the Amiga inked a corner pixel.
//!
//! # What is deliberately NOT claimed
//!
//! Unicode cannot express all of these. Codes 38/39 are rules at two different
//! heights and both become `─`; 79–87 are meter segments with top and bottom rails
//! and become the graded left-blocks without them; 123–126 are reverse-video
//! variants of 92–96 and lose the reversal. Those are approximations the terminal
//! forces, and this suite asserts none of them away — it asserts the one thing that
//! is unambiguously wrong whatever your rendering budget, which is drawing nothing
//! recognisable at all where the machine drew ink.
//!
//! The parser is `blorb::amiga_font`, shared with `native_disk_font.rs` — this suite
//! carried its own copy until SQ-0916 gave the loader a production one, and two
//! parsers for one format is exactly how a fixture and its reader drift apart.
//!
//! Fixtures are gitignored, so every case skips vacuously when one is absent.

use std::path::PathBuf;

fn treasures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../treasures")
}

/// `Graphic.Data` off the Beyond Zork volume of the Amiga *Lost Treasures* set.
fn shipped_font() -> Option<blorb::bitmap_font::BitmapFont> {
    let disk = treasures_dir().join("Lost Treasures of Infocom, The_Disk5.adf");
    if !disk.is_file() {
        eprintln!("SKIP: gitignored Lost Treasures disk 5 absent");
        return None;
    }
    let raw = std::fs::read(&disk).ok()?;
    let vol = blorb::medium::MountedDisk::mount(raw).ok()?;
    let (_, bytes) = vol
        .contents()
        .into_iter()
        .find(|(n, _)| n.rsplit('/').next().is_some_and(|f| f.eq_ignore_ascii_case("Graphic.Data")))?;
    let font = blorb::amiga_font::parse(&bytes)
        .expect("Graphic.Data should parse as an Amiga disk font");
    Some(font)
}

/// The file really is an 8×8 disk font covering font 3's whole range.
///
/// A non-vacuity guard for the two cases below: if this file ever stops parsing, or
/// stops covering 32..=126, they would pass by skipping rather than by agreeing.
#[test]
fn beyond_zorks_graphic_data_is_an_eight_by_eight_font_covering_font_three() {
    let Some(font) = shipped_font() else { return };
    assert_eq!((font.width, font.height), (8, 8), "font 3 is the 8x8 set");
    assert_eq!(font.lo, 32, "font 3 starts at the space");
    for code in 32u8..=126 {
        assert!(font.glyph(code).is_some(), "no glyph for font-3 code {code}");
    }
    let inked = (32u8..=126)
        .filter(|&c| font.glyph(c).is_some_and(|g| g.rows.iter().any(|&r| r != 0)))
        .count();
    assert!(inked >= 80, "only {inked} of 95 codes carry ink — this is not font 3");
}

/// **No code the Amiga inks may translate to a replacement character.**
///
/// This is the case that found the defect. Codes 71–74 are unassigned in ZMSD §16,
/// so bocfel renders them `U+FFFD` and we copied that — but the shipped font draws a
/// corner pixel for each, which means *Beyond Zork* can print them and a player saw
/// `\u{FFFD}` where the Amiga drew a mark.
///
/// Falsified by restoring `71..=74 => '\u{FFFD}'`, which names all four here.
#[test]
fn every_code_the_amiga_draws_translates_to_something_drawable() {
    let Some(font) = shipped_font() else { return };
    // The ONE documented exception, and it is an approximation rather than a hole.
    // Code 79 is the EMPTY member of the meter-bar family at 79-87: its only ink is
    // the top and bottom rail, and 80-87 — which we render as the graded left-blocks
    // `▏▎▍▌▋▊▉█` — drop those same rails. So the family reads as a gradient that
    // starts at nothing, and a space is the consistent zero. Unicode has no
    // "rails-only" cell to do better with.
    const EMPTY_METER_BAR: u8 = 79;
    let mut lost = Vec::new();
    for code in 32u8..=126 {
        let Some(glyph) = font.glyph(code) else { continue };
        if glyph.rows.iter().all(|&r| r == 0) || code == EMPTY_METER_BAR {
            continue; // the machine draws nothing, so a space is a faithful answer
        }
        let ch = zvm::cpu::exec::font3_translate(char::from(code));
        if ch == '\u{FFFD}' || ch == ' ' {
            lost.push((code, ch));
        }
    }
    // …and the exception has to keep earning itself: if 79 ever stops being the
    // rails-only glyph, the reasoning above no longer applies to it.
    let bar = &font.glyph(EMPTY_METER_BAR).expect("code 79 is in range").rows;
    assert_eq!(
        bar.iter().filter(|&&r| r == 0xFF).count(),
        2,
        "code 79 is excused as the empty meter bar, whose ink is exactly two rails: {bar:02X?}",
    );
    assert!(bar.iter().all(|&r| r == 0xFF || r == 0), "code 79's rails are full rows: {bar:02X?}");
    assert!(
        lost.is_empty(),
        "these font-3 codes are INKED in the font Infocom shipped with Beyond Zork, yet \
         translate to nothing a reader can see: {lost:?}\n\
         Unicode cannot match every glyph exactly and this suite does not ask it to — but a \
         replacement character, or a blank, where the Amiga drew ink is not an approximation, \
         it is a hole (SQ-0915).",
    );
}

/// Spot checks: the shipped ink agrees with what the table claims the glyph IS.
///
/// Pinned as corner/edge occupancy rather than exact bitmaps, because the assertion
/// is about which glyph a code denotes, not about Infocom's letterforms. A left
/// arrow inks the left edge and a right arrow does not, whatever its exact shape.
#[test]
fn the_shipped_glyphs_agree_with_the_table_where_the_shape_is_unambiguous() {
    let Some(font) = shipped_font() else { return };
    let ink = |code: u8| font.glyph(code).map_or_else(|| vec![0u8; 8], |g| g.rows.clone());
    let any = |g: Vec<u8>, rows: std::ops::Range<usize>, mask: u8| {
        rows.into_iter().any(|y| g[y] & mask != 0)
    };

    // 33/34 are the horizontal cursor arrows: each reaches its own edge.
    assert!(any(ink(33), 0..8, 0x80), "code 33 (←) should ink the left edge");
    assert!(any(ink(34), 0..8, 0x02), "code 34 (→) should ink the right side");
    // 40/41 are verticals and 38/39 horizontals — neither is the other.
    assert!(
        ink(40).iter().all(|r| r.count_ones() == 1),
        "code 40 (│) is a vertical: one pixel on every row, got {:02X?}",
        ink(40),
    );
    assert_eq!(ink(38).iter().filter(|&&r| r == 0xFF).count(), 1, "code 38 (─) is one full row");
    // 54 is the solid block, 32 and 37 are blank.
    assert_eq!(ink(54), vec![0xFF; 8], "code 54 (█) is the full cell");
    assert_eq!(ink(32), vec![0u8; 8], "code 32 is the space");
    assert_eq!(ink(37), vec![0u8; 8], "code 37 is blank in the table and blank on the disk");
    // 71-74 are the corner pixels the table used to lose: one corner each, and each
    // a DIFFERENT corner, which is what makes four separate codepoints necessary.
    for (code, row, mask, corner) in
        [(71u8, 0usize, 0x01u8, "upper-right"), (72, 7, 0x01, "lower-right"),
         (73, 7, 0x80, "lower-left"), (74, 0, 0x80, "upper-left")]
    {
        let g = ink(code);
        assert_eq!(g[row] & mask, mask, "code {code} should ink the {corner} corner");
        assert_eq!(g.iter().map(|r| r.count_ones()).sum::<u32>(), 1, "code {code} is ONE pixel");
    }
    // The 26 runes all carry ink and all land in Unicode's runic block.
    for code in 97u8..=122 {
        assert_ne!(ink(code), vec![0u8; 8], "rune at code {code} should be drawn");
        let ch = zvm::cpu::exec::font3_translate(char::from(code)) as u32;
        assert!(
            (0x16A0..=0x16F8).contains(&ch) || (0x15BE..=0x15BE).contains(&ch),
            "code {code} should translate into the runic block, got U+{ch:04X}",
        );
    }
}
