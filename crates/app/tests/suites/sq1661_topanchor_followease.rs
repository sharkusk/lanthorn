//! SQ-1661: a game-driven screen clear top-anchors the view (SQ-1656/1660 —
//! `AppState::top_anchor`), and an ORDINARY command typed afterward — no clear
//! of its own, output that still fits one screen — must not briefly reach back
//! into the hidden pre-anchor scrollback while it settles.
//!
//! Root cause: `pager::apply_frame`'s follow-ease (SQ-1595) measures its
//! FROM/TO values against the FULL transcript's wrapped-row count, a
//! coordinate space `render::transcript::window_wrapped_rows` abandons the
//! moment a top-anchored view is showing its content in full (`scroll == 0`
//! is reinterpreted as "N rows back from the bottom of the WHOLE transcript"
//! rather than "the anchor"). Arming an ease there eases `transcript_scroll`
//! from a small non-zero value down to 0; every intermediate frame renders
//! through the WRONG branch of `window_wrapped_rows` and can show whatever
//! scrollback sits above the anchor — Anchorhead's opening quote, in the
//! reported case — before snapping back to the correct top-anchored view at
//! `scroll == 0`. `render::transcript::TranscriptRender::top_anchored_fits`
//! (and the raster path's equivalent on `RasterMetrics`) now tells
//! `apply_frame` this is the case, so it skips arming the ease entirely: the
//! new output just appends below the still-pinned anchored content, and
//! nothing needs to move.
//!
//! Skips vacuously without the gitignored `stories/Anchorhead.gblorb`
//! (CLAUDE.md).

use app::engine::{Engine, KeyInput};
use app::host::{apply_game_driven_result, boot_story, finish_command_turn, BootRequest, LaunchFlags, QuietBoot, TerminalFacts};
use app::launch_options::LaunchOverrides;
use app::pager::Driver;
use app::session::InputKind;
use app::state::AppState;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::fixture_paths::fixture_path;

/// A generously tall pane, exactly `sq1656_preserve_clear_top_anchor.rs`'s
/// `PANE`: both the intro sequence and the one ordinary command after it need
/// to fit comfortably so "hidden by the top-anchor" and "hidden by ordinary
/// overflow" cannot be confused for one another.
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

/// Render one frame of the story pane — mirrors
/// `sq1656_preserve_clear_top_anchor.rs`'s `pane_rows`, but also hands back the
/// `StoryPaneMetrics` the real run loop (`main.rs`) feeds straight into
/// `pager::apply_frame` every frame.
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

/// Render this frame and run the exact post-frame bookkeeping the real run
/// loop runs (`main.rs`'s `app::pager::apply_frame` call, right after
/// `draw_frame`) — the two-phase arm (by `apply_game_driven_result`/
/// `finish_command_turn`) → activate (here) cycle `pager.rs`'s own doc
/// describes. Returns the rendered pane text.
fn settle_frame(b: &mut app::host::BootedStory) -> Vec<String> {
    let (m, rows) = render(&*b.session, &b.state);
    app::pager::apply_frame(
        &mut b.state,
        m.max_scroll,
        m.viewport_rows,
        m.prompt_rows,
        m.total_rows,
        m.transcript_surface,
        m.top_anchored_fits,
    );
    rows
}

const INTRO_TEXT: [&str; 4] = ["Welcome to Anchorhead", "first raindrops", "THE FIRST DAY", "H. P. Lovecraft"];

#[test]
fn an_ordinary_command_after_anchorheads_intro_never_shows_the_hidden_quote() {
    let story = fixture_path("Anchorhead.gblorb");
    if !story.is_file() {
        eprintln!("SKIP: no Anchorhead.gblorb");
        return;
    }
    let home = app::scratch_dir("sq1661-anchorhead-followease");
    let mut b = boot_headless(story, &home);
    assert_eq!(b.session.pending_input(), InputKind::Char, "premise: the intro waits for a keypress");

    // Drive the three "press any key" intro screens exactly as
    // `sq1656_preserve_clear_top_anchor.rs` does, settling a real frame after
    // each press — the real run loop always draws and calls `apply_frame`
    // once per turn, which is what seeds `state.last_transcript_total_rows`
    // for the pager baseline the NEXT turn arms from (without this, the
    // critical turn below would arm from a stale/zero baseline and the bug
    // this test is for would not reproduce).
    for press in 0..3u32 {
        let result = b.session.submit_key(KeyInput::Char(' ')).expect("key reaches the story");
        assert!(result.erase_lower, "press {press}: premise — each intro screen is its own game-driven clear");
        let out = apply_game_driven_result(&mut b.state, &mut b.mapper, &result, &b.game_dir, None, &*b.session, Driver::PlayerInput);
        assert!(!out.quit, "press {press}: the game goes on");
        settle_frame(&mut b);
        assert!(
            b.state.scroll_anim.is_none(),
            "press {press}: a game-driven clear is instant — no scroll animation armed (SQ-1607)"
        );
    }
    assert_eq!(b.session.pending_input(), InputKind::Line, "premise: gameplay is reached (a line prompt)");
    assert_eq!(b.state.transcript_scroll, 0, "premise: settled at the bottom");
    assert!(b.state.top_anchor.is_some(), "premise: the opening-room clear left a standing top-anchor");

    // The critical turn: an ORDINARY command, no clear of its own, output that
    // fits comfortably in the 150-row pane — exactly the reported repro
    // ("after the intro screen, a new command jumps the scroll back to the
    // intro quote, then scrolls quickly to complete the command").
    let cmd = "look";
    let result = b.session.submit(cmd);
    assert!(!result.erase_lower, "premise: an ordinary room look does not clear the screen");
    let mut bg_tidy = 0u32;
    let out = finish_command_turn(
        cmd, true, result, &mut b.state, &mut b.mapper, &mut *b.session,
        &b.game_dir, &b.ifid, &b.arc_file, None, &mut bg_tidy,
    );
    assert!(!out.quit, "the game goes on");

    // SQ-1661's fix direction: no follow-ease needed at all while the
    // top-anchored content still fits — the simplest possible proof the bug
    // cannot occur, independent of how many frames get drawn while it settles.
    let rows = settle_frame(&mut b);
    assert!(
        b.state.scroll_anim.is_none(),
        "SQ-1661: an ordinary command whose output still fits under a standing top-anchor \
         must not arm a follow-ease at all"
    );
    assert_eq!(b.state.transcript_scroll, 0, "nothing needed to move — already at the correct position");

    // Non-vacuity: the settled frame is real gameplay, not a blank/degenerate
    // pane, and none of the hidden intro text leaked into it.
    let text = rows.join("\n");
    assert!(
        text.contains("Outside the Real Estate Office") || text.to_lowercase().contains("look"),
        "the settled frame must show real post-intro gameplay content: {rows:#?}"
    );
    for needle in INTRO_TEXT {
        assert!(
            !text.contains(needle),
            "the settled frame must not show hidden pre-anchor intro content ({needle:?} leaked): {rows:#?}"
        );
    }

    let _ = std::fs::remove_dir_all(&home);
}
