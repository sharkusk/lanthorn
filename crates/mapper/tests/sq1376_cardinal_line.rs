//! A cardinal bearing claims a LINE, not a cell count (SQ-1376).
//!
//! The user, looking at the mapgen Zork I map: *"room 91 should be west of 68. that relationship
//! is getting lost somewhere (it works live)"*. `Forest #91` is the room `West of House #68`'s own
//! `W` exit names, and the generated map drew it a column to the EAST.
//!
//! Two rules came out of that, and this file pins both on synthetic graphs — one per axis, so
//! neither can be fixed for rows and left broken for columns:
//!
//! 1. **A reciprocal cardinal pair is satisfied when it is ALIGNED, at any length.** Same row for
//!    E/W, same column for N/S; the distance along the line is the layout's business.
//!    [`mapper::layout::edge_is_satisfied`] used to demand adjacency as well (SQ-1364), which made
//!    "exactly one cell apart" a claim the solve had to buy — and on Zork I it was bought with a
//!    bearing the game had actually stated.
//! 2. **A one-way cardinal exit is honoured when its destination lies on the correct SIDE.** The
//!    cross-axis offset is free; whether such an exit draws plain or distorted is a separate
//!    question this quest does not touch.
//!
//! **And adjacency is still a PREFERENCE.** `tighten_runs` closes a run's internal gaps when the
//! cells between them are free and nothing else wants them, exactly as it did at SQ-1312 — see
//! [`a_free_gap_inside_a_run_still_closes`] and this crate's `sq1364_clearing_gap`. What changed
//! is that failing to close one is no longer a lie.

use std::collections::BTreeMap;

use mapper::direction::{grid_offset, Direction};
use mapper::graph::{MapGraph, RoomId};
use mapper::layout::{edge_is_satisfied, relayout_auto};

/// Add a two-way passage: `a -dir-> b` and the answering bearing back.
fn pair(g: &mut MapGraph, a: RoomId, b: RoomId, dir: Direction) {
    g.add_edge(a, dir, b);
    g.add_edge(b, mapper::direction::opposite(dir), a);
}

fn rooms(g: &mut MapGraph, ids: &[(RoomId, &str)]) {
    for &(id, name) in ids {
        g.upsert_room(id, name.into());
    }
}

fn pos(g: &MapGraph) -> BTreeMap<RoomId, (i32, i32)> {
    g.rooms().filter_map(|r| r.pos.map(|p| (r.id, p))).collect()
}

/// Find the connection `origin -dir-> dest`, or panic naming what the graph does hold.
fn conn(g: &MapGraph, origin: RoomId, dir: Direction, dest: RoomId) -> mapper::graph::Connection {
    g.connections()
        .iter()
        .find(|c| c.origin == origin && c.dir == dir && c.dest == dest)
        .cloned()
        .unwrap_or_else(|| panic!("no edge #{origin} {dir:?} #{dest} in {:?}", g.connections()))
}

/// **The ROW case.** One room lies west of two others on different rows: west of `EAST` by a
/// walked-both-ways passage, and west of `NORTH` by a one-way exit `NORTH` alone reported.
///
/// The pair must come out on ONE ROW — that is the hard constraint, and its length is not this
/// case's business — and the one-way's destination must still be on the correct SIDE of the room
/// that names it. This is Zork I's `Forest #91` in miniature: it is due west of `Forest Path #247`
/// (walked both ways) and west of `West of House #68` (a one-way), and the map has to say both.
#[test]
fn a_room_west_of_two_rooms_keeps_the_row_and_the_side() {
    let mut g = MapGraph::new();
    rooms(&mut g, &[(1, "West"), (2, "East"), (3, "North"), (4, "Anchor")]);
    // The reciprocal pair, on one row: #1 is due west of #2.
    pair(&mut g, 2, 1, Direction::W);
    // #3 sits north of #2, so it is on a DIFFERENT row from the pair …
    pair(&mut g, 2, 3, Direction::N);
    // … and its own west exit names #1, one way only: nothing leads back from #1 to #3.
    g.add_edge(3, Direction::W, 1);
    // A fourth room, east of #2, so the component is not a bare triangle the solve can rotate.
    pair(&mut g, 2, 4, Direction::E);

    relayout_auto(&mut g);
    let p = pos(&g);
    let (west, east, north) = (p[&1], p[&2], p[&3]);

    assert_eq!(
        west.1, east.1,
        "the reciprocal E/W pair must share a row: #1 {west:?}, #2 {east:?}"
    );
    assert!(west.0 < east.0, "#1 {west:?} must be WEST of #2 {east:?}");
    assert!(
        !conn(&g, 2, Direction::W, 1).distorted && !conn(&g, 1, Direction::E, 2).distorted,
        "an aligned reciprocal pair is honoured however long it is: {:?}",
        g.connections(),
    );
    assert!(
        west.0 < north.0,
        "the one-way #3 W #1 must keep #1 {west:?} on the WEST side of #3 {north:?}"
    );
}

/// **The COLUMN case: the same map rotated ninety degrees.** One room lies north of two others in
/// different columns — north of `SOUTH` by a walked-both-ways passage, north of `EAST` by a
/// one-way.
///
/// Every rule in this file is stated per-axis, and a rule that holds on rows and not on columns is
/// half a rule. Nothing here is Zork-specific: it is the row case above with N/S for E/W.
#[test]
fn a_room_north_of_two_rooms_keeps_the_column_and_the_side() {
    let mut g = MapGraph::new();
    rooms(&mut g, &[(1, "North"), (2, "South"), (3, "East"), (4, "Anchor")]);
    pair(&mut g, 2, 1, Direction::N);
    pair(&mut g, 2, 3, Direction::E);
    g.add_edge(3, Direction::N, 1);
    pair(&mut g, 2, 4, Direction::S);

    relayout_auto(&mut g);
    let p = pos(&g);
    let (north, south, east) = (p[&1], p[&2], p[&3]);

    assert_eq!(
        north.0, south.0,
        "the reciprocal N/S pair must share a column: #1 {north:?}, #2 {south:?}"
    );
    assert!(north.1 < south.1, "#1 {north:?} must be NORTH of #2 {south:?}");
    assert!(
        !conn(&g, 2, Direction::N, 1).distorted && !conn(&g, 1, Direction::S, 2).distorted,
        "an aligned reciprocal pair is honoured however long it is: {:?}",
        g.connections(),
    );
    assert!(
        north.1 < east.1,
        "the one-way #3 N #1 must keep #1 {north:?} on the NORTH side of #3 {east:?}"
    );
}

/// **An aligned pair is NOT distorted at any length, on either axis.**
///
/// The definition itself, read off hand-placed cells rather than off a solve, so the case says
/// what the rule IS rather than what one graph happened to do. The cross axis is the constraint:
/// step a cell off the line and the same pair is distorted again.
#[test]
fn an_aligned_pair_is_not_distorted_however_long_it_is() {
    for (dir, back, far, off) in [
        (Direction::E, Direction::W, (6, 0), (6, 1)),
        (Direction::W, Direction::E, (-6, 0), (-6, 1)),
        (Direction::S, Direction::N, (0, 6), (1, 6)),
        (Direction::N, Direction::S, (0, -6), (1, -6)),
    ] {
        let mut g = MapGraph::new();
        rooms(&mut g, &[(1, "A"), (2, "B")]);
        pair(&mut g, 1, 2, dir);
        g.set_pos(1, (0, 0));

        for (cell, want, why) in [
            (grid_offset(dir).unwrap(), true, "adjacent"),
            (far, true, "aligned, six cells out"),
            (off, false, "one cell off the line"),
        ] {
            g.set_pos(2, cell);
            for c in g.connections() {
                assert_eq!(
                    edge_is_satisfied(&g, c),
                    want,
                    "{dir:?}/{back:?} pair at {cell:?} ({why}): #{} {:?} #{}",
                    c.origin,
                    c.dir,
                    c.dest,
                );
            }
        }
    }
}

/// **Gap-closing survives: a free gap inside a run still closes** (SQ-1312's rule, unchanged).
///
/// Alignment is the constraint and adjacency is the preference — but a preference that never fires
/// is not a preference. A five-room E/W chain whose middle link the stress solve leaves slack must
/// still come out with every member exactly one cell from the next, because nothing else wants
/// those cells.
///
/// The column twin is asserted in the same body, on the same shape rotated, for the reason the
/// two cases above are written twice.
#[test]
fn a_free_gap_inside_a_run_still_closes() {
    for horizontal in [true, false] {
        let dir = if horizontal { Direction::E } else { Direction::S };
        let mut g = MapGraph::new();
        rooms(&mut g, &[(1, "A"), (2, "B"), (3, "C"), (4, "D"), (5, "E")]);
        for a in 1..5 {
            pair(&mut g, a, a + 1, dir);
        }
        relayout_auto(&mut g);
        let p = pos(&g);
        let line = |c: (i32, i32)| if horizontal { c.1 } else { c.0 };
        let along = |c: (i32, i32)| if horizontal { c.0 } else { c.1 };
        let mut cells: Vec<(i32, i32)> = (1..=5).map(|id| p[&id]).collect();
        cells.sort_by_key(|&c| along(c));
        assert!(
            cells.iter().all(|&c| line(c) == line(cells[0])),
            "the whole chain shares one line: {cells:?}"
        );
        for w in cells.windows(2) {
            assert_eq!(
                along(w[1]) - along(w[0]),
                1,
                "a free gap inside a run must still close (horizontal={horizontal}): {cells:?}"
            );
        }
    }
}
