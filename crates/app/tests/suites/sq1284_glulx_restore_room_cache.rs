//! SQ-1284: a restored Glulx session must not answer for the room it left.
//!
//! Reported from a `/export-map` of the COMMERCIAL Anchorhead (Glulx), taken around the
//! real-estate office: `#57029 "Outside the Real Estate Office"` carried edges NW, NE, S, SW, U,
//! D and OUT **all landing on the one room** `#47665 "Office"`, beside the two it really has
//! (E and IN). Every one of those seven directions is refused by the game — `"The street only
//! goes west from here…"`, `"You cannot go up from here."` — and a refused move crosses nothing.
//!
//! # Where the edges came from
//!
//! [`app::return_probe`] asks the shadow to walk all twelve directions out of a room, and records
//! a passage whenever one lands in a room the map already holds. [`app::probe::serve`] restores
//! the shadow to the player's moment before EVERY command, and `restore_state` swaps VM memory
//! and nothing else — so `GlulxSession::last_room`, the host-side cache of the room the story
//! last printed a HEADING for, survived from the shadow's PREVIOUS question. `drive_turn` treats
//! that cache as sticky (right within one run: a turn that prints no heading has not moved you),
//! so a refused move in the shadow reported the room the previous attempt had walked into.
//!
//! `record_probed_passage` already refuses `from == to`, which is why an ordinary refused move
//! records nothing. A STALE room is not `from`, so it sailed through — one wrong edge per refused
//! direction, all pointing at whichever room the shadow happened to be holding.
//!
//! # Why the ids in that dump are name hashes, and why it matters here
//!
//! Every id in the reported map is [`app::roomid::synthetic_room_id`] of the room's own name
//! (`syn("Outside the Real Estate Office") == 57029`, `syn("Office") == 47665`,
//! `syn("Twisting Lane") == 34826`, …), so commercial Anchorhead never resolved its room lock
//! (`app::glulx_roomlock`) and `GlulxSession::room_for` was hashing NAMES for the whole session.
//! That is the case this reproduces: unlocked, the stale name IS the stale id, so a refused move
//! reports a different room outright. With the lock resolved the id comes from RAM and is right —
//! the name is still stale, but the symptom hides — which is why the case below asks its
//! questions at BOOT, before the live session has locked anything, exactly as the report did.
//!
//! # The fixture
//!
//! `AnchorheadDemo.gblorb` is the Glulx demo of the same commercial game and opens in the same
//! room, `#57029` by that same hash. The free 1998 `anchor.z8` cannot reproduce any of this: the
//! Z-machine reads its room out of the restored OBJECT TREE, so it has no host-side room cache to
//! go stale. Both are gitignored; this skips vacuously without them.
//!
//! Falsified: with `self.last_room = None` reverted out of `GlulxSession::restore_state`,
//! [`a_refused_move_after_a_restore_reports_no_room_not_the_previous_one`] reports
//! `Garbage-Choked Alley` from a session standing outside the real estate office, and
//! [`the_shadow_never_answers_a_refused_move_with_its_previous_landing`] answers the refused `up`
//! with the id of the alley the previous question walked into — the dump's fan, one edge of it.

use std::path::PathBuf;
use std::sync::Arc;

use app::engine::{Engine, KeyInput};
use app::glulx_session::GlulxSession;
use app::probe::ShadowRecipe;
use app::roomid::synthetic_room_id;
use app::state::AppState;

use crate::fixture_paths::fixture_path;

/// The demo's opening room, and the room every case below stands in.
const OUTSIDE: &str = "Outside the Real Estate Office";
/// Where `southeast` goes from there — the room the stale cache used to answer with.
const ALLEY: &str = "Garbage-Choked Alley";

fn story() -> Option<Vec<u8>> {
    let path = fixture_path("AnchorheadDemo.gblorb");
    match std::fs::read(&path) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            None
        }
    }
}

fn recipe_in(bytes: &[u8], store: PathBuf) -> ShadowRecipe {
    ShadowRecipe {
        story_bytes: Arc::new(bytes.to_vec()),
        store,
        vfs_bytes: Arc::new(Vec::new()),
        honor_game_colours: true,
        interpreter_number: None,
        random_seed: None,
        acceleration: true,
        screen: (80, 24),
    }
}

/// Boot the demo into play, parked outside the real estate office. Returns the session and the
/// raw story bytes (the shadow recipe wants the blorb, not the extracted image) plus the store
/// the two share, mirroring `startup.rs`.
fn boot(tag: &str) -> Option<(GlulxSession, Vec<u8>, PathBuf)> {
    let bytes = story()?;
    let blorb = blorb::Blorb::parse(bytes.clone()).ok()?;
    let (kind, exec) = blorb.executable().ok()?;
    assert_eq!(kind, blorb::ExecKind::Glulx, "AnchorheadDemo.gblorb is a Glulx blorb");
    let store = app::scratch_dir(tag);
    let mut s = GlulxSession::new_in(
        store.clone(),
        exec.to_vec(),
        80,
        24,
        true,
        false,
        false,
        false,
        (1, 1),
        None,
        &[],
        [[(None, None); 11]; 2],
        false,
        None,
    )
    .expect("AnchorheadDemo.gblorb boots");
    for _ in 0..12 {
        if s.current_location().is_some() {
            break;
        }
        if s.pending_input() != app::session::InputKind::Char {
            break;
        }
        s.submit_key(KeyInput::Enter);
    }
    let here = s.current_location()?;
    // Non-vacuity, and the report's own keying: the demo opens in the reported room, and its id
    // is the NAME hash — the live session has locked nothing yet, exactly as the reported session
    // never did.
    assert_eq!(here.name, OUTSIDE, "the demo opens outside the real estate office");
    assert_eq!(
        here.number,
        synthetic_room_id(OUTSIDE),
        "an unlocked Glulx session keys rooms by name hash, as every id in the reported dump is"
    );
    Some((s, bytes, store))
}

/// The engine-level defect, with no probe in sight: restore a session to a moment, then take a
/// turn that prints no heading, and it must not answer with the room it walked into before the
/// restore.
#[test]
fn a_refused_move_after_a_restore_reports_no_room_not_the_previous_one() {
    let Some((mut s, _bytes, _store)) = boot("sq1284-restore-cache") else { return };
    let save = Engine::save_state(&s);

    // Walk somewhere else, so the host-side room cache holds a room the restore will undo.
    let r = Engine::submit(&mut s, "southeast");
    assert!(r.transcript.contains(ALLEY), "southeast reaches the alley: {:?}", r.transcript);
    assert_eq!(
        s.current_location().map(|l| l.name),
        Some(ALLEY.to_string()),
        "the cache now holds the alley"
    );

    Engine::restore_state(&mut s, &save).expect("the session takes its own snapshot back");

    // A refused move: the game prints a complaint and no room heading at all.
    let r = Engine::submit(&mut s, "up");
    assert!(
        r.transcript.to_lowercase().contains("cannot go up"),
        "\"up\" is refused outside the office: {:?}",
        r.transcript
    );
    assert!(
        !r.transcript.contains(ALLEY),
        "…and prints no heading, so nothing on screen says where we are: {:?}",
        r.transcript
    );

    let got = s.current_location();
    assert_ne!(
        got.as_ref().map(|l| l.name.as_str()),
        Some(ALLEY),
        "SQ-1284: the room cache must not survive a restore — a session standing outside the \
         real estate office reported {got:?}"
    );
    assert_ne!(
        got.as_ref().map(|l| l.number),
        Some(synthetic_room_id(ALLEY)),
        "…nor its id, which under an unlocked lock is the whole of the room's identity: {got:?}"
    );
}

/// The shape the report was made of: the same two questions the return probe asks, through the
/// real shadow seam. The refused one must not be answered with the landing of the one before it —
/// that answer is what `return_probe::deliver` minted the fan of wrong edges from.
#[test]
fn the_shadow_never_answers_a_refused_move_with_its_previous_landing() {
    let Some((s, bytes, store)) = boot("sq1284-shadow-cache") else { return };
    let mut state = AppState::default();
    state.probe.arm(recipe_in(&bytes, store));
    assert!(state.probe.is_armed(), "the recipe carries real story bytes");

    // Question one: walk somewhere real. Both questions run from the SAME live moment, which is
    // what `return_probe::ReturnSearch` does with its one snapshot.
    let token = state.probe.ask(&s, &["southeast".to_string()]).expect("the first question sends");
    let a = state.probe.settle().expect("the worker answers");
    assert_eq!(a.token, token);
    let step = &a.run.as_ref().expect("a run").steps[0];
    assert!(step.reply.contains(ALLEY), "the shadow really walked into the alley: {:?}", step.reply);
    let alley = step.location.expect("…and said so");
    assert_eq!(
        alley,
        synthetic_room_id(ALLEY),
        "an unlocked shadow keys by name hash, like the live session it copies"
    );

    // Question two, from that same moment: a direction the game refuses. It prints no heading, so
    // the only room the shadow could name is one it is not in.
    let token = state.probe.ask(&s, &["up".to_string()]).expect("the second question sends");
    let b = state.probe.settle().expect("the worker answers");
    assert_eq!(b.token, token);
    let step = &b.run.as_ref().expect("a run").steps[0];
    assert!(
        step.reply.to_lowercase().contains("cannot go up"),
        "\"up\" is refused: {:?}",
        step.reply
    );
    assert_ne!(
        step.location,
        Some(alley),
        "SQ-1284: a restore must clear the shadow's room cache, or a refused move reports the \
         PREVIOUS question's landing and `return_probe::deliver` mints an edge to it — the \
         reported fan of NW/NE/S/SW/U/D/OUT edges onto one room"
    );
}
