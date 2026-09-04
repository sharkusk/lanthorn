//! SQ-1301: *Spider and Web* really does define two rooms named "Interrogation
//! Chamber" (and two named "Diagonal Branch") — the mapper is not the one
//! splitting a single room in half.
//!
//! A player's `/export-map` dump showed `ROOM 44 "Interrogation Chamber"` and
//! `ROOM 263 "Interrogation Chamber"` side by side, plus a matching pair of
//! `"Diagonal Branch"` rooms (#175 and #298), and asked whether these were the
//! same room the automap had torn in two. lanthorn's Z-machine room identity is
//! the OBJECT NUMBER (`zvm::location::resolve_room_object`), so two different
//! numbers can only mean two different objects — and object #44 and object #263
//! (dumped from `stories/Tangle.z5`, release 4 / serial 980226, via
//! `zvm::objects::short_name`/`get_parent`) confirm it: both are real, distinct,
//! top-level (parent 0 — a room, not a container or a topic) objects, and both
//! carry the compiled short name `"Interrogation Chamber"`. Object #175 and
//! object #298 are the same shape for `"Diagonal Branch"`.
//!
//! This is not a coincidence of the story's numbering: *Spider and Web* tells
//! almost the whole game as a flashback the player-character is narrating to an
//! interrogator, and the interrogator periodically yanks the narration back to
//! the literal, physical Interrogation Chamber (object #44) mid-scene — and
//! later, per the reporter's dump (`random=[S→(#230 "Security Annex")]` on
//! #263), the story revisits the interrogation with a SECOND chamber object for
//! a later phase of the frame story. Driving the opening moves confirms the
//! mechanism directly: from "Mouth of Alley" (#94), typing `south` leaves the
//! alley and prints `-- glaring light... [Hit any key.]`; the keypress that
//! dismisses it reports the player's location as object #44, short name
//! "Interrogation Chamber" — a real room the object tree backs, not a status-line
//! fiction. The reverse happens on the interrogator's own cue: answering `yes`
//! prints `...glaring light --` and the next keypress drops the player back in
//! "End of Alley" (#91), a *different* real room than the one they left.
//!
//! The mapper keys every room by its Z-machine object number
//! (`mapper::graph::RoomId`), never by name, so two same-named objects were
//! always going to stay two rooms on the map — this test pins that down as a
//! straight documentation-by-test, for the next reporter who finds the same
//! dump and wonders the same thing.
//!
//! Skips vacuously without the gitignored `stories/Tangle.z5` fixture.

use crate::fixture_paths::fixture_path;

use app::engine::LocationInfo;
use app::session::{apply_turn, DeathWatch, GameSession, TurnResult};
use mapper::mapper::Mapper;
use zvm::objects::{get_parent, short_name};

const TANGLE: &str = "Tangle.z5";

/// Object #44: the Interrogation Chamber the game opens its frame story in.
/// `u16` — the Z-machine's own object-number width; cast to
/// [`mapper::graph::RoomId`] (`u32`) wherever the mapper wants one.
const INTERROGATION_CHAMBER_A: u16 = 44;
/// Object #263: the SAME short name, a distinct object entirely (SQ-1301).
const INTERROGATION_CHAMBER_B: u16 = 263;
/// Object #175: one of the two "Diagonal Branch" hallway objects.
const DIAGONAL_BRANCH_A: u16 = 175;
/// Object #298: the other one.
const DIAGONAL_BRANCH_B: u16 = 298;

fn boot() -> Option<GameSession> {
    let path = fixture_path(TANGLE);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    // Fixture identity, named per CLAUDE.md's testing conventions: release 4 /
    // serial 980226 is *Spider and Web*, not some other disk/version of it.
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 4, "Tangle.z5 should be release 4");
    let serial: String = bytes[0x12..0x18].iter().map(|&b| b as char).collect();
    assert_eq!(serial, "980226", "Tangle.z5 should be serial 980226");
    let session =
        GameSession::new_with_trace(bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)))
            .expect("Tangle.z5 should load and boot without a ZError");
    Some(session)
}

#[test]
fn spider_and_web_twin_interrogation_chambers_are_distinct_objects() {
    let Some(session) = boot() else { return };
    let mem = &session.machine.mem;

    assert_eq!(
        short_name(mem, INTERROGATION_CHAMBER_A),
        "Interrogation Chamber",
        "object #44 must carry the short name the dump reported"
    );
    assert_eq!(
        short_name(mem, INTERROGATION_CHAMBER_B),
        "Interrogation Chamber",
        "object #263 carries the SAME short name — that is the whole point"
    );
    assert_ne!(
        INTERROGATION_CHAMBER_A, INTERROGATION_CHAMBER_B,
        "same name, but never the same object"
    );
    // A room is a top-level object (parent 0), never a container or a topic bag
    // — the same discrimination `resolve_room_object` itself applies.
    assert_eq!(get_parent(mem, INTERROGATION_CHAMBER_A), 0, "#44 is top-level, i.e. a room");
    assert_eq!(get_parent(mem, INTERROGATION_CHAMBER_B), 0, "#263 is top-level, i.e. a room");

    // The reporter's second pair, same shape: "Diagonal Branch" is two objects too.
    assert_eq!(short_name(mem, DIAGONAL_BRANCH_A), "Diagonal Branch");
    assert_eq!(short_name(mem, DIAGONAL_BRANCH_B), "Diagonal Branch");
    assert_ne!(DIAGONAL_BRANCH_A, DIAGONAL_BRANCH_B);
    assert_eq!(get_parent(mem, DIAGONAL_BRANCH_A), 0);
    assert_eq!(get_parent(mem, DIAGONAL_BRANCH_B), 0);
}

#[test]
fn mapper_keeps_the_twin_interrogation_chambers_apart() {
    // The mapper never has to tell #44 and #263 apart by NAME — it keys every
    // room by its Z-machine object number — so observing both, in either order,
    // must leave two separate rooms on the map, both correctly labelled.
    let mut map = Mapper::default();
    let mut death = DeathWatch::default();

    let seed_a = TurnResult::observation(LocationInfo {
        number: INTERROGATION_CHAMBER_A as mapper::graph::RoomId,
        parent: 0,
        name: "Interrogation Chamber".to_string(),
    });
    apply_turn(&mut map, "", &seed_a, &mut death);
    assert_eq!(map.graph.current(), Some(INTERROGATION_CHAMBER_A as mapper::graph::RoomId));
    assert_eq!(map.graph.rooms().count(), 1);

    // An involuntary relocation (exactly how the interrogator's cut actually
    // lands the player) is the honest shape here, not a walked direction — see
    // the module doc comment for the real transcript. Either way the mapper
    // must not collapse the two same-named objects into one room.
    map.observe_relocation(INTERROGATION_CHAMBER_B as mapper::graph::RoomId, "Interrogation Chamber");
    assert_eq!(map.graph.current(), Some(INTERROGATION_CHAMBER_B as mapper::graph::RoomId));
    assert_eq!(
        map.graph.rooms().count(),
        2,
        "two distinct objects sharing a short name must stay two rooms, not collapse into one"
    );

    let room_a =
        map.graph.room(INTERROGATION_CHAMBER_A as mapper::graph::RoomId).expect("#44 is still on the map");
    let room_b =
        map.graph.room(INTERROGATION_CHAMBER_B as mapper::graph::RoomId).expect("#263 is on the map too");
    assert_eq!(room_a.label(), "Interrogation Chamber");
    assert_eq!(room_b.label(), "Interrogation Chamber");
}
