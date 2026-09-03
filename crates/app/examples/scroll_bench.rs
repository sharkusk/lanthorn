//! SQ-1033 instrument: does render cost grow with TOTAL SCROLLBACK LENGTH,
//! rather than with the ~40 rows actually on screen — on all three engines?
//!
//! This is a MEASUREMENT tool, not a fix. It does not change anything under
//! `crates/app/src/`; it drives real sessions to a target turn count and times
//! the real render entry point, `render::screen::render_story_pane`, against a
//! `ratatui::buffer::Buffer` — exactly the call the TUI makes every frame.
//!
//! ```sh
//! cargo run --release -p lanthorn --example scroll_bench
//! cargo run --release -p lanthorn --example scroll_bench -- --turns 100,1000,5000,20000 --repeats 8
//! cargo run --release -p lanthorn --example scroll_bench -- --engines zvm-cell,zvm-raster
//! ```
//!
//! **Always run `--release`.** A debug build's timings are dominated by
//! unoptimised code and say nothing about the shipped binary (CLAUDE.md's
//! debug/release note applies here as much as anywhere). This harness prints a
//! loud warning and labels every row `DEBUG` when built without optimisations,
//! so a debug number can never be mistaken for a release one.
//!
//! ## How the turns are driven — READ THIS BEFORE TRUSTING A NUMBER
//!
//! Every turn is a REAL turn through the REAL `Engine` trait: `submit("look")`
//! (or a splash-screen keypress where a game opens on one), with the resulting
//! `TurnResult` pushed into `AppState` through `push_transcript_kind` /
//! `push_transcript_runs` — the same two calls `turn.rs` makes for a live
//! player. This measures the cost of the WHOLE turn (VM execution + transcript
//! bookkeeping + render), not the renderer in isolation, and the numbers below
//! should be read that way: a growing `cold_ms` could in principle come from a
//! VM that gets slower with more turns rather than from the renderer. The
//! `idle_ms`/`keystroke_ms` columns rule that out — they hold the VM and the
//! transcript perfectly still and re-render only, which is the renderer alone.
//! (The alternative — pushing synthetic lines straight through
//! `AppState::push_transcript` with no VM behind them — was not used. It would
//! answer a narrower question, "how expensive is the wrap alone", and mixing
//! the two into one set of numbers is exactly what the quest says not to do.)
//!
//! `look` was chosen because it is a no-op in every fixture below: it never
//! moves, fights, or spends a consumable, so a growing turn count cannot end
//! the game by itself. If a game still quits early (a wandering monster, a
//! scripted end), the harness stops driving it, reports the turn it actually
//! reached, and marks every larger checkpoint SKIPPED — it never fabricates a
//! measurement for a turn count a game did not reach.
//!
//! ## The three things measured at every checkpoint
//!
//! * `cold_ms` — the render immediately after the batch of turns that grew the
//!   transcript to this checkpoint. This is "a frame that follows a mutation"
//!   in the quest's words.
//! * `idle_ms` (mean/max over `--repeats`) — `render_story_pane` called again
//!   `--repeats` times with NOTHING changed at all: same model, same
//!   `AppState`, same buffer. If a render is O(visible rows) once nothing is
//!   moving, this should be small and flat regardless of transcript size.
//! * `keystroke_ms` (mean/max over `--repeats`) — the same repeat, except each
//!   call is preceded by toggling one character of `AppState::input.value`
//!   (simulating the player typing) with the TRANSCRIPT untouched. This is the
//!   one that answers SQ-1033's specific question: `TranscriptWrapKey`
//!   (`state.rs`) does not include the input line, so a keystroke should be a
//!   cache HIT on any path that consults it — cheap regardless of scrollback
//!   length. `v6_raster_gen` (`screen.rs`), the coarse gate the raster canvas
//!   sits behind, DOES hash `state.input.value` — so a keystroke forces a full
//!   canvas rebuild there, and `build_main_text` re-wraps the ENTIRE transcript
//!   with no cache of its own. If `keystroke_ms` tracks transcript size while
//!   `idle_ms` stays flat, that gap IS the mechanism, measured rather than read.
//!
//! `mem_kb` is a cheap lower-bound estimate (string bytes + parallel-vector
//! element counts × `size_of`, not a real allocator sample) of the five
//! parallel transcript vectors (`transcript`, `_styles`, `_runs`, `_para`,
//! `_images`) — enough to see whether memory is also unbounded, not a profiler.
//!
//! ## Fixtures
//!
//! All five are freely-redistributable per `docs/internals/ci-fixture-coverage.md`, kept
//! locally in the gitignored `stories/`. Any fixture absent on this checkout is
//! skipped with a clear `SKIP:` line — never fabricated.

use std::path::Path;
use std::time::{Duration, Instant};

use app::config::V6RenderMode;
use app::engine::{Engine, KeyInput};
use app::glulx_session::GlulxSession;
use app::interpreter::InterpreterProfile;
use app::machine_boot::MachineBoot;
use app::render::graphics::kitty_picker;
use app::render::screen::render_story_pane;
use app::scott_session::ScottSession;
use app::session::{GameSession, InputKind};
use app::state::{AppState, TranscriptKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const DEFAULT_TURNS: &[usize] = &[100, 1_000, 5_000, 20_000];
const DEFAULT_REPEATS: usize = 8;
const AREA: Rect = Rect { x: 0, y: 0, width: 100, height: 40 };
const CELL_PX: (u16, u16) = (8, 16);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut turns = DEFAULT_TURNS.to_vec();
    let mut repeats = DEFAULT_REPEATS;
    let mut engines: Option<Vec<String>> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--turns" => {
                if let Some(v) = args.get(i + 1) {
                    let parsed: Vec<usize> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                    if !parsed.is_empty() {
                        turns = parsed;
                    }
                }
                i += 1;
            }
            "--repeats" => {
                if let Some(v) = args.get(i + 1) {
                    repeats = v.parse().unwrap_or(repeats);
                }
                i += 1;
            }
            "--engines" => {
                if let Some(v) = args.get(i + 1) {
                    engines = Some(v.split(',').map(|s| s.trim().to_string()).collect());
                }
                i += 1;
            }
            other => eprintln!("scroll_bench: ignoring `{other}`"),
        }
        i += 1;
    }
    turns.sort_unstable();
    turns.dedup();

    let profile = if cfg!(debug_assertions) { "DEBUG" } else { "release" };
    println!("scroll_bench — SQ-1033 instrument  ·  build profile: {profile}");
    if cfg!(debug_assertions) {
        eprintln!(
            "\n  *** WARNING: this is a DEBUG build. Timings below are meaningless for the \
             shipped binary. Re-run with `cargo run --release -p lanthorn --example scroll_bench`. ***\n"
        );
    }
    println!("turns sweep: {turns:?}  ·  repeats: {repeats}\n");

    let want = |name: &str| engines.as_ref().is_none_or(|e| e.iter().any(|x| x == name));

    let stories = stories_dir();

    if want("zvm-cell") {
        run_zvm_cell(&stories.join("anchor.z8"), &turns, repeats);
    }
    if want("zvm-raster") || want("zvm-hybrid") {
        run_zvm_v6(&stories.join("advent.z6"), &turns, repeats, &want);
    }
    if want("gvm") {
        run_glulx(&stories.join("Kerkerkruip.gblorb"), &turns, repeats);
    }
    if want("scott") {
        run_scott(&stories.join("golden_baton.blb"), &turns, repeats);
    }
}

/// `stories/` relative to the workspace root, same convention every other
/// example/test in this repo uses.
fn stories_dir() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates/app
    p.pop(); // crates
    p.push("stories");
    p
}

// ── shared plumbing ─────────────────────────────────────────────────────────

/// Advance one turn on any [`Engine`], tolerating a splash screen that opens on
/// a CHAR read (a `[more]`/"hit any key" style prompt) before settling into the
/// game's normal LINE loop. `cmd` is submitted on every LINE read.
fn advance(engine: &mut dyn Engine, cmd: &str) -> app::session::TurnResult {
    match engine.pending_input() {
        InputKind::Line => engine.submit(cmd),
        InputKind::Event => engine.submit(""),
        InputKind::Char => engine
            .submit_key(KeyInput::Enter)
            .or_else(|| engine.submit_key(KeyInput::Char(' ')))
            .unwrap_or_else(|| engine.submit(cmd)),
    }
}

/// Push one turn's result into `state`, mirroring `turn.rs`'s two calls
/// (`push_transcript_kind` for the echoed command, `push_transcript_runs` for
/// the game's own output) without the mapper/pager/event bookkeeping that
/// surrounds them in the real app loop — irrelevant to render cost.
fn push_turn(state: &mut AppState, cmd: &str, r: &app::session::TurnResult) {
    state.push_transcript_kind(&format!("> {cmd}"), TranscriptKind::Input);
    if !r.transcript.is_empty() {
        state.push_transcript_runs(&r.transcript, TranscriptKind::Story, &r.transcript_runs);
    }
}

/// Cheap lower-bound estimate of the five parallel transcript vectors' bytes:
/// string content + element counts × `size_of`. Not an allocator sample — see
/// the module doc.
fn transcript_mem_estimate(state: &AppState) -> usize {
    let text: usize = state.transcript.iter().map(String::len).sum();
    let kinds = state.transcript_kinds.len() * std::mem::size_of::<TranscriptKind>();
    let styles = state.transcript_styles.len() * std::mem::size_of::<Option<ratatui::style::Style>>();
    let runs: usize = state
        .transcript_runs
        .iter()
        .map(|v| std::mem::size_of::<Vec<app::state::StyleRun>>() + v.len() * std::mem::size_of::<app::state::StyleRun>())
        .sum();
    let para = state.transcript_para.len() * std::mem::size_of::<app::state::ParaFmt>();
    let images = state.transcript_images.len() * std::mem::size_of::<Option<app::inline_image::InlineImage>>();
    text + kinds + styles + runs + para + images
}

struct Stat {
    mean_ms: f64,
    max_ms: f64,
}
fn stat(samples: &[Duration]) -> Stat {
    let ms: Vec<f64> = samples.iter().map(Duration::as_secs_f64).map(|s| s * 1000.0).collect();
    let mean = ms.iter().sum::<f64>() / ms.len().max(1) as f64;
    let max = ms.iter().cloned().fold(0.0, f64::max);
    Stat { mean_ms: mean, max_ms: max }
}

/// Time `render_story_pane` once (`cold`), then `repeats` more times with
/// nothing at all changed (`idle`), then `repeats` more times toggling one
/// character of the live input line between calls with the TRANSCRIPT
/// untouched (`keystroke`).
///
/// The raw-wrap probe used to live here too; it is [`probe_raw_wrap`] now,
/// because it has to run after everything else a checkpoint times.
///
/// Returns `(cold_ms, idle Stat, keystroke Stat, StoryPaneMetrics from the cold
/// call)`.
#[allow(clippy::type_complexity)]
fn measure(
    model: &app::engine::ScreenModel,
    state: &mut AppState,
    repeats: usize,
) -> (f64, Stat, Stat, app::render::screen::StoryPaneMetrics) {
    // The v6 raster path backgrounds its picture ENCODE (`spawn_v6_encode`) and
    // relies on the host tick calling `AppState::poll_v6_encode_job` once per
    // loop before the draw (`main.rs`) to reap it — without that poll,
    // `v6_wants_build`'s `self.v6_job.is_some()` clause stays permanently true
    // after the first build and every later frame is silently skipped, no
    // matter how much the transcript has grown since. This harness is not the
    // main loop, so it has to do that poll itself, in the same order, or its
    // numbers would describe a bug in the HARNESS rather than in the renderer.
    // Bounded wait (not a spin-forever) so a still-in-flight encode from the
    // previous checkpoint gets a real chance to land before `cold` is timed —
    // a no-op on every non-raster config, where no job is ever spawned.
    let deadline = Instant::now() + Duration::from_millis(50);
    while Instant::now() < deadline {
        if state.poll_v6_encode_job() {
            break;
        }
        std::thread::sleep(Duration::from_micros(200));
    }
    let mut buf = Buffer::empty(AREA);
    let t0 = Instant::now();
    let cold_metrics = render_story_pane(model, false, None, state, AREA, &mut buf);
    let cold_ms = t0.elapsed().as_secs_f64() * 1000.0;

    let mut idle_samples = Vec::with_capacity(repeats);
    for _ in 0..repeats {
        state.poll_v6_encode_job();
        let t = Instant::now();
        std::hint::black_box(render_story_pane(model, false, None, state, AREA, &mut buf));
        idle_samples.push(t.elapsed());
    }

    let base_input = state.input.value.clone();
    let mut keystroke_samples = Vec::with_capacity(repeats);
    for k in 0..repeats {
        // Alternate appending/removing one char — never touches the transcript.
        if k % 2 == 0 {
            state.input.value.push('x');
        } else {
            state.input.value.pop();
        }
        state.poll_v6_encode_job();
        let t = Instant::now();
        std::hint::black_box(render_story_pane(model, false, None, state, AREA, &mut buf));
        keystroke_samples.push(t.elapsed());
    }
    state.input.value = base_input;
    if std::env::var("SCROLL_BENCH_DEBUG").is_ok() {
        eprintln!(
            "    [debug] idle samples (ms): {:?}",
            idle_samples.iter().map(|d| d.as_secs_f64() * 1000.0).collect::<Vec<_>>()
        );
        eprintln!(
            "    [debug] keystroke samples (ms): {:?}",
            keystroke_samples.iter().map(|d| d.as_secs_f64() * 1000.0).collect::<Vec<_>>()
        );
    }

    (cold_ms, stat(&idle_samples), stat(&keystroke_samples), cold_metrics)
}

/// Call `build_main_text` — the raster wrap builder itself — twice in a row with
/// nothing changed between the two calls, bypassing `render_story_pane`'s
/// whole-canvas `v6_wants_build` gate entirely. That gate is coarse (it skips the
/// ENTIRE raster rebuild, not just the wrap) and can mask the question SQ-1033
/// asks: does `build_main_text` have a cache of its own?
///
/// Before SQ-1034 it did not, and the two calls cost the same, both scaling with
/// transcript size, on every path. After it, the SECOND call is the answer — it
/// must be flat regardless of scrollback.
///
/// **Run this LAST in a checkpoint.** It deliberately wraps at its own width,
/// which is not the width any frame uses, so it leaves the wrap cache keyed to a
/// width the next frame will have to rebuild from. Run before `measure_one_turn`
/// it charges that rebuild to the turn and reports the harness's own footprint as
/// a render cost — which it did, at 29.276 ms against a true 0.169 ms.
fn probe_raw_wrap(state: &AppState) -> (f64, f64) {
    let wrap_cols = AREA.width.saturating_sub(2);
    let wrap_rows = AREA.height.saturating_sub(3);
    let t0 = Instant::now();
    let raw1 = std::hint::black_box(app::render::screen::build_main_text(state, wrap_cols, wrap_rows));
    let raw_wrap_1st_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let t1 = Instant::now();
    let raw2 = std::hint::black_box(app::render::screen::build_main_text(state, wrap_cols, wrap_rows));
    let raw_wrap_2nd_ms = t1.elapsed().as_secs_f64() * 1000.0;
    drop((raw1, raw2));
    (raw_wrap_1st_ms, raw_wrap_2nd_ms)
}

fn print_header(title: &str) {
    println!("=== {title} ===");
    println!(
        "{:>7}  {:>9}  {:>10}  {:>8}  {:>9}  {:>18}  {:>18}  {:>8}  {:>19}",
        "turns", "txn_lines", "total_rows", "mem_kb", "cold_ms", "idle mean/max ms", "keystroke mean/max ms",
        "turn_ms", "raw_wrap 1st/2nd ms"
    );
}
#[allow(clippy::too_many_arguments)]
fn print_row(
    turns: usize,
    lines: usize,
    total_rows: u16,
    mem_kb: f64,
    cold: f64,
    idle: &Stat,
    key: &Stat,
    turn: Option<f64>,
    raw1: f64,
    raw2: f64,
) {
    let turn = turn.map(|t| format!("{t:.3}")).unwrap_or_else(|| "-".to_string());
    println!(
        "{:>7}  {:>9}  {:>10}  {:>8.1}  {:>9.3}  {:>8.3}/{:<8.3}  {:>8.3}/{:<8.3}  {:>8}  {:>8.3}/{:<8.3}",
        turns, lines, total_rows, mem_kb, cold, idle.mean_ms, idle.max_ms, key.mean_ms, key.max_ms, turn, raw1, raw2
    );
}

// ── zvm, non-v6 (cell path) ─────────────────────────────────────────────────

fn run_zvm_cell(path: &Path, turns: &[usize], repeats: usize) {
    print_header(&format!("zvm, cell/terminal path — {}", path.display()));
    let Ok(bytes) = std::fs::read(path) else {
        println!("  SKIP: fixture not found ({})\n", path.display());
        return;
    };
    let mut engine: Box<dyn Engine> = match GameSession::new(bytes, true, false, None) {
        Ok(s) => Box::new(s),
        Err(e) => {
            println!("  SKIP: boot failed: {e:?}\n");
            return;
        }
    };
    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.config.honor_game_colours = true;

    drive_and_measure(&mut *engine, &mut state, turns, repeats);
    println!();
}

// ── zvm, v6 (raster + hybrid) ───────────────────────────────────────────────

fn run_zvm_v6(path: &Path, turns: &[usize], repeats: usize, want: &dyn Fn(&str) -> bool) {
    if want("zvm-raster") {
        run_zvm_v6_mode(path, turns, repeats, V6RenderMode::Raster, "RASTER");
    }
    if want("zvm-hybrid") {
        run_zvm_v6_mode(path, turns, repeats, V6RenderMode::Hybrid, "HYBRID");
    }
}

/// Boots its own session (v6 render modes are cheap enough to boot twice, and
/// this avoids needing `AppState: Clone`, which it isn't — its `RefCell`
/// caches and `game_picker` don't derive it, and there is no reason they
/// should for a live app).
fn run_zvm_v6_mode(path: &Path, turns: &[usize], repeats: usize, mode: V6RenderMode, mode_name: &str) {
    let title = format!("zvm, v6 {mode_name} — {}", path.display());
    let Ok(bytes) = std::fs::read(path) else {
        println!("=== {title} ===\n  SKIP: fixture not found\n");
        return;
    };
    if bytes.first() != Some(&6) {
        println!("=== {title} ===\n  SKIP: not a v6 story\n");
        return;
    }
    zvm::screen::set_palette(InterpreterProfile::IbmPc.palette());
    let boot = MachineBoot::bare();
    let mut engine = match GameSession::new_for_machine(bytes, true, false, false, Vec::new(), None, None, &boot) {
        Ok(s) => s,
        Err(e) => {
            println!("=== {title} ===\n  SKIP: boot failed: {e:?}\n");
            return;
        }
    };
    let _ = engine.take_transcript();

    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.config.honor_game_colours = true;
    state.config.v6_render = mode;
    state.v6_text = boot.text_face();
    state.game_picker = Some(kitty_picker(CELL_PX.0, CELL_PX.1));

    print_header(&title);
    drive_and_measure(&mut engine, &mut state, turns, repeats);
    println!();
}

// ── glulx ────────────────────────────────────────────────────────────────────

fn run_glulx(path: &Path, turns: &[usize], repeats: usize) {
    print_header(&format!("gvm (Glulx), cell/terminal path — {}", path.display()));
    let Ok(bytes) = std::fs::read(path) else {
        println!("  SKIP: fixture not found ({})\n", path.display());
        return;
    };
    let loaded = match app::hints::load_story(path) {
        Ok(l) => l,
        Err(e) => {
            println!("  SKIP: could not classify story: {e}\n");
            return;
        }
    };
    let image = match loaded {
        app::hints::LoadedStory::Glulx(b) => b,
        _ => bytes,
    };
    let mut engine: Box<dyn Engine> = match GlulxSession::new(image, 100, 40, true, false, false, (8, 16), None, &[]) {
        Ok(s) => Box::new(s),
        Err(e) => {
            println!("  SKIP: boot failed: {e:?}\n");
            return;
        }
    };
    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.config.honor_game_colours = true;

    drive_and_measure(&mut *engine, &mut state, turns, repeats);
    println!();
}

// ── scott ────────────────────────────────────────────────────────────────────

fn run_scott(path: &Path, turns: &[usize], repeats: usize) {
    print_header(&format!("scott, cell/terminal path — {}", path.display()));
    let Ok(bytes) = std::fs::read(path) else {
        println!("  SKIP: fixture not found ({})\n", path.display());
        return;
    };
    let loaded = match app::hints::load_story(path) {
        Ok(l) => l,
        Err(e) => {
            println!("  SKIP: could not classify story: {e}\n");
            return;
        }
    };
    let bytes = match loaded {
        app::hints::LoadedStory::Scott(b) => b,
        _ => bytes,
    };
    let mut engine: Box<dyn Engine> = match ScottSession::new(bytes, None) {
        Ok(s) => Box::new(s),
        Err(e) => {
            println!("  SKIP: boot failed: {e}\n");
            return;
        }
    };
    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.config.honor_game_colours = true;

    drive_and_measure(&mut *engine, &mut state, turns, repeats);
    println!();
}

// ── shared drive+measure loop for the non-v6 (single-mode) engines ─────────

fn drive_n(engine: &mut dyn Engine, state: &mut AppState, n: usize) -> usize {
    let mut done = 0;
    for _ in 0..n {
        if engine.has_quit() {
            break;
        }
        let r = advance(engine, "look");
        push_turn(state, "look", &r);
        done += 1;
    }
    done
}

fn drive_and_measure(engine: &mut dyn Engine, state: &mut AppState, turns: &[usize], repeats: usize) {
    let _ = engine.take_transcript();
    let mut turns_done = 0usize;
    for &t in turns {
        if turns_done < t && !engine.has_quit() {
            turns_done += drive_n(engine, state, t - turns_done);
        }
        let quit = engine.has_quit();
        let lines = state.transcript.len();
        let mem_kb = transcript_mem_estimate(state) as f64 / 1024.0;
        let model = engine.screen();
        let (cold, idle, key, m) = measure(&model, state, repeats);
        let turn = (!quit).then(|| measure_one_turn(engine, state, &mut turns_done));
        // Last, and see `probe_raw_wrap`: it leaves the wrap cache at a width no
        // frame uses, so anything timed after it pays for a rebuild it caused.
        let (raw1, raw2) = probe_raw_wrap(state);
        print_row(turns_done, lines, m.total_rows, mem_kb, cold, &idle, &key, turn, raw1, raw2);
        if quit {
            println!("  (game quit at turn {turns_done}; larger checkpoints skipped)");
            break;
        }
    }
}

/// One REAL turn at this scrollback depth, then one frame — and time the frame.
///
/// This is the number a player feels, and it is not `cold_ms` (SQ-1034).
/// `cold_ms` is the frame after the whole BATCH that reached a checkpoint —
/// 15,000 turns between the 5,000 and 20,000 rows — so it is dominated by lines
/// that arrived while nobody was looking and grows with the batch however
/// incremental the wrap is. A live app renders once per turn, so what has to stop
/// growing with total scrollback is THIS.
///
/// `turns_done` is advanced so the next checkpoint drives one turn fewer and the
/// reported turn counts stay true.
fn measure_one_turn(engine: &mut dyn Engine, state: &mut AppState, turns_done: &mut usize) -> f64 {
    let mut buf = Buffer::empty(AREA);
    *turns_done += drive_n(engine, state, 1);
    let model = engine.screen();
    state.poll_v6_encode_job();
    let t = Instant::now();
    std::hint::black_box(render_story_pane(&model, false, None, state, AREA, &mut buf));
    t.elapsed().as_secs_f64() * 1000.0
}
