//! The room a Glulx game BOOTS into must carry the same id a turn would give it.
//!
//! Glulx has no object tree, so a room's id is the hash of its printed name until the
//! `location` global is learned, and the room's own address afterwards (SQ-0526). The
//! address is remembered per story, so on every run after the first the lock is already
//! restored by the time the opening room is recorded — yet the boot path built that room
//! with a bare name hash while every subsequent turn used the address. Two ids for the
//! room you are standing in: the first action you took spawned a duplicate of the opening
//! room, disconnected from the map.


use app::engine::Engine;
use app::glulx_session::GlulxSession;

use crate::fixture_paths::fixture_path;

fn advent_image() -> Option<Vec<u8>> {
    let p = fixture_path("advent.blb");
    let Ok(bytes) = std::fs::read(&p) else {
        eprintln!("SKIP: gitignored story missing at {}", p.display());
        return None;
    };
    let blorb = blorb::Blorb::parse(bytes).expect("advent.blb parses as a Blorb");
    let (_k, exec) = blorb.executable().expect("advent.blb carries an executable chunk");
    Some(exec.to_vec())
}

fn boot_in(dir: &std::path::Path, image: Vec<u8>) -> GlulxSession {
    GlulxSession::new_in(
        dir.to_path_buf(),
        image,
        80,
        24,
        true,
        false,
        false,
        false,
        (8.0, 16.0),
        None,
        &[],
        Default::default(),
        false,
        None,
    )
    .expect("Adventure (Glulx) boots")
}

#[test]
fn the_boot_room_keeps_its_id_through_the_first_turn() {
    let Some(image) = advent_image() else { return };
    let dir = std::env::temp_dir().join(format!("lanthorn-boot-room-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch game dir");

    // First run: walk the opening so the learner identifies the `location` global and
    // persists it into this game dir — the state every run after the first starts from.
    {
        let mut learner = boot_in(&dir, image.clone());
        for cmd in ["in", "take lamp", "wait", "down", "west", "west", "east"] {
            let _ = Engine::submit(&mut learner, cmd);
        }
        assert!(learner.locked_room_global().is_some(), "the opening walk identifies the global");
    }
    assert!(dir.join("room-global").exists(), "and remembers it for the next run");

    // Second run — this is a fresh start, exactly like `reset-game`. The opening room the
    // boot records and the room the very first action reports must be the SAME node.
    let mut s = boot_in(&dir, image);
    let booted = s.current_location().expect("the boot records an opening room");
    let after = Engine::submit(&mut s, "look").location.expect("the first turn reports a room");
    assert_eq!(after.name, booted.name, "precondition: `look` does not move us");
    assert_eq!(
        after.number, booted.number,
        "the opening room must not change id on the first action — a second id for the room \
         you are standing in is a duplicate room on the map ({:?} booted as #{}, then #{})",
        booted.name, booted.number, after.number
    );

    let _ = std::fs::remove_dir_all(&dir);
}
