// CZECH / Praxix regression acceptance gate — Task 16.
//
// Drives CZECH/Praxix headlessly: feed no input (they auto-run), collect all
// output, assert the suite reports success and zero failures.

use zvm::cpu::exec::{Machine, StepResult};
use zvm::memory::Memory;
use zvm::text::input::ZsciiInput;

/// Build a `Machine` with a buffer sink, call `init_caps()`, and run until
/// `Quit` (or the step limit).  Returns all captured output as a String.
///
/// CZECH/Praxix auto-run with no input, but we handle `NeedLine`/`NeedChar`
/// defensively (supply empty input) so the runner can't hang.
fn run_to_quit(story: Vec<u8>) -> String {
    run_with_input(story, &[])
}

/// Like `run_to_quit`, but feeds `inputs` in sequence on successive `NeedLine`
/// prompts (empty once exhausted). CZECH auto-runs with no input; Praxix needs
/// an explicit command ("all") before it runs anything.
fn run_with_input(story: Vec<u8>, inputs: &[&str]) -> String {
    let mem = Memory::new(story).expect("Memory::new failed");
    let mut machine = Machine::new(mem);
    machine.init_caps();

    let mut next = inputs.iter();
    const MAX_STEPS: u64 = 20_000_000;
    let mut fault: Option<String> = None;
    for _ in 0..MAX_STEPS {
        match machine.step() {
            StepResult::Quit => break,
            StepResult::Continue => {}
            StepResult::Restart => break, // shouldn't happen in CZECH
            StepResult::Fault => {
                // Record the fault so callers can assert the machine didn't halt.
                let t = machine.take_fault_trace();
                fault = Some(t.map(|t| t.fault).unwrap_or_else(|| "fault".into()));
                break;
            }
            StepResult::NeedLine { .. } => {
                machine.supply_line(next.next().copied().unwrap_or(""), 13);
            }
            StepResult::NeedChar => {
                machine.supply_char(ZsciiInput::NEWLINE);
            }
            StepResult::SaveRequest => {
                machine.complete_save(false);
            }
            StepResult::RestoreRequest => {
                machine.complete_restore_failure();
            }
            _ => break,
        }
    }

    // Extract captured output from the buffer sink.
    let mut out = machine
        .buffer_output()
        .map(|b| b.buf.clone())
        .unwrap_or_default();
    if let Some(f) = fault {
        out.push_str(&format!("\n[MACHINE-FAULT: {f}]\n"));
    }
    out
}

#[test]
fn czech_reports_no_failures() {
    let Some(story) = zvm::fixtures::load("czech.z5") else {
        // Skip if fixture absent.
        return;
    };
    let out = run_to_quit(story);
    // Print full output for debugging during development.
    println!("CZECH output:\n{out}");

    // Hard-coded section presence: all major sections must run.
    for section in &["Jumps", "Variables", "Arithmetic ops", "Logical ops",
                     "Memory", "Subroutines", "Objects", "Indirect Opcodes",
                     "Misc"] {
        assert!(
            out.contains(section),
            "CZECH missing section {section:?}:\n{out}"
        );
    }

    // Extract passed/failed counts from the CZECH summary line.
    // With the fixed A2 alphabet the line reads:
    //   "Passed: 406, Failed: 0, Print tests: 19"
    // (Previously the buggy A2 shifted ':' → '-' and ',' → '.', producing
    //  "Passed- 406. Failed- 0." — the parser now handles both forms.)
    fn parse_after(line: &str, prefix: &str) -> Option<u32> {
        let rest = line.split(prefix).nth(1)?;
        rest.split_whitespace()
            .next()?
            .trim_end_matches(|c: char| !c.is_ascii_digit())
            .parse()
            .ok()
    }
    let passed: u32 = out
        .lines()
        .find_map(|l| {
            let l = l.trim();
            if l.starts_with("Passed") {
                parse_after(l, "Passed:").or_else(|| parse_after(l, "Passed-"))
            } else {
                None
            }
        })
        .unwrap_or(0);
    let failed: u32 = out
        .lines()
        .find_map(|l| {
            let l = l.trim();
            if l.contains("Failed") {
                parse_after(l, "Failed:").or_else(|| parse_after(l, "Failed-"))
            } else {
                None
            }
        })
        .unwrap_or(u32::MAX);

    assert!(
        passed >= 406,
        "CZECH passed {passed} tests, expected >= 406:\n{out}"
    );
    assert!(
        failed == 0,
        "CZECH reported {failed} failure(s):\n{out}"
    );
}

#[test]
fn praxix_reports_no_failures() {
    let Some(story) = zvm::fixtures::load("praxix.z5") else {
        return; // skip if absent
    };
    // Praxix does NOT auto-run: it waits for a command and runs one group per
    // command. Drive the core opcode/undo/table groups, then quit.
    //
    // The following groups are intentionally NOT asserted here:
    //   - "spec11"/"spec12": these are VISUAL @set_true_colour swatch prints
    //     (coloured spaces, blank in a headless text capture) — not programmatic
    //     pass/fail. True colour IS implemented; nothing to assert here.
    let groups = [
        "operand", "arith", "comarith", "bitwise", "shift", "inc", "incchk",
        "array", "undo", "multiundo", "indirect", "throwcatch", "tables",
        "streamtrip", "streamop",
    ];
    let mut inputs = groups.to_vec();
    inputs.push("quit");
    let out = run_with_input(story, &inputs);
    println!("Praxix output:\n{out}");

    // 1. The machine must not have halted with a fault (guards the loadw/storew
    //    16-bit array-address wrapping — a regression there faults the "array"
    //    group at a huge out-of-bounds address).
    assert!(
        !out.contains("[MACHINE-FAULT"),
        "Praxix halted the interpreter with a fault:\n{out}"
    );
    // 2. Every driven group must run and report success — no failures/mismatches.
    for group_header in ["Basic operand values", "Array loads and stores",
                         "Undo", "Indirect opcodes"] {
        assert!(
            out.contains(group_header),
            "Praxix did not run the {group_header:?} group:\n{out}"
        );
    }
    let passed = out.lines().filter(|l| l.trim() == "Passed.").count();
    assert!(
        passed >= groups.len(),
        "Praxix: only {passed} groups reported Passed (expected >= {}):\n{out}",
        groups.len()
    );
    // Praxix marks a real failure with the uppercase token "FAIL" (e.g.
    // "should be 195 FAIL") or "Mismatch". (Its benign summary line
    // "failures are not counted twice" must NOT trip this.)
    assert!(
        !out.contains("FAIL") && !out.contains("Mismatch"),
        "Praxix reported a failure/mismatch in a core group:\n{out}"
    );
}

// strictz.z5 (SQ-1421) — every object-0 opcode edge case (ZMSD "objects are
// numbered consecutively from 1 upward, with object number 0 being used to
// mean 'nothing'"). strictz feeds `@jin`/`@get_child`/`@get_parent`/
// `@get_sibling`/`@get_prop_addr`/`@get_prop`/`@clear_attr`/`@set_attr`/
// `@test_attr`/`@insert_obj`/`@remove_obj`/`@get_next_prop` object 0 and
// flags each result "(correct)" or "(incorrect)"/"(wrong)". It asks one
// line-read question up front ("Would you like to make a transcript...
// (Y/N)") — answered N so nothing touches disk — then runs to completion,
// ending on one `@read_char` ("Press any key.") that dfrotz also answers
// with a single keystroke.
#[test]
fn strictz_reports_all_correct() {
    let Some(story) = zvm::fixtures::load("strictz.z5") else {
        // Skip if fixture absent.
        return;
    };
    let mem = Memory::new(story).expect("Memory::new failed");
    let mut machine = Machine::new(mem);
    machine.init_caps();

    const MAX_STEPS: u64 = 200_000;
    let mut answered_transcript_prompt = false;
    for _ in 0..MAX_STEPS {
        match machine.step() {
            StepResult::Quit => break,
            StepResult::Continue => {}
            StepResult::Restart => break,
            StepResult::Fault => {
                let t = machine.take_fault_trace();
                panic!(
                    "strictz.z5 faulted the interpreter: {:?}",
                    t.map(|t| t.fault)
                );
            }
            StepResult::NeedLine { .. } => {
                // Only one line-read in the whole run: "make a transcript?".
                assert!(
                    !answered_transcript_prompt,
                    "strictz.z5 asked a second line-read question — driving assumption stale"
                );
                answered_transcript_prompt = true;
                machine.supply_line("N", 13);
            }
            StepResult::NeedChar => {
                // The closing "Press any key." — any keystroke ends it.
                machine.supply_char(ZsciiInput::NEWLINE);
            }
            StepResult::SaveRequest => machine.complete_save(false),
            StepResult::RestoreRequest => machine.complete_restore_failure(),
            other => panic!("unexpected StepResult driving strictz.z5: {other:?}"),
        }
    }

    let out = machine
        .buffer_output()
        .map(|b| b.buf.clone())
        .unwrap_or_default();
    println!("strictz output:\n{out}");

    assert!(
        answered_transcript_prompt,
        "strictz.z5 never asked the transcript question — it did not run:\n{out}"
    );
    assert!(
        out.contains("Test completed!"),
        "strictz.z5 did not reach its completion banner:\n{out}"
    );

    // Every object-0 opcode section must run.
    for section in &[
        "Testing @jin", "Testing @get_child", "Testing @get_parent",
        "Testing @get_sibling", "Testing @get_prop_addr", "Testing @get_prop",
        "Testing @clear_attr", "Testing @set_attr", "Testing @test_attr",
        "Testing @insert_obj", "Testing ?remove_obj", "Testing ?get_next_prop",
    ] {
        assert!(
            out.contains(section),
            "strictz.z5 missing section {section:?}:\n{out}"
        );
    }

    // Every flagged result must read "(correct)"; none may read
    // "(incorrect)" or "(wrong)" (strictz's two spellings for a failure).
    assert!(
        !out.contains("(incorrect)") && !out.contains("(wrong)"),
        "strictz.z5 flagged an object-0 result as incorrect/wrong:\n{out}"
    );
    let correct_count = out.matches("(correct)").count();
    assert!(
        correct_count >= 28,
        "strictz.z5 only flagged {correct_count} results \"(correct)\" (expected >= 28):\n{out}"
    );
}
