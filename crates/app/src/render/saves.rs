//! Saves-manager modal overlay.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::render::dialog::{ButtonId, DialogButton, DialogRects, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::state::AppState;

/// Draw the saves-manager modal centered over `area`.
///
/// The modal lists all save files for the current story: default slot labelled
/// "(default)", named saves showing name, turn count, and save timestamp.
/// The currently-selected row is highlighted. A footer shows the available
/// key actions.
///
/// Does nothing when `state.overlays.saves` is `None`.
/// Returns `Some(DialogRects)` when drawn (for mouse hit-testing), `None` otherwise.
pub fn draw_saves(
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
    vp_out: &mut usize,
) -> Option<DialogRects> {
    let Some(saves) = &state.overlays.saves else { return None };

    // ── Modal geometry ────────────────────────────────────────────────────────

    // Target: up to 66 wide, tall enough for entries + 2 header + 1 footer + chrome overhead.
    let modal_w = 66u16.min(area.width.saturating_sub(4));
    let entry_rows = saves.entries.len() as u16;
    // 2 header rows + entry rows + 1 footer + border overhead (2) + button row (1) = entry_rows + 6
    let modal_h = (entry_rows + 6).min(area.height.saturating_sub(2));
    if modal_w < 20 || modal_h < 4 {
        return None;
    }

    // ── Build DialogStyle from state colors ───────────────────────────────────

    let st = DialogStyle::from_colors(&state.colors);

    let buttons = &[
        DialogButton { id: ButtonId::Done, label: "Done" },
    ];

    let spec = DialogSpec {
        title: "Saves",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Done),
        focus: Some(state.overlays.dialog_focus),
        field: None,
    };

    let rects = draw_dialog(buf, area, &spec, &st);
    let content = rects.content;

    // ── Column headers ────────────────────────────────────────────────────────

    let hdr_style = Style::new().add_modifier(Modifier::UNDERLINED).patch(state.colors.theme.get("dialog.background").style);
    let hdr = format!("{:<25}  {:<6}  {:>5}  {:<16}", "Name", "Type", "Turns", "Saved at");
    if content.height > 0 {
        crate::render::draw_str_clipped(buf, content.x, content.y, &hdr, hdr_style, content);
    }

    // ── Entry rows ────────────────────────────────────────────────────────────

    let normal = state.colors.theme.get("dialog.background").style;
    let selected_style = state.colors.theme.get("dialog.list_selected").style;

    let entries_area = if content.height > 1 {
        Rect::new(content.x, content.y + 1, content.width, content.height - 1)
    } else {
        Rect::new(content.x, content.y, content.width, 0)
    };

    let total = saves.entries.len();
    // Reserve the LAST row of `entries_area` for the footer (SQ-0639):
    // `entries_area.bottom() == content.bottom()` whenever content.height > 1,
    // so the old `footer_y < content.bottom()` check below was always false
    // and no row was ever actually reserved for the footer text.
    let viewport = entries_area.height.saturating_sub(1) as usize;
    *vp_out = viewport;

    // Reserve a 1-column gutter on the right for the scrollbar when overflowing.
    let scrollbar_visible =
        crate::render::scroll::needs_scrollbar(total, viewport) && content.width >= 2;
    let row_w = if scrollbar_visible { content.width.saturating_sub(1) } else { content.width };
    let row_area = Rect::new(content.x, entries_area.y, row_w, entries_area.height.saturating_sub(1));

    let offset = saves.scroll.display_offset();
    for row in 0..viewport {
        let i = offset + row;
        if i >= total {
            break;
        }
        let entry = &saves.entries[i];
        let row_y = entries_area.y + row as u16;

        let style = if i == saves.scroll.selected { selected_style } else { normal };

        // Fill the whole row background with the row style.
        for col in row_area.x..row_area.right() {
            if let Some(cell) = buf.cell_mut((col, row_y)) {
                cell.set_symbol(" ").set_style(style);
            }
        }

        // Marker + name + type + turns + time. The Type cell says what a restore
        // will do AND whether the save travels: "Game" saves hold standard
        // save-instruction-PC bytes any interpreter reads, so they carry the
        // portable glyph; "State" saves are host snapshots taken between turns
        // and stay here (SQ-0531).
        let marker = if i == saves.scroll.selected { ">" } else { " " };
        let name_trunc: String = entry.name.chars().take(23).collect();
        let portable = entry.trigger.is_portable();
        let kind = if portable {
            format!("Game{}", portable_glyph(state))
        } else {
            "State".to_string()
        };
        let short_time = format_time(&entry.saved_at);
        let line = format!(
            "{} {:<24}  {:<6}  {:>5}  {:<16}",
            marker, name_trunc, kind, entry.turns, short_time
        );
        crate::render::draw_str_clipped(buf, row_area.x, row_y, &line, style, row_area);

        // Tint the Type cell so portability reads at a glance. The selected row
        // keeps its highlight — the cursor must stay the loudest thing on screen.
        if i != saves.scroll.selected {
            let sel = if portable { "saves_portable" } else { "saves_host_only" };
            let cell_style = style.patch(state.colors.theme.get(sel).style);
            let cell = Rect::new(
                row_area.x + TYPE_COL,
                row_y,
                TYPE_COL_W.min(row_area.width.saturating_sub(TYPE_COL)),
                1,
            );
            crate::render::draw_str_clipped(buf, cell.x, row_y, &format!("{:<6}", kind), cell_style, cell);
        }
    }

    if scrollbar_visible {
        let sb_area = Rect::new(entries_area.right().saturating_sub(1), entries_area.y, 1, row_area.height);
        crate::render::scroll::draw_scrollbar(
            buf,
            sb_area,
            total,
            viewport,
            saves.scroll.target_offset(),
            crate::render::scroll::ScrollbarLook::from_theme(&state.colors.theme),
        );
    }

    // ── Footer hint (below entries) ───────────────────────────────────────────

    let footer_y = row_area.bottom();
    if footer_y < content.bottom() {
        let footer_style = normal.patch(state.colors.theme.get("dialog.list_footer").style);
        let footer = "Enter:load  s:save-as  d:delete  i:import  Esc:close";
        crate::render::draw_str_clipped(buf, content.x, footer_y, footer, footer_style, content);
    }

    Some(rects)
}

/// Column offset (within a row) and width of the Type cell, kept beside the row
/// format string above: `marker`(1) + space(1) + `name`(24) + 2 spaces = 28.
const TYPE_COL: u16 = 28;
const TYPE_COL_W: u16 = 6;

/// The themeable glyph appended to a portable save's Type cell (default `↗`).
/// Falls back to a plain space when a theme blanks it out.
fn portable_glyph(state: &AppState) -> String {
    state
        .colors
        .theme
        .get("saves_portable")
        .glyph
        .as_ref()
        .and_then(|g| g.single.as_deref())
        .unwrap_or(" ")
        .to_string()
}

/// Format an RFC3339 timestamp for compact display (show only date + HH:MM).
/// Returns empty string for empty input.
///
/// Char-safe (SQ-0638): a saved_at string is normally plain ASCII, but a
/// corrupt or hand-edited archive can carry anything, and a byte-offset slice
/// (`&ts[0..10]`) panics the moment a multibyte char sits before that byte
/// offset. Index by CHAR instead, and fall back to a safely-truncated raw
/// string for anything shorter than the fixed layout expects.
fn format_time(ts: &str) -> String {
    if ts.is_empty() {
        return String::new();
    }
    // "2026-06-18T12:34:56Z" → "2026-06-18 12:34"
    let chars: Vec<char> = ts.chars().collect();
    if chars.len() >= 16 {
        let date: String = chars[0..10].iter().collect();
        let time: String = chars[11..16].iter().collect();
        return format!("{} {}", date, time);
    }
    chars.into_iter().take(16).collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use crate::render::dialog::ButtonId;
    use crate::render::paneframe::BorderStyle;
    use crate::state::{AppState, SavesState};
    use crate::persist_files::SaveInfo;
    use std::path::PathBuf;

    fn dummy_save(name: &str, turns: u32, is_default: bool) -> SaveInfo {
        SaveInfo {
            path: PathBuf::from(format!("/tmp/{}.lanthorn", name)),
            name: name.to_string(),
            turns,
            saved_at: "2026-06-18T10:00:00Z".to_string(),
            location: None, score: None, is_default, trigger: crate::archive::SaveTrigger::HostState,
        }
    }

    fn dummy_qzl(name: &str, turns: u32) -> SaveInfo {
        SaveInfo {
            path: PathBuf::from(format!("/tmp/{}.qzl", name)),
            name: name.to_string(),
            turns,
            saved_at: "2026-06-18T10:00:00Z".to_string(),
            location: None, score: None, is_default: false,
            trigger: crate::archive::SaveTrigger::Ingame,
        }
    }

    #[test]
    fn draw_saves_shows_type_column_for_each_kind() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_saves(
            vec![
                dummy_save("state-slot", 10, false),
                dummy_qzl("game-slot", 20),
            ],
            0,
        );
        terminal.draw(|f| {
            draw_saves(&state, f.area(), f.buffer_mut(), &mut 0);
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("Type"), "Type column header should be present");
        assert!(content.contains("Game"), "an in-game save should show the 'Game' type");
        assert!(content.contains("State"), "a host Save State should show the 'State' type");
    }

    #[test]
    fn draw_saves_marks_portable_saves_and_only_those() {
        // SQ-0531: portability is driven off the trigger, not the extension —
        // an @save `.lanthorn` is portable, a host Save State is not. The ↗ glyph
        // and the themeable tint land on the portable row's Type cell only.
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut ingame_archive = dummy_save("from-at-save", 7, false);
        ingame_archive.trigger = crate::archive::SaveTrigger::Ingame;
        // Selection sits on neither row so both get their portability tint.
        let state = state_with_saves(vec![ingame_archive, dummy_save("host-slot", 8, false)], 9);
        terminal.draw(|f| {
            draw_saves(&state, f.area(), f.buffer_mut(), &mut 0);
        }).unwrap();
        let buf = terminal.backend().buffer().clone();
        let rows: Vec<String> = (0..buf.area.height)
            .map(|y| (0..buf.area.width)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default())
                .collect())
            .collect();
        let portable_row = rows.iter().find(|r| r.contains("from-at-save")).expect("in-game row drawn");
        let host_row = rows.iter().find(|r| r.contains("host-slot")).expect("host row drawn");
        assert!(portable_row.contains("Game↗"), "in-game save is marked portable: {portable_row:?}");
        assert!(!host_row.contains('↗'), "host Save State is NOT marked portable: {host_row:?}");
        assert!(host_row.contains("State"), "host Save State keeps the 'State' type");

        // The tint is themeable, not hard-coded: the portable Type cell picks up
        // the resolved `saves_portable` style rather than the plain row style.
        let want = state.colors.theme.get("saves_portable").style;
        let y = (0..buf.area.height)
            .find(|&y| (0..buf.area.width)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default())
                .collect::<String>()
                .contains("from-at-save"))
            .expect("row index");
        let tinted = (0..buf.area.width)
            .any(|x| buf.cell((x, y)).is_some_and(|c| c.symbol() == "↗" && c.style().fg == want.fg));
        assert!(tinted, "the ↗ marker uses the themeable saves_portable style");
    }

    fn state_with_saves(entries: Vec<SaveInfo>, selected: usize) -> AppState {
        let mut s = AppState::default();
        let mut scroll = crate::list_scroll::ListScroll::new();
        scroll.selected = selected;
        s.overlays.saves = Some(SavesState { entries, scroll });
        s
    }

    /// SQ-0639: `entries_area.bottom() == content.bottom()` whenever
    /// content.height > 1, so the old `footer_y < content.bottom()` guard was
    /// provably always false — no row was ever reserved and the footer text
    /// never rendered. It must now show up on a normal-sized modal.
    #[test]
    fn draw_saves_footer_hint_is_reachable() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_saves(vec![dummy_save("(default)", 0, true)], 0);
        terminal.draw(|f| {
            draw_saves(&state, f.area(), f.buffer_mut(), &mut 0);
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("Enter:load"), "the footer key hint must render, got: {content}");
    }

    #[test]
    fn draw_saves_scrollbar_and_paging_on_overflow() {
        use crate::input::{apply_action, Action};
        use mapper::mapper::Mapper;
        // More entries than the modal can show -> windowed list + scrollbar.
        let entries: Vec<SaveInfo> = (0..40).map(|i| dummy_save(&format!("slot-{i}"), i, false)).collect();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = state_with_saves(entries, 0);
        let mut vp = 0usize;
        terminal.draw(|f| {
            draw_saves(&state, f.area(), f.buffer_mut(), &mut vp);
        }).unwrap();
        assert!(vp > 0 && vp < 40, "entries should overflow the modal (vp={vp})");
        // SQ-0782: the thumb is a background fill, not a glyph.
        let thumb = crate::render::scroll::ScrollbarLook::from_theme(&state.colors.theme).thumb;
        let has_thumb = terminal.backend().buffer().content().iter().any(|c| c.bg == thumb);
        assert!(has_thumb, "a scrollbar thumb should be drawn when entries overflow");

        // PageDown advances the selection by ~one viewport (clamped/wrapped via nav).
        state.modal_list_viewport = vp;
        apply_action(Action::SavesPage(1), &mut state, &mut Mapper::default());
        let sel = state.overlays.saves.as_ref().unwrap().scroll.selected;
        assert!(sel >= vp.saturating_sub(1), "PageDown should advance ~one viewport, got {sel}");
    }

    #[test]
    fn draw_saves_shows_entry_names() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_saves(
            vec![
                dummy_save("(default)", 0, true),
                dummy_save("before-troll", 42, false),
            ],
            0,
        );
        terminal.draw(|f| {
            draw_saves(&state, f.area(), f.buffer_mut(), &mut 0);
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("(default)"), "default slot should be listed");
        assert!(content.contains("before-troll"), "named save should be listed");
    }

    #[test]
    fn draw_saves_labels_default_slot() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_saves(
            vec![dummy_save("(default)", 0, true)],
            0,
        );
        terminal.draw(|f| {
            draw_saves(&state, f.area(), f.buffer_mut(), &mut 0);
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("(default)"), "default slot must be labelled (default)");
    }

    #[test]
    fn draw_saves_shows_turn_count() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_saves(
            vec![dummy_save("after-troll", 99, false)],
            0,
        );
        terminal.draw(|f| {
            draw_saves(&state, f.area(), f.buffer_mut(), &mut 0);
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("99"), "turn count should be displayed");
    }

    #[test]
    fn draw_saves_selection_marker_on_active_row() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        // Two entries; selected = 1.
        let state = state_with_saves(
            vec![
                dummy_save("(default)", 0, true),
                dummy_save("chapter-2", 55, false),
            ],
            1,
        );
        terminal.draw(|f| {
            draw_saves(&state, f.area(), f.buffer_mut(), &mut 0);
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        // "chapter-2" should appear in the content with the marker
        assert!(content.contains("chapter-2"), "selected entry name should appear");
    }

    #[test]
    fn draw_saves_survives_a_terminal_too_narrow_for_the_type_column() {
        // The portability tint overdraws a fixed column offset; on a modal
        // narrower than that offset the cell clips to nothing rather than
        // painting outside the row (SQ-0531).
        for w in [24u16, 30, 40] {
            let backend = TestBackend::new(w, 12);
            let mut terminal = Terminal::new(backend).unwrap();
            let mut ingame = dummy_save("s", 1, false);
            ingame.trigger = crate::archive::SaveTrigger::Ingame;
            let state = state_with_saves(vec![ingame, dummy_save("t", 2, false)], 0);
            terminal.draw(|f| {
                draw_saves(&state, f.area(), f.buffer_mut(), &mut 0);
            }).unwrap();
        }
    }

    #[test]
    fn draw_saves_noop_when_closed() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = AppState::default(); // saves = None
        let before: Vec<_> = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().to_string())
            .collect();
        terminal.draw(|f| {
            draw_saves(&state, f.area(), f.buffer_mut(), &mut 0);
        }).unwrap();
        let after: Vec<_> = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().to_string())
            .collect();
        assert_eq!(before, after, "draw_saves should be a no-op when saves is None");
    }

    #[test]
    fn format_time_truncates_to_date_and_hhmm() {
        assert_eq!(super::format_time("2026-06-18T12:34:56Z"), "2026-06-18 12:34");
        assert_eq!(super::format_time(""), "");
        assert_eq!(super::format_time("2026-06-18T10:00:00Z"), "2026-06-18 10:00");
    }

    /// SQ-0638: a corrupt/hand-edited archive can put anything into `saved_at`.
    /// The old `&ts[0..10]`/`&ts[11..16]` byte slices panicked the instant a
    /// multibyte char sat before byte 16; this must degrade gracefully instead.
    #[test]
    fn format_time_is_char_safe_on_multibyte_and_short_strings() {
        // A multibyte prefix that would misalign any BYTE-offset slice at 10/16
        // (each '€' is 3 bytes, so byte 16 lands mid-character).
        let multibyte = "€€€€€€€€€€€€€€€€€€€€";
        // Must not panic, and must return *something* (a safe truncation).
        let out = super::format_time(multibyte);
        assert!(!out.is_empty());

        // Short strings (< 16 chars) fall back to a raw (safely-truncated) string
        // rather than panicking on an out-of-range slice.
        assert_eq!(super::format_time("hi"), "hi");
        assert_eq!(super::format_time("€€€"), "€€€");
    }

    #[test]
    fn draw_saves_shows_dialog_chrome() {
        // Render test: saves shows bordered titled chrome with [X] + [Done] and dialog_* colors.
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = state_with_saves(
            vec![dummy_save("(default)", 0, true)],
            0,
        );
        state.colors.dialog_box_style = BorderStyle::Single;
        let mut rects_out: Option<DialogRects> = None;
        terminal.draw(|f| {
            rects_out = draw_saves(&state, f.area(), f.buffer_mut(), &mut 0);
        }).unwrap();

        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();

        assert!(content.contains("Saves"), "title 'Saves' should be present");
        assert!(content.contains("Done"), "[Done] button should be visible");
        assert!(content.contains('✕'), "[X] close button should be visible");

        let rects = rects_out.expect("draw_saves should return DialogRects when open");
        assert!(rects.close.is_some(), "close rect should be present");
        assert_eq!(rects.buttons.len(), 1, "should have 1 button");
        let ids: Vec<ButtonId> = rects.buttons.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&ButtonId::Done));
    }
}
