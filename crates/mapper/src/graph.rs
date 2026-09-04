use std::collections::BTreeMap;

use crate::direction::Direction;
use crate::layer::{LayerId, LayerMeta, MapView, MAIN_LAYER};
use crate::suggest::{SeamDecision, SeamKey};

pub type RoomId = u32;

/// Sentinel used only on the wire: a room whose save predates SQ-0685 has no `seq` field at all,
/// and deserializes to this rather than a real ordinal. [`MapGraph::from_parts`] recognises it and
/// backfills the room's true seq from its position in the persisted rooms array before anything
/// else ever reads the field — no in-memory `Room` should carry this value once construction is
/// done. Practically unreachable as a REAL sequence number (it would take 2^64 discoveries).
pub(crate) const ROOM_SEQ_MISSING: u64 = u64::MAX;

fn room_seq_missing() -> u64 {
    ROOM_SEQ_MISSING
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Room {
    pub id: RoomId,
    pub name: String,
    pub label_override: Option<String>,
    pub notes: String,
    pub pos: Option<(i32, i32)>,
    #[serde(default)]
    pub layer: crate::layer::LayerId,
    /// How the mapper worked out that the player was here, recorded the first
    /// time the room was discovered and kept thereafter (SQ-0527). Stored as the
    /// display label rather than an engine enum, so the map file stays readable
    /// and the mapper keeps no dependency on any particular VM crate. `None` for
    /// rooms mapped before this was recorded.
    #[serde(default)]
    pub loc_method: Option<String>,
    /// Compass directions the player has TYPED while standing in this room, whether or not the
    /// move worked (SQ-0391). What it answers is "where have I not tried yet?", so a direction
    /// that bounced off a wall counts as tried just as much as one that led somewhere — the map
    /// stops nagging about it either way.
    ///
    /// A `Vec` rather than a set because [`Direction`] is deliberately not `Ord`, and the list is
    /// at most eight long. Absent from older map files, hence `serde(default)`.
    #[serde(default)]
    pub tried: Vec<Direction>,
    /// Directions a RETURN PROBE has already tried from this room (SQ-0785) — the shadow's
    /// record, kept strictly apart from [`Room::tried`], which is the player's.
    ///
    /// The two answer different questions and must never be merged. `tried` answers "where have
    /// I not been yet?" and drives [`MapGraph::untried`], which is what the map offers as
    /// unexplored; a direction a silent copy of the game tried on the player's behalf has not
    /// been explored by anyone, and folding it in would quietly steer the player away from real
    /// content. This one answers only "is there any point probing that way again?", and its whole
    /// job is to stop the search re-walking ground it has already covered.
    ///
    /// Marked one attempt at a time, as the search makes them, so an aborted search resumes where
    /// it stopped rather than starting over. Persisted for the same reason the player's record is:
    /// the search then converges permanently instead of once per session.
    ///
    /// A `Vec` for the same reason as `tried` ([`Direction`] is deliberately not `Ord`), and at
    /// most twelve long. Absent from older map files, hence `serde(default)`.
    #[serde(default)]
    pub probed: Vec<Direction>,
    /// Compass directions this room's own map data declared a FIXED
    /// destination for, that the player nonetheless left through and arrived
    /// somewhere else (SQ-1257) — Lost Pig's gnome tunnels, where the story's
    /// exit table names nothing and a "before going" rule sends the player to
    /// a random cave. Recorded as a fact about the ROOM, not an edge: a
    /// destination that varies is not a passage `Reciprocal`/`OneWay`/`SelfLoop`
    /// could name truthfully, so the matrix reports a separate cell
    /// ([`crate::matrix::MatrixCell::Random`]) instead of inventing one.
    ///
    /// A `Vec` for the same reason as `tried`/`probed`. Absent from older map
    /// files, hence `serde(default)`.
    #[serde(default)]
    pub random_exits: Vec<Direction>,
    /// Every distinct room a marked direction (see [`Room::random_exits`]) has actually landed
    /// in, first-seen order, no duplicates (SQ-1261). One entry per marked direction; a direction
    /// not yet in [`Room::random_exits`] has no entry here either.
    ///
    /// A `?` mark records only that a direction is random — this is what lets the room card and
    /// the map say WHERE it has sent the player, without pretending the list is exhaustive (the
    /// story may still have destinations nobody has landed in yet) or that any one of them is the
    /// "real" answer. Never touched by a rename-loop: a same-room arrival under a new name has no
    /// destination to record, since the room the player is standing in is the origin itself.
    ///
    /// A `Vec<(Direction, Vec<RoomId>)>` rather than a map, for the same reason as `tried` and
    /// `random_exits` — [`Direction`] is deliberately not `Ord` — and because the whole thing is
    /// at most eight entries long. Absent from a map file written before this existed, hence
    /// `serde(default)`.
    #[serde(default)]
    pub random_destinations: Vec<(Direction, Vec<RoomId>)>,
    /// Every distinct name the game has printed for this room, other than its CURRENT label
    /// (SQ-1257 Phase 3) — Lost Pig's gnome tunnels reroll a fresh name on every compass move,
    /// and this is where the others go so the map can keep saying "this is the same room" while
    /// showing what the story is calling it right now. First-seen order, no duplicates; the
    /// current label is never a member of its own list.
    ///
    /// Maintained by [`Room::note_name_change`], the one place a name transition happens.
    /// Absent from a map file written before this existed, hence `serde(default)`.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Monotonic discovery order, stamped once by [`MapGraph::upsert_room`] the first time this
    /// room is minted and never touched again (SQ-0685). This — not the room id, which for a
    /// Z-machine game is the story's own object number and has nothing to do with when the player
    /// found the room — is what maze numbering ("Maze 3") is ordered by, so finding a low-id
    /// duplicate late never renumbers the ones found earlier.
    ///
    /// `#[serde(default = "room_seq_missing")]` rather than `0`: a save written before this field
    /// existed must be distinguishable from a legitimately-first room, or the backfill in
    /// [`MapGraph::from_parts`] could not tell "missing" from "really seq 0".
    #[serde(default = "room_seq_missing")]
    pub seq: u64,
}

impl Room {
    pub fn label(&self) -> &str {
        match &self.label_override {
            Some(l) => l.as_str(),
            None => self.name.as_str(),
        }
    }

    /// This room's 1-based per-map ordinal — "1" for the first room ever discovered, "2" for the
    /// second, and so on (SQ-1300). Exactly `seq + 1`: `seq` already stamps first-discovery order
    /// once and never renumbers a room afterward (survives a rename, a re-key, a tidy pass), which
    /// is everything a small per-map number for a synthetic (Glulx/name-only) room needs — so this
    /// reuses it rather than carrying a second, parallel counter that could drift from the first.
    /// `app::roomid::room_label_no` is the one place this is ever shown to a player.
    pub fn ordinal(&self) -> u64 {
        self.seq + 1
    }

    /// Record that this room's printed name is about to change to `new_name` (SQ-1257 Phase 3).
    /// Only called when the name is genuinely different — [`MapGraph::upsert_room`]'s revisit
    /// branch checks that before calling this, so it never has to no-op on a same-name
    /// observation.
    ///
    /// The raw name this room carried a moment ago joins `aliases` (deduplicated) and
    /// `new_name` is pulled back out of `aliases` if an earlier rename had put it there: the
    /// list holds every OTHER printed name, never the room's current one. Tracked against the
    /// raw `name` field rather than [`Room::label`] — a `label_override` pins the DISPLAY, but
    /// the story keeps printing its own names underneath it, and those are still worth
    /// remembering as aliases even while the override hides the churn.
    fn note_name_change(&mut self, new_name: String) {
        let old_name = std::mem::replace(&mut self.name, new_name.clone());
        self.aliases.retain(|a| *a != new_name);
        if !self.aliases.contains(&old_name) {
            self.aliases.push(old_name);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Connection {
    pub origin: RoomId,
    pub dir: Direction,
    pub dest: RoomId,
    pub distorted: bool,
}

impl Connection {
    /// True when this edge leads back into the room it leaves — "west takes me back here"
    /// (SQ-0666). Recordable knowledge, but not GEOMETRY: it says nothing about where any room
    /// is, so layout, routing and distortion skip it (the drawn view shows it as a badge on the
    /// room box, the matrix view as `↩`). Feeding one to a compass-offset placer would ask
    /// where a room sits relative to itself, and answer "distorted" forever.
    pub fn is_self_loop(&self) -> bool {
        self.origin == self.dest
    }
}

#[derive(Debug, Clone)]
pub struct MapGraph {
    rooms: BTreeMap<RoomId, Room>,
    conns: Vec<Connection>,
    current: Option<RoomId>,
    layers: BTreeMap<LayerId, LayerMeta>,
    next_layer_id: LayerId,
    /// The next discovery ordinal [`MapGraph::upsert_room`] will stamp on a newly-minted room
    /// (SQ-0685). Persisted so numbering stays stable across a save/load round trip; on a save
    /// from before this existed, [`MapGraph::from_parts`] resumes it past the backfilled max.
    next_seq: u64,
    /// The last room the player stood in on each layer (SQ-0672), keyed by that room's layer AT
    /// THE MOMENT of the visit. Updated by [`MapGraph::set_current`] — the one place the current
    /// room ever changes — so every path that moves the player (a walked step, a relocation, a
    /// restore) keeps it current for free. A room recorded here can later be peeled/merged to a
    /// DIFFERENT layer, which leaves this entry stale; callers treat that exactly like a dangling
    /// id (see the `layer_of` re-check at the recenter call site) rather than this map trying to
    /// stay in sync with every layer edit.
    last_visited: BTreeMap<LayerId, RoomId>,
    /// What the player has already said about each layer-suggestion prompt (SQ-0439), keyed by the
    /// passage the prompt was about. Absent means [`SeamDecision::Armed`] — never asked — so the
    /// map only ever carries the seams the player has actually answered for.
    ///
    /// These are DECISIONS, not derived state: nothing can recompute "the player told us to stop
    /// asking about the trapdoor", so unlike everything else the detector uses, this has to be
    /// carried in the save.
    seam_decisions: BTreeMap<SeamKey, SeamDecision>,
    /// "Never for this story" (SQ-1298): the player has said the layer-suggestion prompt itself is
    /// unwelcome on this map, not merely at one seam. Unlike `seam_decisions` this is a single flag
    /// rather than something keyed — there is only one story per graph — but it is the same kind of
    /// thing: a DECISION nothing can recompute, so it is carried in the save alongside them.
    suggestions_disabled: bool,
}

impl Default for MapGraph {
    fn default() -> Self {
        let mut layers = BTreeMap::new();
        layers.insert(MAIN_LAYER, LayerMeta::main());
        Self {
            rooms: BTreeMap::new(),
            conns: Vec::new(),
            current: None,
            layers,
            next_layer_id: 1,
            next_seq: 0,
            last_visited: BTreeMap::new(),
            seam_decisions: BTreeMap::new(),
            suggestions_disabled: false,
        }
    }
}

impl MapGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reconstruct a `MapGraph` from persisted vecs. Builds the internal `BTreeMap` keyed by id.
    ///
    /// References are validated: a connection whose endpoint is not a room is dropped, and a
    /// `current` naming no room is reset. A hand-edited or corrupt map file could otherwise
    /// smuggle phantom ids into layout components (`connected_components` adjacency-inserts
    /// every connection endpoint), permanently wasting a grid cell and flagging a stray
    /// distorted edge (SQ-0632).
    ///
    /// `seq` back-compat (SQ-0685): a save written before discovery order was tracked has every
    /// room's `seq` at [`ROOM_SEQ_MISSING`] (the wire sentinel `Room::seq` deserializes to when
    /// the field is absent). Those are backfilled from the room's POSITION IN `rooms` — the array
    /// is insertion-ordered, i.e. the true historical first-visit order, so this settles a maze's
    /// numbering to real visit order the first time such a save loads. `next_seq` then resumes
    /// past the highest seq now in play (backfilled or persisted), whichever is greater.
    pub fn from_parts(
        rooms: Vec<Room>,
        connections: Vec<Connection>,
        current: Option<RoomId>,
        layers: BTreeMap<LayerId, LayerMeta>,
        next_layer_id: LayerId,
        last_visited: BTreeMap<LayerId, RoomId>,
        next_seq: u64,
    ) -> Self {
        let mut rooms = rooms;
        for (i, r) in rooms.iter_mut().enumerate() {
            if r.seq == ROOM_SEQ_MISSING {
                r.seq = i as u64;
            }
        }
        let next_seq = rooms.iter().map(|r| r.seq).max().map_or(next_seq, |m| next_seq.max(m + 1));
        let rooms: BTreeMap<RoomId, Room> = rooms.into_iter().map(|r| (r.id, r)).collect();
        let conns: Vec<Connection> = connections
            .into_iter()
            .filter(|c| rooms.contains_key(&c.origin) && rooms.contains_key(&c.dest))
            .collect();
        let current = current.filter(|id| rooms.contains_key(id));
        let mut layers = layers;
        if layers.is_empty() {
            layers.insert(MAIN_LAYER, LayerMeta::main());
        }
        let next_layer_id = next_layer_id.max(1);
        // A dangling last-visited room id (hand-edited or corrupt file, same hazard as
        // `connections`/`current` above) is dropped rather than kept around to misdirect a
        // layer-switch recenter at a room that no longer exists (SQ-0672).
        let last_visited: BTreeMap<LayerId, RoomId> = last_visited
            .into_iter()
            .filter(|(_, room)| rooms.contains_key(room))
            .collect();
        Self {
            rooms,
            conns,
            current,
            layers,
            next_layer_id,
            next_seq,
            last_visited,
            // Restored separately (`restore_seam_decisions`) rather than as an eighth positional
            // argument: this list validates against the rooms `from_parts` has just settled.
            seam_decisions: BTreeMap::new(),
            // Restored separately too (`set_suggestions_disabled`) — a bare bool has nothing to
            // validate against the rooms, but going through the same setter as every other caller
            // keeps this one path the only place the flag is ever written.
            suggestions_disabled: false,
        }
    }

    /// What the player has already said about the suggestion at `key` (SQ-0439). A seam nobody has
    /// answered for is [`SeamDecision::Armed`].
    pub fn seam_decision(&self, key: SeamKey) -> SeamDecision {
        self.seam_decisions.get(&key).copied().unwrap_or_default()
    }

    /// Record the player's answer at `key`. Setting it back to [`SeamDecision::Armed`] forgets the
    /// seam outright, so the map carries only answers actually given.
    pub fn set_seam_decision(&mut self, key: SeamKey, decision: SeamDecision) {
        match decision {
            SeamDecision::Armed => self.seam_decisions.remove(&key),
            other => self.seam_decisions.insert(key, other),
        };
    }

    /// Every answer the player has given, for persistence. Nothing else should need it.
    pub fn seam_decisions(&self) -> &BTreeMap<SeamKey, SeamDecision> {
        &self.seam_decisions
    }

    /// Reinstate persisted seam answers, dropping any that name a room this map no longer has —
    /// the same hygiene `from_parts` applies to connections, `current` and `last_visited`, and for
    /// the same reason: a phantom id here would silence a prompt about a passage that cannot exist.
    pub fn restore_seam_decisions(
        &mut self,
        entries: impl IntoIterator<Item = (SeamKey, SeamDecision)>,
    ) {
        self.seam_decisions = entries
            .into_iter()
            .filter(|(k, _)| self.rooms.contains_key(&k.from))
            .collect();
    }

    /// True once the player has told the layer-suggestion prompt "Never for this story" (SQ-1298).
    /// Unlike a per-seam [`SeamDecision::Ignored`] this is not keyed to any one passage: it stops
    /// `mapper::suggest` from minting a suggestion at all, structural or maze-name alike.
    pub fn suggestions_disabled(&self) -> bool {
        self.suggestions_disabled
    }

    /// Set/clear the story-wide "never suggest layers" flag.
    pub fn set_suggestions_disabled(&mut self, disabled: bool) {
        self.suggestions_disabled = disabled;
    }

    pub fn room(&self, id: RoomId) -> Option<&Room> {
        self.rooms.get(&id)
    }

    pub fn rooms(&self) -> impl Iterator<Item = &Room> {
        self.rooms.values()
    }

    pub fn connections(&self) -> &[Connection] {
        &self.conns
    }

    pub fn current(&self) -> Option<RoomId> {
        self.current
    }

    pub fn layer_of(&self, id: RoomId) -> LayerId {
        self.rooms.get(&id).map(|r| r.layer).unwrap_or(MAIN_LAYER)
    }

    pub fn set_room_layer(&mut self, id: RoomId, layer: LayerId) {
        if let Some(r) = self.rooms.get_mut(&id) { r.layer = layer; }
    }

    pub fn rooms_in_layer(&self, layer: LayerId) -> Vec<RoomId> {
        let mut v: Vec<RoomId> = self.rooms.values().filter(|r| r.layer == layer).map(|r| r.id).collect();
        v.sort();
        v
    }

    pub fn layers(&self) -> &BTreeMap<LayerId, LayerMeta> { &self.layers }

    pub fn layer_name(&self, layer: LayerId) -> &str {
        self.layers.get(&layer).map(|m| m.name.as_str()).unwrap_or("")
    }

    pub fn set_layer_name(&mut self, layer: LayerId, name: String) {
        if let Some(m) = self.layers.get_mut(&layer) { m.name = name; }
    }

    pub fn new_layer(&mut self, parent: Option<LayerId>, name: String) -> LayerId {
        let id = self.next_layer_id;
        self.next_layer_id += 1;
        self.layers.insert(id, LayerMeta::new(name, parent));
        id
    }

    /// True when the player has flagged `layer` as a maze (SQ-0666).
    pub fn layer_is_maze(&self, layer: LayerId) -> bool {
        self.layers.get(&layer).is_some_and(|m| m.maze)
    }

    /// Flag/unflag `layer` as a maze. Returns the new value (unchanged for an unknown layer).
    /// The flag only decides the DEFAULT view, so a layer whose view the player chose by hand
    /// keeps that choice either way.
    pub fn set_layer_maze(&mut self, layer: LayerId, maze: bool) -> bool {
        if let Some(m) = self.layers.get_mut(&layer) {
            m.maze = maze;
        }
        self.layer_is_maze(layer)
    }

    /// The view `layer` draws in — the player's explicit choice, else the maze-flag default.
    pub fn layer_view(&self, layer: LayerId) -> MapView {
        self.layers.get(&layer).map(|m| m.effective_view()).unwrap_or_default()
    }

    /// The player's EXPLICIT view choice for `layer`, or `None` when they have not made one.
    pub fn layer_view_choice(&self, layer: LayerId) -> Option<MapView> {
        self.layers.get(&layer).and_then(|m| m.view)
    }

    /// Set (or clear, with `None`) the player's explicit view choice for `layer`.
    pub fn set_layer_view(&mut self, layer: LayerId, view: Option<MapView>) {
        if let Some(m) = self.layers.get_mut(&layer) {
            m.view = view;
        }
    }

    pub fn remove_layer(&mut self, layer: LayerId) {
        if layer != MAIN_LAYER { self.layers.remove(&layer); }
    }

    pub fn next_layer_id(&self) -> LayerId { self.next_layer_id }

    /// The discovery ordinal [`MapGraph::upsert_room`] will stamp on the NEXT newly-minted room
    /// (SQ-0685). Exposed for persistence; nothing else should need it.
    pub fn next_seq(&self) -> u64 { self.next_seq }

    pub fn upsert_room(&mut self, id: RoomId, name: String) -> &mut Room {
        use std::collections::btree_map::Entry;
        match self.rooms.entry(id) {
            Entry::Occupied(e) => {
                let room = e.into_mut();
                if room.name != name {
                    room.note_name_change(name);
                }
            }
            Entry::Vacant(e) => {
                let seq = self.next_seq;
                self.next_seq += 1;
                e.insert(Room {
                    id,
                    name,
                    label_override: None,
                    notes: String::new(),
                    pos: None,
                    layer: MAIN_LAYER,
                    loc_method: None,
                    tried: Vec::new(),
                    probed: Vec::new(),
                    random_exits: Vec::new(),
                    random_destinations: Vec::new(),
                    aliases: Vec::new(),
                    seq,
                });
            }
        }
        self.rooms.get_mut(&id).unwrap()
    }

    pub fn add_edge(&mut self, origin: RoomId, dir: Direction, dest: RoomId) {
        // A compass direction (or Up/Down/In/Out) can only lead one place, so those edges are
        // keyed (origin, dir) and a repeat observation updates the destination. Unknown is not a
        // direction but a bucket: a room can hold SEVERAL non-compass passages (xyzzy and pray
        // from the same room), so Unknown edges are keyed by the full (origin, dir, dest) triple —
        // a second passage adds a second `?` stub instead of silently erasing the first, and an
        // exact repeat is still one edge (SQ-0632). Layout already treats Unknown as non-spatial
        // (skipped in components, drawn as a stub), so multiples cost nothing there.
        //
        // A SELF-LOOP (`origin == dest`) is triple-keyed for the same reason (SQ-0666): "west
        // leads back here" is a fact about a maze, and it must neither erase a known passage
        // that shares its key nor be erased by one. Keeping both lets the graph hold the
        // contradiction honestly — the matrix view prefers the real destination and falls back
        // to `↩` — rather than silently picking a winner.
        if dir == Direction::Unknown || origin == dest {
            if !self.conns.iter().any(|c| c.origin == origin && c.dir == dir && c.dest == dest) {
                self.conns.push(Connection { origin, dir, dest, distorted: false });
            }
        } else if let Some(conn) =
            self.conns.iter_mut().find(|c| c.origin == origin && c.dir == dir && c.dest != c.origin)
        {
            conn.dest = dest;
        } else {
            self.conns.push(Connection { origin, dir, dest, distorted: false });
        }
    }

    /// Record that `dir` out of `room` leads back INTO `room` — an observed same-room arrival
    /// (SQ-0666). Returns false for an unknown room or a direction with no meaning
    /// ([`Direction::Unknown`], which is a bucket, not a passage).
    ///
    /// Only an observed arrival may call this. A direction that was TYPED and went nowhere is
    /// already recorded by [`MapGraph::mark_tried`] and shows as "tried, no path"; guessing that
    /// every such direction is a loop would invent passages out of walls.
    pub fn add_self_loop(&mut self, room: RoomId, dir: Direction) -> bool {
        if dir == Direction::Unknown || !self.rooms.contains_key(&room) {
            return false;
        }
        self.add_edge(room, dir, room);
        self.mark_tried(room, dir);
        true
    }

    /// The self-loop directions recorded for `room`, in connection order.
    pub fn self_loops(&self, room: RoomId) -> Vec<Direction> {
        self.conns
            .iter()
            .filter(|c| c.origin == room && c.dest == room)
            .map(|c| c.dir)
            .collect()
    }

    /// Drop every Unknown-direction edge whose room pair (same origin→dest) already carries a
    /// known-direction edge; the redundant `?` stub goes, the known edge stays. Reverse
    /// (dest→origin) edges do NOT count — a return trip is not guaranteed to be the geometric
    /// opposite (one-way passages, mazes), so no forward direction is ever inferred from it, and
    /// nothing is relabeled: the replacing direction already exists as its own edge. Unknown edges
    /// with no same-direction known counterpart are left untouched. Returns the number removed.
    /// (SQ-0220)
    pub fn collapse_unknown_edges(&mut self) -> usize {
        let known: std::collections::HashSet<(RoomId, RoomId)> = self
            .conns
            .iter()
            .filter(|c| c.dir != Direction::Unknown)
            .map(|c| (c.origin, c.dest))
            .collect();
        let before = self.conns.len();
        self.conns
            .retain(|c| c.dir != Direction::Unknown || !known.contains(&(c.origin, c.dest)));
        before - self.conns.len()
    }

    /// Give room `old` the id `new`, rewriting every reference to it (SQ-0526).
    ///
    /// The Glulx side identifies a room by hashing its printed NAME until it has
    /// worked out where the game keeps its `location` global, then switches to the
    /// room's real object address. The handful of rooms mapped during that
    /// learning window carry name-derived ids, and would otherwise reappear as
    /// duplicate nodes the moment the player walked back into them. Re-keying them
    /// keeps one node per room across the switch.
    ///
    /// Returns `false` and changes nothing when `old` is unknown, when the ids are
    /// equal, or when `new` is ALREADY a room — that last case is a merge, not a
    /// rename, and silently folding two mapped rooms together could destroy real
    /// structure. The caller is left with the duplicate rather than a guess.
    pub fn rekey_room(&mut self, old: RoomId, new: RoomId) -> bool {
        if old == new || !self.rooms.contains_key(&old) || self.rooms.contains_key(&new) {
            return false;
        }
        let Some(mut room) = self.rooms.remove(&old) else { return false };
        room.id = new;
        self.rooms.insert(new, room);
        for c in &mut self.conns {
            if c.origin == old {
                c.origin = new;
            }
            if c.dest == old {
                c.dest = new;
            }
        }
        // A rename can make two edges identical (both ends re-keyed onto the same
        // pair); keep the first of each.
        let mut seen: Vec<(RoomId, Direction, RoomId)> = Vec::new();
        self.conns.retain(|c| {
            let key = (c.origin, c.dir, c.dest);
            let fresh = !seen.contains(&key);
            if fresh {
                seen.push(key);
            }
            fresh
        });
        if self.current == Some(old) {
            self.current = Some(new);
        }
        // A seam answer names a room too, and a decision that quietly stopped applying because the
        // room was re-keyed would bring a dismissed prompt back from the dead (SQ-0439).
        self.seam_decisions = std::mem::take(&mut self.seam_decisions)
            .into_iter()
            .map(|(mut k, v)| {
                if k.from == old {
                    k.from = new;
                }
                (k, v)
            })
            .collect();
        true
    }

    /// Record how this room was detected, the FIRST time it is discovered
    /// (SQ-0527). Later visits leave it alone: the interesting fact is how the
    /// mapper first came to know the room, and a later visit may well be resolved
    /// by a weaker method (a name match rather than the object) without that
    /// saying anything new. A no-op for an unknown room.
    /// Record that `dir` was TYPED while standing in `id`, whether or not it moved the player
    /// (SQ-0391). Idempotent — a direction you try twice is still one tried direction.
    pub fn mark_tried(&mut self, id: RoomId, dir: Direction) {
        if let Some(r) = self.rooms.get_mut(&id) {
            if !r.tried.contains(&dir) {
                r.tried.push(dir);
            }
        }
    }

    /// Undo a [`MapGraph::mark_tried`] — `dir` was never really tried from `id` after all
    /// (SQ-0671). The one caller is the fatal move: the player typed a direction, the game killed
    /// them for it, and no passage was found either way. Recording it as tried would draw a `×`
    /// ("tried, and there is no path that way") over a direction nobody has any knowledge about,
    /// which is the map asserting something false.
    ///
    /// Only the TYPED record is dropped. A direction that also carries an edge out of `id` stays
    /// tried by [`MapGraph::is_tried`], because the edge is the stronger evidence and this cannot
    /// (and must not) unmint it.
    pub fn unmark_tried(&mut self, id: RoomId, dir: Direction) {
        if let Some(r) = self.rooms.get_mut(&id) {
            r.tried.retain(|d| *d != dir);
        }
    }

    /// The compass directions never typed in this room — what a player has left to explore
    /// (SQ-0391). Directions that LED somewhere are tried by definition, so an edge out counts
    /// even on a map loaded from before this was recorded.
    /// True when `dir` has been explored from `id`: typed there (worked or not), or carrying an
    /// edge that leads somewhere — the only signal a map saved before `tried` existed still has.
    pub fn is_tried(&self, id: RoomId, dir: Direction) -> bool {
        let typed = self.rooms.get(&id).is_some_and(|r| r.tried.contains(&dir));
        typed || self.conns.iter().any(|c| c.origin == id && c.dir == dir)
    }

    pub fn untried(&self, id: RoomId) -> Vec<Direction> {
        if !self.rooms.contains_key(&id) {
            return Vec::new();
        }
        crate::direction::UNTRIED_DIRS.iter().copied().filter(|d| !self.is_tried(id, *d)).collect()
    }

    /// Record that a return probe has tried `dir` out of `id` (SQ-0785). Idempotent.
    ///
    /// **Not [`MapGraph::mark_tried`], and never a substitute for it.** That is the PLAYER's
    /// record and it drives [`MapGraph::untried`] — the exits the map still offers. A shadow
    /// walking a direction on the player's behalf has not explored it, and marking it tried would
    /// take a real unexplored exit off the map and quietly steer the player away from content.
    ///
    /// Called once per attempt, as the search makes it, so an aborted search resumes rather than
    /// restarts. A no-op for an unknown room.
    pub fn mark_probed(&mut self, id: RoomId, dir: Direction) {
        if let Some(r) = self.rooms.get_mut(&id) {
            if !r.probed.contains(&dir) {
                r.probed.push(dir);
            }
        }
    }

    /// True when a return probe has already tried `dir` out of `id` (SQ-0785).
    ///
    /// Unlike [`MapGraph::is_tried`] this reads the record and nothing else: an edge out is
    /// evidence about the WORLD, and this field is a record of what the SEARCH has done.
    /// [`MapGraph::probe_candidates`] consults both, which is the only place they meet.
    pub fn is_probed(&self, id: RoomId, dir: Direction) -> bool {
        self.rooms.get(&id).is_some_and(|r| r.probed.contains(&dir))
    }

    /// Record that `dir` out of `id` is a RANDOM exit (SQ-1257): the room's own map data named a
    /// fixed destination and the player was sent somewhere else. Also marks `dir` tried — the
    /// player DID try it, just not to a destination the map can name — so it never shows as an
    /// unexplored frontier. A no-op for an unknown room or [`Direction::Unknown`].
    pub fn mark_random_exit(&mut self, id: RoomId, dir: Direction) {
        if dir == Direction::Unknown {
            return;
        }
        self.mark_tried(id, dir);
        if let Some(r) = self.rooms.get_mut(&id) {
            if !r.random_exits.contains(&dir) {
                r.random_exits.push(dir);
            }
        }
    }

    /// True when `dir` out of `id` is recorded as a random exit (SQ-1257). Read by
    /// [`crate::matrix::classify`] — beaten by a real edge in the same direction, since a later
    /// direction that behaves deterministically is the stronger fact.
    pub fn is_random_exit(&self, id: RoomId, dir: Direction) -> bool {
        self.rooms.get(&id).is_some_and(|r| r.random_exits.contains(&dir))
    }

    /// Undo a [`MapGraph::mark_random_exit`] — `dir` out of `id` turned out to behave
    /// deterministically after all (SQ-1257 Phase 2: a reseeded re-probe of a random-marked
    /// direction agreed with the live game on every attempt). Called by
    /// `random_exit_probe::deliver` in the same stroke it mints the now-confirmed edge; a no-op
    /// if the direction was never marked. Does NOT touch `tried` — the direction was and remains
    /// tried, whichever way this resolves.
    pub fn unmark_random_exit(&mut self, id: RoomId, dir: Direction) {
        if let Some(r) = self.rooms.get_mut(&id) {
            r.random_exits.retain(|&d| d != dir);
            // The destinations recorded against this direction were evidence for a fact that no
            // longer holds — the direction is confirmed deterministic now, and re-marking it
            // later (SQ-1257 Phase 2's upgrade can be undone by a subsequent disagreement) starts
            // the list over rather than resuming a stale one from before the confirmation.
            r.random_destinations.retain(|(d, _)| *d != dir);
        }
    }

    /// Record that `dir` out of `id` — already marked random — has been seen to land in `dest`
    /// (SQ-1261). First-seen order, no duplicates; a no-op for [`Direction::Unknown`] or an
    /// unknown room. Deliberately does NOT require `dir` to already be marked random: the note
    /// and the mark are two different facts, and callers can order the mark first without this
    /// silently depending on it.
    pub fn note_random_destination(&mut self, id: RoomId, dir: Direction, dest: RoomId) {
        if dir == Direction::Unknown {
            return;
        }
        let Some(r) = self.rooms.get_mut(&id) else { return };
        match r.random_destinations.iter_mut().find(|(d, _)| *d == dir) {
            Some((_, dests)) => {
                if !dests.contains(&dest) {
                    dests.push(dest);
                }
            }
            None => r.random_destinations.push((dir, vec![dest])),
        }
    }

    /// Every distinct room `dir` out of `id` has been seen to land in, first-seen order — empty
    /// when the direction is not marked random, or is but nothing has landed anywhere recorded
    /// yet (SQ-1261). See [`Room::random_destinations`].
    pub fn random_destinations(&self, id: RoomId, dir: Direction) -> &[RoomId] {
        self.rooms
            .get(&id)
            .and_then(|r| r.random_destinations.iter().find(|(d, _)| *d == dir))
            .map(|(_, dests)| dests.as_slice())
            .unwrap_or(&[])
    }

    /// Which directions are worth probing out of `room`, best first (SQ-0785).
    ///
    /// **The one place the two records meet.** A caller that assembled this from `tried` and
    /// `probed` itself would be maintaining the rule about which record means what across files,
    /// which is exactly the shape this repo's refactoring policy names — and the rule is subtle
    /// enough (the player's record must not be written by a probe; the probe's record must not
    /// hide an unexplored exit) that a second copy of it is a defect waiting to happen. So the
    /// filtering AND the order live here, and callers walk the list.
    ///
    /// `moved` is the direction the player took to GET here, when their command named one. The
    /// order is seeded from its opposite, because the way back is overwhelmingly the way you came
    /// — and then widens rather than stopping there, since these games are full of passages that
    /// do not reciprocate:
    ///
    /// 1. `opposite(moved)`
    /// 2. the two directions perpendicular to it (±90°) — only when step 1 has a bearing
    /// 3. the two diagonals adjacent to it (±45°) — only when step 1 has a bearing
    /// 4. everything else that survives the filter: the eight compass points, and NOTHING else
    ///
    /// With no direction to seed from (`climb tree`), there is no opposite, so the order is simply
    /// the eight compass points, cardinals then diagonals ([`crate::direction::PROBE_FALLBACK_DIRS`]).
    /// Steps 2 and 3 are defined by BEARING rather than by a table, so they mean the same thing for
    /// a diagonal opposite as for a cardinal one.
    ///
    /// **Up/Down/In/Out are asked ONLY as `opposite(moved)` when `moved` was itself one of them
    /// (SQ-1290)** — climb down and the seed is Up; walk in and the seed is Out — never as a
    /// fallback once the compass words run out. Reaching a portal in step 4 would mean revealing
    /// an unexplored exit the player has not walked: on an ordinary compass map the only way back
    /// from some room may genuinely be `up`, and finding that BEFORE the player has ever gone up
    /// is not this search's business. [`crate::direction::PROBE_FALLBACK_DIRS`], the list step 4
    /// draws from, carries only the eight compass points for exactly this reason; a portal never
    /// reaches step 4 no matter what `moved` was, because it is not IN that list to reach. The
    /// full [`crate::direction::PROBE_DIRS`] (all twelve) is unaffected — this fallback step is
    /// the only caller narrowed.
    ///
    /// Starting at all eight compass points (nine when `moved` seeded a portal reciprocal) is
    /// deliberate — narrowing further is a measurement decision, not a guess.
    pub fn probe_candidates(&self, room: RoomId, moved: Option<Direction>) -> Vec<Direction> {
        if !self.rooms.contains_key(&room) {
            return Vec::new();
        }
        // Up to eight compass points, plus one portal reciprocal when `moved` seeded one.
        let mut order: Vec<Direction> = Vec::with_capacity(crate::direction::PROBE_FALLBACK_DIRS.len() + 1);
        let push = |order: &mut Vec<Direction>, d: Direction| {
            if !order.contains(&d) {
                order.push(d);
            }
        };
        if let Some(back) = moved.map(crate::direction::opposite).filter(|d| *d != Direction::Unknown)
        {
            push(&mut order, back);
            if let Some(deg) = crate::direction::bearing(back) {
                // `[+270, +90, +315, +45]`, so the pairs come out in the order the quest names
                // them for a southward opposite: south, EAST, WEST, then SOUTH-EAST, SOUTH-WEST.
                // Within a pair the choice is arbitrary but it must be FIXED, or a resumed search
                // walks a different order than the one that was interrupted.
                for turn in [270, 90, 315, 45] {
                    if let Some(d) = crate::direction::from_bearing((deg + turn) % 360) {
                        push(&mut order, d);
                    }
                }
            }
        }
        for d in crate::direction::PROBE_FALLBACK_DIRS {
            push(&mut order, d);
        }
        order.into_iter().filter(|d| !self.is_tried(room, *d) && !self.is_probed(room, *d)).collect()
    }

    /// Test-only: drop a room's recorded attempts, to stand in for a map file written before
    /// they were recorded.
    #[doc(hidden)]
    pub fn room_mut_tried_clear_for_test(&mut self, id: RoomId) {
        if let Some(r) = self.rooms.get_mut(&id) {
            r.tried.clear();
        }
    }

    pub fn set_loc_method(&mut self, id: RoomId, method: &str) {
        if let Some(r) = self.rooms.get_mut(&id) {
            if r.loc_method.is_none() {
                r.loc_method = Some(method.to_string());
            }
        }
    }

    pub fn set_current(&mut self, id: RoomId) {
        self.current = Some(id);
        // Record this as the last room visited on whichever layer it is CURRENTLY on (SQ-0672).
        // Every path that moves the player funnels through here (`Mapper::observe*`), so this is
        // the single choke point for the memory a layer switch recenters against.
        if let Some(layer) = self.rooms.get(&id).map(|r| r.layer) {
            self.last_visited.insert(layer, id);
        }
    }

    /// The last room the player stood in on `layer`, if the mapper has ever recorded a visit
    /// there (SQ-0672). May name a room that has since been peeled/merged to a different layer —
    /// callers wanting a room CURRENTLY on `layer` must re-check `layer_of`.
    pub fn last_visited(&self, layer: LayerId) -> Option<RoomId> {
        self.last_visited.get(&layer).copied()
    }

    /// The full per-layer last-visited map, for persistence.
    pub fn last_visited_map(&self) -> &BTreeMap<LayerId, RoomId> {
        &self.last_visited
    }

    pub fn room_mut_notes(&mut self, id: RoomId, notes: &str) {
        if let Some(room) = self.rooms.get_mut(&id) {
            room.notes = notes.into();
        }
    }

    /// Set the grid position of a room. Used by the layout engine.
    pub fn set_pos(&mut self, id: RoomId, pos: (i32, i32)) {
        if let Some(room) = self.rooms.get_mut(&id) {
            room.pos = Some(pos);
        }
    }

    /// Clear the grid position of a room (set to None). Used by the layout engine
    /// to reset positions before a full re-derivation.
    pub fn clear_pos(&mut self, id: RoomId) {
        if let Some(room) = self.rooms.get_mut(&id) {
            room.pos = None;
        }
    }

    /// Mark a connection as distorted by index. Used by the layout engine when a room
    /// cannot be placed at its preferred compass offset (collision).
    pub fn set_conn_distorted(&mut self, idx: usize, distorted: bool) {
        if let Some(conn) = self.conns.get_mut(idx) {
            conn.distorted = distorted;
        }
    }

    /// Set or clear the label_override for a room.
    pub fn set_label_override(&mut self, id: RoomId, label: Option<String>) {
        if let Some(room) = self.rooms.get_mut(&id) {
            room.label_override = label;
        }
    }

    /// Set the notes for a room.
    pub fn set_notes(&mut self, id: RoomId, notes: String) {
        if let Some(room) = self.rooms.get_mut(&id) {
            room.notes = notes;
        }
    }

    /// Remove the connection(s) with key (origin, dir). Returns true if any was removed.
    /// For a real direction that is at most one edge; for `Unknown` — which may hold several
    /// stubs (see [`MapGraph::add_edge`]) — every `?` edge out of `origin` goes at once, as the
    /// key names no destination to tell them apart by.
    pub fn remove_connection(&mut self, origin: RoomId, dir: Direction) -> bool {
        let before = self.conns.len();
        self.conns.retain(|c| !(c.origin == origin && c.dir == dir));
        self.conns.len() < before
    }

    /// A sub-graph containing only `layer`'s rooms and the connections whose BOTH
    /// endpoints are in `layer`. Positions are preserved; `current` carries over only
    /// if the current room is in `layer`. Layer metadata is not copied (not needed for routing).
    pub fn layer_subgraph(&self, layer: LayerId) -> MapGraph {
        let in_layer: std::collections::BTreeSet<RoomId> =
            self.rooms.values().filter(|r| r.layer == layer).map(|r| r.id).collect();
        let rooms: BTreeMap<RoomId, Room> = self
            .rooms
            .values()
            .filter(|r| in_layer.contains(&r.id))
            .map(|r| (r.id, r.clone()))
            .collect();
        let conns: Vec<Connection> = self
            .conns
            .iter()
            .filter(|c| in_layer.contains(&c.origin) && in_layer.contains(&c.dest))
            .cloned()
            .collect();
        let current = self.current.filter(|id| in_layer.contains(id));
        let mut layers = BTreeMap::new();
        layers.insert(MAIN_LAYER, LayerMeta::main());
        // `next_seq` carries over from the parent rather than restarting at 0: the subgraph's
        // rooms keep the real seqs they were cloned with, and a caller that mints a NEW room on
        // the subgraph (layout scratch graphs do) must not hand out a seq that collides with one
        // already in play back on the parent.
        MapGraph {
            rooms,
            conns,
            current,
            layers,
            next_layer_id: 1,
            next_seq: self.next_seq,
            last_visited: BTreeMap::new(),
            // A routing scratch graph never prompts, so it carries no prompt answers either.
            seam_decisions: BTreeMap::new(),
            suggestions_disabled: false,
        }
    }

    /// Change the direction of the edge keyed (origin, old) to (origin, new).
    /// If an edge with key (origin, new) already exists, refuses and returns false.
    /// Returns true if the relabel happened.
    pub fn relabel_connection(&mut self, origin: RoomId, old: Direction, new: Direction) -> bool {
        // Refuse if a connection with (origin, new) already exists.
        if self.conns.iter().any(|c| c.origin == origin && c.dir == new) {
            return false;
        }
        if let Some(conn) = self.conns.iter_mut().find(|c| c.origin == origin && c.dir == old) {
            conn.dir = new;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    /// SQ-0526: re-keying a room must move every reference with it, and must
    /// refuse the cases where it would destroy structure.
    #[test]
    fn rekey_room_moves_edges_and_current_and_refuses_a_merge() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Cave".into());
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1);
        g.set_current(1);

        assert!(g.rekey_room(1, 99), "a plain rename succeeds");
        assert!(g.room(1).is_none(), "the old id is gone");
        assert_eq!(g.room(99).map(|r| r.name.as_str()), Some("Hall"), "the room came with it");
        assert_eq!(g.room(99).map(|r| r.id), Some(99), "and its own id field was updated");
        assert_eq!(g.current(), Some(99), "the current pointer followed");
        assert!(
            g.connections().iter().any(|c| c.origin == 99 && c.dest == 2),
            "the outgoing edge followed: {:?}",
            g.connections()
        );
        assert!(
            g.connections().iter().any(|c| c.origin == 2 && c.dest == 99),
            "and the incoming edge: {:?}",
            g.connections()
        );

        assert!(!g.rekey_room(99, 99), "renaming to itself is a no-op");
        assert!(!g.rekey_room(1234, 5678), "an unknown room is a no-op");
        assert!(
            !g.rekey_room(99, 2),
            "re-keying ONTO an existing room would be a merge, not a rename, and must be refused"
        );
        assert_eq!(g.room(2).map(|r| r.name.as_str()), Some("Cave"), "the refused merge changed nothing");
    }

    /// SQ-1300: a room's ordinal (its display number, `seq + 1`) is a property of the room NODE,
    /// so a re-key — the Glulx lock landing on a name-derived id and swapping it for the room's
    /// real object address — must carry it across unchanged, exactly like the name and the edges
    /// already do. A room re-keyed a third of the way into a session must keep reading "1", "2",
    /// "3" … in true discovery order rather than picking up a fresh number at its new id.
    #[test]
    fn rekey_room_carries_the_ordinal_with_it() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Cave".into());
        g.upsert_room(3, "Loft".into());
        assert_eq!(g.room(1).unwrap().ordinal(), 1, "first room discovered");
        assert_eq!(g.room(2).unwrap().ordinal(), 2);
        assert_eq!(g.room(3).unwrap().ordinal(), 3);

        assert!(g.rekey_room(2, 0x8000_5678), "re-key the middle room onto a far-away new id");
        assert_eq!(
            g.room(0x8000_5678).unwrap().ordinal(),
            2,
            "the ordinal moved with the room, not with the numeric id"
        );
        assert_eq!(g.room(1).unwrap().ordinal(), 1, "untouched rooms keep their own ordinals");
        assert_eq!(g.room(3).unwrap().ordinal(), 3);

        // A room discovered AFTER the re-key still gets the next ordinal in true order.
        g.upsert_room(4, "Attic".into());
        assert_eq!(g.room(4).unwrap().ordinal(), 4, "next_seq was not disturbed by the re-key");
    }

    use super::*;
    use crate::direction::Direction;

    #[test]
    fn rooms_default_to_main_layer_and_can_move() {
        use crate::layer::MAIN_LAYER;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        assert_eq!(g.layer_of(1), MAIN_LAYER);
        assert_eq!(g.layer_name(MAIN_LAYER), "Main");
        let l = g.new_layer(Some(MAIN_LAYER), "Basement".into());
        g.set_room_layer(2, l);
        assert_eq!(g.layer_of(2), l);
        assert_eq!(g.rooms_in_layer(MAIN_LAYER), vec![1]);
        assert_eq!(g.rooms_in_layer(l), vec![2]);
        assert_eq!(g.layer_name(l), "Basement");
    }

    /// SQ-0666: the maze flag only ever changes the DEFAULT view. A view the player picked by
    /// hand must survive being flagged and unflagged, or `/mark-maze-layer` would silently undo
    /// `/view-map`.
    #[test]
    fn the_maze_flag_moves_the_default_view_but_never_overrides_a_chosen_one() {
        use crate::layer::MapView;
        let mut g = MapGraph::new();
        let l = g.new_layer(None, "Maze".into());
        assert!(!g.layer_is_maze(l));
        assert_eq!(g.layer_view(l), MapView::Drawn, "an ordinary layer draws");
        assert_eq!(g.layer_view_choice(l), None, "and has made no choice");

        assert!(g.set_layer_maze(l, true));
        assert_eq!(g.layer_view(l), MapView::Matrix, "flagging a maze defaults it to the matrix");
        assert_eq!(g.layer_view_choice(l), None, "…without recording a choice the player never made");

        g.set_layer_view(l, Some(MapView::Drawn));
        assert_eq!(g.layer_view(l), MapView::Drawn, "an explicit choice beats the maze default");
        assert!(!g.set_layer_maze(l, false));
        assert_eq!(g.layer_view(l), MapView::Drawn, "and survives the flag going away again");

        g.set_layer_view(l, None);
        assert_eq!(g.layer_view(l), MapView::Drawn, "clearing the choice falls back to the default");

        // An unknown layer answers without panicking.
        assert!(!g.layer_is_maze(999));
        assert_eq!(g.layer_view(999), MapView::Drawn);
    }

    /// SQ-0666: "west leads back here" is a fact, and recording it must not cost the map a
    /// passage it already knew about on the same key.
    #[test]
    fn a_self_loop_is_recorded_beside_a_real_passage_not_instead_of_it() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Maze".into());
        g.upsert_room(2, "Maze".into());
        g.add_edge(1, Direction::W, 2);

        assert!(g.add_self_loop(1, Direction::W), "an observed loop on a used key is recordable");
        assert!(
            g.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::W && c.dest == 2),
            "the real passage 1-W->2 is still there: {:?}",
            g.connections()
        );
        assert_eq!(g.self_loops(1), vec![Direction::W], "and the loop sits beside it");
        assert!(g.is_tried(1, Direction::W), "recording a loop marks the direction tried");

        g.add_self_loop(1, Direction::W); // an exact repeat
        assert_eq!(g.connections().len(), 2, "a repeated observation is still one loop");

        // …and the reverse: a later real destination on that key must not eat the loop.
        g.add_edge(1, Direction::W, 2);
        assert_eq!(g.self_loops(1), vec![Direction::W], "the loop survives a re-observed passage");

        assert!(!g.add_self_loop(1, Direction::Unknown), "`?` is a bucket, not a passage");
        assert!(!g.add_self_loop(404, Direction::N), "an unknown room records nothing");
    }

    /// SQ-0671: a fatal move's `tried` record is taken back — but only the typed one. An edge is
    /// evidence this cannot unmint, and must go on answering for the direction.
    #[test]
    fn unmarking_a_tried_direction_drops_the_typed_record_but_never_a_passage() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Cellar".into());
        g.upsert_room(2, "Forest".into());
        g.mark_tried(1, Direction::N);
        g.mark_tried(1, Direction::E);
        assert!(g.is_tried(1, Direction::N));

        g.unmark_tried(1, Direction::N);
        assert!(!g.is_tried(1, Direction::N), "the typed record is gone");
        assert!(g.untried(1).contains(&Direction::N), "so the direction is a frontier again");
        assert!(g.is_tried(1, Direction::E), "and the room's other attempts are untouched");

        // A direction carrying a passage stays tried whatever the record says.
        g.add_edge(1, Direction::W, 2);
        g.mark_tried(1, Direction::W);
        g.unmark_tried(1, Direction::W);
        assert!(g.is_tried(1, Direction::W), "the edge still proves west was walked");

        g.unmark_tried(404, Direction::N); // an unknown room does not panic
        g.unmark_tried(1, Direction::S); // nor does a direction that was never tried
    }

    #[test]
    fn new_layer_ids_are_unique_and_main_cannot_be_removed() {
        let mut g = MapGraph::new();
        let a = g.new_layer(None, "A".into());
        let b = g.new_layer(None, "B".into());
        assert_ne!(a, b);
        g.remove_layer(crate::layer::MAIN_LAYER); // no-op
        assert_eq!(g.layer_name(crate::layer::MAIN_LAYER), "Main");
    }

    #[test]
    fn distinct_ids_same_name_are_distinct_rooms() {
        let mut g = MapGraph::new();
        g.upsert_room(10, "Forest".into());
        g.upsert_room(11, "Forest".into());
        assert_eq!(g.rooms().count(), 2);
        assert_eq!(g.room(10).unwrap().label(), "Forest");
    }

    #[test]
    fn revisit_same_id_updates_not_duplicates() {
        let mut g = MapGraph::new();
        g.upsert_room(10, "Dark Room".into());
        g.room_mut_notes(10, "has lamp"); // helper or set notes directly in test
        g.upsert_room(10, "Lit Room".into()); // name changed (light came on)
        assert_eq!(g.rooms().count(), 1);
        assert_eq!(g.room(10).unwrap().name, "Lit Room");
        assert_eq!(g.room(10).unwrap().notes, "has lamp"); // edits preserved
    }

    // ── SQ-1257 Phase 3: aliases ─────────────────────────────────────────────

    /// A room renamed several times over collects every OLD name as an alias, in the order it
    /// was first seen, and the current label is never one of them (Lost Pig's gnome tunnels:
    /// "Twisty Cave" → "Confusing Passage" → "Strange Place" → "Twisty Place").
    #[test]
    fn a_repeatedly_renamed_room_collects_its_old_names_as_aliases_in_first_seen_order() {
        let mut g = MapGraph::new();
        g.upsert_room(183, "Twisty Cave".into());
        assert!(g.room(183).unwrap().aliases.is_empty(), "nothing to alias yet on first sight");

        g.upsert_room(183, "Confusing Passage".into());
        assert_eq!(g.room(183).unwrap().name, "Confusing Passage");
        assert_eq!(g.room(183).unwrap().aliases, vec!["Twisty Cave"]);

        g.upsert_room(183, "Strange Place".into());
        assert_eq!(
            g.room(183).unwrap().aliases,
            vec!["Twisty Cave", "Confusing Passage"],
            "first-seen order, oldest first"
        );

        g.upsert_room(183, "Twisty Place".into());
        assert_eq!(
            g.room(183).unwrap().aliases,
            vec!["Twisty Cave", "Confusing Passage", "Strange Place"]
        );
        assert!(
            !g.room(183).unwrap().aliases.contains(&"Twisty Place".to_string()),
            "the current label is never also an alias"
        );

        // A same-name re-observation (no rename) must not touch the list at all.
        g.upsert_room(183, "Twisty Place".into());
        assert_eq!(g.room(183).unwrap().aliases.len(), 3, "no change on a repeat observation");
    }

    /// A rename back to a PREVIOUSLY-seen name pulls that name back out of the alias list (it is
    /// current again) and files the label it just left in its place — no duplicates either way.
    #[test]
    fn renaming_back_to_a_former_name_moves_it_out_of_aliases_and_the_displaced_one_in() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(1, "B".into()); // aliases: [A]
        assert_eq!(g.room(1).unwrap().aliases, vec!["A"]);

        g.upsert_room(1, "A".into()); // back to A: aliases lose A, gain B
        assert_eq!(g.room(1).unwrap().name, "A");
        assert_eq!(g.room(1).unwrap().aliases, vec!["B"], "B is now the alias, not A");
    }

    /// A `label_override` pins the map's display regardless of what the story prints
    /// underneath it, so a name change while one is set changes the room's raw `name` but must
    /// not perturb the aliases the player actually SEES (the override itself is never displaced
    /// into `aliases`, since it is still the current label after the rename).
    #[test]
    fn a_label_override_is_never_displaced_into_aliases_by_a_name_change_underneath_it() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Twisty Cave".into());
        g.set_label_override(1, Some("My Landmark".into()));
        assert_eq!(g.room(1).unwrap().label(), "My Landmark");

        g.upsert_room(1, "Confusing Passage".into()); // the story reroll happens underneath
        assert_eq!(g.room(1).unwrap().label(), "My Landmark", "the override still wins");
        assert_eq!(
            g.room(1).unwrap().aliases,
            vec!["Twisty Cave"],
            "the raw name the room had before the rename is recorded, not the override"
        );
    }

    // ── SQ-1261: random-exit destinations ───────────────────────────────────

    /// Landing in a new room notes it; landing there again does not duplicate it; a different
    /// room joins the list after it, in the order each was first seen.
    #[test]
    fn note_random_destination_dedupes_and_keeps_first_seen_order() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Tunnel".into());
        g.mark_random_exit(1, Direction::N);
        assert!(g.random_destinations(1, Direction::N).is_empty(), "nothing recorded yet");

        g.note_random_destination(1, Direction::N, 2);
        assert_eq!(g.random_destinations(1, Direction::N), &[2]);

        g.note_random_destination(1, Direction::N, 2); // same room again
        assert_eq!(g.random_destinations(1, Direction::N), &[2], "no duplicate");

        g.note_random_destination(1, Direction::N, 3);
        assert_eq!(g.random_destinations(1, Direction::N), &[2, 3], "first-seen order");

        // A different direction out of the same room keeps its own list.
        assert!(g.random_destinations(1, Direction::S).is_empty());
    }

    /// [`Direction::Unknown`] and an unknown room are both no-ops, matching every other
    /// random-exit mutator's guard.
    #[test]
    fn note_random_destination_is_a_no_op_for_unknown_direction_or_room() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Tunnel".into());
        g.note_random_destination(1, Direction::Unknown, 2);
        assert!(g.random_destinations(1, Direction::Unknown).is_empty());
        g.note_random_destination(404, Direction::N, 2);
        assert!(g.random_destinations(404, Direction::N).is_empty());
    }

    /// Undoing a random mark (SQ-1257 Phase 2's upgrade path) clears the destinations recorded
    /// against it too — they were evidence for a fact that no longer holds, and a later re-mark
    /// of the same direction must not resume a stale list from before the confirmation.
    #[test]
    fn unmark_random_exit_clears_its_recorded_destinations() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Tunnel".into());
        g.mark_random_exit(1, Direction::N);
        g.note_random_destination(1, Direction::N, 2);
        g.note_random_destination(1, Direction::N, 3);
        assert_eq!(g.random_destinations(1, Direction::N).len(), 2);

        g.unmark_random_exit(1, Direction::N);
        assert!(g.random_destinations(1, Direction::N).is_empty(), "cleared along with the mark");

        // Re-marking starts the list over, not from where it left off.
        g.mark_random_exit(1, Direction::N);
        g.note_random_destination(1, Direction::N, 4);
        assert_eq!(g.random_destinations(1, Direction::N), &[4]);
    }

    /// SQ-0632: a room can hold several non-compass passages (xyzzy AND pray). Keying Unknown
    /// edges by (origin, dir) alone made the second silently overwrite the first's destination,
    /// losing a recorded passage. Unknown edges to DIFFERENT destinations must coexist; an exact
    /// repeat is still one edge.
    #[test]
    fn a_room_keeps_multiple_unknown_passages() {
        let mut g = MapGraph::new();
        for (id, n) in [(1, "Cave"), (2, "Grotto"), (3, "Chapel")] {
            g.upsert_room(id, n.into());
        }
        g.add_edge(1, Direction::Unknown, 2); // xyzzy
        g.add_edge(1, Direction::Unknown, 3); // pray, from the same room
        assert!(
            g.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::Unknown && c.dest == 2),
            "the xyzzy passage survives the second Unknown edge: {:?}",
            g.connections()
        );
        assert!(
            g.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::Unknown && c.dest == 3),
            "and the pray passage was recorded beside it"
        );
        g.add_edge(1, Direction::Unknown, 2); // repeat observation of the same passage
        assert_eq!(g.connections().len(), 2, "an exact repeat is still one edge");

        // A directional edge over one of the pairs collapses only THAT pair's stub.
        g.add_edge(1, Direction::N, 2);
        assert_eq!(g.collapse_unknown_edges(), 1);
        assert!(
            g.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::Unknown && c.dest == 3),
            "the other room's passage is untouched"
        );
    }

    #[test]
    fn directed_edge_no_symmetry_and_dedup() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::W, 1); // non-reciprocal back-edge
        g.add_edge(1, Direction::N, 2); // duplicate key → still one
        assert_eq!(g.connections().len(), 2);
    }

    #[test]
    fn collapse_unknown_drops_redundant_same_pair_edge() {
        // A→B carries both an Unknown edge and a known N edge (same origin→dest). The Unknown
        // is redundant and is dropped; the known edge stays. (SQ-0220)
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::Unknown, 2);
        g.add_edge(1, Direction::N, 2);
        let removed = g.collapse_unknown_edges();
        assert_eq!(removed, 1, "the redundant Unknown A→B is removed");
        assert_eq!(g.connections().len(), 1);
        assert_eq!(g.connections()[0].dir, Direction::N, "the known edge survives");
    }

    #[test]
    fn collapse_unknown_keeps_reverse_only_and_lone_unknowns() {
        // A→B Unknown must survive when only the REVERSE B→A is directional (return trips are
        // not guaranteed to be the geometric opposite), and when it has no known counterpart.
        let mut g = MapGraph::new();
        for id in [1u16, 2, 3, 4] { g.upsert_room(id.into(), "r".into()); }
        g.add_edge(1, Direction::Unknown, 2); // reverse-only pair
        g.add_edge(2, Direction::S, 1); // return trip is directional, forward was Unknown
        g.add_edge(3, Direction::Unknown, 4); // lone Unknown, no known counterpart
        let removed = g.collapse_unknown_edges();
        assert_eq!(removed, 0, "neither Unknown has a same-origin→dest known edge");
        assert_eq!(g.connections().len(), 3);
    }

    #[test]
    fn collapse_unknown_ignores_known_edge_to_a_different_dest() {
        // A→B Unknown is not affected by a known A→C edge (same origin, different dest).
        let mut g = MapGraph::new();
        for id in [1u16, 2, 3] { g.upsert_room(id.into(), "r".into()); }
        g.add_edge(1, Direction::Unknown, 2);
        g.add_edge(1, Direction::N, 3);
        let removed = g.collapse_unknown_edges();
        assert_eq!(removed, 0);
        assert_eq!(g.connections().len(), 2);
    }

    // ── SQ-0685: discovery sequence ──────────────────────────────────────────

    /// `upsert_room` stamps `seq` only on first discovery. A revisit (the Occupied-entry branch,
    /// used to update a room's name when a light comes on etc.) must never re-mint it — that would
    /// silently move a room's place in the discovery order every time its name changed.
    #[test]
    fn upsert_room_stamps_seq_once_and_a_revisit_never_rewrites_it() {
        let mut g = MapGraph::new();
        g.upsert_room(10, "Dark Room".into());
        g.upsert_room(20, "Forest".into());
        assert_eq!(g.room(10).unwrap().seq, 0, "the first room minted gets seq 0");
        assert_eq!(g.room(20).unwrap().seq, 1);
        assert_eq!(g.next_seq(), 2);

        g.upsert_room(10, "Lit Room".into()); // revisit: name changes, identity does not
        assert_eq!(g.room(10).unwrap().seq, 0, "a revisit must not re-stamp the ordinal");
        assert_eq!(g.next_seq(), 2, "and must not consume a fresh one either");
    }

    /// The back-compat mechanism itself (SQ-0685): a save written before `seq` existed has every
    /// room at the wire sentinel [`ROOM_SEQ_MISSING`] (what `Room::seq` deserializes to when the
    /// field is absent — see `room_seq_missing`). `from_parts` must backfill from each room's
    /// POSITION IN THE ARRAY, not from its id — the array is deliberately built here so the two
    /// orders disagree (ids 5, 2, 9; array position 0, 1, 2), which is exactly the shape a save
    /// where a lower-id room was discovered LATER produces.
    #[test]
    fn from_parts_backfills_missing_seq_from_array_position_not_room_id() {
        let mk = |id: RoomId| Room {
            id,
            name: "Maze".into(),
            label_override: None,
            notes: String::new(),
            pos: None,
            layer: MAIN_LAYER,
            loc_method: None,
            tried: Vec::new(),
            probed: Vec::new(),
            random_exits: Vec::new(),
            random_destinations: Vec::new(),
            aliases: Vec::new(),
            seq: ROOM_SEQ_MISSING,
        };
        let rooms = vec![mk(5), mk(2), mk(9)];
        let g = MapGraph::from_parts(rooms, Vec::new(), None, BTreeMap::new(), 1, BTreeMap::new(), 0);
        assert_eq!(g.room(5).unwrap().seq, 0, "array position 0, despite being the highest id");
        assert_eq!(g.room(2).unwrap().seq, 1, "array position 1, despite being the lowest id");
        assert_eq!(g.room(9).unwrap().seq, 2, "array position 2");
        assert_eq!(g.next_seq(), 3, "resumes past the backfilled max");

        // A room minted after loading must land after everything backfilled, never colliding.
        let mut g = g;
        g.upsert_room(99, "New".into());
        assert_eq!(g.room(99).unwrap().seq, 3);
    }

    /// A save written by a version that HAS `seq` carries real values and a real `next_seq`;
    /// `from_parts` must leave both alone rather than re-deriving them from array position.
    #[test]
    fn from_parts_leaves_real_persisted_seqs_and_next_seq_alone() {
        let room = |id: RoomId, seq: u64| Room {
            id,
            name: "R".into(),
            label_override: None,
            notes: String::new(),
            pos: None,
            layer: MAIN_LAYER,
            loc_method: None,
            tried: Vec::new(),
            probed: Vec::new(),
            random_exits: Vec::new(),
            random_destinations: Vec::new(),
            aliases: Vec::new(),
            seq,
        };
        // Array order (2, 1) deliberately disagrees with seq order (1, 0): if the backfill fired
        // here by mistake it would silently overwrite these with array positions instead.
        let rooms = vec![room(2, 1), room(1, 0)];
        let g = MapGraph::from_parts(rooms, Vec::new(), None, BTreeMap::new(), 1, BTreeMap::new(), 2);
        assert_eq!(g.room(2).unwrap().seq, 1, "the real seq is untouched");
        assert_eq!(g.room(1).unwrap().seq, 0);
        assert_eq!(g.next_seq(), 2, "the persisted next_seq is honoured as-is");
    }
}

/// The return probe's own record, and the one accessor that reads it beside the player's
/// (SQ-0785).
#[cfg(test)]
mod probe_record_tests {
    use super::*;
    use crate::direction::Direction;

    fn two_rooms() -> MapGraph {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Behind House".into());
        g.upsert_room(2, "Kitchen".into());
        g
    }

    /// The whole reason the field exists: a direction the SHADOW walked must not stop the map
    /// offering it to the PLAYER. Falsify by routing `mark_probed` into `mark_tried` and this
    /// fails on the `untried` line — which is the exact defect it guards against.
    #[test]
    fn a_probed_direction_is_still_an_unexplored_exit() {
        let mut g = two_rooms();
        g.mark_probed(2, Direction::N);
        assert!(g.is_probed(2, Direction::N));
        assert!(!g.is_tried(2, Direction::N), "the player's record is untouched");
        assert!(
            g.untried(2).contains(&Direction::N),
            "north is still on the frontier the map offers: {:?}",
            g.untried(2)
        );
        g.mark_probed(2, Direction::N);
        assert_eq!(g.room(2).unwrap().probed.len(), 1, "idempotent");
        g.mark_probed(404, Direction::N); // an unknown room does not panic
    }

    /// An edge out proves the world; the probed list records the search. `is_probed` reads only
    /// the record, so a passage the player walked is not mistaken for ground the search covered.
    #[test]
    fn is_probed_reads_the_record_and_not_the_edges() {
        let mut g = two_rooms();
        g.add_edge(2, Direction::E, 1);
        assert!(g.is_tried(2, Direction::E), "an edge out is tried by definition");
        assert!(!g.is_probed(2, Direction::E), "but the search has not been that way");
    }

    /// The priority order, on the headline shape: the player walked NORTH into room 2, so the
    /// search starts with SOUTH, then the perpendiculars, then the diagonals beside south.
    #[test]
    fn candidates_lead_with_the_way_back_then_widen_by_bearing() {
        let g = two_rooms();
        let c = g.probe_candidates(2, Some(Direction::N));
        // Eight, not twelve (SQ-1290): a compass-seeded search never reaches a portal, because
        // step 4 now draws from `PROBE_FALLBACK_DIRS`, which carries none.
        assert_eq!(c.len(), 8, "the eight compass points, no portal among them: {c:?}");
        assert_eq!(c[0], Direction::S, "the opposite of the move");
        assert_eq!(
            c[..5],
            [Direction::S, Direction::E, Direction::W, Direction::SE, Direction::SW],
            "the way back, its two perpendiculars, then the two diagonals beside it: {c:?}"
        );
        // The bearing arithmetic must mean the same thing for a diagonal opposite.
        let d = g.probe_candidates(2, Some(Direction::NW));
        assert_eq!(
            d[..5],
            [Direction::SE, Direction::NE, Direction::SW, Direction::E, Direction::S],
            "±90° then ±45° of southeast, by the same arithmetic: {d:?}"
        );
    }

    /// `enter window` parses as In, so its opposite is Out and it is treated as directional.
    /// A command that names no direction at all has no bearing to seed from and falls back to
    /// the eight compass points, cardinals then diagonals.
    #[test]
    fn a_move_with_no_direction_falls_back_to_the_plain_order() {
        let g = two_rooms();
        assert_eq!(g.probe_candidates(2, Some(Direction::In))[0], Direction::Out);
        let c = g.probe_candidates(2, None);
        // SQ-1290: with nothing to seed from there is no portal reciprocal either, so this is
        // exactly `PROBE_FALLBACK_DIRS` — not the full `PROBE_DIRS`, which still carries the four
        // portals for callers that want every direction word.
        assert_eq!(c, crate::direction::PROBE_FALLBACK_DIRS.to_vec());
        assert_eq!(c[..4], [Direction::N, Direction::E, Direction::S, Direction::W]);
        assert_eq!(c.len(), 8, "no portal fallback with nothing to seed from: {c:?}");
        for portal in [Direction::Up, Direction::Down, Direction::In, Direction::Out] {
            assert!(!c.contains(&portal), "{portal:?} must not appear unseeded: {c:?}");
        }
    }

    /// Up/Down/In/Out are asked ONLY as the direct reciprocal of a portal move the player just
    /// made — never as a fallback once the compass words run out (SQ-1290). A search seeded by a
    /// COMPASS move must find no portal anywhere in its list; a search seeded by a portal move
    /// must find that one portal and no other.
    #[test]
    fn portals_are_never_a_fallback_only_ever_the_seeded_reciprocal() {
        let g = two_rooms();
        let compass_seeded = g.probe_candidates(2, Some(Direction::N));
        for portal in [Direction::Up, Direction::Down, Direction::In, Direction::Out] {
            assert!(
                !compass_seeded.contains(&portal),
                "a compass move must never fall through to a portal: {compass_seeded:?}"
            );
        }

        let portal_seeded = g.probe_candidates(2, Some(Direction::Down));
        assert_eq!(portal_seeded[0], Direction::Up, "the reciprocal of the player's own move");
        for other in [Direction::Down, Direction::In, Direction::Out] {
            assert!(
                !portal_seeded.contains(&other),
                "no OTHER portal — only the one reciprocal to what was walked: {portal_seeded:?}"
            );
        }
        assert_eq!(portal_seeded.len(), 9, "the seeded Up, plus all eight compass points");
    }

    /// Both records filter, and the order survives the filtering. An unknown room offers
    /// nothing rather than a directions list into the void.
    #[test]
    fn candidates_are_filtered_by_both_records() {
        let mut g = two_rooms();
        g.mark_tried(2, Direction::S); // the player walked into a wall going back
        g.mark_probed(2, Direction::E); // the search has already tried east
        g.add_edge(2, Direction::W, 1); // and west is a known passage
        let c = g.probe_candidates(2, Some(Direction::N));
        for gone in [Direction::S, Direction::E, Direction::W] {
            assert!(!c.contains(&gone), "{gone:?} should be filtered out of {c:?}");
        }
        assert_eq!(c.len(), 5, "eight compass points minus the three filtered away");
        assert_eq!(c[0], Direction::SE, "the surviving head of the priority order");
        assert!(g.probe_candidates(404, None).is_empty());
    }
}
