//! After a move, find the way BACK — in a silent shadow of the game (SQ-0785).
//!
//! ```text
//! > enter window
//! Kitchen
//! ```
//!
//! …and the map now knows that `east` returns you to Behind House, without
//! anybody having typed it, and without claiming that `west` works from Behind
//! House. Two different facts, and the second one is a lie.
//!
//! # The gap this closes, and the one it must not invent
//!
//! An automap built from a player's moves only ever learns one direction of a
//! passage at a time. Half the rooms on a map you have walked through once are
//! joined by a single arrow, and the layout, the routing and the click-to-route
//! all reason about a graph that is thinner than the world it describes.
//!
//! The obvious fix is to assume passages reciprocate. They do not: these games
//! are full of one-way drops, doors that only open from one side, and mazes
//! whose whole design is that the way back is not the way you came. Guessing
//! wrong writes an edge that does not exist, and a wrong edge is worse than the
//! missing one it replaced — it is the map asserting something false, and the
//! player has no way to tell which arrows were observed and which were assumed.
//!
//! So the way back is **discovered**, in a copy of the game that costs nothing:
//! [`crate::probe`]'s shadow is restored to exactly where the player is standing
//! and asked to walk one direction. If it comes out in a room the map already
//! holds, that passage is real and goes on the map. If it comes out anywhere
//! else, nothing at all is recorded.
//!
//! # Success is room identity, not a room
//!
//! **Landing somewhere is not landing back.** The test is `step.location`: the
//! mapper's own location detection, the same `snap.number`
//! [`crate::session::apply_turn`] keys rooms by. The search ENDS when that number
//! is the room the player came from; a landing anywhere else is a different
//! question's answer and leaves the search running.
//!
//! **A v4+ story tells the interpreter where it is through the STATUS LINE**, and
//! Quetzal archives no screen — so a shadow restored into the player's moment
//! used to inherit the previous probe's status line, and a story that repaints
//! only as many columns as its new room name needs left the tail of the longer
//! one behind. Zork I's shadow read `Forest Pathse`, which matches no object; the
//! ladder fell off `PlayerParent` onto the text rung and `resolve_room_object`
//! prefix-matched object 1 — the scenery object named `forest` — so a real return
//! path was discarded as a landing in the wrong room. `restore_state` now blanks
//! the upper window, because memory restored without a screen must not be read
//! against another moment's screen (SQ-0785).
//!
//! That is also why this consumer needs none of the probe seam's
//! [`crate::probe::Refusals`] machinery. A vocabulary offer has to read the
//! story's prose to find out whether anything happened, because "did this verb
//! do something" is only answerable in words. "Am I back where I started" is
//! answerable in a room number.
//!
//! **A probe that lands in a room the map does NOT hold records NOTHING — not
//! even the attempt** (SQ-1292). Not room C, not the edge to it, not its
//! existence, and not "this direction is spent". The map is a record of what the
//! PLAYER has seen, and keeping C "known but hidden" would leak straight back out
//! through the layout, the pathfinder and click-to-route. Total failure likewise
//! says nothing about the map: it proves only that these directions did not work
//! from here, this time. A door may need opening, and a one-way passage is a real
//! and beloved part of these games.
//!
//! The attempt itself is withheld for the same reason the room is. `probed` is
//! read forever after by [`mapper::graph::MapGraph::probe_candidates`], which
//! never offers a direction it holds — so a mark is permanent, and it has to
//! state a fact about the WORLD rather than about the map's coverage at one
//! instant. "Wherever that goes, the player has not been there yet" is the second
//! kind, and it stops being true the moment they walk in. Marking it anyway spent
//! the direction for good: Zork I's forest and cellar rooms finish a playthrough
//! with all twelve marked, and every later arrival there finds no way back until
//! the player walks it. A REFUSED move is not affected — it names no room at all
//! ("The windows are all boarded" moves nobody, so the step reports no location),
//! which is as informative as it will ever be, so it is remembered and never
//! re-asked. Only a landing the map could not READ is held open.
//!
//! **But a room the map ALREADY HOLDS is a room the player has stood in**, and a
//! passage between two such rooms reveals nothing unseen — so it is recorded even
//! though it is not the answer the search was after, and the search carries on
//! looking for the one that is. That is not a bonus, it is the fix for a defect
//! the narrower rule caused (SQ-0785): [`mapper::graph::Room::probed`] says "this
//! direction was walked from here", but the answer it stood for was "…and it did
//! not reach THAT origin". Reused against a different origin it SUPPRESSED the
//! right answer. Zork I's South of House was probed westward while the search was
//! asking about Behind House, reached West of House, and threw that away; on a
//! later visit — with the player now arriving FROM West of House — `W` was
//! already on the probed record, so the first surviving candidate was the
//! diagonal `NW`, and the map recorded a diagonal where a cardinal was known.
//! Recording every landing on a known room closes the gap on the first visit, so
//! the second one has no gap left to ask about.
//!
//! # Two records, and why they must never merge
//!
//! [`mapper::graph::Room::tried`] is what the PLAYER has typed here, and
//! `untried()` turns it into the exits the map still offers as unexplored.
//! [`mapper::graph::Room::probed`] is what the SEARCH has walked. Marking a probe
//! as tried would take a genuine unexplored exit off the map and quietly steer
//! the player away from content they have never seen.
//!
//! Nothing in this module reads either list directly:
//! [`mapper::graph::MapGraph::probe_candidates`] is the single accessor that
//! consults both and applies the priority order, so the rule about which record
//! means what lives in one place rather than being re-derived by every caller.
//!
//! # The order it tries, and why it starts wide
//!
//! The way back is overwhelmingly the way you came, so the search leads with
//! `opposite(D)`, then the two perpendiculars, then the two diagonals beside the
//! opposite, then everything else — the eight compass points, and never a
//! portal that was not the seed itself (see below). Starting wide is
//! deliberate: narrowing the list further is a measurement decision and there
//! was no measurement yet. Every attempt that is answered is recorded
//! permanently, so the cost of a wide list is paid once per room in the life
//! of a map, not once per visit.
//!
//! **Up/Down/In/Out are asked only as the direct reciprocal of a portal move
//! the player just made** (SQ-1290) — climb down and the seed is Up, walk in
//! and the seed is Out — never as a blind fallback once the compass words run
//! out. A search that did not just cross a portal has no business revealing
//! one the player has not walked: on an ordinary compass map the only way
//! back from some room may genuinely be `up`, and finding that and drawing it
//! before the player has ever gone up is exactly what this search must not
//! do. See [`mapper::direction::PROBE_FALLBACK_DIRS`].
//!
//! **And the reciprocal is asked in the player's OWN words, when their move
//! belongs to a vocabulary family the compass does not cover.** After `fore`
//! the way back is `aft`, not the compass `south` — both fill the same
//! [`mapper::direction::Direction::S`] slot, but a story that models
//! FORE/AFT/PORT/STARBOARD as exits distinct from the compass (Shogun) refuses
//! the compass word and answers only the nautical one. See
//! [`mapper::direction::reciprocal_word`].
//!
//! # On the worker, and why staleness does not apply
//!
//! The search runs one attempt at a time through [`crate::probe::ShadowProbe`],
//! which lives on its own thread and holds one question at a time. Two
//! consequences of SQ-1124's threading deliberately do NOT carry over:
//!
//! **A return-path result is never stale.** A vocabulary suggestion is worthless
//! once the player has typed again, so SQ-1124 drops any answer whose
//! `turn_epoch` has moved. *"South from the Kitchen returns to Behind House"* is
//! true wherever the player has wandered since — it is a fact about the map, not
//! about this turn — so an answer arriving three moves later is recorded exactly
//! as one arriving immediately.
//!
//! **A new MOVE aborts the search**, though, because the move may itself be the
//! walk back, which records the true edge for free and makes the search moot. A
//! turn that does not move the player (`look`, `take lamp`, a refused direction)
//! leaves the search running.
//!
//! Aborting is cheap because progress is durable: every ANSWERED attempt marks
//! the probed record, so the next visit resumes where this one stopped instead of
//! starting over. Two things are deliberately not carried. The attempt that was
//! IN FLIGHT when the abort came — its answer was never read, so nothing was
//! learned about it. And any attempt that came out where the map could not name
//! it (above): that one is offered again on a later visit precisely because the
//! map may by then be able to.
//!
//! # Sharing one shadow with the vocabulary offer
//!
//! [`crate::probe::ShadowProbe`] holds one question at a time, and
//! [`crate::vocab`] asks it too. When the shadow is busy the search simply does
//! not ask this pass and tries again on the next one. It cannot starve on a slow
//! game the way a vocabulary offer can, for the reason above: it is not tied to a
//! turn, so waiting costs it nothing but time.

use mapper::direction::{long_label, Direction};
use mapper::graph::RoomId;
use mapper::mapper::{Mapper, ProbedPassage};

use crate::engine::Engine;
use crate::state::AppState;

/// One direction out with the worker: what was asked, and the token it will
/// answer under.
#[derive(Debug, Clone, Copy)]
struct Attempt {
    token: u64,
    dir: Direction,
}

/// A search for the way back from one room to another, in progress.
///
/// Session state and never persisted — what IS persisted is everything it
/// learns, in the graph's own two records. A restore begins with no search
/// running and picks up wherever the probed record left off.
#[derive(Debug)]
pub struct ReturnSearch {
    /// The room the player came FROM: the room a probe has to land in to succeed.
    origin: RoomId,
    /// The room the player is standing in, and the room every attempt starts from.
    /// The search ends the moment this stops being where they are.
    here: RoomId,
    /// The directions still to try, best first, each paired with the WORD that attempt sends —
    /// [`MapGraph::probe_candidates`]'s order, taken once when the search is armed, with one word
    /// substituted (SQ-1290): see [`arm_return_search`]. The vocabulary is a property of the
    /// candidate, not a special case in the pump loop — every entry already carries the exact
    /// command to send, so [`pump_return_search`] never re-derives one from the direction.
    ///
    /// [`MapGraph::probe_candidates`]: mapper::graph::MapGraph::probe_candidates
    queue: Vec<(Direction, &'static str)>,
    /// The attempt out with the worker, if any.
    attempt: Option<Attempt>,
    /// The moment every attempt is asked from: the live game as it stood the
    /// instant the player arrived here.
    ///
    /// **Taken once, for the whole search.** Attempts go out one at a time so
    /// each answer is durable, and [`crate::probe::ShadowProbe::ask`] would
    /// otherwise charge the player's thread for a host snapshot per attempt —
    /// 102 ms each on Counterfeit Monkey in a debug build, twelve times over.
    /// One snapshot is 102 ms once, and the answer is about the map rather than
    /// about this instant. See [`crate::probe::ShadowProbe::snapshot`].
    from: crate::probe::ProbeSnapshot,
}

impl ReturnSearch {
    /// The room the search runs from, for tests and diagnostics.
    pub fn here(&self) -> RoomId {
        self.here
    }

    /// The room it is trying to reach.
    pub fn origin(&self) -> RoomId {
        self.origin
    }

    /// How many directions it has left to try, the one in flight excluded.
    pub fn remaining(&self) -> usize {
        self.queue.len()
    }
}

/// Start a search for the way back, if this turn earned one (SQ-0785).
///
/// Called once per turn, after `apply_turn` has settled where the player is.
/// `room_before` is what `mapper.graph.current()` said BEFORE that call — the
/// only moment it is knowable.
///
/// It is also where a running search is ABORTED: a turn that moved the player
/// ends whatever was in flight, because the move may be the walk back and the
/// search from the old room is about a room nobody is standing in any more.
///
/// The gate, in order:
///
/// * the feature is on for this game;
/// * the probe seam is armed (a session that never kept the story bytes has no
///   shadow to fork);
/// * the player is in a room, came from a different room, and a passage joins
///   them the way they went — so a death, a teleport and a refused move all
///   arm nothing, having crossed nothing;
/// * and **the map does not already know a way back**. That is the whole point:
///   with a return path recorded there is no gap to close, and the cheapest
///   probe is the one not run.
pub fn arm_return_search(
    state: &mut AppState,
    mapper: &Mapper,
    live: &dyn Engine,
    cmd: &str,
    room_before: Option<RoomId>,
    turn_save: &mut crate::engine::TurnSave,
) {
    let here = mapper.graph.current();
    // A move ends any search that was running: the room it was asking about is
    // behind us, and the move itself may have been the answer.
    if state.return_search.as_ref().is_some_and(|s| Some(s.here) != here) {
        state.return_search = None;
    }
    if !state.config.return_probe || !state.probe.is_armed() {
        return;
    }
    let (Some(here), Some(origin)) = (here, room_before) else { return };
    if here == origin {
        return; // no crossing this turn
    }
    // Only a passage the map actually holds is worth asking about the reverse of.
    // A relocation (death, teleport) mints no edge and must arm nothing.
    if !mapper.graph.connections().iter().any(|c| c.origin == origin && c.dest == here) {
        return;
    }
    // THE GATE. A known return path means there is no gap, and nothing to do.
    if mapper.graph.connections().iter().any(|c| c.origin == here && c.dest == origin) {
        return;
    }
    let candidates = mapper.graph.probe_candidates(here, mapper::direction::parse_direction(cmd));
    if candidates.is_empty() {
        return;
    }
    let mut queue: Vec<(Direction, &'static str)> =
        candidates.iter().map(|&d| (d, long_label(d))).collect();
    // SQ-1290: ask the way back in the player's OWN vocabulary first. After "fore" the way back
    // is overwhelmingly "aft", not the compass "south" — both fill the same slot
    // ([`mapper::direction::reciprocal_word`]'s second element), but a story that models
    // FORE/AFT/PORT/STARBOARD as exits distinct from the compass (Shogun) refuses the compass
    // word and answers only the nautical one. Only when that slot survived
    // `probe_candidates`'s own tried/probed filter — respecting the same "never re-ask a
    // direction already spent" rule as every other candidate, not a special case for this one.
    if let Some((word, dir)) = mapper::direction::reciprocal_word(cmd) {
        if candidates.contains(&dir) {
            queue.insert(0, (dir, word));
        }
    }
    // The one snapshot the whole search runs from, and the one thing here the
    // player's thread pays for. Taken now rather than per attempt — and shared
    // with this turn's history capture and auto-save (SQ-1178), so arming a
    // search costs no extra save_state when either of those already paid.
    let Some(from) = state.probe.snapshot_from(live, || turn_save.get(live)) else { return };
    queue.reverse(); // popped from the back, so the best candidate goes last
    state.return_search = Some(ReturnSearch { origin, here, queue, attempt: None, from });
}

/// Hand the next candidate to the worker, if there is one and the shadow is
/// free. Called every pass of the event loop; returns true when something was
/// asked (nothing to redraw, but the caller may want to know).
///
/// The shadow is shared with the vocabulary offer and holds one question at a
/// time, so "busy" is an ordinary outcome and simply means try again next pass.
pub fn pump_return_search(state: &mut AppState) -> bool {
    let Some(search) = &state.return_search else { return false };
    if search.attempt.is_some() {
        return false; // one out already
    }
    let Some(&(dir, word)) = search.queue.last() else {
        // Nothing left to try. Total failure records nothing about the map: a
        // door may need opening, and a one-way passage is a real answer.
        state.return_search = None;
        return false;
    };
    let Some(token) = state.probe.ask_from(&search.from, &[word.to_string()]) else {
        return false; // busy, unarmed, or mid-save — ask again next pass
    };
    if let Some(search) = &mut state.return_search {
        search.queue.pop();
        search.attempt = Some(Attempt { token, dir });
    }
    true
}

/// True when `token` answers a question this search asked.
pub fn owns(state: &AppState, token: u64) -> bool {
    state.return_search.as_ref().and_then(|s| s.attempt).is_some_and(|a| a.token == token)
}

/// Read one answer back, and record what it found (SQ-0785).
///
/// Returns true when the map changed, which is what tells the event loop to
/// bump the graph generation and redraw.
///
/// Four outcomes, in the order they are decided:
///
/// 1. **It came out in a room the map already holds** — the passage is real, and
///    goes on the map through the same call a walked crossing makes.
///    [`Mapper::record_probed_passage`] is what enforces the no-leak rule: it
///    refuses a room the map does not have, so an unvisited room cannot arrive
///    this way however the probe lands.
/// 2. **…and the attempt is recorded as probed** — but ONLY here, because only an
///    ANSWERED attempt is spent (SQ-1292). See the comment at the mark itself: a
///    landing the map cannot name says nothing permanent, and remembering it as
///    spent is what stopped a room from ever learning its way back.
/// 3. **…and if that room is the one the player LEFT, the search is over.**
///    Otherwise it keeps going: the gap it was opened to close is still open, and
///    what it just recorded is a different question's answer (SQ-0785).
/// 4. **Anything else** — an unknown room, nowhere at all, a death, a story that
///    ended, an engine that cannot say where it is — and nothing is recorded at
///    all, the attempt included. The search moves on to the next direction, and a
///    later visit may ask this one again.
pub fn deliver(
    state: &mut AppState,
    mapper: &mut Mapper,
    answer: &crate::probe::Answer,
) -> Option<ProbedPassage> {
    let search = state.return_search.as_mut()?;
    let attempt = search.attempt.filter(|a| a.token == answer.token)?;
    search.attempt = None;
    let (here, origin) = (search.here, search.origin);

    // (1) WHERE did it come out? Room identity and nothing else — a step that
    // ended the story or reached for a file answers nothing about the map,
    // whatever `location` happens to hold.
    let landed = answer.run.as_ref().and_then(|run| {
        run.steps.first().filter(|s| !s.quit && !s.escaped).and_then(|s| s.location)
    });
    // (2) The attempt is spent unless it was ANSWERED — when the shadow came
    // out somewhere the map can name (SQ-1292). `probed` is consulted forever
    // after by `MapGraph::probe_candidates`, which never offers a direction it
    // holds, so a mark written here is permanent: it must therefore record a
    // fact about the WORLD ("this way leads there", or "this way is refused"),
    // never one about the map's coverage at this instant.
    //
    // A landing in a room the map does not hold is the second kind. It says only
    // "wherever that goes, the player has not been there YET" — which stops being
    // true the moment they walk in, and by then the direction is on the record and
    // can never be asked again. That is the reported defect: Zork I's forest and
    // cellar rooms end a playthrough with every one of the twelve directions
    // marked, so every later arrival there finds no way back until the player
    // walks it themselves. And it is DIRECTION-SHAPED, which is how it was seen:
    // a failing search burns the cardinals first (the seed, the two
    // perpendiculars, then the head of `PROBE_FALLBACK_DIRS`), the diagonals only
    // if it gets that far, and — since SQ-1290 took portals out of that fallback —
    // Up/Down/In/Out never at all. So the way back showed up reliably for a
    // staircase, usually for a diagonal, and not until walked for a compass exit.
    //
    // A move that named NO room still burns, exactly as it always did — a refusal
    // (which moves nobody, so the step reports no location at all), a death, a
    // story that ended. Those are as informative as they will ever be, and
    // re-asking them every visit would buy nothing. The one attempt withheld is
    // the one whose answer the map could not READ yet, and it is offered again on
    // a later visit by which time it may be able to.
    let unnameable = landed.is_some_and(|r| mapper.graph.room(r).is_none());
    if !unnameable {
        mapper.graph.mark_probed(here, attempt.dir);
    }
    let Some(landed) = landed.filter(|_| !unnameable) else {
        return None; // no room, or none this map can name: nothing is recorded.
    };

    // (3) A room the map already holds is a room the PLAYER has stood in, so the
    // passage to it can be drawn without revealing anything unseen — whether or
    // not it is the room this search was asking about. `record_probed_passage`
    // refuses an unknown room itself, so the no-leak rule lives in one place
    // rather than being restated here. It also refuses `from == to` (a refused
    // move, where the player never left) and a direction already leaving `from`
    // (the player walked it back while this was in flight, and a real traversal
    // is the better authority on its own passage).
    let passage = ProbedPassage { from: here, dir: attempt.dir, to: landed };
    let recorded = mapper.record_probed_passage(passage);

    // (4) …but the SEARCH ends only on the room it was opened to find, and it
    // ends whether or not the graph took the edge — with the player back there
    // by their own move there is no gap left to close either way. A landing
    // anywhere else leaves it running: what was just recorded is a different
    // question's answer, and this question is still open.
    if landed == origin {
        state.return_search = None;
    }
    recorded.then_some(passage)
}

/// Run a search to its end, waiting for each answer instead of collecting one
/// that has already arrived — the whole search in one call.
///
/// **Not for the event loop.** It is what a test harness and a measurement
/// harness need: the answer without racing the thread, and the shadow's own
/// `probes`/`spent` counters left holding the cost of the whole search.
///
/// Returns the passage back to the ORIGIN if one was found. Edges to other rooms
/// the map already holds are recorded as they turn up and do not end the search,
/// so a caller wanting every change the run made should read the graph rather
/// than this value. Bounded by the candidate list, so it terminates whatever the
/// story does; a shadow that will not answer at all ends it by breaking the seam,
/// which [`crate::probe::ShadowProbe::settle`] reports as `None`.
pub fn settle_return_search(state: &mut AppState, mapper: &mut Mapper) -> Option<ProbedPassage> {
    while state.return_search.is_some() {
        if !pump_return_search(state) {
            // Nothing was asked: either the search just ended, or the shadow is
            // busy with somebody else's question — and in a harness there is
            // nobody else, so this is the seam refusing and the search is over.
            if state.return_search.as_ref().is_some_and(|s| s.attempt.is_none()) {
                state.return_search = None;
            }
            if state.return_search.is_none() {
                break;
            }
        }
        let Some(answer) = state.probe.settle() else { break };
        if !owns(state, answer.token) {
            continue; // somebody else's, and nobody is here to want it
        }
        let passage = deliver(state, mapper, &answer);
        // `deliver` clears the search only on the room it was asking about, so an
        // empty `return_search` here IS the end — and the passage has to be
        // carried out of the loop rather than left to the `while`, which would
        // drop it. A landing on some OTHER known room records its edge and leaves
        // the search running, which is the whole point (SQ-0785).
        if state.return_search.is_none() {
            return passage;
        }
    }
    None
}

#[cfg(all(test, feature = "t-session"))]
mod tests {
    use super::*;

    /// A real engine, because arming takes a snapshot of one. `tiny_cave.dat` is
    /// the smallest story in the repo and is freely redistributable, so these
    /// cases never skip.
    fn blind() -> crate::scott_session::ScottSession {
        let bytes = include_bytes!("../../scott/tests/tiny_cave.dat").to_vec();
        crate::scott_session::ScottSession::new(bytes, None).expect("tiny_cave.dat loads")
    }

    fn armed_state() -> AppState {
        let mut state = AppState::default();
        state.config.return_probe = true;
        state.probe.arm(crate::probe::ShadowRecipe {
            story_bytes: std::sync::Arc::new(
                include_bytes!("../../scott/tests/tiny_cave.dat").to_vec(),
            ),
            ..Default::default()
        });
        state
    }

    fn walked(m: &mut Mapper) {
        m.observe(1, "Behind House", None);
        m.observe(2, "Kitchen", Some(Direction::In));
    }

    /// The gate: a crossing with no way back arms a search, and the same crossing
    /// with a way back already on the map arms nothing at all.
    #[test]
    fn a_known_return_path_means_no_search() {
        let mut m = Mapper::default();
        walked(&mut m);
        let mut state = armed_state();
        arm_return_search(&mut state, &m, &blind(), "enter window", Some(1), &mut crate::engine::TurnSave::default());
        let s = state.return_search.as_ref().expect("a gap to close");
        assert_eq!((s.here(), s.origin()), (2, 1));
        // SQ-1290: Out (the seeded portal reciprocal of `enter`), plus the eight compass
        // points — nine, not twelve, since Up/Down/In/Out no longer fall through as a
        // blind fallback (`PROBE_FALLBACK_DIRS` carries only the compass eight).
        assert_eq!(s.remaining(), 9, "the seeded portal reciprocal, then all eight compass points");

        // Now the player walks back themselves, and the gap is gone.
        m.observe(1, "Behind House", Some(Direction::E));
        m.observe(2, "Kitchen", Some(Direction::In));
        let mut state = armed_state();
        arm_return_search(&mut state, &m, &blind(), "enter window", Some(1), &mut crate::engine::TurnSave::default());
        assert!(state.return_search.is_none(), "no gap, no probe");
    }

    /// Nothing arms without a crossing: a turn that did not move the player, and
    /// a relocation that minted no passage (a death, a teleport), both of which
    /// leave `current` changed but no edge behind them.
    #[test]
    fn only_a_real_crossing_arms_a_search() {
        let mut m = Mapper::default();
        walked(&mut m);
        let mut state = armed_state();

        arm_return_search(&mut state, &m, &blind(), "look", Some(2), &mut crate::engine::TurnSave::default());
        assert!(state.return_search.is_none(), "the player did not cross anything");

        m.observe_relocation(3, "Forest");
        arm_return_search(&mut state, &m, &blind(), "north", Some(2), &mut crate::engine::TurnSave::default());
        assert!(state.return_search.is_none(), "a relocation walked no passage");
    }

    /// Off is off, and an unarmed seam probes nothing — the default state of
    /// every test-built `AppState`, and of any session with no story bytes kept.
    #[test]
    fn the_switch_and_the_seam_both_have_to_be_on() {
        let mut m = Mapper::default();
        walked(&mut m);

        let mut off = armed_state();
        off.config.return_probe = false;
        arm_return_search(&mut off, &m, &blind(), "enter window", Some(1), &mut crate::engine::TurnSave::default());
        assert!(off.return_search.is_none());

        let mut unarmed = AppState::default();
        unarmed.config.return_probe = true;
        assert!(!unarmed.probe.is_armed());
        arm_return_search(&mut unarmed, &m, &blind(), "enter window", Some(1), &mut crate::engine::TurnSave::default());
        assert!(unarmed.return_search.is_none());
    }

    /// A MOVE ends the search; a turn that does not move the player leaves it
    /// running, because the room it is asking about is still the room they are in.
    #[test]
    fn a_move_aborts_the_search_and_a_still_turn_does_not() {
        let mut m = Mapper::default();
        walked(&mut m);
        let mut state = armed_state();
        arm_return_search(&mut state, &m, &blind(), "enter window", Some(1), &mut crate::engine::TurnSave::default());
        assert!(state.return_search.is_some());

        arm_return_search(&mut state, &m, &blind(), "take lamp", Some(2), &mut crate::engine::TurnSave::default());
        assert!(state.return_search.is_some(), "a still turn leaves it alone");

        m.observe(3, "Attic", Some(Direction::Up));
        arm_return_search(&mut state, &m, &blind(), "up", Some(2), &mut crate::engine::TurnSave::default());
        let s = state.return_search.as_ref().expect("a fresh search from the new room");
        assert_eq!((s.here(), s.origin()), (3, 2), "the old one is gone, not resumed");
    }

    /// Every answered attempt is marked before anything is judged, so a search
    /// that is aborted mid-way resumes from where it stopped rather than
    /// re-walking ground it has covered.
    #[test]
    fn an_answered_attempt_is_durable_and_the_next_search_resumes() {
        let mut m = Mapper::default();
        walked(&mut m);
        m.graph.mark_probed(2, Direction::Out); // an earlier search got this far
        m.graph.mark_probed(2, Direction::N);
        let mut state = armed_state();
        arm_return_search(&mut state, &m, &blind(), "enter window", Some(1), &mut crate::engine::TurnSave::default());
        let s = state.return_search.as_ref().expect("still worth asking");
        // SQ-1290: nine to begin with (see above), minus the two already walked.
        assert_eq!(s.remaining(), 7, "the two already walked are not offered again");
    }
}
