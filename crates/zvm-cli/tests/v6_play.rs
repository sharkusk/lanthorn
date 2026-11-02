//! Version 6 text play through `zvm-cli`.
//!
//! A hand-built story is always present. `advent.z6` and `sunburst.z6` are
//! fetched (IF Archive personal-use terms) and skipped when absent.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

fn run(story: &Path, extra_args: &[&str], stdin: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_zvm-cli"))
        .arg(story)
        .args(extra_args)
        .arg("--pager")
        .arg("off")
        .arg("--sound")
        .arg("off")
        .env_remove("NO_COLOR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("zvm-cli spawns");
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    child.wait_with_output().expect("zvm-cli runs")
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn fixture(name: &str) -> Option<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for p in [
        root.join("stories").join(name),
        root.join("crates/app/tests/fixtures/stories").join(name),
    ] {
        if p.is_file() {
            return Some(p);
        }
    }
    eprintln!("SKIP: {name} not in stories/ or fetched fixtures");
    None
}

/// Packed-main Version 6 story: print `READY>`, `@read`, print `OK`, `@quit`.
fn v6_ready_story() -> Vec<u8> {
    let mut b = vec![0u8; 0x800];
    b[0x00] = 6;
    {
        let mut word = |o: usize, v: u16| b[o..o + 2].copy_from_slice(&v.to_be_bytes());
        word(0x04, 0x0500);
        word(0x06, 0x0010);
        word(0x08, 0x0480);
        word(0x0A, 0x0100);
        word(0x0C, 0x0200);
        word(0x0E, 0x0500);
        word(0x18, 0x0180);
        word(0x1A, 0x0100);
    }
    b[0x0480] = 0;
    b[0x0481] = 6;
    b[0x03E0] = 80;
    b[0x0440] = 8;
    b[0x40] = 0;
    let mut pc = 0x41;
    for ch in b"READY>" {
        b[pc] = 0xE5;
        b[pc + 1] = 0x7F;
        b[pc + 2] = *ch;
        pc += 3;
    }
    b[pc] = 0xE4;
    b[pc + 1] = 0x0F;
    b[pc + 2] = 0x03;
    b[pc + 3] = 0xE0;
    b[pc + 4] = 0x04;
    b[pc + 5] = 0x40;
    b[pc + 6] = 0x00;
    pc += 7;
    for ch in b"OK" {
        b[pc] = 0xE5;
        b[pc + 1] = 0x7F;
        b[pc + 2] = *ch;
        pc += 3;
    }
    b[pc] = 0xBA;
    b
}

fn write_temp(name: &str, bytes: &[u8]) -> PathBuf {
    static NTH: AtomicUsize = AtomicUsize::new(0);
    let nth = NTH.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("zvm-cli-v6-{}-{name}-{nth}", std::process::id()));
    std::fs::write(&path, bytes).expect("temp story written");
    path
}

#[test]
fn a_hand_built_v6_story_prints_accepts_a_line_and_quits() {
    let path = write_temp("ready.z6", &v6_ready_story());
    let out = run(&path, &["--plain"], b"look\n");
    let _ = std::fs::remove_file(&path);
    let text = stdout_of(&out);
    let err = stderr_of(&out);
    assert!(!err.contains("not supported"), "v6 must load:\n{err}");
    assert!(
        out.status.success(),
        "exit {:?}\nstdout:\n{text}\nstderr:\n{err}",
        out.status
    );
    assert!(text.contains("READY>"), "prompt:\n{text}");
    assert!(text.contains("OK"), "after the line:\n{text}");
}

#[test]
fn advent_z6_reaches_a_prompt_and_accepts_look() {
    let Some(path) = fixture("advent.z6") else {
        return;
    };
    // Title cards consume the first read_char bytes of this line; the rest is
    // the command. Extra blank lines become "I beg your pardon?" and are not
    // needed to reach the opening room.
    let out = run(&path, &["--plain"], b"look\nquit\ny\n");
    let text = stdout_of(&out);
    let err = stderr_of(&out);
    assert!(
        !err.contains("not supported"),
        "advent.z6 must load:\n{err}"
    );
    assert!(
        text.to_ascii_lowercase().contains("end of")
            || text.contains("building")
            || text.contains("At End Of Road"),
        "advent should describe the opening room:\n{text}\nstderr:\n{err}"
    );
}

#[test]
fn advent_z6_status_keeps_score_and_moves_on_one_line() {
    let Some(path) = fixture("advent.z6") else {
        return;
    };
    let out = run(&path, &["--plain", "--show-status"], b"look\nquit\ny\n");
    let text = stdout_of(&out);
    let err = stderr_of(&out);
    assert!(
        !err.contains("not supported"),
        "advent.z6 must load:\n{err}"
    );
    let status = text.lines().find(|l| l.contains("Score:") && l.contains("Moves:"));
    let Some(line) = status else {
        panic!("status line with both labels:\n{text}\nstderr:\n{err}");
    };
    assert!(
        line.contains("Score:")
            && line
                .split("Score:")
                .nth(1)
                .is_some_and(|rest| rest.chars().any(|c| c.is_ascii_digit())),
        "Score value beside its label: {line:?}"
    );
    assert!(
        line.contains("Moves:")
            && line
                .split("Moves:")
                .nth(1)
                .is_some_and(|rest| rest.chars().any(|c| c.is_ascii_digit())),
        "Moves value beside its label: {line:?}"
    );
}

#[test]
fn piped_image_protocol_kitty_emits_no_graphics() {
    let path = write_temp("ready.z6", &v6_ready_story());
    let out = run(&path, &["--image-protocol", "kitty"], b"look\n");
    let _ = std::fs::remove_file(&path);
    let text = stdout_of(&out);
    let err = stderr_of(&out);
    assert!(
        out.status.success(),
        "exit {:?}\nstdout:\n{text}\nstderr:\n{err}",
        out.status
    );
    assert!(text.contains("READY>"), "prompt:\n{text}");
    assert!(
        !text.contains("\x1b_G") && !text.contains("\x1bPq") && !text.contains('▀'),
        "piped stdout must stay text even when kitty is named:\n{text:?}"
    );
}

#[test]
fn sunburst_z6_starts_and_accepts_typed_look() {
    let Some(path) = fixture("sunburst.z6") else {
        return;
    };
    // Piped v6 read_char is one byte at a time: 's' starts, then "look" + return.
    let out = run(&path, &["--plain"], b"slook\n");
    let text = stdout_of(&out);
    let err = stderr_of(&out);
    assert!(
        !err.contains("not supported"),
        "sunburst.z6 must load:\n{err}"
    );
    assert!(
        text.contains("Sunburst") || text.contains("space harbor"),
        "sunburst should reach the opening room:\n{text}\nstderr:\n{err}"
    );
}
