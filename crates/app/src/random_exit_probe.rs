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
//! # Three shapes, not one
//!
//! **A first walk of an `Absent`/`Code` direction** ([`SearchKind::FirstWalk`]). `apply_turn`
//! already minted the ordinary edge — Phase 1's usual behaviour, since neither answer is proof of
//! anything on its own. Disagreement here DELETES that edge and marks the direction random
//! ([`mapper::graph::MapGraph::mark_random_exit`]); agreement leaves it standing.
//!
//! **A re-walk of a direction ALREADY marked random** ([`SearchKind::Upgrade`]). `apply_turn`'s
//! own check mints no edge this time (see the comment there), so there is nothing to delete —
//! instead this is the UPGRADE path. Lost Pig's gnome leading the player back out of the tunnels
//! is exactly this shape: a direction that wandered randomly before now behaves deterministically,
//! and the map has to be able to say so. Agreement on both reseeded attempts clears the mark
//! ([`mapper::graph::MapGraph::unmark_random_exit`]) and mints the now-confirmed edge, through
//! [`Mapper::record_probed_passage`] — the same path a return-probe-discovered edge takes, which
//! does the same `Mapper::mint_passage` work (`add_edge` + collapsing a now-redundant `?` stub +
//! laying the destination out) a walked crossing does, without touching
//! `MapGraph::set_current`/`arrived_via`: this answer can land several turns after the move it is
//! about, by which point the player may not even be standing in the room any more. Disagreement
//! leaves the mark exactly as it was — the re-walk proved nothing new. **SQ-1269**: agreement no
//! longer upgrades unconditionally — see `deliver_upgrade`'s pool check, the flicker fix.
//!
//! **A SUSPICION** ([`SearchKind::Suspicion`], SQ-1269) — a declared-exit mismatch or a live
//! contradiction against something the map already believed (an existing edge, or an existing
//! self-loop, on the same origin/direction), neither of which `apply_turn` marks on the spot any
//! more when a probe can run: it leaves the old edge/self-loop standing and mints nothing new,
//! deferring the decision here. Agreement on both attempts means the passage is deterministic and
//! has CHANGED — the old edge/self-loop is removed and the new one minted, no mark at all
//! ([`Mapper::resolve_suspicion_as_changed`]). Disagreement means it is genuinely random — the old
//! edge/self-loop is removed, the direction is marked, and the pool gets the old destination (the
//! room itself, for a contradicted self-loop — the room card's "back here"), the live landing, and
//! whatever the shadow itself saw ([`Mapper::resolve_suspicion_as_random`]). Where no probe can
//! run at all, the caller resolves the suspicion immediately via
//! [`Mapper::resolve_suspicion_as_random`] — today's old immediate-marking behaviour, unchanged in
//! effect for an engine or a turn this module never gets a chance to look at. A probe that DID
//! run but came back with no usable evidence at all (every attempt quit, escaped, or reported no
//! location) resolves the same way — see `deliver_suspicion`'s own "no evidence at all" doc.
//!
//! [`RandomExitSearch::kind`] is which of the three a given search is; [`deliver`] is where the
//! fork happens.
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
use mapper::mapper::{Mapper, ProbedPassage, RandomExitSuspicion};

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

/// Which of the module docs' three shapes a [`RandomExitSearch`] is (SQ-1269).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    /// A first walk of an `Absent`/`Code` direction — `apply_turn` already minted the edge being
    /// judged.
    FirstWalk,
    /// A re-walk of a direction ALREADY marked random — agreement upgrades the mark to an edge.
    Upgrade,
    /// A declared-exit mismatch or a live contradiction `apply_turn` left unresolved rather than
    /// marking on the spot — see [`RandomExitSuspicion`]. `old_dest` is what the map already
    /// claimed for the key, if anything.
    Suspicion { old_dest: Option<RoomId> },
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
    /// Which shape this search is (see the module docs).
    kind: SearchKind,
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
    pub fn kind(&self) -> SearchKind {
        self.kind
    }
}

/// Start a Phase-2 search, if this turn earned one.
///
/// Called once per turn from `turn::finish_command_turn`, after `apply_turn`
/// has settled the move. The caller is responsible for the gate — this
/// function assumes it is worth asking and only refuses on infrastructure
/// grounds (unarmed shadow, busy, no seed to read, no usable pre-move
/// snapshot): whether `dir`'s `DeclaredExit` was `Absent`/`Code`, whether it
/// is already marked random, and whether `apply_turn` left a
/// [`RandomExitSuspicion`] pending (which together decide `kind`) are
/// `finish_command_turn`'s own checks, which this module does not read.
#[allow(clippy::too_many_arguments)]
pub fn arm_random_exit_search(
    state: &mut AppState,
    live: &dyn Engine,
    origin: RoomId,
    dir: Direction,
    live_dest: RoomId,
    kind: SearchKind,
    pre_move_save: std::sync::Arc<crate::engine::EngineSave>,
) {
    if !state.probe.is_armed() {
        return;
    }
    let Some(live_seed) = live.rng_seed() else { return };
    let seeds = derived_seeds(live_seed);
    let from = crate::probe::ProbeSnapshot::from_save(live, pre_move_save);
    let command = long_label(dir).to_string();
    let Some(token) = state.probe.ask_from_reseeded(&from, &command, &seeds) else {
        return; // busy, unarmed, or mid-save — this move's outcome (edge or mark) simply stands
    };
    state.random_exit_search = Some(RandomExitSearch { origin, dir, live_dest, kind, token });
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

    // `Suspicion` alone reads `answer.run` even when the shadow is `None` — a BROKEN shadow
    // (the boot failed, or it would not take the live state; see `deliver_suspicion`'s "no
    // evidence at all" doc) is exactly as inconclusive as one that ran and said nothing, and
    // both resolve the same way. `FirstWalk`/`Upgrade` always have somewhere safe to stand pat
    // instead, so a missing run leaves them alone entirely, same as ever.
    if let SearchKind::Suspicion { old_dest } = search.kind {
        return deliver_suspicion(mapper, &search, old_dest, answer.run.as_ref());
    }
    let Some(run) = &answer.run else { return false };
    match search.kind {
        SearchKind::Upgrade => deliver_upgrade(mapper, &search, run),
        SearchKind::FirstWalk => deliver_first_walk(mapper, &search, run),
        SearchKind::Suspicion { .. } => unreachable!("handled above"),
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
/// location) says nothing and is skipped, same as [`judge`]. This deliberately admits a room the
/// live map has never visited at all — a disagreeing shadow attempt finding one is the whole
/// point (an upgrade search's own falsification test covers exactly that), so pool hygiene
/// below must never require "already on the map".
///
/// # Pool hygiene (SQ-1267)
///
/// One specific, known-bad shape is excluded regardless: `live_dest`'s own printed name, hashed
/// the way a Glulx room with no located `location` global falls back to
/// (`crate::roomid::synthetic_room_id`, [`crate::glulx_session::GlulxSession::room_for`]'s `None`
/// branch). [`Engine::room_identity_state`]/[`Engine::apply_room_identity_state`] is the actual
/// fix — carrying the live session's room-keying state into every shadow restore, so the shadow
/// never falls back to that hash for a room the live session keys by address — and after it a
/// shadow simply never reports this value, making the check below cheap insurance rather than
/// the primary guard. But it is computed directly from what the live game printed, not guessed
/// at generically, which is what lets it reject exactly the phantom this bug produced without
/// also rejecting a genuinely new room a disagreeing attempt is the only thing to have found.
fn note_disagreeing_destinations(
    mapper: &mut Mapper,
    origin: RoomId,
    dir: Direction,
    live_dest: RoomId,
    run: &crate::probe::ProbeRun,
) {
    mapper.graph.note_random_destination(origin, dir, live_dest);
    let known_phantom = mapper.graph.room(live_dest).map(|r| crate::roomid::synthetic_room_id(r.label()));
    for step in &run.steps {
        if step.quit || step.escaped {
            continue;
        }
        let Some(loc) = step.location else { continue };
        if Some(loc) == known_phantom && loc != live_dest {
            continue; // SQ-1267: the exact unlocked name-hash phantom this bug reported
        }
        mapper.graph.note_random_destination(origin, dir, loc);
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
    // SQ-1269: the flicker fix. With two possible destinations, two reseeded attempts agree with
    // whatever the live walk just landed in by pure luck one time in four — so a pool that ALREADY
    // holds two or more distinct rooms is proof enough of its own that the direction keeps varying,
    // and a single lucky agreement must not flip it back to a confident arrow. The pool at this
    // point already includes the live landing that armed this very search (`apply_turn`'s own
    // `note_random_destination` call for a re-walk of a marked direction, SQ-1261) — so "fewer than
    // two" here means the mark came from a single mismatch, or a shadow disagreement that pooled
    // exactly one room besides the live one. Upgrade stays possible only then.
    if mapper.graph.random_destinations(search.origin, search.dir).len() >= 2 {
        return false; // still marked — the pool alone outweighs one agreeing pair
    }
    let passage = ProbedPassage { from: search.origin, dir: search.dir, to: search.live_dest };
    if mapper.record_probed_passage(passage) {
        mapper.graph.unmark_random_exit(search.origin, search.dir);
        true
    } else {
        false // e.g. a self-loop, or an edge already there another way — leave the mark as is
    }
}

/// A [`SearchKind::Suspicion`] answer (SQ-1269): a declared-exit mismatch or a live contradiction
/// `apply_turn` left unresolved rather than marking on the spot. Agreement on every usable attempt
/// means the passage is deterministic and has CHANGED; disagreement means it is genuinely random.
///
/// # Staleness
///
/// The same discipline as [`deliver_first_walk`]/[`deliver_upgrade`], adapted for what THIS search
/// needs to be true: whatever `old_dest` names (an edge, a self-loop, or nothing) must still be
/// exactly what stands on `(origin, dir)` now, and the direction must not already be marked —
/// either would mean something else already resolved the question this answer is about.
///
/// # No evidence at all
///
/// Unlike [`deliver_first_walk`]/[`deliver_upgrade`], a Suspicion search that comes back with
/// nothing usable resolves the same way as no probe running at ALL — [`Mapper::resolve_suspicion_as_random`],
/// same as the caller's own immediate fallback. Those two searches always have somewhere safe to
/// "stand pat": `apply_turn` already minted the edge a first walk is judging, or the mark an
/// upgrade is judging, so leaving it be on inconclusive evidence keeps whatever was already true.
/// A Suspicion has nowhere safe to stand — `apply_turn` deliberately minted and marked NOTHING —
/// so standing pat here would leave the direction showing neither an edge nor a `?`, which
/// `mark_tried` (already set for it, whichever way this resolves) turns into `×` ("tried, no path
/// through") on the matrix: an outright lie about a move the player just completed.
///
/// This path is not decorative. It was written for a real, whole-story outage — every Version 6
/// shadow refusing the live restore, SQ-1266 — and though that particular cause is fixed, a
/// shadow can still fail to boot, quit inside a probe, or be handed a snapshot a future engine
/// declines. Whatever the reason, a Suspicion search that learns nothing must land somewhere
/// truthful, and that is here.
fn deliver_suspicion(
    mapper: &mut Mapper,
    search: &RandomExitSearch,
    old_dest: Option<RoomId>,
    run: Option<&crate::probe::ProbeRun>,
) -> bool {
    let graph_dest_now =
        mapper.graph.connections().iter().find(|c| c.origin == search.origin && c.dir == search.dir).map(|c| c.dest);
    if graph_dest_now != old_dest || mapper.graph.is_random_exit(search.origin, search.dir) {
        return false; // the state this search was about has already changed
    }
    // `run: None` — a broken shadow, e.g. a restore the engine refused — is exactly as
    // inconclusive as a run whose every step quit, escaped, or reported no location: neither
    // tells this search anything, so both take the same "no evidence at all" path below.
    let (any_evidence, any_disagree) = run.map(|r| judge(r, search.live_dest)).unwrap_or((false, false));
    let suspicion = RandomExitSuspicion { origin: search.origin, dir: search.dir, old_dest, live_dest: search.live_dest };
    if !any_evidence {
        mapper.resolve_suspicion_as_random(suspicion);
        return true;
    }
    if any_disagree {
        mapper.resolve_suspicion_as_random(suspicion);
        // SQ-1261: everything the SHADOW itself saw is fresh evidence too, beyond the old
        // destination and the live landing `resolve_suspicion_as_random` already pooled.
        note_disagreeing_destinations(mapper, search.origin, search.dir, search.live_dest, run.expect("any_evidence implies a run"));
    } else {
        mapper.resolve_suspicion_as_changed(suspicion);
    }
    true
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

    fn arm(origin: RoomId, dir: Direction, live_dest: RoomId, kind: SearchKind, token: u64) -> RandomExitSearch {
        RandomExitSearch { origin, dir, live_dest, kind, token }
    }

    /// The full cycle a real Lost Pig-shaped direction goes through, driven exactly the way
    /// `turn::finish_command_turn` drives it — `apply_turn` first, `deliver` second — except the
    /// Phase-2 ANSWER is hand-built (`crate::probe::test_answer`) rather than fetched from a real
    /// worker, since only a real Z-machine story can be booted into one. SQ-1257's corrected
    /// design in one pass, updated for SQ-1264's live-walk contradiction rule and SQ-1269's
    /// suspicion-first redesign of it:
    ///
    /// 1. A direction already marked random is walked again and lands somewhere — `apply_turn`
    ///    mints NO edge (same as before the correction).
    /// 2. The Phase-2 re-probe (`SearchKind::Upgrade`) AGREES on both attempts — `deliver` clears
    ///    the mark and mints the edge (an upgrade). This is exactly the "statistical hole"
    ///    SQ-1264's report describes: with two possible destinations, two reseeded attempts agree
    ///    with the live landing by pure luck one time in four, and a confident edge is minted for
    ///    a direction that is still random underneath. The pool holds exactly one room at this
    ///    point (the live landing from step 1, SQ-1261), which is what SQ-1269's flicker fix
    ///    still allows to upgrade — see `upgrade_never_fires_once_the_pool_already_holds_two_destinations`
    ///    for the pool≥2 half of that fix.
    /// 3. The SAME direction, no longer marked, is walked again and lands somewhere ELSE.
    ///    `apply_turn` no longer marks this on the spot (SQ-1269): it leaves the just-minted edge
    ///    standing and stashes a [`mapper::mapper::RandomExitSuspicion`] instead — proven here by
    ///    reading it straight off `Mapper::take_random_exit_suspicion`. A DISAGREEING probe answer
    ///    is what actually reverts it: `deliver`'s `Suspicion` arm removes the edge and marks the
    ///    direction random again, with BOTH the edge's old destination and the new live landing
    ///    recorded, since the contradiction is proof both are real places the story sends the
    ///    player — the same end state SQ-1264's old immediate rule produced, reached one probe
    ///    round trip later instead of on the spot.
    #[test]
    fn a_random_mark_upgrades_on_agreement_then_a_suspicion_disagreement_reverts_it() {
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
        assert!(mapper.take_random_exit_suspicion().is_none(), "an already-random re-walk is Upgrade territory, not a new suspicion");

        // ── 2: Phase 2 re-probe agrees on both attempts. Upgrade — the pool holds only 1 room. ──
        let mut state = AppState::default();
        state.random_exit_search = Some(arm(1, Direction::N, 2, SearchKind::Upgrade, 7));
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
        // SQ-1269: `apply_turn` leaves the edge standing and stashes a suspicion instead of
        // marking on the spot.
        apply_turn(&mut mapper, "north", &TurnResult::observation(snap(3, "B")), &mut death);
        assert_eq!(
            mapper.graph.connections().iter().find(|c| c.origin == 1 && c.dir == Direction::N).map(|c| c.dest),
            Some(2),
            "the edge stands — nothing decided yet, this is a suspicion, not proof"
        );
        assert!(!mapper.graph.is_random_exit(1, Direction::N), "not marked either — still undecided");
        let susp = mapper.take_random_exit_suspicion().expect("the contradiction left a suspicion pending");
        assert_eq!(susp.origin, 1);
        assert_eq!(susp.dir, Direction::N);
        assert_eq!(susp.old_dest, Some(2), "the edge's current destination");
        assert_eq!(susp.live_dest, 3, "where the player actually landed this move");

        // A DISAGREEING probe answer is what actually reverts it.
        let mut state = AppState::default();
        state.random_exit_search =
            Some(arm(susp.origin, susp.dir, susp.live_dest, SearchKind::Suspicion { old_dest: susp.old_dest }, 9));
        let run = ProbeRun { baseline: WorldPrint::default(), steps: vec![step(Some(2)), step(Some(3))] };
        assert!(deliver(&mut state, &mut mapper, &test_answer(9, Some(run))), "the map changed");
        assert_eq!(
            mapper.graph.connections().iter().find(|c| c.origin == 1 && c.dir == Direction::N),
            None,
            "the old edge is removed rather than a second wrong one minted over it"
        );
        assert!(mapper.graph.is_random_exit(1, Direction::N), "and the direction is marked random");
        assert_eq!(
            mapper.graph.random_destinations(1, Direction::N),
            &[2, 3],
            "the edge's old (now-proven-wrong) destination, then the new live landing"
        );
    }

    /// SQ-1269, the flicker fix's other half: once the pool already holds two or more distinct
    /// rooms, a single agreeing pair must never upgrade the mark back to a confident edge, however
    /// clean the agreement — the pool alone is already proof the direction varies. Falsify by
    /// dropping the pool-size check in `deliver_upgrade` and this fails (the mark clears).
    #[test]
    fn upgrade_never_fires_once_the_pool_already_holds_two_destinations() {
        let mut mapper = Mapper::default();
        let mut death = DeathWatch::default();
        apply_turn(&mut mapper, "", &TurnResult::observation(snap(1, "Tunnel")), &mut death);
        mapper.record_random_exit(1, Direction::N);
        mapper.graph.note_random_destination(1, Direction::N, 2);
        mapper.graph.note_random_destination(1, Direction::N, 3);
        assert_eq!(mapper.graph.random_destinations(1, Direction::N), &[2, 3], "pool already holds two rooms");

        let mut state = AppState::default();
        state.random_exit_search = Some(arm(1, Direction::N, 2, SearchKind::Upgrade, 11));
        // Both attempts agree with the live landing — an ordinary upgrade would clear the mark.
        let run = ProbeRun { baseline: WorldPrint::default(), steps: vec![step(Some(2)), step(Some(2))] };
        assert!(!deliver(&mut state, &mut mapper, &test_answer(11, Some(run))), "no upgrade — the pool outweighs it");
        assert!(mapper.graph.is_random_exit(1, Direction::N), "still marked");
        assert_eq!(mapper.graph.connections().iter().find(|c| c.origin == 1 && c.dir == Direction::N), None, "still no edge");
        assert_eq!(mapper.graph.random_destinations(1, Direction::N), &[2, 3], "the pool is untouched");
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
        state.random_exit_search = Some(arm(1, Direction::N, 2, SearchKind::Upgrade, 1));
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
        state.random_exit_search = Some(arm(1, Direction::N, 2, SearchKind::Upgrade, 3));
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

    // ── SQ-1269: suspicion, not proof — the probe decides ─────────────────────────────────────

    /// (2a) A Phase-1 declared-exit mismatch with NO existing edge to contradict: `apply_turn`
    /// leaves a suspicion with `old_dest: None` rather than marking on the spot, and an AGREEING
    /// probe answer concludes the passage is deterministic — it mints the edge, with no mark.
    #[test]
    fn declared_mismatch_suspicion_agreement_mints_the_edge_with_no_mark() {
        let mut mapper = Mapper::default();
        let mut death = DeathWatch::default();
        apply_turn(&mut mapper, "", &TurnResult::observation(snap(1, "Tunnel")), &mut death);

        let mut r = TurnResult::observation(snap(2, "Other Tunnel"));
        r.declared_exit = Some(crate::engine::DeclaredExit::Room(3)); // declared a THIRD room, not 2
        apply_turn(&mut mapper, "north", &r, &mut death);
        assert_eq!(mapper.graph.connections().iter().find(|c| c.origin == 1 && c.dir == Direction::N), None, "no edge minted yet");
        assert!(!mapper.graph.is_random_exit(1, Direction::N), "not marked either — undecided");
        let susp = mapper.take_random_exit_suspicion().expect("the declared mismatch left a suspicion pending");
        assert_eq!((susp.origin, susp.dir, susp.old_dest, susp.live_dest), (1, Direction::N, None, 2));

        let mut state = AppState::default();
        state.random_exit_search =
            Some(arm(susp.origin, susp.dir, susp.live_dest, SearchKind::Suspicion { old_dest: susp.old_dest }, 21));
        let run = ProbeRun { baseline: WorldPrint::default(), steps: vec![step(Some(2)), step(Some(2))] };
        assert!(deliver(&mut state, &mut mapper, &test_answer(21, Some(run))), "the map changed");
        assert!(!mapper.graph.is_random_exit(1, Direction::N), "confirmed deterministic — no mark");
        assert_eq!(
            mapper.graph.connections().iter().find(|c| c.origin == 1 && c.dir == Direction::N).map(|c| c.dest),
            Some(2)
        );
        assert!(mapper.graph.random_destinations(1, Direction::N).is_empty(), "nothing pooled — never marked");
    }

    /// (2b) The same declared mismatch, but the probe DISAGREES: proven random. Marked, and the
    /// pool holds the live landing plus whatever the shadow itself saw.
    #[test]
    fn declared_mismatch_suspicion_disagreement_marks_random_with_pool() {
        let mut mapper = Mapper::default();
        let mut death = DeathWatch::default();
        apply_turn(&mut mapper, "", &TurnResult::observation(snap(1, "Tunnel")), &mut death);

        let mut r = TurnResult::observation(snap(2, "Other Tunnel"));
        r.declared_exit = Some(crate::engine::DeclaredExit::Room(3));
        apply_turn(&mut mapper, "north", &r, &mut death);
        let susp = mapper.take_random_exit_suspicion().expect("pending");

        let mut state = AppState::default();
        state.random_exit_search =
            Some(arm(susp.origin, susp.dir, susp.live_dest, SearchKind::Suspicion { old_dest: susp.old_dest }, 22));
        let run = ProbeRun { baseline: WorldPrint::default(), steps: vec![step(Some(2)), step(Some(4))] };
        assert!(deliver(&mut state, &mut mapper, &test_answer(22, Some(run))), "the map changed");
        assert!(mapper.graph.is_random_exit(1, Direction::N), "proven random — marked");
        assert_eq!(mapper.graph.connections().iter().find(|c| c.origin == 1 && c.dir == Direction::N), None);
        assert_eq!(
            mapper.graph.random_destinations(1, Direction::N),
            &[2, 4],
            "the live landing, then what the disagreeing shadow attempt itself saw"
        );
    }

    /// (2c) A live contradiction against an EXISTING edge: the probe agrees with the new landing,
    /// so the passage merely CHANGED — the stale edge is replaced, no mark.
    #[test]
    fn contradiction_suspicion_agreement_replaces_the_stale_edge() {
        let mut mapper = Mapper::default();
        let mut death = DeathWatch::default();
        apply_turn(&mut mapper, "", &TurnResult::observation(snap(1, "Origin")), &mut death);
        apply_turn(&mut mapper, "east", &TurnResult::observation(snap(2, "Room A")), &mut death); // mints 1--E-->2
        mapper.graph.set_current(1);

        apply_turn(&mut mapper, "east", &TurnResult::observation(snap(3, "Room B")), &mut death); // contradicts
        assert_eq!(
            mapper.graph.connections().iter().find(|c| c.origin == 1 && c.dir == Direction::E).map(|c| c.dest),
            Some(2),
            "the old edge stands — nothing decided yet"
        );
        let susp = mapper.take_random_exit_suspicion().expect("pending");
        assert_eq!((susp.origin, susp.dir, susp.old_dest, susp.live_dest), (1, Direction::E, Some(2), 3));

        let mut state = AppState::default();
        state.random_exit_search =
            Some(arm(susp.origin, susp.dir, susp.live_dest, SearchKind::Suspicion { old_dest: susp.old_dest }, 31));
        let run = ProbeRun { baseline: WorldPrint::default(), steps: vec![step(Some(3)), step(Some(3))] };
        assert!(deliver(&mut state, &mut mapper, &test_answer(31, Some(run))), "the map changed");
        assert!(!mapper.graph.is_random_exit(1, Direction::E), "deterministic but changed — no mark");
        assert_eq!(
            mapper.graph.connections().iter().find(|c| c.origin == 1 && c.dir == Direction::E).map(|c| c.dest),
            Some(3),
            "the stale edge is replaced by the confirmed new one"
        );
    }

    /// (2d) The same contradiction, but the probe DISAGREES: proven random, both the edge's old
    /// destination and the new live landing pooled.
    #[test]
    fn contradiction_suspicion_disagreement_marks_random_with_both_pooled() {
        let mut mapper = Mapper::default();
        let mut death = DeathWatch::default();
        apply_turn(&mut mapper, "", &TurnResult::observation(snap(1, "Origin")), &mut death);
        apply_turn(&mut mapper, "east", &TurnResult::observation(snap(2, "Room A")), &mut death);
        mapper.graph.set_current(1);
        apply_turn(&mut mapper, "east", &TurnResult::observation(snap(3, "Room B")), &mut death);
        let susp = mapper.take_random_exit_suspicion().expect("pending");

        let mut state = AppState::default();
        state.random_exit_search =
            Some(arm(susp.origin, susp.dir, susp.live_dest, SearchKind::Suspicion { old_dest: susp.old_dest }, 32));
        let run = ProbeRun { baseline: WorldPrint::default(), steps: vec![step(Some(2)), step(Some(3))] };
        assert!(deliver(&mut state, &mut mapper, &test_answer(32, Some(run))), "the map changed");
        assert!(mapper.graph.is_random_exit(1, Direction::E));
        assert_eq!(mapper.graph.connections().iter().find(|c| c.origin == 1 && c.dir == Direction::E), None);
        assert_eq!(mapper.graph.random_destinations(1, Direction::E), &[2, 3]);
    }

    /// (2e) No probe possible: the caller (`turn::finish_command_turn`, or here, the test standing
    /// in for it) resolves the suspicion immediately — the same immediate marking `apply_turn`
    /// always did for this shape before SQ-1269.
    #[test]
    fn no_probe_possible_resolves_the_suspicion_immediately_as_random() {
        let mut mapper = Mapper::default();
        let mut death = DeathWatch::default();
        apply_turn(&mut mapper, "", &TurnResult::observation(snap(1, "Tunnel")), &mut death);
        let mut r = TurnResult::observation(snap(2, "Other Tunnel"));
        r.declared_exit = Some(crate::engine::DeclaredExit::Room(3));
        apply_turn(&mut mapper, "north", &r, &mut death);
        let susp = mapper.take_random_exit_suspicion().expect("pending");

        mapper.resolve_suspicion_as_random(susp);
        assert!(mapper.graph.is_random_exit(1, Direction::N), "marked immediately, no probe involved");
        assert_eq!(mapper.graph.connections().iter().find(|c| c.origin == 1 && c.dir == Direction::N), None);
        assert_eq!(mapper.graph.random_destinations(1, Direction::N), &[2]);
    }

    /// A probe that DID arm but came back BROKEN (`Answer::run: None` — a shadow that would not
    /// boot, or whose restore the engine refused)
    /// resolves exactly like no probe running at all: marked random, same as (2e). Falsify by
    /// reverting `deliver` to require `answer.run` up front for every kind and this fails (the
    /// direction is left showing neither an edge nor a mark).
    #[test]
    fn a_broken_shadow_answer_resolves_a_suspicion_the_same_as_no_probe_at_all() {
        let mut mapper = Mapper::default();
        let mut death = DeathWatch::default();
        apply_turn(&mut mapper, "", &TurnResult::observation(snap(1, "Tunnel")), &mut death);
        let mut r = TurnResult::observation(snap(2, "Other Tunnel"));
        r.declared_exit = Some(crate::engine::DeclaredExit::Room(3));
        apply_turn(&mut mapper, "north", &r, &mut death);
        let susp = mapper.take_random_exit_suspicion().expect("pending");

        let mut state = AppState::default();
        state.random_exit_search =
            Some(arm(susp.origin, susp.dir, susp.live_dest, SearchKind::Suspicion { old_dest: susp.old_dest }, 61));
        assert!(deliver(&mut state, &mut mapper, &test_answer(61, None)), "the map changed");
        assert!(mapper.graph.is_random_exit(1, Direction::N), "marked, same as no probe at all");
        assert_eq!(mapper.graph.connections().iter().find(|c| c.origin == 1 && c.dir == Direction::N), None);
        assert_eq!(mapper.graph.random_destinations(1, Direction::N), &[2]);
    }

    /// (3) A self-loop that a live landing elsewhere contradicts: the room ITSELF is a claimed
    /// destination ("back here"), so this routes through suspicion exactly like a real edge would.
    /// A disagreeing probe answer marks the direction, pools the origin room alongside the live
    /// landing, and the self-loop connection (and its render badge) is gone.
    #[test]
    fn self_loop_suspicion_disagreement_pools_the_room_itself_as_back_here() {
        let mut mapper = Mapper::default();
        let mut death = DeathWatch::default();
        apply_turn(&mut mapper, "", &TurnResult::observation(snap(1, "Tunnel")), &mut death);
        assert!(mapper.record_self_loop(Direction::N), "a loop is recorded first");
        assert_eq!(mapper.graph.self_loops(1), vec![Direction::N]);

        apply_turn(&mut mapper, "north", &TurnResult::observation(snap(5, "Elsewhere")), &mut death);
        assert_eq!(mapper.graph.self_loops(1), vec![Direction::N], "the loop stands — nothing decided yet");
        assert!(!mapper.graph.is_random_exit(1, Direction::N));
        let susp = mapper.take_random_exit_suspicion().expect("the contradicted self-loop left a suspicion pending");
        assert_eq!(
            (susp.origin, susp.dir, susp.old_dest, susp.live_dest),
            (1, Direction::N, Some(1), 5),
            "old_dest is the room ITSELF — a self-loop's destination is the room it leaves"
        );

        let mut state = AppState::default();
        state.random_exit_search =
            Some(arm(susp.origin, susp.dir, susp.live_dest, SearchKind::Suspicion { old_dest: susp.old_dest }, 41));
        let run = ProbeRun { baseline: WorldPrint::default(), steps: vec![step(Some(1)), step(Some(5))] };
        assert!(deliver(&mut state, &mut mapper, &test_answer(41, Some(run))), "the map changed");
        assert!(mapper.graph.is_random_exit(1, Direction::N), "proven random — marked");
        assert!(mapper.graph.self_loops(1).is_empty(), "the self-loop connection is gone");
        assert_eq!(
            mapper.graph.random_destinations(1, Direction::N),
            &[1, 5],
            "the room itself (\"back here\"), then the live landing"
        );

        // The render badge follows: no self-loop badge for this direction any more.
        let rm = mapper::render::render(&mapper.graph);
        let room1 = rm.rooms.iter().find(|r| r.id == 1).expect("room 1 is placed");
        assert!(!room1.self_loops.contains(&Direction::N), "the `?` mark supersedes the loop badge");
    }

    // ── Stale answers for `Suspicion` searches ─────────────────────────────────────────────────

    /// If something else already changed what `(origin, dir)` claims by the time the answer
    /// arrives (the player re-walked it, an edit, …), the search is about a question that is no
    /// longer being asked — `deliver` must decline rather than act on stale evidence.
    #[test]
    fn a_suspicion_answer_declines_when_the_edge_it_was_about_has_already_changed() {
        let mut mapper = Mapper::default();
        let mut death = DeathWatch::default();
        apply_turn(&mut mapper, "", &TurnResult::observation(snap(1, "Origin")), &mut death);
        apply_turn(&mut mapper, "east", &TurnResult::observation(snap(2, "Room A")), &mut death); // 1--E-->2

        // The search believes it is judging the edge to room 2 (as if a contradiction had just
        // been recorded against it), but before the answer arrives something else moves it.
        mapper.graph.remove_connection(1, Direction::E);
        mapper.graph.add_edge(1, Direction::E, 4);

        let mut state = AppState::default();
        state.random_exit_search =
            Some(arm(1, Direction::E, 3, SearchKind::Suspicion { old_dest: Some(2) }, 51));
        let run = ProbeRun { baseline: WorldPrint::default(), steps: vec![step(Some(3)), step(Some(3))] };
        assert!(!deliver(&mut state, &mut mapper, &test_answer(51, Some(run))), "stale — declined");
        assert_eq!(
            mapper.graph.connections().iter().find(|c| c.origin == 1 && c.dir == Direction::E).map(|c| c.dest),
            Some(4),
            "the answer must not clobber whatever changed it in the meantime"
        );
    }

    /// The other staleness shape: the direction is already marked random by the time the answer
    /// arrives (something else resolved the very question this search was asking) — declined.
    #[test]
    fn a_suspicion_answer_declines_when_the_direction_is_already_marked() {
        let mut mapper = Mapper::default();
        let mut death = DeathWatch::default();
        apply_turn(&mut mapper, "", &TurnResult::observation(snap(1, "Origin")), &mut death);
        mapper.record_random_exit(1, Direction::N);

        let mut state = AppState::default();
        state.random_exit_search = Some(arm(1, Direction::N, 2, SearchKind::Suspicion { old_dest: None }, 52));
        let run = ProbeRun { baseline: WorldPrint::default(), steps: vec![step(Some(2)), step(Some(2))] };
        assert!(!deliver(&mut state, &mut mapper, &test_answer(52, Some(run))), "stale — declined");
        assert!(mapper.graph.is_random_exit(1, Direction::N), "still marked, untouched by the stale answer");
    }
}
