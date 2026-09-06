//! SQ-0699: `/dump-windows` for non-v6 Z-machine games (v1–v5, v7, v8) used to
//! print exactly one line — `Window layout: Grid {cols}x{rows} over Buffer
//! (Z-machine v{N})` — which tells a reader nothing about what's actually on
//! screen. In particular it collapsed the split height (`upper_window_rows`)
//! and the painted grid height (`upper.rows`) into a single number, even
//! though c54c9e0f (SQ-0696) made a `split_window` shrink keep painted rows
//! (so Inform box quotes survive it) — the two can now legitimately differ,
//! and the old dump had no way to show that.
//!
//! `anchor.z8` exercises both states: at boot its upper window holds a painted
//! quote box behind a 1-row split, and once play begins the split collapses to
//! an ordinary 1-row status line with a 1-row grid to match.
//!
//! **The boot grid is 10 rows, not 11** (SQ-1371). This case pinned 11 from
//! SQ-0699 until SQ-1355, and 11 was the ALLOCATION, not the paint. Traced with
//! `trace_screen` on, Inform's `Box__Routine` veneer does exactly this at boot:
//!
//! ```text
//! @split_window(11)   @set_window(upper)   @set_text_style(reverse)
//! @set_cursor(row=4,  col=16)                     ← top border, spaces
//! @set_cursor(row=5..9, col=16) + (row=5..9, col=18)  ← pad, then the quote
//! @set_cursor(row=10, col=16)                     ← bottom border, spaces
//! @set_text_style(roman)   @set_window(lower)   @split_window(1)
//! ```
//!
//! Row 4 and row 10 are spaces carrying the reverse-video bit, so they are
//! paint and `last_painted_row` counts them. Row **11** is never addressed at
//! all: no cursor is ever set there, nothing is printed there, and a probe at
//! 8b296ff9 (the commit before SQ-1355) confirms it held `style 0`, default
//! colours, eighty spaces. It existed solely because `@split_window(11)` had
//! allocated it, and a real interpreter — which has no per-window grid — shows
//! nothing there either. So 10 is the box's true depth and the old 11 was the
//! artefact SQ-1355 removed.
//!
//! The story is gitignored, so this skips vacuously when absent.


use app::engine::Engine;
use app::session::{GameSession, InputKind};

use crate::fixture_paths::fixture_path;


fn boot_anchor() -> Option<GameSession> {
    let path = fixture_path("anchor.z8");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    Some(
        GameSession::new_with_trace(bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)))
            .expect("anchor.z8 should load and boot without a ZError"),
    )
}

#[test]
fn dump_windows_reports_split_vs_painted_divergence_at_boot() {
    let Some(session) = boot_anchor() else { return };

    let lines = session.window_dump();
    let dump = lines.join("\n");

    assert!(dump.contains("Z-machine v8"), "the version stays reported, as before: {dump}");
    assert!(
        dump.contains("split: 1 row(s) requested"),
        "the boot quote box is behind a 1-row split: {dump}"
    );
    assert!(
        dump.contains("grid: 10 row(s) painted"),
        "the box's reverse-video border reaches row 10, and the shrink must not truncate it: {dump}"
    );
    assert!(
        !dump.contains("grid: 11 row(s) painted"),
        "row 11 was allocated by `@split_window(11)` and never painted, so the shrink releases it (SQ-1371): {dump}"
    );
    assert!(dump.contains("<- diverge"), "split and painted height disagree at boot, and the dump must flag it: {dump}");
    assert!(dump.contains("H.P. Lovecraft"), "the painted rows are printed as quoted text: {dump}");
}

#[test]
fn dump_windows_shows_collapsed_state_during_play() {
    let Some(mut session) = boot_anchor() else { return };

    // Clear the two startup quote screens (each waits for a keypress) so the
    // upper window settles to its ordinary one-row status line.
    for _ in 0..2 {
        if matches!(session.pending_input(), InputKind::Char) {
            let _ = session.submit_char(13);
        }
    }
    let _ = session.submit("look");

    let lines = session.window_dump();
    let dump = lines.join("\n");

    assert!(
        dump.contains("split: 1 row(s) requested  ·  grid: 1 row(s) painted"),
        "once play begins the split and the painted grid both collapse to 1 row: {dump}"
    );
    assert!(!dump.contains("<- diverge"), "the two numbers agree during ordinary play, so no flag: {dump}");
}
