//! SQ-1315: Anchorhead (2018), and the room global that was never the room global.
//!
//! # The report
//!
//! Two complaints against 0.4.4, both on `stories/Anchorhead.gblorb` — the 2018 Steam/itch
//! *Illustrated Edition*, Michael Gentry, Glulx, **release 1 / serial 171017**, Inform 7 build
//! 6M62 (I6/v6.33 lib 6/12N). NOT `AnchorheadDemo.gblorb`, which SQ-1304 uses and which is a
//! different build of a different cut of the game.
//!
//! 1. *"Attempting to enter a room and failing, the game thinks you entered the room."*
//! 2. *"There are multiple rooms that teleport you randomly when you leave them; Lanthorn doesn't
//!    parse that at all."*
//!
//! The reporter's `/export-map` dump carried ONE `"Twisting Lane"` node with four fixed compass
//! passages the layout had given up on — `W`, `E` and `NW` all to *Outside the Real Estate
//! Office*, `NE` to *Riverwalk* — where the game has no fixed exit out of that room at all.
//!
//! # One cause, both complaints
//!
//! `glulx_roomlock` locks onto the RAM word that tracks the current room. On this story
//! [`crate::glulx_roomlock::RoomLock::name_witness`] resolves on the very first move, and among
//! the seven words that all hold the room after an ordinary walk it takes the LOWEST address —
//! `0x2237F8`. Measured on this fixture, that word is not `location`; it is Inform 7's own *room
//! gone to*, the going action's destination variable, and `location` is the next word up at
//! `0x223820`. The two are indistinguishable for as long as every move succeeds, and then:
//!
//! ```text
//! turn  command  transcript                                    0x2237F8      0x223820
//!  [1]  east     "Outside the Real Estate Office" (heading)     RE Office     RE Office
//!  [2]  east     "The glass-paneled door is locked, and you     Office        RE Office
//!                 lack a key."
//! ```
//!
//! — a move a CHECK rule refused, with the word holding the room behind the door. `apply_turn`
//! was handed an arrival in *Office*, minted `RE Office --E--> Office`, and the map said the
//! player had walked through a door the game had just told them was locked. That is complaint 1,
//! and it is exactly what [`a_refused_move_through_a_locked_door_maps_no_arrival`] pins.
//!
//! ```text
//!  [2]  north    "…you are utterly lost… emerging onto a . . .  0               Chilly Avenue
//!                 Chilly Avenue" (heading)
//! ```
//!
//! — Twisting Lane declares **no exits whatever** (all twelve directions `Absent`; the wander is
//! an `instead` rule), and a rerouted `going` leaves *room gone to* at zero. `RoomLock::room_id`
//! refuses a zero and `adopt_heading_for_room` refuses a room it cannot identify, so the map
//! stayed in the lane while the player walked off — and the NEXT move's passage was minted out of
//! the LANE. Walk out four times and you have four fixed lane exits leading to four streets the
//! lane does not touch, which is the dump. The randomness was never seen at all, because from the
//! map's point of view the player never left. That is complaint 2.
//!
//! # The fix, and why the old checks could not have caught it
//!
//! `RoomLock::verify` asks whether the locked word still holds an object of this story (it does,
//! or a zero, which it is documented not to judge) and whether it has frozen while new room names
//! appear (it has not — it moves eagerly, just wrongly). Both questions are about the WORD.
//! [`app::glulx_session::GlulxSession::check_room_lock_against_story`] asks the story instead: the
//! room it NAMED this turn, resolved through its own compiled world model to exactly one address,
//! against what the locked word holds — on a turn the locked word itself calls a move.
//! Disagreement rejects the address for the session
//! ([`crate::glulx_roomlock::RoomLock::reject`]) and re-keys this turn's room from the name, so
//! the move that catches the lock out is also the first one mapped correctly.
//!
//! Measured here: the lock resolves on move one at `0x2237F8`, is rejected on the locked door two
//! moves later, and re-resolves at `0x223820` — the real `location` — two moves after that,
//! tracking every room for the rest of the walk.
//!
//! Every case skips vacuously without `stories/` (gitignored).

use std::path::PathBuf;

use app::engine::{Engine, KeyInput};
use app::glulx_session::GlulxSession;
use app::roomid::synthetic_room_id;
use app::session::{apply_turn, DeathWatch, InputKind};
use app::state::AppState;
use mapper::direction::Direction;
use mapper::graph::RoomId;
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;

/// The 2018 release's own name for the room the game opens in.
const OPENING: &str = "Outside the Real Estate Office";
/// The room whose every direction wanders.
const LANE: &str = "Twisting Lane";

fn story() -> Option<Vec<u8>> {
    let path = fixture_path("Anchorhead.gblorb");
    match std::fs::read(&path) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            None
        }
    }
}

/// Natural play through the REAL pipeline — `apply_turn`, then `random_exit_probe`'s own Phase-2
/// gate ([`app::random_exit_probe::arm_for_finished_turn`], SQ-1314's one implementation of it).
///
/// No probe is armed and no pre-move save is kept, so nothing can arm and every suspicion
/// resolves immediately (`Mapper::resolve_suspicion_as_random`) — the documented "no probe can
/// run" path, and the only affordable one here: a shadow of an 18 MB story per suspicious move
/// would cost minutes per case to test a seam SQ-1264 already covers on Adventure.
///
/// Deliberately no room-lock warmup. The warmup keeps a mid-session remap out of the mapper, and
/// it is not something a player does — the whole of this quest lives in what the lock does on the
/// turns a player actually plays.
struct Play {
    state: AppState,
    mapper: Mapper,
    session: GlulxSession,
    death: DeathWatch,
    /// Every `(command, room label after the turn)`, for a failure message that explains itself.
    log: Vec<(String, String)>,
}

impl Play {
    fn anchorhead(tag: &str, seed: u32) -> Option<Play> {
        let bytes = story()?;
        let blorb = blorb::Blorb::parse(bytes).ok()?;
        let (kind, exec) = blorb.executable().ok()?;
        assert_eq!(kind, blorb::ExecKind::Glulx, "Anchorhead.gblorb is a Glulx blorb");
        let store: PathBuf = app::scratch_dir(tag);
        let mut s = GlulxSession::new_in(
            store,
            exec.to_vec(),
            80,
            24,
            true,
            false,
            false,
            false,
            (1, 1),
            None,
            &[],
            [[(None, None); 11]; 2],
            false,
            Some(seed),
        )
        .unwrap_or_else(|e| panic!("Anchorhead (2018) boots: {e:?}"));
        for _ in 0..40 {
            if s.current_location().is_some() {
                break;
            }
            if s.pending_input() != InputKind::Char {
                break;
            }
            s.submit_key(KeyInput::Enter);
        }
        Some(Play {
            state: AppState::default(),
            mapper: Mapper::default(),
            session: s,
            death: DeathWatch::default(),
            log: Vec::new(),
        })
    }

    /// One turn, exactly as `turn::finish_command_turn` drives one. Returns the transcript.
    fn turn(&mut self, cmd: &str) -> String {
        for _ in 0..6 {
            if self.session.pending_input() != InputKind::Char {
                break;
            }
            self.session.submit_key(KeyInput::Enter);
        }
        if self.session.pending_input() != InputKind::Line {
            return String::new();
        }
        let room_before = self.mapper.graph.current();
        let mut result = Engine::submit(&mut self.session, cmd);
        result.declared_exit =
            app::random_exit_probe::declared_exit_for_command(cmd, room_before, |o, d| {
                Engine::declared_exit(&self.session, o, d)
            });
        for (name, addr) in self.session.take_room_remap() {
            self.mapper.rekey_room(synthetic_room_id(&name), app::roomid::glulx_room_id(addr));
        }
        apply_turn(&mut self.mapper, cmd, &result, &mut self.death);
        app::random_exit_probe::arm_for_finished_turn(
            &mut self.state,
            &self.session,
            &mut self.mapper,
            cmd,
            room_before,
            result.declared_exit,
        );
        app::random_exit_probe::settle_random_exit_search(&mut self.state, &mut self.mapper);
        self.log.push((cmd.to_string(), self.here()));
        result.transcript
    }

    /// What the map calls the room the player is standing in.
    fn here(&self) -> String {
        self.mapper
            .graph
            .current()
            .and_then(|id| self.mapper.graph.room(id))
            .map(|r| r.label().to_string())
            .unwrap_or_default()
    }

    fn room_named(&self, name: &str) -> Vec<RoomId> {
        self.mapper.graph.rooms().filter(|r| r.label() == name).map(|r| r.id).collect()
    }

    fn edge(&self, from: RoomId, dir: Direction) -> Option<RoomId> {
        self.mapper
            .graph
            .connections()
            .iter()
            .find(|c| c.origin == from && c.dir == dir)
            .map(|c| c.dest)
    }

    /// The map as the dump would show it, plus the route that produced it.
    fn picture(&self) -> String {
        let mut out = String::from("  route:\n");
        for (cmd, room) in &self.log {
            out.push_str(&format!("    {cmd:16} -> {room}\n"));
        }
        let mut rooms: Vec<_> = self.mapper.graph.rooms().collect();
        rooms.sort_by_key(|r| r.id);
        for r in rooms {
            let tag = if r.id == synthetic_room_id(r.label()) { "  <== NAME-KEYED" } else { "" };
            out.push_str(&format!(
                "  ROOM #{} {:?} random={:?} pool={:?}{tag}\n",
                r.id,
                r.label(),
                r.random_exits,
                r.random_destinations
            ));
        }
        for c in self.mapper.graph.connections() {
            out.push_str(&format!("  EDGE #{} {:?} #{}\n", c.origin, c.dir, c.dest));
        }
        out
    }
}

/// One step towards Twisting Lane from each room a wander out of it can drop the player in, plus
/// the rooms those steps pass through. Greedy: every entry moves one room closer, so following it
/// terminates.
///
/// Measured on this fixture across sixteen seeds, the wander pool is *Chilly Avenue*, *Narrow
/// Street*, *Outside the Real Estate Office*, *Riverwalk*, *Shadowy Corner*, *Town Square* and the
/// lane itself. A landing this table does not name is a change in the fixture, not in the map, so
/// [`walk_back_to_the_lane`] says so by name rather than wandering on.
fn step_towards_the_lane(room: &str) -> Option<&'static str> {
    Some(match room {
        "Narrow Street" => "south",
        OPENING => "west",
        "North Square" => "east",
        "Local Pub" => "up",
        "Whateley Bridge" => "north",
        "Town Square" => "north",
        "Riverwalk" => "west",
        "Chilly Avenue" => "north",
        "Shadowy Corner" => "east",
        "Mill Road" => "south",
        _ => return None,
    })
}

/// Walk back to Twisting Lane from wherever the last wander landed. `false` when the route ran out
/// of table or out of patience — the caller stops the loop rather than asserting on a walk that
/// never got where it was going.
fn walk_back_to_the_lane(p: &mut Play) -> bool {
    for _ in 0..12 {
        let here = p.here();
        if here == LANE {
            return true;
        }
        let Some(step) = step_towards_the_lane(&here) else {
            eprintln!("SQ-1315: no route home from {here:?} — the wander pool has changed");
            return false;
        };
        p.turn(step);
    }
    false
}

/// (a) The story names its own opening room before a single move is made, and the first move the
/// learner can score resolves the lock (SQ-1303 + SQ-1286).
///
/// The opening room is address-keyed from turn zero — `room_by_static_name` turns the heading back
/// into the one room the compiled map calls that — so the id it carries is never the hash of its
/// heading, before or after the lock lands.
///
/// `look` first, and it is not padding: `RoomLock::name_witness` compares this turn's RAM against
/// the PREVIOUS turn's to see which words moved, so the very first turn a session observes has
/// nothing to be a witness against and can never lock, whatever it does. One quiet turn of any
/// kind gives it the predecessor, and the next move locks — which on this story is the second
/// command and, in ordinary play, whatever the player types before they first walk somewhere.
#[test]
fn the_lock_resolves_on_the_first_scoreable_move_and_the_opening_room_is_address_keyed() {
    let Some(mut p) = Play::anchorhead("sq1315-lock", 1) else { return };

    let opening = p.session.current_location().expect("the story names its opening room at boot");
    assert_eq!(opening.name, OPENING, "the 2018 release opens outside the estate agent's");
    assert_ne!(
        opening.number,
        synthetic_room_id(OPENING),
        "…keyed by the room's ADDRESS, not by the hash of its heading"
    );
    assert!(
        p.session.locked_room_global().is_none(),
        "…and nothing has moved yet, so there is nothing for the lock to correlate against"
    );

    p.turn("look");
    assert!(
        p.session.locked_room_global().is_none(),
        "a turn that moves nobody is not a witness\n{}",
        p.picture()
    );

    p.turn("west");
    assert_eq!(p.here(), "Narrow Street", "west is the way into town\n{}", p.picture());
    assert!(
        p.session.locked_room_global().is_some(),
        "the first move the learner can score resolves the room lock\n{}",
        p.picture()
    );
    assert_eq!(
        p.room_named("Narrow Street").len(),
        1,
        "…and the room it arrives in is one node\n{}",
        p.picture()
    );
    assert!(
        p.mapper.graph.rooms().all(|r| r.id != synthetic_room_id(r.label())),
        "…with every room on the map keyed by its address\n{}",
        p.picture()
    );
}

/// **Complaint 1.** `east` at the opening room is a locked door the game refuses. The player does
/// not move, so the map must not move either — no arrival, no `Office` node, and no east passage
/// out of the room they are still standing in.
///
/// Falsification: revert `check_room_lock_against_story` and this fails with the reported symptom
/// — `Office` on the map and `Outside the Real Estate Office --E--> Office` minted from a turn
/// whose whole text is *"The glass-paneled door is locked, and you lack a key."*
#[test]
fn a_refused_move_through_a_locked_door_maps_no_arrival() {
    let Some(mut p) = Play::anchorhead("sq1315-refused", 1) else { return };
    p.turn("west");
    p.turn("east");
    assert_eq!(p.here(), OPENING, "back outside the estate agent's\n{}", p.picture());
    let outside = p.mapper.graph.current().expect("standing somewhere");

    let text = p.turn("east");

    // Non-vacuity: the game really did refuse this move, in the words the report describes.
    assert!(
        text.contains("locked") && !text.contains("Office\n"),
        "the door is locked and the game prints no arrival, got: {text:?}"
    );

    assert_eq!(
        p.here(),
        OPENING,
        "SQ-1315: a refused move leaves the player where they were\n{}",
        p.picture()
    );
    assert!(
        p.room_named("Office").is_empty(),
        "SQ-1315: the room behind the locked door is not on the map — the player never entered \
         it\n{}",
        p.picture()
    );
    assert_eq!(
        p.edge(outside, Direction::E),
        None,
        "SQ-1315: …and no east passage was minted out of the room they are still standing in\n{}",
        p.picture()
    );
}

/// **Complaint 2.** Every direction out of Twisting Lane is an `instead` rule that wanders the
/// player into a random street. Walk `north` out of it and back several times: the lane stays ONE
/// node, `north` becomes a random exit carrying the landings actually observed, and no fixed
/// passage is left standing on it.
///
/// The route is adaptive rather than pinned, because the landings are the point: a fixed command
/// list is a different walk on every seed, and one that pinned the seed would prove only that the
/// seed lands where it lands. [`walk_back_to_the_lane`] navigates home from wherever the story put
/// the player, so what the case asserts is the SHAPE the map ends in.
///
/// Falsification: revert `check_room_lock_against_story` and this fails with the reported symptom
/// — the map never leaves the lane, so it observes ONE landing (the room the player's NEXT move
/// reaches), records `north` as an ordinary fixed passage out of the lane, and never marks
/// anything random at all.
#[test]
fn the_twisting_lanes_wandering_exit_is_recorded_as_a_random_exit() {
    let Some(mut p) = Play::anchorhead("sq1315-lane", 1) else { return };

    // Into the lane the way a player gets there: west to Narrow Street, south to the lane.
    p.turn("west");
    p.turn("south");
    assert_eq!(p.here(), LANE, "south off Narrow Street is the lane\n{}", p.picture());
    let lane = p.mapper.graph.current().expect("standing in the lane");

    // Non-vacuity, and the reason the wander is invisible to the exit table: this room declares
    // nothing in any direction, so `declared_exit` can never contradict a landing here.
    for d in [Direction::N, Direction::S, Direction::E, Direction::W] {
        assert_eq!(
            Engine::declared_exit(&p.session, lane, d),
            app::engine::DeclaredExit::Absent,
            "Twisting Lane's compiled map declares no {d:?} exit — the wander is a rule"
        );
    }

    let mut landings: Vec<String> = Vec::new();
    for _ in 0..6 {
        p.turn("north");
        let landed = p.here();
        if !landings.contains(&landed) {
            landings.push(landed);
        }
        if !walk_back_to_the_lane(&mut p) {
            break;
        }
    }

    // Non-vacuity AND the reported symptom, which are the same assertion here. The story sends the
    // player somewhere different nearly every time, so a map that has seen fewer than two streets
    // out of this room after six walks is a map that did not see the player leave — which is
    // precisely what the wrong lock produced: every wander reads as `LANE` because `location`
    // never moved as far as the map was concerned, and the fixed `N` passage in the picture below
    // is the one the NEXT move minted out of the lane.
    assert!(
        landings.iter().filter(|l| *l != LANE).count() >= 2,
        "SQ-1315: six walks out of the lane reached fewer than two streets — the map never saw \
         the player leave. Saw {landings:?}\n{}",
        p.picture()
    );

    assert_eq!(
        p.room_named(LANE).len(),
        1,
        "SQ-1315: one lane, not one per landing\n{}",
        p.picture()
    );
    assert!(
        p.mapper.graph.is_random_exit(lane, Direction::N),
        "SQ-1315: north out of the lane lands somewhere different every time — a random exit\n{}",
        p.picture()
    );
    assert_eq!(
        p.edge(lane, Direction::N),
        None,
        "SQ-1315: …and no fixed passage is left standing on it\n{}",
        p.picture()
    );

    let pool = p.mapper.graph.room(lane).map(|r| r.random_destinations.clone()).unwrap_or_default();
    let pooled: Vec<RoomId> = pool
        .iter()
        .find(|(d, _)| *d == Direction::N)
        .map(|(_, dests)| dests.clone())
        .unwrap_or_default();
    assert!(
        pooled.len() >= 2,
        "SQ-1315: the pool holds the streets the wander was seen to reach, got {pooled:?}\n{}",
        p.picture()
    );
    assert!(
        !pooled.contains(&lane),
        "SQ-1315: …and not the lane itself — a landing back home is the room card's own 'back \
         here', not a pooled destination\n{}",
        p.picture()
    );
}
