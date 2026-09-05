//! Orthogonal edge router: turns directed connections into routed connector polylines.
//!
//! # Fine grid
//!
//! Each room at logical cell `(c, r)` occupies fine cell `(2c, 2r)`. Odd fine cells are gutters.
//! This doubles the resolution and gives us gutter lanes between rooms for routing.
//!
//! # Routing algorithm (L/Z router, ≤2 bends)
//!
//! For each compass-direction connection:
//! 1. Compute `departure` = origin room's fine cell offset by 1 in the departure direction.
//! 2. Compute `arrival` = dest room's fine cell offset by 1 in the *opposite* side's direction.
//! 3. If departure and arrival share the same axis, emit a straight 2-point segment (0 bends).
//! 4. Otherwise build two candidate L-bends: horizontal-first and vertical-first. For each,
//!    the interior corner must not collide with any room's fine cell. Pick the first non-colliding
//!    candidate. If both collide, set `routing_failed = true` and emit a 2-point direct segment.
//!
//! # Diagonal directions (NE, SE, NW, SW)
//!
//! Diagonal connections depart the side determined by the dominant axis:
//! - NE → Top (north component wins over east)
//! - SE → Bottom
//! - NW → Top
//! - SW → Bottom
//!
//! This is a conservative choice: the north/south axis takes precedence to match conventional
//! adventure-game map conventions (vertical flow dominates). The routing may look slightly
//! non-intuitive for diagonal edges but remains geometrically consistent.
//!
//! # Reciprocal dedupe
//!
//! An edge `(o, d, dst)` is skipped only if its exact reciprocal-opposite `(dst, opposite(d), o)`
//! was already emitted. This means non-reciprocal back-edges (e.g. A→N→B and B→W→A where W ≠ S)
//! are both kept. Only true opposite pairs (e.g. A→E→B and B→W→A) are deduped.
//! The kept edge is whichever of the true-reciprocal pair is emitted first (by `connections()`
//! order); both are geometrically equivalent so render output is unaffected.
//! Stubs (Up/Down/In/Out/Unknown) are never deduped.
//!
//! # Crossings are fine; OVERLAPS are not
//!
//! The rule the drawn map is held to is the user's, verbatim: *"crossings are okay, overlaps
//! need to be avoided."* Two connectors meeting perpendicular at a point is a crossing — the
//! terminal breaks the horizontal for one cell and both lines stay followable. Two connectors
//! running ALONG each other for any length is an overlap, and one of the two passages simply
//! disappears under the other.
//!
//! This module's `route_all` is the STUB router: one polyline per connection, first
//! non-colliding L, no lanes and no notion of what anything else is doing. Nothing here can
//! honour that rule, and nothing here is asked to — the drawn map is routed by
//! [`crate::route::route_lanes`], and that is where the cost model lives:
//!
//! * **`route_topology_with` chooses a route by cost, not by first fit.** Each connector is
//!   offered both L orientations and (for a one-way) the entry sides still facing its origin.
//!   A candidate that would run alongside an already-placed connector scores an overlap, and
//!   overlaps are the PRIMARY key — an overlap-free route always wins, however many crossings
//!   it costs. Crossings are the secondary key, then a fixed preference rank, then the points
//!   themselves, so the choice is deterministic.
//! * **`assign_lanes` then separates what shares a channel**, and its cost is the lane index:
//!   a busy channel simply widens (`render::map::channel_width` grows with the lane count)
//!   rather than stacking two lines on one. Its ordering is a hard constraint, not a
//!   preference — see [`crate::route::Claim`] for why a connector occupies more of a channel
//!   than its own lane, and what happens when two of them bridge in from opposite sides.
//!
//! `crate::route::plan_overlaps` states the invariant on the finished plan, renderer-
//! independently; `render::map::overlap_cells` states it again on the drawn cells. Both are
//! zero on the Zork I map (SQ-1316).
//!
//! Full crossing MINIMISATION — a Sugiyama-style global reduction — is still not implemented,
//! and is a much weaker want: a crossing is legible, so the greedy per-connector reduction
//! `route_topology` already does is enough.

use std::collections::HashSet;

use crate::direction::{opposite, Direction};
use crate::graph::{MapGraph, RoomId};

// ── Side ─────────────────────────────────────────────────────────────────────

/// The side of a room cell from which a connector departs or arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Side {
    Top,
    Bottom,
    Left,
    Right,
}

/// Map a `Direction` to the room side the connector departs from.
///
/// Cardinal compass:
/// - N → Top, S → Bottom, E → Right, W → Left
///
/// Diagonal — EAST/WEST axis dominates (SQ-0314):
/// - NE → Right, SE → Right, NW → Left, SW → Left
///
/// A diagonal anchors on the box CORNER (`corner_anchor`), not a side midpoint,
/// and leaves it diagonally. The east/west choice is what makes the rest of the
/// route agree with that corner: `attach_bridge` runs its perpendicular leg along
/// the anchor's own ROW, i.e. straight out from the corner in the direction the
/// exit actually points. The previous north/south collapse sent an NE connector's
/// leg up the room's CENTRE column, so the line left the top-right corner and
/// immediately doubled back west — a contradiction the corner arrow hid and a
/// diagonal stub makes plainly visible. It is also CONSISTENT with
/// `oneway_entry_side`, which routes an NE arrival to **Left**
/// (`route::oneway_entry_side` returns `side_for(opposite(dir))` for a diagonal:
/// NE→Left, NW→Right, SE→Right, SW→Left). A connector leaves the origin's right
/// and arrives on the destination's left, which is the same geometry from both
/// ends.
///
/// This sentence used to cite that function as routing NE to *Right*, and read as
/// settling an inconsistency (SQ-1065). It was true when written and stopped being
/// true seven hours later the same morning, when `oneway_entry_side` was replaced
/// and the comment was not — so a later reader could have "restored consistency"
/// against an inverted citation.
///
/// Non-planar (Up, Down, In, Out, Unknown) → `None` (rendered as stubs).
pub fn side_for(dir: Direction) -> Option<Side> {
    match dir {
        Direction::N => Some(Side::Top),
        Direction::S => Some(Side::Bottom),
        Direction::E | Direction::NE | Direction::SE => Some(Side::Right),
        Direction::W | Direction::NW | Direction::SW => Some(Side::Left),
        Direction::Up | Direction::Down | Direction::In | Direction::Out | Direction::Unknown => {
            None
        }
    }
}

/// Like [`side_for`], but also gives Up/Down a routed box side (Up→Top, Down→Bottom).
/// Used ONLY by the lane router so vertical connectors get lanes + border anchors;
/// the old stub router (`route_all`) keeps using `side_for` (None for up/down).
pub fn route_side(dir: Direction) -> Option<Side> {
    match dir {
        Direction::Up => Some(Side::Top),
        Direction::Down => Some(Side::Bottom),
        _ => side_for(dir),
    }
}

// ── Fine grid helpers ─────────────────────────────────────────────────────────

/// Convert a logical room position to its fine-grid cell.
/// Room cells occupy even fine coordinates; odd fine coordinates are gutter lanes.
pub fn fine_cell(pos: (i32, i32)) -> (i32, i32) {
    (2 * pos.0, 2 * pos.1)
}

/// Return the fine-grid departure point: origin fine cell offset by 1 step outward on `side`.
fn departure_point(origin_fine: (i32, i32), side: Side) -> (i32, i32) {
    match side {
        Side::Top => (origin_fine.0, origin_fine.1 - 1),
        Side::Bottom => (origin_fine.0, origin_fine.1 + 1),
        Side::Left => (origin_fine.0 - 1, origin_fine.1),
        Side::Right => (origin_fine.0 + 1, origin_fine.1),
    }
}

/// Return the fine-grid arrival point: dest fine cell offset by 1 step outward on the arrival
/// side (the opposite of the connection's departure side).
fn arrival_point(dest_fine: (i32, i32), departure_side: Side) -> (i32, i32) {
    // The arrival side is opposite to the departure side.
    let arrival_side = opposite_side(departure_side);
    match arrival_side {
        Side::Top => (dest_fine.0, dest_fine.1 - 1),
        Side::Bottom => (dest_fine.0, dest_fine.1 + 1),
        Side::Left => (dest_fine.0 - 1, dest_fine.1),
        Side::Right => (dest_fine.0 + 1, dest_fine.1),
    }
}

fn opposite_side(s: Side) -> Side {
    match s {
        Side::Top => Side::Bottom,
        Side::Bottom => Side::Top,
        Side::Left => Side::Right,
        Side::Right => Side::Left,
    }
}

// ── RoutedEdge ────────────────────────────────────────────────────────────────

/// A routed connector polyline for one directed connection.
#[derive(Debug, Clone)]
pub struct RoutedEdge {
    pub origin: RoomId,
    pub dest: RoomId,
    pub dir: Direction,
    /// Polyline in fine-grid coordinates. Starts at the departure point off the origin's
    /// departure side and ends at the arrival point into the dest's arrival side.
    pub points: Vec<(i32, i32)>,
    /// True if the layout engine marked the connection distorted OR routing could not find
    /// a bend orientation that avoids all room fine cells.
    pub distorted: bool,
    /// True for Up/Down/In/Out/Unknown connections that have no planar route.
    pub is_stub: bool,
    /// Short label for stub connectors ("U", "D", "IN", "OUT", "?").
    pub label: Option<String>,
    /// The side of the destination room this connection touches, if known.
    ///
    /// This is discovered from a reverse edge `(dest, dir2, origin)`: if such an edge exists
    /// in the graph, `arrival_dir = Some(dir2)`. This is the direction the player travels
    /// when leaving `dest` back toward `origin`, which tells us which side of `dest` the
    /// connection touches.
    ///
    /// `None` means no reverse edge has been observed — the arrival side is undiscovered.
    /// We do NOT assume `opposite(self.dir)` because connections are not assumed reciprocal.
    ///
    /// Stubs (is_stub = true) always have `arrival_dir = None`.
    pub arrival_dir: Option<Direction>,
    /// Stubs only: the display name of the target room (`dest`), resolved from the graph so
    /// the renderer can label the badge without re-resolving it (and so the target may live
    /// off the current layer in future). `None` for routed compass edges.
    pub dest_label: Option<String>,
    /// True when `dest` lives on a DIFFERENT layer than `origin` (SQ-0223).
    ///
    /// Set only by `interlayer_badges`; every route within one layer is `false`. The renderer
    /// needs this to tell a cross-layer badge from an ordinary stub, and it cannot infer it —
    /// `dest` is simply absent from the layer's rooms, which is indistinguishable from a room
    /// that has no position yet.
    pub is_interlayer: bool,
}

// ── route_all ─────────────────────────────────────────────────────────────────

/// Route all connections in `graph` and return a `RoutedEdge` for each.
///
/// Connections whose endpoints are not both placed (pos = None) are skipped.
/// Reciprocal-opposite pairs are deduped: an edge `(o, d, dst)` is skipped only if its
/// true reciprocal-opposite `(dst, opposite(d), o)` was already emitted.
pub fn route_all(graph: &MapGraph) -> Vec<RoutedEdge> {
    // Build the set of room fine cells for collision checking.
    let room_fine_cells: HashSet<(i32, i32)> = graph
        .rooms()
        .filter_map(|r| r.pos.map(fine_cell))
        .collect();

    // Track emitted directed edges as (origin, dir, dest) triples.
    // An edge is skipped only when its exact reciprocal-opposite was already emitted.
    let mut emitted: HashSet<(RoomId, Direction, RoomId)> = HashSet::new();

    let mut result = Vec::new();

    for conn in graph.connections() {
        // A self-loop is not a route between two places — the drawn view shows it as a badge
        // on the room box, never a connector looping out and back (SQ-0666).
        if conn.is_self_loop() {
            continue;
        }
        let origin_pos = match graph.room(conn.origin).and_then(|r| r.pos) {
            Some(p) => p,
            None => continue, // origin not placed — skip
        };
        let dest_pos = match graph.room(conn.dest).and_then(|r| r.pos) {
            Some(p) => p,
            None => continue, // dest not placed — skip
        };

        // Stubs are never deduped — always emit them.
        if side_for(conn.dir).is_none() {
            let label = Some(stub_label(conn.dir).to_string());
            let origin_fine = fine_cell(origin_pos);
            // Short stub: emit 1-segment pointing upward in fine coords as a visual indicator.
            let stub_end = (origin_fine.0, origin_fine.1 - 1);
            result.push(RoutedEdge {
                origin: conn.origin,
                dest: conn.dest,
                dir: conn.dir,
                points: vec![origin_fine, stub_end],
                distorted: conn.distorted,
                is_stub: true,
                label,
                arrival_dir: None,
                dest_label: graph.room(conn.dest).map(|r| r.label().to_string()),
                is_interlayer: false,
            });
            continue;
        }

        let side = side_for(conn.dir).unwrap();

        // Reciprocal dedupe: skip this edge only if its true reciprocal-opposite
        // (dest → opposite(dir) → origin) was already emitted.
        let partner = (conn.dest, opposite(conn.dir), conn.origin);
        if emitted.contains(&partner) {
            continue;
        }

        let origin_fine = fine_cell(origin_pos);
        let dest_fine = fine_cell(dest_pos);

        let dep = departure_point(origin_fine, side);
        let arr = arrival_point(dest_fine, side);

        // Build the polyline.
        let (points, routing_failed) = build_path(dep, arr, &room_fine_cells);

        let distorted = conn.distorted || routing_failed;

        // Discover arrival_dir from the reverse edge (dest → ? → origin), if it exists.
        // We do NOT assume opposite(dir) because connections are not assumed reciprocal.
        let arrival_dir = graph
            .connections()
            .iter()
            .find(|c| c.origin == conn.dest && c.dest == conn.origin)
            .map(|c| c.dir);

        result.push(RoutedEdge {
            origin: conn.origin,
            dest: conn.dest,
            dir: conn.dir,
            points,
            distorted,
            is_stub: false,
            label: None,
            arrival_dir,
            dest_label: None,
                is_interlayer: false,
        });

        emitted.insert((conn.origin, conn.dir, conn.dest));
    }

    result
}

/// Build an orthogonal polyline from `dep` to `arr` with ≤2 bends.
///
/// Returns `(points, routing_failed)`. `routing_failed` is true if all bend orientations
/// had interior corners colliding with room fine cells and we fell back to a direct 2-point
/// segment.
fn build_path(
    dep: (i32, i32),
    arr: (i32, i32),
    room_cells: &HashSet<(i32, i32)>,
) -> (Vec<(i32, i32)>, bool) {
    // Straight segment (already aligned on one axis).
    if dep.0 == arr.0 || dep.1 == arr.1 {
        return (vec![dep, arr], false);
    }

    // Candidate 1: horizontal-first bend — corner at (arr.0, dep.1).
    let corner_h = (arr.0, dep.1);
    // Candidate 2: vertical-first bend — corner at (dep.0, arr.1).
    let corner_v = (dep.0, arr.1);

    if !room_cells.contains(&corner_h) {
        return (vec![dep, corner_h, arr], false);
    }
    if !room_cells.contains(&corner_v) {
        return (vec![dep, corner_v, arr], false);
    }

    // Both collide → fallback direct segment, mark routing failed.
    (vec![dep, arr], true)
}

/// Short label string for a stub direction.
pub(crate) fn stub_label(dir: Direction) -> &'static str {
    match dir {
        Direction::Up => "U",
        Direction::Down => "D",
        Direction::In => "IN",
        Direction::Out => "OUT",
        Direction::Unknown => "?",
        _ => "?",
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use crate::graph::MapGraph;
    use crate::layout::relayout_auto;

    #[test]
    fn straight_connector_between_adjacent_rooms() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::E, 2);
        relayout_auto(&mut g);
        let edges = route_all(&g);
        let e = edges.iter().find(|e| e.origin == 1 && e.dest == 2).unwrap();
        assert!(!e.is_stub);
        // departs the RIGHT side of room 1 (east): first point's x == origin fine x + 1
        let o = fine_cell(g.room(1).unwrap().pos.unwrap());
        assert_eq!(e.points.first().unwrap().0, o.0 + 1);
    }

    #[test]
    fn up_edge_is_a_stub_with_label() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::Up, 2);
        relayout_auto(&mut g);
        let edges = route_all(&g);
        let e = edges.iter().find(|e| e.origin == 1).unwrap();
        assert!(e.is_stub);
        assert_eq!(e.label.as_deref(), Some("U"));
    }

    #[test]
    fn side_for_cardinals() {
        assert_eq!(side_for(Direction::N), Some(Side::Top));
        assert_eq!(side_for(Direction::S), Some(Side::Bottom));
        assert_eq!(side_for(Direction::E), Some(Side::Right));
        assert_eq!(side_for(Direction::W), Some(Side::Left));
    }

    #[test]
    fn side_for_diagonals_east_west_axis_wins() {
        // SQ-0314: a diagonal anchors on the box CORNER and leaves it diagonally, so
        // its side must be the one that agrees with that corner — attach_bridge then
        // runs its leg along the anchor's own row, straight out from the corner.
        // The old north/south collapse sent an NE connector up the room's centre
        // column, so the line left the top-right corner and doubled straight back
        // west. This also matches oneway_entry_side, which already enters NE at Right.
        assert_eq!(side_for(Direction::NE), Some(Side::Right));
        assert_eq!(side_for(Direction::SE), Some(Side::Right));
        assert_eq!(side_for(Direction::NW), Some(Side::Left));
        assert_eq!(side_for(Direction::SW), Some(Side::Left));
        // Cardinals are untouched.
        assert_eq!(side_for(Direction::N), Some(Side::Top));
        assert_eq!(side_for(Direction::S), Some(Side::Bottom));
    }

    #[test]
    fn side_for_non_planar_returns_none() {
        assert_eq!(side_for(Direction::Up), None);
        assert_eq!(side_for(Direction::Down), None);
        assert_eq!(side_for(Direction::In), None);
        assert_eq!(side_for(Direction::Out), None);
        assert_eq!(side_for(Direction::Unknown), None);
    }

    #[test]
    fn fine_cell_doubles_coords() {
        assert_eq!(fine_cell((0, 0)), (0, 0));
        assert_eq!(fine_cell((1, 2)), (2, 4));
        assert_eq!(fine_cell((-1, 3)), (-2, 6));
    }

    #[test]
    fn reciprocal_edge_deduped() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1); // reciprocal
        relayout_auto(&mut g);
        let edges = route_all(&g);
        // Only one compass edge for the pair (1, 2) — the lower-origin-id one (origin=1).
        let count = edges.iter().filter(|e| !e.is_stub).count();
        assert_eq!(count, 1, "reciprocal pair should produce exactly one routed edge");
        let kept = edges.iter().find(|e| !e.is_stub).unwrap();
        assert_eq!(kept.origin, 1, "kept edge should have lower origin id");
    }

    #[test]
    fn north_edge_departs_top() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::N, 2);
        relayout_auto(&mut g);
        let edges = route_all(&g);
        let e = edges.iter().find(|e| e.origin == 1 && e.dest == 2).unwrap();
        assert!(!e.is_stub);
        let o = fine_cell(g.room(1).unwrap().pos.unwrap());
        // Departure from Top side: first point's y == origin fine y - 1
        assert_eq!(e.points.first().unwrap().1, o.1 - 1);
    }

    #[test]
    fn distorted_flag_propagated_from_layout() {
        // Impossible northward loop: 1→N→2→N→3→N→1. At least one edge must be distorted
        // (the loop can't be Euclidean). Verify that the distorted flag is carried through
        // to the routed edges.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.upsert_room(3, "C".into());
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::N, 3);
        g.add_edge(3, Direction::N, 1); // closes impossible loop
        relayout_auto(&mut g);
        let edges = route_all(&g);
        assert!(
            edges.iter().any(|e| e.distorted),
            "layout-flagged distortion should be carried through to routed edges"
        );
    }

    #[test]
    fn unplaced_rooms_skipped() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::E, 2);
        // Do NOT call relayout_auto — rooms have no pos.
        let edges = route_all(&g);
        assert!(edges.is_empty(), "connections with unplaced endpoints must be skipped");
    }

    #[test]
    fn stub_labels_for_all_non_planar() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.upsert_room(3, "C".into());
        g.upsert_room(4, "D".into());
        g.upsert_room(5, "E".into());
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(1, Direction::Down, 3);
        g.add_edge(1, Direction::In, 4);
        g.add_edge(1, Direction::Out, 5);
        relayout_auto(&mut g);
        let edges = route_all(&g);
        let find_label = |d: Direction| {
            edges
                .iter()
                .find(|e| e.dir == d)
                .and_then(|e| e.label.as_deref().map(|s| s.to_string()))
        };
        assert_eq!(find_label(Direction::Up).as_deref(), Some("U"));
        assert_eq!(find_label(Direction::Down).as_deref(), Some("D"));
        assert_eq!(find_label(Direction::In).as_deref(), Some("IN"));
        assert_eq!(find_label(Direction::Out).as_deref(), Some("OUT"));
    }

    #[test]
    fn unknown_stub_label() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::Unknown, 2);
        relayout_auto(&mut g);
        let edges = route_all(&g);
        let e = edges.iter().find(|e| e.dir == Direction::Unknown).unwrap();
        assert!(e.is_stub);
        assert_eq!(e.label.as_deref(), Some("?"));
    }

    #[test]
    fn non_reciprocal_back_edge_both_kept() {
        // A(1) →N→ B(2) and B(2) →W→ A(1).
        // N's opposite is S, not W — these are NOT reciprocal-opposites; both must be kept.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::W, 1);
        relayout_auto(&mut g);
        let edges = route_all(&g);
        let has_n = edges
            .iter()
            .any(|e| e.origin == 1 && e.dir == Direction::N && !e.is_stub);
        let has_w = edges
            .iter()
            .any(|e| e.origin == 2 && e.dir == Direction::W && !e.is_stub);
        assert!(has_n, "A→N→B must be present");
        assert!(has_w, "B→W→A must be present (non-reciprocal back-edge)");
    }

    #[test]
    fn multi_edge_same_pair_both_kept() {
        // A→E→B and A→W→B: same pair, different directions, neither is the other's partner.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::E, 2);
        g.add_edge(1, Direction::W, 2);
        relayout_auto(&mut g);
        let edges = route_all(&g);
        let has_e = edges
            .iter()
            .any(|e| e.origin == 1 && e.dir == Direction::E && !e.is_stub);
        let has_w = edges
            .iter()
            .any(|e| e.origin == 1 && e.dir == Direction::W && !e.is_stub);
        assert!(has_e, "A→E→B must be present");
        assert!(has_w, "A→W→B must be present (multi-edge, not a reciprocal)");
    }

    #[test]
    fn north_edge_departs_top_x_unchanged() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::N, 2);
        relayout_auto(&mut g);
        let edges = route_all(&g);
        let e = edges.iter().find(|e| e.origin == 1 && e.dest == 2).unwrap();
        assert!(!e.is_stub);
        let o = fine_cell(g.room(1).unwrap().pos.unwrap());
        // Departure from Top: y decreases by 1, x is unchanged.
        assert_eq!(e.points.first().unwrap().1, o.1 - 1, "y should be origin_fine.y - 1");
        assert_eq!(e.points.first().unwrap().0, o.0, "x should equal origin_fine.x for Top departure");
    }

    #[test]
    fn arrival_dir_known_when_reverse_exists() {
        // A(1) →N→ B(2) and B(2) →W→ A(1): non-reciprocal pair.
        // The A→N→B edge should have arrival_dir = Some(W) (the discovered side of B),
        // NOT Some(S) which would be the assumed opposite of N.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::W, 1);
        relayout_auto(&mut g);
        let edges = route_all(&g);
        let e = edges
            .iter()
            .find(|e| e.origin == 1 && e.dir == Direction::N)
            .expect("A→N→B edge must be present");
        assert_eq!(
            e.arrival_dir,
            Some(Direction::W),
            "arrival_dir for A→N→B should be Some(W) (the discovered reverse direction), not Some(S)"
        );
    }

    #[test]
    fn arrival_dir_none_when_no_reverse() {
        // Only A(1) →N→ B(2), no reverse edge.
        // The arrival side of B is undiscovered, so arrival_dir must be None.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::N, 2);
        relayout_auto(&mut g);
        let edges = route_all(&g);
        let e = edges
            .iter()
            .find(|e| e.origin == 1 && e.dir == Direction::N)
            .expect("A→N→B edge must be present");
        assert_eq!(
            e.arrival_dir,
            None,
            "arrival_dir should be None when no reverse edge exists"
        );
    }

    #[test]
    fn arrival_dir_reciprocal_opposite() {
        // A(1) →N→ B(2) and B(2) →S→ A(1): true reciprocal-opposite pair.
        // The pair is deduped to one edge. The surviving edge has arrival_dir = Some(S).
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1);
        relayout_auto(&mut g);
        let edges = route_all(&g);
        // Only one non-stub edge should exist (deduped).
        let non_stubs: Vec<_> = edges.iter().filter(|e| !e.is_stub).collect();
        assert_eq!(non_stubs.len(), 1, "reciprocal-opposite pair should be deduped to one edge");
        let e = non_stubs[0];
        assert_eq!(
            e.arrival_dir,
            Some(Direction::S),
            "arrival_dir for the surviving A→N→B edge should be Some(S) (the discovered reverse)"
        );
    }

    #[test]
    fn stub_edge_carries_dest_label() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Cellar".into());
        g.upsert_room(2, "Attic".into());
        g.add_edge(1, Direction::Up, 2);
        relayout_auto(&mut g);
        let edges = route_all(&g);
        let e = edges.iter().find(|e| e.origin == 1 && e.is_stub).unwrap();
        assert_eq!(
            e.dest_label.as_deref(),
            Some("Attic"),
            "a portal stub must carry its target room's name"
        );
    }

    #[test]
    fn compass_edge_has_no_dest_label() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::E, 2);
        relayout_auto(&mut g);
        let edges = route_all(&g);
        let e = edges.iter().find(|e| e.origin == 1 && !e.is_stub).unwrap();
        assert_eq!(e.dest_label, None, "routed compass edges carry no dest_label");
    }
}
