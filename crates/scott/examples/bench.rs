//! Repeatable timing harness for the Scott Adams core (SQ-1428).
//!
//! Loads a `.dat`, then drives a scripted command loop for a fixed number of
//! turns with output thrown away, timing the whole thing with
//! [`std::time::Instant`]. No dependencies beyond the standard library.
//!
//! ```text
//! cargo run --release -p lanthorn-scott --example bench -- \
//!     crates/scott/tests/tiny_cave.dat \
//!     crates/scott/tests/fixtures/bench/tiny_cave.script --turns 100000
//! ```
//!
//! **Build `--release`.** A debug build measures the un-inlined, bounds-
//! checked shape of the occurrence/action loop and is several times slower;
//! a debug number is not a slow interpreter, it is a meaningless one.
//!
//! Scott Adams games are two orders of magnitude smaller than a Z-machine or
//! Glulx story, so the turn counts here are correspondingly larger — at a few
//! thousand turns the whole run is under a millisecond and the number is
//! clock granularity rather than interpreter speed.
//!
//! Every turn asks for [`Vm::room_block`] as well as stepping, because that is
//! what ScottFree itself does between prompts: leaving it out would compare a
//! room description the reference formats against one this harness never
//! builds.
//!
//! Recorded baselines, the machine they were taken on and the matching
//! ScottFree commands live in `docs/internals/performance.md`; the script's
//! provenance is in `crates/scott/tests/fixtures/bench/README.md`.
//!
//! ## Options
//!
//! * `--turns N` — how many commands to feed; the script loops from the start
//!   when it is shorter. Defaults to the script's own length (one pass).
//! * `--repeat N` — run it all `N` times and report the FASTEST (default 3).

use std::env;
use std::fs;
use std::process;
use std::time::{Duration, Instant};

use scott::{Database, StepResult, Vm};

/// One timed run's result.
struct Run {
    elapsed: Duration,
    turns: u64,
    bytes: u64,
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

/// Parse `src` and feed the resulting game `turns` commands taken cyclically
/// from `script`, timing parse and play together — the same span a reference
/// interpreter's process wall clock covers.
fn run(src: &str, script: &[String], turns: u64) -> Run {
    let start = Instant::now();

    let db = Database::parse(src).unwrap_or_else(|e| {
        eprintln!("bench: invalid .dat: {e:?}");
        process::exit(1);
    });
    let mut vm = Vm::new(db);
    let mut bytes = vm.take_output().len() as u64;

    let mut fed = 0u64;
    while fed < turns {
        // What ScottFree prints before each prompt; measured, then dropped.
        bytes += vm.room_block().len() as u64;
        vm.supply_line(&script[(fed % script.len() as u64) as usize]);
        let done = matches!(vm.step(), StepResult::Quit);
        bytes += vm.take_output().len() as u64;
        fed += 1;
        if done {
            break;
        }
    }

    Run {
        elapsed: start.elapsed(),
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
        eprintln!("usage: bench <story.dat> <script-file> [--turns N] [--repeat N]");
        process::exit(2);
    }

    let src = fs::read_to_string(&positional[0]).unwrap_or_else(|e| {
        eprintln!("bench: cannot read {}: {e}", positional[0]);
        process::exit(1);
    });
    if !scott::looks_like_scott(&src) {
        eprintln!(
            "bench: {} does not look like a Scott Adams .dat",
            positional[0]
        );
        process::exit(1);
    }
    let script = load_script(&positional[1]);
    let turns = turns.unwrap_or(script.len() as u64);

    let mut best: Option<Run> = None;
    for _ in 0..repeat {
        let r = run(&src, &script, turns);
        if best.as_ref().is_none_or(|b| r.elapsed < b.elapsed) {
            best = Some(r);
        }
    }
    let best = best.expect("repeat is at least 1");

    let secs = best.elapsed.as_secs_f64();
    println!("engine         scott (lanthorn-scott)");
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
        "per turn       {:.2} us",
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

    fn crate_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    /// Compiles and smoke-drives the harness so CI cannot let it rot — a few
    /// turns only, in whatever profile the gate happens to use. It asserts
    /// that turns were played and output produced, NOT how long any of it
    /// took: a timing assertion in a test suite is a flake generator, and the
    /// real numbers are taken by hand per `docs/internals/performance.md`.
    #[test]
    fn bench_harness_drives_a_short_script() {
        let story_path = crate_dir().join("tests/tiny_cave.dat");
        let script_path = crate_dir().join("tests/fixtures/bench/tiny_cave.script");
        let Ok(src) = fs::read_to_string(&story_path) else {
            eprintln!("skipping: {} absent", story_path.display());
            return;
        };
        if !script_path.exists() {
            eprintln!("skipping: {} absent", script_path.display());
            return;
        }
        let script = load_script(script_path.to_str().expect("utf-8 path"));
        assert!(!script.is_empty(), "script parsed to no commands");

        let r = run(&src, &script, 20);
        assert_eq!(r.turns, 20, "harness did not play the requested turns");
        assert!(r.bytes > 0, "harness produced no output at all");
    }

    /// The script must never end the game — `tiny_cave` is won by dropping the
    /// idol in the clearing, and a benchmark that wins on lap one measures a
    /// quit loop from then on. Ten laps' worth of turns must all be played.
    #[test]
    fn script_never_ends_the_game() {
        let story_path = crate_dir().join("tests/tiny_cave.dat");
        let script_path = crate_dir().join("tests/fixtures/bench/tiny_cave.script");
        let (Ok(src), true) = (fs::read_to_string(&story_path), script_path.exists()) else {
            eprintln!("skipping: {} absent", story_path.display());
            return;
        };
        let script = load_script(script_path.to_str().expect("utf-8 path"));
        let want = script.len() as u64 * 10;
        let r = run(&src, &script, want);
        assert_eq!(
            r.turns, want,
            "the game ended after {} of {want} turns — the script is not a cycle",
            r.turns
        );
    }
}
