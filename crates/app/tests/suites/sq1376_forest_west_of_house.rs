//! `Forest #91` is west of `West of House #68` on the generated Zork I map (SQ-1376).
//!
//! The user, looking at the mapgen Zork I map: *"room 91 should be west of 68. that relationship
//! is getting lost somewhere (it works live)"*. It was, and the parenthesis was the clue.
//!
//! # What was actually wrong — two things, and the second is the one nobody had noticed
//!
//! **The layout breaks bearings on purpose, and mapgen was not running the pass that puts them
//! back.** `relayout_auto`'s contiguity stage (`mapper::layout::contiguify`) deliberately trades a
//! bearing for a tight row: it slides a run to open a hole for a hub, then closes the run's own
//! gaps. On Zork I the stress solve had already placed `Forest #91` correctly — due west of
//! `Forest Path #247` on one row, and west of `West of House #68` — and contiguity then moved the
//! house one column west and the forest one column east, which put the forest on the WRONG SIDE of
//! the room whose `W` exit names it.
//!
//! The live map recovers from that, because `tidy::tidy_layer_silent` runs five stages:
//! `relayout_auto`, `cleanup_overlaps`, **`repair_directional_hints`**, `cleanup_overlaps`,
//! `compact_empty_lines`. `mapgen::layout_all_layers` ran the first one alone. That is the whole of
//! "it works live": same solver, four fewer stages. mapgen now runs all five.
//!
//! **And a cardinal bearing was being read as a cell count.** With the forest four cells due west
//! of the Forest Path rather than one, SQ-1364's `edge_is_satisfied` called that pair DISTORTED —
//! so the picture the user approved would have been drawn as a bent red line. The user's rule
//! replaces it: a cardinal names a LINE, and how far along it the far room sits is the layout's
//! business (`mapper::layout::edge_is_satisfied`, and the synthetic per-axis cases in
//! `crates/mapper/tests/sq1376_cardinal_line.rs`).
//!
//! A third fix rides along, in the solve rather than in mapgen: `align_free_axes` now asks a
//! RECIPROCATED cardinal partner before a one-way one when deciding whose row a free room takes
//! (`mapper::layout::sort`). It does not move anything on this map — Zork I's forest was already
//! on the right row when contiguity got hold of it — but it is the same rule one stage earlier,
//! and the synthetic row case in the mapper crate fails without it.
//!
//! Falsified: with `edge_is_satisfied`'s SQ-1364 adjacency clause restored AND the four extra
//! stages removed from `layout_all_layers`, [`the_forest_lies_west_of_west_of_house`] fails with
//! `Forest #91` at `(0, 0)` against `West of House #68` at `(-1, 3)` — the field symptom, to the
//! cell.
//!
//! Zork I lives under the gitignored `stories/`, so every case here skips vacuously off CI and
//! says so. Run `cargo nextest run -p lanthorn sq1376` against a checkout with `stories/`.

use std::path::{Path, PathBuf};

use mapper::graph::RoomId;

/// Zork I release 52 / serial 871125 — the InvisiClues edition, the fixture the quest was
/// reported on and the one `docs/zork1-map.svg` is generated from.
const ZORK1: &str = "zork1-invclues-r52-s871125.z5";

/// `Forest`, the room `West of House`'s own `W` exit leads to. Its `E` leads to `Forest Path`,
/// not back to the house, so `#68 W #91` is one-way in effect.
const FOREST: RoomId = 91;
/// `Forest Path` — reciprocally east of `Forest #91` (`#91 E #247` / `#247 W #91`).
const FOREST_PATH: RoomId = 247;
/// `West of House`, where play begins.
const WEST_OF_HOUSE: RoomId = 68;
/// `Living Room`, immediately east of `West of House` on the house row.
const LIVING_ROOM: RoomId = 79;
/// `Kitchen`, whose row the house sits on and whose doorstep the `Attic` takes (SQ-1375).
const KITCHEN: RoomId = 28;
/// `North of House` and `Clearing`, the other two rooms of `Forest Path`'s column.
const NORTH_OF_HOUSE: RoomId = 143;
const CLEARING: RoomId = 167;

fn story() -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(ZORK1);
    p.is_file().then_some(p)
}

/// Every room this suite names, at the cell mapgen gave it. Panics if any is unplaced, which is
/// the non-vacuity guard: a run that measured a map missing these rooms measured nothing.
fn cells(map: &app::mapgen::GeneratedMap) -> std::collections::BTreeMap<RoomId, (i32, i32)> {
    [FOREST, FOREST_PATH, WEST_OF_HOUSE, LIVING_ROOM, KITCHEN, NORTH_OF_HOUSE, CLEARING]
        .into_iter()
        .map(|id| {
            let p = map
                .graph
                .room(id)
                .and_then(|r| r.pos)
                .unwrap_or_else(|| panic!("non-vacuity: room #{id} must be on the generated map"));
            (id, p)
        })
        .collect()
}

/// **The reported symptom, at the cell.** `Forest #91` sits in a column strictly WEST of
/// `West of House #68`, and on `Forest Path`'s own row.
///
/// Both halves are the user's requirement. The row is what makes `#91 E #247` a straight line; the
/// column is the bearing the report was about. The inequality is deliberately loose — how far west
/// is the layout's business, and pinning the exact cell would fail on any harmless re-tidy — but
/// the reported cell is named explicitly below it so a future change that puts the forest back
/// east has to come here and say so.
#[test]
fn the_forest_lies_west_of_west_of_house() {
    let Some(path) = story() else {
        eprintln!("SKIP the_forest_lies_west_of_west_of_house: stories/{ZORK1} absent");
        return;
    };
    let map = app::mapgen::generate(&path, true).expect("mapgen");
    let c = cells(&map);
    let (forest, house, path_room) = (c[&FOREST], c[&WEST_OF_HOUSE], c[&FOREST_PATH]);

    assert!(
        forest.0 < house.0,
        "Forest #91 at {forest:?} must be WEST of West of House #68 at {house:?}"
    );
    assert_ne!(
        forest,
        (house.0 + 1, 0),
        "…and specifically not the reported cell, one column EAST of the house"
    );
    assert_eq!(
        forest.1, path_room.1,
        "Forest #91 at {forest:?} shares Forest Path #247's row {path_room:?}"
    );
    assert!(
        forest.0 < path_room.0,
        "Forest #91 at {forest:?} is west of Forest Path #247 at {path_room:?}"
    );
}

/// **The pair is honoured as a straight line however long it is.** `#91 E #247` and `#247 W #91`
/// are a passage walked from both ends; they come out four cells apart on one row, and neither leg
/// is drawn distorted.
///
/// This is SQ-1376's definition (`mapper::layout::edge_is_satisfied`) read off a real map rather
/// than a synthetic one. Under SQ-1364's rule both legs would be flagged, and the picture the user
/// approved — *"a plain straight three-cell edge reciprocal"* — would be a bent red line instead.
#[test]
fn the_forest_path_pair_is_a_plain_straight_line() {
    let Some(path) = story() else {
        eprintln!("SKIP the_forest_path_pair_is_a_plain_straight_line: stories/{ZORK1} absent");
        return;
    };
    let map = app::mapgen::generate(&path, true).expect("mapgen");
    let c = cells(&map);
    let gap = (c[&FOREST_PATH].0 - c[&FOREST].0).abs();
    assert!(
        gap > 1,
        "non-vacuity: the pair must actually be STRETCHED for this case to mean anything, got {gap}"
    );
    let mut legs = 0;
    for conn in map.graph.connections() {
        let is_leg = (conn.origin, conn.dest) == (FOREST, FOREST_PATH)
            || (conn.origin, conn.dest) == (FOREST_PATH, FOREST);
        if !is_leg || mapper::direction::grid_offset(conn.dir).is_none() {
            continue;
        }
        legs += 1;
        assert!(
            !conn.distorted,
            "#{} {:?} #{} is aligned {gap} cells away and must draw plain (SQ-1376)",
            conn.origin, conn.dir, conn.dest,
        );
        assert!(
            mapper::layout::edge_is_satisfied(&map.graph, conn),
            "…and `edge_is_satisfied` must agree with the flag on #{} {:?} #{}",
            conn.origin,
            conn.dir,
            conn.dest,
        );
    }
    assert_eq!(legs, 2, "non-vacuity: both legs of the walked-both-ways pair must be on the map");
}

/// **The rest of the house is where SQ-1375 left it.** The three rooms of `Forest Path`'s column
/// stay in one column, and `West of House` stays west of `Living Room` on the `Kitchen`'s row.
///
/// The user asked for all three by name alongside the fix, because a layout change that buys one
/// bearing with three others is not a fix. `sq1375_zork_house_makes_room` holds the `Attic` and the
/// `Studio` ghost; this holds the rest of the shape around them.
#[test]
fn the_house_and_the_forest_path_column_keep_their_shape() {
    let Some(path) = story() else {
        eprintln!("SKIP the_house_and_the_forest_path_column_keep_their_shape: stories/{ZORK1} absent");
        return;
    };
    let map = app::mapgen::generate(&path, true).expect("mapgen");
    let c = cells(&map);

    let column = [c[&CLEARING], c[&FOREST_PATH], c[&NORTH_OF_HOUSE]];
    assert!(
        column.iter().all(|p| p.0 == column[0].0),
        "Clearing #167, Forest Path #247 and North of House #143 share one column: {column:?}"
    );
    assert!(
        column[0].1 < column[1].1 && column[1].1 < column[2].1,
        "…in that order, north to south: {column:?}"
    );

    let (house, living, kitchen) = (c[&WEST_OF_HOUSE], c[&LIVING_ROOM], c[&KITCHEN]);
    assert_eq!(house.1, kitchen.1, "West of House #68 {house:?} is on the Kitchen's row {kitchen:?}");
    assert_eq!(living.1, kitchen.1, "…and so is Living Room #79 {living:?}");
    assert!(
        house.0 < living.0,
        "West of House #68 {house:?} stays WEST of Living Room #79 {living:?}"
    );
}
