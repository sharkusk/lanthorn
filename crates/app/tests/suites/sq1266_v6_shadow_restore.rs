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
//! # What a keypress-driven story can be asked: nothing (SQ-1349)
//!
//! `journey-r83-s890706.z6` has **no line prompt at all**. Its `pending_input` is `Char` from
//! the splash onward — the party is driven entirely from menus — so there is no moment in that
//! story at which "what would happen if I typed `look`" is a question it can be put. Two
//! consequences shape this suite, and neither was here the first time round.
//!
//! **The seam declines rather than guesses.** [`app::probe::ShadowProbe::snapshot_from`] now
//! refuses any story that is not waiting for a `Line`, before it pays for a snapshot, so the
//! Journey case below asserts a REFUSAL: the twin takes the live snapshot, and the probe then
//! answers nothing, types nothing into a shadow and sends nothing to the worker. It used to
//! assert a non-empty reply, and got one for the wrong reason — the shadow typed `look` at
//! Journey's menu, an intro page turned under the `l`, and the page came back looking like an
//! answer to the command. When SQ-1270 landed, `l` stopped turning the page (the older route
//! delivered the line's TERMINATOR, and Enter does turn it), the reply went empty, and the case
//! failed on an assertion that had never been measuring the restore. Nothing ever reached a
//! screen either way: a shadow's output does not leave it.
//!
//! **The splash dismissal spells `submit_line_with_terminator`, not `submit`.** Since SQ-1270
//! `GameSession::submit` routes by `pending_input`, so a line handed to it at a `read_char`
//! prompt is delivered as one keypress and never reaches `zvm`'s `supply_line` — which is the
//! entire path SQ-1266 corrupted the header on. Calling the line entry point directly is what
//! keeps the release-word assertion below able to fail: with `zvm`'s guard disabled it reads
//! 27759, which is `0x6C6F`, which is `"lo"`.
//!
//! # Fixtures
//!
//! `stories/` is gitignored, so every case here skips vacuously when its file is absent.
//! `advent.z6` is the reported specimen; `journey-r83-s890706.z6` (release 83, serial 890706 —
//! the release `real_media_releases.rs` pins) is a second, unrelated v6 press whose opening is
//! also a keypress — the point of the pair is that this was never one story's quirk.

use std::path::PathBuf;
use std::sync::Arc;

use app::engine::Engine;
use app::probe::{ShadowProbe, ShadowRecipe};
use app::session::{GameSession, InputKind};

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
/// blank-line splash dismissal, which is the host behaviour that used to do the damage.
///
/// The dismissal goes through [`GameSession::submit_line_with_terminator`] rather than
/// `submit` (SQ-1349): `submit` has routed a line away from a `read_char` prompt since
/// SQ-1270, so it can no longer put a LINE to a keypress suspension, and the header
/// assertion in `restores_on` would have nothing left to catch.
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
    // Three blank lines and a `look`: enough to clear `advent.z6`'s title card and reach its
    // ordinary line prompt, enough to reach Journey's first menu question (that story never
    // leaves the keypress interface — see the module header), and — before the fix — enough to
    // have written four commands' worth of letters over the header.
    for _ in 0..3 {
        let _ = s.submit_line_with_terminator("", 13);
    }
    let _ = s.submit_line_with_terminator("look", 13);
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

/// The two things SQ-1266 broke that every fixture here shares: the header, then the restore.
/// What the SEAM then does with the live session differs by whether the story can be asked a
/// line at all, so that third act belongs to each case (SQ-1349).
///
/// `waiting` is the prompt kind the fixture must be sitting at when it hands its snapshot over
/// — a non-vacuity guard, because everything each case asserts afterwards is chosen for that
/// kind, and a fixture that quietly stops sitting at it must be re-read rather than asked the
/// wrong sort of question.
fn restores_on(name: &str, waiting: InputKind) -> Option<(Vec<u8>, GameSession)> {
    let bytes = story(name)?;
    assert_eq!(bytes[0], 6, "{name} must be a Version 6 story for this case to mean anything");
    let static_release = ((bytes[2] as u16) << 8) | bytes[3] as u16;

    let live = live_at_a_prompt(&bytes);
    assert_eq!(
        live.pending_input(),
        waiting,
        "{name}: the live session must hand its snapshot over at a {waiting:?} prompt"
    );
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

    Some((bytes, live))
}

/// `advent.z6` reaches an ordinary line prompt, so the whole seam runs: the worker's own boot,
/// restore and command, and a reply that came from the story rather than from a page turning.
#[test]
fn the_shadow_restores_and_answers_on_advent_z6() {
    let Some((bytes, live)) = restores_on("advent.z6", InputKind::Line) else { return };

    let mut probe = ShadowProbe::default();
    probe.arm(recipe(&bytes));
    let live: Box<dyn Engine> = Box::new(live);
    probe
        .ask(&*live, &["look".to_string()])
        .expect("advent.z6: an armed probe asks a story that is waiting for a line");
    let answer = probe.settle().expect("advent.z6: the worker answered");
    let run =
        answer.run.expect("advent.z6: `Answer::run` was None — the shadow refused the restore");
    assert_eq!(run.steps.len(), 1, "advent.z6: one command asked, one step back");
    assert!(!run.steps[0].reply.trim().is_empty(), "advent.z6: the shadow answered something");
    assert!(
        !run.steps[0].quit && !run.steps[0].escaped,
        "advent.z6: and neither quit nor escaped"
    );
}

/// Journey r83 never leaves its menus, so the seam has no question for it and says so —
/// synchronously, without a snapshot, without a worker round trip, and without typing anything
/// into a shadow. See the module header for what this case used to assert instead.
#[test]
fn the_shadow_declines_to_ask_menu_driven_journey_r83_a_line() {
    let Some((bytes, live)) = restores_on("journey-r83-s890706.z6", InputKind::Char) else {
        return;
    };

    let mut probe = ShadowProbe::default();
    probe.arm(recipe(&bytes));
    let live: Box<dyn Engine> = Box::new(live);
    assert!(probe.is_armed(), "journey: armed, so the prompt kind is the only refusal in play");
    assert_eq!(
        probe.ask(&*live, &["look".to_string()]),
        None,
        "journey: a story that takes no line cannot be asked one"
    );
    assert!(!probe.is_busy(), "journey: and nothing went to the worker to wait on");
    assert_eq!(probe.probes, 0, "journey: no command was ever typed into a shadow");
}
