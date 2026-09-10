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
//! - **the reserved indices** — §8.6 says 0 is the darkness picture, 98 the
//!   inventory backdrop and 99 the title picture, and all four titles carry
//!   all three under the naming rule this crate implements;
//! - **the three scrambled releases** — their side A is not a DOS 3.3 disk at
//!   all, so their room artwork is unreachable and this suite pins the refusal
//!   rather than pretending otherwise.
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

use scott::apple_pictures::{decode_family_d, CANVAS_HEIGHT, CANVAS_WIDTH};
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

/// The three §10.6 releases whose `M2` carries §7.4's string, whose side A is
/// not a DOS 3.3 disk, and whose room artwork is therefore unreachable.
const SCRAMBLED: [(&str, &str); 3] = [
    (
        "Voodoo Castle",
        "Scott Adams Graphic Adventure 4 - Voodoo Castle v2.1-119 (4am crack) side ",
    ),
    ("The Count", "Scott Adams Graphic Adventure 5 - The Count v2.1-115 (4am crack) side "),
    (
        "Claymorgue Castle",
        "Scott Adams Graphic Adventure 13 - The Sorcerer of Claymorgue Castle v2.2-122 (4am crack) side ",
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
fn census(file: &[u8]) -> (usize, usize, usize, usize) {
    let declared = usize::from(u16::from_le_bytes([file[2], file[3]]));
    let data = &file[4..(4 + declared).min(file.len())];
    let (mut tokens, mut shorts, mut off, mut unknown) = (0, 0, 0, 0);
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        if b & 0x80 == 0 {
            shorts += 1;
            i += 1;
            continue;
        }
        if i + 3 > data.len() {
            break;
        }
        let x = usize::from(data[i + 1]) | (usize::from(b & 1) << 8);
        let y = usize::from(data[i + 2]);
        i += 3;
        tokens += 1;
        if !matches!(b & 0xE0, 0x80 | 0xA0 | 0xC0 | 0xE0) {
            unknown += 1;
        }
        if x >= CANVAS_WIDTH || y >= CANVAS_HEIGHT {
            off += 1;
        }
    }
    (tokens, shorts, off, unknown)
}

#[test]
fn every_command_byte_is_one_of_the_four_and_almost_every_point_is_on_the_canvas() {
    let Some(_) = apple_dir() else {
        assert!(skipped("apple II token census"));
        return;
    };
    let (mut tokens, mut shorts, mut off, mut unknown, mut files) = (0, 0, 0, 0, 0);
    for p in &PLAIN {
        let Some(pics) = pictures(p) else {
            assert!(skipped(p.title));
            return;
        };
        for file in pics.values() {
            let (t, s, o, u) = census(file);
            tokens += t;
            shorts += s;
            off += o;
            unknown += u;
            files += 1;
        }
    }
    eprintln!("family D census: {files} files, {tokens} tokens, {shorts} one-byte, {off} off-canvas");
    assert!(files >= 300, "only {files} picture files across the four plain releases");
    assert_eq!(unknown, 0, "a command byte outside the four, in {tokens} tokens");
    // The measurement the format's reading rests on. Read with an eight-bit
    // x instead of the ninth bit this decoder takes from the command byte,
    // thousands of these land off the canvas.
    assert_eq!(off, 0, "of {tokens} coordinates, {off} are off the canvas");
    assert_eq!((tokens, shorts), (71_899, 6_272), "the census these disks give");
}

#[test]
fn every_picture_on_every_plain_release_decodes() {
    let Some(_) = apple_dir() else {
        assert!(skipped("apple II picture decode"));
        return;
    };
    for p in &PLAIN {
        let Some(pics) = pictures(p) else {
            assert!(skipped(p.title));
            return;
        };
        let (mut rooms, mut objects, mut inked) = (0, 0, 0);
        for (name, file) in &pics {
            let pic = decode_family_d(file, SagaPlatform::AppleII)
                .unwrap_or_else(|e| panic!("{} {name}: {e}", p.title));
            assert_eq!((pic.width, pic.height), (CANVAS_WIDTH, CANVAS_HEIGHT));
            if pic.pixels.contains(&1) {
                inked += 1;
            }
            match parse_apple_picture_file_name(name).expect("named").1.usage {
                PictureUsage::Room => rooms += 1,
                _ => objects += 1,
            }
        }
        assert_eq!((rooms, objects), (p.rooms, p.objects), "{} picture counts", p.title);
        // A handful of records are stubs — *Strange Odyssey* ships three room
        // files with no tokens in them at all, and two object files draw
        // nothing but an area — so this is "nearly all", not "all".
        assert!(
            inked + 6 >= rooms + objects,
            "{}: only {inked} of {} pictures ink anything",
            p.title,
            rooms + objects
        );
    }
}

#[test]
fn the_three_reserved_indices_are_present_and_are_the_cards_8_6_names() {
    let Some(_) = apple_dir() else {
        assert!(skipped("apple II reserved indices"));
        return;
    };
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
            let ink = pic.pixels.iter().filter(|&&v| v == 1).count();
            assert!(ink > 200, "{} {what} card {name} has only {ink} inked pixels", p.title);
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

/// The darkness card is lettering: ink spread right across the canvas and
/// none of it below the mixed-mode screen's 160 rows. The shape guard that a
/// wrong bit order or a wrong token length would fail while still producing
/// "a picture" — and the pin that says the two cards in this corpus are the
/// two drawings they are.
#[test]
fn the_darkness_card_is_lettering_across_the_canvas() {
    let Some(_) = apple_dir() else {
        assert!(skipped("apple II darkness card"));
        return;
    };
    // *Adventureland* and *Strange Odyssey* ship one drawing of the words and
    // *Pirate Adventure* and *Mission Impossible* another, so four titles pin
    // two ink counts — which is a stronger statement than four loose bounds.
    let expected: [(u16, usize); 4] = [(1, 6163), (2, 2339), (3, 2339), (6, 6163)];
    for p in &PLAIN {
        let Some(pics) = pictures(p) else {
            assert!(skipped(p.title));
            return;
        };
        let name = room_picture_file_name(&release(p.adventure), 0).expect("names it");
        let pic = decode_family_d(&pics[&name], SagaPlatform::AppleII).expect("decodes");
        let lit = |x: usize, y: usize| pic.pixels[y * CANVAS_WIDTH + x] == 1;
        let ink = pic.pixels.iter().filter(|&&v| v == 1).count();
        let columns = (0..CANVAS_WIDTH).filter(|&x| (0..CANVAS_HEIGHT).any(|y| lit(x, y))).count();
        let want = expected.iter().find(|(a, _)| *a == p.adventure).expect("listed").1;
        assert_eq!(ink, want, "{} darkness card ink", p.title);
        assert!(columns > 150, "{} darkness card touches only {columns} columns", p.title);
        for y in 165..CANVAS_HEIGHT {
            assert!(
                !(0..CANVAS_WIDTH).any(|x| lit(x, y)),
                "{} darkness card inks row {y}, below the mixed-mode screen",
                p.title
            );
        }
    }
}

/// The three scrambled releases: §7.4's string test fires on their `M2`, their
/// side A is not a DOS 3.3 disk at all, and their boot side carries the
/// `PAK.*` files §8.4 describes and no room artwork. Pinned so the refusal
/// this crate reports stays honest.
#[test]
fn the_scrambled_releases_keep_their_room_artwork_out_of_reach() {
    let Some(dir) = apple_dir() else {
        assert!(skipped("apple II scrambled releases"));
        return;
    };
    for (title, stem) in SCRAMBLED {
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
            "{title}: side A parses as a DOS 3.3 disk, so the pictures may be reachable after all"
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
        // section the specimens agree with.
        assert_eq!(
            files["PAK.INVEN"].get(4..8),
            Some(&[0x00, 0x00, 0x28, 0xA0][..]),
            "{title}: PAK.INVEN does not open with §8.4's header"
        );
    }
}
