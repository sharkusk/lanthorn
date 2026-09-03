//! SQ-1259: Lost Pig (`LostPig.z8`, Inform 7) is played by Grunk, a named
//! orc — not by an Inform library `selfobj` the game actually uses — and two
//! independent bugs in `zvm::location` conspired to break both the automap
//! and the inventory panel on it:
//!
//! 1. `find_player_object` matched avatar CANDIDATES by short name only
//!    (`PLAYER_NAMES`), which finds #20 `(self object)` — present in every
//!    Inform 6/7 story, but never actually used by this game (its parent is
//!    0, unsituated) — and, being the only match, returned it unvalidated.
//!    The real avatar, #87 "Grunk", answers only to the parse word "me".
//! 2. `resolve_room_object` picked the LONGEST/lowest-numbered short-name
//!    match for the status line's "Outside", which is #18 "outside" — a
//!    child of #6 "compass" (a DIRECTION) — over #93 "Outside", the room
//!    itself, because both objects share the name and #18 sorts first.
//!
//! Together: the inventory panel read `(self object)`'s (empty) children, and
//! the automap tracked a compass direction instead of a room. This file pins
//! the real-game fix: room #93, player #87, and an inventory of exactly the
//! torch and pants Grunk starts the game carrying (the pants are worn but
//! still a child, per Inform's containment model).
//!
//! The story is gitignored, so this skips vacuously when absent.

use app::engine::Engine;
use app::session::{apply_turn, DeathWatch, GameSession};
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;

/// Boot `LostPig.z8` to the first line prompt. Lost Pig has no `read_char`
/// splash screens (unlike Anchorhead) — boot lands directly on a line prompt
/// — but the loop below tolerates one if a future release adds one.
fn boot_lostpig() -> Option<GameSession> {
    let path = fixture_path("LostPig.z8");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut session =
        GameSession::new_with_trace(bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)))
            .expect("LostPig.z8 should load and boot without a ZError");
    let mut n = 0;
    while session.pending_input() == app::session::InputKind::Char && n < 10 {
        let _ = session.submit_char(13);
        n += 1;
    }
    assert_eq!(session.pending_input(), app::session::InputKind::Line, "boot should reach a line prompt");
    Some(session)
}

fn lower(names: &[String]) -> Vec<String> {
    names.iter().map(|n| n.to_lowercase()).collect()
}

#[test]
fn lostpig_room_and_player_survive_a_move() {
    let Some(mut session) = boot_lostpig() else { return };

    // Two blank-ish turns, as the brief specifies: "" then "look". Lost Pig
    // answers "" with a "Huh?" (a real turn, not a re-prompt), so no third ""
    // is needed to clear a pending question.
    let _ = session.submit("");
    let _ = session.submit("look");

    // The room must be #93 "Outside" — the room object itself, parent 0 —
    // never #18, the same-named compass direction (parent #6 "compass").
    let room = session.current_location().expect("Lost Pig's opening room must be detected");
    assert_eq!(room.number, 93, "must resolve to the ROOM #93, not the compass direction #18");
    assert_eq!(room.name, "Outside");
    assert_eq!(room.parent, 0, "a room is top-level; the compass direction #18 is not");

    // The player must be #87 "Grunk" — found only through the parse word
    // "me" — never #20 "(self object)", present but unused (parent 0) here.
    let player = session
        .introspect()
        .and_then(|i| i.player_object())
        .expect("Lost Pig has an identifiable player object");
    assert_eq!(player, 87, "must resolve to Grunk (#87), not the unused (self object) (#20)");
    assert_eq!(zvm::objects::short_name(&session.machine.mem, player), "Grunk");
    assert_eq!(
        zvm::objects::get_parent(&session.machine.mem, player),
        93,
        "Grunk sits directly in the room the status line names"
    );

    // The inventory panel's source: exactly the torch and pants, both
    // children of Grunk — the pants are WORN but Inform keeps a worn item as
    // a child, so it must still appear.
    let carried: Vec<String> = session
        .introspect()
        .unwrap()
        .contents(player)
        .iter()
        .filter_map(|o| o.display_name())
        .collect();
    let carried_l = lower(&carried);
    assert_eq!(carried_l.len(), 2, "Grunk starts carrying exactly two things: {carried:?}");
    assert!(carried_l.iter().any(|n| n.contains("torch")), "carries the torch: {carried:?}");
    assert!(carried_l.iter().any(|n| n.contains("pants")), "carries the pants (worn, still a child): {carried:?}");

    // Perturb: walk north, and the room/player lock must survive the move —
    // not merely look right on the very frame the bug was introduced.
    let before_player = player;
    let north = session.submit("north");
    assert!(!north.quit && north.fault.is_none(), "\"north\" faulted/quit: {:?}", north.fault);

    let player_after = session.introspect().and_then(|i| i.player_object());
    assert_eq!(player_after, Some(before_player), "the avatar lock must survive a move");

    let carried_after: Vec<String> = session
        .introspect()
        .unwrap()
        .contents(before_player)
        .iter()
        .filter_map(|o| o.display_name())
        .collect();
    assert_eq!(
        lower(&carried_after).len(),
        2,
        "the torch and pants are still carried after moving: {carried_after:?}"
    );
}

// ── SQ-1259 follow-up: the gnome's privately-named room ─────────────────────
//
// Lost Pig's gnome sleeps in a room whose COMPILED short name is
// `(gnomeRoom)` — an Inform 7 "privately-named" object, printed only through
// its `printed name` property, which even changes mid-game: the same room
// (object #194) reads "Closet" on the status line before the player wakes
// the gnome, and "Gnome Room" after. No short-name search can ever find
// #194 by matching what the status line shows, because there is no short
// name that matches either string — and worse, "Gnome Room" is a
// word-boundary PREFIX match for an unrelated top-level object, #191
// "gnome" (the NPC himself), so the old code confidently reported the WRONG
// object as the room the instant the gnome woke.
//
// Reproduced from a fresh boot — no user save needed — with a prefix of the
// IF-Archive `walkthru.txt` sequence (the same one `declared_exit.rs`'s
// `LOST_PIG_WALKTHROUGH` on a sibling branch reconstructs; copied here
// rather than depending on that branch) up through the "SHOUT" that wakes
// the gnome.
const LOST_PIG_TO_THE_GNOME: &[&str] = &[
    "X ME", "INVENTORY", "X FARM", "X FOREST", "LOOK FOR PIG", "LISTEN", "NORTHEAST", "X STAIRS",
    "X METAL THING", "TAKE TUBE AND TORCH", "LOOK INSIDE TUBE", "BLOW IN TUBE", "X CRACK", "EAST",
    "X PIG", "FOLLOW PIG", "CATCH IT", "X FOUNTAIN", "X BOWL", "X COIN", "X CURTAIN", "X MAN",
    "NORTH", "X WEST MURAL", "X EAST MURAL", "X STATUE", "X HAT", "TAKE IT", "WEAR IT", "SOUTH",
    "SOUTHWEST", "X BOX", "PUT COIN IN SLOT", "PULL LEVER", "X BRICK", "TAKE IT", "SMELL IT",
    "TASTE IT", "EAT IT", "X DENT", "HIT BOX", "TAKE COIN", "PUT COIN IN SLOT", "PULL LEVER",
    "HIT BOX", "TAKE ALL FROM BASKET", "PUT COIN IN SLOT", "TAKE ALL FROM BASKET", "X CHAIR",
    "TAKE IT", "EAST", "X SHADOW", "LISTEN",
];

#[test]
fn lostpig_gnome_room_survives_its_own_mid_game_rename() {
    let Some(mut session) = boot_lostpig() else { return };
    let _ = session.submit("");

    let mut map = Mapper::default();
    let mut death = DeathWatch::default();
    for cmd in LOST_PIG_TO_THE_GNOME {
        let r = session.submit(cmd);
        apply_turn(&mut map, cmd, &r, &mut death);
    }

    // Standing in the closet, before the gnome wakes: object #194, printed
    // "Closet" — not a synthetic NameOnly room minted because nothing in
    // the tree matched "Closet" by short name.
    let closet = session.current_location().expect("standing in the closet");
    assert_eq!(closet.number, 194, "the closet is object #194 — the gnome's (as yet unnamed) room");
    assert_eq!(closet.name, "Closet");
    let closet_room_id = map.graph.current().expect("the closet is on the map");
    assert_eq!(
        map.graph.room(closet_room_id).map(|r| r.name.as_str()),
        Some("Closet"),
        "the map's room for #194 reads \"Closet\" before the rename"
    );

    let player = session.introspect().and_then(|i| i.player_object()).expect("Grunk is identifiable");
    let carried_before: Vec<String> =
        session.introspect().unwrap().contents(player).iter().filter_map(|o| o.display_name()).collect();

    // SHOUT wakes the gnome; the SAME room (#194) now prints as "Gnome Room".
    let shout = session.submit("SHOUT");
    apply_turn(&mut map, "SHOUT", &shout, &mut death);

    let gnome_room = session.current_location().expect("still standing in the gnome's room");
    assert_eq!(gnome_room.number, 194, "must resolve to the SAME room #194, not the NPC #191 \"gnome\"");
    assert_eq!(gnome_room.name, "Gnome Room", "named from the status line, not the compiled short name");
    assert_eq!(gnome_room.parent, 0, "a room is top-level; the NPC #191 the old prefix match picked is not this");

    // The map's room for #194 must have been RELABELLED in place, not
    // duplicated: same room id as the closet, edges intact.
    let same_room_id = map.graph.current().expect("still on the map after the rename");
    assert_eq!(same_room_id, closet_room_id, "the rename must relabel the existing room, not mint a new one");
    assert_eq!(
        map.graph.room(same_room_id).map(|r| r.name.as_str()),
        Some("Gnome Room"),
        "the map's label follows the rename"
    );

    // The player must still be identifiable, and carrying the same things —
    // this is the "map cannot track the player" half of the user's report.
    let player_after = session.introspect().and_then(|i| i.player_object());
    assert_eq!(player_after, Some(player), "the avatar lock must survive the rename");
    let carried_after: Vec<String> =
        session.introspect().unwrap().contents(player).iter().filter_map(|o| o.display_name()).collect();
    assert_eq!(carried_after, carried_before, "Grunk's inventory is unaffected by the room's rename");
}
