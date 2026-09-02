//! SQ-1032: the third v6 render mode — `extended`.
//!
//! `raster` builds the game's own screen, letterboxes it into the pane, and spends
//! every surplus device pixel on MAGNIFICATION. `extended` builds the same composite,
//! pins the magnification to a whole number of device pixels per NATIVE pixel, and
//! spends the surplus on CONTENT instead: the canvas grows downward and the extra
//! height becomes whole text rows of prose in the game's own bitmap typeface.
//!
//! **The game is told nothing.** `v6_screen_px` is fixed at construction and no
//! resize path updates it, so the game lays its windows out on exactly the screen it
//! always had — which is now the top of a taller canvas. Nothing here fabricates a
//! screen size no machine had (the SQ-0901 trap), and no per-title layout can break
//! from it.
//!
//! What this suite pins, in the order the cases run:
//!
//!   1. the arithmetic — a whole magnification, and a surplus measured in whole text
//!      rows of the machine's own cell;
//!   2. the game's screen is untouched WIDTHWISE and the canvas grows only downward;
//!   3. the extra rows reach the prose box, so the story viewport really is bigger
//!      (which is also the whole of the `[MORE]` improvement — the pager pages on
//!      *added rows > viewport* and the viewport is what the renderer reports);
//!   4. the flanks tile down the extension rather than leaving bare page beside it;
//!   5. and every frame with nowhere to put the rows declines and is **byte-identical
//!      to `raster`** — which is the regression guard that matters, because `extended`
//!      shares the whole composite with `raster` and a change that reached the shared
//!      code would show up here first.
//!
//! Specimens (release and turn count are part of the fixture — CLAUDE.md):
//!
//! ```text
//!   fixture                                release  turns  role
//!   zork0-r393-s890714.z6                    393       6    art reaches the screen bottom
//!   arthur-r74-s890714.z6                     74      12    poles stop short of it
//!   journey-r83-s890706.z6                    83      40    command menu UNDER the story
//! ```
//!
//! Journey is not decoration: it is the frame that must NOT extend. Hybrid meets it by
//! bottom-anchoring the command strip and filling between (`BottomPlan::Menu`), and
//! this mode cannot — the composite is one image built in the game's own coordinates,
//! so relocating the game's own chrome inside it is a composition change rather than a
//! layout one. The flank extension has declined the identical frame since SQ-0819 for
//! the identical reason. So Journey in `extended` is Journey in `raster`, to the byte,
//! and that is asserted rather than described.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::render::v6_layout as v6;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// One fixture: file, the key answered to a character read while tapping in, how many
/// taps reach the frame this case is about, the release it must be holding, and
/// whether this frame has anywhere to spend an extension.
struct Specimen {
    file: &'static str,
    /// The picture archive to mount, when the medium carries more than one — a
    /// Macintosh disk holds `Pic.data` (480x300, art_scale (1,1)) beside
    /// `CPic.data` (320x200 doubled), and they are two different renditions of the
    /// same release, not two spellings of one. `None` takes whatever the medium
    /// resolves on its own.
    pictures: Option<&'static str>,
    keys: u8,
    taps: usize,
    release: u16,
    /// Commands issued after the taps. Arthur takes one: he answers a blank line in
    /// a boxed window 3 across the screen's LAST text row, and one real command
    /// clears the box, which is also what a player does next. It used to be REQUIRED
    /// — a frame with the game's own chrome below its story window declined the
    /// extension (SQ-1008) — and since SQ-1132 that band travels down with the frame
    /// instead, so this now merely pins which of Arthur's two frames the corpus
    /// measures. `a_parser_error_does_not_resize_arthurs_extended_frame` measures the
    /// other one.
    then: &'static [&'static str],
    /// Does this frame EXTEND? False is the Journey case — a text-only command strip
    /// below the story window, which the composite cannot bottom-anchor.
    extends: bool,
}

const CORPUS: &[Specimen] = &[
    Specimen { file: "zork0-r393-s890714.z6", pictures: None, keys: 13, taps: 6, release: 393, then: &[], extends: true },
    Specimen { file: "arthur-r74-s890714.z6", pictures: None, keys: b'n', taps: 12, release: 74, then: &["look"], extends: true },
    Specimen { file: "journey-r83-s890706.z6", pictures: None, keys: 13, taps: 40, release: 83, then: &[], extends: false },
];

/// A pane with real surplus height at the 8x18 kitty cell: 800x900 device pixels
/// against a 640x400 screen, so the whole magnification is 1 and 500 native rows are
/// left over — 31 text rows at an 8x16 cell.
const TALL: (u16, u16) = (100, 50);
/// A pane with LESS than one text row of surplus at the same whole magnification:
/// 640x414 device pixels, so the extension is zero rows and the frame is the game's
/// screen exactly. The control the brief asks for — nothing extends, nothing changes.
const SNUG: (u16, u16) = (80, 23);
/// A pane wide enough for a whole magnification of TWO — 1280x1080 device pixels, so
/// the screen doubles and 140 native rows are left over, 8 text rows of them. Every
/// other pane in this suite pins `s = 1`, and a mode whose whole point is a whole
/// magnification should be measured on more than one of them.
const WIDE: (u16, u16) = (160, 60);
const CELL: (u16, u16) = (8, 18);

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot a v6 story the way `startup.rs` does — the profile from the medium the MOUNT
/// returned, and the screen size through the whole `picts.std_window() →
/// native_std_window → profile.std_window()` chain with `art_scale` beside it, all of
/// it inside one [`app::machine_boot::MachineBoot`] so a fact cannot be omitted — then
/// tap in to the frame. `None` (with a SKIP note) when the gitignored fixture is absent.
///
/// Skipping `native_std_window` is what booted two 560x384 presses at 640x400 and
/// fabricated a frame a whole quest was fixed against (CLAUDE.md, SQ-0901/SQ-1021), so
/// the profile, release, screen and cell this harness booted are all PRINTED.
fn boot(s: &Specimen) -> Option<Booted> {
    let mut b = boot_raw(s.file, s.release, s.pictures)?;
    tap_in(&mut b.session, s.keys, s.taps);
    for cmd in s.then {
        let r = b.session.submit(cmd);
        assert!(r.fault.is_none(), "{}: {cmd:?} faulted: {:?}", s.file, r.fault);
    }
    Some(b)
}

/// Answer whatever the game is waiting on, `taps` times, saying `n` to a `y or n`.
fn tap_in(session: &mut GameSession, keys: u8, taps: usize) {
    for _ in 0..taps {
        let t = match session.pending_input() {
            InputKind::Line => session.submit("").transcript,
            InputKind::Char => session.submit_char(keys).transcript,
            InputKind::Event => session.submit("").transcript,
        };
        if t.to_lowercase().contains("y or n") {
            let _ = session.submit_char(b'n');
        }
    }
}

/// Everything one boot settled, travelling together.
///
/// The CELL is the fact this exists for. It is the MACHINE's — 7x15 on a Macintosh
/// and 8x16 everywhere else (SQ-0917) — it reaches the render through
/// `AppState::v6_text`, and a harness that leaves that at its 8x16 default measures
/// a Macintosh screen on a grid no Macintosh has. That is the shape of SQ-1020 and
/// SQ-1021, so the face is resolved here, beside the screen and the art scale, and
/// handed to `state_for` as one value rather than remembered separately.
struct Booted {
    session: GameSession,
    /// The ARCHIVE's density (SQ-0790) — unit pixels per art pixel.
    art_scale: (u32, u32),
    /// The machine's cell and the release's own typeface, as `startup.rs` builds it.
    face: app::native_font::TextFace,
    /// Whether the artwork left the game's colours licensed (SQ-0806/SQ-0846): a
    /// two-colour archive declares the interpreter colourless.
    honoured: bool,
    screen_px: Option<(u16, u16)>,
    profile: InterpreterProfile,
}

/// The mount and boot alone, with no input answered — so a caller can measure the
/// SPLASH frame, which is what the game shows before anything is typed at it.
fn boot_raw(file: &str, want_release: u16, pictures: Option<&str>) -> Option<Booted> {
    let path = stories_dir().join(file);
    let (bytes, medium) = match app::hints::load_mounted_story(&path) {
        Ok((loaded, medium)) => (loaded.bytes().to_vec(), medium),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            return None;
        }
    };
    // The picture archive first: on a Macintosh disk its FLAVOUR refines the
    // profile and its own picture space is a screen-size link, so both have to be
    // settled before the engine is built (`startup.rs`, and
    // `v6_macintosh_profile::launch`).
    let dir = app::scratch_dir("sq1032");
    let over = match pictures {
        Some(name) => app::graphics::PictureOverride::resolve_with_session(&path, &dir, Some(name)),
        None => app::graphics::PictureOverride::Unset,
    };
    let named_art_std_window = over.std_window();
    let profile = InterpreterProfile::resolve(&path, None, over.flavour(), medium);
    app::v6_set_palette(profile.palette());
    let mut picts = PictSource::resolve_with_override(&path, over, None);
    let _ = std::fs::remove_dir_all(&dir);
    let dims = picts.all_pict_dims();
    let release = u16::from_be_bytes([bytes[2], bytes[3]]);
    assert_eq!(
        release, want_release,
        "{file}: a disk image is a different BUILD, not the same story on other media — this case \
         is pinned to release {want_release}"
    );
    // SQ-0806/SQ-0846: two-colour artwork declares the interpreter colourless — the
    // Macintosh B/W press is exactly that, and skipping this filter boots it with
    // colours no rendition of it ever had.
    let honoured = !picts.declines_game_colours(profile.default_colours());
    // The release's own typeface, resolved the way `startup.rs` resolves it.
    // `disks: None` so the answer cannot depend on what the person running this
    // keeps in `~/.lanthorn/`.
    let faces = app::native_font::resolve(&app::native_font::FaceRequest {
        story_path: &path,
        entry: None,
        profile,
        source: app::interpreter::ProfileSource::Medium,
        art_scale: picts.art_scale(),
        disks: None,
    });
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        named_art_std_window,
        profile.interpreter_number(),
        honoured.then(|| profile.default_colours()).flatten(),
        true,
        faces.clone(),
    );
    let art_scale = boot.art_scale;
    let face = app::native_font::TextFace::new(profile, faces, art_scale);
    eprintln!(
        "{file}: booted as {profile:?} off {medium:?} · release {release} · screen {:?} · \
         art_scale {art_scale:?} · v6 cell {:?} · face scale {:?} · colours {}",
        boot.screen_px,
        face.cell(),
        face.scale(),
        if honoured { "honoured" } else { "declined" },
    );
    let mut session =
        GameSession::new_for_machine(bytes, honoured, false, false, dims, None, None, &boot)
            .unwrap_or_else(|e| panic!("{file}: should boot without a ZError: {e:?}"));
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    Some(Booted {
        session,
        art_scale: art_scale.unwrap_or((2, 2)),
        face,
        honoured,
        screen_px: boot.screen_px,
        profile,
    })
}

/// A v6 app state at a real kitty cell, in `mode`, with the art scale the mount
/// resolved. Only the MODE differs between the two states any case here compares.
#[allow(deprecated)]
fn state_for(mode: app::config::V6RenderMode, transcript: &str, b: &Booted) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    let mut picker =
        ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(CELL.0, CELL.1));
    picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
    state.game_picker = Some(picker);
    state.config.v6_render = mode;
    state.config.honor_game_colours = b.honoured;
    // This suite's default assumption throughout is the whole-magnification
    // extended frame the file's own header describes — `AppState::default()`
    // otherwise leaves `v6_pixel_lock` at its `false` default, which pre-SQ-1239
    // made no difference here (extended always floored regardless). Now that the
    // flag is actually consulted, a case that wants the lock OFF overrides it
    // explicitly rather than relying on this default.
    state.config.v6_pixel_lock = true;
    state.v6_art_scale = b.art_scale;
    // The machine's own cell and the release's own face (SQ-0917, SQ-1009).
    // Leaving this at `AppState::default()`'s 8x16 is how a Macintosh press gets
    // measured on a grid no Macintosh has.
    state.v6_text = b.face.clone();
    for line in transcript.lines() {
        state.push_transcript(line);
    }
    state
}

/// The pane in device pixels at [`CELL`] — the unit `RasterFrame::extended` is stated in.
fn pane_dev(pane: (u16, u16)) -> (u32, u32) {
    (u32::from(pane.0) * u32::from(CELL.0), u32::from(pane.1) * u32::from(CELL.1))
}

/// The two composites one specimen builds at `pane`: the game's own screen
/// (`raster`), and the frame `extended` asks for. Both come out of the same
/// `build_v6_raster_frame`, so a difference between them is the extension and
/// nothing else.
type Pair = (
    (image::RgbaImage, Option<app::render::screen::RasterMetrics>, v6::RasterFrame),
    (image::RgbaImage, Option<app::render::screen::RasterMetrics>, v6::RasterFrame),
);

fn pair(b: &mut Booted, pane: (u16, u16)) -> Pair {
    let transcript = b.session.take_transcript();
    let model = b.session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let plain = state_for(app::config::V6RenderMode::Raster, &transcript, b);
    let ext = state_for(app::config::V6RenderMode::Extended, &transcript, b);
    let native = v6::native_extent(items, &plain.v6_text);
    let layout = v6::classify_windows(items, plain.v6_text.cell());
    let cell = plain.v6_text.cell();
    let want = v6::RasterFrame::extended(native, pane_dev(pane), cell, Some(2.0), ext.config.v6_pixel_lock);
    (
        app::render::screen::build_v6_raster_frame(&layout, v6::RasterFrame::native(native), &plain),
        app::render::screen::build_v6_raster_frame(&layout, want, &ext),
    )
}

// ── 1. The arithmetic ─────────────────────────────────────────────────────────

/// A whole magnification and a surplus measured in whole text rows of the MACHINE's
/// own cell.
///
/// Stated over both cells the corpus has — 8x16 everywhere and 7x15 on a Macintosh
/// (SQ-0917) — because "whole text rows" is a question about the cell and writing it
/// `/ 16` is the shape of SQ-1020.
#[test]
fn the_extension_is_a_whole_magnification_and_whole_text_rows() {
    for cell in [zvm::screen::V6Cell::new(8, 16), zvm::screen::V6Cell::new(7, 15)] {
        let f = v6::RasterFrame::extended((640, 400), (800, 900), cell, Some(2.0), true);
        let s = f.lock.expect("a pane that holds the screen at 1:1 pins a magnification");
        assert_eq!(s, s.floor(), "cell {cell:?}: the magnification is a whole number");
        assert!(s >= 1.0, "cell {cell:?}: and never a minification");
        assert_eq!(
            f.extension() % u32::from(cell.h()),
            0,
            "cell {cell:?}: the surplus is whole text rows of this machine's cell"
        );
        assert_eq!(f.native, (640, 400), "cell {cell:?}: the GAME's screen is never changed");
        // 900 device rows at s=1 is 900 native rows; 500 of them lie below the screen.
        assert_eq!(f.extension(), 500 / u32::from(cell.h()) * u32::from(cell.h()));

        // …and the rung above it. 1280x1080 doubles the screen, so the pane is 540
        // native rows and 140 of them lie below it.
        let two = v6::RasterFrame::extended((640, 400), pane_dev(WIDE), cell, Some(2.0), true);
        assert_eq!(two.lock, Some(2.0), "cell {cell:?}: a pane twice the screen pins 2");
        assert_eq!(two.extension(), 140 / u32::from(cell.h()) * u32::from(cell.h()));
    }
}

/// A pane with less than one text row of surplus extends by nothing, and a pane that
/// cannot hold the game's screen at 1:1 falls all the way back to the plain letterbox
/// — no lock, no extension, which is `raster` exactly.
#[test]
fn a_pane_with_no_surplus_is_the_plain_letterboxed_frame() {
    let cell = zvm::screen::V6Cell::new(8, 16);
    let snug = v6::RasterFrame::extended((640, 400), pane_dev(SNUG), cell, Some(2.0), true);
    assert_eq!(snug.extension(), 0, "640x414 leaves 14 native rows — under one text row");
    assert_eq!(snug.canvas_h, 400);

    let small = v6::RasterFrame::extended((640, 400), (500, 300), cell, Some(2.0), true);
    assert_eq!(small, v6::RasterFrame::native((640, 400)), "below 1:1 there is no whole rung");
    assert_eq!(small.lock, None, "…so the composite keeps the fitted letterbox");
}

// ── 1b. The lock is a switch here too (SQ-1239) ────────────────────────────────

/// The pane the next two cases fit Zork Zero's 640x400 screen against: 920x720
/// device pixels at this suite's [`CELL`] (8x18), so the free letterbox factor is
/// `min(920/640, 720/400) = min(1.4375, 1.8) = 1.4375` — fractional, with the width
/// the binding edge. This file's upscale cap (`Some(2.0)`, matching
/// `v6_upscale_cap`'s kitty answer) never binds here since 1.4375 < 2.0.
const FRACTIONAL: (u16, u16) = (115, 40);

/// `set-v6-pixel-lock` OFF must ask `extended` for the same free/fractional scale
/// `Raster`/`Hybrid` draw at (`FrameGeometry::fitted_scale` with `lock: false` — the
/// call `build_hybrid_frame` makes at `screen.rs:3434`), not the whole-rung
/// magnification the toggle was supposed to turn off. ON keeps today's behaviour.
///
/// FALSIFY by reverting `RasterFrame::extended` to always `.floor()` (its shape
/// before SQ-1239): the OFF assertions below fail, reporting `1.0` — the quantized
/// rung — where the pane's free scale is `1.4375`.
#[test]
fn the_pixel_lock_is_a_switch_in_extended_mode_too() {
    let _g = app::v6_palette_at_boot();
    let Some(mut b) = boot(&CORPUS[0]) else { return };

    let model = b.session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let cell = b.face.cell();
    let native = v6::native_extent(items, &b.face);
    assert_eq!(native, (640, 400), "zork0-r393: this case's pane math assumes the full v6 screen");

    // The reference: the SAME free-scale call `Raster`/`Hybrid` make, at the SAME pane.
    let free = v6::FrameGeometry::new(native, b.art_scale, cell).fitted_scale(pane_dev(FRACTIONAL), false).0.s;
    assert_eq!(free, 1.4375, "the pane's free letterbox factor, unquantized");

    let (_, _, off) = extended_with(&mut b, "", FRACTIONAL, false);
    let s_off = off.lock.expect("a pane holding the screen at 1:1 pins a magnification");
    assert_eq!(s_off, free, "lock OFF: extended reports the same unquantized scale hybrid/raster do");
    assert_ne!(s_off, s_off.floor(), "…and it really is fractional, not accidentally whole");

    let (_, _, on) = extended_with(&mut b, "", FRACTIONAL, true);
    let s_on = on.lock.expect("a pane holding the screen at 1:1 pins a magnification");
    assert_eq!(s_on, s_on.floor(), "lock ON: extended still pins a whole rung");
    assert_eq!(s_on, free.floor(), "…here, the floor of the same free scale");
}

/// `v6_pixel_lock` is read live, not once at boot: the same pane, the same frame,
/// flipping OFF → ON → OFF within one session must answer differently each time it
/// changes and settle back exactly where it started (SQ-1239).
#[test]
fn the_pixel_lock_toggle_flips_extended_geometry_live() {
    let _g = app::v6_palette_at_boot();
    let Some(mut b) = boot(&CORPUS[0]) else { return };

    let mut scales = Vec::new();
    for lock in [false, true, false] {
        let (_, _, f) = extended_with(&mut b, "", FRACTIONAL, lock);
        let s = f.lock.expect("a pane holding the screen at 1:1 pins a magnification");
        scales.push(s);
    }
    assert_eq!(scales[0], 1.4375, "OFF: the free/fractional scale");
    assert_eq!(scales[1], 1.0, "ON: the whole rung below it");
    assert_ne!(scales[0], scales[1], "the toggle must change the geometry, not just be accepted");
    assert_eq!(scales[0], scales[2], "…and flipping back OFF returns to the same free scale");
}

// ── 2..4. The frame the corpus actually builds ────────────────────────────────

/// On a title with surplus pane: the canvas grows DOWNWARD only, the extra rows reach
/// the prose box, and the flanks are carried into them rather than left as bare page.
///
/// FALSIFY by dropping the `th + extension` line in `build_v6_raster_frame` — the
/// canvas still grows and the flanks still tile, and the story viewport does not move,
/// which is the "taller frame, same eleven rows of prose" version of this mode.
#[test]
fn the_extension_grows_downward_and_the_prose_box_takes_it() {
    let _g = app::v6_palette_at_boot();
    let mut seen = 0usize;
    let mut any_present = false;
    for spec in CORPUS.iter().filter(|s| s.extends) {
        any_present |= stories_dir().join(spec.file).exists();
        let Some(mut b) = boot(spec) else { continue };
        for pane in [TALL, WIDE] {
        let ((plain, pm, pf), (extended, em, ef)) = pair(&mut b, pane);
        eprintln!(
            "{}: at {pane:?} raster {}x{} → extended {}x{} (lock {:?})",
            spec.file,
            plain.width(),
            plain.height(),
            extended.width(),
            extended.height(),
            ef.lock,
        );

        // Non-vacuity: this case is about a frame that HAS a prose box and DID extend.
        assert!(pm.is_some(), "{}: the raster frame has a story box to compare against", spec.file);
        assert!(ef.extension() > 0, "{}: this frame is supposed to extend", spec.file);
        assert_eq!(pf.extension(), 0, "{}: and the raster frame is supposed not to", spec.file);

        // (2) Downward only. The width is the game's screen — that is the whole of
        // "fixed raster width" — and the height is the game's screen plus the extension.
        assert_eq!(extended.width(), plain.width(), "{}: the frame never grows sideways", spec.file);
        assert_eq!(
            extended.height(),
            plain.height() + ef.extension(),
            "{}: and grows by exactly the extension",
            spec.file
        );

        // (3) The rows reach the PROSE, which is the point of having them. The pager
        // pages on added-rows > viewport (`pager.rs`), and this is that viewport.
        let (pv, ev) = (pm.expect("checked").viewport_rows, em.expect("extended keeps a story box").viewport_rows);
        assert!(
            ev > pv,
            "{}: the extension must reach the story viewport — raster {pv} rows, extended {ev}",
            spec.file
        );
        assert_eq!(
            u32::from(ev - pv),
            ef.extension() / u32::from(app::state::AppState::default().v6_text.cell().h()),
            "{}: and it is exactly the whole text rows the frame added",
            spec.file
        );

        // (4) The flanks were carried down. Every pixel below the game's screen is
        // opaque (the composite is self-contained — SQ-0510), and the outermost column
        // of the extension is not merely the story page: the border art tiled into it.
        let below = u32::from(ef.native.1);
        assert!(below < extended.height(), "{}: there is an extension to look at", spec.file);
        assert!(
            (below..extended.height()).all(|y| (0..extended.width()).all(|x| extended.get_pixel(x, y)[3] == 255)),
            "{}: the extension ships opaque, like every other pixel of the composite",
            spec.file
        );
        // The BORDER, specifically. The middle of the extension's last row is the
        // story page — nothing is drawn there but prose — and each outer eighth of
        // the frame must carry something that is not it, which is the side artwork
        // tiled down by `flank_source` at the taller target. Stated as an eighth
        // rather than as column 0 because a flank's outermost columns are not
        // necessarily painted: Zork Zero's pillars start inboard of the screen edge.
        let page = extended.get_pixel(extended.width() / 2, extended.height() - 1).0;
        let eighth = (extended.width() / 8).max(1);
        for (label, cols) in [
            ("left", 0..eighth),
            ("right", extended.width() - eighth..extended.width()),
        ] {
            let inked = (below..extended.height())
                .flat_map(|y| cols.clone().map(move |x| (x, y)))
                .any(|(x, y)| extended.get_pixel(x, y).0 != page);
            assert!(
                inked,
                "{}: the {label} border must reach the bottom of the extension rather than \
                 stopping at the game's own screen (page {page:?})",
                spec.file
            );
        }
        seen += 1;
        }
    }
    if any_present {
        assert!(seen > 0, "a present fixture must have been measured, not skipped");
    }
}

// ── 5. The regression guard ──────────────────────────────────────────────────

/// A frame with nowhere to put the extra rows declines it, and the composite it
/// builds is **byte-identical** to the one `raster` builds.
///
/// Journey is the frame: a text-only command strip below the story window, which
/// hybrid bottom-anchors and this mode cannot. The SNUG pane is the other half — every
/// title, including the two that do extend, at a pane with no surplus to spend.
///
/// This is the guard that matters. `extended` shares the entire composite with
/// `raster`, so a change that leaked out of the extension's own branches lands here.
#[test]
fn a_frame_that_declines_the_extension_is_byte_identical_to_raster() {
    let _g = app::v6_palette_at_boot();
    let mut seen = 0usize;
    let mut any_present = false;
    for spec in CORPUS {
        any_present |= stories_dir().join(spec.file).exists();
        // Journey declines at any pane; the others decline at a pane with no surplus.
        let panes: &[(u16, u16)] = if spec.extends { &[SNUG] } else { &[SNUG, TALL] };
        for &pane in panes {
            let Some(mut b) = boot(spec) else { continue };
            let ((plain, _, pf), (extended, _, ef)) = pair(&mut b, pane);
            assert_eq!(
                ef.extension(),
                0,
                "{} at {pane:?}: this frame must decline the extension",
                spec.file
            );
            assert_eq!(ef, pf, "{} at {pane:?}: a declined frame IS the raster frame", spec.file);
            assert_eq!(
                extended.dimensions(),
                plain.dimensions(),
                "{} at {pane:?}: same canvas",
                spec.file
            );
            assert!(
                extended.as_raw() == plain.as_raw(),
                "{} at {pane:?}: a declined extension must not move one byte of the raster \
                 composite",
                spec.file
            );
            seen += 1;
        }
    }
    if any_present {
        assert!(seen > 0, "a present fixture must have been measured, not skipped");
    }
}

/// The whole-pane render agrees with the canvas: in `extended` the story pane reports
/// more viewport rows than in `raster`, through the REAL render entry rather than the
/// canvas builder — so the mode is wired to the frame path and not only to the helper.
#[test]
fn the_render_path_reports_the_larger_viewport() {
    let _g = app::v6_palette_at_boot();
    let mut seen = 0usize;
    let mut any_present = false;
    for spec in CORPUS.iter().filter(|s| s.extends) {
        any_present |= stories_dir().join(spec.file).exists();
        let Some(mut b) = boot(spec) else { continue };
        let transcript = b.session.take_transcript();
        let model = b.session.screen();
        let area = Rect::new(0, 0, TALL.0, TALL.1);
        let rows = |mode| {
            let state = state_for(mode, &transcript, &b);
            let mut buf = Buffer::empty(area);
            app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf)
                .viewport_rows
        };
        let plain = rows(app::config::V6RenderMode::Raster);
        let ext = rows(app::config::V6RenderMode::Extended);
        eprintln!("{}: raster {plain} viewport rows → extended {ext}", spec.file);
        assert!(plain > 0, "{}: the raster path reports a story viewport at all", spec.file);
        assert!(ext > plain, "{}: extended must report more of one ({ext} vs {plain})", spec.file);
        seen += 1;
    }
    if any_present {
        assert!(seen > 0, "a present fixture must have been measured, not skipped");
    }
}

// ── 6. The other v6 SHAPES: splash cards and hint screens ─────────────────────
//
// The corpus above is gameplay. A v6 title has two other shapes, and both were
// asked for by name: the SPLASH card the game shows before anything is typed at
// it, and the HINT screens — a menu of topics, a page of clues, and (Arthur) a
// boxed window the game opens under its story. They are covered here because the
// answer must be DELIBERATE: a splash that grew a prose region and tiled flanks
// around a title card would look wrong, and a hint menu is driven by CLICKS, so a
// frame that both extended and was clickable is where a wrong click map would be
// felt (see `the_click_map_drops_a_click_in_the_rows_lanthorn_added`).

/// How the harness reaches one frame.
#[derive(Clone, Copy, Debug)]
enum Reach {
    /// Nothing answered: the first frame the game paints after boot. For Shogun and
    /// Journey that IS the title card; for Zork Zero and Arthur it is not, which is
    /// why the real splashes have rows of their own below.
    BootFrame,
    /// Zork Zero's *"The Revenge of Megaboz"* title splash, which is **not** the boot
    /// frame: it arrives after the prologue cutscene, when `@split_window(400)` grows
    /// window 1 to the whole screen and window 0 collapses (SQ-0497). Driven the way
    /// `v6_zork0_splash::drive_to_splash` drives it, and parked on its keypress.
    Zork0TitleSplash,
    /// `taps` answers in, for a frame that is neither the boot frame nor a hint
    /// screen — Journey's title BLOCK, which it prints as text into a full-screen
    /// window 0 before it opens its panels (SQ-0755's frame).
    Play { keys: u8, taps: usize },
    /// Clear the intro, then `hint` + `y` — the topic MENU (`v6_zork0_hints`,
    /// `v6_hint_menu_mouse`).
    HintMenu { taps: usize },
    /// …and four Returns further in: a page of CLUE text (`v6_hint_clue_wrap`).
    HintClues { taps: usize },
    /// Arthur's crystal ball, reached by getting arrested — a text-only menu with
    /// no artwork behind it at all (`v6_arthur_hint_page`).
    ArthurHintPage,
    /// Arthur's boxed answer window, which he opens on the LAST text row of the
    /// screen, BELOW window 0 (`v6_arthur_hint_box`, and the frame SQ-1008 was
    /// about).
    ArthurHintBox,
}

/// One v6 shape, and the verdict the extension must reach on it.
struct Shape {
    file: &'static str,
    pictures: Option<&'static str>,
    release: u16,
    reach: Reach,
    /// Does this frame extend? Pinned, so a frame that changes shape says so
    /// rather than flipping the mode's behaviour silently.
    extends: bool,
    /// Why that is the right answer for this shape.
    why: &'static str,
}

const SHAPES: &[Shape] = &[
    // Splash cards. Every one is a full-screen plate or a picture takeover, so
    // there is no prose box to grow — the frame must decline and look exactly as
    // `raster` draws it.
    Shape { file: "zork0-r393-s890714.z6", pictures: None, release: 393, reach: Reach::Zork0TitleSplash, extends: false,
            why: "the title splash: window 1 IS the screen and window 0 collapses, so there is no \
                  story window at all (SQ-0497)" },
    Shape { file: "zork0-r393-s890714.z6", pictures: None, release: 393, reach: Reach::BootFrame, extends: true,
            why: "not a splash: the PROLOGUE, an ordinary framed gameplay screen" },
    Shape { file: "arthur-r74-s890714.z6", pictures: None, release: 74, reach: Reach::BootFrame, extends: false,
            why: "the intro plate, absolutely placed in window 0 and drawn INSTEAD of prose \
                  (SQ-0707)" },
    Shape { file: "shogun-r322-s890706.z6", pictures: None, release: 322, reach: Reach::BootFrame, extends: false,
            why: "title card" },
    // Journey's boot frame is NOT its title card — it is the blank full-screen
    // window 0 the game publishes before it paints anything into it, so there is a
    // prose box and it extends. Nothing is on either screen; the extension makes the
    // empty page taller, which is what extending an empty text window means.
    Shape { file: "journey-r83-s890706.z6", pictures: None, release: 83, reach: Reach::BootFrame, extends: true,
            why: "a blank full-screen window 0, before the title is painted into it" },
    // One tap on: the title PLATE, and a picture that owns the screen leaves no prose
    // box (SQ-0707) — the shape this quest most needed to decline, and it does.
    Shape { file: "journey-r83-s890706.z6", pictures: None, release: 83, reach: Reach::Play { keys: 13, taps: 1 }, extends: false,
            why: "the title plate owns the screen: no prose box (SQ-0707)" },
    // Three taps: the panels are open and the command menu is under the story.
    Shape { file: "journey-r83-s890706.z6", pictures: None, release: 83, reach: Reach::Play { keys: 13, taps: 3 }, extends: false,
            why: "the command menu sits below window 0 (SQ-0819) — hybrid bottom-anchors \
                  it and the composite cannot" },
    // Hint screens.
    Shape { file: "zork0-r393-s890714.z6", pictures: None, release: 393, reach: Reach::HintMenu { taps: 8 }, extends: false,
            why: "InvisiClues: the buffer withdraws and a Grid is the story surface (SQ-1026)" },
    Shape { file: "shogun-r322-s890706.z6", pictures: None, release: 322, reach: Reach::HintMenu { taps: 8 }, extends: false,
            why: "topic menu" },
    Shape { file: "James Clavell's Shogun.adf", pictures: None, release: 295, reach: Reach::HintClues { taps: 14 }, extends: false,
            why: "clue page" },
    Shape { file: "Arthur - The Quest for Excalibur.adf", pictures: None, release: 54, reach: Reach::ArthurHintPage, extends: false,
            why: "the crystal ball's text-only menu" },
    Shape { file: "Arthur - The Quest for Excalibur.adf", pictures: None, release: 54, reach: Reach::ArthurHintBox, extends: true,
            why: "window 3 across native (28,384) 584x16 — the screen's LAST text row, BELOW \
                  window 0, which `menu_strip_below_story` cannot see (SQ-1008). It used to \
                  decline; the box now TRAVELS with the frame's bottom edge (SQ-1132), because \
                  a band that comes and goes with the turn was resizing the whole screen." },
];

/// What `classify_windows` made of this frame's story slot — the fact every decline
/// below turns on, printed so the table reads as a measurement.
fn story_shape(b: &Booted) -> String {
    let model = b.session.screen();
    let WinNode::Layered(items) = &model.root else { return "not layered".into() };
    let layout = v6::classify_windows(items, b.face.cell());
    match layout.story {
        None => "none".into(),
        Some(pw) => format!(
            "{} ({},{}) {}x{}",
            match &pw.node {
                WinNode::Buffer(b) if b.primary => "Buffer",
                WinNode::Buffer(_) => "Buffer(secondary)",
                WinNode::Grid(_) => "Grid",
                _ => "other",
            },
            pw.x_px, pw.y_px, pw.w_px, pw.h_px
        ),
    }
}

fn reach(sh: &Shape) -> Option<Booted> {
    let mut b = boot_raw(sh.file, sh.release, sh.pictures)?;
    let s = &mut b.session;
    match sh.reach {
        Reach::BootFrame => {}
        Reach::Play { keys, taps } => tap_in(s, keys, taps),
        Reach::Zork0TitleSplash => {
            let mut lines = ["get under table", "wait", "wait", "wait", "wait", "wait"].into_iter();
            let mut parked = false;
            for _ in 0..16 {
                match s.pending_input() {
                    InputKind::Line => {
                        s.submit(lines.next().unwrap_or("wait"));
                    }
                    InputKind::Char => {
                        s.submit_char(13);
                    }
                    InputKind::Event => {
                        s.submit("");
                    }
                }
                let win1 = match &s.screen().root {
                    WinNode::Layered(items) => items
                        .iter()
                        .find_map(|pw| matches!(&pw.node, WinNode::Graphics(g) if g.win == 1).then_some(pw.h_px))
                        .unwrap_or(0),
                    _ => 0,
                };
                if win1 > 300 && s.pending_input() == InputKind::Char {
                    parked = true;
                    break;
                }
            }
            assert!(parked, "{}: never reached the ZORK ZERO title splash", sh.file);
        }
        Reach::HintMenu { taps } | Reach::HintClues { taps } => {
            // Zork Zero asks for a LINE first; Shogun holds its title on a CHAR
            // read. Answer whatever is in the way rather than assuming either —
            // the same loop `v6_hint_menu_mouse::hint_menu` uses.
            for _ in 0..taps {
                match s.pending_input() {
                    InputKind::Line => break,
                    InputKind::Char => {
                        let _ = s.submit_char(13);
                    }
                    InputKind::Event => {
                        let _ = s.submit("");
                    }
                }
            }
            s.submit("hint");
            let entered = s.submit_char(b'y');
            assert!(entered.fault.is_none(), "{}: entering the hint menu faulted", sh.file);
            if matches!(sh.reach, Reach::HintClues { .. }) {
                for _ in 0..4 {
                    let _ = s.submit_char(13);
                }
            }
        }
        Reach::ArthurHintPage => {
            tap_in(s, 13, 14);
            // Out through the gate, into the church after curfew: arrested, which
            // is the death prompt that offers HINT (`v6_arthur_hint_page`).
            for cmd in ["open gate", "e", "hint", "hint", ""] {
                let r = s.submit(cmd);
                assert!(r.fault.is_none(), "{}: {cmd:?} faulted", sh.file);
            }
            if s.pending_input() == InputKind::Char {
                let _ = s.submit_char(13);
            }
        }
        Reach::ArthurHintBox => {
            tap_in(s, 13, 14);
            let r = s.submit("hint");
            assert!(r.fault.is_none(), "{}: `hint` faulted", sh.file);
        }
    }
    Some(b)
}

/// Every splash card and every hint screen reaches the verdict its row pins, and a
/// frame that declines is byte-identical to `raster`.
///
/// The table's `extends` column is a MEASUREMENT, not a caption: the case prints
/// what each frame actually did, so a shape that drifts is visible in the output
/// before it is a failure.
#[test]
fn splash_cards_and_hint_screens_reach_the_verdict_their_row_pins() {
    let _g = app::v6_palette_at_boot();
    let mut seen = 0usize;
    let mut any_present = false;
    for sh in SHAPES {
        any_present |= stories_dir().join(sh.file).exists();
        let Some(mut b) = reach(sh) else { continue };
        let story = story_shape(&b);
        let ((plain, pm, _), (extended, em, ef)) = pair(&mut b, TALL);
        eprintln!(
            "  SHAPE {:<38} {:?} → {} (canvas {}x{}, story {story}, prose box {}) — {}",
            sh.file,
            sh.reach,
            if ef.extension() > 0 { "EXTENDS" } else { "declines" },
            extended.width(),
            extended.height(),
            if pm.is_some() { "yes" } else { "none" },
            sh.why,
        );
        if let Ok(dir) = std::env::var("LANTHORN_SHOT_DIR") {
            let tag = format!("{}-{:?}", sh.file.replace(' ', "_"), sh.reach);
            extended.save(format!("{dir}/ext-{tag}.png")).unwrap();
            plain.save(format!("{dir}/ras-{tag}.png")).unwrap();
        }
        assert_eq!(
            ef.extension() > 0,
            sh.extends,
            "{} {:?}: expected {} — {}",
            sh.file,
            sh.reach,
            if sh.extends { "to extend" } else { "to decline" },
            sh.why
        );
        if sh.extends {
            // An extending frame must have somewhere to put the rows, or the
            // extension is bare page under a title card.
            let ev = em.expect("an extending frame keeps a story box").viewport_rows;
            let pv = pm.expect("…and so does the raster frame it grew from").viewport_rows;
            assert!(ev > pv, "{}: {ev} rows against {pv}", sh.file);
        } else {
            assert!(
                extended.as_raw() == plain.as_raw(),
                "{} {:?}: a declined frame must be the raster composite to the byte",
                sh.file,
                sh.reach
            );
        }
        seen += 1;
    }
    if any_present {
        assert!(seen > 0, "a present fixture must have been measured, not skipped");
    }
}

// ── 6b. The frame does not change size with the turn ─────────────────────────

/// The extended composite for the frame the session is parked on, with `transcript`
/// as the host's scrollback.
///
/// `""` leaves the prose region blank, so every inked pixel below the status bar is
/// the GAME's own chrome and two frames can be compared without the transcript — a
/// thing that legitimately differs between them — drowning the comparison.
/// `lock` overrides `state_for`'s default (`v6_pixel_lock = true`) so a caller can
/// ask for the free/fractional scale explicitly (SQ-1239).
fn extended_with(
    b: &mut Booted,
    transcript: &str,
    pane: (u16, u16),
    lock: bool,
) -> (image::RgbaImage, Option<app::render::screen::RasterMetrics>, v6::RasterFrame) {
    let mut st = state_for(app::config::V6RenderMode::Extended, transcript, b);
    st.config.v6_pixel_lock = lock;
    let model = b.session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &st.v6_text);
    let layout = v6::classify_windows(items, st.v6_text.cell());
    let want = v6::RasterFrame::extended(native, pane_dev(pane), st.v6_text.cell(), Some(2.0), st.config.v6_pixel_lock);
    app::render::screen::build_v6_raster_frame(&layout, want, &st)
}

/// Whatever the game has printed BELOW its story window on this frame, joined — the
/// band the whole case turns on, read straight off the model so a frame that stops
/// producing one fails as a vacuous case rather than passing silently.
fn band_text(b: &mut Booted) -> String {
    let cell = b.face.cell();
    let model = b.session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let layout = v6::classify_windows(items, cell);
    let Some(story) = layout.story else { return String::new() };
    let bottom = i32::from(story.y_px) + i32::from(story.h_px);
    let mut out = String::new();
    for w in &layout.chrome {
        let WinNode::Grid(g) = &w.node else { continue };
        for t in &g.px_texts {
            if i32::from(t.y.max(1)) > bottom {
                out.push_str(&t.text);
            }
        }
    }
    out
}

/// **A turn the parser rejects must not change the size of the frame** (SQ-1132).
///
/// Reported from play: in `extended`, typing something Arthur does not understand
/// collapsed the screen back to the plain letterbox, and typing something he did
/// understand grew it again — a frame height that tracked whether the last command
/// parsed.
///
/// The mechanism is entirely the game's own bookkeeping. Arthur prints his parser
/// errors into window 3, laid across native (28, 384) 584x16 — the LAST text row of a
/// 640x400 screen — and shrinks window 0 from 584x192 to 584x176 to make room for it.
/// So a rejected turn is a frame with the game's own chrome below its story window,
/// which `build_v6_raster_frame` used to answer by declining the extension outright
/// (SQ-1008's reading: the composite cannot bottom-anchor anything). The band now
/// travels DOWN with the frame's bottom edge, which is where the extension's own
/// arithmetic already leaves room for it.
///
/// Fixture: `arthur-r74-s890714.z6`, release 74, twelve taps answering `n` to the
/// restore question, then `look` (the clean frame) and `frobozzle the grue` (the
/// rejected one). Checked against `machine-screenshots/amiga-arthur.png`, where the
/// prose runs to the bottom edge of the frame with no band under it — which is what
/// the clean turn must go on looking like.
///
/// FALSIFY by restoring the `break 'ext None` under `menu_band_rows(…) > 0` in
/// `build_v6_raster_frame`: the rejected turn comes back as a 640x400 canvas against
/// the 640x896 `look` builds, which is the collapse as reported.
#[test]
fn a_parser_error_does_not_resize_arthurs_extended_frame() {
    let _g = app::v6_palette_at_boot();
    let spec = Specimen {
        file: "arthur-r74-s890714.z6",
        pictures: None,
        keys: b'n',
        taps: 12,
        release: 74,
        then: &["look"],
        extends: true,
    };
    let Some(mut b) = boot(&spec) else { return };
    let cell = u32::from(b.face.cell().h());

    // The clean turn: `look` parsed, window 3 is empty, and the frame extends.
    let clean_band = band_text(&mut b);
    let (clean, cm, cf) = extended_with(&mut b, "", TALL, true);
    assert!(cf.extension() > 0, "the clean frame is supposed to extend");
    assert!(
        clean_band.trim().is_empty(),
        "a turn Arthur understood must leave nothing below window 0, not {clean_band:?}"
    );

    // …then a word he does not know.
    let r = b.session.submit("frobozzle the grue");
    assert!(r.fault.is_none(), "the rejected command faulted: {:?}", r.fault);
    let error_band = band_text(&mut b);
    let (error, em, ef) = extended_with(&mut b, "", TALL, true);
    eprintln!(
        "clean {}x{} ({} viewport rows) → rejected {}x{} ({} viewport rows), band {error_band:?}",
        clean.width(),
        clean.height(),
        cm.map_or(0, |m| m.viewport_rows),
        error.width(),
        error.height(),
        em.map_or(0, |m| m.viewport_rows),
    );

    // Non-vacuity: this case is only about a frame the game printed a parser error
    // onto, BELOW its story window. Without this it passes on a frame that never
    // produced one.
    assert!(
        error_band.contains("frobozzle"),
        "the rejected turn must put Arthur's parser error below window 0 — got {error_band:?}"
    );

    // The report itself: same frame, same size.
    assert_eq!(
        error.dimensions(),
        clean.dimensions(),
        "a rejected command must not resize the frame"
    );
    assert_eq!(ef.extension(), cf.extension(), "…nor change what the extension is");

    // And the message is at the frame's BOTTOM EDGE, not stranded in the middle of
    // the prose where the game's own screen ends. With no transcript the two
    // composites are the same picture apart from that one row, so the rows that
    // differ ARE the message — and there must be some.
    let h = clean.height();
    let differs: Vec<u32> = (0..h)
        .filter(|&y| (0..clean.width()).any(|x| clean.get_pixel(x, y) != error.get_pixel(x, y)))
        .collect();
    assert!(!differs.is_empty(), "the parser error has to reach the composite at all");
    assert!(
        differs.iter().all(|&y| y >= h.saturating_sub(cell)),
        "the parser error belongs on the frame's last text row ({}..{h}), not at {differs:?} — \
         the game's own screen ends at native {}",
        h.saturating_sub(cell),
        ef.native.1,
    );
}

/// The native rows the game's own band below the story window spans on this frame —
/// 0 with nothing down there, 1 for a parser message that fits, 2 for one that wraps.
/// The quantity the whole of SQ-1157 turns on, so it is measured rather than assumed.
fn band_rows(b: &mut Booted) -> u16 {
    let cell = u32::from(b.face.cell().h().max(1));
    let model = b.session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let layout = v6::classify_windows(items, b.face.cell());
    let Some(story) = layout.story else { return 0 };
    let bottom = u32::from(story.y_px) + u32::from(story.h_px);
    let rows: Vec<u32> = layout
        .chrome
        .iter()
        .filter_map(|w| match &w.node {
            WinNode::Grid(g) => Some(g.px_texts.iter()),
            _ => None,
        })
        .flatten()
        .map(|t| u32::from(t.y.max(1)) - 1)
        .filter(|&y| y >= bottom)
        .map(|y| y / cell)
        .collect();
    match (rows.iter().min(), rows.iter().max()) {
        (Some(&a), Some(&z)) => (z - a + 1) as u16,
        _ => 0,
    }
}

/// The shape of one frame, in whichever mode: what the ART is drawn at, how tall the
/// composite is, and how many rows of prose the player gets.
///
/// The three travel together because the whole claim is about the first two NOT
/// moving while the third does — reading them from separate calls is how a case ends
/// up comparing a magnification from one turn against a viewport from another.
#[derive(Debug, PartialEq)]
struct FrameShape {
    /// Hybrid: the ring plan. Raster and extended: `"raster"`.
    plan: String,
    /// Hybrid: the magnification the ring's art is drawn at. Raster and extended: the
    /// composite's own lock.
    scale: String,
    /// The composite's height in native pixels; the pane's height in cells in hybrid,
    /// where there is no composite.
    canvas_h: u32,
    /// The prose viewport, in terminal rows.
    viewport: u16,
}

fn shape(b: &mut Booted, mode: app::config::V6RenderMode, pane: (u16, u16)) -> FrameShape {
    let st = state_for(mode, "", b);
    let model = b.session.screen();
    let area = Rect::new(0, 0, pane.0, pane.1);
    let mut buf = Buffer::empty(area);
    let m = app::render::screen::render_story_pane(&model, false, None, &st, area, &mut buf);
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &st.v6_text);
    let layout = v6::classify_windows(items, st.v6_text.cell());
    let want = match mode {
        app::config::V6RenderMode::Extended => {
            v6::RasterFrame::extended(native, pane_dev(pane), st.v6_text.cell(), Some(2.0), st.config.v6_pixel_lock)
        }
        _ => v6::RasterFrame::native(native),
    };
    let (canvas, _, built) = app::render::screen::build_v6_raster_frame(&layout, want, &st);
    match mode {
        app::config::V6RenderMode::Hybrid => FrameShape {
            plan: st.v6_ring_plan.get().to_string(),
            scale: format!("{:?}", st.v6_image_scale.get()),
            canvas_h: u32::from(pane.1),
            viewport: m.viewport_rows,
        },
        _ => FrameShape {
            plan: "raster".into(),
            scale: format!("{:?}", built.lock),
            canvas_h: canvas.height(),
            viewport: m.viewport_rows,
        },
    }
}

/// **A parser message that WRAPS TO TWO ROWS must not change the frame either**
/// (SQ-1157) — in ANY of the three modes.
///
/// Reported from play on Arthur: type `was`, and the parser answers *"Sorry, but I
/// don't understand. Please rephrase that, or try something else."* — long enough
/// that his bottom status line wraps. Hybrid's side art jumped to a different size
/// over the ring, and `extended` dropped back to the plain letterbox, both until the
/// next command. Raster looked right, which is the tell: raster is what the other two
/// were collapsing to.
///
/// MEASURED, `arthur-r74-s890714.z6` release 74, twelve taps answering `n` to the
/// restore question then `look`, at the TALL pane. The trigger is the band's HEIGHT
/// and nothing about the text. Arthur publishes window 3 at native `(28, 384) 584x16`
/// with window 0 `584x176` for a message that fits, and at `(28, 368) 584x32` with
/// window 0 `584x160` for one that wraps. `menu_strip_below_story` forgave the first
/// shape and not the second, because its guard was "the story reaches within ONE
/// native text row of the screen bottom": `400 <= 384 + 16` holds and `400 <= 368 +
/// 16` does not. So a two-row band read as Journey's command menu —
/// `BottomPlan::Menu` in hybrid, and an outright decline of the extension in
/// `extended`.
///
/// The band's height is transient and the frame's shape must not track it. What
/// distinguishes Arthur's band from Journey's menu is not how tall it is but whose
/// window it is: Arthur's lies wholly between the story window's bottom and the
/// screen's, so the frame carries it (`bottom_anchored_chrome`, SQ-1132), while
/// Journey's runs belong to a full-screen grid straddling the story and nothing can
/// move it. That test is now what both callers ask, through `anchored_band_bottom`.
///
/// So: **the extra row comes out of the STORY TEXT viewport, and out of nothing
/// else.** Frame height unchanged, art unchanged, one row of prose yielded per row
/// the band gains.
///
/// FALSIFY by dropping the `anchored_band_bottom` arm from `menu_strip_below_story`
/// (and the `reach` it gives `hybrid_bottom_plan`): hybrid's plan goes `frame` →
/// `menu` on the wrapped turn, and `extended`'s canvas comes back 640x400 against the
/// 640x896 the other two turns build.
#[test]
fn a_wrapped_parser_message_costs_one_text_row_and_moves_nothing_else() {
    let _g = app::v6_palette_at_boot();
    let spec = Specimen {
        file: "arthur-r74-s890714.z6",
        pictures: None,
        keys: b'n',
        taps: 12,
        release: 74,
        then: &["look"],
        extends: true,
    };
    let Some(mut b) = boot(&spec) else { return };

    const MODES: [app::config::V6RenderMode; 3] = [
        app::config::V6RenderMode::Hybrid,
        app::config::V6RenderMode::Raster,
        app::config::V6RenderMode::Extended,
    ];

    // Three frames of the same gameplay screen, differing only in how tall Arthur's
    // own band under window 0 is: none, one row, two rows.
    let mut seen: Vec<(u16, String, Vec<FrameShape>)> = Vec::new();
    for cmd in ["", "frobozzle the grue", "was"] {
        if !cmd.is_empty() {
            let r = b.session.submit(cmd);
            assert!(r.fault.is_none(), "{cmd:?} faulted: {:?}", r.fault);
        }
        let rows = band_rows(&mut b);
        let text = band_text(&mut b);
        let shapes: Vec<FrameShape> = MODES.iter().map(|&m| shape(&mut b, m, TALL)).collect();
        eprintln!("after {cmd:?}: band {rows} row(s) {text:?} → {shapes:?}");
        seen.push((rows, text, shapes));
    }

    // NON-VACUITY. The case is worthless unless the three frames really are a clean
    // band, a one-row band and a WRAPPED two-row band — which is the shape SQ-1157 is
    // about, and the one no other case in this suite reaches.
    assert_eq!(seen[0].0, 0, "the `look` frame must have nothing below window 0, got {:?}", seen[0].1);
    assert_eq!(seen[1].0, 1, "the rejected word must give a ONE-row band, got {:?}", seen[1].1);
    assert_eq!(seen[2].0, 2, "`was` must give a WRAPPED two-row band, got {:?}", seen[2].1);
    assert!(
        seen[2].1.contains("rephrase"),
        "the wrapped turn must be Arthur's own parser message — got {:?}",
        seen[2].1
    );

    // The report, in every mode: the art and the frame do not move, and each row the
    // band gains costs exactly one row of prose.
    for (i, mode) in MODES.iter().enumerate() {
        let (base, one, two) = (&seen[0].2[i], &seen[1].2[i], &seen[2].2[i]);
        for (rows, s) in [(1u16, one), (2u16, two)] {
            assert_eq!(
                (&s.plan, &s.scale, s.canvas_h),
                (&base.plan, &base.scale, base.canvas_h),
                "{mode:?}: a {rows}-row band must not change the frame — {s:?} against {base:?}",
            );
            assert_eq!(
                s.viewport,
                base.viewport - rows,
                "{mode:?}: a {rows}-row band costs exactly {rows} row(s) of prose — {s:?} against {base:?}",
            );
        }
    }
}

// ── 7. The click map ─────────────────────────────────────────────────────────
//
// An extended composite is TALLER than the game's screen, so the click map's
// inverse — which is stated over the drawn canvas, because that is what was drawn
// — can land on a native row the game never had. Those rows carry lanthorn's own
// scrollback. A click there is dropped, not clamped onto the game's last row:
// clamping is the worse failure, because it hands the game a plausible in-range
// coordinate the player did not click.

/// The rule, stated over the map alone: `canvas` inverts, `screen` bounds.
///
/// FALSIFY by restoring `map_click`'s old tail — `Some((gx.min(canvas.0), gy.min(
/// canvas.1)))` with no `screen` at all — and the clicks below the game's screen
/// come back as row 400 instead of `None`.
#[test]
fn the_click_map_drops_a_click_in_the_rows_lanthorn_added() {
    use app::render::graphics::V6ClickMap;
    // A 640x896 composite drawn 1:1 at an 8x16 cell, over a 640x400 game screen —
    // the frame `zork0-r393-s890714.z6` builds at the TALL pane in this suite.
    let m = V6ClickMap {
        pane_x: 0,
        pane_y: 0,
        cell_w: 8,
        cell_h: 16,
        img_x: 0.0,
        img_y: 0.0,
        img_w: 640.0,
        img_h: 896.0,
        canvas: (640, 896),
        screen: (640, 400),
        packed_text: Vec::new(),
    };
    // Inside the game's own screen: the ordinary letterbox inverse, unchanged.
    assert_eq!(m.map_click(0, 0), Some((5, 9)), "the top-left cell is the game's");
    assert_eq!(m.map_click(40, 12), Some((325, 201)), "and so is the middle of its screen");
    // The last row the game has (native 385..400) is cell row 24.
    let last = m.map_click(0, 24).expect("the game's last text row still maps");
    assert!(last.1 <= 400, "…and maps inside the game's screen, not past it: {last:?}");
    // The first row lanthorn added is cell row 25 (native 401..416).
    assert_eq!(m.map_click(0, 25), None, "the first added row is not the game's to hear");
    assert_eq!(m.map_click(40, 40), None, "nor is anything below it");
    assert_eq!(m.map_click(0, 55), None, "…down to the bottom of the composite");

    // The same map with nothing added — every path but an extended raster frame —
    // answers exactly as it always did, including at the bottom row.
    let plain = V6ClickMap { canvas: (640, 400), screen: (640, 400), img_h: 400.0, ..m };
    assert_eq!(plain.map_click(0, 0), Some((5, 9)));
    assert!(plain.map_click(0, 24).is_some(), "an unextended frame keeps its last row");
}

/// …and the real frame publishes it: an extended composite's click map carries a
/// canvas taller than the screen, and drops a click below the game.
///
/// The raster control at the same pane publishes `canvas == screen` and drops
/// nothing, which is how this case shows the rejection belongs to the EXTENSION
/// and not to the click map in general.
#[test]
fn the_extended_frame_publishes_the_games_screen_beside_its_canvas() {
    let _g = app::v6_palette_at_boot();
    let mut seen = 0usize;
    let mut any_present = false;
    for spec in CORPUS.iter().filter(|s| s.extends) {
        any_present |= stories_dir().join(spec.file).exists();
        let Some(mut b) = boot(spec) else { continue };
        let transcript = b.session.take_transcript();
        let model = b.session.screen();
        let area = Rect::new(0, 0, TALL.0, TALL.1);
        let map_for = |mode| {
            let state = state_for(mode, &transcript, &b);
            let mut buf = Buffer::empty(area);
            let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
            let map = state
                .graphics_render
                .borrow()
                .last_v6_map
                .clone()
                .unwrap_or_else(|| panic!("{}: the raster path records a click map", spec.file));
            map
        };
        let plain = map_for(app::config::V6RenderMode::Raster);
        let ext = map_for(app::config::V6RenderMode::Extended);
        eprintln!(
            "{}: raster map canvas {:?} screen {:?} · extended map canvas {:?} screen {:?}",
            spec.file, plain.canvas, plain.screen, ext.canvas, ext.screen
        );
        assert_eq!(plain.canvas, plain.screen, "{}: raster draws the game's screen", spec.file);
        assert_eq!(ext.screen, plain.screen, "{}: the game's screen never changes", spec.file);
        assert!(
            ext.canvas.1 > ext.screen.1,
            "{}: this case needs a frame that actually extended ({:?} vs {:?})",
            spec.file,
            ext.canvas,
            ext.screen
        );

        // Sweep the pane: every cell either maps inside the game's screen or is
        // dropped. Nothing may come back as a game pixel the game does not have —
        // which is what the old clamp produced.
        let mut mapped = 0usize;
        let mut dropped = 0usize;
        for row in area.y..area.bottom() {
            for col in area.x..area.right() {
                match ext.map_click(col, row) {
                    Some((gx, gy)) => {
                        assert!(
                            gx >= 1 && gx <= ext.screen.0 && gy >= 1 && gy <= ext.screen.1,
                            "{}: cell ({col},{row}) mapped to ({gx},{gy}), outside the game's \
                             {:?} screen",
                            spec.file,
                            ext.screen
                        );
                        mapped += 1;
                    }
                    None => dropped += 1,
                }
            }
        }
        eprintln!("  {} cells map to the game, {dropped} are dropped", mapped);
        assert!(mapped > 0, "{}: some of the pane is still the game's", spec.file);
        assert!(
            dropped > 0,
            "{}: the rows lanthorn added must be dropped — none were",
            spec.file
        );

        // …and the dropped ones are the EXTENSION's, not merely the letterbox
        // margins. The game's screen ends at the device row where `screen.1` of
        // `canvas.1` has been drawn; every cell whose centre falls below that is
        // lanthorn's, and must be `None` on BOTH axes' worth of columns.
        //
        // Stated separately from the sweep above because that sweep passes under
        // the OLD clamp: clamping keeps every answer inside the screen too, and
        // the horizontal margins are dropped either way. This is the half that
        // fails when the rejection is replaced by `.min()`.
        let screen_dev = ext.img_y + ext.img_h * f32::from(ext.screen.1) / f32::from(ext.canvas.1);
        let first_added = area.y
            + ((screen_dev / f32::from(ext.cell_h)).ceil() as u16)
            + 1;
        assert!(first_added < area.bottom(), "{}: the extension reaches the pane", spec.file);
        for row in first_added..area.bottom() {
            for col in area.x..area.right() {
                assert_eq!(
                    ext.map_click(col, row),
                    None,
                    "{}: cell ({col},{row}) is in the rows lanthorn added (the game's screen \
                     ends at device row {screen_dev}) and must not reach the game",
                    spec.file
                );
            }
        }
        // The raster control drops only its letterbox margins, and every cell it
        // does map is inside the same screen.
        for row in area.y..area.bottom() {
            for col in area.x..area.right() {
                if let Some((gx, gy)) = plain.map_click(col, row) {
                    assert!(
                        gx <= plain.screen.0 && gy <= plain.screen.1,
                        "{}: the raster map is unchanged by any of this",
                        spec.file
                    );
                }
            }
        }
        seen += 1;
    }
    if any_present {
        assert!(seen > 0, "a present fixture must have been measured, not skipped");
    }
}

// ── 8. The MACINTOSH, where the two densities disagree ───────────────────────
//
// Every other press in this suite has an 8x16 cell and a (2, 2) art scale, and on
// such a machine several wrong arithmetics give the right answer. The Macintosh is
// the only machine in the corpus where the ART's density and the TEXT's cell
// disagree — 7x15 text (SQ-0917) under artwork that is either 480x300 at (1, 1)
// (`Pic.data`) or 320x200 doubled (`CPic.data`) — so it is the only machine that can
// falsify this mode's arithmetic:
//
//   * SQ-1012 / SQ-1024 — a whole-ART rung is a HALF-NATIVE one wherever an art
//     pixel is already two native pixels, and a 7-wide glyph at 1.5 native scale
//     gets 10.5 device pixels. `RasterFrame::extended` pins whole DEVICE pixels per
//     NATIVE pixel for exactly this reason; here it is measured rather than argued.
//   * SQ-1039 — scaling a face by `art_scale` declared Geneva 12's fifteen rows as
//     thirty, and ONLY on the colour press. The B/W press is (1, 1) and cannot
//     falsify it, which is why BOTH renditions are driven below.
//
// Both are the same release on the same disk. A rendition is not a spelling.

/// One Macintosh rendition: the medium, the archive, and the two densities it
/// should resolve.
struct MacPress {
    file: &'static str,
    pictures: Option<&'static str>,
    release: u16,
    taps: usize,
    /// The picture space this archive declares, as `MachineBoot` resolves it.
    art_scale: (u32, u32),
}

const MAC: &[MacPress] = &[
    // Zork Zero, Macintosh release 296 / serial 881019 — the disk `v6_macintosh_profile`
    // pins. Six taps in is its PROLOGUE, the framed gameplay screen that the 8x16
    // press extends at.
    MacPress { file: "Zork Zero Disk.image", pictures: Some("CPic.data"), release: 296, taps: 6, art_scale: (2, 2) },
    MacPress { file: "Zork Zero Disk.image", pictures: Some("Pic.data"), release: 296, taps: 6, art_scale: (1, 1) },
    // Shogun, Macintosh release 292 / serial 890314 — the other Macintosh medium.
    MacPress { file: "Shogun.toast", pictures: None, release: 292, taps: 2, art_scale: (2, 2) },
];

/// The extension's arithmetic, on the machine whose cell is 7x15.
///
/// Four things, all of them the ones a `/16` or an `art_scale` would get wrong:
/// the magnification is whole in DEVICE pixels per NATIVE pixel; the extension is a
/// whole number of the MACHINE's text rows with no partial row; the face is drawn at
/// its own space and not at `art_scale`; and the prose grows by exactly the rows the
/// frame added.
///
/// A rendition whose frame DECLINES is reported and skipped rather than forced —
/// declining is a legitimate answer (§5 above), and the case says which renditions
/// actually exercised the arithmetic so a silent skip cannot read as a pass.
#[test]
fn the_macintosh_cell_is_what_the_extension_counts_in() {
    let _g = app::v6_palette_at_boot();
    let mut extended_any = 0usize;
    let mut any_present = false;
    for m in MAC {
        any_present |= stories_dir().join(m.file).exists();
        let Some(mut b) = boot_raw(m.file, m.release, m.pictures) else { continue };
        tap_in(&mut b.session, 13, m.taps);

        // The machine, before anything is measured on it (CLAUDE.md: print the
        // profile, release, screen and cell the harness booted).
        let cell = b.face.cell();
        assert_eq!(
            b.profile,
            InterpreterProfile::Macintosh,
            "{} [{:?}]: this case is about the Macintosh — the medium resolved {:?}",
            m.file, m.pictures, b.profile
        );
        assert_eq!(
            (cell.w(), cell.h()),
            (7, 15),
            "{} [{:?}]: the Version 6 cell is the MACHINE's (SQ-0917)",
            m.file, m.pictures
        );
        assert_eq!(
            b.art_scale, m.art_scale,
            "{} [{:?}]: this rendition's picture space",
            m.file, m.pictures
        );

        // SQ-1039, live: the face is scaled by its OWN space, never by the
        // artwork's. The Macintosh answers `Native` both ways, so a (2, 2) art
        // scale must leave the text scale at (1, 1) — and the COLOUR press is the
        // only rendition that can catch it, because the B/W press is (1, 1) and a
        // wrong multiply is invisible there.
        assert_eq!(
            b.face.scale(),
            (1, 1),
            "{} [{:?}]: the Macintosh draws text at one native pixel per face pixel \
             whatever `CPic.data`'s density is — scaling the face by art_scale is SQ-1039",
            m.file, m.pictures
        );

        for pane in [TALL, WIDE] {
        let ((plain, pm, pf), (extended, em, ef)) = pair(&mut b, pane);
        eprintln!(
            "  MAC {:<22} [{:?}] {pane:?} cell {:?} art_scale {:?} face {:?} :: raster {}x{} -> {} \
             {}x{} (lock {:?})",
            m.file,
            m.pictures,
            (cell.w(), cell.h()),
            b.art_scale,
            b.face.scale(),
            plain.width(),
            plain.height(),
            if ef.extension() > 0 { "extended" } else { "declined" },
            extended.width(),
            extended.height(),
            ef.lock,
        );
        if let Ok(dir) = std::env::var("LANTHORN_SHOT_DIR") {
            let tag = format!("{}-{:?}-{pane:?}", m.file.replace(' ', "_"), m.pictures);
            extended.save(format!("{dir}/mac-ext-{tag}.png")).unwrap();
            plain.save(format!("{dir}/mac-ras-{tag}.png")).unwrap();
        }
        assert_eq!(pf.extension(), 0, "{}: the raster frame never extends", m.file);
        if ef.extension() == 0 {
            eprintln!("      (declines — reported, not forced)");
            assert!(
                extended.as_raw() == plain.as_raw(),
                "{} [{:?}]: a declined Macintosh frame is the raster composite to the byte",
                m.file, m.pictures
            );
            continue;
        }

        // (1) A whole magnification, in DEVICE pixels per NATIVE pixel.
        let s = ef.lock.expect("an extended frame pins one");
        assert_eq!(s, s.floor(), "{} [{:?}]: {s} is not whole", m.file, m.pictures);
        assert!(s >= 1.0, "{} [{:?}]: and never a minification", m.file, m.pictures);

        // (2) The extension is whole 7x15 text rows, with no partial row: the canvas
        // is the game's screen plus a whole multiple of the MACHINE's cell height.
        assert_eq!(
            ef.extension() % u32::from(cell.h()),
            0,
            "{} [{:?}]: the extension must be whole rows of a {}-tall cell, not {}",
            m.file, m.pictures, cell.h(), ef.extension()
        );
        assert_eq!(extended.height(), plain.height() + ef.extension());
        assert_eq!(extended.width(), plain.width(), "{}: never sideways", m.file);

        // (3) …and the prose took exactly those rows, counted in the same cell.
        let pv = pm.expect("the raster frame has a story box").viewport_rows;
        let ev = em.expect("and so does the extended one").viewport_rows;
        assert_eq!(
            u32::from(ev - pv),
            ef.extension() / u32::from(cell.h()),
            "{} [{:?}]: raster {pv} rows, extended {ev} — not the {} rows the frame added",
            m.file,
            m.pictures,
            ef.extension() / u32::from(cell.h())
        );
        eprintln!("      prose {pv} -> {ev} rows of a {}px cell", cell.h());
        extended_any += 1;
        }
    }
    if any_present {
        assert!(
            extended_any > 0,
            "no Macintosh rendition exercised the extension — the arithmetic is still undriven \
             on the only machine whose densities disagree"
        );
    }
}

// ── 9. Restore ───────────────────────────────────────────────────────────────

/// A game saved in `extended` and restored still extends — and still extends one
/// MOVE later, into a pane of a different size.
///
/// `v6_render` is config rather than archive state, so this ought to be free. The
/// project's convention is to distrust "ought to": *restore bugs surface one action
/// AFTER the restore, when the game next repaints, changes palette, splits or
/// resizes* — asserting the frame straight after a restore is when everything still
/// looks correct. So this restores, MOVES, and only then asserts, the way
/// `v6_restore_palette_replay` does.
///
/// The second half is the case a same-session round-trip structurally cannot see:
/// the archive is backend- and terminal-neutral, and a restore into a DIFFERENT pane
/// is a resize the game never saw. The extension is derived from the pane, so the
/// restored frame must re-derive it — at `WIDE` that is a different magnification
/// (2 rather than 1) and a different canvas height, off the same archive.
#[test]
fn a_game_saved_in_extended_still_extends_a_move_after_it_is_restored() {
    let _g = app::v6_palette_at_boot();
    let spec = &CORPUS[0];
    assert_eq!(spec.file, "zork0-r393-s890714.z6", "this case is pinned to Zork Zero's prologue");
    let Some(mut b) = boot(spec) else { return };
    let _ = b.session.submit("look");
    let _ = b.session.take_transcript();

    let before = {
        let (_, (canvas, m, f)) = pair(&mut b, TALL);
        (canvas.dimensions(), m.expect("a story box before saving").viewport_rows, f)
    };
    assert!(before.2.extension() > 0, "the frame being saved is an extended one");

    // Through the real archive, as Save State and auto-resume do.
    let mapper = mapper::mapper::Mapper::default();
    let es = app::engine::Engine::save_state(&b.session);
    let path = std::env::temp_dir().join(format!("sq1032-restore-{}.lanthorn", std::process::id()));
    app::archive::save_archive_meta_pics(
        &path,
        &mapper,
        &es,
        Some(&b.session.machine.screen),
        &b.session.machine.aux_data,
        app::archive::Meta {
            format_version: app::archive::CURRENT_FORMAT_VERSION,
            ifid: None,
            name: None,
            turns: 0,
            saved_at: String::new(),
            location: None,
            score: None,
            trigger: app::archive::SaveTrigger::HostState,
        },
        &app::archive::SessionRecord::empty(),
        &b.session.pictures_png(),
        None,
        None,
    )
    .expect("save archive");
    let ac = app::archive::load_archive(&path).expect("load archive");
    let _ = std::fs::remove_file(&path);

    let mut fresh = boot_raw(spec.file, spec.release, spec.pictures).expect("fresh boot");
    app::engine::Engine::restore_state(&mut fresh.session, &ac.engine_save()).expect("restore");
    app::session::restore_screen(&mut fresh.session, ac.screen.clone().expect("screen"));
    fresh.session.load_pictures_png(&ac.pictures);

    // PERTURB. Everything still looks correct on the frame immediately after a
    // restore; the defect arrives when the game next repaints.
    let r = fresh.session.submit("look");
    assert!(r.fault.is_none(), "the move after the restore faulted: {:?}", r.fault);

    // The pane it was saved at.
    let (_, (canvas, m, f)) = pair(&mut fresh, TALL);
    let rows = m.expect("a story box after the restore").viewport_rows;
    eprintln!(
        "restore @ {TALL:?}: saved {}x{} / {} rows → restored {}x{} / {rows} rows (lock {:?})",
        before.0.0, before.0.1, before.1, canvas.width(), canvas.height(), f.lock
    );
    assert!(f.extension() > 0, "the restored frame must still extend");
    assert_eq!(canvas.dimensions(), before.0, "…to the same canvas at the same pane");
    assert_eq!(f, before.2, "…on the same frame");
    assert_eq!(rows, before.1, "…with the same story viewport");

    // …and into a pane it was NOT saved at, which is a resize the game never saw.
    // `WIDE` affords a whole magnification of two, so the restored frame must
    // re-derive both the scale and the height rather than replay the saved ones.
    let (_, (wide, wm, wf)) = pair(&mut fresh, WIDE);
    let wrows = wm.expect("a story box at the other pane").viewport_rows;
    eprintln!(
        "restore @ {WIDE:?}: {}x{} / {wrows} rows (lock {:?})",
        wide.width(), wide.height(), wf.lock
    );
    assert!(wf.extension() > 0, "the restored frame extends at the other pane too");
    assert_eq!(wf.lock, Some(2.0), "…at that pane's own whole magnification");
    assert_ne!(wide.dimensions(), canvas.dimensions(), "…and a different canvas for it");
    assert_eq!(
        wf.extension() % u32::from(fresh.face.cell().h()),
        0,
        "…still whole text rows"
    );
    assert!(wrows > 0, "…with prose in it");
}
