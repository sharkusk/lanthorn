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
//! Zork I and Anchorhead both live under the gitignored `stories/`, so every case here skips
//! vacuously off CI and says so.

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
#[test]
fn zork1_spends_no_more_turns_than_its_budget() {
    let Some(path) = story(ZORK1) else {
        eprintln!("SKIP zork1_spends_no_more_turns_than_its_budget: stories/{ZORK1} absent");
        return;
    };
    let map = app::mapgen::generate(&path, true).expect("mapgen");
    let (n, bends, opt) = totals(&map);
    assert!(n > 100, "Zork I must draw a real number of connectors, got {n}");
    assert_eq!(opt, 72, "the anchor optimum is a property of the LAYOUT, not the router");
    assert!(bends <= 122, "Zork I draws {bends} turns against a budget of 122 (was 151)");
}

/// The same budget on the denser fixture. Before SQ-1332: **110** turns against an optimum of 52.
/// After: **98**.
#[test]
fn anchorhead_spends_no_more_turns_than_its_budget() {
    let Some(path) = story(ANCHORHEAD) else {
        eprintln!("SKIP anchorhead_spends_no_more_turns_than_its_budget: stories/{ANCHORHEAD} absent");
        return;
    };
    let map = app::mapgen::generate(&path, true).expect("mapgen");
    let (n, bends, opt) = totals(&map);
    assert!(n > 100, "Anchorhead must draw a real number of connectors, got {n}");
    assert_eq!(opt, 52, "the anchor optimum is a property of the LAYOUT, not the router");
    assert!(bends <= 98, "Anchorhead draws {bends} turns against a budget of 98 (was 110)");
}

/// A connector that draws a turn its anchors did not force is paying for something, and this is
/// the ceiling on how MUCH any single one may pay. Four is a Z with a detour on the end; nothing
/// on either reference map needs more, and a route that does is the "long dashed detour up and
/// around the Attic" shape the quest was filed against.
#[test]
fn no_single_connector_takes_more_than_four_turns() {
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
        for l in layers {
            for f in app::render::map::bend_report(&map.graph, l) {
                let name_of = |id| {
                    map.graph.room(id).map(|r| r.label().to_string()).unwrap_or_default()
                };
                assert!(
                    f.bends <= 4,
                    "[{name}/{}] {} -{:?}-> {} draws {} turns (anchors allow {}): {:?}",
                    map.graph.layer_name(l),
                    name_of(f.origin),
                    f.dir,
                    name_of(f.dest),
                    f.bends,
                    f.optimum,
                    f.path
                );
            }
        }
    }
}
