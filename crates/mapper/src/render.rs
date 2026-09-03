/// Zoom-independent render model for the map.
///
/// Produces a `RenderMap` from a `MapGraph`: placed rooms projected into `RenderRoom`s,
/// routed edges via `route_all`, and grid bounds for viewport sizing/scrolling.
///
/// # Unplaced rooms
///
/// Rooms without a `pos` (possible in Manual mode for a freshly observed room) are skipped
/// from `rooms` since they have no grid cell to render. They may still appear as edge endpoints
/// in `edges` only if their partner room is also placed; however, `route_all` already skips
/// connections where either endpoint is unplaced.
use crate::graph::{MapGraph, RoomId};
use crate::layer::LayerId;
use crate::route::{route_lanes, RoutePlan};
use crate::router::{route_all, RoutedEdge};

/// A single room's render data in grid coordinates.
#[derive(Debug, Clone)]
pub struct RenderRoom {
    pub id: RoomId,
    pub label: String,
    /// Logical grid cell `(col, row)`.
    pub cell: (i32, i32),
    /// True when the room has non-empty notes.
    pub has_notes: bool,
    /// True when this room is the current room.
    pub is_current: bool,
    /// Compact chain-membership code, e.g. `"R0"`, `"C1"`, `"R0 C1"`, or `""`.
    pub align_code: String,
    /// True when this room owns an outgoing portal to another layer (set by `render_layer`).
    /// The renderer draws such rooms with a distinct box outline.
    pub has_layer_portal: bool,
    /// Directions that lead back INTO this room — see [`MapGraph::self_loops`] (SQ-0666).
    /// Carried here so the drawn view can badge the box without a second pass over the graph.
    pub self_loops: Vec<crate::direction::Direction>,
    /// How many OTHER names the story has printed for this room (SQ-1257 Phase 3) — see
    /// [`crate::graph::Room::aliases`]. Carried as a count, not the list itself: the drawn box
    /// has room only for a superscript digit beside the label, and the full list belongs to the
    /// room panel, which reads the graph directly.
    pub alias_count: usize,
    /// Every direction this room has a `?` random-exit mark on, paired with how many distinct
    /// destinations it has been seen to land in (SQ-1261) — see [`crate::graph::Room::random_exits`]
    /// / [`crate::graph::Room::random_destinations`]. Filtered to exclude a direction that ALSO
    /// carries a real edge (defensive: `mint_passage`/`unmark_random_exit` never leave the two
    /// coexisting, but a hand-edited or pre-upgrade map file could) — a real passage always wins
    /// the border slot. Carried as counts, not the destination lists themselves, for the same
    /// reason `alias_count` is: the box has room for one glyph per direction, and the full list
    /// belongs to the room panel and the dump.
    pub random_stubs: Vec<(crate::direction::Direction, usize)>,
}

/// The complete zoom-independent render description of the map.
#[derive(Debug, Clone)]
pub struct RenderMap {
    pub rooms: Vec<RenderRoom>,
    pub edges: Vec<RoutedEdge>,
    /// `(min_cell, max_cell)` over placed room cells, for the TUI to size/scroll.
    /// Both components satisfy `min <= max`. Empty graph → `((0,0),(0,0))`.
    pub bounds: ((i32, i32), (i32, i32)),
    pub plan: RoutePlan,
}

/// Build a `RenderMap` from `graph`. Convenience wrapper over
/// [`render_traced`] with a no-op step callback.
pub fn render(graph: &MapGraph) -> RenderMap {
    render_traced(graph, &mut |_| {})
}

/// Build a `RenderMap` from `graph`, calling `on_step` with a short label at the
/// start of each phase. Used by the app's background map-render worker to report
/// progress (the routing phases are the expensive ones). `render` is the same
/// pipeline with an empty callback.
pub fn render_traced(graph: &MapGraph, on_step: &mut dyn FnMut(&str)) -> RenderMap {
    on_step("detect chains");
    let current = graph.current();
    let chains = crate::layout::detect_chains(graph);

    on_step("place rooms");
    let rooms: Vec<RenderRoom> = graph
        .rooms()
        .filter_map(|room| {
            let cell = room.pos?; // skip unplaced rooms
            let mut parts: Vec<String> = Vec::new();
            if let Some(id) = chains.ew.get(&room.id) {
                parts.push(format!("R{id}"));
            }
            if let Some(id) = chains.ns.get(&room.id) {
                parts.push(format!("C{id}"));
            }
            let align_code = parts.join(" ");
            Some(RenderRoom {
                id: room.id,
                label: room.label().to_string(),
                cell,
                has_notes: !room.notes.is_empty(),
                is_current: Some(room.id) == current,
                align_code,
                has_layer_portal: false,
                self_loops: graph.self_loops(room.id),
                alias_count: room.aliases.len(),
                random_stubs: room
                    .random_exits
                    .iter()
                    .filter(|&&d| !graph.connections().iter().any(|c| c.origin == room.id && c.dir == d))
                    .map(|&d| (d, graph.random_destinations(room.id, d).len()))
                    .collect(),
            })
        })
        .collect();

    let bounds = if rooms.is_empty() {
        ((0, 0), (0, 0))
    } else {
        let min_col = rooms.iter().map(|r| r.cell.0).min().unwrap();
        let max_col = rooms.iter().map(|r| r.cell.0).max().unwrap();
        let min_row = rooms.iter().map(|r| r.cell.1).min().unwrap();
        let max_row = rooms.iter().map(|r| r.cell.1).max().unwrap();
        ((min_col, min_row), (max_col, max_row))
    };

    on_step("route edges");
    let edges = route_all(graph);
    on_step("route lanes");
    let plan = route_lanes(graph);

    RenderMap { rooms, edges, bounds, plan }
}

/// Build a `RenderMap` for a single layer. Rooms and grid connectors come from the
/// layer's sub-graph (so the existing routers are reused unchanged). Inter-layer edges
/// (Phase 2) are appended by `interlayer_badges`, which is empty while there is one layer.
pub fn render_layer(graph: &MapGraph, layer: LayerId) -> RenderMap {
    render_layer_traced(graph, layer, &mut |_| {})
}

/// [`render_layer`] with per-phase step reporting (see [`render_traced`]).
pub fn render_layer_traced(
    graph: &MapGraph,
    layer: LayerId,
    on_step: &mut dyn FnMut(&str),
) -> RenderMap {
    on_step("layer subgraph");
    let sub = graph.layer_subgraph(layer);
    let mut rm = render_traced(&sub, on_step);
    on_step("layer badges");
    let badges = crate::layer::interlayer_badges(graph, layer);
    // Flag rooms that own an outgoing cross-layer portal so the renderer can mark them
    // with a distinct box outline.
    let portal_rooms: std::collections::BTreeSet<RoomId> = badges.iter().map(|e| e.origin).collect();
    for r in &mut rm.rooms {
        r.has_layer_portal = portal_rooms.contains(&r.id);
    }
    rm.edges.extend(badges);
    rm
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapper::Mapper;
    use crate::direction::Direction;

    #[test]
    fn render_layer_flags_rooms_with_outgoing_cross_layer_portal() {
        use crate::layer::{move_region, planar_region, MoveTarget, MAIN_LAYER};
        let mut g = crate::graph::MapGraph::new();
        for (id, n) in [(1, "Hall"), (2, "Cellar")] {
            g.upsert_room(id, n.into());
        }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 1));
        g.add_edge(1, Direction::Down, 2);
        g.add_edge(2, Direction::Up, 1);
        let region = planar_region(&g, 2);
        move_region(&mut g, &region, MoveTarget::New).expect("peel cellar");
        let rm = render_layer(&g, MAIN_LAYER);
        let hall = rm.rooms.iter().find(|r| r.id == 1).unwrap();
        assert!(hall.has_layer_portal, "Hall has an outgoing portal to the cellar layer");
        // The all-layers render never flags (no per-layer context).
        let plain = render(&g);
        assert!(plain.rooms.iter().all(|r| !r.has_layer_portal));
    }

    #[test]
    fn render_traced_reports_phase_steps_and_matches_render() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        let mut steps: Vec<String> = Vec::new();
        let traced = render_traced(&m.graph, &mut |s| steps.push(s.to_string()));
        // The routing phases (the expensive ones) must be reported so the app's
        // step overlay can pinpoint delays.
        assert!(steps.iter().any(|s| s == "route edges"), "steps: {steps:?}");
        assert!(steps.iter().any(|s| s == "route lanes"), "steps: {steps:?}");
        // The traced pipeline is the same as `render`.
        let plain = render(&m.graph);
        assert_eq!(traced.rooms.len(), plain.rooms.len());
        assert_eq!(traced.edges.len(), plain.edges.len());
        assert_eq!(traced.bounds, plain.bounds);
    }

    #[test]
    fn render_marks_current_and_notes_and_bounds() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        m.set_notes(1, "start".into());
        let rm = render(&m.graph);
        assert_eq!(rm.rooms.len(), 2);
        let a = rm.rooms.iter().find(|r| r.id == 1).unwrap();
        assert!(a.has_notes);
        let b = rm.rooms.iter().find(|r| r.id == 2).unwrap();
        assert!(b.is_current); // current is the last-observed room (2)
        assert!(rm.bounds.0 .0 <= rm.bounds.1 .0); // min <= max
    }

    #[test]
    fn empty_graph_returns_zero_bounds_and_empty_rooms() {
        use crate::graph::MapGraph;
        let g = MapGraph::new();
        let rm = render(&g);
        assert!(rm.rooms.is_empty());
        assert!(rm.edges.is_empty());
        assert_eq!(rm.bounds, ((0, 0), (0, 0)));
    }

    #[test]
    fn unplaced_room_is_skipped() {
        use crate::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Placed".into());
        g.set_pos(1, (0, 0));
        g.upsert_room(2, "Unplaced".into()); // no pos
        let rm = render(&g);
        assert_eq!(rm.rooms.len(), 1);
        assert_eq!(rm.rooms[0].id, 1);
    }

    #[test]
    fn render_attaches_route_plan() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        let rm = render(&m.graph);
        // The plan routes the single drawn edge as one connector.
        assert_eq!(rm.plan.connectors.len(), 1, "render must attach a 1-connector plan");
    }

    #[test]
    fn single_room_bounds_are_equal_min_max() {
        use crate::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Solo".into());
        g.set_pos(1, (3, -2));
        let rm = render(&g);
        assert_eq!(rm.bounds, ((3, -2), (3, -2)));
    }

    #[test]
    fn render_layer_matches_render_for_single_layer() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        let all = render(&m.graph);
        let only = render_layer(&m.graph, 0);
        assert_eq!(only.rooms.len(), all.rooms.len());
        assert_eq!(only.bounds, all.bounds);
        assert_eq!(only.edges.len(), all.edges.len());
    }

    /// SQ-1261: a marked direction with no recorded destinations carries a zero count; once
    /// destinations are noted, the count follows.
    #[test]
    fn render_room_carries_random_stub_counts() {
        let mut m = Mapper::default();
        m.observe(1, "Windy Cave", None);
        m.observe(2, "A", None);
        assert!(m.record_random_exit(1, Direction::N));
        let rm = render(&m.graph);
        let r1 = rm.rooms.iter().find(|r| r.id == 1).unwrap();
        assert_eq!(r1.random_stubs, vec![(Direction::N, 0)], "marked, nothing recorded yet");

        m.graph.note_random_destination(1, Direction::N, 2);
        let rm = render(&m.graph);
        let r1 = rm.rooms.iter().find(|r| r.id == 1).unwrap();
        assert_eq!(r1.random_stubs, vec![(Direction::N, 1)], "the count follows what is recorded");

        let r2 = rm.rooms.iter().find(|r| r.id == 2).unwrap();
        assert!(r2.random_stubs.is_empty(), "an unmarked room carries no stubs");
    }

    /// Defensive guard (SQ-1261): a direction marked random must never ALSO appear as a stub
    /// once a real edge exists on the same key — `mint_passage`/`unmark_random_exit` never leave
    /// the two coexisting in ordinary play, but a hand-edited or pre-upgrade map file could, and
    /// the render layer must not draw a stub the connector pass is about to draw an arrowhead
    /// over.
    #[test]
    fn render_room_never_stubs_a_direction_that_also_carries_a_real_edge() {
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        g.mark_random_exit(1, Direction::E); // hand-edited/stale: both facts on one key
        let rm = render(&g);
        let r1 = rm.rooms.iter().find(|r| r.id == 1).unwrap();
        assert!(r1.random_stubs.is_empty(), "a real edge on the key wins; no stub is drawn beside it");
    }

    #[test]
    fn render_layer_shows_only_its_layer() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        let l = m.graph.new_layer(Some(0), "Other".into());
        m.graph.set_room_layer(2, l);
        let main = render_layer(&m.graph, 0);
        assert!(main.rooms.iter().any(|r| r.id == 1));
        assert!(!main.rooms.iter().any(|r| r.id == 2), "room 2 lives in another layer");
    }
}
