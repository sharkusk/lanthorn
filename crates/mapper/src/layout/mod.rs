//! Logical room layout for the automapper (VM- and pixel-agnostic).
//!
//! Two regimes produce room grid positions; both keep rooms on integer cells and
//! never overlap:
//!
//! 1. **Incremental placement** (the per-turn path, in `incremental.rs`).
//!    `place_incremental` places ONE newly discovered room relative to the previous
//!    room: a planar compass move offsets by `grid_offset(dir)` and, on collision,
//!    shifts the rooms beyond the insertion point ("shift-beyond"); portal/unknown
//!    moves use `nearest_free_cell`. Existing rooms otherwise never move, so the map
//!    is stable turn-to-turn. `Mapper::observe` drives this.
//!
//! 2. **Constrained stress majorization** (`relayout_auto`, on demand — not per turn).
//!    For graphs with ≤ `MAX_NODES` rooms, re-derives all positions via SMACOF stress
//!    minimization with VPSC separation constraints (`stress.rs`, `vpsc.rs`,
//!    `constraints.rs`), seeded from the longest-path sort (`sort.rs`). Cycle-closing
//!    compass constraints are dropped (and the affected edges flagged distorted).
//!    For graphs above `MAX_NODES`, falls back to the **longest-path sort** directly.
//!    In both cases, components are packed left-to-right and the lowest-id room anchored
//!    at (0,0); residual collisions are resolved on the grid, keeping an aligned room on
//!    its row/column where possible (`place_preserving_alignment`) before spiralling.
//!
//! After either regime, `mark_distorted` flags every compass edge whose final grid
//! geometry contradicts its direction. (Connector routing and any render-aware
//! overlap cleanup live in the `app` crate, not here.)

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::direction::{grid_offset, layout_offset, Direction};
use crate::graph::{Connection, MapGraph, RoomId};

mod sort;
mod incremental;
mod vpsc;
mod constraints;
mod stress;
mod chains;
pub use incremental::place_incremental;
pub use chains::{detect_chains, Chains};
pub use constraints::positionally_unreliable;

/// Separation gap and ideal edge length (in grid cells).
const GAP: f64 = 1.0;
/// Fixed SMACOF iterations (determinism + bounded cost).
const ITERS: usize = 60;
/// Above this room count, skip the O(ITERS·n²) solve and use the longest-path sort.
const MAX_NODES: usize = 400;

// ── Public helpers ────────────────────────────────────────────────────────────

/// Returns the set of all grid cells currently occupied by a placed room.
pub fn occupied_cells(graph: &MapGraph) -> BTreeSet<(i32, i32)> {
    graph.rooms().filter_map(|r| r.pos).collect()
}

/// Occupied grid cells among rooms in `layer` only.
pub fn occupied_cells_in_layer(graph: &MapGraph, layer: crate::layer::LayerId) -> BTreeSet<(i32, i32)> {
    graph.rooms().filter(|r| r.layer == layer).filter_map(|r| r.pos).collect()
}

/// Spiral-search outward from `from` and return the first cell not in `occupied`.
/// Returns `from` itself if it is free.
pub fn nearest_free_cell(occupied: &BTreeSet<(i32, i32)>, from: (i32, i32)) -> (i32, i32) {
    if !occupied.contains(&from) {
        return from;
    }
    // Spiral outward: for radius r=1,2,… walk the perimeter of the square [−r..=r]×[−r..=r].
    for r in 1_i32.. {
        // Top row: y = from.1 - r, x from from.0-r to from.0+r
        for x in (from.0 - r)..=(from.0 + r) {
            let cell = (x, from.1 - r);
            if !occupied.contains(&cell) {
                return cell;
            }
        }
        // Bottom row: y = from.1 + r
        for x in (from.0 - r)..=(from.0 + r) {
            let cell = (x, from.1 + r);
            if !occupied.contains(&cell) {
                return cell;
            }
        }
        // Left column: x = from.0 - r, y from from.1-r+1 to from.1+r-1
        for y in (from.1 - r + 1)..=(from.1 + r - 1) {
            let cell = (from.0 - r, y);
            if !occupied.contains(&cell) {
                return cell;
            }
        }
        // Right column: x = from.0 + r
        for y in (from.1 - r + 1)..=(from.1 + r - 1) {
            let cell = (from.0 + r, y);
            if !occupied.contains(&cell) {
                return cell;
            }
        }
    }
    unreachable!("infinite grid always has a free cell")
}

/// Resolve a collision while preserving an aligned axis. When the room is free on Y
/// (its row is meaningful) but constrained on X, search ALONG X keeping the row; when
/// free on X but constrained on Y, search along Y keeping the column. Otherwise (free on
/// both — e.g. portal-only — or neither) fall back to the spiral `nearest_free_cell`.
/// This keeps an aligned cardinal chain on one row/column under collision resolution.
fn place_preserving_alignment(
    occupied: &BTreeSet<(i32, i32)>,
    from: (i32, i32),
    row_aligned: bool,
    col_aligned: bool,
) -> (i32, i32) {
    if !occupied.contains(&from) {
        return from;
    }
    if row_aligned && !col_aligned {
        for d in 1.. {
            for cand in [(from.0 - d, from.1), (from.0 + d, from.1)] {
                if !occupied.contains(&cand) {
                    return cand;
                }
            }
        }
    } else if col_aligned && !row_aligned {
        for d in 1.. {
            for cand in [(from.0, from.1 - d), (from.0, from.1 + d)] {
                if !occupied.contains(&cand) {
                    return cand;
                }
            }
        }
    }
    nearest_free_cell(occupied, from)
}

/// Returns true iff the connection's geometry is satisfied by the current room positions.
///
/// For an edge with a layout offset (one where `layout_offset(conn.dir)` returns `Some(delta)` —
/// the compass directions, plus Up/Down as weight-1 N/S hints):
///   - Uses a sign-based check: satisfied iff each non-zero axis of `delta` agrees in SIGN
///     with the corresponding axis of `pos(dest) - pos(origin)`, and each zero axis of `delta`
///     is also zero in the actual offset.
///   - Rationale: when both directed edges of a connection are known, the layout may place
///     a room at a combined diagonal (e.g. northeast) that doesn't exactly match `layout_offset`
///     (e.g. one step north). The sign-based check treats such placements as "satisfied" as long
///     as the directional sense is correct (e.g. a North edge is satisfied whenever dest is
///     *anywhere* north, i.e. `dest.y < origin.y`).
///
/// For a non-compass edge (In/Out/Unknown, where `layout_offset` returns `None`):
///   - returns `true` unconditionally. These edges are stubs with no spatial offset to violate;
///     treating them as "satisfied" ensures the post-placement sweep never marks them distorted.
///     (Note: `mark_distorted` still gates on `grid_offset`, so Up/Down are never marked
///     distorted regardless of what this function returns for them.)
pub fn edge_is_satisfied(graph: &MapGraph, conn: &Connection) -> bool {
    match layout_offset(conn.dir) {
        None => true, // non-compass stub — no offset to violate
        Some(delta) => {
            let origin_pos = graph.room(conn.origin).and_then(|r| r.pos);
            let dest_pos = graph.room(conn.dest).and_then(|r| r.pos);
            match (origin_pos, dest_pos) {
                (Some(op), Some(dp)) => {
                    let actual = (dp.0 - op.0, dp.1 - op.1);
                    // Sign-based: each axis of delta must agree in sign (or be zero if delta is 0).
                    axis_sign_ok(actual.0, delta.0) && axis_sign_ok(actual.1, delta.1)
                }
                _ => false, // unplaced endpoint → unsatisfied
            }
        }
    }
}

/// Returns true iff the sign of `actual` is consistent with the sign of `expected`:
/// - `expected == 0`: actual must also be 0.
/// - `expected > 0`: actual must be > 0.
/// - `expected < 0`: actual must be < 0.
fn axis_sign_ok(actual: i32, expected: i32) -> bool {
    match expected.cmp(&0) {
        std::cmp::Ordering::Equal => actual == 0,
        std::cmp::Ordering::Greater => actual > 0,
        std::cmp::Ordering::Less => actual < 0,
    }
}

// ── Core layout ───────────────────────────────────────────────────────────────

/// Connected components over the undirected projection of the graph. Each
/// component is sorted ascending; components are returned in ascending-root order.
pub(crate) fn connected_components(graph: &MapGraph, ids: &[RoomId]) -> Vec<Vec<RoomId>> {
    let mut adjacency: BTreeMap<RoomId, Vec<RoomId>> = BTreeMap::new();
    for &id in ids {
        adjacency.entry(id).or_default();
    }
    for conn in graph.connections() {
        // Unknown-direction edges (e.g. a death/respawn transition the game gave no direction for)
        // are non-spatial: they must not group rooms into a component they have no real position
        // relation to.
        if conn.dir == Direction::Unknown || conn.is_self_loop() {
            continue;
        }
        adjacency.entry(conn.origin).or_default().push(conn.dest);
        adjacency.entry(conn.dest).or_default().push(conn.origin);
    }
    for neighbors in adjacency.values_mut() {
        neighbors.sort_unstable();
        neighbors.dedup();
    }

    let mut visited: BTreeSet<RoomId> = BTreeSet::new();
    let mut components: Vec<Vec<RoomId>> = Vec::new();
    for &id in ids {
        if visited.contains(&id) {
            continue;
        }
        let mut component = Vec::new();
        let mut queue: VecDeque<RoomId> = VecDeque::new();
        queue.push_back(id);
        visited.insert(id);
        while let Some(cur) = queue.pop_front() {
            component.push(cur);
            if let Some(neighbors) = adjacency.get(&cur) {
                for &nb in neighbors {
                    if visited.insert(nb) {
                        queue.push_back(nb);
                    }
                }
            }
        }
        component.sort_unstable();
        components.push(component);
    }
    components
}

/// Largest distance searched along a chain's axis for a bump cell before falling back to a
/// perpendicular spiral. Maps are small (≤ a few hundred rooms), so this stays cheap.
const MAX_BUMP_SPAN: i32 = 16;

/// True iff `actual` is on the correct SIDE of `expected`. Unlike `axis_sign_ok` (which
/// demands `actual == 0` when `expected == 0`, i.e. exact cardinal alignment), a zero
/// `expected` imposes NO constraint here. Used to score where to relocate a bumped room:
/// we want to keep it on the right side of each neighbour, not perfectly axis-aligned
/// (which is usually impossible for a many-edged room and is the layout's general
/// distortion, not the bump's concern).
fn axis_side_respected(actual: i32, expected: i32) -> bool {
    match expected.cmp(&0) {
        std::cmp::Ordering::Equal => true, // no preference on this axis
        std::cmp::Ordering::Greater => actual > 0,
        std::cmp::Ordering::Less => actual < 0,
    }
}

/// Weighted count of room `id`'s compass edges (to already-placed component neighbours) that
/// keep `id` on the correct SIDE of the neighbour if `id` sat at `cell` (see
/// `axis_side_respected`). A RECIPROCAL connection — one with a compass edge back from the
/// neighbour to `id` — counts double: a bidirectional link is a far stronger spatial hint than
/// a one-way exit, so the layout should sacrifice a one-way hint before a reciprocal one (e.g.
/// keep #180's reciprocal N/S links to #81/#80 even if its one-way `180→W→78` must give).
/// Higher = fewer (and weaker) directional hints trampled.
const RECIPROCAL_WEIGHT: usize = 2;

fn edges_respected_at(
    graph: &MapGraph,
    index: &BTreeMap<RoomId, usize>,
    snapped: &[(i32, i32)],
    id: RoomId,
    cell: (i32, i32),
) -> usize {
    let mut sat = 0;
    for c in graph.connections() {
        let (other, is_origin) = if c.origin == id {
            (c.dest, true)
        } else if c.dest == id {
            (c.origin, false)
        } else {
            continue;
        };
        let Some(delta) = layout_offset(c.dir) else { continue };
        let Some(&oi) = index.get(&other) else { continue };
        let op = snapped[oi];
        // `actual` is always (dest - origin); flip when `id` is the destination.
        let actual = if is_origin {
            (op.0 - cell.0, op.1 - cell.1)
        } else {
            (cell.0 - op.0, cell.1 - op.1)
        };
        if axis_side_respected(actual.0, delta.0) && axis_side_respected(actual.1, delta.1) {
            // Reciprocal (bidirectional) links weigh more than one-way exits.
            let reciprocal = graph
                .connections()
                .iter()
                .any(|r| r.origin == other && r.dest == id && grid_offset(r.dir).is_some());
            sat += if reciprocal { RECIPROCAL_WEIGHT } else { 1 };
        }
    }
    sat
}

/// Reciprocal-weighted count of room `id`'s compass edges that keep it on the correct SIDE of
/// each placed neighbour at the graph's CURRENT positions (see `axis_side_respected`; a
/// bidirectional link counts double). Higher = fewer/weaker directional hints violated. Used
/// by the app's render-overlap cleanup to prefer nudges that preserve directional hints.
pub fn room_side_score(graph: &MapGraph, id: RoomId) -> usize {
    let Some(p) = graph.room(id).and_then(|r| r.pos) else { return 0 };
    let mut sat = 0;
    for c in graph.connections() {
        let (other, is_origin) = if c.origin == id {
            (c.dest, true)
        } else if c.dest == id {
            (c.origin, false)
        } else {
            continue;
        };
        let Some(delta) = layout_offset(c.dir) else { continue };
        let Some(op) = graph.room(other).and_then(|r| r.pos) else { continue };
        let actual = if is_origin {
            (op.0 - p.0, op.1 - p.1)
        } else {
            (p.0 - op.0, p.1 - op.1)
        };
        if axis_side_respected(actual.0, delta.0) && axis_side_respected(actual.1, delta.1) {
            let reciprocal = graph
                .connections()
                .iter()
                .any(|r| r.origin == other && r.dest == id && grid_offset(r.dir).is_some());
            sat += if reciprocal { RECIPROCAL_WEIGHT } else { 1 };
        }
    }
    sat
}

/// Like [`room_side_score`] but STRICT: a cardinal edge counts only when its CROSS axis is exactly
/// aligned (column-exact for N/S, row-exact for E/W) — i.e. the edge is [`edge_is_satisfied`], not
/// merely on the right side. Reciprocal-chain edges weigh more. Overlap cleanup uses this to avoid
/// knocking a room off an exact row/column it shares with a neighbour: `room_side_score`, being
/// side-only (`axis_side_respected` ignores the cross axis), treats "below-and-west of X" as just as
/// good as "exactly below X", so it cannot protect a column/row chain when relocating a room.
pub fn room_alignment_score(graph: &MapGraph, id: RoomId) -> usize {
    let Some(p) = graph.room(id).and_then(|r| r.pos) else { return 0 };
    let mut sat = 0;
    for c in graph.connections() {
        let (other, is_origin) = if c.origin == id {
            (c.dest, true)
        } else if c.dest == id {
            (c.origin, false)
        } else {
            continue;
        };
        let Some(delta) = layout_offset(c.dir) else { continue };
        let Some(op) = graph.room(other).and_then(|r| r.pos) else { continue };
        let actual = if is_origin {
            (op.0 - p.0, op.1 - p.1)
        } else {
            (p.0 - op.0, p.1 - op.1)
        };
        if axis_sign_ok(actual.0, delta.0) && axis_sign_ok(actual.1, delta.1) {
            let reciprocal = graph
                .connections()
                .iter()
                .any(|r| r.origin == other && r.dest == id && grid_offset(r.dir).is_some());
            sat += if reciprocal { RECIPROCAL_WEIGHT } else { 1 };
        }
    }
    sat
}

/// Total directional-hint satisfaction across the whole map: the count of compass connections whose
/// dest sits on the correct SIDE of its origin (side-only, like `room_side_score` — the cross axis
/// is free). Each DIRECTED connection counts once, so a reciprocal pair contributes 2, naturally
/// weighting bidirectional links above one-way exits without a separate weight. The directional
/// repair pass maximizes this (subject to not adding illegal overlaps).
///
/// **A satisfied Up/Down hint is worth strictly less than any satisfied compass hint** (SQ-1291),
/// the same rule `constraints::build_axis_constraints` sorts its tiers by: north-for-up is a
/// drawing convention this crate invents in [`layout_offset`], where a compass word is the game's
/// own statement of where the room lies. A compass hint is therefore weighed at more than every
/// stairwell on the map put together, which makes the sum a LEXICOGRAPHIC comparison — compass
/// hints first, stairwells only as the tie-break. Zork I's `East-West Passage` needed it: the
/// solver had already placed the `Chasm` north-east of it, honouring the chasm's own `southwest`
/// return, and this pass then dragged the chasm due south to satisfy the two legs of the
/// stairwell instead — which is the map contradicting the game's prose to draw a staircase
/// straight down. Scores are only ever compared WITHIN one graph (the repair pass's hill climb),
/// so a weight derived from that graph's connection count is well defined.
pub fn directional_hint_score(graph: &MapGraph) -> usize {
    let compass_weight = graph.connections().len() + 1;
    graph
        .connections()
        .iter()
        .filter_map(|c| {
            let delta = layout_offset(c.dir)?;
            let (op, dp) = (
                graph.room(c.origin).and_then(|r| r.pos)?,
                graph.room(c.dest).and_then(|r| r.pos)?,
            );
            let actual = (dp.0 - op.0, dp.1 - op.1);
            if axis_side_respected(actual.0, delta.0) && axis_side_respected(actual.1, delta.1) {
                Some(if grid_offset(c.dir).is_some() { compass_weight } else { 1 })
            } else {
                None
            }
        })
        .sum()
}

/// Number of room `id`'s compass (grid-offset) edges — its directional-constraint count.
/// A room with FEWER compass edges (e.g. a portal-only or leaf room) is a safer room to nudge
/// when resolving a rendered overlap, since moving it disturbs fewer directional hints.
pub fn room_compass_degree(graph: &MapGraph, id: RoomId) -> usize {
    graph
        .connections()
        .iter()
        .filter(|c| {
            (c.origin == id || c.dest == id) && grid_offset(c.dir).is_some() && !c.is_self_loop()
        })
        .count()
}

/// A chain's occupied line: `horizontal` true for an E/W chain (`line` = shared row y,
/// `lo..=hi` = member x-extent), false for an N/S chain (`line` = shared column x, `lo..=hi`
/// = member y-extent).
struct ChainSpan {
    horizontal: bool,
    line: i32,
    lo: i32,
    hi: i32,
}

/// Relocate every FOREIGN room that lies within a chain's member span on the chain line (the
/// shared row for an E/W chain, shared column for an N/S chain) — including a room that rounds
/// onto an ENDPOINT member's own cell. Chain MEMBERS are never moved — moving them would
/// collapse their own diagonals to non-chain neighbours. An ejected room may exit either ALONG
/// the line (past the member span) or OFF the line entirely; the destination is the free,
/// off-span cell that respects the most of the ejected room's own (reciprocal-weighted) compass
/// edges, then nearest, then west/north.
///
/// "Member" here means a member of ANY chain, not just the one whose span is being cleared
/// (SQ-1309): a room that shares a row with one set of neighbours can easily sit, by sheer
/// coincidence of where the stress solve put it, inside a completely unrelated column chain's
/// span. Excluding only the current chain's own members let that unrelated chain treat a real
/// E/W chain member as a foreign interloper and eject it off its own row — Zork I's East-West
/// Passage was pulled off the Round Room/Troll Room row this way, evicted by an unrelated
/// column chain (rooms #128/#229) it happened to cross. `protected` is every room this
/// component's `contiguify` pass has ANY chain claim on, so no chain's cleanup can undo another
/// chain's alignment.
fn eject_interlopers(
    snapped: &mut [(i32, i32)],
    protected: &BTreeSet<usize>,
    span: ChainSpan,
    comp: &[RoomId],
    index: &BTreeMap<RoomId, usize>,
    graph: &MapGraph,
) {
    let ChainSpan { horizontal, line, lo, hi } = span;
    // A cell is "on the span" iff it is ON the chain line and within the member extent,
    // endpoints INCLUDED: a foreign room that rounds onto an endpoint member's cell is an
    // overlap the later collision pass would resolve by shoving the member off its chain.
    let between = |c: (i32, i32)| -> bool {
        let (perp, par) = if horizontal { (c.1, c.0) } else { (c.0, c.1) };
        perp == line && par >= lo && par <= hi
    };
    loop {
        let victim = (0..snapped.len())
            .find(|&q| !protected.contains(&q) && between(snapped[q]));
        let Some(q) = victim else { break };
        let from = snapped[q];
        let occ: BTreeSet<(i32, i32)> =
            (0..snapped.len()).filter(|&k| k != q).map(|k| snapped[k]).collect();
        let id = comp[q];
        let par = if horizontal { from.0 } else { from.1 };
        // Candidate exits: ALONG the line just past each member end, and OFF the line at the
        // room's current parallel coordinate. Both kinds leave the "between" zone.
        let mut cands: Vec<(i32, i32)> = Vec::new();
        for d in 1..=MAX_BUMP_SPAN {
            if horizontal {
                cands.push((lo - d, line)); // along row, west of the span
                cands.push((hi + d, line)); // along row, east of the span
                cands.push((par, line - d)); // off row, north
                cands.push((par, line + d)); // off row, south
            } else {
                cands.push((line, lo - d)); // along column, north of the span
                cands.push((line, hi + d)); // along column, south of the span
                cands.push((line - d, par)); // off column, west
                cands.push((line + d, par)); // off column, east
            }
        }
        let dest = cands
            .into_iter()
            .filter(|&c| !occ.contains(&c) && !between(c))
            .min_by_key(|&c| {
                // Most hints respected first; then nearest; then west, then north (deterministic).
                let manh = (c.0 - from.0).abs() + (c.1 - from.1).abs();
                (
                    std::cmp::Reverse(edges_respected_at(graph, index, snapped, id, c)),
                    manh,
                    c.0,
                    c.1,
                )
            })
            .unwrap_or_else(|| {
                // Fallback (every along/off-line candidate occupied): spiral to the nearest free
                // cell, but treat the whole between-zone as occupied so the victim cannot land
                // back inside it — otherwise it would be re-selected next iteration (a loop).
                let mut occ2 = occ.clone();
                if horizontal {
                    for x in (lo + 1)..hi { occ2.insert((x, line)); }
                } else {
                    for y in (lo + 1)..hi { occ2.insert((line, y)); }
                }
                nearest_free_cell(&occ2, from)
            });
        debug_assert!(!between(dest), "ejected room must leave the between-zone");
        snapped[q] = dest;
    }
}

fn contiguify(
    chains: &Chains,
    comp: &[RoomId],
    index: &BTreeMap<RoomId, usize>,
    snapped: &mut [(i32, i32)],
    graph: &MapGraph,
) {
    // Eject foreign interlopers from between chain members; never move members (see
    // `eject_interlopers`). Chains may legitimately keep gaps between members — only a foreign
    // room sitting *between* them is a problem (it would interleave the chain visually).
    //
    // `protected` covers every room this component's EW or NS chains claim at all (SQ-1309):
    // a room that is legitimately a member of one chain must not be treated as an interloper
    // and evicted by a DIFFERENT, unrelated chain whose span it happens to cross.
    let protected: BTreeSet<usize> = chains
        .ew_members
        .iter()
        .chain(chains.ns_members.iter())
        .flat_map(|members| members.iter().filter_map(|id| index.get(id).copied()))
        .collect();
    let mut snapped_v: Vec<(i32, i32)> = snapped.to_vec();
    for members in &chains.ew_members {
        let idxs: Vec<usize> = members.iter().filter_map(|id| index.get(id).copied()).collect();
        if idxs.len() < 2 { continue; }
        let line = snapped_v[idxs[0]].1; // shared row
        if !idxs.iter().all(|&i| snapped_v[i].1 == line) { continue; } // dropped equality → skip
        let lo = idxs.iter().map(|&i| snapped_v[i].0).min().unwrap();
        let hi = idxs.iter().map(|&i| snapped_v[i].0).max().unwrap();
        let span = ChainSpan { horizontal: true, line, lo, hi };
        eject_interlopers(&mut snapped_v, &protected, span, comp, index, graph);
    }
    for members in &chains.ns_members {
        let idxs: Vec<usize> = members.iter().filter_map(|id| index.get(id).copied()).collect();
        if idxs.len() < 2 { continue; }
        let line = snapped_v[idxs[0]].0; // shared column
        if !idxs.iter().all(|&i| snapped_v[i].0 == line) { continue; }
        let lo = idxs.iter().map(|&i| snapped_v[i].1).min().unwrap();
        let hi = idxs.iter().map(|&i| snapped_v[i].1).max().unwrap();
        let span = ChainSpan { horizontal: false, line, lo, hi };
        eject_interlopers(&mut snapped_v, &protected, span, comp, index, graph);
    }
    snapped.copy_from_slice(&snapped_v);
}

// ── Observer types ────────────────────────────────────────────────────────────

/// Cumulative statistics reported to a [`TidyObserver`] at each layout stage.
///
/// Counts are cumulative from the start of `relayout_auto_observed`:
/// - `rooms_moved`: rooms displaced from their snapped cell during collision
///   resolution (pack + collision-resolve stage). A room that lands exactly on
///   its snapped cell is not counted.
/// - `overlaps_resolved`: reserved for the app-side cleanup passes; always 0
///   inside `relayout_auto_observed`.
/// - `constraints_dropped`: axis-separation constraints that had to be dropped
///   because they would have introduced a cycle in the precedence graph (cycle-
///   closing compass edges). One dropped constraint per affected connection index,
///   accumulated across all components before the SMACOF stage.
/// - `hints_repaired`: reserved for the app-side repair pass; always 0 here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TidyStats {
    pub rooms_moved: u32,
    pub overlaps_resolved: u32,
    pub constraints_dropped: u32,
    pub hints_repaired: u32,
}

/// Observer callback for [`relayout_auto_observed`].
///
/// Called after each internal layout stage with:
/// - the CURRENT graph (positions reflect the stage just completed),
/// - a short `label` (e.g. `"seed"`),
/// - a one-line `description` of the algorithm,
/// - cumulative [`TidyStats`] since the start of the call.
///
/// The `None` path in `relayout_auto_observed` is allocation-free; the observer
/// is only constructed and called when the caller opts in.
pub type TidyObserver<'a> = &'a mut dyn FnMut(&MapGraph, &str, &str, &TidyStats);

// ── Distortion marker ─────────────────────────────────────────────────────────

/// Set the `distorted` flag on every connection: a compass edge is distorted if
/// its connection index is in `dropped`, or its final grid geometry violates its
/// direction. Non-compass edges are never distorted.
pub(crate) fn mark_distorted(graph: &mut MapGraph, dropped: &BTreeSet<usize>) {
    let n_conns = graph.connections().len();
    for idx in 0..n_conns {
        let conn = graph.connections()[idx].clone();
        // A self-loop has no geometry to violate — it is never distorted (SQ-0666).
        let distorted = match grid_offset(conn.dir) {
            None => false,
            Some(_) if conn.is_self_loop() => false,
            Some(_) => dropped.contains(&idx) || !edge_is_satisfied(graph, &conn),
        };
        graph.set_conn_distorted(idx, distorted);
    }
}

/// Re-derive all room positions from scratch on every call.
///
/// Delegates to [`relayout_auto_observed`] with no observer. The graph result
/// is identical to calling `relayout_auto_observed(graph, None)`.
///
/// For graphs with ≤ MAX_NODES rooms, uses constrained stress-majorization
/// (SMACOF + VPSC) seeded from the longest-path sort. For larger graphs,
/// falls back to the longest-path sort directly. Components are packed
/// left-to-right, residual overlaps resolved, and the lowest-id room
/// anchored at (0,0).
pub fn relayout_auto(graph: &mut MapGraph) {
    relayout_auto_observed(graph, None);
}

/// Re-derive all room positions from scratch, optionally notifying an observer
/// after each internal stage.
///
/// When `obs` is `None` this is exactly `relayout_auto` — zero overhead, same
/// final graph state. When `obs` is `Some`, the callback is invoked after each
/// of the 5 layout stages with the current (partial) graph state, a short label,
/// a one-line description, and cumulative [`TidyStats`].
///
/// The 5 stages emitted (in order):
/// 1. `"seed"` — Longest-path layering: integer coords per axis from compass edges.
/// 2. `"stress"` — Stress majorization: places rooms by graph-theoretic distance
///    under VPSC compass-separation constraints.
/// 3. `"align"` — Align free axes: pull single-axis-free rooms onto their
///    neighbour's row/column so cardinal edges render straight.
/// 4. `"contiguify"` — Contiguity: eject foreign rooms interleaved within a
///    chain's span.
/// 5. `"pack"` — Pack components left-to-right; resolve residual same-cell
///    collisions, keeping aligned rooms on their line.
///
/// Intermediate graph positions are written before each observer call so the
/// observer sees a faithful snapshot. The final positions after stage 5 are
/// identical to those produced by `relayout_auto` with no observer.
pub fn relayout_auto_observed(graph: &mut MapGraph, mut obs: Option<TidyObserver<'_>>) {
    let mut ids: Vec<RoomId> = graph.rooms().map(|r| r.id).collect();
    ids.sort_unstable();
    if ids.is_empty() {
        return;
    }

    let mut stats = TidyStats::default();

    // Large graphs: skip the O(ITERS·n²) solve and use the longest-path sort.
    if ids.len() > MAX_NODES {
        let pos = sort::sort_layout(graph);
        for (&id, &p) in &pos {
            graph.set_pos(id, p);
        }
        mark_distorted(graph, &BTreeSet::new());
        if let Some(ref mut cb) = obs {
            cb(graph, "seed",
               "Longest-path layering: integer coords per axis from compass edges.",
               &stats);
        }
        return;
    }

    // Stage 1: seed from the longest-path sort (deterministic, roughly compass-ordered).
    let seed = sort::sort_layout(graph);

    // Apply seed positions to the graph for the stage-1 snapshot.
    if obs.is_some() {
        for (&id, &p) in &seed {
            graph.set_pos(id, p);
        }
    }
    if let Some(ref mut cb) = obs {
        cb(graph, "seed",
           "Longest-path layering: integer coords per axis from compass edges.",
           &stats);
    }

    let chains_for_comp = detect_chains(graph);
    // Rooms whose own compass claims contradict each other (SQ-1289). Their edges make no
    // separation constraints, so they take whatever cell is left over — which is only true if
    // they claim one LAST, below, after every reliable room has had its pick.
    let unreliable = constraints::positionally_unreliable(graph);
    let components = connected_components(graph, &ids);
    let mut dropped_all: BTreeSet<usize> = BTreeSet::new();
    let mut final_pos: BTreeMap<RoomId, (i32, i32)> = BTreeMap::new();
    let mut occupied: BTreeSet<(i32, i32)> = BTreeSet::new();
    let mut pack_x: i32 = 0;

    // Per-component intermediate snapshots for stages 2–4 accumulate into these
    // component-indexed vectors so we can apply them all at once for the snapshot.
    // We store: snapped_after_stress, snapped_after_align, snapped_after_contiguify.
    // Format: BTreeMap<RoomId, (i32,i32)> per stage.
    let mut snap_stress: BTreeMap<RoomId, (i32, i32)> = BTreeMap::new();
    let mut snap_align: BTreeMap<RoomId, (i32, i32)> = BTreeMap::new();
    let mut snap_contiguify: BTreeMap<RoomId, (i32, i32)> = BTreeMap::new();
    // x_constrained/y_constrained are needed for the pack stage; accumulate per-comp.
    let mut x_constrained_all: BTreeMap<RoomId, bool> = BTreeMap::new();
    let mut y_constrained_all: BTreeMap<RoomId, bool> = BTreeMap::new();

    for comp in &components {
        let n = comp.len();
        let index: BTreeMap<RoomId, usize> =
            comp.iter().enumerate().map(|(i, &id)| (id, i)).collect();

        // Local undirected adjacency for BFS distances. Unknown-direction edges are non-spatial and
        // excluded, so they exert no graph-distance pull in the stress solve (the real compass/
        // up-down structure decides positions).
        let mut adj = vec![Vec::new(); n];
        for c in graph.connections() {
            if c.dir == Direction::Unknown || c.is_self_loop() {
                continue;
            }
            if let (Some(&a), Some(&b)) = (index.get(&c.origin), index.get(&c.dest)) {
                adj[a].push(b);
                adj[b].push(a);
            }
        }
        let dist = stress::all_pairs_dist(n, &adj);
        let ac = constraints::build_axis_constraints(graph, comp, GAP);
        dropped_all.extend(ac.dropped.iter().copied());

        let seed_local: Vec<(f64, f64)> = comp
            .iter()
            .map(|&id| {
                let p = seed.get(&id).copied().unwrap_or((0, 0));
                (p.0 as f64, p.1 as f64)
            })
            .collect();

        let cont = stress::stress_layout(n, &dist, &ac.x, &ac.y, &seed_local, ITERS);
        let mut snapped: Vec<(i32, i32)> =
            cont.iter().map(|&(x, y)| (x.round() as i32, y.round() as i32)).collect();

        if obs.is_some() {
            for (i, &id) in comp.iter().enumerate() {
                snap_stress.insert(id, snapped[i]);
            }
        }

        // Align cardinal-edge free axes. The stress solve satisfies separation (B is
        // east of A) but leaves an E/W chain's rooms on slightly different rows (and
        // N/S chains on different columns). Pull each room that is free on the
        // perpendicular axis onto its anchor's row/column, so cardinal edges render
        // crisp — the same alignment the sort fallback applies.
        let mut axs: Vec<i32> = snapped.iter().map(|p| p.0).collect();
        let mut ays: Vec<i32> = snapped.iter().map(|p| p.1).collect();
        let (x_constrained, y_constrained) =
            sort::align_free_axes(graph, &index, &mut axs, &mut ays);
        for (i, p) in snapped.iter_mut().enumerate() {
            *p = (axs[i], ays[i]);
        }

        if obs.is_some() {
            for (i, &id) in comp.iter().enumerate() {
                snap_align.insert(id, snapped[i]);
                x_constrained_all.insert(id, x_constrained[i]);
                y_constrained_all.insert(id, y_constrained[i]);
            }
        } else {
            for (i, &id) in comp.iter().enumerate() {
                x_constrained_all.insert(id, x_constrained[i]);
                y_constrained_all.insert(id, y_constrained[i]);
            }
        }

        contiguify(&chains_for_comp, comp, &index, &mut snapped, graph);

        if obs.is_some() {
            for (i, &id) in comp.iter().enumerate() {
                snap_contiguify.insert(id, snapped[i]);
            }
        }

        // Pack this component to the right of the previous, top-aligned.
        let min_x = snapped.iter().map(|p| p.0).min().unwrap();
        let min_y = snapped.iter().map(|p| p.1).min().unwrap();
        for p in &mut snapped {
            p.0 += pack_x - min_x;
            p.1 -= min_y;
        }

        // Resolve residual same-cell collisions in ascending room-id order. Keep an
        // axis-aligned room on its row/column (shift ALONG the aligned axis) instead of
        // spiraling off it, so collision resolution doesn't re-distort an aligned chain
        // (e.g. #193 bumped off #203's row by #180).
        let mut max_x_used = pack_x;
        // Reliable rooms first, then the positionally unreliable ones (SQ-1289) — a room with
        // no geometry of its own must never bump one that has geometry off its cell. Stable
        // within each group, so the pass stays deterministic.
        let mut claim_order: Vec<usize> = (0..comp.len()).collect();
        claim_order.sort_by_key(|&i| unreliable.contains(&comp[i]));
        for i in claim_order {
            let id = comp[i];
            let row_aligned = !y_constrained[i];
            let col_aligned = !x_constrained[i];
            let before = snapped[i];
            let cell = place_preserving_alignment(&occupied, snapped[i], row_aligned, col_aligned);
            if cell != before {
                stats.rooms_moved += 1;
            }
            occupied.insert(cell);
            final_pos.insert(id, cell);
            max_x_used = max_x_used.max(cell.0);
        }
        pack_x = max_x_used + 2; // 1-cell gap between components
    }

    // Stage 2 observer snapshot: stress positions (unnormalized — no pack/anchor yet).
    if obs.is_some() {
        stats.constraints_dropped = dropped_all.len() as u32;
        for (&id, &p) in &snap_stress {
            graph.set_pos(id, p);
        }
        if let Some(ref mut cb) = obs {
            cb(graph, "stress",
               "Stress majorization: places rooms by graph-theoretic distance under VPSC compass-separation constraints.",
               &stats);
        }
        // Stage 3: axis-align snapshot.
        for (&id, &p) in &snap_align {
            graph.set_pos(id, p);
        }
        if let Some(ref mut cb) = obs {
            cb(graph, "align",
               "Align free axes: pull single-axis-free rooms onto their neighbour's row/column so cardinal edges render straight.",
               &stats);
        }
        // Stage 4: contiguify snapshot.
        for (&id, &p) in &snap_contiguify {
            graph.set_pos(id, p);
        }
        if let Some(ref mut cb) = obs {
            cb(graph, "contiguify",
               "Contiguity: eject foreign rooms interleaved within a chain's span.",
               &stats);
        }
    } else {
        stats.constraints_dropped = dropped_all.len() as u32;
    }

    // Anchor the lowest-id room at (0,0) for a stable reference.
    if let Some(&(ax, ay)) = final_pos.get(&ids[0]) {
        for p in final_pos.values_mut() {
            p.0 -= ax;
            p.1 -= ay;
        }
    }

    for (&id, &p) in &final_pos {
        graph.set_pos(id, p);
    }
    mark_distorted(graph, &dropped_all);

    // Stage 5: pack + collision-resolve — the final graph state.
    if let Some(ref mut cb) = obs {
        cb(graph, "pack",
           "Pack components left-to-right; resolve residual same-cell collisions, keeping aligned rooms on their line.",
           &stats);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use crate::mapper::Mapper;

    #[test]
    fn unknown_edges_are_non_spatial_for_components() {
        // An Unknown-direction edge (e.g. a death/respawn transition) must not group two rooms into
        // one layout component or pull them together; a real compass edge does.
        let mut g = MapGraph::new();
        g.upsert_room(1, "a".into());
        g.upsert_room(2, "b".into());
        g.add_edge(1, Direction::Unknown, 2);
        assert_eq!(connected_components(&g, &[1, 2]).len(), 2, "Unknown edge does not connect a component");

        let mut h = MapGraph::new();
        h.upsert_room(1, "a".into());
        h.upsert_room(2, "b".into());
        h.add_edge(1, Direction::E, 2);
        assert_eq!(connected_components(&h, &[1, 2]).len(), 1, "a compass edge does connect a component");
    }

    #[test]
    fn unknown_edge_does_not_change_relayout() {
        // A phantom 1<->3 Unknown edge (alongside the real 1-E-2-E-3 chain) must not pull on the
        // stress solve: relayout is identical whether or not the Unknown edge is present.
        let build = |unknown: bool| {
            let mut g = MapGraph::new();
            for id in [1u16, 2, 3] {
                g.upsert_room(id.into(), "r".into());
            }
            g.add_edge(1, Direction::E, 2);
            g.add_edge(2, Direction::E, 3);
            if unknown {
                g.add_edge(1, Direction::Unknown, 3);
            }
            g
        };
        let (mut a, mut b) = (build(true), build(false));
        relayout_auto(&mut a);
        relayout_auto(&mut b);
        let pa: Vec<_> = a.rooms().map(|r| (r.id, r.pos)).collect();
        let pb: Vec<_> = b.rooms().map(|r| (r.id, r.pos)).collect();
        assert_eq!(pa, pb, "an Unknown edge must not change the relayout");
    }

    #[test]
    fn directional_hint_score_counts_satisfied_sides_with_reciprocal_weight() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "a".into());
        g.upsert_room(2, "b".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0)); // 2 east of 1
        g.add_edge(1, Direction::E, 2); // satisfied (2 is east)
        g.add_edge(2, Direction::W, 1); // satisfied (1 is west) — reciprocal, so both count
        let both = directional_hint_score(&g);
        // Flip 2 to the wrong side: both directed edges now violated.
        g.set_pos(2, (-1, 0));
        assert_eq!(directional_hint_score(&g), 0, "2 west of 1 violates both E and W edges");

        // Half the pair, half the score. SQ-1291 made a compass hint's WEIGHT depend on how many
        // connections the graph holds, so the reference graph carries a second connection too —
        // an `Unknown` stub, which is not a hint and scores nothing.
        let mut one = MapGraph::new();
        one.upsert_room(1, "a".into());
        one.upsert_room(2, "b".into());
        one.set_pos(1, (0, 0));
        one.set_pos(2, (1, 0));
        one.add_edge(1, Direction::E, 2);
        one.add_edge(1, Direction::Unknown, 2);
        let one_way = directional_hint_score(&one);
        assert!(one_way > 0, "the lone east edge is satisfied");
        assert_eq!(both, 2 * one_way, "reciprocal E/W pair: both directed edges satisfied");
    }

    /// SQ-1291: a satisfied compass hint outranks EVERY satisfied Up/Down hint on the map put
    /// together. North-for-up is this crate's own drawing convention (`layout_offset`); a compass
    /// word is the game's statement of where the room is, so the repair pass must never trade one
    /// away for any number of tidy-looking staircases. Zork I's `Chasm` is where it bit: the
    /// solver put it north-east of the `East-West Passage`, honouring the chasm's own `southwest`
    /// return, and `repair_directional_hints` then dragged it due south to straighten the
    /// stairwell's two legs.
    #[test]
    fn one_compass_hint_outweighs_every_updown_hint_on_the_map() {
        use crate::direction::Direction;
        use crate::graph::MapGraph;
        // A hub with ONE `SW` bearing, plus eight two-room stairwells — sixteen directed
        // Up/Down legs. Both arrangements below hold the same seventeen connections, so the
        // scores are comparable; only which hints are SATISFIED differs.
        let g = |sw_ok: bool, stairs_ok: bool| {
            let mut g = MapGraph::new();
            g.upsert_room(1, "hub".into());
            g.set_pos(1, (0, 0));
            g.upsert_room(2, "the other room".into());
            g.set_pos(2, if sw_ok { (-1, 1) } else { (1, -1) });
            g.add_edge(1, Direction::SW, 2);
            for i in 0..8i32 {
                let (below, above) = (10 + 2 * i as u16, 11 + 2 * i as u16);
                g.upsert_room(below.into(), "below".into());
                g.upsert_room(above.into(), "above".into());
                g.set_pos(below.into(), (5, 10 + i * 3));
                g.set_pos(above.into(), (5, if stairs_ok { 9 + i * 3 } else { 11 + i * 3 }));
                g.add_edge(below.into(), Direction::Up, above.into());
                g.add_edge(above.into(), Direction::Down, below.into());
            }
            g
        };
        let compass_only = directional_hint_score(&g(true, false));
        let stairs_only = directional_hint_score(&g(false, true));
        assert!(stairs_only > 0, "the sixteen stairwell legs are satisfied and do count");
        assert!(
            compass_only > stairs_only,
            "one satisfied compass bearing ({compass_only}) must outweigh all sixteen satisfied \
             stairwell legs ({stairs_only}) put together"
        );
    }

    #[test]
    fn places_rooms_by_compass_offsets() {
        let mut m = Mapper::default();
        m.observe(1, "Center", None);
        m.observe(2, "North Room", Some(Direction::N));
        relayout_auto(&mut m.graph);
        let p1 = m.graph.room(1).unwrap().pos.unwrap();
        let p2 = m.graph.room(2).unwrap().pos.unwrap();
        assert!(p2.1 < p1.1, "north room must be above center: {p2:?} vs {p1:?}");
    }

    #[test]
    fn collision_places_nearest_free_and_marks_distorted() {
        // Set up two rooms both wanting to be north of room 1.
        // We build the graph directly since add_edge deduplicates by (origin, dir),
        // meaning a naive observe sequence would overwrite the first north edge.
        // Instead: room 1 at (0,0), rooms 2 and 3 each connected N from room 1
        // via edges stored on *different* origins (2→N→... won't work either).
        // Simplest: give room 1 two distinct north edges by using upsert_room + add_edge
        // on separate origin rooms that both have pos (0,0) — but that requires two rooms
        // at the same cell, which violates the invariant.
        //
        // The right setup: use the observe sequence from the brief.
        // After the sequence, add_edge(1,N,3) overwrites (1,N,2→3).
        // Room 2 is then reachable only via 2→Unknown→1 (inverted: placed neighbour=1).
        // Room 3 is placed at (0,-1) via compass. Room 2 is placed nearby via Unknown edge.
        // The test only asserts no overlap — which still holds.
        let mut m = Mapper::default();
        m.observe(1, "C", None);
        m.observe(2, "N1", Some(Direction::N));
        m.observe(1, "C", None); // back to center
        m.observe(3, "N2", Some(Direction::N));
        relayout_auto(&mut m.graph);
        // no two rooms share a cell
        let cells: Vec<_> = m.graph.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "rooms must not overlap");
    }

    #[test]
    fn collision_direct_distorted_flag() {
        // Build graph directly so both edges exist simultaneously: two edges both pointing
        // to cells north of room 1. We do this by giving room 2 a pos=(0,0) temporarily and
        // making room 3 unplaced with a north edge from 1.
        // Actually: place room 1 at (0,0), room 2 at (0,-1) manually, then add edge 1→N→3.
        // When relayout_auto runs, room 3 wants (0,-1) which is occupied by 2 → displaced,
        // and the edge 1→N→3 is marked distorted.
        //
        // With new dynamic layout: rooms 1, 2, 3 all re-derive. Root=1 at (0,0).
        // Edge 1→N→2: room 2 placed at (0,-1).
        // Edge 1→N→3: room 3 wants (0,-1) — occupied → nearest free → displaced, distorted.
        let mut graph = MapGraph::new();
        graph.upsert_room(1, "C".into());
        graph.upsert_room(2, "N1".into());
        graph.upsert_room(3, "N2".into());
        // No manual set_pos — new layout clears them anyway.
        graph.add_edge(1, Direction::N, 2);
        graph.add_edge(1, Direction::N, 3); // duplicate key (origin=1, dir=N) → overwrites dest!
        // Note: add_edge deduplicates by (origin, dir), so this sets 1→N→3 (replacing 1→N→2).
        // Room 2 becomes reachable only via connectivity if another edge exists.
        // Let's add room 2 with a different edge so it gets placed too.
        graph.add_edge(2, Direction::S, 1); // gives room 2 connectivity to 1
        relayout_auto(&mut graph);
        // room 3 placed somewhere other than (0,-1) if 2 got there first, or vice versa.
        // The key assertion: no overlap.
        let cells: Vec<_> = graph.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "rooms must not overlap");
        // All rooms placed.
        assert!(graph.room(1).unwrap().pos.is_some());
        assert!(graph.room(2).unwrap().pos.is_some());
        assert!(graph.room(3).unwrap().pos.is_some());
    }

    #[test]
    fn rooms_never_overlap_random_walk() {
        let mut m = Mapper::default();
        let steps = [
            (1, None),
            (2, Some(Direction::N)),
            (3, Some(Direction::E)),
            (4, Some(Direction::S)),
            (5, Some(Direction::W)),
        ];
        for (id, via) in steps {
            m.observe(id, "r", via);
        }
        relayout_auto(&mut m.graph);
        let cells: Vec<_> = m.graph.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len());
    }

    /// OLD: minimal_movement_preserves_existing_pos — intentionally replaced.
    /// NEW: dynamic layout re-derives from scratch each call. Verify that the
    /// root (lowest-id room) is anchored at (0,0) regardless of any previously
    /// set pos, and that connected rooms land at their constraint-derived cells.
    #[test]
    fn dynamic_layout_re_derives_from_scratch() {
        let mut graph = MapGraph::new();
        graph.upsert_room(1, "A".into());
        graph.upsert_room(2, "B".into());
        graph.set_pos(1, (5, 5));
        graph.add_edge(1, Direction::N, 2);
        relayout_auto(&mut graph);
        assert_eq!(graph.room(1).unwrap().pos, Some((0, 0)), "lowest-id room anchors at origin");
        let p2 = graph.room(2).unwrap().pos.unwrap();
        assert!(p2.1 < 0, "room 2 must be north of the anchor: {p2:?}");
    }

    #[test]
    fn relayout_is_deterministic() {
        // Same graph → same positions on repeated calls.
        let mut graph = MapGraph::new();
        for id in 1..=4 {
            graph.upsert_room(id, "r".into());
        }
        graph.add_edge(1, Direction::N, 2);
        graph.add_edge(1, Direction::E, 3);
        graph.add_edge(2, Direction::E, 4);
        relayout_auto(&mut graph);
        let positions_first: Vec<_> = (1u16..=4).map(|id| graph.room(id.into()).unwrap().pos).collect();
        relayout_auto(&mut graph);
        let positions_second: Vec<_> = (1u16..=4).map(|id| graph.room(id.into()).unwrap().pos).collect();
        assert_eq!(positions_first, positions_second, "relayout must be deterministic");
    }

    #[test]
    fn dynamic_relayout_updates_positions() {
        let mut graph = MapGraph::new();
        graph.upsert_room(1, "A".into());
        graph.upsert_room(2, "B".into());
        graph.add_edge(1, Direction::Unknown, 2);
        relayout_auto(&mut graph);
        let pos2_before = graph.room(2).unwrap().pos.unwrap();
        graph.remove_connection(1, Direction::Unknown);
        graph.add_edge(1, Direction::N, 2);
        relayout_auto(&mut graph);
        let pos2_after = graph.room(2).unwrap().pos.unwrap();
        assert!(pos2_after.1 < graph.room(1).unwrap().pos.unwrap().1, "room 2 must be north now");
        assert_ne!(pos2_before, pos2_after, "room 2 must reposition when the constraint changes");
        let cells: Vec<_> = graph.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "rooms must not overlap");
    }

    #[test]
    fn disconnected_component_gets_placed() {
        let mut graph = MapGraph::new();
        graph.upsert_room(1, "A".into());
        graph.upsert_room(2, "B".into()); // no edge connecting to 1
        relayout_auto(&mut graph);
        assert!(graph.room(1).unwrap().pos.is_some());
        assert!(graph.room(2).unwrap().pos.is_some());
        let cells: Vec<_> = graph.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "disconnected rooms must not overlap");
    }

    #[test]
    fn contradictory_geometry_marks_distorted_not_overlap() {
        use crate::direction::Direction;
        // A(1) - N -> B(2); B(2) - N -> C(3); C(3) - N -> A(1)  (impossible loop)
        let mut g = crate::graph::MapGraph::new();
        for id in 1..=3 { g.upsert_room(id, "r".into()); }
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::N, 3);
        g.add_edge(3, Direction::N, 1); // closes an impossible northward loop
        relayout_auto(&mut g);
        // no overlap
        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: std::collections::BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len());
        // at least one edge is distorted (the loop can't be Euclidean)
        assert!(g.connections().iter().any(|c| c.distorted));
    }

    #[test]
    fn nearest_free_cell_returns_from_if_free() {
        let occupied = BTreeSet::new();
        assert_eq!(nearest_free_cell(&occupied, (3, 3)), (3, 3));
    }

    #[test]
    fn nearest_free_cell_spirals_outward() {
        let mut occupied = BTreeSet::new();
        occupied.insert((0, 0));
        // First free cell in spiral should be adjacent
        let free = nearest_free_cell(&occupied, (0, 0));
        assert_ne!(free, (0, 0));
        let dist = (free.0.abs()).max(free.1.abs());
        assert_eq!(dist, 1, "nearest free cell should be at radius 1");
    }

    #[test]
    fn combined_offset_places_northeast() {
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::W, 1);
        relayout_auto(&mut g);
        let pa = g.room(1).unwrap().pos.unwrap();
        let pb = g.room(2).unwrap().pos.unwrap();
        assert!(pb.0 > pa.0 && pb.1 < pa.1, "B must be north-east of A: {pb:?} vs {pa:?}");
        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "rooms must not overlap");
    }

    #[test]
    fn reciprocal_places_one_step_north() {
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1);
        relayout_auto(&mut g);
        let pa = g.room(1).unwrap().pos.unwrap();
        let pb = g.room(2).unwrap().pos.unwrap();
        assert!(pb.1 < pa.1, "B must be north of A: {pb:?} vs {pa:?}");
        assert_eq!(pb.0, pa.0, "no east/west constraint → B stays in A's column");
    }

    #[test]
    fn east_room_is_east() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        relayout_auto(&mut m.graph);
        let pa = m.graph.room(1).unwrap().pos.unwrap();
        let pb = m.graph.room(2).unwrap().pos.unwrap();
        assert!(pb.0 > pa.0, "east room must be to the right: {pb:?} vs {pa:?}");
    }

    #[test]
    fn orientation_pinned_north_is_up() {
        // North must map to smaller y every solve (no rotation), regardless of ids.
        let mut g = crate::graph::MapGraph::new();
        for id in 1..=4 {
            g.upsert_room(id, "r".into());
        }
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::E, 3);
        g.add_edge(3, Direction::S, 4);
        relayout_auto(&mut g);
        let p1 = g.room(1).unwrap().pos.unwrap();
        let p2 = g.room(2).unwrap().pos.unwrap();
        assert!(p2.1 < p1.1, "room 2 (north of 1) must be above it: {p2:?} vs {p1:?}");
    }

    #[test]
    fn constraint_engine_places_reciprocal_due_north() {
        // Reciprocal N/S pair → B due north of A (same column), via the constraint engine.
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1);
        relayout_auto(&mut g);
        let pa = g.room(1).unwrap().pos.unwrap();
        let pb = g.room(2).unwrap().pos.unwrap();
        assert!(pb.1 < pa.1, "B must be north of A: {pb:?} vs {pa:?}");
        assert_eq!(pb.0, pa.0, "no E/W constraint → B stays in A's column");
        // no overlap
        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len());
    }

    #[test]
    fn repair_terminates_on_impossible_mutual_south() {
        // A -S-> B and B -S-> A can't both be true. Repair must terminate, leave no
        // overlap, and leave at least one of the two edges distorted (unsatisfiable).
        use crate::direction::Direction;
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::S, 2);
        g.add_edge(2, Direction::S, 1);
        relayout_auto(&mut g);
        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "no overlap");
        assert!(g.connections().iter().any(|c| c.distorted), "one mutual-S edge stays distorted");
    }

    fn a129_house_graph() -> crate::graph::MapGraph {
        let mut g = crate::graph::MapGraph::new();
        for id in [25u16,26,27,74,75,76,77,78,79,80,81,136,143,180,193,201,203,239] {
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
            (79,W,203),(203,W,193),(193,E,203),(203,E,79),(203,Up,201),(201,Down,203),
        ] { g.add_edge(o, d, dst); }
        g
    }

    #[test]
    fn a129_perpendicular_bidirectional_hint_preserved() {
        // Perpendicular bidirectional pairs imply DIAGONAL placements that the contiguity pass
        // must not collapse:
        //   180→S→80 + 80→W→180  ⇒ 80 SOUTHEAST of 180
        //   180→N→81             ⇒ 81 north of 180
        //   79→N→81 + 81→E→79    ⇒ 81 NORTHWEST of 79
        //   79→S→80 + 80→E→79    ⇒ 80 SOUTHWEST of 79
        // VPSC places all of these correctly; eject-only contiguity must leave members put so
        // the diagonals survive.
        let mut g = a129_house_graph();
        relayout_auto(&mut g);
        let p = |id: u16| g.room(id.into()).unwrap().pos.unwrap();
        let (p79, p80, p81, p180) = (p(79), p(80), p(81), p(180));
        // 80 SOUTHEAST of 180; 81 NORTH of 180.
        assert!(p80.1 > p180.1, "80 must stay SOUTH of 180: 80={p80:?} 180={p180:?}");
        assert!(p80.0 > p180.0, "80 must stay EAST of 180: 80={p80:?} 180={p180:?}");
        assert!(p81.1 < p180.1, "81 must stay NORTH of 180: 81={p81:?} 180={p180:?}");
        // 81 NORTHWEST of 79; 80 SOUTHWEST of 79 (the west component must not collapse).
        assert!(p81.0 < p79.0, "81 must stay WEST of 79: 81={p81:?} 79={p79:?}");
        assert!(p81.1 < p79.1, "81 must stay NORTH of 79: 81={p81:?} 79={p79:?}");
        assert!(p80.0 < p79.0, "80 must stay WEST of 79: 80={p80:?} 79={p79:?}");
        assert!(p80.1 > p79.1, "80 must stay SOUTH of 79: 80={p80:?} 79={p79:?}");
        // No room overlap.
        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "no room overlap");
    }

    #[test]
    fn constraint_engine_beats_sort_distortion_on_a129() {
        // Distortion under the longest-path sort fallback (sort_layout + mark_distorted).
        let mut g_sort = a129_house_graph();
        let pos = sort::sort_layout(&g_sort);
        for (&id, &p) in &pos { g_sort.set_pos(id, p); }
        mark_distorted(&mut g_sort, &BTreeSet::new());
        let sort_distorted = g_sort.connections().iter().filter(|c| c.distorted).count();

        // Distortion under the constraint engine (the default relayout_auto).
        let mut g_cons = a129_house_graph();
        relayout_auto(&mut g_cons);
        let cons_distorted = g_cons.connections().iter().filter(|c| c.distorted).count();

        assert!(
            cons_distorted < sort_distorted,
            "constraint engine must reduce distortion vs sort: constraint={cons_distorted}, sort={sort_distorted}",
        );
        // And it must not overlap rooms.
        let cells: Vec<_> = g_cons.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "no room overlap under the constraint engine");
    }

    #[test]
    fn constraint_engine_aligns_cardinal_chains() {
        // The alignment pass straightens E/W chains whose ends are free on the row axis
        // (79→W→203), and the axis-preserving collision resolver keeps an aligned room on
        // its row even when its cell is taken — so 203→W→193 stays clean (#193 shifts ALONG
        // #203's row past #180 instead of bumping off it). Total distortion falls to 22.
        //
        // #25→E→26 is now legitimately distorted: #26 also has 26→Up→25, so 26 must sit both
        // NORTH (up) and EAST of 25. Post-fix the align stage marks 26 Y-constrained via its
        // Up edge (layout_offset, not grid_offset), so it is NOT flattened onto 25's row —
        // the solver places 26 southeast of 25 (p25=(0,0), p26=(1,1)), satisfying BOTH hints
        // and correctly rendering the E edge diagonal (distorted).
        //
        // It still cannot straighten every cardinal edge: a room pulled by CONFLICTING
        // constraints (e.g. #25 wants both #74's and #76's row, but 74 S 76 forces those
        // apart) must distort one of them — inherent to a non-Euclidean graph.
        let mut g = a129_house_graph();
        relayout_auto(&mut g);
        let e = |o, d| g.connections().iter().find(|c| c.origin == o && c.dest == d).unwrap();
        let p = |id: u16| g.room(id.into()).unwrap().pos.unwrap();
        // 26 sits southeast of 25 (both Up and E hints satisfied) → the E edge is diagonal.
        assert!(e(25, 26).distorted, "25→E→26 is diagonal: 26 is southeast of 25 (up + east)");
        // 26→Up→25 puts 25 NORTH of 26, so 26 sits SOUTH (and E) of 25 → southeast.
        assert!(p(26).1 > p(25).1 && p(26).0 > p(25).0, "26 southeast of 25: p25={:?} p26={:?}", p(25), p(26));
        assert!(!e(79, 203).distorted, "79→W→203 aligns onto one row");
        assert!(!e(203, 193).distorted, "203→W→193: #193 holds #203's row (collision shifts along it)");
        let distorted = g.connections().iter().filter(|c| c.distorted).count();
        assert!(distorted <= 22, "alignment + axis-preserving collision cut distortion to 22; got {distorted}");
    }

    #[test]
    fn relayout_is_deterministic_under_constraint_engine() {
        let mut a = a129_house_graph();
        let mut b = a129_house_graph();
        relayout_auto(&mut a);
        relayout_auto(&mut b);
        let pa: Vec<_> = a.rooms().map(|r| (r.id, r.pos)).collect();
        let pb: Vec<_> = b.rooms().map(|r| (r.id, r.pos)).collect();
        assert_eq!(pa, pb, "constraint engine must be deterministic");
    }

    #[test]
    fn reciprocal_ew_pair_shares_a_row() {
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(1, "a".into());
        g.upsert_room(2, "b".into());
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1); // reciprocal E/W → same row
        relayout_auto(&mut g);
        let p1 = g.room(1).unwrap().pos.unwrap();
        let p2 = g.room(2).unwrap().pos.unwrap();
        assert_eq!(p1.1, p2.1, "reciprocal E/W pair must share a row: {p1:?} {p2:?}");
        assert!(p2.0 > p1.0, "and 2 is east of 1");
    }

    #[test]
    fn updown_only_room_stays_southeast_of_neighbour() {
        // Bug #3: a room whose only vertical hint is Up/Down must keep its N/S offset from
        // that neighbour, not get flattened onto the neighbour's row by the align stage.
        // 22 Up→23 ⇒ 23 is NORTH of 22 (22 south of 23); 23 E→22 ⇒ 22 is EAST of 23.
        // So 22 must land SOUTHEAST of 23. Pre-fix the align stage used grid_offset (None for
        // Up/Down) to decide axis-constrainedness, so 22 was Y-free and flattened onto 23's row.
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(22, "a".into());
        g.upsert_room(23, "b".into());
        g.add_edge(22, Direction::Up, 23); // 23 north of 22
        g.add_edge(23, Direction::E, 22); // 22 east of 23
        relayout_auto(&mut g);
        let p22 = g.room(22).unwrap().pos.unwrap();
        let p23 = g.room(23).unwrap().pos.unwrap();
        assert!(p23.1 < p22.1, "23 must be strictly north of 22: p22={p22:?} p23={p23:?}");
        assert!(p22.0 > p23.0, "22 must stay east of 23: p22={p22:?} p23={p23:?}");
    }

    #[test]
    fn reciprocal_ns_pair_shares_a_column() {
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(1, "a".into());
        g.upsert_room(2, "b".into());
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1); // reciprocal N/S → same column
        relayout_auto(&mut g);
        let p1 = g.room(1).unwrap().pos.unwrap();
        let p2 = g.room(2).unwrap().pos.unwrap();
        assert_eq!(p1.0, p2.0, "reciprocal N/S pair must share a column");
        assert!(p2.1 < p1.1, "and 2 is north of 1");
    }

    #[test]
    fn three_room_ew_chain_all_share_one_row() {
        // 1↔2↔3 reciprocal E/W chain → all three on EXACTLY one row. A single ≤ (gap 0)
        // would let the row drift across three rooms; both-leg equality pins them equal.
        let mut g = crate::graph::MapGraph::new();
        for id in [1u16, 2, 3] { g.upsert_room(id.into(), "r".into()); }
        for (o, d, dst) in [
            (1, Direction::E, 2), (2, Direction::W, 1),
            (2, Direction::E, 3), (3, Direction::W, 2),
        ] { g.add_edge(o, d, dst); }
        relayout_auto(&mut g);
        let y1 = g.room(1).unwrap().pos.unwrap().1;
        let y2 = g.room(2).unwrap().pos.unwrap().1;
        let y3 = g.room(3).unwrap().pos.unwrap().1;
        assert_eq!(y1, y2, "rooms 1 and 2 share a row");
        assert_eq!(y2, y3, "rooms 2 and 3 share a row");
    }

    #[test]
    fn bidirectional_chain_no_foreign_interleave() {
        // 79↔203↔193 (E/W chain) plus a foreign room 180 with no chain edge. Members share a
        // row; no foreign room may sit between them on it (members may have gaps — eject-only
        // never moves members, only ejects interlopers).
        let mut g = crate::graph::MapGraph::new();
        for id in [79u16, 180, 193, 203] { g.upsert_room(id.into(), "r".into()); }
        for (o, d, dst) in [
            (79, Direction::W, 203), (203, Direction::E, 79),
            (203, Direction::W, 193), (193, Direction::E, 203),
        ] { g.add_edge(o, d, dst); }
        // 180 connected loosely so it shares the component but has no chain edge.
        g.add_edge(180, Direction::S, 79);
        g.add_edge(79, Direction::N, 180);
        relayout_auto(&mut g);
        let p = |id| g.room(id).unwrap().pos.unwrap();
        let (a, b, c) = (p(193), p(203), p(79));
        // All three on one row.
        assert_eq!(a.1, b.1);
        assert_eq!(b.1, c.1);
        let mut xs = [a.0, b.0, c.0];
        xs.sort_unstable();
        // 180 is NOT between the chain's extreme members on that row.
        let p180 = p(180);
        let between = p180.1 == a.1 && p180.0 > xs[0] && p180.0 < xs[2];
        assert!(!between, "foreign room must not interleave the chain: 180={p180:?}, chain xs={xs:?}");
        // no overlap
        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len());
    }

    #[test]
    fn a129_peeled_keeps_living_room_next_to_kitchen() {
        // Real-save regression. With #27's planar region ({27,136}) peeled into its own layer,
        // layer 0 holds the E/W chain 193(Living Room)→203(Kitchen)→79(Behind House). The
        // contiguity pass lays them out adjacent, but #180 (West of House) rounds onto the SAME
        // cell as the chain's WEST endpoint #193. The strict between-test (par > lo) let #180
        // survive eject, and the collision pass then bumped #193 one cell further west — wedging
        // #180 between Living Room and Kitchen. Ejecting interlopers coincident with an endpoint
        // keeps 193 directly west-adjacent to 203.
        let mut g = a129_house_graph();
        let region = crate::layer::planar_region(&g, 27);
        crate::layer::move_region(&mut g, &region, crate::layer::MoveTarget::New)
            .expect("27's region peels into a new layer");
        let mut sub = g.layer_subgraph(crate::layer::MAIN_LAYER);
        relayout_auto(&mut sub);
        let p = |id: u16| sub.room(id.into()).unwrap().pos.unwrap();
        let (p193, p203) = (p(193), p(203));
        assert_eq!(p193.1, p203.1, "193 and 203 must share a row: 193={p193:?} 203={p203:?}");
        assert_eq!(
            p203.0 - p193.0,
            1,
            "Living Room (193) must sit directly west-adjacent to Kitchen (203): 193={p193:?} 203={p203:?}",
        );
    }

    #[test]
    fn a129_chain_no_foreign_interleave() {
        // The full A129 house graph contains the E/W chain 79↔203↔193.
        // Without the eject pass, room #180 (connected but not part of the chain) gets
        // interleaved between #193 and #203 on their shared row — the original bug. This test
        // fails RED when the eject pass is a no-op and GREEN when it runs. (Members may have
        // gaps; eject-only never moves them, so we assert "no foreign room BETWEEN", not
        // "consecutive".)
        let mut g = a129_house_graph();
        relayout_auto(&mut g);
        let p = |id: u16| g.room(id.into()).unwrap().pos.unwrap();
        let (p193, p203, p79) = (p(193), p(203), p(79));
        // All three chain members share one row.
        assert_eq!(p193.1, p203.1, "193 and 203 must share a row: {p193:?} {p203:?}");
        assert_eq!(p203.1, p79.1, "203 and 79 must share a row: {p203:?} {p79:?}");
        let mut xs = [p193.0, p203.0, p79.0];
        xs.sort_unstable();
        let chain_row = p193.1;
        let xs_min = xs[0];
        let xs_max = xs[2];
        // No foreign room sits on the chain row strictly between the chain's min and max x.
        let chain_ids: std::collections::BTreeSet<RoomId> = [193, 203, 79].into();
        for r in g.rooms() {
            if chain_ids.contains(&r.id) { continue; }
            let pos = r.pos.unwrap();
            assert!(
                !(pos.1 == chain_row && pos.0 > xs_min && pos.0 < xs_max),
                "foreign room {} interleaves the chain on row {chain_row}: pos={pos:?}, chain xs={xs:?}",
                r.id,
            );
        }
        // No two rooms overlap.
        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "room positions must be unique");
    }

    /// SQ-1309: a chain member must never be evicted by a DIFFERENT chain's contiguity pass.
    ///
    /// A-B-C is a reciprocal E/W chain sharing row y=5; D-E is a wholly unrelated reciprocal N/S
    /// chain sharing column x=0, spanning y=3..=7. B — a real member of the E/W chain — happens to
    /// land at (0, 5): exactly on the N/S chain's column, inside its span. Before SQ-1309,
    /// `eject_interlopers` only protected the CURRENT chain's own members, so processing the D-E
    /// column chain saw B as a foreign interloper and evicted it off its own row, wrecking the E/W
    /// chain's alignment (Zork I's East-West Passage, evicted from the Round Room/Troll Room row
    /// by an unrelated column chain it happened to cross). Falsify by reverting `contiguify`'s
    /// `protected` set to just the current chain's own `idxs`.
    #[test]
    fn a_chain_member_is_never_evicted_by_an_unrelated_chain() {
        let mut g = crate::graph::MapGraph::new();
        for id in [1u32, 2, 3, 4, 5] {
            g.upsert_room(id, "r".into());
        }
        // A-B-C: reciprocal E/W chain.
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        g.add_edge(2, Direction::E, 3);
        g.add_edge(3, Direction::W, 2);
        // D-E: reciprocal N/S chain, wholly unrelated to A/B/C.
        g.add_edge(4, Direction::S, 5);
        g.add_edge(5, Direction::N, 4);

        let chains = detect_chains(&g);
        assert_eq!(chains.ew_members, vec![vec![1, 2, 3]], "sanity: the E/W chain is A-B-C");
        assert_eq!(chains.ns_members, vec![vec![4, 5]], "sanity: the N/S chain is D-E");

        let comp: Vec<RoomId> = vec![1, 2, 3, 4, 5];
        let index: BTreeMap<RoomId, usize> = comp.iter().enumerate().map(|(i, &id)| (id, i)).collect();
        // A=(-1,5) B=(0,5) C=(1,5): the E/W chain's row. D=(0,3) E=(0,7): the N/S chain's column —
        // B sits squarely inside it, sharing no edge with either D or E.
        let mut snapped: Vec<(i32, i32)> = vec![(-1, 5), (0, 5), (1, 5), (0, 3), (0, 7)];

        contiguify(&chains, &comp, &index, &mut snapped, &g);

        assert_eq!(snapped[index[&2]], (0, 5), "B must stay put: it is a real E/W chain member, not an interloper");
        assert_eq!(snapped[index[&1]], (-1, 5), "A must stay put (never a victim's own chain)");
        assert_eq!(snapped[index[&3]], (1, 5), "C must stay put (never a victim's own chain)");
    }

    #[test]
    fn large_graph_uses_sort_fallback_without_overlap() {
        // A chain longer than MAX_NODES forces the fallback path; it must still place
        // every room with no overlap (and not run the O(n²) solve).
        let mut g = crate::graph::MapGraph::new();
        let count = (super::MAX_NODES + 5) as u16;
        for id in 1..=count { g.upsert_room(id.into(), "r".into()); }
        for id in 1..count { g.add_edge(id.into(), Direction::E, (id + 1).into()); }
        relayout_auto(&mut g);
        let placed = g.rooms().filter(|r| r.pos.is_some()).count();
        assert_eq!(placed, count as usize, "every room placed via fallback");
        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "no overlap in the fallback layout");
    }

    /// Observer emits the 5 stage labels in order on a small graph.
    #[test]
    fn observed_emits_five_stage_labels_in_order() {
        // A small 4-room graph with reciprocal edges to exercise all stages.
        let mut g = crate::graph::MapGraph::new();
        for id in [1u16, 2, 3, 4] { g.upsert_room(id.into(), "r".into()); }
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1);
        g.add_edge(2, Direction::E, 3);
        g.add_edge(3, Direction::W, 2);
        g.add_edge(3, Direction::S, 4);

        let mut labels: Vec<String> = Vec::new();
        relayout_auto_observed(&mut g, Some(&mut |_graph, label, _desc, _stats| {
            labels.push(label.to_owned());
        }));

        assert_eq!(
            labels,
            ["seed", "stress", "align", "contiguify", "pack"],
            "observer must receive the 5 stage labels in order: got {labels:?}",
        );
    }

    /// relayout_auto_observed with an observer produces the same final positions
    /// as relayout_auto (no observer) on the same graph.
    #[test]
    fn observed_result_equals_plain_relayout() {
        // Use the A129 house graph — a moderately complex real-world topology.
        let mut plain = a129_house_graph();
        relayout_auto(&mut plain);
        let plain_positions: Vec<(RoomId, Option<(i32, i32)>)> =
            plain.rooms().map(|r| (r.id, r.pos)).collect();
        let plain_distorted: Vec<bool> =
            plain.connections().iter().map(|c| c.distorted).collect();

        let mut observed_g = a129_house_graph();
        let mut call_count = 0u32;
        relayout_auto_observed(&mut observed_g, Some(&mut |_graph, _label, _desc, _stats| {
            call_count += 1;
        }));
        let obs_positions: Vec<(RoomId, Option<(i32, i32)>)> =
            observed_g.rooms().map(|r| (r.id, r.pos)).collect();
        let obs_distorted: Vec<bool> =
            observed_g.connections().iter().map(|c| c.distorted).collect();

        assert_eq!(call_count, 5, "observer called once per stage");
        assert_eq!(
            plain_positions, obs_positions,
            "observed relayout must produce the same room positions as plain relayout",
        );
        assert_eq!(
            plain_distorted, obs_distorted,
            "observed relayout must produce the same distorted flags as plain relayout",
        );
    }

    /// Observer stats: constraints_dropped is populated correctly after the stress stage.
    #[test]
    fn observed_stats_constraints_dropped_reflects_cycle_closing_edges() {
        // A 3-room northward cycle: 1-N-2-N-3-N-1. Two of the three N constraints can be
        // satisfied; the third closes a cycle and must be dropped (constraints_dropped >= 1).
        let mut g = crate::graph::MapGraph::new();
        for id in [1u16, 2, 3] { g.upsert_room(id.into(), "r".into()); }
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::N, 3);
        g.add_edge(3, Direction::N, 1);

        let mut dropped_at_stress: Option<u32> = None;
        relayout_auto_observed(&mut g, Some(&mut |_graph, label, _desc, stats| {
            if label == "stress" {
                dropped_at_stress = Some(stats.constraints_dropped);
            }
        }));

        let dropped = dropped_at_stress.expect("stress stage must fire");
        assert!(dropped >= 1, "at least one cycle-closing constraint must be dropped; got {dropped}");
    }

    #[test]
    fn directional_hint_score_counts_updown_as_ns() {
        use crate::direction::Direction;
        use crate::graph::MapGraph;
        // B is directly north of A, reached by Up. Its N/S side is satisfied.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::Up, 2);
        // The single Up edge (dest on the north side of origin) counts as one satisfied side.
        // (directional_hint_score is side-only — it does NOT require exact column alignment.)
        assert_eq!(directional_hint_score(&g), 1, "an Up edge whose dest is north scores as satisfied");

        // Move B to the WRONG side (south of A): the Up hint (expects north) is unsatisfied.
        g.set_pos(2, (0, 3));
        assert_eq!(directional_hint_score(&g), 0, "an Up edge whose dest is south is unsatisfied");
    }

    #[test]
    fn updown_edge_is_not_reciprocal_weighted() {
        use crate::direction::Direction;
        use crate::graph::MapGraph;
        // Build A with a single Up edge to a room directly north.
        let updown_score = {
            let mut g = MapGraph::new();
            g.upsert_room(1, "A".into());
            g.upsert_room(2, "B".into());
            g.set_pos(1, (0, 0));
            g.set_pos(2, (0, -1));
            g.add_edge(1, Direction::Up, 2);
            g.add_edge(2, Direction::Down, 1); // an Up+Down pair must NOT double-count
            room_side_score(&g, 1)
        };
        // Same geometry, but a REAL reciprocal N/S pair (A--N-->B, B--S-->A), which DOES
        // earn RECIPROCAL_WEIGHT.
        let reciprocal_ns_score = {
            let mut g = MapGraph::new();
            g.upsert_room(1, "A".into());
            g.upsert_room(2, "B".into());
            g.set_pos(1, (0, 0));
            g.set_pos(2, (0, -1));
            g.add_edge(1, Direction::N, 2);
            g.add_edge(2, Direction::S, 1);
            room_side_score(&g, 1)
        };
        // The up/down pair must score STRICTLY LESS than the reciprocal N/S pair — it never
        // gets the reciprocal doubling (reciprocal detection stays keyed on grid_offset,
        // which is None for up/down).
        assert!(updown_score < reciprocal_ns_score,
            "up/down (={updown_score}) must score below a reciprocal N/S pair (={reciprocal_ns_score})");
    }

    #[test]
    fn mark_distorted_never_marks_updown() {
        use crate::direction::Direction;
        use crate::graph::MapGraph;
        use std::collections::BTreeSet;
        // An Up edge that is NOT axis-aligned would be "unsatisfied" per edge_is_satisfied,
        // but mark_distorted gates on grid_offset (None for Up) and must leave it undistorted.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (5, 5)); // wildly off-axis
        g.add_edge(1, Direction::Up, 2);
        mark_distorted(&mut g, &BTreeSet::new());
        assert!(!g.connections()[0].distorted, "up/down is never marked distorted");
    }
}
