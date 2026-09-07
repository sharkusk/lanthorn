//! A reciprocal cardinal pair keeps its neighbour, and a losing passage says so (SQ-1364).
//!
//! `unit_tests/zork1_walked_map.json` is one player's real, partial mapping of Zork I
//! (`zork1-invclues-r52-s871125.z5`), lifted verbatim from the `map.json` inside a lanthorn
//! archive: 26 rooms across three layers, 19 of them above ground. It carries the three-way
//! conflict this quest is about, and no synthetic graph reproduces it as economically —
//! `crates/mapper/src/layout/mod.rs` holds the synthetic pins; this is the map they came from.
//!
//! **The shape.** Three claims land on the `Forest` #230's cell at once:
//!
//! | claim | reciprocated? | what it wants |
//! |---|---|---|
//! | `134 S 230` / `230 N 134` | yes | the Forest in the cell directly below the `Clearing` |
//! | `217 S 230` | no | the Forest a row below `South of House` |
//! | `230 NW 217` | no | `South of House` up and to the left of the Forest |
//!
//! The last two cannot both be true of any pair of cells once the Clearing is fixed, and the
//! stress solve used to split the difference by dropping the Forest an extra row — leaving the
//! reciprocal pair aligned but TWO apart with an empty cell between them, and both its legs
//! flagged undistorted, so seating, the text dump and the SVG all reported an adjacency the grid
//! did not hold. The reciprocated pair outranks a one-way exit (SQ-1287, SQ-1312), so it is the
//! one-ways that now yield and the one-ways that now draw bent.

use std::collections::BTreeSet;

use mapper::direction::{grid_offset, Direction};
use mapper::graph::MapGraph;
use mapper::layer::LayerId;

/// Clearing.
const CLEARING: u32 = 134;
/// The Forest south of it — the room all three claims land on.
const FOREST: u32 = 230;
/// South of House, which makes both of the one-way claims.
const SOUTH_OF_HOUSE: u32 = 217;
/// Canyon View, whose one-way `W` is exactly satisfied once the pair closes up.
const CANYON_VIEW: u32 = 23;

fn fixture() -> MapGraph {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../unit_tests/zork1_walked_map.json");
    let json = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("fixture {} must be readable: {e}", path.display()));
    mapper::persist::from_json(&json).expect("the fixture is a valid map file").graph
}

fn has(g: &MapGraph, origin: u32, dir: Direction, dest: u32) -> bool {
    g.connections().iter().any(|c| c.origin == origin && c.dir == dir && c.dest == dest)
}

fn distorted(g: &MapGraph, origin: u32, dir: Direction, dest: u32) -> bool {
    g.connections()
        .iter()
        .find(|c| c.origin == origin && c.dir == dir && c.dest == dest)
        .unwrap_or_else(|| panic!("the fixture must hold {origin} {dir:?} -> {dest}"))
        .distorted
}

/// What `/relayout` does, and what `app::mapgen::layout_all_layers` does: one solve per layer,
/// on the layer's own subgraph, with positions and distortion flags written back.
fn relayout_every_layer(g: &mut MapGraph) {
    let layers: Vec<LayerId> = g.layers().keys().copied().collect();
    for layer in layers {
        let mut sub = g.layer_subgraph(layer);
        mapper::layout::relayout_auto(&mut sub);
        for id in g.rooms_in_layer(layer) {
            if let Some(p) = sub.room(id).and_then(|r| r.pos) {
                g.set_pos(id, p);
            }
        }
        for idx in 0..g.connections().len() {
            let c = g.connections()[idx].clone();
            if g.layer_of(c.origin) == layer && g.layer_of(c.dest) == layer {
                if let Some(s) = sub
                    .connections()
                    .iter()
                    .find(|s| s.origin == c.origin && s.dir == c.dir && s.dest == c.dest)
                {
                    g.set_conn_distorted(idx, s.distorted);
                }
            }
        }
    }
}

/// The fixture really does hold the three-way conflict — a non-vacuity guard, because every
/// assertion below is about a SHAPE, and a fixture that quietly lost one of these edges would
/// pass the lot of them for the wrong reason.
#[test]
fn the_fixture_holds_the_three_way_claim_on_one_cell() {
    let g = fixture();
    assert!(
        has(&g, CLEARING, Direction::S, FOREST) && has(&g, FOREST, Direction::N, CLEARING),
        "the Clearing and the Forest are a reciprocated N/S pair",
    );
    assert!(
        has(&g, SOUTH_OF_HOUSE, Direction::S, FOREST) && !has(&g, FOREST, Direction::N, SOUTH_OF_HOUSE),
        "South of House's `S` into the Forest is ONE-WAY",
    );
    assert!(
        has(&g, FOREST, Direction::NW, SOUTH_OF_HOUSE)
            && !has(&g, SOUTH_OF_HOUSE, Direction::SE, FOREST),
        "the Forest's `NW` back to South of House is ONE-WAY, and answers an `S` with a diagonal",
    );
    assert!(
        has(&g, CANYON_VIEW, Direction::W, FOREST),
        "Canyon View's one-way `W` reaches the same Forest",
    );
    for id in [CLEARING, FOREST, SOUTH_OF_HOUSE, CANYON_VIEW] {
        assert_eq!(g.layer_of(id), 0, "all four rooms are on the above-ground layer");
    }
}

/// The reciprocal pair ends up ADJACENT, and the one-way pair is what bends.
///
/// Falsify by restoring the reciprocity filter in `layout::shift_is_legal`'s
/// `breaks_an_outside_bearing`: the Forest lands one row further down with the cell between it
/// and the Clearing empty.
#[test]
fn the_clearing_keeps_the_forest_on_its_doorstep() {
    let mut g = fixture();
    relayout_every_layer(&mut g);
    let p = |id: u32| g.room(id).unwrap().pos.unwrap();

    assert_eq!(
        (p(FOREST).0 - p(CLEARING).0, p(FOREST).1 - p(CLEARING).1),
        (0, 1),
        "the Forest sits in the cell directly south of the Clearing: {:?} vs {:?}",
        p(CLEARING),
        p(FOREST),
    );
    assert!(
        !distorted(&g, CLEARING, Direction::S, FOREST)
            && !distorted(&g, FOREST, Direction::N, CLEARING),
        "an honoured reciprocal pair is not distorted",
    );
    // Canyon View's `W` is satisfied for free once the Forest comes up a row.
    assert_eq!(
        (p(FOREST).0 - p(CANYON_VIEW).0, p(FOREST).1 - p(CANYON_VIEW).1),
        (-1, 0),
        "Canyon View's one-way W lands exactly: {:?} vs {:?}",
        p(CANYON_VIEW),
        p(FOREST),
    );
    assert!(
        !distorted(&g, CANYON_VIEW, Direction::W, FOREST),
        "…and so it is not distorted either",
    );
    // The two one-way claims on the same cell are the ones that gave, and they say so.
    assert!(
        distorted(&g, SOUTH_OF_HOUSE, Direction::S, FOREST),
        "the one-way S that lost must be drawn distorted",
    );
    assert!(
        distorted(&g, FOREST, Direction::NW, SOUTH_OF_HOUSE),
        "so must the diagonal that answered it",
    );
}

/// Every compass edge's flag agrees with the geometry it was written from.
///
/// The flag is the only thing a consumer has: `seat`'s `adjacent_reciprocals`, the text dump's
/// `align=col[…]` and the SVG's straight edge all read it and none of them re-derives the
/// geometry, so it must never contradict the positions it was computed from.
///
/// **RE-PINNED at SQ-1376.** SQ-1364 wrote this case to demand that a pair aligned on one column
/// with an empty cell between them draw DISTORTED; the user reversed that rule — a cardinal
/// names a line, not a cell count — so the same two above-ground pairs (each pinned by a run on
/// the perpendicular axis, which is why they cannot be brought together) are now honoured where
/// they stand, and the assertion below is inverted to say so. What has not changed is the point
/// of the case: the flag and the grid agree, and the map really does contain a stretched
/// cardinal, so the claim is not vacuous.
#[test]
fn no_compass_edge_claims_a_geometry_the_grid_does_not_hold() {
    let mut g = fixture();
    relayout_every_layer(&mut g);

    let mut lying = Vec::new();
    let mut stretched_cardinals = 0;
    for c in g.connections() {
        if c.is_self_loop() || grid_offset(c.dir).is_none() {
            continue;
        }
        if g.layer_of(c.origin) != g.layer_of(c.dest) {
            continue; // a cross-layer edge is nobody's geometry
        }
        if c.distorted != !mapper::layout::edge_is_satisfied(&g, c) {
            lying.push((c.origin, c.dir, c.dest, c.distorted));
        }
        let (Some(op), Some(dp)) = (
            g.room(c.origin).and_then(|r| r.pos),
            g.room(c.dest).and_then(|r| r.pos),
        ) else {
            continue;
        };
        let (dx, dy) = grid_offset(c.dir).unwrap();
        let a = (dp.0 - op.0, dp.1 - op.1);
        let aligned = (dx == 0 && a.0 == 0 && a.1.signum() == dy.signum())
            || (dy == 0 && a.1 == 0 && a.0.signum() == dx.signum());
        if aligned && a.0.abs().max(a.1.abs()) > 1 {
            stretched_cardinals += 1;
            assert!(
                !c.distorted,
                "{} {:?} -> {} is aligned {} cells away, which is honoured (SQ-1376)",
                c.origin,
                c.dir,
                c.dest,
                a.0.abs().max(a.1.abs()),
            );
        }
    }
    assert!(lying.is_empty(), "flags out of step with the final positions: {lying:?}");
    assert!(
        stretched_cardinals > 0,
        "this map really does contain stretched cardinals — without one the case is vacuous",
    );

    // …and the layout still places every room in its own cell.
    let cells: Vec<_> = g.rooms().filter_map(|r| r.pos.map(|p| (r.layer, p))).collect();
    let set: BTreeSet<_> = cells.iter().collect();
    assert_eq!(cells.len(), set.len(), "no two rooms share a cell on one layer");
}
