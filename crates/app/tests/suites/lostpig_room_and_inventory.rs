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
use app::session::GameSession;

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
