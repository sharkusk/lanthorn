//! SQ-0752 / SQ-0731: a room the object tree cannot back must still reach the map.
//!
//! Four titles, three failure reports and one open quest all turned out to be the
//! same bug, and it was not in `detect_location` — that function named every one of
//! these rooms correctly, every turn. The value died one layer later, in
//! `apply_turn`, which suppressed an unvalidated `NameOnly` room until the map held
//! an OBJECT-BACKED one. That rule presumed every story eventually produces such a
//! room. These do not, so the gate was never a delay but a permanent mute, and
//! their maps stayed empty for the whole game:
//!
//!   * `the-impossible-bottle.zblorb.blorb` — compiled by Dialog, whose 492 objects
//!     carry no short names AT ALL, so no room can ever be object-backed (SQ-0752).
//!   * `frankenfingers_260330.z5` — the player object is correctly parented to the
//!     room, but the room's short name is the compiler's identifier `partsRoom`
//!     while the screen says "Parts Room", so the name match never lands.
//!   * `Facility.z8` — no object anywhere in the tree is named for a room (SQ-0731).
//!   * `ImpossibleStairs.z8` — Dialog again, and additionally a status line whose
//!     room is a LABELLED FIELD: " Year: 2001  Place: Front Lawn".
//!
//! The replacement rule is corroboration: the map's first room, when nothing backs
//! it, must be one the STORY ITSELF named — a name printed as its own heading line
//! as well as painted on the status line. A pre-game banner or character sheet is
//! named once, on the status line only, which is what `beyond_zork_character_sheet_
//! is_not_the_first_room` pins with the very game the old gate was written for.
//!
//! The stories are gitignored, so every test here skips vacuously when absent.

use std::path::PathBuf;

use app::engine::Engine;
use app::session::{apply_turn, DeathWatch, GameSession, InputKind, TurnResult};
use mapper::mapper::Mapper;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// The Z-code image inside a story file, unwrapping a Blorb. `None` when the
/// gitignored fixture is absent (or is not a Z-machine story).
fn zcode(name: &str) -> Option<Vec<u8>> {
    let path = stories_dir().join(name);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    if !blorb::Blorb::is_blorb(&bytes) {
        return Some(bytes);
    }
    match blorb::Blorb::parse(bytes).ok()?.executable() {
        Ok((blorb::ExecKind::ZCode, data)) => Some(data.to_vec()),
        _ => None,
    }
}

fn boot(name: &str) -> Option<GameSession> {
    let bytes = zcode(name)?;
    let mut s = GameSession::new_with_trace(
        bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)),
    )
    .unwrap_or_else(|e| panic!("{name} should boot without a ZError: {e:?}"));
    // The app consumes the opening banner before the first command (`startup.rs`),
    // so a turn's transcript is that turn's own output. Match that here, or the
    // banner rides along on turn one and changes what the heading test sees.
    let _ = s.take_transcript();
    Some(s)
}

/// A `TurnResult` carrying nothing but a location, as `startup.rs` builds to seed
/// the opening room (note `location_method: None` — the seed is not gated).
fn seed_of(snap: zvm::ObjectSnapshot) -> TurnResult {
    TurnResult {
        transcript: String::new(),
        transcript_runs: Vec::new(),
        location: Some(snap),
        quit: false,
        erase_lower: false,
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
    }
}

/// Drive `cmds` through a booted session into a fresh map, seeding the opening
/// room first exactly as the app does. Returns the rooms the mapper recorded.
fn walk(name: &str, cmds: &[&str]) -> Option<Vec<String>> {
    let mut s = boot(name)?;
    let mut map = Mapper::default();
    let mut death = DeathWatch::default();
    if let Some(snap) = s.current_location() {
        apply_turn(&mut map, "", &seed_of(snap), &mut death);
    }
    for cmd in cmds {
        for _ in 0..6 {
            if s.pending_input() != InputKind::Char {
                break;
            }
            let _ = s.submit_char(13);
        }
        if s.pending_input() != InputKind::Line {
            break;
        }
        let r = s.submit(cmd);
        assert!(r.fault.is_none(), "{name} faulted on {cmd:?}: {:?}", r.fault);
        apply_turn(&mut map, cmd, &r, &mut death);
    }
    let mut rooms: Vec<String> = map.graph.rooms().map(|r| r.name.clone()).collect();
    rooms.sort();
    Some(rooms)
}

#[test]
fn impossible_bottle_maps_the_kitchen_it_starts_in() {
    // The Bottle opens on a conversation menu; "1" repeatedly answers Dad until
    // play begins, in the Kitchen. Every object in this Dialog story is nameless,
    // so "Kitchen" can only ever be a NameOnly room — the map was empty before.
    let Some(rooms) =
        walk("the-impossible-bottle.zblorb.blorb", &["1", "1", "1", "1", "1", "look", "south"])
    else {
        return;
    };
    assert!(
        rooms.contains(&"Kitchen".to_string()),
        "the Bottle's opening room must reach the map; got {rooms:?}"
    );
    assert!(
        rooms.contains(&"Smooth surface".to_string()),
        "walking south out of the Kitchen must map the room it leads to; got {rooms:?}"
    );
}

#[test]
fn frankenfingers_maps_the_room_its_status_bar_shows() {
    // Reported as "shows Parts Room in the upper left corner of the status bar,
    // but nothing shows up on the map". The room object exists and even parents
    // the player, but it is called `partsRoom`, so only the screen names it.
    let Some(rooms) = walk("frankenfingers_260330.z5", &["look"]) else { return };
    assert_eq!(
        rooms,
        vec!["Parts Room".to_string()],
        "the room on the status bar must be the room on the map"
    );
}

#[test]
fn facility_maps_rooms_as_the_player_walks() {
    // SQ-0731. Nothing in Facility's object tree is named for a room, so every
    // room here is NameOnly for the whole game.
    let Some(rooms) = walk("Facility.z8", &["look", "north", "south"]) else { return };
    assert!(
        rooms.contains(&"Lobby".to_string()) && rooms.contains(&"Visitor Gallery".to_string()),
        "walking north out of the Lobby must map both rooms; got {rooms:?}"
    );
}

#[test]
fn impossible_stairs_maps_the_place_not_the_whole_status_line() {
    // " Year: 2001  Place: Front Lawn" — taking the left half whole makes the year
    // part of the room's identity, so every year the story turns mints a brand new
    // room for a place the player never left.
    let Some(rooms) = walk("ImpossibleStairs.z8", &["take airplane", "east", "east"]) else {
        return;
    };
    assert!(
        rooms.contains(&"Front Lawn".to_string()),
        "the opening room is the Place field, not the whole status line; got {rooms:?}"
    );
    assert!(
        rooms.contains(&"Family Room".to_string()) && rooms.contains(&"Kitchen".to_string()),
        "walking into the house must map the rooms it leads to; got {rooms:?}"
    );
    assert!(
        !rooms.iter().any(|r| r.contains("Year")),
        "no room may be named for the status line's other fields; got {rooms:?}"
    );
}

#[test]
fn beyond_zork_character_sheet_is_not_the_first_room() {
    // The case the old gate was written for, and the one the new rule has to keep
    // rejecting. Beyond Zork's character sheet paints " Frank Booth … Level 0 Male
    // Peasant" — a room-shaped status line — while the story text says only "Press
    // any key to begin the story." The player's NAME is not a room, and the story
    // never prints it as a heading, so corroboration turns it away; "Hilltop", one
    // turn later, is named twice and is.
    let Some(mut s) = boot("beyondzork-r57-s871221.z5") else { return };
    let mut map = Mapper::default();
    let mut death = DeathWatch::default();
    let mut saw_character_sheet = false;
    // "yes" (VT220), "begin", then RETURNs through character creation into play.
    for cmd in ["yes", "begin", "", "", "", "", "", ""] {
        let r = match s.pending_input() {
            InputKind::Char => s.submit_char(13),
            InputKind::Line => s.submit(cmd),
            _ => break,
        };
        let sheet = r.location.as_deref_name() == Some("Frank Booth");
        apply_turn(&mut map, cmd, &r, &mut death);
        if sheet {
            saw_character_sheet = true;
            assert_eq!(
                map.graph.rooms().count(),
                0,
                "the character sheet must not seed the map — the player's name is not a room"
            );
        }
    }
    assert!(saw_character_sheet, "the probe never reached the character sheet");
    let rooms: Vec<String> = map.graph.rooms().map(|r| r.name.clone()).collect();
    assert_eq!(rooms, vec!["Hilltop".to_string()], "the first real room must still be mapped");
}

/// Tiny helper so the assertion above reads as one line.
trait LocName {
    fn as_deref_name(&self) -> Option<&str>;
}
impl LocName for Option<zvm::ObjectSnapshot> {
    fn as_deref_name(&self) -> Option<&str> {
        self.as_ref().map(|s| s.name.as_str())
    }
}
