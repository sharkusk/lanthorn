//! Repeatable timing harness for the Z-machine core (SQ-1428).
//!
//! Boots a story with output thrown away and drives a scripted command loop
//! for a fixed number of turns, timing the whole thing with
//! [`std::time::Instant`]. No dependencies beyond the standard library and no
//! sampling profiler — it answers one question only: *how long does this
//! machine take to play N turns of this story?*, in a form anybody can rerun
//! and compare against a reference interpreter driven with the same script.
//!
//! ```text
//! cargo run --release -p lanthorn-zvm --example bench -- \
//!     crates/zvm/tests/fixtures/minizork.z3 \
//!     crates/zvm/tests/fixtures/bench/minizork.script --turns 20000
//! ```
//!
//! **Build `--release`.** A debug build measures the borrow-checked, bounds-
//! checked, un-inlined shape of the interpreter and is three to twenty times
//! slower; a debug number is not a slow interpreter, it is a meaningless one.
//!
//! Recorded baselines, the machine they were taken on and the matching
//! `dfrotz` commands live in `docs/internals/performance.md`; the script's own
//! provenance is in `crates/zvm/tests/fixtures/bench/README.md`.
//!
//! ## Options
//!
//! * `--turns N` — how many commands to feed. The script is looped from the
//!   start when it is shorter than `N`, so a script whose commands return the
//!   game to where they found it can be run for any length. Defaults to the
//!   script's own length (one pass).
//! * `--repeat N` — run the whole thing `N` times and report the FASTEST.
//!   Defaults to 3. The best of several runs is the number least polluted by
//!   whatever else the machine was doing; a mean would fold that noise in.

use std::any::Any;
use std::env;
use std::fs;
use std::process;
use std::time::{Duration, Instant};

use zvm::cpu::exec::{BootConfig, Machine, StepResult};
use zvm::io::Output;
use zvm::memory::Memory;

/// Discards everything the story prints, counting the bytes so a run can be
/// shown to have done the same work as its reference (a benchmark that
/// silently stopped producing output would otherwise look like a speedup).
/// The count is the only reason this is not `fn print(&mut self, _: &str) {}`.
#[derive(Default)]
struct NullSink {
    bytes: u64,
}

impl Output for NullSink {
    fn print(&mut self, s: &str) {
        self.bytes += s.len() as u64;
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// ZSCII 13 — the terminator an ordinary typed line ends with (ZMSD §3.8).
const ENTER: u8 = 13;

/// One timed run's result.
struct Run {
    elapsed: Duration,
    turns: u64,
    bytes: u64,
}

/// Read a script file into one command per line, dropping blank lines and
/// `#` comments. Blank lines are dropped rather than passed through as empty
/// commands so the same file can be turned into reference-interpreter input
/// by `grep -v '^#' | grep -v '^$'` and stay turn-for-turn identical.
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

/// Boot `story` and feed it `turns` commands taken cyclically from `script`,
/// timing load, boot and play together — the same span a reference
/// interpreter's process wall clock covers.
fn run(story: &[u8], script: &[String], turns: u64) -> Run {
    let start = Instant::now();

    let mem = Memory::new(story.to_vec()).unwrap_or_else(|e| {
        eprintln!("bench: invalid story file: {e:?}");
        process::exit(1);
    });
    let mut m = Machine::boot(mem, Box::new(NullSink::default()), BootConfig::new());

    let mut fed = 0u64;
    loop {
        match m.step() {
            StepResult::Continue => {}
            StepResult::NeedLine { .. } => {
                if fed == turns {
                    break;
                }
                m.supply_line(&script[(fed % script.len() as u64) as usize], ENTER);
                fed += 1;
            }
            // "Press any key" and the like: answer without spending a turn.
            StepResult::NeedChar => m.supply_char(ENTER),
            // Nowhere to write and nothing to read — a benchmark must not
            // touch the filesystem, or it measures the disk.
            StepResult::SaveRequest => m.complete_save(false),
            StepResult::RestoreRequest => m.complete_restore_failure(),
            StepResult::Quit => break,
            StepResult::Restart => m.restart(),
            StepResult::Fault => {
                eprintln!(
                    "bench: story faulted after {fed} turns: {:?}",
                    m.take_fault_trace()
                );
                process::exit(1);
            }
            // StepResult is #[non_exhaustive].
            _ => break,
        }
    }

    let elapsed = start.elapsed();
    let bytes = m
        .output()
        .as_any()
        .downcast_ref::<NullSink>()
        .map_or(0, |s| s.bytes);
    Run {
        elapsed,
        turns: fed,
        bytes,
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut positional = Vec::new();
    let mut repeat = 3u32;
    let mut turns: Option<u64> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
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
        eprintln!("usage: bench <story-file> <script-file> [--turns N] [--repeat N]");
        process::exit(2);
    }

    let story = fs::read(&positional[0]).unwrap_or_else(|e| {
        eprintln!("bench: cannot read {}: {e}", positional[0]);
        process::exit(1);
    });
    let script = load_script(&positional[1]);
    let turns = turns.unwrap_or(script.len() as u64);

    let mut best: Option<Run> = None;
    for _ in 0..repeat {
        let r = run(&story, &script, turns);
        if best.as_ref().is_none_or(|b| r.elapsed < b.elapsed) {
            best = Some(r);
        }
    }
    let best = best.expect("repeat is at least 1");

    let secs = best.elapsed.as_secs_f64();
    println!("engine         zvm (lanthorn-zvm)");
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
    println!("output bytes   {}", best.bytes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
    }

    /// Compiles and smoke-drives the harness so CI cannot let it rot — a few
    /// turns only, in whatever profile the gate happens to use. It asserts
    /// that turns were played and output produced, NOT how long any of it
    /// took: a timing assertion in a test suite is a flake generator, and the
    /// real numbers are taken by hand per `docs/internals/performance.md`.
    #[test]
    fn bench_harness_drives_a_short_script() {
        let story_path = fixtures().join("minizork.z3");
        let script_path = fixtures().join("bench/minizork.script");
        let Ok(story) = fs::read(&story_path) else {
            eprintln!("skipping: {} absent", story_path.display());
            return;
        };
        if !script_path.exists() {
            eprintln!("skipping: {} absent", script_path.display());
            return;
        }
        let script = load_script(script_path.to_str().expect("utf-8 path"));
        assert!(!script.is_empty(), "script parsed to no commands");

        let r = run(&story, &script, 16);
        assert_eq!(r.turns, 16, "harness did not play the requested turns");
        assert!(r.bytes > 0, "harness produced no output at all");
    }

    /// The script must be a CYCLE: looping it has to leave the game where it
    /// started, or a long run measures a different game every lap. Two laps
    /// end in the same room as one, which is the cheapest statement of that
    /// property the cell-free harness can make.
    #[test]
    fn script_is_a_closed_cycle() {
        let story_path = fixtures().join("minizork.z3");
        let script_path = fixtures().join("bench/minizork.script");
        let (Ok(story), true) = (fs::read(&story_path), script_path.exists()) else {
            eprintln!("skipping: {} absent", story_path.display());
            return;
        };
        let script = load_script(script_path.to_str().expect("utf-8 path"));
        let n = script.len() as u64;

        // Play one lap, then two, capturing what the last turn printed each
        // time. `inventory` closes the script, so both end on the same reply.
        let one = run(&story, &script, n);
        let two = run(&story, &script, n * 2);
        assert_eq!(one.turns, n);
        assert_eq!(two.turns, n * 2);
        // A cycle prints (very nearly) twice as much in two laps as in one.
        // The first lap carries the boot banner, so allow the shortfall.
        assert!(
            two.bytes > one.bytes && two.bytes < one.bytes * 2,
            "two laps printed {} bytes against one lap's {} — the script is not a cycle",
            two.bytes,
            one.bytes
        );
    }
}
