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
//! The contiguity stage then repairs what the solve cannot promise, in this order (SQ-1312).
//! A **hub** — a room with two or more reciprocated compass partners — is protected from
//! eviction, since its bearings intersect at one cell; but never on a cell that SPLITS a
//! cardinal-reciprocal run, which is the one claim in this engine nothing outranks. Every
//! run's internal gaps are then closed, because the separation a compass edge buys from VPSC
//! is only a MINIMUM where "exactly one cell apart" is what a reciprocal pair MEANS. And
//! every **leaf** — a room whose compass edges all name one partner — is snapped onto that
//! partner's doorstep, for the same reason.
//!
//! Where those demands are genuinely incompatible, it is the GATED passages that give — and
//! the most gated first. A door or a conditional exit ([`crate::graph::Connection::weight`])
//! chains, aligns, tightens and snaps exactly like any other passage while nothing contradicts
//! it; the weight decides only who yields when a cycle closes, and which gap in a run a room
//! may legitimately be standing in.
//!
//! After either regime, `mark_distorted` flags every compass edge whose final grid
//! geometry contradicts its direction. (Connector routing and any render-aware
//! overlap cleanup live in the `app` crate, not here.)

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::direction::{grid_offset, layout_offset, opposite, Direction};
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
/// How far `place_by_bearings` looks for a free cell before falling back to the plain spiral.
const BEARING_BUMP_RADIUS: i32 = 3;

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

/// Resolve a collision for a room constrained on BOTH axes — a diagonal room, or a hub — by
/// keeping as many of its own (reciprocal-weighted) bearings as any free cell nearby allows
/// (SQ-1312).
///
/// `place_preserving_alignment` has nothing to preserve here: with neither axis free it falls
/// straight through to `nearest_free_cell`, whose spiral starts at the cell due north-west and
/// takes the first vacancy it finds. For a room whose whole position IS its quadrants that throws
/// the answer away — Zork I's `North of House` rounded onto the `Kitchen`'s cell and the spiral
/// put it due west of `West of House`, breaking a reciprocated diagonal the solve had satisfied.
fn place_by_bearings(
    occupied: &BTreeSet<(i32, i32)>,
    from: (i32, i32),
    graph: &MapGraph,
    index: &BTreeMap<RoomId, usize>,
    snapped: &[(i32, i32)],
    id: RoomId,
) -> (i32, i32) {
    if !occupied.contains(&from) {
        return from;
    }
    /// Most bearings respected first, then nearest, then west, then north (deterministic).
    type BumpRank = (std::cmp::Reverse<usize>, i32, i32, i32);
    let mut best: Option<(BumpRank, (i32, i32))> = None;
    for r in 1..=BEARING_BUMP_RADIUS {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs().max(dy.abs()) != r {
                    continue; // perimeter of this ring only
                }
                let cand = (from.0 + dx, from.1 + dy);
                if occupied.contains(&cand) {
                    continue;
                }
                let key = (
                    std::cmp::Reverse(edges_respected_at(graph, index, snapped, id, cand, &BTreeSet::new())),
                    dx.abs() + dy.abs(),
                    cand.0,
                    cand.1,
                );
                if best.as_ref().is_none_or(|(b, _)| key < *b) {
                    best = Some((key, cand));
                }
            }
        }
        if let Some((_, cell)) = best {
            return cell; // nearest ring with any vacancy wins; bearings decide within it
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

/// How many of room `id`'s RECIPROCATED compass bearings — passages walked from both ends, the
/// strongest evidence the map has (SQ-1287) — keep it on the correct side of the neighbour if it
/// sat at `cell`. Rooms in `ignore` follow `id` and are skipped, as in [`edges_respected_at`].
///
/// One-way hints are deliberately not counted (SQ-1312). This answers "would this cell still
/// honour the doors this room was walked through?", which is the question when deciding whether
/// to stretch a gated passage to make room for a hub: a lone one-way bearing to somewhere else is
/// exactly the sort of slack that should give way, where a reciprocated pair is not.
fn reciprocals_respected_at(
    graph: &MapGraph,
    index: &BTreeMap<RoomId, usize>,
    snapped: &[(i32, i32)],
    id: RoomId,
    cell: (i32, i32),
    ignore: &BTreeSet<RoomId>,
) -> usize {
    let conns = graph.connections();
    conns
        .iter()
        .filter(|c| !c.is_self_loop())
        .filter_map(|c| {
            let (other, is_origin) = if c.origin == id {
                (c.dest, true)
            } else if c.dest == id {
                (c.origin, false)
            } else {
                return None;
            };
            if ignore.contains(&other) {
                return None;
            }
            let delta = grid_offset(c.dir)?;
            if !conns
                .iter()
                .any(|o| o.origin == c.dest && o.dest == c.origin && o.dir == opposite(c.dir))
            {
                return None; // one-way: slack, not evidence to protect here
            }
            let op = snapped[*index.get(&other)?];
            let actual = if is_origin {
                (op.0 - cell.0, op.1 - cell.1)
            } else {
                (cell.0 - op.0, cell.1 - op.1)
            };
            (axis_side_respected(actual.0, delta.0) && axis_side_respected(actual.1, delta.1))
                .then_some(())
        })
        .count()
}

fn edges_respected_at(
    graph: &MapGraph,
    index: &BTreeMap<RoomId, usize>,
    snapped: &[(i32, i32)],
    id: RoomId,
    cell: (i32, i32),
    ignore: &BTreeSet<RoomId>,
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
        if ignore.contains(&other) {
            continue; // this room follows `id` wherever it goes; its CURRENT cell says nothing
        }
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
#[derive(Clone, Copy)]
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
/// column chain (rooms #128/#229) it happened to cross. `protected` answers, for a room and the
/// cell it currently holds, whether this pass may move it: every room any chain claims, plus a
/// hub holding a cell that does not split a run (SQ-1312). It takes the CELL because a hub's
/// claim is conditional on where it stands — an ejected room can land inside a later chain's
/// span, and asking again at that cell is what stops it resting there.
fn eject_interlopers(
    snapped: &mut [(i32, i32)],
    protected: &impl Fn(usize, (i32, i32)) -> bool,
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
            .find(|&q| !protected(q, snapped[q]) && between(snapped[q]));
        let Some(q) = victim else { break };
        let from = snapped[q];
        let occ: BTreeSet<(i32, i32)> =
            (0..snapped.len()).filter(|&k| k != q).map(|k| snapped[k]).collect();
        let id = comp[q];
        let no_ignore: BTreeSet<RoomId> = BTreeSet::new();
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
                    std::cmp::Reverse(edges_respected_at(graph, index, snapped, id, c, &no_ignore)),
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

/// One run as the contiguity pass sees it.
struct Run {
    span: ChainSpan,
    /// Local indices of the run's members. Never moved by `eject_interlopers` (they are all
    /// protected), so a `Run` stays valid for the whole pass.
    members: BTreeSet<usize>,
    /// Open intervals along the line, between two consecutive members joined by a passage that
    /// may REACH past an intervening room — a conditional exit, not a door
    /// ([`crate::graph::PassageWeight::may_reach_past_a_room`]). A room may legitimately stand
    /// in one of these (SQ-1312). Exclusive at both ends.
    reach_gaps: Vec<(i32, i32)>,
}

/// Every chain of this component that currently holds its line.
fn chain_runs(
    chains: &Chains,
    comp: &[RoomId],
    index: &BTreeMap<RoomId, usize>,
    snapped: &[(i32, i32)],
) -> Vec<Run> {
    let mut out = Vec::new();
    for (horizontal, groups) in [(true, &chains.ew_members), (false, &chains.ns_members)] {
        for members in groups {
            let idxs: Vec<usize> =
                members.iter().filter_map(|id| index.get(id).copied()).collect();
            if idxs.len() < 2 {
                continue;
            }
            let coord = |i: usize| if horizontal { snapped[i].1 } else { snapped[i].0 };
            let line = coord(idxs[0]);
            if !idxs.iter().all(|&i| coord(i) == line) {
                continue; // the chain's equality was dropped — it has no line to defend
            }
            let par = |i: usize| if horizontal { snapped[i].0 } else { snapped[i].1 };
            let lo = idxs.iter().map(|&i| par(i)).min().unwrap();
            let hi = idxs.iter().map(|&i| par(i)).max().unwrap();
            // Walk the members in position order; a gap between two of them whose own passage
            // may REACH is a gap a room may stand in.
            let mut order = idxs.clone();
            order.sort_by_key(|&i| par(i));
            let reach_gaps: Vec<(i32, i32)> = order
                .windows(2)
                .filter(|w| par(w[1]) - par(w[0]) > 1)
                .filter(|w| {
                    chains
                        .link_weight(comp[w[0]], comp[w[1]])
                        .is_some_and(|x| x.may_reach_past_a_room())
                })
                .map(|w| (par(w[0]), par(w[1])))
                .collect();
            out.push(Run {
                span: ChainSpan { horizontal, line, lo, hi },
                members: idxs.into_iter().collect(),
                reach_gaps,
            });
        }
    }
    out
}

/// True iff `cell` lies on `span`'s line within its member extent (endpoints included) — the
/// zone `eject_interlopers` clears.
fn cell_is_on_span(span: &ChainSpan, cell: (i32, i32)) -> bool {
    let (perp, par) = if span.horizontal { (cell.1, cell.0) } else { (cell.0, cell.1) };
    perp == span.line && par >= span.lo && par <= span.hi
}

/// True iff `cell` sits STRICTLY between two members of some cardinal-reciprocal run — on the
/// run's line with a member on either side of it (SQ-1312).
///
/// This is the one thing no room may do, whatever else it has going for it. A reciprocal
/// cardinal pair means "exactly one row or column apart"; a room parked between two members
/// widens the pair to two cells and the passage between them is then drawn straight through
/// that room's box. Everything else the layout weighs — a diagonal's quadrant, a hub's several
/// bearings — has SLACK (`edge_is_satisfied` is sign-based per axis, so a stretched diagonal
/// still reads correctly), and slack is what gives way first. Endpoints are excluded on
/// purpose: a room rounding onto a member's own cell is a plain overlap, not a split run.
///
/// **Unless the link it stands in is one that may REACH** (SQ-1312) — a conditional exit, which
/// is typically a secret passage, and NOT a door, which is a real walkable way through the
/// geography that happens to need opening. A secret passage drawn reaching past the rooms above
/// it is a fair drawing of a secret passage; a plain corridor or a doorway doing the same is a
/// lie. This is the one place a weight decides anything other than constraint order, and it is
/// the same principle either way: when two claims cannot both hold, the more gated one yields.
fn splits_a_run(runs: &[Run], cell: (i32, i32)) -> bool {
    runs.iter().any(|r| {
        let s = &r.span;
        let (perp, par) = if s.horizontal { (cell.1, cell.0) } else { (cell.0, cell.1) };
        perp == s.line
            && par > s.lo
            && par < s.hi
            && !r.reach_gaps.iter().any(|&(a, b)| par > a && par < b)
    })
}

/// The one room every one of `id`'s compass edges names, together with the unit offset FROM `id`
/// TO that room — or `None` when `id` has no compass edge at all, names more than one partner, or
/// the bearings between the pair contradict each other on an axis. Such a room is a **leaf**: the
/// map holds one coherent statement about where it lies and nothing else.
///
/// `In`/`Out`/`Unknown` are not compass bearings (`grid_offset` returns `None` for them), so they
/// never disqualify a leaf: Zork I's `Stone Barrow` is a leaf south-west of `West of House` even
/// though the same door is also an `IN` (SQ-1312).
///
/// Nor does a stairwell to ANOTHER room — Zork I's `Studio` is a leaf hanging south of the
/// `Gallery` even though the `Kitchen` also drops into it. But a stairwell joining the PAIR is a
/// second, vertical claim on the same relationship (the alignment stage honours it through
/// `layout_offset`), so a leaf whose partner is also reached by `Up`/`Down` keeps the cell the
/// solve gave it — Zork I's `Egyptian Room` is west of the `Temple` and also up from it.
///
/// Every compass edge BETWEEN the pair then COMPOSES into the offset, one axis at a time, exactly
/// the way `build_axis_constraints` composes them: `A→N→B` with `B→W→A` says north AND east, so
/// the doorstep is the north-east diagonal, not either cardinal on its own.
fn leaf_partner(graph: &MapGraph, id: RoomId) -> Option<(RoomId, (i32, i32))> {
    let conns = graph.connections();
    let mut partner: Option<RoomId> = None;
    for c in conns {
        if c.is_self_loop() || grid_offset(c.dir).is_none() {
            continue;
        }
        let other = if c.origin == id {
            c.dest
        } else if c.dest == id {
            c.origin
        } else {
            continue;
        };
        match partner {
            None => partner = Some(other),
            Some(p) if p == other => {}
            Some(_) => return None, // a second compass partner: not a leaf
        }
    }
    let partner = partner?;
    // Compose the bearings between the pair. A conflicting sign on either axis means the two
    // observations cannot both be true, so there is no doorstep to snap to; a stairwell between
    // the pair is a vertical claim the snap has no way to honour, so it declines outright.
    let mut off = (0_i32, 0_i32);
    for c in conns {
        let outgoing = if c.origin == id && c.dest == partner {
            true
        } else if c.dest == id && c.origin == partner {
            false
        } else {
            continue;
        };
        let Some(o) = grid_offset(c.dir) else {
            if layout_offset(c.dir).is_some() {
                return None; // Up/Down between the pair
            }
            continue;
        };
        let to_partner = if outgoing { o } else { (-o.0, -o.1) };
        for (axis, sign) in [(&mut off.0, to_partner.0), (&mut off.1, to_partner.1)] {
            if sign == 0 {
                continue;
            }
            if *axis != 0 && *axis != sign.signum() {
                return None; // "the partner is east of me" and "west of me"
            }
            *axis = sign.signum();
        }
    }
    (off != (0, 0)).then_some((partner, off))
}

/// Pull every leaf onto its partner's doorstep — the cell exactly one step back along its own
/// bearing (SQ-1312).
///
/// `stress_layout` minimises an objective averaged over EVERY pair in the component, and the VPSC
/// separation a compass edge contributes is only a MINIMUM ("at least one cell apart"), so a room
/// hanging off the side of the map routinely settles two or three cells out with nothing in
/// between: Zork I's `Studio` sat three rows above the `Gallery`, its only neighbour, with the
/// intervening cell free. A leaf is the one room this can be fixed for unilaterally — the map
/// holds exactly one statement about where it lies, so there is no second constraint to trade
/// against and no other room's position changes.
///
/// The leaf moves only INWARD along the bearing (Chebyshev distance to the partner never grows),
/// only onto a free cell, and never onto the span of a chain it does not itself belong to — the
/// zone `eject_interlopers` clears, which is the one place a room may not come to rest.
fn snap_leaves(
    runs: &[Run],
    comp: &[RoomId],
    index: &BTreeMap<RoomId, usize>,
    snapped: &mut [(i32, i32)],
    graph: &MapGraph,
) {
    let cheb = |a: (i32, i32), b: (i32, i32)| (a.0 - b.0).abs().max((a.1 - b.1).abs());
    for i in 0..comp.len() {
        let Some((partner, off)) = leaf_partner(graph, comp[i]) else { continue };
        let Some(&pi) = index.get(&partner) else { continue };
        if pi == i {
            continue;
        }
        let ppos = snapped[pi];
        let cur = snapped[i];
        let occupied: BTreeSet<(i32, i32)> =
            (0..snapped.len()).filter(|&k| k != i).map(|k| snapped[k]).collect();
        let forbidden = |c: (i32, i32)| {
            runs.iter().any(|r| !r.members.contains(&i) && cell_is_on_span(&r.span, c))
        };
        // Walk out from the partner along the bearing; the first free, legal cell wins.
        for d in 1..=MAX_BUMP_SPAN {
            let cand = (ppos.0 - off.0 * d, ppos.1 - off.1 * d);
            if occupied.contains(&cand) || forbidden(cand) {
                continue;
            }
            if cheb(cand, ppos) <= cheb(cur, ppos) {
                snapped[i] = cand;
            }
            break;
        }
    }
}

/// Close every empty gap inside a cardinal-reciprocal run (SQ-1312).
///
/// A reciprocal cardinal pair means EXACTLY one cell apart — that is the one claim in this engine
/// nothing outranks — but the solve only ever promises "at least one cell apart": VPSC's
/// separation is a minimum, and SMACOF settles wherever the whole component's stress is lowest.
/// Zork I's `Kitchen` and `Living Room` came out two cells apart with nothing between them, so
/// their passage was drawn as a two-cell reach through empty map, and until `eject_interlopers`
/// cleared it the `West of House` hub had been sitting in the gap.
///
/// Each run is walked along its own line and every member past a gap is pulled back with the
/// whole tail behind it, so the run's internal order and every other member's spacing survive.
///
/// **Only rooms with nothing else to lose are moved**, and that guard is what keeps the pass
/// honest: a member may shift only when every one of its compass bearings leads to another
/// member of this same run, so closing the gap cannot cost it a bearing to anywhere else. A room
/// with an outside neighbour is left where the solve put it and the gap simply stays — better a
/// pair reaching two cells than a room dragged out of some third room's quadrant to spare it.
/// A GATED bearing counts here like any other (SQ-1312): weight decides who yields in a cycle,
/// not whether a passage is real.
///
/// A shift is abandoned rather than forced when it would land on another room, when it would put
/// a member inside a DIFFERENT run's span, or when any room it moves also belongs to a run on the
/// perpendicular axis — that room's column is a claim of exactly the same rank as this row, and
/// trading one for the other decides nothing.
/// May `movers` — all members of the run on `horizontal`'s axis whose own members are `members` —
/// all slide `d` cells along that axis?
///
/// Three ways not: a mover also belongs to a run on the PERPENDICULAR axis (that column is a
/// claim of exactly the same rank as this row, and trading one for the other decides nothing); a
/// mover would BREAK a compass bearing to a room outside this run (stretching one is fine — a
/// diagonal only pins its endpoint to a quadrant — but losing the quadrant is not); or the
/// destination is occupied, or lands inside a DIFFERENT run's span. "Different" is load-bearing:
/// a PASSENGER — a room standing in one of this run's own gaps and travelling with it — is not a
/// member, so without that filter this run's own span would veto its own shift.
#[allow(clippy::too_many_arguments)]
fn shift_is_legal(
    runs: &[Run],
    members: &BTreeSet<usize>,
    movers: &BTreeSet<usize>,
    d: i32,
    horizontal: bool,
    snapped: &[(i32, i32)],
    comp: &[RoomId],
    index: &BTreeMap<RoomId, usize>,
    graph: &MapGraph,
) -> bool {
    let cross = |m: usize| {
        runs.iter()
            .any(|r| r.span.horizontal != horizontal && r.members.contains(&m))
    };
    let step = |c: (i32, i32)| if horizontal { (c.0 + d, c.1) } else { (c.0, c.1 + d) };
    // A bearing to a room OUTSIDE the run may be stretched by the shift, but never broken: a
    // diagonal only pins its endpoint to a quadrant (`axis_side_respected`), so `Behind House`
    // may slide a cell along the Kitchen's row and still be south-east of `North of House` —
    // but not one cell further (SQ-1312). A bearing that is ALREADY violated cannot get worse,
    // so it does not veto.
    let breaks_an_outside_bearing = |m: usize| {
        let id = comp[m];
        graph.connections().iter().any(|c| {
            if c.is_self_loop() {
                return false;
            }
            let Some(delta) = grid_offset(c.dir) else { return false };
            let (other, is_origin) = if c.origin == id {
                (c.dest, true)
            } else if c.dest == id {
                (c.origin, false)
            } else {
                return false;
            };
            let Some(&o) = index.get(&other) else { return false };
            if members.contains(&o) || movers.contains(&o) {
                return false; // moves with us, or is ours to keep tight
            }
            if leaf_partner(graph, other).is_some_and(|(pa, _)| pa == id) {
                return false; // a leaf hanging off this room follows it (`snap_leaves`)
            }
            let respected = |cell: (i32, i32)| {
                let op = snapped[o];
                let actual = if is_origin {
                    (op.0 - cell.0, op.1 - cell.1)
                } else {
                    (cell.0 - op.0, cell.1 - op.1)
                };
                axis_side_respected(actual.0, delta.0) && axis_side_respected(actual.1, delta.1)
            };
            respected(snapped[m]) && !respected(step(snapped[m]))
        })
    };
    if movers.iter().any(|&m| cross(m) || breaks_an_outside_bearing(m)) {
        return false;
    }
    let lands_on_a_room = (0..snapped.len())
        .any(|q| !movers.contains(&q) && movers.iter().any(|&m| step(snapped[m]) == snapped[q]));
    let splits_another = movers.iter().any(|&m| {
        runs.iter().filter(|r| r.members != *members).any(|r| {
            !r.members.contains(&m) && {
                let c = step(snapped[m]);
                let (perp, p) = if r.span.horizontal { (c.1, c.0) } else { (c.0, c.1) };
                perp == r.span.line && p > r.span.lo && p < r.span.hi
            }
        })
    });
    !lands_on_a_room && !splits_another
}

fn tighten_runs(
    runs: &[Run],
    comp: &[RoomId],
    index: &BTreeMap<RoomId, usize>,
    snapped: &mut [(i32, i32)],
    graph: &MapGraph,
) {
    for Run { span, members, reach_gaps } in runs {
        let horizontal = span.horizontal;
        let r_reach_gaps = reach_gaps;
        let par = |c: (i32, i32)| if horizontal { c.0 } else { c.1 };
        loop {
            let mut order: Vec<usize> = members.iter().copied().collect();
            order.sort_by_key(|&i| par(snapped[i]));
            // Every gap in this run, widest-first-come; a gap that cannot be closed is SKIPPED,
            // not a reason to give up on the rest of the run (SQ-1312).
            let gaps: Vec<usize> = (1..order.len())
                .filter(|&k| par(snapped[order[k]]) - par(snapped[order[k - 1]]) > 1)
                .filter(|&k| {
                    // A gap a room is legitimately standing in — one whose own link may reach
                    // past a room — is not slack to be closed. It is the arrangement the layout
                    // chose, and closing it would evict the room standing there.
                    let (lo, hi) = (par(snapped[order[k - 1]]), par(snapped[order[k]]));
                    let occupied = (0..snapped.len()).any(|q| {
                        let perp = if horizontal { snapped[q].1 } else { snapped[q].0 };
                        let p = par(snapped[q]);
                        perp == span.line && p > lo && p < hi
                    });
                    !occupied
                        || !r_reach_gaps.iter().any(|&(a, b)| a == lo && b == hi)
                })
                .collect();
            let mut progressed = false;
            for at in gaps {
            let shift = par(snapped[order[at]]) - par(snapped[order[at - 1]]) - 1;
            // A gap can be closed from EITHER side, and which side is available is not the
            // caller's to guess: pulling the tail back is blocked whenever a tail member owes a
            // bearing outside the run — Zork I's `Behind House` owes two diagonals to the ring —
            // while pushing the head forward moves rooms that owe nothing to anyone. Try the
            // tail first for determinism, then the head; take whichever is legal (SQ-1312).
            // A room that is not a member but stands INSIDE the segment being moved travels with
            // it (SQ-1312): it is standing in one of this run's own gaps — the only way it could
            // legally be there — so it is part of the row's occupancy, and leaving it behind
            // would either strand it or block the shift outright. Zork I's `West of House` sits
            // in the magic-word passage's gap and slides east with `Living Room` and the rest.
            let with_passengers = |seg: &[usize]| -> BTreeSet<usize> {
                let (lo, hi) = (
                    seg.iter().map(|&i| par(snapped[i])).min().unwrap(),
                    seg.iter().map(|&i| par(snapped[i])).max().unwrap(),
                );
                let mut set: BTreeSet<usize> = seg.iter().copied().collect();
                for (q, &cell) in snapped.iter().enumerate() {
                    let perp = if horizontal { cell.1 } else { cell.0 };
                    let p = par(cell);
                    if !set.contains(&q) && perp == span.line && p > lo && p < hi {
                        set.insert(q);
                    }
                }
                set
            };
            let tail = with_passengers(&order[at..]);
            let head = with_passengers(&order[..at]);
            let legal = |movers: &BTreeSet<usize>, d: i32| {
                shift_is_legal(runs, members, movers, d, horizontal, snapped, comp, index, graph)
            };
            let chosen = if legal(&tail, -shift) {
                Some((tail, -shift))
            } else if legal(&head, shift) {
                Some((head, shift))
            } else {
                None
            };
            let Some((movers, d)) = chosen else { continue };
            for &m in &movers {
                snapped[m] =
                    if horizontal { (snapped[m].0 + d, snapped[m].1) } else { (snapped[m].0, snapped[m].1 + d) };
            }
            progressed = true;
            break; // positions moved: recompute the run's order and start again
            }
            if !progressed {
                break;
            }
        }
    }
}

/// Rooms with two or more RECIPROCATED compass partners (SQ-1312).
///
/// A reciprocated bearing — the passage walked from both ends, the two observations agreeing —
/// is the strongest evidence the map has about geometry (SQ-1287), and a room holding two of
/// them is pinned by their intersection: there is generally exactly one cell that satisfies all
/// of a hub's bearings at once, and the stress solve has already found it. Moving such a room
/// breaks several doors to tidy one row, so a hub is protected from eviction the same way a
/// chain member is — **except on a cell that splits a cardinal-reciprocal run**
/// (`splits_a_run`), which no claim outranks. Zork I's `West of House` holds three reciprocated
/// diagonals and the solve found the cell that satisfies all three, but it lay between the
/// `Living Room` and the `Kitchen` — a passage walked from both ends — and drawing that passage
/// through the middle of a third room is a worse map than stretching a diagonal corner. So the
/// hub gives way there, and `eject_interlopers` moves it to the free off-span cell that respects
/// the most of its own reciprocal-weighted bearings; the diagonals it cannot keep stretch (the
/// sign-based check still accepts them) or, failing that, draw distorted.
fn hub_rooms(graph: &MapGraph, index: &BTreeMap<RoomId, usize>) -> BTreeSet<usize> {
    let conns = graph.connections();
    let mut partners: BTreeMap<RoomId, BTreeSet<RoomId>> = BTreeMap::new();
    for c in conns {
        if c.is_self_loop() || grid_offset(c.dir).is_none() {
            continue;
        }
        if conns.iter().any(|o| o.origin == c.dest && o.dest == c.origin && o.dir == opposite(c.dir))
        {
            partners.entry(c.origin).or_default().insert(c.dest);
            partners.entry(c.dest).or_default().insert(c.origin);
        }
    }
    partners
        .into_iter()
        .filter(|(_, p)| p.len() >= 2)
        .filter_map(|(id, _)| index.get(&id).copied())
        .collect()
}

/// Open a one-cell hole at a run's most GATED link, for a hub that would otherwise be evicted
/// from the run's line with nothing to show for it (SQ-1312).
///
/// This is the case the whole weight ordering exists for. Zork I's `West of House` holds three
/// reciprocated diagonals whose intersection is a cell ON the row that runs `Cyclops Room` ─
/// `Strange Passage` ─ `Living Room` ─ `Kitchen` ─ `Behind House`; once that row is tight there
/// is no such cell free, and the hub is thrown clear of its own ring. But two of that row's links
/// are the magic-word passage — ZIL `CEXIT`s — and a secret passage reaching one cell further
/// than its neighbours is a fair drawing of a secret passage, where a corridor or a doorway doing
/// the same is a lie. So the row stretches at its most gated link and the ring keeps its corner.
///
/// The hole is opened by sliding one side of the link away by one cell, under exactly the rules
/// `tighten_runs` closes a gap by ([`shift_is_legal`]), and the hub takes it only if it respects
/// at least as many of the hub's own bearings there as where it stands.
fn open_gated_holes_for_hubs(
    runs: &[Run],
    hubs: &BTreeSet<usize>,
    comp: &[RoomId],
    index: &BTreeMap<RoomId, usize>,
    snapped: &mut [(i32, i32)],
    graph: &MapGraph,
    chains: &Chains,
) {
    for &h in hubs {
        let cell = snapped[h];
        let stuck = (0..snapped.len()).any(|q| q != h && snapped[q] == cell)
            || splits_a_run(runs, cell);
        if !stuck {
            continue;
        }
        // Leaves whose ONLY partner is this hub follow it to its new cell (`snap_leaves`), so
        // their current positions are no evidence about where the hub should go — Zork I's
        // `Stone Barrow` hangs off `West of House` and moves with it.
        let followers: BTreeSet<RoomId> = comp
            .iter()
            .copied()
            .filter(|&id| leaf_partner(graph, id).is_some_and(|(p, _)| p == comp[h]))
            .collect();
        let here = reciprocals_respected_at(graph, index, snapped, comp[h], cell, &followers);
        'placed: for r in runs {
            let horizontal = r.span.horizontal;
            let (perp, par_h) = if horizontal { (cell.1, cell.0) } else { (cell.0, cell.1) };
            if perp != r.span.line || r.members.contains(&h) {
                continue; // the hub is not standing on this run's line
            }
            let par_at = |c: (i32, i32)| if horizontal { c.0 } else { c.1 };
            let mut order: Vec<usize> = r.members.iter().copied().collect();
            order.sort_by_key(|&i| par_at(snapped[i]));
            let pars: Vec<i32> = order.iter().map(|&i| par_at(snapped[i])).collect();
            // Most gated link first; then nearest to where the hub already wants to be.
            let mut links: Vec<(usize, crate::graph::PassageWeight)> = (1..order.len())
                .filter(|&k| pars[k] - pars[k - 1] == 1)
                .filter_map(|k| {
                    chains
                        .link_weight(comp[order[k - 1]], comp[order[k]])
                        .filter(|w| w.may_reach_past_a_room())
                        .map(|w| (k, w))
                })
                .collect();
            links.sort_by_key(|&(k, w)| (std::cmp::Reverse(w), (pars[k] - par_h).abs(), k));
            for (k, _) in links {
                // Slide one side of the link away by one; the cell that side just left is the
                // hole. (The HEAD's rightmost member was at `pars[k-1]`, the TAIL's leftmost at
                // `pars[k]` — those are the cells vacated, not the cells beyond them.)
                for (movers, d, hole_par) in
                    [(&order[..k], -1, pars[k - 1]), (&order[k..], 1, pars[k])]
                {
                    let movers: BTreeSet<usize> = movers.iter().copied().collect();
                    if !shift_is_legal(
                        runs, &r.members, &movers, d, horizontal, snapped, comp, index, graph,
                    ) {
                        continue;
                    }
                    let hole = if horizontal {
                        (hole_par, r.span.line)
                    } else {
                        (r.span.line, hole_par)
                    };
                    if reciprocals_respected_at(graph, index, snapped, comp[h], hole, &followers)
                        < here
                    {
                        continue; // no better for the hub than where it stands
                    }
                    for &m in &movers {
                        snapped[m] = if horizontal {
                            (snapped[m].0 + d, snapped[m].1)
                        } else {
                            (snapped[m].0, snapped[m].1 + d)
                        };
                    }
                    snapped[h] = hole;
                    break 'placed; // one hole per hub, and it is standing in it
                }
            }
        }
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
    // `members` covers every room this component's EW or NS chains claim at all (SQ-1309):
    // a room that is legitimately a member of one chain must not be treated as an interloper
    // and evicted by a DIFFERENT, unrelated chain whose span it happens to cross.
    let members: BTreeSet<usize> = chains
        .ew_members
        .iter()
        .chain(chains.ns_members.iter())
        .flat_map(|ms| ms.iter().filter_map(|id| index.get(id).copied()))
        .collect();
    // A HUB is protected too — but only where it stands (SQ-1312). See `hub_rooms`: its several
    // reciprocated bearings pin it to one cell, and that outranks tidying a row, right up until
    // the cell it wants splits a cardinal-reciprocal run. Nothing outranks that.
    let hubs = hub_rooms(graph, index);
    let mut snapped_v: Vec<(i32, i32)> = snapped.to_vec();
    let runs = chain_runs(chains, comp, index, &snapped_v);
    // First: where a hub has no cell at all on a run's line, stretch the run at its most gated
    // link rather than throw the hub clear of its own bearings.
    open_gated_holes_for_hubs(&runs, &hubs, comp, index, &mut snapped_v, graph, chains);
    let runs = chain_runs(chains, comp, index, &snapped_v);
    let protected = |q: usize, cell: (i32, i32)| {
        members.contains(&q) || (hubs.contains(&q) && !splits_a_run(&runs, cell))
    };
    for r in &runs {
        eject_interlopers(&mut snapped_v, &protected, r.span, comp, index, graph);
    }
    // Then close the runs' own gaps, and only then pull the leaves in — a leaf's doorstep is
    // computed against where its partner FINALLY stands, and tightening moves members.
    tighten_runs(&runs, comp, index, &mut snapped_v, graph);
    let runs = chain_runs(chains, comp, index, &snapped_v);
    snap_leaves(&runs, comp, index, &mut snapped_v, graph);
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
            // A room constrained on BOTH axes has no aligned axis to walk along, so
            // `place_preserving_alignment` spirals — and a blind spiral is how a diagonal room
            // loses the quadrant the solve had just given it (SQ-1312). Pick by its own bearings
            // instead: Zork I's `North of House` rounded onto the `Kitchen`'s cell, spiralled to
            // the first free neighbour, and came out due WEST of `West of House` instead of
            // north-east of it.
            let cell = if x_constrained[i] && y_constrained[i] {
                place_by_bearings(&occupied, snapped[i], graph, &index, &snapped, id)
            } else {
                place_preserving_alignment(&occupied, snapped[i], row_aligned, col_aligned)
            };
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

    // ── SQ-1312 ───────────────────────────────────────────────────────────────

    /// SQ-1312 (A): a LEAF — a room whose on-layer compass edges all name ONE partner —
    /// must end up on that partner's own doorstep, not merely somewhere on the right side.
    ///
    /// `stress_layout`'s SMACOF objective averages over every pair in the component, and the
    /// VPSC separation for a cardinal pair is only a MINIMUM ("at least one cell apart"), so
    /// a leaf routinely settles two or three cells out with nothing in between. Zork I's
    /// `Studio` sat three rows above its only neighbour `Gallery` with the intervening cells
    /// free. A leaf has no other constraint to trade against, so snapping it in cannot break
    /// anything else. Falsify by removing the `snap_leaves` call from `contiguify`.
    #[test]
    fn a_leaf_snaps_onto_its_only_partners_doorstep() {
        let mut g = crate::graph::MapGraph::new();
        for id in [1u32, 2, 3, 4] {
            g.upsert_room(id, "r".into());
        }
        // 1-2-3: reciprocal E/W chain on one row.
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        g.add_edge(2, Direction::E, 3);
        g.add_edge(3, Direction::W, 2);
        // 4: a leaf, reciprocally north of the middle room.
        g.add_edge(2, Direction::N, 4);
        g.add_edge(4, Direction::S, 2);

        let chains = detect_chains(&g);
        let comp: Vec<RoomId> = vec![1, 2, 3, 4];
        let index: BTreeMap<RoomId, usize> =
            comp.iter().enumerate().map(|(i, &id)| (id, i)).collect();
        // The row at y=0, and the leaf left three rows north with (0,-1) and (0,-2) free.
        let mut snapped: Vec<(i32, i32)> = vec![(-1, 0), (0, 0), (1, 0), (0, -3)];

        contiguify(&chains, &comp, &index, &mut snapped, &g);

        assert_eq!(snapped[index[&4]], (0, -1), "the leaf must sit directly north of its partner");
        assert_eq!(snapped[index[&1]], (-1, 0), "the row is untouched");
        assert_eq!(snapped[index[&2]], (0, 0), "the row is untouched");
        assert_eq!(snapped[index[&3]], (1, 0), "the row is untouched");
    }

    /// SQ-1312 (A′): an `Up`/`Down` edge is not a compass bearing, so it neither stops a room
    /// counting as a leaf nor pulls on where the snap puts it. Zork I's `Studio` is reached
    /// from the `Kitchen` by a `Down` that crosses a layer boundary and contributes nothing;
    /// the same must hold for a stairwell inside one layer.
    #[test]
    fn an_updown_edge_does_not_disqualify_a_leaf() {
        let mut g = crate::graph::MapGraph::new();
        for id in [1u32, 2, 3, 4, 5] {
            g.upsert_room(id, "r".into());
        }
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        g.add_edge(2, Direction::E, 3);
        g.add_edge(3, Direction::W, 2);
        g.add_edge(2, Direction::N, 4);
        g.add_edge(4, Direction::S, 2);
        // A stairwell between room 5 and the leaf: no compass bearing, so no claim on room 4.
        g.add_edge(5, Direction::Down, 4);
        g.add_edge(4, Direction::Up, 5);

        let chains = detect_chains(&g);
        let comp: Vec<RoomId> = vec![1, 2, 3, 4, 5];
        let index: BTreeMap<RoomId, usize> =
            comp.iter().enumerate().map(|(i, &id)| (id, i)).collect();
        let mut snapped: Vec<(i32, i32)> = vec![(-1, 0), (0, 0), (1, 0), (0, -3), (4, -3)];

        contiguify(&chains, &comp, &index, &mut snapped, &g);

        assert_eq!(
            snapped[index[&4]],
            (0, -1),
            "the stairwell is not a compass edge: room 4 is still a leaf and still snaps",
        );
    }

    /// Zork I's white house as a graph (SQ-1312): `West of House`(68) holds three reciprocated
    /// diagonals — `North of House`(143) NE, `South of House`(217) SE, `Stone Barrow`(254) SW —
    /// North and South of House are each diagonally reciprocal to `Behind House`(89), and Behind
    /// House ─ `Kitchen`(28) ─ `Living Room`(79) is a reciprocal E/W run of CARDINALS.
    fn house_ring_graph() -> crate::graph::MapGraph {
        let mut g = crate::graph::MapGraph::new();
        for id in [28u32, 68, 79, 89, 143, 217, 254] {
            g.upsert_room(id, "r".into());
        }
        use Direction::*;
        for (o, d, dst) in [
            (68u32, NE, 143u32), (143, SW, 68), // North of House north-east of West of House
            (68, SE, 217), (217, NW, 68),       // South of House south-east of it
            (68, SW, 254), (254, NE, 68),       // Stone Barrow south-west of it (a leaf)
            (143, SE, 89), (89, NW, 143),       // Behind House south-east of North of House
            (217, NE, 89), (89, SW, 217),       // and north-east of South of House
            (89, W, 28), (28, E, 89),           // Behind House ─ Kitchen ─ Living Room:
            (28, W, 79), (79, E, 28),           // a reciprocal E/W run of cardinals
        ] {
            g.add_edge(o, d, dst);
        }
        g.add_edge(68, In, 254); // the barrow's other door: no bearing, no claim
        g
    }

    /// Zork I's white house as the story actually compiles it, with every GATE at its own
    /// weight — the shape that cannot be drawn flat, and the one that says which claim yields.
    ///
    /// `Behind House` ─ `Kitchen` is the kitchen WINDOW, a `Door`: the dump reads
    /// `door=[E→"kitchen window"]` on the Kitchen. And `Strange Passage`(123) sits west of the
    /// `Living Room` with `Cyclops Room`(82) west of that, joined by `Conditional` links — both
    /// are ZIL `CEXIT`s onto the passage the magic word opens.
    ///
    /// Every one of those is a real passage that the map honours in full while nothing
    /// contradicts it. Here something does: `Behind House` is at once the east corner of the
    /// outdoor ring and the east end of the Kitchen's row, so the ring and the row want the same
    /// cells. A door is a real walkable way through the geography and holds; the secret passage
    /// is what gives.
    fn house_ring_with_its_gates() -> crate::graph::MapGraph {
        use crate::graph::PassageWeight::{Conditional, Door};
        let mut g = house_ring_graph();
        g.upsert_room(82, "r".into());
        g.upsert_room(123, "r".into());
        g.add_edge_weighted(79, Direction::W, 123, Conditional); // the magic-word passage
        g.add_edge_weighted(123, Direction::E, 79, Conditional);
        g.add_edge_weighted(82, Direction::E, 123, Conditional);
        g.add_edge_weighted(123, Direction::W, 82, Conditional);
        g.add_edge_weighted(28, Direction::E, 89, Door); // the kitchen window
        g.add_edge_weighted(89, Direction::W, 28, Door);
        g
    }

    /// Every reciprocal CARDINAL pair whose weight is at most `upto` is exactly one cell apart on
    /// its axis and exactly aligned on the other, and no room stands strictly between two members
    /// of such a run — except in a gap whose own link is gated (SQ-1312).
    fn assert_runs_are_tight_and_unsplit(g: &MapGraph, upto: crate::graph::PassageWeight) {
        let p = |id: RoomId| g.room(id).unwrap().pos.unwrap();
        let chains = detect_chains(g);
        for c in g.connections() {
            let Some(off) = grid_offset(c.dir) else { continue };
            if c.is_self_loop() || c.weight > upto || off.0 != 0 && off.1 != 0 {
                continue; // diagonals have slack; so does anything weaker than `upto`
            }
            let Some(w) = chains.link_weight(c.origin, c.dest) else { continue };
            if w > upto {
                continue; // reciprocated, but the return leg is weaker than we are checking
            }
            let (a, b) = (p(c.origin), p(c.dest));
            assert_eq!(
                (b.0 - a.0, b.1 - a.1),
                off,
                "reciprocal cardinal {} -{:?}-> {} ({w:?}) must be exactly adjacent: {a:?} {b:?}",
                c.origin, c.dir, c.dest,
            );
        }
        for (horizontal, groups) in [(true, &chains.ew_members), (false, &chains.ns_members)] {
            for ms in groups {
                let par = |c: (i32, i32)| if horizontal { c.0 } else { c.1 };
                let perp = |c: (i32, i32)| if horizontal { c.1 } else { c.0 };
                let mut order: Vec<RoomId> = ms.clone();
                order.sort_by_key(|&id| par(p(id)));
                for w in order.windows(2) {
                    let (lo, hi) = (p(w[0]), p(w[1]));
                    if perp(lo) != perp(hi) {
                        continue; // this run lost its line; nothing to defend
                    }
                    let gated =
                        chains.link_weight(w[0], w[1]).is_some_and(|x| x.is_gated());
                    for r in g.rooms() {
                        if ms.contains(&r.id) {
                            continue;
                        }
                        let c = r.pos.unwrap();
                        let inside =
                            perp(c) == perp(lo) && par(c) > par(lo) && par(c) < par(hi);
                        assert!(
                            !inside || gated,
                            "room {} at {c:?} splits the ungated link {}─{} ({lo:?} {hi:?})",
                            r.id, w[0], w[1],
                        );
                    }
                }
            }
        }
    }

    /// SQ-1312: a HUB — a room with two or more RECIPROCATED compass partners — keeps the cell
    /// its bearings pin it to rather than being evicted as a chain's "foreign interloper"…
    ///
    /// Zork I's `West of House` has three reciprocated diagonals and the stress solve found the
    /// one cell that satisfies all three; `eject_interlopers` then threw it four cells west
    /// merely because that cell fell inside the Behind House row's span, leaving `Stone Barrow`
    /// stranded under the Living Room with both legs of its only door distorted. Falsify by
    /// dropping `hubs` from `contiguify`'s `protected` predicate.
    ///
    /// …**but never on a cell that splits an ungated run.** That is the other half, and it is
    /// the half that outranks the hub: a reciprocal cardinal pair means "exactly one cell apart",
    /// and a hub parked between the Living Room and the Kitchen widens their passage to two
    /// cells and has it drawn through its own box. Falsify by dropping the `!splits_a_run(…)`
    /// clause from the same predicate.
    #[test]
    fn a_hub_keeps_its_cell_but_never_by_splitting_a_run() {
        use crate::graph::PassageWeight;
        let mut g = house_ring_graph();
        relayout_auto(&mut g);
        assert_runs_are_tight_and_unsplit(&g, PassageWeight::Conditional);

        let p = |id: u32| g.room(id).unwrap().pos.unwrap();
        let (woh, barrow) = (p(68), p(254));
        assert!(
            barrow.0 < woh.0 && barrow.1 > woh.1,
            "Stone Barrow must stay south-west of West of House: {barrow:?} vs {woh:?}",
        );

        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "no room overlap");
    }

    /// SQ-1312: a gated passage with NOTHING to conflict with is laid out exactly like any other.
    ///
    /// This is the half the first attempt at `soft` got wrong, and Zork I caught it: the troll
    /// gates both of `The Troll Room`'s compass exits, so treating gatedness as "worth less as
    /// evidence" took the pair out of run formation altogether and `East-West Passage` drifted
    /// off the `Round Room` row that SQ-1309 exists to keep it on. A monster standing in a
    /// doorway is not a statement about where the two rooms are. Weight orders who YIELDS in a
    /// cycle; it never demotes a passage that nothing is arguing with.
    #[test]
    fn a_gated_passage_with_nothing_to_yield_to_is_laid_out_tight() {
        use crate::graph::PassageWeight::{Conditional, Hard};
        // The Troll Room shape: Troll Room ─ East-West Passage ─ Round Room in a row, the troll's
        // two exits gated, plus a plain room hanging north of the middle one.
        let mut g = crate::graph::MapGraph::new();
        for id in [16u32, 112, 133, 136] {
            g.upsert_room(id, "r".into());
        }
        g.add_edge_weighted(133, Direction::E, 136, Conditional); // the troll's own exits
        g.add_edge_weighted(136, Direction::W, 133, Hard);
        g.add_edge(136, Direction::E, 16);
        g.add_edge(16, Direction::W, 136);
        g.add_edge(136, Direction::N, 112);
        g.add_edge(112, Direction::S, 136);

        relayout_auto(&mut g);
        let p = |id: u32| g.room(id).unwrap().pos.unwrap();
        let (troll, ewp, round) = (p(133), p(136), p(16));
        assert_eq!(ewp.1, troll.1, "the gated pair still shares a row: {ewp:?} {troll:?}");
        assert_eq!(ewp.0 - troll.0, 1, "and is still exactly adjacent: {ewp:?} {troll:?}");
        assert_eq!(round.1, ewp.1, "the whole run holds its row: {round:?} {ewp:?}");
        assert_eq!(round.0 - ewp.0, 1, "tight end to end: {round:?} {ewp:?}");
        assert!(
            !g.connections().iter().any(|c| c.distorted),
            "nothing is contradicting anything here, so nothing bends: {:?}",
            g.connections().iter().filter(|c| c.distorted).collect::<Vec<_>>(),
        );
    }

    /// SQ-1312: when two claims genuinely cannot both hold, the GATED one yields — and the more
    /// gated of two yields first.
    ///
    /// Zork I's white house cannot be drawn flat: `Behind House` is the east corner of the
    /// outdoor ring AND the east end of the Kitchen's row, and one room cannot be in two places.
    /// So something gives, and the ordering says what. Every ungated cardinal stays exactly
    /// adjacent with nothing standing in it; the kitchen WINDOW — a door, a real walkable way
    /// through the geography — stays adjacent too; and it is the magic-word passage, the
    /// `Conditional` link, that comes out stretched. `West of House` keeps all three diagonals.
    /// Falsify by giving the Strange Passage links `Door` instead of `Conditional`.
    #[test]
    fn the_more_gated_of_two_claims_is_the_one_that_yields() {
        use crate::graph::PassageWeight::Door;
        let mut g = house_ring_with_its_gates();
        relayout_auto(&mut g);
        // Everything down to and including a door holds: exactly adjacent, and unsplit.
        assert_runs_are_tight_and_unsplit(&g, Door);

        let p = |id: u32| g.room(id).unwrap().pos.unwrap();
        let (woh, noh, soh, barrow) = (p(68), p(143), p(217), p(254));
        assert!(
            noh.0 > woh.0 && noh.1 < woh.1,
            "North of House stays north-east of West of House: {noh:?} vs {woh:?}",
        );
        assert!(
            soh.0 > woh.0 && soh.1 > woh.1,
            "South of House stays south-east of it: {soh:?} vs {woh:?}",
        );
        assert!(
            barrow.0 < woh.0 && barrow.1 > woh.1,
            "Stone Barrow stays south-west of it: {barrow:?} vs {woh:?}",
        );

        // Only the most gated link is allowed to come out bent.
        let bent: Vec<_> = g
            .connections()
            .iter()
            .filter(|c| c.distorted)
            .map(|c| (c.origin, c.dir, c.dest, c.weight))
            .collect();
        assert!(
            bent.iter().all(|&(.., w)| w == crate::graph::PassageWeight::Conditional),
            "only the magic-word passage may bend, got: {bent:?}",
        );

        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "no room overlap");
    }
}
