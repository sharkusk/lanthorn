use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::dialog::{ButtonId, DialogButton, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::state::AppState;

// Minimum dimensions for the reset dialog.
const MIN_W: u16 = 36;
const MIN_H: u16 = 10;

// Dialog dimensions.
const DIALOG_W: u16 = 38;
const DIALOG_H: u16 = 12;

// ── ResetDialogRects ──────────────────────────────────────────────────────────

pub struct ResetDialogRects {
    pub area: Rect,
    pub close: Option<Rect>,
    /// Hit-rect for the "Also clear the map" checkbox.
    pub checkbox: Rect,
    /// Hit-rect for the "Delete saved progress" checkbox.
    pub checkbox_data: Rect,
    pub reset: Option<Rect>,
    pub cancel: Option<Rect>,
}

// ── draw_reset_dialog ─────────────────────────────────────────────────────────

/// Draw the reset-confirmation dialog centered over `area`.
///
/// Returns `None` when `state.overlays.reset_dialog` is false or the area is too small.
/// Returns `ResetDialogRects` with hit-rects for close, checkbox, and buttons.
pub fn draw_reset_dialog(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<ResetDialogRects> {
    if !state.overlays.reset_dialog {
        return None;
    }

    let modal_w = DIALOG_W.min(area.width.saturating_sub(4));
    let modal_h = DIALOG_H.min(area.height.saturating_sub(2));

    if modal_w < MIN_W || modal_h < MIN_H {
        return None;
    }

    let st = DialogStyle::from_colors(&state.colors);

    let buttons = &[
        DialogButton { id: ButtonId::Reset, label: "Reset" },
        DialogButton { id: ButtonId::Cancel, label: "Cancel" },
    ];
    // Focus ring: 0 = "clear map" checkbox, 1 = "delete data" checkbox,
    // 2 = Reset button, 3 = Cancel button. Only buttons highlight via DialogSpec;
    // a focused checkbox is highlighted inline below.
    let button_focus = state.overlays.dialog_focus.checked_sub(2);
    let spec = DialogSpec {
        title: "Reset game?",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Reset),
        focus: button_focus,
        field: None,
    };

    let rects = draw_dialog(buf, area, &spec, &st);
    let content = rects.content;

    // Draw content into the content area.
    // Row 0: body line
    if content.height >= 1 {
        let body_style = state.colors.theme.get("dialog.background").style;
        crate::render::draw_str_clipped(
            buf,
            content.x,
            content.y,
            "Restart the story from the beginning.",
            body_style,
            content,
        );
    }

    // Row 1: blank (skip)

    // Helper: draw one checkbox row, highlighting it when focused. Returns its
    // hit-rect (a zero-size rect when the row is off the content area).
    let draw_checkbox = |buf: &mut Buffer, y: u16, checked: bool, focused: bool, text: &str| -> Rect {
        if y >= content.bottom() {
            return Rect::new(content.x, content.y, 0, 0);
        }
        let check = if checked { "[x]" } else { "[ ]" };
        let label = format!("{} {}", check, text);
        let style = if focused { state.colors.theme.get("dialog.button:active").style } else { state.colors.theme.get("dialog.background").style };
        crate::render::draw_str_clipped(buf, content.x, y, &label, style, content);
        let rect_w = (label.chars().count() as u16).min(content.width);
        Rect::new(content.x, y, rect_w, 1)
    };

    // Row 2: "Also clear the map" checkbox (focus 0).
    let checkbox_rect = draw_checkbox(
        buf,
        content.y + 2,
        state.overlays.reset_clear_map,
        state.overlays.dialog_focus == 0,
        "Also clear the map",
    );

    // Row 3: "Delete saved progress" checkbox (focus 1).
    let checkbox_data_rect = draw_checkbox(
        buf,
        content.y + 3,
        state.overlays.reset_delete_data,
        state.overlays.dialog_focus == 1,
        "Delete saved progress",
    );

    // Map button rects from draw_dialog output.
    let reset_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Reset).map(|(_, r)| *r);
    let cancel_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Cancel).map(|(_, r)| *r);

    Some(ResetDialogRects {
        area: rects.area,
        close: rects.close,
        checkbox: checkbox_rect,
        checkbox_data: checkbox_data_rect,
        reset: reset_rect,
        cancel: cancel_rect,
    })
}

// ── Reset dialog keyboard routing ─────────────────────────────────────────────

/// Action to take when a key is pressed while the reset dialog is open.
pub enum ResetDialogAction {
    None,
    ToggleClearMap,
    ToggleDeleteData,
    Confirm,
    Cancel,
}

/// Map a key code to a ResetDialogAction (focus-agnostic accelerators only).
/// Esc and 'c' cancel; Enter and 'r' confirm; Space toggles the clear-map box.
#[cfg_attr(not(all(test, feature = "t-render")), allow(dead_code))]
fn reset_dialog_key(code: crossterm::event::KeyCode) -> ResetDialogAction {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Esc | KeyCode::Char('c') => ResetDialogAction::Cancel,
        KeyCode::Enter | KeyCode::Char('r') => ResetDialogAction::Confirm,
        KeyCode::Char(' ') => ResetDialogAction::ToggleClearMap,
        _ => ResetDialogAction::None,
    }
}

/// Reset-dialog keys with focus. Tab/BackTab are handled by the caller (which
/// mutates dialog_focus over a 4-slot ring: 0 = clear-map checkbox, 1 = delete-data
/// checkbox, 2 = Reset, 3 = Cancel). Space toggles the focused checkbox; Enter
/// activates the focused button (or confirms when a checkbox is focused); 'r'/'c'
/// stay as confirm/cancel accelerators.
pub fn reset_dialog_key_focused(code: crossterm::event::KeyCode, focus: usize) -> ResetDialogAction {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Esc | KeyCode::Char('c') => ResetDialogAction::Cancel,
        KeyCode::Char('r') => ResetDialogAction::Confirm,
        KeyCode::Char(' ') => match focus {
            0 => ResetDialogAction::ToggleClearMap,
            1 => ResetDialogAction::ToggleDeleteData,
            _ => ResetDialogAction::None,
        },
        KeyCode::Enter => match focus {
            3 => ResetDialogAction::Cancel,
            _ => ResetDialogAction::Confirm, // checkboxes and Reset (focus 2) confirm
        },
        _ => ResetDialogAction::None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;

    #[test]
    fn reset_dialog_renders_title_checkbox_and_buttons() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut state = crate::state::AppState::default();
        state.overlays.reset_dialog = true;
        state.overlays.reset_clear_map = false;
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal.draw(|f| { rects = draw_reset_dialog(&state, f.area(), f.buffer_mut()); }).unwrap();
        let r = rects.expect("dialog should render when reset_dialog is set");
        assert!(r.close.is_some() && r.reset.is_some() && r.cancel.is_some());
        let all: String = terminal.backend().buffer().content().iter()
            .flat_map(|c| c.symbol().chars()).collect();
        assert!(all.contains("Reset game?"), "title present");
        assert!(all.contains("Also clear the map"), "clear-map checkbox label present");
        assert!(all.contains("Delete saved progress"), "delete-data checkbox label present");
        assert!(all.contains("[ ]"), "unchecked box shown when reset flags are false");
    }

    // ── reset_dialog_key_mapping ──────────────────────────────────────────────

    #[test]
    fn reset_dialog_key_mapping() {
        use crossterm::event::KeyCode;
        assert!(matches!(reset_dialog_key(KeyCode::Esc), ResetDialogAction::Cancel));
        assert!(matches!(reset_dialog_key(KeyCode::Char('c')), ResetDialogAction::Cancel));
        assert!(matches!(reset_dialog_key(KeyCode::Enter), ResetDialogAction::Confirm));
        assert!(matches!(reset_dialog_key(KeyCode::Char('r')), ResetDialogAction::Confirm));
        assert!(matches!(reset_dialog_key(KeyCode::Char(' ')), ResetDialogAction::ToggleClearMap));
    }

    // ── reset_dialog_tab_then_enter_fires_focused ─────────────────────────────

    #[test]
    fn reset_dialog_tab_then_enter_fires_focused() {
        use crossterm::event::KeyCode;
        // Focus ring (4): 0 = clear-map checkbox, 1 = delete-data checkbox,
        // 2 = Reset, 3 = Cancel. Default focus 0.
        let mut focus = 0usize;
        // Space on focus 0 toggles the clear-map checkbox.
        assert!(matches!(reset_dialog_key_focused(KeyCode::Char(' '), focus), ResetDialogAction::ToggleClearMap));
        // Tab -> focus 1 (delete-data checkbox); Space toggles delete-data.
        focus = crate::input::cycle_focus(focus, 4, 1);
        assert_eq!(focus, 1);
        assert!(matches!(reset_dialog_key_focused(KeyCode::Char(' '), focus), ResetDialogAction::ToggleDeleteData));
        // Enter on a focused checkbox confirms.
        assert!(matches!(reset_dialog_key_focused(KeyCode::Enter, focus), ResetDialogAction::Confirm));
        // Tab to focus 3 (Cancel); Enter cancels.
        focus = crate::input::cycle_focus(focus, 4, 1); // 2 = Reset
        focus = crate::input::cycle_focus(focus, 4, 1); // 3 = Cancel
        assert_eq!(focus, 3);
        assert!(matches!(reset_dialog_key_focused(KeyCode::Enter, focus), ResetDialogAction::Cancel));
        // Enter on focus 2 (Reset) confirms.
        assert!(matches!(reset_dialog_key_focused(KeyCode::Enter, 2), ResetDialogAction::Confirm));
    }
}
