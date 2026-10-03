//! SQ-1703 P3: a Glulx story that ships a Windows Glk `.cfg` design size lays
//! out at the design size and the whole frame is STRETCHED over the story pane
//! in terminal cells (no letterbox, aspect not honoured).
//!
//! Booted through `boot_story` the way startup boots (so the `.cfg` is
//! discovered and `host::screen::apply_glk_design` runs in the path), then the
//! pane is resized and the frame rendered into a cell buffer. Every frame is
//! the FIRST line prompt after boot (0 inputs submitted), at the pane sizes
//! named per case, 8x16 cells. Both specimens skip vacuously without
//! `stories/`.
//!
//! Photopia 2.01 (`photo201.blb`, 640x480): frames 640x58 (top), 640x79
//! (bottom at y=401), 14x343 (left, right at x=626), story text 612x343 at
//! (14,58). Narcolepsy (`narco.blorb`, 800x600): story text 240x378 at
//! (480,60).

use std::path::PathBuf;

use app::config::Config;
use app::engine::{Engine, WinKind};
use app::glk_cfg::{design_px_to_cell_edge as edge, glk_design_screen};
use app::host::reset::{reset_game, ResetOptions};
use app::host::screen::{apply_glk_design, glk_layout, resize_glulx};
use app::host::{boot_story, BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts};
use app::launch_options::LaunchOverrides;
use app::session::InputKind;
use gvm::glk::WinType;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::fixture_paths::fixture_path;

const CELL: (f64, f64) = (8.0, 16.0);

fn story(name: &str) -> Option<PathBuf> {
    let p = fixture_path(name);
    if !p.is_file() {
        eprintln!("SKIP: {name} missing at {}", p.display());
        return None;
    }
    Some(p)
}

fn boot_in(path: PathBuf, home: PathBuf) -> BootedStory {
    let overrides = LaunchOverrides::default();
    let req = BootRequest {
        story_path: path,
        disk_entry: None,
        overrides: &overrides,
        cfg: Config {
            user_dir: home.clone(),
            config_file: home.join("config.toml"),
            random_seed: Some(1),
            auto_save: false,
            ..Config::default()
        },
        roots: app::data_roots::DataRoots::single(home.join("saves")),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
        fresh_start: true,
    };
    boot_story(req, &mut QuietBoot).expect("boots headlessly")
}

fn boot(path: PathBuf) -> BootedStory {
    boot_in(path, app::scratch_dir("sq1703-stretch"))
}

/// The frame at a `cols`x`rows` pane: the drawn rects of every window.
fn frame(b: &mut BootedStory, cols: u16, rows: u16) -> Vec<(u32, WinKind, Rect)> {
    assert!(resize_glulx(b.session.as_mut(), (cols, rows)));
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let model = b.session.screen();
    app::render::screen::render_story_pane(&model, false, None, &b.state, area, &mut buf).win_rects
}

fn of_kind(r: &[(u32, WinKind, Rect)], k: WinKind) -> Vec<Rect> {
    let mut v: Vec<Rect> = r.iter().filter(|(_, kind, _)| *kind == k).map(|(_, _, r)| *r).collect();
    v.sort_by_key(|r| (r.y, r.x));
    v
}

/// Photopia's four frames and its story text, in design pixels.
const P_TEXT: (u32, u32, u32, u32) = (14, 58, 612, 343);

fn photopia_expect(cols: u16, rows: u16) {
    let Some(path) = story("photo201.blb") else { return };
    let mut b = boot(path);
    // Non-vacuity: the booted story really is the design-mode Photopia at its
    // first line prompt with nothing submitted.
    assert!(b.state.glk_stretch, "the .cfg design size switches stretch mode on");
    assert_eq!(b.state.glk_design.as_ref().and_then(|d| d.size()), Some((640, 480)));
    assert_eq!(Engine::pending_input(b.session.as_ref()), InputKind::Line, "0 inputs, first line prompt");
    let r = frame(&mut b, cols, rows);
    let (c, rw) = (u32::from(cols), u32::from(rows));
    let (ex, ey) = (|x| edge(x, 640, c) as u16, |y| edge(y, 480, rw) as u16);

    let gfx = of_kind(&r, WinKind::Graphics);
    let buf = of_kind(&r, WinKind::Buffer);
    assert_eq!(gfx.len(), 4, "four frame windows: {r:?}");
    assert_eq!(buf.len(), 1, "one story text window: {r:?}");

    let (top_h, bot_y) = (ey(58), ey(401));
    let (left_w, right_x) = (ex(14), ex(626));
    let want = [
        Rect::new(0, 0, cols, top_h),                              // top frame
        Rect::new(0, top_h, left_w, bot_y - top_h),                // left frame
        Rect::new(right_x, top_h, cols - right_x, bot_y - top_h),  // right frame
        Rect::new(0, bot_y, cols, rows - bot_y),                   // bottom frame
    ];
    for w in want {
        assert!(gfx.contains(&w), "frame {w:?} missing from {gfx:?} at {cols}x{rows}");
    }

    // The story text: exactly the characters the game was told, anchored at the
    // window's near corner, inside the interior the frames leave.
    let l = glk_layout(b.session.as_mut()).unwrap();
    assert_eq!(l.screen, glk_design_screen((640, 480), (c as f64 * CELL.0, rw as f64 * CELL.1), CELL));
    let tw = l.windows.iter().find(|w| w.wintype == WinType::TextBuffer).unwrap();
    assert_eq!(
        (tw.rect.left, tw.rect.top, tw.rect.width, tw.rect.height),
        P_TEXT,
        "the game laid out at the design size"
    );
    let told = (l.screen.chars_in(tw.rect.width, false) as u16, l.screen.chars_in(tw.rect.height, true) as u16);
    let text = buf[0];
    assert_eq!((text.x, text.y), (left_w, top_h), "text starts where the left/top frames end");
    assert_eq!((text.width, text.height), told, "chars told == cells drawn at {cols}x{rows}");
    let interior = (right_x - left_w, bot_y - top_h);
    assert!(text.width <= interior.0 && text.height <= interior.1);
    assert!(interior.0 - text.width <= 1 && interior.1 - text.height <= 1, "at most one filler cell per axis");

    // No cell of the pane uncovered or doubly covered: frames + the interior.
    let mut cover = vec![0u8; cols as usize * rows as usize];
    for g in gfx.iter().copied().chain([Rect::new(left_w, top_h, interior.0, interior.1)]) {
        for y in g.y..g.bottom() {
            for x in g.x..g.right() {
                cover[y as usize * cols as usize + x as usize] += 1;
            }
        }
    }
    assert!(cover.iter().all(|&n| n == 1), "every cell covered exactly once at {cols}x{rows}");
}

#[test]
fn photopia_fills_a_100x37_pane() {
    photopia_expect(100, 37);
}

#[test]
fn photopia_fills_a_160x45_pane() {
    photopia_expect(160, 45);
}

#[test]
fn photopia_fills_a_wide_short_pane() {
    photopia_expect(200, 20);
}

#[test]
fn narcolepsy_story_text_lands_on_the_stretched_rect() {
    let Some(path) = story("narco.blorb") else { return };
    for honor in [true, false] {
        let mut b = boot(path.clone());
        b.state.config.honor_game_colours = honor;
        assert!(b.state.glk_stretch);
        assert_eq!(Engine::pending_input(b.session.as_ref()), InputKind::Line, "0 inputs, first line prompt");
        let r = frame(&mut b, 100, 37);
        // Non-vacuity: two text windows (left navy half, story text) and five graphics windows.
        assert_eq!(of_kind(&r, WinKind::Buffer).len(), 2, "{r:?}");
        assert_eq!(of_kind(&r, WinKind::Graphics).len(), 5, "{r:?}");
        let l = glk_layout(b.session.as_mut()).unwrap();
        let story_px = (480, 60, 240, 378);
        let w = l.windows.iter().find(|w| {
            w.wintype == WinType::TextBuffer && (w.rect.left, w.rect.top, w.rect.width, w.rect.height) == story_px
        });
        let w = w.expect("story text laid out at (480,60,240,378)");
        let (x0, x1) = (edge(480, 800, 100) as u16, edge(720, 800, 100) as u16);
        let (y0, y1) = (edge(60, 600, 37) as u16, edge(438, 600, 37) as u16);
        let told = (l.screen.chars_in(w.rect.width, false) as u16, l.screen.chars_in(w.rect.height, true) as u16);
        let want = Rect::new(x0, y0, told.0.min(x1 - x0), told.1.min(y1 - y0));
        assert!(
            of_kind(&r, WinKind::Buffer).contains(&want),
            "story text at the stretched rect {want:?} (honor_game_colours={honor}): {r:?}"
        );
        assert_eq!((want.x, want.y, want.width, want.height), (60, 4, 30, 23));
    }
}

#[test]
fn restart_keeps_design_mode_and_survives_a_resize() {
    let Some(path) = story("photo201.blb") else { return };
    let mut b = boot(path);
    let (_, _) = (frame(&mut b, 100, 37), 0);
    let game_dir = b.game_dir.clone();
    reset_game(
        b.session.as_mut(),
        &mut b.mapper,
        &mut b.state,
        &b.story_bytes,
        &b.story_path,
        &game_dir,
        Some((100, 37)),
        ResetOptions::default(),
    );
    assert!(b.state.glk_stretch, "restart keeps stretch mode on");
    let l = glk_layout(b.session.as_mut()).unwrap();
    assert_eq!(l.screen.unit_px, (1, 1), "the fresh session is laid out in design pixels, not cells");
    assert_eq!(l.screen.size, (640, 480));
    // Perturb, then assert: a resize after the restart still lands on the design edges.
    let r = frame(&mut b, 160, 45);
    let gfx = of_kind(&r, WinKind::Graphics);
    assert!(gfx.contains(&Rect::new(0, 0, 160, edge(58, 480, 45) as u16)), "top frame after restart+resize: {gfx:?}");
    assert_eq!(gfx.len(), 4);
}

#[test]
fn the_per_game_switch_restores_cell_mode() {
    let Some(path) = story("photo201.blb") else { return };
    let home = app::scratch_dir("sq1703-switch");
    let mut b = boot_in(path.clone(), home.clone());
    assert!(b.state.glk_stretch);
    // Write the sidecar the way a player's choice persists, then boot again:
    // the boot path honours it.
    let side = app::styles::PerGameConfig { glk_design: Some(false), ..Default::default() };
    side.write(&b.game_dir).unwrap();
    let mut off = boot_in(path, home);
    assert!(!off.state.glk_stretch, "glk_design = false keeps cell mode");
    let l = glk_layout(off.session.as_mut()).unwrap();
    assert_eq!(l.screen.text_cell, (1.0, 1.0));
    assert_ne!(l.screen.size, (640, 480));
    // And the live switch toggles it back without a reboot.
    std::fs::remove_file(app::styles::per_game_config_path(&b.game_dir)).unwrap();
    assert!(apply_glk_design(off.session.as_mut(), &mut off.state, &b.game_dir));
    assert!(off.state.glk_stretch);
    assert_eq!(glk_layout(off.session.as_mut()).unwrap().screen.size, (640, 480));
    let _ = &mut b;
}

/// A click in a stretched graphics window reports DESIGN pixels inside it, the
/// inverse of the stretch: Photopia's top frame (design 640x58) drawn over
/// 100x4 cells at 100x37, clicked at cell (50,2) -> ((50+.5)*6.4, (2+.5)*14.5).
#[test]
fn a_click_in_a_stretched_graphics_window_reports_design_pixels() {
    let Some(path) = story("photo201.blb") else { return };
    let mut b = boot(path);
    let r = frame(&mut b, 100, 37);
    let top = r.iter().find(|(_, k, rc)| *k == WinKind::Graphics && *rc == Rect::new(0, 0, 100, 4)).copied().expect("top frame");
    let gs = b.session.as_any_mut().downcast_mut::<app::glulx_session::GlulxSession>().unwrap();
    let design_px = gs.design_graphics_px();
    assert!(design_px.contains(&(top.0, (640, 58))), "non-vacuity: {design_px:?}");
    let story_rect = (0, 0, 100, 37);
    let click = |sub| {
        app::glulx_session::glk_mouse_target_design(false, 50, 2, story_rect, &[top.0], &r, (8, 16), sub, &design_px)
    };
    assert_eq!(click(None), Some((top.0, 323, 36)));
    assert_eq!(click(Some((4, 8))), Some((top.0, 323, 36)));
    assert_eq!(click(Some((0, 0))), Some((top.0, 320, 29)));
    // Last cell of the window stays inside the canvas.
    let last = app::glulx_session::glk_mouse_target_design(false, 99, 3, story_rect, &[top.0], &r, (8, 16), Some((7, 15)), &design_px);
    assert!(matches!(last, Some((_, x, y)) if x < 640 && y < 58 && x > 630 && y > 50), "{last:?}");
    // Cell mode (no design table) is the old cells-times-char_px answer.
    let old = app::glulx_session::glk_mouse_target(false, 50, 2, story_rect, &[top.0], &r, (8, 16), None);
    assert_eq!(old, Some((top.0, 50 * 8 + 4, 2 * 16 + 8)));
}
