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
/// maze-flagged layer, alongside only the Grating Room (SQ-1311: absorbed
/// because every one of ITS compass edges leads into the maze too), and no
/// room reachable any other way comes along for the ride.
///
/// The second half is the case that actually needed proving: a bare
/// `mapper::layer::planar_region` walk from a maze room does not stop at the
/// maze at all here — the Cyclops Room has an unconditional compass exit to
/// the Living Room, so the naive walk sweeps up 55 of Mini-Zork's 70 rooms,
/// Kitchen and West of House included. `app::mapgen::maze_region` (private;
/// exercised only through `split_layers`) stops at the room-name boundary
/// instead, which is what this pins — SQ-1311's absorb pass recovers the
/// Grating Room afterward, but by its own compass edges alone, never by
/// sweeping up an unrelated hub the way the naive walk does.
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

    // The other half: every room on the layer is either named "maze" or is the
    // one SQ-1311 absorbed onto it by its own compass edges (the Grating
    // Room) — never one of the rooms the naive `planar_region` walk would
    // wrongly sweep up (Kitchen, Living Room, West of House and the rest of
    // the surface, none of which have every edge leading into the maze).
    for r in map.graph.rooms_in_layer(maze_layer) {
        let room = map.graph.room(r).unwrap();
        assert!(
            mapper::suggest::mentions_maze(room.label()) || room.label() == "Grating Room",
            "{:?} is on the maze layer but its name doesn't mention \"maze\" and it isn't the Grating Room",
            room.label()
        );
    }
    for surface in ["Kitchen", "Living Room", "West of House", "Cyclops Room"] {
        let r = map.graph.rooms().find(|r| r.label() == surface).expect(surface);
        assert_ne!(r.layer, maze_layer, "{surface} must not be swept onto the maze layer");
    }
}

/// SQ-1311: Mini-Zork's maze has no room literally named "Dead End", but it
/// does have a Grating Room — reached from the maze by a reciprocal compass
/// pair (SW/NE) and from the surface only by a portal (`Up`) through the
/// grating door — which must join the maze layer for the same reason a dead
/// end would, and must NOT be pulled in by `maze_region`'s own name-bounded
/// walk (its name never mentions "maze").
#[test]
fn minizork_grating_room_joins_the_maze_by_its_compass_edge_alone() {
    let map = mapgen::generate(&fixture_path("minizork-r34-s871124.z3"), true)
        .expect("minizork.z3 is a tracked fixture and must map");
    let g = &map.graph;

    let grating = g
        .rooms()
        .find(|r| r.label() == "Grating Room")
        .expect("Mini-Zork has a Grating Room");
    assert!(
        !mapper::suggest::mentions_maze(grating.label()),
        "sanity: the Grating Room's own name never mentions \"maze\""
    );

    let maze_layer = g
        .rooms()
        .find(|r| mapper::suggest::mentions_maze(r.label()))
        .expect("Mini-Zork has a maze")
        .layer;
    assert_eq!(grating.layer, maze_layer, "the Grating Room must join the maze layer");
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

/// SQ-1310: a one-room Attic hanging `Up` off Loft 3 (part of the five-room
/// portal region, big enough for its own layer) must land on THAT layer, not
/// stranded on Main the way pass 2 alone would leave it — pass 2 never sees
/// past Main, so a component below the floor defaults there regardless of
/// which OTHER layer it actually opens onto.
#[test]
fn a_below_floor_component_adopts_the_layer_of_the_region_it_opens_onto() {
    let mut g = synthetic_portal_region();
    g.upsert_room(200, "Attic".into());
    g.add_edge(103, Direction::Up, 200);
    g.add_edge(200, Direction::Down, 103);
    // A second singleton with NO portal at all: nothing to adopt onto, so it
    // must stay on Main exactly as pass 2 already leaves it.
    g.upsert_room(201, "Nowhere".into());

    let opts = MapgenOptions::default();
    mapgen::split_layers(&mut g, &opts);

    let loft_layer = g.layer_of(100);
    assert_ne!(loft_layer, MAIN_LAYER, "sanity: the loft region got its own layer");
    assert_eq!(g.layer_of(200), loft_layer, "the Attic must join the loft's layer, not stay on Main");
    assert_eq!(g.layer_of(201), MAIN_LAYER, "a portal-less singleton has nothing to adopt onto: stays Main");
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
    g.upsert_room(5, "Round Room".into());
    for id in 2..=4u32 {
        g.upsert_room(id, "Maze".into());
    }
    g.add_edge(1, Direction::W, 2);
    g.add_edge(2, Direction::E, 1);
    g.add_edge(2, Direction::N, 3);
    g.add_edge(3, Direction::S, 2);
    g.add_edge(3, Direction::E, 4);
    g.add_edge(4, Direction::W, 3);
    // Troll Room also has a compass edge to a non-maze room (SQ-1311's absorb
    // pass would otherwise pull a room with only ONE edge — the maze one — in
    // alongside it; a real Troll Room is a hub with many non-maze edges too).
    g.add_edge(1, Direction::E, 5);
    g.add_edge(5, Direction::W, 1);

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

/// SQ-1311: a "Dead End" hanging off a maze by a single reciprocal compass
/// pair joins the maze layer even though its own name never mentions "maze" —
/// `maze_region`'s walk stops at the name (the Cyclops Room protection), so
/// this has to be recovered afterward by `absorb_maze_adjacent_rooms`.
#[test]
fn a_dead_end_off_a_maze_joins_the_maze_layer() {
    let mut g = MapGraph::new();
    g.upsert_room(1, "Maze".into());
    g.upsert_room(2, "Maze".into());
    g.upsert_room(3, "Maze".into());
    g.upsert_room(4, "Dead End".into());
    g.add_edge(1, Direction::E, 2);
    g.add_edge(2, Direction::W, 1);
    g.add_edge(2, Direction::N, 3);
    g.add_edge(3, Direction::S, 2);
    // The dead end hangs off room 3 by a reciprocal compass pair — its ONLY edge.
    g.add_edge(3, Direction::E, 4);
    g.add_edge(4, Direction::W, 3);

    let splits = mapgen::split_layers(&mut g, &MapgenOptions::default());
    assert_eq!(splits.len(), 1, "one maze layer, splits: {splits:?}");
    let maze_layer = splits[0].id;
    assert!(splits[0].maze);
    assert_eq!(splits[0].rooms, 4, "the dead end must be counted on the maze layer");
    assert_eq!(g.layer_of(4), maze_layer, "the dead end must land on the maze layer");
}

/// The Cyclops Room protection extends to the absorb pass too: a "Dead End"
/// with even ONE compass edge to a non-maze room must stay off the maze layer,
/// exactly as `maze_region`'s own walk already refuses to cross into one.
#[test]
fn a_dead_end_with_a_non_maze_compass_edge_is_not_absorbed() {
    let mut g = MapGraph::new();
    g.upsert_room(1, "Maze".into());
    g.upsert_room(2, "Maze".into());
    g.upsert_room(3, "Maze".into());
    g.upsert_room(4, "Dead End".into());
    g.upsert_room(5, "Cellar".into());
    g.add_edge(1, Direction::E, 2);
    g.add_edge(2, Direction::W, 1);
    g.add_edge(2, Direction::N, 3);
    g.add_edge(3, Direction::S, 2);
    // The "dead end" has a compass edge into the maze AND one to a plain room.
    g.add_edge(3, Direction::E, 4);
    g.add_edge(4, Direction::W, 3);
    g.add_edge(4, Direction::N, 5);
    g.add_edge(5, Direction::S, 4);

    let splits = mapgen::split_layers(&mut g, &MapgenOptions::default());
    assert_eq!(splits.len(), 1, "one maze layer, splits: {splits:?}");
    assert_eq!(splits[0].rooms, 3, "the dead end must NOT be counted on the maze layer");
    assert_eq!(g.layer_of(4), MAIN_LAYER, "a compass edge to a non-maze room keeps it off the maze layer");
    assert_eq!(g.layer_of(5), MAIN_LAYER, "Cellar was never a candidate and stays put");
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

    // SQ-1311: the maze layer is no longer EVERY room named "maze" and NOTHING
    // else — `absorb_maze_adjacent_rooms` also pulls in a dead end or a Grating
    // Room whose every compass edge leads only into this layer, so the room
    // set here is a SUPERSET of the maze-named rooms rather than identical to
    // them. Pin the absorbed rooms by their Z-machine object numbers (several
    // share the printed name "Dead End", so a name lookup can't tell them
    // apart): #148/#150/#154/#160 are the maze's own dead ends and #225 is the
    // Grating Room, all reached ONLY by a compass edge into the maze; #163 is
    // a DIFFERENT "Dead End" at the bottom of the Ladder Bottom mine shaft,
    // with no compass edge to the maze at all, and must stay off this layer.
    for &absorbed in &[148u32, 150, 154, 160, 225] {
        assert_eq!(
            g.layer_of(absorbed),
            maze_layer,
            "#{absorbed} has every compass edge into the maze and must join its layer"
        );
    }
    assert_ne!(
        g.layer_of(163),
        maze_layer,
        "#163 (Ladder Bottom's own dead end) has no compass edge to the maze and must stay off it"
    );

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
    //
    // Re-pinned for SQ-1310: Coal Mine gained Ladder Top (5→6) and Rocky Ledge
    // gained Attic, Up a Tree and the Grating Room (18→21) — three one-room dead
    // ends reached only by a portal (`Up`/`Down`) from a room already on that
    // layer. Before SQ-1310's `adopt_stranded_regions`, pass 2 had no way to see
    // past Main, so every one of those below-floor singletons defaulted there
    // regardless of which layer it actually opened onto — an attic reached by
    // climbing UP from the Kitchen was landing on the SAME layer as the far end
    // of the Coal Mine. Verified by reverting `adopt_stranded_regions`'s call
    // site: the counts go back to 5/5/18/4 and Main gains the four rooms back.
    //
    // Re-pinned again for SQ-1311: Rocky Ledge loses the Grating Room (21→20) —
    // it now joins the maze directly in pass 1b (`absorb_maze_adjacent_rooms`),
    // every one of its compass edges leading into the maze, so it never reaches
    // pass 3's portal-adoption at all. Main also loses five rooms (its own four
    // maze dead ends plus the now-excluded pseudo-room #41), but Main is not
    // pinned here by count — only the portal-only layers are.
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
            "Coal Mine (6 rooms)".to_string(),
            "Ladder Bottom (5 rooms)".to_string(),
            "Rocky Ledge (21 rooms)".to_string(),
            "Torch Room (4 rooms)".to_string(),
        ],
        "the portal-only split at the default floor changed shape"
    );

    // SQ-1310's own named cases: the Attic (off the Kitchen) and Up a Tree (off
    // the Forest Path) must land on Rocky Ledge — the house's own layer — not on
    // Main by default.
    let rocky_ledge = room_id(g, "Kitchen");
    let rocky_ledge_layer = g.layer_of(rocky_ledge);
    assert_ne!(rocky_ledge_layer, MAIN_LAYER, "sanity: the Kitchen is on its own portal layer");
    assert_eq!(g.layer_of(room_id(g, "Attic")), rocky_ledge_layer, "the Attic must join the Kitchen's layer");
    assert_eq!(
        g.layer_of(room_id(g, "Up a Tree")),
        rocky_ledge_layer,
        "Up a Tree must join the Forest Path's layer"
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

// ---------------------------------------------------------------------------
// SQ-1309: each layer must be laid out independently of every other layer
// ---------------------------------------------------------------------------

/// The room in `graph` whose label is exactly `label`. Ids, not names, are what a
/// reader assigns, so a test that wants to pin GEOMETRY names rooms this way rather
/// than hardcoding an id.
fn room_id(graph: &MapGraph, label: &str) -> mapper::graph::RoomId {
    graph
        .rooms()
        .find(|r| r.label() == label)
        .unwrap_or_else(|| panic!("no room named {label:?}"))
        .id
}

/// True if the connection `from --dir--> to` (by label) is marked `distorted`.
fn edge_distorted(graph: &MapGraph, from: &str, dir: Direction, to: &str) -> bool {
    let (o, d) = (room_id(graph, from), room_id(graph, to));
    graph
        .connections()
        .iter()
        .find(|c| c.origin == o && c.dir == dir && c.dest == d)
        .map(|c| c.distorted)
        .unwrap_or_else(|| panic!("no edge {from} -{dir:?}-> {to}"))
}

/// A reproduction of Zork I's Torch Room layer's SHAPE (SQ-1309): four rooms, a
/// trivially planar diamond, on their OWN layer, alongside an unrelated 4x4-grid
/// "town square" on Main — plus a one-way portal from the small layer back into
/// Main (the real game's `Altar --D--> Cave`, a room on Main), so the two layers
/// are still ONE connected component of the raw graph (`connected_components`
/// counts `Down`/`Up`/`In`/`Out` edges as connecting).
///
/// This asserts what `layout_all_layers` must guarantee — the small layer's shape
/// exactly matches the real game (Torch Room, Temple, Altar in one column, Egyptian
/// Room east of the Temple, nothing distorted) and Main's own grid stays intact —
/// but note it does NOT itself falsify against `mapper::layout::relayout_auto` run
/// once over the whole graph: at this scale (twenty rooms, one thin portal, no
/// interlocking chains crossing near the origin) a single combined relayout
/// happens to reach the same answer anyway, verified by reverting
/// `layout_all_layers`'s body to a bare `relayout_auto(graph)` call and rerunning
/// this case (still green). The real defect needed Zork I's actual richness — 64
/// Main rooms with many crossing E/W and N/S chains — to manifest; the real-game
/// Zork I assertions below (`torch_room_layer_matches_the_real_game_and_is_not_distorted`
/// and the Main-layer row/column cases) are what actually falsifies this fix.
/// This case stays as a regression pin on the SHAPE and on `layout_all_layers`'s
/// documented contract (each layer solved and packed independently).
///
/// The Up/Down pair Temple↔Torch Room agrees with the N/S pair; the Up/Down pair
/// Temple↔Egyptian Room CONTRADICTS the E/W pair and must yield (SQ-1291: Up/Down
/// never outranks a compass placement) — that part needs no per-layer fix, since
/// `mark_distorted` already gates on `grid_offset` (`None` for Up/Down) and
/// SQ-1287/1291's tiers already rank Up/Down last.
#[test]
fn a_small_layer_lays_out_independently_of_an_unrelated_main_layer_component() {
    let mut g = MapGraph::new();
    // The small layer: Temple/Torch Room/Altar/Egyptian Room, on their own layer.
    for (id, name) in [(1u32, "Temple"), (2, "Torch Room"), (3, "Altar"), (4, "Egyptian Room")] {
        g.upsert_room(id, name.into());
    }
    let small_layer = g.new_layer(None, "Torch Room".into());
    for id in [1u32, 2, 3, 4] {
        g.set_room_layer(id, small_layer);
    }
    g.add_edge(1, Direction::N, 2);
    g.add_edge(2, Direction::S, 1);
    g.add_edge(1, Direction::E, 4);
    g.add_edge(4, Direction::W, 1);
    g.add_edge(1, Direction::S, 3);
    g.add_edge(3, Direction::N, 1);
    g.add_edge(1, Direction::Up, 2); // agrees with N/S
    g.add_edge(2, Direction::Down, 1);
    g.add_edge(1, Direction::Down, 4); // CONTRADICTS E/W — must yield
    g.add_edge(4, Direction::Up, 1);

    // An unrelated Main-layer 4x4 grid of rooms, richly cross-linked with reciprocal
    // E/W and N/S chains along every row and column (a "town square"), the way
    // Zork I's own 64-room Main layer is.
    for y in 0..4i32 {
        for x in 0..4i32 {
            let id = 100 + (y * 4 + x) as u32;
            g.upsert_room(id, format!("Hall {x},{y}"));
        }
    }
    let id_at = |x: i32, y: i32| 100 + (y * 4 + x) as u32;
    for y in 0..4i32 {
        for x in 0..4i32 {
            if x + 1 < 4 {
                g.add_edge(id_at(x, y), Direction::E, id_at(x + 1, y));
                g.add_edge(id_at(x + 1, y), Direction::W, id_at(x, y));
            }
            if y + 1 < 4 {
                g.add_edge(id_at(x, y), Direction::S, id_at(x, y + 1));
                g.add_edge(id_at(x, y + 1), Direction::N, id_at(x, y));
            }
        }
    }
    // The real game's cross-layer portal: Altar --Down--> Cave (a Main room). This is
    // the ONLY thing that makes the small layer and Main one connected component.
    g.add_edge(3, Direction::Down, id_at(0, 0));

    app::mapgen::layout_all_layers(&mut g);

    let pos = |label: &str| g.room(room_id(&g, label)).unwrap().pos.unwrap();
    let (p_temple, p_torch, p_altar, p_egyptian) =
        (pos("Temple"), pos("Torch Room"), pos("Altar"), pos("Egyptian Room"));

    assert_eq!(p_torch, (p_temple.0, p_temple.1 - 1), "Torch Room is north of the Temple: {p_torch:?} {p_temple:?}");
    assert_eq!(p_altar, (p_temple.0, p_temple.1 + 1), "Altar is south of the Temple: {p_altar:?} {p_temple:?}");
    assert_eq!(
        p_egyptian,
        (p_temple.0 + 1, p_temple.1),
        "Egyptian Room is east of the Temple: {p_egyptian:?} {p_temple:?}"
    );

    for (from, dir, to) in [
        ("Temple", Direction::N, "Torch Room"),
        ("Torch Room", Direction::S, "Temple"),
        ("Temple", Direction::E, "Egyptian Room"),
        ("Egyptian Room", Direction::W, "Temple"),
        ("Temple", Direction::S, "Altar"),
        ("Altar", Direction::N, "Temple"),
    ] {
        assert!(!edge_distorted(&g, from, dir, to), "{from} -{dir:?}-> {to} must not be distorted");
    }

    // Main's own grid must be undisturbed too: each layer is laid out (and anchored
    // at its own origin) independently, so raw coordinates may coincide across
    // layers — that is fine, since they are rendered on separate planes — but
    // Main's sixteen rooms must still occupy sixteen DISTINCT cells among themselves.
    let main_labels: Vec<String> =
        (0..4).flat_map(|y| (0..4).map(move |x| format!("Hall {x},{y}"))).collect();
    let main_cells: std::collections::BTreeSet<(i32, i32)> =
        main_labels.iter().map(|l| pos(l)).collect();
    assert_eq!(main_cells.len(), 16, "Main's own grid must not have collapsed: {main_cells:?}");
}

/// Zork I's real Torch Room layer (SQ-1309), the shape the synthetic case above
/// reproduces: four rooms — Temple, Torch Room, Altar, Egyptian Room — reached from
/// Main only through Altar's `Down` portal into the Cave. This is the real-game
/// falsifier for the per-layer isolation fix in `app::mapgen::layout_all_layers`:
/// before it, this exact layer's four rooms were laid out in the SAME relayout call
/// as all 64 of Main's rooms and 111 rooms overall, and landed with EVERY compass
/// edge distorted and nonsense positions (pinned in the SQ-1309 quest history).
#[test]
fn torch_room_layer_matches_the_real_game_and_is_not_distorted() {
    let Some(path) = story("zork1-invclues-r52-s871125.z5") else {
        eprintln!("SKIP: stories/zork1-invclues-r52-s871125.z5 absent");
        return;
    };
    let map = mapgen::generate(&path, true).expect("Zork I must map");
    let g = &map.graph;
    let pos = |label: &str| g.room(room_id(g, label)).unwrap().pos.unwrap();
    let (p_temple, p_torch, p_altar, p_egyptian) =
        (pos("Temple"), pos("Torch Room"), pos("Altar"), pos("Egyptian Room"));

    assert_eq!(p_torch, (p_temple.0, p_temple.1 - 1), "Torch Room north of Temple: {p_torch:?} {p_temple:?}");
    assert_eq!(p_altar, (p_temple.0, p_temple.1 + 1), "Altar south of Temple: {p_altar:?} {p_temple:?}");
    assert_eq!(
        p_egyptian,
        (p_temple.0 + 1, p_temple.1),
        "Egyptian Room east of Temple: {p_egyptian:?} {p_temple:?}"
    );
    for (from, dir, to) in [
        ("Temple", Direction::N, "Torch Room"),
        ("Torch Room", Direction::S, "Temple"),
        ("Temple", Direction::E, "Egyptian Room"),
        ("Egyptian Room", Direction::W, "Temple"),
        ("Temple", Direction::S, "Altar"),
        ("Altar", Direction::N, "Temple"),
    ] {
        assert!(!edge_distorted(g, from, dir, to), "{from} -{dir:?}-> {to} must not be distorted");
    }
}

/// Zork I's white house (SQ-1309): a diagonal ring — West of House, North of House,
/// Behind House, South of House — plus the front door itself, Behind House ↔
/// Kitchen ↔ Living Room, each leg a reciprocated CARDINAL. Before the per-layer
/// isolation fix, the house's own "Rocky Ledge" layer (18 rooms) was laid out
/// alongside Main's 64 and every other layer in one shared relayout, and Kitchen's
/// real east/west doors — walked from both ends in the game — came out distorted.
#[test]
fn white_house_cardinal_doors_are_not_distorted() {
    let Some(path) = story("zork1-invclues-r52-s871125.z5") else {
        eprintln!("SKIP: stories/zork1-invclues-r52-s871125.z5 absent");
        return;
    };
    let map = mapgen::generate(&path, true).expect("Zork I must map");
    let g = &map.graph;
    for (from, dir, to) in [
        ("Behind House", Direction::W, "Kitchen"),
        ("Kitchen", Direction::E, "Behind House"),
        ("Kitchen", Direction::W, "Living Room"),
        ("Living Room", Direction::E, "Kitchen"),
    ] {
        assert!(!edge_distorted(g, from, dir, to), "{from} -{dir:?}-> {to} must not be distorted");
    }
}

/// Zork I's Round Room / Troll Room / East-West Passage / Chasm (SQ-1309): a real
/// case of the `contiguify` chain-member-eviction bug fixed in `mapper::layout::mod`
/// — East-West Passage is a genuine member of the Round Room/Troll Room row (a
/// reciprocated E/W chain) but, before the fix, an unrelated column chain elsewhere
/// on the same 64-room Main layer treated it as a foreign interloper and evicted it
/// off that row entirely.
#[test]
fn east_west_passage_stays_on_the_round_room_row_and_the_chasm_column_is_intact() {
    let Some(path) = story("zork1-invclues-r52-s871125.z5") else {
        eprintln!("SKIP: stories/zork1-invclues-r52-s871125.z5 absent");
        return;
    };
    let map = mapgen::generate(&path, true).expect("Zork I must map");
    let g = &map.graph;
    let pos = |label: &str| g.room(room_id(g, label)).unwrap().pos.unwrap();

    // East-West Passage shares the Round Room / Troll Room row.
    let (p_round, p_troll, p_ew) = (pos("Round Room"), pos("The Troll Room"), pos("East-West Passage"));
    assert_eq!(p_ew.1, p_round.1, "East-West Passage shares the Round Room's row: {p_ew:?} {p_round:?}");
    assert_eq!(p_ew.1, p_troll.1, "East-West Passage shares the Troll Room's row: {p_ew:?} {p_troll:?}");
    for (from, dir, to) in [
        ("East-West Passage", Direction::E, "Round Room"),
        ("Round Room", Direction::W, "East-West Passage"),
        ("East-West Passage", Direction::W, "The Troll Room"),
        ("The Troll Room", Direction::E, "East-West Passage"),
    ] {
        assert!(!edge_distorted(g, from, dir, to), "{from} -{dir:?}-> {to} must not be distorted");
    }

    // Chasm sits above North-South Passage, in the Round Room's own column.
    let (p_chasm, p_ns) = (pos("Chasm"), pos("North-South Passage"));
    assert_eq!(p_chasm.0, p_round.0, "Chasm shares the Round Room's column: {p_chasm:?} {p_round:?}");
    assert_eq!(p_chasm.0, p_ns.0, "Chasm shares North-South Passage's column: {p_chasm:?} {p_ns:?}");
    assert!(p_chasm.1 < p_ns.1, "Chasm is north (smaller y) of North-South Passage: {p_chasm:?} {p_ns:?}");
    for (from, dir, to) in [("North-South Passage", Direction::N, "Chasm"), ("Chasm", Direction::S, "North-South Passage")] {
        assert!(!edge_distorted(g, from, dir, to), "{from} -{dir:?}-> {to} must not be distorted");
    }

    // Reservoir South sits northeast of the Chasm (reciprocated diagonal).
    let p_reservoir = pos("Reservoir South");
    assert_eq!(p_reservoir.0, p_chasm.0 + 1, "Reservoir South is one east of the Chasm: {p_reservoir:?} {p_chasm:?}");
    assert_eq!(p_reservoir.1, p_chasm.1 - 1, "Reservoir South is one north of the Chasm: {p_reservoir:?} {p_chasm:?}");
    for (from, dir, to) in [("Chasm", Direction::NE, "Reservoir South"), ("Reservoir South", Direction::SW, "Chasm")] {
        assert!(!edge_distorted(g, from, dir, to), "{from} -{dir:?}-> {to} must not be distorted");
    }
}

/// SQ-1312: Zork I's `Studio` sits directly north of the `Gallery`.
///
/// The `Studio`'s only on-layer bearing is a reciprocated `N`/`S` pair with the `Gallery` (the
/// `Kitchen`'s `Down` into it crosses a layer boundary and is not in the subgraph at all). The
/// stress solve left it at `(-1, 5)` with the `Gallery` at `(-1, 8)` and both cells between them
/// empty: SMACOF averages over every pair in the component, and the separation VPSC enforces for
/// a cardinal pair is only a MINIMUM. The leaf snap pulls it onto the doorstep.
#[test]
fn zork1_leaves_sit_on_their_partners_doorstep() {
    let Some(path) = story("zork1-invclues-r52-s871125.z5") else {
        eprintln!("SKIP: stories/zork1-invclues-r52-s871125.z5 absent");
        return;
    };
    let map = mapgen::generate(&path, true).expect("Zork I must map");
    let g = &map.graph;
    let pos = |label: &str| g.room(room_id(g, label)).unwrap().pos.unwrap();

    let (p_gallery, p_studio) = (pos("Gallery"), pos("Studio"));
    assert_eq!(
        p_studio,
        (p_gallery.0, p_gallery.1 - 1),
        "Studio is directly north of the Gallery: {p_studio:?} {p_gallery:?}",
    );
    for (from, dir, to) in
        [("Gallery", Direction::N, "Studio"), ("Studio", Direction::S, "Gallery")]
    {
        assert!(!edge_distorted(g, from, dir, to), "{from} -{dir:?}-> {to} must not be distorted");
    }
}
