//! SQ-1568: a v6 mouse click is delivered by the library, from any host.
//!
//! `app::host::input::deliver_v6_click` is what the TUI's run loop now calls on an
//! unmoved release over the game's screen; these cases drive it the way a host
//! that is not a terminal would — booted through `app::host::boot_story`, with no
//! map pane and no renderer — and assert the game's own answer.
//!
//! Specimens, with how each frame is reached (a frame is a fixture):
//!
//! | fixture                  | release | frame                                               |
//! |--------------------------|---------|-----------------------------------------------------|
//! | `zork0-r393-s890714.z6`  | 393     | Great Hall `>` LINE read: `get under table`, waits, `look` |
//! | `zork0-r393-s890714.z6`  | 393     | InvisiClues CHAR read: `hint`, then `y` (2 inputs)  |
//! | `zork1-r88-s840726.z3`   | 88      | `>` LINE read after `north` — 1 input               |
//! | `journey-r83-s890706.z6` | 83      | Praxix menu CHAR read — Enter through the intro     |
//!
//! Skip-if-missing (the stories are gitignored), and non-vacuous: each case
//! asserts the read it needs before clicking.

use std::path::{Path, PathBuf};

use app::config::Config;
use app::engine::Engine;
use app::engine_helpers::{zvm_session_opt, zvm_session_opt_mut};
use app::host::input::deliver_v6_click;
use app::host::{boot_story, BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts, TurnCtx};
use app::launch_options::LaunchOverrides;
use app::session::InputKind;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot `file` headlessly through the library, or `None` (with a SKIP note) when
/// the gitignored story is absent.
fn boot(file: &str, release: u16, tag: &str) -> Option<BootedStory> {
    let path = stories_dir().join(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), release, "{file} is not the pinned release");
    let home = app::scratch_dir(tag);
    let overrides = LaunchOverrides::default();
    let req = BootRequest {
        story_path: path,
        disk_entry: None,
        overrides: &overrides,
        cfg: config(&home),
        data_base: home.join("saves"),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
    };
    Some(boot_story(req, &mut QuietBoot).expect("the story boots headlessly"))
}

fn config(home: &Path) -> Config {
    Config {
        user_dir: home.to_path_buf(),
        config_file: home.join("config.toml"),
        random_seed: Some(1),
        ..Config::default()
    }
}

/// Deliver a click on game pixel `(x, y)` with the booted story's own context.
fn click(b: &mut BootedStory, tidy: &mut u32, game_px: (u16, u16)) -> Option<app::host::TurnOutcome> {
    let mut ctx = TurnCtx {
        game_dir: &b.game_dir,
        ifid: &b.ifid,
        arc_file: &b.arc_file,
        map_view: None,
        bg_tidy_counter: tidy,
    };
    deliver_v6_click(&mut b.state, &mut b.mapper, &mut *b.session, &mut ctx, game_px)
}

/// Answer whatever read is pending, the way a host would: a typed `line` through
/// `finish_command_turn`, a keypress (Enter) as a game-driven turn.
fn answer(b: &mut BootedStory, tidy: &mut u32, line: &str) -> String {
    match b.session.pending_input() {
        InputKind::Line => {
            let r = b.session.submit(line);
            let text = r.transcript.clone();
            let _ = app::host::finish_command_turn(
                line, true, r, &mut b.state, &mut b.mapper, &mut *b.session,
                &b.game_dir, &b.ifid, &b.arc_file, None, tidy,
            );
            text
        }
        InputKind::Char | InputKind::Event => {
            let r = zvm_session_opt_mut(&mut *b.session).expect("a z-machine story").submit_char(13);
            let text = r.transcript.clone();
            let _ = app::host::apply_game_driven_result(
                &mut b.state, &mut b.mapper, &r, &b.game_dir, None, &*b.session,
                app::pager::Driver::PlayerInput,
            );
            text
        }
    }
}

/// Zork Zero's rose occupies native x 282..372, y 6..80 of its 640x400 screen;
/// (322, 12) is deep inside the NORTH spoke (`v6_mouse_zork0`, `v6_click_vs_selection`).
const NORTH_SPOKE: (u16, u16) = (322, 12);

/// At the `>` prompt, a click on the compass's north spoke is a whole player turn:
/// the command is the game's echoed `north`, the move maps an edge, and the turn
/// counts.
#[test]
fn zork0_compass_click_at_the_prompt_is_a_north_turn_that_maps_an_edge() {
    let Some(mut b) = boot("zork0-r393-s890714.z6", 393, "host-v6-click-line") else { return };
    let mut tidy = 0u32;
    // Drive to the Great Hall (the prelude `v6_mouse_zork0` uses).
    let mut lines = ["get under table", "wait", "wait", "wait", "wait", "wait"].into_iter();
    for _ in 0..16 {
        let _ = answer(&mut b, &mut tidy, lines.next().unwrap_or("wait"));
    }
    let mut seeded = false;
    for _ in 0..6 {
        if answer(&mut b, &mut tidy, "look").contains("Great Hall") {
            seeded = true;
            break;
        }
    }
    assert!(seeded, "reached the Great Hall");
    let here = b.mapper.graph.current().expect("the Great Hall is mapped");

    // The premise: a LINE read that takes a click as its terminator.
    let z = zvm_session_opt(&*b.session).expect("a z-machine story");
    assert_eq!(z.pending_input(), InputKind::Line, "Zork Zero waits at its `>` prompt");
    assert_eq!(z.mouse_click_terminator(), Some(254), "its [255] wildcard takes a click");

    let turns = b.state.turns;
    let edges = b.mapper.graph.connections().len();
    let out = click(&mut b, &mut tidy, NORTH_SPOKE).expect("a line read that takes a click delivers it");
    assert!(!out.quit, "the game goes on");

    assert_eq!(b.state.turns, turns + 1, "a compass click is a counted player turn");
    assert_eq!(
        b.state.command_history.last().map(String::as_str),
        Some("north"),
        "the turn's command is the direction the game echoed"
    );
    let there = b.mapper.graph.current().expect("moved somewhere");
    assert_ne!(here, there, "the click moved the player");
    let conns = b.mapper.graph.connections();
    assert_eq!(conns.len(), edges + 1, "the click-driven move minted exactly one edge");
    assert!(
        conns.iter().any(|c| (c.origin, c.dir, c.dest) == (here, mapper::direction::Direction::N, there)),
        "Great Hall --N--> the room the click reached: {conns:?}"
    );
}

/// The same click during a CHAR read goes in as ZSCII 254 with its coordinates
/// recorded where `read_mouse` and the header extension table (ZMSD §11) report
/// them, and it is a game-driven turn, not a counted one.
#[test]
fn zork0_compass_click_during_a_char_read_delivers_254_and_its_coordinates() {
    let Some(mut b) = boot("zork0-r393-s890714.z6", 393, "host-v6-click-char") else { return };
    let mut tidy = 0u32;
    assert_eq!(b.session.pending_input(), InputKind::Line, "Zork Zero boots to its `>` prompt");
    let _ = answer(&mut b, &mut tidy, "hint");
    {
        let z = zvm_session_opt_mut(&mut *b.session).expect("a z-machine story");
        let entered = z.submit_char(b'y');
        assert!(entered.fault.is_none(), "entering the hint menu faulted: {:?}", entered.fault);
        assert_eq!(z.pending_input(), InputKind::Char, "the InvisiClues menu is a CHAR read");
    }

    let turns = b.state.turns;
    let out = click(&mut b, &mut tidy, NORTH_SPOKE).expect("a char read always takes a click");
    assert!(!out.quit, "the game goes on");
    assert_eq!(b.state.turns, turns, "a click answering a char read is not a counted turn");

    let z = zvm_session_opt(&*b.session).expect("a z-machine story");
    assert!(z.machine.fault_trace.is_none(), "the click faulted the VM");
    let ext = z.machine.mem.read_word(0x36) as u32;
    assert_ne!(ext, 0, "Zork Zero has a header extension table");
    assert_eq!(
        (z.machine.mem.read_word(ext + 2), z.machine.mem.read_word(ext + 4)),
        NORTH_SPOKE,
        "the click's (x, y) is what the game reads back"
    );
}

/// ...and the proof that the key WAS 254 rather than any other character: on the
/// same menu a click on a topic row selects that topic, which only a click can
/// do. GLACIER is drawn at native (87, 111) on this frame (`v6_hint_menu_mouse`).
#[test]
fn zork0_click_on_a_hint_topic_selects_it() {
    let Some(mut b) = boot("zork0-r393-s890714.z6", 393, "host-v6-click-topic") else { return };
    let mut tidy = 0u32;
    let _ = answer(&mut b, &mut tidy, "hint");
    let _ = zvm_session_opt_mut(&mut *b.session).expect("z-machine").submit_char(b'y');
    // Find GLACIER's own run rather than assuming its row.
    let run = topic_runs(&*b.session)
        .into_iter()
        .find(|t| t.text.trim() == "GLACIER")
        .expect("GLACIER is a topic on the InvisiClues menu");
    assert_ne!(selected(&*b.session), vec!["GLACIER".to_string()], "premise: not already selected");
    let out = click(&mut b, &mut tidy, (run.x + 4, run.y + 8)).expect("a char read takes a click");
    assert!(!out.quit);
    assert_eq!(selected(&*b.session), vec!["GLACIER".to_string()], "the click selected the topic under it");
}

fn topic_runs(session: &dyn Engine) -> Vec<app::engine::PxText> {
    let model = session.screen();
    let app::engine::WinNode::Layered(items) = &model.root else { return Vec::new() };
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    match layout.story.map(|st| &st.node) {
        Some(app::engine::WinNode::Grid(g)) => g.px_texts.clone(),
        _ => Vec::new(),
    }
}

fn selected(session: &dyn Engine) -> Vec<String> {
    topic_runs(session)
        .iter()
        .filter(|t| t.style & 1 != 0 && !t.text.trim().is_empty())
        .map(|t| t.text.trim().to_string())
        .collect()
}

/// A LINE read whose terminating-characters table lists no click is not the
/// game's to take a click at: `None`, and nothing about the session or the host's
/// state moves — not even the line the player has typed.
///
/// The brief named Journey r83 for this, but Journey is menu-driven (see
/// `gallery_manifest`'s `every_journey_shot_guards_its_prose`): tapped through its
/// intro it only ever waits on CHAR reads, which DO take a click
/// (`journey_menu_char_read_takes_a_click` below). So the LINE case is Zork I
/// r88's `>` prompt, the ordinary read of a story whose `mouse_click_terminator`
/// is `None` — exactly the answer `v6_click_read` sees for Journey's empty table.
#[test]
fn a_line_read_that_lists_no_click_takes_none_and_nothing_changes() {
    let Some(mut b) = boot("zork1-r88-s840726.z3", 88, "host-v6-click-noclick") else { return };
    let mut tidy = 0u32;
    let _ = answer(&mut b, &mut tidy, "north"); // a real turn first, so there is state to keep
    let z = zvm_session_opt(&*b.session).expect("a z-machine story");
    assert_eq!(z.pending_input(), InputKind::Line, "Zork I waits at its `>` prompt");
    assert_eq!(z.mouse_click_terminator(), None, "premise: this line read lists no click");

    // Something typed, which a delivered click would have taken.
    b.state.input.set("look", true);
    let snapshot = |b: &BootedStory| {
        (
            b.state.turns,
            b.state.transcript.clone(),
            b.state.command_history.clone(),
            b.state.unsaved_progress,
            b.mapper.graph.connections().len(),
            b.mapper.graph.current(),
            b.session.pending_input(),
            b.state.input.as_str().to_string(),
        )
    };
    let before = snapshot(&b);

    assert!(click(&mut b, &mut tidy, (100, 100)).is_none(), "a line read that lists no click takes none");
    assert_eq!(snapshot(&b), before, "nothing about the session or the turn state changed");
}

/// Journey r83's reads, tapped through its intro, are CHAR reads — and a char read
/// always takes a click (ZSCII 254, ZMSD §3.8), whatever the terminator table
/// says. Pinned so the `None` above is not mistaken for "Journey ignores clicks".
#[test]
fn journey_menu_char_read_takes_a_click() {
    let Some(mut b) = boot("journey-r83-s890706.z6", 83, "host-v6-click-journey") else { return };
    let mut tidy = 0u32;
    // Tap Enter through the intro vignettes to the Praxix command menu.
    for _ in 0..40 {
        if answer(&mut b, &mut tidy, "").contains("magical resources") {
            break;
        }
    }
    let z = zvm_session_opt(&*b.session).expect("a z-machine story");
    assert_eq!(z.pending_input(), InputKind::Char, "Journey's command menu is a CHAR read");
    assert_eq!(z.mouse_click_terminator(), None, "and its terminating-characters table is empty");

    let turns = b.state.turns;
    let out = click(&mut b, &mut tidy, (100, 100)).expect("a char read takes a click");
    assert!(!out.quit, "the game goes on");
    assert_eq!(b.state.turns, turns, "a click answering a char read is not a counted turn");
    let z = zvm_session_opt(&*b.session).expect("a z-machine story");
    assert!(z.machine.fault_trace.is_none(), "the click faulted the VM");
}
