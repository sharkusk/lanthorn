//! File-browser modal overlay for importing standard Quetzal saves.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use crate::render::dialog::{ButtonId, DialogButton, DialogRects, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::state::AppState;

/// Draw the file-browser modal centered over `area`.
///
/// Shows the current directory, a list of entries (".." parent, dirs, then
/// matching files in PickFile mode), and a footer with key hints.
///
/// Does nothing when `state.overlays.file_browser` is `None`.
/// Returns `Some(DialogRects)` when drawn (for mouse hit-testing), `None` otherwise.
pub fn draw_file_browser(
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
    vp_out: &mut usize,
) -> Option<DialogRects> {
    let Some(fb) = &state.overlays.file_browser else { return None };

    // ── Modal geometry ────────────────────────────────────────────────────────

    let modal_w = 64u16.min(area.width.saturating_sub(4));
    let entry_rows = fb.entries.len() as u16;
    // 1 cwd row + entry rows + 1 footer + border overhead (2) + button row (1) = entry_rows + 5
    let modal_h = (entry_rows + 5).max(7).min(area.height.saturating_sub(2));
    if modal_w < 20 || modal_h < 4 {
        return None;
    }

    // ── Build DialogStyle from state colors ───────────────────────────────────

    let st = DialogStyle::from_colors(&state.colors);

    use crate::state::FbMode;
    let title = match fb.mode {
        FbMode::PickFile => "Import Save (.qzl/.sav)",
    };

    let buttons = &[
        DialogButton { id: ButtonId::Done, label: "Done" },
    ];

    let spec = DialogSpec {
        title,
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Done),
        focus: None,
        field: None,
    };

    let rects = draw_dialog(buf, area, &spec, &st);
    let content = rects.content;

    // ── CWD row ───────────────────────────────────────────────────────────────

    let cwd_str = format!("  {}", fb.cwd.display());
    let dialog_bg = state.colors.theme.get("dialog.background").style;
    let cwd_style = dialog_bg.patch(state.colors.theme.get("file_browser_cwd").style);
    if content.height > 0 {
        crate::render::draw_str_clipped(buf, content.x, content.y, &cwd_str, cwd_style, content);
    }

    // ── Entry rows ────────────────────────────────────────────────────────────

    let normal = state.colors.theme.get("dialog.background").style;
    let selected_style = state.colors.theme.get("dialog.list_selected").style;
    let dir_style = dialog_bg.patch(state.colors.theme.get("file_browser_dir").style);
    let dir_selected_style = selected_style;

    let entries_area = if content.height > 1 {
        Rect::new(content.x, content.y + 1, content.width, content.height - 1)
    } else {
        Rect::new(content.x, content.y, content.width, 0)
    };

    // Reserve last row of entries_area for footer.
    let entries_max_y = entries_area.bottom().saturating_sub(1);
    // The entry-row band sits above the footer row.
    let band_h = entries_max_y.saturating_sub(entries_area.y);
    let total = fb.entries.len();
    let viewport = band_h as usize;
    *vp_out = viewport;

    // Reserve a 1-column gutter on the right for the scrollbar when overflowing.
    let scrollbar_visible =
        crate::render::scroll::needs_scrollbar(total, viewport) && content.width >= 2;
    let row_w = if scrollbar_visible { content.width.saturating_sub(1) } else { content.width };
    let row_area = Rect::new(content.x, entries_area.y, row_w, band_h);

    let offset = fb.scroll.display_offset();
    for row in 0..viewport {
        let i = offset + row;
        if i >= total {
            break;
        }
        let entry = &fb.entries[i];
        let row_y = entries_area.y + row as u16;

        let is_sel = i == fb.scroll.selected;

        // Fill row background.
        let row_bg = if is_sel { selected_style } else { normal };
        for col in row_area.x..row_area.right() {
            if let Some(cell) = buf.cell_mut((col, row_y)) {
                cell.set_symbol(" ").set_style(row_bg);
            }
        }

        // Choose text style.
        let text_style = if is_sel {
            if entry.is_dir { dir_selected_style } else { selected_style }
        } else if entry.is_dir {
            dir_style
        } else {
            normal
        };

        let marker = if is_sel { ">" } else { " " };
        let suffix = if entry.is_dir && entry.name != ".." { "/" } else { "" };
        let label = format!("{} {}{}", marker, entry.name, suffix);
        crate::render::draw_str_clipped(buf, row_area.x, row_y, &label, text_style, row_area);
    }

    if scrollbar_visible {
        let sb_area = Rect::new(entries_area.right().saturating_sub(1), entries_area.y, 1, band_h);
        crate::render::scroll::draw_scrollbar(
            buf,
            sb_area,
            total,
            viewport,
            fb.scroll.target_offset(),
            crate::render::scroll::ScrollbarLook::from_theme(&state.colors.theme),
        );
    }

    // ── Footer hint ───────────────────────────────────────────────────────────

    let footer_y = entries_max_y;
    if footer_y < entries_area.bottom() {
        let footer_style = dialog_bg.patch(state.colors.theme.get("dialog.list_footer").style);
        let footer = match fb.mode {
            FbMode::PickFile => "Up/Dn:move  Enter:open/import  Esc:cancel",
        };
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
    use crate::render::paneframe::BorderStyle;
    use crate::state::{AppState, FbMode, FileBrowserState};
    use std::path::PathBuf;

    fn state_with_browser(cwd: PathBuf, mode: FbMode) -> AppState {
        let mut s = AppState::default();
        s.overlays.file_browser = Some(FileBrowserState::build(cwd, mode));
        s
    }

    /// SQ-0643: the cwd row's colour used to be a bare `Style::new().fg(Color::Yellow)`
    /// patched THE WRONG WAY onto the dialog background (the background's own fg won,
    /// silently discarding the accent) — no `style.toml` override could ever reach it.
    /// It must now render in the themed `file_browser_cwd` colour, and an override must
    /// actually change it.
    #[test]
    fn draw_file_browser_cwd_row_follows_file_browser_cwd_override() {
        let scheme = crate::colors::GhosttyScheme::default();
        let parsed = crate::theme::toml_schema::parse(
            "[elements]\nfile_browser_cwd = { fg = \"magenta\" }\n",
        ).unwrap();
        let mut state = state_with_browser(PathBuf::from("/tmp"), FbMode::PickFile);
        state.colors.theme = crate::theme::resolve::resolve_theme(&scheme, &parsed);
        state.colors.dialog_box_style = BorderStyle::Single;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| {
            draw_file_browser(&state, f.area(), f.buffer_mut(), &mut 0);
        }).unwrap();

        let want_fg = state.colors.theme.get("file_browser_cwd").style.fg;
        assert_eq!(want_fg, Some(ratatui::style::Color::Magenta), "guard: override must change the resolved style");
        let buf = terminal.backend().buffer();
        let has_override = buf.content().iter().any(|c| c.style().fg == Some(ratatui::style::Color::Magenta));
        assert!(has_override, "the cwd row must render in the overridden file_browser_cwd colour");
    }

    #[test]
    fn draw_file_browser_scrollbar_and_paging_on_overflow() {
        use crate::input::{apply_action, Action};
        use crate::state::FbEntry;
        use mapper::mapper::Mapper;
        let entries: Vec<FbEntry> =
            (0..50).map(|i| FbEntry { name: format!("file-{i}.qzl"), is_dir: false }).collect();
        let mut state = AppState::default();
        state.overlays.file_browser = Some(FileBrowserState {
            cwd: PathBuf::from("/tmp"),
            entries,
            scroll: Default::default(),
            mode: FbMode::PickFile,
        });
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut vp = 0usize;
        terminal.draw(|f| {
            draw_file_browser(&state, f.area(), f.buffer_mut(), &mut vp);
        }).unwrap();
        assert!(vp > 0 && vp < 50, "entries should overflow the modal (vp={vp})");
        // SQ-0782: the thumb is a background fill, not a glyph.
        let thumb = crate::render::scroll::ScrollbarLook::from_theme(&state.colors.theme).thumb;
        let has_thumb = terminal.backend().buffer().content().iter().any(|c| c.bg == thumb);
        assert!(has_thumb, "a scrollbar thumb should be drawn when entries overflow");

        state.modal_list_viewport = vp;
        apply_action(Action::FbPage(1), &mut state, &mut Mapper::default());
        let sel = state.overlays.file_browser.as_ref().unwrap().scroll.selected;
        assert!(sel >= vp.saturating_sub(1), "PageDown should advance ~one viewport, got {sel}");
    }

    #[test]
    fn draw_file_browser_noop_when_closed() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = AppState::default(); // file_browser = None
        let before: Vec<_> = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().to_string())
            .collect();
        terminal.draw(|f| {
            draw_file_browser(&state, f.area(), f.buffer_mut(), &mut 0);
        }).unwrap();
        let after: Vec<_> = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().to_string())
            .collect();
        assert_eq!(before, after, "draw_file_browser should be a no-op when file_browser is None");
    }

    #[test]
    fn draw_file_browser_shows_in_pickfile_mode() {
        let tmp = std::env::temp_dir();
        // Use a taller terminal (30 rows) so the dialog chrome renders with Single border
        // and the title appears in the border row distinct from the content body.
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = state_with_browser(tmp, FbMode::PickFile);
        // Use Single border so title and content do not overlap.
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
        terminal.draw(|f| {
            draw_file_browser(&state, f.area(), f.buffer_mut(), &mut 0);
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("Import"), "should show Import title in PickFile mode");
    }

    #[test]
    fn draw_file_browser_shows_dialog_chrome() {
        // Render test: file browser shows bordered titled chrome with [X] + [Done] and dialog_* colors.
        let tmp = std::env::temp_dir();
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = state_with_browser(tmp, FbMode::PickFile);
        state.colors.dialog_box_style = BorderStyle::Single;
        let mut rects_out: Option<DialogRects> = None;
        terminal.draw(|f| {
            rects_out = draw_file_browser(&state, f.area(), f.buffer_mut(), &mut 0);
        }).unwrap();

        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();

        assert!(content.contains("Import"), "title should be present");
        assert!(content.contains("Done"), "[Done] button should be visible");
        assert!(content.contains('✕'), "[X] close button should be visible");

        let rects = rects_out.expect("draw_file_browser should return DialogRects when open");
        assert!(rects.close.is_some(), "close rect should be present");
        assert_eq!(rects.buttons.len(), 1, "should have 1 button");
        let ids: Vec<ButtonId> = rects.buttons.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&ButtonId::Done));
    }
}
