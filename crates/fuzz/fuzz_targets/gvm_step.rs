//! Coverage-guided fuzzing of the Glulx step loop (SQ-1407). The fuzzer's raw
//! bytes go straight in as the story image — see `zvm_step.rs` for why no
//! synthetic header is built here.

#![no_main]

use gvm::exec::{Machine, StepResult};
use gvm::glk::TestBackend;
use gvm::memory::Memory;
use libfuzzer_sys::fuzz_target;

const MAX_STEPS: usize = 2_000;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() || data.len() > 256 * 1024 {
        return;
    }
    let Ok(mem) = Memory::new(data.to_vec()) else {
        return;
    };
    let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
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
            StepResult::NeedLine { .. } => {
                let n = (next_byte() % 12) as usize;
                let s: String = (0..n).map(|_| (32 + (next_byte() % 95)) as char).collect();
                m.supply_line(&s);
            }
            StepResult::NeedChar { unicode, .. } => {
                let code = if unicode { u32::from(next_byte()) * 4099 } else { u32::from(next_byte()) };
                m.supply_char(code);
            }
            StepResult::SaveRequest => m.complete_save(next_byte() % 2 == 0),
            StepResult::RestoreRequest => {
                if next_byte() % 2 == 0 {
                    m.complete_restore_failure();
                } else {
                    let _ = m.complete_restore_success(data);
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
                    break;
                }
            }
            // StepResult is #[non_exhaustive] (an embedder-facing API); this
            // target has no reason to distinguish a future variant.
            _ => break,
        }
    }
});
