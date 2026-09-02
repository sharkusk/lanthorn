use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::dialog::{ButtonId, DialogButton, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::state::AppState;

// Minimum dimensions for the game-over dialog.
const MIN_W: u16 = 38;
const MIN_H: u16 = 9;

// Dialog dimensions.
const DIALOG_W: u16 = 42;
const DIALOG_H: u16 = 10;

// ── GameOverDialogRects ─────────────────────────────────────────────────────────

pub struct GameOverDialogRects {
    pub area: Rect,
    pub close: Option<Rect>,
    pub play_again: Option<Rect>,
    pub restore: Option<Rect>,
    pub quit: Option<Rect>,
}

// ── draw_game_over_dialog ─────────────────────────────────────────────────────

/// Draw the Scott-only "game is now over" dialog centered over `area`.
///
/// Returns `None` when `state.overlays.game_over` is false or the area is too small.
/// Returns `GameOverDialogRects` with hit-rects for close and the three buttons.
pub fn draw_game_over_dialog(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<GameOverDialogRects> {
    if !state.overlays.game_over {
        return None;
    }

    let modal_w = DIALOG_W.min(area.width.saturating_sub(4));
    let modal_h = DIALOG_H.min(area.height.saturating_sub(2));

    if modal_w < MIN_W || modal_h < MIN_H {
        return None;
    }

    let st = DialogStyle::from_colors(&state.colors);

    let buttons = &[
        DialogButton { id: ButtonId::PlayAgain, label: "Play Again" },
        DialogButton { id: ButtonId::Restore, label: "Restore" },
        DialogButton { id: ButtonId::Quit, label: "Quit" },
    ];
    let spec = DialogSpec {
        title: "The game is now over.",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: false,
        default: Some(ButtonId::PlayAgain),
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
            "The game is now over.",
            body_style,
            content,
        );
    }

    // Map button rects from draw_dialog output.
    let play_again_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::PlayAgain).map(|(_, r)| *r);
    let restore_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Restore).map(|(_, r)| *r);
    let quit_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Quit).map(|(_, r)| *r);

    Some(GameOverDialogRects {
        area: rects.area,
        close: rects.close,
        play_again: play_again_rect,
        restore: restore_rect,
        quit: quit_rect,
    })
}

// ── Game-over dialog keyboard routing ─────────────────────────────────────────

/// Action to take when a key is pressed while the game-over dialog is open.
pub enum GameOverAction {
    None,
    PlayAgain,
    Restore,
    Quit,
}

/// Game-over dialog keys with button focus. Tab/BackTab are handled by the caller
/// (which mutates dialog_focus over a 3-slot ring: 0 = Play Again, 1 = Restore,
/// 2 = Quit). Enter activates the focused button. Esc maps to Quit (the game is
/// over — there is nothing to cancel back to). 'p'/'r'/'q' are accelerators.
pub fn game_over_dialog_key_focused(code: crossterm::event::KeyCode, focus: usize) -> GameOverAction {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Esc | KeyCode::Char('q') => GameOverAction::Quit,
        KeyCode::Char('p') => GameOverAction::PlayAgain,
        KeyCode::Char('r') => GameOverAction::Restore,
        KeyCode::Enter => match focus {
            1 => GameOverAction::Restore,
            2 => GameOverAction::Quit,
            _ => GameOverAction::PlayAgain, // focus 0 = Play Again (default)
        },
        _ => GameOverAction::None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;

    #[test]
    fn game_over_dialog_renders_title_body_and_buttons() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut state = crate::state::AppState::default();
        state.overlays.game_over = true;
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal.draw(|f| { rects = draw_game_over_dialog(&state, f.area(), f.buffer_mut()); }).unwrap();
        let r = rects.expect("dialog should render when game_over is set");
        assert!(r.play_again.is_some(), "play-again button rect must be present");
        assert!(r.restore.is_some(), "restore button rect must be present");
        assert!(r.quit.is_some(), "quit button rect must be present");
        let all: String = terminal.backend().buffer().content().iter()
            .flat_map(|c| c.symbol().chars()).collect();
        assert!(all.contains("The game is now over."), "title/body must be present");
        assert!(all.contains("Play Again"), "play-again button label must be present");
        assert!(all.contains("Restore"), "restore button label must be present");
        assert!(all.contains("Quit"), "quit button label must be present");
    }

    #[test]
    fn game_over_dialog_returns_none_when_flag_false() {
        use ratatui::{backend::TestBackend, Terminal};
        let state = crate::state::AppState::default(); // game_over = false
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal.draw(|f| { rects = draw_game_over_dialog(&state, f.area(), f.buffer_mut()); }).unwrap();
        assert!(rects.is_none(), "dialog must not render when game_over is false");
    }

    #[test]
    fn game_over_dialog_tab_then_enter_fires_focused() {
        use crossterm::event::KeyCode;
        // buttons: [Play Again(0), Restore(1), Quit(2)], default focus 0.
        // Tab -> focus 1 (Restore); Enter on focus 1 -> Restore.
        let mut focus = 0usize;
        focus = crate::input::cycle_focus(focus, 3, 1);
        assert_eq!(focus, 1);
        assert!(matches!(game_over_dialog_key_focused(KeyCode::Enter, focus), GameOverAction::Restore));
        // Enter on focus 2 -> Quit; focus 0 -> Play Again.
        assert!(matches!(game_over_dialog_key_focused(KeyCode::Enter, 2), GameOverAction::Quit));
        assert!(matches!(game_over_dialog_key_focused(KeyCode::Enter, 0), GameOverAction::PlayAgain));
        // Esc always maps to Quit (nothing to cancel back to).
        assert!(matches!(game_over_dialog_key_focused(KeyCode::Esc, 0), GameOverAction::Quit));
    }
}
