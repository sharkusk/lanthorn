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
//! opening would pull one apart is not opened at all. A passage that is one-way, diagonal, gated
//! or already stretched may lengthen — none of those claims a cell count.
//!
//! **A whole-side slide is all-or-nothing, though, and that was not enough** (SQ-1358). Zork I's
//! house: the Studio's stairs arrive from below the `Kitchen`, whose doorstep `South of House`
//! holds. Opening the row would have separated a `Clearing` from the `Forest` directly below it —
//! a walked north/south pair far away on the same cut, with nothing to do with the house — so the
//! slide was vetoed and the ghost was parked BELOW `South of House`, its line looping around a
//! room the crossing never touches. But `South of House` itself is joined to the house only by
//! distorted passages: it is half of no adjacent pair at all, and could have stepped down a cell
//! on its own. So a blocked bearing now tries a **chain push** as well — move the blocker one
//! cell further along the bearing, and whatever IT would then displace with it, as a rigid chain
//! — under exactly the same veto, room by room instead of side by side.

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

/// The most cells a chain push will move rooms out of (SQ-1358).
///
/// A late arrival is worth a nudge to the rooms standing in front of it, not a migration of a
/// dense column across the whole map: past a few cells the map has plainly told you the bearing
/// is full, and the newcomer is better off beside its anchor.
const MAX_PUSH_CHAIN: usize = 4;

/// Push whatever stands on `want` — and whatever THAT would displace — one cell further along
/// `offset`, as a rigid chain (SQ-1358).
///
/// Walks the bearing from `want` until it reaches a free cell, gathering every room on the way;
/// the whole gathered set then moves by `offset`, so each moved room keeps its exact offset from
/// every other moved room. `Some` with the pushed positions when the move is legal — every
/// cardinal-reciprocal pair in `keep` (the map's set BEFORE the push) is still adjacent after, so
/// a partner either travelled in the chain or was never adjacent to begin with; `None` when a
/// partner would be left behind, or when the chain is longer than [`MAX_PUSH_CHAIN`] cells.
///
/// The anchor stands one cell BACK from `want`, against the bearing, so it is never on the chain
/// and never moves.
fn push_chain(
    graph: &MapGraph,
    positions: &BTreeMap<RoomId, (i32, i32)>,
    keep: &BTreeSet<(RoomId, RoomId)>,
    want: (i32, i32),
    offset: (i32, i32),
) -> Option<BTreeMap<RoomId, (i32, i32)>> {
    let mut cell = want;
    let mut moving: BTreeSet<RoomId> = BTreeSet::new();
    for _ in 0..MAX_PUSH_CHAIN {
        let here: Vec<RoomId> =
            positions.iter().filter(|(_, &p)| p == cell).map(|(&r, _)| r).collect();
        if here.is_empty() {
            let mut trial = positions.clone();
            for (r, p) in trial.iter_mut() {
                if moving.contains(r) {
                    *p = (p.0 + offset.0, p.1 + offset.1);
                }
            }
            return keep.is_subset(&adjacent_reciprocals(graph, &trial)).then_some(trial);
        }
        moving.extend(here);
        cell = (cell.0 + offset.0, cell.1 + offset.1);
    }
    None
}

/// Where a late-arriving room seats itself relative to `anchor`, and what the map had to do to
/// let it (SQ-1356).
///
/// The three travel together because a caller that reads the cell without knowing whether the
/// map moved under it would draw the newcomer against stale neighbours: `positions` is mutated
/// in place when `moved_map` is set, and every cell in it — the anchor's included — must be
/// re-read afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seat {
    /// The cell the newcomer takes. Free in `positions` on return.
    pub cell: (i32, i32),
    /// True when the wanted cell was already free and nothing moved.
    pub direct: bool,
    /// True when the map had to move to make room — a blank line opened, or the blocker pushed
    /// along the bearing (SQ-1358). Every other room may have shifted.
    pub moved_map: bool,
}

/// Seat a late-arriving room in the cell adjacent to `anchor` along `offset` (SQ-1356).
///
/// `positions` is every room already standing on this plane — one layer's rooms, plus whatever
/// earlier newcomers have already been seated — and the newcomer itself must NOT be in it. On
/// return the chosen cell is free in `positions`; the caller inserts the newcomer there.
///
/// Five outcomes, in order:
///
/// 1. the wanted cell is free and is taken;
/// 2. it is not, and a blank line is opened at it by sliding that whole side of the map one cell
///    further out — accepted only when every cardinal-reciprocal pair that was exactly adjacent
///    still is (see the module docs). A diagonal `offset` may open EITHER of its two lines, so
///    both are tried, the horizontal one first;
/// 3. the slide is vetoed, so the blocker alone is asked to step aside: it moves one cell further
///    along the bearing, and whatever it would then displace moves with it as a rigid chain, up
///    to [`MAX_PUSH_CHAIN`] cells of it — under the same veto, and the newcomer still gets the
///    cell it wanted. This is what a slide cannot do, because a slide is all-or-nothing: one
///    faraway pair straddling the cut vetoes it for the whole side, however free the blocker
///    itself is (SQ-1358 — Zork I's `South of House`, see the module docs);
/// 4. neither, and the newcomer takes a free cell still ADJACENT to the anchor, off the bearing:
///    the two sides perpendicular to it first, then the side opposite. That is what a real room
///    arriving to find its slot taken gets — a neighbour with a one-bend line — and it beats
///    step 5 by a distance: a newcomer pushed PAST the room blocking its bearing lands on the
///    far side of it, so the line has to be routed all the way around a room that has nothing to
///    do with the passage, and the two boxes it does join are no longer neighbours at all;
/// 5. only then does it walk out along the bearing to the first free cell, which is where a
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
        return Some(Seat { cell: want, direct: true, moved_map: false });
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
            return Some(Seat { cell: want, direct: false, moved_map: true });
        }
    }

    // No line may open, but the blocker itself may still be free to step aside: push it — and
    // whatever it would displace — one cell along the bearing, under the same veto.
    if let Some(trial) = push_chain(graph, positions, &keep, want, offset) {
        *positions = trial;
        return Some(Seat { cell: want, direct: false, moved_map: true });
    }

    // Nothing legal to move: stay ADJACENT to the anchor instead of going round the blocker.
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
            return Some(Seat { cell: c, direct: false, moved_map: false });
        }
    }

    // Boxed in on every side: walk out along the bearing. Each step lands on a distinct cell, so
    // one more step than there are rooms is guaranteed to find a free one.
    let mut c = want;
    for _ in 0..=positions.len() {
        c = (c.0 + offset.0, c.1 + offset.1);
        if !taken(positions, c) {
            return Some(Seat { cell: c, direct: false, moved_map: false });
        }
    }
    Some(Seat { cell: c, direct: false, moved_map: false })
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
        assert!(seat.moved_map);
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
        assert!(!seat.moved_map);
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

    // ── SQ-1358: the blocker itself steps aside ────────────────────────────────

    /// Every room of [`house`] but `South of House`, at the cells none of these cases may move it
    /// from: the whole point is that ONE room steps aside, not that the map rearranges itself.
    const HOUSE_AT_REST: [(RoomId, (i32, i32)); 6] =
        [(1, (-1, 0)), (2, (0, 0)), (3, (1, 0)), (4, (0, -1)), (6, (2, 0)), (7, (2, 1))];

    /// Zork I's house, reduced to its shape: a three-wide row (`1`–`2`–`3`), an `Attic` above the
    /// middle, `South of House` below it, and a `Clearing`/`Forest` pair further east that is
    /// walked north/south across the same row. The Studio's stairs arrive from BELOW the middle.
    ///
    /// The whole-side slide is vetoed by the Clearing/Forest pair — a pair with nothing to do with
    /// the house — but `South of House` is joined to the house only by distorted passages, so it
    /// is half of no adjacent pair and can step down a cell alone. The house grows a row.
    fn house(
        joined_to_the_row: &[(RoomId, Direction, RoomId)],
    ) -> (MapGraph, BTreeMap<RoomId, (i32, i32)>) {
        let mut edges = vec![
            (1, Direction::E, 2),
            (2, Direction::W, 1),
            (2, Direction::E, 3),
            (3, Direction::W, 2),
            // The Attic hangs off the Kitchen by a staircase: Up/Down claim no cell count.
            (2, Direction::Up, 4),
            (4, Direction::Down, 2),
            // The pair that vetoes the slide, walked both ways across the cut at y = 1.
            (6, Direction::S, 7),
            (7, Direction::N, 6),
        ];
        edges.extend_from_slice(joined_to_the_row);
        let g = g_with(
            &[
                (1, "Living Room"),
                (2, "Kitchen"),
                (3, "Behind House"),
                (4, "Attic"),
                (5, "South of House"),
                (6, "Clearing"),
                (7, "Forest"),
            ],
            &edges,
        );
        let pos = HOUSE_AT_REST.into_iter().chain([(5, (0, 1))]).collect();
        (g, pos)
    }

    #[test]
    fn a_lone_blocker_steps_aside_and_the_house_grows_a_row() {
        // Every passage `South of House` owns is distorted — it sits a cell away diagonally from
        // each of the rooms it names, so none of them is an adjacent pair.
        let (g, mut pos) = house(&[
            (5, Direction::W, 1),
            (1, Direction::S, 5),
            (5, Direction::E, 3),
            (3, Direction::S, 5),
        ]);
        let seat = seat_adjacent(&g, &mut pos, 2, (0, 1)).unwrap();
        assert_eq!(seat.cell, (0, 1), "the ghost took the doorstep it wanted");
        assert!(seat.moved_map);
        assert_eq!(pos[&5], (0, 2), "South of House stepped down a row by itself");
        for (id, cell) in HOUSE_AT_REST {
            assert_eq!(pos[&id], cell, "room {id} did not move");
        }
    }

    /// …but a blocker that is half of an adjacent walked pair is not pushed: its partner stands
    /// beside the chain rather than travelling in it, so the push would pull the two apart. The
    /// seating falls through to the old answer — past the blocker, along the bearing.
    #[test]
    fn a_blocker_with_a_walked_partner_is_not_pushed() {
        let (g, mut pos) = house(&[(2, Direction::S, 5), (5, Direction::N, 2)]);
        let seat = seat_adjacent(&g, &mut pos, 2, (0, 1)).unwrap();
        assert!(!seat.moved_map, "nothing legal to move");
        assert_eq!(seat.cell, (0, 2), "past the blocker, exactly as before SQ-1358");
        assert_eq!(pos[&5], (0, 1), "and the walked pair is still exactly one cell apart");
        for (id, cell) in HOUSE_AT_REST {
            assert_eq!(pos[&id], cell, "room {id} did not move");
        }
    }

    /// The chain is RIGID: the blocker's own blocker travels with it, so a pair inside the chain
    /// keeps its offset and only the far end of the column meets open ground.
    #[test]
    fn a_pushed_blocker_takes_the_room_behind_it_with_it() {
        let g = g_with(
            &[(1, "Hall"), (2, "Below"), (3, "Further Below"), (6, "Clearing"), (7, "Forest")],
            &[
                (2, Direction::S, 3),
                (3, Direction::N, 2),
                (6, Direction::S, 7),
                (7, Direction::N, 6),
            ],
        );
        let mut pos: BTreeMap<RoomId, (i32, i32)> =
            [(1, (0, 0)), (2, (0, 1)), (3, (0, 2)), (6, (2, 0)), (7, (2, 1))].into_iter().collect();
        let seat = seat_adjacent(&g, &mut pos, 1, (0, 1)).unwrap();
        assert_eq!(seat.cell, (0, 1));
        assert!(seat.moved_map);
        assert_eq!((pos[&2], pos[&3]), ((0, 2), (0, 3)), "both moved, keeping their own offset");
        assert_eq!((pos[&1], pos[&6], pos[&7]), ((0, 0), (2, 0), (2, 1)), "and nothing else did");
    }

    /// A column deeper than [`MAX_PUSH_CHAIN`] is not pushed at all: past a few cells the bearing
    /// is plainly full, and the newcomer is better off beside its anchor than shunting the map.
    #[test]
    fn a_chain_longer_than_the_cap_is_refused() {
        let g = g_with(
            &[(1, "Hall"), (6, "Clearing"), (7, "Forest")],
            &[(6, Direction::S, 7), (7, Direction::N, 6)],
        );
        let mut pos: BTreeMap<RoomId, (i32, i32)> =
            [(1, (0, 0)), (6, (2, 0)), (7, (2, 1))].into_iter().collect();
        for (n, id) in (11..).take(MAX_PUSH_CHAIN + 1).enumerate() {
            pos.insert(id, (0, n as i32 + 1));
        }
        let seat = seat_adjacent(&g, &mut pos, 1, (0, 1)).unwrap();
        assert!(!seat.moved_map, "the column stayed put");
        assert_eq!(seat.cell, (-1, 0), "beside the anchor instead");
        assert_eq!(pos[&11], (0, 1), "the blocker never budged");
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
