//! Repeatable timing harness for the Glulx core (SQ-1428).
//!
//! Boots a story with output thrown away and drives a scripted command loop
//! for a fixed number of turns, timing the whole thing with
//! [`std::time::Instant`]. Beyond the standard library it uses only
//! `lanthorn-blorb` (already a dev-dependency of this crate, and zero-external-
//! dependency itself) to unwrap a `.gblorb`; a bare `.ulx` needs nothing.
//!
//! ```text
//! cargo run --release -p lanthorn-gvm --example bench -- \
//!     crates/gvm-cli/tests/fixtures/glulxercise.ulx \
//!     crates/gvm/tests/fixtures/bench/glulxercise.script --turns 2700
//! ```
//!
//! **Build `--release`.** A debug build measures the un-inlined, bounds-
//! checked shape of the dispatch loop and is several times slower; a debug
//! number is not a slow interpreter, it is a meaningless one.
//!
//! Recorded baselines, the machine they were taken on and the matching
//! `glulxe` commands live in `docs/internals/performance.md`; the story and
//! script provenance is in `crates/gvm/tests/fixtures/bench/README.md`.
//!
//! Unlike zvm's, this harness can report OPCODES per second as well as turns
//! per second, because [`Machine::insn_count`] already exists. Note its own
//! caveat: accelerated functions bypass the dispatcher, so with acceleration
//! on (the default) the opcode count undercounts the work done — the wall
//! clock is the honest number and the rate is a cross-check.
//!
//! ## Options
//!
//! * `--turns N` — how many commands to feed; the script loops from the start
//!   when it is shorter. Defaults to the script's own length (one pass).
//! * `--repeat N` — run it all `N` times and report the FASTEST (default 3).
//! * `--no-accel` — turn accelerated functions off, which is the shape the
//!   opcode counter measures honestly and roughly what an unaccelerated
//!   reference build does.

use std::any::Any;
use std::env;
use std::fs;
use std::process;
use std::time::{Duration, Instant};

use gvm::{GlkBackend, GlkStyle, Machine, Memory, StepResult};

/// Discards every `put_text`, counting the bytes so a run can be shown to
/// have done the same work as its reference — a benchmark that silently
/// stopped producing output would otherwise look like a speedup.
#[derive(Default)]
struct NullBackend {
    bytes: u64,
}

impl GlkBackend for NullBackend {
    fn put_text(&mut self, _win: u32, _style: GlkStyle, s: &str) {
        self.bytes += s.len() as u64;
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// One timed run's result.
struct Run {
    elapsed: Duration,
    turns: u64,
    bytes: u64,
    insns: u64,
}

/// Read a script file into one command per line, dropping blank lines and
/// `#` comments — so the same file becomes reference-interpreter input via
/// `grep -v '^#' | grep -v '^$'` and stays turn-for-turn identical.
fn load_script(path: &str) -> Vec<String> {
    let text = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("bench: cannot read script {path}: {e}");
        process::exit(1);
    });
    let lines: Vec<String> = text
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .map(str::to_owned)
        .collect();
    if lines.is_empty() {
        eprintln!("bench: script {path} has no commands");
        process::exit(1);
    }
    lines
}

/// Unwrap a `.gblorb`'s Glulx executable chunk; a bare `.ulx` passes through.
fn image_of(bytes: Vec<u8>, path: &str) -> Vec<u8> {
    if !blorb::Blorb::is_blorb(&bytes) {
        return bytes;
    }
    let archive = blorb::Blorb::parse(bytes).unwrap_or_else(|e| {
        eprintln!("bench: invalid Blorb archive: {e:?}");
        process::exit(1);
    });
    let (kind, exec) = archive.executable().unwrap_or_else(|e| {
        eprintln!("bench: no executable chunk in archive: {e:?}");
        process::exit(1);
    });
    if kind != blorb::ExecKind::Glulx {
        eprintln!("bench: {path} is not a Glulx story");
        process::exit(1);
    }
    exec.to_vec()
}

/// Boot `image` and feed it `turns` commands taken cyclically from `script`,
/// timing load, boot and play together — the same span a reference
/// interpreter's process wall clock covers.
fn run(image: &[u8], script: &[String], turns: u64, accel: bool) -> Run {
    let start = Instant::now();

    let mem = Memory::new(image.to_vec()).unwrap_or_else(|e| {
        eprintln!("bench: not a valid Glulx image: {e:?}");
        process::exit(1);
    });
    let mut m = Machine::with_glk(mem, Box::new(NullBackend::default()));
    m.set_acceleration(accel);

    let mut fed = 0u64;
    loop {
        match m.step() {
            StepResult::Continue => {}
            StepResult::NeedLine { .. } => {
                if fed == turns {
                    break;
                }
                m.supply_line(&script[(fed % script.len() as u64) as usize]);
                fed += 1;
            }
            // "Press any key": answer without spending a turn.
            StepResult::NeedChar { .. } => m.supply_char(u32::from(b' ')),
            // Nowhere to write and nothing to read — a benchmark must not
            // touch the filesystem, or it measures the disk.
            StepResult::SaveRequest => m.complete_save(false),
            StepResult::RestoreRequest => m.complete_restore_failure(),
            StepResult::NeedFilename { .. } => m.supply_filename(None),
            StepResult::Quit => break,
            StepResult::Fault => {
                eprintln!(
                    "bench: story faulted after {fed} turns: {:?}",
                    m.take_fault_trace()
                );
                process::exit(1);
            }
            // A timer/mouse/hyperlink wait this harness cannot answer, plus
            // any future #[non_exhaustive] variant.
            _ => break,
        }
    }

    let elapsed = start.elapsed();
    let insns = m.insn_count();
    let bytes = m
        .backend_mut()
        .as_any()
        .downcast_ref::<NullBackend>()
        .map_or(0, |b| b.bytes);
    Run {
        elapsed,
        turns: fed,
        bytes,
        insns,
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut positional = Vec::new();
    let mut repeat = 3u32;
    let mut turns: Option<u64> = None;
    let mut accel = true;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--no-accel" => accel = false,
            "--repeat" | "--turns" => {
                let flag = args[i].clone();
                i += 1;
                let v: u64 = args.get(i).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
                    eprintln!("bench: {flag} needs a positive number");
                    process::exit(2);
                });
                if flag == "--repeat" {
                    repeat = v.max(1) as u32;
                } else {
                    turns = Some(v);
                }
            }
            other => positional.push(other.to_owned()),
        }
        i += 1;
    }
    if positional.len() != 2 {
        eprintln!(
            "usage: bench <story.ulx|.gblorb> <script-file> [--turns N] [--repeat N] [--no-accel]"
        );
        process::exit(2);
    }

    let bytes = fs::read(&positional[0]).unwrap_or_else(|e| {
        eprintln!("bench: cannot read {}: {e}", positional[0]);
        process::exit(1);
    });
    let image = image_of(bytes, &positional[0]);
    let script = load_script(&positional[1]);
    let turns = turns.unwrap_or(script.len() as u64);

    let mut best: Option<Run> = None;
    for _ in 0..repeat {
        let r = run(&image, &script, turns, accel);
        if best.as_ref().is_none_or(|b| r.elapsed < b.elapsed) {
            best = Some(r);
        }
    }
    let best = best.expect("repeat is at least 1");

    let secs = best.elapsed.as_secs_f64();
    println!(
        "engine         gvm (lanthorn-gvm){}",
        if accel { "" } else { ", acceleration OFF" }
    );
    println!("story          {}", positional[0]);
    println!(
        "script         {} ({} commands)",
        positional[1],
        script.len()
    );
    println!("turns          {}", best.turns);
    println!("repeats        {repeat} (best shown)");
    println!("wall clock     {secs:.3} s");
    println!(
        "per turn       {:.1} us",
        secs * 1e6 / best.turns.max(1) as f64
    );
    println!(
        "turns/s        {:.0}",
        best.turns as f64 / secs.max(f64::MIN_POSITIVE)
    );
    println!("opcodes        {}", best.insns);
    println!(
        "opcodes/s      {:.0}",
        best.insns as f64 / secs.max(f64::MIN_POSITIVE)
    );
    println!("output bytes   {}", best.bytes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// The benchmark story. `glulxercise.ulx` lives once in the workspace, in
    /// `gvm-cli`'s fixtures, and every other `gvm` suite that wants it reaches
    /// across by this same relative path (`object_words.rs`, `grammar_tables.rs`,
    /// `disasm.rs`). A second copy under `gvm/tests/fixtures/bench/` would be
    /// 231 KB of binary that can silently drift from the one the conformance
    /// suites read.
    fn story_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../gvm-cli/tests/fixtures/glulxercise.ulx")
    }

    /// The script, which IS this crate's — it is the benchmark's definition,
    /// not a story anyone else uses.
    fn script_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bench/glulxercise.script")
    }

    /// Compiles and smoke-drives the harness so CI cannot let it rot — a few
    /// turns only, in whatever profile the gate happens to use. It asserts
    /// that turns were played and opcodes dispatched, NOT how long any of it
    /// took: a timing assertion in a test suite is a flake generator, and the
    /// real numbers are taken by hand per `docs/internals/performance.md`.
    #[test]
    fn bench_harness_drives_a_short_script() {
        let story_path = story_path();
        let script_path = script_path();
        let Ok(bytes) = fs::read(&story_path) else {
            eprintln!("skipping: {} absent", story_path.display());
            return;
        };
        if !script_path.exists() {
            eprintln!("skipping: {} absent", script_path.display());
            return;
        }
        let image = image_of(bytes, story_path.to_str().expect("utf-8 path"));
        let script = load_script(script_path.to_str().expect("utf-8 path"));
        assert!(!script.is_empty(), "script parsed to no commands");

        let r = run(&image, &script, 4, true);
        assert_eq!(r.turns, 4, "harness did not play the requested turns");
        assert!(r.bytes > 0, "harness produced no output at all");
        assert!(r.insns > 0, "harness dispatched no opcodes");
    }

    /// Every command in the script must be one glulxercise recognises and
    /// reports passing on. A typo'd test name still "runs" — the story prints
    /// its help text and the benchmark quietly measures string printing
    /// instead of the opcode group it was named for.
    #[test]
    fn script_runs_only_passing_tests() {
        let story_path = story_path();
        let script_path = script_path();
        let (Ok(bytes), true) = (fs::read(&story_path), script_path.exists()) else {
            eprintln!("skipping: {} absent", story_path.display());
            return;
        };
        let image = image_of(bytes, story_path.to_str().expect("utf-8 path"));
        let script = load_script(script_path.to_str().expect("utf-8 path"));

        // Re-run capturing text this time, rather than through `run`'s
        // byte-counting sink.
        #[derive(Default)]
        struct TextBackend(String);
        impl GlkBackend for TextBackend {
            fn put_text(&mut self, _win: u32, _style: GlkStyle, s: &str) {
                self.0.push_str(s);
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
            fn as_any_mut(&mut self) -> &mut dyn Any {
                self
            }
        }
        let mem = Memory::new(image).expect("valid Glulx image");
        let mut m = Machine::with_glk(mem, Box::new(TextBackend::default()));
        let mut fed = 0usize;
        loop {
            match m.step() {
                StepResult::Continue => {}
                StepResult::NeedLine { .. } => {
                    if fed == script.len() {
                        break;
                    }
                    m.supply_line(&script[fed]);
                    fed += 1;
                }
                StepResult::NeedChar { .. } => m.supply_char(u32::from(b' ')),
                StepResult::SaveRequest => m.complete_save(false),
                StepResult::RestoreRequest => m.complete_restore_failure(),
                StepResult::NeedFilename { .. } => m.supply_filename(None),
                _ => break,
            }
        }
        assert_eq!(fed, script.len(), "harness did not feed the whole script");

        let text = m
            .backend_mut()
            .as_any()
            .downcast_ref::<TextBackend>()
            .expect("TextBackend installed")
            .0
            .clone();
        let lower = text.to_ascii_lowercase();
        assert!(
            !lower.contains("failed") && !lower.contains("wrong"),
            "glulxercise reported a failure for the benchmark script"
        );
        // One "Passed." per command — that is what proves each line named a
        // real test rather than falling through to the help text.
        assert_eq!(
            text.matches("Passed.").count(),
            script.len(),
            "expected one 'Passed.' per script command"
        );
    }
}
