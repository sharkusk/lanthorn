//! SQ-1417, closing SQ-0229: cross-interpreter `@save`/`@restore` interop.
//!
//! SQ-0229 deferred this twice for want of a headless reference interpreter
//! and an observable-state fixture (see `GLULX_NOTES.md` §14's "Foreign-save
//! interop" note for the full history). Both now exist: glulxe 0.6.1 built
//! from source against cheapglk 1.0.7 (the same oracle
//! `glk_conformance_corpus.rs` uses — see its module doc), and the vendored
//! `unit_tests/statusbufferwin.ulx` corpus story, whose carried-items state
//! is directly observable through its standard-library INVENTORY/TAKE verbs.
//!
//! `tests/fixtures/statusbufferwin_apple.{glulxe,gvm}.glksave` are `FORM
//! IFZS` (Glulx-Quetzal) saves of the identical state — `take apple` then
//! `save` — one written by each interpreter. This file automates the
//! direction that can run in the normal test suite: gvm restoring glulxe's
//! save. The reverse (glulxe restoring gvm's save) was verified manually,
//! since glulxe/cheapglk are an external oracle built once for this audit,
//! not a workspace dependency — see the GLULX_NOTES.md note for the exact
//! commands and output.
//!
//! `unit_tests/` is gitignored (like `stories/`); this test skips vacuously
//! when its fixture is missing, the same pattern the rest of this crate's
//! corpus tests use.

use std::path::PathBuf;

use gvm::{Machine, Memory, StepResult, TestBackend};

fn unit_tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../unit_tests")
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

const MAX_STEPS: u64 = 50_000_000;

/// Restoring a glulxe-written save into gvm — the direction this suite can
/// automate. Restores `statusbufferwin_apple.glulxe.glksave` (glulxe's own
/// save of `take apple` then `save`) into a freshly booted `statusbufferwin`
/// machine, then — per this project's restore-testing convention ("Restore
/// tests must perturb before asserting", `CLAUDE.md` Testing conventions;
/// asserting immediately after a restore is when everything still looks
/// correct even if the restore is subtly wrong) — issues one more command
/// (`inventory`) and asserts the apple shows up in its response, not just
/// somewhere in the whole transcript.
#[test]
fn restores_a_glulxe_written_save_and_the_apple_is_there() {
    let story_path = unit_tests_dir().join("statusbufferwin.ulx");
    let Ok(image) = std::fs::read(&story_path) else {
        eprintln!("skipping: {} not vendored (gitignored fixture)", story_path.display());
        return;
    };
    let save_path = fixtures_dir().join("statusbufferwin_apple.glulxe.glksave");
    let save_blob = std::fs::read(&save_path).unwrap_or_else(|e| panic!("read {}: {e}", save_path.display()));

    let mem = Memory::new(image).expect("valid Glulx image");
    let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
    // The script: ask the story to restore, then (after the restore resumes
    // execution) issue one perturbing command.
    let commands = ["restore", "inventory"];
    let mut next = 0usize;
    let mut steps = 0u64;
    let mut restored = false;
    loop {
        steps += 1;
        assert!(steps < MAX_STEPS, "runaway, {MAX_STEPS} steps without reaching Quit");
        match m.step() {
            StepResult::Continue => {}
            StepResult::NeedLine { .. } => {
                if next >= commands.len() {
                    break;
                }
                let cmd = commands[next];
                next += 1;
                m.supply_line(cmd);
            }
            StepResult::NeedFilename { .. } => {
                // The player's response to "Enter saved game to load: " — any
                // name works here since we hand the bytes to
                // `complete_restore_quetzal` directly rather than resolving a
                // real fileref.
                m.supply_filename(Some("save".to_string()));
            }
            StepResult::RestoreRequest => {
                assert!(m.complete_restore_quetzal(&save_blob), "glulxe's save failed to restore in gvm");
                restored = true;
            }
            StepResult::Quit => break,
            StepResult::Fault => panic!("unexpected VM fault: {:?}", m.diagnostics()),
            other => panic!("unexpected suspension {other:?}"),
        }
    }

    assert!(restored, "never reached a RestoreRequest — the story's own restore path didn't run");
    let text = m.backend_mut().as_any_mut().downcast_mut::<TestBackend>().unwrap().all_text();
    // The restore's own turn already reports "Ok." then the current
    // inventory (statusbufferwin prints it after every command); the
    // explicit "inventory" perturbation below asks again on a LATER turn, so
    // this isn't just reading the restore's own immediate echo.
    let last_inventory = text.rsplit(">Ok.").next().unwrap_or(&text);
    assert!(
        last_inventory.contains("an apple") || last_inventory.contains("the apple"),
        "restored state doesn't show the apple taken before the save; got:\n{text}"
    );
}

/// Sanity companion to the manual glulxe-side verification recorded in
/// `GLULX_NOTES.md`: gvm's OWN save restores in gvm too, so the fixture pair
/// really does represent the same state on both sides rather than gvm's save
/// being unreadable by anything (including itself). This is not a
/// substitute for the manual glulxe-restores-gvm's-save check — it can't be,
/// since a bug shared between gvm's writer and reader would pass here and
/// nowhere else — it only guards the fixture file itself against bit rot.
#[test]
fn gvms_own_save_of_the_same_state_also_restores_in_gvm() {
    let story_path = unit_tests_dir().join("statusbufferwin.ulx");
    let Ok(image) = std::fs::read(&story_path) else {
        eprintln!("skipping: {} not vendored (gitignored fixture)", story_path.display());
        return;
    };
    let save_path = fixtures_dir().join("statusbufferwin_apple.gvm.glksave");
    let save_blob = std::fs::read(&save_path).unwrap_or_else(|e| panic!("read {}: {e}", save_path.display()));

    let mem = Memory::new(image).expect("valid Glulx image");
    let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
    let commands = ["restore", "inventory"];
    let mut next = 0usize;
    let mut steps = 0u64;
    loop {
        steps += 1;
        assert!(steps < MAX_STEPS, "runaway, {MAX_STEPS} steps without reaching Quit");
        match m.step() {
            StepResult::Continue => {}
            StepResult::NeedLine { .. } => {
                if next >= commands.len() {
                    break;
                }
                let cmd = commands[next];
                next += 1;
                m.supply_line(cmd);
            }
            StepResult::NeedFilename { .. } => m.supply_filename(Some("save".to_string())),
            StepResult::RestoreRequest => {
                assert!(m.complete_restore_quetzal(&save_blob), "gvm's own save failed to restore in gvm");
            }
            StepResult::Quit => break,
            StepResult::Fault => panic!("unexpected VM fault: {:?}", m.diagnostics()),
            other => panic!("unexpected suspension {other:?}"),
        }
    }
    let text = m.backend_mut().as_any_mut().downcast_mut::<TestBackend>().unwrap().all_text();
    let last_inventory = text.rsplit(">Ok.").next().unwrap_or(&text);
    assert!(
        last_inventory.contains("an apple") || last_inventory.contains("the apple"),
        "restored state doesn't show the apple taken before the save; got:\n{text}"
    );
}
