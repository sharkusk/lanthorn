//! SQ-1286: a Glulx game whose object table lies beyond the room lock's scan window must still
//! lock onto its `location` global.
//!
//! # The report
//!
//! Two `/export-map` dumps — one from the commercial Anchorhead (Glulx), one from Counterfeit
//! Monkey release 11 — carried nothing but NAME hashes: every id was
//! [`app::roomid::synthetic_room_id`] of the room's own heading, for the whole session. Same-named
//! rooms therefore collapse into one node, a random exit's destination pool only grows to distinct
//! NAMES, and anything wanting a real object number (a declared exit, SQ-1267's shadow identity)
//! is blind. `app::glulx_roomlock` had never resolved on either game.
//!
//! # What it was
//!
//! Not the correlation, and not I6 vs I7 — Anchorhead is Inform 6 (its object table carries
//! `(Inform Parser)` and `(Inform Library)`), Counterfeit Monkey is Inform 7, and the learner
//! scores both identically. It was the last filter a surviving candidate had to pass: its VALUE
//! had to be "a nonzero address inside the scanned region", and the scanned region is 64 KB from
//! `ramstart`.
//!
//! 64 KB is ample for the GLOBAL — Inform lays its globals at the very start of RAM, and
//! `location` is `ramstart+0x28` in Adventure, `+0x2c` in the Anchorhead demo, `+0x98` in
//! Counterfeit Monkey. It is nowhere near enough for what the global POINTS AT: an Inform story's
//! object table sits after its globals and its arrays. Measured across the 42 Glulx stories in
//! `stories/`, only five keep their objects within 64 KB of `ramstart`; Counterfeit Monkey's are
//! **1.9 MB** above it. So on 37 of 42 the one true candidate was thrown away every turn, and the
//! game keyed rooms by name for the whole session.
//!
//! The fix asks the exact question instead of the approximate one: `gvm::objects::ParseNames`
//! already walks the story's object table for the play-aids, so a candidate's value is checked
//! against THAT. The old range test survives only as the fallback for a story whose object table
//! cannot be walked at all.
//!
//! # The fixtures
//!
//! * **`CounterfeitMonkey-11.gblorb`** — the reported game, and the one that reproduces: its
//!   object table is far outside the window, so it never locked. It is also the check that the
//!   new filter accepts a value the old one rejected.
//! * **`AnchorheadDemo.gblorb`** — the Glulx demo of the reported Anchorhead build, and the
//!   NON-regression: it is one of the five whose objects land inside the window, so it locked
//!   before this change and must still lock, onto the same word. (The demo therefore does not
//!   reproduce the report; the commercial build, several times its size, does.)
//!
//! Both are gitignored, so every case here skips vacuously without them.
//!
//! Falsified: with `is_room_value` reverted to `v != 0 && v >= self.base && v < end`, both
//! Counterfeit Monkey cases fail on `locked_room_global() == None` after a walk that really did
//! pass through `["Sigil Street", "Ampersand Bend"]` — the lock never resolves, so every id the
//! session hands out stays the heading's hash. [`the_anchorhead_demo_still_locks_the_same_word`]
//! passes either way, which is the point of keeping it: the demo is the small case the old filter
//! could already serve.

use std::path::PathBuf;

use app::engine::{Engine, KeyInput};
use app::glulx_session::GlulxSession;
use app::roomid::synthetic_room_id;

use crate::fixture_paths::fixture_path;

/// The room lock's scan window, `app::glulx_session::scan_words`' own constant — the reach the
/// old value filter had, and what these fixtures' object tables are measured against.
const WINDOW_BYTES: u32 = 64 * 1024;

fn story(name: &str) -> Option<Vec<u8>> {
    let path = fixture_path(name);
    match std::fs::read(&path) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            None
        }
    }
}

/// Boot a Glulx blorb into play, dismissing any "press a key" splash, exactly as
/// `sq1284_glulx_restore_room_cache::boot` does. Returns the session and the Glulx image's
/// RAMSTART, which the object-table reach assertions need.
fn boot(name: &str, tag: &str) -> Option<(GlulxSession, u32)> {
    let bytes = story(name)?;
    let blorb = blorb::Blorb::parse(bytes).ok()?;
    let (kind, exec) = blorb.executable().ok()?;
    assert_eq!(kind, blorb::ExecKind::Glulx, "{name} is a Glulx blorb");
    // Glulx header, spec §1.2: RAMSTART is the long at offset 8.
    let ramstart = u32::from_be_bytes(exec[8..12].try_into().expect("a Glulx header"));
    let store: PathBuf = app::scratch_dir(tag);
    let mut s = GlulxSession::new_in(
        store,
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
    .unwrap_or_else(|e| panic!("{name} boots: {e:?}"));
    for _ in 0..12 {
        if s.current_location().is_some() {
            break;
        }
        if s.pending_input() != app::session::InputKind::Char {
            break;
        }
        s.submit_key(KeyInput::Enter);
    }
    Some((s, ramstart))
}

/// Where this story's own room id comes from right now: `Named` while the lock is unresolved and
/// the id is nothing but the heading's hash, `Object` once it carries the room's address.
#[derive(Debug, PartialEq, Eq)]
enum Keying {
    Named,
    Object,
}

fn keying(s: &GlulxSession) -> Option<(String, u16, Keying)> {
    let l = s.current_location()?;
    let k = if l.number == synthetic_room_id(&l.name) { Keying::Named } else { Keying::Object };
    Some((l.name, l.number, k))
}

/// Play `cmds`, returning every distinct room NAME the walk passed through. The names themselves
/// are deliberately not pinned — SQ-1285 is separately correcting a heading misread on this very
/// story — only that the walk really moved between rooms, which is what makes the lock's evidence
/// non-vacuous.
fn walk(s: &mut GlulxSession, cmds: &[&str]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for c in cmds {
        // Counterfeit Monkey's prologue pauses on a "press any key" page partway through; drain
        // any such gate before the next command, exactly as `boot` drains the opening one.
        for _ in 0..4 {
            if s.pending_input() != app::session::InputKind::Char {
                break;
            }
            s.submit_key(KeyInput::Enter);
        }
        if s.pending_input() != app::session::InputKind::Line {
            break;
        }
        let _ = Engine::submit(s, c);
        if let Some(l) = s.current_location() {
            if !seen.contains(&l.name) {
                seen.push(l.name);
            }
        }
    }
    seen
}

/// Counterfeit Monkey's opening: a yes/no question, then a street the walk can move around in.
/// `west`, `south` and the second `north` are refused (a shut boutique, a closed office, a locked
/// barrier) — the heading-less turns `REQUIRED_STILLS` wants — while `north`/`east`/`west` move.
const CM_WALK: [&str; 12] = [
    "no", "look", "south", "north", "west", "east", "wait", "south", "north", "west", "east",
    "wait",
];

#[test]
fn counterfeit_monkey_locks_its_location_global() {
    let Some((mut s, ramstart)) = boot("CounterfeitMonkey-11.gblorb", "sq1286-cm-lock") else {
        return;
    };

    // Non-vacuity, part one: this fixture really is one the OLD filter could never accept. Its
    // object table starts far beyond the scanned window, so no word holding a room address could
    // pass "a nonzero address inside the scanned region".
    let head = s.parse_names().expect("Counterfeit Monkey's object table is walkable").head();
    assert!(
        head >= ramstart + WINDOW_BYTES,
        "the case rests on this story's objects lying outside the {WINDOW_BYTES}-byte scan \
         window: ramstart 0x{ramstart:x}, object table 0x{head:x}"
    );

    // Non-vacuity, part two: the walk actually moved between rooms, so the learner saw the
    // room-change evidence it needs.
    let seen = walk(&mut s, &CM_WALK);
    assert!(seen.len() >= 2, "the walk must pass through at least two rooms, saw {seen:?}");

    let addr = s.locked_room_global();
    assert!(
        addr.is_some(),
        "SQ-1286: the lock must resolve on a story whose objects lie outside the scan window \
         (rooms walked: {seen:?})"
    );
    let addr = addr.expect("checked above");
    assert!(
        (ramstart..ramstart + WINDOW_BYTES).contains(&addr),
        "…onto a GLOBAL, which Inform puts at the start of RAM: 0x{addr:x} vs ramstart \
         0x{ramstart:x}"
    );

    let (name, id, k) = keying(&s).expect("the walk ends in a room");
    assert_eq!(
        k,
        Keying::Object,
        "the room the walk ends in must be keyed by its own address, not by its heading: \
         {name:?} #{id} (name hash #{})",
        synthetic_room_id(&name)
    );
}

#[test]
fn two_rooms_keep_distinct_ids_across_a_restore() {
    let Some((mut s, _ramstart)) = boot("CounterfeitMonkey-11.gblorb", "sq1286-cm-restore") else {
        return;
    };
    let seen = walk(&mut s, &CM_WALK);
    assert!(seen.len() >= 2, "the walk must pass through at least two rooms, saw {seen:?}");
    assert!(s.locked_room_global().is_some(), "the walk resolves the lock");

    let (here_name, here, k) = keying(&s).expect("the walk ends in a room");
    assert_eq!(k, Keying::Object, "…keyed by address: {here_name:?}");
    let save = Engine::save_state(&s);

    // Move somewhere else and confirm the two rooms are two ids. Under the name hash they would
    // be two ids as well — as long as the names differ — which is why the restore half below is
    // the part that actually needs the address.
    let _ = Engine::submit(&mut s, "west");
    let (there_name, there, k) = keying(&s).expect("west lands in a room");
    assert_eq!(k, Keying::Object, "…also keyed by address: {there_name:?}");
    assert_ne!(there, here, "two rooms, two ids ({there_name:?} vs {here_name:?})");

    // Back to the first moment. A restore swaps VM memory and clears the host-side room cache
    // (SQ-1284), so the next turn's heading is what re-resolves the room — through the SAME
    // locked global, whose value the restored memory carries.
    Engine::restore_state(&mut s, &save).expect("the session takes its own snapshot back");
    let _ = Engine::submit(&mut s, "look");

    let (back_name, back, k) = keying(&s).expect("the restored session knows where it is");
    assert_eq!(k, Keying::Object, "…still keyed by address after the restore: {back_name:?}");
    assert_eq!(back, here, "the restore returns the room it saved from: {back_name:?} vs {here_name:?}");
    assert_ne!(back, there, "…and it is still not the room the detour reached");
}

#[test]
fn the_anchorhead_demo_still_locks_the_same_word() {
    // The non-regression, and the one fixture where the answer is already known: the demo's
    // objects land INSIDE the scan window, so it locked before this change too, at ramstart+0x2c.
    let Some((mut s, ramstart)) = boot("AnchorheadDemo.gblorb", "sq1286-anchor-demo") else {
        return;
    };
    let head = s.parse_names().expect("the demo's object table is walkable").head();
    assert!(
        head < ramstart + WINDOW_BYTES,
        "the demo is the small case: its objects are inside the window (0x{head:x} vs \
         0x{:x})",
        ramstart + WINDOW_BYTES
    );

    let opening = s.current_location().expect("the demo opens in a room");
    assert_eq!(opening.name, "Outside the Real Estate Office", "the demo's opening room");
    assert_eq!(
        opening.number,
        synthetic_room_id("Outside the Real Estate Office"),
        "…keyed by name at boot, before the learner has seen a single turn"
    );

    let seen = walk(&mut s, &["look", "west", "south", "north", "east", "wait"]);
    assert!(seen.len() >= 3, "the opening streets are three rooms, saw {seen:?}");
    assert_eq!(
        s.locked_room_global(),
        Some(ramstart + 0x2c),
        "the demo locks onto the same word it always did"
    );

    let (name, id, k) = keying(&s).expect("the walk ends in a room");
    assert_eq!(name, "Outside the Real Estate Office", "the walk returns where it started");
    assert_eq!(
        k,
        Keying::Object,
        "…and once locked the opening room stops being the hash of its heading: #{id} vs #{}",
        synthetic_room_id(&name)
    );
}
