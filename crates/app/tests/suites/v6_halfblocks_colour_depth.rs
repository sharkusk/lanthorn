//! SQ-0977: half-blocks does not put the ARTWORK's colours on the wire, it puts the
//! RESAMPLE's, and there are thousands of them.
//!
//! The proposal this suite closes was to spend fewer bytes per cell. Half-blocks emits
//! a truecolor foreground *and* background for every style change —
//! `ESC[38;2;R;G;B;48;2;R;G;Bm` is ~30 bytes where the indexed form
//! `ESC[38;5;N;48;5;Nm` is ~18 — and Infocom v6 artwork is sixteen colours per picture,
//! so the content looked like a natural fit for an indexed palette, exactly or via
//! OSC 4 programming a reserved range of terminal palette entries.
//!
//! The premise about the artwork is true, and structurally so: a decoded
//! [`blorb::infocom_pics::Picture`] is one palette INDEX per pixel resolved through a
//! `[Rgb; 16]`, so sixteen is a ceiling no picture can exceed whichever table it is read
//! through. `an_infocom_picture_is_at_most_sixteen_colours` measures it on every picture
//! in every native archive in `stories/`.
//!
//! The premise about the wire is false, and this is why the work was not done. What
//! half-blocks emits is not the canvas: `Halfblocks::encode` resolves one sample per
//! COLUMN and two per ROW, so the composite is resampled onto that sample grid first
//! (SQ-0973), and [`resize_directional`] filters by the direction each axis moves —
//! `Nearest` when it grows, `Triangle` when it shrinks. Aspect is preserved, so both
//! axes always travel together and the whole thing turns on ONE comparison: the sample
//! grid against the canvas. Below it every cell colour is a blend of neighbours that
//! was never in the picture, and a fourteen-colour illustration reaches the terminal as
//! several thousand distinct colours.
//!
//! ## Measured on the wire, not only here
//!
//! `cargo run -p app --example pty_capture -- --arg --image-protocol --arg halfblocks`,
//! counting truecolor triplets in the raw capture:
//!
//! | story | pane | full-repaint flush | SGR share | distinct colours |
//! |---|---|---|---|---|
//! | `zork0-r393-s890714.z6` | 200x100 cells @ 4x9 px | 180 KB total | 84.7% | 1,083 |
//! | `zork0-r393-s890714.z6` | 458x144 cells @ 4x9 px | 489 KB | 81.7% | 1,419 |
//! | `zork0-r393-s890714.z6` | 700x220 cells @ 4x9 px | 936 KB total | 78.3% | 1,712 |
//! | `journey-r83-s890706.z6` | 117x64 cells @ 8x18 px | 150 KB total | 82.8% | 4,746 |
//!
//! Sixteen entries cannot hold that, and neither can the whole 6x6x6 cube. Mapping it
//! into the standard cube instead costs a mean Euclidean RGB error of 21–26 with only
//! 5–6% of emissions landing exactly, which is visible posterisation on the dithered
//! art the saving would be concentrated in. So neither half was built: not OSC 4, whose
//! reserved range would never be big enough, and not indexed SGR alone, which trades a
//! ~30% stream reduction for a quality regression nobody asked for.
//!
//! ## Which palette is which
//!
//! Three different things are called a palette around this code and none of them is
//! the other. This file is about the **picture file's** table — the `[Rgb; 16]` a
//! `.mg1` record carries, or the hardware table an EGA/CGA archive implies. It is not
//! the **machine** palette a session carries (`zvm::screen::Palette`, a
//! `Machine` field since SQ-1393): nothing here boots a story or resolves a
//! z-colour number, which is why no session appears below. And it is not the
//! **terminal's** 256-entry palette that
//! OSC 4 would have programmed, which does not enter this file at all.
//!
//! ## Fixtures
//!
//! `stories/` is gitignored (CLAUDE.md), so every case skips vacuously without the
//! archives — with the `!any_present || seen > 0` shape, so an archive that IS present
//! and measures nothing still fails. The archives are native Infocom picture files,
//! whose flavour and entry count each case prints; they carry no release header of
//! their own, so the flavour is the identifying fact.
//!
//! FALSIFY by making [`resize_directional`] pick `Nearest` unconditionally:
//! `the_sample_grid_a_real_pane_asks_for_does_not_keep_them` fails on every archive,
//! because the minifying grids then preserve the picture's fourteen colours too and the
//! contrast the whole finding rests on disappears.

use std::collections::HashSet;
use std::path::PathBuf;

use app::render::graphics::{resize_directional, v6_halfblocks_grid};
use blorb::infocom_pics::InfocomPics;
use ratatui_image::picker::Picker;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// The native picture archives in `stories/`, one per graphical v6 title. `zorkzero.mg1`
/// is deliberately absent: it is byte-for-byte the same archive as `zork0.mg1` and would
/// only double the running time.
const ARCHIVES: [&str; 4] = ["zork0.mg1", "shogun.mg1", "journey.mg1", "arthur.mg1"];

/// A parsed archive, with what it is printed — an archive whose flavour is not stated is
/// a fixture without a machine (CLAUDE.md).
fn archive(name: &str) -> Option<InfocomPics> {
    let path = stories_dir().join(name);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored picture archive missing at {}", path.display());
        return None;
    };
    match InfocomPics::parse(bytes) {
        Ok(p) => {
            eprintln!(
                "{name}: flavour {:?}, {} entries, picture space {}x{}",
                p.flavour(),
                p.entries().len(),
                p.picture_space_width(),
                p.picture_space_height(),
            );
            Some(p)
        }
        Err(e) => panic!("{name} is a native picture archive and must parse: {e:?}"),
    }
}

/// How many distinct RGBA values an image carries.
fn colours(img: &image::RgbaImage) -> usize {
    img.pixels().map(|p| p.0).collect::<HashSet<[u8; 4]>>().len()
}

/// The RICHEST picture in an archive — most distinct colours, ties broken by area,
/// among entries big enough to be an illustration rather than an icon — at the extent
/// the unit screen composes it.
///
/// Richest rather than largest, because the largest entry in a native archive is often
/// a flat six-colour backdrop and the finding is about ARTWORK: a picture whose colour
/// count is near the sixteen-entry ceiling is the strongest case an indexed encoding
/// could possibly have had.
///
/// A 320x200 press is laid out on a 640x400 screen at `art_scale` (2, 2), and that
/// magnification is a `Nearest` one, so the canvas the backend is handed still carries
/// exactly the picture's own colours — which the case below asserts before it measures
/// anything, because the contrast is only meaningful if the canvas start point is clean.
fn richest_picture_on_the_unit_screen(pics: &InfocomPics) -> Option<(u16, usize, image::RgbaImage)> {
    let scale = u32::from(640 / pics.picture_space_width().max(1)).max(1);
    let mut best: Option<(u16, usize, u32, image::RgbaImage)> = None;
    for e in pics.entries() {
        if !e.has_pixels() || e.width < 200 || e.height < 100 {
            continue;
        }
        let Ok(pic) = pics.decode(e.id) else { continue };
        let Some(img) =
            image::RgbaImage::from_raw(u32::from(pic.width), u32::from(pic.height), pic.rgba())
        else {
            continue;
        };
        let rank = (colours(&img), img.width() * img.height());
        if best.as_ref().is_none_or(|&(_, n, area, _)| rank > (n, area)) {
            let up = resize_directional(&img, img.width() * scale, img.height() * scale);
            best = Some((e.id, rank.0, rank.1, up));
        }
    }
    best.map(|(id, n, _, img)| (id, n, img))
}

/// The half-block sample grid a `cols x rows` pane resolves this canvas onto: one sample
/// per column and two per row, the cell rect coming from the shipped arithmetic rather
/// than from a restatement of it.
fn sample_grid(canvas: &image::RgbaImage, cols: u16, rows: u16) -> (u32, u32) {
    let fs = Picker::halfblocks().font_size();
    let (box_w, box_h) =
        (u32::from(cols) * u32::from(fs.width), u32::from(rows) * u32::from(fs.height));
    let cells = v6_halfblocks_grid(canvas.dimensions(), box_w, box_h, fs, None);
    (u32::from(cells.width), u32::from(cells.height) * 2)
}

/// The premise, and it holds: no Infocom picture carries more than sixteen colours.
///
/// Structural rather than lucky — `Picture::rgba_with` indexes a `[Rgb; 16]` with
/// `i & 15`, so sixteen is a ceiling however the table is chosen — and 640x400 Amiga
/// hires is four bitplanes, which is where the ceiling comes from. The measurement is
/// here anyway because the whole finding below is a contrast against this number, and a
/// contrast with an assumed half is not a measurement.
#[test]
fn an_infocom_picture_is_at_most_sixteen_colours() {
    let (mut any_present, mut seen) = (false, 0usize);
    for name in ARCHIVES {
        let Some(pics) = archive(name) else { continue };
        any_present = true;
        let (mut worst, mut counted) = (0usize, 0usize);
        for e in pics.entries() {
            if !e.has_pixels() {
                continue;
            }
            let Ok(pic) = pics.decode(e.id) else { continue };
            let Some(img) =
                image::RgbaImage::from_raw(u32::from(pic.width), u32::from(pic.height), pic.rgba())
            else {
                continue;
            };
            let n = colours(&img);
            assert!(
                n <= 16,
                "{name} picture {} is {n} colours — a decoded picture is one palette index \
                 per pixel through a [Rgb; 16], so this cannot happen without the decoder \
                 having changed shape",
                e.id,
            );
            worst = worst.max(n);
            counted += 1;
        }
        eprintln!("{name}: {counted} pictures decoded, richest carries {worst} colours");
        assert!(
            counted > 20,
            "{name}: only {counted} pictures decoded — an archive that yields almost nothing \
             would pass this case vacuously",
        );
        assert!(
            worst >= 4,
            "{name}: the richest picture carries {worst} colours, which is not artwork — \
             something is decoding to a flat fill",
        );
        seen += 1;
    }
    assert!(!any_present || seen > 0, "a present fixture must have been measured");
}

/// And the sample grid a real pane asks for keeps none of them.
///
/// Three panes, all of which MINIFY the unit screen, and one that does not. The three
/// are ordinary: 117x64 is a 936x1152 window at 8x18 cells, 200x60 a 1920x1200 one at a
/// small font, and 458x144 the fine grid SQ-0964 was reported against. On every one of
/// them a fourteen-colour illustration reaches the terminal as several hundred to
/// several thousand distinct colours, which is more than the 240 usable entries of the
/// standard 256-colour palette — so there is no reserved range for OSC 4 to program and
/// no exact indexed encoding to be had.
///
/// The fourth pane is the boundary, and it is what makes the rule legible rather than
/// anecdotal. The grid is one sample per COLUMN, so it stops shrinking only once the
/// pane has as many columns as the canvas has pixels across — **640 for a full v6
/// screen** — and there the colour count returns exactly to the picture's own. That is
/// the terminal width the indexed idea would need, and it is not a width anybody has.
#[test]
fn the_sample_grid_a_real_pane_asks_for_does_not_keep_them() {
    let (mut any_present, mut seen) = (false, 0usize);
    for name in ARCHIVES {
        let Some(pics) = archive(name) else { continue };
        any_present = true;
        let (id, native, canvas) =
            richest_picture_on_the_unit_screen(&pics).expect("an archive holds an illustration");
        assert!(
            (4..=16).contains(&native),
            "{name}: the unit-screen canvas of picture {id} carries {native} colours — the \
             art_scale magnification is Nearest and must not have invented any",
        );

        for (cols, rows) in [(117u16, 64u16), (200, 60), (458, 144)] {
            let (gw, gh) = sample_grid(&canvas, cols, rows);
            assert!(
                gw < canvas.width(),
                "{name}: at {cols}x{rows} the sample grid is {gw}x{gh} against a \
                 {}x{} canvas — this case is only about the grids that SHRINK it",
                canvas.width(),
                canvas.height(),
            );
            let n = colours(&resize_directional(&canvas, gw, gh));
            eprintln!(
                "{name} pic {id}: canvas {}x{} ({native} colours) -> {cols}x{rows} pane, \
                 sample grid {gw}x{gh}: {n} colours",
                canvas.width(),
                canvas.height(),
            );
            assert!(
                n > 256,
                "{name}: at {cols}x{rows} the grid emits {n} distinct colours from {native} in \
                 the artwork. The point of this case is that the number is far past anything \
                 an indexed palette can hold; at {n} it no longer is, so the SQ-0977 finding \
                 needs re-measuring rather than this threshold needs relaxing",
            );
        }

        // The boundary: a grid at least as large as the canvas filters Nearest, and
        // Nearest invents nothing.
        let wide = 640u16;
        let (gw, gh) = sample_grid(&canvas, wide, 200);
        assert!(
            gw >= canvas.width() && gh >= canvas.height(),
            "{name}: a {wide}-column pane should stop the grid shrinking, but it is {gw}x{gh} \
             against a {}x{} canvas",
            canvas.width(),
            canvas.height(),
        );
        assert_eq!(
            colours(&resize_directional(&canvas, gw, gh)),
            native,
            "{name}: at {wide}x200 the sample grid is {gw}x{gh} and no longer minifies, so the \
             picture's own {native} colours must survive intact — that is the ONLY pane on \
             which an indexed encoding of this artwork would be exact",
        );
        seen += 1;
    }
    assert!(!any_present || seen > 0, "a present fixture must have been measured");
}
