// GameSession — drives one VM turn, captures transcript output, mapper bridge.
//
// Transcript capture approach: we use a custom `CaptureSink` (rather than
// reusing `zvm::io::BufferOutput`) because `BufferOutput` has no drain/clear
// method.  `CaptureSink` implements `zvm::io::Output` and exposes `take_text`
// to drain accumulated text between turns.  After construction the sink is
// accessed by downcasting `machine.out` via the `as_any()` supertrait —
// `machine.out` is `pub`, so no zvm visibility change is required.
//
// zvm change made for this module: added Output::as_any_mut (+ BufferOutput/StdoutOutput impls) to allow mutable downcast to CaptureSink.

use std::any::Any;

use mapper::direction::parse_direction;
use mapper::mapper::Mapper;
use zvm::cpu::exec::{Machine, PictureEvent, SoundEvent, StepResult};
use zvm::error::ZError;
use zvm::io::{Output, TextAttrs};
use zvm::location::{detect_location, Location, LocationMethod};
use zvm::screen::ZColour;
use zvm::ObjectSnapshot;

use crate::state::ParaFmt;
use zvm::memory::Memory;

// ── InputKind ─────────────────────────────────────────────────────────────────

/// Which kind of input the VM is waiting for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    /// Waiting for a full line of text (`read` / `sread` opcode).
    Line,
    /// Waiting for a single keypress (`read_char` opcode).
    Char,
    /// Waiting on a non-input Glk event only — a timer, mouse, or hyperlink
    /// (`glk_select` with no line/char request; Glulx, Glk §4.4). The game
    /// requested no typed input, so the host shows no prompt/cursor and delivers
    /// the event (a timer tick on its clock, a mouse/hyperlink event on a click)
    /// rather than a keystroke. Never produced by the Z-machine engine.
    Event,
}

/// Which in-game (game-initiated) I/O the VM is suspended on after a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingIo {
    Save,
    Restore,
}

/// A game-initiated Glk `create_by_prompt` awaiting a host-supplied filename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilenameReq {
    /// Glk fileusage.
    pub usage: u32,
    /// Glk filemode (Read `0x02`, Write `0x01`, ReadWrite `0x03`, WriteAppend `0x05`).
    pub fmode: u32,
}

// ── CaptureSink ───────────────────────────────────────────────────────────────

/// An output sink that accumulates printed text and lets the caller drain it.
///
/// `runs` records one `(char_count, text_style_bits, fg, bg, link, para)` chunk
/// per `print`/`print_styled`/`print_attr` call, in lockstep with the appended
/// text, so callers can reconstruct which spans carried Z-machine emphasis and
/// colour. `link` is the Glk hyperlink value (always 0 on the Z-machine path);
/// `para` is the paragraph layout format (always [`ParaFmt::default`] on the
/// Z-machine path — the Glulx buffer path is the only source of non-default
/// layout, carried via [`crate::glk_backend::AppGlk::take_transcript_elems`]).
/// One captured `(char_count, text_style_bits, fg, bg, link, para, glk_style,
/// nowrap)` chunk. `nowrap` is `true` when the run was printed with the
/// Z-machine's output buffering switched OFF (`buffer_mode 0`, ZMSD §7.2.1), so
/// the renderer must break it after the last character that fits instead of
/// word-wrapping it.
pub type CaptureRun = (usize, u8, ZColour, ZColour, u32, ParaFmt, u8, bool);

pub struct CaptureSink {
    pub text: String,
    pub runs: Vec<CaptureRun>,
    /// Current `buffer_mode` state (ZMSD §7.2.1: on at the start of a game).
    /// Every captured run records `!buffering` so the transcript can honour the
    /// game's choice when it wraps.
    buffering: bool,
    /// How many characters this sink had taken when the game last cleared the
    /// scrolling window, since the last drain — the screen-clear boundary's
    /// position WITHIN the turn (SQ-0751).
    ///
    /// `erase_window` sets a flag the host reads after the turn is over, which
    /// cannot say where in the turn the erase fell; a turn that prints and then
    /// erases would keep its pre-erase text on the cleared screen. `Machine`
    /// announces the erase as it executes ([`Output::screen_cleared`]) and this is
    /// where it lands. The LAST erase of a turn wins: it is the one whose screen
    /// the player is left looking at.
    cleared_at: Option<usize>,
}

impl CaptureSink {
    fn new() -> Self {
        CaptureSink { text: String::new(), runs: Vec::new(), buffering: true, cleared_at: None }
    }

    /// Drain accumulated text and style runs together, leaving both empty.
    pub fn take_styled(&mut self) -> (String, Vec<CaptureRun>) {
        (std::mem::take(&mut self.text), std::mem::take(&mut self.runs))
    }

    /// Take the screen-clear position recorded since the last call, in characters
    /// from the start of the drained text (SQ-0751). See [`Self::cleared_at`].
    pub fn take_cleared_at(&mut self) -> Option<usize> {
        self.cleared_at.take()
    }

    /// Drain all accumulated text, leaving the buffer empty.
    pub fn take_text(&mut self) -> String {
        self.take_styled().0
    }
}

impl Output for CaptureSink {
    fn print(&mut self, s: &str) {
        let nowrap = !self.buffering;
        self.runs.push((s.chars().count(), 0, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, nowrap));
        self.text.push_str(s);
    }
    fn print_styled(&mut self, s: &str, style: u8) {
        let nowrap = !self.buffering;
        self.runs.push((s.chars().count(), style, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, nowrap));
        self.text.push_str(s);
    }
    fn print_attr(&mut self, s: &str, attrs: TextAttrs) {
        let nowrap = !self.buffering;
        self.runs.push((s.chars().count(), attrs.style, attrs.fg, attrs.bg, 0, ParaFmt::default(), 0, nowrap));
        self.text.push_str(s);
    }
    /// ZMSD §7.2.1: the game switched buffering on/off. Runs captured while it
    /// is off are marked `nowrap`, and the transcript breaks them after the last
    /// character that fits (no word-wrap) — see [`ParaFmt::nowrap_from`].
    fn set_buffer_mode(&mut self, on: bool) {
        self.buffering = on;
    }
    /// SQ-0751: the game cleared the scrolling window HERE, this many characters
    /// into what this turn has printed so far. A second erase in the same turn
    /// overwrites the first — the screen the player ends the turn looking at is the
    /// one the last erase opened.
    fn screen_cleared(&mut self) {
        self.cleared_at = Some(self.text.chars().count());
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Trim a `(char_count, bits, fg, bg)` chunk list so its total char-count equals
/// `char_len` (used after `strip_read_prompt` shortens the captured text by a
/// trailing prompt). Chunks past the limit are dropped; the boundary chunk is
/// truncated. A list shorter than `char_len` is returned unchanged (the missing
/// tail is treated as plain by `push_transcript_runs`).
pub(crate) fn clamp_runs(runs: Vec<CaptureRun>, char_len: usize) -> Vec<CaptureRun> {
    let mut out = Vec::with_capacity(runs.len());
    let mut total = 0usize;
    for (c, b, fg, bg, link, para, gs, nw) in runs {
        if total >= char_len {
            break;
        }
        let take = c.min(char_len - total);
        out.push((take, b, fg, bg, link, para, gs, nw));
        total += take;
    }
    out
}

/// Interleave v6 window-0 inline pictures into a turn's styled text as ordered
/// [`TranscriptElem`]s. Each picture carries the absolute win0 output-char
/// offset it was drawn at (`PictureEvent::out_chars`); `base` is the count at
/// the start of this turn's text, so `abs - base` is the picture's position
/// within `text`. Offsets snap DOWN to the start of their line (v6 games draw
/// inline art at the text cursor, i.e. at line starts — snapping keeps a
/// mid-line offset from splitting a paragraph in two, since
/// `push_transcript_runs` starts a new transcript line per `Text` element).
/// The line separator consumed by a split is dropped from the emitted text
/// (the element boundary itself is the break) and its style-chunk char is
/// consumed in lockstep.
/// The factor by which Infocom v6 artwork (320×200 MCGA) is scaled into the
/// presentation UNIT space. Reference interpreters (Frotz DOS/Amiga, `bcpic.c`
/// `scaler = 2`; SDL `m_v6scale = 2`) present v6 on a 640×400 screen and blit
/// each 320×200 picture at 2×, returning the doubled dimensions to the game so
/// its layout math lands on the 640-wide screen (SQ-0479). Both the screen
/// seeding (2×Reso) and every picture crossing into unit space use this one
/// factor, so screen and picture dimensions scale together — the `is_content_art`
/// ratios (below) stay valid because both numerator and denominator double.
pub(crate) const V6_ART_SCALE: u32 = 2;

/// Scale a native-resolution v6 picture into UNIT space (×`scale`,
/// nearest-neighbour — the DOS-authentic crisp pixel double). Every picture that
/// crosses from PictSource's art-native pixels into the 640×400 unit screen goes
/// through here exactly once, so window canvases, inline floats and the
/// `is_content_art` classification all see one consistent unit-space size.
///
/// `scale` is the session's [`GameSession::art_scale`], PER AXIS: `(2, 2)` for
/// art with a standard window to be scaled against, `(1, 1)` for non-scalable
/// art, and `(1, 2)` for an EGA/CGA rendition whose pixels are half as wide
/// (SQ-0790).
fn v6_scaled_art(img: &image::DynamicImage, scale: (u32, u32)) -> image::DynamicImage {
    use image::GenericImageView;
    if scale == (1, 1) {
        return img.clone();
    }
    let (w, h) = img.dimensions();
    image::DynamicImage::ImageRgba8(image::imageops::resize(
        img,
        w * scale.0,
        h * scale.1,
        image::imageops::FilterType::Nearest,
    ))
}

/// Classify a picture as CONTENT art versus decorative FRAME art (borders,
/// tiles). (SQ-0461 decision 3)
///
/// SQ-0461 asked this to decide whether a graphics-window draw was worth an
/// inline transcript band for the frameless mode; SQ-0895 removed the mode and
/// that caller with it. The surviving caller is [`win0_float_align`], where the
/// same question decides whether a window-0 picture floats inline or takes a
/// left margin — so the classifier is still load-bearing, just for one consumer
/// rather than two.
///
/// A picture is CONTENT when it covers **≥ 40% of the screen area**, OR is **≥
/// 60% of screen width AND ≥ 30% of screen height**. Narrow strips (**≤ 15% of
/// screen width** — Shogun's 23px side borders) are always FRAME, as is anything
/// that doesn't clearly meet a content rule (short bands, small compass tiles).
///
/// Worked examples (screen 640×400 unit space; picture dims are the native
/// 320×200 art scaled by [`V6_ART_SCALE`] before the ratio, so both sides of
/// every comparison are in unit space and the ratios are unchanged from the
/// pre-SQ-0479 320×200-everywhere world):
/// - Shogun title splash 320×200 →×2 640×400 → area 100% ⇒ **content**.
/// - Shogun side border 23×200 →×2 46×400 → width 46 ≤ 96 (15% of 640) ⇒ **frame**.
/// - Zork Zero compass tile ~24×24 →×2 48×48 → width 48 < 60% and area ~0.9% ⇒ **frame**.
fn is_content_art(pic_w: u32, pic_h: u32, screen_w: u32, screen_h: u32) -> bool {
    let screen_w = screen_w.max(1);
    let screen_h = screen_h.max(1);
    // Narrow vertical strip → decorative frame, regardless of height.
    if pic_w * 100 <= screen_w * 15 {
        return false;
    }
    let pic_area = pic_w as u64 * pic_h as u64;
    let screen_area = screen_w as u64 * screen_h as u64;
    if pic_area * 100 >= screen_area * 40 {
        return true;
    }
    pic_w * 100 >= screen_w * 60 && pic_h * 100 >= screen_h * 30
}

/// The float alignment for a window-0 picture, given the picture's pixel size,
/// the screen size, the picture's draw x, and window 0's `set_margins` state
/// (all in the game's window pixel space).
///
/// Priority:
/// 1. **Right-margin picture** (`MarginRight`) — Shogun's opening: the game drew
///    the picture at the window's right edge and set a large RIGHT margin so its
///    prose flows in the LEFT column beside it, then full width once the text
///    scrolls past (ZMSD §15). Signature: an asymmetric right margin, a
///    prose-wide left text column (`x_size - right - left`), and the picture
///    beginning at/after that column's right edge.
/// 2. **Content art** (`InlineUp`) — a large centred illustration with no margin
///    reservation renders as a full-width band, not a drop-cap (SQ-0471).
/// 3. **Drop-cap / room icon** (`MarginLeft`) — Zork Zero's initial letter and
///    small tiles float at the left margin with text beside them.
fn win0_pic_align(
    iw: u32,
    ih: u32,
    screen_w: u32,
    screen_h: u32,
    pic_x: u16,
    left_margin: u16,
    right_margin: u16,
    win_w: u16,
) -> crate::inline_image::ImageAlign {
    // Minimum reservations (game pixels): a real right margin (not a thin frame
    // inset), and a prose-wide left column (~6 cells at the 8px v6 cell).
    const MIN_RIGHT_MARGIN_PX: u16 = 48;
    const MIN_TEXT_COL_PX: u16 = 48;
    // Slack for the gap the game leaves between the text column and the picture.
    const PIC_START_TOL: u16 = 48;
    let text_right = win_w.saturating_sub(right_margin); // text column's right edge
    let text_col = text_right.saturating_sub(left_margin); // left text column width
    let is_margin_right = right_margin > left_margin
        && right_margin >= MIN_RIGHT_MARGIN_PX
        && text_col >= MIN_TEXT_COL_PX
        && pic_x as u32 + iw >= win_w as u32 / 2 // picture predominantly on the right
        && pic_x.saturating_add(PIC_START_TOL) >= text_right; // begins at/after the column
    if is_margin_right {
        return crate::inline_image::ImageAlign::MarginRight;
    }
    if is_content_art(iw, ih, screen_w, screen_h) {
        crate::inline_image::ImageAlign::InlineUp
    } else {
        crate::inline_image::ImageAlign::MarginLeft
    }
}

fn interleave_story_elems(
    text: &str,
    runs: &[CaptureRun],
    marks: Vec<(u64, TranscriptElem)>,
    base: u64,
    frozen_through: Option<usize>,
) -> Vec<TranscriptElem> {
    let chars: Vec<char> = text.chars().collect();
    let total = chars.len();
    // Clamp into this turn's text, then snap to the owning line's start — for a
    // PICTURE, which anchors to the paragraph it was drawn beside, so a drop-cap
    // stamped mid-word still leads its paragraph.
    //
    // A `ScreenClear` splits EXACTLY where it was stamped instead (SQ-0751). The
    // boundary is not a position within a line, it is a line break: `erase_window`
    // homes the cursor to the top-left (ZMSD §8.7.3.2.1), so whatever the game prints
    // next begins a new line by definition. Snapping it back would put the text that
    // preceded the erase BELOW the boundary — which is the whole defect, seen from
    // one line's distance. Every clear in the corpus falls on a line start anyway,
    // where the two rules agree.
    let mut inserts: Vec<(usize, TranscriptElem)> = marks
        .into_iter()
        .map(|(abs, elem)| {
            let mut off = (abs.saturating_sub(base) as usize).min(total);
            if !matches!(elem, TranscriptElem::ScreenClear) {
                while off > 0 && chars[off - 1] != '\n' {
                    off -= 1;
                }
            }
            (off, elem)
        })
        .collect();
    inserts.sort_by_key(|(o, _)| *o); // stable: equal offsets keep draw order

    // Lockstep style-chunk consumption: `take(n)` returns the chunks covering
    // the next `n` chars, splitting the boundary chunk as needed.
    let mut run_iter = runs.iter().copied();
    let mut pending: Option<CaptureRun> = run_iter.next();
    let mut take = |n: usize| -> Vec<CaptureRun> {
        let mut out = Vec::new();
        let mut left = n;
        while left > 0 {
            match pending {
                Some(mut r) => {
                    if r.0 <= left {
                        left -= r.0;
                        out.push(r);
                        pending = run_iter.next();
                    } else {
                        let mut head = r;
                        head.0 = left;
                        out.push(head);
                        r.0 -= left;
                        left = 0;
                        pending = Some(r);
                    }
                }
                None => break,
            }
        }
        out
    };

    let mut elems = Vec::new();
    let mut pos = 0usize;
    for (off, img) in inserts {
        if off > pos {
            // Text up to the split, excluding the '\n' the split lands after —
            // the element boundary IS the line break.
            let end = off - 1; // chars[off-1] == '\n' (or off == total edge below)
            let (chunk_end, drop_sep) = if chars[off - 1] == '\n' { (end, true) } else { (off, false) };
            if chunk_end > pos {
                let chunk: String = chars[pos..chunk_end].iter().collect();
                let chunk_runs = take(chunk_end - pos);
                // …unless the retirement below froze it (SQ-0890): this text is
                // PAINT on the screen now, published as its own layer, and a
                // transcript copy of it is a second rendition of pixels that are
                // already there. The style chunks are consumed either way, so the
                // runs stay in lockstep with the text that survives.
                if frozen_through.is_none_or(|f| chunk_end > f) {
                    elems.push(TranscriptElem::Text { text: chunk, runs: chunk_runs });
                }
            }
            if drop_sep {
                let _ = take(1); // consume the dropped separator's style char
            }
            pos = off;
        }
        elems.push(img);
    }
    if pos < total {
        let tail: String = chars[pos..].iter().collect();
        let tail_runs = take(total - pos);
        elems.push(TranscriptElem::Text { text: tail, runs: tail_runs });
    }
    elems
}

// ── Public types ──────────────────────────────────────────────────────────────

/// One ordered piece of a turn's buffer output: a text run (with its style
/// chunks) or an inline image. Preserves emission order so images land between
/// the right lines.
pub enum TranscriptElem {
    Text { text: String, runs: Vec<CaptureRun> },
    Image(crate::inline_image::InlineImage),
    /// A screen-clear boundary *inside* the turn's output (SQ-0697): a v6
    /// wrap+scroll window moved out from under the prose printed before this
    /// point, and the engine froze that prose where it was painted. Everything
    /// above stays in scrollback; the live screen begins here, at the window's
    /// new origin. Carries no text, so it never shifts an offset.
    ScreenClear,
}

/// Trim trailing `Text` elements of `elems` so the total char-count of their
/// text equals `keep` — the element-list counterpart to `strip_read_prompt`
/// shortening the flat text by a trailing read prompt. Walks from the end,
/// clearing whole `Text` elements and truncating the boundary one (its `text`
/// AND its `runs`, via `clamp_runs`) so the concatenation of element text stays
/// exactly equal to the stripped flat `raw`. `Image` elements carry no text, so
/// a strip that reaches across one still lands on the preceding text.
pub(crate) fn trim_elems_to_len(elems: &mut [TranscriptElem], keep: usize) {
    let total: usize = elems
        .iter()
        .map(|e| match e {
            TranscriptElem::Text { text, .. } => text.chars().count(),
            TranscriptElem::Image(_) | TranscriptElem::ScreenClear => 0,
        })
        .sum();
    if total <= keep {
        return;
    }
    let mut remove = total - keep;
    for e in elems.iter_mut().rev() {
        if remove == 0 {
            break;
        }
        if let TranscriptElem::Text { text, runs } = e {
            let n = text.chars().count();
            if n <= remove {
                remove -= n;
                text.clear();
                runs.clear();
            } else {
                let keep_here = n - remove;
                let byte = text
                    .char_indices()
                    .nth(keep_here)
                    .map(|(i, _)| i)
                    .unwrap_or(text.len());
                text.truncate(byte);
                *runs = clamp_runs(std::mem::take(runs), keep_here);
                remove = 0;
            }
        }
    }
}

/// One buffered Glk sound-channel operation, emitted by `AppGlk` during a turn
/// and drained into `TurnResult.glulx_sound_ops` for `AppState` to play. Channel
/// *state* (refs, rocks, volume) lives in `AppGlk`; only the playback-affecting
/// operations travel here. `Play.volume` snapshots the channel's current Glk
/// volume so the player (which cannot see `AppGlk`) can compute gain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchannelOp {
    /// `paused` snapshots the channel's pause state (Glk 0.7.3 §8.3): a sound
    /// played on a channel paused while empty must start paused, and release
    /// only on `unpause`.
    Play { chan: u32, snd: u32, repeats: u32, notify: u32, volume: u32, paused: bool },
    Stop { chan: u32 },
    SetVolume { chan: u32, vol: u32 },
    Destroy { chan: u32 },
    /// Glk 0.7.3 Sound2 `glk_schannel_pause` — pause playback, keeping position.
    Pause { chan: u32 },
    /// Glk 0.7.3 Sound2 `glk_schannel_unpause` — resume a paused channel.
    Unpause { chan: u32 },
    /// Glk 0.7.3 Sound2 `glk_schannel_set_volume_ext` — a volume change ramped
    /// over `duration_ms` (0 = immediate). When `notify != 0` the host fires an
    /// `evtype_VolumeNotify(val2 = notify)` once the ramp completes.
    SetVolumeExt { chan: u32, vol: u32, duration_ms: u32, notify: u32 },
}

/// Result of one player turn.
#[derive(Default)]
pub struct TurnResult {
    pub transcript: String,
    /// Text-style chunks for `transcript`: a `(char_count, bits, fg, bg, link, para)`
    /// list covering every char of `transcript`, fed to `push_transcript_runs`. All
    /// chunks carry bits 0, default colours and default `para` when the turn emitted
    /// no styling (the Z-machine path never sets a non-default `para`).
    pub transcript_runs: Vec<CaptureRun>,
    pub location: Option<ObjectSnapshot>,
    pub quit: bool,
    /// The game cleared the screen this turn — a Z-machine `erase_window`
    /// (lower / all, ZMSD §8.7.3) or a Glulx `glk_window_clear` on the primary
    /// buffer (e.g. a help-menu takeover / Inform 7 menu redraw). The host pins
    /// this turn's output to a fresh screen (scrollback preserved) so stale text
    /// does not bleed through — matching a retained-mode interpreter like Lectrote.
    pub erase_lower: bool,
    /// Optional one-line note to surface to the player (general-purpose; currently unused — no producer sets it).
    pub info: Option<String>,
    /// Sound events emitted this turn (drained from the VM), in order.
    pub sounds: Vec<SoundEvent>,
    /// Glk sound-channel operations emitted this turn (Glulx only; empty for the
    /// Z-machine, which uses `sounds`). Played by `AppState::play_glulx_sound_ops`.
    pub glulx_sound_ops: Vec<SchannelOp>,
    /// Host-facing diagnostic lines emitted this turn (drained from the VM).
    pub diagnostics: Vec<String>,
    /// How the current room was detected this turn (drives the map indicator).
    pub location_method: Option<LocationMethod>,
    /// Set when the game's own `@save`/`@restore` (any version) suspends the VM for host-mediated file I/O; `None` otherwise.
    pub pending_io: Option<PendingIo>,
    /// Set when this turn came from `abort_timed_input` (the pending read was
    /// completed as timed-out, either directly or because `run_timed_interrupt`'s
    /// routine aborted the read). `false` for every other turn.
    pub timed_out: bool,
    /// Pre-formatted crash stack-trace lines when the VM faulted this turn.
    pub fault: Option<Vec<String>>,
    /// v6 `draw_picture`/`erase_picture` events emitted this turn (drained from
    /// the VM), in order. Empty for v1–5/v7/v8 and for the Glulx path (which
    /// composites its own graphics windows). `GameSession::drain_turn` also
    /// applies each event to `GameSession::pictures_canvas` as it drains them
    /// (mirrors `sounds`, but the Z-machine path additionally rasterizes here
    /// rather than leaving that to the app layer, since a v6 window's canvas
    /// must be self-contained on the session for the Task 4 screen adapter).
    pub pictures: Vec<PictureEvent>,
    /// Ordered buffer output for this turn (text runs + inline images). Empty
    /// for the Z-machine path (no images); the Glulx path fills it and the run
    /// loop pushes from it. When empty, the loop falls back to `transcript` +
    /// `transcript_runs`.
    pub transcript_elems: Vec<TranscriptElem>,
    /// Where in `transcript` a v6 wrap+scroll window was moved or resized out
    /// from under the prose it had already printed, so the engine froze that
    /// prose where it was painted (SQ-0697) — a char offset, `None` when nothing
    /// was retired.
    ///
    /// Everything BEFORE the offset is now paint on the screen at its old
    /// coordinates, so the host must not go on streaming it too: it pushes that
    /// head as scrollback, marks a screen-clear boundary, and starts the live
    /// screen at the offset. Scrollback is preserved — `mark_screen_clear`, never
    /// the SQ-0407 truncate, which would eat the session's history along with it.
    /// Always `None` for non-v6 Z-machine stories, Glulx and Scott.
    pub prose_retired: Option<usize>,
}

impl TurnResult {
    /// A result that reports nothing but WHERE THE PLAYER IS: the mapper seed a
    /// host restore / resume-to-a-past-turn feeds to [`apply_turn`]. Nothing
    /// executed, so there is nothing else to report.
    ///
    /// Every other field is the type's own default rather than a hand-written
    /// `false` / `Vec::new()`. Both call sites used to spell all sixteen out, which
    /// is the shape that let the BOOT's seed quietly claim `erase_lower: false`
    /// about a boot that had erased the screen (SQ-1106) — a hand-filled literal
    /// answers a question it was never asked.
    pub fn observation(location: ObjectSnapshot) -> TurnResult {
        TurnResult { location: Some(location), ..TurnResult::default() }
    }
}

/// One `erase_window`'s background fill: the screen rect it painted (0-based pixels,
/// as the composite uses), the window background it painted with as a packed colour
/// (0 = the window inherited the page default), and the draw-order stamp it took from
/// the shared picture sequence so it layers with picture draws (SQ-0584).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowFill {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub bg: u32,
    pub seq: u64,
    /// Story chars printed when the erase happened (`PictureEvent::out_chars`). Prose
    /// is paint too: one character streamed to the story window after this erase means
    /// the fill is no longer the newest thing on the screen, so it stops covering.
    /// Zork Zero is the case that needs it — it erases its full-screen decorative
    /// window 7 to white during BOOT, before a word of the story has printed.
    pub out_chars: u64,
}

/// Where a window's picture canvas was painted, so a later `move_window` can be
/// told from a redraw in place (SQ-0715). See [`GameSession::canvas_anchor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CanvasAnchor {
    /// The window's 1-based screen origin `(x, y)` when the content was drawn.
    origin: (u16, u16),
    /// Union of the draw footprints in canvas coords, `(x, y, w, h)` — the only
    /// part worth carrying off a canvas that may be 1000×1000 of transparency.
    rect: (u32, u32, u32, u32),
}

/// One step of a turn's v6 picture sequence: the screen as it stood after that
/// step, and how long it is held before the next one lands (SQ-0708).
///
/// Cloning the canvas map is a refcount bump per window ([`crate::graphics::Canvas`]
/// keeps its pixels in an `Arc` and paints through `Arc::make_mut`), so a frame
/// costs a real copy only for the windows the sequence goes on to repaint.
struct PacedFrame {
    canvas: std::collections::HashMap<u8, crate::graphics::Canvas>,
    hold: std::time::Duration,
}

/// Notional fill rate for a v6 picture blit, in unit-space pixels per
/// millisecond — the constant that turns "how big is this picture" into "how
/// long did it take to paint" (SQ-0708). Arthur's 584×392 plate lands at ~286 ms,
/// a 64×64 tile at the floor below.
const PACE_PX_PER_MS: u64 = 800;
/// Floor on a paced frame's hold: below this a step is a flicker, not a beat.
const PACE_MIN_MS: u64 = 40;
/// Ceiling on a paced frame's hold, so even a full-screen plate cannot make the
/// game feel stalled.
const PACE_MAX_MS: u64 = 350;
/// Intermediate frames one turn may pace through. A turn that repaints a whole
/// chrome ring queues a dozen small draws; past this many the rest collapse into
/// the settled composite rather than turning one command into a slideshow. Also
/// bounds the snapshots a single turn can hold.
const PACE_MAX_FRAMES: usize = 8;

/// A running Z-machine game session.
pub struct GameSession {
    pub machine: Machine,
    pub quit: bool,
    /// Which kind of input the VM is currently waiting for.
    pending: InputKind,
    /// When false, the game's own trailing `>` read prompt is kept in the
    /// transcript instead of being stripped. Default true. See
    /// [`Engine::set_strip_prompt`].
    strip_prompt: bool,
    /// Where the prose window's cursor sat when the last keypress was supplied
    /// (SQ-0804), armed by [`GameSession::submit_char`] and consumed by
    /// `drain_turn`. `None` for every other kind of turn, and below v6.
    pen_before_char: Option<(u16, u16)>,
    /// Whether the turn just drained began printing exactly where the previous
    /// output left the cursor — see [`Engine::output_continued_line`].
    output_continued: bool,
    /// Lazily-built, memoized disassembly cache (routine-discovery boundaries).
    /// `RefCell` because the Debugger read-path is `&self`; consistent with the
    /// existing `mem_fault` interior-mutability pattern.
    disasm_cache: std::cell::RefCell<Option<zvm::cpu::disasm_cache::DisasmCache>>,
    /// Object-table conventions this story uses — which attribute means "open",
    /// which property lists a room's shared scenery (SQ-0678). Recovered from
    /// the story's own table, which is why it is inferred once and kept: the
    /// numbers describe the compiler's layout and never change. The live game
    /// state is read *through* it on every query, so a container the game opens
    /// mid-turn shows its contents on the next refresh with no rebuild.
    ///
    /// `OnceCell` rather than a constructor field: `GameSession` is built at
    /// nine sites (boot, restore, reset, …) and the model is identical at all
    /// of them, so it is derived on first use instead of nine times over.
    world: std::cell::OnceCell<zvm::world::WorldModel>,
    /// Where this story keeps the words its parser accepts for an object, and
    /// the dictionary those words are read through (SQ-1118). `None` inside the
    /// cell is a real answer, not a failure to look: Journey has no parser at
    /// all, and `advent.z8` implements its own over a table of its own, leaving
    /// the Z-machine dictionary empty — both then answer with printed names and
    /// no words, which is all there is to say about them.
    ///
    /// Cached beside [`world`](Self::world) and for the same reason: `detect`
    /// tallies every property of every object once, and the answer describes the
    /// compiler's layout, which no turn can change.
    parse_names: std::cell::OnceCell<Option<zvm::objects::ParseNames>>,
    /// The [`parse_names`](Self::parse_names) walk folded into the one set the
    /// bulk callers query — "does ANY object answer to this word" (SQ-1176).
    ///
    /// A cache, not a `OnceCell`, because unlike its neighbours it holds LIVE
    /// data: the words sit in dynamic memory and a game can rewrite them, so
    /// the entry is dropped whenever the VM runs ([`drain_turn`] is the funnel
    /// every VM-stepping path drains through, and `restore_state` swaps memory
    /// without stepping). Within a turn the screen is fixed, so one build
    /// serves every reveal press and the seen-words sweep alike.
    ///
    /// [`drain_turn`]: GameSession::drain_turn
    object_word_set: std::cell::RefCell<Option<std::sync::Arc<grammar_model::ObjectWordSet>>>,
    /// The built v6 [`ScreenModel`], memoized against everything its build reads
    /// (SQ-1191) — see [`V6ModelKey`]. `screen_now` runs once per FRAME, and the
    /// build deep-clones every window's runs, grids and prose; between turns
    /// nothing on the screen moves, so a redraw (cursor blink, mouse move, map
    /// pan) costs one key compare instead of a clone tree.
    ///
    /// Same shape and soundness argument as [`object_word_set`](Self::object_word_set)
    /// above: live data behind a `RefCell`, dropped wherever its inputs are
    /// swapped out from under the key — [`restore_screen`] installs a screen
    /// whose fresh [`zvm::screen::ScreenState::v6_generation`] could collide
    /// with a number the memo already holds, and `restore_state` /
    /// `restore_game_save` swap memory without draining a turn. A `@restart`
    /// needs no drop: zvm keeps the generation monotone across the reboot.
    v6_model_memo: std::cell::RefCell<Option<(V6ModelKey, std::sync::Arc<ScreenModel>)>>,
    /// PC at which the disasm cache was last runtime-confirmed; the per-turn
    /// fold is skipped while the VM is parked at the same PC (nav/scroll calls).
    last_confirmed_pc: std::cell::Cell<Option<u32>>,
    /// v6 Pict resolver (self-blorb/sidecar), set via [`set_pict_source`]
    /// (`None` for non-v6 stories, or when set before construction hasn't
    /// happened yet). Kept on the session — rather than only on `AppState` —
    /// so `drain_turn` can rasterize `pending_pictures` into `pictures_canvas`
    /// without the app layer reaching in. (Plan 1b Task 2)
    ///
    /// [`set_pict_source`]: GameSession::set_pict_source
    pict_source: Option<crate::graphics::PictSource>,
    /// Per-v6-window pixel canvas, keyed by window number (1–7; window 0 is
    /// the main text window and never gets a canvas). Populated by
    /// `drain_turn` from `Machine::pending_pictures`; read by the Task 4
    /// screen adapter to build the layered composite.
    pub pictures_canvas: std::collections::HashMap<u8, crate::graphics::Canvas>,
    /// Where each window's canvas content was PAINTED — the window's screen
    /// origin at draw time, plus the footprint the draws actually covered
    /// (SQ-0715). Only windows that have received a real `draw_picture` appear.
    ///
    /// ZMSD §8: "subsequent movements of the window do not move what was
    /// printed". A window canvas is drawn at the window's *current* origin, so it
    /// only tells the truth while the window stays put. scopa uses window 3 as a
    /// scratch pad — it moves it, sizes it to 1000×1000, draws one card picture
    /// at (1,1) and immediately moves it somewhere else for the next — so the
    /// canvas is stranded the instant the next `move_window` lands. When the
    /// origin no longer matches, the pixels are taken off the canvas and painted
    /// onto the screen's [`paint`](GameSession::paint) ground where they were
    /// drawn, which is exactly where they stayed on the hardware.
    canvas_anchor: std::collections::HashMap<u8, CanvasAnchor>,
    /// The per-axis factor this story's pictures are scaled by on their way to
    /// the screen — `(2, 2)` for art authored against a standard window, `(1, 1)`
    /// for art that declares none (SQ-0715), `(1, 2)` for an EGA/CGA rendition
    /// (SQ-0790).
    ///
    /// Blorb §11 (Resolution chunk): "This chunk is optional; if it is not
    /// present, then all of the images in this file are non-scalable", and
    /// "non-scalable images are always displayed at their actual size. (One image
    /// pixel per screen pixel.)" Every Infocom v6 blorb carries a `Reso` declaring
    /// a 320×200 standard window, which we present at 640×400 — so their art
    /// doubles, exactly as Frotz's Amiga/DOS profile does. scopa.blb carries no
    /// `Reso` at all: its cards are drawn for the 640×400 screen already (its own
    /// hardwired vector deck is the same 52×84), and doubling them made the
    /// Neapolitan and Sicilian decks twice the size the game had laid out for,
    /// overlapping each other and hanging off the bottom of the screen.
    ///
    /// SQ-0790 made it a PAIR. A native EGA/CGA archive stores the same artwork
    /// in a 640-wide picture space with pixels half as wide, so it reaches the
    /// same 640×400 unit screen at (1, 2) — see
    /// [`crate::graphics::PictSource::art_scale`], which is where the factor
    /// comes from.
    art_scale: (u32, u32),
    /// The v6 screen's PAINTED ground: filled rectangles an `erase_window` left
    /// behind, accumulated in native pixels (SQ-0706).
    ///
    /// ZMSD §8.7.3.3 makes erasing a window a fill with its background, and on a
    /// v6 screen that is a drawing operation. scopa.z6 draws every playing card
    /// with nothing else — `fastsimplebox` resizes one window, moves it, colours
    /// it and erases it, hundreds of times per card — so a host that treats each
    /// erase as "drop this window's canvas" renders no cards at all.
    ///
    /// A surface rather than a list of rects because the list is unbounded: a card
    /// table repaints continuously, and the pixels are what matter, not the
    /// history. Only a fill naming a colour OUTRIGHT paints (see
    /// [`crate::render::v6_layout::explicit_pixel_rgba`]) — "current"/"default"
    /// mean inherit, which is the host's business, and skipping them keeps games
    /// that merely clear their windows (Arthur's intro erases all eight) from
    /// gaining a backdrop they never asked for.
    ///
    /// `None` until a game actually paints one, so nothing changes for the games
    /// that never do.
    paint: Option<std::sync::Arc<image::RgbaImage>>,
    /// The screens this turn's picture sequence passed through on its way to
    /// `pictures_canvas`, oldest first — empty whenever nothing is playing (SQ-0708).
    ///
    /// A v6 turn can queue several `draw_picture`s: Arthur's intro paints the
    /// graveyard plate and then Merlin fourteen instructions later, in ONE turn.
    /// Compositing them all before anything renders shows the player the finished
    /// screen instantly; real hardware blitted each picture as its opcode executed,
    /// so you watched the graveyard paint and then Merlin paint onto it. There is no
    /// Z-machine construct expressing that — no busy-wait, no intervening read — so
    /// this is a presentation choice, not a standard being implemented.
    ///
    /// It is replayed AFTER the fact. The turn still runs straight through and
    /// `pictures_canvas` is settled before the `TurnResult` is published, so the story
    /// interpreter never blocks and never yields mid-turn; these are snapshots the
    /// renderer shows on the way there, and the last thing it shows is the settled
    /// composite itself, byte for byte. Dropping them (a keypress, a resize, a save)
    /// therefore cannot lose anything — it only arrives sooner.
    paced_frames: std::collections::VecDeque<PacedFrame>,
    /// The last `erase_window` on each v6 window: what it filled and when (SQ-0584).
    ///
    /// ZMSD §8.8.5.3 — erasing a window fills its rect with that window's background
    /// colour. On a real interpreter every v6 window is a clipping region over ONE
    /// screen bitmap, so that fill is opaque paint: it is what makes a menu panel hide
    /// the story behind it (advent.z6's `help` splits window 1 to 160px, erases it,
    /// then paints the menu into it). lanthorn composites layers instead, and an
    /// erased window used to become simply transparent — the panel's text floated over
    /// the story with nothing behind it. Recorded here at drain time, on the same
    /// ordered queue as picture draws, so the fill composites in sequence with them.
    pub window_fills: std::collections::HashMap<u8, WindowFill>,
    /// v6 window-0 inline pictures (drop-caps, room icons) awaiting transcript
    /// interleaving: the absolute win0 output-char offset each was drawn at
    /// (`PictureEvent::out_chars`), plus the prepared float image. Drained into
    /// ordered `TranscriptElem`s so each picture anchors to its paragraph.
    story_pics: Vec<(u64, crate::inline_image::InlineImage)>,
    /// `Machine::v6_win0_out_chars` at the last transcript drain — an event's
    /// offset within the current turn's text is `out_chars - this`.
    v6_win0_chars_seen: u64,
    /// Ordered display list per window canvas: every picture drawn and every region
    /// erased, in the order it happened.
    ///
    /// This exists to make a palette change behave like real hardware. A v6 screen is
    /// a framebuffer of palette INDICES, so loading a new palette recolours
    /// everything already on it, in place, without disturbing what covers what.
    /// lanthorn bakes RGBA at draw time, so the only faithful way to recolour is to
    /// replay the window from scratch under the new palette — which needs the erases
    /// as well as the draws, and needs them in order. (SQ-0567)
    ///
    /// Arthur is the story that needs it twice over: its frame is three adaptive
    /// pictures drawn once at boot (so without a replay it keeps the churchyard's
    /// palette all game), and its map screen draws a full-screen background OVER that
    /// frame (so a replay that ignores order paints the frame back on top and hides
    /// the map).
    display_ops: std::collections::HashMap<u8, Vec<V6Op>>,
    /// Windows whose display list overflowed [`V6_OPS_CAP`]. Replay is skipped for
    /// them rather than replayed from a truncated list, which would invent a screen
    /// that never existed. They keep the pre-SQ-0567 behaviour: stale palette, right
    /// layering.
    unreplayable: std::collections::HashSet<u8>,
    /// The width (in columns) the story image now in memory was laid out for —
    /// the boot width of the session that produced it.
    ///
    /// At construction that is THIS session's boot: the `host_screen` column
    /// count seeded into `new_with_trace` (SQ-0680), or
    /// [`zvm::screen::DEFAULT_SCREEN_COLS`] when boot was unseeded (`None`, or
    /// a construction path that bypasses `new_with_trace` entirely).
    ///
    /// [`declared_story_screen_dims`]'s floor used to assume every v4+ story
    /// booted at the fixed 80-column default; now that boot itself can be
    /// seeded with the real pane, the floor has to track whatever THIS session
    /// actually booted at instead, or a pane narrower than 80 would have its
    /// correct pre-boot seed silently overwritten back to 80 on the very next
    /// poll (`loop_tick::poll_zvm_screen_dims`).
    ///
    /// A RESTORE swaps that image out for one some other session booted, and
    /// with it the baked-in status-bar columns — so every restore raises this
    /// through [`note_restored_screen_cols`](Self::note_restored_screen_cols)
    /// to the restored game's own frame of reference (SQ-0681). It only ever
    /// grows: a session whose own boot was wider keeps its width, because the
    /// header it booted with is still the wider one.
    ///
    /// [`declared_story_screen_dims`]: crate::render::screen::declared_story_screen_dims
    pub boot_screen_cols: u16,
}

// ── GameSession impl ──────────────────────────────────────────────────────────

impl GameSession {
    /// Build a new session from raw story bytes.
    ///
    /// Constructs a `Machine` with a `CaptureSink`, calls `init_caps`, then
    /// steps until the first `NeedLine`/`NeedChar`/`Quit` — this drives the
    /// game's opening text into the sink.  The sink is NOT drained here; the
    /// caller can call `take_transcript` to retrieve the banner/intro text.
    pub fn new(story: Vec<u8>, honor_game_colours: bool, sound_available: bool, interpreter_number: Option<u8>) -> Result<GameSession, ZError> {
        Self::new_with_trace(story, honor_game_colours, sound_available, interpreter_number, false, Vec::new(), None, None, None)
    }

    /// Like [`new`](Self::new) but enables execution tracing BEFORE the VM runs to
    /// its first input prompt, so the boot/initialisation code — the whole reason
    /// `--debug` exists (a mid-game `/debug` can never see it) — is captured into
    /// the cumulative coverage set. (SQ-0449)
    ///
    /// `picture_dims` is the v6 Pict dimension table (`(number, width, height)`),
    /// resolved app-side from a self-blorb/sidecar Blorb — empty for non-v6
    /// stories. It MUST be injected before the boot run below: `picture_data` is
    /// called during boot, which happens inside this very function (Phase 0
    /// boot-tracing lesson), so `set_picture_dims` runs right after
    /// `set_sound_available`, before `init_caps()`/the boot loop.
    ///
    /// `default_colours` is the host's own `(background, foreground)` §8.3.1
    /// standard colour pair for header bytes $2C/$2D (ZMSD §8.3.3). It is applied
    /// BEFORE `init_caps()` for the same reason as `picture_dims`: the game's
    /// initialisation runs inside this function, and a game that reads the header
    /// default pair while booting (Beyond Zork picks its colour scheme there)
    /// must already see the host's real page/ink. `None` leaves the VM's own
    /// §8.3.2 black-on-white seed. (SQ-0532/A-F2)
    ///
    /// `host_screen` is the real `(rows, cols)` of the story pane the caller is
    /// about to render into, measured BEFORE this call — not the pane-measured
    /// dims applied a frame after boot, which is one turn too late for a v4/v5
    /// story whose status-bar routine lays itself out ONCE, at boot, and never
    /// re-reads header byte $21 (SQ-0679/SQ-0680). Seeding the real pane here
    /// means that boot-time layout already targets it, so the SQ-0679 floor
    /// (which never lets a v4+ declared width shrink below the BOOT width) now
    /// only ever guards a genuine mid-session narrowing, instead of permanently
    /// pinning every story to the zvm 80×24 fallback it would otherwise have
    /// booted at. `None` (non-TUI callers, most tests) leaves that 80×24 fallback
    /// in place, matching prior behaviour exactly. Ignored for v6, whose screen
    /// is the native pixel frame seeded from `v6_screen_px` above, never the host
    /// cell pane.
    ///
    /// The art reaches that unit screen at the uniform [`V6_ART_SCALE`]. A
    /// launch that resolved a native picture archive may know better — see
    /// [`Self::new_with_art_scale`] — but every caller of *this* function gets
    /// the rule exactly as it has always been.
    pub fn new_with_trace(story: Vec<u8>, honor_game_colours: bool, sound_available: bool, interpreter_number: Option<u8>, trace_from_boot: bool, picture_dims: Vec<(u16, u16, u16)>, v6_screen_px: Option<(u16, u16)>, default_colours: Option<(u8, u8)>, host_screen: Option<(u16, u16)>) -> Result<GameSession, ZError> {
        Self::new_with_art_scale(story, honor_game_colours, sound_available, interpreter_number, trace_from_boot, picture_dims, v6_screen_px, None, default_colours, host_screen, None, None)
    }

    /// [`Self::new_with_trace`] with the art scale supplied rather than assumed
    /// (SQ-0790).
    ///
    /// `v6_art_scale` is [`crate::graphics::PictSource::art_scale`]: the per-axis
    /// factor the art is blown up by on its way onto the 640×400 unit screen,
    /// when the source has an opinion. `None` — every Blorb-sourced story, and
    /// every non-v6 one — keeps the uniform [`V6_ART_SCALE`] rule. Only a NATIVE
    /// archive answers, and only an EGA/CGA one answers with anything other than
    /// `(2, 2)`, so the two entry points are the same function for the whole
    /// corpus.
    ///
    /// `random_seed` is the value the story's `random` opcode starts from
    /// (SQ-0811). It is applied here, before the boot run below, because a game's
    /// initialisation routine may already draw from the generator — seeding after
    /// the first prompt is one turn too late to change the game the player is
    /// handed. `None` — every caller but the launcher — leaves zvm's own fixed
    /// default, so a test's sequence stays the reproducible one it has always been.
    /// Boot a story on a machine, told what that machine is in ONE argument
    /// (SQ-1022).
    ///
    /// [`crate::machine_boot::MachineBoot`] carries the five per-machine facts —
    /// interpreter number, screen, art scale, default colours, character cell —
    /// so a caller cannot pass four of them. That is the whole point: every one of
    /// those facts has been silently omitted by a real caller at least once, and
    /// the failure is always the same shape, a screen that is entirely
    /// self-consistent and that the player never sees (SQ-0901, SQ-1020, SQ-1021).
    ///
    /// Prefer this over [`Self::new_with_art_scale`] anywhere a medium is
    /// involved. `new_with_trace` remains right for a bare story with no machine
    /// behind it — and [`crate::machine_boot::MachineBoot::bare`] says so
    /// explicitly where a caller wants to be plain about it.
    pub fn new_for_machine(
        story: Vec<u8>,
        honor_game_colours: bool,
        sound_available: bool,
        trace_from_boot: bool,
        picture_dims: Vec<(u16, u16, u16)>,
        host_screen: Option<(u16, u16)>,
        random_seed: Option<u32>,
        boot: &crate::machine_boot::MachineBoot,
    ) -> Result<GameSession, ZError> {
        let mut s = Self::new_with_art_scale(
            story,
            honor_game_colours,
            sound_available,
            boot.interpreter_number,
            trace_from_boot,
            picture_dims,
            boot.screen_px,
            boot.art_scale,
            boot.default_colours,
            host_screen,
            random_seed,
            Some(boot.text_face()),
        )?;
        // SQ-1071. Set here rather than threaded through the private constructor
        // above, whose positional machine facts are the shape SQ-1021 closed the
        // door on. `new_with_trace` — the honest no-machine door — leaves zvm's
        // own default, §8.8.3.1.1 as written, which is what a story file with no
        // medium to name a machine should get.
        s.machine.v6_wrap_regime = boot.wrap_regime;
        // SQ-1154, and set here for the same reason. This is the fourth term of
        // `zvm::screen::machine_rule` — whether this launch presents its machine's
        // per-machine SCREEN RULES at all — and it is the only one not read back
        // out of the header, because it is LAUNCH policy (`--colour`) that no story
        // can reach.
        //
        // It lives on the `Machine` rather than on `ScreenState` deliberately, and
        // that is not a stylistic choice: `Machine::restart` rebuilds a fresh
        // `ScreenState` for `@restart`, and `session::restore_screen` assigns a
        // whole one over the live machine for a host Save State. A licence held
        // there would be silently reset to `ScreenState::default()`'s by BOTH —
        // `restore_screen`'s own `..Default::default()` in `archive::ScreenDto` is
        // exactly that hole. On the `Machine` all three survivals are free: an
        // `@restart` re-boots through `reset.rs`'s `MachineBoot::resolve` (which the
        // compiler forces to re-ask), a Quetzal `@restore` touches memory and not
        // screen state, and a host Save State keeps the licence THIS run was
        // launched with — which is right, because `--colour` is a flag of this run
        // and not a property of the saved game.
        s.machine.machine_colours_licensed = boot.machine_colours_licensed;
        Ok(s)
    }

    /// **Private since SQ-1021.** Every machine fact as a separate positional
    /// argument is the shape this codebase kept getting wrong — four callers
    /// omitted one, including `reset.rs` in production — so the only reachable
    /// doors are [`Self::new_for_machine`], which takes them as one value, and
    /// [`Self::new_with_trace`], which is the honest no-machine case. This is a
    /// compile error rather than a convention, which is the point.
    fn new_with_art_scale(story: Vec<u8>, honor_game_colours: bool, sound_available: bool, interpreter_number: Option<u8>, trace_from_boot: bool, picture_dims: Vec<(u16, u16, u16)>, v6_screen_px: Option<(u16, u16)>, v6_art_scale: Option<(u32, u32)>, default_colours: Option<(u8, u8)>, host_screen: Option<(u16, u16)>, random_seed: Option<u32>, v6_text: Option<crate::native_font::TextFace>) -> Result<GameSession, ZError> {
        let mem = Memory::new(story)?;
        let sink = Box::new(CaptureSink::new());
        let mut machine = Machine::with_output(mem, sink);
        machine.set_honor_game_colours(honor_game_colours);
        machine.set_sound_available(sound_available);
        if let Some(seed) = random_seed {
            machine.set_rng_seed(seed);
        }
        if let Some((bg, fg)) = default_colours {
            machine.set_default_colours(bg, fg);
        }
        // SQ-0917: the machine's Version 6 cell, BEFORE the screen is sized and
        // before the boot run — the story reads `$26`/`$27` and lays its windows
        // out from them, so a cell that arrives later is one the game has already
        // disagreed with. `None` keeps zvm's 8x16 default, which is every profile
        // but the Macintosh.
        //
        // SQ-1009: the PEN travels with it, because on a machine that drew
        // proportionally the two are one fact — see
        // [`crate::native_font::TextFace::metric`]. The engine and the renderer
        // then measure through the same table rather than through two copies of
        // one rule.
        if machine.mem.version() == 6 {
            if let Some(text) = v6_text.as_ref() {
                machine.set_v6_text(text.metric().clone());
            }
        }
        // v6 (SQ-0479): the game lays out on the 640×400 UNIT screen, so
        // `picture_data` must report the doubled (unit-space) picture sizes —
        // Frotz's Amiga/DOS interpreter returns `scaler * size` for every pic.
        // PictSource keeps the raw art-native dims; only the game-facing table
        // is scaled here (one crossing into unit space).
        //
        // …but ONLY for art that declares a standard window to be scaled against
        // (SQ-0715). Blorb §11: a resource file with no `Reso` chunk has no
        // scalable images at all, and non-scalable images are shown at their
        // actual size, one image pixel per screen pixel. `v6_screen_px` IS that
        // chunk's standard window, so its absence is the spec's own signal.
        //
        // SQ-0790: per axis, because an EGA/CGA archive's pixels are half as
        // wide. The source supplies the pair when it knows one; absent that the
        // uniform rule stands, which is every path that existed before.
        let art_scale = if machine.mem.version() == 6 && v6_screen_px.is_some() {
            v6_art_scale.unwrap_or((V6_ART_SCALE, V6_ART_SCALE))
        } else {
            (1, 1)
        };
        let picture_dims = if machine.mem.version() == 6 {
            picture_dims
                .into_iter()
                .map(|(n, w, h)| (n, w * art_scale.0 as u16, h * art_scale.1 as u16))
                .collect()
        } else {
            picture_dims
        };
        machine.set_picture_dims(picture_dims);
        machine.set_interpreter_number(interpreter_number);
        machine.init_caps();
        // v6 (SQ-0479): present the reference-authentic UNIT screen — the Blorb
        // `Reso` standard window (the ART resolution, default 320×200) at the
        // scale the machine drew it, which for the whole corpus but one is the
        // ×2 of Frotz's Amiga/DOS profile (640×400, 8×16 cell → 80×25). The
        // screen and the picture dims (above) scale together, so the game's
        // window/art layout math and our `is_content_art` ratios stay
        // consistent. init_caps seeded the v1–5 default; this overrides it for
        // v6 only, before the game can read it.
        if machine.mem.version() == 6 {
            let (art_w, art_h) = v6_screen_px.unwrap_or((320, 200));
            // SQ-0838: the screen is the art's picture space AT THE SCALE THIS
            // MACHINE DREW IT, which is one statement covering what used to be
            // a fixed doubling. For every rendition that existed before it is
            // the same arithmetic by another name — 320×200 at (2,2) and EGA's
            // 640×200 at (1,2) are both 640×400 — and the difference it buys is
            // the standard Macintosh, whose monochrome plate is drawn for a
            // 480×300 screen and displayed 1:1 (`mac/gfx.p`). Doubling that one
            // anyway would put a 960×600 screen behind a 480×300 plate.
            //
            // Absent a declared window there is no picture space to scale, so
            // the uniform rule stands and the screen is the 640×400 it always
            // was — the Blorb-less v6 stories (scopa, mysterious01) reach this.
            let screen_scale = match (v6_screen_px, v6_art_scale) {
                (Some(_), Some(s)) => s,
                _ => (V6_ART_SCALE, V6_ART_SCALE),
            };
            let w = art_w.saturating_mul(screen_scale.0.max(1) as u16);
            let h = art_h.saturating_mul(screen_scale.1.max(1) as u16);
            // SQ-0917: hand the machine the PIXELS, and let it derive the grid.
            //
            // This used to round the screen to the nearest whole CELL and declare
            // that instead, which was a workaround for the round trip on the other
            // side: `set_screen_dims` took a grid and multiplied it back into
            // `$22`/`$24`, so anything the cell did not divide was lost, and
            // rounding down would have told Zork Zero its 300-pixel Macintosh plate
            // sat on a 288-pixel screen. `set_v6_screen_px` carries the pixels
            // verbatim, so there is nothing to round and nothing to compensate for
            // — the screen IS the archive's, and the character grid is a quotient
            // of it exactly as `mac/xzip.lst` computes `totRows`/`totCols`.
            //
            // The rounding had to go rather than stay harmlessly: at the
            // Macintosh's 7-wide cell it turned 640 into 637 and 480 into 483.
            machine.set_v6_screen_px(w, h);
        } else if let Some((r, c)) = host_screen {
            // SQ-0680: seed the REAL host pane before boot, so a v4/v5 status
            // routine that lays itself out once at boot (Zork 1) bakes in field
            // columns that are already correct for this pane, rather than the
            // zvm 80×24 fallback `init_caps` just seeded a few lines above.
            machine.set_screen_dims(r.clamp(1, 255) as u8, c.clamp(1, 255) as u8);
        }
        // SQ-0680: the width actually declared to the story at boot — the
        // seeded pane column count, or the zvm fallback `init_caps` used absent
        // a seed. `declared_story_screen_dims`'s floor reads this back so it
        // never re-widens a correctly-seeded narrow boot to the old hardcoded
        // default.
        let boot_screen_cols = host_screen
            .filter(|_| machine.mem.version() != 6)
            .map(|(_, c)| c.clamp(1, 255))
            .unwrap_or(zvm::screen::DEFAULT_SCREEN_COLS as u16);
        // Trace from the very first instruction when requested, so the opening
        // run below records boot PCs into `ever_exec_pcs`. Also capture screen
        // opcodes from boot — a v6 game does its whole window/margin/picture
        // layout during boot, so `--trace screen` would otherwise miss it.
        machine.trace_exec = trace_from_boot;
        machine.trace_screen = trace_from_boot;

        let (pending, quit) = run_settled(&mut machine);

        Ok(GameSession {
            machine, quit, pending, strip_prompt: true, pen_before_char: None, output_continued: false,
            disasm_cache: std::cell::RefCell::new(None),
            world: std::cell::OnceCell::new(),
            parse_names: std::cell::OnceCell::new(),
            object_word_set: std::cell::RefCell::new(None),
            v6_model_memo: std::cell::RefCell::new(None),
            last_confirmed_pc: std::cell::Cell::new(None),
            pict_source: None,
            pictures_canvas: std::collections::HashMap::new(),
            canvas_anchor: std::collections::HashMap::new(),
            art_scale,
            paint: None,
            paced_frames: std::collections::VecDeque::new(),
            window_fills: std::collections::HashMap::new(),
            story_pics: Vec::new(),
            v6_win0_chars_seen: 0,
            display_ops: std::collections::HashMap::new(),
            unreplayable: std::collections::HashSet::new(),
            boot_screen_cols,
        })
    }

    /// Set the v6 Pict resolver used to rasterize `draw_picture`/`erase_picture`
    /// events into `pictures_canvas`. Call once, right after construction —
    /// `drain_turn` reads it on every turn (see the `pict_source` field doc).
    pub fn set_pict_source(&mut self, src: Option<crate::graphics::PictSource>) {
        self.pict_source = src;
    }

    /// The v6 Pict resolver, for inspection (its adaptive-palette state and
    /// decoded pictures). Used by the adaptive-palette headless oracle to decode
    /// a compass overlay with the Current Palette the real boot established.
    pub fn pict_source_mut(&mut self) -> Option<&mut crate::graphics::PictSource> {
        self.pict_source.as_mut()
    }

    /// Drain and apply any `draw_picture`/`erase_picture` events the VM queued
    /// during boot (`Machine::pending_pictures`, populated inside
    /// `new_with_trace` before this method can ever run). A v6 game like
    /// Zork0 draws its opening art during boot, before the first turn — call
    /// this once, right after `set_pict_source`, so the very first `screen()`
    /// (rendered before the player types anything) already reflects those
    /// boot draws instead of showing a blank graphics window until the first
    /// turn's `drain_turn` happens to pick them up (Plan 1b Task 5 gap).
    pub fn flush_boot_pictures(&mut self) {
        self.drain_pictures();
    }

    /// Encode each rasterized v6 window canvas (`pictures_canvas`) to PNG bytes,
    /// keyed by window number, for Lane P host Save State persistence. Ordered by
    /// each canvas's draw-order stamp (`z_seq`) ASCENDING — the same order the v6
    /// compositor paints them — so `load_pictures_png` can reproduce the relative
    /// z-order (later-drawn windows on top) from the blob order alone, without
    /// storing the raw stamps. Empty for non-v6 stories / before any graphics are
    /// drawn. Pass the result to `archive::save_archive_meta_pics`. PNG is
    /// lossless for RGBA, so a save → restore round-trip reproduces the canvases
    /// byte-for-byte.
    pub fn pictures_png(&self) -> Vec<(u8, Vec<u8>)> {
        let mut keys: Vec<u8> = self.pictures_canvas.keys().copied().collect();
        keys.sort_by_key(|k| (self.pictures_canvas[k].z_seq, *k));
        let mut out = Vec::with_capacity(keys.len());
        for k in keys {
            let canvas = &self.pictures_canvas[&k];
            let mut bytes = Vec::new();
            if image::DynamicImage::ImageRgba8((*canvas.img).clone())
                .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
                .is_ok()
            {
                out.push((k, bytes));
            }
        }
        out
    }

    /// Encode the v6 screen's PAINTED GROUND ([`paint`](Self::paint), SQ-0706) to
    /// PNG bytes for a host Save State, or `None` when the game has never painted
    /// one. Feed the result to [`archive::save_archive_meta_pics`]'s `ground`
    /// parameter and hand it back through [`load_paint_ground`](Self::load_paint_ground).
    ///
    /// PIXELS, not a recipe, and deliberately so — the exception CLAUDE.md's
    /// "persist the recipe" rule allows for a derived artifact that is itself
    /// authoritative. The ground's inputs are an UNBOUNDED stream of `erase_window`
    /// fills (scopa repaints its table hundreds of times per card), which is exactly
    /// why the ground is a surface rather than a list of rects in the first place;
    /// there is no bounded recipe to store. The surface is in the game's own native
    /// pixels, so it stays backend- and terminal-neutral like the rest of the
    /// archive, and PNG is lossless for RGBA so a round-trip is byte-for-byte.
    ///
    /// [`archive::save_archive_meta_pics`]: crate::archive::save_archive_meta_pics
    pub fn paint_ground_png(&self) -> Option<Vec<u8>> {
        let img = self.paint.as_deref()?;
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(img.clone())
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .ok()?;
        Some(bytes)
    }

    /// Install the painted ground a restore is bringing back (SQ-0787), REPLACING
    /// whatever the pre-restore screen left standing. `None` — an archive from a
    /// game that never painted one, or a decode failure — resets it to empty.
    ///
    /// The reset is the load-bearing half. A host Save State swaps VM memory under
    /// a game that never learns it happened, so the story issues no repaint; and
    /// `auto_load` restores AFTER the story has already booted and painted its
    /// opening screen. scopa is where that shows: it resumes a dealt hand with its
    /// MAIN MENU's cards and buttons still on the ground under the restored game's
    /// text, because the ground was the one v6 screen layer no restore touched.
    pub fn load_paint_ground(&mut self, png: Option<&[u8]>) {
        self.paint = png
            .and_then(|b| image::load_from_memory(b).ok())
            .map(|img| std::sync::Arc::new(img.to_rgba8()));
    }

    /// The v6 display list + Current Palette for a host Save State (SQ-0588),
    /// together with the windows whose PNG must still be stored as a fallback.
    ///
    /// Returns `(dto, fallback_windows, diagnostics)`. Every window is REPLAYED
    /// into a scratch canvas here and compared against the live one:
    ///
    /// - identical → the ops go in the archive and the PNG is dropped;
    /// - different (or the window is already `unreplayable`) → the window is left
    ///   out of the list, its PNG is kept, and a diagnostic names it.
    ///
    /// That check is the whole point of storing ops rather than pixels. A pixel
    /// snapshot restores correctly even when our op recording is incomplete, so a
    /// gap only ever surfaces later and indirectly — as it did in SQ-0587, where a
    /// restored window's art vanished a move later. Comparing at save time turns
    /// the same gap into a named window in a diagnostic, before the archive is
    /// written, while still producing an archive that restores correctly.
    ///
    /// Windows are emitted in paint order (ascending `z_seq`), matching
    /// [`pictures_png`](Self::pictures_png) so relative z-order survives either path.
    pub fn display_list(&mut self) -> (crate::archive::DisplayListDto, Vec<u8>, Vec<String>) {
        let mut keys: Vec<u8> = self.pictures_canvas.keys().copied().collect();
        keys.sort_by_key(|k| (self.pictures_canvas[k].z_seq, *k));

        let palette = self.pict_source.as_ref().and_then(|s| s.current_palette().map(<[u8]>::to_vec));
        let mut windows = Vec::new();
        let mut fallback = Vec::new();
        let mut diags = Vec::new();

        for win in keys {
            let (cw, ch) = {
                let c = &self.pictures_canvas[&win];
                (c.img.width(), c.img.height())
            };
            if self.unreplayable.contains(&win) {
                fallback.push(win);
                // The two ways a window becomes unreplayable have the same
                // consequence and completely different fixes, and they are already
                // distinguishable: an overflow leaves a FULL list behind, while a
                // window restored from pixels has none at all.
                let n = self.display_ops.get(&win).map_or(0, Vec::len);
                diags.push(if n >= V6_OPS_CAP {
                    format!(
                        "v6 window {win}: display list hit the {V6_OPS_CAP}-op cap, so it cannot be \
                         replayed — storing its canvas as a PNG. Its colours will not follow a later \
                         palette change. If a real game reaches this, the cap is too low."
                    )
                } else {
                    format!(
                        "v6 window {win}: restored from pixels, so it has no draw history to replay \
                         ({n} op(s) recorded) — storing its canvas as a PNG. Expected for a save \
                         made before the display list was persisted."
                    )
                });
                continue;
            }
            let ops = self.display_ops.get(&win).cloned().unwrap_or_default();
            let rebuilt = self.replay_into_scratch(&ops, cw, ch);
            if *rebuilt.img == *self.pictures_canvas[&win].img {
                windows.push(crate::archive::V6WindowOpsDto { win, w: cw, h: ch, ops });
            } else {
                fallback.push(win);
                diags.push(format!(
                    "v6 window {win}: replaying its {} recorded op(s) does not reproduce the live \
                     canvas — storing a PNG for it, and its colours will not follow a later palette \
                     change. A draw path that is not being recorded.",
                    ops.len()
                ));
            }
        }
        let layers = self.v6_screen_layers();
        (crate::archive::DisplayListDto { palette, windows, layers }, fallback, diags)
    }

    /// The v6 screen layers that ride BESIDE the window canvases (SQ-0814), as the
    /// recipe an archive carries: the surviving `erase_window` fills
    /// ([`window_fills`](Self::window_fills)) and the canvas anchors
    /// (`canvas_anchor`, SQ-0715).
    ///
    /// A RECIPE, not pixels — the exception the ground takes
    /// ([`paint_ground_png`](Self::paint_ground_png)) does not apply here, because
    /// both of these are bounded at one small struct per window however long the
    /// session runs. Everything in them is in the game's own native pixels or a
    /// packed RGB colour, so the archive stays backend- and terminal-neutral.
    ///
    /// Two per-session numbers are dropped on the way in, because neither survives
    /// into the restoring session with its meaning intact — both are stamps from
    /// counters the Quetzal save knows nothing about, and only what they DECIDE is
    /// worth carrying:
    ///
    /// - a fill's draw stamp comes from a process-global counter, so only the ORDER
    ///   of the fills travels (as this vector's order) and the restore re-stamps them
    ///   from the live counter, exactly as restored canvases are re-stamped;
    /// - a fill's `out_chars` decides one thing, whether the fill is still the newest
    ///   paint on the screen, so only the fills that still COVER travel. See
    ///   [`crate::archive::V6LayersDto::fills`].
    pub fn v6_screen_layers(&self) -> crate::archive::V6LayersDto {
        let now = self.machine.v6_win0_out_chars;
        let mut fills: Vec<(u64, crate::archive::V6FillDto)> = self
            .window_fills
            .iter()
            .filter(|(_, f)| f.out_chars == now)
            .map(|(&win, f)| {
                (f.seq, crate::archive::V6FillDto { win, x: f.x, y: f.y, w: f.w, h: f.h, bg: f.bg })
            })
            .collect();
        fills.sort_by_key(|(seq, f)| (*seq, f.win));
        let mut anchors: Vec<crate::archive::V6AnchorDto> = self
            .canvas_anchor
            .iter()
            .map(|(&win, a)| crate::archive::V6AnchorDto {
                win,
                origin_x: a.origin.0,
                origin_y: a.origin.1,
                x: a.rect.0,
                y: a.rect.1,
                w: a.rect.2,
                h: a.rect.3,
            })
            .collect();
        anchors.sort_by_key(|a| a.win);
        crate::archive::V6LayersDto {
            fills: fills.into_iter().map(|(_, f)| f).collect(),
            anchors,
        }
    }

    /// Install the v6 screen layers a restore is bringing back (SQ-0814), REPLACING
    /// whatever the pre-restore screen left standing. `None` — a non-v6 archive, or
    /// one written before these travelled — resets both to empty.
    ///
    /// The reset is the load-bearing half, exactly as it is for the painted ground
    /// ([`load_paint_ground`](Self::load_paint_ground)). `auto_load` restores after
    /// the story has already booted and painted its opening screen, and a host Save
    /// State swaps VM memory under a game that never learns it happened, so no
    /// repaint is ever issued: a fill left standing keeps hiding what the restored
    /// game drew under it, and an anchor left standing tells the next `move_window`
    /// to strand the restored canvas at coordinates belonging to a screen that no
    /// longer exists.
    pub fn load_v6_screen_layers(&mut self, dto: Option<&crate::archive::V6LayersDto>) {
        self.window_fills.clear();
        self.canvas_anchor.clear();
        let Some(d) = dto else { return };
        let now = self.machine.v6_win0_out_chars;
        for f in &d.fills {
            self.window_fills.insert(f.win, WindowFill {
                x: f.x,
                y: f.y,
                w: f.w,
                h: f.h,
                bg: f.bg,
                // Fresh stamps in the archived order: the saved numbers belong to the
                // process that wrote them, only their sequence is meaningful here.
                seq: crate::graphics::next_draw_seq(),
                // Every archived fill is one that still covers, which is what this
                // count says when it equals the live one.
                out_chars: now,
            });
        }
        for a in &d.anchors {
            self.canvas_anchor.insert(a.win, CanvasAnchor {
                origin: (a.origin_x, a.origin_y),
                rect: (a.x, a.y, a.w, a.h),
            });
        }
    }

    /// The longest display list any v6 window is currently holding, and how many
    /// windows have hit [`V6_OPS_CAP`].
    ///
    /// Exists to answer "is the cap big enough?" with a measurement instead of a
    /// guess. The number that matters is not the peak but whether it GROWS with play:
    /// a story that resets its list on whole-canvas ops (Arthur swapping screens)
    /// plateaus and is safe at any session length, while one that only ever appends
    /// would overflow eventually and the cap would just be a bigger number before the
    /// same failure.
    pub fn display_ops_extent(&self) -> (usize, usize) {
        let longest = self.display_ops.values().map(Vec::len).max().unwrap_or(0);
        let at_cap = self.display_ops.values().filter(|v| v.len() >= V6_OPS_CAP).count();
        (longest, at_cap)
    }

    /// PNG blobs for just `wins` (the fallback set from [`display_list`](Self::display_list)),
    /// in the same paint order [`pictures_png`](Self::pictures_png) uses.
    pub fn pictures_png_for(&self, wins: &[u8]) -> Vec<(u8, Vec<u8>)> {
        self.pictures_png().into_iter().filter(|(w, _)| wins.contains(w)).collect()
    }

    /// Replay `ops` into a fresh `w × h` canvas under the CURRENT palette, without
    /// touching any live canvas — the save-time self-check's scratch surface, and
    /// the restore path's canvas builder. Mirrors
    /// [`replay_under_current_palette`](Self::replay_under_current_palette) op for op;
    /// any divergence between the two would make the self-check meaningless.
    fn replay_into_scratch(&mut self, ops: &[V6Op], w: u32, h: u32) -> crate::graphics::Canvas {
        let mut canvas = crate::graphics::Canvas::new(w, h);
        canvas.erase_rect(0, 0, w, h);
        for op in ops {
            match *op {
                V6Op::Erase { dx, dy, w: ew, h: eh } => canvas.erase_rect(dx, dy, ew, eh),
                V6Op::Draw { number, dx, dy } => {
                    let Some(img) = self
                        .pict_source
                        .as_mut()
                        .and_then(|s| s.image_under_current_palette(number as u32))
                    else {
                        continue;
                    };
                    let img = v6_scaled_art(&img, self.art_scale);
                    canvas.draw_image_clipped(&img, dx, dy, (w, h));
                }
            }
        }
        canvas
    }

    /// Rebuild the v6 screen from a restored display list (SQ-0588) — the counterpart
    /// of [`display_list`](Self::display_list), and the reason a restored window can be
    /// recoloured at all.
    ///
    /// The Current Palette is reinstated FIRST (Blorb §11.3: an adaptive picture has no
    /// palette of its own and decodes through whichever one is live), then each window's
    /// canvas is rebuilt by replaying its ops. Those windows keep their display lists, so
    /// the next palette change replays them again — which is exactly what a window
    /// restored from a PNG cannot do.
    ///
    /// `pngs` covers the windows the list does not: the save-time self-check's fallbacks,
    /// and every window of a pre-SQ-0588 archive. They load as pixels and are marked
    /// `unreplayable`, i.e. today's behaviour, unchanged.
    pub fn load_display_list(&mut self, dto: &crate::archive::DisplayListDto, pngs: &[(u8, Vec<u8>)]) {
        // Pixels first, so a window present in BOTH (which should not happen, but an
        // archive is an external input) ends up rebuilt from ops rather than pixels.
        self.load_pictures_png(pngs);
        if let Some(src) = self.pict_source.as_mut() {
            src.set_current_palette(dto.palette.clone());
        }
        for w in &dto.windows {
            let ops = w.ops.clone();
            let mut canvas = self.replay_into_scratch(&ops, w.w, w.h);
            canvas.version = canvas.version.wrapping_add(1);
            canvas.z_seq = crate::graphics::next_draw_seq();
            self.pictures_canvas.insert(w.win, canvas);
            self.display_ops.insert(w.win, ops);
            self.unreplayable.remove(&w.win);
        }
    }

    /// Rebuild `pictures_canvas` from persisted per-window PNG blobs
    /// (`archive::ArchiveContents::pictures`) after a host Save State restore, so
    /// a v6 story's graphics windows redraw identically without replaying draw
    /// events (Lane P). Replaces the current canvases. `blobs` are expected in
    /// paint order (as `pictures_png` emits them and the archive preserves);
    /// fresh z-order stamps are assigned sequentially so the ORIGINAL relative
    /// z-order (later-drawn windows on top) is reproduced.
    pub fn load_pictures_png(&mut self, blobs: &[(u8, Vec<u8>)]) {
        self.pictures_canvas.clear();
        // A restore swaps the whole screen out; any sequence still playing belongs
        // to the session that was just discarded (SQ-0708).
        self.paced_frames.clear();
        for (win, png) in blobs {
            let Ok(img) = image::load_from_memory(png) else { continue };
            let rgba = img.to_rgba8();
            let mut canvas = crate::graphics::Canvas::new(rgba.width(), rgba.height());
            canvas.img = std::sync::Arc::new(rgba);
            canvas.version = canvas.version.wrapping_add(1);
            canvas.z_seq = crate::graphics::next_draw_seq();
            self.pictures_canvas.insert(*win, canvas);
        }
        // Restored pixels arrive with NO draw history: they were persisted as an
        // image, not as the ops that built them. Mark every one UNREPLAYABLE, which
        // is exactly what that set means (SQ-0567 uses it for a window whose op list
        // can no longer reproduce its canvas).
        //
        // Without this, the first palette change after a restore ERASES the restored
        // art. `replay_under_current_palette` clears each window's canvas and rebuilds
        // it from the display list — and a restored window's list is empty, or worse
        // holds only the Erase ops that `erase_screen_rect` records when a LATER
        // window is erased over it. Arthur shows the cost: one move after a restore
        // recolours the palette, its full-screen border window replays a list of pure
        // erases, and the surrounding art vanishes while the room picture — redrawn by
        // the game that same turn — stays. (SQ-0587)
        self.display_ops.clear();
        self.unreplayable.clear();
        for (win, _) in blobs {
            self.unreplayable.insert(*win);
        }
    }

    /// Drain the transcript accumulated since the last drain (intro or last turn).
    pub fn take_transcript(&mut self) -> String {
        let raw = sink_mut(&mut self.machine).take_text();
        // Keep the win0 char-offset base in sync with the drained sink, so any
        // later inline-picture interleave measures against the right origin.
        self.v6_win0_chars_seen = self.machine.v6_win0_out_chars;
        if self.strip_prompt { strip_read_prompt(&raw).to_owned() } else { raw }
    }

    /// Drain the game's pending screen clear: the per-turn `erase_window` flag
    /// (ZMSD §8.7.3) **and** the position stamp that is the same erase seen from
    /// the other side (SQ-0751, `CaptureSink::cleared_at`).
    ///
    /// One fact, taken together, because a caller that takes the flag alone leaves
    /// the stamp behind to resurface as a mid-turn `ScreenClear` boundary in
    /// somebody else's turn. `.1` is where in the drained text the erase fell, in
    /// characters — `None` when nothing recorded a position.
    ///
    /// The only two callers are [`GameSession::drain_turn`], which needs the
    /// position, and the boot's [`Engine::drain_screen_clear`], which does not
    /// (SQ-1106): the boot's erase is what the game did before printing its banner,
    /// and the host has drawn nothing for it to fall on.
    pub fn take_screen_clear(&mut self) -> (bool, Option<usize>) {
        let at = sink_mut(&mut self.machine).take_cleared_at();
        (std::mem::take(&mut self.machine.screen.erase_lower_requested), at)
    }

    /// Whether the game's trailing `>` read prompt is stripped from transcripts.
    pub fn strip_prompt(&self) -> bool {
        self.strip_prompt
    }

    /// Which kind of input the VM is currently waiting for.
    pub fn pending_input(&self) -> InputKind {
        self.pending
    }

    #[cfg(test)]
    fn interpreter_number_for_test(&self) -> u8 {
        self.machine.mem.read_byte(0x1E)
    }

    /// Supply a player command, step until the next input request or Quit,
    /// and return the turn result.
    pub fn submit(&mut self, command: &str) -> TurnResult {
        self.submit_line_with_terminator(command, 13)
    }

    /// Supply a player command terminated by an explicit ZSCII terminator (v5+
    /// terminating-characters table), step until the next input request or Quit,
    /// and return the turn result. `submit` is this with terminator 13 (Enter).
    pub fn submit_line_with_terminator(&mut self, command: &str, terminator: u8) -> TurnResult {
        self.machine.supply_line(command, terminator);
        self.advance_after_input(false)
    }

    /// v5+: does `ch` terminate a line read per the game's terminating-characters
    /// table? Thin wrapper over [`Machine::is_terminator`].
    pub fn is_terminator(&self, ch: u16) -> bool {
        self.machine.is_terminator(ch)
    }

    /// Supply a single keypress, step until the next input request or Quit,
    /// and return the turn result.
    pub fn submit_char(&mut self, ch: u8) -> TurnResult {
        self.arm_line_continuation();
        self.machine.supply_char(ch);
        self.advance_after_input(false)
    }

    /// Note where the prose window's cursor is, and forget where the last burst
    /// of prose started, so `drain_turn` can tell whether this turn's output
    /// CONTINUED the line the previous one left the cursor on (SQ-0804).
    ///
    /// A `read_char` echoes nothing at all (ZMSD §10.7), so the host has to
    /// decide for itself whether the turn's output opens a transcript line, and
    /// the printed text does not say: a game redrawing a menu `set_cursor`s back
    /// to the top with no newline in sight. The window's own cursor does say.
    /// Armed only for a keypress, which is the only turn that has the question —
    /// an interpreter echoes a `read` together with its terminating newline
    /// (§7.1.1.1), so a command turn's reply always opens a line.
    fn arm_line_continuation(&mut self) {
        let idx = self.machine.screen.v6_input_window as usize;
        self.pen_before_char = self.machine.screen.v6_mut().and_then(|v6| {
            let w = v6.windows.get_mut(idx)?;
            w.clear_stream_origin();
            Some(w.pen())
        });
    }

    /// While a timed read/read_char is pending, `(time_tenths, packed_routine)`
    /// — the interval to poll for and the interrupt routine to run on timeout.
    /// `None` for an untimed read or when no read is pending.
    pub fn pending_timeout(&self) -> Option<(u16, u16)> {
        self.machine.pending_timeout()
    }

    /// Run the pending read's interrupt routine once. If the routine aborts the
    /// read, completes it via `abort_timed_input` (steps to the next input,
    /// `timed_out == true`); otherwise the read is still pending, and the
    /// returned `TurnResult` carries only the routine's drained output
    /// (`pending`/`quit` unchanged, `timed_out == false`).
    pub fn run_timed_interrupt(&mut self) -> TurnResult {
        let out = self.machine.run_timed_interrupt();
        if out.aborted {
            self.abort_timed_input("")
        } else {
            self.collect_turn()
        }
    }

    /// Run a sampled sound's finish-routine (v5+) to completion and drain any
    /// output it produced. The return value is ignored (ZMSD §9.4 — it does not
    /// abort anything). Does not step a pending read forward.
    pub fn run_sound_finish(&mut self, routine: u16) -> TurnResult {
        self.machine.run_routine(routine);
        self.collect_turn()
    }

    /// Complete the pending read as timed-out: `read_char` delivers ZSCII 0;
    /// `read` writes the partial `typed` line with terminator 0. Steps to the
    /// next input request and returns a `TurnResult` with `timed_out == true`.
    pub fn abort_timed_input(&mut self, typed: &str) -> TurnResult {
        self.machine.abort_timed_input(typed);
        self.advance_after_input(true)
    }

    /// Resume after the host performed an in-game SAVE (`wrote_ok` = file written).
    pub fn resume_save(&mut self, wrote_ok: bool) -> TurnResult {
        self.machine.complete_save(wrote_ok);
        let stop = run_until_input(&mut self.machine);
        self.finish_turn(stop)
    }

    /// Resume after the host performed an in-game RESTORE. `Some(bytes)` =
    /// the user picked a save (Quetzal); `None` = cancelled. On corrupt bytes we
    /// fall back to failure so the game sees a clean "Failed.".
    pub fn resume_restore(&mut self, data: Option<&[u8]>) -> TurnResult {
        match data {
            Some(bytes) => {
                if self.machine.complete_restore_success(bytes).is_err() {
                    self.machine.complete_restore_failure();
                }
            }
            None => self.machine.complete_restore_failure(),
        }
        let stop = run_until_input(&mut self.machine);
        self.finish_turn(stop)
    }

    /// Raise [`boot_screen_cols`](Self::boot_screen_cols) to the width the game
    /// memory a restore just installed was laid out for (SQ-0681).
    ///
    /// The SQ-0679 floor keeps a v4/v5 story's one-shot status layout inside the
    /// window it was computed for, and SQ-0680 keyed that floor to THIS session's
    /// boot width. A restore breaks the assumption behind both: the memory image
    /// now running is one another session booted, at ITS width, and the field
    /// columns baked into it answer to that width alone. Restoring an 80-column
    /// Save State into a 60-column session left the floor at 60, so the app
    /// declared 60, the game's `set_cursor` to column 73 became illegal
    /// (ZMSD §8.7.2.3), the interpreter dropped it and the score digits printed
    /// at column 1 over the room name — the SQ-0679 garble, re-manifested every
    /// turn by a save file.
    ///
    /// Only ever grows (`max`): a session that booted WIDER than the save keeps
    /// its own width, since its header — which the restored game reads next —
    /// still reports the wider screen, and the restored layout fits inside it.
    ///
    /// Exempt: v1–3 and v6, the versions [`declared_story_screen_dims`] does not
    /// floor at all (no §8.4 header fields / a native pixel screen), so the field
    /// keeps reporting the boot width for them.
    ///
    /// [`declared_story_screen_dims`]: crate::render::screen::declared_story_screen_dims
    pub fn note_restored_screen_cols(&mut self, cols: u16) {
        let version = self.machine.mem.version();
        if version < 4 || version == 6 {
            return;
        }
        self.boot_screen_cols = self.boot_screen_cols.max(cols);
    }

    /// Build the `TurnResult` from a `RunStop` and drain the VM's per-turn
    /// buffers. Shared by submit/submit_char/resume_*.
    fn finish_turn(&mut self, stop: RunStop) -> TurnResult {
        let (quit, pending, pending_io) = match stop {
            RunStop::Quit => (true, InputKind::Line, None),
            RunStop::Input(k) => (false, k, None),
            RunStop::SavePending => (false, self.pending, Some(PendingIo::Save)),
            RunStop::RestorePending => (false, self.pending, Some(PendingIo::Restore)),
        };
        self.quit = quit;
        self.pending = pending;
        self.drain_turn(quit, pending_io, false)
    }

    /// Step the VM to the next input request (or Quit) and build the
    /// `TurnResult` — the shared tail of `submit`/`submit_char`/
    /// `abort_timed_input` once input has been supplied to the VM. `timed_out`
    /// is `true` only for the `abort_timed_input` caller.
    fn advance_after_input(&mut self, timed_out: bool) -> TurnResult {
        let stop = run_until_input(&mut self.machine);
        let mut result = self.finish_turn(stop);
        result.timed_out = timed_out;
        result
    }

    /// Drain the VM's per-turn output into a `TurnResult` without stepping —
    /// used after a timed-interrupt routine ran but did not abort the read: the
    /// read is still pending, so `quit`/`pending` are left as-is and
    /// `timed_out` stays `false`.
    fn collect_turn(&mut self) -> TurnResult {
        self.drain_turn(self.quit, None, false)
    }

    /// Drain the VM's per-turn buffers (transcript, location, diagnostics, sounds,
    /// erase_lower) into a `TurnResult`, given the already-resolved
    /// `quit`/`pending_io`/`timed_out` state. Shared by
    /// `finish_turn` (after stepping to the next input) and `collect_turn`
    /// (mid-read, after a timed-interrupt routine that did not abort).
    fn drain_turn(
        &mut self,
        quit: bool,
        pending_io: Option<PendingIo>,
        timed_out: bool,
    ) -> TurnResult {
        // The VM ran, so dynamic memory may have changed under the cached
        // object-word set — a game CAN rewrite an object's parse-name property
        // mid-play, and a stale set would keep answering for the old words.
        // Every VM-stepping path drains through here (submit, timed interrupts,
        // the game's own @restore/@restart), so this is the per-turn
        // invalidation the cache's soundness rests on (SQ-1176).
        self.object_word_set.take();
        // A mid-turn @restart re-booted the VM: drop the app-side v6 chrome the
        // VM's own screen reset cannot reach — the rasterized picture-canvas
        // cache and the window-0 char counters — so the reboot's fresh boot art
        // and text aren't offset against pre-restart state. The reboot's own
        // pictures/text are drained below onto the now-clean canvas.
        if std::mem::take(&mut self.machine.just_restarted) {
            self.pictures_canvas.clear();
            self.story_pics.clear();
            // The display list is the RECIPE for `pictures_canvas` — dropping the
            // canvas and keeping the ops leaves a save/replay that cannot be
            // recomputed from its inputs. Concretely (SQ-0658): the reboot's first
            // draw into a window re-creates its canvas and APPENDS to the ops the
            // pre-restart session left there, so the next palette change replays
            // the old game's art onto the new one's screen; and a window marked
            // `unreplayable` before the restart stays excluded from replay for the
            // rest of the session even though its canvas is brand new. Nor do the
            // pre-restart erase FILLS describe any region of the rebooted screen.
            // An `erase_window` clears a window's ops on the way past, so an
            // Infocom boot that erases before it draws heals itself — but only
            // for the windows it happens to erase, and only in that order.
            self.display_ops.clear();
            self.unreplayable.clear();
            self.window_fills.clear();
            // The same argument reaches the other two layers beside the window tree
            // (SQ-0814). A canvas ANCHOR describes where a canvas that no longer
            // exists was painted, and the canvas above was just dropped — left
            // behind, it unions the reboot's first draw into a pre-restart footprint
            // and strands it at a pre-restart origin. And the painted GROUND is the
            // dead screen's own pixels: the reboot inherits them wholesale unless it
            // happens to clear the full screen with an explicitly coloured erase,
            // which is the only thing that drops the ground on its own.
            self.canvas_anchor.clear();
            self.paint = None;
            self.v6_win0_chars_seen = 0;
        }
        // Did this turn's first printed glyph land exactly where the previous
        // output left the cursor (SQ-0804)? Answered before anything else drains,
        // and only for a turn `arm_line_continuation` armed.
        self.output_continued = self.pen_before_char.take().is_some_and(|pen| {
            let idx = self.machine.screen.v6_input_window as usize;
            self.machine
                .screen
                .v6
                .as_ref()
                .and_then(|v6| v6.windows.get(idx))
                .is_some_and(|w| w.stream_origin == Some(pen))
        });
        let win0_base = self.v6_win0_chars_seen;
        let (erase_lower, cleared_at) = self.take_screen_clear();
        let (raw, raw_runs) = sink_mut(&mut self.machine).take_styled();
        self.v6_win0_chars_seen = self.machine.v6_win0_out_chars;
        let transcript = if self.strip_prompt { strip_read_prompt(&raw).to_owned() } else { raw };
        let transcript_runs = clamp_runs(raw_runs, transcript.chars().count());
        let detected = detect_location(&self.machine);
        let location = detected.as_ref().map(location_to_snapshot);
        let location_method = detected.as_ref().map(Location::method);

        let diagnostics = std::mem::take(&mut self.machine.diagnostics);
        let fault = self.machine.take_fault_trace().map(|t| t.to_lines());
        let sounds = std::mem::take(&mut self.machine.pending_sounds);
        // A v6 wrap+scroll window moved out from under prose it had already
        // printed, and the engine froze that prose where it was painted (SQ-0697).
        // The stamp is in the same window-0 output-char space as an inline
        // picture's anchor, so the boundary rides the SAME interleave: it becomes
        // a `ScreenClear` element between the frozen text and what the game
        // printed at the window's new origin. A flat `mark_screen_clear` around
        // the whole push could not split a turn that contains both halves — and
        // Shogun's opening is exactly one such turn.
        let prose_retired_at = std::mem::take(&mut self.machine.v6_prose_retired);
        let prose_retired = prose_retired_at
            .map(|at| (at.saturating_sub(win0_base) as usize).min(transcript.chars().count()));
        // …and the head above that boundary is not scrollback, it is PAINT (SQ-0890).
        // The frozen runs publish as their own layer and the composite draws them at
        // the game's own coordinates; carrying the same characters in the transcript
        // too meant the story box re-rendered them a second time, into the four-row
        // prose box Shogun moves window 0 down to, straight across its START /
        // RESTORE / QUIT menu ("Copyright (c) 1988 by InfocomQUIT the game"). So the
        // host stops carrying what it can see is already on the screen.
        //
        // That rule used to have a picture-side twin to point at —
        // `ImageSource::ContentSplash` (SQ-0461), which marked art the window
        // canvas already carried so the drawing modes would skip the transcript's
        // copy. SQ-0895 retired it: it existed for the frameless mode, and with
        // the mode gone nothing anchored those bands at all. The principle is
        // unchanged and now lives only here, which is why it is spelled out rather
        // than cross-referenced.
        //
        // Only when the freeze took the window's WHOLE streamed screen: a partial
        // retirement interleaves frozen and still-live runs in one character stream
        // and no single offset separates them, so that case keeps every line.
        let froze_whole = std::mem::take(&mut self.machine.v6_prose_retired_whole);
        let frozen_head = prose_retired.filter(|_| froze_whole);
        // …and the BOUNDARY is the same statement as the head-drop above, so it is
        // gated by the same condition (SQ-1155). A `ScreenClear` says "everything
        // above is scrollback; the live screen begins here" — true of Shogun, where
        // window 0 leaves its whole nine-line header behind, and false of a PARTIAL
        // retirement, where the window still displays every run it kept. Arthur is
        // the partial case and reaches it on an ordinary turn: an unknown word makes
        // him shrink window 0 by exactly one row (192→176 native) to open his
        // one-line message window at native y=385, which strands the bottom-most run
        // and nothing else. Announcing that as a screen clear anchored the transcript
        // past everything on screen, and since the rejection prints into window 3
        // rather than window 0 the player was left looking at a blank pane with
        // "You don't need to use the word 'wa.'" alone on the bottom line.
        let prose_cleared_at = prose_retired_at.filter(|_| froze_whole);
        // SQ-0755: the same boundary from the other cause — the game ERASED the
        // window the host's transcript belongs to. A v6 erase never reached the host
        // at all (`erase_lower_requested` is the v1–5 lower window's flag), so the
        // transcript kept re-rendering every line the game had ever printed into
        // whatever the story window is now: Journey's boot brought its full-screen
        // title block, copyright and "[Press any key to begin]" into the 368x272 panel
        // the play layout opens on the right, pushing the intro to the bottom of it.
        // It rides the elems channel rather than `erase_lower` because the app
        // TRUNCATES the transcript on a game-driven `erase_lower` (SQ-0407's
        // menu-redraw collapse) and Journey's every move is a keystroke.
        let screen_cleared_at = std::mem::take(&mut self.machine.v6_screen_cleared);
        // SQ-0751: the same boundary for v1–5/7/8, where the erase is announced by
        // `erase_lower` — a per-TURN flag, which cannot say where inside the turn the
        // erase fell. `finish_command_turn` marks the boundary at the turn's start, so
        // a turn that PRINTS and then erases kept its pre-erase text on the cleared
        // screen. `CaptureSink::screen_cleared` stamps the position as the opcode runs;
        // it becomes a `ScreenClear` element on the same interleave channel the v6
        // boundaries ride, which splits the turn's output around it.
        //
        // ONLY for a genuine mid-turn split. An erase at offset 0 — every game in
        // SQ-0748's sweep, which all erase before they print — is exactly what marking
        // at the turn's start already describes, and is left on the flat path so
        // nothing about those turns changes.
        let cleared_mid_turn = cleared_at
            .filter(|&at| at > 0)
            .map(|at| win0_base + at.min(transcript.chars().count()) as u64);
        let pictures = self.drain_pictures();
        // Window-0 inline pictures interleave into this turn's text as ordered
        // elements; empty for turns without them (the app then uses the flat
        // transcript path unchanged).
        let transcript_elems = if self.story_pics.is_empty()
            && prose_cleared_at.is_none()
            && screen_cleared_at.is_none()
            && cleared_mid_turn.is_none()
        {
            Vec::new()
        } else {
            let mut marks: Vec<(u64, TranscriptElem)> = std::mem::take(&mut self.story_pics)
                .into_iter()
                .map(|(at, img)| (at, TranscriptElem::Image(img)))
                .collect();
            for at in [prose_cleared_at, screen_cleared_at, cleared_mid_turn].into_iter().flatten() {
                // One boundary per offset: a turn that both retires and erases at the
                // same point in its output has cleared the screen once.
                if !marks.iter().any(|(m, e)| *m == at && matches!(e, TranscriptElem::ScreenClear)) {
                    marks.push((at, TranscriptElem::ScreenClear));
                }
            }
            interleave_story_elems(&transcript, &transcript_runs, marks, win0_base, frozen_head)
        };

        TurnResult {
            transcript,
            transcript_runs,
            location,
            quit,
            erase_lower,
            info: None,
            sounds,
            glulx_sound_ops: Vec::new(),
            diagnostics,
            fault,
            location_method,
            pending_io,
            timed_out,
            pictures,
            transcript_elems,
            prose_retired,
        }
    }

    /// Paint one `erase_window` fill onto the screen's painted ground (SQ-0706).
    /// See [`GameSession::paint`].
    ///
    /// Coordinates arrive 1-based in the game's own native pixels, absolute rather
    /// than window-relative — the window that drew them has usually been moved and
    /// resized for this one rectangle and will move again before the next.
    fn apply_erase_fill(&mut self, f: &zvm::cpu::exec::EraseFill) {
        // A colour the game named outright, or nothing to paint: an erase that
        // inherits its colour is asking the host to resolve the ground, which
        // is what the ordinary window background already does.
        let Some(rgba) = crate::render::v6_layout::explicit_pixel_rgba(
            crate::state::pack_zcolour(f.bg),
        ) else {
            return;
        };
        if f.w == 0 || f.h == 0 {
            return; // a window that was never given a box covers no pixels
        }
        let (sw, sh) = self.v6_native_extent();
        // A fill spanning the WHOLE screen is a screen clear, not drawing: the
        // page/backdrop machinery already resolves the ground (SQ-0704), and
        // treating it as paint would blanket every game that merely erases —
        // Arthur's intro erases all eight windows, Zork Zero's boot erases the
        // screen. So it drops the painted ground instead, which is also what
        // keeps this surface bounded when a card table repaints for a new hand.
        // "Whole screen" means the screen: `v6_native_extent` used to answer with
        // window 0's box, which made a fill the size of the STORY window read as a
        // clear and take the status ribbon's ground down with it (SQ-0967).
        if u32::from(f.w) >= sw && u32::from(f.h) >= sh {
            self.paint = None;
            return;
        }
        let canvas = self
            .paint
            .get_or_insert_with(|| std::sync::Arc::new(image::RgbaImage::new(sw, sh)));
        let img = std::sync::Arc::make_mut(canvas);
        let (x0, y0) = (u32::from(f.x.max(1)) - 1, u32::from(f.y.max(1)) - 1);
        let (x1, y1) = ((x0 + u32::from(f.w)).min(img.width()), (y0 + u32::from(f.h)).min(img.height()));
        for y in y0..y1 {
            for x in x0..x1 {
                img.put_pixel(x, y, rgba);
            }
        }
    }

    /// The v6 SCREEN's size in native pixels — the extent the painted ground is
    /// allocated at, and the space every fill recorded into it is addressed in.
    ///
    /// Header words $22/$24 (ZMSD §11.1: screen width and height "in units",
    /// which Version 6 measures in pixels). `zvm::screen::write_screen_dims` is
    /// their only writer and the app seeds it at boot from the whole `std_window`
    /// chain, so this is the one place that states the SCREEN rather than a
    /// window: 640x400 for an IBM PC press, 560x384 for an Apple IIgs one.
    ///
    /// This used to read window 0's box instead, on the standard's word that
    /// window 0 opens as the whole screen — true at boot and false from the first
    /// `window_size`, which every v6 game issues. `erase_window` records its
    /// rectangle in SCREEN coordinates, so a surface cut to the STORY window
    /// silently dropped whatever fell outside it: Shogun r322's status erase at
    /// native (46,0) 548x32 was clipped at x=548 on a 548x368 surface and never
    /// reached the right flank at native 590, and Journey r77 (ProDOS) allocated
    /// 304x288 for a 560x384 screen. One symptom, one side each, two layers —
    /// SQ-0948 fixed the declared page and this is the painted ground beneath it
    /// (SQ-0967).
    ///
    /// Window 0's box remains the fallback for a story booted with no screen size
    /// stated, then the 640x400 the era assumed.
    fn v6_native_extent(&self) -> (u32, u32) {
        let hdr = (
            u32::from(self.machine.mem.read_word(0x22)),
            u32::from(self.machine.mem.read_word(0x24)),
        );
        if hdr.0 > 1 && hdr.1 > 1 {
            return hdr;
        }
        self.machine
            .screen
            .v6
            .as_ref()
            .map(|v6| {
                let w = &v6.windows[0];
                (u32::from(w.x_size).max(1), u32::from(w.y_size).max(1))
            })
            .filter(|&(w, h)| w > 1 && h > 1)
            .unwrap_or((640, 400))
    }

    /// Drain `Machine::pending_pictures` and `Machine::pending_erase_fills`,
    /// applying both to the screen IN THE ORDER THE GAME ISSUED THEM, and return
    /// the drained picture events for `TurnResult` — mirrors `pending_sounds`,
    /// except the rasterization happens here rather than in the app layer (Task 2
    /// decision: canvas store + Pict source both live on `GameSession` so the Task
    /// 4 screen adapter can read `pictures_canvas` without reaching into
    /// `AppState`). A no-op drain for non-v6 stories, which never push either.
    ///
    /// The two queues are one timeline (`EraseFill::pics_before`, SQ-0715). Fills
    /// and pictures paint the same screen, so draining one and then the other
    /// replays the turn out of order: scopa's boot fills the green table, draws
    /// its Neapolitan and Sicilian card pictures and then fills the menu buttons,
    /// and running all the fills last let the opening full-screen clear erase both
    /// cards it had already painted.
    fn drain_pictures(&mut self) -> Vec<PictureEvent> {
        let events = std::mem::take(&mut self.machine.pending_pictures);
        let fills = std::mem::take(&mut self.machine.pending_erase_fills);
        let mut next_fill = 0usize;
        // A new turn supersedes whatever sequence was still playing: those frames
        // describe a screen the game has already moved on from.
        self.paced_frames.clear();
        let palette_before = self.palette_gen();
        // Which of these events PAINT, and for how long (SQ-0708). Everything else
        // — the `erase_window` canvas clears a v6 screen swap opens with, a
        // window-0 inline float that anchors to the transcript instead of a canvas
        // — carries no hold and rides with the next painted frame, exactly as an
        // erase-then-draw pair did on the hardware.
        let holds: Vec<Option<std::time::Duration>> =
            events.iter().map(|ev| self.picture_hold(ev)).collect();
        // The final painted event needs no frame of its own: what it leaves behind
        // IS the settled composite, which is what the renderer falls back to.
        let last_painted = holds.iter().rposition(Option::is_some);
        // …and only a sequence that REVEALS is worth watching (SQ-0708, narrowed).
        // Measuring the corpus showed most multi-picture turns are screen
        // ASSEMBLY — Zork Zero's border tiles, Arthur's gameplay chrome, Shogun's
        // title — disjoint pieces building one static frame, where a delay buys
        // nothing and costs snappiness. Arthur's intro is the other shape: picture
        // 3 lands INSIDE picture 2, painting over ground the plate just covered.
        // Overlap is what separates them, and it is geometry rather than a
        // threshold, so it cannot drift.
        // …decided PER EVENT, not per turn. One boolean for the whole batch made
        // every picture in it pace as soon as anything overlapped: Zork Zero's boot
        // queues its banner, both side pillars AND an eight-frame compass animation
        // together, so the pillars — each a single disjoint image — were held only
        // because the compass shared their turn.
        let covers_earlier = self.events_that_repaint_covered_ground(&events);
        for (i, ev) in events.iter().enumerate() {
            // Every fill the game issued before this picture goes down first.
            while next_fill < fills.len() && fills[next_fill].pics_before as usize <= i {
                let f = fills[next_fill];
                self.apply_erase_fill(&f);
                next_fill += 1;
            }
            self.apply_picture_event(ev);
            // Hold the screen here only when the NEXT picture to paint is about to
            // cover ground already painted — that is the moment there is something
            // to watch. Otherwise the next picture lands beside this one and the
            // pair may as well arrive together.
            let pause_here = events[i + 1..]
                .iter()
                .enumerate()
                .find(|(j, _)| holds[i + 1 + j].is_some())
                .is_some_and(|(j, _)| covers_earlier[i + 1 + j]);
            if let (Some(hold), Some(last)) = (holds[i], last_painted) {
                if pause_here && i < last && self.paced_frames.len() < PACE_MAX_FRAMES {
                    self.paced_frames.push_back(PacedFrame {
                        canvas: self.pictures_canvas.clone(),
                        hold,
                    });
                }
            }
        }
        // …and everything the game filled after its last picture (or, on a turn
        // with no pictures at all, the whole queue).
        for f in &fills[next_fill..] {
            let f = *f;
            self.apply_erase_fill(&f);
        }
        // The last picture of the turn may still be sitting on a canvas whose
        // window has since moved (scopa moves window 3 again for the next fill
        // that is not an erase, and on a turn that ends with a draw there is no
        // later event at all). Settle it now, so what the renderer composites is
        // the screen the game left behind (SQ-0715).
        let stranded: Vec<u8> = self.canvas_anchor.keys().copied().collect();
        for win in stranded {
            let Some(now) = self.window_origin(win) else { continue };
            self.retire_stranded_canvas(win, now);
        }
        // A base draw in this batch established a different Current Palette, so
        // every adaptive picture already on screen is now showing the old one and
        // has to be replotted (Blorb §11.3, SQ-0567). Checked once per batch: with
        // several palette changes in one turn only the final palette is visible.
        // The paced frames were snapshotted BEFORE this replay, so they keep the
        // palette that was live when they were painted — which is what the hardware
        // showed. A v6 framebuffer holds palette INDICES: loading a new palette
        // recolours everything already on the screen at the instant the picture
        // carrying it lands, not a moment before.
        if self.palette_gen() != palette_before {
            self.replay_under_current_palette();
        }
        events
    }

    /// Which of this turn's pictures REPAINT ground an earlier one in the same
    /// turn already covered (SQ-0708) — the events worth watching land.
    ///
    /// This is the whole scope rule for pacing, and it is deliberately geometric
    /// rather than a count, a coverage threshold or a config key: those drift,
    /// and this cannot.
    ///
    /// * **Reveal** — Arthur's intro draws the graveyard plate (584x392 at 29,5)
    ///   and then Merlin (480x300 at 81,51) *inside* it. Zork Zero's boot cycles
    ///   eight pictures through one 45x40 rect at (277,1) — a frame-by-frame
    ///   animation. In both, a picture is only meaningful as a change to what the
    ///   last one put there, so watching it land is the point.
    /// * **Assembly** — Shogun's title draws two pictures side by side; Zork
    ///   Zero's side pillars are one single image each, abutting the banner above
    ///   them and disjoint from everything. No pixel is painted twice, nothing is
    ///   revealed, and holding the screen only makes it slower to finish.
    ///
    /// Answered PER EVENT rather than once per turn, because a single batch
    /// routinely contains both: Zork Zero's boot queues its banner, both pillars
    /// and the compass animation together. A per-turn verdict made the pillars
    /// pace purely because the compass was in the same queue.
    ///
    /// Rects are compared in the game's own unit space (art dims doubled by
    /// [`V6_ART_SCALE`], the coordinates `draw_picture` itself uses), and only
    /// within a window — two windows' pictures overlapping on screen is a layout,
    /// not a redraw.
    fn events_that_repaint_covered_ground(&mut self, events: &[PictureEvent]) -> Vec<bool> {
        let mut covers = vec![false; events.len()];
        let mut painted: Vec<(u8, u32, u32, u32, u32)> = Vec::new();
        for (i, ev) in events.iter().enumerate() {
            // Same exclusions as `picture_hold`: an erase is a fill, a window-0
            // cursor draw is a transcript float, and unresolvable art paints
            // nothing. None of them can cover ground or be covered.
            if ev.erase || ev.number == 0 || self.is_win0_inline_float(ev) {
                continue;
            }
            let Some((w, h)) = self.pict_source.as_mut().and_then(|p| p.dims(ev.number as u32)) else {
                continue;
            };
            let (x0, y0) = (u32::from(ev.x.max(1)) - 1, u32::from(ev.y.max(1)) - 1);
            let (w, h) = (w * self.art_scale.0, h * self.art_scale.1);
            let (x1, y1) = (x0 + w, y0 + h);
            covers[i] = painted.iter().any(|&(win, px0, py0, px1, py1)| {
                win == ev.window && x0 < px1 && px0 < x1 && y0 < py1 && py0 < y1
            });
            painted.push((ev.window, x0, y0, x1, y1));
        }
        covers
    }

    /// How long the screen rests on a picture event before the next one lands, or
    /// `None` for an event that paints no canvas and so cannot be watched (SQ-0708).
    ///
    /// Only a `draw_picture` into a window canvas paints: an erase is a fill, and a
    /// window-0 draw at the text cursor is a transcript float that never touches
    /// `pictures_canvas` at all. A picture whose art will not resolve paints nothing
    /// either — pacing a frame the renderer cannot tell apart from the last one is
    /// just a stall.
    ///
    /// The hold is proportional to the area painted, in the unit space the canvas
    /// uses (art-native dims doubled by [`V6_ART_SCALE`]) — which is effectively
    /// what the original hardware did, and why a full plate reads as visibly slower
    /// than a small icon.
    fn picture_hold(&mut self, ev: &PictureEvent) -> Option<std::time::Duration> {
        if ev.erase || ev.number == 0 || self.is_win0_inline_float(ev) {
            return None;
        }
        let (w, h) = self.pict_source.as_mut()?.dims(ev.number as u32)?;
        let area = (w as u64 * self.art_scale.0 as u64) * (h as u64 * self.art_scale.1 as u64);
        let ms = (area / PACE_PX_PER_MS).clamp(PACE_MIN_MS, PACE_MAX_MS);
        Some(std::time::Duration::from_millis(ms))
    }

    /// Whether a picture event is an INLINE TRANSCRIPT FLOAT — art the game meant
    /// to flow with window 0's prose — rather than art it placed on the window.
    ///
    /// Two things have to hold. First, the game has to have meant the picture to
    /// belong to the prose, which it declares in one of two ways.
    ///
    /// The engine's [`PictureEvent::at_cursor`] says the picture landed on window
    /// 0's current text line (SQ-0695), which is what separates Zork Zero's
    /// drop-caps and Shogun's opening ship from Arthur's centred plates.
    ///
    /// But `at_cursor` is a pixel-exact `y == y_cursor`, and a game that offsets
    /// its inline art by a pixel or two inside the line fails it. Zork Zero does
    /// exactly that when its art comes off the original Amiga floppy rather than a
    /// Blorb: it reads placement picture 478 through `picture_data` and adds it to
    /// the cursor, and that placeholder is `2×1` in the native `Pic.data` where the
    /// Blorb's `Rect` is `0×0` — so the drop-cap is drawn at cursor + (4, 2) in unit
    /// space and every drop-cap and room icon fell through to the canvas path,
    /// vanishing from the transcript entirely (SQ-0741).
    ///
    /// So [`PictureEvent::margin_after`] counts too: a `set_margins` issued on this
    /// window immediately after the draw is ZMSD §15's margin-picture idiom, i.e.
    /// the game reserving the column the prose is to flow in. **That is the game
    /// saying "text flows beside this picture" in as many words** — a stronger
    /// statement of intent than a coordinate that happens to match, and one no
    /// placed backdrop in the corpus makes (Arthur, Journey, fmvpoker and
    /// mysterious01 all draw their plates with no margin call after).
    ///
    /// Intent alone is not enough: `erase_window` puts the cursor
    /// back at (1,1), so a game that erases the screen and then paints a backdrop
    /// at (1,1) — fmvpoker's 640×400 poker table, Journey's opening illustration,
    /// mysterious01's title art — draws "at the cursor" by pure coincidence
    /// (SQ-0714). Reading those as floats left `pictures_canvas` empty, so the
    /// model published no `Graphics` leaf for window 0 at all and the art only
    /// ever appeared as a transcript image in the text flow.
    ///
    /// So the second test asks what a float actually IS: **a float flows beside
    /// text.** A picture that spans window 0's full width leaves no column for
    /// text to flow in, and cannot be one — it is placed art, and belongs on the
    /// window canvas with the prose composited over it.
    ///
    /// Measured across the v6 corpus (unit pixels; picture dims doubled by
    /// [`V6_ART_SCALE`] to match the window box, which is already unit space):
    ///
    /// | game        | picture      | draw x | picture w | window w | out_chars | verdict |
    /// |-------------|--------------|--------|-----------|----------|-----------|---------|
    /// | Zork Zero   | 2 (drop-cap) | 1      | 84        | 468      | 1         | float   |
    /// | Zork Zero   | 216 (icon)   | 1      | 42        | 468      | 270       | float   |
    /// | Zork Zero¹  | 2 (drop-cap) | 5      | 84        | 464      | 1         | float   |
    /// | Zork Zero¹  | 216 (icon)   | 5      | 42        | 464      | 270       | float   |
    /// | Shogun      | 7 (ship)     | 229    | 320       | 548      | 526       | float   |
    /// | Journey     | 160          | 1      | 640       | 640      | 37        | canvas  |
    /// | fmvpoker    | 99           | 1      | 640       | 640      | 151       | canvas  |
    /// | mysterious01| 33           | 1      | 512       | 640      | 0         | canvas  |
    ///
    /// ¹ the same game booted off its Amiga `.adf`, art from the native archive.
    ///
    /// The width gap is not a tuned threshold — the widest float covers 58% of its
    /// window, the narrowest canvas that the WIDTH arm decides covers 100%.
    ///
    /// **mysterious01 is the row the width arm cannot reach, and the reason the
    /// `out_chars` column is here** (SQ-0722). Its title card is 512 px in a 640 px
    /// window, so it spans neither the window nor a threshold anyone would want to
    /// defend — 80% of the window, against Shogun's 58%. This row USED to read
    /// `1024` and pass comfortably, because every v6 picture was doubled by
    /// [`V6_ART_SCALE`]; SQ-0715/SQ-0718 gave the story its true `art_scale` of 1
    /// (its Blorb has no `Reso` chunk, so Blorb §11 draws it 1:1) and the product
    /// silently halved. Nothing in the width arm noticed, and the card fell back to
    /// being a float — into the transcript, where it scrolled away with the prose.
    /// A measured table is only as good as the quantity it measures staying put.
    ///
    /// So the intent test above carries the real discriminator for this row, and it
    /// costs no threshold at all: with **nothing ever streamed to window 0**,
    /// `at_cursor` is comparing against a cursor no text ever moved.
    ///
    /// Art that will not resolve keeps the old answer: neither path paints
    /// anything for it, and guessing a canvas would only create an empty one.
    fn is_win0_inline_float(&mut self, ev: &PictureEvent) -> bool {
        self.win0_inline_float_x(ev).is_some()
    }

    /// The same question, answering with the picture's `x` **inside window 0** —
    /// which is the coordinate the float's alignment is read from, and which is
    /// not `ev.x` when the game drew into some other window (SQ-0888).
    fn win0_inline_float_x(&mut self, ev: &PictureEvent) -> Option<u16> {
        if ev.window != 0 {
            return self.ceded_margin_float_x(ev);
        }
        // `at_cursor` is evidence only when the cursor means something. It is a
        // pixel-exact `y == y_cursor`, and with NOTHING ever streamed to window 0
        // the cursor is simply where `erase_window` left it — home. A picture
        // drawn there matches a position no text ever produced, so the match
        // carries no intent at all (SQ-0722). `margin_after` is unconditional:
        // a `set_margins` is the game speaking, not a coordinate coinciding.
        let intent = (ev.at_cursor && ev.out_chars > 0) || ev.margin_after.is_some();
        if !intent {
            return None;
        }
        let win_w = self
            .machine
            .screen
            .v6
            .as_ref()
            .and_then(|v6| v6.windows.first())
            .map_or(0, |w| u32::from(w.x_size));
        let Some((pic_w, _)) = self.pict_source.as_mut().and_then(|s| s.dims(ev.number as u32))
        else {
            return Some(ev.x);
        };
        // Spans the window: starts at (or left of) its left edge and reaches the
        // right one. Window coords are 1-based (ZMSD §8.8.1).
        let spans_window = ev.x <= 1 && pic_w * self.art_scale.0 >= win_w.max(1);
        (!spans_window).then_some(ev.x)
    }

    /// SQ-0888: the same margin picture, spelled by a game that paints it from a
    /// GRAPHICS window instead of from window 0 — answering with the picture's `x`
    /// inside window 0, or `None` when the event is not that.
    ///
    /// Shogun says one layout two ways. Its Amiga press (release 295 / serial
    /// 890321) draws picture 7 into window 0 at (229,1) and calls `set_margins`
    /// right after, which the arm above already reads as ZMSD §15's margin picture;
    /// its Apple IIgs press (`shogun_s1.dsk`, release 311 / serial 890510) draws
    /// the SAME picture 7 into WINDOW 6 — a graphics window at (1,33) 560x352 that
    /// contains window 0 outright — at (249,1), and then calls
    /// `set_margins(0, 320, win 0)`. Both reserve about 320 px of the same window
    /// for the same ship on the same scene. Only the window number differs.
    ///
    /// The window number was read as a difference in KIND, and it is a difference
    /// in SPELLING. The reference rendition settles it: on the original the ship
    /// **scrolls up with the prose** and the text wraps around its bottom, which is
    /// exactly what a window-0 margin float does here — and is exactly what art
    /// pinned to a window canvas cannot do. So the picture is routed to the same
    /// float, and the Amiga's frame is the acceptance criterion for the Apple's.
    ///
    /// THREE THINGS HAVE TO HOLD, and between them they are why this cannot fire
    /// on a game that merely happens to have a graphics window open:
    ///
    /// 1. The drawing window CONTAINS window 0. A window that merely overlaps it is
    ///    a neighbour; one that encloses it is being used as the surface window 0's
    ///    own prose sits on.
    /// 2. Window 0 holds a `set_margins` reservation *right now* — the margin in
    ///    force, not one the event carries. [`PictureEvent::margin_after`] is
    ///    `None` here, because the engine only attaches a `set_margins` issued on
    ///    the SAME window as the draw, and this game's lands on window 0 while
    ///    window 6 is current.
    /// 3. The picture lies ENTIRELY inside the column that reservation gave up, and
    ///    overlaps window 0 vertically. This is the whole of the safety: Zork Zero
    ///    and the Amiga Shogun run `left_margin` 2 / `right_margin` 2 on every
    ///    gameplay frame, and no picture in the corpus fits in a 2 px column.
    ///
    /// An erase paints no float — it is a fill of the window canvas, and has to
    /// stay one.
    fn ceded_margin_float_x(&mut self, ev: &PictureEvent) -> Option<u16> {
        if ev.erase || ev.number == 0 || ev.window == 1 {
            return None;
        }
        let v6 = self.machine.screen.v6.as_ref()?;
        let w0 = v6.windows.first()?;
        let (w0x, w0y) = (i64::from(w0.x_coord.max(1)), i64::from(w0.y_coord.max(1)));
        let (w0w, w0h) = (i64::from(w0.x_size), i64::from(w0.y_size));
        let (left, right) = (i64::from(w0.left_margin), i64::from(w0.right_margin));
        if w0w <= 0 || w0h <= 0 || left + right >= w0w {
            return None;
        }
        // (1) the drawing window contains window 0's box. `win_box` is the box AT
        // THE MOMENT OF THE CALL (SQ-0715), which is the one the picture was
        // clipped to.
        let (dx, dy) = (i64::from(ev.win_box.0.max(1)), i64::from(ev.win_box.1.max(1)));
        let (dw, dh) = (i64::from(ev.win_box.2), i64::from(ev.win_box.3));
        if dx > w0x || dy > w0y || dx + dw < w0x + w0w || dy + dh < w0y + w0h {
            return None;
        }
        // (3) the picture's box on screen, against the text column and the ceded ones.
        let (pw, ph) = self.pict_source.as_mut()?.dims(u32::from(ev.number))?;
        let (pw, ph) = (i64::from(pw * self.art_scale.0), i64::from(ph * self.art_scale.1));
        let px0 = dx + i64::from(ev.x.max(1)) - 1;
        let py0 = dy + i64::from(ev.y.max(1)) - 1;
        if py0 + ph <= w0y || py0 >= w0y + w0h {
            return None; // nothing of it is beside window 0's prose at all
        }
        let in_right = right > 0 && px0 >= w0x + w0w - right && px0 + pw <= w0x + w0w;
        let in_left = left > 0 && px0 >= w0x && px0 + pw <= w0x + left;
        if !(in_right || in_left) {
            return None;
        }
        u16::try_from(px0 - w0x + 1).ok()
    }

    // ── The paced picture sequence, as the app loop drives it (SQ-0708) ───────

    /// How long the picture frame now on screen is held before the next one lands
    /// — `None` when nothing is playing and the screen already shows the settled
    /// composite.
    pub fn paced_picture_hold(&self) -> Option<std::time::Duration> {
        self.paced_frames.front().map(|f| f.hold)
    }

    /// Advance the sequence one step. `true` when the screen changed, so the caller
    /// knows to redraw; `false` when there was nothing playing.
    pub fn advance_paced_pictures(&mut self) -> bool {
        self.paced_frames.pop_front().is_some()
    }

    /// Collapse the sequence to its settled composite at once — the player pressed
    /// a key, the pane resized, the game is being saved. `true` when frames were
    /// dropped (the screen jumps to the final state).
    ///
    /// Nothing is lost: the settled composite is already built, and these frames
    /// were only ever the way there.
    pub fn settle_paced_pictures(&mut self) -> bool {
        let had = !self.paced_frames.is_empty();
        self.paced_frames.clear();
        had
    }

    /// The window canvases the screen is showing RIGHT NOW: the in-flight paced
    /// frame while a picture sequence plays, else the settled composite. Every
    /// other reader — save, archive, palette replay, `/dump-windows` — wants
    /// `pictures_canvas` itself, which is always the finished screen.
    fn visible_canvas(&self) -> &std::collections::HashMap<u8, crate::graphics::Canvas> {
        self.paced_frames.front().map_or(&self.pictures_canvas, |f| &f.canvas)
    }

    /// The Current Palette's generation, or 0 with no Pict source.
    fn palette_gen(&self) -> u64 {
        self.pict_source.as_ref().map_or(0, |s| s.palette_gen())
    }

    /// A window's 1-based screen origin `(x, y)`, as the game set it — floored at
    /// 1 exactly as [`PictureEvent::win_box`] does, so the two are comparable.
    fn window_origin(&self, win: u8) -> Option<(u16, u16)> {
        let v6 = self.machine.screen.v6.as_ref()?;
        let w = v6.windows.get(win as usize)?;
        Some((w.x_coord.max(1), w.y_coord.max(1)))
    }

    /// Freeze a window's canvas onto the screen if the window has MOVED since the
    /// pixels were drawn (SQ-0715). `now` is the window's origin as of the event
    /// being applied.
    ///
    /// ZMSD §8: plotting is clipped to the current window, but what shows through
    /// "is plotted onto the screen", and "subsequent movements of the window do
    /// not move what was printed". Our per-window canvas is drawn at the window's
    /// *current* origin, which is a faithful model only while the window stays
    /// where it was — so the moment the origin changes, the old pixels have to be
    /// handed to the screen's [painted ground](GameSession::paint) at the
    /// coordinates they were painted at, and taken off the canvas.
    ///
    /// scopa.z6 is the game that cannot survive without this. Its `drawpic`
    /// borrows window 3 as a scratch pad for every single picture —
    ///
    /// ```text
    /// @move_window 3 y x;  @window_size 3 1000 1000;
    /// ws = WinSet(3);  @draw_picture pic 1 1;  WinSet(ws);
    /// ```
    ///
    /// — so the Neapolitan and Sicilian card decks were drawn into a window that
    /// was somewhere else before the frame ever rendered, and then erased outright
    /// by the next `fastsimplebox` fill. Only the vector deck, which is built from
    /// `erase_window` fills and already paints the ground, reached the player.
    ///
    /// A window that has NOT moved is left entirely alone: its canvas is still a
    /// faithful model, and a redraw or an erase in place must keep working as it
    /// always has (Shogun's title splash has to vanish when the game erases the
    /// window it is sitting in).
    fn retire_stranded_canvas(&mut self, win: u8, now: (u16, u16)) {
        let Some(anchor) = self.canvas_anchor.get(&win).copied() else { return };
        if now == anchor.origin {
            return; // a redraw in place — the canvas is still telling the truth
        }
        self.canvas_anchor.remove(&win);
        let Some(canvas) = self.pictures_canvas.remove(&win) else { return };
        // Nothing else can reproduce these pixels once they leave the canvas, so
        // the window's replay list goes with them.
        self.display_ops.remove(&win);
        self.unreplayable.remove(&win);
        let (sw, sh) = self.v6_native_extent();
        let (ox, oy) = (
            u32::from(anchor.origin.0.max(1)) - 1,
            u32::from(anchor.origin.1.max(1)) - 1,
        );
        let src = canvas.img;
        let ground = self
            .paint
            .get_or_insert_with(|| std::sync::Arc::new(image::RgbaImage::new(sw, sh)));
        let dst = std::sync::Arc::make_mut(ground);
        let (rx, ry, rw, rh) = anchor.rect;
        for y in ry..(ry + rh).min(src.height()) {
            for x in rx..(rx + rw).min(src.width()) {
                let px = *src.get_pixel(x, y);
                if px[3] == 0 {
                    continue; // never drawn — the ground underneath still shows
                }
                let (dx, dy) = (ox + x, oy + y);
                if dx < dst.width() && dy < dst.height() {
                    dst.put_pixel(dx, dy, px);
                }
            }
        }
    }

    /// Record that `win`'s canvas now holds pixels drawn while the window sat at
    /// `origin`, covering `(dx, dy, w, h)` in canvas coords (SQ-0715).
    fn anchor_canvas_draw(&mut self, win: u8, origin: (u16, u16), dx: i32, dy: i32, w: u32, h: u32) {
        let (x, y) = (dx.max(0) as u32, dy.max(0) as u32);
        let entry = self.canvas_anchor.entry(win).or_insert(CanvasAnchor {
            origin,
            rect: (x, y, w, h),
        });
        let (ax, ay, aw, ah) = entry.rect;
        let (x0, y0) = (ax.min(x), ay.min(y));
        let (x1, y1) = ((ax + aw).max(x + w), (ay + ah).max(y + h));
        entry.origin = origin;
        entry.rect = (x0, y0, x1 - x0, y1 - y0);
    }

    /// The screen rect a v6 window occupies — `(x, y, w, h)` in the same 0-based
    /// unit-pixel space the window canvases use (window coords are 1-based).
    fn window_screen_rect(&self, win: u8) -> Option<(u32, u32, u32, u32)> {
        let v6 = self.machine.screen.v6.as_ref()?;
        let w = v6.windows.get(win as usize)?;
        Some((
            w.x_coord.saturating_sub(1) as u32,
            w.y_coord.saturating_sub(1) as u32,
            w.x_size as u32,
            w.y_size as u32,
        ))
    }

    /// Erase a SCREEN rect from every window canvas except `skip` (SQ-0568).
    ///
    /// v6 windows are clipping regions over ONE shared screen, not independent
    /// surfaces: plotting is "clipped to the current window, and anything showing
    /// through is plotted onto the screen", and "subsequent movements of the window
    /// do not move what was printed" (ZMSD §8). A plotted pixel therefore belongs to
    /// the screen, so erasing a region has to take out whatever ANY window put
    /// there. Arthur is the case that needs it: its F2 map screen paints a
    /// full-screen background into window 7, and returning to the picture screen
    /// erases windows 2/5/6 — never 7 — so that background has to disappear from
    /// window 7's canvas or it sits under every later screen for the rest of the game.
    ///
    /// The erase is recorded in each affected window's display list too, so a later
    /// palette replay reproduces it rather than restoring pixels the game removed.
    fn erase_screen_rect(&mut self, rect: (u32, u32, u32, u32), skip: Option<u8>) {
        let (rx, ry, rw, rh) = rect;
        if rw == 0 || rh == 0 {
            return;
        }
        let wins: Vec<u8> = self.pictures_canvas.keys().copied().collect();
        for win in wins {
            if Some(win) == skip {
                continue;
            }
            let Some((wx, wy, _, _)) = self.window_screen_rect(win) else { continue };
            // Intersect against the CANVAS extent (which `grow_to` may have taken
            // past the window box), then translate into that canvas's own coords.
            let Some((cw, ch)) = self.pictures_canvas.get(&win).map(|c| (c.img.width(), c.img.height()))
            else {
                continue;
            };
            let x0 = rx.max(wx);
            let y0 = ry.max(wy);
            let x1 = (rx + rw).min(wx + cw);
            let y1 = (ry + rh).min(wy + ch);
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            let (ex, ey, ew, eh) = ((x0 - wx) as i32, (y0 - wy) as i32, x1 - x0, y1 - y0);
            let canvas = self.pictures_canvas.get_mut(&win).expect("checked just above");
            // NOT a z_seq bump: an erase is not a draw, and re-stamping the layer
            // here would reorder the composite for a window nothing was drawn into.
            canvas.erase_rect(ex, ey, ew, eh);
            self.record_op(win, V6Op::Erase { dx: ex, dy: ey, w: ew, h: eh });
        }
    }

    /// Append an op to a window's display list, dropping the window out of replay if
    /// it overflows the cap. A whole-canvas op supersedes everything before it, so the
    /// list resets there — which is what keeps a screen-swapping story (Arthur) short.
    fn record_op(&mut self, win: u8, op: V6Op) {
        let full = self
            .pictures_canvas
            .get(&win)
            .map(|c| (c.img.width(), c.img.height()))
            .is_some_and(|(cw, ch)| match op {
                V6Op::Erase { dx, dy, w, h } => dx <= 0 && dy <= 0 && w >= cw && h >= ch,
                V6Op::Draw { .. } => false,
            });
        let ops = self.display_ops.entry(win).or_default();
        if full {
            ops.clear();
            self.unreplayable.remove(&win);
        } else if let V6Op::Erase { dx, dy, w, h } = op {
            // SQ-0592: an erase paints the window background over its rect, so every
            // EARLIER erase lying entirely inside it contributes nothing to the final
            // canvas and can go. This is the `full` reset above generalized from "covers
            // the whole canvas" to "covers that op's rect" — and it is what keeps the
            // list proportional to what is ON the screen rather than to how long the
            // session has run.
            //
            // Shogun is the case that needs it: it re-erases the same two regions every
            // turn, reaching 266 ops by turn 200 while holding only 7 distinct ones, and
            // would cross V6_OPS_CAP around turn 390 — at which point the window drops
            // out of palette replay for the rest of the session.
            //
            // Only earlier ERASES are pruned. An earlier draw under this rect is dead
            // too, but proving it needs the picture's scaled footprint, and keeping it
            // is harmless: replay order is preserved, so the erase still covers it.
            ops.retain(|prev| match *prev {
                V6Op::Erase { dx: pdx, dy: pdy, w: pw, h: ph } => {
                    let inside = pdx >= dx
                        && pdy >= dy
                        && pdx.saturating_add(pw as i32) <= dx.saturating_add(w as i32)
                        && pdy.saturating_add(ph as i32) <= dy.saturating_add(h as i32);
                    !inside
                }
                V6Op::Draw { .. } => true,
            });
        }
        if ops.len() >= V6_OPS_CAP {
            self.unreplayable.insert(win);
            return;
        }
        ops.push(op);
    }

    /// Replay every window's display list under the Current Palette (SQ-0567).
    ///
    /// The canvas is cleared and rebuilt op by op, so the result is what the screen
    /// would look like if the new palette had been loaded all along: every picture
    /// recoloured, every erase still erased, and — the part a plain replot got wrong —
    /// everything covering what it covered before. Arthur's map background is drawn
    /// over its frame, and must stay over it.
    ///
    /// Decoding goes through [`PictSource::image_under_current_palette`], which
    /// splices the live palette into each picture without letting it establish a new
    /// one: on real hardware the framebuffer holds indices, so a picture already
    /// drawn shows through whatever palette is loaded now, base or adaptive alike.
    fn replay_under_current_palette(&mut self) {
        let mut wins: Vec<u8> = self.display_ops.keys().copied().collect();
        wins.sort();
        for win in wins {
            if self.unreplayable.contains(&win) {
                continue;
            }
            let Some(ops) = self.display_ops.get(&win).cloned() else { continue };
            let Some((cw, ch)) = self.pictures_canvas.get(&win).map(|c| (c.img.width(), c.img.height()))
            else {
                continue;
            };
            if let Some(c) = self.pictures_canvas.get_mut(&win) {
                c.erase_rect(0, 0, cw, ch);
            }
            for op in ops {
                match op {
                    V6Op::Erase { dx, dy, w, h } => {
                        if let Some(c) = self.pictures_canvas.get_mut(&win) {
                            c.erase_rect(dx, dy, w, h);
                        }
                    }
                    V6Op::Draw { number, dx, dy } => {
                        let Some(img) = self
                            .pict_source
                            .as_mut()
                            .and_then(|s| s.image_under_current_palette(number as u32))
                        else {
                            continue;
                        };
                        let img = v6_scaled_art(&img, self.art_scale);
                        if let Some(c) = self.pictures_canvas.get_mut(&win) {
                            c.draw_image_clipped(&img, dx, dy, (cw, ch));
                        }
                    }
                }
            }
        }
    }

    /// Apply one `PictureEvent` to `pictures_canvas`. The event's `(y, x)` are
    /// the spec's 1-based window-relative pixel coords (zero already resolved to
    /// the window cursor by the engine); the canvas is 0-based, so both drop by
    /// one. The canvas is sized to the window's own pixel box and all plotting
    /// is CLIPPED to it — ZMSD §8: "all text and graphics plotting is always
    /// clipped to the current window". (The pre-Rect-support canvas grew to fit
    /// out-of-window draws; those coords were garbage from failed `picture_data`
    /// placement queries, not real game intent.) Erase paints the *picture's
    /// own* footprint (ZMSD §15), falling back to the whole window when the
    /// Pict's dims can't be resolved. Silently no-ops when the story has no v6
    /// window state, the window index is out of range, or (draw only) the
    /// picture fails to resolve.
    fn apply_picture_event(&mut self, ev: &PictureEvent) {
        // The window may have MOVED since its canvas was painted, in which case
        // those pixels belong to the screen where they were drawn and not to
        // wherever the window has gone (ZMSD §8, SQ-0715). Freeze them onto the
        // painted ground before this event touches the canvas.
        self.retire_stranded_canvas(ev.window, (ev.win_box.0, ev.win_box.1));
        // number 0 + erase = a v6 `erase_window`'s canvas-clear, riding the
        // ordered picture queue (so "erase, then draw the borders" replays in
        // order). Drop the whole canvas: Shogun's title splash must actually
        // vanish when the game erases window 7 before drawing the menu frame.
        if ev.erase && ev.number == 0 {
            self.pictures_canvas.remove(&ev.window);
            self.canvas_anchor.remove(&ev.window); // nothing left to strand
            // Nothing of this window survives to be replayed.
            self.display_ops.remove(&ev.window);
            self.unreplayable.remove(&ev.window);
            // The erased region belongs to the shared SCREEN, so take it out of
            // every other window's canvas too (SQ-0568).
            if let Some(rect) = self.window_screen_rect(ev.window) {
                self.erase_screen_rect(rect, Some(ev.window));
                // …and record what the erase PAINTED there (SQ-0584). ZMSD §8.8.5.3:
                // the rect is filled with the window's background colour, opaquely —
                // dropping the canvas above only removes what was under it. A fill of
                // the region a window no longer covers is dead the moment the window
                // moves or shrinks, so it is clamped to the live rect when published.
                let bg = self
                    .machine
                    .screen
                    .v6
                    .as_ref()
                    .and_then(|v6| v6.windows.get(ev.window as usize))
                    .map(|w| crate::state::pack_zcolour(w.bg))
                    .unwrap_or(0);
                let (x, y, w, h) = rect;
                if w > 0 && h > 0 {
                    self.window_fills.insert(
                        ev.window,
                        WindowFill {
                            x,
                            y,
                            w,
                            h,
                            bg,
                            seq: crate::graphics::next_draw_seq(),
                            out_chars: ev.out_chars,
                        },
                    );
                } else {
                    self.window_fills.remove(&ev.window);
                }
            }
            return;
        }
        // Decided before the window borrow below, since it consults `pict_source`
        // for the picture's own width (see `is_win0_inline_float`).
        let inline_float_x = self.win0_inline_float_x(ev);
        let Some(v6) = self.machine.screen.v6.as_ref() else { return };
        if v6.windows.get(ev.window as usize).is_none() {
            return;
        }
        // Snapshot window 0's margin/size state (pixels) so the win0-picture
        // classifier below can detect a right-margin float without holding a
        // borrow across the `pict_source` mutable borrow. The margins reflect the
        // `set_margins` the game issued right after the draw (ZMSD §15 margin
        // picture) — captured here at drain time.
        //
        // WINDOW 0's, not the drawing window's: the Apple Shogun paints its margin
        // picture from window 6 and reserves the column on window 0 (SQ-0888), and
        // reading window 6's own (always zero) margins would classify the ship as a
        // drop-cap. For every window-0 draw the two are the same window.
        let Some(w0) = v6.windows.first() else { return };
        let (win0_left_margin, win0_right_margin, win0_x_size) =
            (w0.left_margin, w0.right_margin, w0.x_size);
        // Window 0 is the main scrolling text window, and a picture drawn ON ITS
        // CURRENT TEXT LINE is INLINE story content (Zork Zero's drop-caps and
        // room icons, drawn at the text cursor with a margin set for the text to
        // flow beside them; Shogun's opening ship, `y = 0` with a right margin).
        // Those anchor to the transcript at their output-char position rather
        // than painting a window canvas — the raster/hybrid renderers float them
        // beside the text they belong to, and they scroll with it.
        //
        // A picture the game placed SOMEWHERE ELSE in window 0 is not inline at
        // all: it is absolutely placed, and falls through to the ordinary window
        // canvas below like any other window's art (SQ-0695). Arthur's intro is
        // the case — for each illustrated screen it erases every window, reads
        // window 0's own size (`get_wind_prop` props 2/3 → 400×640), centres the
        // 584×392 plate itself (x = (640−584)/2+1 = 29, y = (400−392)/2+1 = 5) and
        // draws it there while the text cursor is still at (1,1), then narrates
        // OVER it; the Merlin screen redraws the same plate and composites
        // picture 3 inside it at (81,51). Floating those as transcript images
        // discarded the placement the game had just computed, stacked the two
        // plates as separate full-width bands instead of compositing them, and
        // left `pictures_canvas` empty for the whole intro — so the model
        // published no `Graphics` leaf and the art never rasterized at all.
        //
        // A picture that SPANS window 0's full width is not inline either, even
        // when it lands on the cursor's own line: nothing can flow beside it. See
        // `is_win0_inline_float` for the corpus measurements (SQ-0714).
        if let Some(float_x) = inline_float_x {
            if ev.erase {
                return; // no canvas to erase; a win0 erase_picture is a no-op here
            }
            if let Some(img) = self.pict_source.as_mut().and_then(|s| s.image(ev.number as u32)) {
                // A window-0 picture is normally a drop-cap / room icon floated at
                // the left margin (text flows beside it). But Shogun draws its
                // large opening SHIP illustration into window 0 too — a
                // content-art-sized image must render as a big inline picture, not
                // a 3–4-row left-margin drop-cap (SQ-0471). Classify by size: a
                // content-art image (Shogun's ship) aligns InlineUp (full-size,
                // its own band); a genuine drop-cap (Zork Zero's initial letter,
                // a small tile) keeps MarginLeft.
                // Scale into unit space (SQ-0479) so the float renders at its
                // authentic 2× size beside the 8×16 text and its reserved rows
                // (height/16) stay consistent with the 16px grid.
                let img = v6_scaled_art(&img, self.art_scale);
                let (iw, ih) = (img.width(), img.height());
                let (screen_w, screen_h) = self.v6_screen_px();
                let align = win0_pic_align(
                    iw, ih, screen_w, screen_h,
                    float_x, win0_left_margin, win0_right_margin, win0_x_size,
                );
                let margin_px = match align {
                    // MarginLeft carries the game's own left `set_margins` value
                    // (text-start x); MarginRight reserves the picture's own cell
                    // width on the right (no cross-space margin scaling needed).
                    crate::inline_image::ImageAlign::MarginLeft => ev.margin_after.map(|m| m as u32),
                    _ => None,
                };
                let float = crate::inline_image::InlineImage {
                    pixels: std::sync::Arc::new(img.to_rgba8()),
                    align,
                    scaled: None,
                    margin_px,
                };
                self.story_pics.push((ev.out_chars, float));
            }
            return;
        }
        // Clamp the pixel-canvas backing store so a hostile / buggy story that
        // sets window_size(w, 0xFFFF, 0xFFFF) then draws/erases can't force a
        // ~17 GB RgbaImage allocation (an OOM abort). CANVAS_PX_CAP (4096) far
        // exceeds any real v6 screen (~640 px) yet bounds worst-case storage to
        // ~64 MB — mirroring the grid-cell cap on the engine side (Phase 1a).
        const CANVAS_PX_CAP: u32 = 4096;
        // The window's box AT THE MOMENT OF THE CALL, not now: a scratch window is
        // resized for one drawing operation and moved on (SQ-0715). scopa sizes
        // window 3 to 1000×1000 for each card and had shrunk it to an 80×1 sliver
        // by the time this ran, clipping every picture out of existence.
        let (pw, ph) = (
            (ev.win_box.2.max(1) as u32).min(CANVAS_PX_CAP),
            (ev.win_box.3.max(1) as u32).min(CANVAS_PX_CAP),
        );
        // 1-based window-relative → 0-based canvas coords.
        let dx = (ev.x.max(1) as i32) - 1;
        let dy = (ev.y.max(1) as i32) - 1;
        let canvas = self.pictures_canvas.entry(ev.window)
            .or_insert_with(|| crate::graphics::Canvas::new(pw, ph));
        // Track the window's current box without wiping earlier draws: grow
        // preserves content; a shrunken window only tightens the clip below
        // (window_size "does not change the current display", ZMSD §15).
        canvas.grow_to(pw, ph);
        // An erase_picture footprint to take out of the OTHER windows' canvases too,
        // applied once the canvas borrow above has ended (SQ-0568). Window-relative;
        // translated to screen coords at the bottom of this function.
        let mut screen_erase: Option<(u32, u32, u32, u32)> = None;
        if ev.erase {
            // Picture dims are art-native; the canvas is unit space, so scale the
            // erased footprint by V6_ART_SCALE to match the doubled draw (SQ-0479).
            let dims = self
                .pict_source
                .as_mut()
                .and_then(|s| s.dims(ev.number as u32))
                .map(|(w, h)| (w * self.art_scale.0, h * self.art_scale.1));
            let (ew, eh) = dims.unwrap_or((pw, ph));
            // Clip the erase to the window box.
            let ew = ew.min(pw.saturating_sub(dx.max(0) as u32));
            let eh = eh.min(ph.saturating_sub(dy.max(0) as u32));
            canvas.erase_rect(dx, dy, ew, eh);
            self.record_op(ev.window, V6Op::Erase { dx, dy, w: ew, h: eh });
            // …and the same region of the shared screen (SQ-0568).
            screen_erase = Some((dx.max(0) as u32, dy.max(0) as u32, ew, eh));
        } else if let Some(img) = self.pict_source.as_mut().and_then(|s| s.image(ev.number as u32)) {
            // Blit the art at 2× into the unit-space window canvas (SQ-0479): the
            // game placed it at unit coords (dx,dy) expecting the Amiga/DOS
            // doubled picture, so the scaled pixels fill the box the game reserved.
            let img = v6_scaled_art(&img, self.art_scale);
            canvas.draw_image_clipped(&img, dx, dy, (pw, ph));
            canvas.z_seq = crate::graphics::next_draw_seq();
            // Remember where on the SCREEN these pixels landed, so a later
            // `move_window` freezes them there instead of dragging them along
            // (SQ-0715).
            let drawn_w = img.width().min(pw.saturating_sub(dx.max(0) as u32));
            let drawn_h = img.height().min(ph.saturating_sub(dy.max(0) as u32));
            self.anchor_canvas_draw(
                ev.window,
                (ev.win_box.0, ev.win_box.1),
                dx,
                dy,
                drawn_w,
                drawn_h,
            );
            // Every picture goes in the display list, base or adaptive: under a new
            // palette they ALL recolour, and their order is what a replay has to
            // reproduce. (SQ-0567)
            self.record_op(ev.window, V6Op::Draw { number: ev.number, dx, dy });
            // SQ-0461 decision 3 ALSO anchored a transcript inline band here for a
            // large CONTENT-art draw into a graphics window (Shogun's title
            // splash), marked `ImageSource::ContentSplash`, so that the frameless
            // mode — which drops graphics windows — could still show it. It was
            // the only consumer: hybrid and raster both render the window canvas
            // itself and had to SKIP the band to avoid drawing the art twice.
            // SQ-0895 removed the mode, so the band had no reader left and is no
            // longer emitted; `is_content_art` survives because the window-0
            // margin/inline classifier above still asks it the same question.
        }
        // Apply the deferred cross-window erase now the canvas borrow has ended:
        // translate the window-relative footprint into screen coords (SQ-0568).
        if let Some((ex, ey, ew, eh)) = screen_erase {
            if let Some((wx, wy, _, _)) = self.window_screen_rect(ev.window) {
                self.erase_screen_rect((wx + ex, wy + ey, ew, eh), Some(ev.window));
            }
        }
    }

    /// The reported v6 screen size in pixels (header words 0x22/0x24), falling
    /// back to the classic 320×200 standard window when unset. (SQ-0461)
    fn v6_screen_px(&self) -> (u32, u32) {
        let w = self.machine.mem.read_word(0x22) as u32;
        let h = self.machine.mem.read_word(0x24) as u32;
        (if w == 0 { 320 } else { w }, if h == 0 { 200 } else { h })
    }

    /// The ON-SCREEN part of a v6 window's box, in native pixels (SQ-0710) —
    /// `(w, h)` clipped so `pos + size` never reaches past the screen.
    ///
    /// ZMSD §8.4.3 puts the v6 screen's size in header words $22 (width) and $24
    /// (height), in units — pixels for v6 — and `zvm::screen` seeds both at boot.
    /// Every window the game opens lives inside that box; a `window_size` past it
    /// describes nothing a player can see.
    ///
    /// Games use `window_size` as a MEASURING instrument, not only as layout.
    /// scopa's own Inform source sizes window 5 to 1000×1000 so a string it is
    /// about to measure cannot wrap:
    ///
    /// ```text
    /// textwidth [ txt ws tw;  @window_size 5 1000 1000;  ws = WinSet(5); ...
    /// ```
    ///
    /// Taken literally, that one window ballooned the composite's extent to
    /// 1579×1370 once a hand was dealt, scaling the whole picture down inside the
    /// pane (the report's "the screen zooms out") with black bands wherever the
    /// oversized page painted outside the real screen.
    ///
    /// The clip is deliberately on the PUBLISHED box, never on the VM's stored
    /// size. `get_wind_prop` has to keep reporting what the game wrote (ZMSD
    /// §8.8.3.2) — Shogun centres its title from window 0's prop 3, Arthur reads
    /// props 2/3 — and scopa's own measurement is only correct while the window it
    /// measures in really is 1000px wide, so clamping in zvm would break the very
    /// idiom that motivates this. zvm stores the size verbatim; the renderer draws
    /// the part of it that exists. That is also how `native_extent` already treats
    /// Shogun's unresolved size sentinel (SQ-0481) — and a size with the high bit
    /// set IS that sentinel (a small negative leaked as a large `u16`), so it is
    /// passed through untouched rather than handed a plausible-looking clip.
    fn v6_clip_box(&self, x_px: u16, y_px: u16, x_size: u16, y_size: u16) -> (u16, u16) {
        // An unseeded header word is not a 1×1 screen: clip to nothing rather than
        // to a guess, so a story that never got one keeps today's box.
        let unset = |v: u16| if v == 0 { u16::MAX } else { v };
        let clip = |size: u16, pos: u16, screen: u16| -> u16 {
            if (size as i16) < 0 {
                return size; // unresolved sentinel — not a size to clip (SQ-0481)
            }
            size.min(screen.saturating_sub(pos))
        };
        (
            clip(x_size, x_px, unset(self.machine.mem.read_word(0x22))),
            clip(y_size, y_px, unset(self.machine.mem.read_word(0x24))),
        )
    }

    /// Build the v6 z-ordered layered [`ScreenModel`] from `screen.v6`'s 8-window
    /// table plus `pictures_canvas` (Plan 1b Task 2). Called from `Engine::screen`
    /// when the story has v6 window state; the v1–5 `screen_model_from_machine`
    /// path is untouched and stays byte-identical for non-v6 stories.
    ///
    /// Per window 0..8, skipped when `x_size == 0 || y_size == 0`: absolute cell
    /// rect = `(x_coord/FW, y_coord/FH, grid.cols, grid.rows)` — the grid was
    /// already cell-sized at `window_size` time (Phase 1a), so only the position
    /// needs dividing by the font cell size. Window 0 is the scrolling main
    /// window (`Buffer{primary:true}`, drawn from `state.transcript`); windows
    /// 1–7 become `Grid` leaves built from their own char grid (mirrors
    /// `screen_model_from_machine`'s `UpperWindow`→`GridWindow` mapping). Any
    /// window with an entry in `pictures_canvas` ALSO gets a `Graphics` leaf at
    /// the same rect. z-order (list order): graphics entries first (background),
    /// then text windows by ascending window number — `render_node`'s `Layered`
    /// arm (Task 3) paints text over graphics, cell-text-wins.
    /// `/dump-windows` for a v6 story: ONE BLOCK PER WINDOW.
    ///
    /// Three things have to agree for a v6 layout to be right — the game's window
    /// table, what the model made of each window, and where the renderer put it on
    /// the terminal — and a defect is nearly always a disagreement between two of
    /// them. Reporting them as three parallel lists left the reader correlating by
    /// eye; this reports each window once, with all three.
    ///
    /// `cells` is the last frame's mapping (`AppState::v6_cell_map`), matched back to
    /// windows by native rect. Empty when the caller has none — the engine-only view
    /// still shows the game and model halves.
    ///
    /// `face` is the live [`crate::native_font::TextFace`] — `AppState::v6_text`,
    /// which is where it lives because a style reload rebuilds it (SQ-1047). `None`
    /// for the engine-only view, which says so rather than reporting a default face
    /// nobody is drawing with.
    pub fn v6_window_dump(
        &self,
        cells: &[crate::state::V6CellRect],
        face: Option<&crate::native_font::TextFace>,
    ) -> Vec<String> {
        let Some(v6) = self.machine.screen.v6.as_ref() else {
            return Vec::new();
        };
        let model = self.screen();
        let items: &[PositionedWindow] = match &model.root {
            WinNode::Layered(items) => items,
            _ => &[],
        };
        let find = |label: &str| cells.iter().find(|c| c.label == label);
        let fmt_cells = |c: (u16, u16, u16, u16)| format!("{}x{} at ({},{})", c.2, c.3, c.0, c.1);

        let mut out = Vec::new();
        let mut head = format!("v6 layout — current window {}, input window {}", v6.current, self.machine.screen.v6_input_window);
        if let Some(scale) = find("scale") {
            let (s100, off_y, cw, ch) = scale.native;
            head.push_str(&format!(", scale {:.2}, cell {cw}x{ch}px, y-offset {off_y}", s100 as f32 / 100.0));
        }
        if let Some(path) = cells.iter().find(|c| c.label.starts_with("path:")) {
            head.push_str(&format!("\n  render path: {}", path.label.trim_start_matches("path:")));
        }
        out.push(head);
        if let Some(pane) = find("pane") {
            let vp = find("viewport").map(|v| fmt_cells(v.cells)).unwrap_or_else(|| "—".into());
            out.push(format!("  pane {} · story viewport {vp}", fmt_cells(pane.cells)));
        }
        out.extend(v6_face_lines(face));

        for (i, w) in v6.windows.iter().enumerate() {
            if w.x_size == 0 && w.y_size == 0 && w.texts.is_empty() && w.prose.is_empty() {
                continue;
            }
            // The model publishes the ON-SCREEN part of the box (SQ-0710), so the
            // rect this matches nodes and cells by has to be clipped the same way
            // — or a window the game oversized reads as "not published" here while
            // it is on screen. The game's own size stays on the `win` line below,
            // which is where a reader looking for what the game asked for goes.
            let (cw, chh) = self.v6_clip_box(
                w.x_coord.saturating_sub(1),
                w.y_coord.saturating_sub(1),
                w.x_size,
                w.y_size,
            );
            let native = (w.x_coord.saturating_sub(1), w.y_coord.saturating_sub(1), cw, chh);
            let mut flags = Vec::new();
            if w.wrapping() { flags.push("wrap"); }
            if w.scrolling() { flags.push("scroll"); }
            if w.copy_to_transcript() { flags.push("transcript"); }
            if w.attributes & 0b1000 != 0 { flags.push("buffered"); }
            let mut marks = Vec::new();
            if v6.current as usize == i { marks.push("current"); }
            if self.machine.screen.v6_input_window as usize == i { marks.push("input"); }
            // The flag names spell the attribute bits out, so the raw value rides the
            // detail line below — this one has to stay short enough not to wrap.
            out.push(format!(
                "  win{i}  {}x{} at ({},{})  [{}]{}",
                w.x_size,
                w.y_size,
                w.x_coord,
                w.y_coord,
                flags.join(" "),
                if marks.is_empty() { String::new() } else { format!("  <- {}", marks.join("+")) },
            ));
            out.push(format!(
                "          game: attrs={:04b}, {} paint run(s), {} prose line(s){}",
                w.attributes,
                w.texts.len(),
                w.prose.len(),
                // SQ-0710: say so when the game sized a window past the screen —
                // otherwise the `win` line above reads 1000x1000 and every rect
                // under it reads 61x30, with nothing to explain the difference.
                if (cw, chh) == (w.x_size, w.y_size) {
                    String::new()
                } else {
                    format!(" — off-screen, clipped to {cw}x{chh}")
                },
            ));
            // **What the window's own text metrics are** (SQ-1009). A frame
            // capture answers where a glyph LANDED; only this answers where the
            // engine thought it could put one, and the two disagreeing is the
            // whole class of v6 layout defect. It was absent when Arthur's F5
            // description wrapped past its window, and the question — does the
            // paint path break this line on a column count or a pixel width —
            // had to be answered by reading `cpu::exec` instead of by looking.
            //
            // `usable` is the width a wrap SHOULD respect: ZMSD §8.8.3.2's
            // properties 6 and 7 inset the text from both edges, so a window
            // whose margins are non-zero is narrower than its `x_size` says.
            let cell = self.machine.v6_cell();
            out.push(format!(
                "          text: grid {}x{} cells of {}x{}px · margins l={} r={} · usable {}px = {} cols",
                w.grid.cols,
                w.grid.rows,
                cell.w(),
                cell.h(),
                w.left_margin,
                w.right_margin,
                w.x_size.saturating_sub(w.left_margin).saturating_sub(w.right_margin),
                w.x_size
                    .saturating_sub(w.left_margin)
                    .saturating_sub(w.right_margin)
                    / cell.w().max(1),
            ));
            // **What the WINDOW says it is set in** (SQ-1047). ZMSD §8.8.3.2
            // properties 12 and 13: the font number, and the size as
            // `(height << 8) | width`. The engine seeds both from the declared cell
            // at `restart_screen` and restates them on every `set_v6_text`, so a
            // window that disagrees with the cell is a window the GAME re-sized —
            // Shogun reads the width back out of prop 13 to size its input buffer.
            // Printed per window because it is per window; the FACE those metrics
            // came from is one launch fact and is reported once, above.
            let (fw, fh) = (w.font_size & 0xff, w.font_size >> 8);
            out.push(format!(
                "          font: number {} · size {fw}x{fh}px (props 12/13){}",
                w.font_number,
                if (fw, fh) == (cell.w(), cell.h()) {
                    String::new()
                } else {
                    format!(" <- not the {}x{} cell", cell.w(), cell.h())
                },
            ));
            // What the model made of this window, matched by its native rect.
            let node = items.iter().find(|pw| (pw.x_px, pw.y_px, pw.w_px, pw.h_px) == native);
            out.push(match node.map(|pw| &pw.node) {
                Some(WinNode::Buffer(b)) if b.primary => "          model: Buffer — the story window".to_string(),
                Some(WinNode::Buffer(b)) => format!("          model: Buffer — panel, {} line(s)", b.lines.len()),
                Some(WinNode::Grid(g)) => format!(
                    "          model: Grid — {} run(s){}",
                    g.px_texts.len(),
                    if g.fill.is_some() { ", erase fill live" } else { "" }
                ),
                Some(WinNode::Graphics(_)) => "          model: Graphics".to_string(),
                Some(_) => "          model: ?".to_string(),
                None => "          model: not published (empty placeholder)".to_string(),
            });
            // …and where the last frame put it.
            let placed: Vec<&crate::state::V6CellRect> =
                cells.iter().filter(|c| c.native == native && c.label != "scale").collect();
            if placed.is_empty() {
                out.push(if cells.is_empty() {
                    "          cells: (no frame recorded — render one first)".to_string()
                } else {
                    "          cells: NOT DRAWN this frame".to_string()
                });
            } else {
                for p in placed {
                    out.push(format!("          cells: {} as {}", fmt_cells(p.cells), p.label));
                }
            }
        }

        // SQ-0755: the bottom-anchored MENU band's strips are listed too. They were
        // not, and the omission cost a whole investigation: at the user's 159x61 pane
        // the ring's strips stop at the story viewport's bottom (row 49) and the only
        // other line in the dump was a one-row band at row 61 — eleven rows of the
        // pane that nothing in the dump claimed. They belong to `menu:text`, which is
        // classified through a different scale from the ring's and so was filtered out
        // by a `strip:` prefix test. A dump that cannot account for the whole pane
        // invites the reader to invent an explanation for the gap.
        let strips: Vec<&crate::state::V6CellRect> = cells
            .iter()
            .filter(|c| c.label.starts_with("strip:") || c.label.starts_with("menu:"))
            .collect();
        if !strips.is_empty() {
            out.push("  chrome ring strips:".to_string());
            for s in strips {
                out.push(format!("    {} {}", s.label, fmt_cells(s.cells)));
            }
        }
        out
    }

    /// Build the v6 model over a given set of window canvases: `pictures_canvas`
    /// for the settled screen ([`Engine::screen`]), or an in-flight frame of the
    /// turn's picture sequence for what is on screen right now
    /// ([`Engine::screen_now`], SQ-0708). Everything else about the model — window
    /// geometry, text, z-order — is read from the live `screen.v6` table either way,
    /// so the two differ only in which pixels the `Graphics` leaves carry.
    fn v6_screen_model(&self, visible: &std::collections::HashMap<u8, crate::graphics::Canvas>) -> ScreenModel {
        // SQ-0917: the session's own cell, which every native-pixel-to-cell step
        // below divides by. The engine placed these runs with it, so the model has
        // to recover them with the same number.
        let (font_w, font_h) = (self.machine.v6_cell().w(), self.machine.v6_cell().h());
        let screen = &self.machine.screen;
        let v6 = screen.v6.as_ref().expect("caller checked screen.v6.is_some()");

        // Z-order: ALL graphics first (background), then ALL text on top — the
        // v6 decorative frame (Zork0's window 7 border) sits BEHIND the page
        // text, never over it. Within each band, ascending window number
        // (window 1+ overlays paint after window 0). The pixel compositor and the
        // Phase 1b cell fallback both honour this order.
        // Graphics carry their global draw-order stamp so the composite can be
        // sorted by DRAW ORDER (later draw on top), not window number — the frame
        // background (drawn first) sits behind the overlays the game paints after
        // it (compass, room illustration). (SQ-0186)
        let mut graphics_entries: Vec<(u64, PositionedWindow)> = Vec::new();
        let mut text_entries = Vec::new();
        // The window the game actually streams prose through (SQ-0583). Window 0 is
        // the classic answer and Infocom's own, but Inform 6's v6 library prints into
        // WINDOW 7 — the engine already diverts its output to the transcript on the
        // same wrap+scroll test (SQ-0459, `cpu::exec`), so the model has to agree
        // about WHERE that prose lands or every consumer of the story rect points at
        // the wrong region. advent.z6 shows the cost: it never touches window 0, so
        // window 0 keeps its boot-time full-screen rect forever, and after the game
        // splits the screen (its "style" command opens a text window on top, moves
        // the status bar to the middle and leaves prose in the bottom 200px) the
        // story rect still claimed the whole screen — transcript viewport, chrome
        // ring and mapper band all aimed at it.
        //
        // …and a prose window whose text the engine DIVERTS to its own buffer is not
        // that window (SQ-0746): it is a display panel, and publishing it as the
        // primary Buffer hands the renderer a story window with no lines in it (a
        // primary Buffer's prose is the host transcript by construction) while the
        // text the game actually printed sits unread in the panel. fmvpoker is the
        // report — it prints "Enter the new bet: " into its bottom panel and reads
        // the bet through it — and the two sides must answer with one rule or the
        // model contradicts the engine that filled it.
        let prose_idx = {
            let cur = v6.current as usize;
            if v6.windows[cur].attributes & 0b11 == 0b11 && !self.machine.v6_diverts_prose(cur) { cur } else { 0 }
        };
        for (i, win) in v6.windows.iter().enumerate() {
            // Window 0 with nothing of its own, while the prose streams elsewhere:
            // it is a boot-time placeholder covering the screen (see below), and
            // admitting it would hand `classify_windows` a second, wrong story rect.
            if i == 0 && prose_idx != 0 && win.texts.is_empty() && !visible.contains_key(&0) {
                continue;
            }
            // A zero-pixel-size window is normally inactive and skipped — UNLESS
            // it still holds painted text runs. v6 text is PAINT: a run persists
            // at its screen-absolute pixel position even after the window is
            // resized to zero (ZMSD §15, "window_size does not change the current
            // display"). Journey's bottom command menu is a full-width window
            // sized to HEIGHT 0 that paints "Proceed/Back/Game" plus the party
            // and verb columns via absolute runs at native rows 19–24; dropping
            // the window here loses the entire menu (SQ-0492).
            if (win.x_size == 0 || win.y_size == 0) && win.texts.is_empty() {
                continue;
            }
            // ZMSD §8.8.3.3 boots window 0 "occupying the whole screen", and an
            // `erase_window(-1)` unsplit restores exactly that. Until the game
            // gives it content, though, a full-screen window 0 is a placeholder,
            // not a story page: `render::v6_layout::classify_windows` would make
            // it the story window, and hybrid mode would then open a pane-sized
            // transcript viewport over the title art with a zero-thickness chrome
            // ring around it (Shogun's splash goes blank). So skip window 0 while
            // it still covers the untouched whole screen with nothing in it —
            // neither painted runs nor a single character streamed to it. The
            // moment the game prints or resizes/moves it (Zork Zero sizes it to
            // 468x320 during boot) it takes part again.
            if i == 0
                && win.texts.is_empty()
                && self.machine.v6_win0_out_chars == 0
                && !visible.contains_key(&0)
                && (win.x_coord, win.y_coord) == (1, 1)
                && (win.x_size, win.y_size) == (self.machine.mem.read_word(0x22), self.machine.mem.read_word(0x24))
            {
                continue;
            }
            // ZMSD §8.8.1: window coords are 1-based ((1,1) = screen top-left);
            // the composite raster is 0-based, so positions drop by one here.
            let x_px = win.x_coord.saturating_sub(1);
            let y_px = win.y_coord.saturating_sub(1);
            let x = x_px / font_w;
            let y = y_px / font_h;
            let (cols, rows) = (win.grid.cols, win.grid.rows);
            // …and the box the renderer gets is the part of it that is ON SCREEN
            // (SQ-0710) — see `v6_clip_box`.
            let (w_px, h_px) = self.v6_clip_box(x_px, y_px, win.x_size, win.y_size);

            if let Some(canvas) = visible.get(&(i as u8)) {
                graphics_entries.push((canvas.z_seq, PositionedWindow {
                    x,
                    y,
                    w: cols,
                    h: rows,
                    x_px,
                    y_px,
                    w_px,
                    h_px,
                    left_margin: win.left_margin,
                    right_margin: win.right_margin,
                    node: WinNode::Graphics(GraphicsWindow {
                        win: i as u32,
                        canvas: canvas.arc(),
                        version: canvas.version,
                        upscale: false,
                    }),
                }));
            }

            // RETIRED PROSE (SQ-0697). The prose window publishes as a `Buffer`,
            // whose node carries no pixel runs — the transcript is its text. But a
            // window that has been moved or resized leaves the prose it already
            // printed painted where it was (ZMSD §15), and the engine hands those
            // runs to `texts` when that happens. Publish them as their own paint
            // layer so they still render: Shogun's title header stays up top,
            // centred on the columns the game declared, while the transcript
            // starts again in the 548x64 box it moved window 0 down to.
            //
            // The entry's BOX is the frozen prose's own extent, not the window's:
            // the window has moved on, and a layer claiming the window's new box
            // would be read as an overlay strip sitting inside the story and push
            // the live transcript down out of it (hybrid's `overlay_strip`).
            if !win.retired.is_empty() {
                let (mut rx0, mut ry0, mut rx1, mut ry1) = (u16::MAX, u16::MAX, 0u16, 0u16);
                for t in &win.retired {
                    // **The prose's own extent is the PEN's, not the declared
                    // cell's** (SQ-1062). This measured `chars * cell.w`, which is
                    // `V6Cell::run_px` — documented there as "uniform on purpose,
                    // even for a machine that painted proportionally". That is the
                    // right number for the box a GAME reserved, and the wrong one
                    // here: the comment three lines above says this box is "the
                    // frozen prose's own extent, not the window's", and a retired
                    // entry has no game-declared box to inherit.
                    //
                    // It matters because `build_chrome_canvas` turns this `w_px`
                    // into the proportional pen's CLIP BOUND, and floods
                    // `fill_explicit_bg_rows` to the same width. On Arthur's Amiga
                    // press the face advances ~10.4 native px against a declared 8,
                    // so the box under-measured its own runs by about a quarter and
                    // the tail of every frozen line fell outside the bound. Every
                    // fixed-pen machine is byte-identical, because `V6Metric::advance`
                    // answers `cell.w` for every style there — which is why this
                    // outlived the SQ-1054 sweep.
                    let w = self
                        .machine
                        .v6_metric
                        .run_px(&t.text, t.style)
                        .min(u32::from(u16::MAX)) as u16;
                    rx0 = rx0.min(t.x.saturating_sub(1));
                    ry0 = ry0.min(t.y.saturating_sub(1));
                    rx1 = rx1.max(t.x.saturating_sub(1).saturating_add(w));
                    ry1 = ry1.max(t.y.saturating_sub(1).saturating_add(font_h));
                }
                text_entries.push(PositionedWindow {
                    x: rx0 / font_w,
                    y: ry0 / font_h,
                    w: (rx1 - rx0).div_ceil(font_w),
                    h: (ry1 - ry0).div_ceil(font_h),
                    x_px: rx0,
                    y_px: ry0,
                    w_px: rx1 - rx0,
                    h_px: ry1 - ry0,
                    left_margin: win.left_margin,
                    right_margin: win.right_margin,
                    node: WinNode::Grid(GridWindow {
                        cols: 0,
                        rows: 0,
                        cells: Vec::new(),
                        active_rows: 0,
                        cursor: (1, 1),
                        cursor_active: false,
                        border: BorderPref::Unspecified,
                        bg: None,
                        fg: None,
                        reverse: false,
                        fill: None,
                        px_texts: win
                            .retired
                            .iter()
                            .map(|t| crate::engine::PxText {
                                y: t.y,
                                x: t.x,
                                text: t.text.clone(),
                                style: t.style,
                                fg: crate::state::pack_zcolour(t.fg),
                                bg: crate::state::pack_zcolour(t.bg),
                                grow: t.grow,
                                gcol: t.gcol,
                            })
                            .collect(),
                    }),
                });
            }

            // Window 0 is normally the scrolling transcript Buffer — but with
            // its wrapping attribute CLEARED it is in positioned paint mode
            // (menu screens: Zork Zero's InvisiClues clears bit 0 and paints
            // topics via set_cursor), and its pixel runs render like any grid
            // window's.
            let node = if i == prose_idx && win.attributes & 1 != 0 {
                WinNode::Buffer(BufferWindow {
                    primary: true,
                    bg: (win.bg != ZColour::Default).then(|| crate::state::pack_zcolour(win.bg)),
                    fg: (win.fg != ZColour::Default).then(|| crate::state::pack_zcolour(win.fg)),
                    // Where this window's streamed prose is SITTING (SQ-0729) — see
                    // `BufferWindow::px_runs`. The transcript is still what renders
                    // it; this is the same text read as pixels, for the one window
                    // shape that is a canvas rather than a page.
                    px_runs: win
                        .streamed
                        .iter()
                        .map(|t| crate::engine::PxText {
                            y: t.y,
                            x: t.x,
                            text: t.text.clone(),
                            style: t.style,
                            fg: crate::state::pack_zcolour(t.fg),
                            bg: crate::state::pack_zcolour(t.bg),
                            grow: t.grow,
                            gcol: t.gcol,
                        })
                        .collect(),
                    ..Default::default()
                })
            } else if !win.prose.is_empty() {
                // A SECOND flowing-prose window (SQ-0585): the engine kept its text
                // out of the transcript stream and in the window, so publish it as a
                // non-primary Buffer — the same node Glulx uses for its secondary
                // text buffers. Live screen state: no scrollback, and the lines go
                // when the game erases the window.
                WinNode::Buffer(BufferWindow {
                    primary: false,
                    lines: win.prose.clone(),
                    runs: vec![Vec::new(); win.prose.len()],
                    para: vec![crate::state::ParaFmt::default(); win.prose.len()],
                    images: vec![None; win.prose.len()],
                    scroll: 0,
                    bg: (win.bg != ZColour::Default).then(|| crate::state::pack_zcolour(win.bg)),
                    fg: (win.fg != ZColour::Default).then(|| crate::state::pack_zcolour(win.fg)),
                    panel: false,
                    px_runs: Vec::new(),
                    // …and whether the player is typing into it (SQ-0746): a v6 game
                    // may read through a panel it has declared is not the transcript,
                    // and the host's echo belongs after that panel's own prompt.
                    reads_input: self.machine.screen.v6_input_window as usize == i,
                })
            } else {
                WinNode::Grid(GridWindow {
                    cols,
                    rows,
                    cells: win
                        .grid
                        .cells
                        .iter()
                        .map(|c| GridCell {
                            ch: c.ch,
                            style: c.style,
                            fg: crate::state::pack_zcolour(c.fg),
                            bg: crate::state::pack_zcolour(c.bg),
                            link: 0, // Z-machine grid cells carry no Glk hyperlink
                            glk_style: 0, // Z-machine is always Normal
                        })
                        .collect(),
                    active_rows: rows,
                    // The v6 window cursor is stored in 1-based PIXELS (ZMSD
                    // §8.8.3.2); the cell renderer wants 1-based cells.
                    cursor: (
                        (win.y_cursor.max(1) - 1) / font_h + 1,
                        (win.x_cursor.max(1) - 1) / font_w + 1,
                    ),
                    cursor_active: v6.current == i as u8,
                    border: BorderPref::Unspecified,
                    bg: (win.bg != ZColour::Default).then(|| crate::state::pack_zcolour(win.bg)),
                    fg: (win.fg != ZColour::Default).then(|| crate::state::pack_zcolour(win.fg)),
                    reverse: false,
                    // An erase fill still on top of everything painted here (SQ-0584):
                    // newer than the story's last prose, still covering the window's
                    // live rect (a window that moved or grew since leaves its old fill
                    // behind rather than dragging a stale blank around), and never the
                    // prose window itself — erasing THAT means "clear the transcript",
                    // which rides `erase_lower` instead and would otherwise blank the
                    // pane on every ordinary turn.
                    fill: self.window_fills.get(&(i as u8)).and_then(|f| {
                        let covers = f.x <= x_px as u32
                            && f.y <= y_px as u32
                            && f.x + f.w >= x_px as u32 + win.x_size as u32
                            && f.y + f.h >= y_px as u32 + win.y_size as u32;
                        (i != prose_idx && covers && f.out_chars == self.machine.v6_win0_out_chars)
                            .then_some(crate::engine::ErasedFill { bg: f.bg, seq: f.seq })
                    }),
                    // Exact pixel-positioned runs for the pixel raster (the
                    // cells above stay the cell-mode fallback).
                    px_texts: win
                        .texts
                        .iter()
                        .map(|t| crate::engine::PxText {
                            y: t.y,
                            x: t.x,
                            text: t.text.clone(),
                            style: t.style,
                            fg: crate::state::pack_zcolour(t.fg),
                            bg: crate::state::pack_zcolour(t.bg),
                            grow: t.grow,
                            gcol: t.gcol,
                        })
                        .collect(),
                })
            };
            text_entries.push(PositionedWindow {
                x,
                y,
                w: cols,
                h: rows,
                x_px,
                y_px,
                w_px,
                h_px,
                left_margin: win.left_margin,
                right_margin: win.right_margin,
                node,
            });
        }

        // Sort graphics by draw order (stable: equal stamps keep window order),
        // then drop the stamps — later-drawn windows now composite on top.
        graphics_entries.sort_by_key(|(seq, _)| *seq);
        let mut graphics_entries: Vec<PositionedWindow> =
            graphics_entries.into_iter().map(|(_, pw)| pw).collect();

        // content_size: the max right/bottom cell extent actually covered by a
        // window, or (when no window survived the size-0 skip) the header's
        // whole-screen char dims (0x21 cols / 0x20 rows) — either way nonzero,
        // so the v6 model always leaves the simple/degenerate render path.
        let mut max_x = 0u16;
        let mut max_y = 0u16;
        for pw in graphics_entries.iter().chain(text_entries.iter()) {
            max_x = max_x.max(pw.x + pw.w);
            max_y = max_y.max(pw.y + pw.h);
        }
        let degenerate = max_x == 0 || max_y == 0;
        let content_size = if degenerate {
            (
                self.machine.mem.read_byte(0x21) as u16,
                self.machine.mem.read_byte(0x20) as u16,
            )
        } else {
            (max_x, max_y)
        };

        // Inform 6's v6 library (SQ-0459) leaves every window at height 0 and
        // flows its prose through the transcript (its main window sets the
        // wrapping bit, so `print_text` streams rather than paints). The size-0
        // skip above therefore drops ALL windows, and raster mode would render
        // a blank screen. When nothing survived, synthesise a full-screen
        // primary Buffer so the streamed transcript still renders — Infocom v6
        // titles keep real nonzero windows and never reach this branch.
        //
        // "Nothing survived" is the question this branch asks, and it is NOT the
        // `degenerate` one above (SQ-0805): `degenerate` reads the CELL extent, and
        // a v6 window's cell size is its char grid, which a game that never resizes
        // window 0 off its boot rect never sets. sunburst.z6 is that game — window 0
        // reaches the model as 640x400 PIXELS and 0x0 cells — so the flag fired with
        // a real primary Buffer standing right there and published a SECOND one at
        // the same rect, which `classify_windows` then filed under chrome. The
        // `content_size` fallback above is the consumer that genuinely wants the
        // zero-extent test, so the two ask their own questions from here on.
        if text_entries.is_empty() && graphics_entries.is_empty() {
            text_entries.push(PositionedWindow {
                x: 0,
                y: 0,
                w: content_size.0,
                h: content_size.1,
                x_px: 0,
                y_px: 0,
                w_px: self.machine.mem.read_word(0x22),
                h_px: self.machine.mem.read_word(0x24),
                left_margin: 0,
                right_margin: 0,
                node: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
            });
        }

        graphics_entries.extend(text_entries);
        // `ScreenModel.bg`/`fg` is the PANE PAGE: `render_story_pane` floods the
        // whole story pane with it before anything is drawn. A Version 6 story
        // normally has no such thing — ZMSD §8.3 gives every window its own pair,
        // and each is published on its own node (`GridWindow.bg`,
        // `BufferWindow.bg`) while the region around the scaled 640x400 frame
        // belongs to the host theme. `screen.current_fg/current_bg` hold the
        // CURRENT WINDOW's pair (mirrored there so v6 prose runs carry the right
        // `TextAttrs`, §8.3) — publishing THAT as the page repainted the entire
        // pane in whatever window happened to be selected, e.g. flooding Zork
        // Zero's pane with its white window-0 background and burying the artwork.
        //
        // §8.3's Amiga machine is the one exception, and it is an exception in
        // exactly the way that comment describes: under the Amiga interpreter
        // number there is one pair for the whole screen, it is not the current
        // window's, and it does not move — so it is a page in the full sense the
        // field means. `zvm::screen::amiga_screen_pair` reads it back out of the
        // header ($2D/$2C), which is where §8.3.3 already had lanthorn publishing
        // it to the story; before SQ-0740 nothing painted it, so an Amiga and an
        // IBM PC rendered identically and the profile was invisible on screen.
        // The windows' own pairs still ride on their own nodes and still win —
        // this is the ground under them, not a replacement for them.
        //
        // SQ-0846: the Macintosh is the second machine of that kind, and it was
        // found the same way — by the profile being invisible on screen. See
        // [`machine_screen_pair`].
        let (fg, bg) = machine_screen_pair(&self.machine)
            .unwrap_or((zvm::screen::ZColour::Default, zvm::screen::ZColour::Default));
        ScreenModel {
            root: WinNode::Layered(graphics_entries),
            status: status_model_from_machine(&self.machine),
            bg: crate::state::pack_zcolour(bg),
            fg: crate::state::pack_zcolour(fg),
            content_size,
        }
    }

    /// The [`V6ModelKey`] for one build of the v6 model over `visible` — every
    /// fact [`GameSession::v6_screen_model`] reads, reduced to cheap compares.
    fn v6_model_key(&self, visible: &std::collections::HashMap<u8, crate::graphics::Canvas>) -> V6ModelKey {
        let mut canvases: Vec<(u8, u64, u64)> =
            visible.iter().map(|(w, c)| (*w, c.version, c.z_seq)).collect();
        canvases.sort_unstable();
        let mut fills: Vec<(u8, u64)> =
            self.window_fills.iter().map(|(w, f)| (*w, f.seq)).collect();
        fills.sort_unstable();
        V6ModelKey {
            generation: self.machine.screen.v6_generation(),
            cell: (self.machine.v6_cell().w(), self.machine.v6_cell().h()),
            input_window: self.machine.screen.v6_input_window,
            win0_out_chars: self.machine.v6_win0_out_chars,
            screen_px: (self.machine.mem.read_word(0x22), self.machine.mem.read_word(0x24)),
            screen_chars: (self.machine.mem.read_byte(0x20), self.machine.mem.read_byte(0x21)),
            page_pair: machine_screen_pair(&self.machine)
                .map(|(fg, bg)| (crate::state::pack_zcolour(fg), crate::state::pack_zcolour(bg))),
            canvases,
            fills,
        }
    }

    /// [`GameSession::v6_screen_model`], memoized behind [`V6ModelKey`]
    /// (SQ-1191). A frame whose key matches the held one gets the held `Arc`
    /// back — one key build and compare instead of deep-cloning every window's
    /// runs, grids and prose — and any mismatch rebuilds and replaces the memo,
    /// so the stored model always corresponds to the stored key.
    fn v6_screen_model_shared(
        &self,
        visible: &std::collections::HashMap<u8, crate::graphics::Canvas>,
    ) -> std::sync::Arc<ScreenModel> {
        let key = self.v6_model_key(visible);
        if let Some((held, model)) = self.v6_model_memo.borrow().as_ref() {
            if *held == key {
                return std::sync::Arc::clone(model);
            }
        }
        let model = std::sync::Arc::new(self.v6_screen_model(visible));
        *self.v6_model_memo.borrow_mut() = Some((key, std::sync::Arc::clone(&model)));
        model
    }
}

/// Everything [`GameSession::v6_screen_model`] READS, reduced to cheap compares
/// — the key for [`GameSession::v6_model_memo`] (SQ-1191).
///
/// The expensive tree (eight windows' runs, grids and prose) is stood for by
/// zvm's [`zvm::screen::ScreenState::v6_generation`], which advances with every
/// mutable borrow of the window table. Everything else the build consumes is a
/// cheap scalar read taken fresh per key, so a missed bump cannot hide in it:
///
/// - `cell`: the session's v6 cell (`Machine::v6_cell`) — every native→cell
///   division in the build;
/// - `input_window` / `win0_out_chars`: the prose-window choice and the
///   window-0 placeholder/fill-freshness tests;
/// - `screen_px` / `screen_chars`: header `$22`/`$24` and `$20`/`$21` — the
///   clip box and the degenerate `content_size` fallback;
/// - `page_pair`: [`machine_screen_pair`]'s ANSWER — keying on the output
///   covers the header bytes and the licence flag it reads, whoever wrote them;
/// - `canvases`: each visible canvas's `(version, z_seq)` — content and draw
///   order, and thereby WHICH map the caller handed in (the settled
///   `pictures_canvas` vs a paced in-flight frame, SQ-0708);
/// - `fills`: each window's erase fill by its draw-sequence stamp, which is
///   unique per insertion — a fill is immutable once recorded, and replacing
///   one takes a fresh stamp.
///
/// NOT in the key, and why each omission is sound: `v6_metric` is replaced
/// only by `Machine::set_v6_text`, which moves the generation on its way
/// through the window table; the status model is constant (`HostManaged` for
/// every v4+ story, v6 included); and `pack_zcolour` is pure tag-packing — the
/// palette resolves colours at RENDER time, behind the render's own content
/// key (SQ-1187), so a palette change never alters this model.
#[derive(PartialEq)]
struct V6ModelKey {
    generation: u64,
    cell: (u16, u16),
    input_window: u8,
    win0_out_chars: u64,
    screen_px: (u16, u16),
    screen_chars: (u8, u8),
    page_pair: Option<(u32, u32)>,
    canvases: Vec<(u8, u64, u64)>,
    fills: Vec<(u8, u64)>,
}

/// The `face:` block of `/dump-windows` — which TYPEFACE the metrics below came
/// from (SQ-1047).
///
/// # One block per frame, and it says so
///
/// Every other block in the dump is per window because its subject is. This one
/// is not: the face is a LAUNCH fact, resolved once by
/// [`crate::native_font::resolve`] from the medium the mount returned, and every
/// window on the screen is drawn with the answer. Printing it under each window
/// would invite the reader to look for a difference that cannot exist; leaving it
/// out entirely is what the dump did, and the cost was that a DISK-FONT defect
/// and a METRIC defect are indistinguishable by looking — the same wrong column
/// count comes back whether the wrong face was admitted or the right one was
/// measured wrong.
///
/// # What it has to answer
///
/// Which face is the body and which is the machine's fixed-pitch alternate (a
/// Macintosh has both, and off DIFFERENT media — SQ-1036); where each came from;
/// and the three numbers that differ on a real press and are routinely confused
/// for one another — the face's own size, the cell the story was DECLARED, and
/// the text scale between them (SQ-1039). Then the pen, because "proportional"
/// versus "steps the cell" is the first thing a wrap defect wants to know.
///
/// # The names it can give, and the one it cannot
///
/// [`crate::native_font::FaceOrigin`] already carries a system face's disk and
/// resource name, so nothing here re-derives provenance — the report asks the
/// cascade's own answer, the way `native_font::detected`'s `used` column does. A
/// RELEASE face has no name to give: `resolve` picks it through
/// `mac_font::from_volume_beside` / `amiga_font::from_volume`, which return the
/// face alone, and naming it here would mean a second copy of the pick — the
/// exact shape that shipped SQ-1011 inert twice. So it is reported by its medium
/// and its size, which is what the resolved value knows.
fn v6_face_lines(face: Option<&crate::native_font::TextFace>) -> Vec<String> {
    use crate::native_font::{FaceFit, FaceOrigin};
    let Some(face) = face else {
        return vec!["  face: not supplied — engine-only view".to_string()];
    };
    let from = |o: Option<&FaceOrigin>| match o {
        Some(FaceOrigin::Release) => "the release's own medium".to_string(),
        Some(FaceOrigin::SystemDisk { disk, name }) => format!("{disk} · {name}"),
        None => "nowhere".to_string(),
    };
    let (cell, scale) = (face.cell(), face.scale());
    let mut out = vec!["  face: one launch fact — every window below is set in it".to_string()];
    match face.face() {
        // No face admitted at all: the renderer is on the built-in, which is not a
        // failure and has to read as a resolved state rather than a missing line.
        None => out.push("    body: none — the built-in render::vga16".to_string()),
        Some(f) => out.push(format!(
            "    body: {}x{}px from {} · fit {}",
            f.width,
            f.height,
            from(face.faces().body_origin()),
            match face.fit() {
                Some(FaceFit::Metric) => "Metric",
                Some(FaceFit::Cell) => "Cell",
                None => "—",
            },
        )),
    }
    out.push(match face.faces().fixed() {
        Some(f) => format!(
            "    fixed: {}x{}px from {}",
            f.width,
            f.height,
            from(face.faces().fixed_origin())
        ),
        None => "    fixed: none — a fixed-pitch run takes the body face".to_string(),
    });
    out.push(format!(
        "    declared cell {}x{}px · text scale {}x{} native px per face px",
        cell.w(), cell.h(), scale.0, scale.1
    ));
    // The pen. A proportional one is reported by its RANGE over the printable set,
    // because that is the range a wrap is computed from — and by its bold smear,
    // which widens the advance rather than only the ink (SQ-1009).
    if face.proportional() {
        let (lo, hi) = (' '..='~').fold((u32::MAX, 0u32), |(lo, hi), c| {
            let a = face.advance(c);
            (lo.min(a), hi.max(a))
        });
        let smear = u32::from(face.bold_smear(crate::render::bitfont::STYLE_BOLD));
        out.push(format!(
            "    pen: proportional {lo}–{hi}px over printable ASCII · bold +{}px (smear {smear})",
            smear * scale.0
        ));
    } else {
        out.push(format!("    pen: steps the cell — {}px for every character", cell.w()));
    }
    out
}

/// The MACHINE's own screen pair for a Version 6 frame, `(foreground,
/// background)` — the ground every window that names no colour of its own is
/// read on — or `None` on a machine that has no such thing.
///
/// **This is [`zvm::screen::machine_screen_pair`] and nothing else** (SQ-0872).
/// It used to be half a rule: the Amiga's arm was in `zvm`, this one was the
/// Macintosh's, gated on a constant `blorb` happened to carry, and `zvm-cli`
/// could reach only the first of them. Both are now columns of `zvm`'s machine
/// table, so the two front-ends cannot present different machines. The whole
/// argument — why these two machines and not the Apple's equally real black page
/// — lives at the zvm function.
///
/// Kept as a named wrapper because the call site above is a paragraph of prose
/// about what `ScreenModel.bg`/`fg` mean, and it reads better pointing at a local
/// name than at a path.
fn machine_screen_pair(machine: &Machine) -> Option<(ZColour, ZColour)> {
    zvm::screen::machine_screen_pair(machine)
}

/// Convert a detected `Location` into the `ObjectSnapshot` used as a room id.
/// `NameOnly` (no backing object) gets a stable synthetic id from its name;
/// every other variant carries a real object. Shared by per-turn draining and
/// the startup seed so both assign the same room id.
fn location_to_snapshot(loc: &Location) -> zvm::ObjectSnapshot {
    match loc {
        Location::NameOnly(name) => zvm::ObjectSnapshot {
            number: crate::roomid::synthetic_room_id(name),
            parent: 0,
            name: name.clone(),
        },
        _ => loc.object().expect("non-NameOnly variants carry an object").clone(),
    }
}

// ── Mapper bridge ─────────────────────────────────────────────────────────────

/// The game's own echoed movement command at the head of a turn's transcript,
/// or `None`.
///
/// A compass click (SQ-0576) reaches the VM as a mouse terminator with no typed
/// text, but the game echoes the command it synthesized — Zork Zero prints
/// `north` alone on the first output line before the room text. Adopting that
/// echo lets a click-driven move map exactly like the typed command it stands
/// for.
///
/// Deliberately strict: the first non-empty line must BE a movement command in
/// its entirety — one word, or `go <word>` — that [`parse_direction`] accepts.
/// `parse_direction` matches the first token, so a bare prefix test would read
/// the room heading "North of House" as a move north; the whole-line rule
/// rejects it.
pub fn echoed_direction_command(transcript: &str) -> Option<&str> {
    let line = transcript.lines().find(|l| !l.trim().is_empty())?.trim();
    let mut tokens = line.split_whitespace();
    let first = tokens.next()?;
    let whole_line_is_command = match (tokens.next(), tokens.next()) {
        (None, None) => true,
        (Some(_), None) => first.eq_ignore_ascii_case("go"),
        _ => false,
    };
    (whole_line_is_command && parse_direction(line).is_some()).then_some(line)
}

/// Pure bridge: observe the new location (if any) into the mapper.
///
/// Calls `mapper.observe(snap.number, &snap.name, parse_direction(command))`.
/// In Auto mode, runs a light overlap cleanup (radius 2, max 20 passes) after each
/// observation so the live map never shows an illegal connector overlap.
/// No-op when `result.location` is `None`.
///
/// `death` is the caller's [`DeathWatch`], read and written here: this is where a death is
/// noticed, and where the resurrection that ends it is recognised — by the outstanding death
/// rather than by anything the turn says. It has to be the SAME watch every turn of a session
/// (the app keeps one on `AppState`); a fresh one per call is only correct for a seed or a
/// restore, where there is no previous turn to carry anything from.
pub fn apply_turn(
    mapper: &mut Mapper,
    command: &str,
    result: &TurnResult,
    death: &mut DeathWatch,
) {
    // A direction typed in this room has been TRIED, whatever else the turn did, so the
    // untried-exits overlay must stop offering it (SQ-0391). `Mapper::observe` records this for
    // the ordinary path, but three turns never reach it: one where no location was detected at
    // all, a suppressed unvalidated NameOnly, and a death that relocated the player. The last is
    // the one that matters most — walking north into a grue is the strongest possible evidence
    // that north has been tried. `mark_tried` is idempotent, so the overlap with `observe` costs
    // nothing.
    if let (Some(here), Some(d)) = (mapper.graph.current(), parse_direction(command)) {
        mapper.graph.mark_tried(here, d);
    }
    // Arm the death watch before anything reads it, so a turn that both reports a death and
    // relocates the player (the ordinary banner shape) still leaves the death unresolved: the
    // resurrection the game is about to offer has not happened yet. (SQ-0673)
    let fatal = turn_reports_death(&result.transcript);
    if fatal {
        death.unresolved = true;
    }
    if let Some(snap) = &result.location {
        let arrived = reprinted_room_heading(&result.transcript, &snap.name);
        // The map's FIRST room, when nothing in the object tree backs it, must be one the
        // STORY ITSELF named: the status line alone is not evidence. A pre-game
        // banner/menu/character-sheet paints a room-shaped status line and prints no matching
        // heading — BeyondZork's VT220 setup shows the player's name ("Frank Booth", beside
        // "Level 0 Male Peasant") in a status-line-shaped character sheet while the story text
        // says only "Press any key to begin the story." A real room is named twice, in two
        // independent channels: the status line and the game's own prose.
        //
        // This replaces the old rule, which waited for an OBJECT-BACKED room to seed the map
        // (SQ-0752). That presumed every story eventually produces one, and four titles never
        // do — so the gate was not a delay but a permanent mute, and their maps stayed empty
        // forever. The Impossible Bottle is compiled by Dialog, whose 492 objects carry no
        // short names at all; Facility.z8 and frankenfingers keep their room text outside the
        // object tree. All three paint the room on the status line and print it as a heading,
        // and all three were detected correctly by `detect_location` and then thrown away here.
        //
        // Corroboration is also the rule Glulx already uses — `RoomHeading` reads the room from
        // the story buffer and bypasses this gate for exactly this reason (a Glulx game never
        // produces an object-backed room either). The two engines now agree on the evidence.
        if result.location_method == Some(LocationMethod::NameOnly)
            && mapper.graph.rooms().next().is_none()
            && !arrived
        {
            return;
        }
        let moved_room = mapper.graph.current() != Some(snap.number);
        if fatal {
            // The game said the player died this turn and moved them somewhere that is NOT
            // reachable by the command they typed (e.g. a grue kills you in the dark and drops
            // you in the Forest). Record it as an involuntary relocation so no false directional
            // edge is minted from the room you died in to the resurrection room. (SQ-0259)
            //
            // Widened from the banner alone to whatever [`turn_reports_death`] recognises
            // (SQ-0671): Adventure prints no banner on the turn that kills you, so a fatal move
            // in the dark was minting a west-passage from the room you died in to the bottom of
            // the pit — verified against `advent.blb`.
            mapper.observe_relocation(snap.number, &snap.name);
        } else if death.unresolved && moved_room {
            // The death is still unresolved and the player has just changed rooms without dying
            // again: this is the resurrection, arriving on a turn that says nothing about death
            // at all. Adventure's is `yes` → *"--- POOF!! ---"* and the well house, several turns
            // after the pit that killed you, and it reprints the room heading like any walked
            // move — so without the watch it minted a `?` passage from the corpse's room to the
            // well house. Wherever a resurrection drops you is not a passage out of where you
            // died. (SQ-0673)
            mapper.observe_relocation(snap.number, &snap.name);
            death.unresolved = false;
        } else if arrived {
            // The game printed this room's heading again, so the player MOVED — even if they
            // came out where they went in. That is the only evidence a maze self-loop ever
            // leaves, and without it "west leads back here" is indistinguishable from walking
            // into a wall and thrown away (SQ-0666).
            mapper.observe_moved(snap.number, &snap.name, parse_direction(command));
            // A heading reprinted in the room the player is already standing in — a `look`, a
            // maze self-loop, a move the game refused with a re-description — is ordinary play
            // resuming, so whatever death was outstanding has been settled without a
            // resurrection: games that kill you in place and let you walk on look exactly like
            // this. The watch must not outlive it, or it would eat the player's next real
            // passage. (SQ-0673)
            death.unresolved = false;
        } else {
            // The conservative path: mint only if the location changed. With a death outstanding
            // this is reached only when it did NOT — "Please answer yes or no." is exactly this
            // shape — which is why the watch has to survive an unbounded number of turns rather
            // than just the one after the death. (SQ-0673)
            mapper.observe(snap.number, &snap.name, parse_direction(command));
        }
        // SQ-0527: remember how the mapper knew where the player was, the first
        // time each room is discovered, so the room inspector can show it. Kept on
        // the room rather than a transient corner indicator, which only ever
        // described the LAST detection and was gone by the time you wanted it.
        if let Some(m) = result.location_method {
            mapper.graph.set_loc_method(snap.number, crate::render::map::loc_method_label(m));
        }
        // Overlap cleanup is NOT done here: it is map-layout work and must never run
        // on the interpreter thread. On a geometry change the run loop schedules a
        // background cleanup (or full tidy) job — see `finish_command_turn` and
        // `cleanup_overlaps_layer_silent`. (SQ-0379)
    }
}

/// What the mapper still owes a death the game has not finished with (SQ-0671, SQ-0673).
///
/// A death is not one turn's event. Adventure kills you, offers to reincarnate you, insists
/// ("Please answer yes or no.") for as many turns as it takes, and only then either prints the
/// banner (you declined) or teleports you to the well house (you accepted). Everything the mapper
/// must not conclude in that window lives here, because none of it is recoverable from the graph
/// once the window has closed.
///
/// The two fields have deliberately different lifetimes — see each one. Session state only:
/// nothing here is worth persisting, and a restart or restore replaces it wholesale
/// ([`DeathWatch::default`]) because the game it described is gone.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DeathWatch {
    /// The `tried` record the last directional command left behind — `(room typed in, direction)`
    /// — held only while the player is still standing in that room (SQ-0671).
    ///
    /// Short-lived on purpose: it exists so a death admitted a turn or two late still rolls back
    /// the move that CAUSED it, and a player who has walked out of that room has settled the
    /// question themselves. See [`rollback_tried_on_death`].
    pub pending_tried: Option<(mapper::graph::RoomId, mapper::direction::Direction)>,
    /// A death has been reported and the game has not yet said how it ends (SQ-0673).
    ///
    /// Longer-lived than `pending_tried` by necessity: the yes/no exchange is unbounded, and the
    /// resurrection turn — the one that actually moves the player — carries no death vocabulary
    /// at all, so it is only recognisable as the first room change on this side of the flag. Set
    /// by any turn [`turn_reports_death`] recognises; cleared by the first turn that resolves it
    /// (see [`apply_turn`]) — a room change (the resurrection: relocation, no edge) or a heading
    /// reprinted in place (ordinary play resuming). It suppresses exactly one relocation, never a
    /// second.
    pub unresolved: bool,
}

/// The `tried` record this turn's command is about to create: `(the room the player is standing
/// in, the direction they typed)` — the pair [`apply_turn`] marks (SQ-0671).
///
/// `None` when the command names no direction, when there is no current room, or when the
/// direction was ALREADY on the record before this turn — in that last case the turn creates
/// nothing, so there is nothing it could roll back, and a rollback would erase an older, honest
/// fact. Call it BEFORE `apply_turn`, which is when "the room typed in" is still the current one.
pub fn tried_record_for(
    mapper: &Mapper,
    command: &str,
) -> Option<(mapper::graph::RoomId, mapper::direction::Direction)> {
    let here = mapper.graph.current()?;
    let dir = parse_direction(command)?;
    (!mapper.graph.is_tried(here, dir)).then_some((here, dir))
}

/// True when this turn's output says the player DIED (SQ-0671).
///
/// Two forms, because games admit a death in two ways:
///
/// * The `*** … ***` banner [`is_death_relocation`] reads — the Inform library default and the
///   Infocom convention, printed on the fatal turn itself in nearly every game.
/// * An offer to bring the player back: a death word AND a revival word in one turn. Adventure —
///   the game this whole maze feature is built around — prints no banner on the turn that kills
///   you at all. It prints *"You fell into a pit and broke every bone in your body! … you seem to
///   have gotten yourself killed … Do you want me to try to reincarnate you?"*, and the banner
///   only arrives later, IF the player declines. Requiring both words keeps it off a turn that
///   merely mentions raising the dead.
///
/// It gates both things a death turn must not do: mint a passage to wherever the player was
/// dumped ([`apply_turn`], SQ-0259) and record the direction as tried ([`rollback_tried_on_death`]).
///
/// It is also what arms [`DeathWatch::unresolved`], which covers the turns AFTER this one: the
/// turn that accepts the offer ("yes" → *"--- POOF!! ---"*, and the player wakes up in the well
/// house) carries no death word at all, and is recognised by the outstanding death rather than by
/// anything it says (SQ-0673).
pub fn turn_reports_death(transcript: &str) -> bool {
    is_death_relocation(transcript) || offers_revival(transcript)
}

/// True when this turn's output both reports a death and offers to undo it.
fn offers_revival(transcript: &str) -> bool {
    let lower = transcript.to_ascii_lowercase();
    let died = ["killed", "died", "dead", "your body"].iter().any(|w| lower.contains(w));
    let revive = ["reincarnate", "resurrect"].iter().any(|w| lower.contains(w));
    died && revive
}

/// Take back the `tried` record a fatal move left behind (SQ-0671).
///
/// A move that kills the player finds no passage: the game prints its death text, no room heading
/// is reprinted, no edge is minted — and yet the direction landed in the room's `tried` list,
/// where the matrix draws it as `×`, "tried, and there is no path that way". That is a claim about
/// the map nobody made. Dying tells you nothing about whether the way is open, so the record goes
/// back to `·` (untried) and the direction stays on the exploration frontier.
///
/// `attempted` is this turn's record from [`tried_record_for`]; [`DeathWatch::pending_tried`]
/// carries the previous turn's across the gap, because the death is not always admitted on the
/// turn that caused it — Adventure asks whether to reincarnate you first, and the banner arrives
/// on the turn that answers. It is only held while the player is still standing where they typed
/// the direction: a player who has moved on has settled the question themselves. (That is a
/// shorter life than [`DeathWatch::unresolved`], which has to survive the whole yes/no exchange
/// and the resurrection that ends it — the two answer different questions about the same death.)
///
/// Only the typed record is dropped. `MapGraph::unmark_tried` cannot unmint an edge, so a
/// direction that DID lead somewhere before the kill stays tried on the strength of that edge.
pub fn rollback_tried_on_death(
    mapper: &mut Mapper,
    death: &mut DeathWatch,
    attempted: Option<(mapper::graph::RoomId, mapper::direction::Direction)>,
    fatal: bool,
) {
    if fatal {
        // This turn's move when it named a direction, else the one held over — the turn that
        // CONTAINED the fatal move, which is the one whose record is a lie.
        if let Some((room, dir)) = attempted.or(death.pending_tried) {
            mapper.graph.unmark_tried(room, dir);
        }
        death.pending_tried = None;
        return;
    }
    if attempted.is_some() {
        death.pending_tried = attempted;
    } else if death.pending_tried.is_some_and(|(room, _)| Some(room) != mapper.graph.current()) {
        // The player left that room: whatever it recorded is now settled fact.
        death.pending_tried = None;
    }
}

/// True when this turn's output REPRINTED `name` as a room heading: a line of its own holding
/// exactly the room's name (SQ-0666).
///
/// It is the interpreter-independent signal that the player arrived somewhere — every engine here
/// prints the room heading on a successful move and prints a refusal ("You can't go that way.")
/// without one. That distinction is the whole difference between a maze's "west leads back here"
/// and a wall, and the mapper cannot make it: both leave the location unchanged.
///
/// Deliberately strict. A false positive invents a passage out of a wall; a false negative merely
/// leaves the direction recorded as tried, which is what happened to every self-loop before this
/// existed. A `look` reprints the heading too, but names no direction, so it mints nothing.
fn reprinted_room_heading(transcript: &str, name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && transcript.lines().any(|l| l.trim().eq_ignore_ascii_case(name))
}

/// True when this turn's output carries a death/end banner — the interpreter
/// convention of a `*** … ***` line (Inform's `*** You have died ***`, Infocom's
/// spaced `****  You have died  ****`). On such a turn a game may resurrect the
/// player into a room unrelated to the typed command, so the resulting room change
/// must be recorded as an involuntary relocation rather than a walked passage.
///
/// Kept deliberately tight — an asterisk-delimited banner line containing a death
/// word — so it never fires on ordinary room text that merely mentions the dead
/// (and it ignores the winning banner `*** You have won ***`, which changes no
/// room). Custom death banners without "died"/"dead" are a known gap. (SQ-0259)
fn is_death_relocation(transcript: &str) -> bool {
    transcript.lines().any(|line| {
        let t = line.trim();
        if t.len() < 4 || !t.starts_with("**") || !t.ends_with("**") {
            return false;
        }
        let lower = t.to_ascii_lowercase();
        lower.contains("died") || lower.contains("dead")
    })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Stop reason from `run_until_input`.
enum RunStop {
    /// VM is waiting for player input of this kind.
    Input(InputKind),
    /// VM ended via `@quit` (a mid-run `@restart` re-boots in place and keeps
    /// running, so it never surfaces here).
    Quit,
    /// VM suspended on its own `@save` — host must `resume_save`.
    SavePending,
    /// VM suspended on its own `@restore` — host must `resume_restore`.
    RestorePending,
}

/// Step until the machine pauses for input, quits, or suspends on its own
/// `@save`/`@restore`. In-game save/restore bubbles up as `SavePending`/
/// `RestorePending` for the host to service (all versions, v3 included).
fn run_until_input(machine: &mut Machine) -> RunStop {
    loop {
        match machine.step() {
            StepResult::Quit => return RunStop::Quit,
            StepResult::Fault => return RunStop::Quit,
            StepResult::NeedLine { .. } => return RunStop::Input(InputKind::Line),
            StepResult::NeedChar => return RunStop::Input(InputKind::Char),
            StepResult::SaveRequest => return RunStop::SavePending,
            StepResult::RestoreRequest => return RunStop::RestorePending,
            // @restart (ZMSD §6.1.3): re-boot the machine in place and keep
            // stepping — the game re-runs from its opening (v6 re-enters `main`),
            // so the turn returns a normal input request, NOT a quit. The
            // `just_restarted` flag lets the session drop stale v6 chrome in
            // `drain_turn`.
            StepResult::Restart => machine.restart(),
            StepResult::Continue => {}
        }
    }
}

/// Run to a player-facing stop — an input request or a quit — returning
/// `(pending_kind, quit)`.
///
/// A game `@save`/`@restore` reached along the way is auto-FAILED and the drive
/// continues, because the callers are the paths with no dialog to open: the boot
/// drive, and the two host restores (Save State, an `@save` archive loaded from
/// the saves manager). Leaving the VM suspended there would wedge the next turn,
/// and the suspension itself belongs to a run that is being replaced or has not
/// started. This is the Z-machine twin of the Glulx `drive_settled`, so the two
/// engines behave identically at these three points (SQ-0656).
fn run_settled(machine: &mut Machine) -> (InputKind, bool) {
    loop {
        match run_until_input(machine) {
            RunStop::Input(k) => return (k, false),
            RunStop::Quit => return (InputKind::Line, true),
            RunStop::SavePending => machine.complete_save(false),
            RunStop::RestorePending => machine.complete_restore_failure(),
        }
    }
}

/// Strip a trailing interactive read prompt from captured Z-machine output.
///
/// Infocom-style games print a bare ">" (possibly preceded by whitespace or a
/// newline, possibly followed by a space) as the last thing before issuing a
/// read/sread opcode.  When that output is captured we want to remove it so the
/// app's own fixed bottom input line is the only ">" the player sees.
///
/// The rule: trim trailing ASCII whitespace; if the result ends with ">" AND
/// that ">" is preceded by a newline or is the only character, remove it and
/// trim trailing whitespace again.  Any ">" that appears mid-sentence (e.g.
/// inside a score display like "score > 10") is unaffected because it will not
/// be the last non-whitespace character after a newline.
pub(crate) fn strip_read_prompt(s: &str) -> &str {
    let trimmed = s.trim_end_matches([' ', '\t']);
    // After stripping trailing spaces/tabs the string may still end with "\n>"
    // or just ">".  Check for that and strip.
    if let Some(without_gt) = trimmed.strip_suffix('>') {
        // Only strip if the ">" is at the start of a line (preceded by '\n')
        // or if it's the only character remaining.
        let preceded_by_newline = without_gt.ends_with('\n') || without_gt.is_empty();
        if preceded_by_newline {
            return without_gt.trim_end_matches([' ', '\t', '\n', '\r']);
        }
    }
    trimmed
}

/// Whether `s` ends with the read prompt [`strip_read_prompt`] would remove —
/// i.e. whether the game just handed the player its command prompt.
///
/// Defined in terms of `strip_read_prompt` rather than beside it so the two can
/// never drift: lanthorn has exactly one notion of "the game's read prompt", and
/// a game whose prompt this misses already shows the player a doubled prompt.
pub(crate) fn ends_with_read_prompt(s: &str) -> bool {
    strip_read_prompt(s).len() < s.trim_end_matches([' ', '\t']).len()
}

/// Downcast `machine.out` to `&mut CaptureSink`.
///
/// Panics if the machine was not built with a `CaptureSink` (should never
/// happen within this module since `GameSession::new` always installs one).
fn sink_mut(machine: &mut Machine) -> &mut CaptureSink {
    machine
        .out
        .as_any_mut()
        .downcast_mut::<CaptureSink>()
        .expect("GameSession machine must have a CaptureSink output")
}

/// Install a `ScreenState` restored from a host archive on `session`, re-syncing
/// the output sink's `buffer_mode` from it (ZMSD §7.2.1). The flag lives in two
/// places — the screen model and the sink that tags captured runs — and only the
/// former is archived, so a restore must hand it back to the sink or unbuffered
/// output would silently start word-wrapping again. (`@restart` re-syncs via
/// `init_caps`.)
///
/// Takes the whole `GameSession`, not just its `Machine`, because a restored
/// screen also carries the width its game was laid out for
/// ([`GameSession::note_restored_screen_cols`], SQ-0681) — routing every restore
/// through one function is what keeps that from being missed on a path.
pub fn restore_screen(session: &mut GameSession, screen: zvm::screen::ScreenState) {
    // The upper window's grid width IS the restored game's frame of reference:
    // it was last sized from header byte $21 as the SAVING session declared it
    // (`split_window` / `refit_upper_window_width`), which is the width that
    // session's status routine computed its field columns from. Zero when the
    // game had never split — then there is no baked layout to protect and the
    // `max` below is a no-op. (SQ-0681)
    let restored_cols = screen.upper.cols;
    let buffering = screen.buffer_mode;
    // The restored screen's v6 generation has no history — its numbers can
    // collide with ones the memo already holds — so the memoized model goes
    // with the screen it described (SQ-1191).
    session.v6_model_memo.take();
    let machine = &mut session.machine;
    machine.screen = screen;
    machine.out.set_buffer_mode(buffering);
    // SQ-0551: same class of fix as the `buffer_mode` re-sync above — state that
    // lives in two places, only one of which is archived.
    //
    // `current_fg`/`current_bg` are transient display state the VM deliberately
    // does NOT serialize: ZMSD §8.3 gives every v6 window its own pair, and these
    // fields only MIRROR the current window's so the prose stream can tag its runs
    // (see `Machine::mirror_v6_colours`). A restore therefore brings back every
    // window's colours but hands these back as `Default` — and since the prose
    // stream reads them, the first turn after a resume printed in the HOST THEME's
    // ink on top of the story's own, correctly restored, page. Zork Zero came back
    // with a cyan room name and light grey prose over its white page, healing only
    // once the game next called `set_colour` and re-mirrored.
    //
    // Re-derive the pair from the restored window table exactly as the runtime
    // maintains it — the window table is the authority for v6, so deriving keeps
    // the restored screen self-consistent and needs nothing persisted.
    //
    // Versions 1–5/7/8 have no window table to derive from, and nothing else in
    // the archive holds the game's selected colour, so THEY carry the pair in
    // `screen.json` instead (see `ScreenDto::current_fg`) — it is already in
    // `screen` by the time we get here, and the derivation below simply doesn't
    // fire for them. Beyond Zork, Photopia and Nameless all set colours and
    // depend on that path.
    let pair = machine.screen.v6.as_ref().map(|v| {
        let w = &v.windows[(v.current as usize).min(v.windows.len() - 1)];
        (w.fg, w.bg)
    });
    if let Some((fg, bg)) = pair {
        machine.screen.current_fg = fg;
        machine.screen.current_bg = bg;
    }
    session.note_restored_screen_cols(restored_cols);
    reconcile_restored_screen_size(&mut session.machine, session.boot_screen_cols);
}

/// Make a restored screen follow the terminal it is being restored ON, not the
/// one it was saved from (SQ-0589).
///
/// Every restore path runs the VM restore first — `Machine::restore_file` (host
/// Save State) or `complete_restore_success` (in-game `@restore` of a
/// `.lanthorn`) — and both capture the host's dimensions BEFORE the restore
/// overwrites dynamic memory, then re-stamp them through `post_restore_fixups`.
/// So by the time we get here bytes $20/$21 hold THIS host's pane size.
///
/// `restore_screen` then replaces the whole `ScreenState` with the saved
/// session's — which carries the SAVED terminal's grid geometry — and that
/// silently undoes the refit. Restoring an 80-column save into a 120-column
/// pane left an 80-column upper window under a header claiming 120, so a status
/// line or quote box stayed short until the game happened to re-split (games
/// that split once at boot — Sherlock, Trinity — never do). Restoring into a
/// *narrower* pane was the worse direction: an over-wide grid.
///
/// Re-applying the host dims routes the restored screen through exactly the
/// path a live resize takes ([`Machine::refit_upper_window_width`]): content
/// preserved left-aligned, grown columns blanked, shrunk ones truncated, cursor
/// clamped. A restore into a different size IS a resize the game never saw.
///
/// **v6 is exempt**, for the same reason `Engine::set_screen_dims` exempts it: a
/// v6 story lays out on its own fixed native pixel screen (the `Reso`-seeded
/// 640×400 frame) and the app SCALES that into whatever pane it has, so $20/$21
/// are native-derived rather than terminal-derived and there is no terminal
/// geometry to reconcile. Feeding a pane size in would resize the game's
/// coordinate system underneath its own hardcoded art placement — and would
/// clobber the window-0/1 sizes just restored from the archive.
///
/// `floor_cols` is the caller's [`GameSession::boot_screen_cols`] AFTER
/// [`GameSession::note_restored_screen_cols`] has taken the restored game's own
/// width into account (SQ-0681), so the width reconciled here is the same one
/// `loop_tick::poll_zvm_screen_dims` will declare on the next pass. Without it
/// the reconcile would first narrow the restored grid to this pane and the poll
/// would widen it back a frame later — a needless truncate/re-pad of the very
/// row the restore was carrying. v1–3 are exempt (§8.4's fields start at v4, and
/// their status line is recomputed from the globals every turn), matching
/// [`declared_story_screen_dims`]'s own exemptions; a user-pinned
/// `virtual_screen_cols` is not visible from here, so an explicit pin narrower
/// than the floor is re-applied by that poll rather than here.
///
/// [`declared_story_screen_dims`]: crate::render::screen::declared_story_screen_dims
fn reconcile_restored_screen_size(machine: &mut Machine, floor_cols: u16) {
    if machine.mem.version() == 6 {
        return;
    }
    let (rows, cols) = (machine.mem.read_byte(0x20), machine.mem.read_byte(0x21));
    let cols = if machine.mem.version() >= 4 {
        cols.max(floor_cols.clamp(1, 255) as u8)
    } else {
        cols
    };
    if rows > 0 && cols > 0 {
        machine.set_screen_dims(rows, cols);
    }
}

// ── Adventure-title helpers ───────────────────────────────────────────────────

/// The canonical title for a known game, matched on the release+serial prefix of
/// the IFID.
///
/// The table itself moved to `cli_host::titles` in SQ-0850: the per-game save
/// directory of a story mounted out of a disk image is named after its build,
/// and the readable half of that name is this title — so `app` and `zvm-cli`
/// have to read one table or they name one game's directory two ways. Re-exported
/// here because every caller in this crate asks `session` for it.
pub use cli_host::titles::known_title;

/// Extract the adventure title from the opening banner by anchoring on the
/// Infocom-style boilerplate: the title is the non-blank line immediately ABOVE
/// the first line that looks like copyright / "interactive fiction|fantasy" /
/// "Serial number" / trademark text. Returns the trimmed title (capped at 40
/// chars), or `None` when the banner opens with boilerplate (no title above it)
/// or has no such anchor (e.g. an epigraph or story narration) — the caller then
/// falls back to the filename. This avoids grabbing copyright/quote/narration
/// lines as the title.
pub fn title_from_banner(intro_text: &str) -> Option<String> {
    let lines: Vec<&str> = intro_text
        .lines()
        .map(str::trim)
        .filter(|l| {
            !(l.is_empty() || l.starts_with('>') && l.trim_start_matches('>').trim().is_empty())
        })
        .collect();

    let is_anchor = |l: &str| {
        let lower = l.to_lowercase();
        lower.contains("copyright")
            || lower.contains("interactive fiction")
            || lower.contains("interactive fantasy")
            || lower.contains("serial number")
            || lower.contains("trademark")
    };

    let anchor = lines.iter().position(|l| is_anchor(l))?;
    if anchor == 0 {
        return None; // banner opens with boilerplate; no title line above it
    }
    Some(lines[anchor - 1].chars().take(40).collect())
}

/// Resolve the adventure title using a four-tier priority:
/// 1. `override_name` if provided.
/// 2. `metadata` — what the story browser resolved for this file from real
///    metadata ([`crate::picker::metadata_title`]: a container's own `IFmd`
///    chunk, a fetched IFDB sidecar, then the bundled title tables).
/// 3. `banner` (a title extracted from the boot banner) if provided.
/// 4. The story file's stem (filename without extension).
///
/// Metadata sits ABOVE the banner heuristic deliberately (SQ-0766). The
/// heuristic only fires on Infocom-style boilerplate, so a game that boots into
/// a title plate (`anchor.z8` prints `A N C H O R H E A D` and a keypress
/// prompt), a version notice (`photopia.z5`), or a resume question
/// (`mysterious03.z6`) yields nothing and used to land on the filename stem —
/// while the browser, reading the same story's metadata, listed it correctly.
/// One source, asked by both, is what keeps the list and the pane agreeing.
pub fn resolve_title(
    override_name: Option<&str>,
    metadata: Option<&str>,
    banner: Option<&str>,
    story_path: &std::path::Path,
) -> String {
    if let Some(name) = override_name {
        return name.to_owned();
    }
    if let Some(m) = metadata {
        return m.to_owned();
    }
    if let Some(b) = banner {
        return b.to_owned();
    }
    story_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_owned()
}

/// Format the story pane's border title from a resolved name and the story's
/// bare filename (SQ-0766): `name` alone, or `name (filename)` when the two
/// differ. "Differ" compares normalized forms — [`crate::hints::normalize_ident`]
/// applied to `name` and to the filename's stem (extension excluded, so
/// `bureaucracy.z4` doesn't get a redundant `(bureaucracy.z4)` beside the title
/// "Bureaucracy") — so a release/serial-suffixed filename like
/// `zork1-r88-s840726.z3` reads as different from "Zork I" while a file already
/// named after its title reads as the same. `name` empty (title genuinely
/// unknown) falls back to today's behavior: the bare filename, no parenthetical.
///
/// `disk_image` forces the parenthetical on regardless of how well the name
/// matches (SQ-0766). A disk image is a **different release**, not the same
/// story on other media — `stories/journey.z6` is release 83 while
/// `Journey - The Quest Begins.adf` is release 30, and the two behave
/// differently (SQ-0760) — so which medium is mounted is exactly what the pane
/// has to disclose, and it cannot be inferred from the game's name. Without
/// this the box-spelled filenames normalize onto their own titles
/// (`Arthur - The Quest for Excalibur` and "Arthur: The Quest for Excalibur"
/// both reduce to `arthurthequestforexcalibur`) and the medium disappears.
///
/// A blorb or a zip is deliberately NOT forced: a container of that kind ships
/// the very build its title names — it is the publication, not a medium
/// carrying some other release — so the normalized comparison is the right test
/// there, and it already discloses the ones that don't match
/// (`Photopia (photo201.blb)`).
pub fn format_pane_title(name: &str, filename: &str, disk_image: bool) -> String {
    if name.is_empty() {
        return filename.to_owned();
    }
    let stem = std::path::Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename);
    if !disk_image && crate::hints::normalize_ident(name) == crate::hints::normalize_ident(stem) {
        name.to_owned()
    } else {
        format!("{name} ({filename})")
    }
}

// ── Engine adapter (zvm) ────────────────────────────────────────────────────
//
// `GameSession` implements the engine-neutral `Engine` trait so the app can
// drive the Z-machine through the abstraction. The adapter is a thin wrapper:
// the turn methods delegate to the inherent methods (dot-syntax method calls
// resolve to the inherent impl, which takes precedence over the trait), the
// relocated key→ZSCII mapping lives in `key_input_to_zscii`, and `screen()`
// mirrors the Z-machine screen into the neutral `ScreenModel`.

use crate::engine::{
    BorderPref, BufferWindow, Debugger, DisasmProvenance, Engine, EngineError, EngineSave, GraphicsWindow,
    GridCell, GridWindow, Introspect, KeyInput, LocationInfo, PositionedWindow, ScreenModel, Split,
    StatusField, StatusModel, WinNode,
};

/// The engine tag recorded in an `EngineSave` produced by the Z-machine adapter.
pub const ZMACHINE_ENGINE: &str = "zmachine";
/// The save-format version within the `zmachine` engine (Quetzal).
const ZMACHINE_SAVE_FORMAT: u32 = 1;

impl GameSession {
    /// Build the disasm cache on first use, memoize it, and run `f` against it.
    fn with_disasm_cache<R>(&self, f: impl FnOnce(&zvm::cpu::disasm_cache::DisasmCache) -> R) -> R {
        {
            let mut slot = self.disasm_cache.borrow_mut();
            if slot.is_none() {
                let mut cache = zvm::cpu::disasm_cache::DisasmCache::build(&self.machine.mem);
                // Fold the ENTIRE cumulative "ever executed" set ONCE at build
                // time (covers loaded-sidecar seed + all boot/turn PCs) so those
                // regions decode as real code (soft→rd re-decode). The per-turn
                // `exec_pcs` fold in `fold_confirmations` handles later turns; this
                // stays O(build), never O(turn·|ever|). (SQ-0449)
                let mem = &self.machine.mem;
                for &pc in &self.machine.ever_exec_pcs {
                    cache.confirm_pc(mem, pc);
                }
                self.machine.mem.take_mem_fault();
                *slot = Some(cache);
            }
        } // drop borrow_mut before confirmation / the shared borrow
        // Runtime confirmation, once per turn (skip while parked at same PC).
        if self.last_confirmed_pc.get() != Some(self.machine.state.pc) {
            self.confirm_disasm();
        }
        let slot = self.disasm_cache.borrow();
        f(slot.as_ref().unwrap())
    }

    /// Fold runtime-confirmed boundaries (call-stack func_addrs, parked PC, and
    /// last turn's executed PCs) into the cache. No-op if the cache isn't built.
    fn fold_confirmations(&self) {
        let mut slot = self.disasm_cache.borrow_mut();
        let Some(cache) = slot.as_mut() else { return }; // don't build just to confirm
        let mem = &self.machine.mem;
        for f in &self.machine.state.frames {
            cache.confirm_routine(mem, f.func_addr);
        }
        cache.confirm_pc(mem, self.machine.state.pc);
        // When parked at an input prompt, `state.pc` points PAST the read to the
        // code that consumes the input; confirm the read instruction itself too, so
        // it renders as a real op instead of being eaten by a stale tiling. This is
        // independent of `trace_exec` (the read may have executed before tracing was
        // on — e.g. during startup, for the first prompt). (SQ read-pc fix)
        if let Some(read_pc) = self.machine.pending_read_pc() {
            cache.confirm_pc(mem, read_pc);
        }
        for &pc in &self.machine.exec_pcs {
            cache.confirm_pc(mem, pc);
        }
        // Draining a fault isn't needed here (confirm reads via decode which may
        // latch a fault) — drain to be safe, matching the other debug read paths.
        self.machine.mem.take_mem_fault();
    }

    /// Public entry for the per-turn confirmation fold (also callable in tests).
    ///
    /// Only marks the per-turn gate when the cache actually exists, so calling
    /// this before the cache is built (a bare public call) does not poison the
    /// gate and skip the first real fold.
    pub fn confirm_disasm(&self) {
        let built = self.disasm_cache.borrow().is_some();
        if built {
            self.fold_confirmations();
            self.last_confirmed_pc.set(Some(self.machine.state.pc));
        }
    }

    /// object entry base address -> object number.
    fn object_addr_map(&self) -> std::collections::HashMap<u32, u16> {
        let mem = &self.machine.mem;
        zvm::object_tree_view(&self.machine)
            .iter()
            .map(|s| (zvm::objects::object_entry_addr(mem, s.number), s.number))
            .collect()
    }

    /// dictionary entry base address -> decoded word.
    fn dict_addr_map(&self) -> std::collections::HashMap<u32, String> {
        let mem = &self.machine.mem;
        let d = zvm::dictionary::load(mem); // pub fields: base, count, entry_length
        (0..d.count as u32)
            .filter_map(|i| {
                let addr = d.base + i * d.entry_length as u32;
                let (w, _) = zvm::text::decode::decode_string(mem, addr);
                let w = w.trim().to_string();
                (!w.is_empty()).then_some((addr, w))
            })
            .collect()
    }

    /// Insert a ` [tag]` annotation right after each resolvable `@0x{6hex}` memory
    /// operand in a formatted disassembly line (object wins over dictionary). The
    /// scan is byte-safe (`@0x` + hex digits are all ASCII); insertions are applied
    /// right-to-left so earlier byte positions stay valid.
    fn annotate_refs(
        &self,
        line: &str,
        objs: &std::collections::HashMap<u32, u16>,
        dict: &std::collections::HashMap<u32, String>,
    ) -> String {
        let mut inserts: Vec<(usize, String)> = Vec::new();
        let mut i = 0;
        while i + 9 <= line.len() {
            if line.get(i..i + 3) == Some("@0x") {
                if let Some(hex) = line.get(i + 3..i + 9) {
                    if hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                        if let Ok(a) = u32::from_str_radix(hex, 16) {
                            if let Some(n) = objs.get(&a) {
                                inserts.push((i + 9, format!(" [obj#{n}]")));
                            } else if let Some(w) = dict.get(&a) {
                                inserts.push((i + 9, format!(" [{w}]")));
                            }
                            i += 9;
                            continue;
                        }
                    }
                }
            }
            i += 1;
        }
        let mut s = line.to_string();
        for (pos, text) in inserts.into_iter().rev() {
            s.insert_str(pos, &text);
        }
        s
    }

    /// Map a neutral [`KeyInput`] to a ZSCII input byte (the logic relocated from
    /// the app's former `key_to_zscii`). Returns `None` for keys with no ZSCII
    /// meaning (non-ASCII printables, unhandled specials), so the caller leaves
    /// the turn untouched — matching the old "skip unmapped key" behavior exactly.
    ///
    /// Arrow keys and function keys are mapped to ZSCII cursor/function codes
    /// (ZMSD §3.8): Up=129, Down=130, Left=131, Right=132, F1–F4=133–136.
    /// These match zvm-cli's `decode_escape_seq` in `crates/zvm-cli/src/screen.rs`.
    fn key_input_to_zscii(key: KeyInput) -> Option<u8> {
        match key {
            KeyInput::Enter => Some(13),
            KeyInput::Backspace => Some(8),
            KeyInput::Escape => Some(27),
            KeyInput::Up    => Some(129),
            KeyInput::Down  => Some(130),
            KeyInput::Left  => Some(131),
            KeyInput::Right => Some(132),
            KeyInput::Func(n) => Some(132u8.saturating_add(n)),
            KeyInput::Char(c) if c.is_ascii() => Some(c as u8),
            _ => None,
        }
    }

    /// While a Z-machine *line* read is active, decide whether a special key the
    /// player pressed is one the game listed as a line terminator (v5+ table).
    /// Only arrow keys and function keys are candidate terminators; Enter (13)
    /// flows through the normal submit path, and all other keys are never
    /// terminators. Returns the ZSCII terminator code to submit with, or `None`
    /// to leave the key to its normal app behavior.
    /// Does the story want mouse input? ZMSD §11.1 "Flags 2" bit 5, which a game
    /// sets when it intends to read clicks (`read_mouse`, the header extension's
    /// X/Y words). Zork Zero, Arthur, Shogun, Journey and Scopa all set it;
    /// advent.z6 and Sunburst do not.
    pub fn wants_mouse(&self) -> bool {
        self.machine.mem.read_word(0x10) & (1 << 5) != 0
    }

    /// The ZSCII single-click code (254, ZMSD §3.8) as a LINE terminator, when the
    /// story both wants a mouse and lists a click among its terminating characters
    /// (§10.7 — either 254 itself, as Arthur and Scopa do, or the 255 wildcard for
    /// "any function key", as Zork Zero and Shogun do).
    ///
    /// This is what lets a click on Zork Zero's border compass work during ordinary
    /// play: the game sits at a line prompt, and a listed terminator ends that read
    /// with the text typed so far plus the terminator code, whereupon the game reads
    /// the click coordinates. `None` — Journey, whose table is empty and whose menus
    /// are driven by `read_char` instead — leaves a click to the app's own handling,
    /// so no story ever has a partial command submitted by a stray click.
    pub fn mouse_click_terminator(&self) -> Option<u8> {
        const SINGLE_CLICK: u8 = 254;
        (self.wants_mouse() && self.is_terminator(SINGLE_CLICK as u16)).then_some(SINGLE_CLICK)
    }

    pub fn line_key_terminator(&self, ki: &KeyInput) -> Option<u8> {
        match ki {
            KeyInput::Up | KeyInput::Down | KeyInput::Left | KeyInput::Right | KeyInput::Func(_) => {}
            _ => return None,
        }
        let z = Self::key_input_to_zscii(*ki)?;
        self.is_terminator(z as u16).then_some(z)
    }
}

/// One entry in a v6 window's display list — a picture drawn, or a region erased,
/// in window-canvas coordinates (SQ-0567).
///
/// Serializable because a host Save State persists the display list itself rather
/// than a picture of the result (SQ-0588): these ops ARE the archived form of a v6
/// screen, replayed under the restored palette to rebuild it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum V6Op {
    Draw { number: u16, dx: i32, dy: i32 },
    Erase { dx: i32, dy: i32, w: u32, h: u32 },
}

/// Longest display list kept per window. Comfortably above any real screen (Arthur's
/// busiest is a handful of ops) while bounding a story that redraws forever.
pub const V6_OPS_CAP: usize = 512;

/// Mirror a Z-machine's screen into the neutral [`ScreenModel`].
///
/// The upper window becomes a [`GridWindow`] (logical size + cells + cursor +
/// active-window flag); the lower window is a buffer placeholder (the app owns
/// the transcript). The status is the v3 automatic status line (`Classic`) for
/// v1–3, or `HostManaged` for v4+ (whose globals are not a status line). Shared
/// by the engine adapter and the render-equivalence tests.
pub fn screen_model_from_machine(machine: &Machine) -> ScreenModel {
    let screen = &machine.screen;
    let src = &screen.upper;
    let grid = GridWindow {
        fill: None, // v6-only erase fill (SQ-0584)
        cols: src.cols,
        rows: src.rows,
        cells: src
            .cells
            .iter()
            .map(|c| GridCell {
                ch: c.ch,
                style: c.style,
                fg: crate::state::pack_zcolour(c.fg),
                bg: crate::state::pack_zcolour(c.bg),
                link: 0, // Z-machine grid cells carry no Glk hyperlink
                glk_style: 0, // Z-machine is always Normal
            })
            .collect(),
        // `upper.rows` equals `upper_window_rows` normally, but grows when a game
        // draws in the upper window below the split (e.g. LostPig's HELP menu
        // splits to 7 rows then prints 5 items at rows 6–10). Render/reserve the
        // full grown height so nothing is clipped.
        active_rows: screen.upper.rows,
        cursor: (screen.cursor_row, screen.cursor_col),
        cursor_active: screen.current_window == 1,
        // The Z-machine has no Glk border concept — leave it to the theme (SQ-0286).
        border: BorderPref::Unspecified,
        // The Z-machine simple path carries no per-window colour override; the
        // page colour comes from the model bg/fg below, so draw_grid stays
        // byte-identical (bg=None → theme). (SQ-0328)
        bg: None,
        fg: None,
        // Z-machine grid reverse is per-cell (style bits), not a window-level Glk
        // ReverseColor, so no window-level reverse fill here. (SQ-0403)
        reverse: false,
        px_texts: Vec::new(),
    };
    ScreenModel {
        root: WinNode::Pair {
            vertical: true,
            split: Split { fixed: screen.upper.rows },
            // The Z-machine has no Glk border; its status box is drawn by the simple path.
            border: false,
            key_bg: None,
            key_fg: None,
            first: Box::new(WinNode::Grid(grid)),
            second: Box::new(WinNode::Buffer(BufferWindow::default())),
        },
        status: status_model_from_machine(machine),
        bg: crate::state::pack_zcolour(screen.current_bg),
        fg: crate::state::pack_zcolour(screen.current_fg),
        // Z-machine layout has no snap margin (simple path); the composite never
        // clamps it. (SQ-0303)
        content_size: (0, 0),
    }
}

/// Build the neutral [`StatusModel`] from a Z-machine's screen state: a
/// `Classic` automatic status line (location + score/turns or clock) for v1–3,
/// or `HostManaged` for v4+ (whose globals are not a status line). Shared by
/// the engine adapter and the render-equivalence tests.
pub fn status_model_from_machine(machine: &Machine) -> StatusModel {
    if machine.mem.version() <= 3 {
        let sl = machine.status_line();
        let right = match sl.right {
            zvm::screen::StatusRight::ScoreTurns { score, turns } => {
                StatusField::ScoreTurns { score, turns }
            }
            zvm::screen::StatusRight::Time { hours, minutes } => {
                StatusField::Time { hours, minutes }
            }
        };
        StatusModel::Classic { location: sl.location, right }
    } else {
        StatusModel::HostManaged
    }
}

impl Engine for GameSession {
    fn submit(&mut self, command: &str) -> TurnResult {
        if self.machine.trace_exec { self.machine.exec_pcs.clear(); }
        // A turn executes new code, so its freshly-recorded boundaries must be
        // folded afterward EVEN IF it returns to the same parked PC (every
        // look/examine returns to the same input prompt). Reopen the per-turn
        // confirmation gate so the next disassemble re-folds. (read-pc follow-up)
        self.last_confirmed_pc.set(None);
        // Dot syntax resolves to the inherent `GameSession::submit` (inherent
        // methods take precedence over trait methods), so this is not recursive.
        self.submit(command)
    }

    fn submit_key(&mut self, key: KeyInput) -> Option<TurnResult> {
        if self.machine.trace_exec { self.machine.exec_pcs.clear(); }
        self.last_confirmed_pc.set(None); // reopen the confirmation gate each turn
        let byte = GameSession::key_input_to_zscii(key)?;
        Some(self.submit_char(byte))
    }

    fn paint_surface(&self) -> Option<std::sync::Arc<image::RgbaImage>> {
        self.paint.clone()
    }

    fn set_mouse(&mut self, y_px: u16, x_px: u16) {
        // Primary button (bit 0) — a host left-click. The VM records the coords
        // and writes the header extension table (ZMSD §11); a following
        // `read_mouse` reports them.
        self.machine.set_mouse(y_px, x_px, 0b1);
    }

    fn set_screen_dims(&mut self, rows: u16, cols: u16) {
        // ZMSD §8.4 — publish the host's REAL pane size in bytes $20/$21 (and the
        // v5+ unit words). A story pane wider or narrower than the old fixed 80×24
        // guess otherwise made `split_window` build an upper grid at the WRONG
        // width, so a game's centred form/quote box never lined up with the prose
        // beside it. (SQ-0532/A-F1)
        //
        // v6 is deliberately exempt: it lays out in its own fixed 640×400 pixel
        // screen (`V6_ART_SCALE`d Blorb `Reso` window, seeded before boot), and
        // the app SCALES that native frame into whatever pane it has. Feeding the
        // pane's cell size in would resize the game's coordinate system underneath
        // its own hardcoded art placement.
        if self.machine.mem.version() == 6 {
            return;
        }
        self.machine
            .set_screen_dims(rows.clamp(1, 255) as u8, cols.clamp(1, 255) as u8);
    }

    fn set_default_colours(&mut self, bg: u8, fg: u8) {
        // ZMSD §8.3.3 — the header's default pair should be the interpreter's own
        // (the codes are clamped to 2..=9 by the VM). (SQ-0532/A-F2)
        self.machine.set_default_colours(bg, fg);
    }

    fn take_transcript(&mut self) -> String {
        self.take_transcript()
    }

    fn drain_screen_clear(&mut self) -> bool {
        // The position stamp goes with it — see `GameSession::take_screen_clear`.
        self.take_screen_clear().0
    }

    fn take_transcript_elems(&mut self) -> Vec<TranscriptElem> {
        // Non-empty only when v6 window-0 inline pictures are pending (Zork
        // Zero's boot drop-cap): interleave them into the sink text as ordered
        // elements. Every other story returns empty → the flat path is used.
        if self.story_pics.is_empty() {
            return Vec::new();
        }
        let base = self.v6_win0_chars_seen;
        let (raw, raw_runs) = sink_mut(&mut self.machine).take_styled();
        self.v6_win0_chars_seen = self.machine.v6_win0_out_chars;
        let transcript = if self.strip_prompt { strip_read_prompt(&raw).to_owned() } else { raw };
        let runs = clamp_runs(raw_runs, transcript.chars().count());
        let marks = std::mem::take(&mut self.story_pics)
            .into_iter()
            .map(|(at, img)| (at, TranscriptElem::Image(img)))
            .collect();
        interleave_story_elems(&transcript, &runs, marks, base, None)
    }

    fn set_strip_prompt(&mut self, on: bool) {
        self.strip_prompt = on;
    }

    fn output_continued_line(&self) -> bool {
        self.output_continued
    }

    fn pending_input(&self) -> InputKind {
        self.pending
    }

    fn resume_save(&mut self, wrote_ok: bool) -> TurnResult {
        self.resume_save(wrote_ok)
    }

    fn resume_restore(&mut self, data: Option<&[u8]>) -> TurnResult {
        self.resume_restore(data)
    }

    fn has_quit(&self) -> bool {
        self.quit
    }

    fn screen(&self) -> ScreenModel {
        if self.machine.screen.v6.is_some() {
            self.v6_screen_model(&self.pictures_canvas)
        } else {
            screen_model_from_machine(&self.machine)
        }
    }

    /// The settled screen, EXCEPT while a v6 turn's picture sequence is still
    /// playing out — then it is the frame that is up (SQ-0708). Identical to
    /// [`screen`](Engine::screen) for every non-v6 story and for every v6 frame
    /// once the sequence has settled, which is every frame the player is not
    /// actively watching a picture land on.
    fn screen_now(&self) -> std::sync::Arc<ScreenModel> {
        if self.machine.screen.v6.is_some() {
            // Memoized (SQ-1191): a frame on which nothing changed gets the
            // previous frame's Arc back instead of a fresh clone tree.
            self.v6_screen_model_shared(self.visible_canvas())
        } else {
            std::sync::Arc::new(screen_model_from_machine(&self.machine))
        }
    }

    /// `/dump-windows` for the Z-machine. The trait's default describes the v1–5
    /// shape — one grid over one buffer — which tells you nothing about a v6 story:
    /// its model is a LAYERED composite of up to eight windows.
    ///
    /// See [`GameSession::v6_window_dump`] for the v6 form; this is the fallback for
    /// v1–5 and for a caller with no render mapping to hand.
    fn window_dump(&self) -> Vec<String> {
        if self.machine.screen.v6.is_some() {
            return self.v6_window_dump(&[], None);
        }
        let screen = &self.machine.screen;
        let mut out = vec![format!("Z-machine v{} layout — Grid over Buffer", self.machine.mem.version())];
        // The split height and the painted grid height are now deliberately
        // separate numbers (SQ-0696): a `split_window` shrink keeps whatever rows
        // were painted (Inform box quotes survive it), and a game may paint below
        // its own split (LostPig's HELP menu). Surfacing them apart is the whole
        // point of this dump — collapsed together they hide exactly the
        // divergence a screen bug needs.
        out.push(format!(
            "  split: {} row(s) requested  ·  grid: {} row(s) painted{}",
            screen.upper_window_rows,
            screen.upper.rows,
            if screen.upper.rows != screen.upper_window_rows { "  <- diverge" } else { "" }
        ));
        out.push(format!("  grid cols: {}", screen.upper.cols));
        out.push(format!(
            "  cursor: row {}, col {}  ·  window: {}",
            screen.cursor_row,
            screen.cursor_col,
            if screen.current_window == 1 { "upper" } else { "lower" }
        ));
        out.push(format!("  buffer_mode: {}", screen.buffer_mode));
        out.push(format!("  colours: fg {:?}, bg {:?}", screen.current_fg, screen.current_bg));

        // The grid's non-blank rows, as a reader would see them — trailing
        // blanks trimmed, capped so a runaway grid can't flood the dump.
        const MAX_PRINTED_ROWS: usize = 20;
        let cols = screen.upper.cols as usize;
        let rows = screen.upper.rows as usize;
        let mut printed = 0usize;
        let mut truncated = false;
        for r in 0..rows {
            let start = r * cols;
            if start >= screen.upper.cells.len() {
                break;
            }
            let end = (start + cols).min(screen.upper.cells.len());
            let text: String = screen.upper.cells[start..end].iter().map(|c| c.ch).collect();
            let trimmed = text.trim_end();
            if trimmed.is_empty() {
                continue;
            }
            if printed >= MAX_PRINTED_ROWS {
                truncated = true;
                break;
            }
            // 1-based, matching the `cursor:` line above and the Z-machine's own
            // row coordinates (`set_cursor` is 1-based, ZMSD §8.7.2.3) — a dump
            // that mixed the two bases would be read wrong at a glance.
            out.push(format!("  row {:>2}: {trimmed:?}", r + 1));
            printed += 1;
        }
        if truncated {
            out.push(format!("  … additional non-blank row(s) not shown (cap {MAX_PRINTED_ROWS})"));
        }
        out
    }

    fn save_state(&self) -> EngineSave {
        EngineSave::new(ZMACHINE_ENGINE, ZMACHINE_SAVE_FORMAT, self.machine.save_quetzal())
    }

    fn restore_state(&mut self, save: &EngineSave) -> Result<(), EngineError> {
        if !save.is_engine(ZMACHINE_ENGINE) {
            return Err(EngineError::EngineMismatch {
                expected: ZMACHINE_ENGINE.to_string(),
                found: save.engine.clone(),
            });
        }
        self.machine
            .restore_file(&save.bytes)
            .map_err(|e| EngineError::BadSave(format!("{e:?}")))?;
        // The restore swapped dynamic memory wholesale, and this path does not
        // drain a turn — drop the cached object-word set here as `drain_turn`
        // does, or it keeps answering for the session we just left (SQ-1176).
        self.object_word_set.take();
        self.v6_model_memo.take(); // same duty for the memoized screen model (SQ-1191)
        // The restored memory brings NO screen with it — Quetzal archives none by
        // design — so whatever is in the upper window belongs to the moment we
        // just left, not to the one we just restored. Leaving it there lets a
        // v4+ status line read as a mix of two rooms: the story repaints only as
        // many columns as its new room name needs, and the tail of the longer
        // previous name survives past the end of it. `detect_location` reads that
        // grid, so the mixed string matches no object, the ladder falls off
        // `PlayerParent` onto the text rung, and a *plausible wrong room number*
        // comes back. That is how a return probe restored into Zork I's Clearing
        // read `Forest Pathse` and reported object 1 — the scenery object named
        // `forest` — instead of Forest Path, and discarded a real return path
        // (SQ-0785).
        //
        // Blanked rather than resized: the restored game's status fields were
        // baked at the saving session's width (SQ-0681), and that width is still
        // the right frame of reference. A caller with a real screen to restore
        // (`restore_screen`, the `.lanthorn` archive path) replaces the whole
        // `ScreenState` immediately after this and never sees the blank.
        self.machine.screen.upper.blank();
        // A Save State is snapshotted at an input prompt; its PC points AT the
        // read/read_char instruction (save_pc rewinds it), so run forward to
        // re-execute that read — re-arming the pending input on the freshly
        // restored buffers. Without this the VM would be parked past the read
        // with a stale buffer, and the next line would replay the pre-save
        // command (mirrors `resume_restore` for the game `@restore` path).
        //
        // `run_settled`, not a bare `run_until_input`: a `Quit` here has to SET
        // `self.quit` (the old code moved `pending` to Line and left the session
        // claiming the game was still running, unlike its sibling
        // `restore_game_save` below), and a `@save`/`@restore` reached on the way
        // has to be ANSWERED rather than silently dropped — dropping it parks the
        // VM on a suspension no dialog will ever open for. Uniform with the Glulx
        // adapter, whose restore runs the same settling drive. (SQ-0656)
        let (pending, quit) = run_settled(&mut self.machine);
        self.pending = pending;
        self.quit = quit;
        Ok(())
    }

    fn restore_game_save(&mut self, bytes: &[u8]) -> Result<(), EngineError> {
        self.machine.complete_restore_success(bytes)
            .map_err(|e| EngineError::BadSave(format!("{e:?}")))?;
        // Memory was swapped without draining a turn — same duty as in
        // `restore_state` above (SQ-1176).
        self.object_word_set.take();
        self.v6_model_memo.take(); // same duty for the memoized screen model (SQ-1191)
        // complete_restore_success lands mid-way through the game's save verb
        // (just past the @save descriptor), not at a read. Run forward to the
        // next read so the machine is re-armed at a clean prompt — otherwise the
        // first typed command is dropped while the save-verb tail runs (mirrors
        // resume_restore for the in-game @restore path). The save-verb tail
        // output (e.g. "Ok.") is redundant with the host's "[Game restored]"
        // message, so drain and discard it.
        //
        // `run_settled`: the save-verb tail can chain straight into another
        // `@save`/`@restore` (a game that re-saves after loading), and dropping
        // that stop left the VM suspended with no dialog to answer it — every
        // later turn would re-report it. It is auto-failed and the drive
        // continues, as on the boot and Save State paths. (SQ-0656)
        let (pending, quit) = run_settled(&mut self.machine);
        let _ = self.take_transcript();
        self.pending = pending;
        self.quit = quit;
        Ok(())
    }

    /// The Z-machine twin of the Glulx override (SQ-0661). The trait's default is
    /// `false`, which silently made the two host guards that consult it —
    /// `lifecycle::exit_auto_save` and `lifecycle::quit_dialog_save` — no-ops for
    /// this engine.
    ///
    /// The Z-machine has no un-popped call stub to worry about, but it has its own
    /// hazard: while the game's `@save` is suspended, `Machine::save_pc` reports
    /// the result-descriptor address (Quetzal §5.8), so an exit auto-save fired in
    /// that window writes an archive whose PC points at a branch/store descriptor
    /// byte. Resuming it decodes that byte as an opcode. A suspended `@restore` is
    /// the same window from the other side. Both are the player's own in-game save
    /// in progress, which is the relevant persistence anyway.
    fn is_saveload_pending(&self) -> bool {
        self.machine.is_saveload_pending()
    }

    fn aux_data(&self) -> &std::collections::BTreeMap<String, Vec<u8>> {
        &self.machine.aux_data
    }

    fn set_aux_data(&mut self, data: std::collections::BTreeMap<String, Vec<u8>>) {
        self.machine.aux_data = data;
    }

    fn aux_dirty(&self) -> bool {
        self.machine.aux_dirty
    }

    fn clear_aux_dirty(&mut self) {
        self.machine.aux_dirty = false;
    }

    fn current_location(&self) -> Option<LocationInfo> {
        // Version-aware detection (same as a turn), NOT the v3-only global-0 read:
        // v4+ games have no location global, so `zvm::current_location` returns
        // None at boot, leaving the starting room off the map until the first turn.
        detect_location(&self.machine).as_ref().map(location_to_snapshot)
    }

    fn set_trace_screen(&mut self, on: bool) {
        self.machine.trace_screen = on;
    }

    fn take_screen_trace(&mut self) -> Vec<String> {
        std::mem::take(&mut self.machine.screen_trace)
    }

    fn v6_snapshot(&self) -> Option<Vec<String>> {
        let v6 = self.machine.screen.v6.as_ref()?;
        let mut lines = vec![format!("turn snapshot (current={})", v6.current)];
        for (i, w) in v6.windows.iter().enumerate() {
            let nontrivial =
                w.x_size != 0 || w.y_size != 0 || !w.texts.is_empty() || w.attributes != 0;
            if !nontrivial {
                continue;
            }
            lines.push(format!(
                "win{i}: pos=({},{}) size={}x{} cursor=({},{}) attrs=0b{:04b} margins=({},{}) font=({},{}) runs={}",
                w.x_coord, w.y_coord, w.x_size, w.y_size, w.y_cursor, w.x_cursor,
                w.attributes, w.left_margin, w.right_margin, w.font_number, w.font_size,
                w.texts.len(),
            ));
            let shown = w.texts.len().min(20);
            for t in w.texts.iter().take(shown) {
                let text: String = if t.text.chars().count() > 60 {
                    t.text.chars().take(60).collect()
                } else {
                    t.text.clone()
                };
                lines.push(format!("  y={} x={} style={} {:?}", t.y, t.x, t.style, text));
            }
            if w.texts.len() > shown {
                lines.push(format!("  ... ({} more)", w.texts.len() - shown));
            }
        }
        let mut wins: Vec<&u8> = self.pictures_canvas.keys().collect();
        wins.sort();
        for i in wins {
            let c = &self.pictures_canvas[i];
            lines.push(format!("canvas win{i}: {}x{} z={}", c.img.width(), c.img.height(), c.z_seq));
        }
        Some(lines)
    }

    fn set_debug_trace(&mut self, on: bool) {
        self.machine.trace_exec = on;
        // Only the per-turn set is cleared when tracing stops; the cumulative
        // `ever_exec_pcs` (permanent colour + persisted coverage) is preserved.
        if !on { self.machine.exec_pcs.clear(); }
    }

    fn seed_executed_pcs(&mut self, pcs: &std::collections::HashSet<u32>) {
        self.machine.seed_executed(pcs.iter().copied());
    }

    /// The story's grammar and dictionary, as the engine-neutral snapshot.
    ///
    /// `None` for a story with no grammar table at all — `Grammar::load` answers
    /// `Absent` for a menu-driven Version 6 game such as Journey, which has no
    /// verbs for an offer to reach.
    fn story_vocabulary(&self) -> Option<crate::vocab::StoryVocabulary> {
        let mem = &self.machine.mem;
        let grammar = zvm::grammar::Grammar::load(mem).ok()?;
        let words = grammar
            .words()
            .map(|w| (w.to_string(), grammar.roles(w).unwrap_or_default()))
            .collect();
        let preps = grammar.prepositions().iter().cloned().collect();
        // `Dictionary::key_len` is the encoded key in BYTES — 4 in v1-3, 6 in
        // v4+ (ZMSD §13.3/§13.4) — and two bytes hold three Z-characters, so the
        // dictionary keeps six characters of a word, or nine.
        let key_len = zvm::dictionary::load(mem).key_len() as usize / 2 * 3;
        Some(crate::vocab::StoryVocabulary::new(grammar.verbs().to_vec(), words, preps, key_len))
    }

    /// The story's OWN dictionary lookup, which encodes the word the way the game
    /// encodes it — so the Z-machine's Z-character truncation is applied exactly,
    /// including for a word whose characters do not all cost one Z-character.
    fn knows_word(&self, word: &str) -> Option<bool> {
        let mem = &self.machine.mem;
        Some(zvm::dictionary::load(mem).lookup(mem, word) != 0)
    }

    /// The story's OWN tokeniser, run over prose the story itself printed
    /// (SQ-1116) — `zvm::dictionary::tokenise`, which is the routine `read`
    /// calls, so the dictionary's declared separators (ZMSD §13.1) are the ones
    /// applied.
    ///
    /// Two adaptations, both because a text buffer is a typed LINE and this is
    /// not one:
    ///
    /// - `tokenise` reads ZSCII bytes, one per character, exactly as `supply_line`
    ///   leaves them — so the prose is mapped through `Memory::zscii_from_unicode`
    ///   (the story's own Unicode table first, §3.8.5.4) and a parallel `Vec<char>`
    ///   keeps the printed spelling recoverable at the same indices.
    /// - `Token::text_pos` and `Token::len` are single BYTES, which a 255-byte
    ///   input line can never overflow and a page of prose certainly can. The text
    ///   is fed in chunks that end on a space, so no position is ever truncated;
    ///   the split points are spaces, which the tokeniser would have split on
    ///   anyway, so chunking cannot change the answer.
    fn split_like_parser(&self, text: &str) -> Option<Vec<String>> {
        /// Comfortably under `u8::MAX`, and never mid-word.
        const CHUNK: usize = 200;

        let mem = &self.machine.mem;
        let dict = zvm::dictionary::load(mem);
        let mut words: Vec<String> = Vec::new();
        let mut chars: Vec<char> = Vec::with_capacity(CHUNK + 1);
        let mut bytes: Vec<u8> = Vec::with_capacity(CHUNK + 1);

        let flush = |chars: &mut Vec<char>, bytes: &mut Vec<u8>, out: &mut Vec<String>| {
            for t in dict.tokenise(mem, bytes) {
                let start = t.text_pos as usize;
                let end = start + t.len as usize;
                out.push(chars[start..end].iter().collect::<String>().to_lowercase());
            }
            chars.clear();
            bytes.clear();
        };

        for ch in text.chars() {
            // Every kind of gap is a space: ZSCII has a newline (13) and the
            // tokeniser splits only on space and separators, so a line break left
            // as itself would be glued into the word beside it.
            let ch = if ch.is_whitespace() { ' ' } else { ch };
            if ch == ' ' && chars.len() >= CHUNK {
                flush(&mut chars, &mut bytes, &mut words);
                continue;
            }
            chars.push(ch);
            bytes.push(mem.zscii_from_unicode(ch));
            // A single "word" longer than the chunk cannot be a dictionary word in
            // any story; cut it rather than let the byte position wrap.
            if chars.len() >= u8::MAX as usize {
                flush(&mut chars, &mut bytes, &mut words);
            }
        }
        flush(&mut chars, &mut bytes, &mut words);
        Some(words)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn introspect(&self) -> Option<&dyn Introspect> {
        Some(self)
    }

    fn debugger(&self) -> Option<&dyn Debugger> {
        Some(self)
    }
}

impl GameSession {
    /// The story's inferred object-table conventions, derived on first use.
    pub fn world_model(&self) -> &zvm::world::WorldModel {
        self.world.get_or_init(|| zvm::world::WorldModel::discover_at_boot(&self.machine))
    }

    /// The reader for the words this story's parser accepts for an object,
    /// derived on first use. `None` for a story that keeps no such words —
    /// see [`parse_names`](Self::parse_names) the field.
    pub fn parse_names(&self) -> Option<&zvm::objects::ParseNames> {
        self.parse_names
            .get_or_init(|| zvm::objects::ParseNames::detect(&self.machine.mem))
            .as_ref()
    }
}

impl Introspect for GameSession {
    fn vocabulary(&self) -> Vec<String> {
        zvm::dictionary::load(&self.machine.mem).words(&self.machine.mem)
    }

    fn contents(&self, container: u16) -> Vec<crate::engine::ObjectWords> {
        crate::inventory::list_inventory(&self.machine.mem, self.parse_names(), container)
    }

    fn room_objects(&self, room: u16) -> Vec<crate::engine::ObjectWords> {
        crate::render::room_info::list_room_objects(
            self.world_model(),
            self.parse_names(),
            &self.machine.mem,
            room,
        )
    }

    fn room_objects_excluding(
        &self,
        room: u16,
        exclude: Option<u16>,
    ) -> Vec<crate::engine::ObjectWords> {
        crate::render::room_info::list_room_objects_excluding(
            self.world_model(),
            self.parse_names(),
            &self.machine.mem,
            room,
            exclude.unwrap_or(0),
        )
    }

    fn visible_contents(&self, container: u16) -> Vec<crate::engine::ObjectWords> {
        crate::inventory::list_visible_contents(
            self.world_model(),
            self.parse_names(),
            &self.machine.mem,
            container,
        )
    }

    fn all_object_words(&self) -> Option<Vec<crate::engine::ObjectWords>> {
        // `ParseNames::detect` is cached in `parse_names`; the walk itself is
        // one pass over the object table. The bulk any-object callers go
        // through `object_word_set` below instead, which caches the walk for
        // the rest of the turn.
        Some(self.parse_names()?.all(&self.machine.mem))
    }

    fn object_word_set(&self) -> Option<std::sync::Arc<grammar_model::ObjectWordSet>> {
        // Whether the story keeps parse names at all is a compile-time layout
        // fact (`parse_names` is a `OnceCell`), so a `None` needs no cache.
        let names = self.parse_names()?;
        if let Some(set) = self.object_word_set.borrow().as_ref() {
            return Some(std::sync::Arc::clone(set));
        }
        // One walk of the object table per TURN, not per token: `drain_turn`
        // drops the entry whenever the VM runs, because the words live in
        // dynamic memory and a game can rewrite them (see the field's doc).
        let set = std::sync::Arc::new(grammar_model::ObjectWordSet::build(&names.all(&self.machine.mem)));
        *self.object_word_set.borrow_mut() = Some(std::sync::Arc::clone(&set));
        Some(set)
    }

    fn children_of(&self, parent: u16) -> std::collections::BTreeSet<u16> {
        let max_obj = zvm::object_tree_view(&self.machine)
            .into_iter()
            .map(|s| s.number)
            .max()
            .unwrap_or(0);
        (1..=max_obj)
            .filter(|&o| zvm::objects::get_parent(&self.machine.mem, o) == parent)
            .collect()
    }

    fn player_object(&self) -> Option<u16> {
        zvm::find_player_object(&self.machine)
    }
}

impl Debugger for GameSession {
    fn pc(&self) -> u32 {
        self.machine.state.pc
    }

    fn disassemble(&self, addr: u32, lines: usize) -> Vec<String> {
        use zvm::cpu::disasm_cache::CacheFmt;
        let mut out = self.with_disasm_cache(|c| c.disassemble(&self.machine.mem, addr, lines, CacheFmt::Full));
        // Annotate `@0x……` memory-reference operands with their referent: a
        // clickable ` [obj#N]` for an object entry base, an informational ` [word]`
        // for a dictionary entry base. Build both reverse maps once per call.
        let objs = self.object_addr_map();
        let dict = self.dict_addr_map();
        for line in &mut out {
            *line = self.annotate_refs(line, &objs, &dict);
        }
        // The disassembler can walk past code into data; an out-of-range read
        // latches a fault into Memory's fault cell that the CPU drains each step.
        // Discard it here so this read-only inspection never leaks a phantom fault
        // that would halt the VM on its next instruction. Between turns there is no
        // legitimately-pending fault (the VM consumes its own at step end), so
        // discarding is safe.
        self.machine.mem.take_mem_fault();
        out
    }

    fn disassemble_tiered(&self, addr: u32, lines: usize) -> Vec<(String, DisasmProvenance)> {
        use zvm::cpu::disasm_cache::CacheFmt;
        // Full-form rows carry the same `[obj#N]`/`[word]` annotations as
        // `disassemble`; provenance is display-format-independent, so a caller in
        // basic/raw mode pairs these provenance tags with its own text lines.
        let mut out =
            self.with_disasm_cache(|c| c.disassemble_tiered(&self.machine.mem, addr, lines, CacheFmt::Full));
        let objs = self.object_addr_map();
        let dict = self.dict_addr_map();
        for (line, _prov) in &mut out {
            *line = self.annotate_refs(line, &objs, &dict);
        }
        self.machine.mem.take_mem_fault(); // never leak a debug-read fault into the VM
        out.into_iter().map(|(s, p)| (s, p.into())).collect()
    }

    fn disassemble_raw(&self, addr: u32, lines: usize) -> Vec<String> {
        use zvm::cpu::disasm_cache::CacheFmt;
        let out = self.with_disasm_cache(|c| c.disassemble(&self.machine.mem, addr, lines, CacheFmt::Raw));
        self.machine.mem.take_mem_fault(); // never leak a debug-read fault into the VM
        out
    }

    fn disassemble_basic(&self, addr: u32, lines: usize) -> Vec<String> {
        use zvm::cpu::disasm_cache::CacheFmt;
        // Basic form: plain mnemonic disassembly with NO annotations (the
        // `[obj#N]`/`[word]` reference-following stays exclusive to `disassemble`).
        let out = self.with_disasm_cache(|c| c.disassemble(&self.machine.mem, addr, lines, CacheFmt::Basic));
        self.machine.mem.take_mem_fault(); // never leak a debug-read fault into the VM
        out
    }

    fn next_instr(&self, addr: u32) -> u32 {
        let out = self.with_disasm_cache(|c| c.next_addr(addr));
        self.machine.mem.take_mem_fault(); // never leak a debug-read fault into the VM
        out
    }

    fn prev_instr(&self, addr: u32) -> u32 {
        let out = self.with_disasm_cache(|c| c.prev_addr(addr));
        self.machine.mem.take_mem_fault(); // never leak a debug-read fault into the VM
        out
    }

    fn describe_line(&self, addr: u32) -> Option<Vec<String>> {
        let version = self.machine.mem.version();
        let instr = zvm::cpu::decode::decode(&self.machine.mem, addr, version);
        let unpack = zvm::cpu::disasm::Unpack::from_mem(&self.machine.mem);
        let lines = zvm::cpu::disasm::describe_instruction(&instr, version, &unpack);
        self.machine.mem.take_mem_fault(); // never leak a debug-read fault into the VM
        Some(lines)
    }

    fn executed_pcs(&self) -> std::collections::HashSet<u32> {
        self.machine.exec_pcs.clone()
    }

    fn ever_executed_pcs(&self) -> std::collections::HashSet<u32> {
        self.machine.ever_exec_pcs.clone()
    }

    fn stack_lines(&self) -> Vec<String> {
        let st = &self.machine.state;
        if st.frames.is_empty() {
            return vec!["(no frames)".to_string()];
        }
        let mut out = Vec::with_capacity(st.frames.len());
        for (i, f) in st.frames.iter().enumerate() {
            out.push(format!(
                "#{i}  fn@{:06x}  ret={:06x}  args={}",
                f.func_addr, f.return_pc, f.arg_count
            ));
        }
        out
    }

    fn eval_stack_lines(&self) -> Vec<String> {
        let st = &self.machine.state;
        if st.eval_stack.is_empty() {
            return vec!["(empty)".to_string()];
        }
        let bases: std::collections::HashSet<usize> =
            st.frames.iter().map(|f| f.eval_base).collect();
        st.eval_stack.iter().enumerate().rev().map(|(i, v)| {
            let b = if bases.contains(&i) { "  <- frame base" } else { "" };
            format!("[{i:>3}] {:04x}  ({}){}", v, *v as i16, b)
        }).collect()
    }

    fn locals_lines(&self) -> Vec<String> {
        match self.machine.state.frames.last() {
            None => vec!["(no frame)".to_string()],
            Some(f) if f.locals.is_empty() => vec!["(none)".to_string()],
            Some(f) => f.locals.iter().enumerate()
                .map(|(i, w)| format!("local{i} = {:04x}  ({})", w, w))
                .collect(),
        }
    }

    fn globals_lines(&self) -> Vec<String> {
        let out: Vec<String> =
            (0u8..240).map(|n| format!("g{:02x} = {:04x}", n, self.machine.global(n))).collect();
        self.machine.mem.take_mem_fault(); // never leak a debug-read fault into the VM
        out
    }

    fn object_tree_lines(&self) -> Vec<String> {
        // A real tree: DFS over the child/sibling links so each object renders
        // directly under its parent. (Numeric order + per-object indent, which
        // this replaces, does NOT nest children under their parents.)
        let mem = &self.machine.mem;
        let numbers: Vec<u16> = zvm::object_tree_view(&self.machine)
            .iter().map(|s| s.number).collect();
        let out = build_object_tree(
            &numbers,
            |o| zvm::objects::get_parent(mem, o),
            |o| zvm::objects::get_child(mem, o),
            |o| zvm::objects::get_sibling(mem, o),
            |o| zvm::objects::short_name(mem, o),
            // The row's `@0x……` link jumps the Memory view, so it must land
            // where the object's TEXT is — the property table (§12.4: a
            // one-byte word count, then the short name), not the entry (§12.3:
            // flags, tree links and the pointer that got us here), whose bytes
            // never contain a character of the name and which for a low object
            // number puts the name off the bottom of the window entirely
            // (SQ-0975). The entry stays one click away in the expanded detail.
            // An unaddressable object has no table to point at; its entry
            // address is the only address it has.
            |o| zvm::objects::object_prop_table_addr(mem, o)
                .unwrap_or_else(|| zvm::objects::object_entry_addr(mem, o)),
        );
        self.machine.mem.take_mem_fault(); // never leak a debug-read fault into the VM
        out
    }

    fn dictionary_lines(&self) -> Vec<String> {
        // Each row leads with its entry byte address as a clickable `@0x……`
        // Memory-jump token (debug inspector), then the decoded word.
        let out = zvm::dictionary::load(&self.machine.mem)
            .entries(&self.machine.mem)
            .into_iter()
            .map(|(addr, word)| format!("@0x{:06x} {}", addr, word))
            .collect();
        self.machine.mem.take_mem_fault(); // never leak a debug-read fault into the VM
        out
    }

    fn memory_hex(&self, addr: u32, rows: usize) -> Vec<String> {
        let bytes = self.machine.mem.raw_bytes();
        let len = bytes.len() as u32;
        let mut out = Vec::with_capacity(rows);
        let mut a = addr.min(len);
        for _ in 0..rows {
            if a >= len { break; }
            let end = (a + 16).min(len);
            let row = &bytes[a as usize..end as usize];
            let hex: String = row.iter().map(|b| format!("{:02x} ", b)).collect();
            // VM-correct char column: basic ASCII printable range is a direct
            // identity mapping (same result zscii_to_char would give); the
            // 155-223 ZSCII extended range goes through the story's custom
            // Unicode table if it has one, else zvm's default ZSCII table
            // (mirrors decode_string's own zscii lookup in text/decode.rs).
            // Everything else (control bytes, unassigned ZSCII) is undecodable
            // as a single glyph → '.'.
            let ascii: String = row.iter()
                .map(|&b| match b {
                    0x20..=0x7e => b as char,
                    155..=223 => self.machine.mem.unicode_char(b as u16)
                        .unwrap_or_else(|| zvm::text::decode::zscii_to_char(b as u16)),
                    _ => '.',
                })
                .collect();
            out.push(format!("{:06x}  {:<48}{}", a, hex, ascii));
            a = end;
        }
        out
    }

    fn memory_len(&self) -> u32 {
        self.machine.mem.len() as u32
    }

    fn object_detail(&self, obj: u16) -> Vec<String> {
        let mem = &self.machine.mem;
        let attr_count: u8 = if mem.version() <= 3 { 32 } else { 48 };
        let attrs: Vec<u8> = (0..attr_count).filter(|&a| zvm::objects::get_attr(mem, obj, a)).collect();
        let mut out = Vec::new();
        // The tree row's `@0x……` points at the property table, where the name
        // is; the §12.3 entry — the attribute flags these next lines decode and
        // the tree links — is reached from here, as its own clickable jump
        // (SQ-0975). Both addresses stay one click from the object.
        out.push(format!("entry @0x{:06x}", zvm::objects::object_entry_addr(mem, obj)));
        if attrs.is_empty() {
            out.push("attrs: (none)".to_string());
        } else {
            let list: Vec<String> = attrs.iter().map(|a| a.to_string()).collect();
            out.push(format!("attrs: {}", list.join(", ")));
        }
        // Walk the property table. Properties are stored strictly descending
        // and there are at most 63, so a valid walk is short. A corrupt object
        // (e.g. one the table-bound heuristic mis-identified) could otherwise
        // make get_next_prop cycle or not descend — guard both ways so the
        // debugger never hangs expanding a bad object.
        let mut prop = zvm::objects::get_next_prop(mem, obj, 0);
        for _ in 0..64 {
            if prop == 0 { break; }
            let addr = zvm::objects::get_prop_addr(mem, obj, prop);
            let len = zvm::objects::get_prop_len(mem, addr);
            let bytes: Vec<String> = (0..len as u32)
                .map(|i| format!("{:02x}", mem.read_byte(addr as u32 + i)))
                .collect();
            out.push(format!("  prop {}: {}", prop, bytes.join(" ")));
            let next = zvm::objects::get_next_prop(mem, obj, prop);
            if next >= prop { break; } // must strictly descend; else corrupt
            prop = next;
        }
        self.machine.mem.take_mem_fault(); // never leak a debug-read fault into the VM
        out
    }

    fn frame_locals(&self, idx: usize) -> Vec<String> {
        match self.machine.state.frames.get(idx) {
            None => vec!["(no frame)".to_string()],
            Some(f) if f.locals.is_empty() => vec!["(no locals)".to_string()],
            Some(f) => f.locals.iter().enumerate()
                .map(|(i, w)| format!("local{i} = 0x{:04x}  ({})", w, *w as i16))
                .collect(),
        }
    }

    fn var_value(&self, var: u8) -> Option<u16> {
        let st = &self.machine.state;
        match var {
            0 => st.eval_stack.last().copied(), // peek the top; never pops
            1..=15 => st.frames.last()?.locals.get((var - 1) as usize).copied(),
            n => Some(self.machine.global(n - 16)),
        }
    }

    /// Decode the story's own Z-text over the same window `memory_hex` dumps,
    /// crediting each row with the text its own bytes produced.
    ///
    /// Two tables account for essentially every Z-string whose START address is
    /// knowable from the story's structure alone — which is the only kind that
    /// may be shown, since a decode begun anywhere else is wrong rather than
    /// offset (see [`Debugger::memory_zstrings`]):
    ///
    /// - **dictionary keys** — entries follow the dictionary header immediately
    ///   (ZMSD §13.5) at `base + i * entry_length`, a stride the *game* chose
    ///   (§13.2 gives only a minimum), so entries are very often at odd
    ///   addresses. The key fills the entry's first 4 bytes in v1–3 (§13.3) or 6
    ///   in v4+ (§13.4) and the rest of the entry is game data, not text. The
    ///   whole span is arithmetic — no scan, no decode, so a window nowhere near
    ///   the dictionary costs two comparisons.
    /// - **object short names** — not in the object entry at all but at the head
    ///   of the property table it points at, one byte past a count of the name's
    ///   Z-text words (§12.4, the same layout in every version). That count
    ///   gives the span without decoding, so the scan over objects is three
    ///   reads each and only names that overlap the window are decoded.
    ///
    /// Everything else — the abbreviation table's own strings, `print` literals
    /// inline in code, packed high strings — is deliberately left `None`. Their
    /// starts are only knowable by following a reference, and a plausible wrong
    /// decode is worse here than an honest blank.
    ///
    /// The decode is zvm's own and inherits its version coverage: the v3+ text
    /// rules (Z-chars 1–3 abbreviations, 4/5 one-shot shifts). A v1/v2 story,
    /// where 2/3 shift and 4/5 shift-*lock* (ZMSD §3.2.2–3), is read through
    /// those v3+ rules here exactly as it is everywhere else in the interpreter;
    /// a debug column is not the place to diverge from what the VM prints.
    fn memory_zstrings(&self, addr: u32, rows: usize) -> Vec<Option<String>> {
        let mem = &self.machine.mem;
        let len = mem.len() as u32;
        let start = addr.min(len);
        // Exactly the rows `memory_hex` emits, so the two vectors index alike:
        // it stops at the end of memory rather than padding.
        let n = rows.min(((len - start) as usize).div_ceil(16));
        let mut out: Vec<Option<String>> = vec![None; n];
        if n == 0 {
            return out;
        }
        let end = start + (n as u32 * 16).min(len - start);

        // Every string span [s, e) that overlaps the window, from the two tables.
        let mut spans: Vec<(u32, u32)> = Vec::new();
        let d = zvm::dictionary::load(mem);
        if d.count > 0 && d.entry_length > 0 {
            let elen = d.entry_length as u32;
            let klen = d.key_len() as u32;
            if end > d.base {
                // Index range by arithmetic: the entry holding `start` (0 when
                // the window opens before the table) through the one holding
                // `end - 1`, clamped to the real entry count.
                let lo = start.saturating_sub(d.base) / elen;
                let hi = ((end - 1 - d.base) / elen).min(d.count as u32 - 1);
                for i in lo..=hi {
                    let s = d.base + i * elen;
                    spans.push((s, s + klen));
                }
            }
        }
        for obj in 1..=zvm::location::max_object_number(mem) {
            if let Some(span) = zvm::objects::short_name_span(mem, obj) {
                spans.push(span);
            }
        }

        for (s, e) in spans {
            if s >= end || e <= start {
                continue; // no overlap with the window
            }
            let (frags, _) = zvm::text::decode::decode_string_words(mem, s);
            for (k, frag) in frags.iter().enumerate() {
                let word = s + 2 * k as u32;
                if word >= e {
                    break; // past the span the table declared; not this string's
                }
                // A word straddles a row boundary whenever its string starts on
                // an odd address, so credit it to the row holding its LAST byte
                // — the byte whose arrival completed it.
                let last = word + 1;
                if last < start || last >= end {
                    continue;
                }
                out[((last - start) / 16) as usize]
                    .get_or_insert_with(String::new)
                    .push_str(frag);
            }
        }
        // Deliberately untrimmed: a space at the end of a row is as often the
        // gap inside a name whose next word starts the next row ("brave " /
        // "adventurer") as it is padding, and the two are indistinguishable
        // here. A row that produced nothing visible at all (a lone shift word)
        // still reads as `Some("")` — we know those bytes are text, and that is
        // a different statement from `None`.
        self.machine.mem.take_mem_fault(); // never leak a debug-read fault into the VM
        out
    }
}

/// Render the object hierarchy as indented `[N] name` lines in **tree order**:
/// a depth-first walk from each root (parent 0, ascending) down each object's
/// child chain (child, then that child's siblings), so every object sits
/// directly beneath its parent. Pure over the link/name lookups so it is
/// unit-testable without a `Machine`. Guards against malformed data: a `seen`
/// set breaks parent/child/sibling cycles, and any object never reached from a
/// root (a broken link) is still appended (at its parent-chain depth) so
/// nothing silently disappears.
fn build_object_tree(
    numbers: &[u16],
    parent: impl Fn(u16) -> u16,
    child: impl Fn(u16) -> u16,
    sibling: impl Fn(u16) -> u16,
    name: impl Fn(u16) -> String,
    addr: impl Fn(u16) -> u32,
) -> Vec<String> {
    let mut out = Vec::with_capacity(numbers.len());
    let mut seen = std::collections::HashSet::new();
    // Roots pushed in reverse so ascending roots emit first (stack pops LIFO).
    let mut stack: Vec<(u16, usize)> = numbers.iter().rev()
        .filter(|&&o| parent(o) == 0)
        .map(|&o| (o, 0usize))
        .collect();
    while let Some((obj, depth)) = stack.pop() {
        if obj == 0 || depth > 64 || !seen.insert(obj) {
            continue;
        }
        out.push(format!("@0x{:06x} {}[{}] {}", addr(obj), "  ".repeat(depth), obj, name(obj)));
        // Collect this object's child chain, then push reversed so the first
        // child is visited first. `!kids.contains` + `!seen` guard cycles.
        let mut kids = Vec::new();
        let mut c = child(obj);
        while c != 0 && !seen.contains(&c) && !kids.contains(&c) {
            kids.push(c);
            c = sibling(c);
        }
        for &k in kids.iter().rev() {
            stack.push((k, depth + 1));
        }
    }
    // Safety net: objects unreachable from any root still appear, at their
    // parent-chain depth, in ascending number order.
    for &o in numbers {
        if seen.insert(o) {
            let mut depth = 0usize;
            let mut p = parent(o);
            while p != 0 && depth < 64 {
                depth += 1;
                p = parent(p);
            }
            out.push(format!("@0x{:06x} {}[{}] {}", addr(o), "  ".repeat(depth), o, name(o)));
        }
    }
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mapper::direction::Direction;

    // ── Object tree ordering ──────────────────────────────────────────────────

    #[test]
    fn build_object_tree_walks_children_under_their_parent() {
        // Two roots (1, 2). 1's children are 3 then 5 (siblings); 3 has child 4;
        // 2 has child 6. A numeric-order+indent render would emit 1,2,3,4,5,6 —
        // this DFS must nest each child directly under its parent instead.
        use std::collections::HashMap;
        let parent: HashMap<u16, u16> = [(1, 0), (2, 0), (3, 1), (4, 3), (5, 1), (6, 2)].into();
        let child: HashMap<u16, u16> = [(1, 3), (2, 6), (3, 4), (4, 0), (5, 0), (6, 0)].into();
        let sibling: HashMap<u16, u16> = [(1, 0), (2, 0), (3, 5), (4, 0), (5, 0), (6, 0)].into();
        let lines = build_object_tree(
            &[1, 2, 3, 4, 5, 6],
            |o| parent[&o], |o| child[&o], |o| sibling[&o], |o| format!("o{o}"),
            |o| 0x100 + o as u32,
        );
        assert_eq!(lines, vec![
            "@0x000101 [1] o1".to_string(),
            "@0x000103   [3] o3".to_string(),
            "@0x000104     [4] o4".to_string(),
            "@0x000105   [5] o5".to_string(),
            "@0x000102 [2] o2".to_string(),
            "@0x000106   [6] o6".to_string(),
        ]);
    }

    #[test]
    fn build_object_tree_appends_objects_unreachable_from_a_root() {
        // Object 3 claims parent 2, but 2 has no child pointing back — a broken
        // link. It must still appear (at its parent-chain depth), never vanish.
        use std::collections::HashMap;
        let parent: HashMap<u16, u16> = [(1, 0), (2, 0), (3, 2)].into();
        let child: HashMap<u16, u16> = [(1, 0), (2, 0), (3, 0)].into();
        let sibling: HashMap<u16, u16> = [(1, 0), (2, 0), (3, 0)].into();
        let lines = build_object_tree(
            &[1, 2, 3],
            |o| parent[&o], |o| child[&o], |o| sibling[&o], |o| format!("o{o}"),
            |o| 0x200 + o as u32,
        );
        assert_eq!(lines, vec![
            "@0x000201 [1] o1".to_string(),
            "@0x000202 [2] o2".to_string(),
            "@0x000203   [3] o3".to_string(), // appended, depth 1 (parent 2)
        ]);
    }

    // ── CaptureSink style-run capture ─────────────────────────────────────────

    #[test]
    fn capture_sink_records_style_runs() {
        use zvm::io::Output;
        use zvm::screen::ZColour;
        let mut s = CaptureSink::new();
        s.print("ab");
        s.print_styled("CD", 0x02);
        let (text, runs) = s.take_styled();
        assert_eq!(text, "abCD");
        assert_eq!(runs, vec![
            (2, 0, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false),
            (2, 0x02, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false),
        ]);
    }

    /// ZMSD §7.2.1: buffering is on at the start of a game, and `buffer_mode`
    /// switches it. Runs printed while it is OFF are flagged so the transcript
    /// can char-break them.
    #[test]
    fn capture_sink_flags_runs_printed_with_buffering_off() {
        use zvm::io::Output;
        let mut s = CaptureSink::new();
        s.print("on");
        s.set_buffer_mode(false);
        s.print("off");
        s.print_styled("OFF", 0x02);
        s.set_buffer_mode(true);
        s.print("on again");
        let (text, runs) = s.take_styled();
        assert_eq!(text, "onoffOFFon again");
        let flags: Vec<(usize, bool)> = runs.iter().map(|r| (r.0, r.7)).collect();
        assert_eq!(flags, vec![(2, false), (3, true), (3, true), (8, false)]);
    }

    #[test]
    fn interleave_story_elems_splits_at_line_starts_and_keeps_runs_synced() {
        use crate::inline_image::{ImageAlign, InlineImage};
        let img = InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::new(8, 8)),
            align: ImageAlign::MarginLeft,
            scaled: None,
            margin_px: Some(56),
        };
        let text = "first line\nsecond line";
        // One style chunk covering everything (bold), to verify run splitting.
        let runs = vec![(text.chars().count(), 2u8, ZColour::Default, ZColour::Default, 0u32, ParaFmt::default(), 0u8, false)];
        // Drawn at abs offset base+15 — mid-"second line" — must SNAP to that
        // line's start (offset 11), splitting cleanly at the line boundary.
        let elems = interleave_story_elems(text, &runs, vec![(115, TranscriptElem::Image(img))], 100, None);
        assert_eq!(elems.len(), 3, "Text, Image, Text");
        let TranscriptElem::Text { text: t0, runs: r0 } = &elems[0] else { panic!("elem 0 is Text") };
        assert_eq!(t0, "first line", "separator dropped — element boundary is the break");
        assert_eq!(r0.iter().map(|r| r.0).sum::<usize>(), 10, "runs cover exactly the chunk");
        assert!(matches!(&elems[1], TranscriptElem::Image(i) if i.margin_px == Some(56)));
        let TranscriptElem::Text { text: t2, runs: r2 } = &elems[2] else { panic!("elem 2 is Text") };
        assert_eq!(t2, "second line");
        assert_eq!(r2.iter().map(|r| r.0).sum::<usize>(), 11, "tail runs cover the tail (separator char consumed)");
    }

    #[test]
    fn interleave_story_elems_at_start_needs_no_split() {
        use crate::inline_image::{ImageAlign, InlineImage};
        let img = InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::new(8, 8)),
            align: ImageAlign::MarginLeft,
            scaled: None,
            margin_px: None,
        };
        let elems = interleave_story_elems("story text", &[], vec![(0, TranscriptElem::Image(img))], 0, None);
        assert_eq!(elems.len(), 2, "Image then Text");
        assert!(matches!(&elems[0], TranscriptElem::Image(_)));
        assert!(matches!(&elems[1], TranscriptElem::Text { text, .. } if text == "story text"));
    }

    // ── content-art classification (SQ-0461 decision 3) ───────────────────────

    #[test]
    fn content_art_shogun_title_splash_is_content() {
        // Shogun's title: a full 320×200 splash on a 320×200 screen → 100% area.
        assert!(is_content_art(320, 200, 320, 200));
    }

    #[test]
    fn win0_ship_splash_is_inline_but_dropcap_stays_margin() {
        use crate::inline_image::ImageAlign;
        // No margin reservation (left=right=0, picture at the left): a large
        // centred illustration is a full-size inline band — NOT a drop-cap
        // (SQ-0471) and NOT a right-margin float.
        let noma = |iw, ih, sw: u32, sh| win0_pic_align(iw, ih, sw, sh, 1, 0, 0, sw as u16);
        assert_eq!(noma(320, 200, 320, 200), ImageAlign::InlineUp, "ship splash → inline");
        assert_eq!(noma(288, 176, 320, 200), ImageAlign::InlineUp, "big room illustration → inline");
        // A genuine drop-cap (Zork Zero's initial letter / a small tile) stays a
        // left-margin float — the existing drop-cap behaviour is preserved.
        assert_eq!(noma(24, 32, 320, 200), ImageAlign::MarginLeft, "small drop-cap stays margin");
        assert_eq!(noma(40, 48, 320, 200), ImageAlign::MarginLeft, "small icon stays margin");
    }

    #[test]
    fn win0_shogun_opening_is_a_right_margin_float() {
        use crate::inline_image::ImageAlign;
        // SQ-0489: Shogun's opening — draw_picture(7) at window-x 229 in the 548px
        // window, then set_margins(left=2, right=328). Text flows in the left
        // ~220px column, the picture sits on the right → MarginRight float, NOT the
        // old full-width InlineUp band.
        assert_eq!(
            win0_pic_align(320, 368, 640, 400, 229, 2, 328, 548),
            ImageAlign::MarginRight,
            "right-margin picture floats right with prose beside it"
        );
        // A thin symmetric frame inset (Zork Zero's ~36px side columns) is NOT a
        // right float — a small drop-cap there keeps MarginLeft.
        assert_eq!(
            win0_pic_align(40, 48, 512, 400, 40, 36, 36, 512),
            ImageAlign::MarginLeft,
            "symmetric frame inset is not a right float"
        );
    }

    #[test]
    fn content_art_shogun_side_border_is_frame() {
        // Shogun's 23px-wide side border on a 320×200 screen: width 23 ≤ 48 (15%).
        assert!(!is_content_art(23, 200, 320, 200));
    }

    #[test]
    fn content_art_zork0_compass_tiles_are_frame() {
        // Zork Zero's compass/frame tiles (pictures 9..24) are small squares —
        // neither 40% area nor 60%×30% of the screen.
        for dim in [9u32, 12, 16, 20, 24] {
            assert!(!is_content_art(dim, dim, 320, 200), "{dim}×{dim} tile must be frame art");
        }
    }

    #[test]
    fn content_art_wide_short_band_is_frame() {
        // A full-width but shallow banner (e.g. 320×20 = 10% height, 32% area) is
        // decorative, not content.
        assert!(!is_content_art(320, 20, 320, 200));
    }

    #[test]
    fn content_art_room_illustration_is_content() {
        // A 220×120 room picture: 220/320 = 69% width, 120/200 = 60% height → content
        // (also 41% area).
        assert!(is_content_art(220, 120, 320, 200));
    }

    #[test]
    fn clamp_runs_trims_to_char_len() {
        use zvm::screen::ZColour;
        // strip_read_prompt removed 3 trailing chars ("\n> " etc.) → clamp.
        let runs = vec![
            (2, 0u8, ZColour::Default, ZColour::Default, 0u32, ParaFmt::default(), 0, false),
            (5, 0x02u8, ZColour::Default, ZColour::Default, 0u32, ParaFmt::default(), 0, false),
        ];
        assert_eq!(clamp_runs(runs, 4), vec![
            (2, 0, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false),
            (2, 0x02, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false),
        ]);
    }

    fn dummy_inline_image() -> crate::inline_image::InlineImage {
        crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::new(2, 2)),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None,
        }
    }

    #[test]
    fn trim_elems_strips_trailing_prompt_from_last_text() {
        use zvm::screen::ZColour;
        // raw ends in "\n> " — strip_read_prompt shortens it; the LAST Text
        // element (and its runs) must be trimmed to match the flat stripped text.
        let raw = "You see a rock.\n> ";
        let kept = strip_read_prompt(raw).chars().count();
        let mut elems = vec![TranscriptElem::Text {
            text: raw.to_string(),
            runs: vec![(raw.chars().count(), 0, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false)],
        }];
        trim_elems_to_len(&mut elems, kept);
        let TranscriptElem::Text { text, runs } = &elems[0] else { panic!("expected Text") };
        assert_eq!(text, "You see a rock.");
        assert_eq!(runs.iter().map(|r| r.0).sum::<usize>(), kept);
    }

    #[test]
    fn trim_elems_reaches_across_image_to_reach_length() {
        use zvm::screen::ZColour;
        // Text("foo\n"), Image, Text(">") — flat text "foo\n>" strips to "foo".
        // The trim clears the trailing ">" element and reaches back past the
        // image to trim the "\n" off "foo\n".
        let mut elems = vec![
            TranscriptElem::Text { text: "foo\n".into(), runs: vec![(4, 0, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false)] },
            TranscriptElem::Image(dummy_inline_image()),
            TranscriptElem::Text { text: ">".into(), runs: vec![(1, 0, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false)] },
        ];
        trim_elems_to_len(&mut elems, 3);
        let TranscriptElem::Text { text, .. } = &elems[0] else { panic!("expected Text") };
        assert_eq!(text, "foo");
        assert!(matches!(&elems[1], TranscriptElem::Image(_)));
        let TranscriptElem::Text { text, .. } = &elems[2] else { panic!("expected Text") };
        assert_eq!(text, "");
    }

    // ── Pure bridge test ──────────────────────────────────────────────────────

    #[test]
    fn apply_turn_bridge_sets_current_and_creates_edge() {
        let mut m = Mapper::default();

        // First observation: set current room (no prior → no edge).
        let first = TurnResult {
            transcript: String::new(),
            transcript_runs: Vec::new(),
            location: Some(ObjectSnapshot { number: 1, parent: 0, name: "Hall".into() }),
            quit: false,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops: Vec::new(),
            diagnostics: vec![],
            fault: None,
            location_method: None,
            pending_io: None,
            timed_out: false,
            pictures: Vec::new(),
            transcript_elems: Vec::new(),
            prose_retired: None,
        };
        apply_turn(&mut m, "look", &first, &mut Default::default());
        assert_eq!(m.graph.current(), Some(1));
        assert!(m.graph.room(1).is_some());
        assert_eq!(m.graph.connections().len(), 0, "first observe must not create edge");

        // Second observation: move north → directed N edge 1→2.
        let second = TurnResult {
            transcript: String::new(),
            transcript_runs: Vec::new(),
            location: Some(ObjectSnapshot { number: 2, parent: 0, name: "Attic".into() }),
            quit: false,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops: Vec::new(),
            diagnostics: vec![],
            fault: None,
            location_method: None,
            pending_io: None,
            timed_out: false,
            pictures: Vec::new(),
            transcript_elems: Vec::new(),
            prose_retired: None,
        };
        apply_turn(&mut m, "north", &second, &mut Default::default());
        assert!(m.graph.room(2).is_some());
        assert_eq!(m.graph.current(), Some(2));

        let conns = m.graph.connections();
        assert_eq!(conns.len(), 1);
        assert_eq!(conns[0].origin, 1);
        assert_eq!(conns[0].dir, Direction::N);
        assert_eq!(conns[0].dest, 2);
    }

    /// SQ-0576: a compass click submits no text, but the game echoes the
    /// command it synthesized as the first output line — that echo (and only a
    /// whole-line echo) is adopted as the turn's movement command.
    #[test]
    fn echoed_direction_command_accepts_only_whole_line_moves() {
        use super::echoed_direction_command as echo;
        // Zork Zero's compass-click shape: the direction alone, then room text.
        assert_eq!(echo("north\nEntrance Hall\n   This is where..."), Some("north"));
        assert_eq!(echo("\n  southwest  \nSomewhere."), Some("southwest"));
        assert_eq!(echo("go north\nSomewhere."), Some("go north"));
        // A room HEADING beginning with a direction word must NOT read as a
        // move: `parse_direction` matches the first token, so only the
        // whole-line rule stands between "North of House" and a false N edge.
        assert_eq!(echo("North of House\nYou are standing..."), None);
        assert_eq!(echo("Aft Storage\nA cramped hold."), None);
        assert_eq!(echo("go north now\n..."), None);
        assert_eq!(echo("You wake up.\nIt is dark."), None);
        assert_eq!(echo("look\nGallery"), None);
        assert_eq!(echo(""), None);
    }

    #[test]
    fn apply_turn_noop_when_location_none() {
        let mut m = Mapper::default();
        let result = TurnResult {
            transcript: String::new(),
            transcript_runs: Vec::new(),
            location: None,
            quit: false,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops: Vec::new(),
            diagnostics: vec![],
            fault: None,
            location_method: None,
            pending_io: None,
            timed_out: false,
            pictures: Vec::new(),
            transcript_elems: Vec::new(),
            prose_retired: None,
        };
        apply_turn(&mut m, "look", &result, &mut Default::default());
        assert_eq!(m.graph.current(), None);
    }

    #[test]
    fn is_death_relocation_matches_infocom_and_inform_banners_only() {
        // Infocom's spaced banner (verified against a real Zork I grue death).
        assert!(is_death_relocation(
            "Oh, no! A lurking grue slithered into the room and devoured you!\n \n   ****  You have died  **** \n\nForest\n"
        ));
        // Inform's tight banner.
        assert!(is_death_relocation("*** You have died ***"));
        // The winning banner changes no room — must NOT be treated as a relocation.
        assert!(!is_death_relocation("*** You have won ***"));
        // The pitch-black warning (a legit move) has no banner — must NOT match.
        assert!(!is_death_relocation(
            "It is pitch black. You are likely to be eaten by a grue."
        ));
        // Ordinary room prose mentioning the dead must NOT match.
        assert!(!is_death_relocation("A dead body lies in the corner of the crypt."));
    }

    #[test]
    fn apply_turn_death_records_relocation_not_a_directional_edge() {
        // A typed "north" that triggers a grue death + resurrection into Forest must
        // NOT mint a false N-edge Cellar→Forest. (SQ-0259)
        let mk = |num: u16, name: &str, transcript: &str| TurnResult {
            transcript: transcript.into(),
            transcript_runs: Vec::new(),
            location: Some(ObjectSnapshot { number: num, parent: 0, name: name.into() }),
            quit: false,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops: Vec::new(),
            diagnostics: vec![],
            fault: None,
            location_method: None,
            pending_io: None,
            timed_out: false,
            pictures: Vec::new(),
            transcript_elems: Vec::new(),
            prose_retired: None,
        };
        let mut m = Mapper::default();
        apply_turn(&mut m, "", &mk(1, "Living Room", "Living Room\n"), &mut Default::default());
        apply_turn(
            &mut m,
            "down",
            &mk(2, "Cellar", "You have moved into a dark place.\n"),
            &mut Default::default(),
        );
        let edges_before = m.graph.connections().len();
        // The fatal move: resurrection room arrives on the same turn as the banner.
        apply_turn(
            &mut m,
            "north",
            &mk(3, "Forest", "   ****  You have died  **** \n\nForest\n"),
            &mut Default::default(),
        );
        assert_eq!(m.graph.current(), Some(3), "player is now in the resurrection room");
        assert_eq!(
            m.graph.connections().len(),
            edges_before,
            "the death move must not add any edge (no false Cellar→Forest passage)"
        );
        assert!(
            !m.graph.connections().iter().any(|c| c.origin == 2 && c.dest == 3),
            "no edge from the room we died in to the resurrection room"
        );
    }

    #[test]
    fn apply_turn_gates_nameonly_until_first_real_room() {
        // BeyondZork VT220 setup shows the player's name ("Frank Booth") in a
        // status-line-shaped character sheet → NameOnly. It must NOT seed the
        // map before real play establishes an object-backed room.
        let mk = |method: Option<LocationMethod>, num: u16, name: &str| TurnResult {
            transcript: String::new(),
            transcript_runs: Vec::new(),
            location: Some(ObjectSnapshot { number: num, parent: 0, name: name.into() }),
            quit: false,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops: Vec::new(),
            diagnostics: vec![],
            fault: None,
            location_method: method,
            pending_io: None,
            timed_out: false,
            pictures: Vec::new(),
            transcript_elems: Vec::new(),
            prose_retired: None,
        };

        let mut m = Mapper::default();

        // 1. Pre-game NameOnly on an empty map → suppressed.
        apply_turn(
            &mut m,
            "",
            &mk(Some(LocationMethod::NameOnly), 111, "Frank Booth"),
            &mut Default::default(),
        );
        assert_eq!(m.graph.rooms().count(), 0, "NameOnly must not seed an empty map");
        assert_eq!(m.graph.current(), None);

        // 2. Real play: an object-backed room is observed.
        apply_turn(
            &mut m,
            "",
            &mk(Some(LocationMethod::PlayerParent), 48, "Hilltop"),
            &mut Default::default(),
        );
        assert_eq!(m.graph.current(), Some(48));
        assert_eq!(m.graph.rooms().count(), 1);

        // 3. NameOnly is now trusted as a mid-game fallback (map non-empty).
        apply_turn(
            &mut m,
            "north",
            &mk(Some(LocationMethod::NameOnly), 222, "Foggy Place"),
            &mut Default::default(),
        );
        assert_eq!(m.graph.current(), Some(222));
        assert_eq!(m.graph.rooms().count(), 2);
    }

    #[test]
    fn apply_turn_observes_roomheading_on_empty_map() {
        // Glulx rooms use RoomHeading (never NameOnly) precisely so the
        // NameOnly-empty-graph gate does NOT suppress the first Glulx room —
        // a Glulx game never produces an object-backed room to un-gate it.
        let mut m = Mapper::default();
        let result = TurnResult {
            transcript: String::new(),
            transcript_runs: Vec::new(),
            location: Some(ObjectSnapshot { number: 333, parent: 0, name: "Orbiting Boony".into() }),
            quit: false,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops: Vec::new(),
            diagnostics: vec![],
            fault: None,
            location_method: Some(LocationMethod::RoomHeading),
            pending_io: None,
            timed_out: false,
            pictures: Vec::new(),
            transcript_elems: Vec::new(),
            prose_retired: None,
        };
        apply_turn(&mut m, "", &result, &mut Default::default());
        assert_eq!(m.graph.current(), Some(333));
        assert_eq!(m.graph.rooms().count(), 1);
    }

    // ── TurnResult.info tests ─────────────────────────────────────────────────

    #[test]
    fn turn_result_info_defaults_none_for_normal_turn() {
        // A TurnResult from a normal turn has info == None by default.
        let r = TurnResult {
            transcript: "You are in a maze.".to_string(),
            transcript_runs: Vec::new(),
            location: None,
            quit: false,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops: Vec::new(),
            diagnostics: vec![],
            fault: None,
            location_method: None,
            pending_io: None,
            timed_out: false,
            pictures: Vec::new(),
            transcript_elems: Vec::new(),
            prose_retired: None,
        };
        assert!(r.info.is_none());
    }

    // ── Task-5 overlap cleanup tests ──────────────────────────────────────────

    /// Helper: build a TurnResult with a location (mirrors the pattern used above).
    fn turn(number: u16, name: &str) -> TurnResult {
        TurnResult {
            transcript: String::new(),
            transcript_runs: Vec::new(),
            location: Some(ObjectSnapshot { number, parent: 0, name: name.into() }),
            quit: false,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops: Vec::new(),
            diagnostics: vec![],
            fault: None,
            location_method: None,
            pending_io: None,
            timed_out: false,
            pictures: Vec::new(),
            transcript_elems: Vec::new(),
            prose_retired: None,
        }
    }

    #[test]
    fn auto_mode_background_cleanup_keeps_map_free_of_illegal_overlaps() {
        // Drive a small loop (E, N, W, S toward start) that — under incremental
        // placement — can produce a routing overlap.  `apply_turn` no longer cleans
        // overlaps inline (that is background map work now); running the background
        // cleanup the run loop schedules must leave zero illegal overlaps.
        let mut m = Mapper::default(); // Auto mode by default

        apply_turn(&mut m, "look", &turn(1, "Start"), &mut Default::default());
        apply_turn(&mut m, "east", &turn(2, "East Room"), &mut Default::default());
        apply_turn(&mut m, "north", &turn(3, "North East Room"), &mut Default::default());
        apply_turn(&mut m, "west", &turn(4, "North Room"), &mut Default::default());
        apply_turn(
            &mut m,
            "south",
            &turn(1, "Start"),
            &mut Default::default(),
        ); // back to start — closes the loop

        crate::tidy::cleanup_overlaps_layer_silent(&mut m.graph, mapper::layer::MAIN_LAYER);

        let (illegal, _) = crate::render::map::render_overlap_stats(&m.graph);
        assert_eq!(illegal, 0, "background cleanup must leave zero illegal overlaps");
    }

    // ── Task 7: InputKind / submit_char tests ─────────────────────────────────

    /// Build a minimal v5 story whose program is: read_char (store→G0), quit.
    ///
    /// GameSession::new will step until the first NeedChar, so pending_input()
    /// must be `Char`.  Calling submit_char advances past the read_char and
    /// hits the quit opcode, returning a TurnResult.
    fn read_char_story_v5() -> Vec<u8> {
        let mut buf = vec![0u8; 0x0800];
        // Version 5
        buf[0x00] = 5;
        // high_mem_base = 0x0400
        buf[0x04] = 0x04; buf[0x05] = 0x00;
        // initial_pc = 0x0040
        buf[0x06] = 0x00; buf[0x07] = 0x40;
        // dictionary = 0x0080 (empty: word-sep=0, entry-size=4, entry-count=0)
        buf[0x08] = 0x00; buf[0x09] = 0x80;
        buf[0x0080] = 0; buf[0x0081] = 4; buf[0x0082] = 0; buf[0x0083] = 0;
        // object_table = 0x0100
        buf[0x0A] = 0x01; buf[0x0B] = 0x00;
        // global_vars = 0x0300
        buf[0x0C] = 0x03; buf[0x0D] = 0x00;
        // static_mem_base = 0x0400 → dynamic memory 0x0000–0x03FF
        buf[0x0E] = 0x04; buf[0x0F] = 0x00;
        // abbrev_table = 0x0060
        buf[0x18] = 0x00; buf[0x19] = 0x60;

        // Program at 0x0040:
        //   read_char (VAR opcode 0xF6)
        //     type byte 0x7F: small-const(01), omit(11), omit(11), omit(11)
        //     operand: 1 (keyboard device)
        //     store: 0x10 (G0)
        //   quit (0xBA)
        buf[0x0040] = 0xF6; // VAR read_char
        buf[0x0041] = 0x7F; // type: small(01), omit(11), omit(11), omit(11)
        buf[0x0042] = 1;    // operand: device=1
        buf[0x0043] = 0x10; // store → G0
        buf[0x0044] = 0xBA; // quit

        buf
    }

    /// Timed variant of `read_char_story_v5`: `read_char(device=1, time=5,
    /// routine=packed(0x0050))` -> G0, then `quit`. `routine_body` is placed at
    /// 0x0050 (0 locals) so the caller can make it `rtrue` (abort) or do a
    /// side-effect + `rfalse` (continue). Packed routine address = 0x0050/4 =
    /// 0x0014 (v5 packed multiplier is 4).
    fn timed_read_char_story_v5(routine_body: &[u8]) -> Vec<u8> {
        let mut buf = read_char_story_v5();
        // Program at 0x0040:
        //   read_char (VAR opcode 0xF6)
        //     type byte 0x53: small(01)=device, small(01)=time, large(00)=routine, omit(11)
        //     operands: device=1, time=5, routine=packed(0x0050)=0x0014
        //     store: 0x10 (G0)
        //   quit (0xBA)
        buf[0x0040] = 0xF6; // VAR read_char
        buf[0x0041] = 0x53; // types: small, small, large, omit
        buf[0x0042] = 1;    // device = 1 (keyboard)
        buf[0x0043] = 5;    // time = 5 (tenths of a second)
        buf[0x0044] = 0x00;
        buf[0x0045] = 0x14; // routine packed addr = 0x0050 / 4
        buf[0x0046] = 0x10; // store → G0
        buf[0x0047] = 0xBA; // quit

        // Routine at 0x0050: header byte = 0 locals, then routine_body.
        buf[0x0050] = 0x00;
        for (i, b) in routine_body.iter().enumerate() {
            buf[0x0051 + i] = *b;
        }
        buf
    }

    #[test]
    fn pending_input_is_line_after_new_on_quitting_story() {
        // czech.z5 quits without ever requesting input; the quit path in
        // run_until_input returns InputKind::Line, so pending_input() == Line.
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        if !fixture_path.exists() {
            return; // fixture absent — skip
        }
        let story = std::fs::read(&fixture_path).expect("read czech.z5");
        let session = GameSession::new(story, true, false, None).expect("GameSession::new with czech.z5");
        assert_eq!(session.pending_input(), InputKind::Line,
            "a story that quits without requesting input should leave pending == Line");
    }

    #[test]
    fn v5_start_room_detected_at_boot() {
        // Regression: v4+ games have no location global, so `current_location`
        // must use version-aware detection at boot. With the old global-0 read it
        // returned None for v5 and the starting room stayed off the map until the
        // first turn. Skips when the (git-ignored) story is absent.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/zork1-invclues-r52-s871125.z5");
        if !path.exists() {
            return; // story absent — skip
        }
        let story = std::fs::read(&path).expect("read zork1 r52");
        let session = GameSession::new(story, false, false, None).expect("GameSession::new");
        let loc = session.current_location().expect("v5 starting room must be detected at boot");
        assert!(loc.name.starts_with("West"), "expected West of House, got {:?}", loc.name);
    }

    #[test]
    fn new_with_trace_captures_boot_pcs() {
        // --debug (SQ-0449) traces from the first boot instruction, so the
        // cumulative set is non-empty even before any player turn — capturing the
        // boot/init code a mid-game /debug can never see.
        let story = read_char_story_v5();
        let traced = GameSession::new_with_trace(story.clone(), false, false, None, true, Vec::new(), None, None, None)
            .expect("traced session");
        assert!(!traced.machine.ever_exec_pcs.is_empty(),
            "boot PCs must be captured when tracing from boot");
        // Without tracing, the cumulative set stays empty until a traced turn runs.
        let untraced = GameSession::new(story, false, false, None).expect("untraced session");
        assert!(untraced.machine.ever_exec_pcs.is_empty(),
            "no capture without --debug");
    }

    /// Build a minimal v6 story whose "main" routine (header 0x06/0x07, a packed
    /// routine address per ZMSD §5.5) is `quit` with 0 locals. Just enough for
    /// `Machine::with_output`'s v6 arm (which calls `main` via `call_routine`
    /// before the boot loop runs) to construct without faulting.
    fn v6_boot_stub_story() -> Vec<u8> {
        let mut buf = vec![0u8; 0x0800];
        buf[0x00] = 6; // version
        buf[0x04] = 0x04; buf[0x05] = 0x00; // high_mem_base = 0x0400
        // header 0x06/0x07 = main's packed address. routines_offset (0x28/0x29)
        // is 0, so unpack_routine(p) = 4*p; routine at 0x0100 -> packed 0x0040.
        buf[0x06] = 0x00; buf[0x07] = 0x40;
        // dictionary = 0x0080 (empty: word-sep=0, entry-size=4, entry-count=0)
        buf[0x08] = 0x00; buf[0x09] = 0x80;
        buf[0x0080] = 0; buf[0x0081] = 4; buf[0x0082] = 0; buf[0x0083] = 0;
        buf[0x0A] = 0x01; buf[0x0B] = 0x00; // object_table = 0x0100 (unused by this stub)
        buf[0x0C] = 0x03; buf[0x0D] = 0x00; // global_vars = 0x0300
        buf[0x0E] = 0x04; buf[0x0F] = 0x00; // static_mem_base = 0x0400
        buf[0x18] = 0x00; buf[0x19] = 0x60; // abbrev_table = 0x0060
        // main routine at 0x0100: 0 locals, then `quit` (0OP:0x0A, opcode byte 0xBA).
        buf[0x0100] = 0; // local count
        buf[0x0101] = 0xBA; // quit
        buf
    }

    #[test]
    fn v6_session_injects_picture_dims_before_boot() {
        // The v6 picture-dimension table must be set on `Machine` BEFORE the
        // boot run (picture_data is called during boot, which happens inside
        // new_with_trace itself — the Phase 0 boot-tracing lesson), so it must
        // be visible on the constructed session even for a story that quits
        // immediately in its main routine. For v6 the dims are scaled into unit
        // space by V6_ART_SCALE (SQ-0479): `picture_data` reports the doubled
        // sizes the game lays out on the 640×400 screen with — but only when the
        // Blorb declares a standard window for them to be scaled AGAINST.
        let dims = vec![(5u16, 100u16, 60u16), (9u16, 20u16, 30u16)];

        // A Blorb `Reso` standard window (every Infocom v6 title): the screen is
        // that window doubled, so the art doubles with it.
        let session = GameSession::new_with_trace(v6_boot_stub_story(), false, false, None, false, dims.clone(), Some((320, 200)), None, None)
            .expect("v6 session");
        assert_eq!(session.machine.picture_dims, vec![(5, 200, 120), (9, 40, 60)]);

        // No `Reso` at all (scopa.blb): Blorb §11 makes every image in the file
        // non-scalable — "always displayed at their actual size. (One image pixel
        // per screen pixel.)" — so the game is told the truth (SQ-0715).
        let session = GameSession::new_with_trace(v6_boot_stub_story(), false, false, None, false, dims.clone(), None, None, None)
            .expect("v6 session");
        assert_eq!(session.machine.picture_dims, dims);
    }

    /// SQ-0532/A-F1. ZMSD §8.4: the interpreter "may change the exact dimensions
    /// whenever it likes but must write the current height (in lines) and width
    /// (in characters) into bytes $20 and $21 in the header." Feeding the host's
    /// measured pane size through `Engine::set_screen_dims` must land there — and
    /// land again on every later resize, not just once at startup.
    #[test]
    fn set_screen_dims_publishes_the_host_pane_size_and_tracks_resizes() {
        let mut s = GameSession::new(read_char_story_v5(), false, false, None).expect("v5 session");
        Engine::set_screen_dims(&mut s, 40, 132);
        assert_eq!(s.machine.mem.read_byte(0x20), 40, "$20 = height in lines");
        assert_eq!(s.machine.mem.read_byte(0x21), 132, "$21 = width in characters");
        // §8.4.3: "In Version 5 and later, the screen's width and height in units
        // should be written to the words at $22 and $24."
        assert_eq!(s.machine.mem.read_word(0x22), 132, "$22 = width in units");
        assert_eq!(s.machine.mem.read_word(0x24), 40, "$24 = height in units");

        // A terminal resize re-reports; the header follows rather than sticking.
        Engine::set_screen_dims(&mut s, 24, 60);
        assert_eq!(
            (s.machine.mem.read_byte(0x20), s.machine.mem.read_byte(0x21)),
            (24, 60),
            "a resize re-writes both bytes"
        );
        assert_eq!((s.machine.mem.read_word(0x22), s.machine.mem.read_word(0x24)), (60, 24));

        // Degenerate sizes never publish a zero screen.
        Engine::set_screen_dims(&mut s, 0, 0);
        assert_eq!((s.machine.mem.read_byte(0x20), s.machine.mem.read_byte(0x21)), (1, 1));
    }

    /// A-F1(d): v6 lays out on its own fixed 640×400 pixel screen (seeded before
    /// boot from the Blorb `Reso` window) and the app SCALES that frame into
    /// whatever pane it has, so the pane feed must not touch it.
    #[test]
    fn set_screen_dims_leaves_the_v6_pixel_screen_alone() {
        let mut s = GameSession::new_with_trace(v6_boot_stub_story(), false, false, None, false, Vec::new(), None, None, None)
            .expect("v6 session");
        let before = (
            s.machine.mem.read_byte(0x20),
            s.machine.mem.read_byte(0x21),
            s.machine.mem.read_word(0x22),
            s.machine.mem.read_word(0x24),
        );
        Engine::set_screen_dims(&mut s, 45, 200);
        let after = (
            s.machine.mem.read_byte(0x20),
            s.machine.mem.read_byte(0x21),
            s.machine.mem.read_word(0x22),
            s.machine.mem.read_word(0x24),
        );
        assert_eq!(before, after, "a v6 story keeps its native screen dimensions");
    }

    /// Build a minimal v5 story. In v5 header 0x06/0x07 is the initial PC as a
    /// BYTE address (ZMSD §5.5 — packed only for the v6 `main` routine), so it
    /// points straight at an instruction: `quit` (0OP:0x0A, opcode byte 0xBA).
    fn v5_stub_story() -> Vec<u8> {
        let mut buf = vec![0u8; 0x0800];
        buf[0x00] = 5; // version
        buf[0x04] = 0x04; buf[0x05] = 0x00; // high_mem_base = 0x0400
        buf[0x06] = 0x01; buf[0x07] = 0x00; // initial PC = 0x0100
        // dictionary = 0x0080 (empty: word-sep=0, entry-size=4, entry-count=0)
        buf[0x08] = 0x00; buf[0x09] = 0x80;
        buf[0x0080] = 0; buf[0x0081] = 4; buf[0x0082] = 0; buf[0x0083] = 0;
        buf[0x0A] = 0x01; buf[0x0B] = 0x00; // object_table = 0x0100 (unused by this stub)
        buf[0x0C] = 0x03; buf[0x0D] = 0x00; // global_vars = 0x0300
        buf[0x0E] = 0x04; buf[0x0F] = 0x00; // static_mem_base = 0x0400
        buf[0x18] = 0x00; buf[0x19] = 0x60; // abbrev_table = 0x0060
        buf[0x0100] = 0xBA; // quit
        buf
    }

    fn upper_row_text(m: &Machine, row: u16) -> String {
        (1..=m.screen.upper.cols).map(|c| m.screen.upper.cell(row, c).ch).collect()
    }

    /// A screen saved on one terminal, restored on another (SQ-0589, amended by
    /// SQ-0681).
    ///
    /// `restore_screen` installs the saved `ScreenState` wholesale, so without
    /// reconciliation the upper window keeps the SAVED terminal's width while
    /// the header (already re-stamped by `post_restore_fixups`) reports this
    /// one — the mismatch that leaves a short status bar in a wide pane.
    ///
    /// The width GROWS to the host pane and never shrinks below the restored
    /// game's own layout width: the same asymmetry `declared_story_screen_dims`
    /// applies to a live resize (SQ-0679). SQ-0589 originally refit both ways,
    /// which is what re-manifested the garbled bar when an 80-column save met a
    /// 60-column session (SQ-0681) — every coordinate the saved game baked in is
    /// still legal inside a wider screen, and none of them are inside a narrower
    /// one. In a pane too narrow the pane clips the right of the bar instead.
    #[test]
    fn a_restore_refits_the_upper_window_to_the_host_pane_not_the_saved_one() {
        // (host pane, expected grid width, label)
        for (host_cols, want_cols, label) in [
            (120u8, 120u16, "wider host: the grid follows the pane"),
            (60u8, 80u16, "narrower host: the grid holds the restored game's width"),
        ] {
            let mut s = GameSession::new_with_trace(v5_stub_story(), false, false, None, false, Vec::new(), None, None, None)
                .expect("v5 session");
            // The pane we are restoring INTO, as `post_restore_fixups` leaves it.
            s.machine.set_screen_dims(40, host_cols);

            // A screen saved on an 80-column terminal with a one-row status line.
            let mut saved = s.machine.screen.clone();
            saved.upper_window_rows = 1;
            saved.upper.resize(1, 80);
            for (i, ch) in "West of House".chars().enumerate() {
                saved.upper.put(1, i as u16 + 1, ch, 0, ZColour::Default, ZColour::Default);
            }

            restore_screen(&mut s, saved);

            assert_eq!(s.machine.screen.upper.cols, want_cols, "{label}");
            assert_eq!(
                s.machine.mem.read_byte(0x21) as u16,
                want_cols,
                "{label}: and the header agrees with the grid"
            );
            assert_eq!(
                s.machine.screen.upper_window_rows, 1,
                "{label}: the game's split height is the game's — untouched"
            );
            let row = upper_row_text(&s.machine, 1);
            assert_eq!(row.chars().count(), want_cols as usize, "{label}: the row spans it");
            assert!(row.starts_with("West of House"), "{label}: content preserved: {row:?}");
        }
    }

    /// The v6 counterpart of the exemption in `reconcile_restored_screen_size`:
    /// a v6 story lays out on its own native pixel screen, so a restore must
    /// hand back the archived window geometry untouched rather than re-deriving
    /// window 0/1 from a terminal cell count.
    #[test]
    fn a_v6_restore_keeps_the_archived_window_geometry() {
        let mut s = GameSession::new_with_trace(v6_boot_stub_story(), false, false, None, false, Vec::new(), None, None, None)
            .expect("v6 session");
        let mut saved = s.machine.screen.clone();
        let v6 = saved.v6.as_mut().expect("v6 window table");
        v6.windows[0].x_size = 640;
        v6.windows[0].y_size = 400;
        v6.windows[2].x_coord = 41;
        v6.windows[2].y_size = 96;

        restore_screen(&mut s, saved);

        let v6 = s.machine.screen.v6.as_ref().expect("v6 window table");
        assert_eq!((v6.windows[0].x_size, v6.windows[0].y_size), (640, 400), "window 0 as archived");
        assert_eq!((v6.windows[2].x_coord, v6.windows[2].y_size), (41, 96), "a game window as archived");
    }

    /// SQ-0532/A-F2. ZMSD §8.3.3: the interpreter "should ... write its default
    /// background and foreground colours into bytes $2c and $2d of the header."
    /// The host resolves its own page/ink to the nearest §8.3.1 standard colour
    /// and hands them over; the header must show them.
    #[test]
    fn host_default_colours_reach_header_bytes_2c_2d() {
        use ratatui::style::{Color, Style};
        // A dark page with white ink → black background (2), white foreground (9).
        let dark = Style::new().fg(Color::Rgb(238, 238, 238)).bg(Color::Rgb(12, 12, 16));
        let (bg, fg) = crate::colors::host_default_colour_pair(dark, None, None).expect("resolved");
        assert_eq!((bg, fg), (2, 9));
        let s = GameSession::new_with_trace(read_char_story_v5(), true, false, None, false, Vec::new(), None, Some((bg, fg)), None)
            .expect("v5 session");
        assert_eq!(s.machine.mem.read_byte(0x2C), 2, "$2C = default background");
        assert_eq!(s.machine.mem.read_byte(0x2D), 9, "$2D = default foreground");

        // A light page with black ink is the mirror image, and reaches the header
        // through the live path too (a theme reload has no constructor to use).
        let light = Style::new().fg(Color::Rgb(0, 0, 0)).bg(Color::Rgb(250, 250, 250));
        let (bg, fg) = crate::colors::host_default_colour_pair(light, None, None).expect("resolved");
        assert_eq!((bg, fg), (9, 2));
        let mut s = GameSession::new(read_char_story_v5(), true, false, None).expect("v5 session");
        Engine::set_default_colours(&mut s, bg, fg);
        assert_eq!(s.machine.mem.read_byte(0x2C), 9);
        assert_eq!(s.machine.mem.read_byte(0x2D), 2);
    }

    /// Variant of `read_char_story_v5`: after the read_char completes, instead
    /// of `quit` the program executes a `loadw` with an out-of-bounds address
    /// (array=0xFFFF, index=0xFFFF), which faults the VM mid-turn (ZMSD memory
    /// fault). Mirrors zvm's own `loadw_out_of_bounds_faults_with_trace` test.
    fn faulting_read_char_story_v5() -> Vec<u8> {
        let mut buf = read_char_story_v5();
        // loadw (2OP:0x0F), variable-form encoding with two Large operands:
        // array=0xFFFF index=0xFFFF -> addr = 0xFFFF + 2*0xFFFF, far past this
        // 0x0800-byte story's memory.
        buf[0x0044] = 0xCF; // variable form, bit5=0 -> 2OP, opcode=0x0F (loadw)
        buf[0x0045] = 0x0F; // type byte: large, large, omitted, omitted
        buf[0x0046] = 0xFF; buf[0x0047] = 0xFF; // operand a (array) = 0xFFFF
        buf[0x0048] = 0xFF; buf[0x0049] = 0xFF; // operand b (index) = 0xFFFF
        buf[0x004A] = 0x00; // store var 0x00 = push onto stack
        buf
    }

    #[test]
    fn turn_result_carries_fault_trace_when_vm_faults() {
        // End-to-end: submit a turn whose VM step faults mid-execution and
        // confirm the drained TurnResult.fault carries the formatted trace.
        let mut sess = GameSession::new(faulting_read_char_story_v5(), true, false, None)
            .expect("GameSession::new");
        assert_eq!(sess.pending_input(), InputKind::Char);

        let turn_result = sess.submit_char(b'x');
        assert!(turn_result.quit, "a faulted VM halts (routed through RunStop::Quit)");
        let lines = turn_result.fault.expect("TurnResult.fault must be Some after a VM fault");
        assert_eq!(lines[0], "*** VM FAULT ***");
        assert!(lines[1].starts_with("memory fault: read16 @"), "fault line: {}", lines[1]);
    }

    #[test]
    fn pending_input_is_char_after_new_on_read_char_story() {
        let story = read_char_story_v5();
        let session = GameSession::new(story, true, false, None).expect("GameSession::new failed");
        assert_eq!(session.pending_input(), InputKind::Char,
            "GameSession::new on a read_char story should leave pending == Char");
    }

    #[test]
    fn game_session_take_screen_trace_drains_when_enabled() {
        // No handy fixture issues screen opcodes on turn one, so this asserts
        // the drain plumbing directly: set_trace_screen wires to the machine's
        // flag, and take_screen_trace drains screen_trace exactly once.
        let mut s = GameSession::new(read_char_story_v5(), true, false, None)
            .expect("GameSession::new failed");
        s.set_trace_screen(true);
        assert!(s.machine.trace_screen, "set_trace_screen(true) reaches the machine");
        s.machine.screen_trace.push("@set_colour(fg=std5, bg=std2)".to_string());
        let lines = s.take_screen_trace();
        assert!(lines.iter().any(|l| l.starts_with("@")), "{lines:?}");
        assert!(s.take_screen_trace().is_empty(), "second drain is empty");
    }

    #[test]
    fn session_surfaces_timeout_and_aborts_via_run_timed_interrupt() {
        // Interrupt routine: rtrue (0xB0) -> aborts the pending read_char.
        let bytes = timed_read_char_story_v5(&[0xB0]);
        let mut s = GameSession::new(bytes, true, false, None).expect("GameSession::new");
        assert_eq!(s.pending_input(), InputKind::Char);
        assert_eq!(s.pending_timeout(), Some((5, 0x0014)), "time+packed routine surfaced");

        let tr = s.run_timed_interrupt();
        assert!(tr.timed_out, "routine returned true -> the read was aborted");
        // abort_timed_input completes the read_char (stores 0) and the story
        // immediately hits quit.
        assert!(tr.quit, "story quits right after the aborted read_char");
        assert_eq!(s.pending_timeout(), None, "no read pending once the story has quit");
    }

    #[test]
    fn session_run_timed_interrupt_continues_when_routine_returns_false() {
        // Interrupt routine: inc G1 (0x95, 0x11), then rfalse (0xB1) -> the read
        // stays pending; the host is expected to keep waiting.
        let bytes = timed_read_char_story_v5(&[0x95, 0x11, 0xB1]);
        let mut s = GameSession::new(bytes, true, false, None).expect("GameSession::new");
        assert_eq!(s.pending_timeout(), Some((5, 0x0014)));
        let g_before = s.machine.global(1);

        let tr = s.run_timed_interrupt();
        assert!(!tr.timed_out, "routine returned false -> read still pending");
        assert!(!tr.quit, "read_char has not been completed yet");
        assert_eq!(s.pending_input(), InputKind::Char, "read_char is still the pending input");
        assert_eq!(s.pending_timeout(), Some((5, 0x0014)), "timer stays armed for the next tick");
        assert_eq!(s.machine.global(1), g_before.wrapping_add(1), "routine side effect applied");
    }

    #[test]
    fn abort_timed_input_marks_timed_out_and_advances() {
        // Directly abort a timed read_char (bypassing run_timed_interrupt) and
        // confirm the TurnResult is flagged and the VM advances past the read.
        let bytes = timed_read_char_story_v5(&[0xB0]);
        let mut s = GameSession::new(bytes, true, false, None).expect("GameSession::new");
        assert_eq!(s.pending_input(), InputKind::Char);

        let tr = s.abort_timed_input("");
        assert!(tr.timed_out, "abort_timed_input always marks timed_out");
        assert!(tr.quit, "story quits right after the aborted read_char");
    }

    #[test]
    fn run_sound_finish_returns_turn_result() {
        // Reuse the char-mode fixture: run_sound_finish drives run_routine then
        // collects a TurnResult without stepping the read forward. Passing a 0
        // (bad/no routine) still returns a well-formed TurnResult (no panic).
        let story = read_char_story_v5();
        let mut sess = GameSession::new(story, true, false, None).expect("GameSession::new failed");
        let r = sess.run_sound_finish(0);
        assert!(r.sounds.is_empty(), "no new sounds from a finish callback");
        assert!(!r.quit, "a no-op finish routine does not quit");
    }

    #[test]
    fn new_applies_interpreter_override() {
        // read_char_story_v5 is a v5 story; default would be 1, override to 4.
        let story = read_char_story_v5();
        let session = GameSession::new(story, true, false, Some(4)).expect("GameSession::new");
        assert_eq!(session.interpreter_number_for_test(), 4, "override advertised");
    }

    #[test]
    fn new_default_interpreter_is_dec20() {
        let story = read_char_story_v5();
        let session = GameSession::new(story, true, false, None).expect("GameSession::new");
        assert_eq!(session.interpreter_number_for_test(), 1, "v5 default = DEC-20 (1)");
    }

    #[test]
    fn new_session_forwards_sound_available() {
        // GameSession::new must forward sound_available to Machine::set_sound_available
        // (mirrors honor_game_colours), so the game sees the capability from turn 1.
        let session_on = GameSession::new(read_char_story_v5(), true, true, None).expect("GameSession::new");
        assert!(session_on.machine.sound_available, "sound_available(true) must forward to the Machine");

        let session_off = GameSession::new(read_char_story_v5(), true, false, None).expect("GameSession::new");
        assert!(!session_off.machine.sound_available, "sound_available(false) must forward to the Machine");
    }

    #[test]
    fn submit_char_returns_turn_result_and_advances() {
        let story = read_char_story_v5();
        let mut session = GameSession::new(story, true, false, None).expect("GameSession::new failed");
        assert_eq!(session.pending_input(), InputKind::Char);

        // After read_char the next instruction is quit, so submit_char drives
        // the machine to Quit → TurnResult.quit == true.
        let result = session.submit_char(b'x');
        assert!(result.quit, "submit_char on a read_char→quit story should return quit=true");

        // The quit path sets pending back to Line (no input pending).
        assert_eq!(session.pending_input(), InputKind::Line,
            "after quit, pending should be reset to Line");
    }

    // ── Engine adapter (zvm) tests ─────────────────────────────────────────────

    /// Build the v3 variant of the read_char story (so screen() yields a v3
    /// automatic status line).
    fn read_char_story_v3() -> Vec<u8> {
        let mut buf = read_char_story_v5();
        buf[0x00] = 3;
        buf
    }

    #[test]
    fn key_input_to_zscii_matches_legacy_mapping() {
        // Core text keys: unchanged from original mapping.
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Enter), Some(13));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Backspace), Some(8));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Escape), Some(27));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Char('y')), Some(b'y'));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Char('x')), Some(120));
        // Non-ASCII printable chars carry no ZSCII byte (skip the turn).
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Char('\u{00E9}')), None);
        // Arrow keys now map to ZSCII cursor codes (ZMSD §3.8) so read_char works.
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Up),    Some(129));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Func(1)), Some(133));
    }

    #[test]
    fn engine_submit_key_drives_turn_for_mapped_key() {
        let mut sess = GameSession::new(read_char_story_v5(), true, false, None).expect("new");
        assert_eq!(sess.pending_input(), InputKind::Char);
        // 'x' → read_char → quit.
        let r = sess.submit_key(KeyInput::Char('x'));
        assert!(r.is_some(), "a mapped key produces a turn");
        assert!(r.unwrap().quit);
    }

    #[test]
    fn engine_submit_key_is_noop_for_unmapped_key() {
        let mut sess = GameSession::new(read_char_story_v5(), true, false, None).expect("new");
        // Home has no ZSCII meaning: no turn runs, the VM stays waiting.
        assert!(sess.submit_key(KeyInput::Home).is_none());
        assert_eq!(sess.pending_input(), InputKind::Char, "VM untouched by an unmapped key");
    }

    #[test]
    fn zmachine_take_transcript_elems_is_empty() {
        // The Z-machine has no inline images, so it keeps the trait DEFAULT:
        // `take_transcript_elems` returns empty (draining nothing), and callers
        // fall back to the flat `take_transcript` string path. This guarantees
        // the banner/startup dispatch is byte-identical to the pre-feature path.
        let mut sess = GameSession::new(read_char_story_v5(), true, false, None).expect("new");
        assert!(
            sess.take_transcript_elems().is_empty(),
            "zvm uses the default empty elems; the string path stays authoritative",
        );
        // The default elems method drained nothing: the banner string is identical
        // to a fresh session that never called take_transcript_elems.
        let mut fresh = GameSession::new(read_char_story_v5(), true, false, None).expect("new");
        assert_eq!(
            sess.take_transcript(),
            fresh.take_transcript(),
            "take_transcript_elems must not consume the banner for the Z-machine",
        );
    }

    #[test]
    fn take_transcript_respects_strip_prompt_flag() {
        // strip_prompt gates whether the game's trailing "> " read prompt is
        // removed from the transcript (SQ-0264: inline-prompt mode keeps it).
        let mut sess = GameSession::new(read_char_story_v5(), true, false, None).expect("new");
        let _ = sess.take_transcript(); // drain the banner

        sess.strip_prompt = false;
        sink_mut(&mut sess.machine).print("You are in a room.\n>");
        assert_eq!(
            sess.take_transcript(),
            "You are in a room.\n>",
            "strip_prompt=false keeps the game's trailing '>'"
        );

        sess.strip_prompt = true;
        sink_mut(&mut sess.machine).print("You are in a room.\n>");
        assert_eq!(
            sess.take_transcript(),
            "You are in a room.",
            "strip_prompt=true (default) strips the trailing '>'"
        );
    }

    #[test]
    fn engine_screen_v3_is_classic_status() {
        let sess = GameSession::new(read_char_story_v3(), true, false, None).expect("new v3");
        let model = sess.screen();
        match model.status {
            StatusModel::Classic { right, .. } => {
                // Default flags (bit 1 = 0) → score/turns form.
                assert!(matches!(right, StatusField::ScoreTurns { .. }));
            }
            other => panic!("v3 must yield a Classic status, got {other:?}"),
        }
        // The tree still carries a grid (the upper window) over a buffer.
        assert!(model.grid().is_some(), "screen tree exposes a grid node");
    }

    #[test]
    fn engine_screen_v5_is_host_managed_and_mirrors_upper_grid() {
        let mut sess = GameSession::new(read_char_story_v5(), true, false, None).expect("new v5");
        // Paint the upper window directly and confirm screen() mirrors it exactly.
        sess.machine.screen.upper.resize(2, 5);
        sess.machine.screen.upper.put(1, 1, 'H', 2, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default); // bold
        sess.machine.screen.upper.put(1, 2, 'I', 0, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default);
        sess.machine.screen.upper_window_rows = 2;
        sess.machine.screen.cursor_row = 1;
        sess.machine.screen.cursor_col = 3;
        sess.machine.screen.current_window = 1;

        let model = sess.screen();
        assert_eq!(model.status, StatusModel::HostManaged, "v4+ has no automatic status");
        let g = model.grid().expect("grid node");
        assert_eq!((g.cols, g.rows), (5, 2));
        assert_eq!(g.active_rows, 2);
        assert_eq!(g.cell(1, 1).ch, 'H');
        assert_eq!(g.cell(1, 1).style, 2);
        assert_eq!(g.cell(1, 2).ch, 'I');
        assert_eq!(g.cursor, (1, 3));
        assert!(g.cursor_active, "current_window == 1 marks the grid active");
    }

    #[test]
    fn engine_save_state_round_trips_and_is_tagged() {
        let mut sess = GameSession::new(read_char_story_v5(), true, false, None).expect("new");
        let save = sess.save_state();
        assert_eq!(save.engine, ZMACHINE_ENGINE);
        assert!(!save.bytes.is_empty(), "Quetzal save is non-empty");

        // Advance the VM, then restore the captured state.
        let _ = sess.submit_key(KeyInput::Char('x'));
        sess.restore_state(&save).expect("same-engine restore succeeds");

        // A foreign-engine save is refused.
        let foreign = EngineSave::new("glulx", 1, save.bytes.clone());
        match sess.restore_state(&foreign) {
            Err(EngineError::EngineMismatch { expected, found }) => {
                assert_eq!(expected, ZMACHINE_ENGINE);
                assert_eq!(found, "glulx");
            }
            other => panic!("foreign-engine restore must be refused, got {other:?}"),
        }
    }

    #[test]
    fn engine_introspect_wraps_existing_logic() {
        let sess = GameSession::new(read_char_story_v5(), true, false, None).expect("new");
        let intro = sess.introspect().expect("zvm exposes introspection");
        // vocabulary == today's dictionary load.
        let vocab = intro.vocabulary();
        let expected = zvm::dictionary::load(&sess.machine.mem).words(&sess.machine.mem);
        assert_eq!(vocab, expected);
        // player_object == today's find_player_object.
        assert_eq!(intro.player_object(), zvm::find_player_object(&sess.machine));
    }

    #[test]
    fn engine_aux_data_accessors_round_trip() {
        let mut sess = GameSession::new(read_char_story_v5(), true, false, None).expect("new");
        assert!(sess.aux_data().is_empty());
        let mut table = std::collections::BTreeMap::new();
        table.insert("k".to_string(), vec![1u8, 2, 3]);
        sess.set_aux_data(table.clone());
        assert_eq!(sess.aux_data(), &table);
        sess.machine.aux_dirty = true;
        assert!(sess.aux_dirty());
        sess.clear_aux_dirty();
        assert!(!sess.aux_dirty());
    }

    // ── In-game save/restore plumbing (v4) ─────────────────────────────────────
    //
    // read_char_story_v5 lays out: 0x40 read_char->G0 (4 bytes), 0x44 quit.
    // We re-stamp it to v4 and overwrite the quit at 0x44 with the save/restore
    // opcode so the FIRST keypress drives read_char -> the opcode.
    fn read_char_then_save_v4() -> Vec<u8> {
        let mut buf = read_char_story_v5();
        buf[0x00] = 4;    // version 4 (0OP save/restore store form lives here)
        buf[0x44] = 0xB5; // 0OP:0x05 save (store form) -> G0
        buf[0x45] = 0x10; // store byte: global 0
        buf[0x46] = 0xBA; // quit
        buf
    }

    fn read_char_then_restore_v4() -> Vec<u8> {
        let mut buf = read_char_story_v5();
        buf[0x00] = 4;
        buf[0x44] = 0xB6; // 0OP:0x06 restore (store form) -> G0
        buf[0x45] = 0x10; // store byte: global 0
        buf[0x46] = 0xBA; // quit
        buf
    }

    #[test]
    fn ingame_save_yields_pending_io_and_resume_continues() {
        let mut sess = GameSession::new(read_char_then_save_v4(), true, false, None).expect("new");
        assert_eq!(sess.pending_input(), InputKind::Char);

        // The keypress drives read_char -> @save, which suspends with pending_io.
        let r = sess.submit_char(b'x');
        assert_eq!(r.pending_io, Some(PendingIo::Save));
        assert!(!r.quit, "a save-pending turn is not a quit");
        assert!(r.info.is_none(), "v4+ in-game save shows no 'isn't wired' info line");

        // Host wrote the file OK: resume stores 1 into G0 and runs to quit.
        let r2 = sess.resume_save(true);
        assert!(r2.quit, "resume_save continues the VM to the quit opcode");
        assert_eq!(sess.machine.global(0), 1, "complete_save(true) stored 1 into G0");
    }

    #[test]
    fn ingame_restore_yields_pending_io_and_cancel_fails_cleanly() {
        let mut sess = GameSession::new(read_char_then_restore_v4(), true, false, None).expect("new");

        let r = sess.submit_char(b'x');
        assert_eq!(r.pending_io, Some(PendingIo::Restore));
        assert!(!r.quit);

        // Cancel: resume_restore(None) -> complete_restore_failure stores 0, runs on.
        let r2 = sess.resume_restore(None);
        assert!(r2.quit);
        assert_eq!(sess.machine.global(0), 0, "cancelled restore stored 0 into G0");
    }

    #[test]
    fn v3_ingame_save_and_restore_bubble_pending_io() {
        // v3 @save/@restore are BRANCH instructions (0OP:0x05/0x06 = 0xB5/0xB6 +
        // 1 branch byte). After the standard-PC fix they bubble pending_io like v4+.
        let mut save_buf = read_char_story_v5();
        save_buf[0x00] = 3;              // version 3 (branch form)
        save_buf[0x44] = 0xB5;           // 0OP:0x05 save (branch form)
        save_buf[0x45] = 0x80 | 0x40 | 2; // branch on-true, short form, offset 2 -> quit at 0x46
        save_buf[0x46] = 0xBA;           // quit
        let mut sess = GameSession::new(save_buf, true, false, None).expect("new");
        let r = sess.submit_char(b'x');
        assert_eq!(r.pending_io, Some(PendingIo::Save), "v3 in-game save now bubbles pending_io");
        assert!(r.info.is_none(), "no 'isn't wired' info line for v3 anymore");
        let r2 = sess.resume_save(true);
        assert!(r2.quit, "resume_save completes the branch and runs to quit");

        let mut restore_buf = read_char_story_v5();
        restore_buf[0x00] = 3;
        restore_buf[0x44] = 0xB6;           // 0OP:0x06 restore (branch form)
        restore_buf[0x45] = 0x80 | 0x40 | 2; // branch byte (unused on cancel)
        restore_buf[0x46] = 0xBA;           // quit
        let mut sess = GameSession::new(restore_buf, true, false, None).expect("new");
        let r = sess.submit_char(b'x');
        assert_eq!(r.pending_io, Some(PendingIo::Restore), "v3 in-game restore now bubbles pending_io");
        let r2 = sess.resume_restore(None);
        assert!(r2.quit, "cancelled v3 restore falls through to quit");
    }

    #[test]
    fn engine_is_saveload_pending_follows_the_games_own_save_and_restore() {
        // SQ-0661: `Engine::is_saveload_pending` was overridden by GlulxSession
        // only, so the two host guards that consult it — lifecycle::exit_auto_save
        // and lifecycle::quit_dialog_save — were silently no-ops for the
        // Z-machine. An exit auto-save fired while the game's @save is suspended
        // records `save_pc`'s result-descriptor address (Quetzal §5.8) instead of
        // an instruction address, and resuming that archive decodes a store byte
        // as an opcode.
        let mut sess = GameSession::new(read_char_then_save_v4(), true, false, None).expect("new");
        assert!(!sess.is_saveload_pending(), "nothing pending at the opening prompt");

        // The keypress drives read_char -> @save, which suspends awaiting host I/O.
        let r = sess.submit_char(b'x');
        assert_eq!(r.pending_io, Some(PendingIo::Save));
        assert!(
            Engine::is_saveload_pending(&sess),
            "the host must be told the game's @save is suspended"
        );

        // Resolved once the host answers it.
        let _ = sess.resume_save(true);
        assert!(!Engine::is_saveload_pending(&sess), "cleared once the save completes");

        // The @restore side, and via a `dyn Engine` — the shape the guards see.
        let mut sess = GameSession::new(read_char_then_restore_v4(), true, false, None).expect("new");
        let engine: &mut dyn Engine = &mut sess;
        assert!(!engine.is_saveload_pending());
        let r = engine.submit_key(KeyInput::Char('x')).expect("read_char takes the key");
        assert_eq!(r.pending_io, Some(PendingIo::Restore));
        assert!(engine.is_saveload_pending(), "the game's @restore is suspended");
        let _ = engine.resume_restore(None);
        assert!(!engine.is_saveload_pending(), "cleared once the restore is cancelled");
    }

    #[test]
    fn a_host_restore_blanks_the_upper_window_it_brings_no_screen_for() {
        // SQ-0785. Quetzal archives no screen, so the grid left over from the
        // moment being REPLACED is some other room's status line. On v4+ that
        // grid is where `detect_location` reads the room name, and a story
        // repaints only as many columns as its new name needs — so the tail of a
        // longer previous name survives past the end of the new one. Zork I's
        // return probe read `Forest Pathse` that way and resolved it to object 1,
        // the scenery object named `forest`, instead of Forest Path.
        //
        // Falsify by dropping the `blank()` in `restore_state`: the painted name
        // below survives the restore, and the real-game case
        // `zork1_z5_finds_the_way_back_past_a_scenery_object_of_the_same_name`
        // fails with the symptom as reported.
        let mut sess = GameSession::new(read_char_then_save_v4(), true, false, None).expect("new");
        let snapshot = sess.save_state();

        // A previous moment's status line, at a width the saving session baked.
        sess.machine.screen.upper.resize(1, 20);
        for (i, ch) in "North of House".chars().enumerate() {
            sess.machine.screen.upper.cells[i].ch = ch;
        }

        sess.restore_state(&snapshot).expect("host Save State restore");
        let upper = &sess.machine.screen.upper;
        // `blank()` keeps the grid's extent; the settling drive that re-arms the
        // read may then refit the WIDTH to the host screen, which is a separate
        // and long-standing behaviour (`refit_upper_window_width`). What this
        // case pins is that no character survives either way.
        assert_eq!(upper.rows, 1, "the split the saving session made is still there");
        assert!(
            upper.cells.iter().all(|c| c.ch == ' '),
            "no character of the replaced moment survives: {:?}",
            upper.cells.iter().map(|c| c.ch).collect::<String>()
        );
    }

    #[test]
    fn a_host_restore_clears_a_suspended_ingame_save() {
        // SQ-0661 (the session-level face of the zvm fix): the player suspends the
        // game's own @save, then loads a host Save State instead. The abandoned
        // @save must not keep the guard latched on — with it stuck true, every
        // later exit auto-save would silently skip for the rest of the session.
        let mut sess = GameSession::new(read_char_then_save_v4(), true, false, None).expect("new");
        let snapshot = sess.save_state(); // taken at the clean opening prompt
        let r = sess.submit_char(b'x');
        assert_eq!(r.pending_io, Some(PendingIo::Save));
        assert!(sess.is_saveload_pending());

        sess.restore_state(&snapshot).expect("host Save State restore");
        assert!(
            !Engine::is_saveload_pending(&sess),
            "the restore replaced the run the @save belonged to"
        );

        // Perturb: the restore re-armed the read_char, so play on — and the host
        // snapshot taken after that must record the live run, not the dead
        // descriptor. Driving the same keypress reaches @save again, proving the
        // restored run (not a wedged one) is what is executing.
        let r = sess.submit_char(b'x');
        assert_eq!(r.pending_io, Some(PendingIo::Save), "the restored run reached its own @save");
    }

    #[test]
    fn turn_result_carries_location_method_field() {
        // Build the same way the sibling submit test does; the field just needs to exist
        // and default to a value. For a v3 fixture with global 0 set, method is GlobalVar0.
        let story = read_char_story_v5();
        let mut sess = GameSession::new(story, true, false, None).expect("GameSession::new failed");
        // The story starts with a read_char; submit_char drives it to quit.
        let r = sess.submit_char(b'x');
        // The field exists and is an Option<LocationMethod>; on a v5 story with no
        // location it is None — either is acceptable here.
        let _ = r.location_method;
    }

    #[test]
    fn turn_result_has_empty_sound_fields_by_default() {
        let story = read_char_story_v5();
        let mut sess = GameSession::new(story, true, false, None).expect("GameSession::new failed");
        // The story starts with a read_char; submit_char drives it to quit.
        let r = sess.submit_char(b'x');
        assert!(r.sounds.is_empty(), "no sounds when the game emits no sound");
        assert!(r.diagnostics.is_empty(), "no diagnostics on a clean turn");
        // VM queues are drained after the turn.
        assert!(sess.machine.pending_sounds.is_empty());
        assert!(sess.machine.diagnostics.is_empty());
    }

    // ── Plan 1b Task 2: pending_pictures → per-window canvases ────────────────

    /// A minimal v6 story buffer: version 6, header 0x06/0x07 (main's packed
    /// address) left at 0 so `Machine::with_output`'s v6 boot path
    /// (`call_routine` on the unpacked address) reads byte 0 (the version byte,
    /// 6) as a harmless in-range "locals count" — this test never steps the VM,
    /// so the routine is never actually executed. Mirrors the header layout of
    /// `inventory.rs`'s `sample_story_v3` shim (zvm's own `tests_support` is
    /// crate-private).
    fn minimal_v6_story() -> Vec<u8> {
        let mut buf = vec![0u8; 0x1000];
        buf[0x00] = 6;                       // version = 6
        buf[0x04] = 0x04; buf[0x05] = 0x00; // high_mem_base = 0x0400
        buf[0x06] = 0x00; buf[0x07] = 0x00; // main's packed addr = 0
        buf[0x08] = 0x02; buf[0x09] = 0x00; // dictionary = 0x0200
        buf[0x0A] = 0x01; buf[0x0B] = 0x00; // object_table = 0x0100
        buf[0x0C] = 0x03; buf[0x0D] = 0x00; // global_vars = 0x0300
        buf[0x0E] = 0x04; buf[0x0F] = 0x00; // static_mem_base = 0x0400
        buf[0x18] = 0x00; buf[0x19] = 0x60; // abbrev_table = 0x0060
        buf
    }

    /// A minimal v5 story whose initial PC is 0x0500, so a test can drop a few
    /// opcodes there and step them.
    fn minimal_v5_story(code: &[u8]) -> Vec<u8> {
        let mut buf = vec![0u8; 0x1000];
        buf[0x00] = 5;                       // version = 5
        buf[0x04] = 0x04; buf[0x05] = 0x00; // high_mem_base = 0x0400
        buf[0x06] = 0x05; buf[0x07] = 0x00; // initial PC = 0x0500
        buf[0x08] = 0x02; buf[0x09] = 0x00; // dictionary = 0x0200
        buf[0x0A] = 0x01; buf[0x0B] = 0x00; // object_table = 0x0100
        buf[0x0C] = 0x03; buf[0x0D] = 0x00; // global_vars = 0x0300
        buf[0x0E] = 0x04; buf[0x0F] = 0x00; // static_mem_base = 0x0400
        buf[0x18] = 0x00; buf[0x19] = 0x60; // abbrev_table = 0x0060
        buf[0x0500..0x0500 + code.len()].copy_from_slice(code);
        buf
    }

    #[test]
    fn run_settled_answers_a_game_save_instead_of_parking_on_it() {
        // SQ-0656: `restore_state` and `restore_game_save` used to drop a
        // `SavePending`/`RestorePending` stop on the floor — the VM stayed suspended
        // with no dialog ever opening for it, and `restore_state` additionally left
        // `quit` false on a `Quit` stop. Both now settle through this helper, the
        // Z-machine twin of Glulx's `drive_settled`, so the two engines behave the
        // same at a host restore.
        //
        // `@save -> G0` then `@quit`. A bare `run_until_input` stops AT the save;
        // settling answers it (v4+ stores 0 = failed) and runs on to the quit.
        let code = [
            0xBE, 0x00, 0xFF, 0x10, // EXT:0 save (no operands) -> global 0
            0xBA,                   // quit
        ];
        let mem = Memory::new(minimal_v5_story(&code)).expect("minimal v5 story");
        let mut machine = Machine::with_output(mem, Box::new(CaptureSink::new()));
        // A witness the save's failure result must overwrite (0 is its own default).
        machine.mem.write_word(0x0300, 0xFFFF);

        let (pending, quit) = run_settled(&mut machine);

        assert_eq!(
            machine.mem.read_word(0x0300), 0,
            "the suspended @save is ANSWERED (v4+ stores 0 for failure), not left pending",
        );
        assert!(quit, "and the drive runs on to the game's quit, which must set the quit flag");
        assert_eq!(pending, InputKind::Line, "a quit reports the neutral Line input mode");
    }

    /// The cached object-word set is one build per TURN — and not one per
    /// session, which is the soundness line: a game CAN rewrite an object's
    /// parse-name property in dynamic memory, and the set must see it on the
    /// next turn (SQ-1176).
    ///
    /// Driven on the committed `minizork.z3` fixture, whose lantern (object
    /// 102) files its words under property 17 — pinned by
    /// `zvm/tests/parse_names.rs` against the game's own parser.
    #[test]
    fn the_object_word_set_is_cached_for_a_turn_and_dropped_when_the_vm_runs() {
        use crate::engine::Introspect;
        use std::sync::Arc;

        let story = zvm::fixtures::load("minizork.z3").expect("committed fixture");
        let mut sess = GameSession::new(story, true, false, None).expect("minizork boots");

        let first = Introspect::object_word_set(&sess).expect("minizork has parse names");
        assert!(
            first.contains("lantern") && first.contains("mailbox") && !first.contains("verbose"),
            "the set answers as any(refers_to) answers on the real story"
        );
        let again = Introspect::object_word_set(&sess).expect("still answerable");
        assert!(Arc::ptr_eq(&first, &again), "within a turn, one build serves every caller");

        // The game rewrites its world under the cache: point the lantern's
        // first parse word (the slot holding `lamp`) at the dictionary entry
        // for a word no object answers to today. Not an article — the set
        // deliberately never holds one (`grammar_model::ARTICLES`, SQ-1210),
        // so `a` could never come back however fresh the rebuild.
        let chosen = zvm::grammar::dictionary_words(&sess.machine.mem)
            .into_iter()
            .find(|w| {
                w.text.chars().all(char::is_alphabetic)
                    && !first.contains(&w.text)
                    && !grammar_model::ARTICLES.contains(&w.text.as_str())
            })
            .expect("minizork has verbs no object answers to");
        let prop = zvm::objects::get_prop_addr(&sess.machine.mem, 102, 17);
        assert_ne!(prop, 0, "the lantern keeps its words in property 17");
        sess.machine.mem.write_word(u32::from(prop), chosen.address as u16);

        // Within the same turn the cache is deliberately stale — the screen the
        // player is reading has not changed either.
        let stale = Introspect::object_word_set(&sess).expect("still answerable");
        assert!(
            Arc::ptr_eq(&first, &stale) && !stale.contains(&chosen.text),
            "within a turn the cached build stands"
        );

        // A turn runs; the next build must read the rewritten memory.
        sess.submit("look");
        let fresh = Introspect::object_word_set(&sess).expect("still answerable");
        assert!(
            fresh.contains(&chosen.text),
            "after a turn the set sees the rewritten parse word {:?}",
            chosen.text
        );
        assert!(!Arc::ptr_eq(&first, &fresh), "and it is a fresh build, not the stale one");
    }

    /// A valid 2x2 red PNG, encoded via the `image` crate (mirrors
    /// `graphics.rs`'s private test helper of the same shape).
    fn png_bytes_2x2_red() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([255, 0, 0]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn drain_turn_applies_pending_draw_picture_to_the_window_canvas() {
        use zvm::screen::{V6Windows, ZWindow};

        // A v6 machine with window 7 sized 64x48px, current window = 7, and one
        // pending draw_picture(number=1, window=7, x=2, y=3) event — as if
        // `exec_ext(0x05, ...)` had just run (Task 1/Plan 1a).
        let mem = Memory::new(minimal_v6_story()).expect("minimal v6 story");
        let mut machine = Machine::with_output(mem, Box::new(CaptureSink::new()));
        let mut windows: [ZWindow; 8] = Default::default();
        windows[7] = ZWindow { x_size: 64, y_size: 48, ..Default::default() };
        machine.screen.v6 = Some(V6Windows { windows, current: 7 });
        machine.pending_pictures.push(PictureEvent { number: 1, window: 7, x: 2, y: 3, erase: false, out_chars: 0, margin_after: None, at_cursor: false, win_box: (1, 1, 64, 48) });

        // Construct the session directly (bypassing the constructor's boot
        // loop, which this synthetic story can't usefully run) with a Pict
        // source that resolves resource #1 to the red 2x2 PNG.
        let blorb = crate::graphics::test_blorb_with_pict(1, &png_bytes_2x2_red());
        let mut sess = GameSession {
            machine, quit: false, pending: InputKind::Line, strip_prompt: true, pen_before_char: None, output_continued: false,
            disasm_cache: std::cell::RefCell::new(None),
            world: std::cell::OnceCell::new(),
            parse_names: std::cell::OnceCell::new(),
            object_word_set: std::cell::RefCell::new(None),
            v6_model_memo: std::cell::RefCell::new(None),
            last_confirmed_pc: std::cell::Cell::new(None),
            pict_source: None,
            pictures_canvas: std::collections::HashMap::new(),
            canvas_anchor: std::collections::HashMap::new(),
            art_scale: (V6_ART_SCALE, V6_ART_SCALE),
            paint: None,
            paced_frames: std::collections::VecDeque::new(),
            window_fills: std::collections::HashMap::new(),
            story_pics: Vec::new(),
            v6_win0_chars_seen: 0,
            display_ops: std::collections::HashMap::new(),
            unreplayable: std::collections::HashSet::new(),
            boot_screen_cols: zvm::screen::DEFAULT_SCREEN_COLS as u16,
        };
        sess.set_pict_source(Some(crate::graphics::PictSource::new(Some(blorb))));

        assert!(sess.pictures_canvas.is_empty(), "no canvas before the turn is drained");
        let result = sess.drain_turn(false, None, false);

        assert_eq!(result.pictures, vec![PictureEvent { number: 1, window: 7, x: 2, y: 3, erase: false, out_chars: 0, margin_after: None, at_cursor: false, win_box: (1, 1, 64, 48) }],
            "the drained event is carried on TurnResult (mirrors pending_sounds)");
        assert!(sess.machine.pending_pictures.is_empty(), "the VM queue is drained after the turn");

        let canvas = sess.pictures_canvas.get(&7).expect("a canvas was created for window 7");
        assert_eq!(canvas.img.dimensions(), (64, 48), "canvas sized from the v6 window's pixel dims");
        assert_ne!(canvas.img.get_pixel(2, 3).0, [0, 0, 0, 0], "the picture was drawn (non-blank at its origin)");
        assert_eq!(canvas.img.get_pixel(2, 3).0, [0xFF, 0x00, 0x00, 0xFF], "the drawn pixel is the source PNG's red");
        // Outside the drawn 2x2 picture the canvas stays at its transparent default.
        assert_eq!(canvas.img.get_pixel(0, 0).0, [0, 0, 0, 0], "untouched region stays transparent");
    }

    /// A valid `w`×`h` red PNG.
    fn png_bytes_red(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([255, 0, 0]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    /// Retargeted from `content_splash_anchors_once_and_dedupes_until_canvas_clear`
    /// (SQ-0895), and INVERTED. It used to pin SQ-0461's emission: a large
    /// content-art draw into a graphics window anchored one inline transcript
    /// band, deduped against a per-turn redraw, with a canvas-clear resetting the
    /// dedupe. Only the frameless mode ever drew that band — hybrid and raster
    /// render the window canvas itself and had to skip it — so with the mode gone
    /// the emission went too.
    ///
    /// Kept rather than deleted because the fixture is exactly the one that used
    /// to fire, which makes it a live guard against the band coming back: if
    /// anyone re-adds an emitter, this fails instead of a double-drawn splash
    /// showing up on Shogun's title screen.
    #[test]
    fn a_graphics_window_content_splash_anchors_no_inline_band() {
        use zvm::screen::{V6Windows, ZWindow};
        // Window 7 sized to the full 320×200 screen; picture 1 is a 320×200 splash
        // → CONTENT art by `is_content_art`, i.e. precisely the case SQ-0461
        // anchored. The canvas must carry it and the transcript must not.
        let mem = Memory::new(minimal_v6_story()).expect("minimal v6 story");
        let mut machine = Machine::with_output(mem, Box::new(CaptureSink::new()));
        let mut windows: [ZWindow; 8] = Default::default();
        windows[7] = ZWindow { x_size: 320, y_size: 200, ..Default::default() };
        machine.screen.v6 = Some(V6Windows { windows, current: 7 });

        let blorb = crate::graphics::test_blorb_with_pict(1, &png_bytes_red(320, 200));
        let mut sess = GameSession {
            machine, quit: false, pending: InputKind::Line, strip_prompt: true, pen_before_char: None, output_continued: false,
            disasm_cache: std::cell::RefCell::new(None),
            world: std::cell::OnceCell::new(),
            parse_names: std::cell::OnceCell::new(),
            object_word_set: std::cell::RefCell::new(None),
            v6_model_memo: std::cell::RefCell::new(None),
            last_confirmed_pc: std::cell::Cell::new(None),
            pict_source: None,
            pictures_canvas: std::collections::HashMap::new(),
            canvas_anchor: std::collections::HashMap::new(),
            art_scale: (V6_ART_SCALE, V6_ART_SCALE),
            paint: None,
            paced_frames: std::collections::VecDeque::new(),
            window_fills: std::collections::HashMap::new(),
            story_pics: Vec::new(),
            v6_win0_chars_seen: 0,
            display_ops: std::collections::HashMap::new(),
            unreplayable: std::collections::HashSet::new(),
            boot_screen_cols: zvm::screen::DEFAULT_SCREEN_COLS as u16,
        };
        sess.set_pict_source(Some(crate::graphics::PictSource::new(Some(blorb))));

        let draw = PictureEvent { number: 1, window: 7, x: 1, y: 1, erase: false, out_chars: 0, margin_after: None, at_cursor: false, win_box: (1, 1, 320, 200) };
        sess.apply_picture_event(&draw);
        assert!(sess.story_pics.is_empty(), "graphics-window content art anchors no transcript band");
        // It really did reach the screen — the band's absence is a routing
        // decision, not the picture going missing.
        let canvas = sess.pictures_canvas.get(&7).expect("the splash is on window 7's canvas");
        assert_ne!(canvas.img.get_pixel(0, 0).0, [0, 0, 0, 0], "the splash was actually drawn");

        // A repeat draw (the per-turn redraw the old dedupe existed for) still
        // anchors nothing, so no dedupe key is needed to keep it from spamming.
        sess.apply_picture_event(&draw);
        assert!(sess.story_pics.is_empty(), "a repeat draw anchors nothing either");

        // …and neither does a fresh draw after a canvas clear (erase_window rides
        // the queue as number 0), which used to be the case that reset the dedupe.
        sess.apply_picture_event(&PictureEvent { number: 0, window: 7, x: 1, y: 1, erase: true, out_chars: 0, margin_after: None, at_cursor: false, win_box: (1, 1, 320, 200) });
        sess.apply_picture_event(&draw);
        assert!(sess.story_pics.is_empty(), "a post-clear draw anchors nothing");
    }

    /// SQ-0741: a window-0 picture the game followed with `set_margins` is an
    /// inline float even when its `y` misses the cursor by a pixel or two.
    ///
    /// Zork Zero booted off its Amiga floppy does exactly that: it adds native
    /// placement picture 478 (`2×1`, where the Blorb's `Rect` is `0×0`) to the
    /// cursor, so the drop-cap lands at `y = cursor + 2` and `at_cursor` — a
    /// pixel-exact comparison — says no. The `set_margins` that follows is the
    /// game reserving the prose column, which is the same claim `at_cursor` makes
    /// and a stronger one.
    ///
    /// Fixture-free, so it holds wherever `stories/` is absent. Falsified by
    /// restoring `!ev.at_cursor`: the float count drops to 0 and window 0 takes a
    /// canvas.
    #[test]
    fn a_margin_declared_after_a_win0_draw_makes_it_an_inline_float() {
        use zvm::screen::{V6Windows, ZWindow};
        let mem = Memory::new(minimal_v6_story()).expect("minimal v6 story");
        let mut machine = Machine::with_output(mem, Box::new(CaptureSink::new()));
        let mut windows: [ZWindow; 8] = Default::default();
        // Window 0 as Zork Zero frames it: a 464×320 prose column inside the
        // graphical border, wide enough that a 4×4 unit-space tile cannot span it.
        windows[0] = ZWindow { x_size: 464, y_size: 320, ..Default::default() };
        machine.screen.v6 = Some(V6Windows { windows, current: 0 });

        let blorb = crate::graphics::test_blorb_with_pict(1, &png_bytes_2x2_red());
        let mut sess = GameSession {
            machine, quit: false, pending: InputKind::Line, strip_prompt: true, pen_before_char: None, output_continued: false,
            disasm_cache: std::cell::RefCell::new(None),
            world: std::cell::OnceCell::new(),
            parse_names: std::cell::OnceCell::new(),
            object_word_set: std::cell::RefCell::new(None),
            v6_model_memo: std::cell::RefCell::new(None),
            last_confirmed_pc: std::cell::Cell::new(None),
            pict_source: None,
            pictures_canvas: std::collections::HashMap::new(),
            canvas_anchor: std::collections::HashMap::new(),
            art_scale: (V6_ART_SCALE, V6_ART_SCALE),
            paint: None,
            paced_frames: std::collections::VecDeque::new(),
            window_fills: std::collections::HashMap::new(),
            story_pics: Vec::new(),
            v6_win0_chars_seen: 0,
            display_ops: std::collections::HashMap::new(),
            unreplayable: std::collections::HashSet::new(),
            boot_screen_cols: zvm::screen::DEFAULT_SCREEN_COLS as u16,
        };
        sess.set_pict_source(Some(crate::graphics::PictSource::new(Some(blorb))));

        // Off the cursor by the native placement inset, but with the margin the
        // prose is to flow in declared right after — an inline float.
        sess.apply_picture_event(&PictureEvent {
            number: 1, window: 0, x: 5, y: 19, erase: false, out_chars: 0,
            margin_after: Some(96), at_cursor: false, win_box: (89, 81, 464, 320),
        });
        assert_eq!(sess.story_pics.len(), 1, "the declared margin marks it as flowing with the text");
        assert_eq!(sess.story_pics[0].1.align, crate::inline_image::ImageAlign::MarginLeft);
        assert_eq!(sess.story_pics[0].1.margin_px, Some(96), "the game's own left margin rides along");
        assert!(!sess.pictures_canvas.contains_key(&0), "a float never takes a window canvas");

        // Neither signal: art the game placed for itself, which keeps the canvas
        // (Arthur's centred intro plates — SQ-0695).
        sess.apply_picture_event(&PictureEvent {
            number: 1, window: 0, x: 29, y: 5, erase: false, out_chars: 0,
            margin_after: None, at_cursor: false, win_box: (1, 1, 464, 320),
        });
        assert_eq!(sess.story_pics.len(), 1, "placed art anchors no new float");
        assert!(sess.pictures_canvas.contains_key(&0), "placed art gets the window canvas");
    }

    #[test]
    fn restart_drops_the_pre_restart_v6_display_list() {
        use zvm::screen::{V6Windows, ZWindow};
        // SQ-0658: `@restart` dropped `pictures_canvas` (the rasterized RESULT) but
        // kept `display_ops` (the RECIPE that rebuilds it under a new palette). The
        // reboot's first draw into a window re-creates the canvas and APPENDS to the
        // ops the dead session left behind, so the next palette change replays the
        // old game's art onto the new one's screen. `unreplayable` and the erase
        // `window_fills` were stranded the same way.
        let mem = Memory::new(minimal_v6_story()).expect("minimal v6 story");
        let mut machine = Machine::with_output(mem, Box::new(CaptureSink::new()));
        let mut windows: [ZWindow; 8] = Default::default();
        windows[7] = ZWindow { x_size: 64, y_size: 48, ..Default::default() };
        machine.screen.v6 = Some(V6Windows { windows, current: 7 });

        // A 2×2 picture; every draw covers 4×4 unit pixels (V6_ART_SCALE).
        let blorb = crate::graphics::test_blorb_with_pict(1, &png_bytes_2x2_red());
        let mut sess = GameSession {
            machine, quit: false, pending: InputKind::Line, strip_prompt: true, pen_before_char: None, output_continued: false,
            disasm_cache: std::cell::RefCell::new(None),
            world: std::cell::OnceCell::new(),
            parse_names: std::cell::OnceCell::new(),
            object_word_set: std::cell::RefCell::new(None),
            v6_model_memo: std::cell::RefCell::new(None),
            last_confirmed_pc: std::cell::Cell::new(None),
            pict_source: None,
            pictures_canvas: std::collections::HashMap::new(),
            canvas_anchor: std::collections::HashMap::new(),
            art_scale: (V6_ART_SCALE, V6_ART_SCALE),
            paint: None,
            paced_frames: std::collections::VecDeque::new(),
            window_fills: std::collections::HashMap::new(),
            story_pics: Vec::new(),
            v6_win0_chars_seen: 0,
            display_ops: std::collections::HashMap::new(),
            unreplayable: std::collections::HashSet::new(),
            boot_screen_cols: zvm::screen::DEFAULT_SCREEN_COLS as u16,
        };
        sess.set_pict_source(Some(crate::graphics::PictSource::new(Some(blorb))));

        // The pre-restart session draws at the window's top-left corner…
        sess.apply_picture_event(&PictureEvent {
            number: 1, window: 7, x: 1, y: 1, erase: false, out_chars: 0, margin_after: None, at_cursor: false,
            win_box: (1, 1, 64, 48),
        });
        assert_eq!(sess.display_ops.get(&7).map_or(0, Vec::len), 1, "the draw is recorded for replay");
        // …and something took window 7 out of replay (an op-cap overflow in a long
        // session; forced here, since the count itself is not the point).
        sess.unreplayable.insert(7);

        // @restart: the VM re-boots in place and the session drains the turn.
        sess.machine.just_restarted = true;
        let _ = sess.drain_turn(false, None, false);
        assert!(sess.pictures_canvas.is_empty(), "the rasterized canvas is dropped (pre-existing)");
        assert!(sess.display_ops.is_empty(), "and so is the display list that rebuilds it");
        assert!(sess.unreplayable.is_empty(), "a pre-restart replay veto must not outlive the reboot");
        assert!(sess.window_fills.is_empty(), "nor the erase fills of a screen that no longer exists");

        // The rebooted game draws the SAME picture somewhere else, then a base
        // picture establishes a new palette and every window replays.
        sess.apply_picture_event(&PictureEvent {
            number: 1, window: 7, x: 33, y: 1, erase: false, out_chars: 0, margin_after: None, at_cursor: false,
            win_box: (1, 1, 64, 48),
        });
        sess.replay_under_current_palette();

        let canvas = sess.pictures_canvas.get(&7).expect("the reboot's draw made a canvas");
        assert_eq!(
            canvas.img.get_pixel(32, 0).0, [0xFF, 0x00, 0x00, 0xFF],
            "the rebooted game's own draw replays",
        );
        assert_eq!(
            canvas.img.get_pixel(0, 0).0, [0, 0, 0, 0],
            "the PRE-restart draw must not replay into the rebooted canvas",
        );
    }

    /// SQ-0814. `@restart` reboots the story in place, and the drain already drops the
    /// app-side v6 chrome the VM's own screen reset cannot reach — the canvases, the
    /// display list, the replay vetoes and the erase fills. Two layers were left out
    /// of that list and belong in it by exactly the same argument:
    ///
    /// * the CANVAS ANCHORS (SQ-0715), which say where a canvas that has just been
    ///   dropped was painted. Kept, they union the reboot's first draw into a
    ///   pre-restart footprint and strand it at a pre-restart origin;
    /// * the painted GROUND (SQ-0706), which is the dead screen's own pixels. Nothing
    ///   else drops it: `apply_erase_fill` only clears it on a full-screen erase
    ///   naming a colour outright, so a reboot whose boot erases inherit — Zork Zero,
    ///   Arthur, Journey, advent, the mysterious set: every v6 story measured except
    ///   scopa, Shogun and fmvpoker — keeps the old game's ground under the new one.
    ///
    /// A synthetic v6 screen rather than a real story, because the real ones make the
    /// bug unfalsifiable rather than absent: the three that paint a ground also clear
    /// the full screen on the way back up, and the ones that reboot without clearing
    /// have no ground to lose. The two mechanisms are independent, and this pins the
    /// one that is ours.
    #[test]
    fn restart_drops_the_pre_restart_ground_and_canvas_anchors() {
        use zvm::screen::{V6Windows, ZColour, ZWindow};
        let mem = Memory::new(minimal_v6_story()).expect("minimal v6 story");
        let mut machine = Machine::with_output(mem, Box::new(CaptureSink::new()));
        let mut windows: [ZWindow; 8] = Default::default();
        windows[0] = ZWindow { x_size: 640, y_size: 400, ..Default::default() };
        windows[7] = ZWindow { x_size: 64, y_size: 48, ..Default::default() };
        machine.screen.v6 = Some(V6Windows { windows, current: 7 });

        let blorb = crate::graphics::test_blorb_with_pict(1, &png_bytes_2x2_red());
        let mut sess = GameSession {
            machine, quit: false, pending: InputKind::Line, strip_prompt: true, pen_before_char: None, output_continued: false,
            disasm_cache: std::cell::RefCell::new(None),
            world: std::cell::OnceCell::new(),
            parse_names: std::cell::OnceCell::new(),
            object_word_set: std::cell::RefCell::new(None),
            v6_model_memo: std::cell::RefCell::new(None),
            last_confirmed_pc: std::cell::Cell::new(None),
            pict_source: None,
            pictures_canvas: std::collections::HashMap::new(),
            canvas_anchor: std::collections::HashMap::new(),
            art_scale: (V6_ART_SCALE, V6_ART_SCALE),
            paint: None,
            paced_frames: std::collections::VecDeque::new(),
            window_fills: std::collections::HashMap::new(),
            story_pics: Vec::new(),
            v6_win0_chars_seen: 0,
            display_ops: std::collections::HashMap::new(),
            unreplayable: std::collections::HashSet::new(),
            boot_screen_cols: zvm::screen::DEFAULT_SCREEN_COLS as u16,
        };
        sess.set_pict_source(Some(crate::graphics::PictSource::new(Some(blorb))));

        // The pre-restart game paints a ground (a PART-screen erase naming a colour —
        // a full-screen one would be a screen clear and drop the ground by itself)…
        sess.apply_erase_fill(&zvm::cpu::exec::EraseFill {
            window: 7, x: 1, y: 1, w: 64, h: 48, bg: ZColour::True24(0x00FF00), pics_before: 0,
        });
        // …and draws into window 7, anchoring its canvas where the window sits now.
        sess.apply_picture_event(&PictureEvent {
            number: 1, window: 7, x: 1, y: 1, erase: false, out_chars: 0, margin_after: None, at_cursor: false,
            win_box: (1, 1, 64, 48),
        });
        assert!(sess.paint.is_some(), "premise: the pre-restart screen has a painted ground");
        assert!(sess.canvas_anchor.contains_key(&7), "premise: and window 7's canvas is anchored");

        // @restart: the VM re-boots in place and the session drains the turn.
        sess.machine.just_restarted = true;
        let _ = sess.drain_turn(false, None, false);

        assert!(
            sess.paint.is_none(),
            "the painted ground belongs to the screen the reboot replaced — a rebooted game \
             inherits none of it (SQ-0814)"
        );
        assert!(
            sess.canvas_anchor.is_empty(),
            "nor the anchors of canvases the same drain has just dropped: an anchor left \
             standing strands the reboot's own first draw at a pre-restart origin (SQ-0814)"
        );
    }

    #[test]
    fn frame_art_draw_never_anchors_an_inline_band() {
        use zvm::screen::{V6Windows, ZWindow};
        // A 23×200 side border (Shogun idiom) draws into the canvas but must NOT
        // anchor an inline band — it's decorative frame art.
        let mem = Memory::new(minimal_v6_story()).expect("minimal v6 story");
        let mut machine = Machine::with_output(mem, Box::new(CaptureSink::new()));
        let mut windows: [ZWindow; 8] = Default::default();
        windows[7] = ZWindow { x_size: 320, y_size: 200, ..Default::default() };
        machine.screen.v6 = Some(V6Windows { windows, current: 7 });

        let blorb = crate::graphics::test_blorb_with_pict(3, &png_bytes_red(23, 200));
        let mut sess = GameSession {
            machine, quit: false, pending: InputKind::Line, strip_prompt: true, pen_before_char: None, output_continued: false,
            disasm_cache: std::cell::RefCell::new(None),
            world: std::cell::OnceCell::new(),
            parse_names: std::cell::OnceCell::new(),
            object_word_set: std::cell::RefCell::new(None),
            v6_model_memo: std::cell::RefCell::new(None),
            last_confirmed_pc: std::cell::Cell::new(None),
            pict_source: None,
            pictures_canvas: std::collections::HashMap::new(),
            canvas_anchor: std::collections::HashMap::new(),
            art_scale: (V6_ART_SCALE, V6_ART_SCALE),
            paint: None,
            paced_frames: std::collections::VecDeque::new(),
            window_fills: std::collections::HashMap::new(),
            story_pics: Vec::new(),
            v6_win0_chars_seen: 0,
            display_ops: std::collections::HashMap::new(),
            unreplayable: std::collections::HashSet::new(),
            boot_screen_cols: zvm::screen::DEFAULT_SCREEN_COLS as u16,
        };
        sess.set_pict_source(Some(crate::graphics::PictSource::new(Some(blorb))));

        sess.apply_picture_event(&PictureEvent { number: 3, window: 7, x: 1, y: 1, erase: false, out_chars: 0, margin_after: None, at_cursor: false, win_box: (1, 1, 320, 200) });
        assert!(sess.story_pics.is_empty(), "frame art stays canvas-only");
        assert!(sess.pictures_canvas.contains_key(&7), "but it IS drawn into the window canvas");
    }

    #[test]
    fn drain_turn_applies_pending_erase_picture_to_the_window_canvas() {
        use zvm::screen::{V6Windows, ZWindow};

        let mem = Memory::new(minimal_v6_story()).expect("minimal v6 story");
        let mut machine = Machine::with_output(mem, Box::new(CaptureSink::new()));
        let mut windows: [ZWindow; 8] = Default::default();
        windows[7] = ZWindow { x_size: 64, y_size: 48, ..Default::default() };
        machine.screen.v6 = Some(V6Windows { windows, current: 7 });
        // Draw, then erase the same picture — the erase must clear back to
        // transparent over the picture's own footprint (2x2, ZMSD §15).
        machine.pending_pictures.push(PictureEvent { number: 1, window: 7, x: 2, y: 3, erase: false, out_chars: 0, margin_after: None, at_cursor: false, win_box: (1, 1, 64, 48) });
        machine.pending_pictures.push(PictureEvent { number: 1, window: 7, x: 2, y: 3, erase: true, out_chars: 0, margin_after: None, at_cursor: false, win_box: (1, 1, 64, 48) });

        let blorb = crate::graphics::test_blorb_with_pict(1, &png_bytes_2x2_red());
        let mut sess = GameSession {
            machine, quit: false, pending: InputKind::Line, strip_prompt: true, pen_before_char: None, output_continued: false,
            disasm_cache: std::cell::RefCell::new(None),
            world: std::cell::OnceCell::new(),
            parse_names: std::cell::OnceCell::new(),
            object_word_set: std::cell::RefCell::new(None),
            v6_model_memo: std::cell::RefCell::new(None),
            last_confirmed_pc: std::cell::Cell::new(None),
            pict_source: None,
            pictures_canvas: std::collections::HashMap::new(),
            canvas_anchor: std::collections::HashMap::new(),
            art_scale: (V6_ART_SCALE, V6_ART_SCALE),
            paint: None,
            paced_frames: std::collections::VecDeque::new(),
            window_fills: std::collections::HashMap::new(),
            story_pics: Vec::new(),
            v6_win0_chars_seen: 0,
            display_ops: std::collections::HashMap::new(),
            unreplayable: std::collections::HashSet::new(),
            boot_screen_cols: zvm::screen::DEFAULT_SCREEN_COLS as u16,
        };
        sess.set_pict_source(Some(crate::graphics::PictSource::new(Some(blorb))));

        let result = sess.drain_turn(false, None, false);
        assert_eq!(result.pictures.len(), 2);
        let canvas = sess.pictures_canvas.get(&7).expect("a canvas was created for window 7");
        assert_eq!(canvas.img.get_pixel(2, 3).0, [0, 0, 0, 0], "erased back to transparent");
    }

    // ── Plan 1b Task 4: v6 layered screen-model adapter ───────────────────────

    #[test]
    fn v6_screen_returns_layered_model_graphics_first_then_text_by_window_number() {
        use crate::engine::GraphicsWindow;
        use zvm::screen::{V6Windows, ZWindow};

        let mem = Memory::new(minimal_v6_story()).expect("minimal v6 story");
        let mut machine = Machine::with_output(mem, Box::new(CaptureSink::new()));

        let mut windows: [ZWindow; 8] = Default::default();
        // Window coords are the spec's 1-based pixels ((1,1) = top-left). The v6
        // cell is 8×16 (SQ-0479): X quantizes /8, Y /16 — so a window at cell
        // ROW 1 starts at pixel y=17 (one 16px status row below the top).
        // Window 0: the main scrolling window, at (0, 1) cell, 80x20 cells.
        // attributes 15 = the boot default (wrapping on → transcript Buffer;
        // a cleared wrapping bit would mean positioned paint mode → Grid).
        windows[0] = ZWindow { x_coord: 1, y_coord: 17, x_size: 640, y_size: 320, attributes: 15, ..Default::default() };
        windows[0].grid.resize(20, 80);
        // Window 1: a one-row (16px) status strip along the top, at (0, 0) cell, 80x1 cells.
        windows[1] = ZWindow { x_coord: 1, y_coord: 1, x_size: 640, y_size: 16, ..Default::default() };
        windows[1].grid.resize(1, 80);
        // Window 7: a small picture window at (2, 1) cell, 8x6 cells.
        windows[7] = ZWindow { x_coord: 17, y_coord: 17, x_size: 64, y_size: 96, ..Default::default() };
        windows[7].grid.resize(6, 8);
        machine.screen.v6 = Some(V6Windows { windows, current: 1 });

        let mut sess = GameSession {
            machine, quit: false, pending: InputKind::Line, strip_prompt: true, pen_before_char: None, output_continued: false,
            disasm_cache: std::cell::RefCell::new(None),
            world: std::cell::OnceCell::new(),
            parse_names: std::cell::OnceCell::new(),
            object_word_set: std::cell::RefCell::new(None),
            v6_model_memo: std::cell::RefCell::new(None),
            last_confirmed_pc: std::cell::Cell::new(None),
            pict_source: None,
            pictures_canvas: std::collections::HashMap::new(),
            canvas_anchor: std::collections::HashMap::new(),
            art_scale: (V6_ART_SCALE, V6_ART_SCALE),
            paint: None,
            paced_frames: std::collections::VecDeque::new(),
            window_fills: std::collections::HashMap::new(),
            story_pics: Vec::new(),
            v6_win0_chars_seen: 0,
            display_ops: std::collections::HashMap::new(),
            unreplayable: std::collections::HashSet::new(),
            boot_screen_cols: zvm::screen::DEFAULT_SCREEN_COLS as u16,
        };
        // Window 7 has a rendered picture (a canvas sized to its pixel dims).
        sess.pictures_canvas.insert(7, crate::graphics::Canvas::new(64, 48));

        let model = sess.screen();
        assert_ne!(model.content_size, (0, 0), "v6 always reports a nonzero content size");
        assert_eq!(model.content_size, (80, 21), "max right/bottom cell extent across the windows");

        let items = match model.root {
            WinNode::Layered(items) => items,
            other => panic!("expected WinNode::Layered, got {other:?}"),
        };

        // z-order: graphics entries first (window 7's picture), then text
        // windows by ascending window number (0, then 1, then 7's own grid).
        assert_eq!(items.len(), 4, "graphics(7) + buffer(0) + grid(1) + grid(7)");

        let g7 = &items[0];
        assert_eq!((g7.x, g7.y, g7.w, g7.h), (2, 1, 8, 6), "window 7's absolute cell rect (x px/8, y px/16)");
        match &g7.node {
            WinNode::Graphics(GraphicsWindow { win, .. }) => assert_eq!(*win, 7),
            other => panic!("expected window 7's Graphics leaf first (background), got {other:?}"),
        }

        let w0 = &items[1];
        assert_eq!((w0.x, w0.y, w0.w, w0.h), (0, 1, 80, 20), "window 0's absolute cell rect (x px/8, y px/16)");
        match &w0.node {
            WinNode::Buffer(b) => assert!(b.primary, "window 0 is the primary scrolling buffer"),
            other => panic!("expected window 0's Buffer leaf, got {other:?}"),
        }

        let w1 = &items[2];
        assert_eq!((w1.x, w1.y, w1.w, w1.h), (0, 0, 80, 1), "window 1's absolute cell rect (x px/8, y px/16)");
        match &w1.node {
            WinNode::Grid(g) => assert_eq!((g.cols, g.rows), (80, 1)),
            other => panic!("expected window 1's Grid leaf, got {other:?}"),
        }

        let w7 = &items[3];
        assert_eq!((w7.x, w7.y, w7.w, w7.h), (2, 1, 8, 6), "window 7's own (blank) text grid, same rect as its Graphics leaf");
        match &w7.node {
            WinNode::Grid(g) => assert_eq!((g.cols, g.rows), (8, 6)),
            other => panic!("expected window 7's Grid leaf, got {other:?}"),
        }
    }

    // ── `v6` debug-trace snapshot ──────────────────────────────────────────────

    #[test]
    fn v6_snapshot_is_none_for_non_v6_stories() {
        let sess = GameSession::new(read_char_story_v5(), true, false, None)
            .expect("GameSession::new failed");
        assert!(sess.v6_snapshot().is_none(), "no v6 window table -> no snapshot");
    }

    #[test]
    fn v6_snapshot_reports_nontrivial_windows_runs_and_canvases_and_skips_blank_windows() {
        use zvm::screen::{V6Windows, ZWindow};

        let mem = Memory::new(minimal_v6_story()).expect("minimal v6 story");
        let mut machine = Machine::with_output(mem, Box::new(CaptureSink::new()));
        let mut windows: [ZWindow; 8] = Default::default();
        // Window 1: a real status window with one paint run.
        windows[1] = ZWindow {
            x_coord: 1, y_coord: 1, x_size: 640, y_size: 8,
            y_cursor: 1, x_cursor: 9,
            left_margin: 2, right_margin: 3,
            font_number: 1, font_size: 0x0808,
            attributes: 3, // bit0 wrap + bit1 scroll
            ..Default::default()
        };
        windows[1].texts.push(zvm::screen::V6Text::derived(1, 1, "Score: 10".to_string(), 0, ZColour::Default, ZColour::Default, zvm::screen::V6Cell::DEFAULT));
        // Window 3 stays entirely default (blank) — must be skipped.
        machine.screen.v6 = Some(V6Windows { windows, current: 1 });

        let mut sess = GameSession {
            machine, quit: false, pending: InputKind::Line, strip_prompt: true, pen_before_char: None, output_continued: false,
            disasm_cache: std::cell::RefCell::new(None),
            world: std::cell::OnceCell::new(),
            parse_names: std::cell::OnceCell::new(),
            object_word_set: std::cell::RefCell::new(None),
            v6_model_memo: std::cell::RefCell::new(None),
            last_confirmed_pc: std::cell::Cell::new(None),
            pict_source: None,
            pictures_canvas: std::collections::HashMap::new(),
            canvas_anchor: std::collections::HashMap::new(),
            art_scale: (V6_ART_SCALE, V6_ART_SCALE),
            paint: None,
            paced_frames: std::collections::VecDeque::new(),
            window_fills: std::collections::HashMap::new(),
            story_pics: Vec::new(),
            v6_win0_chars_seen: 0,
            display_ops: std::collections::HashMap::new(),
            unreplayable: std::collections::HashSet::new(),
            boot_screen_cols: zvm::screen::DEFAULT_SCREEN_COLS as u16,
        };
        let mut canvas = crate::graphics::Canvas::new(64, 48);
        canvas.z_seq = 42;
        sess.pictures_canvas.insert(1, canvas);

        let lines = sess.v6_snapshot().expect("v6 story yields Some snapshot");
        assert_eq!(lines[0], "turn snapshot (current=1)");

        let win_line = lines.iter().find(|l| l.starts_with("win1:"))
            .unwrap_or_else(|| panic!("win1 line present: {lines:?}"));
        assert!(win_line.contains("pos=(1,1)"), "{win_line}");
        assert!(win_line.contains("size=640x8"), "{win_line}");
        assert!(win_line.contains("cursor=(1,9)"), "{win_line}");
        assert!(win_line.contains("margins=(2,3)"), "{win_line}");
        assert!(win_line.contains("font=(1,2056)"), "{win_line}"); // 0x0808 = 2056
        assert!(win_line.contains("runs=1"), "{win_line}");

        assert!(lines.iter().any(|l| l.contains("y=1 x=1") && l.contains("\"Score: 10\"")), "{lines:?}");
        assert!(lines.iter().any(|l| l == "canvas win1: 64x48 z=42"), "{lines:?}");

        assert!(!lines.iter().any(|l| l.starts_with("win3:")), "blank window 3 must be skipped: {lines:?}");
    }

    #[test]
    fn v6_picture_canvas_clamps_hostile_window_size() {
        use zvm::cpu::exec::PictureEvent;
        use zvm::screen::{V6Windows, ZWindow};
        // A window sized to the pixel max must not force a ~17 GB canvas alloc.
        let mem = Memory::new(minimal_v6_story()).unwrap();
        let mut machine = Machine::with_output(mem, Box::new(CaptureSink::new()));
        let mut windows: [ZWindow; 8] = Default::default();
        windows[7] = ZWindow { x_size: 0xFFFF, y_size: 0xFFFF, ..Default::default() };
        machine.screen.v6 = Some(V6Windows { windows, current: 7 });
        let mut sess = GameSession {
            machine, quit: false, pending: InputKind::Line, strip_prompt: true, pen_before_char: None, output_continued: false,
            disasm_cache: std::cell::RefCell::new(None),
            world: std::cell::OnceCell::new(),
            parse_names: std::cell::OnceCell::new(),
            object_word_set: std::cell::RefCell::new(None),
            v6_model_memo: std::cell::RefCell::new(None),
            last_confirmed_pc: std::cell::Cell::new(None),
            pict_source: None,
            pictures_canvas: std::collections::HashMap::new(),
            canvas_anchor: std::collections::HashMap::new(),
            art_scale: (V6_ART_SCALE, V6_ART_SCALE),
            paint: None,
            paced_frames: std::collections::VecDeque::new(),
            window_fills: std::collections::HashMap::new(),
            story_pics: Vec::new(),
            v6_win0_chars_seen: 0,
            display_ops: std::collections::HashMap::new(),
            unreplayable: std::collections::HashSet::new(),
            boot_screen_cols: zvm::screen::DEFAULT_SCREEN_COLS as u16,
        };
        // The erase path allocates the canvas even without a resolved image.
        // (number != 0: a real erase_picture — number 0 is the erase_window
        // canvas-clear sentinel, which removes the canvas instead.)
        sess.apply_picture_event(&PictureEvent { number: 5, window: 7, x: 0, y: 0, erase: true, out_chars: 0, margin_after: None, at_cursor: false, win_box: (1, 1, 0xFFFF, 0xFFFF) });
        let c = sess.pictures_canvas.get(&7).expect("erase allocated a canvas");
        assert!(c.img.width() <= 4096 && c.img.height() <= 4096,
            "canvas clamped, got {}x{}", c.img.width(), c.img.height());
    }

    // Fixture-gated: in-game SAVE then RESTORE on Bureaucracy (v4) must leave the
    // upper-window status grid non-empty (the redraw this whole feature is about).
    // NOTE/GAP: this drives the SESSION resume API, not the app event loop, and it
    // depends on reaching @save by typing into the game. If the input sequence does
    // not reach @save within the probe budget, the test skips (no false failure).
    #[test]
    fn bureaucracy_ingame_restore_redraws_status_grid() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/bureaucr.z4");
        if !fixture.exists() {
            return; // fixture absent — skip
        }
        let story = std::fs::read(&fixture).expect("read bureaucr.z4");
        let mut sess = GameSession::new(story, true, false, None).expect("new bureaucr.z4");

        // Probe: type SAVE-ish commands until the VM suspends on @save.
        let mut blob: Option<Vec<u8>> = None;
        for cmd in ["save", "yes", "save", "y", "save"] {
            let r = sess.submit(cmd);
            if r.pending_io == Some(PendingIo::Save) {
                blob = Some(sess.machine.save_quetzal());
                let _ = sess.resume_save(true); // pretend the host wrote the file
                break;
            }
            if r.quit { break; }
        }
        let Some(blob) = blob else {
            // Could not reach @save with this probe sequence — document the gap.
            eprintln!("bureaucr.z4: did not reach @save via the probe; skipping redraw assertion");
            return;
        };

        // Now drive a RESTORE and feed the captured blob back.
        let mut restored = false;
        for cmd in ["restore", "yes", "restore", "y", "restore"] {
            let r = sess.submit(cmd);
            if r.pending_io == Some(PendingIo::Restore) {
                let _ = sess.resume_restore(Some(&blob));
                restored = true;
                break;
            }
            if r.quit { break; }
        }
        if !restored {
            eprintln!("bureaucr.z4: did not reach @restore via the probe; skipping redraw assertion");
            return;
        }

        // The resumed game redrew its own status line into the upper window.
        let any_drawn = sess.machine.screen.upper.cells.iter().any(|c| c.ch != ' ');
        assert!(any_drawn, "after in-game RESTORE the upper-window grid must be non-empty (redraw)");
    }

    // Real v3 game: an in-game @save then @restore must round-trip through the
    // standard branch-form path. Oracle: replaying the same command after a
    // restore reproduces the pre-restore transcript exactly.
    #[test]
    fn minizork_v3_ingame_save_restore_round_trips() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/minizork.z3");
        if !fixture.exists() {
            panic!("minizork.z3 fixture missing at {} — this smoke test must run", fixture.display());
        }
        let story = std::fs::read(&fixture).expect("read minizork.z3");
        let mut sess = GameSession::new(story, true, false, None).expect("new minizork.z3");

        // Reach a stable prompt, then @save via the game's save verb.
        let mut blob: Option<Vec<u8>> = None;
        for cmd in ["open mailbox", "save"] {
            let r = sess.submit(cmd);
            if r.pending_io == Some(PendingIo::Save) {
                blob = Some(sess.machine.save_quetzal());
                let _ = sess.resume_save(true); // host "wrote" the file; @save returns success
                break;
            }
            assert!(!r.quit, "unexpected quit before reaching @save");
        }
        let blob = blob.expect("minizork reached @save via 'save'");

        // Probe command on the post-save branch.
        let t1 = sess.submit("north").transcript;

        // Restore via the game's @restore, supplying the captured blob.
        let r = sess.submit("restore");
        assert_eq!(r.pending_io, Some(PendingIo::Restore), "'restore' reaches @restore");
        sess.resume_restore(Some(&blob));

        // Same probe after restore must reproduce the same transcript.
        let t2 = sess.submit("north").transcript;
        assert_eq!(t2, t1, "post-restore continuation matches the pre-restore continuation");
    }

    // SQ-0233 probe: saves-manager load of a game `.qzl` (host-initiated) goes
    // through restore_game_save (complete_restore_success), NOT resume_restore.
    // Verify the next typed command runs (not dropped / not the pre-save one).
    #[test]
    fn game_save_restore_via_manager_accepts_next_command() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/minizork.z3");
        if !fixture.exists() { panic!("minizork.z3 missing"); }
        let story = std::fs::read(&fixture).expect("read minizork.z3");

        // Producer: reach @save, capture the descriptor-PC game-save blob.
        let mut prod = GameSession::new(story.clone(), true, false, None).expect("new");
        let mut blob = None;
        for cmd in ["open mailbox", "save"] {
            let r = prod.submit(cmd);
            if r.pending_io == Some(PendingIo::Save) {
                blob = Some(prod.machine.save_quetzal());
                let _ = prod.resume_save(true);
                break;
            }
        }
        let blob = blob.expect("reached @save");

        // Consumer: fresh session, restore via the host game-save path.
        let mut sess = GameSession::new(story, true, false, None).expect("new");
        sess.restore_game_save(&blob).expect("restore game save");
        let t = sess.submit("north").transcript;
        assert!(t.contains("North of House"),
            "after saves-manager game-save restore, typed 'north' must run (got {t:?})");
    }

    // Real v3 game, real `.qzl` FILE: extends the test above by exercising the
    // on-disk game-save format end to end. `persist_files::save_game_named` writes
    // the descriptor-PC blob to a real `.qzl` file; a FRESH session's machine is
    // then restored from that file via `persist_files::restore_game` (Task 1's
    // descriptor-completion path) — not `resume_restore` — so the actual
    // file-format restore function is what's under test.
    // Oracle (SQ-0158): `play(prefix).probe()` == `restore(qzl file).probe()`.
    #[test]
    fn minizork_v3_qzl_file_round_trips_end_to_end() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/minizork.z3");
        if !fixture.exists() {
            panic!("minizork.z3 fixture missing at {} — this smoke test must run", fixture.display());
        }
        let story = std::fs::read(&fixture).expect("read minizork.z3");
        let mut sess = GameSession::new(story.clone(), true, false, None).expect("new minizork.z3");

        let dir = std::env::temp_dir().join(format!("lanthorn-task5-qzl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");

        // Reach a stable prompt, then @save via the game's save verb. Capture the
        // descriptor-PC blob AND write it to a real `.qzl` file at the same paused
        // moment, before resume_save continues execution and mutates the machine.
        let mut blob: Option<Vec<u8>> = None;
        let mut qzl_path: Option<std::path::PathBuf> = None;
        for cmd in ["open mailbox", "save"] {
            let r = sess.submit(cmd);
            if r.pending_io == Some(PendingIo::Save) {
                blob = Some(sess.machine.save_quetzal());
                qzl_path = Some(
                    crate::persist_files::save_game_named(&dir, "task5", &sess.machine)
                        .expect("save_game_named writes the .qzl file"),
                );
                let _ = sess.resume_save(true); // host "wrote" the file; @save returns success
                break;
            }
            assert!(!r.quit, "unexpected quit before reaching @save");
        }
        let blob = blob.expect("minizork reached @save via 'save'");
        let qzl_path = qzl_path.expect("save_game_named ran");
        assert!(qzl_path.to_string_lossy().ends_with(".qzl"), "game save is a .qzl file");

        let bytes_from_disk = std::fs::read(&qzl_path).expect("read the .qzl file back");
        assert_eq!(bytes_from_disk, blob, ".qzl file bytes match the captured save_quetzal() blob");

        // Reference leg: play(prefix).probe() — continue the SAME session past the save.
        let t1 = sess.submit("north").transcript;
        assert!(t1.contains("North of House"), "probe must reveal real room state, got: {t1:?}");

        // Restore leg: a FRESH session's machine, restored straight from the real
        // `.qzl` file via persist_files::restore_game.
        let mut sess2 = GameSession::new(story, true, false, None).expect("new minizork.z3 (fresh)");
        crate::persist_files::restore_game(&qzl_path, &mut sess2.machine)
            .expect("restore_game completes the .qzl descriptor");
        // Run forward to the next input request (mirrors resume_restore's own
        // run_until_input) and sync the session's pending/quit bookkeeping.
        let stop = run_until_input(&mut sess2.machine);
        let _ = sess2.finish_turn(stop); // drains stray intro/restore text, not asserted

        // restore(qzl file).probe() — same probe command on the restored session.
        let t2 = sess2.submit("north").transcript;
        assert_eq!(t2, t1, "restore(qzl file).probe() must equal play(prefix).probe()");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── czech.z5 smoke test ───────────────────────────────────────────────────
    //
    // czech.z5 is an auto-running opcode test suite: it runs to `Quit` without
    // ever requesting input, so `session.quit` will be `true` after `new`.
    // We verify that the session was built successfully and produced output.

    #[test]
    fn czech_smoke_initial_transcript_nonempty() {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        if !fixture_path.exists() {
            return; // fixture absent — skip
        }
        let story = std::fs::read(&fixture_path).expect("read czech.z5");
        let mut session = GameSession::new(story, true, false, None).expect("GameSession::new with czech.z5");
        // czech is an automated test suite that runs to completion (quit=true is normal).
        let transcript = session.take_transcript();
        assert!(!transcript.is_empty(), "czech should produce output before quitting");
    }

    // ── Task 4: first_banner_line + resolve_title tests ──────────────────────

    #[test]
    fn title_from_banner_anchors_on_boilerplate() {
        // Title is the line above the copyright/boilerplate anchor.
        assert_eq!(title_from_banner("\n\nZORK I: The Great Underground Empire\nCopyright...\n> ").as_deref(),
                   Some("ZORK I: The Great Underground Empire"));
        // "interactive fiction" / "interactive fantasy" also anchor.
        assert_eq!(title_from_banner("SPELLBREAKER\nAn Interactive Fantasy\nCopyright (c) 1985").as_deref(),
                   Some("SPELLBREAKER"));
        // Banner opens WITH boilerplate (no title above) → None (caller → filename).
        assert_eq!(title_from_banner("Copyright (C) 1987 Infocom, Inc.\nType RESTORE...").as_deref(), None);
        // No anchor (epigraph / narration) → None.
        assert_eq!(title_from_banner("\"Tomorrow never yet\nOn any human being rose or set.").as_deref(), None);
        assert_eq!(title_from_banner("\n\n").as_deref(), None);
    }

    /// The table's own coverage and shape are pinned where it lives now
    /// (`cli_host::titles`); this is the re-export still answering for `session`.
    #[test]
    fn known_title_looks_up_table() {
        assert_eq!(known_title("ZCODE-116-870602-FC65"), Some("Bureaucracy"));
        assert_eq!(known_title("ZCODE-0-000000-0000"), None);
    }

    /// SQ-0766 moved the IFID known-title lookup OUT of `resolve_title` and into
    /// the shared browser resolver (`picker::metadata_title`), which is what the
    /// `metadata` argument now carries — the pane and the story list must consult
    /// one source or they name the same game differently. The tier below it is
    /// unchanged: banner, then the filename stem.
    #[test]
    fn resolve_title_override_then_metadata_then_banner_then_filename() {
        use std::path::Path;
        // override wins over everything.
        assert_eq!(resolve_title(Some("My Game"), Some("Bureaucracy"), Some("X"), Path::new("/x/zork1.z3")), "My Game");
        // metadata wins over the banner heuristic (e.g. Bureaucracy, whose banner is just copyright).
        assert_eq!(resolve_title(None, Some("Bureaucracy"), None, Path::new("/x/bureaucr.z4")), "Bureaucracy");
        // no metadata → banner heuristic.
        assert_eq!(resolve_title(None, None, Some("ZORK I"), Path::new("/x/zork1.z3")), "ZORK I");
        // no metadata + no banner title → filename.
        assert_eq!(resolve_title(None, None, None, Path::new("/x/zork1.z3")), "zork1");
    }

    /// SQ-0766, part C/D: the banner heuristic is genuinely blind to a game that
    /// boots into a title plate or a version notice, so metadata has to outrank
    /// it. Both banners are the real, dumped ones.
    #[test]
    fn resolve_title_prefers_metadata_over_an_unparseable_banner() {
        use std::path::Path;
        // anchor.z8's boot banner: a letter-spaced title plate and a keypress prompt.
        let anchor = "\n\n\n                             A N C H O R H E A D\n\n\n               [Press 'R' to restore; any other key to begin]\n";
        assert_eq!(title_from_banner(anchor), None, "no boilerplate to anchor on");
        assert_eq!(
            resolve_title(None, Some("Anchorhead"), title_from_banner(anchor).as_deref(), Path::new("/x/anchor.z8")),
            "Anchorhead"
        );
        // Without any metadata source it still lands on the stem — the reported bug.
        assert_eq!(resolve_title(None, None, None, Path::new("/x/anchor.z8")), "anchor");
        // photo201.blb has no IFmd chunk; its fetched sidecar is what knows the name.
        assert_eq!(
            resolve_title(None, Some("Photopia"), None, Path::new("/x/photo201.blb")),
            "Photopia"
        );
    }

    #[test]
    fn format_pane_title_same_name_omits_parenthetical() {
        // Normalized name equals normalized stem (extension excluded) → bare name.
        assert_eq!(format_pane_title("Bureaucracy", "bureaucracy.z4", false), "Bureaucracy");
        // Case/punctuation-insensitive: still "the same" after normalizing.
        assert_eq!(format_pane_title("Zork I", "zork-i.z3", false), "Zork I");
    }

    #[test]
    fn format_pane_title_differing_name_appends_filename() {
        // Release/serial-suffixed filename reads as different from the title.
        assert_eq!(
            format_pane_title("Journey: The Quest Begins", "journey-r83-s890706.z6", false),
            "Journey: The Quest Begins (journey-r83-s890706.z6)"
        );
        // A single-character difference (I vs 1) still counts as differing.
        assert_eq!(format_pane_title("Zork I", "zork1.z3", false), "Zork I (zork1.z3)");
        // A blorb whose stem is nothing like its title needs no special case.
        assert_eq!(format_pane_title("Photopia", "photo201.blb", false), "Photopia (photo201.blb)");
    }

    /// SQ-0766 part A: a disk image is a different RELEASE, so the pane always
    /// names it — even when the box-spelled filename normalizes onto the title.
    #[test]
    fn format_pane_title_always_names_a_disk_image() {
        for (name, file) in [
            ("Arthur: The Quest for Excalibur", "Arthur - The Quest for Excalibur.adf"),
            ("Journey: The Quest Begins", "Journey - The Quest Begins.adf"),
        ] {
            // The premise: without the disk-image rule these normalize to the same thing.
            assert_eq!(
                crate::hints::normalize_ident(name),
                crate::hints::normalize_ident(file.trim_end_matches(".adf")),
                "{file}: premise — name and stem normalize alike"
            );
            assert_eq!(format_pane_title(name, file, true), format!("{name} ({file})"));
        }
        // A bare story file with the same name keeps the bare title.
        assert_eq!(format_pane_title("Journey", "journey.z6", false), "Journey");
    }

    #[test]
    fn format_pane_title_unknown_name_falls_back_to_filename() {
        // No title resolved at all: today's behavior, no empty parenthetical.
        assert_eq!(format_pane_title("", "mystery.z5", false), "mystery.z5");
        // …including on a disk image, where an empty parenthetical would be worse.
        assert_eq!(format_pane_title("", "Shogun.adf", true), "Shogun.adf");
    }

    // ── strip_read_prompt unit tests ──────────────────────────────────────────

    #[test]
    fn strip_prompt_removes_trailing_gt_on_own_line() {
        // Typical Infocom pattern: text followed by newline and bare ">".
        assert_eq!(
            strip_read_prompt("You are in a room.\n\n>"),
            "You are in a room."
        );
    }

    #[test]
    fn strip_prompt_removes_trailing_gt_with_trailing_space() {
        // Some games emit "> " (with a space after).
        assert_eq!(
            strip_read_prompt("You are in a room.\n> "),
            "You are in a room."
        );
    }

    #[test]
    fn strip_prompt_does_not_remove_mid_text_gt() {
        // A ">" that is NOT the last non-whitespace token on its own line must
        // be preserved — e.g. a score comparison or a quoted string.
        let s = "Your score is > 10.\nYou are here.";
        assert_eq!(strip_read_prompt(s), s);
    }

    #[test]
    fn strip_prompt_does_not_remove_gt_mid_line() {
        // ">" at the end of the last line but inline (no preceding newline).
        let s = "Go east, then go >";
        assert_eq!(strip_read_prompt(s), s);
    }

    #[test]
    fn strip_prompt_empty_input_unchanged() {
        assert_eq!(strip_read_prompt(""), "");
    }

    #[test]
    fn strip_prompt_sole_gt_removed() {
        // Edge case: the entire captured block is just ">".
        assert_eq!(strip_read_prompt(">"), "");
    }

    #[test]
    fn strip_prompt_gt_with_only_whitespace_before() {
        // "\n>" with no preceding text.
        assert_eq!(strip_read_prompt("\n>"), "");
    }

    #[test]
    fn strip_prompt_no_trailing_prompt_unchanged() {
        let s = "You are in a maze of twisty passages, all alike.";
        assert_eq!(strip_read_prompt(s), s);
    }

    // ── key_input_to_zscii: arrow and function keys (Bug B) ──────────────────

    #[test]
    fn key_input_to_zscii_arrows_map_to_zscii_codes() {
        use crate::engine::KeyInput;
        // Arrow keys → ZSCII cursor codes (ZMSD §3.8), matching zvm-cli decode_escape_seq.
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Up),    Some(129));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Down),  Some(130));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Left),  Some(131));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Right), Some(132));
        // Function keys F1-F4 → ZSCII 133-136.
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Func(1)), Some(133));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Func(4)), Some(136));
    }

    #[test]
    fn key_input_to_zscii_existing_keys_unchanged() {
        use crate::engine::KeyInput;
        // Pre-existing mappings must not change.
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Enter),     Some(13));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Backspace), Some(8));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Escape),    Some(27));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Char('A')), Some(65));
        // Non-ascii char → None (existing behaviour).
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Char('\u{00E9}')), None);
        // Tab → None (not a game key).
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Tab), None);
    }

    // ── SQ-0188: line terminator keys ─────────────────────────────────────────

    /// A v5 story whose header 0x2E points at a terminating-characters table
    /// listing ZSCII 129 (cursor-up), so `is_terminator(129)` is true. Mirrors the
    /// zvm fixture in `terminating_chars_table_is_honoured`.
    fn story_v5_with_up_terminator() -> Vec<u8> {
        let mut buf = read_char_story_v5();
        let tbl: u16 = 0x0090; // dynamic memory, below static base 0x0400
        buf[0x2E] = (tbl >> 8) as u8;
        buf[0x2F] = (tbl & 0xFF) as u8;
        buf[tbl as usize] = 0x81;     // 129 = cursor up
        buf[tbl as usize + 1] = 0x00; // table terminator
        buf
    }

    #[test]
    fn line_key_terminator_maps_listed_key() {
        use crate::engine::KeyInput;
        let s = GameSession::new(story_v5_with_up_terminator(), true, false, None)
            .expect("GameSession::new");
        // Up (129) is listed in the game's table → submit with that terminator.
        assert_eq!(s.line_key_terminator(&KeyInput::Up), Some(129));
        // Down (130) is a candidate but NOT listed → None (keeps app behavior).
        assert_eq!(s.line_key_terminator(&KeyInput::Down), None);
    }

    #[test]
    fn line_key_terminator_none_without_table() {
        use crate::engine::KeyInput;
        // No terminating-characters table → arrows/F-keys are never terminators.
        let s = GameSession::new(read_char_story_v5(), true, false, None)
            .expect("GameSession::new");
        assert_eq!(s.line_key_terminator(&KeyInput::Up), None);
        assert_eq!(s.line_key_terminator(&KeyInput::Func(1)), None);
    }

    #[test]
    fn line_key_terminator_rejects_non_candidate_keys() {
        use crate::engine::KeyInput;
        // Even with a table present, only arrows + F-keys are candidates; Enter
        // flows through the normal submit path and other keys never terminate.
        let s = GameSession::new(story_v5_with_up_terminator(), true, false, None)
            .expect("GameSession::new");
        assert_eq!(s.line_key_terminator(&KeyInput::Char('x')), None);
        assert_eq!(s.line_key_terminator(&KeyInput::Enter), None);
        assert_eq!(s.line_key_terminator(&KeyInput::Backspace), None);
    }

    // ── SQ-1191: the memoized v6 screen model ─────────────────────────────────

    /// A synthetic v6 session with `text` painted on window 7, built the way
    /// `drain_turn_applies_pending_draw_picture_to_the_window_canvas` builds
    /// its fixture (bypassing the boot loop the minimal story can't run).
    fn v6_session_with_run(text: &str) -> GameSession {
        use zvm::screen::{V6Cell, V6Metric, V6Text, V6Windows, ZColour, ZWindow};
        let mem = Memory::new(minimal_v6_story()).expect("minimal v6 story");
        let mut machine = Machine::with_output(mem, Box::new(CaptureSink::new()));
        let mut windows: [ZWindow; 8] = Default::default();
        windows[7] = ZWindow { x_size: 64, y_size: 48, ..Default::default() };
        machine.screen.v6 = Some(V6Windows { windows, current: 7 });
        let metric = V6Metric::fixed(V6Cell::DEFAULT);
        machine.screen.v6_mut().unwrap().paint_run(
            7,
            V6Text::derived(1, 1, text.to_string(), 0, ZColour::Default, ZColour::Default, V6Cell::DEFAULT),
            &metric,
        );
        GameSession {
            machine, quit: false, pending: InputKind::Line, strip_prompt: true, pen_before_char: None, output_continued: false,
            disasm_cache: std::cell::RefCell::new(None),
            world: std::cell::OnceCell::new(),
            parse_names: std::cell::OnceCell::new(),
            object_word_set: std::cell::RefCell::new(None),
            v6_model_memo: std::cell::RefCell::new(None),
            last_confirmed_pc: std::cell::Cell::new(None),
            pict_source: None,
            pictures_canvas: std::collections::HashMap::new(),
            canvas_anchor: std::collections::HashMap::new(),
            art_scale: (V6_ART_SCALE, V6_ART_SCALE),
            paint: None,
            paced_frames: std::collections::VecDeque::new(),
            window_fills: std::collections::HashMap::new(),
            story_pics: Vec::new(),
            v6_win0_chars_seen: 0,
            display_ops: std::collections::HashMap::new(),
            unreplayable: std::collections::HashSet::new(),
            boot_screen_cols: zvm::screen::DEFAULT_SCREEN_COLS as u16,
        }
    }

    /// Every painted-run text reachable in the model, flattened for a contains test.
    fn model_run_texts(model: &ScreenModel) -> Vec<String> {
        let mut out = Vec::new();
        if let WinNode::Layered(entries) = &model.root {
            for pw in entries {
                if let WinNode::Grid(g) = &pw.node {
                    out.extend(g.px_texts.iter().map(|t| t.text.clone()));
                }
            }
        }
        out
    }

    /// SQ-1191: `screen_now` hands the SAME `Arc` back while nothing on the v6
    /// screen changes, and a paint through zvm's one door replaces it with a
    /// fresh model that carries the new run.
    #[test]
    fn screen_now_memoizes_the_v6_model_until_the_screen_changes() {
        use zvm::screen::{V6Cell, V6Metric, V6Text, ZColour};
        let mut sess = v6_session_with_run("steady");

        let first = Engine::screen_now(&sess);
        let again = Engine::screen_now(&sess);
        assert!(
            std::sync::Arc::ptr_eq(&first, &again),
            "no VM step between two frames: the memoized model must be handed back, not rebuilt"
        );
        assert!(model_run_texts(&first).contains(&"steady".to_string()), "the fixture's run is in the model");

        let metric = V6Metric::fixed(V6Cell::DEFAULT);
        sess.machine.screen.v6_mut().unwrap().paint_run(
            7,
            V6Text::derived(17, 1, "painted".to_string(), 0, ZColour::Default, ZColour::Default, V6Cell::DEFAULT),
            &metric,
        );
        let after = Engine::screen_now(&sess);
        assert!(
            !std::sync::Arc::ptr_eq(&again, &after),
            "a paint moved the v6 generation: the next frame must be a fresh build"
        );
        assert!(
            model_run_texts(&after).contains(&"painted".to_string()),
            "…and the fresh build carries the new run: {:?}",
            model_run_texts(&after)
        );
    }

    /// SQ-1191 stale-model trap: a restored `ScreenState` carries a generation
    /// with no history, so its numbers can COLLIDE with the one the memo holds.
    /// `restore_screen` drops the memo, so the first frame after the restore is
    /// built from the restored table even when the generations match exactly.
    #[test]
    fn restore_screen_drops_the_memoized_model() {
        use zvm::screen::{V6Cell, V6Metric, V6Text, V6Windows, ZColour, ZWindow};
        let mut sess = v6_session_with_run("before");
        let held = Engine::screen_now(&sess);
        assert!(model_run_texts(&held).contains(&"before".to_string()));

        // A replacement screen with different content — and, deliberately, the
        // SAME generation number the memo was keyed on.
        let mut saved = zvm::screen::ScreenState::default();
        let mut windows: [ZWindow; 8] = Default::default();
        windows[7] = ZWindow { x_size: 64, y_size: 48, ..Default::default() };
        saved.v6 = Some(V6Windows { windows, current: 7 });
        let metric = V6Metric::fixed(V6Cell::DEFAULT);
        saved.v6_mut().unwrap().paint_run(
            7,
            V6Text::derived(1, 1, "after".to_string(), 0, ZColour::Default, ZColour::Default, V6Cell::DEFAULT),
            &metric,
        );
        saved.v6_generation = sess.machine.screen.v6_generation();

        restore_screen(&mut sess, saved);
        let fresh = Engine::screen_now(&sess);
        assert!(
            !std::sync::Arc::ptr_eq(&held, &fresh),
            "the memo must not survive a wholesale screen install"
        );
        let texts = model_run_texts(&fresh);
        assert!(
            texts.contains(&"after".to_string()) && !texts.contains(&"before".to_string()),
            "the first post-restore frame is built from the restored table: {texts:?}"
        );
    }
}

#[cfg(test)]
mod debugger_impl_tests {
    use super::*;
    use crate::engine::Engine;

    // minizork.z3 is a real game with a populated dictionary and object table
    // (unlike the synthetic read_char_story_v5 fixture, which has neither), so
    // it exercises every Debugger method meaningfully. It's the same fixture
    // zvm's own dictionary/objects/location tests use for this reason.
    fn zvm_session() -> Option<GameSession> {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/minizork.z3");
        if !fixture_path.exists() {
            return None; // fixture absent — skip
        }
        let story = std::fs::read(&fixture_path).expect("read minizork.z3");
        Some(GameSession::new(story, true, false, None).expect("GameSession::new with minizork.z3"))
    }

    #[test]
    fn parked_read_renders_as_a_read_op_before_the_pc() {
        // At an input prompt, `step()` advances state.pc PAST the read, so `pc()`
        // is the code that consumes the input — NOT the read. fold_confirmations
        // confirms the parked read instruction (pending_read_pc) so it still renders
        // as a real read op immediately before the PC, instead of being eaten by a
        // stale tiling and shown as some other opcode. (read-pc disasm fix)
        let Some(session) = zvm_session() else { return };
        assert_eq!(session.pending_input(), InputKind::Line,
            "GameSession::new runs minizork to its first line prompt");
        let dbg = session.debugger().expect("z-machine debugger");
        let pc = dbg.pc();
        let read_addr = dbg.prev_instr(pc);
        let row = &dbg.disassemble(read_addr, 1)[0];
        assert!(row.contains("read"), "parked read renders as a read op, got {row:?}");
        assert_eq!(dbg.next_instr(read_addr), pc, "the read is the instruction immediately before the parked PC");
    }

    #[test]
    fn a_turn_reopens_the_confirmation_gate_at_the_same_prompt() {
        // The per-turn confirmation gate is keyed on the parked PC, but a
        // look/examine returns to the SAME input prompt — its freshly-executed
        // boundaries (which correct false routine headers) must still be folded, so
        // a turn must reopen the gate rather than skip confirmation. (read-pc follow-up)
        let Some(mut session) = zvm_session() else { return };
        session.set_debug_trace(true);
        let pc = session.machine.state.pc;
        let _ = session.debugger().unwrap().disassemble(pc, 1); // builds + confirms, closes the gate
        assert_eq!(session.last_confirmed_pc.get(), Some(pc), "confirm closes the gate on the parked PC");
        let _ = Engine::submit(&mut session, "look");
        assert_eq!(session.last_confirmed_pc.get(), None, "a turn must reopen the confirmation gate");
    }

    // A read-only debug inspection must never leave a latched memory fault in the
    // shared VM `Memory`: the disassembler can walk past code into data, and an
    // OOB read latches into the fault cell the CPU drains each step — so a phantom
    // fault would halt the *game* on its next instruction (the "crash only when
    // /debug is open" bug).
    #[test]
    fn debugger_reads_do_not_leak_a_memory_fault_into_the_vm() {
        let Some(s) = zvm_session() else { return };
        let end = s.machine.mem.len() as u32;
        // Latch a fault the way an out-of-range disassembly read would.
        let _ = s.machine.mem.read_word(end + 100);
        assert!(s.machine.mem.take_mem_fault().is_some(), "sanity: OOB read latches a fault");
        let _ = s.machine.mem.read_word(end + 100); // re-latch (the check above drained it)
        // Any Debugger read must leave the fault cell clean.
        let pc = s.machine.state.pc;
        let dbg = s.debugger().expect("zvm has a debugger");
        let _ = dbg.disassemble(pc, 8);
        assert!(
            s.machine.mem.take_mem_fault().is_none(),
            "a debug read left a phantom fault that would halt the VM on its next step"
        );
        // prev_instr does far more boundary probing (a decode-chain sweep over
        // a whole window) — verify it doesn't leak a fault either.
        let _ = s.machine.mem.read_word(end + 100); // re-latch
        let _ = dbg.prev_instr(pc);
        assert!(
            s.machine.mem.take_mem_fault().is_none(),
            "prev_instr left a phantom fault that would halt the VM on its next step"
        );
        // object_detail reads attributes + property bytes — verify it drains too.
        let _ = s.machine.mem.read_word(end + 100); // re-latch
        let _ = dbg.object_detail(1);
        assert!(
            s.machine.mem.take_mem_fault().is_none(),
            "object_detail left a phantom fault that would halt the VM on its next step"
        );
    }

    #[test]
    fn object_addr_map_maps_object_one_entry_address() {
        let Some(s) = zvm_session() else { return };
        let objs = s.object_addr_map();
        let obj1_addr = zvm::objects::object_entry_addr(&s.machine.mem, 1);
        assert_eq!(objs.get(&obj1_addr), Some(&1));
    }

    #[test]
    fn dict_addr_map_maps_an_entry_to_its_word() {
        let Some(s) = zvm_session() else { return };
        let dict = s.dict_addr_map();
        assert!(!dict.is_empty(), "minizork has a populated dictionary");
        // Every mapped entry decodes to a non-empty word.
        assert!(dict.values().all(|w| !w.is_empty()));
    }

    #[test]
    fn annotate_refs_appends_obj_tag_for_object_entry_address() {
        let Some(s) = zvm_session() else { return };
        let objs = s.object_addr_map();
        let dict = s.dict_addr_map();
        let obj1_addr = zvm::objects::object_entry_addr(&s.machine.mem, 1);
        let line = format!("004a2f  loadw @0x{obj1_addr:06x}, #00");
        let out = s.annotate_refs(&line, &objs, &dict);
        assert!(out.contains(&format!("@0x{obj1_addr:06x} [obj#1]")), "got: {out}");
    }

    #[test]
    fn annotate_refs_appends_word_tag_for_dictionary_entry_address() {
        let Some(s) = zvm_session() else { return };
        let objs = s.object_addr_map();
        let dict = s.dict_addr_map();
        // Pick a dictionary entry whose address is not also an object entry base.
        let (&addr, word) = dict
            .iter()
            .find(|(a, _)| !objs.contains_key(a))
            .expect("some dict entry is not an object entry");
        let line = format!("004a2f  storeb @0x{addr:06x}, #01");
        let out = s.annotate_refs(&line, &objs, &dict);
        assert!(out.contains(&format!("@0x{addr:06x} [{word}]")), "got: {out}");
    }

    #[test]
    fn annotate_refs_leaves_non_matching_reference_unchanged() {
        let Some(s) = zvm_session() else { return };
        let objs = s.object_addr_map();
        let dict = s.dict_addr_map();
        // 0xffffff is neither an object nor a dictionary entry base.
        let line = "004a2f  loadw @0xffffff, #00".to_string();
        let out = s.annotate_refs(&line, &objs, &dict);
        assert_eq!(out, line);
    }

    // ── DisasmCache integration (SQ-0418, Task 6) ──────────────────────────
    // The five disassembly Debugger methods now route through GameSession's
    // lazily-built, memoized DisasmCache. These assert integration stability
    // and nav consistency through the &dyn Debugger surface; the cache's own
    // classification/format guarantees are unit-tested in zvm.

    fn is_six_hex(s: &str) -> bool {
        s.len() >= 6 && s.as_bytes()[..6].iter().all(|b| b.is_ascii_hexdigit())
    }

    #[test]
    fn disassemble_routes_through_cache_and_formats_a_real_line() {
        let Some(s) = zvm_session() else { return };
        let pc = Debugger::pc(&s);
        let line = s.disassemble(pc, 1);
        assert_eq!(line.len(), 1, "one requested line -> one line");
        assert!(!line[0].is_empty(), "line is non-empty");
        assert!(is_six_hex(&line[0]), "line begins with a 6-hex address: {:?}", line[0]);
        assert!(&line[0][6..8] == "  ", "6-hex address followed by two spaces: {:?}", line[0]);
    }

    #[test]
    fn nav_boundary_round_trip_and_monotonicity() {
        let Some(s) = zvm_session() else { return };
        let b = Debugger::pc(&s);
        let n = s.next_instr(b);
        let back = s.prev_instr(n);
        // `n` is a real unit boundary produced by next_instr, so stepping
        // forward from prev_instr(n) returns to n.
        assert_eq!(s.next_instr(back), n, "boundary round-trip holds");
        assert!(s.next_instr(n) >= n, "next_instr is non-decreasing");
        assert!(s.prev_instr(n) <= n, "prev_instr is non-increasing");
    }

    #[test]
    fn prev_instr_clamps_without_stalling() {
        let Some(s) = zvm_session() else { return };
        let mut a = Debugger::pc(&s);
        for _ in 0..500 {
            a = s.prev_instr(a);
        }
        // Reached the region-start clamp: stable fixpoint, no panic/hang.
        assert_eq!(s.prev_instr(a), a, "prev_instr is stable at the region-start clamp");
    }

    #[test]
    fn disassemble_window_is_bounded_and_has_no_empty_lines() {
        let Some(s) = zvm_session() else { return };
        let out = s.disassemble(Debugger::pc(&s), 200);
        assert!(out.len() <= 200, "never returns more lines than requested");
        assert!(out.iter().all(|l| !l.is_empty()), "no empty lines");
    }

    #[test]
    fn all_three_modes_agree_on_the_address_prefix() {
        let Some(s) = zvm_session() else { return };
        let pc = Debugger::pc(&s);
        let full = s.disassemble(pc, 1);
        let basic = s.disassemble_basic(pc, 1);
        let raw = s.disassemble_raw(pc, 1);
        assert_eq!(full.len(), 1);
        assert_eq!(basic.len(), 1);
        assert_eq!(raw.len(), 1);
        let addr6 = &full[0][..6];
        assert!(is_six_hex(&full[0]));
        assert_eq!(&basic[0][..6], addr6, "basic shares the address prefix");
        assert_eq!(&raw[0][..6], addr6, "raw shares the address prefix");
        assert_eq!(&raw[0][6..7], ":", "raw's distinct prefix is a colon after the address: {:?}", raw[0]);
    }

    #[test]
    fn zvm_exposes_a_debugger() {
        let Some(s) = zvm_session() else { return };
        let d = s.debugger().expect("zvm has a debugger");
        assert_eq!(d.pc(), s.machine.state.pc);
        assert_eq!(d.globals_lines().len(), 240);
        assert!(!d.dictionary_lines().is_empty());
        assert!(!d.object_tree_lines().is_empty());
        assert_eq!(d.memory_len(), s.machine.mem.len() as u32);
        let hex = d.memory_hex(0, 2);
        assert_eq!(hex.len(), 2);
        assert!(hex[0].starts_with("000000"));
    }

    // ── The Memory view's decoded-Z-text column (SQ-0448 / SQ-0969) ─────────

    #[test]
    fn the_z_text_column_indexes_row_for_row_with_the_hex_dump() {
        // The two vectors are read by index at the same row, so a disagreement
        // about how many rows a window has would caption the wrong bytes — the
        // exact failure the column exists to prevent.
        let Some(s) = zvm_session() else { return };
        let d = s.debugger().expect("z-machine debugger");
        for (addr, rows) in [(0u32, 4usize), (0x100, 40), (d.memory_len() - 8, 4)] {
            assert_eq!(
                d.memory_hex(addr, rows).len(),
                d.memory_zstrings(addr, rows).len(),
                "hex and Z-text row counts must agree at 0x{addr:06x} x{rows}",
            );
        }
    }

    #[test]
    fn every_dictionary_row_the_inspector_lists_decodes_on_the_row_it_links_to() {
        // The real-game half: the Dictionary tab prints `@0x…… word` rows whose
        // address is what a click jumps the Memory view to, so the Z-text column
        // on the row that jump lands on must contain that row's own word — that
        // round trip is the confirmation SQ-0448/SQ-0969 exists to give.
        let Some(s) = zvm_session() else { return };
        let d = s.debugger().expect("z-machine debugger");
        let rows = d.dictionary_lines();
        assert!(!rows.is_empty(), "minizork has a dictionary");
        let mut odd = 0;
        for row in rows.iter().take(40) {
            let addr = u32::from_str_radix(&row[3..9], 16).expect("`@0x……` row prefix");
            let word = row[10..].trim();
            // The jump aligns down to the 16-byte grid; a key can also straddle
            // that boundary, so take the landing row and the one after it.
            let text: String = d.memory_zstrings(addr & !0xF, 2).into_iter().flatten().collect();
            assert!(
                text.contains(word),
                "row {row:?} must decode into its own landing row, got {text:?}",
            );
            odd += (addr % 2) as usize;
        }
        // Non-vacuity: minizork's entry_length is odd, so half these addresses
        // are odd — the case a decoder that assumed word alignment would get
        // wrong, and the reason each key is decoded from the address the table
        // puts it at rather than from a rounded one.
        assert!(odd > 0, "at least one of the checked entries is at an ODD address");
    }

    /// FALSIFY by decoding each row from its own boundary instead of from a
    /// known string start: every row of the header comes back `Some(…)` full of
    /// plausible-looking text and this fails on "the header is not a string at
    /// all" — which is precisely the wrong answer the anchoring exists to
    /// refuse.
    #[test]
    fn rows_no_table_accounts_for_decode_to_nothing_at_all() {
        // The header is not text, and no table claims it — so the column stays
        // empty there and the char column is all the view offers. Decoding
        // anyway would produce confident nonsense.
        let Some(s) = zvm_session() else { return };
        let d = s.debugger().expect("z-machine debugger");
        assert!(
            d.memory_zstrings(0, 4).iter().all(|z| z.is_none()),
            "the header is not a string at all",
        );
    }

    /// FALSIFY by crediting a whole string to the first row it touches (replace
    /// the per-word `word + 1` with the span's own start): every containment
    /// check still passes — a concatenation cannot see the difference — and the
    /// `split` guard is the one that fails, on "at least one split across two
    /// rows by its own bytes".
    #[test]
    fn real_object_names_decode_across_exactly_the_rows_that_hold_them() {
        let Some(s) = zvm_session() else { return };
        let d = s.debugger().expect("z-machine debugger");
        let mem = &s.machine.mem;
        let mut checked = 0;
        let mut shifted = 0;
        let mut long_word = 0;
        let mut split = 0;
        for obj in 1..=zvm::location::max_object_number(mem).min(60) {
            let name = zvm::objects::short_name(mem, obj);
            let name = name.trim();
            let Some((start, end)) = zvm::objects::short_name_span(mem, obj) else { continue };
            if name.is_empty() {
                continue;
            }
            // The rows the name's own bytes fall on, from the aligned row the
            // Memory view would land on.
            let base = start & !0xF;
            let rows = ((end - base) as usize).div_ceil(16);
            let per_row = d.memory_zstrings(base, rows);
            let text: String = per_row.iter().flatten().map(String::as_str).collect::<String>();
            assert!(
                text.contains(name),
                "object {obj}'s name {name:?} must appear across its own rows, got {text:?}",
            );
            // Genuinely split, not merely present: a name whose bytes cross a
            // 16-byte boundary must have no single row holding all of it. That
            // per-word attribution IS the column; crediting a whole string to
            // its first row reads identically to any check that concatenates.
            split += (rows > 1 && !per_row.iter().flatten().any(|t| t.contains(name))) as usize;
            checked += 1;
            shifted += name.chars().any(|c| c.is_ascii_uppercase()) as usize;
            // Only an abbreviation (§3.3) can make one 2-byte word produce more
            // than three characters, so a name longer than its own span allows
            // proves the expansion survived being attributed per row.
            long_word += (name.chars().count() > ((end - start) / 2 * 3) as usize) as usize;
        }
        assert!(checked >= 10, "minizork has plenty of named objects (checked {checked})");
        assert!(shifted > 0, "…at least one needing an alphabet shift mid-string");
        assert!(long_word > 0, "…at least one expanding an abbreviation");
        assert!(split > 0, "…and at least one split across two rows by its own bytes");
    }

    /// SQ-0975: clicking an object row jumps the Memory view to the row's
    /// `@0x……` token, so that address must be the one whose bytes hold the
    /// object's TEXT. §12.3's entry does not — it is flags, tree links and a
    /// pointer — while §12.4's property table opens with the name's word count
    /// and the name itself. Landing on the entry showed the wrong bytes, and for
    /// a low object number put the name far below the window.
    ///
    /// FALSIFY by restoring `object_entry_addr` as `object_tree_lines`' address
    /// closure: `lands_on_the_property_table` fails on the very first object,
    /// and with the assertion relaxed the decode check fails too — the name is
    /// nowhere in the window, which is the originally reported symptom.
    #[test]
    fn an_object_rows_address_lands_on_its_name_not_its_entry() {
        let Some(s) = zvm_session() else { return };
        let d = s.debugger().expect("z-machine debugger");
        let mem = &s.machine.mem;
        let lines = d.object_tree_lines();
        // `@0x{addr:06x} {indent}[{n}] {name}` — recover both ends of the row.
        let row = |line: &str| -> Option<(u32, u16)> {
            let addr = u32::from_str_radix(line.strip_prefix("@0x")?.get(..6)?, 16).ok()?;
            let n = line.split_once('[')?.1.split_once(']')?.0.parse().ok()?;
            Some((addr, n))
        };
        let mut checked = 0;
        let mut empty_named = 0;
        for line in &lines {
            let Some((addr, obj)) = row(line) else { continue };
            let ptbl = zvm::objects::object_prop_table_addr(mem, obj).expect("a real object");
            assert_eq!(addr, ptbl, "object {obj}'s row must point at its property table");
            // §12.4's length byte is the first thing the landing row shows.
            let name_words = mem.read_byte(ptbl) as u32;
            if name_words == 0 {
                // A zero-length name is legal; the table is still the sensible
                // landing, because the count byte lives there to be read.
                empty_named += 1;
                assert_eq!(zvm::objects::short_name_span(mem, obj), None);
                continue;
            }
            // The Memory view aligns a jump down to the 16-byte row grid, so the
            // name decodes at the TOP of the window it opens — that pairing is
            // the whole point of moving the address.
            let base = addr & !0xF;
            let zs = d.memory_zstrings(base, 4);
            let first = zs.iter().position(|z| z.is_some());
            assert!(
                matches!(first, Some(0 | 1)),
                "object {obj}'s decode must start in the window's first rows, got {first:?}",
            );
            let name = zvm::objects::short_name(mem, obj);
            let window: String = zs.iter().flatten().map(String::as_str).collect();
            assert!(
                window.contains(name.trim()),
                "object {obj}'s name {name:?} must be readable at the top of its own window, got {window:?}",
            );
            checked += 1;
        }
        assert!(checked >= 20, "minizork has plenty of named objects (checked {checked})");
        // Non-vacuity for the empty-name branch: minizork's object table is a
        // real one, so if it happens to hold no unnamed object the branch is
        // simply untested here — `objects.rs` pins it on a built fixture.
        let _ = empty_named;
    }

    /// The §12.3 entry did not become unreachable when the row stopped pointing
    /// at it (SQ-0975): expanding an object publishes it as the detail's own
    /// `@0x……` link, which the panel hit-tests exactly like the tree row's.
    #[test]
    fn an_expanded_object_publishes_its_entry_address_as_its_own_link() {
        let Some(s) = zvm_session() else { return };
        let d = s.debugger().expect("z-machine debugger");
        let detail = d.object_detail(1);
        let entry = zvm::objects::object_entry_addr(&s.machine.mem, 1);
        assert_eq!(
            detail.first().map(String::as_str),
            Some(format!("entry @0x{entry:06x}").as_str()),
            "the entry leads the detail, as a clickable address: {detail:?}",
        );
        // …and it is a DIFFERENT address from the row's, or nothing was gained.
        assert_ne!(
            Some(entry),
            zvm::objects::object_prop_table_addr(&s.machine.mem, 1),
            "the two addresses are genuinely distinct",
        );
    }

    // ── Runtime-confirmation fold (SQ-0418, Task 9) ────────────────────────
    // Executed/parked PCs and call-stack func_addrs are folded into the cache
    // once per turn so regions the VM really runs self-heal to Instr boundaries.

    #[test]
    fn parked_pc_becomes_an_instr_boundary_after_confirmation() {
        let Some(s) = zvm_session() else { return };
        let p = Debugger::pc(&s);
        // A disassemble read builds the cache then folds the parked PC in.
        let line = s.disassemble(p, 1);
        // p is now the start of an Instr unit: stepping to the next unit and back
        // lands exactly on p (prev/next are unit-boundary ops).
        assert_eq!(s.prev_instr(s.next_instr(p)), p, "parked pc is a unit boundary after confirmation");
        // The first disassembled line is addressed exactly at p.
        assert_eq!(line.len(), 1);
        assert!(line[0].starts_with(&format!("{p:06x}")), "first line starts at p: {:?}", line[0]);
    }

    #[test]
    fn frame_func_addrs_are_promoted_to_routine_headers() {
        let Some(s) = zvm_session() else { return };
        let _ = s.disassemble(Debugger::pc(&s), 1); // build cache + fold the call stack in
        for f in &s.machine.state.frames {
            // Only func_addrs inside the code region get a header; disassembling
            // at one now shows a RoutineHeader unit line ("; routine").
            let hdr = s.disassemble(f.func_addr, 1);
            if hdr.is_empty() {
                continue; // outside the tiled code region
            }
            assert!(
                hdr[0].contains("; routine"),
                "func_addr {:06x} did not become a routine header: {:?}",
                f.func_addr, hdr[0]
            );
        }
    }

    #[test]
    fn confirmation_is_idempotent_and_stable() {
        let Some(s) = zvm_session() else { return };
        let _ = s.disassemble(Debugger::pc(&s), 1); // build cache + first fold
        s.confirm_disasm();
        let first = s.disassemble(Debugger::pc(&s), 50);
        s.confirm_disasm();
        let second = s.disassemble(Debugger::pc(&s), 50);
        assert_eq!(first, second, "confirmation must not oscillate the disasm window");
    }
}

#[cfg(test)]
mod untried_turn_tests {
    use super::*;
    use mapper::direction::Direction;

    fn turn(loc: Option<(u16, &str)>, transcript: &str) -> TurnResult {
        TurnResult {
            transcript: transcript.into(),
            transcript_runs: Vec::new(),
            location: loc.map(|(n, name)| zvm::ObjectSnapshot { number: n, parent: 0, name: name.into() }),
            quit: false, erase_lower: false, info: None, sounds: Vec::new(),
            glulx_sound_ops: Vec::new(), diagnostics: vec![], fault: None,
            location_method: None, pending_io: None, timed_out: false,
            pictures: Vec::new(), transcript_elems: Vec::new(), prose_retired: None,
        }
    }

    /// SQ-0391: every direction the player types in a room is tried, including the ones that go
    /// nowhere and the ones that get them killed. Each of these turns takes a different path
    /// through `apply_turn`, and two of them never reach `Mapper::observe` at all.
    #[test]
    fn every_typed_direction_counts_as_tried_however_the_turn_ends() {
        let mut m = Mapper::default();
        apply_turn(&mut m, "", &turn(Some((1, "Hall")), ""), &mut Default::default());

        // 1. A move that simply fails: the same room comes back.
        apply_turn(
            &mut m,
            "north",
            &turn(Some((1, "Hall")), "You can't go that way."),
            &mut Default::default(),
        );
        assert!(!m.graph.untried(1).contains(&Direction::N), "a foiled move is still a try");

        // 2. A turn that detected no location at all — the mapper is otherwise skipped.
        apply_turn(&mut m, "west", &turn(None, "It is pitch black."), &mut Default::default());
        assert!(!m.graph.untried(1).contains(&Direction::W), "no location detected is still a try");

        // 3. A death that teleported the player elsewhere. `observe_relocation` mints no edge, by
        //    design — but the direction that killed you has emphatically been tried.
        apply_turn(
            &mut m,
            "east",
            &turn(Some((2, "Forest")), "*** You have died ***"),
            &mut Default::default(),
        );
        assert!(!m.graph.untried(1).contains(&Direction::E), "the move that killed you is a try");
        assert_eq!(m.graph.connections().len(), 0, "and still mints no false edge (SQ-0259)");

        // The ways never typed are all that remain on offer.
        let left = m.graph.untried(1);
        assert!(left.contains(&Direction::S) && left.contains(&Direction::Up), "{left:?}");
    }

}
