use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::dialog::{ButtonId, DialogButton, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::state::AppState;

// Minimum dimensions for the launch dialog.
const MIN_W: u16 = 38;
const MIN_H: u16 = 9;

// Dialog dimensions.
const DIALOG_W: u16 = 44;
const DIALOG_H: u16 = 10;

// ── LaunchDialogRects ─────────────────────────────────────────────────────────

pub struct LaunchDialogRects {
    pub area: Rect,
    pub close: Option<Rect>,
    pub resume: Option<Rect>,
    pub new_game: Option<Rect>,
}

// ── draw_launch_dialog ────────────────────────────────────────────────────────

/// Draw the launch "Resume saved game?" dialog centered over `area`.
///
/// Returns `None` when `state.overlays.launch_dialog` is false or the area is too small.
/// Returns `LaunchDialogRects` with hit-rects for close and buttons.
pub fn draw_launch_dialog(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<LaunchDialogRects> {
    if !state.overlays.launch_dialog {
        return None;
    }

    let modal_w = DIALOG_W.min(area.width.saturating_sub(4));
    let modal_h = DIALOG_H.min(area.height.saturating_sub(2));

    if modal_w < MIN_W || modal_h < MIN_H {
        return None;
    }

    let st = DialogStyle::from_colors(&state.colors);

    let buttons = &[
        DialogButton { id: ButtonId::Resume, label: "Resume" },
        DialogButton { id: ButtonId::NewGame, label: "New game" },
    ];
    let spec = DialogSpec {
        title: "Resume saved game?",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Resume),
        focus: Some(state.overlays.dialog_focus),
        field: None,
    };

    let rects = draw_dialog(buf, area, &spec, &st);
    let content = rects.content;

    // Draw body line into the content area.
    if content.height >= 1 {
        let body_style = state.colors.theme.get("dialog.background").style;
        crate::render::draw_str_clipped(
            buf,
            content.x,
            content.y,
            "A save was found for this story.",
            body_style,
            content,
        );
    }

    // Map button rects from draw_dialog output.
    let resume_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Resume).map(|(_, r)| *r);
    let new_game_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::NewGame).map(|(_, r)| *r);

    Some(LaunchDialogRects {
        area: rects.area,
        close: rects.close,
        resume: resume_rect,
        new_game: new_game_rect,
    })
}

// ── Launch dialog keyboard routing ────────────────────────────────────────────

/// Action to take when a key is pressed while the launch dialog is open.
pub enum LaunchDialogAction {
    None,
    Resume,
    NewGame,
}

/// Map a key code to a LaunchDialogAction.
/// 'r' or Enter → Resume; 'n' or Esc → New game.
#[cfg_attr(not(all(test, feature = "t-render")), allow(dead_code))]
fn launch_dialog_key(code: crossterm::event::KeyCode) -> LaunchDialogAction {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Char('r') | KeyCode::Enter => LaunchDialogAction::Resume,
        KeyCode::Char('n') | KeyCode::Esc => LaunchDialogAction::NewGame,
        _ => LaunchDialogAction::None,
    }
}

/// Launch-dialog keys with button focus. Tab/BackTab are handled by the caller
/// (which mutates dialog_focus); this maps Enter to the focused button and keeps
/// the existing accelerators.
pub fn launch_dialog_key_focused(code: crossterm::event::KeyCode, focus: usize) -> LaunchDialogAction {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Esc => LaunchDialogAction::NewGame,
        KeyCode::Char('r') => LaunchDialogAction::Resume,
        KeyCode::Char('n') => LaunchDialogAction::NewGame,
        KeyCode::Enter => match focus {
            1 => LaunchDialogAction::NewGame,
            _ => LaunchDialogAction::Resume, // focus 0 = Resume (default)
        },
        _ => LaunchDialogAction::None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;

    #[test]
    fn launch_dialog_renders_title_body_and_buttons() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut state = crate::state::AppState::default();
        state.overlays.launch_dialog = true;
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal.draw(|f| { rects = draw_launch_dialog(&state, f.area(), f.buffer_mut()); }).unwrap();
        let r = rects.expect("dialog should render when launch_dialog is set");
        assert!(r.resume.is_some(), "resume button rect must be present");
        assert!(r.new_game.is_some(), "new_game button rect must be present");
        let all: String = terminal.backend().buffer().content().iter()
            .flat_map(|c| c.symbol().chars()).collect();
        assert!(all.contains("Resume saved game?"), "title must be present");
        assert!(all.contains("save was found"), "body line must be present");
        assert!(all.contains("Resume"), "resume button label must be present");
        assert!(all.contains("New game"), "new_game button label must be present");
    }

    #[test]
    fn launch_dialog_returns_none_when_flag_false() {
        use ratatui::{backend::TestBackend, Terminal};
        let state = crate::state::AppState::default(); // launch_dialog = false
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal.draw(|f| { rects = draw_launch_dialog(&state, f.area(), f.buffer_mut()); }).unwrap();
        assert!(rects.is_none(), "dialog must not render when launch_dialog is false");
    }

    #[test]
    fn launch_dialog_returns_none_when_area_too_small() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut state = crate::state::AppState::default();
        state.overlays.launch_dialog = true;
        // Use an area smaller than MIN_W x MIN_H
        let backend = TestBackend::new(10, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal.draw(|f| { rects = draw_launch_dialog(&state, f.area(), f.buffer_mut()); }).unwrap();
        assert!(rects.is_none(), "dialog must not render when area is too small");
    }

    // ── launch_dialog_key ─────────────────────────────────────────────────────

    #[test]
    fn launch_dialog_key_mapping() {
        use crossterm::event::KeyCode;
        assert!(matches!(launch_dialog_key(KeyCode::Char('r')), LaunchDialogAction::Resume));
        assert!(matches!(launch_dialog_key(KeyCode::Enter), LaunchDialogAction::Resume));
        assert!(matches!(launch_dialog_key(KeyCode::Char('n')), LaunchDialogAction::NewGame));
        assert!(matches!(launch_dialog_key(KeyCode::Esc), LaunchDialogAction::NewGame));
        assert!(matches!(launch_dialog_key(KeyCode::Char('x')), LaunchDialogAction::None));
        assert!(matches!(launch_dialog_key(KeyCode::Left), LaunchDialogAction::None));
    }

    // ── launch_dialog_tab_then_enter_fires_focused ────────────────────────────

    #[test]
    fn launch_dialog_tab_then_enter_fires_focused() {
        use crossterm::event::KeyCode;
        // buttons: [Resume(0), New game(1)], default focus 0.
        // Tab -> focus 1 (New game); Enter on focus 1 -> NewGame.
        let mut focus = 0usize;
        focus = crate::input::cycle_focus(focus, 2, 1);
        assert_eq!(focus, 1);
        let act = launch_dialog_key_focused(KeyCode::Enter, focus);
        assert!(matches!(act, LaunchDialogAction::NewGame));
    }
}
