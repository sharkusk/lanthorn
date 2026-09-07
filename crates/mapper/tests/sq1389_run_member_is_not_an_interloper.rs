//! **A run's own member never splits that run** (SQ-1389).
//!
//! Lost Pig's `(gnomeRoom) #194` is joined to `Table Room #147` and `Shelf Room #157` by two
//! reciprocal E/W pairs and by nothing else, so exactly one cell satisfies it: the free cell
//! between them, directly south of the `Fountain Room` hub. The stress solve found that cell, the
//! alignment stage kept it — and the CONTIGUITY stage moved the room three rows north, wedging it
//! between `Statue Room` and `Windy Cave` and splitting the reciprocal N/S pair those two hold.
//! All four of the gnome room's edges then drew distorted.
//!
//! The cause is one predicate. `mapper::layout::splits_a_run` answers "does a room standing on
//! this cell lie strictly between two members of a cardinal-reciprocal run?" and it was asked
//! about a CELL with no idea who was standing on it, so every interior member of every run
//! answered its own run YES. `#147 ─E→ #194 ─E→ #157` is one such run and #194 is its middle: the
//! room was judged to be splitting the very chain it completes. `open_gated_holes_for_hubs` reads
//! that answer as "this hub is stuck", went looking for a gated link to prise open, found one in
//! the `Fountain ─ Statue ─ Windy` column, slid `Windy Cave` up a row and dropped the gnome room
//! into the hole.
//!
//! The rule the fix states: **a room standing on its run's line between two fellow members IS the
//! run, not an interloper in it.** Membership is asked per run, so a room may still be a legitimate
//! member of one chain and a genuine interloper inside a different one — which is the case the pass
//! exists for.
//!
//! The shape below is Lost Pig's `Hole` layer reduced to the six rooms that produce it: a hub with
//! W/E/N cardinals and SW/SE diagonals, and two rooms on the row beneath it joined to each other
//! through a third. Every one of the six is placed by the solve alone — no positions are given —
//! so the case fails if the contiguity stage ever takes the middle room off its cell again.

use std::collections::BTreeMap;

use mapper::direction::Direction::{self, E, N, SE, SW, W};
use mapper::graph::{MapGraph, PassageWeight, RoomId};
use mapper::layout::relayout_auto;

// The Lost Pig rooms these stand for, so a reader can put the drawing beside the case.
const HUB: RoomId = 1; // Fountain Room #111
const WEST: RoomId = 2; // Hole #102
const EAST: RoomId = 3; // Cave With Stream #166
const NORTH: RoomId = 4; // Statue Room #128
const FAR_N: RoomId = 5; // Windy Cave #177
const LOW_W: RoomId = 6; // Table Room #147
const MIDDLE: RoomId = 7; // (gnomeRoom) #194
const LOW_E: RoomId = 8; // Shelf Room #157

/// A passage walked from both ends: `a -dir-> b` and the answering bearing back.
fn pair(g: &mut MapGraph, a: RoomId, b: RoomId, dir: Direction) {
    pair_weighted(g, a, b, dir, PassageWeight::Hard)
}

fn pair_weighted(g: &mut MapGraph, a: RoomId, b: RoomId, dir: Direction, w: PassageWeight) {
    g.add_edge_weighted(a, dir, b, w);
    g.add_edge_weighted(b, mapper::direction::opposite(dir), a, w);
}

/// The reduced `Hole` layer. Nothing is positioned — `relayout_auto` places all eight rooms.
///
/// The GATED link is load-bearing, and it is the half of the shape that is easy to leave out.
/// `open_gated_holes_for_hubs` only ever prises a run apart at a link the story itself gates
/// ([`PassageWeight::may_reach_past_a_room`]), so without one the mis-flagged hub is judged stuck
/// and then finds nowhere to go — the defect is latent and the case passes with the bug in.
/// Lost Pig's `Statue Room ─ Windy Cave` is exactly such a link (`kind: routine` — a destination
/// the story's own code decides), which is why the gnome room had somewhere to be pushed.
fn hole_layer() -> MapGraph {
    let mut g = MapGraph::new();
    for (id, name) in [
        (HUB, "Hub"),
        (WEST, "West"),
        (EAST, "East"),
        (NORTH, "North"),
        (FAR_N, "Far North"),
        (LOW_W, "Low West"),
        (MIDDLE, "Middle"),
        (LOW_E, "Low East"),
    ] {
        g.upsert_room(id, name.into());
    }
    // The hub's five passages: three cardinals and the two diagonals down to the row below.
    pair(&mut g, HUB, WEST, W);
    pair(&mut g, HUB, EAST, E);
    pair(&mut g, HUB, NORTH, N);
    pair(&mut g, HUB, LOW_W, SW);
    pair(&mut g, HUB, LOW_E, SE);
    // The column above the hub continues through a GATED link — the hole the bug prised open.
    pair_weighted(&mut g, NORTH, FAR_N, N, PassageWeight::Conditional);
    // The row below: two rooms joined to each other only through the middle one.
    pair(&mut g, LOW_W, MIDDLE, E);
    pair(&mut g, MIDDLE, LOW_E, E);
    g
}

fn pos(g: &MapGraph) -> BTreeMap<RoomId, (i32, i32)> {
    g.rooms().filter_map(|r| r.pos.map(|p| (r.id, p))).collect()
}

/// **The middle room lands on the middle cell.** Its two reciprocal E/W pairs name one cell each
/// side of it, and the layout puts it exactly there: one column east of `LOW_W`, one column west of
/// `LOW_E`, all three on one row.
#[test]
fn the_middle_room_of_a_run_keeps_the_middle_cell() {
    let mut g = hole_layer();
    relayout_auto(&mut g);
    let p = pos(&g);
    let (w, m, e) = (p[&LOW_W], p[&MIDDLE], p[&LOW_E]);
    assert_eq!(m.1, w.1, "the middle room shares the low row with LOW_W ({m:?} vs {w:?})");
    assert_eq!(m.1, e.1, "…and with LOW_E ({m:?} vs {e:?})");
    assert_eq!(m.0, w.0 + 1, "…one column east of LOW_W ({m:?} vs {w:?})");
    assert_eq!(m.0 + 1, e.0, "…and one column west of LOW_E ({m:?} vs {e:?})");
}

/// **And nothing was traded for it.** The gated column above the hub stays whole — the three rooms
/// on it are three consecutive cells — which is the run the middle room used to be dropped into.
#[test]
fn the_hubs_north_pair_stays_adjacent() {
    let mut g = hole_layer();
    relayout_auto(&mut g);
    let p = pos(&g);
    let (hub, north, far) = (p[&HUB], p[&NORTH], p[&FAR_N]);
    assert_eq!(north.0, hub.0, "NORTH holds the hub's column ({north:?} vs {hub:?})");
    assert_eq!(north.1, hub.1 - 1, "…and sits one row above it ({north:?} vs {hub:?})");
    assert_eq!(far.0, hub.0, "FAR_N holds the same column ({far:?} vs {hub:?})");
    assert_eq!(
        far.1,
        north.1 - 1,
        "…and the gated link stays one cell long, not stretched open ({far:?} vs {north:?})"
    );
    // The middle room shares that column — it is directly SOUTH of the hub, which is where its own
    // two passages put it. What it must never do is stand between the hub and NORTH, which is the
    // cell `open_gated_holes_for_hubs` used to prise open for it.
    assert!(
        p[&MIDDLE].1 > hub.1,
        "the middle room is south of the hub, not inside its northern pair ({:?} vs {hub:?})",
        p[&MIDDLE]
    );
}

/// **No edge is distorted.** Every passage in this shape is drawable on the grid, so the layout
/// must draw every one of them plainly. This is the reader's version of the two cases above: the
/// gnome room's four edges all drew distorted, which is what the user saw.
#[test]
fn no_edge_of_the_reduced_hole_layer_is_distorted() {
    let mut g = hole_layer();
    relayout_auto(&mut g);
    let bad: Vec<String> = g
        .connections()
        .iter()
        .filter(|c| c.distorted)
        .map(|c| format!("#{} -{:?}-> #{}", c.origin, c.dir, c.dest))
        .collect();
    assert!(bad.is_empty(), "distorted: {}\npositions {:?}", bad.join(", "), pos(&g));
}
