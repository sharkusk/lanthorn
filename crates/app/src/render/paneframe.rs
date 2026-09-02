use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

// ── BorderStyle ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderStyle {
    None,
    Single,
    Double,
    Thick,
    Rounded,
}

pub fn parse_border_style(s: &str) -> BorderStyle {
    match s {
        "none" => BorderStyle::None,
        "single" => BorderStyle::Single,
        "double" => BorderStyle::Double,
        "thick" => BorderStyle::Thick,
        "rounded" => BorderStyle::Rounded,
        _ => BorderStyle::Single,
    }
}

/// Return the canonical style-file name for a [`BorderStyle`].
pub fn border_style_name(style: BorderStyle) -> &'static str {
    match style {
        BorderStyle::None => "none",
        BorderStyle::Single => "single",
        BorderStyle::Double => "double",
        BorderStyle::Thick => "thick",
        BorderStyle::Rounded => "rounded",
    }
}

// ── InsetCaps ─────────────────────────────────────────────────────────────────

/// The title-strip terminator/divider glyphs derived from a pane's border style:
/// the left bracket, the right bracket, and the divider drawn between segments.
pub struct InsetCaps {
    pub left: String,
    pub right: String,
    pub divider: String,
}

impl InsetCaps {
    /// Terminator/divider caps for a border style (see the design's glyph table).
    pub fn for_border(b: BorderStyle) -> InsetCaps {
        let (l, r, d) = match b {
            BorderStyle::Thick => ("┫", "┣", "┃"),
            BorderStyle::Double => ("╡", "╞", "┃"), // divider is thick ┃ by design
            BorderStyle::Single | BorderStyle::Rounded | BorderStyle::None => ("┤", "├", "│"),
        };
        InsetCaps { left: l.into(), right: r.into(), divider: d.into() }
    }
}

// ── PaneFrame ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct PaneFrame {
    pub area: Rect,
    pub content: Rect,
    pub top_inset: Rect,
}

// ── Glyph sets ────────────────────────────────────────────────────────────────

struct Glyphs {
    tl: &'static str,
    top: &'static str,
    tr: &'static str,
    side: &'static str,
    bl: &'static str,
    br: &'static str,
}

const SINGLE:  Glyphs = Glyphs { tl: "┌", top: "─", tr: "┐", side: "│", bl: "└", br: "┘" };
const DOUBLE:  Glyphs = Glyphs { tl: "╔", top: "═", tr: "╗", side: "║", bl: "╚", br: "╝" };
const THICK:   Glyphs = Glyphs { tl: "┏", top: "━", tr: "┓", side: "┃", bl: "┗", br: "┛" };
const ROUNDED: Glyphs = Glyphs { tl: "╭", top: "─", tr: "╮", side: "│", bl: "╰", br: "╯" };

// ── draw_pane_frame ────────────────────────────────────────────────────────────

pub fn draw_pane_frame(buf: &mut Buffer, area: Rect, style: BorderStyle, glyphs: &PaneGlyphs, color: Style) -> PaneFrame {
    let effective = match style {
        BorderStyle::None => {
            // No border drawn; content == area; top_inset is the top row
            let top_inset = Rect::new(area.x, area.y, area.width, 1.min(area.height));
            return PaneFrame { area, content: area, top_inset };
        }
        other => other,
    };

    if area.width < 2 || area.height < 2 {
        // Too small to draw a border; degrade to None
        let top_inset = Rect::new(area.x, area.y, area.width, 1.min(area.height));
        return PaneFrame { area, content: area, top_inset };
    }

    let g = match effective {
        BorderStyle::Single => &SINGLE,
        BorderStyle::Double => &DOUBLE,
        BorderStyle::Thick  => &THICK,
        BorderStyle::Rounded => &ROUNDED,
        _ => unreachable!(),
    };

    let x = area.x;
    let y = area.y;
    let w = area.width;
    let h = area.height;
    let right = x + w - 1;
    let bottom = y + h - 1;

    // Draw top row
    if let Some(cell) = buf.cell_mut((x, y)) { cell.set_symbol(cell_sym(&glyphs.tl, g.tl)).set_style(color); }
    if let Some(cell) = buf.cell_mut((right, y)) { cell.set_symbol(cell_sym(&glyphs.tr, g.tr)).set_style(color); }
    for cx in (x + 1)..right {
        if let Some(cell) = buf.cell_mut((cx, y)) { cell.set_symbol(cell_sym(&glyphs.top, g.top)).set_style(color); }
    }

    // Draw bottom row
    if let Some(cell) = buf.cell_mut((x, bottom)) { cell.set_symbol(cell_sym(&glyphs.bl, g.bl)).set_style(color); }
    if let Some(cell) = buf.cell_mut((right, bottom)) { cell.set_symbol(cell_sym(&glyphs.br, g.br)).set_style(color); }
    for cx in (x + 1)..right {
        if let Some(cell) = buf.cell_mut((cx, bottom)) { cell.set_symbol(cell_sym(&glyphs.bottom, g.top)).set_style(color); }
    }

    // Draw left and right sides
    for cy in (y + 1)..bottom {
        if let Some(cell) = buf.cell_mut((x, cy)) { cell.set_symbol(cell_sym(&glyphs.left, g.side)).set_style(color); }
        if let Some(cell) = buf.cell_mut((right, cy)) { cell.set_symbol(cell_sym(&glyphs.right, g.side)).set_style(color); }
    }

    // content = area inset by 1 on each side
    let content = Rect::new(x + 1, y + 1, w.saturating_sub(2), h.saturating_sub(2));

    // top_inset = top border row between corners (x+1 .. right-1) at row y
    let inset_x = x + 1;
    let inset_w = right.saturating_sub(inset_x);
    let top_inset = Rect::new(inset_x, y, inset_w, 1);

    PaneFrame { area, content, top_inset }
}

// ── PaneGlyphs + PaneSides + per-side frame drawing ───────────────────────────

/// Per-side/corner glyph overrides for a pane border.
///
/// Each field is `Some(glyph)` when the user has explicitly set an override for
/// that position, or `None` to fall back to the border-style default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaneGlyphs {
    pub top: Option<String>,
    pub bottom: Option<String>,
    pub left: Option<String>,
    pub right: Option<String>,
    pub tl: Option<String>,
    pub tr: Option<String>,
    pub bl: Option<String>,
    pub br: Option<String>,
}

/// Per-side border styles for one pane. A side of `None` is omitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneSides {
    pub top: BorderStyle,
    pub bottom: BorderStyle,
    pub left: BorderStyle,
    pub right: BorderStyle,
}

impl PaneSides {
    /// All four sides set to one style.
    pub fn all(style: BorderStyle) -> PaneSides {
        PaneSides { top: style, bottom: style, left: style, right: style }
    }

    /// True if any side draws a border (i.e. not all `None`). Render gates that
    /// decide whether to box a pane MUST use this, not the base `*_style`, or a
    /// `style="none"` + per-side config renders nothing.
    pub fn any_on(self) -> bool {
        self.top != BorderStyle::None
            || self.bottom != BorderStyle::None
            || self.left != BorderStyle::None
            || self.right != BorderStyle::None
    }
}

/// "Weight" used to pick a corner glyph when two adjacent sides differ:
/// thick > double > single > none.
fn border_weight(s: BorderStyle) -> u8 {
    match s {
        BorderStyle::Thick => 3,
        BorderStyle::Double => 2,
        BorderStyle::Single | BorderStyle::Rounded => 1,
        _ => 0, // None never reaches the per-side corner path
    }
}

fn glyphs_for(style: BorderStyle) -> &'static Glyphs {
    match style {
        BorderStyle::Double => &DOUBLE,
        BorderStyle::Thick => &THICK,
        BorderStyle::Rounded => &ROUNDED,
        _ => &SINGLE,
    }
}

/// The plain RUN glyph one side of `style` is drawn with — `horizontal` picks the
/// top/bottom run, else the left/right one.
///
/// A frame is not the only thing that draws a border run: the Glk inter-window
/// separator (`render::screen::draw_window_separator`) is a single rule with no
/// corners and no box, and it must use the same glyph the frame would, or a
/// `style = "double"` in `style.toml` would give a doubled frame and a single
/// separator. (SQ-0821)
pub fn rule_glyph(style: BorderStyle, horizontal: bool) -> &'static str {
    let g = glyphs_for(style);
    if horizontal { g.top } else { g.side }
}

/// Return `over`'s string if set, otherwise `fallback`.
///
/// Used at every border cell write to apply a per-cell glyph override from
/// [`PaneGlyphs`] on top of the style-computed default glyph.
#[inline]
fn cell_sym<'a>(over: &'a Option<String>, fallback: &'a str) -> &'a str {
    over.as_deref().unwrap_or(fallback)
}

/// Which corner of the frame, for `corner_glyph`.
#[derive(Clone, Copy)]
enum Corner { Tl, Tr, Bl, Br }

/// The glyph for one corner, given its horizontal (top/bottom) and vertical
/// (left/right) adjacent side styles. Both present → corner glyph of the heavier
/// style; only horizontal → that horizontal glyph; only vertical → that vertical
/// glyph; neither → a space.
fn corner_glyph(h: BorderStyle, v: BorderStyle, which: Corner) -> &'static str {
    let h_on = h != BorderStyle::None;
    let v_on = v != BorderStyle::None;
    match (h_on, v_on) {
        (true, true) => {
            let g = if border_weight(h) >= border_weight(v) { glyphs_for(h) } else { glyphs_for(v) };
            match which { Corner::Tl => g.tl, Corner::Tr => g.tr, Corner::Bl => g.bl, Corner::Br => g.br }
        }
        (true, false) => glyphs_for(h).top,
        (false, true) => glyphs_for(v).side,
        (false, false) => " ",
    }
}

/// Draw a pane frame with independent per-side styles. Each present side draws
/// its straight glyph; corners resolve via `corner_glyph`; `content` is inset by
/// 1 only on sides that have a border; `top_inset` is the top row (between the
/// left/right insets) only when the top side is present, else zero-height.
pub fn draw_pane_frame_sides(buf: &mut Buffer, area: Rect, sides: PaneSides, glyphs: &PaneGlyphs, color: Style) -> PaneFrame {
    if area.width < 2 || area.height < 2 {
        let top_inset = Rect::new(area.x, area.y, area.width, 1.min(area.height));
        return PaneFrame { area, content: area, top_inset };
    }
    let x = area.x;
    let y = area.y;
    let right = x + area.width - 1;
    let bottom = y + area.height - 1;
    let on = |s: BorderStyle| s != BorderStyle::None;

    // Horizontal runs (top/bottom): straight glyph between the corners.
    if on(sides.top) {
        let g = glyphs_for(sides.top);
        for cx in (x + 1)..right {
            if let Some(c) = buf.cell_mut((cx, y)) { c.set_symbol(cell_sym(&glyphs.top, g.top)).set_style(color); }
        }
    }
    if on(sides.bottom) {
        let g = glyphs_for(sides.bottom);
        for cx in (x + 1)..right {
            if let Some(c) = buf.cell_mut((cx, bottom)) { c.set_symbol(cell_sym(&glyphs.bottom, g.top)).set_style(color); }
        }
    }
    // Vertical runs (left/right).
    if on(sides.left) {
        let g = glyphs_for(sides.left);
        for cy in (y + 1)..bottom {
            if let Some(c) = buf.cell_mut((x, cy)) { c.set_symbol(cell_sym(&glyphs.left, g.side)).set_style(color); }
        }
    }
    if on(sides.right) {
        let g = glyphs_for(sides.right);
        for cy in (y + 1)..bottom {
            if let Some(c) = buf.cell_mut((right, cy)) { c.set_symbol(cell_sym(&glyphs.right, g.side)).set_style(color); }
        }
    }
    // Corners.
    let set = |buf: &mut Buffer, px: u16, py: u16, sym: &str| {
        if sym != " " {
            if let Some(c) = buf.cell_mut((px, py)) { c.set_symbol(sym).set_style(color); }
        }
    };
    set(buf, x, y,       cell_sym(&glyphs.tl, corner_glyph(sides.top,    sides.left,  Corner::Tl)));
    set(buf, right, y,   cell_sym(&glyphs.tr, corner_glyph(sides.top,    sides.right, Corner::Tr)));
    set(buf, x, bottom,  cell_sym(&glyphs.bl, corner_glyph(sides.bottom, sides.left,  Corner::Bl)));
    set(buf, right, bottom, cell_sym(&glyphs.br, corner_glyph(sides.bottom, sides.right, Corner::Br)));

    // Content: inset 1 on each bordered side only.
    let l = if on(sides.left) { 1 } else { 0 };
    let r = if on(sides.right) { 1 } else { 0 };
    let t = if on(sides.top) { 1 } else { 0 };
    let b = if on(sides.bottom) { 1 } else { 0 };
    let content = Rect::new(
        x + l,
        y + t,
        area.width.saturating_sub(l + r),
        area.height.saturating_sub(t + b),
    );

    // top_inset only valid when the top side is present.
    let top_inset = if on(sides.top) {
        let inset_x = x + 1;
        let inset_w = right.saturating_sub(inset_x);
        Rect::new(inset_x, y, inset_w, 1)
    } else {
        Rect::new(x, y, 0, 0)
    };

    PaneFrame { area, content, top_inset }
}

// ── HeaderControl ─────────────────────────────────────────────────────────────

/// One clickable toggle drawn at the right-hand end of a pane's top border
/// (SQ-1123): the glyph that says what state it is in, and the style that says
/// whether it is on.
///
/// Theme-agnostic on purpose — `render::controls` resolves state into a glyph
/// and a selector, and this module only paints what it is given, exactly as
/// [`InsetSegment`] does for the title strip.
pub struct HeaderControl {
    pub glyph: char,
    pub style: Style,
    /// Which of the pane's border clusters this control belongs to (SQ-1123).
    pub placement: ControlPlacement,
}

/// Where on a pane's frame a border control sits.
///
/// **A control sits where the thing it governs is, or where it would appear.**
/// The command band opens BELOW the story pane, so its toggle rides the bottom
/// border; the map lives to the RIGHT, so its toggle takes the bottom border's
/// right-hand end, nearest the pane it summons; and the two v6 switches govern
/// how the story pane ITSELF is drawn, so they stay on that pane's own top
/// border. That is the same reasoning the arrow glyphs already encode — `▲`/`▼`
/// for a panel below, `◀`/`▶` for one to the side — which a single cluster in
/// one corner fought.
///
/// Geometry only, so this stays theme-agnostic like the rest of the module: the
/// three variants are three anchors on the frame, and `render::controls` decides
/// which control claims which.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPlacement {
    /// Right-hand end of the TOP border.
    TopRight,
    /// Centred on the BOTTOM border.
    BottomCentre,
    /// Right-hand end of the BOTTOM border.
    BottomRight,
}

/// The columns a cluster of `n` controls occupies: the two terminator caps plus
/// one cell per control and one space between neighbours. Zero for no controls,
/// so a pane with nothing to show gives its whole border row back to the title.
pub fn header_controls_width(n: usize) -> u16 {
    if n == 0 {
        return 0;
    }
    (2 + n + (n - 1)) as u16
}

/// Draw a control cluster in `row_rect`, bracketed by the border's own
/// terminator caps so it reads as part of the frame rather than as text sitting
/// on it: `┤▶ ● ▲├`.
///
/// `x` is the cluster's LEFT column, which the caller has already chosen —
/// right-aligning against the inset for an anchored group, or centring in it for
/// a floating one. Placing the cluster is the caller's job because on the bottom
/// border two groups share one row and the anchored one has to be resolved first.
///
/// Returns one 1x1 hit-rect per control, in the order given — the same rects the
/// mouse handler hit-tests for both the click and the hover hint, so what is
/// under the pointer can never disagree between them. An empty vec (and nothing
/// drawn) when the row is too narrow to hold the cluster where it was asked for.
pub fn draw_border_controls(
    buf: &mut Buffer,
    row_rect: Rect,
    x: u16,
    controls: &[HeaderControl],
    caps: &InsetCaps,
    cap_style: Style,
) -> Vec<Rect> {
    let w = header_controls_width(controls.len());
    if w == 0 || row_rect.height == 0 || x < row_rect.x || x + w > row_rect.right() {
        return Vec::new();
    }
    let row = row_rect.y;
    let mut cx = x;

    if let Some(c) = buf.cell_mut((cx, row)) {
        c.set_symbol(caps.left.as_str()).set_style(cap_style);
    }
    cx += 1;

    let mut rects = Vec::with_capacity(controls.len());
    for (i, ctl) in controls.iter().enumerate() {
        if i > 0 {
            if let Some(c) = buf.cell_mut((cx, row)) {
                c.set_symbol(" ").set_style(cap_style);
            }
            cx += 1;
        }
        if let Some(c) = buf.cell_mut((cx, row)) {
            c.set_symbol(&ctl.glyph.to_string()).set_style(ctl.style);
        }
        rects.push(Rect::new(cx, row, 1, 1));
        cx += 1;
    }

    if let Some(c) = buf.cell_mut((cx, row)) {
        c.set_symbol(caps.right.as_str()).set_style(cap_style);
    }
    rects
}

// ── InsetSegment ──────────────────────────────────────────────────────────────

pub struct InsetSegment<'a> {
    pub text: &'a str,
    pub active: bool,
}

// ── draw_top_inset ────────────────────────────────────────────────────────────

pub fn draw_top_inset(
    buf: &mut Buffer,
    top_inset: Rect,
    segments: &[InsetSegment],
    base: Style,
    active: Style,
    caps: &InsetCaps,
) -> Vec<Rect> {
    if segments.is_empty() || top_inset.width == 0 || top_inset.height == 0 {
        return segments.iter().map(|_| Rect::default()).collect();
    }

    // Build the full bracketed string to measure total width.
    // Format: ┫ seg0 ┃ seg1 ┃ seg2 ┣
    // We need to know:
    // - total chars to check if it fits
    // - which segment is active (for overflow logic)
    let active_idx = segments.iter().position(|s| s.active);

    let full_width = compute_full_width(segments);

    let avail = top_inset.width as usize;

    if full_width <= avail {
        // It fits: render centered
        let leading = (avail - full_width) / 2;
        let start_x = top_inset.x + leading as u16;
        render_segments(buf, top_inset.y, start_x, segments, base, active, caps)
    } else {
        // Overflow: show active segment ± neighbors with ‹…› markers
        render_overflow(buf, top_inset, segments, base, active, active_idx, caps)
    }
}

fn compute_full_width(segments: &[InsetSegment]) -> usize {
    if segments.is_empty() {
        return 0;
    }
    // ┫ + space + text + space + (┃ + space + text + space)* + ┣
    let n = segments.len();
    let text_len: usize = segments.iter().map(|s| s.text.chars().count()).sum();
    // 1 (┫) + 1 (space) + text_len + 1 (space) * n + (n-1) (┃) + 1 (┣)
    // = 1 + n*2 + text_len + (n-1) + 1
    // = 1 + 2n + text_len + n - 1 + 1 = 1 + 3n + text_len - 1 + 1
    1 + n * 2 + text_len + (n - 1) + 1
}

/// Render all segments into the buffer starting at start_x, returning hit-rects.
fn render_segments(
    buf: &mut Buffer,
    row: u16,
    start_x: u16,
    segments: &[InsetSegment],
    base: Style,
    active: Style,
    caps: &InsetCaps,
) -> Vec<Rect> {
    let mut rects = vec![Rect::default(); segments.len()];
    let mut cx = start_x;

    // left bracket
    if let Some(c) = buf.cell_mut((cx, row)) {
        c.set_symbol(caps.left.as_str()).set_style(base);
    }
    cx += 1;

    for (i, seg) in segments.iter().enumerate() {
        if i > 0 {
            // divider
            if let Some(c) = buf.cell_mut((cx, row)) {
                c.set_symbol(caps.divider.as_str()).set_style(base);
            }
            cx += 1;
        }

        let seg_style = if seg.active { active } else { base };
        let seg_start_x = cx;

        // space
        if let Some(c) = buf.cell_mut((cx, row)) {
            c.set_symbol(" ").set_style(seg_style);
        }
        cx += 1;

        // text chars
        for ch in seg.text.chars() {
            if let Some(c) = buf.cell_mut((cx, row)) {
                let s = ch.to_string();
                c.set_symbol(&s).set_style(seg_style);
            }
            cx += 1;
        }

        // trailing space
        if let Some(c) = buf.cell_mut((cx, row)) {
            c.set_symbol(" ").set_style(seg_style);
        }
        cx += 1;

        let seg_end_x = cx;
        rects[i] = Rect::new(seg_start_x, row, seg_end_x - seg_start_x, 1);
    }

    // right bracket
    if let Some(c) = buf.cell_mut((cx, row)) {
        c.set_symbol(caps.right.as_str()).set_style(base);
    }

    rects
}

/// Overflow rendering: show active ± neighbors, with ‹…› markers.
/// Write one cell of a header row, never past `right` (SQ-1127).
///
/// `Buffer::cell_mut` clips to the BUFFER, which is the whole terminal — so it
/// happily accepts a column belonging to the pane to the right, or to the
/// corner glyph this pane's own frame just drew. Every write in
/// [`render_overflow`] goes through here so the row cannot outrun the rect it
/// was given, which is the one bound that matters when a segment is wider than
/// the inset and the fitting loop gives up and truncates.
fn put_capped(buf: &mut Buffer, x: u16, y: u16, right: u16, sym: &str, style: Style) {
    if x >= right {
        return;
    }
    if let Some(c) = buf.cell_mut((x, y)) {
        c.set_symbol(sym).set_style(style);
    }
}

fn render_overflow(
    buf: &mut Buffer,
    top_inset: Rect,
    segments: &[InsetSegment],
    base: Style,
    active: Style,
    active_idx: Option<usize>,
    caps: &InsetCaps,
) -> Vec<Rect> {
    let n = segments.len();
    let avail = top_inset.width as usize;
    let row = top_inset.y;
    let x = top_inset.x;
    // The one bound every write below respects. Exclusive, like `Rect::right`.
    let right = top_inset.right();

    // Find the active index, default to 0
    let ai = active_idx.unwrap_or(0);

    // Determine window of segments to show
    // Start with just the active segment; expand neighbors while space allows.
    // Markers: ‹…› is 3 chars each side (when needed).
    let marker = "‹…›"; // 3 chars
    let marker_width = 3usize;

    // Try to fit active + expanding neighbors
    let mut lo = ai;
    let mut hi = ai;

    loop {
        let needs_left_marker = lo > 0;
        let needs_right_marker = hi < n - 1;
        let left_overhead = if needs_left_marker { marker_width + 1 } else { 0 }; // marker + space
        let right_overhead = if needs_right_marker { marker_width + 1 } else { 0 };

        // Width for current window [lo..=hi]
        let window_segs = &segments[lo..=hi];
        let window_w = compute_full_width(window_segs);
        let total = left_overhead + window_w + right_overhead;

        if total > avail {
            // Can't fit even the active alone; just show active truncated
            break;
        }

        // Try expanding
        let can_expand_left = lo > 0;
        let can_expand_right = hi < n - 1;

        if !can_expand_left && !can_expand_right {
            break; // no more to expand
        }

        // Try expanding left first
        let mut expanded = false;
        if can_expand_left {
            let new_lo = lo - 1;
            let new_needs_left_marker = new_lo > 0;
            let new_left_overhead = if new_needs_left_marker { marker_width + 1 } else { 0 };
            let new_window_segs = &segments[new_lo..=hi];
            let new_window_w = compute_full_width(new_window_segs);
            let new_total = new_left_overhead + new_window_w + right_overhead;
            if new_total <= avail {
                lo = new_lo;
                expanded = true;
            }
        }

        if can_expand_right {
            let new_hi = hi + 1;
            let new_needs_right_marker = new_hi < n - 1;
            let new_right_overhead = if new_needs_right_marker { marker_width + 1 } else { 0 };
            let new_window_segs = &segments[lo..=new_hi];
            let new_window_w = compute_full_width(new_window_segs);
            let cur_left_overhead = if lo > 0 { marker_width + 1 } else { 0 };
            let new_total = cur_left_overhead + new_window_w + new_right_overhead;
            if new_total <= avail {
                hi = new_hi;
                expanded = true;
            }
        }

        if !expanded {
            break;
        }
    }

    let needs_left_marker = lo > 0;
    let needs_right_marker = hi < n - 1;

    // Calculate total width for centering
    let left_overhead = if needs_left_marker { marker_width + 1 } else { 0 };
    let right_overhead = if needs_right_marker { marker_width + 1 } else { 0 };
    let window_w = compute_full_width(&segments[lo..=hi]);
    let total_w = left_overhead + window_w + right_overhead;

    let leading = if total_w < avail { (avail - total_w) / 2 } else { 0 };
    let mut cx = x + leading as u16;

    let mut rects = vec![Rect::default(); n];

    // Left marker
    if needs_left_marker {
        for ch in marker.chars() {
            put_capped(buf, cx, row, right, &ch.to_string(), base);
            cx += 1;
        }
        // space after marker
        put_capped(buf, cx, row, right, " ", base);
        cx += 1;
    }

    // left bracket
    put_capped(buf, cx, row, right, caps.left.as_str(), base);
    cx += 1;

    // Render visible segments
    for (wi, si) in (lo..=hi).enumerate() {
        let seg = &segments[si];
        if wi > 0 {
            // divider
            put_capped(buf, cx, row, right, caps.divider.as_str(), base);
            cx += 1;
        }

        let seg_style = if seg.active { active } else { base };
        let seg_start_x = cx;

        // space
        put_capped(buf, cx, row, right, " ", seg_style);
        cx += 1;

        // text chars
        for ch in seg.text.chars() {
            put_capped(buf, cx, row, right, &ch.to_string(), seg_style);
            cx += 1;
        }

        // trailing space
        put_capped(buf, cx, row, right, " ", seg_style);
        cx += 1;

        // The hit-rect is clamped for the same reason the writes are: a rect
        // reaching past the inset makes a click on the NEXT pane land on this
        // pane's tab. A segment pushed entirely off the end gets a zero-width
        // rect, which is what `rects` is already initialised to.
        let seg_end_x = cx.min(right);
        rects[si] = Rect::new(seg_start_x, row, seg_end_x.saturating_sub(seg_start_x), 1);
    }

    // right bracket
    put_capped(buf, cx, row, right, caps.right.as_str(), base);
    cx += 1;

    // Right marker
    if needs_right_marker {
        // space before marker
        put_capped(buf, cx, row, right, " ", base);
        cx += 1;
        for ch in marker.chars() {
            put_capped(buf, cx, row, right, &ch.to_string(), base);
            cx += 1;
        }
    }

    let _ = cx; // suppress unused warning
    rects
}

// ── draw_header_plain + draw_framed ─────────────────────────────────────────────

/// Draw header segments centered on a single plain row (no border brackets),
/// returning per-segment hit-rects. Used for a borderless header (top side off).
/// Segments are joined with two spaces; the active one uses `active` style.
pub fn draw_header_plain(buf: &mut Buffer, row: Rect, segments: &[InsetSegment], base: Style, active: Style) -> Vec<Rect> {
    if segments.is_empty() || row.width == 0 || row.height == 0 {
        return segments.iter().map(|_| Rect::default()).collect();
    }
    let widths: Vec<usize> = segments.iter().map(|s| s.text.chars().count()).collect();
    let sep = 2usize;
    let total: usize = widths.iter().sum::<usize>() + sep * segments.len().saturating_sub(1);
    let avail = row.width as usize;
    let leading = if total <= avail { (avail - total) / 2 } else { 0 };
    let mut cx = row.x + leading as u16;
    // The same bound `render_overflow` respects, for the same reason: `leading`
    // is 0 once the text overruns, and every write below would then march past
    // this row's rect and into the pane beside it (SQ-1127). Borderless headers
    // have no brackets to truncate against, so a title cut here loses a character
    // or two more than the framed path would — which is the deal: a row that
    // stays inside its pane beats a row that reads a little further and then
    // scribbles on its neighbour.
    let right = row.right();
    let mut rects = vec![Rect::default(); segments.len()];
    for (i, seg) in segments.iter().enumerate() {
        if i > 0 {
            for _ in 0..sep {
                put_capped(buf, cx, row.y, right, " ", base);
                cx += 1;
            }
        }
        let style = if seg.active { active } else { base };
        let start = cx;
        for ch in seg.text.chars() {
            put_capped(buf, cx, row.y, right, &ch.to_string(), style);
            cx += 1;
        }
        // Clamped like the framed path's, so a click past the header cannot land
        // on a tab that was never drawn there.
        rects[i] = Rect::new(start, row.y, cx.min(right).saturating_sub(start), 1);
    }
    rects
}

/// The result of drawing a pane frame: where content goes, and (optionally) where
/// the header strip should be drawn and whether that row is a border row.
#[derive(Debug, Clone, Copy)]
pub struct FramedPane {
    pub content: Rect,
    /// Where to draw the header strip, or `None` when no header is shown.
    pub header: Option<Rect>,
    /// True when `header` is a top border row (use `draw_top_inset`); false when it
    /// is a reclaimed plain content row (use `draw_header_plain`).
    pub header_bordered: bool,
}

/// Draw a pane frame via the per-side path, and resolve header placement from `header_on`.
pub fn draw_framed(buf: &mut Buffer, area: Rect, sides: PaneSides, glyphs: &PaneGlyphs, color: Style, header_on: bool) -> FramedPane {
    let frame = draw_pane_frame_sides(buf, area, sides, glyphs, color);
    let top_present = sides.top != BorderStyle::None;
    if !header_on {
        FramedPane { content: frame.content, header: None, header_bordered: false }
    } else if top_present {
        FramedPane { content: frame.content, header: Some(frame.top_inset), header_bordered: true }
    } else {
        // Reclaim the first content row for a plain header; content drops one row.
        let c = frame.content;
        if c.height == 0 {
            FramedPane { content: c, header: None, header_bordered: false }
        } else {
            let header = Rect::new(c.x, c.y, c.width, 1);
            let content = Rect::new(c.x, c.y + 1, c.width, c.height - 1);
            FramedPane { content, header: Some(header), header_bordered: false }
        }
    }
}

// ── build_layer_segments ──────────────────────────────────────────────────────

/// An owned layer-tab segment, produced by `build_layer_segments`.
/// Unlike `InsetSegment<'a>`, this owns its text so it can be returned from a
/// function without a lifetime tying it to a caller-supplied string buffer.
/// Convert to `InsetSegment` via `as_inset()` when passing to `draw_top_inset`.
pub struct LayerSegment {
    pub text: String,
    pub active: bool,
}

impl LayerSegment {
    pub fn as_inset(&self) -> InsetSegment<'_> {
        InsetSegment { text: &self.text, active: self.active }
    }
}

/// Build one `LayerSegment` per entry in `layers`, marking the one matching
/// `active_layer` as active.  The `text` field comes from `label(id)` — callers
/// pass a closure so the tab shows the layer's display name (see
/// `MapGraph::layer_name`) rather than a bare id, matching the in-content
/// layer strip.
pub fn build_layer_segments(
    layers: &[mapper::layer::LayerId],
    active_layer: mapper::layer::LayerId,
    label: impl Fn(mapper::layer::LayerId) -> String,
) -> Vec<LayerSegment> {
    layers
        .iter()
        .map(|&id| LayerSegment { text: label(id), active: id == active_layer })
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;

    #[test]
    fn single_border_perimeter_and_content() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        let area = Rect::new(0, 0, 6, 4);
        let mut buf = Buffer::empty(area);
        let f = draw_pane_frame(&mut buf, area, BorderStyle::Single, &PaneGlyphs::default(), Style::default());
        assert_eq!(buf.cell((0,0)).unwrap().symbol(), "┌");
        assert_eq!(buf.cell((5,0)).unwrap().symbol(), "┐");
        assert_eq!(buf.cell((0,3)).unwrap().symbol(), "└");
        assert_eq!(buf.cell((5,3)).unwrap().symbol(), "┘");
        assert_eq!(f.content, Rect::new(1,1,4,2));
    }

    #[test]
    fn none_border_content_is_full_area() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        let area = Rect::new(0,0,6,4);
        let mut buf = Buffer::empty(area);
        let f = draw_pane_frame(&mut buf, area, BorderStyle::None, &PaneGlyphs::default(), Style::default());
        assert_eq!(f.content, area);
    }

    #[test]
    fn parse_border_style_known_and_unknown() {
        assert!(matches!(parse_border_style("double"), BorderStyle::Double));
        assert!(matches!(parse_border_style("bogus"), BorderStyle::Single));
    }

    #[test]
    fn top_inset_centers_single_title() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        let strip = Rect::new(0,0,20,1);
        let mut buf = Buffer::empty(Rect::new(0,0,20,1));
        let rects = draw_top_inset(&mut buf, strip, &[InsetSegment{text:"ZORK I", active:false}], Style::default(), Style::default(), &InsetCaps::for_border(BorderStyle::Thick));
        let row: String = (0..20).map(|x| buf.cell((x,0)).unwrap().symbol().to_string()).collect();
        assert!(row.contains("ZORK I"));
        // centered: leading filler before the bracket
        assert!(row.find("ZORK I").unwrap() > 3);
        assert_eq!(rects.len(), 1);
    }

    /// SQ-1127: a title too long for its inset stays INSIDE the inset.
    ///
    /// The buffer here is deliberately wider than the strip, which is the whole
    /// point and the reason this went unnoticed: every other case in this module
    /// sizes the buffer to the strip exactly, so `Buffer::cell_mut` clipped the
    /// overrun and the row looked correct. Give the row somewhere to run to and
    /// it runs — over the pane's top-right corner, and on into the next pane.
    #[test]
    fn an_overlong_title_is_clipped_to_its_inset_not_to_the_buffer() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 1));
        // The inset occupies columns 5..17; 5 and 17.. belong to the frame and
        // to whatever is drawn beyond it.
        let strip = Rect::new(5, 0, 12, 1);
        let segs = [InsetSegment { text: "A VERY LONG STORY TITLE INDEED", active: true }];
        let rects = draw_top_inset(
            &mut buf,
            strip,
            &segs,
            Style::default(),
            Style::default(),
            &InsetCaps::for_border(BorderStyle::Thick),
        );
        let sym = |x: u16| buf.cell((x, 0)).unwrap().symbol().to_string();
        let row: String = (0..40).map(sym).collect();
        println!("row: {row:?}");

        for x in (0..strip.x).chain(strip.right()..40) {
            assert_eq!(sym(x), " ", "column {x} is outside the inset and must be untouched: {row:?}");
        }
        assert!(
            row.trim_end().chars().count() <= (strip.right()) as usize,
            "nothing is drawn past the inset's right edge: {row:?}"
        );
        // And the hit-rect cannot invite a click on a column the tab does not own.
        assert!(
            rects[0].right() <= strip.right(),
            "hit-rect {:?} escapes the inset {strip:?}",
            rects[0]
        );
    }

    /// SQ-1127, the borderless half: `draw_header_plain` has no brackets to
    /// truncate against, and its `leading` goes to 0 the moment the text
    /// overruns — after which it wrote straight past its own row.
    #[test]
    fn an_overlong_plain_header_is_clipped_to_its_row_not_to_the_buffer() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 1));
        let row = Rect::new(5, 0, 12, 1);
        let segs = [InsetSegment { text: "A VERY LONG STORY TITLE INDEED", active: true }];
        let rects = draw_header_plain(&mut buf, row, &segs, Style::default(), Style::default());
        let sym = |x: u16| buf.cell((x, 0)).unwrap().symbol().to_string();
        let drawn: String = (0..40).map(sym).collect();
        println!("row: {drawn:?}");

        for x in (0..row.x).chain(row.right()..40) {
            assert_eq!(sym(x), " ", "column {x} is outside the header row: {drawn:?}");
        }
        assert!(rects[0].right() <= row.right(), "hit-rect {:?} escapes {row:?}", rects[0]);
    }

    #[test]
    fn layer_tab_segments_mark_active() {
        // build_layer_segments(layers, active, label) -> Vec<LayerSegment>.
        // The label closure supplies the tab text (real callers pass the layer
        // name); here we map id -> a fake name to prove the text comes from it.
        let names = ["Main", "Cellar", "Attic"];
        let segs = build_layer_segments(&[0, 1, 2], 1, |id| names[id as usize].to_string());
        assert_eq!(segs.len(), 3);
        assert!(segs[1].active);
        assert!(!segs[0].active && !segs[2].active);
        assert_eq!(segs[0].text, "Main", "tab text comes from the label closure, not the id");
        assert_eq!(segs[1].text, "Cellar");
    }

    #[test]
    fn top_inset_overflow_keeps_active_with_marker() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        let strip = Rect::new(0,0,9,1);
        let mut buf = Buffer::empty(Rect::new(0,0,9,1));
        let segs = [InsetSegment{text:"0",active:false},InsetSegment{text:"1",active:true},InsetSegment{text:"2",active:false},InsetSegment{text:"3",active:false}];
        let _ = draw_top_inset(&mut buf, strip, &segs, Style::default(), Style::default(), &InsetCaps::for_border(BorderStyle::Thick));
        let row: String = (0..9).map(|x| buf.cell((x,0)).unwrap().symbol().to_string()).collect();
        assert!(row.contains("1"));      // active shown
        assert!(row.contains("‹") || row.contains("…")); // overflow marker present
    }

    #[test]
    fn pane_sides_all_and_left_right_only() {
        let s = PaneSides::all(BorderStyle::Single);
        assert_eq!(s.top, BorderStyle::Single);
        assert_eq!(s.bottom, BorderStyle::Single);

        // left+right only: vertical bars, no corners, content keeps full height.
        let sides = PaneSides { top: BorderStyle::None, bottom: BorderStyle::None, left: BorderStyle::Single, right: BorderStyle::Single };
        let area = Rect::new(0, 0, 10, 4);
        let mut buf = Buffer::empty(area);
        let frame = draw_pane_frame_sides(&mut buf, area, sides, &PaneGlyphs::default(), Style::default());
        // sides present on every row; corners are NOT drawn (no ┌ at 0,0).
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "│");
        assert_eq!(buf.cell((9, 0)).unwrap().symbol(), "│");
        assert_eq!(buf.cell((0, 3)).unwrap().symbol(), "│");
        // content inset 1 left + 1 right, full height (top/bottom open).
        assert_eq!(frame.content, Rect::new(1, 0, 8, 4));
        // no top border → top_inset has zero height.
        assert_eq!(frame.top_inset.height, 0);
    }

    #[test]
    fn draw_sides_corner_when_two_meet_and_heavier_wins() {
        // top thick + left single → top-left corner uses the thick corner (heavier wins).
        let sides = PaneSides { top: BorderStyle::Thick, bottom: BorderStyle::None, left: BorderStyle::Single, right: BorderStyle::None };
        let area = Rect::new(0, 0, 6, 4);
        let mut buf = Buffer::empty(area);
        let frame = draw_pane_frame_sides(&mut buf, area, sides, &PaneGlyphs::default(), Style::default());
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "┏"); // thick tl corner
        // top row interior is the thick horizontal; left side is the single vertical.
        assert_eq!(buf.cell((3, 0)).unwrap().symbol(), "━");
        assert_eq!(buf.cell((0, 2)).unwrap().symbol(), "│");
        // content inset 1 top + 1 left only.
        assert_eq!(frame.content, Rect::new(1, 1, 5, 3));
        // top present → top_inset spans the top row between the left inset and the right edge.
        assert_eq!(frame.top_inset.y, 0);
        assert_eq!(frame.top_inset.height, 1);
    }

    #[test]
    fn draw_header_plain_centers_without_brackets() {
        let row = Rect::new(0, 0, 11, 1);
        let mut buf = Buffer::empty(Rect::new(0, 0, 11, 1));
        let rects = draw_header_plain(&mut buf, row, &[InsetSegment { text: "AB", active: false }], Style::default(), Style::default());
        let line: String = (0..11u16).map(|x| buf.cell((x, 0)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' ')).collect();
        // "AB" centered, no ┫/┣/┃ brackets anywhere.
        assert!(line.contains("AB"), "got {:?}", line);
        assert!(!line.contains('┫') && !line.contains('┣') && !line.contains('┃'), "no brackets: {:?}", line);
        assert_eq!(rects.len(), 1);
    }

    #[test]
    fn draw_framed_header_placement_matrix() {
        let area = Rect::new(0, 0, 12, 6);

        // header on + top present → header is the top border row (bordered).
        let mut b1 = Buffer::empty(area);
        let f1 = draw_framed(&mut b1, area, PaneSides::all(BorderStyle::Single), &PaneGlyphs::default(), Style::default(), true);
        assert!(f1.header_bordered);
        assert_eq!(f1.header.unwrap().y, 0);
        assert_eq!(f1.content, Rect::new(1, 1, 10, 4));

        // header on + top none → header on reclaimed first content row (plain); content drops a row.
        let sides_no_top = PaneSides { top: BorderStyle::None, bottom: BorderStyle::Single, left: BorderStyle::Single, right: BorderStyle::Single };
        let mut b2 = Buffer::empty(area);
        let f2 = draw_framed(&mut b2, area, sides_no_top, &PaneGlyphs::default(), Style::default(), true);
        assert!(!f2.header_bordered);
        let h2 = f2.header.unwrap();
        assert_eq!(h2.height, 1);
        // content starts one row below the header row.
        assert_eq!(f2.content.y, h2.y + 1);

        // header off → no header; content uses the inner area.
        let mut b3 = Buffer::empty(area);
        let f3 = draw_framed(&mut b3, area, PaneSides::all(BorderStyle::Single), &PaneGlyphs::default(), Style::default(), false);
        assert!(f3.header.is_none());
        assert_eq!(f3.content, Rect::new(1, 1, 10, 4));
    }

    #[test]
    fn glyph_override_beats_style_and_base() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        let area = Rect::new(0, 0, 6, 4);
        let mut buf = Buffer::empty(area);
        let sides = PaneSides { top: BorderStyle::Single, bottom: BorderStyle::Single, left: BorderStyle::Single, right: BorderStyle::Single };
        let glyphs = PaneGlyphs { top: Some("═".into()), tl: Some("╔".into()), ..Default::default() };
        draw_pane_frame_sides(&mut buf, area, sides, &glyphs, Style::default());
        // top edge cell uses the override "═", not the single "─"; tl corner uses "╔"
        assert_eq!(buf.cell((2, 0)).unwrap().symbol(), "═");
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "╔");
        // an un-overridden corner falls back to the adaptive single glyph
        assert_eq!(buf.cell((5, 0)).unwrap().symbol(), "┐");
    }

    #[test]
    fn rounded_border_uses_rounded_corners() {
        assert_eq!(glyphs_for(BorderStyle::Rounded).tl, "╭");
        assert_eq!(glyphs_for(BorderStyle::Rounded).br, "╯");
        assert_eq!(parse_border_style("rounded"), BorderStyle::Rounded);
    }

    #[test]
    fn inset_caps_track_border_style() {
        let d = InsetCaps::for_border(BorderStyle::Double);
        assert_eq!((d.left.as_str(), d.right.as_str(), d.divider.as_str()), ("╡", "╞", "┃"));
        let t = InsetCaps::for_border(BorderStyle::Thick);
        assert_eq!((t.left.as_str(), t.right.as_str(), t.divider.as_str()), ("┫", "┣", "┃"));
        for b in [BorderStyle::Single, BorderStyle::Rounded, BorderStyle::None] {
            let c = InsetCaps::for_border(b);
            assert_eq!((c.left.as_str(), c.right.as_str(), c.divider.as_str()), ("┤", "├", "│"), "{b:?}");
        }
    }

    #[test]
    fn top_inset_single_caps_use_thin_brackets() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        let strip = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
        let rects = draw_top_inset(
            &mut buf,
            strip,
            &[InsetSegment { text: "ZORK I", active: false }],
            Style::default(),
            Style::default(),
            &InsetCaps::for_border(BorderStyle::Single),
        );
        // Single-border caps: left ┤ and right ├, never the thick ┫/┣.
        let r = rects[0];
        let left = r.x - 1;
        let right = r.x + r.width;
        assert_eq!(buf.cell((left, 0)).unwrap().symbol(), "┤");
        assert_eq!(buf.cell((right, 0)).unwrap().symbol(), "├");
    }

}
