//! SQ-1294, the other half: a heading printed for a place the player never went.
//!
//! The car driving out of Deep Street moves the player and prints no heading
//! (`sq1294_glulx_silent_vehicle_move`). This is the mirror image — a heading printed
//! for somewhere the player is not — and it used to break the map the same way, for
//! the same reason: `GlulxSession` derived "did the room change?" from the HEADING and
//! `RoomLock::verify` then threw the lock away for disagreeing with it.
//!
//! Counterfeit Monkey's `REMEMBER` is a flashback. `remember lock`, standing in the
//! Dormitory Room, prints
//!
//! ```text
//! Galley
//! You were going through the galley cupboards on the yacht. "If you're looking for
//! coffee, Slango forgot to resupply," Brock said, ...
//!
//! Then we're back in the present.
//! ```
//!
//! An own-line `Subheader` joined to prose at the command prompt: a room heading by
//! every test there is, and the player has not moved an inch. Measured on 0.4.3, the
//! damage was both halves at once — the map minted a `Galley` room and reported the
//! player in it for the next five turns, AND the lock was dropped, so the nineteen
//! turns until it re-resolved were keyed by name hash and duplicated the Hostel and
//! the Dormitory Room on the map.
//!
//! Now the lock decides. Its word did not change, so the turn was not a move, so the
//! heading names nothing — see `GlulxSession::adopt_heading_for_room`. A room the
//! story really does RENAME is re-read the next time the player walks into it, which
//! is the only moment a new name can be told from a memory.
//!
//! # The fixture and the route
//!
//! `stories/CounterfeitMonkey-11.gblorb` — release 11 / serial 230220 / Inform 7 build
//! 6M62. Gitignored, so this skips vacuously without it.
//!
//! [`ROUTE`] is **39 inputs from a cold boot**: the game's own `test me` script
//! (`tools/command scripts/test_me.txt` in the i7/counterfeit-monkey repository) as far
//! as `remember lock` — the locker in the hostel dormitory is what gives the player a
//! memory to have — with a closing `look` so the case can say where they really are.

use app::engine::{Engine, KeyInput};
use app::glulx_session::GlulxSession;
use app::session::{apply_turn, DeathWatch, InputKind, TurnResult};
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;

const STORY: &str = "CounterfeitMonkey-11.gblorb";

/// Every input from a cold boot, one per line. An EMPTY line is the prologue's
/// keypress (the `custom-wait for any key` before the banner), not a blank command.
const ROUTE: &str = "\
y\n\
andra\n\
\n\
tutorial off\n\
random-seed 1234\n\
pauses off\n\
n\n\
wave u-remover at mourning dress\n\
score\n\
e\n\
wave x-remover at codex\n\
x code\n\
unlock barrier\n\
set barrier to 305\n\
go to fair\n\
x wheel\n\
wave w-remover at wheel\n\
get heel\n\
look\n\
score\n\
go to midway\n\
wave p-remover at apple\n\
get pear\n\
wave e-remover at tube\n\
open tub\n\
put gel on pear\n\
put gel on ale\n\
get apple\n\
wave l-remover at pearl\n\
wave p-remover at pear\n\
e\n\
go to garden\n\
wave h-remover at thicket\n\
get all\n\
go to hostel\n\
up\n\
x lock\n\
remember lock\n\
look";

/// Where the player is standing for the whole flashback.
const DORMITORY: &str = "Dormitory Room";
/// The heading the flashback prints — a yacht galley, hundreds of miles and some
/// months away.
const GALLEY: &str = "Galley";

fn steps() -> Vec<&'static str> {
    ROUTE.lines().collect()
}

/// Boot and play [`ROUTE`], returning the session and every turn paired with its input.
fn play() -> Option<(GlulxSession, Vec<(&'static str, TurnResult)>)> {
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
    let mut s = GlulxSession::new(image, 80, 30, true, false, false, (8, 16), pict_blorb, &[])
        .expect("Counterfeit Monkey boots");
    let _ = s.take_transcript();

    let mut turns = Vec::new();
    for (i, cmd) in steps().into_iter().enumerate() {
        let r = if s.pending_input() == InputKind::Char {
            s.submit_key(KeyInput::Enter).expect("Glulx takes keys")
        } else {
            assert_eq!(s.pending_input(), InputKind::Line, "route step {i} ({cmd:?}) wants a line");
            s.submit(cmd)
        };
        turns.push((cmd, r));
    }
    Some((s, turns))
}

/// One route, every assertion.
#[test]
fn a_flashback_heading_neither_moves_the_player_nor_costs_the_lock() {
    let Some((s, turns)) = play() else { return };
    let last = steps().len() - 1;
    let flashback = last - 1;

    // ── Non-vacuity: the route reached the dormitory with the lock resolved, and
    //    the flashback really did print the galley as an own-line heading ─────────
    assert_eq!(turns[flashback].0, "remember lock");
    let before = turns[flashback - 1].1.location.as_ref().expect("a room before the memory");
    assert_eq!(before.name, DORMITORY, "the memory is had in the hostel dormitory");
    assert_ne!(
        before.number,
        app::roomid::synthetic_room_id(DORMITORY),
        "keyed by ADDRESS, so the lock was resolved before the memory"
    );
    let t = &turns[flashback].1;
    assert!(
        subheaders(t).iter().any(|b| b == GALLEY),
        "the flashback must print `{GALLEY}` in the room-heading style, or this case \
         proves nothing: {:?}",
        subheaders(t)
    );
    assert!(
        t.transcript.contains("Then we're back in the present"),
        "and it must be a memory rather than a move: {:?}",
        t.transcript
    );

    // ── The engine's answer: the player never left the dormitory ────────────────
    let here = t.location.as_ref().expect("a room after the memory");
    assert_eq!(here.name, DORMITORY, "a memory of a yacht is not a room the player is in");
    assert_eq!(here.number, before.number, "and not a different one either");
    assert_eq!(
        turns[last].1.location.as_ref().map(|l| l.name.as_str()),
        Some(DORMITORY),
        "the `look` that follows confirms it from the story's own mouth"
    );
    assert!(s.locked_room_global().is_some(), "and the lock survived being right");

    // ── The map's half ─────────────────────────────────────────────────────────
    let mut mapper = Mapper::default();
    let mut death = DeathWatch::default();
    for (cmd, r) in &turns {
        apply_turn(&mut mapper, cmd, r, &mut death);
    }
    let names: Vec<String> = mapper.graph.rooms().map(|r| r.label().to_string()).collect();
    assert!(!names.iter().any(|n| n == GALLEY), "no room is minted for a memory: {names:?}");
    assert_eq!(
        names.iter().filter(|n| n.as_str() == DORMITORY).count(),
        1,
        "and no duplicate is minted for the room the player is really in: {names:?}"
    );
}

/// Contiguous `Subheader` (glk.h `style_Subheader` = 4) runs of a turn's transcript that
/// BEGIN at a line start — the shape `StoryScan` considers a room heading.
fn subheaders(t: &TurnResult) -> Vec<String> {
    let chars: Vec<char> = t.transcript.chars().collect();
    let mut spans: Vec<(usize, String)> = Vec::new();
    let mut at = 0usize;
    let mut prev_sub = false;
    for run in &t.transcript_runs {
        let end = (at + run.0).min(chars.len());
        let text: String = chars[at.min(chars.len())..end].iter().collect();
        let sub = run.6 == 4;
        if sub {
            match spans.last_mut() {
                Some(last) if prev_sub => last.1.push_str(&text),
                _ => spans.push((at, text)),
            }
        }
        at += run.0;
        prev_sub = sub;
    }
    spans
        .into_iter()
        .filter(|(start, _)| *start == 0 || chars.get(start - 1) == Some(&'\n'))
        .map(|(_, text)| text)
        .collect()
}
