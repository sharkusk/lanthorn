//! Logical lane router: routes each drawn edge through reserved lanes in the gaps
//! between rooms, on a doubled-coordinate gap lattice (room cell (c,r) -> (2c,2r);
//! channels live on odd coordinates). Pixel-free — emits lane indices + per-channel
//! lane counts that the renderer turns into gap widths.

use std::collections::{BTreeMap, BTreeSet};
use crate::direction::{grid_offset, is_diagonal, layout_offset, opposite, Direction};
use crate::graph::{Connection, MapGraph, RoomId};
use crate::router::{route_side, side_for, Side};

/// One collapsed (secondary) edge: its unordered pair, its origin room, and its direction.
struct Secondary {
    pair: (RoomId, RoomId),
    origin: RoomId,
    dir: Direction,
}

/// Rank for choosing which passage represents a pair: N, S, E, W, NE, NW, SE, SW, Up, Down,
/// then anything else. Lower wins (SQ-0522).
fn dir_priority(d: Direction) -> u8 {
    match d {
        Direction::N => 0,
        Direction::S => 1,
        Direction::E => 2,
        Direction::W => 3,
        Direction::NE => 4,
        Direction::NW => 5,
        Direction::SE => 6,
        Direction::SW => 7,
        Direction::Up => 8,
        Direction::Down => 9,
        _ => 10, // In/Out/Unknown never reach here — they are badges, not lines
    }
}

/// True when the routing loop can actually PAIR these two into ONE bidirectional connector.
/// `back_edge_idx` pairs an Up/Down edge only with its exact opposite, and never pairs a compass
/// edge with an Up/Down one; anything else goes.
fn can_pair(fwd: Direction, bwd: Direction) -> bool {
    let vertical = |d| matches!(d, Direction::Up | Direction::Down);
    match (vertical(fwd), vertical(bwd)) {
        (true, true) => bwd == opposite(fwd),
        (false, false) => true,
        _ => false,
    }
}

/// Decide which single passage REPRESENTS each room pair on the map; the rest are recorded as
/// `Secondary` and simply not drawn (SQ-0225, generalised by SQ-0522).
///
/// However many ways two rooms connect, they get ONE line. Per pair this keeps at most one edge
/// each way — the two the routing loop then pairs into a single bidirectional connector — and
/// every other edge becomes a secondary. Returns the secondary indices into `graph.connections()`
/// plus the records.
///
/// The choice is: a RECIPROCAL pairing first (the two ends are exact opposites, so the line runs
/// straight and each arrowhead points the way you actually travel), then by direction priority —
/// N, S, E, W, NE, NW, SE, SW, Up, Down. A pairing the router cannot join is not a candidate at
/// all: taking one would draw the second line this pass exists to remove.
///
/// Nothing marks the passages that lost. Two earlier attempts drew them as icons stacked on the
/// room border and both failed in play: the icons competed with the arrowheads for the two or
/// three cells a box side has, and a collapsed staircase's `↑` pointed at a room the retained
/// line did not go to — an icon has no line to follow, so it can say a passage exists but never
/// where it leads. The map now shows one honest path per pair and the room inspector's exit list
/// carries every direction, destination included.
///
/// In/Out/Unknown never appear here: they carry no layout offset, are never routed, and draw as
/// portal badges anchored on the room.
fn select_shared_paths(
    graph: &MapGraph,
) -> (std::collections::BTreeSet<usize>, Vec<Secondary>) {
    use std::collections::{BTreeMap, BTreeSet};
    let conns = graph.connections();
    let mut by_pair: BTreeMap<(RoomId, RoomId), Vec<usize>> = BTreeMap::new();
    for (i, c) in conns.iter().enumerate() {
        if layout_offset(c.dir).is_none() {
            continue; // In/Out/Unknown: portal badges, never routed
        }
        if c.origin == c.dest {
            continue; // a self-loop is drawn only as the room's own `↩` badge — never a line (SQ-1264)
        }
        let pair = (c.origin.min(c.dest), c.origin.max(c.dest));
        by_pair.entry(pair).or_default().push(i);
    }
    let mut secondary_idx: BTreeSet<usize> = BTreeSet::new();
    let mut records: Vec<Secondary> = Vec::new();
    for (pair, idxs) in by_pair {
        let bucket = |origin: RoomId| -> Vec<usize> {
            idxs.iter().copied().filter(|&i| conns[i].origin == origin).collect()
        };
        let (fwd, bwd) = (bucket(pair.0), bucket(pair.1));
        // Best two-way representative: reciprocal first, then the priority of the two ends. Only
        // pairings the router can actually join are considered.
        let best_two_way = fwd
            .iter()
            .flat_map(|&f| bwd.iter().map(move |&b| (f, b)))
            .filter(|&(f, b)| can_pair(conns[f].dir, conns[b].dir))
            .min_by_key(|&(f, b)| {
                let (fd, bd) = (conns[f].dir, conns[b].dir);
                let reciprocal = bd == opposite(fd);
                (
                    std::cmp::Reverse(reciprocal),
                    dir_priority(fd).min(dir_priority(bd)),
                    dir_priority(fd).max(dir_priority(bd)),
                    f,
                    b,
                )
            });
        // Best single edge, for a one-way pair or when no pairing can be joined.
        let best_one_way = idxs
            .iter()
            .copied()
            .min_by_key(|&i| (dir_priority(conns[i].dir), i))
            .expect("a pair has at least one edge");
        let (ret_fwd, ret_bwd) = match best_two_way {
            Some((f, b)) => (Some(f), Some(b)),
            None => (Some(best_one_way), None),
        };
        for &i in &idxs {
            if Some(i) != ret_fwd && Some(i) != ret_bwd {
                secondary_idx.insert(i);
                records.push(Secondary { pair, origin: conns[i].origin, dir: conns[i].dir });
            }
        }
    }
    (secondary_idx, records)
}

/// A routing channel: `H(r)` is the horizontal gap below room-row `r` (line y=2r+1);
/// `V(c)` is the vertical gap right of room-column `c` (line x=2c+1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Channel { H(i32), V(i32) }

/// One laned long-run of a connector inside a channel. `start<=end` is the doubled-coord
/// extent along the channel's free axis (x for H, y for V).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaneSeg { pub channel: Channel, pub lane: u16, pub start: i32, pub end: i32 }

/// A fully-routed connector (one per drawn connection; reciprocal pairs collapsed).
#[derive(Debug, Clone)]
pub struct RoutedConnector {
    pub origin: RoomId,
    pub dest: RoomId,
    pub distorted: bool,
    pub exit: Side,
    pub entry: Side,
    pub points: Vec<(i32, i32)>, // doubled-coord polyline, centre→…→centre
    pub segs: Vec<LaneSeg>,      // laned long-runs (filled by lane assignment)
    /// Slot index of this connector's exit anchor among all connectors sharing the
    /// origin room's exit side (0,1,2,…). Used by the renderer to offset the departure
    /// anchor along the box edge so two connectors on one side don't collide.
    pub exit_slot: u16,
    /// Slot index of this connector's entry anchor among all connectors sharing the
    /// destination room's entry side.
    pub entry_slot: u16,
    /// true when this connector represents a collapsed true-opposite pair
    /// `a→dir→b` + `b→opposite(dir)→a`; the renderer draws a far-end arrow only for these.
    pub reciprocal: bool,
    /// The origin edge's compass direction (so the renderer can pick a diagonal corner).
    pub exit_dir: crate::direction::Direction,
    /// The paired back-edge's compass direction, set only when a bidirectional pairing
    /// collapsed into this connector (used for the far-end diagonal corner). `None` otherwise.
    pub entry_dir: Option<crate::direction::Direction>,
    /// The box CORNER this connector arrives on, seen from the destination — `None` when it
    /// arrives on an ordinary side doorway instead (SQ-0314).
    ///
    /// Resolved once, here, from [`entry_corner`] plus the corner-contention rule; the router
    /// builds its polyline's far end from this and the renderer anchors its arrival from it, so
    /// the two ends of a connector cannot disagree about where it lands.
    pub entry_corner: Option<crate::direction::Direction>,
    /// True for a multi-edge MERGE STUB: an extra edge between an already-connected pair whose
    /// polyline routes from its box-edge exit and ENDS on the trunk (a T-junction) instead of
    /// drawing its own line to the destination. The renderer draws only its departure arrow.
    pub merge: bool,
    /// Compass directions collapsed at the EXIT end (origin room): extra same-pair
    /// edges NOT drawn as their own line. The renderer stamps a marker for each,
    /// beside this connector's exit arrowhead. Empty for ordinary connectors.
    pub secondary_exit: Vec<crate::direction::Direction>,
    /// Compass directions collapsed at the ENTRY end (dest room).
    pub secondary_entry: Vec<crate::direction::Direction>,
}

/// The logical route plan: connectors plus per-channel lane counts.
#[derive(Debug, Clone, Default)]
pub struct RoutePlan {
    pub connectors: Vec<RoutedConnector>,
    pub h_lanes: BTreeMap<i32, u16>,
    pub v_lanes: BTreeMap<i32, u16>,
    /// Gap-lattice corners a DIAGONAL passes through, as `(v_channel, h_channel)` index pairs
    /// (SQ-0314).
    ///
    /// A diagonal carries NO lane: every step of its route touches a room centre or the corner
    /// itself, so `long_runs` finds nothing to lane and both channels it crosses report zero. Left
    /// at that, the renderer gives them the bare `MIN_GUTTER` and the diagonal is crushed into the
    /// tightest gap on the map, with too little room to draw its line at all. This set is how a
    /// diagonal declares the space it needs; the renderer sizes those gaps from it.
    pub diag_corners: BTreeSet<(i32, i32)>,
}

/// Room cell (c,r) → doubled-coordinate centre (2c, 2r).
pub fn cell_to_doubled(cell: (i32, i32)) -> (i32, i32) {
    (cell.0 * 2, cell.1 * 2)
}

/// One doubled-step out of the box on `side` (the exit/entry stub point).
pub fn exit_point(cell: (i32, i32), side: Side) -> (i32, i32) {
    let (x, y) = cell_to_doubled(cell);
    match side {
        Side::Right => (x + 1, y),
        Side::Left => (x - 1, y),
        Side::Top => (x, y - 1),
        Side::Bottom => (x, y + 1),
    }
}

/// The doubled-coord CORNER lattice cell for a diagonal direction: one doubled step out of the
/// box on BOTH axes (NE → up-right, NW → up-left, SE → down-right, SW → down-left).
///
/// Unlike [`exit_point`], the result is already ALL-ODD — the corner *is* its own gap-lattice
/// point, so `snap_to_lattice` is a no-op on it. That is the whole trick behind SQ-0314: a
/// diagonal needs no perpendicular stub to reach the lattice, so it can leave the box corner
/// diagonally and hand straight off to an ordinary orthogonal L.
///
/// Returns `None` for every non-diagonal direction.
pub fn corner_point(cell: (i32, i32), dir: Direction) -> Option<(i32, i32)> {
    if !is_diagonal(dir) {
        return None;
    }
    let (dx, dy) = grid_offset(dir)?;
    let (x, y) = cell_to_doubled(cell);
    Some((x + dx, y + dy))
}

/// The lattice point a connector leaves `cell` from: the box CORNER for a diagonal (SQ-0314),
/// otherwise the `side`'s perpendicular doorway stub.
fn anchor_point(cell: (i32, i32), side: Side, dir: Option<Direction>) -> (i32, i32) {
    dir.and_then(|d| corner_point(cell, d)).unwrap_or_else(|| exit_point(cell, side))
}

/// Pick the destination's entry side: the side of `b_cell` geometrically facing
/// `a_cell` (dominant axis of the offset; horizontal wins ties).
fn entry_side(a_cell: (i32, i32), b_cell: (i32, i32)) -> Side {
    let dx = a_cell.0 - b_cell.0;
    let dy = a_cell.1 - b_cell.1;
    if dx.abs() >= dy.abs() {
        if dx >= 0 { Side::Right } else { Side::Left }
    } else if dy >= 0 {
        Side::Bottom
    } else {
        Side::Top
    }
}

/// Snap a stub point `p` (one even coord when the side is vertical/horizontal) onto the
/// all-odd gap lattice by stepping its even axis toward `toward` (the other endpoint).
fn snap_to_lattice(p: (i32, i32), toward: (i32, i32)) -> (i32, i32) {
    let sx = if p.0 % 2 == 0 { if toward.0 >= p.0 { 1 } else { -1 } } else { 0 };
    let sy = if p.1 % 2 == 0 { if toward.1 >= p.1 { 1 } else { -1 } } else { 0 };
    (p.0 + sx, p.1 + sy)
}

/// L-orientation of a connector's gap-lattice route: which leg comes first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Orient { HorizontalFirst, VerticalFirst }

/// Build the doubled-coord polyline for one connector: centre → exit stub → lattice →
/// horizontal run → vertical run → lattice → entry stub → centre. Horizontal-first by
/// default; see `build_points_orient` to choose the other L.
///
/// `exit_dir`/`entry_dir` are the connector's compass directions; a DIAGONAL one leaves from the
/// box corner instead of a side doorway (SQ-0314). Pass `None` when the direction is not known or
/// not diagonal — the route is then exactly what it was before SQ-0314.
fn build_points(
    a_cell: (i32, i32),
    exit: Side,
    b_cell: (i32, i32),
    entry: Side,
    exit_dir: Option<Direction>,
    entry_dir: Option<Direction>,
) -> Vec<(i32, i32)> {
    build_points_orient(a_cell, exit, b_cell, entry, Orient::HorizontalFirst, exit_dir, entry_dir)
}

/// Like `build_points` but lets the caller pick the L orientation. Both orientations keep
/// the gap-lattice invariant (long runs on odd coords, never through a room) and the
/// `ea==eb` adjacent-straight short-circuit. Horizontal-first corners at `(gb.x, ga.y)`;
/// vertical-first corners at `(ga.x, gb.y)`.
fn build_points_orient(
    a_cell: (i32, i32),
    exit: Side,
    b_cell: (i32, i32),
    entry: Side,
    orient: Orient,
    exit_dir: Option<Direction>,
    entry_dir: Option<Direction>,
) -> Vec<(i32, i32)> {
    let ca = cell_to_doubled(a_cell);
    let cb = cell_to_doubled(b_cell);
    let ea = anchor_point(a_cell, exit, exit_dir);
    let eb = anchor_point(b_cell, entry, entry_dir);
    // Adjacent facing rooms: the exit stub and entry stub are the SAME lattice cell
    // (the shared doorway between the two boxes). Route straight through it with no
    // lattice dip — there is no channel run, hence no lane. Dipping into the channel
    // and back to the same point would draw a dangling out-and-back tail.
    //
    // A reciprocal DIAGONAL pair on diagonally-adjacent rooms lands here too (SQ-0314): both
    // corners resolve to the one shared corner cell between the boxes, so the route collapses to
    // centre → corner → centre — a single pure diagonal, which is exactly the picture we want.
    if ea == eb {
        return vec![ca, ea, cb];
    }
    let ga = snap_to_lattice(ea, cb); // first all-odd point
    let gb = snap_to_lattice(eb, ca); // last all-odd point
    // L on the gap lattice. Horizontal-first: ga → (gb.x, ga.y) → gb. Vertical-first:
    // ga → (ga.x, gb.y) → gb. Both stay on the all-odd lattice (ga, gb both all-odd).
    let corner = match orient {
        Orient::HorizontalFirst => (gb.0, ga.1),
        Orient::VerticalFirst => (ga.0, gb.1),
    };
    let mut pts = vec![ca, ea, ga];
    if corner != ga { pts.push(corner); }
    if gb != corner { pts.push(gb); }
    pts.push(eb);
    pts.push(cb);
    // Drop consecutive duplicates.
    pts.dedup();
    // Merge collinear consecutive points: a degenerate L (zero-length leg) leaves three
    // collinear points, which would split one straight channel run into two windows and
    // hence two LaneSegs that could be assigned different lanes — drawing the single line
    // as a diagonal. Collapse them so each straight run is one segment.
    merge_collinear(&mut pts);
    pts
}

/// Build a multi-edge MERGE STUB polyline (doubled coords): from origin `a`'s `exit` side, route
/// orthogonally to the trunk's junction point (`trunk[len-2]`, the last point before the trunk's
/// destination centre) and END there, so the stub terminates ON the trunk as a T-junction rather
/// than drawing its own line to the destination. Returns `centre → exit-stub → [corner] → junction`.
fn merge_stub_points(a: (i32, i32), exit: Side, trunk: &[(i32, i32)]) -> Option<Vec<(i32, i32)>> {
    if trunk.len() < 2 {
        return None;
    }
    let junction = trunk[trunk.len() - 2];
    let ca = cell_to_doubled(a);
    let ea = exit_point(a, exit);
    // Horizontal-first L: move in x to the junction's column at the exit-stub's row, then in y.
    // The corner lands in a gutter (away from the origin box) for the cardinal exit sides.
    let corner = (junction.0, ea.1);
    let mut pts = vec![ca, ea];
    if corner != ea && corner != junction {
        pts.push(corner);
    }
    if *pts.last().unwrap() != junction {
        pts.push(junction);
    }
    pts.dedup();
    merge_collinear(&mut pts);
    if pts.len() < 2 {
        return None;
    }
    Some(pts)
}

/// The room cells a straight axis-aligned segment from `p` to `q` passes through (inclusive).
fn line_cells(p: (i32, i32), q: (i32, i32)) -> Vec<(i32, i32)> {
    let dx = (q.0 - p.0).signum();
    let dy = (q.1 - p.1).signum();
    let mut v = vec![p];
    let mut c = p;
    while c != q {
        c = (c.0 + dx, c.1 + dy);
        v.push(c);
    }
    v
}

/// Try to route `a → b` as a DIRECT path along the room grid (no channel dip): leave `a` on its
/// `exit` side, run straight to the single turning corner, then straight to `b`. The exit side
/// fixes the first leg's axis (East/West → run horizontally to b's column then turn; North/South
/// → run vertically to b's row then turn), so the corner is determined. Returns the doubled-coord
/// polyline (on even, room-grid coords — carries no lane) plus the entry side, but ONLY when the
/// exit faces toward `b` and EVERY cell both legs pass through (besides `a`/`b`) is empty.
/// Returns None when the path is blocked or the exit points away from `b` — then the caller dips
/// into the lane channels. Subsumes the straight (zero-turn) case when `b` is on `a`'s row/column.
fn direct_route(
    a: (i32, i32),
    exit: Side,
    b: (i32, i32),
    occupied: &std::collections::BTreeSet<(i32, i32)>,
) -> Option<(Side, Vec<(i32, i32)>)> {
    use std::cmp::Ordering::{Equal, Greater, Less};
    // `corner` is where the path turns; `entry` is the side of `b` it arrives on.
    let (corner, entry) = match exit {
        Side::Right => {
            if b.0 <= a.0 { return None; } // exit east but b not east → would route backwards
            ((b.0, a.1), match b.1.cmp(&a.1) { Greater => Side::Top, Less => Side::Bottom, Equal => Side::Left })
        }
        Side::Left => {
            if b.0 >= a.0 { return None; }
            ((b.0, a.1), match b.1.cmp(&a.1) { Greater => Side::Top, Less => Side::Bottom, Equal => Side::Right })
        }
        Side::Bottom => {
            if b.1 <= a.1 { return None; }
            ((a.0, b.1), match b.0.cmp(&a.0) { Greater => Side::Left, Less => Side::Right, Equal => Side::Top })
        }
        Side::Top => {
            if b.1 >= a.1 { return None; }
            ((a.0, b.1), match b.0.cmp(&a.0) { Greater => Side::Left, Less => Side::Right, Equal => Side::Bottom })
        }
    };
    // Every cell on both legs except the endpoints must be empty.
    let blocked = line_cells(a, corner)
        .into_iter()
        .chain(line_cells(corner, b))
        .any(|c| c != a && c != b && occupied.contains(&c));
    if blocked {
        return None;
    }
    let mut pts = vec![cell_to_doubled(a), exit_point(a, exit)];
    if corner != b {
        pts.push(cell_to_doubled(corner)); // the single turn; absent when b is aligned (straight)
    }
    pts.push(exit_point(b, entry));
    pts.push(cell_to_doubled(b));
    pts.dedup(); // adjacent rooms collapse the two stub points into one shared doorway
    Some((entry, pts))
}

/// Remove interior points that lie on the straight line between their neighbours (same x
/// or same y on both sides), so each maximal straight run is a single polyline segment.
fn merge_collinear(pts: &mut Vec<(i32, i32)>) {
    let mut i = 1;
    while i + 1 < pts.len() {
        let (a, b, c) = (pts[i - 1], pts[i], pts[i + 1]);
        let collinear = (a.0 == b.0 && b.0 == c.0) || (a.1 == b.1 && b.1 == c.1);
        if collinear {
            pts.remove(i);
        } else {
            i += 1;
        }
    }
}

/// Count how many times polyline `a` crosses polyline `b`: a lattice cell where a HORIZONTAL
/// run of one polyline passes through the INTERIOR of a VERTICAL run of the other (or vice
/// versa). The crossing cell must be the strict interior of the run it cuts ACROSS — so a
/// connector's turning corner landing mid-span of another connector's straight run counts (it
/// renders as a `┼`), while two connectors merely touching corner-to-corner (each at its own
/// run end) does not. Room-exit stubs are already excluded by `long_runs`. This is the metric
/// the greedy router minimizes.
fn count_crossings(a: &[(i32, i32)], b: &[(i32, i32)]) -> usize {
    // A horizontal run (covering x∈[hs,he] at y=hy) cuts ACROSS a vertical run (at x=vx,
    // covering y∈[vs,ve]) when the horizontal run reaches x=vx (closed) AND hy is in the
    // STRICT interior of the vertical run's y-extent (the vertical run passes straight
    // through the crossing cell). Counting it for the run that passes straight through means
    // corner-on-interior crossings are caught without double-counting the symmetric case.
    fn hv_cross(h: &[(Channel, i32, i32)], v: &[(Channel, i32, i32)]) -> usize {
        let mut n = 0;
        for &(hc, hs, he) in h {
            let Channel::H(r) = hc else { continue };
            let hy = 2 * r + 1;
            for &(vc, vs, ve) in v {
                let Channel::V(c) = vc else { continue };
                let vx = 2 * c + 1;
                // horizontal reaches vx (closed) and cuts the vertical run's interior in y.
                if hs <= vx && vx <= he && vs < hy && hy < ve { n += 1; }
            }
        }
        n
    }
    let ra = long_runs(a);
    let rb = long_runs(b);
    hv_cross(&ra, &rb) + hv_cross(&rb, &ra)
}

/// True if polylines `a` and `b` share a PARALLEL collinear overlap that the lane system
/// cannot separate into a clean crossing: two runs on the SAME channel (same H-line or same
/// V-line) whose extents overlap by more than a single shared turning corner. Such an
/// overlap renders as a line-on-line stomp (the renderer's no-overlap gate rejects it), so a
/// greedy candidate that introduces one is disqualified.
fn has_parallel_overlap(a: &[(i32, i32)], b: &[(i32, i32)]) -> bool {
    let ra = long_runs(a);
    let rb = long_runs(b);
    for &(ca, sa, ea) in &ra {
        for &(cb, sb, eb) in &rb {
            if ca == cb {
                // overlap length of the two closed extents on the shared channel line
                let lo = sa.max(sb);
                let hi = ea.min(eb);
                if hi - lo >= 1 { return true; } // >1 cell of shared collinear run
            }
        }
    }
    false
}

/// Every straight collinear run of a polyline as `(is_horizontal, line, start, end)`, over ALL grid
/// lines — room rows/columns (even coords) AND channels (odd). Unlike `long_runs` (channels only),
/// this also sees DIRECT routes, which run on room grid lines and carry no lane.
fn all_runs(points: &[(i32, i32)]) -> Vec<(bool, i32, i32, i32)> {
    let mut runs = Vec::new();
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a.1 == b.1 && a.0 != b.0 {
            runs.push((true, a.1, a.0.min(b.0), a.0.max(b.0)));
        } else if a.0 == b.0 && a.1 != b.1 {
            runs.push((false, a.0, a.1.min(b.1), a.1.max(b.1)));
        }
    }
    runs
}

/// True if two polylines share a parallel collinear run of more than one cell on the same grid line
/// — a line-on-line stomp the renderer can't separate. Sees room-line runs too (see `all_runs`), so
/// it catches two DIRECT routes hugging the same room row/column, which `has_parallel_overlap` (which
/// only inspects laneable channel runs) misses.
///
/// Only each route's INTERIOR is compared — the first and last segment are the room-exit doorway
/// stubs. Two connectors entering the same room legitimately share its doorway (rendered as parallel
/// arrows out one box side), so that shared stub must not read as a stomp.
fn polylines_overlap(a: &[(i32, i32)], b: &[(i32, i32)]) -> bool {
    let ai: &[(i32, i32)] = if a.len() > 2 { &a[1..a.len() - 1] } else { &[] };
    let bi: &[(i32, i32)] = if b.len() > 2 { &b[1..b.len() - 1] } else { &[] };
    for &(ah, al, a_s, a_e) in &all_runs(ai) {
        for &(bh, bl, b_s, b_e) in &all_runs(bi) {
            if ah == bh && al == bl && a_s.max(b_s) < a_e.min(b_e) {
                return true; // >1 cell of shared collinear run on the same line
            }
        }
    }
    false
}

/// The destination side a ONE-WAY connector arrives on, fixed by its compass direction so the
/// arrow lands on the matching side regardless of the rooms' exact offset. A cardinal enters the
/// side opposite the one it left — it reads as travelling straight through (N enters from the
/// south, S from the north, E from the west, W from the east). Up/Down enter opposite the side
/// they departed, same as a cardinal (Up→Bottom, Down→Top). The remaining non-planar directions
/// (In/Out/Unknown, already filtered from `compass`) → None.
///
/// A DIAGONAL arrives on the corner FACING its origin, so its side is just the exit rule applied
/// to the reversed direction: `side_for(opposite(dir))` — NE→Left, NW→Right, SE→Right, SW→Left
/// (SQ-0314). The old rule rotated a diagonal onto a cardinal side (NW→N, NE→E, SE→S, SW→W),
/// which was arbitrary but harmless while every diagonal left from a side midpoint. Once a
/// diagonal leaves the box CORNER it is not harmless: an NE edge would leave its origin's
/// top-right corner and then wrap all the way around the destination to enter it from the east.
/// Mirroring the exit rule keeps departure and arrival on facing corners.
fn oneway_entry_side(dir: Direction) -> Option<Side> {
    if is_diagonal(dir) {
        return side_for(opposite(dir));
    }
    Some(match dir {
        Direction::N => Side::Bottom,
        Direction::S => Side::Top,
        Direction::E => Side::Left,
        Direction::W => Side::Right,
        Direction::Up => Side::Bottom,
        Direction::Down => Side::Top,
        Direction::NE | Direction::NW | Direction::SE | Direction::SW => unreachable!("handled above"),
        Direction::In | Direction::Out | Direction::Unknown => return None,
    })
}

/// The corner a connector ARRIVES on, seen from the destination — `Some(dir)` when the arrival
/// anchors on a box corner instead of a side doorway (SQ-0314). The renderer picks its arrival
/// anchor with this, and the router builds the matching polyline endpoint with it, so the two
/// cannot drift apart.
///
/// - A reciprocal DIAGONAL pair uses the back edge's own direction: a true NE/SW pair arrives on
///   the SW corner, and both corners resolve to the one shared corner between adjacent boxes.
/// - A ONE-WAY diagonal has no back edge but must still arrive on the corner facing its origin,
///   so it uses `opposite(exit_dir)`.
/// - A non-diagonal back edge (NE out, plain S back) keeps its ordinary side doorway.
pub fn entry_corner(exit_dir: Direction, entry_dir: Option<Direction>) -> Option<Direction> {
    match entry_dir {
        Some(d) => is_diagonal(d).then_some(d),
        None => is_diagonal(exit_dir).then(|| opposite(exit_dir)),
    }
}

/// For every corner that two or more ONE-WAY diagonals want to arrive on, the `compass` index of
/// the connector that keeps it. Corners wanted by only one arrival are absent.
///
/// Since SQ-1274, a one-way arrival never actually KEEPS a corner (`resolve_entry_corner` denies
/// it unconditionally — every one-way diagonal falls to an ordinary side slot instead, the same
/// as any other one-way arrival). This still matters for the one case that survives it: a
/// RECIPROCAL pair's own corner claim (which IS the return path, and is exempt) can share a key
/// with some unrelated one-way diagonal that also wants it; `resolve_entry_corner`'s call site
/// consults this so that phantom contest doesn't cost the reciprocal its rightful corner.
///
/// The winner is the UNDISTORTED arrival where there is one: its direction actually matches the
/// geometry, so it has the better claim to the corner the geometry points at. Only one arrival CAN
/// be undistorted — an undistorted `NE` requires the origin to sit SW-adjacent, and one cell holds
/// one room — so this decides all but pathological ties, which fall back to the lowest index.
/// Computed from the graph, so it does not depend on the order connectors happen to be routed in.
///
/// Reciprocals never appear here: a reciprocal owns its corner via its own back edge.
fn arrival_corner_owners(compass: &[&Connection]) -> std::collections::HashMap<(RoomId, Direction), usize> {
    let mut best: std::collections::HashMap<(RoomId, Direction), (bool, usize)> =
        std::collections::HashMap::new();
    for (ci, c) in compass.iter().enumerate() {
        if !is_diagonal(c.dir) {
            continue;
        }
        // Any reverse compass edge makes this a reciprocal, not a one-way arrival.
        if compass.iter().any(|b| b.origin == c.dest && b.dest == c.origin) {
            continue;
        }
        let key = (c.dest, opposite(c.dir));
        let cand = (c.distorted, ci); // false < true: undistorted wins, then lowest index
        best.entry(key).and_modify(|e| { if cand < *e { *e = cand } }).or_insert(cand);
    }
    best.into_iter().map(|(k, (_, ci))| (k, ci)).collect()
}

/// Resolve where a connector ARRIVES: its corner, or `None` for an ordinary side doorway.
///
/// SQ-1274: a corner is one of a room's eight compass slots (the four side arrowhead cells plus
/// the four corners), and a one-way arrival landing on ANY of them reads as "that direction leads
/// back here" — which isn't true, or it would BE a reciprocal. So a one-way arrival never keeps
/// the corner, unconditionally, whether or not the room's own outgoing diagonal (or anything
/// else) also wants it; it falls to an ordinary side doorway instead, exactly like a one-way
/// arrival that was never diagonal at all (`assign_side_slots` keeps that side's own center cell
/// off-limits to it there too, the cardinal half of the same rule).
///
/// The rule only binds ONE-WAY arrivals. When a back edge exists, that edge IS this connector — the
/// router collapsed the pair — so its arrival IS the return path (a reciprocal is exempt) and it
/// keeps the corner its own back edge names.
fn resolve_entry_corner(exit_dir: Direction, entry_dir: Option<Direction>) -> Option<Direction> {
    entry_dir?; // one-way (no back edge): never a corner, per the doc comment above.
    entry_corner(exit_dir, entry_dir)
}

/// The candidate entry sides for a NON-reciprocal connector from `a` to `b`: the
/// geometric entry side plus the next-nearest side that still faces the origin, in a
/// deterministic order. The geometric side is always first (the default), so when no
/// alternative reduces crossings the chosen route is unchanged.
fn entry_side_alternatives(a_cell: (i32, i32), b_cell: (i32, i32)) -> Vec<Side> {
    let primary = entry_side(a_cell, b_cell);
    let dx = a_cell.0 - b_cell.0;
    let dy = a_cell.1 - b_cell.1;
    // The secondary side is the dominant-of-the-other-axis side that still faces the
    // origin. Only offer it when the origin is genuinely off-axis on that axis.
    let secondary = if dx.abs() >= dy.abs() {
        if dy > 0 { Some(Side::Bottom) } else if dy < 0 { Some(Side::Top) } else { None }
    } else if dx > 0 { Some(Side::Right) } else if dx < 0 { Some(Side::Left) } else { None };
    let mut out = vec![primary];
    if let Some(s) = secondary { if s != primary { out.push(s); } }
    out
}

/// Route every drawn (compass) edge into a connector polyline. True reciprocal pairs
/// (`a→dir→b` + `b→opposite(dir)→a`) collapse to one connector; which member is kept
/// depends on insertion order in `connections()` (the first-seen direction is drawn).
pub fn route_topology(graph: &MapGraph) -> Vec<RoutedConnector> {
    // Build the connectors two ways and keep the one with fewer total crossings:
    //   - `default`: every connector on its canonical horizontal-first / geometric-entry route;
    //   - `greedy`: each connector picks, sequentially, the candidate route that crosses the
    //     fewest ALREADY-placed connectors.
    // Greedy is a local heuristic and can occasionally do worse than the canonical layout on
    // a given graph; keeping the better of the two guarantees crossing reduction is never a
    // regression. Both are deterministic, so the tiebreak (prefer `default` on an exact tie)
    // keeps the whole routine deterministic.
    let default = route_topology_with(graph, false);
    // Accept the greedy reroute only if it BOTH lowers the render-faithful crossing total AND
    // introduces NO new parallel line-on-line overlap pair beyond those already present (and
    // handled cleanly) in the canonical `default` layout. The default is the long-standing
    // overlap-free render, so any overlap it already contains is renderer-safe; greedy must
    // not add a new one. Otherwise we keep `default`. This makes crossing reduction a strict,
    // never-regressing improvement and keeps the renderer's no-overlap gate green.
    let greedy = route_topology_with(graph, true);
    if total_crossings(&greedy) < total_crossings(&default)
        && !greedy_adds_overlap(&default, &greedy)
    {
        greedy
    } else {
        default
    }
}

/// A dirty shared cell: the lattice cell plus the sorted identities of the connectors that
/// collide there, used to compare overlaps across two candidate layouts.
type DirtyCell = ((i32, i32), Vec<(RoomId, RoomId)>);

/// Every lattice cell shared by ≥2 connectors that is NOT a clean perpendicular crossing
/// (a single horizontal pass-through + a single vertical pass-through). The full polyline
/// trace is walked, so room-exit stubs and turning corners are included — exactly the cells
/// where a parallel run, corner-on-corner, T-stomp, or ≥3-way share would render as an
/// illegal overlap. Returned keyed by the involved connectors' `(origin,dest)` identities so
/// the two layouts can be compared connector-for-connector.
fn dirty_shared_cells(conns: &[RoutedConnector]) -> std::collections::BTreeSet<DirtyCell> {
    use std::collections::{BTreeSet, HashMap};
    const N: u8 = 1; const S: u8 = 2; const E: u8 = 4; const W: u8 = 8;
    let mut cells: HashMap<(i32, i32), HashMap<usize, u8>> = HashMap::new();
    for (ci, c) in conns.iter().enumerate() {
        for w in c.points.windows(2) {
            let (a, b) = (w[0], w[1]);
            let dx = (b.0 - a.0).signum();
            let dy = (b.1 - a.1).signum();
            let bit = if dx > 0 { E } else if dx < 0 { W } else if dy > 0 { S } else { N };
            let mut cur = a;
            loop {
                *cells.entry(cur).or_default().entry(ci).or_insert(0) |= bit;
                if cur == b { break; }
                cur = (cur.0 + dx, cur.1 + dy);
            }
        }
    }
    let mut dirty = BTreeSet::new();
    let (horiz, vert) = (E | W, N | S);
    for (cell, per_conn) in &cells {
        if per_conn.len() < 2 { continue; }
        let masks: Vec<u8> = per_conn.values().copied().collect();
        // A clean perpendicular crossing: exactly two connectors, one straight-horizontal and
        // one straight-vertical.
        let clean_cross = per_conn.len() == 2
            && ((masks[0] == horiz && masks[1] == vert) || (masks[0] == vert && masks[1] == horiz));
        // A pure-parallel overlap: every contributor is the SAME single straight orientation
        // (all horizontal or all vertical, none turning here). The lane system separates these
        // onto distinct pixel lines, so they are NOT a render overlap.
        let all_h = masks.iter().all(|&m| m == E || m == W || m == horiz);
        let all_v = masks.iter().all(|&m| m == N || m == S || m == vert);
        let parallel_lane_separable = all_h || all_v;
        if !clean_cross && !parallel_lane_separable {
            let mut ids: Vec<(RoomId, RoomId)> =
                per_conn.keys().map(|&ci| (conns[ci].origin, conns[ci].dest)).collect();
            ids.sort();
            dirty.insert((*cell, ids));
        }
    }
    dirty
}

/// True if `greedy` introduces a dirty (non-clean-crossing) shared cell that the canonical
/// `default` layout does not already contain. The default is the long-standing overlap-free
/// render, so any dirty share it already has is renderer-safe; greedy must not add a new one.
/// Compared by `(cell, involved-connector-identities)` so a pre-existing safe share is not
/// counted against greedy.
fn greedy_adds_overlap(default: &[RoutedConnector], greedy: &[RoutedConnector]) -> bool {
    let base = dirty_shared_cells(default);
    dirty_shared_cells(greedy).iter().any(|d| !base.contains(d))
}

/// Render-faithful total crossing count for a set of connectors: lanes are assigned exactly
/// as the renderer does, then a `┼` is counted wherever a horizontal run of one connector and
/// a vertical run of a DIFFERENT connector meet at a lattice cell (closed extents — corners
/// included), since after lane offsetting two parallel runs sit on distinct pixel lines and a
/// perpendicular crosser cuts each. This mirrors the renderer's per-cell `┼` detection, so
/// minimizing it tracks the visible crossing count rather than a raw-lattice proxy.
fn total_crossings(conns: &[RoutedConnector]) -> usize {
    // (connector idx, channel, start, end) for every long run, with a stable run id.
    let mut runs: Vec<(usize, Channel, i32, i32)> = Vec::new(); // (run id, channel, s, e)
    let mut owner: Vec<usize> = Vec::new(); // run id → connector idx
    for (ci, c) in conns.iter().enumerate() {
        for (ch, s, e) in long_runs(&c.points) {
            runs.push((owner.len(), ch, s, e));
            owner.push(ci);
        }
    }
    let (lane_of, _counts) = assign_lanes(&runs);
    // Split runs into horizontals and verticals, carrying connector idx and lane.
    let mut horiz: Vec<(usize, u16, i32, i32, i32)> = Vec::new(); // (conn, lane, y, xs, xe)
    let mut vert: Vec<(usize, u16, i32, i32, i32)> = Vec::new(); // (conn, lane, x, ys, ye)
    for &(id, ch, s, e) in &runs {
        let lane = lane_of[&id];
        match ch {
            Channel::H(r) => horiz.push((owner[id], lane, 2 * r + 1, s, e)),
            Channel::V(c) => vert.push((owner[id], lane, 2 * c + 1, s, e)),
        }
    }
    let mut n = 0;
    for &(hc, _hl, hy, hxs, hxe) in &horiz {
        for &(vc, _vl, vx, vys, vye) in &vert {
            if hc == vc { continue; } // a connector never crosses itself
            // Closed-extent perpendicular meeting: the horizontal spans vx and the vertical
            // spans hy. Distinct connectors meeting here render as a single ┼.
            if hxs <= vx && vx <= hxe && vys <= hy && hy <= vye {
                n += 1;
            }
        }
    }
    n
}

/// The unconsumed reverse edge (b→a) paired with edge `ci`, preferring the exact compass-opposite
/// so the trunk keeps its natural straight / single-L shape, falling back to the first reverse edge.
///
/// Up/Down edges pair ONLY with the exact opposite Up/Down edge between the same two rooms
/// (`Up`'s only possible back-edge is `Down`, and vice versa — there is no "first reverse edge"
/// fallback since that single candidate IS the exact opposite). A matching pair collapses into
/// one reciprocal connector (SQ-0216 revises Task 6's "never collapse" decision after visual
/// testing). An Up/Down `c` never pairs with a compass edge, and a compass `c` never pairs with
/// an Up/Down edge either (the fallback would otherwise pair e.g. `1→N→2` with an unrelated
/// `2→Down→1`, which are not geometrically opposite) — compass pairing stays byte-identical.
fn back_edge_idx(
    compass: &[&Connection],
    ci: usize,
    c: &Connection,
    consumed: &std::collections::BTreeSet<usize>,
) -> Option<usize> {
    if matches!(c.dir, Direction::Up | Direction::Down) {
        return compass
            .iter()
            .enumerate()
            .find(|&(pi, p)| {
                pi != ci
                    && !consumed.contains(&pi)
                    && p.origin == c.dest
                    && p.dest == c.origin
                    && p.dir == opposite(c.dir)
            })
            .map(|(pi, _)| pi);
    }
    let is_back = |pi: usize, p: &Connection| {
        pi != ci
            && !consumed.contains(&pi)
            && p.origin == c.dest
            && p.dest == c.origin
            && !matches!(p.dir, Direction::Up | Direction::Down)
    };
    compass
        .iter()
        .enumerate()
        .find(|&(pi, p)| is_back(pi, p) && p.dir == opposite(c.dir))
        .or_else(|| compass.iter().enumerate().find(|&(pi, p)| is_back(pi, p)))
        .map(|(pi, _)| pi)
}

/// Manhattan length of a polyline in doubled coords — used to rank competing direct routes.
fn path_len(pts: &[(i32, i32)]) -> i32 {
    pts.windows(2).map(|w| (w[0].0 - w[1].0).abs() + (w[0].1 - w[1].1).abs()).sum()
}

/// One `direct_route_losers` candidate: (compass index, is the route collinear/straight, polyline).
type DirectCand = (usize, bool, Vec<(i32, i32)>);

/// Decide which connectors may keep a straight DIRECT route when several would collide on the same
/// room line. Two direct routes on one room line cannot be separated (a direct route carries no
/// lane), so at most one survives; the LONGER one wins (its detour through channels would be the
/// ugliest) and the shorter ones are returned as "losers" that must weave instead. When two
/// candidates tie on length, the COLLINEAR (straight) one wins over a bent L — a straight room-line
/// is the whole point of a direct route, and a bent one is already halfway to weaving, so it loses
/// nothing by giving way (SQ-1255). Only once length and straightness both tie does edge-listing
/// index settle it, which is the one part of this that is still, unavoidably, order-dependent.
/// Without the straightness tiebreak the outcome was order-dependent on the tie itself — whichever
/// edge is listed first kept the line — which can force a long straight path to weave around a
/// short bent one (e.g. 76↔78 weaving aside for the short 180↔78).
fn direct_route_losers(
    compass: &[&Connection],
    compass_pairs: &std::collections::BTreeSet<(RoomId, RoomId)>,
    graph: &MapGraph,
    occupied: &std::collections::BTreeSet<(i32, i32)>,
) -> std::collections::BTreeSet<usize> {
    // One representative per room pair — the first listed edge, matching where the main loop attempts
    // the pair's direct route (reverse/extra edges are consumed or become merge stubs there).
    let mut seen_pairs: std::collections::BTreeSet<(RoomId, RoomId)> = std::collections::BTreeSet::new();
    let empty = std::collections::BTreeSet::new();
    let mut cands: Vec<DirectCand> = Vec::new();
    for (ci, c) in compass.iter().enumerate() {
        let pair = (c.origin.min(c.dest), c.origin.max(c.dest));
        // A pair's direct room-line belongs to its COMPASS edge. When a pair has both, skip the
        // Up/Down edge so it never becomes the pair's representative (SQ-0224) — the compass edge
        // (listed elsewhere) takes the contest. Pure Up/Down pairs are absent from `compass_pairs`
        // and route exactly as before.
        if matches!(c.dir, Direction::Up | Direction::Down) && compass_pairs.contains(&pair) {
            continue;
        }
        if !seen_pairs.insert(pair) {
            continue;
        }
        let (Some(a), Some(b)) = (graph.room(c.origin).and_then(|r| r.pos),
                                  graph.room(c.dest).and_then(|r| r.pos)) else { continue };
        let Some(exit) = route_side(c.dir) else { continue };
        let required_entry =
            back_edge_idx(compass, ci, c, &empty).and_then(|pi| route_side(compass[pi].dir));
        if let Some((sentry, pts)) = direct_route(a, exit, b, occupied) {
            if required_entry.is_none_or(|req| req == sentry) {
                let straight = is_collinear(&pts);
                cands.push((ci, straight, pts));
            }
        }
    }
    // Longest first; on a length tie, straight (collinear) beats bent; index breaks any remaining
    // tie deterministically. A candidate that collides with an already accepted winner becomes a
    // loser.
    cands.sort_by(|x, y| {
        path_len(&y.2).cmp(&path_len(&x.2)).then(y.1.cmp(&x.1)).then(x.0.cmp(&y.0))
    });
    let mut accepted: Vec<Vec<(i32, i32)>> = Vec::new();
    let mut losers = std::collections::BTreeSet::new();
    for (ci, _straight, pts) in cands {
        if accepted.iter().any(|w| polylines_overlap(&pts, w)) {
            losers.insert(ci);
        } else {
            accepted.push(pts);
        }
    }
    losers
}

/// Route every drawn (compass) edge. When `greedy` is false, each connector takes its
/// canonical route (horizontal-first L, geometric entry side). When true, each connector is
/// routed sequentially and picks — among its candidate routes (both L orientations, plus the
/// alternative facing entry side for non-reciprocal connectors) — the one that crosses the
/// fewest already-placed connectors, with a deterministic integer tiebreak.
fn route_topology_with(graph: &MapGraph, greedy: bool) -> Vec<RoutedConnector> {
    // Draw every compass edge on its OWN exit side, collapsing ONE bidirectional pairing
    // (a→b together with a single b→a, regardless of the two directions) into a single
    // connector: the renderer puts an arrow at each end, each pointing out that end's own
    // compass side. An exact-opposite pair is the straight-line special case. ADDITIONAL
    // edges between the same room pair (e.g. a third direction, 239→S→77 alongside
    // 77→E→239 and 239→N→77) stay separate, so every distinct direction remains visible.
    // Up/Down edges route as their OWN connectors even when the pair also has a compass edge
    // (SQ-0224): a vertical passage and a horizontal one between the same two rooms are distinct and
    // both draw. Two guards keep Up/Down from disturbing compass routing: (1) an Up/Down edge and a
    // compass edge of the same pair sit on SEPARATE trunks (`ch_key` below) so neither collapses into
    // the other as a merge stub; (2) an Up/Down edge on a pair that ALSO has a compass edge stays out
    // of the compass straight-line contest (`direct_route_losers`) so it cannot steal that pair's
    // direct-room-line representative. `compass_pairs` = the unordered pairs that carry a compass
    // (grid_offset — the true 8-way directions) edge; pure Up/Down pairs are absent from it and keep
    // their prior routing exactly.
    let compass_pairs: std::collections::BTreeSet<(RoomId, RoomId)> = graph
        .connections()
        .iter()
        .filter(|c| grid_offset(c.dir).is_some() && c.origin != c.dest)
        .map(|c| (c.origin.min(c.dest), c.origin.max(c.dest)))
        .collect();
    // Pre-pass (SQ-0225): collapse redundant same-pair compass edges into one shared
    // connector, recording the dropped edges as secondaries attached to the retained one.
    let (secondary_idx, secondary_records) = select_shared_paths(graph);
    // Working set: every edge with a layout offset (compass + Up/Down), minus edges collapsed
    // into a secondary above. In/Out/Unknown carry no offset and are not routed here.
    //
    // A SELF-LOOP (`origin == dest`) is excluded too (SQ-1264): it is drawn only as the room's own
    // `↩` badge (`render/map.rs`), never as a line — before this guard `select_shared_paths` paired
    // a self-loop edge with itself (its `origin==dest` pair's forward and backward buckets are the
    // SAME set of indices) and produced a bogus polyline that looped back around the room's own box.
    let compass: Vec<&crate::graph::Connection> = graph
        .connections()
        .iter()
        .enumerate()
        .filter(|(i, c)| layout_offset(c.dir).is_some() && c.origin != c.dest && !secondary_idx.contains(i))
        .map(|(_, c)| c)
        .collect();
    // Edge indices already drawn — either as their own connector or consumed as the
    // back-edge of an earlier bidirectional pairing.
    let mut consumed: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    let occupied: std::collections::BTreeSet<(i32, i32)> =
        graph.rooms().filter_map(|r| r.pos).collect();
    // Resolve direct-route line collisions up front so the LONGER connector keeps the straight line
    // and shorter rivals weave — independent of edge listing order.
    let direct_losers = direct_route_losers(&compass, &compass_pairs, graph, &occupied);
    // Which one-way arrival keeps each contested corner (SQ-0314); see `arrival_corner_owners` —
    // since SQ-1274 this only still matters for a reciprocal contesting an unrelated one-way's
    // phantom claim, not for settling who keeps a corner between one-ways (neither ever does).
    let arr_owners = arrival_corner_owners(&compass);
    let mut out: Vec<RoutedConnector> = Vec::new();
    // The trunk polyline per (unordered room pair, channel), recorded when its connector is built; a
    // later edge on the SAME pair AND channel becomes a MERGE STUB that joins this trunk. The channel
    // bit separates the compass trunk from the Up/Down trunk (SQ-0224) so a pair's vertical and
    // horizontal passages draw as independent connectors instead of one merging into the other.
    let mut trunk_points: BTreeMap<(RoomId, RoomId, bool), Vec<(i32, i32)>> = BTreeMap::new();
    for (ci, c) in compass.iter().enumerate() {
        if consumed.contains(&ci) {
            continue; // already drawn as an earlier pair's back-edge
        }
        let pair = (c.origin.min(c.dest), c.origin.max(c.dest));
        let ch_key = (pair.0, pair.1, matches!(c.dir, Direction::Up | Direction::Down));
        // Extra edge between an already-connected pair+channel → merge stub: route from this edge's
        // exit side to a junction on the trunk and end there (a T-junction). Only the box-edge
        // departure arrow is drawn; no independent line reaches the destination.
        if let Some(trunk) = trunk_points.get(&ch_key) {
            if let (Some(a), Some(exit)) =
                (graph.room(c.origin).and_then(|r| r.pos), route_side(c.dir))
            {
                if let Some(pts) = merge_stub_points(a, exit, trunk) {
                    out.push(RoutedConnector {
                        origin: c.origin,
                        dest: c.dest,
                        distorted: c.distorted,
                        exit,
                        entry: exit, // unused — no arrival is drawn for a merge stub
                        points: pts,
                        segs: Vec::new(),
                        exit_slot: 0,
                        entry_slot: 0,
                        reciprocal: false,
                        exit_dir: c.dir,
                        entry_dir: None,
                        entry_corner: None, // a merge stub ends on the trunk, not at a box
                        merge: true,
                        secondary_exit: Vec::new(),
                        secondary_entry: Vec::new(),
                    });
                }
            }
            consumed.insert(ci);
            continue;
        }
        // Pair with an UNCONSUMED reverse edge (b→a) between the same rooms, if any. PREFER the
        // geometric reciprocal — the reverse edge whose direction is the exact compass-opposite of
        // this edge — so the trunk keeps its natural straight / single-L shape and its junction lands
        // at the doorway between the rooms. Pairing with an arbitrary reverse edge would force the
        // trunk to enter the far room from that edge's side, bending it up-and-over and leaving the
        // remaining same-pair edges as degenerate merge stubs (their junction ends up on the wrong
        // side, collapsing the stub to a zero-width line). Fall back to the first unconsumed reverse
        // edge when no exact opposite exists. Exactly one pairing collapses; further edges between
        // the pair draw on their own (as merge stubs).
        let back_idx = back_edge_idx(&compass, ci, c, &consumed);
        let back = back_idx.map(|pi| compass[pi]);
        let has_reciprocal = back.is_some();
        let (Some(a), Some(b)) = (graph.room(c.origin).and_then(|r| r.pos),
                                  graph.room(c.dest).and_then(|r| r.pos)) else { continue; };
        let Some(exit) = route_side(c.dir) else { continue; };

        // Direct route: if the path from `a` to `b` is clear along the room grid, draw it as a
        // straight line / single-corner L instead of dipping into channels — fewer turns, no
        // weaving around empty cells. A direct route keeps its natural clean entry: only a reciprocal
        // pair constrains it (the far arrow must point out the back-edge's side). A one-way edge does
        // NOT force the ruled side onto a clean direct route — for an aligned room the natural entry
        // already matches the rule, and for an offset room forcing it would wrap the connector around
        // the box. The ruled side applies to the dipped channel route below. Also reject a direct run
        // that stomps an already-routed connector on a shared room line (it carries no lane).
        let required_entry = back.and_then(|bk| route_side(bk.dir));
        // The destination side a one-way edge prefers when it must dip into the channels.
        let ruled_entry = match back {
            Some(bk) => route_side(bk.dir),
            None => oneway_entry_side(c.dir),
        };
        // Where this connector arrives: a box corner (a reciprocal only — SQ-1274), or an
        // ordinary side doorway. Resolve it BEFORE the direct route is considered — an arrival on
        // a side doorway is a plain orthogonal connector again, and may take the direct route a
        // corner-ended one is barred from.
        let arrive = resolve_entry_corner(c.dir, back.map(|bk| bk.dir))
            // Guards a reciprocal's corner against an unrelated one-way's phantom claim on the
            // same (room, direction) key — see `arrival_corner_owners`'s doc comment. Absent from
            // the map => uncontested => kept.
            .filter(|&d| arr_owners.get(&(c.dest, d)).is_none_or(|&w| w == ci));

        // A connector with a CORNER at either end never takes the direct route (SQ-0314).
        // `direct_route` runs its legs out of the rooms' centre row/column and anchors both ends
        // with `exit_point` — it has no notion of a corner. A diagonal departs from the box CORNER,
        // so the two disagree and the connector visibly doubles back to reconcile them; and a
        // cardinal-out/diagonal-back pair (E out, NW back) would arrive at a corner the direct
        // route never routed to, stranding the arrival with a jog. Both fall through to the
        // corner-anchored lattice route below, which asks `anchor_point` for each end.
        let corner_ended = is_diagonal(c.dir) || arrive.is_some();
        let direct = if corner_ended { None } else { direct_route(a, exit, b, &occupied) };
        if let Some((sentry, pts)) = direct {
            // Reject if this straight run stomps an already-placed connector, or if the pre-pass
            // ruled this the shorter rival on a contested room line (so it weaves and the longer
            // connector keeps the line).
            let stomps_existing = out.iter().any(|o| polylines_overlap(&pts, &o.points));
            let is_loser = direct_losers.contains(&ci);
            if required_entry.is_none_or(|req| req == sentry) && !stomps_existing && !is_loser {
                out.push(RoutedConnector {
                    origin: c.origin,
                    dest: c.dest,
                    distorted: c.distorted,
                    exit,
                    entry: sentry,
                    points: pts,
                    segs: Vec::new(),
                    exit_slot: 0,
                    entry_slot: 0,
                    reciprocal: has_reciprocal,
                    exit_dir: c.dir,
                    entry_dir: back.map(|bk| bk.dir),
                    entry_corner: arrive,
                    merge: false,
                    secondary_exit: Vec::new(),
                    secondary_entry: Vec::new(),
                });
                trunk_points.insert(ch_key, out.last().unwrap().points.clone());
                consumed.insert(ci);
                if let Some(pi) = back_idx {
                    consumed.insert(pi);
                }
                continue;
            }
        }

        // Generate candidate routes. The exit side is fixed by the edge's compass direction;
        // we vary the L orientation and (for non-reciprocal connectors, in greedy mode only)
        // the destination entry side among the sides still facing the origin. Candidate order
        // is fixed and integer-tiebroken, so the choice is deterministic.
        // A bidirectional connector enters the far room on the side ITS OWN back-edge departs, so
        // the far-end arrow points out the true compass direction (an exact-opposite pair resolves
        // to the opposite side = a straight reciprocal). A one-way connector enters on the side
        // fixed by its own direction (`oneway_entry_side`). Either way the entry side is determined,
        // so greedy only varies the L orientation; the `entry_side` fallbacks cover the (filtered)
        // non-planar case for safety.
        let entry_choices: Vec<Side> = match ruled_entry {
            // Reciprocal pairs are fixed to the back-edge side. A one-way edge prefers its ruled
            // side first, but keeps the geometric facing sides as fallbacks so the greedy can still
            // dodge a stomp when the ruled side is blocked (rather than forcing an illegal overlap).
            Some(s) if back.is_some() => vec![s],
            Some(s) => {
                let mut v = vec![s];
                for alt in entry_side_alternatives(a, b) {
                    if !v.contains(&alt) {
                        v.push(alt);
                    }
                }
                v
            }
            None if greedy => entry_side_alternatives(a, b),
            None => vec![entry_side(a, b)],
        };
        let orients: &[Orient] = if greedy {
            &[Orient::HorizontalFirst, Orient::VerticalFirst]
        } else {
            &[Orient::HorizontalFirst]
        };
        // (rank, entry, points): `rank` bakes the deterministic preference — earlier entry
        // choices (geometric first) and HorizontalFirst orientation are preferred on ties.
        type Candidate = (usize, Side, Vec<(i32, i32)>);
        let mut candidates: Vec<Candidate> = Vec::new();
        for (ei, &entry) in entry_choices.iter().enumerate() {
            for (oi, &orient) in orients.iter().enumerate() {
                // Default mode emits the canonical horizontal-first route via `build_points`;
                // greedy mode also probes the vertical-first alternative.
                // The connector's own directions decide where it leaves/arrives: a diagonal one
                // anchors on the box corner (SQ-0314). This is the SAME resolved `arrive` that
                // lands on the connector below, so every candidate polyline is built for the end
                // the renderer will actually anchor.
                let pts = match orient {
                    Orient::HorizontalFirst => build_points(a, exit, b, entry, Some(c.dir), arrive),
                    Orient::VerticalFirst => {
                        build_points_orient(a, exit, b, entry, orient, Some(c.dir), arrive)
                    }
                };
                candidates.push((ei * 2 + oi, entry, pts));
            }
        }
        // Pick the candidate with the fewest crossings against already-placed connectors;
        // ties break by the preference rank, then by the points (fully deterministic).
        let chosen = candidates
            .into_iter()
            .map(|(rank, entry, pts)| {
                // Disqualify candidates that introduce a parallel line-on-line overlap the
                // lane system can't resolve (renderer rejects these): make it the PRIMARY key
                // so an overlap-free route always wins. Among overlap-free candidates, minimize
                // the crossings this candidate adds against the already-placed connectors; ties
                // break by preference rank (geometric entry, horizontal-first), then the points.
                let overlaps = out.iter().filter(|o| has_parallel_overlap(&pts, &o.points)).count();
                let crosses: usize = out.iter().map(|o| count_crossings(&pts, &o.points)).sum();
                (overlaps, crosses, rank, entry, pts)
            })
            .min_by(|x, y| {
                x.0.cmp(&y.0)
                    .then(x.1.cmp(&y.1))
                    .then(x.2.cmp(&y.2))
                    .then(x.4.cmp(&y.4))
            })
            .expect("at least one candidate route");

        out.push(RoutedConnector {
            origin: c.origin,
            dest: c.dest,
            distorted: c.distorted,
            exit,
            entry: chosen.3,
            points: chosen.4,
            segs: Vec::new(),
            exit_slot: 0,
            entry_slot: 0,
            reciprocal: has_reciprocal,
            exit_dir: c.dir,
            entry_dir: back.map(|bk| bk.dir),
            entry_corner: arrive,
            merge: false,
            secondary_exit: Vec::new(),
            secondary_entry: Vec::new(),
        });
        trunk_points.insert(ch_key, out.last().unwrap().points.clone());
        consumed.insert(ci);
        if let Some(pi) = back_idx {
            consumed.insert(pi); // the paired back-edge is now drawn too
        }
    }
    for rec in &secondary_records {
        let same_pair =
            |c: &RoutedConnector| (c.origin.min(c.dest), c.origin.max(c.dest)) == rec.pair;
        // Prefer the pair's compass connector; a pair with nothing but Up/Down has only its
        // dotted one to hang the icons on.
        let idx = out
            .iter()
            .position(|c| same_pair(c) && grid_offset(c.exit_dir).is_some())
            .or_else(|| out.iter().position(same_pair));
        if let Some(conn) = idx.map(|i| &mut out[i]) {
            if rec.origin == conn.origin {
                conn.secondary_exit.push(rec.dir);
            } else if rec.origin == conn.dest {
                conn.secondary_entry.push(rec.dir);
            }
        }
    }
    out
}

/// Extract the long runs of a doubled-coord polyline as (channel, start, end) with
/// start<=end, skipping the room-cell endpoints and zero-length steps. A horizontal run
/// (constant odd y) → `H((y-1)/2)`; a vertical run (constant odd x) → `V((x-1)/2)`.
fn long_runs(points: &[(i32, i32)]) -> Vec<(Channel, i32, i32)> {
    let mut runs = Vec::new();
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a.1 == b.1 && a.1 % 2 != 0 && a.0 != b.0 {
            // horizontal run on channel H[(y-1)/2]
            let (s, e) = (a.0.min(b.0), a.0.max(b.0));
            runs.push((Channel::H((a.1 - 1).div_euclid(2)), s, e));
        } else if a.0 == b.0 && a.0 % 2 != 0 && a.1 != b.1 {
            let (s, e) = (a.1.min(b.1), a.1.max(b.1));
            runs.push((Channel::V((a.0 - 1).div_euclid(2)), s, e));
        }
        // steps touching even coords are the stubs (room↔lattice); they carry no lane.
    }
    runs
}

/// Left-edge interval colouring per channel. Returns, for each input run, the lane it was
/// assigned, plus the per-channel lane count. Runs are processed in a deterministic order
/// (by channel, then start, then end) so assignment is stable.
fn assign_lanes(
    runs: &[(usize, Channel, i32, i32)], // (connector-run id, channel, start, end)
) -> (std::collections::HashMap<usize, u16>, BTreeMap<Channel, u16>) {
    use std::collections::HashMap;
    // bucket by channel
    let mut by_ch: BTreeMap<Channel, Vec<(usize, i32, i32)>> = BTreeMap::new();
    for &(id, ch, s, e) in runs {
        by_ch.entry(ch).or_default().push((id, s, e));
    }
    let mut lane_of: HashMap<usize, u16> = HashMap::new();
    let mut counts: BTreeMap<Channel, u16> = BTreeMap::new();
    for (ch, mut items) in by_ch {
        items.sort_by_key(|&(_, s, e)| (s, e));
        // lane_end[l] = current right edge occupied on lane l (exclusive comparison).
        let mut lane_end: Vec<i32> = Vec::new();
        for (id, s, e) in items {
            // first lane whose last extent ends strictly before s
            let lane = match lane_end.iter().position(|&end| end < s) {
                Some(l) => { lane_end[l] = e; l }
                None => { lane_end.push(e); lane_end.len() - 1 }
            };
            lane_of.insert(id, lane as u16);
        }
        counts.insert(ch, lane_end.len() as u16);
    }
    (lane_of, counts)
}

/// Assign per-(room, side) slot indices to every connector endpoint so that two
/// connectors sharing one room side (e.g. an arrival and a departure, or two departures)
/// anchor on distinct cells along that side instead of colliding on the side centre.
///
/// Each connector has an exit endpoint at `(origin, exit)` and an entry endpoint at
/// `(dest, entry)`. Endpoints are grouped by `(room, side)` and assigned 0,1,2,… in a
/// deterministic order (by room, side, then connector index) so rendering is stable.
/// True if a doubled-coord polyline is a single straight segment (all points share one axis),
/// i.e. the connector runs straight through with no turn.
fn is_collinear(points: &[(i32, i32)]) -> bool {
    points.iter().all(|p| p.0 == points[0].0) || points.iter().all(|p| p.1 == points[0].1)
}

fn assign_side_slots(connectors: &mut [RoutedConnector], graph: &MapGraph) {
    // Collect every endpoint as (room, side, is_exit, connector index).
    //
    // A CORNER endpoint is skipped (SQ-0314): it anchors on the box corner, and the corner anchor
    // ignores the slot entirely — so slotting one only burned a cell that nothing ever drew on,
    // pushing the side's real endpoints off-centre for no reason. The corner is its own slot.
    // Skipped endpoints keep the initial slot 0, which the corner path ignores.
    //
    // The arrival test reads the RESOLVED `entry_corner`, not the raw direction: an arrival that
    // lost its corner to the destination's own outgoing diagonal is back on a side doorway, and it
    // needs a real slot there like any other side endpoint.
    let mut endpoints: Vec<(RoomId, Side, bool, usize)> = Vec::new();
    for (ci, c) in connectors.iter().enumerate() {
        if !is_diagonal(c.exit_dir) {
            endpoints.push((c.origin, c.exit, true, ci));
        }
        // A merge stub ends on the trunk (not at the destination box), so it has no arrival
        // endpoint to slot.
        if !c.merge && c.entry_corner.is_none() {
            endpoints.push((c.dest, c.entry, false, ci));
        }
    }
    // Group by (room, side); assign slots within each group.
    //
    // Compass-only sides keep the historical order: a connector that runs STRAIGHT through the
    // side (a collinear polyline — its other end is axis-aligned with this room) takes the center
    // slot ahead of weaving connectors, so it stays a clean straight line while the weaving ones
    // take the offset cells; ties break by (connector index, is_exit). This is preserved
    // byte-for-byte (compass identity).
    //
    // On a side that ALSO hosts an Up/Down endpoint, the priority is refined (SQ-0222):
    //   1. an AXIS-reciprocal compass connector (reciprocal AND cardinal-opposite exit/entry:
    //      N<->S or E<->W — the column/row-locked case) keeps the center (SQ-0216 intent); then
    //   2. a STRAIGHT-through connector keeps the center (so a straight Up/Down line is not forced
    //      to jog across a weaving one-way compass connector, which manufactures an overlap); then
    //   3. compass before Up/Down, then connector index, for determinism.
    // The blanket SQ-0216 "Up/Down always yields to compass" was too broad: it displaced a
    // straight Up/Down line for a NON-reciprocal weaving compass connector. Gating the refined
    // order to sides that actually host an Up/Down endpoint keeps every compass-only side
    // byte-identical.
    // Endpoint: (is up/down?, axis-reciprocal compass?, straight-through?, is_exit, connector index).
    type Endpoint = (bool, bool, bool, bool, usize);
    let mut by_side: BTreeMap<(RoomId, Side), Vec<Endpoint>> = BTreeMap::new();
    for (room, side, is_exit, ci) in endpoints {
        let straight = is_collinear(&connectors[ci].points);
        let is_updown = matches!(connectors[ci].exit_dir, Direction::Up | Direction::Down);
        let axis_recip = connectors[ci].reciprocal
            && matches!(
                (connectors[ci].exit_dir, connectors[ci].entry_dir),
                (Direction::N, Some(Direction::S))
                    | (Direction::S, Some(Direction::N))
                    | (Direction::E, Some(Direction::W))
                    | (Direction::W, Some(Direction::E))
            );
        by_side.entry((room, side)).or_default().push((is_updown, axis_recip, straight, is_exit, ci));
    }
    for (key, mut members) in by_side {
        let has_updown = members.iter().any(|&(is_updown, ..)| is_updown);
        // SQ-1274: the CENTER slot (0) is the room's own compass anchor for this side — an
        // outgoing exit in that direction, or a reciprocal pair's return path. A plain ONE-WAY
        // connector's ARRIVAL there would draw an arrowhead reading as "that direction leads
        // back here", which isn't true (that's exactly what a reciprocal IS, and why it's
        // exempt). So a one-way arrival is never eligible for center: rank every eligible
        // endpoint (a departure, or a reciprocal's arrival) ahead of every one-way arrival,
        // regardless of the existing tie-break keys below, which still decide order WITHIN each
        // of those two groups.
        let eligible_for_center =
            |is_exit: bool, ci: usize| is_exit || connectors[ci].reciprocal;
        members.sort_by_key(|&(is_updown, axis_recip, straight, is_exit, ci)| {
            // `axis_recip` only applies on sides that host an Up/Down endpoint; elsewhere it is
            // neutralized so the key reduces to the historical `(is_updown, Reverse(straight), ci,
            // is_exit)` — every compass-only side stays byte-identical. On a mixed side the order is
            // axis-reciprocal-compass → straight-through → compass-before-Up/Down → index.
            let axis_key = std::cmp::Reverse(has_updown && axis_recip);
            (!eligible_for_center(is_exit, ci), axis_key, std::cmp::Reverse(straight), is_updown, ci, is_exit)
        });
        // Center goes to the sorted-first endpoint ONLY when it is eligible (a departure or a
        // reciprocal) — the SQ-0216/0222 winner within that group. When NOTHING on this side is
        // eligible (every endpoint is a one-way arrival, the SQ-1274 case), center stays empty and
        // every arrival is numbered from 1 instead of 0 — the whole group shifts by one slot, so
        // none of them ever lands on the anchor cell.
        let front_eligible = members
            .first()
            .is_some_and(|&(_, _, _, is_exit, ci)| eligible_for_center(is_exit, ci));
        let base: u16 = if front_eligible { 0 } else { 1 };
        // When the side has exactly ONE offset endpoint besides an eligible center occupant (the
        // common case, incl. the 217->230 / 5->247 fixes), bias it toward its PARTNER room along
        // this side's tangent axis: partner on the + side -> slot 1 (+offset), − side -> slot 2
        // (−offset), so its perpendicular stub bends toward its partner instead of running across
        // the center connector (SQ-0224 Part B). Both are magnitude 1, always inside the
        // border-cell clamp. This bias only makes sense relative to a REAL center occupant; with no
        // eligible center (`base == 1`) every endpoint is an offset one and falls to the plain
        // sequential branch below instead. With MORE than one offset (or no eligible center), fall
        // back to the original sequential 1,2,3,… interleave (shifted by `base`) — `slot_offset`
        // alternates sign as magnitude grows, keeping cells distinct within the side's clamped
        // capacity; packing several same-sign offsets could exceed the clamp (±1 on a vertical
        // side) and collide.
        let this_room = key.0;
        let single_offset = front_eligible && members.len() == 2;
        // The offset endpoint's (idx 1's) tangent-biased slot, precomputed so a nested front (see
        // below) can read it: `single_offset` implies exactly two members, so idx 1 exists whenever
        // this is `Some`.
        let back_tangent_slot: Option<u16> = single_offset.then(|| {
            let (_, _, _, is_exit1, ci1) = members[1];
            let partner = if is_exit1 { connectors[ci1].dest } else { connectors[ci1].origin };
            let tangent = match (
                graph.room(this_room).and_then(|r| r.pos),
                graph.room(partner).and_then(|r| r.pos),
            ) {
                (Some(a), Some(b)) => match key.1 {
                    Side::Top | Side::Bottom => b.0 - a.0, // tangent axis = x
                    Side::Left | Side::Right => b.1 - a.1, // tangent axis = y
                },
                _ => 0,
            };
            if tangent >= 0 { 1 } else { 2 }
        });
        // SQ-1274: an Up/Down RECIPROCAL (the front's own return path — a plain compass departure
        // still keeps literal center, untouched) sharing a side with exactly one plain one-way
        // COMPASS arrival (the back) is a real, if rare, shape: the arrival can never sit at center
        // (see above), and every OFFSET this side's clamp allows it can still clip the reciprocal's
        // own approach in the shared gutter, whichever offset the arrival takes — center is not a
        // safe harbor here the way it is for two ordinary compass endpoints (measured on the A129
        // fixture's `74->E->25` vs `26<->25` Up/Down pair: NO slot for the arrival alone reaches
        // zero illegal cells against a centered reciprocal, only nesting both away from center,
        // opposite the arrival's own bias, does). So in this one narrow case the reciprocal ALSO
        // takes a small tangent-opposed offset instead of literal center — nesting both out of the
        // middle rather than leaving one parked there for the other's bent approach to sweep past.
        // Every other `single_offset` shape (a departure, or a non-Up/Down reciprocal, sharing a
        // side with one offset endpoint) is unchanged: this reads `back_tangent_slot`, computed
        // identically to before, so the back's own slot never moves.
        let nest_reciprocal_too = single_offset && {
            let (is_updown0, _, _, _, ci0) = members[0];
            let (is_updown1, _, _, is_exit1, ci1) = members[1];
            is_updown0
                && connectors[ci0].reciprocal
                && !is_updown1
                && !is_exit1
                && !connectors[ci1].reciprocal
        };
        for (idx, &(_, _, _, is_exit, ci)) in members.iter().enumerate() {
            let slot = if idx == 0 && front_eligible {
                if nest_reciprocal_too {
                    match back_tangent_slot {
                        Some(1) => 2, // back leans +; nest the reciprocal toward -
                        Some(2) => 1, // back leans -; nest the reciprocal toward +
                        _ => 0,
                    }
                } else {
                    0
                }
            } else if single_offset {
                back_tangent_slot.unwrap_or(1)
            } else {
                base + idx as u16
            };
            if is_exit {
                connectors[ci].exit_slot = slot;
            } else {
                connectors[ci].entry_slot = slot;
            }
        }
    }
}

/// Full logical route: topology + lane assignment.
pub fn route_lanes(graph: &MapGraph) -> RoutePlan {
    let mut connectors = route_topology(graph);
    assign_side_slots(&mut connectors, graph);

    // Flatten every connector's long runs into a global list with stable ids.
    let mut runs: Vec<(usize, Channel, i32, i32)> = Vec::new();
    let mut owner: Vec<(usize, Channel, i32, i32)> = Vec::new(); // (connector idx, channel, s, e)
    for (ci, c) in connectors.iter().enumerate() {
        for (ch, s, e) in long_runs(&c.points) {
            let id = runs.len();
            runs.push((id, ch, s, e));
            owner.push((ci, ch, s, e));
        }
    }
    let (lane_of, counts) = assign_lanes(&runs);

    // Attach lanes back onto each connector.
    for (id, (ci, ch, s, e)) in owner.into_iter().enumerate() {
        let lane = lane_of[&id];
        connectors[ci].segs.push(LaneSeg { channel: ch, lane, start: s, end: e });
    }

    let mut h_lanes = BTreeMap::new();
    let mut v_lanes = BTreeMap::new();
    for (ch, n) in counts {
        match ch {
            Channel::H(r) => { h_lanes.insert(r, n); }
            Channel::V(c) => { v_lanes.insert(c, n); }
        }
    }
    let diag_corners = diagonal_corners(&connectors);
    RoutePlan { connectors, h_lanes, v_lanes, diag_corners }
}

/// Collect the gap-lattice corners every diagonal step passes through, as `(v_channel,
/// h_channel)` index pairs (SQ-0314). See `RoutePlan::diag_corners` for why these must be
/// reported separately from the lane counts.
///
/// Read off the POLYLINES rather than the connectors' directions: a step that changes both
/// coordinates is a diagonal step by construction, whoever built it, so a future route that
/// emits one is covered without having to remember this pass exists.
fn diagonal_corners(connectors: &[RoutedConnector]) -> BTreeSet<(i32, i32)> {
    let mut out = BTreeSet::new();
    for c in connectors {
        for w in c.points.windows(2) {
            let (a, b) = (w[0], w[1]);
            if a.0 == b.0 || a.1 == b.1 {
                continue; // orthogonal step (or none) — the lane machinery already covers it
            }
            // The diagonal's all-odd endpoint IS the corner; the other end is a room centre.
            for p in [a, b] {
                if p.0 % 2 != 0 && p.1 % 2 != 0 {
                    out.insert(((p.0 - 1).div_euclid(2), (p.1 - 1).div_euclid(2)));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use crate::graph::MapGraph;
    use crate::router::side_for;

    /// SQ-1264: a self-loop (`origin == dest`, `MapGraph::add_self_loop`) must never produce a
    /// routed connector — it is drawn only as the room's own `↩` badge (`render/map.rs`), never a
    /// line. Before the `c.origin != c.dest` guards in `select_shared_paths`/`route_topology_with`,
    /// a self-loop's own (room, room) "pair" paired the edge with ITSELF —
    /// its forward and backward buckets were the same index set — and produced a bogus polyline
    /// that looped back around the room's own box, on the real Adventure map SQ-1264 was filed
    /// against ("In Forest" #42746, whose W/N/S exits are all self-loops per `advent.inf`'s
    /// `In_Forest_1` object).
    ///
    /// Reconstructs the SHAPE of that save (`~/.lanthorn/saves/advent.blb.save/default.lanthorn`,
    /// read during the SQ-1264 investigation): a hill room leading south into a forest room whose
    /// own W/N/S all loop back into itself, beside one real edge east to a valley.
    #[test]
    fn a_self_loop_is_never_routed_as_a_connector() {
        let mut g = MapGraph::new();
        const HILL: RoomId = 61289;
        const FOREST: RoomId = 42746;
        const VALLEY: RoomId = 49722;
        g.upsert_room(HILL, "At Hill In Road".into());
        g.upsert_room(FOREST, "In Forest".into());
        g.upsert_room(VALLEY, "In A Valley".into());
        g.set_pos(HILL, (0, 0));
        g.set_pos(FOREST, (0, 1));
        g.set_pos(VALLEY, (1, 1));
        g.add_edge(HILL, Direction::S, FOREST);
        g.add_edge(FOREST, Direction::E, VALLEY);
        assert!(g.add_self_loop(FOREST, Direction::W));
        assert!(g.add_self_loop(FOREST, Direction::N));
        assert!(g.add_self_loop(FOREST, Direction::S));
        assert_eq!(g.self_loops(FOREST), vec![Direction::W, Direction::N, Direction::S]);

        let routed = route_topology(&g);
        assert!(
            routed.iter().all(|c| c.origin != c.dest),
            "no routed connector may have origin == dest: {routed:?}"
        );
        // Falsify: the two REAL edges must still route normally — this is not a test that
        // routing produced nothing at all.
        assert_eq!(routed.len(), 2, "the hill→forest and forest→valley edges still route: {routed:?}");
    }

    #[test]
    fn segments_sharing_a_lane_never_overlap() {
        // Build a small congested graph and assert the core invariant across all channels.
        let mut g = MapGraph::new();
        for id in 1..=4 { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0)); g.set_pos(2, (3, 0)); g.set_pos(3, (0, 1)); g.set_pos(4, (3, 1));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(3, Direction::E, 4); // a second horizontal run in a nearby channel
        g.add_edge(1, Direction::S, 3);
        let plan = route_lanes(&g);
        // Group segs by (channel, lane); within a group, extents must be pairwise disjoint.
        use std::collections::BTreeMap;
        let mut by_lane: BTreeMap<(Channel, u16), Vec<(i32, i32)>> = BTreeMap::new();
        for c in &plan.connectors {
            for s in &c.segs {
                by_lane.entry((s.channel, s.lane)).or_default().push((s.start, s.end));
            }
        }
        for ((ch, lane), mut ivs) in by_lane {
            ivs.sort();
            for w in ivs.windows(2) {
                assert!(w[0].1 < w[1].0,
                    "overlap in {ch:?} lane {lane}: {:?} and {:?}", w[0], w[1]);
            }
        }
    }

    #[test]
    fn assign_lanes_shares_a_lane_only_for_disjoint_runs() {
        // Three runs in ONE channel H(0): two disjoint ([0,2],[4,6]) may share lane 0;
        // one overlapping both ([1,5]) must take a new lane. Directly exercises the
        // core invariant (a graph-level test can't force a shared lane reliably).
        let ch = Channel::H(0);
        let runs = vec![(0usize, ch, 0, 2), (1usize, ch, 4, 6), (2usize, ch, 1, 5)];
        let (lane_of, counts) = assign_lanes(&runs);
        assert_eq!(lane_of[&0], 0, "first run → lane 0");
        assert_eq!(lane_of[&1], 0, "disjoint run shares lane 0");
        assert_eq!(lane_of[&2], 1, "overlapping run gets a new lane");
        assert_eq!(counts[&ch], 2, "channel needs 2 lanes");
        // Invariant: any two runs sharing a lane have disjoint extents.
        let mut by_lane: std::collections::BTreeMap<u16, Vec<(i32, i32)>> =
            std::collections::BTreeMap::new();
        for &(id, _, s, e) in &runs {
            by_lane.entry(lane_of[&id]).or_default().push((s, e));
        }
        for (_lane, mut ivs) in by_lane {
            ivs.sort();
            for w in ivs.windows(2) {
                assert!(w[0].1 < w[1].0, "same-lane runs must be disjoint: {:?} {:?}", w[0], w[1]);
            }
        }
    }

    #[test]
    fn two_overlapping_runs_get_two_lanes() {
        // Two connectors whose horizontal runs share a channel and overlap in extent must
        // land on different lanes → that channel reports lane_count 2.
        let mut g = MapGraph::new();
        for id in 1..=5 { g.upsert_room(id, "r".into()); }
        // 1->2 and 3->4 both run horizontally across the same row band, overlapping in x.
        // Room 5 fills the one cell between 3 and 4 so NEITHER pair can take the direct
        // straight-shot (empty-gap) route — both must dip into the H channel and share it.
        g.set_pos(1, (0, 0)); g.set_pos(2, (4, 0));
        g.set_pos(3, (1, 0)); g.set_pos(4, (3, 0));
        g.set_pos(5, (2, 0));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(3, Direction::E, 4);
        let plan = route_lanes(&g);
        let max_h = plan.h_lanes.values().copied().max().unwrap_or(0);
        assert!(max_h >= 2, "two overlapping horizontal runs need ≥2 lanes; got {max_h}");
    }

    #[test]
    fn polylines_overlap_sees_room_line_stomp() {
        // Full routes (room-centre endpoints first/last) both running up room column x=-14 into the
        // room at (-14,-8): their INTERIORS overlap on the column for many cells.
        let a = vec![(-2, 6), (-3, 6), (-14, 6), (-14, -7), (-14, -8)];
        let b = vec![(-10, 2), (-11, 2), (-14, 2), (-14, -7), (-14, -8)];
        assert!(polylines_overlap(&a, &b), "two routes up room column -14 overlap on the interior");
        // The second route reaches the room one channel over (x=-13) and only joins the column for the
        // final doorway stub — interiors share no line, so it is NOT a stomp.
        let c = vec![(-10, 2), (-13, 2), (-13, -7), (-14, -7), (-14, -8)];
        assert!(!polylines_overlap(&a, &c), "sharing only the doorway stub is not a stomp");
        // Different column entirely: no shared line.
        let d = vec![(-10, 2), (-11, 2), (-12, 2), (-12, -7), (-12, -8)];
        assert!(!polylines_overlap(&a, &d), "different columns do not overlap");
    }

    #[test]
    fn two_direct_routes_to_one_room_do_not_stomp() {
        // A and B both lie east of X and would each direct-route west then up X's column, stomping.
        // The router must reject the second straight route and dip it into a lane instead, so no two
        // connectors share a parallel room-line run.
        let mut g = MapGraph::new();
        for id in 1..=3 {
            g.upsert_room(id, "r".into());
        }
        g.set_pos(1, (0, 0)); // X
        g.set_pos(2, (5, 3)); // A
        g.set_pos(3, (5, 1)); // B
        g.add_edge(2, Direction::W, 1);
        g.add_edge(3, Direction::W, 1);
        let plan = route_lanes(&g);
        for i in 0..plan.connectors.len() {
            for j in (i + 1)..plan.connectors.len() {
                assert!(
                    !polylines_overlap(&plan.connectors[i].points, &plan.connectors[j].points),
                    "connectors {i} and {j} stomp on a shared line: {:?} {:?}",
                    plan.connectors[i].points, plan.connectors[j].points
                );
            }
        }
    }

    #[test]
    fn longer_direct_route_keeps_the_contested_line() {
        // A (far) and B (near) both enter T from the south up T's column and cannot both run straight
        // up it. The LONGER connector (A) must keep the direct line; the shorter (B) weaves. Without
        // length priority the outcome is edge-order dependent and the long path can be the one to weave.
        let mut g = MapGraph::new();
        for id in 1..=3 { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0)); // T
        g.set_pos(2, (3, 4)); // A (far)
        g.set_pos(3, (1, 2)); // B (near)
        g.add_edge(2, Direction::W, 1);
        g.add_edge(3, Direction::W, 1);
        let routed = route_topology(&g);
        // T's column is the contested room line (doubled x=0). The winner runs straight up it; the
        // loser yields it and routes up an adjacent channel instead.
        let runs = |o: RoomId, d: RoomId| {
            let c = routed.iter().find(|c| c.origin == o && c.dest == d).unwrap();
            all_runs(if c.points.len() > 2 { &c.points[1..c.points.len() - 1] } else { &[] })
        };
        let on_column_0 = |rs: &[(bool, i32, i32, i32)]| rs.iter().any(|&(h, l, _, _)| !h && l == 0);
        assert!(on_column_0(&runs(2, 1)), "A (longer) keeps the straight run up T's column");
        assert!(!on_column_0(&runs(3, 1)), "B (shorter) yields the column, routing alongside it");
    }

    #[test]
    fn route_lanes_is_deterministic() {
        let mut g = MapGraph::new();
        for id in 1..=3 { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0)); g.set_pos(2, (2, 0)); g.set_pos(3, (0, 2));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(1, Direction::S, 3);
        let a = format!("{:?}", route_lanes(&g));
        let b = format!("{:?}", route_lanes(&g));
        assert_eq!(a, b);
    }

    #[test]
    fn reciprocal_pairing_picks_the_geometric_opposite_added_last() {
        // Adjacent #77/#239 with a reciprocal E/W edge plus extra N and S edges, where the geometric
        // opposite (W) of the forward E edge is added LAST. The pairing must pick E with W (not the
        // first reverse edge N), so the retained line stays straight. Pairing with N would bend it
        // up-and-over — the "missing southern path" bug. Since SQ-0522 the extras collapse to icons
        // rather than merge stubs, but which edge holds the line is the same guarantee.
        let mut g = MapGraph::new();
        g.upsert_room(77, "f".into());
        g.upsert_room(239, "g".into());
        g.set_pos(77, (0, 0));
        g.set_pos(239, (1, 0)); // adjacent
        g.add_edge(77, Direction::E, 239);
        g.add_edge(239, Direction::N, 77);
        g.add_edge(239, Direction::S, 77);
        g.add_edge(239, Direction::W, 77); // opposite of E, added last on purpose
        let plan = route_lanes(&g);
        assert_eq!(plan.connectors.len(), 1, "one line for the pair");
        let trunk = &plan.connectors[0];
        assert!(!trunk.merge, "it is a real connector, not a stub");
        assert_eq!(trunk.entry_dir, Some(Direction::W), "W held the line, not the first reverse edge N");
        assert!(is_collinear(&trunk.points), "so the line stays straight E/W: {:?}", trunk.points);
        let mut secs = trunk.secondary_entry.clone();
        secs.sort_by_key(|d| format!("{d:?}"));
        assert_eq!(secs, vec![Direction::N, Direction::S], "the extras collapse to icons at #239");
    }

    #[test]
    fn same_row_empty_gap_routes_straight() {
        // Two rooms on one row with an empty cell between them: the connector runs STRAIGHT
        // along the room row (no channel dip) — its polyline is collinear and carries no lane.
        let mut g = MapGraph::new();
        for id in 1..=2 { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0)); g.set_pos(2, (2, 0)); // gap at (1,0) is empty
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        let plan = route_lanes(&g);
        assert_eq!(plan.connectors.len(), 1, "reciprocal pair collapses to one connector");
        let conn = &plan.connectors[0];
        assert!(is_collinear(&conn.points), "straight shot must be collinear: {:?}", conn.points);
        assert!(conn.segs.is_empty(), "straight shot carries no lane segments");
    }

    #[test]
    fn same_row_occupied_gap_dips() {
        // Same two rooms, but a third room fills the gap: the connector cannot run straight
        // through an occupied cell, so it dips into a channel — its polyline turns.
        let mut g = MapGraph::new();
        for id in 1..=3 { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0)); g.set_pos(2, (2, 0)); g.set_pos(3, (1, 0)); // gap occupied
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        let plan = route_lanes(&g);
        let conn = plan.connectors.iter().find(|c| c.origin == 1 && c.dest == 2).unwrap();
        assert!(!is_collinear(&conn.points), "occupied gap must dip into a channel: {:?}", conn.points);
    }

    #[test]
    fn direct_route_tie_prefers_straight_over_bent_regardless_of_order() {
        // Three rooms sharing one contested empty cell (SQ-1255): #1 (analogue of Canyon
        // View's #230) sits between #2 due north and #3 to the northeast. `1 N 2` wants the
        // straight vertical room-line; `3 W 1` wants an L that turns onto that same column
        // one row above #1, so the two direct routes' polylines overlap by more than the
        // shared arrival doorway. Both are length 6 in doubled coords, so before the
        // straightness tiebreak this was a pure insertion-index tie — whichever edge was
        // added first kept the straight line, even when it was the BENT one. The straight
        // `1 N 2` route must win regardless of which edge is added first.
        let build = |straight_first: bool| -> MapGraph {
            let mut g = MapGraph::new();
            for id in 1..=3 { g.upsert_room(id, "r".into()); }
            g.set_pos(1, (0, 0));
            g.set_pos(2, (0, -3));
            g.set_pos(3, (2, -1));
            if straight_first {
                g.add_edge(1, Direction::N, 2);
                g.add_edge(3, Direction::W, 1);
            } else {
                g.add_edge(3, Direction::W, 1);
                g.add_edge(1, Direction::N, 2);
            }
            g
        };
        for straight_first in [true, false] {
            let g = build(straight_first);
            let plan = route_lanes(&g);
            let straight = plan.connectors.iter().find(|c| c.origin == 1 && c.dest == 2).unwrap();
            assert!(
                is_collinear(&straight.points),
                "straight_first={straight_first}: #1 N #2 must keep the straight room-line: {:?}",
                straight.points
            );
        }
    }

    /// Count direction changes (turns) in a doubled-coord polyline.
    fn turn_count(pts: &[(i32, i32)]) -> usize {
        pts.windows(3)
            .filter(|w| {
                let d1 = ((w[1].0 - w[0].0).signum(), (w[1].1 - w[0].1).signum());
                let d2 = ((w[2].0 - w[1].0).signum(), (w[2].1 - w[1].1).signum());
                d1 != d2
            })
            .count()
    }

    #[test]
    fn oneway_entry_side_follows_the_compass_rule() {
        use Direction::*;
        // Cardinals enter the side opposite the travel direction (straight through).
        assert_eq!(oneway_entry_side(N), Some(Side::Bottom));
        assert_eq!(oneway_entry_side(S), Some(Side::Top));
        assert_eq!(oneway_entry_side(E), Some(Side::Left));
        assert_eq!(oneway_entry_side(W), Some(Side::Right));
        // Diagonals arrive on the corner FACING the origin, so the rule mirrors the exit rule:
        // `side_for(opposite(dir))` (SQ-0314). The old rotation onto a cardinal side (NW->N,
        // NE->E, SE->S, SW->W) would wrap a corner-departing diagonal around the destination.
        assert_eq!(oneway_entry_side(NE), Some(Side::Left));
        assert_eq!(oneway_entry_side(SW), Some(Side::Right));
        assert_eq!(oneway_entry_side(NW), Some(Side::Right));
        assert_eq!(oneway_entry_side(SE), Some(Side::Left));
        for d in [NE, NW, SE, SW] {
            assert_eq!(
                oneway_entry_side(d),
                side_for(crate::direction::opposite(d)),
                "{d:?}: a one-way diagonal's arrival side is its exit rule, reversed",
            );
        }
        // Up/Down enter opposite the side they departed, same as a cardinal.
        assert_eq!(oneway_entry_side(Up), Some(Side::Bottom));
        assert_eq!(oneway_entry_side(Down), Some(Side::Top));
        // The remaining non-planar directions have no side.
        assert_eq!(oneway_entry_side(In), None);
        assert_eq!(oneway_entry_side(Out), None);
        assert_eq!(oneway_entry_side(Unknown), None);
    }

    #[test]
    fn oneway_cardinal_enters_the_matching_side() {
        // A one-way S edge into an aligned room enters from the north (Top); an E edge from the west.
        let mut g = MapGraph::new();
        for id in 1..=3 { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 3)); // due south of 1
        g.set_pos(3, (3, 0)); // due east of 1
        g.add_edge(1, Direction::S, 2);
        g.add_edge(1, Direction::E, 3);
        let plan = route_lanes(&g);
        let entry = |o: RoomId, d: RoomId| plan.connectors.iter().find(|c| c.origin == o && c.dest == d).unwrap().entry;
        assert_eq!(entry(1, 2), Side::Top, "S edge enters destination's north side");
        assert_eq!(entry(1, 3), Side::Left, "E edge enters destination's west side");
    }

    #[test]
    fn diagonal_clear_path_routes_single_corner() {
        // Room 2 is east AND south of room 1 with all intervening cells empty: the connector
        // turns exactly ONCE (a clean L straight along the room grid), not weaving through
        // channels with extra dips.
        let mut g = MapGraph::new();
        for id in 1..=2 { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0)); g.set_pos(2, (3, 2)); // SE of 1, clear path
        g.add_edge(1, Direction::E, 2); // one-way; exits Right, then turns south to 2
        let plan = route_lanes(&g);
        let conn = &plan.connectors[0];
        assert_eq!(turn_count(&conn.points), 1, "clear diagonal path = single corner: {:?}", conn.points);
    }

    #[test]
    fn diagonal_blocked_path_dips_more() {
        // Same diagonal, but a room blocks the clean L's corner row: the connector cannot take
        // the single-corner direct route and must weave (more than one turn).
        let mut g = MapGraph::new();
        for id in 1..=3 { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0)); g.set_pos(2, (3, 2));
        g.set_pos(3, (2, 0)); // sits on the direct L's first leg (row 0)
        g.add_edge(1, Direction::E, 2);
        let plan = route_lanes(&g);
        let conn = plan.connectors.iter().find(|c| c.origin == 1 && c.dest == 2).unwrap();
        assert!(turn_count(&conn.points) >= 2, "blocked path must weave: {:?}", conn.points);
    }

    /// Expand a doubled-coord polyline into the doubled cells it passes through.
    fn trace_cells(points: &[(i32, i32)]) -> Vec<(i32, i32)> {
        let mut out = Vec::new();
        for w in points.windows(2) {
            let (a, b) = (w[0], w[1]);
            let dx = (b.0 - a.0).signum();
            let dy = (b.1 - a.1).signum();
            let mut cur = a;
            loop {
                if out.last() != Some(&cur) { out.push(cur); }
                if cur == b { break; }
                cur = (cur.0 + dx, cur.1 + dy);
            }
        }
        out
    }

    fn doubled(cell: (i32, i32)) -> (i32, i32) { (cell.0 * 2, cell.1 * 2) }

    #[test]
    fn doubled_and_exit_points() {
        assert_eq!(cell_to_doubled((0, 0)), (0, 0));
        assert_eq!(cell_to_doubled((-1, 2)), (-2, 4));
        assert_eq!(exit_point((0, 0), Side::Right), (1, 0));
        assert_eq!(exit_point((0, 0), Side::Top), (0, -1));
        assert_eq!(exit_point((-1, 2), Side::Left), (-3, 4));
    }

    #[test]
    fn every_long_run_is_on_the_gap_lattice() {
        // Two rooms, A(0,0) -E-> B(2,0): the connector's long runs must lie on odd
        // coordinates (channels), so they never touch a room cell (even,even).
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (2, 0));
        g.add_edge(1, Direction::E, 2);
        let conns = route_topology(&g);
        assert_eq!(conns.len(), 1);
        let c = &conns[0];
        assert_eq!((c.origin, c.dest), (1, 2));
        // points form an orthogonal polyline (each step changes exactly one axis).
        for w in c.points.windows(2) {
            let same_x = w[0].0 == w[1].0;
            let same_y = w[0].1 == w[1].1;
            assert!(same_x ^ same_y, "polyline must be orthogonal; got {:?}->{:?}", w[0], w[1]);
        }
        // The interior run cells (excluding the two room centres at the ends) never land
        // on a room centre (even,even pair that equals a placed room's doubled cell).
        let room_cells: Vec<(i32, i32)> = vec![doubled((0, 0)), doubled((2, 0))];
        let cells = trace_cells(&c.points);
        for (i, &p) in cells.iter().enumerate() {
            let is_endpoint = i == 0 || i == cells.len() - 1;
            if !is_endpoint {
                assert!(!room_cells.contains(&p), "run passes through room cell {p:?}");
            }
        }
    }

    #[test]
    fn adjacent_facing_rooms_route_straight_no_dip() {
        // A(0,0) -E-> B(1,0): the two boxes are adjacent and facing, so the exit stub and
        // entry stub are the SAME shared-doorway cell. The route must go straight through it
        // (ca → ea → cb) with NO lattice dip — no out-and-back tail, and no channel run/lane.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        let plan = route_lanes(&g);
        assert_eq!(plan.connectors.len(), 1);
        let c = &plan.connectors[0];
        assert_eq!(c.points, vec![(0, 0), (1, 0), (2, 0)], "straight ca→ea→cb, no dip");
        assert!(c.segs.is_empty(), "an adjacent straight route needs no laned run; got {:?}", c.segs);
        // No cell is visited twice (no out-and-back).
        let cells = trace_cells(&c.points);
        let unique: std::collections::BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), unique.len(), "route must not revisit any cell");
    }

    #[test]
    fn degenerate_l_is_merged_to_single_run() {
        // A connector whose L has a zero-length leg leaves three collinear points; merge_collinear
        // collapses them so a straight channel run is ONE segment (not two on possibly different
        // lanes — the cause of a line rendering as a diagonal).
        let mut pts = vec![(0, 0), (1, 0), (1, 1), (1, 2), (1, 3), (2, 3)];
        merge_collinear(&mut pts);
        assert_eq!(pts, vec![(0, 0), (1, 0), (1, 3), (2, 3)], "collinear midpoints removed");
    }

    #[test]
    fn same_side_endpoints_get_distinct_slots() {
        // Room 1 has TWO endpoints on its Right side: a departure (1 -E-> 2) and an arrival
        // (3 -W-> 1, entering 1 from the right). They must receive distinct slot indices so
        // the renderer can offset their anchors onto separate cells along that side.
        let mut g = MapGraph::new();
        for id in [1, 2, 3] { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (2, 0));
        g.set_pos(3, (4, 0)); // east of 1, beyond 2: its direct west path to 1 is blocked by 2
        g.add_edge(1, Direction::E, 2); // exits room 1 on its Right
        g.add_edge(3, Direction::W, 1); // blocked by room 2 → dips, entering room 1 from the right
        let plan = route_lanes(&g);
        let mut slots = Vec::new();
        for c in &plan.connectors {
            if c.origin == 1 && c.exit == Side::Right { slots.push(c.exit_slot); }
            if c.dest == 1 && c.entry == Side::Right { slots.push(c.entry_slot); }
        }
        assert_eq!(slots.len(), 2, "two endpoints on room 1's Right side; got {slots:?}");
        assert_ne!(slots[0], slots[1], "same-side endpoints must get distinct slots");
    }

    #[test]
    fn reciprocal_pair_collapses_to_one_connector() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1); // reciprocal
        assert_eq!(route_topology(&g).len(), 1, "reciprocal pair → one connector");
    }

    /// A bidirectional pair joined by NON-opposite directions collapses to ONE connector,
    /// each end's arrow on its own compass side: `1→N→2` + `2→E→1` → one reciprocal
    /// connector exiting room 1 on its north side and entering room 2 on its east side.
    #[test]
    fn non_opposite_bidirectional_pair_collapses() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -2));
        g.add_edge(1, Direction::N, 2); // 1→N→2
        g.add_edge(2, Direction::E, 1); // 2→E→1  (bidirectional, non-opposite)
        let conns = route_topology(&g);
        assert_eq!(conns.len(), 1, "bidirectional pair collapses to one connector; got {}", conns.len());
        assert!(conns[0].reciprocal, "collapsed bidirectional connector must be reciprocal");
        assert_eq!(conns[0].exit, side_for(Direction::N).unwrap(), "exits room 1 on its north side");
        assert_eq!(conns[0].entry, side_for(Direction::E).unwrap(), "enters room 2 on its east side");
    }

    /// Regression: a true opposite pair `1→N→2` + `2→S→1` collapses to exactly ONE
    /// connector and that connector must have `reciprocal == true`.
    #[test]
    fn true_opposite_pair_is_reciprocal() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -2));
        g.add_edge(1, Direction::N, 2); // 1→N→2
        g.add_edge(2, Direction::S, 1); // 2→S→1  (true opposite: S == opposite(N))
        let conns = route_topology(&g);
        assert_eq!(conns.len(), 1, "true opposite pair collapses to one connector");
        assert!(conns[0].reciprocal, "collapsed true-opposite connector must have reciprocal == true");
    }

    /// Regression (A129 #239↔#77): three edges between one pair — `1→E→2`, `2→N→1`, `2→S→1`.
    /// One pairing holds the line; the third edge must not be silently DROPPED. Before SQ-0522 it
    /// was drawn as a second connector; now it is recorded as a stacked icon on the one line. What
    /// this test guards either way is that the passage does not vanish because its room pair had
    /// already been seen.
    #[test]
    fn third_edge_between_pair_becomes_an_icon_not_a_dropped_edge() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -2));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::N, 1);
        g.add_edge(2, Direction::S, 1);
        let conns = route_topology(&g);
        assert_eq!(conns.len(), 1, "one line for the pair; got {}", conns.len());
        assert!(conns[0].reciprocal, "the retained pairing is bidirectional");
        // Whichever of N/S held the line, the other must be recorded at room 2's end.
        let held = conns[0].entry_dir.unwrap_or(conns[0].exit_dir);
        let other = if held == Direction::N { Direction::S } else { Direction::N };
        let at_two =
            if conns[0].origin == 2 { &conns[0].secondary_exit } else { &conns[0].secondary_entry };
        assert!(at_two.contains(&other), "the third edge ({other:?}) survives as an icon: {at_two:?}");
    }

    #[test]
    fn orientation_choice_reduces_crossings() {
        // Connector A is a short horizontal run H(0) on y=1 spanning x∈[1,3].
        let a = build_points((0, 0), Side::Right, (2, 0), Side::Left, None, None);
        // Connector B goes from (4,-2) to (0,2). The two L orientations route differently:
        //  - HORIZONTAL-FIRST drops its vertical leg at x=1, INSIDE A's x-span → crosses A.
        //  - VERTICAL-FIRST drops its vertical leg at x=7, OUTSIDE A's x-span → no crossing.
        let hf = build_points_orient(
            (4, -2), Side::Left, (0, 2), Side::Top, Orient::HorizontalFirst, None, None,
        );
        let vf = build_points_orient(
            (4, -2), Side::Left, (0, 2), Side::Top, Orient::VerticalFirst, None, None,
        );
        assert_eq!(count_crossings(&a, &hf), 1, "horizontal-first must cross A once");
        assert_eq!(count_crossings(&a, &vf), 0, "vertical-first must avoid A");
        // The greedy chooser, presented both orientations against the already-placed A, must
        // pick the non-crossing (vertical-first) route.
        let pick_hf = count_crossings(&a, &hf);
        let pick_vf = count_crossings(&a, &vf);
        assert!(pick_vf < pick_hf, "greedy should prefer the lower-crossing orientation");
    }

    #[test]
    fn greedy_picks_non_crossing_route_end_to_end() {
        // Two connectors whose default (horizontal-first) routes cross, but where rerouting
        // the second connector vertical-first removes the crossing cleanly. `route_topology`
        // must choose the lower-crossing, overlap-free route set.
        let mut g = MapGraph::new();
        for id in [1, 2, 3, 4, 5] { g.upsert_room(id, "r".into()); }
        // A: 1 -E-> 2, a short horizontal pair. Room 5 fills the cell between them so A must
        // DIP into a channel (no direct straight-shot), keeping its run crossable by B.
        g.set_pos(1, (0, 0));
        g.set_pos(2, (2, 0));
        g.set_pos(5, (1, 0));
        g.add_edge(1, Direction::E, 2);
        // B: 3 -W-> 4 from the upper-right down to the left, off-axis so its L can flip.
        g.set_pos(3, (4, -2));
        g.set_pos(4, (0, 2));
        g.add_edge(3, Direction::W, 4);
        let crossings = |conns: &[RoutedConnector]| {
            let mut n = 0;
            for i in 0..conns.len() { for j in (i + 1)..conns.len() {
                n += count_crossings(&conns[i].points, &conns[j].points);
            } }
            n
        };
        // The canonical (non-greedy) routing DOES cross here…
        let default = route_topology_with(&g, false);
        assert!(crossings(&default) > 0, "this graph's default routing must cross (test is meaningful)");
        // …and `route_topology` (greedy + best-of) must pick the crossing-free route set.
        let chosen = route_topology(&g);
        assert_eq!(crossings(&chosen), 0, "route_topology must pick the crossing-free route set");
    }

    #[test]
    fn greedy_routing_is_deterministic() {
        // The same graph that triggers a greedy reroute must produce an identical RoutePlan
        // every time (fixed candidate order, integer tiebreaks, no RNG).
        let mut g = MapGraph::new();
        for id in [1, 2, 3, 4] { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (2, 0));
        g.set_pos(3, (4, -2));
        g.set_pos(4, (0, 2));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(3, Direction::W, 4);
        let a = format!("{:?}", route_lanes(&g));
        let b = format!("{:?}", route_lanes(&g));
        assert_eq!(a, b, "greedy routing must be deterministic");
    }

    #[test]
    fn extra_same_pair_edges_become_stacked_icons() {
        // 77↔239 E/W reciprocal line, plus 239→N→77 and 239→S→77 (the A129 tangle). These used to
        // draw as two merge stubs joining the trunk at T-junctions; SQ-0522 collapses them.
        let mut g = MapGraph::new();
        g.upsert_room(77, "a".into());
        g.upsert_room(239, "b".into());
        g.set_pos(77, (0, 0));
        g.set_pos(239, (2, 0)); // east of 77, a gap of one cell
        g.add_edge(77, Direction::E, 239);
        g.add_edge(239, Direction::W, 77); // reciprocal → holds the line
        g.add_edge(239, Direction::N, 77);
        g.add_edge(239, Direction::S, 77);
        let conns = route_topology(&g);
        assert_eq!(conns.len(), 1, "one line for the pair; got {}", conns.len());
        assert!(!conns[0].merge, "and no merge stubs are left to join it");
        let at_239 =
            if conns[0].origin == 239 { &conns[0].secondary_exit } else { &conns[0].secondary_entry };
        let mut secs = at_239.clone();
        secs.sort_by_key(|d| format!("{d:?}"));
        assert_eq!(secs, vec![Direction::N, Direction::S], "both extras ride #239's end as icons");
    }

    #[test]
    fn stub_excluded_non_compass() {
        // Up/Down are now routed as lane connectors (see `updown_is_routed_as_a_lane_connector_not_collapsed`
        // below); only the truly non-planar directions (In/Out/Unknown) remain excluded stubs.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::In, 2); // non-planar stub
        assert!(route_topology(&g).is_empty(), "non-planar edges are not routed");
    }

    #[test]
    fn connector_carries_diagonal_exit_dir() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 0));
        g.set_pos(2, (0, 1)); // SW of room 1
        g.add_edge(1, Direction::SW, 2);
        let conns = route_topology(&g);
        let c = conns.iter().find(|c| c.origin == 1 && c.dest == 2).unwrap();
        assert_eq!(c.exit_dir, Direction::SW);
        assert_eq!(c.entry_dir, None, "one-way edge has no back-edge dir");
    }

    #[test]
    fn reciprocal_diagonal_carries_both_dirs() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 0));
        g.set_pos(2, (0, 1));
        g.add_edge(1, Direction::SW, 2);
        g.add_edge(2, Direction::NE, 1); // true reciprocal
        let conns = route_topology(&g);
        assert_eq!(conns.len(), 1, "reciprocal diagonal collapses to one connector");
        let c = &conns[0];
        assert!(c.reciprocal);
        let dirs = [c.exit_dir, c.entry_dir.expect("reciprocal carries entry_dir")];
        assert!(
            dirs.contains(&Direction::SW) && dirs.contains(&Direction::NE),
            "both diagonal directions carried: {dirs:?}"
        );
    }

    #[test]
    fn updown_is_routed_as_a_lane_connector_not_collapsed() {
        // A--Up-->B (B north of A). Up should now be a lane connector, not just a stub.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::Up, 2);

        let plan = route_lanes(&g);
        let up_connectors: Vec<_> = plan
            .connectors
            .iter()
            .filter(|c| matches!(c.exit_dir, Direction::Up))
            .collect();
        assert_eq!(up_connectors.len(), 1, "the Up edge produces one lane connector");
        assert!(!up_connectors[0].reciprocal, "an unmatched one-way Up (no reverse Down) is not reciprocal");
    }

    #[test]
    fn reciprocal_updown_pair_collapses_to_one() {
        // A--Up-->B and B--Down-->A: after visual testing (SQ-0216) this REVISES Task 6's
        // "never collapse" decision — a matching Up/Down pair now collapses to ONE
        // reciprocal connector, same as a compass reciprocal pair.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(2, Direction::Down, 1);

        let plan = route_lanes(&g);
        let vertical: Vec<_> = plan
            .connectors
            .iter()
            .filter(|c| matches!(c.exit_dir, Direction::Up | Direction::Down))
            .collect();
        assert_eq!(vertical.len(), 1, "a matching Up/Down pair collapses to one connector");
        assert!(vertical[0].reciprocal, "the collapsed connector is reciprocal");
        assert_eq!(vertical[0].entry_dir, Some(Direction::Down), "carries the back-edge's direction");
        // Regression guard: for an AXIS-ALIGNED pair (B due north of A), the connector must
        // still exit A's Top and enter B's Bottom — the axis-aligned case already worked
        // before SQ-0216's diagonal fix and must keep working after it.
        assert_eq!(vertical[0].exit, Side::Top, "exits A's north border");
        assert_eq!(vertical[0].entry, Side::Bottom, "enters B's south border");
    }

    #[test]
    fn reciprocal_updown_diagonal_enters_on_north_south_border() {
        // SQ-0216 bug repro: a reciprocal Up/Down pair placed DIAGONALLY (room 2 is
        // up-and-left of room 1, not sharing a row or column) must still enter the
        // destination on a north/south border (Top/Bottom) — never a geometric Left/Right
        // side picked by the diagonal cell offset. Before the fix, the reciprocal-collapse
        // branch computed the forced entry side via `side_for(bk.dir)`, which returns `None`
        // for Up/Down back-edges, so the code fell through to the generic geometric
        // `entry_side`, which picked Left/Right for a diagonal offset.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 1));
        g.set_pos(2, (0, 0));
        g.add_edge(2, Direction::Down, 1);
        g.add_edge(1, Direction::Up, 2);

        let plan = route_lanes(&g);
        let vertical: Vec<_> = plan
            .connectors
            .iter()
            .filter(|c| matches!(c.exit_dir, Direction::Up | Direction::Down))
            .collect();
        assert_eq!(vertical.len(), 1, "the diagonal Up/Down pair still collapses to one connector");
        let c = vertical[0];
        assert!(
            matches!(c.exit, Side::Top | Side::Bottom),
            "exit side must be a north/south border, got {:?}", c.exit
        );
        assert!(
            matches!(c.entry, Side::Top | Side::Bottom),
            "entry side must be a north/south border, not a geometric Left/Right pick, got {:?}", c.entry
        );
    }

    #[test]
    fn unmatched_oneway_updown_stays_non_reciprocal() {
        // A--Up-->B with no matching Down back-edge: stays one connector, non-reciprocal.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::Up, 2);

        let plan = route_lanes(&g);
        let vertical: Vec<_> = plan
            .connectors
            .iter()
            .filter(|c| matches!(c.exit_dir, Direction::Up | Direction::Down))
            .collect();
        assert_eq!(vertical.len(), 1);
        assert!(!vertical[0].reciprocal, "an unmatched one-way up/down stays non-reciprocal");
    }

    /// A--Up-->B and B--S-->A: B--S-->A is geometrically the "reverse" edge between the same two
    /// rooms, but up/down must NEVER PAIR (collapse into a reciprocal) with a compass edge — it
    /// pairs only with the exact opposite up/down direction. The Up edge is listed FIRST to
    /// exercise that ordering. Since the two cannot pair, SQ-0522 keeps the higher-priority edge
    /// alone: S (priority 1) beats Up (priority 8), so one one-way line is drawn.
    #[test]
    fn a_staircase_never_pairs_with_a_compass_edge() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(2, Direction::S, 1);

        let plan = route_lanes(&g);
        assert_eq!(plan.connectors.len(), 1, "one line for the pair");
        let c = &plan.connectors[0];
        assert_eq!(c.exit_dir, Direction::S, "S outranks Up, and the two could never pair anyway");
        assert_eq!(c.entry_dir, None, "it did NOT pair with the Up edge");
        assert!(!c.merge, "and stays a full connector, never a merge stub of an up/down trunk");
        assert_eq!(c.secondary_entry, vec![Direction::Up], "Up recorded at A, its own end");
    }
    #[test]
    fn ns_reciprocal_outranks_updown_for_center_slot() {
        // Room 1 has BOTH a reciprocal N/S connector (to room 2, bent — off-column) and a
        // reciprocal Up/Down connector (to room 3, straight — due north) sharing its Top
        // side. Even though the Up/Down connector is the STRAIGHT one (which would otherwise
        // win the center slot on its own), the N/S reciprocal must still take the center slot
        // (0); the Up/Down reciprocal yields to a fanned slot.
        let mut g = MapGraph::new();
        for id in [1, 2, 3] { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (2, -2)); // north-east: bent reciprocal N/S pair
        g.set_pos(3, (0, -2)); // due north: straight reciprocal Up/Down pair
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1);
        g.add_edge(1, Direction::Up, 3);
        g.add_edge(3, Direction::Down, 1);

        let plan = route_lanes(&g);
        let top: Vec<_> = plan
            .connectors
            .iter()
            .filter(|c| c.origin == 1 && c.exit == Side::Top)
            .collect();
        assert_eq!(top.len(), 2, "expected both connectors sharing room 1's Top side: {top:?}");
        let ns = top
            .iter()
            .find(|c| matches!(c.exit_dir, Direction::N | Direction::S))
            .expect("N/S connector on room 1's Top side");
        let updown = top
            .iter()
            .find(|c| matches!(c.exit_dir, Direction::Up | Direction::Down))
            .expect("up/down connector on room 1's Top side");
        assert_eq!(ns.exit_slot, 0, "reciprocal N/S takes the center slot");
        assert_ne!(updown.exit_slot, 0, "up/down yields to a fanned slot");
    }

    #[test]
    fn straight_updown_keeps_center_over_weaving_oneway_compass() {
        // Room 1 has a STRAIGHT Up/Down reciprocal to room 2 (due north) and a WEAVING
        // one-way compass edge N to room 3 (north-east; reverse edge SW, so bidirectional but
        // NOT a cardinal N/S reciprocal). Both exits land on room 1's Top side. Unlike a
        // reciprocal N/S (which keeps the center slot, see `ns_reciprocal_outranks_updown…`),
        // a non-axis-reciprocal weaving compass connector must YIELD the center to the straight
        // Up/Down line so the straight line does not jog across it (SQ-0222).
        let mut g = MapGraph::new();
        for id in [1, 2, 3] { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1)); // due north: straight reciprocal Up/Down pair
        g.set_pos(3, (1, -1)); // north-east: weaving one-way compass (N out, SW back)
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(2, Direction::Down, 1);
        g.add_edge(1, Direction::N, 3);
        g.add_edge(3, Direction::SW, 1);

        let plan = route_lanes(&g);
        let top: Vec<_> = plan
            .connectors
            .iter()
            .filter(|c| c.origin == 1 && c.exit == Side::Top)
            .collect();
        assert_eq!(top.len(), 2, "expected both connectors sharing room 1's Top side: {top:?}");
        let updown = top
            .iter()
            .find(|c| matches!(c.exit_dir, Direction::Up | Direction::Down))
            .expect("up/down connector on room 1's Top side");
        let compass = top
            .iter()
            .find(|c| matches!(c.exit_dir, Direction::N | Direction::S))
            .expect("compass connector on room 1's Top side");
        assert_eq!(updown.exit_slot, 0, "the straight up/down keeps the center slot");
        assert_ne!(compass.exit_slot, 0, "the weaving one-way compass yields to a fanned slot");
    }

    #[test]
    fn single_offset_slot_fans_toward_the_partner_room() {
        // A room whose center slot is taken by a straight N reciprocal (to room 2, due north),
        // plus exactly ONE offset connector on its Top side. The single offset fans toward its
        // partner: a WEST partner takes a − (even) slot, an EAST partner a + (odd) slot — so the
        // offset stub bends toward its partner instead of across the center line (SQ-0224 Part B).
        // Both are magnitude-1 cells, always clamp-safe. (With more than one offset the assignment
        // falls back to the original sequential interleave, so partner-bias is a single-offset rule.)
        //
        // The offset here is an Up edge — `route_side(Up)` is Top, so it shares the side with the
        // N reciprocal and yields the center to it (SQ-0222). A diagonal cannot play this role:
        // diagonals anchor on the corner and no longer take a side slot at all (SQ-0314).
        use crate::direction::Direction;
        let build = |partner_x: i32| {
            let mut g = MapGraph::new();
            for id in [1u16, 2, 3] { g.upsert_room(id.into(), "r".into()); }
            g.set_pos(1, (0, 0));
            g.set_pos(2, (0, -1)); // due north: straight N/S reciprocal → center winner
            g.set_pos(3, (partner_x, -1)); // the single offset's partner (east or west)
            g.add_edge(1, Direction::N, 2);
            g.add_edge(2, Direction::S, 1);
            g.add_edge(1, Direction::Up, 3);
            route_lanes(&g)
        };
        let slot = |plan: &RoutePlan, dir: Direction| {
            plan.connectors.iter().find(|c| c.origin == 1 && c.exit_dir == dir).map(|c| c.exit_slot).unwrap()
        };
        let west = build(-1);
        let east = build(1);
        assert_eq!(slot(&west, Direction::N), 0, "the straight N reciprocal keeps the center slot");
        assert_ne!(slot(&west, Direction::Up), 0, "the offset is not the center winner");
        assert_eq!(slot(&west, Direction::Up) % 2, 0, "west-partner offset takes a − (even) slot");
        assert_eq!(slot(&east, Direction::Up) % 2, 1, "east-partner offset takes a + (odd) slot");
    }

    #[test]
    fn diagonally_adjacent_rooms_route_as_one_pure_diagonal_only_when_reciprocal() {
        // SQ-0314: the corner lattice cell is odd/odd — already ON the all-odd gap lattice — so a
        // diagonal needs no perpendicular stub to reach it. For diagonally-adjacent rooms both
        // boxes' corners resolve to the SAME shared corner, collapsing a RECIPROCAL's route to
        // centre → corner → centre: R1(0,1) doubled (0,2), the shared corner (1,1), R2(1,0)
        // doubled (2,0). The corner is the exact midpoint, so it renders as one clean diagonal.
        //
        // SQ-1274 supersedes this test's old claim that "the one-way and the reciprocal MUST
        // agree here: an arrowhead is the only thing that should distinguish them, never the
        // path itself" — a corner is one of a room's eight compass slots, and a one-way arrival
        // there would read as a return path that does not exist. So a one-way diagonal now takes
        // an ordinary side doorway instead of the shared corner, and the two paths visibly
        // differ, not just their arrowheads.
        use crate::direction::Direction;
        let build = |reciprocal: bool| {
            let mut g = MapGraph::new();
            g.upsert_room(1, "R1".into());
            g.upsert_room(2, "R2".into());
            g.set_pos(1, (0, 1));
            g.set_pos(2, (1, 0)); // up and to the right of R1
            g.add_edge(1, Direction::NE, 2);
            if reciprocal {
                g.add_edge(2, Direction::SW, 1);
            }
            let plan = route_lanes(&g);
            plan.connectors.iter().find(|c| c.origin == 1).expect("the NE connector").clone()
        };

        let reciprocal = build(true);
        assert_eq!(
            reciprocal.points,
            vec![(0, 2), (1, 1), (2, 0)],
            "reciprocal: centre → shared corner → centre, no orthogonal dogleg",
        );
        assert_eq!(reciprocal.exit, Side::Right, "NE departs the right side");
        assert_eq!(reciprocal.entry, Side::Left, "and arrives on the left, its own back edge's side");
        assert_eq!(reciprocal.entry_corner, Some(Direction::SW), "it owns the corner via its own back edge");
        // A diagonal carries no LaneSeg: every step of the route touches an even coord (room
        // centre) or is the corner itself, so `long_runs` finds nothing to lane.
        assert!(reciprocal.segs.is_empty(), "a pure diagonal occupies no channel lane");

        let one_way = build(false);
        assert_eq!(one_way.exit, Side::Right, "NE still departs the right side");
        assert_eq!(one_way.entry_corner, None, "SQ-1274: a one-way arrival never keeps the corner");
        assert_ne!(
            one_way.points,
            vec![(0, 2), (1, 1), (2, 0)],
            "SQ-1274: unlike the reciprocal, the one-way's path itself must differ",
        );
    }

    #[test]
    fn two_one_way_arrivals_never_claim_the_shared_corner() {
        // SQ-0314 built this scenario (two one-way `NE`s into one room, both geometrically
        // wanting its SW corner) to test corner CONTENTION between them. SQ-1274 changes the
        // answer beneath it: a one-way arrival never holds a corner at all — any of a room's
        // eight compass slots landing an arrival reads as a return path that does not exist — so
        // there is no contest left to referee; both simply yield. Kept as the regression pin for
        // that, and for the newer rule: two rooms leading NE into a hub linked by a ladder is an
        // ordinary map shape, not a curiosity.
        //   at A, go northeast  -> Target   (one-way)
        //   at Target, go up    -> B        (non-planar: no compass back-edge, so the next NE
        //                                    cannot collapse into a reciprocal)
        //   at B, go northeast  -> Target   (one-way)  <- a SECOND NE into the same room
        use crate::direction::Direction;
        use crate::mapper::Mapper;
        let mut m = Mapper::default();
        m.observe_command(1, "A", "look");
        m.observe_command(2, "Target", "northeast");
        m.observe_command(3, "B", "up");
        m.observe_command(2, "Target", "northeast");
        let plan = route_lanes(&m.graph);

        let arrivals: Vec<_> = plan
            .connectors
            .iter()
            .filter(|c| c.dest == 2 && c.exit_dir == Direction::NE)
            .collect();
        assert_eq!(arrivals.len(), 2, "both NE edges are drawn: {arrivals:?}");
        for c in &arrivals {
            assert_eq!(c.entry_corner, None, "SQ-1274: no one-way arrival may hold Target's corner: {c:?}");
        }

        // Yielded to Target's side doorway, both land on distinct cells rather than stacking.
        let mut slots: Vec<u16> = arrivals.iter().map(|c| c.entry_slot).collect();
        slots.sort_unstable();
        assert_eq!(slots.len(), arrivals.len(), "one slot per arrival");
        assert!(slots.windows(2).all(|w| w[0] != w[1]), "no two arrivals share a slot: {slots:?}");

        // Corner claims across the whole map are unique (only a DEPARTURE or a reciprocal ever
        // makes one, so this is really checking those don't collide with each other).
        let mut claims: Vec<(RoomId, Direction)> = Vec::new();
        for c in &plan.connectors {
            if is_diagonal(c.exit_dir) {
                claims.push((c.origin, c.exit_dir));
            }
            if let Some(d) = c.entry_corner {
                claims.push((c.dest, d));
            }
        }
        // Direction derives Hash, not Ord, so this is a HashSet.
        let mut seen = std::collections::HashSet::new();
        for claim in &claims {
            assert!(seen.insert(*claim), "two connectors claim {claim:?}: {claims:?}");
        }
    }

    #[test]
    fn the_arrival_corner_owner_prefers_the_undistorted_claim_whatever_the_order() {
        // The winner is decided on merit, not on which edge happened to be discovered first, so the
        // same map always renders the same way. Tested on the rule itself with explicit flags: a
        // hand-built graph cannot produce real distortion marks (`mark_distorted` derives them from
        // the layout's dropped-constraint set, not from geometry), and the integration test above
        // covers the real-layout path.
        use crate::direction::Direction;
        let mk = |origin, dest, distorted| Connection { origin, dir: Direction::NE, dest, distorted, soft: false };
        let undistorted = mk(1, 2, false);
        let distorted = mk(3, 2, true);

        for order in [vec![&undistorted, &distorted], vec![&distorted, &undistorted]] {
            let owners = arrival_corner_owners(&order);
            let idx = owners[&(2, Direction::SW)];
            assert!(!order[idx].distorted, "the undistorted claim keeps the corner");
            assert_eq!(order[idx].origin, 1, "and it is always the same connector");
        }
    }

    #[test]
    fn an_uncontested_or_reciprocal_corner_is_left_alone() {
        // The map holds only CONTESTED corners; everything else must pass through untouched.
        use crate::direction::Direction;
        let mk = |origin, dir, dest| Connection { origin, dir, dest, distorted: false, soft: false };

        // A lone one-way arrival contends with nobody.
        let lone = mk(1, Direction::NE, 2);
        assert_eq!(arrival_corner_owners(&[&lone]).len(), 1, "recorded, but uncontested");

        // A reciprocal pair never enters the map: it owns its corner via its own back edge (and,
        // since SQ-1274, a one-way never keeps a corner regardless). Listing it here could make
        // it yield its own corner to a stranger.
        let out = mk(1, Direction::NE, 2);
        let back = mk(2, Direction::SW, 1);
        let owners = arrival_corner_owners(&[&out, &back]);
        assert!(owners.is_empty(), "a reciprocal pair is not arrival-vs-arrival contention: {owners:?}");

        // Cardinals have no corner to contend for.
        let e = mk(1, Direction::E, 2);
        assert!(arrival_corner_owners(&[&e]).is_empty());
    }

    #[test]
    fn a_departure_outranks_an_arrival_for_a_contested_corner() {
        // SQ-0314 named this "a departure outranks an arrival": the room's OWN outgoing diagonal
        // keeps its corner and the arrival yields to a side slot. SQ-1274 makes the arrival yield
        // UNCONDITIONALLY — no one-way arrival ever keeps a corner, contested or not — so the
        // departure no longer needs to "outrank" anything to keep what was always going to be
        // its own corner regardless. Kept as the regression pin for the shape below; see
        // `two_one_way_arrivals_never_claim_the_shared_corner` for the uncontested case.
        //
        // This is what an asymmetric diagonal passage IS, not an exotic shape: NE from Cave lands
        // in Ledge, but SW from Ledge goes to Pit rather than back to Cave. Ledge's SW corner is
        // wanted by the arrival from Cave; the departure to Pit owns it regardless. (Cave already
        // occupies the cell Ledge's SW edge wants, so that edge is distorted too —
        // `mark_distorted` lives in the mapper layer, above this hand-built graph, so the flag is
        // not asserted here.)
        use crate::direction::Direction;
        let mut g = MapGraph::new();
        for (id, n) in [(1u16, "Cave"), (2, "Ledge"), (3, "Pit")] {
            g.upsert_room(id.into(), n.into());
        }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, -1)); // NE of Cave
        g.set_pos(3, (-1, -1));
        g.add_edge(1, Direction::NE, 2); // one-way in  -> wants Ledge's SW corner
        g.add_edge(2, Direction::SW, 3); // one-way out <- departs Ledge's SW corner
        let plan = route_lanes(&g);
        let find = |o: RoomId| plan.connectors.iter().find(|c| c.origin == o).expect("connector");

        let departure = find(2);
        assert_eq!(departure.exit_dir, Direction::SW, "Ledge's own diagonal departs its SW corner");

        let arrival = find(1);
        assert_eq!(arrival.dest, 2);
        assert_eq!(
            entry_corner(arrival.exit_dir, arrival.entry_dir),
            Some(Direction::SW),
            "it wants Ledge's SW corner...",
        );
        assert_eq!(
            arrival.entry_corner, None,
            "...but SQ-1274 makes it yield unconditionally and take a side doorway",
        );

        // Having yielded, it is an ordinary side endpoint again — so it must be SLOTTED. Skipping
        // it would leave it stacked on whatever else shares that side.
        assert_eq!(arrival.entry, side_for(Direction::NE).map(|_| arrival.entry).unwrap());
        let sharers = plan
            .connectors
            .iter()
            .filter(|c| c.dest == 2 && c.entry_corner.is_none() && !c.merge)
            .count();
        assert_eq!(sharers, 1, "exactly the yielded arrival lands on Ledge's sides");
    }

    #[test]
    fn a_reciprocal_diagonal_pair_keeps_its_contested_corner() {
        // The yield rule (SQ-1274: no one-way arrival keeps a corner) must NOT fire for a
        // reciprocal pair. Ledge's SW edge here IS the back edge of the same connector — the
        // router collapsed the two — so it owns the corner by definition and IS the return path.
        use crate::direction::Direction;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Cave".into());
        g.upsert_room(2, "Ledge".into());
        g.set_pos(1, (0, 1));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::NE, 2);
        g.add_edge(2, Direction::SW, 1); // the reciprocal: same pair, collapses into ONE connector
        let plan = route_lanes(&g);
        assert_eq!(plan.connectors.len(), 1, "a reciprocal pair is one connector");
        assert_eq!(
            plan.connectors[0].entry_corner,
            Some(Direction::SW),
            "the reciprocal keeps the corner it is itself the back edge of",
        );
        assert_eq!(plan.connectors[0].points, vec![(0, 2), (1, 1), (2, 0)], "still one pure diagonal");
    }

    #[test]
    fn a_one_way_arrival_from_the_east_avoids_the_west_mid_side_slot() {
        // SQ-1274 (this is the Adventure report's exact shape — #42746/#55642's `E` edges into
        // "In A Valley" — reduced to two rooms): a one-way `E` edge ("go east") arrives on the
        // destination's WEST side (it approaches from the west, so it enters through the west
        // door), and it must never land on that side's MID-SIDE slot — the cell a real `W` exit
        // or a `?` random-exit mark in that direction would use — even when nothing else is on
        // that side to contest it with. An arrowhead there would read as "west leads back here",
        // which isn't true for a one-way passage.
        use crate::direction::Direction;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Origin".into());
        g.upsert_room(2, "Dest".into());
        g.set_pos(1, (-1, 0)); // west of Dest
        g.set_pos(2, (0, 0));
        g.add_edge(1, Direction::E, 2); // one-way "go east", arrives on Dest's west side
        let plan = route_lanes(&g);
        let arrival = plan.connectors.iter().find(|c| c.dest == 2).expect("the E connector");
        assert_eq!(arrival.entry, Side::Left, "it does land on Dest's west side");
        assert_ne!(arrival.entry_slot, 0, "but never on the west mid-side slot itself");
    }

    #[test]
    fn a_one_way_diagonal_arrival_never_lands_on_any_corner() {
        // SQ-1274, one case per corner: a one-way diagonal never keeps ANY of the four corners —
        // unlike the cardinal case above, there is no side to even partially land on; it falls
        // through to an ordinary side doorway on one of Dest's four SIDES instead.
        use crate::direction::Direction::{NE, NW, SE, SW};
        for (dir, origin_pos) in [(NE, (-1, 1)), (NW, (1, 1)), (SE, (-1, -1)), (SW, (1, -1))] {
            let mut g = MapGraph::new();
            g.upsert_room(1, "Origin".into());
            g.upsert_room(2, "Dest".into());
            g.set_pos(1, origin_pos);
            g.set_pos(2, (0, 0));
            g.add_edge(1, dir, 2); // one-way diagonal, no back edge
            let plan = route_lanes(&g);
            let arrival = plan.connectors.iter().find(|c| c.dest == 2).unwrap_or_else(|| {
                panic!("no connector for {dir:?} (origin {origin_pos:?}): {:?}", plan.connectors)
            });
            assert_eq!(arrival.entry_corner, None, "{dir:?}: a one-way diagonal never keeps a corner");
        }
    }

    #[test]
    fn an_updown_reciprocal_and_a_oneway_arrival_sharing_a_side_both_nest_off_center() {
        // SQ-1274, the shape found on the A129 fixture (`74 E 25` vs `26 Up 25`, see
        // `cleanup_keeps_updown_protected_column_chain_aligned` in the app crate): a reciprocal
        // Up/Down pair and a plain one-way compass arrival can end up sharing one destination
        // side. The one-way can never center (see the sort-key comment above), but pinning the
        // reciprocal to literal center instead left NO offset the one-way could take that didn't
        // clip the reciprocal's own approach in the shared gutter, on the real fixture — so
        // `assign_side_slots` nests BOTH off center, on opposite sides of it, rather than parking
        // the reciprocal dead-center for the one-way's bent approach to sweep past.
        use crate::direction::Direction::{Down, N, Up};
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.upsert_room(3, "C".into());
        g.set_pos(1, (0, 0)); // A: the destination both connectors share
        g.set_pos(2, (2, 2)); // B: A's Up/Down reciprocal partner
        g.set_pos(3, (0, 1)); // C: south of A, one-way N into A
        g.add_edge(2, Up, 1); // reciprocal Up/Down: 2->Up->1 / 1->Down->2
        g.add_edge(1, Down, 2);
        g.add_edge(3, N, 1); // one-way compass, no back edge: C north to A
        let plan = route_lanes(&g);
        let reciprocal = plan.connectors.iter().find(|c| c.reciprocal).expect("the Up/Down pair");
        let one_way = plan.connectors.iter().find(|c| !c.reciprocal).expect("the one-way N edge");
        assert_eq!(reciprocal.entry, one_way.entry, "precondition: both share A's same side");
        assert_ne!(reciprocal.entry_slot, 0, "the reciprocal nests off center too, not pinned to it");
        assert_ne!(one_way.entry_slot, 0, "the one-way never centers (unchanged SQ-1274 invariant)");
        assert_ne!(reciprocal.entry_slot, one_way.entry_slot, "the two land on distinct cells");
    }

    #[test]
    fn entry_corner_picks_the_corner_facing_the_origin() {
        // The single rule the router and the renderer BOTH ask, so their two ends of the same
        // connector cannot disagree about where it lands (SQ-0314).
        use Direction::*;
        // Reciprocal diagonal pair: the back edge's own direction.
        assert_eq!(entry_corner(NE, Some(SW)), Some(SW));
        // One-way diagonal: no back edge, but still the corner facing the origin.
        assert_eq!(entry_corner(NE, None), Some(SW));
        assert_eq!(entry_corner(SW, None), Some(NE));
        assert_eq!(entry_corner(NW, None), Some(SE));
        assert_eq!(entry_corner(SE, None), Some(NW));
        // A non-diagonal back edge keeps its ordinary side doorway.
        assert_eq!(entry_corner(NE, Some(S)), None);
        // A cardinal is never a corner, back edge or not.
        assert_eq!(entry_corner(E, None), None);
        assert_eq!(entry_corner(E, Some(W)), None);
    }

    #[test]
    fn corner_point_is_already_on_the_all_odd_lattice() {
        // The property the whole corner route rests on: unlike `exit_point` (one even coord, needing
        // a `snap_to_lattice` step), a corner is all-odd on arrival — so it IS its lattice point.
        use Direction::*;
        for d in [NE, NW, SE, SW] {
            for cell in [(0, 0), (3, -2), (-1, 4)] {
                let p = corner_point(cell, d).expect("diagonals have a corner");
                assert!(p.0 % 2 != 0 && p.1 % 2 != 0, "{d:?} at {cell:?}: {p:?} must be all-odd");
                assert_eq!(snap_to_lattice(p, (0, 0)), p, "{d:?}: snapping a corner is a no-op");
            }
        }
        assert_eq!(corner_point((0, 0), NE), Some((1, -1)));
        assert_eq!(corner_point((0, 0), SW), Some((-1, 1)));
        // Non-diagonals have no corner — they leave through a side doorway.
        for d in [N, S, E, W, Up, Down, In, Out, Unknown] {
            assert_eq!(corner_point((0, 0), d), None, "{d:?} is not a corner exit");
        }
    }

    #[test]
    fn a_diagonal_does_not_consume_a_side_slot() {
        // SQ-0314: a diagonal anchors on the box CORNER, which ignores the slot. So it must not
        // displace the endpoints that DO render on that side. Room 1 has a straight E reciprocal
        // (to room 2, due east) plus an NE and an SE diagonal — every one of them routes to the
        // Right side. Before SQ-0314 the two diagonals ate Right slots they never drew on, forcing
        // the E line off-centre; now the E line keeps the centre and the corners are free.
        use crate::direction::Direction;
        let mut g = MapGraph::new();
        for id in [1u16, 2, 3, 4] { g.upsert_room(id.into(), "r".into()); }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0)); // due east: straight E/W reciprocal
        g.set_pos(3, (1, -1)); // NE
        g.set_pos(4, (1, 1)); // SE
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        g.add_edge(1, Direction::NE, 3);
        g.add_edge(1, Direction::SE, 4);
        let plan = route_lanes(&g);
        let exit = |dir: Direction| {
            plan.connectors.iter().find(|c| c.origin == 1 && c.exit_dir == dir).expect("connector")
        };
        assert_eq!(exit(Direction::E).exit, Side::Right);
        assert_eq!(exit(Direction::NE).exit, Side::Right);
        assert_eq!(exit(Direction::SE).exit, Side::Right);
        assert_eq!(
            exit(Direction::E).exit_slot,
            0,
            "the straight E line keeps the centre slot: the diagonals leave via the corners, so \
             they never contend for it"
        );
    }

    #[test]
    fn route_side_puts_up_on_top_and_down_on_bottom() {
        use crate::direction::Direction;
        use crate::router::route_side;
        use crate::router::Side;
        // Up always departs the NORTH (top) border; Down always the SOUTH (bottom).
        assert_eq!(route_side(Direction::Up), Some(Side::Top));
        assert_eq!(route_side(Direction::Down), Some(Side::Bottom));
    }

    /// A pair joined by a compass edge AND a staircase draws ONE line; priority decides which,
    /// and E outranks Up (SQ-0522). SQ-0224 drew both on separate trunks, which is how a pair
    /// connected two ways became two lines that had to cross.
    #[test]
    fn compass_outranks_a_staircase_on_the_same_pair() {
        use crate::direction::Direction;
        use crate::graph::MapGraph;

        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(1, Direction::Up, 2);

        let plan = route_lanes(&g);
        assert_eq!(plan.connectors.len(), 1, "one line for the pair, whatever the directions");
        assert_eq!(plan.connectors[0].exit_dir, Direction::E, "E outranks Up");
        assert_eq!(plan.connectors[0].secondary_exit, vec![Direction::Up], "Up recorded, not drawn");
    }
    #[test]
    fn updown_stays_out_of_the_compass_direct_route_contest() {
        // `direct_route_losers` decides which connector keeps a contested straight room-line: the
        // LONGER one wins, the shorter weaves. An up/down edge sharing a pair with a compass edge
        // must stay OUT of that contest (SQ-0224) so it cannot steal the pair's representative slot
        // and flip who wins. Since SQ-0522 it is out of the routing set entirely — W outranks Up on
        // its pair, so Up is recorded and not drawn — which is a stronger version of the same
        // guarantee, so the contest winner must still be byte-identical either way.
        //
        // T(1) is the contested destination. A(2, far) and B(3, near) both want a straight W line
        // into T. The LONGER connector (A) must keep the straight line. B also carries an Up edge
        // to T (listed FIRST, sharing B's pair). Assert the contest WINNER A->T is byte-identical
        // with or without B's Up edge (the guard works), and that B's Up edge now draws.
        let build = |with_up: bool| {
            let mut g = MapGraph::new();
            for id in [1u16, 2, 3] { g.upsert_room(id.into(), "r".into()); }
            g.set_pos(1, (0, 0)); // T
            g.set_pos(2, (3, 4)); // A (far)
            g.set_pos(3, (1, 2)); // B (near)
            if with_up {
                g.add_edge(3, Direction::Up, 1); // shares B's pair; listed first
            }
            g.add_edge(3, Direction::W, 1); // B -> T, short contested line
            g.add_edge(2, Direction::W, 1); // A -> T, long contested line
            g
        };
        let with_up = route_lanes(&build(true));
        let without_up = route_lanes(&build(false));
        let a_with = with_up.connectors.iter().find(|c| c.origin == 2 && c.dest == 1)
            .expect("A->T connector is routed");
        let a_without = without_up.connectors.iter().find(|c| c.origin == 2 && c.dest == 1)
            .expect("A->T connector is routed");
        assert_eq!(a_with.points, a_without.points,
            "the contest winner A->T's straight line must be unaffected by B's Up edge staying out \
             of the contest: {:?} vs {:?}", a_with.points, a_without.points);
        assert!(
            with_up.connectors.iter().any(|c| {
                (c.origin.min(c.dest), c.origin.max(c.dest)) == (1, 3)
                    && c.secondary_exit.contains(&Direction::Up)
            }),
            "B's Up edge is recorded on the pair's retained line rather than drawing its own \
             (SQ-0522), which is a stronger form of staying out of the contest"
        );
    }

    #[test]
    fn lone_updown_pair_is_unaffected_by_suppression() {
        use crate::direction::Direction;
        use crate::graph::MapGraph;

        // Only an Up edge, no compass edge on this pair: the up/down connector survives.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::Up, 2);

        let plan = route_lanes(&g);
        let updown: Vec<_> = plan.connectors.iter()
            .filter(|c| matches!(c.exit_dir, Direction::Up | Direction::Down))
            .collect();
        assert_eq!(updown.len(), 1, "a pair with no compass edge keeps its up/down connector");
    }

    #[test]
    fn redundant_pair_collapses_to_one_shared_connector() {
        use crate::graph::MapGraph;
        use crate::direction::Direction;
        let mut g = MapGraph::new();
        g.upsert_room(68, "West".into());
        g.upsert_room(217, "South".into());
        g.set_pos(68, (0, 0));
        g.set_pos(217, (1, 1)); // 217 is SE of 68 → SE/NW satisfied, S/W not
        g.add_edge(68, Direction::S, 217);
        g.add_edge(68, Direction::SE, 217);
        g.add_edge(217, Direction::W, 68);
        g.add_edge(217, Direction::NW, 68);
        let plan = route_lanes(&g);
        let between: Vec<_> = plan.connectors.iter()
            .filter(|c| (c.origin.min(c.dest), c.origin.max(c.dest)) == (68, 217))
            .collect();
        assert_eq!(between.len(), 1, "the pair must collapse to a single connector");
        let c = between[0];
        // retained = the satisfied diagonal pairing
        assert_eq!(c.exit_dir, Direction::SE);
        assert_eq!(c.entry_dir, Some(Direction::NW));
        // secondaries: S at the 68 (exit/origin) end, W at the 217 (entry/dest) end
        let (exit_dirs, entry_dirs) = if c.origin == 68 {
            (&c.secondary_exit, &c.secondary_entry)
        } else {
            (&c.secondary_entry, &c.secondary_exit)
        };
        assert!(exit_dirs.contains(&Direction::S), "S recorded at the 68 end");
        assert!(entry_dirs.contains(&Direction::W), "W recorded at the 217 end");
    }

    #[test]
    fn one_sided_redundancy_collapses_too() {
        // #33/#175 shape: one way out (E), three back (W/N/S). SQ-0225 collapsed only TRUE
        // bidirectional redundancy (2+ each way) and left this shape as merge stubs. SQ-0522
        // collapses every pair: one-sided redundancy is still several lines between two rooms.
        use crate::graph::MapGraph;
        use crate::direction::Direction;
        let mut g = MapGraph::new();
        g.upsert_room(33, "F".into());
        g.upsert_room(175, "F".into());
        g.set_pos(33, (0, 0));
        g.set_pos(175, (1, 0));
        g.add_edge(33, Direction::E, 175);
        g.add_edge(175, Direction::W, 33);
        g.add_edge(175, Direction::N, 33);
        g.add_edge(175, Direction::S, 33);
        let plan = route_lanes(&g);
        assert_eq!(plan.connectors.len(), 1, "one line for the pair");
        let c = &plan.connectors[0];
        assert_eq!(c.entry_dir, Some(Direction::W), "W is the opposite of E, so it holds the line");
        let at_175 = if c.origin == 175 { &c.secondary_exit } else { &c.secondary_entry };
        let mut secs = at_175.clone();
        secs.sort_by_key(|d| format!("{d:?}"));
        assert_eq!(secs, vec![Direction::N, Direction::S], "the other two ways back become icons");
    }

    #[test]
    fn ordinary_reciprocal_pair_has_no_secondaries() {
        use crate::graph::MapGraph;
        use crate::direction::Direction;
        let mut g = MapGraph::new();
        g.upsert_room(1, "a".into());
        g.upsert_room(2, "b".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        let plan = route_lanes(&g);
        for c in &plan.connectors {
            assert!(c.secondary_exit.is_empty() && c.secondary_entry.is_empty());
        }
    }

    #[test]
    fn collapse_does_not_mutate_the_graph() {
        use crate::graph::MapGraph;
        use crate::direction::Direction;
        let mut g = MapGraph::new();
        g.upsert_room(68, "W".into());
        g.upsert_room(217, "S".into());
        g.set_pos(68, (0, 0));
        g.set_pos(217, (1, 1));
        for (o, d, dst) in [(68, Direction::S, 217), (68, Direction::SE, 217),
                            (217, Direction::W, 68), (217, Direction::NW, 68)] {
            g.add_edge(o, d, dst);
        }
        let before = g.connections().len();
        let _ = route_lanes(&g);
        assert_eq!(g.connections().len(), before, "routing must not delete edges");
    }
}
