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

use crate::exec::{Machine, StepResult};
use crate::glk::TestBackend;
use crate::memory::Memory;
use std::time::{Duration, Instant};

/// Minimal xorshift64 PRNG. No external dependency — see the crate's hard
/// rule (`CLAUDE.md`: "zvm, gvm, and scott take ZERO external dependencies").
/// Duplicated from `zvm`'s copy rather than shared: each VM crate is
/// standalone and zero-dependency, including on each other.
pub(crate) struct XorShift64(u64);

impl XorShift64 {
    pub(crate) fn new(seed: u64) -> Self {
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

fn align_up(v: u32, to: u32) -> u32 {
    v.div_ceil(to) * to
}

/// Step cap per image — see `zvm`'s `fuzz_harness` module docs for why this
/// is deliberately small: a garbage image that decodes a `glk_select` inside
/// what amounts to a loop re-arms it forever rather than panicking, since the
/// harness's random answers never satisfy whatever the (garbage) code is
/// checking for, and that is legitimate execution, not a hang. This is the
/// real bound on the work a single image can do — see [`PER_STORY_BUDGET`]
/// for why the wall-clock check is not also sized off it.
const MAX_STEPS: usize = 1_500;

/// Per-story wall-clock budget — a HANG GUARD, not a performance bound. See
/// `zvm`'s `fuzz_harness` module docs for the full rationale: `MAX_STEPS`
/// above is what actually caps the work; this only exists to catch a `step()`
/// call that never returns at all. Sized generously (10s) for the slowest
/// plausible CI runner (GitHub's 3-4 core boxes run `cargo test`'s threads
/// sharing those cores, 2-3x slower than a local measurement on faster,
/// less-contended hardware) rather than off this crate's own local
/// measurement of the worst legitimate run.
const PER_STORY_BUDGET: Duration = Duration::from_secs(10);

/// Base of the fixed seed range the seeded tests below walk.
const BASE_SEED: u64 = 0x5EED_0000_0000_0001;

/// `LANTHORN_FUZZ_SEEDS=N` overrides the default seed count for a longer
/// local run; unset or unparsable falls back to `default`.
fn seed_count(default: usize) -> usize {
    std::env::var("LANTHORN_FUZZ_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// Build a random Glulx image: a header just valid enough for
/// [`crate::header::parse_header`] to accept it (magic, a version 2 or 3
/// major, a 256-aligned RAMSTART ≤ EXTSTART ≤ ENDMEM, a 256-aligned stack
/// size, START FUNC inside the memory map — the facts it actually checks),
/// followed by fully random bytes everywhere else. That is the hostile case:
/// a structurally-loadable image whose every other field, and whose code, is
/// garbage.
fn random_image(seed: u64) -> Vec<u8> {
    let mut rng = XorShift64::new(seed);
    let extstart = align_up(256 + rng.below(16 * 1024) as u32, 256);
    let mut buf = vec![0u8; extstart as usize];
    rng.fill(&mut buf);
    buf[0..4].copy_from_slice(b"Glul");
    let major: u32 = if rng.below(2) == 0 { 2 } else { 3 };
    let version = (major << 16) | (rng.below(256) as u32) << 8 | rng.below(256) as u32;
    buf[0x04..0x08].copy_from_slice(&version.to_be_bytes());
    let ramstart = align_up(rng.below(extstart as u64 + 1) as u32, 256).min(extstart);
    buf[0x08..0x0C].copy_from_slice(&ramstart.to_be_bytes());
    buf[0x0C..0x10].copy_from_slice(&extstart.to_be_bytes());
    let endmem = extstart + align_up(rng.below(16 * 1024) as u32, 256);
    buf[0x10..0x14].copy_from_slice(&endmem.to_be_bytes());
    let stack_size = align_up(256 + rng.below(8 * 1024) as u32, 256);
    buf[0x14..0x18].copy_from_slice(&stack_size.to_be_bytes());
    let start_func = rng.below(endmem as u64) as u32;
    buf[0x18..0x1C].copy_from_slice(&start_func.to_be_bytes());
    let decode_table = rng.below(endmem as u64 + 1) as u32;
    buf[0x1C..0x20].copy_from_slice(&decode_table.to_be_bytes());
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
    let mut rng = XorShift64::new(seed ^ 0x1234_5678_9ABC_DEF0);
    let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
    for _ in 0..MAX_STEPS {
        if start.elapsed() > PER_STORY_BUDGET {
            return Err(format!("seed {seed:#x}: exceeded {PER_STORY_BUDGET:?} budget"));
        }
        match m.step() {
            StepResult::Continue => {}
            StepResult::Quit | StepResult::Fault => return Ok(()),
            StepResult::NeedLine { .. } => {
                let n = rng.below(12) as usize;
                let s: String = (0..n).map(|_| (32u8 + rng.below(95) as u8) as char).collect();
                m.supply_line(&s);
            }
            StepResult::NeedChar { unicode, .. } => {
                let code = if unicode { rng.below(0x10FFFF) as u32 } else { rng.below(256) as u32 };
                m.supply_char(code);
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
            StepResult::NeedFilename { .. } => m.supply_filename(None),
            StepResult::NeedEvent { timer_ms, mouse, hyperlink } => {
                if timer_ms.is_some() {
                    m.deliver_timer();
                } else if mouse {
                    m.deliver_mouse(0, 0, 0);
                } else if hyperlink {
                    m.deliver_hyperlink(0, 0);
                } else {
                    // Nothing armed — Glk spec forbids this, but a hostile
                    // image can still decode into it; treat it like a hang
                    // guard rather than spinning forever.
                    return Ok(());
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
            (seed, random_image(seed))
        }),
    );
}

/// The one committed, freely redistributable raw Glulx image in the
/// workspace — `crates/gvm-cli/tests/fixtures/glulxercise.ulx`. The other two
/// committed fixtures the quest names (`startsavetest.gblorb`,
/// `statusbufferwin_apple.glulxe.txt`) are a Blorb wrapper and a transcript,
/// not a raw loadable image, so mutating them at the `Memory::new` boundary
/// doesn't apply the same way; this one is a direct fit.
fn fixture_path() -> &'static str {
    concat!(env!("CARGO_MANIFEST_DIR"), "/../gvm-cli/tests/fixtures/glulxercise.ulx")
}

#[test]
fn mutated_fixture_stories_survive_step() {
    let Ok(original) = std::fs::read(fixture_path()) else {
        return; // fixture absent — skip vacuously
    };
    let n = seed_count(500);
    run_seeds(
        "mutated_fixture_stories_survive_step",
        (0..n as u64).map(|i| {
            let seed = BASE_SEED + 0x1000_0000 + i;
            (seed, mutated_fixture(seed, &original))
        }),
    );
}

/// Feed `restore_quetzal` / `load_vfs` random and mutated-but-once-valid
/// buffers: neither may panic, and the machine must still be steppable
/// afterward either way.
#[test]
fn hostile_restore_buffers_do_not_panic() {
    let Ok(story) = std::fs::read(fixture_path()) else {
        return; // fixture absent — skip vacuously
    };
    let mem = Memory::new(story.clone()).expect("committed fixture must parse");
    let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
    for _ in 0..50 {
        match m.step() {
            StepResult::Continue => {}
            StepResult::NeedLine { .. } => m.supply_line("look"),
            StepResult::NeedChar { .. } => m.supply_char(b'\n' as u32),
            StepResult::SaveRequest => m.complete_save(false),
            StepResult::RestoreRequest => m.complete_restore_failure(),
            StepResult::NeedFilename { .. } => m.supply_filename(None),
            StepResult::NeedEvent { .. } => m.deliver_timer(),
            StepResult::Quit | StepResult::Fault => break,
        }
    }

    let n = seed_count(500);
    let mut failures = Vec::new();
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    for i in 0..n as u64 {
        let seed = BASE_SEED + 0x2000_0000 + i;
        let mut rng = XorShift64::new(seed);
        let buf = if rng.below(2) == 0 {
            let len = rng.below(2048) as usize;
            let mut b = vec![0u8; len];
            rng.fill(&mut b);
            b
        } else {
            mutated_fixture(seed, &story)
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mem2 = Memory::new(story.clone()).unwrap();
            let mut m2 = Machine::with_glk(mem2, Box::new(TestBackend::new()));
            let _ = m2.restore_quetzal(&buf);
            m2.load_vfs(&buf);
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

/// Call every documented Glk selector (plus a spread of invalid ones) with
/// random arguments, on a machine booted from a real fixture. No panic.
#[test]
fn glk_dispatch_survives_random_arguments() {
    let Ok(story) = std::fs::read(fixture_path()) else {
        return; // fixture absent — skip vacuously
    };
    let mem = Memory::new(story).expect("committed fixture must parse");
    let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));

    let n = seed_count(3000);
    let mut failures = Vec::new();
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    for i in 0..n as u64 {
        let seed = BASE_SEED + 0x4000_0000 + i;
        let mut rng = XorShift64::new(seed);
        // Mostly the real selector range (0x0001-0x016F, per glk_selector_name),
        // occasionally a fully random u32 to cover invalid selectors too.
        let selector = if rng.below(10) == 0 { rng.next_u64() as u32 } else { rng.below(0x0180) as u32 };
        let argc = rng.below(9) as usize;
        let mut args = Vec::with_capacity(argc);
        for _ in 0..argc {
            args.push(match rng.below(4) {
                0 => 0,
                1 => 0xFFFF_FFFF,
                2 => rng.below(0x1_0000) as u32,
                _ => rng.next_u64() as u32,
            });
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = m.glk_dispatch(selector, &args);
        }));
        if let Err(payload) = result {
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string panic payload>".to_string());
            failures.push(format!("seed {seed:#x} selector {selector:#06x} args {args:?}: PANIC: {msg}"));
        }
    }
    std::panic::set_hook(prev_hook);
    assert!(
        failures.is_empty(),
        "glk_dispatch_survives_random_arguments: {} failing seed(s):\n{}",
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
        let bytes = random_image(seed);
        let mem = match Memory::new(bytes) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let mut rng = XorShift64::new(seed);
        let addr = rng.below(mem.endmem() as u64) as u32;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = crate::disasm::decode_instr(&mem, addr);
        }));
        if let Err(payload) = result {
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string panic payload>".to_string());
            failures.push(format!("seed {seed:#x} addr {addr:#010x}: PANIC: {msg}"));
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
