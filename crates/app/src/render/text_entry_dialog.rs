//! The generic single-field text-entry dialog (SQ-0307).
//!
//! A reusable modal built on the common dialog chrome (`render::dialog`) with one
//! caret [`TextField`], an OK and a Cancel button. It is the common home for the
//! former bottom-bar map-edit / config-path / create-file prompts. Unlike the
//! save-name dialog (which keeps its bespoke greyed-placeholder adopt semantics),
//! this field opens ACTIVE, prefilled with the prompt's initial value and the
//! caret at the end — normal caret editing throughout.
//!
//! Focus ring: 0 = text field, 1 = OK, 2 = Cancel. Enter submits (each consumer
//! decides what an empty value means — see `apply_text_entry`); Esc cancels. Key
//! routing is co-located here (the house style after Wave 2); the run-loop
//! intercept in `main.rs` drives it and owns the submit wiring.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::dialog::{
    ButtonId, DialogButton, DialogField, DialogSpec, DialogStyle, Placement, draw_dialog,
};
use crate::state::{AppState, TextEntryKind};

// Minimum dimensions for the text-entry dialog.
const MIN_W: u16 = 34;
const MIN_H: u16 = 7;

// Dialog dimensions.
const DIALOG_W: u16 = 50;
const DIALOG_H: u16 = 7;

// ── TextEntryDialogRects ──────────────────────────────────────────────────────

pub struct TextEntryDialogRects {
    pub area: Rect,
    pub close: Option<Rect>,
    /// Hit-rect for the text field row.
    pub field: Option<Rect>,
    pub ok: Option<Rect>,
    pub cancel: Option<Rect>,
}

/// The window title and field label for each prompt kind.
fn title_and_label(kind: &TextEntryKind) -> (&'static str, &'static str) {
    match kind {
        TextEntryKind::RenameRoom(_) => ("Rename room", "Name: "),
        TextEntryKind::EditNotes(_) => ("Edit notes", "Notes: "),
        TextEntryKind::RelabelEdge(_, _) => ("Relabel exit", "Direction: "),
        TextEntryKind::RenameLayer(_) => ("Rename layer", "Name: "),
        TextEntryKind::ConfigEditPath { .. } => ("Edit path", "Path: "),
        TextEntryKind::CreateFile => ("Game file name", "Filename: "),
    }
}

// ── draw_text_entry_dialog ────────────────────────────────────────────────────

/// Draw the text-entry dialog (a common-dialog with a caret text field) centered
/// over `area` (the graphics-free dialog region). Returns `None` when the dialog
/// is closed or the area is too small.
///
/// Focus ring: 0 = text field, 1 = OK, 2 = Cancel. Only the buttons highlight via
/// `DialogSpec.focus`; the field's caret shows when it is focused.
pub fn draw_text_entry_dialog(
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
) -> Option<TextEntryDialogRects> {
    let dlg = state.overlays.text_entry.as_ref()?;

    let modal_w = DIALOG_W.min(area.width.saturating_sub(4));
    let modal_h = DIALOG_H.min(area.height.saturating_sub(2));
    if modal_w < MIN_W || modal_h < MIN_H {
        return None;
    }

    let st = DialogStyle::from_colors(&state.colors);
    let (title, label) = title_and_label(&dlg.kind);

    let buttons = &[
        DialogButton { id: ButtonId::Ok, label: "OK" },
        DialogButton { id: ButtonId::Cancel, label: "Cancel" },
    ];
    // Buttons occupy focus slots 1 and 2; slot 0 is the field.
    let button_focus = state.overlays.dialog_focus.checked_sub(1);

    // The field is always active; the caret shows while it is focused.
    let field_focused = state.overlays.dialog_focus == 0;
    let field = DialogField {
        label,
        value: &dlg.field.value,
        cursor: dlg.field.cursor,
        show_caret: field_focused,
        dim: false,
        text_style: state.colors.theme.get("dialog.background").style,
        dim_style: state.colors.theme.get("dialog.background").style,
        caret_style: state.colors.theme.get("dialog.background").style.add_modifier(ratatui::style::Modifier::REVERSED),
    };

    let spec = DialogSpec {
        title,
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Ok),
        focus: button_focus,
        field: Some(field),
    };

    let rects = draw_dialog(buf, area, &spec, &st);

    let ok_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Ok).map(|(_, r)| *r);
    let cancel_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Cancel).map(|(_, r)| *r);

    Some(TextEntryDialogRects {
        area: rects.area,
        close: rects.close,
        field: rects.field,
        ok: ok_rect,
        cancel: cancel_rect,
    })
}

// ── Text-entry dialog keyboard routing ────────────────────────────────────────

/// What a key press resolves to in the text-entry dialog.
pub enum TextEntryAction {
    None,
    Submit,
    Cancel,
}

/// Apply one key to the text-entry dialog's `field` and focus ring (0 = field,
/// 1 = OK, 2 = Cancel). Mutates `field` in place and returns the resolved action
/// plus the new focus slot. Callers gate out Ctrl/Alt-modified printable chars
/// before calling.
///
/// Field-focused behavior: normal caret editing (arrows / Home / End / Backspace /
/// Delete / insert); Tab advances to OK, Shift-Tab reverse-wraps to Cancel; Enter
/// submits, Esc cancels. On the buttons Enter/Space activates and Tab/Shift-Tab
/// cycle the ring.
pub fn text_entry_dialog_key(
    code: crossterm::event::KeyCode,
    field: &mut crate::text_field::TextField,
    focus: usize,
) -> (TextEntryAction, usize) {
    use crossterm::event::KeyCode;
    if focus == 0 {
        // ── Text field focused ──
        match code {
            KeyCode::Esc => return (TextEntryAction::Cancel, focus),
            KeyCode::Enter => return (TextEntryAction::Submit, focus),
            KeyCode::BackTab => return (TextEntryAction::None, 2), // reverse-wrap to Cancel
            KeyCode::Tab => return (TextEntryAction::None, 1),     // advance to OK
            KeyCode::Left => field.left(),
            KeyCode::Right => field.right(),
            KeyCode::Home => field.home(),
            KeyCode::End => field.end(),
            KeyCode::Backspace => field.backspace(),
            KeyCode::Delete => field.delete(),
            KeyCode::Char(c) => field.insert(c),
            _ => {}
        }
        (TextEntryAction::None, focus)
    } else {
        // ── OK / Cancel button focused ──
        match code {
            KeyCode::Esc => (TextEntryAction::Cancel, focus),
            KeyCode::Enter | KeyCode::Char(' ') => {
                if focus == 1 {
                    (TextEntryAction::Submit, focus)
                } else {
                    (TextEntryAction::Cancel, focus)
                }
            }
            KeyCode::Tab | KeyCode::Right => (TextEntryAction::None, crate::input::cycle_focus(focus, 3, 1)),
            KeyCode::BackTab | KeyCode::Left => (TextEntryAction::None, crate::input::cycle_focus(focus, 3, -1)),
            _ => (TextEntryAction::None, focus),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use crate::state::{TextEntryDialog, TextEntryKind};
    use crossterm::event::KeyCode;

    #[test]
    fn text_entry_dialog_renders_field_and_buttons_in_area() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut state = AppState::default();
        state.overlays.text_entry =
            Some(TextEntryDialog::new(TextEntryKind::RenameLayer(1), "Ground floor"));
        state.overlays.dialog_focus = 0;
        let backend = TestBackend::new(70, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal
            .draw(|f| rects = draw_text_entry_dialog(&state, f.area(), f.buffer_mut()))
            .unwrap();
        let r = rects.expect("dialog renders when text_entry is set");
        assert!(r.close.is_some() && r.ok.is_some() && r.cancel.is_some());
        assert!(r.field.is_some(), "field rect recorded for hit-testing");
        assert!(r.area.right() <= 70 && r.area.bottom() <= 20);
        let all: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .flat_map(|c| c.symbol().chars())
            .collect();
        assert!(all.contains("Rename layer"), "title present");
        assert!(all.contains("Name:"), "field label present");
        assert!(all.contains("Ground floor"), "initial value prefilled");
    }

    fn te_field(v: &str) -> crate::text_field::TextField {
        crate::text_field::TextField::new(v)
    }

    #[test]
    fn te_typing_inserts_at_caret_no_placeholder_adopt() {
        // The field opens active (caret at end); typing inserts, it does NOT clear
        // the prefilled value the way the save-name placeholder does.
        let mut f = te_field("abc");
        let (act, focus) = text_entry_dialog_key(KeyCode::Char('d'), &mut f, 0);
        assert!(matches!(act, TextEntryAction::None));
        assert_eq!(focus, 0);
        assert_eq!(f.value, "abcd");
        assert_eq!(f.cursor, 4);
    }

    #[test]
    fn te_caret_movement_and_edit() {
        let mut f = te_field("abc");
        text_entry_dialog_key(KeyCode::Home, &mut f, 0);
        assert_eq!(f.cursor, 0);
        text_entry_dialog_key(KeyCode::Right, &mut f, 0);
        assert_eq!(f.cursor, 1);
        text_entry_dialog_key(KeyCode::Delete, &mut f, 0);
        assert_eq!(f.value, "ac");
        text_entry_dialog_key(KeyCode::End, &mut f, 0);
        text_entry_dialog_key(KeyCode::Backspace, &mut f, 0);
        assert_eq!(f.value, "a");
    }

    #[test]
    fn te_enter_submits_from_field_or_ok_button() {
        let mut f = te_field("x");
        assert!(matches!(text_entry_dialog_key(KeyCode::Enter, &mut f, 0).0, TextEntryAction::Submit));
        assert!(matches!(text_entry_dialog_key(KeyCode::Enter, &mut f, 1).0, TextEntryAction::Submit));
        assert!(matches!(text_entry_dialog_key(KeyCode::Char(' '), &mut f, 1).0, TextEntryAction::Submit));
    }

    #[test]
    fn te_enter_submits_even_when_empty() {
        // Every consumer treats empty as a legal value (clear name/notes, no-op
        // direction, cancel create-file); the dialog does not block an empty submit.
        let mut f = te_field("");
        assert!(matches!(text_entry_dialog_key(KeyCode::Enter, &mut f, 0).0, TextEntryAction::Submit));
    }

    #[test]
    fn te_esc_cancels_from_field_or_button() {
        let mut f = te_field("x");
        assert!(matches!(text_entry_dialog_key(KeyCode::Esc, &mut f, 0).0, TextEntryAction::Cancel));
        assert!(matches!(text_entry_dialog_key(KeyCode::Esc, &mut f, 1).0, TextEntryAction::Cancel));
        assert!(matches!(text_entry_dialog_key(KeyCode::Enter, &mut f, 2).0, TextEntryAction::Cancel));
    }

    #[test]
    fn te_tab_focus_ring() {
        let mut f = te_field("x");
        // Field: Tab -> OK (1); Shift-Tab -> Cancel (2).
        assert_eq!(text_entry_dialog_key(KeyCode::Tab, &mut f, 0).1, 1);
        assert_eq!(text_entry_dialog_key(KeyCode::BackTab, &mut f, 0).1, 2);
        // Buttons cycle 1 -> 2 -> 0; Shift-Tab reverses.
        assert_eq!(text_entry_dialog_key(KeyCode::Tab, &mut f, 1).1, 2);
        assert_eq!(text_entry_dialog_key(KeyCode::Tab, &mut f, 2).1, 0);
        assert_eq!(text_entry_dialog_key(KeyCode::BackTab, &mut f, 1).1, 0);
    }
}
