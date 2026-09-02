//! The "delete this save?" confirm dialog (SQ-0307).
//!
//! A two-button confirm on the common dialog chrome (no text field), following
//! the reset/quit-dialog pattern. It replaces the former bottom-bar y/n
//! `ConfirmDeleteSave` prompt: Delete removes the selected save, Cancel keeps it.
//! Key routing is co-located here; the run-loop intercept in `main.rs` drives it.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::dialog::{ButtonId, DialogButton, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::state::AppState;

// Minimum dimensions for the confirm dialog.
const MIN_W: u16 = 34;
const MIN_H: u16 = 8;

// Dialog dimensions.
const DIALOG_W: u16 = 46;
const DIALOG_H: u16 = 9;

// ── ConfirmDeleteDialogRects ──────────────────────────────────────────────────

pub struct ConfirmDeleteDialogRects {
    pub area: Rect,
    pub close: Option<Rect>,
    pub delete: Option<Rect>,
    pub cancel: Option<Rect>,
}

// ── draw_confirm_delete_dialog ────────────────────────────────────────────────

/// Draw the delete-confirmation dialog centered over `area`.
///
/// Returns `None` when `state.overlays.confirm_delete_save` is `None` or the area is too
/// small. Focus starts on Cancel (the safe default); Delete is reachable by Tab.
pub fn draw_confirm_delete_dialog(
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
) -> Option<ConfirmDeleteDialogRects> {
    let path = state.overlays.confirm_delete_save.as_ref()?;

    let modal_w = DIALOG_W.min(area.width.saturating_sub(4));
    let modal_h = DIALOG_H.min(area.height.saturating_sub(2));
    if modal_w < MIN_W || modal_h < MIN_H {
        return None;
    }

    let st = DialogStyle::from_colors(&state.colors);

    let buttons = &[
        DialogButton { id: ButtonId::Ok, label: "Delete" },
        DialogButton { id: ButtonId::Cancel, label: "Cancel" },
    ];
    let spec = DialogSpec {
        title: "Delete this save?",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Cancel),
        focus: Some(state.overlays.dialog_focus),
        field: None,
    };

    let rects = draw_dialog(buf, area, &spec, &st);
    let content = rects.content;

    // Body: the save file name being deleted (clipped to the content width).
    if content.height >= 1 {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("this save");
        let body_style = state.colors.theme.get("dialog.background").style;
        crate::render::draw_str_clipped(buf, content.x, content.y, name, body_style, content);
        if content.height >= 3 {
            crate::render::draw_str_clipped(
                buf,
                content.x,
                content.y + 2,
                "This cannot be undone.",
                body_style,
                content,
            );
        }
    }

    let delete_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Ok).map(|(_, r)| *r);
    let cancel_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Cancel).map(|(_, r)| *r);

    Some(ConfirmDeleteDialogRects {
        area: rects.area,
        close: rects.close,
        delete: delete_rect,
        cancel: cancel_rect,
    })
}

// ── Confirm-delete keyboard routing ───────────────────────────────────────────

/// Action to take when a key is pressed while the confirm-delete dialog is open.
pub enum ConfirmDeleteAction {
    None,
    Confirm,
    Cancel,
}

/// Map a key to a `ConfirmDeleteAction` given button focus (0 = Delete, 1 =
/// Cancel). Tab/BackTab are handled by the caller (which mutates dialog_focus);
/// this maps Enter to the focused button and keeps the y/n accelerators.
pub fn confirm_delete_key_focused(code: crossterm::event::KeyCode, focus: usize) -> ConfirmDeleteAction {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Esc | KeyCode::Char('n') => ConfirmDeleteAction::Cancel,
        KeyCode::Char('y') => ConfirmDeleteAction::Confirm,
        KeyCode::Enter | KeyCode::Char(' ') => match focus {
            0 => ConfirmDeleteAction::Confirm, // focus 0 = Delete
            _ => ConfirmDeleteAction::Cancel,  // focus 1 = Cancel (default)
        },
        _ => ConfirmDeleteAction::None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;

    #[test]
    fn confirm_delete_renders_title_body_and_buttons() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut state = AppState::default();
        state.overlays.confirm_delete_save = Some(std::path::PathBuf::from("/saves/zork-hero.lanthorn"));
        state.overlays.dialog_focus = 1;
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal
            .draw(|f| rects = draw_confirm_delete_dialog(&state, f.area(), f.buffer_mut()))
            .unwrap();
        let r = rects.expect("dialog renders when confirm_delete_save is set");
        assert!(r.delete.is_some() && r.cancel.is_some());
        let all: String = terminal.backend().buffer().content().iter()
            .flat_map(|c| c.symbol().chars()).collect();
        assert!(all.contains("Delete this save?"), "title present");
        assert!(all.contains("zork-hero.lanthorn"), "save name present");
    }

    #[test]
    fn confirm_delete_returns_none_when_closed() {
        use ratatui::{backend::TestBackend, Terminal};
        let state = AppState::default(); // confirm_delete_save = None
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal
            .draw(|f| rects = draw_confirm_delete_dialog(&state, f.area(), f.buffer_mut()))
            .unwrap();
        assert!(rects.is_none());
    }

    #[test]
    fn confirm_delete_key_mapping() {
        // Accelerators.
        assert!(matches!(confirm_delete_key_focused(KeyCode::Char('y'), 1), ConfirmDeleteAction::Confirm));
        assert!(matches!(confirm_delete_key_focused(KeyCode::Char('n'), 0), ConfirmDeleteAction::Cancel));
        assert!(matches!(confirm_delete_key_focused(KeyCode::Esc, 0), ConfirmDeleteAction::Cancel));
        // Enter follows focus: 0 = Delete, 1 = Cancel (the safe default).
        assert!(matches!(confirm_delete_key_focused(KeyCode::Enter, 0), ConfirmDeleteAction::Confirm));
        assert!(matches!(confirm_delete_key_focused(KeyCode::Enter, 1), ConfirmDeleteAction::Cancel));
        assert!(matches!(confirm_delete_key_focused(KeyCode::Char('x'), 0), ConfirmDeleteAction::None));
    }
}
