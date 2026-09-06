//! Map layers ("segments"): a manual organizing tool. Every room belongs to exactly
//! one layer (default `MAIN_LAYER`). Layers are created/destroyed only by explicit
//! `move-region` — never derived. See docs/superpowers/specs/2026-06-23-manual-map-layers-design.md.

use std::collections::{BTreeSet, VecDeque};

/// Stable layer identifier. Layer `0` (`MAIN_LAYER`) always exists.
pub type LayerId = u16;

/// The permanent base layer every room starts in.
pub const MAIN_LAYER: LayerId = 0;

/// How the map pane draws a layer (SQ-0666). A view mode, not a maze feature: any layer
/// can be shown either way, and the choice is per-layer and persisted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MapView {
    /// The compass-laid-out map (rooms in boxes, connectors between them).
    #[default]
    Drawn,
    /// One row per room, one column per direction — the knowledge a maze actually gives
    /// you, without drawing geometry that is mostly wrong.
    Matrix,
}

/// Per-layer metadata: a display name and the layer it was peeled from (for merge default).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LayerMeta {
    pub name: String,
    pub parent: Option<LayerId>,
    /// The player has declared this layer a maze (SQ-0666). It changes nothing on its own —
    /// it merely decides the DEFAULT view (matrix) for a layer whose view the player has not
    /// chosen explicitly, and turns on maze-only furniture like the walked-trail breadcrumb.
    #[serde(default)]
    pub maze: bool,
    /// The player's EXPLICIT view choice for this layer, when they have made one. `None`
    /// means "whatever the default is", which [`LayerMeta::maze`] decides — so flagging a
    /// layer as a maze switches it to the matrix, but never overrides a view the player
    /// picked by hand, and unflagging it puts an unchosen layer back to drawn.
    #[serde(default)]
    pub view: Option<MapView>,
}

impl LayerMeta {
    /// Metadata for the base "Main" layer.
    pub fn main() -> Self {
        LayerMeta::new("Main".to_string(), None)
    }

    /// A fresh layer with the default flags.
    pub fn new(name: String, parent: Option<LayerId>) -> Self {
        LayerMeta { name, parent, maze: false, view: None }
    }

    /// The view this layer actually draws in: the player's explicit choice when they made
    /// one, otherwise the default that [`LayerMeta::maze`] implies.
    pub fn effective_view(&self) -> MapView {
        self.view.unwrap_or(if self.maze { MapView::Matrix } else { MapView::Drawn })
    }
}

use crate::direction::{grid_offset, opposite, Direction};
use crate::graph::{Connection, MapGraph, RoomId};

/// A set of rooms to re-home onto a layer, together with the room it was computed FROM.
///
/// Computing the set and moving it are two concerns, and keeping them apart is half of what
/// SQ-0439 bought: [`planar_region`], [`region_at_edge`] and [`region_at_arrival`] answer "which
/// rooms", [`move_region`] answers "onto what", and neither knows the other's refusals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    /// The room the region was computed from — always a member of `rooms`. It names a fresh
    /// layer, exactly as the old `peel_region(start)` named one after `start`.
    pub anchor: RoomId,
    /// Every room that moves, `anchor` included. All on one layer by construction: every
    /// region walk here refuses to leave the layer it started in.
    pub rooms: BTreeSet<RoomId>,
}

/// Rooms reachable from `start` through PLANAR edges only (cardinals + diagonals),
/// staying within `start`'s current layer. Portal edges (Up/Down/In/Out/Unknown) are
/// not traversed — they are the cut. Edges are treated as undirected for reachability.
pub fn planar_region(graph: &MapGraph, start: RoomId) -> Region {
    Region { anchor: start, rooms: region_with_cut(graph, start, &|_| false) }
}

/// Every room on `layer`, anchored on the lowest-numbered one. `None` when the layer is empty —
/// there is nothing to move. This is the whole-layer region the old `merge_layer` moved.
pub fn layer_region(graph: &MapGraph, layer: LayerId) -> Option<Region> {
    let rooms = graph.rooms_in_layer(layer);
    let anchor = *rooms.first()?;
    Some(Region { anchor, rooms: rooms.into_iter().collect() })
}

/// True when `region` is everything on its layer.
///
/// The shape [`move_region`] refuses against a NEW target — there is nothing to separate it from,
/// so the move would only rename the layer — and harmless against an existing one, where moving a
/// whole layer's contents into another IS a merge.
pub fn is_whole_layer(graph: &MapGraph, region: &Region) -> bool {
    let all = graph.rooms_in_layer(graph.layer_of(region.anchor));
    all.len() == region.rooms.len() && all.iter().all(|id| region.rooms.contains(id))
}

/// [`planar_region`], plus `cut`: any connection it accepts is severed too, on top of the
/// portal-edge cut. Lets a caller name its own seam (SQ-0360).
fn region_with_cut(
    graph: &MapGraph,
    start: RoomId,
    cut: &dyn Fn(&Connection) -> bool,
) -> BTreeSet<RoomId> {
    let layer = graph.layer_of(start);
    let mut seen = BTreeSet::new();
    seen.insert(start);
    let mut q = VecDeque::new();
    q.push_back(start);
    while let Some(cur) = q.pop_front() {
        for c in graph.connections() {
            if grid_offset(c.dir).is_none() || cut(c) {
                continue; // portal edge, or the caller's own seam → cut
            }
            let other = if c.origin == cur {
                c.dest
            } else if c.dest == cur {
                c.origin
            } else {
                continue;
            };
            if graph.layer_of(other) == layer && seen.insert(other) {
                q.push_back(other);
            }
        }
    }
    seen
}

/// Why a region could not be COMPUTED. Each variant is a distinct thing to tell the player — a
/// command that refuses without saying why reads as broken (SQ-0360). None of these is about
/// where the rooms were headed; that is [`MoveRefusal`]'s job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionRefusal {
    /// The room has no passage that way.
    NoSuchPassage,
    /// The passage exists, but its two ends stay connected by some other route, so cutting it
    /// separates nothing.
    NotASeam,
    /// The passage leads out of the layer. A seam divides a layer; it does not cross one.
    LeavesLayer,
}

/// The region on `from`'s side of a NAMED seam: cut the `dir` passage out of `from` and walk
/// `from`'s own side (SQ-0360).
///
/// [`planar_region`] can only stop where a portal edge already divides a layer, so it is powerless
/// on a layer that is one connected sprawl — Zork's underground being 35 rooms of solid compass
/// maze. This lets the player say where the boundary is instead of waiting for one to exist.
///
/// It takes `from`'s side, exactly as [`planar_region`] takes `start`'s. Taking the FAR side
/// instead made one command mean two opposite things depending on which end of the passage you
/// stood at — and worse, left the anchor room behind while its neighbours moved, so a fresh layer
/// became a SIBLING of the rooms it connects to rather than a child of them. Merging then threw
/// those rooms up to a layer they have no connection to at all (SQ-0364).
///
/// The cut takes the passage's RECIPROCAL with it. A passage is normally two connections
/// (`A -E-> B` and `B -W-> A`), so severing only the named one leaves the back-edge holding the two
/// halves together and no seam could ever cut. It is deliberately just that pair, not every edge
/// between the rooms: if `A` also reaches `B` another way, that is a second passage, the boundary
/// is not real, and `NotASeam` says so.
///
/// Nothing is deleted. The cut is only a PREDICATE for the walk, so the passage survives as a
/// cross-layer connection once the region moves — which is why a move in either direction is
/// reversible and why peel and merge could be unified at all (SQ-0439).
pub fn region_at_edge(
    graph: &MapGraph,
    from: RoomId,
    dir: Direction,
) -> Result<Region, RegionRefusal> {
    region_across(graph, from, dir, false)
}

/// The region on the far side of the passage the player just WALKED IN THROUGH: cut the
/// `from -dir-> dest` pair and walk `dest`'s side — `dest` being the room the player now stands
/// in (SQ-0552).
///
/// A bare move means "separate the area I just entered from the one behind me", so the seam is the
/// incoming edge. [`region_at_edge`] cannot express it: the passage may be one-way, and even when
/// it is not, the way back is often not the reciprocal — Adventure's maze is entered SOUTH from
/// the Long Hall and left by going DOWN, so cutting the current room's north exit cuts a
/// maze-internal edge and separates nothing.
///
/// The kept side is still the one containing the room the player is standing in, exactly as for
/// [`region_at_edge`] and [`planar_region`] — only the seam differs. Keeping the other end instead
/// would leave the player looking at a layer they are not in, the mirror of the SQ-0364 mistake.
pub fn region_at_arrival(
    graph: &MapGraph,
    from: RoomId,
    dir: Direction,
) -> Result<Region, RegionRefusal> {
    region_across(graph, from, dir, true)
}

/// Shared body of [`region_at_edge`] and [`region_at_arrival`]: cut the `from -dir-> dest`
/// passage and walk one of its ends. `keep_far` picks which end — `dest`'s when true, `from`'s
/// when false.
fn region_across(
    graph: &MapGraph,
    from: RoomId,
    dir: Direction,
    keep_far: bool,
) -> Result<Region, RegionRefusal> {
    if grid_offset(dir).is_none() {
        // A portal is already a cut: `planar_region` is the walk for those.
        return Err(RegionRefusal::NoSuchPassage);
    }
    let dest = graph
        .connections()
        .iter()
        .find(|c| c.origin == from && c.dir == dir)
        .map(|c| c.dest)
        .ok_or(RegionRefusal::NoSuchPassage)?;
    let src = graph.layer_of(from);
    if graph.layer_of(dest) != src {
        return Err(RegionRefusal::LeavesLayer);
    }
    let back = opposite(dir);
    let (keep, shed) = if keep_far { (dest, from) } else { (from, dest) };
    let rooms = region_with_cut(graph, keep, &|c: &Connection| {
        // Only the named passage and its exact reciprocal — `dest -back-> from`, not dest's
        // back-direction edge to just ANY room. Without the `c.dest == from` check, a maze-ish
        // A-E->B, B-W->C, C-E->A wrongly cut B's west edge to C too, so a non-seam peeled and
        // the surviving B-W->C edge straddled the "boundary" (SQ-0631).
        (c.origin == from && c.dir == dir) || (c.origin == dest && c.dir == back && c.dest == from)
    });
    if rooms.contains(&shed) {
        return Err(RegionRefusal::NotASeam);
    }
    Ok(Region { anchor: keep, rooms })
}

/// A passage from OUTSIDE into a room, whose cut is a real boundary — a seam a move may take,
/// paired with the region taking it would move (SQ-0439).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundSeam {
    /// The room the passage comes FROM. Cutting the seam leaves it behind, which is what makes it
    /// "outside": the walk that built `region` proved it so rather than assuming it.
    pub from: RoomId,
    /// The direction of travel, `from` → the room the seam was computed for. The passage may be
    /// ONE-WAY — that is the case the old direction-out-of-the-current-room surface could not name
    /// at all, and the whole reason this list is oriented as arrivals.
    pub dir: Direction,
    /// The rooms that move when this seam is cut: the anchor room's own side, `from` excluded.
    pub region: Region,
}

/// Every passage INTO `room` that is a real boundary, in graph order (SQ-0439).
///
/// This is the set a move may legally cut, and it is usually tiny — often one, which is why a
/// player need never point at an edge. The observation it rests on: a move can only ever sever a
/// BRIDGE, because [`region_at_arrival`] refuses [`RegionRefusal::NotASeam`] the moment the two
/// ends stay connected another way. So "which edges could this cut?" is not a question about the
/// whole graph, it is this list.
///
/// Membership is decided by running the cut, not by guessing: an inbound connection is a seam iff
/// [`region_at_arrival`] accepts it, which also throws out the cases that were never candidates —
/// portals (already a cut, and [`planar_region`]'s job), passages from another layer, and self
/// loops.
pub fn inbound_seams(graph: &MapGraph, room: RoomId) -> Vec<InboundSeam> {
    let mut out = Vec::new();
    for c in graph.connections() {
        if c.dest != room || c.origin == room {
            continue;
        }
        if let Ok(region) = region_at_arrival(graph, c.origin, c.dir) {
            out.push(InboundSeam { from: c.origin, dir: c.dir, region });
        }
    }
    out
}

/// True iff the connection's endpoints are in different layers.
pub fn is_interlayer(graph: &MapGraph, conn: &Connection) -> bool {
    graph.layer_of(conn.origin) != graph.layer_of(conn.dest)
}

/// Where a region lands. Peel and merge differ ONLY here: a peel mints a layer, a merge names
/// one. That is a parameter, not a second operation (SQ-0439).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveTarget {
    /// Mint a fresh layer, parented on the layer the rooms leave and named after the region's
    /// anchor room. The old `peel-layer`.
    New,
    /// An existing layer. The old `merge-layer`.
    Existing(LayerId),
}

/// Why a move could not happen. Like [`RegionRefusal`], each variant is a distinct thing to tell
/// the player (SQ-0687). None of these is about which rooms were chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveRefusal {
    /// The region spans its whole layer and the target is a NEW one: there is nothing to separate
    /// it from, so the move would only rename the layer. Harmless against an existing target,
    /// where moving a whole layer's contents into another IS a merge.
    WholeLayer,
    /// The move would leave `MAIN_LAYER` with no rooms at all. Main is the floor everything else
    /// folds into; moving a REGION out of it stays legal, emptying it does not.
    EmptiesMain,
    /// The rooms are already on the target layer. There is nothing to do.
    SelfMove,
    /// The target layer does not exist (or was itself merged away).
    NoSuchLayer,
}

/// The layer `layer` folds back into: its recorded parent when that still exists, else
/// `MAIN_LAYER`.
///
/// The recorded parent may itself have been merged away since the peel (peel L1, peel L2 off it,
/// merge L1, merge L2). Re-homing rooms onto a dead layer id would strand them invisibly forever —
/// ids are never reused. A merged layer's metadata is removed outright, so a missing parent has no
/// grandparent left to walk to: fall back to MAIN_LAYER (SQ-0631).
pub fn parent_layer(graph: &MapGraph, layer: LayerId) -> LayerId {
    graph
        .layers()
        .get(&layer)
        .and_then(|m| m.parent)
        .filter(|p| graph.layers().contains_key(p))
        .unwrap_or(MAIN_LAYER)
}

/// Re-home `region` onto `target`. Returns the layer the rooms landed on.
///
/// This is the whole of peel AND merge (SQ-0439): both were always "move a set of rooms onto a
/// layer", differing only in whether the destination is minted or named. Nothing is deleted from
/// the graph — a passage that now straddles the boundary simply becomes a cross-layer connection —
/// so a move in either direction is reversible by another move.
///
/// A source layer left with no rooms loses its metadata, which is exactly what folding a whole
/// layer into another used to do explicitly. `MAIN_LAYER` never goes: it cannot be emptied.
///
/// Position reconciliation is the one genuine asymmetry, and it belongs to the existing-target
/// branch alone — a freshly minted layer has nothing to collide with. Rooms keep their cells where
/// those are free in the target; a room whose cell is already taken lands on the nearest free one,
/// because two layers laid out independently have no reason to interleave cleanly and two rooms on
/// one cell would draw as one.
///
/// A move onto an EXISTING layer also silences the auto-suggest at every passage it just folded
/// shut (SQ-0439). Putting rooms together is an answer — the strongest one the player can give —
/// and the detector must not turn round and offer to take them apart again next time they walk
/// through. Carving a fresh layer needs no such note: the seam then spans two layers, and a
/// crossing that already spans two layers has nothing left to separate.
pub fn move_region(
    graph: &mut MapGraph,
    region: &Region,
    target: MoveTarget,
) -> Result<LayerId, MoveRefusal> {
    let src = graph.layer_of(region.anchor);
    let is_whole = is_whole_layer(graph, region);
    let dest = match target {
        MoveTarget::New => {
            if is_whole {
                return Err(MoveRefusal::WholeLayer);
            }
            let name = graph.room(region.anchor).map(|r| r.label().to_string()).unwrap_or_default();
            graph.new_layer(Some(src), name)
        }
        MoveTarget::Existing(t) => {
            if !graph.layers().contains_key(&t) {
                return Err(MoveRefusal::NoSuchLayer);
            }
            if t == src {
                return Err(MoveRefusal::SelfMove);
            }
            if src == MAIN_LAYER && is_whole {
                return Err(MoveRefusal::EmptiesMain);
            }
            t
        }
    };
    // Only an existing target can collide; a layer minted a line ago holds no rooms at all.
    let mut occupied = match target {
        MoveTarget::New => BTreeSet::new(),
        MoveTarget::Existing(t) => crate::layout::occupied_cells_in_layer(graph, t),
    };
    for &id in &region.rooms {
        if matches!(target, MoveTarget::Existing(_)) {
            if let Some(pos) = graph.room(id).and_then(|r| r.pos) {
                if occupied.contains(&pos) {
                    let cell = crate::layout::nearest_free_cell(&occupied, pos);
                    graph.set_pos(id, cell);
                    occupied.insert(cell);
                } else {
                    occupied.insert(pos);
                }
            }
        }
        graph.set_room_layer(id, dest);
    }
    if graph.rooms_in_layer(src).is_empty() {
        graph.remove_layer(src); // a no-op for MAIN_LAYER, which is never emptied anyway
    }
    if matches!(target, MoveTarget::Existing(_)) {
        silence_boundary_seams(graph, region);
    }
    Ok(dest)
}

/// Mark every passage across `region`'s boundary as ignored, so the auto-suggest never proposes
/// re-opening a boundary the player has just folded shut (SQ-0439).
///
/// Both orientations of a reciprocal pair are recorded, because the triggers key on opposite ends
/// of the same passage — the way out when a region is noticed on the way out of it, the way in when
/// it is noticed from inside.
fn silence_boundary_seams(graph: &mut MapGraph, region: &Region) {
    let keys: Vec<crate::suggest::SeamKey> = graph
        .connections()
        .iter()
        .filter(|c| region.rooms.contains(&c.origin) != region.rooms.contains(&c.dest))
        .map(|c| crate::suggest::SeamKey { from: c.origin, dir: c.dir })
        .collect();
    for key in keys {
        graph.set_seam_decision(key, crate::suggest::SeamDecision::Ignored);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use crate::graph::MapGraph;

    fn two_floors() -> MapGraph {
        // Floor 1: 1-E-2 (planar). 1-Down-3 (portal) to floor 2: 3-E-4 (planar).
        let mut g = MapGraph::new();
        for (id, n) in [(1, "Hall"), (2, "Study"), (3, "Cellar"), (4, "Wine")] {
            g.upsert_room(id, n.into());
        }
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        g.add_edge(1, Direction::Down, 3);
        g.add_edge(3, Direction::Up, 1);
        g.add_edge(3, Direction::E, 4);
        g.add_edge(4, Direction::W, 3);
        g
    }

    /// Peel `start`'s planar region into a fresh layer — the old `peel_region`, now the two
    /// halves it always was. Every test below that only wants "a peeled layer" uses this.
    fn peel(g: &mut MapGraph, start: RoomId) -> Result<LayerId, MoveRefusal> {
        let region = planar_region(g, start);
        move_region(g, &region, MoveTarget::New)
    }

    #[test]
    fn planar_region_stops_at_portals() {
        let g = two_floors();
        let region = planar_region(&g, 3);
        let want: BTreeSet<RoomId> = [3, 4].into_iter().collect();
        assert_eq!(region.rooms, want, "down-portal cuts the cellar off from the upper floor");
        assert_eq!(region.anchor, 3, "and the region remembers the room it was walked from");
    }

    #[test]
    fn peel_region_moves_floor_to_new_layer() {
        let mut g = two_floors();
        let l = peel(&mut g, 3).expect("a proper sub-region peels");
        assert_eq!(g.layer_of(3), l);
        assert_eq!(g.layer_of(4), l);
        assert_eq!(g.layer_of(1), MAIN_LAYER);
        assert_eq!(g.layer_of(2), MAIN_LAYER);
        assert_eq!(g.layer_name(l), "Cellar");
    }

    // ── SQ-0360: peel at a named seam ────────────────────────────────────────

    /// A chain A-B-C-D with no portal in it: `peel_region` is powerless (one region), but naming
    /// the B→C passage cuts there. This is Zork's Cellar in miniature — 35 rooms of compass maze
    /// with no portal seam to find.
    #[test]
    fn naming_a_passage_cuts_a_layer_that_has_no_portal_seam() {
        let mut g = MapGraph::new();
        for (id, n) in [(1, "A"), (2, "B"), (3, "C"), (4, "D")] {
            g.upsert_room(id, n.into());
        }
        for (a, b) in [(1, 2), (2, 3), (3, 4)] {
            g.add_edge(a, Direction::E, b);
            g.add_edge(b, Direction::W, a);
        }
        assert_eq!(
            peel(&mut g, 1),
            Err(MoveRefusal::WholeLayer),
            "one connected region: the automatic peel has nothing to cut on"
        );

        // Peel from B, cutting east: B's OWN side leaves — the same side `planar_region` takes.
        let region = region_at_edge(&g, 2, Direction::E).expect("the B→C passage is a seam");
        let l = move_region(&mut g, &region, MoveTarget::New).expect("and a proper sub-region moves");
        assert_eq!(g.layers()[&l].parent, Some(MAIN_LAYER), "the new layer hangs off the one it left");
        assert_eq!(g.layer_name(l), "B", "named for the region's anchor, as a planar peel names for `start`");
        assert_eq!(g.rooms_in_layer(l), vec![1, 2], "B's side leaves, B included");
        assert_eq!(g.rooms_in_layer(MAIN_LAYER), vec![3, 4], "the far side stays put");

        // Standing at the other end of the SAME passage peels the other side — and each time it is
        // the side you are standing on, never a coin flip about which end you happened to pick.
        let mut g2 = MapGraph::new();
        for (id, n) in [(1, "A"), (2, "B"), (3, "C"), (4, "D")] {
            g2.upsert_room(id, n.into());
        }
        for (a, b) in [(1, 2), (2, 3), (3, 4)] {
            g2.add_edge(a, Direction::E, b);
            g2.add_edge(b, Direction::W, a);
        }
        let r2 = region_at_edge(&g2, 3, Direction::W).expect("same seam, other end");
        let l2 = move_region(&mut g2, &r2, MoveTarget::New).expect("moves");
        assert_eq!(g2.rooms_in_layer(l2), vec![3, 4], "C's side leaves, C included");
    }

    /// SQ-0364: peeling at a seam must leave the new layer a CHILD of the rooms it still connects
    /// to, so merging puts it back where its connections are.
    ///
    /// Taking the FAR side broke that: the room you peeled FROM stayed behind while its neighbours
    /// moved, making the new layer a sibling of everything it touches. Zork's case — peel east from
    /// a Maze room and the Maze stays on the Cellar while the other 24 rooms leave — then merged
    /// the Maze up to Main, a layer it has no connection to whatsoever.
    #[test]
    fn peeling_at_a_seam_then_merging_puts_the_rooms_back_where_they_were() {
        let mut g = MapGraph::new();
        for (id, n) in [(1, "Troll Room"), (2, "Maze"), (3, "Maze2")] {
            g.upsert_room(id, n.into());
        }
        for (a, b) in [(1, 2), (2, 3)] {
            g.add_edge(a, Direction::W, b);
            g.add_edge(b, Direction::E, a);
        }
        // Everything starts on a sub-layer, as Zork's underground does.
        let cellar = g.new_layer(Some(MAIN_LAYER), "Cellar".into());
        for id in [1, 2, 3] {
            g.set_room_layer(id, cellar);
        }

        // Peel the Maze by standing IN it and cutting the way back out.
        let region = region_at_edge(&g, 2, Direction::E).expect("seam");
        let maze = move_region(&mut g, &region, MoveTarget::New).expect("moves");
        assert_eq!(g.rooms_in_layer(maze), vec![2, 3], "the Maze leaves, both its rooms");
        assert_eq!(g.rooms_in_layer(cellar), vec![1], "the Troll Room stays");
        assert_eq!(
            g.layers()[&maze].parent,
            Some(cellar),
            "the Maze hangs off the Cellar — where the room it still connects to lives"
        );

        // So merging returns it to its connections, not to some layer it never touched.
        let whole = layer_region(&g, maze).expect("the maze layer has rooms");
        let parent = parent_layer(&g, maze);
        let back = move_region(&mut g, &whole, MoveTarget::Existing(parent))
            .expect("folding a whole layer into its parent is just a move");
        assert_eq!(back, cellar);
        assert_eq!(g.layer_of(2), cellar, "the Maze is back beside the Troll Room");
        assert_ne!(g.layer_of(2), MAIN_LAYER, "and never lands on Main, which it has no edge to");
    }

    /// The cut must take the passage's RECIPROCAL with it. Severing only `B -E-> C` would leave
    /// `C -W-> B` holding the halves together, and no seam could ever cut.
    #[test]
    fn cutting_a_passage_severs_its_back_edge_too() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1); // the reciprocal
        let region = region_at_edge(&g, 1, Direction::E).expect("a lone passage is a seam");
        let l = move_region(&mut g, &region, MoveTarget::New).expect("moves");
        assert_eq!(g.rooms_in_layer(l), vec![1], "A peels itself off; B stays");
    }

    // ── SQ-0552: peel at the passage walked IN through ───────────────────────

    /// Adventure's maze in miniature: entered SOUTH from the Long Hall, left by going DOWN.
    /// The way back is neither the reciprocal of the way in nor even a planar edge, so
    /// [`peel_at_edge`] on the current room cannot name the passage that was walked — its north
    /// exit is another maze room, and cutting that separates nothing. Naming the INCOMING edge
    /// cuts the real boundary, and still peels the side the player is standing on.
    #[test]
    fn a_one_way_entrance_peels_from_the_passage_walked_in_through() {
        let mut g = MapGraph::new();
        for (id, n) in [(1, "At West End of Long Hall"), (2, "Maze"), (3, "Maze"), (4, "Maze")] {
            g.upsert_room(id, n.into());
        }
        g.add_edge(1, Direction::S, 2); // walked in
        g.add_edge(2, Direction::Down, 1); // the way back — a portal, and not the reciprocal
        // Maze innards, cross-linked as a maze is: cutting any one of these leaves a way round.
        g.add_edge(2, Direction::N, 3);
        g.add_edge(3, Direction::S, 2);
        g.add_edge(2, Direction::E, 4);
        g.add_edge(4, Direction::N, 3);

        assert_eq!(
            region_at_edge(&g, 2, Direction::N).err(),
            Some(RegionRefusal::NotASeam),
            "cutting the current room's north exit cuts a maze-internal edge — the old behaviour"
        );

        let region = region_at_arrival(&g, 1, Direction::S).expect("the walked passage IS the boundary");
        let l = move_region(&mut g, &region, MoveTarget::New).expect("moves");
        assert_eq!(g.rooms_in_layer(l), vec![2, 3, 4], "the maze leaves — the side the player is on");
        assert_eq!(g.rooms_in_layer(MAIN_LAYER), vec![1], "the hall behind stays put");
        assert_eq!(g.layer_name(l), "Maze", "named for the room entered, not the one left");
        assert_eq!(
            g.layers()[&l].parent,
            Some(MAIN_LAYER),
            "and hangs off the layer it left, so merging puts it back beside the hall"
        );
    }

    /// SQ-0631: the cut is the named passage plus its exact reciprocal — `dest`'s back-direction
    /// edge to a THIRD room is not the reciprocal and must survive. With A-E->B, B-W->C, C-E->A,
    /// the old predicate also severed B-W->C when peeling (A, E); the region check then passed
    /// and {A, C} moved to a new layer while the surviving B-W->C edge straddled the seam.
    #[test]
    fn a_back_edge_to_a_third_room_is_not_the_reciprocal() {
        let mut g = MapGraph::new();
        for (id, n) in [(1, "A"), (2, "B"), (3, "C")] {
            g.upsert_room(id, n.into());
        }
        g.add_edge(1, Direction::E, 2); // the named passage
        g.add_edge(2, Direction::W, 3); // B's west edge — to C, NOT back to A
        g.add_edge(3, Direction::E, 1); // the way round that makes A-E-B a non-seam
        assert_eq!(
            region_at_edge(&g, 1, Direction::E).err(),
            Some(RegionRefusal::NotASeam),
            "cutting A-E->B alone separates nothing: B-W->C and C-E->A hold the ring together"
        );
        assert_eq!(g.layers().len(), 1, "and nothing was peeled");
        assert_eq!(g.rooms_in_layer(MAIN_LAYER), vec![1, 2, 3], "all three rooms stay put");
    }

    // ── SQ-0439: the inbound seams — what a move may legally cut ─────────────

    /// Adventure's maze again, but asked the other way round: which passages INTO the entrance
    /// room are boundaries at all? Exactly one, and it is the ONE-WAY passage that was walked —
    /// no direction out of the maze room names it, which is the entire complaint this answers.
    #[test]
    fn the_only_inbound_seam_at_a_maze_entrance_is_the_one_way_passage_in() {
        let mut g = MapGraph::new();
        for (id, n) in [(1, "At West End of Long Hall"), (2, "Maze"), (3, "Maze"), (4, "Maze")] {
            g.upsert_room(id, n.into());
        }
        g.add_edge(1, Direction::S, 2); // walked in — one way, no reciprocal
        g.add_edge(2, Direction::Down, 1); // the way back: a portal, and not the reciprocal
        g.add_edge(2, Direction::N, 3);
        g.add_edge(3, Direction::S, 2);
        g.add_edge(2, Direction::E, 4);
        g.add_edge(4, Direction::N, 3);

        let seams = inbound_seams(&g, 2);
        assert_eq!(seams.len(), 1, "the maze's innards hold each other together: {seams:?}");
        assert_eq!((seams[0].from, seams[0].dir), (1, Direction::S));
        assert_eq!(
            seams[0].region.rooms,
            [2, 3, 4].into_iter().collect::<BTreeSet<_>>(),
            "and the seam already knows what it would move — the maze, without the hall"
        );
        assert_eq!(seams[0].region.anchor, 2, "anchored on the room the seams were asked about");

        // The consequence the design accepts openly: a room in the MIDDLE of the maze has no
        // inbound boundary at all, because every way in has a way round.
        assert!(inbound_seams(&g, 3).is_empty(), "mid-maze: nothing to cut");
    }

    /// A corridor is the ambiguous case, and it is ambiguous for a real reason: standing in the
    /// middle of A-B-C-D, cutting the passage from A and cutting the passage from C are both
    /// legal boundaries that take opposite halves of the map.
    #[test]
    fn a_room_mid_corridor_has_one_inbound_seam_per_side() {
        let mut g = MapGraph::new();
        for (id, n) in [(1, "A"), (2, "B"), (3, "C"), (4, "D")] {
            g.upsert_room(id, n.into());
        }
        for (a, b) in [(1, 2), (2, 3), (3, 4)] {
            g.add_edge(a, Direction::E, b);
            g.add_edge(b, Direction::W, a);
        }
        let seams = inbound_seams(&g, 2);
        let named: Vec<(RoomId, Direction)> = seams.iter().map(|s| (s.from, s.dir)).collect();
        assert_eq!(named, vec![(1, Direction::E), (3, Direction::W)]);
        assert_eq!(seams[0].region.rooms, [2, 3, 4].into_iter().collect::<BTreeSet<_>>());
        assert_eq!(seams[1].region.rooms, [1, 2].into_iter().collect::<BTreeSet<_>>());
    }

    /// Neither a portal nor a passage from another layer is ever a candidate: a portal is already
    /// a cut ([`planar_region`]'s business), and a seam divides ONE layer rather than crossing one.
    /// Membership is decided by running the cut, so both fall out for free.
    #[test]
    fn portals_and_cross_layer_passages_are_never_inbound_seams() {
        let mut g = two_floors();
        let seams = inbound_seams(&g, 3);
        assert_eq!(
            seams.iter().map(|s| (s.from, s.dir)).collect::<Vec<_>>(),
            vec![(4, Direction::W)],
            "1↓3 is a portal, so it never appears — only the Wine Cellar's planar passage does"
        );

        peel(&mut g, 3).expect("peel the cellar off Main");
        assert_eq!(
            inbound_seams(&g, 1).iter().map(|s| (s.from, s.dir)).collect::<Vec<_>>(),
            vec![(2, Direction::W)],
            "3↑1 now crosses layers as well as being a portal, and is still not a seam"
        );
    }

    /// A passage whose ends stay connected another way is not a boundary, and says so.
    #[test]
    fn a_passage_with_a_way_round_is_not_a_seam() {
        // A→B directly, and A→C→B as well: cutting A-B separates nothing.
        let mut g = MapGraph::new();
        for (id, n) in [(1, "A"), (2, "B"), (3, "C")] {
            g.upsert_room(id, n.into());
        }
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        g.add_edge(1, Direction::N, 3);
        g.add_edge(3, Direction::E, 2);
        assert_eq!(region_at_edge(&g, 1, Direction::E).err(), Some(RegionRefusal::NotASeam));
        assert_eq!(g.layers().len(), 1, "and nothing was peeled");
    }

    #[test]
    fn a_named_seam_needs_a_planar_passage_inside_the_layer() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::Down, 2); // a portal: already a cut, so not this seam's job
        assert_eq!(region_at_edge(&g, 1, Direction::Down).err(), Some(RegionRefusal::NoSuchPassage));
        assert_eq!(
            region_at_edge(&g, 1, Direction::W).err(),
            Some(RegionRefusal::NoSuchPassage),
            "no such exit"
        );

        // A passage that already leaves the layer divides nothing within it.
        let mut g = MapGraph::new();
        for (id, n) in [(1, "A"), (2, "B"), (3, "C")] {
            g.upsert_room(id, n.into());
        }
        g.add_edge(1, Direction::Down, 2);
        g.add_edge(2, Direction::E, 3);
        let l = peel(&mut g, 2).expect("B/C peel off on the portal");
        assert_eq!(g.layer_of(2), l);
        assert_eq!(region_at_edge(&g, 1, Direction::Down).err(), Some(RegionRefusal::NoSuchPassage));
    }

    #[test]
    fn moving_a_whole_layer_to_a_new_one_is_refused() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        assert_eq!(
            peel(&mut g, 1),
            Err(MoveRefusal::WholeLayer),
            "region is the whole layer → a new layer would only rename it, and it says why"
        );
    }

    /// Fold a whole layer into `target` — the old `merge_layer_into`, now a whole-layer region
    /// moved onto an existing one.
    fn merge_into(g: &mut MapGraph, layer: LayerId, target: LayerId) -> Result<LayerId, MoveRefusal> {
        let region = layer_region(g, layer).expect("a layer with rooms in it");
        move_region(g, &region, MoveTarget::Existing(target))
    }

    // ── SQ-0439: peel and merge are one operation ────────────────────────────

    /// The behavioural heart of the unification: a WHOLE layer moved onto an EXISTING one is a
    /// merge, and must succeed where the old `WholeLayer` refusal blocked it. The refusal only
    /// ever meant "a new layer would be a no-op rename", which is untrue of a named target.
    #[test]
    fn a_whole_layer_moves_onto_an_existing_layer_but_never_onto_a_new_one() {
        let mut g = two_floors();
        let cellar = peel(&mut g, 3).expect("peel the cellar off Main"); // {3, 4}
        let whole = layer_region(&g, cellar).expect("the cellar has rooms");

        assert_eq!(
            move_region(&mut g, &whole, MoveTarget::New),
            Err(MoveRefusal::WholeLayer),
            "against a NEW layer the whole-layer region is still nothing but a rename"
        );
        let landed = move_region(&mut g, &whole, MoveTarget::Existing(MAIN_LAYER))
            .expect("but onto an existing layer it is simply a merge");
        assert_eq!(landed, MAIN_LAYER);
        assert_eq!(g.layer_of(3), MAIN_LAYER);
        assert_eq!(g.layer_of(4), MAIN_LAYER);
        assert!(!g.layers().contains_key(&cellar), "the emptied source layer's metadata goes");
    }

    /// The other half: `MainSource` generalised from "main cannot be a merge source" to "main
    /// cannot be EMPTIED". Moving a region out of Main was always legal and stays legal; moving
    /// every last room out of it is what the refusal is actually protecting against.
    #[test]
    fn main_may_be_moved_out_of_but_never_emptied() {
        let mut g = two_floors();
        let cellar = peel(&mut g, 3).expect("peel the cellar off Main"); // Main keeps {1, 2}

        // A sub-region of Main: legal, and it always was.
        let part = region_at_edge(&g, 1, Direction::E).expect("the 1→2 passage is a seam");
        assert_eq!(part.rooms, [1].into_iter().collect::<BTreeSet<_>>());
        move_region(&mut g, &part, MoveTarget::Existing(cellar)).expect("part of Main may leave");
        assert_eq!(g.layer_of(1), cellar);
        assert_eq!(g.rooms_in_layer(MAIN_LAYER), vec![2], "and Main still has a room");

        // All of what is left of Main: refused, because nothing may empty the floor layer.
        let rest = layer_region(&g, MAIN_LAYER).expect("Main has rooms");
        assert_eq!(
            move_region(&mut g, &rest, MoveTarget::Existing(cellar)),
            Err(MoveRefusal::EmptiesMain)
        );
        assert_eq!(g.layer_of(2), MAIN_LAYER, "a refused move moves nothing");
    }

    /// The granularity the unification buys: part of a layer may move onto another layer, and
    /// the source survives with the rooms that stayed. Neither old verb could express this —
    /// peel only ever minted, merge only ever took the whole layer.
    #[test]
    fn a_partial_region_may_move_onto_an_existing_layer_and_the_source_survives() {
        let mut g = two_floors();
        let cellar = peel(&mut g, 3).expect("peel the cellar off Main"); // {3, 4}
        let just_the_wine = region_at_edge(&g, 4, Direction::W).expect("the 3↔4 passage is a seam");

        move_region(&mut g, &just_the_wine, MoveTarget::Existing(MAIN_LAYER)).expect("one room moves");
        assert_eq!(g.layer_of(4), MAIN_LAYER, "the named room went home");
        assert_eq!(g.rooms_in_layer(cellar), vec![3], "and the cellar kept the rest");
        assert!(g.layers().contains_key(&cellar), "a source that still has rooms keeps its metadata");
    }

    /// SQ-0631: folding a layer whose recorded parent was itself merged away must not re-home
    /// rooms onto the dead parent id — layer ids are never reused, so those rooms would be
    /// invisible to every view forever. Peel L1 from Main, peel L2 off it, fold L1, fold L2:
    /// L2's rooms must land on a layer that still exists (Main), not on the ghost of L1.
    #[test]
    fn folding_onto_a_dead_parent_falls_back_to_a_live_layer() {
        let mut g = two_floors();
        let l1 = peel(&mut g, 3).expect("peel the cellar off Main"); // {3, 4}
        let r = region_at_edge(&g, 3, Direction::E).expect("the cellar's own seam");
        let l2 = move_region(&mut g, &r, MoveTarget::New).expect("peel room 3 off the cellar");
        assert_eq!(g.layers()[&l2].parent, Some(l1));

        let p1 = parent_layer(&g, l1);
        let t1 = merge_into(&mut g, l1, p1).expect("fold l1 into its parent");
        assert_eq!(t1, MAIN_LAYER);
        assert!(!g.layers().contains_key(&l1), "l1 is emptied and gone — l2's parent is now dead");

        let p2 = parent_layer(&g, l2);
        assert_eq!(p2, MAIN_LAYER, "the dead parent is skipped in favour of Main");
        let t2 = merge_into(&mut g, l2, p2).expect("fold l2");
        assert!(g.layers().contains_key(&t2), "the target must be a layer that exists");
        assert_eq!(g.layer_of(3), MAIN_LAYER, "room 3 is visible on Main, not stranded on dead l1");
    }

    #[test]
    fn folding_main_into_its_own_parent_refuses() {
        let mut g = two_floors();
        assert_eq!(parent_layer(&g, MAIN_LAYER), MAIN_LAYER, "Main has no parent but itself");
        assert_eq!(merge_into(&mut g, MAIN_LAYER, MAIN_LAYER), Err(MoveRefusal::SelfMove));
        assert!(g.layers().contains_key(&MAIN_LAYER));
    }

    /// SQ-0687: the stranded-room story, end to end. A room discovered while exploring a maze
    /// layer inherits the maze layer even when it belongs to the surface; the player peels the
    /// stranded region off the maze, then folds it into a NAMED target — not the peel's parent,
    /// which would round-trip it straight back into the maze.
    #[test]
    fn a_peeled_region_moves_onto_a_named_target_not_its_parent() {
        let mut g = two_floors();
        let maze = peel(&mut g, 3).expect("peel the cellar off Main"); // {3, 4}
        // Room 4 is the "back door to the surface" discovered from the maze: peel it off.
        let r = region_at_edge(&g, 4, Direction::W).expect("the stranded room's seam");
        let stranded = move_region(&mut g, &r, MoveTarget::New).expect("peel the stranded room");
        assert_eq!(g.layers()[&stranded].parent, Some(maze), "the peel parents onto the maze");

        merge_into(&mut g, stranded, MAIN_LAYER).expect("fold into Main by name");
        assert_eq!(g.layer_of(4), MAIN_LAYER, "the stranded room lands on Main, not back in the maze");
        assert_eq!(g.layer_of(3), maze, "the maze keeps its own rooms");
        assert!(!g.layers().contains_key(&stranded), "the emptied layer's metadata is removed");
    }

    #[test]
    fn moving_onto_the_current_or_a_dead_layer_refuses() {
        let mut g = two_floors();
        let l = peel(&mut g, 3).unwrap();
        assert_eq!(merge_into(&mut g, l, l), Err(MoveRefusal::SelfMove));
        let dead = g.next_layer_id(); // never created
        assert_eq!(merge_into(&mut g, l, dead), Err(MoveRefusal::NoSuchLayer));
        assert_eq!(g.layer_of(3), l, "a refused move moves nothing");
    }

    /// Two layers laid out independently have no reason to interleave cleanly: a room arriving on
    /// a cell already taken in the target must land on the nearest free one, never on top of an
    /// existing room — two rooms on one cell draw as one. This is the ONE asymmetry between the
    /// two targets: a freshly minted layer has nothing to collide with.
    #[test]
    fn moving_onto_an_existing_layer_resolves_cell_collisions_instead_of_stacking() {
        let mut g = two_floors();
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        let l = peel(&mut g, 3).unwrap();
        g.set_pos(3, (0, 0)); // same cell as room 1 on Main
        g.set_pos(4, (5, 5)); // free on Main
        merge_into(&mut g, l, MAIN_LAYER).expect("fold into Main");
        let pos = |id| g.room(id).and_then(|r| r.pos).unwrap();
        assert_eq!(pos(4), (5, 5), "a free cell is kept as-is");
        assert_ne!(pos(3), (0, 0), "the colliding room moved off the occupied cell");
        let mut cells: Vec<_> = [1, 2, 3, 4].iter().map(|&id| pos(id)).collect();
        cells.sort();
        cells.dedup();
        assert_eq!(cells.len(), 4, "no two rooms share a cell after the move");
    }

    /// A move into a FRESH layer leaves positions exactly alone — there is nothing there to
    /// collide with, so reconciling would only shuffle rooms for no reason.
    #[test]
    fn moving_onto_a_new_layer_never_touches_positions() {
        let mut g = two_floors();
        g.set_pos(1, (0, 0));
        g.set_pos(3, (0, 0)); // deliberately the same cell as room 1
        g.set_pos(4, (1, 0));
        peel(&mut g, 3).expect("peel the cellar");
        let pos = |id| g.room(id).and_then(|r| r.pos).unwrap();
        assert_eq!(pos(3), (0, 0), "the peeled room keeps its cell");
        assert_eq!(pos(4), (1, 0));
    }

    /// Neither direction destroys topology: the passage a peel "severs" is only a predicate for
    /// the region walk, so it survives as a cross-layer connection — which is what makes a move
    /// reversible by another move (SQ-0439).
    #[test]
    fn a_cut_seam_survives_as_a_cross_layer_connection() {
        let mut g = two_floors();
        let before = g.connections().len();
        let r = region_at_edge(&g, 1, Direction::E).expect("the 1→2 passage is a seam");
        let l = move_region(&mut g, &r, MoveTarget::New).expect("peel");
        assert_eq!(g.connections().len(), before, "no connection was deleted");
        let seam = g.connections().iter().find(|c| c.origin == 1 && c.dir == Direction::E);
        assert!(seam.is_some_and(|c| is_interlayer(&g, c)), "the cut passage now crosses layers");

        // And it goes back: fold the peeled layer home and the map is whole again.
        merge_into(&mut g, l, MAIN_LAYER).expect("fold back");
        assert_eq!(g.rooms_in_layer(MAIN_LAYER), vec![1, 2, 3, 4]);
    }
}
