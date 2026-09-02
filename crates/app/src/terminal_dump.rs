//! `/dump-terminal` (SQ-0994): what lanthorn detected about this terminal, what
//! it is doing with it, and — for every number that is a guess — that it is one.
//!
//! **The organising principle is MEASURED versus ASSUMED.** Several values the
//! whole graphics path is computed from are guesses that look exactly like
//! measurements, and until this command there was no way to tell which a given
//! one was:
//!
//! * The cell size falls back to `ratatui-image`'s hardcoded 10x20 when
//!   `CSI 16 t` goes unanswered, and on Windows there is no `TIOCGWINSZ`
//!   backstop either. `cell 10x20` and `cell 10x20 (ASSUMED)` mean completely
//!   different things, and every device box downstream is derived from it — this
//!   is what bit SQ-0973, where a nominal 10x20 produced a 4580x2880 box.
//! * A capability list is empty both when the terminal said "no" and when nobody
//!   asked (`--images off`, `--image-protocol halfblocks`, a non-tty).
//! * Kitty transmission compression (`o=z`) fails silently in both directions: a
//!   terminal that cannot inflate simply draws nothing, and the capability
//!   quietly reverting to raw looks like nothing at all (SQ-0991/0992).
//!
//! So [`dump_lines`] marks a guessed or unreachable value with
//! [`DumpKind::Assumed`] and the render applies its own selector to those lines.
//!
//! **It is a pure function of [`TerminalSnapshot`].** The snapshot is gathered in
//! `slash_dispatch` — the only place that can reach the live `Picker`, the tty
//! ioctl and the render state — and everything downstream (the transcript, the
//! `dump-terminal.log` mirror, the tests) reads only these lines. That is what
//! lets the report be asserted without a terminal at all.
//!
//! **Nothing here adds per-frame cost.** [`Traffic`] counts at the WRITER
//! boundary — one `fetch_add` per `write` call and one per `flush`, both O(1) and
//! neither looking at a byte — and every other statistic is read from something
//! the render already tracked for its own reasons. Anything that would have meant
//! instrumenting the render path is reported as unavailable, with the reason.

use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

// ── Traffic: the counters, and the writer that feeds them ────────────────────

/// Bytes and flushes lanthorn has handed the terminal, counted at the writer.
///
/// Deliberately NOT a byte scan. Counting APC `_G` commands (or telling a
/// compressed transmit from a raw one) off the wire would mean searching every
/// buffer written, which is O(bytes) on the frame path; the graphics ops the
/// render already records answer the same questions for free, so those are what
/// the report uses. See [`TerminalSnapshot::apc_note`].
///
/// Atomics rather than a `Cell` because the counters live inside the ratatui
/// backend while the reader is `AppState`, and an `Arc` is the cheapest way for
/// both to hold the same object. There is no contention: one thread writes.
#[derive(Debug, Default)]
pub struct Traffic {
    /// Every byte written to the terminal since launch, through the ratatui
    /// backend. Escapes written directly with `execute!(stdout(), …)` — entering
    /// the alternate screen, the mouse-mode toggles — bypass this, which is why
    /// the report calls it the FRAME traffic rather than the process's output.
    bytes: AtomicU64,
    /// Flushes with something in them. ratatui flushes once at the end of
    /// `Terminal::draw`, so this counts drawn frames.
    flushes: AtomicU64,
    /// Bytes in the last flush that carried any — one frame's worth.
    last_flush: AtomicU64,
    /// Bytes written since the last flush: the frame being built right now.
    pending: AtomicU64,
}

impl Traffic {
    pub fn total_bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }
    pub fn flushes(&self) -> u64 {
        self.flushes.load(Ordering::Relaxed)
    }
    pub fn last_flush_bytes(&self) -> u64 {
        self.last_flush.load(Ordering::Relaxed)
    }
}

/// A shared handle on the counters: the backend writes through one clone, the
/// app reads another.
pub type TrafficHandle = Arc<Traffic>;

/// `W` with [`Traffic`] counted around it.
///
/// One `fetch_add` per `write` and a `swap` per `flush`. It never looks at the
/// bytes, so a 400 KB frame costs the same as a 40-byte one.
#[derive(Debug)]
pub struct CountingWriter<W> {
    inner: W,
    traffic: TrafficHandle,
}

impl<W: Write> CountingWriter<W> {
    pub fn new(inner: W, traffic: TrafficHandle) -> Self {
        CountingWriter { inner, traffic }
    }
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.traffic.bytes.fetch_add(n as u64, Ordering::Relaxed);
        self.traffic.pending.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        // An empty flush is not a frame. ratatui's backend flushes on paths that
        // wrote nothing (a draw whose diff was empty), and counting those would
        // make "bytes per frame" an average over frames that never happened.
        let n = self.traffic.pending.swap(0, Ordering::Relaxed);
        if n > 0 {
            self.traffic.last_flush.store(n, Ordering::Relaxed);
            self.traffic.flushes.fetch_add(1, Ordering::Relaxed);
        }
        self.inner.flush()
    }
}

// ── The snapshot ─────────────────────────────────────────────────────────────

/// Where the cell size in force actually came from.
///
/// The whole reason this enum exists: three of these four print the same
/// `10x20` and only one of them is a measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellSource {
    /// The terminal answered `CSI 16 t` and that answer is still the one in use.
    Measured,
    /// No `CSI 16 t` answer, but the tty's `TIOCGWINSZ` reports a pixel geometry
    /// and the cell in force is exactly `ws_xpixel/ws_col` by `ws_ypixel/ws_row`.
    /// This is also what a mid-session font change re-derives from (SQ-0988).
    Derived,
    /// Nobody answered either, so `ratatui-image`'s hardcoded 10x20 stands. It is
    /// documented in that crate as "completely arbitrary"; on Windows there is no
    /// ioctl to fall back to at all, so this is the ordinary case there.
    Assumed,
    /// A cell size is in force that neither source explains — a stale `CSI 16 t`
    /// answer after the window was rescaled, most likely. Reported rather than
    /// guessed at.
    Unexplained,
    /// There is no picker, so there is no cell size: `--images off`.
    None,
}

/// Whether the capability probe ran at all — an empty list means nothing until
/// this says which kind of empty it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// `Picker::from_query_stdio` ran: the terminal was asked.
    Asked,
    /// `--image-protocol halfblocks`: `Picker::halfblocks()` asks nothing.
    NotAskedHalfblocksForced,
    /// `--images off`: there is no picker.
    NotAskedImagesOff,
}

/// The v6 render facts that explain the traffic. `None` for a non-v6 session,
/// where none of them exist.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderFacts {
    /// `hybrid` or `raster`, from the live `v6_render` setting.
    pub mode: &'static str,
    /// Which `picture_takeover_reason` arm fired on the last frame that
    /// evaluated one, or `None` for "the ring drew it". `takeover_evaluated` is
    /// false in raster mode, where the hatch is never reached and this would be
    /// a stale answer.
    pub takeover: Option<&'static str>,
    pub takeover_evaluated: bool,
    /// The game's own screen in native pixels.
    pub native: (u16, u16),
    /// Per-axis density of the mounted artwork, which sets the pixel lock's rungs.
    pub art_scale: (u32, u32),
    /// The letterbox magnification the last v6 frame published.
    pub magnification: f32,
    pub pixel_lock: bool,
    /// The last frame wanted the lock and the pane was too small for the lowest
    /// rung, so it free-scaled anyway (SQ-0936).
    pub pixel_lock_fell_back: bool,
    /// The last frame wanted the lock on a backend with no rung to snap to, so it
    /// was inert (SQ-0978). Half-blocks: the ladder is quantized in device pixels
    /// and half-blocks resolves the picture into CELLS, one sample per column and
    /// two per row, so the device pixels the rung is counted in are a number
    /// `Picker::halfblocks`'s hardcoded 10x20 invented.
    ///
    /// Separate from [`Self::pixel_lock_fell_back`] because a reader told "the pane
    /// is too small" goes looking for a bigger terminal, and there is no pane size
    /// that would have honoured this one.
    pub pixel_lock_inapplicable: bool,
    /// Recent render paths, newest last, consecutive repeats collapsed.
    pub paths: Vec<String>,
}

/// Everything [`dump_lines`] renders. Gathered once, at command time.
#[derive(Debug, Clone, PartialEq)]
pub struct TerminalSnapshot {
    /// The protocol actually in force (`kitty`, `sixel`, `iterm2`, `halfblocks`),
    /// or `None` when images are off.
    pub protocol: Option<String>,
    /// `Some` when `--image-protocol` named one instead of letting detection run.
    pub forced_protocol: Option<String>,
    pub probe: Probe,
    /// The cell size the graphics geometry is computed from.
    pub cell: Option<(u16, u16)>,
    pub cell_source: CellSource,
    /// What `CSI 16 t` answered, if it did.
    pub reported_cell: Option<(u16, u16)>,
    /// What `TIOCGWINSZ` says right now, if it says anything.
    pub ioctl_cell: Option<(u16, u16)>,
    /// The detected capability list, already rendered to strings.
    pub capabilities: Vec<String>,
    /// The terminal answered the `o=z` probe.
    pub kitty_compression: bool,
    /// The story pane in terminal cells.
    pub pane_cells: (u16, u16),
    pub render: Option<RenderFacts>,
    /// Bytes / flushes, or `None` when the counters were never installed (every
    /// headless harness, which does not build a terminal at all).
    pub traffic: Option<TrafficStats>,
    /// Uploads the chrome-band and composite path has encoded since launch.
    pub band_encodes: u64,
    /// What every kitty upload since launch cost the wire, against what the same
    /// pixels would have cost uncompressed (SQ-1005).
    pub uploads: crate::render::graphics::UploadBytes,
    /// The last recorded frame's graphics ops.
    pub ops: OpCounts,
}

/// The writer counters, read out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrafficStats {
    pub total_bytes: u64,
    pub flushes: u64,
    pub last_flush_bytes: u64,
}

/// The last recorded frame's graphics traffic, as the render itself recorded it
/// (`GraphicsOp`, SQ-0590). This is where the APC command count comes from: the
/// ops ARE the commands, and reading them costs nothing, where counting `\x1b_G`
/// on the wire would mean scanning every byte of every frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpCounts {
    /// Pixels encoded and sent.
    pub uploads: usize,
    /// The terminal already had these pixels; an existing upload was re-placed.
    pub reuses: usize,
    pub places: usize,
    pub drops: usize,
    /// Cells the frame's placements cover — the kitty placeholder grid the images
    /// are drawn through, and since SQ-0977 the larger half of the traffic.
    pub placed_cells: u64,
}

impl TerminalSnapshot {
    /// Why the APC command count is the op log's and not the wire's.
    fn apc_note() -> &'static str {
        "counted from the render's own op log, not from the wire: finding \\x1b_G \
         in the stream would mean scanning every byte of every frame"
    }
}

// ── The report ───────────────────────────────────────────────────────────────

/// What a report line is, so the render can style it without parsing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DumpKind {
    /// A section heading.
    Heading,
    /// An ordinary reported value.
    Value,
    /// A value lanthorn GUESSED, or one it could not reach. The distinction this
    /// whole command exists to draw.
    Assumed,
}

/// One line of the report.
#[derive(Debug, Clone, PartialEq)]
pub struct DumpLine {
    pub text: String,
    pub kind: DumpKind,
}

fn heading(text: impl Into<String>) -> DumpLine {
    DumpLine { text: text.into(), kind: DumpKind::Heading }
}
fn value(text: impl Into<String>) -> DumpLine {
    DumpLine { text: text.into(), kind: DumpKind::Value }
}
fn assumed(text: impl Into<String>) -> DumpLine {
    DumpLine { text: text.into(), kind: DumpKind::Assumed }
}

/// Group digits so a byte count can be read at a glance.
fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The whole report, as a pure function of the snapshot.
pub fn dump_lines(s: &TerminalSnapshot) -> Vec<DumpLine> {
    let mut out = vec![heading("[dump-terminal]"), heading("terminal")];

    // ── protocol ─────────────────────────────────────────────────────────────
    match (&s.protocol, &s.forced_protocol) {
        (None, _) if s.probe == Probe::NotAskedImagesOff => out.push(assumed(
            "  graphics protocol: none — images are off (--images off); nothing below was detected",
        )),
        (None, _) => out.push(assumed(
            "  graphics protocol: none — no picker was built, so the capability query found \
             nothing this terminal supports",
        )),
        (Some(p), Some(f)) => {
            out.push(value(format!("  graphics protocol: {p} — FORCED by --image-protocol {f}, not detected")))
        }
        (Some(p), None) => out.push(value(format!("  graphics protocol: {p} (auto-detected)"))),
    }

    // ── cell size, and where it came from ────────────────────────────────────
    match (s.cell, s.cell_source) {
        (None, _) | (_, CellSource::None) => {
            out.push(assumed("  cell size: unknown — no picker, so no cell geometry"))
        }
        (Some((w, h)), src) => {
            let (label, why, guessed) = match src {
                CellSource::Measured => ("MEASURED", "the terminal answered CSI 16 t", false),
                CellSource::Derived => ("DERIVED", "from the tty's TIOCGWINSZ pixel geometry", false),
                CellSource::Assumed => (
                    "ASSUMED",
                    "ratatui-image's hardcoded default — the terminal answered neither CSI 16 t \
                     nor the tty ioctl, so every device box below is computed from a guess",
                    true,
                ),
                CellSource::Unexplained => (
                    "UNEXPLAINED",
                    "neither the CSI 16 t answer nor the live ioctl accounts for this — it is \
                     probably a stale measurement",
                    true,
                ),
                CellSource::None => unreachable!("handled above"),
            };
            let line = format!("  cell size: {w}x{h} px — {label} ({why})");
            out.push(if guessed { assumed(line) } else { value(line) });

            // The aspect, and its signed distance from the 2.000 that makes a
            // half-block sample square. A cell is round(advance·px) by
            // round(line·px) and the two round at different rates, so real cells
            // swing 1.75..2.25 even for a face whose design ratio is 2.002 —
            // naming the deviation is actionable, because Ghostty's
            // `adjust-cell-height`/`adjust-cell-width` act directly on it.
            if w > 0 {
                let aspect = f64::from(h) / f64::from(w);
                out.push(value(format!(
                    "  cell aspect: {aspect:.3} (height/width), {:+.3} from the 2.000 that makes a \
                     half-block sample square (Ghostty: adjust-cell-height / adjust-cell-width)",
                    aspect - 2.0
                )));
            }
        }
    }
    if let Some((w, h)) = s.reported_cell {
        out.push(value(format!("    CSI 16 t answered: {w}x{h} px")));
    } else {
        out.push(value("    CSI 16 t answered: nothing"));
    }
    match s.ioctl_cell {
        Some((w, h)) => out.push(value(format!("    TIOCGWINSZ says now: {w}x{h} px"))),
        None => out.push(value(
            "    TIOCGWINSZ says now: nothing (the tty reports no pixel geometry; Windows never does)",
        )),
    }

    // ── capabilities ─────────────────────────────────────────────────────────
    match (s.probe, s.capabilities.is_empty()) {
        (Probe::NotAskedImagesOff, _) => {
            out.push(assumed("  capabilities: NOT ASKED — --images off, so no probe was sent"))
        }
        (Probe::NotAskedHalfblocksForced, _) => out.push(assumed(
            "  capabilities: NOT ASKED — --image-protocol halfblocks builds a picker that never \
             queries the terminal",
        )),
        (Probe::Asked, true) => out.push(assumed(
            "  capabilities: NONE — the terminal was asked and answered nothing (an empty list \
             here is a refusal, not a missing probe)",
        )),
        (Probe::Asked, false) => {
            out.push(value(format!("  capabilities ({}), as answered:", s.capabilities.len())));
            for c in &s.capabilities {
                out.push(value(format!("    {c}")));
            }
        }
    }

    // ── compression: the question this command was built to answer ───────────
    out.push(value("  kitty transmission compression (o=z):"));
    let ratatui_line = if s.protocol.as_deref() != Some("kitty") {
        assumed("    ratatui-image uploads: n/a — the protocol in force is not kitty".to_string())
    } else if s.kitty_compression {
        value(
            "    ratatui-image uploads (v6 chrome bands, the raster composite, cover + inline \
             art): COMPRESSED — the terminal answered the o=z probe"
                .to_string(),
        )
    } else {
        assumed(
            "    ratatui-image uploads (v6 chrome bands, the raster composite, cover + inline \
             art): RAW — the terminal did not answer the o=z probe, so every image goes down \
             the wire uncompressed"
                .to_string(),
        )
    };
    out.push(ratatui_line);
    // The two encoders read the same probe since SQ-0997. They are still reported
    // separately, because they are separate code and one of them silently ignored
    // the answer for as long as both existed — a single merged line would say
    // nothing about whether that is still true.
    if s.protocol.as_deref() == Some("kitty") {
        out.push(if s.kitty_compression {
            value(
                "    graphics-window uploads (lanthorn's own transmit — Glulx toolbars, Scott \
                 room pictures, v6 graphics windows): COMPRESSED — the same o=z probe governs \
                 both encoders",
            )
        } else {
            assumed(
                "    graphics-window uploads (lanthorn's own transmit — Glulx toolbars, Scott \
                 room pictures, v6 graphics windows): RAW — the terminal did not answer the o=z \
                 probe, and a transmit it cannot inflate would store no image at all",
            )
        });
    }

    // ── render state, insofar as it explains the traffic ─────────────────────
    out.push(heading("render"));
    out.push(value(format!(
        "  story pane: {}x{} cells = {} cell(s)",
        s.pane_cells.0,
        s.pane_cells.1,
        thousands(u64::from(s.pane_cells.0) * u64::from(s.pane_cells.1))
    )));
    match &s.render {
        None => out.push(value("  v6: not a Version 6 session — no pixel path, no magnification")),
        Some(r) => {
            out.push(value(format!("  v6 mode: {}", r.mode)));
            let takeover = if !r.takeover_evaluated {
                "  picture takeover: not evaluated — the hatch is a hybrid-mode test and this \
                 session renders raster"
                    .to_string()
            } else {
                match r.takeover {
                    None => "  picture takeover: none — the hybrid ring drew the last frame".to_string(),
                    Some(arm) => format!("  picture takeover: {arm} — the last frame fell through to raster"),
                }
            };
            out.push(value(takeover));
            out.push(value(format!(
                "  native screen: {}x{} game pixels, art_scale {}x{}",
                r.native.0, r.native.1, r.art_scale.0, r.art_scale.1
            )));
            // SQ-0978: three outcomes, not two. "FELL BACK" says the pane is too
            // small, which a player can act on by resizing; "INERT" says the backend
            // has no rung at any pane size, which they cannot. Reporting the second
            // as the first sends a reader hunting for a bigger terminal, and claiming
            // "snapped to the artwork's ladder" on half-blocks would be a guarantee
            // that does not hold at all.
            let lock = match (r.pixel_lock, r.pixel_lock_inapplicable, r.pixel_lock_fell_back) {
                (false, _, _) => "pixel lock off (free scaling)".to_string(),
                (true, true, _) => "pixel lock ON but INERT — half-blocks resolves the picture into \
                                    CELLS (one sample per column, two per row) and never sees a \
                                    device pixel, so there is no rung to snap to at any pane size; \
                                    this frame free-scaled"
                    .to_string(),
                (true, false, false) => "pixel lock ON (snapped to the artwork's ladder)".to_string(),
                (true, false, true) => "pixel lock ON but FELL BACK — the pane is too small for even \
                                        the lowest rung, so this frame free-scaled"
                    .to_string(),
            };
            // The magnification is device pixels per unit pixel — and under
            // half-blocks the device pixel is `Picker::halfblocks`'s hardcoded 10x20,
            // not the font on screen, so the bare number reads about ten times what
            // the picture actually resolves at. Say so where it is printed rather
            // than letting the reader assume the units they know (SQ-0978).
            let nominal = matches!(s.protocol.as_deref(), Some("halfblocks"));
            let mag_line = format!(
                "  magnification: {:.3}x{}, {lock}",
                r.magnification,
                if nominal { " in NOMINAL 10x20 device pixels (half-blocks draws cells)" } else { "" }
            );
            let hedged = r.pixel_lock && (r.pixel_lock_fell_back || r.pixel_lock_inapplicable);
            out.push(if hedged || nominal { assumed(mag_line) } else { value(mag_line) });
            if !r.paths.is_empty() {
                out.push(value(format!("  recent render paths (oldest first): {}", r.paths.join(" · "))));
            }
        }
    }

    // ── traffic ──────────────────────────────────────────────────────────────
    out.push(heading("traffic"));
    match s.traffic {
        None => out.push(assumed(
            "  bytes written: unavailable — the counting writer is only installed on the real \
             terminal, and this session has none",
        )),
        Some(t) => {
            out.push(value(format!(
                "  bytes written to the terminal: {} in {} frame flush(es) since launch",
                thousands(t.total_bytes),
                thousands(t.flushes)
            )));
            out.push(value(format!("  last drawn frame: {} bytes", thousands(t.last_flush_bytes))));
            if let Some(mean) = t.total_bytes.checked_div(t.flushes) {
                out.push(value(format!("  mean per frame: {} bytes", thousands(mean))));
            }
            out.push(value(
                "  (the ratatui backend's own writes only; the alternate-screen and mouse-mode \
                 escapes go straight to stdout and are not counted)",
            ));
        }
    }
    out.push(value(format!(
        "  graphics ops on the last recorded frame: {} upload(s), {} reuse(s), {} placement(s), \
         {} drop(s)",
        s.ops.uploads, s.ops.reuses, s.ops.places, s.ops.drops
    )));
    out.push(value(format!(
        "  placeholder cells under those placements: {} — {}",
        thousands(s.ops.placed_cells),
        TerminalSnapshot::apc_note()
    )));
    out.push(value(format!(
        "  chrome-band / composite uploads since launch: {}",
        thousands(s.band_encodes)
    )));
    // SQ-1005: what compression actually bought, on THIS session's real workload.
    // Read off the transmits rather than out of an encoder, which is the only way
    // one number can speak for both of them — lanthorn emits its graphics-window
    // uploads itself and `ratatui-image` encodes everything else.
    let u = s.uploads;
    if u.uploads == 0 {
        out.push(value("  kitty upload bytes: nothing uploaded yet this session"));
    } else {
        let raw = u.wire_uncompressed();
        out.push(value(format!(
            "  kitty uploads: {} image(s), {} pixel bytes",
            thousands(u.uploads),
            thousands(u.pixels)
        )));
        out.push(value(format!(
            "  on the wire: {} bytes, against {} uncompressed{}",
            thousands(u.wire),
            thousands(raw),
            match (raw.checked_sub(u.wire), raw) {
                (Some(saved), r) if r > 0 && saved > 0 => format!(
                    " — {}x smaller, {} bytes saved ({}%)",
                    format_args!("{:.1}", r as f64 / u.wire.max(1) as f64),
                    thousands(saved),
                    saved * 100 / r
                ),
                _ => String::new(),
            }
        )));
        out.push(assumed(
            "  (both measured from the transmits' own control blocks — `s`x`v`x4 for f=32 RGBA, or \
             `S` when one is declared. The uncompressed figure is base64'd too, since `o=z` never \
             removed the 4/3 expansion and crediting it with that would flatter the ratio.)",
        ));
        // SQ-1201: whether an eviction/replacement actually freed the upload it
        // replaced, or just forgot it (SQ-1190's bug class). `stranded_uploads` is
        // this struct's own traffic still resident by id — kept live as typed
        // state (`GraphicsRender::outstanding`), not re-scanned off the wire.
        out.push(value(format!(
            "  wire hygiene: {} delete(s) · {} pixel bytes freed · {} upload(s) stranded ({} pixel bytes)",
            thousands(u.deletes),
            thousands(u.freed_pixels),
            thousands(u.stranded_uploads),
            thousands(u.stranded_pixels),
        )));
    }
    out
}

/// The report as plain text, for the `dump-terminal.log` mirror.
pub fn dump_text(s: &TerminalSnapshot) -> Vec<String> {
    dump_lines(s).into_iter().map(|l| l.text).collect()
}

#[cfg(all(test, feature = "t-misc"))]
mod tests {
    use super::*;

    fn snap() -> TerminalSnapshot {
        TerminalSnapshot {
            protocol: Some("kitty".into()),
            forced_protocol: None,
            probe: Probe::Asked,
            cell: Some((8, 18)),
            cell_source: CellSource::Measured,
            reported_cell: Some((8, 18)),
            ioctl_cell: Some((8, 18)),
            capabilities: vec!["Kitty".into(), "KittyCompression".into()],
            kitty_compression: true,
            pane_cells: (115, 61),
            render: None,
            traffic: Some(TrafficStats { total_bytes: 1_234_567, flushes: 20, last_flush_bytes: 48_213 }),
            band_encodes: 87,
            uploads: crate::render::graphics::UploadBytes {
                wire: 200_000,
                pixels: 12_000_000,
                uploads: 9,
                deletes: 6,
                freed_pixels: 8_000_000,
                stranded_uploads: 2,
                stranded_pixels: 4_000_000,
            },
            ops: OpCounts { uploads: 3, reuses: 12, places: 15, drops: 2, placed_cells: 65_952 },
        }
    }

    fn text(s: &TerminalSnapshot) -> String {
        dump_text(s).join("\n")
    }

    /// The whole point of the command: the same `10x20` reads differently
    /// depending on where it came from, and the report has to say which.
    #[test]
    fn an_assumed_cell_size_is_marked_and_a_measured_one_is_not() {
        let mut s = snap();
        s.cell = Some((10, 20));
        s.cell_source = CellSource::Assumed;
        s.reported_cell = None;
        s.ioctl_cell = None;
        let lines = dump_lines(&s);
        let cell = lines.iter().find(|l| l.text.contains("cell size:")).expect("a cell-size line");
        assert_eq!(cell.kind, DumpKind::Assumed, "an assumed cell size is not an ordinary value");
        assert!(cell.text.contains("ASSUMED"), "{}", cell.text);

        let s = snap();
        let lines = dump_lines(&s);
        let cell = lines.iter().find(|l| l.text.contains("cell size:")).expect("a cell-size line");
        assert_eq!(cell.kind, DumpKind::Value);
        assert!(cell.text.contains("MEASURED"), "{}", cell.text);
        assert!(cell.text.contains("8x18"), "{}", cell.text);
    }

    /// `Derived` is a measurement too — of a different thing. It must not be
    /// flagged as a guess, and it must name the ioctl so the reader knows the
    /// `CSI 16 t` route went unanswered.
    #[test]
    fn a_derived_cell_size_names_the_ioctl_and_is_not_flagged() {
        let mut s = snap();
        s.cell_source = CellSource::Derived;
        s.reported_cell = None;
        let cell = dump_lines(&s).into_iter().find(|l| l.text.contains("cell size:")).unwrap();
        assert_eq!(cell.kind, DumpKind::Value);
        assert!(cell.text.contains("TIOCGWINSZ"), "{}", cell.text);
        assert!(text(&s).contains("CSI 16 t answered: nothing"));
    }

    /// The aspect is reported with its SIGNED distance from 2.000, because the
    /// sign is what says which of Ghostty's two knobs to reach for.
    #[test]
    fn the_aspect_carries_a_signed_distance_from_two() {
        let mut s = snap();
        s.cell = Some((8, 18)); // 2.250
        assert!(text(&s).contains("2.250"), "{}", text(&s));
        assert!(text(&s).contains("+0.250"), "{}", text(&s));

        s.cell = Some((10, 17)); // 1.700
        let t = text(&s);
        assert!(t.contains("1.700"), "{t}");
        assert!(t.contains("-0.300"), "{t}");
    }

    /// An empty capability list is three different facts, and the report must
    /// never let them look alike.
    #[test]
    fn an_empty_capability_list_says_which_kind_of_empty_it_is() {
        let mut s = snap();
        s.capabilities.clear();

        s.probe = Probe::Asked;
        let l = dump_lines(&s).into_iter().find(|l| l.text.contains("capabilities")).unwrap();
        assert_eq!(l.kind, DumpKind::Assumed);
        assert!(l.text.contains("answered nothing"), "{}", l.text);

        s.probe = Probe::NotAskedHalfblocksForced;
        let l = dump_lines(&s).into_iter().find(|l| l.text.contains("capabilities")).unwrap();
        assert!(l.text.contains("NOT ASKED"), "{}", l.text);
        assert!(l.text.contains("halfblocks"), "{}", l.text);

        s.probe = Probe::NotAskedImagesOff;
        let l = dump_lines(&s).into_iter().find(|l| l.text.contains("capabilities")).unwrap();
        assert!(l.text.contains("--images off"), "{}", l.text);
    }

    /// The question that prompted the whole command. Both directions, because
    /// both fail silently: a raw wire looks like nothing, and so does a
    /// terminal that cannot inflate.
    ///
    /// **Both encoders, and the same answer from each** (SQ-0997). This line used
    /// to report the graphics-window path as compressed "unconditionally", which
    /// was true and was the defect; a report that still said so after the fix
    /// would be lying about the one thing it exists to tell you.
    #[test]
    fn compression_is_reported_both_ways_and_raw_is_flagged() {
        let s = snap();
        for path in ["ratatui-image uploads", "graphics-window uploads"] {
            let l = dump_lines(&s).into_iter().find(|l| l.text.contains(path)).unwrap();
            assert_eq!(l.kind, DumpKind::Value, "{path}: {}", l.text);
            assert!(l.text.contains("COMPRESSED"), "{path}: {}", l.text);
        }

        let mut s = snap();
        s.kitty_compression = false;
        s.capabilities = vec!["Kitty".into()];
        for path in ["ratatui-image uploads", "graphics-window uploads"] {
            let l = dump_lines(&s).into_iter().find(|l| l.text.contains(path)).unwrap();
            assert_eq!(l.kind, DumpKind::Assumed, "{path}: a silently raw wire needs marking");
            assert!(l.text.contains("RAW"), "{path}: {}", l.text);
        }
        assert!(
            !text(&s).contains("unconditionally"),
            "no path states o=z whatever the probe said any more (SQ-0997)"
        );
    }

    /// Not-kitty must not claim either answer — `o=z` is a kitty key and a sixel
    /// session has no opinion about it.
    #[test]
    fn a_non_kitty_protocol_reports_compression_as_not_applicable() {
        let mut s = snap();
        s.protocol = Some("halfblocks".into());
        s.kitty_compression = false;
        let t = text(&s);
        assert!(t.contains("n/a — the protocol in force is not kitty"), "{t}");
        assert!(!t.contains("graphics-window uploads"), "no kitty, no kitty transmit: {t}");
    }

    /// The stats the user asked for, and the two the report declines to invent.
    #[test]
    fn the_traffic_section_carries_the_counters_and_names_what_it_cannot_reach() {
        let t = text(&snap());
        assert!(t.contains("1,234,567"), "total bytes, grouped: {t}");
        assert!(t.contains("48,213"), "last frame: {t}");
        assert!(t.contains("61,728"), "mean per frame = 1234567/20: {t}");
        assert!(t.contains("65,952"), "placeholder cells: {t}");
        assert!(t.contains("115x61 cells = 7,015"), "the pane grid and its product: {t}");
        assert!(t.contains("3 upload(s), 12 reuse(s), 15 placement(s), 2 drop(s)"), "{t}");
        // SQ-1005: what compression bought, not a disclaimer that it cannot be known.
        assert!(t.contains("9 image(s), 12,000,000 pixel bytes"), "the upload totals: {t}");
        assert!(t.contains("200,000 bytes, against 16,000,000 uncompressed"), "both sides: {t}");
        assert!(t.contains("80.0x smaller"), "the ratio: {t}");
        assert!(t.contains("15,800,000 bytes saved (98%)"), "the saving: {t}");
    }

    /// SQ-1201: the freed-vs-stranded line beside the upload counter — the one
    /// SQ-1190's whole class of bug (an eviction that replaced or dropped a
    /// `Protocol` without ever emitting its `a=d`) would have been caught by, had
    /// it existed then.
    #[test]
    fn the_wire_hygiene_line_reports_deletes_freed_and_stranded() {
        let t = text(&snap());
        assert!(
            t.contains("wire hygiene: 6 delete(s) · 8,000,000 pixel bytes freed · 2 upload(s) stranded \
                        (4,000,000 pixel bytes)"),
            "{t}"
        );
    }

    /// With no terminal there are no byte counts, and saying so is the honest
    /// answer — a zero would read as "this session emitted nothing".
    #[test]
    fn a_session_with_no_terminal_reports_the_counters_as_unavailable() {
        let mut s = snap();
        s.traffic = None;
        let l = dump_lines(&s).into_iter().find(|l| l.text.contains("bytes written")).unwrap();
        assert_eq!(l.kind, DumpKind::Assumed);
        assert!(l.text.contains("unavailable"), "{}", l.text);
    }

    /// In raster mode the takeover hatch is never reached, so whatever the cell
    /// holds is stale. The report says "not evaluated" rather than "none".
    #[test]
    fn a_raster_session_does_not_claim_a_takeover_verdict() {
        let mut s = snap();
        s.render = Some(RenderFacts {
            mode: "raster",
            takeover: None,
            takeover_evaluated: false,
            native: (640, 400),
            art_scale: (2, 2),
            magnification: 1.5,
            pixel_lock: false,
            pixel_lock_fell_back: false,
            pixel_lock_inapplicable: false,
            paths: vec!["raster x4".into()],
        });
        let t = text(&s);
        assert!(t.contains("picture takeover: not evaluated"), "{t}");
        assert!(t.contains("640x400 game pixels, art_scale 2x2"), "{t}");
        assert!(t.contains("pixel lock off"), "{t}");
        assert!(t.contains("recent render paths (oldest first): raster x4"), "{t}");
    }

    /// A lock the pane was too small to honour is a value that is not what the
    /// user asked for, which is the same class of thing as an assumed cell size.
    #[test]
    fn a_pixel_lock_that_fell_back_is_flagged() {
        let mut s = snap();
        s.render = Some(RenderFacts {
            mode: "hybrid",
            takeover: Some("art_paints_anything"),
            takeover_evaluated: true,
            native: (640, 400),
            art_scale: (2, 2),
            magnification: 1.0,
            pixel_lock: true,
            pixel_lock_fell_back: true,
            pixel_lock_inapplicable: false,
            paths: vec![],
        });
        let l = dump_lines(&s).into_iter().find(|l| l.text.contains("magnification")).unwrap();
        assert_eq!(l.kind, DumpKind::Assumed);
        assert!(l.text.contains("FELL BACK"), "{}", l.text);
        assert!(text(&s).contains("picture takeover: art_paints_anything"));
    }

    /// SQ-0978: a lock the BACKEND cannot honour reads differently from a lock the
    /// PANE was too small for. The report must not offer the reader a resize that
    /// would not help, and must not claim the snap happened.
    #[test]
    fn a_pixel_lock_on_a_cell_backend_is_reported_as_inert_not_as_a_snap() {
        let mut s = snap();
        s.protocol = Some("halfblocks".into());
        s.render = Some(RenderFacts {
            mode: "hybrid",
            takeover: None,
            takeover_evaluated: true,
            native: (640, 400),
            art_scale: (2, 2),
            magnification: 1.531,
            pixel_lock: true,
            pixel_lock_fell_back: false,
            pixel_lock_inapplicable: true,
            paths: vec![],
        });
        let l = dump_lines(&s).into_iter().find(|l| l.text.contains("magnification")).unwrap();
        assert_eq!(l.kind, DumpKind::Assumed, "an inert lock is not the value the user asked for");
        assert!(l.text.contains("INERT"), "{}", l.text);
        assert!(
            !l.text.contains("snapped to the artwork's ladder"),
            "the guarantee did not hold, so the report must not claim it: {}",
            l.text
        );
        assert!(
            !l.text.contains("FELL BACK") && !l.text.contains("too small"),
            "no pane size would have honoured this — pointing at the pane misdirects: {}",
            l.text
        );
        // And the magnification itself is in the picker's invented 10x20, not in
        // pixels this terminal has.
        assert!(l.text.contains("NOMINAL 10x20"), "{}", l.text);
    }

    /// The same number under a backend that really has device pixels is a
    /// measurement, and carries no hedge at all.
    #[test]
    fn a_kitty_magnification_is_reported_without_a_nominal_hedge() {
        let mut s = snap();
        s.protocol = Some("kitty".into());
        s.render = Some(RenderFacts {
            mode: "hybrid",
            takeover: None,
            takeover_evaluated: true,
            native: (640, 400),
            art_scale: (2, 2),
            magnification: 1.5,
            pixel_lock: true,
            pixel_lock_fell_back: false,
            pixel_lock_inapplicable: false,
            paths: vec![],
        });
        let l = dump_lines(&s).into_iter().find(|l| l.text.contains("magnification")).unwrap();
        assert_eq!(l.kind, DumpKind::Value, "{}", l.text);
        assert!(l.text.contains("snapped to the artwork's ladder"), "{}", l.text);
        assert!(!l.text.contains("NOMINAL"), "{}", l.text);
    }

    /// A forced protocol is not a detected one, and a bug report needs to know
    /// which it was looking at.
    #[test]
    fn a_forced_protocol_says_so() {
        let mut s = snap();
        s.forced_protocol = Some("kitty".into());
        assert!(text(&s).contains("FORCED by --image-protocol kitty"), "{}", text(&s));
        assert!(text(&snap()).contains("(auto-detected)"));
    }

    /// `dump_text` and `dump_lines` are the same report — the file mirror must
    /// never be able to drift from the screen copy.
    #[test]
    fn the_text_mirror_is_the_same_lines() {
        let s = snap();
        let lines: Vec<String> = dump_lines(&s).into_iter().map(|l| l.text).collect();
        assert_eq!(dump_text(&s), lines);
    }

    /// One `write` is one `fetch_add`; a flush that carried nothing is not a
    /// frame. Both are the properties that keep this off the frame path.
    #[test]
    fn the_counting_writer_counts_bytes_and_non_empty_flushes() {
        let traffic: TrafficHandle = Arc::new(Traffic::default());
        let mut w = CountingWriter::new(Vec::new(), Arc::clone(&traffic));
        w.write_all(b"hello").unwrap();
        w.write_all(b" world").unwrap();
        assert_eq!(traffic.total_bytes(), 11);
        assert_eq!(traffic.flushes(), 0, "nothing has been flushed yet");
        w.flush().unwrap();
        assert_eq!(traffic.flushes(), 1);
        assert_eq!(traffic.last_flush_bytes(), 11);

        // An empty flush must not count as a frame, or "bytes per frame" becomes
        // an average over frames that were never drawn.
        w.flush().unwrap();
        assert_eq!(traffic.flushes(), 1);
        assert_eq!(traffic.last_flush_bytes(), 11);

        w.write_all(b"!").unwrap();
        w.flush().unwrap();
        assert_eq!(traffic.flushes(), 2);
        assert_eq!(traffic.last_flush_bytes(), 1);
        assert_eq!(traffic.total_bytes(), 12);
    }
}
