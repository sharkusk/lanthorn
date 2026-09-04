//! SQ-1285: a bolded object name at line start is not a room.
//!
//! Reported from the field: *"I went to the Midway and removed p from pear and apple.
//! Then I typed GET ALL, then lanthorn detected a new room called 'ear'."* The map dump
//! that came with it held six rooms, five real and one phantom — `ROOM 39859 "ear"`,
//! hanging off `ROOM 49156 "Midway"` on a `?` (no-direction) edge, with three
//! `ear → Fair` edges minted afterwards as the player walked away from a room that never
//! existed. `39859` is `app::roomid::synthetic_room_id("ear")` exactly, so the name came
//! off a room HEADING: Glulx has no object tree, and `GlulxSession::room_for` falls back
//! to the name hash whenever the room-lock has not resolved (it never does for this
//! story).
//!
//! # Why a severed ear looked like a room
//!
//! Counterfeit Monkey advertises an accessibility option, `HIGHLIGHT` (`hilight`, and
//! `LOOK CAREFULLY` for one turn), whose whole job is to print the names of manipulable
//! objects in bold — *"The names of manipulable objects are especially important in this
//! game, and it's possible for the reader to miss clues through careless reading"*
//! (`Presentation Details.i7x`, "Section 4 - Bolding Help": `Before printing the name of
//! something which is not a quip when boldening is true: say "[bold type]"`). Inform's
//! Glk layer carries bold type as `style_Subheader` — **the same style the room heading
//! is printed in**, which is the only reason heading detection works at all. So with
//! HIGHLIGHT on, the multi-object take listing reads
//!
//! ```text
//! ale: We acquire the ale.
//! ear: We take the ear.
//! ```
//!
//! with `ale` and `ear` in `Subheader` at line start, followed by the parser's own
//! command prompt: an own-line `Subheader` run, joined to the text below it, at the
//! command prompt — every condition the pre-SQ-1285 rule asked of a room.
//!
//! The fix is [`app::glk_backend`]'s `StoryScan::line_rest_disqualifies`: an Inform room
//! heading OWNS its line (at most the library's roman `(on the chair)` after it), where a
//! bolded noun merely OPENS a sentence. Falsified by reverting that check — the `get all`
//! turn then reports `#39859 "ear"` and the mapper mints the phantom, which is precisely
//! the two assertions below.
//!
//! # The fixture, and how this harness got there
//!
//! `stories/CounterfeitMonkey-11.gblorb` — Counterfeit Monkey, **release 11 / serial
//! 230220** / Inform 7 build 6M62. Gitignored, so this skips vacuously without it.
//!
//! The route is [`ROUTE`] below, **18 inputs from a cold boot**, and it is not guessable
//! from play: it is the game's own `Test gel` script (`Tests.i7x`, and
//! `tools/command scripts/test_me.txt` in the i7/counterfeit-monkey repository), trimmed
//! to the fruit. Three of those inputs are the prologue — `y`, `andra`, then a KEYPRESS,
//! not a command — and `highlight` is the load-bearing one: with it off the listing is
//! roman, nothing is a heading candidate, and this suite cannot fail however broken the
//! rule is. [`get_all_bolds_the_object_names`] is the guard on exactly that.
//!
//! The two fruit each need the letter P removed (the device takes every instance):
//! `apple` → `ale`, `pear` → `ear`. The apple must go FIRST — the pear sits in a pan of
//! the word-balance and the barker refuses to let it be taken while he is there, and
//! unbalancing the balance with the apple is what makes him leave.

use std::path::PathBuf;

use app::engine::{Engine, KeyInput};
use app::glulx_session::GlulxSession;
use app::session::{apply_turn, DeathWatch, InputKind, TurnResult};
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;

const STORY: &str = "CounterfeitMonkey-11.gblorb";

/// Every input from a cold boot to the reported turn, in order. `None` is a keypress
/// (the prologue's `custom-wait for any key`, which runs before the banner); everything
/// else is a line.
const ROUTE: &[Option<&str>] = &[
    Some("y"),                       // "Can you hear me?" — the consent that starts play
    Some("andra"),                   // "Do you remember our name?"
    None,                            // the keypress before the banner
    Some("tutorial off"),            // otherwise every turn carries a tutorial nag
    Some("pauses off"),              // an out-of-world command; no [MORE]-style waits
    Some("highlight"),               // ← boldening ON: the whole point of the fixture
    Some("n"),                       // Back Alley → Sigil Street
    Some("e"),                       // Sigil Street → Ampersand Bend
    Some("wave x-remover at codex"), // the museum's codex → a code reading "305"
    Some("unlock barrier"),          // the game dials 305 itself once we know it
    Some("n"),                       // Ampersand Bend → Fair
    Some("w"),                       // Fair → Midway
    Some("wave p-remover at apple"), // apple → ale; the balance tips, the barker leaves
    Some("wave p-remover at pear"),  // pear → ear
    Some("get all"),                 // ← the reported turn
];

/// Where `get all` sits in [`ROUTE`].
const GET_ALL: usize = ROUTE.len() - 1;

/// The room the player is really in for the last three turns.
const MIDWAY: &str = "Midway";
/// The object `pear` becomes, and the phantom room it used to mint.
const EAR: &str = "ear";

/// Boot the story and play [`ROUTE`], returning every turn's result alongside the
/// command that produced it. `None` when the gitignored fixture is absent.
fn play() -> Option<Vec<(String, TurnResult)>> {
    let path = fixture_path(STORY);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let pict_blorb = blorb::Blorb::parse(bytes.clone()).ok();
    let app::hints::LoadedStory::Glulx(image) =
        app::hints::extract_story(bytes).expect("CounterfeitMonkey-11.gblorb is a readable container")
    else {
        panic!("{STORY} is a Glulx story");
    };
    // No persistent store: the game's own fixed-name startup cache auto-fails, so this
    // runs its full initialisation every time and depends on nothing left on disk.
    let mut s = GlulxSession::new(image, 80, 30, true, false, false, (8, 16), pict_blorb, &[])
        .expect("Counterfeit Monkey boots");
    let _ = s.take_transcript();

    let mut turns = Vec::new();
    for (i, step) in ROUTE.iter().enumerate() {
        let (label, result) = match step {
            Some(cmd) => {
                assert_eq!(
                    s.pending_input(),
                    InputKind::Line,
                    "route step {i} ({cmd:?}) expects a line prompt"
                );
                ((*cmd).to_string(), s.submit(cmd))
            }
            None => {
                assert_eq!(
                    s.pending_input(),
                    InputKind::Char,
                    "route step {i} expects the prologue's keypress prompt"
                );
                ("<key>".to_string(), s.submit_key(KeyInput::Enter).expect("Glulx takes keys"))
            }
        };
        turns.push((label, result));
    }
    Some(turns)
}

/// The `Subheader` runs of one turn's transcript, joined per contiguous run.
fn subheader_runs(t: &TurnResult) -> Vec<String> {
    let chars: Vec<char> = t.transcript.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut at = 0usize;
    let mut prev_sub = false;
    for run in &t.transcript_runs {
        let end = (at + run.0).min(chars.len());
        let text: String = chars[at.min(chars.len())..end].iter().collect();
        at += run.0;
        // glk.h: `style_Subheader` = 4. The style the room heading — and, with
        // HIGHLIGHT on, every object name — is printed in.
        let sub = run.6 == 4;
        match (sub, prev_sub) {
            (true, true) => out.last_mut().expect("a previous run").push_str(&text),
            (true, false) => out.push(text),
            _ => {}
        }
        prev_sub = sub;
    }
    out
}

/// Non-vacuity, and the whole reason this fixture is Counterfeit Monkey: the `get all`
/// turn really does print the object names in the room-heading style. Without HIGHLIGHT
/// the listing is roman and neither assertion below could fail.
#[test]
fn get_all_bolds_the_object_names() {
    let Some(turns) = play() else { return };
    let (cmd, get_all) = &turns[GET_ALL];
    assert_eq!(cmd, "get all");
    let bolds = subheader_runs(get_all);
    assert!(
        bolds.iter().any(|b| b == EAR),
        "HIGHLIGHT should print `ear` in the heading style; the listing was {:?} with bolds {bolds:?}",
        get_all.transcript
    );
    assert!(
        get_all.transcript.contains("ear:"),
        "the multi-object listing should open a line with the object's name: {:?}",
        get_all.transcript
    );
}

/// The engine's own answer: `get all` in the Midway leaves the player in the Midway.
#[test]
fn get_all_does_not_move_the_player_to_a_room_named_after_a_taken_object() {
    let Some(turns) = play() else { return };
    // Non-vacuity: the route really did reach the Midway, and really did make an ear.
    let arrival = turns
        .iter()
        .position(|(c, t)| c == "w" && t.location.as_ref().is_some_and(|l| l.name == MIDWAY))
        .expect("the route should walk into the Midway");
    assert!(arrival < GET_ALL, "the Midway comes before the reported turn");
    assert!(
        turns[GET_ALL - 1].1.transcript.contains("ear"),
        "the pear should have become an ear: {:?}",
        turns[GET_ALL - 1].1.transcript
    );

    let here = turns[GET_ALL].1.location.as_ref().expect("a room after `get all`");
    assert_eq!(
        here.name, MIDWAY,
        "`get all` is not a move: taking a severed ear must not relocate the player \
         (id #{}, `synthetic_room_id(\"ear\")` is #{})",
        here.number,
        app::roomid::synthetic_room_id(EAR),
    );
}

/// And the map's: no phantom room, and no `?` edge out of the Midway. This is the half
/// the player saw — `apply_turn` mints a no-direction edge for any room change on a
/// command that is not a direction (a real `xyzzy` teleport is the same shape), so
/// nothing downstream of the heading could have caught this one.
#[test]
fn the_map_gains_no_phantom_room_from_a_take() {
    let Some(turns) = play() else { return };
    let mut mapper = Mapper::default();
    let mut death = DeathWatch::default();
    for (cmd, result) in &turns {
        apply_turn(&mut mapper, cmd, result, &mut death);
    }

    let names: Vec<String> = mapper.graph.rooms().map(|r| r.label().to_string()).collect();
    // Non-vacuity: every room this route WALKS into is on the map. (Back Alley, the
    // sixth room of the reported dump, is where the game starts and prints no heading
    // until something asks it to, so this route never names it.)
    for want in [MIDWAY, "Fair", "Ampersand Bend", "Sigil Street"] {
        assert!(names.iter().any(|n| n == want), "{want} should be mapped: {names:?}");
    }
    assert!(
        !names.iter().any(|n| n == EAR),
        "a severed ear in the player's hands is not a room: {names:?}"
    );
    let midway = mapper
        .graph
        .rooms()
        .find(|r| r.label() == MIDWAY)
        .map(|r| r.id)
        .expect("the Midway is mapped");
    let stray: Vec<_> = mapper
        .graph
        .connections()
        .iter()
        .filter(|c| c.origin == midway && c.dir == mapper::direction::Direction::Unknown)
        .collect();
    assert!(stray.is_empty(), "`get all` is not a teleport out of the Midway: {stray:?}");
}
