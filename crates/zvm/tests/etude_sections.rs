// TerpEtude (etude.z5, SQ-1421) — the remaining 13 of its 14 menu options
// (option 12, "Pre-loading of input line", is `etude_preload.rs`'s own proof
// for SQ-1419 and is left there). TerpEtude's own Inform 6 source ships in
// `etude.tar.Z` (`etude.inf` + its `.inc` files) — read there, not guessed,
// to drive each option correctly:
//
//   1  Version                        6  Multiplication/division/remainder
//   2  Recent changes                 7  Accented character output
//   3  Header flags analysis          8  Single-key input
//   4  Styled text                    9  Full-line input
//   5  Colored text                  10  Timed single-key input
//                                    11  Timed full-line input
//                                    13  Undo capability
//                                    14  Printing before quitting
//
// The top-level menu (`etude.inf`'s `mainloop`) reads the option NUMBER as a
// whole LINE (`@aread`), unlike gntests.z5's single-digit `@read_char` menu —
// so selecting an option is `supply_line("<n>", 13)`, not `supply_char`.
//
// Options 1-9/13/14 are diffed (by content, not exact bytes — see the module
// doc's wrapping note) against `fixtures/etude.dfrotz.txt`, a real dfrotz
// transcript recorded from the SAME input script (see `fixtures/README.md`
// for the dfrotz version and exact command). **Normalisation**: dfrotz's
// dumb interface word-wraps every paragraph to its own terminal width
// (`-w 999`, so most lines fit, but a few of TerpEtude's longer paragraphs
// still wrap); `zvm`'s headless `BufferOutput` sink never does — it
// accumulates exactly the bytes the story printed, with no simulated
// terminal to wrap against. So parity is asserted with `.contains(...)` on
// whitespace-normalised fragments short enough not to straddle a dfrotz wrap
// point, never as a full-transcript `assert_eq!`.
//
// Options 10 and 11 (timed input) are NOT diffed against dfrotz: dfrotz's
// dumb interface does not implement real timed-read polling when stdin is
// piped (confirmed live — selecting option 10 under a piped script prints
// "The timing interrupt function was not called at all... This aspect of
// your interpreter appears to behave WRONG", dfrotz's own honest report that
// no real second elapsed). `zvm` exposes the timed-read protocol as an API
// instead (`Machine::pending_timeout` / `run_timed_interrupt` /
// `abort_timed_input`), simulating ticks deterministically without a real
// wall-clock wait — which is exactly what the SQ-1014 audit meant by
// "drive them with the VM's timed-read protocol, not wall clock". These two
// are asserted against TerpEtude's own Inform source instead (`timedch.inc`,
// `timedstr.inc`), which is unambiguous about what each tick prints.
//
// **A read must never be polled twice — and now `Machine::step()` guarantees
// it (SQ-1432).** This file is where that footgun was first hit: TestTimedString's
// own "You just typed a blank line" appeared after nothing was typed, because
// `step()` used to unconditionally decode and execute whatever was at
// `state.pc`, which the read opcode had already advanced PAST before
// suspending (so that a later `supply_line`/`supply_char` resumes correctly).
// Calling `step()` again before supplying anything executed the INSTRUCTION
// AFTER THE READ as though the read had silently completed with blank/default
// content. `step()` now guards against exactly that: while a read (or a game
// `@save`/`@restore`) is pending it returns the SAME `StepResult` again,
// touching neither the PC nor any buffer, and the only ways forward are
// `supply_line`/`supply_char`/`abort_timed_input` (reads) or `complete_save`/
// `complete_restore_success`/`complete_restore_failure` (save/restore) — see
// `Machine::step`'s own doc comment. `run_timed_interrupt()` is always safe to
// call in a loop (it pushes and steps its own call frame, isolated from the
// outer pending read, via the same nested-call door the guard steps aside
// for) — just never follow it with a bare `step()` until the read is actually
// completed.

use zvm::cpu::exec::{Machine, StepResult};
use zvm::memory::Memory;

fn boot() -> Option<Machine> {
    let story = zvm::fixtures::load("etude.z5")?;
    let mem = Memory::new(story).expect("etude.z5: Memory::new failed");
    let mut machine = Machine::new(mem);
    machine.init_caps();
    Some(machine)
}

/// Step to the next `NeedLine`/`NeedChar`/`Quit`, treating a `Fault` as a
/// hard test failure. Never called while a read is already pending (see the
/// module doc) — every call site here supplies input before stepping again.
fn next_prompt(machine: &mut Machine) -> StepResult {
    const MAX_STEPS: u64 = 2_000_000;
    for _ in 0..MAX_STEPS {
        match machine.step() {
            StepResult::Continue => {}
            StepResult::Fault => {
                let t = machine.take_fault_trace();
                panic!("etude.z5 faulted the interpreter: {:?}", t.map(|t| t.fault));
            }
            StepResult::SaveRequest => machine.complete_save(false),
            StepResult::RestoreRequest => machine.complete_restore_failure(),
            other => return other,
        }
    }
    panic!("etude.z5 did not reach a prompt within {MAX_STEPS} steps");
}

fn buf(machine: &Machine) -> String {
    machine.buffer_output().map(|b| b.buf.clone()).unwrap_or_default()
}

fn assert_no_error_markers(out: &str, option: &str) {
    assert!(
        !out.contains("ERROR") && !out.contains("appears to behave WRONG"),
        "TerpEtude option {option} reported an error:\n{out}"
    );
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Confirm `fragment` (whitespace-normalised, so dfrotz's own `-w 999`
/// line-wrapping in `etude.dfrotz.txt` can't desync a literal match) appears
/// in the checked-in dfrotz reference transcript (options 1-9/13/14 --
/// options 10/11 are deliberately absent from it, see `fixtures/README.md`).
/// This is what makes the golden LOAD-BEARING at test time rather than a
/// static document nothing actually reads: corrupt a byte of
/// `etude.dfrotz.txt` inside one of the fragments checked here and a test
/// fails, naming the fragment and the file.
fn assert_in_dfrotz_golden(fragment: &str) {
    let Some(bytes) = zvm::fixtures::load("etude.dfrotz.txt") else {
        return; // vacuous skip, matching every other fixture in this crate
    };
    let golden = String::from_utf8(bytes).expect("fixtures/etude.dfrotz.txt must be UTF-8");
    assert!(
        normalize_ws(&golden).contains(&normalize_ws(fragment)),
        "fragment {fragment:?} not found in fixtures/etude.dfrotz.txt (the dfrotz reference transcript) -- parity broken"
    );
}

/// Boot and drive straight to the top-menu's first "> " prompt (past the
/// banner), returning the machine positioned to select an option.
fn boot_to_menu() -> Option<Machine> {
    let mut machine = boot()?;
    assert!(
        matches!(next_prompt(&mut machine), StepResult::NeedLine { .. }),
        "expected the top-menu NeedLine after the banner"
    );
    Some(machine)
}

#[test]
fn etude_noninteractive_options_report_ok() {
    // Options 1-6 need no interactive read of their own — each returns
    // straight to the top menu's next NeedLine. Driven in one session
    // (matching the dfrotz transcript's own script) so option 1's
    // ("Version") banner and option 2's ("Recent changes") text can be
    // told apart from the boot banner by position.
    let Some(mut machine) = boot_to_menu() else { return };

    let mut sections = Vec::new();
    for opt in 1..=6 {
        let before = buf(&machine).len();
        machine.supply_line(&opt.to_string(), 13);
        assert!(
            matches!(next_prompt(&mut machine), StepResult::NeedLine { .. }),
            "option {opt} should return straight to the top menu"
        );
        let full = buf(&machine);
        sections.push(full[before..].to_string());
    }
    machine.supply_line(".", 13);
    assert_eq!(next_prompt(&mut machine), StepResult::Quit);

    for (i, out) in sections.iter().enumerate() {
        assert_no_error_markers(out, &format!("{}", i + 1));
    }

    // Option 1 (Version): re-prints the same banner boot did.
    assert!(sections[0].contains("TerpEtude: A Z-machine Interpreter Exerciser"));
    assert!(sections[0].contains("Tests compliance with Z-Machine Standards Document 0.99."));
    assert_in_dfrotz_golden("TerpEtude: A Z-machine Interpreter Exerciser");

    // Option 2 (Recent changes / History).
    assert!(sections[1].contains("In the beginning, TerpEtude was written."));
    assert!(sections[1].contains("Spec Aid: Graham, SJ, PDD, and the rest of the crowd"));
    assert_in_dfrotz_golden("In the beginning, TerpEtude was written.");
    assert_in_dfrotz_golden("Spec Aid: Graham, SJ, PDD, and the rest of the crowd");

    // Option 3 (Header flags analysis) — pinned facts, matching gntests.z5's
    // own Header section (zvm's headless defaults: Standard 1.1, colour off,
    // bold/italic/fixed-width/timed-input on, sound off, undo on).
    assert!(sections[2].contains("Your interpreter claims to follow revision 1.1 of the Z-Spec."));
    assert!(sections[2].contains("Interpreter claims that colored text IS NOT available."));
    assert!(sections[2].contains("Interpreter claims that emphasized (bold) text IS available."));
    assert!(sections[2].contains("Interpreter claims that italic (or underlined) text IS available."));
    assert!(sections[2].contains("Interpreter claims that fixed-width text IS available."));
    assert!(sections[2].contains("Interpreter claims that sound effects ARE NOT available."));
    assert!(sections[2].contains("Interpreter claims that timed input IS available."));
    assert!(sections[2].contains("Interpreter claims that \"undo\" IS available."));
    assert_in_dfrotz_golden("Your interpreter claims to follow revision 1.1 of the Z-Spec.");
    assert_in_dfrotz_golden("Interpreter claims that \"undo\" IS available.");

    // Option 4 (Styled text) — purely visual, just confirm it ran to its end
    // banner (no crash/short-circuit).
    assert!(sections[3].contains("End of styles test."));
    assert_in_dfrotz_golden("End of styles test.");

    // Option 5 (Colored text) — headless (no ANSI host), so the "would you
    // see" phrasing rather than "you should see".
    assert!(sections[4].contains("Interpreter claims that colored text IS NOT available."));
    assert!(sections[4].contains("If it was, in the square below, you would see"));
    assert_in_dfrotz_golden("If it was, in the square below, you would see");

    // Option 6 (Multiplication/division/remainder) — all twelve signed
    // arithmetic checks must read "(ok)", never "ERROR".
    assert!(sections[5].contains("This aspect of your interpreter appears to behave according to spec."));
    assert_eq!(sections[5].matches("(ok)").count(), 12, "all 12 TestDiv checks must read \"(ok)\":\n{}", sections[5]);
    assert_in_dfrotz_golden("-13 % -5 = -3 (ok)");
}

#[test]
fn etude_option_7_accents_display() {
    let Some(mut machine) = boot_to_menu() else { return };
    machine.supply_line("7", 13);
    // TestAccents prints its list unconditionally BEFORE its first
    // `@read_char` (it starts the display loop at `opt = 0`).
    assert_eq!(next_prompt(&mut machine), StepResult::NeedChar);
    let out = buf(&machine);
    assert_no_error_markers(&out, "7");
    // The same ZSCII 155-223 table as gntests.z5's Accents section, in
    // etude's own "name:glyph" layout (four per line).
    assert!(out.contains("a-umlaut:ä"), "missing ZSCII 155:\n{out}");
    assert!(out.contains("inverse-?:¿"), "missing ZSCII 223:\n{out}");
    assert!(out.contains("Type a digit (0..7) to repeat this list in a different text style"));
    assert_in_dfrotz_golden("a-umlaut:ä");
    assert_in_dfrotz_golden("inverse-?:¿");

    // '.' ends the test and returns straight to the top menu.
    machine.supply_char(b'.');
    assert!(
        matches!(next_prompt(&mut machine), StepResult::NeedLine { .. }),
        "'.' at the Accents prompt must return to the top menu"
    );
}

#[test]
fn etude_option_8_single_key_input() {
    let Some(mut machine) = boot_to_menu() else { return };
    machine.supply_line("8", 13);
    assert_eq!(next_prompt(&mut machine), StepResult::NeedChar);
    let before = buf(&machine).len();
    machine.supply_char(b'x');
    assert_eq!(next_prompt(&mut machine), StepResult::NeedChar);
    let out = buf(&machine);
    assert_no_error_markers(&out, "8");
    assert!(
        out[before..].contains("code=120: ASCII character 'x'"),
        "typed 'x' (ZSCII 120) must echo back its code and description:\n{}",
        &out[before..]
    );
    assert_in_dfrotz_golden("code=120: ASCII character 'x'");
    // '.' ends the test.
    machine.supply_char(b'.');
    assert!(matches!(next_prompt(&mut machine), StepResult::NeedLine { .. }));
    let out2 = buf(&machine);
    assert!(out2.contains("Test finished."), "missing exit banner:\n{out2}");
    assert_in_dfrotz_golden("Test finished.");
}

#[test]
fn etude_option_9_full_line_input_lowercases() {
    let Some(mut machine) = boot_to_menu() else { return };
    machine.supply_line("9", 13);
    assert!(matches!(next_prompt(&mut machine), StepResult::NeedLine { .. }));
    let before = buf(&machine).len();
    // ZMSD §3.8: input is folded to lower-case; typing "Hi There" must echo
    // ZSCII for 'h','i',' ','t','h','e','r','e' — never the capitals.
    machine.supply_line("Hi There", 13);
    assert!(matches!(next_prompt(&mut machine), StepResult::NeedLine { .. }));
    let out = buf(&machine);
    let tail = &out[before..];
    assert_no_error_markers(tail, "9");
    for expect in [
        "code=104: ASCII character 'h'", "code=105: ASCII character 'i'",
        "code=32: ASCII character ' '", "code=116: ASCII character 't'",
        "code=104: ASCII character 'h'", "code=101: ASCII character 'e'",
        "code=114: ASCII character 'r'", "code=101: ASCII character 'e'",
    ] {
        assert!(tail.contains(expect), "missing {expect:?} in:\n{tail}");
        assert_in_dfrotz_golden(expect);
    }
    assert!(!tail.contains("code=72"), "'H' (capital, ZSCII 72) must not appear -- input is lower-cased:\n{tail}");

    // A blank line returns to the top menu.
    machine.supply_line("", 13);
    assert!(matches!(next_prompt(&mut machine), StepResult::NeedLine { .. }));
}

#[test]
fn etude_option_10_timed_single_key_input() {
    let Some(mut machine) = boot_to_menu() else { return };
    machine.supply_line("10", 13);
    // TestTimedChar's own GetKey(): '.' returns, anything else begins.
    assert_eq!(next_prompt(&mut machine), StepResult::NeedChar);
    let banner = buf(&machine);
    assert!(banner.contains("Your interpreter claims (by its header bit) that it DOES support timed input."));
    machine.supply_char(b' '); // begin

    // The timed @read_char (`timedch.inc`'s `TimedCharSplot`, time=10 =
    // 1.0s). `next_prompt` drains the intervening `new_line` prints
    // (`Continue` steps) between the menu keypress and the read itself. Per
    // the module doc: run several ticks WITHOUT calling step() in between,
    // then complete the read for real with a keystroke.
    match next_prompt(&mut machine) {
        StepResult::NeedChar => {}
        other => panic!("expected the timed read_char's own NeedChar: {other:?}"),
    }
    let (time, _routine) = machine.pending_timeout().expect("TestTimedChar's read_char must be timed");
    assert_eq!(time, 10, "TerpEtude drives this at 1.0s (time_tenths=10)");
    let before = buf(&machine).len();
    for _ in 1..=3 {
        let r = machine.run_timed_interrupt();
        assert!(!r.aborted, "TimedCharSplot (timedch.inc) always returns 0 -- never aborts on its own");
    }
    let ticked = buf(&machine)[before..].to_string();
    assert_eq!(ticked, "* * * ", "each tick prints one \"* \" (TimedCharSplot's own print):\n{ticked:?}");
    // Stop the test with a real keystroke (not a timeout).
    machine.supply_char(b' ');
    assert_eq!(next_prompt(&mut machine), StepResult::NeedChar, "back at TestTimedChar's own \"any key to begin\" prompt");
    let out = buf(&machine);
    assert_no_error_markers(&out, "10");
    assert!(
        out.contains("Your interpreter calls the timing interrupt function with no arguments. This aspect of your interpreter appears to behave according to spec."),
        "claim=1 (timed input available) + argument-less interrupt calls must read SectionOk:\n{out}"
    );

    machine.supply_char(b'.'); // return to top menu
    assert!(matches!(next_prompt(&mut machine), StepResult::NeedLine { .. }));
}

#[test]
fn etude_option_11_timed_full_line_input() {
    let Some(mut machine) = boot_to_menu() else { return };
    machine.supply_line("11", 13);
    assert_eq!(next_prompt(&mut machine), StepResult::NeedChar);
    machine.supply_char(b' '); // begin

    // TestTimedString prints "Beginning test..." and its first
    // "TimedString> " prompt before the first timed @aread; `next_prompt`
    // drains those `Continue` steps. `timedstr.inc`'s `TimedStringSplot`
    // prints nothing on ticks 1-2, "[Every three seconds....]\nTimedString> "
    // on tick 3.
    match next_prompt(&mut machine) {
        StepResult::NeedLine { .. } => {}
        other => panic!("expected the timed @aread's own NeedLine: {other:?}"),
    }
    let (time, _routine) = machine.pending_timeout().expect("TestTimedString's @aread must be timed");
    assert_eq!(time, 10, "TerpEtude drives this at 1.0s (time_tenths=10) per interrupt tick");
    let mut every_three_seconds_seen = false;
    for tick in 1..=4 {
        let before = buf(&machine).len();
        let r = machine.run_timed_interrupt();
        assert!(!r.aborted, "TimedStringSplot (timedstr.inc) always returns 0 -- never aborts on its own");
        let printed = buf(&machine)[before..].to_string();
        if tick == 3 {
            assert_eq!(printed, "\n[Every three seconds....]\nTimedString> ", "tick 3 prints the redraw line:\n{printed:?}");
            every_three_seconds_seen = true;
        } else {
            assert_eq!(printed, "", "ticks 1/2/4 print nothing (only every 3rd does):\n{printed:?}");
        }
    }
    assert!(every_three_seconds_seen);
    // Complete the read for real with "." -- ends TestTimedString's loop.
    machine.supply_line(".", 13);
    assert!(matches!(next_prompt(&mut machine), StepResult::NeedLine { .. }));
    let out = buf(&machine);
    assert_no_error_markers(&out, "11");
    assert!(out.contains("You just typed \".\"."));
    assert!(out.contains("Test terminated."));
    assert!(
        out.contains("Your interpreter calls the timing interrupt function with no arguments. This aspect of your interpreter appears to behave according to spec."),
        "claim=1 + argument-less interrupt calls must read SectionOk:\n{out}"
    );
}

#[test]
fn etude_option_13_undo_supports_single_undo() {
    let Some(mut machine) = boot_to_menu() else { return };
    machine.supply_line("13", 13);
    assert_eq!(next_prompt(&mut machine), StepResult::NeedChar); // SingleUndo> prompt
    let banner = buf(&machine);
    assert!(banner.contains("Your interpreter claims (by its header bit) that it DOES support undo."));
    assert!(banner.contains("Simulating first move...\nSave succeeded."));
    assert!(banner.contains("Simulating second move...\nSave succeeded."));

    // Any non-'.' key tries the (single) undo -- @restore_undo rewinds
    // execution to just after the SECOND @save_undo, whose result is now 2.
    machine.supply_char(b' ');
    assert_eq!(next_prompt(&mut machine), StepResult::NeedChar); // MultipleUndo> prompt
    let out = buf(&machine);
    assert!(out.contains("Undo succeeded (undid second move)."));

    // '.' declines the second undo -- final verdict: single-undo support,
    // which zvm's undo_cap (>1) satisfies. SectionOk, no ERROR.
    machine.supply_char(b'.');
    assert!(matches!(next_prompt(&mut machine), StepResult::NeedLine { .. }));
    let out2 = buf(&machine);
    let tail = &out2[out.len()..];
    assert_no_error_markers(tail, "13");
    assert!(
        tail.contains("Your interpreter claims to support \"undo\", and it does. This aspect of your interpreter appears to behave according to spec."),
        "single-undo verdict must read SectionOk:\n{tail}"
    );
    assert_in_dfrotz_golden("Your interpreter claims to support \"undo\", and it does. This aspect of your interpreter appears to behave according to spec.");
}

#[test]
fn etude_option_14_prints_before_quitting() {
    let Some(mut machine) = boot_to_menu() else { return };
    machine.supply_line("14", 13);
    assert_eq!(next_prompt(&mut machine), StepResult::NeedChar); // ClosingText> prompt
    let banner = buf(&machine);
    assert!(banner.contains("This tests if you can read text which is displayed immediately before the program quits."));

    // Any non-'.' key prints the closing line and calls @quit immediately
    // afterward -- the whole point of this option is that the line must
    // still be visible in the transcript, not lost to an unflushed buffer.
    let before = buf(&machine).len();
    machine.supply_char(b' ');
    assert_eq!(next_prompt(&mut machine), StepResult::Quit);
    let out = buf(&machine);
    assert_no_error_markers(&out[before..], "14");
    assert!(
        out[before..].contains("This is a final line of text. Goodbye."),
        "text printed immediately before @quit must survive in the transcript:\n{}",
        &out[before..]
    );
    assert_in_dfrotz_golden("This is a final line of text. Goodbye.");
}
