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
         .random{{fill:#f8a;font-size:9px}}\
         .heading{{fill:#dde;font-size:14px}}\
         .legend{{fill:#dde;font-size:9px}}\
         .legend-panel{{fill:#20203a;stroke:#44446a;stroke-width:1}}\
         .legend-title{{fill:#fff;font-size:10px}}\
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

/// A room's box, in layout cells: `(left, top, width, height)`.
fn box_cell_rect(cols: &PosTable, rows: &PosTable, cell: (i32, i32)) -> (i32, i32, i32, i32) {
    (
        cols.room_pixel(cell.0),
        rows.room_pixel(cell.1),
        cols.box_dim_at(cell.0),
        rows.box_dim_at(cell.1),
    )
}

/// A room's box in SVG pixels.
fn box_px_rect(cols: &PosTable, rows: &PosTable, cell: (i32, i32)) -> (f64, f64, f64, f64) {
    let (bx, by, w, h) = box_cell_rect(cols, rows, cell);
    ((bx * CELL_W) as f64, (by * CELL_H) as f64, (w * CELL_W) as f64, (h * CELL_H) as f64)
}

/// The centre of layout cell `c` in SVG pixels.
fn cell_px(c: (i32, i32)) -> (f64, f64) {
    ((c.0 * CELL_W) as f64 + CELL_W as f64 / 2.0, (c.1 * CELL_H) as f64 + CELL_H as f64 / 2.0)
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
    /// The box side this passage leaves by. Up and Down take the top and bottom borders (as
    /// they do in the drawn view's portal slots); In and Out have no bearing of their own and
    /// take the right; a cross-layer COMPASS passage asks the router the same question a real
    /// connector's own perpendicular leg does.
    fn side(self) -> Side {
        match self.dir {
            Direction::Up => Side::Top,
            Direction::Down => Side::Bottom,
            Direction::In | Direction::Out => Side::Right,
            d => mapper::router::side_for(d).unwrap_or(Side::Right),
        }
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

/// A filled arrowhead sitting on the first ~8px of a connector leaving `at` along `u`.
fn arrowhead(at: (f64, f64), u: (f64, f64), class: &str) -> String {
    let tip = (at.0 + u.0 * 8.5, at.1 + u.1 * 8.5);
    let base = (at.0 + u.0 * 0.5, at.1 + u.1 * 0.5);
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
fn render_svg_body(
    rm: &RenderMap,
    weights: &HashMap<(RoomId, Direction), PassageWeight>,
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

    let cell_of: HashMap<RoomId, (i32, i32)> = rm.rooms.iter().map(|r| (r.id, r.cell)).collect();
    let rect_of = |id: RoomId| -> Option<(f64, f64, f64, f64)> {
        cell_of.get(&id).map(|&c| box_px_rect(&cols, &rows, c))
    };

    let mut ext = Extent::default();
    let mut edges = String::new(); // under the rooms
    let mut over = String::new(); // arrowheads, badges, tags — on top of the rooms
    let mut boxes = String::new();

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
        let mut pts: Vec<(f64, f64)> = plot.path.iter().map(|&c| cell_px(c)).collect();

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

        // Departure end: a portal's letter, everything else's arrowhead. An arrow on a room
        // border is that room's own EXIT (the terminal's one arrow rule, SQ-0688), so a
        // one-way passage gets nothing at its far end and the bare line IS the reading.
        let dep_u = outward(conn.exit);
        let arrow_class = if conn.distorted { "arrow distorted" } else { "arrow" };
        if is_portal {
            let at = (pts[0].0 + dep_u.0 * 8.0, pts[0].1 + dep_u.1 * 8.0);
            over.push_str(&badge(at, if conn.exit_dir == Direction::Up { "U" } else { "D" }));
            ext.add(at.0 - 8.0, at.1 - 8.0);
            ext.add(at.0 + 8.0, at.1 + 8.0);
        } else {
            over.push_str(&arrowhead(pts[0], dep_u, arrow_class));
            // The drawn side and the passage's own word disagree — a diagonal walked round
            // the corner orthogonally, or a distorted edge leaving by a side that is not its
            // direction. Say which word it was.
            if side_dir(conn.exit) != conn.exit_dir {
                let at = (pts[0].0 + dep_u.0 * 14.0 + 3.0, pts[0].1 + dep_u.1 * 14.0 - 3.0);
                let _ = write!(
                    over,
                    "<text class=\"tag\" x=\"{}\" y=\"{}\">{}</text>",
                    f(at.0),
                    f(at.1),
                    direction::short_label(conn.exit_dir).to_uppercase()
                );
                ext.add(at.0 + 16.0, at.1);
            }
        }

        if conn.reciprocal && !conn.merge {
            let last = pts[pts.len() - 1];
            let arr_u = outward(conn.entry);
            let arr_dir = conn.entry_dir.unwrap_or(direction::opposite(conn.exit_dir));
            if matches!(arr_dir, Direction::Up | Direction::Down) {
                let at = (last.0 + arr_u.0 * 8.0, last.1 + arr_u.1 * 8.0);
                over.push_str(&badge(at, if arr_dir == Direction::Up { "U" } else { "D" }));
                ext.add(at.0 - 8.0, at.1 - 8.0);
                ext.add(at.0 + 8.0, at.1 + 8.0);
            } else {
                over.push_str(&arrowhead(last, arr_u, arrow_class));
                if side_dir(conn.entry) != arr_dir {
                    let at = (last.0 + arr_u.0 * 14.0 + 3.0, last.1 + arr_u.1 * 14.0 - 3.0);
                    let _ = write!(
                        over,
                        "<text class=\"tag\" x=\"{}\" y=\"{}\">{}</text>",
                        f(at.0),
                        f(at.1),
                        direction::short_label(arr_dir).to_uppercase()
                    );
                    ext.add(at.0 + 16.0, at.1);
                }
            }
        }
    }

    // ── Portal / cross-layer badges ──────────────────────────────────────────────────────
    //
    // A stub is a passage with no planar route — up, down, in, out, or a compass passage whose
    // destination lives on another layer. It gets a lettered badge on the side it leads by,
    // and (when it crosses a layer) the destination's name beside it.
    let mut stubs_by_room: HashMap<RoomId, Vec<Stub<'_>>> = HashMap::new();
    for edge in &rm.edges {
        if !edge.is_stub || edge.dir == Direction::Unknown {
            continue;
        }
        stubs_by_room.entry(edge.origin).or_default().push(Stub {
            dir: edge.dir,
            dest: edge.dest_label.as_deref(),
            interlayer: edge.is_interlayer,
        });
    }
    for room in &rm.rooms {
        let Some(stubs) = stubs_by_room.get(&room.id) else { continue };
        let (bx, by, bw, bh) = box_px_rect(&cols, &rows, room.cell);
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
            for (i, &Stub { dir, dest, interlayer: inter }) in list.iter().enumerate() {
                let step = i as f64 * 17.0;
                let at = match side {
                    Side::Top => (bx + bw / 2.0 + step, by - 9.0),
                    Side::Bottom => (bx + bw / 2.0 + step, by + bh + 9.0),
                    Side::Left => (bx - 9.0, by + bh / 2.0 + step),
                    Side::Right => (bx + bw + 9.0, by + bh / 2.0 + step),
                };
                over.push_str(&badge(at, &direction::short_label(dir).to_uppercase()));
                ext.add(at.0 - 8.0, at.1 - 8.0);
                ext.add(at.0 + 8.0, at.1 + 8.0);
                if inter {
                    if let Some(name) = dest {
                        let lx = at.0 + u.0 * 9.0 + 2.0;
                        let ly = at.1 + u.1 * 9.0 + 3.0;
                        let _ = write!(
                            over,
                            "<text class=\"badge-dest\" x=\"{}\" y=\"{}\">{}</text>",
                            f(lx),
                            f(ly),
                            xml_escape(name)
                        );
                        ext.add(lx + name.chars().count() as f64 * 4.9, ly + 3.0);
                    }
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
        let rect = box_px_rect(&cols, &rows, room.cell);
        for &(dir, count) in &room.random_stubs {
            let Some(side) = mapper::router::side_for(dir) else { continue };
            let Some((arrow, out_cell)) = random_stub_cells(bx, by, bw, bh, dir) else { continue };
            let start = snap_to_edge(cell_px(arrow), rect, side);
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
        let (x, y, w, h) = box_px_rect(&cols, &rows, room.cell);
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
            let _ = write!(
                boxes,
                "<text class=\"{label_cls}\" x=\"{}\" y=\"{}\">{}</text>",
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
            format!("{}{}", line("edge oneway"), arrowhead((6.0, 0.0), (-1.0, 0.0), "arrow")),
            "one-way passage — the arrow is the way OUT",
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
    ]
}

/// The legend block, drawn with its top-left at `(0, 0)`. Returns `(markup, width, height)`.
fn legend() -> (String, i32, i32) {
    let rows = legend_rows();
    let h = LEGEND_ROW * rows.len() as i32 + 34;
    let mut s = format!(
        "<rect class=\"legend-panel\" x=\"0\" y=\"0\" width=\"{LEGEND_W}\" height=\"{h}\" rx=\"5\"/>\
         <text class=\"legend-title\" x=\"10\" y=\"15\">Legend</text>"
    );
    for (i, (sample, caption)) in rows.iter().enumerate() {
        let y = 26 + LEGEND_ROW * i as i32 + LEGEND_ROW / 2;
        let _ = write!(s, "<g transform=\"translate(6,{y})\">{sample}</g>");
        let _ = write!(
            s,
            "<text class=\"legend\" x=\"80\" y=\"{}\">{}</text>",
            y + 3,
            xml_escape(caption)
        );
    }
    (s, LEGEND_W, h)
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
    let Some((body, w, h)) = render_svg_body(rm, &weights) else {
        return "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"></svg>".to_string();
    };
    document(&body, w, h)
}

/// Render every non-empty layer of `graph` as one standalone SVG document, each layer its own
/// coordinate plane stacked top-to-bottom under a heading naming it (SQ-1308) — the same rule
/// [`crate::map_dump::render_dump`] draws its ASCII map by.
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
    const GAP: i32 = 22;
    let weights = weight_table(graph);
    let mut y = 0i32;
    let mut max_w = 0;
    let mut body = String::new();
    for &l in &layers {
        let rm = mapper::render::render_layer(graph, l);
        let heading = format!(
            "{}{} ({} rooms)",
            graph.layer_name(l),
            if graph.layer_is_maze(l) { " [maze]" } else { "" },
            graph.rooms_in_layer(l).len()
        );
        let _ = write!(
            body,
            "<text class=\"heading\" x=\"0\" y=\"{}\">{}</text>",
            y + 16,
            xml_escape(&heading)
        );
        max_w = max_w.max(heading.chars().count() as i32 * 9);
        y += HEADING_H;
        if let Some((frag, w, h)) = render_svg_body(&rm, &weights) {
            let _ = write!(body, "<g transform=\"translate(0,{y})\">{frag}</g>");
            y += h;
            max_w = max_w.max(w);
        }
        y += GAP;
    }
    document(&body, max_w.max(1), y.max(1))
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
        render_svg_body(rm, &weights).expect("a non-empty map").0
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
        for caption in ["one-way passage", "two-way passage", "door", "conditional exit", "distorted"] {
            assert!(svg.contains(caption), "legend must name {caption:?}");
        }
    }

    #[test]
    fn the_stylesheet_defines_every_documented_class() {
        let svg = render_svg(&render(&zork_house().graph));
        for sel in [".room", ".edge", ".edge.reciprocal", ".edge.distorted", ".edge.conditional", ".door", ".badge", ".legend"] {
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
    fn a_diagonal_drawn_orthogonally_is_tagged_with_its_own_word() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::NE));
        m.observe(1, "A", Some(Direction::SW));
        let svg = render_svg(&render(&m.graph));
        assert!(
            svg.contains("class=\"tag\""),
            "a diagonal walked round the corner names the direction it really is"
        );
        assert!(svg.contains(">NE<") || svg.contains(">SW<"));
    }

    #[test]
    fn an_up_passage_with_no_planar_route_gets_a_lettered_badge() {
        let mut m = Mapper::default();
        m.observe(1, "Cellar", None);
        m.observe(2, "Attic", Some(Direction::Up));
        let svg = render_svg(&render(&m.graph));
        assert!(svg.contains("class=\"badge\""), "up/down show as a badge, never a Nerd Font glyph");
        assert!(svg.contains(">U<") || svg.contains(">D<"));
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
}
