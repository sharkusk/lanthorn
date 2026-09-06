//! Connectors must not take turns nothing forced (SQ-1332).
//!
//! The user's report, verbatim: *"MANY cases where our path makes unnecessary turns before
//! reaching the destination. I understand this has to do with routing between rooms, but when
//! there is no room in the way it looks messy."* Against the two rules already on the drawn map —
//! overlaps forbidden, crossings accepted — that makes a third: with nothing in the way a
//! connector is a straight line where its anchors align and a single L where they do not, and a Z
//! or anything longer is only ever the price of dodging a room box or an overlap.
//!
//! [`app::render::map::bend_report`] is the measurement, and it reads the SAME
//! `ConnectorPlot.path` the terminal paints and the SVG draws — so this is about the picture, not
//! about the plan. Its `optimum` is deliberately blind to everything but the room boxes: it asks
//! only what the two anchors and the boxes between them permit, so a turn spent dodging another
//! connector, or forced by the two arrowheads pointing perpendicular ways, still counts as
//! excess. That makes the totals below a BUDGET rather than a target — the number to watch is
//! whether it goes UP.
//!
//! **The totals below count GHOST boxes as rooms** (SQ-1360). Since SQ-1356 a cross-layer ghost
//! is a room the layout seats, `render_layer` shifts the layer's real rooms to make space for one,
//! and `bend_report` reads its boxes from that render — so every number here is of a map with
//! more boxes on it than the one SQ-1332 measured, and none of them is comparable across that
//! change. Each case says what moved and by how much.
//!
//! Zork I and Anchorhead both live under the gitignored `stories/`, so every case here skips
//! vacuously off CI and says so. **That is why this suite reddened main for two days**: the
//! SQ-1356 lane verified with name filters that never matched it, and CI structurally cannot.
//! Run `cargo nextest run -p lanthorn sq1316 sq1332` against a checkout with `stories/` after any
//! layout, seating or routing change.

use std::path::{Path, PathBuf};

/// A story under the gitignored `stories/`, or `None` when this checkout has no copy.
fn story(name: &str) -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(name);
    p.is_file().then_some(p)
}

/// Zork I release 52 / serial 871125 — the fixture the quest was reported on.
const ZORK1: &str = "zork1-invclues-r52-s871125.z5";
/// Anchorhead (2018 illustrated edition) — the second, denser fixture.
const ANCHORHEAD: &str = "Anchorhead.gblorb";

/// `(connectors, bends, optimum)` summed over every non-empty layer of a story's map.
fn totals(map: &app::mapgen::GeneratedMap) -> (usize, usize, usize) {
    let mut layers: Vec<mapper::layer::LayerId> = map
        .graph
        .layers()
        .keys()
        .copied()
        .filter(|&l| !map.graph.rooms_in_layer(l).is_empty())
        .collect();
    layers.sort_unstable();
    let (mut n, mut bends, mut opt) = (0, 0, 0);
    for l in layers {
        for f in app::render::map::bend_report(&map.graph, l) {
            n += 1;
            bends += f.bends;
            opt += f.optimum;
        }
    }
    (n, bends, opt)
}

/// **The three connectors the user named, each at the fewest turns its anchors allow.**
///
/// | passage | layer | was | now | anchors allow |
/// |---|---|---|---|---|
/// | `West of House --W--> Forest` (#68→#91) | Rocky Ledge | 5 | 2 | 1 |
/// | `West of House <-SE-> South of House` (#68↔#217) | Rocky Ledge | 4 | 2 | 1 |
/// | `Canyon Bottom <-N-> End of Rainbow` (#78↔#131) | Main | 3 | 2 | 1 |
///
/// (The report is taken over every layer: the first two are on the house layer the user was
/// looking at, the third is not, and a case that hunted only the house layer would have said the
/// third passage did not exist.)
///
/// Each still draws one turn more than the room boxes alone would require, and that turn is the
/// arrowheads: the first leaves WEST for a room that is north-east, and the other two arrive on a
/// box CORNER whose channel lane sits one cell off it. Neither is a detour — pinning 2 rather
/// than 1 is the honest reading of "as tight as this passage can be drawn".
#[test]
fn zork1_named_connectors_take_the_fewest_turns_their_anchors_allow() {
    let Some(path) = story(ZORK1) else {
        eprintln!("SKIP zork1_named_connectors: stories/{ZORK1} absent");
        return;
    };
    let map = app::mapgen::generate(&path, true).expect("mapgen");
    let mut layers: Vec<mapper::layer::LayerId> = map
        .graph
        .layers()
        .keys()
        .copied()
        .filter(|&l| !map.graph.rooms_in_layer(l).is_empty())
        .collect();
    layers.sort_unstable();
    let report: Vec<_> =
        layers.iter().flat_map(|&l| app::render::map::bend_report(&map.graph, l)).collect();
    for (origin, dest, want) in [(68u32, 91u32, 2usize), (68, 217, 2), (78, 131, 2)] {
        let f = report
            .iter()
            .find(|f| (f.origin, f.dest) == (origin, dest) || (f.origin, f.dest) == (dest, origin))
            .unwrap_or_else(|| panic!("#{origin}↔#{dest} must be drawn somewhere on the map"));
        assert_eq!(f.bends, want, "#{origin}↔#{dest}: {f:?}");
    }
}

/// **The whole Zork I map's turn budget**, over every layer.
///
/// Measured with the router SQ-1332 replaced: **151** drawn turns against an anchor optimum of 72
/// (an excess of 79). After: **122** against the same 72 (an excess of 50). The pins are ceilings,
/// so a router change that straightens more is free and one that bends more has to come here and
/// say why.
///
/// **Re-based at SQ-1360, because the MAP grew, not because the router got worse.** SQ-1356 made a
/// cross-layer ghost a room the layout seats: Zork I's six layers gained twenty ghost boxes
/// between them (five on Main alone), and `render_layer`'s seating shifts the layer's real rooms
/// to make space. So this is a different map with more boxes in the way, and the optimum — "what
/// the anchors and the boxes between them permit" — moved with it: **83**, from 72.
///
/// Two thirds of that jump was a MEASUREMENT fault SQ-1360 also fixed, and the number is not
/// comparable across it: `bend_report` took its boxes from `graph.rooms_in_layer(layer)` while
/// plotting the connectors from `render_layer`'s own plan, so it was asking about rectangles at
/// the cells the layout had BEFORE the ghosts were seated, and about no ghost at all. On the same
/// tree the stale reading says 81 where the honest one says 83.
///
/// The drawn total moved with it: 152, from 122. The excess over optimum is 69, against 50
/// before the ghosts and 79 before SQ-1332 — the ghosts are boxes to go round, and going round
/// them costs turns.
///
/// **And again at SQ-1363, by two rooms on the MAZE layer** — 83 → **85**, 152 → **158**. A pushed
/// room now takes its dependants with it, which moved the `Clearing` ghost from `(0, 5)` to
/// `(1, 6)` and `Maze #169` from `(0, 4)` into the cell the ghost vacated. Two connectors gained,
/// and no other number on the map moved (the Main layer's report is byte-identical, and
/// Anchorhead's totals did not move at all):
///
/// | passage | layer | optimum | drawn |
/// |---|---|---|---|
/// | `Clearing #167 --Down--> Grating Room #225` | Maze | 0 → 1 | 0 → 4 |
/// | `Maze #159 --NW--> Maze #169` | Maze | 1 → 2 | 2 → 4 |
///
/// The first is worth a look rather than only a number: the ghost used to sit directly north of
/// Grating Room `(0, 6)` and the passage was one straight cell, and it now sits directly EAST of
/// it and the same passage loops out, west, north, west and back down — four turns between two
/// adjacent boxes. Both are still inside the per-connector ceiling below, so nothing fails on it.
#[test]
fn zork1_spends_no_more_turns_than_its_budget() {
    let Some(path) = story(ZORK1) else {
        eprintln!("SKIP zork1_spends_no_more_turns_than_its_budget: stories/{ZORK1} absent");
        return;
    };
    let map = app::mapgen::generate(&path, true).expect("mapgen");
    let (n, bends, opt) = totals(&map);
    assert!(n > 100, "Zork I must draw a real number of connectors, got {n}");
    assert_eq!(opt, 85, "the anchor optimum is a property of the LAYOUT, not the router");
    assert!(bends <= 158, "Zork I draws {bends} turns against a budget of 158 (was 152)");
}

/// The same budget on the denser fixture. Before SQ-1332: **110** turns against an optimum of 52.
/// After: **98**.
///
/// Re-based at SQ-1360 for the reason the Zork I case above states at length — eight ghost boxes
/// across Anchorhead's six layers, and the same stale-box measurement fault. Optimum **54**,
/// drawn **112**. SQ-1363 moved neither: its two pushes are both on Zork I's Maze layer.
#[test]
fn anchorhead_spends_no_more_turns_than_its_budget() {
    let Some(path) = story(ANCHORHEAD) else {
        eprintln!("SKIP anchorhead_spends_no_more_turns_than_its_budget: stories/{ANCHORHEAD} absent");
        return;
    };
    let map = app::mapgen::generate(&path, true).expect("mapgen");
    let (n, bends, opt) = totals(&map);
    assert!(n > 100, "Anchorhead must draw a real number of connectors, got {n}");
    assert_eq!(opt, 54, "the anchor optimum is a property of the LAYOUT, not the router");
    assert!(bends <= 112, "Anchorhead draws {bends} turns against a budget of 112 (was 98)");
}

/// A connector that draws a turn its anchors did not force is paying for something, and this is
/// the ceiling on how MUCH any single one may pay. Four is a Z with a detour on the end, and a
/// route that takes more is the "long dashed detour up and around the Attic" shape the quest was
/// filed against.
///
/// **One passage on Zork I is over it, and the two boxes that put it there are GHOSTS** (SQ-1360).
/// `Forest #91 --S--> Forest #230` on the Main layer draws five turns. #91 sits at cell `(0, 1)`
/// and #230 at `(3, 5)`, and of the two L routes their anchors allow, the horizontal-first one is
/// blocked by Forest Path, Up a Tree, Forest #33 and Clearing #134 — all real, all there before —
/// while the vertical-first one runs down column 0 through Living Room and then through the
/// **Cellar** and **Studio** ghost boxes SQ-1356 seated at `(0, 4)` and `(1, 5)`. With the whole
/// column closed the route drops to the channel BELOW row 4, crosses east, and comes back down
/// into #230's left side: five turns, every one of them round a box.
///
/// So the ceiling stays at four and the exception is named rather than the number raised — a
/// SECOND connector over four, or this one growing a sixth turn, still fails here. The list is
/// checked for exactly its one member, so the exemption cannot quietly absorb a pile.
#[test]
fn no_single_connector_takes_more_than_four_turns() {
    // `(story, layer, origin, dest, turns)` — every passage allowed past the ceiling, and why is
    // in the doc comment above. Matched exactly: an extra entry here needs the same treatment.
    const EXCUSED: &[(&str, &str, u32, u32, usize)] = &[(ZORK1, "Main", 91, 230, 5)];

    for name in [ZORK1, ANCHORHEAD] {
        let Some(path) = story(name) else {
            eprintln!("SKIP no_single_connector_takes_more_than_four_turns: stories/{name} absent");
            continue;
        };
        let map = app::mapgen::generate(&path, true).expect("mapgen");
        let mut layers: Vec<mapper::layer::LayerId> = map
            .graph
            .layers()
            .keys()
            .copied()
            .filter(|&l| !map.graph.rooms_in_layer(l).is_empty())
            .collect();
        layers.sort_unstable();
        let mut over: Vec<(String, u32, u32, usize)> = Vec::new();
        for l in layers {
            for f in app::render::map::bend_report(&map.graph, l) {
                let name_of = |id| {
                    map.graph.room(id).map(|r| r.label().to_string()).unwrap_or_default()
                };
                if f.bends <= 4 {
                    continue;
                }
                let layer = map.graph.layer_name(l).to_string();
                assert!(
                    EXCUSED.contains(&(name, layer.as_str(), f.origin, f.dest, f.bends)),
                    "[{name}/{layer}] {} -{:?}-> {} (#{}→#{}) draws {} turns (anchors allow {}): {:?}",
                    name_of(f.origin),
                    f.dir,
                    name_of(f.dest),
                    f.origin,
                    f.dest,
                    f.bends,
                    f.optimum,
                    f.path
                );
                over.push((layer, f.origin, f.dest, f.bends));
            }
        }
        // Non-vacuity, and the other half of "named rather than raised": the exemption must still
        // be describing exactly the shape it was written for on the map it was written for.
        let want: Vec<(String, u32, u32, usize)> = EXCUSED
            .iter()
            .filter(|e| e.0 == name)
            .map(|e| (e.1.to_string(), e.2, e.3, e.4))
            .collect();
        assert_eq!(over, want, "[{name}] the over-four list has changed shape");
    }
}
