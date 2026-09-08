//! SQ-1416: two real conformance-suite `.ulx` fixtures (Andrew Plotkin's
//! `unit_tests/README.md` manifest — gitignored, fetched, "freely
//! redistributable"; see the project `CLAUDE.md` Test Fixtures section)
//! falsifying two of the audit's Glk-conformance findings against the actual
//! upstream test content rather than a hand-written probe.
//!
//! `unit_tests/` is gitignored (like `stories/`) and absent on CI and in a
//! fresh clone — both tests skip vacuously when their fixture is missing, the
//! same pattern `accel_story_equivalence.rs` uses for `stories/`.

use std::path::PathBuf;

use gvm::{Machine, Memory, StepResult, TestBackend};

/// The repo-root `unit_tests/` directory, resolved relative to this crate's
/// manifest so the tests work regardless of `cargo test`'s working directory.
fn unit_tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../unit_tests")
}

/// Ceiling on opcode steps for either fixture — both are short, deterministic
/// harnesses with no player interaction; a runaway means a VM bug, not a slow
/// story, so keep this tight enough to fail fast instead of hanging a run.
const MAX_STEPS: u64 = 50_000_000;

/// Drive `image` to completion, answering `commands` at successive `NeedLine`
/// prompts (in order) and refusing any other kind of suspension (both
/// fixtures are self-contained conformance harnesses; a `NeedChar`/
/// `NeedFilename`/save request here would mean the harness's assumptions
/// about the fixture are wrong, not that the VM misbehaved). Returns the full
/// text-buffer transcript.
fn drive(name: &str, image: Vec<u8>, commands: &[&str]) -> String {
    let mem = Memory::new(image).expect("valid Glulx image");
    let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
    let mut steps = 0u64;
    let mut next = 0usize;
    loop {
        match m.step() {
            StepResult::Continue => {
                steps += 1;
                assert!(steps < MAX_STEPS, "{name}: runaway, {MAX_STEPS} steps without reaching Quit");
            }
            StepResult::NeedLine { .. } => {
                let cmd = commands
                    .get(next)
                    .unwrap_or_else(|| panic!("{name}: asked for a line beyond the scripted {commands:?}"));
                next += 1;
                m.supply_line(cmd);
            }
            StepResult::Quit => break,
            StepResult::Fault => {
                panic!("{name}: unexpected VM fault: {:?}", m.diagnostics());
            }
            other => panic!("{name}: unexpected suspension {other:?}"),
        }
    }
    m.backend_mut().as_any_mut().downcast_mut::<TestBackend>().unwrap().all_text()
}

/// SQ-1416 item 6 (`Model::sanitize_fileref_name`): drives the real
/// `externalfile.ulx` corpus story, which runs entirely on boot with no input
/// (confirmed interactively: `gvm-cli unit_tests/externalfile.ulx < /dev/null`
/// exits 0 having printed "All tests passed."). Its `RunInvalidCharacterTest`
/// creates a fileref named `testx/\<>:"|?*y.fake`, then checks a SEPARATELY
/// constructed fileref named `testxy` resolves to the same file — this is
/// exactly the Glk spec's "delete the nine disallowed characters, truncate at
/// the first period" rule (`/ \ < > : " | ? *` all deleted from between
/// "testx" and "y.fake", then everything from the first "." on is dropped),
/// and is the ONLY thing this story reports diverging from glulxe (per the
/// audit note). Before this fix, `sanitize_fileref_name` REPLACED each
/// disallowed character with `_` and kept the extension, so the same input
/// simplified to `testx_________y.fake` (nine underscores) — not `testxy` —
/// and this story's own check would read `File does not exist!` /
/// `FAILED: name simplification differs`, not "with standard name
/// simplification.".
#[test]
fn externalfile_corpus_reports_all_tests_passed() {
    let path = unit_tests_dir().join("externalfile.ulx");
    let Ok(image) = std::fs::read(&path) else {
        eprintln!("skipping: {} not vendored (gitignored fixture)", path.display());
        return;
    };

    let text = drive("externalfile", image, &[]);

    assert!(
        text.contains("Checking: testxy"),
        "externalfile: expected the RunInvalidCharacterTest's simplified-name check; got:\n{text}"
    );
    assert!(
        text.contains("File exists, with standard name simplification."),
        "externalfile: the simplified fileref name did not resolve to the same file — \
         sanitize_fileref_name's character-deletion/truncation rule is wrong; got:\n{text}"
    );
    assert!(
        text.contains("All tests passed."),
        "externalfile: story reported a failure (see the transcript for which check); got:\n{text}"
    );
    assert!(!text.contains("FAILED"), "externalfile: story reported a FAILED check; got:\n{text}");
}

/// Extract the text between the first line starting with `start_marker` and
/// the following line starting with `end_marker` (both exclusive of the
/// boundary lines' own content isn't trimmed — the whole line at `end_marker`
/// is excluded, everything up to it is kept), searching from `from_line`
/// onward. Panics with the searched range printed if either marker is missing
/// — a silent empty-string compare would make the test vacuously pass. Uses
/// `contains`, not `starts_with`: the transcript has no newline between the
/// game's own `>` input prompt and the first line of its response, so the
/// very first section header after a prompt is glued onto the same line
/// (`>core test: …`).
fn section<'a>(lines: &'a [&'a str], from_line: usize, start_marker: &str, end_marker: &str) -> (Vec<&'a str>, usize) {
    let start = lines[from_line..]
        .iter()
        .position(|l| l.contains(start_marker))
        .unwrap_or_else(|| panic!("marker {start_marker:?} not found from line {from_line}"))
        + from_line;
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.contains(end_marker))
        .unwrap_or_else(|| panic!("marker {end_marker:?} not found after line {start}"))
        + start
        + 1;
    (lines[start..end].to_vec(), end)
}

/// SQ-1416 item 7 (`accel::Machine::accel_error`): drives the real
/// `accelfunctest.ulx` corpus story with its own "slow all" (run every test
/// group through the interpreted Inform library routine) then "fast all" (run
/// every group through the attached `@accelfunc` native routine) commands —
/// confirmed interactively via `gvm-cli`: the story labels each section
/// "Unaccelerated"/"ACCELERATED" but otherwise prints identical content,
/// EXCEPT that the CP__Tab(new)/OC__Cl(new)/RV__Pr(new) groups deliberately
/// feed malformed input to provoke `accel_error`'s three
/// `"[** Programming error: … **]"` diagnostics. Before this fix
/// (`accel_error` a documented no-op) those three diagnostics appeared only
/// in the "Unaccelerated" (slow) sections and were silently absent from
/// "ACCELERATED" (fast) — this is the audit's "slow-vs-fast transcripts
/// DIFFER on CP__Tab(new)/OC__Cl(new)/RV__Pr(new)" finding, verbatim. This
/// test pins that the three "(new)" sections are now identical between the
/// two runs (module the "Unaccelerated"/"ACCELERATED" label itself), and that
/// each section actually contains its diagnostic (so the comparison isn't
/// vacuously passing on two empty/short sections).
#[test]
fn accelfunctest_corpus_slow_and_fast_diagnostics_match() {
    let path = unit_tests_dir().join("accelfunctest.ulx");
    let Ok(image) = std::fs::read(&path) else {
        eprintln!("skipping: {} not vendored (gitignored fixture)", path.display());
        return;
    };

    let text = drive("accelfunctest", image, &["slow all", "fast all", "quit", "y"]);
    let lines: Vec<&str> = text.lines().collect();

    // Two full passes over the same 14 groups: "slow all" runs everything
    // "Unaccelerated" first, then "fast all" runs everything "ACCELERATED".
    // Locate each pass by its "core test:" opener (the very first group), so
    // the second `section()` search starts strictly after the first pass ends.
    let slow_core = lines
        .iter()
        .position(|l| l.contains("core test:"))
        .expect("accelfunctest: no 'core test:' opener at all — wrong fixture or protocol changed");
    let fast_core = lines[slow_core + 1..]
        .iter()
        .position(|l| l.contains("core test:"))
        .map(|i| i + slow_core + 1)
        .expect("accelfunctest: only one 'core test:' pass — 'fast all' did not run a second sweep");

    for (group_start, group_end, diagnostic) in [
        ("CPTab (new) test:", "RAPr (new) test:", "tried to find the \".\" of (something)"),
        ("OCCl (new) test:", "RVPr (new) test:", "tried to apply 'ofclass' with non-class"),
        ("RVPr (new) test:", "OPPr (new) test:", "tried to read (something)"),
    ] {
        let (slow_block, _) = section(&lines, slow_core, group_start, group_end);
        let (fast_block, _) = section(&lines, fast_core, group_start, group_end);

        let normalize = |block: &[&str]| -> Vec<String> {
            block.iter().map(|l| l.replace("Unaccelerated", "X").replace("ACCELERATED", "X")).collect()
        };
        let slow_norm = normalize(&slow_block);
        let fast_norm = normalize(&fast_block);

        assert_eq!(
            slow_norm, fast_norm,
            "accelfunctest: {group_start} slow vs fast diverge beyond the accel-label word.\n\
             --- slow ---\n{}\n--- fast ---\n{}",
            slow_block.join("\n"),
            fast_block.join("\n")
        );
        assert!(
            slow_block.iter().any(|l| l.contains(diagnostic)),
            "accelfunctest: {group_start}'s slow section never printed {diagnostic:?} — the \
             comparison above is vacuous.\n--- slow ---\n{}",
            slow_block.join("\n")
        );
        assert!(
            fast_block.iter().any(|l| l.contains(diagnostic)),
            "accelfunctest: {group_start}'s ACCELERATED (fast) section never printed \
             {diagnostic:?} — accel_error is not reaching the current Glk stream.\n--- fast ---\n{}",
            fast_block.join("\n")
        );
    }

    // The story's own dedicated `errmsg` group (function 8, CP__Tab new)
    // states outright what it is checking: "This tests the error messages in
    // accelerated functions to make sure they go through iosys properly." It
    // is the LAST group in each pass, ending at "Done." — take everything
    // from its (fast-pass) opener to end of transcript, which is at most that
    // one short section.
    let errmsg_start = lines[fast_core..]
        .iter()
        .position(|l| l.contains("errmsg test:"))
        .map(|i| i + fast_core)
        .expect("accelfunctest: no ACCELERATED errmsg section — protocol changed");
    let errmsg_fast = &lines[errmsg_start..];
    assert!(
        errmsg_fast.iter().any(|l| l.contains("tried to find the \".\" of (something)")),
        "accelfunctest: the dedicated ACCELERATED errmsg test never printed its diagnostic; got:\n{}",
        errmsg_fast.join("\n")
    );
}
