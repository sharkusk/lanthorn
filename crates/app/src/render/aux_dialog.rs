use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::dialog::{ButtonId, DialogButton, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::state::AppState;

// Minimum dimensions for the aux-storage dialog.
const MIN_W: u16 = 44;
const MIN_H: u16 = 9;

// Dialog dimensions.
const DIALOG_W: u16 = 56;
const DIALOG_H: u16 = 9;

// ── AuxDialogRects ────────────────────────────────────────────────────────────

pub struct AuxDialogRects {
    pub area: Rect,
    pub close: Option<Rect>,
    pub archive: Option<Rect>,
    pub global: Option<Rect>,
}

// ── draw_aux_dialog ───────────────────────────────────────────────────────────

/// Draw the aux-storage first-use prompt centered over `area`.
///
/// Returns `None` when `state.overlays.aux_prompt` is false or the area is too small.
/// Returns `AuxDialogRects` with hit-rects for close and both choice buttons.
pub fn draw_aux_dialog(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<AuxDialogRects> {
    if !state.overlays.aux_prompt {
        return None;
    }

    let modal_w = DIALOG_W.min(area.width.saturating_sub(4));
    let modal_h = DIALOG_H.min(area.height.saturating_sub(2));

    if modal_w < MIN_W || modal_h < MIN_H {
        return None;
    }

    let st = DialogStyle::from_colors(&state.colors);

    let buttons = &[
        DialogButton { id: ButtonId::Archive, label: "With each save" },
        DialogButton { id: ButtonId::Global,  label: "Globally" },
    ];
    let spec = DialogSpec {
        title: "Side-data",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Archive),
        focus: Some(state.overlays.dialog_focus),
        field: None,
    };

    let rects = draw_dialog(buf, area, &spec, &st);
    let content = rects.content;

    // Draw body text into the content area.
    if content.height >= 1 {
        let body_style = state.colors.theme.get("dialog.background").style;
        crate::render::draw_str_clipped(
            buf,
            content.x,
            content.y,
            "This story saves persistent side-data.",
            body_style,
            content,
        );
    }
    if content.height >= 2 {
        let body_style = state.colors.theme.get("dialog.background").style;
        crate::render::draw_str_clipped(
            buf,
            content.x,
            content.y + 1,
            "Where should lanthorn keep it?",
            body_style,
            content,
        );
    }

    // Map button rects from draw_dialog output.
    let archive_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Archive).map(|(_, r)| *r);
    let global_rect  = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Global).map(|(_, r)| *r);

    Some(AuxDialogRects {
        area: rects.area,
        close: rects.close,
        archive: archive_rect,
        global: global_rect,
    })
}

// ── Aux-storage prompt keyboard routing ──────────────────────────────────────

/// Action to take when a key is pressed while the aux-storage prompt is open.
pub enum AuxDialogAction {
    None,
    Archive,
    Global,
}

/// Aux-dialog keys with button focus. Tab/BackTab are handled by the caller
/// (which mutates dialog_focus); this maps Enter to the focused button.
/// Esc defaults to Archive (conservative: always resolves the prompt).
pub fn aux_dialog_key_focused(code: crossterm::event::KeyCode, focus: usize) -> AuxDialogAction {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Esc => AuxDialogAction::Archive, // conservative default
        KeyCode::Enter => match focus {
            1 => AuxDialogAction::Global,
            _ => AuxDialogAction::Archive, // focus 0 = Archive (default)
        },
        _ => AuxDialogAction::None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;

    #[test]
    fn aux_dialog_renders_title_and_buttons() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut state = crate::state::AppState::default();
        state.overlays.aux_prompt = true;
        let backend = TestBackend::new(70, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal.draw(|f| { rects = draw_aux_dialog(&state, f.area(), f.buffer_mut()); }).unwrap();
        let r = rects.expect("dialog should render when aux_prompt is set");
        assert!(r.close.is_some() && r.archive.is_some() && r.global.is_some());
        let all: String = terminal.backend().buffer().content().iter()
            .flat_map(|c| c.symbol().chars()).collect();
        assert!(all.contains("Side-data"), "title present");
        assert!(all.contains("With each save"), "archive button label present");
        assert!(all.contains("Globally"), "global button label present");
    }

    #[test]
    fn aux_dialog_returns_none_when_not_open() {
        use ratatui::{backend::TestBackend, Terminal};
        let state = crate::state::AppState::default(); // aux_prompt = false
        let backend = TestBackend::new(70, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal.draw(|f| { rects = draw_aux_dialog(&state, f.area(), f.buffer_mut()); }).unwrap();
        assert!(rects.is_none(), "should return None when aux_prompt is false");
    }

    // ── aux_dialog_key_mapping ────────────────────────────────────────────────

    #[test]
    fn aux_dialog_key_mapping() {
        use crossterm::event::KeyCode;
        // Esc → Archive (conservative default so prompt always resolves).
        assert!(matches!(aux_dialog_key_focused(KeyCode::Esc, 0), AuxDialogAction::Archive));
        assert!(matches!(aux_dialog_key_focused(KeyCode::Esc, 1), AuxDialogAction::Archive));
        // Enter on focus 0 → Archive; Enter on focus 1 → Global.
        assert!(matches!(aux_dialog_key_focused(KeyCode::Enter, 0), AuxDialogAction::Archive));
        assert!(matches!(aux_dialog_key_focused(KeyCode::Enter, 1), AuxDialogAction::Global));
        // Other keys → None.
        assert!(matches!(aux_dialog_key_focused(KeyCode::Char('x'), 0), AuxDialogAction::None));
    }

    // ── aux_dialog_tab_then_enter_fires_global ────────────────────────────────

    #[test]
    fn aux_dialog_tab_then_enter_fires_global() {
        use crossterm::event::KeyCode;
        // buttons: [Archive(0), Global(1)], default focus 0.
        // Tab -> focus 1 (Global); Enter on focus 1 -> Global.
        let mut focus = 0usize;
        focus = crate::input::cycle_focus(focus, 2, 1);
        assert_eq!(focus, 1);
        let act = aux_dialog_key_focused(KeyCode::Enter, focus);
        assert!(matches!(act, AuxDialogAction::Global));
    }
}
