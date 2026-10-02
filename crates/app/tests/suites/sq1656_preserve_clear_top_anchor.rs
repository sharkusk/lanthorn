//! SQ-1656: a game-driven screen clear that PRESERVES the prior screen
//! (SQ-1654's `is_screen_reprint` gate said a different screen is taking
//! over, not the same one redrawing) must not top-anchor the render — that
//! pin is for a clear that actually DESTROYED what came before, and SQ-1654
//! made `mark_screen_clear()` run on preserve-shaped clears too, so content
//! that legitimately survives in scrollback was hidden above the fold
//! exactly as if it had been wiped (two user-reported symptoms, same cause:
//! Counterfeit Monkey's opening accessibility Q&A losing the first question,
//! and both CM and Anchorhead briefly scrolling backward then snapping
//! forward on an early action).
//!
//! `AppState::top_anchor` (`crates/app/src/state.rs`) is the render-facing
//! anchor now: `mark_screen_clear` moves it in lockstep with `clear_anchor`
//! (the collapse shape, and every typed-command clear — unchanged), while
//! `mark_screen_clear_preserving_top_anchor` (the preserve shape, reached for
//! by `apply_game_driven_result` in `crates/app/src/host/turn.rs`) leaves it
//! exactly where it was. This file proves that at the RENDERED (viewport)
//! level — `sq1654_scrollback_preserves_screens.rs` already proves the
//! `state.transcript` (data) shape is right; this is the user-facing half
//! SQ-1654 could not see because it never rendered a frame.
//!
//! Skips vacuously without the gitignored `stories/Anchorhead.gblorb` and/or
//! `stories/CounterfeitMonkey-11.gblorb` (CLAUDE.md).

use app::engine::{Engine, KeyInput};
use app::host::{apply_game_driven_result, boot_story, finish_command_turn, BootRequest, LaunchFlags, QuietBoot, TerminalFacts};
use app::launch_options::LaunchOverrides;
use app::pager::Driver;
use app::session::InputKind;
use app::state::AppState;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::fixture_paths::fixture_path;

/// A generously tall pane: both regression cases below depend on the WHOLE
/// scrollback so far fitting comfortably, so a cramped viewport could not
/// tell "hidden by a top-anchor pin" apart from "hidden by ordinary
/// overflow" — the user's own complaint was explicitly that there was "lots
/// of room on the story screen". Glulx's own window tree is sized at boot
/// from `Config::virtual_screen_cols`/`virtual_screen_rows` (`GlulxSession::
/// new_in`, `crates/app/src/host/boot.rs`) — NOT from `TerminalFacts::size`,
/// which only seeds a Z-machine boot — so `boot_headless` below has to set
/// those, not just render into a bigger area.
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

/// The pane's rows, as text, exactly as `render_story_pane` draws them —
/// mirrors `beyondzork_title_repaint.rs`'s own helper of the same shape.
fn pane_rows(session: &dyn Engine, state: &AppState) -> Vec<String> {
    let area = Rect::new(0, 0, PANE.0, PANE.1);
    let mut buf = Buffer::empty(area);
    let model = Engine::screen(session);
    let char_mode = matches!(Engine::pending_input(session), InputKind::Char);
    app::render::screen::render_story_pane(&model, char_mode, None, state, area, &mut buf);
    (0..PANE.1)
        .map(|y| {
            (0..PANE.0)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

/// SQ-1660 (reverting SQ-1656): Anchorhead's intro sequence
/// (`sq1654_scrollback_preserves_screens.rs` drives the identical three
/// presses and confirms per-press content at the data level: press 0 reveals
/// the intro card, press 1 the "THE FIRST DAY" / H. P. Lovecraft epigraph,
/// press 2 the title banner opening onto "Outside the Real Estate Office" —
/// each its own game-driven `erase_lower` clear, essentially no line overlap
/// between them) must top-anchor the RENDER on every one of those clears,
/// exactly like any other clear: each press's rendered viewport shows ONLY
/// that press's new screen by default, with the prior screen(s) scrolled off
/// above the fold rather than stacked together with no visual sign a clear
/// happened (the user's reported symptom this quest restores the fix for).
/// `sq1654_scrollback_preserves_screens.rs`'s own test already proves
/// `state.transcript` keeps every screen's text regardless — this file
/// additionally confirms that un-rendered survival here, and that no scroll
/// animation is armed across the transition (instant, not animated, per the
/// user's explicit request).
#[test]
fn anchorhead_intro_screens_each_top_anchor_and_hide_the_last_by_default() {
    let story = fixture_path("Anchorhead.gblorb");
    if !story.is_file() {
        eprintln!("SKIP: no Anchorhead.gblorb");
        return;
    }
    let home = app::scratch_dir("sq1656-anchorhead-render");
    let mut b = boot_headless(story, &home);
    assert_eq!(b.session.pending_input(), InputKind::Char, "premise: the intro waits for a keypress");

    let press_once = |b: &mut app::host::BootedStory, press: u32| {
        let result = b.session.submit_key(KeyInput::Char(' ')).expect("key reaches the story");
        assert!(result.erase_lower, "press {press}: premise — each intro screen is its own game-driven clear");
        let out = apply_game_driven_result(&mut b.state, &mut b.mapper, &result, &b.game_dir, None, &*b.session, Driver::PlayerInput);
        assert!(!out.quit, "press {press}: the game goes on");
        assert!(
            b.state.scroll_anim.is_none(),
            "press {press}: a game-driven clear is instant — no scroll animation armed"
        );
    };

    press_once(&mut b, 0);
    let rows0 = pane_rows(&*b.session, &b.state);
    let text0 = rows0.join("\n");
    assert!(
        text0.contains("Welcome to Anchorhead") || text0.contains("first raindrops"),
        "press 0: the intro card must be on screen by default: {rows0:#?}"
    );

    press_once(&mut b, 1);
    let rows1 = pane_rows(&*b.session, &b.state);
    let text1 = rows1.join("\n");
    assert!(
        text1.contains("THE FIRST DAY") || text1.contains("H. P. Lovecraft"),
        "press 1: the epigraph quote splash must be on screen by default: {rows1:#?}"
    );
    assert!(
        !text1.contains("Welcome to Anchorhead") && !text1.contains("first raindrops"),
        "press 1: the intro card must NOT be part of the default render any more \
         (it is only reachable by scrolling up): {rows1:#?}"
    );

    press_once(&mut b, 2);
    assert_eq!(b.session.pending_input(), InputKind::Line, "premise: gameplay is reached (a line prompt)");
    let rows2 = pane_rows(&*b.session, &b.state);
    let text2 = rows2.join("\n");
    assert!(
        text2.contains("Outside the Real Estate Office"),
        "press 2: gameplay's own opening room must be on screen by default: {rows2:#?}"
    );
    assert!(
        !text2.contains("Welcome to Anchorhead")
            && !text2.contains("first raindrops")
            && !text2.contains("THE FIRST DAY")
            && !text2.contains("H. P. Lovecraft"),
        "press 2: neither the intro card nor the epigraph must be part of the default \
         render any more: {rows2:#?}"
    );

    // Non-vacuity: every one of those three clears moved BOTH anchors in
    // lockstep (there is no preserve/collapse split in the render-facing
    // anchor any more) — top_anchor must equal clear_anchor, not be left
    // behind the way SQ-1656 left it.
    assert!(b.state.clear_anchor.is_some(), "premise: clear_anchor advances on every clear");
    assert_eq!(
        b.state.top_anchor, b.state.clear_anchor,
        "every clear — collapse or preserve-shaped alike — must move top_anchor in lockstep \
         with clear_anchor, exactly as before SQ-1656 ever existed"
    );

    // SQ-1654's guarantee, unaffected by this quest: every prior screen's text
    // still survives in full in `state.transcript`, reachable by scrolling up,
    // even though none of it is part of the default render above.
    let transcript_text = b.state.transcript.join("\n");
    assert!(
        transcript_text.contains("Welcome to Anchorhead") && transcript_text.contains("first raindrops"),
        "the intro card must still be in state.transcript: {transcript_text}"
    );
    assert!(
        transcript_text.contains("THE FIRST DAY") && transcript_text.contains("H. P. Lovecraft"),
        "the epigraph quote splash must still be in state.transcript: {transcript_text}"
    );
    assert!(
        transcript_text.contains("Outside the Real Estate Office"),
        "gameplay's own opening room must still be in state.transcript: {transcript_text}"
    );

    let _ = std::fs::remove_dir_all(&home);
}

/// The non-regression companion: Counterfeit Monkey's real arrow-navigated
/// HINT menu (`sq1654_scrollback_preserves_screens.rs`'s
/// `counterfeit_monkeys_real_hint_menu_still_collapses_on_arrow_navigation`
/// proves the collapse at the data level) must still top-anchor at the
/// RENDERED level: the menu redraw is the ORIGINAL SQ-0407 case, and it is a
/// real `is_screen_reprint` COLLAPSE, not a preserve, so `top_anchor` must
/// still move and the pre-menu accessibility dialogue must still be hidden
/// above the fold, exactly as before this quest.
#[test]
fn counterfeit_monkeys_real_hint_menu_still_top_anchors_in_the_render() {
    let story = fixture_path("CounterfeitMonkey-11.gblorb");
    if !story.is_file() {
        eprintln!("SKIP: no CounterfeitMonkey-11.gblorb");
        return;
    }
    let home = app::scratch_dir("sq1656-cm-menu-render");
    let mut b = boot_headless(story, &home);

    let mut tidy = 0u32;
    for cmd in ["yes", "andra", "", "tutorial off", "hint"] {
        let result = b.session.submit(cmd);
        let out = finish_command_turn(
            cmd, true, result, &mut b.state, &mut b.mapper, &mut *b.session,
            &b.game_dir, &b.ifid, &b.arc_file, None, &mut tidy,
        );
        assert!(!out.quit, "{cmd}: the game goes on");
    }

    let down_result = b.session.submit_key(KeyInput::Down).expect("the down arrow reaches the menu");
    assert!(down_result.erase_lower, "premise: the menu redraw clears the primary window on every arrow press");
    let out = apply_game_driven_result(&mut b.state, &mut b.mapper, &down_result, &b.game_dir, None, &*b.session, Driver::PlayerInput);
    assert!(!out.quit);

    // Non-vacuity: this really did take the COLLAPSE branch — `top_anchor`
    // must equal `clear_anchor` (both moved together), not have been left
    // behind the way a preserve leaves it.
    assert!(b.state.clear_anchor.is_some(), "premise: the arrow press cleared");
    assert_eq!(
        b.state.top_anchor, b.state.clear_anchor,
        "premise: a menu-redraw collapse must move top_anchor in lockstep with clear_anchor, \
         exactly as every clear did before this quest"
    );

    let rows = pane_rows(&*b.session, &b.state);
    let text = rows.join("\n");
    assert!(
        text.contains("Instructions for Play"),
        "the new cursor position must be on screen: {rows:#?}"
    );
    assert!(
        !text.contains("andra"),
        "the pre-menu accessibility dialogue must stay hidden above the fold — a menu \
         collapse still top-anchors exactly as before this quest: {rows:#?}"
    );

    let _ = std::fs::remove_dir_all(&home);
}
