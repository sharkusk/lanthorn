//! Picture family C on the **Atari 8-bit** companion picture sides (§8.3,
//! §7.3, §12.10 — SQ-1483, SQ-1484).
//!
//! # What these disks are
//!
//! Seven two-sided US S.A.G.A. releases, catalogued in §10.5 with the sha256
//! of every file. Side A is the database side (§12.12 tabulates what each one
//! decodes to) and side B is nothing but artwork: **no Atari DOS 2 directory**
//! — the eight sectors where one would be hold picture data like every other
//! sector — so §8.3's "on the Atari there is no filesystem walk" is right, and
//! a reader has only the bytes.
//!
//! ```text
//! <fixtures>/atari/SAGA #4 - Voodoo Castle [side B].atr        sha256 2a417fb62f14…
//! <fixtures>/atari/SAGA #5 - The Count [side B].atr            sha256 37fd7e4fd1cf…
//! <fixtures>/atari/SAGA No. 13 - The Sorcerer of Claymorgue Castle _ side B.atr
//!                                                              sha256 9542de4bb9d8…
//! ```
//!
//! where `<fixtures>` is `$SCOTT_DIALECT_FIXTURES` or `stories/scott-dialects`.
//! Commercial game files, not redistributable, not committed — every case here
//! **skips vacuously with an explanation** when a disk is absent, because a
//! silent skip reads exactly like a pass.
//!
//! # Only three of the seven are family C
//!
//! The other four — *Adventureland*, *Pirate Adventure*, *Mission Impossible*
//! and *Strange Odyssey* — keep a line-drawing token stream on side B instead,
//! the format Appendix A item 26 measured on the four **plain** Apple II
//! releases. [`no_line_art_side_is_mistaken_for_a_bitmap_side`] is the case
//! that says so, and it is as much a part of this suite as the three that
//! decode: a scanner that found "records" on those four would be finding noise.
//!
//! # What settles the record shape
//!
//! No oracle, unlike [`saga_pictures_specimens`](../saga_pictures_specimens),
//! whose MS-DOS twin decided §8.3's inclusive limit. What settles it here is
//! that a family-C record is **self-proving**: its header says how many byte
//! pairs it holds, and a wrong reading of the header — a twelve-byte one, the
//! other compression scheme, the other edge convention — cannot produce
//! exactly that many. See `scott::saga_atari` for the shape and for how far
//! it departs from §8.3's prose.

use std::path::PathBuf;

use scott::saga_atari::{decode_record, scan_picture_side, splice_vtoc, AtariRecord, SIDE_LEN};
use scott::saga_pictures::{FamilyCScheme, CANVAS_HEIGHT, CANVAS_WIDTH};
use scott::{SagaPlatform, SagaUs};

/// The same three candidates the sibling suites try, in the same order.
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

/// One release's side B, or `None` with a reason on stderr.
fn side_b(file: &str) -> Option<Vec<u8>> {
    let Some(dir) = fixtures() else {
        eprintln!("SKIP: no stories/scott-dialects — see this file's header");
        return None;
    };
    let path = dir.join("atari").join(file);
    let Ok(raw) = std::fs::read(&path) else {
        eprintln!("SKIP: {} is absent — see this file's header", path.display());
        return None;
    };
    // §7.3's identification, so a truncated or re-mastered image cannot read
    // as a pass.
    assert_eq!(raw.len(), SIDE_LEN, "{file} is not a 720-sector single-density image");
    assert_eq!(&raw[..6], &[0x96, 0x02, 0x80, 0x16, 0x80, 0x00], "{file} lacks §7.3's header");
    Some(raw)
}

/// The three family-C titles, with the release identity §12.12 gives each, the
/// record count measured on its side, and how many records the side holds that
/// the consistency check **cannot** read (SQ-1483).
///
/// The unreadable ones are two records on *The Count*'s side and no others.
/// Both declare far more data than their region can hold — the one at file
/// offset `0x7CBA` declares 4,742 bytes against a 2,765-pair region whose
/// run-length units come to 6,322 pairs — so neither satisfies the check, and
/// this suite names them rather than loosening the check until they pass.
///
/// `(file, release, scheme, records, unreadable)`.
const BITMAP_TITLES: [(&str, SagaUs, FamilyCScheme, usize, usize); 3] = [
    (
        "SAGA #4 - Voodoo Castle [side B].atr",
        SagaUs { version: 119, adventure: 4, platform: SagaPlatform::Atari8Bit },
        FamilyCScheme::NoLiteral,
        79,
        0,
    ),
    (
        "SAGA #5 - The Count [side B].atr",
        SagaUs { version: 115, adventure: 5, platform: SagaPlatform::Atari8Bit },
        FamilyCScheme::NoLiteral,
        72,
        2,
    ),
    (
        "SAGA No. 13 - The Sorcerer of Claymorgue Castle _ side B.atr",
        SagaUs { version: 125, adventure: 13, platform: SagaPlatform::Atari8Bit },
        FamilyCScheme::Standard,
        90,
        0,
    ),
];

/// The four titles whose side B is a line-drawing token stream, not family C.
const LINE_ART_TITLES: [&str; 4] = [
    "SAGA #1 - Adventureland [side B].atr",
    "SAGA #2 - Pirate Adventure [side B].atr",
    "SAGA #3 - Mission Impossible [side B].atr",
    "SAGA #6 - Strange Odyssey [side B].atr",
];

/// Per title, how many records the scan finds — the number a re-master or a
/// change to the consistency check would move.
#[test]
fn each_bitmap_side_holds_the_measured_number_of_records() {
    for (file, release, scheme, want, _) in BITMAP_TITLES {
        let Some(raw) = side_b(file) else { continue };
        assert_eq!(release.picture_scheme(), scheme, "{file}: §8.3 names the variant's titles");
        assert_eq!(
            release.atari_picture_format(),
            Some(scott::AtariPictureFormat::FamilyCBitmap),
            "{file}: and this one is a bitmap side",
        );
        let found = scan_picture_side(&raw, scheme);
        assert_eq!(found.len(), want, "{file}: record count");
    }
}

/// Every record decodes, and each one's declared size is its decoded length or
/// one more — never anything else.
///
/// That last is the measurement that refutes §8.3's twelve-byte header for
/// this platform. Read with two more bytes of header the data is two pairs
/// short of the region on every record, and not one of these would decode.
#[test]
fn every_record_decodes_and_its_size_matches_what_it_took() {
    for (file, release, _, _, _) in BITMAP_TITLES {
        let Some(raw) = side_b(file) else { continue };
        let scheme = release.picture_scheme();
        let spliced = splice_vtoc(&raw);
        let found = scan_picture_side(&raw, scheme);
        let mut exact = 0usize;
        for r in &found {
            let slack = r.size() - r.decoded_len();
            assert!(slack <= 1, "{file}: record at 0x{:05X} has {slack} spare bytes", r.file_offset());
            if slack == 0 {
                exact += 1;
            }
            let pic = decode_record(&spliced, r, scheme).expect("a located record decodes");
            assert_eq!((pic.width(), pic.height()), (CANVAS_WIDTH, CANVAS_HEIGHT));
            assert!(
                pic.unrecognised_colours().is_empty(),
                "{file}: every Atari colour byte has a colour, but 0x{:05X} left {:?}",
                r.file_offset(),
                pic.unrecognised_colours(),
            );
            assert_eq!(pic.palette()[0], (0, 0, 0), "§8.3: entry 0 is black whatever the record says");
        }
        assert!(exact > found.len() / 2, "{file}: most records declare exactly what they took");
    }
}

/// Records lie end to end from the head of the side, with nought to six bytes
/// of filler between them — and the only breaks in that run are the two
/// unreadable records [`BITMAP_TITLES`] names.
///
/// The filler is the reason a reader cannot address a picture by adding sizes
/// from the first, and — with §12.10's "not recoverable from the database" —
/// the reason the per-title offset lists §8.3 asks for have to be measured.
///
/// This case is also what would catch the scan quietly losing records: a
/// tightened check, or a re-mastered disk, shows up as a gap of thousands of
/// bytes where a picture used to be.
#[test]
fn records_lie_end_to_end_with_at_most_six_bytes_of_filler() {
    /// Bigger than any filler run and smaller than any record: a gap past this
    /// is a picture that was not read, not slack between two that were.
    const FILLER: usize = 8;
    for (file, release, _, _, unreadable) in BITMAP_TITLES {
        let Some(raw) = side_b(file) else { continue };
        let found = scan_picture_side(&raw, release.picture_scheme());
        assert!(found.len() > 40, "{file}: sanity, the scan found something");
        assert!(
            found[0].file_offset() < 0x300,
            "{file}: the first record is right behind the shared boot loader, at 0x{:05X}",
            found[0].file_offset(),
        );
        let mut breaks = Vec::new();
        for pair in found.windows(2) {
            let gap = pair[1].offset() - (pair[0].offset() + pair[0].size());
            if gap > FILLER {
                breaks.push((pair[0].file_offset(), gap));
            }
        }
        assert_eq!(
            breaks.len(),
            unreadable,
            "{file}: expected {unreadable} unreadable records, found breaks after {breaks:02X?}",
        );
    }
}

/// **The falsification for SQ-1484.** *The Count* and *Voodoo Castle* read
/// with the standard scheme yield almost nothing, and *Claymorgue Castle* read
/// with the variant likewise.
///
/// This is the case that would have caught getting §8.3's variant wrong, and
/// it is stronger than any hand-built record can be: a whole 92 KB side offers
/// 92,000 offsets to be wrong at, and the measured separation is not close.
/// Right scheme against wrong, per title: *Voodoo Castle* 79 against 2, *The
/// Count* 72 against 0, *Claymorgue Castle* 90 against 6.
#[test]
fn the_wrong_scheme_finds_almost_no_records_on_a_real_side() {
    /// Above every wrong-scheme count measured (6) and far below every right
    /// one (72).
    const NOISE: usize = 10;
    for (file, release, _, want, _) in BITMAP_TITLES {
        let Some(raw) = side_b(file) else { continue };
        let other = match release.picture_scheme() {
            FamilyCScheme::Standard => FamilyCScheme::NoLiteral,
            FamilyCScheme::NoLiteral => FamilyCScheme::Standard,
            // `FamilyCScheme` is `#[non_exhaustive]`: only two schemes exist
            // today, and a third would need this test's own "the other one"
            // rule revisited rather than guessed at.
            _ => unreachable!("only two family-C schemes exist"),
        };
        let wrong = scan_picture_side(&raw, other);
        assert!(
            wrong.len() < NOISE && want > NOISE * 4,
            "{file}: the wrong scheme found {} records against {want} right ones",
            wrong.len(),
        );
    }
}

/// A line-art side is not mistaken for a bitmap side under either scheme.
///
/// The four titles here are §8.3's blind spot: it says the Atari releases are
/// family C and four of the seven are not. A scan that found records on these
/// would be reading noise, and the whole method would be worthless.
///
/// Measured, three of the four yield **nothing at all** under either scheme
/// and *Strange Odyssey* yields five accidental hits under the standard one —
/// against 72 to 90 on a side that really is family C.
#[test]
fn no_line_art_side_is_mistaken_for_a_bitmap_side() {
    const NOISE: usize = 10;
    for file in LINE_ART_TITLES {
        let Some(raw) = side_b(file) else { continue };
        for scheme in [FamilyCScheme::Standard, FamilyCScheme::NoLiteral] {
            let found = scan_picture_side(&raw, scheme);
            assert!(
                found.len() < NOISE,
                "{file} is a line-drawing side, but {scheme:?} found {} records",
                found.len(),
            );
        }
    }
    // And the crate says so by release identity rather than by sniffing.
    for (adventure, version) in [(1u16, 416u16), (2, 408), (3, 306), (6, 119)] {
        let r = SagaUs { version, adventure, platform: SagaPlatform::Atari8Bit };
        assert_eq!(r.atari_picture_format(), Some(scott::AtariPictureFormat::LineArt));
    }
    for (adventure, version) in [(4u16, 119u16), (5, 115), (13, 125)] {
        let r = SagaUs { version, adventure, platform: SagaPlatform::Atari8Bit };
        assert_eq!(r.atari_picture_format(), Some(scott::AtariPictureFormat::FamilyCBitmap));
    }
}

/// One picture pinned by geometry and by pixels, per title.
///
/// **Named by what it depicts**, because that is the only thing that says the
/// decode is right rather than merely self-consistent — a sheared or noisy
/// picture is as internally consistent as a good one. Each was read off a
/// render of the record at the offset below.
///
/// `(file, scheme, file offset, what it shows, cols, pairs, the four colour bytes)`
type Pin = (&'static str, FamilyCScheme, usize, &'static str, i32, i32, [u8; 4]);

const PINNED: [Pin; 3] = [
    (
        "SAGA #5 - The Count [side B].atr",
        FamilyCScheme::NoLiteral,
        0x5580,
        "the brass bed of room 1, the player's two feet sticking up out of a white sheet",
        35,
        79,
        [0x36, 0x3D, 0x0E, 0x00],
    ),
    (
        "SAGA #4 - Voodoo Castle [side B].atr",
        FamilyCScheme::NoLiteral,
        0x0297,
        "a coffin on a bier between drawn curtains, a candelabrum at each end",
        37,
        63,
        [0x36, 0x87, 0x50, 0x00],
    ),
    (
        "SAGA No. 13 - The Sorcerer of Claymorgue Castle _ side B.atr",
        FamilyCScheme::Standard,
        0x0DC48,
        "four planks of a wooden shelf, end grain and all, against a dark wall",
        38,
        64,
        [0x3A, 0x15, 0x0E, 0x00],
    ),
];

/// The scheme the crate's own release lookup gives this side's release.
///
/// Every case reaches the scheme through here rather than through
/// [`PINNED`]'s and [`BITMAP_TITLES`]' literals, so that changing
/// `SagaUs::picture_scheme` fails the whole suite and not only the one case
/// that pins it — which is what "falsify the fix" asks for.
fn scheme_of(file: &str) -> FamilyCScheme {
    BITMAP_TITLES
        .iter()
        .find(|(f, ..)| *f == file)
        .map(|(_, release, ..)| release.picture_scheme())
        .unwrap_or_else(|| panic!("{file} is not one of the three bitmap titles"))
}

#[test]
fn one_picture_per_title_is_pinned_by_geometry_and_by_pixels() {
    for (file, pinned_scheme, at, what, cols, pairs, colours) in PINNED {
        let Some(raw) = side_b(file) else { continue };
        let scheme = scheme_of(file);
        assert_eq!(scheme, pinned_scheme, "{file}: the release lookup and the pin agree");
        let spliced = splice_vtoc(&raw);
        let found = scan_picture_side(&raw, scheme);
        let r: &AtariRecord = found
            .iter()
            .find(|r| r.file_offset() == at)
            .unwrap_or_else(|| panic!("{file}: no record at 0x{at:05X} ({what})"));
        assert_eq!((r.layout().cols(), r.layout().pairs()), (cols, pairs), "{file}: {what}");
        assert_eq!(r.colour_bytes(), colours, "{file}: {what}");
        let pic = decode_record(&spliced, r, scheme).expect("decodes");
        // A non-vacuity guard, counted over the record's OWN region rather
        // than the canvas — a small record leaves most of the canvas at value
        // 0 quite properly, and a picture that is all one value inside its own
        // region is a decode that failed quietly.
        let mut seen = [0usize; 4];
        let mut total = 0usize;
        for y in r.layout().top()..r.layout().top() + r.layout().pairs() * 2 {
            for x in r.layout().left()..r.layout().left() + r.layout().cols() * 8 {
                if (0..CANVAS_WIDTH as i32).contains(&x) && (0..CANVAS_HEIGHT as i32).contains(&y) {
                    seen[usize::from(pic.pixels()[y as usize * CANVAS_WIDTH + x as usize])] += 1;
                    total += 1;
                }
            }
        }
        let used = seen.iter().filter(|&&n| n > 0).count();
        assert!(used >= 3, "{file}: {what} uses only {used} of the four pixel values");
        assert!(
            seen.iter().all(|&n| n * 10 < total * 9),
            "{file}: {what} is nine-tenths one colour inside its own region, so it did not decode",
        );
    }
}

/// *The Count*'s room 1, pixel by pixel.
///
/// The picture is a brass bed seen from its foot: the bedstead's two upright
/// posts and the rail between them fill the upper third, and the player's two
/// feet stand up from the white sheet across the bottom. Three points are
/// enough to say it is that picture and not a shifted or sheared one.
#[test]
fn the_counts_room_one_draws_the_brass_bed_its_text_describes() {
    let file = "SAGA #5 - The Count [side B].atr";
    let Some(raw) = side_b(file) else { return };
    let scheme = scheme_of(file);
    let spliced = splice_vtoc(&raw);
    let found = scan_picture_side(&raw, scheme);
    let r = found.iter().find(|r| r.file_offset() == 0x5580).expect("room 1's record");
    let pic = decode_record(&spliced, r, scheme).expect("decodes");
    let at = |x: usize, y: usize| pic.pixels()[y * CANVAS_WIDTH + x];
    // The record covers the whole canvas from the origin.
    assert_eq!((r.layout().left(), r.layout().top()), (0, 0));
    // The bed's white sheet is the bottom third, and it is bright.
    let sheet: usize = (120..150).map(|y| (60..220).filter(|&x| at(x, y) == 3).count()).sum();
    assert!(sheet > 3_000, "the sheet across the bottom is {sheet} bright pixels");
    // The wall above the bedstead is not.
    let wall_bright: usize = (0..10).map(|y| (0..CANVAS_WIDTH).filter(|&x| at(x, y) == 3).count()).sum();
    assert!(wall_bright < 900, "the wall along the top is {wall_bright} bright pixels");
    // And the picture is not blank anywhere it should not be.
    assert!(pic.pixels().contains(&1), "the wall colour is in use");
    assert!(pic.pixels().contains(&2), "the third colour is in use");
}
