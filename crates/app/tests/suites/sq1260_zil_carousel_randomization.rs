//! SQ-1260: Zork II's Carousel Room, the ZIL proving case for the exit
//! convention added in `crates/zvm/src/world.rs`.
//!
//! The room's OWN exit table (`2dungeon.zil`,
//! <https://github.com/historicalsource/zork2>) is eight perfectly ordinary
//! UEXITs:
//!
//! ```text
//! <ROOM CAROUSEL-ROOM
//!       (NORTH TO MARBLE-HALL) (NE TO STREAM-PATH) (EAST TO TOPIARY-ROOM)
//!       (SE TO RIDDLE-ROOM) (SOUTH TO MENHIR-ROOM) (SW TO COBWEBBY-CORRIDOR)
//!       (WEST TO ROOM-8) (NW TO COOL-ROOM)
//!       (ACTION CAROUSEL-ROOM-FCN)>
//! ```
//!
//! The randomness is not in the exit table at all — it lives entirely in
//! `CAROUSEL-ROOM-FCN` (`2actions.zil`), an `M-BEG` "before going" hook that
//! runs BEFORE `V-WALK` ever reads the property above, exactly the shape
//! `crates/zvm/src/world.rs`'s Lost Pig/gnome-tunnel comments describe:
//!
//! ```text
//! (<AND <NOT ,CAROUSEL-FLIP-FLAG> <EQUAL? .RARG ,M-BEG> <VERB? WALK>>
//!  <COND (<EQUAL? ,PRSO ,P?UP ,P?DOWN> <RFALSE>)>
//!  <COND (<EQUAL? ,PRSO ,P?OUT> …picks a direction at random… <SETG PRSO ,P?EAST>)
//!        (T …”You’re not sure which direction is which”…)>
//!  <COND (<OR <EQUAL? ,PRSO ,P?WEST> <PROB 80>>
//!         <SETG PRSO <GET ,EIGHT-DIRECTIONS <- <RANDOM 7> 1>>>)>
//!  <V-WALK> <RTRUE>)
//! ```
//!
//! West is the deterministic case this suite drives: `<EQUAL? ,PRSO ,P?WEST>`
//! makes the `OR` true unconditionally, so `<SETG PRSO …>` runs on EVERY
//! attempt — the typed direction is NEVER honoured, 100% of the time, no
//! `PROB 80` roll needed. (Every OTHER direction is overridden only 80% of
//! the time — real, but a flakier thing to pin a test to.) The seven-entry
//! `EIGHT-DIRECTIONS` table (`GLOBAL EIGHT-DIRECTIONS <TABLE P?NORTH P?EAST
//! P?SOUTH P?NE P?SE P?SW P?NW>` — WEST is never a substitute for itself) is
//! what SQ-1260's `resolve_zil` cannot see: the room's own compiled data has
//! nothing "conditional" shaped about it at all — it is Phase 1's live-walk
//! mismatch and SQ-1264's contradiction rule (`session::apply_turn`) that
//! catch this, exactly as they do for Adventure's forest and Lost Pig's
//! gnome tunnels.
//!
//! Skips vacuously without `stories/` (gitignored).

use std::sync::Arc;

use app::engine::{DeclaredExit, Engine};
use app::probe::ShadowRecipe;
use app::session::{apply_turn, DeathWatch, GameSession, TurnResult};
use app::state::AppState;
use mapper::direction::Direction;
use mapper::graph::RoomId;
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;

fn story(name: &str) -> Option<Vec<u8>> {
    match std::fs::read(fixture_path(name)) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", fixture_path(name).display());
            None
        }
    }
}

fn recipe(bytes: &[u8]) -> ShadowRecipe {
    ShadowRecipe {
        story_bytes: Arc::new(bytes.to_vec()),
        store: std::path::PathBuf::new(),
        vfs_bytes: Arc::new(Vec::new()),
        honor_game_colours: true,
        interpreter_number: None,
        random_seed: None,
        acceleration: true,
        screen: (80, 24),
    }
}

/// Drives `apply_turn` + a synchronously-settled Phase-2 search exactly the
/// way `turn::finish_command_turn` does — `declared_exit.rs`'s and
/// `sq1264_forest_randomization.rs`'s own `Play`/`ZPlay` are the precedent
/// this mirrors.
struct Play {
    state: AppState,
    mapper: Mapper,
    session: GameSession,
    death: DeathWatch,
}

impl Play {
    fn zork2() -> Option<Play> {
        let bytes = story("zork2-r48-s840904.z3")?;
        let mut s = GameSession::new_with_trace(
            bytes.clone(), true, false, None, false, Vec::new(), None, None, Some((25, 80)),
        )
        .expect("Zork II boots without a ZError");
        s.set_strip_prompt(false);
        let mut state = AppState::default();
        state.probe.arm(recipe(&bytes));
        Some(Play { state, mapper: Mapper::default(), session: s, death: DeathWatch::default() })
    }

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
                        app::random_exit_probe::settle_random_exit_search(&mut self.state, &mut self.mapper);
                    }
                }
            }
        }

        // SQ-1269: a suspicion `apply_turn` left pending rather than marking on the spot — arm a
        // probe to decide it, mirroring `turn::finish_command_turn`, resolving immediately when
        // none can run.
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

    fn edge(&self, from: RoomId, dir: Direction) -> Option<RoomId> {
        self.mapper.graph.connections().iter().find(|c| c.origin == from && c.dir == dir).map(|c| c.dest)
    }
}

/// Reach the Carousel Room: take and light the lantern in the Barrow, then
/// `south, south, south, southwest, south, southwest, southwest` — 7 moves,
/// 9 turns total counting `look`/`take`/`turn on`. Verified against a real
/// walk of `stories/zork2-r48-s840904.z3` (Inside the Barrow → Narrow Tunnel
/// → Foot Bridge → Great Cavern → Shallow Ford → Dark Tunnel → Path Near
/// Stream → Carousel Room), and against `2dungeon.zil`'s own `(SW TO
/// CAROUSEL-ROOM)` on `STREAM-PATH`.
fn reach_carousel(p: &mut Play) -> RoomId {
    p.turn("look");
    p.turn("take lantern");
    p.turn("turn on lantern");
    for cmd in ["south", "south", "south", "southwest", "south", "southwest", "southwest"] {
        p.turn(cmd);
    }
    let here = p.mapper.graph.current().expect("standing in the Carousel Room");
    assert_eq!(p.mapper.graph.room(here).map(|r| r.label().to_string()), Some("Carousel Room".to_string()));
    here
}

/// The Carousel Room's own compiled exit table is eight plain UEXITs — every
/// one of them a confident `DeclaredExit::Room(_)`, never `Code`/`Absent`.
/// This is the shape that makes the room a genuine SQ-1264 case rather than
/// an SQ-1257-Phase-1/2 one: nothing about the STATIC table looks
/// conditional at all.
#[test]
fn carousel_rooms_declared_exits_are_all_plain_uexits() {
    let Some(mut p) = Play::zork2() else { return };
    let carousel = reach_carousel(&mut p);
    for dir in [
        Direction::N, Direction::NE, Direction::E, Direction::SE,
        Direction::S, Direction::SW, Direction::W, Direction::NW,
    ] {
        assert!(
            matches!(p.session.declared_exit(carousel, dir), DeclaredExit::Room(_)),
            "{dir:?} must be a plain declared UEXIT room, matching 2dungeon.zil's CAROUSEL-ROOM"
        );
    }
}

/// West is NEVER the direction actually walked (`<EQUAL? ,PRSO ,P?WEST>`
/// alone satisfies `CAROUSEL-ROOM-FCN`'s `OR`, no `PROB 80` roll needed) —
/// the single most deterministic proof this suite has that a plain declared
/// `Room(_)` is not the same thing as a real passage. One walk already
/// contradicts it (Phase 1 alone); a second, real, returned-and-repeated
/// walk gives the `?` matrix cell its second pooled destination.
#[test]
fn carousel_west_never_leads_to_the_declared_room_8_and_ends_marked_random() {
    let Some(mut p) = Play::zork2() else { return };
    let carousel = reach_carousel(&mut p);
    let DeclaredExit::Room(declared_room_8) = p.session.declared_exit(carousel, Direction::W) else {
        panic!("west must be a plain declared UEXIT room (Room-8)");
    };

    // ── Attempt 1: west never lands at the declared room. ──
    p.turn("west");
    let first = p.mapper.graph.current().expect("landed somewhere");
    assert_ne!(first, carousel, "west is never refused here — CAROUSEL-ROOM-FCN always calls V-WALK");
    assert_ne!(first, declared_room_8, "west NEVER honours the typed direction in the carousel");
    assert!(p.mapper.graph.is_random_exit(carousel, Direction::W), "marked random on the very first mismatch");
    assert_eq!(p.edge(carousel, Direction::W), None, "no edge minted");

    // Walk back to the Carousel Room by real navigation — every one of the
    // eight rooms the carousel can redirect to has its own return path
    // straight back (2dungeon.zil's own `(… TO CAROUSEL-ROOM)` lines), and
    // `crate::fixture_paths` is real gameplay, not mapper bookkeeping.
    walk_back_to_carousel(&mut p, first);
    assert_eq!(p.mapper.graph.current(), Some(carousel), "back at the Carousel Room");

    // ── Attempt 2: west again — SQ-1264's rule needs no NEW disagreement to
    // stay marked random (there is no edge to contradict yet), but a second
    // real landing is what gives `random_destinations` more than one entry
    // when it differs from the first. ──
    p.turn("west");
    let second = p.mapper.graph.current().expect("landed somewhere");
    assert_ne!(second, declared_room_8, "west still never honours the typed direction");
    assert!(p.mapper.graph.is_random_exit(carousel, Direction::W), "still marked random");
    assert_eq!(p.edge(carousel, Direction::W), None, "still no edge");

    let pool = p.mapper.graph.random_destinations(carousel, Direction::W);
    assert!(pool.contains(&first), "the first landing is in the pool: {pool:?}");
    if second != first {
        assert!(pool.len() > 1, "a second, DIFFERENT landing must grow the pool: {pool:?}");
        assert!(pool.contains(&second), "the second landing is in the pool: {pool:?}");
    }
    assert_eq!(
        mapper::matrix::classify(&p.mapper.graph, carousel, Direction::W),
        mapper::matrix::MatrixCell::Random { destinations: pool.len() },
        "the matrix reads a `?` cell, not a confident arrow"
    );
}

/// Walk directly back to the Carousel Room from whichever of its eight
/// neighbours the player landed in, using each room's own real return exit
/// (`2dungeon.zil`): Marble Hall south, Stream Path southwest, Topiary Room
/// west, Riddle Room down/northwest, Menhir Room north, Cobwebby Corridor
/// northeast, Room-8 east, Cool Room southeast.
fn walk_back_to_carousel(p: &mut Play, landed_in: RoomId) {
    let name = p.mapper.graph.room(landed_in).map(|r| r.label().to_string()).unwrap_or_default();
    let back = match name.as_str() {
        "Marble Hall" => "south",
        "Path Near Stream" => "southwest",
        "Topiary" => "west",
        "Riddle Room" => "down",
        "Menhir Room" => "north",
        "Cobwebby Corridor" => "northeast",
        "Room 8" => "east",
        "Cool Room" => "southeast",
        other => panic!("unexpected carousel neighbour {other:?} — not one of CAROUSEL-ROOM's eight UEXIT targets"),
    };
    p.turn(back);
}
