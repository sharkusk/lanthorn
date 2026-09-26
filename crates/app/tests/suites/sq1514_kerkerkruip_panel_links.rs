//! SQ-1514: Kerkerkruip's clickable UI — its in-game menus and the "[detailed
//! status report]" link in its statistics panel — did nothing at all under the
//! mouse, while typing the equivalent command worked.
//!
//! # What the game actually does
//!
//! Traced off the real archive (`stories/Kerkerkruip.gblorb`, Kerkerkruip 9.0.1,
//! IFID AC0DAF65-F40F-4A41-A4E4-50414F836E14) with its shipped `Kerkerkruip.ini`
//! beside it — which is the opt-in to this whole presentation, see
//! `glulx_garglk_style_sentinel.rs`. Given a wide enough pane, ordinary play runs
//! a **twenty-one-window** layout: a story buffer in the middle, a status grid
//! above it, side panels left (Inventory) and right (Statistics, Powers), and a
//! graphics window for every rule between them. Its screen trace says, in order:
//!
//! ```text
//! glk_window_open(4, 33, 25, 3, 240) -> win 7    // the Statistics panel
//! glk_set_hyperlink(1)                            // "[detailed status report]"
//! glk_set_hyperlink(0)
//! glk_request_hyperlink_event(7)
//! ```
//!
//! …and eight windows hold a standing hyperlink request at the parser prompt, of
//! which **exactly one is the primary buffer**. Typing HELP replaces the panels
//! with a menu whose nine choices carry links 49..56 and 81. So the reported
//! "clickable menus do nothing" and the reported dead stats link are one bug in
//! two places, not two bugs.
//!
//! # The drop point
//!
//! `render::screen::render_node`'s `WinNode::Buffer` arm forks on `b.primary`:
//! the primary window goes to `render_transcript`, which returns its link cells
//! in `StoryPaneMetrics::links`, and **every other buffer window** went to
//! `render_inline_buffer`, which drew the styled runs and recorded nothing. So
//! the frame's cell→link map came back EMPTY on a screen with nine live links on
//! it, and `main.rs`'s hyperlink arm — which looks the clicked cell up in that
//! map before anything else — never fired. Nothing else was wrong: gvm stamped
//! the links, `AppGlk` carried them onto the runs, and the window's drawn rect
//! was recorded in `win_rects`, so `glk_hyperlink_window` would have resolved the
//! click the instant it was asked. It was never asked.
//!
//! Fixed by `render::transcript::record_run_links`, the transcript's own
//! char-offset→display-column recording lifted into one function and called from
//! `render_inline_buffer` as well — the same "one recorder, every route" shape
//! `record_band_links` took for SQ-1503's pictures, which is the identical defect
//! one level out.
//!
//! The GRAPHICAL main menu ("New Game / Help / Options / Quit") is not affected
//! and never was: it is a real Glk graphics window with
//! `glk_request_mouse_event`, so it travels `glk_mouse_target` →
//! `deliver_mouse`. Checked, not assumed — the third case drives it.
//!
//! # Why nothing here names a window id, and why each case sets a turn budget
//!
//! The first version of this suite went green locally and red on two of three CI
//! runners, and both reasons are worth keeping written down, because both look
//! like flakiness and neither is.
//!
//! **1. A window id is not a fact about the game.** Glk ids are handed out in
//! allocation order, so the SAME help menu is window 47 when it replaces the
//! in-game panels and window **15** when it is opened from the graphical main
//! menu — and the panel layout itself only exists above a pane-width threshold
//! (no side panels at 100x30, all of them at 110x34). A harness that hardcodes an
//! id is asserting about the route it happened to take. So every window here is
//! found by its CONTENT ([`find_linked_panel`]), and each case asserts the click
//! resolved to the window it actually found.
//!
//! **2. The dungeon deal is one enormous turn, and it sits on the app's
//! runaway-game watchdog.** Kerkerkruip generates its whole dungeon inside a
//! single `glk_select`-to-`glk_select` span, and the cost scales with the pane it
//! is laying panels out for. Measured on a quiet machine, debug build, worst
//! single turn:
//!
//! | pane | side panels | worst turn of the deal |
//! |---|---|---|
//! | 80x24 | no | 1.89s |
//! | 100x30 | no | 1.91s |
//! | 110x34 | yes | 10.53s |
//! | 120x40 | yes | 10.51s |
//!
//! against `GlulxSession`'s 10s default. `drive` samples the clock only every
//! million steps, so at the panelled sizes it tips over *intermittently* — and
//! when it does, the turn is aborted as a fault and the game is left half dealt:
//! no menu window, a stats panel that answers a click by doing nothing. That is
//! exactly the CI failure, and it reproduces locally the moment two of these
//! tests share a process (`cargo test`, which is what CI runs; nextest's
//! one-process-per-test hides it, which is why the local gate was green).
//!
//! So each real-game case calls [`GlulxSession::set_turn_budget`] instead of
//! racing the default, and [`assert_turn_survived`] fails loudly naming the
//! watchdog if a turn is ever truncated anyway — turning a silent half-dealt game
//! into a one-line diagnosis. Every drive is a bounded poll on the condition the
//! case needs ([`settle`]), never a tick count.
//!
//! # Fixture
//!
//! `stories/Kerkerkruip.gblorb` plus `Kerkerkruip.ini` beside it — both also in
//! `scripts/fixtures.manifest`, so CI fetches them and these cases really run
//! there. Skips vacuously when absent. Needs no save, no VFS sidecar and no
//! `game_dir`: a fresh install reaching its own first turn.

use std::path::{Path, PathBuf};
use std::time::Duration;

use app::engine::{Engine, WinNode};
use app::glk_backend::GlkStylePairs;
use app::glulx_session::GlulxSession;
use app::session::InputKind;
use app::state::{AppState, Focus};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::fixture_paths::fixture_path;

const STORY: &str = "Kerkerkruip.gblorb";

/// Bound on any poll below. Generous: the real counts are ~110 intro ticks and
/// ~20 more for the deal, and the point of a cap is only that a wedged game
/// fails instead of hanging.
const POLL_CAP: usize = 4000;

/// The budget each real-game case gives one turn. Twelve times the worst deal
/// measured above, so even a heavily loaded runner finishes the turn rather than
/// having it aborted underneath the assertions. See the module docs.
const TURN_BUDGET: Duration = Duration::from_secs(120);

/// The story, and the ini that turns its clickable presentation on, or `None`.
fn story_path() -> Option<PathBuf> {
    let p = fixture_path(STORY);
    if !p.is_file() {
        eprintln!("SKIP: fixture missing at {}", p.display());
        return None;
    }
    if !p.with_file_name("Kerkerkruip.ini").is_file() {
        eprintln!("SKIP: this suite needs the shipped Kerkerkruip.ini beside the story");
        return None;
    }
    Some(p)
}

/// The theme colours the app pushes into the Glk backend for the story at `path`
/// — the real chain `startup.rs` runs: a scheme, the garglk.ini beside the story
/// overlaid, then the per-Glk-style pairs. Same helper as
/// `glulx_garglk_style_sentinel`; without it the game takes its plain
/// screen-reader branch and never builds this UI at all.
fn theme_pairs_for(path: &Path) -> GlkStylePairs {
    let mut cs = app::colors::ColorScheme::default();
    if let Some(ov) = app::garglk_ini::discover(path) {
        ov.apply(&mut cs);
    }
    app::glk_backend::theme_style_colours(&cs)
}

fn boot(path: &Path, cols: u16, rows: u16) -> GlulxSession {
    let bytes = std::fs::read(path).expect("read the story");
    let blorb = blorb::Blorb::parse(bytes).expect("Kerkerkruip is a Blorb");
    let image = blorb.executable().expect("Glulx exec chunk").1.to_vec();
    let mut sess = GlulxSession::new_in(
        PathBuf::new(), // no persistent store: a fresh, never-played install
        image,
        cols as u32,
        rows as u32,
        true,  // acceleration
        true,  // graphics (the panels' rules are graphics windows)
        false, // sound
        false, // borderless
        (8.0, 16.0),
        Some(blorb),
        &[], // no VFS sidecar
        theme_pairs_for(path),
        false,
        None,
    )
    .expect("Kerkerkruip boots");
    // Before any drive that matters: this game's turns are legitimately long.
    sess.set_turn_budget(TURN_BUDGET);
    sess
}

/// Advance the game — its intro and its dungeon deal are timer-driven — until
/// `done` holds, or until there is nothing left a tick could advance, or until
/// the cap. Returns whether `done` ended up true.
///
/// The repo's existing shape for this (`advent_toolbar`'s `settle`): poll a
/// condition with a bounded number of `deliver_timer` calls. Never a fixed count
/// — how many ticks the intro takes is the game's business and it varies with the
/// deal (91, 98 and 110 all observed).
///
/// "Nothing left to advance" is a TIMER question, not an input question, and the
/// difference is load-bearing here: Kerkerkruip's title card asks for a keypress
/// while its animation is still running, so a poll that stopped at the first
/// input request would stop one frame short of the main menu it is still
/// painting. So this keeps ticking while a timer is armed, whatever the game is
/// also asking for, and gives up only when no timer is armed and no event is
/// wanted.
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

/// Fail naming the watchdog if a turn was aborted. Without this the symptom is a
/// game that silently did nothing, which is indistinguishable from the bug this
/// suite is about — see the module docs.
fn assert_turn_survived(sess: &GlulxSession, what: &str) {
    assert!(
        !Engine::has_quit(sess),
        "the game stopped during {what}: either it quit or `GlulxSession`'s runaway-turn \
         watchdog aborted the turn (see set_turn_budget — this suite raises the budget to \
         {TURN_BUDGET:?} precisely so that cannot happen). Nothing below this point is \
         measuring what it claims to."
    );
}

/// One non-primary buffer window with linked text: its Glk id, its lines, and
/// every distinct link value on it.
struct Panel {
    win: u32,
    lines: Vec<String>,
    links: Vec<u32>,
}

/// The non-primary buffer window whose text contains `needle`, if any — found by
/// CONTENT, never by id. See the module docs for why that matters.
fn find_linked_panel(sess: &GlulxSession, needle: &str) -> Option<Panel> {
    fn walk(node: &WinNode, out: &mut Vec<Panel>) {
        match node {
            WinNode::Buffer(b) if !b.primary => {
                let mut links: Vec<u32> =
                    b.runs.iter().flat_map(|rl| rl.iter().filter(|r| r.link != 0).map(|r| r.link)).collect();
                links.dedup();
                out.push(Panel { win: b.win, lines: b.lines.clone(), links });
            }
            WinNode::Pair { first, second, .. } => {
                walk(first, out);
                walk(second, out);
            }
            _ => {}
        }
    }
    let model = Engine::screen(sess);
    let mut out = Vec::new();
    walk(&model.root, &mut out);
    out.into_iter().find(|p| p.lines.iter().any(|l| l.contains(needle)))
}

/// The link value on the line of `panel` that contains `needle`.
fn link_on_line(sess: &GlulxSession, panel_needle: &str, line_needle: &str) -> Option<u32> {
    fn walk<'a>(node: &'a WinNode, out: &mut Vec<&'a app::engine::BufferWindow>) {
        match node {
            WinNode::Buffer(b) if !b.primary => out.push(b),
            WinNode::Pair { first, second, .. } => {
                walk(first, out);
                walk(second, out);
            }
            _ => {}
        }
    }
    let model = Engine::screen(sess);
    let mut bufs = Vec::new();
    walk(&model.root, &mut bufs);
    let b = bufs.into_iter().find(|b| b.lines.iter().any(|l| l.contains(panel_needle)))?;
    b.lines.iter().enumerate().find(|(_, l)| l.contains(line_needle)).and_then(|(i, _)| {
        b.runs.get(i).and_then(|rl| rl.iter().find(|r| r.link != 0)).map(|r| r.link)
    })
}

/// A headless `AppState` for the render — the game's own colours honoured, which
/// is the shipped default.
fn render_state() -> AppState {
    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.focus = Focus::Game;
    state
}

fn render(sess: &GlulxSession, state: &AppState, cols: u16, rows: u16) -> app::render::screen::StoryPaneMetrics {
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    app::render::screen::render_story_pane(&Engine::screen(sess), false, None, state, area, &mut buf)
}

/// Run the two calls `main.rs`'s hyperlink arm makes, for the first cell in the
/// frame's map carrying `link`: resolve the owning window, then deliver the
/// event. Returns the window the click resolved to. Panics — with the empty map
/// that IS the reported symptom — if the click finds no recorded link.
fn click_link(sess: &mut GlulxSession, link: u32, cols: u16, rows: u16) -> u32 {
    let state = render_state();
    let m = render(sess, &state, cols, rows);
    let &((col, row), v) = m.links.iter().find(|&&(_, v)| v == link).unwrap_or_else(|| {
        panic!("a click on the drawn link {link} must find it in the frame's cell→link map; got {:?}", m.links)
    });
    let windows = sess.hyperlink_windows();
    let win = app::glulx_session::glk_hyperlink_window(false, col, row, (0, 0, cols, rows), &windows, &m.win_rects)
        .unwrap_or_else(|| {
            panic!(
                "the click at ({col},{row}) must resolve to a hyperlink-watching window; \
                 windows={windows:?} win_rects={:?}",
                m.win_rects
            )
        });
    let _ = sess.deliver_hyperlink(win, v);
    win
}

/// Boot and reach the parser prompt of a real dungeon at `cols`x`rows`: poll
/// through the timer-driven intro, press the spacebar its title card asks for,
/// poll through the deal.
fn into_gameplay(cols: u16, rows: u16) -> Option<GlulxSession> {
    let mut sess = boot(&story_path()?, cols, rows);
    settle(&mut sess, |s| Engine::pending_input(s) != InputKind::Event);
    assert_turn_survived(&sess, "the opening animation");
    let _ = Engine::submit_key(&mut sess, app::engine::KeyInput::Char(' '));
    settle(&mut sess, |s| Engine::pending_input(s) == InputKind::Line);
    assert_turn_survived(&sess, "the dungeon deal");
    assert_eq!(
        Engine::pending_input(&sess),
        InputKind::Line,
        "the intro should have reached the parser prompt; window layout was:\n{}",
        Engine::window_dump(&sess).join("\n")
    );
    Some(sess)
}

// ── The reported bug, at the two places it was reported ──────────────────────

/// An in-game menu's choices, clicked through the app's real delivery path: the
/// menu must navigate exactly as typing the choice's number does.
///
/// Driven at **100x30**, where Kerkerkruip runs its plain (no side panels)
/// layout: the same linked non-primary buffer this quest is about, reached
/// through a 1.9s deal instead of a 10.5s one, and arriving as a DIFFERENT window
/// id than the panelled route gives it — which is the coverage that makes
/// hardcoding one indefensible. The panelled layout is covered by the case below.
///
/// Reverted (drop `record_run_links` from `render_inline_buffer`), this fails in
/// `click_link` with `got []` — an empty cell→link map on a screen full of live
/// links, which is the report exactly.
#[test]
fn an_in_game_menus_choices_answer_a_click() {
    const COLS: u16 = 100;
    const ROWS: u16 = 30;
    let Some(mut sess) = into_gameplay(COLS, ROWS) else { return };

    let _ = Engine::submit(&mut sess, "help");
    settle(&mut sess, |s| find_linked_panel(s, "Playing Kerkerkruip").is_some());
    assert_turn_survived(&sess, "the HELP menu");

    // Non-vacuity, read off the game rather than assumed: a non-primary buffer
    // holding the top-level menu, with one distinct link per choice.
    let menu = find_linked_panel(&sess, "Playing Kerkerkruip")
        .unwrap_or_else(|| panic!("HELP must open its menu; layout:\n{}", Engine::window_dump(&sess).join("\n")));
    assert!(
        menu.links.len() >= 5,
        "a real menu: several distinct link values, one per choice; got {:?} on {:?}",
        menu.links,
        menu.lines
    );
    let armed = sess.hyperlink_windows();
    assert!(armed.contains(&menu.win), "the menu window must be hyperlink-armed; armed: {armed:?}");
    let choice = link_on_line(&sess, "Playing Kerkerkruip", "Players new to Interactive Fiction")
        .unwrap_or_else(|| panic!("the top-level menu lists its first chapter as a link; menu: {:?}", menu.lines));

    let win = click_link(&mut sess, choice, COLS, ROWS);
    assert_eq!(win, menu.win, "the menu window owns the click");
    settle(&mut sess, |s| find_linked_panel(s, "Interactive Fiction basics").is_some());
    assert_turn_survived(&sess, "the menu click");

    // The menu navigated: the window now holds the CHAPTER's items, not the
    // table of contents it was showing.
    let after = find_linked_panel(&sess, "Interactive Fiction basics").unwrap_or_else(|| {
        panic!(
            "clicking \"Players new to Interactive Fiction\" must open that chapter the way \
             typing its number does; the menu now reads {:?}",
            find_linked_panel(&sess, "Go back").map(|p| p.lines)
        )
    });
    assert!(
        !after.lines.iter().any(|l| l.contains("Credits, Copyright & Afterword")),
        "…and left the table of contents behind; the menu now reads {:?}",
        after.lines
    );
}

/// The statistics panel's "[detailed status report]" link — the user's own
/// words. Clicked through the app's real delivery path, the game must switch the
/// panel to its detailed view.
///
/// Driven at **120x40**, the pane width at which Kerkerkruip opens its side
/// panels at all, so this is the only case that exercises the panelled layout —
/// and the reason it needs the raised turn budget most (a 10.5s deal against a
/// 10s default).
///
/// Reverted, this fails in `click_link` with `got []`.
#[test]
fn the_statistics_panels_detailed_status_report_link_answers_a_click() {
    const COLS: u16 = 120;
    const ROWS: u16 = 40;
    let Some(mut sess) = into_gameplay(COLS, ROWS) else { return };

    // Non-vacuity, and the mechanism read off the game: a NON-PRIMARY buffer
    // holding the reported link, hyperlink-armed, on a screen where most of the
    // armed windows are not the primary one — which is the shape of the bug.
    let panel = find_linked_panel(&sess, "detailed status report").unwrap_or_else(|| {
        panic!(
            "this pane must give Kerkerkruip its side panels; layout:\n{}",
            Engine::window_dump(&sess).join("\n")
        )
    });
    let armed = sess.hyperlink_windows();
    assert!(armed.contains(&panel.win), "the panel must hold a standing hyperlink request; armed: {armed:?}");
    assert!(
        armed.iter().filter(|&&w| !is_primary(&sess, w)).count() >= 4,
        "the shape of the bug: most of this game's hyperlink-watching windows are NOT the \
         primary buffer, so recording links only there covers almost none of its UI; armed: {armed:?}"
    );
    let before = panel.lines.clone();
    let link = link_on_line(&sess, "detailed status report", "detailed status report")
        .unwrap_or_else(|| panic!("the panel's line carries the reported link; panel: {before:?}"));

    let win = click_link(&mut sess, link, COLS, ROWS);
    assert_eq!(win, panel.win, "the statistics panel owns the click");
    settle(&mut sess, |s| {
        find_linked_panel(s, "Health").is_some_and(|p| p.lines != before)
    });
    assert_turn_survived(&sess, "the panel click");

    // …and the game ACTS on it. Asserted as the CHANGE the click caused — the
    // panel was the race plus the link, and the detailed view replaces that with
    // its own way back under a fresh link value — rather than as wording, which
    // would pin a release's phrasing instead of its behaviour.
    let after = find_linked_panel(&sess, "Health")
        .unwrap_or_else(|| panic!("the statistics panel is still on screen after the click"));
    assert_ne!(
        after.lines, before,
        "clicking [detailed status report] must change the panel; it still reads {before:?}"
    );
    assert!(
        after.links.iter().any(|&l| l != 0 && l != link),
        "the view the click opened arms its own link, so the panel is still clickable; \
         got {:?} on {:?}",
        after.links,
        after.lines
    );
}

/// Whether Glk window `win` is the session's primary buffer.
fn is_primary(sess: &GlulxSession, win: u32) -> bool {
    fn walk(node: &WinNode, win: u32) -> bool {
        match node {
            WinNode::Buffer(b) => b.win == win && b.primary,
            WinNode::Pair { first, second, .. } => walk(first, win) || walk(second, win),
            _ => false,
        }
    }
    walk(&Engine::screen(sess).root, win)
}

/// The GRAPHICAL main menu was never on the broken path, and this says so by
/// driving it: "New Game" is a region of a real Glk graphics window with
/// `glk_request_mouse_event`, so a click travels `glk_mouse_target` →
/// `deliver_mouse` and nothing in this quest touches it.
///
/// Here so that "clickable menus do nothing" is not re-investigated from the
/// graphics end: it is the text menus above that were dead. Cheapest case of the
/// three — it never deals a dungeon (worst turn measured 0.66s).
#[test]
fn the_graphical_main_menu_click_was_never_the_broken_path() {
    const COLS: u16 = 120;
    const ROWS: u16 = 40;
    let Some(path) = story_path() else { return };
    let mut sess = boot(&path, COLS, ROWS);
    // The boot animation runs PAST its first keypress request (the sword splash)
    // and keeps painting until the menu bar itself is on the canvas, so poll on
    // the bar's own pixels — which doubles as the guard that this is the menu
    // frame and not the splash one behind it.
    settle(&mut sess, |s| menu_bar_pixels(s) > 0);
    assert_turn_survived(&sess, "the opening animation");
    assert!(
        menu_bar_pixels(&sess) > 0,
        "the intro must reach the frame that paints the New Game / Help / Options / Quit bar"
    );

    // The menu is drawn in a real mouse-watching GRAPHICS window and carries no
    // hyperlink at all — so it is served by `glk_mouse_target`/`deliver_mouse`
    // and this quest's cell→link map has nothing to do with it either way.
    let watching = sess.mouse_windows();
    assert_eq!(watching.len(), 1, "one mouse-watching window — the menu canvas; got {watching:?}");
    let state = render_state();
    let m = render(&sess, &state, COLS, ROWS);
    assert!(m.links.is_empty(), "nothing on the menu is a hyperlink; got {:?}", m.links);

    // "New Game" sits at canvas px ~(240..362, 44..72) = cells ~(30..45, 2..4).
    // The same call main.rs makes, at the cell under its label.
    let target = app::glulx_session::glk_mouse_target(
        false,
        37,
        3,
        (0, 0, COLS, ROWS),
        &watching,
        &m.win_rects,
        sess.char_pixels(),
        None,
    )
    .expect("a click on the menu must resolve to the graphics window");
    assert_eq!(
        (target.1, target.2),
        (300, 56),
        "window-relative pixels for cell (37,3) at an 8x16 cell"
    );
    let _ = sess.deliver_mouse(target.0, target.1, target.2);

    // The game answered, and unmistakably: it dealt a dungeon. A click it
    // ignored leaves it sitting on the menu waiting for the next one.
    settle(&mut sess, |s| Engine::pending_input(s) == InputKind::Line);
    assert_turn_survived(&sess, "the New Game click");
    assert_eq!(Engine::pending_input(&sess), InputKind::Line, "New Game reaches the parser prompt");
    assert!(
        find_linked_panel(&sess, "detailed status report").is_some(),
        "clicking New Game must start a game, laying out its Statistics panel; layout is:\n{}",
        Engine::window_dump(&sess).join("\n")
    );
}

/// Lit pixels in the canvas strip the main menu paints its `New Game / Help /
/// Options / Quit` bar into (y 40..76 of the 960x624 canvas): zero until the
/// intro animation gets that far, ~3200 once it has. The intro asks for a
/// keypress at the sword splash well BEFORE it paints the bar, so polling on
/// "the game wants input" stops one frame short of the menu — this is what tells
/// the two apart.
fn menu_bar_pixels(sess: &GlulxSession) -> usize {
    fn graphics(node: &WinNode) -> Option<&app::engine::GraphicsWindow> {
        match node {
            WinNode::Graphics(g) => Some(g),
            WinNode::Pair { first, second, .. } => graphics(first).or_else(|| graphics(second)),
            _ => None,
        }
    }
    let model = Engine::screen(sess);
    let Some(g) = graphics(&model.root) else { return 0 };
    (40..76)
        .flat_map(|y| (0..g.canvas.width()).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            y < g.canvas.height() && {
                let p = g.canvas.get_pixel(x, y).0;
                p[0] as u32 + p[1] as u32 + p[2] as u32 > 60
            }
        })
        .count()
}
