//! VFS file-picker modal overlay (read-mode `create_by_prompt`).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use crate::render::dialog::{ButtonId, DialogButton, DialogRects, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::state::AppState;

/// Draw the VFS file-picker modal centered over `area`.
///
/// Lists the in-memory VFS filenames offered for a read-mode `create_by_prompt`
/// (distinct from the on-disk `file_browser`). The currently-selected row is
/// highlighted. A footer shows the available key actions.
///
/// Does nothing when `state.overlays.file_picker` is `None`.
/// Returns `Some(DialogRects)` when drawn (for mouse hit-testing), `None` otherwise.
pub fn draw_file_picker(
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
    vp_out: &mut usize,
) -> Option<DialogRects> {
    let Some(picker) = &state.overlays.file_picker else { return None };

    // ── Modal geometry ────────────────────────────────────────────────────────

    // Target: up to 50 wide, tall enough for entries + 1 footer + chrome overhead.
    let modal_w = 50u16.min(area.width.saturating_sub(4));
    let entry_rows = picker.names.len() as u16;
    // entry rows + 1 footer + border overhead (2) + button row (1) = entry_rows + 4
    let modal_h = (entry_rows + 4).min(area.height.saturating_sub(2));
    if modal_w < 20 || modal_h < 4 {
        return None;
    }

    // ── Build DialogStyle from state colors ───────────────────────────────────

    let st = DialogStyle::from_colors(&state.colors);

    let buttons = &[
        DialogButton { id: ButtonId::Done, label: "Cancel" },
    ];

    let spec = DialogSpec {
        title: "Pick a file to read",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Done),
        focus: Some(state.overlays.dialog_focus),
        field: None,
    };

    let rects = draw_dialog(buf, area, &spec, &st);
    let content = rects.content;

    // ── Entry rows ────────────────────────────────────────────────────────────

    let normal = state.colors.theme.get("dialog.background").style;
    let selected_style = state.colors.theme.get("dialog.list_selected").style;

    let entries_area = content;

    let total = picker.names.len();
    let viewport = entries_area.height.saturating_sub(1) as usize;
    *vp_out = viewport;

    if total == 0 {
        let empty_style = normal.patch(state.colors.theme.get("dialog.list_footer").style);
        if content.height > 0 {
            crate::render::draw_str_clipped(buf, content.x, content.y, "(no files)", empty_style, content);
        }
        return Some(rects);
    }

    // Reserve a 1-column gutter on the right for the scrollbar when overflowing.
    let scrollbar_visible =
        crate::render::scroll::needs_scrollbar(total, viewport) && content.width >= 2;
    let row_w = if scrollbar_visible { content.width.saturating_sub(1) } else { content.width };
    let row_area = Rect::new(content.x, entries_area.y, row_w, entries_area.height.saturating_sub(1));

    let offset = picker.scroll.display_offset();
    for row in 0..viewport {
        let i = offset + row;
        if i >= total {
            break;
        }
        let name = &picker.names[i];
        let row_y = row_area.y + row as u16;

        let style = if i == picker.scroll.selected { selected_style } else { normal };

        // Fill the whole row background with the row style.
        for col in row_area.x..row_area.right() {
            if let Some(cell) = buf.cell_mut((col, row_y)) {
                cell.set_symbol(" ").set_style(style);
            }
        }

        let marker = if i == picker.scroll.selected { ">" } else { " " };
        let line = format!("{} {}", marker, name);
        crate::render::draw_str_clipped(buf, row_area.x, row_y, &line, style, row_area);
    }

    if scrollbar_visible {
        let sb_area = Rect::new(row_area.right(), row_area.y, 1, row_area.height);
        crate::render::scroll::draw_scrollbar(
            buf,
            sb_area,
            total,
            viewport,
            picker.scroll.target_offset(),
            crate::render::scroll::ScrollbarLook::from_theme(&state.colors.theme),
        );
    }

    // ── Footer hint (below entries) ───────────────────────────────────────────

    let footer_y = row_area.bottom();
    if footer_y < content.bottom() {
        let footer_style = normal.patch(state.colors.theme.get("dialog.list_footer").style);
        let footer = "Enter:pick  Esc:cancel";
        crate::render::draw_str_clipped(buf, content.x, footer_y, footer, footer_style, content);
    }

    Some(rects)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use crate::render::dialog::ButtonId;
    use crate::state::{AppState, FilePickerState};

    fn state_with_picker(names: Vec<&str>, selected: usize) -> AppState {
        let mut s = AppState::default();
        let mut fp = FilePickerState::new(names.into_iter().map(|s| s.to_string()).collect());
        fp.scroll.selected = selected;
        s.overlays.file_picker = Some(fp);
        s
    }

    #[test]
    fn draw_file_picker_noop_when_closed() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = AppState::default(); // file_picker = None
        let before: Vec<_> = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().to_string())
            .collect();
        terminal.draw(|f| {
            draw_file_picker(&state, f.area(), f.buffer_mut(), &mut 0);
        }).unwrap();
        let after: Vec<_> = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().to_string())
            .collect();
        assert_eq!(before, after, "draw_file_picker should be a no-op when file_picker is None");
    }

    #[test]
    fn draw_file_picker_shows_filenames_and_chrome() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_picker(vec!["notes.txt", "diary.txt"], 0);
        let mut rects_out: Option<DialogRects> = None;
        terminal.draw(|f| {
            rects_out = draw_file_picker(&state, f.area(), f.buffer_mut(), &mut 0);
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("Pick a file to read"), "title should be present");
        assert!(content.contains("notes.txt"), "first entry should be listed");
        assert!(content.contains("diary.txt"), "second entry should be listed");
        let rects = rects_out.expect("draw_file_picker should return DialogRects when open");
        assert!(rects.close.is_some(), "close rect should be present");
        let ids: Vec<ButtonId> = rects.buttons.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&ButtonId::Done));
    }

    #[test]
    fn draw_file_picker_empty_state() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_picker(vec![], 0);
        terminal.draw(|f| {
            draw_file_picker(&state, f.area(), f.buffer_mut(), &mut 0);
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("no files"), "empty-state hint should be shown");
    }
}
