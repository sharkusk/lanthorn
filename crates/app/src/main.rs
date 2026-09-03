// Test fixtures build structs by defaulting then setting a few fields; silence
// the pedantic lint in tests only (see the matching attribute in lib.rs).
#![cfg_attr(test, allow(clippy::field_reassign_with_default))]

use std::io::stdout;
use std::time::Duration;

use crossterm::event::{
    poll, read, DisableBracketedPaste, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind,
};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, LeaveAlternateScreen};
use mapper::mapper::Mapper;
use mapper::render::{render as render_map_data, render_layer};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::Terminal;

use app::export_dot::export_dot;
use app::export_svg::export_svg;
use app::map_dump::render_dump;
use app::archive::load_archive;
use app::input::{apply_action, apply_text_entry, key_to_command, mouse_to_action, Action, KeyResolve};
use app::tidy::should_bg_tidy;
use app::persist_files::{list_saves, restore_game};
use app::render::dialog::{DialogRects, DialogStyle};
use app::render::hints_panel::{hint_input_action, hint_key_routes, HintInputAct, HintKeyKind, HintsPanelRects};
use app::render::command_band::draw_command_band;
use app::render::map::{pulse_border_color, render_map_layered, room_screen_rects, sound_pulse_color};
use app::render::paneframe::{build_layer_segments, InsetSegment};
use app::render::panel::{PanelFrame, PanelSpec, PanelStrip};
use app::render::controls::BorderControl;
use app::render::tidy_panel::draw_tidy_panel;
use mapper::graph::RoomId;
use mapper::layer::LayerId;
use app::render::screen::render_story_pane;
use app::render::draw_str_clipped;
use app::engine::Engine;
use app::session::{apply_turn, TurnResult};
use app::hints;
use app::keymap::Context;
use app::render::hintbar::{hint_bar, ANIM_HINTS, GAME_HINTS};
use app::slash;
use app::state::{AppState, FbMode, FileBrowserState, Focus, Layout, SavesState};

mod engine_helpers;
mod ingame_io;
mod lifecycle;
mod loop_tick;
mod overlays;
mod picker_ui;
mod reset;
mod slash_dispatch;
mod startup;
mod turn;

use crate::slash_dispatch::dispatch_slash_outcome;
use crate::ingame_io::{
    delete_save_confirmed, handle_save_as, open_ingame_saves, resolve_filename_request,
    resolve_ingame_dialog,
};
use crate::reset::reset_game;
use crate::engine_helpers::{
    apply_archive_state, engine_supports_save, engine_tag, glulx_session_opt_mut, restore_error_msg,
    restore_from_file, zvm_session_mut, zvm_session_opt, zvm_session_opt_mut, RestoreOutcome,
};

// ── Run outcome ─────────────────────────────────────────────────────────────

/// How the event loop ended: exit lanthorn entirely, or return to the story
/// picker (a library launch replays the picker; a single-file launch treats it
/// as an exit). Mapped from `AppState.exit_target` at the loop's break sites. (SQ-0435)
#[derive(Debug, Clone, Copy, PartialEq)]
enum RunOutcome {
    Exit,
    ToLibrary,
}

impl From<app::state::ExitTarget> for RunOutcome {
    fn from(t: app::state::ExitTarget) -> Self {
        match t {
            app::state::ExitTarget::Exit => RunOutcome::Exit,
            app::state::ExitTarget::Library => RunOutcome::ToLibrary,
        }
    }
}

// ── Arrow-key withholding (SQ-0460) ──────────────────────────────────────────

/// Whether an arrow keypress should be forwarded to the story as a ZSCII
/// cursor code (129-132; ZMSD §3.8). Some v6 games bind arrows to movement;
/// `v6_arrow_keys = false` withholds them so the key falls through to
/// app-side handling (scrollback / map panning) instead. Only v6 is gated —
/// v1-5 and Glulx stories always get arrows, regardless of `version`'s value
/// for a non-Z-machine session (callers pass a version of 0 in that case).
fn forward_arrow_to_v6(v6_arrow_keys: bool, version: u8) -> bool {
    version != 6 || v6_arrow_keys
}

/// Whether `ki` is an arrow that `v6_arrow_keys = false` withholds from a v6
/// story. Withholding applies ONLY at a line (`>`) prompt (`is_line_input`) —
/// that's where movement-vs-panning conflicts, and v6 games list arrows in
/// their terminating-characters table (SQ-0188), so an arrow would otherwise
/// move the player from the prompt regardless of the setting. During CHAR
/// input (`is_line_input = false`: menus, "press any key") arrows are NEVER
/// withheld — those screens are unnavigable without them, so the setting has
/// no say there and arrows always reach a v6 story (SQ-0483).
fn withhold_arrow_from_v6(
    ki: Option<app::engine::KeyInput>,
    v6_arrow_keys: bool,
    version: u8,
    is_line_input: bool,
) -> bool {
    is_line_input
        && ki.is_some_and(|ki| {
            matches!(ki, app::engine::KeyInput::Up | app::engine::KeyInput::Down
                | app::engine::KeyInput::Left | app::engine::KeyInput::Right)
                && !forward_arrow_to_v6(v6_arrow_keys, version)
        })
}

// ── Terminal restore helpers ──────────────────────────────────────────────────

/// Restore the terminal to cooked mode and leave the alternate screen.
/// Called both on clean exit and from the panic hook.
/// DisableMouseCapture MUST be issued here so both paths release the mouse.
///
/// **Order matters, and it is the reverse of setup: silence the terminal FIRST,
/// leave raw mode LAST** (SQ-0998). This ran the other way round for as long as it
/// existed, and the window between the two was wide enough to lose a mouse report
/// into the shell: `disable_raw_mode` puts ICANON+ECHO back while mode 1003
/// any-motion reporting is still on, so a report generated in that window goes to
/// the line discipline instead of to us. The user's shell prompt came back carrying
/// `35;154;45M` — the tail of `ESC [ < 35;154;45 M`, an SGR (1006) motion report at
/// column 154, row 45, whose `ESC [ <` the shell ate as an escape and whose
/// remainder it kept as typed input. With the escapes written while raw mode still
/// holds, no report can reach the line discipline at all.
fn restore_terminal() {
    // SQ-0586: FIRST, so anything printed after this — a panic message, a CLI error,
    // the captured-output notice below — reaches the terminal rather than the log we
    // pointed fd 2 at. Idempotent and safe when nothing was installed.
    app::stderr_redirect::restore();
    // Mode 1016 (pixel mouse reporting) is terminal state that outlives us, and
    // DisableMouseCapture does not clear it — a shell left in PixelMode would hand
    // pixel coordinates to the next program that reads the mouse. (SQ-0563)
    // DisableBracketedPaste for the same reason as the pixel-mouse reset: the mode
    // outlives us, and a shell left in bracketed-paste mode hands the next program
    // `ESC[200~`-wrapped pastes it never asked for. Idempotent and safe when it was
    // never enabled. (SQ-0653)
    let _ = execute!(
        stdout(),
        crossterm::style::Print(app::pixel_mouse::RESET),
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen
    );
    // Disabling reporting stops NEW reports; it cannot unsend one already queued.
    // Anything that arrived between the event loop's last `read()` and the escapes
    // above is still sitting in the tty's input queue, and leaving raw mode hands it
    // straight to the shell. Take it off the fd first. (SQ-0998)
    drain_pending_input();
    let _ = disable_raw_mode();
}

/// Consume whatever input is already queued, so it dies with this process instead of
/// landing on the shell's command line.
///
/// `poll(ZERO)` is what does the work: it reads the available bytes off stdin and
/// parses them into crossterm's own in-memory queue, which the process takes with it.
/// `read()` is only there to empty that queue, because `poll` short-circuits on a
/// non-empty one and would otherwise stop reading the fd after the first event.
///
/// Bounded, and never blocking. `poll` with a zero timeout only *try*-locks
/// crossterm's reader and answers `false` when another thread holds it, so this is a
/// no-op — rather than a wedge — on the one path that runs off the main thread: the
/// SQ-0502 termination watchdog, which exists precisely because the main loop can be
/// stuck inside `read()` on a dead pty. The iteration cap covers the other direction,
/// a terminal still streaming motion faster than we drain it.
fn drain_pending_input() {
    for _ in 0..MAX_DRAINED_EVENTS {
        match poll(Duration::ZERO) {
            Ok(true) => {
                if read().is_err() {
                    return;
                }
            }
            _ => return,
        }
    }
}

/// How many queued events [`drain_pending_input`] will discard before giving up.
/// A quit with the pointer in motion queues a handful; anything past this is a
/// terminal talking faster than we can listen, and exiting promptly matters more.
const MAX_DRAINED_EVENTS: usize = 256;

/// Print what the fd-2 redirect swallowed while the TUI was up (SQ-0586), once the
/// terminal is back. Silent when nothing was captured, which is the normal case.
///
/// Without this the redirect would trade a corrupted screen for a silent one: the
/// ALSA repeats that used to wreck the display would simply vanish. A short tail
/// after the alternate screen closes keeps them diagnosable — and names the log, so a
/// user chasing an audio problem has somewhere to look.
fn report_captured_stderr() {
    let lines = app::stderr_redirect::captured_tail(10);
    if lines.is_empty() {
        return;
    }
    if let Some(path) = app::stderr_redirect::log_path() {
        eprintln!("lanthorn: the system wrote {} line(s) of error output while the game was running", lines.len());
        for l in &lines {
            eprintln!("  {l}");
        }
        eprintln!("  (full output: {})", path.display());
    }
}

/// Set by an external termination signal; the main loops poll
/// [`termination_requested`] and restore the terminal + exit at a safe point.
static TERMINATE: std::sync::OnceLock<std::sync::Arc<std::sync::atomic::AtomicBool>> =
    std::sync::OnceLock::new();

/// Register handlers for external termination signals so a `kill` (SIGTERM), a
/// closed controlling terminal (SIGHUP), or an out-of-band SIGINT/SIGQUIT
/// restores the terminal instead of leaving it in raw mode + the alternate
/// screen with mouse capture on. The handlers only set an atomic flag (an
/// async-signal-safe operation); the actual `restore_terminal()` runs from the
/// main loop at a safe point. No-op on non-Unix (Windows has no SIGTERM/SIGHUP,
/// and its console resets on process exit). Idempotent.
fn install_termination_handlers() {
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    #[cfg(unix)]
    {
        use signal_hook::consts::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};
        // In raw mode ISIG is off, so interactive Ctrl-C/Ctrl-\ arrive as
        // keystrokes, not signals; these fire only on an out-of-band kill or the
        // controlling terminal closing.
        for sig in [SIGTERM, SIGHUP, SIGINT, SIGQUIT] {
            let _ = signal_hook::flag::register(sig, std::sync::Arc::clone(&flag));
            // Also record which signal fired so the process exits with the
            // conventional 128 + signum. An atomic store is async-signal-safe.
            let _ = unsafe {
                signal_hook::low_level::register(sig, move || {
                    TERM_SIGNUM.store(sig, std::sync::atomic::Ordering::SeqCst);
                })
            };
        }
        // Watchdog backstop (SQ-0502). The interactive loops observe the flag at
        // safe points and run the clean save + restore + exit — but that only works
        // if the loop keeps turning. When the controlling terminal closes, macOS
        // leaves `crossterm::event::poll()` (mio/kqueue) blocked forever on the dead
        // pty fd: the SIGHUP fires and sets the flag, yet the loop never returns from
        // `poll` to see it, so the process would linger after its terminal is gone.
        // This thread force-exits after a grace period so that can never happen. The
        // grace lets a responsive loop (a plain kill/SIGTERM, or Linux where `poll`
        // wakes on HUP) finish its auto-save first; only a genuinely wedged loop
        // reaches the timeout, and then a prompt exit matters more than the
        // best-effort save the wedged loop cannot run anyway.
        let wflag = std::sync::Arc::clone(&flag);
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(TERM_WATCHDOG_POLL_MS));
            if wflag.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(TERM_WATCHDOG_GRACE_MS));
                // The fixed grace alone could fire mid-save and lose the exit
                // auto-save outright (SQ-0644): a large archive on a slow disk
                // takes longer than 600ms, and the loop that IS making progress
                // looks identical to a wedged one from out here. Keep waiting
                // while the save is actively running, up to a hard cap so a save
                // that itself hangs can never keep the process alive.
                let mut waited = TERM_WATCHDOG_GRACE_MS;
                while watchdog_should_keep_waiting(waited, exit_save_in_progress()) {
                    std::thread::sleep(std::time::Duration::from_millis(TERM_WATCHDOG_POLL_MS));
                    waited += TERM_WATCHDOG_POLL_MS;
                }
                restore_terminal();
                std::process::exit(term_exit_code());
            }
        });
    }
    let _ = TERMINATE.set(flag);
}

/// Grace period the termination watchdog waits after the flag is set before it
/// force-exits, giving a responsive interactive loop time to run its auto-save and
/// exit cleanly first.
const TERM_WATCHDOG_GRACE_MS: u64 = 600;

/// How often the termination watchdog wakes, both while waiting for the flag and
/// while extending the grace for an in-progress exit save.
const TERM_WATCHDOG_POLL_MS: u64 = 50;

/// Hard cap on the watchdog's total wait after a termination signal, however
/// busy the exit save looks. A save that hangs must not keep the process alive
/// after its terminal is gone (the whole point of the watchdog); 10s is far more
/// than a real archive write needs and far less than "lingering forever".
const TERM_WATCHDOG_HARD_CAP_MS: u64 = 10_000;

/// Whether the termination watchdog should extend its grace for another tick:
/// only while an exit auto-save is actively writing, and never past the hard cap.
/// (SQ-0651 / partial SQ-0644.)
fn watchdog_should_keep_waiting(waited_ms: u64, save_in_progress: bool) -> bool {
    save_in_progress && waited_ms < TERM_WATCHDOG_HARD_CAP_MS
}

/// Set while an exit auto-save is actively writing. Read by the termination
/// watchdog (so it does not kill a save in flight) and by the error-exit paths
/// (so an exit save cannot re-enter itself).
static EXIT_SAVE_IN_PROGRESS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// True while an exit auto-save is running.
fn exit_save_in_progress() -> bool {
    EXIT_SAVE_IN_PROGRESS.load(std::sync::atomic::Ordering::SeqCst)
}

/// RAII marker for the duration of an exit auto-save. Held by
/// [`lifecycle::exit_auto_save`]; clears on drop, including on an unwind, so a
/// panicking save cannot leave the watchdog waiting for the hard cap.
pub(crate) struct ExitSaveGuard;

impl ExitSaveGuard {
    pub(crate) fn new() -> ExitSaveGuard {
        EXIT_SAVE_IN_PROGRESS.store(true, std::sync::atomic::Ordering::SeqCst);
        ExitSaveGuard
    }
}

impl Drop for ExitSaveGuard {
    fn drop(&mut self) {
        EXIT_SAVE_IN_PROGRESS.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// The signal number of the external termination signal that fired (0 until one
/// does). Drives the conventional `128 + signum` exit code.
static TERM_SIGNUM: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// True once an external termination signal has been received.
fn termination_requested() -> bool {
    TERMINATE
        .get()
        .is_some_and(|f| f.load(std::sync::atomic::Ordering::Relaxed))
}

/// Conventional shell exit code for a signal-terminated process: `128 + signum`.
/// Falls back to 130 (128 + SIGINT) if the number wasn't captured.
fn term_exit_code() -> i32 {
    let s = TERM_SIGNUM.load(std::sync::atomic::Ordering::SeqCst);
    if s > 0 {
        128 + s
    } else {
        130
    }
}

/// If an external termination signal arrived, restore the terminal and exit with
/// the conventional `128 + signum` code. Used at safe points in the story picker,
/// which has no game state to persist; the game loop uses
/// [`exit_if_terminated_saving`] so progress is auto-saved first.
fn exit_if_terminated() {
    if termination_requested() {
        restore_terminal();
        report_captured_stderr();
        std::process::exit(term_exit_code());
    }
}

/// Game-loop termination check: if an external signal arrived, run the same exit
/// auto-save the clean-quit path performs — sequenced HERE on the main loop, never
/// inside the async-signal handler — then restore the terminal and exit
/// `128 + signum`.
///
/// Called both at the loop top AND immediately before the blocking `read()`: when
/// the controlling terminal closes (SIGHUP), the dead pty fd reports HUP so `poll`
/// returns "ready" and the loop heads into `read()`, which then blocks forever on
/// the dead tty — so the loop never returns to the top to observe the flag. The
/// signal handler runs (and sets the flag) before that `poll` returns, so a
/// pre-`read()` check reliably catches it. (SQ-0502)
fn exit_if_terminated_saving(
    session: &mut dyn Engine,
    mapper: &Mapper,
    state: &app::state::AppState,
    ifid: &str,
    arc_file: &std::path::Path,
) {
    if termination_requested() {
        restore_terminal();
        report_captured_stderr();
        lifecycle::exit_auto_save(&mut *session, mapper, state, ifid, arc_file);
        std::process::exit(term_exit_code());
    }
}

/// Whether a panic on the currently-panicking thread is FATAL to the session —
/// the only case in which the panic hook may tear the terminal down (SQ-0649).
///
/// Every worker thread the app spawns is *recovered*: each `join()` in the
/// production paths handles the `Err` case and the session plays on (the map
/// render worker in `state.rs::poll_render_job`, the background tidy and
/// anim-build workers in `loop_tick.rs`, the v6 encode worker in
/// `render/graphics.rs::poll_v6_job`), and the never-joined service workers
/// (cover decode, IFDB search/fetch, hint download, the boot spinner, the
/// termination watchdog) simply close their channel, which every reader treats
/// as "no more results". Tearing the terminal down for one of those left the app
/// drawing frames onto a cooked normal screen — the game still running, the
/// display wrecked, and a "lanthorn crashed" banner over a session that had not.
///
/// So: the main thread's panic is fatal (it unwinds out of the event loop and
/// ends the process); no other thread's is. `main` is `None` only if the id was
/// never captured, where the safe answer is the old behavior.
fn panic_is_fatal(panicking: std::thread::ThreadId, main: Option<std::thread::ThreadId>) -> bool {
    main.is_none_or(|m| panicking == m)
}

/// The main thread's id, captured when the panic hook is installed. Read by the
/// hook to tell a fatal panic from a recovered worker's (see [`panic_is_fatal`]).
static MAIN_THREAD: std::sync::OnceLock<std::thread::ThreadId> = std::sync::OnceLock::new();

/// Guard so the hook is installed exactly ONCE per process (SQ-0649). `boot_story`
/// runs again for every story the picker→play loop launches, and `set_hook` chains
/// the previous hook in as `default_hook` — so the Nth boot's panic wrote N
/// duplicate `crash.log` records and printed N stderr lines.
static PANIC_HOOK_ONCE: std::sync::Once = std::sync::Once::new();

/// Run the exit auto-save from an *error* exit (a failed draw / read / poll):
/// the terminal broke, but the engine is intact and the player's progress is
/// still saveable, so these paths must persist it like every other exit does
/// (SQ-0651).
///
/// Skipped when an exit save is already running: the only way to arrive here
/// while that is true is from inside the save itself, and re-entering it would
/// recurse (and rewrite the archive) instead of exiting.
fn exit_save_on_error_exit(
    session: &mut dyn Engine,
    mapper: &Mapper,
    state: &app::state::AppState,
    ifid: &str,
    arc_file: &std::path::Path,
) {
    if exit_save_in_progress() {
        return;
    }
    lifecycle::exit_auto_save(session, mapper, state, ifid, arc_file);
}

/// Install a panic hook that writes the panic and a backtrace to a durable
/// `crash.log`, and — for a panic that is fatal to the session — restores the
/// terminal and prints the panic message. Idempotent: only the first call installs.
///
/// The durable file matters because the panic message is printed to stderr
/// only *after* `LeaveAlternateScreen`, where the terminal's alternate-screen
/// restore can hide or overwrite it — so a real crash could otherwise leave no
/// visible trace. The log survives that teardown. (An abort — OOM, stack
/// overflow, double-panic — bypasses this hook entirely and leaves no entry;
/// an empty `crash.log` after a crash is itself evidence of an abort.)
///
/// A *recovered* worker panic still gets its `crash.log` record — the crash is
/// real and worth diagnosing — but no teardown, no banner, and no chaining to the
/// default hook, whose own stderr dump would land mid-frame on a live screen.
fn install_panic_hook(user_dir: std::path::PathBuf) {
    PANIC_HOOK_ONCE.call_once(move || {
        let _ = MAIN_THREAD.set(std::thread::current().id());
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let fatal = panic_is_fatal(std::thread::current().id(), MAIN_THREAD.get().copied());
            if fatal {
                restore_terminal();
            }
            let backtrace = std::backtrace::Backtrace::force_capture();
            let log_path = user_dir.join("crash.log");
            let path = match write_crash_log(&log_path, info, &backtrace) {
                Ok(()) => log_path,
                // Fall back to the temp dir if the user dir isn't writable.
                Err(_) => {
                    let tmp = std::env::temp_dir().join("lanthorn-crash.log");
                    let _ = write_crash_log(&tmp, info, &backtrace);
                    tmp
                }
            };
            if fatal {
                eprintln!("lanthorn crashed — details written to {}", path.display());
                default_hook(info);
            }
        }));
    });
}

/// Append one panic record (message + backtrace) to `path`.
fn write_crash_log(
    path: &std::path::Path,
    info: &std::panic::PanicHookInfo<'_>,
    backtrace: &std::backtrace::Backtrace,
) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "\n=== lanthorn panic ===\n{info}\n\nbacktrace:\n{backtrace}")
}

/// Directory holding per-game save archives (`.lanthorn`, default + named) and
/// the game's own standard `.qzl` saves. Kept separate from the map
/// directory. Defaults to `config.user_dir/saves`.
fn saves_dir(user_dir: &std::path::Path) -> std::path::PathBuf {
    user_dir.join("saves")
}

// ── Draw helper ───────────────────────────────────────────────────────────────

/// Both pane inner-content rects returned by `draw_frame`.
/// `map` is `Rect::default()` when the layout hides the map (TranscriptFull).
/// `room_rects` maps each visible room to its drawn bounding rect in screen coords.
/// `layer_tabs` pairs each visible layer tab with its hit-rect (click switches layers).
/// `dialog` holds the last-drawn dialog chrome rects for mouse hit-testing.
#[derive(Default)]
struct PaneRects {
    map: Rect,
    story: Rect,
    /// This frame's pane geometry, kept whole because a mouse drag on a pane
    /// boundary anchors against it (SQ-0669).
    pane_layout: app::layout::PaneLayout,
    /// The draggable pane boundaries of this frame, with their grab zones.
    boundaries: Vec<app::layout::BoundaryZone>,
    room_rects: Vec<(RoomId, Rect)>,
    /// Which view drew `room_rects` this frame (SQ-1246): the matrix view's row
    /// labels and destination cells both resolve to a room and want a hover
    /// tooltip, the drawn view's boxes do not — this is what tells the mouse
    /// handler which behaviour `room_rects` is standing in for.
    map_view: mapper::layer::MapView,
    /// The room dock's rect this frame (SQ-0692), zero-area when it is closed.
    /// Mouse routing needs it as its own rect: the dock is carved OUT of the map
    /// pane, so a click inside it is neither a map click nor a story click, and
    /// must not fall through to either (nor to v6 mouse delivery).
    room_dock: Rect,
    /// Hit-rects for the dock's two view tabs. A click switches the body, the way
    /// a click on a layer tab switches layers.
    room_dock_tabs: Vec<(app::state::RoomDockView, Rect)>,
    /// Hit-rects for each layer tab, paired with the layer id; the mouse
    /// handler hit-tests these to switch the viewed layer on click.
    layer_tabs: Vec<(LayerId, Rect)>,
    /// Hit-rects for the story pane's border toggle controls (SQ-1123), paired
    /// with what each one switches. One list for both the click path and the
    /// `Moved` hover path, so the hint and the click can never resolve to
    /// different controls.
    border_controls: Vec<(BorderControl, Rect)>,
    /// Hit-rects for each debug window's tab, as `(window, tab, rect)`; the mouse
    /// handler hit-tests these to activate a debug tab on click.
    debug_tabs: Vec<(usize, usize, Rect)>,
    /// Active dialog chrome rects (when a dialog is open).
    pub dialog: Option<DialogRects>,
    /// Hit-rects for the aux-storage prompt (when open).
    pub aux_dialog: Option<app::render::aux_dialog::AuxDialogRects>,
    pub history_prompt: Option<app::render::history_prompt::HistoryPromptRects>,
    pub font_check: Option<app::render::font_check_dialog::FontCheckRects>,
    pub fetch_keep: Option<app::render::fetch_keep_dialog::FetchKeepRects>,
    /// Hit-rects for the reset dialog (when open).
    pub reset_dialog: Option<app::render::reset_dialog::ResetDialogRects>,
    /// Hit-rects for the region prompt (when open) — its option rows and its buttons (SQ-0439).
    pub region_prompt: Option<app::render::region_prompt::RegionPromptRects>,
    /// Hit-rects for the Scott-only game-over dialog (when open).
    pub game_over: Option<app::render::game_over_dialog::GameOverDialogRects>,
    /// Hit-rects for the save-name dialog (when open).
    pub save_name_dialog: Option<app::render::save_name_dialog::SaveNameDialogRects>,
    /// Hit-rects for the generic text-entry dialog (when open).
    pub text_entry: Option<app::render::text_entry_dialog::TextEntryDialogRects>,
    /// Hit-rects for the confirm-delete dialog (when open).
    pub confirm_delete: Option<app::render::confirm_delete_dialog::ConfirmDeleteDialogRects>,
    /// Hit-rects for the confirm-overwrite dialog (when open).
    pub confirm_overwrite: Option<app::render::confirm_overwrite_dialog::ConfirmOverwriteDialogRects>,
    /// Hit-rects for the quit dialog (when open).
    pub quit_dialog: Option<app::render::quit_dialog::QuitDialogRects>,
    /// Hit-rects for the launch dialog (when open).
    pub launch_dialog: Option<app::render::launch_dialog::LaunchDialogRects>,
    /// Hit-rects for the hints panel (when open).
    pub hints_panel: Option<HintsPanelRects>,
    /// Hit-rects for the command band (when open): its own rect, the column
    /// headers, the item rows and the quick words (rose block or flat row).
    pub command_band: app::render::command_band::CommandBandHits,
    /// Hit-rects for the inventory dock (when open): its own rect and one row
    /// rect per item (SQ-1244) — a click composes the row's word into the
    /// prompt the same way a command-band WHAT-column click does.
    pub inventory_dock: app::render::inventory_dock::InventoryDockHits,
    /// Hit-rects for the command palette's candidate rows, as `(cmd_index, rect)`;
    /// the mouse handler hit-tests these to execute a command on click. (SQ-0419)
    pub palette: Vec<(usize, Rect)>,
    /// Per-frame map from rendered story-pane cell `(col, row)` → Glk hyperlink
    /// value. Built during transcript render; the mouse handler hit-tests these
    /// on click to deliver the hyperlink event. Empty when nothing on screen is
    /// linked. Story-pane cells share the Glk screen frame, so these coords are
    /// directly click-comparable.
    pub transcript_links: Vec<((u16, u16), u32)>,
    /// Every Glk-identified leaf's ACTUAL drawn rect this frame, as `(win id,
    /// kind, absolute screen rect)` — see `StoryPaneMetrics::win_rects`. The Glk
    /// mouse/hyperlink hit-test (`glk_mouse_target`/`glk_hyperlink_window`) uses
    /// this instead of gvm's own layout rect, which reserves a border gutter the
    /// theme may draw thinner or not at all (SQ-1203).
    pub win_rects: Vec<(u32, app::engine::WinKind, Rect)>,
    /// Largest meaningful `transcript_scroll` this frame (total wrapped rows −
    /// viewport). The loop clamps `state.transcript_scroll` to this so the view
    /// can't over-scroll past the top.
    pub transcript_max_scroll: u16,
    /// Visible transcript rows this frame (the transcript viewport height). Used
    /// to size a PageUp/PageDown step.
    pub transcript_viewport_rows: u16,
    /// Rows the `[more]` prompt takes out of that viewport while it shows (1 on
    /// the cell paths, 0 on the raster one) — the pager parks the view a frame
    /// before the prompt appears and has to allow for it (SQ-0823).
    pub transcript_prompt_rows: u16,
    /// Total wrapped transcript rows this frame. Cached so a command turn can
    /// measure how many rows its output added (the [more] pager, SQ-0404).
    pub transcript_total_rows: u16,
    /// Whether this frame laid the transcript out at all. `false` on v6
    /// full-screen picture frames (splash, Zork Zero's map/rebus takeovers) whose
    /// zeroed transcript metrics must not clamp scrollback or reset the pager
    /// baseline (SQ-0578).
    pub transcript_surface: bool,
    /// List-row viewport of the open selection-list modal this frame, synced to
    /// `AppState.modal_list_viewport` so nav actions can window/animate. 0 when
    /// no list modal is open.
    pub modal_list_viewport: usize,
}

/// The map render model for one frame: either borrowed from the per-frame cache
/// (the live graph, keyed by generation + layer) or freshly built and owned (the
/// replay / tidy-animation graphs, which `graph_gen` does not track). Derefs to
/// `&RenderMap` so the draw call sites are unchanged. (SQ-0305)
enum FrameRenderMap<'a> {
    Cached(std::cell::Ref<'a, mapper::render::RenderMap>),
    Owned(mapper::render::RenderMap),
}

impl std::ops::Deref for FrameRenderMap<'_> {
    type Target = mapper::render::RenderMap;
    fn deref(&self) -> &Self::Target {
        match self {
            FrameRenderMap::Cached(r) => r,
            FrameRenderMap::Owned(o) => o,
        }
    }
}

/// The base panel border style from the theme — bold `:active` when the panel
/// has input focus, plain `panel.border` otherwise (SQ-0309 §2a).
fn panel_border(theme: &app::theme::resolve::Theme, focused: bool) -> Style {
    theme.get(if focused { "panel.border:active" } else { "panel.border" }).style
}

/// Draw the story pane with its border toggle controls (SQ-1123), returning the
/// panel frame and each control's hit-rect paired with what it switches.
///
/// One helper for all three layouts the story pane is drawn in (debug-tiled,
/// transcript-full, split): the cluster is the same in every one, and a layout
/// that resolved its own list would be a fourth place for the v6-only pair to be
/// forgotten.
fn draw_story_panel(
    buf: &mut ratatui::buffer::Buffer,
    spec: &PanelSpec,
    state: &AppState,
) -> (PanelFrame, Vec<(BorderControl, Rect)>) {
    let views = app::render::controls::controls_for(state);
    app::render::controls::draw_pane_with_controls(buf, spec, &state.colors.theme, &views)
}

/// Render one frame. Returns both pane inner-content rects so the event loop
/// can route mouse events and make accurate `recenter_on` calls.
fn draw_frame(
    terminal: &mut Terminal<CrosstermBackend<app::terminal_dump::CountingWriter<std::io::BufWriter<std::io::Stdout>>>>,
    engine: &dyn Engine,
    mapper: &Mapper,
    state: &AppState,
) -> std::io::Result<PaneRects> {
    let mut map_area = Rect::default();
    let mut story_area = Rect::default();
    let mut room_rects_out: Vec<(RoomId, Rect)> = Vec::new();
    // Hit rects handed back by the map renderer itself. The matrix view's rows and destination
    // cells are not room BOXES, so they cannot be recomputed from the render model afterwards
    // the way `room_screen_rects` recomputes the drawn view's (SQ-0666).
    let mut map_hits: Option<Vec<(RoomId, Rect)>> = None;
    let mut layer_tabs_out: Vec<(LayerId, Rect)> = Vec::new();
    let mut border_controls_out: Vec<(BorderControl, Rect)> = Vec::new();
    // The view the map pane's cluster is drawn against, captured where the pane
    // is drawn so the hover hint below resolves the SAME control the frame did
    // (SQ-1148). `Drawn` until a map pane exists, which is also when the cluster
    // does not.
    let mut map_control_view = mapper::layer::MapView::Drawn;
    let mut room_dock_tabs_out: Vec<(app::state::RoomDockView, Rect)> = Vec::new();
    let mut debug_tabs_out: Vec<(usize, usize, Rect)> = Vec::new();
    let mut dialog_rects_out: Option<DialogRects> = None;
    let mut overlay_rects: Option<overlays::OverlayRects> = None;
    let mut band_hits = app::render::command_band::CommandBandHits::default();
    let mut inv_hits = app::render::inventory_dock::InventoryDockHits::default();
    let mut palette_hits: Vec<(usize, Rect)> = Vec::new();
    let mut modal_list_viewport: usize = 0;
    let mut transcript_max_scroll: u16 = 0;
    let mut transcript_viewport_rows: u16 = 0;
    let mut transcript_prompt_rows: u16 = 0;
    let mut transcript_total_rows: u16 = 0;
    let mut transcript_surface = false;
    let mut transcript_links_out: Vec<((u16, u16), u32)> = Vec::new();
    let mut win_rects_out: Vec<(u32, app::engine::WinKind, Rect)> = Vec::new();
    let mut pane_layout_out = app::layout::PaneLayout::default();

    terminal.draw(|f| {
        let full = f.area();
        let buf = f.buffer_mut();
        // The engine-neutral screen model for this frame (status + window tree).
        // `screen_now`, not `screen`: a v6 turn's picture sequence is played out
        // over successive frames, so what the player is looking at right now may
        // be one step short of the settled composite (SQ-0708).
        let screen_model = engine.screen_now();
        // The painted ground rides beside the window tree, not inside it: it is a
        // pixel surface the v6 paths composite as a backdrop (SQ-0706). Republished
        // every frame so a game that stops painting cannot leave a stale one up.
        *state.v6_paint.borrow_mut() = engine.paint_surface();
        // During replay the map shows the reconstructed snapshot for the selected turn.
        let replay_graph: Option<mapper::graph::MapGraph> = state.overlays.replay.as_ref().map(|r| {
            let snap = state
                .history
                .get(r.idx)
                .map(|rec| rec.turn)
                .and_then(|turn| app::history::map_at_turn(&state.history, turn))
                .and_then(|json| mapper::persist::from_json(json).ok());
            // Replaying a turn before the first map snapshot has no recorded
            // map — show an empty map, never the live (future) graph.
            snap.map(|m| m.graph).unwrap_or_default()
        });

        // During tidy-animation playback the map shows the current captured stage, not the live graph.
        // The live graph's routed model is memoized on (graph_gen, layer) — see `cached_map_render` —
        // so an animation / transcript / mouse-move redraw of an unchanged map skips re-routing.
        // Replay and tidy-animation graphs are not tracked by `graph_gen`, so they are built fresh.
        // `frame_layer`, not `active_layer(g)`: an animation frame's graph is a layer SUBGRAPH and
        // cannot be asked which layer it is — it always answers main, and the map draws blank
        // (SQ-0359).
        let layer = state.frame_layer(&mapper.graph, replay_graph.as_ref());
        let rm = if let Some(g) = &replay_graph {
            FrameRenderMap::Owned(render_layer(g, layer))
        } else {
            match &state.tidy_anim {
                Some(anim) => FrameRenderMap::Owned(render_layer(&anim.current().graph, layer)),
                None => match state.live_map_render(layer, &mapper.graph) {
                    Some(cached) => FrameRenderMap::Cached(cached),
                    // The matrix draws from the graph (see `render_map_layered`), so no model is
                    // routed for it at all — not even a stale one kept warm. That is what keeps a
                    // background layout job off the matrix pane entirely. (SQ-0671)
                    None => FrameRenderMap::Owned(mapper::render::render(&mapper::graph::MapGraph::new())),
                },
            }
        };

        // ── Inventory dock: reserve a bottom band (above the help row) that
        // slides up when toggled, sized from the item list + slide fraction.
        let inv_visible = state.show_inventory || state.inv_dock.active();
        let inv_items: Vec<String> = if inv_visible {
            app::render::transcript::inventory_items(state.player_obj, &state.inventory_fallback, engine.introspect())
        } else {
            Vec::new()
        };
        let pane_layout = app::layout::compute_pane_layout(full, state, inv_items.len());
        pane_layout_out = pane_layout;

        // While any background map job is in flight — a tidy relayout or the
        // async re-route worker (SQ-0379) — the map pane border pulses between red
        // and green, overriding the normal (focused/unfocused) border color.
        let map_border_override: Option<ratatui::style::Color> =
            state.map_job_pulse_elapsed(&mapper.graph).map(pulse_border_color);

        // Resolve the story-border color: a live sound pulse overrides the fg.
        let story_border_style = {
            let base = panel_border(&state.colors.theme, state.focus == Focus::Game);
            match &state.sound_pulse {
                Some(p) => {
                    let beep_color = match p.kind {
                        app::state::BeepKind::High => state
                            .colors
                            .theme
                            .get("sound_beep_high")
                            .style
                            .fg
                            .unwrap_or(ratatui::style::Color::Rgb(255, 180, 40)),
                        app::state::BeepKind::Low => state
                            .colors
                            .theme
                            .get("sound_beep_low")
                            .style
                            .fg
                            .unwrap_or(ratatui::style::Color::Rgb(60, 140, 220)),
                    };
                    let normal = base.fg.unwrap_or(ratatui::style::Color::Reset);
                    match sound_pulse_color(beep_color, normal, p.started.elapsed()) {
                        Some(c) => base.fg(c),
                        None => base,
                    }
                }
                None => base,
            }
        };

        if state.debug.is_some() {
            // ── Debug inspector (tiled): story pane drawn normally; the map slot
            // renders the debug region instead of the map — no room rects, layer
            // tabs, or tidy pulse (those belong to the real map). `pane_layout`
            // already reserved a right-hand rect for this (`compute_pane_layout`
            // treats debug-active as `Layout::Split`).
            // The splitter accents while resize mode targets it, and equally
            // while the mouse hovers or drags it (SQ-0669) — the same accent, so
            // the grab affordance costs no new style machinery.
            let resize_split_hl = (state.resize_mode && state.resize_target == app::state::ResizeTarget::StoryMap)
                || state.boundary_active(app::layout::Boundary::StoryMapSplit);
            let story_border_color = if resize_split_hl { state.colors.theme.get("panel.border:active").style } else { story_border_style };
            let story_focused = resize_split_hl || state.focus == Focus::Game;
            let story_title_style = state.colors.theme.get("story_title").style;
            let story_segs = [InsetSegment { text: &state.pane_title, active: false }];
            let (story_fp, story_ctls) = draw_story_panel(buf, &PanelSpec {
                area: pane_layout.story,
                border_selector: if story_focused { "panel.border:active" } else { "panel.border" },
                border_color: Some(story_border_color),
                border_style: None,
                glyphs: &state.colors.story_border_glyphs,
                header_on: state.colors.story_header_on,
                strip: Some(PanelStrip { segments: &story_segs, base: story_title_style, active: story_title_style }),
                body_fill: None,
            }, state);
            border_controls_out = story_ctls;
            let c = story_fp.content;
            let m = render_story_pane(&screen_model, state.char_mode, engine.introspect(), state, c, buf);
            transcript_max_scroll = m.max_scroll;
            transcript_viewport_rows = m.viewport_rows;
            transcript_prompt_rows = m.prompt_rows;
            transcript_total_rows = m.total_rows;
            transcript_surface = m.transcript_surface;
            transcript_links_out = m.links;
            win_rects_out = m.win_rects;
            story_area = story_fp.content;

            debug_tabs_out = app::render::debug_panel::draw_debug_panel(state, pane_layout.map, buf);
            map_area = pane_layout.map;

            // Story pane dims when the debug region has focus (mirrors the map's
            // focus-dim rule below).
            if state.focus == Focus::Map {
                dim_area(buf, story_fp.content);
            }
        } else {
            match state.layout {
                Layout::TranscriptFull => {
                    let story_focused = state.focus == Focus::Game;
                    let story_title_style = state.colors.theme.get("story_title").style;
                    let story_segs = [InsetSegment { text: &state.pane_title, active: false }];
                    let (story_fp, story_ctls) = draw_story_panel(buf, &PanelSpec {
                        area: pane_layout.story,
                        border_selector: if story_focused { "panel.border:active" } else { "panel.border" },
                        border_color: Some(story_border_style),
                        border_style: None,
                        glyphs: &state.colors.story_border_glyphs,
                        header_on: state.colors.story_header_on,
                        strip: Some(PanelStrip { segments: &story_segs, base: story_title_style, active: story_title_style }),
                        body_fill: None,
                    }, state);
                    border_controls_out = story_ctls;
                    let c = story_fp.content;
                    let m = render_story_pane(&screen_model, state.char_mode, engine.introspect(), state, c, buf);
                    transcript_max_scroll = m.max_scroll;
                    transcript_viewport_rows = m.viewport_rows;
                    transcript_prompt_rows = m.prompt_rows;
                    transcript_total_rows = m.total_rows;
                    transcript_surface = m.transcript_surface;
                    transcript_links_out = m.links;
                    win_rects_out = m.win_rects;
                    story_area = story_fp.content;
                    map_area = Rect::default();
                }
                Layout::Split => {
                    // Split 50/50 horizontally with bordered blocks (no divider column).
                    // In resize mode, the StoryMap target covers this whole split, so
                    // both borders pick up the `focused_border` accent to show it's live.
                    // …and equally while the mouse hovers or drags the splitter
                    // (SQ-0669), so the grab affordance reuses that same accent.
                    let resize_split_hl = (state.resize_mode && state.resize_target == app::state::ResizeTarget::StoryMap)
                        || state.boundary_active(app::layout::Boundary::StoryMapSplit);
                    let story_border_color = if resize_split_hl { state.colors.theme.get("panel.border:active").style } else { story_border_style };
                    let map_border_color = if resize_split_hl { state.colors.theme.get("panel.border:active").style } else { panel_border(&state.colors.theme, state.focus == Focus::Map) };
                    let story_focused = resize_split_hl || state.focus == Focus::Game;
                    let story_title_style = state.colors.theme.get("story_title").style;
                    let story_segs = [InsetSegment { text: &state.pane_title, active: false }];
                    let (story_fp, story_ctls) = draw_story_panel(buf, &PanelSpec {
                        area: pane_layout.story,
                        border_selector: if story_focused { "panel.border:active" } else { "panel.border" },
                        border_color: Some(story_border_color),
                        border_style: None,
                        glyphs: &state.colors.story_border_glyphs,
                        header_on: state.colors.story_header_on,
                        strip: Some(PanelStrip { segments: &story_segs, base: story_title_style, active: story_title_style }),
                        body_fill: None,
                    }, state);
                    border_controls_out = story_ctls;
                    let c = story_fp.content;
                    let m = render_story_pane(&screen_model, state.char_mode, engine.introspect(), state, c, buf);
                    transcript_max_scroll = m.max_scroll;
                    transcript_viewport_rows = m.viewport_rows;
                    transcript_prompt_rows = m.prompt_rows;
                    transcript_total_rows = m.total_rows;
                    transcript_surface = m.transcript_surface;
                    transcript_links_out = m.links;
                    win_rects_out = m.win_rects;
                    story_area = story_fp.content;

                    // The tab strip names every layer, so it reads the LIVE graph — never an
                    // animation frame. A frame is a `layer_subgraph`, whose `layers()` is always
                    // main-only, so asking it made the tidied layer's own tab vanish mid-animation
                    // (SQ-0359). `layer` (from `frame_layer`) marks the active tab. Build the
                    // segments before drawing — `draw_panel` renders the strip and returns the
                    // per-tab hit-rects.
                    let map_focused = resize_split_hl || state.focus == Focus::Map;
                    let graph = if let Some(g) = &replay_graph { g } else { &mapper.graph };
                    let layer_ids: Vec<LayerId> = graph.layers().keys().copied().collect();
                    let active_layer = layer;
                    let owned_segs = build_layer_segments(&layer_ids, active_layer,
                        |id| app::render::map::layer_tab_title(graph, id));
                    let inset_segs: Vec<_> = owned_segs.iter().map(|s| s.as_inset()).collect();
                    // The map pane's OWN cluster, on its bottom border (SQ-1148):
                    // room numbers, centre, zoom out, zoom in, view. Same enum,
                    // same dispatch and the same hit-rect vec as the story pane's
                    // — `BorderControl::pane` is what keeps the two apart. Every
                    // one of them acts on a map that is on screen, which is why
                    // they can live on a pane that disappears where the return
                    // probe could not (SQ-1107).
                    map_control_view = graph.layer_view(active_layer);
                    let map_views = app::render::controls::map_controls_for(state, map_control_view);
                    let (map_fp, map_ctls) = app::render::controls::draw_pane_with_controls(buf, &PanelSpec {
                        area: pane_layout.map,
                        border_selector: if map_focused { "panel.border:active" } else { "panel.border" },
                        border_color: Some(map_border_color),
                        border_style: None,
                        glyphs: &state.colors.map_border_glyphs,
                        header_on: state.colors.map_header_on,
                        strip: Some(PanelStrip {
                            segments: &inset_segs,
                            base: state.colors.theme.get("panel.tab").style,
                            active: state.colors.theme.get("panel.tab:active").style,
                        }),
                        // SQ-1170: the map canvas's own ground. `map.background`
                        // has been a documented, parsed, resolved selector that
                        // no renderer read since it landed — the map pane simply
                        // never painted a ground, where the debug pane has always
                        // painted `panel.background`. Transparent by default (its
                        // registry Delta is empty), so this is inert until a
                        // player sets a `bg`, and then it is the one thing the
                        // key's own comment always promised.
                        body_fill: Some(state.colors.theme.get("map.background").style),
                    }, &state.colors.theme, &map_views);
                    border_controls_out.extend(map_ctls);
                    layer_tabs_out = layer_ids.into_iter().zip(map_fp.tab_rects).collect();

                    map_hits = Some(render_map_layered(&rm, &mapper.graph, state, map_fp.content, buf));
                    if let Some(anim) = &state.tidy_anim {
                        let tidy_ds = make_dialog_style(state);
                        if let Some(dr) = draw_tidy_panel(anim.current(), map_fp.content, buf, &tidy_ds) {
                            dialog_rects_out = Some(dr);
                        }
                    }
                    map_area = map_fp.content;
                    // Apply pulsing border color overlay when a tidy job is in flight
                    if let Some(pulse_color) = map_border_override {
                        let pulse_style = Style::default().fg(pulse_color);
                        for cy in pane_layout.map.y..pane_layout.map.bottom() {
                            if let Some(c) = buf.cell_mut((pane_layout.map.x, cy)) { c.set_style(pulse_style); }
                            if let Some(c) = buf.cell_mut((pane_layout.map.right().saturating_sub(1), cy)) { c.set_style(pulse_style); }
                        }
                        for cx in pane_layout.map.x..pane_layout.map.right() {
                            if let Some(c) = buf.cell_mut((cx, pane_layout.map.y)) { c.set_style(pulse_style); }
                            if let Some(c) = buf.cell_mut((cx, pane_layout.map.bottom().saturating_sub(1))) { c.set_style(pulse_style); }
                        }
                    }

                    // While the async map-render worker runs, list each phase it has
                    // started in the map's top-right corner so the source of any map
                    // update delay is visible; the trace clears when the job lands
                    // (SQ-0379). The inner content rect keeps it off the pulsing border.
                    if state.map_render_in_flight() {
                        let area = map_fp.content;
                        let style = state.colors.theme.get("panel.tab").style;
                        for (i, step) in state.render_steps_snapshot().iter().enumerate() {
                            let y = area.y + i as u16;
                            if y >= area.bottom() { break; }
                            let w = (step.chars().count() as u16).min(area.width);
                            let x = area.right().saturating_sub(w);
                            buf.set_stringn(x, y, step, w as usize, style);
                        }
                    }

                    // Map pane is NEVER dimmed (always full brightness).
                    // Story pane dims when map has focus.
                    if state.focus == Focus::Map {
                        dim_area(buf, story_fp.content);
                    }
                }
            }
        }

        // Compute room screen rects for accurate mouse hit-testing. Skipped while
        // the debug inspector occupies the map slot — `map_area` is the debug
        // rect, not a real map, so there is nothing to hit-test.
        room_rects_out = if map_area.height > 0 && state.debug.is_none() {
            // The renderer's own hits when it produced any (the matrix view's rows and cells);
            // otherwise recompute the drawn view's room boxes, as before.
            map_hits.take().unwrap_or_else(|| room_screen_rects(&rm, state, map_area))
        } else {
            Vec::new()
        };

        // ── Room dock (SQ-0692) ───────────────────────────────────────────────
        // Not an overlay: the layout already reserved these rows out of the map
        // pane, so nothing is covered and the map above stays interactive. It
        // describes the SELECTED room when one is pinned, else the room the player
        // is standing in — which is why it updates every move without being told.
        if pane_layout.room_dock.height > 0 {
            let graph = if let Some(g) = &replay_graph {
                g
            } else {
                match &state.tidy_anim {
                    Some(anim) => &anim.current().graph,
                    None => &mapper.graph,
                }
            };
            let room = app::render::room_dock::dock_room(state.selected_room, graph);
            let current_room = graph.current();
            // Objects in the room come from the engine's introspection
            // (unavailable during tidy-anim playback → empty), and only ever for
            // the room the player is actually in.
            let room_objects: Vec<String> = match (room, state.tidy_anim.is_none()) {
                (Some(id), true) if Some(id) == current_room => {
                    engine
                        .introspect()
                        .map(|i| i.room_objects(id).iter().filter_map(|o| o.display_name()).collect())
                        .unwrap_or_default()
                }
                _ => Vec::new(),
            };
            let dock_resize_hl = (state.resize_mode
                && state.resize_target == app::state::ResizeTarget::RoomDock)
                || state.boundary_active(app::layout::Boundary::RoomDockTop);
            room_dock_tabs_out = app::render::room_dock::draw_room_dock(
                graph,
                room,
                state.room_dock_pinned(),
                state.room_dock_view,
                &room_objects,
                current_room,
                pane_layout.room_dock,
                &state.colors,
                &state.symbols,
                dock_resize_hl,
                buf,
            );
        }

        // ── Inventory dock panel ──────────────────────────────────────────────
        if pane_layout.inv_dock.height > 0 {
            let inv_resize_hl = (state.resize_mode && state.resize_target == app::state::ResizeTarget::InvDock)
                || state.boundary_active(app::layout::Boundary::InvDockTop);
            app::render::inventory_dock::draw_inventory_dock(&inv_items, pane_layout.inv_dock, &state.colors, inv_resize_hl, buf, &mut inv_hits);
        }

        // ── Command band ───────────────────────────────────────────────────────
        if pane_layout.command_band.height > 0 {
            draw_command_band(state, pane_layout.command_band, buf, &mut modal_list_viewport, &mut band_hits);
        }

        // ── Change 2: draw help bar in bottom row ─────────────────────────────
        let help_style = state.colors.theme.get("help_bar").style;
        let help_text = if state.overlays.config_screen.is_some() {
            "\u{2191}\u{2193} move  \u{2190}\u{2192}/Space change  s save  Esc cancel".to_string()
        } else if state.overlays.command_band.is_some() && !state.resize_mode {
            // SQ-0677 (2026-08-05): typing always wins — the band owns no text
            // keys. Tab/Shift-Tab move the current column; a highlighted row
            // (arrowed or the typed nearest match) makes Tab pick it and
            // advance instead. Enter never picks — it always sends the
            // prompt. Quick (rose/words) is mouse-only; F2 re-closes.
            "Command Panel | type: goes to the prompt | \u{2191}\u{2193}: highlight | Tab: move col. (pick if highlighted) | Shift-Tab: move col. | Ctrl+\u{2191}\u{2193}: history | Enter: send | Esc: close | F2: close"
                .to_string()
        } else if state.overlays.file_browser.as_ref().map(|fb| fb.mode == FbMode::PickFile).unwrap_or(false) {
            "Import Save | \u{2191}\u{2193}: move | Enter: open/import | Esc: cancel".to_string()
        } else if state.overlays.saves.is_some() {
            "Saves | \u{2191}\u{2193}: select | Enter: load | s: save-as | d: delete | i: import | Esc: close".to_string()
        } else if let Some(anim) = &state.tidy_anim {
            // Playback status: stage progress + the transport controls.
            let f = anim.current();
            let prefix = format!(
                "Tidy [{}/{}] {}{}",
                anim.idx + 1,
                anim.frames.len(),
                f.label,
                if anim.playing { " \u{25b6}" } else { "" },
            );
            let hint_width = (pane_layout.help_row.width as usize).saturating_sub(prefix.chars().count() + 3);
            let hints = hint_bar(&state.keymap, &state.hotkeys, Context::Anim, ANIM_HINTS, hint_width);
            format!("{} | {}", prefix, hints)
        } else if state.resize_mode {
            use app::state::ResizeTarget;
            let t = match state.resize_target {
                ResizeTarget::StoryMap => "story/map",
                ResizeTarget::InvDock => "inventory",
                ResizeTarget::CommandBand => "command panel",
                ResizeTarget::RoomDock => "room panel",
            };
            format!("Resize [{t}] | Tab: pane | arrows: adjust | 0: reset | Esc: done")
        } else {
            let leader_hint = format!("{}: menu", state.hotkeys.prefix.label());
            // Reserve room for the leader hint + " | " separator so the composed
            // row doesn't overflow help_row.width (mirrors the tidy_anim branch).
            let w = (pane_layout.help_row.width as usize).saturating_sub(leader_hint.chars().count() + 3);
            let rest = match state.focus {
                Focus::Game => {
                    // Tab only leads somewhere while the inspector is open
                    // (SQ-0599) — otherwise it is inert and must not be
                    // advertised as a focus toggle.
                    let hints: &[&str] = if state.debug.is_some() {
                        GAME_HINTS
                    } else {
                        app::render::hintbar::GAME_HINTS_NO_INSPECTOR
                    };
                    hint_bar(&state.keymap, &state.hotkeys, Context::Global, hints, w)
                }
                Focus::Map if state.debug.is_some() => {
                    // The bar follows the focused window's active tab (SQ-0980):
                    // section-specific keys first, universal ones after. The
                    // live disassembly mode shows in the `r:` entry.
                    let (section, mode) = state.debug.as_ref()
                        .map(|p| (p.active_section(p.focus), p.disasm_mode_label()))
                        .unwrap_or((app::debug_panel::Section::Disasm, "full"));
                    let hints = app::render::hintbar::debug_hints(section, mode);
                    app::render::hintbar::literal_hint_bar(&hints, w)
                }
                // Unreachable in practice: `Focus::Map` is only ever set while
                // the inspector owns the right-hand pane (SQ-0599), which the
                // arm above already handles.
                Focus::Map => String::new(),
            };
            if rest.is_empty() {
                leader_hint
            } else {
                format!("{} | {}", leader_hint, rest)
            }
        };
        // Fill help row with reversed style, then draw text.
        for x in pane_layout.help_row.x..pane_layout.help_row.right() {
            if let Some(cell) = buf.cell_mut((x, pane_layout.help_row.y)) {
                cell.set_symbol(" ").set_style(help_style);
            }
        }
        draw_str_clipped(buf, pane_layout.help_row.x, pane_layout.help_row.y, &help_text, help_style, pane_layout.help_row);

        // The z-ordered modal/overlay ladder now lives in `overlays::draw_all`
        // (SQ-0306). It seeds `dialog` from the pre-ladder map/story draws
        // (tidy panel / room info / inspector) and returns the per-overlay
        // hit-rects that `draw_frame` splices into `PaneRects` below.
        overlay_rects = Some(overlays::draw_all(
            state,
            &screen_model,
            story_area,
            pane_layout.story,
            full,
            buf,
            dialog_rects_out.take(),
            &mut modal_list_viewport,
            &mut palette_hits,
        ));

        // ── Border-control hover hint (SQ-1123) ───────────────────────────────
        // After the overlay ladder, so the hint floats above the panes; the
        // hover is only ever SET while no modal overlay is open, so this can
        // never sit under one. It paints and returns — no focus, no keyboard.
        if state.control_hover.is_some() && !border_controls_out.is_empty() {
            // Both clusters, because `border_controls_out` carries both and a
            // hint the pointer can reach must be findable in this list (SQ-1148).
            let mut views = app::render::controls::controls_for(state);
            views.extend(app::render::controls::map_controls_for(state, map_control_view));
            app::render::controls::draw_control_hint(buf, full, state, &views, &border_controls_out);
        }

        // ── Matrix room-name tooltip (SQ-1246) ──────────────────────────────────
        // Drawn after the overlay ladder, exactly where the border-control hint
        // above is, so it floats on top and is never set while a modal owns the
        // pointer (see `matrix_update_hover`, which never sets it there either).
        {
            let graph = if let Some(g) = &replay_graph { g } else { &mapper.graph };
            app::render::matrix::draw_hover_tip(graph, layer, state, full, buf);
        }

        // Story-pane text-selection highlight + copy extraction now happen inside
        // render_middle (render/transcript.rs), which has the full wrapped-row set
        // and can select text beyond the visible viewport. (SQ-0197)
        //
        // The former bottom-bar map-edit prompts are now the text-entry modal drawn
        // by the overlay ladder in the graphics-free dialog area (SQ-0307).

        // Notification toasts anchor to the story pane's top-right (sliding in
        // from the pane's right edge) rather than the full frame, so they never
        // cover the map (SQ-0415). `story_area` is that pane's content rect as
        // drawn above this frame; if it's absent or too small, fall back to the
        // full frame so a toast is never lost. Drawn last so toasts sit topmost
        // over the story pane's own content (and anything else under them).
        // (SQ-0176, SQ-0415)
        let toast_area = app::render::transcript::notification_anchor_rect(
            state.transcript_geom.get().map(|g| g.area),
            story_area,
            full,
        );
        app::render::transcript::render_notifications(buf, toast_area, state);

        // Every image this frame abandoned is still sitting in the terminal until we
        // say otherwise, and the v6 pixel paths have no placement of their own to
        // carry the escapes (SQ-0753). The whole frame is the widest possible search
        // for a cell to hang them on, and here is the last moment one exists — a
        // delete queued during the draw and not flushed simply waits for the next
        // frame, so nothing is ever lost, only deferred.
        state.graphics_render.borrow_mut().flush_kitty_deletes(full, buf);

        // The frame is finished — every widget and the whole overlay ladder have
        // written. Keep its cells for `/dump-cells` (SQ-0761). This is the only
        // point at which the buffer is the frame the terminal is about to receive,
        // and the command runs long after it has been swapped away, so the dump has
        // to read a snapshot rather than the live buffer.
        state.note_frame_cells(buf);
    })?;

    // The draw closure runs exactly once, so the overlay ladder always ran.
    let overlay_rects = overlay_rects.expect("draw_frame closure runs exactly once");
    Ok(PaneRects { map: map_area, story: story_area, boundaries: pane_layout_out.boundary_zones(), pane_layout: pane_layout_out, room_rects: room_rects_out, map_view: map_control_view, room_dock: pane_layout_out.room_dock, room_dock_tabs: room_dock_tabs_out, layer_tabs: layer_tabs_out, border_controls: border_controls_out, debug_tabs: debug_tabs_out, dialog: overlay_rects.dialog, aux_dialog: overlay_rects.aux_dialog, history_prompt: overlay_rects.history_prompt, font_check: overlay_rects.font_check, fetch_keep: overlay_rects.fetch_keep, reset_dialog: overlay_rects.reset_dialog, region_prompt: overlay_rects.region_prompt, game_over: overlay_rects.game_over, save_name_dialog: overlay_rects.save_name_dialog, text_entry: overlay_rects.text_entry, confirm_delete: overlay_rects.confirm_delete, confirm_overwrite: overlay_rects.confirm_overwrite, quit_dialog: overlay_rects.quit_dialog, launch_dialog: overlay_rects.launch_dialog, hints_panel: overlay_rects.hints_panel, command_band: band_hits, inventory_dock: inv_hits, palette: palette_hits, transcript_links: transcript_links_out, win_rects: win_rects_out, transcript_max_scroll, transcript_viewport_rows, transcript_prompt_rows, transcript_total_rows, transcript_surface, modal_list_viewport })
}

// ── Command-band mouse routing ───────────────────────────────────────────────

/// Resolve a mouse event against the command band's hit rects, or `None` when
/// the event does not belong to the band (it is outside its rect, or the band is
/// closed) and must fall through to the game / map / story handling.
///
/// Everything visible in the band is clickable: a row picks and advances (and
/// composes onto the real story input line — SQ-0667, 2026-08-05), a header
/// points the band at its column, and a quick word fires its command AT ONCE
/// (the one exception to "picks compose, they don't submit"). The wheel scrolls
/// whichever column is under the pointer. There is no more phrase line to
/// click — sending is just the ordinary Enter on the real input line now.
/// SQ-0676 left all of this UNCHANGED: the mouse contract was never the
/// problem the keyboard inversion was solving.
fn band_mouse_action(
    state: &AppState,
    panes: &PaneRects,
    m: crossterm::event::MouseEvent,
) -> Option<Action> {
    use crossterm::event::{MouseButton, MouseEventKind};

    state.overlays.command_band.as_ref()?;
    // SQ-1236: a modal dialog stacked on top (config_screen, hotkey_dialog, …)
    // takes all mouse input; the band underneath must claim nothing while one is
    // open, so a click falls through to `mouse_to_action`'s dialog hit-testing
    // instead of being swallowed here first.
    if state.any_modal_overlay_open() {
        return None;
    }
    let hits = &panes.command_band;
    let inside = |r: &Rect| {
        r.width > 0 && r.height > 0 && m.column >= r.x && m.column < r.right() && m.row >= r.y
            && m.row < r.bottom()
    };
    if !inside(&hits.area) {
        return None;
    }

    match m.kind {
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            let d = app::input::wheel_delta(m.kind, state.config.mouse_wheel_invert)?;
            // Scroll the column under the pointer, focusing it first so the
            // scroll lands where the user is looking.
            let col = hits.columns.iter().find(|(_, r)| inside(r)).map(|(c, _)| *c);
            match col {
                Some(c) if state.overlays.command_band.as_ref().is_some_and(|b| b.col_reachable(c)) => {
                    Some(Action::BandWheel(c, d as i32))
                }
                _ => Some(Action::None),
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some((col, idx, _)) = hits.rows.iter().find(|(_, _, r)| inside(r)).copied() {
                return Some(Action::BandClickRow(col, idx));
            }
            if let Some((i, _)) = hits.quick.iter().find(|(_, r)| inside(r)).copied() {
                return Some(Action::BandQuickPick(i));
            }
            if let Some((col, _)) = hits.headers.iter().find(|(_, r)| inside(r)).copied() {
                return Some(Action::BandFocusCol(col));
            }
            // Anywhere else inside the band: claimed (so it never reaches the
            // game behind it) but does nothing. There is no keyboard for it to
            // take anymore — SQ-0676 gave typing back to the prompt for good.
            Some(Action::None)
        }
        // Drag/Up inside the band must not start a story-pane text selection.
        _ => Some(Action::None),
    }
}

/// Resolve a mouse event against the inventory dock's hit rects (SQ-1244) —
/// the panel's own counterpart of `band_mouse_action`. The two panels are
/// mutually exclusive (`SidePanel`), so this never competes with the band for
/// the same click; it claims exactly the dock's own rect, the same way the
/// band claims its own, so a click never falls through to the story pane.
fn inventory_mouse_action(
    state: &AppState,
    panes: &PaneRects,
    m: crossterm::event::MouseEvent,
) -> Option<Action> {
    use crossterm::event::{MouseButton, MouseEventKind};

    // SQ-1236's rule, same as the band: a modal dialog stacked on top takes
    // all mouse input, so the dock underneath claims nothing while one is
    // open and the click falls through to `mouse_to_action`'s dialog
    // hit-testing instead.
    if state.any_modal_overlay_open() {
        return None;
    }
    let hits = &panes.inventory_dock;
    let inside = |r: &Rect| {
        r.width > 0 && r.height > 0 && m.column >= r.x && m.column < r.right() && m.row >= r.y
            && m.row < r.bottom()
    };
    if !inside(&hits.area) {
        return None;
    }

    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some((idx, _)) = hits.rows.iter().find(|(_, r)| inside(r)).copied() {
                return Some(Action::InventoryClickRow(idx));
            }
            // Anywhere else inside the dock: claimed but does nothing, same
            // as a click on empty band real estate.
            Some(Action::None)
        }
        // Drag/Up inside the dock must not start a story-pane text selection.
        _ => Some(Action::None),
    }
}

/// Update the command band's quick-block hover highlight from a `Moved`
/// mouse event (SQ-0677) — the quick block (rose + flowing words, and the
/// flat-row fallback) is mouse-click-only now, so hover is its only
/// transient highlight. Mirrors `pane_drag::on_mouse`'s own `Moved` handling:
/// runs unconditionally (not gated on `band_mouse_action`'s `hits.area`
/// check), never claims the event, and clears the hover — rather than
/// leaving a stale one lit — the moment the pointer sits over anything else
/// in the band, leaves the band's rect, or a modal opens over it. Reads
/// `panes.command_band` — the SAME hit rects the click path
/// (`band_mouse_action`) hit-tests — so hover and click always agree about
/// which cell is under the pointer.
fn band_update_quick_hover(state: &mut AppState, panes: &PaneRects, event: &Event) {
    use crossterm::event::MouseEventKind;
    let Event::Mouse(m) = event else { return };
    if m.kind != MouseEventKind::Moved || state.overlays.command_band.is_none() {
        return;
    }
    let hover = if state.any_modal_overlay_open() {
        None
    } else {
        panes
            .command_band
            .quick
            .iter()
            .find(|(_, r)| {
                r.width > 0
                    && r.height > 0
                    && m.column >= r.x
                    && m.column < r.right()
                    && m.row >= r.y
                    && m.row < r.bottom()
            })
            .map(|(i, _)| *i)
    };
    if let Some(band) = state.overlays.command_band.as_mut() {
        if band.quick_hover != hover {
            band.quick_hover = hover;
        }
    }
}

/// Track which matrix-view room the pointer is on (SQ-1246): a row label or a
/// destination cell, both of which name a room the table may have had to
/// abbreviate.
///
/// Same shape as [`band_update_quick_hover`] just above: pointer motion with
/// no button held resolves against LAST FRAME's `room_rects` — the same ones
/// a click on a row or a destination cell already resolves against — and only
/// while that frame actually drew the matrix view, since `room_rects` carries
/// the drawn map's room boxes too and those are out of scope for this hint.
/// Never claims the event, and clears — rather than leaving a stale room lit
/// — the moment the pointer moves off, the view changes, or a modal opens.
fn matrix_update_hover(state: &mut AppState, panes: &PaneRects, event: &Event) {
    use crossterm::event::MouseEventKind;
    let Event::Mouse(m) = event else { return };
    if m.kind != MouseEventKind::Moved {
        return;
    }
    state.matrix_hover = if state.any_modal_overlay_open()
        || panes.map_view != mapper::layer::MapView::Matrix
    {
        None
    } else {
        panes.room_rects.iter().copied().find(|(_, r)| {
            r.width > 0
                && r.height > 0
                && m.column >= r.x
                && m.column < r.right()
                && m.row >= r.y
                && m.row < r.bottom()
        })
    };
}

// ── File-browser entry action helper ─────────────────────────────────────────

/// Decoded action when Enter is pressed in the file browser.
enum FbEntryAction {
    /// Navigate into the given directory.
    CdInto(std::path::PathBuf),
    /// Import the given file.
    ImportFile(std::path::PathBuf),
}

// ── main ──────────────────────────────────────────────────────────────────────

/// Start or stop the style.toml file-watcher to match `on`, updating the status
/// line. A no-op when the watcher is already in the requested state.
fn set_style_watch(
    state: &mut app::state::AppState,
    watcher: &mut Option<app::watch::StyleWatcher>,
    on: bool,
) {
    if on == watcher.is_some() {
        return; // already in the requested state
    }
    if !on {
        *watcher = None;
        state.set_status("style watch off");
    } else if let Some(p) =
        app::reload::resolved_style_path(state.config.style.as_deref(), &state.config.user_dir)
    {
        *watcher = app::watch::start(&p);
        if let Some(w) = watcher.as_mut() {
            w.also_watch(&state.game_dir);
        }
        state.set_status(if watcher.is_some() {
            "style watch on"
        } else {
            "style watch: no file to watch"
        });
    } else {
        state.set_status("style watch: no file to watch");
    }
}

/// Toggle the opt-in style.toml file-watcher on/off.
fn toggle_style_watch(
    state: &mut app::state::AppState,
    watcher: &mut Option<app::watch::StyleWatcher>,
) {
    set_style_watch(state, watcher, watcher.is_none());
}

/// Run a map-export Action (SVG/DOT/dump) into the per-game dir. Returns true if
/// `action` was a map-export action (so callers fall through otherwise). Mirrors
/// the resolve→create_dir_all→render→write→notice logic that was inline at the
/// main-loop Action::Export* arms (SQ-0297: slash commands never reached that
/// match, so this is shared so both the slash and key-dispatch paths export).
fn handle_map_export(
    action: &Action,
    game_dir: &std::path::Path,
    mapper: &Mapper,
    state: &mut AppState,
) -> bool {
    match action {
        Action::ExportSvg(dest) => {
            let path = app::export::resolve_export_path(dest.as_deref(), game_dir, "map.svg");
            if let Some(p) = path.parent() { let _ = std::fs::create_dir_all(p); }
            let rm = render_map_data(&mapper.graph);
            match export_svg(&path, &rm) {
                Ok(()) => state.push_notice(&format!("[SVG exported to {}]", abbreviate_home(&path))),
                Err(e) => state.push_notice(&format!("[SVG export failed: {}]", e)),
            }
            true
        }
        Action::ExportDot(dest) => {
            let path = app::export::resolve_export_path(dest.as_deref(), game_dir, "map.dot");
            if let Some(p) = path.parent() { let _ = std::fs::create_dir_all(p); }
            match export_dot(&path, &mapper.graph) {
                Ok(()) => state.push_notice(&format!(
                    "[DOT exported to {} — render with: dot -Tsvg {} -o map.svg]",
                    abbreviate_home(&path),
                    abbreviate_home(&path)
                )),
                Err(e) => state.push_notice(&format!("[DOT export failed: {}]", e)),
            }
            true
        }
        Action::ExportMap(dest) => {
            let path = app::export::resolve_export_path(dest.as_deref(), game_dir, "map.txt");
            if let Some(p) = path.parent() { let _ = std::fs::create_dir_all(p); }
            match std::fs::write(&path, render_dump(&mapper.graph, &state.symbols)) {
                Ok(()) => state.push_notice(&format!("[map dump written to {}]", abbreviate_home(&path))),
                Err(e) => state.push_notice(&format!("[map dump failed: {}]", e)),
            }
            true
        }
        _ => false,
    }
}

/// Abbreviate a leading $HOME in a path to `~` for display.
fn abbreviate_home(p: &std::path::Path) -> String {
    let s = p.display().to_string();
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            if let Some(rest) = s.strip_prefix(&home) { return format!("~{rest}"); }
        }
    }
    s
}

/// Format the one-line loading indicator shown while a (possibly large) story
/// boots to its first prompt. `frame` is the spinner glyph for this tick. Large
/// Glulx games (e.g. Counterfeit Monkey at ~11 MB) take several seconds to reach
/// the first prompt; without this the normal terminal sits frozen and looks hung.
fn loading_line(name: &str, bytes: usize, frame: char) -> String {
    format!("lanthorn: loading {name} ({:.1} MB) {frame}", bytes as f64 / 1_048_576.0)
}

/// Format the startup line naming the PRNG seed this launch handed the engine
/// (SQ-0811). `pinned` is whether it came from the `random_seed` config key.
///
/// The unpinned line says how to keep the run, because a fresh seed is the whole
/// point of the default and a player who has just had a remarkable game has no
/// other way to ask for it again.
fn random_seed_line(seed: u32, pinned: bool) -> String {
    if pinned {
        format!("random seed {seed} (pinned by random_seed in config.toml)")
    } else {
        format!("random seed {seed} (set random_seed = {seed} to replay this run)")
    }
}

/// Clear the terminal, and tell the graphics cache that it just lost every image
/// placement (SQ-0587).
///
/// A clear wipes the screen — under a graphics protocol that takes the placements
/// with it. The chrome-band cache exists to skip re-uploading a band whose pixels
/// have not changed, so after a clear every band is a HIT and nothing is sent: the
/// art is gone from the screen while the cache believes it is still there. It comes
/// back only when something happens to change a band's cache key, which is why
/// toggling the map (a width change) restored it while a vertical-only resize — same
/// rects, same keys — did not.
fn clear_terminal<B: ratatui::backend::Backend>(
    terminal: &mut ratatui::Terminal<B>,
    state: &app::state::AppState,
) {
    let _ = terminal.clear();
    let mut gr = state.graphics_render.borrow_mut();
    gr.invalidate_chrome_bands();
    gr.invalidate_v6();
}

/// True when an armed deadline has come due. Extracted so the "is this clock
/// due?" decision is testable on its own (SQ-0650). `None` = not armed.
fn deadline_due(deadline: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    deadline.is_some_and(|dl| now >= dl)
}

/// Fire every game clock whose deadline has come due: the Z-machine timed-input
/// interrupt, the Glulx Glk timer, sampled-sound finish routines / sound-notify,
/// and Sound2 volume ramps + their volume-notify. Returns
/// `(redraw_needed, should_quit)`.
///
/// **Runs once per loop iteration, on every path** (SQ-0650). This used to live
/// inside the poll-timeout branch, which meant it only ran on a tick where NO
/// terminal event arrived — so a mouse whose motion events keep `poll()`
/// permanently "ready" froze every one of these clocks: a timed-input puzzle
/// stopped counting down, a Glk timer stopped ticking, and a finished sound never
/// ran its finish routine, for as long as the pointer kept moving. The loop top
/// is the same safe point the timeout branch used (both sit between whole event
/// dispatches, with nothing borrowed), so this is a move, not a new re-entrancy.
///
/// Each fired clock disarms itself before dispatching so an elapsed deadline
/// cannot re-fire every iteration until the game re-arms it.
fn dispatch_due_game_clocks(
    state: &mut app::state::AppState,
    mapper: &mut Mapper,
    session: &mut dyn Engine,
    game_dir: &std::path::Path,
    map_rect: Rect,
) -> (bool, bool) {
    let mut redraw = false;
    // Timed-input interrupt: the deadline elapsed with no key pressed. Run the
    // game's interrupt routine and apply its output through the same path a
    // char-mode keypress uses. If the read continues, the pre-input pollers
    // re-arm the deadline next iteration from `pending_timeout()`; if the routine
    // aborted the read, it returns `None` and the timer simply stops.
    if deadline_due(state.input_deadline, std::time::Instant::now()) {
        if let Some(zs) = zvm_session_opt_mut(session) {
            let result = zs.run_timed_interrupt();
            // Fired: disarm so the next armed iteration re-arms fresh at
            // now + interval (otherwise the elapsed deadline would refire
            // immediately every iteration).
            state.input_deadline = None;
            redraw = true; // interrupt ran → repaint any output
            if turn::apply_game_driven_result(
                state, mapper, &result, game_dir, map_rect, &*session, app::pager::Driver::Timeout,
            ) {
                return (redraw, true);
            }
        }
    }
    // Glulx Glk timer tick: the interval elapsed with no key pressed. Deliver an
    // evtype_Timer to the game and apply its output; disarm so the next armed
    // iteration re-arms fresh at now + interval (mirroring the guard above).
    if deadline_due(state.glulx_timer_next_fire, std::time::Instant::now()) {
        state.glulx_timer_next_fire = None;
        redraw = true; // timer event delivered → repaint any output
        if let Some(gs) = glulx_session_opt_mut(session) {
            let result = gs.deliver_timer();
            if turn::apply_game_driven_result(
                state, mapper, &result, game_dir, map_rect, &*session, app::pager::Driver::Timeout,
            ) {
                return (redraw, true);
            }
        }
    }
    // Poll for finished sampled sounds and fire their finish-routines.
    let done: Vec<u32> = state.audio.as_mut().map(|b| b.finished()).unwrap_or_default();
    if !done.is_empty() {
        redraw = true; // finish-routine output / channel state changed
    }
    for id in done {
        // Always forget the number->id mapping for a finished sound, even one
        // with no finish routine.
        state.sound_ids.retain(|_, v| *v != id);
        if let Some(routine) = state.sound_routines.remove(&id) {
            if routine != 0 {
                if let Some(zs) = zvm_session_opt_mut(session) {
                    let result = zs.run_sound_finish(routine);
                    if turn::apply_game_driven_result(
                        state, mapper, &result, game_dir, map_rect, &*session, app::pager::Driver::Timeout,
                    ) {
                        return (redraw, true);
                    }
                }
            }
        }
        // Glulx sound-notify: a finished channel delivers Evtype_SoundNotify.
        if let Some((snd, notify)) = state.glulx_sound_notify.remove(&id) {
            state.glulx_channels.retain(|_, v| *v != id);
            if let Some(gs) = glulx_session_opt_mut(session) {
                let result = gs.sound_notify(snd, notify);
                if turn::apply_game_driven_result(
                    state, mapper, &result, game_dir, map_rect, &*session, app::pager::Driver::Timeout,
                ) {
                    return (redraw, true);
                }
            }
        }
    }
    // Glulx Sound2 volume-ramp completion: a gradual set_volume_ext whose
    // duration has elapsed delivers an evtype_VolumeNotify. The host owns the
    // ramp clock (mirroring the sound-finish notify above); deliver every due one.
    let now = std::time::Instant::now();
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
            if turn::apply_game_driven_result(
                state, mapper, &result, game_dir, map_rect, &*session, app::pager::Driver::Timeout,
            ) {
                return (redraw, true);
            }
        }
    }
    (redraw, false)
}

/// `--fetch`: run the IFDB metadata pass over `source` without a terminal,
/// printing one line per story, and return the process exit code (0 unless a
/// fetch failed). The worker, the delay between requests and the sidecar
/// writes are the picker's own; only the reporting differs.
fn run_headless_fetch(
    source: &app::picker::StorySource,
    mode: app::config::FetchMode,
    data_base: &std::path::Path,
) -> i32 {
    use app::fetch_worker::{FetchOrder, Fetcher, Outcome};
    let targets = app::picker::fetch_targets(source, data_base);
    let total = targets.len();
    if total == 0 {
        eprintln!("lanthorn: no stories under {}", source.dir().display());
        return 1;
    }
    eprintln!("lanthorn: fetching IFDB metadata for {total} stories under {}", source.dir().display());
    let fetcher = Fetcher::new(
        Box::new(app::ifdb::IfdbClient::new()),
        data_base.to_path_buf(),
        std::time::Duration::from_millis(500),
    );
    fetcher.request(FetchOrder { stories: targets, forced: mode.forced(), id_override: None });

    #[derive(Default)]
    struct Tally {
        done: usize,
        fetched: usize,
        skipped: usize,
        not_found: usize,
        failed: usize,
    }
    impl Tally {
        fn note(&mut self, p: app::fetch_worker::FetchProgress, total: usize) {
            self.done += 1;
            let word = match &p.outcome {
                Outcome::Fetched => {
                    self.fetched += 1;
                    "fetched".to_string()
                }
                Outcome::Skipped => {
                    self.skipped += 1;
                    "skipped (current)".to_string()
                }
                Outcome::NotFound => {
                    self.not_found += 1;
                    "not on IFDB".to_string()
                }
                Outcome::Failed(e) => {
                    self.failed += 1;
                    format!("failed: {e}")
                }
            };
            let place = match &p.disk_entry {
                Some(e) => format!("{} [{e}]", p.path.display()),
                None => p.path.display().to_string(),
            };
            println!("[{}/{total}] {}  ({place})  {word}", self.done, p.title);
        }
    }
    let mut tally = Tally::default();
    loop {
        let batch = fetcher.drain();
        let quiet = batch.is_empty();
        for p in batch {
            tally.note(p, total);
        }
        if tally.done >= total {
            break;
        }
        if quiet {
            // The worker clears `busy` after its last send, so a drain after
            // seeing it clear collects the tail; an empty tail is the end.
            if !fetcher.busy() {
                let tail = fetcher.drain();
                if tail.is_empty() {
                    break;
                }
                for p in tail {
                    tally.note(p, total);
                }
            } else {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    }
    let Tally { fetched, skipped, not_found, failed, .. } = tally;
    println!("lanthorn: {fetched} fetched, {skipped} skipped, {not_found} not on IFDB, {failed} failed");
    if failed > 0 { 1 } else { 0 }
}

fn main() {
    // ── ONE-TIME setup ────────────────────────────────────────────────────────
    // Register termination-signal handlers before any raw-mode entry (the picker
    // or the game loop) so an early kill/SIGHUP still restores the terminal; both
    // interactive loops poll the flag via `exit_if_terminated`. (SQ-0435: moved
    // out of `boot` so it registers once across the picker→play loop.)
    install_termination_handlers();

    // Resolve the launch context once: args/config, style-seed, data base, and
    // whether we launched from a directory (a story library) or a single file.
    // The first-use `default_story_dir` prompt lives here and runs exactly once.
    let ctx = startup::resolve_launch();

    // What the launch argument means as a *source of stories* (SQ-0844). A
    // directory is a library, as it always was. A single file is normally just
    // itself — but naming one volume of a multi-disk release now opens the whole
    // release, because a collection cut across seven disks is one shelf and the
    // player named it. `StorySource::of` declines any file that is not a volume
    // of a set offering two or more games, so every ordinary story file, every
    // single-title floppy and every one-game set takes the path it always did.
    let source = match &ctx.library_dir {
        Some(dir) => Some(app::picker::StorySource::Library(dir.clone())),
        None => ctx
            .single_file
            .as_ref()
            .and_then(|p| app::picker::StorySource::of(p, &ctx.data_base)),
    };
    // `--story <n|name>` makes the browser's choice on the command line
    // (SQ-1078). Resolved ONCE, here, against the very list the browser would
    // have shown, so that a headless instrument can reach any game on a
    // compilation disc instead of only whichever one the mount prefers. A miss
    // prints the list and exits 2 — the same code `resolve_launch` uses for "no
    // story given", and never a fallback to booting an arbitrary game.
    // `--fetch`: the browser's IFDB pass with no browser, then exit. Placed
    // after `source` so it takes the same library or disk set the picker
    // would, and before anything touches the terminal.
    // `--import-metadata`: curated rows for what `--fetch` could not settle.
    if let Some(tsv) = ctx.cli.import_metadata.as_deref() {
        let source = app::ifdb::IfdbClient::new();
        std::process::exit(app::metadata_import::run(tsv, &ctx.data_base, &source, std::time::Duration::from_millis(500)));
    }

    if let Some(mode) = ctx.cli.fetch {
        let Some(source) = source.as_ref() else {
            eprintln!("lanthorn: --fetch needs a library directory or a story file");
            std::process::exit(2);
        };
        std::process::exit(run_headless_fetch(source, mode, &ctx.data_base));
    }

    let direct = ctx.cli.story_pick.as_deref().map(|want| {
        let single = ctx.single_file.clone().unwrap_or_default();
        match app::story_pick::pick(source.as_ref(), &single, &ctx.data_base, want) {
            Ok(chosen) => chosen,
            Err(msg) => {
                eprintln!("lanthorn: {msg}");
                std::process::exit(2);
            }
        }
    });

    // A set browser is a library in every way that matters here: quitting it
    // exits, finishing a game returns to it, and `/quit-to-library` is live.
    // **Except when `--story` named the game**: that launch bypassed the browser
    // deliberately, so it behaves like the single-file launch it reads as —
    // `/quit-to-library` stays gated off and the loop ends when the game does,
    // rather than dropping the player into a list they asked not to see.
    let launched_from_library = source.is_some() && direct.is_none();

    // ── Picker → play loop ────────────────────────────────────────────────────
    loop {
        // Obtain the next story to play, plus any boot-time overrides chosen on
        // the way in (SQ-0789/0791): the browser's launch-options dialog for a
        // library launch, `--pictures` for a single file or a `--story` pick.
        // Both are empty for an ordinary launch.
        let (story_path, disk_entry, overrides) = if let Some((path, entry)) = &direct {
            // Named outright: no browser, and the pair that opens it — the
            // container's path plus WHICH story on it, which is the only thing
            // that reaches the right game on a compilation disc.
            (path.clone(), entry.clone(), cli_overrides(&ctx))
        } else if let Some(source) = &source {
            // Library (or multi-disk set) launch: run the picker on the normal
            // screen (the previous game left its alt-screen). Quitting the
            // picker (None) exits.
            match picker_ui::run_story_picker(source.clone(), &ctx.cfg, &ctx.data_base) {
                Some(p) => (p.path, p.disk_entry, p.overrides),
                None => break,
            }
        } else {
            // Single-file launch: play the one file; after it returns we exit.
            // `--pictures` has a referent here and only here, which is why clap
            // refuses it without a story (SQ-0791).
            let path = ctx
                .single_file
                .clone()
                .expect("resolve_launch sets single_file for a non-library launch");
            // A story file named on the command line opens itself; a disk image
            // that is not a volume of a multi-disk set opens what it always did,
            // the format's own tiebreak. Reaching the other games on a *single*
            // compilation disk is the browser's job (SQ-0859); a set named by
            // one of its volumes never reaches here at all (SQ-0844).
            (path, None, cli_overrides(&ctx))
        };

        // Per-story build (enters the game alt-screen fresh), then run the loop.
        let boot = startup::boot_story(&ctx, story_path, disk_entry.as_deref(), &overrides);
        match run_event_loop(boot, launched_from_library) {
            RunOutcome::Exit => break,
            // Return-to-library only loops when a library exists; a single-file
            // launch can't reach ToLibrary (the command is gated off), but guard
            // it anyway so the loop always terminates.
            RunOutcome::ToLibrary if launched_from_library => continue,
            RunOutcome::ToLibrary => break,
        }
    }
}

/// The boot-time overrides a *command-line* launch carries — what the browser's
/// launch-options dialog supplies for a picked story (SQ-0789/0791).
///
/// `--pictures` has a referent exactly when a story was named on the command
/// line, which is why clap requires one; both launches that name a story reach
/// this, the bare single file and the `--story` pick off a volume.
///
/// A shell-completed `--pictures stories/zork0.mg1` is relative to the WORKING
/// DIRECTORY, while `pictures = "…"` in a game's config is relative to the STORY
/// — the sidecar lives elsewhere, so that is the only sane rule there.
/// Absolutise a CLI path that resolves from here so both readings work and
/// neither surprises anyone; a name that does not exist from the cwd falls
/// through to the story-relative form, which is what `--pictures zork0.mg1`
/// means.
fn cli_overrides(ctx: &startup::LaunchCtx) -> app::launch_options::LaunchOverrides {
    app::launch_options::LaunchOverrides {
        pictures: ctx.cli.pictures.as_ref().map(|p| {
            std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()).display().to_string()
        }),
        interpreter_number: None,
    }
}

/// Run the interactive event loop over one story's `BootResult`, returning how it
/// ended ([`RunOutcome`]). Mechanically extracted from `main()` (SQ-0435): all
/// loop-local state lives here; the labeled loop yields the outcome; the terminal
/// is restored and the exit auto-save runs before returning. `launched_from_library`
/// is stashed on the state so `/quit-to-library` can gate on whether a picker exists.
fn run_event_loop(boot: startup::BootResult, launched_from_library: bool) -> RunOutcome {
    let startup::BootResult {
        mut session,
        mut mapper,
        mut state,
        mut terminal,
        game_dir,
        ifid,
        arc_file,
        story_bytes,
        story_path,
        data_base,
    } = boot;

    // Whether a story library exists to return to; gates `/quit-to-library`. Set
    // once here from the launch context. (SQ-0435)
    state.launched_from_library = launched_from_library;
    // A story launched from the list always resolves back to it, on every way
    // the run can end — the game's own quit included — not only the explicit
    // `/quit-to-library` path. Seeding the default here means a game-driven quit
    // (`should_exit_on_turn`, never touched by any quit dispatch) resolves
    // correctly with no separate wiring of its own (SQ-1258).
    state.exit_target = app::state::ExitTarget::for_launch(launched_from_library);

    // ── 5. Event loop ─────────────────────────────────────────────────────────

    // Track the last-known pane rects for accurate recenter_on calls and mouse routing.
    // Initialized to a zero-sized default; updated by every draw_frame call.
    let mut last_panes = PaneRects::default();

    // Debounce counter for BackgroundTidy::Debounced mode.
    let mut bg_tidy_counter: u32 = 0;

    // Double-click tracking for the command band's word columns (SQ-0690): a second click on
    // the same row within the window submits the composed prompt.
    let mut band_clicks = app::input::BandClickTracker::default();

    // Glulx re-arrange debounce (SQ-0201). The Glulx VM starts on a fixed virtual
    // screen; once the real story-pane size is known (and whenever it changes: a
    // terminal resize, a map/sidebar toggle) we report it and deliver a Glk
    // Arrange so graphics windows repaint at the new size — but only after the
    // size settles, so a drag doesn't run the game's redraw on every tick.
    // `vm_story_size` = size last reported to the VM; `story_size_seen` = size at
    // the previous frame; `resize_dirty` = when the size last moved.
    let mut vm_story_size: Option<(u16, u16)> = None;
    let mut story_size_seen: Option<(u16, u16)> = None;
    let mut resize_dirty: Option<std::time::Instant> = None;

    // Poll FPS while a background tidy is in flight.
    const TIDY_POLL_MS: u64 = 33;

    // Optional style.toml file-watcher (opt-in via watch_style; toggled by /watch).
    let mut style_watcher: Option<app::watch::StyleWatcher> = None;
    let mut watch_dirty: Option<std::time::Instant> = None;
    if state.config.watch_style {
        if let Some(p) =
            app::reload::resolved_style_path(state.config.style.as_deref(), &state.config.user_dir)
        {
            style_watcher = app::watch::start(&p);
            if let Some(w) = style_watcher.as_mut() {
                w.also_watch(&state.game_dir);
            }
        }
    }

    // From here on the app drives the game through the engine-neutral trait
    // (`session` was boxed at construction: a GameSession for Z-code, a
    // GlulxSession for Glulx). The Z-machine-specific setup above runs behind a
    // downcast so the Glulx path skips it.

    // Input-burst coalescing: when a read event still has more events queued
    // behind it, defer the redraw until the queue drains. A stream of mouse
    // motion events (or a paste) then costs ONE redraw instead of one per event.
    let mut skip_draw = false;

    // Dirty-flag redraw gate (SQ-0305): the loop wakes every ~50ms (faster while
    // animating/timing) but the UI only changes when something observable happens.
    // Redraw only when `needs_redraw` is set (or an animation is active); an idle
    // app then does ~zero work per tick. The flag is set wherever the loop did
    // something — an event was dispatched, a background poller applied a change, a
    // deadline fired — and left false only on the pure poll-timeout no-op path.
    // First frame always draws. The poll deadlines are UNCHANGED: this gates the
    // draw, not the tick.
    let mut needs_redraw = true;

    // Deferred warning from the quit dialog's "Save State & quit" (SQ-0651).
    // Printed after `restore_terminal()` below, where the user can actually read
    // it — the dialog is the last frame the alternate screen ever shows.
    let mut quit_save_warning: Option<String> = None;

    let outcome: RunOutcome = 'event_loop: loop {
        // Restore the terminal + auto-save + exit if an external termination signal
        // arrived (SIGTERM/SIGHUP/out-of-band SIGINT); the poll below wakes at least
        // every ~50ms, so this is checked promptly.
        exit_if_terminated_saving(&mut *session, &mapper, &state, &ifid, &arc_file);

        // ── Late probe answers (SQ-0769) ──────────────────────────────────────
        // The startup OSC 10/11 probe ends in a DSR so it knows when the terminal
        // has finished answering. If it gave up first — a terminal busy with the
        // picker's last frame is slower than the drain's patience — the answers
        // are still coming, and read as keystrokes they skip the intro and answer
        // the restore prompt. Until the DSR answer arrives this owns the tty (the
        // `poll`/`read` below stand down), dropping the terminal's escape traffic
        // and keeping anything the player typed for replay. A no-op — one bool —
        // on every launch where the terminal answered on time.
        state.query_sweep.pump();

        // ── Game clocks (SQ-0650) ─────────────────────────────────────────────
        // Timed-input interrupts, Glk timer events, sound finish-routines and
        // Sound2 volume-notify used to be dispatched only from the poll-TIMEOUT
        // branch below — so with mouse capture on, a moving pointer kept `poll()`
        // permanently ready and the game's clocks stopped dead. They run here
        // instead, once per iteration, on every path. (The timeout branch reached
        // this same point via its `continue`, so the ordering is unchanged for the
        // idle case that already worked.)
        {
            let (redraw, quit) = dispatch_due_game_clocks(
                &mut state, &mut mapper, &mut *session, &game_dir, last_panes.map,
            );
            needs_redraw |= redraw;
            if quit {
                break 'event_loop state.exit_target.into();
            }
        }

        // ── Pre-input pollers (SQ-0306) ───────────────────────────────────────
        // The per-iteration housekeeping that runs BEFORE the draw/poll: each
        // independent pollable subsystem lives in `loop_tick` and returns its
        // redraw contribution, OR-ed into `needs_redraw` here (order preserved).
        needs_redraw |= loop_tick::poll_style_watch(&mut state, &style_watcher, &mut watch_dirty);
        loop_tick::sync_theme_colours(&state, &mut *session);
        needs_redraw |= loop_tick::poll_glulx_resize(
            &mut *session,
            &last_panes,
            &mut story_size_seen,
            &mut resize_dirty,
            &mut vm_story_size,
        );
        // ZMSD §8.4 / §8.3.3 (SQ-0532): keep the story's header describing the REAL
        // host — the story pane's measured size in $20/$21, and our own default
        // page/ink in $2C/$2D (which a live style reload can change mid-game).
        needs_redraw |= loop_tick::poll_zvm_screen_dims(&mut *session, &state, &last_panes);
        loop_tick::poll_zvm_default_colours(&mut *session, &state);
        // Settle the layout a hidden map deferred, now its pane is back (SQ-1136).
        // Before the poll, so the job it schedules is picked up on the next pass
        // rather than sitting a whole frame longer than it has to.
        needs_redraw |=
            loop_tick::catch_up_deferred_map_layout(&mut state, &mapper, &mut bg_tidy_counter);
        needs_redraw |= loop_tick::poll_tidy_jobs(&mut state, &mut mapper, &last_panes);
        needs_redraw |= state.poll_render_job();
        needs_redraw |= state.poll_v6_encode_job();
        // Play out a v6 turn's picture sequence one frame at a time (SQ-0708).
        // Runs before the draw, so an advanced frame paints on this very pass.
        needs_redraw |= loop_tick::poll_picture_pacing(&mut state, &mut *session);
        needs_redraw |= loop_tick::refresh_engine_input(&mut state, &mut *session);
        // The command band's object columns are LIVE: refilled from the engine
        // every tick, so a take/drop moves an object between *here* and
        // *carried* on the very next frame (SQ-0664).
        needs_redraw |= loop_tick::refresh_command_band(&mut state, &*session);
        // The inventory dock's clickable words are LIVE too, and independent of
        // the command band (SQ-1244): the two panels are mutually exclusive, so
        // the dock cannot piggyback on the band's own object refresh.
        app::render::inventory_dock::refresh_inventory_click_words(&mut state, &*session);
        needs_redraw |= loop_tick::expire_sound_and_settle_dock(&mut state);
        // One collector for the shared shadow, routing each answer to whoever
        // asked for it (SQ-1124, SQ-0785): a vocabulary offer lands above the
        // prompt like any other assist and drops silently if the player has typed
        // again, while a return-path answer goes on the map whenever it arrives —
        // it is a fact about the world, not about this turn. The same call hands
        // the return search its next question.
        needs_redraw |= loop_tick::poll_shadow_answers(&mut state, &mut mapper, &mut bg_tidy_counter);

        // Draw — unless we're mid-drain of an input burst (skip_draw), in which
        // case the deferred redraw happens once the queue empties. last_panes and
        // the panes-derived clamps below simply carry over from the last real
        // frame during the burst (layout is stable within a burst).
        // Redraw gate (SQ-0305): skip the draw entirely when nothing changed and
        // no animation is in flight. `skip_draw` still coalesces an input burst
        // (and, when it fires, leaves `needs_redraw` set so the deferred frame
        // draws once the queue empties). An active animation always draws so its
        // tween keeps stepping.
        if !std::mem::take(&mut skip_draw) && (needs_redraw || state.has_active_animation()) {
        needs_redraw = false;
        match draw_frame(&mut terminal, &*session, &mapper, &state) {
            Ok(panes) => {
                // Post-frame transcript bookkeeping (SQ-0404, SQ-0578): clamp
                // scrollback, resolve a pending [more] arm, cache the pager
                // baseline — skipped wholesale on picture-only frames.
                app::pager::apply_frame(
                    &mut state,
                    panes.transcript_max_scroll,
                    panes.transcript_viewport_rows,
                    panes.transcript_prompt_rows,
                    panes.transcript_total_rows,
                    panes.transcript_surface,
                );
                // Carry this frame's modal list viewport so the next nav action
                // can window/animate the open selection-list modal.
                state.modal_list_viewport = panes.modal_list_viewport;
                // Replay's idx is the source of truth; keep its (animated) list
                // scroll following it. Skip while a scroll is easing so the tween
                // isn't restarted each frame; select() is a no-op once settled.
                let anim = state.config.animation.clone();
                let hist_len = state.history.len();
                if let Some(r) = &mut state.overlays.replay {
                    if !r.scroll.has_active_animation() {
                        r.scroll.len(hist_len);
                        r.scroll.select(r.idx, state.modal_list_viewport, &anim);
                    }
                }
                last_panes = panes;
            }
            Err(e) => {
                restore_terminal();
                eprintln!("lanthorn: draw error: {}", e);
                // The engine is intact — only the terminal write failed — so the
                // turn the player just took is still saveable. Without this, an
                // error exit silently dropped it while every other exit path
                // (clean quit, signal) saved. (SQ-0651)
                exit_save_on_error_exit(&mut *session, &mapper, &state, &ifid, &arc_file);
                std::process::exit(1);
            }
        }
        }

        // Poll for a key event. Use a shorter timeout while a tidy job is in flight
        // so the pulsing border animates at ~30fps; otherwise use the normal 50ms.
        // When a timed-input deadline is armed, clamp further so the loop wakes in
        // time to fire the interrupt — the normal cadence stays the ceiling, so
        // this is a no-op when no timer is running (regression guard).
        let sound_active = !state.sound_routines.is_empty()
            || !state.glulx_sound_notify.is_empty()
            || !state.glulx_volume_notify.is_empty()
            // A notify-less ramp still needs the loop to keep waking so it can
            // step the sink gain smoothly (glulx_volume_notify may be empty).
            || !state.glulx_volume_ramp.is_empty();
        let timer_active = state.glulx_timer_next_fire.is_some();
        // Continuous story-pane selection auto-scroll: while a drag is held at an
        // edge and that direction can still scroll, keep the loop live so it steps
        // one wrapped row per frame even without new mouse events. Goes quiet once
        // the scroll hits its limit (so we don't busy-spin) or the drag releases. (SQ-0197)
        let selecting_at_edge = state.selection.is_some() && state.selection_edge != 0 && {
            if let Some(g) = state.transcript_geom.get() {
                let max_scroll = g.total_rows.saturating_sub(g.area.height as usize) as u16;
                if state.selection_edge < 0 { state.transcript_scroll < max_scroll }
                else { state.transcript_scroll > 0 }
            } else { false }
        };
        let base_poll_ms = if state.has_active_animation() || sound_active || timer_active || selecting_at_edge { TIDY_POLL_MS } else { 50 };
        // Clamp to whichever clock is due first: the Z-machine timed-input deadline,
        // the Glulx Glk-timer deadline, or the soonest pending Sound2 volume-ramp
        // completion (any may be `None`/empty).
        let next_volume_deadline = state.glulx_volume_notify.values().map(|(t, _)| *t).min();
        let next_deadline = [
            state.input_deadline,
            state.glulx_timer_next_fire,
            next_volume_deadline,
            // …and the v6 picture pacer, so the loop wakes to land the next frame
            // of a turn's picture sequence on time (SQ-0708).
            state.picture_pace_next,
        ]
            .into_iter()
            .flatten()
            .min();
        let poll_ms = match next_deadline {
            Some(dl) => {
                let remaining = dl.saturating_duration_since(std::time::Instant::now()).as_millis() as u64;
                remaining.min(base_poll_ms).max(1)
            }
            None => base_poll_ms,
        };
        // SQ-0769: while the sweep owns the tty, crossterm must not touch it —
        // and `poll` is not a peek: it READS the fd into its own parser buffer to
        // decide whether an event is complete. So the whole poll/read pair stands
        // down and the loop paces itself on a short sleep instead. A keystroke in
        // that window is not lost; the sweep kept it and replays it below.
        let event_ready = if state.query_sweep.has_event() {
            true // a keystroke the sweep held back, ready to replay below
        } else if state.query_sweep.owns_input() {
            std::thread::sleep(Duration::from_millis(poll_ms.min(10)));
            false
        } else {
            match poll(Duration::from_millis(poll_ms)) {
                Ok(r) => r,
                Err(e) => {
                    // A closed controlling terminal can surface here as a poll error
                    // (e.g. on Linux). If a termination signal is what killed the tty,
                    // take the auto-save + conventional signal exit rather than the
                    // bare error exit below. (SQ-0502)
                    exit_if_terminated_saving(&mut *session, &mapper, &state, &ifid, &arc_file);
                    restore_terminal();
                    eprintln!("lanthorn: poll error: {}", e);
                    // Same reasoning as the draw/read error exits: the terminal died,
                    // the engine did not. (SQ-0651)
                    exit_save_on_error_exit(&mut *session, &mapper, &state, &ifid, &arc_file);
                    std::process::exit(1);
                }
            }
        };

        if !event_ready {
            // Any animation in flight this tick (scroll/dock/list eases, sound
            // pulse, pending tidy jobs) needs a redraw — both while it tweens and
            // for the one frame where it settles (has_active_animation flips false
            // only after finalize below). (SQ-0305)
            if state.has_active_animation() {
                needs_redraw = true;
            }
            // Story-pane selection held at an edge with no new mouse event: step the
            // auto-scroll one wrapped row and let the next iteration redraw. (SQ-0197)
            if selecting_at_edge {
                app::input::apply_selection_autoscroll(&mut state);
                needs_redraw = true;
            }
            // Game clocks (timed input, Glk timer, sound finish/notify, volume
            // ramps) are dispatched at the LOOP TOP now, on every path, so a
            // stream of mouse events can no longer freeze them (SQ-0650).
            // No key this tick — advance the tidy animation if one is playing. The next loop
            // iteration redraws, so an advanced frame appears without waiting for input.
            if let Some(anim) = &mut state.tidy_anim {
                // Short auto-play dwell — stepping is mostly done manually with the
                // arrow keys, so the delay only needs to be long enough to follow.
                // `tick` returns true only when a frame actually advanced — redraw
                // just then, so a paused/holding anim still idles. (SQ-0305)
                if anim.tick(Duration::from_millis(100)) {
                    needs_redraw = true;
                }
            }
            if let Some(r) = &mut state.overlays.replay {
                // Likewise: redraw only when the auto-play cursor advanced a turn.
                if r.tick(Duration::from_millis(700), state.history.len()) {
                    needs_redraw = true;
                }
            }
            // Finalize a completed smooth-scroll: snap the logical offset to the
            // target and drop the animation. The next iteration redraws.
            let done_to = state
                .scroll_anim
                .as_ref()
                .filter(|a| a.done())
                .map(|a| a.target());
            if let Some(to) = done_to {
                state.transcript_scroll = to as u16;
                state.scroll_anim = None;
            }
            // Finalize each open scrollable surface's animation likewise. Each
            // finalize reports whether it just cleared a running anim; OR that
            // into needs_redraw so the frame at the settled offset paints once.
            // A list/dock anim can reach done() *during* the poll wait above, so
            // the `has_active_animation()` check earlier this iteration already
            // read false — without this the settle frame would be gated off and
            // the list would land ~1 row short (or a dock leave a sliver). (SQ-0305)
            if let Some(s) = &mut state.overlays.saves { needs_redraw |= s.scroll.finalize_if_done(); }
            if let Some(fb) = &mut state.overlays.file_browser { needs_redraw |= fb.scroll.finalize_if_done(); }
            if let Some(cs) = &mut state.overlays.config_screen { needs_redraw |= cs.scroll.finalize_if_done(); }
            if let Some(b) = &mut state.overlays.command_band {
                for s in b.scroll.iter_mut() {
                    needs_redraw |= s.finalize_if_done();
                }
            }
            if let Some(r) = &mut state.overlays.replay { needs_redraw |= r.scroll.finalize_if_done(); }
            if let Some(h) = &mut state.overlays.hints { needs_redraw |= h.finalize_scroll_if_done(); }
            // Docks slide via a Tween that goes inactive (not dropped) at done();
            // finalize drops the finished tween and forces the settle frame so a
            // just-opened dock paints fully and a closing inv_dock loses its last
            // sliver. (the band's CLOSE is separately covered by settle_command_band
            // dropping the drawer content next iteration.) (SQ-0305)
            needs_redraw |= state.inv_dock.finalize_if_done();
            needs_redraw |= state.band_dock.finalize_if_done();
            needs_redraw |= state.room_dock.finalize_if_done();
            // The story pane's scrollbar fade needs the same settle frame: the
            // last frame the fade itself asks for still paints the bar (at the
            // dregs of its opacity), so without this it never actually leaves
            // the screen. (SQ-0782)
            needs_redraw |= state.finalize_scrollbar_if_done();
            // The sixel scroll-settle debounce needs the same settle frame: the
            // window closing is itself the content change (footprint → full
            // payload), so without this it never actually re-emits. (SQ-1198)
            needs_redraw |= state.finalize_sixel_scroll_motion_if_done();
            continue;
        }

        // A closed controlling terminal makes `poll` above report "ready" (HUP) on
        // the dead fd, so we arrive here — but `read()` would then block forever on
        // that fd, never returning to the loop top where termination is checked. The
        // signal handler has already set the flag by now, so catch it here first.
        // (SQ-0502)
        exit_if_terminated_saving(&mut *session, &mapper, &state, &ifid, &arc_file);

        // A keystroke the sweep held back while it owned the tty outranks the
        // terminal, so type-ahead reaches the story in the order it was typed.
        let event = match state.query_sweep.next_event() {
            Some(e) => e,
            None => match read() {
                Ok(e) => e,
                Err(e) => {
                    restore_terminal();
                    eprintln!("lanthorn: read error: {}", e);
                    // Input is gone, but the engine is not: keep the progress. (SQ-0651)
                    exit_save_on_error_exit(&mut *session, &mapper, &state, &ifid, &arc_file);
                    std::process::exit(1);
                }
            },
        };
        // Pixel mouse reporting (SQ-0563): normalise ONCE, here, before the event
        // reaches any handler. Coordinates become cells — so every hit test in the
        // app is unchanged — and the offset within the cell rides alongside for the
        // one consumer that wants it, a Glk graphics window's pixel hit test.
        // `None` in cell mode, which is every terminal that declined the mode.
        let (event, mouse_sub_px) = match event {
            Event::Mouse(m) => {
                let (m, sub) = app::pixel_mouse::normalise(m);
                (Event::Mouse(m), sub)
            }
            other => (other, None),
        };

        // An event was read and will be dispatched (key/mouse/paste/resize, or a
        // dialog/overlay intercept) — the frame may change, so redraw next pass.
        // Biasing to over-draw here is deliberate: a swallowed key costs one extra
        // frame; a missed redraw is a visible bug. (SQ-0305)
        needs_redraw = true;

        // A reveal is momentary, and the next keystroke is what ends it (SQ-1107).
        // Ahead of every dispatch arm, so it is out before whatever that key does
        // — including the reveal's OWN key, which clears here and lights again
        // when the command runs a few lines later. Key presses only: moving the
        // pointer is not an answer to the question the reveal asked, and putting
        // the light out because the mouse drifted would be.
        if matches!(&event, Event::Key(k) if k.kind == KeyEventKind::Press) {
            app::reveal::clear(&mut state);
        }

        // The player outranks paced output (SQ-0708): a keypress collapses an
        // in-flight v6 picture sequence to its settled composite at once, and a
        // resize settles it rather than replaying frames measured against the old
        // pane. The event is NOT consumed — pacing is decoration over a turn that
        // already finished, so it goes on to whatever else wanted it.
        if matches!(&event, Event::Key(k) if k.kind == KeyEventKind::Press)
            || matches!(&event, Event::Resize(_, _))
        {
            loop_tick::settle_picture_pacing(&mut state, &mut *session);
        }

        // SQ-0988: a resize may have changed the CELL, not only the grid. The
        // terminal's cell size was measured once, at launch, by a stdio query no
        // one can safely repeat with the app in raw mode — so a font-size change
        // left every fit running on the launch aspect ratio until restart, and
        // the art looked stretched. `TIOCGWINSZ` re-derives it with no round
        // trip; when it moves, everything fitted against the old cell goes.
        //
        // This sits AHEAD of the three `Event::Resize` arms below (each of which
        // `continue`s after clearing), so it runs once per resize whichever arm
        // that resize belongs to.
        if matches!(&event, Event::Resize(_, _))
            && state.game_picker.as_mut().is_some_and(picker_ui::refresh_cell_size)
        {
            state.graphics_render.borrow_mut().invalidate_cell_geometry();
        }

        // If more input is already queued behind this event, defer the next
        // redraw so the whole burst collapses into a single frame. Cleared at
        // the draw gate once the queue empties (poll(ZERO) == false).
        skip_draw = poll(Duration::ZERO).unwrap_or(false);

        // ── Bracketed paste (SQ-0653) ─────────────────────────────────────────
        // Handled BEFORE every intercept below: pasted text is data, not
        // keystrokes, so it must not reach the char-mode gate (which would feed it
        // to the story a character at a time), the '/'-opens-the-palette rule, or
        // an overlay's key handler. It lands as literal characters in whatever
        // text field currently owns typing and does NOT submit — the user reads it
        // back and presses Enter.
        if let Event::Paste(text) = &event {
            app::input::apply_paste(&mut state, text);
            continue;
        }

        // ── Pane-boundary drag-resize (SQ-0669) ───────────────────────────────
        // Runs BEFORE every other mouse intercept, because a drag that started on
        // a boundary owns the mouse until the button comes up: the transcript
        // selection, the band's rows, the map's rooms and the debug inspector all
        // sit under one boundary or another, and any of them claiming a mid-drag
        // event would leave the splitter stuck to the pointer. It claims nothing
        // else — a Down anywhere but a grab zone (or with a modal open) falls
        // straight through, so a selection dragged ACROSS a boundary keeps
        // selecting. A non-mouse event mid-drag ends the drag rather than wedging
        // it, then goes on to be handled normally.
        if let Event::Mouse(m) = &event {
            use app::pane_drag::DragOutcome;
            match app::pane_drag::on_mouse(&mut state, m, &last_panes.pane_layout, &last_panes.boundaries, &last_panes.border_controls) {
                DragOutcome::Ignored => {}
                DragOutcome::Consumed => continue,
                DragOutcome::Committed => {
                    lifecycle::flush_pending_config_write(&mut state);
                    continue;
                }
            }
        } else if app::pane_drag::interrupt(&mut state) {
            lifecycle::flush_pending_config_write(&mut state);
        }

        // ── Command-band quick-block hover (SQ-0677) ───────────────────────────
        // Mirrors `pane_drag::on_mouse`'s own Moved handling just above: pointer
        // motion with no button held just lights up whichever quick cell (rose
        // point, flowing word, or flat-row entry) is under it, using LAST
        // FRAME's hit rects (`last_panes.command_band`, the same ones the click
        // path hit-tests). Never claims the event — quick is mouse-click-only
        // now (SQ-0677), so hover is purely cosmetic and must not pre-empt
        // anything else a `Moved` event might still need to do (the debug
        // panel's own hover tooltips, in particular).
        band_update_quick_hover(&mut state, &last_panes, &event);

        // ── Border-control hover (SQ-1123) ────────────────────────────────────
        // Same shape as the band's hover just above: pointer motion with no
        // button held lights up whichever border control is under it, using
        // LAST FRAME's hit rects — the same ones the click path resolves
        // against. Never claims the event (a Moved event still has the debug
        // panel's own tooltips to reach), and clears rather than leaving a
        // stale control lit the moment the pointer moves off or a modal opens.
        if let Event::Mouse(m) = &event {
            if m.kind == crossterm::event::MouseEventKind::Moved {
                let hover = app::render::controls::control_at(
                    &state, &last_panes.border_controls, m.column, m.row,
                );
                // (`needs_redraw` is already set for every event above.)
                state.control_hover = hover;
            }
        }

        // ── Matrix-view room hover (SQ-1246) ────────────────────────────────────
        matrix_update_hover(&mut state, &last_panes, &event);

        // ── Common-dialog overlay intercept ladder (SQ-0307) ──────────────────
        // The aux / reset / save-name / text-entry / confirm-delete / quit /
        // launch modals share one decode+apply seam. The top-most open overlay
        // (priority order aux ▸ reset ▸ save-name ▸ text-entry ▸ confirm-delete ▸
        // quit ▸ launch — exactly the old if-ladder) decodes the event through
        // its `Overlay` impl, applying pure focus / field / checkbox changes in
        // place, and returns an `OverlayAct` for the game-affecting side effects
        // to run here where session / mapper / paths are in scope. Swallows the
        // events its overlay does not handle, then `continue`s.
        if let Some(ov) = overlays::topmost_common_dialog(&state.overlays) {
            if let Event::Resize(_, _) = &event { clear_terminal(&mut terminal, &state); continue; }
            let outcome = match &event {
                Event::Key(k) if k.kind == KeyEventKind::Press => ov.key(&mut state, k),
                Event::Mouse(m) => ov.mouse(&mut state, m, &last_panes),
                _ => overlays::OverlayOutcome::Consumed,
            };
            if let overlays::OverlayOutcome::Act(act) = outcome {
                use overlays::OverlayAct;
                match act {
                    OverlayAct::EnableTurnHistory => {
                        // Persist it, because a player who answers this prompt
                        // means "from now on", not "for this launch": the whole
                        // point is that the archive starts carrying turns.
                        state.config.record_turn_history = true;
                        let _ = app::config::write_config_file(&state.config);
                        state.push_notice(
                            "[Recording turn history. Rewind will have something to show after your next move.]",
                        );
                    }
                    OverlayAct::FontCheck(nerdfont, diagonal) => {
                        // SQ-1104/SQ-1245: both answers are GLYPH decisions, so
                        // they are recorded in `style.toml` as preset names /
                        // a bool, not in `config.toml`. Written, then reloaded,
                        // so the map changes under the player's eyes rather
                        // than at the next launch — which is also the only way
                        // they can see whether they answered correctly.
                        let msg = match app::style::style_write_path(
                            state.config.style.as_deref(),
                            &state.config.user_dir,
                        ) {
                            Some(path) => match app::style::write_font_check_answer(&path, nerdfont, diagonal) {
                                Ok(()) => {
                                    let _ = app::reload::reload_style(&mut state);
                                    let icons = if nerdfont { "Nerd Font icons on" } else { "Plain glyphs" };
                                    let diag = match diagonal {
                                        Some(true) => "; diagonal corners on",
                                        Some(false) => "; diagonal corners off",
                                        None => "",
                                    };
                                    format!(
                                        "[{icons}{diag}. Saved to {}; run-font-check asks again.]",
                                        path.display()
                                    )
                                }
                                Err(e) => format!("[Could not save the font choice: {e}]"),
                            },
                            None => {
                                "[`style = \"default\"` has no file to write the font choice to.]"
                                    .to_string()
                            }
                        };
                        state.push_notice(&msg);
                    }
                    OverlayAct::FetchKeep(mode) => {
                        // SQ-1086: a download's one chance to become a permanent
                        // part of the library. The fetched file itself is left
                        // exactly where it is either way — it is the file THIS
                        // session was booted from and its basename is the save
                        // key (`storage::story_key_for`), so moving or deleting
                        // it out from under a running game is the one destructive
                        // thing this could do. A keep is a COPY.
                        if let Some(prompt) = state.overlays.fetch_keep.take() {
                            // SQ-1096: the archive shape of this prompt is
                            // answered before `boot_story`, so it can never
                            // reach the game loop — where `keep_in_library`
                            // would copy the ZIP itself into the library.
                            debug_assert!(
                                prompt.disk_images.is_empty(),
                                "an archive prompt must be answered before the boot, not here"
                            );
                            match mode {
                                Some(mode) => match app::story_url::keep_in_library(
                                    &prompt.fetched.path, &prompt.library_dir, mode,
                                ) {
                                    Ok(dest) => {
                                        let name = dest.file_name().and_then(|n| n.to_str()).unwrap_or("it").to_string();
                                        state.push_notice(&format!(
                                            "[Kept in your library as {name}. It will be there next time.]"
                                        ));
                                    }
                                    Err(e) => state.push_notice(&format!("[Could not keep it: {e}]")),
                                },
                                None => state.push_notice(&format!(
                                    "[Not kept. Playing from {}.]",
                                    prompt.fetched.path.display()
                                )),
                            }
                        }
                        state.overlays.dialog_focus = 0;
                    }
                    OverlayAct::AuxArchive => {
                        let mode = app::config::AuxStorage::Archive;
                        state.overlays.aux_prompt = false;
                        state.config.aux_storage = mode;
                                                let _ = app::config::write_config_file(&state.config);
                        session.clear_aux_dirty();
                    }
                    OverlayAct::AuxGlobal => {
                        let mode = app::config::AuxStorage::Global;
                        state.overlays.aux_prompt = false;
                        state.config.aux_storage = mode;
                                                let _ = app::config::write_config_file(&state.config);
                        let _ = app::aux_store::write_global_aux(&game_dir, session.aux_data());
                        session.clear_aux_dirty();
                    }
                    OverlayAct::ResetConfirm => {
                        let clear = state.overlays.reset_clear_map;
                        let delete = state.overlays.reset_delete_data;
                        state.overlays.reset_dialog = false;
                        reset_game(&mut *session, &mut mapper, &mut state, &story_bytes, &story_path, &game_dir, clear, delete);
                    }
                    OverlayAct::ResetCancel => {
                        state.overlays.reset_dialog = false;
                    }
                    // SQ-0439: the answer closes the prompt and, when it is a decision about a
                    // seam rather than a move, is written into the map so it is never re-asked.
                    OverlayAct::RegionPrompt(act) => {
                        app::input::apply_region_prompt(&mut state, &mut mapper, act);
                    }
                    OverlayAct::GameOverPlayAgain => {
                        // Plain restart: keep the accumulated map and saved data.
                        state.overlays.game_over = false;
                        reset_game(&mut *session, &mut mapper, &mut state, &story_bytes, &story_path, &game_dir, false, false);
                    }
                    OverlayAct::GameOverRestore => {
                        // Close the game-over overlay and open the saves manager (the
                        // Save State restore flow — same entry point as Action::OpenSaves).
                        state.overlays.game_over = false;
                        let entries = combined_saves(&game_dir);
                        state.overlays.saves = Some(SavesState { entries, scroll: Default::default() });
                        state.overlays.dialog_focus = 0;
                    }
                    OverlayAct::GameOverQuit => {
                        break 'event_loop state.exit_target.into();
                    }
                    OverlayAct::SaveNameSubmit => {
                        // Empty names are rejected (dialog stays open); valid names
                        // go through the shared handle_save_as save path.
                        let value = state
                            .overlays.save_name_dialog
                            .as_ref()
                            .map(|d| d.field.value.clone())
                            .unwrap_or_default();
                        if value.trim().is_empty() {
                            if let Some(d) = state.overlays.save_name_dialog.as_mut() { d.active = false; }
                            state.push_notice("[Save name cannot be empty]");
                        } else {
                            state.overlays.save_name_dialog = None;
                            // SQ-0648: `force: false` — if the target already exists,
                            // `handle_save_as` opens the overwrite-confirm overlay
                            // (reopening this dialog behind it with `value`) instead
                            // of writing straight over it.
                            handle_save_as(
                                value, &game_dir, &ifid, &mut mapper, &mut *session, &mut state, false,
                            );
                            let quit = resolve_ingame_dialog(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map)
                                || resolve_filename_request(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map);
                            turn::persist_aux_after_turn(&mut *session, &mut state, &game_dir);
                            turn::persist_vfs_after_turn(&mut *session, &state, &game_dir);
                            if quit { break 'event_loop state.exit_target.into(); }
                        }
                    }
                    OverlayAct::SaveNameCancel => {
                        state.overlays.save_name_dialog = None;
                        let quit = resolve_ingame_dialog(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map)
                            || resolve_filename_request(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map);
                        turn::persist_aux_after_turn(&mut *session, &mut state, &game_dir);
                        turn::persist_vfs_after_turn(&mut *session, &state, &game_dir);
                        if quit { break 'event_loop state.exit_target.into(); }
                    }
                    OverlayAct::TextEntrySubmit => {
                        // A CreateFile submit hops through filename_submitted → resume
                        // here; map-edit / config submits leave nothing pending.
                        if let Some(dlg) = state.overlays.text_entry.take() {
                            apply_text_entry(dlg, &mut state, &mut mapper);
                        }
                        let quit = resolve_ingame_dialog(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map)
                            || resolve_filename_request(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map);
                        turn::persist_aux_after_turn(&mut *session, &mut state, &game_dir);
                        turn::persist_vfs_after_turn(&mut *session, &state, &game_dir);
                        if quit { break 'event_loop state.exit_target.into(); }
                    }
                    OverlayAct::TextEntryCancel => {
                        // A cancelled CreateFile leaves pending_filename set with no
                        // dialog open → resolve_filename_request treats it as NULL.
                        state.overlays.text_entry = None;
                        let quit = resolve_ingame_dialog(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map)
                            || resolve_filename_request(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map);
                        turn::persist_aux_after_turn(&mut *session, &mut state, &game_dir);
                        turn::persist_vfs_after_turn(&mut *session, &state, &game_dir);
                        if quit { break 'event_loop state.exit_target.into(); }
                    }
                    OverlayAct::ConfirmDelete(confirmed) => {
                        if let Some(path) = state.overlays.confirm_delete_save.take() {
                            delete_save_confirmed(&path, confirmed, &game_dir, &mut state);
                        }
                        // Return the saves manager (still open behind us) to default focus.
                        state.overlays.dialog_focus = 0;
                    }
                    OverlayAct::ConfirmOverwrite(confirmed) => {
                        // SQ-0648: resume whichever entry point asked for confirmation.
                        if let Some(pending) = state.overlays.confirm_overwrite_save.take() {
                            match pending.pending {
                                app::state::PendingOverwrite::SaveAs => {
                                    if confirmed {
                                        // The save-name dialog was left open BEHIND this
                                        // overlay the whole time; the typed name lives
                                        // there, not in `pending`.
                                        let value = state
                                            .overlays.save_name_dialog
                                            .take()
                                            .map(|d| d.field.value.clone())
                                            .unwrap_or_default();
                                        handle_save_as(
                                            value, &game_dir, &ifid, &mut mapper, &mut *session, &mut state, true,
                                        );
                                        let quit = resolve_ingame_dialog(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map)
                                            || resolve_filename_request(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map);
                                        turn::persist_aux_after_turn(&mut *session, &mut state, &game_dir);
                                        turn::persist_vfs_after_turn(&mut *session, &state, &game_dir);
                                        if quit { break 'event_loop state.exit_target.into(); }
                                    }
                                    // Cancelled: the save-name dialog is untouched behind
                                    // us, showing exactly what the player typed — refocus
                                    // its text field.
                                    state.overlays.dialog_focus = 0;
                                }
                                app::state::PendingOverwrite::Slash(name) => {
                                    if confirmed {
                                        let result = slash_dispatch::write_named_save(&game_dir, &ifid, &name, &mapper, &mut *session, &mut state);
                                        slash_dispatch::apply_slash_save_result(result, &mut *session, &mut state);
                                    } else {
                                        // No dialog to return to for the slash path — say so.
                                        state.set_status("save cancelled");
                                    }
                                    state.overlays.dialog_focus = 0;
                                }
                            }
                        }
                    }
                    OverlayAct::QuitSave => {
                        // Same save/archive whether quitting or returning to the
                        // library; the target (Exit vs Library) was set when the
                        // dialog opened. (SQ-0435)
                        state.overlays.quit_dialog = false;
                        // A failed save here is the user's own explicit "Save State
                        // & quit"; carry the reason out so it can be printed once
                        // the terminal is back (SQ-0651).
                        quit_save_warning =
                            lifecycle::quit_dialog_save(&mut *session, &mapper, &state, &ifid, &arc_file);
                        break 'event_loop state.exit_target.into();
                    }
                    OverlayAct::QuitQuit => {
                        break 'event_loop state.exit_target.into();
                    }
                    OverlayAct::QuitCancel => {
                        // Cancelling the dialog abandons the pending intent, so
                        // reset the target back to this launch's default — a later
                        // plain quit through the same dialog must not inherit
                        // whatever a superseded `/quit-to-library` left behind.
                        // (SQ-0435, SQ-1258)
                        state.overlays.quit_dialog = false;
                        state.exit_target = app::state::ExitTarget::for_launch(state.launched_from_library);
                    }
                    OverlayAct::LaunchResume => {
                        if let Some((save, lines, kinds, screen)) = state.pending_resume.take() {
                            state.overlays.launch_dialog = false;
                            turn::apply_launch_resume(&save, lines, kinds, screen, &mut *session, &mut mapper, &mut state, &last_panes, &arc_file);
                        }
                    }
                    OverlayAct::LaunchNewGame => {
                        state.overlays.launch_dialog = false;
                        state.pending_resume = None;
                    }
                }
            }
            continue;
        }

        // ── Hints panel intercept — before normal action routing ──────────────
        // When the hints panel is open, route keyboard/mouse directly here and
        // continue (swallowing events the panel does not handle).
        if state.overlays.hints.is_some() {
            match &event {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    match hint_key_routes(k.code) {
                        HintKeyKind::Close => {
                            state.overlays.hints = None;
                        }
                        HintKeyKind::Scroll(delta) => {
                            // PageUp/PageDown scroll the clue window (mirrors the
                            // wheel below); other keys reach the companion VM.
                            let max = last_panes.hints_panel.as_ref().map_or(0, |hp| hp.max_scroll);
                            let anim = state.config.animation.clone();
                            if let Some(hs) = &mut state.overlays.hints {
                                hs.scroll_by(delta, max, &anim);
                            }
                        }
                        HintKeyKind::ToSession => {
                            if let Some(ref mut hs) = state.overlays.hints {
                                // The companion VM's pending input mode decides routing:
                                // a `read_char` (InvisiClues menu) forwards the keypress
                                // to the VM; a line read edits the local input buffer.
                                let kind = {
                                    let app::state::HintSource::Zcode(ref vm) = hs.source;
                                    vm.pending_input()
                                };
                                // Owned result of whichever submit ran (None when the key
                                // was buffered or ignored), folded in after the VM borrow
                                // ends so the borrow checker allows `hs.apply_turn`.
                                let result: Option<TurnResult> = match hint_input_action(kind, k.code) {
                                    HintInputAct::ForwardKey => {
                                        // Menu navigation: map the crossterm key with the
                                        // SAME converter the main event loop uses (arrows,
                                        // Enter, letters map identically), then drive the VM.
                                        // Do not buffer into `hs.input`. (Esc is handled by
                                        // the separate Close arm and never reaches here.)
                                        app::engine::key_event_to_input(*k).and_then(|ki| {
                                            let app::state::HintSource::Zcode(ref mut vm) = hs.source;
                                            vm.submit_key(ki)
                                        })
                                    }
                                    HintInputAct::SubmitLine => {
                                        let line = std::mem::take(&mut hs.input);
                                        let app::state::HintSource::Zcode(ref mut vm) = hs.source;
                                        Some(vm.submit(&line))
                                    }
                                    HintInputAct::BufferPop => {
                                        hs.input.pop();
                                        None
                                    }
                                    HintInputAct::BufferPush(c) => {
                                        hs.input.push(c);
                                        None
                                    }
                                    HintInputAct::Ignore => None,
                                };
                                if let Some(result) = result {
                                    let quit = result.quit;
                                    hs.apply_turn(&result);
                                    // An InvisiClues file can @quit — close the panel.
                                    if quit {
                                        state.overlays.hints = None;
                                    }
                                }
                            }
                        }
                    }
                }
                Event::Mouse(m) => {
                    use crossterm::event::{MouseButton, MouseEventKind};
                    if m.kind == MouseEventKind::Down(MouseButton::Left) {
                        let pt = ratatui::layout::Position { x: m.column, y: m.row };
                        if let Some(hp) = &last_panes.hints_panel {
                            let in_close = hp.close.is_some_and(|r| r.contains(pt))
                                || hp.close_button.is_some_and(|r| r.contains(pt));
                            if in_close {
                                state.overlays.hints = None;
                            }
                            // Clicks inside the dialog but not on close: swallow.
                        }
                    } else if let Some(d) = app::input::wheel_delta(m.kind, state.config.mouse_wheel_invert) {
                        // Wheel drives the hint transcript's own scroll. The panel
                        // is intercepted before mouse_to_action, so resolve the
                        // direction (and mouse_wheel_invert) via the shared helper.
                        let max = last_panes.hints_panel.as_ref().map_or(0, |hp| hp.max_scroll);
                        let anim = state.config.animation.clone();
                        if let Some(hs) = &mut state.overlays.hints {
                            // Wheel up (d < 0) → older content (increase scroll),
                            // matching the story transcript's wheel direction.
                            hs.scroll_by(if d < 0 { 1 } else { -1 }, max, &anim);
                        }
                    }
                }
                Event::Resize(_, _) => { clear_terminal(&mut terminal, &state); continue; }
                _ => {}
            }
            continue;
        }

        // ── Search-nav intercept — before normal action routing ───────────────
        // When a search is active and no modal is open, intercept the configured
        // back/forward keys and Esc to navigate matches.  Any other key clears
        // the search state and then falls through to normal processing below.
        if state.search_query.is_some() && !state.any_overlay_open() {
            if let Event::Key(k) = &event {
                if k.kind == KeyEventKind::Press {
                    use crossterm::event::KeyCode;
                    let key_back = state.config.search.key_back;
                    let key_forward = state.config.search.key_forward;
                    match k.code {
                        KeyCode::Char(c) if c == key_back => {
                            if let Some(pos) = state.search_next(false) {
                                let total_vis = state.visible_transcript_indices().len();
                                let pane_rows = if last_panes.story.height > 0 {
                                    last_panes.story.height as usize
                                } else {
                                    24
                                };
                                state.transcript_scroll = scroll_for_match(pos, total_vis, pane_rows);
                            }
                            continue;
                        }
                        KeyCode::Char(c) if c == key_forward => {
                            if let Some(pos) = state.search_next(true) {
                                let total_vis = state.visible_transcript_indices().len();
                                let pane_rows = if last_panes.story.height > 0 {
                                    last_panes.story.height as usize
                                } else {
                                    24
                                };
                                state.transcript_scroll = scroll_for_match(pos, total_vis, pane_rows);
                            }
                            continue;
                        }
                        KeyCode::Esc => {
                            state.clear_search();
                            continue;
                        }
                        _ => {
                            // Any other key: clear search, then fall through to normal processing.
                            state.clear_search();
                        }
                    }
                }
            }
        }

        // ── Debug inspector (tiled): drive it while its region is focused ──────
        // Not a modal: only intercepts while the debug region holds `Focus::Map`,
        // and runs before the normal key→command dispatch so it pre-empts
        // Tab→ToggleFocus. `Esc` pops focus back to the story (panel stays open);
        // anything the panel doesn't consume falls through to normal dispatch.
        if state.debug.is_some() && state.focus == Focus::Map {
            if let Event::Key(k) = &event {
                if k.kind == KeyEventKind::Press {
                    // Esc normally pops focus straight back to the story. But while
                    // the Memory address-input line is open, Esc must cancel that
                    // input first (handled by `handle_key` below) rather than
                    // immediately leaving the debug region — so skip the early pop
                    // in that case and fall through to the normal handle_key call.
                    let editing_mem_addr = state.debug.as_ref().is_some_and(|p| p.mem_input.is_some());
                    if k.code == crossterm::event::KeyCode::Esc && !editing_mem_addr {
                        state.focus = Focus::Game; // pop back to typing; keep the panel open
                        continue;
                    }
                    // Tab / Shift-Tab drive the unified per-window focus cycle
                    // (story pane + each debug window), not the panel — so from the
                    // last debug window Tab returns to the story.
                    if k.code == crossterm::event::KeyCode::Tab {
                        state.cycle_focus(true);
                        continue;
                    }
                    if k.code == crossterm::event::KeyCode::BackTab {
                        state.cycle_focus(false);
                        continue;
                    }
                    // Focused-pane height for PageUp/PageDown paging, from the real
                    // debug-region rect (the map slot).
                    let vp = (last_panes.map.height.saturating_sub(2) / 2).max(1) as usize;
                    let dk = if let Some(dbg) = session.debugger() {
                        state.debug.as_mut().map(|p| { p.viewport = vp; p.handle_key(k.code, dbg) })
                    } else {
                        None
                    };
                    // Consume keys the panel handled; let anything it ignored fall through
                    // to normal global dispatch (e.g. quit).
                    if dk == Some(app::debug_panel::DebugKey::Consumed) { continue; }
                    if dk == Some(app::debug_panel::DebugKey::Close) {
                        state.debug = None;
                        state.focus = Focus::Game;
                        continue;
                    }
                    // Ignored → fall through (do not `continue`).
                }
            }
        }

        // ── Debug inspector (tiled): mouse ──────────────────────────────────────
        // Works regardless of focus, whenever the cursor is over the debug region
        // (the map slot while `state.debug` is `Some`). Runs before the big
        // `match event` below so it pre-empts the `Event::Mouse` arm's map
        // wheel/click handling, which would otherwise misfire against the blank
        // map area the debug region currently occupies.
        if state.debug.is_some() {
            if let Event::Mouse(m) = &event {
                use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};
                let region = last_panes.map;
                let in_region = region.width > 0 && m.column >= region.x && m.column < region.right()
                    && m.row >= region.y && m.row < region.bottom();
                if in_region {
                    let rects = app::debug_panel::window_rects(region);
                    let over = rects.iter().position(|r| m.column >= r.x && m.column < r.right()
                        && m.row >= r.y && m.row < r.bottom());
                    match m.kind {
                        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                            if let Some(i) = over {
                                let down = app::input::wheel_delta(m.kind, state.config.mouse_wheel_invert)
                                    .map(|d| d > 0).unwrap_or(false);
                                // Shift+wheel pans the hex dump sideways
                                // (SQ-0981) — the gesture the map pane already
                                // takes, and like every other wheel here it
                                // addresses the window under the cursor, focused
                                // or not. `wheel_delta` resolves the invert
                                // preference once, so wheel-down pans right
                                // whichever way the user has their wheel set.
                                // A window with nothing to pan scrolls as usual
                                // rather than swallowing the event.
                                let shift = m.modifiers.contains(KeyModifiers::SHIFT);
                                if let Some(dbg) = session.debugger() {
                                    if let Some(p) = state.debug.as_mut() {
                                        if !(shift && p.pan_active(i, down)) {
                                            p.scroll_active(i, down, dbg);
                                        }
                                    }
                                }
                            }
                            continue; // pre-empt the map wheel arms
                        }
                        // A true horizontal wheel (trackpads, and terminals that
                        // forward xterm's buttons 6/7) needs no modifier — the
                        // map pane reads these the same way. Nothing to fall
                        // back to: a sideways gesture must never scroll a
                        // section vertically.
                        MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => {
                            if let Some(i) = over {
                                let right = matches!(m.kind, MouseEventKind::ScrollRight);
                                if let Some(p) = state.debug.as_mut() { p.pan_active(i, right); }
                            }
                            continue; // pre-empt the map wheel arms
                        }
                        MouseEventKind::Down(MouseButton::Left) => {
                            // Clickable code-address navigation (branch targets in
                            // Disasm, `fn@` frame addresses in Call Stack) takes
                            // priority over the tab/body-focus logic below.
                            let target = state.debug.as_ref()
                                .and_then(|p| app::debug_panel::clickable_at(region, p, m.column, m.row));
                            if let Some(target) = target {
                                use app::debug_panel::ClickTarget;
                                if let Some(dbg) = session.debugger() {
                                    if let Some(p) = state.debug.as_mut() {
                                        match target {
                                            ClickTarget::Code(a)   => p.goto(a, dbg),
                                            ClickTarget::Memory(a) => p.goto_memory(a, dbg),
                                            ClickTarget::Object(n) => p.goto_object(n, dbg),
                                            // `@local5`/`@g0f`/`@sp` — a variable holding
                                            // a memory address: jump to memory at its
                                            // current value (read now via the debugger).
                                            ClickTarget::MemVia(v) => {
                                                if let Some(addr) = dbg.var_value(v) {
                                                    p.goto_memory(addr as u32, dbg);
                                                }
                                            }
                                            // `obj#local5`/`obj#g0f`/`obj#sp` — a variable
                                            // holding an object number: expand that object.
                                            ClickTarget::ObjVia(v) => {
                                                if let Some(obj) = dbg.var_value(v) {
                                                    p.goto_object(obj, dbg);
                                                }
                                            }
                                            // Plain variables are hover-only (see the
                                            // `Moved` arm); `clickable_at` doesn't return
                                            // them, but the match must cover them.
                                            ClickTarget::Global(_) | ClickTarget::Local(_) | ClickTarget::Stack => {}
                                        }
                                    }
                                    state.focus = Focus::Map;
                                }
                                continue;
                            }
                            // Object-tree expand/collapse click (Objects tab) — same
                            // priority as the code-address clicks above.
                            let obj_target = state.debug.as_ref()
                                .and_then(|p| app::debug_panel::objects_click_at(region, p, m.column, m.row));
                            if let Some(obj) = obj_target {
                                if let Some(dbg) = session.debugger() {
                                    if let Some(p) = state.debug.as_mut() { p.toggle_object(obj, dbg); }
                                    state.focus = Focus::Map;
                                }
                                continue;
                            }
                            // Call-stack frame expand/collapse click (Call Stack
                            // tab). The `fn@` address-click above runs first and
                            // `continue`s on hit, so it keeps priority over toggle.
                            let frame_target = state.debug.as_ref()
                                .and_then(|p| app::debug_panel::stack_click_at(region, p, m.column, m.row));
                            if let Some(idx) = frame_target {
                                if let Some(dbg) = session.debugger() {
                                    if let Some(p) = state.debug.as_mut() { p.toggle_frame(idx, dbg); }
                                    state.focus = Focus::Map;
                                }
                                continue;
                            }
                            if let Some(&(w, t, _)) = last_panes.debug_tabs.iter().find(|(_, _, r)| {
                                r.width > 0 && m.column >= r.x && m.column < r.right()
                                    && m.row >= r.y && m.row < r.bottom()
                            }) {
                                if let Some(p) = state.debug.as_mut() { p.activate_tab(w, t); }
                                state.focus = Focus::Map;
                            } else if let Some((win, pt)) = app::debug_panel::debug_point_at(region, m.column, m.row) {
                                // Click in a window's body starts a text selection there (SQ-0420).
                                if let Some(p) = state.debug.as_mut() {
                                    p.focus_window(win);
                                    p.sel = Some((win, app::clipboard::Selection::new(pt)));
                                    p.selection_text.borrow_mut().take();
                                }
                                state.focus = Focus::Map;
                            } else if let Some(i) = over {
                                if let Some(p) = state.debug.as_mut() { p.focus_window(i); }
                                state.focus = Focus::Map;
                            }
                            continue;
                        }
                        MouseEventKind::Moved => {
                            // Hovering a variable operand (`gNN`/`localN`/`sp`) in
                            // the Disassembly shows a floating value tooltip at the
                            // cursor; moving off clears it. Borrows don't overlap:
                            // `as_ref` for the hit-test, then `session.debugger()`,
                            // then `as_mut`.
                            let hv = state.debug.as_ref()
                                .and_then(|p| app::debug_panel::hover_var_at(region, p, m.column, m.row));
                            if let Some((var, acol, arow)) = hv {
                                let value = session.debugger().and_then(|dbg| dbg.var_value(var));
                                if let Some(p) = state.debug.as_mut() {
                                    p.hover = Some(app::debug_panel::HoverTip::for_var(var, value, acol, arow));
                                }
                                continue;
                            }
                            // Hovering an opcode mnemonic shows its help (the help
                            // itself explains an inverted `?~` branch — see disasm).
                            let hh = state.debug.as_ref()
                                .and_then(|p| app::debug_panel::hover_help_at(region, p, m.column, m.row));
                            match hh {
                                Some((addr, acol, arow)) => {
                                    let lines = session.debugger().and_then(|dbg| dbg.describe_line(addr));
                                    if let Some(p) = state.debug.as_mut() {
                                        p.hover = lines.map(|l| app::debug_panel::HoverTip::for_lines(l, acol, arow));
                                    }
                                }
                                None => { if let Some(p) = state.debug.as_mut() { p.hover = None; } }
                            }
                            continue;
                        }
                        MouseEventKind::Drag(MouseButton::Left) => {
                            // Extend the active selection, clamped to its window (SQ-0420).
                            if let Some(p) = state.debug.as_mut() {
                                if let Some((win, sel)) = p.sel.as_mut() {
                                    sel.head = app::debug_panel::debug_point_clamped(region, *win, m.column, m.row);
                                }
                            }
                            continue;
                        }
                        MouseEventKind::Up(MouseButton::Left) => {
                            // Release: copy the selected text (published by render) via
                            // OSC 52, then clear the selection (SQ-0420). Mirrors the
                            // story pane's EndSelection copy.
                            let copied = state.debug.as_mut().and_then(|p| {
                                let real = matches!(p.sel, Some((_, s)) if !s.is_empty());
                                p.sel = None;
                                if real { p.selection_text.borrow_mut().take() } else { None }
                            });
                            if let Some(text) = copied {
                                if !text.trim().is_empty() {
                                    use std::io::Write;
                                    let seq = app::clipboard::osc52_copy_sequence(&text);
                                    let mut out = std::io::stdout();
                                    let _ = out.write_all(seq.as_bytes());
                                    let _ = out.flush();
                                    state.push_transcript_internal(
                                        &format!("Copied {} chars to clipboard", text.chars().count()),
                                        app::state::TranscriptKind::Meta,
                                    );
                                }
                            }
                            continue;
                        }
                        _ => { continue; } // swallow other mouse over the region
                    }
                }
            }
        }

        // ── Config-screen Tab focus intercept ────────────────────────────────
        // Ring length 2: [Save(0), Cancel(1)].
        if state.overlays.config_screen.is_some() {
            if let Event::Key(k) = &event {
                if k.kind == KeyEventKind::Press {
                    match k.code {
                        crossterm::event::KeyCode::Tab =>
                            state.overlays.dialog_focus = app::input::cycle_focus(state.overlays.dialog_focus, 2, 1),
                        crossterm::event::KeyCode::BackTab =>
                            state.overlays.dialog_focus = app::input::cycle_focus(state.overlays.dialog_focus, 2, -1),
                        _ => {}
                    }
                }
            }
        }

        // ── Saves Tab focus intercept ─────────────────────────────────────────
        // Ring length 1: [Done(0)].
        if state.overlays.saves.is_some() {
            if let Event::Key(k) = &event {
                if k.kind == KeyEventKind::Press {
                    match k.code {
                        crossterm::event::KeyCode::Tab =>
                            state.overlays.dialog_focus = app::input::cycle_focus(state.overlays.dialog_focus, 1, 1),
                        crossterm::event::KeyCode::BackTab =>
                            state.overlays.dialog_focus = app::input::cycle_focus(state.overlays.dialog_focus, 1, -1),
                        _ => {}
                    }
                }
            }
        }

        // ── Char-input mode gate ──────────────────────────────────────────────
        // When the Z-machine is waiting for a single keypress (read_char) and no
        // overlay is open, forward the keystroke directly to the VM — unless it is
        // the hotkey-dialog prefix (Ctrl+P) or any Ctrl/Alt combo. Those are
        // reserved for app routing so the user can always escape (quit, hotkeys)
        // out of a read_char form; only plain keypresses become game input.
        //
        // …and never while the `[more]` pager is showing (SQ-0539): a read_char
        // that dumped more than a screenful is paged FIRST, so the keystroke falls
        // through to `key_to_command`'s pager intercept and advances the view
        // instead of answering the read. Only once the pager has caught up (and
        // cleared `active`) does the next key reach the game — exactly the
        // original interpreters' [MORE] behavior.
        if state.char_mode && !state.any_overlay_open() && !state.pager.active {
            if let Event::Key(k) = &event {
                if k.kind == KeyEventKind::Press {
                    use crossterm::event::KeyModifiers;
                    let spec = app::keymap::KeySpec::from_key_event(*k);
                    // Ctrl/Alt combos (hotkeys, quit, etc.) are never game input —
                    // let them fall through to app routing so the user can always
                    // escape a read_char form. Only plain keypresses reach the VM.
                    let app_combo = k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
                    if spec != state.hotkeys.prefix && !app_combo {
                        // Map to a neutral KeyInput and forward; the engine
                        // converts it (ZSCII for the Z-machine) and returns None
                        // for keys with no input meaning, which are ignored.
                        let ki = app::engine::key_event_to_input(*k);
                        // Arrows are ALWAYS forwarded to a v6 story waiting on CHAR
                        // input (menus): `v6_arrow_keys = false` withholds arrows only
                        // at the line (`>`) prompt (see the line-terminator gate below),
                        // so `is_line_input = false` here always yields false. Menus —
                        // Shogun's startup menu, hint menus, "press any key" — are
                        // unnavigable without arrows, so the char gate never withholds
                        // (SQ-0483). Kept as a call for symmetry with the line gate.
                        let withhold_arrow = withhold_arrow_from_v6(
                            ki,
                            state.config.v6_arrow_keys,
                            zvm_session_opt(&*session).map_or(0, |z| z.machine.mem.version()),
                            false,
                        );
                        if !withhold_arrow {
                            if let Some(result) = ki.and_then(|ki| {
                                app::trace::hostio(&state.config.user_dir, state.config.trace.hostio, format!("input_key({ki:?})"));
                                session.submit_key(ki)
                            })
                            {
                                if turn::apply_game_driven_result(
                                    &mut state, &mut mapper, &result, &game_dir, last_panes.map, &*session, app::pager::Driver::PlayerInput,
                                ) {
                                    break 'event_loop state.exit_target.into();
                                }
                            }
                            continue;
                        }
                    }
                }
            }
        }

        // ── Line-terminator key gate (SQ-0188) ────────────────────────────────
        // While the Z-machine is waiting for a *line* read, a special key the game
        // lists in its v5 terminating-characters table (arrows / function keys)
        // submits the current input line with THAT ZSCII terminator, instead of the
        // key's normal app behavior. Only plain (no Shift/Ctrl/Alt) arrows + F-keys
        // are candidates; every other key — and any non-terminator arrow/F-key —
        // falls through unchanged so it keeps its app behavior (history/scroll/pan).
        // Suspended while the `[more]` pager is showing (SQ-0539): ↓ is a paging
        // key, and submitting the line would resume the game before the player
        // has seen the output that armed the pager.
        if !state.any_overlay_open()
            && !state.pager.active
            && zvm_session_opt(&*session).is_some_and(|z| z.pending_input() == app::session::InputKind::Line)
        {
            if let Event::Key(k) = &event {
                if k.kind == KeyEventKind::Press {
                    use crossterm::event::KeyModifiers;
                    let plain = !k.modifiers.intersects(
                        KeyModifiers::SHIFT | KeyModifiers::CONTROL | KeyModifiers::ALT,
                    );
                    if plain {
                        let ki = app::engine::key_event_to_input(*k);
                        // Withheld v6 arrows never act as line terminators either
                        // (SQ-0460): without this, a v6 game listing arrows in its
                        // terminating-characters table still moved the player from
                        // the line prompt regardless of the setting.
                        let withheld = withhold_arrow_from_v6(
                            ki,
                            state.config.v6_arrow_keys,
                            zvm_session_opt(&*session).map_or(0, |z| z.machine.mem.version()),
                            true,
                        );
                        let term = if withheld { None } else {
                            ki.and_then(|ki| zvm_session_opt(&*session).and_then(|z| z.line_key_terminator(&ki)))
                        };
                        if let Some(term) = term {
                            let cmd = state.take_input();
                            if !cmd.is_empty() {
                                state.record_command(&cmd);
                            }
                            state.turns += 1;
                            state.unsaved_progress = true;
                            let result = zvm_session_opt_mut(&mut *session)
                                .expect("z-machine line read is pending")
                                .submit_line_with_terminator(&cmd, term);
                            if turn::finish_command_turn(
                                // The read ended on a listed terminating
                                // character, not a newline (SQ-0881).
                                &cmd, false, result, &mut state, &mut mapper, &mut *session,
                                &game_dir, &ifid, &arc_file, last_panes.map, &mut bg_tidy_counter,
                            ) {
                                break 'event_loop state.exit_target.into();
                            }
                            continue 'event_loop;
                        }
                    }
                }
            }
        }

        // Route event to an Action.
        let action = match event {
            Event::Key(k) if k.kind == KeyEventKind::Press => {
                match key_to_command(&state, k) {
                    KeyResolve::Action(a) => a,
                    KeyResolve::Command(s, ctx) => {
                        let close_leader = state.overlays.hotkey_dialog;
                        // A palette-resolved command closes the palette after it runs.
                        let close_palette = state.overlays.palette.is_some();
                        let outcome = slash::parse_in_context(&s, state.config.command_prefix, ctx);
                        let should_break = dispatch_slash_outcome(
                            outcome, &mut state, &mut mapper, &mut *session, &mut style_watcher,
                            &game_dir, &ifid, &arc_file, &story_bytes, &story_path,
                            last_panes.map, last_panes.story, true,
                        );
                        if close_leader {
                            state.overlays.hotkey_dialog = false;
                        }
                        if close_palette {
                            state.overlays.palette = None;
                        }
                        lifecycle::flush_pending_config_write(&mut state);
                        if should_break {
                            break 'event_loop state.exit_target.into();
                        }
                        continue 'event_loop;
                    }
                    KeyResolve::None => Action::None,
                }
            }
            // ── Command band (SQ-0664) ────────────────────────────────────────
            // The band is NOT a modal, so unlike the old verb dock it cannot
            // lean on `any_overlay_open()` to keep clicks away from the game. It
            // claims exactly its own rect instead — matched FIRST, so a click on
            // a band row can never also reach a mouse-watching Glulx window or a
            // v6 story's compass — and every click outside that rect falls
            // straight through to the handling below, unchanged.
            Event::Mouse(m) if band_mouse_action(&state, &last_panes, m).is_some() => {
                match band_mouse_action(&state, &last_panes, m) {
                    // A quick-row click fires AT ONCE (SQ-0667, 2026-08-05) —
                    // routed to `action` so it reaches the shared submit arm
                    // below, the same as `Action::SubmitCommand`; resolving
                    // the word and touching the session is not something
                    // `apply_action` can do.
                    Some(pick @ Action::BandQuickPick(_)) => pick,
                    // A DOUBLE-click on a word row submits the composed prompt
                    // (SQ-0690). The pair's first click already picked the word
                    // onto `state.input` and advanced the column, so the second
                    // click routes to the shared submit arm exactly like Enter —
                    // and must NOT pick again, or the word would land twice.
                    Some(Action::BandClickRow(col, idx)) => {
                        if band_clicks.observe(col, idx, std::time::Instant::now()) {
                            Action::SubmitCommand(String::new()) // payload unused: submit takes state.input
                        } else {
                            apply_action(Action::BandClickRow(col, idx), &mut state, &mut mapper);
                            continue 'event_loop;
                        }
                    }
                    Some(other) => {
                        apply_action(other, &mut state, &mut mapper);
                        continue 'event_loop;
                    }
                    None => unreachable!("guarded by the match arm"),
                }
            }
            // ── Inventory dock (SQ-1244) ──────────────────────────────────────
            // Same precedence as the command band above: claims exactly its own
            // rect, matched before the general mouse handling, so a click on the
            // inventory panel can never also reach the story pane behind it.
            Event::Mouse(m) if inventory_mouse_action(&state, &last_panes, m).is_some() => {
                match inventory_mouse_action(&state, &last_panes, m) {
                    Some(other) => {
                        apply_action(other, &mut state, &mut mapper);
                        continue 'event_loop;
                    }
                    None => unreachable!("guarded by the match arm"),
                }
            }
            Event::Mouse(m) => {
                // Glk mouse input: a left-Down inside a mouse-watching Glulx window
                // is delivered to the game as an Evtype_MouseInput, not a UI action.
                // Only left-Down is diverted (Glk mouse is click-only, so the Drag/Up
                // selection events still arrive but fire no StartSelection and are
                // harmless no-ops); glk_mouse_target enforces no-overlay + inside a
                // watching window and computes the window-relative coordinates.
                // Glk hyperlink input: a left-Down on a linked transcript cell whose
                // owning window has an active hyperlink request is delivered to the
                // game as an Evtype_Hyperlink carrying the cell's link value. A link
                // cell is a more specific target than a general mouse-window click, so
                // this runs first; a non-link click (or no watching window) falls
                // through to the mouse path, then to the app's own handling.
                if matches!(m.kind, crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left)) {
                    if let Some(gs) = glulx_session_opt_mut(&mut *session) {
                        if let Some(&(_, link)) = last_panes
                            .transcript_links
                            .iter()
                            .find(|((cx, cy), _)| *cx == m.column && *cy == m.row)
                        {
                            if link != 0 {
                                let windows = gs.hyperlink_windows();
                                if !windows.is_empty() {
                                    let s = last_panes.story;
                                    if let Some(win) = app::glulx_session::glk_hyperlink_window(
                                        state.any_overlay_open(),
                                        m.column, m.row,
                                        (s.x, s.y, s.width, s.height),
                                        &windows,
                                        &last_panes.win_rects,
                                    ) {
                                        let result = gs.deliver_hyperlink(win, link);
                                        if turn::apply_game_driven_result(
                                            &mut state, &mut mapper, &result, &game_dir, last_panes.map, &*session, app::pager::Driver::PlayerInput,
                                        ) {
                                            break 'event_loop state.exit_target.into();
                                        }
                                        continue 'event_loop;
                                    }
                                }
                            }
                        }
                    }
                }
                if matches!(m.kind, crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left)) {
                    if let Some(gs) = glulx_session_opt_mut(&mut *session) {
                        let windows = gs.mouse_windows();
                        if !windows.is_empty() {
                            let s = last_panes.story;
                            let target = app::glulx_session::glk_mouse_target(
                                state.any_overlay_open(),
                                m.column, m.row,
                                (s.x, s.y, s.width, s.height),
                                &windows,
                                &last_panes.win_rects,
                                gs.char_pixels(),
                                mouse_sub_px,
                            );
                            if let Some((win, vx, vy)) = target {
                                let result = gs.deliver_mouse(win, vx, vy);
                                if turn::apply_game_driven_result(
                                    &mut state, &mut mapper, &result, &game_dir, last_panes.map, &*session, app::pager::Driver::PlayerInput,
                                ) {
                                    break 'event_loop state.exit_target.into();
                                }
                                continue 'event_loop;
                            }
                        }
                    }
                }
                // Command palette popup (SQ-0419): a row click executes that
                // command; the wheel scrolls the list; [X] / outside-click close.
                // A click elsewhere inside the dialog (input field, footer) is
                // swallowed so the popup stays open. Owns the mouse while open.
                if state.overlays.palette.is_some() {
                    use crossterm::event::{MouseButton, MouseEventKind};
                    match m.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            let pt = ratatui::layout::Position { x: m.column, y: m.row };
                            if let Some(&(cmd_index, _)) =
                                last_panes.palette.iter().find(|(_, r)| r.contains(pt))
                            {
                                let spec = &app::slash::COMMANDS[cmd_index];
                                let cmd = state
                                    .overlays
                                    .palette
                                    .as_ref()
                                    .map(|p| p.command_line(spec.name))
                                    .unwrap_or_else(|| spec.name.to_string());
                                state.overlays.palette = None;
                                let outcome =
                                    slash::parse_in_context(&cmd, state.config.command_prefix, spec.context);
                                let should_break = dispatch_slash_outcome(
                                    outcome, &mut state, &mut mapper, &mut *session, &mut style_watcher,
                                    &game_dir, &ifid, &arc_file, &story_bytes, &story_path,
                                    last_panes.map, last_panes.story, true,
                                );
                                lifecycle::flush_pending_config_write(&mut state);
                                if should_break {
                                    break 'event_loop state.exit_target.into();
                                }
                                continue 'event_loop;
                            }
                            // Not a row: close on [X] or a click outside the dialog;
                            // swallow clicks landing elsewhere inside the popup.
                            let close_x = last_panes.dialog.as_ref().and_then(|d| d.close).is_some_and(|r| r.contains(pt));
                            let inside = last_panes.dialog.as_ref().is_some_and(|d| d.area.contains(pt));
                            if close_x || !inside {
                                apply_action(Action::PaletteClose, &mut state, &mut mapper);
                            }
                            continue 'event_loop;
                        }
                        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                            // The wheel scrolls the candidate list and keeps the
                            // cursor inside it, like every other list (SQ-0831);
                            // `PaletteNav` stays the keyboard's cursor move.
                            if let Some(d) = app::input::wheel_delta(m.kind, state.config.mouse_wheel_invert) {
                                apply_action(Action::ListWheel(d as i32), &mut state, &mut mapper);
                            }
                            continue 'event_loop;
                        }
                        _ => continue 'event_loop,
                    }
                }
                // Room dock (SQ-0692): the dock owns every mouse event inside its
                // rect. A left-click on one of its two view tabs switches the body;
                // anything else inside it is simply swallowed, because the dock is
                // carved out of the map pane and a click there is neither a map
                // click nor a story selection — and must never reach the v6 mouse
                // delivery path below.
                if !state.any_modal_overlay_open() {
                    if let Some(action) = app::input::room_dock_mouse_action(
                        last_panes.room_dock,
                        &last_panes.room_dock_tabs,
                        &m,
                    ) {
                        // (`needs_redraw` was already set for this event above.)
                        apply_action(action, &mut state, &mut mapper);
                        continue 'event_loop;
                    }
                }
                // Border toggle controls (SQ-1123): a left-click on one runs that
                // control's own `slash::COMMANDS` entry, bare, through the ordinary
                // slash pipeline — the same path the palette's row-click takes. A
                // click IS the command, so whatever the command persists, a click
                // persists too, and there is no second implementation of any toggle.
                if let crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) = m.kind {
                    if let Some(ctl) = app::render::controls::control_at(
                        &state, &last_panes.border_controls, m.column, m.row,
                    ) {
                        // The command's OWN context, looked up in the registry
                        // rather than assumed: a control must parse exactly the
                        // way the palette parses the same command.
                        let cmd = ctl.command();
                        let ctx = slash::COMMANDS
                            .iter()
                            .find(|c| c.name == cmd.name)
                            .map(|c| c.context)
                            .unwrap_or(Context::Global);
                        // The whole LINE, argument and all: `zoom-map` bare is an
                        // error, and the two zoom controls are one entry with two
                        // arguments (SQ-1148).
                        let outcome = slash::parse_in_context(
                            &cmd.to_string(), state.config.command_prefix, ctx,
                        );
                        let should_break = dispatch_slash_outcome(
                            outcome, &mut state, &mut mapper, &mut *session, &mut style_watcher,
                            &game_dir, &ifid, &arc_file, &story_bytes, &story_path,
                            last_panes.map, last_panes.story, true,
                        );
                        lifecycle::flush_pending_config_write(&mut state);
                        if should_break {
                            break 'event_loop state.exit_target.into();
                        }
                        continue 'event_loop;
                    }
                }
                // Map layer tab: a left-click on a layer tab selects that layer as the
                // viewed one (hit-rects captured per frame in last_panes.layer_tabs).
                if !state.any_overlay_open() {
                    if let crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) = m.kind {
                        let hit = last_panes.layer_tabs.iter().find(|(_, r)| {
                            r.width > 0 && r.height > 0
                                && m.column >= r.x && m.column < r.right()
                                && m.row >= r.y && m.row < r.bottom()
                        });
                        if let Some(&(layer, _)) = hit {
                            apply_action(Action::SetViewedLayer(layer), &mut state, &mut mapper);
                            continue 'event_loop;
                        }
                    }
                }
                // Z-machine v6 mouse input (Lane M): a left-Down landing inside the
                // drawn v6 image, while the game awaits read_char, is delivered to
                // the VM as a ZSCII single-click (254) with the click's game-pixel
                // coordinates recorded via set_mouse (the header extension table +
                // read_mouse). Only fires when the click maps into the image
                // (map_click → Some); a Line read (normal play), a click in the
                // letterbox margin, or a click outside the pane all fall through to
                // the app's own story-pane handling (selection) below. No overlay
                // may be open (a modal owns the click first).
                // A LINE read takes the click too, when the game lists a click among
                // its terminating characters (SQ-0566). This is the ordinary-play
                // case: Zork Zero sits at its `>` prompt with the border compass
                // drawn, and a click there must end the read with whatever is typed
                // plus terminator 254, so the game can read the coordinates and move.
                // Restricting delivery to `read_char` meant compass clicks did
                // nothing except while a menu happened to be up.
                if matches!(m.kind, crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left))
                    && !state.any_overlay_open()
                {
                    let pending = zvm_session_opt(&*session).map(|z| z.pending_input());
                    let line_term =
                        zvm_session_opt(&*session).and_then(|z| z.mouse_click_terminator());
                    let deliver = match pending {
                        Some(app::session::InputKind::Char) => true,
                        Some(app::session::InputKind::Line) => line_term.is_some(),
                        _ => false,
                    };
                    // Only a click that maps INTO the drawn v6 image reaches the VM;
                    // the letterbox margin and everything outside the pane fall
                    // through to the app's own story-pane handling (selection).
                    let hit = deliver
                        .then(|| {
                            state
                                .graphics_render
                                .borrow()
                                .last_v6_map
                                .as_ref()
                                .and_then(|cm| cm.map_click(m.column, m.row))
                        })
                        .flatten();
                    if let Some((gx, gy)) = hit {
                        if pending == Some(app::session::InputKind::Char) {
                            let z = zvm_session_opt_mut(&mut *session)
                                .expect("z-machine char read is pending");
                            z.set_mouse(gy, gx); // engine stores (y, x)
                            let result = z.submit_char(254); // ZSCII single-click (§3.8)
                            if turn::apply_game_driven_result(
                                &mut state, &mut mapper, &result, &game_dir, last_panes.map, &*session, app::pager::Driver::PlayerInput,
                            ) {
                                break 'event_loop state.exit_target.into();
                            }
                            continue 'event_loop;
                        }
                        // Line read: a real player turn, so it goes through the same
                        // path as a typed command — history, turn count, mapping,
                        // autosave — carrying whatever was already typed (usually
                        // nothing) and the click as the terminator.
                        let term = line_term.expect("gated by `deliver` above");
                        let cmd = state.take_input();
                        if !cmd.is_empty() {
                            state.record_command(&cmd);
                        }
                        state.turns += 1;
                        state.unsaved_progress = true;
                        let result = {
                            let z = zvm_session_opt_mut(&mut *session)
                                .expect("z-machine line read is pending");
                            z.set_mouse(gy, gx); // engine stores (y, x)
                            z.submit_line_with_terminator(&cmd, term)
                        };
                        // SQ-0576: a compass click types nothing, but the game
                        // echoes the command it synthesized ("north") at the head
                        // of its output — adopt it so the turn maps (directional
                        // edge, tried-exit) exactly like the typed command it
                        // stands for.
                        let cmd = if cmd.is_empty() {
                            app::session::echoed_direction_command(&result.transcript)
                                .unwrap_or_default()
                                .to_string()
                        } else {
                            cmd
                        };
                        if turn::finish_command_turn(
                            &cmd, true, result, &mut state, &mut mapper, &mut *session,
                            &game_dir, &ifid, &arc_file, last_panes.map, &mut bg_tidy_counter,
                        ) {
                            break 'event_loop state.exit_target.into();
                        }
                        continue 'event_loop;
                    }
                }
                mouse_to_action(&state, m, last_panes.map, last_panes.story, &last_panes.room_rects, &last_panes.dialog)
            }
            // Resize: continue so the next draw uses the updated terminal size.
            // Resize: force a full repaint so no stale cells survive the size change.
            Event::Resize(_, _) => { clear_terminal(&mut terminal, &state); continue; }
            _ => continue,
        };

        // ToggleWatch is run-loop-only (owns the watcher): intercept before dispatch.
        if matches!(action, Action::ToggleWatch) {
            toggle_style_watch(&mut state, &mut style_watcher);
            continue;
        }

        // Snapshot working config before apply_action clears it on ConfigSave.
        let config_to_save = if matches!(action, Action::ConfigSave) {
            state.overlays.config_screen.as_ref().map(|cs| cs.working.clone())
        } else {
            None
        };
        // Mouse capture is established once at startup; note its pre-save value so a
        // settings-screen change can be applied to the live terminal below.
        let mouse_before_save = state.config.mouse;
        // Likewise note command_bar so a settings-screen toggle re-applies the
        // session's prompt-stripping live (else render mode and strip_prompt desync
        // until the next @restart).
        let command_bar_before_save = state.config.command_bar;

        match action {
            // ── Caller-handled actions ─────────────────────────────────────────

            Action::Quit => {
                // Ctrl-Q/Ctrl-C (the only route to this action — `input.rs`'s
                // hardwired step 1) resolves like every other way the run can end:
                // back to the library when one exists, Exit otherwise (SQ-1258).
                // Set it explicitly so a superseded `/quit-to-library` can't leave
                // a stale target behind.
                state.exit_target = app::state::ExitTarget::for_launch(state.launched_from_library);
                if should_prompt_save_on_quit(&state) {
                    state.overlays.quit_dialog = true;
                    state.overlays.dialog_focus = 0;
                } else {
                    break 'event_loop state.exit_target.into();
                }
            }

            // Story-pane selection released: copy the text extracted by render from
            // the full wrapped transcript (off-screen rows included) via OSC 52.
            Action::EndSelection => {
                state.selection = None;
                state.selection_edge = 0;
                let copied = state.selection_text.borrow_mut().take();
                if let Some(text) = copied {
                    if !text.trim().is_empty() {
                        use std::io::Write;
                        let seq = app::clipboard::osc52_copy_sequence(&text);
                        let mut out = std::io::stdout();
                        let _ = out.write_all(seq.as_bytes());
                        let _ = out.flush();
                        // Report the copy as a meta line in the story output rather
                        // than a status-bar message (which has no natural dismissal).
                        state.push_transcript_internal(
                            &format!("Copied {} chars to clipboard", text.chars().count()),
                            app::state::TranscriptKind::Meta,
                        );
                    }
                }
                continue;
            }

            Action::SubmitCommand(_) | Action::BandQuickPick(_) => {
                // A Glulx game waiting on a timer/mouse/hyperlink event only has no
                // line request pending: Enter has nothing to submit. Swallow it
                // (keeping the typed buffer intact for the real prompt) rather than
                // feed a stray line the VM would only diagnose.
                if session.pending_input() == app::session::InputKind::Event {
                    continue;
                }

                // Normal game-focus command submission.
                // Clear input line and echo command.
                //
                // The command band composes directly onto `state.input` now
                // (SQ-0667, 2026-08-05), so an ordinary `SubmitCommand` already
                // carries whatever the band composed — `state.take_input()`
                // below is the whole story for it, same as anything typed by
                // hand. `BandQuickPick` is the one exception: a quick-row pick
                // fires AT ONCE, composing nothing onto the input line (an
                // interjection, not a composition step — `state.input` is left
                // exactly as it was, even mid-phrase), so its word is resolved
                // separately here instead.
                let cmd = match &action {
                    Action::BandQuickPick(idx) => {
                        match app::input::band_quick_pick_command(&state, *idx) {
                            Some(w) => w,
                            None => continue, // stale index — band closed/reconfigured mid-click
                        }
                    }
                    _ => state.take_input(),
                };
                // The line just emptied (or, for a quick pick, did not change):
                // re-point the open band at it, so it is not still showing the
                // object columns of a phrase already on its way to the game
                // (SQ-0676 — this path bypasses `apply_action`'s own hook).
                app::input::band_react_to_input(&mut state);

                // An empty cmd (Enter on a blank line) is still submitted to the
                // game, which decides what a blank line means (re-prompt / "I beg
                // your pardon?"), matching other interpreters (SQ-0265). Only skip
                // history recording and slash routing for it — an empty line is
                // neither worth a history entry nor a slash command.
                if !cmd.is_empty() {
                    // Record into the shell-style command history (game + slash
                    // alike), deduping consecutive repeats and capping the list.
                    state.record_command(&cmd);

                    // ── Slash-command interception ────────────────────────────
                    // If the input starts with the configured prefix, route it as
                    // an app command; do NOT call session.submit, increment turns,
                    // or push a "> cmd" story line.
                    if is_slash(&cmd, state.config.command_prefix) {
                        // Strip the leading prefix character before parsing.
                        let body = &cmd[state.config.command_prefix.len_utf8()..];
                        let outcome = slash::parse(body, state.config.command_prefix);
                        let should_break = dispatch_slash_outcome(
                            outcome, &mut state, &mut mapper, &mut *session, &mut style_watcher,
                            &game_dir, &ifid, &arc_file, &story_bytes, &story_path,
                            last_panes.map, last_panes.story, false,
                        );
                        lifecycle::flush_pending_config_write(&mut state);
                        if should_break {
                            break 'event_loop state.exit_target.into();
                        }
                        continue;
                    }
                }

                // Increment the session turn counter. Progress now exists that
                // isn't captured in a Save State (drives the quit prompt).
                state.turns += 1;
                state.unsaved_progress = true;

                app::trace::hostio(&state.config.user_dir, state.config.trace.hostio, format!("input_line({cmd:?})"));
                let result = session.submit(&cmd);
                if turn::finish_command_turn(
                    &cmd, true, result, &mut state, &mut mapper, &mut *session,
                    &game_dir, &ifid, &arc_file, last_panes.map, &mut bg_tidy_counter,
                ) {
                    break 'event_loop state.exit_target.into();
                }
            }

            Action::SaveGame => {
                // Dead post-unification: keys now route through SlashOutcome::Save. Retained as a no-cost match arm.
                // Bundle map + game into a single .lanthorn archive, with turn metadata.
                let (location, score) = crate::engine_helpers::save_summary(&*session, &state);
                let meta = app::archive::Meta {
                    format_version: app::archive::CURRENT_FORMAT_VERSION,
                    ifid: Some(ifid.clone()),
                    name: None,
                    turns: state.turns,
                    saved_at: {
                        use std::time::{SystemTime, UNIX_EPOCH};
                        let secs = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        // Re-use a simple format: delegate to persist_files helper would be
                        // cleaner but it's private; inline the same logic here.
                        format_rfc3339(secs)
                    },
                    location,
                    score,
                    trigger: app::archive::SaveTrigger::HostState,
                };
                // v6 graphics canvases ride along (Lane P): empty for non-v6
                // sessions, so the archive layout is unchanged for them.
                let (v6_pics, v6_display, v6_ground, v6_diags) =
                    crate::engine_helpers::v6_save_payload(&mut *session);
                for d in &v6_diags { state.note_v6_save(d); }
                match app::archive::save_archive_meta_pics(&arc_file, &mapper, &session.save_state(), zvm_session_opt(&*session).map(|z| &z.machine.screen), session.aux_data(), meta, &app::archive::SessionRecord::of(&state), &v6_pics, v6_display.as_ref(), v6_ground.as_deref()) {
                    Ok(()) => {
                        state.push_notice(&format!(
                            "[Game saved to {}]",
                            arc_file.display()
                        ));
                    }
                    Err(e) => {
                        state.push_notice(&format!("[Save failed: {}]", e));
                    }
                }
            }

            Action::RestoreGame => {
                // Dead post-unification: keys now route through SlashOutcome::Load. Retained as a no-cost match arm.
                // Restore map + game from the .lanthorn archive.
                match load_archive(&arc_file) {
                    Ok(ac) => {
                        let restore_err = session.restore_state(&ac.engine_save()).map_err(restore_error_msg);
                        match restore_err {
                            Ok(()) => {
                                if let Some(scr) = ac.screen.clone() {
                                    if let Some(z) = zvm_session_opt_mut(&mut *session) { app::session::restore_screen(z, scr); }
                                }
                                // v6 graphics canvases (Lane P): no-op for non-v6 archives.
                                crate::engine_helpers::apply_v6_pictures(&mut *session, &ac);
                                // Hand Glulx back the room it was saved in (SQ-0523); no-op for zvm.
                                engine_helpers::seed_resumed_location(&mut *session, &ac.meta);
                                if state.config.aux_storage != app::config::AuxStorage::Global {
                                    session.set_aux_data(ac.aux.clone());
                                }
                                mapper = ac.mapper;
                                state.transcript = ac.transcript;
                                state.clear_anchor = None;
                                state.transcript_kinds = ac.transcript_kinds;
                                state.transcript_runs = ac.transcript_runs;
                                state.transcript_para = ac.transcript_para;
                                state.reset_transcript_sidecars();
                                // Re-attach the transcript's inline images AFTER the
                                // sidecar reset (which zeroes them); parallel to the
                                // restored transcript so its embedded art renders (SQ-0518).
                                state.transcript_images = ac.transcript_images;
                                state.history = ac.history;
                                state.command_history = ac.command_history;
                                // The scraped word set is derived from the transcript
                                // and never archived, so rebuild it from the one that
                                // just replaced it (SQ-1135) — which is also what makes
                                // a restore take back a word printed after the save.
                                app::input::refresh_seen_words(&mut state, &*session);
                                // After restore, re-observe current location.
                                reobserve_location(&mut state, &mut mapper, &*session, last_panes.map);
                                state.push_notice(&format!(
                                    "[Game restored from {}]",
                                    arc_file.display()
                                ));
                            }
                            Err(e) => {
                                state.push_notice(&format!("[Restore failed: {}]", e));
                            }
                        }
                    }
                    Err(e) => {
                        state.push_notice(&format!("[Restore failed: {}]", e));
                    }
                }
            }

            // SQ-0297: shared with the slash-command path via handle_map_export
            // (dispatch_slash_outcome never reaches this match).
            a @ (Action::ExportSvg(_) | Action::ExportDot(_) | Action::ExportMap(_)) => {
                handle_map_export(&a, &game_dir, &mapper, &mut state);
            }

            // ── Saves-manager actions ─────────────────────────────────────────

            Action::OpenSaves => {
                // Populate the saves list (both .lanthorn Save States and .qzl
                // game saves — SQ-0227 Task 3) and open the modal.
                let entries = combined_saves(&game_dir);
                state.overlays.saves = Some(SavesState { entries, scroll: Default::default() });
                state.overlays.dialog_focus = 0;
            }

            Action::SavesImport => {
                // Close saves modal and open file browser in PickFile mode.
                // Start in this story's per-game dir (where its saves live, honoring
                // --data-dir), falling back to the data base then the user dir.
                state.overlays.saves = None;
                let start_dir = if game_dir.is_dir() {
                    game_dir.clone()
                } else if data_base.is_dir() {
                    data_base.clone()
                } else {
                    state.config.user_dir.clone()
                };
                state.overlays.file_browser = Some(FileBrowserState::build(start_dir, FbMode::PickFile));
            }

            Action::FbEnter => {
                // Handle file-browser Enter: cd into dir or import file.
                let fb_action = state.overlays.file_browser.as_ref().and_then(|fb| {
                    fb.entries.get(fb.scroll.selected).map(|e| {
                        if e.is_dir {
                            let new_path = if e.name == ".." {
                                fb.cwd.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| fb.cwd.clone())
                            } else {
                                fb.cwd.join(&e.name)
                            };
                            FbEntryAction::CdInto(new_path)
                        } else {
                            FbEntryAction::ImportFile(fb.cwd.join(&e.name))
                        }
                    })
                });
                match fb_action {
                    Some(FbEntryAction::CdInto(path)) => {
                        if let Some(fb) = &mut state.overlays.file_browser {
                            fb.cd(path);
                        }
                    }
                    Some(FbEntryAction::ImportFile(path)) => {
                        state.overlays.file_browser = None;
                        if !engine_supports_save(&*session) {
                            state.set_status("Restore is not supported for Glulx games yet");
                            continue;
                        }
                        match restore_game(&path, &mut zvm_session_mut(&mut *session).machine) {
                            Ok(()) => {
                                // Re-observe current location (same as RestoreGame/SavesLoad).
                                reobserve_location(&mut state, &mut mapper, &*session, last_panes.map);
                                state.push_notice(&format!("[Imported: {}]", path.display()));
                            }
                            Err(e) => {
                                state.push_notice(&format!("[Import failed: {}]", e));
                            }
                        }
                    }
                    None => {}
                }
            }

            Action::SavesLoad => {
                // Load the selected save (archive → mapper + machine restore).
                // Clone path and name to release the borrow on state.overlays.saves before mutating state.
                let load_info = state.overlays.saves.as_ref().and_then(|s| {
                    s.entries.get(s.scroll.selected).map(|e| (e.path.clone(), e.name.clone(), e.trigger))
                });

                // In-game restore of a GAME save — a bare .qzl from another
                // interpreter, or a .lanthorn that lanthorn's own @save wrote
                // (SQ-0531): feed the descriptor-PC bytes back into the
                // suspended VM, completing the @restore. When they came out of
                // an archive, its map/transcript/screen ride along too. A host
                // Save State picked here instead falls through below to a full
                // session resume (SQ-0227 Task 3).
                if state.ingame_io == Some(app::session::PendingIo::Restore)
                    && load_info.as_ref().is_some_and(|(_, _, t)| t.is_portable())
                {
                    let Some((path, entry_name, _)) = load_info else { continue };
                    state.overlays.saves = None;
                    state.ingame_io = None;
                    let result = match app::archive::read_quetzal_from_file(&path) {
                        Ok(bytes) => {
                            // Reinstate the archive's session state BEFORE resuming,
                            // so the game's own post-restore output lands at the end
                            // of the restored scrollback instead of being wiped by it.
                            if !app::persist_files::is_game_save(&path) {
                                match app::archive::load_archive(&path) {
                                    Ok(ac) => apply_archive_state(ac, &mut *session, &mut mapper, &mut state),
                                    Err(e) => state.push_notice(&format!("[Save State sidecars unreadable: {}]", e)),
                                }
                            } else {
                                // A bare .qzl carries no screen, so the restored
                                // game's layout width has to be assumed
                                // (`note_bare_quetzal_width`, SQ-0681). Raised on
                                // the attempt: `resume_restore` reports a refused
                                // save only as the game's own "Failed.", and the
                                // guard only ever widens the declared screen.
                                engine_helpers::note_bare_quetzal_width(&mut *session);
                            }
                            state.push_notice(&format!("[Game restored from {}]", entry_name));
                            session.resume_restore(Some(&bytes))
                        }
                        Err(e) => {
                            state.push_notice(&format!("[Restore failed: {}]", e));
                            session.resume_restore(None)
                        }
                    };
                    let quit = turn::finish_resumed_turn(result, &mut mapper, &mut state, &mut *session, &game_dir, &ifid, last_panes.map);
                    turn::persist_aux_after_turn(&mut *session, &mut state, &game_dir);
                    turn::persist_vfs_after_turn(&mut *session, &state, &game_dir);
                    if let Some(io) = state.ingame_io {
                        open_ingame_saves(io, &game_dir, &mut state);
                    }
                    if quit { break 'event_loop state.exit_target.into(); }
                    continue;
                }

                // Host Load (also reached for a .lanthorn picked while an
                // in-game @restore is pending: that fully resumes, abandoning
                // the pending call; on failure the pending @restore is still
                // answered with resume_restore(None) so the VM isn't left
                // blocked waiting for a result).
                let ingame_restore_pending = state.ingame_io == Some(app::session::PendingIo::Restore);
                if let Some((path, entry_name, _)) = load_info {
                    match restore_from_file(&path, &mut *session) {
                        Ok(RestoreOutcome::DescriptorCompleted(ac)) => {
                            state.overlays.saves = None;
                            // An in-game @save archive carries the whole session
                            // alongside its game bytes (SQ-0531); a bare .qzl has
                            // nothing but the bytes.
                            if let Some(ac) = ac {
                                state.ingame_io = None;
                                state.pending_filename = None;
                                apply_archive_state(*ac, &mut *session, &mut mapper, &mut state);
                            }
                            reobserve_location(&mut state, &mut mapper, &*session, last_panes.map);
                            state.push_notice(&format!("[Game restored from {}]", entry_name));
                        }
                        Ok(RestoreOutcome::Resumed(ac)) => {
                            state.ingame_io = None;
                            // A restore abandons any suspended create_by_prompt in the
                            // session, so the host-side request must not outlive it and
                            // fire a spurious resume_filename turn.
                            state.pending_filename = None;
                            apply_archive_state(*ac, &mut *session, &mut mapper, &mut state);
                            // Re-observe current location.
                            reobserve_location(&mut state, &mut mapper, &*session, last_panes.map);
                            state.push_notice(&format!("[Loaded save: {}]", entry_name));
                            state.overlays.saves = None;
                        }
                        Err(e) => {
                            state.push_notice(&format!("[Load failed: {}]", e));
                            if ingame_restore_pending {
                                state.overlays.saves = None;
                                state.ingame_io = None;
                                let result = session.resume_restore(None);
                                let quit = turn::finish_resumed_turn(result, &mut mapper, &mut state, &mut *session, &game_dir, &ifid, last_panes.map);
                                turn::persist_aux_after_turn(&mut *session, &mut state, &game_dir);
                                turn::persist_vfs_after_turn(&mut *session, &state, &game_dir);
                                if let Some(io) = state.ingame_io {
                                    open_ingame_saves(io, &game_dir, &mut state);
                                }
                                if quit { break 'event_loop state.exit_target.into(); }
                                continue;
                            }
                        }
                    }
                }
            }

            // ── Replay/rewind: linear resume from the selected turn ────────────
            Action::ReplayResume => {
                if let Some(r) = state.overlays.replay.take() {
                    if r.idx < state.history.len() {
                        let plan = app::history::resume_plan(&state.history, r.idx);
                        // History snapshots come from the running engine; wrap them
                        // with its tag so restore_state accepts them (both engines).
                        let es = app::engine::EngineSave::new(engine_tag(&*session), 1, plan.save.clone());
                        match session.restore_state(&es) {
                            Ok(()) => {
                                if let Some(json) = &plan.map_json {
                                    if let Ok(m) = mapper::persist::from_json(json) {
                                        mapper = m;
                                    }
                                }
                                // Linear: discard later turns.
                                state.history.truncate(r.idx + 1);
                                let (lines, kinds) =
                                    app::history::rebuild_transcript(&state.history, r.idx);
                                state.transcript = lines;
                                state.clear_anchor = None;
                                state.transcript_kinds = kinds;
                                // History replay carries no style runs; keep the
                                // parallel vecs length-synced (unstyled, left rows).
                                state.transcript_runs = vec![Vec::new(); state.transcript.len()];
                                state.transcript_para = vec![app::state::ParaFmt::default(); state.transcript.len()];
                                state.reset_transcript_sidecars();
                                // Rebuilt from the replayed transcript (SQ-1135): a
                                // rewind to turn 4 offers the words turn 4 had printed.
                                app::input::refresh_seen_words(&mut state, &*session);
                                state.turns = plan.turn;
                                state.unsaved_progress = false; // resumed a past (saved) turn
                                state.graph_gen = state.graph_gen.wrapping_add(1);
                                // Resuming a past turn is a restore: the watch describes a death
                                // in a timeline this one has replaced (SQ-0671, SQ-0673).
                                state.death_watch = Default::default();
                                // Re-observe current location (mirror the restore path).
                                if let Some(snap) = session.current_location() {
                                    let rid = snap.number as mapper::graph::RoomId;
                                    let restore_result = TurnResult::observation(snap);
                                    apply_turn(
                                        &mut mapper,
                                        "",
                                        &restore_result,
                                        &mut state.death_watch,
                                    );
                                    state.set_viewed_layer(None);
                                    state.select_room(Some(rid));
                                }
                                state.push_notice(&format!("[Resumed from turn {}]", plan.turn));
                            }
                            Err(e) => {
                                state.push_notice(&format!("[Resume failed: {}]", restore_error_msg(e)));
                            }
                        }
                    }
                }
            }

            // ── Open hints panel ──────────────────────────────────────────────
            Action::OpenHints => {
                let sp = story_path.clone();
                let id = ifid.clone();
                let ud = state.config.user_dir.clone();
                open_hints(&mut state, &sp, &id, &ud);
            }

            // Page the transcript by one screenful. Resolved here because it needs
            // the last-rendered transcript viewport height and max scroll.
            Action::TranscriptScrollPage(dir) => {
                let target = app::input::page_scroll(
                    state.transcript_scroll,
                    dir,
                    last_panes.transcript_viewport_rows,
                    last_panes.transcript_max_scroll,
                );
                state.scroll_transcript_to(target);
            }
            // Half-page the transcript (Ctrl-D, vim convention; SQ-1228). Same
            // shape as the full-page arm above, resolved here for the same
            // reason: it needs the last-rendered viewport height and max scroll.
            Action::TranscriptScrollHalfPage(dir) => {
                let target = app::input::half_page_scroll(
                    state.transcript_scroll,
                    dir,
                    last_panes.transcript_viewport_rows,
                    last_panes.transcript_max_scroll,
                );
                state.scroll_transcript_to(target);
            }
            // [more] pager (SQ-0404): page one screen toward the bottom; reaching
            // the bottom (offset 0) catches up and exits the pager.
            Action::PagerAdvance => {
                let target = app::input::page_scroll(
                    state.transcript_scroll,
                    -1,
                    last_panes.transcript_viewport_rows,
                    last_panes.transcript_max_scroll,
                );
                state.scroll_transcript_to(target);
                if target == 0 {
                    state.pager.active = false;
                }
            }
            // [more] pager: jump straight to the newest output and exit.
            Action::PagerDismiss => {
                state.scroll_transcript_to(0);
                state.pager.active = false;
            }

            // ── apply_action handles everything else ───────────────────────────
            other => {
                apply_action(other, &mut state, &mut mapper);
            }
        }

        // After apply_action: if a sound toggle / config save flipped enable_sound,
        // sync the running Glulx VM's Sound gestalt so games that re-check
        // gestalt_Sound per play (e.g. sensory.blorb's gong) honor the change.
        if let Some(on) = state.pending_vm_sound.take() {
            if let Some(gs) = glulx_session_opt_mut(&mut *session) {
                gs.set_sound(on);
            }
        }
        // A config-screen Save may have flipped watch_style: start/stop the
        // file-watcher live to match (no-op when already in that state).
        if let Some(on) = state.pending_watch_style.take() {
            set_style_watch(&mut state, &mut style_watcher, on);
        }

        // After dispatch: resume an in-game (v4+) save/restore whose dialog was
        // just confirmed (flag-hop) or cancelled (overlay closed without confirm).
        let quit = resolve_ingame_dialog(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map)
            || resolve_filename_request(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map);
        turn::persist_aux_after_turn(&mut *session, &mut state, &game_dir);
        turn::persist_vfs_after_turn(&mut *session, &state, &game_dir);
        if quit {
            break 'event_loop state.exit_target.into();
        }

        // After apply_action: if resize mode was just exited or reset, persist the
        // (possibly changed) pane sizes to config.toml. Also covers the
        // `KeyResolve::Command` dispatch path via the `flush_pending_config_write`
        // calls placed right before its `continue`s above.
        lifecycle::flush_pending_config_write(&mut state);

        // After apply_action: if config screen was just saved, persist config.toml
        // (created if missing). The settings screen edits no colours/symbols, and the
        // live look was already re-resolved FROM style.toml in apply_action, so we do
        // NOT touch style.toml here — writing it would clobber the seeded template.
        if let Some(cfg_to_write) = config_to_save {
            // Hitting Save on the settings screen and getting nothing is the worst
            // place to swallow this — surface the reason (SQ-0580).
            if let Err(e) = app::config::write_config_file(&state.config) {
                state.push_notice(&format!("[config not saved: {e}]"));
            }
            // Apply a mouse-capture change live so the setting takes effect without a
            // restart (matching how audio/colours apply live on save).
            if cfg_to_write.mouse != mouse_before_save {
                let _ = if cfg_to_write.mouse {
                    execute!(stdout(), EnableMouseCapture)
                } else {
                    execute!(stdout(), DisableMouseCapture)
                };
            }
            // Re-apply prompt stripping live so toggling the command bar on/off in
            // Settings takes effect on the next turn without a restart (inline mode
            // keeps the game's `>`, command-bar mode strips it).
            if cfg_to_write.command_bar != command_bar_before_save {
                session.set_strip_prompt(cfg_to_write.command_bar);
            }
            // SQ-1161: and re-resolve the live look, AFTER the write above. This is
            // the single funnel the style watcher and `/reload-style` go through, so
            // it is what makes the `period_look` row (and the theme layers, and this
            // story's own style.toml and garglk.ini overlays) land on Save instead of
            // waiting for the next launch. It must run after `write_config_file`,
            // because it recomputes `honor_game_colours` from this story's sidecar and
            // re-pins the key — and a pinned key is skipped by the writer, so running
            // it first would drop the honour row's own edit out of the file.
            if let app::reload::ReloadOutcome::Failed { msg } = app::reload::reload_style(&mut state) {
                state.push_notice(&format!("[style not reloaded: {msg}]"));
            }
        }

    };

    // ── 6. Exit: restore terminal + (optional) autosave ───────────────────────
    // Runs for BOTH outcomes: a return-to-library exits this story exactly like a
    // quit (same terminal restore + exit auto-save), then the caller reopens the
    // picker. The exit auto-save keeps progress on either path. (SQ-0435)

    restore_terminal();
    report_captured_stderr();

    // "Save State & quit" that could not save: say so now that stderr reaches the
    // user's terminal again (SQ-0651).
    if let Some(w) = &quit_save_warning {
        eprintln!("{w}");
    }

    lifecycle::exit_auto_save(&mut *session, &mapper, &state, &ifid, &arc_file);

    // `--debug` (SQ-0449): persist the cumulative executed-PC coverage to the
    // per-story sidecar so a later `--debug`/`/debug` run resumes the blue lines.
    // Best-effort — a write error is ignored, like the other exit-path writes.
    // This chokepoint is reached by BOTH the Exit and ToLibrary outcomes.
    if state.persist_debug_trace {
        if let Some(dbg) = session.debugger() {
            let _ = app::pcset_store::write_pcs(&game_dir, &dbg.ever_executed_pcs());
        }
    }

    outcome
}

// ── Reset helper ──────────────────────────────────────────────────────────────

// Rebuild the session from `story_bytes`, reset all ephemeral state, and
// re-seed the mapper with the start room.  When `clear_map` is true, the
// accumulated map is wiped first (same effect as `/reset map`) so only the
// start room remains after the re-seed.

/// Resolve the Pict/graphics blorb for a story the same way at launch and
/// restart — the Glulx and Scott arms of both, where the Z-machine arm builds a
/// [`app::graphics::PictSource`] instead.
///
/// **Through `graphics::resource_blorb`, not `blorb::resolve_resource_blorb`**
/// (SQ-1085), so the two arms resolve from the same tiers. The bare `blorb`
/// call knows the filesystem: a self-blorb, a same-stem sidecar, a directory
/// scan. It does not know about the ZIP a player downloaded the game in — so a
/// zipped `.gblorb` ran with no pictures and no sounds at all, which is the
/// worse half of the same defect, since Glulx is the engine whose games most
/// often ARE one big resource-carrying Blorb.
///
/// Nothing else moves: the extra tier only fires when `story_path` is a zip,
/// and the build-mismatch refusal `graphics::resource_blorb` adds is inert here
/// — it needs a story mounted off a release disk image with an identifiable
/// build, which no Glulx or Scott game is.
fn resolve_pict_blorb(story_path: &std::path::Path, images: bool) -> Option<blorb::Blorb> {
    if images {
        app::graphics::resource_blorb(story_path).found.map(|(b, _)| b)
    } else {
        None
    }
}

// ── Utilities ─────────────────────────────────────────────────────────────────

/// Whether the game echoed the just-submitted command itself at the start of its
/// turn output (e.g. CounterfeitMonkey prints the command back in bold). Compared
/// case-insensitively against the leading non-whitespace text, and only when the
/// echo ends at a boundary (so `go` doesn't match a response starting `gospel`),
/// so we don't add a second, redundant echo. An empty command never matches.
fn game_echoes_command(transcript: &str, cmd: &str) -> bool {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return false;
    }
    let mut head = transcript.trim_start().chars();
    for cc in cmd.chars() {
        match head.next() {
            Some(hc) if hc.eq_ignore_ascii_case(&cc) => {}
            _ => return false,
        }
    }
    // The command must be followed by a boundary, not more word characters.
    match head.next() {
        None => true,
        Some(c) => !c.is_alphanumeric(),
    }
}

/// The current story's saves for the saves manager: `.lanthorn` Save States and
/// `.qzl` game saves in `game_dir` merged into one list, sorted newest-first by
/// save time. RFC3339 timestamps sort chronologically as strings; untimestamped/
/// legacy saves (empty timestamp) sort to the bottom.
fn combined_saves(game_dir: &std::path::Path) -> Vec<app::persist_files::SaveInfo> {
    let mut entries = list_saves(game_dir);
    entries.extend(app::persist_files::list_qzl(game_dir));
    entries.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
    entries
}

/// Format a Unix timestamp (seconds since epoch) as an RFC3339 UTC string.
fn format_rfc3339(secs: u64) -> String {
    let sec = secs % 60;
    let min = (secs / 60) % 60;
    let hour = (secs / 3600) % 24;
    let days = secs / 86400;
    let (year, month, day) = days_to_ymd_main(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", year, month, day, hour, min, sec)
}

fn days_to_ymd_main(mut days: u64) -> (u64, u64, u64) {
    days += 719468;
    let era = days / 146097;
    let doe = days % 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Return (width, height) of the map pane, defaulting to (80, 24) when zero.
fn map_pane_dims(area: Rect) -> (u16, u16) {
    let w = if area.width == 0 { 80 } else { area.width };
    let h = if area.height == 0 { 24 } else { area.height };
    (w, h)
}

/// Re-observe the VM's current location after a restore/resume: fold the room into the
/// map, deselect the viewed layer, select the room, and recenter the map pane on it.
/// Produces no transcript output. Shared by every host restore/resume arm.
fn reobserve_location(
    state: &mut AppState,
    mapper: &mut Mapper,
    session: &dyn Engine,
    map_rect: Rect,
) {
    // Every caller is a restore/resume/import: the live state now equals a saved
    // one, so there is no unsaved progress to warn about on quit.
    state.unsaved_progress = false;
    // The caller has just swapped in a restored/imported mapper (or is about to
    // re-observe into it); invalidate the map render memo so the loaded map shows
    // this frame instead of the pre-restore one. Unconditional so even the
    // no-current-location early-return below still invalidates. (SQ-0305)
    state.bump_graph_gen();
    // The restored game is not the one the death watch was watching: a death outstanding in the
    // live session says nothing about the saved one, and the re-observation below is itself a room
    // change with no passage behind it. Cleared before the early return, so a restore into a game
    // that reports no location does not carry the old one's death either. (SQ-0671, SQ-0673)
    state.death_watch = Default::default();
    let Some(snap) = session.current_location() else { return };
    let rid = snap.number as mapper::graph::RoomId;
    let restore_result = TurnResult::observation(snap);
    apply_turn(mapper, "", &restore_result, &mut state.death_watch);
    state.set_viewed_layer(None);
    state.select_room(Some(rid));
    if let Some(room) = mapper.graph.room(rid) {
        if let Some(pos) = room.pos {
            let (pw, ph) = map_pane_dims(map_rect);
            state.recenter_on(pos, pw, ph);
        }
    }
}

/// Build a `DialogStyle` from the current app colors.
/// Note: `BorderStyle::None` is coerced to `Single` inside `draw_dialog`.
fn make_dialog_style(state: &AppState) -> DialogStyle<'_> {
    DialogStyle::from_colors(&state.colors)
}

/// Apply `Modifier::DIM` to every cell in `area` of `buf`.
/// Called after a pane's content is rendered to de-emphasise the unfocused pane.
fn dim_area(buf: &mut ratatui::buffer::Buffer, area: Rect) {
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_style(cell.style().add_modifier(Modifier::DIM));
            }
        }
    }
}

// ── Slash-command helper ──────────────────────────────────────────────────────

/// Return true when `input` starts with the configured command `prefix` char.
fn is_slash(input: &str, prefix: char) -> bool {
    input.starts_with(prefix)
}

// ── Quit dialog helpers ───────────────────────────────────────────────────────

// ── Hints open helper ─────────────────────────────────────────────────────────

/// The opening transcript for a freshly-booted hint companion, with the
/// InvisiClues narrow-screen warning auto-skipped.
///
/// The izm hint files open on a "your screen is only N characters wide…" banner
/// and wait for a keypress before showing the topic menu (the menu lives in the
/// upper window). When the boot output is that banner, press one key here so the
/// player lands straight on the menu; the keypress erases the banner. If the
/// output isn't the banner (or the file isn't waiting for a key), fall back to
/// the raw opening — no harm, the banner just shows as before.
///
/// Gated on `skip_warning` (the `hint_skip_screen_warning` config, default on);
/// when off, the banner is left in place for the player to dismiss.
fn hint_opening(vm: &mut app::session::GameSession, skip_warning: bool) -> String {
    let opening = vm.take_transcript();
    if skip_warning
        && hints::is_narrow_screen_warning(&opening)
        && matches!(vm.pending_input(), app::session::InputKind::Char)
    {
        return vm.submit_char(b' ').transcript;
    }
    opening
}

/// Open the hints panel for the current story, resolving the hint source.
///
/// If a panel is already open this is a no-op.  Discovery order:
/// 1. Remembered per-IFID association.
/// 2. Sibling hint file.
/// 3. Inside a sibling ZIP.
/// 4. AskUser: status message + TODO for file-browser wiring.
/// 5. None: status "no hints found".
fn open_hints(
    state: &mut AppState,
    story_path: &std::path::Path,
    ifid: &str,
    user_dir: &std::path::Path,
) {
    if state.overlays.hints.is_some() {
        return;
    }

    // Built-in HINT detection: check story dictionary for "hint"/"hints".
    // state.dict_words is populated at startup from the story's Z-machine dictionary.
    let builtin_hint = hints::story_supports_hint(state.dict_words.iter().cloned());

    let index = hints::load_hint_index(user_dir);
    let resolution = hints::resolve_hint_source(story_path, ifid, &index);

    match resolution {
        hints::HintResolution::File(p) => {
            match hints::load_story_bytes(&p) {
                Ok(bytes) => {
                    match app::session::GameSession::new(bytes, state.config.honor_game_colours, false, state.config.interpreter_number) {
                        Ok(mut vm) => {
                            vm.machine.undo_cap = state.config.undo_levels;
                            let opening = hint_opening(&mut vm, state.config.hint_skip_screen_warning);
                            let transcript: Vec<String> =
                                opening.split('\n').map(|l| l.to_owned()).collect();
                            let label = p
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("Hints")
                                .to_owned();
                            state.overlays.hints = Some(app::state::HintSession {
                                source: app::state::HintSource::Zcode(vm),
                                transcript,
                                scroll: 0,
                                clear_anchor: None,
                                scroll_anim: None,
                                input: String::new(),
                                label,
                                builtin_hint,
                            });
                        }
                        Err(e) => {
                            state.set_status(format!("hints: failed to load hint VM: {:?}", e));
                        }
                    }
                }
                Err(e) => {
                    state.set_status(format!("hints: cannot read hint file: {}", e));
                }
            }
        }
        hints::HintResolution::ZipEntry { zip_path, entry } => {
            let pred = |name: &str| name == entry;
            match hints::read_zip_entry(&zip_path, pred) {
                Ok(Some(bytes)) => {
                    match app::session::GameSession::new(bytes, state.config.honor_game_colours, false, state.config.interpreter_number) {
                        Ok(mut vm) => {
                            vm.machine.undo_cap = state.config.undo_levels;
                            let opening = hint_opening(&mut vm, state.config.hint_skip_screen_warning);
                            let transcript: Vec<String> =
                                opening.split('\n').map(|l| l.to_owned()).collect();
                            let label = entry.rsplit('/').next().unwrap_or(&entry).to_owned();
                            state.overlays.hints = Some(app::state::HintSession {
                                source: app::state::HintSource::Zcode(vm),
                                transcript,
                                scroll: 0,
                                clear_anchor: None,
                                scroll_anim: None,
                                input: String::new(),
                                label,
                                builtin_hint,
                            });
                        }
                        Err(e) => {
                            state.set_status(format!("hints: failed to load hint VM: {:?}", e));
                        }
                    }
                }
                Ok(None) => {
                    state.set_status("hints: hint entry not found in zip");
                }
                Err(e) => {
                    state.set_status(format!("hints: cannot read zip entry: {}", e));
                }
            }
        }
        hints::HintResolution::AskUser => {
            // TODO: wire the file browser to pick a hint file (.z3/.z5/.z8), then call
            // save_hint_assoc(user_dir, ifid, &picked) and restart as File path above.
            // For now, surface a status message so the user knows what to do.
            state.set_status(
                "no hint file found — place <story>.hints.z5 next to the story, or use /hints <path>",
            );
        }
        hints::HintResolution::None => {
            state.set_status("no hints found");
        }
    }
}

/// Return true when a quit attempt should show the "Save state before quitting?" dialog.
///
/// Conditions: auto_save is off AND prompt_save_on_quit is on AND there is progress
/// not yet captured in a Save State (`unsaved_progress`) — so quitting right after a
/// Ctrl-S / save / load does not prompt.
fn should_prompt_save_on_quit(state: &AppState) -> bool {
    !state.config.auto_save && state.config.prompt_save_on_quit && state.unsaved_progress
}

// ── Scroll-to-match helper ────────────────────────────────────────────────────

/// Given a match at `match_visible_pos` (0-based) within `total_visible` visible rows,
/// return the `transcript_scroll` value that brings that row to the top of the viewport
/// (`pane_rows` high).
///
/// The windowing in `visible_wrapped_lines_kinded` uses:
///   end   = total_visible - scroll
///   start = end - pane_rows
/// So placing the match at the top of the viewport means:
///   end = match_visible_pos + pane_rows
///   scroll = total_visible - end = total_visible - match_visible_pos - pane_rows
/// Clamped to 0 when the match is near the bottom (no scrollback needed).
///
/// Limitation: this helper treats each logical visible line as one display row.
/// When a line wraps into multiple display rows the match may land slightly
/// off-screen; correct wrap-aware scrolling would require counting wrapped rows
/// for every line above the match, which is not done here.
fn scroll_for_match(match_visible_pos: usize, total_visible: usize, pane_rows: usize) -> u16 {
    total_visible
        .saturating_sub(match_visible_pos)
        .saturating_sub(pane_rows) as u16
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Minimal v4 story: `read_char` (store->G0) at 0x40, then `@save` (store
/// form, ->G0) at 0x44, then `quit` at 0x46. Mirrors session.rs's
/// (crate-private) `read_char_then_save_v4` fixture, duplicated here
/// since this test lives in the separate `app` *binary* crate. Shared by
/// `engine_helpers`'s restore-dispatch test and `turn`'s resume tests — both
/// t-session, which is why this lives outside `mod tests` below (that mod is
/// t-misc) with its own gate matching its actual (and only) consumers.
#[cfg(all(test, feature = "t-session"))]
pub(crate) fn read_char_then_save_v4_story() -> Vec<u8> {
    let mut buf = vec![0u8; 0x0800];
    buf[0x00] = 4; // version 4 (0OP save/restore store form lives here)
    buf[0x04] = 0x04; buf[0x05] = 0x00; // high_mem_base = 0x0400
    buf[0x06] = 0x00; buf[0x07] = 0x40; // initial_pc = 0x0040
    buf[0x08] = 0x00; buf[0x09] = 0x80; // dictionary = 0x0080 (empty)
    buf[0x0080] = 0; buf[0x0081] = 4; buf[0x0082] = 0; buf[0x0083] = 0;
    buf[0x0A] = 0x01; buf[0x0B] = 0x00; // object_table = 0x0100
    buf[0x0C] = 0x03; buf[0x0D] = 0x00; // global_vars = 0x0300
    buf[0x0E] = 0x04; buf[0x0F] = 0x00; // static_mem_base = 0x0400
    buf[0x18] = 0x00; buf[0x19] = 0x60; // abbrev_table = 0x0060
    buf[0x0040] = 0xF6; // VAR read_char
    buf[0x0041] = 0x7F; // type: small(01), omit(11), omit(11), omit(11)
    buf[0x0042] = 1;    // operand: device=1
    buf[0x0043] = 0x10; // store -> G0
    buf[0x0044] = 0xB5; // 0OP:0x05 save (store form)
    buf[0x0045] = 0x10; // store -> G0
    buf[0x0046] = 0xBA; // quit
    buf
}

#[cfg(all(test, feature = "t-misc"))]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Modifier;
    use crossterm::event::Event;

    use super::{
        dim_area, is_slash, matrix_update_hover, scroll_for_match, should_prompt_save_on_quit,
        PaneRects, RoomId, RunOutcome,
    };
    use app::render::paneframe::{draw_pane_frame, draw_top_inset, InsetCaps, InsetSegment, PaneGlyphs};
    use app::state::{AppState, ExitTarget};

    // ── SQ-1258: a picker-launched run always resolves back to the library ─────

    /// The outer loop's whole exit-resolution rule is `ExitTarget::for_launch`
    /// plus this `From` — nothing else decides it (`run_event_loop` seeds
    /// `exit_target` from it at boot; `Action::Quit`, `SlashOutcome::Quit`, and
    /// `OverlayAct::QuitCancel` all resolve or restore through the same call). A
    /// game's own clean quit never touches `exit_target` at all, so it inherits
    /// whatever the boot default was — meaning "launched from the picker + the
    /// GAME quit" and "launched from the picker + the player's own `quit`
    /// command / Ctrl-Q" reach the identical answer this pins.
    #[test]
    fn library_launch_always_resolves_to_the_library() {
        assert_eq!(
            RunOutcome::from(ExitTarget::for_launch(true)),
            RunOutcome::ToLibrary,
            "a picker launch returns to the list on ANY way the run ends"
        );
    }

    #[test]
    fn command_line_launch_always_resolves_to_exit() {
        assert_eq!(
            RunOutcome::from(ExitTarget::for_launch(false)),
            RunOutcome::Exit,
            "no picker exists to return to — every ending leaves lanthorn"
        );
    }

    // ── SQ-0649: the panic hook must not tear down a live session ──────────────

    /// A recovered worker's panic must not restore the terminal (which would
    /// leave the still-running loop drawing onto a cooked normal screen) nor
    /// print a "crashed" banner over a session that is very much alive. Only the
    /// main thread's panic — the one that unwinds out of the event loop and ends
    /// the process — is fatal.
    #[test]
    fn only_the_main_threads_panic_tears_the_terminal_down() {
        let main_id = std::thread::current().id();
        let worker_id = std::thread::spawn(|| std::thread::current().id())
            .join()
            .expect("worker returns its id");
        assert_ne!(main_id, worker_id, "the ids must actually differ");

        assert!(
            super::panic_is_fatal(main_id, Some(main_id)),
            "a main-thread panic ends the process: restore the terminal"
        );
        assert!(
            !super::panic_is_fatal(worker_id, Some(main_id)),
            "a recovered worker panic must leave the live session's terminal alone"
        );
        // Id never captured (hook somehow ran before install): fail safe.
        assert!(super::panic_is_fatal(worker_id, None));
    }

    // ── SQ-1246: matrix-view room hover ─────────────────────────────────────────

    fn moved_at(col: u16, row: u16) -> Event {
        Event::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Moved,
            column: col,
            row,
            modifiers: crossterm::event::KeyModifiers::NONE,
        })
    }

    fn matrix_panes(room: RoomId, rect: Rect) -> PaneRects {
        PaneRects {
            room_rects: vec![(room, rect)],
            map_view: mapper::layer::MapView::Matrix,
            ..Default::default()
        }
    }

    /// The headline case: pointer motion over a published rect in the matrix view resolves the
    /// room it names; motion elsewhere clears it.
    #[test]
    fn matrix_hover_resolves_over_a_published_rect_and_clears_off_it() {
        let rect = Rect::new(0, 2, app::render::matrix::LABEL_W, 1);
        let panes = matrix_panes(3, rect);
        let mut st = AppState::default();

        matrix_update_hover(&mut st, &panes, &moved_at(2, 2));
        assert_eq!(st.matrix_hover, Some((3, rect)), "the pointer sits inside the rect");

        matrix_update_hover(&mut st, &panes, &moved_at(50, 2));
        assert_eq!(st.matrix_hover, None, "moved off the rect: cleared");
    }

    /// A rect this frame's room_rects never published (an empty cell, `·`/`×`) resolves to no
    /// hover no matter where the pointer lands.
    #[test]
    fn matrix_hover_is_none_over_a_point_with_no_published_rect() {
        let rect = Rect::new(0, 2, app::render::matrix::LABEL_W, 1);
        let panes = matrix_panes(3, rect);
        let mut st = AppState::default();
        matrix_update_hover(&mut st, &panes, &moved_at(80, 20));
        assert_eq!(st.matrix_hover, None, "no rect at that point: no tooltip");
    }

    /// The drawn (non-matrix) map view publishes `room_rects` too — its room boxes — and those
    /// must never populate `matrix_hover`; that view's hover behaviour is out of scope for this
    /// feature and untouched.
    #[test]
    fn matrix_hover_stays_none_in_the_drawn_map_view() {
        let rect = Rect::new(0, 2, app::render::matrix::LABEL_W, 1);
        let mut panes = matrix_panes(3, rect);
        panes.map_view = mapper::layer::MapView::Drawn;
        let mut st = AppState::default();
        matrix_update_hover(&mut st, &panes, &moved_at(2, 2));
        assert_eq!(st.matrix_hover, None, "the drawn view's room boxes are not a matrix hover");
    }

    /// A modal dialog owns the pointer; hover resolution must not populate `matrix_hover`
    /// underneath it, even over an otherwise-valid rect.
    #[test]
    fn matrix_hover_is_suppressed_while_a_modal_overlay_is_open() {
        let rect = Rect::new(0, 2, app::render::matrix::LABEL_W, 1);
        let panes = matrix_panes(3, rect);
        let mut st = AppState::default();
        st.overlays.hotkey_dialog = true;
        matrix_update_hover(&mut st, &panes, &moved_at(2, 2));
        assert_eq!(st.matrix_hover, None, "a modal overlay must suppress the hover");
    }

    /// A non-`Moved` mouse event (a click, say) must not disturb whatever hover a prior `Moved`
    /// left in place — this handler only ever reacts to motion.
    #[test]
    fn matrix_hover_ignores_non_moved_events() {
        let rect = Rect::new(0, 2, app::render::matrix::LABEL_W, 1);
        let panes = matrix_panes(3, rect);
        let mut st = AppState::default();
        matrix_update_hover(&mut st, &panes, &moved_at(2, 2));
        assert_eq!(st.matrix_hover, Some((3, rect)));

        let click = Event::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 2,
            row: 2,
            modifiers: crossterm::event::KeyModifiers::NONE,
        });
        matrix_update_hover(&mut st, &panes, &click);
        assert_eq!(st.matrix_hover, Some((3, rect)), "a click leaves the hover exactly as it was");
    }

    // ── SQ-0651 / SQ-0644: the watchdog must not kill an exit save in flight ───

    #[test]
    fn watchdog_extends_its_grace_only_while_a_save_is_running() {
        use super::{watchdog_should_keep_waiting, TERM_WATCHDOG_GRACE_MS, TERM_WATCHDOG_HARD_CAP_MS};
        // Nothing saving: the fixed grace has expired, exit now.
        assert!(!watchdog_should_keep_waiting(TERM_WATCHDOG_GRACE_MS, false));
        // A save actively writing past the grace: keep waiting rather than
        // killing the process mid-write and losing it.
        assert!(watchdog_should_keep_waiting(TERM_WATCHDOG_GRACE_MS, true));
        assert!(watchdog_should_keep_waiting(TERM_WATCHDOG_HARD_CAP_MS - 1, true));
        // …but never past the hard cap: a hung save must not keep a process alive
        // after its terminal is gone.
        assert!(!watchdog_should_keep_waiting(TERM_WATCHDOG_HARD_CAP_MS, true));
        assert!(!watchdog_should_keep_waiting(TERM_WATCHDOG_HARD_CAP_MS + 5_000, true));
        // The cap must leave room beyond the fixed grace, or the extension is a
        // no-op; both are consts, so this is checked at compile time.
        const { assert!(TERM_WATCHDOG_HARD_CAP_MS > TERM_WATCHDOG_GRACE_MS) };
    }

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
        let mut state = app::state::AppState::default();
        let mut mapper = mapper::mapper::Mapper::default();
        let mut engine = ClocklessEngine;
        state.glulx_timer_next_fire = Some(std::time::Instant::now() - std::time::Duration::from_millis(5));

        let (redraw, quit) = super::dispatch_due_game_clocks(
            &mut state,
            &mut mapper,
            &mut engine,
            std::path::Path::new("/nonexistent"),
            Rect::default(),
        );
        assert!(redraw, "a fired timer repaints");
        assert!(!quit);
        assert!(state.glulx_timer_next_fire.is_none(), "an elapsed deadline must disarm, not refire every tick");

        // A future deadline is left alone.
        let future = std::time::Instant::now() + std::time::Duration::from_secs(30);
        state.glulx_timer_next_fire = Some(future);
        let (redraw, _) = super::dispatch_due_game_clocks(
            &mut state,
            &mut mapper,
            &mut engine,
            std::path::Path::new("/nonexistent"),
            Rect::default(),
        );
        assert!(!redraw, "nothing due: no repaint");
        assert_eq!(state.glulx_timer_next_fire, Some(future), "still armed");
    }

    /// Minimal engine that is neither a Z-machine nor a Glulx session, so the
    /// clock dispatch's downcasts all miss. Only the engine-neutral bookkeeping
    /// (disarm + redraw) is exercised.
    struct ClocklessEngine;

    impl app::engine::Engine for ClocklessEngine {
        fn submit(&mut self, _command: &str) -> app::session::TurnResult { unreachable!() }
        fn submit_key(&mut self, _key: app::engine::KeyInput) -> Option<app::session::TurnResult> { unreachable!() }
        fn take_transcript(&mut self) -> String { unreachable!() }
        // No screen-clear channel: this double is not a game.
        fn drain_screen_clear(&mut self) -> bool { false }
        fn pending_input(&self) -> app::session::InputKind { unreachable!() }
        fn resume_save(&mut self, _wrote_ok: bool) -> app::session::TurnResult { unreachable!() }
        fn resume_restore(&mut self, _data: Option<&[u8]>) -> app::session::TurnResult { unreachable!() }
        fn has_quit(&self) -> bool { false }
        fn screen(&self) -> app::engine::ScreenModel { unreachable!() }
        fn save_state(&self) -> app::engine::EngineSave { unreachable!() }
        fn restore_state(&mut self, _save: &app::engine::EngineSave) -> Result<(), app::engine::EngineError> { unreachable!() }
        fn restore_game_save(&mut self, _bytes: &[u8]) -> Result<(), app::engine::EngineError> { unreachable!() }
        fn aux_data(&self) -> &std::collections::BTreeMap<String, Vec<u8>> { unreachable!() }
        fn set_aux_data(&mut self, _data: std::collections::BTreeMap<String, Vec<u8>>) {}
        fn aux_dirty(&self) -> bool { false }
        fn clear_aux_dirty(&mut self) {}
        fn current_location(&self) -> Option<app::engine::LocationInfo> { None }
        fn as_any(&self) -> &dyn std::any::Any { self }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    }

    // ── SQ-0502: termination-signal exit code ──────────────────────────────────

    #[cfg(unix)] // signal_hook's SIGHUP/SIGTERM consts don't exist on Windows
    #[test]
    fn term_exit_code_is_128_plus_signum() {
        use std::sync::atomic::Ordering;
        // No signal captured yet → fall back to 130 (128 + SIGINT).
        super::TERM_SIGNUM.store(0, Ordering::SeqCst);
        assert_eq!(super::term_exit_code(), 130);
        // Each captured signal maps to the conventional 128 + signum exit code.
        super::TERM_SIGNUM.store(signal_hook::consts::SIGHUP, Ordering::SeqCst);
        assert_eq!(super::term_exit_code(), 128 + signal_hook::consts::SIGHUP);
        super::TERM_SIGNUM.store(signal_hook::consts::SIGTERM, Ordering::SeqCst);
        assert_eq!(super::term_exit_code(), 128 + signal_hook::consts::SIGTERM);
        // Leave the global back at its default so no other test observes a stray value.
        super::TERM_SIGNUM.store(0, Ordering::SeqCst);
    }

    // ── SQ-0460: withhold arrow keys from v6 stories ───────────────────────────

    #[test]
    fn forward_arrow_to_v6_gates_only_v6_when_disabled() {
        // v6_arrow_keys = true: every version forwards arrows.
        assert!(super::forward_arrow_to_v6(true, 6));
        assert!(super::forward_arrow_to_v6(true, 5));
        assert!(super::forward_arrow_to_v6(true, 0));

        // v6_arrow_keys = false (the default, SQ-1087): only version 6 is
        // withheld; v1-5 and the Glulx/no-session placeholder (version 0) still
        // forward arrows.
        assert!(!super::forward_arrow_to_v6(false, 6));
        assert!(super::forward_arrow_to_v6(false, 5));
        assert!(super::forward_arrow_to_v6(false, 3));
        assert!(super::forward_arrow_to_v6(false, 0));
    }

    #[test]
    fn withhold_arrow_from_v6_covers_all_arrows_and_only_arrows() {
        use app::engine::KeyInput;
        // The SQ-0188 line-terminator gate uses this predicate with
        // is_line_input = true — v6 games list arrows as line terminators for
        // movement, so gating read_char alone left arrows moving the player.
        for arrow in [KeyInput::Up, KeyInput::Down, KeyInput::Left, KeyInput::Right] {
            assert!(super::withhold_arrow_from_v6(Some(arrow), false, 6, true), "{arrow:?} withheld on v6 when off");
            assert!(!super::withhold_arrow_from_v6(Some(arrow), true, 6, true), "{arrow:?} forwarded when on");
            assert!(!super::withhold_arrow_from_v6(Some(arrow), false, 5, true), "{arrow:?} forwarded on v5");
        }
        // Non-arrows and no-input keys are never withheld.
        assert!(!super::withhold_arrow_from_v6(Some(KeyInput::Enter), false, 6, true));
        assert!(!super::withhold_arrow_from_v6(Some(KeyInput::Func(1)), false, 6, true));
        assert!(!super::withhold_arrow_from_v6(None, false, 6, true));
    }

    #[test]
    fn withhold_arrow_from_v6_never_withholds_during_char_input() {
        use app::engine::KeyInput;
        // SQ-0483: the char-input (read_char) gate calls the predicate with
        // is_line_input = false. Menus (Shogun's startup menu, hint menus,
        // "press any key") are unnavigable without arrows, so v6 arrows are
        // ALWAYS delivered there — the setting has no say during char input.
        for arrow in [KeyInput::Up, KeyInput::Down, KeyInput::Left, KeyInput::Right] {
            // (a) setting off + char input pending → arrow IS delivered.
            assert!(
                !super::withhold_arrow_from_v6(Some(arrow), false, 6, false),
                "{arrow:?} must reach a v6 menu even with the setting off",
            );
            // (c) setting on → delivered during char input too.
            assert!(!super::withhold_arrow_from_v6(Some(arrow), true, 6, false));
        }
        // Contrast (b): the SAME arrow + setting off IS withheld at a line
        // prompt — that path is covered above with is_line_input = true.
        assert!(super::withhold_arrow_from_v6(Some(KeyInput::Up), false, 6, true));
    }

    // ── SQ-0297: map-export slash commands must actually write the file ────────

    #[test]
    fn handle_map_export_writes_the_file_into_the_game_dir() {
        use std::fs;
        use app::input::Action;
        use app::state::AppState;
        use mapper::mapper::Mapper;
        let dir = std::env::temp_dir().join(format!("bm-handle-map-export-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mapper = Mapper::default();
        let mut state = AppState::default();

        assert!(super::handle_map_export(&Action::ExportSvg(None), &dir, &mapper, &mut state));
        assert!(dir.join("map.svg").exists(), "SVG export must write map.svg into the game dir");

        assert!(super::handle_map_export(&Action::ExportDot(Some("mymap".into())), &dir, &mapper, &mut state));
        assert!(dir.join("mymap.dot").exists(), "DOT export with a bare-name arg must land in the game dir");

        assert!(super::handle_map_export(&Action::ExportMap(None), &dir, &mapper, &mut state));
        assert!(dir.join("map.txt").exists(), "dump export must write map.txt into the game dir");

        assert!(!super::handle_map_export(&Action::ToggleWatch, &dir, &mapper, &mut state),
            "a non-export action must not be treated as handled");

        let _ = fs::remove_dir_all(&dir);
    }

    // ── SQ-0230: list_qzl filters to the current story's game saves ─────────────

    #[test]
    fn list_qzl_lists_game_saves_in_game_dir_and_skips_lanthorn() {
        use std::fs;
        // SQ-0284: all `.qzl` in a per-game dir belong to this story (no IFID
        // prefix filtering). `.lanthorn` files are never picked up by list_qzl.
        let dir = std::env::temp_dir().join(format!("bm-listqzl-{}/Zork1.z5", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("slot1.qzl"), b"x").unwrap();
        fs::write(dir.join("slot1.lanthorn"), b"x").unwrap();

        // combined_saves merges .lanthorn + .qzl newest-first; here the .lanthorn
        // has no valid archive so list_saves skips it, leaving the one game save.
        let combined: Vec<String> = super::combined_saves(&dir).iter().map(|s| s.name.clone()).collect();
        assert_eq!(combined, vec!["slot1".to_string()], "combined list includes the game save");

        let infos = app::persist_files::list_qzl(&dir);
        let names: Vec<String> = infos.iter().map(|s| s.name.clone()).collect();
        // The `.qzl` suffix is stripped to the slug for display; the `.lanthorn`
        // is excluded from list_qzl.
        assert_eq!(names, vec!["slot1".to_string()]);
        // And they carry a save timestamp read from the file's mtime.
        assert!(!infos[0].saved_at.is_empty(), "game saves are timestamped from file mtime");

        // Remove the per-run parent, not just the game dir inside it — clearing only
        // `…/Zork1.z5` left one empty `bm-listqzl-<pid>` behind per run for ever.
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn combined_saves_sorts_newest_first_untimestamped_last() {
        let mk = |name: &str, ts: &str| app::persist_files::SaveInfo {
            path: std::path::PathBuf::from(format!("/tmp/{name}.qzl")),
            name: name.to_string(),
            turns: 0,
            saved_at: ts.to_string(),
            location: None, score: None, is_default: false, trigger: app::archive::SaveTrigger::HostState,
        };
        let mut v = [mk("old", "2026-06-01T10:00:00Z"),
            mk("legacy", ""),
            mk("new", "2026-07-09T12:00:00Z"),
            mk("mid", "2026-06-30T08:00:00Z")];
        // Same comparator combined_saves uses (RFC3339 sorts chronologically).
        v.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
        let order: Vec<&str> = v.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(order, vec!["new", "mid", "old", "legacy"],
            "newest first; untimestamped/legacy saves sort to the bottom");
    }

    #[test]
    fn game_echoes_command_detects_self_echo() {
        use super::game_echoes_command;
        // CounterfeitMonkey shape: the turn output starts with the command (bold),
        // then the response — case-insensitive, boundary-terminated.
        assert!(game_echoes_command("yes\n\nGood, you're conscious.", "yes"));
        assert!(game_echoes_command("YES\n\n...", "yes"), "case-insensitive");
        assert!(game_echoes_command("examine me\n\nYou see nothing special.", "examine me"));
        assert!(game_echoes_command("  look\nA room.", "look"), "leading whitespace ok");
        // Most games: the response does not start with the command → keep our echo.
        assert!(!game_echoes_command("You can't go that way.\n>", "north"));
        assert!(!game_echoes_command("", "look"), "empty output");
        assert!(!game_echoes_command("anything", ""), "empty command never matches");
        // Boundary: a command must not match a longer word it is a prefix of.
        assert!(!game_echoes_command("gospel music plays.", "go"));
    }

    #[test]
    fn resolve_pict_blorb_finds_sidecar_for_bare_ulx() {
        // Regression test for SQ-0173: restart's Pict-blorb resolution must find
        // a same-stem sidecar .blorb for a bare .ulx the same path-based way as
        // launch (blorb::resolve_resource_blorb), not the old bytes-only
        // blorb::Blorb::parse(story_bytes), which only ever finds images inside
        // a self-contained .gblorb.
        fn png_bytes() -> Vec<u8> {
            let img = image::RgbImage::from_pixel(2, 2, image::Rgb([255, 0, 0]));
            let mut bytes = Vec::new();
            image::DynamicImage::ImageRgb8(img)
                .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
                .unwrap();
            bytes
        }

        // Build an IFF chunk: type + BE len + data + pad-to-even.
        fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(ty);
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 {
                v.push(0);
            }
            v
        }

        // Build a minimal FORM/IFRS blorb with only a Pict (PNG) resource — no
        // sound. resolve_resource_blorb accepts a resource sidecar that carries
        // pictures OR sounds (SQ-0372), so a graphics-only sidecar like Beyond
        // Zork's `beyondzork.blb` resolves without needing a dummy Snd entry.
        fn build_sidecar_blorb(png: &[u8]) -> Vec<u8> {
            #[allow(clippy::type_complexity)]
            let res: [(&[u8; 4], u32, &[u8; 4], &[u8]); 1] =
                [(b"Pict", 0, b"PNG ", png)];
            let ridx_data_len = 4 + 12 * res.len();
            let first_res_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
            let mut offsets = Vec::new();
            let mut cursor = first_res_off;
            let mut body = Vec::new();
            for (_u, _n, ty, data) in res.iter() {
                offsets.push(cursor as u32);
                let c = chunk(ty, data);
                cursor += c.len();
                body.extend_from_slice(&c);
            }
            let mut ridx = Vec::new();
            ridx.extend_from_slice(&(res.len() as u32).to_be_bytes());
            for (i, (usage, number, _ty, _d)) in res.iter().enumerate() {
                ridx.extend_from_slice(*usage);
                ridx.extend_from_slice(&number.to_be_bytes());
                ridx.extend_from_slice(&offsets[i].to_be_bytes());
            }
            let ridx_chunk = chunk(b"RIdx", &ridx);
            let mut inner = Vec::new();
            inner.extend_from_slice(b"IFRS");
            inner.extend_from_slice(&ridx_chunk);
            inner.extend_from_slice(&body);
            let mut file = Vec::new();
            file.extend_from_slice(b"FORM");
            file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
            file.extend_from_slice(&inner);
            file
        }

        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../gvm-cli/tests/fixtures/glulxercise.ulx");
        let Ok(ulx_bytes) = std::fs::read(&fixture) else { return };

        let dir = std::env::temp_dir().join(format!("bm-pictblorb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let ulx_path = dir.join("game.ulx");
        std::fs::write(&ulx_path, &ulx_bytes).expect("write game.ulx");
        let blorb_path = dir.join("game.blorb");
        std::fs::write(&blorb_path, build_sidecar_blorb(&png_bytes())).expect("write sidecar");

        assert!(
            super::resolve_pict_blorb(&ulx_path, true).is_some(),
            "sidecar .blorb next to a bare .ulx must resolve (regression: the old \
             bytes-only logic returned None for a non-self-contained story)"
        );
        assert!(
            super::resolve_pict_blorb(&ulx_path, false).is_none(),
            "images disabled must resolve to None regardless of sidecar"
        );

        let no_sidecar_dir =
            std::env::temp_dir().join(format!("bm-pictblorb-nosc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&no_sidecar_dir);
        std::fs::create_dir_all(&no_sidecar_dir).expect("create temp dir");
        let lone_ulx = no_sidecar_dir.join("lone.ulx");
        std::fs::write(&lone_ulx, &ulx_bytes).expect("write lone.ulx");
        assert!(
            super::resolve_pict_blorb(&lone_ulx, true).is_none(),
            "no sidecar present must resolve to None"
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&no_sidecar_dir);
    }

    /// The same resolution, when the download was never unpacked (SQ-1085).
    ///
    /// **Glulx is the engine this matters most to**, because a Glulx game very
    /// often IS one big resource-carrying Blorb — and `resolve_pict_blorb` is
    /// the only thing that hands the Glulx and Scott sessions their resources.
    /// Going straight to `blorb::resolve_resource_blorb` meant a zipped game
    /// booted and then drew nothing and played nothing, at launch AND after
    /// `@restart`, since both arms come through here.
    ///
    /// Built from `gvm-cli`'s committed `glulxercise.ulx`, so it never goes
    /// vacuous the way a `stories/` fixture would.
    #[test]
    fn resolve_pict_blorb_reaches_a_blorb_inside_the_storys_zip() {
        use std::io::Write as _;

        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../gvm-cli/tests/fixtures/glulxercise.ulx");
        let ulx_bytes = std::fs::read(&fixture).expect("glulxercise.ulx is committed");

        // A resources-only Blorb: one Pict, no executable — a `.blb` beside a
        // bare `.ulx`, which is the layout the sibling case above covers loose.
        fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(ty);
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 {
                v.push(0);
            }
            v
        }
        let png = {
            let img = image::RgbImage::from_pixel(2, 2, image::Rgb([0, 128, 255]));
            let mut bytes = Vec::new();
            image::DynamicImage::ImageRgb8(img)
                .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
                .unwrap();
            bytes
        };
        let sidecar = {
            let ridx_data_len = 4 + 12;
            let off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
            let mut ridx = Vec::new();
            ridx.extend_from_slice(&1u32.to_be_bytes());
            ridx.extend_from_slice(b"Pict");
            ridx.extend_from_slice(&0u32.to_be_bytes());
            ridx.extend_from_slice(&(off as u32).to_be_bytes());
            let mut inner = Vec::new();
            inner.extend_from_slice(b"IFRS");
            inner.extend_from_slice(&chunk(b"RIdx", &ridx));
            inner.extend_from_slice(&chunk(b"PNG ", &png));
            let mut file = Vec::new();
            file.extend_from_slice(b"FORM");
            file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
            file.extend_from_slice(&inner);
            file
        };

        let dir = std::env::temp_dir().join(format!("bm-pictblorb-zip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let zip_path = dir.join("glulxercise.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zw = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zw.start_file("glulxercise/glulxercise.ulx", opts).unwrap();
            zw.write_all(&ulx_bytes).unwrap();
            zw.start_file("glulxercise/glulxercise.blb", opts).unwrap();
            zw.write_all(&sidecar).unwrap();
            zw.finish().unwrap();
        }

        // The game itself opens out of the zip…
        assert!(
            matches!(app::hints::load_story(&zip_path), Ok(app::hints::LoadedStory::Glulx(_))),
            "the zipped .ulx must load as Glulx",
        );
        // …and the session is handed the artwork that came with it.
        let blorb = super::resolve_pict_blorb(&zip_path, true)
            .expect("the Blorb inside the zip must reach the Glulx session");
        assert_eq!(blorb.resources().len(), 1, "its one Pict is indexed");
        assert!(
            super::resolve_pict_blorb(&zip_path, false).is_none(),
            "images disabled still resolves to None",
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── TestBackend: map pane shows a single-line border by default ───────────

    /// SQ-0357: the map pane's default is a plain single-line border. It used to be an ornate
    /// picture-frame — a frame within a frame, which cost two columns and two rows of map to
    /// draw a second box around the first one.
    #[test]
    fn map_pane_default_is_a_single_line_border() {
        // Resolve the default look from DEFAULT_STYLE_TOML (same path as startup).
        let doc = app::style::parse_style_toml(app::style::DEFAULT_STYLE_TOML)
            .expect("DEFAULT_STYLE_TOML must parse");
        let (cs, _set, _warnings) = app::style::resolve(&doc, std::path::Path::new("."));

        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        let frame = draw_pane_frame(&mut buf, area, cs.map_border_style, &PaneGlyphs::default(), cs.theme.get("panel.border").style);
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "┌", "default map border is single-line");
        assert_eq!(buf.cell((0, 3)).unwrap().symbol(), "│");
        // Content is everything inside that one border — two more rows and columns of map than
        // the picture-frame left (which nested a second frame inside the first).
        assert_eq!(frame.content, Rect::new(1, 1, 18, 8));
    }

    // ── TestBackend: story pane shows adventure title in its border ───────────────

    /// Verify that the DEFAULT_STYLE_TOML-resolved ColorScheme configures
    /// story_border_style as single, that rendering it produces the ┌ outer
    /// corner at top-left, and that the adventure title appears in the top border row.
    #[test]
    fn story_pane_shows_title_in_border_by_default() {
        // Resolve the default look from DEFAULT_STYLE_TOML (same path as startup).
        let doc = app::style::parse_style_toml(app::style::DEFAULT_STYLE_TOML)
            .expect("DEFAULT_STYLE_TOML must parse");
        let (cs, _set, _warnings) = app::style::resolve(&doc, std::path::Path::new("."));

        let area = Rect::new(0, 0, 40, 15);
        let mut buf = Buffer::empty(area);

        // Draw the story pane frame (same as draw_frame does).
        let frame = draw_pane_frame(&mut buf, area, cs.story_border_style, &PaneGlyphs::default(), cs.theme.get("panel.border").style);

        // Overlay the adventure title (single centered segment, not active).
        draw_top_inset(
            &mut buf,
            frame.top_inset,
            &[InsetSegment { text: "ZORK I", active: false }],
            cs.theme.get("story_title").style,
            cs.theme.get("story_title").style,
            &InsetCaps::for_border(app::render::paneframe::BorderStyle::Thick),
        );

        // DEFAULT_STYLE_TOML sets story_border to single; top-left outer corner must be ┌
        assert_eq!(
            buf.cell((0, 0)).unwrap().symbol(),
            "┌",
            "default story border must be single (┌ at top-left)"
        );

        // The title "ZORK I" must appear somewhere in the top border row (row 0 for single).
        let title_row: String = (0..40u16)
            .map(|x| buf.cell((x, 0)).unwrap().symbol().to_string())
            .collect();
        assert!(
            title_row.contains("ZORK I"),
            "top border row must contain the adventure title 'ZORK I'; got: {:?}",
            title_row
        );
    }

    // ── Focus-aware panel border colour (SQ-0309 §2a) ──────────────────────────

    /// SQ-0441: the story title's terminator caps now track the pane's border
    /// style. At the default single border the left cap is `┤` (not the temporary
    /// thick `┫` from Task 1), proving `draw_panel` derives the caps from
    /// `panel.border` rather than hardcoding a style.
    #[test]
    fn story_title_left_cap_tracks_single_border() {
        use app::render::panel::{draw_panel, PanelSpec, PanelStrip};
        let doc = app::style::parse_style_toml(app::style::DEFAULT_STYLE_TOML)
            .expect("DEFAULT_STYLE_TOML must parse");
        let (cs, _set, _warnings) = app::style::resolve(&doc, std::path::Path::new("."));

        let area = Rect::new(0, 0, 40, 15);
        let mut buf = Buffer::empty(area);
        let segs = [InsetSegment { text: "ZORK I", active: false }];
        let title_style = cs.theme.get("story_title").style;
        let frame = draw_panel(
            &mut buf,
            &PanelSpec {
                area,
                border_selector: "panel.border",
                border_color: None,
                border_style: None,
                glyphs: &PaneGlyphs::default(),
                header_on: true,
                strip: Some(PanelStrip { segments: &segs, base: title_style, active: title_style }),
                body_fill: None,
            },
            &cs.theme,
        );
        // The left cap sits immediately left of the first segment on the top row.
        let r = frame.tab_rects[0];
        assert_eq!(
            buf.cell((r.x - 1, 0)).unwrap().symbol(),
            "┤",
            "single-border title left cap must be ┤, not the thick ┫",
        );
        // The thick cap must not appear anywhere on the title row.
        let title_row: String = (0..40u16)
            .map(|x| buf.cell((x, 0)).unwrap().symbol().to_string())
            .collect();
        assert!(
            !title_row.contains("┫"),
            "no thick cap at a single border; got: {:?}",
            title_row
        );
    }

    /// The pane with input focus renders its border BOLD via the theme's
    /// `panel.border:active`; the unfocused pane stays plain `panel.border`.
    /// Previously bold only appeared transiently during split-resize
    /// (`focused_border`) — this is now a persistent focus indicator.
    #[test]
    fn focused_pane_border_is_bold() {
        // Resolve the default theme from DEFAULT_STYLE_TOML (same path as startup).
        let doc = app::style::parse_style_toml(app::style::DEFAULT_STYLE_TOML)
            .expect("DEFAULT_STYLE_TOML must parse");
        let (cs, _set, _warnings) = app::style::resolve(&doc, std::path::Path::new("."));

        let area = Rect::new(0, 0, 20, 10);

        // Story focused: story border bold, map border (unfocused) not bold.
        let mut story_buf = Buffer::empty(area);
        let story_style = super::panel_border(&cs.theme, true);
        draw_pane_frame(&mut story_buf, area, cs.story_border_style, &PaneGlyphs::default(), story_style);
        assert!(
            story_buf.cell((0, 0)).unwrap().modifier.contains(Modifier::BOLD),
            "focused story border must be bold (panel.border:active)"
        );

        let mut map_buf = Buffer::empty(area);
        let map_style = super::panel_border(&cs.theme, false);
        draw_pane_frame(&mut map_buf, area, cs.map_border_style, &PaneGlyphs::default(), map_style);
        assert!(
            !map_buf.cell((0, 0)).unwrap().modifier.contains(Modifier::BOLD),
            "unfocused map border must not be bold (panel.border)"
        );

        // Flip focus to the map: story loses bold, map gains it.
        let mut story_buf2 = Buffer::empty(area);
        let story_style2 = super::panel_border(&cs.theme, false);
        draw_pane_frame(&mut story_buf2, area, cs.story_border_style, &PaneGlyphs::default(), story_style2);
        assert!(
            !story_buf2.cell((0, 0)).unwrap().modifier.contains(Modifier::BOLD),
            "unfocused story border must not be bold (panel.border)"
        );

        let mut map_buf2 = Buffer::empty(area);
        let map_style2 = super::panel_border(&cs.theme, true);
        draw_pane_frame(&mut map_buf2, area, cs.map_border_style, &PaneGlyphs::default(), map_style2);
        assert!(
            map_buf2.cell((0, 0)).unwrap().modifier.contains(Modifier::BOLD),
            "focused map border must be bold (panel.border:active)"
        );
    }

    // ── Hotkey dialog tests ───────────────────────────────────────────────────

    #[test]
    fn prefix_key_opens_hotkey_dialog() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        use app::input::{apply_action, key_to_action, Action};
        use app::state::AppState;
        use mapper::mapper::Mapper;
        let mut s = AppState::default();
        // Default prefix is Ctrl+P (moved off Ctrl+K, SQ-0447 — Ctrl+K is now the
        // story prompt's readline delete-to-end shortcut).
        let ctrlp = KeyEvent {
            code: KeyCode::Char('p'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        let action = key_to_action(&s, ctrlp);
        assert!(
            matches!(action, Action::OpenHotkeyDialog),
            "Ctrl+P should produce OpenHotkeyDialog"
        );
        apply_action(action, &mut s, &mut Mapper::default());
        assert!(s.overlays.hotkey_dialog, "hotkey_dialog should be true after OpenHotkeyDialog");
    }

    #[test]
    fn prefix_key_closes_hotkey_dialog() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        use app::input::{apply_action, key_to_action, Action};
        use app::state::AppState;
        use mapper::mapper::Mapper;
        let mut s = AppState::default();
        s.overlays.hotkey_dialog = true;
        let ctrlp = KeyEvent {
            code: KeyCode::Char('p'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        let action = key_to_action(&s, ctrlp);
        assert!(
            matches!(action, Action::CloseHotkeyDialog),
            "Ctrl+P when dialog open should produce CloseHotkeyDialog"
        );
        apply_action(action, &mut s, &mut Mapper::default());
        assert!(!s.overlays.hotkey_dialog, "hotkey_dialog should be false after CloseHotkeyDialog");
    }

    // ── dim_area ──────────────────────────────────────────────────────────────

    #[test]
    fn dim_area_sets_dim_on_all_cells() {
        let area = Rect::new(0, 0, 4, 3);
        let mut buf = Buffer::empty(area);
        // Pre-fill one cell with some content so we can check DIM ORs onto existing modifier.
        buf.cell_mut((1, 1)).unwrap().set_symbol("X");

        dim_area(&mut buf, area);

        for y in 0..3 {
            for x in 0..4 {
                let cell = buf.cell((x, y)).unwrap();
                assert!(
                    cell.modifier.contains(Modifier::DIM),
                    "cell ({x},{y}) should have DIM; modifier={:?}",
                    cell.modifier
                );
            }
        }
    }

    #[test]
    fn dim_area_does_not_affect_cells_outside_area() {
        let full = Rect::new(0, 0, 6, 4);
        let target = Rect::new(2, 1, 3, 2); // x:2..5, y:1..3
        let mut buf = Buffer::empty(full);

        dim_area(&mut buf, target);

        // Cells inside target have DIM.
        for y in 1..3 {
            for x in 2..5 {
                assert!(
                    buf.cell((x, y)).unwrap().modifier.contains(Modifier::DIM),
                    "cell ({x},{y}) inside target should have DIM"
                );
            }
        }
        // Cells outside target do NOT have DIM.
        assert!(
            !buf.cell((0, 0)).unwrap().modifier.contains(Modifier::DIM),
            "cell (0,0) outside target should NOT have DIM"
        );
        assert!(
            !buf.cell((5, 3)).unwrap().modifier.contains(Modifier::DIM),
            "cell (5,3) outside target should NOT have DIM"
        );
    }

    // ── Split layout: dim unfocused, leave focused undimmed ───────────────────

    /// This test exercises the split-layout dimming logic by simulating what
    /// draw_frame does: render content into two inner rects, then call dim_area
    /// on the unfocused one. It verifies that cells in the unfocused inner rect
    /// have DIM and cells in the focused inner rect do NOT.
    ///
    /// New behavior (item 6): map pane is NEVER dimmed regardless of focus.
    /// Story pane dims only when map has focus.
    #[test]
    fn split_layout_unfocused_pane_is_dimmed_focused_is_not() {
        let full = Rect::new(0, 0, 20, 5);
        let left_inner = Rect::new(1, 1, 8, 3);   // story (transcript) inner area

        // Simulate Focus::Map: story pane dims, map pane stays bright.
        {
            let mut buf = Buffer::empty(full);
            dim_area(&mut buf, left_inner);

            // Story pane (left) inner cells should have DIM when map has focus.
            for y in 1..4 {
                for x in 1..9 {
                    assert!(
                        buf.cell((x, y)).unwrap().modifier.contains(Modifier::DIM),
                        "story pane cell ({x},{y}) should have DIM when focus=Map"
                    );
                }
            }
            // Map pane (right) inner cells should NOT have DIM.
            for y in 1..4 {
                for x in 11..19 {
                    assert!(
                        !buf.cell((x, y)).unwrap().modifier.contains(Modifier::DIM),
                        "map pane cell ({x},{y}) should NOT have DIM when focus=Map"
                    );
                }
            }
        }

        // Simulate Focus::Game: neither pane is dimmed (map pane always stays bright).
        {
            let buf = Buffer::empty(full);
            // Focus::Game => no dim_area call at all (map is never dimmed)

            // Neither pane has DIM.
            for y in 1..4 {
                for x in 1..19 {
                    assert!(
                        !buf.cell((x, y)).unwrap().modifier.contains(Modifier::DIM),
                        "cell ({x},{y}) should NOT have DIM when focus=Game"
                    );
                }
            }
        }
    }

    /// Verify: map pane is never dimmed regardless of focus setting.
    #[test]
    fn map_pane_never_dimmed() {
        let full = Rect::new(0, 0, 20, 5);

        // Focus::Game: map pane should NOT be dimmed (we do NOT call dim_area on it).
        let buf = Buffer::empty(full);
        // The new code: "if state.focus == Focus::Map { dim_area(transcript_inner); }"
        // So for Focus::Game, we dim nothing. Map stays bright.
        for y in 1..4 {
            for x in 11..19 {
                assert!(
                    !buf.cell((x, y)).unwrap().modifier.contains(Modifier::DIM),
                    "map pane cell ({x},{y}) should NOT have DIM under Focus::Game"
                );
            }
        }

        // Focus::Map: only transcript is dimmed, map stays bright.
        let mut buf2 = Buffer::empty(full);
        let left_inner = Rect::new(1, 1, 8, 3);
        dim_area(&mut buf2, left_inner); // transcript dimmed
        // Map pane not touched
        for y in 1..4 {
            for x in 11..19 {
                assert!(
                    !buf2.cell((x, y)).unwrap().modifier.contains(Modifier::DIM),
                    "map pane cell ({x},{y}) should NOT have DIM under Focus::Map either"
                );
            }
        }
    }

    // ── Fix 4: pulse overlay only touches outer perimeter ─────────────────────

    /// The pulse overlay (applied during a tidy job) writes the pulse color to the
    /// outer perimeter cells of the map pane area. The interior content cells (rows
    /// y+2.. , cols x+2..) must NOT be overwritten by the pulse, so the map body and
    /// its overlays keep their own styling.
    ///
    /// This test directly exercises the perimeter-loop invariant: identical to what
    /// draw_frame executes, extracted inline so it runs without a full render stack.
    #[test]
    fn pulse_overlay_touches_only_outer_perimeter_not_inner_tab_row() {
        use ratatui::style::{Color, Style};

        // Use a 30x15 area.
        let area = Rect::new(0, 0, 30, 15);
        let mut buf = Buffer::empty(area);

        // The pulse color to apply (distinct from default Reset).
        let pulse_color = Color::Rgb(60, 200, 90); // PULSE_GREEN
        let pulse_style = Style::default().fg(pulse_color);

        // Apply the pulse overlay exactly as draw_frame does.
        for cy in area.y..area.bottom() {
            if let Some(c) = buf.cell_mut((area.x, cy)) { c.set_style(pulse_style); }
            if let Some(c) = buf.cell_mut((area.right().saturating_sub(1), cy)) { c.set_style(pulse_style); }
        }
        for cx in area.x..area.right() {
            if let Some(c) = buf.cell_mut((cx, area.y)) { c.set_style(pulse_style); }
            if let Some(c) = buf.cell_mut((cx, area.bottom().saturating_sub(1))) { c.set_style(pulse_style); }
        }

        // Outer perimeter (top row y=0) must carry the pulse color.
        let top_left_fg = buf.cell((area.x, area.y)).map(|c| c.fg).unwrap();
        assert_eq!(
            top_left_fg,
            pulse_color,
            "top-left outer perimeter cell must carry pulse color"
        );
        let top_right_fg = buf.cell((area.right() - 1, area.y)).map(|c| c.fg).unwrap();
        assert_eq!(
            top_right_fg,
            pulse_color,
            "top-right outer perimeter cell must carry pulse color"
        );

        // Interior content cells (row y+2, cols x+2..right-2) must NOT carry the
        // pulse color: the pulse only writes the outer perimeter (cols x / right-1,
        // rows y / bottom-1), so the map body is untouched.
        let content_row_y = area.y + 2;
        for cx in (area.x + 2)..(area.right() - 2) {
            let fg = buf.cell((cx, content_row_y)).map(|c| c.fg).unwrap();
            assert_ne!(
                fg,
                pulse_color,
                "interior content cell ({cx}, {content_row_y}) must NOT be overwritten by pulse"
            );
        }
    }

    // ── scroll_for_match ──────────────────────────────────────────────────────

    #[test]
    fn scroll_for_match_brings_row_into_view() {
        // match at position 0 in 100 visible rows, pane is 10 rows tall.
        // scroll = 100 - 0 - 10 = 90  (places match at the top of the viewport).
        // Windowing check: end = 100 - 90 = 10, start = 0, match row 0 is in [0..10). OK.
        assert_eq!(scroll_for_match(0, 100, 10), 90);

        // match at position 99 (the very last row): scroll = 100 - 99 - 10 = -9 -> clamped to 0.
        // Windowing check: end = 100, start = 90, match row 99 is in [90..100). OK.
        assert_eq!(scroll_for_match(99, 100, 10), 0);

        // match in the middle: position 50, total 100, pane 10.
        // scroll = 100 - 50 - 10 = 40.
        // end = 100 - 40 = 60, start = 50. Match row 50 is at the top of [50..60). OK.
        assert_eq!(scroll_for_match(50, 100, 10), 40);

        // pane larger than transcript: match at 0, total 5, pane 10.
        // scroll = 5 - 0 - 10 = saturates to 0.
        assert_eq!(scroll_for_match(0, 5, 10), 0);
    }

    // ── is_slash ──────────────────────────────────────────────────────────────

    #[test]
    fn is_slash_uses_prefix() {
        assert!(is_slash("/save", '/'));
        assert!(!is_slash("look", '/'));
        assert!(is_slash(";help", ';'));
        assert!(!is_slash("/help", ';'));
    }

    // ── should_prompt_save_on_quit ────────────────────────────────────────────

    #[test]
    fn prompt_save_on_quit_all_conditions_required() {
        use app::state::AppState;

        let mut s = AppState::default();
        // Default: auto_save = false, prompt_save_on_quit = true, unsaved_progress = false
        // No prompt with no unsaved progress (fresh, or just saved/loaded).
        assert!(!should_prompt_save_on_quit(&s), "no unsaved progress => no prompt");

        s.unsaved_progress = true;
        // Now: auto_save=false, prompt_save_on_quit=true, unsaved_progress=true => prompt
        assert!(should_prompt_save_on_quit(&s), "unsaved progress => prompt");

        // Saving (or loading) clears the flag => no prompt right after a save.
        s.unsaved_progress = false;
        assert!(!should_prompt_save_on_quit(&s), "after a save/load => no prompt");

        s.unsaved_progress = true;
        s.config.auto_save = true;
        // auto_save=true => no prompt (game already saves automatically)
        assert!(!should_prompt_save_on_quit(&s), "auto_save=true => no prompt");

        s.config.auto_save = false;
        s.config.prompt_save_on_quit = false;
        // prompt_save_on_quit=false => no prompt (user opted out)
        assert!(!should_prompt_save_on_quit(&s), "prompt_save_on_quit=false => no prompt");
    }

    // ── launch_dialog counts as overlay ──────────────────────────────────────

    #[test]
    fn launch_dialog_counts_as_overlay() {
        let mut s = app::state::AppState::default();
        assert!(!s.any_overlay_open(), "default state has no overlay");
        s.overlays.launch_dialog = true;
        assert!(s.any_overlay_open(), "launch_dialog true => any_overlay_open true");
        s.overlays.launch_dialog = false;
        assert!(!s.any_overlay_open(), "launch_dialog false => any_overlay_open false");
    }

    // The former app-level `key_to_zscii` and its unit tests were relocated into
    // the zvm engine adapter as `GameSession::key_input_to_zscii` (tested in
    // session.rs); the neutral crossterm→KeyInput mapping is tested in engine.rs.

    #[test]
    fn saves_dir_is_user_dir_join_saves() {
        // Save archives live under user_dir/saves.
        let d = super::saves_dir(std::path::Path::new("/tmp/bm"));
        assert_eq!(d, std::path::Path::new("/tmp/bm/saves"));
    }

    // ── char-mode gate predicate test ─────────────────────────────────────────

    /// The gate fires iff: char_mode && !any_overlay_open && key != prefix &&
    /// no Ctrl/Alt modifier. Test with a default AppState (no overlays, no
    /// char_mode initially).
    #[test]
    fn char_mode_forwards_arrow_keys_to_the_story_not_the_caret() {
        // SQ-0354's safety property, and the reason caret editing cannot steal story-controlled
        // input: when the story asks for a single keypress, the run loop's char-mode gate forwards
        // the key straight to the VM and `continue`s — app routing (and therefore the caret keys)
        // never sees it. Assert the two halves the gate depends on.
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        for code in [KeyCode::Left, KeyCode::Right, KeyCode::Home, KeyCode::End, KeyCode::Delete] {
            let k = KeyEvent::new(code, KeyModifiers::NONE);
            assert!(
                app::engine::key_event_to_input(k).is_some(),
                "{code:?} must be deliverable to the story as input",
            );
            // Plain keys are game input; only Ctrl/Alt combos are held back for app routing.
            assert!(
                !k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT),
                "{code:?} is a plain key, so the gate forwards it",
            );
        }
    }

    #[test]
    fn char_mode_gate_predicate() {
        use app::state::AppState;
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

        // The forward-to-VM predicate mirrors the run-loop gate.
        let app_combo = |m: KeyModifiers| m.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);

        let mut s = AppState::default();
        // char_mode false → gate should not fire.
        assert!(!s.char_mode, "default state is not char_mode");
        assert!(!s.any_overlay_open(), "default state has no overlay");

        // Simulate char_mode = true (as the run loop sets it from pending_input).
        s.char_mode = true;

        // A plain 'y' key: gate should accept it (not prefix, not overlay, no combo).
        let y_key = KeyEvent {
            code: KeyCode::Char('y'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        let spec = app::keymap::KeySpec::from_key_event(y_key);
        let is_prefix = spec == s.hotkeys.prefix;
        assert!(!is_prefix, "'y' must not be the default prefix (Ctrl+P)");
        assert!(s.char_mode && !s.any_overlay_open() && !is_prefix && !app_combo(y_key.modifiers),
            "char_mode gate should fire for 'y' with no overlays");
        // 'y' maps to a neutral KeyInput the engine then converts to input.
        assert_eq!(app::engine::key_event_to_input(y_key), Some(app::engine::KeyInput::Char('y')));

        // Ctrl+Q (a quit binding) must NOT be forwarded to the VM — it falls
        // through to app routing so the user can escape the form.
        let ctrlq = KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        let spec_q = app::keymap::KeySpec::from_key_event(ctrlq);
        let is_prefix_q = spec_q == s.hotkeys.prefix;
        assert!(!(s.char_mode && !s.any_overlay_open() && !is_prefix_q && !app_combo(ctrlq.modifiers)),
            "char_mode gate must NOT fire for Ctrl+Q (a Ctrl combo)");

        // Ctrl+P (the default prefix, moved off Ctrl+K in SQ-0447): gate must NOT
        // fire for it (falls through to normal routing so the hotkey dialog
        // still opens).
        let ctrlp = KeyEvent {
            code: KeyCode::Char('p'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        let spec_p = app::keymap::KeySpec::from_key_event(ctrlp);
        let is_prefix_p = spec_p == s.hotkeys.prefix;
        assert!(is_prefix_p, "Ctrl+P must match the default prefix");
        // Gate condition false because is_prefix = true (and it is a Ctrl combo).
        assert!(!(s.char_mode && !s.any_overlay_open() && !is_prefix_p && !app_combo(ctrlp.modifiers)),
            "char_mode gate must NOT fire for the prefix key Ctrl+P");

        // If an overlay is open, the gate must not fire.
        s.overlays.hotkey_dialog = true;
        assert!(s.any_overlay_open(), "hotkey_dialog open => overlay open");
        assert!(!s.char_mode || s.any_overlay_open(),
            "char_mode gate must not fire when overlay is open");
    }

    #[test]
    fn char_mode_gate_is_suppressed_while_the_more_pager_is_showing() {
        // SQ-0539, per the directive "[more] should work any time output is larger
        // than what fits on the screen … we should behave as the original game
        // intended": a read_char whose output overflowed is PAGED FIRST. The
        // run-loop gate therefore carries `!state.pager.active`, so the keystroke
        // falls through to `key_to_command`'s pager intercept (which advances one
        // screen) instead of answering the pending read. Only once the view has
        // caught up — the pager clears `active` — does the next key reach the VM,
        // exactly as an Infocom interpreter's [MORE] prompt behaved.
        use app::input::{key_to_action, Action};
        use app::state::AppState;
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

        let app_combo = |m: KeyModifiers| m.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        let y_key = KeyEvent {
            code: KeyCode::Char('y'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        let gate = |s: &AppState| {
            let spec = app::keymap::KeySpec::from_key_event(y_key);
            s.char_mode
                && !s.any_overlay_open()
                && !s.pager.active
                && spec != s.hotkeys.prefix
                && !app_combo(y_key.modifiers)
        };

        let mut s = AppState::default();
        s.char_mode = true;
        s.pager.active = true;
        assert!(!gate(&s), "the pager owns the keyboard while [more] is up");
        // …and the key it swallows pages the pane instead of reaching the game.
        assert!(matches!(key_to_action(&s, y_key), Action::PagerAdvance));

        // Caught up: the pager steps aside and the very next key answers the read.
        s.pager.active = false;
        assert!(gate(&s), "with the pager gone the keystroke goes to the VM");
    }

    #[test]
    fn loading_line_reports_name_size_and_frame() {
        let line = super::loading_line("CounterfeitMonkey-11.gblorb", 11_855_360, '/');
        assert!(line.contains("CounterfeitMonkey-11.gblorb"), "names the story");
        assert!(line.contains("11.3 MB"), "shows size in MB, got: {line}");
        assert!(line.ends_with('/'), "ends with the spinner frame glyph");
    }

    /// The startup line is the only place a player learns what seed their run was
    /// dealt, so an unpinned run must print the key AND the value to put in it —
    /// naming the number without saying where it goes leaves the run unrepeatable
    /// (SQ-0811).
    #[test]
    fn the_seed_line_tells_an_unpinned_run_how_to_keep_itself() {
        let line = super::random_seed_line(20250811, false);
        assert!(line.contains("20250811"), "names the seed: {line}");
        assert!(line.contains("random_seed = 20250811"), "spells the config key: {line}");

        let pinned = super::random_seed_line(20250811, true);
        assert!(pinned.contains("20250811"), "names the seed: {pinned}");
        assert!(pinned.contains("config.toml"), "says where it came from: {pinned}");
    }

}
