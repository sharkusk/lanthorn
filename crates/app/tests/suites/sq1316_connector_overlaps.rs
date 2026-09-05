//! Connectors must not OVERLAP; crossings are fine (SQ-1316).
//!
//! The user's rule, verbatim: *"crossings are okay, overlaps need to be avoided."* Two
//! connectors sharing a stretch of one lane — running on top of each other for any length —
//! is the defect. Two connectors meeting perpendicular at a point is a crossing, which the
//! terminal already draws with a break in the horizontal and which the user asked to keep.
//!
//! The invariant is checked at BOTH levels, because neither sees the other's failures:
//!
//! * **On the plan**, via [`mapper::route::plan_overlaps`] — renderer-independent, so it holds
//!   for the terminal Boxes zoom and the SVG alike (they share `PosTable`, lanes, anchors and
//!   `plot_connector` since SQ-1313). A plan-level overlap is a routing defect wherever it is
//!   drawn.
//! * **On the rendered cells**, via `render::map::overlap_stats` — which catches what the plan
//!   cannot: a bend whose corner lands on another connector's run, and two runs the plan puts
//!   on different lattice lines that the pixel table resolves to one.
//!
//! Zork I is the fixture the quest was filed against, and it lives under the gitignored
//! `stories/`, so those cases skip vacuously off CI and say so. The Adventure and synthetic
//! graphs below are the CI-runnable half.

use std::path::{Path, PathBuf};

use mapper::graph::MapGraph;
use mapper::route::plan_overlaps;

use crate::fixture_paths::fixture_path;

/// A story under the gitignored `stories/`, or `None` when this checkout has no copy.
fn story(name: &str) -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(name);
    p.is_file().then_some(p)
}

/// Zork I release 52 / serial 871125 — the fixture SQ-1316 was reported on.
const ZORK1: &str = "zork1-invclues-r52-s871125.z5";

/// A human-readable report of one plan's overlaps, naming the rooms so a failure points at a
/// place on the map rather than at two indices.
fn report(graph: &MapGraph, plan: &mapper::route::RoutePlan) -> Vec<String> {
    let name = |id| graph.room(id).map(|r| r.label().to_string()).unwrap_or_else(|| format!("#{id:?}"));
    plan_overlaps(plan)
        .into_iter()
        .map(|o| {
            let (a, b) = (&plan.connectors[o.a], &plan.connectors[o.b]);
            format!(
                "{} {}={} lane {} spans {}..{}: {}->{} ({:?}) vs {}->{} ({:?})",
                if o.horizontal { "H" } else { "V" },
                if o.horizontal { "y" } else { "x" },
                o.line,
                o.lane,
                o.start,
                o.end,
                name(a.origin),
                name(a.dest),
                a.exit_dir,
                name(b.origin),
                name(b.dest),
                b.exit_dir,
            )
        })
        .collect()
}

/// Every layer of `graph`, routed, reported.
fn layer_reports(graph: &MapGraph) -> Vec<(String, Vec<String>)> {
    let mut layers: Vec<mapper::layer::LayerId> = graph
        .layers()
        .keys()
        .copied()
        .filter(|&l| !graph.rooms_in_layer(l).is_empty())
        .collect();
    layers.sort_unstable();
    layers
        .into_iter()
        .map(|l| {
            let rm = mapper::render::render_layer(graph, l);
            (graph.layer_name(l).to_string(), report(graph, &rm.plan))
        })
        .collect()
}

/// The whole Zork I map, generated the way `lanthorn-mapgen` generates it.
fn zork1_map() -> Option<app::mapgen::GeneratedMap> {
    let path = story(ZORK1)?;
    Some(app::mapgen::generate(&path, true).expect("mapgen"))
}

// ---------------------------------------------------------------------------
// The invariant, on the fixture the quest was filed against
// ---------------------------------------------------------------------------

/// No two connectors may share a stretch of one lane, on any layer of the Zork I map.
///
/// This is the spec. It failed on main with the overlaps listed in the quest: the Strange
/// Passage↔Living Room conditional running under West of House in the same channel as the West
/// of House↔South of House diagonal, and the distorted one-ways snaking around the house on top
/// of other lanes.
#[test]
fn zork1_has_no_connector_overlaps_on_any_layer() {
    let Some(map) = zork1_map() else {
        eprintln!("SKIP zork1_has_no_connector_overlaps_on_any_layer: stories/{ZORK1} absent");
        return;
    };
    let mut failures = Vec::new();
    for (layer, overlaps) in layer_reports(&map.graph) {
        for o in overlaps {
            failures.push(format!("[{layer}] {o}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} connector overlap(s) on the Zork I map:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The rendered CELLS agree: every cell shared by two connectors is a clean crossing (exactly
/// one horizontal contributor and one vertical one), never a stomp.
///
/// `overlap_stats` has counted this for years — as an optimisation target for `cleanup_overlaps`
/// to minimise, never as a floor anything had to reach. SQ-1316 makes zero the requirement.
#[test]
fn zork1_renders_no_illegal_cell_overlaps() {
    let Some(map) = zork1_map() else {
        eprintln!("SKIP zork1_renders_no_illegal_cell_overlaps: stories/{ZORK1} absent");
        return;
    };
    let mut layers: Vec<mapper::layer::LayerId> = map
        .graph
        .layers()
        .keys()
        .copied()
        .filter(|&l| !map.graph.rooms_in_layer(l).is_empty())
        .collect();
    layers.sort_unstable();
    let mut failures = Vec::new();
    for l in layers {
        for line in app::render::map::overlap_report(&map.graph, l) {
            failures.push(format!("[{}] {line}", map.graph.layer_name(l)));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Routing is not layout: the fix must not move a single room.
///
/// The positions are read before and after a full route of every layer. `route_lanes` takes
/// `&MapGraph` and cannot move anything, but the cost model reaches into `direct_route_losers`
/// and the entry-side choice, and a future "just nudge that room" is exactly the shortcut this
/// forbids.
#[test]
fn zork1_routing_moves_no_rooms() {
    let Some(map) = zork1_map() else {
        eprintln!("SKIP zork1_routing_moves_no_rooms: stories/{ZORK1} absent");
        return;
    };
    let before: Vec<(mapper::graph::RoomId, Option<(i32, i32)>)> =
        map.graph.rooms().map(|r| (r.id, r.pos)).collect();
    let _ = layer_reports(&map.graph);
    let after: Vec<(mapper::graph::RoomId, Option<(i32, i32)>)> =
        map.graph.rooms().map(|r| (r.id, r.pos)).collect();
    assert_eq!(before, after, "routing must not change any room position");
}

/// Counterfeit Monkey: a Glulx map an order of magnitude larger than Zork I, and the second
/// graph the SVG is regenerated against. A rule that only holds on the graph it was written for
/// is a coincidence.
///
/// Its plan is clean like Zork I's. Its rendered cells keep ONE residual shape, and this case
/// pins that the residual is only ever that shape — a regression anywhere else still fails here.
///
/// **The residual: two DIAGONALS crossing inside one gutter.** Church Forecourt, Park Center and
/// Monumental Staircase sit in a row with Midway, Fair and Heritage Corner directly beneath, and
/// `Fair↔Church Forecourt` (NW) crosses `Park Center↔Midway` (SW) in the gap between the two
/// columns. Drawn as two 45° lines that is a plain X. But the SVG and the terminal both draw a
/// diagonal as an ORTHOGONAL dogleg (`export_svg` passes `None` for the glyph set, and
/// `overlap_stats` measures the same reading on purpose so the tidy metric does not move with a
/// display setting), and two doglegs cannot do it:
///
/// * each has a vertical leg in the gutter, on its own lane — those are separable;
/// * each ALSO has a horizontal leg on the row of the box corner it anchors to, and the two
///   connectors anchor to boxes in the SAME two rows (Fair and Midway share a top row, Park
///   Center and Church Forecourt a bottom row);
/// * on the upper row the legs are disjoint only if Midway's lane is the nearer one; on the lower
///   row only if Church Forecourt's is. Those are the two ENDS of one connector each, so the two
///   requirements are exact opposites. No lane assignment satisfies both.
///
/// It is a property of drawing a crossing as two doglegs, not of the routing, and the fix is for
/// the SVG to draw a crossing diagonal as a real 45° line — which is a change to how a diagonal
/// is DRAWN, not to where it is routed, and so is not this quest.
#[test]
fn counterfeit_monkey_overlaps_are_only_the_crossing_diagonals() {
    let Some(path) = story("CounterfeitMonkey-11.gblorb") else {
        eprintln!("SKIP counterfeit_monkey_overlaps_are_only_the_crossing_diagonals: fixture absent");
        return;
    };
    let map = app::mapgen::generate(&path, true).expect("mapgen");
    let mut failures = Vec::new();
    for (layer, overlaps) in layer_reports(&map.graph) {
        for o in overlaps {
            failures.push(format!("[{layer}] plan: {o}"));
        }
    }
    let mut layers: Vec<mapper::layer::LayerId> = map
        .graph
        .layers()
        .keys()
        .copied()
        .filter(|&l| !map.graph.rooms_in_layer(l).is_empty())
        .collect();
    layers.sort_unstable();
    let mut crossing_diagonal_cells = 0usize;
    let mut isolated_touches = 0usize;
    for l in layers {
        let plan = mapper::render::render_layer(&map.graph, l).plan;
        let name = |id| {
            map.graph.room(id).map(|r| r.label().to_string()).unwrap_or_else(|| format!("#{id:?}"))
        };
        let shared = app::render::map::overlap_cells(&map.graph, l);
        for (cell, who) in &shared {
            let (cell, who) = (*cell, who.clone());
            // A cell the same two connectors share with NO shared neighbour is a TOUCH: two
            // polylines meeting at a point and parting again, which is the crossing the user's
            // rule allows, not the running-alongside it forbids. (`Roget Close→gate` and
            // `Winding Footpath→Roget Close` each turn in one cell of the same gutter.)
            let adjacent = |d: (i32, i32)| {
                let n = (cell.0 + d.0, cell.1 + d.1);
                shared.iter().any(|(c, w)| *c == n && *w == who)
            };
            if !adjacent((1, 0)) && !adjacent((-1, 0)) && !adjacent((0, 1)) && !adjacent((0, -1)) {
                isolated_touches += 1;
                continue;
            }
            // The exempt shape: exactly two connectors, both leaving on a diagonal, and their
            // doubled-coord polylines pass through a common gap-lattice corner — i.e. they cross.
            let diagonal_pair = who.len() == 2
                && who.iter().all(|&ci| mapper::direction::is_diagonal(plan.connectors[ci].exit_dir));
            let shares_a_corner = diagonal_pair && {
                let pts = |ci: usize| plan.connectors[ci].points.clone();
                let (a, b) = (pts(who[0]), pts(who[1]));
                a.iter().any(|p| p.0 % 2 != 0 && p.1 % 2 != 0 && b.contains(p))
            };
            if diagonal_pair && shares_a_corner {
                crossing_diagonal_cells += 1;
                continue;
            }
            let names: Vec<String> = who
                .iter()
                .map(|&ci| {
                    let c = &plan.connectors[ci];
                    format!("{}->{}({:?})", name(c.origin), name(c.dest), c.exit_dir)
                })
                .collect();
            failures.push(format!(
                "[{}] cell {cell:?}: {}",
                map.graph.layer_name(l),
                names.join(" + ")
            ));
        }
    }
    assert!(failures.is_empty(), "{} overlap(s):\n{}", failures.len(), failures.join("\n"));
    // Non-vacuity: the exemption must still be describing the shape it was written for, not
    // quietly absorbing a growing pile. Two crossings, one gutter each, three cells apiece.
    assert_eq!(
        crossing_diagonal_cells, 6,
        "the crossing-diagonal residual has changed size — re-read the doc comment above"
    );
    assert_eq!(isolated_touches, 1, "exactly one point-touch on this map");
}

// ---------------------------------------------------------------------------
// CI-runnable: tracked fixtures and synthetic shapes
// ---------------------------------------------------------------------------

/// The same invariant on a tracked fixture, so CI can fail on it.
#[test]
fn minizork_has_no_connector_overlaps() {
    let path = fixture_path("minizork-r34-s871124.z3");
    if !path.is_file() {
        eprintln!("SKIP minizork_has_no_connector_overlaps: {} absent", path.display());
        return;
    }
    let map = app::mapgen::generate(&path, true).expect("mapgen");
    let mut failures = Vec::new();
    for (layer, overlaps) in layer_reports(&map.graph) {
        for o in overlaps {
            failures.push(format!("[{layer}] {o}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A crossing is NOT an overlap. Two connectors cutting perpendicular through one gap must be
/// reported clean — otherwise the invariant would be a licence to rewrite routes the user is
/// happy with.
#[test]
fn a_perpendicular_crossing_is_not_an_overlap() {
    use mapper::direction::Direction;
    let mut g = MapGraph::new();
    // Four rooms at the corners of a 3x3, two passages crossing through the middle gap.
    for (id, label, pos) in [
        (1u32, "NW", (0, 0)),
        (2, "NE", (2, 0)),
        (3, "SW", (0, 2)),
        (4, "SE", (2, 2)),
    ] {
        g.upsert_room(id, label.into());
        g.set_pos(id, pos);
    }
    g.add_edge(1, Direction::SE, 4);
    g.add_edge(4, Direction::NW, 1);
    g.add_edge(2, Direction::SW, 3);
    g.add_edge(3, Direction::NE, 2);
    let plan = mapper::route::route_lanes(&g);
    assert!(
        plan_overlaps(&plan).is_empty(),
        "two connectors crossing is a crossing, not an overlap: {:?}",
        report(&g, &plan)
    );
}

/// Two connectors that genuinely run along one another ARE reported. Without this the suite
/// could pass by reporting nothing at all.
#[test]
fn plan_overlaps_detects_a_real_stomp() {
    use mapper::route::{Channel, LaneSeg, RoutedConnector};
    use mapper::router::Side;
    let seg = |lane| LaneSeg { channel: Channel::H(0), lane, start: -9, end: 9 };
    let conn = |origin, dest, lane| RoutedConnector {
        origin,
        dest,
        distorted: false,
        exit: Side::Right,
        entry: Side::Left,
        // centre → … a long run along H(0) … → centre; the interior is what is compared.
        points: vec![(0, 0), (1, 1), (7, 1), (8, 0)],
        segs: vec![seg(lane)],
        exit_slot: 0,
        entry_slot: 0,
        reciprocal: false,
        exit_dir: mapper::direction::Direction::E,
        entry_dir: None,
        entry_corner: None,
        merge: false,
        secondary_exit: Vec::new(),
        secondary_entry: Vec::new(),
    };
    let same_lane = mapper::route::RoutePlan {
        connectors: vec![conn(1, 2, 0), conn(3, 4, 0)],
        ..Default::default()
    };
    assert_eq!(plan_overlaps(&same_lane).len(), 1, "two runs on ONE lane overlap");
    let split = mapper::route::RoutePlan {
        connectors: vec![conn(1, 2, 0), conn(3, 4, 1)],
        ..Default::default()
    };
    assert!(plan_overlaps(&split).is_empty(), "the lane system separates them");
}
