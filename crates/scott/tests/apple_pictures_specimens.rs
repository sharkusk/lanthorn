//! Picture family D (SQ-1476) against the seven Apple II *Scott Adams Graphic
//! Adventure* releases of §10.6 — and against the specification, which
//! describes a different format.
//!
//! # What this suite is for
//!
//! `docs/internals/scott-dialects-spec.md` §8.4 says family D is a simulated
//! hi-res PAGE, filled by a three-way address interleave and resolved by an
//! artifact colour model. The four **plain** releases' artwork is nothing of
//! the sort: it is an opcode stream of absolute coordinates
//! ([`scott::apple_pictures`] states the format and how it was settled). This
//! suite is the measurement that settles it, run over the real disks:
//!
//! - **the token census** — under the module's reading, every command byte in
//!   all four titles is one of the four commands and **no coordinate lands off
//!   the 280 x 192 canvas**, which is what falsifies the eight-bit-*x* reading
//!   the same bytes also admit;
//! - **the attribute census** (SQ-1489) — with `0x60` read as a two-byte
//!   token, only eighteen distinct bit-7-clear opcodes remain in the whole
//!   corpus, every `0x60` is followed by an operand, and no operand exceeds
//!   the last entry of the paint table;
//! - **the colour** — the darkness card is white lettering on black, the
//!   inventory card white on black, and the Adventure International logo
//!   green, blue and orange, which a wrong ground or a wrong palette cannot
//!   produce;
//! - **the reserved indices** — §8.6 says 0 is the darkness picture, 98 the
//!   inventory backdrop and 99 the title picture, and all four titles carry
//!   all three under the naming rule this crate implements;
//! - **the three scrambled releases** (SQ-1490) — their side A is not a DOS
//!   3.3 disk at all, so there is no catalogue to walk and the records are
//!   found by their own §8.4 header; this suite pins how many there are per
//!   title, that the ordinal is the picture index, and the colours of the
//!   cards that prove it.
//!
//! # Getting the corpus
//!
//! Commercial game files, not redistributable, not committed. Everything here
//! **skips vacuously with an explanation** when the disks are absent — a
//! silent skip reads exactly like a pass, so the skip says why.
//!
//! ```text
//! <fixtures>/apple/Scott Adams Graphic Adventure <n> - <title> … side A.dsk
//! <fixtures>/apple/Scott Adams Graphic Adventure <n> - <title> … side B ….dsk
//! ```
//!
//! where `<fixtures>` is `$SCOTT_DIALECT_FIXTURES` or `stories/scott-dialects`;
//! §10.6 has the provenance and the per-side digests.
//!
//! # The DOS 3.3 walk in this file
//!
//! `scott` takes no dependencies and reads no disk images — the host does that
//! ([`blorb::medium`] in lanthorn). The walk below is **test scaffolding**, not
//! a second implementation: it exists so this suite can reach a real specimen,
//! and it reads §7.4's flat-sector geometry (35 tracks of 16 256-byte sectors,
//! a VTOC at track 17 sector 0, a catalogue chain of seven 35-byte entries per
//! sector, and track/sector lists of 122 pairs).

use std::collections::BTreeMap;
use std::path::PathBuf;

use scott::apple_pictures::{
    decode_family_d, decode_family_d_scrambled, scan_scrambled_pictures, CANVAS_HEIGHT,
    CANVAS_WIDTH, PALETTE,
};
use scott::{
    parse_apple_picture_file_name, room_picture_file_name, PictureUsage, SagaPlatform, SagaUs,
};

/// The same three candidates the other dialect suites try, in the same order.
fn fixtures() -> Option<PathBuf> {
    [
        std::env::var_os("SCOTT_DIALECT_FIXTURES").map(PathBuf::from),
        Some(PathBuf::from("stories/scott-dialects")),
        Some(PathBuf::from("../../stories/scott-dialects")),
    ]
    .into_iter()
    .flatten()
    .find(|p| p.is_dir())
}

fn skipped(what: &str) -> bool {
    eprintln!(
        "SKIP: {what} — needs the Apple II sides under stories/scott-dialects/apple/ \
         (see this file's header for provenance)"
    );
    true
}

// ── DOS 3.3 test scaffolding (§7.4; see the module header) ───────────────────

fn sector(raw: &[u8], track: usize, sec: usize) -> Option<&[u8]> {
    let at = track * 4096 + sec * 256;
    raw.get(at..at + 256)
}

/// §7.4's per-byte filename normalisation, then trailing spaces trimmed.
fn normalise(raw: &[u8]) -> String {
    let mut s = String::new();
    for &c in raw {
        let ch = if c & 0x80 != 0 {
            if c >= 0xA0 {
                c & 0x7F
            } else {
                (c & 0x7F) + 0x20
            }
        } else {
            ((c & 0x3F) ^ 0x20) + 0x20
        };
        s.push(char::from(ch));
    }
    s.trim_end_matches(' ').to_string()
}

/// A file's data, following its track/sector list.
fn read_file(raw: &[u8], mut track: usize, mut sec: usize) -> Vec<u8> {
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    while track != 0 && track < 35 && sec < 16 && seen.insert((track, sec)) && seen.len() <= 32 {
        let Some(list) = sector(raw, track, sec) else { break };
        let (next_t, next_s) = (usize::from(list[1]), usize::from(list[2]));
        let mut chunk: Vec<(usize, usize)> = (0..122)
            .map(|i| (usize::from(list[0x0C + i * 2]), usize::from(list[0x0C + i * 2 + 1])))
            .collect();
        if next_t == 0 {
            while chunk.last() == Some(&(0, 0)) {
                chunk.pop();
            }
        }
        pairs.extend(chunk);
        (track, sec) = (next_t, next_s);
    }
    let mut out = Vec::new();
    for (t, s) in pairs {
        match sector(raw, t, s) {
            // (0, 0) is a sparse sector: 256 logical zero bytes, no read.
            _ if t == 0 && s == 0 => out.extend_from_slice(&[0u8; 256]),
            Some(bytes) => out.extend_from_slice(bytes),
            None => break,
        }
    }
    out
}

/// Every catalogued file on a DOS 3.3 image, by normalised name.
///
/// Empty when the image has no readable VTOC, which is the honest answer for
/// the three scrambled releases' side A.
fn dos33_contents(raw: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let Some(vtoc) = sector(raw, 17, 0) else { return out };
    let (mut track, mut sec) = (usize::from(vtoc[1]), usize::from(vtoc[2]));
    if track >= 35 || sec >= 16 {
        return out;
    }
    let mut seen = std::collections::HashSet::new();
    while track != 0 && track < 35 && sec < 16 && seen.insert((track, sec)) {
        let Some(cat) = sector(raw, track, sec) else { break };
        for slot in 0..7 {
            let e = &cat[0x0B + slot * 35..0x0B + (slot + 1) * 35];
            if e[0] == 0x00 || e[0] == 0xFF {
                continue;
            }
            let name = normalise(&e[3..33]);
            let data = read_file(raw, usize::from(e[0]), usize::from(e[1]));
            out.insert(name, data);
        }
        (track, sec) = (usize::from(cat[1]), usize::from(cat[2]));
    }
    out
}

// ── The corpus ───────────────────────────────────────────────────────────────

/// One plain release: adventure number, side-A file name, and the counts
/// §10.6's disks actually carry.
struct Plain {
    adventure: u16,
    title: &'static str,
    side_a: &'static str,
    rooms: usize,
    objects: usize,
}

const PLAIN: [Plain; 4] = [
    Plain {
        adventure: 1,
        title: "Adventureland",
        side_a: "Scott Adams Graphic Adventure 1 - Adventureland v2.1-416 (4am crack) side A.dsk",
        rooms: 48,
        objects: 45,
    },
    Plain {
        adventure: 2,
        title: "Pirate Adventure",
        side_a: "Scott Adams Graphic Adventure 2 - Pirate Adventure v2.1-408 (4am crack) side A.dsk",
        rooms: 39,
        objects: 49,
    },
    Plain {
        adventure: 3,
        title: "Mission Impossible",
        side_a: "Scott Adams Graphic Adventure 3 - Mission Impossible v2.1-306 (4am crack) side A.dsk",
        rooms: 33,
        objects: 30,
    },
    Plain {
        adventure: 6,
        title: "Strange Odyssey",
        side_a: "Scott Adams Graphic Adventure 6 - Strange Odyssey v2.1-119 (4am crack) side A.dsk",
        rooms: 38,
        objects: 32,
    },
];

/// One card's colour census: what it is, which record, and how many pixels of
/// each [`PALETTE`] colour it resolves to.
type Card = (&'static str, usize, [usize; 6]);

/// The three §10.6 releases whose `M2` carries §7.4's string and whose side A
/// is not a DOS 3.3 disk: title, file-name stem, records on side A, the
/// release's highest room number, and three cards pinned by colour (SQ-1490).
///
/// The **death card** is the load-bearing one: it is the picture numbered with
/// the release's LAST room, so it fails if any spurious header before it has
/// shifted the numbering. *Voodoo Castle*'s room 25 is "lot of TROUBLE!",
/// *The Count*'s 22 is "LOT OF TROUBLE! (And so Are you!)", and *Claymorgue
/// Castle*'s 32 is "real mess!".
const SCRAMBLED: [(&str, &str, usize, usize, [Card; 3]); 3] = [
    (
        "Voodoo Castle",
        "Scott Adams Graphic Adventure 4 - Voodoo Castle v2.1-119 (4am crack) side ",
        36,
        25,
        [
            ("darkness card", 0, [41401, 0, 0, 270, 298, 2831]),
            ("start room", 1, [8504, 124, 2, 12745, 5677, 17748]),
            ("death card", 25, [15863, 16817, 19, 173, 5613, 6315]),
        ],
    ),
    (
        "The Count",
        "Scott Adams Graphic Adventure 5 - The Count v2.1-115 (4am crack) side ",
        26,
        22,
        [
            ("darkness card", 0, [41176, 228, 252, 0, 0, 3144]),
            ("start room", 1, [9210, 0, 0, 434, 20878, 14278]),
            ("death card", 22, [32270, 695, 1878, 58, 5432, 4467]),
        ],
    ),
    (
        "Claymorgue Castle",
        "Scott Adams Graphic Adventure 13 - The Sorcerer of Claymorgue Castle v2.2-122 (4am crack) side ",
        35,
        32,
        [
            ("darkness card", 0, [43689, 136, 83, 0, 0, 892]),
            ("start room", 1, [14402, 32, 12418, 11571, 1440, 4937]),
            ("death card", 32, [9386, 3474, 26486, 3, 2, 5449]),
        ],
    ),
];

fn apple_dir() -> Option<PathBuf> {
    let d = fixtures()?.join("apple");
    d.is_dir().then_some(d)
}

fn image(name: &str) -> Option<Vec<u8>> {
    std::fs::read(apple_dir()?.join(name)).ok()
}

/// Every family-D picture file on one plain release's side A, by name.
fn pictures(p: &Plain) -> Option<BTreeMap<String, Vec<u8>>> {
    let raw = image(p.side_a)?;
    Some(
        dos33_contents(&raw)
            .into_iter()
            .filter(|(name, _)| {
                parse_apple_picture_file_name(name)
                    .is_some_and(|(adventure, _)| adventure == p.adventure)
            })
            .collect(),
    )
}

fn release(adventure: u16) -> SagaUs {
    SagaUs { version: 0, adventure, platform: SagaPlatform::AppleII }
}

// ── The token census: the measurement that settles the format ────────────────

/// Walk one file's opcode stream the way [`decode_family_d`] does and report
/// (three-byte tokens, one-byte tokens, coordinates off the canvas, command
/// bytes that are none of the four).
///
/// This is deliberately a second, independent walk over the same bytes: it
/// asserts a property of the DATA — that the reading fits — rather than
/// re-testing the decoder.
#[derive(Default, Clone, Copy)]
struct Census {
    /// Three-byte drawing tokens.
    tokens: usize,
    /// Attribute opcodes — bit-7-clear bytes that are NOT a `0x60` operand.
    attributes: usize,
    /// `0x60` tokens, each of which must consume the byte after it.
    paints: usize,
    /// `0x60` tokens whose operand is not itself a bit-7-clear byte, which is
    /// what a wrong "two-byte token" reading would produce.
    paints_without_an_operand: usize,
    /// The largest `0x60` operand seen.
    largest_paint: usize,
    /// Coordinates off the 280 x 192 canvas.
    off_canvas: usize,
    /// Command bytes outside the four drawing commands.
    unknown_commands: usize,
    /// Attribute opcodes outside `0x00`, `0x20`-`0x2F`, `0x40`-`0x4F`, `0x60`.
    unknown_attributes: usize,
    /// End-of-picture tokens (the `0x00`-`0x1F` class).
    ends: usize,
    /// Bytes left after the first end token — zero on every specimen.
    bytes_after_the_end: usize,
}

fn census(file: &[u8]) -> Census {
    let declared = usize::from(u16::from_le_bytes([file[2], file[3]]));
    let data = &file[4..(4 + declared).min(file.len())];
    let mut c = Census::default();
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        if b & 0x80 == 0 {
            match b >> 5 {
                0 => {
                    c.ends += 1;
                    c.bytes_after_the_end += data.len() - (i + 1);
                    c.attributes += 1;
                    break;
                }
                1 | 2 => {
                    c.attributes += 1;
                    i += 1;
                }
                _ => {
                    c.attributes += 1;
                    c.paints += 1;
                    match data.get(i + 1) {
                        Some(&v) if v & 0x80 == 0 => {
                            c.largest_paint = c.largest_paint.max(usize::from(v));
                        }
                        _ => c.paints_without_an_operand += 1,
                    }
                    i += 2;
                }
            }
            if !matches!(b & 0xE0, 0x00 | 0x20 | 0x40 | 0x60) {
                c.unknown_attributes += 1;
            }
            continue;
        }
        if i + 3 > data.len() {
            break;
        }
        let x = usize::from(data[i + 1]) | (usize::from(b & 1) << 8);
        let y = usize::from(data[i + 2]);
        i += 3;
        c.tokens += 1;
        if !matches!(b & 0xE0, 0x80 | 0xA0 | 0xC0 | 0xE0) {
            c.unknown_commands += 1;
        }
        if x >= CANVAS_WIDTH || y >= CANVAS_HEIGHT {
            c.off_canvas += 1;
        }
    }
    c
}

#[test]
fn every_command_byte_is_one_of_the_four_and_almost_every_point_is_on_the_canvas() {
    let Some(_) = apple_dir() else {
        assert!(skipped("apple II token census"));
        return;
    };
    let (mut all, mut files) = (Census::default(), 0usize);
    for p in &PLAIN {
        let Some(pics) = pictures(p) else {
            assert!(skipped(p.title));
            return;
        };
        for file in pics.values() {
            let c = census(file);
            all.tokens += c.tokens;
            all.attributes += c.attributes;
            all.paints += c.paints;
            all.paints_without_an_operand += c.paints_without_an_operand;
            all.largest_paint = all.largest_paint.max(c.largest_paint);
            all.off_canvas += c.off_canvas;
            all.unknown_commands += c.unknown_commands;
            all.unknown_attributes += c.unknown_attributes;
            all.ends += c.ends;
            all.bytes_after_the_end += c.bytes_after_the_end;
            files += 1;
        }
    }
    eprintln!(
        "family D census: {files} files, {} tokens, {} attributes, {} paints, {} off-canvas",
        all.tokens, all.attributes, all.paints, all.off_canvas
    );
    assert!(files >= 300, "only {files} picture files across the four plain releases");
    assert_eq!(all.unknown_commands, 0, "a command byte outside the four, in {} tokens", all.tokens);
    assert_eq!(all.unknown_attributes, 0, "an attribute opcode outside the four classes");
    // The measurement the format's reading rests on. Read with an eight-bit
    // x instead of the ninth bit this decoder takes from the command byte,
    // thousands of these land off the canvas.
    assert_eq!(all.off_canvas, 0, "of {} coordinates, {} are off the canvas", all.tokens, all.off_canvas);
    assert_eq!((all.tokens, all.attributes), (71_899, 4_295), "the census these disks give");
}

/// SQ-1489's half of the census: `0x60` is the only **two-byte** opcode, its
/// operand is always there and always inside the paint table, and the
/// `0x00`-class token ends the picture exactly once per file at the declared
/// end of its stream.
///
/// Three things this would fail. Reading `0x60` as a one-byte token: its
/// operands then read as opcodes, and 87 of them are values no opcode has.
/// A paint table of the wrong length: the largest operand in the corpus is
/// `0x6B` and the table has 0x6C entries, so one entry either way is
/// falsifiable. And a stream that keeps going past its end token: every file
/// here stops dead, which is what makes "end of picture" the right reading of
/// the class rather than "end of path".
#[test]
fn the_paint_token_always_carries_an_operand_and_the_stream_ends_where_it_says() {
    let Some(_) = apple_dir() else {
        assert!(skipped("apple II attribute census"));
        return;
    };
    let mut all = Census::default();
    let mut files = 0usize;
    let mut opcodes = BTreeMap::new();
    for p in &PLAIN {
        let Some(pics) = pictures(p) else {
            assert!(skipped(p.title));
            return;
        };
        for file in pics.values() {
            let c = census(file);
            all.paints += c.paints;
            all.paints_without_an_operand += c.paints_without_an_operand;
            all.largest_paint = all.largest_paint.max(c.largest_paint);
            all.ends += c.ends;
            all.bytes_after_the_end += c.bytes_after_the_end;
            files += 1;

            // …and, separately, which attribute opcodes actually occur.
            let declared = usize::from(u16::from_le_bytes([file[2], file[3]]));
            let data = &file[4..(4 + declared).min(file.len())];
            let mut i = 0;
            while i < data.len() {
                let b = data[i];
                if b & 0x80 != 0 {
                    i += 3;
                    continue;
                }
                *opcodes.entry(b).or_insert(0usize) += 1;
                if b >> 5 == 0 {
                    break;
                }
                i += if b >> 5 == 3 { 2 } else { 1 };
            }
        }
    }
    eprintln!("family D attributes: {opcodes:?}");
    assert_eq!(all.paints_without_an_operand, 0, "a 0x60 with no operand, of {} ", all.paints);
    assert_eq!(all.paints, 1_977, "the paint tokens these disks give");
    assert_eq!(all.largest_paint, 0x6B, "the largest paint operand, one short of the table's length");
    assert_eq!(all.ends, files, "one end-of-picture token per file, {files} files");
    assert_eq!(all.bytes_after_the_end, 0, "a stream carrying bytes past its end token");
    assert_eq!(opcodes.len(), 19, "distinct attribute opcodes: {opcodes:?}");
    assert_eq!(opcodes[&0x00], files, "0x00 is the end token and occurs once a file");
    for (&b, &n) in &opcodes {
        assert!(
            matches!(b, 0x00 | 0x20..=0x27 | 0x40..=0x47 | 0x53 | 0x60),
            "attribute opcode {b:#04X} occurs {n} times and is not one of the nineteen"
        );
    }
}

#[test]
fn every_picture_on_every_plain_release_decodes() {
    let Some(_) = apple_dir() else {
        assert!(skipped("apple II picture decode"));
        return;
    };
    let mut flat = 0usize;
    for p in &PLAIN {
        let Some(pics) = pictures(p) else {
            assert!(skipped(p.title));
            return;
        };
        let (mut rooms, mut objects) = (0, 0);
        for (name, file) in &pics {
            let pic = decode_family_d(file, SagaPlatform::AppleII)
                .unwrap_or_else(|e| panic!("{} {name}: {e}", p.title));
            assert_eq!((pic.width, pic.height), (CANVAS_WIDTH, CANVAS_HEIGHT));
            let mut seen = [false; PALETTE.len()];
            for &v in &pic.pixels {
                let v = usize::from(v);
                assert!(v < PALETTE.len(), "{} {name}: pixel value {v} has no colour", p.title);
                seen[v] = true;
            }
            if seen.iter().filter(|&&s| s).count() == 1 {
                flat += 1;
            }
            match parse_apple_picture_file_name(name).expect("named").1.usage {
                PictureUsage::Room => rooms += 1,
                _ => objects += 1,
            }
        }
        assert_eq!((rooms, objects), (p.rooms, p.objects), "{} picture counts", p.title);
    }
    // A handful of records are stubs — *Strange Odyssey* ships three files
    // with no tokens in them at all, and a few more draw nothing but an
    // unbounded area — so this is "nearly all", not "all". Six of 314.
    assert_eq!(flat, 6, "pictures that resolve to one flat colour");
}

/// The three reserved indices §8.6 names, and what colours they are.
///
/// This is the pin that says the ground is WHITE and the paint model runs.
/// The darkness card is white lettering on a black flood; the inventory
/// backdrop is the other way round; and the Adventure International logo is
/// **the same drawing on all four disks**, green and blue and orange, which no
/// two-colour reading of this format can produce.
#[test]
fn the_three_reserved_indices_are_present_and_are_the_cards_8_6_names() {
    let Some(_) = apple_dir() else {
        assert!(skipped("apple II reserved indices"));
        return;
    };
    // (adventure, index) -> the count of each PALETTE colour on the canvas.
    let expected: [(u16, usize, [usize; 6]); 12] = [
        (1, 0, [51651, 440, 438, 2, 7, 1222]),
        (1, 98, [1668, 35, 101, 227, 572, 51157]),
        (1, 99, [3198, 0, 1011, 22211, 22004, 5336]),
        (2, 0, [51415, 8, 11, 260, 5, 2061]),
        (2, 98, [2423, 4, 5, 1169, 609, 49550]),
        (2, 99, [3198, 0, 1011, 22211, 22004, 5336]),
        (3, 0, [51415, 8, 11, 260, 5, 2061]),
        (3, 98, [932, 0, 0, 1089, 1521, 50218]),
        (3, 99, [3198, 0, 1011, 22211, 22004, 5336]),
        (6, 0, [51651, 440, 438, 2, 7, 1222]),
        (6, 98, [932, 0, 0, 1089, 1521, 50218]),
        (6, 99, [3198, 0, 1011, 22211, 22004, 5336]),
    ];
    for p in &PLAIN {
        let Some(pics) = pictures(p) else {
            assert!(skipped(p.title));
            return;
        };
        let rel = release(p.adventure);
        for (index, what) in [(0usize, "darkness"), (98, "inventory"), (99, "title")] {
            let name = room_picture_file_name(&rel, index).expect("names it");
            let file = pics
                .get(&name)
                .unwrap_or_else(|| panic!("{} has no {what} picture {name}", p.title));
            let pic = decode_family_d(file, SagaPlatform::AppleII).expect("decodes");
            let mut counts = [0usize; PALETTE.len()];
            for &v in &pic.pixels {
                counts[usize::from(v)] += 1;
            }
            let want = expected
                .iter()
                .find(|(a, i, _)| *a == p.adventure && *i == index)
                .expect("listed")
                .2;
            assert_eq!(counts, want, "{} {what} card {name}", p.title);
        }
        // …and enough room pictures to be a picture set rather than three
        // cards. Not "room 1 exists": *Mission Impossible* ships no `R0301`,
        // which is the release's own gap and not a decoding failure.
        let numbered = (1..=79)
            .filter(|&n| {
                room_picture_file_name(&rel, n).is_some_and(|name| pics.contains_key(&name))
            })
            .count();
        assert!(numbered >= 20, "{} has only {numbered} numbered room pictures", p.title);
    }
}

/// The darkness card is lettering: white ink spread right across a black
/// canvas, and none of it below the mixed-mode screen's 160 rows. The shape
/// guard that a wrong bit order or a wrong token length would fail while still
/// producing "a picture" — and, since SQ-1489, the guard that would fail an
/// inverted ground, because "IT'S TOO DARK!" drawn dark on white is exactly
/// what reading the page as starting blank produces.
#[test]
fn the_darkness_card_is_white_lettering_on_black() {
    let Some(_) = apple_dir() else {
        assert!(skipped("apple II darkness card"));
        return;
    };
    // *Adventureland* and *Strange Odyssey* ship one drawing of the words and
    // *Pirate Adventure* and *Mission Impossible* another, so four titles pin
    // two ink counts — which is a stronger statement than four loose bounds.
    let expected: [(u16, usize); 4] = [(1, 1222), (2, 2061), (3, 2061), (6, 1222)];
    for p in &PLAIN {
        let Some(pics) = pictures(p) else {
            assert!(skipped(p.title));
            return;
        };
        let name = room_picture_file_name(&release(p.adventure), 0).expect("names it");
        let pic = decode_family_d(&pics[&name], SagaPlatform::AppleII).expect("decodes");
        let white = |x: usize, y: usize| pic.pixels[y * CANVAS_WIDTH + x] == 5;
        let ink = pic.pixels.iter().filter(|&&v| v == 5).count();
        let black = pic.pixels.iter().filter(|&&v| v == 0).count();
        let columns = (0..CANVAS_WIDTH).filter(|&x| (0..CANVAS_HEIGHT).any(|y| white(x, y))).count();
        let want = expected.iter().find(|(a, _)| *a == p.adventure).expect("listed").1;
        assert_eq!(ink, want, "{} darkness card white ink", p.title);
        assert!(
            black > CANVAS_WIDTH * CANVAS_HEIGHT * 9 / 10,
            "{} darkness card is only {black} pixels of black — is the ground inverted?",
            p.title
        );
        assert!(columns > 140, "{} darkness card touches only {columns} columns", p.title);
        for y in 165..CANVAS_HEIGHT {
            assert!(
                !(0..CANVAS_WIDTH).any(|x| white(x, y)),
                "{} darkness card inks row {y}, below the mixed-mode screen",
                p.title
            );
        }
    }
}

/// The three scrambled releases (§7.4's string test, §10.6, SQ-1490): their
/// side A is not a DOS 3.3 disk, their boot side carries the `PAK.*` files
/// §8.4 describes and no room artwork — and the room artwork is on side A
/// after all, found by header rather than by catalogue.
#[test]
fn the_scrambled_releases_keep_their_room_artwork_on_a_side_with_no_filesystem() {
    let Some(dir) = apple_dir() else {
        assert!(skipped("apple II scrambled releases"));
        return;
    };
    for (title, stem, _, _, _) in SCRAMBLED {
        let boot = std::fs::read_dir(&dir)
            .expect("readable")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .find(|n| n.starts_with(stem) && n.contains("side B"));
        let Some(boot) = boot else {
            assert!(skipped(title));
            return;
        };
        let side_a = boot.replace("side B - boot", "side A").replace("side B (boot)", "side A");
        let raw_a = image(&side_a).unwrap_or_else(|| panic!("{title}: no {side_a}"));
        assert!(
            dos33_contents(&raw_a).is_empty(),
            "{title}: side A parses as a DOS 3.3 disk, so a catalogue walk would find the artwork"
        );

        let files = dos33_contents(&image(&boot).expect("boot side"));
        assert!(files.contains_key("M2"), "{title}: no M2 on the boot side");
        let m2 = &files["M2"];
        assert_eq!(
            m2.get(0x172C..0x172C + 31),
            Some(&b"COPYRIGHT 1983 NORMAN L. SAILER"[..]),
            "{title}: §7.4's string test does not fire"
        );
        assert!(files.contains_key("PAK.INVEN"), "{title}: no PAK.INVEN");
        assert!(
            !files.keys().any(|n| parse_apple_picture_file_name(n).is_some()),
            "{title}: the boot side names a picture file after all"
        );
        // §8.4's four-byte header really is there on a `PAK.*` file — offset
        // 0, offset 0, 40 byte columns, 160 rows — which is the half of that
        // section the specimens agree with, and the same header the records on
        // side A open with.
        assert_eq!(
            files["PAK.INVEN"].get(4..8),
            Some(&[0x00, 0x00, 0x28, 0xA0][..]),
            "{title}: PAK.INVEN does not open with §8.4's header"
        );

        // §8.4's per-release row table is the standard Apple II hi-res
        // interleave and nothing else — which is why this crate computes the
        // address instead of carrying three tables. Both halves are checked:
        // the 384 bytes agree with the arithmetic, row for row.
        let table = &m2[0x174B..0x174B + 0x182];
        for y in 0..0xC0usize {
            let addr = usize::from(table[y]) | (usize::from(table[0xC0 + y]) << 8);
            let interleave = 1024 * (y % 8) + 128 * ((y / 8) % 8) + 40 * (y / 64);
            assert_eq!(addr - 0x2000, interleave, "{title}: M2's row {y} address");
        }
    }
}

/// The record scan, per title: how many, where the first ones sit, and what
/// the cards at the ends of the numbering are.
///
/// **The load-bearing pin is the death card.** Each release's LAST room —
/// *Voodoo Castle*'s 25 "lot of TROUBLE!", *The Count*'s 22 "LOT OF
/// TROUBLE!", *Claymorgue Castle*'s 32 "real mess!" — is the picture that
/// record number carries, which is what says no spurious header anywhere
/// earlier has shifted the numbering. A count alone could not: a scan that
/// found one record too many and one too few would still count right.
#[test]
fn every_scrambled_room_picture_decodes_and_the_ordinal_is_the_picture_index() {
    let Some(dir) = apple_dir() else {
        assert!(skipped("apple II scrambled scan"));
        return;
    };
    for (title, stem, records, rooms, cards) in SCRAMBLED {
        let side_a = std::fs::read_dir(&dir)
            .expect("readable")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .find(|n| n.starts_with(stem) && n.contains("side A"));
        let Some(side_a) = side_a else {
            assert!(skipped(title));
            return;
        };
        let raw = image(&side_a).expect("side A");
        let ranges = scan_scrambled_pictures(&raw);
        assert_eq!(ranges.len(), records, "{title}: records on side A");
        assert_eq!(ranges[0].start, 0x1000, "{title}: the first record is track 1 sector 0");
        for (i, r) in ranges.iter().enumerate() {
            assert_eq!(r.start % 256, 0, "{title}: record {i} does not start on a sector");
            assert!(r.end > r.start, "{title}: record {i} is empty");
            if i + 1 < ranges.len() {
                assert!(r.end <= ranges[i + 1].start, "{title}: record {i} runs into the next");
            }
            // The run-length scheme cannot expand, so no record needs more
            // than one byte-pair token per output pair.
            assert!(
                r.end - r.start <= scott::apple_pictures::SCRAMBLED_MAX_RECORD,
                "{title}: record {i} is {} bytes",
                r.end - r.start
            );
        }
        assert!(rooms < records, "{title}: {rooms} rooms but only {records} records");

        let mut flat = 0usize;
        for (i, r) in ranges.iter().enumerate() {
            let pic = decode_family_d_scrambled(&raw[r.clone()], SagaPlatform::AppleII)
                .unwrap_or_else(|e| panic!("{title} record {i}: {e}"));
            // §8.4's nominal size, and NOT the plain sub-variant's 192-row
            // page: these records declare 40 byte columns by 160 rows.
            assert_eq!((pic.width, pic.height), (280, 160), "{title} record {i}");
            let mut seen = [false; PALETTE.len()];
            for &v in &pic.pixels {
                assert!(usize::from(v) < PALETTE.len(), "{title} record {i}: no such colour");
                seen[usize::from(v)] = true;
            }
            if seen.iter().filter(|&&s| s).count() == 1 {
                flat += 1;
            }
        }
        // Not one of the 97 records is a flat fill — and the closest thing to
        // one, *Claymorgue Castle*'s room 17 "I'm underwater in thick murky
        // fluid", is a field of blue in 256 bytes, the smallest record in the
        // corpus, that still carries the edge colours the artifact model gives
        // its border.
        assert_eq!(flat, 0, "{title}: {flat} records resolve to one flat colour");

        // …and the three cards, by their exact colour census.
        for (what, n, want) in cards {
            let pic = decode_family_d_scrambled(&raw[ranges[n].clone()], SagaPlatform::AppleII)
                .expect("decodes");
            let mut counts = [0usize; PALETTE.len()];
            for &v in &pic.pixels {
                counts[usize::from(v)] += 1;
            }
            assert_eq!(counts, want, "{title} {what} (record {n})");
        }
    }
}

/// The dispatcher (SQ-1490): one entry point, two sub-variants, told apart by
/// the record's own first bytes — a plain record's `$7000` load address
/// against a scrambled record's §8.4 header. Pinned on one real record of each
/// kind, because the whole point is that the caller does not have to know.
#[test]
fn one_entry_point_reads_both_sub_variants() {
    let Some(dir) = apple_dir() else {
        assert!(skipped("apple II dispatcher"));
        return;
    };
    // A plain record: *Adventureland*'s darkness card, 280x192 line art.
    let Some(pics) = pictures(&PLAIN[0]) else {
        assert!(skipped("Adventureland"));
        return;
    };
    let plain = decode_family_d(&pics["R0100"], SagaPlatform::AppleII).expect("decodes");
    assert_eq!((plain.width, plain.height), (CANVAS_WIDTH, CANVAS_HEIGHT), "the plain page");

    // A scrambled record: *The Count*'s, 280x160.
    let side_a = std::fs::read_dir(&dir)
        .expect("readable")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.starts_with(SCRAMBLED[1].1) && n.contains("side A"));
    let Some(side_a) = side_a else {
        assert!(skipped("The Count"));
        return;
    };
    let raw = image(&side_a).expect("side A");
    let ranges = scan_scrambled_pictures(&raw);
    let scrambled = decode_family_d(&raw[ranges[0].clone()], SagaPlatform::AppleII).expect("decodes");
    assert_eq!((scrambled.width, scrambled.height), (280, 160), "the scrambled box");
}
