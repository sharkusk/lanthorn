//! Seating a LATE-ARRIVING room beside the room it hangs off (SQ-1356).
//!
//! Two callers, one question. A cross-layer **ghost** (`crate::render::render_layer`) stands for
//! a room on another layer and is placed after every real room on this one; a **portal-only
//! room** — a room whose every passage is Up/Down/In/Out, Zork I's `Attic` and `Cellar` being
//! the specimens — has no compass bearing for the stress solve to seat it by, so it too arrives
//! after everything with a bearing. Both want the same thing: the cell adjacent to their anchor
//! in the passage's own direction, and a map that OPENS to make room for it rather than parking
//! them wherever a free cell happened to be left over.
//!
//! The gap is opened the way `open_gated_holes_for_hubs` opens one — by sliding a whole side of
//! the map away by a cell — but along a LINE rather than along a run: every room at or beyond
//! the wanted cell's row (or column) moves one cell further out, which inserts a blank line
//! there. Every pair of rooms on the same side of that cut keeps its exact offset, so the only
//! links that can stretch are the ones that STRADDLE the cut.
//!
//! **And a straddling CARDINAL RECIPROCAL is what vetoes the whole shift.** "Exactly one cell
//! apart" is what a reciprocal pair means (see the module docs on `layout`), so a line whose
//! opening would pull one apart is not opened at all: the newcomer falls back to the nearest
//! free cell along its own bearing and the router draws the bent line. A passage that is
//! one-way, diagonal, gated or already stretched may lengthen — none of those claims a cell
//! count.

use std::collections::{BTreeMap, BTreeSet};

use crate::direction::{grid_offset, layout_offset, Direction};
use crate::graph::{MapGraph, RoomId};

/// The grid step a passage travelling `dir` wants its far end to sit at, for SEATING purposes.
///
/// [`layout_offset`]'s answer where it has one (so Up seats north and Down south, exactly as the
/// solve already lays them out), and the horizontal pair the drawn and vector maps already give
/// In and Out — `export_svg::side_for_travel` puts both on the right-hand side, and a seated room
/// needs the two to differ so an In and an Out from one room do not want the same cell.
/// `Unknown` has no bearing at all and is never seated.
pub fn seat_offset(dir: Direction) -> Option<(i32, i32)> {
    match dir {
        Direction::In => Some((1, 0)),
        Direction::Out => Some((-1, 0)),
        Direction::Unknown => None,
        d => layout_offset(d),
    }
}

/// Every pair of rooms in `positions` joined by a CARDINAL RECIPROCAL passage that currently sits
/// exactly one cell apart, in the direction the passage claims.
///
/// The one invariant [`seat_adjacent`] will not trade away. Keyed by the ordered pair so the set
/// is comparable across a trial shift; a pair whose rooms are already further apart than the
/// passage claims is not in here at all, and so cannot be made worse by a shift either.
fn adjacent_reciprocals(
    graph: &MapGraph,
    positions: &BTreeMap<RoomId, (i32, i32)>,
) -> BTreeSet<(RoomId, RoomId)> {
    let conns = graph.connections();
    let mut out = BTreeSet::new();
    for c in conns {
        if c.is_self_loop() {
            continue;
        }
        let Some(off) = grid_offset(c.dir) else { continue };
        let (Some(&a), Some(&b)) = (positions.get(&c.origin), positions.get(&c.dest)) else {
            continue;
        };
        if (b.0 - a.0, b.1 - a.1) != off {
            continue;
        }
        if !conns.iter().any(|o| o.origin == c.dest && o.dest == c.origin) {
            continue; // one-way: no cell-count claim
        }
        out.insert((c.origin, c.dest));
    }
    out
}

/// Slide every room at or beyond `line` on one axis by `step`, inserting a blank row/column.
fn open_line(positions: &mut BTreeMap<RoomId, (i32, i32)>, horizontal: bool, line: i32, step: i32) {
    for p in positions.values_mut() {
        let v = if horizontal { p.0 } else { p.1 };
        let beyond = if step > 0 { v >= line } else { v <= line };
        if beyond {
            if horizontal {
                p.0 += step;
            } else {
                p.1 += step;
            }
        }
    }
}

/// Where a late-arriving room seats itself relative to `anchor`, and what the map had to do to
/// let it (SQ-1356).
///
/// The three travel together because a caller that reads the cell without knowing whether the
/// map moved under it would draw the newcomer against stale neighbours: `positions` is mutated
/// in place when `opened_line` is set, and every cell in it — the anchor's included — must be
/// re-read afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seat {
    /// The cell the newcomer takes. Free in `positions` on return.
    pub cell: (i32, i32),
    /// True when the wanted cell was already free and nothing moved.
    pub direct: bool,
    /// True when a line was opened to make room (every other room may have moved).
    pub opened_line: bool,
}

/// Seat a late-arriving room in the cell adjacent to `anchor` along `offset` (SQ-1356).
///
/// `positions` is every room already standing on this plane — one layer's rooms, plus whatever
/// earlier newcomers have already been seated — and the newcomer itself must NOT be in it. On
/// return the chosen cell is free in `positions`; the caller inserts the newcomer there.
///
/// Four outcomes, in order:
///
/// 1. the wanted cell is free and is taken;
/// 2. it is not, and a blank line is opened at it by sliding that whole side of the map one cell
///    further out — accepted only when every cardinal-reciprocal pair that was exactly adjacent
///    still is (see the module docs). A diagonal `offset` may open EITHER of its two lines, so
///    both are tried, the horizontal one first;
/// 3. neither, and the newcomer takes a free cell still ADJACENT to the anchor, off the bearing:
///    the two sides perpendicular to it first, then the side opposite. That is what a real room
///    arriving to find its slot taken gets — a neighbour with a one-bend line — and it beats
///    step 4 by a distance: a newcomer pushed PAST the room blocking its bearing lands on the
///    far side of it, so the line has to be routed all the way around a room that has nothing to
///    do with the passage, and the two boxes it does join are no longer neighbours at all;
/// 4. only then does it walk out along the bearing to the first free cell, which is where a
///    genuinely boxed-in anchor ends up.
///
/// **The perpendicular sides are tried roomier-first.** Both are equally correct as geometry —
/// the passage is a portal or a crossing, so neither side is the direction anything was walked —
/// and the one with more free cells around it is the one whose line has somewhere to go and
/// whose box has room to be read. Ties keep the left-hand rotation, so the choice stays
/// deterministic.
///
/// `None` when `anchor` is not in `positions` (nothing to seat against).
pub fn seat_adjacent(
    graph: &MapGraph,
    positions: &mut BTreeMap<RoomId, (i32, i32)>,
    anchor: RoomId,
    offset: (i32, i32),
) -> Option<Seat> {
    let a = *positions.get(&anchor)?;
    let want = (a.0 + offset.0, a.1 + offset.1);
    let taken = |p: &BTreeMap<RoomId, (i32, i32)>, c: (i32, i32)| p.values().any(|&q| q == c);
    if !taken(positions, want) {
        return Some(Seat { cell: want, direct: true, opened_line: false });
    }

    let keep = adjacent_reciprocals(graph, positions);
    for (horizontal, step) in [(true, offset.0), (false, offset.1)] {
        if step == 0 {
            continue;
        }
        let line = if horizontal { want.0 } else { want.1 };
        let mut trial = positions.clone();
        open_line(&mut trial, horizontal, line, step);
        // The anchor sits one cell back from `line` on the stationary side, so it never moves and
        // the cell it wanted is now empty — but say so rather than assume it.
        if trial.get(&anchor) != Some(&a) || taken(&trial, want) {
            continue;
        }
        if keep.is_subset(&adjacent_reciprocals(graph, &trial)) {
            *positions = trial;
            return Some(Seat { cell: want, direct: false, opened_line: true });
        }
    }

    // Nothing legal to open: stay ADJACENT to the anchor instead of going round the blocker.
    // Perpendicular sides first, roomier one first; then the side opposite the bearing.
    let free_neighbours = |c: (i32, i32)| {
        [(1, 0), (-1, 0), (0, 1), (0, -1)]
            .into_iter()
            .filter(|(dx, dy)| !taken(positions, (c.0 + dx, c.1 + dy)))
            .count()
    };
    let mut sides = vec![(-offset.1, offset.0), (offset.1, -offset.0)];
    sides.sort_by_key(|&d| std::cmp::Reverse(free_neighbours((a.0 + d.0, a.1 + d.1))));
    sides.push((-offset.0, -offset.1));
    for d in sides {
        let c = (a.0 + d.0, a.1 + d.1);
        if c != want && !taken(positions, c) {
            return Some(Seat { cell: c, direct: false, opened_line: false });
        }
    }

    // Boxed in on every side: walk out along the bearing. Each step lands on a distinct cell, so
    // one more step than there are rooms is guaranteed to find a free one.
    let mut c = want;
    for _ in 0..=positions.len() {
        c = (c.0 + offset.0, c.1 + offset.1);
        if !taken(positions, c) {
            return Some(Seat { cell: c, direct: false, opened_line: false });
        }
    }
    Some(Seat { cell: c, direct: false, opened_line: false })
}

/// Re-seat every PORTAL-ONLY room onto its anchor's doorstep (SQ-1356).
///
/// A room whose every passage is Up/Down/In/Out has no compass bearing for the solve to place it
/// by, so it falls out of the stress stage wherever the collision pass could fit it — Zork I's
/// `Cellar` hanging off the `Living Room`'s `Down` came out across the map from it. This pass
/// runs last, after every room WITH a bearing has settled, and asks [`seat_adjacent`] for the
/// cell the portal itself points at.
///
/// Only rooms that are not already there move, and only within their own layer: a portal that
/// crosses a layer is that layer's ghost's business (`crate::render::render_layer`), not a claim
/// on a cell here.
pub fn seat_portal_leaves(graph: &MapGraph, final_pos: &mut BTreeMap<RoomId, (i32, i32)>) {
    let conns = graph.connections();
    // The rooms to seat, and the anchor + offset each wants: the lowest-id partner it shares a
    // portal with, so the pass is deterministic whatever order the passages were walked in.
    let mut wanted: Vec<(RoomId, RoomId, (i32, i32))> = Vec::new();
    for room in graph.rooms() {
        let mine: Vec<_> = conns
            .iter()
            .filter(|c| !c.is_self_loop() && (c.origin == room.id || c.dest == room.id))
            .collect();
        if mine.is_empty() || mine.iter().any(|c| grid_offset(c.dir).is_some()) {
            continue; // no passages at all, or at least one real compass bearing
        }
        let mut cands: Vec<(RoomId, (i32, i32))> = mine
            .iter()
            .filter_map(|c| {
                let (partner, out) = if c.origin == room.id {
                    // `room --dir--> partner`: the partner sits `dir`-ward, so `room` sits back.
                    (c.dest, seat_offset(crate::direction::opposite(c.dir))?)
                } else {
                    (c.origin, seat_offset(c.dir)?)
                };
                (graph.layer_of(partner) == room.layer && partner != room.id)
                    .then_some((partner, out))
            })
            .collect();
        cands.sort();
        if let Some(&(anchor, off)) = cands.first() {
            wanted.push((room.id, anchor, off));
        }
    }
    wanted.sort();

    for (id, anchor, off) in wanted {
        let layer = graph.layer_of(id);
        let mut plane: BTreeMap<RoomId, (i32, i32)> = final_pos
            .iter()
            .filter(|(&r, _)| r != id && graph.layer_of(r) == layer)
            .map(|(&r, &p)| (r, p))
            .collect();
        let Some(&a) = plane.get(&anchor) else { continue };
        if final_pos.get(&id) == Some(&(a.0 + off.0, a.1 + off.1)) {
            continue; // already on the doorstep: leave the map exactly as it was
        }
        let Some(seat) = seat_adjacent(graph, &mut plane, anchor, off) else { continue };
        for (r, p) in plane {
            final_pos.insert(r, p);
        }
        final_pos.insert(id, seat.cell);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;

    fn g_with(rooms: &[(RoomId, &str)], edges: &[(RoomId, Direction, RoomId)]) -> MapGraph {
        let mut g = MapGraph::new();
        for &(id, n) in rooms {
            g.upsert_room(id, n.into());
        }
        for &(a, d, b) in edges {
            g.add_edge(a, d, b);
        }
        g
    }

    #[test]
    fn a_free_doorstep_is_taken_and_nothing_moves() {
        let g = g_with(&[(1, "Hall"), (2, "Study")], &[(1, Direction::E, 2), (2, Direction::W, 1)]);
        let mut pos: BTreeMap<RoomId, (i32, i32)> = [(1, (0, 0)), (2, (1, 0))].into_iter().collect();
        let seat = seat_adjacent(&g, &mut pos, 1, (0, 1)).unwrap();
        assert_eq!(seat.cell, (0, 1));
        assert!(seat.direct);
        assert_eq!(pos[&2], (1, 0), "nothing moved");
    }

    /// The occupied doorstep opens: the row below slides one further down, and the newcomer takes
    /// the cell. The two rooms that straddle the cut are joined ONE WAY, so no cell count is
    /// claimed and the shift is legal.
    #[test]
    fn an_occupied_doorstep_opens_a_line() {
        let g = g_with(
            &[(1, "Hall"), (2, "Below"), (3, "Beside")],
            &[(1, Direction::Down, 9), (2, Direction::E, 3), (3, Direction::W, 2)],
        );
        let mut pos: BTreeMap<RoomId, (i32, i32)> =
            [(1, (0, 0)), (2, (0, 1)), (3, (1, 1))].into_iter().collect();
        let seat = seat_adjacent(&g, &mut pos, 1, (0, 1)).unwrap();
        assert_eq!(seat.cell, (0, 1), "the newcomer takes the cell it wanted");
        assert!(seat.opened_line);
        assert_eq!(pos[&1], (0, 0), "the anchor never moves");
        assert_eq!((pos[&2], pos[&3]), ((0, 2), (1, 2)), "the row below slid, keeping its own shape");
    }

    /// …but not when opening it would pull a cardinal-reciprocal pair apart: `1`–`2` is walked
    /// both ways, so the line stays shut and the newcomer stays ADJACENT to its anchor, on a side
    /// perpendicular to the bearing — never past `2`, which would put the blocker between the two
    /// boxes the passage joins.
    #[test]
    fn a_blocked_bearing_falls_back_to_a_neighbour_never_past_the_blocker() {
        let g = g_with(
            &[(1, "Hall"), (2, "Below")],
            &[(1, Direction::S, 2), (2, Direction::N, 1), (1, Direction::Down, 9)],
        );
        let mut pos: BTreeMap<RoomId, (i32, i32)> = [(1, (0, 0)), (2, (0, 1))].into_iter().collect();
        let seat = seat_adjacent(&g, &mut pos, 1, (0, 1)).unwrap();
        assert_eq!(seat.cell, (-1, 0), "beside the anchor, perpendicular to the blocked bearing");
        assert_ne!(seat.cell, (0, 2), "never on the far side of the room that blocked it");
        assert!(!seat.opened_line);
        assert_eq!(pos[&2], (0, 1), "and the pair is still exactly one cell apart");
        let (ax, ay) = pos[&1];
        assert!(
            (seat.cell.0 - ax).abs() <= 1 && (seat.cell.1 - ay).abs() <= 1,
            "still on the anchor's own doorstep: {:?} vs {:?}",
            seat.cell,
            (ax, ay)
        );
    }

    /// Both perpendicular sides are free, so the ROOMIER one wins — the side whose own
    /// neighbourhood has somewhere for the line to go and room for the box to be read. Here east
    /// of the anchor is walled in by `3` and `4`, so the newcomer goes west.
    #[test]
    fn the_roomier_perpendicular_side_wins() {
        let g = g_with(
            &[(1, "Hall"), (2, "Below"), (3, "East"), (4, "Corner")],
            &[(1, Direction::S, 2), (2, Direction::N, 1), (1, Direction::Down, 9)],
        );
        let mut pos: BTreeMap<RoomId, (i32, i32)> =
            [(1, (0, 0)), (2, (0, 1)), (3, (2, 0)), (4, (1, -1))].into_iter().collect();
        let seat = seat_adjacent(&g, &mut pos, 1, (0, 1)).unwrap();
        assert_eq!(seat.cell, (-1, 0), "west: east is hemmed in on two more sides");
    }

    /// The case the pass exists for: a portal-only room hanging off a hub in the middle of a
    /// block. It ends up directly below the hub, and the block opens to let it.
    #[test]
    fn a_portal_only_room_seats_below_its_hub_and_the_block_opens() {
        // A 3x3 block, joined into three east-west reciprocal rows (no vertical passages, so
        // nothing straddling the cut claims a cell count). `9` hangs off the centre by Down.
        let mut g = MapGraph::new();
        let mut ids = Vec::new();
        for row in 0..3 {
            for col in 0..3 {
                let id = 1 + row * 3 + col;
                g.upsert_room(id, format!("R{id}"));
                g.set_pos(id, (col as i32 - 1, row as i32 - 1));
                ids.push(id);
            }
        }
        for row in 0..3 {
            for col in 0..2 {
                let (a, b) = (1 + row * 3 + col, 1 + row * 3 + col + 1);
                g.add_edge(a, Direction::E, b);
                g.add_edge(b, Direction::W, a);
            }
        }
        g.upsert_room(9_999, "Cellar".into());
        g.set_pos(9_999, (7, 7)); // wherever the collision pass left it
        g.add_edge(5, Direction::Down, 9_999);
        g.add_edge(9_999, Direction::Up, 5);

        let mut pos: BTreeMap<RoomId, (i32, i32)> =
            g.rooms().filter_map(|r| r.pos.map(|p| (r.id, p))).collect();
        seat_portal_leaves(&g, &mut pos);
        assert_eq!(pos[&5], (0, 0), "the hub stayed put");
        assert_eq!(pos[&9_999], (0, 1), "the cellar is directly below the hub");
        for row in [7, 8, 9] {
            assert_eq!(pos[&row].1, 2, "the bottom row opened to make space: {row} at {:?}", pos[&row]);
        }
        // Every east-west pair is still exactly adjacent.
        for row in 0..3 {
            for col in 0..2 {
                let (a, b) = (1 + row * 3 + col, 1 + row * 3 + col + 1);
                assert_eq!(pos[&b].0 - pos[&a].0, 1, "{a}-{b} stayed adjacent");
                assert_eq!(pos[&b].1, pos[&a].1);
            }
        }
    }

    #[test]
    fn seat_offset_reads_up_down_in_out() {
        assert_eq!(seat_offset(Direction::Up), Some((0, -1)));
        assert_eq!(seat_offset(Direction::Down), Some((0, 1)));
        assert_eq!(seat_offset(Direction::In), Some((1, 0)));
        assert_eq!(seat_offset(Direction::Out), Some((-1, 0)));
        assert_eq!(seat_offset(Direction::NE), Some((1, -1)));
        assert_eq!(seat_offset(Direction::Unknown), None);
    }
}
