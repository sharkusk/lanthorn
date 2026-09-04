//! SQ-0701: Anchorhead's automap must track rooms as the player walks.
//!
//! `anchor.z8` (Inform 6, library 6/7) parses perfectly at the status line —
//! `" Outside the Real Estate Office                      day one"` — and the
//! room name that comes off it was always right. What was wrong was the AVATAR.
//!
//! `detect_location`'s strongest rung validates the status-line name against the
//! object tree by walking up from a player object, and player objects are found
//! by short name. Anchorhead's real avatar is the Inform 6 library's `selfobj`,
//! whose compiled short name is `(self object)` — a name the candidate list did
//! not know. The one name it DID match was `yourself` #654: a conversation TOPIC
//! parked in `(con_topics)` #643 beside "Michael", "lighthouse" and "Danvers
//! Asylum". That topic never validates against any room, so detection fell
//! through to the stale-status-line rescue (`player_room_beside`), which walked
//! one step up from the topic and reported the topic BAG as the player's room —
//! the same object #643 `(con_topics)` in every room of the game.
//!
//! The mapper therefore recorded exactly one room, called "(con_topics)", and
//! never a single connection, no matter how far the player walked. That is the
//! reported symptom, and it is what this test pins.
//!
//! The story is gitignored, so this skips vacuously when absent.


use app::engine::Engine;
use app::session::{apply_turn, DeathWatch, GameSession, InputKind};
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;


/// Boot `anchor.z8` and tap through the three `read_char` intro screens (title
/// splash + epigraph, prologue, `* THE FIRST DAY *`) into play. Returns the
/// session parked at the first line prompt, in "Outside the Real Estate Office".
fn boot_anchor_into_play() -> Option<GameSession> {
    let path = fixture_path("anchor.z8");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut session =
        GameSession::new_with_trace(bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)))
            .expect("anchor.z8 should load and boot without a ZError");
    for _ in 0..6 {
        if session.pending_input() != InputKind::Char {
            break;
        }
        let _ = session.submit_char(13);
    }
    assert_eq!(session.pending_input(), InputKind::Line, "the intro should end at a line prompt");
    Some(session)
}

/// The room the mapper is standing in, as `(id, name)`.
fn here(m: &Mapper) -> Option<(mapper::graph::RoomId, String)> {
    let id = m.graph.current()?;
    Some((id, m.graph.room(id)?.name.clone()))
}

#[test]
fn anchor_automap_tracks_rooms_and_connections() {
    let Some(mut session) = boot_anchor_into_play() else { return };

    let mut map = Mapper::default();
    let mut death = DeathWatch::default();

    // The opening room, before any move. Detection must come off the OBJECT TREE
    // (the avatar's own parent), not off a name match — that is the difference
    // between the real room and the conversation-topic bag.
    let start = session.current_location().expect("Anchorhead's opening room must be detected");
    assert_eq!(
        start.name, "Outside the Real Estate Office",
        "the opening room is the one the status line names, not a topic container"
    );

    // Put the opening room on the map, then walk. Anchorhead's opening cul-de-sac
    // runs west into Narrow Street, and a twisting lane climbs northwest out of
    // that.
    let seed = session.submit("look");
    apply_turn(&mut map, "look", &seed, &mut death);
    assert_eq!(
        here(&map),
        Some((start.number, "Outside the Real Estate Office".to_string())),
        "the first room must reach the map"
    );
    assert_eq!(
        map.graph.room(start.number).and_then(|r| r.loc_method.clone()).as_deref(),
        Some("via player object"),
        "the room must be found by validating the avatar's parent, not by matching a bare name"
    );

    let west = session.submit("west");
    assert!(!west.quit && west.fault.is_none(), "\"west\" faulted/quit: {:?}", west.fault);
    assert!(
        west.transcript.contains("Narrow Street"),
        "\"west\" should reach Narrow Street: {:?}",
        west.transcript
    );
    apply_turn(&mut map, "west", &west, &mut death);
    let narrow = here(&map).expect("a room after moving west");
    assert_eq!(narrow.1, "Narrow Street", "the map must follow the player west");
    assert_ne!(narrow.0, start.number, "…to a DIFFERENT room id");

    let nw = session.submit("northwest");
    assert!(!nw.quit && nw.fault.is_none(), "\"northwest\" faulted/quit: {:?}", nw.fault);
    apply_turn(&mut map, "northwest", &nw, &mut death);
    let lane = here(&map).expect("a room after moving northwest");
    assert_eq!(lane.1, "Twisting Lane", "the map must follow the player northwest");

    // Three distinct rooms, and the two passages that join them — the whole point
    // of the automap, and exactly what a single stuck "(con_topics)" room
    // produced none of.
    let mut names: Vec<&str> = map.graph.rooms().map(|r| r.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["Narrow Street", "Outside the Real Estate Office", "Twisting Lane"],
        "every room walked through must be on the map, once each"
    );
    assert!(
        !names.iter().any(|n| n.contains("con_topics")),
        "a conversation-topic container must never be mapped as a room: {names:?}"
    );

    use mapper::direction::Direction;
    let conns: Vec<(mapper::graph::RoomId, Direction, mapper::graph::RoomId)> =
        map.graph.connections().iter().map(|c| (c.origin, c.dir, c.dest)).collect();
    assert!(
        conns.contains(&(start.number, Direction::W, narrow.0)),
        "the west passage from the cul-de-sac to Narrow Street must be recorded: {conns:?}"
    );
    assert!(
        conns.contains(&(narrow.0, Direction::NW, lane.0)),
        "the northwest passage from Narrow Street to Twisting Lane must be recorded: {conns:?}"
    );
}
