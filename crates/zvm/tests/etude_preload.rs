// TerpEtude option 12 ("Pre-loading of input line") — SQ-1419 proof that
// ZMSD §15 `read`'s pre-loaded input line is implemented.
//
// TerpEtude (Andrew Plotkin, Release 2) is a Z-machine interpreter exerciser
// distributed via the IF Archive. `givenin.inc`'s `TestGivenInput` prints its
// own "Preload> Given" line, THEN sets its own read buffer's byte 1 to 5 and
// bytes 2-6 to "Given" before issuing `@aread` — a genuine game-supplied
// pre-load, exactly like Beyond Zork/Zork Zero/Shogun's "AGAIN" (a function
// key reopens the read with the previous command already in the buffer).
// dfrotz answers a bare Enter at that prompt with `You just typed "given".`
// (lower-cased — ZMSD §15 lower-cases the whole line, pre-load included);
// before this fix zvm answered `You just typed a blank line.` because
// `Machine::supply_line` discarded whatever `read`'s byte 1 already held.
//
// Skips vacuously when `etude.z5` is absent from the fixture dir (see
// `crates/zvm/tests/fixtures/README.md`).

use zvm::cpu::exec::{Machine, StepResult};
use zvm::memory::Memory;
use zvm::text::input::ZsciiInput;

/// Step until the next `NeedLine`/`NeedChar`/terminal `StepResult`, capturing
/// every byte printed along the way. Panics on `Fault` (a real defect, not a
/// thing this test should silently tolerate) and on a step-count runaway.
fn run_to_next_line_prompt(machine: &mut Machine) -> String {
    const MAX_STEPS: u64 = 2_000_000;
    for _ in 0..MAX_STEPS {
        match machine.step() {
            StepResult::NeedLine { .. } => {
                return machine
                    .buffer_output()
                    .map(|b| b.buf.clone())
                    .unwrap_or_default();
            }
            StepResult::NeedChar => {
                // TerpEtude's menu is line-driven; a char prompt here would be
                // an unrelated test section — answer Enter and keep going.
                machine.supply_char(ZsciiInput::NEWLINE);
            }
            StepResult::Continue => {}
            StepResult::Fault => {
                let trace = machine.take_fault_trace();
                panic!(
                    "unexpected machine fault while driving TerpEtude: {:?}",
                    trace.map(|t| t.fault)
                );
            }
            other => panic!("unexpected StepResult driving TerpEtude: {other:?}"),
        }
    }
    panic!("TerpEtude did not reach a line prompt within {MAX_STEPS} steps");
}

#[test]
fn terpetude_option_12_preload_matches_dfrotz() {
    let Some(story) = zvm::fixtures::load("etude.z5") else {
        // Skip if the fixture is absent (fetch-on-demand, see fixtures/README.md).
        return;
    };
    let mem = Memory::new(story).expect("etude.z5: Memory::new failed");
    let mut machine = Machine::new(mem);
    machine.init_caps();

    // Drive to TerpEtude's main-menu prompt (its own `Version()` banner
    // prints unconditionally before the first read).
    let banner = run_to_next_line_prompt(&mut machine);
    assert!(
        banner.contains("TerpEtude") && banner.contains("Options:"),
        "expected the TerpEtude banner + menu before the first prompt:\n{banner}"
    );

    // Select option 12 ("Pre-loading of input line") — a plain line answer,
    // no pre-load in force yet.
    machine.supply_line("12", 13);
    let intro = run_to_next_line_prompt(&mut machine);
    assert!(
        intro.contains("Preload> Given"),
        "TestGivenInput must print its own \"Preload> Given\" line before the read:\n{intro}"
    );

    // THE proof: the game pre-loaded its buffer with "Given" (byte 1 = 5)
    // before this `@aread` — a host that reads `StepResult::NeedLine`'s
    // `preload` field would see exactly that. Answer with nothing further
    // typed (an immediate Enter, the same "accept the default" gesture the
    // task's TerpEtude proof exercises) and check the game's own verdict.
    machine.supply_line("", 13);
    let verdict = run_to_next_line_prompt(&mut machine);
    assert!(
        verdict.contains("You just typed \"given\"."),
        "expected TerpEtude's verdict to match dfrotz's \
         (`You just typed \"given\".`), got:\n{verdict}"
    );
    assert!(
        !verdict.contains("You just typed a blank line."),
        "the pre-loaded \"Given\" must not be discarded as a blank line:\n{verdict}"
    );
}
