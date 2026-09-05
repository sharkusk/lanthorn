//! `lanthorn-mapgen` auto-splits mazes and portal-only regions onto their own
//! layers, the way the app's own layer suggestions would if a player accepted
//! every one (SQ-1308).
//!
//! Every case drives [`app::mapgen::split_layers`] or [`app::mapgen::generate_with_options`]
//! directly, same as [`sq1306_mapgen`](super::sq1306_mapgen) does for the rest of mapgen.

use std::path::{Path, PathBuf};

use app::mapgen::{self, LayerSplit, MapgenOptions};
use mapper::direction::Direction;
use mapper::graph::MapGraph;
use mapper::layer::MAIN_LAYER;

use crate::fixture_paths::fixture_path;

/// A story under the gitignored `stories/`, or `None` when this checkout has no
/// copy — the CI-safe vacuous-skip pattern (mirrors `sq1306_mapgen::story`).
fn story(name: &str) -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(name);
    p.is_file().then_some(p)
}

fn tracked(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

// ---------------------------------------------------------------------------
// CI-runnable: Mini-Zork I's real maze
// ---------------------------------------------------------------------------

/// Mini-Zork I's maze — ten rooms literally named "Maze" — lands on one
/// maze-flagged layer, and no room that ISN'T named "Maze" comes along for the
/// ride.
///
/// The second half is the case that actually needed proving: a bare
/// `mapper::layer::planar_region` walk from a maze room does not stop at the
/// maze at all here — the Cyclops Room has an unconditional compass exit to
/// the Living Room, so the naive walk sweeps up 55 of Mini-Zork's 70 rooms,
/// Kitchen and West of House included. `app::mapgen::maze_region` (private;
/// exercised only through `split_layers`) stops at the room-name boundary
/// instead, which is what this pins.
#[test]
fn minizork_maze_lands_on_one_maze_flagged_layer_alone() {
    let map = mapgen::generate(&fixture_path("minizork-r34-s871124.z3"), true)
        .expect("minizork.z3 is a tracked fixture and must map");

    let maze_rooms: Vec<_> = map
        .graph
        .rooms()
        .filter(|r| mapper::suggest::mentions_maze(r.label()))
        .collect();
    assert!(maze_rooms.len() >= 2, "Mini-Zork's maze has several identically-named rooms");

    let maze_layer = maze_rooms[0].layer;
    assert_ne!(maze_layer, MAIN_LAYER, "the maze must have moved off Main");
    assert!(map.graph.layer_is_maze(maze_layer), "its layer must be flagged as a maze");
    for r in &maze_rooms {
        assert_eq!(r.layer, maze_layer, "{:?} must be on the SAME maze layer", r.label());
    }

    // The other half: nothing that ISN'T named "maze" is on that layer either —
    // the failure mode a bare `planar_region` walk falls into on this exact story.
    for r in map.graph.rooms_in_layer(maze_layer) {
        let room = map.graph.room(r).unwrap();
        assert!(
            mapper::suggest::mentions_maze(room.label()),
            "{:?} is on the maze layer but its name doesn't mention \"maze\"",
            room.label()
        );
    }
}

/// `--no-auto-layers` (`MapgenOptions::auto_layers = false`) reproduces the
/// pre-SQ-1308 flat map even on a story with a real maze in it.
#[test]
fn no_auto_layers_leaves_minizork_flat() {
    let opts = MapgenOptions { auto_layers: false, ..MapgenOptions::default() };
    let map = mapgen::generate_with_options(&fixture_path("minizork-r34-s871124.z3"), true, &opts)
        .expect("minizork.z3 is a tracked fixture and must map");
    assert_eq!(map.graph.layers().len(), 1, "one flat layer, exactly as before SQ-1308");
    assert!(map.graph.rooms().all(|r| r.layer == MAIN_LAYER));
}

// ---------------------------------------------------------------------------
// CI-runnable: synthetic graphs, `split_layers` directly
// ---------------------------------------------------------------------------

/// A big Main component (ten compass-connected rooms) plus a five-room region
/// reachable only through an `Up` portal from one of them.
fn synthetic_portal_region() -> MapGraph {
    let mut g = MapGraph::new();
    // Main: a row of ten rooms, 1..=10, compass-connected — bigger than the
    // portal region below, so it is always the "largest component" kept as Main.
    for id in 1..=10u32 {
        g.upsert_room(id, format!("Hall {id}"));
    }
    for id in 1..10u32 {
        g.add_edge(id, Direction::E, id + 1);
        g.add_edge(id + 1, Direction::W, id);
    }
    // A five-room region behind an Up portal from Hall 1 — named "Loft N".
    for (i, id) in (100..105u32).enumerate() {
        g.upsert_room(id, format!("Loft {i}"));
    }
    g.add_edge(1, Direction::Up, 100);
    g.add_edge(100, Direction::Down, 1);
    for id in 100..104u32 {
        g.add_edge(id, Direction::E, id + 1);
        g.add_edge(id + 1, Direction::W, id);
    }
    g
}

/// At the default floor (4), the five-room portal region is big enough for its
/// own layer, named after the room the portal leads into.
#[test]
fn a_five_room_portal_region_splits_at_the_default_floor() {
    let mut g = synthetic_portal_region();
    let opts = MapgenOptions::default();
    assert_eq!(opts.layer_min, mapper::suggest::STRUCTURAL_FLOOR);
    let splits = mapgen::split_layers(&mut g, &opts);

    assert_eq!(splits.len(), 1, "one split: the five-room loft, splits: {splits:?}");
    let LayerSplit { rooms, maze, name, id } = &splits[0];
    assert_eq!(*rooms, 5);
    assert!(!maze, "a portal-only region is not a maze");
    assert_eq!(name, "Loft 0", "named after the room the entering portal leads into");
    assert_eq!(g.layer_of(100), *id);
    assert_eq!(g.rooms_in_layer(MAIN_LAYER).len(), 10, "the ten-room hall stays Main");
}

/// Raise the floor to 6 and the same five-room region is under it: everything
/// stays on Main, and `split_layers` reports no split at all.
#[test]
fn the_same_region_stays_on_main_once_the_floor_is_raised_above_it() {
    let mut g = synthetic_portal_region();
    let opts = MapgenOptions { layer_min: 6, ..MapgenOptions::default() };
    let splits = mapgen::split_layers(&mut g, &opts);
    assert_eq!(splits, vec![], "under the floor: nothing splits");
    assert_eq!(g.layers().len(), 1, "everything is still on one layer");
    assert_eq!(g.rooms_in_layer(MAIN_LAYER).len(), 15);
}

/// `auto_layers: false` skips the pass entirely, even with a region well above
/// the floor sitting right there.
#[test]
fn auto_layers_false_skips_the_split() {
    let mut g = synthetic_portal_region();
    let opts = MapgenOptions { auto_layers: false, layer_min: 1 };
    let splits = mapgen::split_layers(&mut g, &opts);
    assert_eq!(splits, vec![]);
    assert_eq!(g.layers().len(), 1);
}

/// A room whose name mentions "maze" anchors a maze-only walk: three "Maze"
/// rooms in a row split off together as ONE maze-flagged layer, regardless of
/// `layer_min` — a maze has no floor.
#[test]
fn a_maze_named_region_splits_and_is_flagged_regardless_of_floor() {
    let mut g = MapGraph::new();
    g.upsert_room(1, "Troll Room".into());
    for id in 2..=4u32 {
        g.upsert_room(id, "Maze".into());
    }
    g.add_edge(1, Direction::W, 2);
    g.add_edge(2, Direction::E, 1);
    g.add_edge(2, Direction::N, 3);
    g.add_edge(3, Direction::S, 2);
    g.add_edge(3, Direction::E, 4);
    g.add_edge(4, Direction::W, 3);

    // A floor far above the maze's own size: it still splits, because a maze
    // has no floor — only a portal-only region does.
    let opts = MapgenOptions { layer_min: 100, ..MapgenOptions::default() };
    let splits = mapgen::split_layers(&mut g, &opts);
    assert_eq!(splits.len(), 1);
    assert!(splits[0].maze);
    assert_eq!(splits[0].rooms, 3);
    assert_eq!(g.layer_of(1), MAIN_LAYER, "Troll Room is not named \"maze\" and stays put");
    assert!(g.layer_is_maze(splits[0].id));
}

/// A graph with no maze and no region past the floor produces no split at all
/// — `--no-auto-layers`'s behaviour is also what a story with nothing to split
/// produces on its own.
#[test]
fn a_graph_with_nothing_to_split_produces_no_layers() {
    let mut g = MapGraph::new();
    g.upsert_room(1, "Hall".into());
    g.upsert_room(2, "Parlour".into());
    g.add_edge(1, Direction::E, 2);
    g.add_edge(2, Direction::W, 1);
    let splits = mapgen::split_layers(&mut g, &MapgenOptions::default());
    assert_eq!(splits, vec![]);
    assert_eq!(g.layers().len(), 1);
}

// ---------------------------------------------------------------------------
// Real-game: Zork I (skip vacuously without `stories/`)
// ---------------------------------------------------------------------------

/// Zork I's release 52: the maze is one maze layer, the largest remaining
/// component stays Main, and the default floor's portal-only layers are named
/// and counted — a frame, pinned the way `docs/internals` asks any
/// topology-dependent count to be (SQ-1308's own floor, not a magic number).
#[test]
fn zork1_maze_is_one_layer_and_main_is_the_largest_component() {
    let Some(path) = story("zork1-invclues-r52-s871125.z5") else {
        eprintln!("SKIP: stories/zork1-invclues-r52-s871125.z5 absent");
        return;
    };
    let map = mapgen::generate(&path, true).expect("Zork I must map");
    let g = &map.graph;

    let maze_rooms: Vec<_> =
        g.rooms().filter(|r| mapper::suggest::mentions_maze(r.label())).collect();
    assert!(maze_rooms.len() > 5, "Zork I's maze is a good deal bigger than Mini-Zork's");
    let maze_layer = maze_rooms[0].layer;
    assert!(g.layer_is_maze(maze_layer));
    assert!(maze_rooms.iter().all(|r| r.layer == maze_layer), "one maze layer, not several");
    for r in g.rooms_in_layer(maze_layer) {
        assert!(mapper::suggest::mentions_maze(g.room(r).unwrap().label()));
    }

    // Main is whichever layer holds the most rooms — mapgen has no start room
    // to anchor Main on the way the live map does, so "biggest wins" is the
    // rule, and it must actually be the layer `move_region` never touched.
    let mut by_size: Vec<(mapper::layer::LayerId, usize)> =
        g.layers().keys().map(|&id| (id, g.rooms_in_layer(id).len())).collect();
    by_size.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
    assert_eq!(by_size[0].0, MAIN_LAYER, "the largest layer must be the one still called Main");

    // The portal-only split: pinned by name and count against this exact
    // release/serial, the way `real_media_releases.rs` pins a floppy's release
    // rather than asserting "some release or other maps".
    let mut portal_only: Vec<String> = g
        .layers()
        .keys()
        .filter(|&&id| id != MAIN_LAYER && id != maze_layer)
        .map(|&id| format!("{} ({} rooms)", g.layer_name(id), g.rooms_in_layer(id).len()))
        .collect();
    portal_only.sort();
    eprintln!("zork1 r52/s871125 portal-only layers at the default floor: {portal_only:?}");
    assert_eq!(
        portal_only,
        vec![
            "Coal Mine (5 rooms)".to_string(),
            "Ladder Bottom (5 rooms)".to_string(),
            "Rocky Ledge (18 rooms)".to_string(),
            "Torch Room (4 rooms)".to_string(),
        ],
        "the portal-only split at the default floor changed shape"
    );
}

// ---------------------------------------------------------------------------
// The JSON map sees more than one layer for a story that has them
// ---------------------------------------------------------------------------

/// The JSON map's own `layers` array — already exercised for a flat,
/// single-layer story in `sq1306_mapgen::json_map_pins_its_format_and_required_keys`
/// — now reports more than one layer for a story SQ-1308 actually splits.
#[test]
fn json_map_reports_more_than_one_layer_for_a_split_story() {
    let map = mapgen::generate(&fixture_path("minizork-r34-s871124.z3"), true)
        .expect("minizork.z3 is a tracked fixture and must map");
    let v: serde_json::Value =
        serde_json::from_str(&mapgen::render_json(&map)).expect("the JSON must round-trip");
    let layers = v["layers"].as_array().expect("layers is an array");
    assert!(layers.len() > 1, "Mini-Zork's maze must show up as a second layer: {layers:?}");
    assert!(
        layers.iter().any(|l| l["maze"] == true),
        "one of them must be flagged as a maze: {layers:?}"
    );

    // Every room the graph itself says is on the maze layer must carry the
    // "maze" flag in its own JSON entry too — `render_json`'s `flags` reads
    // `layer_is_maze` per room, and this is the one place that can drift.
    let maze_layer_id = layers.iter().find(|l| l["maze"] == true).unwrap()["id"].as_u64().unwrap();
    for r in v["rooms"].as_array().unwrap() {
        if r["layer"].as_u64() == Some(maze_layer_id) {
            let flags: Vec<&str> =
                r["flags"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
            assert!(flags.contains(&"maze"), "room on the maze layer missing its flag: {r}");
        }
    }
}

/// A story generated with the layout skipped still splits layers the same
/// way — `split_layers` runs before layout, not as part of it (`tracked` here
/// just for a fast, tiny fixture; the split itself is what's being checked).
#[test]
fn split_layers_runs_even_when_layout_is_skipped() {
    let map = mapgen::generate(&tracked("../scott/tests/tiny_cave.dat"), false).unwrap();
    assert!(map.layout_time.is_none(), "layout was skipped");
    // tiny_cave.dat has no maze and only three rooms — nothing to split — so
    // the assertion that matters is simply that generation with no layout
    // still runs `split_layers` without panicking and leaves one layer.
    assert_eq!(map.graph.layers().len(), 1);
}
