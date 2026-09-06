//! SQ-1342: a clean, game-driven quit leaves no resume point.
//!
//! Before this fix, quitting a story from INSIDE the game (its own `@quit` /
//! `glk_exit`, or a Scott win/loss quit) left the same `default.lanthorn`
//! auto-save behind that a HOST-driven exit leaves — `main.rs`'s exit section
//! ran `lifecycle::exit_auto_save` unconditionally, and the per-turn auto-save
//! (`turn.rs`) had already written one for the quit turn itself. Reopening the
//! story then silently resumed from the turn before the player typed `quit`.
//!
//! `turn.rs`/`lifecycle.rs` are modules of the `lanthorn` BINARY crate, not the
//! `app` LIBRARY this suite links against, so the actual run-loop wiring
//! (`AppState::game_ended`, `lifecycle::exit_clear_resume_save`) is not
//! reachable from here — it is covered by in-crate tests alongside those
//! modules instead (`turn::tests::a_clean_quit_sets_game_ended_and_skips_its_own_per_turn_auto_save_sq1342`
//! and `lifecycle::tests::exit_clear_resume_save_empties_the_save_but_keeps_mapper_aux_and_command_history_sq1342`).
//! What IS public, and what this suite drives end to end on a real game, is the
//! archive-level contract those functions rest on: [`app::archive::write_cleared_resume_archive`]
//! — "the function [the fix] factored [the clearing write] into" — must leave
//! [`app::archive::ArchiveContents::save`] empty while keeping the mapper, aux
//! table, and command history; and the ORDINARY write it replaces (what
//! `exit_auto_save` and the per-turn auto-save both are, underneath) must go on
//! leaving a real, non-empty save when nothing calls the new function at all —
//! the guard that this quest's change did not widen.

use std::collections::BTreeMap;

use app::archive::{load_archive, save_archive_meta_pics, write_cleared_resume_archive, Meta, SaveTrigger, SessionRecord};
use app::engine::Engine;
use app::session::{apply_turn, DeathWatch, GameSession};
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;

/// Mini-Zork I r34/s871124 — the same tracked, sha256-verified fixture
/// `library_quit_resolution.rs` uses for its own clean-quit case, so this suite
/// never skips vacuously in CI or a fresh worktree with no `stories/`.
fn minizork() -> Vec<u8> {
    let path = fixture_path("minizork-r34-s871124.z3");
    std::fs::read(&path)
        .unwrap_or_else(|e| panic!("tracked fixture must be present at {}: {e}", path.display()))
}

fn seed_meta(ifid: &str, turns: u32) -> Meta {
    Meta {
        format_version: app::archive::CURRENT_FORMAT_VERSION,
        ifid: Some(ifid.to_string()),
        name: None,
        turns,
        saved_at: String::new(),
        location: None,
        score: None,
        trigger: SaveTrigger::HostState,
    }
}

#[test]
fn a_clean_quit_clears_the_save_but_keeps_the_mapper_and_command_history() {
    let dir = app::scratch_dir("sq1342-suite-clean-quit");
    let arc_file = dir.join("default.lanthorn");

    let mut session = GameSession::new(minizork(), true, false, None).expect("minizork boots without a ZError");
    let mut mapper = Mapper::default();
    let mut death = DeathWatch::default();

    // A turn or two, exactly what the per-turn auto-save (`turn.rs`) would have
    // driven the mapper with for them — real rooms, not a bare `Mapper::default()`.
    for cmd in ["look", "north"] {
        let result = session.submit(cmd);
        apply_turn(&mut mapper, cmd, &result, &mut death);
    }
    let rooms_before_quit = mapper.graph.rooms().count();
    assert!(rooms_before_quit >= 1, "the playthrough must have observed at least one room");

    // The per-turn auto-save this turn range would have written: a real,
    // non-empty, resumable save.
    save_archive_meta_pics(
        &arc_file, &mapper, &session.save_state(), None, &BTreeMap::new(), seed_meta("MINIZORK", 2),
        &SessionRecord::empty(), &[], None, None,
    ).expect("the per-turn auto-save write must succeed");
    let seeded = load_archive(&arc_file).expect("seeded archive readable");
    assert!(!seeded.save.is_empty(), "sanity: the per-turn auto-save left a resumable save");

    // Drive the story's OWN quit — mini-zork's `quit` verb asks "Do you wish to
    // leave the game? (Y is affirmative): " before it actually quits.
    let mut result = session.submit("quit");
    if !result.quit {
        result = session.submit("y");
    }
    assert!(result.quit, "mini-zork's quit (+ y confirmation) must end the game: {:?}", result.transcript);

    // "Runs the exit path (or the function you factored it into)": this is the
    // shared, public core `lifecycle::exit_clear_resume_save` (bin-private)
    // delegates to.
    let command_history =
        vec!["look".to_string(), "north".to_string(), "quit".to_string(), "y".to_string()];
    write_cleared_resume_archive(
        &arc_file, &mapper, &session.save_state(), session.aux_data(), "MINIZORK", "later".to_string(),
        &command_history,
    ).expect("the clearing write must succeed");

    let ac = load_archive(&arc_file).expect("cleared archive readable");
    assert!(ac.save.is_empty(), "a clean game quit must leave no resume point (SQ-1342)");
    assert!(ac.screen.is_none(), "no screen state after a clean quit");
    assert!(ac.transcript.is_empty(), "no transcript after a clean quit");
    assert!(ac.history.is_empty(), "no rewind/replay history after a clean quit");
    assert_eq!(
        ac.mapper.graph.rooms().count(), rooms_before_quit,
        "the mapper (the player's own knowledge) survives a clean quit"
    );
    assert_eq!(
        ac.command_history, command_history,
        "command history (shell-style recall) survives a clean quit"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The guard against widening: nothing about an ordinary (host-driven) exit
/// write goes through `write_cleared_resume_archive`, so a save it leaves
/// behind stays exactly what it always was — real and resumable. `game_ended`
/// itself is set only inside `turn.rs`'s two `should_exit_on_turn` call sites
/// (covered by the in-crate tests referenced in the module doc above); this
/// case pins the other half of the contract, that the archive format itself
/// does not silently produce an empty save from the ordinary write path.
#[test]
fn the_ordinary_write_path_never_empties_the_save_on_its_own() {
    let dir = app::scratch_dir("sq1342-suite-host-quit");
    let arc_file = dir.join("default.lanthorn");

    let mut session = GameSession::new(minizork(), true, false, None).expect("minizork boots without a ZError");
    let mapper = Mapper::default();
    let _ = session.submit("look");

    save_archive_meta_pics(
        &arc_file, &mapper, &session.save_state(), None, session.aux_data(), seed_meta("MINIZORK", 1),
        &SessionRecord::empty(), &[], None, None,
    ).expect("an ordinary auto-save write must succeed");

    let ac = load_archive(&arc_file).expect("archive readable");
    assert!(
        !ac.save.is_empty(),
        "a host-driven exit must still leave a resumable save — SQ-1342 must not widen this"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
