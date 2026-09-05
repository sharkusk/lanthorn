//! Re-tidy "sort" stage: per-axis longest-path layering from compass edges.
//!
//! Each planar edge imposes an ordering on one or both axes (east → larger x,
//! north → smaller y). We build a DAG per axis (dropping cycle-closing edges)
//! and assign integer coordinates by longest path, giving a compact grid that
//! honours every non-contradictory compass relation.

use std::collections::{BTreeMap, BTreeSet};

use crate::direction::{grid_offset, layout_offset};
use crate::graph::{MapGraph, RoomId};

use super::{connected_components, nearest_free_cell};

/// Longest-path layering for one axis. `order_edges` = (lo,hi) meaning coord[lo] < coord[hi].
/// Cycle-closing edges (processed in slice order) are skipped.
pub(crate) fn layer_axis(n: usize, order_edges: &[(usize, usize)]) -> Vec<i32> {
    // Accept edges that don't close a cycle (hi cannot already reach lo).
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(lo, hi) in order_edges {
        if lo == hi {
            continue;
        }
        if reaches(&adj, hi, lo) {
            continue; // would close a cycle → drop
        }
        adj[lo].push(hi);
    }
    // Longest path via Kahn's topological sort + relaxation.
    // in_degree[v] = number of predecessors in the DAG.
    let mut in_degree = vec![0usize; n];
    for successors in &adj {
        for &w in successors {
            in_degree[w] += 1;
        }
    }
    // Queue starts with all sources (in-degree 0).
    let mut queue: std::collections::VecDeque<usize> =
        (0..n).filter(|&v| in_degree[v] == 0).collect();
    let mut coord = vec![0_i32; n];
    while let Some(v) = queue.pop_front() {
        for &w in &adj[v] {
            if coord[v] + 1 > coord[w] {
                coord[w] = coord[v] + 1;
            }
            in_degree[w] -= 1;
            if in_degree[w] == 0 {
                queue.push_back(w);
            }
        }
    }
    coord
}

fn reaches(adj: &[Vec<usize>], from: usize, target: usize) -> bool {
    let mut stack = vec![from];
    let mut seen = BTreeSet::new();
    while let Some(v) = stack.pop() {
        if v == target {
            return true;
        }
        if !seen.insert(v) {
            continue;
        }
        stack.extend(adj[v].iter().copied());
    }
    false
}

/// Number of alignment passes (a free node may follow another free neighbour).
const ALIGN_PASSES: usize = 4;

/// Pull rooms that are FREE on one axis onto the (lower-)median coordinate their
/// perpendicular-aligning neighbours imply, so single-axis edges stay straight.
///
/// The longest-path layering orders each axis independently, so an edge that
/// constrains only one axis (e.g. a due-east edge → x only) lets its endpoints
/// drift apart on the other axis and renders diagonal/distorted. Here, for the Y
/// axis, E/W edges (`dx != 0 && dy == 0`) want their endpoints on the SAME row; a
/// node is "Y-free" iff no incident compass edge constrains its Y (`dy != 0`). Each
/// Y-free node with ≥1 E/W neighbour takes the lower-median of those neighbours' Y.
/// Symmetric for X with N/S edges. Only free nodes move, so no same-axis ordering
/// is violated. Deterministic: dense-index order, integer lower-median, fixed passes.
/// Returns `(x_constrained, y_constrained)` per dense index: whether each room has any
/// compass edge fixing its x / y. A room "free" on an axis was placed there by this pass
/// (or by stress), so callers can keep it on that row/column during collision resolution.
pub(crate) fn align_free_axes(
    graph: &MapGraph,
    index: &BTreeMap<RoomId, usize>,
    xs: &mut [i32],
    ys: &mut [i32],
) -> (Vec<bool>, Vec<bool>) {
    let n = xs.len();
    let mut x_constrained = vec![false; n];
    let mut y_constrained = vec![false; n];
    let mut ew: Vec<Vec<usize>> = vec![Vec::new(); n]; // E/W neighbours → align Y
    let mut ns: Vec<Vec<usize>> = vec![Vec::new(); n]; // N/S neighbours → align X
    for c in graph.connections() {
        if c.is_self_loop() {
            continue; // no geometry: a room is not east of itself (SQ-0666)
        }
        let (Some(&a), Some(&b)) = (index.get(&c.origin), index.get(&c.dest)) else {
            continue;
        };
        // Axis-constrained determination uses `layout_offset` so an up/down-only room
        // (Up→(0,-1)/Down→(0,1); `grid_offset` is None for both) is marked Y-constrained
        // and is NOT flattened onto a compass E/W neighbour's row by the free-axis pull.
        // `layout_offset == grid_offset` for every compass dir, so this is a no-op there.
        if let Some((dx, dy)) = layout_offset(c.dir) {
            if dx != 0 {
                x_constrained[a] = true;
                x_constrained[b] = true;
            }
            if dy != 0 {
                y_constrained[a] = true;
                y_constrained[b] = true;
            }
        }
        // Alignment neighbour lists stay `grid_offset`-only: up/down edges contribute no
        // alignment target (no column pull — that was tried and rejected for long lanes).
        // Nor does a SOFT edge (SQ-1312): a passage the story gates has no vote on which row a
        // room belongs to, and it wins by sheer count where it does. Zork I's `Living Room` has
        // one hard E/W neighbour (the `Kitchen`, on its own run's row) and one gated one (the
        // magic-word `Strange Passage`); the median of the two pulled it off the run the solve
        // had just placed it on, and the run's own passage was then drawn bent.
        if c.soft {
            continue;
        }
        if let Some((dx, dy)) = grid_offset(c.dir) {
            if dx != 0 && dy == 0 {
                ew[a].push(b);
                ew[b].push(a);
            }
            if dy != 0 && dx == 0 {
                ns[a].push(b);
                ns[b].push(a);
            }
        }
    }
    // Dedup neighbour lists (each edge pushed both directions can duplicate a node).
    for v in 0..n {
        ew[v].sort_unstable();
        ew[v].dedup();
        ns[v].sort_unstable();
        ns[v].dedup();
    }
    let median = |coords: &[i32], neigh: &[usize]| -> i32 {
        let mut vals: Vec<i32> = neigh.iter().map(|&u| coords[u]).collect();
        vals.sort_unstable();
        vals[(vals.len() - 1) / 2]
    };
    for _ in 0..ALIGN_PASSES {
        for v in 0..n {
            // Prefer CONSTRAINED neighbours (anchors) as the alignment target, so a free
            // chain inherits a real anchor's coordinate instead of drifting with its free
            // neighbours (which would otherwise out-vote a lone anchor). Fall back to all
            // neighbours when none is constrained — free chains then propagate over passes.
            if !y_constrained[v] && !ew[v].is_empty() {
                let anchors: Vec<usize> =
                    ew[v].iter().copied().filter(|&u| y_constrained[u]).collect();
                ys[v] = median(ys, if anchors.is_empty() { &ew[v] } else { &anchors });
            }
            if !x_constrained[v] && !ns[v].is_empty() {
                let anchors: Vec<usize> =
                    ns[v].iter().copied().filter(|&u| x_constrained[u]).collect();
                xs[v] = median(xs, if anchors.is_empty() { &ns[v] } else { &anchors });
            }
        }
    }
    (x_constrained, y_constrained)
}

/// Full per-component layering, packing, overlap resolution, and origin anchor.
pub(crate) fn sort_layout(graph: &MapGraph) -> BTreeMap<RoomId, (i32, i32)> {
    let mut ids: Vec<RoomId> = graph.rooms().map(|r| r.id).collect();
    ids.sort_unstable();
    let mut pos: BTreeMap<RoomId, (i32, i32)> = BTreeMap::new();
    if ids.is_empty() {
        return pos;
    }
    let components = connected_components(graph, &ids);
    let mut occupied: BTreeSet<(i32, i32)> = BTreeSet::new();
    let mut pack_x: i32 = 0;

    for comp in &components {
        let n = comp.len();
        let index: BTreeMap<RoomId, usize> =
            comp.iter().enumerate().map(|(i, &id)| (id, i)).collect();

        let mut xe: Vec<(usize, usize)> = Vec::new();
        let mut ye: Vec<(usize, usize)> = Vec::new();
        for c in graph.connections() {
            if c.is_self_loop() {
                continue; // a self-edge in a topological order is a cycle (SQ-0666)
            }
            let (Some(&a), Some(&b)) = (index.get(&c.origin), index.get(&c.dest)) else {
                continue;
            };
            // `layout_offset`, not `grid_offset` (SQ-0218): up/down seat on the N/S axis here, the
            // same as they do in the axis marking above and in the constraint engine that refines
            // this layering (`constraints.rs`). On `grid_offset` an up/down edge ordered NOTHING —
            // yet the marking above still pinned both its rooms Y-constrained, so they were held on
            // an axis nothing was ordering them on. `layout_offset == grid_offset` for every
            // compass direction, so no compass-only map moves.
            if let Some((dx, dy)) = layout_offset(c.dir) {
                if dx > 0 { xe.push((a, b)); } else if dx < 0 { xe.push((b, a)); }
                if dy > 0 { ye.push((a, b)); } else if dy < 0 { ye.push((b, a)); }
            }
        }
        let mut xs = layer_axis(n, &xe);
        let mut ys = layer_axis(n, &ye);
        // Straighten single-axis edges: pull free-axis rooms onto their neighbours.
        align_free_axes(graph, &index, &mut xs, &mut ys);

        // Normalise this component to its own origin, then pack to the right.
        let min_x = *xs.iter().min().unwrap();
        let min_y = *ys.iter().min().unwrap();
        let mut max_x_used = pack_x;
        for (i, &id) in comp.iter().enumerate() {
            let desired = (pack_x + xs[i] - min_x, ys[i] - min_y);
            let cell = nearest_free_cell(&occupied, desired);
            occupied.insert(cell);
            pos.insert(id, cell);
            max_x_used = max_x_used.max(cell.0);
        }
        pack_x = max_x_used + 2;
    }

    // Anchor the lowest-id room at (0,0).
    if let Some(&(ax, ay)) = pos.get(&ids[0]) {
        for p in pos.values_mut() {
            p.0 -= ax;
            p.1 -= ay;
        }
    }
    pos
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use crate::graph::MapGraph;
    use std::collections::BTreeSet;

    #[test]
    fn layer_axis_chain_increments() {
        // 0<1<2 : longest paths 0,1,2
        let coords = layer_axis(3, &[(0, 1), (1, 2)]);
        assert_eq!(coords, vec![0, 1, 2]);
    }

    #[test]
    fn layer_axis_breaks_cycle_deterministically() {
        // 0<1, 1<2, 2<0 (cycle) — last edge dropped; no panic, finite coords.
        let coords = layer_axis(3, &[(0, 1), (1, 2), (2, 0)]);
        assert_eq!(coords.len(), 3);
        assert!(coords[1] > coords[0] && coords[2] > coords[1]);
    }

    #[test]
    fn sort_layout_places_north_above_and_east_right() {
        let mut g = MapGraph::new();
        for id in 1..=3 { g.upsert_room(id, "r".into()); }
        g.add_edge(1, Direction::N, 2); // 2 north of 1
        g.add_edge(1, Direction::E, 3); // 3 east of 1
        let pos = sort_layout(&g);
        assert!(pos[&2].1 < pos[&1].1, "north room above");
        assert!(pos[&3].0 > pos[&1].0, "east room right");
        // no overlap
        let cells: Vec<_> = pos.values().collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len());
    }

    #[test]
    fn sort_layout_seats_up_and_down_on_the_n_s_axis() {
        // SQ-0218: the layering built its axis edge lists from `grid_offset`, which is None for
        // Up/Down — so an up/down edge ordered nothing, and its rooms were free to land in any
        // vertical order. That contradicted this same function's own axis marking a few lines
        // above, which uses `layout_offset` to mark those rooms Y-CONSTRAINED: they were pinned on
        // an axis nothing was ordering them on.
        //
        // It matters twice over: this is the whole layout above MAX_NODES rooms, and the SEED for
        // every graph below it — and the constraint engine that refines that seed already treats
        // up/down as N/S (`constraints.rs` uses `layout_offset`), so the seed was starting out
        // disagreeing with it.
        let mut g = MapGraph::new();
        for id in 1..=3 {
            g.upsert_room(id, "r".into());
        }
        g.add_edge(1, Direction::Up, 2); // 2 is "above" 1
        g.add_edge(1, Direction::Down, 3); // 3 is "below" 1
        let pos = sort_layout(&g);
        assert!(pos[&2].1 < pos[&1].1, "Up seats the destination north: {pos:?}");
        assert!(pos[&3].1 > pos[&1].1, "Down seats it south: {pos:?}");
    }

    #[test]
    fn sort_layout_keeps_compass_edges_identical() {
        // `layout_offset == grid_offset` for every compass direction, so widening the layering to
        // up/down must not move a compass-only map by a single cell.
        let mut g = MapGraph::new();
        for id in 1..=4 {
            g.upsert_room(id, "r".into());
        }
        g.add_edge(1, Direction::N, 2);
        g.add_edge(1, Direction::E, 3);
        g.add_edge(3, Direction::S, 4);
        let pos = sort_layout(&g);
        assert!(pos[&2].1 < pos[&1].1);
        assert!(pos[&3].0 > pos[&1].0);
        assert!(pos[&4].1 > pos[&3].1);
    }

    #[test]
    fn sort_layout_anchors_lowest_id_at_origin() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "a".into());
        g.upsert_room(2, "b".into());
        g.add_edge(1, Direction::E, 2);
        let pos = sort_layout(&g);
        assert_eq!(pos[&1], (0, 0));
    }

    #[test]
    fn align_pulls_free_row_onto_east_neighbour() {
        // 3 fixes 1's row (1 south of 3); 2 is due-east of 1 with no N/S edge.
        // After sort, 2 must share 1's row so 1→E→2 is straight (same y).
        let mut g = MapGraph::new();
        for id in [1u16, 2, 3] { g.upsert_room(id.into(), "r".into()); }
        g.add_edge(3, Direction::S, 1); // 1 is south of 3 → fixes y[1]
        g.add_edge(1, Direction::E, 2); // 2 due east of 1, no vertical constraint
        let pos = sort_layout(&g);
        assert_eq!(pos[&1].1, pos[&2].1, "due-east neighbour aligns onto the same row");
        assert!(pos[&2].0 > pos[&1].0, "and stays east");
    }

    #[test]
    fn align_keeps_no_overlap_and_is_deterministic() {
        let build = || {
            let mut g = MapGraph::new();
            for id in 1u16..=5 { g.upsert_room(id.into(), "r".into()); }
            g.add_edge(1, Direction::E, 2);
            g.add_edge(2, Direction::E, 3);
            g.add_edge(1, Direction::S, 4);
            g.add_edge(4, Direction::E, 5);
            g
        };
        let p1 = sort_layout(&build());
        let p2 = sort_layout(&build());
        assert_eq!(p1, p2, "deterministic");
        let cells: Vec<_> = p1.values().collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "no overlap");
    }

    #[test]
    fn free_interior_chain_aligns_to_anchor_row() {
        // #203 (Kitchen) and #193 (Living Room) connect to the house ONLY via E/W edges
        // (79→W→203, 203→W→193), so they are Y-free and must align onto #79's row rather
        // than drifting far above it. Regression: before anchor-preferring alignment they
        // landed ~4 rows up (#203 at y=-2 vs #79 at y=2).
        let mut g = MapGraph::new();
        // Note: the incidental satellite room #201 and its (203,Up,201)/(201,Down,203) edges
        // are intentionally OMITTED here. This test targets the seed-only sort_layout pipeline,
        // whose layer_axis never derives coordinates from Up/Down edges. Post-fix the align
        // stage marks an Up/Down endpoint Y-constrained (via layout_offset), so a 203→Up→201
        // edge would pin #203 at an arbitrary seed y instead of its real E/W anchor row — an
        // artefact of the seed-only stage, not the real relayout_auto pipeline (which keeps
        // 203 on 79's row: see layout::tests::a129_chain_no_foreign_interleave, full graph).
        // Removing them restores this fixture to its stated premise: #203/#193 reach the house
        // ONLY via E/W edges, so they are genuinely Y-free.
        for id in [25u16,26,27,74,75,76,77,78,79,80,81,136,143,180,193,203,239] {
            g.upsert_room(id.into(), "r".into());
        }
        use Direction::*;
        for (o, d, dst) in [
            (180,N,81),(81,W,180),(180,W,78),(78,N,143),(143,E,77),(77,S,74),(74,S,76),
            (76,W,78),(143,W,78),(78,S,76),(76,N,74),(74,E,25),(25,W,76),(74,W,79),(79,E,74),
            (25,E,26),(26,Up,25),(78,E,75),(77,E,239),(239,N,77),(77,Unknown,180),(180,S,80),
            (80,W,180),(80,E,79),(79,S,80),(79,N,81),(81,E,79),(80,S,76),(76,Unknown,180),
            (79,Unknown,180),(75,S,81),(75,W,78),(75,E,77),(239,S,77),(77,W,75),(75,N,143),
            (143,S,75),(26,Down,27),(27,N,136),(136,SW,27),(27,Up,26),(26,Unknown,180),
            (79,W,203),(203,W,193),(193,E,203),(203,E,79),
        ] { g.add_edge(o, d, dst); }
        let pos = sort_layout(&g);
        for (&id, &p) in &pos {
            g.set_pos(id, p);
        }
        crate::layout::mark_distorted(&mut g, &std::collections::BTreeSet::new());
        let y = |id| g.room(id).unwrap().pos.unwrap().1;
        assert!((y(203) - y(79)).abs() <= 1,
            "Kitchen must sit near Behind House's row: 203 y={}, 79 y={}", y(203), y(79));
        assert!((y(193) - y(79)).abs() <= 1,
            "Living Room must sit near Behind House's row: 193 y={}, 79 y={}", y(193), y(79));
    }

    #[test]
    fn a129_alignment_straightens_pendant_edges() {
        let mut g = MapGraph::new();
        for (id, name) in [
            (25, "Canyon View"), (26, "Rocky Ledge"), (74, "Clearing"), (75, "Forest Path"),
            (76, "Forest"), (77, "Forest"), (78, "Forest"), (79, "Behind House"),
            (80, "South of House"), (81, "North of House"), (143, "Clearing"),
            (180, "West of House"), (239, "Forest"),
        ] { g.upsert_room(id, name.into()); }
        for (o, d, dst) in [
            (180, Direction::N, 81), (81, Direction::W, 180), (180, Direction::W, 78),
            (78, Direction::N, 143), (143, Direction::E, 77), (77, Direction::S, 74),
            (74, Direction::S, 76), (76, Direction::W, 78), (143, Direction::W, 78),
            (78, Direction::S, 76), (76, Direction::N, 74), (74, Direction::E, 25),
            (25, Direction::W, 76), (74, Direction::W, 79), (79, Direction::E, 74),
            (25, Direction::E, 26), (78, Direction::E, 75), (77, Direction::E, 239),
            (239, Direction::N, 77), (180, Direction::S, 80), (80, Direction::W, 180),
            (80, Direction::E, 79), (79, Direction::S, 80), (79, Direction::N, 81),
            (81, Direction::E, 79), (80, Direction::S, 76),
        ] { g.add_edge(o, d, dst); }
        let pos = sort_layout(&g);
        for (&id, &p) in &pos {
            g.set_pos(id, p);
        }
        crate::layout::mark_distorted(&mut g, &std::collections::BTreeSet::new());
        let e = |o, d| g.connections().iter().find(|c| c.origin == o && c.dest == d).unwrap();
        assert!(!e(74, 25).distorted, "74→E→25 must be straight after row alignment");
        assert!(!e(78, 75).distorted, "78→E→75 must be straight after row alignment");
        let distorted = g.connections().iter().filter(|c| c.distorted).count();
        assert!(distorted <= 19, "alignment must not increase total distortion; got {distorted}");
    }
}
