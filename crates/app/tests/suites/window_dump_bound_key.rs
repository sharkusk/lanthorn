//! SQ-0759: `/dump-windows` must be reachable by a bound key, and taking it that
//! way must not disturb the state it reports.
//!
//! The command was only ever reachable through the command palette or the hotkey
//! dialog, and both are modal overlays. A modal drops a graphical v6 pane off its
//! pixel ring onto the cell path, and coming back out re-uploads every chrome band
//! — so the palette route stacks modal frames into the render-path history and
//! bumps the band-upload counter, both of which are lines the dump prints. SQ-0756
//! made the dump *describe* the last game frame; the churn itself is this quest.
//!
//! The measurement is the point. A hotkey that skips the palette but still forces a
//! redraw or a path change has not solved anything, so every case here compares the
//! two counters across a capture — and the palette case is kept alongside as the
//! contrast that makes the bound-key case mean something.
//!
//! The frames use a synthetic v6 `Layered` model rather than a story file (the same
//! choice as `window_dump_last_game_frame.rs`), so they run in CI where `stories/`
//! is absent: what is under test is the dispatch route and the render bookkeeping.

use app::config::KeymapConfig;
use app::engine::{BufferWindow, GraphicsWindow, PositionedWindow, ScreenModel, StatusModel, WinNode};
use app::input::{key_to_command, KeyResolve};
use app::keymap::{Context, KeyMap, KeySpec};
use app::state::{AppState, Focus};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

// ── The v6 harness ────────────────────────────────────────────────────────────

/// A native 320x200 v6 screen: an opaque full-screen chrome graphics window with
/// the primary story buffer inset in it — enough for the hybrid ring to have a ring
/// to draw and a story viewport to place.
fn v6_model() -> ScreenModel {
    let chrome_img = image::RgbaImage::from_pixel(320, 200, image::Rgba([40, 30, 20, 255]));
    let chrome = PositionedWindow {
        x: 0, y: 0, w: 40, h: 25, x_px: 0, y_px: 0, w_px: 320, h_px: 200,
        left_margin: 0, right_margin: 0,
        node: WinNode::Graphics(GraphicsWindow {
            win: 7,
            canvas: std::sync::Arc::new(chrome_img),
            version: 1,
            upscale: false,
        }),
    };
    let story = PositionedWindow {
        x: 5, y: 4, w: 29, h: 20, x_px: 43, y_px: 39, w_px: 234, h_px: 160,
        left_margin: 0, right_margin: 0,
        node: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
    };
    ScreenModel {
        root: WinNode::Layered(vec![chrome, story]),
        status: StatusModel::HostManaged,
        bg: 0,
        fg: 0,
        content_size: (40, 25),
    }
}

fn v6_state() -> AppState {
    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.push_transcript("HELLO STORY WORLD");
    state
}

fn draw(state: &AppState, model: &ScreenModel) {
    let area = Rect::new(0, 0, 40, 25);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(model, false, None, state, area, &mut buf);
}

/// The two counters `/dump-windows` prints that the act of capturing can move:
/// the collapsed render-path history and the lifetime band-upload count.
fn counters(state: &AppState) -> (Vec<(String, u32)>, u64) {
    (state.v6_path_log.borrow().clone(), state.graphics_render.borrow().band_encodes)
}

/// A state whose keymap is exactly what the user typed into `config.toml`.
fn state_bound(entries: &[(&str, &str)]) -> (AppState, Vec<String>) {
    let mut cfg = KeymapConfig::default();
    for (key, cmd) in entries {
        cfg.global.insert((*key).to_string(), (*cmd).to_string());
    }
    let (keymap, warnings) = KeyMap::resolve(&cfg);
    let mut state = v6_state();
    state.keymap = keymap;
    state.focus = Focus::Game;
    (state, warnings)
}

// ── (a) The contract: which side is the key ───────────────────────────────────

/// The shipped template's own examples used to be written the other way round, so
/// pin the direction here as well as in `config_template`'s own guard: the LEFT
/// side is the key spec, the RIGHT side is the registry's hyphenated command.
///
/// FALSIFY by swapping the pair: `resolve` rejects it, exactly as it rejected every
/// user who copied the old template.
#[test]
fn the_left_side_is_the_key_and_the_right_side_the_command() {
    let (state, warnings) = state_bound(&[("f9", "dump-windows")]);
    assert!(warnings.is_empty(), "the documented form must resolve cleanly: {warnings:?}");
    assert_eq!(
        state.keymap.lookup(&"f9".parse::<KeySpec>().unwrap(), Context::Global),
        Some("dump-windows"),
        "\"f9\" = \"dump-windows\" must bind F9 to the command"
    );

    // The inverted form — what the old template taught — binds nothing.
    let (inverted, warnings) = state_bound(&[("dump_windows", "f9")]);
    assert!(inverted.keymap.lookup(&"f9".parse::<KeySpec>().unwrap(), Context::Global).is_none());
    assert_eq!(warnings.len(), 1, "the dropped binding's one warning is the only signal: {warnings:?}");
}

/// …and when it is inverted, the warning must name the real mistake. The binding is
/// silently skipped, so this string is the *only* thing the user ever sees, and
/// "cannot parse key 'dump_windows'" pointed at the wrong half of their line.
///
/// FALSIFY by restoring the unconditional `cannot parse key` message: the assertion
/// on "backwards" fails.
#[test]
fn an_inverted_entry_is_reported_as_inverted_not_as_a_bad_key() {
    let (_state, warnings) = state_bound(&[("dump_windows", "f9")]);
    let w = warnings.first().expect("one warning");
    assert!(w.contains("backwards"), "must say the line is inverted: {w}");
    assert!(
        w.contains("f9 = \"dump-windows\""),
        "must quote the corrected line, in the registry's own spelling: {w}"
    );
    assert!(!w.contains("unknown key token"), "must not blame the key: {w}");
}

/// The mirror case: the right spelling on the right side. The registry is
/// hyphenated and the old template said snake_case, so a half-corrected line is
/// the likeliest next attempt — it must not fail mutely either.
#[test]
fn a_snake_case_command_name_is_told_the_registry_spelling() {
    let (_state, warnings) = state_bound(&[("f9", "dump_windows")]);
    let w = warnings.first().expect("one warning");
    assert!(w.contains("'dump-windows'"), "must offer the registry spelling: {w}");
}

/// The keymap accepts the whole registry, not a privileged subset — the binding
/// path validates against `slash::find_command` and nothing else. Checked across
/// categories so a future gate on, say, `Category::Help` would fail here.
#[test]
fn the_keymap_binds_any_registry_command() {
    for cmd in ["dump-windows", "toggle-map", "save-state", "zoom-map in", "animate-tidy"] {
        let (state, warnings) = state_bound(&[("f9", cmd)]);
        assert!(warnings.is_empty(), "{cmd}: {warnings:?}");
        assert_eq!(
            state.keymap.lookup(&"f9".parse::<KeySpec>().unwrap(), Context::Global),
            Some(cmd),
            "{cmd} must be bindable"
        );
    }
}

// ── (b) The bound key reaches dispatch without a modal ────────────────────────

/// The whole point: the key resolves straight to the command string, with every
/// modal overlay still shut. One dispatch path, no palette.
///
/// FALSIFY by resolving the key to `Action::OpenCommandPalette` instead: the
/// `KeyResolve::Command` match fails and `any_modal_overlay_open` goes true.
#[test]
fn a_bound_key_dispatches_the_command_with_no_modal_open() {
    let (state, _) = state_bound(&[("f9", "dump-windows")]);
    assert!(!state.any_modal_overlay_open(), "precondition: nothing is open");

    match key_to_command(&state, KeyEvent::new(KeyCode::F(9), KeyModifiers::NONE)) {
        KeyResolve::Command(cmd, ctx) => {
            assert_eq!(cmd, "dump-windows");
            assert_eq!(ctx, Context::Global);
        }
        other => panic!("a bound key must resolve to the command itself, got {other:?}"),
    }
    // Resolution is pure — no overlay was opened on the way.
    assert!(!state.any_modal_overlay_open(), "the bound key must not open a modal");
}

/// A **Ctrl** binding must work too, and this is the one that matters in practice:
/// the run loop hands every plain keypress to a story that is waiting on one
/// (`read_char`), and F9 has a neutral form — `KeyInput::Func(9)` — so at Journey's
/// "[Press any key to begin]" a bare F9 answers the game instead of dumping. Ctrl
/// and Alt combos are the only classes that gate reserves for app routing.
///
/// Reaching a Ctrl binding means clearing `hotkeys.is_direct_name`, which is why
/// `dump-windows` joined the direct set.
///
/// FALSIFY by dropping `"dump-windows"` from `DEFAULT_DIRECT_COMMANDS`: this
/// resolves to `KeyResolve::None` and the key does nothing at all.
#[test]
fn a_ctrl_binding_survives_the_direct_set_filter() {
    let (state, warnings) = state_bound(&[("ctrl+d", "dump-windows")]);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(
        state.hotkeys.is_direct_name("dump-windows"),
        "dump-windows must be directly available, or no Ctrl binding for it can ever fire"
    );
    match key_to_command(&state, KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)) {
        KeyResolve::Command(cmd, _) => assert_eq!(cmd, "dump-windows"),
        other => panic!("Ctrl+D must dispatch the command, got {other:?}"),
    }
    assert!(!state.any_modal_overlay_open());
}

/// The same binding from map focus. `Context::Map` falls through to Global but
/// applies the same direct-set filter, so this is a second consumer of the fix —
/// and the dump is exactly the sort of thing you reach for while looking at the map.
#[test]
fn the_binding_also_fires_with_the_map_focused() {
    let (mut state, _) = state_bound(&[("ctrl+d", "dump-windows")]);
    state.focus = Focus::Map;
    match key_to_command(&state, KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)) {
        KeyResolve::Command(cmd, _) => assert_eq!(cmd, "dump-windows"),
        other => panic!("Ctrl+D in map focus must dispatch the command, got {other:?}"),
    }
}

// ── (c) The measurement: the capture must not move the counters ───────────────

/// The requirement, measured. Render a settled game frame, take the counters, take
/// the dump the way a bound key takes it, and take them again: identical.
///
/// "The way a bound key takes it" is precisely *nothing extra* — no overlay is set,
/// so the pane keeps drawing the same hybrid-ring frame it drew before. What this
/// case measures is that the ROUTE is quiet: the key resolves and the pane draws
/// on, unlike the palette route below. It stops at the resolver and never enters
/// the dispatch arm, so the arm's own quietness is guarded beside it, in
/// `slash_dispatch.rs`'s `dump_windows_moves_neither_the_band_count_nor_the_path_history`
/// (SQ-0777).
#[test]
fn a_bound_key_capture_leaves_the_render_path_and_upload_count_untouched() {
    let model = v6_model();
    let (state, _) = state_bound(&[("ctrl+d", "dump-windows")]);

    // Settle: the first frames upload the chrome bands, later ones hit the cache.
    for _ in 0..4 {
        draw(&state, &model);
    }
    let before = counters(&state);
    assert_eq!(
        before.0.last().map(|(l, _)| l.as_str()),
        Some("hybrid-ring"),
        "precondition: the settled game frame is on the pixel path"
    );
    assert!(before.1 > 0, "precondition: some bands have been uploaded");

    // The capture: the key resolves to the command, and the app draws on.
    assert!(matches!(
        key_to_command(&state, KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
        KeyResolve::Command(ref c, _) if c == "dump-windows"
    ));
    draw(&state, &model);

    let after = counters(&state);
    assert_eq!(after.1, before.1, "a bound-key capture must upload no bands");
    assert_eq!(
        after.0.iter().map(|(l, _)| l.clone()).collect::<Vec<_>>(),
        before.0.iter().map(|(l, _)| l.clone()).collect::<Vec<_>>(),
        "a bound-key capture must add no render path to the history"
    );
    // The frame the dump would describe is the live one, not something older.
    let (frame, line) = state.v6_dump_frame();
    assert!(frame.is_some(), "a game frame is on record");
    assert!(
        line.contains("the frame on screen now"),
        "no modal frames stand between the dump and the frame it reports: {line}"
    );
}

/// The contrast that gives the case above its meaning: the palette route moves both
/// counters, every time. Without this the test above would pass on a build where
/// the counters simply never change.
///
/// Two effects, both visible in the user's own capture: the modal frames push a
/// `cell — modal overlay open: palette` entry into the history, and resuming the
/// pixel path afterwards invalidates every chrome band so they all re-upload.
#[test]
fn the_palette_route_moves_both_counters_which_is_the_defect() {
    let model = v6_model();
    let mut state = v6_state();

    for _ in 0..4 {
        draw(&state, &model);
    }
    let before = counters(&state);
    assert_eq!(before.0.last().map(|(l, _)| l.as_str()), Some("hybrid-ring"));

    // Ctrl+P, then the frames the palette draws while the user types the command.
    state.overlays.palette = Some(app::state::PaletteState::new(false));
    for _ in 0..3 {
        draw(&state, &model);
    }
    // …and the pane coming back after the palette closes.
    state.overlays.palette = None;
    draw(&state, &model);

    let after = counters(&state);
    assert!(
        after.0.iter().any(|(l, _)| l.contains("modal overlay open: palette")),
        "the palette route stacks modal frames into the history: {:?}",
        after.0
    );
    assert!(
        after.1 > before.1,
        "returning from the palette re-uploads the chrome bands: {} → {}",
        before.1, after.1
    );

    // And the same capture through a bound key, from the same settled start, moves
    // neither — the two routes measured against one another.
    let (bound, _) = state_bound(&[("ctrl+d", "dump-windows")]);
    for _ in 0..4 {
        draw(&bound, &model);
    }
    let b0 = counters(&bound);
    draw(&bound, &model);
    let b1 = counters(&bound);
    assert_eq!((b1.0.len(), b1.1), (b0.0.len(), b0.1), "the bound-key route is inert");
}
