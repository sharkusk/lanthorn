//! Player input a host delivers to the game that is not a typed line (SQ-1568).
//!
//! - [`deliver_v6_click`] — a mouse click on a Version 6 game's own screen.
//!
//! The rule for what a v6 click DOES used to live only inside the terminal
//! binary's event loop, so a host that is not a terminal could not reach it. What
//! stays in the binary is the terminal's own gesture handling — press, drag,
//! release, and which cell maps to which game pixel; this is what happens once a
//! host has decided a click landed on game pixel `(x, y)`.

use mapper::mapper::Mapper;

use crate::engine::Engine;
use crate::engine_helpers::zvm_session_opt_mut;
use crate::state::{AppState, V6ClickRead};

use super::turn::{apply_game_driven_result, finish_command_turn, TurnCtx, TurnOutcome};

/// The ZSCII single-click code (ZMSD §3.8).
const SINGLE_CLICK: u8 = 254;

/// Deliver a left click on 1-based game pixel `game_px` = `(x, y)` to the pending
/// read, and apply the turn it produces.
///
/// `None` — and nothing touched — when the pending read takes no click: not a
/// Z-machine story, no read pending, or a LINE read whose terminating-characters
/// table lists no click (Journey's; see [`crate::input::v6_click_read`]).
///
/// - A CHAR read gets ZSCII 254 with the coordinates recorded first, so the
///   game's `read_mouse` reports them; the result is applied as a game-driven
///   turn, which is not a counted player turn.
/// - A LINE read ends with whatever the player has typed (`AppState::take_input`)
///   and the click as its terminator, and goes through [`finish_command_turn`]
///   like a typed command: history, turn count, mapping, autosave. A compass click
///   types nothing, but the game echoes the command it synthesized ("north") at
///   the head of its output, and that echo is adopted as the turn's command so the
///   move maps exactly like the typed word (SQ-0576).
pub fn deliver_v6_click(
    state: &mut AppState,
    mapper: &mut Mapper,
    session: &mut dyn Engine,
    ctx: &mut TurnCtx<'_>,
    game_px: (u16, u16),
) -> Option<TurnOutcome> {
    let (gx, gy) = game_px;
    let z = zvm_session_opt_mut(session)?;
    let read = crate::input::v6_click_read(Some(z.pending_input()), z.mouse_click_terminator())?;
    match read {
        V6ClickRead::Char => {
            z.set_mouse(gy, gx); // engine stores (y, x)
            let result = z.submit_char(SINGLE_CLICK);
            Some(apply_game_driven_result(
                state,
                mapper,
                &result,
                ctx.game_dir,
                ctx.map_view,
                &*session,
                crate::pager::Driver::PlayerInput,
            ))
        }
        V6ClickRead::Line { terminator } => {
            let cmd = state.take_input();
            z.set_mouse(gy, gx); // engine stores (y, x)
            let result = z.submit_line_with_terminator(&cmd, terminator);
            let cmd = if cmd.is_empty() {
                crate::session::echoed_direction_command(&result.transcript)
                    .unwrap_or_default()
                    .to_string()
            } else {
                cmd
            };
            Some(finish_command_turn(
                &cmd,
                true,
                result,
                state,
                mapper,
                session,
                ctx.game_dir,
                ctx.ifid,
                ctx.arc_file,
                ctx.map_view,
                ctx.bg_tidy_counter,
            ))
        }
    }
}
