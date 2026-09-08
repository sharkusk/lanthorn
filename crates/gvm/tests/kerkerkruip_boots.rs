//! Real-game smoke: Kerkerkruip boots past its external-storage menu (SQ-0277).
//!
//! Kerkerkruip's `KerkerkruipStorage` extension opens a Glk file stream at
//! startup to load preferences. Before SQ-0277 (`gvm` had no Glk filerefs or
//! file streams), `glk_stream_open_file` returned NULL, the storage extension
//! errored (`*** Error on file 'KerkerkruipStorage' ***`), and the game looped
//! on that error instead of ever reaching a stable input prompt. With the
//! in-memory VFS in place, a missing file opened for reading degrades cleanly
//! to "no saved preferences", so the game boots to its main menu and asks for
//! input.
//!
//! This test drives the real `stories/Kerkerkruip.gblorb` headlessly to its
//! first Glk input request and asserts that no external-file error surfaced on
//! the way there. The story asset is gitignored (a large local-only blorb), so
//! this test **skips cleanly** when it is absent — CI and fresh clones stay
//! green, dev worktrees get the real coverage. Mirrors the skip-if-absent
//! pattern in `crates/app/tests/wizard_sniffer.rs` and the gvm blorb loader in
//! `crates/gvm/tests/accel_story_equivalence.rs`.

use std::path::PathBuf;

use gvm::{Machine, Memory, StepResult, TestBackend};

/// Opcode-step ceiling while driving to the first prompt. Kerkerkruip's Inform 7
/// initialisation is heavy; with acceleration on (the default) it settles well
/// under this. If it never reaches an input request within this budget,
/// something is wrong (regression, VM bug, or the storage loop returned) — fail
/// loudly rather than hang.
const MAX_STEPS: u64 = 100_000_000;

/// The repo-root `stories/` directory (a gitignored symlink in dev worktrees,
/// absent in CI), resolved relative to this crate's manifest.
fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Extract the Glulx executable from a Blorb-wrapped image (or pass through a
/// plain `.ulx`), mirroring `gvm-cli`'s `extract_executable`.
fn extract_glulx(bytes: Vec<u8>) -> Vec<u8> {
    if !blorb::Blorb::is_blorb(&bytes) {
        return bytes;
    }
    let b = blorb::Blorb::parse(bytes).expect("valid Blorb");
    match b.executable() {
        Ok((blorb::ExecKind::Glulx, data)) => data.to_vec(),
        Ok((blorb::ExecKind::ZCode | blorb::ExecKind::Scott, _)) => {
            panic!("expected a Glulx Blorb")
        }
        Err(e) => panic!("Blorb has no executable: {e:?}"),
    }
}

#[test]
#[ignore = "slow (~1.5s): boots Kerkerkruip past its storage menu; run with --include-ignored"]
fn kerkerkruip_boots_past_storage_menu() {
    let path = stories_dir().join("Kerkerkruip.gblorb");
    let Ok(raw) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return;
    };

    let image = extract_glulx(raw);
    let mem = Memory::new(image).expect("valid Glulx image");
    let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));

    // Drive to the first Glk input request (line or char). Reaching *any* input
    // request means the storage extension loaded without looping on its error.
    let mut steps = 0u64;
    let mut reached_input = false;
    let mut quit_before_input = false;
    loop {
        match m.step() {
            StepResult::Continue => {
                steps += 1;
                assert!(
                    steps < MAX_STEPS,
                    "runaway: Kerkerkruip did not reach an input request within {MAX_STEPS} steps \
                     (storage-error loop may have regressed)"
                );
            }
            StepResult::NeedLine { .. } | StepResult::NeedChar { .. } => {
                reached_input = true;
                break;
            }
            StepResult::Quit | StepResult::Fault => {
                quit_before_input = true;
                break;
            }
            StepResult::NeedEvent { timer_ms: Some(_), .. } => m.deliver_timer(),
            StepResult::NeedEvent { .. } => {
                panic!("unexpected non-timer event wait during Kerkerkruip boot");
            }
            StepResult::SaveRequest | StepResult::RestoreRequest | StepResult::NeedFilename { .. } => {
                // The boot path should not hit a host @save/@restore before its
                // first input; treat as a boot failure to investigate.
                panic!("unexpected @save/@restore during Kerkerkruip boot");
            }
            _ => break,
        }
    }

    let text = m
        .backend_mut()
        .as_any_mut()
        .downcast_mut::<TestBackend>()
        .unwrap()
        .all_text();
    let diags = m.diagnostics().join("\n");

    // (c) It did not quit/halt before asking for input.
    assert!(
        !quit_before_input,
        "Kerkerkruip quit before reaching an input request; transcript tail:\n{}",
        tail(&text)
    );
    // (b) It reached an input request.
    assert!(reached_input, "Kerkerkruip never reached a Glk input request");
    // (a) No external-file / storage error surfaced on the way to that prompt —
    // the direct proof SQ-0277 unblocked the KerkerkruipStorage load.
    for hay in [&text, &diags] {
        assert!(
            !hay.contains("P48"),
            "external-file error (P48) surfaced before first input; tail:\n{}",
            tail(hay)
        );
        assert!(
            !hay.contains("KerkerkruipStorage"),
            "KerkerkruipStorage error surfaced before first input; tail:\n{}",
            tail(hay)
        );
    }

    eprintln!(
        "Kerkerkruip booted to first input in {steps} steps; {} transcript chars, {} diagnostics",
        text.chars().count(),
        m.diagnostics().len()
    );
    eprintln!("--- transcript tail ---\n{}", tail(&text));
}

/// Last ~600 chars of a string, for failure/diagnostic context.
fn tail(s: &str) -> String {
    let n = s.chars().count();
    s.chars().skip(n.saturating_sub(600)).collect()
}
