//! SQ-0526: same-named Glulx rooms must map as different rooms.
//!
//! Glulx exposes no object tree, so a room's identity was the hash of its printed
//! NAME — which makes Adventure's maze, where every room prints "Maze", a single
//! node however long you wander it. The user's 431-turn save holds exactly that:
//! one "Maze" room, one edge in, no self-loops.
//!
//! The identity does exist in the VM: Inform keeps the current room in its
//! `location` global and the value is the room's object address. Nothing says
//! where that global is — `advent.blb` supplies no `@accelparam` metadata — so it
//! is found by correlation, as the word whose changes coincide with the room
//! changing (`glulx_roomlock`).

use std::collections::BTreeSet;
use std::path::PathBuf;

use app::engine::Engine;
use app::glulx_session::GlulxSession;

use crate::fixture_paths::fixture_path;

fn advent_image() -> Option<Vec<u8>> {
    let p = fixture_path("advent.blb");
    let bytes = match std::fs::read(&p) {
        Ok(b) => b,
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", p.display());
            return None;
        }
    };
    let blorb = blorb::Blorb::parse(bytes).expect("advent.blb parses as a Blorb");
    let (_k, exec) = blorb.executable().expect("advent.blb carries an executable chunk");
    Some(exec.to_vec())
}

fn boot(image: Vec<u8>) -> GlulxSession {
    GlulxSession::new(image, 80, 24, true, false, false, (1, 1), None, &[]).expect("Adventure (Glulx) boots")
}

/// Walking Adventure's opening locks the `location` global, and distinct rooms
/// then carry distinct ids.
#[test]
fn walking_locks_the_location_global() {
    let Some(image) = advent_image() else { return };
    let mut s = boot(image);
    assert_eq!(s.locked_room_global(), None, "nothing is known at boot");

    // A mix of moves and non-moves: the heading-less turns are what separate the
    // room from a turn counter, which changes every turn regardless.
    for cmd in ["in", "take lamp", "wait", "down", "west", "west", "east"] {
        let _ = Engine::submit(&mut s, cmd);
    }
    let addr = s
        .locked_room_global()
        .expect("a few moves is enough evidence to identify the location global");

    // Distinct rooms, distinct ids — and the id is stable when we come back.
    let mut ids = Vec::new();
    for cmd in ["east", "west", "west", "east"] {
        let r = Engine::submit(&mut s, cmd);
        if let Some(l) = r.location {
            ids.push((l.name, l.number));
        }
    }
    let by_name: BTreeSet<&String> = ids.iter().map(|(n, _)| n).collect();
    let by_id: BTreeSet<mapper::graph::RoomId> = ids.iter().map(|(_, i)| *i).collect();
    assert_eq!(
        by_name.len(),
        by_id.len(),
        "distinct room names must still be distinct ids after the lock (addr 0x{addr:x}): {ids:?}"
    );
}

/// The bug itself: three arrivals in Adventure's maze all print the heading
/// "Maze" and must nevertheless be three rooms. Driven from the user's own
/// 431-turn save, which is parked in the maze — reaching it by walking would take
/// hundreds of moves.
#[test]
fn maze_rooms_sharing_a_name_get_distinct_ids() {
    let Some(image) = advent_image() else { return };
    let save = PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".lanthorn/saves/advent.blb.save/stuck-in-maze-no-debug.lanthorn");
    let Ok(ac) = app::archive::load_archive(&save) else {
        eprintln!("SKIP: maze fixture save missing at {}", save.display());
        return;
    };

    // Learn the address the way a real session does — by walking the opening,
    // where headings change and the evidence is unambiguous. Inside the maze every
    // heading repeats, so every turn is Ambiguous and carries no information: a
    // session that began in the maze could never learn, which is exactly why the
    // address is remembered per game and re-locked on resume.
    let mut learner = boot(image.clone());
    for cmd in ["in", "take lamp", "wait", "down", "west", "west", "east"] {
        let _ = Engine::submit(&mut learner, cmd);
    }
    let addr = learner.locked_room_global().expect("the opening walk identifies the global");

    let mut s = boot(image);
    Engine::restore_state(&mut s, &ac.engine_save()).expect("restore the maze save");
    s.relock_room_global(addr);
    let mut seen: Vec<(String, mapper::graph::RoomId)> = Vec::new();
    for cmd in ["look", "n", "s", "e", "w", "n", "e", "s", "w", "n", "u", "d", "n", "e"] {
        let r = Engine::submit(&mut s, cmd);
        if let Some(l) = r.location {
            seen.push((l.name, l.number));
        }
    }
    assert!(!seen.is_empty(), "the walk reported rooms");
    let names: BTreeSet<&String> = seen.iter().map(|(n, _)| n).collect();
    assert_eq!(
        names.len(),
        1,
        "this fixture is the maze: every heading is the same name, got {names:?}"
    );
    let ids: BTreeSet<mapper::graph::RoomId> = seen.iter().map(|(_, i)| *i).collect();
    assert!(
        ids.len() >= 3,
        "the maze rooms must come back as SEPARATE rooms despite the shared name — \
         got {} distinct id(s) across {} arrivals of {names:?}",
        ids.len(),
        seen.len()
    );
}

/// Room ids must be identical every time the story is loaded, or a saved map
/// would not match the game on reload — the ids are what the map is keyed by.
///
/// They are, because a Glulx game's objects live at fixed addresses in the story
/// image: RAM is initialised from that image, so the same room is at the same
/// address in every run. (A different RELEASE of a game is a different image and
/// legitimately a different map, exactly as it is for Z-machine object numbers.)
#[test]
fn room_ids_are_identical_across_separate_loads() {
    let Some(image) = advent_image() else { return };
    let route = ["in", "take lamp", "wait", "down", "west", "west", "east", "east"];

    let walk = |img: Vec<u8>| -> Vec<(String, mapper::graph::RoomId)> {
        let mut s = boot(img);
        let mut out = Vec::new();
        for cmd in route {
            if let Some(l) = Engine::submit(&mut s, cmd).location {
                out.push((l.name, l.number));
            }
        }
        out
    };

    let first = walk(image.clone());
    let second = walk(image);
    assert!(!first.is_empty(), "the route reported rooms");
    assert_eq!(
        first, second,
        "the same story walked the same way must yield the same room ids on every load"
    );
}
