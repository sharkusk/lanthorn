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
    /// This room's 1-based per-map discovery ordinal — [`crate::graph::Room::ordinal`] (SQ-1300).
    /// Carried here rather than left for the drawn box to look up: `draw_box_room` (the one
    /// consumer that needs it, for a synthetic room's `#12`-style label) only ever sees a
    /// `RenderRoom`, never the live `MapGraph`.
    pub ordinal: u64,
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
    ///
    /// Filtered to exclude a direction that ALSO carries a `?` random-exit mark (SQ-1269): current
    /// code never leaves the two coexisting on one key (marking a direction removes whatever old
    /// edge/self-loop stood there — see `Mapper::resolve_suspicion_as_random`), but a map file
    /// saved before that held both, and `crate::matrix::classify` already prefers the mark — the
    /// stronger, more specific fact — over the loop badge on the same key. The box agrees: the `?`
    /// stub supersedes the loop badge for that direction, never both.
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
    /// Groups of this room's own outgoing directions that all lead to the SAME destination
    /// (SQ-1276) — see [`collapse_stacked_exits`]. Only the group's primary direction is routed
    /// and drawn; the render layer accent-styles its arrowhead and the rest surface on hover.
    /// The GRAPH carries every direction regardless — this is a rendering fact only.
    pub stacked_exits: Vec<StackedExit>,
    /// Set when this entry is a cross-layer GHOST rather than one of the layer's own rooms
    /// (SQ-1356) — see [`GhostRoom`] and [`render_layer`]. `None` for every real room, and for
    /// every room in an all-layers [`render`], which has no layer boundary to cross.
    pub ghost: Option<GhostRoom>,
}

/// What a cross-layer ghost stands for (SQ-1356): a room on ANOTHER layer that this one has a
/// passage to, drawn here as a placeholder so the passage has somewhere to land.
///
/// The four facts travel together because a ghost drawn with any of them missing is a lie: the
/// name without the layer says a room is here that is not, and the kind is what decides whether
/// the label reads `Cellar`, `to Cellar` or `from Maze` — see [`GhostRoom::label_for`], the only
/// place that spelling is decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhostRoom {
    /// The foreign room's own plain name — what [`RenderRoom::label`] carries for a real room.
    pub name: String,
    /// The layer that room actually lives on.
    pub layer: LayerId,
    /// That layer's display name, for the second line of the drawn box (or, where there is no
    /// room for one, the room card).
    pub layer_name: String,
    /// How this layer reaches it.
    pub kind: GhostKind,
}

impl GhostRoom {
    /// The label a ghost box shows: the plain room name when the crossing is walkable both ways
    /// (the box reads exactly as a real room's would, and the passage draws the ordinary
    /// two-headed arrow), and a direction word when it is not.
    ///
    /// Never a "to/from" pair: a passage that goes both ways is a passage, not two.
    pub fn label_for(kind: GhostKind, name: &str) -> String {
        match kind {
            GhostKind::TwoWay => name.to_string(),
            GhostKind::To => format!("to {name}"),
            GhostKind::From => format!("from {name}"),
        }
    }
}

/// How a layer reaches the room one of its ghosts stands for (SQ-1356).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GhostKind {
    /// Passages run BOTH ways across the boundary — the ordinary case, and the one a two-headed
    /// arrow is drawn for.
    TwoWay,
    /// A one-way passage LEAVES this layer for it, with no way back.
    To,
    /// A one-way passage ARRIVES from it, which this layer has no way to walk back along. The
    /// only mark the crossing gets on this side, since the other layer's own panel draws nothing
    /// for a passage that does not leave it.
    From,
}

/// One room's redundant fan-out to a single destination (SQ-1276): several of ITS OWN outgoing
/// directions — compass, or compass plus Up/Down/In/Out — that all lead to `dest`. Only
/// `primary` is routed; `secondary` lists the rest, in `MapGraph::connections()` order.
///
/// Built by [`collapse_stacked_exits`] and never by hand — the primary is a routing decision
/// (bearing-matches-the-real-position first, see that function), and drifting the two apart
/// would draw an arrowhead for a direction the graph doesn't actually route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackedExit {
    pub primary: crate::direction::Direction,
    pub dest: RoomId,
    pub secondary: Vec<crate::direction::Direction>,
}

/// True when `dir`'s own compass bearing agrees with `delta = dest.pos - origin.pos`: every
/// axis `dir` fixes (its `grid_offset` component is nonzero) has the SAME SIGN as `delta`'s, and
/// every axis it does NOT fix (zero) is exactly zero in `delta` too — so `W` matches any purely
/// negative-x delta regardless of magnitude (a distant room still reads as "west"), but not one
/// with any north/south component at all. Unlike `Connection::distorted` (which the layout
/// engine computes only for a room it could not seat at its PREFERRED unit-offset slot, and
/// leaves at its default `false` on a raw/unlaid-out graph — exactly the state test graphs and
/// `MapGraph::add_edge` start in) this reads current positions directly, so it answers the same
/// question `collapse_stacked_exits` actually needs regardless of whether layout has run.
fn matches_bearing(dir: crate::direction::Direction, delta: (i32, i32)) -> bool {
    let Some((sx, sy)) = crate::direction::grid_offset(dir) else { return false };
    let sign_ok = |s: i32, d: i32| if s == 0 { d == 0 } else { d.signum() == s };
    sign_ok(sx, delta.0) && sign_ok(sy, delta.1) && delta != (0, 0)
}

/// Fixed tie-break order for [`collapse_stacked_exits`] when two compass directions to the same
/// destination bearing-match equally (both or neither): N, S, E, W, NE, NW, SE, SW, lower wins.
/// This is NOT a geometric claim that one tied direction is more "correct" than the other — a
/// tie only happens when [`matches_bearing`] could not tell them apart either (see that
/// function) — it exists so the choice is deterministic rather than dependent on
/// `MapGraph::connections()`'s order.
fn compass_tie_priority(d: crate::direction::Direction) -> u8 {
    use crate::direction::Direction::*;
    match d {
        N => 0, S => 1, E => 2, W => 3, NE => 4, NW => 5, SE => 6, SW => 7,
        _ => 8,
    }
}

/// Collapse each room's redundant same-destination fan-out to one PRIMARY direction (SQ-1276):
/// several exits from one room that all lead to the same other room draw as a single arrowhead,
/// not one line per direction.
///
/// A group is every OTHER connection sharing one connection's exact `(origin, dest)` (a
/// self-loop or an `Unknown` edge never groups — neither carries a destination reading worth
/// stacking). A group with no COMPASS member at all (every link to that destination is a portal
/// — Up/Down/In/Out) is left alone entirely: there is nothing to prefer a portal direction
/// over, so nothing changes.
///
/// Primary is the compass member whose direction [`matches_bearing`] where the destination
/// ACTUALLY sits, which is unique whenever it exists (only one compass bearing can agree with
/// one real delta). When no candidate matches — or, a degenerate map, more than one does — ties
/// break by [`compass_tie_priority`]. Either room lacking a placed position falls straight to
/// the tie-break (nothing to compare positions of).
///
/// Returns a graph clone with every non-primary member of a stacked group removed, fed to the
/// routers so a suppressed direction is never routed OR drawn — the graph `render_traced` was
/// given is never touched, only this routing-only copy is (matrix/room-card/dump/archive read
/// the original graph directly and see every edge) — plus the per-room facts the renderer needs
/// for the primary's accent style and the hover tooltip.
fn collapse_stacked_exits(
    graph: &MapGraph,
) -> (MapGraph, std::collections::HashMap<RoomId, Vec<StackedExit>>) {
    use std::collections::BTreeMap;
    let mut by_pair: BTreeMap<(RoomId, RoomId), Vec<usize>> = BTreeMap::new();
    for (i, c) in graph.connections().iter().enumerate() {
        if c.origin == c.dest || c.dir == crate::direction::Direction::Unknown {
            continue;
        }
        by_pair.entry((c.origin, c.dest)).or_default().push(i);
    }
    let mut filtered = graph.clone();
    let mut stacked: std::collections::HashMap<RoomId, Vec<StackedExit>> =
        std::collections::HashMap::new();
    let conns = graph.connections();
    for ((origin, dest), idxs) in by_pair {
        if idxs.len() < 2 {
            continue;
        }
        let compass: Vec<usize> = idxs
            .iter()
            .copied()
            .filter(|&i| crate::direction::grid_offset(conns[i].dir).is_some())
            .collect();
        let delta = match (graph.room(origin).and_then(|r| r.pos), graph.room(dest).and_then(|r| r.pos)) {
            (Some(a), Some(b)) => Some((b.0 - a.0, b.1 - a.1)),
            _ => None, // an unplaced end: nothing to compare positions of, fall to the tie-break
        };
        let Some(&primary_idx) = compass.iter().min_by_key(|&&i| {
            let matches = delta.is_some_and(|d| matches_bearing(conns[i].dir, d));
            (!matches, compass_tie_priority(conns[i].dir))
        }) else {
            continue; // portal-only stack: nothing to prefer a portal direction over
        };
        let primary_dir = conns[primary_idx].dir;
        let mut secondary = Vec::new();
        for &i in &idxs {
            if i == primary_idx {
                continue;
            }
            secondary.push(conns[i].dir);
            filtered.remove_connection(origin, conns[i].dir);
        }
        stacked.entry(origin).or_default().push(StackedExit { primary: primary_dir, dest, secondary });
    }
    (filtered, stacked)
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

    on_step("collapse stacked exits");
    let (routing_graph, mut stacked) = collapse_stacked_exits(graph);

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
                ordinal: room.ordinal(),
                cell,
                has_notes: !room.notes.is_empty(),
                is_current: Some(room.id) == current,
                align_code,
                has_layer_portal: false,
                self_loops: graph
                    .self_loops(room.id)
                    .into_iter()
                    .filter(|&d| !graph.is_random_exit(room.id, d))
                    .collect(),
                alias_count: room.aliases.len(),
                random_stubs: room
                    .random_exits
                    .iter()
                    .filter(|&&d| !graph.connections().iter().any(|c| c.origin == room.id && c.dir == d))
                    .map(|&d| (d, graph.random_destinations(room.id, d).len()))
                    .collect(),
                stacked_exits: stacked.remove(&room.id).unwrap_or_default(),
                ghost: None, // `render_layer` fills these in; a plain render crosses no boundary
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
    let edges = route_all(&routing_graph);
    on_step("route lanes");
    let plan = route_lanes(&routing_graph);

    RenderMap { rooms, edges, bounds, plan }
}

/// Build a `RenderMap` for a single layer: the layer's own rooms, plus a GHOST for every room on
/// another layer this one has a passage to (SQ-1356).
///
/// The ghosts are seated in the layer's own grid and their passages routed as ordinary passages,
/// so the drawn map, the vector export and the terminal all show the same thing — a box beside
/// the room the crossing touches, joined to it by a line with a head at whichever end each
/// travel arrives at.
pub fn render_layer(graph: &MapGraph, layer: LayerId) -> RenderMap {
    render_layer_traced(graph, layer, &mut |_| {})
}

/// One cross-layer ghost to seat, before it has a cell (SQ-1356).
///
/// `anchor` is the room on THIS layer the crossing touches and `offset` the grid step the
/// passage's own direction wants the ghost to sit at — the two the seating pass needs together;
/// `kind` decides the label. Built by [`ghost_specs`] and nowhere else.
struct GhostSpec {
    id: RoomId,
    anchor: RoomId,
    offset: (i32, i32),
    kind: GhostKind,
}

/// Every foreign room `layer` needs a ghost for, one per ROOM rather than one per passage: two
/// passages from this layer to the same room beyond it are two lines to one box, exactly as two
/// passages between two rooms of one layer are.
///
/// A crossing walked both ways is [`GhostKind::TwoWay`] (the ordinary case); one that only
/// LEAVES this layer is `To`, one that only arrives is `From`. Where a foreign room is reached
/// several ways at once the strongest reading wins — a reciprocal crossing anywhere makes the
/// ghost two-way, since the room genuinely is walkable both ways from here.
///
/// Anchor and offset come from the FIRST passage in `connections()` order that names the room, so
/// the placement is stable for a given graph. An ARRIVING passage seats its ghost where the
/// return trip would have left from — the opposite of the word walked — since that is the side of
/// the anchor the traveller came in on.
fn ghost_specs(graph: &MapGraph, layer: LayerId) -> Vec<GhostSpec> {
    use std::collections::BTreeMap;
    let mut order: Vec<RoomId> = Vec::new();
    let mut found: BTreeMap<RoomId, GhostSpec> = BTreeMap::new();
    for c in graph.connections() {
        if !crate::layer::is_interlayer(graph, c) {
            continue;
        }
        let leaving = graph.layer_of(c.origin) == layer;
        if !leaving && graph.layer_of(c.dest) != layer {
            continue;
        }
        let reciprocal =
            graph.connections().iter().any(|o| o.origin == c.dest && o.dest == c.origin);
        let (id, anchor, dir) = if leaving {
            (c.dest, c.origin, c.dir)
        } else {
            (c.origin, c.dest, crate::direction::opposite(c.dir))
        };
        let kind = if reciprocal {
            GhostKind::TwoWay
        } else if leaving {
            GhostKind::To
        } else {
            GhostKind::From
        };
        let Some(offset) = crate::layout::seat_offset(dir) else { continue };
        match found.get_mut(&id) {
            // A room reached several ways keeps the first passage's placement, but a reciprocal
            // crossing upgrades the reading: that is true of the ROOM, not of one passage to it.
            Some(spec) => {
                if kind == GhostKind::TwoWay {
                    spec.kind = GhostKind::TwoWay;
                }
            }
            None => {
                order.push(id);
                found.insert(id, GhostSpec { id, anchor, offset, kind });
            }
        }
    }
    order.into_iter().filter_map(|id| found.remove(&id)).collect()
}

/// [`render_layer`] with per-phase step reporting (see [`render_traced`]).
pub fn render_layer_traced(
    graph: &MapGraph,
    layer: LayerId,
    on_step: &mut dyn FnMut(&str),
) -> RenderMap {
    on_step("layer subgraph");
    let mut sub = graph.layer_subgraph(layer);

    on_step("seat ghosts");
    // Every real room's cell, as the plane the ghosts are seated into. `seat_adjacent` may open a
    // line in it to make room — real rooms shift together when it does, so their reciprocal
    // adjacencies survive — which is why the cells are read back out of `plane` afterwards
    // instead of left as the layer's own.
    let mut plane: std::collections::BTreeMap<RoomId, (i32, i32)> =
        sub.rooms().filter_map(|r| r.pos.map(|p| (r.id, p))).collect();
    let specs = ghost_specs(graph, layer);
    let mut ghosts: Vec<(RoomId, GhostRoom)> = Vec::new();
    for spec in &specs {
        let Some(room) = graph.room(spec.id) else { continue };
        let Some(seat) = crate::layout::seat_adjacent(&sub, &mut plane, spec.anchor, spec.offset)
        else {
            continue; // the anchor is unplaced: nothing to seat against
        };
        let dest_layer = graph.layer_of(spec.id);
        sub.upsert_room(spec.id, GhostRoom::label_for(spec.kind, room.label()));
        plane.insert(spec.id, seat.cell);
        ghosts.push((
            spec.id,
            GhostRoom {
                name: room.label().to_string(),
                layer: dest_layer,
                layer_name: graph.layer_name(dest_layer).to_string(),
                kind: spec.kind,
            },
        ));
    }
    // Every crossing this layer touches, as an ordinary connection of the sub-graph: the routers
    // draw a two-way crossing with a head at each end and a one-way with one, exactly as they do
    // for a passage between two real rooms.
    let ghost_ids: std::collections::BTreeSet<RoomId> = ghosts.iter().map(|(id, _)| *id).collect();
    for c in graph.connections() {
        if !crate::layer::is_interlayer(graph, c) {
            continue;
        }
        let touches = (ghost_ids.contains(&c.dest) && graph.layer_of(c.origin) == layer)
            || (ghost_ids.contains(&c.origin) && graph.layer_of(c.dest) == layer);
        if touches {
            sub.add_edge_weighted(c.origin, c.dir, c.dest, c.weight);
        }
    }
    for (&id, &pos) in &plane {
        sub.set_pos(id, pos);
    }

    let mut rm = render_traced(&sub, on_step);
    // Flag rooms that own an outgoing cross-layer portal so the renderer can mark them with a
    // distinct box outline. A ghost is never flagged: the outline says "this room leads off the
    // layer", and a ghost is where it leads.
    let portal_rooms: std::collections::BTreeSet<RoomId> = graph
        .connections()
        .iter()
        .filter(|c| crate::layer::is_interlayer(graph, c) && graph.layer_of(c.origin) == layer)
        .map(|c| c.origin)
        .collect();
    for r in &mut rm.rooms {
        r.has_layer_portal = portal_rooms.contains(&r.id);
        if let Some((_, ghost)) = ghosts.iter().find(|(id, _)| *id == r.id) {
            r.has_layer_portal = false;
            r.is_current = false; // the player is never standing in a placeholder
            r.ghost = Some(ghost.clone());
        }
    }
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

    // ── SQ-1276: stacked same-destination exits collapse to one primary ─────────

    /// Two compass directions from one room to the same destination: only one connector is
    /// routed, the other is recorded as a stacked secondary. Falsify by reverting
    /// `collapse_stacked_exits` and this fails on the connector count.
    #[test]
    fn two_compass_edges_to_one_destination_collapse_to_one_routed_connector() {
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Cellar".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::N, 2);
        g.add_edge(1, Direction::S, 2);
        let rm = render(&g);
        let r1 = rm.rooms.iter().find(|r| r.id == 1).unwrap();
        assert_eq!(
            r1.stacked_exits,
            vec![StackedExit { primary: Direction::N, dest: 2, secondary: vec![Direction::S] }],
            "N wins the fixed tie order over S when neither is distorted",
        );
        let routed: Vec<_> =
            rm.plan.connectors.iter().filter(|c| c.origin == 1 && c.dest == 2).collect();
        assert_eq!(routed.len(), 1, "only the primary is routed: {routed:?}");
        assert_eq!(routed[0].exit_dir, Direction::N);
    }

    /// A compass direction plus a portal (Up/Down) to the same destination: the compass
    /// direction always wins (portals never outrank a compass primary), and the portal is
    /// suppressed from routing — so no portal badge has an edge left to draw from.
    #[test]
    fn compass_plus_portal_to_one_destination_suppresses_the_portal() {
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Cellar".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(1, Direction::Down, 2);
        let rm = render(&g);
        let r1 = rm.rooms.iter().find(|r| r.id == 1).unwrap();
        assert_eq!(
            r1.stacked_exits,
            vec![StackedExit { primary: Direction::E, dest: 2, secondary: vec![Direction::Down] }],
        );
        let routed: Vec<_> =
            rm.plan.connectors.iter().filter(|c| c.origin == 1 && c.dest == 2).collect();
        assert_eq!(routed.len(), 1, "the Down connector is not routed: {routed:?}");
        assert_eq!(routed[0].exit_dir, Direction::E);
        assert!(
            !rm.edges.iter().any(|e| e.origin == 1 && e.dest == 2 && e.dir == Direction::Down),
            "the suppressed Down edge has nothing left for a portal badge to draw",
        );
    }

    /// A destination reached ONLY by portals (no compass direction at all): nothing to prefer a
    /// portal over, so `collapse_stacked_exits` is a no-op — routing comes out identical to
    /// routing the graph directly (the pre-existing SQ-0689 same-pair collapse, unrelated to
    /// SQ-1276, still applies to Up+Down exactly as it always has).
    #[test]
    fn portal_only_stack_is_left_alone() {
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Cellar".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(1, Direction::Down, 2);
        let rm = render(&g);
        let r1 = rm.rooms.iter().find(|r| r.id == 1).unwrap();
        assert!(r1.stacked_exits.is_empty(), "{:?}", r1.stacked_exits);
        let direct = crate::route::route_lanes(&g);
        let describe = |plan: &crate::route::RoutePlan| {
            let mut v: Vec<_> = plan
                .connectors
                .iter()
                .map(|c| (c.origin, c.dest, c.exit_dir, c.secondary_exit.clone()))
                .collect();
            v.sort_by_key(|t| (t.0, t.1));
            v
        };
        assert_eq!(describe(&rm.plan), describe(&direct), "collapse must not change portal-only routing");
    }

    /// Rendering never mutates the graph it was given — the matrix, room card, dump and archive
    /// all read the original graph and must still see every edge (SQ-1276 item 3).
    #[test]
    fn collapse_does_not_mutate_the_source_graph() {
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Cellar".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::N, 2);
        g.add_edge(1, Direction::S, 2);
        let before = g.connections().len();
        let _rm = render(&g);
        assert_eq!(g.connections().len(), before, "render must not remove any connection");
        assert!(g.connections().iter().any(|c| c.dir == Direction::N && c.dest == 2));
        assert!(g.connections().iter().any(|c| c.dir == Direction::S && c.dest == 2));
    }

    #[test]
    fn render_layer_shows_only_its_layer() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        let l = m.graph.new_layer(Some(0), "Other".into());
        m.graph.set_room_layer(2, l);
        let main = render_layer(&m.graph, 0);
        let a = main.rooms.iter().find(|r| r.id == 1).unwrap();
        assert!(a.ghost.is_none(), "room 1 is this layer's own");
        // Room 2 lives elsewhere, so it appears only as the GHOST its crossing lands on (SQ-1356)
        // — never as one of the layer's own rooms.
        let b = main.rooms.iter().find(|r| r.id == 2).expect("the crossing draws a ghost for it");
        assert!(b.ghost.is_some(), "room 2 lives in another layer: it is a ghost here");
    }

    // ── SQ-1356: cross-layer ghosts are rooms the layout places ────────────────

    /// Two layers, one crossing walked BOTH ways and one walked only one way. Each layer's render
    /// carries a ghost per foreign room, labelled by which of the three readings applies, seated
    /// in the free cell the passage's own direction points at.
    #[test]
    fn each_layer_ghosts_its_crossings_with_the_right_label_and_cell() {
        let mut g = crate::graph::MapGraph::new();
        for (id, n) in [(1, "Hall"), (2, "Study"), (3, "Cellar"), (4, "Sump")] {
            g.upsert_room(id, n.into());
        }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.set_pos(3, (0, 0));
        g.set_pos(4, (1, 0));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        g.add_edge(3, Direction::E, 4);
        g.add_edge(4, Direction::W, 3);
        let l = g.new_layer(Some(0), "Under".into());
        g.set_room_layer(3, l);
        g.set_room_layer(4, l);
        // Hall ⇄ Cellar, walked both ways; Study → Sump, walked one way only.
        g.add_edge(1, Direction::Down, 3);
        g.add_edge(3, Direction::Up, 1);
        g.add_edge(2, Direction::Down, 4);

        let main = render_layer(&g, 0);
        let cellar = main.rooms.iter().find(|r| r.id == 3).expect("a ghost for the Cellar");
        let ghost = cellar.ghost.as_ref().unwrap();
        assert_eq!(ghost.kind, GhostKind::TwoWay);
        assert_eq!(ghost.name, "Cellar");
        assert_eq!(ghost.layer_name, "Under");
        assert_eq!(cellar.label, "Cellar", "a two-way crossing names the room plainly");
        assert_eq!(cellar.cell, (0, 1), "Down seats it directly below its anchor");
        let sump = main.rooms.iter().find(|r| r.id == 4).expect("a ghost for the Sump");
        assert_eq!(sump.ghost.as_ref().unwrap().kind, GhostKind::To);
        assert_eq!(sump.label, "to Sump", "a one-way crossing OUT says where it leads");
        assert_eq!(sump.cell, (1, 1));

        let under = render_layer(&g, l);
        let hall = under.rooms.iter().find(|r| r.id == 1).expect("a ghost for the Hall");
        assert_eq!(hall.ghost.as_ref().unwrap().kind, GhostKind::TwoWay);
        assert_eq!(hall.label, "Hall");
        assert_eq!(hall.cell, (0, -1), "Up seats it directly above the Cellar");
        let study = under.rooms.iter().find(|r| r.id == 2).expect("a ghost for the Study");
        assert_eq!(study.ghost.as_ref().unwrap().kind, GhostKind::From);
        assert_eq!(study.label, "from Study", "the arriving end says where it came FROM");
        assert_eq!(study.cell, (1, -1), "seated where the return trip would have left from");
    }

    /// A ghost's passage is an ORDINARY connection, so the router draws it: a two-way crossing
    /// comes out as one reciprocal connector, a one-way as a plain one. Nothing is a stub any
    /// more (`interlayer_badges` is gone with SQ-1356).
    #[test]
    fn a_ghosts_passage_is_routed_like_any_other() {
        let mut g = crate::graph::MapGraph::new();
        for (id, n) in [(1, "Hall"), (2, "Cellar")] {
            g.upsert_room(id, n.into());
        }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 0));
        let l = g.new_layer(Some(0), "Under".into());
        g.set_room_layer(2, l);
        g.add_edge(1, Direction::Down, 2);
        g.add_edge(2, Direction::Up, 1);
        let rm = render_layer(&g, 0);
        let c = rm
            .plan
            .connectors
            .iter()
            .find(|c| c.origin == 1 && c.dest == 2)
            .expect("the crossing is routed as a connector");
        assert!(c.reciprocal, "walked both ways: one line, a head at each end");
        // The only stubs left are the ordinary Up/Down portal stubs ANY vertical passage gets
        // (`router::side_for` gives Up and Down no planar side) — never a cross-layer badge, which
        // SQ-1356 retired along with `interlayer_badges`.
        let mut stubs: Vec<_> = rm
            .edges
            .iter()
            .filter(|e| e.is_stub)
            .map(|e| (e.origin, crate::direction::short_label(e.dir)))
            .collect();
        stubs.sort();
        assert_eq!(stubs, vec![(1, "d"), (2, "u")], "{:?}", rm.edges);
    }

    /// Two staircases from one layer to two different rooms beyond it draw two ghosts, one per
    /// destination room — the reading `interlayer_badges_are_per_edge` used to pin on badges.
    #[test]
    fn two_staircases_to_two_rooms_draw_two_ghosts() {
        let mut g = crate::graph::MapGraph::new();
        for (id, n) in [(1, "HallN"), (2, "HallS"), (3, "CellarN"), (4, "CellarS")] {
            g.upsert_room(id, n.into());
        }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 1));
        g.set_pos(3, (0, 0));
        g.set_pos(4, (0, 1));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        let l = g.new_layer(Some(0), "Cellar".into());
        g.set_room_layer(3, l);
        g.set_room_layer(4, l);
        g.add_edge(1, Direction::Down, 3);
        g.add_edge(2, Direction::Down, 4);
        let rm = render_layer(&g, 0);
        let mut named: Vec<_> = rm
            .rooms
            .iter()
            .filter_map(|r| r.ghost.as_ref().map(|gh| (gh.name.as_str(), gh.layer_name.as_str())))
            .collect();
        named.sort();
        assert_eq!(named, vec![("CellarN", "Cellar"), ("CellarS", "Cellar")]);
    }

    /// A crowded layer: the wanted cell is taken, so the map OPENS to make room — and every real
    /// room's reciprocal adjacencies survive it (SQ-1356). Compared as adjacency PAIRS, not
    /// absolute cells: opening a line moves rooms on purpose.
    #[test]
    fn a_crowded_layer_opens_for_a_ghost_without_costing_a_single_adjacency() {
        let mut g = crate::graph::MapGraph::new();
        for (id, n) in [(1, "Hall"), (2, "West"), (3, "East"), (4, "Cellar")] {
            g.upsert_room(id, n.into());
        }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (-1, 1));
        g.set_pos(3, (0, 1)); // exactly where the Hall's Down wants its ghost
        g.set_pos(4, (0, 0));
        g.add_edge(2, Direction::E, 3);
        g.add_edge(3, Direction::W, 2);
        let l = g.new_layer(Some(0), "Under".into());
        g.set_room_layer(4, l);
        g.add_edge(1, Direction::Down, 4);
        g.add_edge(4, Direction::Up, 1);

        // Every reciprocal cardinal pair, as the offsets they sit at, before ghosts exist.
        let pairs = |rm: &RenderMap| {
            let cell = |id| rm.rooms.iter().find(|r| r.id == id).map(|r| r.cell);
            vec![(cell(2), cell(3))]
        };
        let bare = render(&g.layer_subgraph(0));
        let rm = render_layer(&g, 0);
        let a = rm.rooms.iter().find(|r| r.id == 2).unwrap().cell;
        let b = rm.rooms.iter().find(|r| r.id == 3).unwrap().cell;
        assert_eq!((b.0 - a.0, b.1 - a.1), (1, 0), "West─East is still exactly adjacent");
        assert_eq!(pairs(&bare).len(), pairs(&rm).len());
        let ghost = rm.rooms.iter().find(|r| r.id == 4).unwrap();
        assert_eq!(ghost.cell, (0, 1), "the ghost took the cell it wanted");
        assert_eq!(rm.rooms.iter().find(|r| r.id == 1).unwrap().cell, (0, 0), "the anchor stayed");
        assert!(a.1 > 1 && b.1 > 1, "the row below opened to let it in: {a:?} {b:?}");
    }

    /// …and when opening the line WOULD cost an adjacency, it is not opened: the ghost walks out
    /// to the first free cell along its own bearing instead, and every real room stays put.
    #[test]
    fn a_ghost_never_pulls_a_reciprocal_pair_apart() {
        let mut g = crate::graph::MapGraph::new();
        for (id, n) in [(1, "Hall"), (2, "Below"), (3, "Cellar")] {
            g.upsert_room(id, n.into());
        }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 1));
        g.set_pos(3, (0, 0));
        g.add_edge(1, Direction::S, 2);
        g.add_edge(2, Direction::N, 1);
        let l = g.new_layer(Some(0), "Under".into());
        g.set_room_layer(3, l);
        g.add_edge(1, Direction::Down, 3);
        g.add_edge(3, Direction::Up, 1);
        let rm = render_layer(&g, 0);
        assert_eq!(rm.rooms.iter().find(|r| r.id == 1).unwrap().cell, (0, 0));
        assert_eq!(rm.rooms.iter().find(|r| r.id == 2).unwrap().cell, (0, 1), "still adjacent");
        assert_eq!(rm.rooms.iter().find(|r| r.id == 3).unwrap().cell, (0, 2), "the ghost gave way");
    }
}
