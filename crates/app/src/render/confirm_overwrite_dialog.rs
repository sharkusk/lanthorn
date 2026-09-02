//! The "overwrite existing save?" confirm dialog (SQ-0648).
//!
//! A two-button confirm on the common dialog chrome (no text field), following
//! the confirm-delete pattern almost verbatim: Overwrite replaces the existing
//! file, Cancel leaves it untouched. It exists because two different typed
//! save names can slugify to the same filename ("Before Troll" and "before,
//! troll!" both land on `before-troll.lanthorn`) — writing straight over the
//! target silently destroyed whichever save was there first. The body names
//! the EXISTING save, not the name just typed, so a cross-name collision is
//! visible instead of looking like a same-name re-save.

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

// ── ConfirmOverwriteDialogRects ─────────────────────────────────────────────────

pub struct ConfirmOverwriteDialogRects {
    pub area: Rect,
    pub close: Option<Rect>,
    pub overwrite: Option<Rect>,
    pub cancel: Option<Rect>,
}

// ── draw_confirm_overwrite_dialog ───────────────────────────────────────────────

/// Draw the overwrite-confirmation dialog centered over `area`.
///
/// Returns `None` when `state.overlays.confirm_overwrite_save` is `None` or the
/// area is too small. Focus starts on Cancel (the safe default); Overwrite is
/// reachable by Tab.
pub fn draw_confirm_overwrite_dialog(
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
) -> Option<ConfirmOverwriteDialogRects> {
    let pending = state.overlays.confirm_overwrite_save.as_ref()?;

    let modal_w = DIALOG_W.min(area.width.saturating_sub(4));
    let modal_h = DIALOG_H.min(area.height.saturating_sub(2));
    if modal_w < MIN_W || modal_h < MIN_H {
        return None;
    }

    let st = DialogStyle::from_colors(&state.colors);

    let buttons = &[
        DialogButton { id: ButtonId::Ok, label: "Overwrite" },
        DialogButton { id: ButtonId::Cancel, label: "Cancel" },
    ];
    let spec = DialogSpec {
        title: "Overwrite this save?",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Cancel),
        focus: Some(state.overlays.dialog_focus),
        field: None,
    };

    let rects = draw_dialog(buf, area, &spec, &st);
    let content = rects.content;

    // Body: the EXISTING save's display name (clipped to the content width) —
    // not the name just typed, so a cross-name slugify collision reads as
    // exactly that instead of looking like a harmless same-name re-save.
    if content.height >= 1 {
        let body_style = state.colors.theme.get("dialog.background").style;
        let quoted = format!("\"{}\"", pending.existing_name);
        crate::render::draw_str_clipped(buf, content.x, content.y, &quoted, body_style, content);
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

    let overwrite_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Ok).map(|(_, r)| *r);
    let cancel_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Cancel).map(|(_, r)| *r);

    Some(ConfirmOverwriteDialogRects {
        area: rects.area,
        close: rects.close,
        overwrite: overwrite_rect,
        cancel: cancel_rect,
    })
}

// ── Confirm-overwrite keyboard routing ────────────────────────────────────────

/// Action to take when a key is pressed while the confirm-overwrite dialog is open.
pub enum ConfirmOverwriteAction {
    None,
    Confirm,
    Cancel,
}

/// Map a key to a `ConfirmOverwriteAction` given button focus (0 = Overwrite, 1 =
/// Cancel). Tab/BackTab are handled by the caller (which mutates dialog_focus);
/// this maps Enter to the focused button and keeps the y/n accelerators (mirrors
/// `confirm_delete_key_focused`).
pub fn confirm_overwrite_key_focused(code: crossterm::event::KeyCode, focus: usize) -> ConfirmOverwriteAction {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Esc | KeyCode::Char('n') => ConfirmOverwriteAction::Cancel,
        KeyCode::Char('y') => ConfirmOverwriteAction::Confirm,
        KeyCode::Enter | KeyCode::Char(' ') => match focus {
            0 => ConfirmOverwriteAction::Confirm, // focus 0 = Overwrite
            _ => ConfirmOverwriteAction::Cancel,   // focus 1 = Cancel (default)
        },
        _ => ConfirmOverwriteAction::None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use crate::state::{ConfirmOverwriteSave, PendingOverwrite};
    use crossterm::event::KeyCode;

    fn pending(existing_name: &str) -> ConfirmOverwriteSave {
        ConfirmOverwriteSave {
            path: std::path::PathBuf::from("/saves/before-troll.lanthorn"),
            existing_name: existing_name.to_string(),
            pending: PendingOverwrite::SaveAs,
        }
    }

    #[test]
    fn confirm_overwrite_renders_title_and_existing_name() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut state = AppState::default();
        state.overlays.confirm_overwrite_save = Some(pending("Before Troll"));
        state.overlays.dialog_focus = 1;
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal
            .draw(|f| rects = draw_confirm_overwrite_dialog(&state, f.area(), f.buffer_mut()))
            .unwrap();
        let r = rects.expect("dialog renders when confirm_overwrite_save is set");
        assert!(r.overwrite.is_some() && r.cancel.is_some());
        let all: String = terminal.backend().buffer().content().iter()
            .flat_map(|c| c.symbol().chars()).collect();
        assert!(all.contains("Overwrite this save?"), "title present");
        // The EXISTING save's name is shown, which is what makes a cross-name
        // slugify collision ("before, troll!" -> the same file as "Before
        // Troll") visible instead of reading as a harmless same-name re-save.
        assert!(all.contains("Before Troll"), "existing save's display name present");
    }

    #[test]
    fn confirm_overwrite_returns_none_when_closed() {
        use ratatui::{backend::TestBackend, Terminal};
        let state = AppState::default(); // confirm_overwrite_save = None
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal
            .draw(|f| rects = draw_confirm_overwrite_dialog(&state, f.area(), f.buffer_mut()))
            .unwrap();
        assert!(rects.is_none());
    }

    #[test]
    fn confirm_overwrite_key_mapping() {
        // Accelerators.
        assert!(matches!(confirm_overwrite_key_focused(KeyCode::Char('y'), 1), ConfirmOverwriteAction::Confirm));
        assert!(matches!(confirm_overwrite_key_focused(KeyCode::Char('n'), 0), ConfirmOverwriteAction::Cancel));
        assert!(matches!(confirm_overwrite_key_focused(KeyCode::Esc, 0), ConfirmOverwriteAction::Cancel));
        // Enter follows focus: 0 = Overwrite, 1 = Cancel (the safe default).
        assert!(matches!(confirm_overwrite_key_focused(KeyCode::Enter, 0), ConfirmOverwriteAction::Confirm));
        assert!(matches!(confirm_overwrite_key_focused(KeyCode::Enter, 1), ConfirmOverwriteAction::Cancel));
        assert!(matches!(confirm_overwrite_key_focused(KeyCode::Char('x'), 0), ConfirmOverwriteAction::None));
    }
}
