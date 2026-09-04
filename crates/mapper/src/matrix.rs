//! The direction matrix: what the map knows about a layer as a TABLE rather than a drawing
//! (SQ-0666).
//!
//! Inside a maze the player's knowledge is not geometry, it is a direction table per room:
//! "west from here goes to that room, and the way back is north". A compass layout of Adventure's
//! all-alike maze marks ~62% of its edges distorted, because compass geometry is not what a maze
//! is. This module turns the graph into the table, leaving every glyph, colour and column width to
//! the renderer — the mapper crate stays pure.
//!
//! Everything here is derived — row order, numbering and tags are all functions of the graph, so
//! nothing in this module itself needs a save format — but numbering does lean on one small fact
//! the graph persists: each room's discovery sequence (SQ-0685), stamped once at first upsert. A
//! room's *id* is not usable for this — for a Z-machine game it is the story's own object number,
//! unrelated to when the player found the room — so without a persisted visit order, numbering
//! that used id order instead would renumber every "Maze" room behind a newly-found low-id one.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::direction::{opposite, Direction};
use crate::graph::{Connection, MapGraph, RoomId};
use crate::layer::LayerId;

/// The twelve travel directions, in the order the matrix columns them.
///
/// ALL twelve, always — an untried cell in any direction may be exactly what full exploration
/// needs, so none are hidden however empty the column looks. `Unknown` is not among them: it is a
/// bucket for non-compass passages (xyzzy, pray), not a direction you can type at a compass.
pub const MATRIX_DIRS: [Direction; 12] = [
    Direction::N,
    Direction::S,
    Direction::E,
    Direction::W,
    Direction::NE,
    Direction::NW,
    Direction::SE,
    Direction::SW,
    Direction::Up,
    Direction::Down,
    Direction::In,
    Direction::Out,
];

/// What the map knows about ONE room's ONE direction.
///
/// The renderer maps these to glyphs (`⇄` / `→5⇠w` / `⇢9` / `↩` / `⇱out` / `×` / `·`); the
/// distinctions themselves are graph facts and belong here, where they can be tested against a
/// real map without a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixCell {
    /// The compass inverse returns: go `dir`, come back by `opposite(dir)`. The rarest cell in a
    /// maze — 2 of 47 edges in the reference map.
    Reciprocal { dest: RoomId },
    /// Leads to `dest`, and the way back is known but is NOT the compass inverse. The row is
    /// self-contained: you can read the round trip without leaving it.
    ReturnBy { dest: RoomId, back: Direction },
    /// Leads to `dest`; no way back is known at all.
    OneWay { dest: RoomId },
    /// Leads back into this very room — the classic "west leads back here".
    SelfLoop,
    /// Leaves the layer. The destination is footnoted rather than tagged, because it has no row
    /// in this table to point at.
    LeavesLayer { dest: RoomId },
    /// Tried, and no path was found. A later OBSERVED loop upgrades this to [`MatrixCell::SelfLoop`];
    /// nothing infers one from the probe alone.
    Probed,
    /// Never tried — the exploration frontier.
    Untried,
    /// Tried; the story sent the player somewhere different each time (SQ-1257) —
    /// Lost Pig's gnome tunnels are the specimen. Explored, not a frontier, and
    /// names no SINGLE destination because none is stable enough to name: the
    /// room the story picks varies, so no `dest` can be trusted from one
    /// crossing to the next. A REAL edge in the same direction beats this, and
    /// this beats [`MatrixCell::SelfLoop`] on the same key — see
    /// [`classify_with`].
    ///
    /// `destinations` is a COUNT, not the list (SQ-1261): every distinct room this direction has
    /// actually been seen to land in ([`crate::graph::Room::random_destinations`]), carried here
    /// only so the render layer can draw its superscript without a second graph lookup per cell.
    /// The list itself belongs to the room panel and the dump, which read the graph directly —
    /// `MatrixCell` stays `Copy`, so it carries a size, not the rooms.
    Random { destinations: usize },
}

impl MatrixCell {
    /// The room this cell points at, when it points at one. `None` for the cells that name no
    /// SINGLE destination (`SelfLoop` points at the row's own room, so it names nothing new;
    /// `Random` may have several, which is exactly why it cannot answer with one).
    pub fn dest(&self) -> Option<RoomId> {
        match self {
            MatrixCell::Reciprocal { dest }
            | MatrixCell::ReturnBy { dest, .. }
            | MatrixCell::OneWay { dest }
            | MatrixCell::LeavesLayer { dest } => Some(*dest),
            MatrixCell::SelfLoop
            | MatrixCell::Probed
            | MatrixCell::Untried
            | MatrixCell::Random { .. } => None,
        }
    }

    /// True for the two cells that mark unexplored ground (`×` and `·`) — what the frontier style
    /// dims. [`MatrixCell::Random`] is deliberately excluded: it is explored (the player tried it
    /// and learned the story decides), just not explorable any further.
    pub fn is_frontier(&self) -> bool {
        matches!(self, MatrixCell::Probed | MatrixCell::Untried)
    }
}

/// Classify `dir` out of `room`.
///
/// Precedence, most specific fact first: a REAL destination beats everything else — the graph
/// deliberately keeps a self-loop or a random mark beside a real edge on the same key (see
/// [`MapGraph::add_edge`], [`MapGraph::mark_random_exit`]), and a passage that demonstrably
/// leads somewhere is the more useful fact. Failing that, a RANDOM mark beats a self-loop
/// (SQ-1257 Phase 3): the two are mutually exclusive for any move recorded since the
/// rename-loop check went in (`Mapper::observe_inner` marks one or the other, never both, for
/// the same crossing), but an older map file can carry both on one key, and "the story never
/// commits to a destination" is the stronger of the two things to say about it. `↩` therefore
/// means "the only thing this direction ever did was bring me back, under the same name every
/// time".
///
/// SQ-1269 widens what can produce a mark on this key without changing this precedence at all:
/// a rename-loop stays the one IMMEDIATE mark (structural — the story renamed the room in the
/// same breath, which is proof on its own); an existing self-loop or edge that a NEW landing
/// contradicts is no longer marked on the spot — it is left a suspicion for a probe to judge
/// (`app::random_exit_probe`'s `Suspicion` shape), and only a DISAGREEING probe answer removes
/// the old self-loop/edge and marks the direction (via `Mapper::resolve_suspicion_as_random`),
/// pooling the room itself as a destination when a self-loop was the thing contradicted — the
/// room card's "back here". An AGREEING probe answer instead concludes the passage merely
/// CHANGED and mints straight over the old self-loop/edge with no mark at all
/// (`Mapper::resolve_suspicion_as_changed`). Either way the self-loop and the mark still never
/// coexist going forward; this cell's precedence is what protects an older save that predates
/// the rule from showing both at once.
pub fn classify(graph: &MapGraph, room: RoomId, dir: Direction) -> MatrixCell {
    classify_with(graph, &ConnIndex::new(graph), room, dir)
}

/// The graph's connections indexed by origin room, built once per [`build`]
/// call (SQ-1181).
///
/// `classify` asks "which edges leave this room" up to fifteen times per cell
/// — the dest lookup, the reciprocal check, and one scan per return column —
/// and a table redrawn at animation rate was paying rows x 12 x 15 full-list
/// scans per frame. Each origin's edges keep the whole list's relative order,
/// so the first-match answers are exactly the un-indexed ones.
struct ConnIndex<'a> {
    by_origin: HashMap<RoomId, Vec<&'a Connection>>,
}

impl<'a> ConnIndex<'a> {
    fn new(graph: &'a MapGraph) -> Self {
        let mut by_origin: HashMap<RoomId, Vec<&'a Connection>> = HashMap::new();
        for c in graph.connections() {
            by_origin.entry(c.origin).or_default().push(c);
        }
        ConnIndex { by_origin }
    }

    /// Every connection leaving `room`, in the graph's own insertion order.
    fn from(&self, room: RoomId) -> &[&'a Connection] {
        self.by_origin.get(&room).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// [`classify`], against a prebuilt [`ConnIndex`] — the same logic, scanning
/// only the origin's own edges. The public per-cell `classify` delegates here
/// with a throwaway index (one pass over the list, no worse than the single
/// scan it always cost), so there is exactly one classification to keep right.
fn classify_with(graph: &MapGraph, idx: &ConnIndex<'_>, room: RoomId, dir: Direction) -> MatrixCell {
    if dir == Direction::Unknown {
        return MatrixCell::Untried;
    }
    let dest = idx.from(room).iter().find(|c| c.dir == dir && c.dest != room).map(|c| c.dest);
    let Some(dest) = dest else {
        // A REAL edge (above) beats this, and did not exist — so a direction the story sends
        // somewhere different each time is reported before falling back to a self-loop or the
        // tried/untried read. Checked BEFORE self-loops (SQ-1257 Phase 3): a compass move that
        // returns to the room it left AND renames it is recorded as a random mark, never a
        // self-loop edge (see `Mapper::observe_inner`'s rename-loop check) — but an OLD map file
        // saved before that distinction existed can still carry both a self-loop and a random
        // mark on the same key, and a random exit is the stronger, more specific fact: it says
        // not just "this returns" but "the story never commits to the same room name either".
        // SQ-1257 Phase 2 can UPGRADE this: a random-marked direction that later behaves
        // deterministically gets a real edge and the mark is cleared in the same stroke
        // (`random_exit_probe::deliver`), so the two facts never coexist in the graph for long
        // — but while the mark stands alone (no edge yet, or the edge disagreed and was
        // removed), this is what makes it visible.
        if graph.is_random_exit(room, dir) {
            return MatrixCell::Random { destinations: graph.random_destinations(room, dir).len() };
        }
        if graph.self_loops(room).contains(&dir) {
            return MatrixCell::SelfLoop;
        }
        // No edge at all: the room's own record of what has been TYPED here is the only thing
        // that separates a wall from unexplored ground.
        return if graph.is_tried(room, dir) { MatrixCell::Probed } else { MatrixCell::Untried };
    };
    if graph.layer_of(dest) != graph.layer_of(room) {
        return MatrixCell::LeavesLayer { dest };
    }
    if idx.from(dest).iter().any(|c| c.dir == opposite(dir) && c.dest == room) {
        return MatrixCell::Reciprocal { dest };
    }
    // Any other direction that comes back. Scanned in column order so the answer is stable
    // whatever order the edges were minted in.
    for back in MATRIX_DIRS {
        if idx.from(dest).iter().any(|c| c.dir == back && c.dest == room) {
            return MatrixCell::ReturnBy { dest, back };
        }
    }
    MatrixCell::OneWay { dest }
}

/// Every `(room, direction)` in the graph whose passage ARRIVES at `target` — the answer to "how
/// do I get back here", and the set the matrix bolds when a row is selected.
///
/// Directed, and deliberately not filtered by layer: a way in from outside the layer is still a
/// way in, and the caller decides which of these it can actually draw.
pub fn entrances(graph: &MapGraph, target: RoomId) -> Vec<(RoomId, Direction)> {
    graph
        .connections()
        .iter()
        .filter(|c| c.dest == target && c.dir != Direction::Unknown)
        .map(|c| (c.origin, c.dir))
        .collect()
}

/// Border edges that ENTER `layer` from outside it — the doors a player could walk in through.
///
/// The mirror image of the `⇱out` cells [`classify`] already produces from the other side: there
/// the test is `layer_of(dest) != layer_of(room)` with `room` inside the layer; here it is
/// `layer_of(origin) != layer` with `dest` inside it. `⇱out` cells live in the table itself (the
/// origin has a row to sit on); an inbound edge's origin has no row here at all, so it can only
/// ever be a footnote — this is the query that footnote is built from.
///
/// Ordered by the entering room's row position ([`build`]'s row order — discovery-sequence order,
/// SQ-0685, not room id), then by [`MATRIX_DIRS`] column order, then by origin id: deterministic
/// however the edges were minted, so the block a caller builds from this is stable across a
/// save/load round trip.
pub fn inbound_border_edges(graph: &MapGraph, layer: LayerId) -> Vec<(RoomId, Direction, RoomId)> {
    let row_of: BTreeMap<RoomId, usize> =
        rooms_by_seq(graph, layer).into_iter().enumerate().map(|(i, id)| (id, i)).collect();
    let col_of = |d: Direction| MATRIX_DIRS.iter().position(|&x| x == d).unwrap_or(usize::MAX);

    let mut out: Vec<(RoomId, Direction, RoomId)> = graph
        .connections()
        .iter()
        .filter(|c| {
            c.dir != Direction::Unknown
                && graph.layer_of(c.dest) == layer
                && graph.layer_of(c.origin) != layer
        })
        .map(|c| (c.origin, c.dir, c.dest))
        .collect();
    out.sort_by_key(|&(origin, dir, dest)| {
        (row_of.get(&dest).copied().unwrap_or(usize::MAX), col_of(dir), origin)
    });
    out
}

/// `layer`'s rooms in DISCOVERY order (SQ-0685): ascending [`crate::graph::Room::seq`], the true
/// first-visit order, rather than [`MapGraph::rooms_in_layer`]'s ascending room id — for a
/// Z-machine game the id is the story's own object number and has no relationship to when the
/// player found the room. This is the row order [`build`] draws and the order [`labels`] numbers
/// same-named rooms in, so a room's number never moves when a lower-id duplicate is found later.
///
/// Starts from [`MapGraph::rooms_in_layer`] (ascending id) purely to seed a deterministic sort;
/// every room's `seq` is unique in practice, so that seed order never actually shows through.
fn rooms_by_seq(graph: &MapGraph, layer: LayerId) -> Vec<RoomId> {
    let mut ids = graph.rooms_in_layer(layer);
    ids.sort_by_key(|&id| graph.room(id).map(|r| r.seq).unwrap_or(u64::MAX));
    ids
}

// ── Display naming ────────────────────────────────────────────────────────────

/// Row labels and cell tags for a layer's rooms.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MatrixLabels {
    /// What the label column spells out: `"Maze 3"`, `"Dead End, near Vending Machine"`.
    pub row: BTreeMap<RoomId, String>,
    /// The short reference a destination cell prints: `"3"`, `"DE"`. At most three characters, so
    /// a cell stays inside its column.
    pub tag: BTreeMap<RoomId, String>,
}

impl MatrixLabels {
    pub fn row_of(&self, id: RoomId) -> &str {
        self.row.get(&id).map(String::as_str).unwrap_or("")
    }
    pub fn tag_of(&self, id: RoomId) -> &str {
        self.tag.get(&id).map(String::as_str).unwrap_or("")
    }
}

/// Initials for a room name, upper case, at most three: `"Dead End, near Vending Machine"` → `DE`.
///
/// Only the part before the first comma is used (the tail of an IF room name is nearly always a
/// qualifier), and only capitalised words contribute, which drops the `of`/`the`/`near` filler
/// without a stop-word list.
fn initials(name: &str) -> String {
    let head = name.split(',').next().unwrap_or(name);
    let mut out = String::new();
    for w in head.split_whitespace() {
        let Some(c) = w.chars().next() else { continue };
        if !c.is_uppercase() {
            continue;
        }
        out.push(c);
        if out.chars().count() == 3 {
            break;
        }
    }
    if out.is_empty() {
        out = head.chars().filter(|c| c.is_alphanumeric()).take(3).collect::<String>().to_uppercase();
    }
    if out.is_empty() {
        out.push('?');
    }
    out
}

/// Number the rooms of `layer` for display.
///
/// Rooms that SHARE a display name are numbered in DISCOVERY order (SQ-0685) — eleven rooms called
/// "Maze" become "Maze 1".."Maze 11" in the order the player first walked into each of them —
/// because eleven identical rows is exactly the problem the matrix exists to solve, and the
/// player's own mental numbering is built while exploring, not sorted by the story's object table.
/// A number is minted the moment a room is first discovered and never changes afterward: finding a
/// LOWER-id duplicate later does not renumber the ones already found, because numbering was never
/// keyed on id in the first place. The numbering is otherwise display-only — identity stays the
/// room id — so it is stable across a save/load round trip exactly as before.
///
/// A uniquely-named room gets its initials instead of a number, so cells can point at it without
/// stealing a number from the numbered group.
pub fn labels(graph: &MapGraph, layer: LayerId) -> MatrixLabels {
    let ids = rooms_by_seq(graph, layer);
    let name_of = |id: RoomId| graph.room(id).map(|r| r.label().to_string()).unwrap_or_default();

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for &id in &ids {
        *counts.entry(name_of(id)).or_insert(0) += 1;
    }
    // When only ONE name repeats, its numbers are unambiguous on their own ("5" can only be
    // Maze 5). With two repeating names the numbers would collide, so both carry initials.
    let repeating = counts.values().filter(|&&n| n > 1).count();

    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut out = MatrixLabels::default();
    for &id in &ids {
        let name = name_of(id);
        let (row, tag) = if counts.get(&name).copied().unwrap_or(0) > 1 {
            let n = seen.entry(name.clone()).and_modify(|v| *v += 1).or_insert(1);
            let tag = if repeating == 1 {
                n.to_string()
            } else {
                format!("{}{}", initials(&name), n)
            };
            (format!("{name} {n}"), tag)
        } else {
            (name.clone(), initials(&name))
        };
        out.row.insert(id, row);
        out.tag.insert(id, tag);
    }

    // Force tags unique: two differently-named rooms can still share initials. The LATER row
    // yields, so adding a room never renames one the player has already learned.
    let mut taken: BTreeSet<String> = BTreeSet::new();
    for &id in &ids {
        let base = out.tag.get(&id).cloned().unwrap_or_default();
        if taken.insert(base.clone()) {
            continue;
        }
        let mut n = 2;
        loop {
            let candidate = format!("{base}{n}");
            if taken.insert(candidate.clone()) {
                out.tag.insert(id, candidate);
                break;
            }
            n += 1;
        }
    }
    out
}

// ── The table ─────────────────────────────────────────────────────────────────

/// One room's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixRow {
    pub room: RoomId,
    /// The display label, already numbered (`"Maze 3"`).
    pub label: String,
    /// The short reference other rows' cells use for this room (`"3"`).
    pub tag: String,
    /// One cell per [`MATRIX_DIRS`] entry, same order.
    pub cells: [MatrixCell; 12],
}

/// A layer as a direction table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matrix {
    pub layer: LayerId,
    pub rows: Vec<MatrixRow>,
    pub labels: MatrixLabels,
    /// The room the player is standing in, when it is in this layer.
    pub here: Option<RoomId>,
}

impl Matrix {
    /// The row index of `room`, if it has one.
    pub fn index_of(&self, room: RoomId) -> Option<usize> {
        self.rows.iter().position(|r| r.room == room)
    }
}

/// Build the direction table for `layer`.
///
/// Rows are in DISCOVERY order (SQ-0685): ascending [`crate::graph::Room::seq`], the same order
/// [`labels`] numbers same-named rooms in, so a row's position and the number in its own label
/// always agree — row 1 really is "Maze 1". That order is persisted (each room's `seq`, stamped
/// once at first upsert, plus the graph's `next_seq` counter), so a map reload draws the rows in
/// exactly the same order and a player's "the one I called 7" keeps meaning the same room.
pub fn build(graph: &MapGraph, layer: LayerId) -> Matrix {
    let labels = labels(graph, layer);
    // One index for the whole table (SQ-1181): rows x 12 cells all classify
    // against it instead of each rescanning the full connection list.
    let idx = ConnIndex::new(graph);
    let rows = rooms_by_seq(graph, layer)
        .into_iter()
        .map(|id| MatrixRow {
            room: id,
            label: labels.row_of(id).to_string(),
            tag: labels.tag_of(id).to_string(),
            cells: MATRIX_DIRS.map(|d| classify_with(graph, &idx, id, d)),
        })
        .collect();
    let here = graph.current().filter(|&id| graph.layer_of(id) == layer);
    Matrix { layer, rows, labels, here }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::MAIN_LAYER;

    /// A miniature of the reference map: a numbered group, one uniquely-named room, and one edge
    /// out of the layer.
    fn maze() -> (MapGraph, LayerId) {
        let mut g = MapGraph::new();
        for (id, n) in [
            (1u16, "Maze"),
            (2, "Maze"),
            (3, "Maze"),
            (4, "Dead End, near Vending Machine"),
            (9, "At West End of Long Hall"),
        ] {
            g.upsert_room(id.into(), n.into());
        }
        let l = g.new_layer(Some(MAIN_LAYER), "Maze".into());
        for id in [1, 2, 3, 4] {
            g.set_room_layer(id, l);
        }
        g.add_edge(1, Direction::N, 2); // 2 returns by W, not S
        g.add_edge(2, Direction::W, 1);
        g.add_edge(1, Direction::E, 3); // no return known
        g.add_edge(3, Direction::S, 4); // reciprocal pair
        g.add_edge(4, Direction::N, 3);
        g.add_edge(2, Direction::Down, 9); // leaves the layer
        g.mark_tried(4, Direction::E); // tried east, hit a wall
        (g, l)
    }

    #[test]
    fn every_cell_of_the_vocabulary_classifies() {
        let (mut g, _l) = maze();
        assert_eq!(classify(&g, 1, Direction::N), MatrixCell::ReturnBy { dest: 2, back: Direction::W });
        assert_eq!(classify(&g, 1, Direction::E), MatrixCell::OneWay { dest: 3 });
        assert_eq!(classify(&g, 3, Direction::S), MatrixCell::Reciprocal { dest: 4 });
        assert_eq!(classify(&g, 4, Direction::N), MatrixCell::Reciprocal { dest: 3 });
        assert_eq!(classify(&g, 2, Direction::Down), MatrixCell::LeavesLayer { dest: 9 });
        assert_eq!(classify(&g, 4, Direction::E), MatrixCell::Probed, "tried east, no path");
        assert_eq!(classify(&g, 4, Direction::W), MatrixCell::Untried, "never tried west");
        assert_eq!(classify(&g, 1, Direction::Unknown), MatrixCell::Untried, "no column for `?`");

        // An observed loop upgrades a probe.
        assert!(g.add_self_loop(4, Direction::E));
        assert_eq!(classify(&g, 4, Direction::E), MatrixCell::SelfLoop, "the probe became a loop");

        // …but a real destination on the same key still wins: the loop is the fallback fact.
        g.add_edge(4, Direction::E, 1);
        assert_eq!(classify(&g, 4, Direction::E), MatrixCell::OneWay { dest: 1 });
        assert!(g.self_loops(4).contains(&Direction::E), "and the loop is not destroyed");
    }

    /// SQ-1257: a random exit reads as `?`, beats `Probed`/`Untried`, is beaten by a real edge,
    /// and never counts as a frontier.
    #[test]
    fn a_random_exit_beats_probed_and_untried_but_a_real_edge_beats_it() {
        let (mut g, _l) = maze();
        assert_eq!(classify(&g, 1, Direction::S), MatrixCell::Untried, "never tried south");

        g.mark_random_exit(1, Direction::S);
        assert_eq!(
            classify(&g, 1, Direction::S),
            MatrixCell::Random { destinations: 0 },
            "random beats untried, no destinations recorded yet"
        );
        assert!(!classify(&g, 1, Direction::S).is_frontier(), "random is explored, not a frontier");
        assert!(g.untried(1).iter().all(|&d| d != Direction::S), "and drops out of the untried list");

        g.mark_random_exit(4, Direction::E); // already Probed from `maze()`'s mark_tried
        assert_eq!(
            classify(&g, 4, Direction::E),
            MatrixCell::Random { destinations: 0 },
            "random beats a bare probe too"
        );

        // SQ-1257 Phase 2: a random mark can be UPGRADED — a direction that later behaves
        // deterministically gets a real edge, via `random_exit_probe::deliver`, which clears the
        // mark in the same stroke (`MapGraph::unmark_random_exit`). The classifier does not
        // trust that pairing to be perfect on its own: an edge sitting beside a mark that,
        // for whatever reason, was not cleared must still read as the edge, not the mark —
        // a stale "destination varies" badge on a passage the map can now name is a worse lie
        // than briefly trusting an edge the mapper itself just placed.
        g.add_edge(1, Direction::S, 4);
        assert_eq!(
            classify(&g, 1, Direction::S),
            MatrixCell::OneWay { dest: 4 },
            "a real edge in the same key wins over an un-cleared random mark"
        );
        assert!(g.is_random_exit(1, Direction::S), "the random record itself is untouched by classify");
    }

    /// SQ-1261: the `Random` cell's `destinations` count tracks
    /// [`MapGraph::note_random_destination`], not just whether the direction is marked.
    #[test]
    fn a_random_exit_carries_its_recorded_destination_count() {
        let (mut g, _l) = maze();
        g.mark_random_exit(1, Direction::S);
        assert_eq!(classify(&g, 1, Direction::S), MatrixCell::Random { destinations: 0 });

        g.note_random_destination(1, Direction::S, 2);
        assert_eq!(classify(&g, 1, Direction::S), MatrixCell::Random { destinations: 1 });

        g.note_random_destination(1, Direction::S, 3);
        g.note_random_destination(1, Direction::S, 2); // repeat — no change
        assert_eq!(classify(&g, 1, Direction::S), MatrixCell::Random { destinations: 2 });
    }

    /// SQ-1257 Phase 3: a key that carries BOTH a self-loop and a random mark — which
    /// `Mapper::observe_inner`'s rename-loop check never produces for one crossing, but an older
    /// map file can — must read as `Random`, the more specific fact ("the story never even
    /// commits to a destination room name"). Falsify by reverting `classify_with`'s check order
    /// back to self-loop-before-random and this fails on the first assertion.
    #[test]
    fn a_random_exit_beats_a_self_loop_on_the_same_key() {
        let (mut g, _l) = maze();
        // South out of room 1 carries no real edge in `maze()`, so a self-loop recorded there
        // is the only fact on the key.
        assert!(g.add_self_loop(1, Direction::S));
        assert_eq!(classify(&g, 1, Direction::S), MatrixCell::SelfLoop, "a bare loop reads as a loop");

        g.mark_random_exit(1, Direction::S);
        assert_eq!(
            classify(&g, 1, Direction::S),
            MatrixCell::Random { destinations: 0 },
            "random beats a self-loop recorded on the same key"
        );
        assert!(g.self_loops(1).contains(&Direction::S), "the self-loop record itself survives");
    }

    /// SQ-1181: `build` classifies against a shared per-call [`ConnIndex`];
    /// the public per-cell `classify` builds a throwaway one. Both delegate to
    /// the same `classify_with`, and this pins that the table really carries
    /// the per-cell answers — the guard against the two routes ever drifting.
    #[test]
    fn build_cells_agree_with_per_cell_classify() {
        let (g, l) = maze();
        let m = build(&g, l);
        for row in &m.rows {
            for (i, cell) in row.cells.iter().enumerate() {
                assert_eq!(
                    *cell,
                    classify(&g, row.room, MATRIX_DIRS[i]),
                    "room {} {:?}",
                    row.room,
                    MATRIX_DIRS[i]
                );
            }
        }
    }

    #[test]
    fn same_named_rooms_number_in_row_order_and_unique_names_get_initials() {
        let (g, l) = maze();
        let lbl = labels(&g, l);
        assert_eq!(lbl.row_of(1), "Maze 1");
        assert_eq!(lbl.row_of(3), "Maze 3");
        assert_eq!(lbl.tag_of(3), "3", "one repeating name → bare numbers");
        assert_eq!(lbl.row_of(4), "Dead End, near Vending Machine");
        assert_eq!(lbl.tag_of(4), "DE", "initials of the part before the comma, filler dropped");
    }

    /// SQ-0685: the reported bug, reproduced directly. Numbering by ascending room id renumbers
    /// EVERY duplicate behind a newly-found lower-id one — for a Z-machine game the id is the
    /// story's own object number and has nothing to do with when the player found the room.
    /// Falsified against HEAD: reverting the `rooms_by_seq` ordering in `labels`/`build` back to
    /// plain `rooms_in_layer` makes `after.row_of(5)` come back `"Maze 2"` here, reproducing the
    /// exact symptom reported.
    #[test]
    fn a_lower_id_duplicate_discovered_later_does_not_renumber_earlier_rooms() {
        let mut g = MapGraph::new();
        // Two "Maze" rooms first, both with ids HIGHER than the one found later.
        g.upsert_room(5, "Maze".into()); // seq 0
        g.upsert_room(7, "Maze".into()); // seq 1
        let before = labels(&g, MAIN_LAYER);
        assert_eq!(before.row_of(5), "Maze 1");
        assert_eq!(before.row_of(7), "Maze 2");

        g.upsert_room(2, "Maze".into()); // a LOWER id than both, discovered third → seq 2
        let after = labels(&g, MAIN_LAYER);
        assert_eq!(after.row_of(5), "Maze 1", "room 5 keeps its number: id 2 arriving later must not move it");
        assert_eq!(after.row_of(7), "Maze 2", "nor must it move room 7");
        assert_eq!(after.row_of(2), "Maze 3", "the newcomer takes the next ordinal, whatever its id");

        // A revisit (upsert on an already-known room) must not re-mint its ordinal either.
        g.upsert_room(5, "Maze".into());
        assert_eq!(labels(&g, MAIN_LAYER).row_of(5), "Maze 1", "a revisit does not renumber");
    }

    /// `build`'s row order and `labels`' numbering have to agree — row 1 really is "Maze 1" — so
    /// both must be driven by the same discovery-sequence order, not room id (SQ-0685).
    #[test]
    fn build_row_order_follows_discovery_order_not_room_id() {
        let mut g = MapGraph::new();
        g.upsert_room(5, "E".into());
        g.upsert_room(2, "B".into());
        g.upsert_room(9, "I".into());
        let m = build(&g, MAIN_LAYER);
        assert_eq!(
            m.rows.iter().map(|r| r.room).collect::<Vec<_>>(),
            vec![5, 2, 9],
            "rows are in the order the rooms were discovered, not ascending id"
        );
    }

    /// The numbering is display-only and derived, so it must be identical after a round trip
    /// through the map file — a player who calls a room "7" must still find 7 tomorrow.
    #[test]
    fn numbering_survives_save_and_load() {
        let (g, l) = maze();
        let before = labels(&g, l);
        let m = crate::mapper::Mapper { graph: g, ..Default::default() };
        let json = crate::persist::to_json(&m);
        let m2 = crate::persist::from_json(&json).expect("round trip");
        assert_eq!(labels(&m2.graph, l), before);
    }

    #[test]
    fn two_repeating_names_disambiguate_their_numbers() {
        let mut g = MapGraph::new();
        for (id, n) in [(1u16, "Maze"), (2, "Maze"), (3, "Cave"), (4, "Cave")] {
            g.upsert_room(id.into(), n.into());
        }
        let lbl = labels(&g, MAIN_LAYER);
        assert_eq!(lbl.tag_of(2), "M2");
        assert_eq!(lbl.tag_of(4), "C2", "a bare `2` would name two different rooms");
        assert_ne!(lbl.tag_of(2), lbl.tag_of(4));
    }

    #[test]
    fn colliding_initials_are_forced_apart_and_the_earlier_row_keeps_its_tag() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Dark Room".into());
        g.upsert_room(2, "Damp Ruin".into()); // also "DR"
        let lbl = labels(&g, MAIN_LAYER);
        assert_eq!(lbl.tag_of(1), "DR", "the earlier row never renames");
        assert_eq!(lbl.tag_of(2), "DR2");
    }

    #[test]
    fn build_lays_out_twelve_columns_per_room_and_marks_here() {
        let (mut g, l) = maze();
        g.set_current(2);
        let m = build(&g, l);
        assert_eq!(m.rows.len(), 4, "only the layer's rooms");
        assert_eq!(m.rows[0].cells.len(), 12);
        assert_eq!(m.here, Some(2));
        assert_eq!(m.index_of(4), Some(3));
        // Nothing outside the layer gets a row, even though room 9 is a destination.
        assert!(m.rows.iter().all(|r| r.room != 9));

        g.set_current(9); // stand outside the layer
        assert_eq!(build(&g, l).here, None, "the here-marker belongs to the layer you are in");
    }

    #[test]
    fn entrances_answer_how_do_i_get_back_here() {
        let (g, _) = maze();
        let e = entrances(&g, 3);
        assert_eq!(e.len(), 2, "{e:?}");
        assert!(e.contains(&(1, Direction::E)));
        assert!(e.contains(&(4, Direction::N)));
        assert!(entrances(&g, 9).contains(&(2, Direction::Down)), "a way in from another layer counts");
    }

    /// The mirror of `⇱out`: an edge whose ORIGIN is outside the layer and whose destination is
    /// inside it. `maze()` already has one crossing the other way (room 2 → room 9, `Down`); this
    /// gives room 9 a way back IN, which must show up here and not be confused with the outbound
    /// one.
    #[test]
    fn inbound_border_edges_are_the_doors_into_the_layer() {
        let (mut g, l) = maze();
        g.add_edge(9, Direction::W, 1); // enters at the first row
        g.add_edge(9, Direction::S, 3); // enters at the third row
        let edges = inbound_border_edges(&g, l);
        assert_eq!(
            edges,
            vec![(9, Direction::W, 1), (9, Direction::S, 3)],
            "ordered by the entering room's row position, not insertion order"
        );
        // The existing OUTBOUND crossing (2 → 9) must never appear as inbound.
        assert!(edges.iter().all(|&(o, _, d)| o != 2 || d != 9));
        assert!(inbound_border_edges(&g, l).iter().all(|&(o, _, _)| g.layer_of(o) != l));
    }
}
