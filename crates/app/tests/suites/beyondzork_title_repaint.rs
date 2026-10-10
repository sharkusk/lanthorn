//! SQ-0748: Beyond Zork's title repaint clears the screen it was typed on.
//!
//! From the opening screen you answer the VT220 question and type `BEGIN`; the
//! game plays the "Our doom is sealed" prologue, then — on the next keypress —
//! repaints its centred title. Measured from the screen trace, that repaint is
//! one game-driven turn emitting, in order:
//!
//! ```text
//!   @erase_window(all(unsplit))   … erase_window(-1): whole screen, unsplit
//!   @split_window(20)
//!   @set_cursor(row=8,  col=24)   … "B E Y O N D   Z O R K"
//!   @set_cursor(row=9,  col=29)   … "The Coconut of Quendor"
//!   @set_cursor(row=11, col=29)   … "An Interactive Fantasy"
//!   @set_cursor(row=12, col=14)   … the copyright line
//!   @set_cursor(row=13, col=16)   … the trademark line
//! ```
//!
//! Every character of that title is PLACED in the upper window. The lower window
//! gets nothing at all: the turn's story text is empty. So the transcript's
//! screen-clear anchor lands at exactly `transcript.len()` — cleared, and nothing
//! printed since — and `anchor_row_at` read that one-past-the-end index as "no
//! anchor" rather than "an empty screen". With no anchor the viewport bottom-sticks,
//! and the five rows below the 20-row title showed the tail of the very screen the
//! game had just erased: the copyright block, "Do you want to begin a new game…",
//! and the `[Type BEGIN, RESTORE or QUIT.] >begin` line the command was typed on.
//!
//! The story is gitignored (CLAUDE.md), so this skips vacuously when absent.

use std::path::PathBuf;

use app::engine::Engine;
use app::session::{GameSession, InputKind, TurnResult};
use app::state::{AppState, TranscriptKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// The pane the app renders the story into, in cells — and the screen size the
/// game is told about, so its centring columns are the ones asserted below.
const PANE: (u16, u16) = (80, 25);

/// Text from the screens BEGIN was typed on and above. None of it may survive the
/// repaint. (The copyright line is deliberately absent: the centred title prints
/// its own, so it is not evidence either way.)
const PRE_BEGIN: [&str; 3] = [
    "Do you want to begin a new game",
    "Type BEGIN, RESTORE or QUIT",
    "Is this a VT220?",
];

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn boot(honor: bool) -> Option<GameSession> {
    let path = stories_dir().join("beyondzork-r57-s871221.z5");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    Some(
        GameSession::new_with_trace(
            bytes,
            honor,
            false,
            None,
            true,
            Vec::new(),
            None,
            None,
            Some((PANE.1, PANE.0)),
        )
        .expect("beyondzork-r57-s871221.z5 should load and boot without a ZError"),
    )
}

/// The transcript half of `turn::finish_command_turn`, for a submitted command
/// line in the shipped inline-prompt mode (`command_bar = false`): mark the
/// clear the turn opened with, fold the typed command onto the game's own `>`,
/// then push the turn's output.
fn apply_command(state: &mut AppState, cmd: &str, r: &TurnResult) {
    if r.erase_lower {
        state.mark_screen_clear();
    }
    if state.last_transcript_line_is_story() {
        state.append_to_last_transcript_line(cmd, Default::default());
    } else {
        state.push_transcript_kind(&format!("> {}", cmd), TranscriptKind::Input);
    }
    state.push_transcript_runs(&r.transcript, TranscriptKind::Story, &r.transcript_runs);
}

/// The transcript half of `turn::apply_game_driven_result`, for a `read_char`
/// keypress: a clear collapses the previous reprint back to its anchor, re-marks,
/// then pushes whatever the turn printed.
fn apply_char(state: &mut AppState, r: &TurnResult) {
    if r.erase_lower {
        if let Some(anchor) = state.clear_anchor {
            state.truncate_transcript(anchor);
        }
        state.mark_screen_clear();
    }
    state.push_transcript_runs(&r.transcript, TranscriptKind::Story, &r.transcript_runs);
}

/// Boot, answer the VT220 question, type BEGIN, and press a key through the
/// prologue to the title repaint. Returns the session, the app state, and the
/// screen ops the repainting turn emitted.
fn beyond_zork_title(honor: bool) -> Option<(GameSession, AppState, Vec<String>)> {
    let mut s = boot(honor)?;
    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.config.honor_game_colours = honor;

    let boot_text = Engine::take_transcript(&mut s);
    state.push_transcript_runs(&boot_text, TranscriptKind::Story, &[]);
    let _ = s.take_screen_trace();

    for cmd in ["no", "begin"] {
        assert!(
            matches!(Engine::pending_input(&s), InputKind::Line),
            "honor={honor}: the opening screens read a line"
        );
        let r = s.submit(cmd);
        assert!(r.fault.is_none(), "honor={honor}: {cmd:?} faulted: {:?}", r.fault);
        apply_command(&mut state, cmd, &r);
        let _ = s.take_screen_trace();
    }

    assert!(
        matches!(Engine::pending_input(&s), InputKind::Char),
        "honor={honor}: the prologue waits for a keypress"
    );
    let r = s.submit_char(13);
    assert!(r.fault.is_none(), "honor={honor}: the title repaint faulted: {:?}", r.fault);
    apply_char(&mut state, &r);

    // Premise: the repaint is a whole-screen erase, a 20-row split, and NOTHING
    // printed below. Every character of the title is placed in the upper window.
    let ops = s.take_screen_trace();
    assert!(
        ops.contains(&"@erase_window(all(unsplit))".to_string()),
        "honor={honor}: premise — the repaint opens with erase_window(-1): {ops:?}"
    );
    assert!(
        ops.contains(&"@split_window(20)".to_string()),
        "honor={honor}: premise — …and splits 20 rows off for the title: {ops:?}"
    );
    assert!(
        r.transcript.trim().is_empty(),
        "honor={honor}: premise — the title turn prints nothing into the lower window: {:?}",
        r.transcript
    );

    Some((s, state, ops))
}

/// The pane's rows, as text, exactly as `render_story_pane` draws them.
fn pane_rows(session: &GameSession, state: &AppState) -> Vec<String> {
    let area = Rect::new(0, 0, PANE.0, PANE.1);
    let mut buf = Buffer::empty(area);
    let model = Engine::screen(session);
    let char_mode = matches!(Engine::pending_input(session), InputKind::Char);
    app::render::screen::render_story_pane(&model, char_mode, None, state, area, &mut buf);
    (0..PANE.1)
        .map(|y| {
            (0..PANE.0)
                .map(|x| {
                    buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' ')
                })
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

fn title_repaint_leaves_only_the_title(honor: bool) {
    let Some((session, state, _ops)) = beyond_zork_title(honor) else { return };
    let rows = pane_rows(&session, &state);

    // The title itself is on the pane — without this the assertions below would
    // pass on a blank screen.
    assert!(
        rows.iter().any(|r| r.contains("B  E  Y  O  N  D")),
        "honor={honor}: the centred title must be on the pane: {rows:#?}"
    );
    assert!(
        rows.iter().any(|r| r.contains("The Coconut of Quendor")),
        "honor={honor}: …with its subtitle: {rows:#?}"
    );

    // …and nothing else is. The pre-BEGIN screen was erased.
    for needle in PRE_BEGIN {
        assert!(
            !rows.iter().any(|r| r.contains(needle)),
            "honor={honor}: {needle:?} is the screen BEGIN was typed on — the game erased it, so \
             it must not survive under the title: {rows:#?}"
        );
    }
    // The typed command rides folded onto the game's own prompt line; that whole
    // line went with the erase.
    assert!(
        !rows.iter().any(|r| r.contains(">begin")),
        "honor={honor}: the line BEGIN was typed on must not survive either: {rows:#?}"
    );
    // The prologue the erase replaced is gone too.
    assert!(
        !rows.iter().any(|r| r.contains("Our doom is sealed")),
        "honor={honor}: the prologue the title replaced must not survive: {rows:#?}"
    );

    // The clear itself is scrollback-preserving, not destructive: the erased
    // screen stays reachable above the anchor.
    let anchor = state.clear_anchor.expect("honor={honor}: the repaint marks a screen clear");
    assert_eq!(
        anchor,
        state.transcript.len(),
        "honor={honor}: cleared with nothing printed since — the anchor sits at the very end, \
         which is what used to read as 'no anchor'"
    );
    assert!(
        state.transcript.iter().any(|l| l.contains("Do you want to begin a new game")),
        "honor={honor}: the erased screen is still in scrollback, just not on the screen"
    );
}

#[test]
fn title_repaint_leaves_only_the_title_honoring_game_colours() {
    title_repaint_leaves_only_the_title(true);
}

#[test]
fn title_repaint_leaves_only_the_title_theme_only() {
    title_repaint_leaves_only_the_title(false);
}

/// The next screen — Character Setup — prints prose below its 14-row split, and
/// that prose must be top-anchored under the menu with the erased title gone.
/// Guards the fix from over-reaching: an anchor at the end blanks the box, but an
/// anchor with content after it still shows that content.
fn character_setup_shows_its_own_prose(honor: bool) {
    let Some((mut session, mut state, _ops)) = beyond_zork_title(honor) else { return };
    assert!(matches!(Engine::pending_input(&session), InputKind::Char));
    let r = session.submit_char(13);
    assert!(r.fault.is_none(), "honor={honor}: Character Setup faulted: {:?}", r.fault);
    apply_char(&mut state, &r);

    let rows = pane_rows(&session, &state);
    assert!(
        rows.iter().any(|r| r.contains("Character Setup")),
        "honor={honor}: the menu is placed in the upper window: {rows:#?}"
    );
    assert!(
        rows.iter().any(|r| r.contains("Use the UP and DOWN arrow keys")),
        "honor={honor}: the prose this screen DID print must still show: {rows:#?}"
    );
    for needle in PRE_BEGIN {
        assert!(
            !rows.iter().any(|r| r.contains(needle)),
            "honor={honor}: {needle:?} must not resurface here either: {rows:#?}"
        );
    }
}

#[test]
fn character_setup_shows_its_own_prose_honoring_game_colours() {
    character_setup_shows_its_own_prose(true);
}

#[test]
fn character_setup_shows_its_own_prose_theme_only() {
    character_setup_shows_its_own_prose(false);
}
