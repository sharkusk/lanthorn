//! SQ-1491: the *Hulk*'s Commodore 64 family-C colours, checked against the
//! **real machine** rather than against the specification that described them.
//!
//! Every family-C palette lanthorn draws came from §8.3's colour-byte table,
//! and §8.3 says of it that the mapping "was recovered empirically" and §11
//! that tables so derived "may contain mistakes". Internal measurement cannot
//! find such a mistake: our decoder and our tests share the same table, so
//! they agree with each other whatever the machine does. Only a photograph of
//! the machine can falsify it.
//!
//! `machine-screenshots/c64-hulk-{splash,start,transform,chamber}.png` are
//! that photograph — the *Hulk*'s own release disk running under VICE with its
//! default (Pepto) palette, committed to this repository. This suite decodes
//! the same records out of `QUESTPR1.D64` and lays them over the frames: for
//! every stored pixel value in every frame, the set of colours the machine
//! shows at those positions must be a set of **one**, and that one must be
//! what [`scott::saga_pictures::c64_colour`] resolves the record's colour byte
//! to. Get the table wrong and the second half fails; get the *decoder* wrong
//! and the first half does.
//!
//! # The frames, and how each was reached
//!
//! | frame | record | colour bytes | what it is |
//! |---|---|---|---|
//! | `c64-hulk-splash.png` | `R01099` | 198, 103, 142, 0 | the QUESTPROBE title card, before any input |
//! | `c64-hulk-start.png` | `R01001` | 56, 103, 14, 16 | room 1, the opening screen |
//! | `c64-hulk-transform.png` | `R01084` | 196, 101, 14, 0 | the transformation card |
//! | `c64-hulk-chamber.png` | `R01002` + `B01053R` + `B01033R` | 50/66, 135, 14 | room 2 with two object overlays |
//!
//! **The transform frame is `R01084`, not `R01086`.** It was identified by
//! scoring every one of the disk's seventy records against the frame rather
//! than by reading the story: `R01084` agrees on 98.8% of sampled pixels and
//! the runner-up on 38%. A frame is a fixture, and this is which one.
//!
//! # Where the picture sits in the frame
//!
//! Measured, not assumed. The PNGs are VICE's 368x270 output including the
//! border; canvas pixel `(x, y)` is frame pixel `(x + 48, y + 33)` at 1:1,
//! found by sweeping every offset and taking the one where each stored value
//! resolves to a single colour ([`OX`], [`OY`]). Canvas rows 0 and 1 land in
//! the top border and are not drawn, so the comparison runs over rows
//! [`FIRST_VISIBLE_ROW`]`..=`[`LAST_VISIBLE_ROW`] — 156 of the 160. Over that
//! window the agreement is **exact**: not one pixel of any of the four frames
//! disagrees with the decoder.
//!
//! # Skipping
//!
//! The frames are committed; `QUESTPR1.D64` is a gitignored commercial fixture
//! (§10.7), so every case here skips vacuously without it — and says so,
//! because a silent skip reads exactly like a pass.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use scott::c64_palette::PEPTO_PALETTE;
use scott::saga_pictures::{decode_family_c, Rgb, CANVAS_HEIGHT, CANVAS_WIDTH};
use scott::SagaPlatform;

use crate::fixture_paths::fixture_path;

/// Frame x of canvas x = 0 — see the module header.
const OX: u32 = 48;
/// Frame y of canvas y = 0 — see the module header.
const OY: u32 = 33;
/// The first canvas row the display actually shows.
const FIRST_VISIBLE_ROW: usize = 2;
/// The last canvas row the display actually shows.
const LAST_VISIBLE_ROW: usize = 157;

/// One captured frame: the room record it draws, then the object overlays
/// composited over it in `PictureShow` order.
struct Frame {
    png: &'static str,
    records: &'static [&'static str],
}

const FRAMES: [Frame; 4] = [
    Frame { png: "c64-hulk-splash.png", records: &["R01099"] },
    Frame { png: "c64-hulk-start.png", records: &["R01001"] },
    Frame { png: "c64-hulk-transform.png", records: &["R01084"] },
    Frame { png: "c64-hulk-chamber.png", records: &["R01002", "B01053R", "B01033R"] },
];

fn skipped(what: &str) -> bool {
    eprintln!(
        "SKIP: {what} — needs stories/scott-dialects/c64/QUESTPR1.D64 \
         (gitignored commercial fixture, §10.7)"
    );
    true
}

/// Every family-C record on the *Hulk*'s release disk, by file name, reached
/// the way the app reaches them: through the mount, not by a private disk walk.
fn hulk_records() -> Option<BTreeMap<String, Vec<u8>>> {
    let path = fixture_path("scott-dialects/c64/QUESTPR1.D64");
    if !path.exists() {
        return None;
    }
    let mounted = app::hints::load_mounted_story_full(&path, None).ok()?;
    assert!(
        mounted.saga_pictures.len() > 60,
        "QUESTPR1.D64 carries seventy family-C records; the mount found {}",
        mounted.saga_pictures.len()
    );
    Some(mounted.saga_pictures.into_iter().collect())
}

fn screenshot(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../machine-screenshots").join(name)
}

/// The VIC-II index a captured pixel is, or a panic naming the colour — a
/// frame pixel that is not one of the sixteen means the capture was rescaled
/// or recoloured somewhere, and every number derived from it is worthless.
fn vic_index(rgb: Rgb, png: &str) -> usize {
    PEPTO_PALETTE
        .iter()
        .position(|c| *c == rgb)
        .unwrap_or_else(|| panic!("{png} holds {rgb:?}, which is not a VIC-II colour"))
}

/// The four frames' pixel values with, for each, the record whose palette
/// supplies its colour: the room picture everywhere, and each overlay inside
/// its own painted rectangle (§12.11, and an overlay carries its own colour
/// bytes — `B01053R` stores 66 where the room under it stores 50).
fn composite(
    records: &BTreeMap<String, Vec<u8>>,
    names: &[&str],
) -> (Vec<u8>, Vec<usize>, Vec<[Rgb; 4]>) {
    let mut values = Vec::new();
    let mut owner = vec![0usize; CANVAS_WIDTH * CANVAS_HEIGHT];
    let mut palettes = Vec::new();
    for (slot, name) in names.iter().enumerate() {
        let raw = records.get(*name).unwrap_or_else(|| panic!("{name} is on the disk"));
        let pic = decode_family_c(raw, SagaPlatform::Commodore64)
            .unwrap_or_else(|e| panic!("{name} decodes: {e}"));
        palettes.push(pic.palette);
        if slot == 0 {
            values = pic.pixels;
            continue;
        }
        let painted = pic.painted.unwrap_or_else(|| panic!("{name} paints something"));
        for y in painted.top..=painted.bottom {
            for x in painted.left..=painted.right {
                values[y * CANVAS_WIDTH + x] = pic.pixels[y * CANVAS_WIDTH + x];
                owner[y * CANVAS_WIDTH + x] = slot;
            }
        }
    }
    (values, owner, palettes)
}

/// The heart of it: for every (record, stored value) the frame shows exactly
/// one colour, and it is the one the decoder resolved.
///
/// Both halves matter. "Exactly one colour" is a check on the *decoder* — a
/// mis-read pixel lands in the wrong bucket and the bucket stops being pure.
/// "And it is the decoder's" is a check on the colour *table*, which is the
/// half nothing internal to lanthorn could ever have made.
#[test]
fn every_stored_value_draws_one_colour_and_it_is_the_one_we_resolve() {
    let Some(records) = hulk_records() else {
        assert!(skipped("the Commodore 64 colour oracle"));
        return;
    };
    for frame in &FRAMES {
        let img = image::open(screenshot(frame.png))
            .unwrap_or_else(|e| panic!("{} opens: {e}", frame.png))
            .to_rgb8();
        assert_eq!(
            (img.width(), img.height()),
            (368, 270),
            "{} is VICE's 368x270 frame including the border",
            frame.png
        );
        let (values, owner, palettes) = composite(&records, frame.records);

        let mut seen: BTreeMap<(usize, u8), BTreeSet<Rgb>> = BTreeMap::new();
        let mut compared = 0usize;
        for y in FIRST_VISIBLE_ROW..=LAST_VISIBLE_ROW {
            for x in 0..CANVAS_WIDTH {
                let at = y * CANVAS_WIDTH + x;
                let p = img.get_pixel(x as u32 + OX, y as u32 + OY).0;
                seen.entry((owner[at], values[at])).or_default().insert((p[0], p[1], p[2]));
                compared += 1;
            }
        }
        assert_eq!(
            compared,
            CANVAS_WIDTH * (LAST_VISIBLE_ROW + 1 - FIRST_VISIBLE_ROW),
            "{}: the whole visible canvas was compared",
            frame.png
        );

        for ((slot, value), colours) in &seen {
            let name = frame.records[*slot];
            let want = palettes[*slot][usize::from(*value)];
            assert_eq!(
                colours.len(),
                1,
                "{}: {name} value {value} is drawn in {} different colours ({colours:?}) — \
                 the decoder put pixels in the wrong bucket",
                frame.png,
                colours.len()
            );
            let got = *colours.iter().next().expect("just checked");
            assert_eq!(
                got, want,
                "{}: {name} value {value} (colour byte {}) draws as VIC-II {} on the machine \
                 and we resolve it to VIC-II {}",
                frame.png,
                if *value == 0 {
                    "n/a, value 0 is forced black".to_string()
                } else {
                    format!("{}", decode_bytes(&records, name)[usize::from(*value) - 1])
                },
                vic_index(got, frame.png),
                vic_index(want, frame.png),
            );
        }
        // Non-vacuity: a frame that only ever showed value 0 would pass every
        // assertion above and prove nothing.
        let room_values: BTreeSet<u8> =
            seen.keys().filter(|(slot, _)| *slot == 0).map(|(_, v)| *v).collect();
        assert_eq!(
            room_values,
            BTreeSet::from([0, 1, 2, 3]),
            "{}: all four stored values are on screen",
            frame.png
        );
    }
}

fn decode_bytes(records: &BTreeMap<String, Vec<u8>>, name: &str) -> [u8; 4] {
    let raw = &records[name];
    [raw[8], raw[9], raw[10], raw[11]]
}

/// The derived table, stated as a table: each colour byte the four frames
/// exercise, and the VIC-II colour the machine draws it as.
///
/// This is the finding of SQ-1491 in its raw form — re-derived from the
/// captures on every run rather than copied out of them, so it cannot drift
/// from the frames it came from. Two rows **correct** §8.3, which read both 50
/// and 66 as orange.
#[test]
fn the_colour_bytes_the_frames_settle() {
    let Some(records) = hulk_records() else {
        assert!(skipped("the derived colour-byte table"));
        return;
    };
    let mut derived: BTreeMap<u8, BTreeSet<usize>> = BTreeMap::new();
    for frame in &FRAMES {
        let img = image::open(screenshot(frame.png)).expect("opens").to_rgb8();
        let (values, owner, _) = composite(&records, frame.records);
        for y in FIRST_VISIBLE_ROW..=LAST_VISIBLE_ROW {
            for x in 0..CANVAS_WIDTH {
                let at = y * CANVAS_WIDTH + x;
                if values[at] == 0 {
                    continue;
                }
                let byte = decode_bytes(&records, frame.records[owner[at]])
                    [usize::from(values[at]) - 1];
                let p = img.get_pixel(x as u32 + OX, y as u32 + OY).0;
                derived.entry(byte).or_default().insert(vic_index((p[0], p[1], p[2]), frame.png));
            }
        }
    }
    let flat: BTreeMap<u8, usize> = derived
        .iter()
        .map(|(byte, set)| {
            assert_eq!(set.len(), 1, "colour byte {byte} drew as more than one colour: {set:?}");
            (*byte, *set.iter().next().expect("just checked"))
        })
        .collect();
    assert_eq!(
        flat,
        BTreeMap::from([
            (14, 1),  // white
            (50, 2),  // red — §8.3 read this as orange
            (56, 8),  // orange
            (66, 2),  // red — §8.3 read this as orange
            (101, 4), // purple
            (103, 4), // purple
            (135, 6), // blue
            (142, 1), // white
            (196, 5), // green
            (198, 5), // green
        ]),
        "the byte -> VIC-II index table the four frames settle"
    );
}

/// The boot screen's four colour bars are drawn in **text mode**, not from
/// `B01250R`, and are the disk's only real-machine sighting of colour byte 232.
///
/// `machine-screenshots/c64-hulk-colorbars.png` shows four 64-pixel bars —
/// red, yellow, blue, green — over "Adjust your TV to match above colors", and
/// their four colours are exactly what our table resolves `B01250R`'s four
/// header bytes (50, 135, 232, 198) to. That is where byte 232 comes from:
/// §8.3's table has no entry for it, three of the other four are pinned by the
/// picture frames above, and the hue/luminance structure of the encoding
/// predicts hue 14 luminance 8 as gold before the frame is looked at.
///
/// **It is not that record's bitmap**, and the geometry is why: the bars are
/// exactly 64 pixels wide on 8-pixel boundaries and exactly 160 rows tall on
/// an 8-row boundary, which is a 32x20 block of reversed spaces in the text
/// screen. `B01250R` decodes to *three* bars of 58, 86 and 78 pixels starting
/// at canvas x = 16 — and its data is a perfect fit for that: 51 bytes of
/// run-length expand to 1,856 pixel pairs, and its declared region is 29
/// columns x 64 pairs = 1,856 exactly, with nothing left over. §8.3's
/// no-literal variant expands the same bytes to 4,015 pairs, which divides by
/// neither 63 nor 64. So the decode is arithmetically exact and the screen is
/// simply a different drawing; do not "fix" the decoder to match this frame.
#[test]
fn the_boot_screen_bars_are_text_mode_and_settle_colour_byte_232() {
    let img = image::open(screenshot("c64-hulk-colorbars.png")).expect("opens").to_rgb8();
    // The text screen's origin in this frame: 24 px of left border, 35 of top.
    // (The bitmap screens sit two rows higher — see the module header.)
    const TEXT_OX: u32 = 24;
    const TEXT_OY: u32 = 35;
    for (bar, byte) in [(0u32, 50u8), (1, 232), (2, 135), (3, 198)] {
        let want = scott::saga_pictures::c64_colour(byte)
            .unwrap_or_else(|| panic!("byte {byte} resolves"));
        for dy in 0..160u32 {
            for dx in 0..64u32 {
                let p = img.get_pixel(TEXT_OX + bar * 64 + dx, TEXT_OY + 16 + dy).0;
                assert_eq!(
                    (p[0], p[1], p[2]),
                    want,
                    "bar {bar} (B01250R's colour byte {byte}) at +{dx},+{dy}"
                );
            }
        }
    }

    let Some(records) = hulk_records() else {
        assert!(skipped("B01250R's own geometry"));
        return;
    };
    let pic = decode_family_c(&records["B01250R"], SagaPlatform::Commodore64).expect("decodes");
    let painted = pic.painted.expect("paints");
    assert_eq!(
        (painted.left, painted.right, painted.top, painted.bottom),
        (16, 247, 0, 127),
        "B01250R's own rectangle is not the bars' 0..255 x 0..159"
    );
}
