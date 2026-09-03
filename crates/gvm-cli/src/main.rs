// gvm-cli — an interactive Glulx player.
//
// Usage: gvm-cli <story.ulx | story.gblorb>
//
// Loads a raw `.ulx` Glulx image or extracts the `GLUL` executable from a Blorb,
// drives it through a Glk step loop, routing Glk output to a terminal backend
// and reading the player's input (cooked line / raw single key on a TTY), and
// prints any diagnostics to stderr.

use std::env;
use std::fs;
use std::io::{self};
use std::process;

use cli_host::{HostMode, TerminalGuard};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal;

use gvm::glk::keycode;
#[cfg(test)]
use gvm::GlkBackend;
use gvm::{Machine, Memory, StepResult};

mod glk_term;
use glk_term::TerminalBackend;

// ── key decoding ──────────────────────────────────────────────────────────────

/// Map a crossterm `KeyCode` to a Glk char-input keycode. Printable Latin-1
/// characters pass through as their code point; special keys map to the
/// `keycode::*` constants from the Glk spec.
fn decode_glk_keycode(code: KeyCode) -> u32 {
    match code {
        KeyCode::Char(c) if (c as u32) < 0x110000 => c as u32,
        KeyCode::Enter => keycode::RETURN,
        KeyCode::Backspace | KeyCode::Delete => keycode::DELETE,
        KeyCode::Tab => keycode::TAB,
        KeyCode::Esc => keycode::ESCAPE,
        KeyCode::Up => keycode::UP,
        KeyCode::Down => keycode::DOWN,
        KeyCode::Left => keycode::LEFT,
        KeyCode::Right => keycode::RIGHT,
        KeyCode::Home => keycode::HOME,
        KeyCode::End => keycode::END,
        KeyCode::PageUp => keycode::PAGE_UP,
        KeyCode::PageDown => keycode::PAGE_DOWN,
        KeyCode::F(n) if (1..=12).contains(&n) => keycode::FUNC1 - (n as u32 - 1),
        _ => keycode::UNKNOWN,
    }
}

// ── Blorb extraction ──────────────────────────────────────────────────────────

/// Get the runnable Glulx image: the `GLUL` executable inside a Blorb, or the
/// raw bytes when they aren't a Blorb (a plain `.ulx`). Test-only since `main`
/// uses [`split_story`] (which also retains the Blorb for resource streams).
#[cfg(test)]
fn extract_executable(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    if !blorb::Blorb::is_blorb(&bytes) {
        return Ok(bytes);
    }
    let b = blorb::Blorb::parse(bytes).map_err(|e| format!("Error: invalid Blorb: {e:?}"))?;
    match b.executable() {
        Ok((blorb::ExecKind::Glulx, data)) => Ok(data.to_vec()),
        Ok((blorb::ExecKind::ZCode, _)) => {
            Err("Error: this is a Z-code Blorb; run it with zvm-cli.".to_string())
        }
        Ok((blorb::ExecKind::Scott, _)) => {
            Err("Error: this is a Scott Adams Blorb; run it with lanthorn.".to_string())
        }
        Err(e) => Err(format!("Error: Blorb has no executable: {e:?}")),
    }
}

/// Split story bytes into the runnable Glulx image and, when the input was a
/// Blorb, the parsed Blorb (retained so the backend can serve `Data` resource
/// streams via `glk_stream_open_resource`). A plain `.ulx` yields no Blorb.
fn split_story(bytes: Vec<u8>) -> Result<(Vec<u8>, Option<blorb::Blorb>), String> {
    if !blorb::Blorb::is_blorb(&bytes) {
        return Ok((bytes, None));
    }
    let b = blorb::Blorb::parse(bytes).map_err(|e| format!("Error: invalid Blorb: {e:?}"))?;
    let image = match b.executable() {
        Ok((blorb::ExecKind::Glulx, data)) => data.to_vec(),
        Ok((blorb::ExecKind::ZCode, _)) => {
            return Err("Error: this is a Z-code Blorb; run it with zvm-cli.".to_string())
        }
        Ok((blorb::ExecKind::Scott, _)) => {
            return Err("Error: this is a Scott Adams Blorb; run it with lanthorn.".to_string())
        }
        Err(e) => return Err(format!("Error: Blorb has no executable: {e:?}")),
    };
    Ok((image, Some(b)))
}

// ── machine builder ───────────────────────────────────────────────────────────

/// Build a machine from story bytes, sending Glk output to `backend`. Test-only
/// (a convenience that skips Blorb retention); `main` uses [`split_story`].
#[cfg(test)]
fn build_machine(bytes: Vec<u8>, backend: Box<dyn GlkBackend>) -> Result<Machine, String> {
    let image = extract_executable(bytes)?;
    let mem = Memory::new(image).map_err(|e| format!("Error loading Glulx image: {e:?}"))?;
    Ok(Machine::with_glk(mem, backend))
}

// ── input helpers ─────────────────────────────────────────────────────────────

/// Stop cleanly at end of piped input.
///
/// The read helpers below have no access to the machine or the display, so the
/// terminal restore is the shared one (raw mode off, styling cleared, page
/// background released, cursor shape restored) with no renderer prefix. Without
/// it the shell inherited the game's background and a block cursor.
fn exit_on_eof() -> ! {
    cli_host::input::exit_at_eof("")
}

/// Read a cooked line of input from stdin (the terminal echoes it). The second
/// value is the Glk line terminator keycode; cooked stdin only ever completes a
/// line with Enter, so it is always 0 (normal termination).
///
/// At true EOF this exits rather than returning an empty line: the game would
/// otherwise be handed a blank command forever and never stop (SQ-0604) —
/// piping `/dev/null` to Kerkerkruip repeated its "no destination yet" reply
/// until the process was killed.
fn read_line_stdin() -> (String, u32) {
    match cli_host::read_line_stdin() {
        Some(line) => (line, 0),
        None => exit_on_eof(),
    }
}

/// How [`read_line_raw`] should echo the typed line.
enum LineEcho {
    /// Glk line echo is ON: echo the input (opened with this SGR, empty = plain)
    /// and leave it on screen, ending the line with CR/LF. The library owns the
    /// echo; the game does not repeat it.
    Shown(String),
    /// Glk line echo is OFF (`glk_set_echo_line_event(win, 0)`): the game echoes
    /// the command itself. Show the input live (plain) so the player isn't typing
    /// blind, then ERASE exactly what we echoed on Enter — so the game's own echo
    /// is the only copy that survives on a scrolling terminal (SQ-0282). Mirrors a
    /// redrawable Glk library, which removes echo-off input once the line ends.
    /// Carries the SGR to echo with, so live typing matches the colour the
    /// window will repaint it in (SQ-0603 follow-up).
    EraseOnEnter(String),
}

/// Read a line of input in RAW mode, echoing typed characters manually so the
/// terminal's cooked echo can't double a self-echoing game (SQ-0275). `echo`
/// selects library-echo (kept on screen) vs echo-off (shown live, then erased on
/// Enter — SQ-0282). Falls back to cooked line input on non-TTY stdin (piped
/// input has no terminal echo, so it is already correct). The terminator is
/// always 0 (normal Enter), matching the prior cooked path.
fn read_line_raw(is_tty: bool, echo: LineEcho) -> (String, u32) {
    if !is_tty {
        return read_line_stdin(); // (String, 0)
    }
    let (sgr, erase_after): (&str, bool) = match &echo {
        LineEcho::Shown(s) => (s.as_str(), false),
        LineEcho::EraseOnEnter(s) => (s.as_str(), true),
    };
    let _ = terminal::enable_raw_mode();
    let mut buf = String::new();
    // Count of visible characters we echoed, so EraseOnEnter can wipe exactly them.
    let mut echoed: usize = 0;
    if !sgr.is_empty() {
        print!("{sgr}");
        let _ = io::Write::flush(&mut io::stdout());
    }
    loop {
        match event::read() {
            Ok(Event::Resize(..)) => {} // caught by next before_input size poll
            // Only a *press* is a keystroke: Windows delivers Release events
            // too, and taking one doubles every typed character and lets the
            // Enter release submit a phantom empty command (SQ-0633).
            Ok(ev) => {
                let Some(k) = cli_host::key_press(&ev) else { continue };
                match k.code {
                    KeyCode::Enter => break,
                    // Raw mode swallows signals; exit cleanly on Ctrl-C / Ctrl-D.
                    KeyCode::Char('c') | KeyCode::Char('d')
                        if k.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        if !sgr.is_empty() { print!("\x1b[0m"); }
                        print!("\r\n"); // close the echoed line before the terminal goes back
                        let _ = io::Write::flush(&mut io::stdout());
                        // This path cannot reach the backend, so the teardown comes
                        // from the shared helper: drop the region AND park the
                        // cursor below everything. Resetting without moving left the
                        // shell prompt in the middle of the story (SQ-0913).
                        cli_host::restore_and_exit(&cli_host::leave_and_park_now(), 0);
                    }
                    KeyCode::Char(c) => {
                        buf.push(c);
                        print!("{c}");
                        echoed += 1;
                        let _ = io::Write::flush(&mut io::stdout());
                    }
                    KeyCode::Backspace
                        if buf.pop().is_some() => {
                            print!("\x08 \x08");
                            echoed = echoed.saturating_sub(1);
                            let _ = io::Write::flush(&mut io::stdout());
                        }
                    _ => {} // other special keys consumed (no on-screen garbage)
                }
            }
            _ => {}
        }
    }
    if !sgr.is_empty() { print!("\x1b[0m"); }
    if erase_after {
        // Wipe exactly the characters we echoed so the game's own echo is the only
        // copy; stay on the same line (no CR/LF) so the game's output continues
        // from wherever its prompt left the cursor.
        for _ in 0..echoed {
            print!("\x08 \x08");
        }
    } else {
        print!("\r\n"); // raw mode does not translate Enter to CRLF
    }
    let _ = terminal::disable_raw_mode();
    let _ = io::Write::flush(&mut io::stdout());
    (buf, 0)
}

/// Read one keypress. On a TTY: enter raw mode, read a crossterm `Event::Key`,
/// then restore cooked mode. Resize events during the wait are silently
/// discarded (they will be caught by the next `before_input` poll). Piped
/// stdin: return the first byte of the next line.
/// Read one keypress, and hand back the line it came from.
///
/// The line is empty in raw mode, and is the whole cooked line off a TTY —
/// screen-reader mode hands line editing to the kernel, so a "keypress" there
/// really is a line terminated by Enter. That is what lets the host recognise
/// `/menu` and a multi-digit menu jump without inventing a termination rule of
/// its own (SQ-0609); everything else still reaches the game as its first byte.
fn read_char_input(stdin_is_tty: bool) -> (u32, String) {
    if !stdin_is_tty {
        // Same EOF rule as the line path: a char request at end of input has
        // nothing left to answer with (SQ-0604).
        let Some(line) = cli_host::read_line_stdin() else { exit_on_eof() };
        let byte = line.bytes().next().unwrap_or(b'\n');
        let key = match byte {
            b'\n' | b'\r' => keycode::RETURN,
            0x7f | 0x08 => keycode::DELETE,
            b'\t' => keycode::TAB,
            0x1b => keycode::ESCAPE,
            b => b as u32,
        };
        return (key, line);
    }
    let _ = terminal::enable_raw_mode();
    let key = loop {
        match event::read() {
            Ok(Event::Resize(..)) => {} // caught by next before_input size poll
            // Press-only, as in the line reader (SQ-0633).
            Ok(ev) => {
                if let Some(k) = cli_host::key_press(&ev) {
                    // Ctrl-C / Ctrl-D: raw mode swallows signals, and without
                    // this they decode as plain 'c'/'d' — a game looping on
                    // char input could never be interrupted (SQ-0636). Same
                    // restore path as the line-input arm.
                    if k.modifiers.contains(KeyModifiers::CONTROL)
                        && matches!(k.code, KeyCode::Char('c') | KeyCode::Char('d'))
                    {
                        print!("\r\n");
                        let _ = io::Write::flush(&mut io::stdout());
                        cli_host::restore_and_exit(&cli_host::leave_and_park_now(), 0);
                    }
                    break decode_glk_keycode(k.code);
                }
            }
            _ => {}
        }
    };
    let _ = terminal::disable_raw_mode();
    (key, String::new())
}

// ── drive loop ────────────────────────────────────────────────────────────────

/// Emit an OSC 11 page-background update if the game's Normal-style background
/// changed since `last` (honor-gated, TTY-only). Called just before blocking for
/// input, so the per-instruction step loop never pays for the lookup.
fn emit_page_bg(machine: &mut Machine, honor: bool, stdout_is_tty: bool, last: &mut Option<(u8, u8, u8)>) {
    let cur = if honor && stdout_is_tty {
        // Prefer the live window colours; fall back to the by-wintype style
        // lookup for a game that sets its hints globally.
        let from_window = machine
            .backend_mut()
            .as_any_mut()
            .downcast_mut::<TerminalBackend>()
            .and_then(|t| t.page_bg());
        from_window.or_else(|| {
            machine
                .style_colour(gvm::WinType::TextBuffer, gvm::glk::GlkStyle::Normal)
                .bg
                .map(cli_host::rgb24)
        })
    } else {
        None
    };
    if let Some(esc) = cli_host::page_bg_escape(cur, *last) {
        print!("{esc}");
        let _ = io::Write::flush(&mut io::stdout());
        *last = cur;
    }
}

/// Drive `machine` to completion through the Glk step loop, pulling input from
/// the supplied readers. `before_input` runs just before each blocking read (the
/// CLI uses it to flush output so the prompt is visible). This is the shared,
/// backend-agnostic loop — the CLI wires real stdin/terminal readers; tests wire
/// scripted ones.
/// Announce the score if it moved, above the prompt the backend is holding.
///
/// Glk has no concept of a score, so unlike the Z-machine there is no number to
/// read anywhere — Glulx games write it into a grid window and that text is all
/// there is. Recovered by pattern, which fails to silence rather than to a wrong
/// figure (SQ-0616).
fn announce_score(machine: &mut Machine, watch: &mut cli_host::ScoreWatch, on: bool) {
    if !on {
        return;
    }
    if let Some(t) = machine.backend_mut().as_any_mut().downcast_mut::<TerminalBackend>() {
        let score = cli_host::score_in_status(&t.grids_plain_now());
        if let Some(line) = watch.update(score) {
            t.write_host_line(&line);
        }
    }
}

/// The answer to `/menu` when nothing is open.
const NO_MENU: &str = "[no menu is open]";

/// The terminal backend behind `machine`, when there is one (there is not, in
/// the tests that drive a scripted backend).
fn backend(machine: &mut Machine) -> Option<&mut TerminalBackend> {
    machine.backend_mut().as_any_mut().downcast_mut::<TerminalBackend>()
}

/// [`backend`], with `f` applied — the borrow-friendly spelling for the one-line
/// queries the drive loop makes.
fn with_backend<T>(machine: &mut Machine, f: impl FnOnce(&mut TerminalBackend) -> T) -> Option<T> {
    backend(machine).map(f)
}

/// Let go of the prompt the backend is holding back (plain mode — SQ-0611).
///
/// `before_input` flushes with the hold kept, so the score announcement can
/// land between the status and the prompt (SQ-0635); this is the matching
/// release, and it must run before every blocking read or host prompt —
/// otherwise the game's prompt arrives after whatever overtook it, or never.
/// Mirrors zvm-cli's `release_prompt`.
fn release_prompt(machine: &mut Machine) {
    if let Some(t) = machine.backend_mut().as_any_mut().downcast_mut::<TerminalBackend>() {
        t.release_hold();
    }
}

fn drive(
    machine: &mut Machine,
    vfs_path: &std::path::Path,
    honor: bool,
    stdout_is_tty: bool,
    announce_scores: bool,
    mut before_input: impl FnMut(&mut Machine),
    mut read_line: impl FnMut(LineEcho) -> (String, u32),
    mut read_char: impl FnMut() -> (u32, String),
) {
    // Page background: reflect the game's Normal-style bg onto the terminal's
    // default background (OSC 11), honor-gated and TTY-only. Checked only when
    // about to block for input (the screen is settled then), which keeps the
    // per-instruction step loop free of the lookup. Emits only on change; reset
    // (OSC 111) happens once at the single teardown point in `main`.
    let mut last_page_bg: Option<(u8, u8, u8)> = None;
    let mut score_watch = cli_host::ScoreWatch::new();
    // The per-game directory: `vfs_path` is always `game_dir/default.glkvfs`
    // (constructed that way in `main`, and by the tests below), so its parent
    // is the game dir — no need to thread a second path through.
    let game_dir = vfs_path.parent().unwrap_or(std::path::Path::new("."));
    // A timer-only glk_select (StepResult::NeedEvent) has no async clock in this
    // line-oriented client, so we deliver ticks synchronously. A timer-paced
    // intro converges in a handful of ticks (Kerkerkruip needs ~10); this cap —
    // reset at each real input stop — is a backstop against a game that re-arms a
    // timer and selects forever without ever reaching input.
    const MAX_CONSECUTIVE_TIMER_TICKS: u64 = 10_000;
    let mut consecutive_timer_ticks: u64 = 0;
    loop {
        // Flush the Glk file VFS to its sidecar whenever a game mutation dirtied
        // it, each iteration, so a game's files survive even if the process is
        // killed mid-session. Silently tolerate a write failure.
        if machine.vfs_dirty() {
            std::fs::create_dir_all(vfs_path.parent().unwrap_or(std::path::Path::new("."))).ok();
            let _ = fs::write(vfs_path, machine.vfs_bytes());
            machine.clear_vfs_dirty();
        }
        match machine.step() {
            StepResult::Continue => {}
            StepResult::Quit => break,
            // A glk_select waiting only on a non-input event (Glk §4.4). With a
            // timer armed, drive its clock synchronously; a bare mouse/hyperlink
            // wait has no click to deliver headless, so treat it as end-of-input.
            StepResult::NeedEvent { timer_ms, .. } => {
                if timer_ms.is_some() {
                    consecutive_timer_ticks += 1;
                    if consecutive_timer_ticks > MAX_CONSECUTIVE_TIMER_TICKS {
                        eprintln!(
                            "[timer wait exceeded {MAX_CONSECUTIVE_TIMER_TICKS} ticks without \
                             reaching input; stopping]"
                        );
                        break;
                    }
                    machine.deliver_timer();
                } else {
                    break;
                }
            }
            StepResult::NeedLine { .. } => {
                consecutive_timer_ticks = 0;
                emit_page_bg(machine, honor, stdout_is_tty, &mut last_page_bg);
                before_input(machine);
                // Status, then the announcement, THEN the released prompt — the
                // announcement is about the turn that just ended, so it belongs
                // above the prompt, matching zvm-cli (SQ-0616/0635).
                announce_score(machine, &mut score_watch, announce_scores);
                release_prompt(machine);
                // Show live typing in raw mode and erase it on Enter, then arm a
                // deferred echo: if the game reprints the command itself in
                // style_Input (Inform 7 / Counterfeit Monkey) that stands; otherwise
                // the backend echoes it so it isn't lost (e.g. sensory, which relies
                // on library echo gvm does not implement). See
                // TerminalBackend::arm_input_echo (SQ-0275 + SQ-0282).
                let echo_sgr = machine
                    .backend_mut()
                    .as_any_mut()
                    .downcast_mut::<TerminalBackend>()
                    .map(|t| t.input_echo_sgr())
                    .unwrap_or_default();
                // `/status` is answered by the host and the game never sees it,
                // so loop until a real command arrives (SQ-0610).
                let (line, terminator) = loop {
                    let r = read_line(LineEcho::EraseOnEnter(echo_sgr.clone()));
                    // The prompt has already gone out by now, so a host answer
                    // must start a fresh line and put the prompt back after
                    // itself — otherwise it lands on the prompt line, the very
                    // thing SQ-0611 fixed one layer down.
                    if cli_host::input::is_status_request(&r.0) {
                        if let Some(t) = backend(machine) {
                            let status = t.grids_plain_now();
                            let text = if status.is_empty() { "[no status]" } else { &status };
                            t.write_host_answer(text);
                        }
                        continue;
                    }
                    // `/menu` re-reads the open menu, on the `/status` precedent
                    // (SQ-0609/0610). Menus are char-driven, so this arm mostly
                    // answers the polite refusal; the useful one is at the char
                    // prompt below.
                    if cli_host::is_menu_request(&r.0) {
                        let text = with_backend(machine, |t| t.menu_listing())
                            .flatten()
                            .unwrap_or_else(|| NO_MENU.to_string());
                        if let Some(t) = backend(machine) {
                            t.write_host_answer(text.trim_end());
                        }
                        continue;
                    }
                    break r;
                };
                let cmd = line.trim_end_matches(['\n', '\r']);
                if let Some(t) =
                    machine.backend_mut().as_any_mut().downcast_mut::<TerminalBackend>()
                {
                    t.arm_input_echo(cmd.to_string());
                }
                machine.supply_line_terminated(cmd, terminator);
            }
            StepResult::NeedChar { .. } => {
                consecutive_timer_ticks = 0;
                emit_page_bg(machine, honor, stdout_is_tty, &mut last_page_bg);
                before_input(machine);
                release_prompt(machine);
                // A menu walk feeds the game its own navigation keys instead of
                // reading stdin, and steers from where the marker actually
                // landed — asked here, after `before_input` observed the frame
                // (SQ-0609).
                let key = loop {
                    if let Some(k) = with_backend(machine, |t| t.next_menu_key()).flatten() {
                        break k;
                    }
                    let (key, line) = read_char();
                    if line.is_empty() {
                        break key; // raw mode: a keystroke, not a line
                    }
                    if cli_host::is_menu_request(&line) {
                        let text = with_backend(machine, |t| t.menu_listing())
                            .flatten()
                            .unwrap_or_else(|| NO_MENU.to_string());
                        if let Some(t) = backend(machine) {
                            t.write_host_answer(text.trim_end());
                        }
                        continue;
                    }
                    match with_backend(machine, |t| t.typed_at_menu(&line)) {
                        Some(cli_host::Typed::Jump) => continue,
                        Some(cli_host::Typed::Here(said)) => {
                            if let Some(t) = backend(machine) {
                                t.write_host_answer(&said);
                            }
                            continue;
                        }
                        _ => break key,
                    }
                };
                machine.supply_char(key);
            }
            // Game create_by_prompt: ask the user for a filename (blank = cancel).
            // The prompt goes to STDOUT: it is interactive dialogue, not a
            // diagnostic, and zvm-cli already prompts there (SQ-0635).
            StepResult::NeedFilename { usage, .. } => {
                // SavedGame usage is host-intercepted (@save -> fixed default slot),
                // so don't prompt for it — auto-name and move on.
                if usage & 0x0f == 0x01 {
                    machine.supply_filename(Some(format!("__prompt_{}__", usage & 0x0f)));
                } else {
                    before_input(machine);
                    release_prompt(machine);
                    print!("Filename (blank to cancel): ");
                    let _ = io::Write::flush(&mut io::stdout());
                    let (line, _) = read_line(LineEcho::Shown(String::new()));
                    let name = line.trim_end_matches(['\n', '\r']);
                    machine.supply_filename(if name.is_empty() { None } else { Some(name.to_string()) });
                }
            }
            // @save. The game's OWN fixed-name saves (create_by_name: CM's init
            // cache, undo, autotesting) are serviced SILENTLY against a fixed
            // path in game_dir — no prompt. Only the player's SAVE verb
            // (create_by_prompt) prompts for a filename.
            StepResult::SaveRequest => {
                let req = machine.pending_saveload_request().unwrap_or_default();
                if req.by_prompt || req.name.is_empty() {
                    before_input(machine);
                    release_prompt(machine);
                    // The saves already here, as a reminder of what you would
                    // collide with. A number is NOT accepted at a save prompt — see
                    // `cli_host::pick_save` (SQ-0918).
                    if let Some(l) = cli_host::save_list_line(&cli_host::existing_saves(game_dir, cli_host::QUETZAL_EXT)) {
                        println!("{l}");
                    }
                    print!("Save to file: ");
                    let _ = io::Write::flush(&mut io::stdout());
                    let (line, _) = read_line(LineEcho::Shown(String::new()));
                    let name = line.trim_end_matches(['\n', '\r']);
                    if name.is_empty() {
                        machine.complete_save(false);
                    } else {
                        let path = cli_host::resolve_save_input(name, game_dir, cli_host::QUETZAL_EXT);
                        // Same guard as zvm-cli: `fs::write` is unconditional, so a
                        // repeated name would silently destroy the earlier save
                        // (SQ-0648's defect, SQ-0918).
                        let mut proceed = true;
                        if let Some(warning) = cli_host::overwrite_warning(&path) {
                            print!("{warning}");
                            let _ = io::Write::flush(&mut io::stdout());
                            let (answer, _) = read_line(LineEcho::Shown(String::new()));
                            proceed = cli_host::is_yes(&answer);
                        }
                        if !proceed {
                            eprintln!("[save cancelled]");
                            machine.complete_save(false);
                            continue;
                        }
                        std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new("."))).ok();
                        let ok = fs::write(&path, machine.save_quetzal()).is_ok();
                        if ok {
                            eprintln!("[saved to {}]", path.display());
                        } else {
                            eprintln!("[save failed]");
                        }
                        machine.complete_save(ok);
                    }
                } else {
                    // Silent game-managed save: <game_dir>/<fileref name>.qzl.
                    let path = game_auto_save_path(game_dir, &req.name);
                    std::fs::create_dir_all(game_dir).ok();
                    let ok = fs::write(&path, machine.save_quetzal()).is_ok();
                    machine.complete_save(ok);
                }
            }
            // @restore. Symmetric: the game's own saves read a fixed file
            // silently (clean-failing when it's absent — the first run — so the
            // game runs its init); only the player's RESTORE verb prompts.
            StepResult::RestoreRequest => {
                let req = machine.pending_saveload_request().unwrap_or_default();
                if req.by_prompt || req.name.is_empty() {
                    before_input(machine);
                    release_prompt(machine);
                    let saves = cli_host::existing_saves(game_dir, cli_host::QUETZAL_EXT);
                    if let Some(l) = cli_host::save_list_line(&saves) {
                        println!("{l}");
                    }
                    print!("Restore from file: ");
                    let _ = io::Write::flush(&mut io::stdout());
                    let (line, _) = read_line(LineEcho::Shown(String::new()));
                    let typed = line.trim_end_matches(['\n', '\r']);
                    // A number picks from the list; anything else is a filename
                    // exactly as before.
                    let name = cli_host::pick_save(typed, &saves).unwrap_or(typed);
                    if name.is_empty() {
                        machine.complete_restore_failure();
                    } else {
                        let path = cli_host::resolve_save_input(name, game_dir, cli_host::QUETZAL_EXT);
                        match fs::read(&path) {
                            Ok(bytes) if machine.complete_restore_quetzal(&bytes) => {
                                eprintln!("[restored from {}]", path.display());
                            }
                            _ => {
                                eprintln!("[restore failed]");
                                machine.complete_restore_failure();
                            }
                        }
                    }
                } else {
                    // Silent game-managed restore: read the fixed file if present.
                    let path = game_auto_save_path(game_dir, &req.name);
                    match fs::read(&path) {
                        Ok(bytes) if machine.complete_restore_quetzal(&bytes) => {}
                        _ => machine.complete_restore_failure(),
                    }
                }
            }
        }
    }
}

// ── story-key / per-game save resolution ────────────────────────────────────

/// The fixed path for a game-managed (create_by_name) save slot:
/// `<game_dir>/<sanitized fileref name>.qzl`. `name` is already sanitized by the
/// VM (gvm's `sanitize_fileref_name`), so `@save` and `@restore` on the same
/// fileref name resolve to the same file across launches — the payoff that lets
/// a game skip its init on relaunch. These names are game-internal (CM's begin
/// with `_`), keeping them clear of the player's `<name>.qzl` saves.
fn game_auto_save_path(game_dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    // The named constant rather than a literal, so this cannot drift from what
    // `cli_host::existing_saves` scans for — the two have to agree, and SQ-0919
    // made the extension a parameter precisely because it is a claim about the
    // bytes. These really are Quetzal.
    game_dir.join(format!("{name}.{}", cli_host::QUETZAL_EXT))
}

/// Seed the machine's host-managed SavedGame existence index from every
/// `<game_dir>/*.qzl` on disk (raw basename minus the `.qzl` suffix, matching
/// [`game_auto_save_path`] and how the index is keyed), so a `create_by_name`
/// game probing `glk_fileref_does_file_exist` before `@restore` sees its save
/// across launches (SQ-0301). No-op when `game_dir` is unreadable (e.g. a first
/// run before it exists). Over-seeding player-save `.qzl` names is inert.
fn seed_saved_games(machine: &mut Machine, game_dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(game_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some(cli_host::QUETZAL_EXT) {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0) as u32;
        machine.seed_saved_game_file(name.to_string(), size);
    }
}

// ── main ──────────────────────────────────────────────────────────────────────

const HELP: &str = "\
gvm-cli — DOS-style Glulx player (no map)

Usage: gvm-cli [OPTIONS] <story>

Arguments:
  <story>               Glulx story (.ulx) or Blorb (.gblorb)

Host commands (never passed to the game):
  /status               Repeat the current Glk grid windows (status line)
  /menu                 Re-read the open menu, host-numbered. In --screen-reader
                        mode a menu keypress is a whole typed line, so /menu —
                        and a bare item number, which jumps to that item — work
                        at a menu's own prompt as well as at a line prompt.

Options:
      --screen-reader   Linear plain text (alias: --plain; also selected by
                        TERM=dumb). Emits no escape sequences at all — no
                        colour, no cursor addressing, no window rects — and
                        hands line editing and echo back to the terminal. Grid
                        windows stream inline as text instead of being painted
                        in place. The status bar is not narrated every turn (see
                        --show-status); menus still are. Ask for the status any
                        time with /status. A menu that repaints as its marker
                        moves is announced in one line rather than re-read (see
                        /menu above).
      --story-only      Show only the story window: suppress every grid window,
                        menus and forms included. Stronger than what
                        --screen-reader does to the status bar, and independent
                        of it.
      --show-status     Narrate the status bar whenever the story updates it,
                        undoing --screen-reader's quietening.
      --pin <top|bottom>
                        Where stacked grid windows (the status bar, a menu) are
                        pinned. Default top, where the game asked for them.

                        Pinning at the BOTTOM is what gets you terminal
                        scrollback: a terminal only archives a line that scrolls
                        off the TOP of the screen, judged by the scroll region's
                        top margin, so chrome at the top means nothing ever
                        leaves and every line you have read is discarded.
                        Honoured for the ordinary layout — grids stacked above
                        one full-width story window — and ignored for anything
                        Glk arranges differently, such as a side-by-side split
                        or a graphics window, which have no top and bottom to
                        swap.
      --scrollback      Alias for --pin bottom, named for what it is for.
      --game-colours <on|off>
                        Honour the game's Glk stylehint colours. Default on;
                        NO_COLOR turns it off too. (alias: --game-colors)
      --pager <on|off>  [MORE] paging on long output. Default on, and off
                        wherever it could not work anyway: --screen-reader, or a
                        piped stdout.
      --accel <on|off>  Glulx accelerated-function interception. Default on; off
                        is for debugging a game that misbehaves under it.
      --data-dir <path> Base dir for saves/sidecars (default: beside the story)
  -V, --version         Print version and exit
  -h, --help            Print this help and exit
";

/// Every option `gvm-cli` accepts; `cli_host::args` applies the rules.
const OPTS: &[cli_host::Opt] = &[
    // SQ-1082: `--<noun> on|off`, the spelling `zvm-cli` and `lanthorn` use. One
    // concept under two names across three binaries is the drift SQ-1078 existed
    // to remove, and these were `--no-game-colours` / `--no-accel` / `--no-more`.
    cli_host::Opt::valued(&["--game-colours", "--game-colors"]),
    cli_host::Opt::valued(&["--accel"]),
    cli_host::Opt::valued(&["--pager"]),
    cli_host::Opt::flag(&["--story-only"]),
    cli_host::Opt::flag(&["--show-status"]),
    cli_host::Opt::flag(&["--screen-reader", "--plain"]),
    cli_host::Opt::flag(&["--help", "-h"]),
    cli_host::Opt::flag(&["--version", "-V"]),
    cli_host::Opt::valued(&["--data-dir"]),
    cli_host::Opt::valued(&["--pin"]),
    cli_host::Opt::flag(&["--scrollback"]),
];

fn main() {
    let argv: Vec<String> = env::args().collect();
    if cli_host::handled_common_flags(&argv, HELP, env!("CARGO_BIN_NAME"), buildinfo::LONG) {
        return;
    }
    // Resolve the terminal mode FIRST: `TerminalBackend::new` below reads the
    // installed mode to decide whether it may emit escapes, so `--plain` has to
    // be known before the backend exists (SQ-0606).
    let mode = HostMode::detect_with(cli_host::plain_requested(&argv)).install();
    let name = env!("CARGO_BIN_NAME");
    let m = match cli_host::scan(&argv, OPTS) {
        Ok(m) => m,
        Err(e) => cli_host::usage_error(name, &e, HELP),
    };
    if m.positional.len() > 1 {
        cli_host::usage_error(
            name,
            &format!("unexpected extra argument: {}", m.positional[1]),
            HELP,
        );
    }
    // Honour the game's stylehint colours by default; `--game-colours off` opts
    // out (mirrors zvm-cli), as does NO_COLOR — which is about colour only, not
    // layout.
    // SQ-1082: each switch is `on`/`off` with a default of on, and a value that
    // is neither is a usage error rather than a silent fallback.
    let on_off = |flag: &str| match cli_host::on_off(flag, m.value(flag)) {
        Ok(v) => v.unwrap_or(true),
        Err(e) => cli_host::usage_error(name, &e, HELP),
    };
    let honor = on_off("--game-colours") && !cli_host::no_color();
    // Acceleration (Glulx accelfunc interception) is on by default.
    let accel = on_off("--accel");
    // Resolved HERE and not where the pager is built, so a bad value is a usage
    // error before the story is read rather than after: `--pager bogus x.ulx`
    // must complain about `bogus`, not about `x.ulx`.
    let pager = on_off("--pager");
    let data_dir: Option<String> = m.value("--data-dir").map(str::to_string);
    let Some(path) = m.first_positional().map(str::to_string) else {
        cli_host::usage_error(name, "no story file given", HELP);
    };
    let path = &path;

    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error: cannot read '{path}': {e}");
            process::exit(1);
        }
    };

    let (image, blorb) = match split_story(bytes) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            process::exit(1);
        }
    };
    let mem = match Memory::new(image) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("Error loading Glulx image: {e:?}");
            process::exit(1);
        }
    };
    let mut backend = TerminalBackend::new();
    backend.set_honor_colours(honor);
    // Plain mode does not narrate a one-row status grid every turn unless asked
    // (SQ-0612); taller grids — menus, forms — always come through.
    backend.set_quiet_status_line(mode.plain() && !m.has("--show-status"));
    backend.set_story_only(m.has("--story-only"));
    // `--pin bottom` (alias `--scrollback`) puts stacked chrome at the bottom, which
    // is what lets the terminal keep history — see `cli_host::pin`. Honoured only
    // for the ordinary stacked layout; anything Glk arranges differently stays where
    // it was put (`glk_term::swap_bands`). A bad value falls back to the default
    // rather than refusing to start, as zvm-cli's does.
    backend.set_pin(
        m.value("--pin")
            .and_then(cli_host::Pin::parse)
            .unwrap_or(if m.has("--scrollback") { cli_host::Pin::Bottom } else { cli_host::Pin::Top }),
    );
    // Screen-reader mode only: on a TTY the menu is painted in place and nothing
    // repeats, and a plain pipe is a transcript that must stay byte-identical
    // (SQ-0609).
    backend.set_menus(mode.plain() && !m.has("--story-only"));
    // Paging needs both ends to be a terminal (a pipe would never answer the
    // prompt) and is off in screen-reader mode by choice — a blocking prompt
    // hiding the rest of the output is the worst shape for a reader (SQ-0617).
    let interactive = mode.both_tty();
    let paging = interactive && pager && !mode.plain();
    backend.set_paging(paging, interactive);
    backend.set_data_blorb(blorb);
    let mut machine = Machine::with_glk(mem, Box::new(backend));
    machine.set_acceleration(accel);

    // Resolved above, before the backend was built.
    let stdin_is_tty = mode.raw_input();
    let stdout_is_tty = mode.rich();
    let both_tty = mode.both_tty();

    // Restores the terminal on every way out of `main`, including a panic. The
    // explicit `restore` below runs after the backend drops its own display
    // state; this just guarantees it happens at all.
    let mut guard = TerminalGuard::new();

    // Force a steady block cursor (SQ-0281); reset to the terminal default on exit.
    if stdout_is_tty {
        print!("{}", cli_host::cursor_steady_block());
        let _ = io::Write::flush(&mut io::stdout());
    }

    // Track the last-known terminal size so we can detect resize events.
    // Re-poll before each input using crossterm::terminal::size(); when
    // changed, update the backend and queue a Glk evtype_Arrange event so
    // the game can re-lay out its windows.
    let mut last_size: Option<(u32, u32)> = if both_tty {
        terminal::size().ok().map(|(c, r)| (c as u32, r as u32))
    } else {
        None
    };

    let story_path = std::path::PathBuf::from(path);
    let game_dir = cli_host::game_dir(&story_path, data_dir.as_deref());

    // The Glk file VFS sidecar: `<game_dir>/default.glkvfs`. Loaded here before
    // the run, flushed dirty-gated inside `drive`, so a game's external files
    // (scores, preferences) survive a plain quit-and-relaunch.
    let vfs_path = game_dir.join("default.glkvfs");
    if vfs_path.exists() {
        // Tolerate a missing/unreadable sidecar silently — it just means empty.
        if let Ok(bytes) = fs::read(&vfs_path) {
            machine.load_vfs(&bytes);
        }
        machine.clear_vfs_dirty(); // loading is not a game mutation
    }

    // Reseed host-managed SavedGame slots from disk before the first drive: the
    // machine's existence index is session-transient, so a create_by_name game
    // probing glk_fileref_does_file_exist during init would otherwise never see
    // its own on-disk `.qzl` save across launches (SQ-0301).
    seed_saved_games(&mut machine, &game_dir);

    drive(
        &mut machine,
        &vfs_path,
        honor,
        stdout_is_tty,
        mode.plain() && !m.has("--story-only"),
        |m| {
            // Re-poll terminal size before each input (interactive TTY only).
            if both_tty {
                if let Ok((cols, rows)) = terminal::size() {
                    let new_size = (cols as u32, rows as u32);
                    if last_size != Some(new_size) {
                        last_size = Some(new_size);
                        let changed = m
                            .backend_mut()
                            .as_any_mut()
                            .downcast_mut::<TerminalBackend>()
                            .map(|b| b.update_size(cols as u32, rows as u32))
                            .unwrap_or(false);
                        if changed {
                            m.notify_resize();
                        }
                    }
                }
            }
            // Flush pending output (without tearing down the scroll region) so
            // the prompt is visible before we block for input. The hold is kept:
            // `drive` releases it itself, after any score announcement, so the
            // announcement lands above the prompt (SQ-0635).
            if let Some(t) = m.backend_mut().as_any_mut().downcast_mut::<TerminalBackend>() {
                t.flush_out_holding();
            }
        },
        move |echo| read_line_raw(stdin_is_tty, echo),
        move || read_char_input(stdin_is_tty),
    );

    machine.flush();
    // Leave raw mode BEFORE the backend's teardown: `leave_display` ends with a
    // newline to park the cursor, and in raw mode that would be a bare LF with
    // no carriage return.
    cli_host::end_raw_mode();
    // Drop the scroll region, clear styling, and put the cursor below the last
    // painted row so the shell prompt returns under the display rather than
    // inside it.
    if let Some(t) = machine.backend_mut().as_any_mut().downcast_mut::<TerminalBackend>() {
        t.leave_display();
    }
    // The single teardown point — it covers normal quit and the fault/exit(70)
    // path below, and `drive()` never resets itself so there is no double-reset.
    // The page-background release is no longer honor-gated: OSC 111 restores the
    // terminal's own default, so sending it when no colour was honoured is a
    // no-op, and the Ctrl-C path had already reached that conclusion.
    guard.restore(&cli_host::leave_and_park_now());

    if let Some(trace) = machine.take_fault_trace() {
        for line in trace.to_lines() {
            eprintln!("{line}");
        }
        // Still surface any other diagnostics, then exit non-zero.
        for d in &machine.diagnostics {
            eprintln!("gvm: {d}");
        }
        std::process::exit(70);
    }

    for d in &machine.diagnostics {
        eprintln!("gvm: {d}");
    }
}

#[cfg(test)]
mod stdin_eof_tests {
    // The implementation moved to `cli_host::input` (SQ-0605), but gvm-cli keeps
    // its own regression test: this is the crate the bug actually shipped in,
    // and a test here fails if the dependency is ever swapped for something that
    // reads EOF the old way.
    use cli_host::read_line_or_eof;

    /// SQ-0604: gvm-cli ignored `read_line`'s result, so a closed stdin handed
    /// the game an empty command over and over and the process never stopped —
    /// piping /dev/null to Kerkerkruip repeated its "no destination yet" reply
    /// until it was killed. A 0-byte read is EOF and must be reported as such.
    #[test]
    fn true_eof_is_reported_rather_than_read_as_a_blank_command() {
        let mut empty: &[u8] = b"";
        assert_eq!(read_line_or_eof(&mut empty), None);
    }

    /// The distinction that makes it safe: a blank line is a real, empty
    /// command and still yields a byte.
    #[test]
    fn a_blank_line_is_not_eof() {
        let mut input: &[u8] = b"\n";
        assert_eq!(read_line_or_eof(&mut input).as_deref(), Some("\n"));
    }

    #[test]
    fn a_line_is_returned_whole_and_then_input_ends() {
        let mut input: &[u8] = b"look\n";
        assert_eq!(read_line_or_eof(&mut input).as_deref(), Some("look\n"));
        assert_eq!(read_line_or_eof(&mut input), None, "the stream is exhausted");
    }

    /// A final line with no trailing newline is still a command, not EOF.
    #[test]
    fn an_unterminated_last_line_is_still_input() {
        let mut input: &[u8] = b"quit";
        assert_eq!(read_line_or_eof(&mut input).as_deref(), Some("quit"));
        assert_eq!(read_line_or_eof(&mut input), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gvm::TestBackend;

    /// A hand-assembled Glulx image: open a TextBuffer window, make it current,
    /// print "Hi", then quit. Start function at 0x24 with one 4-byte local.
    ///
    /// Operand mode nibbles are packed low-nibble-first; opcodes ≥ 0x80 use the
    /// 2-byte form `opcode + 0x8000`. (See `crates/gvm/asm.rs` for the encoder
    /// the in-crate tests use; this crate hand-encodes one tiny program.)
    fn hi_image() -> Vec<u8> {
        let func: Vec<u8> = vec![
            0xC1, 0x04, 0x01, 0x00, 0x00, // type C1; one 4-byte local; terminator
            // setiosys 2, 0   (opcode 0x149 → 81 49; modes C8,C8)
            0x81, 0x49, 0x11, 0x02, 0x00,
            // push window_open args (last pushed is popped first = args[0]=split):
            //   rock=0, wintype=3, size=0, method=0, split=0
            0x40, 0x80, // copy const0 -> push (rock)
            0x40, 0x81, 0x03, // copy C8(3) -> push (wintype TextBuffer)
            0x40, 0x80, // size 0
            0x40, 0x80, // method 0
            0x40, 0x80, // split 0
            // @glk glk_window_open (sel 0x23, argc 5) -> local0
            0x81, 0x30, 0x11, 0x09, 0x23, 0x05, 0x00,
            // push local0; @glk glk_set_window (sel 0x2F, argc 1), discard
            0x40, 0x89, 0x00, // copy local0 -> push
            0x81, 0x30, 0x11, 0x00, 0x2F, 0x01,
            // streamchar 'H'; streamchar 'i'  (opcode 0x70; mode C8)
            0x70, 0x01, b'H',
            0x70, 0x01, b'i',
            // quit (0x120 → 81 20)
            0x81, 0x20,
        ];
        let (ramstart, endmem) = (0x100u32, 0x200u32);
        let mut img = vec![0u8; ramstart as usize];
        img[0..4].copy_from_slice(b"Glul");
        img[0x04..0x08].copy_from_slice(&0x0003_0102u32.to_be_bytes());
        img[0x08..0x0C].copy_from_slice(&ramstart.to_be_bytes());
        img[0x0C..0x10].copy_from_slice(&ramstart.to_be_bytes()); // EXTSTART == RAMSTART
        img[0x10..0x14].copy_from_slice(&endmem.to_be_bytes());
        img[0x14..0x18].copy_from_slice(&0x1000u32.to_be_bytes());
        img[0x18..0x1C].copy_from_slice(&0x24u32.to_be_bytes());
        img[0x24..0x24 + func.len()].copy_from_slice(&func);
        img
    }

    // ── minimal Blorb builder (mirrors the blorb crate's test helper) ─────────
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

    /// A Blorb wrapping a single Exec resource of the given chunk type + data.
    fn build_blorb(exec_type: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let ridx_data_len = 4 + 12; // count + one 12-byte entry
        let first_res_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
        let exec_chunk = chunk(exec_type, data);

        let mut ridx = Vec::new();
        ridx.extend_from_slice(&1u32.to_be_bytes()); // count
        ridx.extend_from_slice(b"Exec");
        ridx.extend_from_slice(&0u32.to_be_bytes()); // number
        ridx.extend_from_slice(&(first_res_off as u32).to_be_bytes());
        let ridx_chunk = chunk(b"RIdx", &ridx);

        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&ridx_chunk);
        inner.extend_from_slice(&exec_chunk);
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        file
    }

    /// Run `bytes` against a [`TestBackend`] and return the buffer-window text.
    fn run_capturing(bytes: Vec<u8>) -> String {
        let mut m = build_machine(bytes, Box::new(TestBackend::new())).unwrap();
        m.run();
        m.backend_mut()
            .as_any_mut()
            .downcast_mut::<TestBackend>()
            .unwrap()
            .all_text()
    }

    #[test]
    fn runs_raw_ulx_to_completion() {
        assert_eq!(run_capturing(hi_image()), "Hi");
    }

    #[test]
    fn extracts_and_runs_glulx_blorb() {
        let blorb = build_blorb(b"GLUL", &hi_image());
        assert!(blorb::Blorb::is_blorb(&blorb));
        assert_eq!(run_capturing(blorb), "Hi");
    }

    #[test]
    fn rejects_zcode_blorb() {
        let zblorb = build_blorb(b"ZCOD", b"fake z-code");
        let err = match build_machine(zblorb, Box::new(TestBackend::new())) {
            Err(e) => e,
            Ok(_) => panic!("expected the Z-code Blorb to be rejected"),
        };
        assert!(err.contains("Z-code"), "got: {err}");
    }

    #[test]
    fn passes_through_non_blorb_bytes() {
        let img = hi_image();
        assert_eq!(extract_executable(img.clone()).unwrap(), img);
    }

    // ── key decoding tests ────────────────────────────────────────────────────

    // ── story-key / per-game save resolution ──────────────────────────────────


    #[test]
    fn game_auto_save_path_is_fixed_named_in_game_dir() {
        use std::path::{Path, PathBuf};
        // A game-managed (create_by_name) slot resolves to a fixed
        // <game_dir>/<name>.qzl — same path across launches, so a relaunch's
        // @restore finds it (init-skip). Names are pre-sanitized by the VM.
        let gd = Path::new("/data/CM.gblorb.save");
        assert_eq!(
            game_auto_save_path(gd, "_Counterfeit_Monkey-startup-data"),
            PathBuf::from("/data/CM.gblorb.save/_Counterfeit_Monkey-startup-data.qzl"),
        );
    }

    #[test]
    fn vfs_path_is_default_glkvfs_in_game_dir() {
        use std::path::{Path, PathBuf};
        let gd = Path::new("/data/Advent.gblorb");
        assert_eq!(gd.join("default.glkvfs"), PathBuf::from("/data/Advent.gblorb/default.glkvfs"));
    }

    #[test]
    fn decode_glk_keycode_maps_crossterm_keys() {
        assert_eq!(decode_glk_keycode(KeyCode::Char('a')), b'a' as u32);
        assert_eq!(decode_glk_keycode(KeyCode::Char('7')), b'7' as u32);
        assert_eq!(decode_glk_keycode(KeyCode::Enter), keycode::RETURN);
        assert_eq!(decode_glk_keycode(KeyCode::Backspace), keycode::DELETE);
        assert_eq!(decode_glk_keycode(KeyCode::Delete), keycode::DELETE);
        assert_eq!(decode_glk_keycode(KeyCode::Tab), keycode::TAB);
        assert_eq!(decode_glk_keycode(KeyCode::Esc), keycode::ESCAPE);
        assert_eq!(decode_glk_keycode(KeyCode::Up), keycode::UP);
        assert_eq!(decode_glk_keycode(KeyCode::Down), keycode::DOWN);
        assert_eq!(decode_glk_keycode(KeyCode::Left), keycode::LEFT);
        assert_eq!(decode_glk_keycode(KeyCode::Right), keycode::RIGHT);
        assert_eq!(decode_glk_keycode(KeyCode::Home), keycode::HOME);
        assert_eq!(decode_glk_keycode(KeyCode::End), keycode::END);
        assert_eq!(decode_glk_keycode(KeyCode::PageUp), keycode::PAGE_UP);
        assert_eq!(decode_glk_keycode(KeyCode::PageDown), keycode::PAGE_DOWN);
        assert_eq!(decode_glk_keycode(KeyCode::F(1)), keycode::FUNC1);
        assert_eq!(decode_glk_keycode(KeyCode::F(12)), keycode::FUNC12);
        assert_eq!(decode_glk_keycode(KeyCode::F(13)), keycode::UNKNOWN, "F13 → unknown");
    }

    // A tiny Glulx instruction encoder for the input integration programs.
    // Immediates use the 4-byte constant mode, avoiding sign-extension pitfalls.
    #[derive(Clone, Copy)]
    enum E {
        Imm(u32),
        LocLoad(u8),
        LocStore(u8),
        MemLoad(u32),
        Push,
        Discard,
    }
    fn emode(e: E) -> u8 {
        match e {
            E::Imm(_) => 3,
            E::LocLoad(_) | E::LocStore(_) => 9,
            E::MemLoad(_) => 7,
            E::Push => 8,
            E::Discard => 0,
        }
    }
    fn edata(e: E) -> Vec<u8> {
        match e {
            E::Imm(v) | E::MemLoad(v) => v.to_be_bytes().to_vec(),
            E::LocLoad(o) | E::LocStore(o) => vec![o],
            E::Push | E::Discard => vec![],
        }
    }
    fn enc(op: u32, args: &[E]) -> Vec<u8> {
        let mut out = Vec::new();
        if op <= 0x7f {
            out.push(op as u8);
        } else {
            out.extend_from_slice(&((op | 0x8000) as u16).to_be_bytes());
        }
        let mut modes = vec![0u8; args.len().div_ceil(2)];
        for (i, &a) in args.iter().enumerate() {
            let m = emode(a);
            if i % 2 == 0 {
                modes[i / 2] |= m;
            } else {
                modes[i / 2] |= m << 4;
            }
        }
        out.extend_from_slice(&modes);
        for &a in args {
            out.extend(edata(a));
        }
        out
    }

    /// Wrap `body` as a one-local start function and a runnable image (RAMSTART
    /// 0x100, ENDMEM 0x200 → RAM [0x100, 0x200) for the event struct + buffers).
    fn image_for(body: Vec<u8>) -> Vec<u8> {
        let mut func = vec![0xC1u8, 0x04, 0x01, 0x00, 0x00]; // type C1; one 4-byte local
        func.extend(body);
        let (ramstart, endmem) = (0x100u32, 0x200u32);
        let mut img = vec![0u8; ramstart as usize];
        img[0..4].copy_from_slice(b"Glul");
        img[0x04..0x08].copy_from_slice(&0x0003_0102u32.to_be_bytes());
        img[0x08..0x0C].copy_from_slice(&ramstart.to_be_bytes());
        img[0x0C..0x10].copy_from_slice(&ramstart.to_be_bytes());
        img[0x10..0x14].copy_from_slice(&endmem.to_be_bytes());
        img[0x14..0x18].copy_from_slice(&0x1000u32.to_be_bytes());
        img[0x18..0x1C].copy_from_slice(&0x24u32.to_be_bytes());
        img[0x24..0x24 + func.len()].copy_from_slice(&func);
        img
    }

    /// Open a TextBuffer (id stored in local0) and make it current.
    fn open_buffer_prelude() -> Vec<u8> {
        use E::*;
        let mut b = enc(0x149, &[Imm(2), Imm(0)]); // setiosys glk
        for v in [Imm(0), Imm(3), Imm(0), Imm(0), Imm(0)] {
            b.extend(enc(0x40, &[v, Push])); // rock, wintype=3, size, method, split
        }
        b.extend(enc(0x130, &[Imm(0x23), Imm(5), LocStore(0)])); // window_open → local0
        b.extend(enc(0x40, &[LocLoad(0), Push]));
        b.extend(enc(0x130, &[Imm(0x2f), Imm(1), Discard])); // set_window(local0)
        b
    }

    #[test]
    fn drive_echoes_a_typed_character() {
        use E::*;
        // request_char_event(local0); select(@0x100); put_char(event.val1 @0x108).
        let mut body = open_buffer_prelude();
        body.extend(enc(0x40, &[LocLoad(0), Push]));
        body.extend(enc(0x130, &[Imm(0xd2), Imm(1), Discard])); // request_char_event
        body.extend(enc(0x40, &[Imm(0x100), Push]));
        body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select(@0x100)
        body.extend(enc(0x40, &[MemLoad(0x108), Push])); // the key code (val1)
        body.extend(enc(0x130, &[Imm(0x80), Imm(1), Discard])); // glk_put_char(key)
        body.extend(enc(0x120, &[])); // quit

        let mut m = build_machine(image_for(body), Box::new(TestBackend::new())).unwrap();
        let mut keys = vec![b'Z' as u32].into_iter();
        drive(&mut m, std::path::Path::new("unused.glkvfs"), true, false, false, |_| {}, |_echo| (String::new(), 0), move || (keys.next().unwrap_or(keycode::RETURN), String::new()));
        let text = m.backend_mut().as_any_mut().downcast_mut::<TestBackend>().unwrap().all_text();
        assert_eq!(text, "Z", "the typed key was supplied, stored, and echoed");
    }

    #[test]
    fn drive_reads_and_prints_a_line() {
        use E::*;
        // request_line_event(local0, buf=0x180, maxlen=20, initlen=0); select(@0x100);
        // put_buffer(0x180, event.val1 @0x108).
        let mut body = open_buffer_prelude();
        for v in [Imm(0), Imm(20), Imm(0x180), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push])); // initlen, maxlen, buf, win
        }
        body.extend(enc(0x130, &[Imm(0xd0), Imm(4), Discard])); // request_line_event
        body.extend(enc(0x40, &[Imm(0x100), Push]));
        body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select(@0x100)
        body.extend(enc(0x40, &[MemLoad(0x108), Push])); // len = val1
        body.extend(enc(0x40, &[Imm(0x180), Push])); // addr
        body.extend(enc(0x130, &[Imm(0x84), Imm(2), Discard])); // glk_put_buffer(addr, len)
        body.extend(enc(0x120, &[])); // quit

        let mut m = build_machine(image_for(body), Box::new(TestBackend::new())).unwrap();
        let mut lines = vec!["hello".to_string()].into_iter();
        drive(&mut m, std::path::Path::new("unused.glkvfs"), true, false, false, |_| {}, move |_echo| (lines.next().unwrap_or_default(), 0), || (keycode::RETURN, String::new()));
        let text = m.backend_mut().as_any_mut().downcast_mut::<TestBackend>().unwrap().all_text();
        assert_eq!(text, "hello", "the typed line was supplied into the buffer and printed");
    }

    // ── .glkvfs sidecar persistence ───────────────────────────────────────────

    /// Wrap `body` as a start function with `nlocals` 4-byte locals, and place the
    /// NUL-terminated fileref name `"gvfstest"` in ROM at 0x1C0. RAMSTART is 0x200
    /// (Glulx requires the memory-map bounds to be 256-aligned) so the file
    /// programs' code has room below the name string.
    fn vfs_image(body: Vec<u8>, nlocals: u8) -> Vec<u8> {
        let mut func = vec![0xC1u8, 0x04, nlocals, 0x00, 0x00];
        func.extend(body);
        let (ramstart, endmem) = (0x200u32, 0x300u32);
        let mut img = vec![0u8; ramstart as usize];
        img[0..4].copy_from_slice(b"Glul");
        img[0x04..0x08].copy_from_slice(&0x0003_0102u32.to_be_bytes());
        img[0x08..0x0C].copy_from_slice(&ramstart.to_be_bytes());
        img[0x0C..0x10].copy_from_slice(&ramstart.to_be_bytes());
        img[0x10..0x14].copy_from_slice(&endmem.to_be_bytes());
        img[0x14..0x18].copy_from_slice(&0x1000u32.to_be_bytes());
        img[0x18..0x1C].copy_from_slice(&0x24u32.to_be_bytes());
        assert!(0x24 + func.len() <= 0x1C0, "program code overruns the name string");
        img[0x24..0x24 + func.len()].copy_from_slice(&func);
        let name = b"gvfstest\0";
        img[0x1C0..0x1C0 + name.len()].copy_from_slice(name);
        img
    }

    const NAME_ADDR: u32 = 0x1C0;

    /// A program: open a Data fileref by name, open it for Write, put "Hi", close,
    /// quit. Locals: local0=fref (off 0), local1=stream (off 4).
    fn write_hi_program() -> Vec<u8> {
        use E::*;
        let mut b = enc(0x149, &[Imm(2), Imm(0)]); // setiosys glk
        // fileref_create_by_name(usage=0, nameptr, rock=0) -> local0
        for v in [Imm(0), Imm(NAME_ADDR), Imm(0)] {
            b.extend(enc(0x40, &[v, Push])); // reverse arg order: rock, nameptr, usage
        }
        b.extend(enc(0x130, &[Imm(0x61), Imm(3), LocStore(0)]));
        // stream_open_file(fref=local0, fmode=1 Write, rock=0) -> local1
        b.extend(enc(0x40, &[Imm(0), Push])); // rock
        b.extend(enc(0x40, &[Imm(0x01), Push])); // fmode = Write
        b.extend(enc(0x40, &[LocLoad(0), Push])); // fref
        b.extend(enc(0x130, &[Imm(0x42), Imm(3), LocStore(4)]));
        // put_char_stream(str=local1, ch): reverse order push ch, str
        for ch in *b"Hi" {
            b.extend(enc(0x40, &[Imm(ch as u32), Push]));
            b.extend(enc(0x40, &[LocLoad(4), Push]));
            b.extend(enc(0x130, &[Imm(0x81), Imm(2), Discard]));
        }
        // stream_close(str=local1, resultptr=0)
        b.extend(enc(0x40, &[Imm(0), Push]));
        b.extend(enc(0x40, &[LocLoad(4), Push]));
        b.extend(enc(0x130, &[Imm(0x44), Imm(2), Discard]));
        b.extend(enc(0x120, &[])); // quit
        b
    }

    /// A program: open the same fileref by name, open it for Read, echo its two
    /// bytes to a TextBuffer window, quit. Locals: window(0), fref(4), stream(8),
    /// char(12).
    fn read_hi_program() -> Vec<u8> {
        use E::*;
        let mut b = open_buffer_prelude(); // window -> local0, made current
        // fileref_create_by_name -> local1
        for v in [Imm(0), Imm(NAME_ADDR), Imm(0)] {
            b.extend(enc(0x40, &[v, Push]));
        }
        b.extend(enc(0x130, &[Imm(0x61), Imm(3), LocStore(4)]));
        // stream_open_file(fref=local1, fmode=2 Read, rock=0) -> local2
        b.extend(enc(0x40, &[Imm(0), Push]));
        b.extend(enc(0x40, &[Imm(0x02), Push])); // Read
        b.extend(enc(0x40, &[LocLoad(4), Push]));
        b.extend(enc(0x130, &[Imm(0x42), Imm(3), LocStore(8)]));
        // read two bytes, echo each to the current (window) stream
        for _ in 0..2 {
            b.extend(enc(0x40, &[LocLoad(8), Push])); // str
            b.extend(enc(0x130, &[Imm(0x90), Imm(1), LocStore(12)])); // get_char_stream -> local3
            b.extend(enc(0x40, &[LocLoad(12), Push])); // ch
            b.extend(enc(0x130, &[Imm(0x80), Imm(1), Discard])); // put_char(ch)
        }
        b.extend(enc(0x120, &[])); // quit
        b
    }

    #[test]
    fn drive_persists_and_reloads_the_vfs_sidecar() {
        // A private temp dir so we never write sidecars next to real files.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("gvmcli-vfs-{}-{}", std::process::id(), stamp));
        fs::create_dir_all(&dir).unwrap();
        let vfs_path = dir.join("story.glkvfs");

        // Run 1: a game writes "Hi" into a Glk file, then quits. drive's in-loop,
        // dirty-gated flush should persist the VFS to the sidecar.
        let mut m = build_machine(vfs_image(write_hi_program(), 2), Box::new(TestBackend::new())).unwrap();
        drive(&mut m, &vfs_path, true, false, false, |_| {}, |_echo| (String::new(), 0), || (keycode::RETURN, String::new()));

        assert!(vfs_path.exists(), "the .glkvfs sidecar is written to disk");
        let blob = fs::read(&vfs_path).unwrap();
        let files = gvm::glk::decode_files(&blob);
        let stored = files.values().next().expect("exactly one file persisted");
        assert_eq!(stored.as_slice(), b"Hi", "the written bytes are in the sidecar");

        // Run 2: a fresh machine loads the sidecar (as main() does before drive),
        // then a game reads the file back and echoes it.
        let mut m2 = build_machine(vfs_image(read_hi_program(), 4), Box::new(TestBackend::new())).unwrap();
        m2.load_vfs(&fs::read(&vfs_path).unwrap());
        m2.clear_vfs_dirty();
        drive(&mut m2, &vfs_path, true, false, false, |_| {}, |_echo| (String::new(), 0), || (keycode::RETURN, String::new()));
        let text = m2.backend_mut().as_any_mut().downcast_mut::<TestBackend>().unwrap().all_text();
        assert_eq!(text, "Hi", "the persisted bytes are readable after load_vfs");

        let _ = fs::remove_dir_all(&dir);
    }

    // ── in-game @save/@restore via a prompted filename (SQ-0283 Task 5) ──────

    /// A program that exercises the full in-game save/restore prompt path:
    /// creates a SavedGame fileref by prompt (host-intercepted usage `0x01`,
    /// auto-resolved without a `read_line` call), opens it for writing, issues
    /// `@save`, then — only on the fresh (not-yet-restored) run — creates a
    /// second SavedGame fileref for reading, opens it, and issues `@restore`.
    /// The `jeq` guards against re-running that block on the resumed
    /// (post-restore) run: `@save`'s S1 then reads back `-1` (the "just
    /// restored" sentinel), and execution always resumes just after the
    /// original `@save`, so without the guard the read+`@restore` block would
    /// run again forever. Locals: fref-w(0), stream-w(4), fref-r(8), stream-r(12).
    fn save_restore_by_prompt_program() -> Vec<u8> {
        use E::*;
        let mut b = enc(0x149, &[Imm(2), Imm(0)]); // setiosys glk

        // fileref_create_by_prompt(usage=1 SavedGame, fmode=1 Write, rock=0) -> local0
        b.extend(enc(0x40, &[Imm(0), Push])); // rock
        b.extend(enc(0x40, &[Imm(0x01), Push])); // fmode = Write
        b.extend(enc(0x40, &[Imm(0x01), Push])); // usage = SavedGame (topmost -> args[0])
        b.extend(enc(0x130, &[Imm(0x62), Imm(3), LocStore(0)]));
        // stream_open_file(fref=local0, fmode=1 Write, rock=0) -> local1
        b.extend(enc(0x40, &[Imm(0), Push])); // rock
        b.extend(enc(0x40, &[Imm(0x01), Push])); // fmode = Write
        b.extend(enc(0x40, &[LocLoad(0), Push])); // fref
        b.extend(enc(0x130, &[Imm(0x42), Imm(3), LocStore(4)]));

        // @save L1=local1, S1 -> mem[0x280]
        b.extend(enc(0x123, &[LocLoad(4), MemLoad(0x280)]));

        // The read-back + @restore block, skipped on the resumed run.
        let mut skip = Vec::new();
        // fileref_create_by_prompt(usage=1 SavedGame, fmode=2 Read, rock=0) -> local2
        skip.extend(enc(0x40, &[Imm(0), Push])); // rock
        skip.extend(enc(0x40, &[Imm(0x02), Push])); // fmode = Read
        skip.extend(enc(0x40, &[Imm(0x01), Push])); // usage = SavedGame
        skip.extend(enc(0x130, &[Imm(0x62), Imm(3), LocStore(8)]));
        // stream_open_file(fref=local2, fmode=2 Read, rock=0) -> local3
        skip.extend(enc(0x40, &[Imm(0), Push])); // rock
        skip.extend(enc(0x40, &[Imm(0x02), Push])); // fmode = Read
        skip.extend(enc(0x40, &[LocLoad(8), Push])); // fref
        skip.extend(enc(0x130, &[Imm(0x42), Imm(3), LocStore(12)]));
        // @restore L1=local3, S1 -> mem[0x288]
        skip.extend(enc(0x124, &[LocLoad(12), MemLoad(0x288)]));

        let skip_len = skip.len() as u32;
        // jeq mem[0x280], -1, +skip_len -> skip the read-back+@restore block once
        // S1 reads the "just restored" sentinel.
        b.extend(enc(0x24, &[MemLoad(0x280), Imm(0xFFFF_FFFF), Imm(skip_len + 2)]));
        b.extend(skip);
        b.extend(enc(0x120, &[])); // quit
        b
    }

    #[test]
    fn drive_ingame_save_restore_round_trips_via_prompted_filename() {
        // A private temp dir so we never write next to real files.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("gvmcli-qzl-{}-{}", std::process::id(), stamp));
        fs::create_dir_all(&dir).unwrap();
        let save_path = dir.join("story.qzl");
        let vfs_path = dir.join("story.glkvfs");

        // SavedGame create_by_prompt auto-resolves (no read_line call); the only
        // prompts driven through `read_line` are the SaveRequest/RestoreRequest
        // filename prompts, both answered with the same fixed path so the
        // restore reads back exactly what the save wrote.
        let save_path_str = save_path.to_str().unwrap().to_string();
        let mut m =
            build_machine(vfs_image(save_restore_by_prompt_program(), 4), Box::new(TestBackend::new())).unwrap();
        drive(&mut m, &vfs_path, true, false, false, |_| {}, move |_echo| (save_path_str.clone(), 0), || {
            (keycode::RETURN, String::new())
        });

        let bytes = fs::read(&save_path).expect("the prompted filename was written by SaveRequest");
        fn has_chunk(save: &[u8], id: &[u8; 4]) -> bool {
            save.windows(4).any(|w| w == id)
        }
        assert!(has_chunk(&bytes, b"IFhd"), "the .qzl carries IFhd");
        assert!(has_chunk(&bytes, b"CMem"), "the .qzl carries CMem");
        assert!(has_chunk(&bytes, b"Stks"), "the .qzl carries Stks");
        assert!(!has_chunk(&bytes, b"GReg"), "the .qzl is bare, not a full save_state snapshot");
        assert!(!has_chunk(&bytes, b"Glk "), "the .qzl is bare, not a full save_state snapshot");

        // The drive loop's own RestoreRequest arm already read this file back
        // mid-game (via complete_restore_quetzal, reaching Quit); confirm the
        // written bytes are independently a valid, self-contained save too.
        assert!(m.restore_quetzal(&bytes).is_ok(), "the written .qzl round-trips via restore_quetzal");

        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod help_width_tests {
    /// SQ-1093. One wrap authority across all four front-ends. The reported
    /// symptom was a `--help` showing two at once — prose hand-wrapped to about
    /// 83 columns beside a run that nothing measured — so the right margin read
    /// as a rendering fault rather than a layout choice. `cli_host::HELP_WIDTH`
    /// is the number; this is gvm-cli's half of the pin.
    #[test]
    fn every_help_line_fits_the_one_width_all_four_front_ends_share() {
        let over = cli_host::overlong_help_lines(super::HELP);
        assert!(
            over.is_empty(),
            "--help must wrap at {}, but {over:?} do not:\n{}",
            cli_host::HELP_WIDTH,
            super::HELP
        );
        assert!(
            super::HELP
                .lines()
                .filter(|l| l.chars().count() > cli_host::HELP_WIDTH - 10)
                .count()
                > 5,
            "the text should be filling the width, not merely short of it"
        );
    }
}
