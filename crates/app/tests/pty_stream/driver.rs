//! Run the real `lanthorn` binary under a pty, answer the terminal queries it
//! asks, feed it keystrokes, and keep every byte it writes back (SQ-0762).
//!
//! Unix only — a pty is. The decoder in [`super::decode`] is portable, and the
//! callers gate this half with `#[cfg(unix)]`.
//!
//! WHY WE MUST PRETEND TO BE KITTY, PROPERLY. lanthorn picks its graphics backend
//! from `Picker::from_query_stdio`, which asks the terminal three questions before
//! the UI starts and falls back to half-blocks when nobody answers. A pty answers
//! nothing by itself, so a naive harness silently measures the half-block path —
//! the wrong backend, and every result it produces is worthless. [`Responder`]
//! answers those queries the way kitty does, and [`Capture::negotiated`] reports
//! what actually happened so a caller can refuse to go on if it is not kitty.

use std::ffi::CStr;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::os::unix::process::CommandExt as _;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// One scripted input step.
#[derive(Clone, Debug)]
pub enum Key {
    /// Literal bytes, exactly as a terminal would deliver them.
    Bytes(Vec<u8>),
    /// Let the app settle (or animate) for a while before the next key.
    Wait(Duration),
    /// Hold the next key until the app has ASKED the terminal a named query —
    /// a PHASE of the run rather than a reading off a stopwatch (SQ-1162).
    ///
    /// `Wait` is already silence-relative rather than a bare sleep, so it
    /// stretches when the app is busy. What it cannot do is tell one LULL from
    /// another: "320 ms of quiet after Enter" means "somewhere inside the boot"
    /// on a fast machine and can mean "before the boot probe has even started"
    /// on a loaded one, and a case that means to type INSIDE the window where
    /// the app owns the terminal then types outside it and fails on a property
    /// it never actually exercised. The [`Responder`] sees every query the app
    /// asks, so the phase is observable: wait for it by name and the staging
    /// stops being a race, which is the same bargain [`Spec::defer_queries`]
    /// struck for the answers.
    ///
    /// `cap` is a ceiling, not a target — reaching it fails the run with a
    /// message naming the query, because a query that never came means the
    /// scenario could not be staged and every assertion after it is vacuous.
    AwaitQuery { query: &'static str, cap: Duration },
    /// Resize the pty — cells AND the pixel geometry a cell size is derived from
    /// — with `TIOCSWINSZ` on the master (SQ-0993).
    ///
    /// **This is the only input that changes the CELL SIZE mid-run,** and without
    /// it SQ-0992's property is not observable end to end: the harness sized the
    /// terminal once at [`open_pty`] and never again, so "a font-size change keeps
    /// the capability and therefore keeps compressing" could only be argued from
    /// the source. The kernel raises `SIGWINCH` on the foreground process group
    /// for free, so nothing has to be typed at the app — this is a real resize,
    /// arriving the way a real one does.
    ///
    /// Changing `cell_w`/`cell_h` while holding `cols`/`rows` is a FONT-SIZE
    /// change: same grid, different pixels per cell, which is precisely what
    /// `refresh_cell_size` re-derives from.
    Resize { cols: u16, rows: u16, cell_w: u16, cell_h: u16 },
}

/// An `OSC <n>;rgb:rrrr/gggg/bbbb` reply, in the doubled-hex form terminals use.
fn osc_colour_reply(osc: u8, [r, g, b]: [u8; 3]) -> String {
    format!("\x1b]{osc};rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}\x07")
}

impl Key {
    /// Parse the ad-hoc key spelling used by the example's `--keys`:
    /// `cr esc tab space bs up down left right home end pgup pgdn f1..f4`,
    /// `wait:MS`, `ctrl+X`, and `text:...` for a literal string.
    pub fn parse(token: &str) -> Result<Key, String> {
        let t = token.trim();
        if let Some(ms) = t.strip_prefix("wait:") {
            let ms: u64 = ms.parse().map_err(|_| format!("bad wait `{t}`"))?;
            return Ok(Key::Wait(Duration::from_millis(ms)));
        }
        if let Some(s) = t.strip_prefix("text:") {
            return Ok(Key::Bytes(s.as_bytes().to_vec()));
        }
        // `resize:COLSxROWS@CELLWxCELLH` — the whole winsize, because a pty's
        // cell size IS its pixel geometry over its grid and naming half of it
        // would leave the other half to be guessed.
        if let Some(spec) = t.strip_prefix("resize:") {
            let (grid, cell) = spec.split_once('@').ok_or_else(|| format!("bad resize `{t}` (want COLSxROWS@CWxCH)"))?;
            let (c, r) = grid.split_once('x').ok_or_else(|| format!("bad resize grid in `{t}`"))?;
            let (cw, ch) = cell.split_once('x').ok_or_else(|| format!("bad resize cell in `{t}`"))?;
            let n = |v: &str| v.parse::<u16>().map_err(|_| format!("bad number in resize `{t}`"));
            return Ok(Key::Resize { cols: n(c)?, rows: n(r)?, cell_w: n(cw)?, cell_h: n(ch)? });
        }
        if let Some(c) = t.strip_prefix("ctrl+") {
            let ch = c.chars().next().ok_or_else(|| format!("bad ctrl key `{t}`"))?;
            let b = (ch.to_ascii_uppercase() as u8).wrapping_sub(0x40);
            return Ok(Key::Bytes(vec![b]));
        }
        let bytes: &[u8] = match t.to_ascii_lowercase().as_str() {
            "cr" | "enter" | "return" => b"\r",
            "lf" => b"\n",
            "esc" => b"\x1b",
            "tab" => b"\t",
            "space" | "sp" => b" ",
            "bs" => b"\x7f",
            "up" => b"\x1b[A",
            "down" => b"\x1b[B",
            "right" => b"\x1b[C",
            "left" => b"\x1b[D",
            "home" => b"\x1b[H",
            "end" => b"\x1b[F",
            "pgup" => b"\x1b[5~",
            "pgdn" => b"\x1b[6~",
            "f1" => b"\x1bOP",
            "f2" => b"\x1bOQ",
            "f3" => b"\x1bOR",
            "f4" => b"\x1bOS",
            other => {
                if other.chars().count() == 1 {
                    return Ok(Key::Bytes(t.as_bytes().to_vec()));
                }
                return Err(format!("unknown key `{t}`"));
            }
        };
        Ok(Key::Bytes(bytes.to_vec()))
    }
}

/// What to run and how to drive it.
#[derive(Clone, Debug)]
pub struct Spec {
    pub bin: PathBuf,
    pub story: PathBuf,
    /// A throwaway `--user-dir`, so the run never reads or writes the player's
    /// real `~/.lanthorn` (and never resumes their save into the capture).
    pub user_dir: PathBuf,
    pub cols: u16,
    pub rows: u16,
    /// The cell size we report for `CSI 16 t`. Not cosmetic: v6 art is scaled by
    /// pixel and placed by cell, so the whole geometry hangs off this.
    pub cell_w: u16,
    pub cell_h: u16,
    /// Start with the map pane hidden (writes the per-game `show_map = false`
    /// sidecar), which is what gives the story pane the full frame width.
    pub hide_map: bool,
    pub keys: Vec<Key>,
    /// Silence that counts as "the app has finished drawing".
    pub quiet: Duration,
    /// Wait for this much silence after the last key before giving up.
    pub tail: Duration,
    /// Hard ceiling on the whole run.
    pub timeout: Duration,
    pub extra_args: Vec<String>,
    /// Queries (by [`Responder`] name) to answer LATE rather than at once, by
    /// [`Spec::defer_by`]. A real terminal answers when it gets round to it —
    /// after a screenful of graphics, after an alternate-screen switch — and an
    /// app that stops listening before the answer arrives leaves those bytes in
    /// the tty for whoever reads next. Deferring turns that race into a fact
    /// (SQ-0769).
    ///
    /// The lateness applies to the whole BATCH: once a deferred query is seen in
    /// a burst, every reply from that one onwards is held back together. A busy
    /// terminal goes quiet for all of a probe's questions, not for one of them —
    /// and a probe that ends in a DSR only stops listening when the DSR answer
    /// arrives, so answering that on time while holding the rest back would be a
    /// terminal no one has.
    pub defer_queries: Vec<&'static str>,
    /// How late [`Spec::defer_queries`] are answered.
    pub defer_by: Duration,
    /// Answer the kitty capability query (default `true`).
    ///
    /// Turning it off is a DIFFERENT TERMINAL, not a broken one: `ratatui-image`
    /// treats half-blocks as its universal fallback, so lanthorn still runs the
    /// whole v6 pixel path and simply resolves it into `▀` cells. That is a real
    /// thing lanthorn does on a real terminal without graphics — and it is the
    /// only Version 6 output an asciinema cast can carry, because the player
    /// renders cells and SGR and drops kitty's APC escapes on the floor
    /// (SQ-0943).
    ///
    /// Every measuring caller wants this left alone. [`Negotiation`] still
    /// reports what happened, so a capture that turned it off cannot be mistaken
    /// for one that tried for kitty and lost.
    pub answer_kitty: bool,
    /// Run this exact argument list instead of lanthorn's own
    /// `<story> --user-dir … --sound off`.
    ///
    /// For the CLI clients (`zvm-cli`, `gvm-cli`, `scott-cli`), which take
    /// neither a `--user-dir` nor a map pane, and one of which — `zvm-cli
    /// --machines` — takes no story at all. When this is set, [`Spec::story`] is
    /// only a label and [`Spec::hide_map`] is not acted on, because there is no
    /// per-game sidecar to write for a program that has no per-game anything.
    pub argv: Option<Vec<String>>,
    /// End the run the way **ttyd ends a session whose websocket dropped** —
    /// `kill(-pid, SIGHUP)` at the child's process group — instead of the
    /// unconditional `SIGKILL` every other harness closes with (SQ-1323).
    ///
    /// This is the only shutdown lanthorn's own exit path can survive: SIGKILL
    /// is uncatchable, so a harness that ends that way can never observe the
    /// auto-save, the terminal restore, or the `128 + signum` exit code — and
    /// the browser-in-a-container question ("does a dropped connection lose the
    /// game?") is exactly a question about that path.
    ///
    /// ttyd 1.7.7 `src/protocol.c:373-379` (`LWS_CALLBACK_CLOSED`) calls
    /// `pty_kill(pss->process, server->sig_code)`, and `pty_kill` is
    /// `uv_kill(-process->pid, sig)` (`src/pty.c:158-164`) — a PROCESS-GROUP
    /// signal — with `sig_code` defaulting to `SIGHUP` (`src/server.c:169`).
    /// The child here is a session/group leader (`spawn`'s `setsid`), so the
    /// negative pid reaches it the same way.
    ///
    /// One deliberate difference: ttyd also `pty_pause`s (stops reading the
    /// master) before it signals, and this keeps draining. The bytes an exiting
    /// lanthorn writes are a few hundred — far inside any pty buffer — so the
    /// pause changes nothing about whether it can finish, and draining keeps
    /// the terminal-restore sequence in the capture where a case can assert on
    /// it.
    ///
    /// [`Capture::exit`] then carries how the child actually ended.
    pub hangup: bool,
    /// How long [`Self::hangup`] waits for the child to exit on its own before
    /// falling back to `SIGKILL`. Generous next to the app's own watchdog (600 ms
    /// grace, 10 s hard cap in `main.rs`), because the point is to let the exit
    /// save finish, not to time it.
    pub hangup_grace: Duration,
    /// Run the child from this directory instead of inheriting ours.
    ///
    /// For a launch where the PATH IS PART OF THE PICTURE (SQ-1080): the story
    /// picker prints the directory it scanned exactly as it was given, so a
    /// gallery frame of a library staged under the system temp dir wears
    /// `/var/folders/n8/p3vsw3jn6_77wv_m2zphnxww0000gn/T/…` across its header and
    /// clips the key hints off the end. Handing the child that directory's PARENT
    /// and naming the directory itself is the same launch a person makes by
    /// typing `lanthorn stories`, and it is the one the header describes.
    pub cwd: Option<PathBuf>,
}

impl Spec {
    pub fn new(bin: impl Into<PathBuf>, story: impl Into<PathBuf>, user_dir: impl Into<PathBuf>) -> Spec {
        Spec {
            bin: bin.into(),
            story: story.into(),
            user_dir: user_dir.into(),
            cols: 117,
            rows: 64,
            cell_w: 8,
            cell_h: 18,
            hide_map: true,
            keys: Vec::new(),
            quiet: Duration::from_millis(120),
            tail: Duration::from_millis(900),
            timeout: Duration::from_secs(45),
            extra_args: Vec::new(),
            defer_queries: Vec::new(),
            defer_by: Duration::ZERO,
            answer_kitty: true,
            argv: None,
            hangup: false,
            hangup_grace: Duration::from_secs(15),
            cwd: None,
        }
    }
}

/// One burst of output: bytes that arrived without a [`Spec::quiet`] gap between
/// them. lanthorn writes a frame in one go, so a burst is a frame in practice —
/// and grouping by silence needs no cooperation from the app.
///
/// The gap is measured between READS, not between "the last thing that happened".
/// Sending a keystroke used to reset the same clock the grouping read, so the
/// app's reply — which arrives a few milliseconds after the key — always looked
/// like a continuation of whatever burst came before it, however many seconds
/// earlier that was. Every run therefore collapsed into ONE flush at `at: 0`.
/// Invisible to the decoder, which only wants the grouping for attribution, and
/// fatal to the cast recorder (SQ-0943), for which these timestamps ARE the
/// recording.
#[derive(Clone, Debug)]
pub struct Flush {
    pub at: Duration,
    pub offset: usize,
    pub len: usize,
}

/// One mid-run pty resize, and the point in the stream it split (SQ-0993).
#[derive(Clone, Copy, Debug)]
pub struct Resized {
    pub at: Duration,
    /// Bytes captured before the resize was issued — so a transmit at or past
    /// this offset is one the app emitted knowing about the new cell size.
    pub offset: usize,
    pub cols: u16,
    pub rows: u16,
    pub cell_w: u16,
    pub cell_h: u16,
}

/// Which terminal query was asked, and what we answered.
#[derive(Clone, Debug)]
pub struct Answered {
    pub query: &'static str,
    pub sent: String,
    pub at: Duration,
}

/// Everything the run produced.
pub struct Capture {
    /// Every byte the app emitted, in order — the WIRE stream, which is what
    /// [`Flush`]'s offsets index and what any bandwidth reading must be taken
    /// from. Since SQ-0976 that is not the same thing as the stream a terminal
    /// interprets: see [`Capture::terminal_bytes`].
    pub bytes: Vec<u8>,
    pub flushes: Vec<Flush>,
    pub answered: Vec<Answered>,
    /// Where each [`Key::Resize`] landed: the length of [`Self::bytes`] at the
    /// moment the `TIOCSWINSZ` was issued, with the new winsize (SQ-0993).
    ///
    /// Without this, "the transmits AFTER the font change are still compressed"
    /// cannot be stated — every transmit in the capture looks alike, and a run
    /// that stopped compressing halfway would still pass an "all of them are
    /// compressed" check as long as the tail were empty.
    pub resizes: Vec<Resized>,
    /// Every [`Key::Bytes`] the run actually typed, and WHEN (SQ-1162).
    ///
    /// The script says what to type; only this says when it went out, and a
    /// case whose whole point is that a key landed during some window of the
    /// app's own making cannot state that from the script alone. Without it the
    /// interesting failure — the key drifting outside the window on a loaded
    /// machine, so the property is never exercised and the case passes anyway —
    /// is indistinguishable from the property holding.
    pub typed: Vec<Typed>,
    pub spec: Spec,
    pub duration: Duration,
    pub timed_out: bool,
    /// How the child ended, when [`Spec::hangup`] asked for a survivable
    /// shutdown. `None` for every other run — those close with `SIGKILL`, which
    /// makes the status a statement about the harness rather than the app.
    pub exit: Option<std::process::ExitStatus>,
}

/// One scripted keystroke, and the moment it was written to the pty.
#[derive(Clone, Debug)]
pub struct Typed {
    pub bytes: Vec<u8>,
    pub at: Duration,
}

impl Capture {
    /// The graphics protocol actually in force, decided from the stream rather
    /// than from what we hoped: kitty is proven by APC `_G` traffic, and nothing
    /// else proves it.
    pub fn negotiated(&self) -> Negotiation {
        let answered_kitty = self.answered.iter().any(|a| a.query == "kitty graphics support");
        let apc = count_subslices(&self.bytes, b"\x1b_G");
        Negotiation { answered_kitty, apc_commands: apc }
    }

    /// The same stream with the kitty protocol's `o=z` undone — what to hand
    /// [`super::oracle::resolve`], and the only form it can read (SQ-0976).
    ///
    /// The oracle's terminal core links no zlib, so a compressed transmit is
    /// dropped outright and every placement naming it vanishes. Undoing the
    /// compression is a transport rewrite that changes no image and no geometry;
    /// see [`super::inflate`] for why it belongs here and not in the emitter.
    ///
    /// [`Self::bytes`] is deliberately NOT this: `flushes` index it, and the wire
    /// size is the measurement SQ-0976 exists to move.
    pub fn terminal_bytes(&self) -> std::borrow::Cow<'_, [u8]> {
        super::inflate::kitty_inflate(&self.bytes)
    }
}

/// The protocol verdict. `is_kitty()` is the gate every caller must pass before
/// believing anything else in the capture.
#[derive(Clone, Copy, Debug)]
pub struct Negotiation {
    pub answered_kitty: bool,
    pub apc_commands: usize,
}

impl Negotiation {
    pub fn is_kitty(&self) -> bool {
        self.answered_kitty && self.apc_commands > 0
    }

    pub fn explain(&self) -> String {
        match (self.answered_kitty, self.apc_commands) {
            (false, _) => "NOT KITTY — lanthorn never asked the kitty capability query (or asked it in \
                 a spelling this harness does not answer), so it detected half-blocks"
                .to_string(),
            (true, 0) => "NOT KITTY — the harness answered the capability query, but lanthorn emitted \
                 no APC graphics at all: it fell back to a cell renderer, so this capture measures \
                 the wrong backend"
                .to_string(),
            (true, n) => format!(
                "KITTY — the harness answered the kitty capability query and lanthorn emitted \
                 {n} APC `_G` graphics command(s)"
            ),
        }
    }
}

// ── The pty ───────────────────────────────────────────────────────────────────

struct Pty {
    master: OwnedFd,
    slave: OwnedFd,
}

fn errno() -> std::io::Error {
    std::io::Error::last_os_error()
}

fn open_pty(spec: &Spec) -> std::io::Result<Pty> {
    // SAFETY: plain POSIX pty setup; every returned fd is checked and wrapped in
    // an OwnedFd immediately, and `ptsname`'s buffer is copied before any other
    // libc call can reuse it.
    unsafe {
        let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
        if master < 0 {
            return Err(errno());
        }
        let master = OwnedFd::from_raw_fd(master);
        if libc::grantpt(master.as_raw_fd()) != 0 || libc::unlockpt(master.as_raw_fd()) != 0 {
            return Err(errno());
        }
        let name = libc::ptsname(master.as_raw_fd());
        if name.is_null() {
            return Err(errno());
        }
        let name = CStr::from_ptr(name).to_owned();
        let slave = libc::open(name.as_ptr(), libc::O_RDWR | libc::O_NOCTTY);
        if slave < 0 {
            return Err(errno());
        }
        let slave = OwnedFd::from_raw_fd(slave);

        // Size the terminal, in cells AND in pixels: a v6 title asks the host how
        // big the screen is, and a pixel size of zero is not a size a real
        // terminal ever reports.
        set_winsize(master.as_raw_fd(), spec.cols, spec.rows, spec.cell_w, spec.cell_h)?;

        // Local echo off before the child starts: the app turns raw mode on a few
        // milliseconds in, and anything we type before then would otherwise come
        // straight back and pollute the capture with our own keystrokes.
        let mut tio: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(slave.as_raw_fd(), &mut tio) == 0 {
            tio.c_lflag &= !(libc::ECHO | libc::ECHONL);
            let _ = libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &tio);
        }
        Ok(Pty { master, slave })
    }
}

/// `TIOCSWINSZ` on a pty master, in cells and in pixels.
///
/// Shared by [`open_pty`] and [`Key::Resize`] so the initial size and every later
/// one are set the same way — and so a resize cannot forget the PIXEL half, which
/// is the half a cell size is derived from and the whole point of SQ-0993's
/// mid-run resize.
fn set_winsize(fd: RawFd, cols: u16, rows: u16, cell_w: u16, cell_h: u16) -> std::io::Result<()> {
    let ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: cols.saturating_mul(cell_w),
        ws_ypixel: rows.saturating_mul(cell_h),
    };
    // SAFETY: one initialised winsize on a pty master fd the caller owns.
    if unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &ws) } != 0 {
        return Err(errno());
    }
    Ok(())
}

fn spawn(spec: &Spec, pty: &Pty) -> std::io::Result<Child> {
    let slave_fd: RawFd = pty.slave.as_raw_fd();
    let mut cmd = Command::new(&spec.bin);
    match &spec.argv {
        // A CLI client's whole command line, verbatim: it has no `--user-dir`
        // and no map pane to hide, so nothing of lanthorn's shape applies.
        Some(argv) => {
            cmd.args(argv);
        }
        None => {
            cmd.arg(&spec.story)
                .arg("--user-dir")
                .arg(&spec.user_dir)
                .arg("--sound")
                .arg("off")
                .args(&spec.extra_args);
        }
    }
    if let Some(dir) = &spec.cwd {
        cmd.current_dir(dir);
    }
    cmd.stdin(Stdio::from(pty.slave.try_clone()?))
        .stdout(Stdio::from(pty.slave.try_clone()?))
        .stderr(Stdio::from(pty.slave.try_clone()?));
    // A kitty-shaped environment, with every "you are actually something else"
    // hint cleared: ratatui-image trusts the IO probe first, but TMUX/WezTerm/
    // Konsole hints would blacklist the very protocol we are here to exercise.
    cmd.env("TERM", "xterm-kitty");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("KITTY_WINDOW_ID", "1");
    for stale in ["TMUX", "TMUX_PANE", "WEZTERM_EXECUTABLE", "KONSOLE_VERSION", "TERM_PROGRAM", "LC_TERMINAL"] {
        cmd.env_remove(stale);
    }
    unsafe {
        cmd.pre_exec(move || {
            // Own a session and make the pty its controlling terminal, or
            // crossterm's /dev/tty open (and therefore raw mode) fails.
            if libc::setsid() < 0 {
                return Err(errno());
            }
            if libc::ioctl(slave_fd, libc::TIOCSCTTY as _, 0) < 0 {
                return Err(errno());
            }
            Ok(())
        });
    }
    cmd.spawn()
}

// ── Answering the terminal queries ────────────────────────────────────────────

/// The queries we answer, in the exact spelling ratatui-image and lanthorn send.
/// A query we do NOT answer is not a silent loss: the app's probes all end in a
/// DSR, so an unanswered one costs a timeout and a wrong backend — which is what
/// [`Negotiation`] exists to catch.
struct Responder {
    /// Whether to answer the kitty capability query at all — see
    /// [`Spec::answer_kitty`].
    answer_kitty: bool,
    cell_w: u16,
    cell_h: u16,
    cols: u16,
    rows: u16,
    scanned: usize,
    /// Byte offsets already answered, so the overlapping re-scan below never
    /// types a reply twice — a duplicate reply arrives as phantom keystrokes.
    answered_at: std::collections::HashSet<usize>,
}

impl Responder {
    fn replies(&self) -> Vec<(&'static str, &'static [u8], String)> {
        let (w, h) = (self.cell_w, self.cell_h);
        let mut v = vec![
            // Kitty's own DA1: no `4`, because kitty does not do sixel. Claiming
            // sixel here would let a fallback path look like a success.
            ("primary device attributes", b"\x1b[c".as_slice(), "\x1b[?62;c".to_string()),
            ("cell size in pixels", b"\x1b[16t".as_slice(), format!("\x1b[6;{h};{w}t")),
            (
                "window size in pixels",
                b"\x1b[14t".as_slice(),
                format!("\x1b[4;{};{}t", self.rows * h, self.cols * w),
            ),
            ("window size in cells", b"\x1b[18t".as_slice(), format!("\x1b[8;{};{}t", self.rows, self.cols)),
            (
                "default foreground (OSC 10)",
                b"\x1b]10;?\x07".as_slice(),
                osc_colour_reply(10, super::ANSWERED_FG),
            ),
            (
                "default background (OSC 11)",
                b"\x1b]11;?\x07".as_slice(),
                osc_colour_reply(11, super::ANSWERED_BG),
            ),
            ("cursor position report", b"\x1b[6n".as_slice(), "\x1b[1;1R".to_string()),
            ("keyboard protocol flags", b"\x1b[?u".as_slice(), "\x1b[?0u".to_string()),
            // Last, because every probe ends with it and the reply is the app's
            // signal that the whole batch is answered.
            ("device status report", b"\x1b[5n".as_slice(), "\x1b[0n".to_string()),
        ];
        if self.answer_kitty {
            // First, because it is the first thing the app asks and a burst's
            // replies go back in the order its questions arrived.
            v.insert(
                0,
                (
                    "kitty graphics support",
                    b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\".as_slice(),
                    "\x1b_Gi=31;OK\x1b\\".to_string(),
                ),
            );
            // SQ-0991: the same one-pixel query with the payload deflated, which
            // is how `ratatui-image` asks whether this terminal can inflate an
            // `o=z` transmission. Answer it the way kitty does, or every capture
            // measures the UNCOMPRESSED wire — the backend would still be kitty
            // and nothing would look wrong, which is the expensive kind of wrong.
            // Matched on the parameters only: the payload is a zlib stream whose
            // exact bytes are the encoder's business, not the protocol's.
            v.insert(
                1,
                (
                    "kitty transmission compression",
                    b"\x1b_Gi=32,s=1,v=1,a=q,t=d,f=24,o=z;".as_slice(),
                    "\x1b_Gi=32;OK\x1b\\".to_string(),
                ),
            );
        }
        v
    }

    /// Scan everything captured so far for queries we have not yet answered, and
    /// return the replies to type back, in the order the app asked.
    ///
    /// The scan restarts a little BEHIND the last one and remembers which offsets
    /// it has already answered: a query is a handful of bytes and a read can land
    /// in the middle of one, and a query missed because it straddled a 64 KiB
    /// boundary is a wrong backend measured in silence.
    fn scan(&mut self, all: &[u8]) -> Vec<(&'static str, String)> {
        const OVERLAP: usize = 64;
        let from = self.scanned.saturating_sub(OVERLAP);
        let mut hits: Vec<(usize, &'static str, String)> = Vec::new();
        for (name, pat, reply) in self.replies() {
            let mut i = from;
            while let Some(off) = find_subslice(&all[i..], pat) {
                let at = i + off;
                if self.answered_at.insert(at) {
                    hits.push((at, name, reply.clone()));
                }
                i = at + pat.len();
            }
        }
        self.scanned = all.len();
        hits.sort_by_key(|(off, _, _)| *off);
        hits.into_iter().map(|(_, n, r)| (n, r)).collect()
    }
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

fn count_subslices(hay: &[u8], needle: &[u8]) -> usize {
    let (mut i, mut n) = (0usize, 0usize);
    while let Some(off) = find_subslice(&hay[i..], needle) {
        n += 1;
        i += off + needle.len();
    }
    n
}

// ── The run ───────────────────────────────────────────────────────────────────

/// Boot lanthorn under a pty, drive `spec.keys`, and return every byte it wrote.
pub fn run(spec: Spec) -> std::io::Result<Capture> {
    std::fs::create_dir_all(&spec.user_dir)?;
    // An EMPTY config.toml, and every harness gets one (SQ-1104).
    //
    // "There is no config.toml" is lanthorn's definition of a first run, and a
    // first run raises the font check — a modal that waits for a keypress. That
    // is the normal state of a throwaway user-dir, so without this line every pty
    // harness would hang on a question about fonts before drawing a single frame,
    // and a tty check would not save them: these run under a REAL pty by design.
    //
    // Written here rather than in each harness so a future `Spec` inherits it
    // without its author needing to know any of this. An empty file is respected
    // rather than reseeded (`config_template::auto_seed` skips a file that
    // exists), so it costs nothing but the first-run flag — and a harness that
    // wants real keys in it (`pty_flank_alpha_seam`, `examples/cast`) has already
    // written its own by the time `run` is called, which is why this only fills
    // in an absent one.
    let cfg = spec.user_dir.join("config.toml");
    if !cfg.exists() {
        std::fs::write(&cfg, "")?;
    }
    if spec.hide_map && spec.argv.is_none() {
        // The story pane only gets the whole frame when the map is hidden, and
        // the per-game sidecar sets that BEFORE the first frame — toggling it with
        // a keystroke would put the transition in the capture we are measuring.
        let game_dir = app::storage::game_dir(
            &spec.user_dir.join("saves"),
            &app::storage::story_key_at(&spec.story),
        );
        std::fs::create_dir_all(&game_dir)?;
        std::fs::write(game_dir.join("config.toml"), "show_map = false\n")?;
    }

    let pty = open_pty(&spec)?;
    let mut child = spawn(&spec, &pty)?;
    // The parent must not hold the slave open: with it open, reads on the master
    // block for ever instead of ending when the app exits.
    drop(pty.slave);
    let master = pty.master;

    let start = Instant::now();
    let mut bytes: Vec<u8> = Vec::with_capacity(1 << 20);
    let mut flushes: Vec<Flush> = Vec::new();
    let mut answered: Vec<Answered> = Vec::new();
    let mut resizes: Vec<Resized> = Vec::new();
    let mut typed: Vec<Typed> = Vec::new();
    let mut responder =
        Responder {
            answer_kitty: spec.answer_kitty,
            cell_w: spec.cell_w,
            cell_h: spec.cell_h,
            cols: spec.cols,
            rows: spec.rows,
            scanned: 0,
            answered_at: std::collections::HashSet::new(),
        };
    let mut keys = spec.keys.clone().into_iter();
    let mut pending_wait: Option<Duration> = None;
    // Every query the app has asked so far, by `Responder` name — what
    // `Key::AwaitQuery` waits on. Recorded when the query is SEEN, not when it
    // is answered, because a deferred batch is deliberately not answered for a
    // while and the phase begins at the asking.
    let mut seen_queries: std::collections::HashSet<&'static str> = std::collections::HashSet::new();
    let mut awaiting: Option<(&'static str, Instant, Duration)> = None;
    let mut last_byte = Instant::now();
    // Read timing, kept apart from the key pacing above: `last_byte` moves when
    // we TYPE, which is what "has the app gone quiet enough for the next key"
    // wants and what burst grouping must not see. See [`Flush`].
    let mut last_read = Instant::now();
    let mut file = unsafe { std::fs::File::from_raw_fd(libc::dup(master.as_raw_fd())) };
    let mut timed_out = false;
    // Replies held back by `spec.defer_queries`, each with the instant it is due.
    let mut deferred: Vec<(Instant, &'static str, String)> = Vec::new();

    loop {
        if start.elapsed() > spec.timeout {
            timed_out = true;
            break;
        }
        // Late answers first, so a deferred reply lands on time even while the app
        // is quiet and the loop is only ticking the poll timeout.
        let now = Instant::now();
        let mut still_pending = Vec::new();
        for (due, name, reply) in deferred.drain(..) {
            if due <= now {
                let _ = write_all(master.as_raw_fd(), reply.as_bytes());
                answered.push(Answered { query: name, sent: reply, at: start.elapsed() });
            } else {
                still_pending.push((due, name, reply));
            }
        }
        deferred = still_pending;
        match poll_readable(master.as_raw_fd(), Duration::from_millis(20)) {
            Ok(true) => {
                let mut chunk = [0u8; 65536];
                match file.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        let now = Instant::now();
                        let gap = now.duration_since(last_read);
                        last_read = now;
                        last_byte = now;
                        let offset = bytes.len();
                        match flushes.last_mut() {
                            Some(f) if gap < spec.quiet => f.len += n,
                            _ => flushes.push(Flush { at: start.elapsed(), offset, len: n }),
                        }
                        bytes.extend_from_slice(&chunk[..n]);
                        let mut batch_late = false;
                        for (name, reply) in responder.scan(&bytes) {
                            seen_queries.insert(name);
                            batch_late |= spec.defer_queries.contains(&name);
                            if batch_late {
                                deferred.push((Instant::now() + spec.defer_by, name, reply));
                                continue;
                            }
                            let _ = write_all(master.as_raw_fd(), reply.as_bytes());
                            answered.push(Answered { query: name, sent: reply, at: start.elapsed() });
                        }
                    }
                    // EIO on Linux is how a pty master reports "the child is gone".
                    Err(e) if e.raw_os_error() == Some(libc::EIO) => break,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(e),
                }
            }
            Ok(false) => {
                let quiet_for = last_byte.elapsed();
                // Phase before stopwatch: a key held for a query goes nowhere
                // until the app has asked it, however busy or idle the run is.
                if let Some((q, since, cap)) = awaiting {
                    if seen_queries.contains(q) {
                        awaiting = None;
                    } else if since.elapsed() >= cap {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            format!("the app never asked `{q}`, so the scenario was never staged"),
                        ));
                    } else {
                        continue;
                    }
                }
                if let Some(w) = pending_wait {
                    if quiet_for >= w {
                        pending_wait = None;
                    }
                    continue;
                }
                if quiet_for < spec.quiet {
                    continue;
                }
                match keys.next() {
                    Some(Key::Bytes(b)) => {
                        write_all(master.as_raw_fd(), &b)?;
                        typed.push(Typed { bytes: b, at: start.elapsed() });
                        last_byte = Instant::now();
                    }
                    Some(Key::Wait(d)) => pending_wait = Some(d),
                    Some(Key::AwaitQuery { query, cap }) => {
                        awaiting = Some((query, Instant::now(), cap));
                    }
                    Some(Key::Resize { cols, rows, cell_w, cell_h }) => {
                        // The kernel signals the child for us — no keystroke, no
                        // escape, exactly as a real window resize arrives (SQ-0993).
                        set_winsize(master.as_raw_fd(), cols, rows, cell_w, cell_h)?;
                        resizes.push(Resized {
                            at: start.elapsed(),
                            offset: bytes.len(),
                            cols,
                            rows,
                            cell_w,
                            cell_h,
                        });
                        responder.cols = cols;
                        responder.rows = rows;
                        responder.cell_w = cell_w;
                        responder.cell_h = cell_h;
                        last_byte = Instant::now();
                    }
                    // Keys exhausted: give the app a longer silence to finish
                    // whatever the last one started, then stop — but never while a
                    // deferred reply is still owed, or the run would end before the
                    // very lateness it is measuring.
                    None if quiet_for >= spec.tail && deferred.is_empty() => break,
                    None => {}
                }
            }
            Err(e) => return Err(e),
        }
    }

    let exit = if spec.hangup {
        Some(hang_up(&mut child, master.as_raw_fd(), &mut bytes, spec.hangup_grace)?)
    } else {
        let _ = child.kill();
        let _ = child.wait();
        None
    };
    Ok(Capture { bytes, flushes, answered, resizes, typed, spec, duration: start.elapsed(), timed_out, exit })
}

/// Hang the session up the way ttyd does and wait for the app to end itself.
///
/// See [`Spec::hangup`] for the ttyd source this mirrors. Output keeps being
/// drained into `bytes` while we wait, so the terminal-restore sequence the app
/// writes on its way out is part of the capture; `SIGKILL` is the backstop if
/// `grace` runs out, and a run that needed it is a run in which the app did NOT
/// exit on its own — which the returned status then says.
fn hang_up(
    child: &mut Child,
    master: RawFd,
    bytes: &mut Vec<u8>,
    grace: Duration,
) -> std::io::Result<std::process::ExitStatus> {
    let pid = child.id() as libc::pid_t;
    // The NEGATIVE pid is the point: ttyd signals the process GROUP, and
    // `spawn`'s `setsid` made the child its leader.
    // SAFETY: a plain kill(2) on a pid we spawned.
    unsafe {
        libc::kill(-pid, libc::SIGHUP);
    }
    let deadline = Instant::now() + grace;
    let mut buf = [0u8; 8192];
    // A pty master whose child has gone reports EIO on Linux rather than EOF, and
    // a read can be cut short by the SIGCHLD we are waiting for — so neither a
    // poll nor a read failing here is an error of the RUN. The child's exit
    // status is the only thing this loop actually needs.
    let mut sip = |bytes: &mut Vec<u8>| {
        if matches!(poll_readable(master, Duration::from_millis(20)), Ok(true)) {
            // SAFETY: a read(2) into an owned buffer on a fd the caller holds open.
            let n = unsafe { libc::read(master, buf.as_mut_ptr().cast(), buf.len()) };
            if n > 0 {
                bytes.extend_from_slice(&buf[..n as usize]);
                return true;
            }
        }
        false
    };
    loop {
        if let Some(status) = child.try_wait()? {
            // Drain whatever is still in the pty after the app let go of it.
            while sip(bytes) {}
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            return child.wait();
        }
        sip(bytes);
    }
}

fn poll_readable(fd: RawFd, timeout: Duration) -> std::io::Result<bool> {
    let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
    // SAFETY: one initialised pollfd, count 1, as poll(2) requires.
    let n = unsafe { libc::poll(&mut pfd, 1, timeout.as_millis() as libc::c_int) };
    if n < 0 {
        let e = errno();
        if e.kind() == std::io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(e);
    }
    Ok(n > 0 && pfd.revents & (libc::POLLIN | libc::POLLHUP) != 0)
}

fn write_all(fd: RawFd, data: &[u8]) -> std::io::Result<()> {
    // SAFETY: `fd` stays owned by the caller — dup it so dropping this File does
    // not close the master out from under the run.
    let mut f = unsafe { std::fs::File::from_raw_fd(libc::dup(fd)) };
    f.write_all(data)?;
    f.flush()
}

/// Find `target/<profile>/lanthorn` from a binary that cargo built beside it —
/// the trick that lets `cargo run --example` reach the real app without the
/// `CARGO_BIN_EXE_*` that only integration tests get.
pub fn sibling_lanthorn() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("LANTHORN_BIN") {
        return Some(PathBuf::from(p));
    }
    sibling_binary("lanthorn")
}

/// Any workspace binary cargo built beside this one — `zvm-cli`, `gvm-cli`,
/// `scott-cli` (SQ-0943). Same trick as [`sibling_lanthorn`], which is now one
/// call of it.
pub fn sibling_binary(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join(name), dir.parent()?.join(name)].into_iter().find(|cand| cand.is_file())
}

/// The repo's `stories/` directory, from this crate's manifest.
pub fn stories_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}
