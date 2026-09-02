//! Full-story acceleration on/off equivalence + speed proof.
//!
//! These tests load real Inform 7 Glulx stories from the gitignored `stories/`
//! directory (a local symlink present in dev worktrees but absent in CI and
//! fresh clones). They are the primary anti-divergence guarantee: with
//! acceleration ON, the transcript to the first input prompt must be
//! byte-identical to acceleration OFF, and the opcode count must drop
//! substantially (accelerated calls bypass `step_once`, so their opcodes
//! vanish from `insn_count`).
//!
//! Reaching that prompt means being a host, not just a stepper: a game may run
//! its OWN fixed-name `@save`/`@restore` during startup (Counterfeit Monkey
//! writes a `_Counterfeit_Monkey-startup-data` init cache before it ever asks
//! for input), and those are serviced here the way `app`'s
//! `glulx_session::drive_auto` services one against a read-only store — a clean
//! failure, nothing written. Only the *player's* prompted SAVE/RESTORE verb is
//! a defect this early.
//!
//! Because the story assets aren't committed, both tests here are `#[ignore]`d
//! so the default `cargo test -p gvm` tier stays green without them. Run
//! manually with:
//!
//! ```sh
//! cargo test -p gvm --test accel_story_equivalence -- --ignored --nocapture
//! ```
//!
//! (add `--release` if the debug build is too slow for CounterfeitMonkey).

use std::path::PathBuf;

use gvm::{Machine, Memory, StepResult, TestBackend};

/// A story's own startup output, used as a non-vacuity anchor: the transcript
/// compared on/off must actually contain this, so a boot that stops early (or a
/// harness that reaches the prompt without the game having spoken) fails loudly
/// instead of comparing two empty strings.
const CM_OPENING: &str = "Can you hear me?";
/// The same anchor for `TAKE.gblorb` (see [`CM_OPENING`]).
const TAKE_OPENING: &str = "One joke, until expiration, by Amelia Pinnolla";

/// The repo-root `stories/` directory (gitignored symlink), resolved relative
/// to this crate's manifest so the tests work regardless of `cargo test`'s
/// working directory.
fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Ceiling on opcode steps while driving to the first prompt. If a story
/// never reaches a `NeedLine`/`NeedChar` within this many steps, something is
/// wrong (infinite loop, VM bug, or a story that never asks for input) —
/// panic loudly rather than hang the test run.
const MAX_STEPS: u64 = 100_000_000;

/// Extract the Glulx executable from Blorb-wrapped `bytes` (or pass through
/// plain `.ulx` bytes unchanged), mirroring `gvm-cli`'s `extract_executable`.
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

/// Build a machine over `image`, set acceleration to `accel`, and drive it to
/// the first input prompt (`NeedLine` or `NeedChar`). Returns the full
/// text-buffer transcript captured to that point, the opcode count, and the
/// game-managed `@save`/`@restore` requests serviced on the way (acceleration
/// must not change *those* either).
fn run_to_first_prompt(image: Vec<u8>, accel: bool) -> (String, u64, Vec<String>) {
    let mem = Memory::new(image).expect("valid Glulx image");
    let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
    m.set_acceleration(accel);

    let mut steps = 0u64;
    let mut saveloads = Vec::new();
    loop {
        match m.step() {
            StepResult::Continue => {
                steps += 1;
                assert!(
                    steps < MAX_STEPS,
                    "runaway: did not reach the first input prompt within {MAX_STEPS} steps (accel={accel})"
                );
            }
            StepResult::NeedLine { .. } | StepResult::NeedChar { .. } => break,
            StepResult::NeedEvent { timer_ms: Some(_), .. } => m.deliver_timer(),
            StepResult::NeedEvent { .. } => {
                panic!("unexpected non-timer event wait before the first input prompt (accel={accel})")
            }
            StepResult::Quit => panic!("story quit before reaching an input prompt (accel={accel})"),
            // A game's OWN fixed-name save is startup, not a defect: Counterfeit
            // Monkey `@save`s a `_Counterfeit_Monkey-startup-data` init cache
            // before it ever asks for input, so a host that panics here can
            // never reach the story's first prompt. Answer it exactly the way
            // `app`'s `glulx_session::drive_auto` answers one against a
            // read-only store — a clean failure, nothing written — which is the
            // answer any story's very first run already gets and handles. Only
            // the PLAYER's prompted SAVE/RESTORE verb is unexpected this early.
            StepResult::SaveRequest | StepResult::RestoreRequest => {
                let req = m.pending_saveload_request().unwrap_or_default();
                assert!(
                    !req.by_prompt,
                    "unexpected player-prompted @save/@restore before the first input prompt (accel={accel})"
                );
                saveloads.push(format!(
                    "{} {}",
                    if req.restore { "restore" } else { "save" },
                    req.name
                ));
                if req.restore {
                    m.complete_restore_failure();
                } else {
                    m.complete_save(false);
                }
            }
            StepResult::NeedFilename { .. } => {
                panic!("unexpected filename prompt before the first input prompt (accel={accel})")
            }
        }
    }

    let text = m
        .backend_mut()
        .as_any_mut()
        .downcast_mut::<TestBackend>()
        .unwrap()
        .all_text();
    (text, m.insn_count(), saveloads)
}

/// Load and run one on/off equivalence + speed comparison for the Blorb at
/// `path`, returning `(ops_on, ops_off)` after asserting transcript equality
/// and a material opcode reduction. `opening` is a substring the story is known
/// to print before its first prompt — the non-vacuity guard, without which two
/// runs that both stopped early would compare equal and pass.
fn check_equivalence_and_speed(name: &str, opening: &str) -> (u64, u64) {
    let path = stories_dir().join(name);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("local story (gitignored) missing at {}: {e}", path.display()));
    let image = extract_glulx(bytes);

    let (out_on, ops_on, saves_on) = run_to_first_prompt(image.clone(), true);
    let (out_off, ops_off, saves_off) = run_to_first_prompt(image, false);

    assert_eq!(out_on, out_off, "acceleration changed the transcript to first prompt for {name}");
    assert_eq!(
        saves_on, saves_off,
        "acceleration changed the game-managed @save/@restore sequence for {name}"
    );
    assert!(
        out_on.contains(opening),
        "{name} reached a prompt without printing {opening:?} — the transcript comparison is \
         vacuous; got {out_on:?}"
    );
    assert!(
        ops_on * 3 < ops_off,
        "accel not materially faster for {name}: on={ops_on} off={ops_off}"
    );

    (ops_on, ops_off)
}

/// The headline proof: CounterfeitMonkey (a large, real-world Inform 7 game)
/// produces an identical transcript to first prompt with acceleration on vs.
/// off, and acceleration cuts the dispatched-opcode count by more than 3x
/// (Task 0's baseline measured ~88.8% of interpreted opcodes inside
/// accel-candidate functions).
#[test]
#[ignore = "needs local gitignored stories/CounterfeitMonkey-11.gblorb; run with `cargo test -p gvm --test accel_story_equivalence -- --ignored`"]
fn counterfeit_monkey_accel_matches_interpreted_and_is_faster() {
    let (ops_on, ops_off) = check_equivalence_and_speed("CounterfeitMonkey-11.gblorb", CM_OPENING);
    eprintln!("CounterfeitMonkey-11: ops_on={ops_on} ops_off={ops_off} ratio={:.2}x", ops_off as f64 / ops_on as f64);
}

/// A smaller, faster secondary confirmation on another Inform Glulx title
/// present under `stories/` — the same on/off transcript equivalence check,
/// without the speed margin assertion (a tiny story may not do enough
/// accel-eligible work to clear the 3x bar, but transcript identity must
/// still hold).
#[test]
#[ignore = "needs local gitignored stories/TAKE.gblorb; run with `cargo test -p gvm --test accel_story_equivalence -- --ignored`"]
fn take_accel_matches_interpreted() {
    let path = stories_dir().join("TAKE.gblorb");
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("local story (gitignored) missing at {}: {e}", path.display()));
    let image = extract_glulx(bytes);

    let (out_on, ops_on, saves_on) = run_to_first_prompt(image.clone(), true);
    let (out_off, ops_off, saves_off) = run_to_first_prompt(image, false);

    assert_eq!(out_on, out_off, "acceleration changed the transcript to first prompt for TAKE.gblorb");
    assert_eq!(
        saves_on, saves_off,
        "acceleration changed the game-managed @save/@restore sequence for TAKE.gblorb"
    );
    assert!(
        out_on.contains(TAKE_OPENING),
        "TAKE.gblorb reached a prompt without printing {TAKE_OPENING:?} — the transcript \
         comparison is vacuous; got {out_on:?}"
    );
    eprintln!("TAKE: ops_on={ops_on} ops_off={ops_off}");
}
