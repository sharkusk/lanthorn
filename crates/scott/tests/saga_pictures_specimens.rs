//! Picture family C (spec §8.3) against the *Hulk*'s own Commodore 64 release
//! disk, and against the **MS-DOS twin of the same artwork** as an independent
//! oracle (SQ-1475).
//!
//! # The oracle, and what it settled
//!
//! §10.1's rule — decode the native file and compare it against a second
//! encoding of the same work — has an unusually good instance here. *The Hulk*
//! shipped on `QUESTPR1.D64` as family C and in MS-DOS `.PAK` files as family
//! E (§8.5), one picture per room in both, same artist, same 280-pixel canvas,
//! and family E's storage order is row-major with a two-bank interleave —
//! nothing at all like family C's column strips. So the two decoders share no
//! arithmetic, and a picture that comes out of both the same way is a picture
//! neither got wrong.
//!
//! It settled the one place §8.3's wording is ambiguous. "When it passes the
//! height" reads naturally as an exclusive limit, and an exclusive limit
//! leaves every column one byte pair short of the data, so each column starts
//! two rows lower than the last: the *title screen* still decodes to a
//! coherent picture of the Hulk with a legible `QUESTPROBE` wordmark, sheared
//! by two rows per eight pixels, with `HULK` sliced in half by the wrap. Only
//! the MS-DOS twin says which of the two is right — and the byte arithmetic
//! then agrees exactly, so [`scott::saga_pictures::CANVAS_HEIGHT`] is 160 and
//! not §8.3's stated 158. See that constant and [`decode_family_c`].
//!
//! # Getting the corpus
//!
//! Commercial game files, not redistributable, not committed. Everything here
//! **skips vacuously with an explanation** when the disk is absent — a silent
//! skip reads exactly like a pass, so the skip says why.
//!
//! ```text
//! <fixtures>/c64/QUESTPR1.D64
//!     The Hulk (Commodore 64), sha256 5035c0ae93eb… (§10.7)
//! ```
//!
//! where `<fixtures>` is `$SCOTT_DIALECT_FIXTURES` or `stories/scott-dialects`.
//!
//! # The D64 walk in this file
//!
//! `scott` takes no dependencies and reads no disk images — the host does that
//! ([`blorb::medium`] in lanthorn). The forty lines of block-chain walk below
//! are **test scaffolding**, not a second implementation: they exist so this
//! suite can reach a real specimen, and the D64 format they read (35 tracks,
//! a directory chain from track 18 sector 1, 32-byte entries, two link bytes
//! per 256-byte block) is publicly documented and independent of anything the
//! specification says about pictures.

use std::collections::BTreeMap;
use std::path::PathBuf;

use scott::c64_palette::PEPTO_PALETTE;
use scott::saga_pictures::{decode_family_c, CANVAS_HEIGHT, CANVAS_WIDTH};
use scott::{parse_picture_file_name, PictureUsage, SagaPlatform};

/// The same three candidates `c64_specimens` tries, in the same order: the
/// environment override, the workspace-root path, and the path relative to
/// this crate's own directory — because cargo's working directory for an
/// integration test is the package root, not the workspace root.
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
        "SKIP: {what} — needs QUESTPR1.D64 under stories/scott-dialects/c64/ \
         (see this file's header for provenance)"
    );
    true
}

// ── D64 test scaffolding (see the module header) ─────────────────────────────

/// Sectors per track, 1-based track numbers, for a 35-track image.
fn sectors_per_track(track: usize) -> usize {
    match track {
        1..=17 => 21,
        18..=24 => 19,
        25..=30 => 18,
        _ => 17,
    }
}

fn block_offset(track: usize, sector: usize) -> usize {
    let before: usize = (1..track).map(sectors_per_track).sum();
    (before + sector) * 256
}

/// Follow a file's block chain. The first two bytes of each 256-byte block are
/// the next (track, sector); a zero track ends the chain and the second byte is
/// then the count of bytes used plus one.
fn read_chain(raw: &[u8], mut track: usize, mut sector: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    while track != 0 && seen.insert((track, sector)) {
        let at = block_offset(track, sector);
        let Some(block) = raw.get(at..at + 256) else { break };
        let (next_track, next_sector) = (usize::from(block[0]), usize::from(block[1]));
        if next_track == 0 {
            let used = usize::from(block[1]).saturating_sub(1).min(254);
            out.extend_from_slice(&block[2..2 + used]);
            break;
        }
        out.extend_from_slice(&block[2..]);
        (track, sector) = (next_track, next_sector);
    }
    out
}

/// Every named file on the image, in directory order.
fn d64_contents(raw: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let (mut track, mut sector) = (18usize, 1usize);
    let mut seen = std::collections::HashSet::new();
    while track != 0 && seen.insert((track, sector)) {
        let at = block_offset(track, sector);
        let Some(block) = raw.get(at..at + 256) else { break };
        // Eight 32-byte entries per directory sector, at offsets 0, 32, … 224.
        // Within an entry: +2 the file type (0 = an unused slot), +3/+4 the
        // first (track, sector) of the file's block chain, +5..21 the name,
        // padded with 0xA0.
        for slot in 0..8 {
            let e = &block[slot * 32..slot * 32 + 32];
            if e[2] == 0 {
                continue;
            }
            let name: String =
                e[5..21].iter().copied().filter(|&c| c != 0xA0).map(char::from).collect();
            out.push((name, read_chain(raw, usize::from(e[3]), usize::from(e[4]))));
        }
        (track, sector) = (usize::from(block[0]), usize::from(block[1]));
    }
    out
}

/// The *Hulk*'s picture files off `QUESTPR1.D64`, keyed by name in directory
/// order, or `None` when the disk is absent.
fn hulk_pictures() -> Option<Vec<(String, Vec<u8>)>> {
    let raw = std::fs::read(fixtures()?.join("c64/QUESTPR1.D64")).ok()?;
    if raw.len() != 174_848 {
        return None;
    }
    let pics: Vec<(String, Vec<u8>)> = d64_contents(&raw)
        .into_iter()
        .filter(|(name, _)| parse_picture_file_name(name).is_some())
        .collect();
    (!pics.is_empty()).then_some(pics)
}

// ── The specimens ────────────────────────────────────────────────────────────

/// Every picture file on the *Hulk*'s disk decodes to the full canvas, and the
/// set is the one §10.7 describes: `R01nnn` rooms, `B01nnnR` objects drawn in a
/// room and `B01nnnI` objects drawn in the inventory.
///
/// The counts are **pinned, not floored** (the `ti994a_specimens` rule): 26
/// room pictures, 16 room-object overlays and 19 inventory-object overlays,
/// 61 in all, is what this release carries, and a loader that started reading
/// the directory differently would say something else.
#[test]
fn every_picture_on_the_hulks_disk_decodes_to_the_full_canvas() {
    let Some(pics) = hulk_pictures() else {
        assert!(skipped("the Hulk family-C picture walk"));
        return;
    };
    let mut by_usage: BTreeMap<&str, Vec<u16>> = BTreeMap::new();
    for (name, bytes) in &pics {
        let parsed = parse_picture_file_name(name).expect("filtered on this");
        let pic = decode_family_c(bytes, SagaPlatform::Commodore64)
            .unwrap_or_else(|e| panic!("{name} ({} bytes): {e}", bytes.len()));
        assert_eq!(
            (pic.width(), pic.height()),
            (CANVAS_WIDTH, CANVAS_HEIGHT),
            "{name} decodes to the family-C canvas"
        );
        assert_eq!(pic.pixels().len(), CANVAS_WIDTH * CANVAS_HEIGHT, "{name} pixel count");
        assert!(pic.pixels().iter().all(|&v| v < 4), "{name} stores only two-bit values");
        let usage = match parsed.usage() {
            PictureUsage::Room => "room",
            PictureUsage::ObjectInRoom => "object-in-room",
            PictureUsage::ObjectInInventory => "object-in-inventory",
            // `PictureUsage` is `#[non_exhaustive]`: every usage this release
            // carries is named above, so a new one is a real find worth
            // seeing rather than silently bucketing away.
            other => panic!("unhandled PictureUsage variant: {other:?}"),
        };
        by_usage.entry(usage).or_default().push(parsed.index());
    }
    assert_eq!(pics.len(), 70, "the whole picture set");
    assert_eq!(by_usage["room"].len(), 30, "R01nnn room pictures");
    assert_eq!(by_usage["object-in-room"].len(), 18, "B01nnnR overlays");
    assert_eq!(by_usage["object-in-inventory"].len(), 22, "B01nnnI overlays");

    // §8.6's three reserved indices are all present as room pictures: 0 the
    // darkness image, 98 the inventory backdrop, 99 the title screen.
    let mut rooms = by_usage["room"].clone();
    rooms.sort_unstable();
    for reserved in [0u16, 98, 99] {
        assert!(rooms.contains(&reserved), "reserved room picture {reserved}");
    }
    // And the rooms that have one — which, with §12.11's remap, is every room
    // this game has. Rooms 5-8, 10, 11, 13, 14, 17 and 18 have no `R01nnn` of
    // their own and are exactly the ten §12.11 remaps onto 3, 4, 9, 2 and 16;
    // every one of those five targets is here. That is the remap checked
    // against the disk rather than taken on trust — an eleventh room without a
    // picture, or a remap target without a file, would show up here.
    assert_eq!(
        rooms,
        vec![
            0, 1, 2, 3, 4, 9, 12, 15, 16, 19, 20, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92,
            93, 94, 95, 96, 97, 98, 99
        ],
        "the room-picture indices this release ships"
    );
    let hulk = scott::SagaUs { version: 127, adventure: 1, platform: SagaPlatform::Commodore64 };
    for room in 1..=20usize {
        let picture = hulk.room_picture(room);
        assert!(
            rooms.contains(&(picture as u16)),
            "room {room} resolves to picture {picture}, which this disk carries"
        );
    }
    // The same set the MS-DOS release ships as `.PAK` files (§10.7): rooms
    // 0-4, 9, 12, 15, 16, 19, 20 and the reserved 81-99. Two encodings of one
    // picture set, which is §10.1's oracle.
    assert_eq!(by_usage["room"].len(), 30);
}

/// A room picture is really a picture: four colours actually used, and the
/// canvas neither empty nor a flat fill.
///
/// The non-flat guard is the one that catches a decoder writing nothing (an
/// exclusive width limit, a mis-read data offset) — every count in a
/// self-consistent geometry can look right over a blank canvas.
#[test]
fn room_one_is_a_four_colour_drawing_and_not_a_flat_fill() {
    let Some(pics) = hulk_pictures() else {
        assert!(skipped("the Hulk room-1 picture"));
        return;
    };
    let (_, bytes) = pics
        .iter()
        .find(|(name, _)| {
            parse_picture_file_name(name)
                .is_some_and(|pf| pf.usage() == PictureUsage::Room && pf.index() == 1)
        })
        .expect("R01001 is on the disk");
    let pic = decode_family_c(bytes, SagaPlatform::Commodore64).expect("decodes");
    let mut used = [0usize; 4];
    for &v in pic.pixels() {
        used[usize::from(v)] += 1;
    }
    for (value, count) in used.iter().enumerate() {
        assert!(*count > 0, "pixel value {value} is used somewhere");
    }
    // Room 1 is Bruce Banner tied to a chair on a dithered ground: no value
    // covers more than four fifths of the canvas, and every value covers at
    // least a hundredth of it.
    let total = pic.pixels().len();
    for (value, count) in used.iter().enumerate() {
        assert!(*count * 5 < total * 4, "value {value} covers {count}/{total} — a flat fill?");
        assert!(*count * 100 > total, "value {value} covers only {count}/{total}");
    }
    assert!(pic.unrecognised_colours().is_empty(), "room 1's colours all resolve");
    // Orange skin, purple cloth, white highlight — §8.3's Commodore 64 table.
    assert_eq!(pic.colour_bytes(), [56, 103, 14, 16]);
    assert_eq!(
        pic.palette(),
        [PEPTO_PALETTE[0], PEPTO_PALETTE[8], PEPTO_PALETTE[4], PEPTO_PALETTE[1]],
        "black, orange, purple, white"
    );
}

/// One of this release's own colour bytes is outside §8.3's table, and the
/// decoder surfaces it rather than inventing a colour (§8.3's own
/// instruction). Everything else resolves.
///
/// It was two until SQ-1491: `B01250R`'s 232 is yellow on a real machine
/// (`machine-screenshots/c64-hulk-colorbars.png`), so only `R01012`'s 153 is
/// left. Pinned as the exact set, so a later table correction shows up here as
/// a failure rather than as silence.
#[test]
fn the_only_unrecognised_colour_byte_is_153() {
    let Some(pics) = hulk_pictures() else {
        assert!(skipped("the Hulk colour-byte sweep"));
        return;
    };
    let mut unresolved: BTreeMap<u8, Vec<String>> = BTreeMap::new();
    for (name, bytes) in &pics {
        let pic = decode_family_c(bytes, SagaPlatform::Commodore64).expect("decodes");
        for byte in pic.unrecognised_colours() {
            unresolved.entry(*byte).or_default().push(name.clone());
        }
    }
    let bytes: Vec<u8> = unresolved.keys().copied().collect();
    assert_eq!(bytes, vec![153], "the table is missing exactly this one");
    assert_eq!(unresolved[&153], vec!["R01012"], "153 is room 12's pixel value 2");
}

/// The placement fields put each picture where §8.3 says, and nothing a record
/// draws falls off the canvas.
///
/// Two of the *Hulk*'s room pictures declare a left edge of column **-1**
/// (`header[4]` = 2, and the field is the column plus three), which is off the
/// canvas — and every pixel in that column is value 0, which is why a
/// clipping decoder loses nothing. That is the check that a negative left edge
/// is the right reading of the "+3" rule and not an off-by-one in it.
#[test]
fn the_full_canvas_pictures_declare_the_placements_8_3_predicts() {
    let Some(pics) = hulk_pictures() else {
        assert!(skipped("the Hulk placement sweep"));
        return;
    };
    let mut full = 0usize;
    let mut negative_left = Vec::new();
    for (name, bytes) in &pics {
        let (left_field, top, right_field, bottom) = (bytes[4], bytes[5], bytes[6], bytes[7]);
        let left = (i32::from(left_field) - 3) * 8;
        let right = (i32::from(right_field) - 3) * 8;
        assert!(right >= left, "{name}: right {right} is not left of left {left}");
        assert!(bottom >= top, "{name}: bottom {bottom} above top {top}");
        // The bottom row is inclusive and each pair covers two rows, so the
        // last row a record touches is `bottom + 1` — which is exactly
        // CANVAS_HEIGHT - 1 for a full-canvas picture.
        assert!(
            i32::from(bottom) + 1 < CANVAS_HEIGHT as i32,
            "{name}: bottom {bottom} would draw past the canvas"
        );
        if right - left >= CANVAS_WIDTH as i32 {
            full += 1;
        }
        if left < 0 {
            negative_left.push(name.clone());
        }
    }
    assert!(full >= 20, "most of the set is full-canvas artwork, found {full}");
    assert_eq!(
        negative_left,
        vec!["R01000", "R01020"],
        "the records whose left edge is column -1"
    );
}
