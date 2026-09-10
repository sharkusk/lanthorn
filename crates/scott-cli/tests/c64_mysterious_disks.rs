//! End-to-end: point the compiled `scott-cli` at a Commodore 64 *Mysterious
//! Adventures* compilation disk and let it mount a program off it (SQ-1414).
//!
//! Mirrors `zvm-cli/tests/disk_image.rs`'s own compilation-disk cases: spawn
//! the real binary with stdin piped (never a terminal, so the non-interactive
//! `--story` path is what runs), and read back what it printed. `stories/` is
//! gitignored, so every case skips vacuously when its fixture is absent.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

/// Run `scott-cli <image> [args]` with `stdin_script` piped in, then closed —
/// stdin is never a terminal, which is the non-interactive path under test.
fn run(image: &std::path::Path, extra_args: &[&str], stdin_script: &str, data_dir: &std::path::Path) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_scott-cli"))
        .arg(image)
        .args(extra_args)
        .arg("--data-dir")
        .arg(data_dir)
        .arg("--pager")
        .arg("off")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("scott-cli spawns");
    child.stdin.take().unwrap().write_all(stdin_script.as_bytes()).unwrap();
    child.wait_with_output().expect("scott-cli runs")
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The gitignored `stories/` tree, two levels up from this crate.
fn story_path(name: &str) -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("stories/scott-dialects/c64")
        .join(name);
    if p.exists() {
        return Some(p);
    }
    eprintln!("SKIP: gitignored disk image missing at {}", p.display());
    None
}

/// A fresh, call-unique scratch directory (SQ-1131/SQ-1163: `process::id()`
/// alone is shared by every call in this binary under `cargo test` and every
/// test in this process under nextest).
fn scratch_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NTH: AtomicUsize = AtomicUsize::new(0);
    let nth = NTH.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir()
        .join(format!("scott-cli-c64-mysterious-{tag}-{}-{nth}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// `--story BATON` off `MYSTADV1.D64` drives straight to *The Golden Baton*'s
/// first prompt — the room the disk mount, `scott::c64::parse_c64_mysterious_prg`
/// and this host's own loop all have to agree on.
#[test]
fn story_baton_off_mystadv1_reaches_the_golden_batons_first_prompt() {
    let Some(image) = story_path("MYSTADV1.D64") else { return };
    let dir = scratch_dir("baton");
    let out = run(&image, &["--story", "BATON", "--max-turns", "1"], "", &dir);
    let text = stdout_of(&out);
    assert!(
        text.contains("Opening 1) The Golden Baton  (BATON)"),
        "says which it opened:\n{text}"
    );
    assert!(text.contains("SPOOKY Forest"), "the opening room is the Golden Baton's:\n{text}");
    assert!(text.contains("Old Cloak"), "the opening room's own item:\n{text}");
    assert!(text.contains("Tell me what to do ?"), "reaches the first prompt:\n{text}");
    assert!(!stderr_of(&out).contains("Error"), "must not refuse: {}", stderr_of(&out));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Without `--story` and no terminal on stdin, `MYSTADV1.D64` refuses to guess
/// and lists its six programs — the same shape `zvm-cli`'s own disk menu
/// refuses in, matched through the shared `cli_host::story_pick` rule.
#[test]
fn mystadv1_without_story_lists_its_six_programs_and_refuses_to_guess() {
    let Some(image) = story_path("MYSTADV1.D64") else { return };
    let dir = scratch_dir("menu1");
    let out = run(&image, &[], "", &dir);
    assert!(!out.status.success(), "no terminal, no --story: must not guess");
    let err = stderr_of(&out);
    assert!(
        err.contains("holds 6 Scott Adams programs"),
        "names the count:\n{err}"
    );
    assert!(err.contains("--story <n|name>"), "names the flag:\n{err}");
    for (n, name) in
        ["BATON", "TIME MACHINE", "ARROW I", "ARROW II", "PULSAR 7", "CIRCUS"].iter().enumerate()
    {
        let want = format!("{}) ", n + 1);
        assert!(
            err.lines().any(|l| l.trim_start().starts_with(&want) && l.contains(name)),
            "line {} ({name}) missing from the menu:\n{err}",
            n + 1,
        );
    }
    assert_eq!(err.lines().filter(|l| l.trim_start().starts_with(char::is_numeric)).count(), 6, "exactly six menu lines:\n{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `MYSTADV2.D64` by number, off its own menu — the second disk's five
/// programs, distinct from the first's six.
#[test]
fn mystadv2_without_story_lists_its_five_programs() {
    let Some(image) = story_path("MYSTADV2.D64") else { return };
    let dir = scratch_dir("menu2");
    let out = run(&image, &[], "", &dir);
    assert!(!out.status.success());
    let err = stderr_of(&out);
    assert!(err.contains("holds 5 Scott Adams programs"), "{err}");
    for name in ["EXPERIMENT", "WIZARD OF AKYRZ", "PERSEUS", "INDIANS", "WAXWORKS"] {
        assert!(err.contains(name), "{name} missing from:\n{err}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `QUESTPR1.D64` carries the US S.A.G.A. *Hulk* (`SHULK.DB`) — a different
/// Commodore 64 family from the *Mysterious Adventures* disks above, but one
/// this loader reads since the US S.A.G.A. wiring landed (SQ-1470): the disk
/// holds exactly one Scott row, content-identified as "The Hulk (Commodore
/// 64)" (`scott::SagaUs::display_title`) rather than by any name the disk
/// itself stores it under, and it opens straight to the Hulk's own first
/// room with no `--story` needed (one candidate never asks).
#[test]
fn questpr1_opens_the_us_saga_hulk() {
    let Some(image) = story_path("QUESTPR1.D64") else { return };
    let dir = scratch_dir("questpr1");
    let out = run(&image, &[], "", &dir);
    let text = stdout_of(&out);
    assert!(
        text.contains("Opening 1) The Hulk (Commodore 64)"),
        "content-identified title, platform-qualified:\n{text}"
    );
    assert!(text.contains("Bruce Banner"), "the Hulk's own opening room:\n{text}");
    assert!(text.contains("Tell me what to do ?"), "reaches the first prompt:\n{text}");
    assert!(!stderr_of(&out).contains("Error"), "must not refuse: {}", stderr_of(&out));
    assert!(!stderr_of(&out).contains("panic"), "must not panic: {}", stderr_of(&out));
    let _ = std::fs::remove_dir_all(&dir);
}

/// `QUESTPR3.D64` (*Fantastic Four*) carries a database this loader does not
/// identify at all (`docs/internals/scott-dialects-spec.md` §10.7: "This
/// release's database encoding is unidentified") — a genuine named refusal,
/// listing the disk's files, never a crash.
#[test]
fn questpr3_is_a_named_refusal_not_a_crash() {
    let Some(image) = story_path("QUESTPR3.D64") else { return };
    let dir = scratch_dir("questpr3");
    let out = run(&image, &[], "", &dir);
    assert!(!out.status.success(), "no Scott program on this disk");
    let err = stderr_of(&out);
    assert!(
        err.contains("no Scott Adams program on this disk image"),
        "named refusal, not a crash:\n{err}"
    );
    assert!(err.contains("SAGA.OBJ"), "names the disk's files:\n{err}");
    assert!(!err.contains("panic"), "must not panic:\n{err}");
    let _ = std::fs::remove_dir_all(&dir);
}
