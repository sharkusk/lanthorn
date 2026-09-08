//! ZSCII 10 must never reach a game's `@read_char` (SQ-1423, ZMSD §3.8: Return
//! is ZSCII 13; 10/LF is not a legal input code at all). A bare Enter on
//! piped stdin is a blank line, and reading its raw terminator byte back is
//! exactly that defect.
//!
//! Graham Nelson's `gntests.z5` (bundled with TerpEtude, from the Z-Spec
//! 0.99 appendix) drives this itself: its InputCodes screen reads one
//! `@read_char` per keystroke and prints `"error: code N should not have
//! been returned"` for anything outside the legal set. Piping a bare Enter
//! at it is the fastest oracle there is for this class of bug — no need to
//! hand-roll a fixture story when a real one already asks the question.
//!
//! Both of `zvm-cli`'s non-tty `read_char` paths are exercised here:
//! `read_char_input`'s piped branch (default mode) and `read_cooked_char`
//! (`--screen-reader`/`--plain`, which reads a whole cooked line per
//! keystroke to make `/menu` possible — SQ-0609). They used to disagree with
//! each other and with the spec; both must report 13 now.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

/// `crates/zvm/tests/fixtures/gntests.z5` — shared across crates the same way
/// `crates/app/tests/suites/fixture_paths.rs` reaches `minizork.z3` there.
fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../zvm/tests/fixtures/gntests.z5")
}

/// Run `zvm-cli <extra_args> <gntests.z5>` with `stdin_script` piped in, so
/// stdin is never a terminal — the non-interactive path under test.
fn run(extra_args: &[&str], stdin_script: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_zvm-cli"))
        .args(extra_args)
        .arg(fixture_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("zvm-cli spawns");
    child.stdin.take().unwrap().write_all(stdin_script).unwrap();
    child.wait_with_output().expect("zvm-cli runs")
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Menu "3" (InputCodes), a bare Enter, a space (exits the InputCodes loop),
/// then "0" (quit the outer menu). Every one of these keystrokes is itself
/// read via `@read_char`, so the whole script never touches a `read` (line)
/// opcode.
const SCRIPT: &[u8] = b"3\n\n \n0\n";

fn assert_return_not_lf(out: &Output, mode: &str) {
    assert!(out.status.success(), "{mode}: zvm-cli exited {:?}\nstderr: {}", out.status, String::from_utf8_lossy(&out.stderr));
    let text = stdout_of(out);
    assert!(
        !text.contains("should not have been returned"),
        "{mode}: gntests reported an illegal read_char code — full output:\n{text}"
    );
    assert!(
        text.contains("13 return"),
        "{mode}: bare Enter must read back as ZSCII 13 (\"13 return\") — full output:\n{text}"
    );
}

#[test]
fn bare_enter_reads_as_return_in_default_mode() {
    if !fixture_path().is_file() {
        eprintln!("gntests.z5 fixture absent — skipping (see crates/zvm/tests/fixtures/README.md)");
        return;
    }
    let out = run(&[], SCRIPT);
    assert_return_not_lf(&out, "default");
}

#[test]
fn bare_enter_reads_as_return_in_screen_reader_mode() {
    if !fixture_path().is_file() {
        eprintln!("gntests.z5 fixture absent — skipping (see crates/zvm/tests/fixtures/README.md)");
        return;
    }
    // `--screen-reader` routes NeedChar through `read_cooked_char` instead of
    // `read_char_input` (SQ-0609's menu-jump path) — a separate line in the
    // source that carried the identical bug independently.
    let out = run(&["--screen-reader"], SCRIPT);
    assert_return_not_lf(&out, "screen-reader");
}
