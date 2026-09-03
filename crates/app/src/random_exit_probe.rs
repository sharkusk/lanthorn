//! Phase 2 of SQ-1257: is a move Phase 1 could not classify actually random?
//!
//! [`crate::engine::Engine::declared_exit`] answers `DeclaredExit::Room(x)` for
//! an ordinary passage and `Absent`/`Code` when the room's own map data has
//! nothing static to check the move against — Lost Pig's gnome-tunnel rooms
//! read `Absent` (no `*_to` property at all; a "before going" rule intercepts
//! the move before the library's exit-table code ever runs) and the gateway
//! into them reads `Code` (a routine decides). `session::apply_turn` mints the
//! ordinary edge for both, exactly as it always has, because neither is
//! PROOF the destination varies — most `Code` exits are perfectly
//! deterministic doors whose destination just happens to be computed instead
//! of stored.
//!
//! This module supplies the proof, after the fact, in a silent shadow: walk
//! the SAME direction from the SAME pre-move moment twice, under two
//! different random seeds, and see whether either walk disagrees with where
//! the live player actually landed. Disagreement is direct evidence the story
//! rolled dice for this move; agreement on all three is evidence it did not.
//!
//! # Two shapes, not one
//!
//! **A first walk of an `Absent`/`Code` direction.** `apply_turn` already
//! minted the ordinary edge — Phase 1's usual behaviour, since neither
//! answer is proof of anything on its own. Disagreement here DELETES that
//! edge and marks the direction random
//! ([`mapper::graph::MapGraph::mark_random_exit`]); agreement leaves it
//! standing.
//!
//! **A re-walk of a direction ALREADY marked random.** `apply_turn`'s own
//! check mints no edge this time (see the comment there), so there is
//! nothing to delete — instead this is the UPGRADE path. Lost Pig's gnome
//! leading the player back out of the tunnels is exactly this shape: a
//! direction that wandered randomly before now behaves deterministically,
//! and the map has to be able to say so. Agreement on both reseeded attempts
//! clears the mark ([`mapper::graph::MapGraph::unmark_random_exit`]) and
//! mints the now-confirmed edge, through [`Mapper::record_probed_passage`] —
//! the same path a return-probe-discovered edge takes, which does the same
//! `Mapper::mint_passage` work (`add_edge` + collapsing a now-redundant `?`
//! stub + laying the destination out) a walked crossing does, without
//! touching `MapGraph::set_current`/`arrived_via`: this answer can land
//! several turns after the move it is about, by which point the player may
//! not even be standing in the room any more. Disagreement leaves the mark
//! exactly as it was — the re-walk proved nothing new.
//!
//! [`RandomExitSearch::was_random`] is which of the two a given search is;
//! [`deliver`] is where the fork happens.
//!
//! # Why the snapshot is the END of the PREVIOUS turn
//!
//! By the time `session::apply_turn` (and this module) can be asked to
//! classify a move, the move has already happened — `Engine::submit` ran
//! before `turn::finish_command_turn` was ever called. So the only PRE-move
//! state this module can reach is whatever was kept from the moment before:
//! the engine snapshot taken at the end of turn N-1, which is exactly the
//! state the player was in when they typed the command that became turn N's
//! move. `AppState::random_exit_pre_move_save` carries it forward, one turn
//! behind, refreshed at the end of every Z-machine turn (cheap — Quetzal
//! serialization is sub-millisecond) and validated against `room_before`
//! before use: a stale snapshot (from before a restore, or a turn that never
//! refreshed it) names the room it was taken in, and a mismatch there means
//! "no usable pre-move state this turn" rather than a wrong answer.
//!
//! # Why reseed at all
//!
//! Quetzal saves no RNG state (`zvm::quetzal` never touches it — see the
//! `Machine::rng_state` field docs), so a shadow restored from a snapshot
//! runs its OWN `random` draw, wherever the shadow's last boot or restore
//! left it — not a copy of the live game's. That is not enough on its own:
//! two attempts from the same snapshot could coincidentally inherit the SAME
//! shadow RNG state and always agree with each other, which would look
//! exactly like a deterministic story. [`Engine::reseed_random`] forces each
//! attempt's own draw explicitly, to two seeds neither equal to the live
//! game's own (`derived_seeds`), so agreement between the two attempts is
//! actual evidence rather than an artifact of both starting from the same
//! place.

use mapper::direction::{long_label, Direction};
use mapper::graph::RoomId;
use mapper::mapper::{Mapper, ProbedPassage};

use crate::engine::Engine;
use crate::state::AppState;

/// Two seeds derived from the live game's own RNG state, deliberately never
/// equal to it — see the module docs' "why reseed at all".
pub fn derived_seeds(live_seed: u32) -> [u32; 2] {
    // XOR, not OR: an all-ones seed is a real `u32` value and `x.rotate_left(n) | 1` is a no-op
    // on it (every bit, including bit 0, is already set), which is exactly the "coincides with
    // the live seed" case this exists to rule out.
    [live_seed ^ 0x9E37_79B9, live_seed.rotate_left(13) ^ 0x1]
}

/// A Phase-2 search in progress.
#[derive(Debug)]
pub struct RandomExitSearch {
    /// The room the move was minted FROM.
    origin: RoomId,
    /// The direction walked.
    dir: Direction,
    /// Where the LIVE player actually landed this turn — the ground truth both shadow walks are
    /// judged against, whether or not `apply_turn` minted an edge to it this time.
    live_dest: RoomId,
    /// Which shape this search is (see the module docs): `true` when `dir` out of `origin` was
    /// ALREADY marked random before this move — the UPGRADE path — `false` for a first walk of
    /// an `Absent`/`Code` direction, where `apply_turn` already minted the edge being judged.
    was_random: bool,
    /// The token the answer will carry.
    token: u64,
}

impl RandomExitSearch {
    /// The room the search is about, for tests and diagnostics.
    pub fn origin(&self) -> RoomId {
        self.origin
    }
    /// The direction it is checking, for tests and diagnostics.
    pub fn dir(&self) -> Direction {
        self.dir
    }
    /// Which shape this search is — see the module docs.
    pub fn was_random(&self) -> bool {
        self.was_random
    }
}

/// Start a Phase-2 search, if this turn earned one.
///
/// Called once per turn from `turn::finish_command_turn`, after `apply_turn`
/// has settled the move. The caller is responsible for the gate — this
/// function assumes it is worth asking and only refuses on infrastructure
/// grounds (unarmed shadow, busy, no seed to read, no usable pre-move
/// snapshot): whether `dir`'s `DeclaredExit` was `Absent`/`Code`, and whether
/// it is already marked random (which decides `was_random`), are
/// `finish_command_turn`'s own checks, made with `DeclaredExit` and
/// [`mapper::graph::MapGraph::is_random_exit`], which this module does not
/// read.
#[allow(clippy::too_many_arguments)]
pub fn arm_random_exit_search(
    state: &mut AppState,
    live: &dyn Engine,
    origin: RoomId,
    dir: Direction,
    live_dest: RoomId,
    was_random: bool,
    pre_move_save: std::sync::Arc<crate::engine::EngineSave>,
) {
    if !state.probe.is_armed() {
        return;
    }
    let Some(live_seed) = live.rng_seed() else { return };
    let seeds = derived_seeds(live_seed);
    let from = crate::probe::ProbeSnapshot::from_save(pre_move_save);
    let command = long_label(dir).to_string();
    let Some(token) = state.probe.ask_from_reseeded(&from, &command, &seeds) else {
        return; // busy, unarmed, or mid-save — this move's outcome (edge or mark) simply stands
    };
    state.random_exit_search = Some(RandomExitSearch { origin, dir, live_dest, was_random, token });
}

/// True when `token` answers the search running now, if any.
pub fn owns(state: &AppState, token: u64) -> bool {
    state.random_exit_search.as_ref().is_some_and(|s| s.token == token)
}

/// Judge a Phase-2 answer (SQ-1257).
///
/// Returns true when the map changed — an edge deleted and the direction marked random, OR a
/// mark cleared and an edge minted — what tells the caller to bump the graph generation and
/// redraw, the same signal [`crate::return_probe::deliver`] gives.
///
/// # Evidence, not a vote
///
/// A shadow step that quit, escaped, or could not say where it landed is INCONCLUSIVE and counts
/// toward neither side — an unanswerable question is not evidence the story is deterministic,
/// and treating it as agreement would let a shadow that merely failed to boot silently
/// rubber-stamp every edge (or every upgrade). Evidence only comes from a step that DID land
/// somewhere.
pub fn deliver(state: &mut AppState, mapper: &mut Mapper, answer: &crate::probe::Answer) -> bool {
    let Some(search) = state.random_exit_search.take() else { return false };
    if search.token != answer.token {
        state.random_exit_search = Some(search); // not ours; leave it running
        return false;
    }
    let Some(run) = &answer.run else { return false };

    if search.was_random {
        deliver_upgrade(mapper, &search, run)
    } else {
        deliver_first_walk(mapper, &search, run)
    }
}

/// Every step's verdict against `live_dest`: `(any_evidence, any_disagree)` — see [`deliver`]'s
/// "evidence, not a vote".
fn judge(run: &crate::probe::ProbeRun, live_dest: RoomId) -> (bool, bool) {
    let mut any_evidence = false;
    let mut any_disagree = false;
    for step in &run.steps {
        if step.quit || step.escaped {
            continue;
        }
        let Some(loc) = step.location else { continue };
        any_evidence = true;
        if loc != live_dest {
            any_disagree = true;
        }
    }
    (any_evidence, any_disagree)
}

/// Note every room a disagreeing shadow run actually landed in, plus the live destination itself
/// (SQ-1261) — disagreement is exactly the moment this module proves the story's destination
/// varies, and every room named in that proof (the live landing AND each shadow attempt that
/// reached somewhere) is a real destination the room card and the map should be able to name,
/// not just the fact that it varies. A step with no usable evidence (quit, escaped, or no
/// location) says nothing and is skipped, same as [`judge`].
fn note_disagreeing_destinations(
    mapper: &mut Mapper,
    origin: RoomId,
    dir: Direction,
    live_dest: RoomId,
    run: &crate::probe::ProbeRun,
) {
    mapper.graph.note_random_destination(origin, dir, live_dest);
    for step in &run.steps {
        if step.quit || step.escaped {
            continue;
        }
        if let Some(loc) = step.location {
            mapper.graph.note_random_destination(origin, dir, loc);
        }
    }
}

/// A first walk of an `Absent`/`Code` direction: `apply_turn` already minted the edge being
/// judged. Disagreement deletes it and marks the direction random; agreement (or no evidence)
/// leaves it standing.
///
/// # Staleness
///
/// Mirrors the return probe's silence discipline (SQ-1124), adapted for what this search
/// actually needs to be true: the edge it is judging must still exist, AS MINTED (same origin,
/// direction, and destination), or the player has since moved the map on in some way this answer
/// cannot speak to, and the answer is dropped rather than acted on.
fn deliver_first_walk(mapper: &mut Mapper, search: &RandomExitSearch, run: &crate::probe::ProbeRun) -> bool {
    if !mapper
        .graph
        .connections()
        .iter()
        .any(|c| c.origin == search.origin && c.dir == search.dir && c.dest == search.live_dest)
    {
        return false; // the edge this search was about is gone or changed; nothing to judge
    }
    let (any_evidence, any_disagree) = judge(run, search.live_dest);
    if !any_evidence || !any_disagree {
        return false; // no usable evidence, or full agreement — the edge stands
    }
    mapper.graph.remove_connection(search.origin, search.dir);
    mapper.record_random_exit(search.origin, search.dir);
    // SQ-1261: this is the FIRST evidence the direction is random at all, so nothing about it
    // has been noted yet — not by `apply_turn` (which minted the now-deleted edge on the belief
    // this was an ordinary passage) and not by an earlier disagreement (there isn't one).
    note_disagreeing_destinations(mapper, search.origin, search.dir, search.live_dest, run);
    true
}

/// A re-walk of a direction ALREADY marked random: there is no edge to check for staleness
/// against (`apply_turn` minted none), so the guard instead is that the mark itself must still
/// be there — if something else already resolved it, this answer is about a question that is no
/// longer being asked. Agreement on every usable attempt clears the mark and mints the
/// now-confirmed edge; disagreement (or no evidence) leaves the mark untouched.
fn deliver_upgrade(mapper: &mut Mapper, search: &RandomExitSearch, run: &crate::probe::ProbeRun) -> bool {
    if !mapper.graph.is_random_exit(search.origin, search.dir) {
        return false; // no longer marked; this answer is about a question nobody is asking
    }
    let (any_evidence, any_disagree) = judge(run, search.live_dest);
    if !any_evidence {
        return false; // nothing usable either way — stay marked
    }
    if any_disagree {
        // SQ-1261: `apply_turn`'s own re-walk of an already-marked direction already noted
        // `live_dest` (the walk that armed this very search), so this adds only what the SHADOW
        // attempts saw that the live walk did not — still worth recording, even though the
        // upgrade itself does not go through: disagreement here is fresh evidence the direction
        // keeps varying, not proof of nothing.
        note_disagreeing_destinations(mapper, search.origin, search.dir, search.live_dest, run);
        return false; // at least one attempt disagreed — stay marked
    }
    let passage = ProbedPassage { from: search.origin, dir: search.dir, to: search.live_dest };
    if mapper.record_probed_passage(passage) {
        mapper.graph.unmark_random_exit(search.origin, search.dir);
        true
    } else {
        false // e.g. a self-loop, or an edge already there another way — leave the mark as is
    }
}

/// Run a search to its end, waiting for its answer instead of collecting one
/// that has already arrived.
///
/// **Not for the event loop** — the measurement and test path, mirroring
/// [`crate::return_probe::settle_return_search`]. A Phase-2 search issues
/// exactly one job (both seeds together), so there is no per-pass dispatch
/// loop to drive; this simply blocks for that one answer and delivers it.
pub fn settle_random_exit_search(state: &mut AppState, mapper: &mut Mapper) -> bool {
    if state.random_exit_search.is_none() {
        return false;
    }
    let Some(answer) = state.probe.settle() else {
        state.random_exit_search = None;
        return false;
    };
    if !owns(state, answer.token) {
        return false;
    }
    deliver(state, mapper, &answer)
}

#[cfg(all(test, feature = "t-session"))]
mod tests {
    use super::*;
    use crate::probe::{test_answer, ProbeRun, ProbeStep, WorldPrint};
    use crate::session::{apply_turn, DeathWatch, TurnResult};

    fn step(location: Option<RoomId>) -> ProbeStep {
        ProbeStep {
            command: "north".to_string(),
            reply: String::new(),
            location,
            world: WorldPrint::default(),
            quit: false,
            escaped: false,
        }
    }

    fn snap(number: RoomId, name: &str) -> zvm::ObjectSnapshot {
        zvm::ObjectSnapshot { number, parent: 0, name: name.to_string() }
    }

    /// The full cycle a real Lost Pig-shaped direction goes through, driven exactly the way
    /// `turn::finish_command_turn` drives it — `apply_turn` first, `deliver` second — except the
    /// Phase-2 ANSWER is hand-built (`crate::probe::test_answer`) rather than fetched from a real
    /// worker, since only a real Z-machine story can be booted into one. SQ-1257's corrected
    /// design in one pass:
    ///
    /// 1. A direction already marked random is walked again and lands somewhere — `apply_turn`
    ///    mints NO edge (same as before the correction).
    /// 2. The Phase-2 re-probe (`was_random: true`) AGREES on both attempts — `deliver` clears
    ///    the mark and mints the edge (the new part: an upgrade).
    /// 3. The SAME direction, no longer marked, is walked again and lands somewhere ELSE —
    ///    `apply_turn` mints the (wrong) edge as an ordinary first walk would (Phase 1 cannot
    ///    tell `Absent`/`Code` apart from a real passage on its own).
    /// 4. The Phase-2 first-walk probe DISAGREES — `deliver` deletes that edge and marks the
    ///    direction random again.
    #[test]
    fn a_random_mark_upgrades_on_agreement_and_reverts_on_the_next_disagreement() {
        let mut mapper = Mapper::default();
        let mut death = DeathWatch::default();
        apply_turn(&mut mapper, "", &TurnResult::observation(snap(1, "Tunnel")), &mut death);
        mapper.record_random_exit(1, Direction::N);
        assert!(mapper.graph.is_random_exit(1, Direction::N));

        // ── 1: walk the marked direction, lands in room 2. No edge minted. ──
        apply_turn(&mut mapper, "north", &TurnResult::observation(snap(2, "A")), &mut death);
        assert_eq!(mapper.graph.connections().iter().find(|c| c.origin == 1 && c.dir == Direction::N), None);
        assert!(mapper.graph.is_random_exit(1, Direction::N), "still marked — nothing decided yet");
        // SQ-1261: the live re-walk itself is evidence of where the story sends the player.
        assert_eq!(mapper.graph.random_destinations(1, Direction::N), &[2]);

        // ── 2: Phase 2 re-probe agrees on both attempts. Upgrade. ──
        let mut state = AppState::default();
        state.random_exit_search =
            Some(RandomExitSearch { origin: 1, dir: Direction::N, live_dest: 2, was_random: true, token: 7 });
        let run = ProbeRun { baseline: WorldPrint::default(), steps: vec![step(Some(2)), step(Some(2))] };
        assert!(deliver(&mut state, &mut mapper, &test_answer(7, Some(run))), "the map changed");
        assert!(!mapper.graph.is_random_exit(1, Direction::N), "the mark is cleared");
        assert_eq!(
            mapper.graph.connections().iter().find(|c| c.origin == 1 && c.dir == Direction::N).map(|c| c.dest),
            Some(2),
            "and the confirmed edge exists, to the right destination"
        );
        // SQ-1261: the upgrade-agreement path notes nothing new (there is no "random" fact left
        // to attach it to) and clears what was recorded before, along with the mark itself.
        assert!(mapper.graph.random_destinations(1, Direction::N).is_empty(), "cleared with the mark");

        // Walk back to room 1 (an unrelated move — `observe_relocation` after step 1 left
        // `current` on room 2, and the player has to be standing in room 1 again before "walk
        // north out of room 1" means anything).
        apply_turn(&mut mapper, "back", &TurnResult::observation(snap(1, "Tunnel")), &mut death);

        // ── 3: walk the SAME direction again — no longer marked — and land somewhere ELSE.
        // `apply_turn` mints the edge as it would for any ordinary first walk of a direction it
        // has no reason yet to distrust (Phase 1 alone cannot tell this apart from a real move).
        apply_turn(&mut mapper, "north", &TurnResult::observation(snap(3, "B")), &mut death);
        assert_eq!(
            mapper.graph.connections().iter().find(|c| c.origin == 1 && c.dir == Direction::N).map(|c| c.dest),
            Some(3),
            "Phase 1 minted the new (wrong) edge, same as it always has"
        );

        // ── 4: Phase 2's first-walk probe disagrees. Deleted, marked random again. ──
        state.random_exit_search =
            Some(RandomExitSearch { origin: 1, dir: Direction::N, live_dest: 3, was_random: false, token: 8 });
        let run = ProbeRun { baseline: WorldPrint::default(), steps: vec![step(Some(2)), step(Some(4))] };
        assert!(deliver(&mut state, &mut mapper, &test_answer(8, Some(run))), "the map changed again");
        assert_eq!(
            mapper.graph.connections().iter().find(|c| c.origin == 1 && c.dir == Direction::N),
            None,
            "the wrong edge is gone"
        );
        assert!(mapper.graph.is_random_exit(1, Direction::N), "and the direction is random once more");
        // SQ-1261: the first-walk disagreement names the live destination AND every shadow
        // attempt that reached somewhere — the player's own eyes plus both reseeded witnesses.
        assert_eq!(
            mapper.graph.random_destinations(1, Direction::N),
            &[3, 2, 4],
            "live destination first, then each shadow attempt, first-seen order"
        );
    }

    /// Falsify the upgrade half in isolation: with no usable evidence at all, `deliver` must
    /// leave an already-random mark exactly as it was rather than guessing either way.
    #[test]
    fn an_inconclusive_upgrade_answer_changes_nothing() {
        let mut mapper = Mapper::default();
        let mut death = DeathWatch::default();
        apply_turn(&mut mapper, "", &TurnResult::observation(snap(1, "Tunnel")), &mut death);
        mapper.record_random_exit(1, Direction::N);

        let mut state = AppState::default();
        state.random_exit_search =
            Some(RandomExitSearch { origin: 1, dir: Direction::N, live_dest: 2, was_random: true, token: 1 });
        // Both attempts quit/escaped: no usable evidence either way.
        let run = ProbeRun {
            baseline: WorldPrint::default(),
            steps: vec![
                ProbeStep { quit: true, ..step(None) },
                ProbeStep { escaped: true, ..step(None) },
            ],
        };
        assert!(!deliver(&mut state, &mut mapper, &test_answer(1, Some(run))), "nothing to report");
        assert!(mapper.graph.is_random_exit(1, Direction::N), "still marked");
        assert_eq!(mapper.graph.connections().iter().find(|c| c.origin == 1 && c.dir == Direction::N), None);
        // SQ-1261: no usable evidence either way means no destination is worth recording either —
        // an inconclusive answer must not silently smuggle a room into the list.
        assert!(mapper.graph.random_destinations(1, Direction::N).is_empty());
    }

    /// SQ-1261: an UPGRADE search's disagreement is still fresh evidence — it does not clear the
    /// mark (the upgrade did not happen), but it must still name every room the disagreement
    /// actually surfaced, exactly as a first-walk disagreement does.
    #[test]
    fn an_upgrade_disagreement_still_notes_the_destinations_it_saw() {
        let mut mapper = Mapper::default();
        let mut death = DeathWatch::default();
        apply_turn(&mut mapper, "", &TurnResult::observation(snap(1, "Tunnel")), &mut death);
        mapper.record_random_exit(1, Direction::N);

        let mut state = AppState::default();
        state.random_exit_search =
            Some(RandomExitSearch { origin: 1, dir: Direction::N, live_dest: 2, was_random: true, token: 3 });
        // One attempt agrees (2), the other disagrees (5) — any disagreement keeps the mark.
        let run = ProbeRun { baseline: WorldPrint::default(), steps: vec![step(Some(2)), step(Some(5))] };
        assert!(!deliver(&mut state, &mut mapper, &test_answer(3, Some(run))), "no upgrade — still disagreement");
        assert!(mapper.graph.is_random_exit(1, Direction::N), "the mark stands");
        assert_eq!(
            mapper.graph.random_destinations(1, Direction::N),
            &[2, 5],
            "the live destination and the disagreeing shadow attempt are both recorded"
        );
    }
}
