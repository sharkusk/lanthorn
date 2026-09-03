//! Declared-exit derivation (SQ-1257): what a room's own compiled exit table
//! says for a direction, read independently of anything ever walked, and how
//! that seam changes what the mapper records.
//!
//! Real-game cases skip vacuously without `stories/` (gitignored), the
//! CI-safe pattern documented in `crates/app/tests/suites/fixture_paths.rs`.

use crate::fixture_paths::fixture_path;

use app::engine::{DeclaredExit, Engine};
use app::session::{apply_turn, DeathWatch, GameSession, TurnResult};
use mapper::direction::Direction;
use mapper::mapper::Mapper;

fn story(name: &str) -> Option<Vec<u8>> {
    std::fs::read(fixture_path(name)).ok()
}

fn boot(bytes: Vec<u8>) -> GameSession {
    let mut s = GameSession::new_with_trace(
        bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)),
    )
    .expect("story boots without a ZError");
    s.set_strip_prompt(false);
    s
}

// ── Curses (Inform 6): the derivation must agree with a real move ──────────

/// Curses r18/s941124 opens in the Attic. A declared exit read off the
/// starting room, for a direction the game's own text names, must name the
/// same room the player actually lands in after walking it.
#[test]
fn curses_declared_exit_matches_the_real_move() {
    let Some(bytes) = story("curses.z5") else {
        eprintln!("SKIP: gitignored stories/curses.z5 missing");
        return;
    };
    let mut s = boot(bytes);
    let r0 = s.submit("look");
    let start = r0.location.clone().expect("Curses names a starting room");

    // Try every compass direction the derivation can name; Curses' Attic has
    // at least one real exit among them (verified below by requiring at least
    // one `Room(_)` match this asserts on).
    let mut matched_any = false;
    for (word, dir) in [
        ("north", Direction::N), ("south", Direction::S),
        ("east", Direction::E), ("west", Direction::W),
        ("up", Direction::Up), ("down", Direction::Down),
        ("in", Direction::In), ("out", Direction::Out),
    ] {
        let declared = s.declared_exit(start.number, dir);
        let DeclaredExit::Room(declared_dest) = declared else { continue };

        // Walk it in a fresh boot so trying several directions never compounds moves.
        let bytes2 = story("curses.z5").unwrap();
        let mut s2 = boot(bytes2);
        let _ = s2.submit("look");
        let after = s2.submit(word);
        let Some(arrived) = after.location else { continue };
        if arrived.number == start.number {
            continue; // refused or a non-passage word; not evidence either way
        }
        matched_any = true;
        assert_eq!(
            arrived.number, declared_dest,
            "declared {word} exit of room {} said {declared_dest}, but walking {word} landed in {} ({})",
            start.number, arrived.number, arrived.name
        );
    }
    assert!(matched_any, "expected at least one of Curses' opening room's exits to be a plain declared Room(_) that a real move confirms");
}

// ── Zork I (ZIL, not Inform): the seam must answer Unknown ─────────────────

/// Zork I is not Inform-compiled, so `door_dir`/`*_to` do not exist in its
/// table and the derivation must find nothing to read: every direction comes
/// back `Unknown`, and `apply_turn` behaves exactly as it always has — a real
/// move still mints its edge (West of House → north → North of House).
#[test]
fn zork1_seam_is_unknown_and_the_move_still_mints_its_edge() {
    let Some(bytes) = story("zork1-r88-s840726.z3") else {
        eprintln!("SKIP: gitignored stories/zork1-r88-s840726.z3 missing");
        return;
    };
    let s = boot(bytes);
    let start = s.current_location().expect("Zork I names West of House at boot");

    for dir in [Direction::N, Direction::S, Direction::E, Direction::W] {
        assert_eq!(
            s.declared_exit(start.number, dir),
            DeclaredExit::Unknown,
            "Zork I is ZIL, not Inform — {dir:?} must have no door_dir data to read"
        );
    }

    // And the ordinary path still mints the edge exactly as before this seam existed.
    let mut s = s;
    let mut mapper = Mapper::default();
    let mut death = DeathWatch::default();
    let seed = s.submit("look");
    apply_turn(&mut mapper, "look", &seed, &mut death);
    let west = mapper.graph.current().expect("West of House seeded");

    let r = s.submit("north");
    apply_turn(&mut mapper, "north", &r, &mut death);
    let north = mapper.graph.current().expect("North of House");
    assert_ne!(north, west);
    assert_eq!(
        mapper.graph.connections().iter().find(|c| c.origin == west && c.dir == Direction::N).map(|c| c.dest),
        Some(north),
        "an Unknown declared exit must not stop the ordinary edge from being minted"
    );
}

// ── Falsify: reverting the app-side decision reproduces the original symptom ──

/// With the `declared_exit`-driven branch in `apply_turn` disabled (simulated
/// here by never populating `TurnResult::declared_exit`, exactly what every
/// pre-SQ-1257 caller did), a synthetic random-destination move mints a
/// normal edge — the bug this quest exists to fix. `synthetic_random_exit_*`
/// below is the same scenario WITH the field populated, and asserts the
/// opposite.
#[test]
fn falsify_without_declared_exit_a_random_move_mints_a_normal_edge() {
    let mut mapper = Mapper::default();
    let mut death = DeathWatch::default();
    apply_turn(&mut mapper, "", &TurnResult::observation(snap(1, "Tunnel")), &mut death);

    // No `declared_exit` populated (the pre-SQ-1257 shape of every TurnResult).
    let mut r = TurnResult::observation(snap(2, "Other Tunnel"));
    r.declared_exit = None;
    apply_turn(&mut mapper, "north", &r, &mut death);

    assert!(
        mapper.graph.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::N && c.dest == 2),
        "the original symptom: an edge minted for a move the declared-exit seam was never asked about"
    );
}

/// The mirror of the falsify case above: WITH `declared_exit` populated as a
/// mismatching `Room(_)`, the same shape of move records NO edge and instead
/// marks the origin's direction as a random exit — [`mapper::matrix::classify`]
/// then reads `?`, not `⇢`.
#[test]
fn synthetic_random_exit_records_no_edge_and_marks_the_cell_random() {
    let mut mapper = Mapper::default();
    let mut death = DeathWatch::default();
    apply_turn(&mut mapper, "", &TurnResult::observation(snap(1, "Tunnel")), &mut death);

    let mut r = TurnResult::observation(snap(2, "Other Tunnel"));
    r.declared_exit = Some(DeclaredExit::Room(3)); // declared a THIRD room, not 2
    apply_turn(&mut mapper, "north", &r, &mut death);

    assert!(
        !mapper.graph.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::N),
        "no edge north out of room 1"
    );
    assert_eq!(mapper.graph.current(), Some(2), "the player is still observed as having arrived in 2");
    assert_eq!(
        mapper::matrix::classify(&mapper.graph, 1, Direction::N),
        mapper::matrix::MatrixCell::Random,
        "the matrix must read this cell as `?`, not untried/probed"
    );
}

fn snap(number: u16, name: &str) -> zvm::ObjectSnapshot {
    zvm::ObjectSnapshot { number, parent: 0, name: name.to_string() }
}

// ── Lost Pig: the gateway into the gnome's random tunnels ──────────────────
//
// Lost Pig's maze rooms carry NO declared exit data at all for any direction
// (every `*_to` property is simply absent) — the "before going" rule that
// randomises the destination fires before the library's own exit-table code
// ever runs, so there is nothing here to compare against. What Phase 1 CAN
// and does read correctly is the GATEWAY: Statue Room's north exit (opened by
// putting the lit torch in the statue's hand) and Windy Cave's north exit
// both hold a ROUTINE, not a room — `DeclaredExit::Code` — which is the
// honest "cannot say" answer for a destination the story computes at run
// time, and is what stops this derivation from ever mislabelling a doorway it
// cannot resolve as a fixed room. Catching the maze itself needs Phase 2
// (see the module docs / SQ-1257 report); this suite proves Phase 1 is sound
// on the real game up to the door it cannot see through.
//
// The command sequence below is a verified path — 128 commands, the last of
// which (`PUT TORCH IN HAND`) opens the secret door — from `LostPig.z8`'s
// boot to the Statue Room's north wall opening. Reconstructed by driving the
// game headless and reading its own replies turn by turn, starting from the
// published `walkthru.txt` on the IF-Archive
// (if-archive/games/competition2007/zcode/lostpig/walkthru.txt, written by
// the game's author) and confirming every room name and object number
// against this build directly rather than trusting the transcript's prose.

const LOST_PIG_WALKTHROUGH: &[&str] = &[
    "X ME", "INVENTORY", "X FARM", "X FOREST", "LOOK FOR PIG", "LISTEN", "NORTHEAST", "X STAIRS",
    "X METAL THING", "TAKE TUBE AND TORCH", "LOOK INSIDE TUBE", "BLOW IN TUBE", "X CRACK", "EAST",
    "X PIG", "FOLLOW PIG", "CATCH IT", "X FOUNTAIN", "X BOWL", "X COIN", "X CURTAIN", "X MAN",
    "NORTH", "X WEST MURAL", "X EAST MURAL", "X STATUE", "X HAT", "TAKE IT", "WEAR IT", "SOUTH",
    "SOUTHWEST", "X BOX", "PUT COIN IN SLOT", "PULL LEVER", "X BRICK", "TAKE IT", "SMELL IT",
    "TASTE IT", "EAT IT", "X DENT", "HIT BOX", "TAKE COIN", "PUT COIN IN SLOT", "PULL LEVER",
    "HIT BOX", "TAKE ALL FROM BASKET", "PUT COIN IN SLOT", "TAKE ALL FROM BASKET", "X CHAIR",
    "TAKE IT", "EAST", "X SHADOW", "LISTEN", "SHOUT", "GREET GNOME", "TELL GNOME ABOUT GRUNK",
    "ASK GNOME ABOUT STATUE", "ASK WHAT GNOME LOOKING FOR", "LOOK UNDER BED",
    "TALK TO GNOME ABOUT MOGGLEV", "LOOK", "LOOK UNDER BED", "OPEN TRUNK", "X BALL", "TAKE BALL",
    "SHOW TORCH TO GNOME", "ASK GNOME ABOUT FIRE", "SHOW BRICK TO GNOME", "ASK GNOME ABOUT MOTHER",
    "EAST", "X SHELF", "X TOP SHELF", "DROP CHAIR", "STAND ON CHAIR", "X TOP SHELF", "TAKE BOOK",
    "X IT", "GET DOWN", "OPEN CHEST", "TAKE POLE", "X IT", "WEST", "SHOW POLE TO GNOME",
    "ASK GNOME ABOUT COLOR MAGNET", "SHOW BOOK TO GNOME", "GIVE BOOK TO GNOME", "EAST",
    "ASK GNOME ABOUT PAGE", "EAST", "NORTHWEST", "EAST", "X RIVER", "X THING", "TAKE THING",
    "CROSS RIVER", "TOUCH THING WITH POLE", "X KEY", "TAKE WATER", "FILL HAT WITH WATER", "WEST",
    "SOUTHEAST", "UNLOCK CHEST", "OPEN IT", "POUR WATER ON POWDER", "LIGHT TORCH WITH FIRE",
    "NORTHWEST", "WEST", "X CRACK", "TAKE PAPER", "TAKE PAPER WITH POLE", "BURN POLE WITH TORCH",
    "TAKE PAPER WITH POLE", "EAST", "SOUTHWEST", "EAST", "GIVE PAPER TO GNOME", "WAIT",
    "GO TO PIG", "SHOW BRICK TO PIG", "DROP ALL BRICKS", "Z", "Z", "Z", "Z", "TAKE PIG",
    "GO TO STATUE", "X HAND", "PUT TORCH IN HAND",
];

/// Boots Lost Pig and drives the 128-command opening (see
/// [`LOST_PIG_WALKTHROUGH`]) up to the moment the secret door opens — the
/// non-vacuity guard is the door's own printed line, so a harness that never
/// reaches it fails loudly rather than passing vacuously.
#[test]
fn lost_pig_gateway_into_the_tunnels_reads_as_code_not_a_guessed_room() {
    let Some(bytes) = story("LostPig.z8") else {
        eprintln!("SKIP: gitignored stories/LostPig.z8 missing");
        return;
    };
    let mut s = boot(bytes);
    let _ = s.submit("");
    let mut opened = false;
    for cmd in LOST_PIG_WALKTHROUGH {
        let r = s.submit(cmd);
        if r.transcript.contains("part of north wall open up") {
            opened = true;
        }
    }
    assert!(opened, "non-vacuity guard: the walkthrough must reach the secret door opening");

    let statue_room = s.current_location().expect("standing in the Statue Room").number;
    assert_eq!(
        s.declared_exit(statue_room, Direction::N),
        DeclaredExit::Code,
        "the gnome's tunnel beyond the newly-opened door is code-decided, not a fixed room — \
         Phase 1 must say so rather than guessing a destination it cannot see"
    );

    // A move that IS a fixed room must still read as one on the very same room — the derivation
    // is not simply refusing everything here.
    assert_eq!(
        s.declared_exit(statue_room, Direction::S),
        DeclaredExit::Room(111),
        "south, back to the Fountain Room, is an ordinary declared exit"
    );
}
