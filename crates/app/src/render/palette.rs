//! Command-palette popup overlay (SQ-0419).
//!
//! A fuzzy-searchable list of every registry command, drawn with the shared
//! dialog chrome. The top content row is the palette's own input line (query +
//! args); below it is the ranked candidate list, matched characters highlighted,
//! the selected row emphasised. Works anywhere — even where no story prompt
//! exists (modal / debug views).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use crate::complete::palette_candidates;
use crate::render::dialog::{
    draw_dialog, DialogField, DialogRects, DialogSpec, DialogStyle, Placement,
};
use crate::render::{draw_str_clipped, put_char};
use crate::state::AppState;

/// Column (relative to the list area's left edge) where a candidate's one-line
/// help starts: the shared help column, or one clear cell past a name too long to
/// fit before it. The name column is the same in every row so the list stays
/// scannable, and the wrapped sole match (SQ-1149) hangs its continuation rows on
/// this same column.
fn help_col(name: &str) -> u16 {
    const HELP_COL: u16 = 26;
    (name.chars().count() as u16 + 2).max(HELP_COL)
}

/// The wrapped help of a lone candidate, or `None` while several are listed.
///
/// Measured against the modal's own width, so it can size the dialog *before* the
/// chrome is drawn — the list area is the modal minus its one-cell border.
fn sole_match_wrap(cands: &[crate::complete::PaletteCandidate], modal_w: u16) -> Option<Vec<String>> {
    let [cand] = cands else { return None };
    let spec = &crate::slash::COMMANDS[cand.cmd_index];
    let w = modal_w.saturating_sub(2).saturating_sub(help_col(spec.name));
    Some(crate::render::transcript::wrap_line(spec.description, w))
}

/// Draw the command palette centered over `area`.
///
/// `vp_out` receives the candidate-list viewport height (rows) so nav actions can
/// window/animate the scroll. `hits` is filled with `(cmd_index, row_rect)` for
/// every drawn candidate row so a click can execute that command.
///
/// Does nothing when `state.overlays.palette` is `None`. Returns `Some(DialogRects)`
/// when drawn (for `[X]` / outside-click hit-testing), `None` otherwise.
pub fn draw_palette(
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
    vp_out: &mut usize,
    hits: &mut Vec<(usize, Rect)>,
) -> Option<DialogRects> {
    let Some(palette) = &state.overlays.palette else { return None };

    let cands = palette_candidates(palette.query());

    // ── Modal geometry ────────────────────────────────────────────────────────
    let modal_w = 72u16.min(area.width.saturating_sub(4));
    // 1 field row + candidate rows + 1 footer + border(2). Cap the list height so
    // a huge registry never overflows the screen.
    //
    // A SINGLE match has the dialog to itself, so its help is WRAPPED over as many
    // rows as it needs and the modal grows downward to fit (SQ-1149). With several
    // matches the help stays one truncated row each, because a column you can scan
    // beats a sentence you can finish. Growth is still bounded by the pane (the
    // `.min` below): a description too long for the space clips at the last row,
    // exactly as a truncated one does today.
    let sole_wrap = sole_match_wrap(&cands, modal_w);
    let list_rows = match &sole_wrap {
        Some(lines) => lines.len().max(1) as u16,
        None => (cands.len() as u16).clamp(1, 14),
    };
    let modal_h = (list_rows + 4).min(area.height.saturating_sub(2));
    if modal_w < 24 || modal_h < 5 {
        return None;
    }

    // ── Dialog chrome + input field ───────────────────────────────────────────
    let st = DialogStyle::from_colors(&state.colors);
    let theme = &state.colors.theme;
    let query_style = theme.get("palette_query").style;
    let caret_style = query_style.add_modifier(Modifier::REVERSED);

    let field = DialogField {
        label: "/",
        value: &palette.input.value,
        cursor: palette.input.cursor,
        show_caret: true,
        dim: false,
        text_style: query_style,
        dim_style: query_style.add_modifier(Modifier::DIM),
        caret_style,
    };

    let spec = DialogSpec {
        title: "Command Palette  (/ commands)",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons: &[],
        show_close: true,
        default: None,
        focus: None,
        field: Some(field),
    };

    let rects = draw_dialog(buf, area, &spec, &st);
    let content = rects.content;
    if content.height == 0 {
        return Some(rects);
    }

    // Content layout: row 0 = the input field (drawn by draw_dialog); the last
    // row = footer hint; the candidate list fills the middle.
    let footer_y = content.bottom().saturating_sub(1);
    let list_top = content.y + 1;
    let list_h = footer_y.saturating_sub(list_top);
    let list_area = Rect::new(content.x, list_top, content.width, list_h);
    *vp_out = list_area.height as usize;

    // ── Candidate rows ────────────────────────────────────────────────────────
    let name_style = theme.get("palette_name").style;
    let match_style = theme.get("palette_match").style;
    let desc_style = theme.get("palette_desc").style;
    let sel_style = theme.get("palette_selected").style;

    if cands.is_empty() {
        draw_str_clipped(buf, list_area.x, list_area.y, "(no matching commands)", desc_style, list_area);
    } else {
        let total = cands.len();
        let viewport = list_area.height as usize;
        let scrollbar_visible =
            crate::render::scroll::needs_scrollbar(total, viewport) && list_area.width >= 2;
        let row_w = if scrollbar_visible { list_area.width.saturating_sub(1) } else { list_area.width };
        let offset = palette.scroll.display_offset();
        let selected = palette.scroll.selected;

        // Rows consumed so far. Every candidate is one row except the sole match,
        // whose wrapped help makes its entry as many rows tall as `sole_wrap`.
        let mut row = 0u16;
        let mut i = offset;
        while i < total && row < list_area.height {
            let cand = &cands[i];
            let spec = &crate::slash::COMMANDS[cand.cmd_index];
            let row_y = list_area.y + row;
            let is_sel = i == selected;
            let base = if is_sel { sel_style } else { name_style };

            // Help column: col 26, or one clear cell past a name too long to fit
            // before it. `sole_wrap` was measured against this same offset.
            let help_col = help_col(spec.name);
            let desc_x = list_area.x + help_col;
            let desc_w = row_w.saturating_sub(help_col);

            let owned;
            let lines: &[String] = match &sole_wrap {
                Some(l) => l,
                None => {
                    owned = [spec.description.chars().take(desc_w as usize).collect::<String>()];
                    &owned
                }
            };
            let h = (lines.len() as u16).min(list_area.height - row);

            // Fill the entry so the selection highlight spans its full width — and,
            // for a wrapped sole match, all of its rows.
            for y in row_y..row_y + h {
                for col in list_area.x..list_area.x + row_w {
                    if let Some(cell) = buf.cell_mut((col, y)) {
                        cell.set_symbol(" ").set_style(base);
                    }
                }
            }
            // The whole entry is the click target, wrapped continuation included.
            hits.push((cand.cmd_index, Rect::new(list_area.x, row_y, row_w, h)));

            // Command name, matched chars highlighted, on the entry's first row.
            let name_x = list_area.x + 1;
            for (ci, ch) in spec.name.chars().enumerate() {
                let x = name_x + ci as u16;
                let matched = cand.positions.contains(&ci);
                let style = match (is_sel, matched) {
                    (true, true) => sel_style.add_modifier(Modifier::BOLD),
                    (true, false) => sel_style,
                    (false, true) => match_style,
                    (false, false) => name_style,
                };
                put_char(buf, x as i32, row_y as i32, ch, style, list_area);
            }

            // Help text, right of the name column. One row in a list, every
            // wrapped row at the same column for a sole match.
            if desc_w > 0 {
                let d_style = if is_sel { sel_style } else { desc_style };
                for (k, line) in lines.iter().take(h as usize).enumerate() {
                    let y = row_y + k as u16;
                    let clip = Rect::new(desc_x, y, desc_w, 1);
                    draw_str_clipped(buf, desc_x, y, line, d_style, clip);
                }
            }

            row += h;
            i += 1;
        }

        if scrollbar_visible {
            let sb = Rect::new(list_area.right().saturating_sub(1), list_area.y, 1, list_area.height);
            crate::render::scroll::draw_scrollbar(
                buf,
                sb,
                total,
                viewport,
                palette.scroll.target_offset(),
                crate::render::scroll::ScrollbarLook::from_theme(theme),
            );
        }
    }

    // ── Footer hint ───────────────────────────────────────────────────────────
    if footer_y > content.y {
        let footer = "\u{2191}\u{2193} move  Tab complete  Enter run  Esc close";
        draw_str_clipped(buf, content.x, footer_y, footer, desc_style, content);
    }

    Some(rects)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use crate::state::{AppState, PaletteState};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn buf_text(buf: &Buffer) -> String {
        buf.content().iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect()
    }

    fn state_with_palette(query: &str) -> AppState {
        let mut s = AppState::default();
        let mut p = PaletteState::new(false);
        p.input = crate::text_field::TextField::new(query.to_string());
        s.overlays.palette = Some(p);
        s
    }

    #[test]
    fn draw_palette_noop_when_closed() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = AppState::default();
        let before: Vec<_> = terminal.backend().buffer().content().iter().map(|c| c.symbol().to_string()).collect();
        terminal.draw(|f| { draw_palette(&state, f.area(), f.buffer_mut(), &mut 0, &mut Vec::new()); }).unwrap();
        let after: Vec<_> = terminal.backend().buffer().content().iter().map(|c| c.symbol().to_string()).collect();
        assert_eq!(before, after, "draw_palette must be a no-op when the palette is closed");
    }

    #[test]
    fn draw_palette_lists_matching_commands_and_chrome() {
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_palette("zoom");
        let mut rects = None;
        let mut hits = Vec::new();
        terminal.draw(|f| { rects = draw_palette(&state, f.area(), f.buffer_mut(), &mut 0, &mut hits); }).unwrap();
        let text = buf_text(terminal.backend().buffer());
        assert!(text.contains("Command Palette"), "title should be present");
        assert!(text.contains("zoom-map"), "zoom-map should be listed for query 'zoom'");
        assert!(text.contains('\u{2715}'), "[X] close button should be visible");
        let rects = rects.expect("palette open should return rects");
        assert!(rects.close.is_some());
        // Row hits recorded, and zoom-map is among them.
        assert!(hits.iter().any(|(i, _)| crate::slash::COMMANDS[*i].name == "zoom-map"));
    }

    /// SQ-1149: a lone candidate gets its whole help, wrapped, and the modal grows
    /// downward to hold it — where a list of several still truncates each row.
    #[test]
    fn sole_match_wraps_its_help_and_grows_the_dialog() {
        // A command whose help is far longer than the ~44-column help field, and a
        // query that narrows to it alone.
        let query = "returnprobe";
        let cands = palette_candidates(query);
        assert_eq!(cands.len(), 1, "query {query:?} must narrow to exactly one command");
        let spec = &crate::slash::COMMANDS[cands[0].cmd_index];
        assert!(spec.description.chars().count() > 60, "fixture command must have a long help");

        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_palette(query);
        let mut hits = Vec::new();
        terminal
            .draw(|f| {
                draw_palette(&state, f.area(), f.buffer_mut(), &mut 0, &mut hits);
            })
            .unwrap();
        let buf = terminal.backend().buffer();

        // Every word of the help is on screen somewhere, not just the first 44 chars.
        let rows: Vec<String> = (0..buf.area.height)
            .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect())
            .collect();
        let screen = rows.join(" ");
        for word in spec.description.split_whitespace() {
            assert!(screen.contains(word), "help word {word:?} missing — description was truncated");
        }

        // The dialog grew: the entry is taller than one row, and the click target
        // covers all of it.
        let (_, hit) = hits.first().expect("the sole candidate must be clickable");
        assert!(hit.height > 1, "a wrapped sole match must occupy more than one row, got {hit:?}");
    }

    /// The counterpart: several matches keep one row each, so the name column stays
    /// scannable (SQ-1149 deliberately only grows the single-match case).
    #[test]
    fn several_matches_keep_one_row_each() {
        let query = "map";
        let cands = palette_candidates(query);
        assert!(cands.len() > 1, "query {query:?} should match several commands");

        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_palette(query);
        let mut hits = Vec::new();
        terminal
            .draw(|f| {
                draw_palette(&state, f.area(), f.buffer_mut(), &mut 0, &mut hits);
            })
            .unwrap();
        assert!(hits.iter().all(|(_, r)| r.height == 1), "list rows must stay one row tall");
    }

    #[test]
    fn draw_palette_shows_no_match_message() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_palette("zzzznotacommand");
        terminal.draw(|f| { draw_palette(&state, f.area(), f.buffer_mut(), &mut 0, &mut Vec::new()); }).unwrap();
        let text = buf_text(terminal.backend().buffer());
        assert!(text.contains("no matching"), "expected a no-match message");
    }
}
