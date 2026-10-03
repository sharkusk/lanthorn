//! SQ-1710: `AppState::current_room_name` was set only by a normal turn, so after
//! any restore it kept the PRE-restore room until the next turn -- and a Save State
//! made right after the restore recorded that wrong room as its location.
//!
//! Each case perturbs before asserting (move, save, move, restore) and checks the
//! name immediately -- before any turn runs -- and then that a fresh save's summary
//! carries the restored room. Skips vacuously without the story fixture.

use app::engine::Engine;
use app::host::{boot_story, finish_command_turn, BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts};
use app::launch_options::LaunchOverrides;

use crate::fixture_paths::fixture_path;

fn boot(story: std::path::PathBuf, home: &std::path::Path) -> BootedStory {
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

fn play(b: &mut BootedStory, cmd: &str) {
    let result = b.session.submit(cmd);
    let mut tidy = 0u32;
    let _ = finish_command_turn(cmd, true, result, &mut b.state, &mut b.mapper, &mut *b.session, &b.game_dir, &b.ifid, &b.arc_file, None, &mut tidy);
}

fn engine_room(b: &BootedStory) -> String {
    b.session.current_location().expect("the engine reports a room").name
}

/// Save State to `path` and return the location its archive recorded.
fn save_and_read_location(b: &mut BootedStory, path: &std::path::Path) -> Option<String> {
    let out = app::host::persist::save_state_now(&mut *b.session, &b.mapper, &b.state, &b.ifid, path);
    assert!(matches!(out, app::host::persist::ExitSave::Saved), "premise: the save is written");
    app::archive::load_archive(path).expect("read back the save").meta.location
}

/// Move to room A, Save State, move to room B, Restore State: the name is A at once.
fn host_restore_round_trip(story: &str, to_a: &[&str], to_b: &[&str], tag: &str) {
    let path = fixture_path(story);
    if !path.is_file() {
        eprintln!("SKIP: no {story}");
        return;
    }
    let home = app::scratch_dir(tag);
    let mut b = boot(path, &home);
    for c in to_a {
        play(&mut b, c);
    }
    let room_a = engine_room(&b);
    assert_eq!(b.state.current_room_name.as_deref(), Some(room_a.as_str()), "premise: the turn path tracks room A");
    let slot = home.join("slot.lanthorn");
    save_and_read_location(&mut b, &slot);

    for c in to_b {
        play(&mut b, c);
    }
    let room_b = engine_room(&b);
    assert_ne!(room_a, room_b, "premise: the player really moved");
    assert_eq!(b.state.current_room_name.as_deref(), Some(room_b.as_str()), "premise: now tracking room B");

    app::host::persist::restore_file(&mut *b.session, &mut b.mapper, &mut b.state, &slot, None).expect("restore");
    assert_eq!(engine_room(&b), room_a, "premise: the engine is back in A");
    assert_eq!(b.state.current_room_name.as_deref(), Some(room_a.as_str()), "name is A right after the restore, before any turn");

    let again = home.join("again.lanthorn");
    assert_eq!(save_and_read_location(&mut b, &again).as_deref(), Some(room_a.as_str()), "a save made right after the restore records A");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn scott_host_restore_syncs_room_name() {
    host_restore_round_trip("adv01.dat", &["go swamp"], &["go stump"], "sq1710-scott");
}

#[test]
fn zmachine_host_restore_syncs_room_name() {
    host_restore_round_trip("zork1-r88-s840726.z3", &["north"], &["east"], "sq1710-zork");
}

#[test]
fn zmachine_ingame_restore_syncs_room_name() {
    use app::session::PendingIo;
    let path = fixture_path("zork1-r88-s840726.z3");
    if !path.is_file() {
        eprintln!("SKIP: no zork1-r88-s840726.z3");
        return;
    }
    let home = app::scratch_dir("sq1710-ingame");
    let mut b = boot(path, &home);
    play(&mut b, "north");
    let room_a = engine_room(&b);
    // The game's own @save, answered the way the host does.
    play(&mut b, "save");
    assert_eq!(b.state.ingame_io, Some(PendingIo::Save), "premise: SAVE reaches @save");
    let _ = app::host::ingame_io::handle_save_as("slot".into(), &home, &b.ifid, &mut b.mapper, &mut *b.session, &mut b.state, false);
    let _ = b.session.resume_save(true);
    b.state.ingame_io = None;

    play(&mut b, "east");
    let room_b = engine_room(&b);
    assert_ne!(room_a, room_b, "premise: moved");
    play(&mut b, "restore");
    assert_eq!(b.state.ingame_io, Some(PendingIo::Restore), "premise: RESTORE reaches @restore");

    let slot = home.join("slot.lanthorn");
    let loaded = app::host::persist::load_save(
        &mut *b.session, &mut b.mapper, &mut b.state,
        (&slot, "slot", app::archive::SaveTrigger::Ingame),
        &b.game_dir, &b.ifid, None,
    );
    assert!(loaded.answered_game, "premise: the pending @restore was answered");
    assert_eq!(engine_room(&b), room_a, "premise: the engine is back in A");
    assert_eq!(b.state.current_room_name.as_deref(), Some(room_a.as_str()), "name is A right after the in-game restore");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn reset_game_syncs_room_name_to_the_opening_room() {
    let path = fixture_path("zork1-r88-s840726.z3");
    if !path.is_file() {
        eprintln!("SKIP: no zork1-r88-s840726.z3");
        return;
    }
    let home = app::scratch_dir("sq1710-reset");
    let mut b = boot(path, &home);
    let opening = engine_room(&b);
    play(&mut b, "north");
    assert_ne!(b.state.current_room_name.as_deref(), Some(opening.as_str()), "premise: moved off the opening room");
    app::host::reset::reset_game(
        &mut *b.session, &mut b.mapper, &mut b.state, &b.story_bytes, &b.story_path, &b.game_dir, None,
        app::host::reset::ResetOptions::default(),
    );
    assert_eq!(b.state.current_room_name.as_deref(), Some(opening.as_str()), "name is the opening room right after the restart");
    let _ = std::fs::remove_dir_all(&home);
}
