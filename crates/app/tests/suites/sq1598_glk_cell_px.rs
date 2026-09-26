//! SQ-1598: a host-facing way to set the pixel size a Glulx game's Glk cell is
//! measured in, both at boot (`host::boot::TerminalFacts::glk_cell_px`) and on
//! resize (`host::screen::set_glk_cell_px`) — so a host whose own text cells
//! are not the constructor's 8×16 fallback (a proportional-font frontend, or
//! any host stating its own pixel size directly) gets Glk graphics canvases
//! matching its actual windows 1:1, `(window cells) × (its own cell size)`,
//! instead of stretching every canvas unevenly to compensate.
//!
//! Specimen: `Kerkerkruip.gblorb`, driven by the exact `" "` + `"look"` script
//! `sq1565_kerkerkruip_title_rule_pixel_height.rs` and
//! `sq1515_kerkerkruip_restore_arrange.rs` already use. Those suites' own
//! module docs warn that window ids drift across a REAL PLAY SESSION (panel
//! close/reopen hands out fresh, higher ids) — but for a FRESH boot driven by
//! this exact deterministic script, the ids come out identical every time:
//! confirmed here by three independent process runs at three different cell
//! sizes ((8,16), (10,23), (13,29)) while writing this suite, all producing
//! the same tree shape and the same ids (4, 7, 9, 11, 13, 15, 17, 19, 21, 23,
//! 25, 27, 29, 31, 33, 35, 37, 39, 41, 43, 45). So — unlike those suites,
//! which deliberately find their specimens by SHAPE because their own scripts
//! run deeper (help-screen open/close) — hard-coding a few representative ids
//! for this SAME short script is safe and keeps the assertions legible.
//!
//! Two shapes cover everything `canvas_size` (`glk_backend.rs`) produces:
//! `win=19`/`win=33` are panel graphics windows sized purely by CELLS on both
//! axes (no `fixed_px` on either); `win=29`/`win=39` are Kerkerkruip's
//! pixel-exact divider rules (`sq1565`'s own specimen) with one axis pinned to
//! an absolute pixel count and the other cell-driven.

use std::path::{Path, PathBuf};
use std::time::Duration;

use app::config::{Config, FALLBACK_SCREEN_COLS};
use app::engine::{Engine, GraphicsWindow, KeyInput, WinNode};
use app::glk_backend::GlkStylePairs;
use app::glulx_session::GlulxSession;
use app::host::{boot_story, BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts};
use app::launch_options::LaunchOverrides;
use app::session::InputKind;

use crate::fixture_paths::fixture_path;

const STORY: &str = "Kerkerkruip.gblorb";
const COLS: u16 = 225;
const ROWS: u16 = 51;
const POLL_CAP: usize = 8000;
const TURN_BUDGET: Duration = Duration::from_secs(600);
/// The title-art graphics window's cell height on a fresh `boot_story` boot
/// at the fallback 80×24 pane — measured at two cell sizes while writing this
/// suite (384px at (8,16), 552px at (10,23); both divide out to 24 exactly).
const TITLE_ART_ROWS: u32 = 24;

fn story_path() -> Option<PathBuf> {
    let p = fixture_path(STORY);
    if !p.is_file() {
        eprintln!("SKIP: fixture missing at {}", p.display());
        return None;
    }
    if !p.with_file_name("Kerkerkruip.ini").is_file() {
        eprintln!("SKIP: needs the shipped Kerkerkruip.ini beside the story");
        return None;
    }
    Some(p)
}

fn theme_pairs_for(path: &Path) -> GlkStylePairs {
    let mut cs = app::colors::ColorScheme::default();
    if let Some(ov) = app::garglk_ini::discover(path) {
        ov.apply(&mut cs);
    }
    app::glk_backend::theme_style_colours(&cs)
}

/// Boot Kerkerkruip directly through the same constructor `boot_story` itself
/// calls, at an explicit `cell_px` — the level `sq1565`/`sq1515` already test
/// at, parametrized on cell size for this suite's own purposes.
fn boot(path: &Path, cell_px: (u32, u32)) -> GlulxSession {
    let bytes = std::fs::read(path).expect("read the story");
    let blorb = blorb::Blorb::parse(bytes).expect("Kerkerkruip is a Blorb");
    let image = blorb.executable().expect("Glulx exec chunk").1.to_vec();
    let mut sess = GlulxSession::new_in(
        PathBuf::new(),
        image,
        COLS as u32,
        ROWS as u32,
        true,
        true,
        false,
        false,
        cell_px,
        Some(blorb),
        &[],
        theme_pairs_for(path),
        false,
        None,
    )
    .expect("Kerkerkruip boots");
    sess.set_turn_budget(TURN_BUDGET);
    sess
}

fn settle(sess: &mut GlulxSession, done: impl Fn(&GlulxSession) -> bool) -> bool {
    for _ in 0..POLL_CAP {
        if done(sess) {
            return true;
        }
        let can_advance =
            sess.timer_interval().is_some() || Engine::pending_input(sess) == InputKind::Event;
        if !can_advance {
            return done(sess);
        }
        let _ = sess.deliver_timer();
    }
    done(sess)
}

fn into_gameplay(cell_px: (u32, u32)) -> Option<GlulxSession> {
    let mut sess = boot(&story_path()?, cell_px);
    settle(&mut sess, |s| Engine::pending_input(s) != InputKind::Event);
    let _ = Engine::submit_key(&mut sess, KeyInput::Char(' '));
    settle(&mut sess, |s| Engine::pending_input(s) == InputKind::Line);
    assert_eq!(Engine::pending_input(&sess), InputKind::Line, "reached the parser prompt");
    let _ = Engine::submit(&mut sess, "look");
    settle(&mut sess, |s| Engine::pending_input(s) != InputKind::Event);
    Some(sess)
}

/// Find a graphics window's live canvas by its window id, anywhere in the
/// tree (see the module doc for why a fixed id is safe for THIS script).
fn find_graphics(sess: &GlulxSession, win: u32) -> GraphicsWindow {
    fn walk(node: &WinNode, win: u32) -> Option<GraphicsWindow> {
        match node {
            WinNode::Graphics(g) if g.win == win => Some(g.clone()),
            WinNode::Pair { first, second, .. } => walk(first, win).or_else(|| walk(second, win)),
            _ => None,
        }
    }
    walk(&Engine::screen(sess).root, win).unwrap_or_else(|| panic!("graphics window {win} not found in the tree"))
}

/// The one graphics window a FRESH Kerkerkruip boot has opened before the
/// intro's first Arrange (its title art), found by kind — `boot_story`'s own
/// pipeline resolves the story through more steps than [`boot`] above, so its
/// window ids are not guaranteed to match those tests' hard-coded ones.
fn only_graphics_window(session: &dyn Engine) -> GraphicsWindow {
    fn walk(node: &WinNode) -> Option<GraphicsWindow> {
        match node {
            WinNode::Graphics(g) => Some(g.clone()),
            WinNode::Pair { first, second, .. } => walk(first).or_else(|| walk(second)),
            _ => None,
        }
    }
    walk(&session.screen().root).expect("a fresh Kerkerkruip boot has one graphics window (title art)")
}

fn headless_config(home: &Path) -> Config {
    Config {
        user_dir: home.to_path_buf(),
        config_file: home.join("config.toml"),
        random_seed: Some(1),
        auto_save: true,
        ..Config::default()
    }
}

fn boot_via_terminal_facts(story: PathBuf, home: &Path, glk_cell_px: Option<(u32, u32)>) -> BootedStory {
    let overrides = LaunchOverrides::default();
    let req = BootRequest {
        story_path: story,
        disk_entry: None,
        overrides: &overrides,
        cfg: headless_config(home),
        data_base: home.join("saves"),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts { glk_cell_px, ..TerminalFacts::default() },
    };
    boot_story(req, &mut QuietBoot).expect("the story boots headlessly")
}

// ── The constructor mechanism: char_px reaches every graphics canvas ───────

/// A graphics canvas is allocated `(window cells) × char_px` (`canvas_size`,
/// `glk_backend.rs`), so a cell size genuinely different from the 8×16
/// fallback must reach every graphics window, not just the ones a resize
/// later touches. Every dimension asserted here was measured against a real
/// boot at three cell sizes before being pinned (see the module doc).
#[test]
fn host_cell_size_reaches_every_graphics_canvas_not_just_8x16() {
    const HOST_PX: (u32, u32) = (10, 23);
    let Some(mut sess) = into_gameplay(HOST_PX) else { return };
    assert_eq!(sess.char_pixels(), HOST_PX);

    // Two panels sized purely by cells on both axes: 54 and 45 cells wide,
    // one text row tall — neither axis is pinned to an absolute pixel count.
    assert_eq!(
        find_graphics(&sess, 19).canvas.dimensions(),
        (54 * HOST_PX.0, HOST_PX.1),
        "win=19: a plain cell-driven panel must scale on BOTH axes with the host's cell size"
    );
    assert_eq!(
        find_graphics(&sess, 33).canvas.dimensions(),
        (45 * HOST_PX.0, HOST_PX.1),
        "win=33: same shape as win=19, a second specimen"
    );

    // The full-height side rule: 1 cell wide, the whole ROWS-cell pane tall.
    assert_eq!(
        find_graphics(&sess, 11).canvas.dimensions(),
        (HOST_PX.0, ROWS as u32 * HOST_PX.1),
        "win=11: a full-height rule scales on both axes too"
    );

    // sq1565's own pixel-exact dividers: the FIXED axis (the game's own
    // absolute pixel request) must stay exactly what the game asked for,
    // untouched by the host's cell size; the other (cell-driven) axis scales.
    assert_eq!(
        find_graphics(&sess, 29).canvas.dimensions(),
        (2, ROWS as u32 * HOST_PX.1),
        "win=29: fixed 2px WIDTH unaffected by cell size; full-pane HEIGHT scales"
    );
    assert_eq!(
        find_graphics(&sess, 39).canvas.dimensions(),
        (115 * HOST_PX.0, 7),
        "win=39: fixed 7px HEIGHT unaffected by cell size; 115-cell WIDTH scales"
    );
}

// ── Boot-time plumbing: TerminalFacts::glk_cell_px → boot_story ────────────

/// The host-facing entry point end to end: `TerminalFacts::glk_cell_px`
/// reaches the real `boot_story` pipeline's `GlulxSession`, and a host that
/// leaves it `None` keeps today's honest 8×16 fallback (no picker in a
/// headless `TerminalFacts::default()`) exactly as before.
#[test]
fn terminal_facts_glk_cell_px_reaches_the_boot_story_pipeline() {
    let Some(story) = story_path() else { return };

    let home_host = app::scratch_dir("sq1598-boot-host-px");
    let mut host = boot_via_terminal_facts(story.clone(), &home_host, Some((10, 23)));
    {
        let gs = host.session.as_any_mut().downcast_mut::<GlulxSession>().expect("Glulx story");
        assert_eq!(gs.char_pixels(), (10, 23), "TerminalFacts::glk_cell_px reaches the constructor's char_px");
    }
    let title = only_graphics_window(&*host.session);
    assert_eq!(
        title.canvas.dimensions(),
        (FALLBACK_SCREEN_COLS as u32 * 10, TITLE_ART_ROWS * 23),
        "the title-art canvas scales with the host's own cell size"
    );
    let _ = std::fs::remove_dir_all(&home_host);

    let home_default = app::scratch_dir("sq1598-boot-default-px");
    let mut default = boot_via_terminal_facts(story, &home_default, None);
    {
        let gs = default.session.as_any_mut().downcast_mut::<GlulxSession>().expect("Glulx story");
        assert_eq!(gs.char_pixels(), (8, 16), "no host override and no picker: the unchanged 8x16 fallback");
    }
    let title2 = only_graphics_window(&*default.session);
    assert_eq!(
        title2.canvas.dimensions(),
        (FALLBACK_SCREEN_COLS as u32 * 8, TITLE_ART_ROWS * 16),
        "and the SAME story at the fallback cell size renders a genuinely different (smaller) canvas"
    );
    let _ = std::fs::remove_dir_all(&home_default);
}

// ── Resize-time: host::screen::set_glk_cell_px ──────────────────────────────

/// Changing the cell size on an ALREADY-BOOTED session resizes every open
/// graphics canvas live — `GlulxSession::set_char_px` calls
/// `Machine::rearrange()` (`relayout_glk` + `deliver_arrange`) and
/// `refresh_screen()` exactly as `resize`/`set_borderless` already do for
/// their own field, the same mechanism a terminal resize already uses to get
/// the game to redraw. Kerkerkruip's gameplay-screen panels are flat colour
/// fills (`Canvas::resize`'s own blank-to-background refill already looks
/// identical to a genuine repaint for a solid panel — confirmed empirically
/// while writing this suite, by diffing pixel content with the Arrange call
/// temporarily removed), so this test's falsifiable claim is the dimension
/// change itself, not an independent pixel-content proof that this specimen
/// cannot supply; the Arrange delivery is verified by code-path equivalence
/// with `resize`/`set_borderless` instead (see `set_char_px`'s own doc).
#[test]
fn set_glk_cell_px_resizes_open_graphics_canvases_live() {
    const FROM_PX: (u32, u32) = (8, 16);
    const TO_PX: (u32, u32) = (13, 29);
    let Some(mut sess) = into_gameplay(FROM_PX) else { return };

    let before = find_graphics(&sess, 19);
    assert_eq!(before.canvas.dimensions(), (54 * FROM_PX.0, FROM_PX.1));

    let changed = app::host::screen::set_glk_cell_px(&mut sess, TO_PX);
    assert!(changed, "a Glulx session accepts the cell-size change");
    assert_eq!(sess.char_pixels(), TO_PX);
    assert!(!Engine::has_quit(&sess), "resizing must not itself end the game");

    let after = find_graphics(&sess, 19);
    assert_eq!(
        after.canvas.dimensions(),
        (54 * TO_PX.0, TO_PX.1),
        "an already-booted game's graphics canvas resizes live to the new cell size"
    );

    // The pixel-exact divider's FIXED axis must be untouched by the change —
    // only the cell-driven axis moves.
    let rule = find_graphics(&sess, 29);
    assert_eq!(rule.canvas.width(), 2, "the game's own 2px request is unaffected by the host's cell size");
    assert_eq!(rule.canvas.height(), ROWS as u32 * TO_PX.1);
}

/// A cell-size change delivered while no engine other than Glulx is running
/// is a no-op that reports `false`, the same contract `resize_glulx` has for
/// a Z-machine session (`host::screen::resize_glulx`'s own doc).
#[test]
fn set_glk_cell_px_is_a_no_op_for_a_non_glulx_engine() {
    // A trivial v3 story: version byte + a QUIT opcode at the initial PC, just
    // enough to construct a `GameSession` — this test only needs an engine
    // that is not Glulx, not a playable game.
    let mut buf = vec![0u8; 0x0100];
    buf[0x00] = 3;
    buf[0x04] = 0x00; // high memory
    buf[0x06] = 0x40; // initial PC (v3: PC itself, not PC-1)
    buf[0x08] = 0x40; // dictionary (empty-ish, unused)
    buf[0x0A] = 0x10; // objects
    buf[0x0C] = 0x20; // globals
    buf[0x0E] = 0x00; // static memory
    buf[0x40] = 0xBA; // QUIT
    let mut sess = app::session::GameSession::new(buf, true, false, None).expect("a minimal v3 story boots");
    assert!(!app::host::screen::set_glk_cell_px(&mut sess, (10, 23)), "not a Glulx engine");
}

// ── @restart parity: AppState::glk_cell_px survives reset_game ─────────────

/// An `@restart` must agree with the launch's host-declared cell size, not
/// silently revert to the picker/8×16 fallback — `reset.rs` derives its own
/// `char_px` from persisted `AppState` fields (there is no fresh
/// `TerminalFacts` on a restart), so the launch's choice has to be carried
/// forward onto `AppState::glk_cell_px` for this to hold.
#[test]
fn restart_keeps_the_hosts_glk_cell_px_not_the_8x16_fallback() {
    let Some(story) = story_path() else { return };
    let home = app::scratch_dir("sq1598-restart");
    let mut b = boot_via_terminal_facts(story, &home, Some((10, 23)));
    {
        let gs = b.session.as_any_mut().downcast_mut::<GlulxSession>().expect("Glulx story");
        assert_eq!(gs.char_pixels(), (10, 23), "premise: the launch used the host's cell size");
    }
    assert_eq!(b.state.glk_cell_px, Some((10, 23)), "carried onto AppState for reset.rs to re-read");

    app::host::reset::reset_game(
        &mut *b.session,
        &mut b.mapper,
        &mut b.state,
        &b.story_bytes,
        &b.story_path,
        &b.game_dir,
        None,
        app::host::reset::ResetOptions::default(),
    );

    let gs = b.session.as_any_mut().downcast_mut::<GlulxSession>().expect("still Glulx after @restart");
    assert_eq!(
        gs.char_pixels(),
        (10, 23),
        "an @restart must agree with the launch's host-declared cell size, not the picker/8x16 fallback"
    );
    let _ = std::fs::remove_dir_all(&home);
}
