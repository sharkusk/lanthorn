//! Map projection and ratatui rendering.
//!
//! # Coordinate system
//!
//! Logical room cells (col, row) are placed on a grid.  Each zoom level defines a "step"
//! (cell stride in terminal columns/rows):
//!
//! | Zoom     | step_w | step_h |
//! |----------|--------|--------|
//! | Boxes    |  29    |  17    |
//! | Compact  |  12    |   5    |
//! | Overview |   2    |   2    |
//!
//! The screen position of a room at cell (cx, cy) with scroll (sx, sy) inside area `a` is:
//!   screen_x = a.x + (cx - sx) * step_w
//!   screen_y = a.y + (cy - sy) * step_h
//!
//! # Fine-grid connector projection
//!
//! Connectors live in a fine grid where room cell (c, r) → fine (2c, 2r).
//! A fine point (fx, fy) maps to screen as:
//!   screen_x = a.x + (fx - scroll.0 * 2) * (step_w / 2) + gutter_offset_x
//!   screen_y = a.y + (fy - scroll.1 * 2) * (step_h / 2) + gutter_offset_y
//!
//! For Overview zoom, connectors are skipped (step/2=1 but single-glyph boxes fill the cell).

use mapper::graph::RoomId;
use mapper::render::{RenderMap, RenderRoom};
use mapper::route::RoutePlan;
use mapper::router::{RoutedEdge, Side};
use mapper::direction::Direction;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::state::{AppState, Zoom};
use crate::symbols::{BoxStyle, SymbolSet};

// ── Pulsing border ────────────────────────────────────────────────────────────

/// Pulse frequency in Hz (cycles per second) for the background-tidy border animation.
pub const PULSE_HZ: f64 = 1.0;
/// Red endpoint of the pulse (220, 60, 60).
const PULSE_RED: (u8, u8, u8) = (220, 60, 60);
/// Green endpoint of the pulse (60, 200, 90).
const PULSE_GREEN: (u8, u8, u8) = (60, 200, 90);

/// Compute the pulsed map-border color for a given elapsed time since job spawn.
///
/// The color oscillates between `PULSE_RED` and `PULSE_GREEN` at `PULSE_HZ` Hz
/// using a sine-based lerp:
///   f = (sin(t * TAU * PULSE_HZ) + 1) / 2  →  [0, 1]
///
/// At `elapsed = 0` (phase 0, sin = 0) the result is the midpoint.
/// At quarter-period (sin = 1, f = 1) the result is the green endpoint.
/// At three-quarter-period (sin = -1, f = 0) the result is the red endpoint.
///
/// Called only when a tidy job is in flight; the caller picks `normal` when idle.
pub fn pulse_border_color(elapsed: std::time::Duration) -> Color {
    let t = elapsed.as_secs_f64();
    let f = ((t * std::f64::consts::TAU * PULSE_HZ).sin() + 1.0) / 2.0;
    let lerp = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * f).round() as u8;
    Color::Rgb(
        lerp(PULSE_RED.0, PULSE_GREEN.0),
        lerp(PULSE_RED.1, PULSE_GREEN.1),
        lerp(PULSE_RED.2, PULSE_GREEN.2),
    )
}

/// Duration of the one-shot story-border flash for a `sound_effect` bleep.
pub const SOUND_PULSE_MS: u64 = 500;

/// Extract RGB channels from a `Color`, or `None` for non-RGB colors
/// (named/indexed/Reset have no fixed RGB to interpolate toward).
fn rgb_of(c: Color) -> Option<(u8, u8, u8)> {
    if let Color::Rgb(r, g, b) = c {
        Some((r, g, b))
    } else {
        None
    }
}

/// One-shot fade for a sound bleep: full `beep` color at `elapsed == 0`, lerping
/// toward `normal` as `elapsed` approaches `SOUND_PULSE_MS`. Returns `None` once
/// the window has elapsed (the caller then clears the pulse and the border
/// renders normally). When `normal` is not an RGB color (e.g. a terminal/named
/// border color), fade toward a dimmed copy of the beep color instead.
pub fn sound_pulse_color(
    beep: Color,
    normal: Color,
    elapsed: std::time::Duration,
) -> Option<Color> {
    let ms = elapsed.as_millis() as u64;
    if ms >= SOUND_PULSE_MS {
        return None;
    }
    let (br, bg, bb) = rgb_of(beep).unwrap_or((255, 180, 40));
    let (nr, ng, nb) = rgb_of(normal).unwrap_or((br / 4, bg / 4, bb / 4));
    let f = ms as f64 / SOUND_PULSE_MS as f64; // 0.0 -> 1.0 across the window
    let lerp = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * f).round() as u8;
    Some(Color::Rgb(lerp(br, nr), lerp(bg, ng), lerp(bb, nb)))
}

// ── Step sizes and box dimensions ─────────────────────────────────────────────

/// Returns (step_w, step_h) for the given zoom level.
fn zoom_steps(zoom: Zoom) -> (i32, i32) {
    zoom.steps()
}

/// Returns (box_w, box_h): the visual size of a room box drawn within one cell step.
///
/// The box is drawn SMALLER than the step so there is a gutter on the right/bottom
/// where connector glyphs are visible between adjacent rooms.
///
/// | Zoom    | step  | box   | gutter (right / bottom) |
/// |---------|-------|-------|-------------------------|
/// | Boxes   | 19×11 | 11×5  | 8 cols / 6 rows         |
/// | Compact | 12×5  | 8×3   | 4 cols / 2 rows         |
/// | Overview| 2×2   | 1×1   | — (single glyph)        |
///
/// The 11×5 box (both odd) is ~2:1 width:height so it reads as square given the
/// terminal's ~1:2 cell aspect, and odd dims centre the side anchors on the box.
fn zoom_box_size(zoom: Zoom) -> (u16, u16) {
    match zoom {
        Zoom::Boxes => (11, 5),
        Zoom::Compact => (8, 3),
        Zoom::Overview => (1, 1),
    }
}

// ── Virtual map space ─────────────────────────────────────────────────────────
//
// The whole map is built in a scroll-independent "virtual" coordinate space where
// a room at logical cell (c, r) sits at pixel (c * step_w, r * step_h). Rooms and
// connectors are placed and routed here ONCE, regardless of scroll, so the routes
// never change as the view pans. Scrolling is then a pure translate-and-clip blit:
// screen = virtual + (area.origin - scroll * step).

/// An integer rectangle in virtual map space (coordinates may be negative).
#[derive(Debug, Clone, Copy)]
struct VRect {
    x: i32,
    y: i32,
    w: i32,
}

impl VRect {
    fn right(&self) -> i32 {
        self.x + self.w
    }
}

/// Virtual top-left pixel of a room cell: `cell * step` (no scroll, no area offset).
fn cell_to_virtual(cell: (i32, i32), zoom: Zoom) -> (i32, i32) {
    let (sw, sh) = zoom_steps(zoom);
    (cell.0 * sw, cell.1 * sh)
}

// ── Boxes-zoom position tables ────────────────────────────────────────────────

/// Cells between adjacent lanes in a channel (so lines are visually separated).
const LANE_SPACING: i32 = 2;
/// Gap between the box edge (doorway) and lane 0, so channel runs never graze the box edge
/// where same-side departure/arrival anchors live.
const LANE_BASE: i32 = 1;
/// Minimum channel pixel size even when it carries no lanes.
pub(crate) const MIN_GUTTER: i32 = 2;
/// Boxes-zoom box size (matches `zoom_box_size(Zoom::Boxes)`), in cells.
pub(crate) const BOX_W: i32 = 11;
pub(crate) const BOX_H: i32 = 5;

/// One axis of the non-uniform Boxes-zoom layout: where each room line starts (pixels)
/// and how wide each channel after it is.
#[derive(Debug)]
pub struct PosTable {
    room_start: std::collections::BTreeMap<i32, i32>, // grid line index → pixel start of the box
    channel_w: std::collections::BTreeMap<i32, i32>,  // grid line index → pixel width of the gap after it
    lo: i32,                                           // lowest grid line index
    hi: i32,                                           // highest grid line index
    box_dim: i32,                                      // box size along this axis (pixels)
}
impl PosTable {
    pub fn room_pixel(&self, idx: i32) -> i32 { self.line_pixel(idx) }
    pub fn channel_span(&self, idx: i32) -> i32 { *self.channel_w.get(&idx).unwrap_or(&MIN_GUTTER) }

    /// Total pixel extent from the first room's box-left to just past the last room's
    /// trailing channel. This is the minimum pixel span needed to draw all rooms and
    /// their inter-room channels without clipping.
    pub fn total_pixels(&self) -> i32 {
        let last = self.room_pixel(self.hi);
        last + self.box_dim + self.channel_span(self.hi)
    }

    /// Pixel-x (or -y) of the box left/top edge at grid line `idx`, extrapolating with a
    /// uniform `box_dim + MIN_GUTTER` stride for lines outside the tabulated bounds so
    /// scrolling beyond the placed rooms stays well-defined and continuous.
    fn line_pixel(&self, idx: i32) -> i32 {
        if let Some(&p) = self.room_start.get(&idx) {
            p
        } else if idx < self.lo {
            // Steps of the default (empty-channel) stride below the first room.
            self.room_start.get(&self.lo).copied().unwrap_or(0)
                - (self.lo - idx) * (self.box_dim + MIN_GUTTER)
        } else {
            // Past the last room: its start, its own box+channel, then default strides.
            let last = self.room_start.get(&self.hi).copied().unwrap_or(0);
            let after = last + self.box_dim + self.channel_span(self.hi);
            after + (idx - self.hi - 1) * (self.box_dim + MIN_GUTTER)
        }
    }
}

fn channel_width(lanes: u16) -> i32 {
    // Reserve LANE_BASE before lane 0 plus LANE_SPACING per additional lane, so the widest
    // lane (LANE_BASE + (lanes-1)*LANE_SPACING) stays inside the channel. Empty channels keep
    // MIN_GUTTER so adjacent boxes never touch.
    if lanes == 0 {
        MIN_GUTTER
    } else {
        (LANE_BASE + (lanes as i32 - 1) * LANE_SPACING + 1).max(MIN_GUTTER)
    }
}

/// Build the (columns, rows) position tables from the plan and the room bounds.
pub fn boxes_axes(plan: &RoutePlan, bounds: ((i32, i32), (i32, i32))) -> (PosTable, PosTable) {
    let ((min_c, min_r), (max_c, max_r)) = bounds;
    let build = |lo: i32,
                 hi: i32,
                 box_dim: i32,
                 lanes: &std::collections::BTreeMap<i32, u16>,
                 floor: &std::collections::BTreeMap<i32, i32>| {
        let mut room_start = std::collections::BTreeMap::new();
        let mut channel_w = std::collections::BTreeMap::new();
        let mut x = 0;
        for idx in lo..=hi {
            room_start.insert(idx, x);
            let w = channel_width(lanes.get(&idx).copied().unwrap_or(0))
                .max(floor.get(&idx).copied().unwrap_or(0));
            channel_w.insert(idx, w);
            x += box_dim + w;
        }
        PosTable { room_start, channel_w, lo, hi, box_dim }
    };
    // Rows first: a diagonal's COLUMN demand is expressed relative to the row gap it must cross
    // (SQ-0314), so the vertical spacing has to be known before the horizontal is sized.
    //
    // A route may leave the rooms' own bounds — an edge that wraps around its destination uses the
    // channel beyond the last room. `build` only tabulates `lo..=hi` and `channel_span` answers
    // `MIN_GUTTER` for anything outside, so a diagonal's floor out there would be silently dropped
    // and the diagonal would vanish. Widen the range to cover every channel a diagonal uses.
    let mut row_floor: std::collections::BTreeMap<i32, i32> = std::collections::BTreeMap::new();
    for &(_, h) in &plan.diag_corners {
        row_floor.insert(h, DIAG_GUTTER);
    }
    let (min_r, max_r) = span_over(min_r, max_r, plan.diag_corners.iter().map(|&(_, h)| h));
    let rows = build(min_r, max_r, BOX_H, &plan.h_lanes, &row_floor);
    let mut col_floor: std::collections::BTreeMap<i32, i32> = std::collections::BTreeMap::new();
    for &(v, h) in &plan.diag_corners {
        let need = diagonal_col_gap(rows.channel_span(h));
        let slot = col_floor.entry(v).or_insert(0);
        *slot = (*slot).max(need);
    }
    let (min_c, max_c) = span_over(min_c, max_c, plan.diag_corners.iter().map(|&(v, _)| v));
    let cols = build(min_c, max_c, BOX_W, &plan.v_lanes, &col_floor);
    (cols, rows)
}

/// Widen `lo..=hi` to cover every channel index in `chans`. Channel `i` lies between grid lines `i`
/// and `i+1`, so tabulating it needs both. Room positions are unaffected: `build` lays lines out in
/// order from `lo`, and the extra lines only ever sit outside the rooms' own span.
fn span_over(lo: i32, hi: i32, chans: impl Iterator<Item = i32>) -> (i32, i32) {
    let (mut lo, mut hi) = (lo, hi);
    for i in chans {
        lo = lo.min(i);
        hi = hi.max(i + 1);
    }
    (lo, hi)
}

/// Minimum gap on BOTH axes at a corner some diagonal passes through (SQ-0314).
///
/// `MIN_GUTTER` (2) is enough for the diagonally-ADJACENT case, which runs corner to corner and so
/// spans `gap + 1`. It is not enough for any other diagonal: those hand off to a channel lane, and
/// lane 0 sits `LANE_BASE` inside the gap, leaving only `gap - LANE_BASE` = 1 row to climb — too
/// little for even one `🮣🮠` pair, so the diagonal would vanish into an orthogonal dogleg. One more
/// cell of gutter buys the two rows a pair needs.
const DIAG_GUTTER: i32 = 3;

/// The column gap a diagonal needs to cross a row gap of `row_gap` without a dogleg (SQ-0314).
///
/// `diagonal_chain` can draw any ratio — it spends a surplus on either axis as fill — so this is
/// about how the result READS, not whether it can be drawn. A square gap gives one column per row
/// (a 63° climb on a ~1:2 cell); wider gives a shallower line, narrower a steeper one. Matching the
/// column gap to the row gap keeps diagonals looking consistent across a map whose channels carry
/// wildly different lane counts.
///
/// Never below `MIN_GUTTER`, so this can only ever widen a gap.
fn diagonal_col_gap(row_gap: i32) -> i32 {
    row_gap.max(MIN_GUTTER)
}


/// Arrow glyph for a diagonal departure/arrival (caller guards with `is_diagonal`).
fn diagonal_arrow(dir: Direction, arrows: &crate::symbols::Arrows) -> char {
    match dir {
        Direction::NE => arrows.ne,
        Direction::NW => arrows.nw,
        Direction::SE => arrows.se,
        Direction::SW => arrows.sw,
        _ => arrows.ne, // unreachable when guarded by is_diagonal
    }
}

/// The box-corner cell (virtual pixels) for a diagonal direction: NE→top-right, NW→top-left,
/// SE→bottom-right, SW→bottom-left.
fn corner_anchor(cols: &PosTable, rows: &PosTable, cell: (i32, i32), dir: Direction) -> (i32, i32) {
    let bx = cols.room_pixel(cell.0);
    let by = rows.room_pixel(cell.1);
    match dir {
        Direction::NE => (bx + BOX_W - 1, by),
        Direction::NW => (bx, by),
        Direction::SE => (bx + BOX_W - 1, by + BOX_H - 1),
        Direction::SW => (bx, by + BOX_H - 1),
        _ => (bx + BOX_W / 2, by), // unreachable when guarded by is_diagonal
    }
}

/// Return the arrowhead glyph that points OUTWARD from the origin along `dep_side`.
fn arrow_for_departure(dep_side: Side, arrows: &crate::symbols::Arrows) -> char {
    match dep_side {
        Side::Right  => arrows.east,
        Side::Left   => arrows.west,
        Side::Top    => arrows.north,
        Side::Bottom => arrows.south,
    }
}

/// True if screen cell `(sx, sy)` lies inside `area`.
fn in_area(sx: i32, sy: i32, area: Rect) -> bool {
    sx >= area.x as i32 && sx < area.right() as i32 && sy >= area.y as i32 && sy < area.bottom() as i32
}

/// Style for a room given the current selection/current state.
///
/// When a room is BOTH current AND selected, combine both states: use the
/// selected background with the REVERSED modifier from room_current so the
/// room is visually distinct from either state alone.
fn room_style(room: &RenderRoom, state: &AppState) -> Style {
    let is_selected = state.selected_room == Some(room.id);
    let theme = &state.colors.theme;
    if room.is_current && is_selected {
        theme.get("map.room_selected").style.add_modifier(Modifier::REVERSED)
    } else if room.is_current {
        theme.get("map.room_current").style
    } else if is_selected {
        theme.get("map.room_selected").style
    } else {
        theme.get("map.room").style
    }
}

// ── cell_to_screen / screen_to_cell / room_at_cell ───────────────────────────

/// Map a logical room cell to an absolute screen coordinate within `area`.
///
/// Returns `None` if the resulting position falls outside `area`.
pub fn cell_to_screen(
    cell: (i32, i32),
    zoom: Zoom,
    scroll: (i32, i32),
    area: Rect,
) -> Option<(u16, u16)> {
    let (step_w, step_h) = zoom_steps(zoom);
    let sx = area.x as i32 + (cell.0 - scroll.0) * step_w;
    let sy = area.y as i32 + (cell.1 - scroll.1) * step_h;

    // Bounds check: must be inside [area.x, area.right()) × [area.y, area.bottom())
    if sx < area.x as i32
        || sx >= area.right() as i32
        || sy < area.y as i32
        || sy >= area.bottom() as i32
    {
        return None;
    }
    Some((sx as u16, sy as u16))
}

/// Map an absolute screen coordinate back to a logical room cell — the exact
/// inverse of `cell_to_screen`.
///
/// `cell.x = (screen.x - area.x) / step_w + scroll.x` (integer division).
/// The result is a grid cell; whether a room actually occupies it is determined
/// separately by `room_at_cell`.
pub fn screen_to_cell(screen: (i32, i32), zoom: Zoom, scroll: (i32, i32), area: Rect) -> (i32, i32) {
    let (step_w, step_h) = zoom_steps(zoom);
    let cx = (screen.0 - area.x as i32).div_euclid(step_w) + scroll.0;
    let cy = (screen.1 - area.y as i32).div_euclid(step_h) + scroll.1;
    (cx, cy)
}

/// Return the screen-space bounding `Rect` for every room in `rm`, clipped to
/// `area`. Uses the same offset logic as `render_map` so click hit-testing is
/// pixel-accurate at all zoom levels, including the non-uniform Boxes layout.
///
/// Rooms whose box falls completely outside `area` are omitted. Rooms that are
/// only partially visible are clipped to `area`.
pub fn room_screen_rects(
    rm: &mapper::render::RenderMap,
    state: &crate::state::AppState,
    area: Rect,
) -> Vec<(mapper::graph::RoomId, Rect)> {
    let zoom = state.zoom;
    let scroll = state.scroll;
    let (bw, bh) = zoom_box_size(zoom);

    // The same cached tables `render_map` draws from (SQ-1182) — this runs right
    // after it on every drawn frame, and was rebuilding `boxes_axes` again.
    let derived = derived_tables(rm, state, zoom);
    let axes = &derived.axes;
    let (off_x, off_y) = match axes {
        Some((cols, rows)) => (
            area.x as i32 - cols.room_pixel(scroll.0) + state.char_pan.0,
            area.y as i32 - rows.room_pixel(scroll.1) + state.char_pan.1,
        ),
        None => {
            let (step_w, step_h) = zoom_steps(zoom);
            (area.x as i32 - scroll.0 * step_w + state.char_pan.0,
             area.y as i32 - scroll.1 * step_h + state.char_pan.1)
        }
    };
    let room_virtual = |cell: (i32, i32)| -> (i32, i32) {
        match axes {
            Some((cols, rows)) => (cols.room_pixel(cell.0), rows.room_pixel(cell.1)),
            None => cell_to_virtual(cell, zoom),
        }
    };

    let mut rects = Vec::with_capacity(rm.rooms.len());
    for room in &rm.rooms {
        let (vx, vy) = room_virtual(room.cell);
        let sx = vx + off_x;
        let sy = vy + off_y;
        // Skip completely off-screen rooms.
        if sx >= area.right() as i32
            || sy >= area.bottom() as i32
            || sx + bw as i32 <= area.x as i32
            || sy + bh as i32 <= area.y as i32
        {
            continue;
        }
        // Clamp to area.
        let rx = (sx.max(area.x as i32)) as u16;
        let ry = (sy.max(area.y as i32)) as u16;
        let rx2 = ((sx + bw as i32).min(area.right() as i32)) as u16;
        let ry2 = ((sy + bh as i32).min(area.bottom() as i32)) as u16;
        if rx2 <= rx || ry2 <= ry {
            continue;
        }
        rects.push((room.id, Rect::new(rx, ry, rx2 - rx, ry2 - ry)));
    }
    rects
}

/// Return the `RoomId` of the room in `layer` at grid `cell`, or `None` if no
/// placed room sits at exactly that cell.  Clicks in the gutter between boxes
/// (where `pos` would fall on a non-integer part of the grid) naturally land on
/// a cell that no room occupies, so they return `None`.
pub fn room_at_cell(
    graph: &mapper::graph::MapGraph,
    layer: mapper::layer::LayerId, // LayerId is u8 (pub type alias in mapper)
    cell: (i32, i32),
) -> Option<RoomId> {
    for id in graph.rooms_in_layer(layer) {
        if let Some(room) = graph.room(id) {
            if room.pos == Some(cell) {
                return Some(id);
            }
        }
    }
    None
}

// ── Styles ────────────────────────────────────────────────────────────────────
//
// Room and connector styles are now read from `state.colors` at render time
// rather than from compile-time constants.  The constants have been removed.
// See `room_style()` and the connector-drawing functions for usage.

// ── Derived tables (SQ-1182) ─────────────────────────────────────────────────

/// The scroll-independent tables one routed model implies at one zoom: every
/// room's virtual-space rect, the Boxes-zoom position tables, and the per-edge
/// kind classification. None depend on scroll or pan — they were nevertheless
/// rebuilt on every drawn frame, over every room, the whole grid span and every
/// edge, during 30 fps animation windows included.
///
/// All inputs are the model and the zoom, so the LIVE model's tables are cached
/// in [`AppState::map_derived`] and rebuilt only when the model is replaced
/// (`poll_render_job` clears the cache) or the zoom changes. Replay, tidy-anim
/// and test models are built fresh per frame, exactly as before — they are not
/// tracked by `graph_gen`, so nothing keyed on it may describe them.
#[derive(Debug)]
pub(crate) struct MapDerived {
    /// The zoom this was derived at — part of the cache key.
    zoom: Zoom,
    /// Every room's virtual-space rect (step 1 of `render_map`).
    placed: std::collections::HashMap<RoomId, VRect>,
    /// The Boxes-zoom lane-routing position tables; `None` at other zooms.
    axes: Option<(PosTable, PosTable)>,
    /// Per-edge kind classification; only built (and only read) at Boxes zoom.
    kinds: std::collections::HashMap<(RoomId, RoomId, Direction), EdgeKind>,
}

impl MapDerived {
    /// How many rooms the placement table covers — the freshness probe the
    /// `state.rs` cache-invalidation test reads (SQ-1182).
    #[cfg(all(test, feature = "t-state"))]
    pub(crate) fn rooms_placed(&self) -> usize {
        self.placed.len()
    }
}

fn build_derived(rm: &RenderMap, zoom: Zoom) -> MapDerived {
    let boxes = matches!(zoom, Zoom::Boxes);
    let axes = boxes.then(|| boxes_axes(&rm.plan, rm.bounds));
    let (bw, _bh) = zoom_box_size(zoom);
    let mut placed: std::collections::HashMap<RoomId, VRect> =
        std::collections::HashMap::with_capacity(rm.rooms.len());
    for room in &rm.rooms {
        let (vx, vy) = match &axes {
            Some((cols, rows)) => (cols.room_pixel(room.cell.0), rows.room_pixel(room.cell.1)),
            None => cell_to_virtual(room.cell, zoom),
        };
        placed.insert(room.id, VRect { x: vx, y: vy, w: bw as i32 });
    }
    let kinds = if boxes { edge_kinds(rm) } else { std::collections::HashMap::new() };
    MapDerived { zoom, placed, axes, kinds }
}

/// The derived tables for `rm` at `zoom` — reused from [`AppState::map_derived`]
/// when `rm` IS the live cached model, built fresh otherwise.
///
/// Liveness is decided by address: the production path passes a `Ref`-projected
/// `&MapRenderCache::rm`, so pointer identity to the entry in `state.map_render`
/// is exact — a replay graph, a tidy-animation frame or a test's local model can
/// never alias it. The key carries the entry's own `(gen, layer)` (not
/// `state.graph_gen`, which runs ahead of a stale model mid-reroute) plus the
/// zoom; `poll_render_job` clears the cache whenever it installs a new model, so
/// a same-`(gen, layer)` replacement (the empty placeholder giving way to the
/// first real route) cannot serve tables derived from the placeholder.
fn derived_tables<'a>(rm: &RenderMap, state: &'a AppState, zoom: Zoom) -> DerivedSource<'a> {
    let live_key = state.map_render.try_borrow().ok().and_then(|mr| {
        mr.as_ref().and_then(|c| std::ptr::eq(&c.rm, rm).then_some((c.gen, c.layer)))
    });
    match live_key {
        Some((gen, layer)) => {
            let hit = matches!(
                &*state.map_derived.borrow(),
                Some((g, l, d)) if *g == gen && *l == layer && d.zoom == zoom
            );
            if !hit {
                *state.map_derived.borrow_mut() = Some((gen, layer, build_derived(rm, zoom)));
            }
            DerivedSource::Cached(std::cell::Ref::map(state.map_derived.borrow(), |o| {
                &o.as_ref().expect("populated above").2
            }))
        }
        None => DerivedSource::Fresh(Box::new(build_derived(rm, zoom))),
    }
}

/// Where [`derived_tables`] found its answer; derefs to the tables either way.
enum DerivedSource<'a> {
    Cached(std::cell::Ref<'a, MapDerived>),
    // Boxed so the enum stays pointer-sized either way (clippy: large_enum_variant).
    Fresh(Box<MapDerived>),
}

impl std::ops::Deref for DerivedSource<'_> {
    type Target = MapDerived;
    fn deref(&self) -> &MapDerived {
        match self {
            DerivedSource::Cached(r) => r,
            DerivedSource::Fresh(d) => d,
        }
    }
}

// ── render_map ────────────────────────────────────────────────────────────────

/// Draw the map from `rm` into `buf` for `area`, using view state from `state`.
///
/// The whole map is built in scroll-independent virtual space (see `VRect`) and
/// blitted to the screen with a single translation, so panning never re-routes
/// connectors — the routes are identical at every scroll offset.
pub fn render_map(rm: &RenderMap, state: &AppState, area: Rect, buf: &mut Buffer) {
    let zoom = state.zoom;
    let scroll = state.scroll;

    // Build-frame manifest: when the active tidy frame carries a manifest, draw it
    // as text in the map pane and skip room drawing. Overflow past the pane is
    // truncated (diagnostic view).
    if let Some(anim) = &state.tidy_anim {
        if let Some(lines) = anim.current().manifest.as_ref() {
            // The tidy transport panel overlays the top-left of the map pane (see
            // draw_tidy_panel); start the manifest below it, when the panel is drawn,
            // so the panel doesn't cover the connection list.
            let top = if area.width >= crate::render::tidy_panel::PANEL_W
                && area.height >= crate::render::tidy_panel::PANEL_H
            {
                crate::render::tidy_panel::PANEL_H
            } else {
                0
            };
            let avail_h = area.height.saturating_sub(top);
            let transcript_style = state.colors.theme.get("transcript").style;
            for (i, line) in lines.iter().take(avail_h as usize).enumerate() {
                let clamped: String = line.chars().take(area.width as usize).collect();
                put_str(buf, area.x as i32, (area.y + top) as i32 + i as i32, &clamped,
                    transcript_style, area);
            }
            return;
        }
    }

    // Overview zoom: one glyph per room, no connectors. Uniform stride.
    if matches!(zoom, crate::state::Zoom::Overview) {
        let (step_w, step_h) = zoom_steps(zoom);
        let off_x = area.x as i32 - scroll.0 * step_w + state.char_pan.0;
        let off_y = area.y as i32 - scroll.1 * step_h + state.char_pan.1;
        for room in &rm.rooms {
            let (vx, vy) = cell_to_virtual(room.cell, zoom);
            put_char(buf, vx + off_x, vy + off_y, '■', room_style(room, state), area);
        }
        return;
    }

    // Boxes zoom uses the non-uniform lane-routing position tables; Compact keeps the
    // uniform schematic stride. `room_virtual` maps a logical cell to its virtual
    // top-left pixel; the scroll offset is computed in the SAME space so panning is a
    // pure translate-and-clip and connector geometry is scroll-invariant.
    let boxes = matches!(zoom, crate::state::Zoom::Boxes);
    // ── 1. The scroll-independent tables: room placement, position tables, edge
    //       kinds — cached for the live model, fresh for any other (SQ-1182).
    let derived = derived_tables(rm, state, zoom);
    let axes = &derived.axes;
    let placed = &derived.placed;
    let (off_x, off_y) = match axes {
        Some((cols, rows)) => (
            area.x as i32 - cols.room_pixel(scroll.0) + state.char_pan.0,
            area.y as i32 - rows.room_pixel(scroll.1) + state.char_pan.1,
        ),
        None => {
            let (step_w, step_h) = zoom_steps(zoom);
            (area.x as i32 - scroll.0 * step_w + state.char_pan.0,
             area.y as i32 - scroll.1 * step_h + state.char_pan.1)
        }
    };
    let room_virtual = |cell: (i32, i32)| -> (i32, i32) {
        match axes {
            Some((cols, rows)) => (cols.room_pixel(cell.0), rows.room_pixel(cell.1)),
            None => cell_to_virtual(cell, zoom),
        }
    };

    // ── 2. Stub (portal) edges at non-Boxes zoom keep the bare-label `draw_stub`; Boxes zoom draws
    //       the in-room portal-icon overlay after the rooms (below).
    let connector_style = state.colors.theme.get("map.connector").style;
    for edge in &rm.edges {
        if edge.is_stub && !boxes {
            draw_stub(edge, placed, off_x, off_y, area, buf, connector_style);
        }
    }

    // ── 3. Boxes zoom: draw line-art connectors along their assigned lanes, on top of
    //       the rooms drawn below them in step 2.
    let mut arrowheads: Vec<Arrowhead> = Vec::new();
    if let Some((cols, rows)) = axes {
        arrowheads = render_lane_connectors(&rm.plan, cols, rows, (off_x, off_y), area, buf, &state.symbols.arrows, &state.symbols.path, &state.symbols.portal, &state.colors, state.symbols.diagonal_corners, &derived.kinds);
    }

    // ── 4. Draw rooms on top of the line-art (translate + clip) ───────────────
    for room in &rm.rooms {
        let (vx, vy) = room_virtual(room.cell);
        let sx = vx + off_x;
        let sy = vy + off_y;
        draw_room(room, state, zoom, sx, sy, area, buf);
    }

    // Portal-icon overlay (Boxes zoom), drawn after the rooms so icons sit on the box. In
    // normal view the icons go on the interior right column; in portal view (show_portal_labels)
    // they move onto the border and the destination names float outside the box.
    if boxes {
        draw_portal_icons(rm, placed, state, state.show_portal_labels, (off_x, off_y), area, buf);
    }

    // ── 5. Draw departure/arrival arrowheads LAST, so each embeds in the room ─
    //       border it sits on (replacing the box-edge glyph, pointing outward).
    // Portal view hides the cardinal connector arrowheads so only portal icons sit on borders.
    if !state.show_portal_labels {
        let current_room = rm.rooms.iter().find(|r| r.is_current).map(|r| r.id);
        draw_connector_arrows(&arrowheads, (off_x, off_y), area, buf, &state.colors, state.selected_room, current_room);
    }
}

// ── Layer tab strip ───────────────────────────────────────────────────────────

/// The tab title for `layer`: its name and room count, with a trailing `⌗` marker when the
/// layer is flagged a maze (SQ-0672). Shared by both tab strips — the bordered panel-header
/// inset (`main.rs`) and this borderless in-content one — so the marker can never appear in one
/// and not the other, and toggling `/mark-maze-layer` moves it in both at once.
pub fn layer_tab_title(graph: &mapper::graph::MapGraph, layer: mapper::layer::LayerId) -> String {
    let name = graph.layer_name(layer);
    let count = graph.rooms_in_layer(layer).len();
    if graph.layer_is_maze(layer) {
        format!("{name} ⌗({count})")
    } else {
        format!("{name}({count})")
    }
}

/// Draw a one-row layer tab strip at the top of `area` and return the remaining body area.
///
/// Draws nothing (returns `area` unchanged) when:
/// - fewer than 2 non-empty layers exist (single-layer maps are visually unchanged), or
/// - zoom is `Overview`.
///
/// Each non-empty layer is rendered as `name(count)` with a space separator, styled via
/// the `panel.tab` / `panel.tab:active` selectors (the same ones the bordered variant's
/// top-inset strip uses). All drawing is clipped to the strip row.
pub fn draw_layer_strip(
    graph: &mapper::graph::MapGraph,
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
) -> Rect {
    use crate::render::draw_str_clipped;

    // Skip in Overview zoom.
    if matches!(state.zoom, crate::state::Zoom::Overview) {
        return area;
    }
    if area.height == 0 {
        return area;
    }

    // Collect non-empty layers in sorted order.
    let mut layers: Vec<_> = graph.layers().keys().copied()
        .filter(|&l| !graph.rooms_in_layer(l).is_empty())
        .collect();
    layers.sort_unstable();

    // Only draw when there are 2+ non-empty layers.
    if layers.len() < 2 {
        return area;
    }

    let active = state.active_layer(graph);
    let strip_y = area.y;
    let strip_area = Rect { x: area.x, y: strip_y, width: area.width, height: 1 };

    // Clear the strip row first. Themed via panel.tab / panel.tab:active — the
    // SAME selectors the bordered variant's top-inset strip uses (main.rs), so
    // a borderless map pane's layer tabs match a bordered one's instead of
    // drawing bare/unthemeable `Style::new()` + a hardcoded REVERSED modifier.
    let normal_style = state.colors.theme.get("panel.tab").style;
    for x in area.x..area.right() {
        if let Some(cell) = buf.cell_mut((x, strip_y)) {
            cell.set_symbol(" ").set_style(normal_style);
        }
    }

    let active_style = state.colors.theme.get("panel.tab:active").style;
    let mut x = area.x;
    for layer_id in &layers {
        let label = format!(" {} ", layer_tab_title(graph, *layer_id));
        let style = if *layer_id == active { active_style } else { normal_style };
        // Clip label to available width.
        let remaining = area.right().saturating_sub(x);
        if remaining == 0 {
            break;
        }
        draw_str_clipped(buf, x, strip_y, &label, style, strip_area);
        x = x.saturating_add(label.chars().count() as u16);
    }

    // Return the area below the strip.
    if area.height <= 1 {
        Rect { x: area.x, y: area.y, width: area.width, height: 0 }
    } else {
        Rect { x: area.x, y: area.y + 1, width: area.width, height: area.height - 1 }
    }
}

/// Variant of [`render_map`] that also draws the layer tab strip when multiple layers exist.
///
/// Production callers (`main.rs`, `map_dump.rs`) should use this function.
/// Tests that call [`render_map`] directly are unaffected.
///
/// The in-content strip is suppressed when `state.colors.map_border_style != BorderStyle::None`,
/// Descriptive label for the room-detection method shown in the map corner.
pub(crate) fn loc_method_label(m: zvm::location::LocationMethod) -> &'static str {
    use zvm::location::LocationMethod::*;
    match m {
        GlobalVar0 => "via status variable",
        PlayerParent => "via player object",
        StatusName => "via name match",
        NameOnly => "via name (unlinked)",
        RoomHeading => "via room heading",
    }
}

/// because in that case the border carries layer tabs via `draw_top_inset` and drawing the
/// in-content strip would produce a double indicator and consume a content row.
pub fn render_map_layered(
    rm: &RenderMap,
    graph: &mapper::graph::MapGraph,
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
) -> Vec<(RoomId, Rect)> {
    use crate::render::paneframe::BorderStyle;
    // Hand the pane's size to input handlers that never see a pane rect (`Action::Recenter`).
    // Recorded here, from the rect actually drawn into, so it cannot drift from what the player
    // is looking at. `area`, not `body_area`: the run loop's own recentres measure the whole
    // content rect via `map_pane_dims(last_panes.map)`, and a key recentre must agree with them
    // rather than differ by the layer strip's row (SQ-0349).
    state.map_pane_size.set(Some((area.width, area.height)));
    let body_area = if state.colors.map_border_style == BorderStyle::None {
        draw_layer_strip(graph, state, area, buf)
    } else {
        area
    };
    // SQ-0666: a layer set to the matrix view draws a direction TABLE instead of a map. The fork
    // is here, at the single production entry point, so every caller of `render_map` (the dump
    // harness, the tidy animation, the map.rs tests) keeps drawing the drawn view unconditionally
    // — none of them is showing the player a layer they chose a view for.
    let layer = state.active_layer(graph);
    let hits = if graph.layer_view(layer) == mapper::layer::MapView::Matrix {
        crate::render::matrix::render_matrix(graph, layer, state, body_area, buf)
    } else {
        render_map(rm, state, body_area, buf);
        room_screen_rects(rm, state, body_area)
    };

    // Progress bar while the `animate-tidy` frames are built on a worker thread.
    // The bar vanishes when the build completes and `anim_build_job` becomes None.
    if let Some(job) = &state.anim_build_job {
        draw_tidy_progress(job, state, area, buf);
    }
    hits
}

/// Draw a centered, bordered progress box in the map pane while the tidy animation
/// builds off-thread. A single-line box (top/bottom border + one content row) holds
/// a "Tidying map… NN%" label plus filled/empty block glyphs, all styled by
/// `tidy_progress`. `job.total` is a room-count estimate, so the fraction is only
/// approximate; it is clamped below 1.0 so the bar never reads "done" before the
/// worker actually finishes.
fn draw_tidy_progress(
    job: &crate::state::AnimBuildJob,
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
) {
    use crate::render::draw_str_clipped;
    // Need at least a 3-row box (border + content + border) and some width.
    if area.width < 12 || area.height < 3 {
        return;
    }
    let done = job.progress.load(std::sync::atomic::Ordering::Relaxed);
    let frac = (done as f32 / job.total.max(1) as f32).min(0.99);
    let pct = (frac * 100.0) as u16;
    let label = format!("Tidying map… {pct}%");
    let label_w = label.chars().count() as u16;
    // Bar cells fit inside the box (label + a space + bar + 2 border columns), capped.
    let bar_cells = 24u16.min(area.width.saturating_sub(label_w + 3)) as usize;
    let filled = (frac * bar_cells as f32).round() as usize;
    let bar: String = (0..bar_cells)
        .map(|i| if i < filled { '█' } else { '░' })
        .collect();
    let text = if bar_cells > 0 { format!("{label} {bar}") } else { label };
    let inner_w = text.chars().count() as u16;

    // Box: content width + 2 border columns, 3 rows tall, centered in the pane.
    let box_w = (inner_w + 2).min(area.width);
    let box_h = 3u16;
    let bx = area.x + area.width.saturating_sub(box_w) / 2;
    let by = area.y + area.height.saturating_sub(box_h) / 2;
    let style = state.colors.theme.get("tidy_progress").style;
    let right = bx + box_w - 1;
    let bottom = by + box_h - 1;

    // Border + opaque fill (so the map doesn't show through the box).
    for yy in by..=bottom {
        for xx in bx..=right {
            let ch = match (xx == bx, xx == right, yy == by, yy == bottom) {
                (true, _, true, _) => '┌',
                (_, true, true, _) => '┐',
                (true, _, _, true) => '└',
                (_, true, _, true) => '┘',
                (_, _, true, _) | (_, _, _, true) => '─',
                (true, _, _, _) | (_, true, _, _) => '│',
                _ => ' ',
            };
            if let Some(cell) = buf.cell_mut((xx, yy)) {
                let mut b = [0u8; 4];
                cell.set_symbol(ch.encode_utf8(&mut b)).set_style(style);
            }
        }
    }
    // Content row.
    let content = Rect::new(bx + 1, by + 1, box_w.saturating_sub(2), 1);
    draw_str_clipped(buf, content.x, content.y, &text, style, content);
}

// ── Line-art connector rendering (Boxes zoom) ─────────────────────────────────

/// Direction bits a connector enters/leaves a cell on. Two perpendicular bits → a turn;
/// all four (from two crossing connectors) → `┼`.
const DIR_N: u8 = 1;
const DIR_E: u8 = 2;
const DIR_S: u8 = 4;
const DIR_W: u8 = 8;

/// The cell-edge midpoints a chain glyph reaches, as direction bits — `None` for a glyph
/// `diagonal_chain` never emits.
///
/// Every half-diagonal endpoint is an edge MIDPOINT, exactly where `─` and `│` attach, so a
/// chain glyph and an orthogonal run through the same cell describe strokes to the SAME points.
/// That is what lets the two merge into one mask rather than one overwriting the other
/// (SQ-0356). Matched against `path`, not against literals: every glyph here is themeable.
fn chain_glyph_bits(ch: char, path: &crate::symbols::PathGlyphs) -> Option<u8> {
    Some(match ch {
        c if c == path.diag_ul => DIR_N | DIR_W,
        c if c == path.diag_ur => DIR_N | DIR_E,
        c if c == path.diag_ll => DIR_S | DIR_W,
        c if c == path.diag_lr => DIR_S | DIR_E,
        c if c == path.ns => DIR_N | DIR_S,
        c if c == path.ew => DIR_E | DIR_W,
        _ => return None,
    })
}

/// Box-drawing glyph for a set of direction bits.
fn glyph_for(mask: u8, path: &crate::symbols::PathGlyphs) -> Option<char> {
    Some(match mask {
        m if m == DIR_E | DIR_W => path.ew,
        m if m == DIR_N | DIR_S => path.ns,
        m if m == DIR_S | DIR_E => path.se,
        m if m == DIR_S | DIR_W => path.sw,
        m if m == DIR_N | DIR_E => path.ne,
        m if m == DIR_N | DIR_W => path.nw,
        m if m == DIR_N | DIR_S | DIR_E => path.nse,
        m if m == DIR_N | DIR_S | DIR_W => path.nsw,
        m if m == DIR_E | DIR_W | DIR_S => path.ews,
        m if m == DIR_E | DIR_W | DIR_N => path.ewn,
        m if m == DIR_N | DIR_E | DIR_S | DIR_W => path.nesw,
        // A bare stub end (single direction) — render as the matching straight glyph so
        // the line visibly reaches the box edge rather than vanishing.
        m if m == DIR_E || m == DIR_W => path.ew,
        m if m == DIR_N || m == DIR_S => path.ns,
        _ => return None,
    })
}

/// Resolve the lane a connector point runs on within `channel`, by finding the `LaneSeg`
/// whose channel AND doubled-coord extent (`start..=end`) contains the point's position
/// along that channel's free axis. A single connector legitimately has TWO segments in the
/// same channel on different lanes (one per run), so a per-channel-index lookup is wrong —
/// it would collapse both runs onto one lane and draw them overlapping.
fn seg_lane(segs: &[mapper::route::LaneSeg], channel: mapper::route::Channel, along: i32) -> u16 {
    segs.iter()
        .find(|s| s.channel == channel && s.start <= along && along <= s.end)
        .map(|s| s.lane)
        .unwrap_or(0)
}

/// Map a doubled-coord polyline point to its virtual pixel, resolving each odd (channel)
/// coordinate's lane against THIS connector's lane segments by extent.
fn lane_pixel(
    pt: (i32, i32),
    cols: &PosTable,
    rows: &PosTable,
    segs: &[mapper::route::LaneSeg],
) -> (i32, i32) {
    use mapper::route::Channel;
    let (dx, dy) = pt;
    // x: even 2c → box-column centre; odd 2c+1 → channel V[c]. Lane 0 sits ONE cell into the
    // gutter (room_pixel + BOX_W + LANE_BASE), NOT on the box-edge doorway, so a channel run
    // never grazes the box edge where departure/arrival anchors live (otherwise an arriving
    // lane-0 line would run right alongside every same-side departure anchor). Each further
    // lane steps LANE_SPACING deeper. The departure/arrival anchors bridge to lane 0 across
    // the doorway cell, so lines still visibly touch the box.
    let px = if dx.rem_euclid(2) == 0 {
        let c = dx.div_euclid(2);
        cols.room_pixel(c) + BOX_W / 2
    } else {
        let c = (dx - 1).div_euclid(2);
        // A V(c) run varies along y; pick the segment whose y-extent contains dy.
        let lane = seg_lane(segs, Channel::V(c), dy) as i32;
        cols.room_pixel(c) + BOX_W + LANE_BASE + lane * LANE_SPACING
    };
    let py = if dy.rem_euclid(2) == 0 {
        let r = dy.div_euclid(2);
        rows.room_pixel(r) + BOX_H / 2
    } else {
        let r = (dy - 1).div_euclid(2);
        // An H(r) run varies along x; pick the segment whose x-extent contains dx.
        let lane = seg_lane(segs, Channel::H(r), dx) as i32;
        rows.room_pixel(r) + BOX_H + LANE_BASE + lane * LANE_SPACING
    };
    (px, py)
}

/// A departure/arrival glyph queued by `render_lane_connectors` for `draw_connector_arrows` to
/// paint on top of the rooms: `(virtual pixel, glyph string, distorted, is_portal, owning room,
/// shared)`. `is_portal` selects the `map.connector_portal` theme style for up/down glyphs instead
/// of `map.connector`/`map.connector_distorted`. `shared` selects `map.shared_path` for a
/// connector that collapsed secondary compass directions into itself.
/// How honest a drawn edge is about the trip back (SQ-0666).
///
/// The drawn view used to say the same thing about all three: one arrow leaving the origin, and
/// (for a collapsed reciprocal pair) one at the far end. Each now gets its own selector — both
/// defaulting to `map.connector`, so nothing changes appearance until someone chooses to style
/// it. Arrows themselves follow one rule: an arrow on a room border is that room's own EXIT
/// (SQ-0688), so a one-way edge draws no far-end glyph at all — the line ending bare on the
/// destination IS the "no known way back" reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    /// The compass inverse comes back: an ordinary two-way corridor.
    Reciprocal,
    /// A return exists, but by some other direction.
    Asymmetric,
    /// No way back is known.
    OneWay,
}

impl EdgeKind {
    fn selector(self) -> &'static str {
        match self {
            EdgeKind::Reciprocal => "map.connector",
            EdgeKind::Asymmetric => "map.edge:asym",
            EdgeKind::OneWay => "map.edge:oneway",
        }
    }
}

/// One arrowhead to stamp on a room border after the rooms are drawn.
#[derive(Debug, Clone)]
struct Arrowhead {
    at: (i32, i32),
    glyph: String,
    distorted: bool,
    is_portal: bool,
    room: RoomId,
    shared: bool,
    kind: EdgeKind,
}

/// Classify each drawn edge from the render model's own reverse-edge lookup.
///
/// Keyed by the full `(origin, dest, dir)` triple because a room pair can hold several passages
/// and they need not agree about the way back.
fn edge_kinds(rm: &RenderMap) -> std::collections::HashMap<(RoomId, RoomId, Direction), EdgeKind> {
    rm.edges
        .iter()
        .map(|e| {
            let kind = match e.arrival_dir {
                None => EdgeKind::OneWay,
                Some(d) if d == mapper::direction::opposite(e.dir) => EdgeKind::Reciprocal,
                Some(_) => EdgeKind::Asymmetric,
            };
            ((e.origin, e.dest, e.dir), kind)
        })
        .collect()
}

/// The cells (in virtual space) one connector writes, each with the direction-bit mask it
/// contributes there, plus its departure/arrival arrowhead anchors. This is the single
/// source of truth for connector plotting: the renderer ORs these per-cell masks into the
/// shared buffer, and tests re-derive per-connector ownership from the same geometry.
struct ConnectorPlot {
    cells: Vec<((i32, i32), u8)>,
    /// Explicit-glyph cells for a diagonal corner stub (SQ-0314), painted directly
    /// rather than through the 4-bit orthogonal mask — a diagonal has no
    /// representation in `glyph_for`'s N/E/S/W bits, and `dir_bit` would misfile a
    /// (+1,+1) step as East. Empty unless `diagonal_corners` is on.
    diag_cells: Vec<((i32, i32), char)>,
    dep_anchor: (i32, i32),
    arr_anchor: (i32, i32),
}

/// The direction bit that seams a `dir` chain to the orthogonal cell it hands off to (SQ-0314).
///
/// A chain leaves its last cell through that cell's upper- or lower-centre, and the handoff cell
/// sits immediately beyond it. For the orthogonal glyph there to actually MEET the chain, it needs
/// a stroke running from its centre back to the shared edge — i.e. pointing at the chain. A N-ward
/// chain (NE/NW) hands off upward, so the cell above it needs `DIR_S`; a S-ward chain needs
/// `DIR_N`. Without this bit the handoff cell draws a bare `─` through its middle and the diagonal
/// visibly stops one half-cell short.
fn chain_seam_bit(dir: Direction) -> u8 {
    match dir {
        Direction::NE | Direction::NW => DIR_S,
        _ => DIR_N,
    }
}

/// A half-diagonal chain: the `(cell, glyph)` pairs it plots, plus the point where an orthogonal
/// path resumes. See `diagonal_chain`.
type DiagonalChain = (Vec<((i32, i32), char)>, (i32, i32));

/// The chain of half-diagonals leaving a room's corner `anchor` toward `target` (SQ-0314), plus
/// the point where an orthogonal path resumes. `None` when `dir` is not diagonal, or when the gap
/// is too small to hold even one pair.
///
/// The chain starts in the corner's OWN row, one column along — for NE that is `(cx+1, cy)`. The
/// glyph there joins the corner's middle-right edge to the cell above's bottom edge, so the line
/// leaves the corner edge-to-edge with no seam, and the corner stays a usable connector slot
/// distinct from the side anchors.
///
/// Each half-diagonal joins two EDGE MIDPOINTS. Within a pair, the near cell enters on its
/// lower-centre (for a N-ward exit; upper-centre for a S-ward one) and hands off at its
/// middle-left/right; the far cell picks that up on its facing edge and exits at its upper/lower
/// centre — exactly where `│` attaches, so the resume point continues an orthogonal path with no
/// seam.
///
/// Pairs CHAIN by overlapping a column: step `k` sits in row `cy + k*sy`, leaving the line attached
/// at the top/bottom of its far cell — which is where step `k+1` picks it up. Each step crosses
/// exactly one row.
///
/// Step 0 is a HALF pair — the far glyph only, since its near cell would be the corner itself. That
/// is also what makes a single-row hop drawable. One row is not a rare case: a connector handing off
/// to lane 1 or beyond of a channel has only that much room, because higher lanes sit closer to the
/// next box.
///
/// A step can be RESHAPED with fill, because both fill glyphs attach exactly where the
/// half-diagonals hand off:
///   * `─` attaches middle-left/middle-right, so it goes BETWEEN a step's two halves: `🮣─🮠` chains
///     exactly as `🮣🮠` does, two columns wide instead of one — a shallower step.
///   * `│` attaches upper-/lower-centre, so it goes AFTER a step's far glyph: one column across two
///     rows — a steeper step.
///
/// So the chain absorbs a surplus on EITHER axis and still lands exactly on its target, instead of
/// leaving a stray run beside the corner (too wide) or refusing to draw at all (too tall). On a
/// ~1:2 cell a bare step is a 63° climb, one `─` of fill makes it a true 45°, and one `│` makes it
/// 76°.
fn diagonal_chain(
    anchor: (i32, i32),
    target: (i32, i32),
    dir: Direction,
    g: &crate::symbols::PathGlyphs,
) -> Option<DiagonalChain> {
    // `(sx, sy)`: the chain's per-step direction. `(near, far)`: the pair's two glyphs, in the
    // order the line travels through them.
    let (sx, sy, near, far) = match dir {
        Direction::NE => (1, -1, g.diag_lr, g.diag_ul),
        Direction::NW => (-1, -1, g.diag_ll, g.diag_ur),
        Direction::SE => (1, 1, g.diag_ur, g.diag_ll),
        Direction::SW => (-1, 1, g.diag_ul, g.diag_lr),
        _ => return None,
    };
    let (cx, cy) = anchor;
    let rows = (target.1 - cy).abs();
    let cols = (target.0 - cx).abs();
    if rows < 1 || cols < 1 {
        return None; // level with the corner on one axis: nothing diagonal to draw
    }
    // A bare step crosses one row and one column. Whichever axis has more to cover is the surplus,
    // and it is spent as FILL inside the steps so the chain lands exactly on `target`.
    let steps = rows.min(cols);
    let h_surplus = cols - steps; // spent as `─` BETWEEN a step's two halves → shallower
    let v_surplus = rows - steps; // spent as `│` AFTER a step's far glyph → steeper
    //
    // Step 0 never takes `─` fill. Its near cell IS the corner, so fill there would sit between the
    // corner and the first diagonal glyph — the line would leave the room HORIZONTALLY and only
    // then turn diagonal, which is the one thing the corner exit exists to avoid. Steps 1.. have a
    // real near glyph, so their fill lands mid-step and reads as part of the diagonal. `│` fill is
    // safe on any step, including 0: it goes AFTER the far glyph, never touching the corner.
    let hf_steps = steps - 1;
    let (hf, hf_wide) = if hf_steps > 0 { (h_surplus / hf_steps, h_surplus % hf_steps) } else { (0, 0) };
    let (vf, vf_tall) = (v_surplus / steps, v_surplus % steps);
    let mut cells = Vec::with_capacity((rows + cols) as usize);
    let (mut x, mut y) = (cx, cy); // where the line currently sits; starts on the corner itself
    for k in 0..steps {
        // Spread each surplus as evenly as its steps allow, widest first, so no single step is
        // conspicuously shallower or steeper than its neighbours.
        let h = if k == 0 { 0 } else { hf + i32::from(k - 1 < hf_wide) };
        let v = vf + i32::from(k < vf_tall);
        if k > 0 {
            cells.push(((x, y), near)); // step 0's near cell would BE the corner
        }
        for i in 1..=h {
            cells.push(((x + i * sx, y), g.ew));
        }
        x += (h + 1) * sx;
        cells.push(((x, y), far));
        for i in 1..=v {
            cells.push(((x, y + i * sy), g.ns));
        }
        y += (v + 1) * sy;
    }
    // `y` always lands on `target.1`. `x` does too, EXCEPT when there was only step 0 to hold `─`
    // fill — then the chain stays one column wide and stops short, and the caller bridges the rest
    // along the TARGET's row. That row is the channel lane the line was heading for anyway, so the
    // remainder reads as part of that run rather than as a stub hanging off the corner.
    Some((cells, (x, y)))
}

/// Compute the virtual cells + per-cell masks a single connector occupies.
///
/// `diag` carries the path glyphs when `diagonal_corners` is on (SQ-0314): a diagonal exit then
/// leaves its corner on a chain of half-diagonals before any orthogonal path resumes. `None`
/// selects the fallback for terminals without those glyphs — the SAME corner anchor, walked
/// orthogonally. The toggle picks glyphs, not geometry: where a connector departs and arrives is
/// the router's business, and both settings ask it the same questions.
fn plot_connector(
    conn: &mapper::route::RoutedConnector,
    cols: &PosTable,
    rows: &PosTable,
    diag: Option<&crate::symbols::PathGlyphs>,
) -> Option<ConnectorPlot> {
    // Convert the doubled polyline to a virtual-pixel polyline, resolving each point's lane
    // against this connector's segments by channel + extent (a connector may have two runs
    // in one channel on different lanes).
    let pix: Vec<(i32, i32)> = conn
        .points
        .iter()
        .map(|&p| lane_pixel(p, cols, rows, &conn.segs))
        .collect();
    // A merge stub may legitimately collapse to just centre→junction (2 points) — it still must
    // render its box-edge exit arrow and a short line to the junction. Every other connector needs
    // centre + interior + centre (3 points).
    if pix.len() < if conn.merge { 2 } else { 3 } {
        return None;
    }

    // The connector runs centre→…→centre. A line must not be drawn inside a room box, so
    // trim the two room centres. In their place, anchor each end on the box's edge cell for
    // that side (the doorway just outside the box), displaced along the edge by the slot, so
    // the line visibly touches both rooms even when the channel is wider than the lane it
    // runs in, and two connectors sharing a side land on distinct cells.
    let origin_cell = (conn.points[0].0.div_euclid(2), conn.points[0].1.div_euclid(2));
    let dep_anchor = if mapper::direction::is_diagonal(conn.exit_dir) {
        corner_anchor(cols, rows, origin_cell, conn.exit_dir)
    } else {
        box_edge_anchor(cols, rows, origin_cell, conn.exit, conn.exit_slot)
    };

    // The connector leaves the box straight out at 90° (a perpendicular stub on the anchor's own
    // row/col), then steps along the edge into the first interior channel point. Distinct slots
    // give distinct border cells; the straight connector on each side keeps slot 0 (centre), so a
    // displaced connector crosses it as a single clean ┼ instead of a corner stomp.
    let first_interior = pix[1];

    // The arrival anchor does not depend on the departure geometry, so resolve it first: a
    // corner-to-corner diagonal aims its chain straight at it (SQ-0314), and so needs it up front.
    //
    // The arrival sits on a box corner exactly when the ROUTER built its polyline to end there:
    // `conn.entry_corner` is the router's own resolved answer, read rather than re-derived, so the
    // two ends cannot drift apart. It covers the one-way diagonal (no back edge, but still arrives
    // on the corner facing its origin) and the arrival that YIELDED its corner to the destination's
    // own outgoing diagonal — that one is back on a side doorway, at a real slot.
    let arr_target = (!conn.merge).then(|| {
        let last = conn.points[conn.points.len() - 1];
        let dest_cell = (last.0.div_euclid(2), last.1.div_euclid(2));
        match conn.entry_corner {
            Some(d) => corner_anchor(cols, rows, dest_cell, d),
            None => box_edge_anchor(cols, rows, dest_cell, conn.entry, conn.entry_slot),
        }
    });

    // SQ-0314: a diagonal exit leaves the corner on a chain of half-diagonals, and the orthogonal
    // path resumes at the chain's far end (a │ attachment point).
    //
    // A PURE diagonal — centre → shared corner → centre, the diagonally-adjacent case the router
    // collapses — aims the chain at the ARRIVAL corner and has no orthogonal leg at all when the
    // two gaps are square. Every other diagonal chains a while and then bridges to its first
    // interior channel point as usual.
    //
    // With `diag` off, or for a non-diagonal exit, `bridge_from` stays the anchor and the geometry
    // below is the plain corner/edge-to-bridge route.
    let arrive_dir = conn.entry_corner;
    // "Pure" means the WHOLE connector is one diagonal, corner to corner — so BOTH ends must be
    // diagonal. Testing only the arrival was a bug: a cardinal-out/diagonal-in pair (E out, NW
    // back) between adjacent rooms also collapses to three points, and it would then take the pure
    // branch with an empty chain — suppressing the arrival diagonal and drawing nothing diagonal at
    // all. (SQ-0314)
    let pure_diagonal = arr_target.is_some()
        && pix.len() == 3
        && arrive_dir.is_some()
        && mapper::direction::is_diagonal(conn.exit_dir);
    let mut diag_cells: Vec<((i32, i32), char)> = Vec::new();
    // Mask bits that seam a chain to the orthogonal cell it hands off to. The chain attaches at a
    // cell EDGE midpoint, but an orthogonal run is drawn through the cell's CENTRE — so the
    // handoff cell must also carry a stroke reaching the edge the chain arrives at, or the two
    // leave a visible gap. `chain_seam_bit` names that edge.
    let mut seams: Vec<((i32, i32), u8)> = Vec::new();
    let mut bridge_from = dep_anchor;
    if let Some(g) = diag {
        if mapper::direction::is_diagonal(conn.exit_dir) {
            let target = if pure_diagonal { arr_target.unwrap() } else { first_interior };
            if let Some((chain, resume)) = diagonal_chain(dep_anchor, target, conn.exit_dir, g) {
                diag_cells = chain;
                bridge_from = resume;
                if !pure_diagonal {
                    seams.push((resume, chain_seam_bit(conn.exit_dir)));
                }
            }
        }
    }

    // The ARRIVAL end mirrors the departure: the router emits a diagonal step INTO the destination
    // corner too (SQ-0314), so a non-adjacent diagonal reads as diagonal-out, run, diagonal-in
    // rather than losing its diagonals to doglegs at both ends. Chain BACKWARDS from the corner
    // toward the last interior point — same helper, same geometry, just aimed the other way.
    let mut bridge_to = arr_target;
    if let (Some(g), Some(d), Some(aa)) = (diag, arrive_dir, arr_target) {
        if !pure_diagonal && !conn.merge {
            let last_interior = pix[pix.len() - 2];
            if let Some((chain, resume)) = diagonal_chain(aa, last_interior, d, g) {
                diag_cells.extend(chain);
                bridge_to = Some(resume);
                seams.push((resume, chain_seam_bit(d)));
            }
        }
    }

    let mut inner_v: Vec<(i32, i32)> = Vec::with_capacity(pix.len() + 6);
    inner_v.push(bridge_from);
    let arr_anchor = if conn.merge {
        // A merge stub ENDS ON the trunk at the junction (`pix.last()`), not at a destination box —
        // no arrival anchor or bridge; the line simply reaches the junction (a T-junction).
        inner_v.extend_from_slice(&attach_bridge(bridge_from, first_interior, conn.exit));
        inner_v.extend_from_slice(&pix[1..]);
        *pix.last().unwrap()
    } else if pure_diagonal && !diag_cells.is_empty() {
        // The chain IS the connector: corner to corner. `pix[1]` here is the corner lattice point
        // — the channel intersection, which does not lie on the diagonal — so it must be skipped,
        // not bridged to. `diagonal_chain` stops on whichever axis runs out first, so anything left
        // over from a non-square gap is a single straight run to the arrival corner.
        let aa = arr_target.unwrap();
        inner_v.push(aa);
        aa
    } else {
        inner_v.extend_from_slice(&attach_bridge(bridge_from, first_interior, conn.exit));
        // `bridge_to` is the arrival corner, or — when a chain claimed the last stretch — the point
        // where that chain picks the line up.
        let aa = bridge_to.unwrap();
        let last_interior = pix[pix.len() - 2];
        let arr_bridge = attach_bridge(aa, last_interior, conn.entry);
        inner_v.extend_from_slice(&pix[1..pix.len() - 1]);
        for &p in arr_bridge.iter().rev() {
            inner_v.push(p);
        }
        inner_v.push(aa);
        arr_target.unwrap() // the ARROWHEAD still belongs on the box corner, not the chain's end
    };
    inner_v.dedup();
    let inner = &inner_v[..];
    if inner.is_empty() {
        return None;
    }

    // Walk the inner polyline cell-by-cell.
    let mut run: Vec<(i32, i32)> = Vec::new();
    for w in inner.windows(2) {
        let (a, b) = (w[0], w[1]);
        debug_assert!(a.0 == b.0 || a.1 == b.1, "bridge must be orthogonal: {a:?}->{b:?}");
        let dxs = (b.0 - a.0).signum();
        let dys = (b.1 - a.1).signum();
        let mut cur = a;
        loop {
            if run.last() != Some(&cur) {
                run.push(cur);
            }
            if cur == b {
                break;
            }
            cur = (cur.0 + dxs, cur.1 + dys);
        }
    }
    if run.is_empty() {
        run.push(inner[0]);
    }
    // Remove out-and-back spurs: a slot-offset anchor whose stub centre sits one cell off the
    // run's natural direction can leave a 1-cell dead-end (…A,B,A…). Collapse them so the
    // line is a clean path with no dangling tail that would clip a neighbour.
    let mut changed = true;
    while changed && run.len() >= 3 {
        changed = false;
        let mut i = 1;
        while i + 1 < run.len() {
            if run[i - 1] == run[i + 1] {
                run.remove(i + 1);
                run.remove(i);
                changed = true;
            } else {
                i += 1;
            }
        }
    }

    let mut cells = Vec::with_capacity(run.len());
    for i in 0..run.len() {
        let c = run[i];
        let mut mask = 0u8;
        if i > 0 {
            mask |= dir_bit(c, run[i - 1]);
        }
        if i + 1 < run.len() {
            mask |= dir_bit(c, run[i + 1]);
        }
        // Seam a chain's handoff cell to the chain (SQ-0314); see `chain_seam_bit`.
        for &(at, bit) in &seams {
            if at == c {
                mask |= bit;
            }
        }
        cells.push((c, mask));
    }
    Some(ConnectorPlot { cells, diag_cells, dep_anchor, arr_anchor })
}

/// Draw every plan connector as box-drawing line-art along its lanes, and RETURN the departure
/// (and reciprocal arrival) arrowheads as `(virtual pixel, glyph, distorted, is_portal, room_id)`.
/// The arrowheads are NOT drawn here: each sits ON a room's border cell, so the caller draws them
/// AFTER the rooms (which render on top of the line-art) so the arrow replaces the box-edge glyph.
///
/// Up/Down connectors (`exit_dir == Up | Down`) are lane-routed like any compass connector but
/// render differently: their body uses the portal's DOTTED glyphs (not the shared solid set),
/// styled with the `map.connector_portal` theme style (not `map.connector`/`map.connector_distorted`
/// — up/down are never distorted), and their departure anchor carries the up/down glyph instead of
/// an arrowhead. They accumulate into a SEPARATE per-cell mask (`updown_cells`) from the compass
/// connectors' `cells` map, so compass crossings/turns are computed exactly as before — up/down
/// never contributes to or reads a compass cell's mask. A matching Up/Down pair now collapses to
/// one RECIPROCAL connector (SQ-0216): the far-end block below draws the up/down glyph (derived
/// from `entry_dir`) at the arrival end too, instead of an arrowhead, so both ends show their own
/// glyph — styled `map.connector_portal` just like the departure end.
fn render_lane_connectors(
    plan: &RoutePlan,
    cols: &PosTable,
    rows: &PosTable,
    offset: (i32, i32),
    area: Rect,
    buf: &mut Buffer,
    arrows: &crate::symbols::Arrows,
    path: &crate::symbols::PathGlyphs,
    portal: &crate::symbols::PortalGlyphs,
    colors: &crate::colors::ColorScheme,
    diagonal_corners: bool,
    kinds: &std::collections::HashMap<(RoomId, RoomId, Direction), EdgeKind>,
) -> Vec<Arrowhead> {
    let (off_x, off_y) = offset;
    // SQ-0314: when on, a diagonal exit leaves its corner on a chain of half-diagonals; `None`
    // walks the same corner orthogonally for terminals that lack the glyphs.
    let diag = diagonal_corners.then_some(path);

    // Bound once: this loop below reads these per connector, not per cell.
    let connector_distorted = colors.theme.get("map.connector_distorted").style;
    let connector = colors.theme.get("map.connector").style;
    let connector_portal = colors.theme.get("map.connector_portal").style;
    let shared_path = colors.theme.get("map.shared_path").style;

    // Per-cell accumulated direction mask. ORing masks means a perpendicular crossing of
    // two connectors (one ─, one │) combines to ┼; a connector revisiting its own cell is
    // idempotent and harmless. Compass connectors accumulate in `cells`; up/down connectors
    // accumulate separately in `updown_cells` so the two never mix (dotted vs solid glyphs).
    // Value is `(mask, owning connector index)`. The owner is what lets an unrelated CROSSING be
    // told from one connector's own turn: only a cell two DIFFERENT connectors want can be one
    // (SQ-0525).
    let mut cells: std::collections::HashMap<(i32, i32), (u8, usize)> =
        std::collections::HashMap::new();
    let mut updown_cells: std::collections::HashMap<(i32, i32), (u8, usize)> =
        std::collections::HashMap::new();
    // Dotted glyph set for up/down connector bodies: straight runs read as dotted; any turn
    // glyph falls back to the solid corner set (up/down routes like N/S so may still turn).
    let dotted_path = crate::symbols::PathGlyphs {
        ns: portal.path,
        ew: portal.path_h,
        ..*path
    };
    // Arrowheads: (virtual pixel, glyph string, distorted, is_portal, owning room id). Returned
    // for the caller to draw on top of the rooms (the arrow embeds in the room border). Up/down
    // glyphs are flagged `is_portal` so the caller styles them with `map.connector_portal`
    // instead of `map.connector`/`map.connector_distorted`.
    let mut arrowheads: Vec<Arrowhead> = Vec::new();

    // Plot every connector up front: the diagonal-chain merge below needs to know whether ANY
    // connector claims a cell with compass line-art, which a single pass painting as it goes
    // cannot answer for connectors it has not reached yet.
    let plots: Vec<(&mapper::route::RoutedConnector, ConnectorPlot)> = plan
        .connectors
        .iter()
        .filter_map(|c| plot_connector(c, cols, rows, diag).map(|p| (c, p)))
        .collect();
    // Cells carrying compass line-art. Up/down connectors are excluded: they accumulate in their
    // own mask with their own dotted glyphs, so a chain has nothing there to merge WITH.
    let compass_cells: std::collections::HashSet<(i32, i32)> = plots
        .iter()
        .filter(|(c, _)| !matches!(c.exit_dir, Direction::Up | Direction::Down))
        .flat_map(|(_, p)| p.cells.iter().map(|(c, _)| *c))
        .collect();

    let mut pending_markers: Vec<PendingMarker> = Vec::new();
    for (ci, (conn, plot)) in plots.iter().enumerate() {
        let is_updown = matches!(conn.exit_dir, Direction::Up | Direction::Down);
        let has_secondary = !conn.secondary_exit.is_empty() || !conn.secondary_entry.is_empty();
        // Up/down connectors always use the portal selector (they're never distorted);
        // a connector with collapsed secondaries uses the brighter shared_path color;
        // compass connectors otherwise keep their existing connector/connector_distorted styling.
        // How honest this edge is about the trip back (SQ-0666). Unknown to the plan — a
        // connector records where it is drawn, not what comes back along it — so it is looked up
        // from the render model's reverse-edge scan.
        let kind = kinds
            .get(&(conn.origin, conn.dest, conn.exit_dir))
            .copied()
            .unwrap_or(EdgeKind::Reciprocal);
        let style = if is_updown {
            connector_portal
        } else if has_secondary {
            shared_path
        } else if conn.distorted {
            connector_distorted
        } else if kind != EdgeKind::Reciprocal {
            // Both default to `map.connector`, so this is invisible until it is styled — which is
            // the point: it puts the hook where the fact is, without changing anyone's map.
            colors.theme.get(kind.selector()).style
        } else {
            connector
        };
        let (cell_map, glyphs) = if is_updown {
            (&mut updown_cells, &dotted_path)
        } else {
            (&mut cells, path)
        };

        for (c, mask) in &plot.cells {
            let (sx, sy) = (c.0 + off_x, c.1 + off_y);
            if !in_area(sx, sy, area) {
                continue;
            }
            let entry = cell_map.entry(*c).or_insert((0, ci));
            let (prev, owner) = *entry;
            // Two DIFFERENT connectors, one running N|S and the other E|W: that cell is a
            // crossing, not a junction, and ORing them into `┼` said the two passages meet there
            // (SQ-0525). Connectors are point-to-point and never branch, so a four-bit mask can
            // only ever arise this way — there is no real junction to preserve. The vertical run
            // passes through and the horizontal one breaks, leaving a one-cell gap: horizontal
            // gaps are a single cell wide and read better than a broken vertical.
            //
            // Deliberately ONLY the clean four-bit case. Three-bit cells (a turn stomping a
            // straight run) are what `overlap_stats` counts as ILLEGAL and `cleanup_overlaps`
            // exists to remove; breaking a line there would hide a real layout defect and cost
            // the turning connector its corner.
            let crossing = owner != ci
                && matches!((prev, *mask), (m, n) | (n, m) if m == DIR_N | DIR_S && n == DIR_E | DIR_W);
            if crossing {
                if *mask == DIR_N | DIR_S {
                    *entry = (*mask, ci); // the vertical arrives second and takes the cell
                } else {
                    continue; // the horizontal yields; its neighbours already drew the approach
                }
            } else {
                entry.0 |= *mask;
            }
            let glyph_s = glyph_for(entry.0, glyphs).unwrap_or('·').to_string();
            if let Some(cell) = buf.cell_mut((sx as u16, sy as u16)) {
                cell.set_symbol(&glyph_s).set_style(style);
            }
        }

        // Diagonal corner stub (SQ-0314): explicit glyphs, painted AFTER this connector's mask
        // cells. On a cell of its own these do NOT enter `cell_map` — a half-diagonal has no
        // 4-bit mask representation, and letting it OR into a neighbour's mask would corrupt
        // that neighbour's glyph choice. Always empty when `diagonal_corners` is off.
        //
        // On a cell some OTHER connector runs orthogonal line-art through, there is no glyph
        // for "half-diagonal crossing a line", so the chain MERGES instead: its endpoint bits
        // OR into the shared mask and the cell renders as the junction that joins them — the
        // diagonal flattens for that one cell rather than either line losing it (SQ-0356).
        // Merging via the mask (not by painting a glyph) is what makes this order-independent:
        // a connector painting the cell later ORs on top and the chain's bits survive.
        for (c, ch) in &plot.diag_cells {
            let (sx, sy) = (c.0 + off_x, c.1 + off_y);
            if !in_area(sx, sy, area) {
                continue;
            }
            let merge = (!is_updown && compass_cells.contains(c))
                .then(|| chain_glyph_bits(*ch, glyphs))
                .flatten();
            let glyph_s = match merge {
                Some(bits) => {
                    let entry = cell_map.entry(*c).or_insert((0, ci));
                    entry.0 |= bits;
                    glyph_for(entry.0, glyphs).unwrap_or(*ch).to_string()
                }
                None => ch.to_string(),
            };
            if let Some(cell) = buf.cell_mut((sx as u16, sy as u16)) {
                cell.set_symbol(&glyph_s).set_style(style);
            }
        }

        let dep_ch = if is_updown {
            match conn.exit_dir {
                Direction::Up => portal.up,
                Direction::Down => portal.down,
                _ => unreachable!("is_updown guards to Up | Down"),
            }
        } else if mapper::direction::is_diagonal(conn.exit_dir) {
            diagonal_arrow(conn.exit_dir, arrows)
        } else {
            arrow_for_departure(conn.exit, arrows)
        };
        arrowheads.push(Arrowhead {
            at: plot.dep_anchor,
            glyph: dep_ch.to_string(),
            distorted: conn.distorted,
            is_portal: is_updown,
            room: conn.origin,
            shared: has_secondary,
            kind,
        });
        // Far-end glyph for a true reciprocal connector (collapsed opposite pair). An up/down
        // reciprocal draws its own up/down glyph (from the back-edge's direction) at the far end
        // too, same as the departure end, rather than an arrow.
        if conn.reciprocal {
            let arr_ch = match conn.entry_dir {
                Some(Direction::Up) if is_updown => portal.up,
                Some(Direction::Down) if is_updown => portal.down,
                Some(d) if mapper::direction::is_diagonal(d) => diagonal_arrow(d, arrows),
                _ => arrow_for_departure(conn.entry, arrows),
            };
            arrowheads.push(Arrowhead {
                at: plot.arr_anchor,
                glyph: arr_ch.to_string(),
                distorted: conn.distorted,
                is_portal: is_updown,
                room: conn.dest,
                shared: has_secondary,
                kind,
            });
        }
        // A ONE-WAY passage gets no glyph at its far end (SQ-0688, reversing the arrival arrow
        // SQ-0666 added). Every arrow on a room border is that room's own EXIT — the map's one
        // arrow rule — so an inbound arrow on the destination reads as an exit that does not
        // exist (a NE arrival stamped its side-arrow `▶` on Deep Canyon's corner: "east leads
        // out of here"). The line simply ends on the box: a connector with a departure arrow at
        // one end and nothing at the other IS the one-way reading.

        // SQ-0689: stamp the marker each collapsed secondary was always promised (see
        // `RoutedConnector::secondary_exit`). The plan folds an extra same-pair passage into the
        // winning connector instead of drawing a second line — correct, but the passage then
        // vanished entirely: the shared line got a brighter colour and nothing else, and the
        // portal-icon pass deliberately leaves up/down to the connector it assumes exists. A
        // staircase that lost the pairing (Zork's Chasm: N wins the line, Up collapses) was
        // invisible. Each secondary direction now queues its glyph beside the shared line's
        // anchor, on the border of the room the collapsed edge DEPARTS from.
        for (dirs, anchor, side, room) in [
            (&conn.secondary_exit, plot.dep_anchor, conn.exit, conn.origin),
            (&conn.secondary_entry, plot.arr_anchor, conn.entry, conn.dest),
        ] {
            for &dir in dirs.iter() {
                pending_markers.push(PendingMarker {
                    anchor,
                    side,
                    glyph: arrow_for_direction(dir, arrows, portal),
                    is_portal: matches!(dir, Direction::Up | Direction::Down),
                    distorted: conn.distorted,
                    room,
                    kind,
                });
            }
        }
    }

    // Place the queued secondary markers where they collide with nothing already stamped: the
    // anchor cell itself first — the marker sits ON the shared line, aligned with it — then the
    // nearest free border cell stepping ±1, ±2 along the box edge. The anchor is free whenever
    // this end carries no arrow of its own (every entry end except a reciprocal's, under the one
    // arrow rule); a departure arrow or an earlier marker there pushes this one a step along.
    // Placement runs after EVERY connector has queued its arrowheads, so a marker can never
    // overwrite another connector's departure arrow that lands beside the same anchor.
    let mut occupied: std::collections::HashSet<(i32, i32)> =
        arrowheads.iter().map(|a| a.at).collect();
    for m in pending_markers {
        let along: fn((i32, i32), i32) -> (i32, i32) = match m.side {
            Side::Top | Side::Bottom => |a, k| (a.0 + k, a.1),
            Side::Left | Side::Right => |a, k| (a.0, a.1 + k),
        };
        let Some(at) = [0, 1, -1, 2, -2]
            .into_iter()
            .map(|k| along(m.anchor, k))
            .find(|c| !occupied.contains(c))
        else {
            continue; // border full — the shared-path colour still marks the collapse
        };
        occupied.insert(at);
        arrowheads.push(Arrowhead {
            at,
            glyph: m.glyph.to_string(),
            distorted: m.distorted,
            is_portal: m.is_portal,
            room: m.room,
            shared: true,
            kind: m.kind,
        });
    }
    arrowheads
}

/// A secondary-passage marker queued by the connector loop for the collision-avoiding
/// placement pass (SQ-0689). Markers are placed in queue order, so several at one anchor
/// take successive free cells deterministically.
struct PendingMarker {
    anchor: (i32, i32),
    side: Side,
    glyph: char,
    is_portal: bool,
    distorted: bool,
    room: RoomId,
    kind: EdgeKind,
}

/// Draw the embedded-in-border arrowheads (from [`render_lane_connectors`]) on top of the rooms.
///
/// Each arrowhead carries the `RoomId` of the room it belongs to.  The arrow sits on the
/// room's border, so its background is painted to match that room box's border background —
/// for normal, current, selected, and current+selected rooms alike.  This mirrors
/// `room_style`'s precedence.  The current room reverses only its interior, so its border
/// (where the arrow sits) is not reverse-video and its background is the style's plain `bg`.
/// The arrow glyph foreground is always the connector/path color — `map.connector_portal`
/// for up/down glyphs (`is_portal`), otherwise `map.connector`/`map.connector_distorted`.
fn draw_connector_arrows(
    arrowheads: &[Arrowhead],
    offset: (i32, i32),
    area: Rect,
    buf: &mut Buffer,
    colors: &crate::colors::ColorScheme,
    selected_room: Option<RoomId>,
    current_room: Option<RoomId>,
) {
    let (off_x, off_y) = offset;
    // Bound once: the loop below reads these per arrowhead, not per cell.
    let connector_distorted = colors.theme.get("map.connector_distorted").style;
    let connector = colors.theme.get("map.connector").style;
    let connector_portal = colors.theme.get("map.connector_portal").style;
    let shared_path = colors.theme.get("map.shared_path").style;
    let room_selected = colors.theme.get("map.room_selected").style;
    let room_current = colors.theme.get("map.room_current").style;
    let room_normal = colors.theme.get("map.room").style;
    let edge_oneway = colors.theme.get("map.edge:oneway").style;
    let edge_asym = colors.theme.get("map.edge:asym").style;
    for Arrowhead { at, glyph, distorted, is_portal, room: room_id, shared, kind } in arrowheads {
        let (vx, vy) = *at;
        let (sx, sy) = (vx + off_x, vy + off_y);
        if in_area(sx, sy, area) {
            let connector_style = if *is_portal {
                connector_portal
            } else if *shared {
                shared_path
            } else if *distorted {
                connector_distorted
            } else {
                match kind {
                    EdgeKind::OneWay => edge_oneway,
                    EdgeKind::Asymmetric => edge_asym,
                    EdgeKind::Reciprocal => connector,
                }
            };
            let connector_fg = connector_style.fg;
            // Pick the room box's base style with the same precedence as room_style, then
            // derive its VISIBLE background (REVERSED swaps fg/bg at render time).
            let is_sel = selected_room == Some(*room_id);
            let is_cur = current_room == Some(*room_id);
            let base = if is_cur && is_sel {
                room_selected
            } else if is_cur {
                room_current
            } else if is_sel {
                room_selected
            } else {
                room_normal
            };
            // The arrow sits on the box border, which is never reverse-video, so the
            // visible background is the style's plain `bg`.
            let visible_bg = base.bg;
            // Start from reset so no prior highlight bleeds through, then set the matching bg
            // and the connector fg.
            let mut style = Style::reset();
            if let Some(bg) = visible_bg {
                style = style.bg(bg);
            }
            if let Some(fg) = connector_fg {
                style = style.fg(fg);
            }
            if let Some(cell) = buf.cell_mut((sx as u16, sy as u16)) {
                cell.set_symbol(glyph).set_style(style);
            }
        }
    }
}

/// Arrow glyph for a compass Direction (used by secondary markers). Up/Down never
/// appear here (they are not collapsed into compass secondaries).
///
/// The one resolver for "what glyph names this direction", shared by the map badge
/// and by `map_dump`'s PORTALS legend. The dump used to carry its own hard-coded
/// `PORTAL_IN`/`PORTAL_OUT` constants beside this, so `/export-map` printed ⊙/⊗
/// whatever the player's `map.portal_icons` preset said (SQ-0989).
pub(crate) fn arrow_for_direction(
    dir: Direction,
    arrows: &crate::symbols::Arrows,
    portal: &crate::symbols::PortalGlyphs,
) -> char {
    match dir {
        Direction::N => arrows.north,
        Direction::S => arrows.south,
        Direction::E => arrows.east,
        Direction::W => arrows.west,
        Direction::NE => arrows.ne,
        Direction::NW => arrows.nw,
        Direction::SE => arrows.se,
        Direction::SW => arrows.sw,
        // A collapsed staircase keeps its portal icon rather than borrowing a compass arrow:
        // the portal arms serve `dir_glyph` below; secondaries themselves are compass-only.
        Direction::Up => portal.up,
        Direction::Down => portal.down,
        Direction::In => portal.in_,
        Direction::Out => portal.out,
        Direction::Unknown => portal.unknown,
    }
}

/// Map a per-(room, side) slot index to a signed offset ALONG the box edge so multiple
/// connectors on one side anchor on distinct cells. Slot 0 stays on the side centre;
/// further slots fan out symmetrically (+1, -1, +2, -2, …), clamped to `max` so anchors
/// never leave the box edge.
fn slot_offset(slot: u16, max: i32) -> i32 {
    let step = ((slot as i32) + 1) / 2;
    let signed = if slot % 2 == 1 { step } else { -step };
    signed.clamp(-max, max)
}

/// The virtual-pixel cell ON the box border at logical `cell` on `side`, displaced ALONG the
/// box edge by this connector's per-(room, side) `slot`. This is the cell where the outgoing
/// arrowhead is drawn — it REPLACES the box-border glyph (a `│` on a vertical side, a `─` on
/// a horizontal side), so the arrow reads as embedded in the room outline. The connector line
/// then continues perpendicular OUT from this cell (see [`attach_bridge`]).
///
/// Slots map to distinct INTERIOR rows/cols along the side (never the corners), so two
/// connectors sharing a side land on distinct border cells.
fn box_edge_anchor(cols: &PosTable, rows: &PosTable, cell: (i32, i32), side: Side, slot: u16) -> (i32, i32) {
    let bx = cols.room_pixel(cell.0);
    let by = rows.room_pixel(cell.1);
    let cx = bx + BOX_W / 2;
    let cy = by + BOX_H / 2;
    // Along a vertical side (Left/Right) the edge runs in y; offset rows, clamped so the
    // anchor stays on the box's interior rows (off the corners). Along a horizontal side
    // (Top/Bottom) offset cols likewise.
    let v_max = BOX_H / 2 - 1; // keep off the corners
    let h_max = BOX_W / 2 - 1;
    match side {
        Side::Right => (bx + BOX_W - 1, cy + slot_offset(slot, v_max)),
        Side::Left => (bx, cy + slot_offset(slot, v_max)),
        Side::Bottom => (cx + slot_offset(slot, h_max), by + BOX_H - 1),
        Side::Top => (cx + slot_offset(slot, h_max), by),
    }
}

/// Build the orthogonal bridge from a border `anchor` out to its first/last `interior` channel
/// point, returning the single intermediate turn point (anchor and interior are NOT included),
/// or empty when they already line up.
///
/// The connector leaves the box PERPENDICULAR to `side` (a straight stub at 90°), running in the
/// ANCHOR's own column/row all the way out to the interior's perpendicular level, then steps
/// ALONG the edge into the interior. Keeping the perpendicular leg on the anchor's own
/// column/row (not the interior's) means the only along-edge move happens AT the interior — so
/// where a slot-displaced connector must cross a straight connector sitting on the side centre,
/// it crosses that centre line as a single straight pass, yielding a clean ┼ rather than a
/// corner-on-corner stomp.
fn attach_bridge(anchor: (i32, i32), interior: (i32, i32), side: Side) -> Vec<(i32, i32)> {
    let turn = match side {
        // Perpendicular axis = x: run in x at the anchor's row out to interior.x, then step in y.
        Side::Right | Side::Left => (interior.0, anchor.1),
        // Perpendicular axis = y: run in y at the anchor's column out to interior.y, then step x.
        Side::Top | Side::Bottom => (anchor.0, interior.1),
    };
    if turn == anchor || turn == interior {
        Vec::new()
    } else {
        vec![turn]
    }
}

/// Direction bit pointing from cell `from` toward orthogonally-adjacent cell `to`.
fn dir_bit(from: (i32, i32), to: (i32, i32)) -> u8 {
    if to.1 < from.1 {
        DIR_N
    } else if to.1 > from.1 {
        DIR_S
    } else if to.0 > from.0 {
        DIR_E
    } else {
        DIR_W
    }
}

// ── Portal badges ─────────────────────────────────────────────────────────────

/// Draw a stub connector label in the top-right gutter cell outside the origin box.
/// `off_x`/`off_y` translate the origin's virtual rect into screen space.
fn draw_stub(
    edge: &RoutedEdge,
    placed: &std::collections::HashMap<RoomId, VRect>,
    off_x: i32,
    off_y: i32,
    area: Rect,
    buf: &mut Buffer,
    connector_style: Style,
) {
    let Some(&origin_rect) = placed.get(&edge.origin) else {
        return;
    };
    let label = edge.label.as_deref().unwrap_or("?");
    // Top-right gutter: just right of the box, at the top row.
    let lx = origin_rect.right() + off_x;
    let ly = origin_rect.y + off_y;
    put_str(buf, lx, ly, label, connector_style, area);
}

/// Which way a portal badge should be pulled inside its room box (SQ-0351, SQ-0223).
///
/// The DIRECTION is the source of truth wherever it has one — including Up/Down, which pull to the
/// top/bottom of the box. Reading it straight from the compass beats deriving it from the rooms'
/// cells: a distorted edge can leave its destination somewhere the direction plainly contradicts,
/// and the badge should agree with the word the player typed, not with where the layout engine
/// happened to put the other room.
///
/// `In`/`Out` are the exception: they carry no bearing at all, so the only thing that can aim them
/// is the partner's cell — which is exactly SQ-0351's ask ("towards the room they connect with").
/// `partner` is `None` when the destination is on another layer (a cross-layer `In`/`Out` has
/// nothing to aim at on this plane) or has no position yet; the badge then stays centred.
///
/// Returned as a unit-ish `(dx, dy)` in room-cell space, y down.
fn badge_bearing(dir: Direction, origin: (i32, i32), partner: Option<(i32, i32)>) -> Option<(i32, i32)> {
    match dir {
        Direction::Up => Some((0, -1)),
        Direction::Down => Some((0, 1)),
        Direction::In | Direction::Out | Direction::Unknown => {
            let p = partner?;
            let (dx, dy) = (p.0 - origin.0, p.1 - origin.1);
            (dx != 0 || dy != 0).then_some((dx.signum(), dy.signum()))
        }
        // Every compass direction already knows its own bearing.
        _ => mapper::direction::grid_offset(dir),
    }
}

/// The blank interior cell of the box at `(bx, by)` furthest along `bearing` (SQ-0351).
///
/// "Blank" is read back from the BUFFER, after `draw_room` has written the name and id — so this is
/// literally the closest empty spot, whatever the room happens to be called. Deriving it instead
/// from the centring maths would mean a second copy of `draw_room`'s layout, free to drift from it.
/// It also composes: each badge drawn fills a cell, so the next badge's scan sees it taken.
///
/// Cells are ranked by projection onto `bearing` (furthest that way wins), then by how close they
/// stay to the box's centre line across it, then by position — so the choice is deterministic and
/// hugs the middle rather than sliding into a corner. `None` when the interior is completely full;
/// the caller then keeps its fixed placement rather than overwriting a character.
fn nearest_free_interior(
    buf: &Buffer,
    (bx, by): (i32, i32),
    bearing: Option<(i32, i32)>,
    (off_x, off_y): (i32, i32),
    area: Rect,
) -> Option<(i32, i32)> {
    // A ranked interior cell: `(ranking key, cell)`.
    type RankedCell = ((i32, i32, i32), (i32, i32));
    let (dx, dy) = bearing.unwrap_or((0, 0));
    // Interior of an 11x5 box: columns bx+1..=bx+9, rows by+1..=by+3.
    let (cx, cy) = (bx + BOX_W / 2, by + BOX_H / 2);
    let mut best: Option<RankedCell> = None;
    for row in (by + 1)..=(by + BOX_H - 2) {
        for col in (bx + 1)..=(bx + BOX_W - 2) {
            let (sx, sy) = (col + off_x, row + off_y);
            if !in_area(sx, sy, area) {
                continue;
            }
            let blank = buf
                .cell((sx as u16, sy as u16))
                .is_some_and(|c| c.symbol() == " " || c.symbol().is_empty());
            if !blank {
                continue;
            }
            let along = (col - cx) * dx + (row - cy) * dy; // furthest toward the bearing
            let across = ((col - cx) * dy).abs() + ((row - cy) * dx).abs(); // hug the centre line
            let key = (-along, across, (row - by) * BOX_W + (col - bx));
            if best.as_ref().is_none_or(|(k, _)| key < *k) {
                best = Some((key, (col, row)));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// In-room icon slot for a portal direction: 0 = row 1 (Up), 1 = row 2 (mid: In/Out/Unknown),
/// 2 = row 3 (Down). Cardinal directions have no portal slot.
fn portal_slot(dir: Direction) -> Option<usize> {
    match dir {
        Direction::Up => Some(0),
        Direction::Down => Some(2),
        Direction::In | Direction::Out | Direction::Unknown => Some(1),
        _ => None,
    }
}

/// Where a portal-view badge sits for a passage leaving on `bearing`: the border cell it leads
/// through, and where its floating name goes (SQ-0363).
///
/// Returns `(glyph_cell, label_cell, right_align)`. `right_align` means the name ends AT
/// `label_cell` rather than starting there — a westward passage's name has to run back toward the
/// box, not away from it.
///
/// One rule for all eight directions. It reproduces the three fixed slots exactly — Up lands on
/// `(bx + BOX_W/2, by)`, Down on `(bx + BOX_W/2, by + BOX_H - 1)`, an eastward In/Out on
/// `(bx + BOX_W - 1, by + BOX_H/2)` — which is what says it is the same rule they always were,
/// just written for every direction instead of the four that could cross a layer before SQ-0360.
fn portal_border_placement(
    (bx, by): (i32, i32),
    bearing: (i32, i32),
) -> ((i32, i32), (i32, i32), bool) {
    let (dx, dy) = bearing;
    let col = match dx.signum() {
        -1 => bx,
        1 => bx + BOX_W - 1,
        _ => bx + BOX_W / 2,
    };
    let row = match dy.signum() {
        -1 => by,
        1 => by + BOX_H - 1,
        _ => by + BOX_H / 2,
    };
    // A name floats clear of the box on whichever side the passage leaves by. With any vertical
    // component it goes above or below, aligned to the box's left edge (as Up/Down always have);
    // a purely horizontal one goes out to the side, on the glyph's own row.
    let label = if dy != 0 {
        (bx, if dy < 0 { by - 1 } else { by + BOX_H })
    } else if dx > 0 {
        (bx + BOX_W, row)
    } else {
        (bx - 1, row)
    };
    ((col, row), label, dy == 0 && dx < 0)
}

/// Mid-slot precedence when a room has several of In/Out/Unknown (lower wins): In ▸ Out ▸ Unknown.
fn mid_precedence(dir: Direction) -> u8 {
    match dir {
        Direction::In => 0,
        Direction::Out => 1,
        _ => 2, // Unknown
    }
}

/// One room's portal icon choices: three slots (Up / Mid / Down), each holding an optional
/// `(glyph_char, dest_label)` pair chosen with `mid_precedence` for the shared mid slot.
type PortalSlots<'a> = [Option<(char, Option<&'a str>)>; 3];

/// Draw in-room portal indicators at Boxes zoom as a post-room overlay (so icons sit on top of
/// the box interior). Each room's portal (stub) edges map to a right-interior-column slot:
/// Up→row 1, In/Out/Unknown→row 2 (middle, by `mid_precedence`), Down→row 3. Default = the
/// direction glyph in that slot's far-right interior cell. When `show_labels` is set, the
/// portal's destination name is drawn right-aligned on that row with the icon pinned far-right.
/// In the default view an up-portal claims the upper-right corner, shifting the `●` notes marker
/// one cell left so both stay visible.
fn draw_portal_icons(
    rm: &RenderMap,
    placed: &std::collections::HashMap<RoomId, VRect>,
    state: &AppState,
    show_labels: bool,
    offset: (i32, i32),
    area: Rect,
    buf: &mut Buffer,
) {
    use std::collections::HashMap;
    let (off_x, off_y) = offset;
    let sym_portal = &state.symbols.portal;

    let sym_arrows = &state.symbols.arrows;

    // Helper: the glyph for the move that crosses to the other layer.
    //
    // A COMPASS edge can cross layers since `move-region <dest> <direction>` cuts a seam at one
    // (SQ-0360). Only portals could before, so every compass direction fell through to the
    // `unknown` marker — a badge that said "?" about a passage whose direction we know
    // perfectly well. Show the arrow you travel (SQ-0362).
    let dir_glyph = |dir: Direction| -> char { arrow_for_direction(dir, sym_arrows, sym_portal) };

    // Room cells, so a badge can be aimed at the partner it connects to (SQ-0351).
    let cell_of: HashMap<RoomId, (i32, i32)> = rm.rooms.iter().map(|r| (r.id, r.cell)).collect();

    // Per room, the chosen (glyph_char, dest_label) for each of the 3 slots; mid slot by precedence.
    let mut chosen: HashMap<RoomId, PortalSlots<'_>> = HashMap::new();
    let mut mid_rank: HashMap<RoomId, u8> = HashMap::new();
    // The mid slot's own edge, kept so its badge can be aimed (the glyph alone can't say where).
    let mut mid_edge: HashMap<RoomId, (Direction, RoomId)> = HashMap::new();
    // Cross-layer portals: the direction of travel to the other layer, per room (SQ-0223).
    let mut layer_badges: HashMap<RoomId, Vec<Direction>> = HashMap::new();
    // Portal view only: cross-layer COMPASS passages, which have no portal slot (SQ-0363).
    let mut layer_borders: HashMap<RoomId, Vec<(Direction, Option<&str>)>> = HashMap::new();
    for edge in &rm.edges {
        if !edge.is_stub {
            continue;
        }
        if edge.dir == Direction::Unknown {
            continue; // Unknown edges are non-spatial (e.g. death/respawn) — show no portal icon
        }
        // A cross-layer portal gets its own badge, placed by the same rule but marking a way OFF
        // this layer (SQ-0223). It must not also feed the slots: its destination is not on this
        // plane, so the slot machinery — which assumes a same-layer partner — cannot aim it.
        //
        // Portal view is exempt. There the icons live on the BORDER with the destination name
        // floating outside, and a cross-layer badge already names its target layer ("Cellar ·
        // Cellar") — a strictly better answer than a bare glyph. Diverting it there would delete
        // that label, so in that view the edge keeps its old path through the slots.
        if edge.is_interlayer && !show_labels {
            layer_badges.entry(edge.origin).or_default().push(edge.dir);
            continue;
        }
        // A cross-layer COMPASS passage has no portal slot — the slots only ever had to hold the
        // four directions that could leave a layer before a named seam could cut at
        // compass ones (SQ-0360). Falling through to `portal_slot` therefore dropped it silently,
        // icon and label both. Place it by bearing instead (SQ-0363).
        if edge.is_interlayer && portal_slot(edge.dir).is_none() {
            layer_borders
                .entry(edge.origin)
                .or_default()
                .push((edge.dir, edge.dest_label.as_deref()));
            continue;
        }
        let Some(slot) = portal_slot(edge.dir) else { continue };
        let glyph_ch = dir_glyph(edge.dir);
        let label = edge.dest_label.as_deref();
        let slots = chosen.entry(edge.origin).or_insert([None, None, None]);
        if slot == 1 {
            let rank = mid_precedence(edge.dir);
            let cur = mid_rank.entry(edge.origin).or_insert(u8::MAX);
            if rank < *cur {
                *cur = rank;
                slots[1] = Some((glyph_ch, label));
                mid_edge.insert(edge.origin, (edge.dir, edge.dest));
            }
        } else if slots[slot].is_none() {
            slots[slot] = Some((glyph_ch, label));
        }
    }

    let icon_col = BOX_W - 2; // far-right interior column — the fallback when the interior is full
    for room in &rm.rooms {
        let Some(&rect) = placed.get(&room.id) else { continue };
        let empty: PortalSlots<'_> = [None, None, None];
        let slots = chosen.get(&room.id).unwrap_or(&empty);
        let layers: &[Direction] = layer_badges.get(&room.id).map_or(&[], |v| v.as_slice());
        let borders = layer_borders.get(&room.id).map_or(&[][..], |v| v.as_slice());
        if slots.iter().all(Option::is_none) && layers.is_empty() && borders.is_empty() {
            continue;
        }
        let style = room_style(room, state);
        let (bx, by) = (rect.x, rect.y);

        // Place a badge on the free interior cell nearest the way it leads (SQ-0351/SQ-0223), and
        // draw it immediately: the next badge's scan reads the buffer, so it sees this cell taken.
        let place = |dir: Direction, dest: Option<RoomId>, glyph: char, buf: &mut Buffer| {
            let partner = dest.and_then(|d| cell_of.get(&d).copied());
            let bearing = badge_bearing(dir, room.cell, partner);
            let at = nearest_free_interior(buf, (bx, by), bearing, (off_x, off_y), area);
            let (col, row) = at.unwrap_or((bx + icon_col, by + 2)); // interior full: keep the old spot
            put_str(buf, col + off_x, row + off_y, &glyph.to_string(), style, area);
        };

        // A cross-layer portal is drawn in every view: it marks a way OFF this layer, which the
        // border icons below never expressed. Its glyph is the direction of travel (SQ-0223), so
        // the badge reads as the move the player makes — ↑/↓ stairs, ◉/◎ a doorway.
        for &dir in layers {
            place(dir, None, dir_glyph(dir), buf);
        }

        if show_labels {
            // A cross-layer compass passage sits on the border it points through, with its
            // "Room · Layer" name floating outside on that side — the same shape the slotted
            // portals have always had, for the directions they never covered (SQ-0363).
            for &(dir, label) in borders {
                let Some(bearing) = badge_bearing(dir, room.cell, None) else { continue };
                let ((gc, gr), (lc, lr), right_align) = portal_border_placement((bx, by), bearing);
                put_str(buf, gc + off_x, gr + off_y, &dir_glyph(dir).to_string(), style, area);
                if let Some(name) = label {
                    let col = if right_align { lc - name.chars().count() as i32 + 1 } else { lc };
                    put_str(buf, col + off_x, lr + off_y, name, style, area);
                }
            }
            // Portal view: icons move onto the border; destination names float OUTSIDE the box.
            if let Some((glyph_ch, label)) = slots[0] {
                let gs = glyph_ch.to_string();
                put_str(buf, bx + BOX_W / 2 + off_x, by + off_y, &gs, style, area); // top border
                if let Some(name) = label {
                    put_str(buf, bx + off_x, by - 1 + off_y, name, style, area); // above
                }
            }
            if let Some((glyph_ch, label)) = slots[2] {
                let gs = glyph_ch.to_string();
                put_str(buf, bx + BOX_W / 2 + off_x, by + BOX_H - 1 + off_y, &gs, style, area); // bottom border
                if let Some(name) = label {
                    put_str(buf, bx + off_x, by + BOX_H + off_y, name, style, area); // below
                }
            }
            if let Some((glyph_ch, label)) = slots[1] {
                let gs = glyph_ch.to_string();
                put_str(buf, bx + BOX_W - 1 + off_x, by + 2 + off_y, &gs, style, area); // right border
                // Unknown has no target semantics → glyph only, no floating name.
                if glyph_ch != sym_portal.unknown {
                    if let Some(name) = label {
                        put_str(buf, bx + BOX_W + off_x, by + 2 + off_y, name, style, area); // right
                    }
                }
            }
        } else {
            // Both in-room views now share one rule: the mid-slot icon (In/Out/Unknown) lands on
            // the free interior cell nearest the room it leads to (SQ-0351) rather than on a fixed
            // column. The old placements — far-right column with numbers on, centred with them off
            // — ignored the partner entirely and silently overwrote the last letter of a long name
            // (Zork's `◀  House ◉▶`: "House" centres as "  House  " and the badge took column 9).
            //
            // Up/Down (slots 0/2) still show their glyph on the connector's border anchor instead
            // (see `render_lane_connectors`), so only the mid slot draws here.
            if let Some((glyph_ch, _label)) = slots[1] {
                let dest = mid_edge.get(&room.id).map(|&(_, d)| d);
                let dir = mid_edge.get(&room.id).map_or(Direction::Unknown, |&(d, _)| d);
                place(dir, dest, glyph_ch, buf);
            }
        }
    }
}

// ── Room drawing ──────────────────────────────────────────────────────────────

/// Pick the outline `BoxStyle` for a room given its flags.
///
/// Precedence: current > portal > selected > normal.
fn outline_for(
    sym: &SymbolSet,
    is_current: bool,
    has_portal: bool,
    selected: bool,
) -> &BoxStyle {
    if is_current { &sym.room_current }
    else if has_portal { &sym.room_portal }
    else if selected { &sym.room_selected }
    else { &sym.room_normal }
}

/// Draw a room at screen top-left `(sx, sy)` (already translated from virtual space;
/// may be partially or fully off-area — drawing is clipped per cell).
fn draw_room(
    room: &RenderRoom,
    state: &AppState,
    zoom: Zoom,
    sx: i32,
    sy: i32,
    area: Rect,
    buf: &mut Buffer,
) {
    let base_style = room_style(room, state);
    let selected = state.selected_room == Some(room.id);

    match zoom {
        Zoom::Overview => {
            put_char(buf, sx, sy, '■', base_style, area);
        }
        Zoom::Compact => {
            draw_compact_room(room, sx, sy, base_style, &state.symbols, selected, area, buf);
        }
        Zoom::Boxes => {
            let alias_marker_style = state.colors.theme.get("map.room_alias_marker").style;
            let random_stub_style = state.colors.theme.get("map.room_random_stub").style;
            draw_box_room(
                room, sx, sy, base_style, alias_marker_style, random_stub_style, &state.symbols,
                selected, state.show_alignment, state.show_room_numbers, area, buf,
            );
        }
    }
}

/// Superscript alias-count marker for a room box (SQ-1257 Phase 3): `""` for zero aliases, one
/// Unicode superscript digit (¹²³⁴⁵⁶⁷⁸⁹, U+00B9/U+00B2/U+00B3/U+2074–2079) for 1–9, and `"⁹⁺"`
/// (superscript nine plus a superscript plus, U+207A) for ten or more — the box has no room for
/// a two-digit count, and "at least this many" is still an honest thing to say with one.
/// A marker's own selector supplies its COLOUR; the ground it sits on supplies everything
/// else. A selected room paints its box with `map.room_selected`'s background (and the
/// current room reverses it), and a marker drawn with its selector's full style would punch a
/// hole of default background through that — which is exactly what the alias superscript did
/// on a selected Gnome Room. So take the base style the surrounding text or border was drawn
/// with and swap in only the accent's foreground.
fn accent_on(base: Style, accent: Style) -> Style {
    match accent.fg {
        Some(fg) => base.fg(fg),
        None => base,
    }
}

fn alias_marker(count: usize) -> String {
    super::superscript_count(count)
}

/// Draw a compact (10×4 step) room: 8×3 box with label row.
///
/// Box is 8 cols wide × 3 rows tall (step 10×4, gutter = 2 cols right, 1 row bottom).
/// Normal rooms use rounded corners; current room uses a heavy border with a
/// REVERSED interior (the border itself stays non-reversed).
fn draw_compact_room(
    room: &RenderRoom,
    sx: i32,
    sy: i32,
    style: Style,
    sym: &SymbolSet,
    selected: bool,
    area: Rect,
    buf: &mut Buffer,
) {
    let (bw, bh) = zoom_box_size(Zoom::Compact); // (8, 3)
    let (bw, bh) = (bw as i32, bh as i32);
    // `room.is_current` directly (SQ-0309): the old "REVERSED bit ⇒ current" sniff broke once
    // `map.room_selected`'s own theme default also carries `reversed` — that would misidentify
    // a merely-selected (not current) room as current.
    let is_current = room.is_current;

    // The current room reverses only its interior; keep its border non-reversed.
    let mut border_style = style;
    border_style.add_modifier.remove(Modifier::REVERSED);

    let bs = outline_for(sym, is_current, room.has_layer_portal, selected);
    let (tl, tr, bl, br, h, v) = (bs.tl, bs.tr, bs.bl, bs.br, bs.h, bs.v);

    // Top border
    put_char(buf, sx, sy, tl, border_style, area);
    for dx in 1..bw - 1 {
        put_char(buf, sx + dx, sy, h, border_style, area);
    }
    put_char(buf, sx + bw - 1, sy, tr, border_style, area);

    // Middle row: sides + label (inner width = bw - 2 = 6)
    let label_width = (bw - 2) as usize; // 6
    let label: String = room.label.chars().take(label_width).collect();
    put_char(buf, sx, sy + 1, v, border_style, area);
    put_str(buf, sx + 1, sy + 1, &label, style, area);
    put_char(buf, sx + bw - 1, sy + 1, v, border_style, area);

    // Bottom border
    put_char(buf, sx, sy + bh - 1, bl, border_style, area);
    for dx in 1..bw - 1 {
        put_char(buf, sx + dx, sy + bh - 1, h, border_style, area);
    }
    put_char(buf, sx + bw - 1, sy + bh - 1, br, border_style, area);
}

/// Word-wrap `s` into up to two lines no wider than `width` (break on spaces; a single
/// over-long word, or overflow past two lines, is truncated to `width`).
fn wrap_two(s: &str, width: usize) -> [String; 2] {
    let mut lines = [String::new(), String::new()];
    let mut idx = 0;
    for word in s.split_whitespace() {
        if idx >= 2 {
            break;
        }
        if lines[idx].is_empty() {
            lines[idx] = word.chars().take(width).collect();
        } else if lines[idx].chars().count() + 1 + word.chars().count() <= width {
            lines[idx].push(' ');
            lines[idx].push_str(word);
        } else {
            idx += 1;
            if idx < 2 {
                lines[idx] = word.chars().take(width).collect();
            }
        }
    }
    lines
}

/// Center `s` within `width` columns (truncated to `width` if longer).
fn center(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        return s.chars().take(width).collect();
    }
    let pad = width - len;
    let left = pad / 2;
    format!("{}{}{}", " ".repeat(left), s, " ".repeat(pad - left))
}

/// Draw a boxes (19×11 step) room: bordered box 11 wide × 5 tall.
///
/// Layout (11 cols × 5 rows, within a 19×11 step):
///   Row 0: ╭─────────╮  (or ┏━━━━━━━━━┓ for current room)
///   Row 1: │  name   │  (first word-wrap line, centered)
///   Row 2: │  name2  │  (second word-wrap line, centered)
///   Row 3: │  #id    │  (unique room id, centered; align code appended when enabled)
///   Row 4: ╰─────────╯
///   Gutter: cols 11-18 (right), rows 5-10 (bottom)
///
/// Current room: heavy border (┏ ┓ ┗ ┛ ━ ┃) with a REVERSED interior; the
/// border glyphs themselves are drawn non-reversed.
/// Selected room: yellow style (SELECTED_STYLE).
/// Notes: ● marker in top-right inner corner (row 1, col bw-2).
#[allow(clippy::too_many_arguments)]
fn draw_box_room(
    room: &RenderRoom,
    sx: i32,
    sy: i32,
    style: Style,
    alias_marker_style: Style,
    random_stub_style: Style,
    sym: &SymbolSet,
    selected: bool,
    show_alignment: bool,
    show_room_numbers: bool,
    area: Rect,
    buf: &mut Buffer,
) {
    let (w, h) = zoom_box_size(Zoom::Boxes); // (11, 5)
    let (w, h) = (w as i32, h as i32);
    // `room.is_current` directly (SQ-0309): see `draw_compact_room` for why the old
    // "REVERSED bit ⇒ current" sniff is no longer sound against the themed styles.
    let is_current = room.is_current;

    // The current room reverses only its interior; its border keeps the plain
    // (non-reversed) style so the heavy outline stays readable.
    let mut border_style = style;
    border_style.add_modifier.remove(Modifier::REVERSED);

    // Box outline picked by precedence: current > portal > selected > normal.
    let bs = outline_for(sym, is_current, room.has_layer_portal, selected);
    let (tl, tr, bl, br, horiz, vert) = (bs.tl, bs.tr, bs.bl, bs.br, bs.h, bs.v);

    // Top border
    put_char(buf, sx, sy, tl, border_style, area);
    for dx in 1..w - 1 {
        put_char(buf, sx + dx, sy, horiz, border_style, area);
    }
    put_char(buf, sx + w - 1, sy, tr, border_style, area);

    // Inner rows (h=5 → rows 1, 2, 3 are interior: 1=name wrap, 2=name wrap, 3=#id + align)
    for dy in 1..h - 1 {
        put_char(buf, sx, sy + dy, vert, border_style, area);
        // Fill interior with spaces (for background/style)
        for dx in 1..w - 1 {
            put_char(buf, sx + dx, sy + dy, ' ', style, area);
        }
        put_char(buf, sx + w - 1, sy + dy, vert, border_style, area);
    }

    // Room name word-wrapped + centered across the first two interior rows.
    let iw = (w - 2) as usize; // interior width (9)
    // A room the story keeps renaming (SQ-1257 Phase 3, Lost Pig's gnome tunnels) carries a
    // small superscript count of its other names beside the label. The marker is never dropped
    // for lack of room — the NAME shortens instead, by wrapping into a narrower width that
    // reserves space for it.
    let marker = alias_marker(room.alias_count);
    if marker.is_empty() {
        let name_lines = wrap_two(&room.label, iw);
        put_str(buf, sx + 1, sy + 1, &center(&name_lines[0], iw), style, area);
        put_str(buf, sx + 1, sy + 2, &center(&name_lines[1], iw), style, area);
    } else {
        let marker_w = marker.chars().count();
        let name_w = iw.saturating_sub(marker_w).max(1);
        let name_lines = wrap_two(&room.label, name_w);
        // The marker rides on whichever line actually holds text — the second wrapped line when
        // there is one, else the first — so it reads immediately after the last word of the name.
        let target = if !name_lines[1].is_empty() { 1 } else { 0 };
        for (i, line) in name_lines.iter().enumerate() {
            let y = sy + 1 + i as i32;
            if i == target {
                let full_len = line.chars().count() + marker_w;
                let pad = iw.saturating_sub(full_len);
                let left = (pad / 2) as i32;
                put_str(buf, sx + 1 + left, y, line, style, area);
                put_str(
                    buf,
                    sx + 1 + left + line.chars().count() as i32,
                    y,
                    &marker,
                    accent_on(style, alias_marker_style),
                    area,
                );
            } else {
                put_str(buf, sx + 1, y, &center(line, iw), style, area);
            }
        }
    }

    // Row 3: #id (centered), with alignment diagnostics appended when enabled.
    // Only drawn when show_room_numbers is true; when hidden, the row is freed for portal icons.
    if show_room_numbers {
        let mut row3 = format!("#{}", room.id);
        if show_alignment && !room.align_code.is_empty() {
            row3.push(' ');
            row3.push_str(&room.align_code);
        }
        put_str(buf, sx + 1, sy + 3, &center(&row3, iw), style, area);
    }

    // Notes marker in top-right inner corner (row 1, col w-2).
    if room.has_notes {
        put_char(buf, sx + w - 2, sy + 1, sym.portal.marker, style, area);
    }

    // Self-loop badge (SQ-0666): `↩` plus the directions that lead back into this room, on the
    // bottom interior row. A BADGE, never a drawn loop — a loop out of a box and back into it
    // would need a lane, cross whatever is beside the room, and say no more than three
    // characters do. It is deliberately in the box, not on the border: it is a fact about the
    // room, not a passage to anywhere the reader could follow.
    if !room.self_loops.is_empty() {
        let dirs: String = room
            .self_loops
            .iter()
            .map(|&d| mapper::direction::short_label(d))
            .collect::<Vec<_>>()
            .join("");
        let badge = format!("↩{dirs}");
        let badge: String = badge.chars().take(iw).collect();
        put_str(buf, sx + 1, sy + h - 2, &badge, style, area);
    }

    // Bottom border
    put_char(buf, sx, sy + h - 1, bl, border_style, area);
    for dx in 1..w - 1 {
        put_char(buf, sx + dx, sy + h - 1, horiz, border_style, area);
    }
    put_char(buf, sx + w - 1, sy + h - 1, br, border_style, area);

    // `?` random-exit stubs (SQ-1261): one cell on the border/corner per marked direction, drawn
    // LAST so it overwrites whatever the border loops above already painted there — a straight
    // run of `─`/`│`, or a corner glyph for a diagonal. Never a connector beyond it: the whole
    // point of the mark is that there is nowhere stable to route to.
    for &(dir, count) in &room.random_stubs {
        if let Some((x, y)) = random_stub_pos(sx, sy, dir, w, h) {
            put_str(buf, x, y, &random_stub_marker(count), accent_on(border_style, random_stub_style), area);
        }
    }
}

/// The single glyph a `?` random-exit stub shows (SQ-1261): a bare `?` when nothing is recorded
/// yet, else the superscript count of recorded destinations — never both, since the box border
/// has room for exactly one character per direction (see [`random_stub_pos`]).
fn random_stub_marker(count: usize) -> String {
    if count == 0 { "?".to_string() } else { super::superscript_count(count) }
}

/// Where a `?` stub lands on a room box `w`×`h` cells with its top-left at `(sx, sy)`: a cardinal
/// direction takes the same border-centre cell a real exit arrow would (see
/// [`slot_offset`]/`arrow_for_departure`'s callers), a diagonal takes the corner a diagonal
/// departure draws at (see [`corner_anchor`]) — so a stub never invents a position a real passage
/// would not also use. `None` for a non-planar direction (Up/Down/In/Out/Unknown): those have no
/// side or corner of their own to sit on, and stay visible on the matrix and the room card
/// instead. [`mapper::render::RenderRoom::random_stubs`] never carries a direction a real edge
/// also uses (`mapper::render::render_traced`'s own filter) — a real exit's own arrowhead is
/// drawn in a separate later pass and would win the same cell anyway.
fn random_stub_pos(sx: i32, sy: i32, dir: Direction, w: i32, h: i32) -> Option<(i32, i32)> {
    match dir {
        Direction::N => Some((sx + w / 2, sy)),
        Direction::S => Some((sx + w / 2, sy + h - 1)),
        Direction::E => Some((sx + w - 1, sy + h / 2)),
        Direction::W => Some((sx, sy + h / 2)),
        Direction::NE => Some((sx + w - 1, sy)),
        Direction::NW => Some((sx, sy)),
        Direction::SE => Some((sx + w - 1, sy + h - 1)),
        Direction::SW => Some((sx, sy + h - 1)),
        Direction::Up | Direction::Down | Direction::In | Direction::Out | Direction::Unknown => None,
    }
}

// ── Clipped drawing helpers ───────────────────────────────────────────────────

use super::{put_char, put_str};

// ── Router-measured overlap cleanup ───────────────────────────────────────────

/// Count illegal connector overlaps and clean ┼ crossings in a rendered plan.
/// For each virtual cell, OR each connector's mask bits (a connector may write a cell
/// twice). A cell written by ≥2 DISTINCT connectors is a clean crossing ONLY if exactly
/// 2 connectors share it, one contributing exactly E|W and the other exactly N|S;
/// everything else (≥3 connectors, corner-on-corner, parallel run-alongside) is illegal.
/// Returns (illegal_count, clean_crossing_count). Counts are order-independent, so the
/// internal HashMap accumulation is deterministic in its RESULT.
pub(crate) fn overlap_stats(
    plan: &mapper::route::RoutePlan, cols: &PosTable, rows: &PosTable,
) -> (usize, usize) {
    use std::collections::{BTreeMap, HashMap};
    let mut owners: HashMap<(i32, i32), BTreeMap<usize, u8>> = HashMap::new();
    for (ci, conn) in plan.connectors.iter().enumerate() {
        // Deliberately the orthogonal reading (`None`, no half-diagonals): this metric scores the
        // ROUTER's layout quality and feeds the tidy pipeline, so it must not move when a user
        // toggles the `diagonal_corners` DISPLAY setting — otherwise tidy would make different
        // layout decisions per theme. Both settings share the router's corner departure, so this
        // sees the same route either way; only the glyphs differ. (SQ-0314)
        if let Some(plot) = plot_connector(conn, cols, rows, None) {
            for (c, mask) in &plot.cells {
                *owners.entry(*c).or_default().entry(ci).or_insert(0) |= *mask;
            }
        }
    }
    let ew = DIR_E | DIR_W;
    let ns = DIR_N | DIR_S;
    let mut expected = [ns, ew];
    expected.sort_unstable();
    let (mut illegal, mut crossings) = (0usize, 0usize);
    for per_conn in owners.values() {
        if per_conn.len() < 2 {
            continue;
        }
        // Merge junction: every connector meeting at this cell belongs to the SAME unordered room
        // pair (a trunk plus its merge stubs joining it). That is a legal T-junction, not an overlap.
        let same_pair = {
            let mut pairs = per_conn.keys().map(|&ci| {
                let c = &plan.connectors[ci];
                (c.origin.min(c.dest), c.origin.max(c.dest))
            });
            let first = pairs.next().unwrap();
            pairs.all(|p| p == first)
        };
        if same_pair {
            continue;
        }
        let mut masks: Vec<u8> = per_conn.values().copied().collect();
        masks.sort_unstable();
        if per_conn.len() == 2 && masks == expected {
            crossings += 1;
        } else {
            illegal += 1;
        }
    }
    (illegal, crossings)
}

/// Render `graph` and return its (illegal_overlaps, crossings).
pub(crate) fn render_overlap_stats(graph: &mapper::graph::MapGraph) -> (usize, usize) {
    let rm = mapper::render::render(graph);
    let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);
    overlap_stats(&rm.plan, &cols, &rows)
}

/// True unless moving room `id` to `cell` would disturb a well-placed Up/Down relationship: an Up
/// room must stay north of its partner (a Down room south), and a room currently stacked in its
/// partner's COLUMN must stay in that column. This stops overlap cleanup from sacrificing a stacked
/// portal room — flipping its side OR dragging it off-column — to clear an overlap; other rooms can
/// move instead. Only currently-good relationships are protected; an already-broken one imposes
/// nothing.
fn move_keeps_updown_sides(
    graph: &mapper::graph::MapGraph,
    id: mapper::graph::RoomId,
    cell: (i32, i32),
) -> bool {
    use mapper::direction::Direction;
    for c in graph.connections() {
        let req = match c.dir {
            Direction::Up => -1,  // dest north of origin (dest.y - origin.y < 0)
            Direction::Down => 1, // dest south of origin
            _ => continue,
        };
        if c.origin != id && c.dest != id {
            continue;
        }
        let (Some(o0), Some(d0)) = (
            graph.room(c.origin).and_then(|r| r.pos),
            graph.room(c.dest).and_then(|r| r.pos),
        ) else {
            continue;
        };
        let o = if c.origin == id { cell } else { o0 };
        let d = if c.dest == id { cell } else { d0 };
        // Side: a currently-correct side must stay correct.
        if (d0.1 - o0.1).signum() == req && (d.1 - o.1).signum() != req {
            return false;
        }
        // Column: a room currently stacked in its partner's column must stay in it.
        if d0.0 == o0.0 && d.0 != o.0 {
            return false;
        }
    }
    true
}

/// Per-room axis lock derived from reciprocal cardinal chains, mirroring the hard equality the
/// VPSC solver (`relayout_auto`) enforces. A room in a reciprocal N/S chain (shares a column) is
/// COLUMN-locked — a greedy cleanup move may only change its Y, never its X; a room in a reciprocal
/// E/W chain (shares a row) is ROW-locked — only its X, never its Y. A room in BOTH is fully pinned.
/// The greedy stages would otherwise break a reciprocal by sliding a locked room off its shared axis
/// to clear an overlap; this is a hard constraint (same spirit as `move_keeps_updown_sides`), so an
/// overlap that can only be cleared by moving a reciprocal room off-axis is left as a residual.
///
/// Precomputed ONCE per cleanup call: chains are a pure function of the graph's connections, which
/// the greedy passes never mutate (they only `set_pos`), so the lock never changes mid-cleanup.
/// Returns `(x_locked, y_locked)` per room; absent rooms are unrestricted.
fn reciprocal_axis_locks(
    graph: &mapper::graph::MapGraph,
) -> std::collections::HashMap<mapper::graph::RoomId, (bool, bool)> {
    let chains = mapper::layout::detect_chains(graph);
    let mut locks: std::collections::HashMap<mapper::graph::RoomId, (bool, bool)> =
        std::collections::HashMap::new();
    for &id in chains.ns.keys() {
        locks.entry(id).or_default().0 = true; // N/S chain → column-locked (X fixed)
    }
    for &id in chains.ew.keys() {
        locks.entry(id).or_default().1 = true; // E/W chain → row-locked (Y fixed)
    }
    locks
}

/// Nudge rooms (bounded Chebyshev `radius`, ≤ `max_passes` passes) until the rendered
/// plan has zero illegal overlaps, secondarily fewer crossings. Deterministic, no overlap,
/// integer cells. Existing position is restored on every rejected trial.
pub(crate) fn cleanup_overlaps(graph: &mut mapper::graph::MapGraph, radius: i32, max_passes: usize) {
    cleanup_overlaps_observed(graph, radius, max_passes, None);
}

/// Observer for animated tidy passes: `(graph, kind, detail, stats)` per step.
type TidyObserver<'a> = &'a mut dyn FnMut(&mapper::graph::MapGraph, &str, &str, &mapper::layout::TidyStats);

pub(crate) fn cleanup_overlaps_observed(
    graph: &mut mapper::graph::MapGraph,
    radius: i32,
    max_passes: usize,
    mut obs: Option<TidyObserver>,
) {
    let moves: Vec<(i32, i32)> = {
        let mut v = Vec::new();
        for dist in 1..=radius {
            let mut candidates: Vec<(i32, i32)> = (-dist..=dist)
                .flat_map(|dy| (-dist..=dist).map(move |dx| (dy, dx)))
                .filter(|&(dy, dx)| dy.abs().max(dx.abs()) == dist)
                .collect();
            candidates.sort_unstable();
            v.extend(candidates);
        }
        v
    };

    let mut stats = mapper::layout::TidyStats::default();
    let locks = reciprocal_axis_locks(graph);

    for _ in 0..max_passes {
        let base = render_overlap_stats(graph);
        if base.0 == 0 {
            break;
        }

        let room_ids: Vec<mapper::graph::RoomId> = graph
            .rooms()
            .filter(|r| r.pos.is_some())
            .map(|r| r.id)
            .collect();

        type Key = (usize, usize, usize, usize, usize, mapper::graph::RoomId, usize);
        let mut best: Option<(Key, mapper::graph::RoomId, (i32, i32))> = None;
        for &id in &room_ids {
            let Some(orig) = graph.room(id).and_then(|r| r.pos) else { continue };
            // Reciprocal N/S rooms are column-locked (X fixed), E/W rooms row-locked (Y fixed).
            let (x_locked, y_locked) = locks.get(&id).copied().unwrap_or((false, false));
            let score_orig = mapper::layout::room_side_score(graph, id);
            let align_orig = mapper::layout::room_alignment_score(graph, id);
            let degree = mapper::layout::room_compass_degree(graph, id);
            for (move_idx, &(dy, dx)) in moves.iter().enumerate() {
                // Skip any candidate that would slide a reciprocal-locked room off its shared axis.
                if (x_locked && dx != 0) || (y_locked && dy != 0) {
                    continue;
                }
                let trial = (orig.0 + dx, orig.1 + dy);
                if graph.rooms().any(|r| r.id != id && r.pos == Some(trial)) {
                    continue;
                }
                if !move_keeps_updown_sides(graph, id, trial) {
                    continue;
                }
                graph.set_pos(id, trial);
                let s = render_overlap_stats(graph);
                let score_trial = mapper::layout::room_side_score(graph, id);
                let align_trial = mapper::layout::room_alignment_score(graph, id);
                graph.set_pos(id, orig);
                if score_trial < score_orig {
                    continue;
                }
                if (s.0, s.1) < (base.0, base.1) {
                    let align_broken = align_orig.saturating_sub(align_trial);
                    let broken = score_orig.saturating_sub(score_trial);
                    let key: Key = (s.0, align_broken, broken, s.1, degree, id, move_idx);
                    if best.as_ref().is_none_or(|(bk, _, _)| key < *bk) {
                        best = Some((key, id, trial));
                    }
                }
            }
        }

        match best {
            Some((_, id, trial)) => {
                let orig = graph.room(id).and_then(|r| r.pos).unwrap_or(trial);
                graph.set_pos(id, trial);
                if let Some(ref mut cb) = obs {
                    stats.overlaps_resolved += 1;
                    let name = graph.room(id).map(|r| r.name.as_str()).unwrap_or("?").to_owned();
                    let desc = format!(
                        "Overlap cleanup: moved room {} ({}) from {:?} to {:?} to clear overlap.",
                        id, name, orig, trial
                    );
                    cb(graph, "cleanup_overlaps", &desc, &stats);
                }
            }
            None => break,
        }
    }
}

/// Nudge rooms to satisfy currently-VIOLATED directional hints — e.g. a one-way `W` edge whose dest
/// ended up east of its origin because a post-solve stage (contiguity ejection, collision spiral)
/// moved a room across it. Sibling to [`cleanup_overlaps`]; runs after it in the Retidy flow.
///
/// Greedy and bounded, like cleanup, but it OPTIMIZES `directional_hint_score` instead of overlaps:
/// each pass commits the single room move that most increases the total satisfied-hint count while
/// (a) not introducing any illegal connector overlap and (b) not breaking any exact row/column
/// alignment the moved room currently holds (`room_alignment_score`, so it never undoes the chain
/// alignment relayout/cleanup established). Only strict improvements are taken, so it converges.
pub(crate) fn repair_directional_hints(graph: &mut mapper::graph::MapGraph, radius: i32, max_passes: usize) {
    repair_directional_hints_observed(graph, radius, max_passes, None);
}

pub(crate) fn repair_directional_hints_observed(
    graph: &mut mapper::graph::MapGraph,
    radius: i32,
    max_passes: usize,
    mut obs: Option<TidyObserver>,
) {
    let moves: Vec<(i32, i32)> = {
        let mut v = Vec::new();
        for dist in 1..=radius {
            let mut candidates: Vec<(i32, i32)> = (-dist..=dist)
                .flat_map(|dy| (-dist..=dist).map(move |dx| (dy, dx)))
                .filter(|&(dy, dx)| dy.abs().max(dx.abs()) == dist)
                .collect();
            candidates.sort_unstable();
            v.extend(candidates);
        }
        v
    };

    let mut stats = mapper::layout::TidyStats::default();
    let locks = reciprocal_axis_locks(graph);

    for _ in 0..max_passes {
        let base = render_overlap_stats(graph);
        let base_score = mapper::layout::directional_hint_score(graph);

        let room_ids: Vec<mapper::graph::RoomId> =
            graph.rooms().filter(|r| r.pos.is_some()).map(|r| r.id).collect();

        type Key = (std::cmp::Reverse<usize>, usize, usize, usize, mapper::graph::RoomId, usize);
        let mut best: Option<(Key, mapper::graph::RoomId, (i32, i32))> = None;
        for &id in &room_ids {
            let Some(orig) = graph.room(id).and_then(|r| r.pos) else { continue };
            // Reciprocal N/S rooms are column-locked (X fixed), E/W rooms row-locked (Y fixed).
            let (x_locked, y_locked) = locks.get(&id).copied().unwrap_or((false, false));
            let align_orig = mapper::layout::room_alignment_score(graph, id);
            let degree = mapper::layout::room_compass_degree(graph, id);
            for (move_idx, &(dy, dx)) in moves.iter().enumerate() {
                // Skip any candidate that would slide a reciprocal-locked room off its shared axis.
                if (x_locked && dx != 0) || (y_locked && dy != 0) {
                    continue;
                }
                let trial = (orig.0 + dx, orig.1 + dy);
                if graph.rooms().any(|r| r.id != id && r.pos == Some(trial)) {
                    continue;
                }
                graph.set_pos(id, trial);
                let s = render_overlap_stats(graph);
                let score = mapper::layout::directional_hint_score(graph);
                let align_trial = mapper::layout::room_alignment_score(graph, id);
                graph.set_pos(id, orig);
                if score > base_score && s.0 <= base.0 && align_trial >= align_orig {
                    let gain = score - base_score;
                    let key: Key = (std::cmp::Reverse(gain), s.0, s.1, degree, id, move_idx);
                    if best.as_ref().is_none_or(|(bk, _, _)| key < *bk) {
                        best = Some((key, id, trial));
                    }
                }
            }
        }

        match best {
            Some((_, id, trial)) => {
                let orig = graph.room(id).and_then(|r| r.pos).unwrap_or(trial);
                graph.set_pos(id, trial);
                if let Some(ref mut cb) = obs {
                    stats.hints_repaired += 1;
                    let name = graph.room(id).map(|r| r.name.as_str()).unwrap_or("?").to_owned();
                    let desc = format!(
                        "Repair hint: moved room {} ({}) from {:?} to {:?} to restore directional edge.",
                        id, name, orig, trial
                    );
                    cb(graph, "repair_hints", &desc, &stats);
                }
            }
            None => break,
        }
    }
}

/// Collapse the fully-empty interior rows and columns the tidy passes leave behind (e.g. a gap
/// opened when `repair_directional_hints` pushes a room out), shifting rooms together so the map
/// carries no wasted gap line. Runs last in the Retidy flow.
///
/// A collapse moves every room BEYOND the empty line one cell toward it, leaving the rest put. That
/// translates one half-plane uniformly, so every room keeps its relative order on both axes — all
/// directional and exact-alignment relationships survive — and no two rooms can share a cell. The
/// only thing a tighter layout can disturb is connector routing, so if the result raises illegal
/// overlaps the whole compaction is reverted (cosmetic tightening is never worth a new overlap).
pub(crate) fn compact_empty_lines(graph: &mut mapper::graph::MapGraph) {
    compact_empty_lines_observed(graph, None);
}

pub(crate) fn compact_empty_lines_observed(
    graph: &mut mapper::graph::MapGraph,
    mut obs: Option<TidyObserver>,
) {
    let stats = mapper::layout::TidyStats::default();

    for is_x in [true, false] {
        let mut floor = i32::MIN;
        loop {
            let coords: std::collections::BTreeSet<i32> = graph
                .rooms()
                .filter_map(|r| r.pos.map(|p| if is_x { p.0 } else { p.1 }))
                .collect();
            let (Some(&min), Some(&max)) = (coords.iter().next(), coords.iter().next_back()) else {
                break;
            };
            let Some(empty) = ((min + 1)..max).find(|c| !coords.contains(c) && *c > floor) else {
                break;
            };
            let rooms: Vec<(mapper::graph::RoomId, (i32, i32))> =
                graph.rooms().filter_map(|r| r.pos.map(|p| (r.id, p))).collect();
            let before = render_overlap_stats(graph).0;
            for &(id, p) in &rooms {
                let c = if is_x { p.0 } else { p.1 };
                if c > empty {
                    graph.set_pos(id, if is_x { (p.0 - 1, p.1) } else { (p.0, p.1 - 1) });
                }
            }
            if render_overlap_stats(graph).0 > before {
                for (id, p) in rooms {
                    graph.set_pos(id, p);
                }
                floor = empty;
            } else {
                if let Some(ref mut cb) = obs {
                    let axis = if is_x { "column" } else { "row" };
                    let desc = format!(
                        "Compact: collapsed empty {} at coordinate {}.",
                        axis, empty
                    );
                    cb(graph, "compact", &desc, &stats);
                }
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use mapper::direction::Direction;
    use mapper::mapper::Mapper;
    use mapper::render::render;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    /// Build a `Theme` with the given selectors' style (fg/bg/modifiers) overridden (like a
    /// `style.toml` decl), so tests exercising render code migrated to `theme.get("<selector>")`
    /// (SQ-0309) can still inject a custom style instead of mutating the (no-longer-read) legacy
    /// `ColorScheme` field. See `render/transcript.rs`'s `theme_with_overrides` for the fg-only
    /// original; this copy also carries bg + modifiers since several map.rs tests need both.
    fn theme_with_overrides(overrides: &[(&str, Style)]) -> crate::theme::resolve::Theme {
        let mut decls = std::collections::HashMap::new();
        for &(sel, style) in overrides {
            let m = style.add_modifier;
            decls.insert(sel.to_string(), crate::theme::registry::Delta {
                fg: style.fg,
                bg: style.bg,
                // `Some` either way, never `None`: this helper injects a COMPLETE
                // style, so a modifier the caller left off must be explicitly off
                // rather than inherited from the parent (SQ-1171's tri-state). The
                // old plain `bool` could not say that — an absent modifier lowered
                // to a no-op, so a test injecting a non-bold style over a bold
                // parent silently got bold back.
                bold: Some(m.contains(Modifier::BOLD)),
                italic: Some(m.contains(Modifier::ITALIC)),
                underline: Some(m.contains(Modifier::UNDERLINED)),
                reversed: Some(m.contains(Modifier::REVERSED)),
                dim: Some(m.contains(Modifier::DIM)),
                ..crate::theme::registry::Delta::EMPTY
            });
        }
        crate::theme::resolve::resolve(
            &crate::theme::resolve::Roles::terminal_default(),
            &decls,
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
        )
    }

    /// The two edge-midpoints each half-diagonal reaches, per its Unicode name. Guards the
    /// bits table against the endpoints actually being somewhere else (SQ-0356).
    #[test]
    fn chain_glyph_bits_name_each_half_diagonals_own_endpoints() {
        let p = crate::symbols::SymbolSet::default().path;
        // U+1FBA0 upper-centre ↔ middle-left; U+1FBA1 upper-centre ↔ middle-right;
        // U+1FBA2 middle-left ↔ lower-centre; U+1FBA3 middle-right ↔ lower-centre.
        assert_eq!(chain_glyph_bits(p.diag_ul, &p), Some(DIR_N | DIR_W));
        assert_eq!(chain_glyph_bits(p.diag_ur, &p), Some(DIR_N | DIR_E));
        assert_eq!(chain_glyph_bits(p.diag_ll, &p), Some(DIR_S | DIR_W));
        assert_eq!(chain_glyph_bits(p.diag_lr, &p), Some(DIR_S | DIR_E));
        // The fill glyphs a chain also emits reach the same midpoints ─/│ always do.
        assert_eq!(chain_glyph_bits(p.ns, &p), Some(DIR_N | DIR_S));
        assert_eq!(chain_glyph_bits(p.ew, &p), Some(DIR_E | DIR_W));
        // Anything a chain never emits has no merge reading.
        assert_eq!(chain_glyph_bits(p.nesw, &p), None);
        assert_eq!(chain_glyph_bits('x', &p), None);
    }

    /// SQ-0356: a chain cell landing on another connector's orthogonal run must MERGE with it.
    ///
    /// The fixture was originally Zork's "West of House" (#68) / "North of House" (#143) pair,
    /// whose reciprocal NE/SW diagonal collided with a second W edge back between the SAME two
    /// rooms. SQ-0522 collapses same-pair extras into icons, so that shape can no longer produce
    /// two connectors at all. The diagonal is kept and the colliding run now belongs to a
    /// DIFFERENT pair — a column-aligned A/B link that lane-routes past #68 — which is the shape
    /// this merge exists for anyway: two unrelated connectors wanting one cell.
    #[test]
    fn a_chain_cell_on_another_connectors_run_merges_into_a_junction() {
        use mapper::graph::MapGraph;
        use mapper::render::render;

        let mut g = MapGraph::new();
        g.upsert_room(68, "West of House".into());
        g.upsert_room(143, "North of House".into());
        g.set_pos(68, (-2, 3));
        g.set_pos(143, (1, 2));
        g.add_edge(68, Direction::NE, 143);
        g.add_edge(143, Direction::SW, 68); // reciprocal: collapses with the NE into one diagonal
        g.upsert_room(300, "A".into());
        g.upsert_room(301, "B".into());
        g.set_pos(300, (-2, -1));
        g.set_pos(301, (-2, 4)); // same column as #68, which sits between them
        g.add_edge(300, Direction::S, 301);
        g.add_edge(301, Direction::N, 300);

        let rm = render(&g);
        let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);
        let glyphs = crate::symbols::SymbolSet::default().path;

        // Find where a chain cell and an orthogonal run want the same cell. Derived from the
        // plots, not hard-coded, so a geometry change relocates the assertion instead of
        // silently aiming it at blank space.
        let mut chain: std::collections::HashMap<(i32, i32), char> = std::collections::HashMap::new();
        let mut orth: std::collections::HashMap<(i32, i32), u8> = std::collections::HashMap::new();
        for conn in rm.plan.connectors.iter() {
            let Some(plot) = plot_connector(conn, &cols, &rows, Some(&glyphs)) else { continue };
            for (c, mask) in &plot.cells {
                *orth.entry(*c).or_insert(0) |= *mask;
            }
            chain.extend(plot.diag_cells.iter().cloned());
        }
        let hits: Vec<(i32, i32)> =
            chain.keys().filter(|c| orth.contains_key(c)).cloned().collect();
        assert_eq!(hits.len(), 1, "fixture must still produce exactly one collision");
        let hit = hits[0];
        // The junction both strokes describe: the run's mask ORed with the chain glyph's bits.
        // Derived the same way the renderer derives it, so this survives a geometry change.
        let want = glyph_for(orth[&hit] | chain_glyph_bits(chain[&hit], &glyphs).expect("a chain glyph"), &glyphs)
            .expect("the merged mask has a glyph");

        // Render the whole map off-screen, the way `map_dump::ascii_map` does.
        let ((min_col, min_row), _) = rm.bounds;
        let pad_w = cols.room_pixel(min_col) - cols.room_pixel(min_col - 2);
        let pad_h = rows.room_pixel(min_row) - rows.room_pixel(min_row - 2);
        let area = Rect::new(
            0,
            0,
            (cols.total_pixels() + pad_w + 30) as u16,
            (rows.total_pixels() + pad_h + 20) as u16,
        );
        let mut state = AppState::default();
        state.zoom = Zoom::Boxes;
        state.scroll = (min_col - 2, min_row - 2);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let sx = hit.0 - cols.room_pixel(min_col - 2);
        let sy = hit.1 - rows.room_pixel(min_row - 2);
        let sym = buf.cell((sx as u16, sy as u16)).unwrap().symbol();

        // Both strokes must survive as one junction glyph. Neither line may lose the cell —
        // a bare `│` would be the vertical winning, a bare chain glyph the diagonal winning.
        assert_eq!(
            sym,
            want.to_string(),
            "chain-on-run cell must render as the junction carrying both strokes, got {sym:?}"
        );
    }

    #[test]
    fn up_connector_draws_updown_glyph_on_border_not_arrow() {
        // A at origin, B directly north, reached by Up. At Boxes zoom the Up connector
        // must render the up glyph (default '↑') somewhere on the border between them,
        // and must NOT render a filled N arrow ('▲') for that vertical link.
        use mapper::direction::Direction;
        use mapper::graph::MapGraph;

        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::Up, 2);

        let state = AppState::default(); // Boxes zoom by default
        let rm = mapper::render::render(&g);
        let area = Rect::new(0, 0, 60, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let text: String = buf.content.iter().flat_map(|c| c.symbol().chars()).collect();
        assert!(text.contains('↑'), "the Up connector shows the up glyph on the border");
        assert!(!text.contains('▲'), "the Up connector must NOT render a filled N arrow");
    }

    /// A--North-->B AND A--Up-->B: only the N line is drawn (SQ-0522 priority). The `\u{2191}` used to be
    /// re-stamped on the border so vertical access still read, but a glyph with no line attached
    /// says a staircase exists while pointing nowhere — the room inspector answers that properly.
    #[test]
    fn a_pair_with_both_a_compass_edge_and_a_staircase_draws_only_the_compass_line() {
        use mapper::direction::Direction;
        use mapper::graph::MapGraph;

        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::N, 2);
        g.add_edge(1, Direction::Up, 2);

        let state = AppState::default(); // default/Boxes view, numbers per default
        let rm = mapper::render::render(&g);
        let area = Rect::new(0, 0, 60, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let up = state.symbols.portal.up;
        let text: String = buf.content.iter().flat_map(|c| c.symbol().chars()).collect();
        // SQ-0689: the staircase loses the LINE to N on priority, but no longer vanishes — it
        // stamps its ↑ beside the shared line's anchor. One line, both passages visible.
        assert_eq!(text.matches(up).count(), 1, "the collapsed staircase stamps its glyph");
        assert!(text.contains(state.symbols.arrows.north), "the N passage keeps its own arrowhead");
    }
    #[test]
    fn reciprocal_updown_connector_draws_glyph_at_both_ends() {
        // Task 9 (SQ-0216): a reciprocal up/down connector draws its glyph at BOTH ends —
        // the up glyph on the lower room's (departure) top border, the down glyph on the
        // upper room's (arrival) bottom border — never an arrow at the far end. Build a
        // routed one-way Up connector via the real pipeline, then patch its metadata to
        // simulate the router's collapse (`reciprocal = true`, `entry_dir = Some(Down)`) so
        // this exercises `render_lane_connectors`'s far-end block directly, independent of
        // whether the router itself pairs the edge.
        use mapper::direction::Direction;
        use mapper::graph::MapGraph;

        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into()); // lower room
        g.upsert_room(2, "B".into()); // upper room (north of A)
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::Up, 2);

        let rm = mapper::render::render(&g);
        let mut plan = rm.plan.clone();
        let conn = plan
            .connectors
            .iter_mut()
            .find(|c| c.exit_dir == Direction::Up)
            .expect("routed Up connector");
        conn.reciprocal = true;
        conn.entry_dir = Some(Direction::Down);

        let (cols, rows) = boxes_axes(&plan, rm.bounds);
        let area = Rect::new(0, 0, 60, 30);
        let offset = (
            area.x as i32 - cols.room_pixel(rm.bounds.0 .0),
            area.y as i32 - rows.room_pixel(rm.bounds.0 .1),
        );
        let mut buf = Buffer::empty(area);
        let state = AppState::default();
        let arrowheads = render_lane_connectors(
            &plan,
            &cols,
            &rows,
            offset,
            area,
            &mut buf,
            &state.symbols.arrows,
            &state.symbols.path,
            &state.symbols.portal,
            &state.colors,
            state.symbols.diagonal_corners,
            &edge_kinds(&rm),
        );

        let dep = arrowheads.iter().find(|a| a.room == 1).expect("A's departure glyph");
        let arr = arrowheads.iter().find(|a| a.room == 2).expect("B's arrival glyph");
        assert_eq!(dep.glyph, "↑", "A (lower room) shows the up glyph on its top border");
        assert_eq!(arr.glyph, "↓", "B (upper room) shows the down glyph on its bottom border, not an arrow");
    }

    #[test]
    fn reciprocal_updown_glyphs_sit_on_north_and_south_borders() {
        // Task 10 (SQ-0216, regression lock-in): A at origin, B directly north, joined by a
        // real reciprocal Up/Down pair. The up glyph must land on a TOP border row (north
        // side, A's border) and the down glyph on a BOTTOM border row (south side, B's
        // border) — never swapped, never on a left/right side.
        use mapper::direction::Direction;
        use mapper::graph::MapGraph;

        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(2, Direction::Down, 1);

        let rm = render(&g);
        let mut state = AppState::default();
        state.zoom = Zoom::Boxes;
        state.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 60, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // Find the up glyph and the down glyph, record their rows.
        let up = state.symbols.portal.up; // default '↑'
        let down = state.symbols.portal.down; // default '↓'
        let mut up_row = None;
        let mut down_row = None;
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                let s = buf.cell((x, y)).expect("cell in area").symbol();
                if s.starts_with(up) {
                    up_row = Some(y);
                }
                if s.starts_with(down) {
                    down_row = Some(y);
                }
            }
        }
        let (up_row, down_row) = (up_row.expect("up glyph present"), down_row.expect("down glyph present"));
        // B is north of A (A is the lower room, B the upper room). The up glyph marks A's
        // (lower room's) top border; the down glyph marks B's (upper room's) bottom border.
        // Since the upper room sits at a smaller screen row than the lower room, the down
        // glyph's row is ABOVE the up glyph's row: down_row < up_row.
        assert!(
            down_row < up_row,
            "down glyph (upper room's south border) sits above the up glyph (lower room's north border): down_row={down_row} up_row={up_row}"
        );
    }

    #[test]
    fn loc_method_label_strings() {
        use zvm::location::LocationMethod::*;
        assert_eq!(loc_method_label(GlobalVar0), "via status variable");
        assert_eq!(loc_method_label(PlayerParent), "via player object");
        assert_eq!(loc_method_label(StatusName), "via name match");
        assert_eq!(loc_method_label(NameOnly), "via name (unlinked)");
        assert_eq!(loc_method_label(RoomHeading), "via room heading");
    }

    /// SQ-0349: the far half of the recentre wiring. `Action::Recenter` reads
    /// `state.map_pane_size`; nothing centres correctly unless the renderer writes it.
    #[test]
    fn rendering_the_map_records_the_pane_size_for_recentring() {
        use mapper::graph::MapGraph;
        let g = MapGraph::default();
        let rm = mapper::render::render(&g);
        let state = AppState::default();
        assert!(state.map_pane_size.get().is_none(), "nothing measured before a render");

        let area = Rect::new(3, 2, 140, 48); // offset origin: the SIZE is what recentring needs
        let mut buf = Buffer::empty(Rect::new(0, 0, 200, 60));
        render_map_layered(&rm, &g, &state, area, &mut buf);

        assert_eq!(
            state.map_pane_size.get(),
            Some((140, 48)),
            "the pane actually drawn into is what a later recentre must measure"
        );
    }

    #[test]
    fn cleanup_reduces_overlaps_keeping_updown_protected_rooms_aligned() {
        // The A129 house. With correct up/down placement (SQ-0216 #3), room 26 sits SOUTHEAST of
        // 25 — its Up edge (26→Up→25) marks it Y-constrained in the align stage, so it is NOT
        // flattened onto 25's row — and it stacks a protected up/down column with 27
        // (26→Down→27, 27→Up→26 ⇒ 27 directly below 26). cleanup_overlaps must keep those
        // hard-protected up/down rooms in place (`move_keeps_updown_sides`) while nudging
        // unprotected rooms to clear what overlaps it can.
        //
        // HARD-PROTECT DECISION (SQ-0216 #3): up/down placement is inviolable. On this dense
        // 26/27/136 cluster the protected 26↔27 up/down lane used to leave 2 illegal connector
        // overlaps unclearable by cleanup's greedy single-room search. SQ-0222 removed that residual
        // at its source: the straight up/down line now keeps its center slot instead of jogging
        // across the weaving compass connector, so the cluster routes cleanly and cleanup reaches 0
        // WITHOUT moving any protected up/down room. This test still fails if overlaps reappear
        // (routing/cleanup regressed) or the protected up/down column breaks.
        use mapper::graph::MapGraph;
        use Direction::*;
        let mut g = MapGraph::new();
        for id in [25u16, 26, 27, 74, 75, 76, 77, 78, 79, 80, 81, 136, 143, 180, 193, 201, 203, 239] {
            g.upsert_room(id, "r".into());
        }
        for (o, d, dst) in [
            (180, N, 81), (81, W, 180), (180, W, 78), (78, N, 143), (143, E, 77), (77, S, 74), (74, S, 76),
            (76, W, 78), (143, W, 78), (78, S, 76), (76, N, 74), (74, E, 25), (25, W, 76), (74, W, 79), (79, E, 74),
            (25, E, 26), (26, Up, 25), (78, E, 75), (77, E, 239), (239, N, 77), (77, Unknown, 180), (180, S, 80),
            (80, W, 180), (80, E, 79), (79, S, 80), (79, N, 81), (81, E, 79), (80, S, 76), (76, Unknown, 180),
            (79, Unknown, 180), (75, S, 81), (75, W, 78), (75, E, 77), (239, S, 77), (77, W, 75), (75, N, 143),
            (143, S, 75), (26, Down, 27), (27, N, 136), (136, SW, 27), (27, Up, 26), (26, Unknown, 180),
            (79, W, 203), (203, W, 193), (193, E, 203), (203, E, 79), (203, Up, 201), (201, Down, 203),
        ] {
            g.add_edge(o, d, dst);
        }
        mapper::layout::relayout_auto(&mut g);
        cleanup_overlaps(&mut g, 3, 40);
        // SQ-0222: the 26/27/136 cluster now routes cleanly, so cleanup clears every illegal overlap.
        assert_eq!(render_overlap_stats(&g).0, 0,
            "cleanup clears all illegal overlaps while keeping protected up/down rooms in place");
        let p = |id: u16| g.room(id).unwrap().pos.unwrap();
        // Up/down-protected column stays aligned: 27 stays directly below 26 (26→Down→27).
        assert_eq!(p(26).0, p(27).0, "26/27 up/down column must stay aligned: 26={:?} 27={:?}", p(26), p(27));
        assert!(p(27).1 > p(26).1, "27 stays south of 26 (below it in the up/down lane)");
        // 26's Up edge to 25 stays satisfied: 25 north of 26, and directional x-order 74<25<26.
        assert!(p(25).1 < p(26).1, "25 stays north of 26 (26→Up→25): 25={:?} 26={:?}", p(25), p(26));
        assert!(p(25).0 > p(74).0 && p(26).0 > p(25).0, "directional x-order 74<25<26 preserved");
    }

    #[test]
    fn cell_to_screen_respects_scroll_and_offarea() {
        let area = Rect::new(0, 0, 80, 80);

        // Cell (0,0) with no scroll at Boxes → screen (0,0), inside area.
        let on = cell_to_screen((0, 0), Zoom::Boxes, (0, 0), area);
        assert_eq!(on, Some((0, 0)));

        // Cell (1,0) at Boxes → x = 0 + (1-0)*19 = 19
        let right = cell_to_screen((1, 0), Zoom::Boxes, (0, 0), area);
        assert_eq!(right, Some((19, 0)));

        // Cell (0,1) at Boxes → y = 0 + (1-0)*11 = 11
        let down = cell_to_screen((0, 1), Zoom::Boxes, (0, 0), area);
        assert_eq!(down, Some((0, 11)));

        // Far off-area cell.
        let off = cell_to_screen((1000, 1000), Zoom::Boxes, (0, 0), area);
        assert!(off.is_none());

        // Scroll pushes cell off-screen: scroll=(1,0) so cell (0,0) → x = 0+(0-1)*19 = -19 → None.
        let scrolled_off = cell_to_screen((0, 0), Zoom::Boxes, (1, 0), area);
        assert!(scrolled_off.is_none());

        // Compact zoom: step 12×5 → cell (1,1) → (12, 5)
        let compact = cell_to_screen((1, 1), Zoom::Compact, (0, 0), area);
        assert_eq!(compact, Some((12, 5)));

        // Overview zoom: step 2×2 → cell (5,3) → (10, 6)
        let overview = cell_to_screen((5, 3), Zoom::Overview, (0, 0), area);
        assert_eq!(overview, Some((10, 6)));
    }

    #[test]
    fn renders_current_room_highlighted_into_buffer() {
        let mut m = Mapper::default();
        m.observe(1, "Start", None);
        m.observe(2, "North", Some(Direction::N));
        let rm = render(&m.graph);
        // room 2 ("North") is placed at cell (0, -1) by the layout engine.
        // With default scroll (0,0) and Boxes zoom (step_h=6), its screen y = -6 (off screen).
        // Scroll up by 1 row so that cell (0,-1) maps to screen y=0.
        let mut state = AppState::default();
        state.scroll = (0, -1); // scroll y=-1 so cell (0,-1) → screen y = 0 + (-1-(-1))*6 = 0
        // The default `map.room_current` theme style (accent, no reversed) no longer exercises
        // the border/interior REVERSED split this test guards, so override it to a REVERSED
        // style (like the old ColorScheme default) via the theme, matching SQ-0309's rewire of
        // direct `ColorScheme` field mutation to `theme_with_overrides` for migrated selectors.
        state.colors.theme = theme_with_overrides(&[
            ("map.room_current", Style::new().add_modifier(Modifier::REVERSED).fg(Color::White)),
        ]);

        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // SOME non-space content was drawn (rooms/connectors present).
        let drawn = buf.content.iter().filter(|c| c.symbol() != " ").count();
        assert!(drawn > 0, "map should render something");

        // Find the current room cell from the RenderMap and verify it's on screen.
        let current_room = rm.rooms.iter().find(|r| r.is_current).expect("should have a current room");
        let pos = cell_to_screen(current_room.cell, state.zoom, state.scroll, area);
        assert!(pos.is_some(), "current room should be on screen with scroll adjusted");
        let (cx, cy) = pos.unwrap();

        // The current room reverses only its interior: the top-left corner (a border
        // cell) is NOT reversed, while an interior cell (one in, one down) IS.
        let border = buf.cell((cx, cy)).expect("border cell should exist");
        assert!(
            !border.modifier.contains(Modifier::REVERSED),
            "current room border cell must NOT be REVERSED; got modifier={:?}",
            border.modifier
        );
        let interior = buf.cell((cx + 1, cy + 1)).expect("interior cell should exist");
        assert!(
            interior.modifier.contains(Modifier::REVERSED),
            "current room interior cell should have REVERSED modifier; got modifier={:?}",
            interior.modifier
        );
    }

    #[test]
    fn connector_drawn_between_two_rooms() {
        let mut m = Mapper::default();
        m.observe(1, "Start", None);
        m.observe(2, "East", Some(Direction::E));
        let rm = render(&m.graph);
        let state = AppState::default(); // Boxes zoom, scroll (0,0)
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // Count box-drawing and rounded corner characters.
        let box_drawing: usize = buf
            .content
            .iter()
            .filter(|c| {
                matches!(
                    c.symbol(),
                    "─" | "│" | "╭" | "╮" | "╰" | "╯" | "┏" | "┓" | "┗" | "┛" | "━" | "┃"
                )
            })
            .count();
        // The room boxes themselves use these chars too — we just need more than zero.
        assert!(box_drawing > 0, "should have box-drawing chars from rooms or connectors");
    }

    #[test]
    fn notes_marker_drawn() {
        use mapper::graph::MapGraph;

        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.set_pos(1, (0, 0));
        g.set_notes(1, "some notes".into());
        let rm = render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // Notes marker '●' should appear somewhere in the buffer.
        let has_notes_marker = buf.content.iter().any(|c| c.symbol() == "●");
        assert!(has_notes_marker, "notes marker '●' should be drawn for a room with notes");
    }

    #[test]
    fn recenter_keeps_cell_on_screen() {
        // After recenter_on(cell, pane_w, pane_h), cell_to_screen must return
        // Some((x,y)) that lies inside the area — proving the map is not blank.
        let area = Rect::new(40, 0, 40, 24); // right-half pane, x offset 40
        let cell = (0_i32, 0_i32);

        let mut state = AppState::default(); // Boxes zoom
        state.recenter_on(cell, area.width, area.height);

        let result = cell_to_screen(cell, state.zoom, state.scroll, area);
        assert!(
            result.is_some(),
            "cell_to_screen should return Some after recenter_on; scroll={:?}",
            state.scroll
        );
        let (sx, sy) = result.unwrap();
        assert!(
            sx >= area.x && sx < area.right() && sy >= area.y && sy < area.bottom(),
            "screen position ({sx},{sy}) should be inside area {area:?}"
        );
    }

    #[test]
    fn overview_zoom_draws_single_glyph() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        let rm = render(&m.graph);
        let mut state = AppState::default();
        state.zoom = Zoom::Overview;
        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let has_block = buf.content.iter().any(|c| c.symbol() == "■");
        assert!(has_block, "overview zoom should draw '■' glyph");
    }

    #[test]
    fn room_box_shows_label_at_boxes_zoom() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "West of House".into());
        g.set_pos(1, (0, 0));
        let rm = render(&g);
        let state = AppState::default(); // Boxes zoom
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // The box is 14 wide × 4 tall at (0,0). Inner area is cols 1..13, rows 1..3.
        // Label "West of House" truncated to 12 chars = "West of Hous"
        // Should find 'W', 'e', 's', 't' at row 1, cols 1..4
        let row1_chars: String = (1u16..=12).map(|x| {
            buf.cell((x, 1)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' ')
        }).collect();
        assert!(row1_chars.contains("West"), "label row should contain 'West'; got '{row1_chars}'");
    }

    #[test]
    fn room_box_shows_id() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(7, "Hall".into());
        g.set_pos(7, (0, 0));
        let rm = render(&g);
        let mut state = AppState::default(); // Boxes zoom
        state.show_room_numbers = true; // enable to see #id on row 3
        let area = Rect::new(0, 0, 60, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // The unique id "#7" is drawn centered on row 3 (moved off row 2).
        let row3: String = (1u16..=9)
            .map(|x| buf.cell((x, 3)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(row3.contains("#7"), "row 3 should show the room id '#7'; got '{row3}'");
    }

    /// SQ-1257 Phase 3: a room the story has renamed three times over (Lost Pig's gnome tunnels
    /// are the specimen) draws its CURRENT name with a superscript "³" beside it, never dropping
    /// the marker to fit. Falsify by reverting `draw_box_room`'s marker branch back to the plain
    /// `wrap_two`/`center` pair and this fails on the `contains('³')` assertion.
    #[test]
    fn room_box_shows_the_superscript_alias_count_beside_the_current_name() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(1, "B".into());
        g.upsert_room(1, "C".into());
        g.upsert_room(1, "Cave".into()); // current label "Cave"; aliases A, B, C (3 of them)
        assert_eq!(g.room(1).unwrap().aliases.len(), 3, "sanity: the fixture really has 3 aliases");
        g.set_pos(1, (0, 0));
        let rm = render(&g);
        let state = AppState::default(); // Boxes zoom
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // Interior rows 1-2, cols 1-9 (box width 11, interior width 9).
        let interior: String = (1u16..=2)
            .flat_map(|y| (1u16..=9).map(move |x| (x, y)))
            .map(|(x, y)| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(interior.contains('³'), "the superscript count '³' must appear: {interior:?}");
        assert!(interior.contains("Cave"), "the current name still appears: {interior:?}");
        assert!(interior.contains("Cave³"), "the marker sits right after the name: {interior:?}");
    }

    /// The marker is its own themeable element (`map.room_alias_marker`), not a reuse of the
    /// room's base colour — so styling it in `style.toml` must actually change what is drawn.
    #[test]
    fn room_box_alias_marker_uses_its_own_style_selector() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(1, "Cave".into()); // one alias: "A"
        g.set_pos(1, (0, 0));
        let rm = render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let (_, _, marker_fg) = (1u16..=9)
            .flat_map(|x| (1u16..=2).map(move |y| (x, y)))
            .find_map(|(x, y)| buf.cell((x, y)).filter(|c| c.symbol() == "¹").map(|c| (x, y, c.fg)))
            .expect("the alias marker glyph '¹' must be drawn somewhere in the box");
        let marker_selector_fg = state.colors.theme.get("map.room_alias_marker").style.fg;
        assert_eq!(
            Some(marker_fg), marker_selector_fg,
            "the drawn marker's colour must come from the map.room_alias_marker selector"
        );
        let room_selector_fg = state.colors.theme.get("map.room").style.fg;
        assert_ne!(
            marker_selector_fg, room_selector_fg,
            "sanity: the two selectors resolve to different defaults, so this test can tell them apart"
        );
    }

    /// A SELECTED room's box is painted with the selection background; the alias marker must sit
    /// on that same ground rather than punching a default-background hole through it (reported
    /// on Lost Pig's Gnome Room, 2026-09-03). Only the marker's colour is its own.
    #[test]
    fn room_box_alias_marker_keeps_the_selected_rooms_background() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(1, "Cave".into()); // one alias: "A"
        g.set_pos(1, (0, 0));
        let rm = render(&g);
        let mut state = AppState::default();
        state.selected_room = Some(1);
        // A selection background this test can see (the default theme's `map.room_selected`
        // sets none), and a marker colour that differs from the selected text's.
        state.colors.theme = theme_with_overrides(&[
            ("map.room_selected", Style::new().fg(Color::Black).bg(Color::Yellow)),
            // The marker selector carries a background of its own — as it does under any
            // theme whose `muted` role sets one — which is exactly what used to punch
            // through the selection.
            ("map.room_alias_marker", Style::new().fg(Color::Red).bg(Color::Black)),
        ]);
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let (mx, my) = (1u16..=9)
            .flat_map(|x| (1u16..=2).map(move |y| (x, y)))
            .find(|&(x, y)| buf.cell((x, y)).is_some_and(|c| c.symbol() == "¹"))
            .expect("the alias marker glyph '¹' must be drawn somewhere in the box");
        let marker = buf.cell((mx, my)).unwrap();
        // The cell just before the marker holds the last letter of the name, drawn in the room's
        // (selected) style — the marker must share its background and modifiers.
        let name = buf.cell((mx - 1, my)).unwrap();
        assert_eq!(name.symbol(), "e", "sanity: the marker rides right after 'Cave'");
        assert_eq!(marker.bg, name.bg, "the marker keeps the selected room's background");
        assert_eq!(marker.modifier, name.modifier, "…and its modifiers");
        assert_eq!(marker.bg, Color::Yellow, "…which is the selection background");
        assert_eq!(marker.fg, Color::Red, "while its colour stays the marker selector's own");
    }

    // ── SQ-1261: `?` random-exit stubs on the room box ──────────────────────────

    /// A `?` mark with no recorded destinations draws a bare `?` on the border, at the same
    /// centre-bottom cell a real south exit's arrowhead would take; nothing beyond it.
    #[test]
    fn room_box_draws_a_bare_random_stub_with_no_destinations() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Windy Cave".into());
        g.set_pos(1, (0, 0));
        g.mark_random_exit(1, mapper::direction::Direction::S);
        let rm = render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // Box is 11×5 at (0,0): south's border-centre cell is (5, 4).
        let sym = buf.cell((5u16, 4u16)).map(|c| c.symbol().to_string()).unwrap_or_default();
        assert_eq!(sym, "?", "bare `?`, no destinations recorded");
        // Nothing beyond the box — no connector, no second glyph past the border.
        assert!(
            buf.cell((5u16, 5u16)).map(|c| c.symbol()).unwrap_or(" ").trim().is_empty(),
            "no connector drawn beyond the stub"
        );
    }

    /// A `?` mark with recorded destinations draws the superscript count in the same slot
    /// instead of the bare `?` — `╰───²───╯` on the south border for two recorded destinations.
    /// Falsify by reverting `random_stub_marker` to always return `"?"` and this fails.
    #[test]
    fn room_box_draws_the_superscript_destination_count_on_the_stub() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Windy Cave".into());
        g.upsert_room(2, "A".into());
        g.upsert_room(3, "B".into());
        g.set_pos(1, (0, 0));
        g.mark_random_exit(1, mapper::direction::Direction::S);
        g.note_random_destination(1, mapper::direction::Direction::S, 2);
        g.note_random_destination(1, mapper::direction::Direction::S, 3);
        let rm = render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let bottom_row: String =
            (0u16..=10).map(|x| buf.cell((x, 4)).map(|c| c.symbol().to_string()).unwrap_or_default()).collect();
        assert_eq!(bottom_row, "╰────²────╯", "the south border carries the superscript count, no bare `?`");
    }

    /// A diagonal `?` mark lands at the box CORNER — the same cell a diagonal departure's
    /// arrowhead would take — overwriting the rounded corner glyph.
    #[test]
    fn room_box_draws_a_diagonal_random_stub_at_the_corner() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Windy Cave".into());
        g.set_pos(1, (0, 0));
        g.mark_random_exit(1, mapper::direction::Direction::SE);
        let rm = render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // Box is 11×5 at (0,0): the SE corner is (10, 4).
        let sym = buf.cell((10u16, 4u16)).map(|c| c.symbol().to_string()).unwrap_or_default();
        assert_eq!(sym, "?", "the diagonal stub overwrites the rounded corner glyph");
    }

    /// The stub is its own themeable element (`map.room_random_stub`), not a reuse of the room's
    /// base colour.
    #[test]
    fn room_box_random_stub_uses_its_own_style_selector() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Windy Cave".into());
        g.set_pos(1, (0, 0));
        g.mark_random_exit(1, mapper::direction::Direction::S);
        let rm = render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let stub_fg = buf.cell((5u16, 4u16)).and_then(|c| (c.symbol() == "?").then_some(c.fg));
        assert!(stub_fg.is_some(), "the stub glyph must be drawn");
        let stub_selector_fg = state.colors.theme.get("map.room_random_stub").style.fg;
        assert_eq!(
            stub_fg,
            stub_selector_fg,
            "the drawn stub's colour must come from the map.room_random_stub selector"
        );
        let room_selector_fg = state.colors.theme.get("map.room").style.fg;
        assert_ne!(
            stub_selector_fg, room_selector_fg,
            "sanity: the two selectors resolve to different defaults, so this test can tell them apart"
        );
    }

    /// A direction that carries BOTH a real edge and a stale random mark (a hand-edited or
    /// pre-upgrade map file — never produced by ordinary play) draws the real edge's line, never
    /// the stub — [`mapper::render::RenderRoom::random_stubs`] already filters this out, and this
    /// pins the drawn consequence.
    #[test]
    fn room_box_a_real_edge_wins_the_border_slot_over_a_stale_random_mark() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 1)); // south of room 1
        g.add_edge(1, mapper::direction::Direction::S, 2);
        g.mark_random_exit(1, mapper::direction::Direction::S); // stale/hand-edited
        let rm = render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let sym = buf.cell((5u16, 4u16)).map(|c| c.symbol().to_string()).unwrap_or_default();
        assert_ne!(sym, "?", "the real edge's arrowhead wins the slot, not the stub: got {sym:?}");
    }

    // connector_has_corner_glyph: removed — called build_connector_mask which is gone;
    // superseded by new tests in Task 4.

    // connector_has_arrowhead_at_dest: removed — arrowhead rendering is stubbed out in Task 1;
    // superseded by new tests in Task 4.

    // connector_is_contiguous_no_gaps: segment_screen_points unit portion removed (function gone);
    // full-render connector assertions superseded by new tests in Task 4.

    // ── Line-art connector tests (Task 5) ─────────────────────────────────────

    /// Box-drawing line-art glyphs a connector may render as.
    const LINE_GLYPHS: [&str; 11] =
        ["─", "│", "┌", "┐", "└", "┘", "├", "┤", "┬", "┴", "┼"];
    const ARROW_GLYPHS: [&str; 4] = ["▶", "◀", "▲", "▼"];

    fn is_line(sym: &str) -> bool {
        LINE_GLYPHS.contains(&sym)
    }

    #[test]
    fn connector_renders_line_art_glyphs() {
        // room1(0,0) →E→ room2(1,0): the connection must render as box-drawing line-art,
        // NOT a solid background ribbon.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "R1".into());
        g.upsert_room(2, "R2".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        let rm = mapper::render::render(&g);
        let state = AppState::default(); // Boxes zoom, scroll (0,0)
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // Some line-art glyph appears, and NO solid Cyan/Magenta background ribbon exists.
        let mut line_cells = 0;
        for y in 0..area.height {
            for x in 0..area.width {
                let c = buf.cell((x, y)).unwrap();
                assert_ne!(c.bg, Color::Cyan, "no solid Cyan ribbon at ({x},{y})");
                assert_ne!(c.bg, Color::Magenta, "no solid Magenta ribbon at ({x},{y})");
                if is_line(c.symbol()) {
                    line_cells += 1;
                }
            }
        }
        assert!(line_cells > 0, "connector must render box-drawing line-art");
    }

    /// R1 at (0,1) with a NE exit up-right to R2 at (1,0). Both cells are >= the
    /// bounds minimum, so the whole connector is on-screen (a room at a negative
    /// row would put the path above the viewport and render nothing).
    fn ne_graph() -> mapper::graph::MapGraph {
        let mut g = mapper::graph::MapGraph::new();
        g.upsert_room(1, "R1".into());
        g.upsert_room(2, "R2".into());
        g.set_pos(1, (0, 1));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::NE, 2);
        g
    }

    fn render_ne(diagonal_corners: bool) -> Buffer {
        let rm = mapper::render::render(&ne_graph());
        let mut state = AppState::default();
        state.symbols.diagonal_corners = diagonal_corners;
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        buf
    }

    fn count_diag_glyphs(buf: &Buffer, area: Rect) -> usize {
        let mut n = 0;
        for y in 0..area.height {
            for x in 0..area.width {
                let s = buf.cell((x, y)).unwrap().symbol();
                if matches!(s, "🮠" | "🮡" | "🮢" | "🮣") {
                    n += 1;
                }
            }
        }
        n
    }

    #[test]
    fn a_one_row_hop_draws_a_single_half_diagonal() {
        // SQ-0314. Found on Zork's #217 "South of House" <-> #89 "Behind House": a dangling
        // eastward line hung off #217's top corner.
        //
        // The connector handed off to LANE 1 of its channel, not lane 0 — higher lanes sit closer
        // to the next box, so only ONE row separated the corner from the run. The chain required a
        // full pair (two rows), bailed, and the arrival fell back to the orthogonal bridge: it came
        // into the corner sideways, leaving a stray one-cell `─` beside it. `DIAG_GUTTER` cannot
        // fix this — it sizes the gap, but the lane's position inside the gap is what bites.
        //
        // A step in the corner's OWN row crosses exactly one row, and joins the corner edge-to-edge:
        // 🮠's middle-left meets the corner's middle-right, and its upper-centre meets the bottom of
        // the cell holding the run.
        let g = AppState::default().symbols.path;
        let (cells, resume) =
            diagonal_chain((40, 41), (41, 40), Direction::NE, &g).expect("a one-row hop draws");
        assert_eq!(cells, vec![((41, 41), g.diag_ul)], "a single 🮠, in the corner's own row");
        assert_eq!(resume, (41, 40));

        // With spare columns and only step 0 to work with, the chain does NOT widen: `─` fill in
        // step 0 would sit between the corner and the first diagonal glyph, so the line would leave
        // the room horizontally and only then turn diagonal — the very thing the corner exit is
        // for. It stays one column wide and lets the caller bridge the rest along the target's row,
        // which is the channel lane it was heading for anyway.
        let (cells, resume) =
            diagonal_chain((40, 41), (43, 40), Direction::NE, &g).expect("draws");
        assert_eq!(cells, vec![((41, 41), g.diag_ul)], "still just 🮠 — no fill beside the corner");
        assert_eq!(resume.1, 40, "the row is always reached");
        assert_eq!(resume, (41, 40), "and the spare columns are left to the caller's bridge");

        // Level with the corner there is genuinely nothing diagonal to draw, and the caller keeps
        // its orthogonal geometry.
        assert!(diagonal_chain((40, 41), (42, 41), Direction::NE, &g).is_none(), "zero rows");
    }

    #[test]
    fn a_chain_absorbs_surplus_columns_and_lands_exactly_on_its_target() {
        // SQ-0314. Reported on Zork's #217 <-> #89: a dangling eastward line off #217's top corner.
        // The two rooms are diagonally adjacent (a pure diagonal), but the column gap was FIVE and
        // the row gap FOUR — the V channel carries lanes, so `channel_width` exceeded the
        // diagonal's floor. The chain covered 4x4 and the spare column was left over as a one-cell
        // horizontal run into the corner: the dangle.
        //
        // Both surpluses chain, because both fill glyphs attach where the half-diagonals hand off:
        //   * `─` attaches middle-left/middle-right, so it goes BETWEEN a step's two halves —
        //     `🮣─🮠` chains just like `🮣🮠`, at two columns per row instead of one (shallower).
        //   * `│` attaches upper-/lower-centre, so it goes AFTER a step's far glyph — one column
        //     per two rows (steeper). #143's SW arrival needs this: four rows to three columns.
        let g = AppState::default().symbols.path;
        let ratios = [
            (4, 4), (4, 5), (4, 8), (4, 11), // square, then progressively shallower
            (1, 1), (3, 4), (6, 7),
            (4, 3), (5, 2), (8, 3), (7, 1), // and progressively steeper
        ];
        for (rows, cols) in ratios {
            let anchor = (58, 31);
            let target = (anchor.0 - cols, anchor.1 + rows); // SW-ward, as the Zork case is
            let (cells, resume) =
                diagonal_chain(anchor, target, Direction::SW, &g).expect("draws");
            assert_eq!(resume, target, "{rows} rows x {cols} cols: nothing may be left over");

            // One far glyph per step, and the surplus on each axis spent as its own fill.
            let steps = rows.min(cols);
            let count = |ch: char| cells.iter().filter(|(_, c)| *c == ch).count();
            assert_eq!(count(g.diag_lr), steps as usize, "{rows}x{cols}: one far glyph per step");
            assert_eq!(count(g.ew), (cols - steps) as usize, "{rows}x{cols}: `─` fill");
            assert_eq!(count(g.ns), (rows - steps) as usize, "{rows}x{cols}: `│` fill");

            // Every cell distinct — a step must never stack two glyphs on one cell.
            let uniq: std::collections::HashSet<_> = cells.iter().map(|(p, _)| *p).collect();
            assert_eq!(uniq.len(), cells.len(), "{rows}x{cols}: {cells:?}");

            // And never `─` beside the corner: step 0's fill would leave the room horizontally.
            let inline = (anchor.0 - 1, anchor.1);
            assert_eq!(
                cells.iter().find(|(p, _)| *p == inline).map(|(_, c)| *c),
                Some(g.diag_lr),
                "{rows}x{cols}: the first cell out of the corner must be the diagonal itself",
            );
        }

        // Level with the corner on either axis: nothing diagonal to draw.
        assert!(diagonal_chain((58, 31), (56, 31), Direction::SW, &g).is_none(), "zero rows");
        assert!(diagonal_chain((58, 31), (58, 35), Direction::SW, &g).is_none(), "zero columns");
    }

    #[test]
    fn a_chain_starts_edge_to_edge_with_its_corner() {
        // Step 0 is a HALF pair: its near cell would be the corner itself, so it contributes only
        // its far glyph, in the corner's own row. That is what makes the chain touch the corner
        // rather than start diagonally offset from it with a visible seam.
        let g = AppState::default().symbols.path;
        for (dir, first) in [
            (Direction::NE, ((41, 40), g.diag_ul)),
            (Direction::NW, ((39, 40), g.diag_ur)),
            (Direction::SE, ((41, 40), g.diag_ll)),
            (Direction::SW, ((39, 40), g.diag_lr)),
        ] {
            let (sx, sy) = match dir {
                Direction::NE => (1, -1),
                Direction::NW => (-1, -1),
                Direction::SE => (1, 1),
                _ => (-1, 1),
            };
            let target = (40 + 4 * sx, 40 + 4 * sy);
            let (cells, _) = diagonal_chain((40, 40), target, dir, &g).expect("draws");
            assert_eq!(cells[0].0, (40 + sx, 40), "{dir:?}: step 0 sits in the corner's OWN row");
            assert_eq!(cells[0], first, "{dir:?}: and carries the far glyph");
            // n steps emit 2n+1 glyphs — the half pair plus n full ones.
            assert_eq!(cells.len(), 2 * 3 + 1, "{dir:?}: {cells:?}");
        }
    }

    #[test]
    fn no_two_connectors_share_a_corner_anchor() {
        use mapper::mapper::Mapper;
        // The realistic sequence that exposed the collision: a 3-command asymmetric diagonal
        // passage, with the layout engine choosing every position. Ledge's SW corner was claimed by
        // BOTH the arrival from Cave and the departure to Pit, and the two chains overwrote each
        // other. The departure keeps it; the arrival must have moved to a side doorway. (SQ-0314)
        let mut m = Mapper::default();
        m.observe_command(1, "Cave", "look");
        m.observe_command(2, "Ledge", "northeast");
        m.observe_command(3, "Pit", "southwest");
        let plan = mapper::route::route_lanes(&m.graph);
        let rm = mapper::render::render(&m.graph);
        let (cols, rows) = boxes_axes(&plan, rm.bounds);

        // Every corner-anchored endpoint, as (room, corner). No (room, corner) twice.
        let mut claims: Vec<(mapper::graph::RoomId, Direction)> = Vec::new();
        for c in &plan.connectors {
            if mapper::direction::is_diagonal(c.exit_dir) {
                claims.push((c.origin, c.exit_dir));
            }
            if let Some(d) = c.entry_corner {
                claims.push((c.dest, d));
            }
        }
        let mut seen = std::collections::HashSet::new();
        for claim in &claims {
            assert!(seen.insert(*claim), "two connectors claim {claim:?}");
        }

        // And the same in PIXELS, which is what actually collides on screen.
        let mut anchors: std::collections::HashMap<(i32, i32), usize> = std::collections::HashMap::new();
        for c in &plan.connectors {
            if let Some(plot) = plot_connector(c, &cols, &rows, None) {
                if mapper::direction::is_diagonal(c.exit_dir) {
                    *anchors.entry(plot.dep_anchor).or_default() += 1;
                }
                if c.entry_corner.is_some() {
                    *anchors.entry(plot.arr_anchor).or_default() += 1;
                }
            }
        }
        for (pt, n) in &anchors {
            assert_eq!(*n, 1, "{n} connectors anchor on the corner at {pt:?}");
        }
    }

    #[test]
    fn every_diagonal_direction_pair_actually_draws_a_diagonal() {
        // SQ-0314: sweep all 64 reciprocal direction pairs between two adjacent rooms. If EITHER
        // end of the connector is diagonal, the render must contain at least one half-diagonal —
        // the corner is the whole point of the feature, and a diagonal that quietly degrades into
        // an orthogonal dogleg is the bug this pins.
        //
        // Three separate faults each used to silence a slice of this matrix, and none of them were
        // visible from the adjacent NE case alone:
        //   * `direct_route` anchors both ends with `exit_point` and knows nothing of corners, so a
        //     cardinal-out/diagonal-back pair (E out, NW back) never got a corner route at all.
        //   * `pure_diagonal` only checked the ARRIVAL, so a cardinal exit took the pure branch
        //     with an empty chain and suppressed the arrival diagonal too.
        //   * a route that wraps around its destination uses a channel OUTSIDE the rooms' bounds,
        //     where the diagonal's gutter floor was silently dropped (see `span_over`).
        use Direction::*;
        let dirs = [N, S, E, W, NE, NW, SE, SW];
        let area = Rect::new(0, 0, 120, 40);
        let mut missing = Vec::new();
        for d1 in dirs {
            for d2 in dirs {
                let off = mapper::direction::grid_offset(d1).expect("compass dirs have an offset");
                // Away from the origin so no part of the route falls outside the render area.
                let p1 = (2i32, 2i32);
                let mut g = mapper::graph::MapGraph::new();
                g.upsert_room(1, "R1".into());
                g.upsert_room(2, "R2".into());
                g.set_pos(1, p1);
                g.set_pos(2, (p1.0 + off.0, p1.1 + off.1));
                g.add_edge(1, d1, 2);
                g.add_edge(2, d2, 1);
                let rm = mapper::render::render(&g);
                let mut state = AppState::default();
                state.symbols.diagonal_corners = true;
                let mut buf = Buffer::empty(area);
                render_map(&rm, &state, area, &mut buf);
                let drew = count_diag_glyphs(&buf, area) > 0;
                let wants = mapper::direction::is_diagonal(d1) || mapper::direction::is_diagonal(d2);
                if wants && !drew {
                    missing.push(format!("{d1:?}<->{d2:?}"));
                }
                // And the converse: a pair with no diagonal end must not sprout one.
                if !wants {
                    assert!(!drew, "{d1:?}<->{d2:?} has no diagonal end but drew a half-diagonal");
                }
            }
        }
        assert!(missing.is_empty(), "these pairs lost their diagonal: {missing:?}");
    }

    #[test]
    fn diagonal_corners_on_draws_an_unbroken_corner_to_corner_diagonal() {
        // SQ-0314: two diagonally-adjacent rooms render as ONE clean diagonal staircase from
        // R1's top-right corner to R2's bottom-left corner, with no orthogonal jog anywhere:
        //
        //    4|              ↙─────────╯
        //    5|             🮣🮠
        //    6|            🮣🮠
        //    7|           🮣🮠
        //    8|╭─────────↗🮠
        //
        // Each 🮣🮠 pair climbs one row and one column, and pairs overlap by a column, so the chain
        // steps up-right cleanly. Every step must be present: drawing only the first (the original
        // fixed-length stub) leaves the line stranded mid-gap needing a dogleg home.
        //
        // `n` steps emit `2n+1` glyphs, not `2n`: step 0 is a HALF pair sitting in the corner's own
        // row, whose single glyph joins the corner edge-to-edge.
        let area = Rect::new(0, 0, 80, 30);
        let buf = render_ne(true);
        assert_eq!(
            count_diag_glyphs(&buf, area),
            7,
            "corner to corner across the DIAG_GUTTER-sized gap: a half pair, then three full ones",
        );

        // Each pair is 🮣 immediately left of 🮠 on one row — the ascending order. And the pairs
        // step: the second sits exactly one row up and one column right of the first.
        let mut pairs: Vec<(u16, u16)> = Vec::new();
        for y in 0..area.height {
            for x in 0..area.width.saturating_sub(1) {
                if buf.cell((x, y)).unwrap().symbol() == "🮣"
                    && buf.cell((x + 1, y)).unwrap().symbol() == "🮠"
                {
                    pairs.push((x, y));
                }
            }
        }
        pairs.sort_by_key(|&(_, y)| std::cmp::Reverse(y));
        assert_eq!(pairs.len(), 3, "three ascending 🮣🮠 pairs, found {pairs:?}");
        for w in pairs.windows(2) {
            assert_eq!(
                (w[1].0, w[1].1),
                (w[0].0 + 1, w[0].1 - 1),
                "each pair steps one up and one right of the one below it: {pairs:?}",
            );
        }

        // And no orthogonal line-art leaks in between: a jog would show up as a corner glyph.
        for y in 0..area.height {
            for x in 0..area.width {
                let s = buf.cell((x, y)).unwrap().symbol();
                assert!(
                    !matches!(s, "└" | "┘" | "┌" | "┐" | "├" | "┤" | "┬" | "┴" | "┼"),
                    "a pure diagonal draws no orthogonal jog, found {s:?} at ({x},{y})",
                );
            }
        }
    }

    #[test]
    fn a_diagonal_widens_its_column_gap_to_match_a_busy_row_gap() {
        // SQ-0314: a diagonal carries NO lane, so nothing else asks its channels to be any wider
        // than MIN_GUTTER — `diag_corners` is how it declares the space it needs. The chain climbs
        // one column per row, so the column gap must keep up with the row gap or the diagonal runs
        // out of columns mid-climb and finishes on a dogleg.
        use mapper::route::RoutePlan;
        let mut plan = RoutePlan::default();
        plan.h_lanes.insert(0, 3); // a busy row channel: tall gap
        plan.diag_corners.insert((0, 0)); // a diagonal crosses corner V(0)/H(0)
        let (cols, rows) = boxes_axes(&plan, ((0, 0), (1, 1)));
        assert_eq!(
            cols.channel_span(0),
            rows.channel_span(0),
            "the diagonal's column gap tracks the row gap it must cross",
        );

        // Without the diagonal the same column channel stays at the bare minimum — so the widening
        // is genuinely the diagonal's doing, and costs nothing on maps that have none.
        let mut plain = RoutePlan::default();
        plain.h_lanes.insert(0, 3);
        let (plain_cols, _) = boxes_axes(&plain, ((0, 0), (1, 1)));
        assert_eq!(plain_cols.channel_span(0), MIN_GUTTER);
        assert!(cols.channel_span(0) > plain_cols.channel_span(0));
    }

    #[test]
    fn diagonal_col_gap_never_narrows_a_channel() {
        // It is a floor, not an override: a quiet row gap must not shrink the column gap below the
        // minimum that keeps adjacent boxes from touching.
        for row_gap in 0..8 {
            assert!(diagonal_col_gap(row_gap) >= MIN_GUTTER, "row_gap={row_gap}");
        }
        assert_eq!(diagonal_col_gap(5), 5, "a square gap draws an unbroken chain");
    }

    #[test]
    fn diagonal_corners_off_still_departs_the_corner_orthogonally() {
        // The fallback contract (SQ-0314). The toggle picks GLYPHS, not geometry: leaving by the
        // corner now lives in the ROUTER, so it happens either way, and a terminal without Unicode
        // 13 Legacy Computing coverage still gets the corner slot — just walked orthogonally
        // instead of on a diagonal. That is the user's "shift the room slot for these to be in the
        // same place as our diagonal": same anchors, same corner, no exotic glyphs.
        let area = Rect::new(0, 0, 80, 30);
        let off = render_ne(false);
        assert_eq!(count_diag_glyphs(&off, area), 0, "no diagonal glyphs when the toggle is off");

        // The arrowhead still sits on the box CORNER, not a side midpoint.
        let plan = mapper::route::route_lanes(&ne_graph());
        let (cols, rows) = boxes_axes(&plan, ((0, 0), (1, 1)));
        let corner = corner_anchor(&cols, &rows, (0, 1), Direction::NE);
        let conn = &plan.connectors[0];
        let plot = plot_connector(conn, &cols, &rows, None).expect("the NE connector plots");
        assert_eq!(plot.dep_anchor, corner, "the fallback departs the same corner the diagonal does");
        assert!(plot.diag_cells.is_empty(), "and draws no half-diagonals");

        // The two renders genuinely differ, so the assertion above isn't vacuous.
        let on = render_ne(true);
        assert_ne!(
            cells_of(&on, area), cells_of(&off, area),
            "the toggle must actually change the render",
        );
    }

    /// Every cell symbol in `area`, row-major — a cheap whole-buffer fingerprint.
    fn cells_of(buf: &Buffer, area: Rect) -> Vec<String> {
        let mut v = Vec::new();
        for y in 0..area.height {
            for x in 0..area.width {
                v.push(buf.cell((x, y)).unwrap().symbol().to_string());
            }
        }
        v
    }

    #[test]
    fn connector_departs_origin_correct_side() {
        // room1(0,0) →E→ room2(1,0). The departure gutter just right of room1's box
        // (col 11) must carry a connector glyph (line-art or arrowhead), not a space and
        // not a room-box border.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "R1".into());
        g.upsert_room(2, "R2".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        let rm = mapper::render::render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // The departure anchor sits in the gutter column just right of room1 (col 11),
        // on the box's vertical-centre row (row 2). It must be a connector glyph.
        let sym = buf.cell((11, 2)).map(|c| c.symbol().to_string()).unwrap_or_default();
        assert!(
            is_line(&sym) || ARROW_GLYPHS.contains(&sym.as_str()),
            "departure cell (11,2) should be a connector glyph; got '{sym}'"
        );
    }

    #[test]
    fn arrowhead_at_departure_side() {
        // room1(0,0) →E→ room2(1,0): a filled ▶ arrowhead marks the outgoing east departure
        // EMBEDDED IN room1's right border. The box is 11 wide at x=0, so the right border is
        // column 10; the vertical-centre row is 2. The arrow replaces that border │ at (10,2),
        // drawn fg Cyan (no bg ribbon). The line then continues perpendicular out (col 11+).
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "R1".into());
        g.upsert_room(2, "R2".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        let rm = mapper::render::render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let cell = buf.cell((10, 2)).expect("arrow cell must exist");
        assert_eq!(cell.symbol(), "▶", "outgoing east arrow ▶ embedded in room1's right border");
        assert_eq!(cell.fg, Color::Cyan, "arrowhead fg should be Cyan; got {:?}", cell.fg);
        assert_ne!(cell.bg, Color::Cyan, "arrowhead must not sit on a solid ribbon");
        // No hollow arrowhead is ever drawn.
        let has_hollow = buf.content.iter().any(|c| matches!(c.symbol(), "▷" | "◁" | "△" | "▽"));
        assert!(!has_hollow, "hollow arrowheads must not appear");
    }

    #[test]
    fn reciprocal_draws_arrow_at_both_rooms() {
        // A(1) at (1,1) →N→ B(2) at (1,0) and back B →S→ A. The collapsed connector must
        // still render BOTH outgoing arrows: ▲ at A (north) and ▼ at B (south).
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 1));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1);
        let rm = mapper::render::render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let up = buf.content.iter().filter(|c| c.symbol() == "▲").count();
        let down = buf.content.iter().filter(|c| c.symbol() == "▼").count();
        assert_eq!(up, 1, "exactly one ▲ (A leaving north); got {up}");
        assert_eq!(down, 1, "exactly one ▼ (B leaving south); got {down}");
    }

    #[test]
    fn connectors_are_scroll_invariant() {
        // Connector geometry is identical at every scroll offset — scrolling is a pure
        // translate-and-clip in the non-uniform Boxes position tables. Render the same
        // map at two scrolls, map each line-art cell back to virtual space, assert equal.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.upsert_room(3, "C".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.set_pos(3, (2, 0));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::E, 3);
        let rm = mapper::render::render(&g);

        let area = Rect::new(0, 0, 120, 40);
        let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);

        let virtual_lines = |scroll: (i32, i32)| -> std::collections::BTreeSet<(i32, i32)> {
            let mut st = AppState::default();
            st.scroll = scroll;
            let mut buf = Buffer::empty(area);
            render_map(&rm, &st, area, &mut buf);
            // Inverse of the table-based offset used by render_map.
            let off = (cols.room_pixel(scroll.0), rows.room_pixel(scroll.1));
            let mut set = std::collections::BTreeSet::new();
            for y in 0..area.height {
                for x in 0..area.width {
                    let c = buf.cell((x, y)).unwrap();
                    if is_line(c.symbol()) || ARROW_GLYPHS.contains(&c.symbol()) {
                        set.insert((x as i32 + off.0, y as i32 + off.1));
                    }
                }
            }
            set
        };

        let a = virtual_lines((0, 0));
        let b = virtual_lines((-1, -1));
        assert!(!a.is_empty(), "expected some line-art cells");
        assert_eq!(a, b, "connector geometry must be scroll-independent in virtual space");
    }

    #[test]
    fn no_connector_glyph_inside_room_interior() {
        // 3 rooms A(0,0) B(1,0) C(2,0) with a direct A→C edge that passes B's column.
        // No connector line-art may land inside B's box interior.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.upsert_room(3, "C".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.set_pos(3, (2, 0));
        g.add_edge(1, Direction::E, 3);
        let rm = mapper::render::render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 120, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // B is at cell (1,0). Its virtual box top-left and size from the tables.
        let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);
        let bx = cols.room_pixel(1);
        let by = rows.room_pixel(0);
        for y in (by + 1)..(by + BOX_H - 1) {
            for x in (bx + 1)..(bx + BOX_W - 1) {
                if let Some(cell) = buf.cell((x as u16, y as u16)) {
                    assert!(
                        !is_line(cell.symbol()),
                        "connector line-art '{}' inside room B interior at ({x},{y})",
                        cell.symbol()
                    );
                }
            }
        }
    }

    #[test]
    fn boxes_axes_widen_busy_channels() {
        // A column-channel carrying 2 lanes must be wider than an empty one, and room
        // pixel-positions are cumulative (a later room sits further right when an earlier
        // gap is wide).
        use mapper::route::RoutePlan;
        let mut plan = RoutePlan::default();
        plan.v_lanes.insert(0, 2); // V[0] carries 2 lanes
        let (cols, _rows) = boxes_axes(&plan, ((0, 0), (2, 0)));
        let gap0 = cols.channel_span(0);
        let gap1 = cols.channel_span(1);
        assert!(gap0 > gap1, "a 2-lane channel must be wider than an empty one");
        assert!(cols.room_pixel(2) > cols.room_pixel(1));
    }

    /// Per-virtual-cell connector ownership: for each cell, the list of (connector_index,
    /// per-connector direction-bit mask) pairs from every connector that wrote that cell.
    /// Re-derives plotting per connector from the same `plot_connector` geometry the renderer
    /// uses, so a cell shared by ≥2 distinct connectors is detectable with full per-connector
    /// mask information (not just the OR, which masks corner-on-corner collisions).
    fn connector_ownership(
        plan: &mapper::route::RoutePlan,
        cols: &PosTable,
        rows: &PosTable,
    ) -> std::collections::HashMap<(i32, i32), Vec<(usize, u8)>> {
        let mut owners: std::collections::HashMap<(i32, i32), Vec<(usize, u8)>> =
            std::collections::HashMap::new();
        for (ci, conn) in plan.connectors.iter().enumerate() {
            // The orthogonal reading, matching overlap_stats: this helper audits the router's
            // ownership of cells, which the display toggle does not change.
            if let Some(plot) = plot_connector(conn, cols, rows, None) {
                for (c, mask) in &plot.cells {
                    owners.entry(*c).or_default().push((ci, *mask));
                }
            }
        }
        owners
    }

    /// Assert no virtual cell is written by ≥2 distinct connectors unless it is a TRUE
    /// perpendicular crossing: exactly 2 connectors, one contributing exactly E|W (horizontal
    /// straight) and the other exactly N|S (vertical straight). Corner-on-corner collisions
    /// (e.g. ┌ + ┘ or └ + ┐, which OR to all-four bits but are not traceable) and any cell
    /// with ≥3 connectors are rejected. Returns the number of clean ┼ crossings seen.
    fn assert_no_overlap(
        owners: &std::collections::HashMap<(i32, i32), Vec<(usize, u8)>>,
    ) -> usize {
        let ew = DIR_E | DIR_W;
        let ns = DIR_N | DIR_S;
        let mut crossings = 0;
        for (cell, entries) in owners {
            // Deduplicate by connector index (a connector may contribute the same cell twice
            // due to run deduplication; OR their masks together per connector).
            let mut per_conn: std::collections::BTreeMap<usize, u8> = std::collections::BTreeMap::new();
            for &(ci, mask) in entries {
                *per_conn.entry(ci).or_insert(0) |= mask;
            }
            if per_conn.len() >= 2 {
                let idx_list: Vec<usize> = per_conn.keys().copied().collect();
                let masks: Vec<u8> = per_conn.values().copied().collect();
                assert_eq!(
                    per_conn.len(), 2,
                    "cell {cell:?} shared by {n} connectors {idx_list:?} (masks={masks:?}) — \
                     only 2-connector perpendicular crossings are legal",
                    n = per_conn.len(),
                );
                // True perpendicular crossing: one connector carries E|W, the other N|S.
                // Sorted so the comparison is order-independent.
                let mut sorted_masks = masks.clone();
                sorted_masks.sort_unstable();
                let mut expected = [ns, ew];
                expected.sort_unstable();
                assert_eq!(
                    sorted_masks, expected,
                    "cell {cell:?} shared by connectors {idx_list:?} with masks {masks:?} is not \
                     a clean ┼ crossing — each contributor must be exactly E|W ({ew:#04b}) or \
                     N|S ({ns:#04b}); corner-on-corner turns are rejected",
                );
                crossings += 1;
            }
        }
        crossings
    }

    #[test]
    fn two_connectors_perpendicular_crossing_breaks_the_horizontal() {
        // A vertical connector (1 above 2) and a horizontal connector (3 left of 4) routed so
        // their long runs cross exactly once. The two passages do NOT meet there, so the cell must
        // not read as a junction: the vertical passes through and the horizontal breaks (SQ-0525).
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        for id in [1, 2, 3, 4] { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (2, 0));
        g.set_pos(2, (2, 2));
        g.set_pos(3, (0, 1));
        g.set_pos(4, (4, 1));
        g.add_edge(1, Direction::S, 2);
        g.add_edge(3, Direction::E, 4);
        let rm = mapper::render::render(&g);
        let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);
        let owners = connector_ownership(&rm.plan, &cols, &rows);
        let crossings = assert_no_overlap(&owners);
        assert_eq!(crossings, 1, "the two perpendicular connectors must cross at exactly one ┼");

        // The rendered glyph at the crossing is ┼.
        let cross_cell = owners.iter()
            .find(|(_, entries)| {
                let unique: std::collections::BTreeSet<usize> = entries.iter().map(|&(ci, _)| ci).collect();
                unique.len() >= 2
            })
            .map(|(k, _)| *k).unwrap();
        let area = Rect::new(0, 0, 160, 80);
        let mut buf = Buffer::empty(area);
        let mut st = AppState::default();
        st.zoom = Zoom::Boxes;
        st.scroll = rm.bounds.0;
        render_map(&rm, &st, area, &mut buf);
        let off = (cols.room_pixel(rm.bounds.0.0), rows.room_pixel(rm.bounds.0.1));
        let (sx, sy) = (cross_cell.0 - off.0, cross_cell.1 - off.1);
        assert_eq!(
            buf.cell((sx as u16, sy as u16)).unwrap().symbol(), "│",
            "the vertical run passes through the crossing; a ┼ would say the two passages meet",
        );
        // The horizontal's own cells either side are still drawn, so the break is a one-cell gap
        // rather than a truncated line.
        for dx in [-1i32, 1] {
            let sym = buf.cell(((sx + dx) as u16, sy as u16)).unwrap().symbol();
            assert_eq!(sym, "─", "the horizontal run resumes at dx={dx}, got {sym:?}");
        }
    }

    /// A vertical N/S reciprocal pair plus an extra A\u{2192}E\u{2192}B edge. This used to draw the extra as a
    /// merge stub whose polyline could collapse to 2 points, and a `< 3` guard once dropped that
    /// whole connector, arrow included. SQ-0522 retains exactly one connector per pair, so no
    /// same-pair edge reaches the merge-stub path at all and the collapse cannot recur.
    #[test]
    fn an_extra_same_pair_edge_never_becomes_a_collapsible_merge_stub() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 2)); // B directly south of A
        g.add_edge(1, Direction::S, 2);
        g.add_edge(2, Direction::N, 1); // vertical reciprocal \u{2014} outranks the lone E edge
        g.add_edge(1, Direction::E, 2); // the extra same-pair edge
        let (illegal, _) = render_overlap_stats(&g);
        assert_eq!(illegal, 0, "no illegal overlap");
        let rm = mapper::render::render(&g);
        assert_eq!(rm.plan.connectors.len(), 1, "one line for the pair");
        assert!(rm.plan.connectors.iter().all(|c| !c.merge), "and no merge stub to collapse");
        assert_eq!(rm.plan.connectors[0].exit_dir, Direction::S, "the reciprocal S/N pairing wins");
        assert_eq!(rm.plan.connectors[0].secondary_exit, vec![Direction::E], "E recorded, not drawn");
    }
    #[test]
    fn reciprocal_pairing_outranks_the_edge_order() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(77, "Forest".into());
        g.upsert_room(239, "Forest".into());
        g.set_pos(77, (0, 0));
        g.set_pos(239, (1, 0)); // adjacent, no gap
        g.add_edge(77, Direction::E, 239);
        g.add_edge(239, Direction::N, 77);
        g.add_edge(239, Direction::S, 77);
        g.add_edge(239, Direction::W, 77); // the geometric opposite, added LAST on purpose
        let rm = mapper::render::render(&g);
        assert_eq!(rm.plan.connectors.len(), 1, "one line for the pair");
        let c = &rm.plan.connectors[0];
        assert_eq!(
            (c.exit_dir, c.entry_dir),
            (Direction::E, Some(Direction::W)),
            "W held the line despite being added last, so the passage runs straight"
        );
        let mut secs = c.secondary_entry.clone();
        secs.sort_by_key(|d| format!("{d:?}"));
        assert_eq!(secs, vec![Direction::N, Direction::S], "the extras are recorded, not drawn");
    }
    #[test]
    fn four_passages_between_two_rooms_draw_one_line() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(77, "F".into());
        g.upsert_room(239, "G".into());
        g.set_pos(77, (0, 0));
        g.set_pos(239, (2, 0)); // east of 77, a gap of one cell
        g.add_edge(77, Direction::E, 239);
        g.add_edge(239, Direction::W, 77); // reciprocal → wins outright
        g.add_edge(239, Direction::N, 77);
        g.add_edge(239, Direction::S, 77);
        let (illegal, _) = render_overlap_stats(&g);
        assert_eq!(illegal, 0, "the single retained connector overlaps nothing");

        let rm = mapper::render::render(&g);
        assert_eq!(rm.plan.connectors.len(), 1, "one line for the pair, not a trunk plus stubs");
        assert!(!rm.plan.connectors[0].merge, "and it is a real connector, not a merge stub");
        assert_eq!(
            (rm.plan.connectors[0].exit_dir, rm.plan.connectors[0].entry_dir),
            (Direction::E, Some(Direction::W)),
            "the reciprocal E/W pairing outranks the one-way N and S edges"
        );

        let mut st = AppState::default();
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 120, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let count = |sym: &str| buf.content.iter().filter(|c| c.symbol() == sym).count();
        let tjuncts: usize = ["\u{251c}", "\u{2524}", "\u{252c}", "\u{2534}", "\u{253c}"].iter().map(|s| count(s)).sum();
        assert_eq!(tjuncts, 0, "no T-junction: there are no merge stubs left to join a trunk");
    }
    #[test]
    fn overlap_stats_clean_pair_is_zero() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "a".into());
        g.upsert_room(2, "b".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        let (illegal, _) = render_overlap_stats(&g);
        assert_eq!(illegal, 0);
    }

    #[test]
    fn cleanup_keeps_updown_protected_column_chain_aligned() {
        // cleanup_overlaps must keep protected COLUMN chains aligned through overlap resolution.
        //
        // Two hard-protected columns are guarded here:
        //  - the up/down lane 26→Down→27 / 27→Up→26 (`move_keeps_updown_sides`), and
        //  - (SQ-0216 reciprocal-compass lock) the reciprocal N/S pair 74 S->76 / 76 N->74.
        // An earlier build lacked the reciprocal-compass lock, so cleanup's greedy search would
        // shift the then-unprotected 76 one column west to cut crossings, breaking 74<->76. With the
        // reciprocal lock, 76 is column-locked and can only slide along 74's column, so both columns
        // now survive cleanup. We verify both, plus that all illegal overlaps clear (SQ-0222).
        use mapper::graph::MapGraph;
        use Direction::*;
        let mut g = MapGraph::new();
        for id in [25u16,26,27,74,75,76,77,78,79,80,81,136,143,180,193,201,203,239] {
            g.upsert_room(id, "r".into());
        }
        for (o, d, dst) in [
            (180,N,81),(81,W,180),(180,W,78),(78,N,143),(143,E,77),(77,S,74),(74,S,76),
            (76,W,78),(143,W,78),(78,S,76),(76,N,74),(74,E,25),(25,W,76),(74,W,79),(79,E,74),
            (25,E,26),(26,Up,25),(78,E,75),(77,E,239),(239,N,77),(77,Unknown,180),(180,S,80),
            (80,W,180),(80,E,79),(79,S,80),(79,N,81),(81,E,79),(80,S,76),(76,Unknown,180),
            (79,Unknown,180),(75,S,81),(75,W,78),(75,E,77),(239,S,77),(77,W,75),(75,N,143),
            (143,S,75),(26,Down,27),(27,N,136),(136,SW,27),(27,Up,26),(26,Unknown,180),
            (79,W,203),(203,W,193),(193,E,203),(203,E,79),(203,Up,201),(201,Down,203),
            (239,W,77),(81,N,75),(25,Down,26),
        ] { g.add_edge(o, d, dst); }
        mapper::layout::relayout_auto(&mut g);
        let p = |g: &MapGraph, id: u16| g.room(id).unwrap().pos.unwrap();
        assert_eq!(p(&g,26).0, p(&g,27).0, "precondition: relayout column-aligns the 26↔27 up/down lane");
        cleanup_overlaps(&mut g, 3, 40);
        assert_eq!(render_overlap_stats(&g).0, 0,
            "cleanup clears all illegal overlaps (SQ-0222 clean routing) while protecting the up/down column");
        assert_eq!(p(&g,26).0, p(&g,27).0,
            "27 must stay directly below 26 after cleanup (up/down-protected): 26={:?} 27={:?}", p(&g,26), p(&g,27));
        assert!(p(&g,27).1 > p(&g,26).1, "27 stays south of 26 in the up/down lane");
        assert_eq!(p(&g,74).0, p(&g,76).0,
            "76 must stay on 74's column after cleanup (reciprocal N/S locked): 74={:?} 76={:?}", p(&g,74), p(&g,76));
    }

    #[test]
    fn repair_puts_78_west_of_180_after_retidy() {
        // The full Retidy flow (relayout -> cleanup_overlaps -> repair_directional_hints) on A129
        // must leave 78 west of 180 (the 180->W->78 hint). With the length-priority router,
        // cleanup_overlaps now settles this ordering directly; repair_directional_hints stays in the
        // flow as the safety net that recovers the hint on inputs where a post-solve stage
        // sacrifices it.
        //
        // With SQ-0222 clean routing this dense fixture clears to zero illegal overlaps; repair must
        // keep it at zero (introduce none). It must also leave the hard-protected columns intact:
        // the 26↔27 up/down lane and (with the reciprocal-compass lock) the reciprocal N/S pair
        // 74<->76, which is now column-locked through the whole flow.
        use mapper::graph::MapGraph;
        use Direction::*;
        let mut g = MapGraph::new();
        for id in [25u16,26,27,74,75,76,77,78,79,80,81,136,143,180,193,201,203,239] {
            g.upsert_room(id, "r".into());
        }
        for (o, d, dst) in [
            (180,N,81),(81,W,180),(180,W,78),(78,N,143),(143,E,77),(77,S,74),(74,S,76),
            (76,W,78),(143,W,78),(78,S,76),(76,N,74),(74,E,25),(25,W,76),(74,W,79),(79,E,74),
            (25,E,26),(26,Up,25),(78,E,75),(77,E,239),(239,N,77),(77,Unknown,180),(180,S,80),
            (80,W,180),(80,E,79),(79,S,80),(79,N,81),(81,E,79),(80,S,76),(76,Unknown,180),
            (79,Unknown,180),(75,S,81),(75,W,78),(75,E,77),(239,S,77),(77,W,75),(75,N,143),
            (143,S,75),(26,Down,27),(27,N,136),(136,SW,27),(27,Up,26),(26,Unknown,180),
            (79,W,203),(203,W,193),(193,E,203),(203,E,79),(203,Up,201),(201,Down,203),
            (239,W,77),(81,N,75),(25,Down,26),
        ] { g.add_edge(o, d, dst); }
        mapper::layout::relayout_auto(&mut g);
        cleanup_overlaps(&mut g, 3, 40);
        repair_directional_hints(&mut g, 3, 40);
        let p = |g: &MapGraph, id: u16| g.room(id).unwrap().pos.unwrap();
        assert!(p(&g,78).0 < p(&g,180).0,
            "retidy must place 78 west of 180: 78={:?} 180={:?}", p(&g,78), p(&g,180));
        assert_eq!(render_overlap_stats(&g).0, 0,
            "repair keeps all illegal overlaps cleared (SQ-0222 clean routing)");
        assert_eq!(p(&g,26).0, p(&g,27).0,
            "repair must not knock the up/down-protected 26↔27 column off alignment: 26={:?} 27={:?}", p(&g,26), p(&g,27));
        assert_eq!(p(&g,74).0, p(&g,76).0,
            "repair must keep the reciprocal N/S pair 74<->76 column-locked: 74={:?} 76={:?}", p(&g,74), p(&g,76));
    }

    #[test]
    fn yielded_updown_pair_draws_a_lane_connector_not_a_stub() {
        // Up/Down pair placed far apart (yielded from a clean stack). Task 6 routes Up/Down as a
        // full lane connector regardless of adjacency, so the pair now draws a routed dotted line
        // (not the old draw_portal_connectors/portal_stub right-column stub), plus the up/down
        // glyphs on each room's border.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (3, 2)); // far from (0,-1) — yielded, not stacked
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(2, Direction::Down, 1);
        let rm = render(&g);
        let mut st = AppState::default();
        st.zoom = Zoom::Boxes;
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 100, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let count = |s: &str| buf.content.iter().filter(|c| c.symbol() == s).count();
        assert!(count("┊") + count("┄") >= 1, "the routed Up/Down connector body is dotted");
        assert!(count("↑") >= 1, "up glyph present on a border/icon");
        assert!(count("↓") >= 1, "down glyph present on a border/icon");
    }

    #[test]
    fn updown_connector_uses_portal_connector_color_not_connector() {
        // Regression (SQ-0216 review finding): up/down connectors must style their dotted body
        // AND their up/down border glyphs with `map.connector_portal`, not the generic
        // `map.connector` used by compass connectors. Build a map with BOTH an up/down pair
        // (far apart so the body draws a routed dotted line, not just a direct bridge) and an
        // unrelated compass connector, set `map.connector_portal` and `map.connector` to distinct
        // colors, and assert each connector kind picked up the right one.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (3, 2)); // far from (0,-1) — yielded, forces a routed body
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(2, Direction::Down, 1);

        g.upsert_room(3, "C".into());
        g.upsert_room(4, "D".into());
        g.set_pos(3, (0, 4));
        g.set_pos(4, (1, 4));
        g.add_edge(3, Direction::E, 4);

        let rm = render(&g);
        let mut st = AppState::default();
        st.zoom = Zoom::Boxes;
        st.scroll = rm.bounds.0;
        st.colors.theme = theme_with_overrides(&[
            ("map.connector", Style::new().fg(Color::Green)),
            ("map.connector_portal", Style::new().fg(Color::Rgb(10, 20, 30))),
        ]);
        let area = Rect::new(0, 0, 100, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);

        let portal_fg = st.colors.theme.get("map.connector_portal").style.fg;
        let connector_fg = st.colors.theme.get("map.connector").style.fg;
        assert_ne!(portal_fg, connector_fg, "test colors must be distinct to be meaningful");

        // Every dotted body glyph and up/down border glyph must use portal_connector's fg.
        let mut found_dotted = false;
        let mut found_updown_glyph = false;
        for cell in buf.content.iter() {
            match cell.symbol() {
                "┊" | "┄" => {
                    found_dotted = true;
                    assert_eq!(cell.fg, portal_fg.unwrap(), "dotted up/down body must use portal_connector fg");
                }
                "↑" | "↓" => {
                    found_updown_glyph = true;
                    assert_eq!(cell.fg, portal_fg.unwrap(), "up/down border glyph must use portal_connector fg");
                }
                _ => {}
            }
        }
        assert!(found_dotted, "expected at least one dotted up/down body glyph");
        assert!(found_updown_glyph, "expected at least one up/down border glyph");

        // The unrelated compass connector (C -E-> D) must still use `map.connector`.
        let mut found_compass_arrow = false;
        for cell in buf.content.iter() {
            if cell.symbol() == "▶" {
                found_compass_arrow = true;
                assert_eq!(cell.fg, connector_fg.unwrap(), "compass arrowhead must keep map.connector fg");
                assert_ne!(cell.fg, portal_fg.unwrap(), "compass arrowhead must not use portal_connector fg");
            }
        }
        assert!(found_compass_arrow, "expected the compass connector's ▶ arrowhead");
    }

    #[test]
    fn cleanup_guard_protects_a_stacked_updown_room() {
        // 2 is up from 1 and stacked directly in its column (both x=0), north of it. The guard must
        // forbid moving 2 south of 1, moving 1 north of 2, and dragging 2 off 1's column — while
        // still allowing 2 to move vertically within the column (staying north).
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "p".into());
        g.upsert_room(2, "u".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1)); // 2 north of 1, same column
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(2, Direction::Down, 1);
        assert!(!move_keeps_updown_sides(&g, 2, (0, 1)), "must forbid moving the up room SOUTH");
        assert!(!move_keeps_updown_sides(&g, 1, (0, -5)), "must forbid moving the partner NORTH of it");
        assert!(!move_keeps_updown_sides(&g, 2, (3, -2)), "must forbid dragging it off the column");
        assert!(move_keeps_updown_sides(&g, 2, (0, -3)), "moving it up within the column is fine");
    }

    #[test]
    fn reciprocal_axis_locks_classify_ns_ew_and_cross_rooms() {
        // reciprocal_axis_locks encodes the VPSC hard equality the greedy cleanup must respect:
        // a reciprocal N/S pair (share a column) → column-locked (x_locked, Y free); a reciprocal
        // E/W pair (share a row) → row-locked (y_locked, X free); a room in BOTH → fully pinned;
        // a non-reciprocal room → absent (unrestricted).
        use mapper::graph::MapGraph;
        use Direction::*;
        let mut g = MapGraph::new();
        for id in [1u16, 2, 3, 4, 5] {
            g.upsert_room(id, "r".into());
        }
        // 1<->2 reciprocal N/S (1 N->2, 2 S->1): column chain.
        g.add_edge(1, N, 2);
        g.add_edge(2, S, 1);
        // 3<->4 reciprocal E/W (3 E->4, 4 W->3): row chain.
        g.add_edge(3, E, 4);
        g.add_edge(4, W, 3);
        // 2 is ALSO reciprocal E/W with 3 (2 E->3, 3 W->2): 2 is a cross-chain (both) room.
        g.add_edge(2, E, 3);
        g.add_edge(3, W, 2);
        // 5 has only a one-way edge — no reciprocal, so no lock.
        g.add_edge(1, W, 5);

        let locks = reciprocal_axis_locks(&g);
        assert_eq!(locks.get(&1).copied(), Some((true, false)), "1 is N/S-reciprocal only → column-locked");
        assert_eq!(locks.get(&2).copied(), Some((true, true)), "2 is in an N/S AND an E/W chain → fully pinned");
        assert_eq!(locks.get(&3).copied(), Some((false, true)), "3 is E/W-reciprocal (with 2 and 4) only → row-locked");
        assert_eq!(locks.get(&4).copied(), Some((false, true)), "4 is E/W-reciprocal only → row-locked");
        assert_eq!(locks.get(&5).copied(), None, "5 has no reciprocal edge → unrestricted");
    }

    #[test]
    fn cleanup_locks_reciprocal_ns_pair_to_its_shared_column() {
        // SQ-0216: the greedy overlap cleanup must honor the reciprocal N/S hard equality the VPSC
        // solver enforces — a room in a reciprocal N/S chain is COLUMN-locked and may only slide
        // along its shared column, never off it. On this dense A129 fixture the reciprocal pair
        // 74<->76 (74 S->76, 76 N->74) shares a column after relayout; WITHOUT the lock, cleanup's
        // greedy search shifts the (then-unprotected) 76 one column WEST to cut crossings, breaking
        // the reciprocal (verified: 76 moves from x=-1 to x=-2). WITH the lock, 76 can only move in
        // Y, so it stays on 74's column. All illegal overlaps clear (SQ-0222 clean routing) with the
        // reciprocal pair still column-locked — the lock constrains 76 without leaving any residual.
        use mapper::graph::MapGraph;
        use Direction::*;
        let mut g = MapGraph::new();
        for id in [25u16,26,27,74,75,76,77,78,79,80,81,136,143,180,193,201,203,239] {
            g.upsert_room(id, "r".into());
        }
        for (o, d, dst) in [
            (180,N,81),(81,W,180),(180,W,78),(78,N,143),(143,E,77),(77,S,74),(74,S,76),
            (76,W,78),(143,W,78),(78,S,76),(76,N,74),(74,E,25),(25,W,76),(74,W,79),(79,E,74),
            (25,E,26),(26,Up,25),(78,E,75),(77,E,239),(239,N,77),(77,Unknown,180),(180,S,80),
            (80,W,180),(80,E,79),(79,S,80),(79,N,81),(81,E,79),(80,S,76),(76,Unknown,180),
            (79,Unknown,180),(75,S,81),(75,W,78),(75,E,77),(239,S,77),(77,W,75),(75,N,143),
            (143,S,75),(26,Down,27),(27,N,136),(136,SW,27),(27,Up,26),(26,Unknown,180),
            (79,W,203),(203,W,193),(193,E,203),(203,E,79),(203,Up,201),(201,Down,203),
            (239,W,77),(81,N,75),(25,Down,26),
        ] { g.add_edge(o, d, dst); }
        mapper::layout::relayout_auto(&mut g);
        let p = |g: &MapGraph, id: u16| g.room(id).unwrap().pos.unwrap();
        assert_eq!(p(&g,74).0, p(&g,76).0, "precondition: relayout column-aligns the 74<->76 reciprocal N/S pair");
        cleanup_overlaps(&mut g, 3, 40);
        assert_eq!(p(&g,74).0, p(&g,76).0,
            "76 must stay on 74's column after cleanup (reciprocal N/S locked): 74={:?} 76={:?}", p(&g,74), p(&g,76));
        assert!(p(&g,76).1 > p(&g,74).1, "76 stays south of 74 (only slid along the shared column, if at all)");
        assert_eq!(render_overlap_stats(&g).0, 0, "all illegal overlaps clear (SQ-0222) with the reciprocal N/S pair still locked");
    }

    #[test]
    fn cleanup_keeps_reciprocal_ew_chain_on_its_row() {
        // Row-lock analog to cleanup_locks_reciprocal_ns_pair_to_its_shared_column: a room in a
        // reciprocal E/W chain (shares a row) is ROW-locked — cleanup may change only its X, never
        // its Y. This asserts the symmetric guarantee on the same A129 fixture: the reciprocal E/W
        // chain 74<->79<->203<->193 stays on one shared row through overlap cleanup. (Up/Down
        // connectors are inherently vertical, so this dense fixture happens to apply no off-row
        // pressure here — the guard nonetheless pins the symmetric row-lock the code applies
        // identically to N/S; see reciprocal_axis_locks_classify_ns_ew_and_cross_rooms.)
        use mapper::graph::MapGraph;
        use Direction::*;
        let mut g = MapGraph::new();
        for id in [25u16,26,27,74,75,76,77,78,79,80,81,136,143,180,193,201,203,239] {
            g.upsert_room(id, "r".into());
        }
        for (o, d, dst) in [
            (180,N,81),(81,W,180),(180,W,78),(78,N,143),(143,E,77),(77,S,74),(74,S,76),
            (76,W,78),(143,W,78),(78,S,76),(76,N,74),(74,E,25),(25,W,76),(74,W,79),(79,E,74),
            (25,E,26),(26,Up,25),(78,E,75),(77,E,239),(239,N,77),(77,Unknown,180),(180,S,80),
            (80,W,180),(80,E,79),(79,S,80),(79,N,81),(81,E,79),(80,S,76),(76,Unknown,180),
            (79,Unknown,180),(75,S,81),(75,W,78),(75,E,77),(239,S,77),(77,W,75),(75,N,143),
            (143,S,75),(26,Down,27),(27,N,136),(136,SW,27),(27,Up,26),(26,Unknown,180),
            (79,W,203),(203,W,193),(193,E,203),(203,E,79),(203,Up,201),(201,Down,203),
            (239,W,77),(81,N,75),(25,Down,26),
        ] { g.add_edge(o, d, dst); }
        mapper::layout::relayout_auto(&mut g);
        let p = |g: &MapGraph, id: u16| g.room(id).unwrap().pos.unwrap();
        let ew_row = [74u16, 79, 203, 193];
        let r0 = p(&g, 74).1;
        assert!(ew_row.iter().all(|&id| p(&g, id).1 == r0),
            "precondition: relayout row-aligns the reciprocal E/W chain");
        cleanup_overlaps(&mut g, 3, 40);
        let r = p(&g, 74).1;
        for &id in &ew_row {
            assert_eq!(p(&g, id).1, r,
                "reciprocal E/W room {id} must stay on the shared row after cleanup: {:?}", p(&g, id));
        }
    }

    #[test]
    fn compact_collapses_empty_interior_column_and_row() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        for id in [1u16, 2, 3] { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (2, 0)); // empty column at x=1
        g.set_pos(3, (0, 2)); // empty row at y=1
        g.add_edge(1, Direction::E, 2);
        g.add_edge(1, Direction::S, 3);
        compact_empty_lines(&mut g);
        // Column 1 and row 1 collapse: 2 moves to (1,0), 3 moves to (0,1). Order preserved.
        assert_eq!(g.room(1).unwrap().pos, Some((0, 0)));
        assert_eq!(g.room(2).unwrap().pos, Some((1, 0)), "empty column collapsed");
        assert_eq!(g.room(3).unwrap().pos, Some((0, 1)), "empty row collapsed");
        // No empty interior line remains.
        let xs: std::collections::BTreeSet<i32> = g.rooms().map(|r| r.pos.unwrap().0).collect();
        let ys: std::collections::BTreeSet<i32> = g.rooms().map(|r| r.pos.unwrap().1).collect();
        assert!((*xs.iter().next().unwrap()..*xs.iter().next_back().unwrap()).all(|x| xs.contains(&x)));
        assert!((*ys.iter().next().unwrap()..*ys.iter().next_back().unwrap()).all(|y| ys.contains(&y)));
    }

    #[test]
    fn compact_preserves_directional_order_introducing_no_overlap() {
        // Full A129 Retidy flow plus compaction: 78 stays west of 180, the hard-protected 26↔27
        // up/down column stays aligned, compaction introduces no illegal overlap (SQ-0222 clean
        // routing keeps the cluster clear), and no fully-empty interior column/row is left behind.
        //
        // With the SQ-0216 reciprocal-compass lock, "76 stays under 74" holds again: 76 is
        // column-locked to its reciprocal N/S partner 74 through the whole flow. We assert that,
        // the still-guaranteed directional order (78 west of 180), the hard-protected 26↔27 up/down
        // column, and zero illegal overlaps.
        use mapper::graph::MapGraph;
        use Direction::*;
        let mut g = MapGraph::new();
        for id in [25u16,26,27,74,75,76,77,78,79,80,81,136,143,180,193,201,203,239] {
            g.upsert_room(id, "r".into());
        }
        for (o, d, dst) in [
            (180,N,81),(81,W,180),(180,W,78),(78,N,143),(143,E,77),(77,S,74),(74,S,76),
            (76,W,78),(143,W,78),(78,S,76),(76,N,74),(74,E,25),(25,W,76),(74,W,79),(79,E,74),
            (25,E,26),(26,Up,25),(78,E,75),(77,E,239),(239,N,77),(77,Unknown,180),(180,S,80),
            (80,W,180),(80,E,79),(79,S,80),(79,N,81),(81,E,79),(80,S,76),(76,Unknown,180),
            (79,Unknown,180),(75,S,81),(75,W,78),(75,E,77),(239,S,77),(77,W,75),(75,N,143),
            (143,S,75),(26,Down,27),(27,N,136),(136,SW,27),(27,Up,26),(26,Unknown,180),
            (79,W,203),(203,W,193),(193,E,203),(203,E,79),(203,Up,201),(201,Down,203),
            (239,W,77),(81,N,75),(25,Down,26),
        ] { g.add_edge(o, d, dst); }
        mapper::layout::relayout_auto(&mut g);
        cleanup_overlaps(&mut g, 3, 40);
        repair_directional_hints(&mut g, 3, 40);
        compact_empty_lines(&mut g);
        let p = |g: &MapGraph, id: u16| g.room(id).unwrap().pos.unwrap();
        assert!(p(&g,78).0 < p(&g,180).0, "78 stays west of 180 through compaction");
        assert_eq!(p(&g,26).0, p(&g,27).0, "26↔27 up/down column stays aligned through compaction");
        assert_eq!(p(&g,74).0, p(&g,76).0, "reciprocal N/S pair 74<->76 stays column-locked through compaction");
        assert_eq!(render_overlap_stats(&g).0, 0,
            "compaction introduces no illegal overlap (SQ-0222 clean routing keeps the cluster clear)");
        // Compaction must leave only GUTTER lines — an empty interior column/row remains only when
        // collapsing it would create an illegal overlap (e.g. the column a long direct route runs up).
        // Any empty interior line that could still collapse cleanly is a compaction miss.
        let collapsible = |g: &MapGraph, is_x: bool, line: i32| -> bool {
            let mut t = g.clone();
            let before = render_overlap_stats(&t).0;
            let rooms: Vec<_> = t.rooms().map(|r| (r.id, r.pos.unwrap())).collect();
            for (id, pos) in rooms {
                let c = if is_x { pos.0 } else { pos.1 };
                if c > line {
                    t.set_pos(id, if is_x { (pos.0 - 1, pos.1) } else { (pos.0, pos.1 - 1) });
                }
            }
            render_overlap_stats(&t).0 <= before
        };
        let xs: std::collections::BTreeSet<i32> = g.rooms().map(|r| r.pos.unwrap().0).collect();
        let ys: std::collections::BTreeSet<i32> = g.rooms().map(|r| r.pos.unwrap().1).collect();
        for (is_x, set) in [(true, &xs), (false, &ys)] {
            let (min, max) = (*set.iter().next().unwrap(), *set.iter().next_back().unwrap());
            for line in (min + 1)..max {
                if !set.contains(&line) {
                    assert!(!collapsible(&g, is_x, line),
                        "empty interior {} {line} should have compacted (its collapse adds no overlap)",
                        if is_x { "column" } else { "row" });
                }
            }
        }
    }

    #[test]
    fn repair_directional_hints_is_deterministic() {
        use mapper::graph::MapGraph;
        use mapper::layout::relayout_auto;
        let build = || {
            let mut g = MapGraph::new();
            for id in [1u16, 2, 3, 4, 5] { g.upsert_room(id, "r".into()); }
            g.add_edge(1, Direction::E, 2);
            g.add_edge(2, Direction::N, 3);
            g.add_edge(3, Direction::W, 4);
            g.add_edge(4, Direction::S, 5);
            g.add_edge(5, Direction::E, 1);
            relayout_auto(&mut g);
            g
        };
        let mut g1 = build();
        let mut g2 = build();
        repair_directional_hints(&mut g1, 3, 40);
        repair_directional_hints(&mut g2, 3, 40);
        let p1: Vec<_> = g1.rooms().map(|r| (r.id, r.pos)).collect();
        let p2: Vec<_> = g2.rooms().map(|r| (r.id, r.pos)).collect();
        assert_eq!(p1, p2, "repair must be deterministic");
    }

    #[test]
    fn cleanup_clears_a129_illegal_overlaps() {
        // The real A129 graph: pure sort layout leaves an illegal corner overlap; the
        // router-measured cleanup must move rooms until zero illegal overlaps remain.
        use mapper::graph::MapGraph;
        use mapper::layout::relayout_auto;
        let mut g = MapGraph::new();
        for (id, name) in [
            (74, "Clearing"), (75, "Forest Path"), (77, "Forest"), (78, "Forest"),
            (79, "Behind House"), (80, "South of House"), (81, "North of House"),
            (143, "Clearing"), (180, "West of House"), (239, "Forest"),
        ] { g.upsert_room(id, name.into()); }
        for (o, d, dst) in [
            (180, Direction::N, 81), (81, Direction::W, 180), (180, Direction::S, 80),
            (80, Direction::E, 79), (79, Direction::N, 81), (81, Direction::E, 79),
            (79, Direction::S, 80), (80, Direction::W, 180), (180, Direction::W, 78),
            (78, Direction::N, 143), (143, Direction::S, 75), (75, Direction::N, 143),
            (143, Direction::W, 78), (143, Direction::E, 77), (77, Direction::S, 74),
            (74, Direction::N, 77), (77, Direction::E, 239), (239, Direction::N, 77),
            (239, Direction::S, 77),
        ] { g.add_edge(o, d, dst); }
        relayout_auto(&mut g);
        cleanup_overlaps(&mut g, 3, 40);
        let (illegal, _) = render_overlap_stats(&g);
        assert_eq!(illegal, 0, "cleanup must clear all illegal overlaps on A129");
        // rooms still distinct cells
        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: std::collections::BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "no room overlap after cleanup");
    }

    #[test]
    fn cleanup_is_deterministic() {
        use mapper::graph::MapGraph;
        use mapper::layout::relayout_auto;
        let build = || {
            let mut g = MapGraph::new();
            for id in [1u16, 2, 3, 4, 5] { g.upsert_room(id, "r".into()); }
            g.add_edge(1, Direction::E, 2);
            g.add_edge(2, Direction::N, 3);
            g.add_edge(3, Direction::W, 4);
            g.add_edge(4, Direction::S, 5);
            g.add_edge(5, Direction::E, 1);
            relayout_auto(&mut g);
            g
        };
        let mut g1 = build();
        let mut g2 = build();
        cleanup_overlaps(&mut g1, 3, 40);
        cleanup_overlaps(&mut g2, 3, 40);
        let p1: Vec<_> = g1.rooms().map(|r| (r.id, r.pos)).collect();
        let p2: Vec<_> = g2.rooms().map(|r| (r.id, r.pos)).collect();
        assert_eq!(p1, p2, "cleanup must be deterministic");
    }

    #[test]
    fn multi_lane_in_one_channel_resolves_per_segment() {
        // Regression for the CRITICAL bug: a connector with TWO runs in the SAME channel on
        // DIFFERENT lanes must map each run's points to its OWN lane, resolved by the segment
        // whose extent contains the point — not by a per-channel-index lookup that overwrites
        // and collapses both runs onto one lane (which drew two connectors overlapping).
        use mapper::route::{Channel, LaneSeg};
        let plan = mapper::route::RoutePlan::default();
        let (cols, rows) = boxes_axes(&plan, ((0, 0), (1, 0)));
        // Two V(0) runs at different y-extents on different lanes.
        let segs = vec![
            LaneSeg { channel: Channel::V(0), lane: 0, start: 1, end: 3 },
            LaneSeg { channel: Channel::V(0), lane: 1, start: 5, end: 7 },
        ];
        let p_lane0 = lane_pixel((1, 2), &cols, &rows, &segs); // odd x=1 → V(0), y=2 ∈ [1,3]
        let p_lane1 = lane_pixel((1, 6), &cols, &rows, &segs); // odd x=1 → V(0), y=6 ∈ [5,7]
        assert_ne!(
            p_lane0.0, p_lane1.0,
            "two runs in one channel on different lanes must map to different columns; \
             a per-channel-index map would collapse them (both {:?})",
            p_lane0.0,
        );
        assert_eq!(p_lane1.0 - p_lane0.0, LANE_SPACING, "lane 1 sits one LANE_SPACING beyond lane 0");
    }

    #[test]
    fn box_name_wraps_centered_and_id_on_row3() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(7, "Rocky Ledge".into());
        g.set_pos(7, (0, 0));
        let rm = render(&g);
        let mut state = AppState::default(); // Boxes, align off
        state.show_room_numbers = true; // enable to verify #id on row 3
        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        let row = |y: u16| -> String {
            (0..11u16).map(|x| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default()).collect()
        };
        // Name word-wraps across rows 1 and 2.
        assert!(row(1).contains("Rocky"), "row 1 has the first word: '{}'", row(1));
        assert!(row(2).contains("Ledge"), "row 2 has the second word: '{}'", row(2));
        // #id is on row 3 (moved off row 2).
        assert!(row(3).contains("#7"), "row 3 shows the id: '{}'", row(3));
        assert!(!row(2).contains("#7"), "id is no longer on row 2: '{}'", row(2));
        // Centered: a leading pad space after the left border on the name + id rows.
        assert!(row(1).starts_with("│ "), "name centered (leading pad): '{}'", row(1));
        assert!(row(3).starts_with("│ "), "id centered (leading pad): '{}'", row(3));
    }

    #[test]
    fn alignment_overlay_off_by_default_then_shows_code() {
        use mapper::graph::MapGraph;
        use mapper::layout::relayout_auto;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1); // reciprocal → row chain
        relayout_auto(&mut g);
        let rm = mapper::render::render(&g);
        let area = Rect::new(0, 0, 160, 60);
        let render_buf = |show: bool| {
            let mut st = AppState::default();
            st.zoom = Zoom::Boxes;
            st.scroll = rm.bounds.0;
            st.show_alignment = show;
            st.show_room_numbers = true; // alignment codes ride the #id row
            let mut buf = Buffer::empty(area);
            render_map(&rm, &st, area, &mut buf);
            buf
        };
        let off = render_buf(false);
        let on = render_buf(true);
        assert_ne!(format!("{off:?}"), format!("{on:?}"), "overlay changes the buffer when on");
        // an 'R' appears somewhere only when on
        let has_r = |b: &Buffer| (0..area.width).any(|x| (0..area.height).any(|y|
            b.cell((x, y)).map(|c| c.symbol() == "R").unwrap_or(false)));
        assert!(!has_r(&off));
        assert!(has_r(&on), "row-chain code R appears when overlay on");
    }

    #[test]
    fn portal_glyphs_map_directions() {
        let s = crate::symbols::SymbolSet::default();
        let g = |d| arrow_for_direction(d, &s.arrows, &s.portal);
        assert_eq!(g(Direction::Up), '↑');
        assert_eq!(g(Direction::Down), '↓');
        assert_eq!(g(Direction::In), '◉');
        assert_eq!(g(Direction::Out), '◎');
        assert_eq!(g(Direction::Unknown), '?');
    }

    #[test]
    fn portal_icons_render_in_room_slots() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Attic".into());
        g.upsert_room(3, "Cellar".into());
        g.upsert_room(4, "Vault".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1)); // placed portal targets (route_all skips unplaced dests)
        g.set_pos(3, (0, 1));
        g.set_pos(4, (1, 0));
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(1, Direction::Down, 3);
        g.add_edge(1, Direction::In, 4);
        let rm = render(&g);
        let mut state = AppState::default(); // Boxes, scroll (0,0), labels off
        state.show_room_numbers = true; // right-column layout requires numbers shown
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        let sym = |x: u16, y: u16| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default();
        // Box of room 1 is at screen (0,0); right interior column is col 9 (BOX_W-2).
        // In (non-spatial) still gets the mid-slot interior icon.
        assert_eq!(sym(9, 2), "◉", "in icon in middle-right interior (row 2)");
        // Up/Down no longer draw an interior icon — they show their glyph on the connector's
        // border anchor instead (top/bottom centre of the box, col 5 = BOX_W/2).
        assert_ne!(sym(9, 1), "↑", "up icon leaves the upper-right interior");
        assert_ne!(sym(9, 3), "↓", "down icon leaves the lower-right interior");
        assert_eq!(sym(5, 0), "↑", "up glyph on the top border centre");
        assert_eq!(sym(5, 4), "↓", "down glyph on the bottom border centre");
    }

    #[test]
    fn portal_mid_slot_in_beats_out() {
        // Room 1 has BOTH an In portal (→ room 2) and an Out portal (→ room 3).
        // The mid-slot precedence rule is In ▸ Out ▸ Unknown, so the middle-right interior
        // cell (col 9, row 2 of a box at screen (0,0)) must show ◉ (In), not ◎ (Out).
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Inner".into());
        g.upsert_room(3, "Outer".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0)); // placed so route_all processes this edge
        g.set_pos(3, (2, 0)); // placed so route_all processes this edge
        g.add_edge(1, Direction::In, 2);
        g.add_edge(1, Direction::Out, 3);
        let rm = render(&g);
        let mut state = AppState::default(); // Boxes zoom, scroll (0,0), labels off
        state.show_room_numbers = true; // right-column layout requires numbers shown
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        let sym = |x: u16, y: u16| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default();
        // col 9 = BOX_W - 2 = 11 - 2 = 9; row 2 = mid slot
        assert_eq!(sym(9, 2), "◉", "In beats Out in mid slot: expected ◉, got '{}'", sym(9, 2));
    }

    #[test]
    fn portal_icon_up_no_longer_shifts_notes_marker() {
        // The Up icon used to claim the same interior cell as the notes marker (upper-right
        // corner), forcing the marker to shift one cell left. Now Up shows its glyph on the
        // connector's border anchor instead, so the interior cell is free and the notes marker
        // stays in its normal (unshifted) spot.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Attic".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.set_notes(1, "stuff".into());
        g.add_edge(1, Direction::Up, 2);
        let rm = render(&g);
        let mut state = AppState::default();
        state.show_room_numbers = true; // right-column layout requires numbers shown
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        let sym = |x: u16, y: u16| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default();
        assert_eq!(sym(9, 1), "●", "notes marker stays put; the interior up icon is gone");
        assert_eq!(sym(5, 0), "↑", "up glyph now appears on the top border centre");
    }

    #[test]
    fn portal_view_moves_icons_to_border_and_floats_destinations() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Mid".into());    // portal owner
        g.upsert_room(2, "Attic".into());  // up target
        g.upsert_room(3, "Cellar".into()); // down target
        g.set_pos(1, (0, 1));
        g.set_pos(2, (0, 0));
        g.set_pos(3, (0, 2));
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(1, Direction::Down, 3);
        let rm = render(&g);
        let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);
        let mut st = AppState::default();
        st.show_portal_labels = true;
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 120, 60);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let off = (cols.room_pixel(rm.bounds.0 .0), rows.room_pixel(rm.bounds.0 .1));
        let bx = cols.room_pixel(0) - off.0;
        let by = rows.room_pixel(1) - off.1;
        let sym = |x: i32, y: i32| buf.cell((x as u16, y as u16)).map(|c| c.symbol().to_string()).unwrap_or_default();
        // Icons sit on the border (top/bottom centre), not the interior right column.
        assert_eq!(sym(bx + BOX_W / 2, by), "↑", "up icon on the top border centre");
        assert_eq!(sym(bx + BOX_W / 2, by + BOX_H - 1), "↓", "down icon on the bottom border centre");
        // Destinations float above / below the box.
        let above: String = (0..area.width).map(|x| sym(x as i32, by - 1)).collect();
        let below: String = (0..area.width).map(|x| sym(x as i32, by + BOX_H)).collect();
        assert!(above.contains("Attic"), "up destination floats above; got '{above}'");
        assert!(below.contains("Cellar"), "down destination floats below; got '{below}'");
        // The interior right-column icon is gone in portal view.
        assert_ne!(sym(bx + BOX_W - 2, by + 1), "↑", "icons leave the interior in portal view");
    }

    #[test]
    fn unknown_portal_draws_no_icon_or_name() {
        // An Unknown-direction edge is non-spatial (e.g. a death/respawn the game gave no direction
        // for), so it draws no portal icon and no destination name in either view.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "West of House".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::Unknown, 2);
        let rm = render(&g);
        let mut state = AppState::default();
        state.show_portal_labels = true;
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        let count = |s: &str| buf.content.iter().filter(|c| c.symbol() == s).count();
        assert_eq!(count("?"), 0, "an Unknown portal draws no ? icon");
        // No destination name to the right of room 1's box (row 2, the portal-label region).
        let sym = |x: u16, y: u16| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default();
        let right: String = ((BOX_W as u16)..40).map(|x| sym(x, 2)).collect();
        assert!(!right.contains("West"), "unknown portal shows no destination name; got '{right}'");
    }

    #[test]
    fn diagonal_edge_draws_corner_arrow() {
        // 1 →SW→ 2 (room 2 south-west of room 1): ↙ replaces room 1's bottom-left corner.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 0));
        g.set_pos(2, (0, 1)); // SW of room 1
        g.add_edge(1, Direction::SW, 2);
        let rm = render(&g);
        let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);
        let mut st = AppState::default();
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 120, 60);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let off = (cols.room_pixel(rm.bounds.0 .0), rows.room_pixel(rm.bounds.0 .1));
        let bx = cols.room_pixel(1) - off.0; // room 1 at col 1
        let by = rows.room_pixel(0) - off.1; // room 1 at row 0
        let sym = buf
            .cell((bx as u16, (by + BOX_H - 1) as u16))
            .map(|c| c.symbol().to_string())
            .unwrap_or_default();
        assert_eq!(sym, "↙", "SW edge draws ↙ at room 1's bottom-left corner");
    }

    #[test]
    fn reciprocal_diagonal_draws_corner_arrow_at_both_ends() {
        // 1 →SW→ 2 and 2 →NE→ 1 (true reciprocal): ↙ at room 1's bottom-left corner and
        // ↗ at room 2's top-right corner (the far end uses the back-edge direction).
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 0));
        g.set_pos(2, (0, 1)); // SW of room 1
        g.add_edge(1, Direction::SW, 2);
        g.add_edge(2, Direction::NE, 1); // reciprocal
        let rm = render(&g);
        let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);
        let mut st = AppState::default();
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 120, 60);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let off = (cols.room_pixel(rm.bounds.0 .0), rows.room_pixel(rm.bounds.0 .1));
        let sym = |x: i32, y: i32| buf.cell((x as u16, y as u16)).map(|c| c.symbol().to_string()).unwrap_or_default();
        // Origin (room 1): SW → bottom-left corner.
        let bx1 = cols.room_pixel(1) - off.0;
        let by1 = rows.room_pixel(0) - off.1;
        assert_eq!(sym(bx1, by1 + BOX_H - 1), "↙", "origin SW corner arrow");
        // Far end (room 2): NE back-edge → top-right corner.
        let bx2 = cols.room_pixel(0) - off.0;
        let by2 = rows.room_pixel(1) - off.1;
        assert_eq!(sym(bx2 + BOX_W - 1, by2), "↗", "far-end NE corner arrow");
    }

    #[test]
    fn portal_view_suppresses_connector_arrows() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        let rm = render(&g);
        let area = Rect::new(0, 0, 80, 30);
        let count_arrows = |show: bool| -> usize {
            let mut st = AppState::default();
            st.show_portal_labels = show;
            let mut buf = Buffer::empty(area);
            render_map(&rm, &st, area, &mut buf);
            buf.content.iter().filter(|c| matches!(c.symbol(), "▶" | "◀" | "▲" | "▼")).count()
        };
        assert!(count_arrows(false) > 0, "normal view draws connector arrowheads");
        assert_eq!(count_arrows(true), 0, "portal view suppresses connector arrowheads");
    }

    #[test]
    fn up_portal_draws_dotted_connector_when_no_compass_edge() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 1));
        g.set_pos(2, (0, 0)); // NW of room 1
        g.add_edge(1, Direction::Up, 2);
        let rm = render(&g);
        let mut st = AppState::default();
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 120, 60);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let has_dotted = buf.content.iter().any(|c| matches!(c.symbol(), "┊" | "┄"));
        assert!(has_dotted, "an Up portal with no compass edge draws a dotted connector");
    }

    /// A pair joined by BOTH a compass edge and a staircase draws ONE line, and priority picks
    /// which: N outranks Up (SQ-0522). SQ-0224 drew both, on separate trunks — two lines between
    /// two rooms that then had to cross each other. The staircase is not lost, it is simply not
    /// drawn; the room inspector's exit list names every direction with its destination.
    #[test]
    fn compass_and_updown_on_same_pair_draw_one_line_by_priority() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 1));
        g.set_pos(2, (1, 0)); // due north of room 1
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(1, Direction::N, 2); // a compass connector also joins the pair
        assert_eq!(render_overlap_stats(&g).0, 0, "the single retained connector overlaps nothing");

        let rm = render(&g);
        assert_eq!(rm.plan.connectors.len(), 1, "one line for the pair, whatever the directions");
        assert_eq!(rm.plan.connectors[0].exit_dir, Direction::N, "N outranks Up");
        assert_eq!(rm.plan.connectors[0].secondary_exit, vec![Direction::Up], "Up is recorded, not drawn");

        let mut st = AppState::default();
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 120, 60);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let dotted = buf.content.iter().filter(|c| matches!(c.symbol(), "\u{250a}" | "\u{2504}")).count();
        assert_eq!(dotted, 0, "no second, dotted line for the passage that lost");
        // SQ-0689 flips the second half of this pin: the collapsed staircase used to leave no
        // icon either ("an icon has no line to follow"), which made a real, known Up passage
        // invisible — Zork's Chasm. It now stamps its portal glyph beside the shared line's
        // anchor, ON the line it follows.
        let ups = buf.content.iter().filter(|c| c.symbol() == "\u{2191}").count();
        assert_eq!(ups, 1, "the collapsed staircase stamps its ↑ beside the shared line");
    }

    /// SQ-0689, the Zork1 Chasm shape exactly: the winning connector's origin is the OTHER room,
    /// so the collapsed staircase lands in `secondary_entry` and its marker must sit on the
    /// border of the room the staircase departs from — the connector's DESTINATION end.
    #[test]
    fn a_secondary_collapsed_at_the_entry_end_stamps_on_the_destination_room() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(112, "Chasm".into());
        g.upsert_room(136, "Passage".into());
        g.set_pos(112, (1, 0));
        g.set_pos(136, (1, 1)); // due south of the Chasm
        g.add_edge(136, Direction::N, 112); // the passage walks north into the Chasm…
        g.add_edge(112, Direction::Up, 136); // …and the way back is Up, which loses the pairing
        let rm = render(&g);
        assert_eq!(rm.plan.connectors.len(), 1, "one line for the pair");
        let c = &rm.plan.connectors[0];
        assert_eq!((c.origin, c.exit_dir), (136, Direction::N), "N wins the line");
        assert_eq!(c.secondary_entry, vec![Direction::Up], "Up collapses at the entry end");

        let mut st = AppState::default();
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 120, 60);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let ups: Vec<(u16, u16)> = (0..area.height)
            .flat_map(|y| (0..area.width).map(move |x| (x, y)))
            .filter(|&(x, y)| buf.cell((x, y)).is_some_and(|c| c.symbol() == "\u{2191}"))
            .collect();
        assert_eq!(ups.len(), 1, "exactly one ↑ marker");
        // The Chasm's box is the NORTH one; its bottom border row is where the marker belongs —
        // the room Up departs from, not the passage the connector happens to originate at.
        let corner_row = (0..area.height)
            .find(|&y| (0..area.width).any(|x| buf.cell((x, y)).is_some_and(|c| c.symbol() == "\u{2570}")))
            .expect("a box corner");
        assert_eq!(ups[0].1, corner_row, "the ↑ sits on the Chasm's bottom border");
    }

    /// SQ-0688: arrows are only ever OUTGOING — an arrow on a room border is that room's own
    /// exit. A one-way diagonal used to stamp a side-derived arrival arrow on the destination's
    /// corner (`▶` on a SW corner), which read as an exit east out of a room with no such exit.
    /// Now only the departure corner wears an arrow; the line ends bare on the destination.
    #[test]
    fn a_one_way_diagonal_wears_an_arrow_only_at_its_departure_corner() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(183, "Passage".into());
        g.upsert_room(170, "Canyon".into());
        g.set_pos(183, (0, 1));
        g.set_pos(170, (1, 0)); // NE of the passage
        g.add_edge(183, Direction::NE, 170); // one-way: nothing comes back
        let rm = render(&g);

        let mut st = AppState::default();
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 120, 60);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let count = |s: &str| buf.content.iter().filter(|c| c.symbol() == s).count();
        assert_eq!(count("\u{2197}"), 1, "exactly one ↗ — the departure corner");
        assert_eq!(count("\u{25b6}"), 0, "no cardinal ▶ pretending the canyon has an east exit");
    }
    #[test]
    fn interlayer_badge_dest_label_appears_in_portal_view() {
        // Build a two-layer graph: Hall (1) and Study (2) on MAIN_LAYER, linked by a Down
        // portal from Hall to Cellar (3). Cellar + Wine (4) are peeled into a new layer.
        // Rendering MAIN_LAYER in portal view must show the destination layer name ("Cellar")
        // floating beside Hall's box — confirming inter-layer stubs render their dest_label.
        use mapper::graph::MapGraph;
        use mapper::layer::{move_region, planar_region, MoveTarget, MAIN_LAYER};
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Study".into());
        g.upsert_room(3, "Cellar".into());
        g.upsert_room(4, "Wine".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.set_pos(3, (0, 1));
        g.set_pos(4, (1, 1));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        g.add_edge(1, Direction::Down, 3);
        g.add_edge(3, Direction::Up, 1);
        g.add_edge(3, Direction::E, 4);
        g.add_edge(4, Direction::W, 3);
        let region = planar_region(&g, 3);
        move_region(&mut g, &region, MoveTarget::New).expect("cellar + wine must peel into a new layer");
        // render_layer builds the MAIN_LAYER sub-graph and appends inter-layer badge stubs.
        let rm = mapper::render::render_layer(&g, MAIN_LAYER);
        // At least one inter-layer badge stub with a dest_label must be present.
        assert!(
            rm.edges.iter().any(|e| e.is_stub && e.dest_label.as_deref().is_some()),
            "render_layer must include inter-layer badge stubs with dest_label"
        );
        let mut st = AppState::default();
        st.show_portal_labels = true; // portal view floats destination names outside boxes
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 120, 60);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        // The dest_label is "<room> · <layer>": e.g. "Cellar · Cellar". The layer name
        // assigned by peel_region is the first-room label. Assert that "Cellar" appears
        // somewhere in the buffer (both the room name and layer name contain it).
        let all_text: String = buf.content.iter().map(|c| c.symbol()).collect();
        assert!(
            all_text.contains("Cellar"),
            "inter-layer badge dest_label must appear in portal view; buffer text: '{}'",
            all_text.chars().filter(|c| !c.is_whitespace()).collect::<String>()
        );
    }

    #[test]
    fn layer_portal_room_gets_double_line_outline() {
        // A room with an outgoing portal to another layer renders with a double-line box
        // outline (╔═╗ … ║) instead of the rounded one, so cross-layer exits read at a glance.
        use mapper::graph::MapGraph;
        use mapper::layer::{move_region, planar_region, MoveTarget, MAIN_LAYER};
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Cellar".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 1));
        g.add_edge(1, Direction::Down, 2);
        g.add_edge(2, Direction::Up, 1);
        let region = planar_region(&g, 2);
        move_region(&mut g, &region, MoveTarget::New).expect("peel cellar into its own layer");
        let rm = mapper::render::render_layer(&g, MAIN_LAYER);
        assert!(
            rm.rooms.iter().find(|r| r.id == 1).unwrap().has_layer_portal,
            "Hall owns the outgoing cross-layer portal"
        );
        let mut st = AppState::default(); // Boxes zoom
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let all_text: String = buf.content.iter().map(|c| c.symbol()).collect();
        assert!(
            all_text.contains('╔') && all_text.contains('║'),
            "the layer-portal room must render with a double-line outline"
        );
    }

    #[test]
    fn path_and_portal_use_symbol_set() {
        // Two rooms connected N-S: glyph_for should produce the NS path char at the connector.
        // Also: a room with notes should show the portal.marker glyph.
        use mapper::graph::MapGraph;
        use crate::symbols::SymbolSet;
        use crate::config::SymbolConfig;

        // --- Path glyph test: two horizontally-connected rooms produce EW path segments ---
        let mut g = MapGraph::new();
        g.upsert_room(1, "R1".into());
        g.upsert_room(2, "R2".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        let rm = mapper::render::render(&g);

        let mut state = AppState::default();
        state.scroll = (0, 0);
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // The EW connector between the two rooms should have '─' (light path) somewhere
        let has_ew = buf.content.iter().any(|c| c.symbol() == "─");
        assert!(has_ew, "default light path: EW connector must use '─'");

        // With heavy preset, EW should be '━'
        let mut cfg = SymbolConfig::default();
        cfg.path_style = "heavy".into();
        state.symbols = SymbolSet::resolve(&cfg);
        let mut buf2 = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf2);
        let has_heavy_ew = buf2.content.iter().any(|c| c.symbol() == "━");
        assert!(has_heavy_ew, "heavy path preset: EW connector must use '━'");

        // --- Portal marker test: a room with notes shows portal.marker ---
        let mut g2 = MapGraph::new();
        g2.upsert_room(10, "A".into());
        g2.set_pos(10, (0, 0));
        g2.set_notes(10, "some notes".into());
        let rm2 = mapper::render::render(&g2);
        state.symbols = SymbolSet::default();
        let mut buf3 = Buffer::empty(area);
        render_map(&rm2, &state, area, &mut buf3);
        let has_marker = buf3.content.iter().any(|c| c.symbol() == "●");
        assert!(has_marker, "default portal.marker '●' must appear for room with notes");
    }

    #[test]
    fn arrow_uses_symbol_set() {
        // room1(0,0) →E→ room2(1,0): with default symbols the departure arrow is '▶';
        // with arrow_set = "line" it becomes '→'.
        use mapper::graph::MapGraph;
        use crate::symbols::SymbolSet;
        use crate::config::SymbolConfig;

        let mut g = MapGraph::new();
        g.upsert_room(1, "R1".into());
        g.upsert_room(2, "R2".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        let rm = mapper::render::render(&g);

        // Default: '▶' at the departure arrow cell (10, 2)
        let mut state = AppState::default();
        state.scroll = (0, 0);
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        assert_eq!(
            buf.cell((10, 2)).map(|c| c.symbol()),
            Some("▶"),
            "default symbols: east departure arrow must be '▶'"
        );

        // Line preset: '→' at the same cell
        let mut cfg = SymbolConfig::default();
        cfg.arrow_set = "line".into();
        state.symbols = SymbolSet::resolve(&cfg);
        let mut buf2 = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf2);
        assert_eq!(
            buf2.cell((10, 2)).map(|c| c.symbol()),
            Some("→"),
            "line preset: east departure arrow must be '→'"
        );
    }

    #[test]
    fn room_outline_uses_symbol_set() {
        // Default symbols: a normal (non-current, non-portal) room at cell (0,0) with
        // scroll (0,0) and Boxes zoom renders its top-left corner as '╭'.
        use mapper::graph::MapGraph;
        use crate::symbols::SymbolSet;
        use crate::config::SymbolConfig;

        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.set_pos(1, (0, 0));
        // Room 1 is not current, not a portal room.
        let rm = mapper::render::render(&g);

        // --- Default symbols: expect '╭' at (0,0) ---
        let mut state = AppState::default(); // SymbolSet::default() inside
        state.scroll = (0, 0);
        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        assert_eq!(
            buf.cell((0, 0)).map(|c| c.symbol()),
            Some("╭"),
            "default symbols must render normal room top-left as rounded corner"
        );

        // --- ASCII preset: expect '+' at (0,0) ---
        let mut cfg = SymbolConfig::default();
        cfg.box_style = "ascii".into();
        state.symbols = SymbolSet::resolve(&cfg);
        let mut buf2 = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf2);
        assert_eq!(
            buf2.cell((0, 0)).map(|c| c.symbol()),
            Some("+"),
            "ascii preset must render normal room top-left as '+'"
        );
    }

    // ── screen_to_cell / room_at_cell tests ───────────────────────────────────

    /// screen_to_cell is the exact inverse of cell_to_screen for placed rooms.
    #[test]
    fn screen_to_cell_inverts_cell_to_screen() {
        use crate::state::Zoom;
        use ratatui::layout::Rect;

        for zoom in [Zoom::Boxes, Zoom::Compact, Zoom::Overview] {
            let scroll = (2, 3);
            let area = Rect::new(5, 2, 100, 50);
            let cell = (4, 5);

            // Forward: cell → screen.
            let screen = cell_to_screen(cell, zoom, scroll, area).expect("should be in area");

            // Inverse: screen → cell.
            let back = screen_to_cell((screen.0 as i32, screen.1 as i32), zoom, scroll, area);
            assert_eq!(
                back, cell,
                "screen_to_cell should invert cell_to_screen for zoom {:?}: cell {:?} -> screen {:?} -> back {:?}",
                zoom, cell, screen, back
            );
        }
    }

    #[test]
    fn screen_to_cell_with_zero_scroll_and_origin_area() {
        use crate::state::Zoom;
        use ratatui::layout::Rect;

        let zoom = Zoom::Compact; // step = (12, 5)
        let scroll = (0, 0);
        let area = Rect::new(0, 0, 80, 40);

        // A click at screen (24, 10) should land in cell (2, 2).
        let cell = screen_to_cell((24, 10), zoom, scroll, area);
        assert_eq!(cell, (2, 2));

        // A click at (0, 0) lands at (0, 0).
        let cell0 = screen_to_cell((0, 0), zoom, scroll, area);
        assert_eq!(cell0, (0, 0));
    }

    /// room_at_cell finds a placed room and returns None for an empty cell.
    #[test]
    fn room_at_cell_finds_placed_room() {
        use mapper::graph::MapGraph;
        use mapper::layer::MAIN_LAYER;

        let mut g = MapGraph::new();
        g.upsert_room(1, "Start".into());
        g.upsert_room(2, "North".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));

        // Room 1 is at (0,0).
        assert_eq!(room_at_cell(&g, MAIN_LAYER, (0, 0)), Some(1));
        // Room 2 is at (0,-1).
        assert_eq!(room_at_cell(&g, MAIN_LAYER, (0, -1)), Some(2));
        // (1, 0) has no room.
        assert_eq!(room_at_cell(&g, MAIN_LAYER, (1, 0)), None);
        // (0, 1) has no room.
        assert_eq!(room_at_cell(&g, MAIN_LAYER, (0, 1)), None);
    }

    /// room_screen_rects returns non-empty rects within the area, and hit-testing
    /// a click at each rect's centre finds the correct room.
    #[test]
    fn room_screen_rects_basic_hit_test() {
        use crate::state::{AppState, Zoom};
        use mapper::graph::MapGraph;
        use mapper::render::render_layer;
        use ratatui::layout::Rect;

        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (2, 0));
        g.add_edge(1, mapper::direction::Direction::E, 2);

        let mut state = AppState::default();
        state.zoom = Zoom::Compact;
        state.scroll = (0, 0);

        let area = Rect::new(0, 0, 80, 40);
        let rm = render_layer(&g, mapper::layer::MAIN_LAYER);
        let rects = room_screen_rects(&rm, &state, area);

        // Both rooms must appear.
        assert_eq!(rects.len(), 2, "both rooms must have screen rects");

        // Every rect must be fully within the area.
        for (_, r) in &rects {
            assert!(r.x >= area.x, "rect left must be within area");
            assert!(r.y >= area.y, "rect top must be within area");
            assert!(r.right() <= area.right(), "rect right must be within area");
            assert!(r.bottom() <= area.bottom(), "rect bottom must be within area");
            assert!(r.width > 0 && r.height > 0, "rect must have positive dimensions");
        }

        // Hit-testing: a click at each rect's centre must find that room.
        for (id, r) in &rects {
            let cx = r.x + r.width / 2;
            let cy = r.y + r.height / 2;
            let hit = rects.iter()
                .find(|(_, rect)| cx >= rect.x && cx < rect.right() && cy >= rect.y && cy < rect.bottom())
                .map(|(rid, _)| *rid);
            assert_eq!(hit, Some(*id), "click at centre of room {:?} rect must hit that room", id);
        }
    }

    // ── Item 1: char_pan shifts room screen rects ─────────────────────────────

    /// char_pan should shift room screen rects by the same offset so that
    /// mouse hit-testing remains accurate after a drag pan.
    #[test]
    fn char_pan_shifts_room_screen_rects() {
        use crate::state::{AppState, Zoom};
        use mapper::graph::MapGraph;
        use mapper::render::render_layer;
        use ratatui::layout::Rect;

        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.set_pos(1, (0, 0));
        let rm = render_layer(&g, mapper::layer::MAIN_LAYER);

        let area = Rect::new(0, 0, 80, 40);

        // Baseline: no char_pan.
        let mut state = AppState::default();
        state.zoom = Zoom::Compact;
        state.scroll = (0, 0);
        state.char_pan = (0, 0);
        let rects_base = room_screen_rects(&rm, &state, area);
        assert_eq!(rects_base.len(), 1);
        let (_, r0) = rects_base[0];

        // Apply char_pan = (5, 3).
        state.char_pan = (5, 3);
        let rects_shifted = room_screen_rects(&rm, &state, area);
        assert_eq!(rects_shifted.len(), 1);
        let (_, r1) = rects_shifted[0];

        assert_eq!(
            (r1.x as i32 - r0.x as i32, r1.y as i32 - r0.y as i32),
            (5, 3),
            "char_pan (5,3) should shift screen rect by exactly (5,3)"
        );
    }

    // ── Item 3: current+selected combined style ───────────────────────────────

    /// When a room is both current AND selected, room_style combines both states:
    /// it returns room_selected with REVERSED added (not just one or the other).
    #[test]
    fn room_style_current_and_selected_combines() {
        use mapper::render::RenderRoom;
        use crate::state::AppState;

        let room = RenderRoom {
            id: 1,
            cell: (0, 0),
            label: "Test".into(),
            is_current: true,
            has_layer_portal: false,
            self_loops: Vec::new(),
            has_notes: false,
            align_code: String::new(),
            alias_count: 0,
            random_stubs: Vec::new(),
        };

        let mut state = AppState::default();
        state.selected_room = Some(1); // room is both current AND selected

        let style = room_style(&room, &state);

        // Must have REVERSED (from the combined path) AND use the selected base.
        assert!(
            style.add_modifier.contains(Modifier::REVERSED),
            "current+selected room must have REVERSED modifier; got {:?}",
            style
        );
        // The base must NOT be room_current alone (which would be REVERSED on its own style).
        // It should be room_selected with REVERSED added.
        let expected = state.colors.theme.get("map.room_selected").style.add_modifier(Modifier::REVERSED);
        assert_eq!(style, expected, "current+selected must equal room_selected + REVERSED");
    }

    /// When a room is current but NOT selected, room_style returns room_current.
    #[test]
    fn room_style_current_only() {
        use mapper::render::RenderRoom;
        use crate::state::AppState;

        let room = RenderRoom {
            id: 2,
            cell: (0, 0),
            label: "Test".into(),
            is_current: true,
            has_layer_portal: false,
            self_loops: Vec::new(),
            has_notes: false,
            align_code: String::new(),
            alias_count: 0,
            random_stubs: Vec::new(),
        };

        let mut state = AppState::default();
        state.selected_room = Some(99); // different room selected

        let style = room_style(&room, &state);
        assert_eq!(style, state.colors.theme.get("map.room_current").style, "current-only room must use room_current style");
    }

    /// When a room is selected but NOT current, room_style returns room_selected.
    #[test]
    fn room_style_selected_only() {
        use mapper::render::RenderRoom;
        use crate::state::AppState;

        let room = RenderRoom {
            id: 3,
            cell: (0, 0),
            label: "Test".into(),
            is_current: false,
            has_layer_portal: false,
            self_loops: Vec::new(),
            has_notes: false,
            align_code: String::new(),
            alias_count: 0,
            random_stubs: Vec::new(),
        };

        let mut state = AppState::default();
        state.selected_room = Some(3);

        let style = room_style(&room, &state);
        assert_eq!(style, state.colors.theme.get("map.room_selected").style, "selected-only room must use room_selected style");
    }

    // ── Item 4: arrow color does not bleed selection bg ───────────────────────

    /// draw_connector_arrows must reset the cell background before applying the
    /// connector fg, so a selection-highlighted room border cell does not keep
    /// the selection bg color after the arrowhead is drawn (non-selected room case).
    #[test]
    fn arrow_style_resets_bg_for_non_selected_room() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::style::{Color, Style};
        use crate::colors::ColorScheme;

        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);

        // Pre-paint the cell with a selection bg color to simulate a selected room border.
        let selection_bg = Color::Yellow;
        if let Some(cell) = buf.cell_mut((5, 5)) {
            cell.set_style(Style::new().bg(selection_bg));
        }
        assert_eq!(buf.cell((5, 5)).unwrap().bg, selection_bg);

        // Room 10's arrow; selected_room is None (no selection) — bg must be reset.
        let arrowheads: Vec<Arrowhead> = vec![Arrowhead { at: (5, 5), glyph: ">".to_string(), distorted: false, is_portal: false, room: 10, shared: false, kind: EdgeKind::Reciprocal }];
        let colors = ColorScheme::terminal_default();
        draw_connector_arrows(&arrowheads, (0, 0), area, &mut buf, &colors, None, None);

        let after_bg = buf.cell((5, 5)).unwrap().bg;
        assert_ne!(
            after_bg, selection_bg,
            "arrow draw must reset selection bg; bg is still Yellow after arrow"
        );
    }

    /// draw_connector_arrows must paint the cell background with the selected room's bg color
    /// when the arrow belongs to the currently selected room.
    #[test]
    fn arrow_style_selected_room_gets_room_bg() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::style::{Color, Style};
        use crate::colors::ColorScheme;

        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);

        // Use a color scheme where room_selected has a distinct bg.
        let mut colors = ColorScheme::terminal_default();
        let selected_bg = Color::Cyan;
        // connector fg is Green so we can check it independently.
        colors.theme = theme_with_overrides(&[
            ("map.room_selected", Style::new().fg(Color::White).bg(selected_bg)),
            ("map.connector", Style::new().fg(Color::Green)),
        ]);

        // Arrow at (5, 5) belongs to room 7; room 7 is the selected room (not current).
        let arrowheads: Vec<Arrowhead> = vec![Arrowhead { at: (5, 5), glyph: ">".to_string(), distorted: false, is_portal: false, room: 7, shared: false, kind: EdgeKind::Reciprocal }];
        draw_connector_arrows(&arrowheads, (0, 0), area, &mut buf, &colors, Some(7), None);

        let cell = buf.cell((5, 5)).unwrap();
        assert_eq!(
            cell.bg, selected_bg,
            "selected-room arrow must have the room_selected bg color as background"
        );
        assert_eq!(
            cell.fg,
            Color::Green,
            "selected-room arrow glyph fg must be the connector color"
        );
    }

    /// When the arrow belongs to a room that is BOTH current AND selected, the arrow sits on
    /// the room's border. The border is not reverse-video (only the interior is), so the arrow
    /// background matches the border's plain bg = room_selected.BG.
    #[test]
    fn arrow_style_current_and_selected_uses_reversed_bg() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::style::{Color, Style};
        use crate::colors::ColorScheme;

        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);

        let mut colors = ColorScheme::terminal_default();
        // Distinct fg/bg so the reversed-swap is observable.
        colors.theme = theme_with_overrides(&[
            ("map.room_selected", Style::new().fg(Color::Magenta).bg(Color::Cyan)),
            ("map.connector", Style::new().fg(Color::Green)),
        ]);

        // Arrow at (5, 5) belongs to room 7; room 7 is BOTH selected AND current.
        let arrowheads: Vec<Arrowhead> = vec![Arrowhead { at: (5, 5), glyph: ">".to_string(), distorted: false, is_portal: false, room: 7, shared: false, kind: EdgeKind::Reciprocal }];
        draw_connector_arrows(&arrowheads, (0, 0), area, &mut buf, &colors, Some(7), Some(7));

        let cell = buf.cell((5, 5)).unwrap();
        assert_eq!(
            cell.bg,
            Color::Cyan,
            "current+selected arrow bg must use room_selected.bg (the non-reversed border bg)"
        );
        assert_eq!(cell.fg, Color::Green, "arrow glyph fg must still be the connector color");
    }

    /// When the arrow belongs to the current room that is NOT selected, the arrow sits on the
    /// room's border. Only the interior is reverse-video, so the border (and thus the arrow)
    /// keeps room_current's plain background.
    #[test]
    fn arrow_style_current_only_matches_reversed_room_current_bg() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::style::{Color, Modifier, Style};
        use crate::colors::ColorScheme;

        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);

        let mut colors = ColorScheme::terminal_default();
        // room_current carries REVERSED, but the border it sits on is drawn non-reversed;
        // give it a distinct plain bg so the border background is observable.
        colors.theme = theme_with_overrides(&[
            ("map.room_current", Style::new().add_modifier(Modifier::REVERSED).fg(Color::Blue).bg(Color::Yellow)),
            ("map.connector", Style::new().fg(Color::Green)),
        ]);

        // Arrow at (5, 5) belongs to room 7; room 7 is the current room, NOT selected.
        let arrowheads: Vec<Arrowhead> = vec![Arrowhead { at: (5, 5), glyph: ">".to_string(), distorted: false, is_portal: false, room: 7, shared: false, kind: EdgeKind::Reciprocal }];
        draw_connector_arrows(&arrowheads, (0, 0), area, &mut buf, &colors, None, Some(7));

        let cell = buf.cell((5, 5)).unwrap();
        assert_eq!(
            cell.bg,
            Color::Yellow,
            "current-only arrow bg must use room_current.bg (the non-reversed border bg)"
        );
        assert_eq!(cell.fg, Color::Green, "arrow glyph fg must still be the connector color");
    }

    /// draw_connector_arrows must NOT apply the selected room's bg to an arrow belonging
    /// to a different (non-selected) room, even when a selection is active.
    #[test]
    fn arrow_style_other_room_unaffected_by_selection() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::style::{Color, Style};
        use crate::colors::ColorScheme;

        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);

        let mut colors = ColorScheme::terminal_default();
        colors.theme = theme_with_overrides(&[
            ("map.room_selected", Style::new().fg(Color::White).bg(Color::Cyan)),
            ("map.connector", Style::new().fg(Color::Green)),
        ]);

        // Arrow belongs to room 5; selected room is 7 — different rooms.
        let arrowheads: Vec<Arrowhead> = vec![Arrowhead { at: (5, 5), glyph: ">".to_string(), distorted: false, is_portal: false, room: 5, shared: false, kind: EdgeKind::Reciprocal }];
        draw_connector_arrows(&arrowheads, (0, 0), area, &mut buf, &colors, Some(7), None);

        let cell = buf.cell((5, 5)).unwrap();
        assert_ne!(
            cell.bg,
            Color::Cyan,
            "arrow of a non-selected room must not get the selected room bg"
        );
    }

    // ── pulse_border_color ────────────────────────────────────────────────────

    /// At three-quarter period (sin = -1, f = 0) the result is the red endpoint.
    #[test]
    fn pulse_border_color_red_at_three_quarter_period() {
        use std::time::Duration;
        // Three-quarter period: sin = -1, f = 0 → pure red endpoint.
        let three_quarter = Duration::from_secs_f64(3.0 / (4.0 * PULSE_HZ));
        let color = pulse_border_color(three_quarter);
        assert_eq!(color, Color::Rgb(PULSE_RED.0, PULSE_RED.1, PULSE_RED.2),
            "at three-quarter period the border must be the red endpoint");
    }

    /// At quarter period (sin = 1, f = 1) the result is the green endpoint.
    #[test]
    fn pulse_border_color_green_at_quarter_period() {
        use std::time::Duration;
        // Quarter period: sin = 1, f = 1 → pure green endpoint.
        let quarter = Duration::from_secs_f64(1.0 / (4.0 * PULSE_HZ));
        let color = pulse_border_color(quarter);
        assert_eq!(color, Color::Rgb(PULSE_GREEN.0, PULSE_GREEN.1, PULSE_GREEN.2),
            "at quarter period the border must be the green endpoint");
    }

    /// The pulsing border smoke test: with a tidy_job active, the map border cell
    /// style differs from the idle border (which uses the normal focused_border color).
    #[test]
    fn tidy_job_active_border_color_differs_from_idle() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        use ratatui::widgets::{Block, Borders};
        use ratatui::prelude::Widget;
        use ratatui::style::Style;
        use std::time::Duration;
        use crate::state::AppState;

        let state = AppState::default();
        let normal_border_color = state.colors.theme.get("panel.border:active").style.fg.unwrap_or(Color::White);

        // At quarter period the pulse is the green endpoint.
        let quarter = Duration::from_secs_f64(1.0 / (4.0 * PULSE_HZ));
        let active_color = pulse_border_color(quarter);

        // The pulsed green color must differ from the normal idle color (Cyan).
        assert_ne!(normal_border_color, active_color,
            "pulsing border color at quarter period must differ from the normal border color");

        // Render smoke: draw a Block with each border style into a TestBackend and
        // verify the border cell fg differs between idle and active.
        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| {
            let area = f.area();
            let buf = f.buffer_mut();

            // Idle: normal border color.
            let idle_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(normal_border_color));
            idle_block.render(area, buf);
            let idle_cell_fg = buf.cell((0, 0)).map(|c| c.fg).unwrap_or(Color::Reset);

            // Active: pulsing color.
            let active_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(active_color));
            active_block.render(area, buf);
            let active_cell_fg = buf.cell((0, 0)).map(|c| c.fg).unwrap_or(Color::Reset);

            assert_ne!(idle_cell_fg, active_cell_fg,
                "rendered border cell fg must differ when tidy_job is active");
        }).unwrap();
    }

    // ── sound_pulse_color ──────────────────────────────────────────────────────

    #[test]
    fn sound_pulse_full_color_at_start() {
        let beep = Color::Rgb(255, 180, 40);
        let normal = Color::Rgb(0, 0, 0);
        let c = sound_pulse_color(beep, normal, std::time::Duration::from_millis(0));
        assert_eq!(c, Some(Color::Rgb(255, 180, 40)), "elapsed 0 => full beep color");
    }

    #[test]
    fn sound_pulse_fades_toward_normal_partway() {
        let beep = Color::Rgb(200, 0, 0);
        let normal = Color::Rgb(0, 0, 0);
        // Halfway through the window: roughly the midpoint between beep and normal.
        let c = sound_pulse_color(beep, normal, std::time::Duration::from_millis(SOUND_PULSE_MS / 2));
        match c {
            Some(Color::Rgb(r, _, _)) => assert!((90..=110).contains(&r), "expected ~100, got {r}"),
            other => panic!("expected an Rgb mid-fade color, got {other:?}"),
        }
    }

    #[test]
    fn sound_pulse_expires_after_window() {
        let beep = Color::Rgb(255, 180, 40);
        let normal = Color::Rgb(0, 0, 0);
        let c = sound_pulse_color(beep, normal, std::time::Duration::from_millis(SOUND_PULSE_MS));
        assert_eq!(c, None, "at/after the window the pulse is over");
    }

    #[test]
    fn sound_pulse_non_rgb_normal_fades_toward_dim_beep() {
        // When the border color is a named/terminal color (no RGB), fade toward a
        // dimmed copy of the beep color instead (spec fallback).
        let beep = Color::Rgb(200, 200, 200);
        let c = sound_pulse_color(beep, Color::Reset, std::time::Duration::from_millis(SOUND_PULSE_MS - 1));
        match c {
            Some(Color::Rgb(r, _, _)) => assert!(r < 200, "must fade below full beep, got {r}"),
            other => panic!("expected an Rgb color, got {other:?}"),
        }
    }

    // ── Fix 1: render_map_layered layer-strip suppression ─────────────────────

    /// Helper: build a two-layer graph (Hall on MAIN, Cellar peeled to a second layer).
    fn two_layer_graph() -> mapper::graph::MapGraph {
        use mapper::direction::Direction;
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Cellar".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 0));
        g.add_edge(1, Direction::Down, 2);
        g.add_edge(2, Direction::Up, 1);
        let region = mapper::layer::planar_region(&g, 2);
        mapper::layer::move_region(&mut g, &region, mapper::layer::MoveTarget::New).expect("peel cellar");
        g
    }

    /// With a border active (`map_border_style != None`) and 2+ layers,
    /// `render_map_layered` must NOT draw the in-content strip (no lost content row).
    /// The in-content strip uses REVERSED modifier on tab labels; with a border active,
    /// no REVERSED cells should appear in the content area row 0.
    #[test]
    fn render_map_layered_no_in_content_strip_when_border_present() {
        use crate::render::paneframe::BorderStyle;
        let g = two_layer_graph();
        let rm = mapper::render::render_layer(&g, mapper::layer::MAIN_LAYER);

        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);

        // State with a non-None border style.
        let mut state = AppState::default();
        state.colors.map_border_style = BorderStyle::Single;

        render_map_layered(&rm, &g, &state, area, &mut buf);

        // The strip would write REVERSED style to cells in row 0. With a border active,
        // the strip is suppressed so no REVERSED cells appear in row 0.
        // (render_map does not set REVERSED anywhere in the map content area.)
        let reversed_in_row0 = (area.x..area.right())
            .filter(|&x| {
                buf.cell((x, area.y))
                    .map(|c| c.modifier.contains(ratatui::style::Modifier::REVERSED))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(
            reversed_in_row0, 0,
            "with a non-None border, the in-content layer strip must NOT be drawn (no REVERSED cells in row 0)"
        );
    }

    /// With `map_border_style == None` and 2+ layers, `render_map_layered` MUST draw
    /// the in-content strip (fallback indicator for the borderless case).
    #[test]
    fn render_map_layered_draws_in_content_strip_when_no_border() {
        use crate::render::paneframe::BorderStyle;
        let g = two_layer_graph();
        let rm = mapper::render::render_layer(&g, mapper::layer::MAIN_LAYER);

        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);

        // State with None border style.
        let mut state = AppState::default();
        state.zoom = crate::state::Zoom::Boxes; // strip requires non-Overview
        state.colors.map_border_style = BorderStyle::None;

        render_map_layered(&rm, &g, &state, area, &mut buf);

        // SQ-0643: the strip's active-tab marker is now the themed
        // panel.tab:active selector (unified with the bordered variant's tab
        // strip), not a hardcoded REVERSED literal — so detect the strip by its
        // TEXT (a layer name) rather than a specific style bit.
        let row0: String = (area.x..area.right())
            .map(|x| buf.cell((x, area.y)).map(|c| c.symbol().to_owned()).unwrap_or_default())
            .collect();
        assert!(
            row0.contains("Main") || row0.contains("Layer"),
            "with BorderStyle::None, the in-content layer strip MUST be drawn (a layer name expected in row 0), got {row0:?}"
        );
    }

    /// SQ-0643: `draw_layer_strip`'s borderless variant used bare `Style::new()`
    /// / a hardcoded REVERSED modifier — no `style.toml` selector could reach
    /// it. It must now read `panel.tab`/`panel.tab:active` like the bordered
    /// variant does, so a user override actually changes what's drawn.
    #[test]
    fn draw_layer_strip_active_tab_follows_panel_tab_active_override() {
        use crate::render::paneframe::BorderStyle;
        let g = two_layer_graph();

        let scheme = crate::colors::GhosttyScheme::default();
        let parsed = crate::theme::toml_schema::parse(
            "[panel]\n\"tab:active\" = { fg = \"magenta\" }\n",
        ).unwrap();
        let mut state = AppState::default();
        state.colors.theme = crate::theme::resolve::resolve_theme(&scheme, &parsed);
        state.zoom = crate::state::Zoom::Boxes;
        state.colors.map_border_style = BorderStyle::None;

        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        draw_layer_strip(&g, &state, area, &mut buf);

        let want_fg = state.colors.theme.get("panel.tab:active").style.fg;
        assert_eq!(want_fg, Some(Color::Magenta), "guard: the override must actually change the resolved style");
        let has_magenta = (area.x..area.right())
            .any(|x| buf.cell((x, area.y)).is_some_and(|c| c.style().fg == Some(Color::Magenta)));
        assert!(has_magenta, "the active tab must render in the overridden panel.tab:active colour");
    }

    // ── SQ-0672: the maze tab marker ──────────────────────────────────────────

    #[test]
    fn layer_tab_title_carries_the_maze_marker_only_when_flagged() {
        let g = two_layer_graph();
        let cellar = g.layer_of(2);
        assert_eq!(layer_tab_title(&g, cellar), "Cellar(1)", "no marker while unflagged");

        let mut g2 = two_layer_graph();
        g2.set_layer_maze(cellar, true);
        assert_eq!(
            layer_tab_title(&g2, cellar), "Cellar ⌗(1)",
            "flagged as a maze: a trailing ⌗ marker after the name"
        );
        g2.set_layer_maze(cellar, false);
        assert_eq!(layer_tab_title(&g2, cellar), "Cellar(1)", "unflagging removes it again");
    }

    /// The in-content (borderless) tab strip must show the same `⌗` marker
    /// `layer_tab_title` produces — it is the single source both strips draw from.
    #[test]
    fn draw_layer_strip_shows_the_maze_marker_when_flagged() {
        use crate::render::paneframe::BorderStyle;
        let mut g = two_layer_graph();
        let cellar = g.layer_of(2);
        g.set_layer_maze(cellar, true);

        let mut state = AppState::default();
        state.zoom = crate::state::Zoom::Boxes;
        state.colors.map_border_style = BorderStyle::None;
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        draw_layer_strip(&g, &state, area, &mut buf);

        let row0: String = (area.x..area.right())
            .map(|x| buf.cell((x, area.y)).map(|c| c.symbol().to_owned()).unwrap_or_default())
            .collect();
        assert!(row0.contains('⌗'), "the maze-flagged tab must carry the ⌗ marker, got {row0:?}");
    }

    #[test]
    fn room_number_visibility_toggles_id_and_icon_placement() {
        // Hall(0,0) has an Out portal to Cellar(0,1) — due SOUTH. `Out` carries no bearing of its
        // own, so the badge is aimed at the partner's cell (SQ-0351), which puts it on the bottom
        // interior row either way. What `show_room_numbers` changes is what OCCUPIES that row:
        //   false -> "#1" absent, so the badge takes the centre of row 3;
        //   true  -> "#1" holds the centre, so the badge steps aside to the nearest blank.
        // The badge must never overwrite the id — the old rule dodged that by using a fixed column
        // on a different row, which ignored the partner entirely.
        use mapper::graph::MapGraph;

        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Cellar".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 1)); // due south of Hall
        g.add_edge(1, Direction::Out, 2);
        let rm = render(&g);

        let render_buf = |show_room_numbers: bool| {
            let mut st = AppState::default(); // Boxes zoom, scroll (0,0), show_portal_labels off
            st.show_room_numbers = show_room_numbers;
            let area = Rect::new(0, 0, 80, 40);
            let mut buf = Buffer::empty(area);
            render_map(&rm, &st, area, &mut buf);
            buf
        };

        // ── show_room_numbers = false (default) ──────────────────────────────────
        {
            let buf = render_buf(false);
            let sym = |x: u16, y: u16| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default();
            let row3: String = (1u16..=9).map(|x| sym(x, 3)).collect();
            assert!(!row3.contains("#1"), "numbers off: #id must be absent from row 3; got '{row3}'");
            // Pulled south, and with row 3 empty it takes the centre column.
            assert_eq!(sym(5, 3), "◎", "numbers off: badge centres on row 3; row3='{row3}'");
        }

        // ── show_room_numbers = true ──────────────────────────────────────────────
        {
            let buf = render_buf(true);
            let sym = |x: u16, y: u16| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default();
            let row3: String = (1u16..=9).map(|x| sym(x, 3)).collect();
            assert!(row3.contains("#1"), "numbers on: #id must appear on row 3; got '{row3}'");
            // Still pulled south — but the id now holds the centre, so it takes the nearest blank
            // beside it rather than landing on top of it.
            let on_row3 = (1u16..=9).find(|&x| sym(x, 3) == "◎");
            assert!(on_row3.is_some(), "numbers on: badge stays on row 3, toward Cellar; row3='{row3}'");
            assert!(row3.contains("#1"), "numbers on: the id survives the badge; got '{row3}'");
        }
    }

    /// The interior of the box at `(bx, by)` as 3 rows of 9 chars, for badge-placement asserts.
    fn interior_rows(buf: &Buffer, bx: u16, by: u16) -> Vec<String> {
        (1..=3)
            .map(|dy| {
                (1..=9)
                    .map(|dx| {
                        buf.cell((bx + dx, by + dy)).map(|c| c.symbol().to_string()).unwrap_or_default()
                    })
                    .collect()
            })
            .collect()
    }


    /// SQ-0363: with portal labels on, a cross-layer COMPASS passage rendered NOTHING — no icon,
    /// no name. `portal_slot` only ever had to hold Up/Down/In/Out, the four directions that could
    /// leave a layer before a named seam could cut at compass ones, so a compass edge
    /// fell through it and was dropped. Each direction must land on the border it leads through,
    /// with its "Room · Layer" name floating clear on that side.
    #[test]
    fn portal_view_shows_a_cross_layer_compass_passage_on_the_border_it_leads_through() {
        use mapper::graph::MapGraph;
        use mapper::render::render_layer;

        // (direction, the Vault's cell, the row/col the badge must land on relative to the box)
        for (dir, cell) in [
            (Direction::E, (1, 0)),
            (Direction::W, (-1, 0)),
            (Direction::N, (0, -1)),
            (Direction::S, (0, 1)),
            (Direction::NE, (1, -1)),
        ] {
            let mut g = MapGraph::new();
            g.upsert_room(1, "Here".into());
            g.upsert_room(2, "Vault".into());
            g.set_pos(1, (0, 0));
            g.set_pos(2, cell);
            g.add_edge(1, dir, 2);
            g.add_edge(2, mapper::direction::opposite(dir), 1);
            // Peel the VAULT's side (SQ-0364: a peel takes the selected room's own side), so
            // Here stays on Main and its `dir` passage is the one that crosses.
            let region = mapper::layer::region_at_edge(&g, 2, mapper::direction::opposite(dir))
                .expect("the walked passage is a seam");
            mapper::layer::move_region(&mut g, &region, mapper::layer::MoveTarget::New)
                .expect("cut at the seam");

            let rm = render_layer(&g, mapper::layer::MAIN_LAYER);
            let mut st = AppState::default();
            st.scroll = (rm.bounds.0 .0 - 1, rm.bounds.0 .1 - 1);
            st.show_portal_labels = true;
            let area = Rect::new(0, 0, 46, 26);
            let mut buf = Buffer::empty(area);
            render_map(&rm, &st, area, &mut buf);

            let text: String = (0..area.height)
                .map(|y| {
                    (0..area.width)
                        .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" ").to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");

            let arrow = arrow_for_direction(dir, &st.symbols.arrows, &st.symbols.portal);
            assert!(
                text.contains(arrow),
                "{dir:?}: the badge shows the direction travelled ({arrow:?})\n{text}"
            );
            assert!(
                text.contains("Vault · Vault"),
                "{dir:?}: and names the room and layer it leads to\n{text}"
            );
        }
    }

    #[test]
    fn a_cross_layer_compass_badge_shows_its_direction_not_the_unknown_marker() {
        // SQ-0362. Until a named seam (SQ-0360) could cut at compass passages, only
        // portals could ever cross layers — so the badge mapped Up/Down/In/Out and let every
        // compass direction fall through to `unknown`. A room whose east passage leads to another
        // layer then wore a "?", about a direction we know perfectly well.
        use mapper::graph::MapGraph;
        use mapper::render::render_layer;

        let mut g = MapGraph::new();
        g.upsert_room(1, "Here".into());
        g.upsert_room(2, "There".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        // Peel THERE's side, so HERE stays on Main and its east passage crosses layers. A peel
        // takes the selected room's OWN side (SQ-0364), hence standing at the far end.
        let region = mapper::layer::region_at_edge(&g, 2, Direction::W).expect("cut at the seam");
        let peeled = mapper::layer::move_region(&mut g, &region, mapper::layer::MoveTarget::New)
            .expect("and the region moves onto a fresh layer");
        assert_eq!(g.layer_of(2), peeled, "There is now a layer away, across a COMPASS edge");
        assert_eq!(g.layer_of(1), mapper::layer::MAIN_LAYER, "Here stayed put");

        let rm = render_layer(&g, mapper::layer::MAIN_LAYER);
        let mut st = AppState::default();
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);

        let here: Vec<String> = interior_rows(&buf, 0, 0);
        let joined = here.join("");
        let east_arrow = st.symbols.arrows.east; // '▶' by default
        assert!(
            joined.contains(east_arrow),
            "the badge shows the direction travelled ({east_arrow:?}): {here:?}"
        );
        assert!(
            !joined.contains(st.symbols.portal.unknown),
            "and never the unknown marker for a passage whose direction is known: {here:?}"
        );
    }

    #[test]
    fn an_in_out_badge_is_pulled_toward_the_room_it_connects_to() {
        // SQ-0351. `In`/`Out` carry no bearing of their own, so the badge is aimed at the partner's
        // cell. Two rooms side by side, each with a portal to the other, must put their badges on
        // OPPOSITE sides — each facing the other room:
        //
        //     ╭─────────╮ ╭─────────╮
        //     │  West   │ │  East   │
        //     │        ◎│ │◉        │
        //     ╰─────────╯ ╰─────────╯
        //              └───┘ facing each other
        //
        // The old rule pinned both to a fixed column, so the westward badge pointed away from its
        // partner — visible on Zork's Behind House, whose `◉` sat on the east side while Kitchen,
        // the room it leads to, is west.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "West".into());
        g.upsert_room(2, "East".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0)); // due east of West
        g.add_edge(1, Direction::Out, 2);
        g.add_edge(2, Direction::In, 1);
        let rm = render(&g);
        let mut st = AppState::default();
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);

        let west = interior_rows(&buf, 0, 0);
        let east = interior_rows(&buf, (BOX_W + MIN_GUTTER) as u16, 0);
        let col_of = |rows: &[String], want: &str| -> Option<usize> {
            rows.iter().find_map(|r| r.chars().position(|c| c.to_string() == want))
        };
        let w = col_of(&west, "◎").unwrap_or_else(|| panic!("West's ◎ must render: {west:?}"));
        let e = col_of(&east, "◉").unwrap_or_else(|| panic!("East's ◉ must render: {east:?}"));
        assert!(w > e, "West's ◎ leans east ({w}) and East's ◉ leans west ({e}): {west:?} {east:?}");
        assert_eq!(w, 8, "West's badge takes the last interior column, nearest East");
        assert_eq!(e, 0, "East's badge takes the first interior column, nearest West");
    }

    #[test]
    fn a_badge_never_overwrites_the_room_name() {
        // SQ-0351's other half: "the closest EMPTY spot". Blankness is read back from the buffer
        // after the name is drawn, so a badge lands in the name's padding instead of on a letter.
        // The old fixed column silently clipped long names — Zork's `◀  House ◉▶` is "House"
        // centred as "  House  " with the badge sitting on column 9.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Behind".into()); // 6 chars in a 9-wide interior: " Behind  "
        g.upsert_room(2, "K".into());
        g.set_pos(1, (1, 0));
        g.set_pos(2, (0, 0)); // due WEST of Behind
        g.add_edge(1, Direction::In, 2);
        let rm = render(&g);
        let mut st = AppState::default();
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);

        let rows = interior_rows(&buf, (BOX_W + MIN_GUTTER) as u16, 0);
        assert!(rows[0].contains("Behind"), "the name survives intact: {rows:?}");
        assert!(
            rows.iter().any(|r| r.contains("◉")),
            "the badge renders: {rows:?}"
        );
        // Every name character is still present — nothing was overwritten.
        let all: String = rows.concat();
        assert_eq!(all.matches("Behind").count(), 1, "name not clipped by the badge: {rows:?}");
    }

    #[test]
    fn a_cross_layer_portal_shows_its_direction_of_travel_inside_the_room() {
        // SQ-0223. A room with a staircase to another layer carries a badge of the direction the
        // player travels — `Down` → `↓` — placed by SQ-0351's rule. `Down` HAS a bearing, so it is
        // read straight off the compass and lands on the bottom row; no partner lookup, which
        // matters because the destination is on another plane entirely.
        use mapper::graph::MapGraph;
        use mapper::layer::{move_region, planar_region, MoveTarget, MAIN_LAYER};
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Cellar".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 1));
        g.add_edge(1, Direction::Down, 2);
        g.add_edge(2, Direction::Up, 1);
        let region = planar_region(&g, 2);
        move_region(&mut g, &region, MoveTarget::New).expect("the cellar peels into its own layer");
        let rm = mapper::render::render_layer(&g, MAIN_LAYER);
        assert!(
            rm.rooms.iter().find(|r| r.id == 1).unwrap().has_layer_portal,
            "Hall owns the cross-layer portal",
        );
        let mut st = AppState::default();
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);

        let rows = interior_rows(&buf, 0, 0);
        assert!(
            rows[2].contains("↓"),
            "Down to another layer shows ↓ on the bottom interior row: {rows:?}",
        );
        // Before SQ-0223 a cross-layer portal drew NOTHING inside the room — same-layer Up/Down
        // put their glyph on the connector's border anchor, and a cross-layer stub has no
        // connector, so it fell through every branch.
        assert!(!rows[0].contains("↓") && !rows[1].contains("↓"), "only one badge: {rows:?}");
    }

    #[test]
    fn build_frame_manifest_drawn_in_map_pane() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use crate::state::{AppState, TidyAnim, TidyFrame};
        use mapper::graph::MapGraph;
        use mapper::layout::TidyStats;

        let mut state = AppState::default();
        state.tidy_anim = Some(TidyAnim::new(vec![TidyFrame {
            label: "Build".into(),
            graph: MapGraph::new(),
            description: "Graph built: 2 rooms, 1 connections".into(),
            stats: TidyStats::default(),
            stage_start: true,
            manifest: Some(vec!["Foyer \u{2192}N\u{2192} Hall".into()]),
        }], mapper::layer::MAIN_LAYER));

        // Empty render map, built with the same helper the neighboring tests use.
        let rm = mapper::render::render(&MapGraph::new());
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let text: String = buf.content.iter().flat_map(|c| c.symbol().chars()).collect();
        assert!(text.contains("Foyer"), "manifest line should be drawn in the map pane");
        assert!(text.contains("Hall"));
    }

    #[test]
    fn build_frame_manifest_starts_below_tidy_panel() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use crate::render::tidy_panel::PANEL_H;
        use crate::state::{AppState, TidyAnim, TidyFrame};
        use mapper::graph::MapGraph;
        use mapper::layout::TidyStats;

        let mut state = AppState::default();
        state.tidy_anim = Some(TidyAnim::new(vec![TidyFrame {
            label: "Build".into(),
            graph: MapGraph::new(),
            description: "Graph built: 2 rooms, 1 connections".into(),
            stats: TidyStats::default(),
            stage_start: true,
            manifest: Some(vec!["Foyer \u{2192}N\u{2192} Hall".into()]),
        }], mapper::layer::MAIN_LAYER));

        // Pane large enough for the tidy transport panel (>= PANEL_W x PANEL_H), so the
        // manifest must be offset below the panel rows instead of under it.
        let rm = mapper::render::render(&MapGraph::new());
        let area = Rect::new(0, 0, 80, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let row = |y: u16| -> String {
            (0..area.width)
                .map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' '))
                .collect()
        };
        for y in 0..PANEL_H {
            assert!(!row(y).contains("Foyer"), "manifest must not be drawn in panel row {y}");
        }
        assert!(row(PANEL_H).contains("Foyer"), "manifest should start at row PANEL_H");
    }

    #[test]
    fn shared_connector_line_uses_shared_path_color() {
        use crate::state::AppState;
        use mapper::graph::MapGraph;
        use mapper::direction::Direction;
        let mut g = MapGraph::new();
        g.upsert_room(68, "W".into());
        g.upsert_room(217, "S".into());
        g.set_pos(68, (0, 0));
        g.set_pos(217, (1, 1));
        for (o, d, dst) in [(68, Direction::S, 217), (68, Direction::SE, 217),
                            (217, Direction::W, 68), (217, Direction::NW, 68)] {
            g.add_edge(o, d, dst);
        }
        let state = AppState::default(); // Boxes zoom by default
        let rm = mapper::render::render(&g);
        let area = Rect::new(0, 0, 60, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        // At least one cell painted with the shared_path fg color exists (the shared line/arrow).
        // Compared via `cell.fg` (not `cell.style() ==`, which can never match a partially-set
        // Style: ratatui's `Cell::set_style` patches rather than replaces, so `Cell::style()`
        // always synthesizes concrete `bg`/`underline_color`, unlike `shared_path`'s bg: None).
        let shared_fg = state.colors.theme.get("map.shared_path").style.fg.expect("shared_path has an fg color");
        let found = (0..area.width).flat_map(|x| (0..area.height).map(move |y| (x, y)))
            .any(|(x, y)| buf.cell((x, y)).map(|c| c.fg == shared_fg).unwrap_or(false));
        assert!(found, "the collapsed pair's shared path must paint with shared_path color");
    }

    #[test]
    fn house_ring_collapses_to_clean_lines_with_no_illegal_overlap() {
        use mapper::graph::MapGraph;
        use mapper::direction::Direction;
        let mut g = MapGraph::new();
        for (id, p) in [(143, (1, 2)), (89, (2, 3)), (217, (1, 4)), (68, (0, 3))] {
            g.upsert_room(id, "r".into());
            g.set_pos(id, p);
        }
        // Diamond ring: each adjacent pair reachable by a cardinal AND a diagonal, both ways.
        let edges = [
            (68, Direction::N, 143), (68, Direction::NE, 143),
            (143, Direction::S, 68), (143, Direction::SW, 68),
            (143, Direction::E, 89), (143, Direction::SE, 89),
            (89, Direction::W, 143), (89, Direction::NW, 143),
            (89, Direction::S, 217), (89, Direction::SW, 217),
            (217, Direction::N, 89), (217, Direction::NE, 89),
            (217, Direction::W, 68), (217, Direction::NW, 68),
            (68, Direction::S, 217), (68, Direction::SE, 217),
        ];
        for (o, d, dst) in edges { g.add_edge(o, d, dst); }
        let plan = mapper::route::route_lanes(&g);
        // One compass connector per ring pair.
        for pair in [(68, 143), (89, 143), (89, 217), (68, 217)] {
            let n = plan.connectors.iter()
                .filter(|c| (c.origin.min(c.dest), c.origin.max(c.dest)) == pair
                    && mapper::direction::grid_offset(c.exit_dir).is_some())
                .count();
            assert_eq!(n, 1, "pair {pair:?} must collapse to one compass connector");
        }
        // No illegal overlaps in the rendered result.
        assert_eq!(render_overlap_stats(&g).0, 0, "ring must render without illegal overlap");
    }

    #[test]
    fn tidy_progress_draws_bordered_box() {
        use crate::state::AnimBuildJob;
        use std::sync::atomic::AtomicUsize;
        use std::sync::Arc;
        let mut state = AppState::default();
        let handle = std::thread::spawn(|| (Vec::new(), mapper::graph::MapGraph::new()));
        state.anim_build_job = Some(AnimBuildJob {
            handle,
            layer: 0,
            gen: 0,
            started: std::time::Instant::now(),
            progress: Arc::new(AtomicUsize::new(5)),
            total: 10,
            animate: true,
        });
        let area = Rect::new(0, 0, 60, 10);
        let mut buf = Buffer::empty(area);
        draw_tidy_progress(state.anim_build_job.as_ref().unwrap(), &state, area, &mut buf);
        let content: String = buf.content().iter().map(|c| c.symbol().to_owned()).collect();
        assert!(content.contains("Tidying"), "shows the Tidying label");
        assert!(content.contains('┌'), "has a top-left border corner");
        assert!(content.contains('│'), "has vertical border sides");
    }
}






// ── SQ-1255 ───────────────────────────────────────────────────────────────────
//
// Reproduction of the Zork I (release 52) "Canyon View" report of 2026-09-02: the
// automap threads connectors past/through room boxes around #23 Canyon View. The
// fixture is the player's own `/dump-map`, quoted below as `DUMP_POS` / `DUMP_DISTORTED`.
//
// This module is diagnosis-only and changes no layout, router or cleanup code. Its
// cases are `#[ignore]`d so they never redden CI while the fix is undecided; run with
//   cargo nextest run -p lanthorn --lib --features t-render sq1255 --run-ignored all
#[cfg(all(test, feature = "t-render"))]
mod sq1255_canyon_view {
    use super::*;
    use mapper::direction::Direction::{self, Up, E, N, NW, S, W};
    use mapper::graph::{MapGraph, RoomId};
    use mapper::layer::MAIN_LAYER;
    use mapper::mapper::Mapper;

    /// The 29 edges in the graph's insertion order, exactly as the dump lists them.
    const WALK: &[(RoomId, Direction, RoomId)] = &[
        (68, S, 217),
        (217, W, 68),
        (217, E, 89),
        (89, S, 217),
        (89, W, 28),
        (28, E, 89),
        (28, W, 79),
        (79, E, 28),
        (89, N, 143),
        (143, E, 89),
        (143, W, 68),
        (68, N, 143),
        (68, W, 91),
        (91, N, 167),
        (167, W, 91),
        (167, E, 33),
        (33, S, 134),
        (134, N, 33),
        (134, E, 23),
        (23, NW, 134),
        (23, E, 22),
        (22, Up, 23),
        (23, W, 230),
        (230, N, 134),
        (230, W, 91),
        (230, NW, 217),
        (134, S, 230),
        (134, W, 89),
        (89, E, 134),
    ];

    /// `pos=` from the dump's ROOMS legend.
    const DUMP_POS: &[(RoomId, (i32, i32))] = &[
        (22, (0, 0)),
        (23, (-1, -1)),
        (28, (-4, -2)),
        (33, (-2, -3)),
        (68, (-6, -2)),
        (79, (-5, -2)),
        (89, (-3, -2)),
        (91, (-7, -2)),
        (134, (-2, -2)),
        (143, (-4, -3)),
        (167, (-3, -3)),
        (217, (-4, -1)),
        (230, (-2, 0)),
    ];

    /// The edges the dump marks `distorted`, as (origin, dest) pairs in WALK order.
    const DUMP_DISTORTED: &[(RoomId, RoomId)] = &[
        (68, 217),
        (217, 68),
        (217, 89),
        (89, 217),
        (89, 143),
        (143, 89),
        (143, 68),
        (68, 143),
        (68, 91),
        (91, 167),
        (167, 91),
        (134, 23),
        (23, 22),
        (23, 230),
        (230, 91),
    ];

    fn name(id: RoomId) -> &'static str {
        match id {
            22 => "Rocky Ledge",
            23 => "Canyon View",
            28 => "Kitchen",
            33 => "Forest",
            68 => "West of House",
            79 => "Living Room",
            89 => "Behind House",
            91 => "Forest",
            134 => "Clearing",
            143 => "North of House",
            167 => "Clearing",
            217 => "South of House",
            230 => "Forest",
            _ => "?",
        }
    }

    /// `turn.rs::schedule_map_maintenance` + `loop_tick.rs::poll_tidy_jobs`, collapsed
    /// to one synchronous call. Same predicates, same order, same budgets; the only
    /// difference from the shipped app is that the tidy lands on this turn rather than
    /// a frame or two later, which the app's own in-crate tests already do
    /// (`session.rs::auto_mode_background_cleanup_keeps_map_free_of_illegal_overlaps`).
    fn maintain(m: &mut Mapper, new_room: bool, new_conn: bool, counter: &mut u32) -> &'static str {
        let changed = new_room || new_conn;
        if !crate::tidy::should_schedule_tidy(&m.graph, MAIN_LAYER, changed) {
            return "-";
        }
        let cells = mapper::layout::occupied_cells_in_layer(&m.graph, MAIN_LAYER);
        let total_rooms = m.graph.rooms_in_layer(MAIN_LAYER).len();
        let has_overlap = cells.len() < total_rooms;
        let has_distorted = m.graph.connections().iter().any(|c| {
            c.distorted
                && m.graph.layer_of(c.origin) == MAIN_LAYER
                && m.graph.layer_of(c.dest) == MAIN_LAYER
        });
        let overlap = has_overlap || has_distorted;
        let full = crate::tidy::should_bg_tidy(
            crate::config::BackgroundTidy::EveryRoom,
            new_room,
            overlap,
            changed,
            counter,
        );
        if full {
            crate::tidy::tidy_layer_silent(&mut m.graph, MAIN_LAYER);
            "FULL"
        } else {
            crate::tidy::cleanup_overlaps_layer_silent(&mut m.graph, MAIN_LAYER);
            "cleanup"
        }
    }

    /// Replay the first `turns` edges of the walk with the session's per-turn maintenance.
    ///
    /// The player's real transcript walked back over known passages between some of these
    /// crossings. Those turns are layout-INERT and can be skipped safely rather than
    /// reconstructed: `MapGraph::add_edge` is keyed by `(origin, dir)` for a compass
    /// passage, so re-walking one adds no connection; `place_incremental` returns early
    /// for an already-placed destination; and with neither a new room nor a new connection
    /// `should_schedule_tidy`'s `changed` is false, so no tidy is scheduled. Setting the
    /// current room directly is therefore exactly what those turns would have left behind.
    fn replay(turns: usize) -> Mapper {
        let mut m = Mapper::default();
        let mut counter = 0u32;
        // The opening `look` in West of House: an observation with no direction.
        m.observe(WALK[0].0, name(WALK[0].0), None);
        maintain(&mut m, true, false, &mut counter);
        for &(origin, dir, dest) in WALK.iter().take(turns) {
            m.graph.set_current(origin);
            let rooms_before = m.graph.rooms().count();
            let conns_before = m.graph.connections().len();
            m.observe_moved(dest, name(dest), Some(dir));
            let new_room = m.graph.rooms().count() > rooms_before;
            let new_conn = m.graph.connections().len() > conns_before;
            maintain(&mut m, new_room, new_conn, &mut counter);
        }
        m
    }

    /// Every connector cell that lands inside a room's 11x5 box, in the router's virtual space.
    ///
    /// A cell on the border ring of the connector's OWN origin or destination box is the
    /// legitimate arrival/departure anchor and is not reported. Everything else is a
    /// connector drawn over a room.
    /// (room whose box is entered, connector origin, exit direction, connector dest, cell).
    type Intrusion = (RoomId, RoomId, Direction, RoomId, (i32, i32));
    /// (room whose ring is occupied, connector origin, exit direction, connector dest, cells).
    type Hug = (RoomId, RoomId, Direction, RoomId, usize);

    fn box_intrusions(graph: &MapGraph) -> Vec<Intrusion> {
        let rm = mapper::render::render_layer(graph, MAIN_LAYER);
        let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);
        let boxes: Vec<(RoomId, (i32, i32))> = rm
            .rooms
            .iter()
            .map(|r| (r.id, (cols.room_pixel(r.cell.0), rows.room_pixel(r.cell.1))))
            .collect();
        let mut out = Vec::new();
        for conn in rm.plan.connectors.iter() {
            let Some(plot) = plot_connector(conn, &cols, &rows, None) else { continue };
            for (c, _mask) in &plot.cells {
                for &(rid, (bx, by)) in &boxes {
                    let inside = c.0 >= bx && c.0 < bx + BOX_W && c.1 >= by && c.1 < by + BOX_H;
                    if !inside {
                        continue;
                    }
                    let own = rid == conn.origin || rid == conn.dest;
                    let on_border =
                        c.0 == bx || c.0 == bx + BOX_W - 1 || c.1 == by || c.1 == by + BOX_H - 1;
                    if own && on_border {
                        continue;
                    }
                    out.push((rid, conn.origin, conn.exit_dir, conn.dest, *c));
                }
            }
        }
        out
    }

    fn pos_of(m: &Mapper, id: RoomId) -> Option<(i32, i32)> {
        m.graph.room(id).and_then(|r| r.pos)
    }

    /// Print the whole replay: per-turn maintenance kind, positions and intrusions.
    /// Diagnostic only — asserts nothing.
    #[test]
    #[ignore = "SQ-1255 diagnostic trace, not a pass/fail case"]
    fn sq1255_trace() {
        let mut m = Mapper::default();
        let mut counter = 0u32;
        m.observe(WALK[0].0, name(WALK[0].0), None);
        let k = maintain(&mut m, true, false, &mut counter);
        println!("turn  0  seed #{}   [{k}]", WALK[0].0);
        for (i, &(origin, dir, dest)) in WALK.iter().enumerate() {
            m.graph.set_current(origin);
            let rooms_before = m.graph.rooms().count();
            let conns_before = m.graph.connections().len();
            m.observe_moved(dest, name(dest), Some(dir));
            let new_room = m.graph.rooms().count() > rooms_before;
            let new_conn = m.graph.connections().len() > conns_before;
            let k = maintain(&mut m, new_room, new_conn, &mut counter);
            let intr = box_intrusions(&m.graph);
            let (illegal, cross) = render_overlap_stats(&m.graph);
            let mut ps: Vec<String> = m
                .graph
                .rooms()
                .filter_map(|r| r.pos.map(|p| (r.id, p)))
                .map(|(id, p)| format!("{id}@{},{}", p.0, p.1))
                .collect();
            ps.sort();
            println!(
                "turn {:2}  {origin} {dir:?} {dest}{}  [{k}]  illegal={illegal} cross={cross} intrude={}  {}",
                i + 1,
                if new_room { " NEW" } else { "    " },
                intr.len(),
                ps.join(" ")
            );
            for (rid, o, d, de, c) in &intr {
                println!("          intrusion: {o} {d:?} {de} at {c:?} inside box of #{rid}");
            }
        }
        println!(
            "\n--- final dump ---\n{}",
            crate::map_dump::render_dump(&m.graph, &crate::symbols::SymbolSet::default())
        );
    }

    /// Does the replay land on the same grid the player's dump recorded?
    #[test]
    #[ignore = "SQ-1255 fixture comparison; see the report"]
    fn sq1255_replay_matches_dump_positions() {
        let m = replay(WALK.len());
        let mut bad = Vec::new();
        for &(id, want) in DUMP_POS {
            let got = pos_of(&m, id);
            if got != Some(want) {
                bad.push(format!("#{id} {}: dump {want:?} replay {got:?}", name(id)));
            }
        }
        assert!(bad.is_empty(), "positions differ from the dump:\n  {}", bad.join("\n  "));
    }

    /// Does the replay mark the same edges distorted?
    #[test]
    #[ignore = "SQ-1255 fixture comparison; see the report"]
    fn sq1255_replay_matches_dump_distortion() {
        let m = replay(WALK.len());
        let got: Vec<(RoomId, RoomId)> = m
            .graph
            .connections()
            .iter()
            .filter(|c| c.distorted)
            .map(|c| (c.origin, c.dest))
            .collect();
        assert_eq!(got, DUMP_DISTORTED.to_vec(), "distorted set differs from the dump");
    }

    /// The defect itself: no connector may be drawn inside a room's box.
    #[test]
    #[ignore = "SQ-1255: the reported defect — a connector drawn through a room box"]
    fn sq1255_no_connector_is_drawn_through_a_room_box() {
        let m = replay(WALK.len());
        let intr = box_intrusions(&m.graph);
        let lines: Vec<String> = intr
            .iter()
            .map(|(rid, o, d, de, c)| {
                format!(
                    "connector {o} {d:?} {de} draws at {c:?} inside the box of #{rid} {}",
                    name(*rid)
                )
            })
            .collect();
        assert!(intr.is_empty(), "connectors drawn through room boxes:\n  {}", lines.join("\n  "));
    }

    /// Foreign connector cells in the one-cell ring around a room box, for one graph.
    fn hug_count(g: &MapGraph) -> Vec<Hug> {
        let rm = mapper::render::render_layer(g, MAIN_LAYER);
        let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);
        let mut out = Vec::new();
        for room in rm.rooms.iter() {
            let (bx, by) = (cols.room_pixel(room.cell.0), rows.room_pixel(room.cell.1));
            for conn in rm.plan.connectors.iter() {
                if conn.origin == room.id || conn.dest == room.id {
                    continue;
                }
                let Some(plot) = plot_connector(conn, &cols, &rows, None) else { continue };
                let n = plot
                    .cells
                    .iter()
                    .filter(|(c, _)| {
                        c.0 >= bx - 1 && c.0 <= bx + BOX_W && c.1 >= by - 1 && c.1 <= by + BOX_H
                    })
                    .count();
                if n > 0 {
                    out.push((room.id, conn.origin, conn.exit_dir, conn.dest, n));
                }
            }
        }
        out
    }

    /// The turn at which each symptom first appears, walked turn by turn.
    #[test]
    #[ignore = "SQ-1255 diagnostic: names the turn the defect appears"]
    fn sq1255_first_bad_turn() {
        let mut first_intrusion: Option<usize> = None;
        let mut first_hug: Option<usize> = None;
        for t in 1..=WALK.len() {
            let m = replay(t);
            if first_intrusion.is_none() && !box_intrusions(&m.graph).is_empty() {
                first_intrusion = Some(t);
            }
            let hugs = hug_count(&m.graph);
            if !hugs.is_empty() {
                let (o, d, de) = WALK[t - 1];
                println!("turn {t:2} ({o} {d:?} {de}): {hugs:?}");
                if first_hug.is_none() {
                    first_hug = Some(t);
                }
            }
        }
        println!("first connector drawn INSIDE a box: {first_intrusion:?} (None = never)");
        println!("first foreign connector HUGGING a box: {first_hug:?}");
    }

    /// Room boxes, channel widths, per-connector cell extents, foreign connectors hugging
    /// a box, and multiply-owned cells — the geometry behind the reported picture.
    #[test]
    #[ignore = "SQ-1255 diagnostic"]
    fn sq1255_geometry_report() {
        let m = replay(WALK.len());
        let rm = mapper::render::render_layer(&m.graph, MAIN_LAYER);
        let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);
        let boxes: Vec<(RoomId, (i32, i32))> = rm
            .rooms
            .iter()
            .map(|r| (r.id, (cols.room_pixel(r.cell.0), rows.room_pixel(r.cell.1))))
            .collect();
        println!("== room boxes (virtual) ==");
        for &(id, (bx, by)) in &boxes {
            println!("  #{id:<4} x {bx}..{}  y {by}..{}", bx + BOX_W - 1, by + BOX_H - 1);
        }

        println!("\n== channel gaps ==");
        let mut xs: Vec<i32> = boxes.iter().map(|b| b.1 .0).collect();
        xs.sort_unstable();
        xs.dedup();
        for w in xs.windows(2) {
            println!("  cols {}..{} -> {} cells", w[0] + BOX_W, w[1] - 1, w[1] - (w[0] + BOX_W));
        }
        let mut ys: Vec<i32> = boxes.iter().map(|b| b.1 .1).collect();
        ys.sort_unstable();
        ys.dedup();
        for w in ys.windows(2) {
            println!("  rows {}..{} -> {} cells", w[0] + BOX_H, w[1] - 1, w[1] - (w[0] + BOX_H));
        }

        println!("\n== connectors ==");
        for conn in rm.plan.connectors.iter() {
            let Some(plot) = plot_connector(conn, &cols, &rows, None) else {
                println!("  {} {:?} {} -> NO PLOT", conn.origin, conn.exit_dir, conn.dest);
                continue;
            };
            let cs: Vec<(i32, i32)> = plot.cells.iter().map(|(c, _)| *c).collect();
            println!(
                "  {} {:?} {}  dep{:?} arr{:?}  {} cells  x[{}..{}] y[{}..{}]",
                conn.origin,
                conn.exit_dir,
                conn.dest,
                plot.dep_anchor,
                plot.arr_anchor,
                cs.len(),
                cs.iter().map(|c| c.0).min().unwrap_or(0),
                cs.iter().map(|c| c.0).max().unwrap_or(0),
                cs.iter().map(|c| c.1).min().unwrap_or(0),
                cs.iter().map(|c| c.1).max().unwrap_or(0),
            );
        }

        println!("\n== foreign connector cells in the 1-cell ring around a box ==");
        for &(rid, (bx, by)) in &boxes {
            let mut hits: Vec<String> = Vec::new();
            for conn in rm.plan.connectors.iter() {
                if conn.origin == rid || conn.dest == rid {
                    continue;
                }
                let Some(plot) = plot_connector(conn, &cols, &rows, None) else { continue };
                let n = plot
                    .cells
                    .iter()
                    .filter(|(c, _)| {
                        c.0 >= bx - 1 && c.0 <= bx + BOX_W && c.1 >= by - 1 && c.1 <= by + BOX_H
                    })
                    .count();
                if n > 0 {
                    hits.push(format!("{} {:?} {} ({n})", conn.origin, conn.exit_dir, conn.dest));
                }
            }
            if !hits.is_empty() {
                println!("  #{rid}: {}", hits.join(", "));
            }
        }

        println!("\n== multiply-owned cells ==");
        use std::collections::BTreeMap;
        let mut owners: BTreeMap<(i32, i32), Vec<(usize, u8)>> = BTreeMap::new();
        for (ci, conn) in rm.plan.connectors.iter().enumerate() {
            if let Some(plot) = plot_connector(conn, &cols, &rows, None) {
                for (c, mask) in &plot.cells {
                    let e = owners.entry(*c).or_default();
                    if let Some(slot) = e.iter_mut().find(|(i, _)| *i == ci) {
                        slot.1 |= *mask;
                    } else {
                        e.push((ci, *mask));
                    }
                }
            }
        }
        for (c, v) in owners.iter().filter(|(_, v)| v.len() > 1) {
            let who: Vec<String> = v
                .iter()
                .map(|(ci, mask)| {
                    let k = &rm.plan.connectors[*ci];
                    format!("{} {:?} {} mask={mask:04b}", k.origin, k.exit_dir, k.dest)
                })
                .collect();
            println!("  {c:?}: {}", who.join(" | "));
        }
    }

    /// Report the 134↔230 connector's cell extent and how many of its cells hug #23.
    fn hug_report(g: &MapGraph, tag: &str) {
        let rm = mapper::render::render_layer(g, MAIN_LAYER);
        let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);
        let b23 = rm
            .rooms
            .iter()
            .find(|r| r.id == 23)
            .map(|r| (cols.room_pixel(r.cell.0), rows.room_pixel(r.cell.1)));
        for conn in rm.plan.connectors.iter() {
            let pair = (conn.origin.min(conn.dest), conn.origin.max(conn.dest));
            if pair != (134, 230) {
                continue;
            }
            let Some(plot) = plot_connector(conn, &cols, &rows, None) else { continue };
            let xs: Vec<i32> = plot.cells.iter().map(|(c, _)| c.0).collect();
            let hug = match b23 {
                Some((bx, by)) => plot
                    .cells
                    .iter()
                    .filter(|(c, _)| {
                        c.0 >= bx - 1 && c.0 <= bx + BOX_W && c.1 >= by - 1 && c.1 <= by + BOX_H
                    })
                    .count(),
                None => 0,
            };
            let (illegal, _) = render_overlap_stats(g);
            println!(
                "{tag}: 134<->230 spans x[{}..{}] ({} cells), hugs #23 in {hug} cells; illegal={illegal}",
                xs.iter().min().unwrap(),
                xs.iter().max().unwrap(),
                plot.cells.len(),
            );
        }
        for conn in rm.plan.connectors.iter() {
            if conn.origin != 230 && conn.dest != 230 {
                continue;
            }
            let Some(plot) = plot_connector(conn, &cols, &rows, None) else { continue };
            println!(
                "      {} {:?} {}: exit {:?} slot {} / entry {:?} slot {} corner {:?}  dep{:?} arr{:?}",
                conn.origin,
                conn.exit_dir,
                conn.dest,
                conn.exit,
                conn.exit_slot,
                conn.entry,
                conn.entry_slot,
                conn.entry_corner,
                plot.dep_anchor,
                plot.arr_anchor,
            );
        }
    }

    /// Counterfactuals: is the detour caused by #23's presence, by the two-row gap
    /// between #134 and #230, or by the `230 NW 217` constraint that opens the gap?
    #[test]
    #[ignore = "SQ-1255 diagnostic"]
    fn sq1255_counterfactuals() {
        // (0) As shipped.
        let m = replay(WALK.len());
        hug_report(&m.graph, "shipped        ");

        // (1) Same layout, #23 moved far away: does the 134<->230 connector still detour?
        let mut g1 = m.graph.clone();
        g1.set_pos(23, (3, -1));
        hug_report(&g1, "#23 moved away ");

        // (2) Same graph, #230 pulled onto the cell directly north (the gap closed).
        let mut g2 = m.graph.clone();
        g2.set_pos(230, (-2, -1));
        hug_report(&g2, "#230 at (-2,-1)");

        // (3) The walk without `230 NW 217` — the edge that pins #230 a row south of
        //     #217's row and so opens the two-row gap in the 33/134/230 column.
        let mut m3 = Mapper::default();
        let mut counter = 0u32;
        m3.observe(WALK[0].0, name(WALK[0].0), None);
        maintain(&mut m3, true, false, &mut counter);
        for &(origin, dir, dest) in WALK.iter() {
            if (origin, dest) == (230, 217) {
                continue;
            }
            m3.graph.set_current(origin);
            let rb = m3.graph.rooms().count();
            let cb = m3.graph.connections().len();
            m3.observe_moved(dest, name(dest), Some(dir));
            let nr = m3.graph.rooms().count() > rb;
            let nc = m3.graph.connections().len() > cb;
            maintain(&mut m3, nr, nc, &mut counter);
        }
        hug_report(&m3.graph, "no 230 NW 217  ");

        // (4) The same 29 edges, discovered in a different order: the two Canyon View
        //     crossings walked LAST. If the layout is order-stable this changes nothing.
        let mut order: Vec<(RoomId, Direction, RoomId)> = Vec::new();
        let late: &[(RoomId, RoomId)] = &[(23, 230), (230, 217)];
        for &e in WALK {
            if !late.contains(&(e.0, e.2)) {
                order.push(e);
            }
        }
        for &e in WALK {
            if late.contains(&(e.0, e.2)) {
                order.push(e);
            }
        }
        let mut m4 = Mapper::default();
        let mut counter = 0u32;
        m4.observe(order[0].0, name(order[0].0), None);
        maintain(&mut m4, true, false, &mut counter);
        for &(origin, dir, dest) in &order {
            m4.graph.set_current(origin);
            let rb = m4.graph.rooms().count();
            let cb = m4.graph.connections().len();
            m4.observe_moved(dest, name(dest), Some(dir));
            let nr = m4.graph.rooms().count() > rb;
            let nc = m4.graph.connections().len() > cb;
            maintain(&mut m4, nr, nc, &mut counter);
        }
        hug_report(&m4.graph, "reordered walk ");

        // (5) The MINIMAL order change: swap the two adjacent crossings 23 (23 W 230)
        //     and 24 (230 N 134). Both are the first-listed edge of their room pair, so
        //     the swap is exactly the `ci` tiebreak in `direct_route_losers` and in
        //     `assign_side_slots`, and nothing else.
        let mut order5: Vec<(RoomId, Direction, RoomId)> = WALK.to_vec();
        order5.swap(22, 23);
        let mut m5 = Mapper::default();
        let mut counter = 0u32;
        m5.observe(order5[0].0, name(order5[0].0), None);
        maintain(&mut m5, true, false, &mut counter);
        for &(origin, dir, dest) in &order5 {
            m5.graph.set_current(origin);
            let rb = m5.graph.rooms().count();
            let cb = m5.graph.connections().len();
            m5.observe_moved(dest, name(dest), Some(dir));
            let nr = m5.graph.rooms().count() > rb;
            let nc = m5.graph.connections().len() > cb;
            maintain(&mut m5, nr, nc, &mut counter);
        }
        hug_report(&m5.graph, "swap 23<->24   ");
    }

    /// **The reported defect.** No connector belonging to some OTHER pair of rooms may run
    /// flush along a room's box border — the one-cell ring around the box must stay clear.
    ///
    /// `overlap_stats` (and therefore `cleanup_overlaps`, which minimises it) scores only
    /// cells owned by two or more CONNECTORS; a connector running alongside a BOX is owned
    /// by one connector and scores zero, so nothing in the pipeline has a reason to prefer
    /// the straight route. Fixed by `direct_route_losers`' straightness tiebreak (SQ-1255):
    /// kept un-ignored as the regression pin.
    #[test]
    fn sq1255_no_foreign_connector_hugs_a_room_box() {
        let m = replay(WALK.len());
        let rm = mapper::render::render_layer(&m.graph, MAIN_LAYER);
        let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);
        let mut bad: Vec<String> = Vec::new();
        for room in rm.rooms.iter() {
            let (bx, by) = (cols.room_pixel(room.cell.0), rows.room_pixel(room.cell.1));
            for conn in rm.plan.connectors.iter() {
                if conn.origin == room.id || conn.dest == room.id {
                    continue;
                }
                let Some(plot) = plot_connector(conn, &cols, &rows, None) else { continue };
                let n = plot
                    .cells
                    .iter()
                    .filter(|(c, _)| {
                        c.0 >= bx - 1 && c.0 <= bx + BOX_W && c.1 >= by - 1 && c.1 <= by + BOX_H
                    })
                    .count();
                if n > 0 {
                    bad.push(format!(
                        "{} {:?} {} lays {n} cells in the ring around #{} {}",
                        conn.origin,
                        conn.exit_dir,
                        conn.dest,
                        room.id,
                        name(room.id)
                    ));
                }
            }
        }
        assert!(bad.is_empty(), "connectors run flush along a room box:\n  {}", bad.join("\n  "));
    }
}
