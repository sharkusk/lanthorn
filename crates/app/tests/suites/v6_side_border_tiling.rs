//! SQ-0698 / SQ-0781 — the v6 side border art is **tiled**, not stretched, and
//! the frame agrees at its corners.
//!
//! Three titles frame their story window with side artwork authored for a
//! 320x200 screen. Until this suite existed, lanthorn made up the difference by
//! stretching the flank band vertically (SQ-0511) — measured here at 2.2x the
//! horizontal factor on Zork Zero and 3.0x on Shogun at a 117x64 terminal — and
//! Arthur's flank was simply CLIPPED at the row his poles stop, leaving the
//! frame open down the lower half of the pane.
//!
//! What is asserted, in the order the requirements were set:
//!
//! 1. **Every side flank is TILED.** The render's own band log says which draw
//!    each band took, so this reads the render's report rather than
//!    re-implementing the pipeline.
//! 2. **The frame reaches the bottom.** The pane's own cells, at the flank's
//!    columns, on the band's last row.
//! 3. **The corners agree.** The invariant a uniform stretch gives Bocfel for
//!    free (it composes the whole frame in native pixels and scales the canvas
//!    once): the side art and the top plate must land at the same horizontal
//!    factor at every pane width, and no band may be anisotropic. Asserted as a
//!    RELATION between the bands, never as a particular factor — the factor is
//!    an implementation detail, the agreement is the requirement.
//! 4. **Nothing else moved.** No other v6 title in the corpus draws a tiled
//!    band at all.
//! 5. **Every RENDITION tiles cleanly** (SQ-0799). Since SQ-0790 the player
//!    picks the picture archive, and Zork Zero's MCGA, EGA and CGA plates do not
//!    agree on where its pillars begin — so a repeat unit pinned to one of them
//!    seams on the other two.
//! 6. **Every rendition is recognised as its own TITLE** (SQ-0802). Shogun's DOS
//!    art reaches the native screen bottom where its Amiga art stops short, so a
//!    recogniser that decides on that alone hands it Zork Zero's masonry.
//! 7. **No tile join steps harder than the art does by itself** (SQ-0808). The
//!    cut landing in the plain shaft is not sufficient: Zork Zero's CGA pillar is
//!    a *lit* column, and a repeat that only translates it resets the shading.
//! 8. **RASTER mode ships the same frame** (SQ-0793). Tiling landed on the hybrid
//!    ring alone, so the two pixel modes drew different screens from one turn.
//!    Raster composes at the 640x400 native screen and scales ONCE, so the flanks
//!    must be complete before that scale — a hybrid-only case proves nothing here.
//! 9. **A picture column over a command MENU is not a border** (SQ-0819). The
//!    exclusion hybrid makes and raster did not: Journey's illustration sits in
//!    the flank columns but stops above its menu strip, and extending it tiled
//!    canyon wall over "The Party".
//! 10. **All three of Zork Zero's SCENE borders** (SQ-0792). Only one of them,
//!     the castle, is ever reached by a play session this suite can afford, so
//!     the other two are composed from each archive's own pictures the way
//!     `DISPLAY_BORDER` draws them — a method the castle validates, because
//!     composed and in-game agree to the row.
//!
//! Fixtures are named by exact release, per CLAUDE.md: a disk image is a
//! different build, not the same story on other media. `stories/` is gitignored,
//! so every case skips vacuously when its fixture is absent.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::render::v6_border::BorderArt;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// One border specimen: the exact build it was measured on, and the layout its
/// flanks must be recognised as.
struct Specimen {
    title: &'static str,
    file: &'static str,
    release: u16,
    serial: &'static str,
    /// Blank turns from boot to a gameplay frame with the frame on screen.
    turns: usize,
}

/// The three titles whose side art this work extends — the same three named in
/// Bocfel's `draw_border.cpp` header ("Used by Arthur, Shogun, and Zork Zero").
/// Journey is deliberately absent: its frame is glyphs, not artwork (SQ-0750),
/// and Bocfel's border file does not mention it either.
const SPECIMENS: &[Specimen] = &[
    Specimen { title: "Arthur", file: "Arthur - The Quest for Excalibur.adf", release: 54, serial: "890606", turns: 12 },
    Specimen { title: "Shogun", file: "James Clavell's Shogun.adf", release: 295, serial: "890321", turns: 12 },
    Specimen { title: "Zork Zero", file: "zork0-r393-s890714.z6", release: 393, serial: "890714", turns: 12 },
    // SQ-0898: BOTH Arthur presses again, at the turn count that reaches the
    // Frame-plan flank STRETCH. Appended rather than filed beside their neighbours
    // because four cases below index this table positionally.
    //
    // **Five turns, not twelve, and that is the whole reproduction.** The plan is
    // `Frame` only while the story window reaches the screen bottom, and Arthur
    // sizes window 0 to the text he is about to print: it is `(28, 208, 584, 192)`
    // — bottom 400 — for exactly one frame, and settles at 128 (release 74) or 96
    // (the Amiga) from the next turn on. Every other case in this file drives past
    // it. Both presses do this on the same turn, so the fixture is a turn count and
    // not a medium, and `arthur-r74-s890714.z6` joins the table because the two
    // presses draw the flank differently once they get there: release 74's
    // content-derived flank columns are 30 native px, a bare pole his 72-run status
    // bar cuts in two, so the upper piece lies wholly inside the artwork and
    // `flank_tiled_source` correctly declines to extend it.
    Specimen { title: "Arthur@5", file: "Arthur - The Quest for Excalibur.adf", release: 54, serial: "890606", turns: 5 },
    Specimen { title: "Arthur@5", file: "arthur-r74-s890714.z6", release: 74, serial: "890714", turns: 5 },
];

/// v6 stories with no side border art of these shapes, each with the RENDITION to
/// boot it against (`None` for the medium's own art). None of them may grow a tiled
/// band: this work is the three titles above and nothing else.
///
/// **Journey's three DOS plates are here because of SQ-1094**, which was reported
/// as a recurring flank defect rather than an Arthur one, and Journey was named as
/// the other title showing it. It does not reach this code at all, and this is
/// where that is stated rather than assumed: measured at the gameplay frame, the
/// left column of Journey's own frame is art rows 25..279 of a 400-row screen, an
/// INSET of 146 — more than the quarter-screen `v6_border::recognize` allows a
/// flank — so it answers `None` and nothing is extended, on the medium's art and on
/// `journey.cg1`, `journey.eg1` and `journey.mg1` alike. Journey's rule is glyphs
/// and a Menu-plan panel (SQ-0750, and case 11's divider extension); a border
/// SECTIONING defect cannot reach it.
/// A named picture archive and the build it is paired with — the release and
/// serial ride along because [`boot_named`] checks them.
#[derive(Clone, Copy)]
struct Rendition {
    archive: &'static str,
    release: u16,
    serial: &'static str,
}

/// One story to check, on the medium's own art or on a named rendition.
struct Untouched {
    file: &'static str,
    rendition: Option<Rendition>,
}

const UNAFFECTED: &[Untouched] = &[
    Untouched { file: "advent.z6", rendition: None },
    Untouched { file: "journey-r83-s890706.z6", rendition: None },
    Untouched { file: "journey-r83-s890706.z6", rendition: Some(Rendition { archive: "journey.cg1", release: 83, serial: "890706" }) },
    Untouched { file: "journey-r83-s890706.z6", rendition: Some(Rendition { archive: "journey.eg1", release: 83, serial: "890706" }) },
    Untouched { file: "journey-r83-s890706.z6", rendition: Some(Rendition { archive: "journey.mg1", release: 83, serial: "890706" }) },
    Untouched { file: "Journey - The Quest Begins.adf", rendition: None },
    Untouched { file: "fmvpoker.z6", rendition: None },
    Untouched { file: "mysterious01.z6", rendition: None },
];

/// Pane sizes swept. The last is the deliberately TALL one, where the stretch
/// factor this work removes was largest.
///
/// Every one of them MAGNIFIES: 1.25, 1.4375, 1.4625, 1.5. That is a precondition
/// for most of this file and not an oversight — a flank only has to be tiled at a
/// pane whose letterbox leaves vertical slack past the artwork, and where it does
/// not, "must be TILED" is not a property of a correct render. See
/// [`MINIFYING_PANES`] for the half that was missing.
/// `(108, 50)` is the user's own reported pane (scale 1.35), the one that reaches
/// the Frame-plan flank STRETCH — see [`SPECIMENS`] on why Arthur's second press
/// had to join this file for it.
const PANES: &[(u16, u16)] = &[(100, 40), (108, 50), (115, 61), (117, 64), (120, 90)];

/// Panes BELOW scale 1, swept by the cases about magnification (SQ-0898).
///
/// Everything in this file was checked above scale 1 until SQ-0898, and the
/// blindness that bought was double. A minifying pane is the only place a flank's
/// SOURCE is clipped by the artwork's edge while its DESTINATION is not, which is
/// the defect; and `scale_halo` is zero at or above 1, so the band log's `native`
/// field — which every case here parses — only tells the truth where it cannot
/// matter. Two defects shipped through this suite for that reason alone.
///
/// `(76, 46)` is the user's own 78x49 terminal at an 8x18 cell (scale 0.95).
/// `(70, 19)` is the pane they reported the visible corner fragment on, and it is
/// the only pane in either list wide enough for its height that the letterbox
/// leaves a HORIZONTAL margin (`off_x = 6`) — which is what made the fragment six
/// device pixels wide there and sub-pixel everywhere else.
const MINIFYING_PANES: &[(u16, u16)] = &[(76, 46), (70, 19)];

/// Both lists: for the cases that assert a magnification, which is a property of
/// every pane rather than of a pane with slack.
fn all_panes() -> impl Iterator<Item = (u16, u16)> {
    PANES.iter().chain(MINIFYING_PANES).copied()
}

use crate::fixture_paths::fixture_path;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot exactly as `startup.rs` does — the profile comes from the MEDIUM, the
/// artwork from whatever that medium supplies — after checking the build.
fn boot(file: &str, release: Option<(u16, &str)>) -> Option<GameSession> {
    boot_machine(file, release).map(|(s, _)| s)
}

/// [`boot`], keeping the [`app::machine_boot::MachineBoot`] it resolved.
///
/// The v6 CELL rides on that value, and it is the machine's: 7x15 on a Macintosh
/// press, 8x16 everywhere else (SQ-0917). A case that measures native geometry has
/// to read it from here rather than assume `V6Cell::DEFAULT`, or it lays a
/// Macintosh frame out on a grid the game was never told about — SQ-1020/SQ-1021
/// exactly. One boot, so the two forms cannot disagree.
fn boot_machine(
    file: &str,
    release: Option<(u16, &str)>,
) -> Option<(GameSession, app::machine_boot::MachineBoot)> {
    let path = fixture_path(file);
    let (loaded, _) = app::hints::load_mounted_story(&path).ok().or_else(|| {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        None
    })?;
    let bytes = loaded.bytes().to_vec();
    if let Some((rel, serial)) = release {
        assert_eq!(
            u16::from_be_bytes([bytes[2], bytes[3]]),
            rel,
            "{file}: this medium carries a DIFFERENT build than the table says"
        );
        assert_eq!(String::from_utf8_lossy(&bytes[0x12..0x18]), serial, "{file}: serial");
    }
    // No tier-3 archive is named here — this suite resolves art through
    // `PictSource::resolve`, so the profile comes from the medium alone.
    let profile = InterpreterProfile::resolve(&path, None, None, None);
    let mut picts = PictSource::resolve(&path, None);
    let picture_dims = picts.all_pict_dims();
    // `startup.rs`'s own chain, `native_std_window` included. Without that step a
    // press whose art is not 640x400 is told it has a 640x400 screen, and the GAME
    // lays its windows out to fit it — so every rect the case then measures belongs
    // to a screen the player never sees. Arthur's and Journey's ProDOS releases are
    // both 560x384 presses, and both read as 640x400 here until this was added.
    // SQ-1021/SQ-1022: every per-machine fact in one value, so this
    // harness cannot omit one — it was omitting the CELL.
    let machine = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        profile.default_colours(),
        true,
        app::native_font::FaceSet::none(),
        profile.palette(),
        None,
    );
    let mut s = GameSession::new_for_machine(bytes, true, false, false, picture_dims, None, None, &machine)
    .unwrap_or_else(|e| panic!("{file}: should boot without a ZError: {e:?}"));
    assert!(!s.quit, "{file}: quit during boot");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some((s, machine))
}

/// Boot a story against a NAMED picture archive — SQ-0734 tier 3, the door the
/// player uses to pick a rendition. The archive's own flavour selects the
/// machine (`for_art_flavour`), it supplies the standard window, and it supplies
/// the per-axis art scale, which is `(1, 2)` for a 640-wide EGA or CGA plate and
/// `(2, 2)` for a 320-wide MCGA or Amiga one (SQ-0790). Exactly what
/// `startup.rs` does. `None` when either file is absent.
fn boot_named(story: &str, archive: &str, release: (u16, &str)) -> Option<GameSession> {
    let path = fixture_path(story);
    let apath = fixture_path(archive);
    let raw = std::fs::read(&apath)
        .map_err(|_| eprintln!("SKIP: gitignored archive missing at {}", apath.display()))
        .ok()?;
    let pics = blorb::infocom_pics::InfocomPics::parse(raw).expect("a native Infocom archive parses");
    let (loaded, _) = app::hints::load_mounted_story(&path).ok().or_else(|| {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        None
    })?;
    let bytes = loaded.bytes().to_vec();
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), release.0, "{story}: release");
    assert_eq!(String::from_utf8_lossy(&bytes[0x12..0x18]), release.1, "{story}: serial");
    let profile = InterpreterProfile::for_art_flavour(pics.flavour());
    let mut picts = PictSource::from_native(pics);
    let picture_dims = picts.all_pict_dims();
    // `PictSource::std_window` answers from a Blorb's `Reso` chunk only; the
    // standard window a NAMED archive implies is `PictureOverride::std_window`,
    // and it is the same for every rendition (SQ-0790).
    // The chain `startup.rs` runs: a Blorb's `Reso`, else the archive's own
    // picture space (SQ-0838 — 320x200 for MCGA/Amiga, 640x200 for EGA/CGA, and
    // 480x300 for the standard Macintosh's mono plate). The screen is that space
    // times the density below, which is 640x400 for every rendition here.
    // SQ-1021/SQ-1022: every per-machine fact in one value, so this harness
    // cannot omit one — it was omitting the CELL.
    let machine = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        profile.default_colours(),
        true,
        app::native_font::FaceSet::none(),
        profile.palette(),
        None,
    );
    let mut s =
        GameSession::new_for_machine(bytes, true, false, false, picture_dims, None, None, &machine)
    .unwrap_or_else(|e| panic!("{story} + {archive}: should boot without a ZError: {e:?}"));
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some(s)
}

/// Drive to a gameplay frame: Enter at a keypress, an empty line at a prompt,
/// `n` at Arthur's "Please press Y or N" restore question.
fn drive(s: &mut GameSession, turns: usize) {
    for _ in 0..turns {
        let r = match s.pending_input() {
            InputKind::Line => s.submit(""),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = s.submit_char(b'n');
        }
        if s.quit {
            return;
        }
    }
}

/// A hybrid render at a plausible kitty cell (8x18). Halfblocks is the protocol
/// (no terminal to query), which is what lets a case assert on the pane's own
/// CELLS: the image lands in them.
#[allow(deprecated)] // `from_fontsize`: a headless test has no terminal to query.
fn render_state_with(honor: bool) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::from_fontsize(CELL));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    state
}

/// The one statement of this suite's device cell, and the ONLY one (SQ-0990).
///
/// 8x18 is a plausible kitty cell and deliberately NOT the gallery's 8x16: the
/// gallery wants an exact 1:2 so a half-block sample comes out square (SQ-0963),
/// while this suite is about how bands tile and stretch, where a cell that is not
/// 1:2 is the harder case and the one a real terminal is more likely to hand you.
/// The value is not the point — the duplication was. It used to be restated at
/// four `pane.1 as u32 * 18` sites beside the picker that actually decides, and
/// nothing tied them: change the picker and every one of those numbers silently
/// keeps measuring a screen the render no longer draws, which is exactly how
/// `ring_scout` and this file's own `boot()` came to measure 560x384 presses at
/// 640x400 and hide two real defects for two rounds (SQ-0901). [`pane_dev`] now
/// reads the cell back off the picker, so the two cannot disagree at all.
const CELL: ratatui_image::FontSize = ratatui_image::FontSize { width: 8, height: 18 };

/// The pane in DEVICE pixels, taken from the picker the render will actually use.
fn pane_dev(state: &app::state::AppState, pane: (u16, u16)) -> (u32, u32) {
    let fs = state
        .game_picker
        .as_ref()
        .expect("every render_state carries a picker; the cell is what this converts")
        .font_size();
    (u32::from(pane.0) * u32::from(fs.width), u32::from(pane.1) * u32::from(fs.height))
}

/// The shipped default, and this suite's primary baseline.
fn render_state() -> app::state::AppState {
    render_state_with(true)
}

/// One line of the render's band log, parsed: the band's device cell rect, how
/// it was drawn, and the size of the source it was drawn from in NATIVE pixels.
#[derive(Debug, Clone, Copy)]
struct Band {
    cells: (u16, u16),
    at: (u16, u16),
    tiled: bool,
    stretched: bool,
    src: (u32, u32),
    /// The longest contiguous run of source rows with no opaque pixel at all,
    /// and the row it starts at — a hole in a tiled flank (SQ-0698).
    blank: (u32, u32),
}

/// Parse `band WxH@(x,y) [Slot, how]: … · source WxH native px` (tiled) and
/// `band WxH@(x,y): … · native WxH@(x,y)` (plain / stretched).
fn parse_bands(log: &[String]) -> Vec<Band> {
    fn pair(s: &str, sep: char) -> Option<(u32, u32)> {
        let (a, b) = s.split_once(sep)?;
        Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
    }
    log.iter()
        .filter_map(|line| {
            let rest = line.strip_prefix("band ")?;
            let (dims, rest) = rest.split_once('@')?;
            let (w, h) = pair(dims, 'x')?;
            let (at, rest) = rest.strip_prefix('(')?.split_once(')')?;
            let (x, y) = pair(at, ',')?;
            let tiled = rest.contains("tiled]");
            let stretched = rest.contains("stretched]");
            let src = if tiled {
                let s = rest.rsplit_once("· source ")?.1;
                pair(s.split_once(" native")?.0, 'x')?
            } else {
                let s = rest.rsplit_once("· native ")?.1;
                pair(s.split('@').next()?, 'x')?
            };
            let blank = rest
                .rsplit_once("longest run ")
                .and_then(|(_, t)| t.split_once(" at "))
                .and_then(|(n, a)| Some((n.trim().parse().ok()?, a.trim().parse().ok()?)))
                .unwrap_or((0, 0));
            Some(Band { cells: (w as u16, h as u16), at: (x as u16, y as u16), tiled, stretched, src, blank })
        })
        .collect()
}

/// Render one frame and hand back the pane's cells and the bands it drew.
fn frame(session: &GameSession, pane: (u16, u16)) -> (Buffer, Vec<Band>, (u16, u16)) {
    let model = session.screen();
    let state = render_state();
    let area = Rect::new(0, 0, pane.0, pane.1);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    let bands = parse_bands(&state.graphics_render.borrow().band_log);
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    (buf, bands, native)
}

/// One band's magnification, taken from the render's structured record rather
/// than parsed back out of the log (SQ-0898): the band's rect, whether it claims
/// to be on the letterbox grid at all, and the sizes that went into its resample
/// — source in NATIVE pixels, destination in DEVICE pixels.
///
/// [`parse_bands`] cannot answer this and the difference is the whole of SQ-0898.
/// A plain crop's `native` field in the log is a HASH FOOTPRINT carrying the area
/// filter's halo, so below scale 1 it reads two or three pixels wider than the
/// crop and neighbouring bands appear to overlap where they in fact partition the
/// scaled canvas exactly. At or above scale 1 the halo is zero and the two agree,
/// which is precisely why every case in this file was written against the log and
/// every one of them passed while the defect was on screen.
/// The magnifications one frame drew at, beside the frame's own letterbox scale.
fn frame_mags(session: &GameSession, pane: (u16, u16)) -> (Vec<app::render::graphics::BandMag>, f32) {
    let model = session.screen();
    let state = render_state();
    let area = Rect::new(0, 0, pane.0, pane.1);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let pane_dev = pane_dev(&state, pane);
    let s = app::render::v6_layout::uniform_scale(native, pane_dev).s;
    let mags = state.graphics_render.borrow().band_mags.clone();
    (mags, s)
}

/// The bands that are SIDE flanks.
///
/// Selected by EDGE since SQ-0894, not by `cells.0 < pane_w`. The content-built
/// ring narrows the top and bottom bands to the story viewport's columns on any
/// row a flank took, so "narrower than the pane" now matches those too: on Arthur
/// at 100x40 it collected his `90x13` top band as a fifth flank.
fn flanks(bands: &[Band], pane_w: u16) -> Vec<Band> {
    bands
        .iter()
        .copied()
        .filter(|b| b.cells.0 < pane_w && (b.at.0 == 0 || b.at.0 + b.cells.0 >= pane_w))
        .collect()
}

/// The lowest flank piece on each side — the ones that must reach the story's
/// bottom and so are the ones that must EXTEND past the artwork.
fn lowest_flank_per_side(fl: &[Band], pane_w: u16) -> Vec<Band> {
    let side = |b: &Band| b.at.0 == 0;
    let mut out = Vec::new();
    for left in [true, false] {
        if let Some(b) = fl.iter().filter(|b| side(b) == left).max_by_key(|b| b.at.1) {
            out.push(*b);
        }
    }
    let _ = pane_w;
    out
}

// ── 1. Every side flank is tiled ─────────────────────────────────────────────

#[test]
fn every_side_flank_is_tiled_and_none_is_stretched() {
    let mut ran = 0;
    for sp in SPECIMENS {
        let Some(mut s) = boot(sp.file, Some((sp.release, sp.serial))) else { continue };
        drive(&mut s, sp.turns);
        for &(w, h) in PANES {
            let (_, bands, _) = frame(&s, (w, h));
            let fl = flanks(&bands, w);
            // SQ-0894: a flank is no longer one band per side. The content-built ring
            // stops a flank at any row carrying full-width chrome TEXT, so Arthur's
            // column is two pieces — rows 0..13 and 14..40, split at his status bar —
            // and the split is at the bar rather than at the arbitrary edge of the
            // story window it used to fall on. What must still hold is that BOTH
            // sides are present.
            let left = fl.iter().filter(|b| b.at.0 == 0).count();
            let right = fl.iter().filter(|b| b.at.0 != 0).count();
            assert!(
                left > 0 && right > 0,
                "{} [release {}] at {w}x{h}: expected a left and a right flank band, got {fl:?}",
                sp.title,
                sp.release
            );
            // Never STRETCHED — that is the defect this case is named for, and it
            // applies to every piece.
            for b in &fl {
                assert!(
                    !b.stretched,
                    "{} [release {}] at {w}x{h}: flank {b:?} must never be stretched",
                    sp.title,
                    sp.release
                );
            }
            // TILED is required of the piece that has to reach past the artwork. A
            // piece wholly inside the art (Arthur's upper 5x13, native rows 0..187 of
            // 400) needs no extension and is a plain crop, which is not the SQ-0698
            // defect — that was the flank stopping SHORT, asserted directly below.
            for b in lowest_flank_per_side(&fl, w) {
                assert!(
                    b.tiled,
                    "{} [release {}] at {w}x{h}: the lowest flank {b:?} must be TILED to reach the story's bottom",
                    sp.title,
                    sp.release
                );
            }
            ran += 1;
        }
    }
    if fixture_path(SPECIMENS[0].file).exists() {
        assert!(ran > 0, "the fixtures are present but nothing ran — check the filenames");
    }
}

// ── 2. The frame reaches the bottom ──────────────────────────────────────────

/// The defect as reported: *"the side columns for arthur does not stretch all
/// the way down"*.
///
/// Two statements, both of which fail with the original symptom when the
/// extension is removed. The band must run to the story viewport's own bottom —
/// SQ-0511 used to CLIP it to the artwork's lowest opaque native row instead,
/// which on Arthur is row 379 of 400 and left the frame standing open down the
/// pane's lower quarter — and the pane cell beside the bottom of the viewport
/// must carry something, rather than the theme backdrop the clip left behind.
#[test]
fn a_flank_reaches_the_story_viewports_bottom() {
    for sp in SPECIMENS {
        let Some(mut s) = boot(sp.file, Some((sp.release, sp.serial))) else { continue };
        drive(&mut s, sp.turns);
        for &(w, h) in PANES {
            let model = s.screen();
            let state = render_state();
            let area = Rect::new(0, 0, w, h);
            let mut buf = Buffer::empty(area);
            let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
            let bands = parse_bands(&state.graphics_render.borrow().band_log);
            let vp = state.transcript_geom.get().expect("hybrid renders the story as a transcript").area;
            // SQ-0894: only the LOWEST piece per side has to reach the bottom — a
            // flank may now be split by a full-width chrome text row (Arthur's status
            // bar), and an upper piece legitimately stops at it.
            let fl = flanks(&bands, w);
            for b in lowest_flank_per_side(&fl, w) {
                assert!(
                    b.at.1 + b.cells.1 >= vp.bottom(),
                    "{} [release {}] at {w}x{h}: the flank band {b:?} stops at row {} while the \
                     story viewport runs to {} — the frame stands open below it",
                    sp.title,
                    sp.release,
                    b.at.1 + b.cells.1,
                    vp.bottom()
                );
                let cell = &buf[(b.at.0, vp.bottom() - 1)];
                assert_ne!(
                    cell.bg,
                    ratatui::style::Color::Reset,
                    "{} [release {}] at {w}x{h}: column {} beside the bottom of the story is the \
                     theme backdrop, not border art",
                    sp.title,
                    sp.release,
                    b.at.0
                );
            }
        }
    }
}

/// SQ-0698 — **the gap between the tiled pieces**, reported on Shogun:
/// *"there is a gap between the tiled shogun side-art pieces"*.
///
/// Case 2 above only asks that the band REACH the bottom, which a flank with a
/// hole punched through its middle does — the placement rect is the full height
/// either way, and that is precisely why this defect survived the first suite.
/// The source image is the only place it shows, so the render reports the
/// longest run of rows in it that carry no opaque pixel, and where that run
/// starts; this reads those two numbers.
///
/// A run at row 0 is legitimate and is measured, not tolerated: the renderer
/// clears the rows a chrome TEXT strip covers so they draw as crisp cells
/// (SQ-0500), and a flank whose crop begins inside that band starts blank —
/// 3 rows on Shogun at 100x40, 4 and 6 on Arthur. A run starting anywhere BELOW
/// row 0 is a hole.
///
/// Measured on `James Clavell's Shogun.adf` (release 295, serial 890321) at
/// 120x90: an interior run of 64 blank native rows starting at band row 599 —
/// native 636, centred on the join at native 668 (`2·border_height − overlap`) —
/// which the uniform 1.475 scale put on the user's screen as a 94px black band
/// between the two ornate gold panels.
///
/// The cause is worth naming here because any future handler can fall into it:
/// the chrome canvas a band ships is the artwork MINUS whatever the renderer
/// draws as cells instead, and Shogun's status line is two 16px rows that the
/// top of its border sits behind. A repeat cut from that canvas carries the hole
/// twice — once at the flipped copy's foot, once at the tiled block's head — and
/// the two meet at the join. A repeat cut from the graphics-only canvas does not.
#[test]
fn a_flank_has_no_gap_between_its_tiled_pieces() {
    for sp in SPECIMENS {
        let Some(mut s) = boot(sp.file, Some((sp.release, sp.serial))) else { continue };
        drive(&mut s, sp.turns);
        for &(w, h) in PANES {
            let (_, bands, _) = frame(&s, (w, h));
            for b in flanks(&bands, w) {
                let (run, at) = b.blank;
                assert_eq!(
                    (run > 0).then_some(at),
                    (run > 0).then_some(0),
                    "{} [release {}] at {w}x{h}: the flank source {b:?} has an INTERIOR hole — \
                     {run} row(s) with no art in them starting at row {at} of the band. A blank \
                     run at row 0 is the chrome text strip's own rows, cleared so they draw as \
                     cells; one below that is a gap between the tiled pieces",
                    sp.title,
                    sp.release
                );
            }
        }
    }
}

// ── 3. The corners agree ─────────────────────────────────────────────────────

/// **A REGRESSION TRIPWIRE, NOT A NEW PROPERTY.** This case passes on main
/// before SQ-0698 as well as after it, and that is the point: the user's
/// horizontal rule ("top artwork present → stretch the side artwork by the top
/// plate's factor, so the frame agrees at the corners") describes a result they
/// already see and prefer to the reference implementation —
///
/// > "our arthur side-art looks much cleaner than spatterlight, since our side
/// > art perfectly aligns with the header image (in spacing and width). I want
/// > to keep this look."
///
/// Bocfel gets its horizontal consistency from a single uniform stretch of the
/// whole native canvas at the end (`flush_bitmap` stretch-blits the pixmap to
/// fill the window), which guarantees agreement but gives no control over how
/// the pieces relate. lanthorn places each band itself, so the agreement is a
/// property that can be broken — hence this case. SQ-0698 changed the VERTICAL
/// axis only; nothing here should ever move.
///
/// The relation asserted is between the bands actually drawn: every one of them,
/// side flanks and the full-width top plate alike, maps its native columns to
/// the pane at one horizontal factor.
/// **ONE FRAME, ONE MAGNIFICATION** — the whole class, in one assertion (SQ-0898).
///
/// The two cases below it assert the same thing as a RELATION between bands, from
/// the band log, at panes that all magnify. This asserts it as a PROPERTY, against
/// the frame's own letterbox scale, from the render's structured record, at panes
/// that minify as well. It is strictly stronger on every axis, and each of those
/// three differences is one of the reasons the defect got through:
///
///   * a relation is satisfied by every band being wrong together;
///   * the log's `native` is a halo'd hash footprint below scale 1 ([`BandMag`]);
///   * the flank's source is clipped by the artwork's edge only when the pane is
///     LARGER than the scaled screen on one axis, i.e. only below scale 1.
///
/// The property: every band showing the game's screen lands at `s` device pixels
/// per native pixel, on both axes. The extension changes WHAT a flank draws — rows
/// of art past the ones the game painted — never at what magnification, which is
/// the difference between tiling a column and stretching it.
///
/// The tolerance is one native pixel. A source is whole pixels and a destination is
/// whole device pixels, so half a native pixel of rounding is unavoidable on each;
/// anything beyond that is a piece placed somewhere the frame's scale does not put
/// it. FALSIFIED by restoring the pre-SQ-0898 destination (the whole band, however
/// much of it the artwork reaches): Arthur's poles come back at 6.35 and 7.20 device
/// pixels adrift at `(70, 19)`, and every other pane in the list stays clean —
/// which is also the proof that this is the pane that reproduces.
///
/// **The exemption is keyed on the SITE, and that took a second round of SQ-0898.**
/// As first written this case exempted every band whose fit was not `Letterbox`,
/// and the fit was recorded inside `draw_chrome_band_stretched` as a constant — so
/// the exemption belonged to the DRAWING FUNCTION, and every caller of it, present
/// and future, was outside the gate by construction. Two callers were the intended
/// exceptions. The third was the Frame-plan flank stretch, which drew Arthur's
/// banner-row poles at 0.63 vertical against the frame's 1.35 while this case
/// reported green. A caller now names its own fit and only two named sites are
/// exempt. FALSIFIED a second time, by restoring that arm: `Arthur@5 [release 54] at
/// 100x40 (scale 1.2500): band 5x13 at (0,0) draws 40x234 device px from 32x400
/// native px — 1.2500/0.5850`, and the same on release 74 and at every other pane in
/// both lists.
#[test]
fn every_band_draws_at_the_frames_one_magnification() {
    let mut ran = 0;
    for sp in SPECIMENS {
        let Some(mut s) = boot(sp.file, Some((sp.release, sp.serial))) else { continue };
        drive(&mut s, sp.turns);
        ran += 1;
        for (w, h) in all_panes() {
            let (mags, scale) = frame_mags(&s, (w, h));
            assert!(!mags.is_empty(), "{} at {w}x{h}: no bands drawn", sp.title);
            // One native pixel, and never less than one device pixel — at a
            // minifying pane a native pixel is worth less than a device one and the
            // destination's own rounding still costs a whole one.
            let tol = scale.max(1.0);
            for &(r, fit, src, dst) in &mags {
                // The exemption is keyed on the SITE that drew the band, never on
                // the FUNCTION that drew it (SQ-0898, second round). `BandFit` used
                // to be a two-state answer recorded inside
                // `draw_chrome_band_stretched`, so the Menu panel, the divider
                // extension and the Frame-plan flank stretch — three unrelated
                // decisions sharing one drawing routine — were exempt together, and
                // the third was the defect this case exists for.
                if !fit.on_the_letterbox_grid() {
                    continue;
                }
                let off_x = dst.0 as f32 - src.0 as f32 * scale;
                let off_y = dst.1 as f32 - src.1 as f32 * scale;
                assert!(
                    off_x.abs() <= tol && off_y.abs() <= tol,
                    "{} [release {}] at {w}x{h} (scale {scale:.4}): band {}x{} at ({},{}) draws \
                     {}x{} device px from {}x{} native px — {:.4}/{:.4} px per native px, where the \
                     frame's own letterbox scale is {scale:.4}. Its far edge sits {off_x:.2}/{off_y:.2} \
                     device px from where every other piece of this screen puts it, and a column drawn \
                     in two pieces at two magnifications is a visible seam.\nall bands: {mags:?}",
                    sp.title,
                    sp.release,
                    r.width, r.height, r.x, r.y,
                    dst.0, dst.1, src.0, src.1,
                    dst.0 as f32 / src.0.max(1) as f32,
                    dst.1 as f32 / src.1.max(1) as f32,
                );
            }
        }
    }
    assert!(ran > 0 || !stories_dir().exists(), "no border specimen present — every case skipped");
}

/// SQ-0898 retargeted this onto [`frame_mags`] and widened it to
/// [`MINIFYING_PANES`]. It read the factor as `cells x 8 / src.0` out of the band
/// LOG, and on a plain crop the log's `native` is a hash footprint carrying the
/// area filter's halo — zero at or above scale 1, two or three pixels below it. At
/// Arthur's `(76, 46)` that reads 0.865 for a band the render draws at 0.950,
/// against 0.941 for the flank beside it: a 9% "disagreement" between two pieces
/// that agree to a third of a pixel. Left on the log this case could not be given
/// a minifying pane at all, because it would fail on frames that are correct.
///
/// The statement is unchanged and is still a RELATION — every band maps its native
/// columns to the pane at ONE factor, whatever that factor is. That is weaker than
/// [`every_band_draws_at_the_frames_one_magnification`] above and is kept anyway,
/// because it makes no reference to [`app::render::graphics::BandFit`]: a band that
/// claims one of the two named exemptions it is not entitled to would escape the
/// absolute gate and not this one. That is not hypothetical — a band claiming an
/// exemption it had not earned IS the second half of SQ-0898.
#[test]
fn side_art_and_top_plate_share_one_horizontal_scale() {
    for sp in SPECIMENS {
        let Some(mut s) = boot(sp.file, Some((sp.release, sp.serial))) else { continue };
        drive(&mut s, sp.turns);
        for (w, h) in all_panes() {
            let (mags, _) = frame_mags(&s, (w, h));
            assert!(!mags.is_empty(), "{} at {w}x{h}: no bands drawn", sp.title);
            let factors: Vec<f32> = mags.iter().map(|(_, _, src, dst)| dst.0 as f32 / src.0.max(1) as f32).collect();
            let (lo, hi) = factors.iter().fold((f32::MAX, 0.0f32), |(lo, hi), f| (lo.min(*f), hi.max(*f)));
            assert!(
                hi - lo < 0.06 * hi,
                "{} [release {}] at {w}x{h}: the bands disagree on the horizontal factor \
                 ({lo:.3}..{hi:.3}) — the side art no longer aligns with the header plate\n{mags:?}",
                sp.title,
                sp.release
            );
        }
    }
}

/// …and the vertical half, which is what SQ-0698 changed: no band is
/// ANISOTROPIC. The stretch this work removes elongated the side art by whatever
/// the letterbox slack happened to be — measured here at 1.84 vertical against
/// 1.26 horizontal on Shogun at a 100x40 pane, and 2.2x against 1.0x on Zork
/// Zero at 117x64. Tiling fills the same space at the art's own aspect.
///
/// Retargeted onto [`frame_mags`] and widened to [`MINIFYING_PANES`] with its
/// neighbour above, for the same reason: read off the band log's halo'd `native`,
/// Arthur's banner-row flank crop at `(76, 46)` reads 0.865 horizontal against
/// 0.938 vertical and looks anisotropic, when a crop of the one scaled canvas
/// cannot be anisotropic — it copies device pixels 1:1 out of an image that was
/// resized once, isotropically.
#[test]
fn no_side_flank_is_stretched_out_of_aspect() {
    for sp in SPECIMENS {
        let Some(mut s) = boot(sp.file, Some((sp.release, sp.serial))) else { continue };
        drive(&mut s, sp.turns);
        for (w, h) in all_panes() {
            let (mags, _) = frame_mags(&s, (w, h));
            for &(r, fit, src, dst) in &mags {
                // A one-row band is a rounding artefact of its own height, not a
                // statement about scale; skip it rather than loosen the bound.
                if r.height <= 1 {
                    continue;
                }
                let hx = dst.0 as f32 / src.0.max(1) as f32;
                let vy = dst.1 as f32 / src.1.max(1) as f32;
                assert!(
                    (hx - vy).abs() < 0.06 * hx.max(vy),
                    "{} [release {}] at {w}x{h}: band {r:?} [{fit:?}] {src:?}n -> {dst:?}px is \
                     anisotropic — horizontal {hx:.3}, vertical {vy:.3}",
                    sp.title,
                    sp.release
                );
            }
        }
    }
}

// ── 4. Nothing else moved ────────────────────────────────────────────────────

/// The corpus guard the quest asked for: check for any OTHER title whose side
/// art this reaches. A tiled band is the signature; no other v6 story may draw
/// one.
///
/// The panes include `(120, 90)`, the tall one, which is where a border extension
/// runs at all — so this is also SQ-1094's statement about JOURNEY: at a pane far
/// taller than its frame, on its own artwork and on each of its three DOS plates,
/// nothing in Journey's frame is tiled.
#[test]
fn no_other_v6_title_grows_a_tiled_band() {
    for u in UNAFFECTED {
        let file = u.file;
        let Some(mut s) = (match u.rendition {
            Some(r) => boot_named(file, r.archive, (r.release, r.serial)),
            None => boot(file, None),
        }) else {
            continue;
        };
        let what = u.rendition.map_or_else(|| file.to_string(), |r| format!("{file} + {}", r.archive));
        drive(&mut s, 12);
        for &(w, h) in PANES {
            let (_, bands, _) = frame(&s, (w, h));
            let tiled: Vec<_> = bands.iter().filter(|b| b.tiled).collect();
            assert!(
                tiled.is_empty(),
                "{what} at {w}x{h}: this title's flanks are not one of the three border \
                 layouts and must be drawn exactly as before, but got {tiled:?}"
            );
        }
    }
}

// ── 5. Every rendition tiles cleanly ─────────────────────────────────────────

/// The opaque column span of row `y` over columns `[x0, x1)`, as an offset from
/// `x0` so a native flank and the band cropped out of it are comparable.
/// `None` when nothing in that row is painted.
fn opaque_span(img: &image::RgbaImage, y: u32, x0: u32, x1: u32) -> Option<(u32, u32)> {
    let (mut first, mut last) = (None, 0);
    for x in x0..x1.min(img.width()) {
        if img.get_pixel(x, y)[3] >= 128 {
            first.get_or_insert(x - x0);
            last = x - x0;
        }
    }
    first.map(|f| (f, last))
}

/// SQ-0799, as the user reported it: *"for cg1 and eg1 we get a horizontal line
/// on zork0 where we are tiling"*.
///
/// Zork Zero's banner is **34 raw rows on MCGA, 37 on EGA and 39 on CGA** while
/// its pillars are 166 rows in all three (`zork0.mg1` id 5 is 320x34 and id 497
/// 36x166; `zork0.eg1` 640x37 and 74x166; `zork0.cg1` 640x39 and 70x166) — so
/// the pillars begin 6 unit rows lower under EGA and 10 lower under CGA, and a
/// repeat unit cut at a pinned row lands inside the ring beneath the capital
/// instead of in the plain shaft. Every tile boundary then repeats that ring.
///
/// Neither lane that built this could have seen it: SQ-0698's constants were
/// measured and verified when there was exactly ONE Zork Zero geometry, and
/// SQ-0790 made the renditions selectable afterwards.
///
/// **The oracle is independent of the fix.** The shaft's span is taken as the
/// MODAL opaque column span of the flank's own native rows — the one span that
/// holds for hundreds of rows, against a capital, a banner and a base that each
/// hold theirs for a few dozen. Below the art's own bottom every band row is
/// something this code composed, so every one of them, down to where the base is
/// stamped, must carry that span and nothing else. A ring tiled into the shaft
/// is a wider span and shows here; so would a base, or a slice of banner.
///
/// Falsifiable: pin the cut back to unit row 86 and `zork0.eg1` and `zork0.cg1`
/// fail while `zork0.mg1`, `zork0.pic` and the Blorb still pass.
#[test]
fn every_zork_zero_rendition_tiles_only_its_pillar_shaft() {
    /// Tall enough for a SECOND tile to land below the art's own bottom, which
    /// is what makes a repeated ring visible in the extension itself.
    const BAND: u32 = 800;
    /// Every picture archive shipped for Zork Zero, plus its Blorb (`None`).
    const RENDITIONS: &[Option<&str>] = &[
        Some("zork0.mg1"), // MCGA — the layout SQ-0698's constants were tuned to
        Some("zork0.eg1"), // EGA  — banner 3 raw rows taller
        Some("zork0.cg1"), // CGA  — banner 5 raw rows taller
        Some("zork0.pic"), // Amiga/Mac
        None,              // Zork0.blb
    ];
    let sp = &SPECIMENS[2];
    assert_eq!(sp.title, "Zork Zero");
    for rendition in RENDITIONS {
        let booted = match rendition {
            Some(a) => boot_named(sp.file, a, (sp.release, sp.serial)),
            None => boot(sp.file, Some((sp.release, sp.serial))),
        };
        let Some(mut s) = booted else { continue };
        drive(&mut s, sp.turns);
        let name = rendition.unwrap_or("Zork0.blb");
        let model = s.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
        let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
        let gfx = app::render::v6_layout::build_graphics_canvas(&layout.chrome, native);
        let story = layout.story.expect("a story window");
        let flank_x = [(0u32, story.x_px as u32), ((story.x_px + story.w_px) as u32, gfx.width())];
        for (x0, x1) in flank_x {
            // The flank's own columns of the native canvas; the extended band is
            // already cropped to them, so it is read from column 0.
            let native_span = |y: u32| opaque_span(&gfx, y, x0, x1);
            let art = app::render::v6_border::art_extent(&gfx, x0, x1);
            assert_eq!(
                app::render::v6_border::recognize(&gfx, x0, x1, art, native.1 as u32),
                Some(BorderArt::ZorkZeroPillars),
                "{name} cols {x0}..{x1}: Zork Zero's flank is pillars in every rendition"
            );
            // The modal native span — the shaft's, by a margin of hundreds of rows.
            let mut tally: std::collections::HashMap<Option<(u32, u32)>, u32> = Default::default();
            for y in 0..art.1 {
                *tally.entry(native_span(y)).or_default() += 1;
            }
            let (shaft, held) = tally.iter().max_by_key(|(_, n)| **n).map(|(s, n)| (*s, *n)).expect("rows");
            assert!(held > art.1 / 2, "{name} cols {x0}..{x1}: no span holds for most of the flank");
            // …and the base is whatever sits below the last row that carries it.
            let base = art.1 - (0..art.1).filter(|&y| native_span(y) == shaft).max().expect("a shaft row") - 1;
            let out = app::render::v6_border::flank_source(&gfx, &gfx, x0, x1, art, native.1 as u32, 0, BAND)
                .unwrap_or_else(|| panic!("{name} cols {x0}..{x1}: the flank should be extended"));
            let w = out.width();
            let wrong: Vec<u32> = (art.1..BAND - base).filter(|&y| opaque_span(&out, y, 0, w) != shaft).collect();
            assert!(
                wrong.is_empty(),
                "{name} [release {}] cols {x0}..{x1}: {} row(s) of the EXTENSION are not the \
                 pillar's own shaft span {shaft:?} — first at band row {:?}. The banner above \
                 these pillars is a different height in this rendition, so a repeat unit cut at a \
                 pinned row carries the ring under the capital and tiles it down the column",
                sp.release,
                wrong.len(),
                wrong.first(),
            );
        }
    }
}

// ── 7. No tile join steps harder than the art itself (SQ-0808) ───────────────

/// Mean luminance of row `y` over the whole of `img`; a transparent pixel
/// contributes nothing. This is the flank reduced to one number per row, which
/// is what lets a SHADING discontinuity be seen through a dither that changes
/// every pixel from row to row.
fn row_luma(img: &image::RgbaImage, y: u32) -> f64 {
    let mut s = 0.0;
    for x in 0..img.width() {
        let p = img.get_pixel(x, y);
        if p[3] >= 128 {
            s += (p[0] as f64 + p[1] as f64 + p[2] as f64) / 3.0;
        }
    }
    s / img.width() as f64
}

/// `|mean(y-k..y) - mean(y..y+k)|` — a low-pass step detector. Zork Zero's CGA
/// masonry differs wildly between ADJACENT rows (its 1-bit line work is a dither,
/// and consecutive raw rows are essentially uncorrelated: a mean absolute row
/// difference of 7486 against 16065 at the worst), so a per-pixel comparison
/// cannot tell a seam from the texture. A 16-row average can: it sees the
/// pillar's shading and nothing else.
fn luma_step(prof: &[f64], y: usize, k: usize) -> f64 {
    let a: f64 = prof[y - k..y].iter().sum::<f64>() / k as f64;
    let b: f64 = prof[y..y + k].iter().sum::<f64>() / k as f64;
    (a - b).abs()
}

/// SQ-0808, as reported: *"the tiling seam on zork0.cg1 is plainly visible, and
/// Spatterlight's CGA flank does not show it"* — with the page no longer painted
/// white after SQ-0806, there was nothing left to hide it.
///
/// **The cause is not the dither and not the repeat unit.** SQ-0797's blend was
/// ruled out of CGA by its own note; SQ-0799 already derives the cut from the art,
/// and `every_zork_zero_rendition_tiles_only_its_pillar_shaft` proves the cut
/// lands in the plain shaft on all five renditions. The seam survived both
/// because **Zork Zero's CGA pillar is a lit column**: mean row luminance down
/// its shaft runs 97 → 82 top to bottom, where `zork0.mg1` holds a flat 54 and
/// `zork0.eg1` a flat 51. A translation repeat butts the strip's darkest row
/// against its brightest and resets the shading at every join. Measured on
/// `zork0-r393-s890714.z6` + `zork0.cg1` at an 800-row band: a step of **29.3**
/// at band row 654 — the second tile boundary — against the art's own steepest
/// internal step of 16.8.
///
/// The oracle is the art's own behaviour, so it needs no per-rendition constant:
/// nothing this code composes below the artwork may step harder than the pillar
/// shaft steps by itself. Falsifiable: pass `flip = false` to `extend_pillars`
/// in `v6_border::zork_zero` and `zork0.cg1` fails on both flanks with that
/// 29.3-against-16.8 step at row 654, while the other four renditions still pass
/// — which is why no earlier lane could have caught this on MCGA alone.
#[test]
fn no_tile_join_steps_harder_than_the_pillar_shaft_itself() {
    /// Tall enough for two tile boundaries to land below the artwork.
    const BAND: u32 = 800;
    /// The low-pass window, in unit rows — 8 raw rows of a doubled archive.
    const K: usize = 16;
    const RENDITIONS: &[Option<&str>] =
        &[Some("zork0.mg1"), Some("zork0.eg1"), Some("zork0.cg1"), Some("zork0.pic"), None];
    let sp = &SPECIMENS[2];
    assert_eq!(sp.title, "Zork Zero");
    for rendition in RENDITIONS {
        let booted = match rendition {
            Some(a) => boot_named(sp.file, a, (sp.release, sp.serial)),
            None => boot(sp.file, Some((sp.release, sp.serial))),
        };
        let Some(mut s) = booted else { continue };
        drive(&mut s, sp.turns);
        let name = rendition.unwrap_or("Zork0.blb");
        let model = s.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
        let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
        let gfx = app::render::v6_layout::build_graphics_canvas(&layout.chrome, native);
        let story = layout.story.expect("a story window");
        for (x0, x1) in [(0u32, story.x_px as u32), ((story.x_px + story.w_px) as u32, gfx.width())] {
            let art = app::render::v6_border::art_extent(&gfx, x0, x1);
            // The shaft, as the modal opaque span — the same oracle case 5 uses.
            let mut tally: std::collections::HashMap<Option<(u32, u32)>, u32> = Default::default();
            for y in 0..art.1 {
                *tally.entry(opaque_span(&gfx, y, x0, x1)).or_default() += 1;
            }
            let shaft = tally.iter().max_by_key(|(_, n)| **n).map(|(s, _)| *s).expect("rows");
            let rows: Vec<u32> = (0..art.1).filter(|&y| opaque_span(&gfx, y, x0, x1) == shaft).collect();
            let (top, bottom) = (*rows.first().expect("a shaft row"), rows.last().expect("a shaft row") + 1);
            let base = art.1 - bottom;
            let out = app::render::v6_border::flank_source(&gfx, &gfx, x0, x1, art, native.1 as u32, 0, BAND)
                .unwrap_or_else(|| panic!("{name} cols {x0}..{x1}: the flank should be extended"));
            let prof: Vec<f64> = (0..out.height()).map(|y| row_luma(&out, y)).collect();
            // What the pillar does to itself, inside its own shaft…
            let natural = (top as usize + K..bottom as usize - K)
                .map(|y| luma_step(&prof, y, K))
                .fold(0.0f64, f64::max);
            // …against every row this code composed below the artwork.
            let (at, worst) = (art.1 as usize + K..(BAND - base) as usize - K)
                .map(|y| (y, luma_step(&prof, y, K)))
                .fold((0usize, 0.0f64), |a, b| if b.1 > a.1 { b } else { a });
            // A few ulps of slack, because both sides are MEANS of the same
            // pixels summed in a different order: a seamless tile makes the two
            // maxima the same step, and `zork0.cg1` lands on that tie exactly —
            // 9.38953488372091 against 9.38953488372088, equal to twelve
            // significant figures and ordered by the last three bits. (It came to
            // the surface when SQ-0956 moved the CGA lit state from 255 to 170,
            // which rescales every luma in the profile and re-rolls the rounding;
            // the pixels are the same tile they always were.)
            const ULPS: f64 = 1e-9;
            assert!(
                worst <= natural + ULPS,
                "{name} [release {}] cols {x0}..{x1}: the extension steps {worst:.2} at band row \
                 {at}, harder than the {:.2} the pillar shaft ({top}..{bottom}) ever steps by \
                 itself. That is a tile join resetting the pillar's shading — the CGA column is \
                 lit, so a repeat that merely translates it cannot be seamless",
                sp.release,
                natural,
            );
        }
    }
}
// ── 7b. And the same of the FOOT-LESS arm (SQ-1097) ──────────────────────────

/// SQ-1097, as the user reported it: Shogun's flank *"grows from the bottom and
/// meets a single fixed copy of the art at the top"*, with a visible break where
/// the two meet.
///
/// The sibling case above pins the join a flank with a FOOT makes. This pins the
/// other arm, which had no case at all and was wrong in a way `flank_shape` states
/// in one line:
///
/// ```text
/// ShogunSinglePiece · banner 0..0 (0 rows) · middle 0..400 (period 400) · footer 400..400 (0 rows)
/// ```
///
/// A 400-row middle at period 400 means no sub-period was found — the repeat unit
/// is the whole drawing — and a 586-row band leaves 186 rows to fill, which is less
/// than one copy of it. The foot-less fill used to run UPWARD from the pane's
/// bottom, so it took its partial branch on the first iteration and stamped the
/// art's own rows 214..400 straight under art row 399. Every rendition of Shogun
/// that extends at all does this, because they all measure alike.
///
/// ## The oracle
///
/// The same low-pass step detector the sibling uses, at the one row that matters:
/// the boundary between the art and the first row this code composed. The
/// threshold is the drawing's own MEDIAN internal step rather than its maximum,
/// because a slab of ornament has no plain shaft to be quiet — Shogun's vine steps
/// as hard as 34.28 somewhere inside itself, so "no worse than the worst" is
/// satisfied by anything. "No more of a step than the drawing makes at half its own
/// row boundaries" is the property a join has to meet: it must be UNREMARKABLE, not
/// merely not-the-worst.
///
/// Measured at K = 16, all three extending presses alike, both flanks:
///
/// | | join step | the drawing's median |
/// |---|---|---|
/// | before | **18.18** | 11.43 |
/// | after | **0.00** | 11.43 |
///
/// Zero, because the copy adjacent to the art is now the MIRRORED one and a
/// flipped copy opens on the row the art closed with — SQ-1063's own argument for
/// the footed arm, which the foot-less arm was not using.
///
/// Falsifiable: cut the leftover fragment from the unit's other end (`let src_y =
/// if flipped { 0 } else { uh - rest }`) and all three presses fail here at 18.18
/// against 11.43, while Zork Zero's castle — the footed arm, which this case also
/// drives — still passes at 0.00.
///
/// **The fitting pane is the control**: at a band no taller than the art there is
/// nothing to extend, `flank_source` says so by answering `None`, and the case
/// checks that before it measures anything. `stories/Shogun.po` (Apple IIgs,
/// release 311 / serial 890510) is the other control and is absent on purpose — it
/// is a 560x384 press whose art covers its screen, so it never reaches this code.
#[test]
fn no_flank_join_steps_harder_than_the_art_itself() {
    /// The 586-row band `flank_shape` composes by default: a 129x60 pane at an
    /// 8x18 cell over a 400-row screen. Deliberately LESS than twice the art, which
    /// is the case the fill got wrong — at 800 a whole copy fits and it did not.
    const BAND: u32 = 586;
    /// The low-pass window, in unit rows — the sibling case's own constant.
    const K: usize = 16;

    /// One press, and the shape its flank must still have when it is measured.
    struct Press {
        name: &'static str,
        file: &'static str,
        release: u16,
        serial: &'static str,
        /// Turns of blank input from boot to the gameplay frame.
        turns: usize,
        art: (u32, u32),
        want: BorderArt,
    }
    // All three Shogun presses that extend — the defect is not a medium — plus Zork
    // Zero's castle, which is the FOOTED arm and must be untouched by this.
    const PRESSES: &[Press] = &[
        Press { name: "Shogun IBM PC", file: "shogun-r322-s890706.z6", release: 322, serial: "890706", turns: 12, art: (0, 400), want: BorderArt::ShogunSinglePiece },
        Press { name: "Shogun Amiga", file: "James Clavell's Shogun.adf", release: 295, serial: "890321", turns: 12, art: (0, 400), want: BorderArt::ShogunSinglePiece },
        Press { name: "Shogun Macintosh", file: "Shogun.toast", release: 292, serial: "890314", turns: 12, art: (0, 400), want: BorderArt::ShogunSinglePiece },
        Press { name: "Zork Zero IBM PC", file: "zork0-r393-s890714.z6", release: 393, serial: "890714", turns: 12, art: (0, 400), want: BorderArt::ZorkZeroPillars },
    ];

    for p in PRESSES {
        let Some((mut s, machine)) = boot_machine(p.file, Some((p.release, p.serial))) else {
            continue;
        };
        drive(&mut s, p.turns);
        let model = s.screen();
        let WinNode::Layered(items) = &model.root else { panic!("{}: v6 Layered root", p.name) };
        // The CELL is the machine's, and the Macintosh press is told on 7x15
        // (SQ-0917). Reading it off the boot rather than assuming the default is
        // what stops this measuring a screen the player never sees.
        let native = app::render::v6_layout::native_extent(items, &machine.text_face());
        let layout = app::render::v6_layout::classify_windows(items, machine.cell);
        let gfx = app::render::v6_layout::build_graphics_canvas(&layout.chrome, native);
        let story = layout.story.expect("a story window");
        eprintln!(
            "{}: profile {:?}, release {} serial {}, screen {:?}, cell {}x{}, native {}x{}, {} turns",
            p.name, machine.profile, p.release, p.serial, machine.screen_px,
            machine.cell.w(), machine.cell.h(), native.0, native.1, p.turns,
        );
        for (x0, x1, side) in [
            (0u32, story.x_px as u32, "left"),
            ((story.x_px + story.w_px) as u32, gfx.width(), "right"),
        ] {
            let art = app::render::v6_border::art_extent(&gfx, x0, x1);
            // Non-vacuity: this is the frame the numbers above were measured on.
            assert_eq!(art, p.art, "{} [release {}] {side}: the flank's native art rows", p.name, p.release);
            assert_eq!(
                app::render::v6_border::recognize(&gfx, x0, x1, art, native.1 as u32),
                Some(p.want),
                "{} [release {}] {side}: recognised layout", p.name, p.release,
            );

            // The FITTING pane: a band the art already fills is not extended at all.
            assert!(
                app::render::v6_border::flank_source(&gfx, &gfx, x0, x1, art, native.1 as u32, 0, art.1)
                    .is_none(),
                "{} [release {}] {side}: a band no taller than the art needs no extension",
                p.name, p.release,
            );

            let out = app::render::v6_border::flank_source(&gfx, &gfx, x0, x1, art, native.1 as u32, 0, BAND)
                .unwrap_or_else(|| panic!("{} {side}: a {BAND}-row band over {art:?} must be extended", p.name));
            let prof: Vec<f64> = (0..out.height()).map(|y| row_luma(&out, y)).collect();
            // What the drawing does to itself, at half its own row boundaries…
            let mut internal: Vec<f64> =
                (art.0 as usize + K..art.1 as usize - K).map(|y| luma_step(&prof, y, K)).collect();
            internal.sort_by(f64::total_cmp);
            let median = internal[internal.len() / 2];
            // …against the one row this code put next to it.
            let join = luma_step(&prof, art.1 as usize, K);
            assert!(
                join <= median,
                "{} [release {}] {side}: the art→extension join at band row {} steps {join:.2}, \
                 where the drawing itself steps {median:.2} at half its own row boundaries \
                 (worst {:.2}). A join the eye can find is a phase jump: the fill laid a fragment \
                 of the drawing against the drawing instead of continuing it (SQ-1097)",
                p.name, p.release, art.1, internal.last().copied().unwrap_or_default(),
            );
        }
    }
}

// ── The recogniser, pinned per specimen ──────────────────────────────────────

/// What each title's flank art measures, and therefore which handler runs. These
/// are the numbers every constant in `v6_border` was derived against; if a
/// release ever lays its border out differently, this is where it shows.
#[test]
fn each_specimen_is_recognised_as_its_own_layout() {
    use app::render::v6_border::{art_extent, recognize};
    let expected = [
        ("Arthur", BorderArt::ArthurPoles, (11u32, 379u32)),
        // **336 until SQ-1076, and that number was the defect.** Shogun's Amiga
        // border is a full-screen picture whose left columns are painted for all
        // 200 of its rows — 400 native at this press's (2, 2) art scale. It
        // measured 336 here because `@split_window(336)` reset windows 0 and 1 to
        // the full screen width, out of the inset box the game had just given
        // them, and the `erase_window(lower)` on the next instruction then took
        // a 640x64 strip out of window 7 — the frame art's bottom four text rows,
        // permanently. `machine-screenshots/amiga-shogun-main.png` is that frame
        // on the machine, ornament running past the menu text to the bottom.
        ("Shogun", BorderArt::ShogunSinglePiece, (0, 400)),
        ("Zork Zero", BorderArt::ZorkZeroPillars, (0, 400)),
        // The two SQ-0898 entries. Release 54's poles are the same rows whatever
        // turn they are read on; release 74's sit four native rows lower and end
        // four higher.
        ("Arthur@5", BorderArt::ArthurPoles, (11, 379)),
        ("Arthur@5", BorderArt::ArthurPoles, (16, 384)),
    ];
    for (sp, (_, want, want_rows)) in SPECIMENS.iter().zip(expected) {
        let Some(mut s) = boot(sp.file, Some((sp.release, sp.serial))) else { continue };
        drive(&mut s, sp.turns);
        let model = s.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
        let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
        let gfx = app::render::v6_layout::build_graphics_canvas(&layout.chrome, native);
        let story = layout.story.expect("a story window");
        let rows = art_extent(&gfx, 0, story.x_px as u32);
        assert_eq!(
            rows, want_rows,
            "{} [release {}]: the left flank's native art rows",
            sp.title, sp.release
        );
        assert_eq!(
            recognize(&gfx, 0, story.x_px as u32, rows, native.1 as u32),
            Some(want),
            "{} [release {}]: recognised layout",
            sp.title,
            sp.release
        );
    }
}

// ── 6. Every rendition is recognised as its own title (SQ-0802) ──────────────

/// SQ-0802 — **Shogun's DOS renditions were classified as Zork Zero pillars.**
///
/// `recognize` used to decide on one measurement, and reaching the native screen
/// bottom won outright. Shogun's Amiga border stops at native row 336 of 400, but
/// its DOS art is authored for the full 200-row screen: `shogun.mg1` (23x200),
/// `shogun.eg1` (46x200), `shogun.cg1` (58x195) and `Shogun.blb` all paint to row
/// 400 in unit space and therefore satisfied it, so every one of them was handed
/// Zork Zero's masonry recipe — cut at unit row 86, a 284-row repeat, the bottom
/// 26 rows stamped back as a foot — applied to a Japanese lacquer frame. Worse,
/// `shogun.cg1`'s two flanks DISAGREED: the left stops at row 390 and was
/// correctly a single piece while the right reached 400 and was pillars.
///
/// The second measurement is the flank's SHAPE. Both ends of the cut are pinned
/// here from the corpus, at a gameplay frame, both flanks:
///
/// | flank                                       | narrowest ÷ widest painted row |
/// |---------------------------------------------|--------------------------------|
/// | Shogun `.mg1` / `.eg1` / `Shogun.blb`       | 1.00                           |
/// | Shogun `.cg1`                               | 1.00 (L), 0.96 (R)             |
/// | `James Clavell's Shogun.adf` (release 295)  | 1.00                           |
/// | Zork Zero castle, all five renditions       | 0.02 – 0.56                    |
/// | Zork Zero underground / jungle (composed)   | 0.37 – 0.81                    |
///
/// Falsifiable: drop the width test and the four DOS Shogun rows below come back
/// `ZorkZeroPillars` while the Amiga row still passes — which is exactly why the
/// original suite, which only ever booted the `.adf`, could not see it.
#[test]
fn every_rendition_is_recognised_as_its_own_titles_layout() {
    use app::render::v6_border::{art_extent, recognize};
    /// One rendition and the layout its flanks must be recognised as. `archive`
    /// is `None` for whatever `PictSource::resolve` picks off the medium.
    struct Rendition {
        name: &'static str,
        story: &'static str,
        archive: Option<&'static str>,
        release: u16,
        serial: &'static str,
        want: BorderArt,
    }
    const SHOGUN: &str = "shogun-r322-s890706.z6";
    const ZORK0: &str = "zork0-r393-s890714.z6";
    let cases = [
        Rendition { name: "Shogun Amiga", story: SPECIMENS[1].file, archive: None, release: 295, serial: "890321", want: BorderArt::ShogunSinglePiece },
        Rendition { name: "Shogun Blorb", story: SHOGUN, archive: None, release: 322, serial: "890706", want: BorderArt::ShogunSinglePiece },
        Rendition { name: "Shogun MCGA", story: SHOGUN, archive: Some("shogun.mg1"), release: 322, serial: "890706", want: BorderArt::ShogunSinglePiece },
        Rendition { name: "Shogun EGA", story: SHOGUN, archive: Some("shogun.eg1"), release: 322, serial: "890706", want: BorderArt::ShogunSinglePiece },
        Rendition { name: "Shogun CGA", story: SHOGUN, archive: Some("shogun.cg1"), release: 322, serial: "890706", want: BorderArt::ShogunSinglePiece },
        Rendition { name: "Zork Zero MCGA", story: ZORK0, archive: Some("zork0.mg1"), release: 393, serial: "890714", want: BorderArt::ZorkZeroPillars },
        Rendition { name: "Zork Zero EGA", story: ZORK0, archive: Some("zork0.eg1"), release: 393, serial: "890714", want: BorderArt::ZorkZeroPillars },
        Rendition { name: "Zork Zero CGA", story: ZORK0, archive: Some("zork0.cg1"), release: 393, serial: "890714", want: BorderArt::ZorkZeroPillars },
        Rendition { name: "Zork Zero Amiga", story: ZORK0, archive: Some("zork0.pic"), release: 393, serial: "890714", want: BorderArt::ZorkZeroPillars },
        Rendition { name: "Zork Zero Blorb", story: ZORK0, archive: None, release: 393, serial: "890714", want: BorderArt::ZorkZeroPillars },
        Rendition { name: "Arthur Amiga", story: SPECIMENS[0].file, archive: None, release: 54, serial: "890606", want: BorderArt::ArthurPoles },
    ];
    for Rendition { name, story, archive, release, serial, want } in &cases {
        let booted = match archive {
            Some(a) => boot_named(story, a, (*release, serial)),
            None => boot(story, Some((*release, serial))),
        };
        let Some(mut s) = booted else { continue };
        drive(&mut s, 12);
        let model = s.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
        let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
        let gfx = app::render::v6_layout::build_graphics_canvas(&layout.chrome, native);
        let story_win = layout.story.expect("a story window");
        // BOTH flanks: `shogun.cg1`'s disagreed, and one of them was right.
        for (x0, x1, side) in [
            (0u32, story_win.x_px as u32, "left"),
            ((story_win.x_px + story_win.w_px) as u32, gfx.width(), "right"),
        ] {
            let art = art_extent(&gfx, x0, x1);
            assert_eq!(
                recognize(&gfx, x0, x1, art, native.1 as u32),
                Some(*want),
                "{name} [release {release}] {side} flank, native art rows {art:?} of {}: this \
                 title's border is {want:?}. Reaching the screen bottom is not what makes a flank \
                 Zork Zero's pillars — narrowing below its banner is",
                native.1,
            );
        }
    }
}

/// …and the same fix seen through the real render, in both `honor_game_colours`
/// modes: Shogun's DOS flanks draw one tiled band per side with no hole in it,
/// exactly as its Amiga art does. The colour mode cannot reach the flank SOURCE
/// (it is composed from the graphics canvas before any theme decision), but it
/// can reach what the pane does with it, and single-mode suites have masked
/// regressions here before.
#[test]
fn shoguns_dos_flanks_tile_cleanly_in_both_colour_modes() {
    for archive in ["shogun.mg1", "shogun.eg1", "shogun.cg1"] {
        let Some(mut s) = boot_named("shogun-r322-s890706.z6", archive, (322, "890706")) else { continue };
        drive(&mut s, 12);
        for honor in [true, false] {
            for &(w, h) in PANES {
                let model = s.screen();
                let state = render_state_with(honor);
                let area = Rect::new(0, 0, w, h);
                let mut buf = Buffer::empty(area);
                let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
                let bands = parse_bands(&state.graphics_render.borrow().band_log);
                let fl = flanks(&bands, w);
                assert_eq!(
                    fl.len(),
                    2,
                    "{archive} [release 322] at {w}x{h}, honor_game_colours={honor}: expected a \
                     left and a right flank band, got {fl:?}"
                );
                for b in &fl {
                    assert!(
                        b.tiled && !b.stretched,
                        "{archive} at {w}x{h}, honor_game_colours={honor}: flank {b:?} must be TILED"
                    );
                    let (run, at) = b.blank;
                    assert!(
                        run == 0 || at == 0,
                        "{archive} at {w}x{h}, honor_game_colours={honor}: the flank source {b:?} \
                         has an INTERIOR hole — {run} row(s) with no art starting at row {at}"
                    );
                }
            }
        }
    }
}

// ── 8. RASTER mode composes the same frame (SQ-0793) ─────────────────────────

/// SQ-0793 — the two v6 pixel modes must ship the same frame.
///
/// SQ-0698 taught the HYBRID ring to tile side border art and left raster alone,
/// so the same turn drew two different screens. Raster builds the whole frame in
/// the fixed **640x400** native screen (`INFOCOM_V6_STD_WINDOW` doubled —
/// SQ-0479; every rendition maps onto it, MCGA and Amiga doubling on both axes
/// and EGA and CGA vertically only, SQ-0790) and hands the finished canvas to
/// ONE resize, the way Bocfel's `flush_bitmap` stretch-blits its pixmap once.
/// That geometry was already right; what was missing is that the flanks were
/// never completed before the scale.
///
/// **Measured before the fix**, by building the composite each specimen's own
/// gameplay frame produces:
///
/// | fixture (release / serial)                          | flank cols          | art rows | flat band |
/// |-----------------------------------------------------|---------------------|----------|-----------|
/// | `Arthur - The Quest for Excalibur.adf` (54, 890606)  | 0..28, 612..640     | 11..379  | **21 rows, 1 colour** |
/// | `James Clavell's Shogun.adf` (295, 890321)           | 0..46, 594..640     | 0..336   | **64 rows, 1 colour** |
/// | `zork0-r393-s890714.z6` (393, 890714)                | 0..86, 554..640     | 0..400   | none — its pillars already reach the bottom |
///
/// Two statements, both of which fail with that symptom when
/// `extend_raster_flanks` is dropped from `build_v6_raster_canvas`:
///
/// 1. **The composition is native, and it is 640x400.** The canvas the resize is
///    handed is exactly the native screen, so there is one scale for the whole
///    frame and the corners agree structurally rather than by assertion.
/// 2. **The flank carries the ART below the art's own extent**, pixel for pixel,
///    wherever the extension composed from the game's own pictures is opaque —
///    not a flat fill, which is what those 21 and 64 rows were.
#[test]
fn the_raster_composite_extends_its_side_art_to_the_native_bottom() {
    let mut ran = 0;
    for sp in SPECIMENS {
        let Some(mut s) = boot(sp.file, Some((sp.release, sp.serial))) else { continue };
        drive(&mut s, sp.turns);
        // Both modes (CLAUDE.md): true is the shipped default, and a game-set
        // page is exactly what floods the band a short flank leaves behind.
        for honor in [true, false] {
            let model = s.screen();
            let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
            let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
            let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
            let gfx = app::render::v6_layout::build_graphics_canvas(&layout.chrome, native);
            let story = layout.story.expect("a story window");
            let mut state = render_state_with(honor);
            state.config.v6_render = app::config::V6RenderMode::Raster;
            let (canvas, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
            assert_eq!(
                native,
                (640, 400),
                "{} [release {}]: the v6 native screen is the 320x200 standard window doubled",
                sp.title,
                sp.release
            );
            assert_eq!(
                (canvas.width(), canvas.height()),
                (native.0 as u32, native.1 as u32),
                "{} [release {}], honor_game_colours={honor}: the raster composite must BE the \
                 native screen, so the whole frame takes one scale",
                sp.title,
                sp.release
            );
            let native_h = native.1 as u32;
            for (x0, x1) in [(0u32, story.x_px as u32), ((story.x_px + story.w_px) as u32, gfx.width())] {
                let art = app::render::v6_border::art_extent(&gfx, x0, x1);
                // The extension the art itself dictates, composed from the
                // graphics-only canvas — so every pixel it claims is ARTWORK, and
                // the flank's ground (which `gfx` does not carry) is skipped.
                let Some(want) =
                    app::render::v6_border::flank_source(&gfx, &gfx, x0, x1, art, native_h, 0, native_h)
                else {
                    assert_eq!(
                        art.1, native_h,
                        "{} [release {}] cols {x0}..{x1}: a flank with no extension must be one \
                         whose art already reaches the native bottom",
                        sp.title, sp.release
                    );
                    continue;
                };
                let mut flat = std::collections::HashSet::new();
                let mut wrong: Option<(u32, u32, [u8; 4], [u8; 4])> = None;
                for y in art.1..native_h {
                    for x in 0..want.width().min(canvas.width().saturating_sub(x0)) {
                        let got = canvas.get_pixel(x0 + x, y).0;
                        flat.insert(got);
                        let w = want.get_pixel(x, y).0;
                        if w[3] >= 128 && got != w && wrong.is_none() {
                            wrong = Some((x0 + x, y, got, w));
                        }
                    }
                }
                assert!(
                    flat.len() > 1,
                    "{} [release {}], honor_game_colours={honor}: cols {x0}..{x1} of the raster \
                     composite are ONE flat colour {:?} for all {} native rows below the art \
                     (rows {}..{native_h}) — the unpainted band inside the frame's own lower edge",
                    sp.title,
                    sp.release,
                    flat.iter().next(),
                    native_h - art.1,
                    art.1,
                );
                assert_eq!(
                    wrong, None,
                    "{} [release {}], honor_game_colours={honor}: cols {x0}..{x1} of the raster \
                     composite disagree with the extension the art dictates — at (x, y, got, want) \
                     above. Hybrid has tiled this flank since SQ-0698; raster must ship the same \
                     frame",
                    sp.title, sp.release,
                );
                ran += 1;
            }
        }
    }
    if fixture_path(SPECIMENS[0].file).exists() {
        assert!(ran > 0, "the fixtures are present but nothing ran — check the filenames");
    }
}

// ── 9. All THREE of Zork Zero's scene borders (SQ-0792) ──────────────────────

/// Blit `src` into `dst` at `(x, y)`, integer-scaled by `(sx, sy)` — the
/// per-axis art scale a native archive implies (SQ-0790).
fn blit_scaled(dst: &mut image::RgbaImage, src: &image::DynamicImage, x: u32, y: u32, s: (u32, u32)) {
    use image::GenericImageView;
    let (w, h) = src.dimensions();
    for sy in 0..h {
        for k in 0..s.1 {
            let dy = y + sy * s.1 + k;
            if dy >= dst.height() {
                return;
            }
            for sx in 0..w {
                let p = src.get_pixel(sx, sy);
                for j in 0..s.0 {
                    let dx = x + sx * s.0 + j;
                    if dx < dst.width() {
                        dst.put_pixel(dx, dy, image::Rgba(p.0));
                    }
                }
            }
        }
    }
}

/// Columns `[x0, x1)` of `img` as an image of its own — the flank strip
/// `v6_border`'s per-flank routines work in.
fn crop(img: &image::RgbaImage, x0: u32, x1: u32) -> image::RgbaImage {
    let w = x1.min(img.width()).saturating_sub(x0);
    let mut out = image::RgbaImage::new(w.max(1), img.height());
    for y in 0..img.height() {
        for x in 0..w {
            out.put_pixel(x, y, *img.get_pixel(x0 + x, y));
        }
    }
    out
}

/// One of Zork Zero's three scene borders, composed into a native 640x400 canvas
/// exactly as `DISPLAY_BORDER` draws it: the top strip at `(0, 0)`, then the left
/// pillar flush left and the right pillar flush right, both at `y = strip
/// height`. Picture numbers from Bocfel's `zorkzero.hpp`.
fn compose_scene_border(
    picts: &mut PictSource,
    scene: (&str, u32, u32, u32),
    scale: (u32, u32),
    native: (u16, u16),
) -> Option<image::RgbaImage> {
    let (_, strip, left, right) = scene;
    let mut c = image::RgbaImage::new(native.0 as u32, native.1 as u32);
    let top = picts.image(strip)?;
    let (sw, sh) = (top.width() * scale.0, top.height() * scale.1);
    blit_scaled(&mut c, &top, 0, 0, scale);
    if let Some(l) = picts.image(left) {
        blit_scaled(&mut c, &l, 0, sh, scale);
    }
    if let Some(r) = picts.image(right) {
        blit_scaled(&mut c, &r, sw.saturating_sub(r.width() * scale.0), sh, scale);
    }
    Some(c)
}

/// SQ-0792 — Zork Zero's UNDERGROUND and JUNGLE borders, which no play session
/// this suite can afford ever reaches.
///
/// Bocfel dispatches on the game's own border global and gives each scene its own
/// routine. lanthorn cannot read that global — `WinNode::Graphics` carries a
/// flattened `RgbaImage` and picture numbers do not survive the engine boundary —
/// so the question this case settles is how much of the dispatch the PIXELS can
/// replace. It composes each scene's flanks from the archive's own pictures the
/// way `DISPLAY_BORDER` draws them, which is trustworthy because the castle
/// composed this way reproduces the in-game shaft to the row (asserted below).
///
/// **The defect, measured before the fix.** SQ-0799 derives the repeat unit from
/// the art instead of pinning it, which is right for the castle and wrong here:
/// the underground is alternating stone blocks and the jungle is foliage, so the
/// longest constant-span run in them is a coincidence, and it is a DIFFERENT
/// coincidence in each flank. On `zork0.cg1` underground the left flank cut at
/// row 78 and the right at row 296; on `zork0.mg1` jungle the left derived a
/// 14-row repeat unit while the right fell back to the castle's 284. Six of the
/// eight non-castle flank PAIRS got different recipes from each other — a border
/// is symmetric by construction, and this made it asymmetric.
///
/// Two statements:
///
/// 1. **The castle, and only the castle, declares a pillar shaft** — 280 to 292
///    rows of 400 (70–73%) on every rendition and both flanks, against 12 to 180
///    (3–45%) for the other two.
/// 2. **Both flanks of a border therefore get the same recipe**, which is the
///    property that had broken.
///
/// Falsifiable: drop the majority test from `v6_border::pillar_shaft` and
/// `zork0.mg1` underground comes back `Some((74, 128))` on the left and
/// `Some((220, 366))` on the right — two spurious shafts, disagreeing.
#[test]
fn zork_zeros_other_two_scene_borders_declare_no_shaft_and_agree_across_flanks() {
    /// `(name, top strip, left pillar, right pillar)` — Bocfel's `zorkzero.hpp`:
    /// `CASTLE_BORDER` 5 / `OUTSIDE_BORDER` 6 / `UNDERGROUND_BORDER` 7, and
    /// `*_BORDER_L`/`_R` 0x1f1..0x1f6.
    const SCENES: &[(&str, u32, u32, u32)] =
        &[("castle", 5, 0x1f1, 0x1f2), ("underground", 7, 0x1f3, 0x1f4), ("jungle", 6, 0x1f5, 0x1f6)];
    /// Every NATIVE archive shipped for Zork Zero. The Blorb is absent on
    /// purpose: it carries the MCGA plates `zork0.mg1` already covers.
    const RENDITIONS: &[&str] = &["zork0.mg1", "zork0.eg1", "zork0.cg1", "zork0.pic"];
    let sp = &SPECIMENS[2];
    assert_eq!(sp.title, "Zork Zero");
    let mut ran = 0;
    for r in RENDITIONS {
        let Some(mut s) = boot_named(sp.file, r, (sp.release, sp.serial)) else { continue };
        drive(&mut s, sp.turns);
        let model = s.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
        let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
        let gfx = app::render::v6_layout::build_graphics_canvas(&layout.chrome, native);
        let story = layout.story.expect("a story window");
        // One `TEXT_WINDOW_PIC_LOC` picture fixes the flank width for all three
        // scenes, so the castle's in-game width is the right one to read them at.
        let fw = story.x_px as u32;
        let flanks = [(0u32, fw), (native.0 as u32 - fw, native.0 as u32)];
        // What the CASTLE declares in play — the ground truth the composition is
        // checked against below.
        let in_game: Vec<Option<(u32, u32)>> = flanks
            .iter()
            .map(|&(x0, x1)| {
                let art = app::render::v6_border::art_extent(&gfx, x0, x1);
                app::render::v6_border::pillar_shaft(&crop(&gfx, x0, x1), art.1)
            })
            .collect();
        let mut picts = PictSource::from_native(
            blorb::infocom_pics::InfocomPics::parse(std::fs::read(fixture_path(r)).expect("archive"))
                .expect("a native Infocom archive parses"),
        );
        let scale = picts.art_scale().expect("a native archive implies an art scale");
        for scene in SCENES {
            let Some(c) = compose_scene_border(&mut picts, *scene, scale, native) else {
                panic!("{r}: picture {} ({}) is missing", scene.1, scene.0)
            };
            let shafts: Vec<Option<(u32, u32)>> = flanks
                .iter()
                .map(|&(x0, x1)| {
                    let art = app::render::v6_border::art_extent(&c, x0, x1);
                    app::render::v6_border::pillar_shaft(&crop(&c, x0, x1), art.1)
                })
                .collect();
            if scene.0 == "castle" {
                assert_eq!(
                    shafts, in_game,
                    "{r} [release {}]: the castle border COMPOSED from pictures {}/{:#x}/{:#x} \
                     must reproduce the shaft the game itself draws — that agreement is the whole \
                     reason the other two scenes below can be trusted",
                    sp.release, scene.1, scene.2, scene.3
                );
                for (i, sh) in shafts.iter().enumerate() {
                    let (top, bottom) =
                        sh.unwrap_or_else(|| panic!("{r} flank {i}: the castle is a pillar"));
                    assert!(
                        (bottom - top) * 2 >= native.1 as u32,
                        "{r} [release {}] flank {i}: the castle shaft {top}..{bottom} is only \
                         {} of {} rows — a pillar is mostly shaft",
                        sp.release,
                        bottom - top,
                        native.1
                    );
                }
            } else {
                assert_eq!(
                    shafts,
                    vec![None, None],
                    "{r} [release {}]: the {} border declares a pillar shaft. Its stonework holds \
                     no span for long, so any run found in it is a coincidence — and a DIFFERENT \
                     one per flank, which hands the two sides of one symmetric border different \
                     repeat units. Both must fall back to the castle constants",
                    sp.release,
                    scene.0
                );
            }
            ran += 1;
        }
    }
    if fixture_path(RENDITIONS[0]).exists() {
        assert!(ran > 0, "the archives are present but nothing ran — check the filenames");
    }
}

// ── 10. Autocorrelation cannot replace the scene dispatch (SQ-0813) ──────────

/// One flank reduced to a row of luma per pixel, transparent marked `-1`. The
/// autocorrelation below runs over this rather than over the image, because a
/// bounds-checked `get_pixel` per comparison costs ~40x in a debug test binary.
fn luma_rows(img: &image::RgbaImage, rows: (u32, u32)) -> Vec<Vec<f32>> {
    (rows.0..rows.1)
        .map(|y| {
            (0..img.width())
                .map(|x| {
                    let p = img.get_pixel(x, y);
                    if p[3] >= 128 {
                        (p[0] as f32 + p[1] as f32 + p[2] as f32) / 3.0
                    } else {
                        -1.0
                    }
                })
                .collect()
        })
        .collect()
}

/// The strongest repeat period in a flank and how confident it is:
/// `(lag, d')`, where `d'` is YIN's cumulative-mean-normalised difference of
/// `mean |row(y) − row(y+p)|` over every column. **Lower `d'` is more confident**
/// — it is the best lag's mismatch divided by the average lag's, so `d' ≈ 1`
/// means the winner is no better than a coin toss and `d' < 0.15` is the
/// threshold YIN itself calls a detection.
///
/// This is the quest's own proposal, implemented as written and run over the
/// corpus. Nothing in `v6_border` calls it: it is the evidence for a decision,
/// which is why it lives here and not there.
fn best_period(img: &image::RgbaImage, rows: (u32, u32)) -> (u32, f64) {
    let r = luma_rows(img, rows);
    let (h, w) = (r.len(), img.width() as usize);
    let diff = |a: usize, b: usize| -> f64 {
        let mut s = 0.0;
        for (p, q) in r[a].iter().zip(&r[b]) {
            s += match (*p >= 0.0, *q >= 0.0) {
                (true, true) => (p - q).abs() as f64,
                (false, false) => 0.0,
                _ => 255.0,
            };
        }
        s / w as f64
    };
    let (mut best, mut score, mut run) = (0u32, f64::MAX, 0.0);
    for p in 1..h / 2 {
        let d: f64 = (0..h - p).map(|y| diff(y, y + p)).sum::<f64>() / (h - p) as f64;
        run += d;
        let dp = if run > 0.0 { d * p as f64 / run } else { 1.0 };
        if dp < score {
            score = dp;
            best = p as u32;
        }
    }
    (best, score)
}

/// SQ-0813 — **the pixels cannot tell Zork Zero's three scene borders apart by
/// their period, and the castle is the reason.**
///
/// The proposal was to retire the per-scene dispatch: autocorrelate a flank down
/// y, take the strongest period as the tile height, and fall back to a stretch
/// when nothing scores clearly. It subsumes the constant-span case, the argument
/// went, because a constant shaft autocorrelates at every lag. The bar SQ-0813
/// set for itself was that it must first reproduce the castle's shipped
/// derivation — Bocfel's 86 / 26 / 400 / 284 in unit space — on the MCGA art.
///
/// **It reproduces it on nothing.** Measured on the flanks each archive's own
/// pictures compose (the method case 9 validates against the in-game castle), in
/// the window below the top strip, `(best lag, d')`:
///
/// | rendition   | castle          | underground     | jungle          |
/// |-------------|-----------------|-----------------|-----------------|
/// | `zork0.mg1` | **4** (0.730 / 0.739) | 74 (0.909 / 0.925) | 84 (0.469 / 0.502) |
/// | `zork0.eg1` | **4** (1.009 / 1.004) | 74 (0.813 / 0.745) | 84 (0.381 / 0.384) |
/// | `zork0.cg1` | **44** (1.056 / 1.056) | 76 / 74 (0.956 / 0.972) | 84 (0.881 / 0.855) |
///
/// against a shipped repeat unit of 284, 284 / 282 and 272 rows. Driven through
/// the real `zork_zero` handler on the in-game castle it is worse still: the
/// per-pixel form returns **4** on `zork0.mg1`, `zork0.pic` and `Zork0.blb` and
/// **nothing at all** on `zork0.eg1` and `zork0.cg1`, and the low-passed-luma form
/// the quest specified returns nothing on nine of those ten flanks.
///
/// **Why, and it is structural rather than a matter of tuning.** A pillar shaft
/// has no period. `zork0.mg1`'s is uniform — its rows are pixel-identical, mean
/// mismatch 0.30 of 255 at every lag from 4 to 20 alike — so autocorrelation is
/// maximally confident and maximally uninformative, and answers with the smallest
/// lag it is offered. `zork0.cg1`'s is a graded lit column (mean row luma 97 → 82,
/// SQ-0808) — a gradient is not periodic, so its best lag scores `d'` = 1.045,
/// *worse* than the average lag. The statistic measures self-similarity, and a
/// plain shaft is more self-similar than patterned masonry: it is anti-correlated
/// with the thing it was asked to detect.
///
/// Hence the two assertions below. The first is the quest's own bar. The second
/// is that no confidence threshold can admit the two scenes autocorrelation was
/// meant to rescue without also admitting the one it must not touch — the most
/// confident castle flank (0.730) beats the least confident underground flank
/// (0.972) outright, so the accepts and the rejects interleave. SQ-0792's cut sat
/// in a 36%..70% gap and needed no fitting; this one has **no gap at all**, and
/// the corroboration from outside Zork Zero is the same: Arthur's poles, whose
/// repeat unit is 4 rows cut at 90% of their height, score a spurious 64-row
/// period at `d'` = 0.451 — more confident than ten of these twelve flanks — and
/// Shogun's slab a spurious 114-row one at 0.90.
///
/// Two further things the measurement settles, recorded because they are not
/// assertable here: autocorrelation yields only the UNIT, never the `top_cut` or
/// the `foot` that [`app::render::v6_border::extend_pillars`] also needs and that
/// the shape measurement it would replace does supply; and where it does fire on
/// a target scene the two flanks of one symmetric border disagree (`zork0.cg1`
/// underground: 76 left against 74 right), which is precisely the asymmetry
/// SQ-0792 removed.
///
/// `honor_game_colours` is not swept: this composes flanks from the archives'
/// own pictures and never renders, so the colour mode cannot reach it — the same
/// reason case 9 does not sweep it either.
///
/// Falsifiable in both directions. Invert the second assertion — claim the castle
/// is *less* confident than every other scene, which is what a working
/// discriminator would mean — and it fails at 0.730 against 0.972 on the real
/// archives. Should a future statistic ever separate the corpus, this test says
/// so and SQ-0813 can be reopened.
#[test]
fn autocorrelation_cannot_separate_zork_zeros_scene_borders() {
    /// `(name, top strip, left pillar, right pillar)` — Bocfel's `zorkzero.hpp`.
    const SCENES: &[(&str, u32, u32, u32)] =
        &[("castle", 5, 0x1f1, 0x1f2), ("underground", 7, 0x1f3, 0x1f4), ("jungle", 6, 0x1f5, 0x1f6)];
    /// The three DOS plates. `zork0.pic`'s picture 5 is a full 320x200 screen
    /// rather than a top strip, so it composes with no banner to trim below and
    /// its window is not comparable with these; case 9 covers it where the whole
    /// flank is the window.
    const RENDITIONS: &[&str] = &["zork0.mg1", "zork0.eg1", "zork0.cg1"];
    let sp = &SPECIMENS[2];
    assert_eq!(sp.title, "Zork Zero");
    let (mut castle_best, mut other_worst) = (f64::MAX, 0.0f64);
    let mut ran = 0;
    for r in RENDITIONS {
        let Some(mut s) = boot_named(sp.file, r, (sp.release, sp.serial)) else { continue };
        drive(&mut s, sp.turns);
        let model = s.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
        let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
        let story = layout.story.expect("a story window");
        let fw = story.x_px as u32;
        let flanks = [(0u32, fw), (native.0 as u32 - fw, native.0 as u32)];
        let mut picts = PictSource::from_native(
            blorb::infocom_pics::InfocomPics::parse(std::fs::read(fixture_path(r)).expect("archive"))
                .expect("a native Infocom archive parses"),
        );
        let scale = picts.art_scale().expect("a native archive implies an art scale");
        for scene in SCENES {
            let banner = picts.image(scene.1).expect("the scene's top strip").height() * scale.1;
            let Some(c) = compose_scene_border(&mut picts, *scene, scale, native) else {
                panic!("{r}: picture {} ({}) is missing", scene.1, scene.0)
            };
            for (i, &(x0, x1)) in flanks.iter().enumerate() {
                let art = app::render::v6_border::art_extent(&c, x0, x1);
                let f = crop(&c, x0, x1);
                let window = (art.0.max(banner), art.1);
                let (lag, conf) = best_period(&f, window);
                if scene.0 == "castle" {
                    // The quest's own bar: reproduce the shipped repeat unit.
                    let (top, bottom) =
                        app::render::v6_border::pillar_shaft(&f, art.1).expect("the castle is a pillar");
                    let unit = bottom - top - 8; // `zork_zero`'s 2·INSET
                    assert!(
                        lag * 4 < unit,
                        "{r} [release {}] castle flank {i}: the strongest period in the art is \
                         {lag} row(s) at d'={conf:.3}, while the shipped repeat unit measured from \
                         the pillar's shape is {unit}. SQ-0813's bar was that autocorrelation \
                         reproduce that unit before replacing it; if this now fails because the \
                         two AGREE, the derivation has become viable and the quest can be reopened",
                        sp.release,
                    );
                    castle_best = castle_best.min(conf);
                } else {
                    other_worst = other_worst.max(conf);
                }
                ran += 1;
            }
        }
    }
    if !fixture_path(RENDITIONS[0]).exists() {
        return;
    }
    assert!(ran > 0, "the archives are present but nothing ran — check the filenames");
    assert!(
        castle_best < other_worst,
        "the castle's most confident flank scores d'={castle_best:.3} while the least confident \
         underground/jungle flank scores d'={other_worst:.3}. They no longer interleave, so a \
         threshold between them WOULD tell the scene borders apart — which is exactly what \
         SQ-0813 could not find and exactly what would let the per-scene dispatch go"
    );
}

// ── 11. A command MENU under the story window is not a border (SQ-0819) ─────

/// Journey's two builds, at the turn each reaches its title frame — the picture
/// up, the command menu printed, the story window short of the screen bottom.
///
/// A disk image is a different release, not the same story on other media: r30
/// narrates through window 2 and r83 through window 0 (SQ-0755), which is why
/// they need different turn counts to arrive at the same picture.
const MENU_STRIP: &[Specimen] = &[
    Specimen { title: "Journey", file: "Journey - The Quest Begins.adf", release: 30, serial: "890322", turns: 2 },
    Specimen { title: "Journey", file: "journey-r83-s890706.z6", release: 83, serial: "890706", turns: 4 },
];

/// The raster composite must NOT extend a picture column down over a command
/// menu (SQ-0819).
///
/// The hybrid ring has always excluded this case — `tiled_flanks` is empty under
/// the `Menu` bottom plan, because Journey's frame is glyphs and its flank is a
/// picture seated in a panel, not a border to extend (SQ-0750). SQ-0793 gave
/// raster the extension without that exclusion, and the two modes drew different
/// screens from one turn again: measured on `Journey - The Quest Begins.adf`
/// (release 30, serial 890322) the illustration paints native rows 25..279 of
/// columns 0..264, the story window is `(264,16) 368x272`, and "The Party" is
/// printed at native `(152, 288)` — inside the rows the extension claimed. With
/// no title recognised, `v6_border::recognize` fell through to `ArthurPoles`,
/// which tiled a 4-row cut of canyon wall from row 269 down to row 400 and
/// stamped a 28-row "foot" over the menu. The player saw "Individual Commands"
/// alone, the art column smeared across where "The Party" belongs, and the
/// illustration itself reading a third taller than it is. The Amiga original
/// stops the picture above the strip and runs both labels side by side.
///
/// Asserted by COLOUR COUNT, the inverse of the "one flat colour" reading case 8
/// makes: a strip of the game's own glyphs is two-tone — its ink on its page —
/// and a slice of Journey's canyon is not. Measured on r30 at both honour
/// settings, the whole flank below the story window went **10** colours to
/// **2**, and "The Party"'s own glyph box **8** to **2**. The lower bound is
/// asserted alongside the upper one, so a future extension that paints the menu
/// FLAT cannot pass by being monochrome.
#[test]
fn the_raster_composite_leaves_a_command_menu_alone() {
    let mut ran = 0;
    for sp in MENU_STRIP {
        let Some(mut s) = boot(sp.file, Some((sp.release, sp.serial))) else { continue };
        drive(&mut s, sp.turns);
        // Both modes (CLAUDE.md): true is the shipped default, and the page the
        // label sits on is exactly what a declined game colour changes.
        for honor in [true, false] {
            let model = s.screen();
            let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
            let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
            let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
            let story = layout.story.expect("a story window");
            let native_h = native.1 as u32;
            let story_bottom = story.y_px as u32 + story.h_px as u32;
            // The case is only worth anything while the frame still HAS a menu
            // strip below a short story window. Pin that, so a later change to
            // Journey's boot cannot turn this into a vacuous pass.
            let label = layout
                .chrome
                .iter()
                .filter_map(|it| match &it.node {
                    app::engine::WinNode::Grid(g) => Some(g.px_texts.iter()),
                    _ => None,
                })
                .flatten()
                .find(|t| t.text.contains("The Party"))
                .unwrap_or_else(|| {
                    panic!(
                        "{} [release {}]: no \"The Party\" run on screen after {} turns — this \
                         case needs the title frame's command menu",
                        sp.title, sp.release, sp.turns
                    )
                });
            let (lx, ly) = (label.x.max(1) as u32 - 1, label.y.max(1) as u32 - 1);
            assert!(
                ly >= story_bottom && story_bottom + 16 < native_h,
                "{} [release {}]: the menu must sit BELOW a story window short of the screen \
                 bottom — label row {ly}, story bottom {story_bottom}, native height {native_h}",
                sp.title,
                sp.release,
            );

            let mut state = render_state_with(honor);
            state.config.v6_render = app::config::V6RenderMode::Raster;
            let (canvas, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);

            let hues = |x0: u32, x1: u32, y0: u32, y1: u32| {
                let mut set = std::collections::HashSet::new();
                for y in y0..y1.min(canvas.height()) {
                    for x in x0..x1.min(canvas.width()) {
                        set.insert(canvas.get_pixel(x, y).0);
                    }
                }
                set.len()
            };

            // (a) The whole strip below the story window, in the flank columns —
            //     the menu's rules and labels, and nothing else. The flank art
            //     still OFFERS an extension (`flank_source` recognises the column
            //     as poles); the composite must simply not have taken it.
            for (x0, x1) in [(0u32, story.x_px as u32), ((story.x_px + story.w_px) as u32, native.0 as u32)] {
                if x1 <= x0 {
                    continue;
                }
                let n = hues(x0, x1, story_bottom, native_h);
                assert!(
                    (1..=4).contains(&n),
                    "{} [release {}], honor_game_colours={honor}: cols {x0}..{x1} of the raster \
                     composite carry {n} colours below the story window (rows \
                     {story_bottom}..{native_h}) — the menu strip is glyphs on a page, so anything \
                     richer is border art tiled over it",
                    sp.title,
                    sp.release,
                );
            }

            // (b) …and the label itself, which is what the player misses.
            let n = hues(lx, lx + 8 * label.text.chars().count() as u32, ly, ly + 16);
            assert!(
                (2..=4).contains(&n),
                "{} [release {}], honor_game_colours={honor}: \"The Party\" at native ({lx}, {ly}) \
                 is painted in {n} colours — its own ink on its own page is two, so this label is \
                 either buried under artwork or gone",
                sp.title,
                sp.release,
            );
            ran += 1;
        }
    }
    if fixture_path(MENU_STRIP[0].file).exists() {
        assert!(ran > 0, "the fixtures are present but nothing ran — check the filenames");
    }
}

// ── 11. A rule is a rule, not a picture (SQ-0883) ────────────────────────────

/// Journey's four presses, for the flank's own vertical RULE.
///
/// The ProDOS pair is the fixture SQ-0883 was reported on; the other two are the
/// controls that say whether a defect is Apple-specific or medium-general.
/// `journey_s1.dsk` is the same release off the same press, and it is here
/// because a five-disk set and a single `.po` image are two mounts of one build
/// and either could have diverged.
/// The MENU frame — two turns in, where the player answers the restore question
/// and the verb menu comes up — is the one the defect was reported from, and it is
/// a different screen from the gameplay frame four turns in. Both are here: a turn
/// count is part of the specimen, not an incidental, and this case was blind to its
/// own reproduction for want of two rows in this table.
const JOURNEY_MEDIA: &[Specimen] = &[
    Specimen { title: "Journey ProDOS@2", file: "Journey.po", release: 77, serial: "890616", turns: 2 },
    Specimen { title: "Journey ProDOS@2", file: "journey_s1.dsk", release: 77, serial: "890616", turns: 2 },
    Specimen { title: "Journey ProDOS", file: "Journey.po", release: 77, serial: "890616", turns: 4 },
    Specimen { title: "Journey ProDOS", file: "journey_s1.dsk", release: 77, serial: "890616", turns: 4 },
    Specimen { title: "Journey IBM", file: "journey-r83-s890706.z6", release: 83, serial: "890706", turns: 4 },
    Specimen { title: "Journey IBM@2", file: "journey-r83-s890706.z6", release: 83, serial: "890706", turns: 2 },
    Specimen { title: "Journey Amiga", file: "Journey - The Quest Begins.adf", release: 30, serial: "890322", turns: 2 },
];

/// One band the render drew as a DIVIDER EXTENSION: the cells it covers and the
/// native crop it replicates down them.
///
/// Parsed here rather than through [`parse_bands`] because the slot is the whole
/// question — an extension is the one draw in the ring allowed to magnify
/// vertically without bound, and that licence is only sound for a column that is
/// uniform down its whole length.
fn divider_extensions(log: &[String]) -> Vec<(Rect, (u32, u32))> {
    log.iter()
        .filter(|l| l.contains("[DividerExtension"))
        .filter_map(|l| {
            let rest = l.strip_prefix("band ")?;
            let (dims, rest) = rest.split_once('@')?;
            let (w, h) = dims.split_once('x')?;
            let (at, rest) = rest.strip_prefix('(')?.split_once(')')?;
            let (x, y) = at.split_once(',')?;
            let src = rest.rsplit_once("· native ")?.1;
            let (sw, sh) = src.split_once('@')?.0.split_once('x')?;
            Some((
                Rect::new(x.parse().ok()?, y.parse().ok()?, w.parse().ok()?, h.parse().ok()?),
                (sw.parse().ok()?, sh.parse().ok()?),
            ))
        })
        .collect()
}

/// Every ART band's cell rect, off the same log.
fn art_band_rects(log: &[String]) -> Vec<Rect> {
    log.iter()
        .filter(|l| l.contains("[Art,"))
        .filter_map(|l| {
            let rest = l.strip_prefix("band ")?;
            let (dims, rest) = rest.split_once('@')?;
            let (w, h) = dims.split_once('x')?;
            let (at, _) = rest.strip_prefix('(')?.split_once(')')?;
            let (x, y) = at.split_once(',')?;
            Some(Rect::new(x.parse().ok()?, y.parse().ok()?, w.parse().ok()?, h.parse().ok()?))
        })
        .collect()
}

/// A divider extension replicates ONE native row down a whole column, so the
/// column it replicates must be a RULE (SQ-0883).
///
/// The extension is the ring's one licensed anisotropy: a 1-native-row crop
/// stretched the full height of the flank, invisible precisely because the
/// game's rule is uniform down its length. Point it at anything else and the
/// vertical magnification — 738 device pixels out of one native row, at the
/// 171x50 terminal this was reported from — smears that row down the column.
///
/// It found something else on `stories/Journey.po` (**release 77, serial
/// 890616**, booted as `AppleIIgs` off ProDOS), because the run that locates the
/// rule was grown across the PAGE `honor_game_colours` floods behind every
/// window rather than across the game's own ink. The rule is at native x 72..80;
/// the run reported **0..80**, the whole flank. What came back was an **83x1
/// crop through the illustration**, stretched over the entire left column — the
/// artwork replaced by vertical bands of its own row 152 — and drawn AFTER the
/// picture, so it buried it. Both readings are asserted:
///
/// (a) the source may be no wider than the game's own text cell plus the two
///     terminal columns the crop's outward rounding can add, which is the widest
///     a rule can honestly be at any scale;
/// (b) no extension may overlap an ART band, which is SQ-0779's ruling read the
///     other way round — if a game draws a border the artwork does not overlap
///     it, and neither does the border overlap the artwork.
///
/// Both `honor_game_colours` modes, because only one of them was ever wrong: the
/// page flood is what the probe walked through, so the theme-only mode drew this
/// frame correctly throughout and a single-mode case would have passed.
#[test]
fn a_divider_extension_replicates_a_rule_and_never_a_picture() {
    let panes: Vec<(u16, u16)> = all_panes().collect();
    let mut ran = 0;
    for sp in JOURNEY_MEDIA {
        let Some(mut s) = boot(sp.file, Some((sp.release, sp.serial))) else { continue };
        drive(&mut s, sp.turns);
        let model = s.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
        let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        for &(w, h) in &panes {
            for honor in [true, false] {
                let state = render_state_with(honor);
                let area = Rect::new(0, 0, w, h);
                let mut buf = Buffer::empty(area);
                let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
                let log = state.graphics_render.borrow().band_log.clone();
                let scale = app::render::v6_layout::uniform_scale(native, pane_dev(&state, (w, h))).s;
                // One native text cell, plus the terminal column the crop's own
                // outward rounding can add at each end.
                let widest = 8.0 + 2.0 * (8.0 / scale).ceil();
                let arts = art_band_rects(&log);
                for (rect, (sw, sh)) in divider_extensions(&log) {
                    assert!(
                        sh == 1 && sw as f32 <= widest,
                        "{} [release {}] at {w}x{h}, honor_game_colours={honor}: the divider \
                         extension at {rect:?} replicates a {sw}x{sh} native crop down {} cells. \
                         A rule is at most {widest} native px wide at this scale ({scale:.4}); \
                         anything wider is a slice of the picture smeared down the flank",
                        sp.title,
                        sp.release,
                        rect.height,
                    );
                    for a in &arts {
                        assert!(
                            a.intersection(rect).area() == 0,
                            "{} [release {}] at {w}x{h}, honor_game_colours={honor}: the divider \
                             extension at {rect:?} covers the art band at {a:?} — the rule is \
                             drawn after the picture, so an overlap is the picture buried",
                            sp.title,
                            sp.release,
                        );
                    }
                }
                ran += 1;
            }
        }
    }
    if fixture_path(JOURNEY_MEDIA[0].file).exists() {
        assert!(ran > 0, "the fixtures are present but nothing ran — check the filenames");
    }
}

// ── 12. Extending a flank lengthens its shaft and adds nothing (SQ-0899) ─────

/// One flank whose SHAPE is pinned: the columns it occupies in from each edge,
/// and how many full-width bands its artwork carries.
///
/// The width is measured, not guessed — it is the whole flank on every entry, so
/// a window this wide sees the art and nothing beside it. Shogun is absent by
/// design: its border is a slab two dozen columns wide, a different recipe, and
/// cases 1–10 above already measure its extension.
///
/// **`archive` is the RENDITION, and it is what SQ-1094 adds.** These rows used to
/// name a medium and nothing else, so every one of them measured whatever artwork
/// that medium happens to carry — and the defect SQ-1094 reports is
/// rendition-specific: at one pane, one release and one frame, the Blorb beside the
/// story extends Arthur's poles cleanly while `stories/arthur.cg1` prints a clipped,
/// upside-down copy of the banner's clasp partway down them. A table with no
/// archive column cannot see that at all, because the plates the player picks with
/// `--pictures` were never composed here. `Some(name)` boots the tier-3 door
/// ([`boot_named`]); `None` takes the medium's own art. Both are `startup.rs`.
struct FlankShape {
    title: &'static str,
    file: &'static str,
    /// The picture archive to boot against, or `None` for the medium's own art.
    archive: Option<&'static str>,
    release: u16,
    serial: &'static str,
    turns: usize,
    cols: u32,
    /// The flank's own opaque row extent, pinned. A frame is a fixture: a rendition
    /// that draws a different picture in these columns fails here rather than
    /// quietly having its extension measured on something else.
    art: (u32, u32),
    bands: u32,
}

const FLANK_SHAPES: &[FlankShape] = &[
    // Arthur's three ornaments, on every press he has. The ProDOS release is the
    // one SQ-0899 was reported on and the only one whose poles reach both screen
    // edges, because its screen is 560x384 where the others are 640x400.
    FlankShape { title: "Arthur ProDOS", file: "Arthur.po", archive: None, release: 63, serial: "890622", turns: 6, cols: 17, art: (0, 384), bands: 3 },
    FlankShape { title: "Arthur ProDOS", file: "Arthur Quest 4 Excalibur.2mg", archive: None, release: 63, serial: "890622", turns: 6, cols: 17, art: (0, 384), bands: 3 },
    FlankShape { title: "Arthur Amiga", file: "Arthur - The Quest for Excalibur.adf", archive: None, release: 54, serial: "890606", turns: 6, cols: 17, art: (11, 379), bands: 3 },
    FlankShape { title: "Arthur DOS", file: "arthur-r74-s890714.z6", archive: None, release: 74, serial: "890714", turns: 6, cols: 17, art: (16, 384), bands: 3 },
    // Arthur's five PLATES — the renditions `--pictures` selects (SQ-1094).
    // `arthur.cg1` is the one the defect was reported on; the other four are what
    // stop "it works on the one I fixed" passing for a fix. The CGA and EGA plates
    // are a 640x200 picture space and the MCGA and Amiga ones 320x200, so the same
    // poles are drawn at different widths and with different dither — the ornament
    // count is the thing that does not move.
    FlankShape { title: "Arthur CGA", file: "arthur-r74-s890714.z6", archive: Some("arthur.cg1"), release: 74, serial: "890714", turns: 6, cols: 17, art: (16, 384), bands: 3 },
    FlankShape { title: "Arthur EGA", file: "arthur-r74-s890714.z6", archive: Some("arthur.eg1"), release: 74, serial: "890714", turns: 6, cols: 17, art: (16, 384), bands: 3 },
    FlankShape { title: "Arthur EGA-2", file: "arthur-r74-s890714.z6", archive: Some("arthur.eg2"), release: 74, serial: "890714", turns: 6, cols: 17, art: (16, 384), bands: 3 },
    FlankShape { title: "Arthur MCGA", file: "arthur-r74-s890714.z6", archive: Some("arthur.mg1"), release: 74, serial: "890714", turns: 6, cols: 17, art: (16, 384), bands: 3 },
    FlankShape { title: "Arthur Amiga plate", file: "arthur-r74-s890714.z6", archive: Some("arthur.pic"), release: 74, serial: "890714", turns: 6, cols: 17, art: (11, 379), bands: 3 },
    // Zork Zero's one capital, the shape the pillar recipe is FOR.
    FlankShape { title: "Zork Zero DOS", file: "zork0-r393-s890714.z6", archive: None, release: 393, serial: "890714", turns: 6, cols: 17, art: (0, 400), bands: 1 },
    FlankShape { title: "Zork Zero Amiga", file: "Zork Zero - The Revenge of Megaboz.adf", archive: None, release: 366, serial: "890323", turns: 6, cols: 17, art: (0, 400), bands: 1 },
    // Zork Zero's own four PLATES are deliberately absent, and the reason is that
    // this case's oracle cannot state them. `full_width_bands` has no height floor
    // on purpose, and Zork Zero's CGA capital is dithered — its span oscillates back
    // to full width for two to ten rows the whole way down, so `zork0.cg1` measures
    // FIVE bands on its left flank and its two crops are not even symmetric (art
    // rows 2..400 on the left, 4..400 on the right; SQ-0845's own asymmetry). Its
    // renditions are swept instead by
    // `every_zork_zero_rendition_tiles_only_its_pillar_shaft`, whose oracle is the
    // shaft's modal span and is immune to dither.
];

/// Count the maximal runs of rows whose painted span reaches `hi` — the flank's
/// own widest row — with NO minimum height.
///
/// Deliberately re-implemented here rather than called out of `v6_border`: a test
/// that shares the implementation's statistic agrees with it however wrong it is
/// (CLAUDE.md). This reads pixels and counts, and nothing else.
///
/// The absent height floor is the point, and it is why the first draft of this
/// case passed against the defect it was written for. `v6_border` needs a floor —
/// Zork Zero's CGA pillar is dithered and its span oscillates to full width for
/// two rows at a time the whole way down its capital, so a bare count there reads
/// 6 where the architecture is 1. A floor is right for CLASSIFYING one column in
/// isolation and wrong here, where the same column is compared with itself before
/// and after extension: any band the generated rows add is a defect at any
/// height, and Arthur's reprinted ornament comes out **twelve** rows tall against
/// the thirty-four of the real ones.
fn full_width_bands(img: &image::RgbaImage, x0: u32, x1: u32, rows: std::ops::Range<u32>) -> u32 {
    let span = |y: u32| {
        let (mut first, mut last) = (None, 0u32);
        for x in x0..x1.min(img.width()) {
            if img.get_pixel(x, y)[3] >= 128 {
                first.get_or_insert(x);
                last = x;
            }
        }
        first.map_or(0, |f| last - f + 1)
    };
    let rows = rows.start..rows.end.min(img.height());
    let hi = rows.clone().map(span).max().unwrap_or(0);
    if hi == 0 {
        return 0;
    }
    let (mut bands, mut run) = (0, 0);
    for y in rows {
        if span(y) >= hi {
            if run == 0 {
                bands += 1;
            }
            run += 1;
        } else {
            run = 0;
        }
    }
    bands
}

/// Extending a flank down a tall pane LENGTHENS ITS SHAFT. It never adds a band
/// the artwork does not have (SQ-0899).
///
/// This is the invariant behind every per-title recipe in `v6_border`, stated
/// without reference to any of them: whatever a title's border is made of, a pane
/// taller than the art gets more of the plain part, not another copy of the
/// decorated part. A recipe that stamps a capital, an ornament or a mirrored
/// section into the generated rows breaks it, and that is visible on screen as
/// the frame's ornament repeating down the side.
///
/// It caught Arthur's ProDOS press (**release 63, serial 890622**, `AppleIIgs` off
/// ProDOS). Its poles are painted to both edges of its own 560x384 screen, so the
/// INSET that identifies Arthur everywhere else — 32 native px on the Amiga and
/// DOS presses, against a 400-row screen — reads 0, and `recognize` handed him
/// Zork Zero's masonry. Composed to the 586 rows a 129x60 pane asks for, his
/// flank came back
///
/// ```text
///   8x4 17x34 8x30 17x34 8x32 17x34 8x380 17x12 8x26
/// ```
///
/// — a FOURTH 17-wide band, twelve rows of it, printed over the plain shaft near
/// the bottom, where his artwork has three. That is the "banner repeating down
/// the flank" the quest reports. With the ornament count in `recognize` the same
/// composition is `8x4 17x34 8x30 17x34 8x32 17x34 8x418`: the shaft grows from
/// 216 rows to 418 and nothing else moves.
///
/// The band counts are asserted outright as well as compared, so a change that
/// quietly stopped finding the ornaments would fail here rather than pass by
/// making both sides zero.
///
/// ## SQ-1094 — the same defect, one rendition down, and the pane that shows it
///
/// The table now names an ARCHIVE as well as a medium (see [`FlankShape`]), and the
/// row that fails without the fix is `arthur.cg1`. Reported from a 100x50 pane at a
/// 16x32 cell: the pane is 1568x1504 device px against a 1280x800 frame, so the
/// extension runs, and a 174-row clipped copy of the banner's clasp appears at
/// device rows 1330..1504 with plain rule above and below it. The **control is the
/// pane that fits** — at 82x28 the same frame at the same 2.0x scale exactly fills
/// the pane, the extension never runs, and the margin holds two runs and no third.
/// Both halves are stated below: `flank_source` must DECLINE for a band the art
/// already covers, and must add no band for one it does not.
///
/// Why that rendition and no other: `flank_sections` picks the middle out of the
/// column's own pixels, and on the CGA plate the pole below the ornaments is
/// interrupted twice by a two-row narrowing where the MCGA and EGA plates run one
/// texture straight down. The pole's longest exact repeat is therefore 110 rows at
/// period 4, against 128 rows at period 64 for the three ORNAMENTS above it — and
/// the search ranked by length. The ornaments won, the pole became banner, and the
/// flank fell through to the "one drawing" arm, which repeats the whole column
/// mirrored. See `v6_border::flank_sections`.
#[test]
fn extending_a_flank_lengthens_its_shaft_and_adds_no_band() {
    let mut ran = 0;
    for sp in FLANK_SHAPES {
        let Some(mut s) = (match sp.archive {
            Some(a) => boot_named(sp.file, a, (sp.release, sp.serial)),
            None => boot(sp.file, Some((sp.release, sp.serial))),
        }) else {
            continue;
        };
        drive(&mut s, sp.turns);
        let model = s.screen();
        let WinNode::Layered(items) = &model.root else { panic!("{}: v6 Layered root", sp.title) };
        let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
        let gfx = app::render::v6_layout::build_graphics_canvas(&layout.chrome, native);
        let w = native.0 as u32;
        for (edge, x0, x1) in [("left", 0, sp.cols), ("right", w - sp.cols, w)] {
            let art = app::render::v6_border::art_extent(&gfx, x0, x1);
            assert!(art.1 > art.0, "{} [{}] {edge}: this frame paints no flank", sp.title, sp.release);
            assert_eq!(
                art, sp.art,
                "{} [release {}] {edge} flank: this frame's art runs {art:?}, not the {:?} the \
                 table pins — the frame moved, so everything measured below belongs to a \
                 different picture",
                sp.title, sp.release, sp.art,
            );
            let before = full_width_bands(&gfx, x0, x1, art.0..art.1);
            assert_eq!(
                before, sp.bands,
                "{} [release {}] {edge} flank: the ARTWORK carries {before} full-width band(s),                  not the {} this case is pinned to — the frame moved, so the comparison below                  would be measuring the wrong thing",
                sp.title, sp.release, sp.bands,
            );
            // The 586 rows a 129x60 pane at an 8x18 cell asks of a 560x384 screen —
            // the pane SQ-0899 was reported from, and taller than every art here.
            const DESIRED: u32 = 586;
            let Some(img) = app::render::v6_border::flank_source(
                &gfx, &gfx, x0, x1, art, native.1 as u32, 0, DESIRED,
            ) else {
                panic!("{} [{}] {edge}: no extension for a pane taller than the art", sp.title, sp.release)
            };
            let after = full_width_bands(&img, 0, sp.cols, 0..img.height());
            assert_eq!(
                after, before,
                "{} [release {}] {edge} flank extended to {DESIRED} rows: {after} full-width                  band(s) where the artwork has {before}. Extending a flank lengthens its shaft;                  a band that appears only in the generated rows is the frame's ornament reprinted                  down the side",
                sp.title, sp.release,
            );
            // **The CONTROL: a band the art already covers is not extended at all**
            // (SQ-1094). This is the 82x28 pane in the reproduction — the frame fills
            // it exactly and there is nothing to lengthen — and it is what says the
            // comparison above is measuring an extension rather than a crop.
            assert!(
                app::render::v6_border::flank_source(
                    &gfx, &gfx, x0, x1, art, native.1 as u32, 0, art.1,
                )
                .is_none(),
                "{} [release {}] {edge} flank: a band the artwork already covers must not be \
                 extended — a pane that fits the frame is the control for the case above",
                sp.title,
                sp.release,
            );
            ran += 1;
        }
    }
    if fixture_path(FLANK_SHAPES[0].file).exists() {
        assert!(ran > 0, "the fixtures are present but nothing ran — check the filenames");
    }
}

// ── 12. Arthur's ProDOS flank — WITHDRAWN, see SQ-0899 ───────────────────────
//
// A case stood here asserting that Arthur's ProDOS press has no side columns,
// because its frame is a single illustration standing clear of both edges. That
// frame does not exist. It was measured through a `boot()` that skipped
// `native_std_window`, so the story was told it had a 640x400 screen; release 63
// is a 560x384 press, and on its own screen this frame is the ordinary gameplay
// frame with poles at both edges. The case's own non-vacuity guard is what caught
// it once the helper above was fixed — "the art runs 0..=559 of 560 native
// columns".
//
// The function's contract is still pinned, synthetically, by
// `render::screen::tests::a_flanks_columns_come_from_the_narrowest_row_of_its_art`.
// What is NOT pinned, and is the real defect SQ-0899 reports, is the flank's tile
// SOURCE: at the user's 129x60 pane the band at rows 21..60 composes native rows
// 0..381 of the column and draws them starting at device row 378, so the banner
// cap at the column's top is painted a second time partway down the flank. Above
// a taller pane the composition runs to 586 native rows on a 384-row screen and
// the repeat is unmistakable. That needs a fix before it can have a test, and it
// must be measured on the 560x384 screen the app actually gives it.

// ── 13. SQ-0936: the locked magnification, on a real frame ───────────────────

/// The art density this launch would mount, the way `startup.rs` resolves it —
/// `PictSource::art_scale` off the same medium `boot` opens. `None` when the
/// fixture is absent.
///
/// An archive with no picture space to declare (a Blorb) answers `None`, which is
/// the uniform `V6_ART_SCALE` of `(2, 2)` — the same default `AppState` carries.
fn launch_art_scale(file: &str) -> Option<(u32, u32)> {
    let path = fixture_path(file);
    if !path.exists() {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    }
    Some(PictSource::resolve(&path, None).art_scale().unwrap_or((2, 2)))
}

/// [`frame_mags`] with `v6_pixel_lock` ON and the launch's own art density
/// published to the render exactly as `startup.rs` publishes it (SQ-0936).
///
/// Returns the bands, the LOCKED scale, and the free one the same pane would have
/// given — the pair being what makes the case falsifiable, since a pane where the
/// two are equal cannot tell a quantized render from an unquantized one.
fn frame_mags_locked(
    session: &GameSession,
    pane: (u16, u16),
    art_scale: (u32, u32),
) -> (Vec<app::render::graphics::BandMag>, f32, f32) {
    let model = session.screen();
    let mut state = render_state();
    // SQ-0978: the lock is a DEVICE-pixel guarantee and only binds a backend that has
    // device pixels. `render_state`'s picker is Halfblocks — deliberately, so this
    // suite's other cases can assert on the pane's own cells — and half-blocks resolves
    // the picture into cells, where a rung means nothing and the render free-scales.
    // A case whose whole subject is the ladder has to render on a backend the ladder
    // binds, or it asserts a factor nothing used. The CELL is untouched (`pane_dev`
    // reads it back off this picker), so only the backend changes.
    state
        .game_picker
        .as_mut()
        .expect("render_state carries a picker")
        .set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
    state.config.v6_pixel_lock = true;
    state.v6_art_scale = art_scale;
    let area = Rect::new(0, 0, pane.0, pane.1);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let pane_dev = pane_dev(&state, pane);
    let free = app::render::v6_layout::uniform_scale(native, pane_dev).s;
    let locked = app::render::v6_layout::FrameGeometry::new(native, art_scale, zvm::screen::V6Cell::DEFAULT).fitted_scale(pane_dev, true).0.s;
    let mags = state.graphics_render.borrow().band_mags.clone();
    (mags, locked, free)
}

/// With `v6_pixel_lock` on, the frame the render actually draws puts one ART
/// pixel on a whole number of device pixels — on real presses, at real panes.
///
/// Two halves, and both are needed:
///
///   * the frame's factor is on the ladder `1 / gcd(art_scale)` derives
///     (`v6_layout::scale_ladder_step`), which is what "crisp" means here — an
///     art pixel resampled onto a whole device pixel is nearest-neighbour exact;
///   * every band showing the game's screen is drawn at THAT factor, which is
///     what says the render adopted it rather than the test having computed a
///     number nobody used.
///
/// The exemption is the same one
/// [`every_band_draws_at_the_frames_one_magnification`] takes and for the same
/// reason: the Menu-plan panel and the divider extension are not windows onto the
/// game's screen, so the frame's scale is not what sizes them.
///
/// FALSIFIED by making `locked_scale` return the free scale unquantized — see the
/// quest's report for the exact numbers it came back with.
///
/// The `differed` guard is what stops that falsification passing by luck. A pane
/// whose free scale already sits on a rung draws the same frame either way, so a
/// case that only ever saw such panes would be green with the quantization gone.
#[test]
fn locked_scaling_draws_the_frame_on_the_art_pixel_ladder() {
    let mut ran = 0;
    let mut bands_seen = 0;
    let mut differed = 0;
    for sp in SPECIMENS {
        let Some(art_scale) = launch_art_scale(sp.file) else { continue };
        let Some(mut s) = boot(sp.file, Some((sp.release, sp.serial))) else { continue };
        drive(&mut s, sp.turns);
        ran += 1;
        let step = app::render::v6_layout::scale_ladder_step(art_scale);
        for (w, h) in all_panes() {
            let (mags, locked, free) = frame_mags_locked(&s, (w, h), art_scale);
            assert!(!mags.is_empty(), "{} at {w}x{h}: no bands drawn", sp.title);
            if (locked - free).abs() > 1e-4 {
                differed += 1;
            }
            // (a) The factor is a rung: one art pixel, whole device pixels, both axes.
            for (axis, n) in [("x", art_scale.0), ("y", art_scale.1)] {
                let dev = n as f32 * locked;
                assert!(
                    dev >= 1.0 && (dev - dev.round()).abs() <= 1e-3,
                    "{} [release {}] at {w}x{h}: art_scale {art_scale:?} (step {step}) locked to \
                     {locked}, which puts one art pixel on {dev} device pixels along {axis} — a \
                     fraction of a device pixel is exactly the resample softness this mode removes",
                    sp.title,
                    sp.release,
                );
            }
            // (b) …and the render drew at it. Same one-native-pixel tolerance as the
            // sibling case: a source is whole native pixels and a destination whole
            // device pixels, so half of each is unavoidable rounding.
            let tol = locked.max(1.0);
            for &(r, fit, src, dst) in &mags {
                if !fit.on_the_letterbox_grid() {
                    continue;
                }
                bands_seen += 1;
                let off_x = dst.0 as f32 - src.0 as f32 * locked;
                let off_y = dst.1 as f32 - src.1 as f32 * locked;
                assert!(
                    off_x.abs() <= tol && off_y.abs() <= tol,
                    "{} [release {}] at {w}x{h}: band {}x{} at ({},{}) draws {}x{} device px from \
                     {}x{} native px, but the frame's LOCKED scale is {locked} (free would be \
                     {free}). Its far edge sits {off_x:.2}/{off_y:.2} device px from the ladder — \
                     the render is not using the quantized factor.\nall bands: {mags:?}",
                    sp.title,
                    sp.release,
                    r.width, r.height, r.x, r.y,
                    dst.0, dst.1, src.0, src.1,
                );
            }
        }
    }
    assert!(ran > 0 || !stories_dir().exists(), "no border specimen present — every case skipped");
    assert!(
        bands_seen > 0 || ran == 0,
        "specimens booted but no letterbox band was measured — this case was vacuous"
    );
    assert!(
        differed > 0 || ran == 0,
        "every pane's free scale was already on the ladder, so nothing here could tell a \
         quantized render from an unquantized one — add a pane that lands between two rungs"
    );
}

/// The floor, on a real press: a pane too small for the smallest rung falls back
/// to free scaling and still draws a frame. It must not block, clip, or print
/// anything on the game screen — only report the fallback for a diagnostic.
///
/// `(20, 6)` is 160x108 device pixels. Every specimen here is a 640x400 or
/// 560x384 native screen whose art doubles, so its smallest rung is 0.5 and needs
/// at least 320x200 — well past what this pane has.
#[test]
fn a_pane_below_the_smallest_rung_still_renders_freely() {
    let mut ran = 0;
    for sp in SPECIMENS {
        let Some(art_scale) = launch_art_scale(sp.file) else { continue };
        let Some(mut s) = boot(sp.file, Some((sp.release, sp.serial))) else { continue };
        drive(&mut s, sp.turns);
        ran += 1;
        let pane = (20u16, 6u16);
        let model = s.screen();
        let native = match &model.root {
            WinNode::Layered(items) => app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT)),
            other => panic!("v6 Layered root, got {other:?}"),
        };
        let pane_dev = pane_dev(&render_state(), pane);
        assert!(
            app::render::v6_layout::FrameGeometry::new(native, art_scale, zvm::screen::V6Cell::DEFAULT).locked_scale(pane_dev).is_none(),
            "{}: {native:?} into {pane_dev:?} must have no rung, or this case proves nothing",
            sp.title,
        );
        let (scale, fell_back) =
            app::render::v6_layout::FrameGeometry::new(native, art_scale, zvm::screen::V6Cell::DEFAULT).fitted_scale(pane_dev, true);
        assert!(fell_back, "{}: the fallback is reported", sp.title);
        assert_eq!(
            scale.s,
            app::render::v6_layout::uniform_scale(native, pane_dev).s,
            "{}: and it degrades to the free scale rather than blocking",
            sp.title,
        );
        // And the render still produces a frame at that pane with the mode on: a
        // pane this small may legitimately draw no band at all — not drawing is
        // fine, panicking or hanging is not.
        let _ = frame_mags_locked(&s, pane, art_scale);
    }
    assert!(ran > 0 || !stories_dir().exists(), "no border specimen present — every case skipped");
}

/// Arthur's **F2 map screen is not a side border** — SQ-1010.
///
/// The game erases both pole windows and draws picture 137 — 320x96, so 640x192
/// at this press's (2, 2) art scale — across the top of window 7 at (1, 1). Its
/// left and right columns are the scroll's own end-caps, so they genuinely reach
/// the screen's edge and `machine-screenshots/amiga-arthur-map.png` shows them
/// there.
///
/// What does not belong is a COPY of the cap running down the flank past the
/// panel and behind the score bar, which is what `recognize` used to make of it:
/// its single-piece arm tests `top == 0`, and this picture starts at row 0 like a
/// real border does. It ends at row 192 of 400, though — an inset of 208, where
/// every real flank in the corpus insets by at most two text rows — so it is now
/// declined outright and nothing is extended.
///
/// Reported at a pane of 85x38 and visible at every pane swept (80x30 through
/// 160x70); the classification is pane-independent, which is why this case
/// asserts on it rather than on a rendered frame.
#[test]
fn arthurs_map_backdrop_is_not_mistaken_for_a_side_border() {
    use app::render::v6_border::{art_extent, recognize};
    let Some(mut s) = boot("Arthur - The Quest for Excalibur.adf", Some((54, "890606"))) else {
        return;
    };
    drive(&mut s, 16);
    // F2 — ZSCII 133..144 are function keys 1..12 (ZMSD §3.8).
    let _ = s.submit_char(134);

    let model = s.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(
        items,
        &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT),
    );
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let gfx = app::render::v6_layout::build_graphics_canvas(&layout.chrome, native);

    // The frame's own signature first: a release that stops drawing the backdrop,
    // or a route that stops reaching the map, must fail HERE rather than pass
    // vacuously below.
    let rows = art_extent(&gfx, 0, 40);
    assert_eq!(
        rows,
        (0, 192),
        "the map backdrop spans the top 192 native rows of a 400-row frame; \
         got {rows:?} — is this still the map screen?"
    );

    for x1 in [20u32, 28, 32, 40, 64] {
        assert_eq!(
            recognize(&gfx, 0, x1, art_extent(&gfx, 0, x1), native.1 as u32),
            None,
            "crop 0..{x1}: a picture that leaves over half the frame unpainted is \
             not a side flank and must not be extended down the pane",
        );
    }

    // …and the ordinary gameplay frame still IS a flank, so the guard has not
    // simply switched flank extension off for this press.
    let mut s = boot("Arthur - The Quest for Excalibur.adf", Some((54, "890606")))
        .expect("the fixture is present — the case above returned otherwise");
    drive(&mut s, 16);
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(
        items,
        &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT),
    );
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let gfx = app::render::v6_layout::build_graphics_canvas(&layout.chrome, native);
    let rows = art_extent(&gfx, 0, 40);
    assert_eq!(rows, (11, 379), "Arthur's poles, unchanged");
    assert_eq!(
        recognize(&gfx, 0, 40, rows, native.1 as u32),
        Some(BorderArt::ArthurPoles),
        "the poles inset by two text rows and are still recognised",
    );
}
