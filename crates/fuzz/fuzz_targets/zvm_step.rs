//! Coverage-guided fuzzing of the Z-machine step loop (SQ-1407).
//!
//! Unlike the in-crate harness (`crates/zvm/src/fuzz_harness.rs`), this feeds
//! the fuzzer's raw bytes straight in as the story image — libFuzzer's
//! coverage guidance explores toward whatever byte patterns reach new code,
//! so no synthetic "valid-ish header" construction is needed here.

#![no_main]

use libfuzzer_sys::fuzz_target;
use zvm::cpu::exec::{Machine, StepResult};
use zvm::memory::Memory;
use zvm::text::ZsciiInput;

/// Small — libFuzzer calls this thousands of times a second; the in-crate
/// harness is where step-count depth is explored.
const MAX_STEPS: usize = 2_000;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() || data.len() > 256 * 1024 {
        return;
    }
    let Ok(mem) = Memory::new(data.to_vec()) else {
        return;
    };
    let mut m = Machine::new(mem);
    // A deterministic cursor over the fuzzer's own bytes answers every
    // suspend point — no separate PRNG needed; libFuzzer's mutations already
    // vary these bytes across runs.
    let mut cursor = 0usize;
    let mut next_byte = || {
        let b = data[cursor % data.len()];
        cursor = cursor.wrapping_add(1);
        b
    };
    for _ in 0..MAX_STEPS {
        match m.step() {
            StepResult::Continue => {}
            StepResult::Quit | StepResult::Fault => break,
            StepResult::Restart => m.restart(),
            StepResult::NeedLine { .. } => {
                let n = (next_byte() % 12) as usize;
                let s: String = (0..n).map(|_| (32 + (next_byte() % 95)) as char).collect();
                m.supply_line(&s, 13);
            }
            StepResult::NeedChar => {
                let code = next_byte();
                let z = ZsciiInput::new(code).unwrap_or(ZsciiInput::NEWLINE);
                m.supply_char(z);
            }
            StepResult::SaveRequest => m.complete_save(next_byte() % 2 == 0),
            StepResult::RestoreRequest => {
                if next_byte() % 2 == 0 {
                    m.complete_restore_failure();
                } else {
                    let _ = m.complete_restore_success(data);
                }
            }
            // StepResult is #[non_exhaustive] (an embedder-facing API); this
            // target has no reason to distinguish a future variant.
            _ => break,
        }
    }
});
