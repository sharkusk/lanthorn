//! SQ-0510: with no style.toml and no scheme, the upper window must follow the
//! TERMINAL's background rather than painting a black band across it.
//!
//! The reported symptom was Anchorhead's status bar: a black band on a terminal
//! whose background is not black. The story is not involved — `anchor.z8` issues
//! no `set_colour` at boot or in play, so every status cell packs to "default".
//! The black was ours: `resolve_base(None)` hands back an all-`Reset` scheme,
//! `Roles::from_scheme` early-returns to `Roles::terminal_default()`, and that
//! hard-codes `chrome: white on BLACK` — which `upper_window` inherits and
//! `draw_grid` fills the window from. Meanwhile the OSC 10/11 probe taken at
//! startup already knew the terminal's real page and was never handed to the
//! theme.
//!
//! These tests go through the loader the app actually uses (`reload::reload_style`
//! on an empty user dir — no style.toml, no scheme, exactly the reported setup)
//! and then through the real render path, reading the painted cells back.
//!
//! **Colour mode**: every case runs in BOTH `honor_game_colours` modes. Anchorhead
//! sets no colours either way, so the answer must not depend on the gate — and a
//! single-mode suite would hide it if it did.
//!
//! Gitignored fixture: the render half skips vacuously when `anchor.z8` is absent;
//! the loader half always runs.

use std::path::PathBuf;

use app::engine::{Engine, GridWindow, WinNode};
use app::render::upper_window::draw_upper_window;
use app::session::{GameSession, InputKind};
use app::state::AppState;
use app::term_colors::TermDefaultColors;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

/// A terminal page nobody would mistake for the hard-coded fallback: Solarized
/// Light's cream paper with its grey ink.
const TERM_FG: (u8, u8, u8) = (0x58, 0x6e, 0x75);
const TERM_BG: (u8, u8, u8) = (0xfd, 0xf6, 0xe3);

use crate::fixture_paths::fixture_path;


fn answered_probe() -> TermDefaultColors {
    TermDefaultColors {
        fg: Some(image::Rgba([TERM_FG.0, TERM_FG.1, TERM_FG.2, 255])),
        bg: Some(image::Rgba([TERM_BG.0, TERM_BG.1, TERM_BG.2, 255])),
    }
}

/// The reported configuration: an empty user dir (no `style.toml`), no `scheme`,
/// and whatever the terminal answered — loaded exactly the way startup and
/// `/reload style` load it.
fn state_with_probe(tag: &str, probe: TermDefaultColors, honor: bool) -> (AppState, PathBuf) {
    let dir = app::scratch_dir(&format!("uwtb-{tag}"));

    let mut state = AppState::default();
    state.config.user_dir = dir.clone();
    state.config.style = None;
    state.config.honor_game_colours = honor;
    state.honor_game_colours_base = honor;
    state.term_default_colors = probe;
    match app::reload::reload_style(&mut state) {
        app::reload::ReloadOutcome::Reloaded { .. } => {}
        app::reload::ReloadOutcome::Failed { msg } => panic!("the default look must load: {msg}"),
    }
    // `reload_style` recomputes the honor gate from disk; nothing on disk says
    // anything, so it lands back on the base we just set. Pin that assumption.
    assert_eq!(state.config.honor_game_colours, honor);
    (state, dir)
}

/// Anchorhead, driven past its two startup quote boxes to an ordinary turn whose
/// upper window is the one-row status line.
fn anchor_in_play() -> Option<GameSession> {
    let path = fixture_path("anchor.z8");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut s = GameSession::new_with_trace(
        bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)),
    )
    .expect("anchor.z8 should load and boot without a ZError");
    for _ in 0..4 {
        if matches!(s.pending_input(), InputKind::Char) {
            let _ = s.submit_char(13);
        }
    }
    let _ = s.submit("look");
    Some(s)
}

fn find_grid(n: &WinNode) -> Option<&GridWindow> {
    match n {
        WinNode::Grid(g) => Some(g),
        WinNode::Pair { first, second, .. } => find_grid(first).or_else(|| find_grid(second)),
        _ => None,
    }
}

/// Render the story's live upper window and report the distinct backgrounds the
/// status band was actually painted in, across every row it consumed.
fn painted_backgrounds(session: &GameSession, state: &AppState) -> Vec<Option<Color>> {
    let model = session.screen();
    let grid = find_grid(&model.root).expect("a v8 story in play has an upper window");
    assert!(grid.active_rows > 0, "the status line is one active row");

    let area = Rect::new(0, 0, 100, 10);
    let mut buf = Buffer::empty(area);
    let used = draw_upper_window(
        grid,
        false,
        &state.colors,
        area,
        &mut buf,
        state.config.honor_game_colours,
        &mut Vec::new(),
    );
    assert!(used > 0, "the upper window must occupy at least its content row");

    let mut seen: Vec<Option<Color>> = Vec::new();
    for y in area.y..area.y + used {
        for x in area.x..area.right() {
            let bg = buf.cell((x, y)).unwrap().style().bg;
            if !seen.contains(&bg) {
                seen.push(bg);
            }
        }
    }
    seen
}

// ── The reported bug ─────────────────────────────────────────────────────────

/// No style.toml, no scheme, a terminal that answered the probe: Anchorhead's
/// status band is painted in the TERMINAL's page — never the hard-coded black.
#[test]
fn anchor_status_band_follows_the_probed_terminal_background() {
    for honor in [true, false] {
        let (state, dir) = state_with_probe(&format!("on-{honor}"), answered_probe(), honor);

        // The theme half, which runs with or without the fixture.
        let uw = state.colors.theme.get("upper_window").style;
        assert_eq!(
            uw.bg,
            Some(Color::Rgb(TERM_BG.0, TERM_BG.1, TERM_BG.2)),
            "honor={honor}: upper_window must resolve to the probed terminal page"
        );

        // The render half: the real story, really painted.
        if let Some(session) = anchor_in_play() {
            let bgs = painted_backgrounds(&session, &state);
            assert!(
                !bgs.contains(&Some(Color::Black)),
                "honor={honor}: no black band may be painted on a non-black terminal: {bgs:?}"
            );
            assert!(
                bgs.contains(&Some(Color::Rgb(TERM_BG.0, TERM_BG.1, TERM_BG.2))),
                "honor={honor}: the band is painted in the terminal's own page: {bgs:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// The control, and the compatibility guarantee: a terminal that answers nothing
/// keeps today's behaviour exactly — the band is the hard-coded black, because
/// there is no better answer to be had. Without this the test above could pass
/// by simply dropping the chrome background altogether.
#[test]
fn an_unanswered_probe_keeps_the_hard_coded_fallback_band() {
    for honor in [true, false] {
        let (state, dir) =
            state_with_probe(&format!("off-{honor}"), TermDefaultColors::default(), honor);

        assert_eq!(
            state.colors.theme.get("upper_window").style.bg,
            Some(Color::Black),
            "honor={honor}: an unanswered probe leaves the terminal-default roles alone"
        );

        if let Some(session) = anchor_in_play() {
            let bgs = painted_backgrounds(&session, &state);
            assert!(
                bgs.contains(&Some(Color::Black)),
                "honor={honor}: today's behaviour is unchanged when the terminal is silent: {bgs:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A HALF answer (fg without bg) is skipped whole, matching the rule
/// `colors::host_default_colour_pair` already documents: page and ink always come
/// from the same design, so a probed ink is never mixed onto a guessed page.
#[test]
fn a_half_answered_probe_is_skipped_whole() {
    let probe = TermDefaultColors {
        fg: Some(image::Rgba([TERM_FG.0, TERM_FG.1, TERM_FG.2, 255])),
        bg: None,
    };
    let (state, dir) = state_with_probe("half", probe, true);
    let uw = state.colors.theme.get("upper_window").style;
    assert_eq!(uw.bg, Some(Color::Black), "half an answer is no answer");
    assert_eq!(
        uw.fg,
        Some(Color::White),
        "and the probed ink must not be mixed onto the guessed page"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
