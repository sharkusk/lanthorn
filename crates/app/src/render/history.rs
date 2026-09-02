//! Rewind/replay history modal overlay.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use crate::render::dialog::{
    ButtonId, DialogButton, DialogRects, DialogSpec, DialogStyle, Placement, draw_dialog,
};
use crate::state::AppState;

/// Draw the replay/rewind modal centered over `area`: a turn list (turn# +
/// command) with the selected turn highlighted, the selected turn's transcript,
/// and a transport footer. Does nothing when `state.overlays.replay` is `None`.
/// Returns `Some(DialogRects)` when drawn (for mouse hit-testing).
pub fn draw_history(
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
    vp_out: &mut usize,
) -> Option<DialogRects> {
    let replay = state.overlays.replay.as_ref()?;
    if state.history.is_empty() {
        return None;
    }

    let modal_w = 64u16.min(area.width.saturating_sub(4));
    // up to 12 list rows + a transcript preview (separator + up to 4 lines) +
    // 1 footer + 2 header/sep + chrome.
    let list_rows = (state.history.len() as u16).min(12);
    let transcript_rows = 5u16; // 1 separator + up to 4 transcript lines
    let modal_h = (list_rows + transcript_rows + 6).min(area.height.saturating_sub(2));
    if modal_w < 24 || modal_h < 8 {
        return None;
    }

    let st = DialogStyle::from_colors(&state.colors);
    let buttons = &[DialogButton { id: ButtonId::Done, label: "Done" }];
    let spec = DialogSpec {
        title: "Replay",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Done),
        focus: Some(state.overlays.dialog_focus),
        field: None,
    };
    let rects = draw_dialog(buf, area, &spec, &st);
    let content = rects.content;

    let normal = state.colors.theme.get("dialog.background").style;
    let selected_style = state.colors.theme.get("dialog.list_selected").style;

    let dim_style = normal.patch(state.colors.theme.get("dialog.list_footer").style);

    // Partition the content: footer (1 row) at the bottom, the selected turn's
    // transcript just above it, and the turn list filling the rest at the top.
    let footer_y = content.bottom().saturating_sub(1);
    let tr_h = transcript_rows.min(content.height.saturating_sub(2));
    let tr_top = footer_y.saturating_sub(tr_h);

    // ── Turn list ────────────────────────────────────────────────────────────
    // Window the list via the (idx-synced) ListScroll so the selection stays
    // visible; reserve a 1-col gutter for the scrollbar when it overflows.
    let total = state.history.len();
    let visible = tr_top.saturating_sub(content.y) as usize;
    *vp_out = visible;
    let scrollbar_visible =
        crate::render::scroll::needs_scrollbar(total, visible) && content.width >= 2;
    let row_w = if scrollbar_visible { content.width.saturating_sub(1) } else { content.width };
    let row_area = Rect::new(content.x, content.y, row_w, visible as u16);

    let first = replay.scroll.display_offset();
    for (row, i) in (first..total).take(visible).enumerate() {
        let row_y = content.y + row as u16;
        if row_y >= tr_top { break; }
        let rec = &state.history[i];
        let style = if i == replay.idx { selected_style } else { normal };
        for col in row_area.x..row_area.right() {
            if let Some(cell) = buf.cell_mut((col, row_y)) {
                cell.set_symbol(" ").set_style(style);
            }
        }
        let marker = if i == replay.idx { ">" } else { " " };
        let cmd_trunc: String = rec.command.chars().take(40).collect();
        let map_tag = if rec.map_snapshot.is_some() { "*" } else { " " };
        let line = format!("{} T{:<5} {} {}", marker, rec.turn, map_tag, cmd_trunc);
        crate::render::draw_str_clipped(buf, row_area.x, row_y, &line, style, row_area);
    }

    if scrollbar_visible {
        let sb_area = Rect::new(content.right().saturating_sub(1), content.y, 1, visible as u16);
        crate::render::scroll::draw_scrollbar(
            buf,
            sb_area,
            total,
            visible,
            replay.scroll.target_offset(),
            crate::render::scroll::ScrollbarLook::from_theme(&state.colors.theme),
        );
    }

    // ── Selected turn's transcript ───────────────────────────────────────────
    if tr_h > 0 {
        let rec = &state.history[replay.idx];
        let sep = format!("─ turn {} text ─", rec.turn);
        crate::render::draw_str_clipped(buf, content.x, tr_top, &sep, dim_style, content);
        let text_top = tr_top + 1;
        let text_h = footer_y.saturating_sub(text_top) as usize;
        let lines: Vec<&str> = rec.transcript.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.is_empty() && text_h > 0 {
            crate::render::draw_str_clipped(buf, content.x, text_top, "(no output)", dim_style, content);
        } else {
            for (row, line) in lines.iter().take(text_h).enumerate() {
                let y = text_top + row as u16;
                crate::render::draw_str_clipped(buf, content.x, y, line, normal, content);
            }
        }
    }

    // ── Footer ───────────────────────────────────────────────────────────────
    if footer_y >= content.y {
        let footer = "←/→:step  Space:play  Enter/r:resume  Esc:close";
        crate::render::draw_str_clipped(buf, content.x, footer_y, footer, dim_style, content);
    }

    Some(rects)
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use crate::state::{AppState, ReplayState};
    use mapper::mapper::Mapper;

    #[test]
    fn draw_history_renders_when_open_and_noops_when_closed() {
        let mut state = AppState::default();
        let m = Mapper::default();
        for t in 1..=2 {
            crate::history::record_turn(&mut state.history, t, "go north", vec![t as u8], &m, false, "Forest");
        }

        // Closed → None.
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| {
            let area = f.area();
            let out = draw_history(&state, area, f.buffer_mut(), &mut 0);
            assert!(out.is_none(), "draw_history is a no-op when replay is None");
        }).unwrap();

        // Open → Some, and the selected turn's command AND transcript appear.
        state.overlays.replay = Some(ReplayState::new(1));
        let mut term2 = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut rects: Option<DialogRects> = None;
        term2.draw(|f| {
            let area = f.area();
            rects = draw_history(&state, area, f.buffer_mut(), &mut 0);
        }).unwrap();
        assert!(rects.is_some(), "draw_history returns rects when open");

        // Scan the rendered buffer for the turn command and its transcript text.
        let buf = term2.backend().buffer();
        let screen: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(screen.contains("go north"), "turn command should render");
        assert!(screen.contains("Forest"), "selected turn's transcript should render");
    }

    #[test]
    fn draw_history_scrollbar_when_turns_overflow() {
        let mut state = AppState::default();
        let m = Mapper::default();
        for t in 1..=40 {
            crate::history::record_turn(&mut state.history, t, "go north", vec![t as u8], &m, false, "Forest");
        }
        state.config.animation.enabled = false; // settle the offset instantly
        state.overlays.replay = Some(ReplayState::new(state.history.len() - 1));
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        // First render captures the list viewport.
        let mut vp = 0usize;
        term.draw(|f| { let a = f.area(); draw_history(&state, a, f.buffer_mut(), &mut vp); }).unwrap();
        assert!(vp > 0 && vp < 40, "turns should overflow the list band (vp={vp})");
        // Sync the idx-driven scroll (run loop does this) and re-render.
        {
            let anim = state.config.animation.clone();
            let r = state.overlays.replay.as_mut().unwrap();
            r.scroll.len(40);
            r.scroll.select(r.idx, vp, &anim);
        }
        term.draw(|f| { let a = f.area(); draw_history(&state, a, f.buffer_mut(), &mut vp); }).unwrap();
        // SQ-0782: the thumb is a background fill, not a glyph.
        let thumb = crate::render::scroll::ScrollbarLook::from_theme(&state.colors.theme).thumb;
        let has_thumb = term.backend().buffer().content().iter().any(|c| c.bg == thumb);
        assert!(has_thumb, "a scrollbar thumb should be drawn when turns overflow");
    }
}
