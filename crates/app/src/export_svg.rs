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

/// The type size of the LAYER line inside a cross-layer ghost's box (SQ-1356) — the small second
/// line naming where the room it stands for actually lives, beneath the name drawn at
/// `LABEL_PX` like any other room's. Smaller than the name for the same reason the room card's
/// footnotes are: it is a qualifier on the name, not a competitor for the eye.
const GHOST_LAYER_PX: f64 = 8.0;
/// The layer line's own character advance — see `ADVANCE` for the name's. Two different sizes
/// of the same monospace stack, so the ratio between them (`GHOST_LAYER_PX / LABEL_PX`) is also
/// the ratio between how much room one character of each takes (SQ-1385).
const GHOST_LAYER_ADVANCE: f64 = GHOST_LAYER_PX * 0.6;

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
         .note-badge{{fill:#463b12;stroke:#fc0;stroke-width:1.2}}\
         .note-badge-text{{fill:#fc0;font-size:8px;text-anchor:middle}}\
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
         .ghost-name{{fill:#cdd;font-size:{LABEL_PX}px;text-anchor:middle}}\
         .ghost-layer{{fill:#99b;font-size:{GHOST_LAYER_PX}px;text-anchor:middle}}\
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

/// The most characters a box of `cells` cells may hold on one line, at a line drawn with
/// per-character advance `advance` — `ADVANCE` for a room NAME, [`GHOST_LAYER_ADVANCE`] for a
/// ghost's own layer-name subtitle (SQ-1385): the same box-width arithmetic at two different
/// type sizes, not two different rules.
fn chars_in_scaled(cells: i32, advance: f64) -> usize {
    (((cells * CELL_W) as f64 - 2.0 * LABEL_PAD) / advance).floor().max(1.0) as usize
}

/// The most characters a box of `cells` cells may hold on one NAME line.
fn chars_in(cells: i32) -> usize {
    chars_in_scaled(cells, ADVANCE)
}

/// The two-line balanced wrap [`wrap_label`] and [`wrap_ghost_subtitle`] (SQ-1385) share: split at
/// whichever word boundary minimises the LONGER of the two lines, which is what makes a box grow
/// by as little as possible, then ellipsise anything still too long for `cap` — a single word
/// wider than the widest box allows, or a caller with less than two words to split at all.
fn wrap_to_caps(label: &str, one_line: usize, cap: usize) -> Vec<String> {
    let label = label.trim();
    if label.chars().count() <= one_line {
        return vec![label.to_string()];
    }
    let words: Vec<&str> = label.split_whitespace().collect();
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

/// Wrap `label` onto at most two lines, balanced so the box need be no wider than it must.
///
/// A label that already fits one default-width line is left alone; anything longer is split at
/// whichever word boundary minimises the LONGER of the two lines, which is what makes a box
/// grow by as little as possible. A single word too long for the widest box is ellipsised.
fn wrap_label(label: &str) -> Vec<String> {
    wrap_to_caps(label, chars_in(BOX_W), chars_in(MAX_BOX_CELLS))
}

/// [`wrap_label`]'s own rule, at the smaller [`GHOST_LAYER_PX`] scale a cross-layer ghost's
/// subtitle draws at (SQ-1385) — only reached once [`ghost_box_cells`] finds a single subtitle
/// line would overflow even the widest box the layout allows, so widening the box has already
/// been preferred and failed; this is the fallback.
fn wrap_ghost_subtitle(layer_name: &str) -> Vec<String> {
    wrap_to_caps(
        layer_name,
        chars_in_scaled(BOX_W, GHOST_LAYER_ADVANCE),
        chars_in_scaled(MAX_BOX_CELLS, GHOST_LAYER_ADVANCE),
    )
}

/// The box width, in layout cells, that holds `lines` drawn at per-character advance `advance`.
fn box_cells_scaled(lines: &[String], advance: f64) -> i32 {
    let widest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f64;
    let need = widest * advance + 2.0 * LABEL_PAD;
    ((need / CELL_W as f64).ceil() as i32).clamp(BOX_W, MAX_BOX_CELLS)
}

/// The box width, in layout cells, that holds `lines` of a room NAME.
fn box_cells(lines: &[String]) -> i32 {
    box_cells_scaled(lines, ADVANCE)
}

/// A cross-layer ghost's own box width, in layout cells (SQ-1385), and the subtitle line(s) to
/// draw inside it: the wider of its NAME lines — as any room's box already is — and its
/// LAYER-NAME subtitle, sized at the subtitle's OWN [`GHOST_LAYER_ADVANCE`] scale rather than
/// counted in the name's characters. Before this a ghost's box was sized from `name_lines`
/// alone, so a short room name on a long-named layer (Counterfeit Monkey's "Samuel Johnson
/// Basement", "Tunnel through Chalk") overflowed the box the name itself was happy in.
///
/// Widening is always preferred: only a subtitle that would still overflow the widest box the
/// layout allows (`MAX_BOX_CELLS`) is wrapped onto two lines, with the same balanced-split rule
/// `wrap_label` uses for a name (see [`wrap_ghost_subtitle`]).
fn ghost_box_cells(name_lines: &[String], layer_name: &str) -> (i32, Vec<String>) {
    let name_want = box_cells(name_lines);
    // The UNCLAMPED cell count a single subtitle line needs — `box_cells_scaled` clamps to
    // `MAX_BOX_CELLS`, which would hide the very overflow this is checking for.
    let raw_cells = |chars: usize| -> f64 {
        (chars as f64 * GHOST_LAYER_ADVANCE + 2.0 * LABEL_PAD) / CELL_W as f64
    };
    if raw_cells(layer_name.chars().count()) <= MAX_BOX_CELLS as f64 {
        let one_line = vec![layer_name.to_string()];
        let sub_want = box_cells_scaled(&one_line, GHOST_LAYER_ADVANCE);
        (name_want.max(sub_want), one_line)
    } else {
        let sub_lines = wrap_ghost_subtitle(layer_name);
        let sub_want = box_cells_scaled(&sub_lines, GHOST_LAYER_ADVANCE);
        (name_want.max(sub_want), sub_lines)
    }
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

// ── Room notes ───────────────────────────────────────────────────────────────

/// Every room's own NOTE text (SQ-1384), keyed by room id — filtered to the non-empty ones, the
/// same population [`mapper::render::RenderRoom::has_notes`] flags but the TEXT `RenderMap` never
/// carries (only the bool). Read off the graph rather than the render model for the same reason
/// [`weight_table`] does: the render model is zoom-independent geometry, and a note's text isn't
/// geometry at all.
fn notes_table(graph: &MapGraph) -> HashMap<RoomId, String> {
    graph.rooms().filter(|r| !r.notes.is_empty()).map(|r| (r.id, r.notes.clone())).collect()
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
    ///
    /// `wide_channels` names the channel `idx`s (the channel following that row/column, in the
    /// same sense `channel_span(idx)` uses) that must additionally clear `PORTAL_MIN_CHANNEL_PX`
    /// rather than the plain `MIN_CHANNEL_PX` — SQ-1362's portal head+badge reaches farther out
    /// from a box edge than a bare arrowhead, so a channel carrying one needs more room than a
    /// channel that doesn't. Empty for the column axis, which a portal never touches (Up always
    /// departs the north border and Down the south — `route_side`).
    fn build(axis: &PosTable, cell_unit: i32, wide_channels: &std::collections::HashSet<i32>) -> PxAxis {
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
            let floor = if wide_channels.contains(&idx) { PORTAL_MIN_CHANNEL_PX } else { MIN_CHANNEL_PX };
            let per_cell = if default_px < floor { floor / chan as f64 } else { cell_unit as f64 };
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

/// The box CORNER `dir` names, in SVG pixels — `rect`'s own top-right/top-left/bottom-right/
/// bottom-left. Mirrors `render::map::corner_anchor`'s cell-space corner exactly (NE = top-right,
/// NW = top-left, SE = bottom-right, SW = bottom-left) so a pure diagonal's SVG endpoint and the
/// terminal's own anchor cell name the same physical corner (SQ-1365).
fn px_corner(rect: (f64, f64, f64, f64), dir: Direction) -> (f64, f64) {
    let (x, y, w, h) = rect;
    match dir {
        Direction::NE => (x + w, y),
        Direction::NW => (x, y),
        Direction::SE => (x + w, y + h),
        Direction::SW => (x, y + h),
        _ => (x + w / 2.0, y), // unreachable when guarded by is_diagonal
    }
}

/// `outward`'s counterpart for the four intercardinal directions: the outward unit vector at a
/// box corner, pointing away from the box along the slope (SQ-1365).
fn outward_diag(dir: Direction) -> (f64, f64) {
    const D: f64 = std::f64::consts::FRAC_1_SQRT_2;
    match dir {
        Direction::NE => (D, -D),
        Direction::NW => (-D, -D),
        Direction::SE => (D, D),
        Direction::SW => (-D, D),
        _ => (0.0, -1.0), // unreachable when guarded by is_diagonal
    }
}

/// Which axis, and which channel `idx` on it (the sense `PosTable::channel_span(idx)` uses: the
/// channel following row/column `idx`, between it and `idx + 1`), a portal marker leaving `room`'s
/// box by `side` reaches into.
///
/// A portal's DEPARTURE side is always `Top`/`Bottom` (SQ-1362; `route_side` puts Up on the north
/// border and Down on the south, always) — but its ARRIVAL side is not: the router picks whichever
/// side the geometry favours, and a cross-layer ghost seated BESIDE its anchor rather than in line
/// with it (SQ-1356's own fallback, when the straight seat was taken) arrives on the anchor's
/// `Left` or `Right` edge exactly as easily as its `Top` or `Bottom` (SQ-1366). So this reaches
/// into a ROW channel for `Top`/`Bottom`, same as it always did, and a COLUMN channel — the row
/// axis's own counterpart — for `Left`/`Right`.
enum PortalChannel {
    Row(i32),
    Col(i32),
}

fn portal_channel(cell_of: &HashMap<RoomId, (i32, i32)>, room: RoomId, side: Side) -> Option<PortalChannel> {
    let (cx, cy) = *cell_of.get(&room)?;
    Some(match side {
        Side::Top => PortalChannel::Row(cy - 1),
        Side::Bottom => PortalChannel::Row(cy),
        Side::Left => PortalChannel::Col(cx - 1),
        Side::Right => PortalChannel::Col(cx),
    })
}

/// The box side a passage travelling `dir` leaves by. Up and Down take the top and bottom
/// borders (as they do in the drawn view's portal slots); In and Out have no bearing of their
/// own and take the right; a compass passage (including one walked diagonally, or one that
/// crossed a layer per SQ-0360) asks the router the same question a real connector's own
/// perpendicular leg does.
fn side_for_travel(dir: Direction) -> Side {
    match dir {
        Direction::Up => Side::Top,
        Direction::Down => Side::Bottom,
        Direction::In | Direction::Out => Side::Right,
        d => mapper::router::side_for(d).unwrap_or(Side::Right),
    }
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

/// Distance from a box edge to a portal badge's own root, before `settle_badge` slides it clear
/// of anything already placed — behind the arrowhead's flat back (`ARROW_TIP`, 8.5px) so the two
/// read as one marker, head first (SQ-1362). Shares the `17` quantum `settle_badge`'s own slide
/// step and the stub side-stacking step already use elsewhere in this file.
const PORTAL_BADGE_GAP: f64 = 17.0;

/// How far a portal's head+badge pair reaches out from its box edge: the badge's own root
/// (`PORTAL_BADGE_GAP`) plus half its rendered footprint (`badge_rect`'s 8px half-width,
/// radius 6.5 plus a pixel of air).
const PORTAL_ARROW_REACH: f64 = PORTAL_BADGE_GAP + 8.0;

/// The SVG's own minimum channel width for a channel that carries a portal head+badge
/// (SQ-1362), the portal counterpart of `MIN_CHANNEL_PX` above and derived the same way: two
/// reaches facing each other across a channel of width `w` leave a shaft of `w - 2 *
/// PORTAL_ARROW_REACH` between them, and the ask is the same shaft `MIN_CHANNEL_PX` asks for — at
/// least twice one (bare) arrowhead's own length — giving `w >= 2 * PORTAL_ARROW_REACH + 2 *
/// ARROW_HEAD_LEN`. Only the channels `render_svg_body`'s pre-pass names actually widen to this —
/// see `PxAxis::build`'s `wide_channels` — everything else keeps the plain floor.
const PORTAL_MIN_CHANNEL_PX: f64 = 2.0 * PORTAL_ARROW_REACH + 2.0 * ARROW_HEAD_LEN;

/// The badge's own drawn radius — the `badge()` circle's `r` — pulled out so anything computing
/// how much room a badge needs, like `PORTAL_ARRIVAL_RUN_PX` below, states it once.
const BADGE_R: f64 = 6.5;

/// The shortest a portal arrival's FINAL straight run — the leg the badge rides, ending at the
/// head — may be (SQ-1366). The user's rule: the badge is always ON the line, on the straight
/// run right before the head; a right angle before the badge is fine, a right angle INTO the head
/// is not. `PORTAL_ARROW_REACH` already reaches the badge's own far edge from the box, so a run
/// that long fits the badge with nothing to spare; this adds one more badge diameter of slack plus
/// `CORNER_R`'s own rounding radius, since the corner at the FAR end of this run eats into it by
/// exactly as much as it rounds.
const PORTAL_ARRIVAL_RUN_PX: f64 = PORTAL_ARROW_REACH + 2.0 * BADGE_R + CORNER_R;

/// A lettered badge — the export's up/down/in/out glyph, spelled as a letter so the document
/// needs no symbol font at all.
fn badge(at: (f64, f64), letter: &str) -> String {
    format!(
        "<circle class=\"badge\" cx=\"{}\" cy=\"{}\" r=\"{}\"/>\
         <text class=\"badge-text\" x=\"{}\" y=\"{}\">{}</text>",
        f(at.0),
        f(at.1),
        f(BADGE_R),
        f(at.0),
        f(at.1 + 3.0),
        xml_escape(letter)
    )
}

/// SQ-1384's own badge: the same circle-plus-text shape [`badge`] draws a portal letter with, at
/// the same [`BADGE_R`], but in the yellow the plain `.notes` dot always used and carrying a
/// footnote NUMBER — assigned in reading order over the rooms a layer panel actually holds — so a
/// reader can find the room's own text in the panel's "Notes" block below the map.
fn note_badge(at: (f64, f64), n: usize) -> String {
    format!(
        "<circle class=\"note-badge\" cx=\"{}\" cy=\"{}\" r=\"{}\"/>\
         <text class=\"note-badge-text\" x=\"{}\" y=\"{}\">{n}</text>",
        f(at.0),
        f(at.1),
        f(BADGE_R),
        f(at.0),
        f(at.1 + 3.0)
    )
}

/// The marker set for ONE travel of a connector: an `arrowhead_inward` into the room the travel
/// arrives at, exactly like every other passage — plus, for a vertical (Up/Down) word, a
/// lettered badge riding just behind the head on the same line, since a flat arrowhead cannot
/// show "up" or "down" on its own (SQ-1362; before this the badge stood in for the head instead
/// of beside it, and a portal line carried no arrival marker at all). A horizontal word instead
/// gets the mismatched-word `tag` when the side the passage is drawn leaving by disagrees with
/// its own compass word (a diagonal walked round the corner orthogonally, or a distorted edge).
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
        // SQ-1366: the badge is always ON the line, riding the FINAL STRAIGHT RUN into the head —
        // never off to the side of it. A right angle before the badge is fine (the line can turn
        // wherever the router put its corner); a right angle INTO the head is not, because there
        // is then no straight run left for the badge to ride. `pos`/`u` are the box edge and its
        // outward normal, so `root`, sitting `PORTAL_BADGE_GAP` back along that same axis, is
        // guaranteed to fall ON the polyline's final segment — `extend_portal_arrival` (called by
        // this file's connector pass before `pos`'s line is even drawn) has already lengthened
        // that segment to at least `PORTAL_ARRIVAL_RUN_PX`, wider than `PORTAL_BADGE_GAP` plus the
        // badge's own reach, so `root` never has to slide to find room: it is placed exactly
        // there and reserved for later placements, never nudged sideways by `settle_badge`. A
        // collision here would mean the channel was not widened enough — the bug to fix, not a
        // reason to slide the badge off its line.
        over.push_str(&arrowhead_inward(pos, u, arrow_class));
        let root = (pos.0 + u.0 * PORTAL_BADGE_GAP, pos.1 + u.1 * PORTAL_BADGE_GAP);
        placer.block(badge_rect(root));
        let at = root;
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

/// Lengthen a portal's polyline so its FINAL leg into `pos` — the room/ghost edge it arrives
/// at, along `pos`'s own outward normal `u` — is at least `PORTAL_ARRIVAL_RUN_PX` long (SQ-1366).
///
/// The router's own polyline can turn its last corner closer to the box edge than that run
/// needs — a cross-layer ghost seated BESIDE its anchor rather than in line with it (SQ-1356's own
/// fallback) routes exactly this way, arriving on a side the portal-widened channels never
/// expected (`PortalChannel`). The corner itself is fine wherever the router put it; what breaks
/// is a corner too close to the edge, which leaves no straight room for the badge behind the head.
/// The fix moves that corner farther out along the SAME axis it was already on — the line still
/// turns exactly where the router chose, just farther from the room — never re-routing the turn.
///
/// `at_start` is `true` to extend the polyline's own START (`pts[0]`, used for a reciprocal's
/// back-travel, which arrives there) or `false` to extend its END (the connector's own arrival,
/// `pts[last]`). Does nothing when the leg leading to that end is not already parallel to `u` —
/// an orthogonally-routed approach's final leg always is, so a mismatch means this polyline's
/// shape is not what this function assumes, and guessing at a fix would be worse than leaving it.
fn extend_portal_arrival(pts: &mut [(f64, f64)], at_start: bool, u: (f64, f64)) {
    let n = pts.len();
    if n < 4 {
        // A single bend (n == 3) sits directly between the connector's two fixed anchors — its
        // OTHER neighbour is the far end's own departure/arrival point, not a free corner this
        // function may move. Sliding the near bend along `u` without also sliding that neighbour
        // would leave the corner no longer square; sliding the neighbour would drag a fixed
        // box-edge anchor off the box it's snapped to. Neither is right, so this is left alone
        // rather than guessed at — every case this bug actually produces has a real dogleg
        // (n >= 4), because a bend that close to a single-corner route only arises from a second
        // corner (the departure jogging around something) in the first place.
        return;
    }
    let (pos, bend_i) = if at_start { (pts[0], 1) } else { (pts[n - 1], n - 2) };
    let bend = pts[bend_i];
    let (dx, dy) = (bend.0 - pos.0, bend.1 - pos.1);
    // The bend must already lie on the arrival axis (an orthogonal approach's final leg is
    // always parallel to the side it lands on) — a perpendicular deviation means this polyline's
    // shape is not what this function assumes, so leave it untouched rather than guess.
    let perp = (dx * u.1 - dy * u.0).abs();
    if perp > 0.5 {
        return;
    }
    let len = (dx * dx + dy * dy).sqrt();
    if len >= PORTAL_ARRIVAL_RUN_PX {
        return;
    }
    let shift = PORTAL_ARRIVAL_RUN_PX - len;
    pts[bend_i] = (bend.0 + u.0 * shift, bend.1 + u.1 * shift);
    // The segment on the OTHER side of this bend met it at a right angle (consecutive legs of an
    // orthogonal route always alternate axis), so translating the bend along `u` must translate
    // that far neighbour by the same amount too, or the corner stops being square. `n >= 4`
    // (checked above) guarantees that neighbour is itself an interior point, never one of the
    // two fixed box-edge anchors — the segment beyond IT is aligned WITH `u`, so shifting it only
    // changes that segment's own length, not its direction, and needs no further propagation.
    let nb_i = if at_start { bend_i + 1 } else { bend_i - 1 };
    let nb = pts[nb_i];
    pts[nb_i] = (nb.0 + u.0 * shift, nb.1 + u.1 * shift);
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
/// A cross-layer GHOST is an ordinary entry of `rm.rooms` carrying [`mapper::render::GhostRoom`]
/// (SQ-1356), so it is sized, seated and routed to exactly like a room — the only difference is
/// the class its box is drawn with and the small layer line inside it.
/// One drawn COMPASS connector's own snapped/extended geometry — SQ-1373's own record in
/// `compass_connector_pts`, keyed by `(conn.origin, conn.dest)`. See that map's own comment.
struct CompassConnectorGeom {
    merge: bool,
    entry: Side,
    exit: Side,
    pts: Vec<(f64, f64)>,
}

fn render_svg_body(
    rm: &RenderMap,
    weights: &HashMap<(RoomId, Direction), PassageWeight>,
    notes: &HashMap<RoomId, String>,
) -> Option<(String, i32, i32)> {
    if rm.rooms.is_empty() {
        return None;
    }

    // SQ-1384: every room THIS map draws that carries a note, numbered in READING ORDER —
    // top-to-bottom, left-to-right by grid cell, which is the same order pixel position sorts to
    // since a channel widened for a long name never reorders the cells either side of it. A
    // ghost never carries its own note (its `id` is the REAL target room's, on another layer's
    // panel — the note belongs there, not to the placeholder standing in for it here).
    let mut noted_rooms: Vec<&mapper::render::RenderRoom> =
        rm.rooms.iter().filter(|r| r.ghost.is_none() && notes.contains_key(&r.id)).collect();
    noted_rooms.sort_by_key(|r| (r.cell.1, r.cell.0));
    let note_number: HashMap<RoomId, usize> =
        noted_rooms.iter().enumerate().map(|(i, r)| (r.id, i + 1)).collect();

    // ── Axes: the terminal's own, with each column widened to its widest room name ────────
    let labels: HashMap<RoomId, Vec<String>> =
        rm.rooms.iter().map(|r| (r.id, wrap_label(&r.label))).collect();
    // SQ-1385: a ghost's own subtitle lines, sized alongside its name below — kept so the draw
    // pass below draws exactly the lines the box was WIDENED for, rather than recomputing (and
    // risking disagreeing with) the wrap.
    let mut ghost_subtitles: HashMap<RoomId, Vec<String>> = HashMap::new();
    let mut col_dims: BTreeMap<i32, i32> = BTreeMap::new();
    for room in &rm.rooms {
        let empty = Vec::new();
        let name_lines = labels.get(&room.id).unwrap_or(&empty);
        let want = if let Some(ghost) = &room.ghost {
            let (want, sub_lines) = ghost_box_cells(name_lines, &ghost.layer_name);
            ghost_subtitles.insert(room.id, sub_lines);
            want
        } else {
            box_cells(name_lines)
        };
        let slot = col_dims.entry(room.cell.0).or_insert(BOX_W);
        *slot = (*slot).max(want);
    }
    let no_rows = BTreeMap::new();
    let (cols, rows) = boxes_axes_sized(&rm.plan, rm.bounds, BOX_W, &col_dims, BOX_H, &no_rows);

    let cell_of: HashMap<RoomId, (i32, i32)> = rm.rooms.iter().map(|r| (r.id, r.cell)).collect();
    // SQ-1362/SQ-1366: which ROW and COLUMN channels carry a portal head+badge, so
    // `PxAxis::build` can grow only those to `PORTAL_MIN_CHANNEL_PX` rather than bumping every
    // channel on the map (see `PortalChannel`). Only a NON-merge connector's marker counts: a
    // merge stub's own departure badge (see below) stays small and needs no extra room, and only
    // the entry end always gets a marker while the exit end gets one only when the connector is
    // reciprocal, mirroring exactly what `draw_travel_arrival` draws.
    let mut wide_row_channels: std::collections::HashSet<i32> = std::collections::HashSet::new();
    let mut wide_col_channels: std::collections::HashSet<i32> = std::collections::HashSet::new();
    for conn in &rm.plan.connectors {
        if conn.merge || !matches!(conn.exit_dir, Direction::Up | Direction::Down) {
            continue;
        }
        match portal_channel(&cell_of, conn.dest, conn.entry) {
            Some(PortalChannel::Row(idx)) => {
                wide_row_channels.insert(idx);
            }
            Some(PortalChannel::Col(idx)) => {
                wide_col_channels.insert(idx);
            }
            None => {}
        }
        if conn.reciprocal {
            match portal_channel(&cell_of, conn.origin, conn.exit) {
                Some(PortalChannel::Row(idx)) => {
                    wide_row_channels.insert(idx);
                }
                Some(PortalChannel::Col(idx)) => {
                    wide_col_channels.insert(idx);
                }
                None => {}
            }
        }
    }
    // SQ-1322: the SVG's own pixel geometry for each axis, widening a channel already at
    // `MIN_GUTTER` cells so a two-way passage between adjacent boxes gets a real shaft — see
    // `PxAxis`. `cols`/`rows` themselves are untouched and still the terminal's own cell layout.
    let px_cols = PxAxis::build(&cols, CELL_W, &wide_col_channels);
    let px_rows = PxAxis::build(&rows, CELL_H, &wide_row_channels);

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
    for room in &rm.rooms {
        placer.block(box_px_rect(&cols, &rows, &px_cols, &px_rows, room.cell));
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
    // SQ-1373: every drawn COMPASS connector's own snapped/extended geometry, keyed by
    // `(conn.origin, conn.dest)` — what a `StackedExit`'s marker pass (below) rides, since a
    // stacked exit's own primary direction is never routed as its own connector: it just IS
    // this pair's compass connector (see `collapse_stacked_exits`). Portal (Up/Down) connectors
    // never key this map — a stack always has a compass primary, or `collapse_stacked_exits`
    // leaves the group untouched (see that function's doc comment).
    let mut compass_connector_pts: HashMap<(RoomId, RoomId), CompassConnectorGeom> = HashMap::new();

    // ── Connectors ───────────────────────────────────────────────────────────────────────
    //
    // `None` for the diagonal glyph set: half-diagonal corner stubs are a terminal line-art
    // affair (`SymbolSet::diagonal_corners`), and the orthogonal reading is exactly what the
    // router laid out either way — the toggle only ever picked which GLYPHS the intermediate
    // run used. A diagonal therefore arrives here as the dogleg it is, and says so with a
    // direction tag at its departure anchor.
    //
    // EXCEPT a PURE diagonal (SQ-1365): the whole connector is one corner-to-corner run — centre
    // → shared corner → centre, `conn.points.len() == 3` — the diagonally-adjacent case the
    // router collapses (see `route::mod`'s own comment on `build_points_orient`, "a reciprocal
    // DIAGONAL pair on diagonally-adjacent rooms lands here too"). That is exactly the terminal's
    // own `pure_diagonal` test (`render::map::plot_connector`, guarded by `diag.is_some()` there
    // only because the CHAIN needs glyphs — the shape itself doesn't). SVG has no glyph budget,
    // so it draws the shape outright: one straight line between the two box corners, instead of
    // the orthogonal dogleg every other connector still gets below.
    for conn in &rm.plan.connectors {
        let is_portal = matches!(conn.exit_dir, Direction::Up | Direction::Down);

        let pure_diag = !conn.merge
            && conn.points.len() == 3
            && conn.entry_corner.is_some()
            && direction::is_diagonal(conn.exit_dir);
        // Both rects must resolve for the straight-line path to be taken; a missing room is not
        // expected to happen (`cell_of` is built from `rm.rooms`, the same source `rect_of`
        // reads), but falling through to the ordinary dogleg is the safe answer if it ever did.
        let pure_diag_corners = pure_diag
            .then(|| Some((rect_of(conn.origin)?, rect_of(conn.dest)?, conn.entry_corner?)))
            .flatten();

        let (pts, dep_u, arr_u) = if let Some((ro, rd, entry_corner)) = pure_diag_corners {
            let op = px_corner(ro, conn.exit_dir);
            let dp = px_corner(rd, entry_corner);
            (vec![op, dp], outward_diag(conn.exit_dir), outward_diag(entry_corner))
        } else {
            let Some(plot) = plot_connector(conn, &cols, &rows, None) else { continue };
            if plot.path.len() < 2 {
                continue;
            }
            let mut pts: Vec<(f64, f64)> =
                plot.path.iter().map(|&c| cell_px(&px_cols, &px_rows, c)).collect();

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
            // SQ-1366: every portal arrival's final leg must be a real straight run, long enough
            // for the badge that rides it — see `extend_portal_arrival`. A merge stub has no
            // arrival end of its own (its `pts[last]` is a trunk junction, not a room edge) so it
            // is excluded here exactly as it is from `draw_travel_arrival` below. The start end is
            // extended only for a reciprocal, matching the one case `draw_travel_arrival` draws a
            // second arrival at all.
            if is_portal && !conn.merge {
                extend_portal_arrival(&mut pts, false, outward(conn.entry));
                if conn.reciprocal {
                    extend_portal_arrival(&mut pts, true, outward(conn.exit));
                }
            }
            (pts, outward(conn.exit), outward(conn.entry))
        };
        for &p in &pts {
            ext.add(p.0, p.1);
        }

        // SQ-1373: record this connector's own finished geometry for the `StackedExit` pass
        // below, which runs after every connector has been plotted (a stacked exit's primary
        // may route as EITHER a forward `c` or a paired back-edge — see that pass's own
        // comment — so it needs the connector keyed both ways, not just `conn.origin`).
        if !is_portal {
            compass_connector_pts.insert(
                (conn.origin, conn.dest),
                CompassConnectorGeom { merge: conn.merge, entry: conn.entry, exit: conn.exit, pts: pts.clone() },
            );
        }

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
        // SQ-0688). Since SQ-1362 a portal's own arrival marker is a head with its badge riding
        // just behind it (see `draw_travel_arrival`), not the badge alone — matching every other
        // passage's grammar: the arrow points where the travel leads, the letter says how.
        //
        // `dep_u`/`arr_u` are already the right outward vector for wherever `pts` actually lands
        // — the box edge's cardinal normal for the ordinary dogleg, or the box corner's diagonal
        // normal for a pure diagonal (SQ-1365) — bound above alongside `pts` itself, since a merge
        // connector (below) never takes the pure-diagonal branch and so always gets the cardinal
        // one.
        let arrow_class = if conn.distorted { "arrow distorted" } else { "arrow" };
        if conn.merge {
            // A merge stub ends on another connector's TRUNK (a T-junction), not on a room edge —
            // see `RoutedConnector::merge` — so it has no arrival end of its own to carry a head
            // to. It keeps its long-standing departure-only marker: the shared trunk it joins is
            // what actually carries the arrival head into the destination room. That is still true
            // after SQ-1362 — this is a DEPARTURE mark, not an arrival, so it stays badge-only
            // exactly as the non-portal sibling below stays a bare outward arrowhead with no
            // badge; neither grew a second marker.
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

        // SQ-1368: a same-pair passage the router folded onto this shared line — instead of
        // drawing its own — still keeps a marker of its own
        // (`RoutedConnector::secondary_exit`/`secondary_entry`), one per collapsed direction, at
        // the end THAT direction's own travel arrives at (SQ-1346's rule, the same one every
        // other marker in this file follows): a direction recorded in `secondary_exit` travels
        // origin→dest and so arrives at `pts`'s own END; `secondary_entry` travels dest→origin
        // and arrives at `pts`'s own START. It carries no arrowhead of its own — one line, one
        // head per travel — so only the letter/tag says which way it goes, rooted beside
        // whatever already marks that end rather than on top of it. A merge stub's `pts` END is
        // a trunk junction, not a room edge, so it has nowhere valid to arrive and is skipped.
        //
        // A collapsed Up/Down/In/Out direction is ALSO still a stub in `rm.edges` (the graph
        // never learns its own passage got folded onto another room's line) — `room` here is the
        // direction's own true graph-origin (`conn.origin` for `secondary_exit`, `conn.dest` for
        // `secondary_entry`), fed into `portal_ends` exactly as `draw_travel_arrival` feeds its
        // own badges, so the stub pass below recognises this one as already drawn and skips it
        // rather than stamping a second badge for the same passage.
        //
        // `arr_u`/`dep_u`, not a fresh `outward(conn.entry)`/`outward(conn.exit)`: `pts`'s own
        // ends already carry whichever normal is right for wherever they actually landed (a pure
        // diagonal's box CORNER per SQ-1365, same as everywhere else in this loop), and re-deriving
        // a cardinal one here would point a collapsed marker off to the side of the line it rides.
        if !conn.merge {
            for (dirs, pos, u, room) in [
                (&conn.secondary_exit, pts[pts.len() - 1], arr_u, conn.origin),
                (&conn.secondary_entry, pts[0], dep_u, conn.dest),
            ] {
                for &dir in dirs {
                    if matches!(dir, Direction::Up | Direction::Down | Direction::In | Direction::Out) {
                        let root = (pos.0 + u.0 * PORTAL_BADGE_GAP, pos.1 + u.1 * PORTAL_BADGE_GAP);
                        let at = settle_badge(&mut placer, root, (-u.1, u.0));
                        let letter = direction::short_label(dir).to_uppercase();
                        over.push_str(&badge(at, &letter));
                        portal_ends.insert((room, dir));
                        ext.add(at.0 - 8.0, at.1 - 8.0);
                        ext.add(at.0 + 8.0, at.1 + 8.0);
                    } else {
                        let tag = direction::short_label(dir).to_uppercase();
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
            }
        }
    }

    // ── SQ-1373: exits `collapse_stacked_exits` folded before the router ever saw them ─────
    //
    // A `StackedExit` (SQ-1276) is resolved at the SOURCE, in `mapper::render`, not by the
    // router (contrast SQ-1368's `RoutedConnector::secondary_exit`/`secondary_entry`, folded
    // AFTER routing onto a shared line): several of ONE room's own outgoing directions to the
    // SAME destination collapse to one PRIMARY before `route_all` ever runs, so the connector
    // this file draws for that pair already IS the primary — there is no `conn.secondary_*` to
    // read for it. Every direction in one `StackedExit` (primary and secondary alike) shares
    // its origin (`room.id`) AND its destination (`stacked.dest`), so unlike SQ-1368's fold
    // (which can arrive at either end) a stacked exit's own travel is always room.id → dest and
    // its marker always lands at the DEST end of that connector.
    //
    // The connector for that pair may have been built with room.id as `conn.origin` (the
    // primary was chosen as the pair's forward edge, or drawn as a plain one-way — dest end is
    // `pts[pts.len() - 1]`) or with room.id as `conn.dest` (the OTHER room's own edge was chosen
    // as forward and the primary became its paired back-edge — dest end is `pts[0]`), so both
    // keys of `compass_connector_pts` are tried.
    for room in &rm.rooms {
        for stacked in &room.stacked_exits {
            let landing = compass_connector_pts
                .get(&(room.id, stacked.dest))
                .filter(|g| !g.merge)
                .map(|g| (g.pts[g.pts.len() - 1], outward(g.entry)))
                .or_else(|| {
                    compass_connector_pts
                        .get(&(stacked.dest, room.id))
                        .filter(|g| !g.merge)
                        .map(|g| (g.pts[0], outward(g.exit)))
                });
            let Some((pos, u)) = landing else {
                // Should not happen — `collapse_stacked_exits` only ever stacks a room's own
                // directions onto a destination it also keeps a primary, drawn route to. Stamp
                // at the origin's own edge (the `tag` class, per the SQ-1373 brief) rather than
                // silently dropping the direction.
                let Some(rect) = rect_of(room.id) else { continue };
                for &dir in &stacked.secondary {
                    let side = side_for_travel(dir);
                    let su = outward(side);
                    let root = side_root(rect, side, 9.0, 0.0);
                    let tag = direction::short_label(dir).to_uppercase();
                    if let Some(spot) = placer.place(tag.chars().count(), &spots_around(root, su, 5.0)) {
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
                continue;
            };
            for &dir in &stacked.secondary {
                if matches!(dir, Direction::Up | Direction::Down | Direction::In | Direction::Out) {
                    let root = (pos.0 + u.0 * PORTAL_BADGE_GAP, pos.1 + u.1 * PORTAL_BADGE_GAP);
                    let at = settle_badge(&mut placer, root, (-u.1, u.0));
                    let letter = direction::short_label(dir).to_uppercase();
                    over.push_str(&badge(at, &letter));
                    portal_ends.insert((room.id, dir));
                    ext.add(at.0 - 8.0, at.1 - 8.0);
                    ext.add(at.0 + 8.0, at.1 + 8.0);
                } else {
                    let tag = direction::short_label(dir).to_uppercase();
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
        }
    }

    // ── Portal / cross-layer badges ──────────────────────────────────────────────────────
    //
    // A stub is a passage with no planar route — up, down, in or out. It gets a lettered badge
    // on the side it leads by. A passage that CROSSES A LAYER is no longer one of these: since
    // SQ-1356 the room it leads to is drawn as a ghost box on this very panel, and the passage
    // to it is an ordinary routed connector like any other.
    let mut stubs_by_room: HashMap<RoomId, Vec<Direction>> = HashMap::new();
    for edge in &rm.edges {
        if !edge.is_stub || edge.dir == Direction::Unknown {
            continue;
        }
        if portal_ends.contains(&(edge.origin, edge.dir)) {
            continue; // the connector pass already badged this end — see `portal_ends`
        }
        stubs_by_room.entry(edge.origin).or_default().push(edge.dir);
    }
    for room in &rm.rooms {
        let Some(stubs) = stubs_by_room.get(&room.id) else { continue };
        let rect = box_px_rect(&cols, &rows, &px_cols, &px_rows, room.cell);
        // Group by the side each passage leads out of, then stack along that side.
        let mut per_side: HashMap<u8, Vec<Direction>> = HashMap::new();
        for &dir in stubs {
            per_side.entry(side_for_travel(dir) as u8).or_default().push(dir);
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
            for (i, &dir) in list.iter().enumerate() {
                let step = i as f64 * 17.0;
                let root = side_root(rect, side, 9.0, step);
                let at = settle_badge(&mut placer, root, tangent);
                over.push_str(&badge(at, &direction::short_label(dir).to_uppercase()));
                ext.add(at.0 - 8.0, at.1 - 8.0);
                ext.add(at.0 + 8.0, at.1 + 8.0);
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
    //
    // A cross-layer GHOST is drawn here, with the rooms, because that is what it is (SQ-1356):
    // the same box at the same size, with a dashed outline, muted text, and the layer it really
    // lives on as a small second line inside the box.
    for room in &rm.rooms {
        let (x, y, w, h) = box_px_rect(&cols, &rows, &px_cols, &px_rows, room.cell);
        ext.add_rect(x, y, w, h);
        // SQ-1384: a noted room's whole box is wrapped in a `<g><title>` so the note text is a
        // hover tooltip over the box — nothing to wrap for a room without one, and a ghost
        // never carries this (see `noted_rooms` above).
        let note = note_number.get(&room.id).map(|&n| (n, notes[&room.id].as_str()));
        if let Some((_, text)) = note {
            let _ = write!(boxes, "<g><title>{}</title>", xml_escape(text));
        }
        let cls = match (&room.ghost, room.is_current) {
            (Some(_), _) => "ghost",
            (None, true) => "room current",
            (None, false) => "room",
        };
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
        let label_cls = match (&room.ghost, room.is_current) {
            (Some(_), _) => "ghost-name",
            (None, true) => "room-label current",
            (None, false) => "room-label",
        };
        // A ghost's name lifts by half the layer line(s) it makes room for, so the two together
        // sit centred in the box the way a plain name does on its own — one lift's worth per
        // subtitle line, since SQ-1385 lets that subtitle wrap to two.
        let sub_line_count = if room.ghost.is_some() {
            ghost_subtitles.get(&room.id).map_or(1, |l| l.len().max(1))
        } else {
            0
        };
        let lift = GHOST_LAYER_PX * 0.62 * sub_line_count as f64;
        let first =
            y + h / 2.0 - (lines.len() as f64 - 1.0) * (LABEL_PX * 0.62) + LABEL_PX * 0.36 - lift;
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
        if room.ghost.is_some() {
            let sub_lines = ghost_subtitles.get(&room.id).map(Vec::as_slice).unwrap_or(&[]);
            let sub_first = first + (lines.len() as f64 - 1.0) * LABEL_PX * 1.24 + LABEL_PX * 1.1;
            for (i, line) in sub_lines.iter().enumerate() {
                let _ = write!(
                    boxes,
                    "<text class=\"ghost-layer\" text-anchor=\"middle\" x=\"{}\" y=\"{}\">{}</text>",
                    f(x + w / 2.0),
                    f(sub_first + i as f64 * GHOST_LAYER_PX * 1.24),
                    xml_escape(line)
                );
            }
        }
        if let Some((n, _)) = note {
            // SQ-1388: bottom-right corner, same inset as the old top-right placement — matches
            // the terminal map's own move of its `●` marker off the up-portal's former corner.
            let _ = write!(boxes, "{}", note_badge((x + w - 10.0, y + h - 10.0), n));
        } else if room.has_notes {
            // `has_notes` true but no text in `notes` only happens when a caller has no graph to
            // read the text from (`render_svg(rm)`, SQ-1313's own headless path) — the plain dot
            // SQ-1384 replaces everywhere the text IS known, kept here so that path still shows
            // something rather than silently losing the mark.
            let _ = write!(
                boxes,
                "<circle class=\"notes\" cx=\"{}\" cy=\"{}\" r=\"2.6\"/>",
                f(x + w - 6.0),
                f(y + h - 6.0)
            );
        }
        if note.is_some() {
            boxes.push_str("</g>");
        }
    }

    let (min_x, min_y, max_x, max_y) = ext.get();
    let (ox, oy) = (-min_x, -min_y);
    let mut width = (max_x - min_x).ceil() as i32;
    let mut height = (max_y - min_y).ceil() as i32;
    let mut body = format!(
        "<g transform=\"translate({},{})\">{edges}{boxes}{over}</g>",
        f(ox),
        f(oy)
    );

    // SQ-1384: the "Notes" block, drawn once the map's own extent is settled — its rows are
    // wrapped to that width, and the canvas grows DOWN to hold it, never sideways past what the
    // map already needed unless a note itself is wider still.
    if !noted_rooms.is_empty() {
        let rows: Vec<(usize, String)> = noted_rooms
            .iter()
            .map(|r| (note_number[&r.id], notes[&r.id].clone()))
            .collect();
        let (markup, block_w, block_h) = notes_block(&rows, width.max(1));
        width = width.max(block_w);
        const NOTES_GAP: i32 = 14;
        let _ = write!(body, "<g transform=\"translate(0,{})\">{markup}</g>", height + NOTES_GAP);
        height += NOTES_GAP + block_h;
    }

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

// ── Legend ────────────────────────────────────────────────────────────────────

/// Which room the legend's highlighted-room row is talking about (SQ-1392). [`render_svg_layered`]
/// (and everything it wraps) draws a live session's export, where the highlighted room really is
/// where the player is standing; `lanthorn-mapgen` has no player at all and only sets a current
/// room so the map has a starting point to highlight, so its legend must not claim anyone is in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegendVoice {
    /// A map exported while playing — the highlighted room is where the player is now.
    Played,
    /// A map generated offline by `lanthorn-mapgen` — the highlighted room is just where the
    /// story begins.
    Generated,
}

const LEGEND_W: i32 = 336;
const LEGEND_ROW: i32 = 15;

/// The legend rows: `(sample markup drawn at (0, 0), caption)`.
fn legend_rows(voice: LegendVoice) -> Vec<(String, &'static str)> {
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
            // SQ-1362: a portal draws the same arrival head every other passage does — the badge
            // rides just behind it on the line, at the destination end, never in place of it.
            format!(
                "{}{}{}",
                line("edge portal"),
                arrowhead((54.0, 0.0), (1.0, 0.0), "arrow"),
                badge((37.0, 0.0), "U")
            ),
            "stairs, ladders, in/out — the arrow points where it leads, the letter is the way you travel",
        ),
        (
            // SQ-1368: the router folds an extra passage between the SAME two rooms onto the
            // winning connector's own line rather than drawing a second one
            // (`RoutedConnector::secondary_exit`/`secondary_entry`) — the line still carries only
            // one head per travel, and the folded direction gets a tag of its own where it
            // arrives instead of vanishing.
            format!(
                "{}{}<text class=\"tag\" x=\"37\" y=\"-4\">E</text>",
                line("edge shared"),
                arrowhead((54.0, 0.0), (1.0, 0.0), "arrow")
            ),
            "two passages on one line — the extra direction is tagged where it arrives",
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
            match voice {
                LegendVoice::Played => "the room you are in",
                LegendVoice::Generated => "starting room",
            },
        ),
        (
            format!(
                "<line class=\"edge reciprocal\" x1=\"2\" y1=\"0\" x2=\"13\" y2=\"0\"/>\
                 {}\
                 <rect class=\"ghost\" x=\"13\" y=\"-8\" width=\"44\" height=\"16\" rx=\"3\"/>\
                 <text class=\"ghost-name\" x=\"35\" y=\"-1\">Studio</text>\
                 <text class=\"ghost-layer\" x=\"35\" y=\"6\">Main</text>",
                arrowhead_inward((13.0, 0.0), (-1.0, 0.0), "arrow")
            ),
            "a room on another layer — the layer it lives on beneath its name",
        ),
        (
            // SQ-1384: the badge itself is enough to draw the reader's eye to a noted room; what
            // it MEANS is spelled out here rather than assumed.
            note_badge((30.0, 0.0), 1),
            "a room with notes — find its text in the panel's own \"Notes\" list below the map",
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
fn legend(voice: LegendVoice) -> (String, i32, i32) {
    let rows = legend_rows(voice);
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

// ── Room notes block (SQ-1384) ──────────────────────────────────────────────────

/// The per-character pixel width [`notes_block`] wraps a `.legend`-sized (9px) row against —
/// same estimate `text_boxes()`/`legend()` already charge every 9px `.legend` label, so the two
/// never disagree about how much text a row of a given width can hold.
const NOTES_CHAR_W: f64 = 9.0 * 0.6125;

/// Greedy word-wrap of `text` to at most `max_chars` characters per line, splitting on
/// whitespace and hard-breaking any single word that alone exceeds `max_chars`. An embedded
/// newline starts a fresh line of its own (SQ-1384: a note may be more than one paragraph, and
/// the block keeps the writer's own breaks rather than running everything together).
///
/// General-purpose in the way `wrap_label` deliberately is not: a note has no line cap and no
/// ellipsis — it is read in full in the block below the map, never squeezed into a room box.
fn word_wrap(text: &str, max_chars: usize) -> Vec<String> {
    let max_chars = max_chars.max(1);
    let mut out: Vec<String> = Vec::new();
    for para in text.split('\n') {
        if para.trim().is_empty() {
            out.push(String::new());
            continue;
        }
        let mut cur = String::new();
        for word in para.split_whitespace() {
            let word_chars: Vec<char> = word.chars().collect();
            if word_chars.len() > max_chars {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                for chunk in word_chars.chunks(max_chars) {
                    out.push(chunk.iter().collect());
                }
                continue;
            }
            let added = word_chars.len() + if cur.is_empty() { 0 } else { 1 };
            if cur.chars().count() + added > max_chars {
                out.push(std::mem::take(&mut cur));
            }
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(word);
        }
        if !cur.is_empty() {
            out.push(cur);
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// The "Notes" block a layer panel draws beneath its own map (SQ-1384): one row per noted room,
/// numbered to match the badge on its box, its text wrapped to `wrap_w` at `.legend`'s own 9px —
/// styled like the legend for the same reason the legend IS a list of what a mark means: a
/// footnote badge only draws the eye, this is what makes the note actually readable without
/// hovering the box's `<title>`.
///
/// `rows` is `(footnote number, note text)`, already in the reading order the badges use.
/// Drawn with its top-left at `(0, 0)`; returns `(markup, width, height)`.
fn notes_block(rows: &[(usize, String)], wrap_w: i32) -> (String, i32, i32) {
    let max_chars = ((wrap_w as f64) / NOTES_CHAR_W).floor().max(1.0) as usize;
    let mut s = "<text class=\"legend-title\" x=\"0\" y=\"12\">Notes</text>".to_string();
    let mut y = 30;
    let mut widest_chars = 0usize;
    for (n, text) in rows {
        for (i, line) in word_wrap(text, max_chars).iter().enumerate() {
            let shown = if i == 0 { format!("{n}. {line}") } else { line.clone() };
            widest_chars = widest_chars.max(shown.chars().count());
            let _ = write!(s, "<text class=\"legend\" x=\"0\" y=\"{y}\">{}</text>", xml_escape(&shown));
            y += LEGEND_ROW;
        }
    }
    let h = y - LEGEND_ROW + 8;
    let w = wrap_w.max((widest_chars as f64 * NOTES_CHAR_W).ceil() as i32);
    (s, w, h)
}

// ── Documents ─────────────────────────────────────────────────────────────────

/// Wrap a body of markup — already at its own `(0, 0)` — in a document, with the legend below
/// it in the bottom-left corner.
fn document(body: &str, body_w: i32, body_h: i32, voice: LegendVoice) -> String {
    let (leg, leg_w, leg_h) = legend(voice);
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
    render_svg_of_voiced(rm, graph, LegendVoice::Played)
}

fn render_svg_of_voiced(rm: &RenderMap, graph: Option<&MapGraph>, voice: LegendVoice) -> String {
    let weights = graph.map(weight_table).unwrap_or_default();
    let notes = graph.map(notes_table).unwrap_or_default();
    let Some((body, w, h)) = render_svg_body(rm, &weights, &notes) else {
        return "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"></svg>".to_string();
    };
    document(&body, w, h, voice)
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
    render_svg_layered_voiced(graph, LegendVoice::Played)
}

/// As [`render_svg_layered`], but for a map nobody is playing: `lanthorn-mapgen` sets the story's
/// starting room current purely so the map has a room to highlight, and the live wording ("the
/// room you are in") would misdescribe a map with no player on it — this says "starting room"
/// instead (SQ-1392).
pub fn render_svg_layered_generated(graph: &MapGraph) -> String {
    render_svg_layered_voiced(graph, LegendVoice::Generated)
}

fn render_svg_layered_voiced(graph: &MapGraph, voice: LegendVoice) -> String {
    let mut layers: Vec<mapper::layer::LayerId> = graph
        .layers()
        .keys()
        .copied()
        .filter(|&l| !graph.rooms_in_layer(l).is_empty())
        .collect();
    layers.sort_unstable();
    if layers.len() <= 1 {
        return render_svg_of_voiced(&mapper::render::render(graph), Some(graph), voice);
    }

    const HEADING_H: i32 = 26;
    const FRAME_PAD: i32 = 12;
    const PANEL_GAP: i32 = 16;
    let weights = weight_table(graph);
    let notes = notes_table(graph);

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
        let frag = render_svg_body(&rm, &weights, &notes);
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
    document(&body, panel_w.max(1), y.max(1), voice)
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
    box_rects(svg, &["room", "ghost"])
}

/// Only the rooms this layer actually holds — a cross-layer ghost is NOT one (SQ-1356). Used
/// where the two have to be told apart: `ghost_box_overlaps` compares one list against the other,
/// and would otherwise report every ghost as sitting on itself.
fn plain_room_rects(svg: &str) -> Vec<(f64, f64, f64, f64)> {
    box_rects(svg, &["room"])
}

/// Every drawn box in `svg` whose class carries one of `kinds`, in the document's own coordinate
/// space, excluding the legend's own samples.
fn box_rects(svg: &str, kinds: &[&str]) -> Vec<(f64, f64, f64, f64)> {
    let doc = roxmltree::Document::parse(svg).expect("well-formed SVG");
    doc.descendants()
        .filter(|n| {
            n.tag_name().name() == "rect"
                && n
                    .attribute("class")
                    .unwrap_or("")
                    .split_whitespace()
                    .any(|c| kinds.contains(&c))
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
        // collision for the label placer to solve. `note-badge-text` (SQ-1384) is the same shape
        // at a fixed corner of its own room's box, for the same reason.
        if cls.starts_with("badge-text") || cls.starts_with("note-badge-text") {
            continue;
        }
        let text: String = node.text().unwrap_or("").to_string();
        if text.is_empty() {
            continue;
        }
        let px: f64 = match cls.split_whitespace().next().unwrap_or("") {
            "room-label" | "ghost-name" => LABEL_PX,
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
        // A label drawn INSIDE a box is not a placed label — the box is what keeps it clear, and
        // it necessarily overlaps its own room rect. `ghost-name`/`ghost-layer` join `room-label`
        // here for exactly that reason since SQ-1356, when a ghost stopped being a floating
        // caption and became a box of its own (which `room_rects` now gathers).
        if matches!(
            cls.split_whitespace().next(),
            Some("room-label") | Some("ghost-name") | Some("ghost-layer")
        ) {
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

/// Every ghost BOX in `svg` — the dashed `<rect class="ghost">`, not its text — that overlaps a
/// room or another ghost (SQ-1319; SQ-1356).
///
/// Since SQ-1356 a ghost takes a CELL, so `render_layer` keeps this empty by construction rather
/// than by search. It is kept as the check that says so: a seating pass that stopped honouring
/// "free cells only" would fail here, on a real story's map, before anyone noticed on screen.
pub fn ghost_box_overlaps(svg: &str) -> Vec<String> {
    let rooms = plain_room_rects(svg);
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
        let notes = graph.map(notes_table).unwrap_or_default();
        render_svg_body(rm, &weights, &notes).expect("a non-empty map").0
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

    /// Straight-line distance from `p` to `rect`'s nearer boundary — `0.0` for a point already
    /// inside or on it, the plain perpendicular gap for a point squarely off one side (the shape
    /// every portal marker sits in, SQ-1362), general enough to answer "which is farther from the
    /// room" regardless of `settle_badge`'s own tangential slide.
    fn dist_from_rect_edge(p: (f64, f64), rect: (f64, f64, f64, f64)) -> f64 {
        let (x, y, w, h) = rect;
        let dx = if p.0 < x { x - p.0 } else if p.0 > x + w { p.0 - (x + w) } else { 0.0 };
        let dy = if p.1 < y { y - p.1 } else if p.1 > y + h { p.1 - (y + h) } else { 0.0 };
        (dx * dx + dy * dy).sqrt()
    }

    /// Every non-legend portal badge's own `(letter, centre)` — `badge()` always pushes the
    /// circle immediately followed by its text, so pairing the two node lists by document order
    /// is safe (SQ-1362; used to tell a `U` badge from its own `D` on a two-way stairway).
    fn badges_of(svg: &str) -> Vec<(String, (f64, f64))> {
        let doc = roxmltree::Document::parse(svg).expect("well-formed SVG");
        let circles: Vec<_> = doc
            .descendants()
            .filter(|n| {
                n.tag_name().name() == "circle"
                    && n.attribute("class") == Some("badge")
                    && !under_class(*n, "legend-block")
            })
            .collect();
        let texts: Vec<_> = doc
            .descendants()
            .filter(|n| {
                n.tag_name().name() == "text"
                    && n.attribute("class") == Some("badge-text")
                    && !under_class(*n, "legend-block")
            })
            .collect();
        circles
            .iter()
            .zip(texts.iter())
            .map(|(c, t)| {
                let o = translate_of(*c);
                let x = c.attribute("cx").unwrap().parse::<f64>().unwrap() + o.0;
                let y = c.attribute("cy").unwrap().parse::<f64>().unwrap() + o.1;
                (t.text().unwrap_or("").to_string(), (x, y))
            })
            .collect()
    }

    /// SQ-1384's own [`badges_of`]: every non-legend `.note-badge` circle paired with its
    /// footnote number, in the document's own coordinate space.
    fn note_badges_of(svg: &str) -> Vec<(String, (f64, f64))> {
        let doc = roxmltree::Document::parse(svg).expect("well-formed SVG");
        let circles: Vec<_> = doc
            .descendants()
            .filter(|n| {
                n.tag_name().name() == "circle"
                    && n.attribute("class") == Some("note-badge")
                    && !under_class(*n, "legend-block")
            })
            .collect();
        let texts: Vec<_> = doc
            .descendants()
            .filter(|n| {
                n.tag_name().name() == "text"
                    && n.attribute("class") == Some("note-badge-text")
                    && !under_class(*n, "legend-block")
            })
            .collect();
        circles
            .iter()
            .zip(texts.iter())
            .map(|(c, t)| {
                let o = translate_of(*c);
                let x = c.attribute("cx").unwrap().parse::<f64>().unwrap() + o.0;
                let y = c.attribute("cy").unwrap().parse::<f64>().unwrap() + o.1;
                (t.text().unwrap_or("").to_string(), (x, y))
            })
            .collect()
    }

    /// The TRUE vertices of an `M`/`L`/`Q` path — unlike `path_points`, only a `Q`'s CONTROL
    /// point (the un-rounded corner `rounded_path` was given) is kept, not the point just past it
    /// that exists only to trace the rounding arc onto the next leg. `path_points` deliberately
    /// keeps that point too (a crossing check wants every point the drawn curve visits, rounding
    /// included); this instead reconstructs the exact polyline BEFORE `rounded_path` shortened
    /// each corner's two adjoining legs by `CORNER_R` — the shape `extend_portal_arrival` itself
    /// measures (SQ-1366), so a reader checking its work needs the same un-rounded lengths.
    ///
    /// The grammar `rounded_path` emits is `M x y` then, for each interior point, `L a Q c b`,
    /// ending in a final plain `L`. So a `L` immediately followed by a `Q` is that corner's
    /// pre-rounding approach point — an artifact, not a vertex — and every other `L` (the very
    /// last one) is real.
    fn path_vertices(d: &str) -> Vec<(f64, f64)> {
        let mut out = Vec::new();
        let mut toks = d.split_whitespace().peekable();
        let take2 = |toks: &mut std::iter::Peekable<std::str::SplitWhitespace<'_>>| -> (f64, f64) {
            let x = toks.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let y = toks.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
            (x, y)
        };
        while let Some(t) = toks.next() {
            match t {
                "M" => out.push(take2(&mut toks)),
                "L" => {
                    let p = take2(&mut toks);
                    if toks.peek() != Some(&"Q") {
                        out.push(p); // not followed by a corner: this IS the final vertex
                    } // else: the pre-rounding approach point for the corner that follows
                }
                "Q" => {
                    let c = take2(&mut toks);
                    let _b = take2(&mut toks); // the post-rounding point; not a real vertex
                    out.push(c);
                }
                _ => {}
            }
        }
        out
    }

    /// The final segment `(from, to)` of the connector in `svg` whose polyline ARRIVES nearest
    /// `target` — the last two TRUE vertices of its `<path class="edge …">`'s `d` (see
    /// `path_vertices`), already in the document's own coordinate space via `translate_of`. This
    /// is the leg a portal arrival's badge is meant to ride (SQ-1366).
    ///
    /// A layered document can draw several connectors (a crossing's own line on the panel it
    /// leaves, PLUS the ghost line into it on the panel it arrives at), so picking "the first
    /// path in the document" is not enough to name one connector — `target` (ordinarily the room
    /// whose arrival is under test) disambiguates by proximity, the same way `near_rect_edge` and
    /// `dist_from_rect_edge` already do for a single point. Excludes the legend's own sample line
    /// exactly as `edge_segments` does. `None` when `svg` draws no such path at all.
    fn last_connector_segment(svg: &str, target: (f64, f64, f64, f64)) -> Option<((f64, f64), (f64, f64))> {
        let doc = roxmltree::Document::parse(svg).ok()?;
        let mut best_seg: Option<((f64, f64), (f64, f64))> = None;
        let mut best_dist = f64::INFINITY;
        for node in doc.descendants().filter(|n| {
            n.tag_name().name() == "path"
                && n.attribute("class").is_some_and(|c| c.split_whitespace().any(|w| w == "edge"))
                && !under_class(*n, "legend-block")
        }) {
            let offset = translate_of(node);
            let pts = path_vertices(node.attribute("d").unwrap_or(""));
            let n = pts.len();
            if n < 2 {
                continue;
            }
            let seg = (
                (pts[n - 2].0 + offset.0, pts[n - 2].1 + offset.1),
                (pts[n - 1].0 + offset.0, pts[n - 1].1 + offset.1),
            );
            let d = dist_from_rect_edge(seg.1, target);
            if d < best_dist {
                best_dist = d;
                best_seg = Some(seg);
            }
        }
        best_seg
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

    // ── SQ-1356: a cross-layer ghost is a room the layout places ────────────────────────

    /// A reciprocal Up/Down crossing between two layers: each panel draws a ghost box for the
    /// room beyond it, at the same size as a real room's, in the cell the passage's own direction
    /// points at — and names the layer it really lives on inside the box.
    ///
    /// Falsify by reverting `render_layer`'s ghost seating and this fails on the ghost count.
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
        assert_eq!(map.matches("class=\"ghost\"").count(), 2, "one ghost per panel");
        assert!(svg.contains(">Cellar<"), "Hall's panel names the room it leads to");
        assert!(svg.contains(">Below<"), "…and the layer that room lives on");
        assert!(svg.contains(">Hall<"), "Cellar's panel names the room it leads back to");
        assert!(svg.contains(">Main<"), "…and the layer THAT room lives on");
        // Walked both ways, so the label is the plain room name — never a "to/from" form.
        assert!(!svg.contains(">to Cellar<") && !svg.contains(">from Hall<"));
        assert!(label_collisions(&svg).is_empty());
        assert!(ghost_box_overlaps(&svg).is_empty());
        assert!(
            !svg.contains("ghost-line"),
            "SQ-1356 retired the ghost's own connector: it is an ordinary routed passage now"
        );

        // Same box SIZE as the room it sits beside — a ghost takes a cell like any other room.
        let rooms = plain_room_rects(&svg);
        let ghosts = ghost_rects_of(&svg, "ghost");
        assert_eq!(ghosts.len(), 2);
        for gh in &ghosts {
            assert!(
                rooms.iter().any(|r| (r.2 - gh.2).abs() < 0.5 && (r.3 - gh.3).abs() < 0.5),
                "a ghost box is a room box: {gh:?} vs {rooms:?}"
            );
        }

        // SQ-1362: a ghost is an ordinary passage's arrival room, so the portal into it gets the
        // same head-plus-trailing-badge marker every other passage's arrival gets — not the badge
        // alone. Each panel draws a FULL reciprocal (its own real room is drawn beside a ghost
        // standing in for the other, SQ-1356), so each panel carries two heads: one back into its
        // own room, one into its ghost — four heads and four badges over the two panels.
        let tips = arrow_tips(&svg);
        assert_eq!(tips.len(), 4, "each panel's reciprocal draws a head into its own room AND into its ghost");
        let badges = badges_of(&svg);
        assert_eq!(badges.len(), 4, "and a badge riding behind each of those heads");
        for gh in &ghosts {
            let tip = *tips
                .iter()
                .find(|t| near_rect_edge(**t, *gh, 1.5))
                .unwrap_or_else(|| panic!("a head must land on this ghost: {gh:?} tips={tips:?}"));
            let (letter, badge_pos) = badges
                .iter()
                .min_by(|(_, a), (_, b)| {
                    dist_from_rect_edge(*a, *gh).partial_cmp(&dist_from_rect_edge(*b, *gh)).unwrap()
                })
                .expect("at least one badge on the map");
            assert!(letter == "U" || letter == "D", "a portal badge reads U or D, got {letter:?}");
            let (tip_dist, badge_dist) = (dist_from_rect_edge(tip, *gh), dist_from_rect_edge(*badge_pos, *gh));
            assert!(
                badge_dist > tip_dist,
                "the {letter} badge must ride behind its own head into the ghost: \
                 tip={tip:?} ({tip_dist}) badge={badge_pos:?} ({badge_dist})"
            );
        }
    }

    /// A one-way crossing says which way it runs, in words, on both panels: the leaving end
    /// reads `to <room>`, the arriving end `from <room>` — and never a two-way "to/from" pair.
    #[test]
    fn a_one_way_crossing_labels_its_two_ends_to_and_from() {
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
        assert_eq!(map.matches("class=\"ghost\"").count(), 2, "one ghost per panel");
        assert!(svg.contains(">to Vault<"), "Alcove's panel says where the passage LEADS: {svg}");
        assert!(svg.contains(">from Alcove<"), "Vault's panel says where it CAME FROM: {svg}");
        assert!(svg.contains(">Deep<") && svg.contains(">Main<"), "each names the other's layer");
        assert!(label_collisions(&svg).is_empty());
        assert!(ghost_box_overlaps(&svg).is_empty());

        // One travel, so one head — at the end that travel ARRIVES at (SQ-1346), which on each
        // panel is the box standing for the far end of the crossing.
        let tips = arrow_tips(&svg);
        assert_eq!(tips.len(), 2, "one head per panel: {tips:?}");
    }

    /// A ghost is seated into a FREE cell, so it can never sit on a room — and its passage is an
    /// ordinary connector, so it can never be routed through one either. Both halves are checked
    /// with `Blocker` standing exactly where the crossing's own bearing points.
    #[test]
    fn a_crowded_ghost_still_lands_clear_of_every_room() {
        use mapper::graph::MapGraph;
        let build = |crowd: bool| {
            let mut g = MapGraph::new();
            g.upsert_room(1, "Landing".into());
            g.upsert_room(2, "Loft".into());
            g.set_pos(1, (0, 0));
            g.set_pos(2, (0, 0));
            g.add_edge(1, Direction::Up, 2);
            if crowd {
                // Directly north of Landing: the Up ghost's own cell.
                g.upsert_room(3, "Blocker".into());
                g.set_pos(3, (0, -1));
            }
            let loft = g.new_layer(Some(mapper::layer::MAIN_LAYER), "Loft Layer".into());
            g.set_room_layer(2, loft);
            render_svg_layered(&g)
        };
        for svg in [&build(false), &build(true)] {
            assert!(svg.contains(">to Loft<"), "the ghost must always name the room: {svg}");
            assert!(svg.contains(">Loft Layer<"), "the ghost must always name the layer: {svg}");
            let bad = label_collisions(svg);
            assert!(bad.is_empty(), "labels must stay clear: {bad:#?}");
            let bad = ghost_box_overlaps(svg);
            assert!(bad.is_empty(), "the ghost must stay clear of rooms and ghosts: {bad:#?}");
            // `room_rects` gathers ghost boxes too since SQ-1356, so this is the SQ-1333 reading
            // as well: no connector runs through a ghost box either.
            let bad = connector_room_crossings(svg);
            assert!(bad.is_empty(), "no passage may run through a box: {bad:?}");
        }
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
            "a room on another layer",
        ] {
            assert!(svg.contains(caption), "legend must name {caption:?}");
        }
    }

    /// SQ-1392: `lanthorn-mapgen` has no player, so the legend row for the highlighted room must
    /// not claim one is standing there — it says "starting room" instead of the live map's "the
    /// room you are in". Both forms go through [`render_svg_layered`] (the played form) versus
    /// [`render_svg_layered_generated`] (mapgen's), the two production entry points SQ-1392 added.
    #[test]
    fn a_generated_map_says_starting_room_and_a_played_map_says_you_are_in_it() {
        let m = zork_house();

        let played = render_svg_layered(&m.graph);
        assert!(played.contains("the room you are in"), "the played legend keeps its live wording");
        assert!(!played.contains("starting room"), "a played map has no reason to say \"starting room\"");

        let generated = render_svg_layered_generated(&m.graph);
        assert!(generated.contains("starting room"), "a generated map's legend says \"starting room\"");
        assert!(
            !generated.contains("the room you are in"),
            "a generated map has no player, so it must not claim one is in the highlighted room"
        );
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
        assert!(checked >= legend_rows(LegendVoice::Played).len(), "must have checked every legend row");
    }

    #[test]
    fn the_stylesheet_defines_every_documented_class() {
        let svg = render_svg(&render(&zork_house().graph));
        for sel in [
            ".room", ".edge", ".edge.reciprocal", ".edge.distorted", ".edge.conditional", ".door",
            ".badge", ".legend", ".ghost", ".ghost-name", ".ghost-layer",
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

    /// SQ-1385: a ghost's LAYER-NAME subtitle can be far longer than the room name it stands
    /// beside (Counterfeit Monkey ghosts a room named just "Cellar" onto "Samuel Johnson
    /// Basement") — the box must widen to hold the SUBTITLE too, not just the name it was sized
    /// from before. Falsify by reverting `ghost_box_cells` to size from `name_lines` alone and
    /// this fails on the box width.
    #[test]
    fn a_ghost_box_widens_for_a_long_layer_name_not_just_its_room_name() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "X".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 1));
        g.add_edge(1, Direction::Down, 2);
        g.add_edge(2, Direction::Up, 1);
        let below = g.new_layer(Some(mapper::layer::MAIN_LAYER), "Samuel Johnson Basement".into());
        g.set_room_layer(2, below);

        let svg = render_svg_layered(&g);
        let map = layered_map_only(&svg);
        assert!(map.contains("class=\"ghost\""), "the case must actually draw the ghost");
        assert!(map.contains(">Samuel Johnson Basement<"), "the ghost must name the long layer");

        let ghosts = ghost_rects_of(&svg, "ghost");
        assert_eq!(ghosts.len(), 2, "one ghost per panel (Hall's panel and Below's)");
        let subtitle_chars = "Samuel Johnson Basement".chars().count() as f64;
        let estimated_w = subtitle_chars * GHOST_LAYER_ADVANCE + 2.0 * LABEL_PAD;
        assert!(
            ghosts.iter().any(|g| g.2 >= estimated_w - 0.5),
            "some ghost box must be at least as wide as its subtitle needs ({estimated_w}px): {ghosts:?}"
        );
        assert!(label_collisions(&svg).is_empty());
        assert!(ghost_box_overlaps(&svg).is_empty());
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

    /// SQ-1362: a portal reads exactly like every other passage now — an arrowhead into the room
    /// the travel arrives at, with the lettered badge riding just behind it on the same line.
    /// Before this, a portal's arrival end carried the badge and NOTHING else, so a reader could
    /// not tell "leads down" from "arrived by going down". Unlike the stub cases above, this
    /// graph gives both rooms real positions (`set_pos`), so `route_lanes` draws a genuine routed
    /// portal connector — the code path `draw_travel_arrival` actually marks.
    #[test]
    fn a_one_way_portal_head_sits_near_destination_with_its_badge_riding_behind() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Cellar".into());
        g.upsert_room(2, "Attic".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::Up, 2);

        let svg = render_svg_of(&render(&g), Some(&g));
        let rooms = room_rects(&svg);
        assert_eq!(rooms.len(), 2, "the case must draw exactly the two rooms");
        let (cellar, attic) = (rooms[0], rooms[1]);

        let tips = arrow_tips(&svg);
        assert_eq!(tips.len(), 1, "a one-way portal carries exactly one arrowhead");
        let tip = tips[0];
        assert!(
            near_rect_edge(tip, attic, 1.5),
            "the head must sit on Attic's own edge: tip={tip:?} attic={attic:?}"
        );
        assert!(!near_rect_edge(tip, cellar, 1.5), "the head must not sit on Cellar's edge: tip={tip:?}");

        let badges = badges_of(&svg);
        assert_eq!(badges.len(), 1, "a one-way portal carries exactly one badge");
        let (letter, badge_pos) = &badges[0];
        assert_eq!(letter, "U", "an Up travel reads U");

        let (tip_dist, badge_dist) = (dist_from_rect_edge(tip, attic), dist_from_rect_edge(*badge_pos, attic));
        assert!(
            badge_dist > tip_dist,
            "the badge must sit farther from Attic than the head that points into it: \
             tip={tip:?} ({tip_dist}) badge={badge_pos:?} ({badge_dist})"
        );
    }

    /// SQ-1362, the two-way case: a reciprocal stairway draws a head AND a badge at each end,
    /// each badge riding behind its own head — never a bare pair of letters facing each other.
    #[test]
    fn a_two_way_stairway_draws_two_heads_each_with_its_badge_riding_behind() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Cellar".into());
        g.upsert_room(2, "Attic".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(2, Direction::Down, 1);

        let svg = render_svg_of(&render(&g), Some(&g));
        let rooms = room_rects(&svg);
        assert_eq!(rooms.len(), 2, "the case must draw exactly the two rooms");
        let (cellar, attic) = (rooms[0], rooms[1]);

        let tips = arrow_tips(&svg);
        assert_eq!(tips.len(), 2, "a two-way stairway carries a head at each end");
        let badges = badges_of(&svg);
        assert_eq!(badges.len(), 2, "and a badge riding behind each head");

        for tip in &tips {
            let (near_attic, near_cellar) = (near_rect_edge(*tip, attic, 1.5), near_rect_edge(*tip, cellar, 1.5));
            assert!(near_attic != near_cellar, "a head belongs to exactly one box's edge: tip={tip:?}");
            let (dest, want_letter) = if near_attic { (attic, "U") } else { (cellar, "D") };
            let (_, badge_pos) = badges
                .iter()
                .find(|(l, _)| l == want_letter)
                .unwrap_or_else(|| panic!("a {want_letter} badge for the head into {dest:?}: {badges:?}"));
            let (tip_dist, badge_dist) = (dist_from_rect_edge(*tip, dest), dist_from_rect_edge(*badge_pos, dest));
            assert!(
                badge_dist > tip_dist,
                "the {want_letter} badge must sit farther from its own room than its own head: \
                 tip={tip:?} ({tip_dist}) badge={badge_pos:?} ({badge_dist})"
            );
        }
    }

    /// SQ-1346, extended to compass tags: each end's mismatched-word `tag` travels with the head
    /// for the SAME travel — `NE` (A→B) now reads at B, `SW` (B→A) reads at A, a swap from where
    /// each sat before this quest (previously both departure-anchored).
    ///
    /// SQ-1365: A and B are diagonally-adjacent rooms with a reciprocal NE/SW pair between
    /// them — a PURE diagonal, per `render::map::plot_connector`'s own test — so the line is now a
    /// single straight run between the two box corners, not the dogleg this case was named for
    /// before this quest (the tag/head assertions below are unaffected: a diagonal word never
    /// matches the cardinal `side` a pure diagonal still carries, so both still get tagged).
    #[test]
    fn a_pure_diagonal_between_neighbours_is_a_straight_corner_to_corner_line() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::NE));
        m.observe(1, "A", Some(Direction::SW));
        let svg = render_svg_of(&render(&m.graph), Some(&m.graph));
        assert!(
            svg.contains("class=\"tag\""),
            "a diagonal names the direction it really is even when drawn as a true slope"
        );
        assert!(svg.contains(">NE<") || svg.contains(">SW<"));

        let rooms = room_rects(&svg);
        assert_eq!(rooms.len(), 2, "the case must draw exactly the two rooms");
        let (a_rect, b_rect) = (rooms[0], rooms[1]);

        // The one connector between A and B is drawn as exactly one segment, corner to corner —
        // no orthogonal dogleg, no shared horizontal/vertical stub in the gutter between them.
        let segs = edge_segments(&svg);
        assert_eq!(segs.len(), 1, "a pure diagonal draws ONE segment, not a multi-leg dogleg: {segs:?}");
        let (p0, p1) = segs[0];
        assert!(
            (p0.0 - p1.0).abs() > 1.0 && (p0.1 - p1.1).abs() > 1.0,
            "the segment must move on BOTH axes — an axis-aligned stub means the old hairpin \
             survived: {p0:?} -> {p1:?}"
        );

        let tips = arrow_tips(&svg);
        assert_eq!(tips.len(), 2, "a reciprocal diagonal carries a head at each end");

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

    /// SQ-1365: a 2x2 block with BOTH diagonals routed (an X between four rooms) draws two
    /// straight lines that cross in the shared gutter, not two orthogonal hairpins forced to
    /// share their horizontal stubs — which is the unreadable picture this quest was filed
    /// against (Anchorhead's nine-room "Out to Sea" grid is the real-game case, four of these).
    #[test]
    fn a_crossing_diagonal_pair_is_two_straight_lines_not_shared_stubs() {
        let mut g = MapGraph::new();
        for (id, label, pos) in
            [(1u32, "NW", (0, 0)), (2, "NE", (1, 0)), (3, "SW", (0, 1)), (4, "SE", (1, 1))]
        {
            g.upsert_room(id, label.into());
            g.set_pos(id, pos);
        }
        g.add_edge(1, Direction::SE, 4);
        g.add_edge(4, Direction::NW, 1);
        g.add_edge(2, Direction::SW, 3);
        g.add_edge(3, Direction::NE, 2);

        let svg = render_svg_of(&render(&g), Some(&g));
        let rooms = room_rects(&svg);
        assert_eq!(rooms.len(), 4, "the case must draw exactly the four rooms");

        let segs = edge_segments(&svg);
        assert_eq!(segs.len(), 2, "each diagonal draws ONE segment, two connectors: {segs:?}");
        for (p0, p1) in &segs {
            assert!(
                (p0.0 - p1.0).abs() > 1.0 && (p0.1 - p1.1).abs() > 1.0,
                "each of the crossing pair must move on BOTH axes — an axis-aligned stub means a \
                 hairpin survived: {p0:?} -> {p1:?}"
            );
        }
        // The two segments must not share a horizontal (or vertical) run — the defect this quest
        // fixes is exactly the two doglegs' shared stub landing on one line in the gutter.
        let (a0, a1) = segs[0];
        let (b0, b1) = segs[1];
        let is_horiz = |p: (f64, f64), q: (f64, f64)| (p.1 - q.1).abs() < 0.5;
        let is_vert = |p: (f64, f64), q: (f64, f64)| (p.0 - q.0).abs() < 0.5;
        assert!(
            !(is_horiz(a0, a1) && is_horiz(b0, b1) && (a0.1 - b0.1).abs() < 0.5),
            "the two diagonals must not share one horizontal run: {a0:?}->{a1:?} vs {b0:?}->{b1:?}"
        );
        assert!(
            !(is_vert(a0, a1) && is_vert(b0, b1) && (a0.0 - b0.0).abs() < 0.5),
            "the two diagonals must not share one vertical run: {a0:?}->{a1:?} vs {b0:?}->{b1:?}"
        );
    }

    /// A diagonal that is NOT pure — the destination is two columns over, so the router's
    /// polyline bends rather than collapsing to centre→corner→centre — still draws the
    /// orthogonal dogleg exactly as before this quest (SQ-1365 only touches the corner-to-corner
    /// case; everything else is untouched).
    #[test]
    fn a_non_pure_diagonal_still_draws_the_orthogonal_dogleg() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (2, -1));
        g.add_edge(1, Direction::NE, 2);
        g.add_edge(2, Direction::SW, 1);

        let svg = render_svg_of(&render(&g), Some(&g));
        let rooms = room_rects(&svg);
        assert_eq!(rooms.len(), 2, "the case must draw exactly the two rooms");

        let segs = edge_segments(&svg);
        assert!(
            segs.len() > 1,
            "a non-pure diagonal must still bend — a single segment here would mean this case \
             stopped exercising the dogleg path: {segs:?}"
        );
        assert!(
            segs.iter().any(|(p0, p1)| (p0.0 - p1.0).abs() < 0.5 || (p0.1 - p1.1).abs() < 0.5),
            "the dogleg's own legs are axis-aligned: {segs:?}"
        );
    }

    // ── SQ-1366: the badge always rides the final straight run into the head ────────────

    /// The Gallery shape from the field report: a cross-layer ghost that could not seat in line
    /// with its anchor (its straight north seat is a real reciprocal neighbour, and so is the
    /// seat beyond THAT — SQ-1356's own fallback) lands beside it instead, one cell to the west.
    /// The portal connector into the anchor then has to dogleg round the corner rather than
    /// running straight up/down into it, and its arrival ends up on the anchor's LEFT edge — a
    /// side `portal_channel_row`'s old Top/Bottom-only assumption never widened a channel for.
    ///
    /// The user's rule: the badge is always ON the line, riding the final straight run right
    /// before the head. A right angle before the badge is fine; a right angle INTO the head is
    /// not. This asserts exactly that: the polyline's final segment is straight, at least
    /// `PORTAL_ARRIVAL_RUN_PX` long, and the badge's centre sits within 1px of that segment's own
    /// axis (never off to the side of it, as `settle_badge`'s slide used to leave it here).
    #[test]
    fn a_ghost_seated_beside_its_anchor_still_rides_the_badge_on_the_line() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Living Room".into());
        g.upsert_room(2, "Cellar".into());
        g.upsert_room(3, "North Room".into());
        g.upsert_room(4, "East Room".into());
        g.set_pos(2, (1, 1));
        g.set_pos(3, (1, 0));
        g.set_pos(4, (2, 1));
        // A real reciprocal neighbour on Cellar's straight (north) seat, so the ghost cannot seat
        // there — and one to the east too, so the fallback that finds west clear is exercised
        // deterministically rather than by whichever side happens to have more free neighbours.
        g.add_edge(2, Direction::N, 3);
        g.add_edge(3, Direction::S, 2);
        g.add_edge(2, Direction::E, 4);
        g.add_edge(4, Direction::W, 2);
        g.add_edge(1, Direction::Down, 2); // one-way crossing: Living Room, Down, into Cellar
        let gallery = g.new_layer(Some(mapper::layer::MAIN_LAYER), "Gallery".into());
        g.set_room_layer(2, gallery);
        g.set_room_layer(3, gallery);
        g.set_room_layer(4, gallery);

        let svg = render_svg_layered(&g);
        let map = layered_map_only(&svg);
        assert!(map.contains("class=\"ghost\""), "the case must actually draw the ghost");

        // Falsify the fixture itself: this only exercises SQ-1366 if the ghost really did land
        // BESIDE Cellar rather than in line with it — a straight seat needs no dogleg and so
        // never reproduces the bug this test is for. Find Cellar's own box by its LABEL rather
        // than by row alone: East Room shares Cellar's row too, and matching on that alone picks
        // whichever of the two happens first.
        let doc = roxmltree::Document::parse(&svg).expect("well-formed SVG");
        let cellar_label = doc
            .descendants()
            .find(|n| {
                n.tag_name().name() == "text"
                    && n.attribute("class") == Some("room-label")
                    && n.text() == Some("Cellar")
            })
            .map(|n| {
                let o = translate_of(n);
                (
                    n.attribute("x").unwrap().parse::<f64>().unwrap() + o.0,
                    n.attribute("y").unwrap().parse::<f64>().unwrap() + o.1,
                )
            })
            .expect("Cellar's own label");
        let cellar = plain_room_rects(&svg)
            .into_iter()
            .find(|&(x, y, w, h)| {
                cellar_label.0 >= x && cellar_label.0 <= x + w && cellar_label.1 >= y && cellar_label.1 <= y + h
            })
            .expect("Cellar's own box, under its own label");
        let ghosts = ghost_rects_of(&svg, "ghost");
        let ghost = *ghosts
            .iter()
            .find(|&&(_, gy, _, gh)| (gy - cellar.1).abs() < 0.5 && (gh - cellar.3).abs() < 0.5)
            .expect("a ghost sharing Cellar's row");
        assert!(
            (ghost.1 - cellar.1).abs() < 0.5,
            "the ghost must sit beside Cellar (same row), not above/below it: ghost={ghost:?} cellar={cellar:?}"
        );
        assert!(ghost.0 < cellar.0, "the ghost must land WEST of Cellar, matching the field report: {ghost:?} vs {cellar:?}");

        let (from, to) = last_connector_segment(&svg, cellar)
            .expect("the crossing draws a connector with at least one final segment");
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let len = (dx * dx + dy * dy).sqrt();
        assert!(dy.abs() < 0.5, "the final run must be straight (horizontal, arriving on Cellar's side): {from:?} -> {to:?}");
        assert!(
            len + 0.5 >= PORTAL_ARRIVAL_RUN_PX,
            "the final run must be at least PORTAL_ARRIVAL_RUN_PX ({PORTAL_ARRIVAL_RUN_PX}) long: got {len} ({from:?} -> {to:?})"
        );

        let badges = badges_of(&svg);
        assert_eq!(badges.len(), 1, "one badge for the one-way crossing's single travel");
        let (letter, badge_pos) = &badges[0];
        assert_eq!(letter, "D", "a Down travel reads D");
        assert!(
            (badge_pos.1 - from.1).abs() < 1.0,
            "the badge's centre must sit within 1px of the final segment's own axis: \
             badge={badge_pos:?} segment y={} ({from:?} -> {to:?})",
            from.1
        );
        let (lo, hi) = (from.0.min(to.0), from.0.max(to.0));
        assert!(
            badge_pos.0 >= lo && badge_pos.0 <= hi,
            "the badge must sit ON the final segment, between its two ends: badge={badge_pos:?} run=[{lo},{hi}]"
        );

        assert!(label_collisions(&svg).is_empty());
        assert!(ghost_box_overlaps(&svg).is_empty());
        assert!(connector_room_crossings(&svg).is_empty());
    }

    // ── SQ-1368: a passage folded onto a shared line keeps its own marker ───────────────

    /// A→B carries a compass `E` AND a portal `Down`, both leaving A for B — the same shape as
    /// the field report's Canyon View→Down→Rocky Ledge plus Canyon View→E→Rocky Ledge. This is
    /// `collapse_stacked_exits` territory (SQ-1276), NOT `RoutedConnector::secondary_exit`: a
    /// portal never has a bearing to prefer over a compass one, so `Down` is suppressed at the
    /// SOURCE — before the router ever sees it — as room A's `stacked_exits`. This file used to
    /// draw nothing for a stacked exit at all; SQ-1373 closed that gap (see
    /// `a_stacked_exit_keeps_its_marker_in_both_renders`, below) — this case still asserts only
    /// `Up`, which is SQ-1368's own gap, not SQ-1373's.
    ///
    /// `E` wins and is routed as an ordinary one-way. What SQ-1368 actually fixes shows up one
    /// level later: B ALSO has its own `Up` back to A, and since `Up` can only pair with a
    /// `Down` (never a compass word), the router cannot join it to `E` as a reciprocal — so `Up`
    /// becomes `RoutedConnector::secondary_entry` on E's own connector, arriving back at A
    /// (SQ-1346's rule) instead of drawing its own line. Before this fix that arrival vanished
    /// entirely; now it gets the same lettered badge a plain portal arrival would.
    #[test]
    fn a_passage_folded_onto_a_shared_line_keeps_its_marker() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0)); // east of A, matching E's own bearing — keeps the line undistorted
        g.add_edge(1, Direction::Down, 2);
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::Up, 1);

        let svg = render_svg_of(&render(&g), Some(&g));
        let rooms = room_rects(&svg);
        assert_eq!(rooms.len(), 2, "the case must draw exactly the two rooms");
        let a = rooms[0];

        let tips = arrow_tips(&svg);
        assert_eq!(tips.len(), 1, "E is routed as one ordinary one-way head, into B");

        let b = rooms[1];
        let badges = badges_of(&svg);
        // SQ-1373: `Down` is `collapse_stacked_exits`'s OWN fold (SQ-1276), not the router's —
        // it now keeps a badge too, alongside `Up`'s router-level one this case was written for.
        assert_eq!(badges.len(), 2, "Up (router-folded) and Down (stacked) each keep a badge");
        let up = badges.iter().find(|(l, _)| l == "U").expect("Up's own badge").1;
        let down = badges.iter().find(|(l, _)| l == "D").expect("Down's own badge").1;
        assert!(near_rect_edge(up, a, 20.0), "Up travels B→A and so arrives at A: {up:?} vs a={a:?}");
        assert!(near_rect_edge(down, b, 20.0), "Down travels A→B and so arrives at B: {down:?} vs b={b:?}");

        // Neither badge may sit on top of a room box or overlap the room label.
        assert!(label_collisions(&svg).is_empty());
        assert!(connector_room_crossings(&svg).is_empty());
    }

    // ── SQ-1373: a stacked exit (SQ-1276) keeps its own marker too ──────────────────────

    /// Room A fans out to room B with THREE directions: `N` (matching B's true bearing, so it
    /// is `collapse_stacked_exits`'s primary and the only one routed/drawn), `E` (a second
    /// compass member, stacked away — wants a compass `tag`), and `Down` (a portal member,
    /// stacked away too — wants a badge). B also has its own `Up` back to A, which cannot pair
    /// with `N` (`can_pair` never joins a compass and a vertical direction) and so becomes
    /// `RoutedConnector::secondary_entry` on N's own connector — SQ-1368's OWN fix, arriving
    /// back at A. Three markers total, none of them the primary's own arrowhead: `E` (tag) and
    /// `Down` (badge `D`) at B, where all three of A's fanned-out directions travel to; `Up`
    /// (badge `U`) at A, where B's own back-travel arrives. This is the shape SQ-1368's own
    /// test case explicitly left open (see its doc comment).
    #[test]
    fn a_stacked_exit_keeps_its_marker_in_both_renders() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        // Two rows apart, not one — the U badge (arriving at A) and the E tag / D badge
        // (arriving at B) each want their own room in the gap between the boxes; a bare
        // one-row gap crushes all three into the same few px and the placer starts dropping
        // whichever loses the race, which is a geometry artifact of this fixture, not a defect.
        g.set_pos(2, (0, -2)); // north of A, matching N's own bearing
        g.add_edge(1, Direction::N, 2);
        g.add_edge(1, Direction::E, 2);
        g.add_edge(1, Direction::Down, 2);
        g.add_edge(2, Direction::Up, 1);

        let rm = render(&g);
        assert_eq!(
            rm.rooms.iter().find(|r| r.id == 1).unwrap().stacked_exits,
            vec![mapper::render::StackedExit {
                primary: Direction::N,
                dest: 2,
                secondary: vec![Direction::E, Direction::Down],
            }],
            "fixture check: N must win primary and E/Down must both stack — falsifies against \
             the wrong graph shape rather than a real defect below"
        );

        let svg = render_svg_of(&rm, Some(&g));
        let rooms = room_rects(&svg);
        assert_eq!(rooms.len(), 2, "the case must draw exactly the two rooms");
        let a = rooms[0];
        let b = rooms[1];

        let tips = arrow_tips(&svg);
        assert_eq!(tips.len(), 1, "N is routed as one ordinary one-way head, into B");

        let badges = badges_of(&svg);
        assert_eq!(badges.len(), 2, "Down (stacked) and Up (router-folded) each keep a badge");
        let d = badges.iter().find(|(l, _)| l == "D").expect("Down's own badge").1;
        let u = badges.iter().find(|(l, _)| l == "U").expect("Up's own badge").1;
        assert!(near_rect_edge(d, b, 20.0), "Down travels A→B and so arrives at B: {d:?} vs b={b:?}");
        assert!(near_rect_edge(u, a, 20.0), "Up travels B→A and so arrives at A: {u:?} vs a={a:?}");

        let tag = text_boxes(&svg)
            .into_iter()
            .find(|(cls, text, _)| cls == "tag" && text == "E")
            .expect("E's own compass tag");
        let tag_pos = (tag.2 .0, tag.2 .1);
        assert!(
            near_rect_edge(tag_pos, b, 24.0),
            "E travels A→B and so arrives at B: {tag_pos:?} vs b={b:?}"
        );

        // No two of the three markers (E's tag, Down's badge, Up's badge) may overlap each
        // other, a room box, or the room labels.
        let d_rect = badge_rect(d);
        let u_rect = badge_rect(u);
        let overlaps = |r1: PxRect, r2: PxRect| {
            r1.0 < r2.0 + r2.2 && r2.0 < r1.0 + r1.2 && r1.1 < r2.1 + r2.3 && r2.1 < r1.1 + r1.3
        };
        assert!(!overlaps(d_rect, u_rect), "Down's badge and Up's badge must not overlap: {d_rect:?} vs {u_rect:?}");
        assert!(!overlaps(d_rect, tag.2), "Down's badge and E's tag must not overlap: {d_rect:?} vs {:?}", tag.2);
        assert!(!overlaps(u_rect, tag.2), "Up's badge and E's tag must not overlap: {u_rect:?} vs {:?}", tag.2);

        assert!(label_collisions(&svg).is_empty());
        assert!(connector_room_crossings(&svg).is_empty());
    }

    // ── SQ-1384: a noted room carries its text as a tooltip AND a numbered footnote ──────

    /// A noted room's whole box is wrapped in a `<title>` carrying its note text; a room with no
    /// notes carries none. Falsify by reverting the `<g><title>` wrap in `render_svg_body`'s room
    /// pass and this fails on the title count.
    #[test]
    fn a_noted_room_carries_a_title_tooltip_and_others_do_not() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Cellar".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 1));
        g.add_edge(1, Direction::Down, 2);
        g.add_edge(2, Direction::Up, 1);
        g.set_notes(1, "a loose floorboard hides something".into());

        let svg = render_svg_of(&render(&g), Some(&g));
        let doc = roxmltree::Document::parse(&svg).expect("well-formed SVG");
        let titles: Vec<&str> =
            doc.descendants().filter(|n| n.tag_name().name() == "title").map(|n| n.text().unwrap_or("")).collect();
        assert_eq!(
            titles,
            vec!["a loose floorboard hides something"],
            "only the noted room may carry a title"
        );
    }

    /// Two noted rooms are numbered in READING ORDER (top-to-bottom by cell), and the panel's own
    /// "Notes" block lists both texts under matching numbers. Falsify by reverting the reading-order
    /// sort in `render_svg_body` to room-id order and this fails on which room gets "1".
    #[test]
    fn two_noted_rooms_get_badges_in_reading_order_and_a_notes_block() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        // Room ids run OPPOSITE to reading order on purpose — id 1 (Cellar) sits south of id 2
        // (Hall) — so a numbering that (wrongly) followed room-id order would give the very
        // answer this test rejects, rather than passing by coincidence.
        g.upsert_room(2, "Hall".into());
        g.upsert_room(1, "Cellar".into());
        g.set_pos(2, (0, 0));
        g.set_pos(1, (0, 1)); // south of Hall: reads AFTER it
        g.add_edge(2, Direction::Down, 1);
        g.add_edge(1, Direction::Up, 2);
        g.set_notes(2, "first note".into());
        g.set_notes(1, "second note".into());

        let svg = render_svg_of(&render(&g), Some(&g));
        let rooms = room_rects(&svg);
        assert_eq!(rooms.len(), 2, "the case must draw exactly the two rooms");
        let (hall, cellar) =
            if rooms[0].1 < rooms[1].1 { (rooms[0], rooms[1]) } else { (rooms[1], rooms[0]) };

        let badges = note_badges_of(&svg);
        assert_eq!(badges.len(), 2, "one badge per noted room");
        let inside =
            |p: (f64, f64), r: PxRect| p.0 >= r.0 && p.0 <= r.0 + r.2 && p.1 >= r.1 && p.1 <= r.1 + r.3;
        let hall_badge = badges.iter().find(|(_, p)| inside(*p, hall)).expect("Hall's own badge");
        let cellar_badge = badges.iter().find(|(_, p)| inside(*p, cellar)).expect("Cellar's own badge");
        assert_eq!(hall_badge.0, "1", "Hall reads first (top-to-bottom)");
        assert_eq!(cellar_badge.0, "2", "Cellar reads second");

        assert!(svg.contains(">Notes<"), "the Notes block heading must be drawn");
        assert!(svg.contains("1. first note"), "row 1 must carry Hall's own text");
        assert!(svg.contains("2. second note"), "row 2 must carry Cellar's own text");
    }

    /// A note containing `<`, `&` and a newline is escaped the same way in both places it is
    /// drawn — the box's own `<title>` tooltip and the panel's "Notes" block.
    #[test]
    fn a_note_with_special_characters_is_escaped_in_title_and_block() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.set_pos(1, (0, 0));
        g.set_notes(1, "a <script> & a\nsecond line".into());

        let svg = render_svg_of(&render(&g), Some(&g));
        assert!(
            svg.contains("<title>a &lt;script&gt; &amp; a\nsecond line</title>"),
            "the title must escape markup and keep the writer's own newline: {svg}"
        );
        assert!(
            svg.contains(">1. a &lt;script&gt; &amp; a</text>"),
            "the block's first line must escape markup too: {svg}"
        );
        assert!(
            svg.contains(">second line</text>"),
            "the block keeps the writer's own line break as a fresh row: {svg}"
        );
    }

    /// A layer with no noted room draws no "Notes" block at all — the block is additive, not a
    /// standing fixture of every panel.
    #[test]
    fn a_layer_without_notes_emits_no_notes_block() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Cellar".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 0));
        g.add_edge(1, Direction::Down, 2);
        g.add_edge(2, Direction::Up, 1);
        let below = g.new_layer(Some(mapper::layer::MAIN_LAYER), "Below".into());
        g.set_room_layer(2, below);
        g.set_notes(1, "Hall has a note".into());
        // Room 2, on the "Below" layer, carries no notes at all.

        let svg = render_svg_layered(&g);
        assert_eq!(svg.matches(">Notes<").count(), 1, "only the noted layer draws a block");
        let notes_idx = svg.find(">Notes<").expect("the one Notes heading");
        let below_heading_idx = svg.find("Below (").expect("the Below layer's own panel heading");
        assert!(
            notes_idx < below_heading_idx,
            "the Notes block is drawn inside Main's own panel, before Below's begins: {svg}"
        );
    }

    /// A layer's own `layer-frame` grows taller to hold its "Notes" block — falsify by reverting
    /// the block's height out of `render_svg_body`'s own returned height and this fails on the
    /// frame comparison (both panels come back the same height).
    #[test]
    fn a_notes_block_grows_the_frame_height() {
        use mapper::graph::MapGraph;
        fn build(note: bool) -> MapGraph {
            let mut g = MapGraph::new();
            g.upsert_room(1, "Hall".into());
            g.upsert_room(2, "Cellar".into());
            g.set_pos(1, (0, 0));
            g.set_pos(2, (0, 0));
            g.add_edge(1, Direction::Down, 2);
            g.add_edge(2, Direction::Up, 1);
            let below = g.new_layer(Some(mapper::layer::MAIN_LAYER), "Below".into());
            g.set_room_layer(2, below);
            if note {
                g.set_notes(
                    1,
                    "a long note about several things at once, long enough to wrap across \
                     more than one line of the panel's own Notes block"
                        .into(),
                );
            }
            g
        }
        fn main_frame_height(g: &MapGraph) -> f64 {
            let svg = render_svg_layered(g);
            let doc = roxmltree::Document::parse(&svg).expect("well-formed SVG");
            let mut frames: Vec<(f64, f64)> = doc
                .descendants()
                .filter(|n| n.tag_name().name() == "rect" && n.attribute("class") == Some("layer-frame"))
                .map(|n| {
                    let o = translate_of(n);
                    let y = o.1 + n.attribute("y").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                    let h = n.attribute("height").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                    (y, h)
                })
                .collect();
            frames.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            frames[0].1 // Main sorts first: its own LayerId is the lowest
        }
        let h_without = main_frame_height(&build(false));
        let h_with = main_frame_height(&build(true));
        assert!(
            h_with > h_without,
            "Main's own frame must grow to hold its Notes block: {h_with} vs {h_without}"
        );
    }
}
