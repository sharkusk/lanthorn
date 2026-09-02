//! `/dump-cells` — the rendered cell buffer as copy-pasteable plain text: the
//! glyphs, and the STYLE of every cell (SQ-0761).
//!
//! Nothing else dumps this. `export-transcript` writes transcript text,
//! `export-svg`/`export-dot`/`export-map` write the map, and `/dump-windows`
//! writes GEOMETRY — which window mapped onto which cells. Every Journey defect
//! chased in the session that asked for this was ultimately *a colour landing in a
//! cell*, and geometry cannot show one:
//!
//!   * a panel fill painting nine rows underneath the menu,
//!   * border cells carrying the fill's colour instead of the frame's,
//!   * a menu label missing from the screen while the cell buffer held it intact.
//!
//! Each cost a round trip through a screenshot. A glyph-only dump would have caught
//! none of them, so the styling is the point here, not a nicety.
//!
//! ## The encoding
//!
//! A compact encoding beats a faithful one — this has to read as text in a chat
//! window at 115x61, where a per-cell table would be seven thousand lines. So:
//!
//!   * one GLYPH row and one STYLE row per terminal row, interleaved so a colour
//!     sits directly under the character it painted;
//!   * a style row is one key character per cell, indexing a LEGEND of the distinct
//!     styles, commonest first, each with its cell count, its bounding box, and —
//!     the line that turns nine mystery rows into one fact — the rows it covers
//!     from end to end;
//!   * no ANSI escapes anywhere. Escapes paste as unreadable soup, which is the
//!     whole reason the screen itself cannot be copied (SQ-0756).
//!
//! ## Graphics
//!
//! Excluded from the grid, but never silently. An uploaded image reaches the
//! terminal through cells that carry a protocol escape rather than a character —
//! kitty puts a whole row's escape in the row's first cell and marks the rest
//! `Skip` — so nothing of the GLYPH those cells once held is visible. Their glyphs
//! read `#` and their rectangles are listed, so a region covered by art is
//! identifiable instead of reading as blank.
//!
//! Their STYLE row is kept, though, and deliberately: placing an image does not
//! touch the colours of the cells it covers, so what a band was painted over is
//! still recorded there — which is precisely how a panel fill under an art strip
//! becomes visible. Halfblock rendering is genuinely made of cells and is dumped as
//! cells, because that IS what the terminal shows.

use ratatui::buffer::{Buffer, Cell, CellDiffOption};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

/// The key characters a style legend draws from, commonest style first.
///
/// `#` is reserved for graphics and `.` for the frame's most common style, so the
/// two reserved marks can never be confused with a legend key.
const KEYS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789\
                      +-=*/\\<>()[]{}!?$%&@~^_|;:,'\"`";

/// The mark for a cell an uploaded image is composited over.
const GRAPHICS_MARK: char = '#';

/// The key given to the single commonest style, so the background of the frame
/// reads as texture rather than as a wall of letters.
const GROUND_KEY: char = '.';

/// The mark shared by every style past [`LEGEND_LIMIT`].
const OVERFLOW_MARK: char = '*';

/// How many styles the legend names individually.
///
/// A frame usually has a couple of dozen. A picture rendered INTO cells does not —
/// halfblocks and sixel give every pair of image pixels its own colour, and the
/// Journey menu frame comes to 736 distinct styles that way, which is more legend
/// than grid and buries the eight that matter. The tail is bucketed instead. It is
/// safe to bucket because the ranking is by cell count and the defects this exists
/// for are runs: a mis-coloured border column is fifty cells and sorts far above
/// the per-pixel noise.
const LEGEND_LIMIT: usize = 48;

/// A style as the dump distinguishes them: everything a terminal cell can carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CellStyle {
    fg: Color,
    bg: Color,
    underline: Color,
    modifier: Modifier,
}

impl CellStyle {
    fn of(cell: &Cell) -> Self {
        CellStyle {
            fg: cell.fg,
            bg: cell.bg,
            underline: cell.underline_color,
            modifier: cell.modifier,
        }
    }
}

/// One image placement to report beside the grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePlacement {
    /// Where it came from — a render-side label, or `cell buffer` when the rect was
    /// recovered from the escape-carrying cells themselves.
    pub label: String,
    /// Cell rect: `(x, y, w, h)`, in the same coordinates as the grid's rulers.
    pub rect: (u16, u16, u16, u16),
}

/// The non-grid facts a dump carries: which frame this is, and what art covers it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DumpMeta {
    /// One line saying which frame the grid below is — the same contract as
    /// `/dump-windows`'s `frame described:` (SQ-0756).
    pub frame: String,
    /// Image placements the RENDERER recorded this frame. Kept beside the rects
    /// recovered from the buffer because the two do not always coincide: a raster
    /// or halfblock backend paints without leaving escape cells behind, and a
    /// placement that was skipped still says where it would have gone.
    pub images: Vec<ImagePlacement>,
}

/// Colour, as short as it can be written and still be exact.
fn color_text(c: Color) -> String {
    match c {
        Color::Reset => "default".to_string(),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Indexed(n) => format!("idx{n}"),
        other => format!("{other:?}").to_lowercase(),
    }
}

/// Attributes as single letters, `-` when there are none.
fn modifier_text(m: Modifier) -> String {
    const NAMES: &[(Modifier, &str)] = &[
        (Modifier::BOLD, "bold"),
        (Modifier::DIM, "dim"),
        (Modifier::ITALIC, "italic"),
        (Modifier::UNDERLINED, "underline"),
        (Modifier::SLOW_BLINK, "blink"),
        (Modifier::RAPID_BLINK, "blink!"),
        (Modifier::REVERSED, "reverse"),
        (Modifier::HIDDEN, "hidden"),
        (Modifier::CROSSED_OUT, "strike"),
    ];
    let set: Vec<&str> = NAMES.iter().filter(|(bit, _)| m.contains(*bit)).map(|(_, n)| *n).collect();
    if set.is_empty() { "-".to_string() } else { set.join("+") }
}

/// Is this cell one an image is composited over rather than a character?
///
/// Three signals, because the graphics backends leave three different traces. The
/// kitty path writes the whole row's escape string into the row's first cell (with
/// a `ForcedWidth(1)`, since the escape's text width is meaningless) and marks the
/// rest of the row `Skip` so the diff never overwrites the placeholders; the
/// placeholder character itself (`U+10EEEE`) rides in that string. Sixel and any
/// other escape-passthrough backend leave an `ESC` in the symbol the same way.
pub fn is_graphics_cell(cell: &Cell) -> bool {
    matches!(cell.diff_option, CellDiffOption::Skip | CellDiffOption::ForcedWidth(_))
        || cell.symbol().contains('\u{10EEEE}')
        || cell.symbol().contains('\u{1b}')
}

/// The character that stands for a cell in the glyph grid.
///
/// The first character of the symbol: a grapheme cluster occupies one cell, and the
/// dump is a grid, so a combining sequence has to collapse to its base. Control
/// characters would break the alignment they are printed into, so they read as `?`.
fn glyph_of(cell: &Cell) -> char {
    let ch = cell.symbol().chars().next().unwrap_or(' ');
    if ch.is_control() { '?' } else { ch }
}

/// Coalesce the graphics cells in `area` into rectangles: per-row runs, merged
/// downward while the run's column span is unchanged.
fn graphics_rects(buf: &Buffer, area: Rect) -> Vec<(u16, u16, u16, u16)> {
    let mut open: Vec<(u16, u16, u16, u16)> = Vec::new(); // still growing downward
    let mut done: Vec<(u16, u16, u16, u16)> = Vec::new();
    for y in area.top()..area.bottom() {
        let mut runs: Vec<(u16, u16)> = Vec::new();
        let mut start: Option<u16> = None;
        for x in area.left()..area.right() {
            let hit = buf.cell((x, y)).is_some_and(is_graphics_cell);
            match (hit, start) {
                (true, None) => start = Some(x),
                (false, Some(s)) => {
                    runs.push((s, x - s));
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(s) = start {
            runs.push((s, area.right() - s));
        }
        let mut next_open = Vec::new();
        for (x, w) in runs {
            match open.iter().position(|r| r.0 == x && r.2 == w && r.1 + r.3 == y) {
                Some(i) => {
                    let mut r = open.remove(i);
                    r.3 += 1;
                    next_open.push(r);
                }
                None => next_open.push((x, y, w, 1)),
            }
        }
        done.append(&mut open);
        open = next_open;
    }
    done.append(&mut open);
    done.sort_by_key(|r| (r.1, r.0));
    done
}

/// The two ruler lines above a grid: tens on top, units below, aligned to `gutter`.
fn rulers(area: Rect, gutter: usize) -> Vec<String> {
    let pad = " ".repeat(gutter);
    let mut tens = String::new();
    let mut units = String::new();
    for x in area.left()..area.right() {
        if x % 10 == 0 {
            tens.push_str(&format!("{x}"));
        }
        // A multi-digit tens label writes its own following columns.
        while tens.chars().count() < (x - area.left()) as usize + 1 {
            tens.push(' ');
        }
        units.push(char::from_digit((x % 10) as u32, 10).unwrap_or('?'));
    }
    vec![format!("{pad}{}", tens.trim_end()), format!("{pad}{units}")]
}

/// Render `area` of `buf` as the plain-text cell dump (SQ-0761).
///
/// The returned lines carry no escape sequences at all, so they survive a copy out
/// of a chat window, a paste into an issue, and a terminal that renders none of it.
pub fn format_cell_dump(buf: &Buffer, area: Rect, meta: &DumpMeta) -> Vec<String> {
    let mut out = Vec::new();
    let area = area.intersection(buf.area);
    out.push(format!(
        "grid: {}x{} cells at ({},{}) — glyphs and per-cell style, no escape codes",
        area.width, area.height, area.x, area.y
    ));
    if !meta.frame.is_empty() {
        out.push(format!("frame: {}", meta.frame));
    }
    if area.is_empty() {
        out.push("(empty area — nothing was drawn)".to_string());
        return out;
    }

    // ── Styles, counted, so the legend can lead with the ground ──────────────
    let mut styles: Vec<CellStyle> = Vec::new();
    let mut index: Vec<Option<usize>> = Vec::with_capacity(area.area() as usize);
    let mut counts: Vec<u32> = Vec::new();
    // Per style: (min x, min y, max x, max y).
    let mut extent: Vec<(u16, u16, u16, u16)> = Vec::new();
    let mut lookup: std::collections::HashMap<CellStyle, usize> = std::collections::HashMap::new();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let Some(cell) = buf.cell((x, y)) else {
                index.push(None);
                continue;
            };
            // A graphics cell keeps its STYLE — placing an image leaves the colours
            // of the cells it covers exactly as they were, so this is where a fill
            // painted underneath a band shows up. Only its glyph is unknowable.
            let st = CellStyle::of(cell);
            let i = *lookup.entry(st).or_insert_with(|| {
                styles.push(st);
                counts.push(0);
                extent.push((x, y, x, y));
                styles.len() - 1
            });
            counts[i] += 1;
            let e = &mut extent[i];
            e.0 = e.0.min(x);
            e.1 = e.1.min(y);
            e.2 = e.2.max(x);
            e.3 = e.3.max(y);
            index.push(Some(i));
        }
    }

    // Commonest first: the frame's ground gets `.` and stops shouting.
    let mut order: Vec<usize> = (0..styles.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(counts[i]), extent[i].1, extent[i].0));
    let named = order.len().min(LEGEND_LIMIT).min(KEYS.len() + 1);
    let mut key_of: Vec<char> = vec![OVERFLOW_MARK; styles.len()];
    for (rank, &i) in order.iter().take(named).enumerate() {
        key_of[i] = match rank {
            0 => GROUND_KEY,
            r => KEYS[r - 1] as char,
        };
    }

    // ── Graphics ─────────────────────────────────────────────────────────────
    let rects = graphics_rects(buf, area);
    if rects.is_empty() && meta.images.is_empty() {
        out.push("graphics: none on this frame".to_string());
    } else {
        out.push(format!(
            "graphics: {} region(s) of the buffer carry an uploaded image — their glyphs read \
             '{GRAPHICS_MARK}' (the image draws over them); their style row still shows what \
             the cells underneath carry",
            rects.len()
        ));
        for (x, y, w, h) in &rects {
            out.push(format!("  {GRAPHICS_MARK} cells ({x},{y}) {w}x{h}"));
        }
        // What the renderer MEANT to place, which a backend that paints without
        // escape cells (raster, halfblocks) leaves no trace of in the buffer.
        for img in &meta.images {
            let (x, y, w, h) = img.rect;
            out.push(format!("  placement recorded by the renderer: {} at ({x},{y}) {w}x{h}", img.label));
        }
    }

    // ── Backgrounds, by row ──────────────────────────────────────────────────
    //
    // The headline the panel-fill investigations kept having to reconstruct from a
    // screenshot: which rows are one colour from end to end. Backgrounds rather than
    // whole styles, because a row with one bold word in it is still that row's fill
    // — and it was the FILL that was painting nine rows under the menu.
    let mut row_bg: Vec<(Color, Vec<u16>)> = Vec::new();
    for y in area.top()..area.bottom() {
        let first = buf.cell((area.left(), y)).map(|c| c.bg);
        let Some(bg) = first else { continue };
        if !(area.left()..area.right()).all(|x| buf.cell((x, y)).is_some_and(|c| c.bg == bg)) {
            continue;
        }
        match row_bg.iter_mut().find(|(c, _)| *c == bg) {
            Some((_, rows)) => rows.push(y),
            None => row_bg.push((bg, vec![y])),
        }
    }
    if row_bg.is_empty() {
        out.push("row backgrounds: no row of this frame is a single background end to end".to_string());
    } else {
        out.push("row backgrounds: rows every cell of which shares one background".to_string());
        row_bg.sort_by_key(|(_, rows)| std::cmp::Reverse(rows.len()));
        for (bg, rows) in &row_bg {
            out.push(format!("  bg {} — rows {}", color_text(*bg), ranges(rows)));
        }
    }

    // ── Legend ───────────────────────────────────────────────────────────────
    out.push(format!(
        "styles: {} distinct, commonest first{}",
        styles.len(),
        if styles.len() > named {
            format!(" ({named} named below, the rest bucketed under '{OVERFLOW_MARK}')")
        } else {
            String::new()
        }
    ));
    for &i in order.iter().take(named) {
        let st = styles[i];
        let (x0, y0, x1, y1) = extent[i];
        // The line the panel-fill defects needed: a style that owns a row from end
        // to end owns that row, and nine of them in a range say so once.
        let full: Vec<u16> = (area.top()..area.bottom())
            .filter(|&y| {
                (area.left()..area.right()).all(|x| {
                    let off = (y - area.top()) as usize * area.width as usize + (x - area.left()) as usize;
                    index.get(off).copied().flatten() == Some(i)
                })
            })
            .collect();
        let full_note = match full.as_slice() {
            [] => String::new(),
            rows => format!("  FULL rows {}", ranges(rows)),
        };
        let ul = if st.underline == Color::Reset {
            String::new()
        } else {
            format!(" underline {}", color_text(st.underline))
        };
        out.push(format!(
            "  {}  fg {} on bg {}  attrs {}{}  {} cells  rows {}-{} cols {}-{}{}",
            key_of[i],
            color_text(st.fg),
            color_text(st.bg),
            modifier_text(st.modifier),
            ul,
            counts[i],
            y0,
            y1,
            x0,
            x1,
            full_note
        ));
    }
    if styles.len() > named {
        let tail = &order[named..];
        let cells: u32 = tail.iter().map(|&i| counts[i]).sum();
        let (mut x0, mut y0, mut x1, mut y1) = extent[tail[0]];
        for &i in tail {
            let e = extent[i];
            x0 = x0.min(e.0);
            y0 = y0.min(e.1);
            x1 = x1.max(e.2);
            y1 = y1.max(e.3);
        }
        out.push(format!(
            "  {OVERFLOW_MARK}  {} further style(s), {cells} cells  rows {y0}-{y1} cols {x0}-{x1} \
             — a long tail like this is a picture rendered INTO cells (halfblocks/sixel), one \
             style per pixel pair",
            tail.len()
        ));
    }

    // ── The grid ─────────────────────────────────────────────────────────────
    let gutter = format!("{}", area.bottom().saturating_sub(1)).len().max(2) + 3;
    out.push(String::new());
    out.push("rows: 'g' is the glyph row, 's' the style keys under it".to_string());
    out.extend(rulers(area, gutter));
    for y in area.top()..area.bottom() {
        let mut glyphs = String::new();
        let mut keys = String::new();
        for x in area.left()..area.right() {
            let Some(c) = buf.cell((x, y)) else {
                glyphs.push(' ');
                keys.push(' ');
                continue;
            };
            glyphs.push(if is_graphics_cell(c) { GRAPHICS_MARK } else { glyph_of(c) });
            let off = (y - area.top()) as usize * area.width as usize + (x - area.left()) as usize;
            keys.push(index[off].map(|i| key_of[i]).unwrap_or(' '));
        }
        let n = format!("{y:>width$}", width = gutter - 3);
        out.push(format!("{n} g|{glyphs}"));
        out.push(format!("{n} s|{keys}"));
    }
    out
}

/// `[3, 4, 5, 9]` → `"3-5,9"`.
fn ranges(rows: &[u16]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        let start = rows[i];
        let mut end = start;
        while i + 1 < rows.len() && rows[i + 1] == end + 1 {
            i += 1;
            end = rows[i];
        }
        parts.push(if start == end { format!("{start}") } else { format!("{start}-{end}") });
        i += 1;
    }
    parts.join(",")
}

/// The last frame drawn without a modal overlay over it, held for `/dump-cells`
/// (SQ-0761).
///
/// Snapshotted rather than read live for the reason SQ-0756 established: the frame
/// standing in front of the renderer when the command runs is the command's own if
/// it was reached through the palette, and a modal drops a v6 pane off its pixel
/// path entirely. Bound to a key (SQ-0759) no modal opens at all and this is simply
/// the frame on screen.
#[derive(Debug, Clone)]
pub struct FrameCells {
    /// The frame's own buffer, clipped to nothing — `buf.area` is the frame.
    pub buf: Buffer,
    /// Frames drawn since, all of them under a modal — how stale this is.
    pub modal_frames_since: u32,
    /// The v6 render path that drew it, if any (`path:hybrid-ring`, …).
    pub path: Option<String>,
    /// Image placements the renderer recorded on that frame.
    pub images: Vec<ImagePlacement>,
}

impl FrameCells {
    /// The whole dump, ready for the transcript and the log.
    pub fn lines(&self) -> Vec<String> {
        let age = match self.modal_frames_since {
            0 => "the frame on screen now".to_string(),
            n => format!("{n} modal frame(s) ago — NOT the palette/dialog frame this command runs in"),
        };
        let path = self.path.clone().unwrap_or_else(|| "no v6 render path recorded".into());
        format_cell_dump(
            &self.buf,
            self.buf.area,
            &DumpMeta {
                frame: format!("the last frame drawn with no modal over it, {age} · {path}"),
                images: self.images.clone(),
            },
        )
    }
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use ratatui::style::Style;

    fn buf_2x2() -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, 2, 2));
        buf.cell_mut((0, 0)).unwrap().set_symbol("A").set_style(Style::new().fg(Color::Rgb(1, 2, 3)));
        buf.cell_mut((1, 0)).unwrap().set_symbol("B").set_style(Style::new().bg(Color::Rgb(9, 9, 9)));
        buf
    }

    #[test]
    fn the_dump_carries_no_escape_sequences() {
        let lines = format_cell_dump(&buf_2x2(), Rect::new(0, 0, 2, 2), &DumpMeta::default());
        assert!(
            lines.iter().all(|l| !l.contains('\u{1b}')),
            "an escape in the dump is the defect this exists to avoid"
        );
    }

    #[test]
    fn the_glyph_row_reads_as_text_and_the_style_row_indexes_the_legend() {
        let lines = format_cell_dump(&buf_2x2(), Rect::new(0, 0, 2, 2), &DumpMeta::default());
        let joined = lines.join("\n");
        assert!(joined.contains("g|AB"), "the glyphs read straight off the grid:\n{joined}");
        // Three distinct styles over four cells: two spaces share the ground.
        assert!(joined.contains("#010203"), "the fg colour is named exactly:\n{joined}");
        assert!(joined.contains("#090909"), "the bg colour is named exactly:\n{joined}");
    }

    #[test]
    fn a_style_that_owns_a_whole_row_says_so_once() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 3));
        for x in 0..4 {
            for y in 1..3 {
                buf.cell_mut((x, y)).unwrap().set_style(Style::new().bg(Color::Rgb(34, 34, 34)));
            }
        }
        let lines = format_cell_dump(&buf, Rect::new(0, 0, 4, 3), &DumpMeta::default());
        let joined = lines.join("\n");
        assert!(
            lines.iter().any(|l| l.contains("bg #222222 — rows 1-2")),
            "the fill's rows are one line of their own:\n{joined}"
        );
        assert!(
            lines.iter().any(|l| l.contains("on bg #222222") && l.contains("FULL rows 1-2")),
            "and the style that owns them says so in the legend:\n{joined}"
        );
    }

    #[test]
    fn graphics_cells_are_excluded_and_their_rect_reported() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 3));
        for y in 0..2 {
            buf.cell_mut((1, y)).unwrap().set_symbol("\u{1b}_Gf=32\u{1b}\\\u{10eeee}");
            buf.cell_mut((2, y)).unwrap().set_diff_option(CellDiffOption::Skip);
        }
        let lines = format_cell_dump(&buf, Rect::new(0, 0, 4, 3), &DumpMeta::default());
        let joined = lines.join("\n");
        assert!(joined.contains("cells (1,0) 2x2"), "the image rect is named:\n{joined}");
        assert!(joined.contains("g| ## "), "its cells are marked, not printed:\n{joined}");
        assert!(!joined.contains('\u{10eeee}'), "no placeholder glyph escapes into the dump");
    }

    #[test]
    fn row_ranges_collapse() {
        assert_eq!(ranges(&[3, 4, 5, 9]), "3-5,9");
        assert_eq!(ranges(&[7]), "7");
    }
}
