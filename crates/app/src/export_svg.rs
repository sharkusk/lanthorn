//! SVG map export: render a `RenderMap` to a standalone SVG document.
//!
//! # The routes are the terminal's own routes (SQ-1313)
//!
//! This file draws **no** geometry of its own. It calls exactly what the Boxes-zoom cell
//! renderer calls — [`crate::render::map::boxes_axes_sized`] for the non-uniform axes whose
//! channels widen to hold the lanes [`mapper::route::RoutePlan`] assigned, and
//! [`crate::render::map::plot_connector`] for each connector's orthogonal run, its
//! side-anchor slots and its arrowhead anchors — and then scales the result from layout
//! cells to SVG pixels. A `RoutePlan` is expressed in doubled cell coordinates and knows
//! nothing about how big a box is, so widening a column to fit a long room name moves the
//! boxes and the channels together and leaves every routing decision untouched.
//!
//! What that buys is a single source of truth: there is no second router to drift from the
//! first. `plot_connector` hands back `ConnectorPlot::path` — the same run its per-cell
//! glyph masks are built from, reduced to its turning points — and this file strokes that
//! polyline. The terminal renderer never reads `path`; this file never reads `cells`.
//!
//! # Coordinate mapping
//!
//! Everything is laid out on the shared **cell** lattice and multiplied into pixels at the
//! last moment:
//!
//! * a room at grid line `(c, r)` occupies cells `[cols.room_pixel(c), + cols.box_dim_at(c))`
//!   × `[rows.room_pixel(r), + rows.box_dim_at(r))`, i.e. px rect
//!   `(bx * CELL_W, by * CELL_H, w * CELL_W, h * CELL_H)`;
//! * a connector point is a cell, drawn through its CENTRE: `(cx * CELL_W + CELL_W / 2, …)`;
//! * a connector's first and last points are anchors ON a box border cell, snapped out to the
//!   box's exact pixel edge so the line visibly touches the room.
//!
//! The document is emitted in that unshifted space and wrapped in one `translate(…)` that
//! brings its top-left corner to the margin, so nothing has to know the canvas size up front.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as FmtWrite;
use std::path::Path;

use mapper::direction::{self, Direction};
use mapper::graph::{MapGraph, PassageWeight, RoomId};
use mapper::render::RenderMap;
use mapper::router::Side;

use crate::render::map::{
    boxes_axes_sized, plot_connector, random_stub_cells, PosTable, BOX_H, BOX_W,
};

/// SVG pixels per layout cell. The terminal's cell is about 1:2, so its 11×5 box reads square
/// there; on a square SVG pixel the same box is a 99×45 rectangle, which is the shape a room
/// name wants anyway.
const CELL_W: i32 = 9;
const CELL_H: i32 = 9;

/// Room-label type size, and the fixed monospace advance a box is measured against. 0.6 em is
/// the advance of every monospace face in the fallback stack below (Menlo, DejaVu Sans Mono,
/// Consolas are all 0.6), so the measurement holds whichever one the viewer resolves.
const LABEL_PX: f64 = 11.0;
const ADVANCE: f64 = LABEL_PX * 0.6;
/// Horizontal breathing room inside a room box, per side.
const LABEL_PAD: f64 = 6.0;
/// The widest a box may grow to fit its name; anything longer is ellipsised.
const MAX_BOX_CELLS: i32 = 30;

/// Corner radius of a connector's right-angle turns.
const CORNER_R: f64 = 5.0;

/// A cross-layer ghost's two text lines (SQ-1319): the room name, and the layer name smaller
/// beneath it. Padding, sizes and per-character advances for both, at the small scale the map's
/// direction tags and badge names already use (`SMALL_ADV`/`SMALL_PX` below share the same
/// 0.6125 ratio for the same monospace stack).
const GHOST_PAD: f64 = 4.0;
const GHOST_NAME_PX: f64 = 8.0;
const GHOST_LAYER_PX: f64 = 6.5;
const GHOST_NAME_ADV: f64 = GHOST_NAME_PX * 0.6125;
const GHOST_LAYER_ADV: f64 = GHOST_LAYER_PX * 0.6125;
const GHOST_LINE_GAP: f64 = 3.0;
/// The distance `place_ghost` extends by each time the current spot is occupied — one lane's
/// worth, matching `settle_badge`'s own step so a pushed-out ghost still reads as "one channel
/// over" rather than landing at an arbitrary distance.
const GHOST_STEP: f64 = 17.0;

/// A departure ghost's own starting gap (SQ-1330): the minimum distance, in SVG px, from a
/// portal badge's centre to its ghost's near edge. Wide enough for the badge's own visual
/// radius (6.5, see `badge`) plus one arrowhead's full length (`ARROW_HEAD_LEN`) plus a couple
/// of pixels of daylight, so the new "into the ghost" arrowhead — which occupies the LAST
/// `ARROW_HEAD_LEN` px before the ghost — never overlaps the badge it travels away from.
const GHOST_DEPARTURE_GAP: f64 = 6.5 + ARROW_HEAD_LEN + 2.5;

/// Margin between the drawing and the canvas edge.
const MARGIN: i32 = 24;

/// The one font stack every text element uses. No Nerd Font, no symbol font: the badges are
/// letters and the marks are drawn as paths, so the export renders the same everywhere.
const MONO: &str = "ui-monospace, SFMono-Regular, Menlo, DejaVu Sans Mono, Consolas, monospace";

/// The stylesheet every document carries. Classes, not per-element attributes, so a consumer
/// can restyle the export without re-rendering it (SQ-1313).
fn stylesheet() -> String {
    format!(
        "<style>\
         .bg{{fill:#1a1a2e}}\
         text{{font-family:{MONO}}}\
         .room{{fill:#2a2a4a;stroke:#8bf;stroke-width:1.2}}\
         .room.current{{fill:#3a2f22;stroke:#f0c040;stroke-width:2.4}}\
         .room-label{{fill:#dde;font-size:{LABEL_PX}px;text-anchor:middle}}\
         .room.current+.room-label,.room-label.current{{fill:#ffe9b0}}\
         .notes{{fill:#fc0}}\
         .edge{{fill:none;stroke:#8bf;stroke-width:1.6;stroke-linecap:round;stroke-linejoin:round}}\
         .edge.reciprocal{{stroke:#9cf}}\
         .edge.oneway{{stroke:#8bf}}\
         .edge.asym{{stroke:#8bf}}\
         .edge.shared{{stroke:#cfa}}\
         .edge.portal{{stroke:#b9f;stroke-dasharray:1 3}}\
         .edge.conditional{{stroke-dasharray:1.5 3.5}}\
         .edge.distorted{{stroke:#e88;stroke-dasharray:5 3}}\
         .edge.stub{{stroke:#888;stroke-dasharray:2 2}}\
         .arrow{{fill:#8bf;stroke:none}}\
         .arrow.distorted{{fill:#e88}}\
         .arrow.shared{{fill:#cfa}}\
         .door{{stroke:#ffd479;stroke-width:2;fill:none}}\
         .door-gap{{fill:#1a1a2e;stroke:none}}\
         .badge{{fill:#241f36;stroke:#b9f;stroke-width:1.2}}\
         .badge-text{{fill:#d9c8ff;font-size:8px;text-anchor:middle}}\
         .badge-dest{{fill:#a99cc8;font-size:8px}}\
         .tag{{fill:#9ab;font-size:8px}}\
         .ghost{{fill:#1f1f38;stroke:#77c;stroke-width:1;stroke-dasharray:3 2}}\
         .ghost.arrival{{stroke:#8bf}}\
         .ghost-line{{stroke:#77c;stroke-width:1;stroke-dasharray:2 2;fill:none}}\
         .ghost text{{text-anchor:middle}}\
         .ghost-name{{fill:#cdd;font-size:{GHOST_NAME_PX}px}}\
         .ghost-layer{{fill:#99b;font-size:{GHOST_LAYER_PX}px}}\
         .random{{fill:#f8a;font-size:9px}}\
         .heading{{fill:#fff;font-size:14px}}\
         .legend{{fill:#dde;font-size:9px}}\
         .legend-panel{{fill:#20203a;stroke:#44446a;stroke-width:1}}\
         .legend-title{{fill:#fff;font-size:10px}}\
         .layer-frame{{fill:#20203a;stroke:#44446a;stroke-width:1}}\
         </style>"
    )
}

/// Escape XML special characters in a text value.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Format a float for an SVG attribute: at most one decimal, no trailing `.0`.
fn f(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    if (r - r.round()).abs() < f64::EPSILON {
        format!("{}", r.round() as i64)
    } else {
        format!("{r}")
    }
}

/// The running extent of everything emitted, so the document can be sized and shifted once at
/// the end rather than every piece having to know the canvas up front.
#[derive(Debug, Default, Clone, Copy)]
struct Extent {
    min: Option<(f64, f64, f64, f64)>, // (min_x, min_y, max_x, max_y)
}
impl Extent {
    fn add(&mut self, x: f64, y: f64) {
        self.min = Some(match self.min {
            None => (x, y, x, y),
            Some((a, b, c, d)) => (a.min(x), b.min(y), c.max(x), d.max(y)),
        });
    }
    fn add_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.add(x, y);
        self.add(x + w, y + h);
    }
    fn get(&self) -> (f64, f64, f64, f64) {
        self.min.unwrap_or((0.0, 0.0, 0.0, 0.0))
    }
}

// ── Room labels ───────────────────────────────────────────────────────────────

/// The most characters a box of `cells` cells may hold on one line.
fn chars_in(cells: i32) -> usize {
    (((cells * CELL_W) as f64 - 2.0 * LABEL_PAD) / ADVANCE).floor().max(1.0) as usize
}

/// Wrap `label` onto at most two lines, balanced so the box need be no wider than it must.
///
/// A label that already fits one default-width line is left alone; anything longer is split at
/// whichever word boundary minimises the LONGER of the two lines, which is what makes a box
/// grow by as little as possible. A single word too long for the widest box is ellipsised.
fn wrap_label(label: &str) -> Vec<String> {
    let label = label.trim();
    let one_line = chars_in(BOX_W);
    if label.chars().count() <= one_line {
        return vec![label.to_string()];
    }
    let words: Vec<&str> = label.split_whitespace().collect();
    let cap = chars_in(MAX_BOX_CELLS);
    let clip = |s: String| -> String {
        if s.chars().count() > cap {
            s.chars().take(cap.saturating_sub(1)).chain(std::iter::once('…')).collect()
        } else {
            s
        }
    };
    if words.len() < 2 {
        return vec![clip(label.to_string())];
    }
    let mut best: Option<(usize, String, String)> = None;
    for split in 1..words.len() {
        let a = words[..split].join(" ");
        let b = words[split..].join(" ");
        let key = a.chars().count().max(b.chars().count());
        if best.as_ref().is_none_or(|(k, _, _)| key < *k) {
            best = Some((key, a, b));
        }
    }
    let (_, a, b) = best.expect("at least one split for two or more words");
    vec![clip(a), clip(b)]
}

/// The box width, in layout cells, that holds `lines`.
fn box_cells(lines: &[String]) -> i32 {
    let widest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f64;
    let need = widest * ADVANCE + 2.0 * LABEL_PAD;
    ((need / CELL_W as f64).ceil() as i32).clamp(BOX_W, MAX_BOX_CELLS)
}

// ── Passage weights ───────────────────────────────────────────────────────────

/// Every passage's weight (SQ-1312), keyed the way a connector names itself.
///
/// Read off the graph rather than the render model because `RoutedConnector` carries where a
/// passage is DRAWN, not what kind of passage it is — the weight only ever mattered to the
/// layout before, so nothing carried it out this far.
fn weight_table(graph: &MapGraph) -> HashMap<(RoomId, Direction), PassageWeight> {
    graph.connections().iter().map(|c| ((c.origin, c.dir), c.weight)).collect()
}

// ── Geometry helpers ──────────────────────────────────────────────────────────

/// One axis's SVG pixel geometry (SQ-1322): the per-cell pixel width of every cell in `cols`'s or
/// `rows'`s virtual grid — `CELL_W`/`CELL_H` for a box-interior cell, or for a channel whose plain
/// `cell_count * CELL_W` already meets `MIN_CHANNEL_PX` (a "busy" channel — two or more lanes, or
/// widened for a diagonal bend), or `MIN_CHANNEL_PX` split evenly across the cells of a channel
/// that does not.
///
/// This is the ONLY place a shared cell count (`render::map::MIN_GUTTER`, or `DIAG_GUTTER` for a
/// diagonal) turns into an SVG pixel count. `PosTable`, `lane_pixel` and `plot_connector` are
/// untouched — every point they hand back is
/// still an ordinary CELL index, exactly what the terminal resolves it to — so nothing about the
/// shared routing or the terminal's own layout changes; only the LAST step, converting one of
/// those cell indices into a pixel for this file's own drawing, reads this table instead of a
/// flat multiply. A lane's cell-domain offset is therefore unaffected (still `LANE_BASE +
/// lane * LANE_SPACING` cells from the box edge, same as the terminal); it simply lands somewhere
/// inside the channel's own (possibly widened) pixel span rather than outside it, since every
/// cell in that span — including the lane's own — got an equal, non-negative share of it.
struct PxAxis {
    /// Pixel position of the START of cell `origin + i`.
    start: Vec<f64>,
    /// Pixel width of cell `origin + i`.
    width: Vec<f64>,
    origin: i32,
    /// The flat per-cell scale (`CELL_W` or `CELL_H`) used outside the tabulated span.
    cell_unit: i32,
}

impl PxAxis {
    /// Build from `axis`'s own cell layout — `range()`, `box_dim_at()`, `channel_span()`, the same
    /// public accessors any other consumer of a `PosTable` uses.
    fn build(axis: &PosTable, cell_unit: i32) -> PxAxis {
        let (lo, hi) = axis.range();
        let mut width = Vec::new();
        for idx in lo..=hi {
            for _ in 0..axis.box_dim_at(idx) {
                width.push(cell_unit as f64);
            }
            // A floor on the RESULTING pixel width, not on the cell count that produced it: a
            // channel widened to `DIAG_GUTTER` (3 cells, SQ-0314) for a diagonal bend elsewhere
            // in this same column/row is still only 27px at `CELL_W`=9 — short of
            // `MIN_CHANNEL_PX` just as a bare `MIN_GUTTER` (2 cells) is, and a straight two-way
            // passage sharing that column reads as a bowtie exactly the same way. Whatever
            // ALREADY clears the floor (two or more lanes) is left at the plain `cell_unit` scale
            // it always had.
            let chan = axis.channel_span(idx);
            let default_px = chan as f64 * cell_unit as f64;
            let per_cell =
                if default_px < MIN_CHANNEL_PX { MIN_CHANNEL_PX / chan as f64 } else { cell_unit as f64 };
            for _ in 0..chan {
                width.push(per_cell);
            }
        }
        let mut start = Vec::with_capacity(width.len());
        let mut acc = 0.0;
        for &w in &width {
            start.push(acc);
            acc += w;
        }
        PxAxis { start, width, origin: axis.room_pixel(lo), cell_unit }
    }

    /// Pixel position of the START (left/top edge) of cell `c`.
    fn start_of(&self, c: i32) -> f64 {
        let i = c - self.origin;
        if i >= 0 && (i as usize) < self.start.len() {
            self.start[i as usize]
        } else {
            // Outside the tabulated span. A real route never reaches here in practice —
            // `span_over` already widens `lo..hi` to cover everywhere a route runs — so this is a
            // defensive fallback, continuous with the tabulated portion at the flat `cell_unit`
            // scale rather than a hard failure.
            c as f64 * self.cell_unit as f64
        }
    }

    /// Pixel CENTRE of cell `c` — what a connector's own resolved cell/lane coordinate draws at.
    fn center_of(&self, c: i32) -> f64 {
        let i = c - self.origin;
        let w = if i >= 0 && (i as usize) < self.width.len() {
            self.width[i as usize]
        } else {
            self.cell_unit as f64
        };
        self.start_of(c) + w / 2.0
    }
}

/// A room's box, in layout cells: `(left, top, width, height)`.
fn box_cell_rect(cols: &PosTable, rows: &PosTable, cell: (i32, i32)) -> (i32, i32, i32, i32) {
    (
        cols.room_pixel(cell.0),
        rows.room_pixel(cell.1),
        cols.box_dim_at(cell.0),
        rows.box_dim_at(cell.1),
    )
}

/// A room's box in SVG pixels. `px_cols`/`px_rows` are the axes' [`PxAxis`] geometry (SQ-1322) —
/// only the box's START moves when a channel before it was widened; its own width never does,
/// since a box's interior cells are never widened.
fn box_px_rect(
    cols: &PosTable,
    rows: &PosTable,
    px_cols: &PxAxis,
    px_rows: &PxAxis,
    cell: (i32, i32),
) -> (f64, f64, f64, f64) {
    let (bx, by, w, h) = box_cell_rect(cols, rows, cell);
    (px_cols.start_of(bx), px_rows.start_of(by), (w * CELL_W) as f64, (h * CELL_H) as f64)
}

/// The centre of layout cell `c` in SVG pixels.
fn cell_px(px_cols: &PxAxis, px_rows: &PxAxis, c: (i32, i32)) -> (f64, f64) {
    (px_cols.center_of(c.0), px_rows.center_of(c.1))
}

/// The outward unit normal of `side`.
fn outward(side: Side) -> (f64, f64) {
    match side {
        Side::Right => (1.0, 0.0),
        Side::Left => (-1.0, 0.0),
        Side::Top => (0.0, -1.0),
        Side::Bottom => (0.0, 1.0),
    }
}

/// One passage with no planar route (`RoutedEdge::is_stub`), as the badge pass needs it: which
/// way it leads, what it leads TO, and whether that is on another layer.
///
/// The three travel together because the badge is wrong without all of them — a letter with no
/// side to sit on, or a destination name on a passage that never left the layer.
#[derive(Debug, Clone, Copy)]
struct Stub<'a> {
    dir: Direction,
    dest: Option<&'a str>,
    interlayer: bool,
}

impl Stub<'_> {
    /// The box side this passage leaves by — see [`side_for_travel`].
    fn side(self) -> Side {
        side_for_travel(self.dir)
    }
}

/// The box side a passage travelling `dir` leaves by. Up and Down take the top and bottom
/// borders (as they do in the drawn view's portal slots); In and Out have no bearing of their
/// own and take the right; a compass passage (including one walked diagonally, or one that
/// crossed a layer per SQ-0360) asks the router the same question a real connector's own
/// perpendicular leg does.
///
/// Also used, with `dir` reversed, to guess which side a one-way cross-layer ARRIVAL belongs on
/// (SQ-1319's `arrival_ghosts`) — there is no routed geometry to ask on that end, so the arrival
/// is placed as if the return trip had been walked.
fn side_for_travel(dir: Direction) -> Side {
    match dir {
        Direction::Up => Side::Top,
        Direction::Down => Side::Bottom,
        Direction::In | Direction::Out => Side::Right,
        d => mapper::router::side_for(d).unwrap_or(Side::Right),
    }
}

/// A one-way cross-layer arrival (SQ-1319): a passage lands on a room being drawn in the current
/// layer's panel, travelled by `traveled` on the ORIGIN's side, from `origin_name` on
/// `origin_layer` — and no connection runs back the other way, so this room's own ghost is the
/// only place the crossing is ever named (see `arrival_ghosts`).
#[derive(Debug, Clone)]
struct ArrivalGhost {
    traveled: Direction,
    origin_name: String,
    origin_layer: String,
}

/// One-way cross-layer arrivals landing in `layer` (SQ-1319): every interlayer connection whose
/// destination lives here, with no connection back the other way.
///
/// `interlayer_badges` only ever emits a room's own OUTGOING crossing — when the graph has a
/// connection back (however indirect the reciprocity), the destination's own panel draws its own
/// outgoing ghost for it, which IS the mirror this crossing needs. Only a genuinely one-way
/// crossing leaves the arriving side with nothing, and that is what this fills in.
fn arrival_ghosts(graph: &MapGraph, layer: mapper::layer::LayerId) -> HashMap<RoomId, Vec<ArrivalGhost>> {
    let mut out: HashMap<RoomId, Vec<ArrivalGhost>> = HashMap::new();
    for c in graph.connections() {
        if !mapper::layer::is_interlayer(graph, c) || graph.layer_of(c.dest) != layer {
            continue;
        }
        let reciprocal =
            graph.connections().iter().any(|c2| c2.origin == c.dest && c2.dest == c.origin);
        if reciprocal {
            continue;
        }
        let Some(origin) = graph.room(c.origin) else { continue };
        out.entry(c.dest).or_default().push(ArrivalGhost {
            traveled: c.dir,
            origin_name: origin.label().to_string(),
            origin_layer: graph.layer_name(graph.layer_of(c.origin)).to_string(),
        });
    }
    out
}

/// The compass direction a connector leaving by `side` APPEARS to take.
fn side_dir(side: Side) -> Direction {
    match side {
        Side::Right => Direction::E,
        Side::Left => Direction::W,
        Side::Top => Direction::N,
        Side::Bottom => Direction::S,
    }
}

/// Pull an anchor point out onto the box's exact pixel edge.
///
/// The shared geometry anchors on a border CELL, whose centre sits half a cell inside the box's
/// pixel edge — invisible in a terminal, a visible gap in a vector drawing. Only the coordinate
/// perpendicular to `side` moves, so a leg that left the anchor at 90° still does.
fn snap_to_edge(p: (f64, f64), rect: (f64, f64, f64, f64), side: Side) -> (f64, f64) {
    let (x, y, w, h) = rect;
    match side {
        Side::Right => (x + w, p.1),
        Side::Left => (x, p.1),
        Side::Top => (p.0, y),
        Side::Bottom => (p.0, y + h),
    }
}

/// An orthogonal polyline as an SVG path with rounded corners.
fn rounded_path(pts: &[(f64, f64)]) -> String {
    if pts.len() < 2 {
        return String::new();
    }
    let mut d = format!("M {} {}", f(pts[0].0), f(pts[0].1));
    for i in 1..pts.len() - 1 {
        let (p, c, n) = (pts[i - 1], pts[i], pts[i + 1]);
        let len_in = ((c.0 - p.0).powi(2) + (c.1 - p.1).powi(2)).sqrt();
        let len_out = ((n.0 - c.0).powi(2) + (n.1 - c.1).powi(2)).sqrt();
        let r = CORNER_R.min(len_in / 2.0).min(len_out / 2.0);
        if r < 0.5 || len_in < 0.01 || len_out < 0.01 {
            let _ = write!(d, " L {} {}", f(c.0), f(c.1));
            continue;
        }
        let a = (c.0 - (c.0 - p.0) / len_in * r, c.1 - (c.1 - p.1) / len_in * r);
        let b = (c.0 + (n.0 - c.0) / len_out * r, c.1 + (n.1 - c.1) / len_out * r);
        let _ = write!(
            d,
            " L {} {} Q {} {} {} {}",
            f(a.0),
            f(a.1),
            f(c.0),
            f(c.1),
            f(b.0),
            f(b.1)
        );
    }
    let last = pts[pts.len() - 1];
    let _ = write!(d, " L {} {}", f(last.0), f(last.1));
    d
}

/// `arrowhead`'s own two reaches along its `(at, u)` axis: the sharp point at `ARROW_TIP`, the
/// flat back (the two `perp`-offset corners) at `ARROW_BASE`. Named so `MIN_CHANNEL_PX` below can
/// derive its number from the actual triangle instead of repeating "8.5"/"0.5" unexplained.
const ARROW_TIP: f64 = 8.5;
const ARROW_BASE: f64 = 0.5;
/// The triangle's own length along its axis, tip to back.
const ARROW_HEAD_LEN: f64 = ARROW_TIP - ARROW_BASE;

/// A filled arrowhead sitting on the first ~8px of a connector leaving `at` along `u`.
fn arrowhead(at: (f64, f64), u: (f64, f64), class: &str) -> String {
    let tip = (at.0 + u.0 * ARROW_TIP, at.1 + u.1 * ARROW_TIP);
    let base = (at.0 + u.0 * ARROW_BASE, at.1 + u.1 * ARROW_BASE);
    let perp = (-u.1, u.0);
    let (l, r) = (
        (base.0 + perp.0 * 3.7, base.1 + perp.1 * 3.7),
        (base.0 - perp.0 * 3.7, base.1 - perp.1 * 3.7),
    );
    format!(
        "<polygon class=\"{class}\" points=\"{},{} {},{} {},{}\"/>",
        f(tip.0),
        f(tip.1),
        f(l.0),
        f(l.1),
        f(r.0),
        f(r.1)
    )
}

/// The head of a TWO-WAY passage at one of its box ends: the same triangle, occupying the same
/// stretch of channel, but pointing INTO the box instead of out of it (SQ-1317).
///
/// A reciprocal draws a head at each end. Pointed outward — the way plain `arrowhead` points,
/// which since SQ-1346 is reserved for a MERGE stub's departure marker (it has no destination
/// edge of its own to arrive at — see `RoutedConnector::merge`) — the two heads of a passage
/// between ADJACENT boxes come nose to nose across a channel a few pixels wide, and the line
/// reads as a bowtie. Turned around they sit at the two ends of one line pointing into the rooms
/// it joins, which is the ordinary double-headed arrow for "you can go both ways", is what the
/// legend has always drawn for two-way — and, since SQ-1322, is GUARANTEED a real shaft between
/// the two backs however short the channel's own CELL count: `MIN_CHANNEL_PX` widens the SVG's
/// pixel mapping of a channel that would otherwise leave the two backs meeting or overlapping
/// (see `PxAxis`).
///
/// A one-way passage's own single head uses this same inward style (SQ-1346): every marker for a
/// travel sits at the end that travel arrives at, so a one-way's head reads as its destination's
/// arrival exactly the way each of a two-way's two heads does — never as its origin's departure.
///
/// `at` is the point ON the box edge; `u` is that side's outward normal, as everywhere else.
/// Reflecting through `at + (ARROW_TIP + ARROW_BASE)·u` swaps `arrowhead`'s two reaches: the sharp
/// tip lands `ARROW_BASE` from `at` (hugging the edge, pointing into the room) and the flat back
/// lands `ARROW_TIP` out — the reach `MIN_CHANNEL_PX` is sized against.
fn arrowhead_inward(at: (f64, f64), u: (f64, f64), class: &str) -> String {
    let flip = ARROW_TIP + ARROW_BASE;
    arrowhead((at.0 + u.0 * flip, at.1 + u.1 * flip), (-u.0, -u.1), class)
}

/// The SVG's own minimum channel width, in pixels (SQ-1322) — wide enough that a two-way passage
/// between ADJACENT boxes shows a real shaft between its two inward-pointing heads' own flat
/// backs, rather than the two backs meeting or overlapping (a "bowtie", `◄►`).
///
/// Derived from the arrowhead geometry itself, never a bare number: each inward head's flat back
/// sits `ARROW_TIP` px out from its own box edge (see `arrowhead_inward`), so two heads facing
/// each other across a channel of width `w` leave a shaft of `w - 2 * ARROW_TIP` between their
/// backs. The ask is a shaft at least twice one head's own length (`ARROW_HEAD_LEN`), giving
/// `w >= 2 * ARROW_TIP + 2 * ARROW_HEAD_LEN`.
///
/// This is an SVG-only pixel floor — see `PxAxis` — and never changes `render::map::MIN_GUTTER`,
/// the shared CELL count the terminal also lays channels out by.
const MIN_CHANNEL_PX: f64 = 2.0 * ARROW_TIP + 2.0 * ARROW_HEAD_LEN;

/// A lettered badge — the export's up/down/in/out glyph, spelled as a letter so the document
/// needs no symbol font at all.
fn badge(at: (f64, f64), letter: &str) -> String {
    format!(
        "<circle class=\"badge\" cx=\"{}\" cy=\"{}\" r=\"6.5\"/>\
         <text class=\"badge-text\" x=\"{}\" y=\"{}\">{}</text>",
        f(at.0),
        f(at.1),
        f(at.0),
        f(at.1 + 3.0),
        xml_escape(letter)
    )
}

/// The marker set for ONE travel of a connector — a lettered badge for a vertical (Up/Down)
/// word, since a flat arrowhead cannot show "up" or "down" in a 2-D drawing; a plain
/// `arrowhead_inward` for everything else, plus the mismatched-word `tag` when the side the
/// passage is drawn leaving by disagrees with its own compass word (a diagonal walked round the
/// corner orthogonally, or a distorted edge).
///
/// SQ-1346: every marker describing a travel sits at the end that travel ARRIVES at, beside the
/// head that points into the room it enters — so `pos`/`u` here are always the ARRIVAL point and
/// its outward normal, never the departure. `side` is nonetheless the travel's own DEPARTURE
/// side (`conn.exit` for the connector's own A→B word, `conn.entry` for a reciprocal's B→A word)
/// because the mismatch this checks for is about how the passage LEFT its room, not where its
/// head is drawn. `room` is the room this travel's word belongs to (the badge's own room, not
/// necessarily the one nearest `pos`), used only to key `portal_ends` so the stub pass below
/// doesn't also badge the same compass exit a second time.
#[allow(clippy::too_many_arguments)]
fn draw_travel_arrival(
    over: &mut String,
    ext: &mut Extent,
    placer: &mut TextPlacer,
    portal_ends: &mut std::collections::HashSet<(RoomId, Direction)>,
    pos: (f64, f64),
    u: (f64, f64),
    side: Side,
    word: Direction,
    room: RoomId,
    arrow_class: &str,
) {
    if matches!(word, Direction::Up | Direction::Down) {
        let root = (pos.0 + u.0 * 8.0, pos.1 + u.1 * 8.0);
        let at = settle_badge(placer, root, (-u.1, u.0));
        portal_ends.insert((room, word));
        over.push_str(&badge(at, if word == Direction::Up { "U" } else { "D" }));
        ext.add(at.0 - 8.0, at.1 - 8.0);
        ext.add(at.0 + 8.0, at.1 + 8.0);
        return;
    }
    over.push_str(&arrowhead_inward(pos, u, arrow_class));
    if side_dir(side) != word {
        let tag = direction::short_label(word).to_uppercase();
        let root = (pos.0 + u.0 * 11.0, pos.1 + u.1 * 11.0);
        if let Some(spot) = placer.place(tag.chars().count(), &spots_around(root, u, 5.0)) {
            let _ = write!(
                over,
                "<text class=\"tag\"{} x=\"{}\" y=\"{}\">{}</text>",
                spot.anchor.attr(),
                f(spot.x),
                f(spot.y),
                tag
            );
            let r = spot.rect(tag.chars().count());
            ext.add(r.0, r.1);
            ext.add(r.0 + r.2, r.1 + r.3);
        }
    }
}

/// The door mark: a bar across the line with a gap punched under it.
fn door_mark(at: (f64, f64), u: (f64, f64)) -> String {
    let perp = (-u.1, u.0);
    format!(
        "<circle class=\"door-gap\" cx=\"{}\" cy=\"{}\" r=\"4.5\"/>\
         <line class=\"door\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"/>",
        f(at.0),
        f(at.1),
        f(at.0 + perp.0 * 5.0),
        f(at.1 + perp.1 * 5.0),
        f(at.0 - perp.0 * 5.0),
        f(at.1 - perp.1 * 5.0)
    )
}

/// A place ON a drawn line: the point, and the unit direction of the run it sits on. A mark
/// stamped there — the door bar — needs both, and is wrong if it gets one from one segment and
/// the other from another.
type OnLine = ((f64, f64), (f64, f64));

/// A drawn line segment's two endpoints, in SVG pixels — what `all_segments` collects and
/// `place_ghost` keeps a ghost box clear of (SQ-1319).
type Segment = ((f64, f64), (f64, f64));

/// The midpoint of the longest segment of `pts`, with that segment's unit direction.
fn longest_mid(pts: &[(f64, f64)]) -> Option<OnLine> {
    let mut best_len = 0.0f64;
    let mut best: Option<OnLine> = None;
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        if len < 1.0 || len <= best_len {
            continue;
        }
        best_len = len;
        best = Some((
            ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0),
            ((b.0 - a.0) / len, (b.1 - a.1) / len),
        ));
    }
    best
}

// ── The body ──────────────────────────────────────────────────────────────────

/// The room/edge markup for one `RenderMap`, with no outer `<svg>` tag, no stylesheet and no
/// background rect — just the pieces [`render_svg`] wraps directly, and [`render_svg_layered`]
/// wraps once per layer inside a translated `<g>`. `None` for an empty map.
///
/// Returns the markup plus the `(width, height)` a caller needs to size its own canvas around
/// it. The markup's own top-left is already at `(0, 0)`.
///
/// `arrivals` names the one-way cross-layer crossings landing on THIS panel's rooms with no
/// connection back the other way (SQ-1319) — see [`arrival_ghosts`], the only place that builds
/// one. Empty for [`render_svg_of`], which has no layer context to compute it from.
fn render_svg_body(
    rm: &RenderMap,
    weights: &HashMap<(RoomId, Direction), PassageWeight>,
    arrivals: &HashMap<RoomId, Vec<ArrivalGhost>>,
) -> Option<(String, i32, i32)> {
    if rm.rooms.is_empty() {
        return None;
    }

    // ── Axes: the terminal's own, with each column widened to its widest room name ────────
    let labels: HashMap<RoomId, Vec<String>> =
        rm.rooms.iter().map(|r| (r.id, wrap_label(&r.label))).collect();
    let mut col_dims: BTreeMap<i32, i32> = BTreeMap::new();
    for room in &rm.rooms {
        let want = labels.get(&room.id).map(|l| box_cells(l)).unwrap_or(BOX_W);
        let slot = col_dims.entry(room.cell.0).or_insert(BOX_W);
        *slot = (*slot).max(want);
    }
    let no_rows = BTreeMap::new();
    let (cols, rows) = boxes_axes_sized(&rm.plan, rm.bounds, BOX_W, &col_dims, BOX_H, &no_rows);
    // SQ-1322: the SVG's own pixel geometry for each axis, widening a channel already at
    // `MIN_GUTTER` cells so a two-way passage between adjacent boxes gets a real shaft — see
    // `PxAxis`. `cols`/`rows` themselves are untouched and still the terminal's own cell layout.
    let px_cols = PxAxis::build(&cols, CELL_W);
    let px_rows = PxAxis::build(&rows, CELL_H);

    let cell_of: HashMap<RoomId, (i32, i32)> = rm.rooms.iter().map(|r| (r.id, r.cell)).collect();
    let rect_of = |id: RoomId| -> Option<(f64, f64, f64, f64)> {
        cell_of.get(&id).map(|&c| box_px_rect(&cols, &rows, &px_cols, &px_rows, c))
    };

    let mut ext = Extent::default();
    let mut edges = String::new(); // under the rooms
    let mut over = String::new(); // arrowheads, badges, tags — on top of the rooms
    // Every room box is off-limits to a label from the start (SQ-1317). A room's NAME is drawn
    // inside its box, so blocking the box blocks the name — and blocks the box outline too,
    // which a tag written across is just as unreadable on.
    let mut placer = TextPlacer::default();
    // The same boxes, kept apart from `placer.taken` (which also gathers badges and labels): a
    // ghost's CONNECTOR LINE is only ever checked against a room or another ghost (SQ-1333), never
    // against those — see `place_ghost`.
    let mut room_boxes: Vec<PxRect> = Vec::with_capacity(rm.rooms.len());
    for room in &rm.rooms {
        let r = box_px_rect(&cols, &rows, &px_cols, &px_rows, room.cell);
        placer.block(r);
        room_boxes.push(r);
    }
    // The `(room, direction)` ends the CONNECTOR pass badges. An Up/Down passage reaches this
    // function twice — once as a routed portal connector and once as a `RoutedEdge` stub, since
    // `router::side_for` gives Up and Down no planar side — and each pass drew its own badge. The
    // two landed a pixel apart and read as one slightly bold circle, so the duplication went
    // unnoticed until `settle_badge` started sliding coincident badges apart and turned every
    // vertical passage on the map into `Ⓤ Ⓤ`. The stub pass now yields to this one.
    let mut portal_ends: std::collections::HashSet<(RoomId, Direction)> =
        std::collections::HashSet::new();
    let mut boxes = String::new();
    // Every drawn connector segment, so a ghost box can be kept off the lanes just as it is kept
    // off rooms and other labels (SQ-1319) — filled by the connector pass below, read by the
    // ghost passes after it.
    let mut all_segments: Vec<Segment> = Vec::new();
    // Every ghost box already placed on this panel, so a LATER ghost's connector line can be kept
    // off an EARLIER one too (SQ-1333) — `room_boxes` above is fixed for the whole panel, this
    // grows as the two ghost passes below place each one.
    let mut ghost_boxes: Vec<PxRect> = Vec::new();

    // ── Connectors ───────────────────────────────────────────────────────────────────────
    //
    // `None` for the diagonal glyph set: half-diagonal corner stubs are a terminal line-art
    // affair (`SymbolSet::diagonal_corners`), and the orthogonal reading is exactly what the
    // router laid out either way — the toggle only ever picked which GLYPHS the intermediate
    // run used. A diagonal therefore arrives here as the dogleg it is, and says so with a
    // direction tag at its departure anchor.
    for conn in &rm.plan.connectors {
        let Some(plot) = plot_connector(conn, &cols, &rows, None) else { continue };
        if plot.path.len() < 2 {
            continue;
        }
        let is_portal = matches!(conn.exit_dir, Direction::Up | Direction::Down);
        let mut pts: Vec<(f64, f64)> = plot.path.iter().map(|&c| cell_px(&px_cols, &px_rows, c)).collect();

        // Snap the two ends onto their boxes' pixel edges (see `snap_to_edge`).
        if let Some(r) = rect_of(conn.origin) {
            pts[0] = snap_to_edge(pts[0], r, conn.exit);
        }
        if !conn.merge {
            if let Some(r) = rect_of(conn.dest) {
                let last = pts.len() - 1;
                pts[last] = snap_to_edge(pts[last], r, conn.entry);
            }
        }
        for &p in &pts {
            ext.add(p.0, p.1);
        }
        all_segments.extend(pts.windows(2).map(|w| (w[0], w[1])));

        let weight = [
            weights.get(&(conn.origin, conn.exit_dir)).copied(),
            conn.entry_dir.and_then(|d| weights.get(&(conn.dest, d)).copied()),
        ]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(PassageWeight::Hard);

        let mut class = String::from("edge");
        if is_portal {
            class.push_str(" portal");
        } else if conn.distorted {
            class.push_str(" distorted");
        } else if !conn.secondary_exit.is_empty() || !conn.secondary_entry.is_empty() {
            class.push_str(" shared");
        }
        if !is_portal && !conn.distorted && weight == PassageWeight::Conditional {
            class.push_str(" conditional");
        }
        class.push(' ');
        class.push_str(if conn.reciprocal {
            "reciprocal"
        } else if conn.entry_dir.is_some() {
            "asym"
        } else {
            "oneway"
        });
        let _ = write!(edges, "<path class=\"{}\" d=\"{}\"/>", class, rounded_path(&pts));

        // A door is a real walkable way that happens to need opening: mark the line, don't
        // restyle it.
        if weight == PassageWeight::Door && !is_portal {
            if let Some((mid, u)) = longest_mid(&pts) {
                edges.push_str(&door_mark(mid, u));
            }
        }

        // Every marker sits at the end its own travel ARRIVES at (SQ-1346): a two-way passage's
        // heads point INTO the rooms, so the two of them sit at the ends of one line instead of
        // meeting nose to nose in the channel (see `arrowhead_inward`) — and the badge/tag that
        // goes with each head travels to the SAME end, not the room it started from. A one-way
        // passage has exactly one travel and gets exactly one marker set, at its destination; the
        // bare line back to its origin IS the reading there (the terminal's one arrow rule,
        // SQ-0688).
        let dep_u = outward(conn.exit);
        let arrow_class = if conn.distorted { "arrow distorted" } else { "arrow" };
        if conn.merge {
            // A merge stub ends on another connector's TRUNK (a T-junction), not on a room edge —
            // see `RoutedConnector::merge` — so it has no arrival end of its own to carry a head
            // to. It keeps its long-standing departure-only marker: the shared trunk it joins is
            // what actually carries the arrival head into the destination room.
            if is_portal {
                let root = (pts[0].0 + dep_u.0 * 8.0, pts[0].1 + dep_u.1 * 8.0);
                let at = settle_badge(&mut placer, root, (-dep_u.1, dep_u.0));
                portal_ends.insert((conn.origin, conn.exit_dir));
                over.push_str(&badge(at, if conn.exit_dir == Direction::Up { "U" } else { "D" }));
                ext.add(at.0 - 8.0, at.1 - 8.0);
                ext.add(at.0 + 8.0, at.1 + 8.0);
            } else {
                over.push_str(&arrowhead(pts[0], dep_u, arrow_class));
                if side_dir(conn.exit) != conn.exit_dir {
                    let tag = direction::short_label(conn.exit_dir).to_uppercase();
                    let root = (pts[0].0 + dep_u.0 * 11.0, pts[0].1 + dep_u.1 * 11.0);
                    if let Some(spot) = placer.place(tag.chars().count(), &spots_around(root, dep_u, 5.0)) {
                        let _ = write!(
                            over,
                            "<text class=\"tag\"{} x=\"{}\" y=\"{}\">{}</text>",
                            spot.anchor.attr(),
                            f(spot.x),
                            f(spot.y),
                            tag
                        );
                        let r = spot.rect(tag.chars().count());
                        ext.add(r.0, r.1);
                        ext.add(r.0 + r.2, r.1 + r.3);
                    }
                }
            }
        } else {
            // This connector's own word (A→B, `conn.exit_dir`) arrives at the far end.
            let last = pts[pts.len() - 1];
            let arr_u = outward(conn.entry);
            draw_travel_arrival(
                &mut over,
                &mut ext,
                &mut placer,
                &mut portal_ends,
                last,
                arr_u,
                conn.exit,
                conn.exit_dir,
                conn.origin,
                arrow_class,
            );

            if conn.reciprocal {
                // The paired back-edge (B→A) is the OTHER travel this one line stands for, and
                // it arrives back at the departure end.
                let arr_dir = conn.entry_dir.unwrap_or(direction::opposite(conn.exit_dir));
                draw_travel_arrival(
                    &mut over,
                    &mut ext,
                    &mut placer,
                    &mut portal_ends,
                    pts[0],
                    dep_u,
                    conn.entry,
                    arr_dir,
                    conn.dest,
                    arrow_class,
                );
            }
        }
    }

    // ── Portal / cross-layer badges ──────────────────────────────────────────────────────
    //
    // A stub is a passage with no planar route — up, down, in, out, or a compass passage whose
    // destination lives on another layer. It gets a lettered badge on the side it leads by,
    // and — when it crosses a layer — a ghost box naming the room and layer it leads to,
    // joined to the badge by a short connector with an arrowhead into the ghost (SQ-1319; SQ-1330;
    // see `place_ghost`). The ghost is never dropped: unlike the single inline label this
    // replaced, its placement search always succeeds by extending outward until the panel has
    // room for it.
    let mut stubs_by_room: HashMap<RoomId, Vec<Stub<'_>>> = HashMap::new();
    for edge in &rm.edges {
        if !edge.is_stub || edge.dir == Direction::Unknown {
            continue;
        }
        if portal_ends.contains(&(edge.origin, edge.dir)) {
            continue; // the connector pass already badged this end — see `portal_ends`
        }
        stubs_by_room.entry(edge.origin).or_default().push(Stub {
            dir: edge.dir,
            dest: edge.dest_label.as_deref(),
            interlayer: edge.is_interlayer,
        });
    }
    for room in &rm.rooms {
        let Some(stubs) = stubs_by_room.get(&room.id) else { continue };
        let rect = box_px_rect(&cols, &rows, &px_cols, &px_rows, room.cell);
        // Group by the side each passage leads out of, then stack along that side.
        let mut per_side: HashMap<u8, Vec<Stub<'_>>> = HashMap::new();
        for &stub in stubs {
            per_side.entry(stub.side() as u8).or_default().push(stub);
        }
        let mut sides: Vec<u8> = per_side.keys().copied().collect();
        sides.sort_unstable();
        for s in sides {
            let side = [Side::Right, Side::Left, Side::Top, Side::Bottom]
                .into_iter()
                .find(|x| *x as u8 == s)
                .unwrap_or(Side::Right);
            let list = &per_side[&s];
            let u = outward(side);
            let tangent = (-u.1, u.0);
            for (i, &Stub { dir, dest, interlayer: inter }) in list.iter().enumerate() {
                let step = i as f64 * 17.0;
                let root = side_root(rect, side, 9.0, step);
                let at = settle_badge(&mut placer, root, tangent);
                over.push_str(&badge(at, &direction::short_label(dir).to_uppercase()));
                ext.add(at.0 - 8.0, at.1 - 8.0);
                ext.add(at.0 + 8.0, at.1 + 8.0);
                if inter {
                    if let Some(full) = dest {
                        let (name, layer_name) = full.split_once(" · ").unwrap_or((full, ""));
                        let text = GhostText { name, layer: layer_name };
                        let (gw, gh) = ghost_dims(text);
                        let gp = place_ghost(
                            &mut placer,
                            &all_segments,
                            &room_boxes,
                            &ghost_boxes,
                            at,
                            u,
                            GHOST_DEPARTURE_GAP,
                            gw,
                            gh,
                        );
                        ghost_boxes.push(gp.rect);
                        // A departure ghost stands for THIS room's own exit leaving toward it, so
                        // the arrow sits at the GHOST end, tip on its near edge, pointing further
                        // in — reading as "leaving here, arriving there" (SQ-1330). That is the
                        // mirror of an arrival ghost's arrow, which sits at the ROOM end instead
                        // (below): a ghost pair is two one-ways, one per panel, never a single
                        // two-way head pointing back at the room it started from. `gp.dir` is the
                        // axis the ghost actually landed on — `u` only when the direct side was
                        // clear, a tangent when SQ-1333's bend fired instead.
                        let near = ghost_near_edge(gp.rect, gp.dir);
                        over.push_str(&arrowhead_inward(near, (-gp.dir.0, -gp.dir.1), "arrow"));
                        over.push_str(&draw_ghost(at, gp.dir, gp.bend, gp.rect, text, false));
                        ext.add(gp.rect.0, gp.rect.1);
                        ext.add(gp.rect.0 + gp.rect.2, gp.rect.1 + gp.rect.3);
                        if let Some(b) = gp.bend {
                            ext.add(b.0, b.1);
                        }
                    }
                }
            }
        }
    }

    // ── Cross-layer arrivals (one-way only) ─────────────────────────────────────────────
    //
    // A one-way crossing has no connection back, so the loop above — which only ever fires from
    // the ORIGIN's own layer — never draws anything on the arriving side. SQ-1319 requires both
    // ends to say where a crossing goes, so the arriving room gets an inward arrowhead (arriving,
    // not leaving — no letter, since there is no local direction the story ever printed for it)
    // and its own ghost, naming where the passage came FROM instead of where it leads.
    for room in &rm.rooms {
        let Some(list) = arrivals.get(&room.id) else { continue };
        let rect = box_px_rect(&cols, &rows, &px_cols, &px_rows, room.cell);
        let mut per_side: HashMap<u8, Vec<&ArrivalGhost>> = HashMap::new();
        for a in list {
            per_side.entry(side_for_travel(direction::opposite(a.traveled)) as u8).or_default().push(a);
        }
        let mut sides: Vec<u8> = per_side.keys().copied().collect();
        sides.sort_unstable();
        for s in sides {
            let side = [Side::Right, Side::Left, Side::Top, Side::Bottom]
                .into_iter()
                .find(|x| *x as u8 == s)
                .unwrap_or(Side::Right);
            let u = outward(side);
            for (i, a) in per_side[&s].iter().enumerate() {
                let step = i as f64 * 17.0;
                let edge_pt = side_root(rect, side, 0.0, step);
                over.push_str(&arrowhead_inward(edge_pt, u, "arrow"));
                ext.add(edge_pt.0 - 9.0, edge_pt.1 - 9.0);
                ext.add(edge_pt.0 + 9.0, edge_pt.1 + 9.0);
                let anchor = side_root(rect, side, 13.0, step);
                // The connector line runs from `anchor` to the ghost box's near edge; both ends
                // must be IN `ext` for the panel's height to include the whole line. The
                // arrowhead footprint above only reaches ~8.5px out, short of `anchor` at 13px.
                ext.add(anchor.0, anchor.1);
                let text = GhostText { name: &a.origin_name, layer: &a.origin_layer };
                let (gw, gh) = ghost_dims(text);
                let gp = place_ghost(&mut placer, &all_segments, &room_boxes, &ghost_boxes, anchor, u, 4.0, gw, gh);
                ghost_boxes.push(gp.rect);
                over.push_str(&draw_ghost(anchor, gp.dir, gp.bend, gp.rect, text, true));
                ext.add(gp.rect.0, gp.rect.1);
                ext.add(gp.rect.0 + gp.rect.2, gp.rect.1 + gp.rect.3);
                if let Some(b) = gp.bend {
                    ext.add(b.0, b.1);
                }
            }
        }
    }

    // ── Random-exit (`?`) marks ──────────────────────────────────────────────────────────
    //
    // `random_stub_cells` is the primitive a real exit's own departure anchor is built from,
    // so a `?` can never be drawn somewhere a real exit would not.
    for room in &rm.rooms {
        let (bx, by, bw, bh) = box_cell_rect(&cols, &rows, room.cell);
        let rect = box_px_rect(&cols, &rows, &px_cols, &px_rows, room.cell);
        for &(dir, count) in &room.random_stubs {
            let Some(side) = mapper::router::side_for(dir) else { continue };
            let Some((arrow, out_cell)) = random_stub_cells(bx, by, bw, bh, dir) else { continue };
            let start = snap_to_edge(cell_px(&px_cols, &px_rows, arrow), rect, side);
            let u = outward(side);
            let end = (start.0 + u.0 * 13.0, start.1 + u.1 * 13.0);
            let _ = write!(
                edges,
                "<line class=\"edge stub\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"/>",
                f(start.0),
                f(start.1),
                f(end.0),
                f(end.1)
            );
            let label = if count > 1 { format!("?{count}") } else { "?".to_string() };
            let tx = end.0 + if u.0 < 0.0 { -10.0 } else { 2.0 };
            let ty = end.1 + if u.1 < 0.0 { -2.0 } else { 8.0 };
            let _ = write!(over, "<text class=\"random\" x=\"{}\" y=\"{}\">{}</text>", f(tx), f(ty), label);
            ext.add(tx - 4.0, ty - 8.0);
            ext.add(tx + 14.0, ty + 4.0);
            let _ = out_cell; // the count cell is where the line runs; the px reach is fixed
        }
    }

    // ── Rooms ────────────────────────────────────────────────────────────────────────────
    for room in &rm.rooms {
        let (x, y, w, h) = box_px_rect(&cols, &rows, &px_cols, &px_rows, room.cell);
        ext.add_rect(x, y, w, h);
        let cls = if room.is_current { "room current" } else { "room" };
        let _ = write!(
            boxes,
            "<rect class=\"{cls}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"4\"/>",
            f(x),
            f(y),
            f(w),
            f(h)
        );
        let empty = Vec::new();
        let lines = labels.get(&room.id).unwrap_or(&empty);
        let label_cls = if room.is_current { "room-label current" } else { "room-label" };
        let first = y + h / 2.0 - (lines.len() as f64 - 1.0) * (LABEL_PX * 0.62) + LABEL_PX * 0.36;
        for (i, line) in lines.iter().enumerate() {
            // `text-anchor="middle"` inline, not just via the `.room-label` CSS rule (which
            // already centres it visually): `text_boxes()` reads the ATTRIBUTE to measure a
            // label's true box, defaulting to "start" without it — understating how far right a
            // long name reaches and making the collision check it feeds blind to a real overlap
            // (SQ-1319 found this once a ghost's own box came close enough for the miss to
            // matter, on a hundred-room Anchorhead panel no synthetic case reaches).
            let _ = write!(
                boxes,
                "<text class=\"{label_cls}\" text-anchor=\"middle\" x=\"{}\" y=\"{}\">{}</text>",
                f(x + w / 2.0),
                f(first + i as f64 * LABEL_PX * 1.24),
                xml_escape(line)
            );
        }
        if room.has_notes {
            let _ = write!(
                boxes,
                "<circle class=\"notes\" cx=\"{}\" cy=\"{}\" r=\"2.6\"/>",
                f(x + w - 6.0),
                f(y + 6.0)
            );
        }
    }

    let (min_x, min_y, max_x, max_y) = ext.get();
    let (ox, oy) = (-min_x, -min_y);
    let width = (max_x - min_x).ceil() as i32;
    let height = (max_y - min_y).ceil() as i32;
    let body = format!(
        "<g transform=\"translate({},{})\">{edges}{boxes}{over}</g>",
        f(ox),
        f(oy)
    );
    Some((body, width.max(1), height.max(1)))
}

// ── Text placement ────────────────────────────────────────────────────────────

/// A rectangle in SVG pixels: `(x, y, w, h)`.
type PxRect = (f64, f64, f64, f64);

/// The advance of one character of the 8px labels (`.tag`, `.badge-dest`) in the monospace stack
/// the document asks for. The badge pass has always used this number to grow the drawing's
/// extent; it is named here because the overlap test needs the same one.
const SMALL_ADV: f64 = 4.9;
/// Cap height of those labels, and how far the box rises above the baseline.
const SMALL_PX: f64 = 8.0;

/// Which end of a `<text>` sits on its `x`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Anchor {
    Start,
    Middle,
    End,
}

impl Anchor {
    /// The `text-anchor` attribute, or `""` for the SVG default (`start`).
    fn attr(self) -> &'static str {
        match self {
            Anchor::Start => "",
            Anchor::Middle => " text-anchor=\"middle\"",
            Anchor::End => " text-anchor=\"end\"",
        }
    }
}

/// One candidate place for a short label: the anchor point, and which end of the text sits on it.
#[derive(Debug, Clone, Copy)]
struct Spot {
    x: f64,
    y: f64,
    anchor: Anchor,
}

impl Spot {
    /// The box `len` characters would occupy here. SVG `y` is the BASELINE, so the box rises
    /// above it.
    fn rect(self, len: usize) -> PxRect {
        let w = len as f64 * SMALL_ADV;
        let x = match self.anchor {
            Anchor::Start => self.x,
            Anchor::Middle => self.x - w / 2.0,
            Anchor::End => self.x - w,
        };
        (x, self.y - SMALL_PX * 0.8, w, SMALL_PX)
    }
}

/// Keeps short labels off things already drawn (SQ-1317).
///
/// A direction tag and a cross-layer badge's destination name are both written OUTWARD from an
/// anchor, and an outward offset says nothing about what is out there. On Zork I that put
/// `Maze` straight through `Cyclops Room`'s own name — the badge sits on the room's LEFT side,
/// the name was written left-anchored from it, and left-anchored text runs RIGHT, back across
/// the box it was meant to sit beside.
///
/// So a label states several places it would accept, in order of preference, and takes the first
/// that is clear of every room, every badge, and every label already placed. When none is clear
/// the label is DROPPED and its glyph stays: a badge without its destination name still says a
/// passage leads off the layer, and the legend explains the letter, where a name written over a
/// room name costs both.
#[derive(Default)]
struct TextPlacer {
    taken: Vec<PxRect>,
}

impl TextPlacer {
    fn block(&mut self, r: PxRect) {
        self.taken.push(r);
    }

    fn is_free(&self, r: PxRect) -> bool {
        !self.taken.iter().any(|&t| {
            r.0 < t.0 + t.2 && t.0 < r.0 + r.2 && r.1 < t.1 + t.3 && t.1 < r.1 + r.3
        })
    }

    /// The first candidate whose box is clear, marked taken. `None` when every one is occupied.
    fn place(&mut self, len: usize, candidates: &[Spot]) -> Option<Spot> {
        let spot = *candidates.iter().find(|s| self.is_free(s.rect(len)))?;
        self.block(spot.rect(len));
        Some(spot)
    }
}

/// The box a badge drawn at `at` occupies (radius 6.5, plus a pixel of air).
fn badge_rect(at: (f64, f64)) -> PxRect {
    (at.0 - 8.0, at.1 - 8.0, 16.0, 16.0)
}

/// Settle a badge at `at`, sliding it along `tangent` until it is clear of every badge and room
/// already placed, and reserve where it lands (SQ-1317).
///
/// Two badges landing on one point is not only a same-room affair, which is why the stub pass's
/// own `i * 17` stacking was not enough: `Clearing`'s DOWN badge sits below its box and
/// `Forest Path`'s UP badge sits above its own, and the two rooms are vertical neighbours — one
/// gutter, one point, two letters on top of each other. Sliding on a shared occupancy map sees
/// that, where per-room stacking cannot.
///
/// Gives up after a few steps and returns the last position rather than sliding a badge halfway
/// across the map: a badge belongs to the box side it names, and one that has wandered says less
/// than one that overlaps.
fn settle_badge(placer: &mut TextPlacer, at: (f64, f64), tangent: (f64, f64)) -> (f64, f64) {
    let mut p = at;
    for _ in 0..4 {
        if placer.is_free(badge_rect(p)) {
            break;
        }
        p = (p.0 + tangent.0 * 17.0, p.1 + tangent.1 * 17.0);
    }
    placer.block(badge_rect(p));
    p
}

/// The four places a short label can sit around an anchor whose outward normal is `u`: along the
/// normal first (the historical placement, and the one that reads as belonging to the anchor),
/// then back the other way, then above and below.
fn spots_around(at: (f64, f64), u: (f64, f64), gap: f64) -> Vec<Spot> {
    let away = |ux: f64, uy: f64| {
        let (x, y) = (at.0 + ux * gap, at.1 + uy * gap);
        Spot {
            x,
            y: y + if uy == 0.0 { 3.0 } else { 0.0 },
            anchor: if ux > 0.0 {
                Anchor::Start
            } else if ux < 0.0 {
                Anchor::End
            } else {
                Anchor::Middle
            },
        }
    };
    vec![away(u.0, u.1), away(-u.0, -u.1), away(0.0, -1.0), away(0.0, 1.0)]
}

/// The point on a room's box `side` — offset `gap` px clear of the edge along that side's outward
/// normal, and `step` px along the side itself — that every stub badge, ghost and arrival arrow
/// in this file anchors from. Successive stubs on one side pass increasing `step` so they stack
/// along it rather than landing on top of one another.
fn side_root((bx, by, bw, bh): PxRect, side: Side, gap: f64, step: f64) -> (f64, f64) {
    match side {
        Side::Top => (bx + bw / 2.0 + step, by - gap),
        Side::Bottom => (bx + bw / 2.0 + step, by + bh + gap),
        Side::Left => (bx - gap, by + bh / 2.0 + step),
        Side::Right => (bx + bw + gap, by + bh / 2.0 + step),
    }
}

// ── Cross-layer ghosts (SQ-1319) ─────────────────────────────────────────────────

/// A ghost's two lines of text: the room a crossing leads to (or arrives from), and the layer it
/// lives on. The two travel together because a ghost is never drawn with only one of them.
#[derive(Debug, Clone, Copy)]
struct GhostText<'a> {
    name: &'a str,
    layer: &'a str,
}

/// The ghost box's own size for `text`, in SVG pixels: two centred lines, the room name at
/// `GHOST_NAME_PX` and the layer name smaller beneath it at `GHOST_LAYER_PX`.
fn ghost_dims(text: GhostText<'_>) -> (f64, f64) {
    let w = (text.name.chars().count() as f64 * GHOST_NAME_ADV)
        .max(text.layer.chars().count() as f64 * GHOST_LAYER_ADV)
        + 2.0 * GHOST_PAD;
    let h = GHOST_NAME_PX + GHOST_LINE_GAP + GHOST_LAYER_PX + 2.0 * GHOST_PAD;
    (w, h)
}

/// The ghost box's rect were it placed `dist` px out from `anchor` along `u` — always one of the
/// four outward unit normals, so the box is never rotated: it sits centred on the anchor's other
/// axis and offset along `u`'s.
fn ghost_rect_at(anchor: (f64, f64), u: (f64, f64), dist: f64, w: f64, h: f64) -> PxRect {
    if u.1 == 0.0 {
        let x = if u.0 > 0.0 { anchor.0 + dist } else { anchor.0 - dist - w };
        (x, anchor.1 - h / 2.0, w, h)
    } else {
        let y = if u.1 > 0.0 { anchor.1 + dist } else { anchor.1 - dist - h };
        (anchor.0 - w / 2.0, y, w, h)
    }
}

/// The point on the ghost box's own near edge (the one facing the room) a connector line lands
/// on.
fn ghost_near_edge((x, y, w, h): PxRect, u: (f64, f64)) -> (f64, f64) {
    if u.1 == 0.0 {
        (if u.0 > 0.0 { x } else { x + w }, y + h / 2.0)
    } else {
        (x + w / 2.0, if u.1 > 0.0 { y } else { y + h })
    }
}

/// How many `GHOST_STEP` extensions [`search_ghost_side`] tries along the DIRECT side (`u`
/// itself) before giving up on it (SQ-1333): a room sitting squarely in that corridor blocks
/// every distance beyond it — `crosses`' bounding-box test only grows MORE true as the segment
/// lengthens through the room, never less — so an unbounded search there would never terminate
/// now that the line check is part of it. Six steps (~100px) is generous room for anything short
/// of that permanent block.
const GHOST_SIDE_TRIES: u32 = 6;

/// The same search's bound on a TANGENT side, tried only once the direct side above has given up.
/// A tangent isn't provably blocked forever the way the direct side can be — a real map's rooms
/// are finite, so sliding along a row eventually clears the last one and reaches open canvas —
/// so this is a generous hard backstop against a genuine bug looping forever, not a distance any
/// real map is expected to need (Anchorhead's dense house layer needed several hundred px here).
const GHOST_TANGENT_TRIES: u32 = 2000;

/// Where a ghost landed (SQ-1319; SQ-1333): its box, the axis its connector line runs along —
/// `u` when the direct side was clear, a tangent to it when a room forced the bend below — and,
/// only in the bent case, the point where the connector turns once on its way there.
struct GhostPlacement {
    rect: PxRect,
    dir: (f64, f64),
    bend: Option<(f64, f64)>,
}

/// The first spot along `dir`, starting `base_gap` out from `origin` and stepping by
/// `GHOST_STEP` up to `max_tries` times, whose box is clear of every room, badge, label and other
/// ghost (via `placer`) and every connector lane (`segments`), AND whose connector line back to
/// `origin` crosses no room and no other ghost box (SQ-1333) — `rooms`/`ghosts` are checked
/// against the line rather than folded into `placer`, since a connector may legally run near a
/// label or badge it would be wrong to route through a room or another ghost's box.
fn search_ghost_side(
    placer: &mut TextPlacer,
    segments: &[Segment],
    rooms: &[PxRect],
    ghosts: &[PxRect],
    origin: (f64, f64),
    dir: (f64, f64),
    base_gap: f64,
    w: f64,
    h: f64,
    max_tries: u32,
) -> Option<PxRect> {
    let mut dist = base_gap;
    for _ in 0..max_tries {
        let r = ghost_rect_at(origin, dir, dist, w, h);
        let near = ghost_near_edge(r, dir);
        let line_clear = !rooms.iter().any(|&room| crosses(origin, near, room))
            && !ghosts.iter().any(|&g| crosses(origin, near, g));
        if line_clear && placer.is_free(r) && !segments.iter().any(|&(a, b)| crosses(a, b, r)) {
            placer.block(r);
            return Some(r);
        }
        dist += GHOST_STEP;
    }
    None
}

/// Settle a ghost box clear of every room, badge, label and other ghost (via `placer`), every
/// connector lane (`segments`), and — SQ-1333 — clear of any room or ghost box its OWN connector
/// line would otherwise run through.
///
/// Tries the travel direction's own side first (`u`, SQ-1330's historical placement, bounded by
/// [`GHOST_SIDE_TRIES`] since a room directly ahead blocks every distance beyond it and never
/// clears). When that side is blocked, it swings to whichever of the two sides tangent to `u` a
/// room isn't on, joined to the anchor by a short stub along `u` and a single bend — the
/// connector still reads as leaving by the badge's own side before it turns; a tangent search is
/// bounded only by [`GHOST_TANGENT_TRIES`], generous enough to be effectively unbounded on a real
/// map. Only when every one of those fails does it fall back to the historical unbounded push
/// straight out along `u` with no line check (SQ-1319's "never dropped": a ghost that lands
/// somewhere, even crossing a room, beats one silently missing) — every caller folds the
/// returned rect, and the bend point when there is one, into `Extent`.
fn place_ghost(
    placer: &mut TextPlacer,
    segments: &[Segment],
    rooms: &[PxRect],
    ghosts: &[PxRect],
    anchor: (f64, f64),
    u: (f64, f64),
    base_gap: f64,
    w: f64,
    h: f64,
) -> GhostPlacement {
    if let Some(rect) =
        search_ghost_side(placer, segments, rooms, ghosts, anchor, u, base_gap, w, h, GHOST_SIDE_TRIES)
    {
        return GhostPlacement { rect, dir: u, bend: None };
    }

    // The direct side is blocked by a room for every distance (SQ-1333) — the stub itself must
    // also be clear, or the bend has nowhere honest to start from.
    let stub = (anchor.0 + u.0 * base_gap, anchor.1 + u.1 * base_gap);
    let stub_clear = !rooms.iter().any(|&room| crosses(anchor, stub, room))
        && !ghosts.iter().any(|&g| crosses(anchor, stub, g));
    if stub_clear {
        for tangent in [(-u.1, u.0), (u.1, -u.0)] {
            if let Some(rect) = search_ghost_side(
                placer, segments, rooms, ghosts, stub, tangent, GHOST_STEP, w, h, GHOST_TANGENT_TRIES,
            ) {
                return GhostPlacement { rect, dir: tangent, bend: Some(stub) };
            }
        }
    }

    let mut dist = base_gap + GHOST_STEP * GHOST_SIDE_TRIES as f64;
    loop {
        let r = ghost_rect_at(anchor, u, dist, w, h);
        if placer.is_free(r) && !segments.iter().any(|&(a, b)| crosses(a, b, r)) {
            placer.block(r);
            return GhostPlacement { rect: r, dir: u, bend: None };
        }
        dist += GHOST_STEP;
    }
}

/// Render one ghost box at `rect`, joined to `anchor` by a short connector line along `dir`
/// (SQ-1319): a passage that leaves the layer always says where it goes, at both ends, and never
/// drops the name for want of room — see `place_ghost`. `arrival` distinguishes the mirror drawn
/// on a one-way crossing's arriving end (see `arrival_ghosts`), which carries no departure letter
/// of its own. `bend`, when `Some` (SQ-1333), draws the connector as two segments — `anchor` to
/// the bend, then the bend to the ghost's near edge — rather than one straight line, for the case
/// where the direct line would have crossed a room.
///
/// Draws no arrowhead itself (SQ-1330): a departure's caller places one at the ghost's own near
/// edge (pointing further in) and an arrival's caller places one at the room's own edge (pointing
/// further in there instead) — the two ends of one passage, never both on the same box.
fn draw_ghost(
    anchor: (f64, f64),
    dir: (f64, f64),
    bend: Option<(f64, f64)>,
    rect: PxRect,
    text: GhostText<'_>,
    arrival: bool,
) -> String {
    let (x, y, w, h) = rect;
    let near = ghost_near_edge(rect, dir);
    let cls = if arrival { "ghost arrival" } else { "ghost" };
    let name_y = y + GHOST_PAD + GHOST_NAME_PX * 0.8;
    let layer_y = name_y + GHOST_LINE_GAP + GHOST_LAYER_PX * 0.8;
    let line = |a: (f64, f64), b: (f64, f64)| {
        format!(
            "<line class=\"ghost-line\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"/>",
            f(a.0),
            f(a.1),
            f(b.0),
            f(b.1)
        )
    };
    let lines = match bend {
        Some(b) => line(anchor, b) + &line(b, near),
        None => line(anchor, near),
    };
    format!(
        "{lines}\
         <rect class=\"{cls}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"3\"/>\
         <text class=\"ghost-name\" text-anchor=\"middle\" x=\"{}\" y=\"{}\">{}</text>\
         <text class=\"ghost-layer\" text-anchor=\"middle\" x=\"{}\" y=\"{}\">{}</text>",
        f(x),
        f(y),
        f(w),
        f(h),
        f(x + w / 2.0),
        f(name_y),
        xml_escape(text.name),
        f(x + w / 2.0),
        f(layer_y),
        xml_escape(text.layer)
    )
}

// ── Legend ────────────────────────────────────────────────────────────────────

const LEGEND_W: i32 = 336;
const LEGEND_ROW: i32 = 15;

/// The legend rows: `(sample markup drawn at (0, 0), caption)`.
fn legend_rows() -> Vec<(String, &'static str)> {
    let line = |class: &str| {
        format!("<path class=\"{class}\" d=\"M 4 0 L 56 0\"/>")
    };
    vec![
        (
            // SQ-1346: the head sits at the DESTINATION end, same as a two-way's own arrival
            // head (below) — a one-way's line just never grows the matching head at its origin.
            format!("{}{}", line("edge oneway"), arrowhead((54.0, 0.0), (1.0, 0.0), "arrow")),
            "one-way passage — the arrow points where it leads",
        ),
        (
            format!(
                "{}{}{}",
                line("edge reciprocal"),
                arrowhead((6.0, 0.0), (-1.0, 0.0), "arrow"),
                arrowhead((54.0, 0.0), (1.0, 0.0), "arrow")
            ),
            "two-way passage",
        ),
        (
            format!("{}{}", line("edge oneway"), door_mark((30.0, 0.0), (1.0, 0.0))),
            "door — a way through that must be opened",
        ),
        (line("edge conditional"), "conditional exit — the story gates it"),
        (line("edge distorted"), "distorted — drawn out of true"),
        (
            format!("{}{}", line("edge portal"), badge((30.0, 0.0), "U")),
            "up / down (U D I O = the way you travel)",
        ),
        (
            format!(
                "{}<text class=\"random\" x=\"26\" y=\"3\">?</text>",
                "<line class=\"edge stub\" x1=\"4\" y1=\"0\" x2=\"22\" y2=\"0\"/>"
            ),
            "random exit — destination varies",
        ),
        (
            "<rect class=\"room current\" x=\"12\" y=\"-6\" width=\"36\" height=\"12\" rx=\"3\"/>".to_string(),
            "the room you are in",
        ),
        (
            format!(
                "<line class=\"ghost-line\" x1=\"2\" y1=\"0\" x2=\"13\" y2=\"0\"/>\
                 {}\
                 <rect class=\"ghost\" x=\"13\" y=\"-8\" width=\"40\" height=\"16\" rx=\"3\"/>\
                 <text class=\"ghost-name\" text-anchor=\"middle\" x=\"33\" y=\"-1\">Studio</text>\
                 <text class=\"ghost-layer\" text-anchor=\"middle\" x=\"33\" y=\"6\">Main</text>",
                arrowhead_inward((13.0, 0.0), (-1.0, 0.0), "arrow")
            ),
            "exit to another layer — arrow shows the way you travel",
        ),
    ]
}

/// The x each row's caption is drawn at, and the right margin left past its longest line.
const LEGEND_TEXT_X: i32 = 80;
const LEGEND_TEXT_MARGIN: i32 = 10;

/// The legend block, drawn with its top-left at `(0, 0)`. Returns `(markup, width, height)`.
///
/// The panel is at least `LEGEND_W` wide, but a caption longer than that (SQ-1344: "exit to
/// another layer — arrow shows the way you travel" ran past the right edge) widens it — using the
/// same 9px-class character-width estimate `text_boxes()` charges every 9px `.legend` label, so
/// the two never disagree about how wide a row's text really is.
fn legend() -> (String, i32, i32) {
    let rows = legend_rows();
    let h = LEGEND_ROW * rows.len() as i32 + 34;
    let max_caption_w = rows
        .iter()
        .map(|(_, caption)| caption.chars().count() as f64 * 9.0 * 0.6125)
        .fold(0.0_f64, f64::max);
    let w = LEGEND_W.max((LEGEND_TEXT_X as f64 + max_caption_w).ceil() as i32 + LEGEND_TEXT_MARGIN);
    let mut s = format!(
        "<rect class=\"legend-panel\" x=\"0\" y=\"0\" width=\"{w}\" height=\"{h}\" rx=\"5\"/>\
         <text class=\"legend-title\" x=\"10\" y=\"15\">Legend</text>"
    );
    for (i, (sample, caption)) in rows.iter().enumerate() {
        let y = 26 + LEGEND_ROW * i as i32 + LEGEND_ROW / 2;
        let _ = write!(s, "<g transform=\"translate(6,{y})\">{sample}</g>");
        let _ = write!(
            s,
            "<text class=\"legend\" x=\"{LEGEND_TEXT_X}\" y=\"{}\">{}</text>",
            y + 3,
            xml_escape(caption)
        );
    }
    (s, w, h)
}

// ── Documents ─────────────────────────────────────────────────────────────────

/// Wrap a body of markup — already at its own `(0, 0)` — in a document, with the legend below
/// it in the bottom-left corner.
fn document(body: &str, body_w: i32, body_h: i32) -> String {
    let (leg, leg_w, leg_h) = legend();
    let width = 2 * MARGIN + body_w.max(leg_w);
    let height = 2 * MARGIN + body_h + 12 + leg_h;
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" \
         viewBox=\"0 0 {width} {height}\">{}\
         <rect class=\"bg\" width=\"{width}\" height=\"{height}\"/>\
         <g class=\"map-block\" transform=\"translate({MARGIN},{MARGIN})\">{body}</g>\
         <g class=\"legend-block\" transform=\"translate({MARGIN},{})\">{leg}</g></svg>",
        stylesheet(),
        MARGIN + body_h + 12
    )
}

/// Render a `RenderMap` to a standalone SVG document string.
///
/// Passage weights (SQ-1312) are unknown without the graph — see [`render_svg_of`], which the
/// exports that have one call.
///
/// Empty map (no rooms): returns a minimal valid `<svg></svg>`.
pub fn render_svg(rm: &RenderMap) -> String {
    render_svg_of(rm, None)
}

/// [`render_svg`], with the graph the map was rendered from so each passage can be drawn at its
/// own weight: a door marked, a conditional exit dotted (SQ-1312/SQ-1313).
pub fn render_svg_of(rm: &RenderMap, graph: Option<&MapGraph>) -> String {
    let weights = graph.map(weight_table).unwrap_or_default();
    // No layer context here, so no cross-layer arrivals to draw (see `render_svg_layered`, which
    // is the only caller that ever has a non-empty one).
    let arrivals = HashMap::new();
    let Some((body, w, h)) = render_svg_body(rm, &weights, &arrivals) else {
        return "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"></svg>".to_string();
    };
    document(&body, w, h)
}

/// Render every non-empty layer of `graph` as one standalone SVG document, each layer its own
/// coordinate plane framed in its own panel and stacked top-to-bottom, a heading naming it in the
/// panel's own title bar (SQ-1308, panels SQ-1343) — the same rule [`crate::map_dump::render_dump`]
/// draws its ASCII map by.
///
/// [`render_svg`] draws a single [`RenderMap`] on one shared canvas with no notion of layer at
/// all, which [`mapper::render::render`] (as opposed to [`mapper::render::render_layer`]) never
/// distinguishes either — safe only when there is exactly one layer. A room peeled onto a fresh
/// layer keeps whatever cell it already had on the layer it left
/// ([`mapper::layer::move_region`]'s doc comment), so two rooms on different layers can and
/// routinely do share a cell; drawing every layer on one canvas would then draw them on top of
/// each other. Stacking each layer's own [`mapper::render::render_layer`] output avoids that by
/// construction, since each one gets its own canvas.
///
/// Every panel is framed at the SAME width — the widest layer's fragment or heading, plus
/// `FRAME_PAD` on each side — so the stack reads as a column of equal-width panels rather than a
/// ragged one, which is why this is two passes: the first measures every layer before the second
/// draws any of them.
///
/// A single-layer graph renders exactly as `render_svg_of(&render(graph), Some(graph))`.
pub fn render_svg_layered(graph: &MapGraph) -> String {
    let mut layers: Vec<mapper::layer::LayerId> = graph
        .layers()
        .keys()
        .copied()
        .filter(|&l| !graph.rooms_in_layer(l).is_empty())
        .collect();
    layers.sort_unstable();
    if layers.len() <= 1 {
        return render_svg_of(&mapper::render::render(graph), Some(graph));
    }

    const HEADING_H: i32 = 26;
    const FRAME_PAD: i32 = 12;
    const PANEL_GAP: i32 = 16;
    let weights = weight_table(graph);

    // Pass 1: render every layer's own heading and fragment, and find the widest of either —
    // nothing is emitted yet, because the panel width below has to be settled first.
    type Panel = (String, Option<(String, i32, i32)>);
    let mut panels: Vec<Panel> = Vec::with_capacity(layers.len());
    let mut max_w = 0;
    for &l in &layers {
        let rm = mapper::render::render_layer(graph, l);
        let heading = format!(
            "{}{} ({} rooms)",
            graph.layer_name(l),
            if graph.layer_is_maze(l) { " [maze]" } else { "" },
            graph.rooms_in_layer(l).len()
        );
        max_w = max_w.max(heading.chars().count() as i32 * 9);
        let arrivals = arrival_ghosts(graph, l);
        let frag = render_svg_body(&rm, &weights, &arrivals);
        if let Some((_, w, _)) = &frag {
            max_w = max_w.max(*w);
        }
        panels.push((heading, frag));
    }
    let panel_w = max_w + 2 * FRAME_PAD;

    // Pass 2: emit each layer inside one panel of that shared width, stacked top-to-bottom.
    let mut y = 0i32;
    let mut body = String::new();
    for (heading, frag) in &panels {
        let h = frag.as_ref().map(|&(_, _, h)| h).unwrap_or(0);
        let panel_h = HEADING_H + FRAME_PAD + h + FRAME_PAD;
        let _ = write!(
            body,
            "<g transform=\"translate(0,{y})\">\
             <rect class=\"layer-frame\" x=\"0\" y=\"0\" width=\"{panel_w}\" height=\"{panel_h}\" rx=\"6\"/>\
             <text class=\"heading\" x=\"{FRAME_PAD}\" y=\"18\">{}</text>",
            xml_escape(heading)
        );
        if let Some((frag, _, _)) = frag {
            let _ = write!(body, "<g transform=\"translate({FRAME_PAD},{})\">{frag}</g>", HEADING_H + FRAME_PAD);
        }
        body.push_str("</g>");
        y += panel_h + PANEL_GAP;
    }
    document(&body, panel_w.max(1), y.max(1))
}

/// Write `render_svg_of(rm, graph)` to the file at `path`.
pub fn export_svg(path: &Path, rm: &RenderMap, graph: Option<&MapGraph>) -> std::io::Result<()> {
    crate::storage::atomic_write(path, render_svg_of(rm, graph).as_bytes())
}

// ── Reading the drawing back out of the document ────────────────────────────

/// The `class` of `node` or any of its ancestors names `want`.
pub fn under_class(node: roxmltree::Node<'_, '_>, want: &str) -> bool {
    std::iter::successors(Some(node), |n| n.parent())
        .any(|n| n.attribute("class").unwrap_or("").split_whitespace().any(|c| c == want))
}

/// Every `<path class="edge …">`/`<line class="edge …">` segment in `svg`, as pixel
/// endpoint pairs in the document's own coordinate space.
///
/// The legend is excluded: it draws a SAMPLE of every mark the map can carry, and counting
/// those as drawn passages would make every measurement of the drawing wrong by a constant.
///
/// Parses the emitted document rather than re-deriving the geometry, so the assertion is
/// about what a viewer actually draws.
fn edge_segments(svg: &str) -> Vec<((f64, f64), (f64, f64))> {
    let doc = roxmltree::Document::parse(svg).expect("well-formed SVG");
    let mut out = Vec::new();
    for node in doc.descendants() {
        let cls = node.attribute("class").unwrap_or("");
        if !cls.split_whitespace().any(|c| c == "edge") || under_class(node, "legend-block") {
            continue;
        }
        let offset = translate_of(node);
        match node.tag_name().name() {
            "line" => {
                let g = |a: &str| node.attribute(a).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                out.push((
                    (g("x1") + offset.0, g("y1") + offset.1),
                    (g("x2") + offset.0, g("y2") + offset.1),
                ));
            }
            "path" => {
                let pts = path_points(node.attribute("d").unwrap_or(""));
                for w in pts.windows(2) {
                    out.push(((w[0].0 + offset.0, w[0].1 + offset.1), (w[1].0 + offset.0, w[1].1 + offset.1)));
                }
            }
            _ => {}
        }
    }
    out
}

/// Every `<line class="ghost-line">` segment in `svg`, as pixel endpoint pairs in the document's
/// own coordinate space — a ghost's connector, one or two segments per ghost when [`draw_ghost`]
/// drew a bend (SQ-1333). The legend is excluded, same as [`edge_segments`].
fn ghost_line_segments(svg: &str) -> Vec<((f64, f64), (f64, f64))> {
    let doc = roxmltree::Document::parse(svg).expect("well-formed SVG");
    let mut out = Vec::new();
    for node in doc.descendants() {
        let cls = node.attribute("class").unwrap_or("");
        if node.tag_name().name() != "line"
            || !cls.split_whitespace().any(|c| c == "ghost-line")
            || under_class(node, "legend-block")
        {
            continue;
        }
        let offset = translate_of(node);
        let g = |a: &str| node.attribute(a).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        out.push(((g("x1") + offset.0, g("y1") + offset.1), (g("x2") + offset.0, g("y2") + offset.1)));
    }
    out
}

/// The accumulated `translate(x,y)` of every ancestor of `node`, including itself.
fn translate_of(node: roxmltree::Node<'_, '_>) -> (f64, f64) {
    let mut acc = (0.0, 0.0);
    let mut cur = Some(node);
    while let Some(n) = cur {
        if let Some(t) = n.attribute("transform") {
            if let Some(args) = t.strip_prefix("translate(").and_then(|s| s.strip_suffix(')')) {
                let mut it = args.split(',');
                let x: f64 = it.next().unwrap_or("0").trim().parse().unwrap_or(0.0);
                let y: f64 = it.next().unwrap_or("0").trim().parse().unwrap_or(0.0);
                acc = (acc.0 + x, acc.1 + y);
            }
        }
        cur = n.parent();
    }
    acc
}

/// The vertices of an `M/L/Q` path — a `Q`'s control point is the corner it rounds, so it
/// is the vertex the un-rounded polyline had, and the endpoint after it is on the run.
fn path_points(d: &str) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let mut toks = d.split_whitespace().peekable();
    while let Some(t) = toks.next() {
        let take = |toks: &mut std::iter::Peekable<std::str::SplitWhitespace<'_>>, n: usize| {
            let mut v = Vec::new();
            for _ in 0..n {
                v.push(toks.next().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0));
            }
            v
        };
        match t {
            "M" | "L" => {
                let v = take(&mut toks, 2);
                out.push((v[0], v[1]));
            }
            "Q" => {
                let v = take(&mut toks, 4);
                out.push((v[0], v[1]));
                out.push((v[2], v[3]));
            }
            _ => {}
        }
    }
    out
}

/// Every `<rect class="room …">` in `svg`, in the document's own coordinate space.
fn room_rects(svg: &str) -> Vec<(f64, f64, f64, f64)> {
    let doc = roxmltree::Document::parse(svg).expect("well-formed SVG");
    doc.descendants()
        .filter(|n| {
            n.tag_name().name() == "rect"
                && n.attribute("class").unwrap_or("").split_whitespace().any(|c| c == "room")
                && !under_class(*n, "legend-block")
        })
        .map(|n| {
            let g = |a: &str| n.attribute(a).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
            let o = translate_of(n);
            (g("x") + o.0, g("y") + o.1, g("width"), g("height"))
        })
        .collect()
}

/// True when segment `a→b` passes strictly through the interior of `rect`.
///
/// The segments are axis-aligned by construction (the router only ever turns at right
/// angles), so this is a 1-D overlap on each axis rather than a general clipper. A rounded
/// corner's `Q` legs are the only near-diagonal pieces and they live at a turn, at most
/// `CORNER_R` from a vertex that is itself outside every box — the shrink below is what
/// keeps a legitimate anchor ON the border from reading as a crossing.
fn crosses(a: (f64, f64), b: (f64, f64), rect: (f64, f64, f64, f64)) -> bool {
    let (x, y, w, h) = rect;
    let eps = 0.6;
    let (rx0, ry0, rx1, ry1) = (x + eps, y + eps, x + w - eps, y + h - eps);
    let (sx0, sx1) = (a.0.min(b.0), a.0.max(b.0));
    let (sy0, sy1) = (a.1.min(b.1), a.1.max(b.1));
    sx1 > rx0 && sx0 < rx1 && sy1 > ry0 && sy0 < ry1
}

/// Every drawn connector segment in `svg` that passes through a room box, described.
///
/// Empty for a well-formed export, and that is the invariant the lane arrangement exists to
/// keep: a connection you cannot follow because it vanishes under a room box is not drawn.
/// **Public so a real story's generated map can be checked by the same code the unit cases
/// use** — the synthetic graphs in this file cannot produce the lane pressure a hundred rooms
/// do (`sq1306_mapgen`'s Zork I case is the one that can).
pub fn connector_room_crossings(svg: &str) -> Vec<String> {
    let rooms = room_rects(svg);
    let mut out = Vec::new();
    for (a, b) in edge_segments(svg) {
        for r in &rooms {
            if crosses(a, b, *r) {
                out.push(format!("segment {a:?} → {b:?} crosses room rect {r:?}"));
            }
        }
    }
    out
}

/// Every `<line class="ghost-line">` in `svg` that passes through a room box (SQ-1333): a
/// ghost's connector is part of its footprint just as much as its box, and one running through a
/// room reads as that room's own exit — see `place_ghost`, which now keeps this empty by
/// construction rather than only the box being clear (`ghost_box_overlaps`).
///
/// **Public so a real story's generated map can be checked by the same code the unit cases
/// use**, the way [`connector_room_crossings`] is: the synthetic graphs in this file cannot
/// produce the badge-side pressure a hundred rooms do.
pub fn ghost_line_room_crossings(svg: &str) -> Vec<String> {
    let rooms = room_rects(svg);
    let mut out = Vec::new();
    for (a, b) in ghost_line_segments(svg) {
        for r in &rooms {
            if crosses(a, b, *r) {
                out.push(format!("ghost line {a:?} → {b:?} crosses room rect {r:?}"));
            }
        }
    }
    out
}

/// Every gap between two SAME-ROW or SAME-COLUMN room boxes in `svg` narrower than
/// `MIN_CHANNEL_PX` (SQ-1322): the SVG's own pixel floor for a channel already at the shared
/// `MIN_GUTTER` cell minimum, sized so a two-way passage between the two rooms shows a real shaft
/// between its two inward-pointing heads rather than the two meeting or overlapping (a "bowtie",
/// `◄►`) — see `PxAxis`.
///
/// Geometric, not routing-aware: any two room boxes at the same height (a row) or the same
/// width-and-x (a column) with nothing between them are exactly one channel apart, whether or not
/// a connector actually routes through it — `PxAxis` widens every minimum channel
/// unconditionally, so this needs no connector attribution to check, only room positions.
///
/// **Public so a real story's generated map can be checked by the same code the unit cases
/// use** — the synthetic graphs in this file cannot produce a hundred rooms' worth of adjacent
/// pairs (`sq1306_mapgen`'s Zork I and Anchorhead cases are what can).
pub fn narrow_channel_gaps(svg: &str) -> Vec<String> {
    let rooms = room_rects(svg);
    let close = |a: f64, b: f64| (a - b).abs() < 0.5;
    let mut out = Vec::new();

    let mut by_row: Vec<&PxRect> = rooms.iter().collect();
    by_row.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap().then(a.0.partial_cmp(&b.0).unwrap()));
    for w in by_row.windows(2) {
        let (a, b) = (w[0], w[1]);
        if close(a.1, b.1) && close(a.3, b.3) {
            let gap = b.0 - (a.0 + a.2);
            if gap > 0.0 && gap < MIN_CHANNEL_PX - 0.5 {
                out.push(format!("row gap {gap} between {a:?} and {b:?} is narrower than {MIN_CHANNEL_PX}"));
            }
        }
    }
    let mut by_col: Vec<&PxRect> = rooms.iter().collect();
    by_col.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap().then(a.1.partial_cmp(&b.1).unwrap()));
    for w in by_col.windows(2) {
        let (a, b) = (w[0], w[1]);
        if close(a.0, b.0) && close(a.2, b.2) {
            let gap = b.1 - (a.1 + a.3);
            if gap > 0.0 && gap < MIN_CHANNEL_PX - 0.5 {
                out.push(format!("column gap {gap} between {a:?} and {b:?} is narrower than {MIN_CHANNEL_PX}"));
            }
        }
    }
    out
}

/// One `<text>` the map draws: its `class`, its content, and its approximate box in document
/// coordinates. The three travel together because a collision report is useless without all
/// of them — a rectangle nobody can name says only that something is somewhere.
type LabelBox = (String, String, PxRect);

/// Every `<text>` the MAP draws, as `(class, content, bounding box)` in document coordinates.
///
/// The box is approximate — SVG carries no metrics and the document asks for a monospace stack,
/// so a character is charged `0.6125 * font-size` wide, the same ratio the badge pass has always
/// used to grow the drawing's extent. That is enough to tell a label sitting ON a room name from
/// one sitting beside it, which is the question (SQ-1317).
///
/// The legend is excluded, as everywhere else here: it draws a sample of every mark the map can
/// carry, in a panel of its own.
fn text_boxes(svg: &str) -> Vec<LabelBox> {
    let doc = roxmltree::Document::parse(svg).expect("well-formed SVG");
    let mut out = Vec::new();
    for node in doc.descendants() {
        if node.tag_name().name() != "text" || under_class(node, "legend-block") {
            continue;
        }
        let cls = node.attribute("class").unwrap_or("").to_string();
        if cls.starts_with("heading") || cls.starts_with("legend") {
            continue;
        }
        // `badge-text` is the letter INSIDE a badge circle, not a placed label: it goes wherever
        // its badge goes, and `settle_badge` is what keeps the badge off rooms and other badges.
        // Charging it a monospace box would also mismeasure it — a two-letter badge like `NW`
        // overhangs its own 13px circle, which is a cosmetic matter for the badge, not a
        // collision for the label placer to solve.
        if cls.starts_with("badge-text") {
            continue;
        }
        let text: String = node.text().unwrap_or("").to_string();
        if text.is_empty() {
            continue;
        }
        let px: f64 = match cls.split_whitespace().next().unwrap_or("") {
            "room-label" => 11.0,
            "random" => 9.0,
            "ghost-layer" => GHOST_LAYER_PX,
            _ => 8.0,
        };
        let offset = translate_of(node);
        let g = |a: &str| node.attribute(a).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        let (x, y) = (g("x") + offset.0, g("y") + offset.1);
        let w = text.chars().count() as f64 * px * 0.6125;
        let x = match node.attribute("text-anchor").unwrap_or("start") {
            "middle" => x - w / 2.0,
            "end" => x - w,
            _ => x,
        };
        out.push((cls, text, (x, y - px * 0.8, w, px)));
    }
    out
}

/// Every label the map draws over something it must not: another label, or a room box that is
/// not its own (SQ-1317).
///
/// A room's NAME is drawn inside its box and is the one label a box legitimately holds, so a
/// `room-label` is exempt from the box test — everything else (a direction tag, a cross-layer
/// badge's destination name, a `?` count) is not.
///
/// **Public so a real story's generated map can be checked by the same code the unit cases use**,
/// the way [`connector_room_crossings`] is: the synthetic graphs in this file cannot produce the
/// label pressure a hundred rooms do.
pub fn label_collisions(svg: &str) -> Vec<String> {
    let rooms = room_rects(svg);
    let texts = text_boxes(svg);
    let hits = |a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)| {
        a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
    };
    let mut out = Vec::new();
    for (cls, text, r) in &texts {
        if cls.split_whitespace().next() == Some("room-label") {
            continue;
        }
        for room in &rooms {
            if hits(*r, *room) {
                out.push(format!("{cls:?} {text:?} at {r:?} sits on room box {room:?}"));
            }
        }
    }
    for i in 0..texts.len() {
        for j in (i + 1)..texts.len() {
            if hits(texts[i].2, texts[j].2) {
                out.push(format!(
                    "{:?} {:?} overlaps {:?} {:?}",
                    texts[i].0, texts[i].1, texts[j].0, texts[j].1
                ));
            }
        }
    }
    out
}

/// Every ghost BOX in `svg` — the dashed `<rect class="ghost">`/`<rect class="ghost arrival">`, not
/// its text — that overlaps a room or another ghost (SQ-1319).
///
/// [`label_collisions`] already catches a ghost's own TEXT running onto a room or a neighbouring
/// label, since the text is measured like any other. It cannot see the box's own padding, though
/// — the rect is bigger than the text it holds — so this checks the rect directly, the way
/// [`connector_room_crossings`] checks a connector's drawn line rather than inferring it from
/// something else.
pub fn ghost_box_overlaps(svg: &str) -> Vec<String> {
    let rooms = room_rects(svg);
    let doc = roxmltree::Document::parse(svg).expect("well-formed SVG");
    let mut ghosts: Vec<PxRect> = Vec::new();
    for node in doc.descendants() {
        if node.tag_name().name() != "rect" || under_class(node, "legend-block") {
            continue;
        }
        if !node.attribute("class").unwrap_or("").split_whitespace().any(|c| c == "ghost") {
            continue;
        }
        let off = translate_of(node);
        let g = |a: &str| node.attribute(a).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        ghosts.push((g("x") + off.0, g("y") + off.1, g("width"), g("height")));
    }
    let hits = |a: PxRect, b: PxRect| a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3;
    let mut out = Vec::new();
    for g in &ghosts {
        for r in &rooms {
            if hits(*g, *r) {
                out.push(format!("ghost box {g:?} sits on room box {r:?}"));
            }
        }
    }
    for i in 0..ghosts.len() {
        for j in (i + 1)..ghosts.len() {
            if hits(ghosts[i], ghosts[j]) {
                out.push(format!("ghost box {:?} overlaps ghost box {:?}", ghosts[i], ghosts[j]));
            }
        }
    }
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-state"))]
mod tests {
    use super::*;
    use mapper::direction::Direction;
    use mapper::mapper::Mapper;
    use mapper::render::render;

    /// The MAP's own markup, with no legend and no document chrome — what a count of arrowheads
    /// or a search for a class has to be made against, since the legend draws a sample of every
    /// mark the map can carry and would otherwise answer every such question by itself.
    fn body_of(rm: &RenderMap, graph: Option<&MapGraph>) -> String {
        let weights = graph.map(weight_table).unwrap_or_default();
        render_svg_body(rm, &weights, &HashMap::new()).expect("a non-empty map").0
    }

    /// [`render_svg_layered`]'s document, up to (not including) the legend block — what a count
    /// of a mark drawn on the MAP has to be made against, since the legend draws one sample of
    /// every mark (a ghost included, SQ-1319) and `render_svg_layered` has no `body_of` to call
    /// instead (it stacks several panels' own bodies, not one `RenderMap`'s).
    fn layered_map_only(svg: &str) -> &str {
        svg.split("<g class=\"legend-block\"").next().unwrap_or(svg)
    }

    /// Every `<rect class="...">` in `svg` whose class attribute is EXACTLY `class` (excluding
    /// the legend's own sample), in the document's own coordinate space — an exact match tells a
    /// departure ghost (`"ghost"`) apart from an arrival one (`"ghost arrival"`), the same way
    /// `map.matches("class=\"ghost\"")` above already does (SQ-1330).
    fn ghost_rects_of(svg: &str, class: &str) -> Vec<(f64, f64, f64, f64)> {
        let doc = roxmltree::Document::parse(svg).expect("well-formed SVG");
        doc.descendants()
            .filter(|n| {
                n.tag_name().name() == "rect"
                    && n.attribute("class") == Some(class)
                    && !under_class(*n, "legend-block")
            })
            .map(|n| {
                let g = |a: &str| n.attribute(a).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                let o = translate_of(n);
                (g("x") + o.0, g("y") + o.1, g("width"), g("height"))
            })
            .collect()
    }

    /// The tip (first vertex) of every non-legend `<polygon class="arrow">` in `svg`, in the
    /// document's own coordinate space (SQ-1330).
    fn arrow_tips(svg: &str) -> Vec<(f64, f64)> {
        let doc = roxmltree::Document::parse(svg).expect("well-formed SVG");
        doc.descendants()
            .filter(|n| {
                n.tag_name().name() == "polygon"
                    && n.attribute("class").unwrap_or("").split_whitespace().any(|c| c == "arrow")
                    && !under_class(*n, "legend-block")
            })
            .map(|n| {
                let o = translate_of(n);
                let first =
                    n.attribute("points").unwrap_or("0,0").split_whitespace().next().unwrap_or("0,0");
                let (a, b) = first.split_once(',').unwrap_or(("0", "0"));
                (a.parse::<f64>().unwrap_or(0.0) + o.0, b.parse::<f64>().unwrap_or(0.0) + o.1)
            })
            .collect()
    }

    /// True if `p` sits within `tol` px of one of `rect`'s four edges (and within the OTHER
    /// axis's span, so a point merely level with an edge's infinite line doesn't count).
    fn near_rect_edge(p: (f64, f64), rect: (f64, f64, f64, f64), tol: f64) -> bool {
        let (x, y, w, h) = rect;
        let on_x_edge = ((p.0 - x).abs() < tol || (p.0 - (x + w)).abs() < tol)
            && p.1 > y - tol
            && p.1 < y + h + tol;
        let on_y_edge = ((p.1 - y).abs() < tol || (p.1 - (y + h)).abs() < tol)
            && p.0 > x - tol
            && p.0 < x + w + tol;
        on_x_edge || on_y_edge
    }

    /// The Zork-house shape used by the layout tests: a ring of rooms with a couple of
    /// diagonals and a vertical, which is enough to exercise lanes, corners and portals.
    fn zork_house() -> Mapper {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "North of House", Some(Direction::N));
        m.observe(3, "Behind House", Some(Direction::E));
        m.observe(4, "South of House", Some(Direction::S));
        m.observe(1, "West of House", Some(Direction::W));
        m.observe(3, "Behind House", Some(Direction::NE));
        m.observe(5, "Kitchen", Some(Direction::E));
        m.observe(3, "Behind House", Some(Direction::W));
        m.observe(5, "Kitchen", Some(Direction::E));
        m.observe(6, "Living Room", Some(Direction::W));
        m.observe(5, "Kitchen", Some(Direction::E));
        m.observe(7, "Attic", Some(Direction::Up));
        m
    }

    #[test]
    fn svg_contains_rooms_and_edges() {
        let mut m = Mapper::default();
        m.observe(1, "Start <Room>", None); // XML-special char in label
        m.observe(2, "North", Some(Direction::N));
        let svg = render_svg(&render(&m.graph));
        assert!(svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
        assert!(svg.contains("class=\"room\"") || svg.contains("class=\"room current\""));
        assert!(svg.contains("&lt;Room&gt;")); // label XML-escaped
        assert!(svg.contains("class=\"edge")); // a connector
    }

    #[test]
    fn empty_map_returns_valid_svg() {
        use mapper::graph::MapGraph;
        let g = MapGraph::new();
        let svg = render_svg(&render(&g));
        assert!(svg.contains("<svg"), "must open svg tag");
        assert!(svg.contains("</svg>"), "must close svg tag");
        assert!(!svg.contains("<rect"), "no rooms expected");
    }

    #[test]
    fn document_is_well_formed_xml() {
        let m = zork_house();
        let svg = render_svg_of(&render(&m.graph), Some(&m.graph));
        roxmltree::Document::parse(&svg).expect("the export must be well-formed XML");
    }

    #[test]
    fn no_connector_segment_crosses_a_room_box() {
        let m = zork_house();
        let svg = render_svg_of(&render(&m.graph), Some(&m.graph));
        assert!(!room_rects(&svg).is_empty(), "the case must actually draw some rooms");
        let bad = connector_room_crossings(&svg);
        assert!(bad.is_empty(), "connectors must not run through room boxes: {bad:?}");
    }

    #[test]
    fn a_reciprocal_pair_draws_two_arrowheads_and_a_one_way_draws_one() {
        // Reciprocal: A —E→ B and B —W→ A collapse to one connector with an arrow at each end.
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        m.observe(1, "A", Some(Direction::W));
        let recip = body_of(&render(&m.graph), None);
        assert_eq!(
            recip.matches("class=\"arrow\"").count(),
            2,
            "a reciprocal pair carries an arrowhead at both ends"
        );
        assert!(recip.contains("edge") && recip.contains("reciprocal"));

        // One-way: only A —E→ B is known, so only A's own exit is arrowed.
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        let one = body_of(&render(&m.graph), None);
        assert_eq!(
            one.matches("class=\"arrow\"").count(),
            1,
            "a one-way passage carries one arrowhead — the line ending bare IS the reading"
        );
        assert!(one.contains("oneway"), "and says so in its class");
    }


    /// SQ-1317: between ADJACENT boxes a two-way passage must not read as a bowtie.
    ///
    /// The channel there is a few pixels wide, and two heads pointed outward — the way a one-way's
    /// single head points, which is that room's own exit (SQ-0688) — come nose to nose in the
    /// middle of it. So a reciprocal's heads point INTO the boxes: one line, a head at each box
    /// end, which is also what the legend has always drawn for two-way.
    ///
    /// Measured off the emitted geometry rather than the source: each head's TIP (the first
    /// vertex `arrowhead` writes) must sit on a box edge, and its base must be out in the channel
    /// on the far side of that tip from the box.
    #[test]
    fn a_two_way_passage_between_adjacent_boxes_points_its_heads_into_the_rooms() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        m.observe(1, "A", Some(Direction::W));
        let svg = render_svg_of(&render(&m.graph), Some(&m.graph));
        let rooms = room_rects(&svg);
        assert_eq!(rooms.len(), 2, "the case must draw exactly the two adjacent rooms");

        let doc = roxmltree::Document::parse(&svg).expect("well-formed SVG");
        let arrows: Vec<Vec<(f64, f64)>> = doc
            .descendants()
            .filter(|n| {
                n.tag_name().name() == "polygon"
                    && n.attribute("class").unwrap_or("").split_whitespace().any(|c| c == "arrow")
                    && !under_class(*n, "legend-block")
            })
            .map(|n| {
                let off = translate_of(n);
                n.attribute("points")
                    .unwrap_or("")
                    .split_whitespace()
                    .filter_map(|p| {
                        let (a, b) = p.split_once(',')?;
                        Some((a.parse::<f64>().ok()? + off.0, b.parse::<f64>().ok()? + off.1))
                    })
                    .collect()
            })
            .collect();
        assert_eq!(arrows.len(), 2, "a two-way passage carries a head at each end");

        // The two heads are between the boxes; they must not meet. `A`'s right edge and `B`'s
        // left edge bound the channel.
        let mut xs: Vec<f64> = rooms.iter().map(|r| r.0).collect();
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for tri in &arrows {
            assert_eq!(tri.len(), 3, "an arrowhead is a triangle");
            let tip = tri[0];
            let base_x = (tri[1].0 + tri[2].0) / 2.0;
            // The tip sits on one of the two box edges facing the channel...
            let on_edge = rooms
                .iter()
                .any(|&(x, _, w, _)| (tip.0 - x).abs() < 1.5 || (tip.0 - (x + w)).abs() < 1.5);
            assert!(on_edge, "a head's tip belongs on a box edge, got {tip:?} against {rooms:?}");
            // ...and its base is further out in the channel than its tip, i.e. it points INTO
            // the box. Two heads built this way can never meet, whatever the channel's width.
            let left_box = rooms.iter().any(|&(x, _, w, _)| (tip.0 - (x + w)).abs() < 1.5);
            if left_box {
                assert!(base_x > tip.0, "the left box's head must point back into it: {tri:?}");
            } else {
                assert!(base_x < tip.0, "the right box's head must point back into it: {tri:?}");
            }
        }
    }

    /// SQ-1322: between ADJACENT boxes, a two-way passage's shaft — the plain gap between the two
    /// heads' own flat backs — must be at least `2 * ARROW_HEAD_LEN`, not merely non-overlapping.
    /// SQ-1317 (above) fixed the OVERLAP; a channel that leaves the two backs a hair's breadth
    /// apart still reads as a bowtie (`◄►`), which is what this pins.
    #[test]
    fn a_two_way_passage_between_adjacent_boxes_shows_a_real_shaft() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        m.observe(1, "A", Some(Direction::W));
        let svg = render_svg_of(&render(&m.graph), Some(&m.graph));
        let doc = roxmltree::Document::parse(&svg).expect("well-formed SVG");
        let mut base_xs: Vec<f64> = doc
            .descendants()
            .filter(|n| {
                n.tag_name().name() == "polygon"
                    && n.attribute("class").unwrap_or("").split_whitespace().any(|c| c == "arrow")
                    && !under_class(*n, "legend-block")
            })
            .map(|n| {
                let off = translate_of(n);
                let pts: Vec<(f64, f64)> = n
                    .attribute("points")
                    .unwrap_or("")
                    .split_whitespace()
                    .filter_map(|p| {
                        let (a, b) = p.split_once(',')?;
                        Some((a.parse::<f64>().ok()? + off.0, b.parse::<f64>().ok()? + off.1))
                    })
                    .collect();
                (pts[1].0 + pts[2].0) / 2.0
            })
            .collect();
        assert_eq!(base_xs.len(), 2, "a two-way passage carries a head at each end");
        base_xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let shaft = base_xs[1] - base_xs[0];
        assert!(
            shaft >= 2.0 * ARROW_HEAD_LEN - 0.01,
            "shaft must be at least 2x an arrowhead's own length ({}), got {shaft}",
            2.0 * ARROW_HEAD_LEN
        );

        // The same invariant, checked the way a real-game map's rooms (not its arrowheads) get
        // checked (`narrow_channel_gaps`) — the two must agree, since both describe one channel.
        let bad = narrow_channel_gaps(&svg);
        assert!(bad.is_empty(), "the channel itself must already meet the floor: {bad:?}");
    }

    /// SQ-1317: no label may sit on a room box or on another label.
    ///
    /// The bug this pins: a cross-layer badge's destination name was written left-anchored from a
    /// badge on the room's LEFT side, and left-anchored text runs RIGHT — straight back across the
    /// box, through the room's own name. Zork I drew `Maze` over `Cyclops Room`. Labels now state
    /// several places they would accept and take the first that is clear, and drop themselves
    /// rather than land on something (see `TextPlacer`).
    #[test]
    fn no_label_sits_on_a_room_or_another_label() {
        let m = zork_house();
        let svg = render_svg_of(&render(&m.graph), Some(&m.graph));
        assert!(!room_rects(&svg).is_empty(), "the case must actually draw some rooms");
        let bad = label_collisions(&svg);
        assert!(bad.is_empty(), "labels must stay clear: {bad:#?}");
    }

    /// The same rule with a cross-layer badge in play — the shape the Zork I defect had.
    ///
    /// A room on the LEFT of the map with a passage to a room peeled onto another layer gets a
    /// badge on its left side and a ghost box naming the room and layer beside it (SQ-1319; a
    /// plain floating caption before it). The box has to go left of the badge, away from the box,
    /// which is the placement the old outward-offset-with-start-anchor could not express.
    #[test]
    fn a_cross_layer_badge_name_does_not_run_back_across_its_room() {
        use mapper::layer::LayerId;
        let mut m = Mapper::default();
        m.observe(1, "Cyclops Room", None);
        m.observe(2, "Strange Passage", Some(Direction::E));
        m.observe(3, "Maze", Some(Direction::NW));
        let ids: Vec<_> = m.graph.rooms().map(|r| r.id).collect();
        let far = *ids.iter().find(|&&i| m.graph.room(i).unwrap().label() == "Maze").unwrap();
        let root = m.graph.layer_of(ids[0]);
        let other: LayerId = m.graph.new_layer(Some(root), "Maze".into());
        m.graph.set_room_layer(far, other);
        let svg = render_svg_layered(&m.graph);
        assert!(svg.contains("class=\"ghost\""), "the case must actually draw a cross-layer ghost");
        assert!(svg.contains(">Maze<"), "the ghost must name the room it leads to");
        let bad = label_collisions(&svg);
        assert!(bad.is_empty(), "a cross-layer ghost's text must stay off its room: {bad:#?}");
        let bad = ghost_box_overlaps(&svg);
        assert!(bad.is_empty(), "a cross-layer ghost box must stay off rooms and other ghosts: {bad:#?}");
    }

    /// SQ-1343: a two-layer graph draws exactly two `layer-frame` panels of equal width, each
    /// layer's heading sits inside its own panel, and each layer's room lies inside that same
    /// panel — never another layer's.
    #[test]
    fn a_two_layer_graph_frames_each_layer_in_an_equal_width_panel() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.set_pos(1, (0, 0));
        let below = g.new_layer(Some(mapper::layer::MAIN_LAYER), "Below".into());
        g.upsert_room(2, "Cellar".into());
        g.set_room_layer(2, below);
        g.set_pos(2, (0, 0));

        let svg = render_svg_layered(&g);
        let doc = roxmltree::Document::parse(&svg).expect("well-formed SVG");

        let mut frames: Vec<(f64, f64, f64, f64)> = doc
            .descendants()
            .filter(|n| n.tag_name().name() == "rect" && n.attribute("class") == Some("layer-frame"))
            .map(|n| {
                let o = translate_of(n);
                let a = |name: &str| n.attribute(name).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                (a("x") + o.0, a("y") + o.1, a("width"), a("height"))
            })
            .collect();
        assert_eq!(frames.len(), 2, "one frame per non-empty layer");
        frames.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        assert!(
            (frames[0].2 - frames[1].2).abs() < 0.01,
            "both panels must share the same width: {frames:?}"
        );

        let mut headings: Vec<(f64, f64, usize)> = doc
            .descendants()
            .filter(|n| n.tag_name().name() == "text" && n.attribute("class") == Some("heading"))
            .map(|n| {
                let o = translate_of(n);
                let a = |name: &str| n.attribute(name).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                (a("x") + o.0, a("y") + o.1, n.text().unwrap_or("").chars().count())
            })
            .collect();
        headings.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        assert_eq!(headings.len(), 2, "one heading per panel");
        for (i, &(x, y, chars)) in headings.iter().enumerate() {
            let (fx, fy, fw, fh) = frames[i];
            assert!(x >= fx && x <= fx + fw, "heading {i}'s x must sit inside its frame: {x} vs {frames:?}");
            assert!(y >= fy && y <= fy + fh, "heading {i}'s baseline must sit inside its frame: {y} vs {frames:?}");
            // Same estimate `render_svg_layered` sized the panel with.
            let right = x + chars as f64 * 9.0;
            assert!(
                right <= fx + fw + 0.01,
                "heading {i}'s estimated right edge must stay inside its frame: {right} vs {frames:?}"
            );
        }

        let mut rooms = room_rects(&svg);
        assert_eq!(rooms.len(), 2, "one room per layer");
        rooms.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        for (i, &(rx, ry, rw, rh)) in rooms.iter().enumerate() {
            let (fx, fy, fw, fh) = frames[i];
            assert!(
                rx >= fx && ry >= fy && rx + rw <= fx + fw && ry + rh <= fy + fh,
                "layer {i}'s room rect must lie inside its own frame: room {:?} frame {:?}",
                (rx, ry, rw, rh),
                frames[i]
            );
        }
    }

    // ── SQ-1319: ghosts at both ends, never dropped ──────────────────────────────────────

    /// A reciprocal Up/Down crossing between two layers: each panel draws its own OUTGOING
    /// ghost, which is the mirror the other panel needs — no separate "arrival" mechanism is
    /// required when the graph already carries a connection back.
    #[test]
    fn a_reciprocal_crossing_ghosts_both_panels_naming_the_partner() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Cellar".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 1));
        g.add_edge(1, Direction::Down, 2);
        g.add_edge(2, Direction::Up, 1);
        let below = g.new_layer(Some(mapper::layer::MAIN_LAYER), "Below".into());
        g.set_room_layer(2, below);
        let svg = render_svg_layered(&g);
        let map = layered_map_only(&svg);
        assert_eq!(map.matches("class=\"ghost\"").count(), 2, "one departure ghost per panel");
        assert_eq!(map.matches("class=\"ghost arrival\"").count(), 0, "neither end is one-way");
        assert!(svg.contains(">Cellar<"), "Hall's panel names the room it leads to");
        assert!(svg.contains(">Below<"), "Hall's panel names the layer it leads to");
        assert!(svg.contains(">Hall<"), "Cellar's panel names the room it leads back to");
        assert!(svg.contains(">Main<"), "Cellar's panel names the layer it leads back to");
        assert!(label_collisions(&svg).is_empty());
        assert!(ghost_box_overlaps(&svg).is_empty());

        // SQ-1330: a ghost pair is two one-ways, one per panel — each panel's departure ghost
        // carries its OWN arrowhead, at the ghost's own edge, never at the room's (that would
        // read as the two-way inward-head convention SQ-1317 reserves for adjacent room boxes,
        // and never as a single head pointing back at the room it started from).
        let deps = ghost_rects_of(&svg, "ghost");
        assert_eq!(deps.len(), 2, "one departure ghost per panel");
        let tips = arrow_tips(&svg);
        assert_eq!(tips.len(), 2, "each panel's departure ghost carries its own arrowhead");
        for &tip in &tips {
            assert!(
                deps.iter().any(|&r| near_rect_edge(tip, r, 1.5)),
                "arrowhead {tip:?} must sit on a departure ghost's own edge: {deps:?}"
            );
        }
        let rooms = room_rects(&svg);
        for &tip in &tips {
            assert!(
                !rooms.iter().any(|&r| near_rect_edge(tip, r, 1.5)),
                "a departure arrowhead must never sit on a room's own edge: {tip:?} vs {rooms:?}"
            );
        }
        assert!(svg.contains(">D<"), "Hall's own badge names the direction it travels");
        assert!(svg.contains(">U<"), "Cellar's own badge names the direction it travels back");
    }

    /// A ONE-WAY crossing has no connection back, so `interlayer_badges` never fires on the
    /// arriving side — the far panel would otherwise say nothing at all about it. The arriving
    /// room gets its own ghost instead, naming where the passage came from.
    #[test]
    fn a_one_way_crossing_gets_an_arrival_ghost_on_the_far_panel() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Alcove".into());
        g.upsert_room(2, "Vault".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2); // one-way: no edge back
        let deep = g.new_layer(Some(mapper::layer::MAIN_LAYER), "Deep".into());
        g.set_room_layer(2, deep);
        let svg = render_svg_layered(&g);
        let map = layered_map_only(&svg);
        assert_eq!(map.matches("class=\"ghost\"").count(), 1, "Alcove's panel gets a departure ghost");
        assert_eq!(map.matches("class=\"ghost arrival\"").count(), 1, "Vault's panel gets an arrival ghost");
        assert!(svg.contains(">Vault<") && svg.contains(">Deep<"), "the departure ghost names Vault/Deep");
        assert!(svg.contains(">Alcove<") && svg.contains(">Main<"), "the arrival ghost names Alcove/Main");
        assert!(label_collisions(&svg).is_empty());
        assert!(ghost_box_overlaps(&svg).is_empty());

        // SQ-1330: the departure end's arrow sits at the GHOST (Alcove leaving toward Vault); the
        // arrival end's sits at the ROOM (Vault arriving from Alcove) — the two ends of one
        // passage, each read from its own panel, never both at the same box.
        let deps = ghost_rects_of(&svg, "ghost");
        let arrs = ghost_rects_of(&svg, "ghost arrival");
        let rooms = room_rects(&svg);
        assert_eq!(deps.len(), 1);
        assert_eq!(arrs.len(), 1);
        let tips = arrow_tips(&svg);
        assert_eq!(tips.len(), 2, "one arrowhead at the departure ghost, one at the arrival room");
        assert!(
            tips.iter().any(|&t| deps.iter().any(|&r| near_rect_edge(t, r, 1.5))),
            "the departure ghost must carry its own arrowhead: {tips:?} vs {deps:?}"
        );
        assert!(
            tips.iter().any(|&t| rooms.iter().any(|&r| near_rect_edge(t, r, 1.5))),
            "the arrival room must carry its own arrowhead: {tips:?} vs {rooms:?}"
        );
        assert!(
            !tips.iter().any(|&t| arrs.iter().any(|&r| near_rect_edge(t, r, 1.5))),
            "the arrival GHOST box (unlike the room) carries no letter and no arrowhead of its own: {tips:?} vs {arrs:?}"
        );
    }

    /// SQ-1330: a departure badge's letter and its arrow must agree about which way the passage
    /// runs — a `D` badge (down) sends its arrow BELOW the badge, toward the ghost it leads to,
    /// never above it.
    #[test]
    fn a_departure_arrow_points_the_way_its_badge_letter_says() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Attic".into());
        g.upsert_room(2, "Cellar".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 0));
        g.add_edge(1, Direction::Down, 2); // one-way: no edge back
        let below = g.new_layer(Some(mapper::layer::MAIN_LAYER), "Below".into());
        g.set_room_layer(2, below);
        let svg = render_svg_layered(&g);
        assert!(svg.contains(">D<"), "the badge names the direction actually travelled");
        let doc = roxmltree::Document::parse(&svg).expect("well-formed SVG");
        let badge_y = doc
            .descendants()
            .find(|n| n.tag_name().name() == "circle" && n.attribute("class") == Some("badge"))
            .map(|n| {
                let o = translate_of(n);
                n.attribute("cy").unwrap().parse::<f64>().unwrap() + o.1
            })
            .expect("Attic's panel draws a badge");
        // One-way, so Cellar's own panel also gets an arrival arrow — filter to the one that
        // belongs to Attic's departure ghost specifically, by finding the tip that sits on it.
        let deps = ghost_rects_of(&svg, "ghost");
        assert_eq!(deps.len(), 1, "Attic's panel gets exactly one departure ghost");
        let ghost_tip = arrow_tips(&svg)
            .into_iter()
            .find(|&t| deps.iter().any(|&r| near_rect_edge(t, r, 1.5)))
            .expect("the departure ghost must carry its own arrowhead");
        assert!(
            ghost_tip.1 > badge_y,
            "a `D` badge's arrow must sit BELOW it, toward the ghost it leads to: badge y={badge_y}, tip={ghost_tip:?}"
        );
    }

    /// A room with every side already crowded still gets its ghost, pushed farther out by
    /// `place_ghost`'s fallback rather than dropped (SQ-1319's whole point — falsify by reverting
    /// `place_ghost` to a fixed-candidate search and this must fail on `ghost_box_overlaps`, or
    /// on the ghost vanishing outright).
    #[test]
    fn a_crowded_departure_ghost_still_lands_clear() {
        use mapper::graph::MapGraph;
        let build = |crowd: bool| {
            let mut g = MapGraph::new();
            g.upsert_room(1, "Landing".into());
            g.upsert_room(2, "Loft".into());
            g.set_pos(1, (0, 0));
            g.set_pos(2, (0, 0));
            g.add_edge(1, Direction::Up, 2);
            if crowd {
                // Directly north of Landing: crowds the Up ghost's usual spot right above the box.
                g.upsert_room(3, "Blocker".into());
                g.set_pos(3, (0, -1));
            }
            let loft = g.new_layer(Some(mapper::layer::MAIN_LAYER), "Loft Layer".into());
            g.set_room_layer(2, loft);
            render_svg_layered(&g)
        };
        let base = build(false);
        let crowded = build(true);
        for svg in [&base, &crowded] {
            assert!(svg.contains(">Loft<"), "the ghost must always name the room: {svg}");
            assert!(svg.contains(">Loft Layer<"), "the ghost must always name the layer: {svg}");
        }
        let bad = label_collisions(&crowded);
        assert!(bad.is_empty(), "the crowded case must still land clear of labels: {bad:#?}");
        let bad = ghost_box_overlaps(&crowded);
        assert!(bad.is_empty(), "the crowded case must still land clear of rooms/ghosts: {bad:#?}");
        // SQ-1333: `Blocker` sits squarely in the corridor the Up ghost's straight line would
        // otherwise run through — the exact shape `place_ghost`'s bend exists for.
        let bad = ghost_line_room_crossings(&crowded);
        assert!(bad.is_empty(), "the crowded case's ghost line must not run through Blocker: {bad:?}");
    }

    /// SQ-1333: a room directly on the far side of a cross-layer exit must not put that room's
    /// own box in the ghost's connector line, or the ghost reads as the room's own exit — the
    /// exact defect on Zork I's house layer, where the Kitchen's Down ghost ran straight through
    /// South of House to reach its "Studio · Main" label. Falsify by reverting the line check in
    /// `search_ghost_side` (or the bend fallback in `place_ghost`) and this fails.
    #[test]
    fn a_ghost_line_never_runs_through_a_room_in_its_path() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Kitchen".into());
        g.upsert_room(2, "Studio".into());
        // Directly south of Kitchen — the Down ghost's straight-line spot.
        g.upsert_room(3, "South of House".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 0));
        g.set_pos(3, (0, 1));
        g.add_edge(1, Direction::Down, 2);
        let studio_layer = g.new_layer(Some(mapper::layer::MAIN_LAYER), "Studio Layer".into());
        g.set_room_layer(2, studio_layer);
        let svg = render_svg_layered(&g);

        assert!(svg.contains(">Studio<"), "the ghost must always name the room: {svg}");
        let deps = ghost_rects_of(&svg, "ghost");
        assert_eq!(deps.len(), 1, "one departure ghost for the one interlayer exit");
        let bad = ghost_box_overlaps(&svg);
        assert!(bad.is_empty(), "the ghost box must stay clear of South of House: {bad:#?}");
        // The invariant this whole quest is about: the connector is drawn (never dropped for
        // want of a clear line, SQ-1319's rule extended by SQ-1333), and it does not run through
        // South of House to get there.
        assert!(!ghost_line_segments(&svg).is_empty(), "the ghost's connector must still be drawn");
        let bad = ghost_line_room_crossings(&svg);
        assert!(bad.is_empty(), "the ghost's connector must not cross South of House: {bad:?}");
    }

    #[test]
    fn a_conditional_passage_carries_the_conditional_class_and_a_door_is_marked() {
        use mapper::graph::PassageWeight;
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        m.observe(1, "A", Some(Direction::W));
        let ids: Vec<_> = m.graph.rooms().map(|r| r.id).collect();
        m.graph.add_edge_weighted(ids[0], Direction::E, ids[1], PassageWeight::Conditional);
        let svg = body_of(&render(&m.graph), Some(&m.graph));
        assert!(svg.contains("conditional"), "a gated exit is dotted via its own class");

        m.graph.add_edge_weighted(ids[0], Direction::E, ids[1], PassageWeight::Door);
        let svg = body_of(&render(&m.graph), Some(&m.graph));
        assert!(svg.contains("class=\"door\""), "a door carries its bar mark");
        assert!(!svg.contains("conditional"), "and is not dotted");
    }

    #[test]
    fn the_legend_is_present_and_names_every_mark() {
        let m = zork_house();
        let svg = render_svg_of(&render(&m.graph), Some(&m.graph));
        assert!(svg.contains("class=\"legend-panel\""));
        assert!(svg.contains(">Legend<"));
        for caption in [
            "one-way passage",
            "two-way passage",
            "door",
            "conditional exit",
            "distorted",
            "exit to another layer",
        ] {
            assert!(svg.contains(caption), "legend must name {caption:?}");
        }
    }

    /// SQ-1344: every `.legend` caption's estimated right edge stays inside the
    /// `legend-panel` rect — the panel is sized off the longest row rather than a fixed
    /// `LEGEND_W`, using the same 9px character-width estimate `text_boxes()` charges a 9px
    /// `.legend` label elsewhere in this file.
    #[test]
    fn every_legend_row_fits_inside_its_panel() {
        let m = zork_house();
        let svg = render_svg_of(&render(&m.graph), Some(&m.graph));
        let doc = roxmltree::Document::parse(&svg).expect("well-formed SVG");

        let panel = doc
            .descendants()
            .find(|n| n.tag_name().name() == "rect" && n.attribute("class") == Some("legend-panel"))
            .expect("the legend panel must be drawn");
        let panel_off = translate_of(panel);
        let panel_right =
            panel_off.0 + panel.attribute("x").unwrap().parse::<f64>().unwrap()
                + panel.attribute("width").unwrap().parse::<f64>().unwrap();

        let mut checked = 0;
        for text in doc
            .descendants()
            .filter(|n| n.tag_name().name() == "text" && n.attribute("class") == Some("legend"))
        {
            let off = translate_of(text);
            let x = off.0 + text.attribute("x").unwrap().parse::<f64>().unwrap();
            let caption = text.text().unwrap_or("");
            // Same estimate `legend()` sized the panel with, and `text_boxes()` uses for every
            // other 9px `.legend`-class label.
            let w = caption.chars().count() as f64 * 9.0 * 0.6125;
            assert!(
                x + w <= panel_right + 0.01,
                "caption {caption:?} right edge {} must sit inside the panel's {panel_right}",
                x + w
            );
            checked += 1;
        }
        assert!(checked >= legend_rows().len(), "must have checked every legend row");
    }

    #[test]
    fn the_stylesheet_defines_every_documented_class() {
        let svg = render_svg(&render(&zork_house().graph));
        for sel in [
            ".room", ".edge", ".edge.reciprocal", ".edge.distorted", ".edge.conditional", ".door",
            ".badge", ".legend", ".ghost", ".ghost.arrival",
        ] {
            assert!(svg.contains(sel), "the stylesheet must define {sel}");
        }
    }

    #[test]
    fn a_room_box_is_wide_enough_for_its_own_label() {
        let mut m = Mapper::default();
        m.observe(1, "Sensitive Equipment Testing Room", None);
        m.observe(2, "A", Some(Direction::E));
        let svg = render_svg(&render(&m.graph));
        let doc = roxmltree::Document::parse(&svg).unwrap();
        // Every drawn label line must fit inside the box on its own row.
        let mut widest_line = 0usize;
        for n in doc.descendants().filter(|n| {
            n.attribute("class").unwrap_or("").split_whitespace().any(|c| c == "room-label")
        }) {
            widest_line = widest_line.max(n.text().unwrap_or("").chars().count());
        }
        let widest_box = room_rects(&svg)
            .iter()
            .map(|r| r.2)
            .fold(0.0f64, f64::max);
        assert!(
            widest_line as f64 * ADVANCE + 2.0 * LABEL_PAD <= widest_box + 0.01,
            "widest label line ({widest_line} chars) must fit the widest box ({widest_box}px)"
        );
        assert!(widest_box > (BOX_W * CELL_W) as f64, "a long name widens its column");
    }

    #[test]
    fn an_up_passage_with_no_planar_route_gets_a_lettered_badge() {
        let mut m = Mapper::default();
        m.observe(1, "Cellar", None);
        m.observe(2, "Attic", Some(Direction::Up));
        let svg = render_svg(&render(&m.graph));
        assert!(svg.contains("class=\"badge\""), "up/down show as a badge, never a Nerd Font glyph");
        assert!(svg.contains(">U<") || svg.contains(">D<"));
        // SAME-layer: a ghost only ever draws for a crossing that actually leaves the panel — the
        // MAP's own markup, not the legend (whose sample carries the class too), settles it.
        let body = body_of(&render(&m.graph), None);
        assert!(!body.contains("class=\"ghost"), "a same-layer passage carries no ghost: {body}");
    }

    #[test]
    fn current_room_has_distinct_style() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        let svg = render_svg(&render(&m.graph));
        assert!(svg.contains("class=\"room current\""), "the current room carries its own class");
    }

    #[test]
    fn xml_escape_covers_all_specials() {
        assert_eq!(xml_escape("a & b"), "a &amp; b");
        assert_eq!(xml_escape("<tag>"), "&lt;tag&gt;");
        assert_eq!(xml_escape("\"quoted\""), "&quot;quoted&quot;");
        assert_eq!(xml_escape("it's"), "it&#39;s");
        assert_eq!(xml_escape("plain"), "plain");
    }

    #[test]
    fn wrap_label_balances_two_lines_and_ellipsises_one_long_word() {
        assert_eq!(wrap_label("Attic"), vec!["Attic"]);
        assert_eq!(wrap_label("East-West Passage"), vec!["East-West", "Passage"]);
        let long = wrap_label(&"x".repeat(200));
        assert_eq!(long.len(), 1);
        assert!(long[0].ends_with('…'), "a single unsplittable word is ellipsised");
    }

    /// SQ-1346: a one-way passage's single arrowhead reads as its DESTINATION's arrival, the
    /// same "head points into the room it enters" rule a two-way's two heads already follow —
    /// falsify by reverting `draw_travel_arrival`'s call site back to `arrowhead(pts[0], ...)`
    /// and this fails (the tip lands back on A instead of B).
    #[test]
    fn a_one_way_arrowhead_sits_at_the_destination_not_the_origin() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        let svg = render_svg_of(&render(&m.graph), Some(&m.graph));
        let rooms = room_rects(&svg);
        assert_eq!(rooms.len(), 2, "the case must draw exactly the two rooms");
        let (a_rect, b_rect) = (rooms[0], rooms[1]);
        let tips = arrow_tips(&svg);
        assert_eq!(tips.len(), 1, "a one-way passage carries exactly one arrowhead");
        let tip = tips[0];
        assert!(
            near_rect_edge(tip, b_rect, 1.5),
            "the one-way head must sit on B's own edge: tip={tip:?} a={a_rect:?} b={b_rect:?}"
        );
        assert!(
            !near_rect_edge(tip, a_rect, 1.5),
            "the one-way head must not sit on A's edge: tip={tip:?} a={a_rect:?}"
        );
    }

    /// SQ-1346, extended to badges: a lettered U/D badge is this rule's stand-in for a head
    /// where a flat arrowhead can't show "up"/"down" — it travels to the SAME end a head would,
    /// so a one-way "up" passage's badge reads nearer its destination than its origin.
    #[test]
    fn a_one_way_vertical_badge_sits_nearer_the_destination_than_the_origin() {
        let mut m = Mapper::default();
        m.observe(1, "Cellar", None);
        m.observe(2, "Attic", Some(Direction::Up));
        let svg = render_svg_of(&render(&m.graph), Some(&m.graph));
        let rooms = room_rects(&svg);
        assert_eq!(rooms.len(), 2, "the case must draw exactly the two rooms");
        let (cellar, attic) = (rooms[0], rooms[1]);
        let doc = roxmltree::Document::parse(&svg).expect("well-formed SVG");
        let badge_pos = doc
            .descendants()
            .find(|n| {
                n.tag_name().name() == "circle"
                    && n.attribute("class") == Some("badge")
                    && !under_class(*n, "legend-block")
            })
            .map(|n| {
                let o = translate_of(n);
                (
                    n.attribute("cx").unwrap().parse::<f64>().unwrap() + o.0,
                    n.attribute("cy").unwrap().parse::<f64>().unwrap() + o.1,
                )
            })
            .expect("the map draws exactly one badge");
        let edge_dist_y = |y: f64, (_, ry, _, rh): (f64, f64, f64, f64)| -> f64 {
            (y - ry).abs().min((y - (ry + rh)).abs())
        };
        let (d_cellar, d_attic) = (edge_dist_y(badge_pos.1, cellar), edge_dist_y(badge_pos.1, attic));
        assert!(
            d_attic < d_cellar,
            "the U badge must sit nearer Attic than Cellar: badge={badge_pos:?} cellar={cellar:?} attic={attic:?}"
        );
    }

    /// SQ-1346, extended to compass tags: each end's mismatched-word `tag` travels with the head
    /// for the SAME travel — `NE` (A→B) now reads at B, `SW` (B→A) reads at A, a swap from where
    /// each sat before this quest (previously both departure-anchored).
    #[test]
    fn a_diagonal_drawn_orthogonally_is_tagged_with_its_own_word() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::NE));
        m.observe(1, "A", Some(Direction::SW));
        let svg = render_svg_of(&render(&m.graph), Some(&m.graph));
        assert!(
            svg.contains("class=\"tag\""),
            "a diagonal walked round the corner names the direction it really is"
        );
        assert!(svg.contains(">NE<") || svg.contains(">SW<"));

        let rooms = room_rects(&svg);
        assert_eq!(rooms.len(), 2, "the case must draw exactly the two rooms");
        let (a_rect, b_rect) = (rooms[0], rooms[1]);
        let doc = roxmltree::Document::parse(&svg).expect("well-formed SVG");
        let tag_pos = |word: &str| -> Option<(f64, f64)> {
            doc.descendants()
                .find(|n| {
                    n.tag_name().name() == "text"
                        && n.attribute("class") == Some("tag")
                        && n.text() == Some(word)
                        && !under_class(*n, "legend-block")
                })
                .map(|n| {
                    let o = translate_of(n);
                    (
                        n.attribute("x").unwrap().parse::<f64>().unwrap() + o.0,
                        n.attribute("y").unwrap().parse::<f64>().unwrap() + o.1,
                    )
                })
        };
        if let Some(ne) = tag_pos("NE") {
            assert!(
                near_rect_edge(ne, b_rect, 20.0),
                "NE (A→B) must read near B, where that travel arrives: {ne:?} vs b={b_rect:?}"
            );
        }
        if let Some(sw) = tag_pos("SW") {
            assert!(
                near_rect_edge(sw, a_rect, 20.0),
                "SW (B→A) must read near A, where that travel arrives: {sw:?} vs a={a_rect:?}"
            );
        }
    }
}
