//! The Journal's Hints tab (SQ-1685): the hint-story session in a tab instead of a
//! centred modal. Tab model, automatic start, lifetime across tab switches, the
//! no-hint state and its button, and — the bulk — who holds the keyboard and how
//! that is shown.
//!
//! The session is a real Z-machine VM. Cases that only need "a running hint
//! session" boot the fetched `minizork` fixture as a stand-in hint file (it reads
//! lines, which is all the focus rules need); the InvisiClues `read_char` menu case
//! boots Zork I's real `zork1izm.z5` and skips vacuously without `stories/`.
//!
//! Colour assertions run in BOTH `honor_game_colours` modes, per CLAUDE.md: the tab
//! is app chrome and the game's palette must never reach it.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use app::hints_tab::{self, HintMouse, KeyOutcome, Phase};
use app::input::{apply_action, journal_tab_click_action, key_to_action, Action};
use app::journal::{draw_tab_bar, tab_bar_cells, JournalTab, TabBarHit};
use app::keymap::Context;
use app::layout::compute_pane_layout;
use app::slash::{parse_in_context, SlashOutcome};
use app::state::{AppState, Focus, Layout};

use crate::fixture_paths::fixture_path;

const FRAME: Rect = Rect { x: 0, y: 0, width: 120, height: 40 };

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn mouse(kind: MouseEventKind, col: u16, row: u16) -> MouseEvent {
    MouseEvent { kind, column: col, row, modifiers: KeyModifiers::NONE }
}

fn mapper() -> mapper::mapper::Mapper {
    mapper::mapper::Mapper::default()
}

/// A scratch library holding a story, a stand-in hint file and the association
/// that points the first at the second.
struct Scratch {
    dir: PathBuf,
    story: PathBuf,
    ifid: &'static str,
}

/// `Some` when the minizork fixture is present (CI fetches it).
fn scratch_with_hint() -> Option<Scratch> {
    let src = fixture_path("minizork-r34-s871124.z3");
    let bytes = std::fs::read(&src).ok()?;
    let dir = app::scratch_dir("journal-hints-tab");
    let story = dir.join("story.z3");
    let hint = dir.join("hint-file.z3");
    std::fs::write(&story, &bytes).unwrap();
    std::fs::write(&hint, &bytes).unwrap();
    app::hints::save_hint_assoc(&dir, "TEST-IFID", &hint).unwrap();
    Some(Scratch { dir, story, ifid: "TEST-IFID" })
}

/// A state on `tab`, whose user dir is the scratch library.
fn state_in(s: &Scratch, tab: JournalTab) -> AppState {
    let mut st = AppState::default();
    st.config.user_dir = s.dir.clone();
    st.set_journal_tab(tab);
    st
}

/// A running Hints tab (started the way the run loop starts it).
fn running() -> Option<(AppState, Scratch)> {
    let s = scratch_with_hint()?;
    let mut st = state_in(&s, JournalTab::Hints);
    assert!(hints_tab::ensure_started(&mut st, &s.story, s.ifid));
    assert_eq!(st.hints_tab.phase, Phase::Running);
    Some((st, s))
}

/// An empty scratch directory: no hint file resolves.
fn no_hint_state() -> (AppState, PathBuf) {
    let dir = app::scratch_dir("journal-hints-none");
    let story = dir.join("lonely.z3");
    let mut st = AppState::default();
    st.config.user_dir = dir;
    st.set_journal_tab(JournalTab::Hints);
    (st, story)
}

/// Draw the tab into the Journal body the way the frame does; returns the body.
fn draw_tab(st: &AppState) -> (Buffer, Rect) {
    let pl = compute_pane_layout(FRAME, st);
    let mut buf = Buffer::empty(FRAME);
    hints_tab::draw(st, pl.journal_body, &mut buf);
    (buf, pl.journal_body)
}

fn text_in(buf: &Buffer, r: Rect) -> String {
    (r.y..r.bottom())
        .map(|y| (r.x..r.right()).map(|x| buf.cell((x, y)).map_or(" ", |c| c.symbol())).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn click_inside_window(st: &mut AppState) -> Option<HintMouse> {
    let (_, body) = draw_tab(st);
    hints_tab::on_mouse(st, &mouse(MouseEventKind::Down(MouseButton::Left), body.x + 2, body.y + 2))
}

// ── Tab model ────────────────────────────────────────────────────────────────

#[test]
fn hints_is_the_fourth_tab_on_alt_4_and_journal_tab_hints() {
    assert_eq!(
        JournalTab::ALL,
        [JournalTab::Map, JournalTab::Room, JournalTab::Inventory, JournalTab::Hints, JournalTab::Documents]
    );
    let st = AppState::default();
    let alt4 = KeyEvent::new(KeyCode::Char('4'), KeyModifiers::ALT);
    assert_eq!(key_to_action(&st, alt4), Action::SetJournalTab(JournalTab::Hints));
    assert!(matches!(
        parse_in_context("journal-tab hints", '/', Context::Global),
        SlashOutcome::Action(Action::SetJournalTab(JournalTab::Hints))
    ));
    assert!(matches!(parse_in_context("open-hints", '/', Context::Global), SlashOutcome::OpenHints));
}

#[test]
fn the_bar_names_hints_between_inventory_and_documents_and_degrades() {
    let wide = tab_bar_cells(60, JournalTab::Map);
    let text: Vec<&str> = wide.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(text, [" Map ", " Room ", " Inventory ", " Hints ", " Documents "]);
    let short = tab_bar_cells(26, JournalTab::Map);
    assert_eq!(short[3].text, " Hint ");
    let narrow = tab_bar_cells(10, JournalTab::Hints);
    assert!(narrow.iter().any(|c| c.hit == TabBarHit::Tab(JournalTab::Hints)), "the active tab always shows");
    assert_eq!(narrow.len(), 3, "‹ active ›");
}

// ── Open and lifetime ────────────────────────────────────────────────────────

#[test]
fn the_session_starts_the_first_time_the_tab_is_shown_and_not_before() {
    let Some(s) = scratch_with_hint() else {
        eprintln!("SKIP: minizork fixture absent");
        return;
    };
    let mut st = state_in(&s, JournalTab::Map);
    assert!(!hints_tab::ensure_started(&mut st, &s.story, s.ifid), "the Map tab is up: nothing starts");
    assert!(st.overlays.hints.is_none());
    assert_eq!(st.hints_tab.phase, Phase::NotStarted);

    st.set_journal_tab(JournalTab::Hints);
    assert!(hints_tab::ensure_started(&mut st, &s.story, s.ifid), "shown: it starts");
    assert_eq!(st.hints_tab.phase, Phase::Running);
    assert!(st.overlays.hints.is_some());
    assert!(!hints_tab::ensure_started(&mut st, &s.story, s.ifid), "and only once");
}

#[test]
fn a_hidden_journal_does_not_start_the_session() {
    let Some(s) = scratch_with_hint() else { return };
    let mut st = state_in(&s, JournalTab::Hints);
    st.layout = Layout::TranscriptFull;
    assert!(!hints_tab::ensure_started(&mut st, &s.story, s.ifid), "the tab is not on screen");
    assert!(st.overlays.hints.is_none());
}

#[test]
fn the_session_survives_tab_switches_as_the_same_object() {
    let Some((mut st, s)) = running() else { return };
    {
        let hs = st.overlays.hints.as_mut().unwrap();
        hs.input = "abc".into();
        hs.transcript.push("a clue I scrolled to".into());
    }
    let before = st.overlays.hints.as_ref().unwrap() as *const _;
    for tab in [JournalTab::Map, JournalTab::Documents, JournalTab::Room, JournalTab::Hints] {
        st.set_journal_tab(tab);
        hints_tab::ensure_started(&mut st, &s.story, s.ifid);
    }
    let after = st.overlays.hints.as_ref().unwrap() as *const _;
    assert_eq!(before, after, "the very same session object, never rebuilt");
    let hs = st.overlays.hints.as_ref().unwrap();
    assert_eq!(hs.input, "abc");
    assert_eq!(hs.transcript.last().map(String::as_str), Some("a clue I scrolled to"));
    assert!(!st.any_modal_overlay_open(), "a tab, not a modal: it never blocks the story");
}

#[test]
fn open_hints_shows_the_tab_and_starts_the_session_without_moving_the_cursor() {
    let Some(s) = scratch_with_hint() else { return };
    let mut st = state_in(&s, JournalTab::Map);
    st.layout = Layout::TranscriptFull;
    hints_tab::show(&mut st, &mut mapper(), &s.story, s.ifid);
    assert_eq!(st.journal_tab, JournalTab::Hints);
    assert_eq!(st.layout, Layout::Split, "the hidden Journal comes back");
    assert_eq!(st.hints_tab.phase, Phase::Running, "and the session started");
    assert_eq!(st.focus, Focus::Game, "open-hints does NOT move the cursor");
    assert!(!st.hints_have_keyboard());
}

// ── No hint file ─────────────────────────────────────────────────────────────

#[test]
fn no_hint_file_shows_the_message_and_a_download_button() {
    for honor in [true, false] {
        let (mut st, story) = no_hint_state();
        st.config.honor_game_colours = honor;
        assert!(hints_tab::ensure_started(&mut st, &story, "NOPE"));
        assert_eq!(st.hints_tab.phase, Phase::NoHint);
        assert!(st.overlays.hints.is_none());

        let (buf, body) = draw_tab(&st);
        let shown = text_in(&buf, body);
        assert!(shown.contains("no hint file found"), "honor={honor}: {shown}");
        assert!(!shown.contains("/hints"), "no advice about a command that does not exist: {shown}");
        assert!(shown.contains("link this game to IFDB"), "unlinked: say to link first: {shown}");
        assert!(shown.contains("[ Download hints… ]"), "{shown}");

        let btn = st.hints_tab.hits().download.expect("the button is a hit target");
        let click = mouse(MouseEventKind::Down(MouseButton::Left), btn.x + 1, btn.y);
        assert_eq!(hints_tab::on_mouse(&mut st, &click), Some(HintMouse::Command("download-hints")));
        assert_eq!(st.focus, Focus::Game, "no session, nothing to focus");
    }
}

#[test]
fn download_hints_is_one_command_in_the_game_and_the_browser() {
    assert!(matches!(parse_in_context("download-hints", '/', Context::Global), SlashOutcome::DownloadHints));
    assert!(matches!(parse_in_context("download-hints", '/', Context::Browser), SlashOutcome::DownloadHints));
    assert_eq!(
        app::browser::action_for_command("download-hints"),
        Some(app::browser::BrowserAction::DownloadHints)
    );
    let spec = app::slash::find_command("download-hints").expect("registered");
    assert!(app::slash::in_both_worlds(spec));
    assert!(app::slash::slash_names().iter().any(|n| n == "download-hints"), "the in-game palette offers it");
}

#[test]
fn unrelated_generic_sidecars_are_not_a_choice_and_show_the_no_hint_body() {
    // Two story-less generic sidecars beside an unrelated story: nothing says either
    // is THIS game's, so the tab does not offer them (a shared stories folder would
    // otherwise list every hint file in it).
    let dir = app::scratch_dir("journal-hints-ambiguous");
    for name in ["zork1inv.z5", "zork2inv.z5"] {
        std::fs::write(dir.join(name), b"x").unwrap();
    }
    let story = dir.join("unrelated.z3");
    let mut st = AppState::default();
    st.config.user_dir = dir;
    st.set_journal_tab(JournalTab::Hints);
    hints_tab::ensure_started(&mut st, &story, "AMBIG");
    assert_eq!(st.hints_tab.phase, Phase::NoHint);
    let (buf, body) = draw_tab(&st);
    assert!(text_in(&buf, body).contains("Download hints"));
}

#[test]
fn a_linked_game_s_no_hint_body_names_its_documents_folder() {
    let (mut st, story) = no_hint_state();
    let docs = app::scratch_dir("journal-hints-docs-empty").join("Lonely [tuid]");
    assert!(hints_tab::ensure_started_in(&mut st, &story, "NOPE", Some(&docs)));
    assert_eq!(st.hints_tab.phase, Phase::NoHint);
    let (buf, body) = draw_tab(&st);
    // Collapse whitespace (and drop the frame's side borders): where the path wraps depends on
    // the platform's temp-dir length, and each wrap puts a `│` pair between the pieces.
    let shown = text_in(&buf, body)
        .split_whitespace()
        .filter(|t| *t != "│")
        .collect::<Vec<_>>()
        .join(" ");
    assert!(shown.contains("documents folder"), "{shown}");
    assert!(shown.contains("Lonely [tuid]"), "the path is shown: {shown}");
    assert!(!shown.contains("link this game to IFDB"), "{shown}");
}

// ── SQ-1690: documents folder first, and the chooser ─────────────────────────

/// A state on the Hints tab with a documents folder holding `names` (each the
/// minizork stand-in), or `None` without the fixture.
fn with_docs(tag: &str, names: &[&str]) -> Option<(AppState, PathBuf, PathBuf)> {
    let bytes = std::fs::read(fixture_path("minizork-r34-s871124.z3")).ok()?;
    let dir = app::scratch_dir(tag);
    let story = dir.join("story.z3");
    std::fs::write(&story, &bytes).unwrap();
    let docs = dir.join("Game [tuid]");
    std::fs::create_dir_all(&docs).unwrap();
    for n in names {
        std::fs::write(docs.join(n), &bytes).unwrap();
    }
    let mut st = AppState::default();
    st.config.user_dir = dir;
    st.set_journal_tab(JournalTab::Hints);
    Some((st, story, docs))
}

#[test]
fn a_hint_program_in_the_documents_folder_starts_the_session() {
    let Some((mut st, story, docs)) = with_docs("journal-hints-docs-first", &["whatever-hints.z3"]) else { return };
    assert!(hints_tab::ensure_started_in(&mut st, &story, "IFID", Some(&docs)));
    assert_eq!(st.hints_tab.phase, Phase::Running);
    assert_eq!(st.overlays.hints.as_ref().unwrap().label, "whatever-hints.z3");
}

#[test]
fn tied_hint_programs_show_a_chooser_that_a_click_resolves_and_remembers() {
    for honor in [true, false] {
        let Some((mut st, story, docs)) = with_docs("journal-hints-choose", &["aaa-inv.z3", "bbb-inv.z3"]) else { return };
        st.config.honor_game_colours = honor;
        assert!(hints_tab::ensure_started_in(&mut st, &story, "IFID", Some(&docs)));
        assert!(matches!(&st.hints_tab.phase, Phase::Choose(c) if c.len() == 2), "{:?}", st.hints_tab.phase);
        assert!(st.overlays.hints.is_none());

        let (buf, body) = draw_tab(&st);
        let shown = text_in(&buf, body);
        assert!(shown.contains("aaa-inv.z3") && shown.contains("bbb-inv.z3"), "honor={honor}: {shown}");
        let rows = st.hints_tab.hits().choices;
        assert_eq!(rows.len(), 2);

        // Click the second row: it is remembered per IFID and the session starts.
        let click = mouse(MouseEventKind::Down(MouseButton::Left), rows[1].x + 1, rows[1].y);
        assert_eq!(hints_tab::on_mouse(&mut st, &click), Some(HintMouse::Handled));
        assert!(hints_tab::ensure_started_in(&mut st, &story, "IFID", Some(&docs)));
        assert_eq!(st.hints_tab.phase, Phase::Running);
        assert_eq!(st.overlays.hints.as_ref().unwrap().label, "bbb-inv.z3");
        let index = app::hints::load_hint_index(&st.config.user_dir);
        assert_eq!(index.get("IFID"), Some(docs.join("bbb-inv.z3")), "remembered in hints/index.toml");

        // A fresh run resolves it straight away, no chooser.
        let mut next = AppState::default();
        next.config.user_dir = st.config.user_dir.clone();
        next.set_journal_tab(JournalTab::Hints);
        hints_tab::ensure_started_in(&mut next, &story, "IFID", Some(&docs));
        assert_eq!(next.hints_tab.phase, Phase::Running);
    }
}

#[test]
fn the_chooser_takes_the_keyboard_modelessly_and_arrows_enter_and_esc_work() {
    let Some((mut st, story, docs)) = with_docs("journal-hints-choose-keys", &["aaa-inv.z3", "bbb-inv.z3"]) else { return };
    hints_tab::ensure_started_in(&mut st, &story, "IFID", Some(&docs));
    // Showing the chooser does not move the keyboard; typing is the story's.
    assert_eq!(st.focus, Focus::Game);
    assert_eq!(hints_tab::on_key(&mut st, key(KeyCode::Down)), KeyOutcome::PassThrough);
    // Tab (the ordinary focus cycle) brings the keyboard in, Esc gives it back.
    st.cycle_focus(true);
    assert_eq!(st.focus, Focus::Hints);
    assert_eq!(hints_tab::on_key(&mut st, key(KeyCode::Down)), KeyOutcome::Handled);
    assert_eq!(st.hints_tab.choice, 1);
    assert_eq!(hints_tab::on_key(&mut st, key(KeyCode::Up)), KeyOutcome::Handled);
    assert_eq!(hints_tab::on_key(&mut st, key(KeyCode::Down)), KeyOutcome::Handled);
    assert_eq!(hints_tab::on_key(&mut st, key(KeyCode::Enter)), KeyOutcome::Handled);
    hints_tab::ensure_started_in(&mut st, &story, "IFID", Some(&docs));
    assert_eq!(st.overlays.hints.as_ref().unwrap().label, "bbb-inv.z3");

    let Some((mut st2, story2, docs2)) = with_docs("journal-hints-choose-esc", &["aaa-inv.z3", "bbb-inv.z3"]) else { return };
    hints_tab::ensure_started_in(&mut st2, &story2, "IFID", Some(&docs2));
    st2.cycle_focus(true);
    assert_eq!(hints_tab::on_key(&mut st2, key(KeyCode::Esc)), KeyOutcome::Handled);
    assert_eq!(st2.focus, Focus::Game);
}

#[test]
fn download_hints_for_a_linked_game_saves_into_its_documents_folder() {
    // The destination rule the tab hands the downloader: documents folder when the
    // game has one, beside the story otherwise (the finished-file behaviour is
    // pinned in hint_download's own cases).
    use app::hint_download::HintDest;
    let story = std::path::Path::new("/lib/deadline.z3");
    let docs = PathBuf::from("/docs/Deadline [t]");
    assert!(matches!(
        HintDest::for_story(story, "deadlineinv.z5", Some(docs)),
        HintDest::Documents { filename, .. } if filename == "deadlineinv.z5"
    ));
    assert_eq!(HintDest::for_story(story, "deadlineinv.z5", None), HintDest::Beside("/lib/deadlineinv.z5".into()));
}

// ── Focus: who gets the keyboard ─────────────────────────────────────────────

#[test]
fn the_tab_label_alt_4_and_open_hints_do_not_move_focus() {
    let Some((mut st, s)) = running() else { return };
    st.set_journal_tab(JournalTab::Map);
    // Clicking the tab label.
    apply_action(journal_tab_click_action(TabBarHit::Tab(JournalTab::Hints)), &mut st, &mut mapper());
    assert_eq!(st.journal_tab, JournalTab::Hints);
    assert_eq!(st.focus, Focus::Game, "the label click shows the tab; the cursor stays in the story");
    // Alt+4.
    st.set_journal_tab(JournalTab::Map);
    let a = key_to_action(&st, KeyEvent::new(KeyCode::Char('4'), KeyModifiers::ALT));
    apply_action(a, &mut st, &mut mapper());
    assert_eq!((st.journal_tab, st.focus), (JournalTab::Hints, Focus::Game));
    // open-hints.
    st.set_journal_tab(JournalTab::Map);
    hints_tab::show(&mut st, &mut mapper(), &s.story, s.ifid);
    assert_eq!((st.journal_tab, st.focus), (JournalTab::Hints, Focus::Game));
    assert!(!st.hints_have_keyboard());
}

#[test]
fn only_a_click_inside_the_hint_window_gives_it_the_keyboard() {
    let Some((mut st, _s)) = running() else { return };
    assert_eq!(click_inside_window(&mut st), Some(HintMouse::Handled));
    assert_eq!(st.focus, Focus::Hints);
    assert!(st.hints_have_keyboard());
}

#[test]
fn a_click_on_the_input_row_also_takes_it() {
    let Some((mut st, _s)) = running() else { return };
    let (_, _) = draw_tab(&st);
    let input = st.hints_tab.hits().input;
    let click = mouse(MouseEventKind::Down(MouseButton::Left), input.x + 1, input.y);
    assert_eq!(hints_tab::on_mouse(&mut st, &click), Some(HintMouse::Handled));
    assert!(st.hints_have_keyboard());
}

#[test]
fn clicking_the_story_pane_returns_the_keyboard() {
    let Some((mut st, _s)) = running() else { return };
    click_inside_window(&mut st);
    assert!(st.hints_have_keyboard());
    // Left-down in the story is `StartSelection` (it also activates the game pane).
    apply_action(Action::StartSelection(5, 5), &mut st, &mut mapper());
    assert_eq!(st.focus, Focus::Game);
    assert!(!st.hints_have_keyboard());
}

#[test]
fn esc_in_the_hint_input_returns_the_keyboard() {
    let Some((mut st, _s)) = running() else { return };
    click_inside_window(&mut st);
    assert_eq!(hints_tab::on_key(&mut st, key(KeyCode::Esc)), KeyOutcome::Handled);
    assert_eq!(st.focus, Focus::Game);
    assert!(st.overlays.hints.is_some(), "Esc leaves; it does not close the session");
}

#[test]
fn switching_to_another_tab_returns_the_keyboard() {
    let Some((mut st, _s)) = running() else { return };
    click_inside_window(&mut st);
    assert!(st.hints_have_keyboard());
    st.set_journal_tab(JournalTab::Inventory);
    assert_eq!(st.focus, Focus::Game);
    st.set_journal_tab(JournalTab::Hints);
    assert!(!st.hints_have_keyboard(), "coming back does not hand it over again");
}

#[test]
fn tab_on_an_empty_story_prompt_reaches_the_hint_input_only_while_the_tab_is_visible() {
    let Some((mut st, _s)) = running() else { return };
    // Visible: story → hints.
    assert!(st.input.is_empty());
    let a = key_to_action(&st, key(KeyCode::Tab));
    assert_eq!(a, Action::ToggleFocus);
    apply_action(a, &mut st, &mut mapper());
    assert!(st.hints_have_keyboard(), "Tab from the story reaches the hint input");
    // Shift-Tab reverses: hints → story.
    st.cycle_focus(false);
    assert_eq!(st.focus, Focus::Game);

    // Another tab up: Tab is inert, as ever (SQ-0599) — the map is never a stop either.
    st.set_journal_tab(JournalTab::Map);
    st.cycle_focus(true);
    assert_eq!(st.focus, Focus::Game, "no hint window on screen, no stop");
    // Hidden Journal: same.
    st.set_journal_tab(JournalTab::Hints);
    st.layout = Layout::TranscriptFull;
    st.cycle_focus(true);
    assert_eq!(st.focus, Focus::Game);
}

#[test]
fn tab_has_no_hint_stop_without_a_session() {
    let (mut st, story) = no_hint_state();
    hints_tab::ensure_started(&mut st, &story, "NOPE");
    st.cycle_focus(true);
    assert_eq!(st.focus, Focus::Game, "nothing to focus: the keyboard can never sit in an invisible window");
}

#[test]
fn the_hint_stop_comes_before_the_debug_windows() {
    let Some((mut st, _s)) = running() else { return };
    // The inspector takes the Journal's slot, so the two never show together; the
    // order is still story → hints → debug windows → story where both stops exist.
    st.debug = Some(app::debug_panel::DebugPanelState::new(0));
    st.cycle_focus(true);
    assert_eq!(st.focus, Focus::Map, "with the inspector up the hint tab is hidden: straight to debug 0");
    st.cycle_focus(false);
    assert_eq!(st.focus, Focus::Game);
}

#[test]
fn tab_on_an_empty_hint_input_returns_to_the_story_and_a_typed_one_keeps_it() {
    let Some((mut st, _s)) = running() else { return };
    st.overlays.hints.as_mut().unwrap().input.clear();
    st.cycle_focus(true);
    assert!(st.hints_have_keyboard());
    // Text typed: Tab is not a way out (it would drop the line unseen).
    assert_eq!(hints_tab::on_key(&mut st, key(KeyCode::Char('x'))), KeyOutcome::Handled);
    assert_eq!(hints_tab::on_key(&mut st, key(KeyCode::Tab)), KeyOutcome::Handled);
    assert!(st.hints_have_keyboard(), "a non-empty input keeps Tab");
    assert_eq!(st.overlays.hints.as_ref().unwrap().input, "x");
    // Emptied: Tab returns it.
    hints_tab::on_key(&mut st, key(KeyCode::Backspace));
    assert_eq!(hints_tab::on_key(&mut st, key(KeyCode::Tab)), KeyOutcome::Handled);
    assert_eq!(st.focus, Focus::Game);
    // And Shift-Tab (BackTab) the same way.
    st.cycle_focus(true);
    hints_tab::on_key(&mut st, KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert_eq!(st.focus, Focus::Game);
}

#[test]
fn keys_reach_the_hint_vm_when_focused_and_the_story_when_not() {
    let Some((mut st, _s)) = running() else { return };
    st.overlays.hints.as_mut().unwrap().input.clear();
    let q = key(KeyCode::Char('q'));

    // Story focus: the key is the story's, the hint input is untouched.
    assert_eq!(hints_tab::on_key(&mut st, q), KeyOutcome::PassThrough);
    assert!(matches!(key_to_action(&st, q), Action::InputChar('q')));
    assert_eq!(st.overlays.hints.as_ref().unwrap().input, "");

    // Hint focus: the hint session takes it.
    click_inside_window(&mut st);
    assert_eq!(hints_tab::on_key(&mut st, q), KeyOutcome::Handled);
    assert_eq!(st.overlays.hints.as_ref().unwrap().input, "q");
    assert!(st.input.is_empty(), "…and the story's prompt did not");

    // Ctrl/Alt chords keep their usual meaning even then (quit, tab keys).
    let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(hints_tab::on_key(&mut st, ctrl_c), KeyOutcome::PassThrough);
    let alt1 = KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT);
    assert_eq!(hints_tab::on_key(&mut st, alt1), KeyOutcome::PassThrough);
}

#[test]
fn page_keys_scroll_the_transcript_while_hint_focused_and_the_wheel_always_does() {
    let Some((mut st, _s)) = running() else { return };
    {
        let hs = st.overlays.hints.as_mut().unwrap();
        hs.transcript = (0..200).map(|i| format!("clue line {i}")).collect();
    }
    let (_, body) = draw_tab(&st); // records the max scroll
    assert!(st.hints_tab.hits().max_scroll > 20);

    // The wheel scrolls with the keyboard in the STORY.
    assert_eq!(st.focus, Focus::Game);
    let wheel = mouse(MouseEventKind::ScrollUp, body.x + 2, body.y + 2);
    assert_eq!(hints_tab::on_mouse(&mut st, &wheel), Some(HintMouse::Handled));
    assert_eq!(st.overlays.hints.as_ref().unwrap().scroll, 1, "wheel scrolls regardless of focus");

    // PgUp/PgDn while hint-focused.
    click_inside_window(&mut st);
    assert_eq!(hints_tab::on_key(&mut st, key(KeyCode::PageUp)), KeyOutcome::Handled);
    assert!(st.overlays.hints.as_ref().unwrap().scroll >= 11);
    hints_tab::on_key(&mut st, key(KeyCode::PageDown));
    assert_eq!(st.overlays.hints.as_ref().unwrap().scroll, 1);
}

#[test]
fn a_mouse_event_outside_the_tab_is_not_the_tabs() {
    let Some((mut st, _s)) = running() else { return };
    let (_, body) = draw_tab(&st);
    let outside = mouse(MouseEventKind::Down(MouseButton::Left), 0, body.bottom() + 1);
    assert_eq!(hints_tab::on_mouse(&mut st, &outside), None);
    assert_eq!(st.focus, Focus::Game);
}

#[test]
fn focus_cannot_outlive_the_visible_hint_window() {
    let Some((mut st, _s)) = running() else { return };
    click_inside_window(&mut st);
    st.layout = Layout::TranscriptFull; // the Journal is hidden
    assert!(!st.hints_have_keyboard(), "a hidden window cannot hold the keyboard");
    assert_eq!(hints_tab::on_key(&mut st, key(KeyCode::Char('z'))), KeyOutcome::PassThrough);
}

// ── Focus indicator ──────────────────────────────────────────────────────────

fn input_row_cells(buf: &Buffer, st: &AppState) -> Vec<(u16, Modifier)> {
    let r = st.hints_tab.hits().input;
    (r.x..r.right()).map(|x| (x, buf.cell((x, r.y)).unwrap().modifier)).collect()
}

#[test]
fn the_cursor_shows_only_in_the_focused_input_and_the_other_is_dimmed() {
    for honor in [true, false] {
        let Some((mut st, _s)) = running() else { return };
        st.config.honor_game_colours = honor;

        // Story has the keyboard: the hint input is dim and has no cursor.
        let (buf, _) = draw_tab(&st);
        let cells = input_row_cells(&buf, &st);
        assert!(cells.iter().all(|(_, m)| m.contains(Modifier::DIM)), "honor={honor}: unfocused hint input dimmed");
        assert!(cells.iter().all(|(_, m)| !m.contains(Modifier::REVERSED)), "honor={honor}: no cursor in it");

        // Hint session has it: not dim, one cursor cell.
        click_inside_window(&mut st);
        let (buf, _) = draw_tab(&st);
        let cells = input_row_cells(&buf, &st);
        assert!(cells.iter().all(|(_, m)| !m.contains(Modifier::DIM)), "honor={honor}: focused input is not dim");
        assert_eq!(
            cells.iter().filter(|(_, m)| m.contains(Modifier::REVERSED)).count(),
            1,
            "honor={honor}: exactly one cursor cell"
        );
    }
}

#[test]
fn the_hints_tab_label_wears_a_focus_marker_only_while_hint_focused() {
    for honor in [true, false] {
        let mut st = AppState::default();
        st.config.honor_game_colours = honor;
        let r = Rect::new(0, 0, 60, 1);

        let mut plain = Buffer::empty(r);
        let hits = draw_tab_bar(r, JournalTab::Hints, false, &st.colors, &mut plain);
        let hint_rect = hits.iter().find(|(h, _)| *h == TabBarHit::Tab(JournalTab::Hints)).unwrap().1;
        assert_eq!(plain.cell((hint_rect.x, r.y)).unwrap().symbol(), " ");

        let mut marked = Buffer::empty(r);
        draw_tab_bar(r, JournalTab::Map, true, &st.colors, &mut marked);
        assert_eq!(
            marked.cell((hint_rect.x, r.y)).unwrap().symbol(),
            "\u{25b8}",
            "honor={honor}: the marker takes the label's leading pad, shifting nothing"
        );
        // The other labels are unmarked.
        assert_eq!(marked.cell((1, r.y)).unwrap().symbol(), "M");
    }
}

// ── The real InvisiClues menu (read_char) ────────────────────────────────────

/// Zork I's real hint file beside its story: `None` (skip) without `stories/`.
fn real_izm() -> Option<(AppState, PathBuf)> {
    let story = fixture_path("zork1-r88-s840726.z3");
    if !story.is_file() || !story.with_file_name("zork1izm.z5").is_file() {
        return None;
    }
    let bytes = std::fs::read(&story).ok()?;
    let ifid = app::ifid::compute_ifid(&bytes);
    let mut st = AppState::default();
    st.config.user_dir = app::scratch_dir("journal-hints-izm");
    st.set_journal_tab(JournalTab::Hints);
    assert!(hints_tab::ensure_started(&mut st, &story, &ifid));
    Some((st, story))
}

fn menu_screen(st: &AppState) -> String {
    let hs = st.overlays.hints.as_ref().unwrap();
    let app::state::HintSource::Zcode(vm) = &hs.source;
    let model = app::session::screen_model_from_machine(&vm.machine);
    model.grid().map(|g| g.cells.iter().map(|c| c.ch).collect()).unwrap_or_default()
}

#[test]
fn a_read_char_menu_keystroke_reaches_the_hint_vm_when_focused_and_the_story_when_not() {
    let Some((mut st, _story)) = real_izm() else {
        eprintln!("SKIP: stories/zork1izm.z5 absent");
        return;
    };
    assert_eq!(st.hints_tab.phase, Phase::Running);
    {
        let hs = st.overlays.hints.as_ref().unwrap();
        let app::state::HintSource::Zcode(vm) = &hs.source;
        assert_eq!(vm.pending_input(), app::session::InputKind::Char, "the izm menu navigates by keypress");
    }
    let before = menu_screen(&st);

    // Story focus: Enter is the story's (it submits its prompt); the menu does not move.
    assert_eq!(hints_tab::on_key(&mut st, key(KeyCode::Enter)), KeyOutcome::PassThrough);
    assert_eq!(menu_screen(&st), before);

    // Hint focus: Enter drives the menu — and is never buffered as text.
    click_inside_window(&mut st);
    assert_eq!(hints_tab::on_key(&mut st, key(KeyCode::Enter)), KeyOutcome::Handled);
    assert_ne!(menu_screen(&st), before, "the menu advanced: the key reached the VM");
    assert!(st.overlays.hints.as_ref().unwrap().input.is_empty());

    // Arrows are the menu's too, and Esc still hands the keyboard back.
    for code in [KeyCode::Down, KeyCode::Up] {
        assert_eq!(hints_tab::on_key(&mut st, key(code)), KeyOutcome::Handled);
    }
    assert!(st.hints_have_keyboard());
    assert_eq!(hints_tab::on_key(&mut st, key(KeyCode::Esc)), KeyOutcome::Handled);
    assert_eq!(st.focus, Focus::Game);
    // Tab in a char-mode menu counts as an empty input: it leaves.
    click_inside_window(&mut st);
    hints_tab::on_key(&mut st, key(KeyCode::Tab));
    assert_eq!(st.focus, Focus::Game);
}

#[test]
fn the_real_menu_draws_in_the_tab_with_its_prompt_in_both_focus_states() {
    let Some((mut st, _story)) = real_izm() else { return };
    let (buf, body) = draw_tab(&st);
    let shown = text_in(&buf, body);
    assert!(shown.contains("click here, or Tab"), "unfocused char-mode prompt: {shown}");
    click_inside_window(&mut st);
    let (buf, body) = draw_tab(&st);
    assert!(text_in(&buf, body).contains("press a key"), "focused char-mode prompt");
}
