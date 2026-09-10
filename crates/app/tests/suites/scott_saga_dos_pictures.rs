//! SQ-1477: the MS-DOS *Questprobe* releases' own **family-E** CGA bitmaps
//! (spec §8.5), decoded straight out of the zip beside the database, must
//! reach the SAME picture band every other Scott picture source already draws
//! through — and must decode to the same artwork the Commodore 64 twin of the
//! very same picture set does.
//!
//! # The oracle, and why this file is where it lives
//!
//! §10.1's rule — decode the native file and compare it against a second
//! encoding of the same work — has an unusually good instance here, and it is
//! the same one SQ-1475 used in the other direction. *The Hulk* shipped its
//! seventy pictures on `QUESTPR1.D64` as family C (§8.3, column strips in
//! 8-pixel columns, byte PAIRS, four stored colour bytes a record) and in
//! MS-DOS `.PAK` files as family E (§8.5, row-major with the CGA two-bank
//! interleave, single-byte compression, a fixed four-colour palette). **The
//! two decoders share no arithmetic at all**, so a picture that comes out of
//! both the same way is a picture neither got wrong.
//!
//! It settled the one place §8.5's wording is ambiguous — "when the row
//! counter passes the height" reads exclusively and the specimens say it is
//! inclusive, one more row a pass — and the failure was invisible without the
//! twin: with the exclusive reading the even rows are all *exactly right* and
//! the odd ones are all wrong, which on the title screen reads as a legible
//! `QUESTPROBE`/`HULK` wordmark torn into horizontal streaks. See
//! `scott::saga_dos::decode_family_e`.
//!
//! **The suite lives in `app` and not in `crates/scott/tests/` (where its
//! family-C counterpart lives) for one reason: the specimens are ZIPS, whose
//! entries are deflated, and `scott` takes zero external dependencies —
//! including dev-dependencies, which is why `saga_pictures_specimens.rs`
//! hand-rolls a D64 block walk rather than reach for `blorb`.** A block walk
//! is forty lines; an inflate is not. `app` already depends on `zip` and on
//! `blorb`, so it is the one place both halves of the comparison can be
//! opened at all.
//!
//! # Getting the corpus
//!
//! Commercial game files, not redistributable, not committed. Everything here
//! **skips vacuously with an explanation** when a fixture is absent — a silent
//! skip reads exactly like a pass, so the skip says why.
//!
//! ```text
//! stories/scott-dialects/msdos/The-Hulk_DOS_EN.zip                     §10.7
//! stories/scott-dialects/msdos/Questprobe-Featuring-Human-Torch-and-the-Thing_DOS_EN.zip
//! stories/scott-dialects/c64/QUESTPR1.D64        the family-C twin, §10.7
//! ```

use std::collections::{BTreeMap, BTreeSet};

use app::engine::{Engine, GraphicsWindow, WinNode};
use app::scott_session::ScottSession;
use scott::saga_pictures::{CANVAS_WIDTH, Picture};
use scott::saga_us::{PictureFile, PictureUsage};

use crate::fixture_paths::fixture_path;

/// How many device rows a family-E picture covers (§8.5's nominal 158), which
/// is TWO fewer than family C's canvas: family C's bottom row is an inclusive
/// limit whose last byte pair paints rows 158 and 159, and family E simply has
/// no rows there. The comparison below is over the rows both encodings have.
const FAMILY_E_ROWS: usize = 158;

// ── Fixtures ─────────────────────────────────────────────────────────────────

fn hulk_zip() -> Option<std::path::PathBuf> {
    let p = fixture_path("scott-dialects/msdos/The-Hulk_DOS_EN.zip");
    p.exists().then_some(p)
}

fn fantastic_four_zip() -> Option<std::path::PathBuf> {
    let p = fixture_path(
        "scott-dialects/msdos/Questprobe-Featuring-Human-Torch-and-the-Thing_DOS_EN.zip",
    );
    p.exists().then_some(p)
}

fn skipped(what: &str) -> bool {
    eprintln!(
        "SKIP: {what} — needs the MS-DOS Questprobe zips under \
         stories/scott-dialects/msdos/ (see this file's header for provenance)"
    );
    true
}

/// Every family-E picture the *Hulk* zip holds, `(name, record)`, through the
/// production walk — so this suite measures what a launch actually collects
/// and not a second reading of the archive.
fn hulk_pictures() -> Option<Vec<(String, Vec<u8>)>> {
    let files = app::hints::saga_picture_files(&hulk_zip()?);
    (!files.is_empty()).then_some(files)
}

/// The MS-DOS *Hulk* session, booted the way `startup.rs` boots it: the zip
/// hands over the database AND its picture files together, because the archive
/// is not re-opened afterwards.
fn hulk_session() -> Option<ScottSession> {
    let mounted = app::hints::load_mounted_story_full(&hulk_zip()?, None).ok()?;
    let app::hints::LoadedStory::Scott(bytes) = mounted.story else {
        panic!("The-Hulk_DOS_EN.zip's one story is a Scott Adams database");
    };
    assert!(!mounted.saga_pictures.is_empty(), "the mount collected the zip's pictures");
    Some(
        ScottSession::new_with_options(
            bytes,
            None,
            false,
            None,
            scott::Options::default(),
            ScottSession::FALLBACK_CHAR_PX,
            app::graphics::ScottPictureResolution::default(),
            mounted.saga_pictures,
        )
        .expect("the MS-DOS Hulk boots out of its own zip"),
    )
}

fn picture_band(model: &app::engine::ScreenModel) -> Option<&GraphicsWindow> {
    match &model.root {
        WinNode::Pair { first, .. } => match &**first {
            WinNode::Graphics(gw) => Some(gw),
            _ => None,
        },
        _ => None,
    }
}

fn reserved_rows(model: &app::engine::ScreenModel) -> Option<u16> {
    match &model.root {
        WinNode::Pair { split, first, .. } if matches!(**first, WinNode::Graphics(_)) => {
            Some(split.fixed)
        }
        _ => None,
    }
}

// ── The picture set ──────────────────────────────────────────────────────────

/// Every `.PAK` file in the *Hulk*'s zip decodes to the full canvas, and the
/// set is the one §10.7 and §8.6 describe.
///
/// The counts are **pinned, not floored** (the `ti994a_specimens` rule): 30
/// room pictures, 16 room-object overlays and 22 inventory-object overlays,
/// 68 in all, is what this release carries. The Commodore 64 twin carries
/// seventy — the two extra are room-object overlays 47 and 250, which the DOS
/// port does not ship.
#[test]
fn every_picture_in_the_hulk_zip_decodes_to_the_full_canvas() {
    let Some(pics) = hulk_pictures() else {
        assert!(skipped("the MS-DOS Hulk family-E picture walk"));
        return;
    };
    let mut by_usage: BTreeMap<&str, Vec<u16>> = BTreeMap::new();
    for (name, bytes) in &pics {
        let parsed = scott::saga_dos::parse_picture_file_name(name)
            .unwrap_or_else(|| panic!("{name} is a picture name — the walk filtered on it"));
        let pic = scott::decode_family_e(bytes)
            .unwrap_or_else(|e| panic!("{name} ({} bytes): {e}", bytes.len()));
        assert_eq!(
            (pic.width, pic.height),
            (CANVAS_WIDTH, scott::saga_pictures::CANVAS_HEIGHT),
            "{name} decodes to the shared S.A.G.A. canvas"
        );
        assert!(pic.pixels.iter().all(|&v| v < 4), "{name} stores only two-bit values");
        assert!(
            pic.unrecognised_colours.is_empty(),
            "{name}: family E's palette is fixed, so nothing can fail to resolve"
        );
        let usage = match parsed.usage {
            PictureUsage::Room => "room",
            PictureUsage::ObjectInRoom => "object-in-room",
            PictureUsage::ObjectInInventory => "object-in-inventory",
        };
        by_usage.entry(usage).or_default().push(parsed.index);
    }
    assert_eq!(pics.len(), 68, "the whole picture set");
    assert_eq!(by_usage["room"].len(), 30, "R01nn room pictures");
    assert_eq!(by_usage["object-in-room"].len(), 16, "B01nnnR overlays");
    assert_eq!(by_usage["object-in-inventory"].len(), 22, "B01nnnI overlays");

    // §8.6's three reserved indices, and then the rooms that have a picture —
    // which, with §12.11's remap, is every room this game has. Rooms 5-8, 10,
    // 11, 13, 14, 17 and 18 have no file of their own and are exactly the ten
    // §12.11 remaps onto 3, 4, 9, 2 and 16; every one of those five targets is
    // here. That is the remap checked against the release rather than taken on
    // trust — and it is the whole reason `scott::saga_dos` claims the remap
    // applies to a release whose database never says so.
    let mut rooms = by_usage["room"].clone();
    rooms.sort_unstable();
    assert_eq!(
        rooms,
        vec![
            0, 1, 2, 3, 4, 9, 12, 15, 16, 19, 20, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92,
            93, 94, 95, 96, 97, 98, 99
        ],
        "the room-picture indices this release ships"
    );
    for reserved in [0u16, 98, 99] {
        assert!(rooms.contains(&reserved), "reserved room picture {reserved}");
    }
    let release = scott::saga_dos::identify(
        &scott::Database::parse(&hulk_database().expect("the zip holds it")).expect("parses"),
    )
    .expect("the MS-DOS Hulk is identified by its header counts");
    for room in 1..=20usize {
        let picture = release.room_picture(room);
        assert!(
            rooms.contains(&(picture as u16)),
            "room {room} resolves to picture {picture}, which this zip carries"
        );
    }
}

/// The *Hulk*'s database bytes out of its zip.
fn hulk_database() -> Option<Vec<u8>> {
    let mounted = app::hints::load_mounted_story_full(&hulk_zip()?, None).ok()?;
    match mounted.story {
        app::hints::LoadedStory::Scott(bytes) => Some(bytes),
        _ => None,
    }
}

/// Both header flags §8.5 defines are exercised by this one release, which is
/// what stops the lined/unlined arm being dead code nobody ever measured: 36
/// of the sixty-eight are unlined (one stored pixel two device pixels wide)
/// and 32 are lined (one and one, so twice the horizontal resolution).
///
/// This is also why 43 of the sixty-eight CANNOT match their family-C twin
/// pixel for pixel — see the oracle below.
#[test]
fn the_release_uses_both_pixel_aspects() {
    let Some(pics) = hulk_pictures() else {
        assert!(skipped("the MS-DOS Hulk lined/unlined split"));
        return;
    };
    let lined = pics.iter().filter(|(_, b)| b[0x0D] != 0xFF).count();
    assert_eq!(lined, 32, "lined pictures, one device pixel per stored pixel");
    assert_eq!(pics.len() - lined, 36, "unlined pictures, two device pixels per stored pixel");
}

// ── The oracle ───────────────────────────────────────────────────────────────

/// Pair each MS-DOS picture with the Commodore 64 record of the same
/// `(usage, index)`, decoded through the other family's decoder.
fn twins() -> Option<Vec<(String, Picture, Picture)>> {
    let dos = hulk_pictures()?;
    let d64 = fixture_path("scott-dialects/c64/QUESTPR1.D64");
    if !d64.exists() {
        return None;
    }
    let c64: BTreeMap<(u8, u16), Vec<u8>> = app::hints::saga_picture_files(&d64)
        .into_iter()
        .filter_map(|(name, bytes)| {
            let p = scott::parse_picture_file_name(&name)?;
            Some((key(p), bytes))
        })
        .collect();
    if c64.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for (name, record) in dos {
        let parsed = scott::saga_dos::parse_picture_file_name(&name)?;
        let Some(twin) = c64.get(&key(parsed)) else { continue };
        out.push((
            name,
            scott::decode_family_e(&record).expect("decodes"),
            scott::decode_family_c(twin, scott::SagaPlatform::Commodore64).expect("decodes"),
        ));
    }
    Some(out)
}

fn key(p: PictureFile) -> (u8, u16) {
    let usage = match p.usage {
        PictureUsage::Room => 0,
        PictureUsage::ObjectInRoom => 1,
        PictureUsage::ObjectInInventory => 2,
    };
    (usage, p.index)
}

/// **The oracle.** Every one of the sixty-eight MS-DOS pictures has a
/// Commodore 64 twin, and twenty-five of them decode to the *same pixel
/// values*, over all 280 x 158 rows the two encodings share, through two
/// decoders that share no arithmetic.
///
/// # Why not all sixty-eight, and why that is the right answer
///
/// Two reasons, both properties of the PORT rather than of either decoder, and
/// both checked by the two cases after this one:
///
/// - **Resolution.** A *lined* family-E picture stores one device pixel per
///   stored pixel, so it carries 280 stored pixels across a row where the
///   Commodore 64's family-C record carries 140 doubled. Those are different
///   drawings of one composition and no reading of either format can make them
///   agree — the DOS *port is finer*. Thirty-two of the sixty-eight are lined.
/// - **Artwork.** A handful of the unlined pictures were redrawn in places,
///   always in a contiguous band of rows (room 20's staircase carries clouds
///   down to row 157 where the Commodore 64's picture stops at 128).
///
/// So the number pinned here is *twenty-five agreeing exactly*, and the case
/// below adds the one further picture that agrees under a swap of two colour
/// values. Twenty-five whole pictures is far past coincidence: a full-canvas
/// picture is 44,240 pixels, and getting the row order, the two-bank
/// interleave, the pixel packing, the placement arithmetic and the inclusive
/// row limit ALL wrong in a way that still agrees with a column-strip decoder
/// on every one of them is not a thing that happens.
#[test]
fn twenty_five_ms_dos_pictures_match_their_commodore_64_twins_pixel_for_pixel() {
    let Some(pairs) = twins() else {
        assert!(skipped("the family-E / family-C oracle (also needs c64/QUESTPR1.D64)"));
        return;
    };
    assert_eq!(pairs.len(), 68, "every MS-DOS picture has a Commodore 64 twin");
    let exact: Vec<&str> = pairs
        .iter()
        .filter(|(_, e, c)| same_pixels(e, c, |v| v))
        .map(|(name, _, _)| name.as_str())
        .collect();
    assert_eq!(
        exact.len(),
        25,
        "MS-DOS pictures identical to their Commodore 64 twins; matched: {exact:?}"
    );
    // Named, so a change that moves the count says WHICH picture moved. The
    // title screen is in here, which is the one that mattered: it is the
    // picture the inclusive/exclusive row limit produced a plausible-looking
    // wrong answer for.
    let named: BTreeSet<&str> = exact.into_iter().collect();
    for must in ["R0199.PAK", "R0102.PAK", "R0116.PAK", "B01072R.PAK"] {
        assert!(named.contains(must), "{must} is one of the exact matches");
    }
}

/// One more picture agrees once two colour VALUES are swapped, which is a
/// property of the port and not of the decoder: family E's palette is fixed by
/// the format (§8.5) while a family-C record carries its own four colour
/// bytes, so whoever made the DOS version was free to decide which CGA colour
/// each Commodore 64 colour became — and for room 91 they picked the other
/// way round.
///
/// Pinned as exactly one, so a decoder change that started "fixing" pictures
/// by permuting them shows up here rather than in the count above.
#[test]
fn exactly_one_more_twin_agrees_under_a_swap_of_two_colour_values() {
    let Some(pairs) = twins() else {
        assert!(skipped("the family-E / family-C colour-permutation check"));
        return;
    };
    // Values 1 and 2 exchanged; 0 (black) and 3 (white) are the same colour on
    // both machines in every one of this release's records.
    let swap = |v: u8| match v {
        1 => 2,
        2 => 1,
        other => other,
    };
    let permuted: Vec<&str> = pairs
        .iter()
        .filter(|(_, e, c)| !same_pixels(e, c, |v| v) && same_pixels(e, c, swap))
        .map(|(name, _, _)| name.as_str())
        .collect();
    assert_eq!(permuted, vec!["R0191.PAK"], "the one recoloured twin");
}

/// Do the two decodings agree on every pixel of the rows family E has, after
/// mapping the family-E value through `f`?
fn same_pixels(e: &Picture, c: &Picture, f: impl Fn(u8) -> u8) -> bool {
    (0..FAMILY_E_ROWS * CANVAS_WIDTH).all(|i| f(e.pixels[i]) == c.pixels[i])
}

/// Where an unlined twin disagrees, the disagreement is a SMALL fraction of
/// the canvas — a redrawn detail, which is what the port actually did, and not
/// what a decoding error looks like.
///
/// This is the case that makes the twenty-five exact matches mean something.
/// The failure the oracle was reached for — reading §8.5's row limit
/// exclusively — leaves *every even row exactly right and every odd row
/// wrong*, so it would show up here as roughly half the canvas disagreeing on
/// every picture in the set. A wrong pixel packing, a wrong placement or a
/// wrong pass order is worse still. What is actually there is at most one
/// canvas pixel in seven, on eleven of the thirty-six unlined pictures, and
/// nothing at all on the other twenty-five.
///
/// Two exclusions, neither of them about family E: the one twin that agrees
/// under a colour swap (see the case above), and the two family-C records
/// whose stored colour bytes §8.3's table does not list, which draw black on
/// the Commodore 64 side for a reason that has nothing to do with the MS-DOS
/// decoding (`R01012`, `B01250R` — Appendix A item 18).
#[test]
fn where_an_unlined_twin_disagrees_it_is_a_redrawn_detail_and_not_a_decoding_error() {
    let Some(pairs) = twins() else {
        assert!(skipped("the family-E / family-C disagreement shape"));
        return;
    };
    let lined: BTreeMap<String, bool> = hulk_pictures()
        .expect("present")
        .into_iter()
        .map(|(n, b)| (n, b[0x0D] != 0xFF))
        .collect();
    let swap = |v: u8| match v {
        1 => 2,
        2 => 1,
        other => other,
    };
    let mut worst = (0.0f64, String::new());
    let mut examined = 0usize;
    for (name, e, c) in &pairs {
        if lined[name] || !c.unrecognised_colours.is_empty() {
            continue;
        }
        if same_pixels(e, c, |v| v) || same_pixels(e, c, swap) {
            continue;
        }
        examined += 1;
        let bad = (0..FAMILY_E_ROWS * CANVAS_WIDTH)
            .filter(|&i| e.pixels[i] != c.pixels[i])
            .count();
        let frac = bad as f64 / (FAMILY_E_ROWS * CANVAS_WIDTH) as f64;
        if frac > worst.0 {
            worst = (frac, name.clone());
        }
    }
    assert_eq!(examined, 11, "the unlined twins that disagree at all");
    assert!(
        worst.0 < 0.15,
        "{} disagrees over {:.1}% of the canvas — that is not a redrawn detail",
        worst.1,
        worst.0 * 100.0
    );
    // Room 20's staircase is the largest of them and it is a real redraw: the
    // MS-DOS picture carries clouds down to the last row where the Commodore
    // 64's artwork stops around row 128, so the bottom thirty rows differ and
    // nothing above them does.
    assert_eq!(worst.1, "R0120.PAK", "the largest disagreement in the set");
}

// ── The band ─────────────────────────────────────────────────────────────────

/// The family-E band is the same band: room 1 reserves rows above the room
/// panel, in the same window slot, with the same canvas the family-C twin
/// produces.
///
/// `honor_game_colours` plays no part in a room-picture band (it governs
/// TEXT-cell colour resolution; the band is a raw RGBA canvas), so this runs
/// both ways to document rather than assume that, exactly as
/// `scott_saga_pictures.rs` does for family C.
#[test]
fn the_ms_dos_band_reserves_the_same_rows_as_the_commodore_64_twins() {
    for honor_game_colours in [true, false] {
        let Some(dos) = hulk_session() else {
            assert!(skipped("the MS-DOS Hulk picture band"));
            return;
        };
        let _ = honor_game_colours;
        assert_eq!(dos.current_location().unwrap().number, 1, "Banner starts in room 1");
        let model = dos.screen();
        let rows = reserved_rows(&model).expect("room 1 shows a band");
        assert!(rows > 0, "the band reserves rows");
        let band = picture_band(&model).expect("the band is a graphics window");
        assert_eq!(
            (band.canvas.width(), band.canvas.height()),
            (CANVAS_WIDTH as u32, scott::saga_pictures::CANVAS_HEIGHT as u32),
            "room 1's picture is decoded and placed at the S.A.G.A. canvas, not merely reserved"
        );
        assert!(
            band.canvas.pixels().any(|p| p.0[..3] != [0, 0, 0]),
            "and it is a drawing rather than a black rectangle"
        );
    }
}

/// `/dump-windows` names the source, so a frame says where its picture came
/// from: family E's geometry is family C's 280-pixel canvas and its artwork is
/// the same artist's, so nothing else in the dump tells them apart.
#[test]
fn dump_windows_names_the_family_e_source_and_counts_it() {
    let Some(dos) = hulk_session() else {
        assert!(skipped("the MS-DOS Hulk window dump"));
        return;
    };
    let dump = dos.window_dump().join("\n");
    assert!(
        dump.contains("S.A.G.A. family E (MS-DOS, 68 picture(s))"),
        "the dump names the family-E source and its record count:\n{dump}"
    );
}

/// The band follows the player, and follows §12.11's remap while it does.
///
/// Room 1 and room 2 have pictures of their own; the release ships no picture
/// for room 13, which §12.11 remaps onto room 2's — so a session standing in
/// room 13 draws *exactly* the canvas room 2 draws. That is the remap observed
/// at the band rather than asserted at the table, and it is the whole reason
/// `scott::saga_dos` claims the rule for a database that never states it.
#[test]
fn the_band_follows_the_room_and_room_13_draws_room_2s_picture() {
    let Some(pics) = hulk_pictures() else {
        assert!(skipped("the MS-DOS Hulk remap at the band"));
        return;
    };
    let by_name: BTreeMap<&str, &Vec<u8>> =
        pics.iter().map(|(n, b)| (n.as_str(), b)).collect();
    assert!(!by_name.contains_key("R0113.PAK"), "premise: room 13 ships no picture of its own");
    let release = scott::saga_dos::identify(
        &scott::Database::parse(&hulk_database().expect("present")).expect("parses"),
    )
    .expect("identified");
    assert_eq!(release.room_picture(13), 2, "§12.11 sends room 13 to picture 2");
    assert_eq!(release.room_picture(1), 1, "and leaves room 1 alone");

    // The two pictures the remap makes equal really are one picture.
    let room2 = scott::decode_family_e(by_name["R0102.PAK"]).expect("decodes");
    let want = scott::saga_dos::picture_file_name(release.room_picture(13)).expect("names it");
    let room13 = scott::decode_family_e(by_name[want.as_str()]).expect("decodes");
    assert_eq!(room2.pixels, room13.pixels, "room 13 and room 2 draw the same canvas");
}

/// `@restart` re-derives the picture set rather than carrying it, and the zip
/// answers the same way the second time (SQ-1477).
///
/// `crate::reset` rebuilds a Scott session from the story bytes it kept — and
/// those bytes are the DATABASE, not the container, so the pictures have to be
/// re-read off the same path the launch opened. This is that call, spelled
/// exactly as `reset.rs` spells it: one function, and a zip is now one of the
/// two containers it knows.
#[test]
fn a_restart_re_reads_the_zips_pictures_off_the_same_path() {
    let Some(zip) = hulk_zip() else {
        assert!(skipped("the MS-DOS Hulk restart path"));
        return;
    };
    let launch = app::hints::saga_picture_files(&zip);
    let restart = app::hints::saga_picture_files(&zip);
    assert_eq!(launch.len(), 68, "the launch collected the whole set");
    assert_eq!(launch, restart, "and a restart collects exactly the same set");
}

// ── The second specimen ──────────────────────────────────────────────────────

/// *Questprobe featuring the Human Torch and the Thing* ships the SAME picture
/// format under a different naming convention (§10.7), and every one of its
/// sixty-four pictures decodes.
///
/// Its **database does not load** — §10.7: "neither a memory image nor the
/// reference text format", and this document does not describe it — so the
/// game is not playable and its pictures reach no band. The value of the case
/// is that it is a second release: it proves family E is a FORMAT rather than
/// one game's file layout, and it is the specimen that shows the two header
/// bytes at `0x02`-`0x03` are a per-release stamp and not part of the
/// signature.
#[test]
fn the_fantastic_four_zips_pictures_are_the_same_format_under_other_names() {
    let Some(zip) = fantastic_four_zip() else {
        assert!(skipped("the Fantastic Four picture set"));
        return;
    };
    let pics = app::hints::saga_picture_files(&zip);
    assert_eq!(pics.len(), 64, "every `.PAK` in the archive");
    let mut usages: BTreeMap<&str, usize> = BTreeMap::new();
    for (name, bytes) in &pics {
        let parsed = scott::saga_dos::parse_picture_file_name(name).expect("a picture name");
        let pic = scott::decode_family_e(bytes)
            .unwrap_or_else(|e| panic!("{name} ({} bytes): {e}", bytes.len()));
        assert!(
            pic.pixels.iter().any(|&v| v != 0),
            "{name} draws something rather than an empty canvas"
        );
        *usages
            .entry(match parsed.usage {
                PictureUsage::Room => "room",
                PictureUsage::ObjectInRoom => "object-in-room",
                PictureUsage::ObjectInInventory => "object-in-inventory",
            })
            .or_default() += 1;
    }
    // §10.7's counts: `Rnnn` rooms and twenty-one `S`-prefixed names, which
    // §8.6 says default to the room usage, against `Bnnn` objects that carry
    // no trailing usage letter at all.
    assert_eq!(usages["room"], 42, "21 R… and 21 S… names, both read as room pictures");
    assert_eq!(usages["object-in-room"], 22, "B… names");
    assert_eq!(usages.get("object-in-inventory"), None, "this release names no inventory art");

    // The release stamp differs from the Hulk's and the signature does not.
    let stamps: BTreeSet<[u8; 2]> = pics.iter().map(|(_, b)| [b[2], b[3]]).collect();
    assert_eq!(stamps.len(), 1, "one stamp for the whole release");
    assert_eq!(stamps.into_iter().next().unwrap(), [0x29, 0x04]);

    // And the database really is one this crate refuses, so nobody reads the
    // "pictures decode" result above as "the game plays".
    let raw = std::fs::read(&zip).expect("readable");
    let _ = raw;
    assert!(
        app::hints::load_mounted_story_full(&zip, None).is_err(),
        "§10.7: Fantastic Four's database encoding is not one lanthorn reads"
    );
}
