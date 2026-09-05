//! SQ-1305: a stale `room-global` sidecar after a story rebuild.
//!
//! `GlulxSession` persists the learned `location` global's address in
//! `<save dir>/room-global`, keyed by the story's FILENAME. A story replaced
//! under the same name — a new release of a `.gblorb`, an author's rebuild —
//! reuses the old sidecar. If the rebuilt image happens to hold a real OBJECT
//! (not a room) at the old address, the pre-SQ-1305 lock never falsified: since
//! SQ-1294 the lock is the authority on movement, `movement()` reported
//! `Unchanged` forever and the map never moved again.
//!
//! Two independent defenses, each tested here against a REAL compiled world
//! model (`gvm::i7map`) and real gameplay — neither a synthetic unit fixture
//! can fabricate:
//!
//! * [`a_boot_time_sidecar_pointing_at_a_non_room_object_is_refused`] —
//!   `GlulxSession::sidecar_addr_plausible`: a sidecar address whose value is
//!   neither `0` nor one of this story's own ROOMS is refused before it is
//!   ever trusted, so the map plays exactly as a fresh boot would.
//! * [`a_lock_frozen_on_a_perpetual_zero_recovers_within_a_few_turns`] —
//!   `RoomLock::verify`'s `FROZEN_LOCK_HEADINGS`: even a lock that DID get
//!   installed wrongly (bypassing the boot-time check the way
//!   `GlulxSession::relock_room_global` does for a live shadow-sync,
//!   simulating what a defeated defense would have let through) is dropped
//!   and relearned after three straight turns of a fresh heading over a
//!   motionless word.
//!
//! Both are falsified in the finding this suite is built from by temporarily
//! reverting the fix under test and confirming the assertion below fails with
//! the originally reported symptom (CLAUDE.md's "falsify fixes" convention) —
//! not by a runtime toggle in the test itself, since neither defense exposes
//! one to the outside.
//!
//! `stories/CounterfeitMonkey-11.gblorb` — release 11 / serial 230220, Inform 7
//! build 6M62 (the same fixture `sq1294b_glulx_flashback_heading` and
//! `sq1303_glulx_static_world` use). Gitignored, so both cases skip vacuously
//! without it.

use std::collections::HashSet;

use app::engine::{Debugger, Engine, KeyInput};
use app::glulx_session::GlulxSession;
use app::session::InputKind;

use crate::fixture_paths::fixture_path;

const STORY: &str = "CounterfeitMonkey-11.gblorb";

/// The prologue every CM route in this codebase starts from
/// (`sq1294b_glulx_flashback_heading`'s `ROUTE`): answer "do you remember our
/// name?", dismiss the opening banner, and fix the RNG / turn off pauses so
/// play is deterministic. Deliberately stops BEFORE the first real move (`n`,
/// north out of the Back Alley) — that is [`HOPS`]' first step, the one this
/// suite cares about.
const PROLOGUE: &[&str] = &["y", "andra", "", "tutorial off", "random-seed 1234", "pauses off"];

/// Real moves: a loop through Back Alley / Sigil Street / Ampersand Bend.
/// Several of these are refused by the story (a locked shop, a locked
/// barrier) and print no room heading at all — deliberately kept in, since a
/// real fixture always has some.
const HOPS: &[&str] = &["n", "n", "e", "e", "s", "n", "w", "s", "n", "w", "s"];

/// The route [`a_lock_frozen_on_a_perpetual_zero_recovers_within_a_few_turns`]
/// plays, chosen so THREE turns in a row each print a heading that is NOT the
/// room the bad lock is installed in.
///
/// That "not the frozen room" condition matters because of what a frozen lock
/// does to `GlulxSession::last_room`: with the lock reporting `Unchanged`
/// every turn, `adopt_heading_for_room` never updates it, so it stays parked
/// on the opening room ("Back Alley") for the whole walk — and `heading_movement`
/// compares each turn's printed heading against THAT stuck value, not against
/// the previous turn's. A move that returns to Back Alley therefore reads as
/// `Ambiguous` (same name), which resets the frozen streak exactly like a
/// blocked turn does. `n, e, w, e, w, e` starts with the one arrival move (`n`,
/// exempt: no predecessor to compare against) and then shuttles between Sigil
/// Street and Ampersand Bend — never back through Back Alley — so `e, w, e`
/// gives three straight fresh headings over the motionless word, which is
/// exactly `FROZEN_LOCK_HEADINGS` (`RoomLock::verify`,
/// `crates/app/src/glulx_roomlock.rs`). The trailing `w, e` plays past the
/// drop to confirm the RE-lock (via `RoomLock::name_witness`, since the
/// object/room tables survive a `relearn`) lands back on the true address.
const FREEZE_HOPS: &[&str] = &["n", "e", "w", "e", "w", "e"];

/// Submit one step the way every CM route in this codebase does
/// (`sq1294b_glulx_flashback_heading::play`): a `Char`-pending turn takes a
/// bare keypress regardless of the scripted line (CM's own "press any key"
/// pages), a `Line`-pending turn takes the line verbatim.
fn step(s: &mut GlulxSession, cmd: &str) -> app::session::TurnResult {
    if s.pending_input() == InputKind::Char {
        s.submit_key(KeyInput::Enter).expect("Glulx takes keys")
    } else {
        assert_eq!(s.pending_input(), InputKind::Line, "step {cmd:?} wants a line");
        Engine::submit(s, cmd)
    }
}

/// The whole `.gblorb` file's bytes, read once. `None` (with a SKIP message)
/// if the gitignored fixture is missing.
fn story_bytes() -> Option<Vec<u8>> {
    let path = fixture_path(STORY);
    match std::fs::read(&path) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            None
        }
    }
}

/// The raw Glulx image `GlulxSession` actually boots from a `.gblorb`'s bytes —
/// the same extraction `boot_in` performs, exposed separately so a test can
/// compute [`image_identity`] from it without booting a session first.
fn glulx_image(bytes: Vec<u8>) -> Vec<u8> {
    match app::hints::extract_story(bytes).expect("a readable container") {
        app::hints::LoadedStory::Glulx(image) => image,
        _ => panic!("{STORY} is a Glulx story"),
    }
}

/// This image's `(checksum, EXTSTART)` — the token
/// `GlulxSession::image_identity` stamps a `room-global` sidecar with
/// (SQ-1305), computed independently here so a test can hand-craft a sidecar
/// file that passes it.
fn image_identity(bytes: Vec<u8>) -> (u32, u32) {
    let mem = gvm::memory::Memory::new(glulx_image(bytes)).expect("a valid Glulx header");
    (mem.checksum(), mem.extstart())
}

/// Boot from the whole-file bytes — matching how
/// `sq1294b_glulx_flashback_heading::play` boots this exact fixture (80x30,
/// `(8, 16)` char pixels, WITH the picture Blorb) rather than a bare exec at
/// some other geometry: a narrower window or no picture Blorb changes CM's
/// own pagination (more "press a key" banner pages), which would desync
/// [`PROLOGUE`]/[`HOPS`] from the wider one they were scripted against.
fn boot_in(dir: &std::path::Path, bytes: Vec<u8>) -> GlulxSession {
    let pict_blorb = blorb::Blorb::parse(bytes.clone()).ok();
    GlulxSession::new_in(
        dir.to_path_buf(),
        glulx_image(bytes),
        80,
        30,
        true,
        false,
        false,
        false,
        (8, 16),
        pict_blorb,
        &[],
        Default::default(),
        false,
        None,
    )
    .expect("Counterfeit Monkey boots")
}

/// Read the big-endian u32 at `addr` through the debug inspector's hex dump —
/// the only memory-reading seam these tests have
/// (`app::engine::Debugger::memory_hex`).
fn read_word(s: &GlulxSession, addr: u32) -> u32 {
    let dbg = Engine::debugger(s).expect("Glulx exposes a debugger");
    let row_start = addr & !0xF;
    let rows = dbg.memory_hex(row_start, 2);
    let mut bytes = Vec::new();
    for row in &rows {
        // `"{a:06x}  {hex:<48}{ascii}{tag}"` (glulx_debug.rs's `memory_hex`): a
        // 6-digit address, two literal spaces, then 16 "xx " hex groups.
        bytes.extend(row[8..56].split_whitespace().map(|h| u8::from_str_radix(h, 16).unwrap()));
    }
    let off = (addr - row_start) as usize;
    u32::from_be_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
}

/// Is `addr` inside the region a fresh `memory_hex` row tags `<RAM>`?
fn is_ram(s: &GlulxSession, addr: u32) -> bool {
    let dbg = Engine::debugger(s).expect("Glulx exposes a debugger");
    dbg.memory_hex(addr & !0xF, 1)[0].contains("<RAM>")
}

/// RAMSTART, found by binary search over the `<RAM>` tag `memory_hex` already
/// computes from `mem.ramstart()` — the same boundary `GlulxSession::scan_words`
/// gives the room-lock learner its 64 KB window from.
fn find_ramstart(s: &GlulxSession) -> u32 {
    let dbg = Engine::debugger(s).expect("Glulx exposes a debugger");
    let (mut lo, mut hi) = (0u32, dbg.memory_len());
    while hi - lo > 256 {
        let mid = ((lo + hi) / 2) & !0xFF;
        if mid <= lo || mid >= hi {
            break;
        }
        if is_ram(s, mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

/// A RAM word inside the room-lock's 64 KB scan window whose value, across the
/// whole of [`HOPS`], is consistently one of this story's OBJECTS but never
/// one of its ROOMS and never the true `location` global — a stand-in for the
/// kind of word a rebuild can leave a stale sidecar pointing at (the globals
/// region is full of them: `player`, `actor`, `real_location`, `noun`).
///
/// Boots its OWN session (independent of the caller's) and plays [`PROLOGUE`]
/// then [`HOPS`] once to observe every candidate across the whole walk before
/// settling on one, so what this hands back is a value already KNOWN never to
/// move during the very route the other tests drive it with.
fn find_wrong_candidate(bytes: Vec<u8>, true_addr: u32) -> u32 {
    let dir = app::scratch_dir("sq1305-candidate-scan");
    let mut s = boot_in(&dir, bytes);
    for c in PROLOGUE {
        let _ = step(&mut s, c);
    }
    let objects: HashSet<u32> = s.parse_names().expect("CM has a readable object list").objects().collect();
    let rooms: HashSet<u32> =
        s.i7_world().expect("CM has a compiled I7 world model").rooms().iter().copied().collect();

    let ramstart = find_ramstart(&s);
    let window_words = (64 * 1024) / 4;
    let read_all = |s: &GlulxSession| -> Vec<u32> {
        (0..window_words).map(|i| read_word(s, ramstart + i * 4)).collect()
    };
    let plausible = |addr: u32, v: u32| addr != true_addr && v != 0 && objects.contains(&v) && !rooms.contains(&v);

    let mut alive: Vec<u32> = read_all(&s)
        .iter()
        .enumerate()
        .filter(|&(i, &v)| plausible(ramstart + (i as u32) * 4, v))
        .map(|(i, _)| ramstart + (i as u32) * 4)
        .collect();

    for c in HOPS {
        let _ = step(&mut s, c);
        let now = read_all(&s);
        alive.retain(|&addr| plausible(addr, now[((addr - ramstart) / 4) as usize]));
    }

    *alive
        .first()
        .unwrap_or_else(|| panic!("no RAM word in the scan window held a stable non-room object across {HOPS:?}"))
}

/// A RAM word inside the scan window that reads `0` at every point across
/// [`PROLOGUE`] + `hops` — an unused Inform global, of which there are always
/// several. `RoomLock::verify`'s object-value check explicitly treats a zero
/// as "no evidence either way" (a game may legitimately park `location` at
/// nothing mid-scene), so a lock frozen on a word like this is the ONE case
/// the pre-SQ-1305 code could never falsify by value at all — only the
/// frozen-heading check can, which makes this the cleanest possible isolation
/// of that one fix from everything else `verify` does. `hops` is whatever
/// route the caller is actually about to play, so the candidate is guaranteed
/// stable for exactly as long as it will be trusted.
fn find_stuck_zero_candidate(bytes: Vec<u8>, true_addr: u32, hops: &[&str]) -> u32 {
    let dir = app::scratch_dir("sq1305-zero-scan");
    let mut s = boot_in(&dir, bytes);
    for c in PROLOGUE {
        let _ = step(&mut s, c);
    }
    let ramstart = find_ramstart(&s);
    let window_words = (64 * 1024) / 4;
    let read_all = |s: &GlulxSession| -> Vec<u32> {
        (0..window_words).map(|i| read_word(s, ramstart + i * 4)).collect()
    };
    let mut alive: Vec<u32> = read_all(&s)
        .iter()
        .enumerate()
        .filter(|&(i, &v)| v == 0 && ramstart + (i as u32) * 4 != true_addr)
        .map(|(i, _)| ramstart + (i as u32) * 4)
        .collect();

    for c in hops {
        let _ = step(&mut s, c);
        let now = read_all(&s);
        alive.retain(|&addr| now[((addr - ramstart) / 4) as usize] == 0);
    }

    *alive.first().unwrap_or_else(|| panic!("no RAM word in the scan window stayed 0 across {hops:?}"))
}

/// **The boot-time defense.** A `room-global` sidecar stamped with THIS image's
/// identity token (so it passes that check) but pointing at a word that holds
/// a real object of the story which is not a room — exactly the shape a
/// rebuild can leave behind — is refused before it is ever trusted: the
/// session boots UNLOCKED, exactly as it would with no sidecar at all, and the
/// TRUE `location` global resolves on the very first room-changing move (`n`,
/// out of the Back Alley) the same way it does with no sidecar in play.
#[test]
fn a_boot_time_sidecar_pointing_at_a_non_room_object_is_refused() {
    let Some(bytes) = story_bytes() else { return };

    // First: an ordinary boot, purely to learn the TRUE address and a wrong
    // candidate to write into the sidecar afterward.
    let scout_dir = app::scratch_dir("sq1305-boot-scout");
    let mut scout = boot_in(&scout_dir, bytes.clone());
    for c in PROLOGUE {
        let _ = step(&mut scout, c);
    }
    let _ = step(&mut scout, "n");
    let true_addr = scout.locked_room_global().expect("the first move resolves the lock");
    let wrong_addr = find_wrong_candidate(bytes.clone(), true_addr);
    assert_ne!(wrong_addr, true_addr, "the candidate search must not just hand back the truth");

    // Now the reproduction: a FRESH game dir whose sidecar was never learned by
    // this session at all — hand-written, as if left behind by an earlier
    // build of the story, pointing at `wrong_addr` with today's image's token.
    let (checksum, extstart) = image_identity(bytes.clone());
    let dir = app::scratch_dir("sq1305-boot-refused");
    std::fs::write(dir.join("room-global"), format!("{wrong_addr} {checksum:x}:{extstart:x}"))
        .expect("write the planted sidecar");

    let mut s = boot_in(&dir, bytes);
    assert_ne!(
        s.locked_room_global(),
        Some(wrong_addr),
        "a sidecar pointing at a non-room object must never be trusted, whatever its token says"
    );

    // Play exactly as the scout did, and the map must behave identically: the
    // opening room, then a resolved lock on the TRUE address after the first
    // move, then real room changes throughout.
    let mut names = vec![s.current_location().map(|l| l.name)];
    for c in PROLOGUE.iter().chain(HOPS.iter()) {
        let r = step(&mut s, c);
        names.push(r.location.map(|l| l.name));
    }
    assert_eq!(
        s.locked_room_global(),
        Some(true_addr),
        "the lock must resolve to the story's real `location` global, not the planted address"
    );
    let distinct: HashSet<_> = names.into_iter().flatten().collect();
    assert!(
        distinct.len() >= 3,
        "the walk visits Back Alley, Sigil Street and Ampersand Bend: {distinct:?}"
    );
}

/// **The frozen-lock recovery.** Simulates what a DEFEATED boot-time defense
/// would have let through: the lock is forced onto a word that reads `0`
/// throughout the whole walk via [`GlulxSession::relock_room_global`] — the
/// same call a live shadow-sync uses, carrying no plausibility check of its
/// own by design (see its docs) — immediately before the walk, so this test is
/// independent of whichever route got a bad address installed. A perpetual
/// zero is deliberate: `RoomLock::verify`'s pre-existing object-value check
/// explicitly treats `0` as "no evidence either way" and never falsifies it,
/// so this isolates the NEW frozen-heading check from that pre-existing one —
/// nothing else in `verify` can drop this particular lock. See [`FREEZE_HOPS`]
/// for why the route shuttles between Sigil Street and Ampersand Bend rather
/// than reusing [`HOPS`]'s wider loop through Back Alley.
#[test]
fn a_lock_frozen_on_a_perpetual_zero_recovers_within_a_few_turns() {
    let Some(bytes) = story_bytes() else { return };

    let scout_dir = app::scratch_dir("sq1305-freeze-scout");
    let mut scout = boot_in(&scout_dir, bytes.clone());
    for c in PROLOGUE {
        let _ = step(&mut scout, c);
    }
    let _ = step(&mut scout, "n");
    let true_addr = scout.locked_room_global().expect("the first move resolves the lock");
    let wrong_addr = find_stuck_zero_candidate(bytes.clone(), true_addr, FREEZE_HOPS);

    let dir = app::scratch_dir("sq1305-freeze-live");
    let mut s = boot_in(&dir, bytes);
    for c in PROLOGUE {
        let _ = step(&mut s, c);
    }
    s.relock_room_global(wrong_addr);
    assert_eq!(s.locked_room_global(), Some(wrong_addr), "precondition: the bad lock took");

    let mut locations: Vec<Option<String>> = Vec::new();
    let mut transcript = String::new();
    for c in FREEZE_HOPS {
        let r = step(&mut s, c);
        transcript.push_str(&r.transcript);
        locations.push(r.location.map(|l| l.name));
    }

    // Non-vacuity, read from the STORY's own words rather than the (frozen)
    // engine location: the walk must actually have printed several distinct
    // room headings, or "the map still moves" would hold vacuously because
    // nothing tried to move it anywhere.
    for room in ["Sigil Street", "Ampersand Bend"] {
        assert!(transcript.contains(room), "the route must actually reach {room:?}: {transcript}");
    }

    // The bug this reproduces: while the lock is frozen, `current_location`
    // does not follow those headings at all — `adopt_heading_for_room` refuses
    // every `Unchanged`-per-the-lock turn, so the engine repeats whatever room
    // it was parked on when the bad lock took. If this holds for the WHOLE
    // route, the freeze never lifted and the fix below did nothing.
    let distinct_locations: HashSet<&Option<String>> = locations.iter().collect();
    assert!(
        distinct_locations.len() >= 2,
        "the engine's own idea of the current room must change by the end of the route, not \
         stay frozen on whatever it was when the bad lock took: {locations:?}"
    );

    // The reproduction's resolution: by the end of the route the lock must
    // have dropped the frozen address and re-resolved to the STORY's real
    // `location` global — the map recovers on its own within a few turns
    // rather than staying wrong for the rest of the session.
    assert_eq!(
        s.locked_room_global(),
        Some(true_addr),
        "a lock frozen on a non-room object must drop itself and relearn the true address"
    );
}
