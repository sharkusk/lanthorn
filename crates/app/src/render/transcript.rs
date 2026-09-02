//! GAME pane rendering: status line (top), scrolling transcript (middle), input line (bottom).
//!
//! The inventory is shown in a separate docked panel below the input line
//! (see `render::inventory_dock`), not inside this pane.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

// Only test code refers to `Color` bare; production code always spells the
// fully-qualified `ratatui::style::Color` (SQ-0643 removed the last bare
// production usage — the hardcoded search-highlight style).
#[cfg(test)]
use ratatui::style::Color;

use crate::engine::{Introspect, StatusField, StatusModel};
use crate::state::{transcript_filter_matches, AppState, Focus, ParaFmt, StyleRun, TranscriptFilter, TranscriptKind};
use crate::render::wrap_cache::{CellWrapCache, WrapKey, WrapPlan};
use crate::render::paneframe::{draw_framed, BorderStyle};
use super::draw_str_clipped;

/// One wrapped display row: its `text`, `kind`, resolved base `style`, per-run
/// style `runs`, and — for a row that is part of an inline-image band — the
/// `band` geometry to blit (Task 8). Text rows carry `band: None`.
#[derive(Clone)]
pub(crate) struct WrappedRow {
    pub text: String,
    pub kind: TranscriptKind,
    pub style: Style,
    pub runs: Vec<StyleRun>,
    /// For a row that is part of an inline-image band, the geometry to blit
    /// (Task 8). Text rows carry `None`.
    pub band: Option<ImageBand>,
    /// For a row that flows *beside* a left-margin float (SQ-0454): the image
    /// strip to blit at the left margin (`x_off == 0`) after the row's text is
    /// drawn. Unlike `band`, a float row also carries text (already indented past
    /// the picture); a leftover float row taller than its text carries an empty
    /// `text`. `None` for ordinary rows and for band rows.
    pub float: Option<ImageBand>,
}

/// Geometry for one terminal row of an inline-image band: the source `image`,
/// the band's total `cols`x`rows` cell footprint, this row's index `row` in
/// `0..rows`, and the horizontal cell offset `x_off` (nonzero for margin-right).
///
/// The fields are read by the Task 8 blitter (`render/inline_image.rs`); bands
/// are only constructed under `images_enabled`.
#[derive(Clone)]
pub(crate) struct ImageBand {
    pub image: crate::inline_image::InlineImage,
    pub cols: u16,
    pub rows: u16,
    pub row: u16,
    pub x_off: u16,
}

// ── Styles ─────────────────────────────────────────────────────────────────────
//
// Status, normal text, and suggestion styles are read from `state.colors` at
// render time.  The CURSOR style remains a local constant as it is structural
// (REVERSED only, no color content mapped by ColorScheme).

pub const CURSOR_STYLE: Style = Style::new()
    .add_modifier(Modifier::REVERSED);

/// The block-cursor style for the input caret. A cursor is reverse-video of the
/// text it sits on, so when the game has set page colours (`game_input` is
/// `Some`) reverse the resolved input `text_style` — otherwise a bare REVERSED
/// cursor reverses the *theme*, which can render near-invisible on a recoloured
/// page (e.g. a white game background). With no game colour it stays the
/// structural theme cursor. (SQ-0268)
pub(crate) fn cursor_style(text_style: Style, game_input: Option<Style>) -> Style {
    match game_input {
        Some(_) => text_style.add_modifier(Modifier::REVERSED),
        None => CURSOR_STYLE,
    }
}

/// Draw the input caret into `cell`.
///
/// `over_text` is true when the caret sits ON something — mid-line, or over the
/// completion hint's first glyph — in which case the glyph is kept and only the
/// style applies, so the text stays readable while it is edited.
///
/// **The machine's own caret when there is one** (SQ-0873). Not one of the five
/// interpreters measured draws the reverse-video block a terminal front-end
/// gives by default: three shapes across the five, and on two of them the
/// cursor's colour is neither the page nor the ink, so it cannot be built out of
/// the pair either. [`crate::period`] holds the shapes and what a cell grid can
/// and cannot say about them; with no period look in force this is exactly the
/// behaviour it always had.
///
/// **And a look need not state a caret.** The Amiga's and the IBM PC's Version 6
/// interpreters draw the pair on screen reversed — no fixed colour, so nothing to
/// hold in the machine table — and `crate::period` answers `None` for them
/// (SQ-0947). That falls through to the structural arms below, which reverse the
/// live style and are therefore already the machine's caret. Before this, an Amiga
/// v6 launch drew the fixed `#FF7E1C` orange block its *v3* interpreter used, and a
/// DOS v6 one drew its v3 underscore.
fn draw_caret(
    cell: &mut ratatui::buffer::Cell,
    over_text: bool,
    look: Option<zvm::interpreter::PeriodLook>,
    text_style: Style,
    game_input: Option<Style>,
) {
    // `None` is both "no period look" and "a look with no caret of its own", and
    // the structural fallback is the right answer to both.
    let stated = look.and_then(|l| {
        if over_text {
            crate::period::caret_over_text(&l).map(|s| (None, s))
        } else {
            crate::period::caret_cell(&l).map(|(g, s)| (Some(g), s))
        }
    });
    match stated {
        Some((glyph, style)) => {
            if let Some(glyph) = glyph {
                cell.set_symbol(glyph);
            }
            cell.set_style(style);
        }
        None if over_text => {
            cell.set_style(cursor_style(text_style, game_input));
        }
        None => {
            cell.set_symbol(" ").set_style(cursor_style(text_style, game_input));
        }
    }
}

// ── Pure helpers (testable without Machine) ────────────────────────────────────

/// The field values available to status-bar segment templates for one turn.
pub(crate) struct StatusFields {
    pub location: String,
    pub score: Option<String>,
    pub moves: Option<String>,
    pub time: Option<String>,
    pub turns: String,
    pub title: String,
    pub filter: String,
}

fn status_field_value<'a>(f: &'a StatusFields, name: &str) -> &'a str {
    match name {
        "location" => &f.location,
        "score" => f.score.as_deref().unwrap_or(""),
        "moves" => f.moves.as_deref().unwrap_or(""),
        "time" => f.time.as_deref().unwrap_or(""),
        "turns" => &f.turns,
        "title" => &f.title,
        "filter" => &f.filter,
        _ => "", // unknown token → empty
    }
}

/// Resolve a segment's `{placeholder}` template against `f`.
///
/// Returns `Some(resolved)` for a pure-literal segment or one with at least one
/// non-empty placeholder; returns `None` (hide the segment) when the template
/// contains placeholders that ALL resolve to empty. An unterminated `{` is
/// treated as a literal brace.
pub(crate) fn resolve_placeholders(text: &str, f: &StatusFields) -> Option<String> {
    let mut out = String::new();
    let mut had_placeholder = false;
    let mut any_nonempty = false;
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        if let Some(close) = after.find('}') {
            let name = &after[..close];
            had_placeholder = true;
            let val = status_field_value(f, name);
            if !val.is_empty() {
                any_nonempty = true;
            }
            out.push_str(val);
            rest = &after[close + 1..];
        } else {
            out.push('{');
            rest = after;
        }
    }
    out.push_str(rest);
    if had_placeholder && !any_nonempty {
        None
    } else {
        Some(out)
    }
}

/// Pack visible `(text, style, align)` segments into draw ops `(x_col, text, style)`.
///
/// Left cluster packs from the left edge; right cluster packs flush against the
/// right edge; center cluster centers in the gap between them. Truncation when
/// space runs short: drop the center cluster, then truncate the left cluster to
/// the space before the right cluster, preserving the right cluster (clipped only
/// if it alone exceeds the width). `x_col` is relative to the region's left edge.
pub(crate) fn pack_status_clusters(
    visible: &[(String, ratatui::style::Style, crate::colors::Align)],
    width: usize,
) -> Vec<(u16, String, ratatui::style::Style)> {
    use crate::colors::Align;
    let cw = |s: &str| s.chars().count();
    let pick = |a: Align| -> Vec<&(String, ratatui::style::Style, Align)> {
        visible.iter().filter(|(_, _, sa)| *sa == a).collect()
    };
    let left = pick(Align::Left);
    let center = pick(Align::Center);
    let right = pick(Align::Right);
    let sum = |v: &[&(String, ratatui::style::Style, Align)]| v.iter().map(|(t, _, _)| cw(t)).sum::<usize>();
    let left_w = sum(&left);
    let right_w = sum(&right);
    let center_w = sum(&center);

    let mut ops: Vec<(u16, String, ratatui::style::Style)> = Vec::new();

    // RIGHT cluster: flush right, declared order, clipped to the row.
    let right_start = width.saturating_sub(right_w);
    {
        let mut x = right_start;
        for (t, s, _) in &right {
            let avail = width.saturating_sub(x);
            if avail == 0 { break; }
            let txt = truncate_line(t, avail).to_string();
            let adv = cw(&txt);
            ops.push((x as u16, txt, *s));
            x += adv;
        }
    }
    // LEFT cluster: flush left, truncated to the space before the right cluster.
    // This is where the {location} segment sits, so an overlong name breaks at a
    // space and gets an ellipsis (ZMSD §8.2.2.2) instead of a hard clip.
    let left_budget = right_start;
    {
        let mut x = 0usize;
        for (t, s, _) in &left {
            if x >= left_budget { break; }
            let avail = left_budget - x;
            let txt = truncate_status_text(t, avail);
            let adv = cw(&txt);
            ops.push((x as u16, txt, *s));
            x += adv;
        }
    }
    // CENTER cluster: only when it fits in the gap; otherwise dropped.
    let gap_start = left_w;
    let gap_end = right_start;
    if gap_end > gap_start && center_w <= gap_end - gap_start {
        let mut x = gap_start + (gap_end - gap_start - center_w) / 2;
        for (t, s, _) in &center {
            let avail = gap_end.saturating_sub(x);
            if avail == 0 { break; }
            let txt = truncate_line(t, avail).to_string();
            let adv = cw(&txt);
            ops.push((x as u16, txt, *s));
            x += adv;
        }
    }
    ops
}

/// Return the slice of transcript lines visible in `rows` rows, honouring
/// `scroll` (0 = newest at bottom; higher = further back in history).
///
/// The returned slice always has ≤ `rows` entries and is ordered oldest-first
/// so the caller can draw them top-to-bottom.
///
/// Note: the renderer now uses `visible_wrapped_lines` which handles word-wrap.
/// This function is retained for unit testing the slice logic in isolation.
#[cfg(test)]
pub(crate) fn visible_lines(
    transcript: &[String],
    rows: usize,
    scroll: u16,
) -> &[String] {
    if rows == 0 || transcript.is_empty() {
        return &[];
    }
    // Total lines available.
    let n = transcript.len();
    // `scroll` offsets the window upward from the bottom.
    let scroll = scroll as usize;
    // The window ends (exclusive) at n - scroll, clamped to [0, n].
    let end = n.saturating_sub(scroll);
    // The window starts at end - rows, clamped to 0.
    let start = end.saturating_sub(rows);
    &transcript[start..end]
}

/// Truncate a status-bar segment to `width` columns the way ZMSD §8.2.2.2 asks:
/// "If the object's short name exceeds the available room on the status line,
/// the author suggests that an interpreter should break it at the last space and
/// append an ellipsis". We use the single-character ellipsis '…' rather than the
/// spec's three dots so the marker itself costs one column, not three.
///
/// Only applied when the text actually overflows — a segment that fits is
/// returned unchanged, so nothing gains a spurious '…'. A single word longer
/// than `width` (no space to break at) falls back to a hard character break,
/// still marked with the ellipsis.
pub(crate) fn truncate_status_text(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let head: String = text.chars().take(width - 1).collect(); // one column for '…'
    let kept = match head.rfind(' ') {
        Some(i) => head[..i].trim_end(),
        None => head.as_str(),
    };
    format!("{kept}…")
}

/// Truncate `line` to at most `width` characters (not bytes).
pub(crate) fn truncate_line(line: &str, width: usize) -> &str {
    // Find the byte position after `width` chars.
    let byte_pos = line
        .char_indices()
        .nth(width)
        .map(|(i, _)| i)
        .unwrap_or(line.len());
    &line[..byte_pos]
}

/// Word-wrap a single logical line into display rows of at most `width` columns.
///
/// - Tries to break at spaces (word-wrap): the line is split at the last space
///   that allows a row of ≤ `width` chars.
/// - Falls back to hard char-break for words longer than `width`.
/// - An empty line produces a single empty string (preserves blank lines).
/// - Zero width returns the line unsplit.
pub(crate) fn wrap_line(line: &str, width: u16) -> Vec<String> {
    wrap_line_ranges(line, width).into_iter().map(|(s, _, _)| s).collect()
}

/// Like `wrap_line`, but each emitted row carries the `[start, end)` char range
/// it occupies in the *original* `line` (so per-line style runs can be re-based
/// onto wrapped rows). The break space dropped between two rows is excluded from
/// both: row N ends before the space, row N+1 starts after it. Hard-broken long
/// words split into contiguous ranges.
pub(crate) fn wrap_line_ranges(line: &str, width: u16) -> Vec<(String, usize, usize)> {
    wrap_line_ranges_nw(line, width, None)
}

/// [`wrap_line_ranges`] with the Z-machine's `buffer_mode` honoured (ZMSD §7.2.1):
/// `nowrap_from` is the char offset at/after which the game had buffering OFF, so
/// from there on the text must break after the last character that FITS — no
/// word-wrap, no word carried to the next row (a long word splits mid-word, and
/// the break space is kept rather than swallowed).
///
/// Rule for a MIXED line (buffering turned off part-way through): a row
/// word-wraps only while it lies entirely inside the buffered prefix; the first
/// row that reaches into the unbuffered region — and every row after it —
/// char-breaks. This matches the spec's model (buffering is a property of the
/// moment the text was printed) without needing per-word state.
///
/// The fit is measured in display CELLS, not chars (SQ-0662): a double-width
/// CJK/emoji glyph costs two columns, so `width` here is a cell budget and a wide
/// glyph that only half-fits moves WHOLE to the next row. The row's `[start, end)`
/// range stays in CHARS, because that is the coordinate system `StyleRun` uses;
/// only the fitting is in cells. The rest of the body pipeline measures the same
/// way — `draw_str_runs` advances by each glyph's width, and the link map, the
/// coloured-background fill and the selection highlight all derive their columns
/// from `textwidth` — so the wrap, the draw and the copy agree cell for cell.
fn wrap_line_ranges_nw(line: &str, width: u16, nowrap_from: Option<usize>) -> Vec<(String, usize, usize)> {
    let width = width as usize;
    if width == 0 {
        return vec![(line.to_string(), 0, line.chars().count())];
    }
    if line.is_empty() {
        return vec![(String::new(), 0, 0)];
    }

    let mut rows: Vec<(String, usize, usize)> = Vec::new();
    let mut remaining = line;
    let mut base_col = 0usize; // char offset of `remaining`'s start within `line`

    while !remaining.is_empty() {
        // One scan finds the cell budget's end, how many chars fit inside it, and
        // the last space to word-wrap at (see `textwidth::row_break`).
        let br = crate::textwidth::row_break(remaining, width);
        let Some(byte_at_width) = br.overflow else {
            let char_count = remaining.chars().count();
            rows.push((remaining.to_string(), base_col, base_col + char_count));
            break;
        };
        // This row reaches into text printed with buffering off → hard char-break,
        // so the word-wrap point is discarded. The row covers chars
        // `[base_col, base_col + fit_chars)`, which is where the unbuffered region
        // has to be tested against.
        let unbuffered = nowrap_from.is_some_and(|n| n < base_col + br.fit_chars);
        let last_space_before = if unbuffered { None } else { br.last_space };

        if let Some(sp) = last_space_before {
            // Break at the space: take everything before it, skip the space.
            let head = &remaining[..sp];
            let head_chars = head.chars().count();
            rows.push((head.to_string(), base_col, base_col + head_chars));
            // Advance past the space (sp is a byte offset of ' ', so sp+1 is safe for ASCII ' ').
            let next = sp + ' '.len_utf8();
            remaining = &remaining[next..];
            base_col += head_chars + 1; // +1 for the dropped break space
        } else {
            // No space found: hard-break at the cell budget. A wide glyph in a
            // 1-cell column fits nowhere, so take it anyway rather than spin.
            let byte_at_width = force_progress(remaining, byte_at_width);
            let head = &remaining[..byte_at_width];
            let head_chars = head.chars().count();
            rows.push((head.to_string(), base_col, base_col + head_chars));
            remaining = &remaining[byte_at_width..];
            base_col += head_chars;
        }
    }

    if rows.is_empty() {
        rows.push((String::new(), 0, 0));
    }
    rows
}

/// The hard-break offset to actually cut at, guaranteeing the wrap makes progress.
///
/// A cell budget can fit NOTHING — a double-width glyph in a one-column pane — and
/// the fitting prefix is then empty, which would emit an empty row forever. Cut
/// after the first char instead: the glyph overflows its row and the draw clips it,
/// which is the only lossless option in a column too narrow to hold it. (SQ-0662)
fn force_progress(s: &str, byte_at_width: usize) -> usize {
    if byte_at_width > 0 {
        return byte_at_width;
    }
    s.chars().next().map(|c| c.len_utf8()).unwrap_or(0)
}

/// Intersect a logical line's `StyleRun`s with a wrapped row's `[start, end)`
/// source char range, re-basing the surviving spans to the row's own offsets.
/// Empty intersections are dropped (so an unstyled row yields an empty vec).
fn rebase_runs(line_runs: Option<&Vec<StyleRun>>, start: usize, end: usize) -> Vec<StyleRun> {
    let Some(line_runs) = line_runs else {
        return Vec::new();
    };
    let mut out: Vec<StyleRun> = Vec::new();
    for r in line_runs {
        let s = r.start.max(start);
        let e = r.end.min(end);
        if s < e {
            out.push(StyleRun { start: s - start, end: e - start, bits: r.bits, fg: r.fg, bg: r.bg, link: r.link, glk_style: r.glk_style });
        }
    }
    out
}

/// Shift every run in `runs` right by `pad` columns (added leading spaces), so a
/// row's style runs stay aligned with its padded/justified text (SQ-0330).
fn shift_runs(runs: Vec<StyleRun>, pad: u16) -> Vec<StyleRun> {
    if pad == 0 {
        return runs;
    }
    let p = pad as usize;
    runs.into_iter()
        .map(|mut r| {
            r.start += p;
            r.end += p;
            r
        })
        .collect()
}

/// The style run for the margin a left-margin float reserves — the `pad` leading
/// spaces that push a row's prose out past the picture (SQ-0827).
///
/// Those spaces carry no run of their own (`shift_runs` moves the line's runs
/// right past them), so they drew in the row's BASE style while the prose beside
/// them drew on whatever background its own run named. Wherever the two differ,
/// the reserved margin reads as a stripe of a different colour down the picture's
/// flank — reported on Zork Zero under the Amiga profile, where §8.3's machine
/// pair is the base (dark grey) and the game's window-0 page is the prose's
/// (light grey). Give the margin the ground the prose beside it sits on and the
/// stripe is gone.
///
/// Colours only: the returned run copies the prose run's BACKGROUND and nothing
/// else — no bold/reverse bits (a reversed run would paint the margin in ink),
/// no hyperlink (the margin is not part of the link's glyphs), no foreground
/// (the margin is blank). A prose run that names no background (`bg == 0`)
/// yields `None`, so the margin keeps inheriting the base exactly as before —
/// which is every frame off that one machine, and every frame with the game's
/// colours declined, since `draw_str_runs` drops a run's game colour there.
fn margin_ground_run(runs: &[StyleRun], pad: u16) -> Option<StyleRun> {
    if pad == 0 {
        return None;
    }
    let p = pad as usize;
    // The run covering the FIRST prose char (the one the margin abuts), else the
    // row's first run — the ground of the text on this row either way.
    let prose = runs.iter().find(|r| r.start <= p && p < r.end).or_else(|| runs.first())?;
    (prose.bg != 0).then_some(StyleRun {
        start: 0,
        end: p,
        bits: 0,
        fg: 0,
        bg: prose.bg,
        link: 0,
        glk_style: 0,
    })
}

/// Wrap a Story/Input logical `line` into display rows honouring its Glk paragraph
/// layout `pf` (SQ-0330). Returns one `(text, start, end, pad)` per wrapped row:
/// `text` is the padded row (leading spaces already prepended), `[start, end)` are
/// the row's char offsets in the ORIGINAL `line` (for run re-basing), and `pad` is
/// the number of leading spaces added (so callers can shift the row's runs).
///
/// Layout is rendered purely as LEADING-SPACE padding so the drawn text, its runs,
/// and selection/search coordinates stay consistent:
/// - `indent` cells indent every row; `para_indent` adds to the FIRST row only
///   (negative = hanging first line), clamped so a row still fits.
/// - Justification pads within the row's usable width: Centered → half the slack,
///   RightFlush → all of it, LeftFlush/LeftRight (fill) → none (fill is treated as
///   left for now; full inter-word fill is out of scope).
///
/// A default `pf` (left, no indent) reduces to `wrap_line_ranges` with `pad == 0`,
/// so the Z-machine path and un-hinted buffers render byte-identically.
///
/// `pf.nowrap_from` (the game switched `buffer_mode` off) makes the affected rows
/// char-break instead of word-wrap — see [`wrap_line_ranges_nw`].
fn wrap_para_ranges(line: &str, width: u16, pf: ParaFmt) -> Vec<(String, usize, usize, u16)> {
    let nw = pf.nowrap_from.map(|n| n as usize);
    // Fast path: the common no-layout case is exactly the old behaviour (an
    // unbuffered Z-machine line carries no Glk layout, so it lands here too).
    let no_layout = pf.indent == 0 && pf.para_indent == 0 && pf.justify == 0;
    if no_layout || width == 0 {
        return wrap_line_ranges_nw(line, width, nw)
            .into_iter()
            .map(|(t, s, e)| (t, s, e, 0))
            .collect();
    }
    let wmax = width.saturating_sub(1); // always leave room for at least 1 char
    let indent = pf.indent.min(wmax);
    // Row-0 indent = indent + para_indent, clamped into [0, wmax].
    let row0_indent = (indent as i32 + pf.para_indent as i32).clamp(0, wmax as i32) as u16;
    let cont_w = width.saturating_sub(indent).max(1);
    let first_w = width.saturating_sub(row0_indent).max(1);

    // Wrap row 0 at first_w, then the remainder at cont_w, keeping original char
    // offsets so runs re-base correctly.
    let mut ranges: Vec<(String, usize, usize)> = wrap_line_ranges_nw(line, first_w, nw);
    if ranges.len() > 1 && cont_w != first_w {
        let second_start = ranges[1].1;
        let remainder: String = line.chars().skip(second_start).collect();
        ranges.truncate(1);
        // The remainder is re-wrapped in its own coordinates, so rebase `nw` too.
        let rem_nw = nw.map(|n| n.saturating_sub(second_start));
        for (t, s, e) in wrap_line_ranges_nw(&remainder, cont_w, rem_nw) {
            ranges.push((t, s + second_start, e + second_start));
        }
    }

    ranges
        .into_iter()
        .enumerate()
        .map(|(ri, (text, s, e))| {
            let lead = if ri == 0 { row0_indent } else { indent };
            let usable = if ri == 0 { first_w } else { cont_w };
            // Cells (SQ-0662): justification pads a row out to the usable COLUMN
            // count, so a CJK row's own width has to be measured the same way the
            // wrap fitted it — by display width, not by char count.
            let rowlen = crate::textwidth::str_cells(&text) as u16;
            let slack = usable.saturating_sub(rowlen);
            let just_pad = match pf.justify {
                2 => slack / 2,   // Centered
                3 => slack,       // RightFlush
                _ => 0,           // LeftFlush / LeftRight (fill → left for now)
            };
            let pad = lead + just_pad;
            let padded = if pad == 0 { text } else { format!("{}{}", " ".repeat(pad as usize), text) };
            (padded, s, e, pad)
        })
        .collect()
}

/// Like `wrap_line`, but every continuation row after the first is prefixed
/// with `indent` spaces so wrapped text hangs under the first row's content.
pub(crate) fn wrap_line_hanging(line: &str, width: u16, indent: u16) -> Vec<String> {
    let indent = (indent as usize).min(width.saturating_sub(1) as usize);
    if width == 0 || (crate::textwidth::str_cells(line) as u16) <= width {
        return wrap_line(line, width);
    }
    // Wrap the body at the reduced width, then re-prefix continuations.
    let first = wrap_line(line, width);
    let mut out: Vec<String> = Vec::new();
    for (i, row) in first.into_iter().enumerate() {
        if i == 0 {
            out.push(row);
        } else {
            // Re-wrap continuation content within (width - indent) to keep the
            // hang stable, prefixing the indent.
            let pad = " ".repeat(indent);
            for sub in wrap_line(&row, width.saturating_sub(indent as u16)) {
                out.push(format!("{pad}{sub}"));
            }
        }
    }
    out
}

/// Count leading ASCII spaces in `s`.
pub(crate) fn leading_spaces(s: &str) -> u16 {
    s.chars().take_while(|c| *c == ' ').count() as u16
}

/// Columns reserved at the left of a META line for the gutter marker (`▏` + space).
pub(crate) const META_GUTTER: u16 = 2;

/// Screen-column offset (relative to the row's own left edge, i.e. `body_area.x`)
/// at which a row's TEXT actually starts. Meta/Warning rows reserve `META_GUTTER`
/// columns for their marker glyph before the text; Story/Input draw flush left. A
/// `WrappedRow`'s `text` field never includes this prefix — the marker is drawn
/// separately — so anything that maps a screen column onto `text` (the draw loop,
/// the live-input row, the selection highlight, and the clipboard extract) has to
/// shift by this SAME offset. Kept in one place: every one of those reads this
/// function rather than re-deriving `META_GUTTER` from `kind` itself, so drawing
/// and selecting can never drift apart again (SQ-0665).
pub(crate) fn text_origin_col(kind: TranscriptKind) -> u16 {
    match kind {
        TranscriptKind::Meta | TranscriptKind::Warning | TranscriptKind::Assist => META_GUTTER,
        TranscriptKind::Story | TranscriptKind::Input => 0,
    }
}

/// Expand a slice of logical transcript lines into wrapped display rows, carrying
/// each row's `TranscriptKind`. META lines wrap to `width - META_GUTTER` so the
/// gutter marker has room; STORY lines use the full `width`. `kinds` parallels
/// `transcript`; a missing/short entry defaults to `Story`.
///
/// `styles` parallels `transcript`: each logical line's resolved style is
/// carried onto **every** wrapped row it produces, so a line's style is decided
/// once (on the whole logical line) and never re-derived from a wrapped
/// fragment. A missing/short `styles` entry defaults to `Style::default()`.
///
/// Test-facing convenience over [`wrap_lines_kinded_indexed`]; the render paths
/// take the indexed form, whose line index the clear anchor rides on (SQ-0640).
#[cfg(test)]
pub(crate) fn wrap_lines_kinded(
    transcript: &[String],
    kinds: &[TranscriptKind],
    styles: &[Style],
    runs: &[Vec<StyleRun>],
    para: &[ParaFmt],
    images: &[Option<crate::inline_image::InlineImage>],
    char_px: (u16, u16),
    images_enabled: bool,
    left_float: bool,
    width: u16,
) -> Vec<WrappedRow> {
    wrap_lines_kinded_indexed(transcript, kinds, styles, runs, para, images, char_px, images_enabled, left_float, width).0
}

/// [`wrap_lines_kinded`], plus the display row each source line's output STARTS at
/// (`starts[i]`, always `≤ rows.len()`).
///
/// The index exists because the wrap carries state ACROSS lines — a margin float's
/// picture strips ride beside later lines — so "how many rows do lines `[..a]`
/// occupy" cannot be answered by wrapping that prefix on its own: the prefix wrap
/// flushes the float's leftover strips as extra rows, while in the full wrap those
/// same strips are shared with the lines after `a` and add nothing before it. Only
/// the full wrap knows where a line really begins. (SQ-0640)
#[allow(clippy::too_many_arguments)]
pub(crate) fn wrap_lines_kinded_indexed(
    transcript: &[String],
    kinds: &[TranscriptKind],
    styles: &[Style],
    runs: &[Vec<StyleRun>],
    para: &[ParaFmt],
    images: &[Option<crate::inline_image::InlineImage>],
    char_px: (u16, u16),
    images_enabled: bool,
    left_float: bool,
    width: u16,
) -> (Vec<WrappedRow>, Vec<usize>) {
    let mut out: Vec<WrappedRow> = Vec::new();
    let mut starts: Vec<usize> = Vec::with_capacity(transcript.len());
    // The currently-active left-margin float (Zork Zero's drop-cap idiom): its
    // picture occupies the left `indent` columns and the following Story/Input
    // rows wrap beside it. Only ONE float is active at a time; a new image or a
    // non-prose line flushes it first (so the whole picture always renders, even
    // when the text beside it is shorter than the image — or absent). (SQ-0454)
    let mut float: Option<FloatState> = None;
    // Test/measurement callers don't need the pretail carry (SQ-1179's tail
    // repair); a throwaway sink keeps the extend function's one signature.
    let mut pretail: Option<FloatState> = None;
    wrap_lines_kinded_extend(
        &mut out, &mut starts, &mut float, transcript, kinds, styles, runs, para, images, char_px, images_enabled,
        left_float, width, &mut pretail,
    );
    // Finish any float whose picture outran (or had no) text beside it.
    flush_float(&mut out, &mut float);
    (out, starts)
}

/// [`wrap_lines_kinded_indexed`]'s body, resumable: wrap `transcript` into an
/// existing `out`/`starts` carrying an existing `float`, and DO NOT flush.
///
/// This is what makes the wrap cache incremental (SQ-1034). Two things are true
/// of this loop and neither is obvious:
///
/// * the wrap carries state ACROSS lines — an open margin float narrows the rows
///   after it — so an appended line cannot be wrapped in isolation. `float` is
///   that carry, in and out;
/// * the trailing [`flush_float`] is NOT final. A float whose picture outran its
///   text emits its remaining strips as empty rows at the end; the next prose
///   line to arrive rides beside the picture and claims those strips instead. So
///   the cache records how many rows preceded the flush and truncates back to
///   there before extending.
///
/// `starts` indices are absolute rows in `out` and stay valid across an append,
/// because every one of them precedes the flush.
///
/// `pretail` is written on every line processed to the `float` carry ENTERING
/// that line (SQ-1179) — so once the loop finishes it holds the carry entering
/// whichever line was LAST in `transcript`, which is exactly the carry a tail
/// repair needs to resume wrapping from that same point. Left untouched when
/// `transcript` is empty (nothing changed, so nothing to report).
#[allow(clippy::too_many_arguments)]
pub(crate) fn wrap_lines_kinded_extend(
    out: &mut Vec<WrappedRow>,
    starts: &mut Vec<usize>,
    float: &mut Option<FloatState>,
    transcript: &[String],
    kinds: &[TranscriptKind],
    styles: &[Style],
    runs: &[Vec<StyleRun>],
    para: &[ParaFmt],
    images: &[Option<crate::inline_image::InlineImage>],
    char_px: (u16, u16),
    images_enabled: bool,
    left_float: bool,
    width: u16,
    pretail: &mut Option<FloatState>,
) {
    for (i, line) in transcript.iter().enumerate() {
        *pretail = float.clone();
        starts.push(out.len());
        // An image unit either starts a left-margin float or expands into an
        // N-row band (or zero rows when images are disabled). `images.get(i)`
        // yields `None` for a plain text line (and for a short `images` slice).
        if let Some(Some(img)) = images.get(i) {
            if !images_enabled {
                continue;
            }
            // A new picture ends any active float (finishing it as strip rows).
            flush_float(out, float);
            if left_float {
                if let Some(fl) = FloatState::start(img, char_px, width) {
                    *float = Some(fl);
                    continue; // the float emits no rows of its own; text rides beside it
                }
            }
            // Band fallback: inline aligns, margin-right, floats-disabled, or a
            // left-margin image too wide (or too cramped) to float.
            let (cols, rows) = img.fitted_cells(width, char_px);
            let x_off = match img.align {
                crate::inline_image::ImageAlign::MarginRight => width.saturating_sub(cols),
                _ => 0,
            };
            for r in 0..rows {
                out.push(WrappedRow {
                    text: String::new(),
                    kind: TranscriptKind::Story,
                    style: Style::default(),
                    runs: Vec::new(),
                    band: Some(ImageBand { image: img.clone(), cols, rows, row: r, x_off }),
                    float: None,
                });
            }
            continue;
        }

        let kind = kinds.get(i).copied().unwrap_or(TranscriptKind::Story);
        let style = styles.get(i).copied().unwrap_or_default();
        let line_runs = runs.get(i);
        let is_prose = matches!(kind, TranscriptKind::Story | TranscriptKind::Input);
        // A float only wraps the game's own prose. Any other line kind flushes
        // the picture first, then renders normally at full width.
        if !is_prose {
            flush_float(out, float);
        }

        match kind {
            // Meta/Warning are app-generated (always unstyled) and use hanging
            // wrap, whose indentation shifts offsets — emit empty runs.
            TranscriptKind::Meta | TranscriptKind::Warning | TranscriptKind::Assist => {
                let w = width.saturating_sub(META_GUTTER);
                for row in wrap_line_hanging(line, w, leading_spaces(line).max(2)) {
                    out.push(WrappedRow { text: row, kind, style, runs: Vec::new(), band: None, float: None });
                }
            }
            TranscriptKind::Story | TranscriptKind::Input if float.is_some() => {
                // Wrap this prose line beside the active float: the first `rem`
                // output rows are narrowed by the float's `reserve` and carry the
                // next image strip; rows past the picture reclaim full width. The
                // rows are padded by `pad` (nonzero for a LEFT float, pushing text
                // right past the picture; zero for a RIGHT float, flush left) and
                // the strip blits at the float's `x_off`.
                // Copy the geometry up front so the row loop borrows nothing.
                let (image, cols, total, reserve, pad, x_off, start_strip, rem) = {
                    let fl = float.as_ref().unwrap();
                    (fl.image.clone(), fl.cols, fl.rows, fl.reserve, fl.pad, fl.x_off, fl.next_strip, fl.remaining() as usize)
                };
                let narrow = width.saturating_sub(reserve).max(1);
                let nw = para.get(i).and_then(|p| p.nowrap_from).map(|n| n as usize);
                let ranges = wrap_line_ranges_var(line, nw, |k| if k < rem { narrow } else { width });
                let nrows = ranges.len();
                for (k, (text, start, end)) in ranges.into_iter().enumerate() {
                    let (pad, float_band) = if k < rem {
                        let band = ImageBand { image: image.clone(), cols, rows: total, row: start_strip + k as u16, x_off };
                        (pad, Some(band))
                    } else {
                        (0, None)
                    };
                    let padded = if pad == 0 { text } else { format!("{}{}", " ".repeat(pad as usize), text) };
                    // Shift the row's runs right by the leading padding so
                    // selection/copy/search coordinates match the drawn text…
                    let mut runs = shift_runs(rebase_runs(line_runs, start, end), pad);
                    // …and give the margin those spaces occupy the prose's own
                    // ground, so the reserved columns are the page the text sits
                    // on rather than the row's base style (SQ-0827).
                    if let Some(m) = margin_ground_run(&runs, pad) {
                        runs.insert(0, m);
                    }
                    out.push(WrappedRow {
                        text: padded,
                        kind,
                        style,
                        runs,
                        band: None,
                        float: float_band,
                    });
                }
                // Advance the float by the strips just placed; retire it once the
                // whole picture has been laid down.
                let placed = nrows.min(rem) as u16;
                let fl = float.as_mut().unwrap();
                fl.next_strip += placed;
                if fl.remaining() == 0 {
                    *float = None;
                }
            }
            TranscriptKind::Story | TranscriptKind::Input => {
                let pf = para.get(i).copied().unwrap_or_default();
                for (row, start, end, pad) in wrap_para_ranges(line, width, pf) {
                    out.push(WrappedRow {
                        text: row,
                        kind,
                        style,
                        // Shift the row's runs right by the leading padding so
                        // selection/copy/search coordinates match the padded
                        // text that is actually drawn (SQ-0330).
                        runs: shift_runs(rebase_runs(line_runs, start, end), pad),
                        band: None,
                        float: None,
                    });
                }
            }
        }
    }
}

/// The minimum prose column (cells) worth floating a picture beside; a narrower
/// column falls back to a full-width band.
const FLOAT_MIN_TEXT_COLS: u16 = 8;

/// An in-progress margin float while `wrap_lines_kinded` walks the transcript:
/// the source `image`, its cell footprint (`cols` wide × `rows` tall), and how
/// its rows lay out. The float side is expressed by three geometry fields rather
/// than an enum:
/// - LEFT float (Zork Zero's drop-cap): `pad == reserve` pushes text right past
///   the picture, `x_off == 0` blits it at the left.
/// - RIGHT float (Shogun's opening picture, ZMSD §15 margin picture): `pad == 0`
///   keeps text flush left, `x_off` blits the picture at the right edge.
///
/// Either way the wrap width on covered rows is `width - reserve`. (SQ-0454/0489)
///
/// `Clone` because it is the wrap's CARRY: the incremental cache stores the float
/// still open after the last line it consumed and hands a copy back on the next
/// append (SQ-1034). Without it an appended prose line would wrap at full width
/// beside a picture it cannot see.
#[derive(Clone)]
pub(crate) struct FloatState {
    image: crate::inline_image::InlineImage,
    cols: u16,
    rows: u16,
    /// Columns removed from the text width on covered rows.
    reserve: u16,
    /// Leading pad added to covered rows' text (left float pushes text right).
    pad: u16,
    /// Image band x offset (right float right-aligns the picture).
    x_off: u16,
    next_strip: u16,
}

impl FloatState {
    /// Begin a float for a `MarginLeft` or `MarginRight` image, or `None` to fall
    /// back to a band: any other align, or a picture that leaves no prose column.
    fn start(img: &crate::inline_image::InlineImage, char_px: (u16, u16), width: u16) -> Option<FloatState> {
        let (cols, rows) = img.fitted_cells(width, char_px);
        if cols == 0 || rows == 0 {
            return None;
        }
        match img.align {
            crate::inline_image::ImageAlign::MarginLeft => {
                // Starve-guard: a left drop-cap wider than ~half the viewport
                // falls back to a band (the historic SQ-0454 rule).
                if cols.saturating_mul(2) > width {
                    return None;
                }
                let indent = float_text_indent(img, char_px.0, cols);
                if width.saturating_sub(indent) < FLOAT_MIN_TEXT_COLS {
                    return None; // no room for text beside the picture
                }
                Some(FloatState { image: img.clone(), cols, rows, reserve: indent, pad: indent, x_off: 0, next_strip: 0 })
            }
            crate::inline_image::ImageAlign::MarginRight => {
                // Reserve the picture's own cell width plus a one-column gutter;
                // the picture right-aligns and text stays flush left. Fall back to
                // a band when that leaves no usable prose column.
                let reserve = (cols + 1).min(width);
                if width.saturating_sub(reserve) < FLOAT_MIN_TEXT_COLS {
                    return None;
                }
                Some(FloatState { image: img.clone(), cols, rows, reserve, pad: 0, x_off: width.saturating_sub(cols), next_strip: 0 })
            }
            _ => None,
        }
    }

    /// Strip rows not yet placed.
    fn remaining(&self) -> u16 {
        self.rows.saturating_sub(self.next_strip)
    }

    /// The band geometry for strip `row` of this float, blitted at its `x_off`.
    fn strip(&self, row: u16) -> ImageBand {
        ImageBand { image: self.image.clone(), cols: self.cols, rows: self.rows, row, x_off: self.x_off }
    }
}

/// The text indent (in cells) for a left-margin float: the game's own
/// `set_margins` value (`margin_px`, in GAME pixels — scaled the same way the
/// picture is, then rounded up to whole cells) when present, else the picture's
/// cell width plus a one-column gutter. Never less than the picture width, so
/// prose can't overlap the image. (SQ-0454)
fn float_text_indent(img: &crate::inline_image::InlineImage, cell_w: u16, cols: u16) -> u16 {
    let cell_w = cell_w.max(1) as u32;
    let indent = match img.margin_px {
        Some(margin) => {
            // `scaled` already encodes the picture's factor; derive the same one
            // and apply it to the game-pixel margin so text lines up with the
            // scaled picture. Since SQ-1002 that factor is the TEXT's rate
            // (`device_cell / 8`), so this resolves to the game's own margin in
            // native character cells — which is what it was authored as.
            let native_w = img.pixels.width().max(1);
            let scaled_w = img.scaled.map(|(w, _)| w).unwrap_or(native_w).max(1);
            let scaled_margin = (margin as u64 * scaled_w as u64 / native_w as u64) as u32;
            scaled_margin.div_ceil(cell_w).max(1) as u16
        }
        None => cols + 1,
    };
    indent.max(cols)
}

/// Emit a float's not-yet-placed strips as empty rows so its whole picture
/// renders even when the text beside it is shorter than the image (or absent),
/// then clear the float. (SQ-0454)
pub(crate) fn flush_float(out: &mut Vec<WrappedRow>, float: &mut Option<FloatState>) {
    if let Some(fl) = float.take() {
        for r in fl.next_strip..fl.rows {
            out.push(WrappedRow {
                text: String::new(),
                kind: TranscriptKind::Story,
                style: Style::default(),
                runs: Vec::new(),
                band: None,
                float: Some(fl.strip(r)),
            });
        }
    }
}

/// Word-wrap `line` into rows where output row `k` may use a different width
/// `width_for(k)` (clamped to ≥ 1). Like [`wrap_line_ranges`] but the usable
/// width can shrink or grow per row — used to wrap prose beside a left-margin
/// float (narrow while the picture is present, full width once it ends). Each
/// row carries its `[start, end)` char range in the original `line` so per-line
/// style runs can be re-based onto the wrapped rows. (SQ-0454)
///
/// `nowrap_from` char-breaks the rows that reach into unbuffered text, exactly as
/// in [`wrap_line_ranges_nw`] (ZMSD §7.2.1).
fn wrap_line_ranges_var(line: &str, nowrap_from: Option<usize>, width_for: impl Fn(usize) -> u16) -> Vec<(String, usize, usize)> {
    if line.is_empty() {
        return vec![(String::new(), 0, 0)];
    }
    let mut rows: Vec<(String, usize, usize)> = Vec::new();
    let mut remaining = line;
    let mut base_col = 0usize; // char offset of `remaining`'s start within `line`

    while !remaining.is_empty() {
        let width = width_for(rows.len()).max(1) as usize;
        // Cells, not chars (SQ-0662) — same measurement as `wrap_line_ranges_nw`.
        let br = crate::textwidth::row_break(remaining, width);
        let Some(byte_at_width) = br.overflow else {
            let char_count = remaining.chars().count();
            rows.push((remaining.to_string(), base_col, base_col + char_count));
            break;
        };
        // Last space to break at (see `wrap_line_ranges`); suppressed once the row
        // reaches unbuffered text (hard char-break).
        let unbuffered = nowrap_from.is_some_and(|n| n < base_col + br.fit_chars);
        let last_space_before = if unbuffered { None } else { br.last_space };
        if let Some(sp) = last_space_before {
            let head = &remaining[..sp];
            let head_chars = head.chars().count();
            rows.push((head.to_string(), base_col, base_col + head_chars));
            let next = sp + ' '.len_utf8();
            remaining = &remaining[next..];
            base_col += head_chars + 1; // +1 for the dropped break space
        } else {
            let byte_at_width = force_progress(remaining, byte_at_width);
            let head = &remaining[..byte_at_width];
            let head_chars = head.chars().count();
            rows.push((head.to_string(), base_col, base_col + head_chars));
            remaining = &remaining[byte_at_width..];
            base_col += head_chars;
        }
    }
    if rows.is_empty() {
        rows.push((String::new(), 0, 0));
    }
    rows
}

/// Wrapped-row count before the screen-clear anchor `clear_anchor` (a source-line
/// index into the given slices), or `None` when the anchor is unset or out of
/// range. This is the number of display rows that precede the post-clear content,
/// used to top-anchor it. (SQ-0305)
///
/// Derived from the FULL wrap, not from a standalone wrap of `[..a]` (SQ-0640):
/// a margin float's picture strips ride beside the lines that follow it, so a
/// prefix wrap flushes strips as extra rows that the full wrap never spends there.
/// The over-count then pushed the anchor past the post-clear content's real first
/// row — at worst to `rows.len()`, which top-anchored an EMPTY viewport at scroll 0.
///
/// The render paths wrap once and call [`anchor_row_at`] on that wrap's line index
/// instead of paying for a second wrap here.
#[cfg(test)]
pub(crate) fn anchor_wrapped_rows(
    transcript: &[String],
    kinds: &[TranscriptKind],
    styles: &[Style],
    runs: &[Vec<StyleRun>],
    para: &[ParaFmt],
    images: &[Option<crate::inline_image::InlineImage>],
    char_px: (u16, u16),
    images_enabled: bool,
    left_float: bool,
    width: u16,
    clear_anchor: Option<usize>,
) -> Option<usize> {
    clear_anchor?;
    let (rows, starts) = wrap_lines_kinded_indexed(
        transcript, kinds, styles, runs, para, images, char_px, images_enabled, left_float, width,
    );
    anchor_row_at(&starts, rows.len(), clear_anchor)
}

/// The display row the post-clear content starts at: the row source line
/// `clear_anchor` begins on in the full wrap (`starts`, from
/// [`wrap_lines_kinded_indexed`]). `None` when the anchor is unset or past the end
/// of the transcript. Clamped to `total` so it can never index past the rows.
/// (SQ-0640)
///
/// An anchor sitting exactly AT the end (`a == starts.len()`) is not out of range:
/// it is a screen the game has cleared and printed nothing into yet — every row
/// precedes it, so the post-clear screen is *empty*. That is one row past the last
/// `starts` entry, so a bare `get` returned `None` and the viewport fell back to
/// bottom-sticking the very scrollback the game just erased. Beyond Zork's title
/// repaint is exactly that turn: `erase_window(-1)`, `split_window(20)`, the whole
/// centred title placed in the upper window, and not one character printed below
/// it (SQ-0748).
pub(crate) fn anchor_row_at(starts: &[usize], total: usize, clear_anchor: Option<usize>) -> Option<usize> {
    let a = clear_anchor?;
    if a == starts.len() {
        return Some(total);
    }
    starts.get(a).map(|&r| r.min(total))
}

/// Window the fully wrapped `display_rows` down to the `rows` rows visible at
/// `scroll` (0 = newest at bottom; higher = further back). `anchor_row` is the
/// pre-computed [`anchor_row_at`] value; when present and the view is at the
/// bottom (`scroll == 0`) and the post-clear content still fits, those lines are
/// pinned to the TOP (returning fewer than `rows` rows → the caller leaves the
/// rest blank) instead of bottom-sticking, which would pull pre-clear history
/// back into view. Older lines above the anchor stay reachable by scrolling up;
/// once post-clear content overflows the viewport this no longer triggers.
///
/// Returns (visible rows oldest-first, total wrapped-row count, first visible
/// absolute row). This does NOT wrap — the wrapping is done once by the caller
/// (cached across frames), so windowing/scroll is cheap. (SQ-0305)
pub(crate) fn window_wrapped_rows(
    display_rows: &[WrappedRow],
    anchor_row: Option<usize>,
    rows: usize,
    scroll: u16,
) -> (Vec<WrappedRow>, usize, usize) {
    if rows == 0 || display_rows.is_empty() {
        return (Vec::new(), 0, 0);
    }
    let n = display_rows.len();
    if scroll == 0 {
        // Clamped: the anchor indexes THESE rows, and an anchor past their end would
        // slice out of range rather than merely mis-anchor (SQ-0640).
        if let Some(anchor_row) = anchor_row.map(|a| a.min(n)) {
            if n.saturating_sub(anchor_row) <= rows {
                return (display_rows[anchor_row..n].to_vec(), n, anchor_row);
            }
        }
    }
    // Clamp scroll so it never exceeds the top: past `n - rows` the window would
    // otherwise shrink from the bottom, blanking viewport rows.
    let max_scroll = n.saturating_sub(rows);
    let scroll = (scroll as usize).min(max_scroll);
    let end = n.saturating_sub(scroll);
    let start = end.saturating_sub(rows);
    (display_rows[start..end].to_vec(), n, start)
}

/// Return the **wrapped** display rows (with kinds) visible in `rows` rows,
/// honouring `scroll` (0 = newest at bottom; higher = further back in history).
///
/// The returned vec is ordered oldest-first so the caller can draw top-to-bottom.
/// Returns the visible window of wrapped rows AND the total wrapped-row count
/// (so callers can size a scrollbar without re-wrapping).
///
/// This wraps the whole slice every call; the main transcript pane instead caches
/// the wrapped product (see `AppState::transcript_wrap`) and calls
/// [`window_wrapped_rows`] directly. This entry point is retained for secondary
/// buffer windows and unit tests.
pub(crate) fn visible_wrapped_lines_kinded(
    transcript: &[String],
    kinds: &[TranscriptKind],
    styles: &[Style],
    runs: &[Vec<StyleRun>],
    para: &[ParaFmt],
    images: &[Option<crate::inline_image::InlineImage>],
    char_px: (u16, u16),
    images_enabled: bool,
    rows: usize,
    scroll: u16,
    width: u16,
    clear_anchor: Option<usize>,
) -> (Vec<WrappedRow>, usize, usize) {
    if rows == 0 || transcript.is_empty() {
        return (Vec::new(), 0, 0);
    }
    // Secondary buffer windows (the only caller besides tests) keep the legacy
    // band rendering for margin images — left-margin floats are a main-transcript
    // affordance (SQ-0454), so `left_float` is off here.
    let (display_rows, starts) =
        wrap_lines_kinded_indexed(transcript, kinds, styles, runs, para, images, char_px, images_enabled, false, width);
    // Only the bottom (scroll == 0) view can top-anchor. The anchor comes out of
    // THIS wrap's line index (SQ-0640) — no second wrap, and no float-carry skew.
    let anchor_row = if scroll == 0 {
        anchor_row_at(&starts, display_rows.len(), clear_anchor)
    } else {
        None
    };
    window_wrapped_rows(&display_rows, anchor_row, rows, scroll)
}

/// Draw `text` at `(x, y)` into `buf`, using `base_style` for normal characters
/// and `highlight_style` for every case-insensitive occurrence of `query`.
/// Advances `x` by the number of characters drawn. Does not exceed `clip_area`.
///
/// The implementation builds a lowered copy of `text` alongside a map from
/// lowered-byte-offset to original-byte-offset so that match positions found in
/// the lowered string can be safely mapped back to char boundaries in `text`.
/// This is necessary because `to_lowercase()` is not byte-length-preserving
/// (e.g. Turkish dotted-I U+0130 expands from 2 bytes to 3 bytes), so naive
/// offset reuse can produce non-char-boundary panics.
///
/// Retained as the reference renderer for the search-highlight path: production
/// drawing now goes through `draw_str_runs` (which `highlight_mask` keeps
/// consistent with this function), and the tests assert the two stay identical.
#[cfg(test)]
fn draw_str_highlighted(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    text: &str,
    base_style: Style,
    query_lower: &str,
    highlight_style: Style,
    clip_area: ratatui::layout::Rect,
) {
    if query_lower.is_empty() {
        draw_str_clipped(buf, x, y, text, base_style, clip_area);
        return;
    }

    // Build a lowered string and a map: for each source char, record
    // (lowered_byte_start, original_byte_start).  A sentinel entry is appended
    // for the end of both strings.
    let mut tl = String::with_capacity(text.len() + 4);
    let mut map: Vec<(usize, usize)> = Vec::with_capacity(text.chars().count() + 1);
    let mut ob = 0usize;
    for ch in text.chars() {
        map.push((tl.len(), ob));
        for lc in ch.to_lowercase() {
            tl.push(lc);
        }
        ob += ch.len_utf8();
    }
    map.push((tl.len(), text.len())); // sentinel

    // Convert a lowered-byte-offset to the nearest original-byte-offset.
    // For a START offset we round DOWN (largest entry with lowered_start <= L).
    // For an END offset we round UP (smallest entry with lowered_start >= L).
    let orig_start_for = |l: usize| -> usize {
        // Binary search for the largest map entry with .0 <= l.
        let idx = map.partition_point(|&(lb, _)| lb <= l);
        // idx is the first entry AFTER l; step back one.
        let i = if idx > 0 { idx - 1 } else { 0 };
        map[i].1
    };
    let orig_end_for = |l: usize| -> usize {
        // Binary search for the smallest map entry with .0 >= l.
        let idx = map.partition_point(|&(lb, _)| lb < l);
        map[idx].1
    };

    let mut cursor_x = x;
    let mut search_from = 0usize; // byte index into tl

    while search_from < tl.len() {
        if let Some(rel) = tl[search_from..].find(query_lower) {
            let tl_match_start = search_from + rel;
            let tl_match_end   = tl_match_start + query_lower.len();

            // Map lowered offsets back to original char boundaries.
            let orig_ms = orig_start_for(tl_match_start);
            let orig_me = orig_end_for(tl_match_end);
            let orig_ss = orig_start_for(search_from);

            // Draw the non-matching prefix.
            let prefix = &text[orig_ss..orig_ms];
            if !prefix.is_empty() {
                draw_str_clipped(buf, cursor_x, y, prefix, base_style, clip_area);
                cursor_x = cursor_x.saturating_add(prefix.chars().count() as u16);
            }
            // Draw the matching segment.
            let matched = &text[orig_ms..orig_me];
            draw_str_clipped(buf, cursor_x, y, matched, highlight_style, clip_area);
            cursor_x = cursor_x.saturating_add(matched.chars().count() as u16);

            search_from = tl_match_end;
        } else {
            // No more matches: draw the rest with the base style.
            let orig_ss = orig_start_for(search_from);
            draw_str_clipped(buf, cursor_x, y, &text[orig_ss..], base_style, clip_area);
            break;
        }
    }
}

/// Mark which char positions in `text` fall inside a case-insensitive match of
/// `query_lower`. Mirrors `draw_str_highlighted`'s lowered-string matching so the
/// two stay consistent. Returns one bool per char in `text`.
fn highlight_mask(text: &str, query_lower: &str) -> Vec<bool> {
    let nchars = text.chars().count();
    let mut mask = vec![false; nchars];
    if query_lower.is_empty() {
        return mask;
    }
    // Build a lowered copy and record each source char's lowered byte-start.
    let mut tl = String::with_capacity(text.len() + 4);
    let mut char_lb: Vec<usize> = Vec::with_capacity(nchars + 1);
    for ch in text.chars() {
        char_lb.push(tl.len());
        for lc in ch.to_lowercase() {
            tl.push(lc);
        }
    }
    char_lb.push(tl.len()); // sentinel

    let qlen = query_lower.len();
    let mut from = 0usize;
    while from < tl.len() {
        if let Some(rel) = tl[from..].find(query_lower) {
            let ms = from + rel;
            let me = ms + qlen;
            // Mark every source char whose lowered span overlaps [ms, me).
            for i in 0..nchars {
                if char_lb[i] < me && ms < char_lb[i + 1] {
                    mask[i] = true;
                }
            }
            from = me;
        } else {
            break;
        }
    }
    mask
}

/// Draw `s` at `(x, y)` advancing by each glyph's DISPLAY width rather than one
/// cell per char, and writing whole grapheme clusters into a cell (so a combining
/// mark or a ZWJ emoji stays with its base). The trailing cell of a double-width
/// glyph is blanked in the same style — ratatui's own convention, and what keeps
/// the terminal's wide-glyph cell skip from swallowing the next character.
///
/// Returns the cells consumed, so the caller can place what follows (the caret,
/// the completion hint) at a real column. For pure ASCII this is cell-for-cell
/// identical to [`draw_str_clipped`]. (SQ-0655)
///
/// Used by the input line, whose caret and click-to-caret mapping
/// (`AppState::input_click_index`) are in cells. The transcript body measures the
/// same way (SQ-0662) but draws through `draw_str_runs`, which has to keep the
/// per-CHAR `StyleRun` indexing the wrap hands it.
fn draw_str_cells(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    s: &str,
    style: Style,
    area: ratatui::layout::Rect,
) -> u16 {
    use unicode_segmentation::UnicodeSegmentation;
    if y < area.y || y >= area.bottom() {
        return 0;
    }
    let mut cx = x;
    for g in s.graphemes(true) {
        if cx >= area.right() {
            break;
        }
        let w = crate::textwidth::str_cells(g).max(1) as u16;
        if cx >= area.x {
            if let Some(cell) = buf.cell_mut((cx, y)) {
                // Control chars are blanked, as `draw_char_clipped` does: game and
                // paste text is untrusted and ratatui's debug build asserts on them.
                if g.chars().all(|c| c.is_control()) {
                    cell.set_symbol(" ");
                } else {
                    cell.set_symbol(g);
                }
                cell.set_style(style);
            }
            for k in 1..w {
                if cx + k >= area.right() {
                    break;
                }
                if let Some(cell) = buf.cell_mut((cx + k, y)) {
                    cell.set_symbol(" ").set_style(style);
                }
            }
        }
        cx = cx.saturating_add(w);
    }
    cx.saturating_sub(x)
}

/// Draw `text` at `(x, y)` applying per-char style: `base_style` plus the bits of
/// the `StyleRun` covering that char, and its resolved fg/bg colours. When
/// `search` is `Some((query_lower, highlight_style))`, characters inside a query
/// match use `highlight_style` instead (the search affordance wins over game
/// styling). With empty `runs` and no search this is byte-identical to
/// `draw_str_clipped`; with empty `runs` and a search it matches
/// `draw_str_highlighted`.
///
/// `ink` ([`crate::render::TextInk`]) carries the two facts a run's colours resolve
/// against, which always come from the same place and so travel together (SQ-1028):
/// the theme, always supplied (palette + per-Glk-style theme slots), and `honor`,
/// which gates the GAME's own run colours (garglk `stylehint 0/1`) — when off, a
/// run's game-set fg/bg is IGNORED, but the theme slot and element base still apply
/// (SQ-0331). Style bits (bold/italic/reverse) and the hyperlink affordance are
/// unaffected by `honor`, exactly as before.
///
/// Columns advance by each glyph's DISPLAY WIDTH (SQ-0662), so a CJK/emoji glyph
/// occupies two cells — its trailing cell blanked in the same style, ratatui's own
/// convention — and a zero-width scalar (combining mark, ZWJ) is appended to the
/// cell of the glyph it modifies instead of claiming a column of its own. `runs`
/// and the search mask stay indexed by CHAR, the coordinate the wrap re-bases them
/// in; only the column each char lands on is a width. For pure ASCII this is
/// cell-for-cell what the old one-char-per-cell loop drew.
pub(crate) fn draw_str_runs(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    text: &str,
    base_style: Style,
    runs: &[StyleRun],
    search: Option<(&str, Style)>,
    area: ratatui::layout::Rect,
    ink: crate::render::TextInk,
) {
    if y < area.y || y >= area.bottom() {
        return;
    }
    let (scheme, honor) = (ink.colors(), ink.honor());
    let hi: Vec<bool> = match search {
        Some((q, _)) if !q.is_empty() => highlight_mask(text, q),
        _ => Vec::new(),
    };
    let mut col = x;
    // The cell the last real glyph went into, so a following combining mark can
    // join it rather than overwrite the column with a bare mark.
    let mut glyph_col: Option<u16> = None;
    for (i, ch) in text.chars().enumerate() {
        if col >= area.right() {
            break;
        }
        let style = if hi.get(i).copied().unwrap_or(false) {
            search.unwrap().1
        } else {
            let run = runs.iter().find(|r| i >= r.start && i < r.end);
            let bits = run.map(|r| r.bits).unwrap_or(0);
            let mut s = crate::render::apply_text_style(base_style, bits);
            // Per-channel colour resolution (SQ-0331): game-set run colour (gated
            // by `honor`), then the theme's per-Glk-style slot (buffer = row 0),
            // then the element base (`base_style`). fg/bg are logical (pre-reverse);
            // apply_text_style already set REVERSED for bit 1, so the terminal
            // performs exactly one swap.
            {
                use crate::state::unpack_zcolour;
                use zvm::screen::ZColour;
                use crate::render::resolve_glk_channel;
                let game = |packed: u32| -> Option<ratatui::style::Color> {
                    let z = unpack_zcolour(packed);
                    (!matches!(z, ZColour::Default)).then(|| crate::render::resolve_zcolour(z, scheme))
                };
                let glk = run.map(|r| r.glk_style as usize).unwrap_or(0);
                let slot = scheme.glk_styles[0].get(glk).copied().unwrap_or_default();
                let game_fg = run.and_then(|r| game(r.fg));
                let game_bg = run.and_then(|r| game(r.bg));
                if let Some(c) = resolve_glk_channel(game_fg, slot.fg, base_style.fg, honor) {
                    s = s.fg(c);
                }
                if let Some(c) = resolve_glk_channel(game_bg, slot.bg, base_style.bg, honor) {
                    s = s.bg(c);
                }
                // SQ-0309 §3: apply the Glk style's typographic modifiers — the registry theme
                // slot's canonical modifiers (Emphasized→italic, Header→bold, …) plus any override
                // modifiers (garglk/game stylehints, populated later). Colours resolved above.
                let glk_mods =
                    crate::render::glk_theme_modifiers(scheme, false, glk) | slot.add_modifier;
                if !glk_mods.is_empty() {
                    s = s.add_modifier(glk_mods);
                }
            }
            // Glk hyperlink affordance: layer the themeable `hyperlink` colour
            // and an underline ON TOP of the styling. The underline is applied
            // here (not via `bits`) because `apply_text_style` has no underline
            // bit. Colour is gated on `honor`, matching the prior behaviour.
            if run.map(|r| r.link).unwrap_or(0) != 0 {
                if honor {
                    s = s.patch(scheme.theme.get("hyperlink").style);
                }
                s = s.add_modifier(ratatui::style::Modifier::UNDERLINED);
            }
            s
        };
        let w = crate::textwidth::char_cells(ch);
        if w == 0 {
            // A combining mark / ZWJ owns no column: append it to the cell holding
            // the glyph it modifies, so "e" + U+0301 renders as one "é" cell.
            if let Some(gc) = glyph_col {
                if gc >= area.x && gc < area.right() {
                    if let Some(cell) = buf.cell_mut((gc, y)) {
                        let mut sym = cell.symbol().to_string();
                        sym.push(ch);
                        cell.set_symbol(&sym);
                    }
                }
            }
            continue;
        }
        crate::render::draw_char_clipped(buf, col, y, ch, style, area);
        // Blank the trailing cell of a double-width glyph in the same style: the
        // terminal's own wide-glyph cell skip would otherwise swallow whatever
        // followed it (which is exactly how the char==cell body dropped glyphs).
        for k in 1..w as u16 {
            crate::render::draw_char_clipped(buf, col.saturating_add(k), y, ' ', style, area);
        }
        glyph_col = Some(col);
        col = col.saturating_add(w as u16);
    }
}

/// Format the input prompt line: `"> " + input`.
pub(crate) fn format_input_line(input: &str) -> String {
    format!("> {}", input)
}

/// Format the autocomplete suggestion bar from a list of candidates and the
/// currently-highlighted index.  Returns an empty string when `suggestions` is
/// empty.  The highlighted entry is wrapped in `[brackets]`; others are plain.
///
/// Example: `north  [northeast]  northwest`
pub(crate) fn format_suggestion_line(suggestions: &[String], highlight_idx: usize) -> String {
    if suggestions.is_empty() {
        return String::new();
    }
    let idx = highlight_idx % suggestions.len();
    suggestions
        .iter()
        .enumerate()
        .map(|(i, w)| {
            if i == idx {
                format!("[{}]", w)
            } else {
                w.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("  ")
}

/// Like `format_suggestion_line`, but horizontally scrolls the line to fit
/// `width` columns while keeping the highlighted `[bracketed]` entry visible.
///
/// When the full line fits, it is returned unchanged. Otherwise the window is
/// scrolled the minimum amount needed to bring the highlighted entry's right
/// edge into view, so Tabbing toward off-screen candidates pulls them into the
/// window instead of dropping the brackets off the right edge.
pub(crate) fn visible_suggestion_line(
    suggestions: &[String],
    highlight_idx: usize,
    width: usize,
) -> String {
    if suggestions.is_empty() || width == 0 {
        return String::new();
    }
    let idx = highlight_idx % suggestions.len();
    let line = format_suggestion_line(suggestions, highlight_idx);
    let total = line.chars().count();
    if total <= width {
        return line;
    }

    // Char span of the highlighted entry within the joined line: each preceding
    // entry contributes its word length plus a 2-space separator; the
    // highlighted entry itself adds 2 chars for the surrounding brackets.
    let hl_start: usize = suggestions[..idx]
        .iter()
        .map(|w| w.chars().count() + 2)
        .sum();
    let hl_end = hl_start + suggestions[idx].chars().count() + 2;
    // Scroll just enough to keep the highlighted entry's end on screen. For an
    // entry wider than the window, anchor on its start so the opening bracket
    // shows.
    let offset = if hl_end <= width {
        0
    } else if hl_end - hl_start >= width {
        hl_start
    } else {
        hl_end - width
    };
    line.chars().skip(offset).take(width).collect()
}

/// SQ-0542: the dim completion hint drawn immediately AFTER the typed input,
/// replacing the candidate bar for STORY-WORD completions.
///
/// The bar cost a row, and in the default inline-prompt mode that row came out of
/// the transcript viewport — so every keystroke that gained or lost a candidate
/// shifted the prompt row, and all the scrollback with it, by one line. A hint
/// rendered on the prompt row itself cannot move anything.
///
/// Story-word candidates ([`crate::complete::suggest`]) match by PREFIX, so the
/// candidate always extends what you typed and the hint is simply its tail:
/// typing `op` with `open` offered draws a dim `en` after the caret.
///
/// The COMMAND PALETTE is deliberately untouched: a line starting with the command
/// prefix keeps the bracketed candidate bar below the prompt (its names match by
/// substring, so they have no tail to show, and the palette is an explicit mode
/// where seeing every candidate at once is the point).
///
/// `None` when the caret is not at the end of the line (a hint mid-line reads as
/// text you typed), when focus is elsewhere or an overlay is up, or when the
/// candidate adds nothing — which is exactly the state right after Tab applied it,
/// so the hint clears itself without any extra bookkeeping.
pub(crate) fn ghost_completion(state: &AppState) -> Option<String> {
    if state.focus != Focus::Game || state.any_modal_overlay_open() {
        return None;
    }
    if state.input.as_str().starts_with(state.config.command_prefix) {
        return None; // command palette — keeps its bar
    }
    if state.input.cursor < state.input.char_len() {
        return None;
    }
    let candidate = state.suggestions.get(state.suggestion_idx % state.suggestions.len().max(1))?;
    let partial = state.current_partial();
    if !candidate.to_lowercase().starts_with(&partial.to_lowercase()) {
        return None;
    }
    let hint: String = candidate.chars().skip(partial.chars().count()).collect();
    (!hint.is_empty()).then_some(hint)
}

/// The current inventory item list: the engine's live object-tree contents for
/// the player object, otherwise the last parsed `inventory_fallback` list. Used
/// by the inventory dock panel (`render::inventory_dock`) and by the top-level
/// render to size the dock.
///
/// `player_obj` is the per-turn LOCKED avatar (`turn.rs`), which is only set
/// once a turn has run with a known location — so on its own it leaves the dock
/// empty for the whole first turn, and empty forever in any game the lock never
/// fires for. Ask the engine directly when it is unset, exactly as the command
/// band's `refresh_objects` does; the two panels must never disagree about who
/// the player is.
pub fn inventory_items(
    player_obj: Option<u16>,
    inventory_fallback: &[String],
    introspect: Option<&dyn Introspect>,
) -> Vec<String> {
    let player = player_obj.or_else(|| introspect.and_then(|i| i.player_object()));
    match (player, introspect) {
        (Some(obj), Some(intro)) => {
            // The printed name where the story has one; on Inform 7, which
            // gives an object no hardware short name at all, the words it
            // answers to are the only text naming it (SQ-1042).
            intro.contents(obj).iter().filter_map(|o| o.display_name()).collect()
        }
        _ => inventory_fallback.to_vec(),
    }
}

/// The word a click on each [`inventory_items`] row composes into the
/// prompt, in the SAME order and over the SAME filter (SQ-1244) — so the two
/// lists always line up index-for-index and a click can never grab the wrong
/// row's word.
///
/// This is the command band's WHAT column's own derivation
/// ([`crate::vocab::typeable_name`]), not the display name `inventory_items`
/// shows: the dock may read "brass lantern" while Zork I's parser answers
/// only to `lamp`, `lanter` and `light` (see `typeable_name`'s doc). Falls
/// back to the display name itself when `typeable_name` cannot derive
/// anything, and to the raw fallback text when the engine has no object tree
/// — both exactly mirroring `inventory_items`'s own fallbacks, so a row is
/// never drawn without a word to click.
pub fn inventory_click_words(
    player_obj: Option<u16>,
    inventory_fallback: &[String],
    introspect: Option<&dyn Introspect>,
    vocab: Option<&crate::vocab::StoryVocabulary>,
) -> Vec<String> {
    let player = player_obj.or_else(|| introspect.and_then(|i| i.player_object()));
    match (player, introspect) {
        (Some(obj), Some(intro)) => intro
            .contents(obj)
            .iter()
            .filter_map(|o| {
                let display = o.display_name()?;
                Some(crate::vocab::typeable_name(o, vocab).unwrap_or(display))
            })
            .collect(),
        _ => inventory_fallback.to_vec(),
    }
}

// ── Main render function ───────────────────────────────────────────────────────

/// What a rendered transcript pass reports back to the story pane.
#[derive(Debug, Default, Clone)]
pub struct TranscriptRender {
    /// Whether a scrollbar gutter was drawn in the rightmost column.
    pub scrollbar: bool,
    /// The largest meaningful `transcript_scroll` for this frame
    /// (`total_rows - viewport_rows`).
    pub max_scroll: u16,
    /// Total wrapped rows of the whole transcript (the `[more]` pager needs the
    /// true total even when it fits).
    pub total_rows: u16,
    /// Rows of the pane that actually carry transcript prose this frame: what is
    /// left of it after the status line, the input bar, a suggestion/search strip
    /// and — while it is showing — the `[more]` prompt row. This is the number the
    /// pager and the paging keys must measure against; the pane rect they used to
    /// get instead counted every one of those reserved rows as readable, and a
    /// turn that overflowed by exactly those rows scrolled past unpaged (SQ-0823).
    pub viewport_rows: u16,
    /// Rows the `[more]` prompt takes OUT of `viewport_rows` while it is showing —
    /// `1` on this cell path (it reserves its own row), `0` when the region is too
    /// short to spare one. The pager parks the view on the frame BEFORE the prompt
    /// appears, so it has to subtract this itself or the top row of the first new
    /// screenful is the one the prompt bar displaces (SQ-0823).
    pub prompt_rows: u16,
    /// Per-frame map from rendered cell `(col, row)` to Glk hyperlink value.
    pub links: Vec<((u16, u16), u32)>,
}

/// Render the GAME pane into `buf` within `area`:
///
/// - Top row(s): v3 status line (location left, score/turns or time right), reversed style.
///   When `state.colors.status_header_style != None`, the status line is wrapped in a box
///   (3 rows total: border-top, content, border-bottom).  Falls back to plain when the area
///   is too small.
/// - Middle rows: scrolling transcript from `state.transcript` (newest at bottom).
/// - Bottom row(s): `"> " + state.input`; cursor indicator `_` when `state.focus == Focus::Game`.
///   When `state.colors.input_line_style != None`, the input line is wrapped in a box
///   (3 rows total).  Falls back to plain when the area is too small.
///
/// Returns this pass's [`TranscriptRender`]: the scrollbar gutter flag (so the
/// caller can exclude that column from text selection), the scroll clamps, and
/// the rows this frame really gave to prose.
pub fn render_transcript(
    status: &StatusModel,
    // No longer used here: the inventory moved out of this pane into the
    // docked panel (`render::inventory_dock`), which sources its own items
    // via `inventory_items` at the top-level render. Kept so callers (the
    // window-tree walk in `screen.rs`) don't need their own plumbing change.
    _introspect: Option<&dyn Introspect>,
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
    game_input: Option<Style>,
) -> TranscriptRender {
    if area.height == 0 || area.width == 0 {
        return TranscriptRender::default();
    }

    // SQ-0740: under ZMSD §8.3's Amiga interpreter the machine itself has one ink
    // and one page for the whole screen, and the story's prose sits on it — the
    // same pair `render::screen::v6_host_pair` gives the pixel ring around this
    // viewport, so the reading surface matches the frame drawn about it instead of
    // punching a themed hole through it. A no-op on every other frame.
    let normal_style =
        crate::render::screen::v6_machine_page(state, state.colors.theme.get("transcript").style);

    // ── Determine status and input heights based on border style ─────────────

    let status_style_kind = state.colors.status_header_style;
    let input_style_kind  = state.colors.input_line_style;

    // The status bar always shows for v3 (its automatic status line is valid).
    // For v4+/Glulx (HostManaged) the synthesized bar is removed entirely —
    // transient app feedback no longer reuses the score bar; it surfaces as a
    // top-right notification toast instead (SQ-0176).
    let status_visible = matches!(status, StatusModel::Classic { .. });

    // When boxed, status/input each take 3 rows; fall back to 1 if too small.
    // Gate on "any side present" (base OR per-side) so a style="none" + per-side
    // config (e.g. left/right-only) still boxes and draws its side bars.
    let status_boxed = status_visible && (status_style_kind != BorderStyle::None || state.colors.status_header_sides.any_on()) && area.height >= 5;
    let input_boxed  = (input_style_kind  != BorderStyle::None || state.colors.input_line_sides.any_on()) && area.height >= 5;
    let status_rows: u16 = if !status_visible { 0 } else if status_boxed { 3 } else { 1 };
    // Inline-prompt mode (`command_bar` off): no dedicated bottom bar — the live
    // input is drawn flush after the game's kept `>` in the transcript body, so
    // the whole bottom flows into the middle area (`input_rows == 0`).
    let input_rows:  u16 = if !state.config.command_bar { 0 } else if input_boxed { 3 } else { 1 };

    // ── Top row(s): status line ──────────────────────────────────────────────

    let status_region = Rect::new(area.x, area.y, area.width, status_rows.min(area.height));

    if status_boxed {
        // Draw a pane frame around the status region.
        let status_header = state.colors.theme.get("status_header").style;
        let frame = draw_framed(buf, status_region, state.colors.status_header_sides, &state.colors.status_header_glyphs, status_header, false);
        // Render status text into the inner content row.
        render_status_content(status, state, buf, frame.content);
    } else {
        render_status_content(status, state, buf, status_region);
    }

    if area.height < status_rows + 1 {
        return TranscriptRender::default();
    }

    // ── Bottom row(s): input line ─────────────────────────────────────────────

    let input_region_top = area.bottom().saturating_sub(input_rows);
    let input_region = Rect::new(area.x, input_region_top, area.width, input_rows.min(area.height));

    // Only the command-bar mode draws the dedicated bottom input bar. In inline
    // mode (`input_rows == 0`) the live input is drawn by `render_middle` flush
    // after the last transcript row (the game's kept `>` prompt).
    if state.config.command_bar {
        if input_boxed {
            let input_line = state.colors.theme.get("input_line").style;
            let frame = draw_framed(buf, input_region, state.colors.input_line_sides, &state.colors.input_line_glyphs, input_line, false);
            render_input_content(state, buf, frame.content, normal_style, game_input);
        } else {
            render_input_content(state, buf, input_region, normal_style, game_input);
        }
    }

    // ── Middle area: transcript + inventory + suggestion ─────────────────────

    let middle_top = area.y + status_rows;
    let middle_bottom = input_region_top;
    if middle_top >= middle_bottom {
        return TranscriptRender::default();
    }
    let middle_area = Rect::new(area.x, middle_top, area.width, middle_bottom - middle_top);
    render_middle(state, buf, middle_area, normal_style, game_input)
}

/// Choose the rect notification toasts anchor to: the live transcript
/// viewport when one is published, else the story pane's inner content rect
/// when it has room for at least a 1-row strip, else the full frame.
///
/// Toasts are terminal cells, and cells lose to image placements: a game whose
/// story window opens with graphics across its top (a v6 chrome band, a
/// Scott/Glulx top graphics window) drew OVER a pane-anchored toast, leaving
/// it unreadable (SQ-0577). The transcript viewport is exactly the region the
/// renderer laid out as real text this frame — cells always win there — so
/// prefer it. It is clamped to the story pane (the published geom can be a
/// frame stale after a resize) and must still fit the minimum toast strip. On
/// a fully-imaged story pane (the v6 splash / map / rebus takeovers publish no
/// fresh geom) the anchor degrades to the pane rect as before.
///
/// `draw_frame` calls this with the frame's published transcript geometry, the
/// story pane's content rect (as drawn this frame) and the full terminal
/// frame, so a toast is never lost even in `Layout::TranscriptFull`-without-
/// room edge cases or a terminal so small the story pane collapses to zero
/// content area. (SQ-0415)
pub fn notification_anchor_rect(transcript: Option<Rect>, story_area: Rect, full: Rect) -> Rect {
    if let Some(t) = transcript {
        let t = t.intersection(story_area);
        if t.width >= 6 && t.height > 0 {
            return t;
        }
    }
    if story_area.width >= 6 && story_area.height > 0 {
        story_area
    } else {
        full
    }
}

/// Cap on the text rows a single notification box will grow to before its
/// last visible row is ellipsised (SQ-1253) — otherwise the box grows freely
/// with the message.
const NOTIFY_TEXT_ROW_CAP: usize = 5;

/// Word-wrap `text` to `width` columns for a notification box, capped at
/// [`NOTIFY_TEXT_ROW_CAP`] rows.
///
/// A message that wraps within the cap is returned in full — nothing is
/// lost. A message with text left over past the cap has its last visible row
/// rebuilt from where that row starts in the original text and re-truncated
/// at a word boundary with a trailing `…` (never a middle row, so every row
/// before the cut stays untouched).
fn wrap_notification_text(text: &str, width: u16) -> Vec<String> {
    let ranges = wrap_line_ranges(text, width);
    if ranges.len() <= NOTIFY_TEXT_ROW_CAP {
        return ranges.into_iter().map(|(s, _, _)| s).collect();
    }
    let last_start = ranges[NOTIFY_TEXT_ROW_CAP - 1].1;
    let remainder: String = text.chars().skip(last_start).collect();
    let mut rows: Vec<String> = ranges.into_iter().take(NOTIFY_TEXT_ROW_CAP).map(|(s, _, _)| s).collect();
    rows[NOTIFY_TEXT_ROW_CAP - 1] = truncate_status_text(&remainder, width as usize);
    rows
}

/// Draw the top-right notification toasts over `area` — normally the story
/// pane's content rect, or the full frame as a fallback (see
/// [`notification_anchor_rect`]).
///
/// Newest is drawn on top; each toast slides in from `area`'s right edge,
/// holds for a few seconds, and slides out (see [`crate::notify`]), clipped to
/// `area` throughout. Called last in `draw_frame` so toasts overlay the story
/// pane's own content (and anything else drawn under `area`). When animations
/// are disabled the toast simply appears and disappears on the same clock,
/// without sliding. (SQ-0176, SQ-0415)
///
/// A message too wide for one row wraps at word boundaries instead of being
/// cut off: the box grows downward to fit, up to [`NOTIFY_TEXT_ROW_CAP`] rows,
/// past which the last row ends in `…` (SQ-1253). A message that fits in one
/// row draws exactly as it did before wrapping existed.
pub fn render_notifications(buf: &mut Buffer, area: Rect, state: &AppState) {
    let notes = state.notifications.active();
    if notes.is_empty() || area.width < 6 || area.height == 0 {
        return;
    }
    let style = state.colors.theme.get("notification").style;
    let animate = state.config.animation.enabled;
    let easing = state.config.animation.easing;
    // A single-bordered box by default (SQ-0176); collapses to a border-less
    // strip only if the border is themed off or there's no vertical room for a
    // frame.
    let boxed = (state.colors.notification_style != BorderStyle::None
        || state.colors.notification_sides.any_on())
        && area.height >= 3;
    let border_rows: u16 = if boxed { 2 } else { 0 };
    // Cap the inner text width (leave room for a space of padding each side).
    let max_inner = (area.width as usize).min(48).saturating_sub(if boxed { 2 } else { 0 });
    let text_w = max_inner.saturating_sub(2) as u16;
    let right = area.right();

    // `active()` is oldest-first; draw the newest (last) in the top box, then
    // stack older ones below it — each box's own (now possibly multi-row)
    // height decides where the next one starts.
    let mut top = area.y;
    for note in notes.iter().rev() {
        let rows = wrap_notification_text(&note.text, text_w);
        let box_h = border_rows + rows.len() as u16;
        if top + box_h > area.bottom() {
            break;
        }
        let reveal = note.reveal(animate, easing);
        let content_w = rows.iter().map(|r| r.chars().count()).max().unwrap_or(0) as u16;
        let box_w = content_w + 2 + if boxed { 2 } else { 0 };
        // Slide in from the right: the box translates leftward from off the right
        // edge; columns past `right` clip naturally against the buffer bounds.
        let shown = (reveal * box_w as f64).round().clamp(0.0, box_w as f64) as u16;
        if shown != 0 {
            let box_left = right - shown;
            let box_region = Rect::new(box_left, top, box_w, box_h);
            if boxed {
                let frame = draw_framed(
                    buf,
                    box_region,
                    state.colors.notification_sides,
                    &state.colors.notification_glyphs,
                    style,
                    false,
                );
                for (r, line) in rows.iter().enumerate() {
                    let inner = format!(" {line:<content_w$} ", content_w = content_w as usize);
                    draw_str_clipped(buf, frame.content.x, frame.content.y + r as u16, &inner, style, frame.content);
                }
            } else {
                for (r, line) in rows.iter().enumerate() {
                    let inner = format!(" {line:<content_w$} ", content_w = content_w as usize);
                    draw_str_clipped(buf, box_left, top + r as u16, &inner, style, box_region);
                }
            }
        }
        top += box_h;
    }
}

/// Draw the status bar into `region`.
///
/// Each segment in `state.colors.statusbar_layout` is resolved (placeholders
/// substituted, empty ones hidden), styled (base patched with the segment
/// style), and packed into left/center/right clusters. Transient app feedback
/// is no longer drawn here — it surfaces as a notification toast (SQ-0176).
fn render_status_content(
    status: &StatusModel,
    state: &AppState,
    buf: &mut Buffer,
    region: Rect,
) {
    if region.height == 0 || region.width == 0 {
        return;
    }
    let base = state.colors.theme.get("status_bar").style;
    let status_y = region.y;
    let w = region.width as usize;

    // Fill the row with the base style.
    for x in region.x..region.right() {
        if let Some(cell) = buf.cell_mut((x, status_y)) {
            cell.set_symbol(" ").set_style(base);
        }
    }

    // For HostManaged (v4+/Glulx) the synthesized bar is removed entirely; a
    // Classic v3 bar renders its automatic status line below.
    let (location, right_field) = match status {
        StatusModel::HostManaged => return,
        StatusModel::Classic { location, right } => (location.clone(), *right),
    };

    // Build the field values for this turn (classic automatic status line).
    let (score, moves, time) = match right_field {
        StatusField::ScoreTurns { score, turns } => (Some(score.to_string()), Some(turns.to_string()), None),
        StatusField::Time { hours, minutes } => (None, None, Some(format!("{:02}:{:02}", hours, minutes))),
    };
    let filter = match state.transcript_filter {
        TranscriptFilter::Both => String::new(),
        TranscriptFilter::Story => "[filter: story]".to_string(),
        TranscriptFilter::Meta => "[filter: meta]".to_string(),
    };
    let fields = StatusFields {
        location,
        score,
        moves,
        time,
        turns: state.turns.to_string(),
        title: state.title.clone(),
        filter,
    };

    // SQ-0873: on the Amiga the status line is not a band at all — the reversal
    // is applied per RUN of text and the page shows between them (376 px of it in
    // `amiga-spellbreaker.png`, between "Council Chamber" and "Score: 0/0"). So
    // the row's fill above is the machine's page and the segments carry the
    // reverse. Every other machine's band is uniform, `status_run_style` answers
    // `None`, and the segments inherit the base exactly as they always did.
    let run_base = state
        .period_look
        .and_then(|l| crate::period::status_run_style(&l))
        .unwrap_or(base);

    // Resolve + style + drop hidden segments.
    let visible: Vec<(String, Style, crate::colors::Align)> = state
        .colors
        .statusbar_layout
        .segments
        .iter()
        .filter_map(|seg| {
            resolve_placeholders(&seg.text, &fields).map(|txt| (txt, run_base.patch(seg.style), seg.align))
        })
        .collect();

    // Pack into clusters and draw.
    for (x, txt, style) in pack_status_clusters(&visible, w) {
        draw_str_clipped(buf, region.x + x, status_y, &txt, style, region);
    }
}

/// Draw the input prompt (and cursor) into `region` with `normal_style`.
///
/// Hidden during char-input mode (`state.char_mode == true`) because the game
/// is awaiting a single keypress, not a typed line.
fn render_input_content(
    state: &AppState,
    buf: &mut Buffer,
    region: Rect,
    normal_style: Style,
    game_input: Option<Style>,
) {
    if region.height == 0 || region.width == 0 {
        return;
    }
    // In read_char mode — or while a Glulx game waits on a timer/mouse/hyperlink
    // event only — the prompt is meaningless: hide it entirely.
    if state.char_mode || state.event_wait {
        return;
    }
    let w = region.width as usize;
    let input_y = region.y;

    // The "> " prompt and the typed text are separately styleable (patched over
    // the normal style, so an unset selector renders identically to before).
    // The game's current colour, when honoured, wins over the theme fields.
    let input_prompt = state.colors.theme.get("input_prompt").style;
    let input_text = state.colors.theme.get("input_text").style;
    let base_prompt = normal_style.patch(input_prompt);
    let base_text = normal_style.patch(input_text);
    let (prompt_style, text_style) = match game_input {
        Some(gs) => (base_prompt.patch(gs), base_text.patch(gs)),
        None => (base_prompt, base_text),
    };
    let prefix = format_input_line(""); // "> "
    let prefix_trunc = truncate_line(&prefix, w);
    draw_str_clipped(buf, region.x, input_y, prefix_trunc, prompt_style, region);
    let text_x = region.x + prefix_trunc.chars().count() as u16;
    let text_w = w.saturating_sub(prefix_trunc.chars().count());
    // The typed line is measured and drawn in CELLS (SQ-0655): a wide glyph the
    // player typed or pasted takes two, and the caret — plus the click that places
    // it (`input_click_index`) — has to land on the same columns the text does.
    let input_trunc = crate::textwidth::truncate_to_cols(state.input.as_str(), text_w);
    let input_cells = draw_str_cells(buf, text_x, input_y, input_trunc, text_style, region);
    // Where the text actually landed, so a click can be mapped back to a caret index (SQ-0354).
    state.input_text_origin.set(Some((text_x, input_y)));

    // SQ-0542: the completion hint rides on this row, after the typed text, so it
    // never moves anything. Drawn BEFORE the caret so the caret can sit on its
    // first glyph (the fish/zsh look) rather than blanking it.
    let ghost = ghost_completion(state);
    if let Some(g) = &ghost {
        let gx = text_x + input_cells;
        let gw = region.right().saturating_sub(gx) as usize;
        let g_trunc = crate::textwidth::truncate_to_cols(g, gw);
        draw_str_cells(buf, gx, input_y, g_trunc, state.colors.theme.get("suggestion").style, region);
    }

    // Not focus-gated: the caret shows what you typed and where, which stays true
    // while the keyboard is on the map. A modal still suppresses it.
    if !state.any_modal_overlay_open() {
        // Draw the caret where it actually IS, not always after the last char (SQ-0354). Clamped to
        // the drawn text: a long line is truncated to fit, and the caret must not be painted past
        // what is on screen.
        let drawn = input_trunc.chars().count();
        // Caret column = the display width of the text BEFORE it, not its char count.
        let cursor_x = text_x + crate::textwidth::cols_of_chars(input_trunc, state.input.cursor.min(drawn)) as u16;
        if cursor_x < region.right() {
            if let Some(cell) = buf.cell_mut((cursor_x, input_y)) {
                // Mid-line, or sitting on the ghost's first glyph (SQ-0542): keep
                // the symbol and just restyle, so the text — and the hint — stay
                // readable under the caret.
                let over_text = state.input.cursor < drawn || ghost.is_some();
                draw_caret(cell, over_text, state.period_look, text_style, game_input);
            }
        }
    }
}

/// Render the middle section: suggestion line (or search hint), transcript body.
/// Returns this pass's [`TranscriptRender`] — the scrollbar gutter flag, the
/// scroll clamps, the rows this frame really gave to prose, and the per-frame
/// cell → hyperlink map.
fn render_middle(
    state: &AppState,
    buf: &mut Buffer,
    area: Rect,
    normal_style: Style,
    game_input: Option<Style>,
) -> TranscriptRender {
    if area.height == 0 || area.width == 0 {
        return TranscriptRender::default();
    }
    let w = area.width as usize;

    // The input_y used by the original code was area.bottom() - 1 of the *full* area;
    // here area is already the middle section, so its bottom is the boundary.
    let middle_bottom = area.bottom(); // exclusive

    // ── Suggestion line or search hint: one row above middle_bottom ──────────
    // When search is active, the search hint replaces the suggestion line.
    let suggestion_y = middle_bottom.saturating_sub(1);
    let has_search = state.search_query.is_some();
    // SQ-0542: only the COMMAND PALETTE still shows the candidate bar. Story-word
    // completions draw as a ghost tail on the prompt row itself
    // ([`ghost_completion`]) and reserve nothing — the bar's row used to come out
    // of the transcript viewport, so in inline-prompt mode every keystroke that
    // gained or lost a candidate shifted the prompt row and all the scrollback
    // with it. Palette names match by substring and have no tail to show, and the
    // palette is an explicit mode where seeing every candidate at once is the
    // point, so it keeps the bar (and its bounce) unchanged.
    let has_suggestions = state.focus == Focus::Game
        && !state.suggestions.is_empty()
        && !has_search
        && state.input.as_str().starts_with(state.config.command_prefix);

    // Optional box chrome for the auto-complete popup: mirrors the input-line
    // boxing (base OR any per-side on, and enough room). When enabled, the popup
    // becomes a 3-row framed mini-window; otherwise it stays the 1-row strip.
    let sug_style_kind = state.colors.suggestion_line_style;
    let suggestion_boxed = has_suggestions
        && (sug_style_kind != BorderStyle::None || state.colors.suggestion_line_sides.any_on())
        && area.height >= 5;
    let box_top = middle_bottom.saturating_sub(3);
    let suggestion = state.colors.theme.get("suggestion").style;

    if suggestion_boxed {
        // Draw a pane frame around the 3-row popup region, then render the
        // suggestion strip into the inner content row.
        let box_region = Rect::new(area.x, box_top, area.width, 3);
        let frame = draw_framed(buf, box_region, state.colors.suggestion_line_sides, &state.colors.suggestion_line_glyphs, suggestion, false);
        let content = frame.content;
        if content.height >= 1 && content.width >= 1 {
            let sug_line = visible_suggestion_line(&state.suggestions, state.suggestion_idx, content.width as usize);
            draw_str_clipped(buf, content.x, content.y, &sug_line, suggestion, content);
        }
    } else if has_search && area.height >= 2 && suggestion_y > area.y {
        // Draw the search hint line.
        let q = state.search_query.as_deref().unwrap_or("");
        let match_count = state.search_matches.len();
        let cur_idx = if match_count > 0 { state.search_idx + 1 } else { 0 };
        let key_back = state.config.search.key_back;
        let key_forward = state.config.search.key_forward;
        let hint = format!(
            "search: {}  [{}/{}]  {}:back {}:fwd  Esc:clear",
            q, cur_idx, match_count, key_back, key_forward
        );
        let hint_trunc = truncate_line(&hint, w);
        let hint_style = suggestion;
        draw_str_clipped(buf, area.x, suggestion_y, hint_trunc, hint_style, area);
    } else if has_suggestions && area.height >= 2 && suggestion_y > area.y {
        // Horizontally scroll so the highlighted entry stays on screen rather
        // than being clipped off the right edge.
        let sug_line = visible_suggestion_line(&state.suggestions, state.suggestion_idx, w);
        let sug_style = suggestion;
        draw_str_clipped(buf, area.x, suggestion_y, &sug_line, sug_style, area);
    }

    // ── Transcript body ───────────────────────────────────────────────────────
    if area.height < 2 {
        // Not enough room for transcript when there's a suggestion row.
        return TranscriptRender::default();
    }

    let transcript_top = area.y;
    let transcript_bottom = if suggestion_boxed {
        box_top
    } else if (has_search || has_suggestions) && suggestion_y > area.y {
        suggestion_y
    } else {
        middle_bottom
    };
    // [more] pager (SQ-0404): reserve the bottom row for the prompt when active,
    // as long as at least one transcript row remains above it. `prompt_rows`
    // reports that reservation whether or not the prompt is up right now — the
    // pager parks the view one frame BEFORE it appears, and has to know that this
    // layout will spend a row on it (SQ-0823).
    let prompt_rows = u16::from(transcript_bottom > transcript_top + 1);
    let more_row = (state.pager.active && prompt_rows == 1).then_some(transcript_bottom - 1);
    let transcript_bottom = transcript_bottom - more_row.is_some() as u16;

    if transcript_top >= transcript_bottom {
        return TranscriptRender::default();
    }
    let transcript_rows = (transcript_bottom - transcript_top) as usize;

    let images_enabled = state.game_picker.is_some();
    let char_px = state
        .game_picker
        .as_ref()
        .map(|p| {
            let f = p.font_size();
            (f.width, f.height)
        })
        .unwrap_or((1, 1));
    // Reserve the rightmost column of the body as the scrollbar gutter so text
    // never collides with the scrollbar. Wrap and clip to this narrower body.
    let body_area = if area.width >= 2 {
        Rect { width: area.width - 1, ..area }
    } else {
        area
    };
    // Effective scroll: the animated displayed offset (line-rounded) while a
    // smooth scroll is in flight, else the logical target. Clamped below.
    let effective_scroll = state.effective_transcript_scroll();

    // Wrapped-transcript cache (SQ-0305; incremental since SQ-1034): re-wrapping
    // the whole filtered history and cloning every visible line/run/image is the
    // dominant per-frame cost. What this frame owes is
    // `wrap_cache::WrapKey::plan`'s to say — the ONE owner of that decision, and
    // the same one the raster path asks. An idle redraw or a scroll reuses the
    // rows; a turn that only printed extends them; an insert-above-the-prompt or
    // a moved screen-clear anchor REPAIRS the disturbed tail instead of
    // rebuilding whole (SQ-1179); a resize, a filter or a theme still rebuilds.
    //
    // The key is built ONCE here and compared without cloning, so the hot path
    // never pays for the colour scheme or the room name.
    // `old_anchor`/`old_anchor_filtered` are this cache's OWN synced anchor and
    // its filtered position, from before this frame touched anything — the
    // baseline `clear_anchor_filtered` below needs to tell "the anchor is
    // exactly where it was last frame" (carry the old filtered position
    // unchanged) apart from "the anchor moved into this frame's new tail"
    // (recompute it), which `plan` alone does not carry (SQ-1223).
    let (plan, old_anchor, old_anchor_filtered) = match state.transcript_wrap.borrow().as_ref() {
        Some(c) => (c.key.plan(state, body_area.width), c.key.shape.clear_anchor, c.clear_anchor_filtered),
        None => (WrapPlan::Rebuild, None, None),
    };
    if plan != WrapPlan::Reuse {
        // The source lines this frame has to wrap: all of them on a rebuild;
        // only the ones that just arrived on an append; from the old cached
        // tail's own raw index on a repair, which re-collects it too — SQ-1179
        // needs it wrapped fresh, since new content now precedes it there.
        let wrap_from = match plan {
            WrapPlan::Append { from } | WrapPlan::Repair { at: from } => from,
            _ => 0,
        };
        let visible_indices = state.visible_transcript_indices_from(wrap_from);
        let filtered_lines: Vec<String> = visible_indices.iter().map(|&i| state.transcript[i].clone()).collect();
        let filtered_kinds: Vec<TranscriptKind> = visible_indices.iter().map(|&i| state.transcript_kinds.get(i).copied().unwrap_or(TranscriptKind::Story)).collect();
        // Resolve each logical line's text style ONCE, before wrapping. Story lines
        // run through the rule list (user → location → system → base); the other
        // kinds use their fixed per-category style. Resolving here (not per wrapped
        // fragment) keeps whole-line matching correct when a line wraps.
        let room_name = state.current_room_name.as_deref();
        // SQ-0822: `normal_style` already carries §8.3's Amiga machine pair when
        // there is one (`v6_machine_page`, above), so a Story line whose channels
        // are inherited resolves them from the MACHINE rather than from the theme —
        // and the built-in "bracketed line came from the interpreter" rule stands
        // down, because on that machine the line is the game's prose in the game's
        // pens. Off the Amiga `normal_style` IS `colors.transcript` and the flag is
        // false, so every other frame resolves exactly as before.
        let machine_owns_ink = state.v6_page_pair.get().is_some();
        // SQ-0954: AND THE STORY WINDOW'S OWN PAGE OVER THAT, for lanthorn's own
        // annotations.
        //
        // `period::painted` gives the echoed command, the meta gutter and a warning
        // the machine's PAGE — deliberately, so their ground is the paper the prose
        // is on rather than the theme's punched through the middle of the
        // transcript. That is right whenever the machine's page IS the ground. In
        // v6 it need not be: a game that calls `set_colour` on window 0 declares
        // its own, and Zork Zero does.
        //
        // Measured at 120x45, release 393 off the DOS floppy, colours honoured: the
        // story page is `Rgb(173, 173, 173)` and the meta line came out with its
        // TEXT on the IBM PC's blue `Rgb(0, 0, 173)` and its gutter glyph on the
        // grey — two grounds on one row, a blue stripe through a grey page. The
        // Amiga floppy reads the same way against `Rgb(7, 75, 161)`. The Macintosh
        // disk does NOT, because there the machine page and the story page are both
        // white, which is exactly why a single-machine test would have missed it.
        //
        // So re-ground them on the window's page — and ONLY where the period look
        // is what put the page there, which is what the `bg == look.page` test
        // says. A user who set one of these backgrounds in `style.toml` was never
        // painted over by `period::painted` (it paints only selectors still at
        // `Provenance::Default`) and is not painted over here either; a user whose
        // colour happens to equal the machine's loses nothing, because the two
        // agree. All three move together because one table row paints all three
        // with the same justification.
        // THE GROUND THE PROSE IS READ ON — the window's own page, else the
        // MACHINE's. The same two layers `inline_image::page_for` resolves a float's
        // ground through, and for the same reason: it is the paper, and everything
        // printed on the page has to agree about it.
        //
        // NOT `normal_style.bg`, which looks like the obvious answer and is not one.
        // It is what an INHERITED channel falls back to, and Zork Zero's prose runs
        // name their own background, so the cells the player reads carry the game's
        // grey while `normal_style` carries whatever the period look painted the
        // `transcript` selector.
        //
        // Both layers are needed because each press has only one of them. Zork Zero
        // r393 declares a window page (`Rgb(173, 173, 173)`) and publishes no machine
        // pair; Arthur's Amiga floppy declares no window page and publishes a pair
        // (`Rgb(66, 66, 66)`). Either way the PERIOD LOOK's page is a third,
        // independent number — `Rgb(0, 0, 173)` and `Rgb(7, 75, 161)` respectively —
        // which is what makes a fix keyed on one layer alone silently miss the other
        // press. It did: this shipped keyed on the story page and Arthur was still a
        // blue sentence in a grey row.
        let prose_ground = state
            .v6_story_page
            .get()
            .map(|(r, g, b)| ratatui::style::Color::Rgb(r, g, b))
            .or_else(|| crate::render::screen::v6_machine_page(state, Style::default()).bg);
        let reground = |s: Style| -> Style {
            match (prose_ground, state.period_look) {
                (Some(ground), Some(look))
                    if s.bg == Some(ratatui::style::Color::Rgb(look.page.0, look.page.1, look.page.2)) =>
                {
                    s.bg(ground)
                }
                _ => s,
            }
        };
        let transcript_input = reground(state.colors.theme.get("transcript_input").style);
        let transcript_meta = reground(state.colors.theme.get("transcript_meta").style);
        let transcript_warning = reground(state.colors.theme.get("transcript_warning").style);
        // Assist lines arrive with their tone's style already resolved (the
        // `transcript_styles` override above), so this is only the floor a
        // hand-made or restored Assist line falls back to.
        let transcript_assist = reground(state.colors.theme.get("transcript_assist").style);
        let filtered_styles: Vec<Style> = visible_indices
            .iter()
            .zip(filtered_kinds.iter())
            .map(|(&i, kind)| {
                // An explicitly-styled line takes the same rule, so the rule does
                // not depend on which push a caller reached for.
                //
                // `/dump-terminal`'s headings and its ASSUMED values arrive through
                // `push_transcript_internal_styled`, which lands a resolved style
                // here and used to return before the re-grounding below ever ran.
                // Measured: they are NOT affected today — `terminal_dump_heading`
                // inherits `heading` and `terminal_dump_assumed` inherits `alert`,
                // `period::painted` paints neither role, and both resolve with no
                // background at all, so their cells keep whatever the pane put
                // down. This is applied for uniformity rather than to fix an
                // observed frame: a style that DOES carry the period page has no
                // business being treated differently for having come in through
                // the other door.
                //
                // `reground` is the same self-limiting test either way — only a
                // background the period look itself put there is replaced — so a
                // style a caller or a user chose is passed through untouched.
                if let Some(ov) = state.transcript_styles.get(i).copied().flatten() {
                    return reground(ov);
                }
                match kind {
                    TranscriptKind::Story   => state.colors.resolve_story_style(normal_style, &state.transcript[i], room_name, machine_owns_ink),
                    TranscriptKind::Input   => transcript_input,
                    TranscriptKind::Meta    => transcript_meta,
                    TranscriptKind::Warning => transcript_warning,
                    TranscriptKind::Assist  => transcript_assist,
                }
            })
            .collect();
        let filtered_runs: Vec<Vec<StyleRun>> = visible_indices
            .iter()
            .map(|&i| state.transcript_runs.get(i).cloned().unwrap_or_default())
            .collect();
        let filtered_para: Vec<ParaFmt> = visible_indices
            .iter()
            .map(|&i| state.transcript_para.get(i).copied().unwrap_or_default())
            .collect();
        // Inline images parallel the filtered lines, indexed by the SAME visible
        // indices. Bands are only emitted when a game Picker is present (images
        // enabled); `char_px` is the picker's cell pixel size for pixel-accurate fit.
        //
        // A v6 STORY PICTURE FOLLOWS THE TEXT, NOT THE FRAME (SQ-1002). Zork Zero's
        // drop-caps and room icons are authored in native game pixels on a screen
        // whose character cell is 8x16, to sit beside a specific number of lines of
        // the game's own prose — the cap that opens a paragraph is drawn four text
        // lines tall. Hybrid maps the game's native pixel space onto the terminal
        // at two different rates: art by the letterbox factor `s`, and text at one
        // native cell per TERMINAL cell. This used to scale the pictures by `s`,
        // "to match the chrome ring", which is the wrong half of the frame — at
        // `s = 2` the cap claimed eight terminal rows beside a four-row paragraph,
        // and it grew further the larger the pane got.
        //
        // The text's rate is `device_cell / native_cell` per axis, so a picture
        // `w x h` native pixels lands on `ceil(w/8) x ceil(h/16)` cells — the
        // footprint the game drew it for, and exactly what RASTER mode has always
        // given it (`build_main_text`, which composes glyphs and art together at
        // the native cell and scales the finished canvas once).
        //
        // Per axis and not one scalar, because a terminal cell is very rarely
        // 1:2 — at 8x18 the horizontal rate is 1.0 and the vertical 1.125.
        // `fit_preserving_aspect` keeps the picture's own shape inside the box, so
        // an uneven box letterboxes rather than stretching.
        let hybrid_ring = state.v6_hybrid_ring.get();
        let text_rate = |px: u32, cell: u16, native: u32| (px * u32::from(cell)).div_ceil(native);
        // SQ-0895: this used to fork on frameless, which applied its own
        // inline-image sizing and was the ONLY mode that drew graphics-window
        // CONTENT splashes inline — every other mode had to DROP the
        // `ContentSplash` entries to avoid double-rendering what it already drew
        // as a window canvas. With the mode gone nothing draws them, so the
        // entries are no longer emitted at all and the fork collapses to the
        // hybrid letterbox scaling that was always the other branch.
        let filtered_images: Vec<Option<crate::inline_image::InlineImage>> = visible_indices
            .iter()
            .map(|&i| {
                let mut img = state.transcript_images.get(i).cloned().flatten();
                if let Some(im) = img.as_mut() {
                    if im.scaled.is_none() && hybrid_ring {
                        let (w, h) = (im.pixels.width().max(1), im.pixels.height().max(1));
                        im.scaled = Some((
                            text_rate(w, char_px.0, u32::from(state.v6_text.cell().w())),
                            text_rate(h, char_px.1, u32::from(state.v6_text.cell().h())),
                        ));
                    }
                }
                img
            })
            .collect();
        // Bound the inline-image protocol cache to the images present in the
        // filtered transcript, keyed by source Arc-ptr. Combined with the pinned
        // Arc in each cache value, this drops entries only once their image is
        // truly gone. Cached and re-applied every frame (below).
        let live_bands: std::collections::HashSet<usize> = filtered_images
            .iter()
            .flatten()
            .map(|img| std::sync::Arc::as_ptr(&img.pixels) as usize)
            .collect();
        let mut slot = state.transcript_wrap.borrow_mut();
        if matches!(plan, WrapPlan::Rebuild) {
            *slot = Some(CellWrapCache {
                key: WrapKey::of(state, body_area.width),
                rows: Vec::new(),
                starts: Vec::new(),
                stable_rows: 0,
                carry: None,
                tail_entry_carry: None,
                // Set below, uniformly for every plan (SQ-1179) — computed
                // fresh from `state` after the sync either way, so the
                // placeholder here is never read.
                tail_visible: false,
                // Set below, uniformly for every plan (SQ-1179) — a fresh
                // cache's `starts` is empty, so the shared formula degenerates
                // to exactly the old rebuild-only computation.
                clear_anchor_filtered: None,
                anchor_row: None,
                live_bands: std::collections::HashSet::new(),
            });
        }
        let cache = slot.as_mut().expect("rebuilt above, or appending onto a live cache");
        // Drop the trailing float flush before extending: those strip rows are not
        // final, and the prose that just arrived claims them (SQ-1034).
        cache.rows.truncate(cache.stable_rows);
        let mut carry = cache.carry.clone();
        // A repair discards the cache's OWN last consumed line before
        // re-wrapping the tail fresh (SQ-1179): whatever now sits at its raw
        // index has moved (an insert landed before it), so the cached entry
        // for it no longer describes anything real. `tail_visible` — captured
        // at the PREVIOUS sync, before this frame's edits — is what makes that
        // decision correctly: reading the CURRENT kind at that raw index would
        // describe whatever moved there instead of what the cache wrapped.
        if let WrapPlan::Repair { .. } = plan {
            if cache.tail_visible {
                let popped = cache.starts.pop().expect(
                    "repair: tail_visible said the cache's last entry is the old tail line, so one must exist",
                );
                cache.rows.truncate(popped);
                carry = cache.tail_entry_carry.clone();
            }
            // else: the old tail line never made it into the filtered product
            // (it didn't pass the filter), so there is nothing to undo and
            // `carry` is already correct as the state after the last VISIBLE
            // line, unaffected by an invisible one.
        }
        // Filtered lines with raw index < `wrap_from` are untouched by this
        // frame (SQ-1179) — every one of `cache.starts`'s entries up to here
        // describes them, so this is the base the anchor formula below adds
        // the newly-wrapped suffix's own count onto.
        let starts_before = cache.starts.len();
        wrap_lines_kinded_extend(
            &mut cache.rows,
            &mut cache.starts,
            &mut carry,
            &filtered_lines,
            &filtered_kinds,
            &filtered_styles,
            &filtered_runs,
            &filtered_para,
            &filtered_images,
            char_px,
            images_enabled,
            true, // main transcript: left-margin images float, text wraps beside (SQ-0454)
            body_area.width,
            &mut cache.tail_entry_carry,
        );
        cache.stable_rows = cache.rows.len();
        cache.carry = carry.clone();
        // Finish any float whose picture outran (or had no) text beside it.
        flush_float(&mut cache.rows, &mut carry);
        cache.live_bands.extend(live_bands);
        // Map the screen-clear boundary (a full-transcript index) into the
        // filtered line list, so top-anchoring works under any transcript
        // filter. Recomputing this every frame from `starts_before` is only
        // sound when the anchor itself moved INTO this frame's new tail —
        // that is the one case `WrapKey::plan` has proven sits at or after
        // this cache's synced length, which is what makes every filtered line
        // up to `starts_before` unconditionally precede it. An anchor that
        // did NOT move this frame carries no such proof: `starts_before` is
        // "how much is already wrapped", not "how much precedes the anchor",
        // and on a long-lived anchor those are wildly different — recomputing
        // anyway made `clear_anchor_filtered` chase the transcript's growing
        // length every frame, which force-pinned every frame's display to
        // just its own new tail and dropped a still-open margin float's
        // earlier strips out of the rendered window (SQ-1223). So an
        // unmoved anchor keeps the filtered position it already had; only a
        // genuine move (or a Rebuild, where `starts_before` is 0 and
        // `visible_indices` is the whole transcript) recomputes it.
        cache.clear_anchor_filtered = if !matches!(plan, WrapPlan::Rebuild) && old_anchor == state.clear_anchor {
            old_anchor_filtered
        } else {
            state.clear_anchor.map(|a| starts_before + visible_indices.iter().filter(|&&i| i < a).count())
        };
        // The anchor is where that line STARTS in the wrap just built (SQ-0640) — a
        // separate wrap of the prefix would count a margin float's strips twice
        // over. Recomputed on every append and not merely on a rebuild: an anchor
        // sitting exactly at the end is an EMPTY post-clear screen, and the next
        // line printed is what gives it a real row.
        cache.anchor_row = anchor_row_at(&cache.starts, cache.rows.len(), cache.clear_anchor_filtered);
        cache.key = WrapKey::of(state, body_area.width);
        // This cache is now synced to `cache.key`'s edits value, so any run of
        // tail-inserts it reflects is spent — the NEXT insert starts a fresh
        // run anchored at that new baseline (SQ-1179's `WrapKey::plan`).
        cache.tail_visible = cache.key.content.len > 0
            && state
                .transcript_kinds
                .get(cache.key.content.len - 1)
                .is_some_and(|&k| transcript_filter_matches(state.transcript_filter, k));
        state.transcript_tail_insert.set(None);
    }
    let cache = state.transcript_wrap.borrow();
    let entry = cache.as_ref().expect("wrap cache populated above");
    // Per-frame eviction: bound the inline-image protocol cache to present images,
    // AND to the current cell size / page — a still-live image's variant from
    // BEFORE the last theme flip, font-size change, or page change is otherwise
    // never looked up again but stays cached for as long as the image is on
    // screen (SQ-1195). `current_cell` matches what `render_row` will key any
    // fresh entry with below; a missing picker (nothing can be drawn this frame)
    // falls back to `(0, 0)`, which no real cell size ever is, so a live image's
    // stale entries are dropped rather than kept on a guess.
    let current_cell = state
        .game_picker
        .as_ref()
        .map(|p| {
            let fs = p.font_size();
            (fs.width.max(1), fs.height.max(1))
        })
        .unwrap_or((0, 0));
    // Every evicted band's kitty upload must be freed in the terminal, not
    // merely forgotten (SQ-1190) — `InlineImageRender` has no `GraphicsRender`
    // of its own, so route the ids it hands back into the sibling field's queue.
    let evicted_bands = state.inline_image_render.borrow_mut().retain_live(
        &entry.live_bands,
        current_cell,
        crate::render::inline_image::float_page(state),
    );
    state.graphics_render.borrow_mut().queue_external_deletes(evicted_bands);
    // Window the cached rows to the visible viewport (cheap; no re-wrap). The
    // top-anchor only applies at the bottom, handled inside `window_wrapped_rows`.
    let (lines, total_rows, first_abs_row) =
        window_wrapped_rows(&entry.rows, entry.anchor_row, transcript_rows, effective_scroll);
    // Search highlight style, themed via the `transcript_search_highlight`
    // selector (SQ-0643; default reproduces the old hardcoded black-on-yellow).
    let search_highlight_style = state.colors.theme.get("transcript_search_highlight").style;
    let query_lower = state.search_query.as_deref().map(|q| q.to_lowercase()).unwrap_or_default();

    // Per-frame map from rendered cell (col, row) → Glk hyperlink value, so a
    // mouse click can be hit-tested to its link (consumed downstream in the
    // click gate). Cells are in the story-pane frame, which equals the Glk
    // screen frame.
    let mut links: Vec<((u16, u16), u32)> = Vec::new();
    // The current game-set background band, carried across blank rows so the gaps
    // between a game's coloured paragraphs fill too (SQ-0263).
    let mut band_bg: Option<ratatui::style::Color> = None;
    let meta_marker = state.colors.theme.get("meta_marker").style;
    let warning_marker = state.colors.theme.get("warning_marker").style;

    for (i, wr) in lines.iter().enumerate() {
        let row_y = transcript_top + i as u16;
        if row_y >= transcript_bottom {
            break;
        }
        // Inline-image band row: blit the strip for this row instead of text.
        if crate::render::inline_image::try_blit_band_row(state, wr, body_area.x, body_area.width, row_y, buf) {
            continue;
        }
        // Meta/Warning reserve the 2-col gutter and draw their marker glyph;
        // Story/Input draw flush left. The text style was resolved per logical
        // line above and is carried on every wrapped row.
        let (gutter, marker_style) = match wr.kind {
            TranscriptKind::Meta    => (Some(state.symbols.meta_gutter), meta_marker),
            TranscriptKind::Warning => (Some(state.symbols.warning_gutter), warning_marker),
            // The assist gutter is drawn in the line's OWN style, so the caution
            // tone's mark is as loud as its text; meta/warning take a separate
            // marker selector because their text is uniformly muted.
            TranscriptKind::Assist  => (Some(state.symbols.assist_gutter), wr.style),
            TranscriptKind::Story | TranscriptKind::Input => (None, Style::default()),
        };
        if let Some(glyph) = gutter {
            draw_str_clipped(buf, body_area.x, row_y, &glyph.to_string(), marker_style, body_area);
        }
        let text_x = body_area.x + text_origin_col(wr.kind);
        let search = has_search.then_some((query_lower.as_str(), search_highlight_style));
        draw_str_runs(buf, text_x, row_y, &wr.text, wr.style, &wr.runs, search, body_area, crate::render::TextInk::of(state));
        // …and, while a reveal is lit, re-style the words on this row that name
        // one of the story's own things (SQ-1107, SQ-1207). A pass OVER the
        // drawn cells, after the text and its runs: the reveal is a property of
        // the moment, not of the text, and folding it into `wr.runs` would write
        // a decoration into the game's own output — which is what gets
        // persisted in the archive.
        crate::reveal::paint_row(buf, text_x, row_y, &wr.text, body_area, state);

        // Record cell→link for every linked span on this row. `run.start/end` are
        // CHAR offsets within `wr.text` (re-based by `rebase_runs`), while the
        // cells they were drawn in are DISPLAY columns — a wide glyph or a
        // multibyte prefix moves the link's columns right of its char indices
        // (SQ-0662). Convert through the same width table `draw_str_runs` advanced
        // by, so the click lands on the link's actual glyphs. Clip to the body.
        // One pass builds char index → start column for the whole row, so N linked
        // runs cost one scan rather than N (wrapping and drawing are per-frame work
        // over the whole window; nothing here may go quadratic in the row length).
        let link_cols: Vec<usize> = if wr.runs.iter().any(|r| r.link != 0) {
            let mut v = Vec::with_capacity(wr.text.chars().count() + 1);
            let mut c = 0usize;
            for ch in wr.text.chars() {
                v.push(c);
                c += crate::textwidth::char_cells(ch);
            }
            v.push(c);
            v
        } else {
            Vec::new()
        };
        let col_of = |i: usize| -> usize { link_cols.get(i).copied().unwrap_or_else(|| link_cols.last().copied().unwrap_or(0)) };
        for run in &wr.runs {
            if run.link == 0 {
                continue;
            }
            let (c0, c1) = (col_of(run.start), col_of(run.end));
            for j in c0..c1 {
                let col = text_x.saturating_add(j as u16);
                if col >= body_area.right() {
                    break;
                }
                links.push(((col, row_y), run.link));
            }
        }

        // Extend a game-set background so a coloured paragraph reads as a solid
        // band, not a ragged block that stops at the text (when the pane's own
        // background differs from the row's). A row whose trailing run set a
        // background fills its trailing space with it and opens/continues a band;
        // a BLANK row inside that band fills fully (so the gaps between a game's
        // black-on-white paragraphs are white too); a non-blank Default row closes
        // the band (its own theme-coloured text must stay legible, so it is left
        // untouched). Only when honouring game colours. (SQ-0263)
        if state.config.honor_game_colours {
            let row_bg = wr.runs.last().and_then(|last| {
                let rbg = crate::state::unpack_zcolour(last.bg);
                (!matches!(rbg, zvm::screen::ZColour::Default))
                    .then(|| crate::render::resolve_zcolour(rbg, &state.colors))
            });
            if let Some(bg) = row_bg {
                // A coloured row: fill trailing space and (re)open the band.
                let fill = Style::default().bg(bg);
                // Cells, not chars: the fill starts where the drawn text ENDS on
                // screen, which a wide glyph pushes past its char count (SQ-0662).
                let start = text_x.saturating_add(crate::textwidth::str_cells(&wr.text) as u16);
                for x in start..body_area.right() {
                    if let Some(cell) = buf.cell_mut((x, row_y)) {
                        cell.set_symbol(" ").set_style(fill);
                    }
                }
                band_bg = Some(bg);
            } else if wr.text.trim().is_empty() {
                // A blank row inside a coloured band: fill it whole with the band bg.
                if let Some(bg) = band_bg {
                    let fill = Style::default().bg(bg);
                    for x in body_area.x..body_area.right() {
                        if let Some(cell) = buf.cell_mut((x, row_y)) {
                            cell.set_symbol(" ").set_style(fill);
                        }
                    }
                }
            } else {
                // A non-blank Default row closes the band.
                band_bg = None;
            }
        }

        // Left-margin float (SQ-0454): blit the picture strip over the left
        // `cols` columns AFTER the row's (indented) text and any background fill,
        // so the image always wins. The prose already started past `indent`, so
        // it never collides with the picture.
        if let Some(float) = &wr.float {
            crate::render::inline_image::blit_float_row(state, float, body_area.x, body_area.width, row_y, buf);
        }
    }

    // ── Inline live input (command_bar off) ──────────────────────────────────
    // Draw the typed command + block cursor flush after the last transcript row
    // (the game's kept `>` prompt), so scrollback and the live line read as one
    // continuous prompt. Only when the bottom of the transcript is on screen
    // (effective_scroll == 0) so scrolled-up history is never overwritten.
    if !state.config.command_bar
        && !state.char_mode
        && !state.event_wait
        && !state.any_modal_overlay_open()
        && effective_scroll == 0
        && !lines.is_empty()
    {
        // A tall margin float outlives the prose beside it: every wrapped row
        // below the game's `>` carries the float geometry but an EMPTY text (see
        // `WrappedRow::float`). The live input belongs on the PROMPT row, so walk
        // back over that text-less tail — otherwise the typed command and its
        // caret land at the left margin somewhere down the picture's flank, which
        // is exactly what Shogun's opening did beside the ship (SQ-0544).
        // Ordinary rows (no float, or a float row that still carries its text)
        // stop the walk immediately, so nothing else moves.
        let bottom_i = lines.len().min(transcript_rows) - 1;
        let mut last_i = bottom_i;
        while last_i > 0
            && lines[last_i].text.is_empty()
            && lines[last_i].band.is_none()
            && lines[last_i].float.is_some()
        {
            last_i -= 1;
        }
        let row_y = transcript_top + last_i as u16;
        let last = &lines[last_i];
        // Only draw when the true last wrapped row is the one visible at the
        // bottom, it fits inside the transcript region, and it is a text row
        // (never an inline-image band).
        // The scroll test still asks about the TRUE bottom row (is the end of the
        // transcript on screen?), not the prompt row the walk above settled on.
        if row_y < transcript_bottom
            && first_abs_row + bottom_i == total_rows.saturating_sub(1)
            && last.band.is_none()
        {
            // Flush after the last line's text — matching Task 3's flush command
            // echo. Use the SAME text_x the draw loop used for this row's kind:
            // Story/Input draw at body_area.x; Meta/Warning reserve the gutter.
            let input_text = state.colors.theme.get("input_text").style;
            let base_text = normal_style.patch(input_text);
            let text_style = match game_input {
                Some(gs) => base_text.patch(gs),
                None => base_text,
            };
            let row_text_x = body_area.x + text_origin_col(last.kind);
            // Where the row's text ENDS on screen — cells, not chars (SQ-0662).
            let start_col = row_text_x + crate::textwidth::str_cells(&last.text) as u16;
            let avail = body_area.right().saturating_sub(start_col) as usize;
            // Cells, not chars, for the typed line — see `draw_str_cells` (SQ-0655).
            let input_trunc = crate::textwidth::truncate_to_cols(state.input.as_str(), avail);
            let input_cells = draw_str_cells(buf, start_col, row_y, input_trunc, text_style, body_area);
            // Where the input text landed, so a click maps back to a caret index
            // (SQ-0354). The command-bar path sets this too; in inline mode this is
            // the only place it's set, so a click can find the line at all.
            state.input_text_origin.set(Some((start_col, row_y)));
            // SQ-0542: the completion hint, drawn on this very row after the typed
            // text. This is the mode the bounce came from — the old bar took its row
            // out of the transcript viewport, so the prompt row and every line above
            // it shifted on each keystroke that gained or lost a candidate. Drawn
            // before the caret so the caret can sit on its first glyph.
            let drawn = input_trunc.chars().count();
            let ghost = ghost_completion(state);
            if let Some(g) = &ghost {
                let gx = start_col + input_cells;
                let gw = body_area.right().saturating_sub(gx) as usize;
                let g_trunc = crate::textwidth::truncate_to_cols(g, gw);
                draw_str_cells(buf, gx, row_y, g_trunc, state.colors.theme.get("suggestion").style, body_area);
            }
            // Draw the caret where it actually IS, not always after the last char
            // (SQ-0354). Clamped to the drawn (truncated) text; its column is the
            // display width of the text before it, not that text's char count.
            let cursor_x = start_col
                + crate::textwidth::cols_of_chars(input_trunc, state.input.cursor.min(drawn)) as u16;
            if cursor_x < body_area.right() {
                if let Some(cell) = buf.cell_mut((cursor_x, row_y)) {
                    // Mid-line, or over the ghost's first glyph: restyle the char
                    // the caret sits on (keep it readable); at the end of the line
                    // with no hint: draw the caret's own shape.
                    let over_text = state.input.cursor < drawn || ghost.is_some();
                    draw_caret(cell, over_text, state.period_look, text_style, game_input);
                }
            }
        }
    }

    // Publish this frame's transcript geometry so the mouse handlers and the copy
    // path can map screen cells ↔ absolute wrapped rows. (SQ-0197)
    state.transcript_geom.set(Some(crate::clipboard::TranscriptGeom {
        area: body_area,
        first_abs_row,
        total_rows,
    }));
    if let Some(sel) = state.selection {
        let width = body_area.width;
        // Highlight the visible portion (reverse video) by absolute row. The span
        // is snapped out to whole glyphs, so it covers exactly the cells
        // `clipboard::extract` copies (SQ-0662): a selection edge that lands on
        // the second cell of a CJK glyph copies that glyph whole, and the
        // highlight has to say so rather than reverse half a character.
        for (i, wr) in lines.iter().enumerate() {
            let row_y = transcript_top + i as u16;
            if row_y >= transcript_bottom { break; }
            let abs = first_abs_row + i;
            // A Meta/Warning row's text starts at screen column `origin` (its
            // gutter marker occupies the columns before it); `row_span` pulls a
            // span that lands in the gutter up to the first real text cell, so
            // the highlight never paints the marker and a gutter click behaves
            // like a click on the row's own first glyph (SQ-0665).
            let origin = text_origin_col(wr.kind);
            let Some((c0, c1)) = crate::clipboard::row_span(width, sel, abs, origin) else { continue };
            // Snap in TEXT-relative space — `wr.text` holds only the text, not
            // the gutter prefix — then shift back to the screen column the
            // glyph actually occupies to paint the buffer.
            let (tc0, tc1) = crate::textwidth::snap_cols_to_glyphs(&wr.text, (c0 - origin) as usize, (c1 - origin) as usize);
            let last_col = width.saturating_sub(1);
            let hi = (tc1 as u16 + origin).min(last_col);
            for col in (tc0 as u16 + origin)..=hi {
                if let Some(cell) = buf.cell_mut((body_area.x + col, row_y)) {
                    let s = cell.style();
                    cell.set_style(s.add_modifier(ratatui::style::Modifier::REVERSED));
                }
            }
        }
        // Extract the copy from the FULL wrapped set (off-screen rows included),
        // reusing the cached rows rather than re-wrapping. (SQ-0305)
        if sel.is_empty() {
            *state.selection_text.borrow_mut() = None;
        } else {
            let texts: Vec<&str> = entry.rows.iter().map(|r| r.text.as_str()).collect();
            let origins: Vec<u16> = entry.rows.iter().map(|r| text_origin_col(r.kind)).collect();
            *state.selection_text.borrow_mut() = Some(crate::clipboard::extract(&texts, &origins, width, sel));
        }
    }

    // ── Scrollbar (only when the content overflows the viewport) ──────────────
    // SQ-0782: the story pane's bar auto-hides. It can, because it is drawn in
    // the MARGIN BAND (see below) rather than in a gutter taken out of the text
    // width — showing or hiding it cannot reflow a character, unlike the modals,
    // whose bars are reserved from `content.width - 1` and so stay up always.
    //
    // The reported flag still tracks "this pane's gutter column IS a scrollbar
    // gutter", not "a bar is on screen right now" — the gutter is reserved out
    // of `body_area` whether or not the bar is currently faded out, and callers
    // use the flag to keep that column out of text selection.
    let sb_opacity = state.transcript_scrollbar_opacity();
    let drew_scrollbar = total_rows > transcript_rows && area.width >= 2 && transcript_bottom > transcript_top;
    if drew_scrollbar && sb_opacity > 0.0 {
        let start = total_rows
            .saturating_sub(effective_scroll as usize)
            .saturating_sub(transcript_rows);
        // Push the scrollbar past the right text margin so it sits flush against
        // the pane border — only the text is inset by the margin (SQ-0345). The
        // margin band was already painted blank by `reserve_text_margin`.
        let right_margin = state.text_margin_applied.get();
        let sb_area = Rect {
            x: area.right() - 1 + right_margin,
            y: transcript_top,
            width: 1,
            height: transcript_bottom - transcript_top,
        };
        // Fade toward whatever the bar sits on: the transcript's own background
        // when the theme sets one, else the terminal's probed default page. With
        // neither (a terminal that declined OSC 11 and a transparent theme) there
        // is no RGB to mix, and `faded` leaves the colours alone — the bar pops.
        let backdrop = state
            .colors
            .transcript
            .bg
            .filter(|c| *c != ratatui::style::Color::Reset)
            .or_else(|| {
                state
                    .term_default_colors
                    .bg
                    .map(|p| ratatui::style::Color::Rgb(p[0], p[1], p[2]))
            })
            .unwrap_or(ratatui::style::Color::Reset);
        let look = crate::render::scroll::ScrollbarLook::from_theme(&state.colors.theme)
            .faded(sb_opacity, backdrop);
        crate::render::scroll::draw_scrollbar(buf, sb_area, total_rows, transcript_rows, start, look);
    }
    // [more] pager prompt (SQ-0404): a reverse-video bar on the reserved row.
    if let Some(row) = more_row {
        let mp = state.colors.theme.get("more_prompt").style;
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, row)) {
                cell.set_symbol(" ").set_style(mp);
            }
        }
        let label = "[more]  Space/PgDn continue  Esc skip";
        for (x, ch) in (area.x + 1..area.right()).zip(label.chars()) {
            if let Some(cell) = buf.cell_mut((x, row)) {
                cell.set_symbol(&ch.to_string()).set_style(mp);
            }
        }
    }
    let max_scroll = total_rows.saturating_sub(transcript_rows).min(u16::MAX as usize) as u16;
    let total = total_rows.min(u16::MAX as usize) as u16;
    TranscriptRender {
        scrollbar: drew_scrollbar,
        max_scroll,
        total_rows: total,
        viewport_rows: transcript_rows.min(u16::MAX as usize) as u16,
        prompt_rows,
        links,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use zvm::cpu::exec::Machine;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    // ── Pure helper tests (no Machine required) ──────────────────────────────

    /// Build a `Theme` with the given selectors' fg overridden (like a
    /// `style.toml` decl), so tests exercising render code migrated to
    /// `theme.get("<selector>")` (SQ-0309) can still inject a custom colour
    /// instead of mutating the (no-longer-read) legacy `ColorScheme` field.
    fn theme_with_overrides(overrides: &[(&str, ratatui::style::Color)]) -> crate::theme::resolve::Theme {
        let mut decls = std::collections::HashMap::new();
        for &(sel, fg) in overrides {
            decls.insert(sel.to_string(), crate::theme::registry::Delta { fg: Some(fg), ..crate::theme::registry::Delta::EMPTY });
        }
        crate::theme::resolve::resolve(
            &crate::theme::resolve::Roles::terminal_default(),
            &decls,
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
        )
    }

    /// A helper: read `row` of `buf` as a String across `[x0, x1)`.
    fn read_row(buf: &Buffer, row: u16, x0: u16, x1: u16) -> String {
        (x0..x1)
            .map(|x| buf.cell((x, row)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect()
    }

    /// **What a late insert-above-prompt costs the wrap cache** (SQ-1124,
    /// SQ-1179).
    ///
    /// An insert-above-prompt is not an append: it moves a line the cache has
    /// already wrapped (the trailing prompt itself). Before SQ-1179 that meant
    /// `TranscriptEdit::Rewrote` and a full rebuild from line zero, on EVERY
    /// `push_transcript_internal` call in inline-prompt mode — every `/help`,
    /// every save banner, every assist. SQ-1179 gave the edit its own
    /// `TranscriptEdit::Inserted { at, count }`, which the cache can REPAIR
    /// through instead: everything before `at` provably did not move, so only
    /// the (here, one-line) tail is re-wrapped.
    ///
    /// # The measurement this replaced, and why it is a comment and not an
    /// assertion
    ///
    /// Before this fix, at 40 columns — a narrow pane, where wrapping is
    /// worst — on a 12-core machine in a **debug** build: 200 lines rebuilt in
    /// 0.8 ms, 1,000 in 3.7 ms, 5,000 in 18.1 ms and 20,000 in 71.8 ms — linear
    /// in scrollback. A wall-clock ceiling on that number is a flake by
    /// construction (it failed on all three CI platforms' 3-4 shared cores
    /// while passing locally every time), which is why this case asserts the
    /// SHAPE of the work — which `WrapPlan` this frame owes — rather than its
    /// duration.
    ///
    /// # What is asserted
    ///
    /// The insert lands ABOVE the prompt (which is what makes it a tail edit
    /// rather than a plain append), the frame after it owes exactly one
    /// REPAIR (not a `Rebuild` — that is the regression this case exists to
    /// catch), the frame after THAT owes nothing at all, and the wrap it
    /// produced is correct — the assist is in the rows, the prompt is still
    /// last, and no earlier source line was disturbed. A regression that
    /// rebuilt per line, or per frame forever, fails on the plan; a
    /// regression that wrapped the wrong thing fails on the rows.
    #[test]
    fn a_late_insert_above_the_prompt_repairs_the_wrap_exactly_once() {
        use crate::render::wrap_cache::WrapPlan;

        let cols = 40u16;
        let area = Rect::new(0, 0, cols, 24);
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.assist_preamble_shown = true;
        for i in 0..200usize {
            state.push_transcript_kind(
                &format!(
                    "{i} You are standing in an open field west of a white house, with a \
                     boarded front door. There is a small mailbox here."
                ),
                TranscriptKind::Story,
            );
        }
        let prompt = state.transcript.last().cloned().expect("a trailing story line");

        // The plan this frame owes against the product the cache is holding. The
        // width is the cache's own — the renderer wraps to the BODY area, which is
        // narrower than the pane by whatever gutter it reserved, and asking with
        // the pane's width would report a rebuild that is really a mismatch here.
        let plan = |state: &AppState| -> WrapPlan {
            let cache = state.transcript_wrap.borrow();
            let key = &cache.as_ref().expect("cache populated by a render").key;
            key.plan(state, key.shape.width)
        };

        let mut buf = Buffer::empty(area);
        let normal = state.colors.theme.get("story_text").style;
        // Two, because the first frame is what SETTLES the layout facts the key is
        // taken over (the v6 page cells are filled in as the pane is drawn); the
        // second is the steady state a player's idle frame is in.
        render_middle(&state, &mut buf, area, normal, None);
        render_middle(&state, &mut buf, area, normal, None);
        assert_eq!(plan(&state), WrapPlan::Reuse, "a warm cache on an unchanged transcript");

        // The late arrival: exactly what `push_assist` does in inline-prompt mode
        // — an insert ABOVE the trailing story prompt, hence `Inserted`.
        let style = state.colors.theme.get("assist_help").style;
        state.push_transcript_internal_styled("try instead — light", TranscriptKind::Assist, style);
        assert_eq!(
            state.transcript.last().map(String::as_str),
            Some(prompt.as_str()),
            "the assist went above the prompt, not after it — otherwise this is an append \
             and the case is measuring nothing",
        );
        assert_eq!(
            plan(&state),
            WrapPlan::Repair { at: 199 },
            "a line already wrapped moved, but only the tail — a REPAIR, not a whole rebuild",
        );

        render_middle(&state, &mut buf, area, normal, None);
        assert_eq!(plan(&state), WrapPlan::Reuse, "the repair is paid ONCE, not every frame");

        // …and the wrap it repaired is the right one.
        let cache = state.transcript_wrap.borrow();
        let rows = &cache.as_ref().expect("cache").rows;
        assert!(
            rows.iter().any(|r| r.text.contains("try instead") && r.kind == TranscriptKind::Assist),
            "the assist is in the wrapped product",
        );
        let last = rows.last().expect("rows");
        assert!(
            prompt.contains(last.text.trim_end()) && last.kind == TranscriptKind::Story,
            "the prompt is still the last thing wrapped: {:?}",
            last.text,
        );
    }

    #[test]
    fn notification_toast_is_a_right_anchored_bordered_box() {
        let mut state = AppState::default();
        // Disable animation so reveal snaps to 1 (fully shown) — deterministic.
        state.config.animation.enabled = false;
        state.notifications.push("[Saved as: foo]");

        // The full frame is wider than the story pane (SQ-0415: a map pane sits
        // to the right of it, cols 40..60) — proves the toast anchors to the
        // PANE's right edge, not the frame's.
        let full = Rect::new(0, 0, 60, 10);
        let story_area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(full);
        render_notifications(&mut buf, story_area, &state);

        // Default is a single-bordered box: top border (row 0), content (row 1),
        // bottom border (row 2). The bracket pair is stripped for a clean toast:
        // inner is " Saved as: foo " (15 chars), box 17 wide, right-anchored to
        // col 40 (the pane's right edge), so it spans cols 23..40.
        let content = read_row(&buf, 1, 24, 39);
        assert_eq!(content, " Saved as: foo ", "content row, bracket-stripped + padded: {content:?}");
        // Borders drawn above and below (non-blank in the box columns).
        assert!(!read_row(&buf, 0, 23, 40).trim().is_empty(), "top border drawn");
        assert!(!read_row(&buf, 2, 23, 40).trim().is_empty(), "bottom border drawn");
        // Nothing drawn past the story pane into the map's columns — the toast
        // is clipped to the pane, not the wider frame.
        assert_eq!(read_row(&buf, 1, 40, 60).trim(), "", "toast is clipped to the story pane, not the map");
        // The notification style — the registry's `notification` selector, which
        // derives from the `accent` role reversed (cyan reverse-video) — is
        // applied to the content cells. (SQ-0309: was a baked black-on-cyan
        // Style; now REVERSED cyan fg, same visual result.)
        let cell = buf.cell((30, 1)).expect("content cell exists");
        let themed = state.colors.theme.get("notification").style;
        assert_eq!(Some(cell.fg), themed.fg, "toast uses the themed notification fg");
        assert_eq!(cell.modifier, themed.add_modifier, "toast uses the themed notification modifiers");
    }

    #[test]
    fn notification_toasts_stack_newest_on_top() {
        let mut state = AppState::default();
        state.config.animation.enabled = false;
        state.notifications.push("older");
        state.notifications.push("newer");

        // Same pane-narrower-than-frame setup as above (SQ-0415).
        let full = Rect::new(0, 0, 60, 12);
        let story_area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(full);
        render_notifications(&mut buf, story_area, &state);

        // Each toast is a 3-row box: newest in rows 0-2, older in rows 3-5.
        let newest_box = (0..3).map(|r| read_row(&buf, r, 0, 40)).collect::<String>();
        let older_box = (3..6).map(|r| read_row(&buf, r, 0, 40)).collect::<String>();
        assert!(newest_box.contains("newer"), "newest is in the top box");
        assert!(older_box.contains("older"), "older is pushed to the box below");
        // Nothing spills past the pane into the map columns on either box.
        let map_cols = (0..6).map(|r| read_row(&buf, r, 40, 60)).collect::<String>();
        assert_eq!(map_cols.trim(), "", "toasts stay within the story pane's width");
    }

    /// SQ-1253: a message that fits in one row draws exactly as it did before
    /// wrapping existed — same box height (3), same content row, same width.
    /// This pins the pre-change cells (captured from the code as it stood
    /// before this fix, which drew a single `" {text} "` row with no wrap
    /// path at all) so a future change to the wrap/cap logic can't creep into
    /// the common, non-wrapping case.
    #[test]
    fn notification_short_message_renders_identically_to_before_wrapping() {
        let mut state = AppState::default();
        state.config.animation.enabled = false;
        state.notifications.push("[Saved as: foo]");

        let full = Rect::new(0, 0, 60, 10);
        let story_area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(full);
        render_notifications(&mut buf, story_area, &state);

        // Still exactly a 3-row box (border, content, border) — no extra rows
        // were added for a message that never needed to wrap.
        let content = read_row(&buf, 1, 24, 39);
        assert_eq!(content, " Saved as: foo ", "content row unchanged: {content:?}");
        assert!(!read_row(&buf, 0, 23, 40).trim().is_empty(), "top border drawn");
        assert!(!read_row(&buf, 2, 23, 40).trim().is_empty(), "bottom border drawn");
        // Row 3 (what would be a 4th row if the box had grown) stays blank.
        assert_eq!(read_row(&buf, 3, 0, 40).trim(), "", "no extra row for a one-row message");
    }

    /// SQ-1253: a message wider than the box wraps at word boundaries onto
    /// extra rows instead of being cut off, and no word is lost.
    #[test]
    fn notification_long_message_wraps_at_word_boundaries_without_losing_words() {
        let words: Vec<String> = (0..12).map(|i| format!("word{i}")).collect();
        let text = words.join(" ");

        let mut state = AppState::default();
        state.config.animation.enabled = false;
        state.notifications.push(text.clone());

        let full = Rect::new(0, 0, 60, 20);
        let story_area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(full);
        render_notifications(&mut buf, story_area, &state);

        // Same wrap width the renderer computed for this pane: boxed, 40-wide
        // pane → max_inner 38, text budget 36.
        let text_w: u16 = 36;
        let rows = wrap_notification_text(&text, text_w);
        assert!(rows.len() > 1, "text longer than the width must wrap onto more than one row");

        // Box grew past the old fixed 3 rows: border + N content rows + border.
        let box_h = 2 + rows.len() as u16;
        assert!(box_h > 3, "box grew past the old 3-row height");
        assert!(!read_row(&buf, box_h - 1, 0, 40).trim().is_empty(), "bottom border sits past the old row 2");

        // Falsify: the pre-fix behaviour (`truncate_line` at this width) really
        // did drop text rather than wrap it — confirms this fixture would have
        // caught the SQ-1253 symptom.
        assert!(
            truncate_line(&text, text_w as usize).chars().count() < text.chars().count(),
            "sanity: the old one-row truncation would have lost text at this width"
        );

        // The box's interior columns (inside the border, excluding the ` … `
        // padding), same geometry the renderer used: right-anchored, snug to
        // the widest wrapped row.
        let content_w = rows.iter().map(|r| r.chars().count() as u16).max().unwrap();
        let box_w = content_w + 2 + 2; // padding + border
        let box_left = story_area.right() - box_w;
        let (cx0, cx1) = (box_left + 2, box_left + box_w - 2); // inside border + the 1-space pad

        // Reassemble every rendered content row (trimmed of the row's own
        // right-padding) and confirm every word survives, in order.
        let rendered_words: Vec<String> = (0..rows.len() as u16)
            .flat_map(|i| {
                read_row(&buf, 1 + i, cx0, cx1)
                    .trim()
                    .split_whitespace()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(rendered_words, words, "every word survives the wrap, in order");
    }

    /// SQ-1253: a message that would need more than the row cap wraps up to
    /// the cap and ends with a single trailing `…` on the last row — nothing
    /// drawn after it, and no row above it is touched.
    #[test]
    fn notification_message_beyond_row_cap_ellipsises_last_row_only() {
        let words: Vec<String> = (0..60).map(|i| format!("word{i}")).collect();
        let text = words.join(" ");

        let mut state = AppState::default();
        state.config.animation.enabled = false;
        state.notifications.push(text.clone());

        let full = Rect::new(0, 0, 60, 20);
        let story_area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(full);
        render_notifications(&mut buf, story_area, &state);

        // The same wrap the renderer used for this pane, capped at
        // NOTIFY_TEXT_ROW_CAP rows.
        let text_w: u16 = 36;
        let rows = wrap_notification_text(&text, text_w);
        assert_eq!(rows.len(), NOTIFY_TEXT_ROW_CAP, "wrap itself is capped at the row limit");
        let content_w = rows.iter().map(|r| r.chars().count() as u16).max().unwrap();
        let box_w = content_w + 2 + 2;
        let box_left = story_area.right() - box_w;
        let (cx0, cx1) = (box_left + 2, box_left + box_w - 2);

        // Boxed height is border(1) + NOTIFY_TEXT_ROW_CAP + border(1); content
        // rows are 1..=NOTIFY_TEXT_ROW_CAP, and the row right after them is
        // the bottom border, not a 6th text row.
        let content_rows: Vec<String> = (1..=NOTIFY_TEXT_ROW_CAP as u16)
            .map(|r| read_row(&buf, r, cx0, cx1).trim().to_string())
            .collect();
        for (i, row) in content_rows.iter().enumerate() {
            assert!(!row.is_empty(), "content row {i} should hold text");
        }
        let bottom_border_row = read_row(&buf, NOTIFY_TEXT_ROW_CAP as u16 + 1, 0, 40);
        assert!(!bottom_border_row.trim().is_empty(), "bottom border sits right after the capped rows");
        let past_box_row = read_row(&buf, NOTIFY_TEXT_ROW_CAP as u16 + 2, 0, 40);
        assert_eq!(past_box_row.trim(), "", "nothing drawn past the box — not a 6th text row");

        let last = content_rows.last().unwrap();
        assert!(last.ends_with('…'), "last visible row ends with an ellipsis: {last:?}");
        assert_eq!(last.matches('…').count(), 1, "exactly one ellipsis, nothing drawn after it");
        // No row before the last one is ellipsised — only the tail is cut.
        for row in &content_rows[..content_rows.len() - 1] {
            assert!(!row.contains('…'), "only the last row may be ellipsised: {row:?}");
        }
    }

    #[test]
    fn no_toasts_when_notifications_empty() {
        let state = AppState::default();
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_notifications(&mut buf, area, &state);
        // Nothing drawn: the top-right stays blank.
        assert_eq!(read_row(&buf, 0, 0, 40).trim(), "");
    }

    #[test]
    fn notification_anchor_falls_back_to_full_frame_when_pane_absent_or_tiny() {
        let full = Rect::new(0, 0, 80, 24);

        // A real story pane with room: anchors there, not the frame (SQ-0415).
        let roomy_pane = Rect::new(0, 0, 40, 24);
        assert_eq!(notification_anchor_rect(None, roomy_pane, full), roomy_pane);

        // No story pane at all (e.g. a layout/edge case with zero content rect):
        // falls back to the full frame so a toast is never lost.
        assert_eq!(notification_anchor_rect(None, Rect::default(), full), full);

        // A pane too narrow for even the 1-row/6-col minimum toast strip: also
        // falls back to the full frame.
        let tiny_pane = Rect::new(0, 0, 4, 24);
        assert_eq!(notification_anchor_rect(None, tiny_pane, full), full);

        // A pane with zero height (borders alone ate all the rows): falls back.
        let zero_height_pane = Rect::new(0, 0, 40, 0);
        assert_eq!(notification_anchor_rect(None, zero_height_pane, full), full);
    }

    /// SQ-0577: toasts anchor to the transcript viewport when one was published
    /// this frame — the region where terminal cells always win — instead of the
    /// pane rect, whose top rows can be covered by image placements (a v6 chrome
    /// band, a Scott/Glulx top graphics window) that draw over cell toasts.
    #[test]
    fn notification_anchor_prefers_the_transcript_viewport() {
        let full = Rect::new(0, 0, 80, 24);
        let pane = Rect::new(0, 0, 40, 24);

        // A published viewport inset below top-of-window graphics wins.
        let viewport = Rect::new(2, 5, 36, 15);
        assert_eq!(notification_anchor_rect(Some(viewport), pane, full), viewport);

        // A stale viewport from before a layout change is clamped to the pane;
        // fully outside → ignored, pane anchor as before.
        let stale = Rect::new(50, 0, 20, 10);
        assert_eq!(notification_anchor_rect(Some(stale), pane, full), pane);

        // Partially outside → the clamped intersection anchors when it still
        // fits the minimum toast strip.
        let overhanging = Rect::new(30, 2, 20, 10);
        assert_eq!(
            notification_anchor_rect(Some(overhanging), pane, full),
            Rect::new(30, 2, 10, 10)
        );

        // A viewport too small for the minimum strip degrades to the pane.
        let sliver = Rect::new(0, 0, 4, 24);
        assert_eq!(notification_anchor_rect(Some(sliver), pane, full), pane);
    }

    #[test]
    fn input_uses_full_width_warning_wraps_like_meta() {
        let line = vec!["abcdefgh".to_string()];
        let st = [Style::default()];
        // Input: full width 8 (no gutter) → unsplit.
        let i = wrap_lines_kinded(&line, &[TranscriptKind::Input], &st, &[], &[], &[], (1, 1), false, false, 8);
        assert_eq!(i.iter().map(|wr| wr.text.as_str()).collect::<Vec<_>>(), vec!["abcdefgh"]);
        // Warning: wraps to width-2 = 6 like Meta; continuation gets 2-space
        // hanging indent (leading_spaces("abcdefgh")=0, .max(2)=2).
        let w = wrap_lines_kinded(&line, &[TranscriptKind::Warning], &st, &[], &[], &[], (1, 1), false, false, 8);
        assert_eq!(w.iter().map(|wr| wr.text.as_str()).collect::<Vec<_>>(), vec!["abcdef", "  gh"]);
    }

    // ── Inline-image band wrapping ────────────────────────────────────────────

    fn dummy_img(w: u32, h: u32, align: crate::inline_image::ImageAlign) -> crate::inline_image::InlineImage {
        crate::inline_image::InlineImage { pixels: std::sync::Arc::new(image::RgbaImage::new(w, h)), align, scaled: None, margin_px: None }
    }

    #[test]
    fn image_unit_expands_to_band_rows() {
        // Two text lines with an image unit between; image 16x24 px, cell 8x8 →
        // 2 cols x 3 rows band.
        let transcript = vec!["hi".to_string(), String::new(), "bye".to_string()];
        let kinds = vec![TranscriptKind::Story; 3];
        let styles = vec![Style::default(); 3];
        let runs = vec![Vec::new(); 3];
        let images = vec![None, Some(dummy_img(16, 24, crate::inline_image::ImageAlign::InlineUp)), None];
        let rows = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), true, false, 40);
        // 1 (hi) + 3 (band) + 1 (bye) = 5 rows.
        assert_eq!(rows.len(), 5);
        assert!(rows[0].band.is_none());
        assert_eq!(rows[1].band.as_ref().unwrap().rows, 3);
        assert_eq!(rows[1].band.as_ref().unwrap().row, 0);
        assert_eq!(rows[3].band.as_ref().unwrap().row, 2);
        assert_eq!(rows[4].text, "bye");
    }

    #[test]
    fn image_unit_emits_zero_rows_when_disabled() {
        let transcript = vec!["hi".to_string(), String::new()];
        let kinds = vec![TranscriptKind::Story; 2];
        let styles = vec![Style::default(); 2];
        let runs = vec![Vec::new(); 2];
        let images = vec![None, Some(dummy_img(16, 24, crate::inline_image::ImageAlign::InlineUp))];
        let rows = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), false, false, 40);
        assert_eq!(rows.len(), 1); // only "hi"
    }

    #[test]
    fn band_reflows_narrower_on_smaller_width() {
        // 800x400 px, cell 8x8: width 40 → 40x20; width 20 → 20x10.
        let transcript = vec![String::new()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let runs = vec![Vec::new()];
        let images = vec![Some(dummy_img(800, 400, crate::inline_image::ImageAlign::InlineUp))];
        let wide = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), true, false, 40);
        let narrow = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), true, false, 20);
        assert_eq!(wide.len(), 20);
        assert_eq!(narrow.len(), 10);
    }

    #[test]
    fn margin_right_band_rows_run_0_to_rows_with_pinned_x_off() {
        // 16x24 px, cell 8x8 -> 2x3 cells (same geometry as
        // image_unit_expands_to_band_rows); MarginRight at width 40 pins
        // x_off = 40 - 2 = 38 on every one of the 3 band rows, with `row`
        // running 0, 1, 2 in order.
        let transcript = vec![String::new()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let runs = vec![Vec::new()];
        let images = vec![Some(dummy_img(16, 24, crate::inline_image::ImageAlign::MarginRight))];
        let rows = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), true, false, 40);
        assert_eq!(rows.len(), 3);
        for (expected_row, wr) in rows.iter().enumerate() {
            let band = wr.band.as_ref().unwrap();
            assert_eq!(band.row, expected_row as u16);
            assert_eq!(band.cols, 2);
            assert_eq!(band.rows, 3);
            assert_eq!(band.x_off, 38);
        }
    }

    #[test]
    fn margin_right_sets_x_offset() {
        let transcript = vec![String::new()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let runs = vec![Vec::new()];
        let images = vec![Some(dummy_img(16, 8, crate::inline_image::ImageAlign::MarginRight))]; // 2x1 cells
        let rows = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), true, false, 40);
        assert_eq!(rows[0].band.as_ref().unwrap().x_off, 38); // 40 - 2
    }

    // ── Left-margin floats (SQ-0454) ─────────────────────────────────────────

    fn left_img(w: u32, h: u32, margin_px: Option<u32>) -> crate::inline_image::InlineImage {
        crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::new(w, h)),
            align: crate::inline_image::ImageAlign::MarginLeft,
            scaled: None,
            margin_px,
        }
    }

    #[test]
    fn float_indent_derives_from_width_with_gutter_when_no_margin() {
        // No margin_px: text offset = image cell width + a 1-column gutter.
        let img = left_img(16, 24, None); // 2 cols at cell width 8
        assert_eq!(float_text_indent(&img, 8, 2), 3);
    }

    #[test]
    fn float_indent_honours_margin_px_scaled() {
        // margin_px is in GAME pixels; scale 1 → 48px / 8 = 6 cols.
        let img = left_img(16, 24, Some(48));
        assert_eq!(float_text_indent(&img, 8, 2), 6);
        // With a 2x scaled request (scaled_w 32 / native 16), the game margin
        // scales too: 48 * 2 = 96px / 8 = 12 cols.
        let mut scaled = left_img(16, 24, Some(48));
        scaled.scaled = Some((32, 48));
        assert_eq!(float_text_indent(&scaled, 8, 4), 12);
    }

    #[test]
    fn float_indent_never_below_image_width() {
        // A tiny game margin can't pull text under the picture: indent ≥ cols.
        let img = left_img(64, 8, Some(1)); // margin 1px → 1 col, but 8 cols wide
        assert_eq!(float_text_indent(&img, 8, 8), 8);
    }

    #[test]
    fn float_start_falls_back_to_band_for_unsupported_or_wide() {
        // Inline (non-margin) alignment never floats.
        assert!(FloatState::start(&dummy_img(16, 24, crate::inline_image::ImageAlign::InlineUp), (8, 8), 40).is_none());
        // A left picture wider than ~half the viewport (25 cols of 40) → band.
        assert!(FloatState::start(&left_img(200, 8, None), (8, 8), 40).is_none());
        // A right picture that leaves no prose column (34 cols of 40 → reserve 35,
        // only 5 cols of text < the 8-col floor) → band.
        assert!(FloatState::start(&dummy_img(272, 8, crate::inline_image::ImageAlign::MarginRight), (8, 8), 40).is_none());
        // A normal left-margin picture floats: 16x24 px at cell 8x8 → 2x3 cells,
        // reserve 3 (cols + gutter), text pushed right (pad 3), image at x_off 0.
        let fs = FloatState::start(&left_img(16, 24, None), (8, 8), 40).unwrap();
        assert_eq!((fs.cols, fs.rows, fs.reserve, fs.pad, fs.x_off, fs.next_strip), (2, 3, 3, 3, 0, 0));
        // A right-margin picture (Shogun's opening) floats at the RIGHT: 16x24 →
        // 2x3 cells, reserve 3, text flush left (pad 0), image right-aligned
        // (x_off = 40 - 2 = 38).
        let fr = FloatState::start(&dummy_img(16, 24, crate::inline_image::ImageAlign::MarginRight), (8, 8), 40).unwrap();
        assert_eq!((fr.cols, fr.rows, fr.reserve, fr.pad, fr.x_off, fr.next_strip), (2, 3, 3, 0, 38, 0));
    }

    #[test]
    fn left_float_wraps_following_prose_beside_the_picture() {
        // A left-margin drop-cap (16x24 px → 2x3 cells, indent 3) followed by a
        // long paragraph: the image emits NO band rows of its own; the first 3
        // output rows carry the picture strip and are indented by 3, wrapping the
        // prose at 40-3=37; rows past the picture reclaim full width.
        let para = "word ".repeat(30);
        let transcript = vec![para.trim_end().to_string()];
        let images = [Some(left_img(16, 24, None))];
        // The image is a SEPARATE transcript unit before the prose.
        let mut full_transcript = vec![String::new()];
        full_transcript.extend(transcript);
        let full_images = vec![images[0].clone(), None];
        let kinds = vec![TranscriptKind::Story; 2];
        let styles = vec![Style::default(); 2];
        let runs = vec![Vec::new(); 2];
        let rows = wrap_lines_kinded(&full_transcript, &kinds, &styles, &runs, &[], &full_images, (8, 8), true, true, 40);

        // No standalone band rows for a floated image.
        assert!(rows.iter().all(|r| r.band.is_none()), "a floated image emits no bands");
        let float_rows: Vec<&WrappedRow> = rows.iter().filter(|r| r.float.is_some()).collect();
        assert_eq!(float_rows.len(), 3, "one float strip per image cell-row");
        for (k, r) in float_rows.iter().enumerate() {
            let fb = r.float.as_ref().unwrap();
            assert_eq!((fb.row, fb.cols, fb.rows, fb.x_off), (k as u16, 2, 3, 0));
            if !r.text.is_empty() {
                assert!(r.text.starts_with("   "), "float-row prose indented by 3");
            }
            assert!(r.text.chars().count() <= 40, "drawn width fits the viewport");
        }
        // The float begins at the first output row (no prose preceded the image).
        assert!(rows[0].float.is_some());
        // Rows past the 3-row picture reclaim full width (no forced float indent).
        assert!(
            rows.iter().skip(3).any(|r| r.float.is_none() && !r.text.is_empty()),
            "prose past the picture flows full-width"
        );
    }

    #[test]
    fn right_float_wraps_prose_flush_left_and_pins_picture_right() {
        // SQ-0489: a MarginRight picture (Shogun's opening) followed by a long
        // paragraph. The image emits NO band rows of its own; the covered rows
        // keep prose FLUSH LEFT (no pad) but narrowed to width - reserve, and each
        // carries the picture strip pinned to the RIGHT (x_off = width - cols);
        // rows past the picture reclaim full width. (16x24 px → 2x3 cells,
        // reserve 3, x_off 38 in a 40-col viewport.)
        let para = "word ".repeat(30);
        let transcript = vec![String::new(), para.trim_end().to_string()];
        let images = vec![Some(dummy_img(16, 24, crate::inline_image::ImageAlign::MarginRight)), None];
        let kinds = vec![TranscriptKind::Story; 2];
        let styles = vec![Style::default(); 2];
        let runs = vec![Vec::new(); 2];
        let rows = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), true, true, 40);

        assert!(rows.iter().all(|r| r.band.is_none()), "a floated image emits no bands");
        let float_rows: Vec<&WrappedRow> = rows.iter().filter(|r| r.float.is_some()).collect();
        assert_eq!(float_rows.len(), 3, "one float strip per image cell-row");
        for (k, r) in float_rows.iter().enumerate() {
            let fb = r.float.as_ref().unwrap();
            assert_eq!((fb.row, fb.cols, fb.rows, fb.x_off), (k as u16, 2, 3, 38), "strip pinned to the right edge");
            // Prose is flush left (no leading pad) and narrowed to 40 - 3 = 37.
            assert!(!r.text.starts_with(' '), "right-float prose stays flush left: {:?}", r.text);
            assert!(r.text.chars().count() <= 37, "right-float prose narrowed to width - reserve");
        }
        // Rows past the 3-row picture reclaim full width.
        assert!(
            rows.iter().skip(3).any(|r| r.float.is_none() && r.text.chars().count() > 37),
            "prose past the picture flows full-width"
        );
    }

    #[test]
    fn left_float_emits_leftover_strips_when_taller_than_text() {
        // A 1-col x 5-row picture beside a single short line: the line rides strip
        // 0 (indented by 2), and the remaining 4 strips render as empty float rows
        // so the whole picture still draws.
        let transcript = vec![String::new(), "hi".to_string()];
        let images = vec![Some(left_img(8, 40, None)), None]; // 1 col, 5 rows
        let kinds = vec![TranscriptKind::Story; 2];
        let styles = vec![Style::default(); 2];
        let runs = vec![Vec::new(); 2];
        let rows = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), true, true, 40);
        assert_eq!(rows.len(), 5, "5 output rows: 1 with text + 4 leftover strips");
        assert_eq!(rows[0].text, "  hi", "text indented by 2 (1 col + gutter)");
        for (k, r) in rows.iter().enumerate() {
            let fb = r.float.as_ref().expect("every row carries a strip");
            assert_eq!((fb.row, fb.cols, fb.rows), (k as u16, 1, 5));
            if k > 0 {
                assert_eq!(r.text, "", "leftover strip rows carry no text");
            }
        }
    }

    #[test]
    fn left_float_too_wide_renders_as_band() {
        // A left-margin image wider than half the viewport falls back to the
        // existing full-width band path (band Some, float None).
        let transcript = vec![String::new()];
        let images = vec![Some(left_img(400, 8, None))]; // 40 cols after fit-to-width
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let runs = vec![Vec::new()];
        let rows = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), true, true, 40);
        assert!(rows.iter().all(|r| r.float.is_none()), "too-wide image never floats");
        assert!(rows[0].band.is_some(), "it renders as a band instead");
    }

    #[test]
    fn non_prose_line_flushes_the_float_first() {
        // An app-generated Meta line between the picture and prose finishes the
        // picture (as leftover strips) before rendering at full width.
        let transcript = vec![String::new(), "note".to_string()];
        let images = vec![Some(left_img(8, 24, None)), None]; // 1 col, 3 rows
        let kinds = vec![TranscriptKind::Story, TranscriptKind::Meta];
        let styles = vec![Style::default(); 2];
        let runs = vec![Vec::new(); 2];
        let rows = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), true, true, 40);
        // 3 leftover float strips, then the meta line.
        assert_eq!(rows.len(), 4);
        for (k, r) in rows.iter().take(3).enumerate() {
            assert_eq!(r.float.as_ref().unwrap().row, k as u16);
            assert_eq!(r.text, "");
        }
        assert!(rows[3].float.is_none() && rows[3].band.is_none());
        assert_eq!(rows[3].kind, TranscriptKind::Meta);
    }

    // ── Clear anchor vs. margin floats (SQ-0640) ─────────────────────────────

    #[test]
    fn anchor_row_is_where_post_clear_content_really_starts_beside_a_float() {
        // SQ-0640: the last PRE-clear unit is a left-margin float whose picture (5
        // strips) outruns the prose beside it, and the POST-clear prose wraps beside
        // the remaining strips. Counting the anchor from a standalone wrap of the
        // pre-clear prefix flushed all 5 strips as pre-clear rows — the whole wrap is
        // 5 rows, so the anchor landed at the END and top-anchoring returned an EMPTY
        // viewport at scroll 0. The strips are shared with the post-clear lines, so
        // NOTHING precedes them: the anchor is row 0.
        let transcript = vec![String::new(), "hi".to_string(), "there".to_string()];
        let images = vec![Some(left_img(8, 40, None)), None, None]; // 1 col × 5 rows
        let kinds = vec![TranscriptKind::Story; 3];
        let styles = vec![Style::default(); 3];
        let runs = vec![Vec::new(); 3];
        let rows = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), true, true, 40);
        assert_eq!(rows.len(), 5, "picture strips, two of them carrying the prose");
        let anchor = anchor_wrapped_rows(
            &transcript, &kinds, &styles, &runs, &[], &images, (8, 8), true, true, 40, Some(1),
        );
        assert_eq!(anchor, Some(0), "the float's strips ride BESIDE the post-clear prose");
        let (visible, total, first) = window_wrapped_rows(&rows, anchor, 10, 0);
        assert_eq!((visible.len(), total, first), (5, 5, 0), "the post-clear screen is on screen");
        assert!(
            visible.iter().any(|r| r.text.trim() == "hi") && visible.iter().any(|r| r.text.trim() == "there"),
            "post-clear prose is visible, not scrolled off the top: {:?}",
            visible.iter().map(|r| r.text.clone()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn anchor_row_still_counts_plain_pre_clear_rows() {
        // The float-free case is unchanged: three pre-clear prose lines (one row
        // each) anchor the post-clear content at row 3.
        let transcript = vec!["a".into(), "b".into(), "c".into(), "post".to_string()];
        let kinds = vec![TranscriptKind::Story; 4];
        let styles = vec![Style::default(); 4];
        let runs = vec![Vec::new(); 4];
        let anchor = anchor_wrapped_rows(
            &transcript, &kinds, &styles, &runs, &[], &[], (8, 8), false, true, 40, Some(3),
        );
        assert_eq!(anchor, Some(3));
        // Out-of-range and unset anchors still yield None.
        assert_eq!(
            anchor_wrapped_rows(&transcript, &kinds, &styles, &runs, &[], &[], (8, 8), false, true, 40, Some(9)),
            None
        );
        assert_eq!(
            anchor_wrapped_rows(&transcript, &kinds, &styles, &runs, &[], &[], (8, 8), false, true, 40, None),
            None
        );
    }

    #[test]
    fn an_anchor_at_the_end_is_an_empty_screen_not_an_absent_one() {
        // SQ-0748: the game cleared the screen and printed nothing into it — Beyond
        // Zork's title repaint, which places every character in the upper window.
        // The anchor is then one past the last `starts` entry, and reading that as
        // "no anchor" bottom-sticks the scrollback the game just erased.
        let transcript = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let kinds = vec![TranscriptKind::Story; 3];
        let styles = vec![Style::default(); 3];
        let runs = vec![Vec::new(); 3];
        let rows = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &[], (8, 8), false, true, 40);
        let anchor = anchor_wrapped_rows(
            &transcript, &kinds, &styles, &runs, &[], &[], (8, 8), false, true, 40, Some(3),
        );
        assert_eq!(anchor, Some(3), "every row precedes the anchor");
        let (visible, total, first) = window_wrapped_rows(&rows, anchor, 10, 0);
        assert_eq!(
            (visible.len(), total, first),
            (0, 3, 3),
            "the post-clear screen is blank; the three erased rows stay in scrollback"
        );
        // Scrolling back still reaches them.
        let (back, _, _) = window_wrapped_rows(&rows, anchor, 2, 1);
        assert_eq!(back.len(), 2, "the erased screen is still reachable above the anchor");
    }

    #[test]
    fn wrapped_line_starts_index_every_source_line() {
        // The index the anchor rides on: one entry per source line, each the row its
        // output begins at. A wrapped line advances by its row count; a floated image
        // emits none of its own.
        let transcript = vec![String::new(), "word ".repeat(20).trim_end().to_string(), "tail".to_string()];
        let images = vec![Some(left_img(8, 40, None)), None, None];
        let kinds = vec![TranscriptKind::Story; 3];
        let styles = vec![Style::default(); 3];
        let runs = vec![Vec::new(); 3];
        let (rows, starts) =
            wrap_lines_kinded_indexed(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), true, true, 40);
        assert_eq!(starts.len(), 3, "one start per source line");
        assert_eq!(starts[0], 0);
        assert_eq!(starts[1], 0, "the floated picture emits no rows of its own");
        assert!(starts[2] > 0 && starts[2] <= rows.len(), "the tail starts after the wrapped paragraph");
        assert!(starts.windows(2).all(|w| w[0] <= w[1]), "starts are monotonic");
    }

    #[test]
    fn float_disabled_keeps_left_margin_as_band() {
        // With left_float off (the secondary-buffer-window path), a MarginLeft
        // image renders as a band exactly as before.
        let transcript = vec![String::new()];
        let images = vec![Some(left_img(16, 24, None))];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let runs = vec![Vec::new()];
        let rows = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), true, false, 40);
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| r.band.is_some() && r.float.is_none()));
    }

    fn fields_score() -> StatusFields {
        StatusFields {
            location: "West of House".into(),
            score: Some("10".into()),
            moves: Some("5".into()),
            time: None,
            turns: "7".into(),
            title: "Zork".into(),
            filter: String::new(),
        }
    }

    #[test]
    fn resolve_placeholders_substitutes_and_hides() {
        let f = fields_score();
        assert_eq!(resolve_placeholders("Score: {score}  Moves: {moves}", &f).as_deref(), Some("Score: 10  Moves: 5"));
        // pure literal always shown
        assert_eq!(resolve_placeholders(" | ", &f).as_deref(), Some(" | "));
        // all-empty placeholder segment hides (time is None on a score game)
        assert_eq!(resolve_placeholders("{time}", &f), None);
        // mixed: one empty, one non-empty placeholder → shown
        assert_eq!(resolve_placeholders("{time}{location}", &f).as_deref(), Some("West of House"));
        // unknown token → empty; all-empty → hidden
        assert_eq!(resolve_placeholders("{bogus}", &f), None);
        // turns vs moves are distinct
        assert_eq!(resolve_placeholders("{turns}/{moves}", &f).as_deref(), Some("7/5"));
    }

    #[test]
    fn pack_clusters_positions_and_truncates() {
        use crate::colors::Align;
        let s = Style::default();
        let mk = |t: &str, a: Align| (t.to_string(), s, a);
        // width 30: left "abc"(0), right "XY"(28)
        let ops = pack_status_clusters(&[mk("abc", Align::Left), mk("XY", Align::Right)], 30);
        let left = ops.iter().find(|(_, t, _)| t == "abc").unwrap();
        let right = ops.iter().find(|(_, t, _)| t == "XY").unwrap();
        assert_eq!(left.0, 0);
        assert_eq!(right.0, 28); // 30 - 2
        // center centered in the gap between left end (3) and right start (28): gap 25, center "cc"(2) at 3 + (25-2)/2 = 14
        let ops2 = pack_status_clusters(&[mk("abc", Align::Left), mk("cc", Align::Center), mk("XY", Align::Right)], 30);
        let center = ops2.iter().find(|(_, t, _)| t == "cc").unwrap();
        assert_eq!(center.0, 14);
        // narrow width 6: right "XY" preserved at 4; center dropped; left
        // "abcdef" truncated into the 4 cols before it — one word, so a hard
        // break plus the §8.2.2.2 ellipsis ("abc…")
        let ops3 = pack_status_clusters(&[mk("abcdef", Align::Left), mk("cc", Align::Center), mk("XY", Align::Right)], 6);
        assert!(ops3.iter().all(|(_, t, _)| t != "cc"), "center dropped under pressure");
        let right3 = ops3.iter().find(|(x, _, _)| *x == 4).unwrap();
        assert_eq!(right3.1, "XY");
        let left3 = ops3.iter().find(|(x, _, _)| *x == 0).unwrap();
        assert_eq!(left3.1, "abc…"); // 4 cols before the right cluster, ellipsis included
    }

    #[test]
    fn status_truncation_breaks_at_the_last_space_with_an_ellipsis() {
        // ZMSD §8.2.2.2: "If the object's short name exceeds the available room
        // on the status line, the author suggests that an interpreter should
        // break it at the last space and append an ellipsis."
        assert_eq!(truncate_status_text("West of House", 13), "West of House", "fits → untouched");
        assert_eq!(truncate_status_text("West of House", 20), "West of House");
        assert_eq!(truncate_status_text("West of House", 12), "West of…", "breaks at the last space");
        assert_eq!(truncate_status_text("Cyclops Room", 8), "Cyclops…");
        assert_eq!(truncate_status_text("Antechamber", 6), "Antec…", "one long word → hard break");
        assert_eq!(truncate_status_text("Antechamber", 1), "…");
        assert_eq!(truncate_status_text("Antechamber", 0), "");
        for (name, w) in [("West of House", 12), ("Cyclops Room", 8), ("Antechamber", 6)] {
            assert!(truncate_status_text(name, w).chars().count() <= w, "never exceeds the budget");
        }
    }

    #[test]
    fn render_status_default_bar_matches_today() {
        // With no custom [statusbar], the bar shows location left and the filter
        // indicator right; score/moves come from the (empty) minimal machine.
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.transcript_filter = crate::state::TranscriptFilter::Story;

        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        let row: String = (0..40u16)
            .map(|x| buf.cell((x, 0)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        // filter indicator pinned right (default bar includes ` {filter}`).
        assert!(row.contains("[filter: story]"), "default bar must show the filter indicator: {:?}", row);
        // status row keeps the reversed-video base fill.
        assert!(buf.cell((0, 0)).unwrap().modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn suggestions_render_boxed_when_styled() {
        // With the suggestion_line border configured, the auto-complete popup is
        // drawn as a framed mini-window: a border glyph appears and the suggestion
        // text renders inside it.
        use crate::render::paneframe::PaneSides;
        let machine = minimal_machine();
        let mut state = AppState::default();
        // SQ-0542: the bar is the COMMAND PALETTE's presentation now — a line
        // starting with the command prefix. Story-word completions ghost instead.
        state.input.set("/pan", true);
        state.suggestions = vec!["panh".into(), "panv".into()];
        state.suggestion_idx = 0;
        state.colors.suggestion_line_style = BorderStyle::Single;
        state.colors.suggestion_line_sides = PaneSides::all(BorderStyle::Single);

        let area = Rect::new(0, 0, 40, 8);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        let mut has_border = false;
        let mut has_text = false;
        for y in 0..area.height {
            let row: String = (0..area.width)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect();
            if row.contains('│') { has_border = true; }
            if row.contains("panh") { has_text = true; }
        }
        assert!(has_border, "boxed suggestion popup must draw a border glyph");
        assert!(has_text, "suggestion text must render inside the box");
    }

    #[test]
    fn suggestions_stay_inline_when_box_off() {
        // With the default (off) suggestion_line border, the popup stays the flat
        // one-row strip: the text renders but no box chrome is drawn.
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.input.set("/pan", true);
        state.suggestions = vec!["panh".into(), "panv".into()];
        state.suggestion_idx = 0;

        let area = Rect::new(0, 0, 40, 8);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        let mut has_border = false;
        let mut has_text = false;
        for y in 0..area.height {
            let row: String = (0..area.width)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect();
            if row.contains('│') { has_border = true; }
            if row.contains("panh") { has_text = true; }
        }
        assert!(!has_border, "inline suggestion strip must not draw box chrome");
        assert!(has_text, "suggestion text must still render inline");
    }

    #[test]
    fn visible_lines_newest_at_bottom() {
        let transcript: Vec<String> = (0..10).map(|i| format!("line {}", i)).collect();
        // 5 rows, scroll 0 → last 5 lines: line5..line9
        let vis = visible_lines(&transcript, 5, 0);
        assert_eq!(vis.len(), 5);
        assert_eq!(vis[4], "line 9");
        assert_eq!(vis[0], "line 5");
    }

    #[test]
    fn visible_lines_scroll_up() {
        let transcript: Vec<String> = (0..10).map(|i| format!("line {}", i)).collect();
        // 5 rows, scroll 2 → lines 3..7 (end = 10-2=8, start = 8-5=3)
        let vis = visible_lines(&transcript, 5, 2);
        assert_eq!(vis.len(), 5);
        assert_eq!(vis[0], "line 3");
        assert_eq!(vis[4], "line 7");
    }

    #[test]
    fn visible_lines_fewer_than_rows() {
        let transcript = vec!["only one".to_string()];
        let vis = visible_lines(&transcript, 5, 0);
        assert_eq!(vis.len(), 1);
        assert_eq!(vis[0], "only one");
    }

    #[test]
    fn truncate_line_clips_at_width() {
        assert_eq!(truncate_line("hello world", 5), "hello");
        assert_eq!(truncate_line("hi", 10), "hi");
        assert_eq!(truncate_line("abc", 3), "abc");
    }

    #[test]
    fn draw_str_runs_applies_span_modifier() {
        use ratatui::{buffer::Buffer, layout::Rect, style::{Modifier, Style}};
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        let runs = vec![StyleRun { start: 2, end: 4, bits: 0x02, fg: 0, bg: 0, link: 0, glk_style: 0 }]; // bold chars 2..4
        draw_str_runs(&mut buf, 0, 0, "abcdef", Style::default(), &runs, None, area, crate::render::TextInk::new(false, &crate::colors::ColorScheme::terminal_default()));
        assert!(!buf[(0, 0)].modifier.contains(Modifier::BOLD));
        assert!(buf[(2, 0)].modifier.contains(Modifier::BOLD));
        assert!(buf[(3, 0)].modifier.contains(Modifier::BOLD));
        assert!(!buf[(4, 0)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn draw_str_runs_hyperlink_underlines_and_colors() {
        use ratatui::{buffer::Buffer, layout::Rect, style::{Color, Modifier, Style}};
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        let mut cs = crate::colors::ColorScheme::terminal_default();
        cs.theme = theme_with_overrides(&[("hyperlink", Color::Magenta)]);
        // chars 2..5 carry link 7 (bold too, to prove the link layers on top).
        let runs = vec![StyleRun { start: 2, end: 5, bits: 0x02, fg: 0, bg: 0, link: 7, glk_style: 0 }];
        draw_str_runs(&mut buf, 0, 0, "abcdefgh", Style::default(), &runs, None, area, crate::render::TextInk::new(true, &cs));
        for x in 2..5u16 {
            assert!(buf[(x, 0)].modifier.contains(Modifier::UNDERLINED), "linked cell {x} underlined");
            assert_eq!(buf[(x, 0)].fg, Color::Magenta, "linked cell {x} uses hyperlink fg");
            assert!(buf[(x, 0)].modifier.contains(Modifier::BOLD), "linked cell {x} keeps its bold");
        }
        // Unlinked neighbours: no underline, no hyperlink colour.
        assert!(!buf[(1, 0)].modifier.contains(Modifier::UNDERLINED));
        assert_ne!(buf[(1, 0)].fg, Color::Magenta);
        assert!(!buf[(5, 0)].modifier.contains(Modifier::UNDERLINED));
        assert_ne!(buf[(5, 0)].fg, Color::Magenta);
    }

    #[test]
    fn draw_str_runs_glk_style_slots_seed_and_gate() {
        use ratatui::{buffer::Buffer, layout::Rect, style::{Color, Style}};
        let area = Rect::new(0, 0, 6, 1);
        let base = Style::new().fg(Color::White); // element = transcript White
        let mut cs = crate::colors::ColorScheme::terminal_default();
        // Seed buffer (row 0): Input(8) cyan, Subheader(4) green.
        cs.glk_styles[0][8] = Style::default().fg(Color::Cyan);
        cs.glk_styles[0][4] = Style::default().fg(Color::Green);

        let draw = |glk_style: u8, fg: u32, honor: bool| {
            let mut b = Buffer::empty(area);
            let runs = vec![StyleRun { start: 0, end: 3, bits: 0, fg, bg: 0, link: 0, glk_style }];
            draw_str_runs(&mut b, 0, 0, "abc", base, &runs, None, area, crate::render::TextInk::new(honor, &cs));
            b[(0, 0)].fg
        };

        // Input run, no game colour → input_text (cyan) in BOTH gate states.
        assert_eq!(draw(8, 0, false), Color::Cyan, "Input slot applies (honor off)");
        assert_eq!(draw(8, 0, true), Color::Cyan, "Input slot applies (honor on)");
        // Subheader run → transcript_location (green).
        assert_eq!(draw(4, 0, false), Color::Green, "Subheader slot applies");
        // Normal run (empty runs) → element base (white).
        let mut b = Buffer::empty(area);
        draw_str_runs(&mut b, 0, 0, "abc", base, &[], None, area, crate::render::TextInk::new(false, &cs));
        assert_eq!(b[(0, 0)].fg, Color::White, "Normal → element base");

        // honor gate: a game-set red fg on an Input run — honor ON → game red wins
        // over the slot; honor OFF → game IGNORED, slot cyan shows.
        let red = crate::state::pack_zcolour(zvm::screen::ZColour::True24(0x00FF_0000));
        assert_eq!(draw(8, red, true), Color::Rgb(255, 0, 0), "honor ON: game colour wins over slot");
        assert_eq!(draw(8, red, false), Color::Cyan, "honor OFF: game ignored, slot wins");
    }

    #[test]
    fn glk_buffer_emphasized_renders_italic() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        let area = Rect::new(0, 0, 6, 1);
        let base = Style::new().fg(Color::White);
        let cs = crate::colors::ColorScheme::terminal_default();

        let draw = |glk_style: u8| {
            let mut b = Buffer::empty(area);
            let runs = vec![StyleRun { start: 0, end: 3, bits: 0, fg: 0, bg: 0, link: 0, glk_style }];
            draw_str_runs(&mut b, 0, 0, "abc", base, &runs, None, area, crate::render::TextInk::new(false, &cs));
            b[(0, 0)].modifier
        };

        // Emphasized (glk_style 1) → registry theme's italic modifier.
        assert!(draw(1).contains(Modifier::ITALIC), "Emphasized run renders italic");
        // Normal (glk_style 0) → no italic.
        assert!(!draw(0).contains(Modifier::ITALIC), "Normal run has no italic");
    }

    #[test]
    fn glk_buffer_header_renders_bold() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        let area = Rect::new(0, 0, 6, 1);
        let base = Style::new().fg(Color::White);
        let cs = crate::colors::ColorScheme::terminal_default();

        let mut b = Buffer::empty(area);
        let runs = vec![StyleRun { start: 0, end: 3, bits: 0, fg: 0, bg: 0, link: 0, glk_style: 3 }];
        draw_str_runs(&mut b, 0, 0, "abc", base, &runs, None, area, crate::render::TextInk::new(false, &cs));
        assert!(b[(0, 0)].modifier.contains(Modifier::BOLD), "Header run renders bold");
    }

    #[test]
    fn render_transcript_builds_cell_link_map() {
        use zvm::screen::ZColour;
        use ratatui::style::Modifier;
        let machine = minimal_machine();
        let mut state = AppState::default();
        // A linked line ("northgate" → link 42) followed by a plain line.
        state.push_transcript_runs("northgate", TranscriptKind::Story, &[(9, 0, ZColour::Default, ZColour::Default, 42, ParaFmt::default(), 0, false)]);
        state.push_transcript("plain text");
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        let links = render_transcript(
            &crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None,
        )
        .links;

        // Exactly the 9 linked chars map to 42; the plain line contributes none.
        assert_eq!(links.len(), 9, "one entry per linked char, none from plain text");
        assert!(links.iter().all(|(_, v)| *v == 42), "all entries carry the link value");
        // Every recorded cell is where a linked glyph actually rendered — proves
        // the map coordinates line up with the rendered cells.
        for &((cx, cy), _) in &links {
            let cell = buf.cell((cx, cy)).unwrap();
            assert_ne!(cell.symbol(), " ", "linked cell holds a glyph");
            assert!(cell.modifier.contains(Modifier::UNDERLINED), "linked cell underlined");
        }
    }

    #[test]
    fn game_background_fills_to_row_end() {
        // A short line with a game-set white background (like CM's black-on-white)
        // must paint the whole row width, not just behind the glyphs. (SQ-0263)
        use zvm::screen::ZColour;
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.honor_game_colours = true;
        state.push_transcript_runs(
            "hi", TranscriptKind::Story,
            &[(2, 0, ZColour::True24(0), ZColour::True24(0x00FF_FFFF), 0, ParaFmt::default(), 0, false)],
        );
        state.focus = Focus::Game;
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        let white = ratatui::style::Color::Rgb(255, 255, 255);
        let mut found = false;
        for y in 0..10u16 {
            if buf.cell((0, y)).unwrap().symbol() == "h" {
                found = true;
                assert_eq!(buf.cell((20, y)).unwrap().bg, white, "trailing space fills with the game bg");
                assert_eq!(buf.cell((37, y)).unwrap().bg, white, "fill reaches the body's right edge");
            }
        }
        assert!(found, "rendered the coloured line");
    }

    #[test]
    fn game_background_fills_blank_rows_within_a_band() {
        // Two black-on-white paragraphs separated by a blank line: the blank line
        // between them must also fill white, so the band is contiguous. (SQ-0263)
        use zvm::screen::ZColour;
        let white_run = |n: usize| (n, 0u8, ZColour::True24(0), ZColour::True24(0x00FF_FFFF), 0u32, ParaFmt::default(), 0, false);
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.honor_game_colours = true;
        state.push_transcript_runs(
            "para1\n\npara2", TranscriptKind::Story,
            &[white_run(5), white_run(2), white_run(5)],
        );
        state.focus = Focus::Game;
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        let white = ratatui::style::Color::Rgb(255, 255, 255);
        let mut blank_white = false;
        for y in 0..10u16 {
            let text: String = (0..30).map(|x| buf.cell((x, y)).unwrap().symbol().to_owned()).collect();
            if text.trim().is_empty()
                && buf.cell((5, y)).unwrap().bg == white
                && buf.cell((20, y)).unwrap().bg == white
            {
                blank_white = true;
            }
        }
        assert!(blank_white, "the blank line inside the white band is filled white");
    }

    #[test]
    fn default_background_line_leaves_trailing_untouched() {
        // A normal (Default-bg) line must NOT get a trailing fill — the feature is
        // scoped to game-set backgrounds so ordinary games are unaffected. (SQ-0263)
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.honor_game_colours = true;
        state.push_transcript("hi");
        state.focus = Focus::Game;
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        let white = ratatui::style::Color::Rgb(255, 255, 255);
        for y in 0..10u16 {
            if buf.cell((0, y)).unwrap().symbol() == "h" {
                assert_ne!(buf.cell((37, y)).unwrap().bg, white, "no game bg → no trailing fill");
            }
        }
    }

    #[test]
    fn draw_str_runs_empty_matches_clipped() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        let area = Rect::new(0, 0, 10, 1);
        let mut a = Buffer::empty(area);
        let mut b = Buffer::empty(area);
        draw_str_runs(&mut a, 0, 0, "hello", Style::default(), &[], None, area, crate::render::TextInk::new(false, &crate::colors::ColorScheme::terminal_default()));
        crate::render::draw_str_clipped(&mut b, 0, 0, "hello", Style::default(), area);
        assert_eq!(a, b, "empty runs render identically to draw_str_clipped");
    }

    #[test]
    fn draw_str_runs_empty_with_search_matches_highlighted() {
        use ratatui::{buffer::Buffer, layout::Rect, style::{Color, Style}};
        let area = Rect::new(0, 0, 20, 1);
        let base = Style::default();
        let hl = Style::new().fg(Color::Black).bg(Color::Yellow);
        let mut a = Buffer::empty(area);
        let mut b = Buffer::empty(area);
        draw_str_runs(&mut a, 0, 0, "the cat sat", base, &[], Some(("cat", hl)), area, crate::render::TextInk::new(false, &crate::colors::ColorScheme::terminal_default()));
        draw_str_highlighted(&mut b, 0, 0, "the cat sat", base, "cat", hl, area);
        assert_eq!(a, b, "empty runs + search render identically to draw_str_highlighted");
    }

    #[test]
    fn wrap_line_ranges_round_trips_word_wrap() {
        // Same row strings as wrap_line, plus correct source char ranges.
        let rows = wrap_line_ranges("AAAAA BBBBB", 5);
        assert_eq!(rows.iter().map(|(s, _, _)| s.clone()).collect::<Vec<_>>(),
                   wrap_line("AAAAA BBBBB", 5));
        // first row covers chars 0..5, second covers 6..11 (break space dropped)
        assert_eq!((rows[0].1, rows[0].2), (0, 5));
        assert_eq!((rows[1].1, rows[1].2), (6, 11));
    }

    #[test]
    fn wrap_lines_kinded_rebases_runs_per_row() {
        let lines = vec!["AAAAA BBBBB".to_string()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let runs = vec![vec![StyleRun { start: 6, end: 11, bits: 0x02, fg: 0, bg: 0, link: 0, glk_style: 0 }]]; // bold "BBBBB"
        let out = wrap_lines_kinded(&lines, &kinds, &styles, &runs, &[], &[], (1, 1), false, false, 5);
        // row 0 ("AAAAA", 0..5) → no runs; row 1 ("BBBBB", 6..11) → bold 0..5
        assert!(out[0].runs.is_empty());
        assert_eq!(out[1].runs, vec![StyleRun { start: 0, end: 5, bits: 0x02, fg: 0, bg: 0, link: 0, glk_style: 0 }]);
    }

    /// SQ-0827: the margin a left float reserves takes the prose's BACKGROUND and
    /// nothing else, and only when the prose names one.
    #[test]
    fn margin_ground_run_copies_only_the_proses_background() {
        let bg = crate::state::pack_zcolour(zvm::screen::ZColour::Standard(9));
        // A reversed, bold, linked run beside the margin: only its bg travels.
        let runs = vec![StyleRun { start: 4, end: 9, bits: 0x03, fg: 7, bg, link: 42, glk_style: 3 }];
        assert_eq!(
            margin_ground_run(&runs, 4),
            Some(StyleRun { start: 0, end: 4, bits: 0, fg: 0, bg, link: 0, glk_style: 0 })
        );
        // Prose on the inherited background: nothing to copy, so the margin keeps
        // inheriting too — every non-Amiga frame takes this arm.
        let plain = vec![StyleRun { start: 4, end: 9, bits: 0, fg: 0, bg: 0, link: 0, glk_style: 0 }];
        assert_eq!(margin_ground_run(&plain, 4), None);
        // No margin, or no runs at all: nothing to do.
        assert_eq!(margin_ground_run(&runs, 0), None);
        assert_eq!(margin_ground_run(&[], 4), None);
        // A run that starts PAST the margin (the row's prose begins mid-run) is
        // still the ground the margin abuts, via the `first()` fallback.
        let later = vec![StyleRun { start: 6, end: 9, bits: 0, fg: 0, bg, link: 0, glk_style: 0 }];
        assert_eq!(margin_ground_run(&later, 4).map(|r| (r.start, r.end, r.bg)), Some((0, 4, bg)));
    }

    /// …and the wrap really does hand the margin that run, ahead of the prose's
    /// own (SQ-0827). A `MarginLeft` picture 2 cells wide with a 4-cell margin:
    /// the row's text is padded by 4, its own run shifts to 4.., and a new run
    /// covers 0..4 carrying the prose's page.
    #[test]
    fn wrap_lines_kinded_grounds_a_floats_reserved_margin() {
        let bg = crate::state::pack_zcolour(zvm::screen::ZColour::Standard(9));
        let img = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::new(2, 2)),
            align: crate::inline_image::ImageAlign::MarginLeft,
            scaled: None,
            margin_px: Some(4),
        };
        let lines = vec![String::new(), "AAAA".to_string()];
        let kinds = vec![TranscriptKind::Story; 2];
        let styles = vec![Style::default(); 2];
        let runs = vec![vec![], vec![StyleRun { start: 0, end: 4, bits: 0, fg: 0, bg, link: 0, glk_style: 0 }]];
        let images = vec![Some(img), None];
        let out = wrap_lines_kinded(&lines, &kinds, &styles, &runs, &[], &images, (1, 1), true, true, 20);
        let row = out.iter().find(|r| r.text.ends_with("AAAA")).expect("prose flows beside the float");
        assert_eq!(row.text, "    AAAA", "the float reserves 4 columns of leading pad");
        assert_eq!(
            row.runs,
            vec![
                StyleRun { start: 0, end: 4, bits: 0, fg: 0, bg, link: 0, glk_style: 0 },
                StyleRun { start: 4, end: 8, bits: 0, fg: 0, bg, link: 0, glk_style: 0 },
            ],
            "the reserved margin carries the prose's own ground"
        );
    }

    // ── SQ-0538 / ZMSD §7.2.1: buffer_mode off ⇒ char-break, never word-wrap ──

    /// Helper: a line whose text is unbuffered from char `from` onwards.
    fn nowrap_para(from: u32) -> ParaFmt {
        ParaFmt { nowrap_from: Some(from), ..ParaFmt::default() }
    }

    fn wrapped(line: &str, width: u16, pf: ParaFmt) -> Vec<String> {
        let lines = vec![line.to_string()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        wrap_lines_kinded(&lines, &kinds, &styles, &[], &[pf], &[], (1, 1), false, false, width)
            .into_iter()
            .map(|r| r.text)
            .collect()
    }

    #[test]
    fn unbuffered_line_breaks_after_last_char_that_fits() {
        // Buffered (the default) word-wraps; unbuffered breaks at the column.
        assert_eq!(wrapped("AAAAA BBBBB", 8, ParaFmt::default()), vec!["AAAAA", "BBBBB"]);
        assert_eq!(wrapped("AAAAA BBBBB", 8, nowrap_para(0)), vec!["AAAAA BB", "BBB"]);
    }

    #[test]
    fn unbuffered_long_word_splits_mid_word() {
        // A word longer than the width hard-breaks either way; the point is that
        // an unbuffered line never carries a partial word to the next row.
        assert_eq!(wrapped("Please wait...........", 10, nowrap_para(0)),
                   vec!["Please wai", "t.........", ".."]);
    }

    #[test]
    fn buffering_back_on_resumes_word_wrap() {
        // Same text, fully buffered again → ordinary word-wrap.
        assert_eq!(wrapped("Please wait...........", 10, ParaFmt::default()),
                   vec!["Please", "wait......", "....."]);
    }

    /// Documented mixed-line rule: rows that lie entirely inside the buffered
    /// prefix word-wrap; the first row reaching into the unbuffered region — and
    /// every row after it — char-breaks.
    #[test]
    fn mixed_line_word_wraps_until_buffering_turns_off() {
        // "one two " is buffered (8 chars); "xxxxxxxxxxxx" is not.
        let rows = wrapped("one two xxxxxxxxxxxx", 8, nowrap_para(8));
        assert_eq!(rows, vec!["one two", "xxxxxxxx", "xxxx"]);
    }

    #[test]
    fn unbuffered_wrap_ranges_stay_contiguous_for_selection() {
        // Selection/copy geometry (SQ-0197) re-bases runs from these ranges, so a
        // char-broken row must cover its source chars with no gaps.
        let rows = wrap_line_ranges_nw("AAAAA BBBBB", 8, Some(0));
        assert_eq!(rows.iter().map(|r| (r.1, r.2)).collect::<Vec<_>>(), vec![(0, 8), (8, 11)]);
        assert_eq!(rows.iter().map(|r| r.0.chars().count()).sum::<usize>(), 11,
                   "no break space is swallowed when buffering is off");
    }

    // ── SQ-0662: the body fits, draws and selects in display CELLS ────────────

    #[test]
    fn wide_glyphs_wrap_by_cells_not_chars() {
        // A 10-cell pane fits FIVE CJK ideographs, not ten: each costs two columns.
        // Fitting by char count packed all seven onto one row, and the terminal's
        // wide-glyph cell skip then dropped every second one.
        let rows = wrap_line("日本語です朝日", 10);
        assert_eq!(rows, vec!["日本語です", "朝日"]);
        for r in &rows {
            assert!(crate::textwidth::str_cells(r) <= 10, "row {r:?} fits the cell budget");
        }
        // ASCII is untouched: one char, one cell.
        assert_eq!(wrap_line("ABCDEFGHIJKL", 10), vec!["ABCDEFGHIJ", "KL"]);
    }

    #[test]
    fn a_wide_glyph_that_half_fits_moves_whole_to_the_next_row() {
        // Odd pane width: the 5th ideograph would straddle columns 8/9, so it goes
        // whole to the next row rather than being split (or silently eaten).
        let rows = wrap_line_ranges("日本語です朝日", 9);
        assert_eq!(rows.iter().map(|r| r.0.clone()).collect::<Vec<_>>(), vec!["日本語で", "す朝日"]);
        for (t, _, _) in &rows {
            assert!(crate::textwidth::str_cells(t) <= 9, "no row overflows the pane");
        }
        // The source char ranges stay contiguous — style runs re-base off them.
        assert_eq!(rows.iter().map(|r| (r.1, r.2)).collect::<Vec<_>>(), vec![(0, 4), (4, 7)]);
        assert_eq!(rows.iter().map(|r| r.0.clone()).collect::<String>(), "日本語です朝日");
    }

    #[test]
    fn mixed_width_line_word_wraps_by_cells() {
        // "ab 日本語" is 3 + 6 = 9 cells: in an 8-cell pane the CJK word no longer
        // fits after "ab ", so it wraps at the space. By char count it was 6 chars
        // and stayed on one row, overflowing the pane.
        assert_eq!(wrap_line("ab 日本語", 8), vec!["ab", "日本語"]);
    }

    #[test]
    fn a_wide_glyph_in_a_one_cell_pane_still_makes_progress() {
        // Nothing fits a 1-column pane, but the wrap must terminate: the glyph
        // takes its own (overflowing) row and the draw clips it.
        let rows = wrap_line("日本", 1);
        assert_eq!(rows, vec!["日", "本"]);
    }

    #[test]
    fn justification_measures_the_row_in_cells() {
        // "日本" is 4 cells; centred in 10 → (10-4)/2 = 3 leading spaces. Measured
        // by char count it was 2 "wide", and the row came out pushed 4 right.
        let lines = vec!["日本".to_string()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let para = vec![ParaFmt { indent: 0, para_indent: 0, justify: 2, nowrap_from: None }];
        let out = wrap_lines_kinded(&lines, &kinds, &styles, &[], &para, &[], (1, 1), false, false, 10);
        assert_eq!(out[0].text, "   日本");
        assert_eq!(crate::textwidth::str_cells(&out[0].text), 7, "3 pad + 4 text cells");
    }

    #[test]
    fn prose_beside_a_float_wraps_by_cells_too() {
        // The float's own geometry was always in cells (`fitted_cells`), but the
        // prose that rides beside it goes through `wrap_line_ranges_var` — which
        // fitted by char count, so a CJK paragraph overran the picture's column and
        // the viewport. 16x24 px at 8x8 cells → 2 cols wide, indent/reserve 3, so
        // the covered rows have 37 cells for text after a 3-cell pad.
        let para: String = "日".repeat(30);
        let full_transcript = vec![String::new(), para];
        let full_images = vec![Some(left_img(16, 24, None)), None];
        let kinds = vec![TranscriptKind::Story; 2];
        let styles = vec![Style::default(); 2];
        let runs = vec![Vec::new(); 2];
        let rows = wrap_lines_kinded(&full_transcript, &kinds, &styles, &runs, &[], &full_images, (8, 8), true, true, 40);
        for r in &rows {
            assert!(crate::textwidth::str_cells(&r.text) <= 40, "row {:?} fits the viewport", r.text);
        }
        let first = &rows[0];
        assert!(first.text.starts_with("   "), "float-row prose indented past the picture");
        assert_eq!(first.text.chars().filter(|c| *c == '日').count(), 18,
                   "18 ideographs = 36 cells, the most that fit in the 37-cell narrowed column");
    }

    #[test]
    fn draw_str_runs_paints_both_cells_of_a_wide_glyph() {
        let area = Rect::new(0, 0, 10, 1);
        let mut b = Buffer::empty(area);
        let cs = crate::colors::ColorScheme::terminal_default();
        draw_str_runs(&mut b, 0, 0, "日本x", Style::default(), &[], None, area, crate::render::TextInk::new(false, &cs));
        assert_eq!(b[(0, 0)].symbol(), "日");
        assert_eq!(b[(1, 0)].symbol(), " ", "the wide glyph's trailing cell is blanked…");
        assert_eq!(b[(2, 0)].symbol(), "本", "…so the next glyph is not swallowed by the cell skip");
        assert_eq!(b[(3, 0)].symbol(), " ");
        assert_eq!(b[(4, 0)].symbol(), "x");
    }

    #[test]
    fn draw_str_runs_keeps_a_combining_mark_with_its_base() {
        // "e" + U+0301 is ONE cell: the mark joins the base's cell instead of
        // claiming a column of its own (which shifted everything after it right).
        let area = Rect::new(0, 0, 10, 1);
        let mut b = Buffer::empty(area);
        let cs = crate::colors::ColorScheme::terminal_default();
        draw_str_runs(&mut b, 0, 0, "e\u{0301}!", Style::default(), &[], None, area, crate::render::TextInk::new(false, &cs));
        assert_eq!(b[(0, 0)].symbol(), "e\u{0301}");
        assert_eq!(b[(1, 0)].symbol(), "!");
    }

    #[test]
    fn draw_str_runs_styles_the_cells_of_the_char_its_run_covers() {
        // Runs stay CHAR-indexed (the wrap re-bases them that way); the cells they
        // paint are display columns. A bold run on char 1 of "日本x" must land on
        // columns 2..3, not on column 1.
        let area = Rect::new(0, 0, 10, 1);
        let mut b = Buffer::empty(area);
        let cs = crate::colors::ColorScheme::terminal_default();
        let runs = vec![StyleRun { start: 1, end: 2, bits: 0x02, fg: 0, bg: 0, link: 0, glk_style: 0 }];
        draw_str_runs(&mut b, 0, 0, "日本x", Style::default(), &runs, None, area, crate::render::TextInk::new(false, &cs));
        assert!(!b[(0, 0)].modifier.contains(Modifier::BOLD));
        assert!(b[(2, 0)].modifier.contains(Modifier::BOLD), "run lands on 本's own cell");
        assert!(b[(3, 0)].modifier.contains(Modifier::BOLD), "…and on its trailing cell");
        assert!(!b[(4, 0)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn render_transcript_link_cells_follow_a_wide_prefix() {
        use zvm::screen::ZColour;
        // "日本 north" links only "north": chars 3..8, but COLUMNS 5..10 — the two
        // ideographs are two cells each. Mapping the link by char index put the
        // clickable cells two columns left of the word.
        for honor in [true, false] {
            let machine = minimal_machine();
            let mut state = AppState::default();
            state.config.honor_game_colours = honor;
            state.push_transcript_runs(
                "日本 north",
                TranscriptKind::Story,
                &[
                    (3, 0, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false),
                    (5, 0, ZColour::Default, ZColour::Default, 7, ParaFmt::default(), 0, false),
                ],
            );
            state.focus = Focus::Game;
            let area = Rect::new(0, 0, 40, 10);
            let mut buf = Buffer::empty(area);
            let links = render_transcript(
                &crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None,
            )
            .links;
            assert_eq!(links.len(), 5, "one cell per linked char (honor={honor})");
            let cols: Vec<u16> = links.iter().map(|((c, _), _)| *c).collect();
            assert_eq!(cols, vec![5, 6, 7, 8, 9], "link cells sit on north's glyphs");
            for &((cx, cy), v) in &links {
                assert_eq!(v, 7);
                assert!("north".contains(buf.cell((cx, cy)).unwrap().symbol()),
                        "cell ({cx},{cy}) holds a glyph of the link");
            }
        }
    }

    #[test]
    fn cjk_story_line_renders_both_cells_of_every_glyph() {
        // The char==cell body wrote 日 at col 0 and 本 at col 1; the terminal skips
        // the cell after a wide glyph, so 本 was never seen. Both cells now belong
        // to their glyph.
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.push_transcript("日本語");
        state.focus = Focus::Game;
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);
        let row_y = (0..10u16)
            .find(|&y| buf.cell((0, y)).map(|c| c.symbol()) == Some("日"))
            .expect("the CJK line rendered");
        assert_eq!(buf.cell((1, row_y)).unwrap().symbol(), " ");
        assert_eq!(buf.cell((2, row_y)).unwrap().symbol(), "本");
        assert_eq!(buf.cell((3, row_y)).unwrap().symbol(), " ");
        assert_eq!(buf.cell((4, row_y)).unwrap().symbol(), "語");
    }

    #[test]
    fn selection_highlight_covers_exactly_the_cells_copy_takes() {
        use ratatui::style::Modifier;
        // The highlight and `clipboard::extract` share one coordinate system now,
        // so they must agree on a CJK row: dragging from the SECOND cell of 日 to
        // the FIRST cell of 語 copies all three glyphs whole, and the highlight has
        // to paint all six of their cells — the char-indexed highlight reversed
        // four cells while the (cell-based) copy took three glyphs.
        for honor in [true, false] {
            let machine = minimal_machine();
            let mut state = AppState::default();
            state.config.honor_game_colours = honor;
            // The bottom input bar keeps the block caret (also reverse video) off
            // the transcript row, so every reversed cell here is the selection.
            state.config.command_bar = true;
            state.push_transcript("日本語です");
            assert_eq!(state.transcript.len(), 1, "one logical line → wrapped row 0");
            state.focus = Focus::Game;
            state.selection = Some(crate::clipboard::Selection {
                anchor: crate::clipboard::Point { row: 0, col: 1 },
                head: crate::clipboard::Point { row: 0, col: 4 },
            });
            let area = Rect::new(0, 0, 40, 10);
            let mut buf = Buffer::empty(area);
            render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

            let copied = state.selection_text.borrow().clone().expect("copy published");
            assert_eq!(copied, "日本語", "a clipped wide glyph copies whole");
            let row_y = (0..10u16)
                .find(|&y| buf.cell((0, y)).map(|c| c.symbol()) == Some("日"))
                .expect("the CJK line rendered");
            let reversed: Vec<u16> = (0..40u16)
                .filter(|&x| buf.cell((x, row_y)).unwrap().modifier.contains(Modifier::REVERSED))
                .collect();
            assert_eq!(reversed, (0..6).collect::<Vec<u16>>(),
                       "highlight spans exactly the copied glyphs' cells (honor={honor})");
            assert_eq!(reversed.len(), crate::textwidth::str_cells(&copied),
                       "highlighted cell count == copied text's display width");
        }
    }

    #[test]
    fn meta_row_selection_highlight_and_copy_share_the_gutter_offset() {
        use ratatui::style::Modifier;
        // SQ-0665: a Meta/Warning row draws its TEXT at body_area.x + META_GUTTER
        // (the marker glyph + a blank cell occupy the first two columns), but
        // `wr.text` holds ONLY the text — no gutter prefix. A selection column is
        // always relative to the row's own left edge (screen col 0 = body_area.x),
        // so both the highlight and the copy must shift by META_GUTTER before
        // they mean anything against `wr.text`, or they land on the wrong glyphs.
        for honor in [true, false] {
            let machine = minimal_machine();
            let mut state = AppState::default();
            state.config.honor_game_colours = honor;
            // Keep the block caret (also reverse video) off this row.
            state.config.command_bar = true;
            state.push_transcript_kind("hello world", TranscriptKind::Meta);
            state.focus = Focus::Game;
            // Screen columns 2..=6 (past the 2-col gutter) should select "hello".
            state.selection = Some(crate::clipboard::Selection {
                anchor: crate::clipboard::Point { row: 0, col: 2 },
                head: crate::clipboard::Point { row: 0, col: 6 },
            });
            let area = Rect::new(0, 0, 40, 10);
            let mut buf = Buffer::empty(area);
            render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

            // v3's status bar occupies screen row 0, so find the Meta row rather
            // than assume it's first.
            let row_y = (0..10u16)
                .find(|&y| buf.cell((2, y)).map(|c| c.symbol()) == Some("h"))
                .expect("the Meta line rendered");
            // The marker glyph drew at col 0; "hello world" starts at col 2.
            assert_eq!(buf.cell((0, row_y)).unwrap().symbol(), "▏", "gutter marker drawn (honor={honor})");

            let copied = state.selection_text.borrow().clone().expect("copy published");
            assert_eq!(copied, "hello",
                       "copy matches the glyphs under the highlight, not shifted by the gutter (honor={honor})");

            let reversed: Vec<u16> = (0..40u16)
                .filter(|&x| buf.cell((x, row_y)).unwrap().modifier.contains(Modifier::REVERSED))
                .collect();
            assert_eq!(reversed, (2..=6).collect::<Vec<u16>>(),
                       "highlight lands on the drawn glyph cells, gutter excluded (honor={honor})");
        }
    }

    #[test]
    fn meta_row_gutter_click_selects_from_the_first_text_cell() {
        // A selection edge that lands IN the gutter (screen col 0, before the
        // marker's blank cell at col 1 and the text at col 2) must not copy or
        // highlight any of it — the gutter is never itself selectable.
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.command_bar = true;
        state.push_transcript_kind("hello", TranscriptKind::Meta);
        state.focus = Focus::Game;
        // Anchor sits IN the gutter (col 0); head is on the 3rd text glyph.
        state.selection = Some(crate::clipboard::Selection {
            anchor: crate::clipboard::Point { row: 0, col: 0 },
            head: crate::clipboard::Point { row: 0, col: 4 },
        });
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        let copied = state.selection_text.borrow().clone().expect("copy published");
        assert_eq!(copied, "hel", "a gutter-anchored selection starts at the first text cell, not the marker");

        use ratatui::style::Modifier;
        // v3's status bar occupies screen row 0, so find the Meta row rather than
        // assume it's first.
        let row_y = (0..10u16)
            .find(|&y| buf.cell((2, y)).map(|c| c.symbol()) == Some("h"))
            .expect("the Meta line rendered");
        assert!(!buf.cell((0, row_y)).unwrap().modifier.contains(Modifier::REVERSED),
                 "the gutter marker cell itself is never highlighted");
        assert!(!buf.cell((1, row_y)).unwrap().modifier.contains(Modifier::REVERSED),
                 "the gutter's blank cell is never highlighted");
        assert!(buf.cell((2, row_y)).unwrap().modifier.contains(Modifier::REVERSED),
                 "the first text cell IS highlighted");
    }

    #[test]
    fn story_row_selection_is_unaffected_by_the_meta_gutter_fix() {
        // Regression: an ordinary Story/Input row has no gutter (text_origin_col
        // == 0), so threading the per-row origin through must not shift its
        // selection at all.
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.command_bar = true;
        state.push_transcript("hello world");
        state.focus = Focus::Game;
        state.selection = Some(crate::clipboard::Selection {
            anchor: crate::clipboard::Point { row: 0, col: 0 },
            head: crate::clipboard::Point { row: 0, col: 4 },
        });
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);
        let copied = state.selection_text.borrow().clone().expect("copy published");
        assert_eq!(copied, "hello");
    }

    // ── SQ-0330: Glk paragraph layout (indent / para_indent / justification) ──

    #[test]
    fn centered_line_is_padded_to_centre_and_shifts_runs() {
        // "hi" (2 chars) centered in width 10 → (10-2)/2 = 4 leading spaces.
        let lines = vec!["hi".to_string()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let runs = vec![vec![StyleRun { start: 0, end: 2, bits: 0x02, fg: 0, bg: 0, link: 0, glk_style: 0 }]];
        let para = vec![ParaFmt { indent: 0, para_indent: 0, justify: 2, nowrap_from: None }];
        let out = wrap_lines_kinded(&lines, &kinds, &styles, &runs, &para, &[], (1, 1), false, false, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "    hi", "text padded to centre");
        // The bold run must move right by the 4 padding columns so selection/copy
        // stays aligned with the drawn text.
        assert_eq!(out[0].runs, vec![StyleRun { start: 4, end: 6, bits: 0x02, fg: 0, bg: 0, link: 0, glk_style: 0 }]);
    }

    #[test]
    fn right_flush_line_pads_all_slack() {
        let lines = vec!["hi".to_string()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let para = vec![ParaFmt { indent: 0, para_indent: 0, justify: 3, nowrap_from: None }];
        let out = wrap_lines_kinded(&lines, &kinds, &styles, &[], &para, &[], (1, 1), false, false, 10);
        assert_eq!(out[0].text, "        hi", "right-flush pads to the right edge");
    }

    #[test]
    fn indented_paragraph_indents_every_wrapped_row() {
        // indent=2, width 8 → usable 6; "AAAAA BBBBB" wraps to "AAAAA"/"BBBBB",
        // each prefixed with 2 spaces.
        let lines = vec!["AAAAA BBBBB".to_string()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let para = vec![ParaFmt { indent: 2, para_indent: 0, justify: 0, nowrap_from: None }];
        let out = wrap_lines_kinded(&lines, &kinds, &styles, &[], &para, &[], (1, 1), false, false, 8);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text, "  AAAAA");
        assert_eq!(out[1].text, "  BBBBB");
    }

    #[test]
    fn para_indent_adds_to_first_row_only() {
        // indent=1, para_indent=2 → row 0 leads with 3 spaces, continuations with 1.
        let lines = vec!["AAAAA BBBBB".to_string()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let para = vec![ParaFmt { indent: 1, para_indent: 2, justify: 0, nowrap_from: None }];
        let out = wrap_lines_kinded(&lines, &kinds, &styles, &[], &para, &[], (1, 1), false, false, 10);
        assert_eq!(out[0].text, "   AAAAA", "row 0 gets indent+para_indent");
        assert_eq!(out[1].text, " BBBBB", "continuation gets only indent");
    }

    #[test]
    fn default_para_fmt_renders_identically_to_no_layout() {
        // The Z-machine path (default ParaFmt) must wrap byte-identically to the
        // pre-SQ-0330 behaviour: no padding, unshifted runs.
        let lines = vec!["AAAAA BBBBB".to_string()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let runs = vec![vec![StyleRun { start: 6, end: 11, bits: 0x02, fg: 0, bg: 0, link: 0, glk_style: 0 }]];
        let para = vec![ParaFmt::default()];
        let with_para = wrap_lines_kinded(&lines, &kinds, &styles, &runs, &para, &[], (1, 1), false, false, 5);
        let without = wrap_lines_kinded(&lines, &kinds, &styles, &runs, &[], &[], (1, 1), false, false, 5);
        let texts_p: Vec<&str> = with_para.iter().map(|r| r.text.as_str()).collect();
        let texts_n: Vec<&str> = without.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts_p, texts_n);
        assert_eq!(with_para[1].runs, without[1].runs);
    }

    #[test]
    fn selection_over_centered_line_copies_the_right_characters() {
        // Simulate a copy: a selection picks columns [4, 6) of the padded row and
        // must yield "hi" (the visible text), because the padding is real row text
        // and the runs are shifted to match. A naive draw-time offset (not padding
        // the text) would make column 4 land on a space.
        let lines = vec!["hi".to_string()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let para = vec![ParaFmt { indent: 0, para_indent: 0, justify: 2, nowrap_from: None }];
        let out = wrap_lines_kinded(&lines, &kinds, &styles, &[], &para, &[], (1, 1), false, false, 10);
        let row = &out[0].text;
        let sel: String = row.chars().skip(4).take(2).collect();
        assert_eq!(sel, "hi", "columns [4,6) of the padded row are the visible text");
        // Leading padding columns are spaces (selectable but blank).
        assert_eq!(row.chars().take(4).collect::<String>(), "    ");
    }

    #[test]
    fn wrap_line_basic_word_wrap() {
        // "the quick brown fox" at width 9: "the quick" + "brown fox"
        let result = wrap_line("the quick brown fox", 9);
        assert_eq!(result, vec!["the quick", "brown fox"]);
    }

    #[test]
    fn wrap_line_hard_break_long_word() {
        // "abcdefghij" at width 4: "abcd" + "efgh" + "ij"
        let result = wrap_line("abcdefghij", 4);
        assert_eq!(result, vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn wrap_line_fits_in_one_row() {
        let result = wrap_line("hello", 10);
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn wrap_line_empty_string() {
        let result = wrap_line("", 10);
        assert_eq!(result, vec![""]);
    }

    #[test]
    fn wrap_line_exact_width() {
        // "abc" at width 3: exactly fits
        let result = wrap_line("abc", 3);
        assert_eq!(result, vec!["abc"]);
    }

    #[test]
    fn wrap_lines_kinded_expands_multiple_logical_lines() {
        let lines = vec![
            "hello world test".to_string(),
            "short".to_string(),
        ];
        let kinds = vec![TranscriptKind::Story; 2];
        let styles = vec![Style::default(); 2];
        // width 5: "hello" + "world" + "test" + "short"
        let result = wrap_lines_kinded(&lines, &kinds, &styles, &[], &[], &[], (1, 1), false, false, 5);
        let rows: Vec<&str> = result.iter().map(|wr| wr.text.as_str()).collect();
        assert_eq!(rows, vec!["hello", "world", "test", "short"]);
    }

    #[test]
    fn meta_lines_wrap_narrower_and_carry_kind() {
        // An 8-char wordless line: STORY fits in width 8; META wraps to width-2 = 6.
        // Continuation gets a 2-space hanging indent (leading_spaces("abcdefgh")=0,
        // .max(2)=2).
        let transcript = vec!["abcdefgh".to_string()];
        let st = [Style::default()];
        let m = wrap_lines_kinded(&transcript, &[TranscriptKind::Meta], &st, &[], &[], &[], (1, 1), false, false, 8);
        assert_eq!(m.iter().map(|wr| wr.text.as_str()).collect::<Vec<_>>(), vec!["abcdef", "  gh"]);
        assert!(m.iter().all(|wr| matches!(wr.kind, TranscriptKind::Meta)));
        let s = wrap_lines_kinded(&transcript, &[TranscriptKind::Story], &st, &[], &[], &[], (1, 1), false, false, 8);
        assert_eq!(s.iter().map(|wr| wr.text.as_str()).collect::<Vec<_>>(), vec!["abcdefgh"]);
    }

    #[test]
    fn wrap_carries_logical_line_style_onto_every_row() {
        use ratatui::style::Color;
        // One logical line that wraps to 3 rows; its style must appear on all rows.
        let transcript = vec!["alpha beta gamma".to_string()];
        let styles = [Style::new().fg(Color::Magenta)];
        let rows = wrap_lines_kinded(&transcript, &[TranscriptKind::Story], &styles, &[], &[], &[], (1, 1), false, false, 5);
        assert!(rows.len() >= 3, "expected wrap to >= 3 rows, got {}", rows.len());
        assert!(rows.iter().all(|wr| wr.style.fg == Some(Color::Magenta)),
            "every wrapped row must carry the logical line's style");
    }

    #[test]
    fn visible_wrapped_lines_kinded_newest_at_bottom() {
        // 3 logical lines at width 10 = 3 display rows; scroll=0, rows=3
        let transcript = vec!["abc".to_string(), "def".to_string(), "ghi".to_string()];
        let kinds = vec![TranscriptKind::Story; 3];
        let styles = vec![Style::default(); 3];
        let (vis, total, first) = visible_wrapped_lines_kinded(&transcript, &kinds, &styles, &[], &[], &[], (1, 1), false, 3, 0, 10, None);
        assert_eq!(vis.len(), 3);
        assert_eq!(total, 3, "total wrapped rows reported");
        assert_eq!(first, 0, "top visible row is absolute row 0");
        assert_eq!(vis[2].text, "ghi");
    }

    #[test]
    fn visible_wrapped_lines_kinded_scroll_offset() {
        // "hello world" wraps to ["hello", "world"] at width 5
        let transcript = vec!["hello world".to_string()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        // scroll=1: end = 2-1=1, start = 1-1=0 → ["hello"]
        let (vis, total, first) = visible_wrapped_lines_kinded(&transcript, &kinds, &styles, &[], &[], &[], (1, 1), false, 1, 1, 5, None);
        assert_eq!(vis[0].text, "hello");
        assert_eq!(total, 2, "both wrapped rows counted");
        assert_eq!(first, 0, "scroll=1 window starts at row 0");
        // scroll=0: end=2, start=1 → ["world"]
        let (vis2, _, first2) = visible_wrapped_lines_kinded(&transcript, &kinds, &styles, &[], &[], &[], (1, 1), false, 1, 0, 5, None);
        assert_eq!(vis2[0].text, "world");
        assert_eq!(first2, 1, "scroll=0 window starts at row 1");
    }

    #[test]
    fn visible_wrapped_lines_kinded_over_scroll_clamps_to_top() {
        // 5 logical lines, viewport 3 rows. Over-scrolling past the top must
        // keep showing the TOP 3 lines, not shrink the window from the bottom.
        let transcript: Vec<String> = (0..5).map(|i| format!("L{}", i)).collect();
        let kinds = vec![TranscriptKind::Story; 5];
        let styles = vec![Style::default(); 5];
        let (vis, total, _first) = visible_wrapped_lines_kinded(&transcript, &kinds, &styles, &[], &[], &[], (1, 1), false, 3, 999, 10, None);
        assert_eq!(total, 5);
        assert_eq!(vis.len(), 3, "over-scroll still fills the viewport");
        assert_eq!(vis[0].text, "L0", "top line stays at the top");
        assert_eq!(vis[2].text, "L2");
    }

    #[test]
    fn format_input_line_prefix() {
        assert_eq!(format_input_line("open mailbox"), "> open mailbox");
        assert_eq!(format_input_line(""), "> ");
    }

    #[test]
    fn visible_wrapped_lines_kinded_top_anchors_after_clear() {
        // 5 lines, viewport 3. A clear boundary at index 3 → post-clear content
        // is lines 3,4 (2 lines); they pin to the TOP (2 rows returned, caller
        // leaves the rest blank) rather than bottom-sticking the full viewport.
        let transcript: Vec<String> = (0..5).map(|i| format!("L{}", i)).collect();
        let kinds = vec![TranscriptKind::Story; 5];
        let styles = vec![Style::default(); 5];
        let (vis, total, _first) =
            visible_wrapped_lines_kinded(&transcript, &kinds, &styles, &[], &[], &[], (1, 1), false, 3, 0, 10, Some(3));
        assert_eq!(total, 5);
        assert_eq!(vis.len(), 2, "only post-clear lines returned (top-anchored)");
        assert_eq!(vis[0].text, "L3");
        assert_eq!(vis[1].text, "L4");
        // No anchor → full viewport, bottom-stick (history stays in view).
        let (vis2, _, _) =
            visible_wrapped_lines_kinded(&transcript, &kinds, &styles, &[], &[], &[], (1, 1), false, 3, 0, 10, None);
        assert_eq!(vis2.len(), 3);
        assert_eq!(vis2[0].text, "L2", "bottom-stick pulls in the pre-clear line");
        // Scrolled up (scroll>0): anchor ignored so history is reachable.
        let (vis3, _, _) =
            visible_wrapped_lines_kinded(&transcript, &kinds, &styles, &[], &[], &[], (1, 1), false, 3, 1, 10, Some(3));
        assert_eq!(vis3.len(), 3, "scrolled up ignores the clear anchor");
        assert_eq!(vis3[2].text, "L3");
        // Once post-clear content overflows the viewport, top-anchor stops
        // triggering and normal bottom-stick resumes (anchor at 0, 5 lines > 3).
        let (vis4, _, _) =
            visible_wrapped_lines_kinded(&transcript, &kinds, &styles, &[], &[], &[], (1, 1), false, 3, 0, 10, Some(0));
        assert_eq!(vis4.len(), 3);
        assert_eq!(vis4[2].text, "L4", "overflow → bottom-stick");
    }

    // ── Render tests: transcript + input rows (no Machine) ───────────────────
    //
    // We still need a Machine for render_transcript. We build a minimal one from
    // zvm's sample_story (v3) to avoid needing a real fixture file.

    fn minimal_machine() -> Machine {
        use zvm::memory::Memory;
        // Use the same sample_story helper that zvm's own tests use.
        // It's in zvm::header::tests_support but that's cfg(test)-only.
        // Instead we build a minimal valid v3 story buffer ourselves.
        //
        // Minimum valid v3 story file:
        //   byte 0x00 = version (3)
        //   bytes 0x04-0x05 = high memory base (e.g. 0x0040)
        //   bytes 0x06-0x07 = initial PC (e.g. 0x0040)
        //   bytes 0x0A-0x0B = dictionary base (0x0080)
        //   bytes 0x0C-0x0D = object table base (0x0100)
        //   bytes 0x0E-0x0F = global var table base (0x0300)
        //   bytes 0x08-0x09 = static mem base (0x0400)
        //   bytes 0x02-0x03 = (release number, ignored)
        //   Total: 0x500 bytes should be enough.
        //
        // We use the same layout as zvm/src/header.rs tests_support::sample_story(3).
        let mut buf = vec![0u8; 0x0800];
        buf[0x00] = 3;                       // version = 3
        // high_mem_base = 0x0040
        buf[0x04] = 0x00; buf[0x05] = 0x40;
        // initial_pc = 0x0040 (will contain a QUIT/quit opcode: 0x00 = rtrue? use 0xba = quit)
        buf[0x06] = 0x00; buf[0x07] = 0x40;
        // dict base = 0x0080
        buf[0x0A] = 0x00; buf[0x0B] = 0x80;
        // object table = 0x0100
        buf[0x0C] = 0x01; buf[0x0D] = 0x00;
        // global var table = 0x0300
        buf[0x0E] = 0x03; buf[0x0F] = 0x00;
        // static mem base = 0x0400 (dynamic = 0x0000..0x03FF)
        buf[0x08] = 0x04; buf[0x09] = 0x00;
        // abbreviation table = 0x0060 (word at 0x18-0x19)
        buf[0x18] = 0x00; buf[0x19] = 0x60;

        // Place a valid dictionary at 0x0080: word-separators count=0, entry_size=4, entry_count=0.
        buf[0x0080] = 0; // 0 word-separators
        buf[0x0081] = 4; // entry size = 4 bytes
        buf[0x0082] = 0; buf[0x0083] = 0; // entry count = 0

        // Object table at 0x0100: 31 prop-default words (62 bytes), then no objects.
        // Property defaults: all zero (62 bytes, already 0).

        // Put a QUIT opcode at 0x0040 so stepping won't panic.
        buf[0x0040] = 0xba; // opcode for 'quit' in v3 (0OP:0x0a → encoded as 0xba).

        let mem = Memory::new(buf).expect("minimal v3 story should be valid");
        Machine::new(mem)
    }

    /// Minimal v4 story (same minimal header as minimal_machine, version byte 4)
    /// so version() >= 4 — used to exercise the v4+ status-bar path.
    fn minimal_machine_v4() -> Machine {
        use zvm::memory::Memory;
        let mut buf = vec![0u8; 0x0800];
        buf[0x00] = 4; // version = 4
        buf[0x04] = 0x00; buf[0x05] = 0x40; // high mem
        buf[0x06] = 0x00; buf[0x07] = 0x40; // initial pc
        buf[0x0A] = 0x00; buf[0x0B] = 0x80; // dict
        buf[0x0C] = 0x01; buf[0x0D] = 0x00; // object table
        buf[0x0E] = 0x03; buf[0x0F] = 0x00; // globals
        buf[0x08] = 0x04; buf[0x09] = 0x00; // static base
        buf[0x18] = 0x00; buf[0x19] = 0x60; // abbrev table
        buf[0x0080] = 0; buf[0x0081] = 4; buf[0x0082] = 0; buf[0x0083] = 0; // dict
        let mem = Memory::new(buf).expect("minimal v4 story should be valid");
        Machine::new(mem)
    }

    #[test]
    fn v4_status_bar_does_not_render_synthesized_content() {
        // The synthesized lanthorn status bar (room + turn counter) is removed for
        // v4+/HostManaged during normal play. The bar must be fully hidden when
        // there is no transient notification message.
        let machine = minimal_machine_v4();
        let mut state = AppState::default();
        state.current_room_name = Some("Outside".to_string());
        state.turns = 7;
        state.transcript = vec!["FIRSTLINE".to_string()];

        let area = Rect::new(0, 0, 60, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // Room name and turn counter must NOT appear anywhere in the buffer.
        let all_text: String = {
            let mut s = String::new();
            for y in 0..area.height {
                for x in 0..area.width {
                    s.push(buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '));
                }
            }
            s
        };
        assert!(!all_text.contains("Outside"),
            "v4 must not render synthesized room name: {:?}", all_text);
        assert!(!all_text.contains("turn 7"),
            "v4 must not render synthesized turn counter: {:?}", all_text);
        // Transcript starts at y=0 (status bar occupies 0 rows).
        let top: String = (0..area.width)
            .map(|x| buf.cell((x, 0)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(top.contains("FIRSTLINE"),
            "transcript must begin at y=0 when HostManaged bar is hidden: {:?}", top);
    }

    #[test]
    fn v4_status_bar_collapsed_always_v3_always_shows() {
        let area = Rect::new(0, 0, 60, 5);

        // v4 + no message → top row is the transcript regardless of show_status_bar.
        // The synthesized bar is now unconditionally removed for HostManaged.
        let mv4 = minimal_machine_v4();
        let mut s = AppState::default();
        // show_status_bar=true is the default; the bar must still be hidden.
        s.current_room_name = Some("Outside".to_string());
        s.transcript = vec!["FIRSTLINE".to_string()];
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&mv4), None, &s, area, &mut buf, None);
        let top: String = (0..area.width)
            .map(|x| buf.cell((x, 0)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(!top.contains("Outside"), "v4 synthesized bar must never render: {:?}", top);

        // v3 always shows its Classic status line (unaffected by this change).
        let mv3 = minimal_machine();
        let mut s3 = AppState::default();
        s3.show_status_bar = false;
        let mut buf3 = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&mv3), None, &s3, area, &mut buf3, None);
        let top3_reversed = buf3.cell((0, 0)).map(|c| c.modifier.contains(Modifier::REVERSED)).unwrap_or(false);
        assert!(top3_reversed, "v3 Classic status bar always renders regardless of show_status_bar");
    }

    // ── New: status-bar removal contract for HostManaged (v4+/Glulx) ──────────

    /// (a) HostManaged → bar hidden, no synthesized content (SQ-0176: transient
    /// feedback never reuses the score bar).
    #[test]
    fn host_managed_no_msg_status_row_hidden() {
        let mut state = AppState::default();
        state.current_room_name = Some("West of House".to_string());
        state.turns = 42;
        state.transcript = vec!["You are standing in front of a house.".to_string()];

        let area = Rect::new(0, 0, 60, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&StatusModel::HostManaged, None, &state, area, &mut buf, None);

        // Synthesized content must not appear anywhere.
        let all_text: String = {
            let mut s = String::new();
            for y in 0..area.height {
                for x in 0..area.width {
                    s.push(buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '));
                }
            }
            s
        };
        assert!(!all_text.contains("West of House"),
            "HostManaged bar must not render room name when no status_msg: {:?}", all_text);
        assert!(!all_text.contains("turn 42"),
            "HostManaged bar must not render turn counter when no status_msg: {:?}", all_text);
        // Transcript flows from y=0 (status bar takes no rows).
        let top: String = (0..area.width)
            .map(|x| buf.cell((x, 0)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(top.contains("standing"),
            "transcript must start at y=0 (no status bar row): {:?}", top);
    }

    /// (c) Classic status line is unaffected — still always renders.
    #[test]
    fn classic_status_always_renders_unchanged() {
        // The Classic (v3) status line must be visible with its reversed background
        // and its location/score content regardless of show_status_bar.
        let machine = minimal_machine(); // v3 → Classic
        let mut state = AppState::default();
        state.show_status_bar = false; // toggling this must not hide the Classic bar
        state.transcript = vec!["some story text".to_string()];

        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // Top row (y=0) must have the reversed status bar background.
        assert!(
            buf.cell((0, 0)).unwrap().modifier.contains(Modifier::REVERSED),
            "Classic status bar must always render (reversed modifier) regardless of toggle"
        );
        // Transcript is below y=0, not at y=0.
        let top: String = (0..area.width)
            .map(|x| buf.cell((x, 0)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(!top.contains("some story text"),
            "transcript must not occupy y=0 when Classic bar is visible: {:?}", top);
    }

    #[test]
    fn render_kinds_draw_their_own_styles_and_gutters() {
        use ratatui::style::Color;
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.push_transcript_kind("> go north", TranscriptKind::Input);
        state.push_transcript_kind("app message", TranscriptKind::Meta);
        state.push_transcript_kind("VAR 0x15 unimplemented", TranscriptKind::Warning);
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // Find the row index (1..8) for each tagged line by its first glyph / content.
        let row_text = |y: u16| -> String {
            (0..40u16).map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' ')).collect()
        };
        // Locate the warning row by its gutter glyph '!' in column 0.
        let warn_y = (1u16..9).find(|&y| buf.cell((0, y)).map(|c| c.symbol()) == Some("!"))
            .expect("warning gutter '!' must appear in column 0");
        // Warning gutter cell uses warning_marker (Yellow).
        assert_eq!(buf.cell((0, warn_y)).unwrap().style().fg, Some(Color::Yellow));
        // Warning text is indented past the 2-col gutter and uses transcript_warning (Yellow).
        assert_eq!(buf.cell((2, warn_y)).unwrap().style().fg, Some(Color::Yellow));

        // Meta row: gutter glyph '▏' in column 0.
        let meta_y = (1u16..9).find(|&y| buf.cell((0, y)).map(|c| c.symbol()) == Some("▏"))
            .expect("meta gutter '▏' must appear in column 0");
        assert_eq!(buf.cell((2, meta_y)).unwrap().style().fg, Some(Color::DarkGray)); // transcript_meta

        // Input row: no gutter (text at column 0), cyan fg.
        let input_y = (1u16..9).find(|&y| row_text(y).starts_with("> go north"))
            .expect("input line must render at column 0");
        assert_eq!(buf.cell((0, input_y)).unwrap().style().fg, Some(Color::Cyan)); // transcript_input
    }

    /// SQ-1045: on screen an assist is identified by its MARK and by nothing
    /// else — the text carries no `Lanthorn: ` any more — so the mark had better
    /// be drawn, in the row's own style, with the words starting past it.
    ///
    /// Falsify by dropping the `Assist` arm from either `text_origin_col` or the
    /// gutter match: the glyph vanishes (or the text slides under it) and the one
    /// thing that says whose the line is goes with it.
    #[test]
    fn an_assist_row_is_identified_by_its_mark_in_the_gutter() {
        use ratatui::style::Color;
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.assist_preamble_shown = true; // the introduction has its own case
        state.push_assist(&crate::assist::Assist::caution("that cannot be undone."));
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        let mark = state.symbols.assist_gutter.to_string();
        let y = (1u16..9)
            .find(|&y| buf.cell((0, y)).map(|c| c.symbol()) == Some(mark.as_str()))
            .expect("the assist mark must appear in column 0");
        // Drawn in the LINE's style, not a separate marker selector: the caution
        // tone's mark is as loud as its text.
        assert_eq!(buf.cell((0, y)).unwrap().style().fg, Some(Color::Yellow));
        assert!(buf.cell((0, y)).unwrap().modifier.contains(Modifier::BOLD), "the caution mark is the loud one");
        // The words start past the two-column gutter, and are the caller's alone.
        let row: String = (0..40u16)
            .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(row.starts_with(&format!("{mark} that cannot")), "mark, gutter, then the words: {row:?}");
        assert!(!row.contains("Lanthorn:"), "the marker belongs to the exported file, not the screen: {row:?}");
    }

    #[test]
    fn crash_lines_render_with_crash_style() {
        use ratatui::style::Color;
        let machine = minimal_machine();
        let mut state = AppState::default();
        let crash_style = state.colors.theme.get("transcript_crash").style;
        state.push_transcript_styled("*** VM FAULT ***", TranscriptKind::Warning, crash_style);
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // Single Warning-kind line → its gutter '!' marks the crash row.
        let y = (1u16..9)
            .find(|&y| buf.cell((0, y)).map(|c| c.symbol()) == Some("!"))
            .expect("crash line gutter '!' must appear in column 0");
        // The crash TEXT (past the 2-col gutter) must carry the crash style's fg
        // AND bold — proving the explicit style override applied. SQ-0309:
        // `transcript_crash` derives from the `alert` role same as the default
        // Warning yellow now (docs/design/2026-07-14-styling-role-redesign.md
        // §2), so BOLD is what distinguishes a crash line, not colour.
        assert_eq!(buf.cell((2, y)).unwrap().style().fg, crash_style.fg);
        assert_eq!(crash_style.fg, Some(Color::Yellow));
        assert!(crash_style.add_modifier.contains(ratatui::style::Modifier::BOLD));
        assert!(buf.cell((2, y)).unwrap().style().add_modifier.contains(ratatui::style::Modifier::BOLD));
    }

    #[test]
    fn wrapped_system_line_keeps_style_on_all_rows() {
        // Regression: a bracketed system line long enough to wrap must keep its
        // transcript:system style on EVERY wrapped row. The style is resolved on
        // the whole logical line, not the wrapped fragments (neither of which is
        // itself fully bracketed).
        use ratatui::style::Color;
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.push_transcript("[Your score just went up by ten points.]"); // Story
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 20, 12); // narrow → the 40-char line wraps
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        let mut checked = 0;
        for y in 1u16..10 {
            let c0 = buf.cell((0, y)).expect("cell");
            if c0.symbol().trim().is_empty() {
                continue; // blank row (not part of the wrapped line)
            }
            assert_eq!(
                c0.style().fg,
                Some(Color::DarkGray),
                "row {} must carry transcript:system (DarkGray) on a wrapped system line",
                y
            );
            checked += 1;
        }
        assert!(checked >= 2, "system line should wrap to >= 2 rows; checked {}", checked);
    }

    #[test]
    fn render_story_location_line_is_accent_coloured() {
        // SQ-0309: `transcript_location` derives from the `accent` role, not
        // bold-only-white (docs/design/2026-07-14-styling-role-redesign.md §2).
        use ratatui::style::Color;
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.current_room_name = Some("West of House".to_string());
        state.push_transcript("West of House"); // Story
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        let y = (1u16..9).find(|&y| {
            let row: String = (0..40u16).map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' ')).collect();
            row.starts_with("West of House")
        }).expect("location line must render");
        assert_eq!(
            buf.cell((0, y)).unwrap().style().fg,
            Some(Color::Cyan),
            "location header must carry the accent colour"
        );
    }

    #[test]
    fn render_transcript_input_and_transcript_lines() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.command_bar = true; // this test exercises the dedicated bottom bar
        state.transcript = vec![
            "You are in a hall.".to_string(),
            "It is dark.".to_string(),
        ];
        state.input.set("open mailbox", true);
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // Bottom row (y=9) should contain "> open mailbox".
        let bottom_row: String = (0..40u16)
            .map(|x| buf.cell((x, 9)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(
            bottom_row.contains("> open mailbox"),
            "bottom row should contain '> open mailbox'; got: {:?}",
            bottom_row
        );

        // A middle row should contain one of the transcript lines.
        let found_transcript = (1u16..9u16).any(|y| {
            let row: String = (0..40u16)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect();
            row.contains("You are in a hall.") || row.contains("It is dark.")
        });
        assert!(found_transcript, "a middle row should contain a transcript line");
    }

    #[test]
    fn input_line_uses_game_colour() {
        use ratatui::style::Color;
        let mut state = AppState::default();
        state.config.honor_game_colours = true;
        state.input.set("x", true);
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        let game = Some(ratatui::style::Style::new().fg(Color::Cyan));
        render_input_content(&state, &mut buf, area, ratatui::style::Style::new(), game);
        // The "> " prompt occupies cols 0-1; the typed 'x' is at col 2.
        assert_eq!(buf.cell((2, 0)).unwrap().style().fg, Some(Color::Cyan),
            "typed input uses the game colour");
    }

    #[test]
    fn input_line_places_wide_glyphs_and_the_caret_by_cell() {
        // SQ-0655: a typed/pasted double-width glyph owns TWO cells. Drawing one
        // char per cell put every later glyph — and the caret — one column left per
        // wide glyph before it, and disagreed with `input_click_index`, which reads
        // the same line back in cells.
        let mut state = AppState::default();
        state.input.set("日本x", true); // 3 chars, 5 cells
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        render_input_content(&state, &mut buf, area, Style::new(), None);

        // "> " prompt at cols 0-1, then 日 at 2 (3 blanked), 本 at 4 (5 blanked), x at 6.
        assert_eq!(buf.cell((2, 0)).unwrap().symbol(), "日");
        assert_eq!(buf.cell((3, 0)).unwrap().symbol(), " ", "the wide glyph's second cell is blanked");
        assert_eq!(buf.cell((4, 0)).unwrap().symbol(), "本");
        assert_eq!(buf.cell((6, 0)).unwrap().symbol(), "x");
        // The caret sits after the text: 2 + 5 cells = col 7.
        assert_eq!(state.input.cursor, 3);
        assert!(buf.cell((7, 0)).unwrap().modifier.contains(Modifier::REVERSED),
            "caret at the end of a wide-glyph line sits past its last CELL");
        // …and a click on that same cell maps back to the caret index it was drawn from.
        assert_eq!(state.input_click_index(7, 0), Some(3));
        assert_eq!(state.input_click_index(4, 0), Some(1), "clicking 本 selects 本");

        // Mid-line caret: before 本, i.e. two cells in from the text origin.
        let mut mid = AppState::default();
        mid.input.set("日本x", true);
        mid.input.cursor = 1;
        let mut buf2 = Buffer::empty(area);
        render_input_content(&mid, &mut buf2, area, Style::new(), None);
        assert!(buf2.cell((4, 0)).unwrap().modifier.contains(Modifier::REVERSED),
            "mid-line caret lands on the glyph it precedes, in cells");
    }

    #[test]
    fn cursor_style_reverses_game_ink_but_falls_back_to_theme() {
        use ratatui::style::Color;
        // No game colour → the structural theme cursor (bare REVERSED), unchanged.
        assert_eq!(cursor_style(Style::new().fg(Color::Green), None), CURSOR_STYLE);
        // Game page set → reverse-video of the resolved input text so the caret is
        // visible on the recoloured page (SQ-0268). It carries the input fg/bg and
        // adds REVERSED (the terminal performs the swap).
        let text = Style::new().fg(Color::Rgb(0, 0, 0)).bg(Color::Rgb(255, 255, 255));
        let game = Some(Style::new().bg(Color::Rgb(255, 255, 255)));
        let cs = cursor_style(text, game);
        assert!(cs.add_modifier.contains(Modifier::REVERSED), "reversed block");
        assert_eq!(cs.fg, Some(Color::Rgb(0, 0, 0)), "keeps game fg (swapped by terminal)");
        assert_eq!(cs.bg, Some(Color::Rgb(255, 255, 255)), "keeps game bg (swapped by terminal)");
    }

    /// The first row of `buf` whose text contains `needle`, or `None`.
    fn find_row(buf: &Buffer, area: Rect, needle: &str) -> Option<u16> {
        (area.y..area.bottom()).find(|&y| {
            (area.x..area.right())
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default())
                .collect::<String>()
                .contains(needle)
        })
    }

    #[test]
    fn render_transcript_cursor_shown_when_focused() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.command_bar = true; // this test exercises the dedicated bottom bar
        state.input.set("hi", true);
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // Cursor at position 4 ("> hi" = 4 chars → cursor at x=4).
        let cursor_cell = buf.cell((4, 4)).expect("cursor cell should exist");
        assert_eq!(cursor_cell.symbol(), " ", "block cursor is a reverse-video space at end of input");
        assert!(
            cursor_cell.modifier.contains(Modifier::REVERSED),
            "cursor cell should have REVERSED modifier"
        );
    }

    /// SQ-0873. Not one of the five machines measured draws the reverse-video
    /// block a terminal front-end gives by default: the Commodores put a single
    /// scanline on the cell's bottom row, the Macintosh a one-pixel caret in the
    /// gap after the last glyph. The cell-grid analogue is the glyph occupying
    /// the same eighth of the cell.
    #[test]
    fn the_caret_takes_its_machines_shape_under_a_period_look() {
        let machine = minimal_machine();
        for (number, glyph) in [
            (zvm::interpreter::COMMODORE_64_INTERPRETER_NUMBER, "▁"),
            (zvm::interpreter::MACINTOSH_INTERPRETER_NUMBER, "▏"),
            (zvm::interpreter::AMIGA_INTERPRETER_NUMBER, " "),
        ] {
            let look = zvm::interpreter::period_look_for(number, None).unwrap();
            let mut state = AppState::default();
            state.config.command_bar = true;
            state.input.set("hi", true);
            state.focus = Focus::Game;
            state.period_look = Some(look);

            let area = Rect::new(0, 0, 40, 5);
            let mut buf = Buffer::empty(area);
            render_transcript(
                &crate::session::status_model_from_machine(&machine),
                None,
                &state,
                area,
                &mut buf,
                None,
            );

            let cell = buf.cell((4, 4)).expect("cursor cell should exist");
            assert_eq!(cell.symbol(), glyph, "interpreter {number} draws its own caret");
            assert!(
                !cell.modifier.contains(Modifier::REVERSED),
                "interpreter {number}: the shape is stated outright, not by reversing the theme"
            );
        }
    }

    /// SQ-0947, the reported symptom: an Amiga **v6** launch drew the `#FF7E1C`
    /// orange block its own *v3* interpreter uses, and a DOS v6 one drew its v3
    /// underscore. Both are stored measurements applied a version too far.
    ///
    /// The machine table answers `ReverseSpace` for these two at v6 — the caret is
    /// the pair on screen, reversed, which is what `amiga-zorkzero.png`,
    /// `amiga-shogun.png` and `dos-arthur.png` show — and this pins the consequence
    /// here: the look states no caret, so the structural one draws, and the
    /// structural one IS that reversal.
    ///
    /// Driven through `crate::period::resolve` rather than off a table row, because
    /// the version is the whole variable and a row cannot carry it.
    ///
    /// Both `honor_game_colours` modes, per CLAUDE.md — and the `game_input`
    /// argument with them, since that is the value the caret actually reverses.
    #[test]
    fn the_version_six_caret_reverses_the_live_pair_instead_of_the_v3_measurement() {
        let machine = minimal_machine();
        let orange = ratatui::style::Color::Rgb(0xFF, 0x7E, 0x1C);
        for profile in [crate::interpreter::InterpreterProfile::Amiga, crate::interpreter::InterpreterProfile::IbmPc] {
            let look = crate::period::resolve(profile, true, true, true, Some(6))
                .expect("a measured machine at a version it shipped for");
            assert_eq!(
                look.cursor_shape,
                zvm::interpreter::CursorShape::ReverseSpace,
                "{profile:?} at v6",
            );
            for honor in [true, false] {
                for game_input in [None, Some(Style::new().fg(Color::Black).bg(Color::Gray))] {
                    let mut state = AppState::default();
                    state.config.command_bar = true;
                    state.config.honor_game_colours = honor;
                    state.input.set("hi", true);
                    state.focus = Focus::Game;
                    state.period_look = Some(look);

                    let area = Rect::new(0, 0, 40, 5);
                    let mut buf = Buffer::empty(area);
                    render_transcript(
                        &crate::session::status_model_from_machine(&machine),
                        None,
                        &state,
                        area,
                        &mut buf,
                        game_input,
                    );

                    let cell = buf.cell((4, 4)).expect("cursor cell should exist");
                    let where_ = format!("{profile:?}, honor={honor}, game_input={}", game_input.is_some());
                    assert_eq!(cell.symbol(), " ", "{where_}: a reversed SPACE, not a shape glyph");
                    assert!(
                        cell.modifier.contains(Modifier::REVERSED),
                        "{where_}: reversed, so the caret follows whatever pair is on screen",
                    );
                    assert_ne!(cell.fg, orange, "{where_}: the v3 orange is not a v6 caret");
                    assert_ne!(cell.bg, orange, "{where_}: the v3 orange is not a v6 caret");
                }
            }
        }
    }

    /// …and with no period look in force the caret is exactly what it always was.
    /// The shape is the machine's; a story with no machine keeps the theme's.
    #[test]
    fn without_a_period_look_the_caret_is_the_structural_block() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.command_bar = true;
        state.input.set("hi", true);
        state.focus = Focus::Game;
        assert!(state.period_look.is_none());

        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);
        let cell = buf.cell((4, 4)).expect("cursor cell should exist");
        assert_eq!(cell.symbol(), " ");
        assert!(cell.modifier.contains(Modifier::REVERSED));
    }

    /// The point of the whole feature: the prose sits on the machine's page in
    /// the machine's ink, and the line being typed sits on the same page rather
    /// than punching the theme's through it.
    #[test]
    fn the_body_and_the_input_line_stand_on_the_machines_page() {
        let machine = minimal_machine();
        let look =
            zvm::interpreter::period_look_for(zvm::interpreter::AMIGA_INTERPRETER_NUMBER, None)
                .unwrap();
        let mut state = AppState::default();
        state.period_look = Some(look);
        crate::period::apply_to_theme(&mut state.colors.theme, &look, Some(3));
        state.transcript = vec!["West of House".to_string()];
        state.transcript_kinds = vec![crate::state::TranscriptKind::Story];
        state.input.set("open mailbox", true);
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 60, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(
            &crate::session::status_model_from_machine(&machine),
            None,
            &state,
            area,
            &mut buf,
            None,
        );

        let page = Color::Rgb(look.page.0, look.page.1, look.page.2);
        let ink = Color::Rgb(look.ink.0, look.ink.1, look.ink.2);
        let prose = find_row(&buf, area, "West of House").expect("the prose is on screen");
        assert_eq!(buf.cell((0, prose)).unwrap().bg, page, "the prose stands on the page");
        assert_eq!(buf.cell((0, prose)).unwrap().fg, ink, "in the machine's ink");
        let typed = find_row(&buf, area, "open mailbox").expect("the typed line is on screen");
        assert_eq!(buf.cell((0, typed)).unwrap().bg, page, "and so does what you are typing");
    }

    /// SQ-0873. The Amiga's status line is a full-width reverse of its body pair.
    ///
    /// Its CAPTURE reverses per run, with 376 px of page showing between "Council
    /// Chamber" and "Score: 0/0" — and we draw the band whole, on the user's
    /// ruling that a band in pieces reads as damage in a terminal. The
    /// measurement lives in `StatusBand::PerRun`'s doc; nothing renders it.
    #[test]
    fn the_amiga_status_line_is_a_full_width_reverse_of_its_body_pair() {
        let machine = minimal_machine();
        let look =
            zvm::interpreter::period_look_for(zvm::interpreter::AMIGA_INTERPRETER_NUMBER, None)
                .unwrap();
        let mut state = AppState::default();
        state.period_look = Some(look);
        crate::period::apply_to_theme(&mut state.colors.theme, &look, Some(3));

        let area = Rect::new(0, 0, 60, 8);
        let mut buf = Buffer::empty(area);
        let status = crate::session::status_model_from_machine(&machine);
        render_transcript(&status, None, &state, area, &mut buf, None);

        let page = Color::Rgb(look.page.0, look.page.1, look.page.2);
        let ink = Color::Rgb(look.ink.0, look.ink.1, look.ink.2);
        // The DRAWN ground, not the stored one. Since SQ-0935 the band is the
        // machine's pair patched under the row's own REVERSED modifier rather than
        // a pre-swapped pair stated absolutely — reverse is just reverse — so the
        // cell holds `bg = page` and the terminal swaps it. Asserting the stored
        // value would be asserting which of two identical renderings we chose.
        let drawn_bg = |x: u16| {
            let c = buf.cell((x, 0)).expect("status row");
            if c.modifier.contains(Modifier::REVERSED) { c.fg } else { c.bg }
        };
        let row: Vec<Color> = (area.x..area.right()).map(drawn_bg).collect();
        assert!(row.iter().all(|&c| c == ink), "the whole band is the reversed ground: {row:?}");
        assert!(!row.contains(&page), "no page shows through it");
    }

    #[test]
    fn render_transcript_keeps_the_cursor_when_the_map_has_focus() {
        // The caret used to be hidden whenever focus left the story pane. It shows
        // what you typed and where, which stays true while the keyboard is on the
        // map — and hiding it meant a half-typed command vanished with no sign it
        // was still buffered, so Enter ran something invisible. A real MODAL still
        // suppresses it (see `render_transcript_no_cursor_when_overlay_open`), and
        // the raster/v6 path behaves the same way.
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.command_bar = true; // this test exercises the dedicated bottom bar
        state.input.set("hi", true);
        state.focus = Focus::Map;

        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        let cell = buf.cell((4, 4)).expect("cell should exist");
        assert!(
            cell.modifier.contains(Modifier::REVERSED),
            "the caret must still be drawn with the map focused"
        );
    }

    #[test]
    fn the_room_dock_does_not_suppress_the_story_caret() {
        // The regression the user hit playing advent.blb: `any_overlay_open()`
        // counted the room panel, so opening Room Info or the inspector blanked the
        // live input line AND its caret. SQ-0692 settled it at the root — the dock
        // that replaced both is not an overlay at all — but the SYMPTOM is what this
        // test guards, so it keeps checking the prompt itself, in both dock views.
        use crate::state::RoomDockView;
        let machine = minimal_machine();
        for mode in [RoomDockView::Info, RoomDockView::Diagnostics] {
            let mut state = AppState::default();
            state.config.command_bar = true;
            state.input.set("hi", true);
            state.room_dock.toggle_to(true, true);
            state.room_dock_view = mode;
            assert!(!state.any_overlay_open(), "the room panel is not an overlay at all…");
            assert!(!state.any_modal_overlay_open(), "…and certainly not a MODAL one");

            let area = Rect::new(0, 0, 40, 5);
            let mut buf = Buffer::empty(area);
            render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

            let row: String = (0..40u16)
                .map(|x| buf.cell((x, 4)).unwrap().symbol().chars().next().unwrap_or(' '))
                .collect();
            assert!(row.contains("hi"), "the typed text must be visible with the {mode:?} dock open: {row:?}");
            assert!(
                buf.cell((4, 4)).unwrap().modifier.contains(Modifier::REVERSED),
                "and so must the caret, with the {mode:?} dock open"
            );
        }
    }

    #[test]
    fn render_transcript_no_cursor_when_overlay_open() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.command_bar = true; // this test exercises the dedicated bottom bar
        state.input.set("hi", true);
        state.focus = Focus::Game; // focused on game, but overlay is open

        // Open the hotkey dialog — the simplest boolean overlay.
        state.overlays.hotkey_dialog = true;

        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // Position x=4 (after "> hi") should NOT have '_' because an overlay is open.
        let cell = buf.cell((4, 4)).expect("cell should exist");
        assert_ne!(
            cell.symbol(),
            "_",
            "cursor must be suppressed when an overlay is open even if focus is Game"
        );
        assert!(
            !cell.modifier.contains(Modifier::REVERSED),
            "cursor REVERSED modifier must be absent when overlay is open"
        );
    }

    #[test]
    fn render_transcript_status_line_reversed() {
        let machine = minimal_machine();
        let state = AppState::default();

        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // Top row (y=0) should all have REVERSED modifier (status line background).
        let top_cell = buf.cell((0, 0)).expect("top-left cell should exist");
        assert!(
            top_cell.modifier.contains(Modifier::REVERSED),
            "top row should have REVERSED modifier for status line"
        );
    }

    #[test]
    fn render_transcript_applies_input_text_and_prompt_styles() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.command_bar = true; // this test exercises the dedicated bottom bar
        state.focus = Focus::Game;
        state.input.set("zq", true);
        state.colors.theme = theme_with_overrides(&[("input_prompt", Color::Green), ("input_text", Color::Red)]);

        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // Find the '>' prompt cell and the typed 'z' cell; check their fg.
        let mut prompt_fg = None;
        let mut text_fg = None;
        for y in 0..area.height {
            for x in 0..area.width {
                if let Some(c) = buf.cell((x, y)) {
                    match c.symbol() {
                        ">" if prompt_fg.is_none() => prompt_fg = Some(c.fg),
                        "z" if text_fg.is_none() => text_fg = Some(c.fg),
                        _ => {}
                    }
                }
            }
        }
        assert_eq!(prompt_fg, Some(Color::Green), "'>' uses input_prompt style");
        assert_eq!(text_fg, Some(Color::Red), "typed text uses input_text style");
    }

    // ── Inline-prompt mode (command_bar off) ──────────────────────────────────

    #[test]
    fn inline_draws_flush_prompt_and_cursor_no_bar() {
        // command_bar off: the live input is drawn flush after the game's kept `>`
        // on the last transcript row (">look", no space), with a block cursor
        // right after it, and the dedicated bottom bar is gone.
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.command_bar = false;
        state.transcript = vec![
            "You are in a hall.".to_string(),
            ">".to_string(),
        ];
        state.input.set("look", true);
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // Locate the row that renders the flush prompt+input.
        let mut hit = None;
        for y in 0..area.height {
            let row: String = (0..area.width)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect();
            if row.contains(">look") {
                hit = Some((y, row));
                break;
            }
        }
        let (y, row) = hit.expect("inline prompt row with '>look' must render");
        // Flush: '>' at col 0, "look" at cols 1..5, reverse-video block cursor at col 5.
        assert!(row.starts_with(">look"), "prompt+input must be flush: {:?}", row);
        assert_eq!(buf.cell((5, y)).unwrap().symbol(), " ", "block cursor sits right after '>look'");
        assert!(buf.cell((5, y)).unwrap().modifier.contains(Modifier::REVERSED));
        // The old dedicated bottom bar is dropped: the bottom row is blank.
        let bottom: String = (0..area.width)
            .map(|x| buf.cell((x, 9)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(!bottom.contains('>'), "no dedicated bottom input bar in inline mode: {:?}", bottom);
    }

    #[test]
    fn command_bar_mode_still_draws_bottom_bar() {
        // command_bar on: the dedicated bottom bar renders "> look" as before,
        // unaffected by the inline path.
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.command_bar = true;
        state.input.set("look", true);
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        let bottom: String = (0..area.width)
            .map(|x| buf.cell((x, 9)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(bottom.contains("> look"), "command-bar bottom row shows '> look': {:?}", bottom);
    }

    /// SQ-0542: a story-word completion must not move the input line — the bug that
    /// started this. The old candidate bar took its row out of the transcript
    /// viewport, so in inline-prompt mode (the default) every keystroke that gained
    /// or lost a candidate shifted the prompt row and every scrollback line with it,
    /// making the input bounce as you typed at the bottom of the pane.
    ///
    /// Renders the SAME state twice, once with candidates and once without, and
    /// requires the two frames to differ ONLY by the ghost tail: same prompt row,
    /// same scrollback, nothing reserved.
    #[test]
    fn story_word_completion_ghosts_without_moving_the_input_line() {
        let machine = minimal_machine();
        let render = |sugs: Vec<String>| -> Vec<String> {
            let mut state = AppState::default();
            state.config.command_bar = false; // inline prompt: the mode that bounced
            state.transcript = (0..50).map(|i| format!("L{i}")).collect();
            state.input.set("op", true);
            state.focus = Focus::Game;
            state.suggestions = sugs;
            let area = Rect::new(0, 0, 40, 12);
            let mut buf = Buffer::empty(area);
            render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);
            (0..area.height)
                .map(|y| {
                    (0..area.width)
                        .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                        .collect::<String>()
                })
                .collect()
        };
        let without = render(Vec::new());
        let with = render(vec!["open".to_string(), "operate".to_string()]);

        let prompt_row = |rows: &[String]| rows.iter().position(|r| r.contains("op")).expect("prompt row");
        assert_eq!(
            prompt_row(&without),
            prompt_row(&with),
            "the input line must not move when a completion appears\n  without: {:?}\n  with:    {:?}",
            without,
            with
        );
        // Every row above the prompt is untouched: no scrollback shifted, nothing reserved.
        let p = prompt_row(&without);
        for y in 0..p {
            assert_eq!(without[y], with[y], "row {y} shifted when the completion appeared");
        }
        // And the hint itself rides on the prompt row as the candidate's tail.
        assert!(with[p].contains("open"), "the ghost tail completes the typed word: {:?}", with[p]);
        assert!(!without[p].contains("open"), "no hint without candidates: {:?}", without[p]);
        // Nothing is drawn below the prompt row in either frame.
        for (y, row) in with.iter().enumerate().skip(p + 1) {
            assert!(row.trim().is_empty(), "row {y} below the prompt must stay empty: {row:?}");
        }
    }

    /// SQ-0542: the hint is a tail, so it only makes sense at the end of the line.
    /// Mid-line it would read as text you had typed.
    #[test]
    fn ghost_completion_hidden_when_the_caret_is_mid_line() {
        let mut state = AppState::default();
        state.focus = Focus::Game;
        state.input.set("op", true);
        state.suggestions = vec!["open".to_string()];
        assert_eq!(ghost_completion(&state).as_deref(), Some("en"), "at end of line the tail shows");
        state.input.home();
        assert_eq!(ghost_completion(&state), None, "mid-line the hint is suppressed");
    }

    /// SQ-0542: once Tab has applied the candidate the input already IS the word, so
    /// the tail is empty and the hint clears itself — no separate "applied" flag to
    /// keep in sync.
    #[test]
    fn ghost_completion_clears_once_the_candidate_is_applied() {
        let mut state = AppState::default();
        state.focus = Focus::Game;
        state.suggestions = vec!["open".to_string()];
        state.input.set("op", true);
        assert_eq!(ghost_completion(&state).as_deref(), Some("en"));
        state.input.set("open", true); // what Tab leaves behind
        assert_eq!(ghost_completion(&state), None);
    }

    /// SQ-0542: the command palette is deliberately untouched — its names match by
    /// substring and have no tail to show, so it keeps the candidate bar.
    #[test]
    fn ghost_completion_never_applies_to_the_command_palette() {
        let mut state = AppState::default();
        state.focus = Focus::Game;
        state.input.set("/set", true);
        state.suggestions = vec!["set-v6-render".to_string()];
        assert_eq!(ghost_completion(&state), None, "the palette keeps its bar, not a ghost");
    }

    /// SQ-0542: only the last WORD is completed, so a hint after an earlier word
    /// completes that word's tail and never re-completes the whole line.
    #[test]
    fn ghost_completion_completes_only_the_word_being_typed() {
        let mut state = AppState::default();
        state.focus = Focus::Game;
        state.input.set("take lan", true);
        state.suggestions = vec!["lantern".to_string()];
        assert_eq!(ghost_completion(&state).as_deref(), Some("tern"));
    }

    #[test]
    fn inline_input_suppressed_when_scrolled_up() {
        // command_bar off but scrolled up (effective_transcript_scroll > 0): the
        // live input must NOT be drawn, so scrolled-up history is never clobbered.
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.command_bar = false;
        state.transcript = (0..50).map(|i| format!("L{}", i)).collect();
        state.input.set("SECRETCMD", true);
        state.focus = Focus::Game;
        state.transcript_scroll = 5; // scrolled up from the bottom
        assert!(state.effective_transcript_scroll() > 0, "test must actually be scrolled up");

        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        for y in 0..area.height {
            let row: String = (0..area.width)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect();
            assert!(!row.contains("SECRETCMD"), "scrolled-up must not draw the live input: {:?}", row);
        }
    }

    #[test]
    fn more_pager_prompt_drawn_only_when_active() {
        // SQ-0404: an active pager reserves a bottom row for the `[more]` prompt.
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.transcript = (0..50).map(|i| format!("L{i}")).collect();
        let area = Rect::new(0, 0, 40, 12);

        let has_more = |state: &AppState| {
            let mut buf = Buffer::empty(area);
            render_transcript(&crate::session::status_model_from_machine(&machine), None, state, area, &mut buf, None);
            (0..area.height).any(|y| {
                let row: String = (0..area.width)
                    .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                    .collect();
                row.contains("[more]")
            })
        };

        state.pager.active = false;
        assert!(!has_more(&state), "no prompt when the pager is inactive");
        state.pager.active = true;
        assert!(has_more(&state), "the [more] prompt shows on the reserved row when active");
    }

    #[test]
    fn render_transcript_shows_scrollbar_when_overflowing() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        // Far more lines than the viewport → scrollbar must appear.
        state.transcript = (0..50).map(|i| format!("L{}", i)).collect();
        state.colors.theme =
            theme_with_overrides(&[("scrollbar", Color::Magenta), ("scrollbar_track", Color::Blue)]);
        state.scroll_transcript_to(1); // SQ-0782: the bar shows because you scrolled

        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // SQ-0782: the gutter is the rightmost column, and the bar in it is two
        // BACKGROUND fills with no glyph of their own — that is what gives the
        // text one column over a visual gutter.
        let mut thumb_rows = 0;
        let mut track_rows = 0;
        for y in 0..area.height {
            let cell = buf.cell((area.width - 1, y)).unwrap();
            match cell.bg {
                Color::Magenta => thumb_rows += 1,
                Color::Blue => track_rows += 1,
                _ => continue, // a row outside the transcript (status/input)
            }
            assert_eq!(cell.symbol(), " ", "the bar draws no glyph (row {y})");
        }
        assert!(thumb_rows > 0, "thumb is styleable via the `scrollbar` selector");
        assert!(track_rows > 0, "track is styleable via the `scrollbar_track` selector");
    }

    /// SQ-0782: the story pane's bar auto-hides. It is up right after a scroll,
    /// gone once the reveal window and its fade have passed, and a fresh state
    /// (nobody has scrolled yet) has never shown it at all.
    #[test]
    fn render_transcript_scrollbar_auto_hides_after_the_reveal_window() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.transcript = (0..50).map(|i| format!("L{}", i)).collect();
        state.colors.theme = theme_with_overrides(&[("scrollbar", Color::Magenta)]);
        state.config.animation.scrollbar_hide_ms = 50;
        state.config.animation.scrollbar_fade_ms = 0; // pop, so the test needn't sleep a fade
        let area = Rect::new(0, 0, 40, 12);

        let painted = |state: &AppState| {
            let mut buf = Buffer::empty(area);
            render_transcript(&crate::session::status_model_from_machine(&machine), None, state, area, &mut buf, None);
            (0..area.height).any(|y| buf.cell((area.width - 1, y)).unwrap().bg == Color::Magenta)
        };

        assert!(!painted(&state), "no bar before the first scroll of a session");
        state.scroll_transcript_to(3);
        assert!(painted(&state), "a scroll summons the bar");
        state.scrollbar_shown_at = Some(std::time::Instant::now() - std::time::Duration::from_millis(500));
        assert!(!painted(&state), "it hides once the reveal window has passed");
        // Game text arriving does NOT bring it back — that would flash it every turn.
        state.push_transcript("You are in a maze of twisty little passages.");
        assert!(!painted(&state), "new output leaves the bar hidden");
    }

    #[test]
    fn render_transcript_no_scrollbar_when_fits() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.transcript = vec!["only one line".to_string()];
        state.colors.theme = theme_with_overrides(&[("scrollbar", Color::Magenta)]);
        state.scroll_transcript_to(1); // even a scroll can't summon a bar with nothing to scroll

        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        let painted = (0..area.height)
            .any(|y| buf.cell((area.width - 1, y)).unwrap().bg == Color::Magenta);
        assert!(!painted, "no scrollbar when content fits");
    }

    #[test]
    fn render_transcript_scroll_offset() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        // 10 lines, scroll=5 should show lines 0..4 (end=10-5=5, start=5-4=1 for 4-row middle)
        state.transcript = (0..10).map(|i| format!("L{}", i)).collect();
        state.transcript_scroll = 5;

        // 7-row area: 1 status + 5 transcript + 1 input
        let area = Rect::new(0, 0, 40, 7);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // Middle rows y=1..5: should NOT show L9 (newest) but should show L4 or earlier.
        let found_l9 = (1u16..6u16).any(|y| {
            let row: String = (0..40u16)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect();
            row.contains("L9")
        });
        assert!(!found_l9, "L9 (newest) should not be visible when scrolled back 5");
    }

    // ── Status line test with czech.z5 fixture (skipped if absent) ───────────

    // ── format_suggestion_line tests ─────────────────────────────────────────

    #[test]
    fn format_suggestion_line_empty() {
        assert_eq!(format_suggestion_line(&[], 0), "");
    }

    #[test]
    fn format_suggestion_line_single_highlighted() {
        let sug = vec!["north".to_string()];
        let line = format_suggestion_line(&sug, 0);
        assert_eq!(line, "[north]");
    }

    #[test]
    fn format_suggestion_line_highlight_first() {
        let sug = vec!["north".to_string(), "northeast".to_string(), "northwest".to_string()];
        let line = format_suggestion_line(&sug, 0);
        assert!(line.starts_with("[north]"), "first entry should be highlighted: {}", line);
        assert!(line.contains("northeast") && !line.contains("[northeast]"));
    }

    #[test]
    fn format_suggestion_line_highlight_second() {
        let sug = vec!["north".to_string(), "northeast".to_string()];
        let line = format_suggestion_line(&sug, 1);
        assert!(line.contains("[northeast]"), "second entry highlighted: {}", line);
        assert!(!line.contains("[north]"), "first not highlighted: {}", line);
    }

    #[test]
    fn format_suggestion_line_idx_wraps() {
        let sug = vec!["north".to_string(), "northeast".to_string()];
        // idx=2 wraps to 0
        let line = format_suggestion_line(&sug, 2);
        assert!(line.starts_with("[north]"), "idx wraps: {}", line);
    }

    // ── visible_suggestion_line (horizontal scroll) tests ────────────────────
    #[test]
    fn visible_suggestion_line_returns_full_when_it_fits() {
        let sug = vec!["north".to_string(), "south".to_string()];
        let full = format_suggestion_line(&sug, 0);
        // Plenty of width: unchanged.
        assert_eq!(visible_suggestion_line(&sug, 0, 80), full);
    }

    #[test]
    fn visible_suggestion_line_no_scroll_when_highlight_near_start() {
        let sug = vec!["aa".to_string(), "bb".to_string(), "cccccccc".to_string()];
        // Full line: "[aa]  bb  cccccccc" (18 chars). Width 10 overflows, but the
        // highlighted entry sits at the start, so no scroll is needed.
        let out = visible_suggestion_line(&sug, 0, 10);
        assert!(out.starts_with("[aa]"), "highlight at start stays visible: {out:?}");
        assert_eq!(out.chars().count(), 10);
    }

    #[test]
    fn visible_suggestion_line_scrolls_to_keep_highlight_visible() {
        let sug = vec!["aaaa".to_string(), "bbbb".to_string(), "cccc".to_string()];
        // Full line: "aaaa  bbbb  [cccc]" (18 chars). At width 10 the highlighted
        // last entry would be clipped off the right — it must scroll into view.
        let out = visible_suggestion_line(&sug, 2, 10);
        assert!(out.contains("[cccc]"), "highlighted entry must stay on screen: {out:?}");
        assert_eq!(out.chars().count(), 10);
    }

    #[test]
    fn visible_suggestion_line_entry_wider_than_window_shows_opening_bracket() {
        let sug = vec!["xx".to_string(), "supercalifragilistic".to_string()];
        // The highlighted entry alone exceeds the window; anchor on its start so
        // the opening bracket is visible.
        let out = visible_suggestion_line(&sug, 1, 8);
        assert!(out.starts_with("[super"), "anchors on the opening bracket: {out:?}");
        assert_eq!(out.chars().count(), 8);
    }

    #[test]
    fn render_transcript_shows_suggestion_line_above_input() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.command_bar = true; // this test exercises the dedicated bottom bar
        state.focus = Focus::Game;
        // SQ-0542: the bar belongs to the command palette now.
        state.input.set("/set", true);
        state.suggestions = vec!["set-v6-render".to_string()];
        state.suggestion_idx = 0;

        // 10-row area: row 0=status, rows 1..7=transcript, row 8=suggestion, row 9=input
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // Row 9 (bottom) must contain the input.
        let input_row: String = (0..40u16)
            .map(|x| buf.cell((x, 9)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(input_row.contains("> /set"), "input row: {:?}", input_row);

        // Row 8 must contain the suggestion.
        let sug_row: String = (0..40u16)
            .map(|x| buf.cell((x, 8)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(sug_row.contains("set-v6-render"), "suggestion row: {:?}", sug_row);
    }

    #[test]
    fn transcript_color_override_paints_line_in_its_style() {
        use ratatui::style::{Color, Style};
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.push_transcript_styled("connector sample", TranscriptKind::Meta, Style::new().fg(Color::Cyan));
        let area = Rect::new(0, 0, 60, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);
        // Find a cell of the line and assert its fg is Cyan (the override), not transcript_meta.
        let found = (0..area.height).any(|y| (0..area.width).any(|x| {
            let c = &buf[(x, y)];
            c.symbol().starts_with('c') && c.style().fg == Some(Color::Cyan)
        }));
        assert!(found, "color override paints the line in its style");
    }

    #[test]
    fn render_transcript_status_line_nonblank_with_fixture() {
        // Relative to the crate, not to one checkout. This was an absolute path
        // into a developer's home directory, so it skipped vacuously for everyone
        // else and would have gone on doing so silently; the lanthorn rename broke
        // it outright, which is the only reason anyone noticed.
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        if !fixture.exists() {
            eprintln!("SKIP: czech.z5 fixture not found");
            return;
        }

        let data = std::fs::read(fixture).expect("read czech.z5");
        let mem = zvm::memory::Memory::new(data).expect("parse czech.z5");
        let machine = Machine::new(mem);

        // czech.z5 is a v5 story → HostManaged → synthesized bar is removed.
        // Without a status_msg the bar occupies 0 rows; y=0 is transcript content,
        // not a reversed-video status line.
        let state = AppState::default();
        let area = Rect::new(0, 0, 80, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // Top row must NOT have the reversed modifier (the synthesized bar is gone).
        let top_has_reversed = (0..80u16).all(|x| {
            buf.cell((x, 0))
                .map(|c| c.modifier.contains(Modifier::REVERSED))
                .unwrap_or(false)
        });
        assert!(
            !top_has_reversed,
            "v5 HostManaged story must not show reversed status bar when no status_msg"
        );
    }

    // ── Inventory: no longer rendered in-pane ─────────────────────────────────

    #[test]
    fn render_transcript_never_shows_inventory_strip() {
        // The inventory moved to the docked panel (render::inventory_dock); this
        // pane must never draw an "Inv:" strip regardless of show_inventory.
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.show_inventory = true;
        state.inventory_fallback = vec!["brass lamp".to_string(), "sword".to_string()];

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        let found_inv = (0..10u16).any(|y| {
            let row: String = (0..40u16)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect();
            row.contains("Inv:")
        });
        assert!(!found_inv, "Inv: strip must not appear inside the transcript pane");
    }

    #[test]
    fn inventory_items_live_vs_fallback() {
        // player_obj known + introspect available → not exercised here (no fake
        // Introspect impl in this module); covered by the (player_obj, None) and
        // (None, _) fallback arms.
        let items = vec!["brass lamp".to_string()];
        assert_eq!(inventory_items(None, &items, None), items);
        assert_eq!(inventory_items(Some(7), &items, None), items);
        assert!(inventory_items(None, &[], None).is_empty());
    }

    /// SQ-1244: with no introspection, `inventory_click_words` falls back to
    /// the same fallback text as `inventory_items` — the two must always be
    /// the same length so a click index resolves against the right row. The
    /// object-tree path (where the two diverge, e.g. "brass lantern" shown
    /// but `lamp` clicked) is covered against a real story in
    /// `tests/suites/zork1_inventory.rs`, which has no fake `Introspect` here
    /// to drive it with.
    #[test]
    fn inventory_click_words_matches_inventory_items_length_with_no_introspection() {
        let items = vec!["brass lamp".to_string(), "sword".to_string()];
        assert_eq!(inventory_click_words(None, &items, None, None), items);
        assert_eq!(inventory_click_words(Some(7), &items, None, None), items);
        assert!(inventory_click_words(None, &[], None, None).is_empty());
    }

    // ── Task 8: status-header + input-line boxing + opt-out ───────────────────

    /// Default (status_header_style = None): top row is the plain reversed bar
    /// with no border glyphs.  When status_header_style = Single, the status row
    /// is wrapped in a box (3 rows: top border, content, bottom border).
    #[test]
    fn status_header_plain_by_default_boxed_when_styled() {
        let machine = minimal_machine();

        // -- Default: plain reversed bar, no border glyphs --
        {
            let state = AppState::default();
            // status_header_style defaults to None
            assert!(
                matches!(state.colors.status_header_style, BorderStyle::None),
                "default status_header_style must be None"
            );

            let area = Rect::new(0, 0, 40, 10);
            let mut buf = Buffer::empty(area);
            render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

            // Row 0 (status) must have REVERSED modifier (plain bar style).
            let top_cell = buf.cell((0, 0)).expect("top-left must exist");
            assert!(
                top_cell.modifier.contains(Modifier::REVERSED),
                "default status row must be reversed-video (plain bar)"
            );

            // No box corners in the top 3 rows.
            let has_corner_glyph = (0..3u16).any(|y| {
                (0..40u16).any(|x| {
                    let sym = buf.cell((x, y)).map(|c| c.symbol()).unwrap_or("");
                    matches!(sym, "┌" | "└" | "┐" | "┘" | "╔" | "╚" | "╗" | "╝" | "┏" | "┗" | "┓" | "┛")
                })
            });
            assert!(
                !has_corner_glyph,
                "default (None) status header must not render box corners"
            );
        }

        // -- Boxed: status_header_style = Single → 3-row box around status --
        {
            let mut state = AppState::default();
            state.colors.status_header_style = BorderStyle::Single;
            state.colors.status_header_sides = crate::render::paneframe::PaneSides::all(BorderStyle::Single);

            // Use a large enough area so boxing is not suppressed (needs >= 5 rows).
            let area = Rect::new(0, 0, 40, 12);
            let mut buf = Buffer::empty(area);
            render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

            // Row 0 must be the top border: top-left corner must be "┌".
            assert_eq!(
                buf.cell((0, 0)).unwrap().symbol(),
                "┌",
                "boxed status header top-left must be a single-border corner"
            );
            // Row 1 (content) must have the status text with REVERSED style.
            // col 0 is the side border glyph; col 1 is the first content cell.
            let content_cell = buf.cell((1, 1)).expect("status content row must exist");
            assert!(
                content_cell.modifier.contains(Modifier::REVERSED),
                "status content row (inside box) must have REVERSED modifier"
            );
            // Row 2 must be the bottom border: bottom-left corner must be "└".
            assert_eq!(
                buf.cell((0, 2)).unwrap().symbol(),
                "└",
                "boxed status header bottom-left must be a single-border corner"
            );
        }
    }

    /// Default (input_line_style = None): bottom row is a plain `> ` prompt with
    /// no border glyphs.
    #[test]
    fn input_line_plain_by_default() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.command_bar = true; // this test exercises the dedicated bottom bar
        state.input.set("go north", true);
        state.focus = Focus::Game;

        // input_line_style defaults to None
        assert!(
            matches!(state.colors.input_line_style, BorderStyle::None),
            "default input_line_style must be None"
        );

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // Bottom row (y=9) must contain "> go north" (plain, no box).
        let bottom_row: String = (0..40u16)
            .map(|x| buf.cell((x, 9)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(
            bottom_row.contains("> go north"),
            "default input row must contain '> go north'; got: {:?}",
            bottom_row
        );

        // No corner glyphs in the bottom 3 rows.
        let has_corner = (7u16..10u16).any(|y| {
            (0..40u16).any(|x| {
                let sym = buf.cell((x, y)).map(|c| c.symbol()).unwrap_or("");
                matches!(sym, "┌" | "└" | "┐" | "┘" | "╔" | "╚" | "╗" | "╝" | "┏" | "┗" | "┓" | "┛")
            })
        });
        assert!(
            !has_corner,
            "default (None) input line must not render box corners"
        );
    }

    /// With map_border_style = None and story_border_style = None, calling draw_pane_frame
    /// with those styles produces no border glyphs (the opt-out path).  This is the
    /// "plain borderless" mode that reproduces the pre-beautification pane appearance.
    #[test]
    fn panes_none_reproduce_plain_borderless() {
        use crate::render::paneframe::{draw_pane_frame, BorderStyle, PaneGlyphs};
        use ratatui::style::Style;

        // Resolve a color scheme with none borders (simulate `map_border = none`).
        let area = Rect::new(0, 0, 20, 10);
        let mut buf_map = Buffer::empty(area);
        let frame = draw_pane_frame(&mut buf_map, area, BorderStyle::None, &PaneGlyphs::default(), Style::default());

        // Content must equal the full area (no inset).
        assert_eq!(
            frame.content, area,
            "BorderStyle::None must return content == area (no inset)"
        );

        // No border glyphs anywhere in the buffer.
        let has_border_glyph = (0..10u16).any(|y| {
            (0..20u16).any(|x| {
                let sym = buf_map.cell((x, y)).map(|c| c.symbol()).unwrap_or("");
                matches!(sym,
                    "┌" | "─" | "┐" | "│" | "└" | "┘" |
                    "╔" | "═" | "╗" | "║" | "╚" | "╝" |
                    "┏" | "━" | "┓" | "┃" | "┗" | "┛"
                )
            })
        });
        assert!(
            !has_border_glyph,
            "BorderStyle::None must not render any border glyphs (opt-out path)"
        );

        // Same for story pane simulation.
        let mut buf_story = Buffer::empty(area);
        let story_frame = draw_pane_frame(&mut buf_story, area, BorderStyle::None, &PaneGlyphs::default(), Style::default());
        assert_eq!(story_frame.content, area, "story pane None border must also have content == area");
    }

    #[test]
    fn status_header_left_right_only_draws_side_bars_no_top() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        // base none, left/right single, large enough to box.
        state.colors.status_header_style = crate::render::paneframe::BorderStyle::None;
        state.colors.status_header_sides = crate::render::paneframe::PaneSides {
            top: crate::render::paneframe::BorderStyle::None,
            bottom: crate::render::paneframe::BorderStyle::None,
            left: crate::render::paneframe::BorderStyle::Single,
            right: crate::render::paneframe::BorderStyle::Single,
        };
        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);
        // The left side bar must actually be drawn in column 0 of the boxed status
        // region (rows 0..3) — this is the headline left/right-only use case and
        // must NOT be inert. (Regression guard: with the old base-only boxing gate
        // nothing was boxed and this column was blank.)
        let has_left_bar = (0u16..3).any(|y| buf.cell((0, y)).map(|c| c.symbol()) == Some("│"));
        assert!(has_left_bar, "left/right-only status header must draw a side bar in column 0");
        // The right side bar too.
        let has_right_bar = (0u16..3).any(|y| buf.cell((39, y)).map(|c| c.symbol()) == Some("│"));
        assert!(has_right_bar, "left/right-only status header must draw a side bar at the right edge");
        // And no top corner glyph (top side is off).
        assert_ne!(buf.cell((0, 0)).unwrap().symbol(), "┌");
    }

    // ── draw_str_highlighted regression tests ────────────────────────────────

    /// Helper: render `draw_str_highlighted` into a fresh buffer and return the
    /// symbol string for row 0 (concatenated cell symbols).
    fn highlighted_row(text: &str, query_lower: &str, width: u16) -> String {
        let area = Rect::new(0, 0, width, 1);
        let mut buf = Buffer::empty(area);
        draw_str_highlighted(
            &mut buf,
            0, 0,
            text,
            Style::default(),
            query_lower,
            Style::new().fg(ratatui::style::Color::Yellow),
            area,
        );
        (0..width)
            .map(|x| buf.cell((x, 0)).map(|c| c.symbol().to_string()).unwrap_or_default())
            .collect()
    }

    /// Turkish dotted-I (U+0130, 2 UTF-8 bytes) lowercases to the two-byte
    /// sequence "i\u{307}" (3 UTF-8 bytes).  When such a char precedes the
    /// search query, the old byte-offset arithmetic would produce a
    /// non-char-boundary panic.  This test verifies the fix: no panic and the
    /// correct glyphs appear in the buffer.
    #[test]
    fn draw_str_highlighted_dotted_i_no_panic() {
        // "İkey diary" with query "key": İ precedes the match, offsets shift.
        let row = highlighted_row("İkey diary", "key", 20);
        // Must contain the glyphs for the original text (no panic).
        assert!(row.contains('İ'), "dotted-I glyph must appear; got: {:?}", row);
        assert!(row.contains('k'), "k of key must appear; got: {:?}", row);
        assert!(row.contains('e'), "e of key must appear; got: {:?}", row);
        assert!(row.contains('y'), "y of key must appear; got: {:?}", row);
    }

    /// Verify the highlight STYLE is applied to the matched segment and only to
    /// it.  We use a plain ASCII line so the style boundaries are unambiguous.
    #[test]
    fn draw_str_highlighted_ascii_highlight_style() {
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        let highlight_style = Style::new().fg(ratatui::style::Color::Yellow);
        draw_str_highlighted(
            &mut buf,
            0, 0,
            "hello world",
            Style::default(),
            "world",
            highlight_style,
            area,
        );

        // "hello " (6 chars, x=0..5) must NOT have Yellow fg.
        for x in 0u16..6 {
            let cell = buf.cell((x, 0)).unwrap();
            assert_ne!(
                cell.style().fg,
                Some(ratatui::style::Color::Yellow),
                "x={} should not be highlighted", x
            );
        }
        // "world" (5 chars, x=6..10) must have Yellow fg.
        for x in 6u16..11 {
            let cell = buf.cell((x, 0)).unwrap();
            assert_eq!(
                cell.style().fg,
                Some(ratatui::style::Color::Yellow),
                "x={} should be highlighted", x
            );
        }
    }

    /// Multiple occurrences of the query on the same line all get highlighted.
    #[test]
    fn draw_str_highlighted_multiple_matches() {
        let area = Rect::new(0, 0, 30, 1);
        let mut buf = Buffer::empty(area);
        let highlight_style = Style::new().fg(ratatui::style::Color::Yellow);
        // "aXa" with query "a": matches at x=0 and x=2.
        draw_str_highlighted(
            &mut buf,
            0, 0,
            "aXa",
            Style::default(),
            "a",
            highlight_style,
            area,
        );
        let cell0 = buf.cell((0, 0)).unwrap();
        let cell2 = buf.cell((2, 0)).unwrap();
        assert_eq!(cell0.style().fg, Some(ratatui::style::Color::Yellow), "first 'a' highlighted");
        assert_eq!(cell2.style().fg, Some(ratatui::style::Color::Yellow), "second 'a' highlighted");
        let cell1 = buf.cell((1, 0)).unwrap();
        assert_ne!(cell1.style().fg, Some(ratatui::style::Color::Yellow), "'X' not highlighted");
    }

    /// Empty query draws text with base style and no panic.
    #[test]
    fn draw_str_highlighted_empty_query_no_highlight() {
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        draw_str_highlighted(
            &mut buf,
            0, 0,
            "hello",
            Style::default(),
            "",
            Style::new().fg(ratatui::style::Color::Yellow),
            area,
        );
        for x in 0u16..5 {
            let cell = buf.cell((x, 0)).unwrap();
            assert_ne!(cell.style().fg, Some(ratatui::style::Color::Yellow), "no highlight for empty query at x={}", x);
        }
    }

    /// Query longer than text produces no match and no panic.
    #[test]
    fn draw_str_highlighted_query_longer_than_text_no_panic() {
        let row = highlighted_row("hi", "hello world", 10);
        assert!(row.contains('h'), "text glyphs must still appear; got: {:?}", row);
    }

    #[test]
    fn hanging_indent_wraps_continuations() {
        // A 2-space-indented line longer than width wraps with continuations
        // indented 2 spaces.
        let line = "  abcd efgh ijkl mnop";
        let rows = wrap_line_hanging(line, 10, 2);
        assert!(rows.len() >= 2);
        assert!(rows[0].starts_with("  abcd"), "first row keeps original indent");
        for cont in &rows[1..] {
            assert!(cont.starts_with("  "), "continuation '{cont}' is indented 2");
        }
    }

    // ── Engine-abstraction equivalence (3b-i) ─────────────────────────────────
    //
    // These prove the new ScreenModel-fed render path carries exactly the same
    // facts the old machine-fed path read, so output stays byte-identical.

    /// The v3 status model carries the same location + score/turns the render
    /// path formerly read from `machine.status_line()`.
    #[test]
    fn status_model_mirrors_v3_status_line() {
        let m = minimal_machine(); // v3
        let model = crate::session::status_model_from_machine(&m);
        let sl = m.status_line();
        match model {
            StatusModel::Classic { location, right } => {
                assert_eq!(location, sl.location);
                match (right, sl.right) {
                    (
                        StatusField::ScoreTurns { score, turns },
                        zvm::screen::StatusRight::ScoreTurns { score: s2, turns: t2 },
                    ) => assert_eq!((score, turns), (s2, t2)),
                    (
                        StatusField::Time { hours, minutes },
                        zvm::screen::StatusRight::Time { hours: h2, minutes: m2 },
                    ) => assert_eq!((hours, minutes), (h2, m2)),
                    _ => panic!("right-field variant mismatch"),
                }
            }
            other => panic!("v3 must yield Classic, got {other:?}"),
        }
    }

    /// A v4+ machine has no automatic status line in the model (the app draws
    /// its own room/turn info), matching the old `version() >= 4` branch.
    #[test]
    fn status_model_is_host_managed_for_v4() {
        let m = minimal_machine_v4();
        assert_eq!(crate::session::status_model_from_machine(&m), StatusModel::HostManaged);
    }

    /// Painting the engine upper window then rendering through the ScreenModel
    /// grid reproduces those cells exactly (the v4+ upper-window render path).
    #[test]
    fn upper_window_model_render_reproduces_painted_grid() {
        use crate::render::paneframe::{BorderStyle, PaneSides};
        let mut m = minimal_machine_v4();
        m.screen.upper.resize(1, 3);
        m.screen.upper.put(1, 1, 'A', 0, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default);
        m.screen.upper.put(1, 3, 'Z', 0, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default);
        m.screen.upper_window_rows = 1;

        let model = crate::session::screen_model_from_machine(&m);
        let grid = model.grid().expect("model carries a grid");

        let mut colors = crate::colors::ColorScheme::terminal_default();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = PaneSides::all(BorderStyle::None);

        let area = Rect::new(0, 0, 9, 3);
        let mut buf = Buffer::empty(area);
        let used = crate::render::upper_window::draw_upper_window(grid, false, &colors, area, &mut buf, true, &mut Vec::new());
        // SQ-0286: the Z-machine upper window's border preference is Unspecified, so
        // a theme with every border side disabled renders frameless (the theme
        // decides). The painted cells reproduce exactly at row 0.
        assert_eq!(used, 1, "one active upper row consumed, no frame");
        // cols=3 centered in 9 → x_off = 3.
        assert_eq!(buf.cell((3, 0)).unwrap().symbol(), "A");
        assert_eq!(buf.cell((5, 0)).unwrap().symbol(), "Z");
    }

    // ── Transcript wrap cache (SQ-0305, SQ-1034) ──────────────────────────────
    //
    // The cache holds the fully wrapped rows so an unchanged transcript is not
    // re-wrapped. These tests POISON the cached rows with a sentinel after a
    // render, then render again, which observes the three outcomes directly and
    // without a counter: a REUSE leaves the sentinel alone, an APPEND leaves it
    // alone and adds rows after it, and a REBUILD wipes it.
    //
    // The sentinel row is what a re-wrap can never produce, so "is it still
    // there?" is exactly "did this frame re-wrap line zero?".

    fn wrap_render(state: &AppState, area: Rect) {
        let machine = minimal_machine();
        let status = crate::session::status_model_from_machine(&machine);
        let mut buf = Buffer::empty(area);
        render_transcript(&status, None, state, area, &mut buf, None);
    }

    fn poison_wrap_cache(state: &AppState) {
        let mut c = state.transcript_wrap.borrow_mut();
        let e = c.as_mut().expect("cache built by a prior render");
        e.rows.clear();
        e.rows.push(WrappedRow {
            text: "SENTINEL".to_string(),
            kind: TranscriptKind::Story,
            style: Style::default(),
            runs: Vec::new(),
            band: None,
            float: None,
        });
        // Keep the product self-consistent: `stable_rows` is where an append
        // truncates back to, so a poisoned row that sat past it would be silently
        // dropped and the probe would report a rebuild that never happened.
        e.stable_rows = e.rows.len();
        e.starts = vec![0];
    }

    fn cached_row_texts(state: &AppState) -> Vec<String> {
        state
            .transcript_wrap
            .borrow()
            .as_ref()
            .expect("cache present")
            .rows
            .iter()
            .map(|r| r.text.clone())
            .collect()
    }

    fn cached_first_text(state: &AppState) -> String {
        state
            .transcript_wrap
            .borrow()
            .as_ref()
            .expect("cache present")
            .rows
            .first()
            .map(|r| r.text.clone())
            .unwrap_or_default()
    }

    #[test]
    fn wrap_cache_hit_reuses_rows_when_nothing_changed() {
        let mut state = AppState::default();
        state.push_transcript_kind("hello world", TranscriptKind::Story);
        let area = Rect::new(0, 0, 20, 8);
        wrap_render(&state, area);
        poison_wrap_cache(&state);
        // Nothing changed → the second render must reuse the cached rows.
        wrap_render(&state, area);
        assert_eq!(cached_first_text(&state), "SENTINEL", "unchanged transcript must not re-wrap");
    }

    #[test]
    fn wrap_cache_extends_on_append_without_rewrapping_what_it_already_wrapped() {
        let mut state = AppState::default();
        state.push_transcript_kind("hello world", TranscriptKind::Story);
        let area = Rect::new(0, 0, 20, 8);
        wrap_render(&state, area);
        poison_wrap_cache(&state);
        state.push_transcript_kind("more", TranscriptKind::Story); // grows, nothing moves
        wrap_render(&state, area);
        // The sentinel survives: this frame wrapped ONE new line rather than the
        // whole history, which is the whole of SQ-1034. Before it, the same frame
        // re-wrapped from line zero and 20,000 turns of scrollback cost 17.9 ms.
        assert_eq!(
            cached_row_texts(&state),
            vec!["SENTINEL".to_string(), "more".to_string()],
            "an append must EXTEND the wrapped rows, not rebuild them"
        );
    }

    // ── Append == rebuild (SQ-1034) ───────────────────────────────────────────
    //
    // The property the incremental wrap rests on, and the only one that matters:
    // a product reached by N appends must be EXACTLY the product one rebuild
    // would have produced. Everything else here is a performance claim; this is
    // the correctness claim, and it is asserted directly rather than sampled —
    // two states are driven to the same transcript by different routes and their
    // whole wrapped products are compared field by field.

    /// Every wrapped row, projected to something comparable. `WrappedRow` is not
    /// `PartialEq` (it carries an `InlineImage`), so the image is compared by the
    /// Arc it points at plus its band geometry — which is what the blitter reads.
    fn wrap_product(state: &AppState) -> Vec<String> {
        fn band(b: &Option<ImageBand>) -> String {
            match b {
                None => "-".to_string(),
                Some(b) => format!(
                    "{:p}/{}x{}@{}+{}",
                    std::sync::Arc::as_ptr(&b.image.pixels),
                    b.cols,
                    b.rows,
                    b.row,
                    b.x_off
                ),
            }
        }
        state
            .transcript_wrap
            .borrow()
            .as_ref()
            .expect("cache present")
            .rows
            .iter()
            .map(|r| {
                format!(
                    "{:?}|{:?}|{:?}|{:?}|{}|{}",
                    r.text,
                    r.kind,
                    r.style,
                    r.runs,
                    band(&r.band),
                    band(&r.float)
                )
            })
            .collect()
    }

    /// The wrap product's bookkeeping, which the draw path reads and a stale
    /// value in which would misplace a whole screen.
    fn wrap_bookkeeping(state: &AppState) -> (Option<usize>, Option<usize>, Vec<usize>, usize) {
        let c = state.transcript_wrap.borrow();
        let c = c.as_ref().expect("cache present");
        (c.anchor_row, c.clear_anchor_filtered, c.starts.clone(), c.live_bands.len())
    }

    /// Drive `state` through the same script every append test uses: prose that
    /// wraps, a multi-line push (hard newlines), a Meta line with its own gutter
    /// wrap, styled runs, and a left-margin float whose picture outruns the text
    /// beside it — which is the case that makes the trailing flush non-final.
    ///
    /// `render_after_each` is what separates the two routes: true drives a frame
    /// between every step (so the cache appends, over and over), false pushes the
    /// lot and renders once (so the cache rebuilds).
    /// A state the script can exercise every branch of the wrap in: a picker, so
    /// inline images are emitted at all (without one the float path is dead code
    /// and the comparison is vacuous), and a resolved theme.
    fn script_state() -> AppState {
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state
    }

    /// ONE picture, shared by both routes through the script.
    ///
    /// Shared rather than built twice so the products can be compared by the Arc
    /// the rows point at — which is not pedantry: `live_bands` is a set of exactly
    /// those pointers, and it is what bounds the inline-image protocol cache. Two
    /// separately-allocated copies of the same pixels would compare unequal here
    /// while being indistinguishable on screen, so the comparison would have had
    /// to drop the identity and stop checking the thing that matters.
    fn script_image() -> crate::inline_image::InlineImage {
        static IMG: std::sync::OnceLock<crate::inline_image::InlineImage> = std::sync::OnceLock::new();
        IMG.get_or_init(|| left_img(24, 64, Some(32))).clone()
    }

    fn drive_script(state: &mut AppState, area: Rect, render_after_each: bool) {
        let step = |state: &AppState| {
            if render_after_each {
                wrap_render(state, area);
            }
        };
        state.push_transcript_kind("You are standing in an open field west of a white house.", TranscriptKind::Story);
        step(state);
        state.push_transcript_kind("one\ntwo\nthree", TranscriptKind::Story);
        step(state);
        state.push_transcript_kind("> look", TranscriptKind::Input);
        step(state);
        state.push_transcript_kind("a meta line long enough to wrap past the gutter it reserves", TranscriptKind::Meta);
        step(state);
        // A picture four rows tall with only two short lines beside it: two rows
        // ride the text and two are flushed as strips — then more prose arrives
        // and takes them over. That retake is what `stable_rows` exists for.
        state.push_transcript_image(script_image());
        step(state);
        state.push_transcript_kind("beside one", TranscriptKind::Story);
        step(state);
        state.push_transcript_kind("beside two", TranscriptKind::Story);
        step(state);
        state.push_transcript_kind(&"word ".repeat(30), TranscriptKind::Story);
        step(state);
        state.push_transcript_kind("tail", TranscriptKind::Story);
        step(state);
    }

    #[test]
    fn appending_lands_on_exactly_what_a_rebuild_would_have_produced() {
        let area = Rect::new(0, 0, 34, 12);

        let mut incremental = script_state();
        drive_script(&mut incremental, area, true);
        wrap_render(&incremental, area);

        let mut rebuilt = script_state();
        drive_script(&mut rebuilt, area, false);
        wrap_render(&rebuilt, area);

        // Non-vacuity by SHAPE, not by count: the comparison is worthless unless
        // the script actually reached the branches that carry state across lines.
        let rows = wrap_product(&rebuilt);
        assert!(rows.len() > 15, "the script must produce a real wrap, got {} rows", rows.len());
        assert!(
            rows.iter().filter(|r| r.ends_with("+0") && r.contains("/3x4@")).count() == 4,
            "all four strips of the float must be placed beside prose: {rows:#?}"
        );
        assert!(rows.iter().any(|r| r.contains("|Meta|")), "the hanging-indent wrap must be exercised");
        assert_eq!(
            wrap_product(&incremental),
            wrap_product(&rebuilt),
            "nine appends must produce the same rows as one rebuild"
        );
        assert_eq!(
            wrap_bookkeeping(&incremental),
            wrap_bookkeeping(&rebuilt),
            "and the same anchor, line index and live-band set"
        );
    }

    #[test]
    fn appending_across_a_resize_lands_on_exactly_what_a_rebuild_would_have_produced() {
        // A resize is the common LAYOUT move, and the one the raster path never
        // makes — which is exactly why it has to be pinned on the path that does.
        let narrow = Rect::new(0, 0, 28, 12);
        let wide = Rect::new(0, 0, 52, 12);

        let mut incremental = script_state();
        drive_script(&mut incremental, narrow, true);
        // Resize partway, then keep appending at the new width.
        wrap_render(&incremental, wide);
        incremental.push_transcript_kind(&"after the resize ".repeat(6), TranscriptKind::Story);
        wrap_render(&incremental, wide);
        incremental.push_transcript_kind("last", TranscriptKind::Story);
        wrap_render(&incremental, wide);

        let mut rebuilt = script_state();
        drive_script(&mut rebuilt, wide, false);
        rebuilt.push_transcript_kind(&"after the resize ".repeat(6), TranscriptKind::Story);
        rebuilt.push_transcript_kind("last", TranscriptKind::Story);
        wrap_render(&rebuilt, wide);

        assert_eq!(
            wrap_product(&incremental),
            wrap_product(&rebuilt),
            "a resize mid-stream must leave the same rows a fresh wrap at that width gives"
        );
        assert_eq!(wrap_bookkeeping(&incremental), wrap_bookkeeping(&rebuilt));
    }

    #[test]
    fn appending_after_a_screen_clear_lands_on_exactly_what_a_rebuild_would_have_produced() {
        // `clear_anchor` is in the key, so moving it rebuilds — but the ANCHOR ROW
        // it resolves to moves as lines are appended without it moving at all: an
        // anchor at the very end is an empty post-clear screen until something is
        // printed into it (SQ-0748). That is recomputed per append, and this is
        // what says so.
        let area = Rect::new(0, 0, 34, 12);

        let mut incremental = script_state();
        drive_script(&mut incremental, area, true);
        incremental.mark_screen_clear();
        wrap_render(&incremental, area);
        assert_eq!(
            wrap_bookkeeping(&incremental).0,
            Some(wrap_product(&incremental).len()),
            "non-vacuity: a clear with nothing printed since anchors past the last row"
        );
        incremental.push_transcript_kind("after the clear", TranscriptKind::Story);
        wrap_render(&incremental, area);

        let mut rebuilt = script_state();
        drive_script(&mut rebuilt, area, false);
        rebuilt.mark_screen_clear();
        rebuilt.push_transcript_kind("after the clear", TranscriptKind::Story);
        wrap_render(&rebuilt, area);

        assert_eq!(wrap_product(&incremental), wrap_product(&rebuilt));
        assert_eq!(
            wrap_bookkeeping(&incremental),
            wrap_bookkeeping(&rebuilt),
            "the anchor row must follow the line that was printed into the cleared screen"
        );
        assert!(
            wrap_bookkeeping(&incremental).0.is_some_and(|a| a < wrap_product(&incremental).len()),
            "non-vacuity: printing into the cleared screen must give the anchor a real row"
        );
    }

    #[test]
    fn a_pure_screen_clear_does_not_rewrap_anything_already_wrapped() {
        // SQ-1179 (B): before this fix, `clear_anchor` moving in `WrapShape`
        // meant a screen clear ALONE — nothing printed, nothing else moved —
        // still forced a whole rebuild. It no longer does, because
        // `mark_screen_clear` always sets the new anchor to the CURRENT
        // transcript length, and every already-cached filtered line then
        // unconditionally precedes it (`WrapKey::plan`'s anchor-safety guard).
        // Proven the way SQ-1034's own tests prove an append doesn't rewrap:
        // poison the cached rows with a sentinel a re-wrap can never produce,
        // then check it survives.
        let area = Rect::new(0, 0, 34, 12);
        let mut state = script_state();
        drive_script(&mut state, area, true);
        wrap_render(&state, area);
        poison_wrap_cache(&state);

        state.mark_screen_clear();
        wrap_render(&state, area);
        assert_eq!(
            cached_first_text(&state),
            "SENTINEL",
            "a screen clear alone must not re-wrap what was already wrapped"
        );
        // …and the anchor bookkeeping it exists to move is nevertheless correct:
        // an anchor with nothing printed since it anchors past the last row.
        assert_eq!(
            wrap_bookkeeping(&state).0,
            Some(cached_row_texts(&state).len()),
            "non-vacuity: the anchor row must still track the (poisoned) product's own length"
        );

        // Printing after the clear is an ordinary append on top — the sentinel
        // must survive THAT too, extended rather than rebuilt away.
        state.push_transcript_kind("after the clear", TranscriptKind::Story);
        wrap_render(&state, area);
        assert_eq!(
            cached_row_texts(&state).first().map(String::as_str),
            Some("SENTINEL"),
            "printing after the clear must EXTEND the poisoned rows, not rebuild them"
        );
        assert_eq!(
            cached_row_texts(&state).last().map(String::as_str),
            Some("after the clear"),
            "the new line must actually be there"
        );
    }

    /// **The equivalence matrix (SQ-1179): a repaired cache must be
    /// indistinguishable from a rebuilt one.**
    ///
    /// `wrap_lines_kinded_extend`'s wrap is a left-to-right scan that carries
    /// exactly one thing ACROSS lines — the open margin float (`FloatState`,
    /// this file's own doc comment on it) — and reads only its own line's
    /// text/kind/style/runs/paragraph-format/image plus that carried float
    /// otherwise (verified by reading the function body above: every other
    /// input it touches — `styles`, `runs`, `para`, `images` — is indexed by
    /// the CURRENT line alone). So a line's wrap can only ever depend on
    /// itself and what came before it, never on what comes after — which is
    /// the property a tail repair rests on: everything before the disturbed
    /// tail is provably unreachable from the edit and can be left exactly as
    /// cached.
    ///
    /// The matrix, for every combination of:
    ///   * width 40 and 80 (the repair's own wrap width, and whether the
    ///     baseline's float/hanging-indent/style-run content wraps
    ///     differently at each);
    ///   * transcript filter `Both`/`Story`/`Meta` — `Meta` hides the
    ///     trailing prompt the insert lands above, so the cache never held an
    ///     entry for it (`tail_visible == false`) — the OTHER branch of the
    ///     repair's pop-or-not decision from the other two filters;
    ///   * the screen cleared in the SAME edit batch as the insert or not —
    ///     (A) and (B) firing together, the realistic "erase_window then
    ///     print" shape;
    ///   * a single-line vs a multi-line insert (one `push_transcript_internal`
    ///     call whose text carries an embedded `\n`, one `Inserted { count: 2 }`);
    ///   * one insert vs an unbroken RUN of two — what `push_assist`'s
    ///     per-line loop actually does (a preamble line then an offer line,
    ///     each its own `Inserted`, chaining `TailInsertRun::min_at`).
    ///
    /// performs the SAME final edit two ways — INCREMENTAL (synced to a warm
    /// pre-insert cache, so the insert is a REPAIR) and REBUILT (the identical
    /// final content, rendered for the first time, so it is a Rebuild from
    /// line zero) — and asserts the two caches' entire comparable surface
    /// (every wrapped row, and the anchor/starts/live-band bookkeeping the
    /// draw path reads) is IDENTICAL. `insert-at-end`/`several-lines-up`
    /// are not in the matrix: every real caller (`push_transcript_internal`,
    /// `_styled`) inserts EXACTLY one place — immediately above the current
    /// last line — so that is the only shape a repair ever has to prove
    /// itself against; `wrap_key_plan_falls_back_to_rebuild_when_the_insert_is_not_at_the_cached_tail`
    /// below covers the "several lines up" guard directly instead.
    ///
    /// This is the falsification too. Shifting the popped-row/offset
    /// arithmetic in the render path's repair branch by one — truncating to
    /// `popped + 1` instead of `popped`, or seeding `carry` from `cache.carry`
    /// instead of `cache.tail_entry_carry` — was tried by hand while writing
    /// this case: both broke `wrap_product` equality on every filter/width
    /// combination that actually exercises the pop (i.e. every `filter` other
    /// than `Meta`), confirming the case can see the offset it exists to
    /// guard. Restored before committing.
    #[test]
    fn a_tail_insert_repair_lands_on_exactly_what_a_rebuild_would_have_produced() {
        use crate::render::wrap_cache::WrapPlan;

        // The baseline every variant starts from is SQ-1034's own equivalence
        // script (style runs, a hanging-indent Meta line, a left-margin image
        // float outrunning its text) plus a trailing Story prompt line, so the
        // repair's carried float/style state and its pop-the-old-tail branch
        // are both exercised for real rather than vacuously.
        let build_baseline = |state: &mut AppState, filter: TranscriptFilter, area: Rect| {
            state.transcript_filter = filter;
            drive_script(state, area, false);
            state.push_transcript_kind(">", TranscriptKind::Story);
        };

        for &width in &[40u16, 80u16] {
            for &filter in &[TranscriptFilter::Both, TranscriptFilter::Story, TranscriptFilter::Meta] {
                for &clear in &[false, true] {
                    for &multi in &[false, true] {
                        for &two in &[false, true] {
                            let area = Rect::new(0, 0, width, 16);
                            let insert_text = if multi { "one\ntwo" } else { "single" };
                            let label = format!(
                                "width={width} filter={filter:?} clear={clear} multi={multi} two={two}"
                            );

                            // INCREMENTAL: sync the cache to the pre-insert baseline
                            // first, so the insert below lands on a WARM cache and
                            // is a repair rather than this frame's first ever wrap.
                            let mut incremental = script_state();
                            build_baseline(&mut incremental, filter, area);
                            wrap_render(&incremental, area);
                            if clear {
                                incremental.mark_screen_clear();
                            }
                            incremental.push_transcript_internal(insert_text, TranscriptKind::Assist);
                            if two {
                                incremental.push_transcript_internal("second", TranscriptKind::Assist);
                            }
                            // Non-vacuity: this frame must actually take the
                            // REPAIR branch, or the comparison below proves
                            // nothing about it.
                            let plan = {
                                let cache = incremental.transcript_wrap.borrow();
                                let key = &cache.as_ref().expect("cache populated by the sync render").key;
                                key.plan(&incremental, key.shape.width)
                            };
                            assert!(
                                matches!(plan, WrapPlan::Repair { .. }),
                                "{label}: expected a Repair, got {plan:?}"
                            );
                            wrap_render(&incremental, area);

                            // REBUILT: push the IDENTICAL final content with no
                            // intermediate render at all, so the only render sees
                            // an empty cache and rebuilds from line zero.
                            let mut rebuilt = script_state();
                            build_baseline(&mut rebuilt, filter, area);
                            if clear {
                                rebuilt.mark_screen_clear();
                            }
                            rebuilt.push_transcript_internal(insert_text, TranscriptKind::Assist);
                            if two {
                                rebuilt.push_transcript_internal("second", TranscriptKind::Assist);
                            }
                            wrap_render(&rebuilt, area);

                            assert_eq!(
                                wrap_product(&incremental),
                                wrap_product(&rebuilt),
                                "{label}: rows diverged between repair and rebuild"
                            );
                            assert_eq!(
                                wrap_bookkeeping(&incremental),
                                wrap_bookkeeping(&rebuilt),
                                "{label}: bookkeeping diverged between repair and rebuild"
                            );
                        }
                    }
                }
            }
        }
    }

    /// A gap the matrix above cannot see: `drive_script`'s picture always
    /// fully closes two lines before the trailing prompt, so `cache.carry`
    /// (the float state AFTER the cache's last consumed line) and
    /// `cache.tail_entry_carry` (the float state BEFORE it, what a repair
    /// must actually seed the re-wrap with) are always both `None` there —
    /// indistinguishable, so a repair that read the wrong one would still
    /// pass every case in that matrix. This one leaves the float OPEN across
    /// the prompt line itself (one strip claimed, three left), so the two
    /// carries hold different `next_strip` values and a wrong read is a
    /// picture-strip mismatch, not silence. Reading `cache.carry` instead of
    /// `cache.tail_entry_carry` in the repair branch was tried by hand while
    /// writing this case (this test was what finally caught it — the main
    /// matrix above did not); restored before committing.
    #[test]
    fn a_tail_insert_repair_with_an_open_float_entering_the_prompt_lands_on_exactly_what_a_rebuild_would_have_produced() {
        use crate::render::wrap_cache::WrapPlan;

        let area = Rect::new(0, 0, 40, 16);

        let build = |state: &mut AppState| {
            state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
            state.push_transcript_kind("west of house", TranscriptKind::Story);
            state.push_transcript_image(script_image()); // 4 strips tall
            state.push_transcript_kind("beside one", TranscriptKind::Story); // claims 1 strip
            // No more prose before the prompt: the float still has strips left
            // when the trailing line is wrapped.
            state.push_transcript_kind(">", TranscriptKind::Story);
        };

        let mut incremental = script_state();
        build(&mut incremental);
        wrap_render(&incremental, area);
        // Non-vacuity: the prompt itself must actually be riding the float —
        // and the carry entering it must actually differ from the carry after
        // it — or this proves nothing about which one a repair reads.
        {
            let cache = incremental.transcript_wrap.borrow();
            let entry = cache.as_ref().expect("cache populated by the sync render");
            assert!(
                entry.tail_entry_carry.is_some() && entry.carry.is_some(),
                "the float must still be open both entering AND after the prompt line"
            );
            let last = entry.rows.last().expect("rows");
            assert!(last.float.is_some(), "the prompt must be riding the float's strip, not a plain row");
        }

        incremental.push_transcript_internal("an assist", TranscriptKind::Assist);
        let plan = {
            let cache = incremental.transcript_wrap.borrow();
            let key = &cache.as_ref().expect("cache").key;
            key.plan(&incremental, key.shape.width)
        };
        assert!(matches!(plan, WrapPlan::Repair { .. }), "expected a Repair, got {plan:?}");
        wrap_render(&incremental, area);

        let mut rebuilt = script_state();
        build(&mut rebuilt);
        rebuilt.push_transcript_internal("an assist", TranscriptKind::Assist);
        wrap_render(&rebuilt, area);

        assert_eq!(
            wrap_product(&incremental),
            wrap_product(&rebuilt),
            "rows diverged between repair and rebuild with an open float entering the prompt"
        );
        assert_eq!(wrap_bookkeeping(&incremental), wrap_bookkeeping(&rebuilt));
    }

    /// The guard `a_tail_insert_repair_lands_on_exactly_what_a_rebuild_would_have_produced`
    /// leaves untested because no real caller can reach it: an `Inserted` run
    /// whose earliest `at` sits BEFORE this cache's own synced tail — the
    /// "several lines up" case CLAUDE.md's own review named — must never be
    /// offered a `Repair`, because the cache cannot prove content before its
    /// own tail is untouched. Built the way
    /// `an_in_place_edit_of_the_last_line_is_caught_even_when_it_is_misclassified`
    /// (`render::wrap_cache`'s own test) builds its misclassification: reach
    /// past the mutator and set the state directly, since no mutator in this
    /// codebase produces the shape being guarded against.
    ///
    /// Asserted as "not a `Repair`" rather than "a `Rebuild`": with
    /// `transcript_edits` left untouched (as it genuinely would be by a plain
    /// append), the honest answer once a repair is correctly declined is
    /// `Append` — nothing else claims the cache's own prefix moved. What this
    /// case exists to catch is the min-at guard silently offering a `Repair`
    /// it cannot back up, not which of the other two safe answers follows.
    #[test]
    fn wrap_key_plan_never_offers_a_repair_when_the_insert_predates_the_cached_tail() {
        use crate::render::wrap_cache::{WrapKey, WrapPlan};

        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.push_transcript_kind("first\nsecond\nthird", TranscriptKind::Story);
        let key = WrapKey::of(&state, 40);
        assert_eq!(key.plan(&state, 40), WrapPlan::Reuse, "nothing moved yet");

        // A plain append (so `transcript_edits`/the tail fingerprint stay
        // exactly what a legitimate append leaves them), plus a FABRICATED
        // run claiming an insert at raw index 0 — well before
        // `key.content.len - 1 == 2` — which `push_transcript_internal` can
        // never produce (it always targets exactly the cache's own tail).
        state.push_transcript_kind("fourth", TranscriptKind::Story);
        state.transcript_tail_insert.set(Some(crate::state::TailInsertRun { since_edits: state.transcript_edits, min_at: 0 }));

        assert!(
            !matches!(key.plan(&state, 40), WrapPlan::Repair { .. }),
            "an insert claiming a position before the cached tail must never be repaired through: {:?}",
            key.plan(&state, 40),
        );
    }

    #[test]
    fn a_restore_into_a_different_size_and_backend_rebuilds_rather_than_appending() {
        // The cell path's half of `render::wrap_cache`'s restore case — see its
        // `restore_transcript` for the four production sites this mirrors, and
        // CLAUDE.md for why a restore is asserted one move LATER: on the frame it
        // lands, a cache that quietly appended onto the pre-restore scrollback
        // still shows the archive's own rows correctly.
        let before = Rect::new(0, 0, 52, 12);
        let after = Rect::new(0, 0, 30, 12);
        let archived: &[&str] = &[
            "West of House",
            "You are standing in an open field west of a white house, with a boarded front door.",
            "There is a small mailbox here.",
        ];
        let moved = "You open the mailbox, revealing a small leaflet.";

        let restore = |state: &mut AppState| {
            state.transcript = archived.iter().map(|s| s.to_string()).collect();
            state.clear_anchor = None;
            state.transcript_kinds = vec![TranscriptKind::Story; archived.len()];
            state.transcript_runs = vec![Vec::new(); archived.len()];
            state.transcript_para = vec![ParaFmt::default(); archived.len()];
            state.reset_transcript_sidecars();
        };

        let mut live = script_state();
        drive_script(&mut live, before, true);
        restore(&mut live);
        // A different pane and a different graphics backend.
        live.game_picker = Some(crate::render::graphics::kitty_picker(8, 16));
        wrap_render(&live, after);
        // PERTURB, then assert.
        live.push_transcript_kind(moved, TranscriptKind::Story);
        wrap_render(&live, after);

        let mut fresh = script_state();
        fresh.game_picker = Some(crate::render::graphics::kitty_picker(8, 16));
        restore(&mut fresh);
        fresh.push_transcript_kind(moved, TranscriptKind::Story);
        wrap_render(&fresh, after);

        let want = wrap_product(&fresh);
        assert!(
            want.iter().any(|r| r.contains("leaflet")),
            "non-vacuity: the move after the restore must be on screen: {want:#?}"
        );
        assert!(
            !want.iter().any(|r| r.contains("beside one")),
            "non-vacuity: the pre-restore scrollback must be GONE: {want:#?}"
        );
        assert_eq!(
            wrap_product(&live),
            want,
            "a restore then a move must leave the archive's rows, not the old ones"
        );
        assert_eq!(wrap_bookkeeping(&live), wrap_bookkeeping(&fresh));
    }

    #[test]
    fn wrap_cache_rebuilds_when_an_already_wrapped_line_is_edited() {
        let mut state = AppState::default();
        state.push_transcript_kind("hello world", TranscriptKind::Story);
        let area = Rect::new(0, 0, 20, 8);
        wrap_render(&state, area);
        poison_wrap_cache(&state);
        // The inline-prompt echo: the LAST line grows in place. Nothing was
        // appended, so a length co-key cannot see it — `TranscriptEdit::Rewrote`
        // and the tail fingerprint both can.
        state.append_to_last_transcript_line("!");
        wrap_render(&state, area);
        assert_eq!(
            cached_first_text(&state),
            "hello world!",
            "an in-place edit of a wrapped line must rebuild"
        );
    }

    #[test]
    fn wrap_cache_invalidates_on_width_change() {
        let mut state = AppState::default();
        state.push_transcript_kind("hello world", TranscriptKind::Story);
        wrap_render(&state, Rect::new(0, 0, 20, 8));
        poison_wrap_cache(&state);
        wrap_render(&state, Rect::new(0, 0, 30, 8)); // different wrap width
        assert_ne!(cached_first_text(&state), "SENTINEL", "width change must re-wrap");
    }

    #[test]
    fn wrap_cache_invalidates_on_filter_change() {
        let mut state = AppState::default();
        state.push_transcript_kind("hello world", TranscriptKind::Story);
        let area = Rect::new(0, 0, 20, 8);
        wrap_render(&state, area);
        poison_wrap_cache(&state);
        state.transcript_filter = TranscriptFilter::Meta; // hides the Story line
        wrap_render(&state, area);
        assert_ne!(cached_first_text(&state), "SENTINEL", "filter change must re-wrap");
    }

    #[test]
    fn wrap_cache_invalidates_on_anchor_change() {
        let mut state = AppState::default();
        state.push_transcript_kind("hello world", TranscriptKind::Story);
        let area = Rect::new(0, 0, 20, 8);
        wrap_render(&state, area);
        poison_wrap_cache(&state);
        state.clear_anchor = Some(0); // screen-clear boundary moved
        wrap_render(&state, area);
        assert_ne!(cached_first_text(&state), "SENTINEL", "anchor change must re-wrap");
    }

    #[test]
    fn wrap_cache_invalidates_on_same_length_content_replacement() {
        // A rewind/restore can replace the transcript with a DIFFERENT content of
        // the SAME length — a length check alone would serve stale rows. The
        // generation counter (bumped by reset_transcript_sidecars) catches it.
        let mut state = AppState::default();
        state.push_transcript_kind("AAAAA", TranscriptKind::Story);
        let area = Rect::new(0, 0, 20, 8);
        wrap_render(&state, area);
        poison_wrap_cache(&state);
        let gen_before = state.transcript_gen;
        state.transcript = vec!["BBBBB".to_string()];
        state.transcript_kinds = vec![TranscriptKind::Story];
        state.transcript_runs = vec![Vec::new()];
        state.reset_transcript_sidecars(); // bumps gen; length unchanged (1)
        assert_ne!(state.transcript_gen, gen_before, "reset must bump the generation");
        assert_eq!(state.transcript.len(), 1, "same length as before");
        wrap_render(&state, area);
        assert_eq!(cached_first_text(&state), "BBBBB", "same-length replacement must re-wrap to new content");
    }

    #[test]
    fn window_wrapped_rows_windows_and_top_anchors() {
        let rows: Vec<WrappedRow> = (0..6)
            .map(|i| WrappedRow {
                text: format!("R{i}"),
                kind: TranscriptKind::Story,
                style: Style::default(),
                runs: Vec::new(),
                band: None,
                float: None,
            })
            .collect();
        // No anchor, scroll 0 → newest 3 at the bottom.
        let (vis, total, first) = window_wrapped_rows(&rows, None, 3, 0);
        assert_eq!(total, 6);
        assert_eq!(first, 3);
        assert_eq!(vis.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), vec!["R3", "R4", "R5"]);
        // Anchor at row 4 with the post-anchor content (2 rows) fitting the 3-row
        // viewport → pin from the anchor (top-anchored), fewer than `rows` returned.
        let (vis2, _t, first2) = window_wrapped_rows(&rows, Some(4), 3, 0);
        assert_eq!(first2, 4);
        assert_eq!(vis2.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), vec!["R4", "R5"]);
        // While scrolled (scroll != 0) the anchor does not apply.
        let (_v3, _t3, first3) = window_wrapped_rows(&rows, Some(4), 3, 1);
        assert_eq!(first3, 2);
    }
}
