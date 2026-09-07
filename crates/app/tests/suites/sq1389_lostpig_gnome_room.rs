//! Lost Pig's gnome room keeps the free cell its two passages name (SQ-1389), and nothing on that
//! layer bends into an arrowhead (SQ-1390).
//!
//! The user, looking at the mapgen Lost Pig map's `Hole` layer: `(gnomeRoom) #194` was drawn at
//! `(1, -2)`, three rows north of where it belongs, wedged between `Statue Room #128` and
//! `Windy Cave #177` and separating the reciprocal N/S pair those two hold. All four of its edges
//! drew distorted, and the layer's own annotation said so:
//! `align=row[#147,#157,#194] dropped=[#194→E→#157, #194→W→#147]`.
//!
//! Its only passages are two reciprocal E/W pairs — `Table Room #147 ─E→ #194 ─E→ Shelf Room #157`,
//! walked from both ends — so exactly one cell satisfies it, and that cell was free.
//!
//! # Which stage moved it
//!
//! Traced per stage on the layer's own subgraph, `#194` is on `(1, 1)` after `seed`, after
//! `stress` and after `align`, and on `(1, -2)` after `contiguify`; the three app-side passes that
//! follow (`cleanup_overlaps`, `repair_directional_hints`, `cleanup_overlaps`) leave it there. So
//! it is not the tidy metric and not the crossing count — it is `mapper::layout::splits_a_run`
//! answering a question about a CELL with no idea who was standing on it, so every interior member
//! of every run was reported as splitting its own run. `#194` is the middle of
//! `[#147, #194, #157]`; `open_gated_holes_for_hubs` read "stuck", went looking for a gated link to
//! prise open, and found `Statue Room ─ Windy Cave` (`kind: routine`, so `Conditional`).
//!
//! `crates/mapper/tests/sq1389_run_member_is_not_an_interloper.rs` states the rule on a synthetic
//! reduction of this exact shape, so CI can fail on it. This suite is the specimen.
//!
//! The layer's LABELS have nothing to do with any of it: `(gnomeRoom)` is the Inform object
//! identifier, printed because the room's short name is set during play, and nothing under
//! `mapper::layout` or in the three tidy passes reads a label at all (`mentions_maze` does, but
//! only in mapgen's layer SPLIT, which put every one of these eight rooms on the same layer).
//!
//! Lost Pig lives under the gitignored `stories/`, so every case here skips vacuously off CI and
//! says so. Run `cargo nextest run -p lanthorn sq1389` against a checkout with `stories/`.

use std::path::{Path, PathBuf};

use mapper::graph::RoomId;
use mapper::layer::LayerId;

/// Lost Pig — the fixture the quest was reported on.
const LOSTPIG: &str = "LostPig.z8";

const FOUNTAIN: RoomId = 111; // the hub: W→#102, E→#166, N→#128, SW→#147, SE→#157
const STATUE: RoomId = 128;
const TABLE: RoomId = 147;
const SHELF: RoomId = 157;
const WINDY: RoomId = 177;
const GNOME: RoomId = 194;

fn story() -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(LOSTPIG);
    p.is_file().then_some(p)
}

/// The `Hole` layer, found by a room on it rather than by name or index — a layer's display name
/// is the user's to change and its id is an allocation order.
fn hole_layer(map: &app::mapgen::GeneratedMap) -> LayerId {
    map.graph.layer_of(FOUNTAIN)
}

fn cell(map: &app::mapgen::GeneratedMap, id: RoomId) -> (i32, i32) {
    map.graph.room(id).and_then(|r| r.pos).unwrap_or_else(|| panic!("#{id} must be placed"))
}

/// **The reported symptom, at the cell.** The gnome room sits between the `Table Room` and the
/// `Shelf Room`, one column from each and on their row — the only cell its two reciprocal pairs
/// allow.
///
/// The inequality names the exact cell the field report was looking at, so a future change that
/// moves it somewhere else entirely still has to come here and say what it did.
#[test]
fn the_gnome_room_stands_between_the_table_and_the_shelf() {
    let Some(path) = story() else {
        eprintln!("SKIP the_gnome_room_stands_between_the_table_and_the_shelf: stories/{LOSTPIG} absent");
        return;
    };
    let map = app::mapgen::generate(&path, true).expect("mapgen");
    let (table, shelf, gnome) = (cell(&map, TABLE), cell(&map, SHELF), cell(&map, GNOME));
    assert_eq!(
        gnome,
        (table.0 + 1, table.1),
        "gnomeRoom #194 at {gnome:?} is not one column east of Table Room #147 ({table:?})"
    );
    assert_eq!(
        gnome,
        (shelf.0 - 1, shelf.1),
        "…nor one column west of Shelf Room #157 ({shelf:?})"
    );
    assert_ne!(gnome, (table.0 + 1, table.1 - 3), "…and specifically not the reported (1, -2)");
}

/// **And nothing was traded for it.** The N/S pair the gnome room used to be dropped between is
/// adjacent again: `Windy Cave` is one cell north of `Statue Room`, on its column.
#[test]
fn the_statue_and_windy_pair_is_adjacent_again() {
    let Some(path) = story() else {
        eprintln!("SKIP the_statue_and_windy_pair_is_adjacent_again: stories/{LOSTPIG} absent");
        return;
    };
    let map = app::mapgen::generate(&path, true).expect("mapgen");
    let (statue, windy) = (cell(&map, STATUE), cell(&map, WINDY));
    assert_eq!(windy.0, statue.0, "Windy Cave #177 {windy:?} holds Statue Room #128's column {statue:?}");
    assert_eq!(windy.1, statue.1 - 1, "…and sits one row above it");
}

/// **No edge on the layer draws distorted.** Every passage on the `Hole` layer is drawable on the
/// grid, and the gnome room's four were not — which is the picture the report was of.
#[test]
fn no_edge_on_the_hole_layer_is_distorted() {
    let Some(path) = story() else {
        eprintln!("SKIP no_edge_on_the_hole_layer_is_distorted: stories/{LOSTPIG} absent");
        return;
    };
    let map = app::mapgen::generate(&path, true).expect("mapgen");
    let layer = hole_layer(&map);
    let name = |id: RoomId| {
        map.graph.room(id).map(|r| r.label().to_string()).unwrap_or_else(|| format!("#{id}"))
    };
    let bad: Vec<String> = map
        .graph
        .connections()
        .iter()
        .filter(|c| map.graph.layer_of(c.origin) == layer && map.graph.layer_of(c.dest) == layer)
        .filter(|c| c.distorted)
        .map(|c| format!("{} -{:?}-> {}", name(c.origin), c.dir, name(c.dest)))
        .collect();
    assert!(bad.is_empty(), "distorted on the Hole layer: {}", bad.join(", "));
    // Non-vacuity: the layer has to be the eight-room one the report is about.
    assert_eq!(map.graph.rooms_in_layer(layer).len(), 8, "the Hole layer holds eight rooms");
}

/// **`arrival_approach_report` over the whole Lost Pig map** (SQ-1390): no connector on any layer
/// turns in the cell touching an arrowhead, at either end of the line.
///
/// This is the story-only half of the rule; `render::map::sq1390_arrowhead_clearance` states it on
/// synthetic graphs so CI can fail on it, and `sq1316_connector_overlaps` states it over Zork I.
/// The `└◀` the user reported was on the `Hole` layer and was a DEPARTURE arrowhead — a reciprocal
/// pair is drawn once, from whichever room the router made the origin — so a report that read only
/// the arrival end could not see it, and did not.
#[test]
fn no_connector_bends_into_an_arrowhead_on_the_lost_pig_map() {
    let Some(path) = story() else {
        eprintln!("SKIP no_connector_bends_into_an_arrowhead_on_the_lost_pig_map: stories/{LOSTPIG} absent");
        return;
    };
    let map = app::mapgen::generate(&path, true).expect("mapgen");
    let mut layers: Vec<LayerId> = map
        .graph
        .layers()
        .keys()
        .copied()
        .filter(|&l| !map.graph.rooms_in_layer(l).is_empty())
        .collect();
    layers.sort_unstable();
    let (mut checked, mut excused) = (0usize, 0usize);
    let mut failures = Vec::new();
    for l in layers {
        let (n, e, jogs) = app::render::map::arrival_approach_report(&map.graph, l);
        checked += n;
        excused += e;
        for line in jogs {
            failures.push(format!("[{}] {line}", map.graph.layer_name(l)));
        }
    }
    assert!(
        failures.is_empty(),
        "{} connector(s) bend into an arrowhead on the Lost Pig map:\n{}",
        failures.len(),
        failures.join("\n")
    );
    // Non-vacuity. Lost Pig is a small map and every one of these six side approaches is on the
    // `Hole` layer — its other layers join their rooms by adjacent or corner connectors, which have
    // no side approach to measure. Pinned rather than guessed at: a drop below six means the filter
    // has started eating connectors, which is how a rule quietly stops being one.
    assert!(checked >= 6, "only {checked} approaches measured — the filter has eaten the map");
    assert_eq!(excused, 0, "no jog on the Lost Pig map needs the crowded-side exemption");
}
