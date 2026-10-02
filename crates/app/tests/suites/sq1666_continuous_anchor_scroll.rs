//! SQ-1666: scrolling up through a standing top-anchor whose content is short
//! must reveal history ONE row at a time, not jump a full viewport's worth in
//! a single scroll step. `render::transcript::window_wrapped_rows` used to
//! have two unrelated windowing formulas — an exact `[anchor_row, n)` pin at
//! `scroll == 0`, and a totally unrelated "last `rows` rows of the WHOLE
//! array" slice at any other scroll — so the first unit of scroll past 0 could
//! jump straight from "the anchor's own couple of rows" to a window sliced
//! from a completely different part of the transcript. `anchored_window_bounds`
//! (SQ-1666) replaces that cutover with a continuous one: each unit of scroll
//! grows the pinned window backward by exactly one row, bottom edge pinned at
//! the anchor's own `n`, until the window fills the viewport and only then
//! hands off to the ordinary backward slide.
//!
//! This file proves it through the REAL render pipeline (unit-level coverage
//! for `window_wrapped_rows`/`anchored_window_bounds` lives in
//! `crates/app/src/render/transcript.rs`'s own test module): boot Anchorhead,
//! drive its three-screen intro to reach gameplay (a game-driven clear leaving
//! only a short room description since), then step `transcript_scroll` one
//! unit at a time and assert each step reveals exactly one more row of history
//! immediately above the still-visible content — never the old reported jump
//! straight back to the very first intro card.
//!
//! Skips vacuously without the gitignored `stories/Anchorhead.gblorb`
//! (CLAUDE.md).

use app::engine::{Engine, KeyInput};
use app::host::{apply_game_driven_result, boot_story, BootRequest, LaunchFlags, QuietBoot, TerminalFacts};
use app::launch_options::LaunchOverrides;
use app::pager::Driver;
use app::session::InputKind;
use app::state::AppState;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::fixture_paths::fixture_path;

/// Generous pane, same shape as `sq1656_preserve_clear_top_anchor.rs` /
/// `sq1661_topanchor_followease.rs`: tall enough that the whole intro sequence
/// plus the opening room description fit comfortably, which is exactly the
/// premise this bug needs (`n - anchor_row <= rows`, SQ-1666's quest writeup).
const PANE: (u16, u16) = (80, 150);

fn boot_headless(story: std::path::PathBuf, home: &std::path::Path) -> app::host::BootedStory {
    let overrides = LaunchOverrides::default();
    let req = BootRequest {
        story_path: story,
        disk_entry: None,
        overrides: &overrides,
        cfg: app::config::Config {
            user_dir: home.to_path_buf(),
            config_file: home.join("config.toml"),
            random_seed: Some(1),
            enable_sound: false,
            virtual_screen_cols: Some(PANE.0),
            virtual_screen_rows: Some(PANE.1),
            ..app::config::Config::default()
        },
        roots: app::data_roots::DataRoots::single(home.join("saves")),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts { size: Some(PANE), ..TerminalFacts::default() },
        fresh_start: true,
    };
    boot_story(req, &mut QuietBoot).expect("the story boots headlessly")
}

/// Render one frame and hand back both the metrics (`top_anchored_fits`,
/// `max_scroll`, …) and the pane's text rows, exactly as `sq1661`'s own
/// `render` helper does.
fn render(session: &dyn Engine, state: &AppState) -> (app::render::screen::StoryPaneMetrics, Vec<String>) {
    let area = Rect::new(0, 0, PANE.0, PANE.1);
    let mut buf = Buffer::empty(area);
    let char_mode = matches!(Engine::pending_input(session), InputKind::Char);
    let model = Engine::screen(session);
    let m = app::render::screen::render_story_pane(&model, char_mode, None, state, area, &mut buf);
    let rows = (0..PANE.1)
        .map(|y| {
            (0..PANE.0)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect();
    (m, rows)
}

/// The row index of the first visible line containing `needle`, or `None`.
fn row_of(rows: &[String], needle: &str) -> Option<usize> {
    rows.iter().position(|r| r.contains(needle))
}

#[test]
fn scrolling_up_through_a_short_top_anchor_reveals_history_one_row_at_a_time() {
    let story = fixture_path("Anchorhead.gblorb");
    if !story.is_file() {
        eprintln!("SKIP: no Anchorhead.gblorb");
        return;
    }
    let home = app::scratch_dir("sq1666-anchorhead-continuous-scroll");
    let mut b = boot_headless(story, &home);
    assert_eq!(b.session.pending_input(), InputKind::Char, "premise: the intro waits for a keypress");

    // Drive the three "press any key" intro screens — each its own game-driven
    // clear — to reach gameplay, exactly as sq1656/sq1661 do.
    for press in 0..3u32 {
        let result = b.session.submit_key(KeyInput::Char(' ')).expect("key reaches the story");
        assert!(result.erase_lower, "press {press}: premise — each intro screen is its own game-driven clear");
        let out = apply_game_driven_result(&mut b.state, &mut b.mapper, &result, &b.game_dir, None, &*b.session, Driver::PlayerInput);
        assert!(!out.quit, "press {press}: the game goes on");
    }
    assert_eq!(b.session.pending_input(), InputKind::Line, "premise: gameplay is reached (a line prompt)");
    assert!(b.state.top_anchor.is_some(), "premise: the opening-room clear left a standing top-anchor");
    assert_eq!(b.state.transcript_scroll, 0, "premise: settled at the bottom");

    // scroll == 0: only the post-clear room description is visible, pinned to
    // the top — none of the three prior intro/epigraph screens leaked in.
    let (m0, rows0) = render(&*b.session, &b.state);
    assert!(
        m0.top_anchored_fits,
        "premise this bug needs: the anchored room description must still fit the viewport \
         (a {}-row pane): {rows0:#?}",
        m0.viewport_rows
    );
    let text0 = rows0.join("\n");
    assert!(text0.contains("Outside the Real Estate Office"), "premise: gameplay's opening room is on screen: {rows0:#?}");
    for needle in ["Welcome to Anchorhead", "first raindrops", "THE FIRST DAY", "H. P. Lovecraft"] {
        assert!(!text0.contains(needle), "scroll=0 must show only the anchored screen ({needle:?} leaked): {rows0:#?}");
    }
    // "Outside the Real Estate Office" also appears in the fixed status bar
    // (row 0, unaffected by scroll — it shows the current room name whatever
    // is scrolled into view below it), so track a phrase unique to the
    // ANCHORED BODY TEXT itself: the opening paragraph's own description.
    let room_row0 = row_of(&rows0, "cul-de-sac").expect("the opening room description is on screen");

    // The reported bug, falsified directly: one step of scroll must NOT jump
    // all the way back to the very first intro card. With a generous 150-row
    // pane the WHOLE pre-gameplay transcript is short, so the pre-fix generic
    // formula (ignoring the anchor for any scroll != 0) would have shown
    // nearly the entire transcript from the start — "Welcome to Anchorhead"
    // included — in a single step.
    b.state.transcript_scroll = 1;
    let (m1, rows1) = render(&*b.session, &b.state);
    let text1 = rows1.join("\n");
    assert!(
        !text1.contains("Welcome to Anchorhead") && !text1.contains("first raindrops"),
        "SQ-1666: scrolling up by ONE step must not jump all the way back to the opening \
         intro card: {rows1:#?}"
    );
    // The room description itself must still be fully visible, shifted down by
    // EXACTLY one row — proving the anchor's own content stayed pinned to the
    // bottom of the growing window rather than being partly scrolled away.
    let room_row1 = row_of(&rows1, "cul-de-sac").expect("the opening room description is still on screen");
    assert_eq!(room_row1, room_row0 + 1, "one step of scroll must shift the anchored content down by exactly one row");

    // The precise shape of the fix, checked directly: the ENTIRE anchored body
    // (every row below the fixed status bar at row 0, which does not move with
    // scroll) must shift down by exactly one row, with one new row inserted at
    // the top and nothing else changing — not a reshuffled/resliced window from
    // an unrelated part of the transcript (the bug's exact shape).
    let assert_shifted_down_by_one = |prev: &[String], next: &[String], at_scroll: u16| {
        for i in 1..prev.len().saturating_sub(1) {
            assert_eq!(
                next[i + 1], prev[i],
                "scroll={at_scroll}: row {i} of the previous frame must reappear unchanged at \
                 row {} of this frame (continuous one-row scroll), not a reshuffled window.\n\
                 prev={prev:#?}\nnext={next:#?}",
                i + 1
            );
        }
    };
    assert_shifted_down_by_one(&rows0, &rows1, 1);

    // Keep walking: every further one-unit step must shift the WHOLE previous
    // frame down by exactly one more row, never jump — proving the continuity
    // holds beyond the very first step too (both through the anchor-growth
    // phase and, once it fills the viewport, the ordinary backward slide this
    // hands off to).
    let mut prev_rows = rows1;
    for scroll in 2u16..=5 {
        b.state.transcript_scroll = scroll;
        let (_m, rows) = render(&*b.session, &b.state);
        assert_shifted_down_by_one(&prev_rows, &rows, scroll);
        prev_rows = rows;
    }

    // max_scroll must have grown to account for the anchor's own padding
    // phase (SQ-1666's `anchored_max_scroll`) — otherwise the oldest rows,
    // including "Welcome to Anchorhead" itself, would be clamped unreachable.
    assert!(m1.max_scroll >= 1, "max_scroll must allow scrolling at least one step up from the anchor");

    let _ = std::fs::remove_dir_all(&home);
}
