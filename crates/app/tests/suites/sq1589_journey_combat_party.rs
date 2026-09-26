//! Amiga Journey's combat-menu party column reprints one slot with a shorter
//! name over a longer one and must not leave a stray tail behind — SQ-1589.
//!
//! **Repro, from a fresh boot of the Amiga release floppy** (release 30,
//! serial 890322 — see CLAUDE.md's "a disk image is a different release"):
//! tapping through the opening vignettes (blank line at a line prompt, Enter
//! at a char prompt) reaches a scripted combat encounter partway through the
//! intro. Just before the 8th such input, the party column reads
//! `Bergon / Praxix / Esher / Tag` (4 rows); that 8th input resolves the
//! encounter and the party column collapses to `Bergon / Praxix / Tag` (3
//! rows) — "Esher" leaves the party and "Tag" is reprinted at Esher's old
//! screen slot (native x=137, y=337), one row up from where "Tag" sat
//! before. A shorter word ("Tag       ") painted over a longer one
//! ("Esher     ") at the same pixel slot is exactly [`zvm::screen::V6Windows::paint_run`]'s
//! SQ-1589 shape, and pre-fix this left a stray "er" behind, reading as
//! "Tager".
//!
//! Reached deterministically (same fixed inputs every run — verified by
//! diffing two independent runs of the driving loop) and turn-counted per
//! CLAUDE.md's "a frame is a fixture" convention, rather than driven to a
//! prose landmark that might dodge the encounter on a future release.
//!
//! Skips cleanly when the gitignored release floppy is absent.

use std::path::PathBuf;

use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

const AMIGA_RELEASE: &str = "Journey - The Quest Begins.adf";

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot the Amiga release floppy the way the other Amiga Journey suites do
/// (`v6_journey_amiga_frame.rs`'s `journey_floppy`) and drive it `steps`
/// blank-line/Enter inputs from boot. No mouse, no line text — the encounter
/// is scripted into the intro and needs neither.
fn journey_floppy(steps: usize) -> Option<GameSession> {
    let path = stories_dir().join(AMIGA_RELEASE);
    let story_bytes = match app::hints::load_story(&path) {
        Ok(s) => s.into_bytes(),
        Err(_) => {
            eprintln!("SKIP: gitignored release floppy missing at {}", path.display());
            return None;
        }
    };
    let profile = InterpreterProfile::resolve(&path, None, None, None);
    assert_eq!(profile, InterpreterProfile::Amiga, "the floppy names the machine");
    let mut picts = PictSource::resolve(&path, None);
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px = picts.std_window().or_else(|| profile.std_window());
    let mut session = GameSession::new_with_trace(
        story_bytes,
        true,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        profile.default_colours(),
        None,
    )
    .expect("Journey's release floppy should mount and boot without a ZError");
    // SQ-1393: the machine's own colour table.
    session.machine.set_palette(profile.palette());
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    for _ in 0..steps {
        let _ = match session.pending_input() {
            InputKind::Line => session.submit(""),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
    }
    Some(session)
}

/// win1's current painted text runs — screen-absolute native pixels, exactly
/// [`zvm::screen::V6Windows::paint_run`]'s own record (not a rendered pane),
/// since the defect is in that model and any render path would faithfully
/// reproduce it either way.
fn win1_runs(s: &GameSession) -> Vec<(u16, u16, String)> {
    let v6 = s.machine.screen.v6.as_ref().expect("v6 windows");
    v6.windows[1].texts.iter().map(|t| (t.y, t.x, t.text.clone())).collect()
}

/// (a) Seven inputs in: the party still carries "Esher" at native (337, 137),
/// a fourth row below "Praxix". A non-vacuity guard — if a future release
/// changes the intro's pacing and the encounter no longer fires here, this
/// must fail loudly rather than let the case below pass on an empty premise.
#[test]
fn seven_inputs_in_the_party_still_carries_esher() {
    let Some(session) = journey_floppy(7) else { return };
    let runs = win1_runs(&session);
    assert!(
        runs.iter().any(|(y, x, t)| *y == 337 && *x == 137 && t.trim() == "Esher"),
        "turn 7: expected \"Esher\" at native (337,137); got the party column: {:?}",
        runs.iter().filter(|(_, x, _)| *x == 137).collect::<Vec<_>>(),
    );
}

/// (b) The 8th input resolves the encounter and reprints that slot with the
/// shorter "Tag" — and no fragment of "Esher" ("er", the stray tail that
/// read as "Tager") survives anywhere in win1's run list.
///
/// FALSIFY by reverting the `overwrites_same_slot` fix in
/// `zvm::screen::V6Windows::paint_run` (restoring the pre-SQ-1589 rule that a
/// mixed run's padding space never erases ink): the third assertion fails,
/// holding a leftover `"er     "` run at native x=160 — the exact remnant
/// this quest's own repro reported.
#[test]
fn the_eighth_input_erases_eshers_tail_under_the_shorter_tag() {
    let Some(session) = journey_floppy(8) else { return };
    let runs = win1_runs(&session);
    assert!(
        runs.iter().any(|(y, x, t)| *y == 337 && *x == 137 && t.trim() == "Tag"),
        "turn 8: expected \"Tag\" at native (337,137); got the party column: {:?}",
        runs.iter().filter(|(_, x, _)| *x == 137).collect::<Vec<_>>(),
    );
    assert!(
        !runs.iter().any(|(_, _, t)| t.trim() == "Esher"),
        "turn 8: \"Esher\" must have left the party column entirely: {:?}",
        runs.iter().filter(|(_, x, _)| *x == 137).collect::<Vec<_>>(),
    );
    // The composite reading, not just the individual runs: reconstruct row
    // 337 and check it never spells "Tager" — the exact reported symptom —
    // and carries no stray letter past "Tag" at all.
    let row_337: String = {
        let mut chars: Vec<(u16, char)> = runs
            .iter()
            .filter(|(y, _, _)| *y == 337)
            .flat_map(|(_, x, t)| t.chars().enumerate().map(move |(i, c)| (*x + i as u16 * 7, c)))
            .collect();
        chars.sort_by_key(|(x, _)| *x);
        chars.into_iter().map(|(_, c)| c).collect()
    };
    assert!(
        !row_337.contains("Tager"),
        "turn 8: row 337 must not spell \"Tager\": {row_337:?}"
    );
    assert!(
        !runs.iter().any(|(y, x, t)| *y == 337 && *x > 137 && *x < 137 + 70 && t.trim().chars().any(|c| c.is_alphabetic())),
        "turn 8: no stray letters survive in Tag's own field width past its own run: {:?}",
        runs.iter().filter(|(y, _, _)| *y == 337).collect::<Vec<_>>(),
    );
}
