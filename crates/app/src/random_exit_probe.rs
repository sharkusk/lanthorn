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
//! rolled dice for this move; agreement on all three is evidence it did not,
//! and the edge Phase 1 already minted stands.
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
//!
//! # Sticky, and why this module never re-asks
//!
//! Once a direction is marked random ([`mapper::graph::MapGraph::mark_random_exit`]),
//! `session::apply_turn`'s own sticky check stops minting an edge for it at
//! all — see the comment there — so there is nothing left for this module to
//! confirm or retract on a later walk of the same direction. `arm` is never
//! even called for one: `turn::finish_command_turn` checks
//! [`mapper::graph::MapGraph::is_random_exit`] before arming, the same way it
//! checks `DeclaredExit`.

use mapper::direction::{long_label, Direction};
use mapper::graph::RoomId;
use mapper::mapper::Mapper;

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
    /// Where the LIVE player actually landed this turn — the edge Phase 1
    /// already minted, and the ground truth both shadow walks are judged
    /// against.
    live_dest: RoomId,
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
}

/// Start a Phase-2 search, if this turn earned one.
///
/// Called once per turn from `turn::finish_command_turn`, after `apply_turn`
/// has settled the move. The caller is responsible for the gate — this
/// function assumes it is worth asking and only refuses on infrastructure
/// grounds (unarmed shadow, busy, no seed to read, no usable pre-move
/// snapshot): whether `dir`'s `DeclaredExit` was `Absent`/`Code` and whether
/// it is already marked random are `finish_command_turn`'s own checks, made
/// with `DeclaredExit` and [`mapper::graph::MapGraph::is_random_exit`], which
/// this module does not read.
pub fn arm_random_exit_search(
    state: &mut AppState,
    live: &dyn Engine,
    origin: RoomId,
    dir: Direction,
    live_dest: RoomId,
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
        return; // busy, unarmed, or mid-save — the edge Phase 1 minted simply stands
    };
    state.random_exit_search = Some(RandomExitSearch { origin, dir, live_dest, token });
}

/// True when `token` answers the search running now, if any.
pub fn owns(state: &AppState, token: u64) -> bool {
    state.random_exit_search.as_ref().is_some_and(|s| s.token == token)
}

/// Judge a Phase-2 answer (SQ-1257).
///
/// Returns true when the map changed (an edge was deleted and the direction
/// marked random) — what tells the caller to bump the graph generation and
/// redraw, the same signal [`crate::return_probe::deliver`] gives.
///
/// # Evidence, not a vote
///
/// A shadow step that quit, escaped, or could not say where it landed is
/// INCONCLUSIVE and counts toward neither side — an unanswerable question is
/// not evidence the story is deterministic, and treating it as agreement
/// would let a shadow that merely failed to boot silently rubber-stamp every
/// edge. Only a shadow step that DID land somewhere, and landed somewhere
/// OTHER than the live destination, is evidence of randomness; only when at
/// least one step gives usable evidence at all does the search have anything
/// to decide with. No usable evidence, or every usable step agreeing with the
/// live destination, both keep the edge Phase 1 already minted — an
/// unproven deletion is exactly the kind of invented fact this module exists
/// to avoid on the minting side.
///
/// # Staleness
///
/// Mirrors the return probe's silence discipline (SQ-1124), adapted for what
/// this search actually needs to be true: the edge it is judging must still
/// exist, AS MINTED (same origin, direction, and destination), or the player
/// has since moved the map on in some way this answer cannot speak to, and
/// the answer is dropped rather than acted on.
pub fn deliver(state: &mut AppState, mapper: &mut Mapper, answer: &crate::probe::Answer) -> bool {
    let Some(search) = state.random_exit_search.take() else { return false };
    if search.token != answer.token {
        state.random_exit_search = Some(search); // not ours; leave it running
        return false;
    }
    let Some(run) = &answer.run else { return false };
    if !mapper.graph.connections().iter().any(|c| {
        c.origin == search.origin && c.dir == search.dir && c.dest == search.live_dest
    }) {
        return false; // the edge this search was about is gone or changed; nothing to judge
    }

    let mut any_evidence = false;
    let mut any_disagree = false;
    for step in &run.steps {
        if step.quit || step.escaped {
            continue;
        }
        let Some(loc) = step.location else { continue };
        any_evidence = true;
        if loc != search.live_dest {
            any_disagree = true;
        }
    }
    if !any_evidence || !any_disagree {
        return false; // no usable evidence, or full agreement — the edge stands
    }

    mapper.graph.remove_connection(search.origin, search.dir);
    mapper.record_random_exit(search.origin, search.dir);
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
