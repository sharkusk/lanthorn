//! SQ-0841 — the **Macintosh monochrome pillars** land their feet on the bottom,
//! and their bands repeat at a uniform stride however tall the pane is.
//!
//! Fixture: `stories/Zork Zero Disk.image` — **Zork Zero v6 release 296, serial
//! 881019** — with `--pictures Pic.data`, the standard Mac's two-colour archive.
//! `stories/` is gitignored, so every case here skips vacuously without it.
//!
//! ## What was wrong
//!
//! A v6 screen is the archive's picture space rounded UP to a whole 8x16 cell.
//! The mono archive's space is 480x300, so the screen is **304** rows and the
//! pillars — painted to row 300 — stopped four pixels short of it. `recognize`
//! asked whether the art reached the native bottom EXACTLY, so it read a pillar
//! as Shogun's single-piece slab, and `shogun()` extends a slab by stamping a
//! MIRRORED copy of the whole thing below the first. On screen that reads as a
//! doubled foot at the art's own bottom followed by the mirror's shaft and, at a
//! tall enough pane, a second capital: *bare shaft below the foot*, which is
//! what the user reported.
//!
//! ## What replaces it
//!
//! Recognised as a pillar, the column now takes the Zork Zero path. Its shaft is
//! not the plain constant span `pillar_shaft` looks for — a ring interrupts it —
//! so `banded_shaft` measures it instead and `extend_banded_pillars` composes
//! it: the repeat unit is everything BELOW the capital, foot included, and `k`
//! further copies are laid at a stride dividing the extension exactly, so the
//! last copy's foot lands on the bottom row.
//!
//! ## What is asserted
//!
//! 1. **The foot is the bottom-most ink** — no shaft under it, at any pane.
//! 2. **The bands are uniformly spaced**, to the row, and the run's last gap is
//!    the art's own capital-to-band distance rather than a remainder.
//! 3. **The band count is derived from the span** and grows with it; at the
//!    120x40 the user judged it, it does not regress.
//! 4. **The COLOUR rendition on the same disk is byte-for-byte unmoved** — its
//!    shaft is featureless, `pillar_shaft` measures it, and it keeps Bocfel's
//!    `extend_pillars` untouched.
//!
//! Both `honor_game_colours` modes, per the project's colour convention: the
//! flank source is composed from the graphics canvas either way, and a mode that
//! hands the page back to the theme must not move the art.

use app::engine::Engine;
use app::graphics::{PictSource, PictureOverride};
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};
use std::path::PathBuf;

const RELEASE: u16 = 296;
const SERIAL: &[u8] = b"881019";

fn mac_disk() -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/Zork Zero Disk.image");
    if !path.exists() {
        eprintln!("SKIP: gitignored Macintosh medium missing at {}", path.display());
        return None;
    }
    Some(path)
}

/// Boot the disk the way `startup.rs` does, optionally naming an archive.
fn launch(pictures: Option<&str>, honor: bool) -> GameSession {
    let path = mac_disk().expect("caller checked the fixture is present");
    let bytes = match app::hints::load_story(&path).expect("Story.data mounts") {
        app::hints::LoadedStory::ZCode(b) => b,
        other => panic!("expected Z-code, got {other:?}"),
    };
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), RELEASE, "this disk carries r{RELEASE}");
    assert_eq!(&bytes[0x12..0x18], SERIAL);

    let dir = app::scratch_dir("mac-pillar");
    let over = match pictures {
        Some(name) => PictureOverride::resolve_with_session(&path, &dir, Some(name)),
        None => PictureOverride::Unset,
    };
    let named = over.std_window();
    let profile = InterpreterProfile::resolve(&path, None, over.flavour(), None);
    let mut picts = PictSource::resolve_with_override(&path, over, None);
    let dims = picts.all_pict_dims();
    // SQ-1021/SQ-1022: every per-machine fact in one value, so this
    // harness cannot omit one — it was omitting the CELL.
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        named,
        profile.interpreter_number(),
        honor.then(|| profile.default_colours()).flatten(),
        true,
        app::native_font::FaceSet::none(),
        profile.palette(),
        None,
    );
    let mut s = GameSession::new_for_machine(bytes, honor, false, false, dims, None, None, &boot)
    .expect("Zork Zero boots off the Macintosh disk");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    for _ in 0..12 {
        match s.pending_input() {
            InputKind::Line => {
                s.submit("");
            }
            InputKind::Char => {
                let _ = s.submit_char(13);
            }
            InputKind::Event => {
                s.submit("");
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    s
}

/// One flank of the frame, as `screen.rs` hands it to `flank_source`.
struct Flank {
    x0: u32,
    x1: u32,
    art: (u32, u32),
    native_h: u32,
    gfx: image::RgbaImage,
}

/// The left and right flanks of the border, from a gameplay frame.
fn flanks(s: &GameSession) -> Vec<Flank> {
    let model = s.screen();
    let app::engine::WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let gfx = app::render::v6_layout::build_graphics_canvas(&layout.chrome, native);
    let story = layout.story.expect("a story window");
    [(0u32, story.x_px as u32), ((story.x_px + story.w_px) as u32, gfx.width())]
        .into_iter()
        .map(|(x0, x1)| Flank {
            x0,
            x1,
            art: app::render::v6_border::art_extent(&gfx, x0, x1),
            native_h: native.1 as u32,
            gfx: gfx.clone(),
        })
        .collect()
}

/// The opaque column span of every row of `img`, or `None` where nothing is
/// painted — the shape of a column reduced to one number per row.
fn spans(img: &image::RgbaImage) -> Vec<Option<(u32, u32)>> {
    (0..img.height())
        .map(|y| {
            let mut first = None;
            let mut last = 0;
            for x in 0..img.width() {
                if img.get_pixel(x, y)[3] >= 128 {
                    first.get_or_insert(x);
                    last = x;
                }
            }
            first.map(|f| (f, last))
        })
        .collect()
}

/// The last row of `img` carrying any ink.
fn last_ink(img: &image::RgbaImage) -> Option<u32> {
    spans(img).iter().rposition(|s| s.is_some()).map(|y| y as u32)
}

/// The rows where a **band** sits: the shaft's span is the modal one, so a band
/// is a cluster of rows that are not it. Each band is reported by its first row.
///
/// `inner` is the column between its capital and its foot, whose flares would
/// otherwise count as bands. The clustering matters: the Macintosh ring is TWO
/// thin rules three rows apart (native 163..166 and 169..173 on the left flank),
/// which reads as one band on screen and must be counted as one here.
fn bands(img: &image::RgbaImage, inner: (u32, u32)) -> Vec<u32> {
    /// Plain rows this many apart or fewer keep one band together.
    const KNIT: u32 = 8;
    let sp = spans(img);
    let mut tally: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
    for s in sp[inner.0 as usize..inner.1 as usize].iter().flatten() {
        *tally.entry(*s).or_default() += 1;
    }
    let plain = *tally.iter().max_by_key(|(_, n)| **n).expect("the shaft has rows").0;
    let mut out: Vec<u32> = Vec::new();
    let mut last_odd: Option<u32> = None;
    for y in inner.0..inner.1 {
        if sp[y as usize] == Some(plain) {
            continue;
        }
        if last_odd.is_none_or(|p| y - p > KNIT) {
            out.push(y);
        }
        last_odd = Some(y);
    }
    out
}

/// The pane heights swept. Deliberately not multiples of each other, and 120x40
/// first because that is the render the user judged.
const PANES: &[(u16, u16)] = &[(120, 40), (100, 30), (117, 64), (91, 51), (120, 90)];

/// The native band each pane asks the flank for, as `screen.rs` computes it:
/// `crop_top + rows` off the render's own band log.
fn desired_heights(s: &GameSession, panes: &[(u16, u16)]) -> Vec<u32> {
    let model = s.screen();
    panes
        .iter()
        .map(|&(w, h)| {
            let mut st = app::state::AppState::default();
            st.colors = app::colors::ColorScheme::terminal_default();
            // `from_fontsize`: a headless test has no terminal to query.
            #[allow(deprecated)]
            let picker = ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18));
            st.game_picker = Some(picker);
            st.config.v6_render = app::config::V6RenderMode::Hybrid;
            let area = ratatui::layout::Rect::new(0, 0, w, h);
            let mut buf = ratatui::buffer::Buffer::empty(area);
            let _ = app::render::screen::render_story_pane(&model, false, None, &st, area, &mut buf);
            // `band WxH@(x,y) [Art, tiled]: … · source WxH native px` — the flank
            // is the band narrower than the pane, and its source height plus the
            // rows above it is the `desired_height` the composer is handed.
            let log = st.graphics_render.borrow().band_log.clone();
            let mut top = 0;
            let mut rows = 0;
            for line in &log {
                let Some(rest) = line.strip_prefix("band ") else { continue };
                let Some((dims, tail)) = rest.split_once('@') else { continue };
                let cols: u16 = dims.split_once('x').and_then(|(a, _)| a.parse().ok()).unwrap_or(0);
                if cols >= w {
                    // The full-width plate above the flanks: its native rows are
                    // the crop the flank band starts below.
                    if let Some(n) = tail.rsplit_once("· native ").and_then(|(_, t)| t.split_once('x')) {
                        top = n.1.split('@').next().and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
                    }
                    continue;
                }
                if let Some(src) = tail.rsplit_once("· source ").and_then(|(_, t)| t.split_once(" native")) {
                    rows = src.0.split_once('x').and_then(|(_, b)| b.parse::<u32>().ok()).unwrap_or(0);
                }
            }
            assert!(rows > 0, "{w}x{h}: the render drew no tiled side flank");
            // `top` is the native row the flank's crop began at — the depth of the
            // full-width plate that used to own the column above it. Since SQ-0894
            // built the ring from content there is usually no such plate: the flank
            // starts at the pane's own top row and crops from native row 0, so `top`
            // is legitimately 0 and the whole column is in `rows`.
            //
            // The QUANTITY this helper yields is unchanged, which is why it is still
            // the right one to assert on. Measured on Zork Zero at a 98x37 pane: the
            // flank source was 91x456 under a plate 88 native rows deep (88 + 456),
            // and is now 91x544 with no plate (0 + 544).
            top + rows
        })
        .collect()
}

// ── 1. The foot is the bottom-most ink ───────────────────────────────────────

/// **The reported defect.** At every pane, on both flanks, the column's last
/// painted row is the last row of the band — the foot — and the rows above it
/// are the foot's own flare, not shaft.
///
/// FALSIFY by restoring `recognize`'s exact `bottom >= native_h`: the flank is
/// read as Shogun's slab again, `shogun()` mirrors the whole column below
/// itself, and the assertion fails with bare shaft running past the foot.
#[test]
fn the_macintosh_pillar_puts_its_foot_on_the_bottom_row() {
    if mac_disk().is_none() {
        return;
    }
    for honor in [true, false] {
        let s = launch(Some("Pic.data"), honor);
        let desired = desired_heights(&s, PANES);
        for f in flanks(&s) {
            for (&(w, h), &d) in PANES.iter().zip(&desired) {
                let out = app::render::v6_border::flank_source(
                    &f.gfx, &f.gfx, f.x0, f.x1, f.art, f.native_h, 0, d,
                )
                .unwrap_or_else(|| panic!("{w}x{h}: the flank must extend to {d} native rows"));
                assert_eq!(
                    last_ink(&out),
                    Some(d - 1),
                    "{w}x{h} cols {}..{}, honor_game_colours={honor}: the last painted row of a \
                     {d}-row column must be its last row — the foot is the bottom-most thing on \
                     the pillar, with no shaft beneath it",
                    f.x0,
                    f.x1,
                );
                // …and what sits there is the FOOT, which flares wider than the
                // shaft above it rather than merely being the shaft's last row.
                let sp = spans(&out);
                let foot = sp[d as usize - 1].expect("the last row is painted");
                let shaft = sp[(d - 40) as usize].expect("40 rows up is shaft or foot");
                assert!(
                    foot.1 - foot.0 > shaft.1 - shaft.0,
                    "{w}x{h} cols {}..{}, honor_game_colours={honor}: the bottom row spans \
                     {foot:?} and the column 40 rows above it spans {shaft:?} — a foot is WIDER \
                     than the shaft it stands on, so this bottom is shaft, not a base",
                    f.x0,
                    f.x1,
                );
            }
            // …and the reason it comes out that way, so a failure above says
            // which link decided.
            assert_eq!(
                app::render::v6_border::recognize(&f.gfx, f.x0, f.x1, f.art, f.native_h),
                Some(app::render::v6_border::BorderArt::ZorkZeroPillars),
                "cols {}..{}, honor_game_colours={honor}: the Macintosh monochrome flank is a \
                 PILLAR — art rows {:?} of a {}-row screen",
                f.x0,
                f.x1,
                f.art,
                f.native_h,
            );
        }
    }
}

// ── 2 & 3. Uniform stride, and a count the span decides ──────────────────────

/// **The bands repeat at ONE stride, and there are more of them in a taller
/// pane.** The user's requirement in their own words: *"the key is that repeats
/// need uniform spacing"* and *"as the pilars grow in length the repeats will
/// need to be recalculated"*.
///
/// The stride is allowed to vary by a single row, and by no more: the extension
/// is divided into `k` whole rows and the remainder has to land somewhere.
///
/// FALSIFY by handing `zork_zero`'s banded arm to `extend_pillars` instead: its
/// fixed stride leaves an arbitrary remainder against the foot, and the last gap
/// stops matching the others.
#[test]
fn the_bands_are_evenly_spaced_and_the_pane_decides_how_many() {
    if mac_disk().is_none() {
        return;
    }
    for honor in [true, false] {
        let s = launch(Some("Pic.data"), honor);
        let desired = desired_heights(&s, PANES);
        for f in flanks(&s) {
            // The art's own rhythm, measured on the picture: where the capital
            // ends and the foot begins.
            let (top, bottom) =
                app::render::v6_border::banded_shaft(&crop(&f.gfx, f.x0, f.x1, f.native_h), f.art.1)
                    .expect("the Macintosh monochrome flank declares a banded shaft");
            /// Clear of the capital's taper above and the foot's flare below.
            const CLEAR: u32 = 4;
            let art_bands = bands(&f.gfx_flank(), (top + CLEAR, bottom - CLEAR));
            let art_tail = bottom - *art_bands.last().expect("the picture carries a band");
            let mut counts = Vec::new();
            for (&(w, h), &d) in PANES.iter().zip(&desired) {
                let out = app::render::v6_border::flank_source(
                    &f.gfx, &f.gfx, f.x0, f.x1, f.art, f.native_h, 0, d,
                )
                .expect("extended");
                let foot_top = d - (f.art.1 - bottom);
                let found = bands(&out, (top + CLEAR, foot_top - CLEAR));
                assert!(
                    !found.is_empty(),
                    "{w}x{h} cols {}..{}, honor_game_colours={honor}: a {d}-row column shows no \
                     band at all — the articulation the art carries has been tiled away",
                    f.x0,
                    f.x1,
                );
                let strides: Vec<u32> = found.windows(2).map(|p| p[1] - p[0]).collect();
                if let (Some(lo), Some(hi)) = (strides.iter().min(), strides.iter().max()) {
                    assert!(
                        hi - lo <= 1,
                        "{w}x{h} cols {}..{}, honor_game_colours={honor}: the bands of a {d}-row \
                         column sit at {found:?}, strides {strides:?} — they must be one stride \
                         apart to the row, never a run plus a remainder",
                        f.x0,
                        f.x1,
                    );
                }
                // The gap from the LAST band to the foot is the art's own, which
                // is the gap a fixed-stride run cannot hold: the extension ends
                // on a whole copy of the picture rather than on a remainder.
                assert_eq!(
                    foot_top - found.last().copied().expect("a band"),
                    art_tail,
                    "{w}x{h} cols {}..{}, honor_game_colours={honor}: the last band of a {d}-row \
                     column stands {} rows above the foot where the picture itself stands \
                     {art_tail} — the run must end on a whole copy of the art, not on whatever \
                     the stride had left over",
                    f.x0,
                    f.x1,
                    foot_top - found.last().copied().unwrap(),
                );
                counts.push((d, found.len()));
            }
            // A taller pane gets MORE bands, never longer gaps.
            let mut sorted = counts.clone();
            sorted.sort();
            for pair in sorted.windows(2) {
                assert!(
                    pair[0].1 <= pair[1].1,
                    "cols {}..{}, honor_game_colours={honor}: {:?} — a taller column must not \
                     carry FEWER bands",
                    f.x0,
                    f.x1,
                    sorted,
                );
            }
            assert!(
                sorted.last().expect("panes").1 > sorted.first().expect("panes").1,
                "cols {}..{}, honor_game_colours={honor}: {sorted:?} — the tallest pane must \
                 carry more bands than the shortest, or the count is not being recalculated",
                f.x0,
                f.x1,
            );
            // …and the pane the user judged keeps the articulation it had.
            assert!(
                counts[0].1 >= 1,
                "cols {}..{}, honor_game_colours={honor}: 120x40 ({} rows) shows {} bands, and \
                 the render the user approved shows one",
                f.x0,
                f.x1,
                counts[0].0,
                counts[0].1,
            );
        }
    }
}

impl Flank {
    /// The flank's own columns, cropped out of the frame — what `flank_source`
    /// measures internally.
    fn gfx_flank(&self) -> image::RgbaImage {
        crop(&self.gfx, self.x0, self.x1, self.native_h)
    }
}

fn crop(gfx: &image::RgbaImage, x0: u32, x1: u32, h: u32) -> image::RgbaImage {
    let mut out = image::RgbaImage::new(x1.min(gfx.width()) - x0, h.min(gfx.height()));
    for y in 0..out.height() {
        for x in 0..out.width() {
            out.put_pixel(x, y, *gfx.get_pixel(x0 + x, y));
        }
    }
    out
}

// ── 4. The colour rendition is unmoved ───────────────────────────────────────

/// **The other archive on the same disk must not have moved.** `CPic.data`'s
/// pillars have a FEATURELESS shaft, so `pillar_shaft` measures them and they
/// keep Bocfel's `extend_pillars` exactly as it was — the mirror of SQ-0808
/// included, which a banded column cannot use because a mirror moves its band.
///
/// Pinned as the composition's own numbers rather than as a hash, so a failure
/// says WHICH part moved.
#[test]
fn the_colour_pillars_on_the_same_disk_keep_bocfels_composition() {
    if mac_disk().is_none() {
        return;
    }
    for honor in [true, false] {
        let s = launch(None, honor);
        for f in flanks(&s) {
            assert_eq!(
                (f.native_h, f.art),
                (400, (0, 400)),
                "honor_game_colours={honor}: CPic.data is 320x200 doubled onto a 640x400 screen, \
                 and its pillars are painted to the last row of it",
            );
            assert_eq!(
                app::render::v6_border::pillar_shaft(&f.gfx_flank(), f.art.1),
                Some((82, 374)),
                "cols {}..{}, honor_game_colours={honor}: the colour pillar's shaft is PLAIN, and \
                 the derivation of Bocfel's four constants rests on it (SQ-0799)",
                f.x0,
                f.x1,
            );
            // A plain shaft never reaches the banded arm, so nothing about this
            // rendition changed — asserted, not assumed.
            for d in [591u32, 810] {
                let out = app::render::v6_border::flank_source(
                    &f.gfx, &f.gfx, f.x0, f.x1, f.art, f.native_h, 0, d,
                )
                .expect("extended");
                let sp = spans(&out);
                // Bocfel's foot: 13 raw rows doubled, stamped flush at the bottom
                // with the shaft's span unbroken all the way down to it.
                assert!(sp[d as usize - 1].is_some(), "the colour foot reaches row {}", d - 1);
                let shaft = sp[82].expect("the shaft's first row");
                assert!(
                    (82..d - 26).all(|y| sp[y as usize] == Some(shaft)),
                    "cols {}..{}, honor_game_colours={honor}: the colour shaft must hold ONE span \
                     from row 82 to the foot of a {d}-row column — it is the featureless shaft \
                     that lets Bocfel's fixed stride be invisible",
                    f.x0,
                    f.x1,
                );
            }
        }
    }
}
