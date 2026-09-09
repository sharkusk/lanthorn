//! A headless GRAPHICAL host for a Version 6 story — the seams `run_story`
//! refuses, drawn into a PPM instead of onto a screen.
//!
//! [`run_story`](../run_story.rs) is the text-only half of this pair and turns
//! a Version 6 story away, because v6 does not print into a scrolling column:
//! it paints. This example is what a host has to do instead, with every step
//! reduced to the smallest thing that still exercises the seam:
//!
//! * **boot as a machine, not as a default.** [`BootConfig`] carries the five
//!   facts a graphical boot needs — the picture space, how dense the artwork
//!   in it is, the character cell, the colour table and the interpreter
//!   number — and applies them in the one order that works (see
//!   `zvm::cpu::boot`). Assemble them by hand and the *game* lays its windows
//!   out on a screen the player never sees.
//! * **answer `picture_data` on demand.** [`Resources`] is three questions
//!   about a picture file; [`StubPictures`] below answers them from a formula
//!   so this example needs no archive parser. A real host implements the same
//!   trait over a Blorb `Pict` chunk or a release disk's native picture file,
//!   and answers in the ARCHIVE's own pixels — `BootConfig` scales every
//!   answer into the story's unit screen for it, so no implementation of this
//!   trait ever has to know the scale.
//! * **drain what the story painted.** The engine never rasterises. It records
//!   `draw_picture`/`erase_picture` calls and the rectangles `erase_window`
//!   filled, and [`Machine::take_paint_events`] hands them back on one
//!   timeline in issue order. Replaying pictures and fills as two lists paints
//!   the turn in the wrong order.
//! * **draw the text from the window table.** Each v6 window keeps its printed
//!   runs as pixel-positioned paint (`ZWindow::texts`), and the machine's
//!   [`V6Metric`] says how wide a character is. This example fills a box per
//!   glyph; a real host blits the release's own typeface at that same cell.
//! * **snapshot the screen beside the memory.** Quetzal deliberately carries
//!   no screen state, so a host "Save State" that swaps memory under a running
//!   game must carry the screen itself: [`Machine::screen_snapshot`] and
//!   [`Machine::restore_screen_snapshot`], plus the folded paint history from
//!   [`Machine::paint_log`], are that pair. The run below proves it by
//!   restoring and re-rendering.
//!
//! What a real graphical host does that this does not: decode the pictures
//! (this draws flat rectangles at the reported size), render glyphs (this
//! draws boxes), lay out window 0's scrolling prose (that arrives through the
//! [`Output`] sink, exactly as in `run_story`, and is the host's to wrap and
//! page), honour text styles beyond reverse video, play sounds, and take its
//! input from a person rather than a fixed script.
//!
//! ```text
//! cargo run -p lanthorn-zvm --example v6_host -- stories/zork0-r393-s890714.z6
//! cargo run -p lanthorn-zvm --example v6_host -- story.z6 --turns 6 --out /tmp/frame.ppm
//! ```

use std::any::Any;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};

use zvm::cpu::exec::{BootConfig, Machine, PaintEvent, StepResult};
use zvm::io::Output;
use zvm::memory::Memory;
use zvm::resources::Resources;
use zvm::screen::{rgb15_to_888, Palette, V6Cell, V6Metric, V6Text, ZColour};

/// ZSCII 13 — "Enter", the terminator of an ordinary typed line (ZMSD §3.8).
const ENTER: u8 = 13;

/// The machine this example presents: the 320x200 picture space of Infocom's
/// own Version 6 renditions, drawn at two device pixels per art pixel, on the
/// 8x16 cell every machine but the Macintosh declares.
///
/// A real host reads the first two off the archive (a Blorb `Reso` chunk, or a
/// release disk's native picture file) and the cell off the machine profile
/// the medium named — a Macintosh is 7x15 and a 480x300 plate at 1:1.
const PICTURE_SPACE: (u16, u16) = (320, 200);
const ART_SCALE: (u32, u32) = (2, 2);
const CELL: V6Cell = V6Cell::new(8, 16);

// ── The host's three implementations ────────────────────────────────────────

/// The [`Output`] sink: window 0's scrolling prose, accumulated so the run can
/// report what the story said. Version 6 paints everything else, so unlike
/// `run_story`'s sink this one is not the whole screen — it is one window's
/// worth of text the host would wrap and page for itself.
#[derive(Default)]
struct TranscriptSink {
    text: String,
}

impl Output for TranscriptSink {
    fn print(&mut self, s: &str) {
        self.text.push_str(s);
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// [`Resources`] answered from a formula rather than an archive.
///
/// The three questions are ZMSD §15 `picture_data`'s: how many pictures the
/// file holds, what release number it carries, and how big one picture is.
/// Parsing a Blorb or an Infocom `.mg1` is a whole archive reader and this
/// crate takes no dependencies, so the sizes here are INVENTED — deterministic,
/// plausible, and wrong. The story lays its border and banners out from them,
/// so the frame this example writes is a faithful drawing of a screen no real
/// interpreter would show; that is the price of a dependency-free example, and
/// the seam it demonstrates is unaffected.
///
/// Note what is NOT here: any notion of the ×2 art scale. Answers are in the
/// picture file's OWN pixels and `BootConfig` scales every one of them into the
/// story's unit screen on the way out, whichever way the resources were
/// installed — so [`Machine::picture_dims`] below reports doubled values that
/// this implementation never computed.
struct StubPictures;

impl StubPictures {
    /// As many pictures as *Zork Zero*'s own file is in the same ballpark of —
    /// enough that the story finds every number it asks for.
    const COUNT: u16 = 64;
}

impl Resources for StubPictures {
    fn picture_count(&self) -> u16 {
        Self::COUNT
    }
    fn picture_release(&self) -> u16 {
        1
    }
    fn picture_dims(&self, number: u16) -> Option<(u16, u16)> {
        if number == 0 || number > Self::COUNT {
            return None;
        }
        // Deterministic and bounded well inside the picture space, so nothing
        // the story draws can be clipped away entirely.
        Some((16 + (number % 9) * 8, 16 + (number % 5) * 8))
    }
}

/// A packed RGB framebuffer in the story's own native pixels.
#[derive(Clone, PartialEq, Eq)]
struct Frame {
    w: u32,
    h: u32,
    px: Vec<u8>,
}

impl Frame {
    fn new(w: u32, h: u32, fill: (u8, u8, u8)) -> Frame {
        let mut f = Frame {
            w,
            h,
            px: vec![0; (w as usize) * (h as usize) * 3],
        };
        f.fill(0, 0, i64::from(w), i64::from(h), fill);
        f
    }

    /// Paint a rectangle, clipped to the frame. Coordinates are 0-based; the
    /// engine reports v6 geometry 1-based (ZMSD §8.8.1), so every caller here
    /// subtracts one exactly once.
    fn fill(&mut self, x: i64, y: i64, w: i64, h: i64, rgb: (u8, u8, u8)) {
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w).min(i64::from(self.w));
        let y1 = (y + h).min(i64::from(self.h));
        for row in y0..y1 {
            let base = (row as usize * self.w as usize + x0 as usize) * 3;
            for i in 0..(x1 - x0).max(0) as usize {
                self.px[base + i * 3] = rgb.0;
                self.px[base + i * 3 + 1] = rgb.1;
                self.px[base + i * 3 + 2] = rgb.2;
            }
        }
    }

    /// Binary PPM (P6), maxval 255 — the simplest image format with no encoder.
    fn write_ppm(&self, path: &Path) -> io::Result<()> {
        let mut out = format!("P6\n{} {}\n255\n", self.w, self.h).into_bytes();
        out.extend_from_slice(&self.px);
        fs::write(path, out)
    }
}

// ── Rendering ───────────────────────────────────────────────────────────────

/// One §8.3 logical colour as RGB, resolved through THIS machine's table.
///
/// `palette` is a parameter for the reason `zvm` made it a `Machine` field:
/// resolving a standard colour number without saying whose table you mean is
/// the bug (SQ-1393). `default_number` is the header's own `$2C`/`$2D` entry
/// for the channel, which is what `ZColour::Default` means.
fn rgb(c: ZColour, palette: Palette, default_number: u8) -> (u8, u8, u8) {
    rgb15_to_888(c.true_value(palette, default_number))
}

/// Draw one painted text run as a box per glyph.
///
/// The run's pixel origin is where the machine actually put the first glyph and
/// the cell is what the story was told a character measures, so this is the
/// geometry seam a real renderer blits a typeface through — with the glyph
/// bitmaps swapped in for the ink box below.
fn draw_run(f: &mut Frame, run: &V6Text, cell: V6Cell, palette: Palette, def_bg: u8, def_fg: u8) {
    let (mut ink, mut paper) = (rgb(run.fg, palette, def_fg), rgb(run.bg, palette, def_bg));
    // ZMSD §8.7.2, style bit 0: reverse video swaps the two.
    if run.style & 1 != 0 {
        std::mem::swap(&mut ink, &mut paper);
    }
    let (cw, ch) = (i64::from(cell.w()), i64::from(cell.h()));
    let y = i64::from(run.y.max(1)) - 1;
    for (i, c) in run.text.chars().enumerate() {
        let x = i64::from(run.x.max(1)) - 1 + i as i64 * cw;
        f.fill(x, y, cw, ch, paper);
        if !c.is_whitespace() {
            f.fill(x + 1, y + 2, cw - 2, ch - 4, ink);
        }
    }
}

/// Rasterise the machine's current screen: the paint the story issued, then the
/// text the window table is holding.
///
/// `display` is the host's own display list — every [`PaintEvent`] drained so
/// far, in issue order. The engine does not keep one (it hands each turn's
/// events over once and forgets them), which is why a host Save State must
/// archive this list beside the machine's own bytes.
fn render(m: &Machine, display: &[PaintEvent]) -> Frame {
    // Header `$22`/`$24`: the Version 6 screen in native pixels (ZMSD §8.4).
    let (w, h) = (
        u32::from(m.mem.read_word(0x22)),
        u32::from(m.mem.read_word(0x24)),
    );
    let palette = m.palette();
    let def_bg = m.mem.read_byte(0x2C);
    let def_fg = m.mem.read_byte(0x2D);
    let mut f = Frame::new(w.max(1), h.max(1), rgb(ZColour::Default, palette, def_bg));

    for ev in display {
        match *ev {
            PaintEvent::Erase(fill) => {
                let (x, y) = (i64::from(fill.x.max(1)) - 1, i64::from(fill.y.max(1)) - 1);
                f.fill(
                    x,
                    y,
                    i64::from(fill.w),
                    i64::from(fill.h),
                    rgb(fill.bg, palette, def_bg),
                );
            }
            PaintEvent::Picture(p) => {
                // The picture's box is inside the window's box AT THE MOMENT OF
                // THE CALL (SQ-0715) — a v6 game borrows one window for picture
                // after picture, moving and resizing it between each, so the
                // window's CURRENT box would clip and place every one of them
                // wrongly.
                let (wx, wy) = (
                    i64::from(p.win_box.0.max(1)) - 1,
                    i64::from(p.win_box.1.max(1)) - 1,
                );
                let (x, y) = (
                    wx + i64::from(p.x.max(1)) - 1,
                    wy + i64::from(p.y.max(1)) - 1,
                );
                let Some((pw, ph)) = m.picture_dims(p.number) else {
                    continue;
                };
                // Clip to the drawing window, as a real interpreter does.
                let cw = i64::from(pw).min(wx + i64::from(p.win_box.2) - x);
                let ch = i64::from(ph).min(wy + i64::from(p.win_box.3) - y);
                // An erase_picture blanks the region; a draw fills it with a
                // flat placeholder, since nothing here decodes an image.
                let paint = if p.erase {
                    rgb(ZColour::Default, palette, def_bg)
                } else {
                    // Deterministic per picture number, so the frame is
                    // reproducible and different pictures are distinguishable.
                    let n = u32::from(p.number);
                    (
                        ((n * 53) % 200 + 40) as u8,
                        ((n * 97) % 200 + 40) as u8,
                        ((n * 29) % 200 + 40) as u8,
                    )
                };
                f.fill(x, y, cw, ch, paint);
            }
            // `PaintEvent` is `#[non_exhaustive]`: a future kind is skipped
            // rather than failing this build.
            _ => {}
        }
    }

    let cell = m.v6_cell();
    if let Some(v6) = m.screen.v6.as_ref() {
        for win in v6.windows.iter() {
            // `texts` is the window's live painted layer, `retired` what it left
            // behind when it moved (ZMSD §15: "window_size does not change the
            // current display"), `streamed` the shadow of prose it has sent to
            // the transcript and is still showing. All three are paint, in the
            // same screen-absolute pixel space.
            for run in win
                .texts
                .iter()
                .chain(win.retired.iter())
                .chain(win.streamed.iter())
            {
                draw_run(&mut f, run, cell, palette, def_bg, def_fg);
            }
        }
    }
    f
}

// ── Driving the story ───────────────────────────────────────────────────────

/// Where a turn stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pause {
    Line,
    Char,
    Quit,
    Fault,
}

/// A run that never reaches an input request is a bug in the host, not a story
/// that is thinking: this caps it so the example terminates headlessly.
const STEP_BUDGET: u64 = 60_000_000;

/// Step until the story wants input (or stops), draining everything it painted
/// into `display` on the way out.
fn advance(m: &mut Machine, display: &mut Vec<PaintEvent>) -> Pause {
    let mut steps = 0u64;
    let pause = loop {
        match m.step() {
            StepResult::Continue => {
                steps += 1;
                if steps >= STEP_BUDGET {
                    break Pause::Fault;
                }
            }
            StepResult::NeedLine { .. } => break Pause::Line,
            StepResult::NeedChar => break Pause::Char,
            StepResult::Quit => break Pause::Quit,
            StepResult::Fault => break Pause::Fault,
            // ZMSD §6.1.3: answer with `restart()`, never by rebuilding the
            // machine — that would lose the two game-writable Flags 2 bits.
            StepResult::Restart => m.restart(),
            // Nowhere to write, so decline every in-game save and fail every
            // in-game restore. The HOST snapshot below is a different thing
            // entirely and does not go through these.
            StepResult::SaveRequest => m.complete_save(false),
            StepResult::RestoreRequest => m.complete_restore_failure(),
            _ => {}
        }
    };
    display.extend(m.take_paint_events());
    pause
}

/// The scripted input this run answers with. Enough blank lines and keypresses
/// to walk *Zork Zero*'s opening cards; a host with a person attached reads
/// these from a keyboard.
const SCRIPT: [&str; 4] = ["", "look", "wait", ""];

/// Answer whichever input request the story parked on, from `SCRIPT[cursor]`.
/// The cursor is HOST state and travels with the snapshot below, the same way
/// the display list does.
fn answer(m: &mut Machine, pause: Pause, cursor: &mut usize) {
    match pause {
        Pause::Line => {
            let line = SCRIPT[*cursor % SCRIPT.len()];
            *cursor += 1;
            m.supply_line(line, ENTER);
        }
        Pause::Char => {
            *cursor += 1;
            m.supply_char(ENTER);
        }
        Pause::Quit | Pause::Fault => {}
    }
}

// ── The run ─────────────────────────────────────────────────────────────────

struct Args {
    story: PathBuf,
    turns: u32,
    out: PathBuf,
}

/// A scratch path unique per CALL, not per process.
///
/// The pid alone is unique per process, which under a multi-threaded test
/// binary is the same directory for every caller — so the counter is what makes
/// the name distinct. `zvm` takes no dependencies, so it is spelled here rather
/// than reached for.
fn default_out_path() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    env::temp_dir().join(format!("zvm-v6-host-{}-{n}.ppm", process::id()))
}

fn default_story() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/zork0-r393-s890714.z6")
}

fn parse_args<I: Iterator<Item = String>>(mut it: I) -> Result<Args, String> {
    let mut story = None;
    let mut turns = 4u32;
    let mut out = None;
    while let Some(a) = it.next() {
        match a.as_str() {
            "--turns" => {
                let v = it.next().ok_or("--turns wants a number")?;
                turns = v
                    .parse()
                    .map_err(|_| format!("--turns: not a number: {v}"))?;
            }
            "--out" => out = Some(PathBuf::from(it.next().ok_or("--out wants a path")?)),
            "--help" | "-h" => {
                return Err("usage: v6_host [story.z6] [--turns N] [--out FILE]".into())
            }
            other if other.starts_with('-') => return Err(format!("unknown option: {other}")),
            other => story = Some(PathBuf::from(other)),
        }
    }
    Ok(Args {
        story: story.unwrap_or_else(default_story),
        turns,
        out: out.unwrap_or_else(default_out_path),
    })
}

/// Boot, drive, render, and prove the snapshot pair round-trips. Returns the
/// report `main` prints, or the reason it could not.
fn run(args: &Args) -> Result<String, String> {
    let bytes =
        fs::read(&args.story).map_err(|e| format!("cannot read {}: {e}", args.story.display()))?;
    let mem = Memory::new(bytes).map_err(|e| format!("invalid story file: {e:?}"))?;
    if mem.version() != 6 {
        return Err(format!(
            "{} is a Version {} story; this example is the graphical half of the pair — use `run_story` for text",
            args.story.display(),
            mem.version()
        ));
    }

    // Every fact the machine needs, in one value, applied in one order.
    let config = BootConfig::new()
        .with_v6_screen_px(PICTURE_SPACE)
        .with_v6_art_scale(ART_SCALE)
        .with_v6_text(V6Metric::fixed(CELL))
        .with_palette(Palette::Standard)
        .with_interpreter_number(Some(6)) // ZMSD §11.1.3: 6 = IBM PC
        .with_default_colours(9, 2) // §8.3.1: white paper, black ink
        .with_honor_game_colours(true)
        .with_resources(Box::new(StubPictures));
    let mut m = Machine::boot(mem, Box::new(TranscriptSink::default()), config);

    let mut report = String::new();
    let _ = writeln!(
        report,
        "booted {} at {}x{} native px, cell {}x{}, {} pictures (picture 1 reports {:?} after the {:?} art scale)",
        args.story.display(),
        m.mem.read_word(0x22),
        m.mem.read_word(0x24),
        m.v6_cell().w(),
        m.v6_cell().h(),
        m.picture_count(),
        m.picture_dims(1),
        ART_SCALE,
    );

    // The host's own display list, and the input cursor: both host state, both
    // archived with the snapshot below.
    let mut display: Vec<PaintEvent> = Vec::new();
    let mut cursor = 0usize;

    let mut pause = advance(&mut m, &mut display);
    for turn in 1..=args.turns {
        if matches!(pause, Pause::Quit | Pause::Fault) {
            let _ = writeln!(report, "story stopped at turn {turn} ({pause:?})");
            break;
        }
        answer(&mut m, pause, &mut cursor);
        pause = advance(&mut m, &mut display);
    }
    let _ = writeln!(
        report,
        "{} paint events drained over {} turns",
        display.len(),
        args.turns
    );

    if matches!(pause, Pause::Quit | Pause::Fault) {
        return Err(format!(
            "story stopped ({pause:?}) before the snapshot demonstration"
        ));
    }

    // ── The snapshot pair ───────────────────────────────────────────────────
    //
    // Quetzal carries no screen state by design: the standard assumes the STORY
    // repaints after an in-game `@restore`. A host Save State gets no such
    // repaint — it swaps dynamic memory under a game that never learns it
    // happened — so everything the screen needs is the host's to carry.
    let quetzal = m.save_quetzal();
    let screen = m.screen_snapshot();
    let paint_log = zvm::paint_log::encode(m.paint_log());
    let saved_display = display.clone();
    let saved_cursor = cursor;
    let frame_at_snapshot = render(&m, &display);

    // Perturb: one more move, which must change the screen.
    answer(&mut m, pause, &mut cursor);
    let pause_after = advance(&mut m, &mut display);
    let frame_after_move = render(&m, &display);

    // Restore all four: memory and stack, the screen, the folded paint history
    // a redraw replays from, and the host's own display list and input cursor.
    m.restore_quetzal(&quetzal)
        .map_err(|e| format!("restore_quetzal: {e:?}"))?;
    m.restore_screen_snapshot(&screen)
        .map_err(|e| format!("restore_screen_snapshot: {e:?}"))?;
    m.restore_paint_log(&paint_log)
        .map_err(|e| format!("restore_paint_log: {e:?}"))?;
    display = saved_display;
    cursor = saved_cursor;
    // A Quetzal save taken at an input prompt records the READ, so the restored
    // machine re-executes it and parks exactly where the snapshot was taken.
    let pause_restored = advance(&mut m, &mut display);
    let frame_restored = render(&m, &display);

    if frame_restored != frame_at_snapshot {
        return Err("restored frame differs from the frame captured at snapshot time".into());
    }
    if frame_after_move == frame_at_snapshot {
        return Err("the perturbing move changed nothing — the round-trip proves nothing".into());
    }

    // CLAUDE.md's restore discipline: a restore bug surfaces one action AFTER
    // the restore, when the game next repaints. So make the SAME move again and
    // require the same screen — the frame immediately after a restore is when
    // everything still looks correct.
    if pause_restored != pause {
        return Err(format!(
            "restored machine parked on {pause_restored:?}, not {pause:?}"
        ));
    }
    answer(&mut m, pause_restored, &mut cursor);
    let pause_replayed = advance(&mut m, &mut display);
    let frame_replayed = render(&m, &display);
    if pause_replayed != pause_after || frame_replayed != frame_after_move {
        return Err("replaying the move after the restore produced a different screen".into());
    }

    let _ = writeln!(
        report,
        "snapshot round-trip: {} B Quetzal + {} B screen + {} B paint log restored the frame, and replaying the move reproduced the next one",
        quetzal.len(),
        screen.len(),
        paint_log.len(),
    );

    frame_replayed
        .write_ppm(&args.out)
        .map_err(|e| format!("cannot write {}: {e}", args.out.display()))?;
    let _ = writeln!(
        report,
        "wrote {}x{} PPM to {}",
        frame_replayed.w,
        frame_replayed.h,
        args.out.display()
    );

    let transcript = m
        .output()
        .as_any()
        .downcast_ref::<TranscriptSink>()
        .map_or(0, |s| s.text.chars().count());
    let _ = writeln!(
        report,
        "window 0 streamed {transcript} characters to the host transcript"
    );
    Ok(report)
}

fn main() {
    let args = match parse_args(env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("v6_host: {e}");
            process::exit(2);
        }
    };
    match run(&args) {
        Ok(report) => print!("{report}"),
        Err(e) => {
            eprintln!("v6_host: {e}");
            process::exit(1);
        }
    }
}

/// The whole example, against the fixture, so the gate compiles AND exercises
/// it. `stories/` is gitignored (commercial game files) and absent in CI, so
/// this skips vacuously rather than failing there.
#[test]
fn v6_host_boots_renders_and_round_trips_a_snapshot() {
    let story = default_story();
    if !story.exists() {
        eprintln!("SKIP: gitignored story missing at {}", story.display());
        return;
    }
    let args = Args {
        story,
        turns: 3,
        out: default_out_path(),
    };
    let report = match run(&args) {
        Ok(r) => r,
        Err(e) => panic!("v6_host example failed: {e}"),
    };
    eprint!("{report}");
    let meta = fs::metadata(&args.out).expect("the example wrote its PPM");
    assert!(meta.len() > 16, "PPM is header-only");
    let ppm = fs::read(&args.out).expect("read back the PPM");
    assert_eq!(&ppm[..2], b"P6", "binary PPM magic");
    let _ = fs::remove_file(&args.out);
}
