// gntests.z5 (SQ-1421) — Graham Nelson's Z-Spec 0.99 test programs (Fonts,
// Accents, InputCodes, Colours, Header, TimedInput) bundled behind one
// idiot menu ("1: Fonts; 2: Accents; 3: InputCodes, 4: Colours, 5: Header,
// 6: TimedInput, 0: Exit"), shared with `gntests_input_codes.rs`
// (`crates/zvm-cli/tests/`, SQ-1423) and driven here at the `zvm` core level
// instead of through a spawned `zvm-cli` process. Each section is a single
// `#[test]` that boots a fresh machine, selects the section by its menu
// digit, drives it to completion and asserts no "error"/"should not" marker
// appears — matching the SQ-1014 audit's own manual drive of all six.
//
// The Fonts section is the one surprise here: it paints its four glyph
// tables into the UPPER (split) window, not stream 1 — for a v3-5 story a
// `@split_window`ed upper window is a screen-grid the engine tracks in
// `Machine::screen.upper`, entirely separate from `BufferOutput`'s stream-1
// buffer (ZMSD §8: the upper window is never part of the scrolling
// transcript). `dfrotz`'s dumb interface merges both onto one terminal, so a
// transcript diff would need to do the same; reading `screen.upper` directly
// is the more honest assertion at this layer.
//
// TimedInput is driven through the real timed-read protocol
// (`Machine::pending_timeout` / `run_timed_interrupt` / `abort_timed_input`)
// rather than wall-clock sleeps — `pending_timeout()` exposes exactly the
// `(time_tenths, packed_routine)` gntests' own read_char asked for, and
// calling `run_timed_interrupt()` in a loop simulates the host's timer tick
// without a real second (or tenth of one) elapsing per test.

use zvm::cpu::exec::{Machine, StepResult};
use zvm::memory::Memory;
use zvm::text::input::ZsciiInput;

fn boot() -> Option<Machine> {
    let story = zvm::fixtures::load("gntests.z5")?;
    let mem = Memory::new(story).expect("gntests.z5: Memory::new failed");
    let mut machine = Machine::new(mem);
    machine.init_caps();
    Some(machine)
}

/// Step `machine` to `Quit`, feeding `inputs` in order to every `NeedChar`
/// that is NOT a pending timed read (those are answered by running the
/// interrupt routine to its own timeout — see module doc). Once `inputs` is
/// exhausted, '0' (the menu's Exit digit) is fed as a safety net so an
/// under-provisioned script still terminates instead of hanging into the
/// step cap. Returns the full stream-1 transcript.
fn drive(machine: &mut Machine, inputs: &[u8]) -> String {
    let mut it = inputs.iter();
    const MAX_STEPS: u64 = 2_000_000;
    for _ in 0..MAX_STEPS {
        match machine.step() {
            StepResult::Quit | StepResult::Restart => {
                return machine.buffer_output().map(|b| b.buf.clone()).unwrap_or_default();
            }
            StepResult::Continue => {}
            StepResult::Fault => {
                let t = machine.take_fault_trace();
                panic!("gntests.z5 faulted the interpreter: {:?}", t.map(|t| t.fault));
            }
            StepResult::NeedLine { .. } => machine.supply_line("", 13),
            StepResult::NeedChar => {
                if machine.pending_timeout().is_some() {
                    // Simulate the host's timer: run the interrupt routine
                    // once per tick until it reports the read should abort.
                    loop {
                        if machine.run_timed_interrupt().aborted {
                            break;
                        }
                    }
                    machine.abort_timed_input("");
                } else {
                    let raw = *it.next().unwrap_or(&b'0');
                    machine.supply_char(
                        ZsciiInput::new(raw).expect("gntests menu digits are printable ASCII"),
                    );
                }
            }
            StepResult::SaveRequest => machine.complete_save(false),
            StepResult::RestoreRequest => machine.complete_restore_failure(),
            other => panic!("unexpected StepResult driving gntests.z5: {other:?}"),
        }
    }
    panic!("gntests.z5 did not reach Quit within {MAX_STEPS} steps");
}

/// The upper-window grid, flattened row-major with '\n' between rows.
fn upper_window_text(machine: &Machine) -> String {
    let upper = &machine.screen.upper;
    let mut s = String::with_capacity((upper.cols as usize + 1) * upper.rows as usize);
    for r in 0..upper.rows as usize {
        for c in 0..upper.cols as usize {
            s.push(upper.cells[r * upper.cols as usize + c].ch);
        }
        s.push('\n');
    }
    s
}

/// No section may print an error/failure marker. gntests spells these
/// "error: ..." (InputCodes' illegal-code report) — checked case-sensitively
/// so this cannot trip on the word "error" inside ordinary prose (none of
/// these six sections happens to use it).
fn assert_no_error_markers(out: &str, section: &str) {
    assert!(
        !out.contains("error:") && !out.to_lowercase().contains("should not have been"),
        "gntests.z5 {section} section reported an error:\n{out}"
    );
}

#[test]
fn gntests_fonts_renders_upper_window_glyph_tables() {
    let Some(mut machine) = boot() else { return };
    // '1' selects Fonts; the display needs one "any key" to dismiss, then
    // '0' exits the outer menu (padded — see `drive`'s safety net).
    let out = drive(&mut machine, b"1 00");
    assert_no_error_markers(&out, "Fonts");

    let upper = upper_window_text(&machine);
    println!("Fonts upper-window content:\n{upper}");
    assert!(upper.contains("Font 1"), "missing Font 1 heading:\n{upper}");
    assert!(
        upper.contains("Font 2 unavailable"),
        "Font 2 (picture font) must read unavailable here:\n{upper}"
    );
    assert!(upper.contains("Font 3"), "missing Font 3 heading:\n{upper}");
    assert!(upper.contains("Font 4"), "missing Font 4 heading:\n{upper}");
    // Font 1's printable-ASCII row, byte-identical to dfrotz's own rendering.
    assert!(
        upper.contains("!\"#$%&'()*+,-./0123456789:;<=>?"),
        "Font 1's ASCII row is missing or altered:\n{upper}"
    );
    assert!(
        upper.contains("@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_"),
        "Font 1's uppercase row is missing or altered:\n{upper}"
    );
}

#[test]
fn gntests_accents_prints_the_full_zscii_table_without_errors() {
    let Some(mut machine) = boot() else { return };
    // '2' selects Accents. It prints TWO font passes (font 1, then font 4),
    // each paginating through 30-odd `@read_char`s (harmlessly over-padded
    // here) before its own "Please press SPACE." — then '0' exits the menu.
    let mut inputs = vec![b'2'];
    inputs.extend(std::iter::repeat_n(b' ', 40));
    inputs.push(b'0');
    let out = drive(&mut machine, &inputs);
    assert_no_error_markers(&out, "Accents");

    assert!(
        out.contains("Accented characters test in font 1"),
        "missing font-1 accents banner:\n{out}"
    );
    // The ZSCII 155-223 accented-character table (ZMSD §3.8.5.3), spot-checked
    // at both ends and matching dfrotz's own transcript byte for byte.
    assert!(out.contains("155:   ä     a-umlaut  ae"), "missing ZSCII 155 row:\n{out}");
    assert!(out.contains("223:   ¿     upside-down ?"), "missing ZSCII 223 row:\n{out}");
    assert_eq!(
        out.matches("Please press SPACE.").count(),
        2,
        "Accents drives one \"Please press SPACE.\" per font pass (font 1, font 4):\n{out}"
    );
}

#[test]
fn gntests_colours_reports_unavailable_headless() {
    let Some(mut machine) = boot() else { return };
    let out = drive(&mut machine, b"4 00");
    assert_no_error_markers(&out, "Colours");
    assert!(
        out.contains("Fine: the interpreter says colours are unavailable."),
        "expected the headless (no ANSI colour host) verdict:\n{out}"
    );
}

#[test]
fn gntests_header_reports_pinned_facts() {
    let Some(mut machine) = boot() else { return };
    let out = drive(&mut machine, b"5 00");
    assert_no_error_markers(&out, "Header");

    // Pinned facts (SQ-1421): zvm's headless default boot presents itself as
    // interpreter number 1 ("DECSYSTEM-20") version 'A' (screen.rs's
    // `default_interpreter_number`/`init_header_caps` defaults), claims
    // Standard 1.1, and boots at 80x24 — the same defaults `regression.rs`'s
    // CZECH/Praxix runs and `zvm::cpu::exec::Machine::new` boot with.
    for fact in [
        "Interpreter (machine) number 1 version A",
        "Standard specification claimed by the interpreter: 1.1",
        "Screen height: 24 lines",
        "Screen width: 80 fixed-pitch font characters",
    ] {
        assert!(out.contains(fact), "missing pinned Header fact {fact:?}:\n{out}");
    }
}

#[test]
fn gntests_timedinput_completes_via_timed_read_protocol() {
    let Some(mut machine) = boot() else { return };
    // '6' selects TimedInput, which issues two timed `@read_char`s (one full
    // second, one tenth) that `drive` answers via `pending_timeout` +
    // `run_timed_interrupt` — never a real keystroke — then a real
    // "Please press SPACE." prompt (' '), then '0' exits the menu.
    let out = drive(&mut machine, b"6 00");
    assert_no_error_markers(&out, "TimedInput");

    assert!(
        out.contains("Testing timed input"),
        "missing TimedInput banner:\n{out}"
    );
    assert_eq!(
        out.matches("Test complete.").count(),
        2,
        "TimedInput drives two timed reads (1s, then 1/10s), each printing its own \"Test complete.\":\n{out}"
    );
}

#[test]
fn gntests_inputcodes_bare_enter_reads_as_return() {
    let Some(mut machine) = boot() else { return };
    // '3' selects InputCodes; ZSCII 13 (Enter) is fed directly to the core VM
    // (bypassing zvm-cli's terminal glue, which `gntests_input_codes.rs`
    // covers separately — SQ-1423) so this is a pure-engine proof that
    // `@read_char` never sees ZSCII 10 for Enter; ' ' then exits the
    // InputCodes loop, '0' exits the menu.
    let out = drive(&mut machine, &[b'3', 13, b' ', b'0', b'0']);
    assert_no_error_markers(&out, "InputCodes");

    assert!(
        out.contains("Keyboard input code testing"),
        "missing InputCodes banner:\n{out}"
    );
    assert!(
        out.contains("13 return"),
        "a bare Enter (ZSCII 13) must read back as \"13 return\":\n{out}"
    );
    assert!(
        !out.contains("10 "),
        "ZSCII 10 (LF) must never reach @read_char for Enter (ZMSD §3.8):\n{out}"
    );
}
