//! The "open this spoiler?" confirm dialog (SQ-1681).
//!
//! A two-button confirm on the common dialog chrome, the confirm-overwrite
//! pattern: the Documents tab asks before showing a file whose name flags it as a
//! walkthrough, hint sheet or solution. Cancel is the default, so Enter without
//! reading spoils nothing.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::dialog::{draw_dialog, ButtonId, DialogButton, DialogSpec, DialogStyle, Placement};
use crate::state::AppState;

const MIN_W: u16 = 34;
const MIN_H: u16 = 8;
const DIALOG_W: u16 = 50;
const DIALOG_H: u16 = 9;

pub struct ConfirmSpoilerDialogRects {
    pub area: Rect,
    pub close: Option<Rect>,
    pub open: Option<Rect>,
    pub cancel: Option<Rect>,
}

/// Draw the dialog centred over `area`; `None` when it is not open or `area` is too small.
pub fn draw_confirm_spoiler_dialog(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<ConfirmSpoilerDialogRects> {
    let name = state.overlays.confirm_spoiler_document.as_ref()?;
    draw_spoiler_confirm(name, state.overlays.dialog_focus, &state.colors, area, buf)
}

/// The same dialog for a caller with no `AppState` (the story browser's info
/// panel, SQ-1700): `focus` is 0 = Open it, 1 = Cancel.
pub fn draw_spoiler_confirm(
    name: &str,
    focus: usize,
    colors: &crate::colors::ColorScheme,
    area: Rect,
    buf: &mut Buffer,
) -> Option<ConfirmSpoilerDialogRects> {
    let modal_w = DIALOG_W.min(area.width.saturating_sub(4));
    let modal_h = DIALOG_H.min(area.height.saturating_sub(2));
    if modal_w < MIN_W || modal_h < MIN_H {
        return None;
    }
    let st = DialogStyle::from_colors(colors);
    let buttons = &[
        DialogButton { id: ButtonId::Ok, label: "Open it" },
        DialogButton { id: ButtonId::Cancel, label: "Cancel" },
    ];
    let spec = DialogSpec {
        title: "Open a spoiler?",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Cancel),
        focus: Some(focus),
        field: None,
    };
    let rects = draw_dialog(buf, area, &spec, &st);
    let content = rects.content;
    let body = colors.theme.get("dialog.background").style;
    if content.height >= 1 {
        let quoted = crate::textwidth::clip_to_cols_ellipsis(&format!("\"{name}\""), content.width as usize);
        crate::render::draw_str_clipped(buf, content.x, content.y, &quoted, body, content);
    }
    if content.height >= 3 {
        crate::render::draw_str_clipped(buf, content.x, content.y + 2, "It may give away the game.", body, content);
    }
    let find = |id| rects.buttons.iter().find(|(b, _)| *b == id).map(|(_, r)| *r);
    Some(ConfirmSpoilerDialogRects {
        area: rects.area,
        close: rects.close,
        open: find(ButtonId::Ok),
        cancel: find(ButtonId::Cancel),
    })
}

/// What a key means to the dialog.
pub enum ConfirmSpoilerAction {
    None,
    Open,
    Cancel,
}

/// Map a key given the focused button (0 = Open it, 1 = Cancel). The caller moves
/// the focus on Tab / Shift-Tab; Enter activates the focused button.
pub fn confirm_spoiler_key_focused(code: crossterm::event::KeyCode, focus: usize) -> ConfirmSpoilerAction {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Esc | KeyCode::Char('n') => ConfirmSpoilerAction::Cancel,
        KeyCode::Char('y') => ConfirmSpoilerAction::Open,
        KeyCode::Enter | KeyCode::Char(' ') => match focus {
            0 => ConfirmSpoilerAction::Open,
            _ => ConfirmSpoilerAction::Cancel,
        },
        _ => ConfirmSpoilerAction::None,
    }
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;

    #[test]
    fn renders_the_name_and_both_buttons_only_while_open() {
        let mut state = AppState::default();
        let area = Rect::new(0, 0, 70, 20);
        let mut buf = Buffer::empty(area);
        assert!(draw_confirm_spoiler_dialog(&state, area, &mut buf).is_none());
        state.overlays.confirm_spoiler_document = Some("walkthrough.txt".into());
        state.overlays.dialog_focus = 1;
        let r = draw_confirm_spoiler_dialog(&state, area, &mut buf).expect("open");
        assert!(r.open.is_some() && r.cancel.is_some());
        let all: String = buf.content().iter().map(|c| c.symbol().to_string()).collect();
        assert!(all.contains("Open a spoiler?") && all.contains("walkthrough.txt"));
    }

    #[test]
    fn enter_follows_focus_and_cancel_is_the_default() {
        assert!(matches!(confirm_spoiler_key_focused(KeyCode::Enter, 1), ConfirmSpoilerAction::Cancel));
        assert!(matches!(confirm_spoiler_key_focused(KeyCode::Enter, 0), ConfirmSpoilerAction::Open));
        assert!(matches!(confirm_spoiler_key_focused(KeyCode::Esc, 0), ConfirmSpoilerAction::Cancel));
        assert!(matches!(confirm_spoiler_key_focused(KeyCode::Char('y'), 1), ConfirmSpoilerAction::Open));
        assert!(matches!(confirm_spoiler_key_focused(KeyCode::Char('x'), 1), ConfirmSpoilerAction::None));
    }
}
