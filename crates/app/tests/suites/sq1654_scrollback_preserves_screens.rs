//! SQ-1654: a game-driven screen clear (`apply_game_driven_result`,
//! `crates/app/src/host/turn.rs`) used to collapse the PRIOR screen
//! unconditionally whenever `result.erase_lower` was set and a `clear_anchor`
//! was already recorded — correct for Counterfeit Monkey's arrow-navigated
//! help menu (same menu, reprinted on every keypress; collapsing it is the
//! whole point, SQ-0407), but wrong for a totally different screen taking
//! over: Anchorhead's intro→quote→game-start sequence of "press any key"
//! screens, and clicking an inline picture then returning. Because
//! `clear_anchor` is STICKY (only ever moves forward on another clear), the
//! second failure mode is worse than "lose the screen right before a
//! clear" — the FIRST game-driven clear in a session plants a landmine that
//! the NEXT one detonates, discarding every real turn played in between.
//!
//! `is_screen_reprint` (`crates/app/src/host/turn.rs`) now gates the
//! collapse on whether the new content actually looks like the old screen
//! redrawing itself (>=50% of its non-blank lines matching), rather than
//! collapsing on every clear. `host::turn::tests::
//! game_driven_screen_clear_collapses_menu_reprints` /
//! `_preserves_dissimilar_screens` prove that gate as a unit; the three cases
//! below drive REAL commercial archives through the real host turn-apply path
//! — the first two prove the actual reported symptoms are gone (Anchorhead),
//! the third proves the ORIGINAL SQ-0407 collapse case this gate must not
//! regress (Counterfeit Monkey's real arrow-navigated help menu).
//!
//! Skips vacuously without the gitignored `stories/Anchorhead.gblorb` (and,
//! for the second case, `stories/Anchorhead-thumbnail.lanthorn` beside it; for
//! the third, `stories/CounterfeitMonkey-11.gblorb`).

use app::engine::{Engine, KeyInput};
use app::engine_helpers::glulx_session_opt_mut;
use app::host::{apply_game_driven_result, boot_story, finish_command_turn, BootRequest, LaunchFlags, QuietBoot, TerminalFacts};
use app::launch_options::LaunchOverrides;
use app::pager::Driver;
use app::session::{InputKind, TurnResult};

use crate::fixture_paths::fixture_path;

/// Boot a real, gitignored story headlessly through the same `boot_story`
/// production path the TUI uses, with no prior save to resume.
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
            ..app::config::Config::default()
        },
        roots: app::data_roots::DataRoots::single(home.join("saves")),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
        fresh_start: true,
    };
    boot_story(req, &mut QuietBoot).expect("the story boots headlessly")
}

/// The user's reported symptom #1: the intro card, the epigraph quote splash,
/// and the title banner/first-room screen are three SEPARATE `read_char`
/// "press any key" turns, each its own `erase_lower` game-driven clear, with
/// essentially zero line overlap between them (confirmed directly against
/// this archive: press 0 is the intro, press 1 is "THE FIRST DAY" / H. P.
/// Lovecraft epigraph, press 2 is the "Anchorhead" banner opening onto
/// "Outside the Real Estate Office"). All three must still be readable in
/// scrollback once gameplay starts, not just the last one.
#[test]
fn anchorhead_intro_quote_and_gameplay_all_survive_in_scrollback() {
    let story = fixture_path("Anchorhead.gblorb");
    if !story.is_file() {
        eprintln!("SKIP: no Anchorhead.gblorb");
        return;
    }
    let home = app::scratch_dir("sq1651-intro");
    let mut b = boot_headless(story, &home);
    assert_eq!(b.session.pending_input(), InputKind::Char, "premise: the intro waits for a keypress");

    for press in 0..3 {
        let result = b.session.submit_key(KeyInput::Char(' ')).expect("key reaches the story");
        assert!(result.erase_lower, "press {press}: premise — each intro screen is its own game-driven clear");
        let out = apply_game_driven_result(&mut b.state, &mut b.mapper, &result, &b.game_dir, None, &*b.session, Driver::PlayerInput);
        assert!(!out.quit, "press {press}: the game goes on");
    }

    assert_eq!(b.session.pending_input(), InputKind::Line, "premise: gameplay is reached (a line prompt)");
    let text = b.state.transcript.join("\n");
    assert!(
        text.contains("Welcome to Anchorhead") && text.contains("first raindrops"),
        "the intro card must still be in scrollback: {text}"
    );
    assert!(
        text.contains("THE FIRST DAY") && text.contains("H. P. Lovecraft") && text.contains("The Festival"),
        "the epigraph quote splash must still be in scrollback: {text}"
    );
    assert!(
        text.contains("Outside the Real Estate Office") && text.contains("an interactive Lovecraftian gothic"),
        "the title banner and first room must still be in scrollback: {text}"
    );

    let _ = std::fs::remove_dir_all(&home);
}

/// The user's reported symptom #2, and the more severe one: `clear_anchor` is
/// sticky, so once ANY game-driven clear has happened, the picture-click's
/// own empty-content clear plants an anchor that a LATER, unrelated
/// game-driven clear detonates — wiping every real turn played in between,
/// not just the room text right before the click.
///
/// Reproduces the exact sequence confirmed directly against this fixture:
/// restore the user's own host snapshot of a thumbnail moment
/// (`Anchorhead-thumbnail.lanthorn`), click the thumbnail (empty-content
/// `erase_lower`, `clear_anchor` was `None` so nothing truncates yet, then
/// gets set to the pre-click length), "return" (another `erase_lower`,
/// reprints the room — a no-op truncate since nothing was added yet), drive
/// three REAL typed commands through `finish_command_turn` (`look`,
/// `inventory`, `examine me` — real scrollback, `clear_anchor` untouched
/// since neither clears), then a SECOND game-driven clear (simulated with
/// empty content, the real shape a second thumbnail click takes — this
/// fixture only has the one real clickable thumbnail). Without the fix, that
/// second clear unconditionally truncates back to the FIRST click's anchor,
/// discarding the return-reprint and all three intervening real commands.
#[test]
fn picture_click_and_return_do_not_wipe_intervening_play() {
    let story = fixture_path("Anchorhead.gblorb");
    let snap = fixture_path("Anchorhead-thumbnail.lanthorn");
    if !story.is_file() || !snap.is_file() {
        eprintln!("SKIP: no Anchorhead.gblorb and/or Anchorhead-thumbnail.lanthorn");
        return;
    }
    let home = app::scratch_dir("sq1651-thumb");
    let mut b = boot_headless(story, &home);

    // Restore the user's own host snapshot of the thumbnail moment — the same
    // production path `/restore-state` takes.
    app::host::persist::restore_file(&mut *b.session, &mut b.mapper, &mut b.state, &snap, None)
        .expect("restore the host snapshot");
    assert_eq!(b.state.clear_anchor, None, "premise: a fresh restore starts with no clear anchor");
    let restored_len = b.state.transcript.len();

    let (_, link) = b
        .state
        .transcript_images
        .iter()
        .enumerate()
        .find_map(|(i, o)| o.as_ref().map(|im| (i, im.link)))
        .expect("the snapshot must hold the thumbnail moment");
    assert_ne!(link, 0, "premise: the thumbnail carries the game's own hyperlink value");

    // Click the thumbnail: a real game-driven `erase_lower` turn with EMPTY
    // new content — it opens a graphics window and prints nothing.
    let win = {
        let gs = glulx_session_opt_mut(&mut *b.session).expect("Anchorhead is a Glulx game");
        gs.hyperlink_windows().first().copied().expect("hyperlink window armed")
    };
    let click_result = glulx_session_opt_mut(&mut *b.session).unwrap().deliver_hyperlink(win, link);
    assert!(click_result.erase_lower, "premise: opening the full illustration clears the lower window");
    assert!(click_result.transcript.is_empty(), "premise: the click's own turn prints nothing");
    let out = apply_game_driven_result(&mut b.state, &mut b.mapper, &click_result, &b.game_dir, None, &*b.session, Driver::PlayerInput);
    assert!(!out.quit);
    assert_eq!(b.state.clear_anchor, Some(restored_len), "the click anchors at the restored length");

    // "Return": a keypress that closes the illustration and reprints the room.
    let return_result = b.session.submit_key(KeyInput::Char(' ')).expect("key reaches the story");
    assert!(return_result.erase_lower, "premise: closing the illustration clears again");
    let out = apply_game_driven_result(&mut b.state, &mut b.mapper, &return_result, &b.game_dir, None, &*b.session, Driver::PlayerInput);
    assert!(!out.quit);
    assert!(
        b.state.transcript.iter().any(|l| l.contains("Outside the Real Estate Office")),
        "the return's room reprint must be present"
    );

    // Several REAL typed commands — real turns through `finish_command_turn`,
    // not synthetic pushes — build real scrollback after the click+return.
    let mut tidy = 0u32;
    for cmd in ["look", "inventory", "examine me"] {
        let result = b.session.submit(cmd);
        let out = finish_command_turn(
            cmd, true, result, &mut b.state, &mut b.mapper, &mut *b.session,
            &b.game_dir, &b.ifid, &b.arc_file, None, &mut tidy,
        );
        assert!(!out.quit, "{cmd}: the game goes on");
    }
    assert_eq!(
        b.state.clear_anchor,
        Some(restored_len),
        "premise: clear_anchor is STICKY — still the first click's boundary after three \
         ordinary (non-clearing) commands"
    );
    let len_before_second_click = b.state.transcript.len();
    let text_before = b.state.transcript.join("\n");
    assert!(text_before.contains("You are wearing your trenchcoat"), "premise: inventory's reply is in scrollback");
    assert!(text_before.contains("You look good, considering."), "premise: examine me's reply is in scrollback");

    // A SECOND picture click, later in the game. The real thumbnail-click
    // shape is empty new content with `erase_lower`; simulated directly since
    // this fixture only carries one real clickable thumbnail moment.
    let second_click = TurnResult {
        transcript: String::new(),
        transcript_runs: Vec::new(),
        location: None,
        quit: false,
        erase_lower: true,
        info: None,
        sounds: Vec::new(),
        glulx_sound_ops: Vec::new(),
        diagnostics: vec![],
        fault: None,
        location_method: None,
        pending_io: None,
        timed_out: false,
        pictures: Vec::new(),
        transcript_elems: Vec::new(),
        prose_retired: None,
        declared_exit: None,
        description: None,
        items: Vec::new(),
    };
    let out = apply_game_driven_result(&mut b.state, &mut b.mapper, &second_click, &b.game_dir, None, &*b.session, Driver::PlayerInput);
    assert!(!out.quit);

    let text_after = b.state.transcript.join("\n");
    assert!(
        text_after.contains("You are wearing your trenchcoat"),
        "the inventory reply — a real turn played between the two clicks — must survive \
         the second click; clear_anchor is sticky and an unconditional truncate here would \
         wipe everything back to the FIRST click's boundary (SQ-1654). Transcript now:\n{text_after}"
    );
    assert!(
        text_after.contains("You look good, considering."),
        "the examine-me reply must survive the second click too: {text_after}"
    );
    assert!(
        b.state.transcript.len() >= len_before_second_click,
        "an empty-content clear with no line overlap must not shrink the transcript"
    );

    let _ = std::fs::remove_dir_all(&home);
}

/// The ORIGINAL SQ-0407 case, against the real game: Counterfeit Monkey's
/// in-game `HINT` menu (reached after its accessibility gate: "yes", a name,
/// a blank keypress, `TUTORIAL OFF`, then `HINT`) is a real arrow-navigable,
/// screen-clearing menu — confirmed directly against
/// `stories/CounterfeitMonkey-11.gblorb`: `HINT` prints
/// `" > Introduction to Counterfeit Monkey\n   Instructions for Play\n   \
/// Commands specific to this game\n   Testing Credits\n   Other Credits\n   \
/// Contacting the author\n   Hints\n"` with `erase_lower = true`, and a
/// `KeyInput::Down` reprints the SAME seven lines with only the cursor marker
/// moved onto "Instructions for Play" — six of seven lines identical, ~86%
/// overlap, comfortably over `is_screen_reprint`'s 50% threshold. This must
/// still COLLAPSE: the arrow press's reprint replaces the menu that was there,
/// it does not stack a second copy in scrollback.
#[test]
fn counterfeit_monkeys_real_hint_menu_still_collapses_on_arrow_navigation() {
    let story = fixture_path("CounterfeitMonkey-11.gblorb");
    if !story.is_file() {
        eprintln!("SKIP: no CounterfeitMonkey-11.gblorb");
        return;
    }
    let home = app::scratch_dir("sq1651-cm-menu");
    let mut b = boot_headless(story, &home);

    // Real typed commands through the accessibility gate — line input, never a
    // game-driven turn, so none of these touch `is_screen_reprint` (SQ-0403's
    // scrollback-preserving rule for command-turn clears is untouched by this
    // quest; see `finish_command_turn`).
    let mut tidy = 0u32;
    for cmd in ["yes", "andra", "", "tutorial off", "hint"] {
        let result = b.session.submit(cmd);
        let out = finish_command_turn(
            cmd, true, result, &mut b.state, &mut b.mapper, &mut *b.session,
            &b.game_dir, &b.ifid, &b.arc_file, None, &mut tidy,
        );
        assert!(!out.quit, "{cmd}: the game goes on");
    }
    assert!(
        b.state.transcript.iter().any(|l| l.trim() == "> Introduction to Counterfeit Monkey"),
        "premise: HINT lands on the real menu with the cursor on its first item: {:?}",
        b.state.transcript
    );
    assert_eq!(
        b.state.transcript.iter().filter(|l| l.trim() == "Testing Credits" || l.trim() == "> Testing Credits").count(),
        1,
        "premise: the menu's own items appear exactly once each before any navigation"
    );

    // Arrow-navigate: a real game-driven turn (char input), so THIS is the
    // path `is_screen_reprint` gates.
    let down_result = b.session.submit_key(KeyInput::Down).expect("the down arrow reaches the menu");
    assert!(down_result.erase_lower, "premise: the menu redraw clears the primary window on every arrow press");
    let out = apply_game_driven_result(&mut b.state, &mut b.mapper, &down_result, &b.game_dir, None, &*b.session, Driver::PlayerInput);
    assert!(!out.quit);

    assert!(
        !b.state.transcript.iter().any(|l| l.trim() == "> Introduction to Counterfeit Monkey"),
        "the old cursor position must be collapsed away, not stacked: {:?}",
        b.state.transcript
    );
    assert!(
        b.state.transcript.iter().any(|l| l.trim() == "> Instructions for Play"),
        "the new cursor position must be present: {:?}",
        b.state.transcript
    );
    assert_eq!(
        b.state.transcript.iter().filter(|l| l.trim() == "Testing Credits").count(),
        1,
        "an unmoved menu item ('Testing Credits') must appear exactly ONCE after the arrow \
         press, not twice — a second copy would mean the reprint stacked instead of collapsing, \
         which is the ORIGINAL SQ-0407 bug this gate must not reopen"
    );

    let _ = std::fs::remove_dir_all(&home);
}
