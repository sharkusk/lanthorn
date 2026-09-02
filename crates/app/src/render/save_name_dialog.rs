use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use crate::render::dialog::{
    ButtonId, DialogButton, DialogField, DialogSpec, DialogStyle, Placement, draw_dialog,
};
use crate::state::AppState;

// Minimum dimensions for the save-name dialog.
const MIN_W: u16 = 34;
const MIN_H: u16 = 7;

// Dialog dimensions.
const DIALOG_W: u16 = 44;
const DIALOG_H: u16 = 7;

// ── SaveNameDialogRects ───────────────────────────────────────────────────────

pub struct SaveNameDialogRects {
    pub area: Rect,
    pub close: Option<Rect>,
    /// Hit-rect for the text field row.
    pub field: Option<Rect>,
    pub save: Option<Rect>,
    pub cancel: Option<Rect>,
}

// ── draw_save_name_dialog ─────────────────────────────────────────────────────

/// Draw the save-name dialog (a common-dialog with a caret text field) centered
/// over `area` (the graphics-free dialog region). Returns `None` when the dialog
/// is closed or the area is too small.
///
/// Focus ring: 0 = text field, 1 = Save, 2 = Cancel. Only the buttons highlight
/// via `DialogSpec.focus`; the field's caret is shown when it is focused and active.
pub fn draw_save_name_dialog(
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
) -> Option<SaveNameDialogRects> {
    let dlg = state.overlays.save_name_dialog.as_ref()?;

    let modal_w = DIALOG_W.min(area.width.saturating_sub(4));
    let modal_h = DIALOG_H.min(area.height.saturating_sub(2));
    if modal_w < MIN_W || modal_h < MIN_H {
        return None;
    }

    let st = DialogStyle::from_colors(&state.colors);

    let buttons = &[
        DialogButton { id: ButtonId::Save, label: "Save" },
        DialogButton { id: ButtonId::Cancel, label: "Cancel" },
    ];
    // Buttons occupy focus slots 1 and 2; slot 0 is the field.
    let button_focus = state.overlays.dialog_focus.checked_sub(1);

    // The caret shows only while the field is focused and being edited; the
    // placeholder (default) is dimmed until adopted.
    let field_focused = state.overlays.dialog_focus == 0;
    let field = DialogField {
        label: "Name: ",
        value: &dlg.field.value,
        cursor: dlg.field.cursor,
        show_caret: field_focused && dlg.active,
        dim: !dlg.active,
        text_style: state.colors.theme.get("dialog.background").style,
        dim_style: state.colors.theme.get("dialog.background").style.add_modifier(Modifier::DIM),
        caret_style: state.colors.theme.get("dialog.background").style.add_modifier(Modifier::REVERSED),
    };

    let spec = DialogSpec {
        title: if dlg.ingame { "Save game as" } else { "Save State as" },
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Save),
        focus: button_focus,
        field: Some(field),
    };

    let rects = draw_dialog(buf, area, &spec, &st);

    let save_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Save).map(|(_, r)| *r);
    let cancel_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Cancel).map(|(_, r)| *r);

    Some(SaveNameDialogRects {
        area: rects.area,
        close: rects.close,
        field: rects.field,
        save: save_rect,
        cancel: cancel_rect,
    })
}

// ── Save-name dialog keyboard routing ─────────────────────────────────────────

/// What a key press resolves to in the save-name dialog.
pub enum SaveNameAction {
    None,
    Save,
    Cancel,
}

/// Apply one key to the save-name dialog's field state machine and focus ring
/// (0 = text field, 1 = Save, 2 = Cancel). Mutates `dlg.field`/`dlg.active` in
/// place and returns the resolved action plus the new focus slot. Callers gate
/// out Ctrl/Alt-modified printable chars before calling.
///
/// Field-focused behavior (see SQ-0289 table): a greyed placeholder (`active =
/// false`) is adopted for editing by Tab/→/Home/End/←/Backspace/Delete (without
/// advancing focus for Tab/→); typing a printable char starts fresh; Enter on the
/// placeholder saves the default; in active mode Enter saves a non-empty value and
/// reverts an empty one to the placeholder.
pub fn save_name_dialog_key(
    code: crossterm::event::KeyCode,
    dlg: &mut crate::state::SaveNameDialog,
    focus: usize,
) -> (SaveNameAction, usize) {
    use crossterm::event::KeyCode;
    if focus == 0 {
        // ── Text field focused ──
        match code {
            KeyCode::Esc => return (SaveNameAction::Cancel, focus),
            KeyCode::Enter => {
                if dlg.active && dlg.field.value.is_empty() {
                    dlg.active = false; // empty: revert to placeholder
                    return (SaveNameAction::None, focus);
                }
                return (SaveNameAction::Save, focus); // placeholder saves default
            }
            KeyCode::BackTab => return (SaveNameAction::None, 2), // reverse-wrap to Cancel
            KeyCode::Tab => {
                if dlg.active {
                    return (SaveNameAction::None, 1); // advance to Save
                }
                dlg.active = true; // adopt default for editing
                dlg.field.end();
            }
            KeyCode::Right => {
                if dlg.active {
                    dlg.field.right();
                } else {
                    dlg.active = true; // adopt default
                    dlg.field.end();
                }
            }
            KeyCode::Left => {
                if !dlg.active {
                    dlg.active = true;
                    dlg.field.end();
                }
                dlg.field.left();
            }
            KeyCode::Home => {
                dlg.active = true;
                dlg.field.home();
            }
            KeyCode::End => {
                dlg.active = true;
                dlg.field.end();
            }
            KeyCode::Backspace => {
                if !dlg.active {
                    dlg.active = true;
                    dlg.field.end();
                }
                dlg.field.backspace();
            }
            KeyCode::Delete => {
                if !dlg.active {
                    dlg.active = true;
                    dlg.field.end();
                }
                dlg.field.delete();
            }
            KeyCode::Char(c) => {
                if dlg.active {
                    dlg.field.insert(c);
                } else {
                    // Typing on the placeholder starts fresh.
                    dlg.field.set(String::new(), false);
                    dlg.field.insert(c);
                    dlg.active = true;
                }
            }
            _ => {}
        }
        (SaveNameAction::None, focus)
    } else {
        // ── Save / Cancel button focused ──
        match code {
            KeyCode::Esc => (SaveNameAction::Cancel, focus),
            KeyCode::Enter | KeyCode::Char(' ') => {
                if focus == 1 {
                    (SaveNameAction::Save, focus)
                } else {
                    (SaveNameAction::Cancel, focus)
                }
            }
            KeyCode::Tab | KeyCode::Right => (SaveNameAction::None, crate::input::cycle_focus(focus, 3, 1)),
            KeyCode::BackTab | KeyCode::Left => (SaveNameAction::None, crate::input::cycle_focus(focus, 3, -1)),
            _ => (SaveNameAction::None, focus),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;

    #[test]
    fn save_name_dialog_renders_field_and_buttons_in_area() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut state = crate::state::AppState::default();
        state.overlays.save_name_dialog =
            Some(crate::state::SaveNameDialog::new("2026-07-13 1432".to_string(), false));
        state.overlays.dialog_focus = 0;
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal
            .draw(|f| rects = draw_save_name_dialog(&state, f.area(), f.buffer_mut()))
            .unwrap();
        let r = rects.expect("dialog renders when save_name_dialog is set");
        assert!(r.close.is_some() && r.save.is_some() && r.cancel.is_some());
        assert!(r.field.is_some(), "field rect recorded for hit-testing");
        // The dialog is confined to the passed area (graphics-safe): its frame sits
        // inside the terminal, not painted over a story/graphics pane out of bounds.
        assert!(r.area.right() <= 60 && r.area.bottom() <= 20);
        let all: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .flat_map(|c| c.symbol().chars())
            .collect();
        assert!(all.contains("Save State as"), "title present");
        assert!(all.contains("Name:"), "field label present");
        assert!(all.contains("2026-07-13 1432"), "default name prefilled");
    }

    // ── SQ-0289: save-name dialog field state machine ─────────────────────────

    use crossterm::event::KeyCode;
    use crate::state::SaveNameDialog;

    fn sn_dialog() -> SaveNameDialog {
        SaveNameDialog::new("2026-07-13 1432".to_string(), false)
    }

    #[test]
    fn sn_placeholder_adopts_default_on_tab_and_right_without_advancing_focus() {
        for code in [KeyCode::Tab, KeyCode::Right] {
            let mut d = sn_dialog();
            let (act, focus) = save_name_dialog_key(code, &mut d, 0);
            assert!(matches!(act, SaveNameAction::None));
            assert_eq!(focus, 0, "adopting the default must not advance focus");
            assert!(d.active, "field becomes active");
            assert_eq!(d.field.value, "2026-07-13 1432", "value keeps the default");
            assert_eq!(d.field.cursor, d.field.char_len(), "caret at end");
        }
    }

    #[test]
    fn sn_typing_on_placeholder_starts_fresh() {
        let mut d = sn_dialog();
        let (act, focus) = save_name_dialog_key(KeyCode::Char('h'), &mut d, 0);
        assert!(matches!(act, SaveNameAction::None));
        assert_eq!(focus, 0);
        assert!(d.active);
        assert_eq!(d.field.value, "h");
        assert_eq!(d.field.cursor, 1);
        // Subsequent chars insert normally.
        save_name_dialog_key(KeyCode::Char('i'), &mut d, 0);
        assert_eq!(d.field.value, "hi");
    }

    #[test]
    fn sn_enter_on_placeholder_saves_the_default() {
        let mut d = sn_dialog();
        let (act, _) = save_name_dialog_key(KeyCode::Enter, &mut d, 0);
        assert!(matches!(act, SaveNameAction::Save));
        assert_eq!(d.field.value, "2026-07-13 1432", "the default is what gets saved");
    }

    #[test]
    fn sn_enter_active_empty_reverts_to_placeholder_and_nonempty_saves() {
        let mut d = sn_dialog();
        // Emptying the field: type then delete it all.
        save_name_dialog_key(KeyCode::Char('x'), &mut d, 0);
        save_name_dialog_key(KeyCode::Backspace, &mut d, 0);
        assert!(d.field.value.is_empty() && d.active);
        let (act, _) = save_name_dialog_key(KeyCode::Enter, &mut d, 0);
        assert!(matches!(act, SaveNameAction::None), "empty Enter does not save");
        assert!(!d.active, "reverts to placeholder");
        // Now type a real name and Enter → Save.
        save_name_dialog_key(KeyCode::Char('a'), &mut d, 0);
        let (act2, _) = save_name_dialog_key(KeyCode::Enter, &mut d, 0);
        assert!(matches!(act2, SaveNameAction::Save));
        assert_eq!(d.field.value, "a");
    }

    #[test]
    fn sn_backspace_and_home_adopt_default_for_editing() {
        let mut d = sn_dialog();
        let (_, _) = save_name_dialog_key(KeyCode::Backspace, &mut d, 0);
        assert!(d.active, "backspace on placeholder adopts");
        assert_eq!(d.field.value, "2026-07-13 143", "then deletes the last char");

        let mut d2 = sn_dialog();
        save_name_dialog_key(KeyCode::Home, &mut d2, 0);
        assert!(d2.active);
        assert_eq!(d2.field.cursor, 0, "Home adopts then moves to start");
    }

    #[test]
    fn sn_field_focus_tab_and_shifttab_move_to_buttons() {
        // Editing mode: Tab advances to Save (1); Shift-Tab wraps to Cancel (2).
        let mut d = sn_dialog();
        d.active = true;
        let (act, focus) = save_name_dialog_key(KeyCode::Tab, &mut d, 0);
        assert!(matches!(act, SaveNameAction::None));
        assert_eq!(focus, 1);
        let (_, focus2) = save_name_dialog_key(KeyCode::BackTab, &mut d, 0);
        assert_eq!(focus2, 2, "Shift-Tab from the field reverse-wraps to Cancel");
    }

    #[test]
    fn sn_active_right_moves_cursor_not_focus() {
        let mut d = sn_dialog();
        d.active = true;
        d.field.home();
        let (act, focus) = save_name_dialog_key(KeyCode::Right, &mut d, 0);
        assert!(matches!(act, SaveNameAction::None));
        assert_eq!(focus, 0, "Right in edit mode stays on the field");
        assert_eq!(d.field.cursor, 1);
    }

    #[test]
    fn sn_esc_cancels_from_field_or_button() {
        let mut d = sn_dialog();
        assert!(matches!(save_name_dialog_key(KeyCode::Esc, &mut d, 0).0, SaveNameAction::Cancel));
        assert!(matches!(save_name_dialog_key(KeyCode::Esc, &mut d, 1).0, SaveNameAction::Cancel));
    }

    #[test]
    fn sn_buttons_activate_and_cycle() {
        let mut d = sn_dialog();
        // Save button (focus 1): Enter/Space → Save.
        assert!(matches!(save_name_dialog_key(KeyCode::Enter, &mut d, 1).0, SaveNameAction::Save));
        assert!(matches!(save_name_dialog_key(KeyCode::Char(' '), &mut d, 1).0, SaveNameAction::Save));
        // Cancel button (focus 2): Enter → Cancel.
        assert!(matches!(save_name_dialog_key(KeyCode::Enter, &mut d, 2).0, SaveNameAction::Cancel));
        // Tab cycles 1 → 2 → 0; Shift-Tab reverses.
        assert_eq!(save_name_dialog_key(KeyCode::Tab, &mut d, 1).1, 2);
        assert_eq!(save_name_dialog_key(KeyCode::Tab, &mut d, 2).1, 0);
        assert_eq!(save_name_dialog_key(KeyCode::BackTab, &mut d, 1).1, 0);
    }
}
