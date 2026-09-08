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

    // Wait until every group has reported, or give up after a generous deadline.
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let count = {
            let b = buf.lock().unwrap();
            String::from_utf8_lossy(&b).matches("Passed.").count()
        };
        if count >= want_count || Instant::now() >= deadline {
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
    let passed = out.matches("Passed.").count();
    assert!(
        passed >= want_count,
        "expected >= {want_count} groups ({want:?}) to report Passed., got {passed}.\n\
         --- transcript ---\n{out}"
    );
    assert!(
        !out.contains("tests failed"),
        "a group reported a failure.\n--- transcript ---\n{out}"
    );
}
