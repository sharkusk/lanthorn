//! A fatal move must not leave the direction on the room's `tried` record (SQ-0671).
//!
//! Walking north into a grue mints no edge, so the matrix drew the direction as `_` — "tried, and
//! there is no path that way". That is a claim the turn never made: dying tells you nothing about
//! whether the passage is open. The record is rolled back and the cell stays `·`, on the
//! exploration frontier where it belongs.
//!
//! The timing is the interesting part. Most games print `*** You have died ***` on the turn that
//! kills you, but Adventure asks whether to reincarnate you first — so the banner (if it ever
//! comes) arrives on the turn that ANSWERS, and the rollback still has to undo the move that
//! killed the player, not whatever they typed at the prompt.
//!
//! …and the turn that gets you up again mints no passage either (SQ-0673). A death stays
//! outstanding until the game says how it ends, and the next room change on that side of it is the
//! resurrection — recognised by the death nobody has resolved, because the turn itself
//! ("--- POOF!! ---", and you are in the well house) says nothing about dying at all.

use mapper::direction::Direction;
use mapper::mapper::Mapper;

use app::session::{
    apply_turn, rollback_tried_on_death, tried_record_for, turn_reports_death, DeathWatch,
    TurnResult,
};

use crate::fixture_paths::fixture_path;

fn turn(num: u16, name: &str, transcript: &str) -> TurnResult {
    TurnResult {
        transcript: transcript.into(),
        transcript_runs: Vec::new(),
        location: Some(zvm::ObjectSnapshot { number: num, parent: 0, name: name.into() }),
        quit: false,
        erase_lower: false,
        info: None,
        sounds: Vec::new(),
        glulx_sound_ops: Vec::new(),
        diagnostics: Vec::new(),
        fault: None,
        location_method: None,
        pending_io: None,
        timed_out: false,
        pictures: Vec::new(),
        transcript_elems: Vec::new(),
        prose_retired: None,
        declared_exit: None,
    }
}

/// One turn of the run loop's mapping, exactly as `finish_command_turn` runs it: capture what the
/// turn is about to record, apply it, then roll back if the turn reported a death.
fn play(m: &mut Mapper, death: &mut DeathWatch, cmd: &str, r: &TurnResult) {
    let attempted = tried_record_for(m, cmd);
    apply_turn(m, cmd, r, death);
    rollback_tried_on_death(m, death, attempted, turn_reports_death(&r.transcript));
}

/// Every connection in the graph, as `(origin, direction, destination)` — the whole map's edges,
/// so a test can assert that a turn minted NOTHING rather than merely nothing it thought to name.
fn edges(m: &Mapper) -> Vec<(u16, Direction, u16)> {
    m.graph.connections().iter().map(|c| (c.origin, c.dir, c.dest)).collect()
}

const DEATH_BANNER: &str = "Oh, no! A lurking grue slithered into the room and devoured you!\n \n   ****  You have died  **** \n\nForest\n";

// ── The turn that kills you says so ───────────────────────────────────────────

#[test]
fn a_fatal_move_leaves_the_direction_untried_while_a_wall_still_records() {
    let mut m = Mapper::default();
    let mut death = DeathWatch::default();

    play(&mut m, &mut death, "", &turn(1, "Living Room", "Living Room\n"));
    play(&mut m, &mut death, "down", &turn(2, "Cellar", "Cellar\n"));

    // A move that bounces off a wall: the location does not change and no heading is reprinted.
    // That IS knowledge — the direction was tried and there is no way through — and must survive.
    play(&mut m, &mut death, "east", &turn(2, "Cellar", "You can't go that way.\n"));
    assert!(m.graph.is_tried(2, Direction::E), "a refused move is still a tried direction");

    // The fatal move: a grue eats the player and the game resurrects them in the Forest.
    play(&mut m, &mut death, "north", &turn(3, "Forest", DEATH_BANNER));

    assert!(
        !m.graph.is_tried(2, Direction::N),
        "the direction that killed the player is not on the record: {:?}",
        m.graph.room(2).unwrap().tried,
    );
    assert!(
        m.graph.untried(2).contains(&Direction::N),
        "…so it is still on the exploration frontier"
    );
    assert!(m.graph.is_tried(2, Direction::E), "and the wall to the east is untouched by that");
    assert!(
        !m.graph.connections().iter().any(|c| c.origin == 2 && c.dest == 3),
        "no passage was minted either (SQ-0259)"
    );
}

/// The rollback drops the TYPED record only. A direction that had already proved itself keeps its
/// passage, and stays tried on the strength of it.
#[test]
fn a_direction_that_already_led_somewhere_keeps_its_passage() {
    let mut m = Mapper::default();
    let mut death = DeathWatch::default();
    play(&mut m, &mut death, "", &turn(1, "Hall", "Hall\n"));
    play(&mut m, &mut death, "north", &turn(2, "Study", "Study\n"));
    play(&mut m, &mut death, "south", &turn(1, "Hall", "Hall\n"));
    // North again — and this time something in the Study kills the player.
    play(&mut m, &mut death, "north", &turn(3, "Forest", DEATH_BANNER));

    assert!(m.graph.is_tried(1, Direction::N), "the Hall's north passage is still known");
    assert!(
        m.graph.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::N && c.dest == 2),
        "and the edge that proves it was never touched"
    );
}

// ── The turn that kills you asks a question first ─────────────────────────────

/// Adventure's shape: the fatal turn offers to reincarnate you, and the banner only arrives if
/// you decline — several turns later, because "Please answer yes or no." is a turn of its own.
#[test]
fn a_death_admitted_turns_later_still_rolls_back_the_move_that_caused_it() {
    let mut m = Mapper::default();
    let mut death = DeathWatch::default();
    play(&mut m, &mut death, "", &turn(1, "In Cobble Crawl", "In Cobble Crawl\n"));
    play(&mut m, &mut death, "west", &turn(2, "Darkness", "Darkness\n"));

    // The fatal move. Adventure prints no banner here — just the offer.
    play(
        &mut m,
        &mut death,
        "up",
        &turn(
            2,
            "Darkness",
            "You fell into a pit and broke every bone in your body!\n\nOh dear, you seem to have \
             gotten yourself killed. I might be able to help you out, but I've never really done \
             this before. Do you want me to try to reincarnate you?\n",
        ),
    );
    assert!(
        !m.graph.is_tried(2, Direction::Up),
        "the offer is an admission of death: roll the move back on the spot"
    );

    // Now the same shape for a game that only admits it on the answer. Replay from a clean slate
    // with the offer text stripped of its death words, so the fatal turn is NOT recognised.
    let mut m = Mapper::default();
    let mut death = DeathWatch::default();
    play(&mut m, &mut death, "", &turn(1, "In Cobble Crawl", "In Cobble Crawl\n"));
    play(&mut m, &mut death, "west", &turn(2, "Darkness", "Darkness\n"));
    play(&mut m, &mut death, "up", &turn(2, "Darkness", "You fall a long way.\n"));
    assert!(m.graph.is_tried(2, Direction::Up), "nothing said death yet, so it reads as a probe");
    assert_eq!(
        death.pending_tried,
        Some((2, Direction::Up)),
        "…but the move is held, in case it was fatal"
    );

    // Two turns of the game insisting on an answer, then the banner.
    play(&mut m, &mut death, "look", &turn(2, "Darkness", "Please answer yes or no.\n"));
    play(&mut m, &mut death, "no", &turn(2, "Darkness", DEATH_BANNER));
    assert!(
        !m.graph.is_tried(2, Direction::Up),
        "the rollback belongs to the turn that CONTAINED the fatal move, not the answer"
    );
}

/// The held record is not held forever: once the player has walked out of the room they typed it
/// in, whatever it recorded is settled and a later death must not reach back for it.
#[test]
fn a_death_after_the_player_has_moved_on_does_not_reach_back() {
    let mut m = Mapper::default();
    let mut death = DeathWatch::default();
    play(&mut m, &mut death, "", &turn(1, "Hall", "Hall\n"));
    play(&mut m, &mut death, "east", &turn(1, "Hall", "You can't go that way.\n"));
    assert_eq!(death.pending_tried, Some((1, Direction::E)));

    play(&mut m, &mut death, "north", &turn(2, "Study", "Study\n"));
    play(&mut m, &mut death, "wait", &turn(2, "Study", "Time passes.\n"));
    assert_eq!(
        death.pending_tried,
        None,
        "the player left the Hall: its east wall is settled knowledge"
    );

    play(&mut m, &mut death, "wait", &turn(3, "Forest", DEATH_BANNER));
    assert!(m.graph.is_tried(1, Direction::E), "the Hall's east wall survived a later death");
}

// ── Getting up again ──────────────────────────────────────────────────────────
//
// SQ-0673. The resurrection is a turn of ordinary-looking text — Adventure's is `yes` →
// "--- POOF!! ---", and it reprints the destination's heading exactly like a walked move. Nothing
// in that turn says "death"; the only thing that knows is the death still outstanding from turns
// ago. Without it the mapper minted a `?` passage from wherever the corpse was to the well house.

/// Adventure's full shape, synthetically: the pit kills you, the game nags for an answer for as
/// many turns as it likes, then `yes` teleports you to a room across the map. That arrival is a
/// relocation — the current room moves, no edge is minted, and the destination is still on the map.
#[test]
fn a_resurrection_relocates_the_player_and_mints_no_passage() {
    let mut m = Mapper::default();
    let mut death = DeathWatch::default();
    play(&mut m, &mut death, "", &turn(1, "Inside Building", "Inside Building\n"));
    play(&mut m, &mut death, "out", &turn(2, "At End Of Road", "At End Of Road\n"));
    play(&mut m, &mut death, "down", &turn(3, "In Cobble Crawl", "In Cobble Crawl\n"));

    // The fatal move: no banner, just the offer. Adventure drops the corpse in a pit room.
    play(
        &mut m,
        &mut death,
        "west",
        &turn(
            4,
            "Darkness",
            "You fell into a pit and broke every bone in your body!\n\nOh dear, you seem to have \
             gotten yourself killed. Do you want me to try to reincarnate you?\n",
        ),
    );
    assert!(death.unresolved, "the death is outstanding until the game says how it ends");
    let after_death = edges(&m);

    // The game insists on an answer. These turns move nobody, so the death stays outstanding —
    // the watch has to survive an unbounded number of them.
    play(&mut m, &mut death, "maybe", &turn(4, "Darkness", "Please answer yes or no.\n"));
    play(&mut m, &mut death, "hmm", &turn(4, "Darkness", "Please answer yes or no.\n"));
    assert!(death.unresolved, "a prompt the player has not answered resolves nothing");

    // "yes" → POOF, and the player wakes up in the well house. The turn reprints that room's
    // heading, so without the watch this reads as a walked arrival and mints an edge.
    play(
        &mut m,
        &mut death,
        "yes",
        &turn(
            1,
            "Inside Building",
            "All right. But don't blame me if something goes wr......\n\n--- POOF!! ---\n\nYou \
             are engulfed in a cloud of orange smoke, and find that you're....\n\nInside \
             Building\nYou are inside a building, a well house for a large spring.\n",
        ),
    );

    assert_eq!(m.graph.current(), Some(1), "the resurrection moved the player to the well house");
    assert_eq!(
        edges(&m),
        after_death,
        "and minted nothing: the well house is not a passage out of the room you died in"
    );
    assert!(
        m.graph.room(4).is_some_and(|r| r.tried.is_empty()),
        "nor did it record a direction against the corpse's room"
    );
    assert!(!death.unresolved, "the resurrection resolved the death");
}

/// The watch suppresses exactly ONE relocation. The move after the resurrection is ordinary play
/// again and must mint its passage — a flag that stayed set would quietly stop mapping the game.
#[test]
fn the_watch_suppresses_one_relocation_and_not_the_next_move() {
    let mut m = Mapper::default();
    let mut death = DeathWatch::default();
    play(&mut m, &mut death, "", &turn(1, "Inside Building", "Inside Building\n"));
    play(&mut m, &mut death, "down", &turn(2, "Darkness", "Darkness\n"));
    play(
        &mut m,
        &mut death,
        "west",
        &turn(3, "Darkness", "You are dead. Shall I reincarnate you?\n"),
    );
    play(&mut m, &mut death, "yes", &turn(1, "Inside Building", "Inside Building\n"));
    assert!(!death.unresolved);

    // Back on their feet, the player walks out of the well house.
    play(&mut m, &mut death, "out", &turn(4, "At End Of Road", "At End Of Road\n"));
    assert!(
        edges(&m).contains(&(1, Direction::Out, 4)),
        "the first move after a resurrection maps like any other: {:?}",
        edges(&m)
    );

    // And a second death later still gets its own suppression — the watch re-arms.
    play(&mut m, &mut death, "north", &turn(5, "Darkness", "You are dead. Reincarnate you?\n"));
    play(&mut m, &mut death, "yes", &turn(1, "Inside Building", "Inside Building\n"));
    assert!(
        !edges(&m).iter().any(|&(o, _, d)| o == 5 && d == 1),
        "the second resurrection mints nothing either: {:?}",
        edges(&m)
    );
}

/// A death the player walks away from — games that kill you and leave you standing where you are —
/// is resolved by ordinary play resuming, not by a teleport that never comes. The next room the
/// player walks into is a room they WALKED into, and the passage has to be minted.
#[test]
fn a_death_the_player_keeps_playing_after_resolves_without_a_relocation() {
    let mut m = Mapper::default();
    let mut death = DeathWatch::default();
    play(&mut m, &mut death, "", &turn(1, "Hall", "Hall\n"));
    play(&mut m, &mut death, "north", &turn(2, "Chapel", "Chapel\n"));

    // Killed in place: the banner arrives, the room does not change.
    play(
        &mut m,
        &mut death,
        "pray",
        &turn(2, "Chapel", "A bolt of lightning strikes you.\n*** You have died ***\n"),
    );
    assert!(death.unresolved);

    // The player looks around and finds themselves still standing in the Chapel. That heading,
    // reprinted in the room they are already in, is ordinary play — the death is over.
    play(&mut m, &mut death, "look", &turn(2, "Chapel", "Chapel\nA quiet chapel.\n"));
    assert!(!death.unresolved, "a heading reprinted in place is the game carrying on");

    play(&mut m, &mut death, "east", &turn(3, "Crypt", "Crypt\n"));
    assert!(
        edges(&m).contains(&(2, Direction::E, 3)),
        "the passage they then walked is on the map: {:?}",
        edges(&m)
    );
}

// ── The real thing ────────────────────────────────────────────────────────────

/// Adventure, driven for real: walk into the dark below the grate and fall into the pit. The
/// killing move is `west` out of a room whose west has never been tried, so the rollback has
/// something to undo and the assertion cannot pass by accident.
///
/// The route is deterministic — no dwarves, no lamp, and the pit takes the second move made in
/// the dark. Skips vacuously without the gitignored story file.
/// Adventure, booted and walked to the edge of the pit: the lamp and keys taken, the grate
/// unlocked, and two moves made into the dark below it — the next `west` is the one that kills
/// you. `None` (with a SKIP note) without the gitignored story file.
///
/// The route is deterministic: no dwarves this early, the lamp is never lit, and the pit takes the
/// second move made in the dark.
fn adventure_at_the_pit() -> Option<(app::glulx_session::GlulxSession, Mapper, DeathWatch)> {
    use app::engine::Engine;
    use app::glulx_session::GlulxSession;

    let path = fixture_path("advent.blb");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let blorb = blorb::Blorb::parse(bytes).expect("advent.blb parses as a Blorb");
    let (_k, exec) = blorb.executable().expect("advent.blb carries an executable chunk");
    let mut s = GlulxSession::new(exec.to_vec(), 80, 24, true, false, false, (1, 1), None, &[])
        .expect("Adventure (Glulx) boots");

    let mut m = Mapper::default();
    let mut death = DeathWatch::default();
    let approach = [
        "in", "get lamp", "get keys", "out", "south", "south", "south",
        "unlock grate with keys", "open grate", "down", "west", "west", "up",
    ];
    for cmd in approach {
        let r = Engine::submit(&mut s, cmd);
        play(&mut m, &mut death, cmd, &r);
    }
    Some((s, m, death))
}

#[test]
fn adventures_pit_leaves_the_direction_untried() {
    use app::engine::Engine;

    let Some((mut s, mut m, mut death)) = adventure_at_the_pit() else { return };
    // Standing in the dark, one move from the pit, with west untried from here.
    let here = m.graph.current().expect("the mapper is following the player");
    assert!(!m.graph.is_tried(here, Direction::W), "west is fresh in this room");
    let elsewhere: Vec<_> = m
        .graph
        .rooms()
        .filter(|r| r.id != here && !r.tried.is_empty())
        .map(|r| (r.id, r.tried.clone()))
        .collect();
    assert!(!elsewhere.is_empty(), "ordinary moves are being recorded as tried");

    let fatal = Engine::submit(&mut s, "west");
    assert!(
        fatal.transcript.contains("broke every bone"),
        "expected the pit death, got: {}",
        fatal.transcript
    );
    assert!(turn_reports_death(&fatal.transcript), "Adventure's death must be recognised as one");
    play(&mut m, &mut death, "west", &fatal);

    assert!(
        !m.graph.is_tried(here, Direction::W),
        "the move that killed the player is not recorded as tried: {:?}",
        m.graph.room(here).unwrap().tried,
    );
    assert!(
        !m.graph.connections().iter().any(|c| c.origin == here && c.dir == Direction::W),
        "and no passage was minted to wherever the pit dropped them"
    );
    // Every other room's record is exactly as it was: the rollback is one direction in one room.
    let after: Vec<_> = m
        .graph
        .rooms()
        .filter(|r| r.id != here && !r.tried.is_empty())
        .map(|r| (r.id, r.tried.clone()))
        .collect();
    assert_eq!(after, elsewhere, "normal moves elsewhere still record their directions");
}

/// The other half of the same death, driven for real: answer the reincarnation offer with `yes`
/// and Adventure prints *"--- POOF!! ---"* and puts the player back in the well house, hundreds of
/// feet and several rooms away from the pit they died in (SQ-0673).
///
/// That turn says nothing about death and reprints the well house's heading exactly like a walked
/// move, so before the death watch it minted a `?` passage from the corpse's room to the well
/// house — an edge the player could never walk. Skips vacuously without the story file.
#[test]
fn adventures_resurrection_mints_no_passage_from_the_corpse() {
    use app::engine::Engine;

    let Some((mut s, mut m, mut death)) = adventure_at_the_pit() else { return };
    let fatal = Engine::submit(&mut s, "west");
    assert!(fatal.transcript.contains("broke every bone"), "expected the pit death");
    play(&mut m, &mut death, "west", &fatal);

    let corpse = m.graph.current().expect("the mapper followed the player into the pit");
    let before = edges(&m);
    assert!(death.unresolved, "the offer left a death outstanding");

    // Accept. Adventure answers immediately — no "Please answer yes or no." on `yes` — but the
    // watch is what makes this turn recognisable, not its text.
    let poof = Engine::submit(&mut s, "yes");
    assert!(
        poof.transcript.contains("POOF"),
        "expected the resurrection, got: {}",
        poof.transcript
    );
    assert!(
        !turn_reports_death(&poof.transcript),
        "the POOF turn carries no death vocabulary — that is the whole problem: {}",
        poof.transcript
    );
    play(&mut m, &mut death, "yes", &poof);

    let revived = m.graph.current().expect("the player is somewhere after the smoke clears");
    assert_ne!(revived, corpse, "the resurrection moved the player out of the pit");
    assert_eq!(
        m.graph.room(revived).map(|r| r.name.as_str()),
        Some("Inside Building"),
        "…and into the well house"
    );
    assert_eq!(
        edges(&m),
        before,
        "the relocation minted NO connection — not from the corpse's room, not anywhere"
    );
    assert!(
        !m.graph.connections().iter().any(|c| {
            (c.origin == corpse && c.dest == revived) || (c.origin == revived && c.dest == corpse)
        }),
        "and above all no passage between the room you died in and the one you woke up in"
    );
    assert!(!death.unresolved, "the resurrection resolved the death");

    // Ordinary play resumes: walking out of the well house maps as it always did.
    let out = Engine::submit(&mut s, "out");
    play(&mut m, &mut death, "out", &out);
    assert!(
        edges(&m).iter().any(|&(o, d, _)| o == revived && d == Direction::Out),
        "the first move after the resurrection is mapped normally: {:?}",
        edges(&m)
    );
}
