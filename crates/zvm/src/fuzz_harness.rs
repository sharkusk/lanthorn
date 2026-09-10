//! In-crate hostile-input fuzzing harness (SQ-1407).
//!
//! This is the REGRESSION GUARD half of the fuzzing story: a hand-rolled
//! xorshift64 PRNG (zero dependencies, per the crate's hard rule) drives a
//! few thousand fixed-seed random and mutated story images through
//! [`Machine::step`], asserting no panic and no hang. It runs under
//! `cargo nextest`/`cargo test` and therefore on every CI push.
//!
//! The COVERAGE-GUIDED half — `cargo-fuzz` targets run by hand on a nightly
//! toolchain — lives in the separate `crates/fuzz` package (kept out of this
//! crate so the zero-dependency rule holds: `libfuzzer-sys` never touches
//! this `Cargo.toml`). See `docs/internals/fuzzing.md`. Any crash `cargo
//! fuzz` finds becomes a pinned `#[test]` in this file, next to the cases
//! below it already found.
//!
//! Seeds are fixed (`BASE_SEED..`) so a failure reproduces; set
//! `LANTHORN_FUZZ_SEEDS=N` to run more of them locally than the default.

use crate::cpu::exec::{Machine, StepResult};
use crate::memory::Memory;
use crate::text::ZsciiInput;
use std::time::{Duration, Instant};

/// Minimal xorshift64 PRNG. No external dependency — see the crate's hard
/// rule (`CLAUDE.md`: "zvm, gvm, and scott take ZERO external dependencies").
pub(crate) struct XorShift64(u64);

impl XorShift64 {
    pub(crate) fn new(seed: u64) -> Self {
        // xorshift64 never advances from state 0.
        XorShift64(if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed })
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A uniform value in `0..n` (0 if `n == 0`).
    pub(crate) fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next_u64() % n
        }
    }

    pub(crate) fn fill(&mut self, buf: &mut [u8]) {
        let mut i = 0;
        while i < buf.len() {
            let r = self.next_u64().to_le_bytes();
            let n = (buf.len() - i).min(8);
            buf[i..i + n].copy_from_slice(&r[..n]);
            i += n;
        }
    }
}

/// How many steps a single hostile image gets before the harness gives up on
/// it as a "normal" run (most either fault or quit far sooner). A random
/// image occasionally decodes a `read`/`read_char` in what amounts to a loop
/// that keeps re-arming it forever — a story ignoring random typed input and
/// asking again is legitimate (if pointless) Z-machine behaviour, not a bug,
/// and the harness's own random answers never type "quit" to end it. This is
/// the real bound on the work a single image can do — see
/// [`PER_STORY_BUDGET`] for why the wall-clock check is not also sized off
/// it. Investigated during this quest's own run: seed `0x5eed000000000234`
/// (version 8) ran the full step cap on a `NeedLine`/`NeedChar` loop that
/// never once returned `Continue`, at a measured ~280µs/step in an
/// unoptimized debug build — 5x the ~72µs/step of a mostly-`Continue` run
/// (seed `0x5eed000000000074`, version 4).
const MAX_STEPS: usize = 1_500;

/// Per-story wall-clock budget — a HANG GUARD, not a performance bound.
/// `MAX_STEPS` above is what actually caps the work a single image can do;
/// this only exists to catch a genuine hang (a single `step()` call that
/// never returns at all, e.g. an unbounded loop inside one opcode handler —
/// the pre-SQ-1395 shape). It is deliberately generous: this crate's own
/// local measurement put the worst *legitimate* full-`MAX_STEPS` run at
/// ~403ms on an M2 Max, but CI runs on 3-4 core GitHub runners with the
/// binary's tests sharing those cores as threads (`cargo test`, not
/// `nextest`'s one-process-per-test), where the same run can easily take
/// 2-3x longer — a budget sized off a fast local machine's measurement would
/// be a CI flake waiting to happen. 10s has ample headroom over any plausible
/// CI slowdown of that ~403ms figure while still catching an actual hang
/// (which would either never return at all, or balloon by orders of
/// magnitude, not merely by a constant multiplier).
const PER_STORY_BUDGET: Duration = Duration::from_secs(10);

/// Base of the fixed seed range `random_image_stories_survive_step` and
/// `mutated_fixture_stories_survive_step` walk.
const BASE_SEED: u64 = 0x5EED_0000_0000_0001;

/// `LANTHORN_FUZZ_SEEDS=N` overrides the default seed count for a longer
/// local run; unset or unparsable falls back to `default`.
fn seed_count(default: usize) -> usize {
    std::env::var("LANTHORN_FUZZ_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// Build a random story image: a header just valid enough for [`Memory::new`]
/// to accept it (version 3-8, `static_mem_base` inside the buffer — the only
/// two facts it actually checks), followed by fully random bytes everywhere
/// else, including the rest of the header. That is the hostile case: a
/// structurally-loadable image whose every other field, and whose code, is
/// garbage.
fn random_image(seed: u64, version: u8) -> Vec<u8> {
    let mut rng = XorShift64::new(seed);
    let len = 4096 + rng.below(60_000) as usize;
    let mut buf = vec![0u8; len];
    rng.fill(&mut buf);
    buf[0x00] = version;
    let static_mem_base = rng.below(len as u64 + 1) as u16;
    buf[0x0E] = (static_mem_base >> 8) as u8;
    buf[0x0F] = (static_mem_base & 0xFF) as u8;
    buf
}

/// A committed fixture with random damage: a run of byte flips, a random
/// truncation, or a random block splice.
fn mutated_fixture(seed: u64, original: &[u8]) -> Vec<u8> {
    let mut rng = XorShift64::new(seed);
    let mut buf = original.to_vec();
    if buf.is_empty() {
        return buf;
    }
    match rng.below(3) {
        0 => {
            let flips = 1 + rng.below(64) as usize;
            for _ in 0..flips {
                let idx = rng.below(buf.len() as u64) as usize;
                buf[idx] ^= (1 + rng.below(255)) as u8;
            }
        }
        1 => {
            let cut = 1 + rng.below(buf.len() as u64) as usize;
            buf.truncate(cut);
        }
        _ => {
            let start = rng.below(buf.len() as u64) as usize;
            let splice_len = 1 + rng.below((buf.len() - start).max(1) as u64) as usize;
            let mut junk = vec![0u8; splice_len];
            rng.fill(&mut junk);
            buf[start..start + splice_len].copy_from_slice(&junk);
        }
    }
    buf
}

/// Drive one story image through `step()`, answering every suspend point
/// with random input, until `Quit`/`Fault`, the step cap, or the wall-clock
/// budget. `Err` means a hang (budget exceeded); everything else — including
/// a rejected header — is `Ok`.
fn drive(bytes: Vec<u8>, seed: u64) -> Result<(), String> {
    let mem = match Memory::new(bytes) {
        Ok(m) => m,
        Err(_) => return Ok(()), // a rejected header is a correct outcome, not a bug
    };
    let start = Instant::now();
    let mut rng = XorShift64::new(seed ^ 0xA5A5_A5A5_A5A5_A5A5);
    let mut m = Machine::new(mem);
    for _ in 0..MAX_STEPS {
        if start.elapsed() > PER_STORY_BUDGET {
            return Err(format!("seed {seed:#x}: exceeded {PER_STORY_BUDGET:?} budget"));
        }
        match m.step() {
            StepResult::Continue => {}
            StepResult::Quit | StepResult::Fault => return Ok(()),
            StepResult::Restart => m.restart(),
            StepResult::NeedLine { .. } => {
                let n = rng.below(12) as usize;
                let s: String = (0..n).map(|_| (32u8 + rng.below(95) as u8) as char).collect();
                m.supply_line(&s, 13);
            }
            StepResult::NeedChar => {
                let code = rng.below(256) as u8;
                let z = ZsciiInput::new(code).unwrap_or(ZsciiInput::NEWLINE);
                m.supply_char(z);
            }
            StepResult::SaveRequest => m.complete_save(rng.below(2) == 0),
            StepResult::RestoreRequest => {
                if rng.below(2) == 0 {
                    m.complete_restore_failure();
                } else {
                    let mut junk = vec![0u8; 16 + rng.below(64) as usize];
                    rng.fill(&mut junk);
                    let _ = m.complete_restore_success(&junk);
                }
            }
        }
    }
    Ok(())
}

/// Run `drive` under `catch_unwind` so one bad seed doesn't stop the sweep —
/// every seed runs, and every failure (panic or hang) is collected before the
/// case fails once, listing every seed that found one.
fn run_seeds(label: &str, seeds: impl Iterator<Item = (u64, Vec<u8>)>) {
    let mut failures = Vec::new();
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {})); // one bad seed shouldn't spam stderr thousands of times
    for (seed, bytes) in seeds {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drive(bytes, seed)));
        match result {
            Ok(Ok(())) => {}
            Ok(Err(msg)) => failures.push(msg),
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "<non-string panic payload>".to_string());
                failures.push(format!("seed {seed:#x}: PANIC: {msg}"));
            }
        }
    }
    std::panic::set_hook(prev_hook);
    assert!(failures.is_empty(), "{label}: {} failing seed(s):\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn random_image_stories_survive_step() {
    let n = seed_count(1500);
    run_seeds(
        "random_image_stories_survive_step",
        (0..n as u64).map(|i| {
            let seed = BASE_SEED + i;
            let version = 3 + (i % 6) as u8;
            (seed, random_image(seed, version))
        }),
    );
}

/// Committed, freely redistributable fixtures to mutate — real story shapes
/// rather than pure noise, so mutation finds a different class of bug (a
/// plausible-but-corrupt header/table) than [`random_image`] does.
///
/// `minizork.z3` is loaded through [`crate::fixtures::load`] rather than a
/// bare path, because it moved to the fetched fixture set (SQ-1453) — `load`
/// also checks the app crate's fetched location for that one name.
fn fixture_bytes() -> Vec<Vec<u8>> {
    ["czech.z5", "minizork.z3", "curses.z5"]
        .iter()
        .filter_map(|name| crate::fixtures::load(name))
        .collect()
}

#[test]
fn mutated_fixture_stories_survive_step() {
    let fixtures: Vec<Vec<u8>> = fixture_bytes();
    if fixtures.is_empty() {
        return; // no committed fixture available — skip vacuously
    }
    let n = seed_count(500);
    run_seeds(
        "mutated_fixture_stories_survive_step",
        (0..n as u64).map(|i| {
            let seed = BASE_SEED + 0x1000_0000 + i;
            let original = &fixtures[(i as usize) % fixtures.len()];
            (seed, mutated_fixture(seed, original))
        }),
    );
}

/// Feed `restore_quetzal` / `restore_file` / `restore_screen_snapshot` random
/// and mutated-but-once-valid buffers: each must return `Err` or `Ok` without
/// panicking, and the machine must still be steppable afterward either way.
#[test]
fn hostile_restore_buffers_do_not_panic() {
    let fixtures = fixture_bytes();
    let Some(story) = fixtures.into_iter().next() else {
        return; // fixture absent — skip vacuously
    };
    let mem = Memory::new(story.clone()).expect("committed fixture must parse");
    let mut m = Machine::new(mem);
    // Play a few turns so save_quetzal/screen encode something non-trivial.
    for _ in 0..50 {
        match m.step() {
            StepResult::Continue => {}
            StepResult::NeedLine { .. } => m.supply_line("look", 13),
            StepResult::NeedChar => m.supply_char(ZsciiInput::NEWLINE),
            StepResult::SaveRequest => m.complete_save(false),
            StepResult::RestoreRequest => m.complete_restore_failure(),
            StepResult::Restart => m.restart(),
            StepResult::Quit | StepResult::Fault => break,
        }
    }
    let valid_quetzal = m.save_quetzal();
    let valid_screen = crate::screen_snapshot::encode(&m.screen);

    let n = seed_count(500);
    let mut failures = Vec::new();
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    for i in 0..n as u64 {
        let seed = BASE_SEED + 0x2000_0000 + i;
        let mut rng = XorShift64::new(seed);
        let buf = match rng.below(2) {
            0 => {
                let len = rng.below(2048) as usize;
                let mut b = vec![0u8; len];
                rng.fill(&mut b);
                b
            }
            _ => {
                let base = if rng.below(2) == 0 { &valid_quetzal } else { &valid_screen };
                mutated_fixture(seed, base)
            }
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mem2 = Memory::new(story.clone()).unwrap();
            let mut m2 = Machine::new(mem2);
            let _ = m2.restore_quetzal(&buf);
            let _ = m2.restore_file(&buf);
            let _ = m2.restore_screen_snapshot(&buf);
            // Whatever happened, the machine must still be steppable.
            let _ = m2.step();
        }));
        if let Err(payload) = result {
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string panic payload>".to_string());
            failures.push(format!("seed {seed:#x}: PANIC: {msg}"));
        }
    }
    std::panic::set_hook(prev_hook);
    assert!(
        failures.is_empty(),
        "hostile_restore_buffers_do_not_panic: {} failing seed(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Random addresses on random images must never panic the disassembler.
#[test]
fn hostile_disassembly_does_not_panic() {
    let n = seed_count(500);
    let mut failures = Vec::new();
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    for i in 0..n as u64 {
        let seed = BASE_SEED + 0x3000_0000 + i;
        let version = 3 + (i % 6) as u8;
        let bytes = random_image(seed, version);
        let mem = match Memory::new(bytes) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let mut rng = XorShift64::new(seed);
        let addr = rng.below(mem.len() as u64) as u32;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = crate::cpu::disasm::disassemble(&mem, addr, version, 20);
            let _ = crate::cpu::disasm::disassemble_raw(&mem, addr, version, 20);
        }));
        if let Err(payload) = result {
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string panic payload>".to_string());
            failures.push(format!("seed {seed:#x} addr {addr:#06x}: PANIC: {msg}"));
        }
    }
    std::panic::set_hook(prev_hook);
    assert!(
        failures.is_empty(),
        "hostile_disassembly_does_not_panic: {} failing seed(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

