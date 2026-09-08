//! Zork0 v6 window-geometry headless smoke — the Phase 1a acceptance gate
//! (SQ-0186, `docs/superpowers/plans/2026-07-21-v6-phase1a-engine.md` Task 10).
//!
//! Drives the real `stories/zork0-r393-s890714.z6` (v6/graphical) headlessly
//! through boot and a couple of turns, and asserts that Tasks 1–9's v6 window
//! model + graphics boundary produce sane, non-underflowed geometry and real
//! injected picture dimensions/draw events — everything Plan 1b needs to
//! render Zork0.
//!
//! Zork0's release layout is a bare `.z6` executable beside a resources-only
//! sidecar Blorb (`Zork0.blb`, no `Exec` chunk of its own) — discovered during
//! Task 9. The story loaded here is the bare `.z6`; `blorb::resolve_resource_blorb`
//! finds `Zork0.blb` as the picture sidecar via its dir-scan stem-prefix match,
//! exactly as `startup.rs`'s `ZCode` arm does.
//!
//! The story asset is gitignored (large, local-only), so this test **skips
//! cleanly** when absent — CI and fresh clones stay green, dev worktrees get
//! the real coverage. Mirrors the skip-if-absent pattern in
//! `crates/gvm/tests/kerkerkruip_boots.rs` / `crates/app/tests/wizard_sniffer.rs`.
//!
//! **Colour mode: `honor_game_colours = true`** — the app's shipped config
//! default, so these render assertions are made in the mode real players run.
//! (Before SQ-0532 wave 4 every v6 smoke booted with the game's colours
//! DECLINED, which is exactly why three colour-driven render regressions
//! shipped unseen. The theme-only `false` path is covered by the paired cases
//! in `v6_game_colour_regression.rs`.)

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::GameSession;

/// The repo-root `stories/` directory (a gitignored symlink in dev worktrees,
/// absent in CI), resolved relative to this crate's manifest.
fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// The Phase-0 layout-underflow signature: a `u16` that wrapped negative
/// (e.g. `0 - 16` in pixel-rect math) lands in the top of the `u16` range.
/// Any v6 window whose `x_size`/`y_size` falls in here is a bug, not geometry.
const UNDERFLOW_RANGE: std::ops::RangeInclusive<u16> = 0xFFF0..=0xFFFF;

/// The palette this suite's colour assertions resolve through, **stated rather than
/// inherited** (SQ-0958).
///
/// Every story these cases drive is a bare file that names no machine, so its colour
/// numbers resolve through ZMSD §8.3.1's own table — which is what every assertion
/// below was written against. Until now nothing here said so, and the suite believed
/// whatever the last suite in its group binary left behind: harmless only while every
/// one of them happened to leave `Standard` there, and not at all once a sibling boots
/// a machine press. See [`app::v6_palette`], which is why this both names a palette
/// and takes the shared lock. Hold the guard for the whole case.
fn standard_palette() -> app::V6PaletteGuard {
    app::v6_palette(zvm::screen::Palette::Standard)
}

#[test]
fn zork0_v6_windows_smoke() {
    let _g = standard_palette();
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };

    // Build the v6 Pict dimension table the same way `startup.rs`'s ZCode arm
    // does: resolve the story's resource Blorb (Zork0's actual release layout
    // is a dir-scan stem-prefix match onto `Zork0.blb`, a resources-only Blorb
    // with no `Exec` of its own), header-sniff every Pict's size, and inject
    // the table BEFORE constructing the session — `picture_data` is called
    // during boot, which happens inside `GameSession::new_with_trace` itself
    // (Phase 0 lesson).
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    eprintln!("resolved {} Pict dimension entries from the sidecar", picture_dims.len());

    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Zork0 (v6) should load and boot without a ZError");

    // (a) No fault/panic during boot.
    assert!(!session.quit, "Zork0 quit during boot (fault or premature quit), before reaching the first prompt");
    assert!(
        session.machine.fault_trace.is_none(),
        "Zork0 faulted during boot: {:?}",
        session.machine.fault_trace.as_ref().map(|t| t.to_lines())
    );

    // (b) The sidecar dims were injected into the machine.
    assert!(
        !session.machine.picture_dims.is_empty(),
        "machine.picture_dims should be non-empty after Task 9's sidecar injection"
    );

    let v6 = session
        .machine
        .screen
        .v6
        .as_ref()
        .expect("a v6 story must populate ScreenState.v6 (Task 2)");
    eprintln!("--- v6 window table at first prompt (current={}) ---", v6.current);
    for (i, w) in v6.windows.iter().enumerate() {
        eprintln!(
            "  window {i}: y={} x={} y_size={} x_size={} attrs={:#06x}",
            w.y_coord, w.x_coord, w.y_size, w.x_size, w.attributes
        );
    }
    eprintln!("pending_pictures at first prompt: {:?}", session.machine.pending_pictures());

    // (c) At least one window has nonzero, non-underflowed size — locked to the
    // real geometry observed from a `--nocapture` run: window 0 (the status/
    // banner grid, 630x184px) and window 7 (Zork0's graphics window, 640x192px)
    // are both real, sane rects, nowhere near the Phase-0 underflow range.
    for (i, w) in v6.windows.iter().enumerate() {
        assert!(
            !UNDERFLOW_RANGE.contains(&w.x_size) && !UNDERFLOW_RANGE.contains(&w.y_size),
            "window {i} shows the Phase-0 underflow signature: x_size={} y_size={}",
            w.x_size,
            w.y_size
        );
    }
    // With the reference-authentic 640×400 unit screen advertised before boot
    // (SQ-0479: 2× the 320×200 Blorb `Reso`) AND the Rect placement pictures
    // answering `picture_data` with DOUBLED dims, Zork0 computes its real layout:
    // the full-screen graphics window (7) is 640×400 and the main text window (0)
    // is inset to 468×320 inside the border art (split pseudo-picture #387 = 86×78
    // — doubled from 43×39 — → win0 at (87,79), 640−2·86 wide, 400−78 tall).
    assert_eq!((v6.windows[0].x_size, v6.windows[0].y_size), (468, 320), "window 0 geometry");
    assert_eq!((v6.windows[0].x_coord, v6.windows[0].y_coord), (87, 79), "window 0 position (1-based)");
    assert_eq!((v6.windows[7].x_size, v6.windows[7].y_size), (640, 400), "window 7 (graphics) geometry");

    // Thread the Pict source onto the session (Plan 1b Task 2), the same way
    // `startup.rs`'s ZCode arm does after construction: `drain_turn` uses it to
    // rasterize `pending_pictures` into `pictures_canvas` as it drains them.
    session.set_pict_source(Some(picts));

    // Flush Zork0's boot-time picture events (Plan 1b Task 5): Zork0 draws its
    // opening art during boot, inside `new_with_trace`, before a Pict source
    // existed to rasterize it — those events sit in `machine.pending_pictures`
    // until something drains them. Without this one-time flush, the very
    // first `screen()` below (before any turn) would show a blank graphics
    // window; `startup.rs`'s ZCode arm calls this in the same spot.
    session.flush_boot_pictures();

    // (d) Task 2's whole point, now exercised at boot rather than via the first
    // turn's drain: Zork0's boot-time `draw_picture` events (`pending_pictures
    // at first prompt` above, several targeting window 7 — the v6 convention
    // for its banner/graphics window) must have been rasterized into a real,
    // non-blank canvas by the flush just above — not left as raw undrained
    // events.
    let canvas = session.pictures_canvas.get(&7)
        .expect("window 7 should have a canvas after the boot-picture flush");
    let non_transparent = canvas.img.pixels().any(|p| p.0[3] != 0);
    assert!(non_transparent, "window 7's canvas should have at least one non-transparent pixel drawn");

    // (f) The whole v6 render path, end to end (Plan 1b Task 5): the initial
    // screen model — built before the player has typed anything — must already
    // be the z-ordered layered composite (Task 4's adapter), with at least one
    // rasterized graphics leaf (the boot art just flushed above) and at least
    // one text leaf (Grid or primary Buffer), at a nonzero content size.
    let initial_screen = session.screen();
    let WinNode::Layered(items) = &initial_screen.root else {
        panic!("v6 story's screen() root must be WinNode::Layered, got {:?}", initial_screen.root);
    };
    assert!(
        items.iter().any(|pw| matches!(pw.node, WinNode::Graphics(_))),
        "initial (post-boot-flush) screen model should have at least one Graphics leaf"
    );
    assert!(
        items.iter().any(|pw| matches!(pw.node, WinNode::Grid(_) | WinNode::Buffer(_))),
        "initial screen model should have at least one text leaf (Grid or Buffer)"
    );
    assert_ne!(initial_screen.content_size, (0, 0), "v6 content_size must be nonzero");

    // Drive a couple of turns past the first prompt as a regression check: with
    // the boot backlog now flushed up front, ordinary movement commands should
    // neither fault nor leave anything stuck in the VM's own picture queue
    // (`drain_turn` still drains it every turn, even when it stays empty).
    let _ = session.take_transcript();
    for cmd in ["look", "north"] {
        let result = session.submit(cmd);
        eprintln!(
            "turn {cmd:?}: quit={} fault={:?} transcript_chars={} pictures={}",
            result.quit,
            result.fault,
            result.transcript.chars().count(),
            result.pictures.len(),
        );
        assert!(!result.quit, "Zork0 quit on command {cmd:?}");
        assert!(result.fault.is_none(), "Zork0 faulted on command {cmd:?}: {:?}", result.fault);
        assert!(
            session.machine.pending_pictures().is_empty(),
            "drain_turn must drain the VM's queue every turn (Task 2)"
        );
    }
}

/// Phase 1c Task 2: `PositionedWindow` now carries game-pixel rects alongside
/// the existing cell rect. Boots Zork0 the same way `zork0_v6_windows_smoke`
/// does, builds the initial v6 `ScreenModel`, and asserts every positioned
/// window's pixel rect is consistent with its cell rect at the non-square 8×16
/// v6 cell (`cell_x = px_x/8`, `cell_y = px_y/16`, SQ-0479) and that live windows
/// have a nonzero pixel size.
#[test]
fn v6_positioned_windows_carry_game_pixel_rects() {
    let _g = standard_palette();
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };

    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();

    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Zork0 (v6) should load and boot without a ZError");
    assert!(!session.quit, "Zork0 quit during boot");
    assert!(session.machine.fault_trace.is_none(), "Zork0 faulted during boot");

    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();

    let initial_screen = session.screen();
    let WinNode::Layered(items) = &initial_screen.root else {
        panic!("v6 story's screen() root must be WinNode::Layered, got {:?}", initial_screen.root);
    };
    assert!(!items.is_empty(), "v6 model has positioned windows");
    for it in items {
        assert_eq!(it.x, it.x_px / 8, "cell x derived from px x");
        assert_eq!(it.y, it.y_px / 16, "cell y derived from px y (16px cell height)");
        assert!(it.w_px > 0 && it.h_px > 0, "live window has nonzero pixel size");
    }
}

/// Phase 1c Task 5: end-to-end pixel-composite smoke. Boots Zork0 the same way
/// the other tests in this file do, builds the initial v6 `ScreenModel`, then
/// feeds its `Layered` items straight into `build_v6_canvas` (Task 3) with a
/// synthetic `MainText` and a fixed 8x8 cell size, asserting the resulting
/// device-resolution canvas is sized as expected and isn't left as a flat,
/// unpainted background — i.e. the real Zork0 boot state (graphics + grid
/// windows) actually composites into pixels.
#[test]
fn zork0_v6_pixel_canvas_is_nonempty() {
    let _g = standard_palette();
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };

    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();

    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Zork0 (v6) should load and boot without a ZError");
    assert!(!session.quit, "Zork0 quit during boot");
    assert!(session.machine.fault_trace.is_none(), "Zork0 faulted during boot");

    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();

    let initial_screen = session.screen();
    let WinNode::Layered(items) = &initial_screen.root else {
        panic!("v6 story's screen() root must be WinNode::Layered, got {:?}", initial_screen.root);
    };
    assert!(!items.is_empty(), "v6 model has positioned windows");

    let colors = app::colors::ColorScheme::default();
    use app::render::v6_layout as v6;
    let native = v6::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = v6::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let default_fg = image::Rgba([220, 220, 220, 255]);
    let default_bg = image::Rgba([0, 0, 0, 255]);
    let mut canvas = v6::build_chrome_canvas(&layout.chrome, native, default_fg, default_bg, &colors, v6::TextLayer::All, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    // A visible story line of 'X's; assert none of its glyph pixels land on an opaque chrome
    // pixel of the frame (they sit in the clear interior).
    let main = v6::MainText { lines: vec!["X".repeat(30)], styles: Vec::new(), input: String::new(), cursor_col: 0, awaiting: false, floats: Vec::new() };
    let chrome_only = canvas.clone(); // snapshot BEFORE story text, to know which pixels are chrome
    if let Some((sx, sy, sw, sh)) = v6::story_clear_native(layout.story, &canvas) {
        let (cols, rows) = ((sw / 8).max(1) as u16, (sh / 16).max(1) as u16);
        v6::draw_story_text(&mut canvas, &main, sx, sy, cols, rows, default_fg, &[], &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT), None);
    }
    // non-empty
    assert!(canvas.pixels().any(|p| p[3] > 0), "composited canvas has content");
    // Every pixel that CHANGED from chrome_only (a story glyph pixel) must NOT be an opaque
    // chrome pixel in the snapshot (story text lands in the clear interior, not on the frame).
    for (x, y, p) in canvas.enumerate_pixels() {
        if *p != *chrome_only.get_pixel(x, y) {
            assert!(chrome_only.get_pixel(x, y)[3] < 128, "story glyph at ({x},{y}) overlaps opaque chrome");
        }
    }
}

/// Phase A (Task A6) end-to-end acceptance gate: boots Zork0 the same way the
/// other tests in this file do, then asserts `classify_windows` correctly
/// separates the primary story `Buffer` from the frame/status chrome, and
/// that `story_clear_native`'s clear interior lands below the banner and
/// inside the story window's own rect — the top-margin + status-overlap fix
/// this task exists to lock in. Asserts invariants (sizes/relationships), not
/// exact pixels, since the art could be re-authored — except the story
/// window's own rect (6,6,310,192), which is Zork0's stable window geometry.
#[test]
fn zork0_v6_story_classified_and_clear_interior_inside_frame() {
    let _g = standard_palette();
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };

    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();

    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Zork0 (v6) should load and boot without a ZError");
    assert!(!session.quit, "Zork0 quit during boot");
    assert!(session.machine.fault_trace.is_none(), "Zork0 faulted during boot");

    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();

    let initial_screen = session.screen();
    let WinNode::Layered(items) = &initial_screen.root else {
        panic!("v6 story's screen() root must be WinNode::Layered, got {:?}", initial_screen.root);
    };

    use app::render::v6_layout as v6;
    let native = v6::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = v6::classify_windows(items, zvm::screen::V6Cell::DEFAULT);

    // Story is the primary buffer window (Zork0's window 0), inset inside the
    // 640×400 frame (SQ-0479) at the Rect-derived position (1-based (87,79) →
    // 0-based (86,78), 468×320 — see the geometry assertions in the smoke test).
    let story = layout.story.expect("Zork0 boot has a primary story buffer");
    assert!(matches!(&story.node, WinNode::Buffer(b) if b.primary));
    assert_eq!((story.x_px, story.y_px, story.w_px, story.h_px), (86, 78, 468, 320));

    // Chrome carries the frame graphics + status grids (the full-screen bg + banner + grids).
    assert!(layout.chrome.iter().any(|w| matches!(&w.node, WinNode::Graphics(_))), "chrome has frame graphics");
    assert!(layout.chrome.iter().any(|w| matches!(&w.node, WinNode::Grid(_))), "chrome has a status grid");

    // The clear-interior story viewport lands below the banner and inside the frame — the
    // top-margin + status-overlap fix. (Values are ~ (46,47) 238×125 for r393; assert the
    // invariants, not exact pixels, so a rebuild of the art can't make this brittle.)
    let colors = app::colors::ColorScheme::default();
    let default_fg = image::Rgba([220, 220, 220, 255]);
    let default_bg = image::Rgba([0, 0, 0, 255]);
    let chrome = v6::build_chrome_canvas(&layout.chrome, native, default_fg, default_bg, &colors, v6::TextLayer::All, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let (sx, sy, sw, sh) = v6::story_clear_native(layout.story, &chrome).expect("story has a clear interior");
    assert!(sw > 100 && sh > 60, "clear interior is a real story region, got {sw}x{sh}");
    assert!(sy >= story.y_px as u32, "story text starts at/below the window top (banner cleared)");
    assert!(sx >= story.x_px as u32, "story text starts at/right-of the window left (frame cleared)");
    assert!(sx + sw <= (story.x_px + story.w_px) as u32, "clear interior stays inside the story window");
    assert!(sy + sh <= (story.y_px + story.h_px) as u32, "clear interior stays inside the story window");
}
