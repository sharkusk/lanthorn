//! SQ-1266: the fork-and-probe shadow must restore a Version 6 session's snapshot.
//!
//! # The symptom
//!
//! Two lanes reported the same thing from opposite ends. SQ-1264 could not exercise Phase 2 of
//! the random-exit search on `advent.z6` at all — `GameSession::restore_state` answered
//! `BadSave("SaveMismatch")` every time — and SQ-1269 saw every probe on that story come back
//! with `Answer::run: None` and had to give `deliver_suspicion` a no-evidence path to fall
//! through. Both blamed the story: SQ-1264's note called it "a V6Lib private beta test compile
//! whose OWN init code writes a runtime-random value into the header's release-number field",
//! because the banner really does read a different `Release NNN` on each run.
//!
//! # What it actually was
//!
//! Ours, in `zvm`, and nothing to do with `advent.z6` in particular. `Machine::supply_line` did
//! not check that the suspension it was completing was a `read` at all. A `read_char` leaves
//! `PendingInput`'s `text_buf` and `parse_buf` at zero, so answering a keypress prompt with a
//! line wrote the v5+ layout — count byte at `text_buf + 1`, text from `text_buf + 2` — to
//! ABSOLUTE addresses 1 and 2: the header's Flags1 and its RELEASE NUMBER word. Quetzal's IFhd
//! validates a save's release against CURRENT memory (§5.8), so the moment a session did that,
//! every restore of its snapshot into a separately booted twin failed. The "randomness" was the
//! first command's own letters landing on the release word — `advent.z6` at the hill read
//! release 27759, which is `0x6C6F`, which is `"lo"`.
//!
//! And a Version 6 title splash is a `read_char`, so this was reachable by any host that
//! dismissed one with a line. Measured across `stories/` before the fix (`app::probe::ask` +
//! `settle` after a three-blank-line opening and one `look`): **eighteen of the nineteen** v6
//! stories present came back `run: None`, against `Some` for `advent.z8` (v8) and
//! `zork1-r88-s840726.z3` (v3), whose openings are line reads and which therefore never took
//! the damage. That is the whole seam — return probes, vocabulary vetting, every random-exit
//! probe — silently dead on Version 6.
//!
//! The fix is `zvm`'s: `PendingInput::line_read` states which instruction suspended instead of
//! leaving it to be inferred from `text_buf == 0`, and `supply_line` delivers the terminator
//! through `supply_char` and touches no memory when the suspension is a `read_char`. That was
//! already the only OBSERVABLE half of what it did (`do_store(store_var, terminator)`), so the
//! memory writes were pure damage. `zvm`'s own
//! `a_line_supplied_to_a_read_char_stores_the_key_and_writes_no_memory` pins the unit; this
//! suite pins the consequence the seam cares about, on real Version 6 stories.
//!
//! # Fixtures
//!
//! `stories/` is gitignored, so every case here skips vacuously when its file is absent.
//! `advent.z6` is the reported specimen; `journey-r83-s890706.z6` (release 83, serial 890706) is
//! a second, unrelated v6 press whose opening is also a keypress — the point of the pair is that
//! this was never one story's quirk.

use std::path::PathBuf;
use std::sync::Arc;

use app::engine::Engine;
use app::probe::{ShadowProbe, ShadowRecipe};
use app::session::GameSession;

use crate::fixture_paths::fixture_path;

fn story(name: &str) -> Option<Vec<u8>> {
    match std::fs::read(fixture_path(name)) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", fixture_path(name).display());
            None
        }
    }
}

/// Boot the way `sq1264_forest_randomization.rs`'s `ZPlay::advent` boots — including its
/// `submit("")` splash dismissal, which is the host behaviour that used to do the damage.
fn live_at_a_prompt(bytes: &[u8]) -> GameSession {
    let mut s = GameSession::new_with_trace(
        bytes.to_vec(),
        true,
        false,
        None,
        false,
        Vec::new(),
        None,
        None,
        Some((25, 80)),
    )
    .expect("the story boots without a ZError");
    s.set_strip_prompt(false);
    // Three blank lines and a `look`: enough to clear a title card and reach ordinary play on
    // both fixtures, and — before the fix — enough to have written four commands' worth of
    // letters over the header.
    for _ in 0..3 {
        let _ = s.submit("");
    }
    let _ = s.submit("look");
    s
}

fn release(s: &GameSession) -> u16 {
    ((s.machine.mem.read_byte(0x02) as u16) << 8) | s.machine.mem.read_byte(0x03) as u16
}

fn recipe(bytes: &[u8]) -> ShadowRecipe {
    ShadowRecipe {
        story_bytes: Arc::new(bytes.to_vec()),
        store: PathBuf::new(),
        vfs_bytes: Arc::new(Vec::new()),
        honor_game_colours: true,
        interpreter_number: None,
        random_seed: None,
        acceleration: true,
        screen: (80, 24),
    }
}

/// The three things SQ-1266 broke, in the order they break: the header, then the restore, then
/// the probe. Asserting all three together is deliberate — the first is the cause and the last
/// is the symptom the two lanes actually reported, and a case that pinned only the last would
/// pass again for the wrong reason the next time a shadow learns to boot around it.
fn probe_answers_on(name: &str) {
    let Some(bytes) = story(name) else { return };
    assert_eq!(bytes[0], 6, "{name} must be a Version 6 story for this case to mean anything");
    let static_release = ((bytes[2] as u16) << 8) | bytes[3] as u16;

    let live = live_at_a_prompt(&bytes);
    assert_eq!(
        release(&live),
        static_release,
        "{name}: the header's release word must still be the file's own after a splash \
         dismissed with a line — a different value here is `supply_line` writing to address 2"
    );

    // The restore itself, outside the worker thread, so a failure names its own error.
    let mut twin = GameSession::new_with_trace(
        bytes.clone(),
        true,
        false,
        None,
        false,
        Vec::new(),
        None,
        None,
        None,
    )
    .expect("the twin boots");
    assert!(
        twin.restore_state(&live.save_state()).is_ok(),
        "{name}: a freshly booted twin must take the live snapshot (was SaveMismatch)"
    );

    // And the real seam: the worker's own boot, restore and command.
    let mut probe = ShadowProbe::default();
    probe.arm(recipe(&bytes));
    let live: Box<dyn Engine> = Box::new(live);
    probe.ask(&*live, &["look".to_string()]).unwrap_or_else(|| panic!("{name}: the probe armed"));
    let answer = probe.settle().unwrap_or_else(|| panic!("{name}: the worker answered"));
    let run = answer
        .run
        .unwrap_or_else(|| panic!("{name}: `Answer::run` was None — the shadow refused the restore"));
    assert_eq!(run.steps.len(), 1, "{name}: one command asked, one step back");
    assert!(!run.steps[0].reply.trim().is_empty(), "{name}: the shadow printed something");
    assert!(!run.steps[0].quit && !run.steps[0].escaped, "{name}: and neither quit nor escaped");
}

#[test]
fn the_shadow_restores_and_answers_on_advent_z6() {
    probe_answers_on("advent.z6");
}

#[test]
fn the_shadow_restores_and_answers_on_journey_r83() {
    probe_answers_on("journey-r83-s890706.z6");
}
