// zvm-cli — dumb-terminal Z-machine host driver (Task 16).
//
// Usage: zvm-cli <story-file>
//
// Reads a story file and plays it via stdin/stdout.  The host loop:
//   Continue     → keep stepping
//   Quit         → exit
//   Restart      → reload and restart from the original story bytes
//   NeedLine     → read a line from stdin, supply to machine
//   NeedChar     → read one byte from stdin, supply to machine
//   SaveRequest  → prompt for filename, write Quetzal bytes, complete_save
//   RestoreRequest → prompt for filename, read Quetzal bytes, complete_restore_success

use std::any::Any;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal;

use cli_host::{HostMode, TerminalGuard};

use zvm::cpu::exec::{Machine, StepResult};
use zvm::io::Output;
use zvm::memory::Memory;

mod screen;
mod auxiliary; // "aux" is a reserved filename on Windows — module renamed accordingly
mod media;

// ── sound ──────────────────────────────────────────────────────────────────────

/// The CLI's owned audio state: backend + resolved Blorb + live sound tracking.
struct CliSound {
    backend: audio::AudioBackend,
    blorb: Option<blorb::Blorb>,
    /// Sounds the story's own MEDIUM carries, by effect number, already wrapped as
    /// AIFF (SQ-0907). The two Infocom games that use sound ship it on the release
    /// disk rather than in a Blorb, and the CLI plays from the same source the TUI
    /// does — `blorb::infocom_sound::from_volume` is shared so the two cannot drift.
    disk: HashMap<u16, Vec<u8>>,
    ids: HashMap<u16, audio::SoundId>,
    routines: HashMap<audio::SoundId, u16>,
}

fn sound_kind_to_format(k: blorb::SoundKind) -> Option<audio::SoundFormat> {
    match k {
        blorb::SoundKind::Aiff => Some(audio::SoundFormat::Aiff),
        blorb::SoundKind::Ogg => Some(audio::SoundFormat::Ogg),
        blorb::SoundKind::Mod => Some(audio::SoundFormat::Mod),
        blorb::SoundKind::Other => None,
    }
}

/// Play a drained batch of `SoundEvent`s: bleeps (#1/#2) → tones; samples (#>=3)
/// → Blorb resource playback tracked by number, remembering finish routines.
fn play_cli_sounds(cs: &mut CliSound, events: &[zvm::cpu::exec::SoundEvent]) {
    for ev in events {
        match ev.number {
            0 => {}
            1 | 2 => {
                if ev.effect == 0 || ev.effect == 2 {
                    let freq = if ev.number == 1 { 800.0 } else { 400.0 };
                    cs.backend.play_tone(freq, 150, ev.volume);
                }
            }
            n => match ev.effect {
                3 => { if let Some(id) = cs.ids.remove(&n) { cs.backend.stop(id); } }
                1 => {}
                _ => {
                    // THE MEDIUM ANSWERS FIRST (SQ-0914): a release disk is the
                    // rendition Infocom pressed, a `.blb` is a later re-rendering of
                    // it, and graphics has always resolved this way round. Shared
                    // wording with the TUI on purpose — the two front-ends must not
                    // disagree about which source a story is playing from.
                    let picked = cs
                        .disk
                        .get(&n)
                        .map(|a| (a.clone(), audio::SoundFormat::Aiff))
                        .or_else(|| {
                            cs.blorb.as_ref().and_then(|b| b.sound(n as u32)).and_then(
                                |(bytes, kind)| {
                                    sound_kind_to_format(kind).map(|fmt| (bytes.to_vec(), fmt))
                                },
                            )
                        });
                    if let Some((bytes, fmt)) = picked {
                        if let Some(id) = cs.backend.play_sample(&bytes, fmt, ev.volume, ev.repeats) {
                            cs.ids.insert(n, id);
                            if ev.routine != 0 { cs.routines.insert(id, ev.routine); }
                        }
                    }
                }
            },
        }
    }
}

/// Poll finished sampled sounds; run their finish-routines and reprint the frame.
fn poll_sound_finish(sound: Option<&mut CliSound>, machine: &mut Machine, view: &mut screen::ScreenView, is_tty: bool) {
    let Some(cs) = sound else { return };
    let done = cs.backend.finished();
    let mut ran = false;
    for id in done {
        // Always forget the number->id mapping for a finished sound, even one
        // with no finish routine.
        cs.ids.retain(|_, v| *v != id);
        if let Some(routine) = cs.routines.remove(&id) {
            if routine != 0 {
                machine.run_routine(routine);
                ran = true;
            }
        }
    }
    // A finish routine may itself start sounds (into machine.pending_sounds);
    // play them now rather than deferring to the next main-loop step().
    if !machine.pending_sounds.is_empty() {
        let events: Vec<zvm::cpu::exec::SoundEvent> = std::mem::take(&mut machine.pending_sounds);
        play_cli_sounds(cs, &events);
    }
    if ran && is_tty {
        print!("{}", view.frame(machine));
        release_prompt(machine);
        let _ = io::stdout().flush();
    }
}

// ── Pure word-wrap helper ─────────────────────────────────────────────────────

/// Soft-wrap `text` by word-token boundaries, tracking `current_col` as the
/// starting column position. Returns `(wrapped_text, new_col)`. Explicit `\n`
/// in `text` always resets the column to 0. When `cols` is `u16::MAX`,
/// wrapping is disabled (used when `buffer_mode` is off).
///
/// `text` must be the game's own CHARACTERS — see [`format_output`], which owns
/// the ordering rule that keeps it that way.
fn wrap_line(text: &str, cols: u16, current_col: u16) -> (String, u16) {
    let mut out = String::with_capacity(text.len());
    let mut col = current_col;
    for word in text.split_inclusive(&[' ', '\n'][..]) {
        let is_nl = word.ends_with('\n');
        let clean = if is_nl { &word[..word.len() - 1] } else { word };
        let wlen = clean.chars().count() as u16;
        if col > 0 && col.saturating_add(wlen) > cols {
            out.push('\n');
            let trimmed = clean.trim_start();
            out.push_str(trimmed);
            col = trimmed.chars().count() as u16;
        } else {
            out.push_str(clean);
            col = col.saturating_add(wlen);
        }
        if is_nl {
            out.push('\n');
            col = 0;
        }
    }
    (out, col)
}

/// Compose the bytes for one game write: wrap first, decorate second.
///
/// The ORDER is the whole point (SQ-0702). `style_wrap` decorates text with SGR
/// escapes, and an escape occupies no column on screen — ZMSD §8.8.3.1.2.2 puts
/// the soft wrap "after the last word which could fit on a line", and a `\x1b[1m`
/// is not part of any word. Styling first and wrapping the result charged the
/// game nine columns for every bold space: Anchorhead's title splash prints its
/// centring indent one styled space at a time, so `A N C H O R H E A D` broke
/// mid-title eight spaces in, at what the sink thought was column 80. Wrapping
/// the game's own characters and applying the style to the result keeps the
/// column arithmetic in the game's units.
///
/// Returns `(bytes_to_emit, new_col)`; `attrs` is `None` for an unstyled write.
fn format_output(
    text: &str,
    attrs: Option<zvm::io::TextAttrs>,
    cols: u16,
    current_col: u16,
    is_tty: bool,
) -> (String, u16) {
    let (wrapped, new_col) = wrap_line(text, cols, current_col);
    let out = match attrs {
        Some(a) => crate::screen::style_wrap(&wrapped, a, is_tty),
        None => wrapped,
    };
    (out, new_col)
}

// ── StdoutOutput ──────────────────────────────────────────────────────────────

/// Output sink that writes directly to stdout and flushes after each call.
/// On a TTY it wraps styled lower-window text in SGR; when piped it stays plain.
struct StdoutOutput {
    is_tty: bool,
    /// `[MORE]` paging, shared with the other two hosts (`cli_host::Pager`).
    pager: cli_host::Pager,
    cols: u16,
    current_col: u16,
    /// Mirrors `machine.screen.buffer_mode`: when `false`, soft word-wrap at
    /// the column limit is suppressed (per Z-spec, unwrapped output is flushed
    /// immediately; explicit `\n` and `[MORE]` paging still apply).
    buffer_mode: bool,
    /// When `false`, game-supplied fg/bg colour SGR is suppressed at render time
    /// even if a non-conformant game sets colour after the header bit is cleared.
    /// Style bits (reverse/bold/italic) are always preserved.
    honor_game_colours: bool,
    /// Line-position tracking, and — in plain mode — holding the prompt back so
    /// the status block can be written before it. Shared with gvm-cli, which had
    /// grown its own half of the same answer (`cli_host::LineHold`, SQ-0611).
    /// Outside plain mode it holds nothing: the status is painted in a pinned
    /// region and never enters the text flow at all.
    hold: cli_host::LineHold,
    /// Did the last thing actually written to stdout — by the game *or* by the
    /// host — leave the cursor mid-line?
    ///
    /// `LineHold` answers this for the game's stream only, and that stops being
    /// the sink's answer as soon as the host writes something itself: a menu
    /// announcement replacing a block the game repainted (SQ-0609) is a whole
    /// line, but the hold never sees it, so the next announcement would insert a
    /// second newline and read as a blank line between every keypress.
    sink_mid_line: bool,
}

impl StdoutOutput {
    fn new(
        is_tty: bool,
        paging: bool,
        page_height: u16,
        cols: u16,
        honor_game_colours: bool,
        hold_partial: bool,
    ) -> Self {
        StdoutOutput {
            is_tty,
            pager: cli_host::Pager::new(paging, page_height),
            cols,
            current_col: 0,
            buffer_mode: false,
            honor_game_colours,
            hold: cli_host::LineHold::new(hold_partial),
            sink_mid_line: false,
        }
    }

    /// Write one character, holding it back when it is an unterminated tail and
    /// this sink is holding. A complete line always goes straight out.
    fn emit_char(&mut self, ch: char) {
        let mut buf = [0u8; 4];
        let out = self.hold.feed(ch.encode_utf8(&mut buf));
        print!("{out}");
        self.note_sink(&out);
    }

    /// Record what `text` did to the cursor's line position. Every write to
    /// stdout must pass through here or [`Self::sink_mid_line`] goes stale.
    fn note_sink(&mut self, text: &str) {
        if !text.is_empty() {
            self.sink_mid_line = !text.ends_with('\n');
        }
    }

    /// Would host text written now begin its own line?
    fn sink_at_line_start(&self) -> bool {
        !self.sink_mid_line
    }

    /// Release the held prompt.
    ///
    /// Must be called before anything else writes to stdout, and before
    /// blocking or exiting — otherwise the prompt arrives after whatever
    /// overtook it, or never.
    fn release_partial(&mut self) {
        let held = self.hold.release();
        if !held.is_empty() {
            print!("{held}");
            self.note_sink(&held);
            let _ = io::stdout().flush();
        }
    }

    /// Emit unstyled `s` with optional token-based word wrapping.
    fn write_counted(&mut self, s: &str) {
        self.write_formatted(s, None);
    }

    /// Emit `s` with optional token-based word wrapping (gated by `buffer_mode`)
    /// and, when `attrs` is set, the game's style applied to the wrapped result.
    /// Calls `check_paging` after each newline (soft or hard).
    fn write_formatted(&mut self, s: &str, attrs: Option<zvm::io::TextAttrs>) {
        // When buffer_mode is off, pass u16::MAX as cols: saturating_add in
        // wrap_line will never trigger the wrap condition.
        let cols = if self.buffer_mode { self.cols } else { u16::MAX };
        let (bytes, new_col) = format_output(s, attrs, cols, self.current_col, self.is_tty);
        for ch in bytes.chars() {
            self.emit_char(ch);
            if ch == '\n' && self.pager.line() {
                let _ = io::stdout().flush();
                self.pager.pause(&mut io::stdout(), self.is_tty);
            }
        }
        self.current_col = new_col;
        let _ = io::stdout().flush();
    }
}

impl Output for StdoutOutput {
    fn print(&mut self, s: &str) {
        self.write_counted(s);
    }

    fn print_styled(&mut self, s: &str, style: u8) {
        use zvm::io::TextAttrs;
        self.print_attr(s, TextAttrs { style, ..Default::default() });
    }

    fn print_attr(&mut self, s: &str, attrs: zvm::io::TextAttrs) {
        // When honour is off, strip game fg/bg so a non-conformant game that sets
        // colour after the header bit is cleared still renders without colour.
        // Style bits (reverse/bold/italic) are preserved unconditionally.
        let effective = if self.honor_game_colours {
            attrs
        } else {
            zvm::io::TextAttrs {
                fg: zvm::screen::ZColour::Default,
                bg: zvm::screen::ZColour::Default,
                ..attrs
            }
        };
        self.write_formatted(s, Some(effective));
    }

    fn set_buffer_mode(&mut self, on: bool) {
        self.buffer_mode = on;
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Let go of the prompt the sink is holding back (plain mode — SQ-0611).
///
/// Pair it with every status emission (status first, then this) and call it
/// before anything else writes to stdout, blocks, or exits.
fn release_prompt(machine: &mut Machine) {
    if let Some(o) = machine.out.as_any_mut().downcast_mut::<StdoutOutput>() {
        o.release_partial();
    }
}

/// The game's current score, or `None` when there is no reading it.
///
/// Two different questions behind one name. In v1–v3 the interpreter owns the
/// status line and the standard puts the score in global 2 (ZMSD §8.2), so the
/// number is exact — and a *time* game has a clock there instead, and no score
/// at all. From v4 the game draws its own status line and the globals mean
/// whatever it likes, so the only score available is whatever it wrote on the
/// screen, recovered by pattern (SQ-0616).
fn current_score(machine: &Machine) -> Option<i32> {
    if machine.mem.version() < 4 {
        return match machine.status_line().right {
            zvm::screen::StatusRight::ScoreTurns { score, .. } => Some(score as i32),
            zvm::screen::StatusRight::Time { .. } => None,
        };
    }
    cli_host::score_in_status(&screen::ScreenView::status_now(machine))
}

/// Announce the score if it moved, above the prompt the sink is holding.
fn announce_score(machine: &mut Machine, watch: &mut cli_host::ScoreWatch, on: bool) {
    if !on {
        return;
    }
    if let Some(line) = watch.update(current_score(machine)) {
        println!("{line}");
        if let Some(o) = machine.out.as_any_mut().downcast_mut::<StdoutOutput>() {
            o.note_sink("\n");
        }
    }
}

/// Answer a host command (`/status`) mid-turn without wrecking the transcript.
///
/// The prompt has already gone out by now, so the answer would otherwise land on
/// the prompt line — the very thing SQ-0611 fixed one layer down. Start a fresh
/// line, print the answer, then put the prompt back so the player can see it is
/// still their turn.
fn print_host_answer(machine: &mut Machine, text: &str) {
    let (at_line_start, prompt) = match machine.out.as_any().downcast_ref::<StdoutOutput>() {
        Some(o) => (o.sink_at_line_start(), o.hold.last_prompt().to_string()),
        None => (true, String::new()),
    };
    if !at_line_start {
        println!();
    }
    println!("{text}");
    print!("{prompt}");
    if let Some(o) = machine.out.as_any_mut().downcast_mut::<StdoutOutput>() {
        o.note_sink(&format!("\n{prompt}"));
    }
    let _ = io::stdout().flush();
}

// ── Blorb extraction ──────────────────────────────────────────────────────────

/// If `bytes` is a Blorb, return its Z-code executable; reject Glulx with a
/// clear error; otherwise pass the bytes through unchanged (a raw story file).
fn extract_story(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    if !blorb::Blorb::is_blorb(&bytes) {
        return Ok(bytes);
    }
    let b = blorb::Blorb::parse(bytes).map_err(|e| format!("Error: invalid Blorb: {e:?}"))?;
    match b.executable() {
        Ok((blorb::ExecKind::ZCode, data)) => Ok(data.to_vec()),
        Ok((blorb::ExecKind::Glulx, _)) => {
            Err("Error: Glulx story files are not yet supported.".to_string())
        }
        Ok((blorb::ExecKind::Scott, _)) => {
            Err("Error: this is a Scott Adams Blorb; run it with lanthorn.".to_string())
        }
        Err(e) => Err(format!("Error: Blorb has no executable: {e:?}")),
    }
}

// ── disk images ───────────────────────────────────────────────────────────────

/// Mount an original release floppy and return the story to play (SQ-0834).
///
/// The menu goes to stdout and the questions are answered from stdin in the
/// terminal's own cooked line editing — this runs before raw mode is entered,
/// so there is nothing to hand back. Anything that leaves no story to open is
/// fatal and says why: an unmountable image, a disk with no story on it, a
/// `--story` that matches nothing, or a choice nobody is there to make.
///
/// `path` is the image the bytes came off, and it is passed for one reason: a
/// multi-volume release keeps its story on no single floppy, so the other
/// volumes have to be findable, and they are findable by **name** (SQ-0874).
fn mount_and_pick(
    path: &std::path::Path,
    raw: Vec<u8>,
    stdin_is_tty: bool,
    want: Option<&str>,
) -> (Vec<u8>, Option<blorb::medium::DiskImage>) {
    let die = |e: String| -> ! {
        eprintln!("{e}");
        std::process::exit(1);
    };
    let mut cands = match media::story_candidates(path, raw) {
        Ok(c) => c,
        Err(e) => die(e),
    };
    let chosen = media::choose(
        &cands,
        want,
        stdin_is_tty,
        |s| {
            print!("{s}");
            let _ = io::stdout().flush();
        },
        || {
            let mut line = String::new();
            match io::stdin().read_line(&mut line) {
                Ok(0) | Err(_) => None,
                Ok(_) => Some(line),
            }
        },
    );
    let chosen = match chosen {
        Ok(i) => i,
        Err(e) => die(e),
    };
    // Say which one opened whenever there was a choice to get wrong — including
    // the scripted `--story` path, where nothing else on screen would show it.
    if cands.len() > 1 {
        println!("Opening {}) {}", chosen + 1, cands[chosen].label());
    }
    let c = cands.swap_remove(chosen);
    (c.bytes, c.image)
}

// ── build_machine ─────────────────────────────────────────────────────────────

fn build_machine(
    story: Vec<u8>,
    stdout_is_tty: bool,
    paging: bool,
    page_height: u16,
    term_rows: u16,
    term_cols: u16,
    honor_game_colours: bool,
    interpreter_number: Option<u8>,
    // May this launch advertise the MACHINE's own `$2C`/`$2D` pair? (SQ-0928)
    machine_colours: bool,
    // Plain mode: hold the game's prompt back so the status can precede it.
    hold_prompt: bool,
) -> Result<Machine, String> {
    use zvm::error::ZError;
    let mem = Memory::new(story).map_err(|e| match e {
        ZError::UnsupportedVersion(v) => format!("Error: Z-machine version {v} is not supported."),
        ZError::NotAStoryFile => "Error: file is not a valid Z-machine story file.".to_string(),
        ZError::Truncated => "Error: story file is truncated.".to_string(),
        _ => format!("Error loading story: {e:?}"),
    })?;
    // v6 is a graphical, mouse-and-menu format: its games drive a windowed
    // display this front-end has no way to present, and none of them can be
    // driven through a plain character stream. Measured (SQ-0601): every v6
    // story we have — Zork Zero, Shogun, Arthur, Journey, advent.z6 — runs away
    // the moment its opening screen asks for input, whatever key it is given.
    // Zork Zero and Arthur flood the terminal with newlines; Shogun spins
    // silently with no output at all and no prompt to interrupt.
    //
    // So refuse here rather than hang. The refusal is the FRONT-END's, not the
    // library's: `zvm` supports v6 fully and lanthorn plays these games — run
    // them there.
    if mem.version() == 6 {
        return Err(
            "Error: Z-machine v6 graphical games are not supported by zvm-cli.\n\
             Run it with lanthorn, which renders v6 graphics and menus."
                .to_string(),
        );
    }
    let mut machine = Machine::with_output(mem, Box::new(StdoutOutput::new(
        stdout_is_tty,
        paging,
        page_height,
        term_cols,
        honor_game_colours,
        hold_prompt,
    )));
    machine.set_interpreter_number(interpreter_number);
    // …and the REST of that machine (SQ-0872). Setting `$1E` alone told the story
    // which machine it was on and left it to work out what that machine looked
    // like from zvm's own §8.3.2 seed, which is nobody's machine — so off a
    // ProDOS disk *Beyond Zork* was an Apple IIgs advertising black-on-white.
    // `zvm::interpreter` is the table both front-ends read, so the CLI and the
    // TUI now present the same machine off the same bytes.
    //
    // The palette is process-wide and set unconditionally: it governs how a
    // colour NUMBER resolves to an actual colour, so it must agree with the
    // number in `$1E` whether or not the game's colours are being honoured.
    // The `$2C`/`$2D` pair is gated on `honor_game_colours`, exactly as the TUI
    // gates it (`startup`'s `host_default_colours`): with colours declined the
    // interpreter has told the story it is colourless (§8.3.2) and has no page
    // of its own to advertise.
    // SQ-0928: …and the PAIR is gated once more, on where the machine came from.
    // A machine's `$2C`/`$2D` describes a machine, and running a story off its
    // release disk makes that description true of the launch. Naming a number by
    // hand does not — `--interpreter 6` on a bare `.z5` used to advertise the IBM
    // PC's page, and now that the IBM PC states one (blue under white) that would
    // paint every story anyone opened that way. `--colour machine` is the opt-in
    // for a player who named the machine and meant it.
    if let Some(m) = interpreter_number.and_then(zvm::interpreter::machine) {
        zvm::screen::set_palette(m.palette);
        if honor_game_colours && machine_colours {
            if let Some((bg, fg)) = m.default_colours {
                machine.set_default_colours(bg, fg);
            }
        }
    }
    machine.init_caps();
    // Report the real terminal size to the game. init_caps seeds a generous
    // 80×24 default; without this override the game centres and wraps against
    // 80 columns regardless of the actual pane width (e.g. a title page stays
    // centred for 80 in a 50-column terminal). Kept in sync on resize.
    //
    // `Machine::set_screen_dims`, not the bare `write_screen_dims` it wraps: the
    // header bytes are only half the report, and the other half (refitting a
    // live upper window to the new width) is what the app has always used.
    machine.set_screen_dims(term_rows.min(255) as u8, term_cols.min(255) as u8);
    Ok(machine)
}

// ── argument parsing ──────────────────────────────────────────────────────────

#[derive(Debug)]
struct Args {
    story: Option<String>,
    /// `--story <n|name>`: which story to take off a multi-story disk image.
    story_pick: Option<String>,
    story_only: bool,
    show_status: bool,
    /// SQ-1082: the four switches below are `--<noun> on|off`, resolved to the
    /// value in force for this run. `zvm-cli` has no config file, so the third
    /// state `lanthorn` needs — "not mentioned", which must leave a persisted
    /// value alone — collapses here into the default beside each one. What does
    /// NOT collapse is the spelling: one concept under two names across two
    /// binaries is the defect SQ-1078 existed to remove, and a `--no-sound` here
    /// beside a `--sound on|off` there would put it straight back.
    aux: bool,
    pager: bool,
    timed_input: bool,
    sound: bool,
    data_dir: Option<String>,
    honor_colours: bool,
    /// `--period-look`: dress the screen as the story's own machine did (SQ-0873).
    period_look: bool,
    /// `--colour machine`: present a named machine's own `$2C`/`$2D` pair
    /// (SQ-0928, respelled by SQ-1082). `None` leaves the medium to decide, which
    /// is the rule `machine_colours` applies further down.
    colour_machine: Option<bool>,
    interpreter: Option<u8>,
    volume: Option<u8>,
    /// `--pin <top|bottom>` (or `--scrollback`): where the fixed rows sit.
    pin: cli_host::Pin,
}

/// Every option `zvm-cli` accepts. The scanner in `cli_host::args` applies the
/// rules; this table is the only thing that differs between the three CLIs.
///
/// Options whose value is read further down (`--volume`, `-I`) still belong
/// here, because this is what tells an unrecognised flag from a story path.
const OPTS: &[cli_host::Opt] = &[
    cli_host::Opt::flag(&["--story-only", "--lower-only"]),
    cli_host::Opt::flag(&["--show-status"]),
    cli_host::Opt::valued(&["--aux"]),
    cli_host::Opt::valued(&["--pager"]),
    cli_host::Opt::valued(&["--timed-input"]),
    cli_host::Opt::valued(&["--sound"]),
    cli_host::Opt::valued(&["--game-colours", "--game-colors"]),
    cli_host::Opt::flag(&["--period-look"]),
    cli_host::Opt::valued(&["--colour", "--color"]),
    cli_host::Opt::flag(&["--screen-reader", "--plain"]),
    cli_host::Opt::flag(&["--help", "-h"]),
    cli_host::Opt::flag(&["--version", "-V"]),
    cli_host::Opt::flag(&["--machines"]),
    cli_host::Opt::valued(&["--volume"]),
    cli_host::Opt::valued(&["--interpreter", "-I"]),
    cli_host::Opt::valued(&["--data-dir"]),
    cli_host::Opt::valued(&["--story"]),
    cli_host::Opt::valued(&["--pin"]),
    cli_host::Opt::flag(&["--scrollback"]),
];

/// `--colour terminal|machine`: which source the story pane's DEFAULT page and
/// ink come from (SQ-1082). `Some(true)` is the machine, `Some(false)` the
/// terminal, `None` the rule already in force — the medium decides.
///
/// `lanthorn` takes a third value here, `theme`, and `zvm-cli` cannot: a theme is
/// a `style.toml`, and this binary has none. Named in the error rather than
/// merely absent from it, because "unknown value" would read as a typo when it is
/// really the right word at the wrong front-end.
fn parse_colour_source(value: Option<&str>) -> Result<Option<bool>, String> {
    match value {
        None => Ok(None),
        Some("machine") => Ok(Some(true)),
        Some("terminal") => Ok(Some(false)),
        Some("theme") => Err(
            "--colour theme names lanthorn's style.toml theme, and zvm-cli has none; \
             try terminal or machine"
                .to_string(),
        ),
        Some(v) => Err(format!("--colour takes terminal or machine, got '{v}'")),
    }
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let m = cli_host::scan(argv, OPTS)?;
    if m.positional.len() > 1 {
        return Err(format!("unexpected extra argument: {}", m.positional[1]));
    }
    // SQ-0613 renamed `--no-status` to `--story-only`, because it read as the same
    // thing plain mode does to the status line and is stronger than that — it
    // suppresses the whole upper window, menus included. It survived as a third
    // alias that printed a notice; SQ-1082 removed it outright, on the same rule
    // that took `--no-sound` away. Pre-release, an alias is only the old spelling
    // living on somewhere nobody maintains it, and `--status on|off` is not the
    // conversion: it would re-tell the very lie the rename removed.
    Ok(Args {
        story: m.first_positional().map(str::to_string),
        story_pick: m.value("--story").map(str::to_string),
        story_only: m.has("--story-only"),
        show_status: m.has("--show-status"),
        aux: cli_host::on_off("--aux", m.value("--aux"))?.unwrap_or(true),
        pager: cli_host::on_off("--pager", m.value("--pager"))?.unwrap_or(true),
        timed_input: cli_host::on_off("--timed-input", m.value("--timed-input"))?.unwrap_or(true),
        sound: cli_host::on_off("--sound", m.value("--sound"))?.unwrap_or(true),
        data_dir: m.value("--data-dir").map(str::to_string),
        honor_colours: cli_host::on_off("--game-colours", m.value("--game-colours"))?
            .unwrap_or(true),
        period_look: m.has("--period-look"),
        colour_machine: parse_colour_source(m.value("--colour"))?,
        // Lenient, as before: a bad value falls back to the engine default
        // rather than refusing to start.
        interpreter: m.value("--interpreter").and_then(|v| v.parse::<u8>().ok()),
        volume: m.value("--volume").and_then(|v| v.parse::<u8>().ok()).map(|v| v.min(100)),
        // `--scrollback` is the same request said the other way round — the
        // placement is the mechanism, the history is what a player actually wants —
        // on the `--screen-reader`/`--plain` precedent. An explicit `--pin` wins,
        // and a bad value falls back to the default rather than refusing to start,
        // as `--interpreter` does above.
        pin: m
            .value("--pin")
            .and_then(cli_host::Pin::parse)
            .unwrap_or(if m.has("--scrollback") { cli_host::Pin::Bottom } else { cli_host::Pin::Top }),
    })
}

// ── terminal size ─────────────────────────────────────────────────────────────

/// Detect the terminal size. Returns `(rows, cols)` using crossterm; falls
/// back to 24×80 if crossterm returns an error (e.g. stdout is piped).
fn detect_term_size() -> (u16, u16) {
    match terminal::size() {
        // A zero dimension (some PTYs, e.g. macOS `script`, return Ok((0, 0)))
        // would yield a 0 wrap width; fall back to the default like an error.
        Ok((cols, rows)) if cols > 0 && rows > 0 => (rows, cols),
        _ => (screen::DEFAULT_ROWS, screen::DEFAULT_COLS),
    }
}

// ── key input (crossterm) ─────────────────────────────────────────────────────

/// Map a crossterm `KeyCode` to the Z-machine key code used by `supply_char`.
/// Printable ASCII passes through; special keys map to ZMSD §3.8 codes.
fn decode_keycode(code: KeyCode) -> u8 {
    match code {
        KeyCode::Char(c) if (c as u32) < 128 => c as u8,
        KeyCode::Enter => b'\n',
        KeyCode::Backspace | KeyCode::Delete => 8, // DEL/BS
        KeyCode::Esc => 0x1B,
        KeyCode::Up => 129,
        KeyCode::Down => 130,
        KeyCode::Left => 131,
        KeyCode::Right => 132,
        // Function keys F1–F12 → ZSCII 133–144 (ZMSD §3.8). Keypad digits
        // (ZSCII 145–154) are unreachable: terminals report them as ordinary
        // Char events, indistinguishable from the number row.
        KeyCode::F(n) if (1..=12).contains(&n) => 132 + n,
        _ => b'\n', // unknown → newline
    }
}

/// Read one keypress in raw mode. Resize events that arrive before the key are
/// captured and returned alongside the key so the caller can update its state.
/// Piped stdin: reads the first byte of the next line.
///
/// When `timeout` is `Some((time, _))` (a timed `read_char` and timed input is
/// honored), each wait is bounded to `time * 100` ms; on expiry the pending
/// interrupt routine runs via `machine.run_timed_interrupt()`. If it aborts the
/// read, this returns with `aborted = true` (caller completes via
/// `abort_timed_input`); otherwise the routine's output is redrawn via `view`
/// and the wait resumes. `timeout = None` keeps today's exact blocking read.
fn read_char_input(
    is_tty: bool,
    machine: &mut Machine,
    view: &mut screen::ScreenView,
    timeout: Option<(u16, u16)>,
    sound: &mut Option<CliSound>,
) -> (u8, Option<(u16, u16)>, bool) {
    if !is_tty {
        return (read_byte_stdin(), None, false);
    }
    let _ = terminal::enable_raw_mode();
    let mut last_resize: Option<(u16, u16)> = None;
    let result = loop {
        if timeout.is_none() && sound.as_ref().is_some_and(|cs| !cs.routines.is_empty())
            && !event::poll(std::time::Duration::from_millis(50)).unwrap_or(false) {
                let _ = terminal::disable_raw_mode();
                poll_sound_finish(sound.as_mut(), machine, view, is_tty);
                let _ = terminal::enable_raw_mode();
                continue;
            }
        if let Some((t, _)) = timeout {
            if !event::poll(std::time::Duration::from_millis(t as u64 * 100)).unwrap_or(false) {
                let _ = terminal::disable_raw_mode();
                let out = machine.run_timed_interrupt();
                poll_sound_finish(sound.as_mut(), machine, view, is_tty);
                let _ = terminal::enable_raw_mode();
                if out.aborted {
                    break (0u8, last_resize, true);
                }
                print!("{}", view.frame(machine));
                release_prompt(machine);
                let _ = io::stdout().flush();
                continue;
            }
        }
        match event::read() {
            Ok(Event::Resize(c, r)) => last_resize = Some((c, r)),
            // Only a *press* is a keystroke: Windows delivers Release events
            // too, and taking one doubles every typed character (SQ-0633).
            Ok(ev) => {
                if let Some(k) = cli_host::key_press(&ev) {
                    // Ctrl-C / Ctrl-D: raw mode swallows signals, and without
                    // this they decode as plain 'c'/'d' — a game looping on
                    // read_char could never be interrupted (SQ-0636). Exit via
                    // the same restore path as the line-input arm.
                    if k.modifiers.contains(KeyModifiers::CONTROL)
                        && matches!(k.code, KeyCode::Char('c') | KeyCode::Char('d'))
                    {
                        print!("\r\n");
                        let _ = io::stdout().flush();
                        cli_host::restore_and_exit(&view.leave(), 0);
                    }
                    break (decode_keycode(k.code), last_resize, false);
                }
            }
            _ => {}
        }
    };
    let _ = terminal::disable_raw_mode();
    result
}

/// Print a pinned-region block, starting a fresh line when the story stream is
/// mid-line.
///
/// Plain mode normally holds the game's unterminated prompt back so the block
/// lands above it (SQ-0611), but two paths put text on the line before the block
/// is due: a prompt already released at an earlier stop with nothing new written
/// since (Arthur redraws its menu under a standing `[Press any key]`), and a
/// host answer that reprints the prompt after itself (`/status`, `/menu`).
/// Either way the block would be welded to the end of a prompt, which reads as
/// though the prompt were showing it to you — the same defect `LineHold` exists
/// to prevent. Plain mode only: elsewhere `frame` is cursor-addressed ANSI and a
/// newline in it would be a stray blank row.
fn print_frame(machine: &mut Machine, plain: bool, text: &str) {
    if text.is_empty() {
        return;
    }
    let Some(o) = machine.out.as_any_mut().downcast_mut::<StdoutOutput>() else {
        print!("{text}");
        return;
    };
    if plain && !o.sink_at_line_start() {
        println!();
        o.note_sink("\n");
    }
    print!("{text}");
    if !o.is_tty {
        // On a TTY the block is cursor-addressed inside DECSC/DECRC and the
        // cursor comes back where it was; there is nothing to record.
        o.note_sink(text);
    }
}

/// Read a keypress in screen-reader mode, where the terminal is cooked and a
/// "keypress" therefore arrives as a whole line.
///
/// Plain mode hands line editing back to the kernel (`HostMode::raw_input` is
/// false), so this path was already reading a line and throwing all but its
/// first byte away. Keeping the line is what makes the menu commands possible:
/// `/menu` is a word, and a jump to item 12 is two digits that only a
/// Enter-terminated read can tell from item 1 followed by item 2 (SQ-0609).
///
/// Everything else still hands the game the first byte, so `n`, `p`, Enter and
/// `q` reach the menu untouched.
fn read_cooked_char(machine: &mut Machine, view: &mut screen::ScreenView) -> u8 {
    loop {
        let Some(line) = cli_host::read_line_stdin() else {
            cli_host::input::exit_at_eof(&screen::leave_region())
        };
        if cli_host::is_menu_request(&line) {
            let text = view.menu_listing().unwrap_or_else(|| NO_MENU.to_string());
            print_host_answer(machine, text.trim_end());
            continue;
        }
        match view.typed_at_menu(&line) {
            cli_host::Typed::Jump => {
                if let Some(key) = view.next_menu_key() {
                    return key;
                }
            }
            cli_host::Typed::Here(said) => {
                print_host_answer(machine, &said);
                continue;
            }
            cli_host::Typed::Passthrough => {}
        }
        return line.bytes().next().unwrap_or(b'\n');
    }
}

/// The answer to `/menu` when nothing is open.
const NO_MENU: &str = "[no menu is open]";

// ── aux ("global state") persistence ──────────────────────────────────────────

/// Load the IFID-keyed aux file into the machine's aux_data (preload); warn on decode error.
fn aux_preload(machine: &mut Machine, aux_file: &Path, aux: bool) {
    if !aux {
        return;
    }
    if let Ok(bytes) = fs::read(aux_file) {
        match auxiliary::decode_aux(&bytes) {
            Ok(map) => {
                machine.aux_data = map;
                machine.aux_dirty = false;
            }
            Err(e) => eprintln!("zvm: warning: ignoring corrupt {}: {:?}", aux_file.display(), e),
        }
    }
}

/// Flush aux_data to the per-game aux file when dirty; clear the flag regardless.
fn aux_flush(machine: &mut Machine, aux_file: &Path, aux: bool) {
    if !aux || !machine.aux_dirty {
        return;
    }
    if let Some(dir) = aux_file.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Err(e) = fs::write(aux_file, auxiliary::encode_aux(&machine.aux_data)) {
        eprintln!("zvm: warning: aux save to {} failed: {}", aux_file.display(), e);
    }
    machine.aux_dirty = false;
}

// ── prompt + read helpers ─────────────────────────────────────────────────────

/// Prompt for a save/restore filename, listing the saves already in this game's
/// directory above it (SQ-0918).
///
/// A free-text filename with no indication of what exists means remembering what you
/// called things, at the one moment you cannot look. The list goes ABOVE the prompt
/// rather than in it, so the prompt itself is unchanged for anyone piping input, and
/// nothing at all is printed before the first save.
fn prompt_and_read_line(prompt: &str, saves: &[String]) -> String {
    if let Some(line) = cli_host::save_list_line(saves) {
        println!("\n{line}");
    }
    print!("{}", prompt);
    let _ = io::stdout().flush();
    // EOF here yields an empty filename, which the caller treats as "cancel"
    // (see `handle_save_request` / `handle_restore_request`) — unlike a *game's*
    // input request, an unanswered save prompt is recoverable, so this one does
    // not exit.
    cli_host::read_line_stdin().unwrap_or_default()
}

/// Complete a game `@save` with the player's `filename` answer.
///
/// An empty filename — a bare Enter, or EOF — is a cancel: the save fails and
/// the game is told so, matching gvm-cli. It used to fall through to
/// `resolve_save_input("")`, which writes a hidden `<game_dir>/.qzl` and
/// reports success for a save the player never asked for (SQ-0635).
fn handle_save_request(machine: &mut Machine, game_dir: &Path, filename: &str) {
    if filename.is_empty() {
        machine.complete_save(false);
        return;
    }
    let path = cli_host::resolve_save_input(filename, game_dir, cli_host::QUETZAL_EXT);
    // `fs::write` below is unconditional, so without this a repeated name silently
    // destroys the earlier save — the defect SQ-0648 fixed in the TUI, still live
    // out here (SQ-0918). A refusal is a cancel, which is the same path an empty
    // filename already takes, so the game is told the save failed rather than being
    // left waiting.
    if let Some(warning) = cli_host::overwrite_warning(&path) {
        if !cli_host::is_yes(&prompt_and_read_line(&warning, &[])) {
            println!("Save cancelled.");
            machine.complete_save(false);
            return;
        }
    }
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let save_data = machine.save_quetzal();
    match fs::write(&path, &save_data) {
        Ok(()) => {
            println!("Saved to '{}'.", path.display());
            machine.complete_save(true);
        }
        Err(e) => {
            eprintln!("Save failed: {e}");
            machine.complete_save(false);
        }
    }
}

/// Complete a game `@restore` with the player's `filename` answer.
///
/// Empty = cancel, same rule as [`handle_save_request`]: without it, a bare
/// Enter silently read whatever `<game_dir>/.qzl` happened to hold.
fn handle_restore_request(machine: &mut Machine, game_dir: &Path, filename: &str) {
    if filename.is_empty() {
        machine.complete_restore_failure();
        return;
    }
    let path = cli_host::resolve_save_input(filename, game_dir, cli_host::QUETZAL_EXT);
    match fs::read(&path) {
        Ok(data) => match machine.complete_restore_success(&data) {
            Ok(()) => {} // restored; @save descriptor completed forward
            Err(e) => {
                eprintln!("Restore failed: {e:?}");
                machine.complete_restore_failure();
            }
        },
        Err(e) => {
            eprintln!("Restore failed: {e}");
            machine.complete_restore_failure();
        }
    }
}

/// One line of piped input, or a clean exit at true EOF.
///
/// This path had the SQ-0604 bug all along, one path over from where it was
/// fixed: a 0-byte read left `line` empty and the game was handed a blank
/// command forever. Measured before the fix, `zvm-cli gostak.z5 < /dev/null`
/// printed 90 KB of `>Louk?` and had to be killed. The char path next door
/// (`read_byte_stdin`) had checked for EOF for months.
fn read_line_stdin() -> String {
    match cli_host::read_line_stdin() {
        Some(line) => line,
        None => cli_host::input::exit_at_eof(&crate::screen::leave_region()),
    }
}

/// Read a line of input in RAW mode, echoing as the user types. Unlike cooked
/// `read_line_stdin`, arrow/function-key escape sequences are parsed by
/// crossterm into key events (so they never leak onto the screen as garbage),
/// and the echo is drawn in the game's current style/colour (`echo`). Resize
/// events during the read are captured and returned so the caller can update
/// its layout. Falls back to cooked line input when stdin is not a TTY (piped).
///
/// When `timeout` is `Some((time, _))` (a timed `read` and timed input is
/// honored), each wait is bounded to `time * 100` ms; on expiry the pending
/// interrupt routine runs via `machine.run_timed_interrupt()`. If it aborts the
/// read, this returns with `aborted = true` (caller completes via
/// `abort_timed_input`, buffer preserved in `line`); otherwise the routine's
/// output is redrawn via `view` and the line-edit resumes. `timeout = None`
/// keeps today's exact blocking read.
fn read_line_raw(
    is_tty: bool,
    echo: zvm::io::TextAttrs,
    machine: &mut Machine,
    view: &mut screen::ScreenView,
    timeout: Option<(u16, u16)>,
    sound: &mut Option<CliSound>,
) -> (String, u8, Option<(u16, u16)>, bool) {
    if !is_tty {
        return (read_line_stdin(), 13, None, false);
    }
    let _ = terminal::enable_raw_mode();
    let mut buf = String::new();
    let mut terminator: u8 = 13; // Enter unless a function-key terminator ends the line
    let mut last_resize: Option<(u16, u16)> = None;
    let mut aborted = false;
    let sgr = crate::screen::sgr_open(echo);
    if !sgr.is_empty() {
        print!("{sgr}");
        let _ = io::stdout().flush();
    }
    loop {
        if timeout.is_none() && sound.as_ref().is_some_and(|cs| !cs.routines.is_empty())
            && !event::poll(std::time::Duration::from_millis(50)).unwrap_or(false) {
                let _ = terminal::disable_raw_mode();
                poll_sound_finish(sound.as_mut(), machine, view, is_tty);
                let _ = terminal::enable_raw_mode();
                continue;
            }
        if let Some((t, _)) = timeout {
            if !event::poll(std::time::Duration::from_millis(t as u64 * 100)).unwrap_or(false) {
                let _ = terminal::disable_raw_mode();
                let out = machine.run_timed_interrupt();
                poll_sound_finish(sound.as_mut(), machine, view, is_tty);
                let _ = terminal::enable_raw_mode();
                if out.aborted {
                    aborted = true;
                    break;
                }
                print!("{}", view.frame(machine));
                release_prompt(machine);
                let _ = io::stdout().flush();
                continue;
            }
        }
        match event::read() {
            Ok(Event::Resize(c, r)) => last_resize = Some((c, r)),
            // Only a *press* is a keystroke: Windows delivers Release events
            // too, and taking one doubles every typed character and lets the
            // Enter release submit a phantom empty command (SQ-0633).
            Ok(ev) => {
                let Some(k) = cli_host::key_press(&ev) else { continue };
                match k.code {
                    KeyCode::Enter => break,
                    // Ctrl-C / Ctrl-D: raw mode swallows signals, so exit cleanly
                    // ourselves (drop the colour, leave raw mode + the scroll region).
                    KeyCode::Char('c') | KeyCode::Char('d')
                        if k.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        if !sgr.is_empty() { print!("\x1b[0m"); }
                        print!("\r\n"); // close the echoed line before the terminal goes back
                        let _ = io::stdout().flush();
                        // The scroll region is this renderer's own teardown, so it
                        // rides along as the prefix; everything after it is the
                        // shared restore.
                        cli_host::restore_and_exit(&view.leave(), 0);
                    }
                    KeyCode::Char(c) => {
                        buf.push(c);
                        print!("{c}");
                        let _ = io::stdout().flush();
                    }
                    KeyCode::Backspace => {
                        if buf.pop().is_some() {
                            // Move left, erase, move left again.
                            print!("\x08 \x08");
                            let _ = io::stdout().flush();
                        }
                    }
                    // A function key listed in the game's terminating-characters
                    // table (header 0x2E) ends the line, reported to the game via the
                    // stored terminator — e.g. the cursor keys BeyondZork uses to
                    // scroll its boxed description (ZMSD §10.7). Any other special key
                    // is consumed (no on-screen garbage), as before.
                    _ => {
                        let z = decode_keycode(k.code) as u16;
                        if machine.is_terminator(z) {
                            terminator = z as u8;
                            break;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if !sgr.is_empty() {
        print!("\x1b[0m");
    }
    let _ = terminal::disable_raw_mode();
    if terminator == 13 {
        // Raw mode does not translate Enter to CRLF. Skip for a function-key
        // terminator so the prompt doesn't drift as the game redraws in place.
        //
        // The newline SCROLLS, and a terminal erases the newly exposed line with
        // whatever background is in force at that moment. Resetting first — which is
        // what the line above just did — hands the game's next line the TERMINAL's
        // default background beyond the text it writes, so exactly one row per turn
        // came out with a pale tail: the first line the game prints after your
        // command, which in BeyondZork is the room name. Every later line of the
        // turn scrolls inside a styled run and is fine, which is what made it look
        // like a property of room names (SQ-0920).
        //
        // Only the BACKGROUND is re-applied, not the whole run: reverse video would
        // make the erase paint the foreground colour instead.
        print!(
            "{}",
            commit_line_bytes(&screen::bg_sgr(machine.screen.current_bg, machine.honor_game_colours))
        );
    }
    let _ = io::stdout().flush();
    (buf, terminator, last_resize, aborted)
}

/// The bytes that close an echoed input line: the game's background, the newline,
/// then a reset.
///
/// The ORDER is the whole point (SQ-0920). That newline scrolls, and a terminal
/// erases the newly exposed line with whatever background is in force at that
/// moment, so emitting it after a reset gives the game's next line a tail of the
/// terminal's own default background beyond the text it writes. Exactly one row per
/// turn came out that way — the first line printed after the command, which in
/// BeyondZork is the room name; every later line of the turn scrolls inside a styled
/// run and looked fine, which is what made it seem like a property of room names.
///
/// Only the BACKGROUND is re-applied, never the whole run: reverse video would make
/// the erase paint the foreground colour across the line instead.
fn commit_line_bytes(bg: &str) -> String {
    if bg.is_empty() {
        "\r\n".to_string()
    } else {
        format!("{bg}\r\n\x1b[0m")
    }
}

/// One byte of piped input, or a clean exit at true EOF.
///
/// The EOF rule (a 0-byte read is the end; a blank line is still a real `\n`)
/// lives in `cli_host::input` — see SQ-0604/0605 for why it is worth having in
/// exactly one place. What this path used to get wrong was the *exit*: it called
/// `process::exit(0)` directly, leaving the terminal wearing the game's page
/// background and a block cursor whenever stdin was a pipe but stdout was not
/// (`echo commands | zvm-cli story.z5`). `exit_at_eof` restores first.
fn read_byte_stdin() -> u8 {
    match cli_host::read_byte_stdin() {
        Some(b) => b,
        None => cli_host::input::exit_at_eof(&crate::screen::leave_region()),
    }
}

// ── terminal resize helper ────────────────────────────────────────────────────

/// Apply a new terminal size `(new_rows, new_cols)` if it differs from the
/// current `(last_rows, last_cols)`. Updates the output sink, paging height,
/// and screen view. Only runs when `is_tty` is true.
fn apply_resize(
    new_rows: u16,
    new_cols: u16,
    last_rows: &mut u16,
    last_cols: &mut u16,
    page_height: &mut u16,
    machine: &mut Machine,
    view: &mut screen::ScreenView,
) {
    if new_rows == *last_rows && new_cols == *last_cols {
        return;
    }
    *last_rows = new_rows;
    *last_cols = new_cols;
    *page_height = cli_host::Pager::height_for(new_rows);
    view.set_term_rows(new_rows);
    view.set_term_cols(new_cols);
    if let Some(o) = machine.out.as_any_mut().downcast_mut::<StdoutOutput>() {
        o.cols = new_cols;
        o.pager.set_page_height(*page_height);
    }
    // Keep the game's view of the screen size current so it re-centres and
    // re-wraps against the new pane width on its next redraw. `set_screen_dims`
    // also refits a LIVE upper window to the new width (SQ-0679): a game that
    // splits once at boot and never re-splits — Sherlock, Trinity — otherwise
    // keeps its boot-time grid width while the header tracks the resized
    // terminal, so its status band stops short of (or overflows) the new edge.
    machine.set_screen_dims(new_rows.min(255) as u8, new_cols.min(255) as u8);
}

/// Poll the current terminal size and apply it if it changed. Only runs when
/// `is_tty` is true (otherwise no-op — avoids crossterm errors on piped I/O).
fn maybe_resize(
    is_tty: bool,
    last_rows: &mut u16,
    last_cols: &mut u16,
    page_height: &mut u16,
    machine: &mut Machine,
    view: &mut screen::ScreenView,
) {
    if !is_tty {
        return;
    }
    let (new_rows, new_cols) = detect_term_size();
    apply_resize(new_rows, new_cols, last_rows, last_cols, page_height, machine, view);
}

// ── main ──────────────────────────────────────────────────────────────────────

/// The help text, with its two disk-media facts still to be filled in — see
/// [`help`]. Everything else here is prose about this front-end and belongs in
/// this file.
const HELP: &str = "\
zvm-cli — DOS-style Z-machine player (no map)

Usage: zvm-cli [OPTIONS] <story-file>

Arguments:
  <story-file>          Z-code story (.z3/.z5/.z8 …, or a .zblorb container), or
                        an original release floppy — recognised by its contents,
                        never by its name — whose story is mounted straight off
                        the disk. Conventionally spelt:
                          {DISK_EXTENSIONS}
                        The medium also sets the interpreter number its machine
                        implies, unless -I says otherwise:
                          {DISK_MACHINES}
                        A compilation disk holds several stories; pick one with
                        --story, and each of them keeps its own saves. A release
                        pressed across several volumes is opened by naming ANY
                        one of them — the rest are found beside it. Graphical v6
                        stories are not supported — play those with lanthorn.

Host commands (never passed to the game):
  /status               Repeat the current status line / upper window
  /pin [top|bottom]     Move the status line / upper window between the top of
                        the screen and the bottom; bare /pin swaps. See --pin.
  /menu                 Re-read the open menu, host-numbered. In --screen-reader
                        mode a menu keypress is a whole typed line, so /menu —
                        and a bare item number, which jumps to that item — work
                        at a menu's own prompt as well as at a line prompt.

Options:
      --screen-reader   Linear plain text (alias: --plain; also selected by
                        TERM=dumb). Emits no escape sequences at all — no
                        colour, no cursor addressing, no pinned status line —
                        hands line editing and echo back to the terminal, and
                        turns off the [MORE] pager. The status line is not
                        narrated every turn (see --show-status); menus and forms
                        still are. Ask for the status any time with /status. A
                        menu that repaints as its marker moves is announced in
                        one line rather than re-read (see /menu above).
      --story-only      Show only the story text: suppress the whole upper
                        window, menus and forms included. Stronger than what
                        --screen-reader does to the status line, and independent
                        of it. (alias: --lower-only)
      --show-status     Narrate the status line whenever the story updates it,
                        undoing --screen-reader's quietening. Off there because
                        a v3 status line carries a move counter, so it changes —
                        and would be re-read — on every single turn.
      --story <n|name>  Which story to take off a disk image that holds more
                        than one: its menu number, or part of its name. A disk
                        with a single story needs no flag; without one, and
                        without a terminal to ask at, a multi-story disk lists
                        what it found and stops rather than blocking.
      --aux <on|off>    Read and write v5 auxiliary (VFS) sidecar files. Default
                        on.
      --pager <on|off>  [MORE] paging on long output. Default on, and off
                        wherever it could not work anyway: --screen-reader, or a
                        piped stdout.
      --pin <top|bottom>
                        Where the status line and upper window are pinned.
                        Default top, where Infocom put them.

                        Pinning at the BOTTOM is what gets you terminal
                        scrollback, and the reason is not obvious: a terminal
                        only archives a line that scrolls off the TOP of the
                        screen, which it judges by the scroll region's top
                        margin. Pinned at the top, the region starts at row 2,
                        nothing ever leaves the screen, and every line you have
                        read is discarded. Pinned at the bottom the region
                        starts at row 1 again, so the story scrolls into your
                        terminal's own history — with its wheel, its selection
                        and its search. Nothing is buffered by lanthorn either
                        way. Swap it mid-game with /pin.
      --scrollback      Alias for --pin bottom, named for what it is for.
      --timed-input <on|off>
                        Honour timed-input interrupts (read / read_char with a
                        time and a routine). Default on.
      --game-colours <on|off>
                        Honour the game's set_colour / true-colour output.
                        Default on; NO_COLOR turns it off too. (alias:
                        --game-colors)
      --colour <terminal|machine>
                        Where the DEFAULT page and ink reported in $2C/$2D come
                        from. `machine` advertises a named machine's own pair;
                        `terminal` declines it and leaves your terminal's
                        colours in force.

                        Unset, the medium decides: a story opened off a release
                        disk gets that machine's pair, because the disk is what
                        makes the description true of the launch, while a number
                        you typed at a bare story file does not. So `machine` is
                        the opt-in for a machine you named yourself with -I and
                        meant. (alias: --color; lanthorn also takes `theme`,
                        which needs a style.toml this binary has none of.)
      --period-look     Dress the screen as this story's own machine did: its
                        page and ink, its status band, its cursor shape. Only
                        for a v1-v4 story (colour arrives with v5, so anything
                        shown for one is presentation), and only where the
                        medium or -I names a machine we have a capture of. Off
                        by default — this is your terminal, not a pane we own;
                        lanthorn turns it on. Suppressed by --game-colours off
                        and NO_COLOR.
      --sound <on|off>  Sound: bleeps and sampled audio. Default on.
      --volume <0-100>  Set the master volume
  -I, --interpreter <n> Set the Z-machine interpreter number (ZMSD 11.1.3: 1
                        DECSystem-20, 2 Apple IIe, 3 Macintosh, 4 Amiga, 5 Atari
                        ST, 6 IBM PC, …). Overrides the medium: a story opened
                        off a release floppy defaults to that machine's number,
                        and this beats it.
      --data-dir <path> Base dir for saves/sidecars (default: beside the story)
      --machines        Print the ZMSD 11.1.3 machine table — every setting each
                        interpreter number carries (its default page and ink,
                        the palette those colour numbers resolve through, and
                        the two screen rules) — and exit. This is the table -I
                        selects a row of, and the one lanthorn presents from.
  -V, --version         Print version and exit
  -h, --help            Print this help and exit
";

/// [`HELP`] with its disk-media facts filled in **from `blorb::medium`'s table**
/// rather than typed out here.
///
/// The hand-written version said "an original Amiga release floppy (.adf) …
/// sets the interpreter number to the Amiga's 4", and had been wrong since
/// SQ-0833 and SQ-0835 added the DOS and Atari ST rows: the mount had opened
/// Macintosh, DOS and ST disks for months while the help still promised one
/// machine. That is the same defect SQ-0849 had just fixed on the TUI's side —
/// a second list of formats, kept by hand, going quietly stale — so this one is
/// generated. A new row in `FORMATS` reaches this text with no edit here.
///
/// The two substitutions are placed on lines of their own, and WRAPPED to
/// `cli_host::HELP_WIDTH` as they land (SQ-1093). Joining the lists and printing
/// them was a line nothing measured: the extension list had grown to 117 columns
/// and ran off the terminal's edge in the middle of a help whose every other line
/// is 80 or less, which is the ragged right margin the wrap fix exists to end.
fn help() -> String {
    use blorb::medium::DiskImage;
    /// The column the two substituted blocks sit at in [`HELP`].
    const INDENT: usize = 26;
    let exts: Vec<String> = comma_run(blorb::medium::image_extensions().map(|e| format!(".{e}")));
    // A machine's number is a DEFAULT the row states; a row may honestly have
    // none — the IBM PC's is version-dependent (6 for Version 6, 1 otherwise),
    // so `Fat12Dos` states nothing and the rule already in force stands.
    let numbered = comma_run(
        DiskImage::all()
            .filter_map(|d| d.interpreter_number().map(|n| format!("{} {n}", d.label()))),
    );
    let unnumbered: Vec<&str> =
        DiskImage::all().filter(|d| d.interpreter_number().is_none()).map(|d| d.label()).collect();
    let mut machines = cli_host::wrap_tokens(&numbered, INDENT);
    if !unnumbered.is_empty() {
        // Its own line: the clause is a sentence rather than a list item, and
        // reads as one only when it starts fresh.
        let clause = format!("{} states none — whatever default is in force stands", unnumbered.join(", "));
        let words: Vec<&str> = clause.split_whitespace().collect();
        machines.push('\n');
        machines.push_str(&" ".repeat(INDENT));
        machines.push_str(&cli_host::wrap_tokens(&words, INDENT));
    }
    HELP.replace("{DISK_EXTENSIONS}", &cli_host::wrap_tokens(&exts, INDENT))
        .replace("{DISK_MACHINES}", &machines)
}

/// The items of a comma-separated run, each carrying its own trailing comma so
/// `cli_host::wrap_tokens` can break between them and never inside one.
fn comma_run(items: impl Iterator<Item = String>) -> Vec<String> {
    let mut v: Vec<String> = items.collect();
    let last = v.len().saturating_sub(1);
    for (i, s) in v.iter_mut().enumerate() {
        if i != last {
            s.push(',');
        }
    }
    v
}

fn main() {
    let argv: Vec<String> = env::args().collect();
    if cli_host::handled_common_flags(&argv, &help(), env!("CARGO_BIN_NAME"), buildinfo::LONG) {
        return;
    }
    // Answered beside `--help`, and for the same reason: it describes the
    // program rather than a story, so demanding a story file to see it would be
    // the wrong question. Printed before anything reads a terminal.
    if argv.iter().any(|a| a == "--machines") {
        print!("{}", zvm::machines::table());
        return;
    }
    let args = match parse_args(&argv) {
        Ok(a) => a,
        Err(e) => cli_host::usage_error(env!("CARGO_BIN_NAME"), &e, &help()),
    };
    let Some(story_arg) = args.story.clone() else {
        cli_host::usage_error(env!("CARGO_BIN_NAME"), "no story file given", &help());
    };
    let story_path = std::path::PathBuf::from(&story_arg);

    // One decision about what this terminal can take, published process-wide so
    // the exit paths buried in the read helpers can reach it (SQ-0605).
    // `--plain`/`--screen-reader`, or TERM=dumb, turns off both escape output
    // and raw-mode line editing (SQ-0606). Settled before the story is read
    // because mounting a disk image may have to ask the player a question, and
    // whether there is anybody there to answer is exactly this decision.
    let mode = HostMode::detect_with(cli_host::plain_requested(&argv)).install();
    let stdout_is_tty = mode.rich();
    let stdin_is_tty = mode.raw_input();
    let both_tty = mode.both_tty();

    let story_bytes = match fs::read(&story_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error: cannot read '{}': {}", story_path.display(), e);
            std::process::exit(1);
        }
    };

    // What machine the medium says this is, read BEFORE the mount replaces the
    // image bytes with the story's (SQ-0839). The mapping is `blorb`'s, shared
    // with the TUI, so both front-ends default the same way off the same disk
    // and neither can drift: an Amiga floppy means interpreter 4, an ordinary
    // story file means nothing at all. `-I` still overrides it — see below.
    let medium = blorb::medium::DiskImage::detect(&story_bytes);

    // An original release floppy (SQ-0834): mount it and take a story off it.
    // A single-game disk opens straight away; a compilation asks which one; a
    // volume of a multi-disk release reaches for its siblings (SQ-0874), which
    // is why the PATH goes in and not only the bytes.
    // The question is `stdin_tty` — the device fact — not `stdin_is_tty`: a
    // screen-reader run has a real terminal in front of it and should be asked,
    // even though it wants no raw-mode line editing anywhere else.
    // SQ-0930: the mount also reports the medium THIS story came off, which on a
    // hybrid disc is not the image's own format. `medium` above is the image's;
    // this narrows it, exactly as `app::hints::read_story_file` has since SQ-0876.
    let mut medium = medium;
    let story_bytes = if media::looks_like_image(&story_bytes) {
        let (bytes, per_story) =
            mount_and_pick(&story_path, story_bytes, mode.stdin_tty(), args.story_pick.as_deref());
        if per_story.is_some() {
            medium = per_story;
        }
        bytes
    } else {
        story_bytes
    };

    // A .zblorb is a raw Blorb container; extract the embedded Z-code (reject
    // Glulx cleanly). Non-Blorb bytes (a raw .z5) pass through unchanged.
    let story_bytes = match extract_story(story_bytes) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    // Keep the original bytes for Restart.
    let original_bytes = story_bytes.clone();

    // Where this game's saves and sidecars go (SQ-0850). A plain story file is
    // keyed by its filename as it always was; a story taken off a disk image is
    // keyed by its OWN release and serial, because `--story` can pick any of the
    // six games on an ST compilation and the image's filename says nothing about
    // which. The rule is `cli_host`'s, shared with the TUI, so opening the same
    // game in either front-end reaches the same directory.
    let disk_build = medium.and_then(|kind| cli_host::DiskBuild::of(&story_bytes, kind));
    let game_dir = cli_host::game_dir_with_key(
        &story_path,
        args.data_dir.as_deref(),
        &cli_host::story_key_for(cli_host::StoryOrigin {
            path: &story_path,
            // `None`, stated rather than omitted (SQ-1098): the entry is what
            // keys a story out of a container with no builds, and the only
            // container that can be is a zip — which `zvm-cli` does not open.
            // A story off a disk image is keyed by the build beside this.
            entry: None,
            build: disk_build.as_ref(),
        }),
    );
    let aux_file = auxiliary::aux_path(&game_dir);

    // Restores the terminal on every way out of `main`, including a panic. The
    // explicit `restore` calls below hand it the scroll-region teardown; this
    // just guarantees it happens at all.
    let mut guard = TerminalGuard::new();

    // Force a steady block cursor (SQ-0281); reset to the terminal default on exit.
    if stdout_is_tty {
        print!("{}", cli_host::cursor_steady_block());
        let _ = io::stdout().flush();
    }

    // Paging is only safe when BOTH ends are TTYs (else it would block the
    // headless harness); --pager off disables it.
    let (mut term_rows, mut term_cols) = detect_term_size();
    // Plain mode also drops the pager: a [MORE] prompt is a blocking modal that
    // hides the rest of the output behind a keypress, which is exactly the shape
    // a screen reader cannot cope with (SQ-0606).
    let paging = both_tty && args.pager && !mode.plain();
    let mut page_height = cli_host::Pager::height_for(term_rows);
    // Timed reads (read/read_char time+routine) are honored unless disabled.
    let timed = args.timed_input;
    let sound_enabled = args.sound;
    let volume = args.volume.unwrap_or(100);

    // `NO_COLOR` (no-color.org) means colour, not layout: it drops the game's
    // colours exactly as `--game-colours off` does, and leaves the pinned status
    // line alone. Plain mode emits no escapes at all, so colour is moot there.
    let honor = args.honor_colours && !cli_host::no_color();
    // The override ordering, and it is a contract (SQ-0839): a number named on
    // the command line is the machine the player asked for and beats the
    // medium's own answer, which in turn beats zvm's default rule (`None` here —
    // Frotz's 6-for-v6, 1 otherwise). Same order the TUI's
    // `InterpreterProfile::resolve` applies; only the default moves.
    let interpreter = args
        .interpreter
        .or_else(|| medium.and_then(|m| m.interpreter_number()))
        // SQ-0930: …and a DOS medium NAMES the IBM PC. Its `interpreter_number` is
        // `None` because the machine's own number is a version rule, not because
        // the disk is silent — so reading the two alike left every PC build on a
        // hybrid disc with no machine at all, and no period look on a disc whose
        // paths literally say `PC/`. The TUI does this in
        // `Config::advertised_interpreter_number`.
        .or_else(|| {
            medium
                .filter(|m| m.implies_ibm_pc())
                .map(|_| zvm::interpreter::IBM_PC_INTERPRETER_NUMBER)
        });
    // SQ-0928: a machine's `$2C`/`$2D` describes a MACHINE, and running a story off
    // its release disk makes that description true of the launch. Naming a number
    // by hand does not — and now that the IBM PC states a pair (blue under white),
    // `-I 6` on a bare `.z5` would otherwise paint it. `--colour machine` is the
    // opt-in for a player who named the machine and meant it.
    let machine_colours = args.colour_machine.unwrap_or(args.interpreter.is_none());
    // SQ-0872: a number naming a machine `zvm::interpreter` does not model still
    // reaches `$1E` — that is the honest fallback, since the story asked and the
    // standard has an answer — but everything else about the presentation is then
    // the IBM PC's, and a silent substitution is what this quest exists to stop.
    // Say it once, on the explicit route only: the medium's own answers are all
    // modelled, so a disk launch stays quiet.
    if let Some(n) = args.interpreter.filter(|n| zvm::interpreter::machine(*n).is_none()) {
        eprintln!(
            "zvm: warning: interpreter {n} names a machine zvm does not model; \
             header $1E says {n}, but its default colours and palette are the IBM PC's"
        );
    }

    let mut machine = match build_machine(
        story_bytes,
        stdout_is_tty,
        paging,
        page_height,
        term_rows,
        term_cols,
        honor,
        interpreter,
        machine_colours,
        mode.plain(),
    ) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            // Past the guard, and `process::exit` runs no destructors, so the
            // guard's Drop never fires — restore explicitly (SQ-0913).
            cli_host::restore_and_exit(&crate::screen::leave_region(), 1);
        }
    };
    machine.set_honor_game_colours(honor);
    machine.set_sound_available(sound_enabled);
    aux_preload(&mut machine, &aux_file, args.aux);

    // SQ-0873: `--period-look` — dress the terminal as this story's own machine
    // did. Every clause of the gate below is load-bearing:
    //
    // - **v1-v4 only.** `set_colour` and the `$2C`/`$2D` header bytes are v5+, so
    //   a v1-v4 story has no colour concept for a page to be the DEFAULT of, and
    //   what a machine drew for one is presentation. For v5+ the machine's own
    //   pair is already seeded above, out of Infocom's source, as a fact the story
    //   can read — a different claim entirely, and this must not touch it.
    // - **`honor`**, which is `--game-colours off` and `NO_COLOR` folded together
    //   a hundred lines up. A player who said "keep my terminal's colours" is not
    //   answered by painting it a different colour of our own choosing.
    // - **a TTY, and not plain mode.** Escapes into a pipe are noise, and plain
    //   mode exists so a screen reader sees none at all.
    // - **a machine with a capture.** `period_look` is `None` for the Atari ST and
    //   the IBM PC, which have none, and for a number that names no row.
    //
    // The page and ink go through OSC 11/10 — the TERMINAL's own defaults, not
    // SGR — because every styled run ends with `\x1b[0m` and would otherwise drop
    // an SGR ink on the floor. The cursor goes through DECSCUSR, which states the
    // real shape rather than an approximation of it; `cli_host::term` records what
    // that cannot say (the colour).
    // Asked of `period_look_for` rather than read off the row, because one row
    // stores no pair: the IBM PC's screen is its own palette resolving the pair it
    // reports, and the Version is what picks the palette (SQ-0939/SQ-0983).
    let period_look = machine
        .mem
        .version()
        .le(&4)
        .then_some(interpreter)
        .flatten()
        .and_then(|n| zvm::interpreter::period_look_for(n, Some(machine.mem.version())))
        .filter(|_| args.period_look && honor && stdout_is_tty && !mode.plain());
    if let Some(look) = period_look {
        use zvm::interpreter::CursorShape;
        print!(
            "{}{}{}",
            cli_host::osc_set_bg(look.page),
            cli_host::osc_set_fg(look.ink),
            match look.cursor_shape {
                CursorShape::Block => cli_host::cursor_steady_block(),
                CursorShape::Underscore => cli_host::cursor_steady_underline(),
                CursorShape::Bar => cli_host::cursor_steady_bar(),
                // Unreachable behind the `version() <= 4` gate above — the reversed
                // cell is a Version 6 caret, and this front-end plays no v6 story.
                // It is also what a terminal draws when nobody states a shape, so
                // saying nothing IS saying it (SQ-0947).
                CursorShape::ReverseSpace => "",
            }
        );
        let _ = io::stdout().flush();
    }

    let mut sound: Option<CliSound> = if sound_enabled {
        let blorb = match blorb::resolve_resource_blorb(&story_path) {
            Some((b, path)) => {
                eprintln!("zvm: loaded sound resources from {}", path.display());
                Some(b)
            }
            None => None,
        };
        // The medium's own `Sound/` directory, when the story came off a disk.
        // Through the set seam, not `MountedDisk::mount`: the platter alone is
        // never the right question about a release, and a paged Apple II or
        // Commodore volume does not even mount on its own (SQ-0961).
        let disk: HashMap<u16, Vec<u8>> = match std::fs::read(&story_path)
            .ok()
            .and_then(|raw| cli_host::disk_set::mount_at(&story_path, raw).ok())
        {
            Some(d) => {
                let files = d.contents();
                let found = blorb::infocom_sound::from_volume(
                    files.iter().map(|(p, b)| (p.as_str(), b.as_slice())),
                );
                if !found.is_empty() {
                    eprintln!(
                        "zvm: {} sound effect(s) on the medium ({}){}",
                        found.len(),
                        found.keys().map(u16::to_string).collect::<Vec<_>>().join(", "),
                        // Say so where both exist, because the line above has already
                        // announced the Blorb and would otherwise read as the winner.
                        if blorb.is_some() { " — these outrank the blorb" } else { "" },
                    );
                }
                found.into_iter().map(|(e, (_, s))| (e, s.to_aiff())).collect()
            }
            None => HashMap::new(),
        };
        Some(CliSound {
            backend: audio::AudioBackend::new(volume),
            blorb,
            disk,
            ids: HashMap::new(),
            routines: HashMap::new(),
        })
    } else {
        None
    };

    // Plain mode does not narrate the status line every turn unless asked
    // (SQ-0612). `--story-only` still wins outright: it is the stronger,
    // already-documented switch and suppresses the upper window entirely.
    let quiet_status_line = mode.plain() && !args.show_status;
    let mut view = screen::ScreenView::new(
        stdout_is_tty,
        args.story_only,
        quiet_status_line,
        term_rows,
        term_cols,
    );
    view.set_pin(args.pin);
    // SQ-0873: the status band is the one part of a period look the terminal's
    // own defaults cannot carry — four of the five machines set it apart in four
    // different ways, and only one of those is `\x1b[7m`.
    view.set_period_look(period_look);
    // Screen-reader mode only, and for the same reason: on a TTY the menu is
    // painted in place and nothing repeats, and a plain pipe is a transcript
    // that must stay byte-identical (SQ-0609).
    let plain_menus = mode.plain() && !args.story_only;
    view.set_menus(plain_menus);
    // Screen-reader mode only: elsewhere the status line is on screen the whole
    // time, so announcing what it already says would be noise (SQ-0616).
    let mut score_watch = cli_host::ScoreWatch::new();
    let announce_scores = mode.plain() && !args.story_only;
    print!("{}", view.start());
    let _ = io::stdout().flush();

    // Page background: reflects the game's current bg onto the terminal's own
    // default background (OSC 11) so the whole window is painted, not just
    // coloured text runs. Honor- and stdout-TTY-gated; reset (OSC 111) on
    // every exit path below.
    let mut last_page_bg: Option<(u8, u8, u8)> = None;

    loop {
        let step = machine.step();
        for d in machine.diagnostics.drain(..) {
            eprintln!("zvm: warning: {d}");
        }
        // Bleeps + sampled sounds: drain the turn's sound events. Ring the bell for
        // #1/#2 (TTY only), and play audio when enabled.
        if !machine.pending_sounds.is_empty() {
            let events: Vec<zvm::cpu::exec::SoundEvent> = std::mem::take(&mut machine.pending_sounds);
            let beeps = events.iter().filter(|e| e.number == 1 || e.number == 2).count();
            if beeps > 0 {
                // The device fact, not `rich`: a bell is not an escape sequence
                // and a game ringing it is saying something, so plain mode keeps
                // it. Identical to `rich` in every other case.
                print!("{}", screen::bleep_bytes(beeps, mode.stdout_tty()));
                let _ = io::stdout().flush();
            }
            if let Some(cs) = sound.as_mut() {
                play_cli_sounds(cs, &events);
            }
        }
        // v3 show_status redraw request.
        if machine.screen.show_status_requested {
            print!("{}", view.frame(&machine));
            release_prompt(&mut machine);
            let _ = io::stdout().flush();
            machine.screen.show_status_requested = false;
        }
        // erase_window request (ZMSD §8.7.3): clear the lower window and reset
        // the pinned region so a game's screen clear / help-menu takeover (e.g.
        // Lost Pig's HELP, which issues erase_window -1) doesn't leave stale
        // story text bleeding through beneath the upper window.
        if machine.screen.erase_lower_requested {
            print!("{}", view.erase(machine.screen.current_bg, machine.honor_game_colours));
            let _ = io::stdout().flush();
            if let Some(o) = machine.out.as_any_mut().downcast_mut::<StdoutOutput>() {
                o.pager.reset();
                o.current_col = 0;
            }
            machine.screen.erase_lower_requested = false;
        }
        // Persist aux tables as soon as the game commits one.
        aux_flush(&mut machine, &aux_file, args.aux);

        match step {
            StepResult::Continue => {}

            StepResult::Quit => {
                // `view.leave()` is the renderer's own teardown (drop the scroll
                // region, park the cursor below it); the guard does the rest.
                guard.restore(&view.leave());
                break;
            }

            StepResult::Fault => {
                guard.restore(&view.leave());
                if let Some(trace) = machine.take_fault_trace() {
                    for line in trace.to_lines() {
                        eprintln!("{line}");
                    }
                }
                std::process::exit(70); // EX_SOFTWARE: internal software error
            }

            StepResult::Restart => {
                machine = match build_machine(
                    original_bytes.clone(),
                    stdout_is_tty,
                    paging,
                    page_height,
                    term_rows,
                    term_cols,
                    honor,
                    interpreter,
                    machine_colours,
                    mode.plain(),
                ) {
                    Ok(m) => m,
                    Err(e) => {
                        eprintln!("{e}");
                        // As above: past the guard, so restore explicitly.
                        cli_host::restore_and_exit(&crate::screen::leave_region(), 1);
                    }
                };
                machine.set_honor_game_colours(honor);
                machine.set_sound_available(sound_enabled);
                aux_preload(&mut machine, &aux_file, args.aux);
            }

            StepResult::NeedLine { .. } => {
                // Poll for terminal resize before line input (crossterm returns
                // current size; on piped stdout this is a no-op via is_tty guard).
                maybe_resize(both_tty, &mut term_rows, &mut term_cols, &mut page_height, &mut machine, &mut view);
                let frame = view.frame(&machine);
                print_frame(&mut machine, mode.plain(), &frame);
                // Between the status and the held prompt: the announcement is
                // about the turn that just ended, so it belongs with it rather
                // than after the prompt (SQ-0611/0616).
                announce_score(&mut machine, &mut score_watch, announce_scores);
                release_prompt(&mut machine);
                let _ = io::stdout().flush();
                let cur_bg = if stdout_is_tty && machine.honor_game_colours {
                    screen::zcolour_rgb(machine.screen.current_bg)
                } else {
                    None
                };
                if let Some(esc) = cli_host::page_bg_escape(cur_bg, last_page_bg) {
                    print!("{esc}");
                    let _ = io::stdout().flush();
                    last_page_bg = cur_bg;
                }
                // Echo input in the game's current style/colour (Default unless a
                // game set colour and honoring is on — matching the output sink).
                let echo = if honor {
                    zvm::io::TextAttrs {
                        style: machine.screen.text_style,
                        fg: machine.screen.current_fg,
                        bg: machine.screen.current_bg,
                    }
                } else {
                    zvm::io::TextAttrs::default()
                };
                let timeout = if timed { machine.pending_timeout() } else { None };
                // `/status` is answered by the host and the game never sees it,
                // so loop until a real command arrives (SQ-0610). A timed read
                // that expired is NOT re-read here: the interrupt has already
                // run and the game is owed its answer.
                let (line, terminator, resize, aborted) = loop {
                    let r = read_line_raw(stdin_is_tty, echo, &mut machine, &mut view, timeout, &mut sound);
                    if r.3 {
                        break r;
                    }
                    if cli_host::input::is_status_request(&r.0) {
                        let status = screen::ScreenView::status_now(&machine);
                        let text = if status.is_empty() { "[no status]".to_string() } else { status };
                        print_host_answer(&mut machine, &text);
                        continue;
                    }
                    // `/menu` re-reads the open menu, on the `/status` precedent
                    // (SQ-0609/0610). Menus are char-driven, so this arm mostly
                    // answers the polite refusal; the useful one is the cooked
                    // char reader.
                    if cli_host::is_menu_request(&r.0) {
                        let text = view.menu_listing().unwrap_or_else(|| NO_MENU.to_string());
                        print_host_answer(&mut machine, text.trim_end());
                        continue;
                    }
                    // `/pin` trades a pinned upper window for terminal scrollback,
                    // on the same host-answered precedent (SQ-0909). It is a
                    // toggle rather than a launch flag because the answer changes
                    // within one session: pinned while you read BeyondZork's
                    // compass, released while you scroll back through what you
                    // just did.
                    // `/pin` moves the status line and upper window between the
                    // top of the screen and the bottom, on the same host-answered
                    // precedent (SQ-0909). It is a runtime toggle and not only a
                    // launch flag because the reason to move it changes within one
                    // session: at the top while you play, at the bottom when you
                    // want to scroll back over what you just read.
                    if let Some(want) = cli_host::pin_request(&r.0, view.pin()) {
                        let text = match want {
                            Some(p) => {
                                view.set_pin(p);
                                p.note()
                            }
                            None => "[/pin takes top or bottom, or nothing to swap]",
                        };
                        print_host_answer(&mut machine, text);
                        continue;
                    }
                    break r;
                };
                if let Some((new_cols, new_rows)) = resize {
                    apply_resize(new_rows, new_cols, &mut term_rows, &mut term_cols,
                                 &mut page_height, &mut machine, &mut view);
                }
                if aborted {
                    machine.abort_timed_input(line.trim_end());
                } else {
                    machine.supply_line(line.trim_end(), terminator);
                }
                if let Some(o) = machine.out.as_any_mut().downcast_mut::<StdoutOutput>() {
                    o.pager.reset();
                    o.current_col = 0; // cursor is at line start after user input + Enter
                }
            }

            StepResult::NeedChar => {
                // Poll for terminal resize before char input.
                maybe_resize(both_tty, &mut term_rows, &mut term_cols, &mut page_height, &mut machine, &mut view);
                let frame = view.frame(&machine);
                print_frame(&mut machine, mode.plain(), &frame);
                // Between the status and the held prompt: the announcement is
                // about the turn that just ended, so it belongs with it rather
                // than after the prompt (SQ-0611/0616).
                announce_score(&mut machine, &mut score_watch, announce_scores);
                release_prompt(&mut machine);
                let _ = io::stdout().flush();
                let cur_bg = if stdout_is_tty && machine.honor_game_colours {
                    screen::zcolour_rgb(machine.screen.current_bg)
                } else {
                    None
                };
                if let Some(esc) = cli_host::page_bg_escape(cur_bg, last_page_bg) {
                    print!("{esc}");
                    let _ = io::stdout().flush();
                    last_page_bg = cur_bg;
                }
                let timeout = if timed { machine.pending_timeout() } else { None };
                // A host-driven menu walk feeds the game its own navigation keys
                // instead of reading stdin, so a typed number lands on the item
                // the player named (SQ-0609).
                // Asked after the frame above, so the walk steers from where the
                // marker actually landed rather than from a count made before
                // the first press (SQ-0609).
                let (ch, resize, aborted) = match view.next_menu_key() {
                    Some(key) => (key, None, false),
                    None if plain_menus => (read_cooked_char(&mut machine, &mut view), None, false),
                    None => {
                        read_char_input(stdin_is_tty, &mut machine, &mut view, timeout, &mut sound)
                    }
                };
                // Handle any resize that happened DURING the char read.
                if let Some((new_cols, new_rows)) = resize {
                    apply_resize(new_rows, new_cols, &mut term_rows, &mut term_cols,
                                 &mut page_height, &mut machine, &mut view);
                }
                if aborted {
                    machine.abort_timed_input("");
                } else {
                    machine.supply_char(ch);
                }
                if let Some(o) = machine.out.as_any_mut().downcast_mut::<StdoutOutput>() {
                    o.pager.reset();
                    o.current_col = 0;
                }
            }

            StepResult::SaveRequest => {
                // Plain mode holds the game's unterminated prompt back; let it
                // out before the dialog, or it surfaces after — out of order
                // (the invariant documented at `release_partial`, SQ-0635).
                release_prompt(&mut machine);
                // The list is a reminder of what you would collide with. A number
                // is NOT accepted here — see `cli_host::pick_save` on why an
                // overwrite has to be spelled out.
                let saves = cli_host::existing_saves(&game_dir, cli_host::QUETZAL_EXT);
                let filename = prompt_and_read_line("Save to file: ", &saves);
                handle_save_request(&mut machine, &game_dir, filename.trim());
            }

            StepResult::RestoreRequest => {
                release_prompt(&mut machine);
                let saves = cli_host::existing_saves(&game_dir, cli_host::QUETZAL_EXT);
                let filename = prompt_and_read_line("Restore from file: ", &saves);
                // A number picks from the list; anything else is a filename exactly
                // as before, so a save actually called `2` stays reachable.
                let filename =
                    cli_host::pick_save(&filename, &saves).map_or(filename.clone(), str::to_string);
                handle_restore_request(&mut machine, &game_dir, filename.trim());
            }
        }
    }
}

#[cfg(test)]
mod v6_tests {
    use super::*;

    /// A story file with just enough header to load: `version` in byte 0 and a
    /// plausible layout. Not runnable — `build_machine` only has to get as far
    /// as reading the version.
    pub(crate) fn story_of_version(v: u8) -> Vec<u8> {
        let mut buf = vec![0u8; 0x0800];
        buf[0x00] = v;
        buf[0x04] = 0x04; // high_mem_base = 0x0400
        buf[0x05] = 0x00;
        buf[0x06] = 0x00; // initial_pc = 0x0040
        buf[0x07] = 0x40;
        buf[0x0C] = 0x01; // object table
        buf[0x0D] = 0x00;
        buf[0x0E] = 0x02; // globals
        buf[0x0F] = 0x00;
        buf[0x18] = 0x00; // abbrev table
        buf[0x19] = 0x60;
        buf
    }

    fn build(story: Vec<u8>) -> Result<Machine, String> {
        build_machine(story, false, false, 24, 24, 80, true, None, true, false)
    }

    /// SQ-0601: v6 is a graphical, mouse-and-menu format this front-end cannot
    /// present, and every v6 story we have runs away the moment its opening
    /// screen asks for input — whatever key it is given. Zork Zero and Arthur
    /// flood the terminal; Shogun spins silently with nothing to interrupt.
    /// Refusing at load is the only outcome that neither crashes nor hangs.
    #[test]
    fn a_v6_story_is_refused_rather_than_run() {
        let err = match build(story_of_version(6)) {
            Err(e) => e,
            Ok(_) => panic!("v6 must be refused, not loaded"),
        };
        assert!(err.contains("not supported by zvm-cli"), "{err}");
        assert!(err.contains("lanthorn"), "the message points at the front-end that can: {err}");
    }

    /// The refusal is zvm-cli's alone — the library still supports v6, which is
    /// what lanthorn plays. A version check that lived in `Memory::new` would
    /// take the TUI's v6 support down with it.
    #[test]
    fn the_library_still_loads_a_v6_story() {
        let mem = Memory::new(story_of_version(6)).expect("zvm itself accepts v6");
        assert_eq!(mem.version(), 6);
    }

    #[test]
    fn the_ordinary_versions_still_load() {
        for v in [3u8, 5, 8] {
            assert!(build(story_of_version(v)).is_ok(), "v{v} must still load");
        }
    }

    /// SQ-0616. Where the score comes from is a different question per version,
    /// and getting it wrong is silent: v1-v3 have it in a global the standard
    /// pins down, v4+ do not, and a time game has a clock there instead.
    #[test]
    fn the_score_source_follows_the_story_version() {
        // Ask the header rather than assuming: the synthetic story puts static
        // memory at 0x0200, so a hard-coded globals base lands in read-only
        // memory and panics.
        let globals = |m: &Machine| m.mem.global_vars() as u32;

        // v3: the interpreter owns the status line, and global 2 is the score
        // (ZMSD §8.2). Exact, and it must survive being negative.
        let mut m = build(story_of_version(3)).expect("v3 builds");
        let g = globals(&m);
        m.mem.write_word(g + 2, 42);
        assert_eq!(current_score(&m), Some(42));
        m.mem.write_word(g + 2, (-7i16) as u16);
        assert_eq!(current_score(&m), Some(-7), "scores can go negative");

        // A v3 *time* game (Flags 1 bit 1) keeps a clock there, not a score.
        let mut time_story = story_of_version(3);
        time_story[0x01] |= 1 << 1;
        let m = build(time_story).expect("v3 time game builds");
        assert_eq!(current_score(&m), None, "a clock is not a score");

        // v4+: the game draws its own status line and the globals mean whatever
        // it likes, so the same global must NOT be read as a score.
        let mut m = build(story_of_version(5)).expect("v5 builds");
        let g = globals(&m);
        m.mem.write_word(g + 2, 42);
        assert_eq!(
            current_score(&m), None,
            "v4+ globals are the game's own; the score has to come from the text"
        );
    }

    /// SQ-0636: the v3 status bar is padded to the *tracked* terminal width,
    /// not a hard-coded 80 — on a narrower terminal an 80-column reverse-video
    /// bar wraps out of its 1-row pinned region into the story text every frame.
    #[test]
    fn the_v3_status_bar_is_padded_to_the_tracked_width() {
        let machine = build(story_of_version(3)).expect("v3 builds");
        let mut view = screen::ScreenView::new(true, false, false, 24, 40);
        let frame = view.frame(&machine);
        let start = frame.find("\x1b[7m").expect("v3 bar is reverse-video") + 4;
        let end = frame[start..].find("\x1b[0m").expect("bar closes reset") + start;
        assert_eq!(
            frame[start..end].chars().count(),
            40,
            "bar fills exactly the real width: {frame:?}"
        );

        // And a resize retunes it — the width is live, not construction-only.
        view.set_term_cols(60);
        let frame = view.frame(&machine);
        let start = frame.find("\x1b[7m").unwrap() + 4;
        let end = frame[start..].find("\x1b[0m").unwrap() + start;
        assert_eq!(frame[start..end].chars().count(), 60, "resized: {frame:?}");
    }

    /// SQ-0611. The sink writes to the real stdout, so what is testable here is
    /// the holding decision itself: with holding on, an unterminated prompt must
    /// still be pending after the write, leaving the stream at a line start for
    /// the status block to occupy — and gone once released.
    #[test]
    fn plain_mode_holds_the_prompt_so_the_status_can_precede_it() {
        let mut o = StdoutOutput::new(false, false, 24, 80, true, true);
        o.write_counted("You are in a room.\n");
        assert!(!o.hold.is_holding(), "a complete line goes straight out");
        o.write_counted("\n>");
        assert!(o.hold.is_holding(), "the prompt is held back");
        assert!(o.hold.at_line_start(), "so the status starts its own line");
        o.release_partial();
        assert!(!o.hold.is_holding(), "and the prompt follows the status");
    }

    #[test]
    fn without_plain_mode_nothing_is_ever_held() {
        // The pinned-region path must keep writing straight through: its status
        // never enters the text flow, so there is nothing to make room for.
        let mut o = StdoutOutput::new(true, false, 24, 80, true, false);
        o.write_counted("\n>");
        assert!(!o.hold.is_holding(), "the TTY path holds nothing");
    }
}

#[cfg(test)]
mod arg_tests {
    use super::*;

    #[test]
    fn parses_flags_and_story() {
        let a = parse_args(&["zvm-cli".into(), "--story-only".into(), "game.z5".into()]).expect("valid args");
        assert_eq!(a.story.as_deref(), Some("game.z5"));
        assert!(a.story_only && a.aux);

        let b = parse_args(&["zvm-cli".into(), "--aux".into(), "off".into(), "g".into()])
            .expect("valid args");
        assert!(!b.aux && !b.story_only);

        let c = parse_args(&["zvm-cli".into(), "g".into()]).expect("valid args");
        assert!(!c.story_only && c.aux);
    }

    /// `--story` (which story off a disk image) and `--story-only` (suppress the
    /// upper window) are different options that share a prefix; the scanner
    /// matches whole arguments, so asking for one never turns on the other.
    #[test]
    fn picking_a_story_off_a_disk_is_not_story_only() {
        let argv = ["zvm-cli", "--story", "2", "disk.adf"].map(String::from);
        let a = parse_args(&argv).expect("valid args");
        assert_eq!(a.story_pick.as_deref(), Some("2"));
        assert_eq!(a.story.as_deref(), Some("disk.adf"));
        assert!(!a.story_only);

        let b = parse_args(&["zvm-cli".into(), "--story-only".into(), "disk.adf".into()])
            .expect("valid args");
        assert!(b.story_only && b.story_pick.is_none());
    }

    /// SQ-0614. Every flag the binary accepts has to be listed in `parse_args`,
    /// even the ones whose value is read elsewhere, or a valid invocation would
    /// be rejected as unknown. This is the test that catches a flag added to
    /// `HELP` and to its reader but not to the parser.
    #[test]
    fn every_documented_flag_parses() {
        for flag in [
            "--story-only", "--lower-only", "--show-status", "--period-look",
            "--screen-reader", "--plain", "--scrollback",
        ] {
            let a = parse_args(&["zvm-cli".into(), flag.into(), "g".into()]).expect("valid args");
            assert_eq!(a.story.as_deref(), Some("g"), "{flag} should leave the story path alone");
        }
        // Value-taking flags: the value must be consumed, not read as the story.
        for (flag, value) in [
            ("--volume", "50"), ("-I", "6"), ("--interpreter", "6"),
            ("--data-dir", "/tmp/x"), ("--story", "2"), ("--pin", "bottom"),
            ("--aux", "on"), ("--pager", "off"), ("--timed-input", "on"),
            ("--sound", "off"), ("--game-colours", "on"), ("--game-colors", "on"),
            ("--colour", "machine"), ("--color", "terminal"),
        ] {
            let a = parse_args(&["zvm-cli".into(), flag.into(), value.into(), "g".into()]).expect("valid args");
            assert_eq!(a.story.as_deref(), Some("g"), "{flag} {value} swallowed the story path");
        }
    }

    /// SQ-1082. Every negative-only switch is `--<noun> on|off` now, the value is
    /// required, and the old spelling is gone outright — pre-release, an alias is
    /// only the old name living on somewhere nobody maintains it.
    ///
    /// `--no-status` goes with them and does NOT become `--status on|off`:
    /// SQ-0613 renamed it to `--story-only` precisely because it was STRONGER
    /// than its name (it suppresses the whole upper window, menus included), and
    /// `--status` would re-tell the same lie in the new grammar.
    #[test]
    fn the_negative_only_spellings_are_gone_and_the_new_ones_need_a_value() {
        let err = |args: &[&str]| {
            let v: Vec<String> = std::iter::once("zvm-cli".to_string())
                .chain(args.iter().map(|s| s.to_string()))
                .collect();
            parse_args(&v).expect_err("should be rejected")
        };
        for old in [
            "--no-aux", "--no-more", "--no-page", "--no-timed-input", "--no-sound",
            "--no-game-colours", "--no-status", "--system-colours", "--system-colors",
        ] {
            assert!(err(&[old, "g"]).contains("unknown option"), "{old} should be gone");
        }
        // A bare form is an ambiguity ("is that on, or a toggle?"), not a
        // shorthand: the scanner takes the story path as the value and then finds
        // no story, or refuses the value outright.
        for bare in ["--aux", "--pager", "--timed-input", "--sound", "--game-colours"] {
            let e = err(&[bare, "g"]);
            assert!(e.contains("takes on or off"), "{bare}: {e}");
        }
        assert!(err(&["--sound"]).contains("--sound needs a value"));
        assert!(err(&["--colour", "chartreuse", "g"]).contains("terminal or machine"));
        // The value `lanthorn` takes and this binary cannot, named rather than
        // merely absent: it is the right word at the wrong front-end.
        assert!(err(&["--colour", "theme", "g"]).contains("style.toml"));
    }

    /// SQ-1093. One wrap authority across all four front-ends, and this is the
    /// help that showed two of them at once: prose hand-wrapped to about 83
    /// columns, and a disk-format list joined from `blorb::medium`'s table that
    /// nothing measured and that had reached 117.
    ///
    /// Asserted on `help()`, not on `HELP`, because the two substituted runs are
    /// exactly the part that went unmeasured — a new row in `FORMATS` reaches
    /// this text with no edit here, and must not push it off the edge.
    #[test]
    fn every_help_line_fits_the_one_width_all_four_front_ends_share() {
        let text = help();
        let over = cli_host::overlong_help_lines(&text);
        assert!(
            over.is_empty(),
            "--help must wrap at {}, but {over:?} do not:\n{text}",
            cli_host::HELP_WIDTH
        );
        assert!(!text.contains('{'), "every substitution was made: {text}");
        assert!(
            text.lines().filter(|l| l.chars().count() > cli_host::HELP_WIDTH - 10).count() > 5,
            "the text should be filling the width, not merely short of it"
        );
    }

    /// SQ-0614. A mistyped flag used to be ignored outright: `--no-statu` did
    /// nothing and the process exited 0, so the spelling was never the suspect.
    #[test]
    fn unknown_and_malformed_arguments_are_rejected() {
        let err = |args: &[&str]| {
            let v: Vec<String> = std::iter::once("zvm-cli".to_string())
                .chain(args.iter().map(|s| s.to_string()))
                .collect();
            parse_args(&v).expect_err("should be rejected")
        };
        assert!(err(&["--no-statu", "g"]).contains("unknown option: --no-statu"));
        // Single-dash forms too: `-x` used to be taken as the story path and
        // then reported as a missing file.
        assert!(err(&["-x"]).contains("unknown option: -x"));
        assert!(err(&["--data-dir"]).contains("--data-dir needs a value"));
        assert!(err(&["a.z5", "b.z5"]).contains("unexpected extra argument: b.z5"));
    }

    /// SQ-0613. `--no-status` read as the same thing plain mode does to the
    /// status line while being stronger than it, so it was renamed; SQ-1082
    /// dropped the surviving alias. Both remaining spellings still select it.
    #[test]
    fn both_spellings_select_story_only() {
        for flag in ["--story-only", "--lower-only"] {
            let a = parse_args(&["zvm-cli".into(), flag.into(), "g".into()]).expect("valid args");
            assert!(a.story_only, "{flag} should select story-only");
            assert_eq!(a.story.as_deref(), Some("g"), "{flag} is not the story path");
        }
    }

    /// SQ-1082. Each converted switch says both things, and says nothing when it
    /// is absent — which here means the default beside it in `Args`, `zvm-cli`
    /// having no config file for a third state to protect.
    #[test]
    fn the_converted_switches_say_on_off_and_default_on() {
        let p = |args: &[&str]| {
            let v: Vec<String> = std::iter::once("zvm-cli")
                .chain(args.iter().copied())
                .map(String::from)
                .collect();
            parse_args(&v).expect("valid args")
        };
        for (flag, get) in [
            ("--aux", (|a: &Args| a.aux) as fn(&Args) -> bool),
            ("--pager", |a: &Args| a.pager),
            ("--timed-input", |a: &Args| a.timed_input),
            ("--sound", |a: &Args| a.sound),
            ("--game-colours", |a: &Args| a.honor_colours),
        ] {
            assert!(get(&p(&["g"])), "{flag} defaults on");
            assert!(get(&p(&[flag, "on", "g"])), "{flag} on");
            assert!(!get(&p(&[flag, "off", "g"])), "{flag} off");
        }
    }

    /// SQ-1082. `--colour` is `--system-colours` said on the axis it belongs to,
    /// with the arm nobody could ask for before: declining a machine the MEDIUM
    /// named. Unset leaves SQ-0928's rule alone — the disk licenses the pair, a
    /// number you typed does not.
    #[test]
    fn colour_names_the_default_page_source_and_unset_leaves_the_medium_to_decide() {
        let p = |args: &[&str]| {
            let v: Vec<String> = std::iter::once("zvm-cli")
                .chain(args.iter().copied())
                .map(String::from)
                .collect();
            parse_args(&v).expect("valid args")
        };
        assert_eq!(p(&["g"]).colour_machine, None, "unset: the medium decides");
        assert_eq!(p(&["--colour", "machine", "g"]).colour_machine, Some(true));
        assert_eq!(p(&["--colour", "terminal", "g"]).colour_machine, Some(false));
        assert_eq!(p(&["--color", "machine", "g"]).colour_machine, Some(true), "US spelling");
        // The rule `machine_colours` applies, stated here so the three states are
        // visible together: unset means "did anyone but me name this machine?".
        let licensed = |a: &Args| a.colour_machine.unwrap_or(a.interpreter.is_none());
        assert!(licensed(&p(&["g"])), "nothing named a machine by hand");
        assert!(!licensed(&p(&["-I", "4", "g"])), "a number you typed licenses nothing");
        assert!(licensed(&p(&["-I", "4", "--colour", "machine", "g"])), "…until you say so");
        assert!(!licensed(&p(&["--colour", "terminal", "g"])), "and it declines both ways");
    }

    /// These were three separate scans of argv; they read off the one scan now
    /// (SQ-0614), so the behaviour is pinned through `parse_args`.
    #[test]
    fn value_options_are_read_leniently() {
        let p = |args: &[&str]| {
            let v: Vec<String> = std::iter::once("zvm-cli")
                .chain(args.iter().copied())
                .map(String::from)
                .collect();
            parse_args(&v).expect("valid args")
        };
        assert_eq!(p(&["--volume", "60", "g"]).volume, Some(60));
        assert_eq!(p(&["--volume", "200", "g"]).volume, Some(100), "clamped");
        assert_eq!(p(&["g"]).volume, None);

        assert!(p(&["g"]).honor_colours);
        assert!(!p(&["--game-colours", "off", "g"]).honor_colours);
        assert!(!p(&["g"]).period_look, "off by default: this is the user's terminal");
        assert!(p(&["--period-look", "g"]).period_look);

        assert_eq!(p(&["-I", "4", "g"]).interpreter, Some(4));
        assert_eq!(p(&["--interpreter", "3", "g"]).interpreter, Some(3));
        assert_eq!(p(&["g"]).interpreter, None);
        // Lenient: a bad value falls back to the engine default rather than
        // refusing to start.
        assert_eq!(p(&["-I", "notanumber", "g"]).interpreter, None);
    }

}

#[cfg(test)]
mod stdin_eof_tests {
    // The implementation moved to `cli_host::input` (SQ-0605); zvm-cli keeps its
    // own regression test, because this is one of the two crates the bug shipped
    // in — and it shipped here twice, the char path fixed months before the line
    // path (`zvm-cli gostak.z5 < /dev/null` printed 90 KB of prompts until it
    // was killed).
    use cli_host::read_byte_or_eof;

    #[test]
    fn true_eof_returns_none_instead_of_looping() {
        // A closed/empty stdin yields a 0-byte read_line, which must be
        // reported as EOF (None) rather than a synthesized b'\n' — the bug
        // that caused read_byte_stdin() to busy-spin forever on piped input.
        let mut empty: &[u8] = b"";
        assert_eq!(read_byte_or_eof(&mut empty), None);
    }

    #[test]
    fn blank_line_is_not_confused_with_eof() {
        let mut input: &[u8] = b"\n";
        assert_eq!(read_byte_or_eof(&mut input), Some(b'\n'));
    }

    #[test]
    fn returns_first_byte_of_line() {
        let mut input: &[u8] = b"abc\n";
        assert_eq!(read_byte_or_eof(&mut input), Some(b'a'));
    }
}

#[cfg(test)]
mod stdout_tests {
    // The sink writes to the real stdout, so its behavior is exercised by the
    // manual smoke; this pins the wrapping helper the sink must use.
    #[test]
    fn print_styled_wraps_only_on_tty() {
        use zvm::io::TextAttrs;
        assert_eq!(crate::screen::style_wrap("hi", TextAttrs { style: 2, ..Default::default() }, true), "\x1b[1mhi\x1b[0m");
        assert_eq!(crate::screen::style_wrap("hi", TextAttrs { style: 2, ..Default::default() }, false), "hi");
    }

    /// CLI gate: when honor_game_colours is OFF, print_attr strips fg/bg before
    /// passing to style_wrap so no colour SGR is emitted — but reverse/bold/italic
    /// style bits are always preserved.
    #[test]
    fn honour_off_strips_colour_preserves_style_bits() {
        use zvm::io::TextAttrs;
        use zvm::screen::ZColour;
        // Attrs with fg=red (Standard(3)→SGR 31), bg=blue (Standard(6)→SGR 44),
        // and reverse+bold style bits.
        let attrs = TextAttrs { style: 0x03, fg: ZColour::Standard(3), bg: ZColour::Standard(6) };

        // With honour ON: colour SGR present.
        let with_honour = crate::screen::style_wrap("hi", attrs, true);
        assert!(with_honour.contains("31"), "fg red SGR present with honour: {with_honour:?}");
        assert!(with_honour.contains("44"), "bg blue SGR present with honour: {with_honour:?}");

        // With honour OFF: strip fg/bg, pass Default channels to style_wrap.
        // This mirrors what StdoutOutput::print_attr does when honor_game_colours=false.
        let stripped = TextAttrs { fg: ZColour::Default, bg: ZColour::Default, ..attrs };
        let without_honour = crate::screen::style_wrap("hi", stripped, true);
        assert!(!without_honour.contains("31"), "fg colour absent when honour=false: {without_honour:?}");
        assert!(!without_honour.contains("44"), "bg colour absent when honour=false: {without_honour:?}");
        // Reverse (7) and bold (1) must still be present.
        assert!(without_honour.contains('7'), "reverse SGR preserved: {without_honour:?}");
        assert!(without_honour.contains('1'), "bold SGR preserved: {without_honour:?}");
    }
}

#[cfg(test)]
mod keycode_tests {
    use super::*;
    use crossterm::event::KeyCode;

    // ── commit_line_bytes (SQ-0920) ─────────────────────────────────────────

    /// **The background is applied BEFORE the newline**, because that newline
    /// scrolls and the terminal erases the exposed line with the background in
    /// force. Reversed, the game's next line gets a pale tail beyond its text.
    #[test]
    fn the_commit_newline_carries_the_games_background() {
        let out = commit_line_bytes("\x1b[44m");
        let nl = out.find("\r\n").expect("commits the line");
        let bg = out.find("\x1b[44m").expect("re-applies the background");
        assert!(bg < nl, "background must precede the newline that scrolls: {out:?}");
        assert!(out.ends_with("\x1b[0m"), "and be dropped again after it: {out:?}");
    }

    /// With no game background there is nothing to re-apply, so the bytes are what
    /// they always were — a story that never sets a colour is byte-identical.
    #[test]
    fn no_game_background_means_no_extra_bytes() {
        assert_eq!(commit_line_bytes(""), "\r\n");
    }

    #[test]
    fn decode_keycode_printable_ascii() {
        assert_eq!(decode_keycode(KeyCode::Char('a')), b'a');
        assert_eq!(decode_keycode(KeyCode::Char('Z')), b'Z');
        assert_eq!(decode_keycode(KeyCode::Char('5')), b'5');
    }

    #[test]
    fn decode_keycode_special_keys() {
        assert_eq!(decode_keycode(KeyCode::Enter), b'\n');
        assert_eq!(decode_keycode(KeyCode::Backspace), 8);
        assert_eq!(decode_keycode(KeyCode::Esc), 0x1B);
        assert_eq!(decode_keycode(KeyCode::Up), 129);
        assert_eq!(decode_keycode(KeyCode::Down), 130);
        assert_eq!(decode_keycode(KeyCode::Left), 131);
        assert_eq!(decode_keycode(KeyCode::Right), 132);
        assert_eq!(decode_keycode(KeyCode::F(1)), 133);
        assert_eq!(decode_keycode(KeyCode::F(4)), 136);
        assert_eq!(decode_keycode(KeyCode::F(5)), 137);
        assert_eq!(decode_keycode(KeyCode::F(12)), 144);
    }
}

#[cfg(test)]
mod wrap_tests {
    use super::wrap_line;

    #[test]
    fn no_wrap_when_enough_space() {
        let (out, col) = wrap_line("hello world", 80, 0);
        assert_eq!(out, "hello world");
        assert_eq!(col, 11);
    }

    #[test]
    fn wraps_word_that_overflows_line() {
        // "world" at col 76: 76+5=81 > 80 → soft newline then "world"
        let (out, col) = wrap_line("world", 80, 76);
        assert_eq!(out, "\nworld");
        assert_eq!(col, 5);
    }

    #[test]
    fn no_wrap_at_line_start_even_if_word_is_long() {
        // A word longer than cols at col 0 is printed as-is (avoids infinite wrap).
        let long = "x".repeat(100);
        let (out, col) = wrap_line(&long, 80, 0);
        assert_eq!(out, long);
        assert_eq!(col, 100);
    }

    #[test]
    fn buffer_mode_off_disables_wrap() {
        // With cols = u16::MAX (buffer_mode off), no soft newline is ever inserted.
        let text = "this is a very long sentence that would normally be wrapped at 80 columns";
        let (out, _) = wrap_line(text, u16::MAX, 0);
        assert_eq!(out, text);
        assert!(!out.contains('\n'));
    }

    #[test]
    fn explicit_newline_resets_column() {
        let (out, col) = wrap_line("foo\nbar", 80, 70);
        assert_eq!(out, "foo\nbar");
        assert_eq!(col, 3);
    }

    #[test]
    fn trims_space_token_that_triggers_wrap() {
        // A space token at col 80 triggers a wrap; the space itself is trimmed.
        // " " at col 80: 80+1=81>80 → wrap, trim(" ")="", col=0
        // "world" at col 0: 0+5=5<=80 → emit, col=5
        let (out, col) = wrap_line(" world", 80, 80);
        assert_eq!(out, "\nworld");
        assert_eq!(col, 5);
    }

    #[test]
    fn multi_word_wrap_sequence() {
        // At cols=10, starting at col 0: "hello world" wraps.
        let (out, col) = wrap_line("hello world", 10, 0);
        // "hello " is 6 chars (≤10), then "world" (5) pushes 6+5=11 > 10 → wrap
        assert!(out.contains('\n'), "expected soft newline: {out:?}");
        assert_eq!(col, 5); // "world" = 5 chars after the wrap
    }
}

/// SQ-0702 — game-centred text has to land where the game centred it.
///
/// Reported against Anchorhead's title splash: `A N C H O R H E A D` came out
/// visibly off-centre in `zvm-cli` while the very next line looked right, and
/// lanthorn showed both correctly. The splash prints its centring indent one
/// BOLD space at a time, and the sink styled each write before measuring it —
/// so `\x1b[1m \x1b[0m` was charged nine columns instead of one, the sink
/// believed it had passed the right margin after eight spaces, and it inserted
/// a soft wrap in the middle of the title.
///
/// ZMSD §8.8.3.1.2.2 is the rule that was broken: "If 'buffered printing' is on,
/// then text is wrapped after the last word which could fit on a line." An SGR
/// escape is not part of any word and takes up no column.
///
/// (The *other* line, `[Press 'R' to restore…]`, was never wrong: it is plain
/// text, and its indent is byte-for-byte what lanthorn emits at the same width —
/// Anchorhead simply centres it inside a margin of its own. It is asserted here
/// as the control that says so.)
#[cfg(test)]
mod centring_tests {
    use super::*;
    use std::path::PathBuf;
    use zvm::io::TextAttrs;

    const BOLD: TextAttrs = TextAttrs { style: 2, fg: zvm::screen::ZColour::Default, bg: zvm::screen::ZColour::Default };

    // ── the root cause, at the unit ─────────────────────────────────────────

    #[test]
    fn a_styled_space_costs_one_column_not_nine() {
        let (out, col) = format_output(" ", Some(BOLD), 80, 0, true);
        assert_eq!(out, "\x1b[1m \x1b[0m", "the bytes are unchanged — only the accounting was wrong");
        assert_eq!(col, 1, "an SGR escape occupies no column (ZMSD §8.8.3.1.2.2)");
    }

    /// The reported shape: a centring indent printed one styled space at a time
    /// must reach its column with no soft wrap in it. At 80 columns the old
    /// order broke after the eighth space (8 × 9 = 72, and the ninth reset put
    /// it past 80).
    #[test]
    fn a_styled_centring_indent_does_not_wrap_early() {
        let mut col = 0u16;
        let mut out = String::new();
        for _ in 0..29 {
            let (bytes, new_col) = format_output(" ", Some(BOLD), 80, col, true);
            out.push_str(&bytes);
            col = new_col;
        }
        assert_eq!(col, 29, "29 styled spaces are 29 columns");
        assert!(!out.contains('\n'), "no soft wrap belongs inside an indent that fits: {out:?}");
    }

    // ── the whole title splash, driven from the real story ──────────────────

    /// Every write the game made, as the sink saw it: `(text, attrs, buffered)`.
    type Writes = Vec<(String, Option<TextAttrs>, bool)>;

    /// Records the game's stream exactly where `StdoutOutput` sits, so the test
    /// can replay it through the real formatter instead of a copy of it.
    #[derive(Default)]
    struct Recorder {
        buffer_mode: bool,
        writes: Writes,
    }

    impl Output for Recorder {
        fn print(&mut self, s: &str) {
            self.writes.push((s.to_string(), None, self.buffer_mode));
        }
        fn print_styled(&mut self, s: &str, style: u8) {
            self.print_attr(s, TextAttrs { style, ..Default::default() });
        }
        fn print_attr(&mut self, s: &str, attrs: TextAttrs) {
            self.writes.push((s.to_string(), Some(attrs), self.buffer_mode));
        }
        fn set_buffer_mode(&mut self, on: bool) {
            self.buffer_mode = on;
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }

    /// Boot `anchor.z8` to its first keypress at `cols` columns, exactly as
    /// `build_machine` sets a game up, and return what it printed. `None` when
    /// the gitignored story is absent (CI).
    fn boot_anchor(cols: u16) -> Option<Writes> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/anchor.z8");
        let Ok(bytes) = fs::read(&path) else {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            return None;
        };
        let mem = Memory::new(bytes).expect("anchor.z8 loads");
        let mut m = Machine::with_output(mem, Box::new(Recorder::default()));
        m.init_caps();
        m.set_screen_dims(24, cols.min(255) as u8);
        for _ in 0..20_000_000 {
            if !matches!(m.step(), StepResult::Continue) {
                break;
            }
        }
        let rec = m.out.as_any().downcast_ref::<Recorder>().expect("recorder");
        assert!(!rec.writes.is_empty(), "the title splash must have printed something");
        Some(rec.writes.clone())
    }

    /// Replay `writes` through the sink's own formatter at `cols`.
    fn render(writes: &Writes, cols: u16) -> String {
        let mut out = String::new();
        let mut col = 0u16;
        for (text, attrs, buffered) in writes {
            let width = if *buffered { cols } else { u16::MAX };
            let (bytes, new_col) = format_output(text, *attrs, width, col, true);
            out.push_str(&bytes);
            col = new_col;
        }
        out
    }

    /// Drop SGR sequences — what the player's terminal is left showing.
    fn visible(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c != '\x1b' {
                out.push(c);
                continue;
            }
            if chars.next() != Some('[') {
                continue;
            }
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        }
        out
    }

    /// The indent of the one line containing `needle`, or a panic naming what
    /// went wrong (a split title has no such line at all — the reported bug).
    fn indent_of(text: &str, needle: &str, what: &str) -> usize {
        let line = text
            .lines()
            .find(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("{what}: no single line holds {needle:?}; rendered:\n{text}"));
        line.len() - line.trim_start_matches(' ').len()
    }

    #[test]
    fn anchorheads_title_splash_lands_where_the_game_centred_it() {
        for cols in [60u16, 80, 100] {
            let Some(writes) = boot_anchor(cols) else { return };

            // What the GAME laid out, before any host wrapping: the reference.
            let raw: String = writes.iter().map(|(t, _, _)| t.as_str()).collect();
            // What zvm-cli actually puts on the terminal.
            let shown = visible(&render(&writes, cols));

            const TITLE: &str = "A N C H O R H E A D";
            const PROMPT: &str = "[Press 'R' to restore";

            let title = indent_of(&shown, TITLE, &format!("{cols} cols, title"));
            assert_eq!(
                title,
                indent_of(&raw, TITLE, "the game's own stream"),
                "{cols} cols: rendering must not spend or invent columns the game did not"
            );
            let ideal = (cols as usize - TITLE.chars().count()) / 2;
            assert!(
                title.abs_diff(ideal) <= 1,
                "{cols} cols: the title sits at column {title}, centre is {ideal}"
            );

            // Control: plain text, and already correct before the fix. Its
            // indent is byte-identical to lanthorn's at the same width, so the
            // game — not the host — is what puts it left of dead centre.
            let prompt = indent_of(&shown, PROMPT, &format!("{cols} cols, prompt"));
            assert_eq!(
                prompt,
                indent_of(&raw, PROMPT, "the game's own stream"),
                "{cols} cols: the plain line was never the bug and must stay put"
            );
            assert_eq!(prompt, (cols as usize - 50) / 2, "{cols} cols: Anchorhead's own margin");
        }
    }

    // ── declared width == rendered width ────────────────────────────────────

    /// The other half of "centred correctly": the width the story is told
    /// (ZMSD §8.4, bytes $20/$21; §8.4.3, words $22/$24) has to be the width the
    /// sink wraps at. A story centring against one number while the host wraps
    /// at another drifts at every width.
    #[test]
    fn the_width_declared_to_the_story_is_the_width_the_sink_renders_at() {
        for cols in [40u16, 60, 80, 100, 132] {
            let m = build_machine(v6_tests::story_of_version(5), true, false, 24, 24, cols, true, None, true, false)
                .expect("a v5 story builds");
            assert_eq!(m.mem.read_byte(0x21) as u16, cols, "$21 = screen width in characters (§8.4)");
            assert_eq!(m.mem.read_word(0x22), cols, "$22 = screen width in units (§8.4.3)");
            let sink = m.out.as_any().downcast_ref::<StdoutOutput>().expect("the stdout sink");
            assert_eq!(sink.cols, cols, "the sink wraps at exactly what the story was told");
        }
    }

    /// …and a resize keeps all three in step — the header, the sink, and the
    /// upper window the game already split.
    ///
    /// That last one is why this goes through `Machine::set_screen_dims` rather
    /// than the bare `zvm::screen::write_screen_dims` it wraps. Writing only the
    /// header leaves a game that splits ONCE at boot and never re-splits —
    /// Sherlock, Trinity — centring its status band against the new width inside
    /// a grid still the old one (SQ-0679, which the app has had all along).
    #[test]
    fn a_resize_retells_the_story_and_retunes_the_sink_together() {
        let mut m = build_machine(v6_tests::story_of_version(5), true, false, 24, 24, 80, true, None, true, false)
            .expect("a v5 story builds");
        // A live one-row upper window, as a boot-time `split_window 1` leaves.
        m.screen.upper.resize(1, 80);
        m.screen.upper_window_rows = 1;
        let mut view = screen::ScreenView::new(true, false, false, 24, 80);
        let (mut rows, mut cols, mut page) = (24u16, 80u16, 20u16);
        for (r, c) in [(30u16, 132u16), (20, 50)] {
            apply_resize(r, c, &mut rows, &mut cols, &mut page, &mut m, &mut view);
            assert_eq!(m.mem.read_byte(0x21) as u16, c, "the story is told the new width");
            assert_eq!(m.mem.read_byte(0x20) as u16, r, "…and the new height");
            let sink = m.out.as_any().downcast_ref::<StdoutOutput>().expect("the stdout sink");
            assert_eq!(sink.cols, c, "the sink follows it");
            assert_eq!(m.screen.upper.cols, c, "and so does the grid the game is drawing into");
        }
    }
}

#[cfg(test)]
mod sound_idmap_tests {
    use std::collections::HashMap;

    // Mirrors the `cs.ids.retain(|_, v| *v != id)` line in `poll_sound_finish`:
    // a finished sound's number->id mapping must be cleared even when it has no
    // finish routine (Bug A — previously this only ran inside the
    // `if let Some(routine) = cs.routines.remove(&id)` branch, so a routine-less
    // sound left a stale entry). Exercises the exact retain predicate against a
    // plain `HashMap<u16, SoundId>`, device-free.
    #[test]
    fn retain_clears_finished_id_without_routine() {
        let id: audio::SoundId = 7;
        let mut ids: HashMap<u16, audio::SoundId> = HashMap::new();
        ids.insert(3, id); // sound #3 played with routine == 0 → no `routines` entry
        ids.insert(4, 99); // unrelated sound, still playing

        ids.retain(|_, v| *v != id);

        assert!(!ids.contains_key(&3), "finished id must be cleared even without a routine");
        assert_eq!(ids.get(&4), Some(&99), "unrelated entries must be untouched");
    }
}

#[cfg(test)]
mod restore_request_tests {
    use super::*;

    /// Minimal but structurally valid v4 story buffer. Mirrors zvm's own
    /// `header::tests_support::sample_story`, which is `pub(crate)` to the
    /// `zvm` crate and not visible from here.
    fn sample_v4_story() -> Vec<u8> {
        let mut buf = vec![0u8; 0x400];
        buf[0x00] = 4; // version
        buf[0x04] = 0x04;
        buf[0x05] = 0x00; // high_mem_base = 0x0400
        buf[0x06] = 0x00;
        buf[0x07] = 0x40; // initial_pc = 0x0040
        buf[0x08] = 0x02;
        buf[0x09] = 0x00; // dictionary = 0x0200
        buf[0x0A] = 0x01;
        buf[0x0B] = 0x00; // object_table = 0x0100
        buf[0x0C] = 0x03;
        buf[0x0D] = 0x00; // global_vars = 0x0300
        buf[0x0E] = 0x04;
        buf[0x0F] = 0x00; // static_mem_base = 0x0400
        buf[0x18] = 0x00;
        buf[0x19] = 0x40; // abbrev_table = 0x0040
        buf
    }

    /// A v4 story: `save -> G0` at 0x40 (store form, one store byte), then quit.
    fn save_v4_into_g0_story() -> Vec<u8> {
        let mut buf = sample_v4_story();
        buf[0x40] = 0xB5; // 0OP:0x05 save (store form, v4+)
        buf[0x41] = 0x10; // store -> global 0 (var 0x10)
        buf[0x42] = 0xBA; // quit
        buf
    }

    /// SQ-0283 Task 7: the `RestoreRequest` arm must call
    /// `machine.complete_restore_success(&data)`, not the raw
    /// `machine.restore_quetzal(&data)`. The raw call would leave the
    /// machine resuming AT the `@save`'s result descriptor (the store byte
    /// itself, per Quetzal §5.8) with that descriptor never resolved;
    /// `complete_restore_success` completes it forward — storing 2 into the
    /// `@save`'s result and resuming PAST the descriptor — matching how the
    /// app completes an in-game restore.
    #[test]
    fn restore_request_completes_the_save_descriptor_forward() {
        let mem = Memory::new(save_v4_into_g0_story()).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x40;

        let r = m.step();
        assert_eq!(r, StepResult::SaveRequest, "save opcode suspends with SaveRequest");
        assert_eq!(m.state.pc, 0x42, "PC is post-instruction after @save suspends");

        let blob = m.save_quetzal();
        m.complete_save(true);
        assert_eq!(m.global(0), 1, "save success stored 1 into G0");

        // Clobber state the way later play would, so the restore must actually reset it.
        m.do_store(Some(0x10), 0x99);
        m.state.pc = 0x00AB;

        // This is the exact call zvm-cli's RestoreRequest arm makes.
        m.complete_restore_success(&blob).expect("in-game restore must succeed");

        assert_eq!(m.global(0), 2, "descriptor advanced: the original @save 'returns' 2");
        assert_eq!(m.state.pc, 0x42, "execution resumes PAST the @save, not at its descriptor");

        // A properly-resumed machine must not immediately re-suspend.
        let r2 = m.step();
        assert_ne!(r2, StepResult::SaveRequest, "resumed machine does not re-suspend on save");
    }

    /// A v4 story: `save -> G0`, then `restore -> G1`, then quit.
    fn save_then_restore_story() -> Vec<u8> {
        let mut buf = sample_v4_story();
        buf[0x40] = 0xB5; // 0OP:0x05 save (store form, v4+)
        buf[0x41] = 0x10; // store -> global 0
        buf[0x42] = 0xB6; // 0OP:0x06 restore (store form, v4+)
        buf[0x43] = 0x11; // store -> global 1
        buf[0x44] = 0xBA; // quit
        buf
    }

    fn scratch_dir(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NTH: AtomicUsize = AtomicUsize::new(0);
        let nth = NTH.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("zvmcli-{tag}-{}-{nth}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// SQ-0635: an empty filename — a bare Enter at the prompt, or EOF — is a
    /// cancel. It used to fall through to `resolve_save_input("")`, which wrote
    /// a hidden `<game_dir>/.qzl` and told the game the save SUCCEEDED.
    #[test]
    fn an_empty_save_filename_cancels_instead_of_writing_a_hidden_file() {
        let dir = scratch_dir("save-cancel");
        let mem = Memory::new(save_then_restore_story()).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x40;
        assert_eq!(m.step(), StepResult::SaveRequest);

        handle_save_request(&mut m, &dir, "");

        assert_eq!(m.global(0), 0, "the game is told the save FAILED (cancelled)");
        assert!(!dir.join(".qzl").exists(), "no hidden .qzl is written");
        let leftovers: Vec<_> = fs::read_dir(&dir).unwrap().flatten().collect();
        assert!(leftovers.is_empty(), "nothing at all is written: {leftovers:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    /// The restore arm draws the same line — and the discriminating case is a
    /// `<game_dir>/.qzl` that happens to exist: a bare Enter must NOT silently
    /// restore from it (the pre-fix behaviour), it must cancel.
    #[test]
    fn an_empty_restore_filename_cancels_even_when_a_hidden_file_exists() {
        let dir = scratch_dir("restore-cancel");
        let mem = Memory::new(save_then_restore_story()).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x40;

        // Make a genuine save for this story and plant it where the empty
        // filename used to resolve.
        assert_eq!(m.step(), StepResult::SaveRequest);
        let blob = m.save_quetzal();
        m.complete_save(true);
        assert_eq!(m.global(0), 1, "save success stored 1");
        fs::write(dir.join(".qzl"), &blob).unwrap();

        assert_eq!(m.step(), StepResult::RestoreRequest);
        handle_restore_request(&mut m, &dir, "");

        // Restored, G0 would read 2 (the @save descriptor completed forward)
        // and execution would be back at the restore; cancelled, G0 keeps its
        // post-save 1 and the @restore stores 0 (failure).
        assert_eq!(m.global(0), 1, "the planted .qzl was NOT restored");
        assert_eq!(m.global(1), 0, "the game is told the restore failed");
        assert_eq!(m.step(), StepResult::Quit, "play continues past the cancelled restore");
        let _ = fs::remove_dir_all(&dir);
    }
}
