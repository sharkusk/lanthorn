//! The game's clocks (SQ-1539): timed input, Glk timers, sound finish
//! routines and Sound2 volume ramps, as calls a host makes on its own schedule.
//!
//! The engines expose each clock as data — a Z-machine read's timeout
//! (`pending_timeout`), a Glulx game's `glk_request_timer_events` interval
//! (`timer_interval`), a sound's finish routine, a volume ramp's deadline —
//! and the rules for arming, firing and disarming them used to live in the
//! TUI's event loop (`main::dispatch_due_game_clocks`, `loop_tick::
//! refresh_engine_input`). A host drives them the same way the loop does:
//!
//! 1. after anything that may have changed what the game is waiting for, call
//!    [`refresh_input`] — it re-arms the deadlines (and seeds a pre-loaded input
//!    line, SQ-0562/SQ-1419);
//! 2. sleep until [`next_deadline`] (or until the player acts);
//! 3. call [`fire_due`] with the current time, and apply its outcome.
//!
//! The TUI does exactly this once per loop pass.

use std::time::{Duration, Instant};

use mapper::mapper::Mapper;

use crate::engine::Engine;
use crate::engine_helpers::{glulx_session_opt, glulx_session_opt_mut, zvm_session_opt, zvm_session_opt_mut};
use crate::session::InputKind;
use crate::state::AppState;

use super::turn::{apply_game_driven_result, next_input_deadline};

/// True when an armed deadline has come due. Extracted so the "is this clock
/// due?" decision is testable on its own (SQ-0650). `None` = not armed.
pub fn deadline_due(deadline: Option<Instant>, now: Instant) -> bool {
    deadline.is_some_and(|dl| now >= dl)
}

/// What [`fire_due`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[must_use]
pub struct Fired {
    /// A clock fired, so the game may have printed or the channel state moved.
    pub redraw: bool,
    /// A clock's routine ended the game; the host closes the session.
    pub quit: bool,
}

/// Fire every game clock whose deadline has come due at `now`: the Z-machine
/// timed-input interrupt, the Glulx Glk timer, sampled-sound finish routines /
/// sound-notify (for every id the state's [`SoundSink`](super::sound::SoundSink)
/// reports finished), and Sound2 volume ramps + their volume-notify.
///
/// **Runs once per loop iteration, on every path** (SQ-0650). This used to live
/// inside the TUI's poll-timeout branch, which meant it only ran on a tick where
/// NO terminal event arrived — so a mouse whose motion events keep `poll()`
/// permanently "ready" froze every one of these clocks: a timed-input puzzle
/// stopped counting down, a Glk timer stopped ticking, and a finished sound never
/// ran its finish routine, for as long as the pointer kept moving. The loop top
/// is the same safe point the timeout branch used (both sit between whole event
/// dispatches, with nothing borrowed), so this is a move, not a new re-entrancy.
///
/// Each fired clock disarms itself before dispatching so an elapsed deadline
/// cannot re-fire every iteration until the game re-arms it ([`refresh_input`]
/// re-arms it fresh at `now + interval`).
pub fn fire_due(
    state: &mut AppState,
    mapper: &mut Mapper,
    session: &mut dyn Engine,
    game_dir: &std::path::Path,
    map_view: Option<(u16, u16)>,
    now: Instant,
) -> Fired {
    let mut redraw = false;
    // Timed-input interrupt: the deadline elapsed with no key pressed. Run the
    // game's interrupt routine and apply its output through the same path a
    // char-mode keypress uses. If the read continues, the pre-input pollers
    // re-arm the deadline next iteration from `pending_timeout()`; if the routine
    // aborted the read, it returns `None` and the timer simply stops.
    if deadline_due(state.input_deadline, now) {
        if let Some(zs) = zvm_session_opt_mut(session) {
            let result = zs.run_timed_interrupt();
            // Fired: disarm so the next armed iteration re-arms fresh at
            // now + interval (otherwise the elapsed deadline would refire
            // immediately every iteration).
            state.input_deadline = None;
            redraw = true; // interrupt ran → repaint any output
            if apply_game_driven_result(
                state, mapper, &result, game_dir, map_view, &*session, crate::pager::Driver::Timeout,
            ).quit {
                return Fired { redraw, quit: true };
            }
        }
    }
    // Glulx Glk timer tick: the interval elapsed with no key pressed. Deliver an
    // evtype_Timer to the game and apply its output; disarm so the next armed
    // iteration re-arms fresh at now + interval (mirroring the guard above).
    if deadline_due(state.glulx_timer_next_fire, now) {
        state.glulx_timer_next_fire = None;
        redraw = true; // timer event delivered → repaint any output
        if let Some(gs) = glulx_session_opt_mut(session) {
            let result = gs.deliver_timer();
            if apply_game_driven_result(
                state, mapper, &result, game_dir, map_view, &*session, crate::pager::Driver::Timeout,
            ).quit {
                return Fired { redraw, quit: true };
            }
        }
    }
    // Poll for finished sampled sounds and fire their finish-routines. What a
    // finished sound runs is `sound_finished`'s; the sink is only what noticed.
    let done: Vec<audio::SoundId> = state.audio.as_mut().map(|b| b.finished()).unwrap_or_default();
    if !done.is_empty() {
        redraw = true; // finish-routine output / channel state changed
    }
    for id in done {
        if super::sound::sound_finished(state, mapper, session, id, game_dir, map_view) {
            return Fired { redraw, quit: true };
        }
    }
    // Glulx Sound2 volume-ramp completion: a gradual set_volume_ext whose
    // duration has elapsed delivers an evtype_VolumeNotify. The host owns the
    // ramp clock (mirroring the sound-finish notify above); deliver every due one.
    // Step any in-flight Sound2 volume ramp toward its target (host owns the ramp
    // clock). Pure audio — no redraw needed.
    state.advance_volume_ramps(now);
    let due_volume: Vec<(u32, u32)> = state
        .glulx_volume_notify
        .iter()
        .filter(|(_, (deadline, _))| *deadline <= now)
        .map(|(&chan, &(_, notify))| (chan, notify))
        .collect();
    if !due_volume.is_empty() {
        redraw = true;
    }
    for (chan, notify) in due_volume {
        state.glulx_volume_notify.remove(&chan);
        if let Some(gs) = glulx_session_opt_mut(session) {
            let result = gs.volume_notify(notify);
            if apply_game_driven_result(
                state, mapper, &result, game_dir, map_view, &*session, crate::pager::Driver::Timeout,
            ).quit {
                return Fired { redraw, quit: true };
            }
        }
    }
    Fired { redraw, quit: false }
}

/// The soonest game clock: the Z-machine timed-input deadline, the Glulx Glk
/// timer, or the earliest pending Sound2 volume-ramp completion. `None` when no
/// clock is armed — the host has nothing to wake for but the player.
pub fn next_deadline(state: &AppState) -> Option<Instant> {
    let next_volume_deadline = state.glulx_volume_notify.values().map(|(t, _)| *t).min();
    [state.input_deadline, state.glulx_timer_next_fire, next_volume_deadline]
        .into_iter()
        .flatten()
        .min()
}

/// What the game is waiting for right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputRequest {
    /// A line, a single key, or (Glulx) only an event such as a timer or a click.
    pub kind: InputKind,
    /// A Z-machine timed read's interval (ZMSD §15 `read`/`read_char` `time`, in
    /// tenths of a second), or a Glulx game's Glk timer interval. `None` when no
    /// clock is running.
    pub timeout: Option<Duration>,
}

/// What the game is waiting for, and how long before a clock fires without the
/// player.
pub fn input_request(session: &dyn Engine) -> InputRequest {
    let zvm_timeout = zvm_session_opt(session)
        .and_then(|s| s.pending_timeout())
        .map(|(t, _)| Duration::from_millis(t as u64 * 100));
    let glk_timer = glulx_session_opt(session).and_then(|s| s.timer_interval());
    InputRequest { kind: session.pending_input(), timeout: zvm_timeout.or(glk_timer) }
}

/// Refresh the engine input-mode flags and re-arm the timed-input / Glk-timer
/// deadlines. Returns `true` only for a prompt-visibility transition (the timer
/// re-arm never forces a redraw).
///
/// A clock is armed only while no overlay covers the pane — a host that shows
/// the player no dialogs never has one open — and a Z-machine timed read only
/// when `honor_timed_input` is on.
pub fn refresh_input(state: &mut AppState, session: &mut dyn Engine) -> bool {
    let mut redraw = false;

    // Each new Glulx line-input request says what the input line should now hold
    // (Glk spec §4.2 `initlen`): text the game pre-loaded, editable, or nothing.
    // advent.blb's toolbar needs both — clicking Examine must leave "Examine " at
    // the prompt for the player to finish, while clicking a verb over an
    // already-typed noun runs that command itself and asks again empty, which must
    // not strand the noun at the prompt. Either way the game has just consumed or
    // cancelled the previous line, so its answer is authoritative and replaces
    // whatever the app was showing. (SQ-0562, SQ-0565)
    if let Some(text) = glulx_session_opt_mut(session).and_then(|gs| gs.take_line_seed()) {
        if state.input.value != text {
            state.input.clear();
            state.input.insert_str(&text);
            redraw = true;
        }
    }
    // ...and keep the game's buffer holding what the player has actually typed.
    // Glk lends the interpreter that buffer for the whole request so a cancel can
    // report the partial input; advent.blb's toolbar cancels on every button press
    // and preserves what it finds there, so a stale buffer made every later button
    // re-insert the FIRST verb — text the player may have already deleted. Written
    // after the seed above so a fresh prefill lands in the buffer too. (SQ-0565)
    if let Some(gs) = glulx_session_opt_mut(session) {
        gs.sync_line_input(&state.input.value);
    }

    // The Z-machine twin (SQ-1419): ZMSD §15 `read`'s pre-loaded input line
    // (v5+ — "if byte 1 contains a positive value at the start of the input,
    // then read assumes that number of characters are left over from an
    // interrupted previous input"), which TerpEtude option 12 and Beyond
    // Zork's "AGAIN" both rely on. One-shot per request (see
    // `GameSession::take_line_seed`), so it never re-clobbers what the
    // player has since typed — unlike Glulx there is no live buffer to keep
    // in sync afterwards: the whole displayed line is handed back to
    // `Machine::supply_line` as one string when the player submits.
    if let Some(text) = zvm_session_opt_mut(session).and_then(|gs| gs.take_line_seed()) {
        state.input.clear();
        state.input.insert_str(&text);
        redraw = true;
    }

    // Update char_mode flag so the renderer hides the prompt during read_char.
    let prev_char_mode = state.char_mode;
    let prev_event_wait = state.event_wait;
    state.char_mode = matches!(session.pending_input(), InputKind::Char);
    // A Glulx timer/mouse/hyperlink-only glk_select: hide the prompt too (no
    // typed input is requested), but unlike char_mode do NOT forward keys to
    // the game — the timer clock / click delivers the event instead.
    state.event_wait = matches!(session.pending_input(), InputKind::Event);
    // A prompt-visibility transition changes the frame even with no new input.
    if state.char_mode != prev_char_mode || state.event_wait != prev_event_wait {
        redraw = true;
    }

    // Re-arm the timed-input deadline each iteration. Only while the game is
    // actually awaiting input (no dialog/overlay/prompt covering the pane) and
    // honoring timers; `pending_timeout()` is `None` for an untimed read, so
    // this is a no-op for the vast majority of games (regression guard). Timed
    // input is a Z-machine-only concept (ZMSD): `zvm_session_opt` is `None` for
    // a Glulx engine, so the timer never arms there.
    let timer_interval = zvm_session_opt(session)
        .and_then(|s| s.pending_timeout())
        .map(|(t, _)| Duration::from_millis(t as u64 * 100));
    let should_arm = state.config.honor_timed_input
        && !state.any_overlay_open()
        && timer_interval.is_some();
    state.input_deadline = next_input_deadline(
        state.input_deadline,
        should_arm,
        timer_interval.unwrap_or(Duration::ZERO),
        Instant::now(),
    );

    // Re-arm the Glulx Glk timer-events clock (glk_request_timer_events) — the
    // Glulx analogue of `input_deadline`, and independent of it. Armed only
    // when a Glulx game has requested a timer interval and no overlay covers
    // the pane; uses the same arm-once semantics (`next_input_deadline`) so the
    // deadline holds steady until it fires (the fire path above re-arms fresh).
    let glk_timer_interval = glulx_session_opt(session).and_then(|s| s.timer_interval());
    let should_arm_glk_timer = !state.any_overlay_open() && glk_timer_interval.is_some();
    state.glulx_timer_next_fire = next_input_deadline(
        state.glulx_timer_next_fire,
        should_arm_glk_timer,
        glk_timer_interval.unwrap_or(Duration::ZERO),
        Instant::now(),
    );

    redraw
}

#[cfg(all(test, feature = "t-session"))]
mod tests {
    // ── SQ-0650: game clocks must not be starved by a busy event stream ────────

    #[test]
    fn deadline_due_only_once_armed_and_elapsed() {
        let now = std::time::Instant::now();
        assert!(!super::deadline_due(None, now), "not armed: never due");
        assert!(
            super::deadline_due(Some(now - std::time::Duration::from_millis(1)), now),
            "elapsed deadline is due"
        );
        assert!(super::deadline_due(Some(now), now), "exactly at the deadline is due");
        assert!(
            !super::deadline_due(Some(now + std::time::Duration::from_secs(1)), now),
            "a future deadline is not due yet"
        );
    }

    /// The Glulx timer arm of the clock dispatch, driven with a non-Glulx engine:
    /// an elapsed deadline must DISARM and report a redraw regardless of which
    /// engine is running, which is what makes the loop-top dispatch safe to run on
    /// every path. (The engine-specific delivery is covered by the Glulx suites.)
    #[test]
    fn due_game_clocks_disarm_an_elapsed_glulx_timer() {
        let mut state = crate::state::AppState::default();
        let mut mapper = mapper::mapper::Mapper::default();
        let mut engine = ClocklessEngine;
        state.glulx_timer_next_fire = Some(std::time::Instant::now() - std::time::Duration::from_millis(5));

        let fired = super::fire_due(
            &mut state,
            &mut mapper,
            &mut engine,
            std::path::Path::new("/nonexistent"),
            None,
            std::time::Instant::now(),
        );
        assert!(fired.redraw, "a fired timer repaints");
        assert!(!fired.quit);
        assert!(state.glulx_timer_next_fire.is_none(), "an elapsed deadline must disarm, not refire every tick");

        // A future deadline is left alone.
        let future = std::time::Instant::now() + std::time::Duration::from_secs(30);
        state.glulx_timer_next_fire = Some(future);
        let fired = super::fire_due(
            &mut state,
            &mut mapper,
            &mut engine,
            std::path::Path::new("/nonexistent"),
            None,
            std::time::Instant::now(),
        );
        assert!(!fired.redraw, "nothing due: no repaint");
        assert_eq!(state.glulx_timer_next_fire, Some(future), "still armed");
        assert_eq!(super::next_deadline(&state), Some(future), "and it is the next thing to wake for");
    }

    /// Minimal engine that is neither a Z-machine nor a Glulx session, so the
    /// clock dispatch's downcasts all miss. Only the engine-neutral bookkeeping
    /// (disarm + redraw) is exercised.
    struct ClocklessEngine;

    impl crate::engine::Engine for ClocklessEngine {
        fn submit(&mut self, _command: &str) -> crate::session::TurnResult { unreachable!() }
        fn submit_key(&mut self, _key: crate::engine::KeyInput) -> Option<crate::session::TurnResult> { unreachable!() }
        fn take_transcript(&mut self) -> String { unreachable!() }
        // No screen-clear channel: this double is not a game.
        fn drain_screen_clear(&mut self) -> bool { false }
        fn pending_input(&self) -> crate::session::InputKind { unreachable!() }
        fn resume_save(&mut self, _wrote_ok: bool) -> crate::session::TurnResult { unreachable!() }
        fn resume_restore(&mut self, _data: Option<&[u8]>) -> crate::session::TurnResult { unreachable!() }
        fn has_quit(&self) -> bool { false }
        fn screen(&self) -> crate::engine::ScreenModel { unreachable!() }
        fn save_state(&self) -> crate::engine::EngineSave { unreachable!() }
        fn restore_state(&mut self, _save: &crate::engine::EngineSave) -> Result<(), crate::engine::EngineError> { unreachable!() }
        fn restore_game_save(&mut self, _bytes: &[u8]) -> Result<(), crate::engine::EngineError> { unreachable!() }
        fn aux_data(&self) -> &std::collections::BTreeMap<String, Vec<u8>> { unreachable!() }
        fn set_aux_data(&mut self, _data: std::collections::BTreeMap<String, Vec<u8>>) {}
        fn aux_dirty(&self) -> bool { false }
        fn clear_aux_dirty(&mut self) {}
        fn current_location(&self) -> Option<crate::engine::LocationInfo> { None }
        fn as_any(&self) -> &dyn std::any::Any { self }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    }
}
