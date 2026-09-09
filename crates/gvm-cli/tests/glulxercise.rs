//! glulxercise conformance smoke (phase 3a Glk I/O capstone; widened to full
//! coverage by SQ-1417).
//!
//! Drives the vendored `glulxercise.ulx` (see `tests/fixtures/README.md`) through
//! the real `gvm-cli` binary, headlessly, and asserts every test group the STORY
//! ITSELF names reports `Passed.` with no failures.
//!
//! The group list is not hand-maintained here: `gvm-cli`'s first `help` reply
//! echoes glulxercise's own "or one of the following test options: ..." banner,
//! which this harness parses at run time. That means a future glulxercise
//! release that adds a 71st group is picked up automatically and asserted on —
//! the old hand-curated `IN_SCOPE` allow-list (26 groups, then 39 under SQ-1415)
//! could silently leave a new group untested forever; a parsed list cannot go
//! stale that way. Confirmed against the story's help banner (2026-09-08,
//! Release 13 / Serial 241202): 70 named groups.
//!
//! Two of those 70 are excluded from the *must-pass* assertion, by name, for
//! reasons that are properties of the groups themselves rather than of gvm:
//! - `random` genuinely exercises the RNG and prints its own disclaimer —
//!   "Tests may, very occasionally, fail through sheer bad luck." A
//!   conformance gate that can fail on bad luck is not a gate; `nonrandom`
//!   (deterministic, asserted) exercises the same opcode.
//! - `safari5` is not a test of the interpreter at all — its own description
//!   says it tracks "a known Javascript bug in Safari 5 ... on Quixe", and it
//!   always reports `Passed.` regardless of VM behavior.
//!
//! 70 - 2 = 68, which is why any note about this suite says "68 groups".
//!
//! glulxercise's `quit` does not exit on its own (it loops on end-of-input), so
//! the harness keeps stdin open — the VM simply blocks awaiting the next command
//! after the last one — drains stdout in a reader thread until every group has
//! reported, then terminates the child.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Groups excluded from the must-pass assertion. See the module doc for why
/// each one is excluded — never silently; a group not in this list and not
/// reporting `Passed.` fails the test loudly.
const EXCLUDED: &[&str] = &["random", "safari5"];

/// The line glulxercise's boot banner (and its `help` echo) uses to introduce
/// the quoted, comma-separated list of test-group command names.
const OPTIONS_MARKER: &str = "test options:";

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/glulxercise.ulx")
}

/// A fresh, call-unique scratch directory (SQ-1131/SQ-1163: `process::id()`
/// alone is shared by every test in this BINARY under `cargo test` and every
/// test in this PROCESS under nextest — neither is unique per call, so a
/// counter travels beside the pid). SQ-1417's widened group list runs
/// `protect`/`undo`/`restore`/`memsize`/`heap`, several of which exercise
/// `@save`/`@restore` and, without `--data-dir`, gvm-cli writes those
/// alongside the story file by default — i.e. into this crate's COMMITTED
/// `tests/fixtures/` — which a first run of this widening did (a stray
/// `glulxercise.ulx.save/` briefly appeared there). Passing this path via
/// `--data-dir` keeps the write off both the fixtures directory and any path
/// another test/process could collide on.
fn scratch_dir() -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NTH: AtomicUsize = AtomicUsize::new(0);
    let nth = NTH.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("gvm-cli-glulxercise-{}-{nth}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Extract the quoted group names following [`OPTIONS_MARKER`] on its line,
/// e.g. `"operand", "arith", ...`. Returns them in the order the story lists
/// them. Panics (with the searched text) if the marker is missing, so a
/// protocol change fails loudly rather than producing an empty (vacuously
/// passing) group list.
fn parse_group_names(transcript: &str) -> Vec<String> {
    let line = transcript
        .lines()
        .find(|l| l.contains(OPTIONS_MARKER))
        .unwrap_or_else(|| panic!("{OPTIONS_MARKER:?} not found in transcript:\n{transcript}"));
    let after = &line[line.find(OPTIONS_MARKER).unwrap() + OPTIONS_MARKER.len()..];
    // Splitting on '"' alternates outside-quote / inside-quote segments,
    // starting outside (index 0): the group names are the odd-indexed ones.
    let names: Vec<String> = after
        .split('"')
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, s)| s.to_string())
        .collect();
    assert!(
        !names.is_empty(),
        "no quoted group names found after {OPTIONS_MARKER:?} in: {after:?}"
    );
    names
}

#[test]
fn glulxercise_all_groups_pass() {
    let fixture = fixture_path();
    if !fixture.exists() {
        eprintln!("skipping: {} not vendored", fixture.display());
        return;
    }

    let exe = env!("CARGO_BIN_EXE_gvm-cli");
    let data_dir = scratch_dir();
    let mut child = Command::new(exe)
        .arg(&fixture)
        .arg("--data-dir")
        .arg(&data_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn gvm-cli");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    // Drain stdout in the background so the child never blocks on a full pipe.
    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let buf_reader = Arc::clone(&buf);
    let reader = thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        loop {
            match stdout.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => buf_reader.lock().unwrap().extend_from_slice(&chunk[..n]),
            }
        }
    });

    // The boot banner alone carries the options line (glulxercise prints it
    // once on startup, before reading any command), so no "help" round trip
    // is needed — just wait for it to show up.
    let deadline = Instant::now() + Duration::from_secs(30);
    let groups = loop {
        let snapshot = String::from_utf8_lossy(&buf.lock().unwrap()).to_string();
        if snapshot.contains(OPTIONS_MARKER) {
            break parse_group_names(&snapshot);
        }
        assert!(Instant::now() < deadline, "timed out waiting for the boot banner's options line");
        thread::sleep(Duration::from_millis(50));
    };

    // Non-vacuity guard: if parsing regressed to an empty (or implausibly
    // short) list, every assertion below would pass trivially. 70 is what the
    // story names today; allow drift in either direction but not collapse.
    assert!(
        groups.len() >= 60,
        "parsed implausibly few test groups ({}): {groups:?}",
        groups.len()
    );
    for must_be_present in EXCLUDED {
        assert!(
            groups.iter().any(|g| g == must_be_present),
            "expected {must_be_present:?} among the parsed groups (excluded deliberately, not \
             because it vanished): {groups:?}"
        );
    }

    let want: Vec<&str> = groups.iter().map(String::as_str).filter(|g| !EXCLUDED.contains(g)).collect();
    let want_count = want.len();

    for group in &want {
        writeln!(stdin, "{group}").expect("write command");
    }
    stdin.flush().expect("flush stdin");

    // Wait until every group has REPORTED — passed or failed, either one — or
    // fail fast on a stall, rather than waiting out a flat total deadline
    // regardless of outcome. Originally this loop counted only "Passed."
    // markers, so a group that FAILED (glulxercise prints "N tests failed."
    // instead of "Passed.") was indistinguishable from one still running: the
    // count could never reach `want_count`, and the loop burned the entire
    // 30s-per-group deadline (2040s for 68 groups) before falling through to
    // the failure assertion below. That is exactly what happened on CI run
    // 34283470859 (commit 553cb556): the floatexp group's four NaN-pow
    // failures ran out the clock instead of failing in under a second
    // (SQ-1433).
    //
    // "N tests failed." is glulxercise's own failure counterpart to
    // "Passed." — captured directly from the interpreter (2026-09-08, by
    // deliberately breaking `@sqrt` and running `floatexp`; see
    // `count_tests_failed`'s doc for the raw transcript). Now BOTH markers
    // count toward "reported", so the loop exits the instant every sent
    // group has one or the other, whether that is a quick pass or a quick
    // failure.
    //
    // A per-result INACTIVITY timer replaces the flat deadline as the
    // primary guard: if 60s pass with no NEW group reporting, the test fails
    // immediately, naming the next group in send order (the one `reported`
    // results have not yet reached) as the one that stalled — that is a
    // useful failure on its own, rather than a 2040s wait ending in "got 67".
    // The original per-group-scaled total (30s * want_count) remains as an
    // outer safety net beneath it, in case the child hangs before reporting
    // ANYTHING (so the very first group still gets that much room to boot
    // and run before the harness gives up entirely).
    const INACTIVITY_TIMEOUT: Duration = Duration::from_secs(60);
    let wait_start = Instant::now();
    let outer_deadline = wait_start + Duration::from_secs(30 * want_count as u64);
    let mut last_progress = wait_start;
    let mut reported = 0usize;
    let mut stalled_on: Option<&str> = None;
    loop {
        let snapshot = String::from_utf8_lossy(&buf.lock().unwrap()).to_string();
        let count = snapshot.matches("Passed.").count() + count_tests_failed(&snapshot);
        if count > reported {
            reported = count;
            last_progress = Instant::now();
        }
        if reported >= want_count {
            break;
        }
        let now = Instant::now();
        if now.duration_since(last_progress) >= INACTIVITY_TIMEOUT {
            stalled_on = Some(want.get(reported).copied().unwrap_or("<index past want_count>"));
            break;
        }
        if now >= outer_deadline {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    let _ = child.kill();
    let _ = child.wait(); // reap the killed child instead of leaving a zombie
    drop(stdin);
    let _ = reader.join();
    let _ = std::fs::remove_dir_all(&data_dir);

    let out = String::from_utf8_lossy(&buf.lock().unwrap()).to_string();

    if let Some(group) = stalled_on {
        panic!(
            "glulxercise: no new group result for {INACTIVITY_TIMEOUT:?} ({reported}/{want_count} \
             groups reported after {:?}); stalled waiting on {group:?} (next group in send order, \
             index {reported} of {want:?}).\n--- transcript so far ---\n{out}",
            wait_start.elapsed()
        );
    }

    let passed = out.matches("Passed.").count();
    let failed = count_tests_failed(&out);
    if passed != want_count || failed != 0 {
        // Segment the transcript by the interpreter's own `>` prompt so the
        // message names WHICH group failed ("floatexp: 4 tests failed.")
        // instead of a raw, unattributed count.
        let chunks = split_by_prompt(&out);
        let problems: Vec<String> = want
            .iter()
            .enumerate()
            .filter_map(|(i, group)| match chunks.get(i) {
                Some(chunk) if chunk.contains("Passed.") => None,
                Some(chunk) => Some(match tests_failed_line(chunk) {
                    Some(line) => format!("{group}: {line}\n{chunk}"),
                    None => format!("{group}: reported neither Passed. nor a failure summary\n{chunk}"),
                }),
                None => Some(format!("{group}: did not report")),
            })
            .collect();
        panic!(
            "glulxercise: {passed}/{want_count} groups passed, {failed} failure summaries seen, \
             after {:?}.\n--- problem groups ---\n{}\n--- full transcript ---\n{out}",
            wait_start.elapsed(),
            problems.join("\n---\n")
        );
    }
}

/// Count occurrences of glulxercise's own failure summary line, `"N tests
/// failed."` — the fail-path counterpart to `"Passed."`, printed once per
/// group that had at least one bad assertion. Captured directly from the
/// interpreter (2026-09-08): deliberately breaking `@sqrt` to add 1.0 to its
/// result and running `floatexp` prints per-assertion detail lines like
/// `"sqrt 2.25=2.50000 or $40200000 (should be 1.50000 or $3FC00000 FAIL)"`,
/// then closes with `"10 tests failed."` in exactly the position `"Passed."`
/// occupies on a clean run. Hand-scanned rather than pulling in a regex
/// dependency for one fixed-shape pattern: a run of ASCII digits immediately
/// followed by `" tests failed."`.
fn count_tests_failed(text: &str) -> usize {
    let marker = " tests failed.";
    let mut count = 0;
    let mut search_from = 0;
    while let Some(rel) = text[search_from..].find(marker) {
        let idx = search_from + rel;
        if idx > 0 && text.as_bytes()[idx - 1].is_ascii_digit() {
            count += 1;
        }
        search_from = idx + marker.len();
    }
    count
}

/// If `chunk` contains glulxercise's failure summary, return the exact `"N
/// tests failed."` substring (e.g. `"4 tests failed."` for the floatexp
/// failure SQ-1433 fixes) — the digit-run twin of [`count_tests_failed`],
/// used to name a specific group's failure rather than merely detect one.
fn tests_failed_line(chunk: &str) -> Option<&str> {
    let marker = " tests failed.";
    let idx = chunk.find(marker)?;
    let digits_start = chunk[..idx].rfind(|c: char| !c.is_ascii_digit()).map_or(0, |i| i + 1);
    (digits_start != idx).then(|| &chunk[digits_start..idx + marker.len()])
}

/// Split a transcript into one chunk per submitted command, in send order,
/// dropping the boot banner (everything before the first prompt). The
/// interpreter's prompt (`>`) is printed at the START of a line, immediately
/// before that command's own response — confirmed empirically (2026-09-08):
/// `grep -n '>'` against a captured two-command transcript (`arith`,
/// `floatexp`) shows exactly one `>`-prefixed line per sent command, never
/// mid-line, and never more than that (the harness never sends `quit`, so
/// there is no trailing prompt after the last group). Used only to build a
/// human-readable failure message — a group whose own output happened to
/// start a line with `>` would mis-segment here, but that only affects which
/// name a diagnostic prints, not the pass/fail verdict itself (which comes
/// from whole-transcript marker counts, computed independently above).
fn split_by_prompt(transcript: &str) -> Vec<&str> {
    transcript.split("\n>").skip(1).collect()
}
