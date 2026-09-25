//! A connector's folded Up/Down/In/Out directions now travel with `layer_layout`'s own data
//! (SQ-1562).
//!
//! `select_shared_paths` (`crates/mapper/src/route/mod.rs`) draws only ONE line per room pair —
//! when a pair is joined by both a compass passage and an Up/Down one, the compass line wins and
//! the Up/Down folds onto it as `RoutedConnector::secondary_exit`/`secondary_entry`. SQ-1368 gave
//! the SVG export a badge for that folded direction, but `LayerLayout::connectors`
//! (`app::export_svg::LayoutConnector`, SQ-1540) never carried the fact at all — a host reading
//! the layout data had no way to know the folded passage existed. This suite pins the fix: the
//! fields now on `LayoutConnector`, and one real Zork I occurrence confirmed live.
//!
//! **The reachable shape has the fold on only ONE end, never both**, which the cases below use
//! deliberately rather than the two-ended shape a first reading of the fix might expect: a
//! direction sharing its OWN room's origin+destination with the drawn connector's own departure
//! (e.g. both `N` and `Up` leaving the SAME room A for the SAME room B) is intercepted earlier,
//! by `collapse_stacked_exits` (SQ-1276, `mapper::render`) — a separate fold this quest does not
//! touch, tracked as `RenderRoom::stacked_exits` rather than a router-level secondary. Only a
//! direction departing the OTHER room (which cannot pair with the drawn connector's own — see
//! `can_pair`) survives to reach `select_shared_paths`, and it can only ever land at the end that
//! isn't the drawn connector's own origin. `zork1_canyon_view_rocky_ledge_carries_its_folded_up_in_layout_data`
//! below is the real-world instance: Canyon View's own `E` is drawn, and it is Rocky Ledge's own
//! `Up` back — the OTHER room's edge — that folds onto it. All eight real occurrences in a fully
//! explored Zork I confirm the same one-ended shape.

use std::path::{Path, PathBuf};

use app::export_svg::layer_layout;
use mapper::direction::Direction;
use mapper::graph::MapGraph;
use mapper::render::{render, render_layer};

/// A story under the gitignored `stories/`, or `None` when this checkout has no copy.
fn story(name: &str) -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(name);
    p.is_file().then_some(p)
}

/// Zork I release 52 / serial 871125.
const ZORK1: &str = "zork1-invclues-r52-s871125.z5";

/// A→E→B (compass, drawn), A→Down→B (same origin+dest as E — collapsed by `collapse_stacked_exits`
/// into a `RenderRoom::stacked_exits` entry, NOT a router-level fold, so it must NOT show up on
/// the connector below), and B→Up→A (the OTHER room's edge — cannot pair with E, so it reaches
/// `select_shared_paths` and folds onto E's own line). Same graph as export_svg's own
/// `a_passage_folded_onto_a_shared_line_keeps_its_marker` (SQ-1368), asserted here at the
/// `layer_layout` data level instead of the SVG's rendered badge.
#[test]
fn compass_connector_carries_the_other_rooms_folded_direction_in_layout_data() {
    let mut g = MapGraph::new();
    g.upsert_room(1, "A".into());
    g.upsert_room(2, "B".into());
    g.set_pos(1, (0, 0));
    g.set_pos(2, (1, 0)); // east of A, matching E's own bearing
    g.add_edge(1, Direction::Down, 2);
    g.add_edge(1, Direction::E, 2);
    g.add_edge(2, Direction::Up, 1);

    let rm = render(&g);
    let layout = layer_layout(&rm, Some(&g)).expect("a non-empty map lays out");
    assert_eq!(layout.connectors.len(), 1, "one drawn line for the pair");
    let c = &layout.connectors[0];
    assert_eq!(c.origin, 1, "A");
    assert_eq!(c.dest, 2, "B");
    assert_eq!(c.exit_dir, Direction::E, "E is the primary; Down is stacked away, not a rival pairing");
    assert!(c.secondary_exit.is_empty(), "Down never reaches the router — collapse_stacked_exits owns it");
    assert_eq!(c.secondary_entry, vec![Direction::Up], "B's own Up back to A folds onto E's line");
}

/// The existing portal-marker-suppression behaviour (`layer_layout`'s own `portal_ends`
/// bookkeeping, built from these same `secondary_exit`/`secondary_entry` facts) must be
/// unchanged by adding the fields above: the folded Up must NOT also appear as its own
/// `LayoutPortalMarker` stub — it has a marker via the connector fold now exposed above, and a
/// second one would double-badge the same passage.
#[test]
fn folded_updown_does_not_also_yield_a_portal_marker() {
    let mut g = MapGraph::new();
    g.upsert_room(1, "A".into());
    g.upsert_room(2, "B".into());
    g.set_pos(1, (0, 0));
    g.set_pos(2, (1, 0));
    g.add_edge(1, Direction::Down, 2);
    g.add_edge(1, Direction::E, 2);
    g.add_edge(2, Direction::Up, 1);

    let rm = render(&g);
    let layout = layer_layout(&rm, Some(&g)).expect("a non-empty map lays out");
    assert!(
        layout.portals.iter().all(|p| p.direction != Direction::Up && p.direction != Direction::Down),
        "the folded Up/Down must not also surface as a LayoutPortalMarker: {:?}",
        layout.portals
    );
}

// A merge stub (`LayoutConnector::merge`) is guarded to carry no folded directions, mirroring
// `render_svg_body`'s own `!conn.merge` guard on the very same `RoutedConnector::secondary_exit`/
// `secondary_entry` fields this fix reads (its own points' ends are trunk junctions, not room
// edges, so there is nowhere valid to anchor a badge). No test pins that guard directly: since
// SQ-0522, `select_shared_paths` folds every non-representative same-pair edge into a secondary
// badge before the router ever runs (`extra_same_pair_edges_become_stacked_icons` in
// `crates/mapper/src/route/mod.rs`), and neither real fixture below (Zork I, Anchorhead, fully
// explored) produces a single `merge: true` connector any more — so a synthetic trigger for this
// case could not be verified against real behaviour rather than guessed.

/// **Real Zork I fixture.** Canyon View↔Rocky Ledge (#23↔#22) is one of eight pairs across the
/// fully-explored map where a compass passage and an Up/Down share a room pair — Canyon View→E→
/// Rocky Ledge is drawn, and Rocky Ledge's own `Up` back to Canyon View folds onto that line
/// instead of drawing its own (the other seven: Maze #44↔#169, #54↔#165, #85↔#140, #85↔#44,
/// #227↔#57, #227↔#140, and Cave↔Atlantis Room #66↔#4 — confirmed live against this same
/// fixture while writing this suite).
///
/// Room ids are stable for this exact story build, the same convention `sq1332_connector_bends.rs`
/// relies on for its own named-connector pins.
#[test]
fn zork1_canyon_view_rocky_ledge_carries_its_folded_up_in_layout_data() {
    let Some(path) = story(ZORK1) else {
        eprintln!("SKIP zork1_canyon_view_rocky_ledge: stories/{ZORK1} absent");
        return;
    };
    let map = app::mapgen::generate(&path, true).expect("mapgen");

    const CANYON_VIEW: mapper::graph::RoomId = 23;
    const ROCKY_LEDGE: mapper::graph::RoomId = 22;
    assert_eq!(map.graph.room(CANYON_VIEW).map(|r| r.name.as_str()), Some("Canyon View"));
    assert_eq!(map.graph.room(ROCKY_LEDGE).map(|r| r.name.as_str()), Some("Rocky Ledge"));

    let mut layers: Vec<mapper::layer::LayerId> = map
        .graph
        .layers()
        .keys()
        .copied()
        .filter(|&l| !map.graph.rooms_in_layer(l).is_empty())
        .collect();
    layers.sort_unstable();

    let mut found = None;
    for l in layers {
        let rm = render_layer(&map.graph, l);
        let Some(layout) = layer_layout(&rm, Some(&map.graph)) else { continue };
        if let Some(c) = layout.connectors.iter().find(|c| {
            (c.origin, c.dest) == (CANYON_VIEW, ROCKY_LEDGE)
                || (c.origin, c.dest) == (ROCKY_LEDGE, CANYON_VIEW)
        }) {
            found = Some(c.clone());
            break;
        }
    }
    let c = found.expect("Canyon View<->Rocky Ledge must be drawn as a connector somewhere");

    assert_eq!(c.exit_dir, Direction::E, "the compass exit is the one drawn, per select_shared_paths");
    assert!(!c.is_portal, "the drawn line is the compass one, not the folded portal");
    // The folded edge's real origin is Rocky Ledge (its own `Up` back to Canyon View) — whichever
    // end of the drawn connector Rocky Ledge sits at is where `select_shared_paths` recorded it
    // (`secondary_exit` when Rocky Ledge is `conn.origin`, `secondary_entry` when it is `conn.dest`).
    let folded = if c.dest == ROCKY_LEDGE { &c.secondary_entry } else { &c.secondary_exit };
    assert_eq!(
        folded,
        &vec![Direction::Up],
        "Rocky Ledge's own Up back to Canyon View must ride the drawn line's layout data: {c:?}"
    );
}
