//! SQ-1314: the yacht's passages were erased as the player used them.
//!
//! # The report
//!
//! *"Initially, the map seemed to understand nautical directions, but as I used them, the map
//! gradually erased them, until the boat was full of disconnected rooms."* — against 0.4.4, on
//! `CounterfeitMonkey-11.gblorb`.
//!
//! The dump that came with it shows ten yacht rooms, nine of them holding a `random=` pool, and
//! **every pool naming the origin room itself**:
//!
//! ```text
//! ROOM #86 "Your Head"  random=[N→(#82 "Your Bunk", #86 "Your Head")]
//! ROOM #83 "Galley"     random=[SW→(#84 "Slango's Bunk", #83 "Galley"),
//!                               SE→(#82 "Your Bunk",     #83 "Galley")]
//! ```
//!
//! A pool member equal to its own origin means something recorded a REFUSED move as an arrival.
//! The 0.4.3 dump of the same ship has no pools at all — the ship words did not parse then, so
//! every passage was an anonymous `?`.
//!
//! # What the story actually declares
//!
//! Read straight off the compiled image by [`gvm::i7map`] — the same reader
//! `GlulxSession::i7_declared_exit` uses — Counterfeit Monkey has **twenty** direction columns.
//! Twelve are the compass and its portals; the last eight are its own:
//!
//! ```text
//! col 12: Starboard  col 13: port     col 14: fore           col 15: aft
//! col 16: aft-port   col 17: aft-starboard  col 18: fore-port  col 19: fore-starboard
//! ```
//!
//! and every yacht room hangs its passages on those, declaring **nothing** on the compass point
//! the map projects them onto:
//!
//! ```text
//! ROOM "Galley"  up -> Navigation Area | fore -> Brock's Stateroom
//!                aft-port -> Slango's Bunk | aft-starboard -> Your Bunk
//!     compass asks: N=Absent S=Absent … Sw=Absent … Up=Some(Navigation Area)
//! ```
//!
//! # The mechanism, and why `up` survived
//!
//! SQ-1296 taught `parse_direction` the quarter directions, so `ap` began resolving to
//! `Direction::SW`. From there, per turn:
//!
//! 1. `turn::finish_command_turn` asked `Engine::declared_exit(Galley, SW)`, which reads the
//!    story's SOUTHWEST column — empty — and answered `Absent`.
//! 2. `apply_turn` minted the ordinary edge. The map was still right at this instant.
//! 3. `Absent` put the move in the "worth probing" set, arming a `SearchKind::FirstWalk`.
//! 4. The probe typed `long_label(SW)` — **"southwest"** — into a reseeded shadow of the Galley.
//!    The story refuses it, so the shadow never left and reported the Galley as its landing.
//! 5. `judge` read that as a disagreement, `deliver_first_walk` DELETED the edge, marked the
//!    direction random, and pooled both Slango's Bunk and the Galley itself.
//!
//! Which is exactly the reported dump — and exactly why the four `up`/`down` passages between
//! Galley/Navigation Area and Foredeck/Crew Cabin are the only yacht edges left standing in it:
//! there, the compass word IS the ship's word.
//!
//! # The three rules this suite holds
//!
//! 1. **A probe asks in the player's vocabulary.** `mapper::direction::WalkedDir` carries the
//!    word beside the slot and is the only thing `arm_random_exit_search` accepts.
//! 2. **A refused move is not an arrival.** A shadow step landing on its own origin is discarded.
//! 3. **The compass column is not asked about a non-compass move.** Which is the rule this suite
//!    can prove on the fixture without reaching the yacht, and the one that stops step 1 above.
//!
//! Rules 1 and 2 are unit-tested in `app::random_exit_probe`; rule 3 in `app::session` as well as
//! here.
//!
//! # Why this suite does not play to the yacht
//!
//! Slango's ship is the endgame. Counterfeit Monkey's own `tools/command scripts/
//! test_full_game_alt.txt` is the only script that reaches it, and driving all 553 of its inputs
//! headless costs **4m10s** and desynchronises before the last dozen — the yacht is not reachable
//! by any route this suite could afford to pin. So the fixture is asked the question it CAN
//! answer authoritatively and with no turn played: its own compiled map. Every route below is
//! then built out of the passages the story itself declares, so it cannot drift from the fixture
//! the way a hand-copied walkthrough can.

use app::session::{apply_turn, DeathWatch, TurnResult};
use gvm::i7map::I7World;
use gvm::memory::Memory;
use gvm::objects::ParseNames;
use gvm::world::Compass;
use mapper::direction::{is_compass_command, Direction, WalkedDir};
use mapper::graph::RoomId;
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;

const STORY: &str = "CounterfeitMonkey-11.gblorb";

/// Slango's yacht, by printed name. `Brock's Stateroom` is `fore` of the Galley and is not one of
/// the ten the report's dump names, so it is left out of the specimen and turns up only as a
/// destination.
const YACHT: [&str; 9] = [
    "Sunning Deck",
    "Navigation Area",
    "Foredeck",
    "Crew Cabin",
    "Galley",
    "Your Bunk",
    "Your Head",
    "Slango's Bunk",
    "Slango's Head",
];

/// The eight direction columns Counterfeit Monkey declares itself, by printed name.
const NAUTICAL: [&str; 8] = [
    "Starboard",
    "port",
    "fore",
    "aft",
    "aft-port",
    "aft-starboard",
    "fore-port",
    "fore-starboard",
];

/// One declared passage: the room it leaves, the direction object's printed name (which is the
/// word a player types), and the room it reaches.
struct Passage {
    from: (RoomId, String),
    word: String,
    to: (RoomId, String),
    compass: Option<Compass>,
}

/// The story's own compiled map, with no turn played — [`I7World::detect`] over the boot image,
/// exactly as `GlulxSession` builds it.
fn world() -> Option<(Memory, ParseNames, I7World)> {
    let path = fixture_path(STORY);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let app::hints::LoadedStory::Glulx(image) =
        app::hints::extract_story(bytes).expect("CounterfeitMonkey-11.gblorb is a readable container")
    else {
        panic!("{STORY} is a Glulx story");
    };
    let mem = Memory::new(image).expect("the image loads");
    let names = ParseNames::detect(&mem).expect("an object table");
    let world = I7World::detect(&mem, &names).expect("Counterfeit Monkey has a compiled I7 map");
    Some((mem, names, world))
}

/// Every passage the yacht rooms declare, read off the compiled map.
fn yacht_passages(mem: &Memory, names: &ParseNames, w: &I7World) -> Vec<Passage> {
    let named = |addr: u32| w.printed_name(mem, names, addr);
    let mut out = Vec::new();
    for &room in w.rooms() {
        let Some(from) = named(room) else { continue };
        if !YACHT.contains(&from.as_str()) {
            continue;
        }
        for (compass, dir_obj, exit) in w.exits(mem, names, room) {
            let (Some(word), Some(dest)) = (named(dir_obj), exit.destination()) else { continue };
            let Some(to) = named(dest) else { continue };
            out.push(Passage {
                from: (app::roomid::glulx_room_id(room), from.clone()),
                word,
                to: (app::roomid::glulx_room_id(dest), to),
                compass,
            });
        }
    }
    // A stable order, so a failure names the same passage every run.
    out.sort_by(|a, b| (&a.from.1, &a.word).cmp(&(&b.from.1, &b.word)));
    out
}

/// GROUND TRUTH, and the suite's non-vacuity guard: the yacht really does declare its passages on
/// direction objects of its own, and really does declare NOTHING on the compass points those
/// project onto. Without both halves, nothing else here is about the reported bug.
#[test]
fn the_yacht_declares_its_passages_off_the_compass() {
    let Some((mem, names, w)) = world() else { return };
    let passages = yacht_passages(&mem, &names, &w);
    assert_eq!(
        passages.len(),
        17,
        "the nine yacht rooms declare seventeen passages between them: {:?}",
        passages.iter().map(|p| (&p.from.1, &p.word, &p.to.1)).collect::<Vec<_>>()
    );

    let mut nautical = 0;
    for p in &passages {
        match p.compass {
            // The only compass passages aboard are the two vertical pairs, and there the compass
            // word IS the ship's word — which is precisely why they were the only edges left
            // standing in the reported dump.
            Some(c) => assert!(
                matches!(c, Compass::Up | Compass::Down),
                "{} {} -> {} is on the compass, and only up/down should be",
                p.from.1,
                p.word,
                p.to.1
            ),
            None => {
                nautical += 1;
                assert!(
                    NAUTICAL.contains(&p.word.as_str()),
                    "{:?} is not one of Counterfeit Monkey's own direction objects",
                    p.word
                );
                // …and the slot the MAP projects that word onto declares nothing at all. This is
                // the `Absent` that armed the probe that erased the passage.
                let dir = WalkedDir::parse(&p.word).expect("the map understands the ship's words");
                let compass = compass_of(dir.dir());
                let col = w.compass_column(compass).expect("CM declares every compass point");
                let raw = w.raw_exit(&mem, room_addr(&w, &mem, &names, &p.from.1), col);
                assert!(
                    raw.is_none(),
                    "{} declares {:?} on {:?}, but its {compass:?} column names {raw:?} — the \
                     projection is the MAP's, and the story does not share it",
                    p.from.1,
                    p.word,
                    dir.dir()
                );
            }
        }
    }
    assert_eq!(nautical, 13, "thirteen of the seventeen are walked with a ship word");
}

/// Walking the yacht with the ship's own words leaves every passage DRAWN — no random pool at
/// all, and above all no pool naming the room it leaves.
///
/// The turn loop is `turn::finish_command_turn`'s, in its order: read the story's declared exit
/// for the command (rule 3: only for a compass word), then `apply_turn`, then decide whether the
/// move is worth a randomness probe. The probe itself is not armed here — proving that it is not
/// armed is the point.
#[test]
fn walking_the_yacht_with_the_ships_own_words_leaves_every_passage_drawn() {
    let Some((mem, names, w)) = world() else { return };
    let passages = yacht_passages(&mem, &names, &w);
    let mut mapper = Mapper::default();
    let mut death = DeathWatch::default();
    let mut trace: Vec<String> = Vec::new();
    // Every (room, direction) the story's own compass map was read for. For a nautical move it
    // must stay empty: that read is the `Absent` the erasing probe was armed on.
    let asked: std::cell::RefCell<Vec<(RoomId, Direction)>> = std::cell::RefCell::new(Vec::new());

    // Stand in the first room the route leaves.
    let first = &passages[0].from;
    apply_turn(&mut mapper, "", &TurnResult::observation(loc(first)), &mut death);

    for p in &passages {
        mapper.graph.set_current(p.from.0);
        let walked = WalkedDir::parse(&p.word).expect("every declared ship word parses");

        // ── turn.rs, step 1: what does the story declare for the command just typed? ──────────
        // The PRODUCTION gate, not a restatement of it: `declared_exit_for_command` decides
        // whether the compass column is even a question worth asking (SQ-1314 rule 3), and the
        // closure is the engine seam — here reading the compiled map straight, because no live
        // session can stand in a yacht room to be asked.
        let declared = app::random_exit_probe::declared_exit_for_command(
            &p.word,
            Some(p.from.0),
            |origin, d| {
                asked.borrow_mut().push((origin, d));
                let col = w.compass_column(compass_of(d)).expect("CM declares every compass point");
                match w.exit(&mem, &names, room_addr(&w, &mem, &names, &p.from.1), col) {
                    None => app::engine::DeclaredExit::Absent,
                    Some(e) => match e.destination() {
                        Some(r) => app::engine::DeclaredExit::Room(app::roomid::glulx_room_id(r)),
                        None => app::engine::DeclaredExit::Code,
                    },
                }
            },
        );
        if !is_compass_command(&p.word) {
            assert!(
                asked.borrow().is_empty(),
                "{} {} -> {}: the story's compass map was read for a move made off the compass — \
                 it answered {declared:?} about a passage nobody walked",
                p.from.1, p.word, p.to.1
            );
        }
        asked.borrow_mut().clear();

        // ── turn.rs, step 2: apply the turn ──────────────────────────────────────────────────
        let mut r = TurnResult::observation(loc(&p.to));
        r.transcript = format!("{}\n", p.to.1); // the game reprints the heading on arrival
        r.declared_exit = declared;
        apply_turn(&mut mapper, &p.word, &r, &mut death);

        // ── turn.rs, step 3: is this move worth a randomness probe? ──────────────────────────
        let worth_probing = p.to.0 != p.from.0
            && (mapper.graph.is_random_exit(p.from.0, walked.dir())
                || matches!(
                    declared,
                    Some(app::engine::DeclaredExit::Absent)
                        | Some(app::engine::DeclaredExit::Code)
                ));
        if !is_compass_command(&p.word) {
            assert!(
                !worth_probing,
                "{} {} -> {}: a ship word must not arm a randomness probe off a compass column \
                 the player never walked — that probe is what erased the passage",
                p.from.1, p.word, p.to.1
            );
        }
        assert!(
            mapper.take_random_exit_suspicion().is_none(),
            "{} {} -> {} raised a suspicion; nothing about it is suspicious",
            p.from.1, p.word, p.to.1
        );
        trace.push(format!("{} --{}--> {}", p.from.1, p.word, p.to.1));
    }

    // ── The headline: the reported damage, absent ────────────────────────────────────────────
    for room in mapper.graph.rooms() {
        for d in mapper::direction::UNTRIED_DIRS {
            let pool = mapper.graph.random_destinations(room.id, d);
            assert!(
                !pool.contains(&room.id),
                "{:?} {d:?} pools its OWN room — a refused move recorded as an arrival, which is \
                 the whole of SQ-1314 (pool {pool:?})",
                room.label()
            );
            assert!(
                pool.is_empty() && !mapper.graph.is_random_exit(room.id, d),
                "{:?} {:?} is marked random; nothing aboard the yacht varies (pool {:?})\n\
                 route:\n  {}",
                room.label(),
                d,
                pool,
                trace.join("\n  ")
            );
        }
    }

    // ── Every walked passage is still a drawn edge ───────────────────────────────────────────
    for p in &passages {
        let dir = WalkedDir::parse(&p.word).expect("parses").dir();
        assert_eq!(
            mapper
                .graph
                .connections()
                .iter()
                .find(|c| c.origin == p.from.0 && c.dir == dir)
                .map(|c| c.dest),
            Some(p.to.0),
            "{} --{}--> {} was walked and must still be drawn",
            p.from.1,
            p.word,
            p.to.1
        );
    }

    // ── And the reciprocal pairs the ship is made of ─────────────────────────────────────────
    // Galley aft-port ⇄ Slango's Bunk fore-starboard, and aft ⇄ fore below decks: the map draws
    // both halves, in opposite slots, exactly as the story declares them.
    let edge = |from: &str, word: &str| -> Option<String> {
        let p = passages.iter().find(|p| p.from.1 == from && p.word == word)?;
        let dir = WalkedDir::parse(word)?.dir();
        let c = mapper.graph.connections().iter().find(|c| c.origin == p.from.0 && c.dir == dir)?;
        Some(mapper.graph.room(c.dest)?.label().to_string())
    };
    assert_eq!(edge("Galley", "aft-port").as_deref(), Some("Slango's Bunk"));
    assert_eq!(edge("Slango's Bunk", "fore-starboard").as_deref(), Some("Galley"));
    assert_eq!(edge("Galley", "aft-starboard").as_deref(), Some("Your Bunk"));
    assert_eq!(edge("Your Bunk", "fore-port").as_deref(), Some("Galley"));
    assert_eq!(edge("Your Bunk", "aft").as_deref(), Some("Your Head"));
    assert_eq!(edge("Your Head", "fore").as_deref(), Some("Your Bunk"));
    assert_eq!(
        mapper::direction::opposite(Direction::SW),
        Direction::NE,
        "and the two halves sit in opposite slots, which is what makes the pair drawable"
    );
}

/// The room address for a printed name, for the two places above that need one.
fn room_addr(w: &I7World, mem: &Memory, names: &ParseNames, name: &str) -> u32 {
    *w.rooms()
        .iter()
        .find(|&&r| w.printed_name(mem, names, r).as_deref() == Some(name))
        .unwrap_or_else(|| panic!("{name:?} is one of Counterfeit Monkey's rooms"))
}

fn loc(room: &(RoomId, String)) -> app::engine::LocationInfo {
    app::engine::LocationInfo { number: room.0, parent: 0, name: room.1.clone() }
}

fn compass_of(d: Direction) -> Compass {
    match d {
        Direction::N => Compass::N,
        Direction::S => Compass::S,
        Direction::E => Compass::E,
        Direction::W => Compass::W,
        Direction::NE => Compass::Ne,
        Direction::NW => Compass::Nw,
        Direction::SE => Compass::Se,
        Direction::SW => Compass::Sw,
        Direction::Up => Compass::Up,
        Direction::Down => Compass::Down,
        Direction::In => Compass::In,
        Direction::Out => Compass::Out,
        Direction::Unknown => unreachable!("no declared passage has an unknown direction"),
    }
}
