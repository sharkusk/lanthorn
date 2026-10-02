//! Room description capture (SQ-1625): the mapper's most-recent-description-per-room feature,
//! end to end through a real Z-machine session. Real-game cases skip vacuously without
//! `stories/` (gitignored), the CI-safe pattern documented in `fixture_paths.rs`.
//!
//! Only the Z-machine (Tier 3, bounded scope) is exercised here with a real story — Scott's
//! `room_description_text` and Glk's `StoryScan` extension are unit-tested directly in their own
//! crates (`crates/scott/src/vm.rs`, `crates/app/src/glk_backend.rs`), where a synthetic fixture
//! is exact and immediate rather than dependent on a commercial story's own prose.

use crate::fixture_paths::fixture_path;

use app::engine::{Engine, KeyInput};
use app::glulx_session::GlulxSession;
use app::session::GameSession;

fn story(name: &str) -> Option<Vec<u8>> {
    std::fs::read(fixture_path(name)).ok()
}

/// `stories/zork1-r88-s840726.z3`: release 88 / serial 840726, a Version 3 story with the
/// ordinary status-line-plus-one-scrolling-window convention (`docs/internals/interpreter.md`).
///
/// The BOOT's own opening-room print never reaches `description` at all — `Engine::seed_turn`
/// deliberately drains only `location`/`quit`/`erase_lower` (see its own doc: "the three OTHER
/// per-turn facts... were looked for and are not there"), never the transcript `drain_turn`
/// builds `description` from. So this drives the FIRST real turn (`submit`, exactly the path
/// `finish_command_turn` uses) rather than the boot frame — an explicit LOOK, which is one of
/// this feature's two named triggers in its own right, not a stand-in for the other.
#[test]
fn single_window_zmachine_arrival_and_look_capture_a_description() {
    let Some(bytes) = story("zork1-r88-s840726.z3") else {
        eprintln!("SKIP: gitignored stories/zork1-r88-s840726.z3 missing");
        return;
    };
    let mut s = GameSession::new_with_trace(bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)))
        .expect("zork1 boots without a ZError");
    assert!(s.machine.screen.v6.is_none(), "zork1 is not a v6 story");
    // Drain and discard the boot's own banner/opening print first (the same idiom
    // `declared_exit.rs`'s `Play::for_story` uses) — `Engine::seed_turn` never touches
    // `description` at all (see this test's own doc), so the boot's undrained buffer would
    // otherwise land INSIDE the first real `submit`'s transcript and double-print the room.
    let _ = s.submit("");

    // Explicit LOOK in the boot's starting room.
    let r1 = s.submit("look");
    let loc1 = r1.location.clone().expect("zork1 names a room");
    assert_eq!(loc1.name, "West of House");
    let desc1 = r1.description.clone().expect("an explicit LOOK must capture a description");
    assert!(!desc1.is_empty());
    assert!(!desc1.contains("West of House"), "the heading itself must not reappear in the body: {desc1:?}");
    assert!(!desc1.to_lowercase().contains("obvious exits"), "no exits-section leakage: {desc1:?}");

    // A second LOOK re-captures the SAME room's text, replacing rather than accumulating.
    let r2 = s.submit("look");
    let desc2 = r2.description.expect("a second LOOK re-captures the description");
    assert_eq!(desc2, desc1, "West of House's own text is unchanged, and not doubled, by a second look");

    // A genuine ARRIVAL — walking north into a different, differently-described room —
    // captures ITS OWN text, distinct from West of House's.
    let r3 = s.submit("north");
    let loc3 = r3.location.expect("the walk north names a room");
    assert_ne!(loc3.name, "West of House", "the walk actually left the starting room");
    let desc3 = r3.description.expect("a genuine arrival must capture a description");
    assert_ne!(desc3, desc1, "a different room's description must not be the old room's leftover text");
}

// ── Glulx real-game coverage (SQ-1639) ────────────────────────────────────────
//
// Every case above is Z-machine only (see the module doc's own note on that).
// `AppGlk::take_room_description`'s state machine (`glk_backend.rs`) is a pure
// transcript heuristic — there is no structural "this is prose" signal in Glulx
// the way the Z-machine's own status-line convention sometimes gives one — and
// until this quest it had ZERO real-game coverage. Seven games, deliberately
// varied: a game whose room reprint carries no leading blank line at all
// (Anchorhead — the SQ-1639 fix below), a multi-paragraph description with a
// trailing call-to-action (Counterfeit Monkey), a short single-paragraph one
// (Coloratura), one with a randomized trailing flourish (Sub Rosa), one behind
// a title-card/letter/picture-placeholder banner (King of Shreds and Patches),
// one with a non-plain-line, parenthetical-author heading behind a content
// warning gate (Cragne Manor), and one that never prints a `Subheader` heading
// at all — the room name comes only from the status line (Wizard Sniffer,
// SQ-1302) — pinning that the gate declines cleanly rather than guessing.

/// The Glulx image inside a Blorb, or a bare `.ulx`/`.gblorb` passed through —
/// the same helper `glulx_inventory.rs`/`item_tracking.rs` use. `None` when the
/// gitignored fixture is absent.
fn glulx_image(name: &str) -> Option<Vec<u8>> {
    let path = fixture_path(name);
    let bytes = std::fs::read(&path).ok()?;
    if !blorb::Blorb::is_blorb(&bytes) {
        return Some(bytes);
    }
    let b = blorb::Blorb::parse(bytes).ok()?;
    match b.executable() {
        Ok((blorb::ExecKind::Glulx, data)) => Some(data.to_vec()),
        _ => None,
    }
}

/// Boot a Glulx story past any "press any key" splash to the first command
/// prompt, exactly like `glulx_inventory.rs`'s `boot`/`item_tracking.rs`'s
/// `boot_cm`. `random_seed` is `None` throughout this file's helpers (gvm's own
/// fixed default — see `GlulxSession::new_in`'s own doc: "leaves gvm's own fixed
/// default, so a test's sequence stays the reproducible one it has always
/// been"), which is why Sub Rosa's randomized ambient flourish below can be
/// pinned exactly rather than merely shown to vary.
fn glulx_boot(name: &str) -> Option<GlulxSession> {
    let image = glulx_image(name)?;
    let mut s = GlulxSession::new(image, 80, 24, true, false, false, (1.0, 1.0), None, &[]).ok()?;
    for _ in 0..6 {
        if s.pending_input() != app::session::InputKind::Char {
            break;
        }
        s.submit_key(KeyInput::Enter);
    }
    Some(s)
}

/// Anchorhead (Michael Gentry, Inform 6/Glulx): `stories/Anchorhead.gblorb`.
///
/// **The SQ-1639 fix, at the game that exposed it.** Anchorhead's own room
/// reprint on an explicit LOOK prints its `Subheader` heading with NO leading
/// blank line at all — unlike the Inform 7 standard library's own room-name
/// rule, which always calls `new_line()` first. Every turn ends at the game's
/// bare `">"` read prompt (never a trailing newline), so `StoryScan::
/// at_line_start` was left `false` from one turn into the next; the heading's
/// own `Subheader` run then began "mid-line" by that stale reckoning — exactly
/// the test `capture_heading`'s own doc uses to recognize an INLINE hyperlink —
/// and was silently ignored, taking the SQ-1625 body paired with it down with
/// it. Confirmed by temporarily reverting `AppGlk::begin_command_line` and its
/// two call sites (`GlulxSession::submit`'s Line branch, `silent_look`): this
/// exact test then fails with `description=None` on every turn after the
/// first, reproducing the originally observed symptom before the fix restores
/// it (falsified per this repo's testing conventions).
#[test]
fn glulx_anchorhead_recaptures_a_description_with_no_leading_blank_line_between_turns() {
    let Some(mut s) = glulx_boot("Anchorhead.gblorb") else {
        eprintln!("SKIP: gitignored stories/Anchorhead.gblorb missing");
        return;
    };

    let r1 = s.submit("look");
    let loc1 = r1.location.clone().expect("Anchorhead names its opening room");
    assert_eq!(loc1.name, "Outside the Real Estate Office");
    let desc1 = r1.description.clone().expect("an explicit LOOK must capture a description");
    assert!(desc1.contains("grim little cul-de-sac"), "the real body text: {desc1:?}");
    assert!(!desc1.contains("Outside the Real Estate Office"), "the heading must not reappear in the body: {desc1:?}");

    // A second LOOK re-captures the SAME room's text — the exact case the bug
    // broke, since the transcript between turns is nothing but a bare ">".
    let r2 = s.submit("look");
    let desc2 = r2.description.expect("a second LOOK, with no leading blank line, must still re-capture the description");
    assert_eq!(desc2, desc1, "the same room's own text, not doubled and not lost");

    // A genuine arrival into a different room (there is no working `north` exit
    // here, so this drives `southeast` into the alley) captures ITS OWN text.
    let r3 = s.submit("southeast");
    let loc3 = r3.location.expect("the walk southeast names a room");
    assert_eq!(loc3.name, "Garbage-Choked Alley");
    let desc3 = r3.description.expect("a genuine arrival must capture a description");
    assert_ne!(desc3, desc1, "a different room's description must not be the old room's leftover text");
    assert!(desc3.contains("rotting cardboard boxes"), "the alley's real body text: {desc3:?}");
}

/// Counterfeit Monkey (Emily Short, Inform 7 6M62, IF Archive release 10):
/// multi-paragraph description with a trailing call-to-action paragraph — the
/// stress case for a JOINED body that spans several blank-line-separated
/// paragraphs before the read prompt, not just one line of prose.
#[test]
fn glulx_counterfeit_monkey_captures_a_multi_paragraph_description_intact() {
    let Some(mut s) = glulx_boot("CounterfeitMonkey-11.gblorb") else {
        eprintln!("SKIP: gitignored stories/CounterfeitMonkey-11.gblorb missing");
        return;
    };
    for cmd in ["yes", "yes", "yes"] {
        s.submit(cmd);
    }
    s.submit_key(KeyInput::Enter);
    let _ = s.take_transcript();

    let r1 = s.submit("look");
    let loc1 = r1.location.clone().expect("CM names Back Alley once asked");
    assert_eq!(loc1.name, "Back Alley");
    let desc1 = r1.description.clone().expect("an explicit LOOK must capture a description");
    // The whole joined body, not truncated at the first paragraph break: the
    // opening line, the middle paragraph, AND the trailing hint paragraph.
    assert!(desc1.contains("peeling yellow paint"), "the opening paragraph: {desc1:?}");
    assert!(desc1.contains("This alley runs north"), "the middle paragraph: {desc1:?}");
    assert!(desc1.contains("LOOK AT THE YELLOW BUILDINGS"), "the trailing hint paragraph: {desc1:?}");
    assert!(!desc1.contains("Back Alley"), "the heading must not reappear in the body: {desc1:?}");

    // A second LOOK re-captures a description again (CM rephrases the opening
    // line on a re-look — "There is nothing here but..." — so this checks
    // re-capture happened at all, not verbatim equality).
    let r2 = s.submit("look");
    let desc2 = r2.description.expect("a second LOOK must still capture a description");
    assert!(desc2.contains("This alley runs north"), "still the same room's own text: {desc2:?}");

    // A genuine arrival into Sigil Street captures its own, different text.
    let r3 = s.submit("north");
    let loc3 = r3.location.expect("the walk north names a room");
    assert_eq!(loc3.name, "Sigil Street");
    let desc3 = r3.description.expect("a genuine arrival must capture a description");
    assert_ne!(desc3, desc1, "a different room's description must not be the old room's leftover text");
    assert!(desc3.contains("two and three stories"), "Sigil Street's real body text: {desc3:?}");
}

/// SQ-1663: Counterfeit Monkey's opening Q&A ("Can you hear me? >>" / "Do you
/// remember our name? >") closes and reopens its own windows (ids 1-3) the
/// instant the FIRST answer completes — it is rebuilding its intro overlay
/// into the real game's split-screen layout, a one-time transition. Our drain
/// model keeps a window's undrained log inside the window object itself, so
/// the `window_close` that retired the intro window silently destroyed
/// whatever had been printed there and never drained: the player's own "yes"
/// echo, "Can you hear me?"'s reply, AND the lead-in to the second question
/// all vanished from that turn's `TurnResult.transcript`, surfacing a turn
/// later as what looked like a missing line of story text. Confirmed with a
/// headless trace of every `put_text`/`window_close`/`window_open` call: the
/// undrained content was genuinely destroyed at the moment of closing, not
/// merely captured late.
#[test]
fn glulx_counterfeit_monkey_intro_reply_lands_in_the_same_turn_as_the_answer() {
    let Some(mut s) = glulx_boot("CounterfeitMonkey-11.gblorb") else {
        eprintln!("SKIP: gitignored stories/CounterfeitMonkey-11.gblorb missing");
        return;
    };
    assert_eq!(
        s.pending_input(),
        app::session::InputKind::Line,
        "CM's first prompt, 'Can you hear me? >>', reads a line"
    );

    // The turn that answers "Can you hear me?" must carry ITS OWN reply —
    // including the lead-in to the next question — not a bare prompt.
    let r1 = s.submit("yes");
    assert!(
        r1.transcript.contains("Good, you're conscious"),
        "SQ-1663: the reply to the FIRST question was dropped: {:?}",
        r1.transcript
    );
    assert!(
        r1.transcript.contains("Do you remember our name?"),
        "SQ-1663: the lead-in to the SECOND question was dropped: {:?}",
        r1.transcript
    );

    // The turn that answers the second question must carry ONLY ITS OWN
    // reply, not a leaked copy of the host's internal silent-look probe
    // (`GlulxSession::silent_look`, SQ-1293): CM's window close/reopen is
    // itself a one-time transition, so injecting a silent `look` at this
    // exact point triggers it too, and the probe's own output — which
    // `silent_look` promises to throw away — must still be discarded rather
    // than rescued as if it were real play.
    let r2 = s.submit("yes");
    assert!(
        r2.transcript.contains("Right, we're Alexandra now"),
        "the second question's real reply: {:?}",
        r2.transcript
    );
    assert!(
        !r2.transcript.contains("er, no"),
        "SQ-1663: the silent room-naming probe's own discarded output leaked into \
         a real turn: {:?}",
        r2.transcript
    );
}

/// Coloratura (Lynnea Glasser, Inform 7): a short, single-paragraph, poetic
/// description — the case with the least text to get wrong.
#[test]
fn glulx_coloratura_captures_a_short_description() {
    let Some(mut s) = glulx_boot("Coloratura.gblorb.blorb") else {
        eprintln!("SKIP: gitignored stories/Coloratura.gblorb.blorb missing");
        return;
    };

    let r1 = s.submit("look");
    let loc1 = r1.location.clone().expect("Coloratura names its opening room");
    assert_eq!(loc1.name, "Inside the Cellarium");
    let desc1 = r1.description.clone().expect("an explicit LOOK must capture a description");
    assert!(desc1.contains("crystalline Ancient structure"), "the real body text: {desc1:?}");
    assert!(!desc1.contains("Inside the Cellarium"), "the heading must not reappear in the body: {desc1:?}");

    let r2 = s.submit("look");
    assert_eq!(r2.description, Some(desc1), "a second LOOK re-captures the same, unchanging text");
}

/// Sub Rosa (Mike Gerwat, Inform 7): the fixed prose is followed by a
/// RANDOMIZED one-line ambient flourish that differs LOOK to LOOK — a game that
/// would look like "stale leftover text" to a weaker check if the two turns'
/// descriptions were compared for plain equality. Pinned exactly rather than
/// merely shown to vary, since `glulx_boot`'s `random_seed: None` is gvm's own
/// fixed default (see its own doc) — this sequence is the reproducible one gvm
/// has always produced, not a live roll.
#[test]
fn glulx_sub_rosa_description_carries_its_randomized_ambient_flourish_without_losing_the_fixed_prose() {
    let Some(mut s) = glulx_boot("Sub_Rosa.gblorb") else {
        eprintln!("SKIP: gitignored stories/Sub_Rosa.gblorb missing");
        return;
    };

    let r1 = s.submit("look");
    let loc1 = r1.location.clone().expect("Sub Rosa names its opening room");
    assert_eq!(loc1.name, "Leathery Cliff");
    let desc1 = r1.description.clone().expect("an explicit LOOK must capture a description");
    let fixed_prefix = "You stand calf-deep in mud beneath a leathery cliff";
    assert!(desc1.starts_with(fixed_prefix), "the fixed prose opens the body: {desc1:?}");
    assert!(!desc1.contains("Leathery Cliff"), "the heading must not reappear in the body: {desc1:?}");

    // A second LOOK re-captures the SAME fixed prose (not lost, not doubled)
    // plus a DIFFERENT randomized flourish (not stale leftover text from the
    // first LOOK — the two ambient lines below are known to differ under gvm's
    // fixed default seed).
    let r2 = s.submit("look");
    let desc2 = r2.description.expect("a second LOOK must still capture a description");
    assert!(desc2.starts_with(fixed_prefix), "the same fixed prose again: {desc2:?}");
    assert_ne!(desc2, desc1, "the randomized trailing flourish must actually differ turn to turn");
}

/// King of Shreds and Patches (Jimmy Maher, Inform 7 6M62/6.31): the room
/// print is preceded by a full title card, an in-fiction letter, a credits
/// block, AND a bracketed illustration placeholder (`[Picture number 67
/// here.]`) — all in the SAME turn's transcript, all of which must be excluded
/// from the captured description. The title menu wants `S` (start without the
/// tutorial), not a bare keypress, so this drives its own boot rather than
/// `glulx_boot`'s generic Enter loop.
#[test]
fn glulx_king_of_shreds_and_patches_description_excludes_the_letter_and_banner_that_precede_it() {
    let path = fixture_path("King_of_Shreds_and_Patches.gblorb");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored stories/King_of_Shreds_and_Patches.gblorb missing at {}", path.display());
        return;
    };
    let Ok(b) = blorb::Blorb::parse(bytes) else {
        eprintln!("SKIP: King_of_Shreds_and_Patches.gblorb did not parse as a Blorb");
        return;
    };
    let Ok((blorb::ExecKind::Glulx, image)) = b.executable() else {
        eprintln!("SKIP: King_of_Shreds_and_Patches.gblorb carries no Glulx executable");
        return;
    };
    let mut s = GlulxSession::new(image.to_vec(), 80, 24, true, false, false, (1.0, 1.0), None, &[])
        .expect("GlulxSession::new");
    assert_eq!(s.pending_input(), app::session::InputKind::Char, "premise: the title menu reads a keypress");
    let r1 = s.submit_key(KeyInput::Char('s')).expect("'s' starts the story without the tutorial");

    let loc1 = r1.location.clone().expect("the opening room is named on the very turn the title menu is dismissed");
    assert_eq!(loc1.name, "Fletcher's Printworks");
    let desc1 = r1.description.clone().expect("the title-menu-dismissing turn must still capture a description");
    assert!(desc1.contains("printing press"), "the real body text: {desc1:?}");
    assert!(!desc1.contains("Fletcher's Printworks"), "the heading must not reappear in the body: {desc1:?}");
    assert!(!desc1.contains("Dear friend"), "the in-fiction letter must not leak into the body: {desc1:?}");
    assert!(!desc1.contains("Release 12"), "the credits block must not leak into the body: {desc1:?}");
    assert!(!desc1.contains("Picture number"), "the illustration placeholder must not leak into the body: {desc1:?}");

    let r2 = s.submit("look");
    assert_eq!(r2.description, Some(desc1), "a second LOOK re-captures the same room text, banner-free again");
}

/// Cragne Manor (various authors, Inform 7 6M62): behind a two-step content-
/// warning gate, the opening room's own heading is NOT a plain single line —
/// `"Railway Platform (Naomi Hinchen)"` carries a parenthetical author credit
/// as part of the heading text itself.
#[test]
fn glulx_cragne_manor_captures_description_behind_a_parenthetical_author_heading() {
    let Some(mut s) = glulx_boot("cragne.gblorb") else {
        eprintln!("SKIP: gitignored stories/cragne.gblorb missing");
        return;
    };
    for cmd in ["yes", "yes"] {
        s.submit(cmd);
    }
    // "[press any key to begin]" — dismissed with a bare keypress, separately
    // from the LOOK that follows, so this exercises a clean, ordinary LOOK
    // rather than conflating the dismissal with it.
    assert_eq!(s.pending_input(), app::session::InputKind::Char, "premise: a keypress gates the story proper");
    s.submit_key(KeyInput::Enter);

    let r1 = s.submit("look");
    let loc1 = r1.location.clone().expect("Cragne Manor names its opening room");
    assert_eq!(loc1.name, "Railway Platform (Naomi Hinchen)");
    let desc1 = r1.description.clone().expect("an explicit LOOK must capture a description");
    assert!(desc1.contains("overhanging roof"), "the real body text: {desc1:?}");
    assert!(
        !desc1.contains("Railway Platform (Naomi Hinchen)"),
        "the whole parenthetical heading must not reappear in the body: {desc1:?}"
    );
    assert!(!desc1.contains("CONCEPT WARNING"), "the content-warning gate must not leak into the body: {desc1:?}");

    // A second LOOK re-captures the room's own text — checked by a stable
    // substring rather than exact equality, because Cragne Manor's clock
    // ("6:20 pm") advances every turn and would otherwise make this a flaky
    // pin on the game's own passage of time rather than on capture itself.
    let r2 = s.submit("look");
    let desc2 = r2.description.expect("a second LOOK must still capture a description");
    assert!(desc2.contains("overhanging roof"), "still the same room's own text: {desc2:?}");
}

/// The Wizard Sniffer (Buster Hudson, Inform 7 build 6L38): the negative case.
/// This game never prints a `Subheader` room heading in ANY style — its whole
/// presentation puts the room name in a two-row status grid instead
/// (`AppGlk::status_room_name`'s own doc names this exact game for SQ-1302).
/// `location` is still resolved correctly, from the status line — but with no
/// `Subheader` run ever confirmed, there is nothing for the SQ-1625 body
/// mechanism to pair a description with. Pinned here so the gate's silence is
/// a DOCUMENTED refusal rather than something that could quietly start
/// returning garbage.
#[test]
fn glulx_wizard_sniffer_names_the_room_from_the_status_line_but_never_captures_a_description() {
    let Some(mut s) = glulx_boot("The_Wizard_Sniffer.gblorb.blorb") else {
        eprintln!("SKIP: gitignored stories/The_Wizard_Sniffer.gblorb.blorb missing");
        return;
    };

    let r1 = s.submit("look");
    let loc1 = r1.location.clone().expect("the status line still names the room (SQ-1302)");
    assert_eq!(loc1.name, "Atop a Mountain");
    assert!(!r1.transcript.is_empty(), "the game does print prose — just never via a Subheader heading");
    assert_eq!(r1.description, None, "no Subheader heading is ever printed, so no body is ever paired with one");

    let r2 = s.submit("look");
    assert_eq!(r2.description, None, "still no description on a second LOOK — no crash, no garbage");
}
