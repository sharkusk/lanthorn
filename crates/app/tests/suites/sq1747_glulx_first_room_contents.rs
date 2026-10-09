//! SQ-1747: Glulx room contents and item sightings stay empty until the player first
//! changes room.
//!
//! # The report
//!
//! Toby's Nose (Inform 7 build 6M62): answer the prologue's prompts and the player stands in
//! the Drawing-Room, whose object (0x136b0f) holds the carpet, the sofa, the mantelpiece, the
//! decanter and more — yet `room_objects_excluding` returned nothing, because the room lock
//! forms only on a turn where the heading CHANGED, and nothing has changed yet.
//!
//! # Why it took two readers to fix
//!
//! 1. `gvm::i7map::I7World::detect` refused the story: its two rooms declare no exits, so the
//!    reciprocity scan for `Map_Storage` finds nothing. The room set is now recovered from the
//!    object tree instead (the one instance-count kind whose members are all parentless).
//! 2. The room lock could not use the name: the rooms' static `printed name` reads `DR`/`EP`
//!    while the story prints "Drawing-Room". `RoomLock::first_heading_witness` locks on the
//!    room set alone (every room-holding word agrees), with the name only as a veto.
//!
//! The fixture is gitignored (`stories/Toby's Nose.gblorb`), so this skips vacuously without it.
//! Route: the prologue (a line, a keypress, a keypress, then the first command prompt) and no
//! move — the assertions are about the OPENING room.

use app::engine::{Engine, Introspect, KeyInput};
use app::glulx_session::GlulxSession;
use app::session::InputKind;

use crate::fixture_paths::fixture_path;

fn boot() -> Option<GlulxSession> {
    let path = fixture_path("Toby's Nose.gblorb");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let pict_blorb = blorb::Blorb::parse(bytes.clone()).ok();
    let app::hints::LoadedStory::Glulx(image) =
        app::hints::extract_story(bytes).expect("Toby's Nose is a readable container")
    else {
        panic!("Toby's Nose is a Glulx story");
    };
    let mut s = GlulxSession::new(image, 80, 30, true, false, false, (8.0, 16.0), pict_blorb, &[])
        .expect("Toby's Nose boots");
    let _ = s.take_transcript();
    Some(s)
}

/// Answer the prologue (the tutorial question with "n", the key presses) until the story is
/// in the Drawing-Room, and return the session. No movement command is ever sent.
fn opening_room() -> Option<GlulxSession> {
    let mut s = boot()?;
    for _ in 0..6 {
        if s.current_location().is_some() {
            break;
        }
        match s.pending_input() {
            InputKind::Char => {
                s.submit_key(KeyInput::Enter).expect("Glulx takes keys");
            }
            _ => {
                s.submit("n");
            }
        }
    }
    Some(s)
}

#[test]
fn the_opening_room_has_its_contents_before_the_player_moves() {
    let Some(s) = opening_room() else { return };
    let here = s.current_location().expect("the opening room is known");
    assert_eq!(here.name, "Drawing-Room", "the label stays the heading the story printed, not the static `DR`");

    // Non-vacuity: the story's room set is readable, and the handle is the ROOM OBJECT's
    // identity rather than the hash of a heading.
    let world = s.i7_world().expect("a room-set-only world model is recovered for Toby's Nose");
    assert!(!world.has_map(), "no Map_Storage: this story's rooms declare no exits");
    assert!(
        world.rooms().iter().any(|&r| app::roomid::glulx_room_id(r) == here.number),
        "the location handle must be a room object's id, not a heading hash: {}",
        here.number
    );

    let words: Vec<Vec<String>> =
        s.room_objects_excluding(here.number, None).into_iter().map(|o| o.words).collect();
    assert!(!words.is_empty(), "the Drawing-Room's contents are known before any move");
    for thing in ["sofa", "carpet"] {
        assert!(
            words.iter().any(|w| w.iter().any(|x| x == thing)),
            "{thing:?} is in the Drawing-Room: {words:?}"
        );
    }
    // The I7 pronoun-group placeholders are not things in the room.
    for w in &words {
        for pronoun in ["he", "him", "she", "her"] {
            assert!(!w.iter().any(|x| x == pronoun), "a pronoun-group placeholder leaked: {w:?}");
        }
    }
}

/// SQ-1751: the contents are NAMED, not listed as runs of parse words, and the Inform 7
/// scaffolding (scent, kind, pronoun and analogy groups) the room text never mentions is gone.
#[test]
fn the_opening_room_lists_named_things_not_runs_of_parse_words() {
    let Some(s) = opening_room() else { return };
    let here = s.current_location().expect("the opening room is known");
    let objs = s.room_objects_excluding(here.number, None);
    let names: Vec<String> = objs.iter().filter_map(|o| o.display_name()).collect();
    for thing in ["carpet", "sofa", "door"] {
        assert!(names.iter().any(|n| n == thing), "{thing:?} is named in the Drawing-Room: {names:?}");
    }
    for n in &names {
        assert!(!n.contains(' '), "a space-joined synonym run was listed: {n:?} in {names:?}");
        for scaffold in ["dr-door", "persongro", "analogy", "smell scents"] {
            assert!(!n.contains(scaffold), "{scaffold:?} scaffolding leaked: {n:?}");
        }
    }
    // The kept objects still answer to every word the parser takes.
    assert!(objs.iter().any(|o| o.display_name().as_deref() == Some("sofa") && o.refers_to("couch")));
}
