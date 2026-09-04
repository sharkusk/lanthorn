//! SQ-1295: a bolded object name on the line BELOW a room heading must not eat the heading.
//!
//! # The report
//!
//! *"The game doesn't seem to recognize that Brown's Lab is a room with a name. The map
//! thinks there are a bunch of 'Samuel Johnson' rooms."*
//!
//! The dump that came with it:
//!
//! ```text
//! ROOM 34451 "Samuel Johnson Basement"   ROOM 38250 "Samuel Johnson Basement"
//! ROOM 43593 "Samuel Johnson Hall"       ROOM 46268 "Samuel Johnson Hall"
//! EDGE 34451 SW 38250      EDGE 38250 NE 38250      EDGE 38250 U 46268
//! ```
//!
//! `#34451` / `#43593` are `app::roomid::glulx_room_id` of the rooms' addresses;
//! `#38250` / `#46268` are `app::roomid::synthetic_room_id` of the same two NAMES. There
//! is no "Brown's Lab" room at all, and `38250 NE 38250` is the basement recorded as its
//! own destination.
//!
//! # What it is: a REGRESSION from SQ-1285, and only with HIGHLIGHT on
//!
//! Counterfeit Monkey's HIGHLIGHT accessibility option prints every manipulable object's
//! name in bold, which Inform's Glk layer carries as `style_Subheader` — the very style
//! the room heading uses. Brown's Lab opens with its NPC:
//!
//! ```text
//! **Brown's Lab**
//! **Professor Brown**, the Reification of Abstracts researcher, is hunched over ...
//! ```
//!
//! Walk that through `glk_backend`'s `StoryScan`. `finalize_heading` makes "Brown's Lab"
//! the PENDING candidate and `advance_heading_tail` moves it to `HeadingTail::LineEnd` —
//! one character away from being confirmed by the description below it. That character
//! is `Subheader` AT LINE START, so `capture_heading` starts a NEW heading run instead of
//! feeding it to `advance_heading_tail`, and the pending candidate is never confirmed.
//! "Professor Brown" then reaches `finalize_heading`, which **overwrites
//! `heading_pending` without settling what was already in it**, and SQ-1285's
//! `line_rest_disqualifies` correctly rejects it — taking the real room heading with it.
//!
//! Before SQ-1285 the same overwrite happened and the bogus candidate was CONFIRMED, so
//! the turn at least reported a room change (to a phantom room called "Professor Brown").
//! Now the turn reports nothing at all, which is worse in two ways:
//!
//! * the map keeps the previous room's name for a room the player has left; and
//! * `GlulxSession::finish_turn` derives `Movement::Unchanged` from the missing heading
//!   while the story's `location` global really did change, so `RoomLock::verify`
//!   contradicts itself and `relearn`s — **the resolved lock is thrown away**. Every room
//!   after that is keyed by name hash, which is the "bunch of Samuel Johnson rooms".
//!
//! The rule is about the LINE BELOW, so it is not confined to this one room: measured on
//! this fixture with HIGHLIGHT on, the same shape appears in Brown's Lab, Waterstone's
//! Office, Higgate's office and the Language Studies Department Office — every room whose
//! description opens with its NPC.
//!
//! # The fixture and the route
//!
//! `stories/CounterfeitMonkey-11.gblorb` — release 11 / serial 230220 / Inform 7 build
//! 6M62. Gitignored, so this skips vacuously without it.
//!
//! [`ROUTE`] is **237 inputs from a cold boot**: the game's own `test me` script
//! (`tools/command scripts/test_me.txt` in the i7/counterfeit-monkey repository) as far
//! as the Language Studies Seminar Room — the university is behind the car, which is
//! behind most of the first act — with `highlight` inserted after `pauses off`, and the
//! player's two commands at the end. **`highlight` is load-bearing**: with it off the
//! description below the heading is roman, the heading is confirmed, and nothing here can
//! fail however broken the rule is. [`the_arrival_bolds_the_npc_name_below_the_heading`]
//! is the guard on exactly that.

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
highlight\n\
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
open locker\n\
x girl\n\
show ear to girl\n\
turn dial\n\
x dial\n\
d\n\
talk to attendant\n\
ask attendant about locker\n\
up\n\
put gel on dial\n\
get clock\n\
open locker\n\
get all\n\
x letter\n\
remember\n\
x plans\n\
wave l-remover at plans\n\
wave s-remover at pans\n\
d\n\
w\n\
w\n\
go to cinema\n\
give ticket to ticket-taker\n\
w\n\
get all\n\
open backpack\n\
wear monocle\n\
n\n\
get jotter\n\
go to hesychius street\n\
talk to farmer\n\
ask about sale\n\
buy asparagus\n\
buy lime\n\
wave m-remover at lime\n\
n\n\
ne\n\
get chard\n\
look\n\
sw\n\
go to crumbling wall\n\
get fossil\n\
wave s-remover at fossil\n\
wave f-remover at foil\n\
go to close\n\
put gel on pan\n\
wave l-remover at plans\n\
put pans on spinner\n\
go to beach\n\
get funnel\n\
wave n-remover at funnel\n\
go to high street\n\
z\n\
go to high street\n\
wave h-remover at chard\n\
wave d-remover at card\n\
put fuel in car\n\
wave b-remover at garbage\n\
talk to mechanic\n\
ask mechanic about car\n\
give oil to mechanic\n\
go to counterfeit monkey\n\
e\n\
take off monocle\n\
put all in backpack\n\
close backpack\n\
go to counterfeit monkey\n\
open backpack\n\
y\n\
go to outdoor\n\
open backpack\n\
look\n\
wave p-remover at spill\n\
get sill\n\
go to counterfeit monkey\n\
talk to barman\n\
ask about slango\n\
challenge parker about the rum\n\
ask about paste\n\
ask how\n\
play\n\
show sill to barman\n\
put gel on sill\n\
wave s-remover at spill\n\
show pill to barman\n\
put gel on pill\n\
wave p-remover at spill\n\
get sill\n\
get paste\n\
go to tin hut\n\
prop trap door with sill\n\
down\n\
open crate\n\
get all\n\
wave r-remover at crate\n\
get cate\n\
up\n\
get sill\n\
go to aquarium\n\
ask whether\n\
say who\n\
x contraband\n\
wave m-remover at modems\n\
wave s-remover at odes\n\
put gel on ode\n\
wave m-remover at modems\n\
wave s-remover at preamps\n\
wave p-remover at preamp\n\
put paste on ream\n\
put paste on odes\n\
ask whether\n\
encourage lena\n\
put gel on as\n\
go to counterfeit monkey\n\
x slango\n\
say who\n\
explain\n\
z\n\
a trouble\n\
complain\n\
look\n\
e\n\
go to convenience\n\
get out\n\
put paste on car\n\
get out\n\
z\n\
get rifle\n\
shoot tree\n\
drop rifle\n\
enter car\n\
go to oval\n\
go to convenience\n\
wave s-remover at sink\n\
get ink\n\
x hole\n\
look in hole\n\
look at ash through monocle\n\
smell ash\n\
gel ash\n\
get trash\n\
go to rotunda\n\
get bin\n\
go to antiques\n\
x maps\n\
get slangovia map\n\
buy slangovia map\n\
go to drinks club\n\
show legend to bartender\n\
x legend\n\
go to palm square\n\
w\n\
w\n\
s\n\
get problem of adjectives\n\
n\n\
t girlfriend\n\
claim\n\
z\n\
reassure\n\
n\n\
get key\n\
get ring\n\
s\n\
ne\n\
go to babel\n\
go to palm\n\
unlock gate with ring\n\
go to oval\n\
say no\n\
encourage\n\
ask how consciousness\n\
s\n\
se\n\
n\n\
ask why\n\
ask whether\n\
x printer\n\
open printer\n\
go to graduate student\n\
get sticky\n\
wave y-remover at sticky\n\
open fridge\n\
get cream\n\
wave c-remover at cream\n\
go to language studies department office\n\
put ream in printer\n\
close printer\n\
get draft\n\
w\n\
no\n\
ask how\n\
ask what\n\
ask about seminar\n\
t book\n\
put problem on shelves\n\
go to samuel johnson basement\n\
sw";

/// The room the last input walks into.
const BROWNS_LAB: &str = "Brown's Lab";
/// The room it walks out of — and the name the map wrongly keeps.
const BASEMENT: &str = "Samuel Johnson Basement";
/// The bolded NPC name that opens Brown's Lab's description.
const PROFESSOR_BROWN: &str = "Professor Brown";

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

/// Where the walk into Brown's Lab is in [`ROUTE`].
fn arrival() -> usize {
    steps().len() - 1
}

/// One route, every assertion — the walk is 237 turns of a commercial story and is
/// paid for once.
#[test]
fn a_bolded_npc_name_below_the_heading_costs_neither_the_room_nor_the_lock() {
    let Some((s, turns)) = play() else { return };
    let i = arrival();

    // ── Non-vacuity: HIGHLIGHT is on and the shape is the reported one ──────
    assert_eq!(turns[i].0, "sw");
    assert_eq!(turns[i - 1].0, "go to samuel johnson basement");
    let before = turns[i - 1].1.location.as_ref().expect("a room before the walk");
    assert_eq!(before.name, BASEMENT, "the route should be standing in the basement first");
    assert_ne!(
        before.number,
        app::roomid::synthetic_room_id(BASEMENT),
        "and key it by ADDRESS, so the lock was resolved before the walk"
    );
    let bolds = subheaders(&turns[i].1);
    assert_eq!(
        bolds.first().map(String::as_str),
        Some(BROWNS_LAB),
        "the story does print the room heading: {bolds:?}"
    );
    assert!(
        bolds.iter().any(|b| b == PROFESSOR_BROWN),
        "HIGHLIGHT must be on, so the NPC's name opens the next line in the same style: {bolds:?}"
    );
    assert!(
        turns[i].1.transcript.contains("Reification of Abstracts"),
        "and the walk really did arrive in Brown's Lab: {:?}",
        turns[i].1.transcript
    );

    // ── The engine's answer: the heading survives, and so does the lock ─────
    let here = turns[i].1.location.as_ref().expect("a room after walking southwest");
    assert_eq!(
        here.name, BROWNS_LAB,
        "the heading was printed and must be read; keeping the basement's name (or \
         minting `synthetic_room_id(\"{BASEMENT}\")` = #{}) is the reported defect",
        app::roomid::synthetic_room_id(BASEMENT)
    );
    assert!(s.locked_room_global().is_some(), "the lock must survive the walk");

    // ── The map's half: no "bunch of Samuel Johnson rooms" ─────────────────
    let mut mapper = Mapper::default();
    let mut death = DeathWatch::default();
    for (cmd, r) in &turns {
        apply_turn(&mut mapper, cmd, r, &mut death);
    }
    let names: Vec<String> = mapper.graph.rooms().map(|r| r.label().to_string()).collect();
    assert!(
        names.iter().any(|n| n == BROWNS_LAB),
        "Brown's Lab is a room with a name and belongs on the map: {names:?}"
    );
    let basements = names.iter().filter(|n| n.as_str() == BASEMENT).count();
    assert_eq!(basements, 1, "one Samuel Johnson Basement, not one per identity scheme: {names:?}");
}

/// Contiguous `Subheader` (glk.h `style_Subheader` = 4) runs of a turn's transcript that
/// BEGIN at a line start — the shape `StoryScan` considers a room heading.
fn subheaders(t: &TurnResult) -> Vec<String> {
    let chars: Vec<char> = t.transcript.chars().collect();
    // Contiguous bold spans first, keyed by where each begins; the line-start filter is
    // applied afterwards, so a mid-line bold run cannot be glued onto the span before it.
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
