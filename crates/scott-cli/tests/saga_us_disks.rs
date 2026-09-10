//! End-to-end: point the compiled `scott-cli` at a US S.A.G.A. release disk —
//! Atari 8-bit `.atr`, Apple II DOS 3.3 `.dsk` — and let it mount a database
//! off it (SQ-1470).
//!
//! Mirrors `c64_mysterious_disks.rs`'s own shape: spawn the real binary with
//! stdin piped (never a terminal, so the non-interactive `--story` path is
//! what runs), and read back what it printed. `stories/` is gitignored, so
//! every case skips vacuously when its fixture is absent.

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

/// The gitignored `stories/scott-dialects/<platform>` tree, two levels up
/// from this crate.
fn story_path(platform: &str, name: &str) -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("stories/scott-dialects")
        .join(platform)
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
    let d =
        std::env::temp_dir().join(format!("scott-cli-saga-us-{tag}-{}-{nth}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The Atari 8-bit side A is the database side (spec §7.3/§12.3) and is not a
/// directory entry at all — it opens through `blorb::atr::IMAGE_ENTRY`, and
/// this is the spawn test that reaches its first prompt through that door.
#[test]
fn voodoo_castle_atari_side_a_opens_to_the_first_prompt() {
    let Some(image) = story_path("atari", "SAGA #4 - Voodoo Castle [side A].atr") else { return };
    let dir = scratch_dir("voodoo-atari");
    let out = run(&image, &[], "", &dir);
    let text = stdout_of(&out);
    assert!(
        text.contains("Opening 1) Voodoo Castle (Atari 8-bit)"),
        "content-identified, platform-qualified title:\n{text}"
    );
    assert!(text.contains("I'm in a chapel"), "Voodoo Castle's own opening room:\n{text}");
    assert!(text.contains("Tell me what to do ?"), "reaches the first prompt:\n{text}");
    assert!(!stderr_of(&out).contains("Error"), "must not refuse: {}", stderr_of(&out));
    assert!(!stderr_of(&out).contains("panic"), "must not panic: {}", stderr_of(&out));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Atari side B is the companion PICTURE disk (spec §7.3): no database, so no
/// Scott candidate and a clean refusal, never a panic.
#[test]
fn voodoo_castle_atari_side_b_is_a_named_refusal() {
    let Some(image) = story_path("atari", "SAGA #4 - Voodoo Castle [side B].atr") else { return };
    let dir = scratch_dir("voodoo-atari-b");
    let out = run(&image, &[], "", &dir);
    assert!(!out.status.success(), "the picture side holds no database");
    let err = stderr_of(&out);
    assert!(err.contains("no Scott Adams program on this disk image"), "named refusal:\n{err}");
    assert!(!err.contains("panic"), "must not panic:\n{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Mission Impossible's Atari side A is a damaged specimen — its pointer
/// tables do not resolve (`scott::saga_us::parse_saga_us`'s own doc) — so it
/// refuses with a named error, never a panic, once the mount tries to load it.
#[test]
fn mission_impossible_atari_side_a_refuses_without_panic() {
    let Some(image) = story_path("atari", "SAGA #3 - Mission Impossible [side A].atr") else {
        return;
    };
    let dir = scratch_dir("mission-impossible-atari");
    let out = run(&image, &[], "", &dir);
    assert!(!out.status.success(), "a damaged database must not open");
    let err = stderr_of(&out);
    assert!(err.contains("invalid Scott game data"), "named refusal:\n{err}");
    assert!(!err.contains("panic"), "must not panic:\n{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The Apple II boot side's database is an ordinary DOS 3.3 catalogue file
/// (`A4.DAT`) rather than a reserved whole-image door, and opens to the SAME
/// first room as the Atari release above — both are release build 119 of
/// adventure 4 (spec §12.12's per-release table).
#[test]
fn voodoo_castle_apple_ii_boot_side_opens_to_the_same_first_room() {
    let Some(image) = story_path(
        "apple",
        "Scott Adams Graphic Adventure 4 - Voodoo Castle v2.1-119 (4am crack) side B (boot).dsk",
    ) else {
        return;
    };
    let dir = scratch_dir("voodoo-apple");
    let out = run(&image, &[], "", &dir);
    let text = stdout_of(&out);
    assert!(
        text.contains("Opening 1) Voodoo Castle (Apple II)"),
        "content-identified, platform-qualified title:\n{text}"
    );
    assert!(text.contains("I'm in a chapel"), "the same room the Atari release opens on:\n{text}");
    assert!(text.contains("Tell me what to do ?"), "reaches the first prompt:\n{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two DIFFERENT Apple II titles — *The Count* and *The Sorcerer of
/// Claymorgue Castle* — both name their database file `DATABASE` (spec
/// §7.4), and both boot disks live in the same `stories/scott-dialects/apple/`
/// directory, so a save key built from the container entry NAME alone would
/// collide. Content-identified titles read correctly for each, and the two
/// `/save`s land in two DIFFERENT directories under one shared `--data-dir`
/// (SQ-1470).
#[test]
fn the_count_and_claymorgue_castle_do_not_share_a_save_key() {
    let Some(count) = story_path(
        "apple",
        "Scott Adams Graphic Adventure 5 - The Count v2.1-115 (4am crack) side B - boot.dsk",
    ) else {
        return;
    };
    let Some(claymorgue) = story_path(
        "apple",
        "Scott Adams Graphic Adventure 13 - The Sorcerer of Claymorgue Castle v2.2-122 (4am crack) side B (boot).dsk",
    ) else {
        return;
    };
    let dir = scratch_dir("database-collision");

    let count_out = run(&count, &[], "/save\nx\n", &dir);
    assert!(
        stdout_of(&count_out).contains("Opening 1) The Count (Apple II)"),
        "{}",
        stdout_of(&count_out)
    );

    let claymorgue_out = run(&claymorgue, &[], "/save\nx\n", &dir);
    assert!(
        stdout_of(&claymorgue_out).contains("Opening 1) The Sorcerer of Claymorgue Castle (Apple II)"),
        "{}",
        stdout_of(&claymorgue_out)
    );

    let saves: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(".save")))
        .collect();
    assert_eq!(saves.len(), 2, "two distinct save directories, not one shared 'DATABASE.save': {saves:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
