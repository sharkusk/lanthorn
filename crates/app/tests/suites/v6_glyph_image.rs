//! SQ-1569 — `TextFace::glyph_image` and `TextFace::repertoire`: the exact pixels
//! the v6 raster path draws for one glyph, and the set of glyphs it can draw.
//!
//! # The claim, and why it is tested against the blit itself
//!
//! A host that draws v6 text with its own font machinery needs the pixels
//! `render::bitfont::blit_glyph_styled` would paint, and those come out of a chain
//! of decisions: the release or system face at its own text scale (SQ-1009,
//! SQ-1053), the fixed-pitch alternate (SQ-1036), `misc7x14` in a 7-wide cell
//! (SQ-1016), `vga16`, the 8×8 masters and their tiling columns (SQ-1027), and
//! synthesised bold and italic — or the rule a machine draws instead of italic
//! (SQ-1028). So every case here stamps `glyph_image` at the pen and compares the
//! result with `blit_glyph_styled` drawing the same string directly, pixel for
//! pixel, in all sixteen §8.7.1 style bytes (reverse 1, bold 2, italic 4, fixed
//! pitch 8 — bit 8 changes the FACE on a machine with an alternate, so it is not
//! a no-op to skip) and with and without a painted background.
//!
//! # Faces
//!
//! | face | source | cell | scale |
//! |---|---|---|---|
//! | Arthur's Amiga `char.data`, 10×10 proportional | `stories/Arthur - The Quest for Excalibur.adf` (gitignored; skips) | 8×20 | (2, 2) |
//! | Macintosh `FONT` 524 alone | `unit_tests/relfont.hfs` | 7×15 | (1, 1) |
//! | `FONT` 524 as the alternate, synthetic Geneva as body | `relfont.hfs` + `unit_tests/sysfont.hfs` | 7×15 | (1, 1) |
//! | the real `FONT` 524 | `stories/Zork Zero Disk.image` (gitignored; skips) | 7×15 | (1, 1) |
//! | topaz 8, 8×8 fixed | `unit_tests/kickfont.rom` (synthetic Kickstart) | 8×16 | (1, 2) |
//! | `TextFace::cell_only` | none | 8×16 and 7×15 | (1, 1) |
//!
//! The boot media are copied into an `app::scratch_dir` standing in for
//! `~/.lanthorn/`, exactly as `system_face_cascade` and `amiga_rom_face` do.

use app::interpreter::InterpreterProfile as P;
use app::native_font::{FaceFit, FaceRequest, FaceSet, TextFace};
use app::system_fonts::UserDisks;
use image::{Rgba, RgbaImage};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Letters (ascenders, descenders, accents), punctuation, box drawing that must
/// TILE (corners, tees, the one-eighth bars whose only ink is an edge column) and
/// block shades — so the text faces, the fallback masters and the tiling column
/// map are all on the page at once.
const SAMPLE: &str = "Quick fog, jumpy WAX! \u{250C}\u{2500}\u{252C}\u{2510}\u{2502}\u{251C}\u{253C}\u{2524}\u{2514}\u{2534}\u{2518} \u{2595}\u{258F}\u{2504}\u{2591}\u{2592}\u{2588} g\u{00FF}\u{00C9}\u{0153} \u{2395}\u{2190}";

const INK: Rgba<u8> = Rgba([255, 255, 255, 255]);
const PAGE: Rgba<u8> = Rgba([10, 20, 30, 255]);
const CLEAR: Rgba<u8> = Rgba([0, 0, 0, 0]);

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A scratch directory standing in for `~/.lanthorn/`.
struct Disks {
    dir: PathBuf,
}

impl Disks {
    fn new(tag: &str) -> Disks {
        Disks { dir: app::scratch_dir(&format!("sq1569-{tag}")) }
    }

    fn with(self, name: &str, fixture: &str) -> Disks {
        let bytes = std::fs::read(root().join("unit_tests").join(fixture))
            .unwrap_or_else(|e| panic!("unit_tests/{fixture} is committed and readable: {e}"));
        std::fs::write(self.dir.join(name), bytes).expect("write fixture");
        self
    }

    fn disks(&self) -> UserDisks {
        UserDisks { dir: self.dir.clone(), prefer: None }
    }
}

impl Drop for Disks {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn cascade(story: &Path, profile: P, art_scale: Option<(u32, u32)>, disks: Option<&UserDisks>) -> FaceSet {
    app::native_font::resolve(&FaceRequest {
        story_path: story,
        entry: None,
        profile,
        source: app::interpreter::ProfileSource::Medium,
        art_scale,
        disks,
    })
}

/// The first pixel two canvases disagree on, for a failure that says where.
fn first_difference(a: &RgbaImage, b: &RgbaImage) -> Option<(u32, u32, Rgba<u8>, Rgba<u8>)> {
    a.enumerate_pixels().find(|(x, y, p)| *p != b.get_pixel(*x, *y)).map(|(x, y, p)| (x, y, *p, *b.get_pixel(x, y)))
}

/// **Stamping `glyph_image` at the pen IS the composite**, for `SAMPLE` in every
/// style byte, transparent and on a painted page.
fn stamping_reproduces_the_blit(tf: &TextFace, label: &str) {
    let (cw, chh) = (u32::from(tf.cell().w()), u32::from(tf.cell().h()));
    let repertoire: BTreeSet<char> = tf.repertoire().into_iter().collect();
    for c in SAMPLE.chars() {
        assert!(repertoire.contains(&c), "{label}: sample {c:?} (U+{:04X}) must be drawable", c as u32);
    }
    for style in 0u8..16 {
        for bg in [None, Some(PAGE)] {
            // Slack past the last pen position for a glyph whose footprint
            // overhangs its advance (a smeared bold on a scaled fixed face).
            let total: u32 = SAMPLE.chars().map(|c| tf.advance_styled(c, style)).sum();
            let w = total + 4 * cw;
            let mut direct = RgbaImage::from_pixel(w, chh, CLEAR);
            let mut stamped = RgbaImage::from_pixel(w, chh, CLEAR);
            let mut pen = 0u32;
            for c in SAMPLE.chars() {
                app::render::bitfont::blit_glyph_styled(&mut direct, c, pen, 0, cw, chh, INK, bg, style, Some(tf));
                let img = tf.glyph_image(c, style).unwrap_or_else(|| panic!("{label}: {c:?} style {style} draws"));
                assert_eq!(img.advance, tf.advance_styled(c, style), "{label}: {c:?} style {style} advance");
                assert_eq!(img.height, chh, "{label}: {c:?} style {style} is one cell tall");
                assert_eq!(img.bits.len(), img.row_bytes() * img.height as usize, "{label}: {c:?} bit buffer size");
                for y in 0..img.height {
                    for x in 0..img.width {
                        if pen + x >= w {
                            continue;
                        }
                        if img.ink(x, y) {
                            stamped.put_pixel(pen + x, y, INK);
                        } else if let Some(b) = bg {
                            stamped.put_pixel(pen + x, y, b);
                        }
                    }
                }
                pen += img.advance;
            }
            assert!(direct.pixels().any(|p| *p == INK), "{label}: non-vacuity — style {style} inked something");
            assert!(
                first_difference(&direct, &stamped).is_none(),
                "{label}: style {style} bg {bg:?} — stamped glyph images differ from the blit at {:?}",
                first_difference(&direct, &stamped),
            );
        }
    }
}

/// **`repertoire` is exactly the set `glyph_image` answers `Some` for**, in every
/// style byte, and every `Some` reports the pen's own advance.
///
/// The universe covers every table the chain consults — Basic Latin through the
/// block elements, the APL quad and cursor arrows, the runes, U+FFFD and the
/// Legacy Computing diagonals — plus anything `repertoire` itself names, so a code
/// outside the probe range cannot hide.
fn repertoire_is_exactly_what_draws(tf: &TextFace, label: &str) -> usize {
    let rep = tf.repertoire();
    let set: BTreeSet<char> = rep.iter().copied().collect();
    assert_eq!(set.len(), rep.len(), "{label}: repertoire has no duplicates");
    assert!(rep.windows(2).all(|w| w[0] < w[1]), "{label}: repertoire is sorted");
    let universe: BTreeSet<char> = (0u32..=0x33FF)
        .chain(0xFB00..=0xFFFF)
        .chain(0x1FB00..=0x1FBFF)
        .filter_map(char::from_u32)
        .chain(rep.iter().copied())
        .collect();
    for style in 0u8..16 {
        let mut drawn = BTreeSet::new();
        for &c in &universe {
            if let Some(img) = tf.glyph_image(c, style) {
                assert_eq!(img.advance, tf.advance_styled(c, style), "{label}: {c:?} style {style} advance");
                drawn.insert(c);
            }
        }
        assert_eq!(
            drawn,
            set,
            "{label}: style {style} — drawn but not in repertoire: {:?}; in repertoire but not drawn: {:?}",
            drawn.difference(&set).collect::<Vec<_>>(),
            set.difference(&drawn).collect::<Vec<_>>(),
        );
    }
    rep.len()
}

fn check(tf: &TextFace, label: &str) {
    stamping_reproduces_the_blit(tf, label);
    let n = repertoire_is_exactly_what_draws(tf, label);
    eprintln!("{label}: cell {:?} scale {:?} — {n} characters, 16 styles x 2 backgrounds match", tf.cell(), tf.scale());
}

// ── the faces ───────────────────────────────────────────────────────────────

/// Arthur's own `char.data`, release 54 / serial 890606, resolved at launch (no
/// turns — the cascade reads the medium, not the screen). A PROPORTIONAL face at
/// (2, 2), so letters take the scaled blit and box drawing falls through to the
/// masters in the 8×20 cell.
#[test]
fn arthurs_amiga_face() {
    let path = root().join("stories/Arthur - The Quest for Excalibur.adf");
    if !path.is_file() {
        eprintln!("SKIP: gitignored floppy absent at {}", path.display());
        return;
    }
    let (profile, source) = P::resolve_with_source(&path, None, None, None);
    assert_eq!(profile, P::Amiga, "non-vacuity: the medium names the Amiga");
    let faces = app::native_font::resolve(&FaceRequest {
        story_path: &path,
        entry: None,
        profile,
        source,
        art_scale: Some((2, 2)),
        disks: None,
    });
    assert_eq!(faces.body().map(|f| (f.width, f.height)), Some((10, 10)), "non-vacuity: char.data");
    let tf = TextFace::new(profile, faces, Some((2, 2)));
    assert!(tf.proportional() && tf.scale() == (2, 2), "non-vacuity: a typeface at the release scale");
    assert!(tf.underlines_emphasis(), "non-vacuity: the Amiga rules under italic");
    check(&tf, "Arthur char.data");
}

/// `FONT` 524 with no System disk: it is the body AND the alternate, a `Cell`
/// face stamped 1:1 into the 7×15 cell.
#[test]
fn macintosh_font_524_alone() {
    let faces = cascade(&root().join("unit_tests/relfont.hfs"), P::Macintosh, None, None);
    assert_eq!(faces.body().map(|f| (f.width, f.height)), Some((7, 15)), "non-vacuity: FONT 524");
    assert_eq!(faces.body(), faces.fixed(), "non-vacuity: in both roles");
    let tf = TextFace::new(P::Macintosh, faces, None);
    assert_eq!((tf.cell().w(), tf.cell().h()), (7, 15));
    assert_eq!(tf.fit(), Some(FaceFit::Cell));
    assert!(!tf.draws_scaled(0), "non-vacuity: the cell path, not the scaled one");
    check(&tf, "FONT 524 alone");
}

/// `FONT` 524 as the fixed-pitch alternate under a System disk's proportional
/// body face — so bit 8 of the style byte switches faces mid-string.
#[test]
fn macintosh_font_524_under_a_system_face() {
    let boot = Disks::new("sys").with("System.img", "sysfont.hfs");
    let faces = cascade(&root().join("unit_tests/relfont.hfs"), P::Macintosh, None, Some(&boot.disks()));
    assert_eq!(faces.body().map(|f| (f.width, f.height)), Some((9, 15)), "non-vacuity: the System face is body");
    assert_eq!(faces.fixed().map(|f| (f.width, f.height)), Some((7, 15)), "non-vacuity: FONT 524 is the alternate");
    let tf = TextFace::new(P::Macintosh, faces, None);
    assert!(tf.draws_proportionally(0) && !tf.draws_proportionally(8), "non-vacuity: bit 8 changes the pen");
    check(&tf, "FONT 524 + System face");
}

/// The real `FONT` 524 off *Zork Zero*'s Macintosh platter, release 296 / serial
/// 881019 — the synthetic fixture's coverage is not the resource's.
#[test]
fn macintosh_font_524_off_the_real_disk() {
    let path = root().join("stories/Zork Zero Disk.image");
    if !path.is_file() {
        eprintln!("SKIP: gitignored Macintosh medium absent");
        return;
    }
    let (profile, source) = P::resolve_with_source(&path, None, None, None);
    assert_eq!(profile, P::Macintosh, "non-vacuity: the medium names the Macintosh");
    let faces = app::native_font::resolve(&FaceRequest {
        story_path: &path,
        entry: None,
        profile,
        source,
        art_scale: None,
        disks: None,
    });
    assert_eq!(faces.body().map(|f| (f.width, f.height)), Some((7, 15)), "non-vacuity: FONT 524");
    let tf = TextFace::new(profile, faces, None);
    check(&tf, "Zork Zero FONT 524");
}

/// Topaz 8 out of a (synthetic) Kickstart: an 8×8 FIXED face drawn at the hires
/// system scale (1, 2) — the scaled blit, every face row twice.
#[test]
fn amiga_rom_topaz() {
    let media = Disks::new("kick").with("Kick12.rom", "kickfont.rom");
    let faces = cascade(Path::new("/nonexistent.z6"), P::Amiga, Some((2, 2)), Some(&media.disks()));
    assert_eq!(faces.body().map(|f| (f.width, f.height)), Some((8, 8)), "non-vacuity: topaz 8");
    let tf = TextFace::new(P::Amiga, faces, Some((2, 2)));
    assert_eq!(tf.scale(), (1, 2), "non-vacuity: the system face's hires scale");
    assert!(tf.draws_scaled(0) && !tf.proportional(), "non-vacuity: a fixed face through the scaled blit");
    check(&tf, "topaz 8");
}

/// No face at all, at both cells a v6 machine declares: `vga16` at 8×16, and at
/// 7×15 `misc7x14` with the tiling endpoint map for box drawing.
#[test]
fn cell_only_faces() {
    for (w, h) in [(8u16, 16u16), (7, 15)] {
        let tf = TextFace::cell_only(zvm::screen::V6Cell::new(w, h));
        assert!(tf.face().is_none(), "non-vacuity: no face");
        check(&tf, &format!("cell_only {w}x{h}"));
    }
}
