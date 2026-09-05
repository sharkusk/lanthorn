use std::collections::BTreeSet;
use crate::direction::{Direction, parse_direction};
use crate::graph::{MapGraph, RoomId};
use crate::layout::{nearest_free_cell, occupied_cells, place_incremental};
use crate::layout::mark_distorted;
use crate::suggest::LayerSuggestion;

#[derive(Debug, Default)]
pub struct Mapper {
    pub graph: MapGraph,
    /// The passage the player last WALKED to reach the current room: the room they
    /// left and the direction they took (SQ-0552). `None` when the current room was
    /// not arrived at by a walked passage — the first room of a game, an involuntary
    /// relocation, or a move whose command named no direction.
    ///
    /// The graph alone cannot answer this: several edges can lead into one room, and
    /// the one just used is not recoverable from them. It is only knowable at the
    /// moment of the move, so it is recorded there. The room is recorded alongside the
    /// direction because the passage is not reliably reciprocal — a bare peel needs the
    /// edge itself, not a guess at the way back.
    pub(crate) arrived_via: Option<(RoomId, Direction)>,
    /// What the map noticed about the move just made, waiting for a prompt to show it (SQ-0439).
    ///
    /// Transient and derived — never persisted, and replaced by every move, because a suggestion is
    /// about the crossing that produced it and a stale one describes a step the player has already
    /// walked away from. What DOES persist is the answer (`MapGraph::seam_decision`).
    pub(crate) pending_suggestion: Option<LayerSuggestion>,
    /// A move `apply_turn` could not settle on its own — a declared-exit mismatch or a live
    /// contradiction against something the map already believed — waiting for a caller with a
    /// probe to judge it (SQ-1269). See [`RandomExitSuspicion`].
    ///
    /// Transient like `pending_suggestion`: set at most once per move, and taken (never merely
    /// read) by whoever resolves it, so a suspicion nobody looked at cannot leak into the next
    /// move's decision.
    pub(crate) pending_random_exit_suspicion: Option<RandomExitSuspicion>,
}

/// A move that CONTRADICTS what the map already believed about `(origin, dir)`, left for a caller
/// with a probe to decide (SQ-1269) — see the module's "suspicion, not proof" design at
/// [`Mapper::note_random_exit_suspicion`].
///
/// `old_dest` is whatever the map already claimed for `(origin, dir)` before this move: `Some(x)`
/// for an existing edge OR self-loop (a self-loop's "destination" is the room itself — `x ==
/// origin` — which is itself a real pooled destination once the direction is marked, the room
/// card's "back here"), `None` when nothing existed yet (a Phase-1 declared-exit mismatch with no
/// prior edge to contradict). `live_dest` is where the player actually landed this move — `==
/// origin` for a same-room arrival that contradicts an edge elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RandomExitSuspicion {
    pub origin: RoomId,
    pub dir: Direction,
    pub old_dest: Option<RoomId>,
    pub live_dest: RoomId,
}

/// One passage a return probe walked: out of `from`, heading `dir`, arriving in `to` (SQ-0785).
///
/// Three facts that are only meaningful together, so they travel together rather than as three
/// arguments — and the shape of the value is itself the guarantee the feature rests on.
///
/// **It cannot express reciprocity.** A search launched after `enter window` discovers that EAST
/// takes you back; the two facts that produces are a TRAVERSAL — the way in is still `enter
/// window`, the command the player actually typed — and a GEOMETRY, that the room they are in
/// sits west of the one they left. Both are carried by this one value and neither can be
/// mistaken for the other, because it names only the passage that was walked: `Kitchen --E-->
/// Behind House`. There is no field in which "and therefore WEST works from Behind House" could
/// be written, which is exactly the invention this feature exists not to make. The geometry
/// follows from the edge, through the ordinary layout the edge is fed to.
///
/// Plain data — three `Copy` scalars — because it crosses a thread boundary: the search runs on
/// the shadow's worker and the graph does not, so what comes back is what was DISCOVERED and
/// never a graph mutation to replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbedPassage {
    /// The room the shadow started in — the room the player is standing in.
    pub from: RoomId,
    /// The direction it walked out of `from`.
    pub dir: Direction,
    /// The room it arrived in — the room the player had just left.
    pub to: RoomId,
}

impl Mapper {
    /// A mapper around a graph loaded from a save. Both of the fields beside the graph describe the
    /// CURRENT session — the passage just walked, and what the map made of it — and a restore has
    /// walked nothing yet, so both start empty.
    pub fn restored(graph: MapGraph) -> Self {
        Mapper { graph, arrived_via: None, pending_suggestion: None, pending_random_exit_suspicion: None }
    }

    /// Observe the player's location after a turn. The conservative form: when the location has
    /// not changed, nothing is minted — the direction is merely recorded as tried.
    pub fn observe(&mut self, location: RoomId, name: &str, via: Option<Direction>) {
        self.observe_inner(location, name, via, false);
    }

    /// [`Mapper::observe`] for a turn where the caller can PROVE the player moved (SQ-0666).
    ///
    /// When the room they arrived in is the room they left, that is a self-loop — Adventure's
    /// maze is full of them, and the old code threw the fact away because it could not tell a
    /// loop from a wall. It still cannot, which is why the proof has to come from the caller: a
    /// direction that bounced off a wall reaches [`Mapper::observe`] and stays a probe (`×`),
    /// while an observed arrival reaches this and mints the loop (`↩`).
    pub fn observe_moved(&mut self, location: RoomId, name: &str, via: Option<Direction>) {
        self.observe_inner(location, name, via, true);
    }

    /// Record an observed same-room arrival directly: `dir` out of the current room leads back
    /// into it. For the retroactive path — a player converting a probe they know is a loop.
    /// Returns false when there is no current room, or `dir` is not a passage.
    pub fn record_self_loop(&mut self, dir: Direction) -> bool {
        let Some(here) = self.graph.current() else { return false };
        self.graph.add_self_loop(here, dir)
    }

    /// Record `dir` out of `origin` as a RANDOM exit (SQ-1257): the room's own declared exit
    /// data named a fixed destination and the player was sent somewhere else — Lost Pig's gnome
    /// tunnels, where a "before going" rule overrides the room's exit table entirely. Mints NO
    /// edge; the caller still moves [`MapGraph::set_current`] to wherever the player actually
    /// landed via [`Mapper::observe_relocation`], exactly as for any other involuntary move. This
    /// call is the one that leaves a trace of the ATTEMPT — without it, a direction the story
    /// randomised is indistinguishable from one nobody has ever tried.
    ///
    /// Returns false for an unknown room or [`Direction::Unknown`], matching
    /// [`Self::record_probed_passage`] and [`Self::record_self_loop`].
    pub fn record_random_exit(&mut self, origin: RoomId, dir: Direction) -> bool {
        if dir == Direction::Unknown || self.graph.room(origin).is_none() {
            return false;
        }
        self.graph.mark_random_exit(origin, dir);
        true
    }

    /// Leave a move `apply_turn` could not settle on its own for a caller with a probe to decide
    /// (SQ-1269): a Phase-1 declared-exit mismatch or a live contradiction against something the
    /// map already believed. Mints nothing and marks nothing — the caller's own move already
    /// relocated the player (via [`Mapper::observe_relocation`]) — this only stashes the fact so
    /// [`Mapper::take_random_exit_suspicion`] can hand it to a probe, or, when no probe can run,
    /// straight to [`Mapper::resolve_suspicion_as_random`] (today's old immediate-marking
    /// behaviour, unchanged in effect).
    pub fn note_random_exit_suspicion(&mut self, origin: RoomId, dir: Direction, old_dest: Option<RoomId>, live_dest: RoomId) {
        self.pending_random_exit_suspicion = Some(RandomExitSuspicion { origin, dir, old_dest, live_dest });
    }

    /// Take the suspicion the last move left, if it left one (SQ-1269). Taking it is how a caller
    /// claims responsibility for resolving it — arming a probe, or resolving it immediately when
    /// none can run — so it is asked at most once.
    pub fn take_random_exit_suspicion(&mut self) -> Option<RandomExitSuspicion> {
        self.pending_random_exit_suspicion.take()
    }

    /// Resolve a [`RandomExitSuspicion`] as PROVEN random (SQ-1269): remove whatever old edge or
    /// self-loop stood on `(origin, dir)`, mark the direction `?`, and pool both the old
    /// destination (if any) and the live landing. Used both when no probe can run at all — the
    /// same immediate marking `apply_turn` always did for this shape before SQ-1269 — and by
    /// `app::random_exit_probe::deliver` when a Phase-2 probe disagrees.
    pub fn resolve_suspicion_as_random(&mut self, s: RandomExitSuspicion) {
        if let Some(old) = s.old_dest {
            self.graph.remove_connection(s.origin, s.dir);
            self.graph.note_random_destination(s.origin, s.dir, old);
        }
        self.record_random_exit(s.origin, s.dir);
        if s.live_dest != s.origin {
            self.graph.note_random_destination(s.origin, s.dir, s.live_dest);
        }
    }

    /// Resolve a [`RandomExitSuspicion`] as PROVEN deterministic but CHANGED (SQ-1269): the
    /// passage used to lead to `old_dest` (or nowhere recorded at all) and a probe confirms it now
    /// reliably leads to `live_dest` instead — remove the stale edge/self-loop and mint the new
    /// one. No mark, no pool: the direction never was, and still is not, random.
    pub fn resolve_suspicion_as_changed(&mut self, s: RandomExitSuspicion) {
        if s.old_dest.is_some() {
            self.graph.remove_connection(s.origin, s.dir);
        }
        if s.live_dest != s.origin {
            self.mint_passage(s.origin, s.dir, s.live_dest);
        } else {
            self.graph.add_self_loop(s.origin, s.dir);
        }
    }

    fn observe_inner(&mut self, location: RoomId, name: &str, via: Option<Direction>, moved: bool) {
        // Asked BEFORE the upsert, because after it every room is a room the map knows. Only this
        // moment can answer it, and the detector needs it: a region that grew by one room is worth
        // mentioning once, while walking back into a room already on the map is not (SQ-0853).
        let newly_seen = self.graph.room(location).is_none();
        // Also captured BEFORE the upsert, which is the only moment this room's PREVIOUS label is
        // still readable — `upsert_room` is about to overwrite it. Read below, only for a same-room
        // move (SQ-1257 Phase 3): a compass move that returns to the room it left is a self-loop
        // ONLY when the story keeps calling that room the same thing; a rename in the same
        // breath is Lost Pig's gnome tunnels re-rolling a fresh name on every step, and gets
        // recorded as a random exit instead. `label()` rather than `name`, so a room pinned by a
        // `label_override` compares against what the player actually sees, not the churn under it.
        let label_before = self.graph.room(location).map(|r| r.label().to_string());
        self.graph.upsert_room(location, name.to_string());
        // Whatever the last move had to say is about the last move; this one answers for itself.
        self.pending_suggestion = None;
        let prev = self.graph.current();
        // Record the direction against the room it was TYPED IN — the one we are leaving, not the
        // one we arrive at — and do it whether or not the move worked (SQ-0391). A direction that
        // bounced off a wall still answers "have I tried this way?", and that case is exactly the
        // one the `location != prev` arm below skips.
        if let (Some(prev_id), Some(d)) = (prev, via) {
            self.graph.mark_tried(prev_id, d);
        }
        match prev {
            None => {
                // First room ever: anchor at the origin. Nothing was walked to
                // reach it, so there is no arrival passage.
                self.arrived_via = None;
                if self.graph.room(location).and_then(|r| r.pos).is_none() {
                    self.graph.set_pos(location, (0, 0));
                }
            }
            Some(prev_id) => {
                if location != prev_id {
                    let edge_dir = via.unwrap_or(Direction::Unknown);
                    self.arrived_via = via.map(|d| (prev_id, d));
                    self.mint_passage(prev_id, edge_dir, location);
                    // Only now, with the passage minted and both rooms placed, does the map have
                    // enough to judge the crossing by (SQ-0439).
                    self.pending_suggestion = crate::suggest::on_arrival(
                        &self.graph,
                        prev_id,
                        edge_dir,
                        location,
                        newly_seen,
                    );
                } else if let (true, Some(d)) = (moved, via) {
                    // The player walked `d` and came out where they went in: a self-loop
                    // (SQ-0666) — UNLESS the story renamed the room in the same breath, which is
                    // Lost Pig's gnome tunnels rerolling a fresh name every step (SQ-1257 Phase
                    // 3). No probe, no declared-exit mismatch: a rename on a same-room arrival is
                    // itself the structural signal, exactly like Phase 1's exit-table mismatch.
                    // Either way no placement and no `collapse_unknown_edges` run here — neither
                    // a loop nor a random mark carries geometry, so there is nothing to lay out
                    // and no `?` stub either could make redundant. `arrived_via` still records
                    // the passage: it IS the one walked, whichever way this resolves.
                    self.arrived_via = Some((prev_id, d));
                    if label_before.as_deref() != Some(name) {
                        self.record_random_exit(prev_id, d);
                    } else {
                        self.graph.add_self_loop(prev_id, d);
                    }
                }
            }
        }
        self.graph.set_current(location);
        // Re-evaluate distortion over the whole graph (cheap); no relayout.
        mark_distorted(&mut self.graph, &BTreeSet::new());
    }

    /// Mint one passage and lay its far end out: the whole of what recording a crossing does to
    /// the GRAPH, and the only place any of it happens.
    ///
    /// Extracted so that a passage a RETURN PROBE discovered (SQ-0785) and a passage the player
    /// walked cannot differ — not in the edge, not in the `?`-stub hygiene that follows it, and
    /// not in the placement. Two code paths that both "record an edge" are two paths that drift;
    /// after the fact there is no such thing as a probed passage anyway, only a passage, because
    /// the map's claim is "this way leads there" and not "this is how we found out".
    ///
    /// What is NOT here is everything that is about the PLAYER rather than the map: who is
    /// standing where ([`MapGraph::set_current`]), which passage they just walked
    /// (`arrived_via`), and whether the crossing was worth remarking on (`pending_suggestion`).
    /// A probe moves nobody and crosses nothing, so it wants exactly this and none of that.
    fn mint_passage(&mut self, from: RoomId, dir: Direction, to: RoomId) {
        self.graph.add_edge(from, dir, to);
        // Drop a now-redundant `?` stub: fires whether the Unknown came first and a
        // directional move just followed, or a directional edge already existed and
        // this move was Unknown. Edge hygiene is independent of layout mode. (SQ-0220)
        self.graph.collapse_unknown_edges();
        place_incremental(&mut self.graph, from, to, dir);
    }

    /// Record a passage a return probe found: `dir` out of `from` leads to `to` (SQ-0785).
    ///
    /// Goes through [`Mapper::mint_passage`], the same call a walked crossing makes, so the edge
    /// is indistinguishable from one the player walked — which is what it is. It persists,
    /// routes, lays out and draws as one, and a save/restore brings it back as one.
    ///
    /// Three things it deliberately does NOT do, each of which would be a lie:
    ///
    /// * it does not move [`MapGraph::set_current`] — the player is still standing in `from`;
    /// * it does not touch `arrived_via` — nobody walked anything;
    /// * it does not mark `dir` TRIED in `from`. The player has not tried it. The SEARCH has, and
    ///   that is [`MapGraph::mark_probed`]'s record, written by the caller per attempt.
    ///
    /// Returns false — recording nothing — when either room is unknown to the map, when `dir` is
    /// [`Direction::Unknown`] (a probe always walks a named direction), or when a passage already
    /// leaves `from` that way. **That last one is the race**: the player may have walked the way
    /// back themselves while the search was running, and a real traversal is the better authority
    /// on its own passage, so the probe's answer stands down rather than overwriting or
    /// duplicating it.
    pub fn record_probed_passage(&mut self, passage: ProbedPassage) -> bool {
        let ProbedPassage { from, dir, to } = passage;
        if dir == Direction::Unknown
            || from == to
            || self.graph.room(from).is_none()
            || self.graph.room(to).is_none()
            || self.graph.connections().iter().any(|c| c.origin == from && c.dir == dir)
        {
            return false;
        }
        self.mint_passage(from, dir, to);
        // The same whole-graph distortion pass `observe_inner` ends on. A new edge can make an
        // existing placement inconsistent, and the drawn map reads `distorted` per connection.
        mark_distorted(&mut self.graph, &BTreeSet::new());
        true
    }

    /// The passage the player last walked to reach the current room — `(room left,
    /// direction taken)` — if it was reached by a walked passage at all. See
    /// [`Mapper::arrived_via`].
    pub fn arrived_via(&self) -> Option<(RoomId, Direction)> {
        self.arrived_via
    }

    /// Take the suggestion the last move produced, if it produced one (SQ-0439).
    ///
    /// Taking it is how a prompt claims it: the map has said its piece and will not say it again
    /// until the player crosses something else worth mentioning.
    pub fn take_suggestion(&mut self) -> Option<LayerSuggestion> {
        self.pending_suggestion.take()
    }

    pub fn observe_command(&mut self, location: RoomId, name: &str, command: &str) {
        self.observe(location, name, parse_direction(command));
    }

    /// Record an *involuntary* relocation — the current room changed, but NOT via a
    /// real passage the player walked (e.g. death + resurrection, or a teleport that
    /// drops the player somewhere unrelated to the command they typed). Move the
    /// current pointer to `location` without minting any edge, so a typed "north"
    /// that got the player killed never mints a false N-edge to the resurrection
    /// room. A previously-unseen resurrection room is added and placed at a free
    /// cell (so it is visible but disconnected); an already-known room keeps its
    /// position. (SQ-0259)
    pub fn observe_relocation(&mut self, location: RoomId, name: &str) {
        // A death/teleport is not a walked passage, so it leaves no arrival
        // direction for a bare peel to cut at — and nothing for the detector to
        // judge either: there is no crossing here to be on either side of.
        self.arrived_via = None;
        self.pending_suggestion = None;
        self.graph.upsert_room(location, name.to_string());
        let prev = self.graph.current();
        if self.graph.room(location).and_then(|r| r.pos).is_none() {
            match prev {
                // First room ever seen (defensive): anchor at the origin.
                None => self.graph.set_pos(location, (0, 0)),
                // New resurrection room: drop it at a free cell near the room we
                // died in, visible but with no edge asserting a connection.
                Some(prev_id) => {
                    let from = self.graph.room(prev_id).and_then(|r| r.pos).unwrap_or((0, 0));
                    let cell = nearest_free_cell(&occupied_cells(&self.graph), from);
                    self.graph.set_pos(location, cell);
                }
            }
        }
        self.graph.set_current(location);
        mark_distorted(&mut self.graph, &BTreeSet::new());
    }

    /// Give room `old` the id `new`, rewriting every reference — the graph's rooms, connections
    /// and current pointer ([`MapGraph::rekey_room`]) AND the mapper's own `arrived_via`, which
    /// the graph knows nothing about. Callers must use this, not the graph method, whenever a
    /// `Mapper` owns the graph: re-keying underneath it leaves `arrived_via` holding the dead id,
    /// and a bare `/move-region` within the one-turn window then refuses with `NoSuchPassage`
    /// (SQ-0632). Same return contract as the graph method.
    pub fn rekey_room(&mut self, old: RoomId, new: RoomId) -> bool {
        let done = self.graph.rekey_room(old, new);
        if done {
            if let Some((room, dir)) = self.arrived_via {
                if room == old {
                    self.arrived_via = Some((new, dir));
                }
            }
        }
        done
    }

    /// Set or clear the label_override for a room.
    pub fn rename_room(&mut self, id: RoomId, label: Option<String>) {
        self.graph.set_label_override(id, label);
    }

    /// Set the notes for a room.
    pub fn set_notes(&mut self, id: RoomId, notes: String) {
        self.graph.set_notes(id, notes);
    }

    /// Remove the connection with key (origin, dir). Returns true if removed.
    pub fn delete_connection(&mut self, origin: RoomId, dir: Direction) -> bool {
        self.graph.remove_connection(origin, dir)
    }

    /// Change the direction of the edge (origin, old) to (origin, new).
    /// Returns true if changed. Refuses (returns false) if (origin, new) already exists.
    pub fn relabel_edge(&mut self, origin: RoomId, old: Direction, new: Direction) -> bool {
        self.graph.relabel_connection(origin, old, new)
    }
}

#[cfg(test)]
mod arrival_tests {
    use super::*;

    /// SQ-0552: a bare `/move-region` cuts the passage the player arrived through, so the
    /// mapper has to remember the whole passage — the room left AND the direction. The
    /// graph cannot answer it: several edges can lead into one room and the one just used
    /// is not recoverable from them.
    #[test]
    fn arrival_passage_is_remembered_and_reset_when_nothing_was_walked() {
        let mut m = Mapper::default();
        m.observe(1, "Hall", Some(Direction::N));
        assert_eq!(m.arrived_via(), None, "the first room was not reached by walking");

        m.observe(2, "Cave", Some(Direction::N));
        assert_eq!(
            m.arrived_via(),
            Some((1, Direction::N)),
            "walked north out of the Hall to get here — that edge is the seam a bare peel cuts"
        );

        m.observe(3, "Vault", None);
        assert_eq!(m.arrived_via(), None, "a move naming no direction leaves no seam to cut");

        m.observe(4, "Crypt", Some(Direction::E));
        assert_eq!(m.arrived_via(), Some((3, Direction::E)));
        m.observe_relocation(5, "Forest");
        assert_eq!(m.arrived_via(), None, "death or teleport is not a walked passage");
    }

    /// SQ-0632: the SQ-0526 Glulx id remap re-keys rooms mapped during the learning window —
    /// including, possibly, the room the player just walked out of. `arrived_via` must follow
    /// the rename, or a bare `/move-region` in the one-turn window refuses with `NoSuchPassage`
    /// because the edge it looks for now hangs off the new id.
    #[test]
    fn rekeying_a_room_carries_arrived_via_with_it() {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(2, "Cave", Some(Direction::N));
        assert_eq!(m.arrived_via(), Some((1, Direction::N)));

        assert!(m.rekey_room(1, 99), "the rename itself succeeds");
        assert_eq!(
            m.arrived_via(),
            Some((99, Direction::N)),
            "arrived_via follows the room it referenced to its new id"
        );
        assert!(
            m.graph.connections().iter().any(|c| c.origin == 99 && c.dir == Direction::N),
            "and the edge it names exists under that id, so a bare peel can find it"
        );

        assert!(m.rekey_room(2, 77), "re-keying the CURRENT room, not the one left");
        assert_eq!(m.arrived_via(), Some((99, Direction::N)), "an unrelated rekey leaves it alone");

        assert!(!m.rekey_room(1234, 5678), "a refused rekey…");
        assert_eq!(m.arrived_via(), Some((99, Direction::N)), "…changes nothing");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;

    #[test]
    fn first_observation_sets_current_no_edge() {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        assert_eq!(m.graph.current(), Some(1));
        assert_eq!(m.graph.connections().len(), 0);
    }

    #[test]
    fn compass_move_creates_directed_edge() {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "Forest", Some(Direction::N));
        assert_eq!(m.graph.connections(), &[crate::graph::Connection{origin:1,dir:Direction::N,dest:2,distorted:false,weight:crate::graph::PassageWeight::Hard}]);
        assert_eq!(m.graph.current(), Some(2));
    }

    #[test]
    fn noncompass_move_creates_unknown_edge_room_not_lost() {
        let mut m = Mapper::default();
        m.observe(1, "Cave Mouth", None);
        m.observe_command(2, "Secret Grotto", "xyzzy"); // teleport
        assert!(m.graph.room(2).is_some());
        assert_eq!(m.graph.connections()[0].dir, Direction::Unknown);
    }

    /// SQ-0632: xyzzy then pray from the same room — the second non-compass passage must not
    /// silently overwrite the first's recorded destination.
    #[test]
    fn two_noncompass_passages_from_one_room_both_survive() {
        let mut m = Mapper::default();
        m.observe(1, "Cave", None);
        m.observe_command(2, "Grotto", "xyzzy");
        m.observe(1, "Cave", Some(Direction::S)); // walk back
        m.observe_command(3, "Chapel", "pray");
        assert!(
            m.graph.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::Unknown && c.dest == 2),
            "the xyzzy passage 1→2 is still recorded: {:?}",
            m.graph.connections()
        );
        assert!(
            m.graph.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::Unknown && c.dest == 3),
            "and the pray passage 1→3 sits beside it"
        );
    }

    #[test]
    fn observe_collapses_unknown_when_directional_edge_appears() {
        // Unknown arrives first (a non-directional move), then the same passage is later walked
        // with a compass command — the redundant `?` 1→2 collapses. (SQ-0220)
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe_command(2, "B", "xyzzy"); // (1, Unknown, 2)
        assert!(
            m.graph.connections().iter().any(|c| c.dir == Direction::Unknown && c.origin == 1 && c.dest == 2),
            "the Unknown 1→2 exists before a directional edge appears"
        );
        m.observe_command(1, "A", "south"); // walk back: (2, S, 1) — reverse, does not collapse
        m.observe_command(2, "B", "north"); // forward directional: (1, N, 2) → Unknown collapses
        assert!(
            !m.graph.connections().iter().any(|c| c.dir == Direction::Unknown),
            "the redundant Unknown 1→2 collapsed once the N edge appeared: {:?}", m.graph.connections()
        );
        assert!(m.graph.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::N && c.dest == 2));
    }

    #[test]
    fn observe_unknown_does_not_persist_when_directional_edge_exists() {
        // A directional edge 1→2 already exists; a later non-directional move over the same
        // passage must not leave a lingering `?` stub. (SQ-0220)
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe_command(2, "B", "north"); // (1, N, 2)
        m.observe_command(1, "A", "south"); // (2, S, 1)
        m.observe_command(2, "B", "xyzzy"); // Unknown 1→2, immediately collapsed
        assert!(
            !m.graph.connections().iter().any(|c| c.dir == Direction::Unknown),
            "an Unknown 1→2 must not persist alongside the existing N edge: {:?}", m.graph.connections()
        );
        assert_eq!(m.graph.current(), Some(2));
    }

    #[test]
    fn relocation_updates_current_without_minting_edge() {
        // Grue death: walk A→(down)→Cellar, then a typed move kills the player and
        // resurrects them in a brand-new Forest. The relocation must move current to
        // Forest but create NO edge (no false passage Cellar→Forest). (SQ-0259)
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "Cellar", Some(Direction::Down));
        let edges_before = m.graph.connections().len();
        m.observe_relocation(3, "Forest");
        assert_eq!(m.graph.current(), Some(3), "current follows the player to the resurrection room");
        assert_eq!(m.graph.connections().len(), edges_before, "an involuntary relocation mints no edge");
        assert!(m.graph.room(3).is_some(), "resurrection room is added to the map");
        assert!(m.graph.room(3).unwrap().pos.is_some(), "resurrection room is placed so it renders");
    }

    #[test]
    fn relocation_to_known_room_keeps_position_and_mints_no_edge() {
        // Resurrecting into an already-mapped room must not move it or connect it.
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "Forest", Some(Direction::N)); // Forest already known & placed
        let forest_pos = m.graph.room(2).unwrap().pos;
        m.observe(1, "A", Some(Direction::S)); // back to A; current = 1
        let edges_before = m.graph.connections().len();
        m.observe_relocation(2, "Forest"); // die in A, resurrect in the known Forest
        assert_eq!(m.graph.current(), Some(2));
        assert_eq!(m.graph.room(2).unwrap().pos, forest_pos, "a known resurrection room does not move");
        assert_eq!(m.graph.connections().len(), edges_before, "no false edge to the known room");
    }

    #[test]
    fn relocation_as_first_observation_anchors_origin() {
        let mut m = Mapper::default();
        m.observe_relocation(1, "Forest");
        assert_eq!(m.graph.current(), Some(1));
        assert_eq!(m.graph.room(1).unwrap().pos, Some((0, 0)));
        assert_eq!(m.graph.connections().len(), 0);
    }

    #[test]
    fn restated_same_room_no_edge() {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(1, "Hall", Some(Direction::N)); // look/again — same room
        assert_eq!(m.graph.connections().len(), 0);
    }

    /// SQ-0666: a maze's "west leads back here" and a wall's "you can't go that way" look
    /// identical to the mapper — same room before, same room after. Only the caller can tell them
    /// apart, so only the caller may mint the loop. `observe` stays conservative; `observe_moved`
    /// is the one that has been given the proof.
    #[test]
    fn a_same_room_turn_mints_a_loop_only_when_the_caller_proves_a_move() {
        let mut m = Mapper::default();
        m.observe(1, "Maze", None);

        m.observe(1, "Maze", Some(Direction::E)); // bounced off a wall
        assert_eq!(m.graph.connections().len(), 0, "a wall must not become a passage");
        assert!(m.graph.is_tried(1, Direction::E), "but it is on the record as tried");
        assert!(m.graph.self_loops(1).is_empty());

        m.observe_moved(1, "Maze", Some(Direction::W)); // walked west, came out here
        assert_eq!(m.graph.self_loops(1), vec![Direction::W], "an observed arrival IS a loop");
        assert_eq!(m.graph.room(1).unwrap().pos, Some((0, 0)), "a loop moves nothing: no geometry");
        assert_eq!(m.arrived_via(), Some((1, Direction::W)), "it is still the passage just walked");

        m.observe_moved(1, "Maze", Some(Direction::W)); // walk it again
        assert_eq!(m.graph.connections().len(), 1, "a second lap is the same loop");

        m.observe_moved(1, "Maze", None); // a proven move with no direction named
        assert_eq!(m.graph.connections().len(), 1, "nothing to record a loop against");

        // A loop never marks itself distorted, however many relayouts run over it.
        assert!(m.graph.connections().iter().all(|c| !c.distorted));
    }

    /// SQ-1257 Phase 3: Lost Pig's gnome tunnels. A compass move that returns to the room it
    /// left AND renames it mints NO self-loop edge — it is recorded as a random exit instead, the
    /// same structural fact Phase 1 records for a declared-exit mismatch. Falsify by reverting the
    /// `label_before` check in `observe_inner` back to unconditional `add_self_loop` and this
    /// fails on the `connections().is_empty()` assertion, reproducing the original symptom (a
    /// self-loop badge and a flickering label instead of a stable room with a `?` exit).
    #[test]
    fn a_same_room_move_that_also_renames_the_room_is_a_random_exit_not_a_loop() {
        let mut m = Mapper::default();
        m.observe_moved(183, "Twisty Cave", None); // first sighting of the tunnels

        m.observe_moved(183, "Confusing Passage", Some(Direction::N));
        assert!(
            m.graph.connections().is_empty(),
            "no self-loop (or any edge) is minted for a rename-loop: {:?}",
            m.graph.connections()
        );
        assert!(m.graph.self_loops(183).is_empty());
        assert!(m.graph.is_random_exit(183, Direction::N), "north is recorded as a random exit");
        assert!(m.graph.is_tried(183, Direction::N), "and it still counts as tried");
        assert_eq!(m.graph.room(183).unwrap().name, "Confusing Passage", "the label is the CURRENT name");
        assert_eq!(
            m.graph.room(183).unwrap().aliases,
            vec!["Twisty Cave"],
            "the old name joins the aliases"
        );
        assert_eq!(m.arrived_via(), Some((183, Direction::N)), "the passage walked is still recorded");

        // A second rename-loop, a different direction: the mark and the alias both accumulate.
        m.observe_moved(183, "Strange Place", Some(Direction::E));
        assert!(m.graph.is_random_exit(183, Direction::E));
        assert_eq!(
            m.graph.room(183).unwrap().aliases,
            vec!["Twisty Cave", "Confusing Passage"]
        );
        assert!(m.graph.connections().is_empty(), "still no edges at all");
    }

    /// The companion case: a same-room move that does NOT rename the room stays an ordinary
    /// self-loop, exactly as SQ-0666 always recorded it — the rename check must not turn every
    /// maze loop into a random exit.
    #[test]
    fn a_same_room_move_with_no_rename_stays_an_ordinary_self_loop() {
        let mut m = Mapper::default();
        m.observe_moved(1, "Maze", None);
        m.observe_moved(1, "Maze", Some(Direction::W)); // same name both times

        assert_eq!(m.graph.self_loops(1), vec![Direction::W]);
        assert!(!m.graph.is_random_exit(1, Direction::W));
        assert!(m.graph.room(1).unwrap().aliases.is_empty(), "no rename, so no alias either");
    }

    /// The retroactive path: a player who KNOWS a probe is a loop can say so, and the fact lands
    /// on the room they are standing in.
    #[test]
    fn record_self_loop_needs_somewhere_to_stand() {
        let mut m = Mapper::default();
        assert!(!m.record_self_loop(Direction::N), "no current room, nothing to record against");
        m.observe(1, "Maze", None);
        assert!(m.record_self_loop(Direction::N));
        assert_eq!(m.graph.self_loops(1), vec![Direction::N]);
    }

    #[test]
    fn first_room_anchors_at_origin() {
        let mut m = Mapper::default();
        m.observe(1, "Start", None);
        assert_eq!(m.graph.room(1).unwrap().pos, Some((0, 0)));
    }

    #[test]
    fn incremental_observe_does_not_move_existing_rooms() {
        use crate::direction::Direction;
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E)); // east of A
        let a = m.graph.room(1).unwrap().pos.unwrap();
        let b = m.graph.room(2).unwrap().pos.unwrap();
        m.observe(3, "C", Some(Direction::E)); // east of B
        // A and B must not have moved (C is placed past them, not into them).
        assert_eq!(m.graph.room(1).unwrap().pos.unwrap(), a, "A stayed put");
        assert_eq!(m.graph.room(2).unwrap().pos.unwrap(), b, "B stayed put");
        assert!(m.graph.room(3).unwrap().pos.unwrap().0 > b.0, "C is east of B");
    }

    #[test]
    fn revisit_adds_edge_without_moving_rooms() {
        use crate::direction::Direction;
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::N));
        let snapshot: Vec<_> = m.graph.rooms().map(|r| (r.id, r.pos)).collect();
        // walk back south to A (already-placed room)
        m.observe(1, "A", Some(Direction::S));
        let after: Vec<_> = m.graph.rooms().map(|r| (r.id, r.pos)).collect();
        assert_eq!(snapshot, after, "returning to a placed room moves nothing");
    }

    #[test]
    fn light_corrections() {
        use crate::direction::Direction;
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe_command(2, "B", "xyzzy"); // unknown edge
        m.rename_room(2, Some("The Grotto".into()));
        assert_eq!(m.graph.room(2).unwrap().label(), "The Grotto");
        m.set_notes(2, "secret".into());
        assert_eq!(m.graph.room(2).unwrap().notes, "secret");
        assert!(m.relabel_edge(1, Direction::Unknown, Direction::Down));
        assert_eq!(m.graph.connections()[0].dir, Direction::Down);
        assert!(m.delete_connection(1, Direction::Down));
        assert_eq!(m.graph.connections().len(), 0);
    }

    #[test]
    fn observe_incremental_shift_beyond_is_not_global_relayout() {
        // Discriminates incremental placement from a from-scratch global solve.
        // Build: A at origin, B north of A. Return to A, then observe C north of A.
        // C's ideal cell (0,-1) is occupied by B, so shift-beyond moves the BLOCKER
        // (B) further north and places the newcomer (C) truthfully at (0,-1) while A
        // stays put. A global relayout never shifts a blocker like this, so these
        // exact coordinates can only come from the incremental path.
        use crate::direction::Direction;
        let mut m = Mapper::default();
        m.observe(1, "A", None);                 // (0,0)
        m.observe(2, "B", Some(Direction::N));    // (0,-1)
        m.observe(1, "A", Some(Direction::S));    // return to A; current=1, A does not move
        m.observe(3, "C", Some(Direction::N));    // N of A: (0,-1) occupied -> shift-beyond
        assert_eq!(m.graph.room(1).unwrap().pos, Some((0, 0)), "A must stay at origin");
        assert_eq!(m.graph.room(3).unwrap().pos, Some((0, -1)), "newcomer C truthfully north of A");
        assert_eq!(m.graph.room(2).unwrap().pos, Some((0, -2)), "blocker B shifted beyond, not the newcomer");
        // no overlap
        let cells: Vec<_> = m.graph.rooms().filter_map(|r| r.pos).collect();
        let set: std::collections::BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len());
    }
}

#[cfg(test)]
mod untried_tests {
    use super::*;

    /// SQ-0391: the map can offer a "where haven't I been?" prompt only if the mapper records
    /// every direction the player TYPES, not just the ones that worked.
    #[test]
    fn a_direction_that_goes_nowhere_still_counts_as_tried() {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        assert_eq!(m.graph.untried(1).len(), 10, "a fresh room has all ten ways untried");

        // A move that WORKS is tried, and so is the edge it minted.
        m.observe(2, "Cave", Some(Direction::N));
        assert!(!m.graph.untried(1).contains(&Direction::N), "north led somewhere");
        assert_eq!(m.graph.untried(1).len(), 9);

        // A move that goes NOWHERE — same room back — is the case worth recording: without it the
        // map would keep offering a wall forever.
        m.observe(2, "Cave", Some(Direction::E));
        assert!(!m.graph.untried(2).contains(&Direction::E), "east was tried and refused");
        m.observe(2, "Cave", Some(Direction::E)); // twice is still once
        assert_eq!(m.graph.untried(2).iter().filter(|d| **d == Direction::E).count(), 0);

        // A command naming no direction records nothing.
        let before = m.graph.untried(2).len();
        m.observe(2, "Cave", None);
        assert_eq!(m.graph.untried(2).len(), before, "a non-directional command tries nothing");
    }

    /// A map saved before this was recorded has no `tried` list, but its EDGES still prove which
    /// ways were walked — those must not come back as untried.
    #[test]
    fn walked_edges_count_as_tried_on_an_older_map() {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(2, "Cave", Some(Direction::N));
        m.graph.room_mut_tried_clear_for_test(1);
        assert!(!m.graph.untried(1).contains(&Direction::N), "the N edge out of #1 says it was walked");
    }
}

/// A passage a return probe discovered goes in through the same door a walked one does
/// (SQ-0785), and stays out of everything that is about the player rather than the map.
#[cfg(test)]
mod probed_passage_tests {
    use super::*;

    /// Behind House, `enter window`, Kitchen — then the shadow finds that EAST comes back.
    ///
    /// The map gains `Kitchen --E--> Behind House` and nothing else: the outbound passage keeps
    /// the traversal the player actually used, and nothing anywhere claims that WEST works from
    /// Behind House. That claim is reciprocity in a new costume, and the value the crossing
    /// carries cannot express it — `record_probed_passage` names only the passage it walked.
    #[test]
    fn a_discovered_return_leaves_the_outbound_traversal_alone() {
        let mut m = Mapper::default();
        m.observe(1, "Behind House", None);
        m.observe(2, "Kitchen", Some(Direction::In)); // `enter window` parses as In
        assert_eq!(m.graph.current(), Some(2));

        assert!(m.record_probed_passage(ProbedPassage { from: 2, dir: Direction::E, to: 1 }));
        assert!(
            m.graph.connections().iter().any(|c| c.origin == 2 && c.dir == Direction::E && c.dest == 1),
            "the way back is on the map: {:?}",
            m.graph.connections()
        );
        assert!(
            !m.graph.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::W),
            "and west out of Behind House was NOT invented: {:?}",
            m.graph.connections()
        );
        assert!(
            m.graph.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::In && c.dest == 2),
            "the outbound passage is still the one the player used"
        );
    }

    /// A probe moves nobody and crosses nothing, so none of the player-facing state moves with
    /// the edge — and in particular the direction is not marked TRIED, which would take a real
    /// unexplored exit off the map.
    #[test]
    fn recording_a_probed_passage_moves_neither_the_player_nor_the_frontier() {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(2, "Cave", Some(Direction::N));
        let arrived = m.arrived_via();
        let _ = m.take_suggestion();

        assert!(m.record_probed_passage(ProbedPassage { from: 2, dir: Direction::S, to: 1 }));
        assert_eq!(m.graph.current(), Some(2), "the player is still in the room they are in");
        assert_eq!(m.arrived_via(), arrived, "nobody walked anything");
        assert!(m.take_suggestion().is_none(), "and there was no crossing to remark on");
        assert!(
            !m.graph.room(2).unwrap().tried.contains(&Direction::S),
            "the PLAYER's record is untouched"
        );
    }

    /// The race the search has to lose: the player may walk the way back themselves while it is
    /// running. A real traversal is the better authority on its own passage, so the answer that
    /// lands afterwards is a no-op — not an overwrite, not a duplicate.
    #[test]
    fn a_passage_the_player_already_walked_wins_the_race() {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(2, "Cave", Some(Direction::N));
        m.observe(1, "Hall", Some(Direction::S)); // the player walked back themselves
        let before = m.graph.connections().to_vec();

        assert!(!m.record_probed_passage(ProbedPassage { from: 2, dir: Direction::S, to: 1 }), "the probe stands down");
        assert_eq!(m.graph.connections(), before.as_slice(), "and changed nothing at all");

        // Even when the probe's answer DISAGREES about where south goes, the walked edge stands.
        m.graph.upsert_room(3, "Attic".into());
        assert!(!m.record_probed_passage(ProbedPassage { from: 2, dir: Direction::S, to: 3 }));
        assert_eq!(m.graph.connections(), before.as_slice());
    }

    /// The refusals that are not about the race: an unknown room, a self-passage, and the
    /// `?` bucket — a probe always walks a direction it named.
    #[test]
    fn a_probed_passage_refuses_what_it_cannot_honestly_record() {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(2, "Cave", Some(Direction::N));
        assert!(!m.record_probed_passage(ProbedPassage { from: 2, dir: Direction::Unknown, to: 1 }));
        assert!(!m.record_probed_passage(ProbedPassage { from: 2, dir: Direction::S, to: 2 }));
        assert!(!m.record_probed_passage(ProbedPassage { from: 2, dir: Direction::S, to: 404 }));
        assert!(!m.record_probed_passage(ProbedPassage { from: 404, dir: Direction::S, to: 1 }));
        assert_eq!(m.graph.connections().len(), 1, "still only the walked edge");
    }

    /// After the fact there is no such thing as a probed passage, only a passage — so it must
    /// survive the archive exactly as a walked one does, through the same save path, and come
    /// back indistinguishable from it.
    #[test]
    fn a_discovered_passage_round_trips_the_archive_as_an_ordinary_one() {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(2, "Cave", Some(Direction::N));
        m.graph.mark_probed(2, Direction::E); // an attempt that failed, on the record
        assert!(m.record_probed_passage(ProbedPassage { from: 2, dir: Direction::S, to: 1 }));

        let back = crate::persist::from_json(&crate::persist::to_json(&m)).expect("round trip");
        let g = back.graph;
        let walked: Vec<_> =
            g.connections().iter().filter(|c| c.origin == 1 && c.dir == Direction::N).collect();
        let found: Vec<_> =
            g.connections().iter().filter(|c| c.origin == 2 && c.dir == Direction::S).collect();
        assert_eq!(walked.len(), 1);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].dest, 1);
        assert_eq!(
            (found[0].distorted, found[0].dir == Direction::S),
            (walked[0].distorted, true),
            "nothing on the connection says how it was found"
        );
        assert!(g.is_probed(2, Direction::E), "and the search's own progress came back with it");
        assert!(!g.is_probed(2, Direction::W));
    }
}
