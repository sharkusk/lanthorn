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

// ── Zork I (ZIL, not Inform): the seam now reads ZIL's OWN exit convention ──

/// Zork I is not Inform-compiled — `door_dir`/`*_to` do not exist in its
/// table — but since SQ-1260 the derivation reads ZIL's OWN `<DIRECTIONS>`
/// convention instead of answering `Unknown` everywhere. West of House's
/// north/northeast are plain UEXITs to the same room, its east is a NEXIT
/// ("the door is boarded and you can't remove the boards" — no passage in
/// any state), matching `1dungeon.zil` exactly
/// (<https://github.com/historicalsource/zork1>: `(NORTH TO
/// NORTH-OF-HOUSE) (NE TO NORTH-OF-HOUSE) (EAST "The door is boarded…")`).
/// `apply_turn` behaves exactly as it always has regardless — a real move
/// still mints its edge (West of House → north → North of House), and now
/// the declared UEXIT's own room number matches that real destination too.
#[test]
fn zork1_declared_exits_read_zils_own_convention_and_the_move_still_mints_its_edge() {
    let Some(bytes) = story("zork1-r88-s840726.z3") else {
        eprintln!("SKIP: gitignored stories/zork1-r88-s840726.z3 missing");
        return;
    };
    let s = boot(bytes);
    let start = s.current_location().expect("Zork I names West of House at boot");

    let DeclaredExit::Room(north_dest) = s.declared_exit(start.number, Direction::N) else {
        panic!("north must be a plain declared UEXIT room");
    };
    assert_eq!(
        s.declared_exit(start.number, Direction::NE),
        DeclaredExit::Room(north_dest),
        "NE is a separate UEXIT to the SAME declared room as north"
    );
    assert_eq!(
        s.declared_exit(start.number, Direction::E),
        DeclaredExit::Message,
        "east is a NEXIT — a refusal string, never a passage"
    );

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
    assert_eq!(north, north_dest, "the declared UEXIT's room number matches the real move's destination");
    assert_eq!(
        mapper.graph.connections().iter().find(|c| c.origin == west && c.dir == Direction::N).map(|c| c.dest),
        Some(north),
        "the declared exit must not stop the ordinary edge from being minted"
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
///
/// SQ-1269: `apply_turn` no longer marks this on the spot — it leaves a
/// [`mapper::mapper::RandomExitSuspicion`] pending instead (proven directly here) and the mark
/// only lands once that is resolved. This bare call has no probe apparatus at all, so it stands
/// for the "no probe can run" case — `resolve_suspicion_as_random` is exactly the immediate
/// fallback `turn::finish_command_turn` reaches for then, and produces the same end state the old
/// immediate marking always did.
#[test]
fn synthetic_random_exit_records_no_edge_and_marks_the_cell_random() {
    let mut mapper = Mapper::default();
    let mut death = DeathWatch::default();
    apply_turn(&mut mapper, "", &TurnResult::observation(snap(1, "Tunnel")), &mut death);

    let mut r = TurnResult::observation(snap(2, "Other Tunnel"));
    r.declared_exit = Some(DeclaredExit::Room(3)); // declared a THIRD room, not 2
    apply_turn(&mut mapper, "north", &r, &mut death);
    assert!(!mapper.graph.is_random_exit(1, Direction::N), "not marked yet — this is a suspicion, not proof");
    let susp = mapper.take_random_exit_suspicion().expect("the declared mismatch left a suspicion pending");
    assert_eq!((susp.origin, susp.dir, susp.old_dest, susp.live_dest), (1, Direction::N, None, 2));
    mapper.resolve_suspicion_as_random(susp); // no probe available — resolve immediately

    assert!(
        !mapper.graph.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::N),
        "no edge north out of room 1"
    );
    assert_eq!(mapper.graph.current(), Some(2), "the player is still observed as having arrived in 2");
    assert_eq!(
        mapper::matrix::classify(&mapper.graph, 1, Direction::N),
        mapper::matrix::MatrixCell::Random { destinations: 1 },
        "the matrix must read this cell as `?¹`, not untried/probed"
    );
    // SQ-1261: the walk that EARNED the mark is itself evidence of where the story sends the
    // player — it must not have to wait for a second walk to be recorded.
    assert_eq!(
        mapper.graph.random_destinations(1, Direction::N),
        &[2],
        "the live destination of the very first random walk is recorded"
    );

    // A second walk of the now-marked direction, landing somewhere ELSE, adds to the list rather
    // than replacing it — the story has now shown two different destinations for one direction.
    // Back to room 1 first (an unrelated relocation — `apply_turn` above left `current` on room
    // 2, and "walk north out of room 1" needs the player standing there again).
    mapper.graph.set_current(1);
    let mut r2 = TurnResult::observation(snap(4, "Third Tunnel"));
    r2.declared_exit = Some(DeclaredExit::Room(3)); // still not what the story actually did
    apply_turn(&mut mapper, "north", &r2, &mut death);
    assert_eq!(
        mapper.graph.random_destinations(1, Direction::N),
        &[2, 4],
        "a second live destination joins the first, in first-seen order"
    );
    assert_eq!(
        mapper::matrix::classify(&mapper.graph, 1, Direction::N),
        mapper::matrix::MatrixCell::Random { destinations: 2 },
        "and the matrix count follows"
    );
}

fn snap(number: mapper::graph::RoomId, name: &str) -> app::engine::LocationInfo {
    app::engine::LocationInfo { number, parent: 0, name: name.to_string() }
}

// ── SQ-1264: the live-walk contradiction rule ───────────────────────────────
//
// A backstop for the statistical hole in SQ-1257 Phase 2's upgrade path, and for
// engines with no `declared_exit` seam at all (or one that, like Adventure's
// forests, cannot see the randomness because it lives in a REDIRECT the
// destination performs on arrival rather than in the origin's own exit table —
// see `crates/gvm/src/world.rs`'s module docs for the full mechanism). No
// `declared_exit` is populated in either case below (`TurnResult::observation`
// leaves it `None`), so the rule is proven here on evidence alone: the graph's
// own edges contradicting each other.

/// E→A, then E→B: the second walk contradicts the edge the first one minted. SQ-1269: this is no
/// longer settled on the spot — `apply_turn` leaves the edge standing and a
/// [`mapper::mapper::RandomExitSuspicion`] pending (proven directly here); this bare call has no
/// probe apparatus, so it stands for the "no probe can run" case, and `resolve_suspicion_as_random`
/// (the same immediate fallback `turn::finish_command_turn` reaches for then) is what actually
/// removes the stale edge, marks the direction random, and records BOTH destinations — the same
/// end state SQ-1264's old immediate rule produced, with no `declared_exit` involved at any point.
#[test]
fn a_contradicting_live_walk_marks_the_exit_random_with_no_declared_exit_seam() {
    let mut mapper = Mapper::default();
    let mut death = DeathWatch::default();
    apply_turn(&mut mapper, "", &TurnResult::observation(snap(1, "Origin")), &mut death);

    // E → A: an ordinary first walk, mints the edge exactly as always.
    apply_turn(&mut mapper, "east", &TurnResult::observation(snap(2, "Room A")), &mut death);
    assert_eq!(
        mapper.graph.connections().iter().find(|c| c.origin == 1 && c.dir == Direction::E).map(|c| c.dest),
        Some(2),
        "an ordinary first walk mints its edge"
    );
    assert!(!mapper.graph.is_random_exit(1, Direction::E));

    // Back to room 1, then E → B: a DIFFERENT destination for the same (origin, direction).
    mapper.graph.set_current(1);
    apply_turn(&mut mapper, "east", &TurnResult::observation(snap(3, "Room B")), &mut death);
    assert_eq!(
        mapper.graph.connections().iter().find(|c| c.origin == 1 && c.dir == Direction::E).map(|c| c.dest),
        Some(2),
        "the edge stands — nothing decided yet, this is a suspicion, not proof"
    );
    let susp = mapper.take_random_exit_suspicion().expect("the contradiction left a suspicion pending");
    assert_eq!((susp.origin, susp.dir, susp.old_dest, susp.live_dest), (1, Direction::E, Some(2), 3));
    mapper.resolve_suspicion_as_random(susp); // no probe available — resolve immediately

    assert!(
        !mapper.graph.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::E),
        "the contradicting edge is removed rather than silently overwritten"
    );
    assert!(mapper.graph.is_random_exit(1, Direction::E), "the direction is marked random");
    assert_eq!(
        mapper.graph.random_destinations(1, Direction::E),
        &[2, 3],
        "both the edge's old destination (A) and the new live landing (B) are recorded"
    );
    assert_eq!(
        mapper::matrix::classify(&mapper.graph, 1, Direction::E),
        mapper::matrix::MatrixCell::Random { destinations: 2 },
        "the matrix reads `?²`, not a confident arrow"
    );

    // E → A again: the direction is already marked, so no edge is minted and the destination set
    // is unchanged — a THIRD sighting of a room already in the set adds nothing new.
    mapper.graph.set_current(1);
    apply_turn(&mut mapper, "east", &TurnResult::observation(snap(2, "Room A")), &mut death);
    assert!(
        !mapper.graph.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::E),
        "still no edge — the direction stays marked random"
    );
    assert_eq!(
        mapper.graph.random_destinations(1, Direction::E),
        &[2, 3],
        "no duplicate: A was already in the set"
    );
}

/// Falsify: disabling the rule (simulated by never taking the contradiction branch — i.e. the
/// pre-SQ-1264 shape, reproduced by asserting what a bare `Mapper::observe`-only path would have
/// done) would silently overwrite the first edge instead of catching the contradiction. This is
/// the same falsification shape `declared_exit.rs`'s other `falsify_*` case uses: prove the OLD
/// symptom is what the rule now prevents, by exercising `Mapper::observe` directly (bypassing
/// `apply_turn`'s contradiction check entirely, exactly as every caller did before SQ-1264).
#[test]
fn falsify_without_the_contradiction_rule_a_second_destination_silently_overwrites_the_first() {
    let mut mapper = Mapper::default();
    mapper.observe(1, "Origin", None);
    mapper.observe(2, "Room A", Some(Direction::E));
    assert_eq!(
        mapper.graph.connections().iter().find(|c| c.origin == 1 && c.dir == Direction::E).map(|c| c.dest),
        Some(2)
    );
    mapper.graph.set_current(1);
    mapper.observe(3, "Room B", Some(Direction::E));
    assert_eq!(
        mapper.graph.connections().iter().find(|c| c.origin == 1 && c.dir == Direction::E).map(|c| c.dest),
        Some(3),
        "the original symptom: a bare `observe` overwrites the edge with no memory of A, and \
         nothing here is ever marked random — this is exactly what `apply_turn`'s contradiction \
         rule now intercepts before `observe` is ever reached for this shape"
    );
    assert!(!mapper.graph.is_random_exit(1, Direction::E));
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

// ── Phase 2: is a Code/Absent move actually random? ─────────────────────────
//
// Continues [`LOST_PIG_WALKTHROUGH`] into the tunnels themselves: get the
// glowing ball from the gnome (a light source the tunnels require), then walk
// north from the newly-opened door into Windy Cave, then further north into
// Twisty Cave — the first room whose OWN exits are genuinely `Absent` (no
// `*_to` property at all; verified in the SQ-1257 exploration this test
// reconstructs).
const LOST_PIG_INTO_THE_TUNNELS: &[&str] = &[
    "NORTH", "X WINDY TUNNEL", "NORTH", "SOUTH", "TAKE TORCH", "GO TO GNOME", "ASK GNOME FOR BALL",
    "GIVE TORCH TO GNOME", "THANK GNOME", "GO TO WINDY CAVE",
];

use std::path::PathBuf;
use std::sync::Arc;

use app::probe::ShadowRecipe;
use app::state::AppState;

fn recipe(bytes: &[u8]) -> ShadowRecipe {
    ShadowRecipe {
        story_bytes: Arc::new(bytes.to_vec()),
        store: PathBuf::new(),
        vfs_bytes: Arc::new(Vec::new()),
        honor_game_colours: true,
        interpreter_number: None,
        random_seed: None,
        acceleration: true,
        screen: (80, 24),
    }
}

/// Drives a real story through the SAME Phase-1 + Phase-2 sequence
/// `turn::finish_command_turn` drives, minus everything about that function
/// that is not the mapper/probe wiring (history, auto-save, the file paths) —
/// return_probe.rs's own `Play` is the precedent for testing this seam
/// without dragging in the rest of the turn driver.
struct Play {
    state: AppState,
    mapper: Mapper,
    session: GameSession,
    death: DeathWatch,
}

impl Play {
    fn lost_pig() -> Option<Play> {
        Play::for_story("LostPig.z8")
    }

    fn for_story(name: &str) -> Option<Play> {
        let bytes = story(name)?;
        let mut s = boot(bytes.clone());
        let _ = s.submit("");
        let mut state = AppState::default();
        state.probe.arm(recipe(&bytes));
        Some(Play { state, mapper: Mapper::default(), session: s, death: DeathWatch::default() })
    }

    /// One command, through Phase 1 (`declared_exit` + `apply_turn`) and — when it earns one —
    /// a synchronously-settled Phase 2 search, exactly as `turn.rs` arms one and the event loop
    /// later settles it, collapsed into one call for a deterministic test.
    fn turn(&mut self, cmd: &str) {
        let room_before = self.mapper.graph.current();
        let mut result = self.session.submit(cmd);
        let dir = mapper::direction::parse_direction(cmd);
        if let (Some(o), Some(d)) = (room_before, dir) {
            result.declared_exit = Some(self.session.declared_exit(o, d));
        }
        apply_turn(&mut self.mapper, cmd, &result, &mut self.death);
        let live_dest = self.mapper.graph.current();

        if let (Some(origin), Some(d), Some(dest)) = (room_before, dir, live_dest) {
            let already_random = self.mapper.graph.is_random_exit(origin, d);
            let worth_probing = dest != origin
                && (already_random
                    || matches!(result.declared_exit, Some(DeclaredExit::Absent) | Some(DeclaredExit::Code)));
            if worth_probing {
                if let Some((saved_room, save)) = &self.state.random_exit_pre_move_save {
                    if *saved_room == origin {
                        let save = Arc::clone(save);
                        let kind = if already_random {
                            app::random_exit_probe::SearchKind::Upgrade
                        } else {
                            app::random_exit_probe::SearchKind::FirstWalk
                        };
                        app::random_exit_probe::arm_random_exit_search(
                            &mut self.state, &self.session, origin, d, dest, kind, save,
                        );
                        app::random_exit_probe::settle_random_exit_search(
                            &mut self.state, &mut self.mapper,
                        );
                    }
                }
            }
        }

        // SQ-1269: a suspicion `apply_turn` left pending (a declared-exit mismatch, or a live
        // contradiction against something the map already believed) rather than marking on the
        // spot — arm a probe to decide it, mirroring `turn::finish_command_turn`, and resolve it
        // immediately when no probe can run.
        if let Some(susp) = self.mapper.take_random_exit_suspicion() {
            let mut armed = false;
            if let Some((saved_room, save)) = &self.state.random_exit_pre_move_save {
                if *saved_room == susp.origin {
                    let save = Arc::clone(save);
                    app::random_exit_probe::arm_random_exit_search(
                        &mut self.state, &self.session, susp.origin, susp.dir, susp.live_dest,
                        app::random_exit_probe::SearchKind::Suspicion { old_dest: susp.old_dest }, save,
                    );
                    if self.state.random_exit_search.is_some() {
                        app::random_exit_probe::settle_random_exit_search(&mut self.state, &mut self.mapper);
                        armed = true;
                    }
                }
            }
            if !armed {
                self.mapper.resolve_suspicion_as_random(susp);
            }
        }

        self.state.random_exit_pre_move_save = self
            .session
            .rng_seed()
            .map(|_| (self.mapper.graph.current().unwrap_or(0), Arc::new(self.session.save_state())));
    }

    fn edge(&self, from: mapper::graph::RoomId, dir: Direction) -> Option<mapper::graph::RoomId> {
        self.mapper.graph.connections().iter().find(|c| c.origin == from && c.dir == dir).map(|c| c.dest)
    }
}

/// SQ-1257 Phase 2 (and Phase 3) on the real game, and what the real game turned out to be.
///
/// Lost Pig's "random tunnels" are ONE room object (#183) whose `printed name` is
/// re-rolled on every move — "Confusing Passage", "Strange Place", "Twisty
/// Place", "Different Place", "Twisty Passage" — not several rooms with random
/// passages between them. Before SQ-1259 the room-lock keyed rooms by status-line
/// NAME, so every re-roll minted a fresh map room and the map ran wild; since
/// SQ-1259 the lock holds #183 through the game's own `location` global, so a
/// tunnel move is a move that led back to the room it left. Since SQ-1257 Phase 3,
/// a same-room move that ALSO renamed the room is no longer read as a self-loop —
/// it is a random exit (`?`), because "leads back here" would be a lie about a
/// direction whose destination never even keeps a stable NAME. Phase 2 still never
/// fires here either way, because its trigger is a move that CHANGED rooms. The
/// `?` machinery itself is proven on the synthetic engine in
/// `random_exit_probe::tests`; this case pins the real game's shape so the next
/// person does not go looking for random rooms that are not there.
///
/// Non-vacuity: `Twisty Cave` is asserted by name before anything else is asked.
#[test]
fn lost_pig_tunnels_are_one_room_whose_name_rerolls_as_random_exits_and_phase_2_never_fires() {
    let Some(mut p) = Play::lost_pig() else {
        eprintln!("SKIP: gitignored stories/LostPig.z8 missing");
        return;
    };
    for cmd in LOST_PIG_WALKTHROUGH {
        p.turn(cmd);
    }
    for cmd in LOST_PIG_INTO_THE_TUNNELS {
        p.turn(cmd);
    }
    let windy_cave = p.mapper.graph.current().expect("standing in Windy Cave");
    assert_eq!(p.mapper.graph.room(windy_cave).map(|r| r.name.as_str()), Some("Windy Cave"));

    // The gateway (Statue Room -> north -> Windy Cave) is `Code` but DETERMINISTIC: Phase 2
    // ran for it and found agreement, so its edge survives untouched.
    let statue_room: mapper::graph::RoomId = 128;
    assert_eq!(p.edge(statue_room, Direction::N), Some(windy_cave), "the deterministic gateway edge survives Phase 2");
    assert!(!p.mapper.graph.is_random_exit(statue_room, Direction::N), "and is not marked random");
    let probed_before_tunnels = p.state.probe.probes;
    assert!(probed_before_tunnels > 0, "the gateway's Code exit must have been Phase-2 probed at all");

    p.turn("NORTH"); // Windy Cave -> Twisty Cave
    let twisty = p.mapper.graph.current().expect("standing in Twisty Cave");
    assert_eq!(
        // The label is the status line's own text since SQ-1259 (the compiled short
        // name is lowercase "twisty cave"); compare case-insensitively.
        p.mapper.graph.room(twisty).map(|r| r.name.to_lowercase()).as_deref(),
        Some("twisty cave"),
        "non-vacuity guard: must actually be in Twisty Cave before asserting anything about it"
    );
    assert_eq!(
        p.session.declared_exit(twisty, Direction::E),
        DeclaredExit::Absent,
        "the tunnel room's exit table is empty — the compass was found, this room declares nothing"
    );

    // Walk the tunnels. Every move lands in the SAME object, under a different name.
    // (The NORTH into Twisty Cave was itself a `Code` exit that changed rooms, so it
    // was probed — count from here, not from before it.)
    let probed_before_tunnels = p.state.probe.probes;
    let rooms_before = p.mapper.graph.rooms().count();
    for cmd in ["EAST", "WEST", "NORTH", "SOUTH", "EAST"] {
        p.turn(cmd);
        assert_eq!(p.mapper.graph.current(), Some(twisty), "{cmd}: a tunnel move leads back to the tunnel room itself");
    }
    assert_eq!(p.mapper.graph.rooms().count(), rooms_before, "no new map room is minted for a re-rolled name");
    assert_eq!(p.state.probe.probes, probed_before_tunnels, "Phase 2 never fires: no move changed rooms");

    // SQ-1257 Phase 3: the room's label is whatever the story is CURRENTLY calling it — the
    // same text `current_location` itself reports — and every OTHER name it printed along
    // the way lives in the room's aliases, never as a fresh map room.
    let current_name = p.session.current_location().map(|l| l.name).unwrap_or_default();
    assert_eq!(
        p.mapper.graph.room(twisty).map(|r| r.name.as_str()),
        Some(current_name.as_str()),
        "the room's label is the CURRENT name the story is showing"
    );
    let aliases = p.mapper.graph.room(twisty).map(|r| r.aliases.clone()).unwrap_or_default();
    assert!(aliases.len() >= 2, "at least two other names were seen along the way: {aliases:?}");
    assert!(
        !aliases.contains(&current_name),
        "the current label is never also listed among its own aliases: {aliases:?}"
    );

    // Every rename-loop direction walked above reads as `?` ("destination varies"), not `↩`
    // ("leads back here") — and no self-loop connection was minted for #183 at all.
    //
    // SQ-1261: and every one of those marks carries an EMPTY destination set. A rename-loop's
    // "destination" is the room the player is already standing in — there is nothing to name,
    // unlike an ordinary random-exit walk that lands somewhere else — so `random_destinations`
    // must never smuggle the tunnel room itself in as though it were a real destination.
    for dir in [Direction::E, Direction::W, Direction::N, Direction::S] {
        assert_eq!(
            mapper::matrix::classify(&p.mapper.graph, twisty, dir),
            mapper::matrix::MatrixCell::Random { destinations: 0 },
            "{dir:?}: a rename-loop reads as `destination varies`, not `leads back here`"
        );
        assert!(
            p.mapper.graph.random_destinations(twisty, dir).is_empty(),
            "{dir:?}: a rename-loop records no destination — the room never actually changed"
        );
    }
    assert!(
        p.mapper.graph.self_loops(twisty).is_empty(),
        "no self-loop connection exists for the tunnel room: {:?}",
        p.mapper.graph.connections()
    );

    // And the drawn box agrees: every rename-loop direction is a bare `?` stub (no destinations
    // recorded to count), never an exit arrow — same fact, read off the render layer this time.
    let rm = mapper::render::render(&p.mapper.graph);
    let twisty_room = rm.rooms.iter().find(|r| r.id == twisty).expect("the tunnel room is placed");
    for dir in [Direction::E, Direction::W, Direction::N, Direction::S] {
        assert_eq!(
            twisty_room.random_stubs.iter().find(|(d, _)| *d == dir).map(|(_, count)| *count),
            Some(0),
            "{dir:?}: the box draws a bare `?` stub, not an exit arrow: {:?}",
            twisty_room.random_stubs
        );
    }
}

/// The seam's own reseed derivation never repeats the input seed and never repeats itself between
/// the two draws — a cheap, always-on guard for the fact the real-game case above depends on but
/// cannot itself prove in isolation (a coincidental live-seed match is not something a real game
/// run can be relied on to exercise).
#[test]
fn derived_seeds_differ_from_the_live_seed_and_from_each_other() {
    for live in [0u32, 1, 0x1234_5678, 0x9E37_79B9, u32::MAX] {
        let [a, b] = app::random_exit_probe::derived_seeds(live);
        assert_ne!(a, live, "seed A must not be the live seed ({live:#x})");
        assert_ne!(b, live, "seed B must not be the live seed ({live:#x})");
        assert_ne!(a, b, "the two derived seeds must not collide with each other");
    }
}

/// Since SQ-1260, Zork I's `declared_exit` seam no longer answers `Unknown` everywhere — see
/// [`zork1_declared_exits_read_zils_own_convention_and_the_move_still_mints_its_edge`] — but a
/// plain declared UEXIT (north, out of West of House) that a real move CONFIRMS is still worth
/// nothing to Phase 2: `worth_probing` only ever fires on `Absent`/`Code` (never a matching
/// `Room(_)`), so an ordinary UEXIT move costs this seam nothing at all — no snapshot stashed with
/// intent to probe, and no probe ever asked. (A Zork I move through a CEXIT/FEXIT-declared `Code`
/// exit is a different story — see `docs/internals/architecture.md`'s "Declared exits: the ZIL
/// convention" section for the cost that DOES now exist there, measured on Living Room's `down`.)
#[test]
fn zork1_uexit_move_never_arms_a_phase_2_probe() {
    let Some(mut p) = Play::for_story("zork1-r88-s840726.z3") else {
        eprintln!("SKIP: gitignored stories/zork1-r88-s840726.z3 missing");
        return;
    };
    p.turn("look");
    let west = p.mapper.graph.current().expect("West of House seeded");
    let probes_before = p.state.probe.probes;

    p.turn("north");
    let north = p.mapper.graph.current().expect("North of House");
    assert_ne!(north, west, "the move actually crossed something");
    assert_eq!(p.edge(west, Direction::N), Some(north), "an ordinary edge, minted as always");

    assert_eq!(p.state.probe.probes, probes_before, "no Phase-2 probe was ever asked");
    assert!(p.state.random_exit_search.is_none(), "and no search was ever armed");
}
