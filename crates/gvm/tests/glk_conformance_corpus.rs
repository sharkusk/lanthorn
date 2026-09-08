//! SQ-1416: two real conformance-suite `.ulx` fixtures (Andrew Plotkin's
//! `unit_tests/README.md` manifest — gitignored, fetched, "freely
//! redistributable"; see the project `CLAUDE.md` Test Fixtures section)
//! falsifying two of the audit's Glk-conformance findings against the actual
//! upstream test content rather than a hand-written probe.
//!
//! `unit_tests/` is gitignored (like `stories/`) and absent on CI and in a
//! fresh clone — both tests skip vacuously when their fixture is missing, the
//! same pattern `accel_story_equivalence.rs` uses for `stories/`.
//!
//! SQ-1417 widens this to twelve more corpus stories, nine of them diffed
//! byte-for-byte against a real external oracle: glulxe 0.6.1 + cheapglk
//! 1.0.7, built from source (`cc`, no package manager involved — `zvm`/`gvm`
//! stay zero-dependency, but the ORACLE is an external C program run once to
//! record a transcript, not a crate dependency) and invoked with `-q -u`
//! (`-q` suppresses cheapglk's own banner line; `-u` requests UTF-8 I/O so
//! its output needs no Latin-1 renormalization to compare against gvm's,
//! which is always UTF-8). The recorded transcripts live beside this file as
//! `tests/fixtures/<story>.glulxe.txt` — plain text, `<end of input>\n`
//! (cheapglk's own EOF notice, printed once stdin closes) trimmed from the
//! end. Three legitimate sources of divergence are normalized before
//! comparing (see `normalize_interpreter_version`, `redact_clock_hm`,
//! `normalize_dispid` below); none of the eight recolour anything the STORY
//! itself decided.
//!
//! Not every one of the twelve compares cleanly, for reasons that are
//! properties of the *oracle* rather than of gvm:
//! - `randomgen.ulx` genuinely cannot: the Glulx spec (§2.14, `setrandom`)
//!   promises a *reproducible* sequence for a nonzero seed, never a specific
//!   *algorithm* — gvm seeds xorshift32 (see `Machine::set_rng_seed`'s own
//!   doc comment), glulxe seeds xoshiro128** (`osdepend.c`,
//!   `glulx_setrandom`). Same seed, two different (both spec-legal) PRNGs,
//!   so the actual numbers can never coincide. Tested structurally instead
//!   (right counts, right ranges) against gvm's own output.
//! - `statusbufferwin.ulx` needs two Glk windows (a text buffer plus a text
//!   grid); cheapglk is a deliberately minimal "dumb terminal" Glk library
//!   and refuses outright — every script hits its
//!   `"WARNING: This interpreter does not support multiple windows!"` before
//!   any story text prints. Pinned against gvm's own transcript instead,
//!   labelled as such.
//! - `inputfeaturetest.ulx` exercises `glk_set_terminators_line_event` and
//!   `glk_set_echo_line_event`; cheapglk implements neither
//!   (`"This Glk library does not support ...()."`) and there's no scripted
//!   path through the story that avoids both. Pinned against gvm's own
//!   transcript instead, labelled as such.
//!
//! Per the project CLAUDE.md ("Verify spec constants against authoritative
//! sources, never from memory"), the `randomgen` PRNG-divergence and the two
//! cheapglk gaps above were confirmed against glulxe's/cheapglk's own C
//! source at `/tmp/glxbuild` before being written off as "not gvm's fault" —
//! see the doc comments on the tests themselves for the exact source lines.

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
    drive_backend(name, image, commands, TestBackend::new())
}

/// Like [`drive`], but also answers `NeedChar` prompts (SQ-1417's
/// `selectvarianttest`/`inputeventtest` corpus stories exercise single-key
/// input) with the first character of the next scripted command — matching
/// how a real terminal line, typed one key at a time, is consumed a
/// character at a time when the story asks for a char event instead of a
/// line — and takes a caller-configured `backend` (SQ-1417's `datetimetest`
/// needs a fixed `with_local_offset` so its local-time text is
/// reproducible instead of reading the host clock's real UTC offset).
fn drive_backend(name: &str, image: Vec<u8>, commands: &[&str], backend: TestBackend) -> String {
    let mem = Memory::new(image).expect("valid Glulx image");
    let mut m = Machine::with_glk(mem, Box::new(backend));
    let mut steps = 0u64;
    let mut next = 0usize;
    loop {
        match m.step() {
            StepResult::Continue => {
                steps += 1;
                assert!(steps < MAX_STEPS, "{name}: runaway, {MAX_STEPS} steps without reaching Quit");
            }
            // Once the scripted commands are exhausted, stop driving — the
            // transcript already holds the response to the LAST scripted
            // command (the two `Continue`-until-the-next-prompt steps that
            // just ran captured it), and the story is simply idling at
            // another input request the way glulxercise's own `quit`
            // deliberately never satisfies (see the `glulxercise.rs` module
            // doc). Only `externalfile`/`accelfunctest` above end their own
            // scripts in a `quit`/`y` that reaches `StepResult::Quit`
            // naturally; every other corpus story here ends by running out
            // of script on purpose.
            StepResult::NeedLine { .. } if next >= commands.len() => break,
            StepResult::NeedChar { .. } if next >= commands.len() => break,
            StepResult::NeedLine { .. } => {
                let cmd = commands[next];
                next += 1;
                m.supply_line(cmd);
            }
            StepResult::NeedChar { .. } => {
                let cmd = commands[next];
                next += 1;
                let ch = cmd.chars().next().unwrap_or(' ');
                m.supply_char(ch as u32);
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

// ── SQ-1417: glulxe-transcript fixtures and normalization ──────────────────

/// Read a recorded glulxe transcript from `tests/fixtures/<story>.glulxe.txt`
/// (see the module doc for how these were captured). Panics if missing —
/// unlike the `.ulx` fixtures themselves (gitignored, may be absent), these
/// transcripts are committed, so a missing one is a build-tree problem, not
/// an environment one.
fn read_golden(story: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{story}.glulxe.txt"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Load `unit_tests/<file>`, or print why and return `None` if it isn't
/// vendored (gitignored, absent on CI and in a fresh clone) — the same
/// skip-vacuously pattern the two SQ-1416 tests above use directly.
fn load_or_skip(file: &str) -> Option<Vec<u8>> {
    let path = unit_tests_dir().join(file);
    match std::fs::read(&path) {
        Ok(image) => Some(image),
        Err(_) => {
            eprintln!("skipping: {} not vendored (gitignored fixture)", path.display());
            None
        }
    }
}

/// Replace `Interpreter version X.Y.Z` with a fixed placeholder wherever it
/// appears. Every glulxercise-family boot banner reports the interpreter's
/// OWN version (gvm's is its crate version; glulxe's is 0.6.1) — the two can
/// never match and were never meant to; this is not game behavior at all.
/// Zero-dependency by hand (no regex crate; `zvm`/`gvm` take none, tests
/// included — see the project CLAUDE.md hard rules).
fn normalize_interpreter_version(s: &str) -> String {
    let marker = "Interpreter version ";
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find(marker) {
        out.push_str(&rest[..pos + marker.len()]);
        rest = &rest[pos + marker.len()..];
        let skip = rest.bytes().take_while(|b| b.is_ascii_digit() || *b == b'.').count();
        out.push_str("X.Y.Z");
        rest = &rest[skip..];
    }
    out.push_str(rest);
    out
}

/// Replace every run of ASCII digits immediately following `marker` with a
/// fixed placeholder. Used for `selectvarianttest`'s `"Notestream dispid: "`
/// numbers — a Glk dispatch id is an opaque host-assigned handle (the Glk
/// spec never constrains its numbering), so gvm's and glulxe's differ by
/// construction and comparing them is not a meaningful conformance check.
fn normalize_after_marker(s: &str, marker: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find(marker) {
        out.push_str(&rest[..pos + marker.len()]);
        rest = &rest[pos + marker.len()..];
        let skip = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
        out.push('N');
        rest = &rest[skip..];
    }
    out.push_str(rest);
    out
}

/// Redact the quoted text immediately before `marker` back to the nearest
/// `"`. Used for `datetimetest`'s two live-clock readings — `"1:14 pm
/// (local)"` / `"8:14 pm (utc)"` in the room description report the REAL
/// current wall-clock time (there is no way to freeze "now" itself, only the
/// UTC offset via `TestBackend::with_local_offset`, which this test already
/// sets to match), so the two processes' transcripts drift a digit apart
/// whenever a minute boundary falls between the two runs. Deliberately
/// narrow (only strips text directly before `marker`, not any `H:MM` — the
/// calendar's fixed historical entries use the `H:MM:SS` form and a
/// different closing marker, so they are never touched).
fn redact_before_marker(s: &str, marker: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find(marker) {
        match rest[..pos].rfind('"') {
            Some(qpos) => {
                out.push_str(&rest[..qpos + 1]);
                out.push_str("H:MM ap");
            }
            None => out.push_str(&rest[..pos]),
        }
        out.push_str(&rest[pos..pos + marker.len()]);
        rest = &rest[pos + marker.len()..];
    }
    out.push_str(rest);
    out
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

// ── SQ-1417: per-story corpus tests, diffed against the glulxe fixtures ────

/// `resizememstreamtest.ulx` self-reports; runs to completion on boot with no
/// input (the memory-pointer-resize bug it probes — glkop.c holding a stale
/// pointer across a `realloc` — is exercised entirely by the story's own
/// `Initialise`). Diffed byte-for-byte against glulxe after normalizing only
/// the two interpreters' own version banners.
#[test]
fn resizememstreamtest_corpus_matches_glulxe() {
    let Some(image) = load_or_skip("resizememstreamtest.ulx") else { return };
    let live = normalize_interpreter_version(&drive("resizememstreamtest", image, &[]));
    let golden = normalize_interpreter_version(&read_golden("resizememstreamtest"));
    assert!(live.contains("Test passed."), "resizememstreamtest: no self-reported pass line; got:\n{live}");
    assert_eq!(live, golden, "resizememstreamtest: transcript diverges from glulxe's");
}

/// `memcopytest.ulx` (SQ-1415 hardened `@mcopy`'s descending-overlap branch):
/// `copy 0 3 5` overlaps forward (dest > src, must copy back-to-front to
/// avoid clobbering unread source bytes — glulxe's exec.c takes the
/// descending branch here), `copy 3 0 5` overlaps backward (dest < src, the
/// ascending branch). Diffed byte-for-byte against glulxe (no other
/// normalization needed — this story's boot banner has no version line).
#[test]
fn memcopytest_corpus_matches_glulxe() {
    let Some(image) = load_or_skip("memcopytest.ulx") else { return };
    let commands = ["say hello", "copy 0 3 5", "say hello", "copy 3 0 5"];
    let live = normalize_interpreter_version(&drive("memcopytest", image, &commands));
    let golden = normalize_interpreter_version(&read_golden("memcopytest"));
    assert_eq!(live, golden, "memcopytest: transcript diverges from glulxe's");
}

/// `memstreamtest.ulx`: byte and Unicode memory streams (`glk_stream_open_memory`
/// / `_uni`), null streams (length-only), positioning, and reading — the
/// `read`/`uniread` commands replay canned lines including one with a
/// non-Latin-1 character (`Liηe`), exercising the Unicode char-array read
/// path specifically. Diffed byte-for-byte against glulxe.
#[test]
fn memstreamtest_corpus_matches_glulxe() {
    let Some(image) = load_or_skip("memstreamtest.ulx") else { return };
    let commands = [
        "stream grape",
        "streamuni umlauts",
        "nullstream pie",
        "uninullstream restaurant",
        "pos",
        "unipos",
        "read",
        "uniread",
    ];
    let live = normalize_interpreter_version(&drive("memstreamtest", image, &commands));
    let golden = normalize_interpreter_version(&read_golden("memstreamtest"));
    assert_eq!(live, golden, "memstreamtest: transcript diverges from glulxe's");
}

/// `memheaptest.ulx`: `@mallocheap`/`@malloc`/`@mfree` status and block
/// addresses. The allocated addresses (114432, 114452, ...) are deterministic
/// — driven by the story's own `ENDMEM`/heap layout, not by host allocator
/// behavior — so they compare exactly, unlike a real host malloc's addresses
/// would. Diffed byte-for-byte against glulxe.
#[test]
fn memheaptest_corpus_matches_glulxe() {
    let Some(image) = load_or_skip("memheaptest.ulx") else { return };
    let commands = ["status", "alloc 20", "alloc 30", "status", "free 114432", "status"];
    let live = normalize_interpreter_version(&drive("memheaptest", image, &commands));
    let golden = normalize_interpreter_version(&read_golden("memheaptest"));
    assert_eq!(live, golden, "memheaptest: transcript diverges from glulxe's");
}

/// `randomgen.ulx`: NOT diffed against glulxe — see the module doc's
/// "randomgen" bullet. The Glulx spec (§2.14) leaves the PRNG algorithm
/// implementation-defined; gvm's `set_rng_seed` doc comment says outright it
/// uses xorshift32, while glulxe's `osdepend.c` `glulx_setrandom` seeds
/// xoshiro128** for any nonzero seed — confirmed by reading both source
/// files, not assumed. Same seed, two different (both spec-legal)
/// generators, so the actual printed numbers can never coincide; comparing
/// them would be asserting a coincidence, not conformance. Tested
/// structurally instead: right section headers, right counts, and — for the
/// bounded `range`/`nrange` commands, which DO constrain output regardless
/// of algorithm — every value inside its documented range.
#[test]
fn randomgen_corpus_seed_is_reproducible_and_ranges_hold() {
    let Some(image) = load_or_skip("randomgen.ulx") else { return };
    let commands = ["seed 12345", "print 20", "range 100", "nrange 50"];
    let text = drive("randomgen", image, &commands);

    assert!(text.contains("Setting RNG seed to 12345."), "randomgen: seed command not acknowledged:\n{text}");

    // Each section's sliced text ends at the START of the next header, which
    // (with no blank line between the trailing `>` prompt and the header
    // text) leaves a trailing `>`-only "line" in the slice — filtered out
    // below alongside blank lines, since neither parses as a number.
    let section = |header: &str, next_header: &str| -> Vec<i64> {
        let start = text.find(header).unwrap_or_else(|| panic!("randomgen: missing {header:?}:\n{text}"));
        let after = &text[start + header.len()..];
        // `str::find("")` always matches at 0, so an empty `next_header` (the
        // last section, running to end of transcript) must skip the search
        // rather than take that vacuous match.
        let end = if next_header.is_empty() { after.len() } else { after.find(next_header).unwrap_or(after.len()) };
        after[..end]
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && *l != ">")
            .map(|l| l.parse::<i64>().unwrap_or_else(|e| panic!("randomgen: bad number {l:?}: {e}")))
            .collect()
    };

    let numbers = section("Here are 20 random numbers...\n", "Here are 20 numbers from 0 to 100.");
    assert_eq!(numbers.len(), 20, "randomgen: expected 20 raw numbers, got {}: {numbers:?}", numbers.len());

    let range100 = section("Here are 20 numbers from 0 to 100.\n", "Here are 20 numbers from 0 to -50.");
    assert_eq!(range100.len(), 20, "randomgen: expected 20 range(100) numbers, got {range100:?}");
    assert!(range100.iter().all(|&v| (0..100).contains(&v)), "randomgen: range(100) value out of [0,100): {range100:?}");

    let nrange50 = section("Here are 20 numbers from 0 to -50.\n", "");
    assert_eq!(nrange50.len(), 20, "randomgen: expected 20 nrange(50) numbers, got {nrange50:?}");
    assert!(
        nrange50.iter().all(|&v| (-49..=0).contains(&v)),
        "randomgen: nrange(50) value out of [-49,0]: {nrange50:?}"
    );

    // Reproducibility: the same seed replayed on a fresh machine gives the
    // exact same sequence (gvm's own contract, independent of which PRNG
    // algorithm is behind it).
    let Some(image2) = load_or_skip("randomgen.ulx") else { return };
    let replay = drive("randomgen", image2, &commands);
    assert_eq!(text, replay, "randomgen: same seed produced a different sequence on replay");
}

/// `datetimetest.ulx`: `x calendar` reports six FIXED historical dates
/// (Moon landing, Unix epoch, ...) formatted via the story's own
/// timeval-to-date conversion — the actual conformance target here, and
/// exactly the sort of thing `read_glkdate`/`read_timeval`'s -1
/// input-struct-convention fix (SQ-1416 item 2) and glk_date formatting need
/// to agree with glulxe on. The room description ALSO prints the live local
/// and UTC clock (there is no way to freeze "now" itself), so the backend is
/// given a fixed `with_local_offset` matching this machine's real zone
/// (confirmed via `date +%z`; -25200s = UTC-7 = PDT) and the resulting
/// `H:MM ap` pair is redacted before comparing (`redact_before_marker`) —
/// the calendar's own six entries use `H:MM:SS` and a different closing
/// marker, so they're never touched by that redaction and DO compare
/// exactly.
#[test]
fn datetimetest_corpus_matches_glulxe() {
    let Some(image) = load_or_skip("datetimetest.ulx") else { return };
    let backend = TestBackend::new().with_local_offset(-25_200); // UTC-7 (PDT), this host's zone
    let raw = drive_backend("datetimetest", image, &["x calendar"], backend);
    let live = normalize_interpreter_version(&raw);
    let live = redact_before_marker(&live, " (local)\"");
    let live = redact_before_marker(&live, " (utc)\"");

    let golden_raw = read_golden("datetimetest");
    let golden = normalize_interpreter_version(&golden_raw);
    let golden = redact_before_marker(&golden, " (local)\"");
    let golden = redact_before_marker(&golden, " (utc)\"");

    assert!(live.contains("The first Moon landing:"), "datetimetest: calendar section missing:\n{live}");
    assert_eq!(live, golden, "datetimetest: transcript diverges from glulxe's");
}

/// `unicasetest.ulx`: Unicode case-folding (`glk_buffer_to_upper_case_uni`
/// etc.), decomposition, and normalization, including combining-mark and
/// ligature edge cases. `-u` on the glulxe invocation asks cheapglk for UTF-8
/// I/O, which turned out to make the CLAUDE.md-anticipated Latin-1
/// normalization unnecessary — the two transcripts already agree byte for
/// byte once the version banner is normalized (confirmed empirically before
/// writing this comment, not assumed). Asserts the story's own pass line too.
#[test]
fn unicasetest_corpus_matches_glulxe() {
    let Some(image) = load_or_skip("unicasetest.ulx") else { return };
    let live = normalize_interpreter_version(&drive("unicasetest", image, &["all"]));
    let golden = normalize_interpreter_version(&read_golden("unicasetest"));
    assert!(live.contains("All tests okay."), "unicasetest: no self-reported pass line; got:\n{live}");
    assert_eq!(live, golden, "unicasetest: transcript diverges from glulxe's");
}

/// `extbinaryfile.ulx`: binary/text, char/word, and Unicode fileref I/O
/// round trips, entirely on boot with no input (confirmed interactively).
/// Neither gvm's `TestBackend` nor cheapglk (run from an isolated scratch
/// directory) actually touches the working directory for this story's
/// filerefs (confirmed: `ls` before/after showed nothing written), so no
/// sandboxing is needed here beyond what `TestBackend` already provides.
/// Diffed byte-for-byte against glulxe; asserts the story's own pass line.
#[test]
fn extbinaryfile_corpus_matches_glulxe() {
    let Some(image) = load_or_skip("extbinaryfile.ulx") else { return };
    let live = normalize_interpreter_version(&drive("extbinaryfile", image, &[]));
    let golden = normalize_interpreter_version(&read_golden("extbinaryfile"));
    assert!(live.contains("All tests passed."), "extbinaryfile: no self-reported pass line; got:\n{live}");
    assert_eq!(live, golden, "extbinaryfile: transcript diverges from glulxe's");
}

/// `selectvarianttest.ulx`: a memory stream kept open across `glk_select`
/// char-event input, plus the opcode-operand-source variations (stack vs.
/// local) the story's own description calls out. Every input here is a
/// single key, so this exercises `drive_backend`'s `NeedChar` path. The
/// story's own `Notestream dispid: N` line names a Glk dispatch id — an
/// opaque, host-assigned handle the spec never constrains the numbering of
/// (confirmed: gvm reports 2, glulxe reports 194, for the exact same input)
/// — normalized away with `normalize_after_marker` before comparing.
#[test]
fn selectvarianttest_corpus_matches_glulxe() {
    let Some(image) = load_or_skip("selectvarianttest.ulx") else { return };
    let commands = ["a", "b", "0", "2", "m", "c", "u"];
    let live = normalize_interpreter_version(&drive("selectvarianttest", image, &commands));
    let live = normalize_after_marker(&live, "dispid: ");
    let golden = normalize_interpreter_version(&read_golden("selectvarianttest"));
    let golden = normalize_after_marker(&golden, "dispid: ");
    assert_eq!(live, golden, "selectvarianttest: transcript diverges from glulxe's");
}

/// `statusbufferwin.ulx`: NOT diffed against glulxe — see the module doc's
/// "statusbufferwin" bullet. This story opens a text-grid inventory window
/// alongside its text-buffer window at startup; cheapglk (confirmed by
/// reading `cgwindow.c`'s window-open path and by direct observation: every
/// scripted input here hits the same line before any story text prints)
/// refuses multi-window layouts outright with
/// `"WARNING: This interpreter does not support multiple windows!"`, so no
/// script through this story is comparable against it. Pinned against gvm's
/// own transcript instead: inventory-list formatting (`- the X` items),
/// the "newline mode" toggle switching from prepended to appended blank
/// lines, and the yes/no confirmation prompt.
#[test]
fn statusbufferwin_corpus_pinned_reference() {
    let Some(image) = load_or_skip("statusbufferwin.ulx") else { return };
    let commands = ["take apple", "inventory", "turn switch", "take pear", "inventory", "query", "yes"];
    let text = drive("statusbufferwin", image, &commands);

    assert!(text.contains("Taken.\n\n>You are carrying:\n  an apple"), "statusbufferwin:\n{text}");
    assert!(
        text.contains("You flip the switch. Newline mode is now \"end\"."),
        "statusbufferwin: newline-mode toggle message missing:\n{text}"
    );
    assert!(
        text.contains("You are carrying 2 items:\n- the pear\n- the apple"),
        "statusbufferwin: two-item inventory listing missing:\n{text}"
    );
    assert!(
        text.contains("Type \"yes\" or \"no\"...\nYou said yes."),
        "statusbufferwin: yes/no confirmation prompt missing:\n{text}"
    );
}

/// `inputeventtest.ulx`: a single scripted char event (`get character
/// input`, then the raw key `g`) followed by a scripted line event (`get
/// line input`, then a full line) — the two input-request kinds a real Glk
/// story alternates between, both exercised through `drive_backend`'s
/// `NeedLine`/`NeedChar` handling in one script. Diffed byte-for-byte against
/// glulxe (no version-banner line in this story's boot text, so no
/// normalization is needed at all).
#[test]
fn inputeventtest_corpus_matches_glulxe() {
    let Some(image) = load_or_skip("inputeventtest.ulx") else { return };
    let commands = [
        "get character input",
        "g",
        "get line input",
        "some test line",
        "x story-window button",
        "x status-window button",
    ];
    let live = drive("inputeventtest", image, &commands);
    let golden = read_golden("inputeventtest");
    assert_eq!(live, golden, "inputeventtest: transcript diverges from glulxe's");
}

/// `inputfeaturetest.ulx`: NOT diffed against glulxe — see the module doc's
/// "inputfeaturetest" bullet. Every interesting command this story offers
/// (`keys`/`terminate`/`even`/`odd`/`none` for line-input terminator keys;
/// `noecho`/`echo` for library echo control) routes through
/// `glk_set_terminators_line_event` or `glk_set_echo_line_event`; cheapglk
/// implements neither and answers every one of them with
/// `"This Glk library does not support ...()."` (confirmed: even the
/// read-only `keys` listing hits it, so there is no subset of commands that
/// avoids both gestalt gaps). Pinned against gvm's own transcript instead:
/// the full terminator-key listing, the terminator-count changes from
/// `terminate`/`even`, and the echo-mode toggle messages.
#[test]
fn inputfeaturetest_corpus_pinned_reference() {
    let Some(image) = load_or_skip("inputfeaturetest.ulx") else { return };
    let commands = ["keys", "terminate", "keys", "even", "noecho", "test line", "echo"];
    let text = drive("inputfeaturetest", image, &commands);

    assert!(
        text.contains("<escape>, <f1>, <f2>, <f3>, <f4>, <f5>, <f6>, <f7>, <f8>, <f9>, <f10>, <f11>, <f12>"),
        "inputfeaturetest: full terminator-key listing missing:\n{text}"
    );
    assert!(
        text.contains("There are now 13 special line input terminator keys."),
        "inputfeaturetest: terminate count missing:\n{text}"
    );
    assert!(
        text.contains("There are now 6 special line input terminator keys."),
        "inputfeaturetest: even count missing:\n{text}"
    );
    assert!(
        text.contains("The library has been set to no-echo mode for line input."),
        "inputfeaturetest: noecho message missing:\n{text}"
    );
    assert!(
        text.contains("The library has been set to echo mode for line input (the normal behavior)."),
        "inputfeaturetest: echo message missing:\n{text}"
    );
}
