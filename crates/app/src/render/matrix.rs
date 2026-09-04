//! The matrix view: a layer drawn as a direction TABLE rather than a map (SQ-0666).
//!
//! One row per room, one column per direction — all twelve, always. Inside a maze the compass
//! layout is wrong about most of what it draws (29 of 47 edges in the reference map are flagged
//! distorted), because a maze is not a place with geometry; it is a set of facts of the form "west
//! from here goes to that one, and the way back is north". This draws exactly those facts.
//!
//! The classification lives in `mapper::matrix`, which knows nothing about terminals. Everything
//! here is presentation: glyphs, widths, styles, hit rects, and the responsive degradation.
//!
//! It is also, incidentally, the only map view a screen reader can read: a table linearises.

use std::collections::BTreeSet;

use mapper::direction::{short_label, Direction};
use mapper::graph::{MapGraph, RoomId};
use mapper::layer::LayerId;
use mapper::matrix::{self, Matrix, MatrixCell, MATRIX_DIRS};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::render::draw_str_clipped;
use crate::state::AppState;

/// The MINIMUM label-column reservation, including the one-cell `▸`/`⇲` marker gutter — the floor
/// the direction-column ladder is sized against (SQ-1247).
///
/// How many of the twelve columns fit, and at what cell width, is decided as if the label column
/// were exactly this wide (see [`density`]), so a label column that later grows past it (see
/// [`MatrixLayout::label_w`]) can never steal width from the direction table — "never shrink the
/// direction columns" is enforced by never letting them see the grown number at all. The actual
/// drawn label width is `MatrixLayout::label_w`, which grows to whatever the shown direction
/// columns leave behind, capped at the longest room name, and can be wider OR narrower than this
/// constant depending on what the data actually needs. Ten columns of name is enough for
/// `"Maze 11"` and for most short room names; this is only ever the floor.
pub const LABEL_W: u16 = 11;

/// Minimum width of a direction column: two for the widest header (`NE`) plus a separating space.
const MIN_CELL_W: u16 = 3;

/// How much of each cell is spelled out — the responsive ladder (see [`density`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Density {
    /// Cells carry the return direction: `→5⇠w`. The row is self-contained.
    Full,
    /// The `⇠x` suffix is dropped: `→5`. The return is still on the destination's own row and in
    /// its room-info card, so nothing is lost — only a lookup is added.
    Compact,
    /// Compact cells AND horizontal scrolling, with the label column pinned. The last resort:
    /// once the table cannot be read across, it has to be read a piece at a time.
    Scroll,
}

/// The chosen density and the column width it implies, for a given pane width.
///
/// Computed, never configured. The player is not in a position to know that their pane is four
/// columns short of the full form, and a setting would only let them get it wrong.
pub fn density(m: &Matrix, width: u16) -> (Density, u16) {
    let full = cell_width(m, true);
    if LABEL_W + full * 12 <= width {
        return (Density::Full, full);
    }
    let compact = cell_width(m, false);
    if LABEL_W + compact * 12 <= width {
        return (Density::Compact, compact);
    }
    (Density::Scroll, compact)
}

/// The column width this particular table needs: the widest cell it actually contains, plus a
/// separating space. Measured from the data, so a layer whose destinations are all one digit gets
/// a tighter table than one numbering to 11.
fn cell_width(m: &Matrix, with_return: bool) -> u16 {
    let mut w = MIN_CELL_W - 1;
    for row in &m.rows {
        for cell in row.cells {
            w = w.max(cell_text(m, &cell, with_return).chars().count() as u16);
        }
    }
    w + 1
}

/// One cell's text.
///
/// | glyph  | means                                          |
/// |--------|------------------------------------------------|
/// | `⇄4`   | reciprocal — the compass inverse returns        |
/// | `→5⇠w` | goes to 5, and W comes back                    |
/// | `⇢9`   | one-way — no return known                      |
/// | `↩`    | self-loop — this direction leads back here     |
/// | `⇱out` | leaves the layer; the destination is footnoted |
/// | `×`    | tried, and there is no path that way           |
/// | `·`    | untried — the exploration frontier             |
/// | `?`    | tried; the story sends a different room each time (SQ-1257) |
pub fn cell_text(m: &Matrix, cell: &MatrixCell, with_return: bool) -> String {
    let tag = |id: RoomId| m.labels.tag_of(id).to_string();
    match cell {
        MatrixCell::Reciprocal { dest } => format!("⇄{}", tag(*dest)),
        MatrixCell::ReturnBy { dest, back } => {
            if with_return {
                format!("→{}⇠{}", tag(*dest), short_label(*back))
            } else {
                format!("→{}", tag(*dest))
            }
        }
        MatrixCell::OneWay { dest } => format!("⇢{}", tag(*dest)),
        MatrixCell::SelfLoop => "↩".to_string(),
        // The compact form drops `out` for the same reason it drops `⇠x`: the word is a nicety,
        // and one crossing cell was otherwise widening all twelve columns by one for the whole
        // table. The footnote below says where it goes either way.
        MatrixCell::LeavesLayer { .. } => {
            if with_return { "⇱out".to_string() } else { "⇱".to_string() }
        }
        MatrixCell::Probed => "×".to_string(),
        MatrixCell::Untried => "·".to_string(),
        // A bare `?` when nothing is recorded yet; a superscript count of recorded destinations
        // once there are some (SQ-1261) — `?²` for two rooms this direction has actually landed
        // in, agreeing with the room card's "destination varies: A, B" and the same glyph table
        // the map box's stub uses.
        MatrixCell::Random { destinations } => format!("?{}", super::superscript_count(*destinations)),
    }
}

/// A footnote under the table: its marker and the line it explains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Footnote {
    pub marker: String,
    pub text: String,
}

/// Superscript digits, so a footnote marker costs one column and cannot be mistaken for part of
/// the room's name.
const SUPERSCRIPTS: [char; 9] = ['¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];

fn marker(n: usize) -> String {
    SUPERSCRIPTS.get(n).copied().map(String::from).unwrap_or_else(|| format!("({})", n + 1))
}

/// The label column's text for a row, plus the footnote it needs (if any).
///
/// A name that does not fit is cut at its first comma before it is cut mid-word — IF room names
/// nearly always carry their qualifier after a comma, so `"Dead End, near Vending Machine"`
/// becomes `"Dead End"` rather than `"Dead End,…"`. Either way the full name goes in a footnote:
/// an abbreviation that cannot be resolved is worse than no abbreviation.
fn label_cell(full: &str, width: usize) -> (String, Option<String>) {
    if full.chars().count() <= width {
        return (full.to_string(), None);
    }
    let head = full.split(',').next().unwrap_or(full).trim();
    let room = width.saturating_sub(1); // one column for the marker
    let short: String = if head.chars().count() <= room {
        head.to_string()
    } else {
        head.chars().take(room).collect()
    };
    (short, Some(full.to_string()))
}

/// Everything the matrix needs to draw, resolved for one pane width.
pub struct MatrixLayout {
    pub matrix: Matrix,
    pub density: Density,
    pub cell_w: u16,
    /// How many of the twelve direction columns are actually drawn — all of them, unless
    /// [`Density::Scroll`] has to drop some off the left. Computed against [`LABEL_W`], the
    /// minimum reservation, so it never shrinks because [`Self::label_w`] later grew (SQ-1247).
    pub shown_cols: usize,
    /// The label column's ACTUAL drawn width this frame (SQ-1247): whatever `shown_cols` columns
    /// of direction table leave behind, capped at the longest room name (plus its one-cell marker
    /// gutter) so it never claims width no name needs. Can be wider OR narrower than [`LABEL_W`].
    pub label_w: u16,
    pub footnotes: Vec<Footnote>,
    /// Per-row label text and the footnote marker it carries (empty when it needs none).
    pub labels: Vec<(String, String)>,
    /// Rooms that are the destination of an inbound border edge — where the `⇲` row marker goes.
    pub entry_rooms: BTreeSet<RoomId>,
}

/// Resolve the table for a pane `width`: density, column width, abbreviated labels and footnotes.
pub fn layout(graph: &MapGraph, layer: LayerId, width: u16) -> MatrixLayout {
    let m = matrix::build(graph, layer);
    let (density, cell_w) = density(&m, width);

    // How many direction columns are shown, exactly as before (SQ-0666): the floor-based ladder
    // against `LABEL_W`, never the grown `label_w` below — that is what keeps a wide label column
    // from ever costing the direction table a column (SQ-1247).
    let shown_cols = ((width.saturating_sub(LABEL_W)) / cell_w).min(12) as usize;

    // SQ-1247: the label column grows to whatever the shown direction columns leave behind,
    // capped at the longest room name plus its one-cell marker gutter — so it never claims width
    // no name actually needs. When every name's length is within that cap, `label_cell` below
    // finds nothing left to truncate, and the whole per-row footnote-and-superscript machinery
    // stays silent for every row.
    let longest_name = m.rows.iter().map(|r| r.label.chars().count() as u16).max().unwrap_or(0);
    let cap = longest_name.saturating_add(1); // one column for the ▸/⇲ marker
    let avail = width.saturating_sub(cell_w * shown_cols as u16);
    let label_w = avail.min(cap).max(1);

    let mut footnotes: Vec<Footnote> = Vec::new();
    let mut labels = Vec::with_capacity(m.rows.len());
    for row in &m.rows {
        let (text, long) = label_cell(&row.label, label_w.saturating_sub(1) as usize);
        let mark = match long {
            Some(full) => {
                let mk = marker(footnotes.len());
                footnotes.push(Footnote { marker: mk.clone(), text: full });
                mk
            }
            None => String::new(),
        };
        labels.push((text, mark));
    }

    // `⇱out` cells name a room with no row here, so the cell cannot print its destination. One
    // footnote per distinct crossing, in row-then-column order.
    for row in &m.rows {
        for (i, cell) in row.cells.iter().enumerate() {
            let MatrixCell::LeavesLayer { dest } = cell else { continue };
            let name =
                graph.room(*dest).map(|r| r.label().to_string()).unwrap_or_else(|| format!("#{dest}"));
            let text = format!(
                "⇱out: {} from {} → {}",
                short_label(MATRIX_DIRS[i]).to_uppercase(),
                row.tag,
                name
            );
            if !footnotes.iter().any(|f| f.text == text) {
                footnotes.push(Footnote { marker: String::new(), text });
            }
        }
    }

    // `⇲ in:` lines are the mirror of `⇱out`: every border edge that ENTERS the layer from
    // outside it, one line per edge, in the same deterministic order `inbound_border_edges`
    // already sorted them in — so the block is stable across a save/load round trip whatever
    // order the edges were minted in, exactly like the `⇱out` lines above.
    let mut entry_rooms: BTreeSet<RoomId> = BTreeSet::new();
    for (origin, dir, dest) in matrix::inbound_border_edges(graph, layer) {
        entry_rooms.insert(dest);
        let origin_name =
            graph.room(origin).map(|r| r.label().to_string()).unwrap_or_else(|| format!("#{origin}"));
        let dest_label = m.labels.row_of(dest).to_string();
        let text =
            format!("⇲ in:  {origin_name} —{}→ {dest_label}", short_label(dir).to_uppercase());
        footnotes.push(Footnote { marker: String::new(), text });
    }

    MatrixLayout { matrix: m, density, cell_w, shown_cols, label_w, footnotes, labels, entry_rooms }
}

/// Rows of chrome the table spends before and after its data: header, rule, rule.
const HEADER_ROWS: u16 = 2;
const FOOTER_RULE_ROWS: u16 = 1;

/// How many data rows fit in `area`, given the footnotes that must also fit.
pub fn visible_rows(area: Rect, footnotes: usize) -> usize {
    let chrome = HEADER_ROWS + FOOTER_RULE_ROWS + footnotes.min(4) as u16;
    area.height.saturating_sub(chrome) as usize
}

/// Draw the matrix into `area`, returning click targets as `(room, rect)` pairs.
///
/// The rects go straight into the same `room_rects` list the drawn view publishes, so a click on a
/// row — or on a cell that NAMES a room — reaches the existing `ShowRoomInfo` path and selects it.
/// That is what makes "clicking a destination cell jumps selection to that room" a one-line
/// consequence rather than a second input pathway.
pub fn render_matrix(
    graph: &MapGraph,
    layer: LayerId,
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
) -> Vec<(RoomId, Rect)> {
    let mut hits = Vec::new();
    if area.width < LABEL_W + MIN_CELL_W || area.height < HEADER_ROWS + 1 {
        return hits;
    }
    let ml = layout(graph, layer, area.width);
    let m = &ml.matrix;

    let header_style = state.colors.theme.get("map.matrix.header").style;
    let base_style = state.colors.theme.get("map.room").style;
    let here_style = state.colors.theme.get("map.matrix.row:here").style;
    let selected_style = state.colors.theme.get("map.matrix.row:selected").style;
    let entrance_style = state.colors.theme.get("map.matrix.cell:entrance").style;
    let path_style = state.colors.theme.get("map.matrix.cell:path").style;
    let frontier_style = state.colors.theme.get("map.matrix.cell:frontier").style;
    let random_style = state.colors.theme.get("map.matrix.cell:random").style;
    let footnote_style = state.colors.theme.get("map.matrix.footnote").style;
    let trail_style = state.colors.theme.get("map.trail").style;

    // The columns actually shown — `ml.shown_cols`, resolved in `layout()` against the MINIMUM
    // label reservation so the grown `ml.label_w` below can never cost the direction table a
    // column (SQ-1247). Horizontal scroll drops whole columns off the LEFT while the label column
    // stays put — a half-column would be unreadable and a floating label column would lose the
    // reader entirely.
    let shown = ml.shown_cols;
    let first_col = if ml.density == Density::Scroll {
        (state.matrix_scroll.0 as usize).min(12usize.saturating_sub(shown))
    } else {
        0
    };

    // ── Header + rule ────────────────────────────────────────────────────────
    let table_w = ml.label_w + ml.cell_w * shown as u16;
    for (slot, i) in (first_col..first_col + shown).enumerate() {
        let text = short_label(MATRIX_DIRS[i]).to_uppercase();
        let x =
            area.x + ml.label_w + ml.cell_w * slot as u16 + ml.cell_w - text.chars().count() as u16;
        draw_str_clipped(buf, x, area.y, &text, header_style, area);
    }
    let rule: String = "─".repeat(table_w.min(area.width) as usize);
    draw_str_clipped(buf, area.x, area.y + 1, &rule, header_style, area);

    // ── Rows ─────────────────────────────────────────────────────────────────
    let rows_fit = visible_rows(area, ml.footnotes.len());
    let first_row = (state.matrix_scroll.1 as usize).min(m.rows.len().saturating_sub(rows_fit.max(1)));
    let selected = state.selected_room;
    // Bolding the cells that ARRIVE at the selected room answers "how do I get back here" — the
    // one question the table cannot answer by reading across a row.
    let entrances = selected.map(|id| matrix::entrances(graph, id)).unwrap_or_default();
    // The route to the selected room (SQ-0693), as the cells it is walked THROUGH: row = the room
    // you are standing in, column = the direction you leave by. One cell per step, so the highlight
    // reads down the table as walking instructions and never overwrites the glyph that says what
    // kind of passage each step is.
    //
    // Steps whose room has no row here — the search deliberately crosses layers — simply do not
    // draw. The step that LEAVES this layer does have a row, so its `⇱out` cell lights up and marks
    // the departure; that cell already footnotes where it goes.
    let path: Vec<(RoomId, Direction)> = state.room_path.iter().map(|s| (s.room, s.dir)).collect();
    // The walked trail is maze furniture: on an ordinary layer a breadcrumb is noise, because the
    // drawn map already shows you where you came from.
    let maze = graph.layer_is_maze(layer);
    // The fade is one selector plus DIM on the older half, rather than eight registry rows for
    // eight steps of the same idea: what the reader needs is "recent" vs "a while ago", and a
    // themer who wants a different trail colour should have one knob to turn, not eight.
    let trail_at = |room: RoomId| -> Option<Style> {
        if !maze {
            return None;
        }
        let age = state.trail_age(room)?;
        Some(if age * 2 >= crate::state::MAP_TRAIL_LEN {
            trail_style.add_modifier(ratatui::style::Modifier::DIM)
        } else {
            trail_style
        })
    };

    for (slot, row) in m.rows.iter().skip(first_row).take(rows_fit).enumerate() {
        let y = area.y + HEADER_ROWS + slot as u16;
        let is_here = m.here == Some(row.room);
        let is_selected = selected == Some(row.room);
        let row_style = match (is_selected, is_here) {
            (true, _) => selected_style,
            (_, true) => here_style,
            _ => trail_at(row.room).unwrap_or(base_style),
        };

        let (text, mark) = &ml.labels[first_row + slot];
        // `▸` (you are here) wins over `⇲` (a way in) when a room is both: the entrance fact is
        // still true and still lives in the footnote, but the row only has one marker column and
        // "you are standing here" is the more urgent thing to say about it.
        let is_entry = ml.entry_rooms.contains(&row.room);
        if is_here || !is_entry {
            let lead = if is_here { "▸" } else { " " };
            let label = format!("{lead}{text}{mark}");
            draw_str_clipped(buf, area.x, y, &label, row_style, area);
        } else {
            draw_str_clipped(buf, area.x, y, "⇲", entrance_style, area);
            let rest = format!("{text}{mark}");
            draw_str_clipped(buf, area.x + 1, y, &rest, row_style, area);
        }
        hits.push((row.room, Rect::new(area.x, y, ml.label_w.min(area.width), 1)));

        for (col_slot, i) in (first_col..first_col + shown).enumerate() {
            let cell = row.cells[i];
            let text = cell_text(m, &cell, ml.density == Density::Full);
            let w = text.chars().count() as u16;
            let x = area.x + ml.label_w + ml.cell_w * col_slot as u16 + ml.cell_w - w;
            // An entrance to the selected room is bolded wherever it appears — style, not a
            // glyph, so the cell keeps saying exactly one thing. A step of the route wins over
            // that: the LAST step is necessarily an entrance too, and the answer the player just
            // asked for beats the standing cross-reference.
            let style = if path.contains(&(row.room, MATRIX_DIRS[i])) {
                path_style
            } else if entrances.contains(&(row.room, MATRIX_DIRS[i])) {
                entrance_style
            } else if cell.is_frontier() {
                frontier_style
            } else if matches!(cell, MatrixCell::Random { .. }) {
                random_style
            } else {
                row_style
            };
            draw_str_clipped(buf, x, y, &text, style, area);
            if let Some(dest) = cell.dest() {
                if m.index_of(dest).is_some() {
                    hits.push((dest, Rect::new(x, y, w, 1)));
                }
            }
        }
    }

    // ── Closing rule + footnotes ─────────────────────────────────────────────
    let shown_rows = m.rows.len().saturating_sub(first_row).min(rows_fit) as u16;
    let mut y = area.y + HEADER_ROWS + shown_rows;
    if y < area.bottom() {
        draw_str_clipped(buf, area.x, y, &rule, header_style, area);
        y += 1;
    }
    for f in &ml.footnotes {
        if y >= area.bottom() {
            break;
        }
        let line = if f.marker.is_empty() {
            f.text.clone()
        } else {
            format!("{} {}", f.marker, f.text)
        };
        draw_str_clipped(buf, area.x, y, &line, footnote_style, area);
        y += 1;
    }
    hits
}

/// Draw the hover tooltip for whichever matrix room the pointer is on, if any (SQ-1246).
///
/// `state.matrix_hover` is what `main.rs`'s `matrix_update_hover` last resolved from a `Moved`
/// event — a row label or a destination cell, paired with the exact rect the pointer was found
/// under. Either way it names a room: a row label is truncated on screen whenever the name did
/// not fit the label column, and a destination cell never carries a name at all, only a two- or
/// three-letter tag. The full name is read from the same [`MatrixLabels`](matrix::MatrixLabels)
/// the table itself printed from, so a tooltip and the table's own footnote (when a name was too
/// long to fit) can never disagree.
///
/// Reuses the shared floating-box renderer the border controls already draw their hints with
/// (`tooltip::draw_tip`) rather than a second tooltip mechanism, so styling (`tooltip.*`) and
/// placement (beside/below the anchor, clamped to `area`) are identical to theirs.
///
/// Returns `None` — and paints nothing — while a modal overlay owns the pointer, or when nothing
/// is hovered, or when the hovered room has no name to show.
pub fn draw_hover_tip(
    graph: &MapGraph,
    layer: LayerId,
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
) -> Option<Rect> {
    if state.any_modal_overlay_open() {
        return None;
    }
    let (room, rect) = state.matrix_hover?;
    let m = matrix::build(graph, layer);
    let name = m.labels.row_of(room);
    if name.is_empty() {
        return None;
    }
    let anchor_col = rect.x + rect.width / 2;
    super::tooltip::draw_tip(
        buf,
        area,
        anchor_col,
        rect.y,
        &[name.to_string()],
        &state.colors.theme,
        &state.symbols,
    )
}

/// Move the selection `delta` rows through the matrix, scrolling to keep it visible.
///
/// Returns the newly selected room, or `None` when the layer has no rows. Selection lands on the
/// room you are standing in when nothing was selected, because that is where a player who just
/// pressed a key is looking.
pub fn step_selection(
    graph: &MapGraph,
    layer: LayerId,
    current: Option<RoomId>,
    delta: i32,
) -> Option<RoomId> {
    let m = matrix::build(graph, layer);
    if m.rows.is_empty() {
        return None;
    }
    // With nothing selected the FIRST press lands on the room you are standing in and moves no
    // further — otherwise the first arrow both creates a selection and immediately steps it past
    // the row the player was looking at.
    let Some(at) = current.and_then(|id| m.index_of(id)) else {
        let start = m.here.and_then(|id| m.index_of(id)).unwrap_or(0);
        return Some(m.rows[start].room);
    };
    // Saturating, so Home/End can be spelled `i32::MIN`/`i32::MAX` without overflowing.
    let next = (at as i32).saturating_add(delta).clamp(0, m.rows.len() as i32 - 1);
    Some(m.rows[next as usize].room)
}

/// The row-scroll offset that keeps `room` on screen, given the current offset.
pub fn scroll_to_show(graph: &MapGraph, layer: LayerId, room: RoomId, area: Rect, at: u16) -> u16 {
    let ml = layout(graph, layer, area.width);
    let Some(idx) = ml.matrix.index_of(room) else { return at };
    let fit = visible_rows(area, ml.footnotes.len()).max(1);
    let at = at as usize;
    if idx < at {
        idx as u16
    } else if idx >= at + fit {
        (idx + 1 - fit) as u16
    } else {
        at as u16
    }
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use mapper::direction::Direction;
    use mapper::graph::MapGraph;
    use mapper::layer::MAIN_LAYER;

    fn tiny() -> MapGraph {
        let mut g = MapGraph::new();
        for (id, n) in [(1u16, "Maze"), (2, "Maze"), (3, "Dead End, near Vending Machine")] {
            g.upsert_room(id.into(), n.into());
        }
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::W, 1);
        g.add_edge(2, Direction::S, 3);
        g.add_edge(3, Direction::N, 2);
        g.mark_tried(3, Direction::E);
        g.set_current(1);
        g
    }

    #[test]
    fn cells_spell_the_vocabulary_the_design_settled_on() {
        let g = tiny();
        let m = matrix::build(&g, MAIN_LAYER);
        let t = |room, dir, full| cell_text(&m, &matrix::classify(&g, room, dir), full);
        assert_eq!(t(1, Direction::N, true), "→2⇠w", "goes to 2; west comes back");
        assert_eq!(t(1, Direction::N, false), "→2", "compact drops the return, not the destination");
        assert_eq!(t(2, Direction::S, true), "⇄DE", "reciprocal, pointing at an initials tag");
        assert_eq!(t(3, Direction::E, true), "×", "tried, no path");
        assert_eq!(t(3, Direction::W, true), "·", "untried");
        assert_eq!(t(1, Direction::S, true), "·");
    }

    /// SQ-1261: a `?` cell is bare with no recorded destinations, and carries a superscript
    /// count once some are recorded — the same glyph table the map box's stub and the alias
    /// marker use.
    #[test]
    fn random_cell_carries_a_superscript_destination_count() {
        let mut g = tiny();
        g.mark_random_exit(1, Direction::E);
        let m = matrix::build(&g, MAIN_LAYER);
        assert_eq!(
            cell_text(&m, &matrix::classify(&g, 1, Direction::E), true),
            "?",
            "no destinations recorded yet"
        );

        g.note_random_destination(1, Direction::E, 2);
        g.note_random_destination(1, Direction::E, 3);
        let m = matrix::build(&g, MAIN_LAYER);
        assert_eq!(
            cell_text(&m, &matrix::classify(&g, 1, Direction::E), true),
            "?²",
            "two distinct destinations recorded"
        );
    }

    /// A long name is cut at its comma, marked, and spelled out below the table — never left as
    /// an abbreviation the reader cannot resolve.
    #[test]
    fn a_long_name_is_abbreviated_and_footnoted() {
        let (text, long) = label_cell("Dead End, near Vending Machine", 10);
        assert_eq!(text, "Dead End", "cut at the comma, not mid-word");
        assert_eq!(long.as_deref(), Some("Dead End, near Vending Machine"));
        assert_eq!(label_cell("Maze 3", 10), ("Maze 3".into(), None), "a name that fits is untouched");
    }

    /// The degradation ladder: spell the return out when there is room, drop it when there is not,
    /// and only scroll when even that will not fit.
    #[test]
    fn density_degrades_before_it_scrolls() {
        let g = tiny();
        let m = matrix::build(&g, MAIN_LAYER);
        let full = cell_width(&m, true);
        let compact = cell_width(&m, false);
        assert!(compact < full, "dropping `⇠x` really does buy width");

        assert_eq!(density(&m, LABEL_W + full * 12).0, Density::Full, "exactly enough is enough");
        assert_eq!(density(&m, LABEL_W + full * 12 - 1).0, Density::Compact, "one short → compact");
        assert_eq!(density(&m, LABEL_W + compact * 12).0, Density::Compact);
        assert_eq!(
            density(&m, LABEL_W + compact * 12 - 1).0,
            Density::Scroll,
            "scrolling is the last resort, not the first"
        );
    }

    /// SQ-1247: with no slack left after the direction columns — the exact width at which
    /// [`density_degrades_before_it_scrolls`] above finds `Density::Compact`, one row short of
    /// `Full` — the label column has nothing to grow into and must stay at today's fixed
    /// [`LABEL_W`], byte for byte: same text, same footnote-marker presence, for every row.
    #[test]
    fn a_narrow_pane_keeps_todays_fixed_label_width_byte_for_byte() {
        let g = tiny();
        let m = matrix::build(&g, MAIN_LAYER);
        let compact = cell_width(&m, false);
        let width = LABEL_W + compact * 12; // Density::Compact, no width left over for the label
        let ml = layout(&g, MAIN_LAYER, width);
        assert_eq!(ml.label_w, LABEL_W, "no slack: the column must stay at today's fixed width");

        for row in &m.rows {
            let (want_text, want_long) = label_cell(&row.label, LABEL_W as usize - 1);
            let idx = ml.matrix.index_of(row.room).expect("every row indexes itself");
            let (text, mark) = &ml.labels[idx];
            assert_eq!(*text, want_text, "room {}: today's label text must be unchanged", row.room);
            assert_eq!(
                !mark.is_empty(),
                want_long.is_some(),
                "room {}: today's footnote-marker presence must be unchanged",
                row.room
            );
        }
    }

    /// SQ-1247's headline: given room to spare, the label column grows past `LABEL_W` — but only
    /// as far as the longest name actually needs, never further. A name that would have been
    /// truncated under yesterday's fixed 11 now fits in full, and with every name fitting, nothing
    /// gets a superscript and no truncation footnote is added.
    ///
    /// Falsified by reverting `layout`'s `label_w` to the constant `LABEL_W`: with that revert,
    /// "Medium Length Room" (18 chars, fits comfortably once the column is allowed to grow) is cut
    /// down to 10 and footnoted instead — the very shrinkage this feature removes.
    #[test]
    fn a_wide_pane_with_short_names_grows_the_column_and_drops_the_footnotes() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "AAA".into());
        g.upsert_room(2, "Medium Length Room".into()); // 18 chars: > old LABEL_W-1 (10), fits new
        g.set_current(1);

        let width = 80;
        let ml = layout(&g, MAIN_LAYER, width);
        // Capped at the longest name (18) plus its one-column marker gutter — not ballooned out
        // to whatever the direction columns left behind (which is far more than 19 at width 80).
        assert_eq!(ml.label_w, 19, "the column grows only as far as the longest name needs");

        for (text, mark) in &ml.labels {
            assert!(mark.is_empty(), "no room needs a footnote marker here: {text:?} got {mark:?}");
        }
        assert!(ml.footnotes.is_empty(), "every name fits: no footnote list at all: {:?}", ml.footnotes);

        let area = Rect { x: 0, y: 0, width, height: 24 };
        let text = render_lines(&g, MAIN_LAYER, area).join("\n");
        assert!(text.contains("Medium Length Room"), "the full name is on screen: {text:?}");
        for d in SUPERSCRIPTS {
            assert!(!text.contains(d), "no superscript marker anywhere on screen: {text:?}");
        }
    }

    /// The mixed case: one name too long for even the grown column, alongside names short enough
    /// to fit it — only the long one gets a superscript and a footnote entry, exactly as today's
    /// per-row truncation already does, just against a wider column.
    #[test]
    fn only_the_name_that_still_does_not_fit_is_footnoted() {
        let mut g = MapGraph::new();
        let long_name = "This Name Is Deliberately Long For The Direction Table Test";
        g.upsert_room(1, "AAA".into());
        g.upsert_room(2, "Medium Length Room".into()); // 18 chars
        g.upsert_room(3, long_name.into()); // far longer than the other two
        g.set_current(1);

        let width = 60;
        let ml = layout(&g, MAIN_LAYER, width);
        assert!(ml.label_w > LABEL_W, "there was room to grow: got {}", ml.label_w);
        assert!(
            (ml.label_w as usize) < long_name.chars().count(),
            "the column must NOT have grown enough to swallow the long name whole: {}",
            ml.label_w
        );

        let idx = |id| ml.matrix.index_of(id).expect("every room has a row");
        let (aaa_text, aaa_mark) = &ml.labels[idx(1)];
        let (medium_text, medium_mark) = &ml.labels[idx(2)];
        let (long_text, long_mark) = &ml.labels[idx(3)];
        assert_eq!(aaa_text, "AAA", "short name: untouched");
        assert!(aaa_mark.is_empty());
        assert_eq!(medium_text, "Medium Length Room", "medium name: fits the grown column whole");
        assert!(medium_mark.is_empty(), "…so it needs no footnote marker");
        assert_ne!(long_text, long_name, "the long name is still cut down");
        assert!(!long_mark.is_empty(), "…and still carries a footnote marker");

        assert_eq!(
            ml.footnotes.iter().filter(|f| !f.marker.is_empty()).count(),
            1,
            "exactly one row-truncation footnote — the long name's, and no other: {:?}",
            ml.footnotes
        );
    }

    /// A rendered buffer's lines as plain strings, right-trimmed — enough to search for a
    /// footnote or a row's leading marker.
    fn render_lines(g: &MapGraph, layer: LayerId, area: Rect) -> Vec<String> {
        let st = AppState::default();
        let mut buf = Buffer::empty(area);
        render_matrix(g, layer, &st, area, &mut buf);
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" ").to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// A room in ANOTHER layer with a passage into room 2 — the mirror of the `⇱out` crossing
    /// `tiny()` already has via room 2 → room 3 is in-layer, so this adds a genuinely different
    /// layer to cross from.
    fn tiny_with_door() -> MapGraph {
        let mut g = tiny();
        g.upsert_room(4, "Outside".into());
        let other = g.new_layer(Some(MAIN_LAYER), "Other".into());
        g.set_room_layer(4, other);
        g.add_edge(4, Direction::S, 2);
        g
    }

    #[test]
    fn an_inbound_border_edge_gets_a_footnote_naming_origin_direction_and_target() {
        let g = tiny_with_door();
        let ml = layout(&g, MAIN_LAYER, 200);
        assert!(
            ml.footnotes.iter().any(|f| f.text == "⇲ in:  Outside —S→ Maze 2"),
            "expected an `⇲ in:` footnote, got {:?}",
            ml.footnotes
        );
        assert_eq!(ml.entry_rooms.len(), 1, "only the actual entrance target is marked: {:?}", ml.entry_rooms);
        assert!(ml.entry_rooms.contains(&2));
    }

    #[test]
    fn the_entry_marker_appears_on_the_target_rooms_row_and_nowhere_else() {
        let g = tiny_with_door();
        let area = Rect { x: 0, y: 0, width: 100, height: 24 };
        let lines = render_lines(&g, MAIN_LAYER, area);
        // Rows start at HEADER_ROWS (2): room 1 (here), room 2 (the entrance), the Dead End.
        assert!(lines[2].starts_with("▸Maze 1"), "room 1 is `here`, not an entrance: {:?}", lines[2]);
        assert!(lines[3].starts_with("⇲Maze 2"), "room 2 is the entrance target: {:?}", lines[3]);
        assert!(!lines[4].starts_with('⇲') && !lines[4].starts_with('▸'), "{:?}", lines[4]);
        // The table rows only — the footnote block legitimately spells out `⇲ in:` in prose, so a
        // whole-text search for the glyph would conflate the two.
        let table_rows = &lines[2..5];
        assert_eq!(
            table_rows.iter().filter(|l| l.starts_with('⇲')).count(),
            1,
            "exactly one row carries the entry marker: {table_rows:?}"
        );
    }

    /// `▸` (here) wins over `⇲` (entrance) when a room is both — the entrance fact still lives in
    /// the footnote, but the row only has one marker column.
    #[test]
    fn the_here_marker_wins_over_the_entry_marker() {
        let mut g = tiny_with_door();
        g.set_current(2); // stand in the very room the outside door leads to
        let area = Rect { x: 0, y: 0, width: 100, height: 24 };
        let lines = render_lines(&g, MAIN_LAYER, area);
        assert!(lines[3].starts_with("▸Maze 2"), "the here-marker must win: {:?}", lines[3]);
        assert!(!lines[3].starts_with('⇲'), "…and the entry marker must not also show: {:?}", lines[3]);
        // No OTHER row picks up a stray marker either.
        let table_rows = &lines[2..5];
        assert!(
            table_rows.iter().all(|l| !l.starts_with('⇲')),
            "no bare entry marker anywhere in the table: {table_rows:?}"
        );
        // The fact is still recorded — just in the footnote, not the row.
        let ml = layout(&g, MAIN_LAYER, area.width);
        assert!(ml.entry_rooms.contains(&2), "room 2 is still a recognised entrance target");
    }

    #[test]
    fn selection_steps_through_rows_and_stops_at_the_ends() {
        let g = tiny();
        assert_eq!(step_selection(&g, MAIN_LAYER, None, 1), Some(1), "nothing selected → where you are");
        assert_eq!(step_selection(&g, MAIN_LAYER, Some(1), 1), Some(2));
        assert_eq!(step_selection(&g, MAIN_LAYER, Some(3), 1), Some(3), "the last row does not wrap");
        assert_eq!(step_selection(&g, MAIN_LAYER, Some(1), -1), Some(1), "nor the first");
        assert_eq!(step_selection(&g, MAIN_LAYER, Some(3), -2), Some(1));
        assert_eq!(step_selection(&MapGraph::new(), MAIN_LAYER, None, 1), None, "an empty layer");
    }

    /// SQ-1246: every drawn row label publishes a hit-rect (a tooltip needs an anchor to hover
    /// on), and every non-empty destination cell does too. A cell with nothing to name — untried
    /// or probed, the two "empty" glyphs (`·` and `×`) — must not.
    #[test]
    fn every_row_label_and_non_empty_cell_publishes_a_hit_rect() {
        let g = tiny();
        let m = matrix::build(&g, MAIN_LAYER);
        let st = AppState::default();
        let area = Rect { x: 0, y: 0, width: 100, height: 24 };
        let mut buf = Buffer::empty(area);
        let hits = render_matrix(&g, MAIN_LAYER, &st, area, &mut buf);

        // One row-label rect per row, at the pane's left edge (cell rects start past `LABEL_W`).
        let label_hits: Vec<_> = hits.iter().filter(|(_, r)| r.x == area.x).collect();
        assert_eq!(label_hits.len(), m.rows.len(), "every row label gets a rect: {hits:?}");
        for row in &m.rows {
            assert!(
                label_hits.iter().any(|(room, _)| *room == row.room),
                "room {} has no row-label rect: {hits:?}",
                row.room
            );
        }

        // Every cell whose classification actually names a room IN this layer gets a rect;
        // untried/probed cells (no destination at all) contribute none.
        let expected_cell_hits = m
            .rows
            .iter()
            .flat_map(|row| row.cells.iter())
            .filter(|cell| cell.dest().is_some_and(|d| m.index_of(d).is_some()))
            .count();
        assert_eq!(hits.len() - label_hits.len(), expected_cell_hits, "hits: {hits:?}");
    }

    /// Hovering the truncated row label shows the FULL name — falsified by reverting
    /// `draw_hover_tip` to always return `None`, which turns this into "the room's name never
    /// appears anywhere on screen but the footnote", the originally requested behaviour.
    #[test]
    fn hovering_a_truncated_row_label_shows_the_full_room_name() {
        let g = tiny();
        let st_default = AppState::default();
        let area = Rect { x: 0, y: 0, width: 100, height: 24 };
        let mut buf = Buffer::empty(area);
        let hits = render_matrix(&g, MAIN_LAYER, &st_default, area, &mut buf);

        // Room 3's name ("Dead End, near Vending Machine") does not fit `LABEL_W` and is
        // truncated on screen — exactly the case a tooltip exists for.
        let (_, label_rect) =
            *hits.iter().find(|(room, r)| *room == 3 && r.x == area.x).expect("room 3's row label");

        let mut st = AppState::default();
        st.matrix_hover = Some((3, label_rect));
        let mut buf2 = Buffer::empty(area);
        render_matrix(&g, MAIN_LAYER, &st, area, &mut buf2);
        let painted =
            draw_hover_tip(&g, MAIN_LAYER, &st, area, &mut buf2).expect("a tip was painted");
        assert!(
            buf_contains(&buf2, painted, "Dead End, near Vending Machine"),
            "the full name must appear in the tip box"
        );
    }

    /// Hovering a destination cell shows the DESTINATION's full name, not the row it sits in's.
    #[test]
    fn hovering_a_destination_cell_shows_the_destinations_full_name() {
        let g = tiny();
        let area = Rect { x: 0, y: 0, width: 100, height: 24 };
        let st_default = AppState::default();
        let mut probe = Buffer::empty(area);
        let hits = render_matrix(&g, MAIN_LAYER, &st_default, area, &mut probe);

        // A cell rect sits past the label column and names room 3 as its destination — the
        // `→3⇠n`/`⇄DE`-shaped cell on room 2's row.
        let (room, cell_rect) = *hits
            .iter()
            .find(|(room, r)| *room == 3 && r.x > area.x)
            .expect("a destination cell pointing at room 3");
        assert_eq!(room, 3);

        let mut st = AppState::default();
        st.matrix_hover = Some((room, cell_rect));
        let mut buf = Buffer::empty(area);
        render_matrix(&g, MAIN_LAYER, &st, area, &mut buf);
        let painted = draw_hover_tip(&g, MAIN_LAYER, &st, area, &mut buf).expect("a tip was painted");
        assert!(
            buf_contains(&buf, painted, "Dead End, near Vending Machine"),
            "the destination's full name must appear in the tip box"
        );
    }

    /// No hovered room, or one that names nothing in this layer's table, paints nothing.
    #[test]
    fn no_hover_or_an_unresolvable_room_paints_no_tooltip() {
        let g = tiny();
        let area = Rect { x: 0, y: 0, width: 100, height: 24 };
        let mut st = AppState::default();
        let mut buf = Buffer::empty(area);
        render_matrix(&g, MAIN_LAYER, &st, area, &mut buf);
        assert_eq!(draw_hover_tip(&g, MAIN_LAYER, &st, area, &mut buf), None, "nothing hovered");

        st.matrix_hover = Some((999, Rect::new(0, 2, LABEL_W, 1)));
        assert_eq!(
            draw_hover_tip(&g, MAIN_LAYER, &st, area, &mut buf),
            None,
            "a room with no name in this layer's table gets no tip"
        );
    }

    /// A modal dialog owns the pointer — same rule the border-control hint follows — so no tip
    /// draws underneath it even with a perfectly valid hover still recorded.
    #[test]
    fn a_modal_overlay_suppresses_the_tooltip() {
        let g = tiny();
        let area = Rect { x: 0, y: 0, width: 100, height: 24 };
        let st_default = AppState::default();
        let mut probe = Buffer::empty(area);
        let hits = render_matrix(&g, MAIN_LAYER, &st_default, area, &mut probe);
        let (room, rect) = *hits.iter().find(|(room, r)| *room == 3 && r.x == area.x).unwrap();

        let mut st = AppState::default();
        st.matrix_hover = Some((room, rect));
        st.overlays.hotkey_dialog = true;
        let mut buf = Buffer::empty(area);
        assert_eq!(
            draw_hover_tip(&g, MAIN_LAYER, &st, area, &mut buf),
            None,
            "a modal overlay must suppress the tip"
        );
    }

    /// Cell contents as plain strings within `rect`, concatenated — enough to search for text a
    /// tooltip box painted.
    fn buf_contains(buf: &Buffer, rect: Rect, needle: &str) -> bool {
        let mut joined = String::new();
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                joined.push_str(buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "));
            }
        }
        joined.contains(needle)
    }
}
