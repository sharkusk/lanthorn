//! Story-pane follow-ease on new output (SQ-1595).
//!
//! Before this, a turn's new text simply appeared: `push_transcript*` never
//! touches `transcript_scroll`/`scroll_anim` at all, so the next frame just
//! shows the new bottom with no animation. `[animation]` already eases the
//! story pane's own explicit scroll actions and the hint panel's — this gives
//! new output arriving at the bottom the same treatment, on its own timing key
//! (`follow_ms`), without moving a reader who scrolled into history.
//!
//! The mechanics — arming, clamping to the `[more]` pager's park, and the
//! event-driven cancellation — are pinned as plain unit tests next to what
//! they touch (`crates/app/src/pager.rs`, `crates/app/src/state.rs`, both
//! `t-render`/`t-state`), which is where the actual arithmetic lives and
//! where `more_pager_first_new_row.rs`'s own SETTLED-position assertions
//! already prove the fix doesn't move where anything parks. This suite is the
//! one thing only a rendered frame, driven through a real session, can show:
//! that a real game turn — not a synthetic `ScreenModel` — reaches the same
//! follow-ease, and that turning it off (`follow_ms = 0`) reproduces the old
//! instant jump byte-for-byte.

use std::path::PathBuf;

use app::engine::Engine;
use app::session::GameSession;
use app::state::AppState;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// A fresh, non-v6 session at its opening prompt — no picture pipeline, no
/// medium/release pinning needed, since this suite's subject (the follow-ease
/// arming decision) is engine- and machine-neutral, exactly like the pager it
/// rides on.
fn boot() -> Option<GameSession> {
    let path = stories_dir().join("cutthroats-r23-s840809.z3");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    Some(
        GameSession::new_with_trace(bytes, true, false, None, false, Default::default(), None, None, None)
            .expect("Cutthroats boots"),
    )
}

/// Flush whatever the boot banner buffered into the host transcript, exactly as
/// `startup.rs` does before the reader ever sees a prompt — otherwise the
/// first command's turn measures the WHOLE unflushed banner as "added by this
/// turn" (mirrors `more_pager_first_new_row.rs`'s `park_with`).
fn flush_boot(s: &mut GameSession, state: &mut AppState) {
    let pending = s.take_transcript();
    if !pending.is_empty() {
        state.push_transcript(&pending);
    }
}

fn render(state: &AppState, s: &GameSession, area: Rect) -> (app::render::screen::StoryPaneMetrics, Buffer) {
    let mut buf = Buffer::empty(area);
    let m = app::render::screen::render_story_pane(&s.screen(), false, None, state, area, &mut buf);
    (m, buf)
}

/// Drive one command's worth of output through the exact arm → frame →
/// activate → frame cycle the run loop uses (`host::turn::finish_command_turn`
/// arms; `pager::apply_frame` activates on the next render).
fn drive_one_turn(s: &mut GameSession, state: &mut AppState, area: Rect, cmd: &str) {
    let (m0, _) = render(state, s, area);
    state.last_transcript_total_rows = m0.total_rows;

    let t = s.submit(cmd);
    state.push_transcript_runs(&t.transcript, app::state::TranscriptKind::Story, &t.transcript_runs);
    state.pager.arm_after_turn(
        state.last_transcript_total_rows,
        s.pending_input(),
        app::pager::more_suppressed(s),
        app::pager::Driver::PlayerInput,
    );

    let (m1, _) = render(state, s, area);
    app::pager::apply_frame(
        state,
        m1.max_scroll,
        m1.viewport_rows,
        m1.prompt_rows,
        m1.total_rows,
        m1.transcript_surface,
    );
}

/// A real turn's output, with the reader at the bottom and `follow_ms` on,
/// arms a follow-ease exactly as the synthetic `pager::apply_frame` unit
/// tests predict — proving the wiring from a real session's turn all the way
/// through `pager::apply_frame` actually reaches it.
#[test]
fn a_real_turn_arms_the_follow_ease_when_the_reader_is_at_the_bottom() {
    let Some(mut s) = boot() else { return };
    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.config.animation.enabled = true;
    state.config.animation.follow_ms = 60_000; // long enough to still be mid-flight below
    let area = Rect::new(0, 0, 80, 30);
    flush_boot(&mut s, &mut state);

    assert_eq!(state.transcript_scroll, 0, "premise: reader is at the bottom");
    drive_one_turn(&mut s, &mut state, area, "look");

    assert!(state.scroll_anim.is_some(), "a follow-ease must be armed for output arriving at the bottom");
    assert_ne!(
        state.effective_transcript_scroll(),
        state.transcript_scroll,
        "immediately after the turn the display must still be mid-tween, not already settled"
    );
}

/// `follow_ms = 0` is the documented instant path (matches `enabled = false`
/// and `scroll_ms = 0` elsewhere): the turn above, replayed with it off, must
/// show the pre-SQ-1595 behavior exactly — no animation at all.
#[test]
fn follow_ms_zero_reproduces_the_old_instant_jump() {
    let Some(mut s) = boot() else { return };
    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.config.animation.enabled = true;
    state.config.animation.follow_ms = 0;
    let area = Rect::new(0, 0, 80, 30);
    flush_boot(&mut s, &mut state);

    drive_one_turn(&mut s, &mut state, area, "look");

    assert!(state.scroll_anim.is_none(), "follow_ms = 0 must not arm any animation");
    assert_eq!(state.effective_transcript_scroll(), state.transcript_scroll, "the jump is instant");
}

/// A reader who scrolled into history before asking for another command keeps
/// their place — the follow-ease (like the pager itself) must never drag a
/// history read back to the bottom.
#[test]
fn a_reader_scrolled_into_history_is_not_pulled_back_by_the_follow_ease() {
    let Some(mut s) = boot() else { return };
    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.config.animation.enabled = true;
    state.config.animation.follow_ms = 60_000;
    let area = Rect::new(0, 0, 80, 30);
    flush_boot(&mut s, &mut state);

    // Settle on an opening frame, then scroll up into scrollback.
    let (m0, _) = render(&state, &s, area);
    state.last_transcript_total_rows = m0.total_rows;
    state.transcript_scroll = 3;

    drive_one_turn(&mut s, &mut state, area, "look");

    assert_eq!(state.transcript_scroll, 3, "the reader's chosen offset must not move");
    assert!(state.scroll_anim.is_none(), "no follow-ease while the reader is away from the bottom");
}
