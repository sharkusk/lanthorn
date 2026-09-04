//! SQ-0724: the *Mysterious Adventures* ports must put rooms on the automap.
//!
//! Brian Howarth's eleven *Mysterious Adventures* ship here as v6 Z-machine
//! re-implementations of the Scott Adams engine, and they mapped nothing at all:
//! the player walked the whole game with an empty map. Three things about them
//! defeat every rung of `detect_location`'s v6 ladder at once —
//!
//!   1. The room text is not in the transcript. These games repaint a Scott-style
//!      block ("I'm in a dense SPOOKY Forest" / "Obvious exits: NORTH SOUTH" /
//!      "Visible items: …") into the upper window every turn, and the story
//!      buffer below it carries only the parser's replies. Anything watching the
//!      transcript sees nothing.
//!   2. The avatar is not in the object tree. `player` (object #5) has parent 0
//!      for the entire game, so the PlayerParent rung has nothing to walk.
//!   3. Every room object carries the same compiled short name, `ScottRoom`. The
//!      text the player reads lives in a PROPERTY, so matching the shown name
//!      against short names cannot name a room either.
//!
//! What the games do have is a global holding the current room's object number.
//! The new bottom rung takes it — but only after the object it names is shown to
//! carry, in one of its own properties, the very text the status band is painting
//! this turn. Object tree and screen corroborate each other every turn, and the
//! result is an exact room id: these games reuse a description across many rooms
//! (mysterious07 has ten rooms that all read "I'm in a Tunnel"), so a name-based
//! answer would fold every maze into a single node.
//!
//! The stories are gitignored, so each test skips vacuously when absent.


use app::engine::Engine;
use app::graphics::PictSource;
use app::session::{apply_turn, DeathWatch, GameSession, InputKind};
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;


/// Boot one Mysterious Adventures port and tap past its "Resume play on a game ?"
/// prompt into play. Returns the session parked at the first line prompt, in the
/// opening room.
fn boot_into_play(file: &str) -> Option<GameSession> {
    let path = fixture_path(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut s = GameSession::new_with_trace(bytes, true, false, None, false, dims, picts.std_window(), None, None)
        .expect("story should load and boot without a ZError");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    // "Resume play on a game ?" is a read_char; "n" starts a fresh game.
    for _ in 0..4 {
        if s.pending_input() != InputKind::Char {
            break;
        }
        let r = s.submit_char(b'n');
        assert!(r.fault.is_none(), "{file} faulted answering the resume prompt: {:?}", r.fault);
    }
    assert_eq!(s.pending_input(), InputKind::Line, "{file}: the intro should end at a line prompt");
    Some(s)
}

/// The room the mapper is standing in, as `(id, name)`.
fn here(m: &Mapper) -> Option<(mapper::graph::RoomId, String)> {
    let id = m.graph.current()?;
    Some((id, m.graph.room(id)?.name.clone()))
}

/// Whether any Mysterious Adventures fixture is present. `stories/` is gitignored
/// (the files are commercial), so CI and fresh worktrees legitimately have none.
/// The `ran > 0` guard in the sweep must not fire there.
fn any_mysterious_present() -> bool {
    (1..=11).any(|n| fixture_path(&format!("mysterious{n:02}.z6")).exists())
}

/// The reported bug, end to end: walk mysterious01 and watch the map fill in.
///
/// Before the fix this asserted at the very first line — `current_location` was
/// `None` on every turn of the game, so `apply_turn` recorded nothing and
/// `map.graph.rooms()` stayed empty forever.
#[test]
fn mysterious01_automap_tracks_rooms_and_connections() {
    let Some(mut session) = boot_into_play("mysterious01.z6") else { return };

    let mut map = Mapper::default();
    let mut death = DeathWatch::default();

    let start = session.current_location().expect("the opening room must be detected");
    assert_eq!(
        start.name, "I'm in a dense SPOOKY Forest",
        "the room takes its name from the text the game paints, not from the object's \
         short name (which is `ScottRoom` for every room in the game)"
    );

    let seed = session.submit("look");
    apply_turn(&mut map, "look", &seed, &mut death);
    assert_eq!(
        here(&map),
        Some((start.number, "I'm in a dense SPOOKY Forest".to_string())),
        "the opening room must reach the map"
    );

    // North twice, then back south twice. The Stream is a real second room and
    // the Path a third; coming back must land on the SAME ids, not on new ones.
    let north1 = session.submit("go north");
    assert!(north1.fault.is_none(), "\"go north\" faulted: {:?}", north1.fault);
    apply_turn(&mut map, "go north", &north1, &mut death);
    let stream = here(&map).expect("a room after going north");
    assert_eq!(stream.1, "I'm by a Stream", "the map must follow the player north");
    assert_ne!(stream.0, start.number, "…to a DIFFERENT room id");

    let north2 = session.submit("go north");
    apply_turn(&mut map, "go north", &north2, &mut death);
    let path = here(&map).expect("a room after going north again");
    assert_eq!(path.1, "I'm by a Path", "the map must follow the player north again");

    let south1 = session.submit("go south");
    apply_turn(&mut map, "go south", &south1, &mut death);
    assert_eq!(here(&map), Some(stream.clone()), "going back south must revisit the Stream, not mint a new room");

    let south2 = session.submit("go south");
    apply_turn(&mut map, "go south", &south2, &mut death);
    assert_eq!(
        here(&map),
        Some((start.number, "I'm in a dense SPOOKY Forest".to_string())),
        "going back south again must revisit the opening Forest"
    );

    let mut names: Vec<&str> = map.graph.rooms().map(|r| r.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["I'm by a Path", "I'm by a Stream", "I'm in a dense SPOOKY Forest"],
        "exactly the three rooms walked through, once each"
    );

    use mapper::direction::Direction;
    let conns: Vec<(mapper::graph::RoomId, Direction, mapper::graph::RoomId)> =
        map.graph.connections().iter().map(|c| (c.origin, c.dir, c.dest)).collect();
    assert!(
        conns.contains(&(start.number, Direction::N, stream.0)),
        "the north passage out of the Forest must be recorded: {conns:?}"
    );
    assert!(
        conns.contains(&(stream.0, Direction::N, path.0)),
        "the north passage from the Stream to the Path must be recorded: {conns:?}"
    );
}

/// The other ten titles are the same engine with different data, so the fix has
/// to work for all of them, not just the one that was reported. Each is booted
/// into play and must name its opening room off the painted band.
#[test]
fn every_mysterious_adventure_names_its_opening_room() {
    // (file, opening room text)
    let cases: [(&str, &str); 11] = [
        ("mysterious01.z6", "I'm in a dense SPOOKY Forest"),
        ("mysterious02.z6", "I'm in a dense Fog on the Moors"),
        ("mysterious03.z6", "I'm in a Courtyard"),
        ("mysterious04.z6", "I'm by a marsh"),
        ("mysterious05.z6", "I'm in the freighter's social room"),
        ("mysterious06.z6", "I'm in a field"),
        ("mysterious07.z6", "I feel a surge of strange Power"),
        ("mysterious08.z6", "I'm in the Throne-Room"),
        ("mysterious09.z6", "I'm in a Marble Hallway"),
        ("mysterious10.z6", "I'm in a Railway Carriage"),
        ("mysterious11.z6", "I'm in a Leisure Lounge"),
    ];
    let mut ran = 0;
    for (file, opening) in cases {
        let Some(mut session) = boot_into_play(file) else { continue };
        ran += 1;
        let loc = session.current_location().unwrap_or_else(|| panic!("{file}: no opening room detected"));
        assert_eq!(loc.name, opening, "{file}: wrong opening room");
        // A room id is only useful if it is a real object number.
        assert!(loc.number >= 1, "{file}: room id must be a real object number");

        // …and a turn of play must not fault. The room probe walks property
        // tables and decodes words that may not be strings at all, and a
        // speculative read landing outside the story file must answer "not a
        // string" rather than latch a memory fault that ends the session a turn
        // later (`Memory::without_fault_latch`).
        let r = session.submit("look");
        assert!(r.fault.is_none(), "{file}: a turn of play faulted: {:?}", r.fault);
        assert!(!r.quit, "{file}: the session ended during ordinary play");
        assert!(session.current_location().is_some(), "{file}: the room must still be detected after a turn");
    }
    assert!(ran > 0 || !any_mysterious_present(), "fixtures are present but none was exercised");
}
