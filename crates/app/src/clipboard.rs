//! Text selection + clipboard copy for the story pane (no dependencies).
//!
//! Selection is a linear (browser-style) range from an anchor to a head, stored
//! in **stable absolute wrapped-row coordinates** (a row index into the full
//! wrapped transcript plus a 0-based column within the story band). Because the
//! coordinates are anchored to transcript content rather than screen cells, a
//! selection stays correct across scrolling and can cover text beyond the
//! visible viewport. Copy is delivered via the OSC 52 terminal escape, which
//! works over SSH and needs no clipboard lib.

use ratatui::layout::Rect;

/// A point in the wrapped transcript: absolute wrapped-row index + 0-based column
/// within the story band. Stable across scrolling (unlike screen cells).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub row: usize,
    pub col: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Point,
    pub head: Point,
}

/// Per-frame transcript geometry stashed by render so the mouse handlers and the
/// copy path can map screen cells ↔ absolute wrapped rows. `area` is the body text
/// region (scrollbar gutter already excluded); `first_abs_row` is the absolute
/// wrapped-row index drawn at `area.y`.
#[derive(Debug, Clone, Copy)]
pub struct TranscriptGeom {
    pub area: Rect,
    pub first_abs_row: usize,
    pub total_rows: usize,
}

impl Selection {
    pub fn new(at: Point) -> Self {
        Selection { anchor: at, head: at }
    }
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

fn ordered(a: Point, b: Point) -> (Point, Point) {
    if (a.row, a.col) <= (b.row, b.col) { (a, b) } else { (b, a) }
}

/// Inclusive column span [c0,c1] selected on absolute wrapped `row`, within a story
/// band `width` cells wide (0-based), given `origin` — the screen column at which
/// the row's own selectable content actually starts. Ordinary rows pass `origin =
/// 0`; a row that reserves a leading gutter no glyph occupies (the transcript's
/// Meta/Warning marker column) passes its gutter width, and any part of the span
/// that falls left of it is pulled up to `origin` — so the gutter itself is never
/// selectable and a click inside it behaves as a click on the row's first real
/// cell (SQ-0665). `None` if `row` is outside the selection, or the origin-clamped
/// span is empty (the whole selected part of this row was gutter).
pub fn row_span(width: u16, sel: Selection, row: usize, origin: u16) -> Option<(u16, u16)> {
    if width == 0 { return None; }
    let (s, e) = ordered(sel.anchor, sel.head);
    if row < s.row || row > e.row { return None; }
    let last = width - 1;
    let c0 = if row == s.row { s.col.min(last) } else { 0 };
    let c1 = if row == e.row { e.col.min(last) } else { last };
    let c0 = c0.max(origin.min(last));
    if c0 > c1 { return None; }
    Some((c0, c1))
}

/// True if absolute cell (row, col) is inside the selection. See [`row_span`] for
/// `origin`.
pub fn contains(width: u16, sel: Selection, row: usize, col: u16, origin: u16) -> bool {
    match row_span(width, sel, row, origin) { Some((c0, c1)) => col >= c0 && col <= c1, None => false }
}

/// Extract the selected text from the full set of wrapped-row texts. `rows[i]` is the
/// plain text of absolute wrapped row `i`, already stripped of any leading gutter —
/// `origins[i]` (missing/short entries default to 0) is the screen column at which
/// that text starts, so a screen-column selection can be shifted back to index into
/// it (SQ-0665). Rows joined with `\n`, trailing ws trimmed; a row whose selected
/// span is entirely gutter contributes an empty line, keeping row correspondence
/// with what the highlight painted.
pub fn extract(rows: &[&str], origins: &[u16], width: u16, sel: Selection) -> String {
    let (s, e) = ordered(sel.anchor, sel.head);
    let mut out: Vec<String> = Vec::new();
    for row in s.row..=e.row {
        let origin = origins.get(row).copied().unwrap_or(0);
        match row_span(width, sel, row, origin) {
            Some((c0, c1)) => {
                // Columns are CELLS, not chars: a CJK ideograph or emoji covers two
                // of them, so indexing the row by char number copied a different
                // span than the one highlighted on screen (SQ-0655). And they're
                // relative to the row's own text, which starts at screen column
                // `origin` — a Meta/Warning row's gutter marker occupies the
                // columns before it (SQ-0665).
                let text = rows.get(row).copied().unwrap_or("");
                let line = crate::textwidth::slice_cols(text, (c0 - origin) as usize, (c1 - origin) as usize);
                out.push(line.trim_end().to_string());
            }
            None => out.push(String::new()),
        }
    }
    out.join("\n")
}

/// Standard base64 (RFC 4648) with padding.
pub fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { ALPHABET[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// Build the OSC 52 "set clipboard" escape sequence for `text`.
pub fn osc52_copy_sequence(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64_encode(text.as_bytes()))
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;

    fn p(row: usize, col: u16) -> Point {
        Point { row, col }
    }

    #[test]
    fn base64_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"M"), "TQ==");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
    }

    #[test]
    fn osc52_wraps_base64_in_escape() {
        assert_eq!(osc52_copy_sequence("Man"), "\x1b]52;c;TWFu\x07");
    }

    #[test]
    fn ordered_normalizes_reversed_points() {
        let a = p(5, 3);
        let b = p(2, 7);
        assert_eq!(ordered(a, b), (b, a));
        assert_eq!(ordered(b, a), (b, a));
        // Same row: order by column.
        let c = p(4, 8);
        let d = p(4, 2);
        assert_eq!(ordered(c, d), (d, c));
    }

    #[test]
    fn row_span_first_last_middle() {
        // Selection from (row 1, col 3) to (row 3, col 6), width 10.
        let sel = Selection { anchor: p(1, 3), head: p(3, 6) };
        assert_eq!(row_span(10, sel, 1, 0), Some((3, 9)), "first row from col to right edge");
        assert_eq!(row_span(10, sel, 2, 0), Some((0, 9)), "middle row is full width");
        assert_eq!(row_span(10, sel, 3, 0), Some((0, 6)), "last row from left edge to head col");
    }

    #[test]
    fn row_span_single_row() {
        let sel = Selection { anchor: p(2, 2), head: p(2, 5) };
        assert_eq!(row_span(10, sel, 2, 0), Some((2, 5)));
    }

    #[test]
    fn row_span_out_of_range_is_none() {
        let sel = Selection { anchor: p(1, 0), head: p(3, 0) };
        assert_eq!(row_span(10, sel, 0, 0), None);
        assert_eq!(row_span(10, sel, 4, 0), None);
    }

    #[test]
    fn row_span_clamps_to_width() {
        // Head col 50 but width only 10 → clamp to last col 9.
        let sel = Selection { anchor: p(1, 3), head: p(1, 50) };
        assert_eq!(row_span(10, sel, 1, 0), Some((3, 9)));
        // Width 0 yields None.
        assert_eq!(row_span(0, sel, 1, 0), None);
    }

    #[test]
    fn contains_inside_and_outside() {
        let sel = Selection { anchor: p(1, 3), head: p(3, 6) };
        // Middle row: full width is contained.
        assert!(contains(10, sel, 2, 0, 0));
        assert!(contains(10, sel, 2, 9, 0));
        // First row: before c0 not contained.
        assert!(!contains(10, sel, 1, 2, 0));
        assert!(contains(10, sel, 1, 3, 0));
        // Last row: after head col not contained.
        assert!(!contains(10, sel, 3, 7, 0));
        assert!(contains(10, sel, 3, 6, 0));
        // Row beyond selection not contained.
        assert!(!contains(10, sel, 4, 0, 0));
    }

    // ── SQ-0665: per-row text origin (the transcript's Meta/Warning gutter) ──

    #[test]
    fn row_span_origin_pulls_a_gutter_click_up_to_the_first_text_cell() {
        // A row whose text starts at screen column `origin` (a Meta/Warning
        // row's gutter marker occupies the columns before it) must never
        // report the gutter itself as selectable: a span that starts inside
        // it is pulled up to `origin`.
        let sel = Selection { anchor: p(0, 0), head: p(0, 5) };
        assert_eq!(row_span(10, sel, 0, 0), Some((0, 5)), "origin 0: ordinary row unaffected");
        assert_eq!(row_span(10, sel, 0, 2), Some((2, 5)), "origin 2: pulled up past the gutter");
    }

    #[test]
    fn row_span_origin_spanning_only_the_gutter_is_none() {
        // The whole selected span on this row falls inside the gutter (cols
        // 0..=1, origin 2): nothing on this row to highlight or copy.
        let sel = Selection { anchor: p(0, 0), head: p(0, 1) };
        assert_eq!(row_span(10, sel, 0, 2), None);
    }

    #[test]
    fn row_span_origin_wider_than_the_row_never_panics() {
        // Pathologically narrow row (width 1) with a larger origin: the
        // origin clamps to the row's own last column rather than underflowing.
        let sel = Selection { anchor: p(0, 0), head: p(0, 5) };
        assert_eq!(row_span(1, sel, 0, 5), Some((0, 0)));
    }

    #[test]
    fn extract_single_row_substring() {
        let rows = ["HELLOWORLD"];
        // Select columns 2..=5 → "LLOW".
        let sel = Selection { anchor: p(0, 2), head: p(0, 5) };
        let refs: Vec<&str> = rows.to_vec();
        assert_eq!(extract(&refs, &[], 10, sel), "LLOW");
    }

    #[test]
    fn extract_multi_row() {
        let rows = ["aaaaaaaaaa", "bbbbbbbbbb", "cccccccccc"];
        // From (row 0, col 3) to (row 2, col 4).
        let sel = Selection { anchor: p(0, 3), head: p(2, 4) };
        let refs: Vec<&str> = rows.to_vec();
        let out = extract(&refs, &[], 10, sel);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "aaaaaaa", "first row from col 3 to right edge");
        assert_eq!(lines[1], "bbbbbbbbbb", "middle row is full width");
        assert_eq!(lines[2], "ccccc", "last row from left edge to col 4");
    }

    #[test]
    fn extract_is_measured_in_cells_not_chars() {
        // SQ-0655: selection columns are screen CELLS. "日本語" is 3 chars but 6
        // cells, so indexing the row by char number copied the wrong span — a
        // selection over the first two glyphs came back as the whole line.
        let rows = ["日本語です"]; // 5 glyphs, 10 cells
        let refs: Vec<&str> = rows.to_vec();
        // Cells 0..=3 cover 日本.
        let sel = Selection { anchor: p(0, 0), head: p(0, 3) };
        assert_eq!(extract(&refs, &[], 10, sel), "日本");
        // Cells 4..=7 cover 語で.
        let sel2 = Selection { anchor: p(0, 4), head: p(0, 7) };
        assert_eq!(extract(&refs, &[], 10, sel2), "語で");
        // A span that clips a glyph in half still copies it whole.
        let sel3 = Selection { anchor: p(0, 1), head: p(0, 2) };
        assert_eq!(extract(&refs, &[], 10, sel3), "日本");
        // Mixed widths: "a日b" is cells a=0, 日=1..2, b=3.
        let mixed = ["a日b"];
        let mrefs: Vec<&str> = mixed.to_vec();
        let sel4 = Selection { anchor: p(0, 1), head: p(0, 3) };
        assert_eq!(extract(&mrefs, &[], 10, sel4), "日b");
    }

    #[test]
    fn extract_trims_trailing_whitespace() {
        let rows = ["hi        "];
        let sel = Selection { anchor: p(0, 0), head: p(0, 9) };
        let refs: Vec<&str> = rows.to_vec();
        assert_eq!(extract(&refs, &[], 10, sel), "hi");
    }

    #[test]
    fn extract_row_past_end_is_empty() {
        // Selection spans rows 0..=2 but only one row exists.
        let rows = ["hello"];
        let sel = Selection { anchor: p(0, 0), head: p(2, 4) };
        let refs: Vec<&str> = rows.to_vec();
        let out = extract(&refs, &[], 10, sel);
        let lines: Vec<&str> = out.split('\n').collect();
        assert_eq!(lines[0], "hello");
        assert_eq!(lines[1], "", "missing row yields empty line");
        assert_eq!(lines[2], "", "missing row yields empty line");
    }

    #[test]
    fn extract_shifts_by_the_row_s_own_text_origin() {
        // SQ-0665: `rows[i]` holds ONLY the text — a Meta row's gutter marker
        // is drawn separately and never appears in it — so a screen-column
        // selection has to shift left by that row's origin before it means
        // anything as an index into the row's text, or it reads a span
        // shifted by the gutter width relative to what's on screen.
        let rows = ["hello world"];
        let refs: Vec<&str> = rows.to_vec();
        let origins = [2u16];
        // Screen cols 2..=6 (past the 2-col gutter) cover "hello".
        let sel = Selection { anchor: p(0, 2), head: p(0, 6) };
        assert_eq!(extract(&refs, &origins, 20, sel), "hello");
    }

    #[test]
    fn extract_gutter_anchored_selection_starts_at_the_first_text_cell() {
        // A selection that starts IN the gutter (screen col 0, before the
        // row's text starts at col `origin`) must not copy blank gutter
        // cells — it behaves as if it started at the first text cell.
        let rows = ["hello"];
        let refs: Vec<&str> = rows.to_vec();
        let origins = [2u16];
        let sel = Selection { anchor: p(0, 0), head: p(0, 3) }; // screen cols 0..=3
        // origin 2 pulls the start to col 2 → text cols 0..=1 → "he".
        assert_eq!(extract(&refs, &origins, 20, sel), "he");
    }

    #[test]
    fn extract_row_wholly_inside_the_gutter_is_an_empty_line_not_skipped() {
        // Row 0 is ordinary; row 1's selected span (head col 1, origin 2) is
        // entirely gutter. The empty line for row 1 must still be emitted —
        // not omitted — so the copied text keeps one line per selected row,
        // matching what the highlight painted.
        let rows = ["story line", "meta line"];
        let refs: Vec<&str> = rows.to_vec();
        let origins = [0u16, 2u16];
        let sel = Selection { anchor: p(0, 0), head: p(1, 1) };
        let out = extract(&refs, &origins, 20, sel);
        let lines: Vec<&str> = out.split('\n').collect();
        assert_eq!(lines, vec!["story line", ""]);
    }

    #[test]
    fn is_empty_when_anchor_equals_head() {
        let sel = Selection::new(p(3, 4));
        assert!(sel.is_empty());
        let sel2 = Selection { anchor: p(3, 4), head: p(3, 5) };
        assert!(!sel2.is_empty());
    }
}
