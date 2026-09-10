//! SQ-1491: family B's Commodore 64 **remap table A**, checked against the
//! real machine — the other half of the Commodore 64 colour question.
//!
//! §8.2 says the Commodore 64 releases store the *ZX Spectrum's* colour indices
//! in their attribute bytes ("the artwork was converted, the numbering was
//! not"), so every stored index passes through a sixteen-entry remap before the
//! palette lookup — and §11 says of all four remaps that they "were derived by
//! eye and may contain mistakes". The family-C half of this quest found two
//! such mistakes in §8.3's table by photographing the machine
//! (`scott_c64_picture_colours.rs`); this is the same instrument pointed at
//! §8.2, and it finds none.
//!
//! `machine-screenshots/c64-golden-{1,2,3}.png` are *The Golden Baton* off
//! `MYSTADV1.D64` under VICE. This suite rasterises the same pictures through
//! `scott::c64` and asks, per **stored** index, which VIC-II colour the machine
//! draws it in — one colour per index, and that colour `PALETTE[index]`.
//!
//! # The frames, and which picture each is
//!
//! Identified by scoring every one of the release's thirty-one pictures against
//! every frame rather than by reading the map, the way the *Hulk*'s transform
//! card was:
//!
//! | frame | picture | room | agreement |
//! |---|---|---|---|
//! | `c64-golden-1.png` | 0 | 1, "dense SPOOKY Forest" | 95.1% |
//! | `c64-golden-2.png` | 1 | 2, "I'm by a Stream" | 95.7% |
//! | `c64-golden-3.png` | 5 | **6**, "I'm by a Path" | 96.9% |
//!
//! **The third frame is room 6, not room 3** — two moves north from the forest
//! do not land on room 3, and a suite that assumed they did would have compared
//! the wrong picture. Picture index is the room number minus one throughout.
//!
//! # Why fill INTERIORS and not every pixel
//!
//! The colour question is answered over **eroded interiors** — a pixel counts
//! only when all eight of its neighbours carry the same stored index — because
//! that is the part of the canvas whose *position* is not in doubt. A
//! one-pixel line has no pixel that survives a one-pixel disagreement, so
//! reading a colour off one would be answering a palette question through a
//! placement question. On interiors the agreement is 100% on
//! `c64-golden-3.png` and 98.5-99.6% per index elsewhere, and the shortfall
//! has a shape worth pinning rather than tolerating: the only colour a fill
//! ever disagrees into is **black**, never another fill's colour — our flood
//! fill reaches a little further than the machine's in two places, and never
//! into the wrong colour. [`FILL_PURITY_FLOOR`] and
//! [`only_black_is_ever_the_disagreement`] hold that line.
//!
//! # The whole frame, and the colour clash
//!
//! Whole-frame agreement was **95.0%** when this suite was written, and every
//! missing pixel was on or beside a one-pixel line — which looked like a
//! Bresenham difference and was not. Sweeping every plausible line rule (error
//! initialisation, tie direction, endpoint order, DDA with truncation and with
//! rounding, sixteen combinations) moved it between 92.1% and 95.0% and never
//! above, and the line-pixel sets said why: **the machine never lights a line
//! pixel we do not**, in any of the three frames. Our set was a strict
//! superset, and 97% of the surplus was showing a *fill's* colour on the
//! machine rather than the background.
//!
//! The cause is the machine, not the geometry. Sweeping the cell grid's
//! vertical phase against the frames finds exactly one at which **no 8 x 8
//! cell of any frame holds three colours** — the signature of a
//! high-resolution bitmap, one bit per pixel and one ink per cell. An outline
//! drawn first and flooded past second comes out in the flood's ink wherever
//! the two share a cell. Modelling that ([`scott::c64`]'s `Ink`, SQ-1491) took
//! whole-frame agreement to **99.2%**, and what is left is the flood-fill
//! reach above plus 37 pixels of genuine line placement across all three
//! frames. [`the_whole_frame_agrees_but_for_the_fills_reach`] pins it, and
//! [`the_machine_never_shows_three_colours_in_a_cell`] re-derives the phase.
//!
//! # The line colour, which has no interior at all
//!
//! Index 7 is the line index (§8.2: the line colour is 0 unless the background
//! index is 0, when it is 7), and a one-pixel line has no eroded interior, so
//! the method above cannot reach it. It is derived by **elimination** instead,
//! which on these three frames is exact: take the colours the picture area
//! shows, subtract the ones the eroded fills account for, and exactly one
//! colour is left over in each frame. It is white in all three.
//!
//! # What three frames can and cannot settle
//!
//! Line art uses few colours, so these three exercise **six** of the sixteen
//! stored indices. The other ten are untouched and stay exactly as §8.2 has
//! them, listed in [`UNVERIFIED`] with no claim made either way. The Appendix A
//! item names what would reach the rest.
//!
//! # Skipping
//!
//! The frames are committed; `BATON.prg` is a gitignored commercial fixture
//! (§10.4), so every case here skips vacuously without it — and says so,
//! because a silent skip reads exactly like a pass.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use scott::c64::{decode_family_b_picture_lists, prg_image, PALETTE};
use scott::c64_palette::PEPTO_PALETTE;
use scott::saga_pictures::Rgb;

use crate::fixture_paths::fixture_path;

/// Frame x of canvas x = 0, measured by sweeping every offset — see
/// [`the_alignment_is_the_measured_one`].
const OX: u32 = 56;
/// Frame y of canvas y = 0, measured the same way.
const OY: u32 = 36;

const CANVAS_W: usize = scott::c64::PICTURE_WIDTH;
const CANVAS_H: usize = scott::c64::PICTURE_HEIGHT;

/// The lowest share of a fill's eroded interior that may agree with the machine
/// before this suite calls it a colour question rather than a fill-extent one.
///
/// Measured **per index**, which is the tighter reading: every index of
/// `c64-golden-3.png` is exactly 1.0, and the worst anywhere is 0.9886 —
/// stored index 2 on `c64-golden-1.png`, 63 pixels of 5,513. See the module
/// header for the shape of the shortfall, and
/// [`only_black_is_ever_the_disagreement`] for what those 63 pixels are.
const FILL_PURITY_FLOOR: f64 = 0.98;

/// The stored indices no frame in this repository exercises.
///
/// **Not a claim that §8.2 has them wrong** — a claim that nothing here has
/// checked them. Six of the sixteen are settled; these are the rest.
const UNVERIFIED: [u8; 10] = [1, 3, 8, 9, 10, 11, 12, 13, 14, 15];

/// The line index, which has no eroded interior — see the module header.
const LINE_INDEX: u8 = 7;

struct Frame {
    png: &'static str,
    picture: usize,
    room: &'static str,
}

const FRAMES: [Frame; 3] = [
    Frame { png: "c64-golden-1.png", picture: 0, room: "1, dense SPOOKY Forest" },
    Frame { png: "c64-golden-2.png", picture: 1, room: "2, I'm by a Stream" },
    Frame { png: "c64-golden-3.png", picture: 5, room: "6, I'm by a Path" },
];

fn skipped(what: &str) -> bool {
    eprintln!(
        "SKIP: {what} — needs stories/scott-dialects/c64/prg/MYSTADV1.D64/BATON.prg \
         (gitignored commercial fixture, §10.4)"
    );
    true
}

fn baton_pictures() -> Option<Vec<scott::c64::Picture>> {
    let raw = std::fs::read(fixture_path("scott-dialects/c64/prg/MYSTADV1.D64/BATON.prg")).ok()?;
    let (image, load) = prg_image(&raw)?;
    let lists = decode_family_b_picture_lists(image, load).ok()?;
    assert_eq!(lists.len(), 31, "BATON carries one picture per room");
    Some(lists.iter().map(|l| l.rasterise()).collect())
}

fn screenshot(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../machine-screenshots").join(name)
}

fn vic_index(rgb: Rgb, whose: &str) -> usize {
    PEPTO_PALETTE
        .iter()
        .position(|c| *c == rgb)
        .unwrap_or_else(|| panic!("{whose} holds {rgb:?}, which is not a VIC-II colour"))
}

/// A pixel counts when all eight of its neighbours carry the same stored index
/// — see the module header on why the rest cannot answer a colour question.
fn is_interior(pixels: &[u8], x: usize, y: usize) -> bool {
    if x == 0 || y == 0 || x + 1 == CANVAS_W || y + 1 == CANVAS_H {
        return false;
    }
    let v = pixels[y * CANVAS_W + x];
    (-1i32..=1).all(|dy| {
        (-1i32..=1).all(|dx| {
            pixels[(y as i32 + dy) as usize * CANVAS_W + (x as i32 + dx) as usize] == v
        })
    })
}

/// One frame's measurement: per stored index, how many eroded-interior pixels
/// the machine drew in each VIC-II colour, and the whole set of colours the
/// picture area shows.
struct Measured {
    fills: BTreeMap<u8, BTreeMap<usize, u64>>,
    all_colours: BTreeSet<usize>,
}

fn measure(pic: &scott::c64::Picture, png: &str) -> Measured {
    let img = image::open(screenshot(png)).expect("the capture opens").to_rgb8();
    assert_eq!(
        (img.width(), img.height()),
        (368, 270),
        "{png} is VICE's 368x270 frame including the border"
    );
    let mut fills: BTreeMap<u8, BTreeMap<usize, u64>> = BTreeMap::new();
    let mut all_colours = BTreeSet::new();
    for y in 0..CANVAS_H {
        for x in 0..CANVAS_W {
            let p = img.get_pixel(x as u32 + OX, y as u32 + OY).0;
            let vic = vic_index((p[0], p[1], p[2]), png);
            all_colours.insert(vic);
            if is_interior(&pic.pixels, x, y) {
                *fills.entry(pic.pixels[y * CANVAS_W + x]).or_default().entry(vic).or_default() +=
                    1;
            }
        }
    }
    Measured { fills, all_colours }
}

fn dominant(counts: &BTreeMap<usize, u64>) -> (usize, f64) {
    let total: u64 = counts.values().sum();
    let (vic, n) = counts.iter().max_by_key(|(_, n)| **n).expect("non-empty");
    (*vic, *n as f64 / total as f64)
}

/// Every stored index draws in one colour, and it is the one §8.2's remap gives.
///
/// The purity half checks the **decoder** — a fill that ran somewhere it should
/// not puts pixels in the wrong bucket and the bucket stops being pure. The
/// dominant-colour half checks the **remap**, which is the half nothing
/// internal to lanthorn could ever have made: our renderer and our tests read
/// the same table, so they agree with each other whatever the machine does.
#[test]
fn every_stored_index_draws_one_colour_and_it_is_the_one_the_remap_gives() {
    let Some(pics) = baton_pictures() else {
        assert!(skipped("the Golden Baton colour oracle"));
        return;
    };
    for frame in &FRAMES {
        let m = measure(&pics[frame.picture], frame.png);
        assert!(
            m.fills.len() >= 3,
            "{}: only {} stored indices have any interior — a frame that showed one \
             flat colour would pass every assertion below and prove nothing",
            frame.png,
            m.fills.len()
        );
        for (stored, counts) in &m.fills {
            let (vic, purity) = dominant(counts);
            let want = vic_index(PALETTE[usize::from(*stored)], "PALETTE");
            assert_eq!(
                vic, want,
                "{} (room {}): stored index {stored} draws as VIC-II {vic} on the machine \
                 and §8.2's remap A gives VIC-II {want}",
                frame.png, frame.room
            );
            assert!(
                purity >= FILL_PURITY_FLOOR,
                "{}: stored index {stored} is only {purity:.4} one colour over its interior \
                 ({counts:?}) — that is a decode question, not a palette one",
                frame.png
            );
        }
    }
}

/// Where a fill disagrees at all, it disagrees into **black** and never into
/// another fill's colour.
///
/// This is what makes the 0.4% shortfall on two of the frames a statement about
/// our flood fill's *extent* rather than about the palette: our fill reaches a
/// little further than the machine's in two places, into ground the machine
/// left unpainted. A disagreement into a different fill's colour would mean the
/// two decoders disagree about the picture, which is a different and much worse
/// bug — so it is asserted against rather than merely tolerated.
#[test]
fn only_black_is_ever_the_disagreement() {
    let Some(pics) = baton_pictures() else {
        assert!(skipped("the fill-extent shape"));
        return;
    };
    for frame in &FRAMES {
        for (stored, counts) in measure(&pics[frame.picture], frame.png).fills {
            let (vic, _) = dominant(&counts);
            for other in counts.keys().filter(|c| **c != vic) {
                assert_eq!(
                    *other, 0,
                    "{}: stored index {stored} disagrees into VIC-II {other}, not black",
                    frame.png
                );
            }
        }
    }
}

/// The line index by elimination: exactly one colour on screen is not one the
/// eroded fills account for, and it is white in all three frames.
///
/// A one-pixel line has no interior, so it cannot be read the way a fill is —
/// but it does not need to be. Subtract what the fills explain from what the
/// screen shows, and what is left has nowhere else to come from.
#[test]
fn the_line_colour_is_what_the_fills_do_not_account_for() {
    let Some(pics) = baton_pictures() else {
        assert!(skipped("the line colour"));
        return;
    };
    let want = vic_index(PALETTE[usize::from(LINE_INDEX)], "PALETTE");
    for frame in &FRAMES {
        let m = measure(&pics[frame.picture], frame.png);
        let explained: BTreeSet<usize> =
            m.fills.values().map(|c| dominant(c).0).chain(std::iter::once(0)).collect();
        let left: Vec<usize> = m.all_colours.difference(&explained).copied().collect();
        assert_eq!(
            left.len(),
            1,
            "{}: {} colours are unaccounted for ({left:?}); the elimination only works \
             while there is exactly one",
            frame.png,
            left.len()
        );
        assert_eq!(
            left[0], want,
            "{} (room {}): the line colour is VIC-II {} and §8.2's remap A gives stored \
             index {LINE_INDEX} as VIC-II {want}",
            frame.png, frame.room, left[0],
        );
    }
}

/// [`OX`] and [`OY`] are the offsets that actually maximise agreement, and each
/// frame really is the picture [`FRAMES`] names.
///
/// A frame is a fixture, and the two things a reader would otherwise take on
/// trust — where the picture sits and which picture it is — are recomputed here
/// rather than asserted from a comment. This is the guard that would have caught
/// pairing `c64-golden-3.png` with picture 2 because two moves north
/// "obviously" reaches room 3.
#[test]
fn the_alignment_is_the_measured_one() {
    let Some(pics) = baton_pictures() else {
        assert!(skipped("the alignment sweep"));
        return;
    };
    for frame in &FRAMES {
        let img = image::open(screenshot(frame.png)).expect("opens").to_rgb8();
        let score = |pic: &scott::c64::Picture, ox: u32, oy: u32| -> f64 {
            let mut groups: BTreeMap<u8, BTreeMap<[u8; 3], u64>> = BTreeMap::new();
            for y in (0..CANVAS_H).step_by(2) {
                for x in (0..CANVAS_W).step_by(2) {
                    *groups
                        .entry(pic.pixels[y * CANVAS_W + x])
                        .or_default()
                        .entry(img.get_pixel(x as u32 + ox, y as u32 + oy).0)
                        .or_default() += 1;
                }
            }
            let total: u64 = groups.values().map(|g| g.values().sum::<u64>()).sum();
            let hit: u64 = groups.values().filter_map(|g| g.values().max()).sum();
            hit as f64 / total as f64
        };
        let ours = score(&pics[frame.picture], OX, OY);

        // No nearby offset does better for this picture.
        for oy in OY - 3..=OY + 3 {
            for ox in OX - 3..=OX + 3 {
                assert!(
                    score(&pics[frame.picture], ox, oy) <= ours,
                    "{}: ({ox}, {oy}) beats the pinned ({OX}, {OY})",
                    frame.png
                );
            }
        }
        // And no other picture does as well at the pinned offset.
        for (i, pic) in pics.iter().enumerate() {
            if i == frame.picture {
                continue;
            }
            assert!(
                score(pic, OX, OY) < ours,
                "{} (pinned as picture {}, room {}) matches picture {i} at least as well",
                frame.png,
                frame.picture,
                frame.room
            );
        }
    }
}

/// The derived table, stated as a table — SQ-1491's family-B finding in raw
/// form, re-derived from the captures on every run so it cannot drift from
/// them, and a standing record of which ten indices are still owed a frame.
#[test]
fn the_stored_indices_the_baton_frames_settle() {
    let Some(pics) = baton_pictures() else {
        assert!(skipped("the derived remap rows"));
        return;
    };
    let mut derived: BTreeMap<u8, BTreeSet<usize>> = BTreeMap::new();
    for frame in &FRAMES {
        let m = measure(&pics[frame.picture], frame.png);
        for (stored, counts) in &m.fills {
            derived.entry(*stored).or_default().insert(dominant(counts).0);
        }
        let explained: BTreeSet<usize> =
            m.fills.values().map(|c| dominant(c).0).chain(std::iter::once(0)).collect();
        for left in m.all_colours.difference(&explained) {
            derived.entry(LINE_INDEX).or_default().insert(*left);
        }
    }
    let flat: BTreeMap<u8, usize> = derived
        .iter()
        .map(|(stored, set)| {
            assert_eq!(
                set.len(),
                1,
                "stored index {stored} draws as more than one colour across the frames: {set:?}"
            );
            (*stored, *set.iter().next().expect("just checked"))
        })
        .collect();
    assert_eq!(
        flat,
        BTreeMap::from([
            (0, 0), // black
            (2, 2), // red
            (4, 5), // green
            (5, 3), // cyan
            (6, 7), // gold
            (7, 1), // white, the line colour
        ]),
        "stored index -> VIC-II colour, off c64-golden-{{1,2,3}}.png. \
         Every row confirms §8.2's remap table A; none corrects it."
    );
    for stored in UNVERIFIED {
        assert!(
            !flat.contains_key(&stored),
            "stored index {stored} is listed unverified but the frames do exercise it — \
             move it out of UNVERIFIED and into the table above"
        );
    }
    assert_eq!(flat.len() + UNVERIFIED.len(), 16, "every stored index is accounted for");
}


/// The lowest whole-frame agreement any one frame may show.
///
/// Measured after the colour clash landed: 0.99053, 0.98936 and 0.99666, for
/// 0.99218 over the three together. Before it, 0.94618, 0.93191 and 0.97272.
const WHOLE_FRAME_FLOOR: f64 = 0.989;

/// The lowest agreement over the three frames together.
const WHOLE_CORPUS_FLOOR: f64 = 0.992;

/// The most pixels of one frame that may be ours-line-where-the-machine-has-
/// background: the genuine line-placement residue, after the clash explains
/// the rest. Measured 3, 16 and 18.
const LINE_RESIDUE_CEILING: usize = 25;

/// Whole-frame agreement, and the two shapes the residue is allowed to take.
///
/// This is the case the colour clash was found through, and the one that would
/// notice it being lost again. Every disagreeing pixel must be one of:
///
/// * **ours drawn, the machine's background** — our flood fill reaching a
///   little further than the machine's (the bulk), or a line pixel we place
///   and it does not (at most [`LINE_RESIDUE_CEILING`] a frame);
/// * **ours the line's colour, the machine's a fill's** — a cell whose ink the
///   two resolve differently, of which there is exactly one pixel in the whole
///   corpus.
///
/// And never the third shape: **the machine painting where we leave the
/// background**, which is asserted at zero. That is the strong half — our
/// raster's ink is a superset of the machine's, so nothing it draws is
/// missing from ours and every remaining difference is something of ours to
/// take away.
#[test]
fn the_whole_frame_agrees_but_for_the_fills_reach() {
    let Some(pics) = baton_pictures() else {
        assert!(skipped("whole-frame agreement"));
        return;
    };
    let (mut corpus_hit, mut corpus_n) = (0usize, 0usize);
    for frame in &FRAMES {
        let img = image::open(screenshot(frame.png)).expect("opens").to_rgb8();
        let pic = &pics[frame.picture];
        let background = PALETTE[usize::from(pic.background)];
        let line = PALETTE[usize::from(pic.line)];
        let (mut hit, mut line_residue, mut machine_only) = (0usize, 0usize, 0usize);
        for y in 0..CANVAS_H {
            for x in 0..CANVAS_W {
                let p = img.get_pixel(x as u32 + OX, y as u32 + OY).0;
                let (theirs, ours) = ((p[0], p[1], p[2]), PALETTE[usize::from(pic.pixels[y * CANVAS_W + x])]);
                if ours == theirs {
                    hit += 1;
                } else if ours == background {
                    machine_only += 1;
                } else if ours == line && theirs == background {
                    line_residue += 1;
                }
            }
        }
        let n = CANVAS_W * CANVAS_H;
        assert_eq!(
            machine_only, 0,
            "{}: the machine paints {machine_only} pixels we leave as background — our ink \
             is supposed to be a superset of the machine's",
            frame.png
        );
        assert!(
            line_residue <= LINE_RESIDUE_CEILING,
            "{}: {line_residue} pixels of line placement, over the ceiling of {LINE_RESIDUE_CEILING}",
            frame.png
        );
        let agreement = hit as f64 / n as f64;
        assert!(
            agreement >= WHOLE_FRAME_FLOOR,
            "{}: whole-frame agreement {agreement:.5} is under {WHOLE_FRAME_FLOOR}",
            frame.png
        );
        corpus_hit += hit;
        corpus_n += n;
    }
    let corpus = corpus_hit as f64 / corpus_n as f64;
    assert!(
        corpus >= WHOLE_CORPUS_FLOOR,
        "the three frames together agree {corpus:.5}, under {WHOLE_CORPUS_FLOOR}"
    );
}

/// No 8 x 8 cell of any frame holds three colours — and only one of the eight
/// vertical phases can say so.
///
/// This is the evidence for `scott::c64`'s cell grid, re-derived rather than
/// restated: a high-resolution bitmap has one ink and one paper per cell and
/// physically cannot show a third colour in one, so the phase at which that
/// holds is the phase the machine uses. Canvas row 7 begins a cell row; the
/// other seven phases leave 12 to 43 cells of 341 holding three colours.
///
/// Horizontally there is nothing to find — canvas column 0 begins a cell —
/// and the case asserts that too, since it is an assumption either way.
#[test]
fn the_machine_never_shows_three_colours_in_a_cell() {
    const PHASE: u32 = 7;
    for frame in &FRAMES {
        let img = image::open(screenshot(frame.png)).expect("opens").to_rgb8();
        let over = |phase: u32| -> usize {
            let mut over = 0;
            for cy in 0..11u32 {
                for cx in 0..31u32 {
                    let mut set = BTreeSet::new();
                    for dy in 0..8u32 {
                        for dx in 0..8u32 {
                            let fy = OY + phase + cy * 8 + dy;
                            if fy >= OY + CANVAS_H as u32 {
                                continue;
                            }
                            let p = img.get_pixel(OX + cx * 8 + dx, fy).0;
                            set.insert((p[0], p[1], p[2]));
                        }
                    }
                    if set.len() > 2 {
                        over += 1;
                    }
                }
            }
            over
        };
        assert_eq!(
            over(PHASE),
            0,
            "{}: a cell holds three colours at the phase the decoder uses",
            frame.png
        );
        for phase in 0..8 {
            if phase == PHASE {
                continue;
            }
            assert!(
                over(phase) > 0,
                "{}: phase {phase} also holds to two colours a cell, so the frames do not \
                 pin the grid after all",
                frame.png
            );
        }
    }
}
