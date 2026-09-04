//! SQ-1294: the story moved the player and printed no heading — the map must still follow.
//!
//! # The report
//!
//! *"Counterfeit Monkey has a `go to` command. I used `go to Deep Street` from the
//! Counterfeit Monkey [the bar], and then I went south. It teleports me to Traffic
//! Circle, but Lanthorn doesn't realize I'm in a car in the Traffic Circle; it thinks
//! I'm in Deep Street. I `out` to exit the car; it thinks there's an exit `out` from
//! Deep Street to Traffic Circle."*
//!
//! The dump that came with it shows the damage in three lines:
//!
//! ```text
//! ROOM 38193 "Deep Street"  random=[SW-> (#40068 "Roundabout", #36534 "Deep Street")]
//! ROOM 36534 "Deep Street"
//! EDGE 36534 OUT 61262      (#61262 "Traffic Circle")
//! ```
//!
//! `#38193` and `#61262` are `app::roomid::glulx_room_id` of the rooms' own addresses;
//! `#36534` is `app::roomid::synthetic_room_id("Deep Street")` exactly. So a SECOND,
//! name-derived Deep Street was minted beside the real one and the drive out of it was
//! hung off the phantom.
//!
//! # What happens, measured on the fixture
//!
//! Driving the car out of Deep Street prints the whole arrival scene and **no room
//! heading**:
//!
//! ```text
//! > sw
//! We switch the ignition on.
//!
//! The whole Roundabout has ground to a halt, with protesters walking in the street ...
//!
//! I give the wheel a yank and run the car up onto the central traffic circle a little
//! way. Call it a parking job. ...
//! ```
//!
//! (The same arrival reached by `go to` DOES print "Traffic Circle (jammed into the
//! car)", which is why this only shows up when you drive by compass direction.)
//!
//! Two things then go wrong at once, and they are one thing:
//!
//! 1. `GlulxSession::finish_turn` derives `Movement` from the HEADING — no heading is
//!    `Movement::Unchanged` — while the story's `location` global really did change. So
//!    `RoomLock::verify` (`glulx_roomlock.rs`) sees a contradiction and calls
//!    `relearn`: **the resolved lock is thrown away by the very evidence that should
//!    have been used**. Everything until it re-locks is keyed by name hash again.
//! 2. With the lock gone, `GlulxSession::room_for` falls back to `heading_to_room` of
//!    the STALE name, so the room the player has just driven into is recorded as a
//!    second "Deep Street" — `EDGE 38193 SW 36534`, this suite's headline assertion and
//!    the `random=[SW-> ...]` entry in the reported dump.
//!
//! # Answering the player's `out`
//!
//! Getting out of the car is **not** a room change: Inform's `location` is the room, and
//! a vehicle is in the room with you. Measured here, `location` is identical before and
//! after `enter car`. So once the lock is the authority there is no `out` edge to draw —
//! the phantom exists only because the map had the wrong room to draw it from.
//!
//! # The fixture and the route
//!
//! `stories/CounterfeitMonkey-11.gblorb` — release 11 / serial 230220 / Inform 7 build
//! 6M62. Gitignored, so this skips vacuously without it.
//!
//! [`ROUTE`] is **162 inputs from a cold boot** and is not guessable: it is the game's
//! own `test me` script (`tools/command scripts/test_me.txt` in the i7/counterfeit-monkey
//! repository) as far as the bar, which is what it takes to have a working car and the
//! marina district unlocked, plus the player's own three commands. `random-seed 1234` is
//! the script's, and load-bearing. The last three inputs are the report:
//! `go to deep street`, `enter car`, `sw`.

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
go to deep street\n\
enter car\n\
sw";

/// The room the drive starts from.
const DEEP_STREET: &str = "Deep Street";
/// Where the car ends up.
const TRAFFIC_CIRCLE: &str = "Traffic Circle";

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

/// Where the drive is in [`ROUTE`].
fn drive() -> usize {
    steps().len() - 1
}

/// One route, every assertion — the drive is 162 turns of a commercial story and is
/// paid for once.
#[test]
fn a_silent_drive_moves_the_player_without_costing_the_lock_or_minting_a_room() {
    let Some((s, turns)) = play() else { return };
    let i = drive();

    // ── Non-vacuity: the route is the one the report describes ──────────────
    assert_eq!(turns[i].0, "sw");
    assert_eq!(turns[i - 2].0, "go to deep street");
    let arrival = turns[i - 2].1.location.as_ref().expect("a room after `go to deep street`");
    assert_eq!(arrival.name, DEEP_STREET, "the route should arrive in Deep Street");
    assert_ne!(
        arrival.number,
        app::roomid::synthetic_room_id(DEEP_STREET),
        "and should key it by the room's own address, which means the lock had resolved"
    );
    let t = &turns[i].1;
    assert!(
        t.transcript.contains("run the car up onto the central traffic circle"),
        "the `sw` must be the drive into the Traffic Circle: {:?}",
        t.transcript
    );
    assert!(
        subheaders(t).is_empty(),
        "the whole point is that the drive prints NO room heading; it printed {:?}",
        subheaders(t)
    );

    // ── The engine's answer: the story moved us, so the map moves ───────────
    let after = t.location.as_ref().expect("a room after the drive");
    assert_eq!(
        after.name, TRAFFIC_CIRCLE,
        "`sw` drove the car from Deep Street to the Traffic Circle; the map must follow \
         the story's own `location`, not the heading it did not print"
    );
    assert_ne!(
        after.number,
        app::roomid::synthetic_room_id(DEEP_STREET),
        "and must not mint a second, name-derived Deep Street for the room it left"
    );

    // ── The lock is the thing that KNEW, and must survive being right ───────
    assert!(
        s.locked_room_global().is_some(),
        "a turn the lock got RIGHT must not throw the lock away: the heading disagreed \
         because the story printed none, which is not evidence against the lock"
    );

    // ── The map's half, and the reported `random=[SW-> (..., #36534)]` ──────
    let mut mapper = Mapper::default();
    let mut death = DeathWatch::default();
    for (cmd, r) in &turns {
        apply_turn(&mut mapper, cmd, r, &mut death);
    }
    let deep: Vec<_> = mapper.graph.rooms().filter(|r| r.label() == DEEP_STREET).map(|r| r.id).collect();
    assert_eq!(
        deep.len(),
        1,
        "one Deep Street, not one per identity scheme: {deep:?} \
         (synthetic_room_id(\"Deep Street\") is #{})",
        app::roomid::synthetic_room_id(DEEP_STREET)
    );
    let self_edges: Vec<_> = mapper
        .graph
        .connections()
        .iter()
        .filter(|c| deep.contains(&c.origin) && deep.contains(&c.dest))
        .collect();
    assert!(self_edges.is_empty(), "Deep Street has no exit to itself: {self_edges:?}");
    // And getting into the car was never a room change, which is why no `out` edge is
    // owed on the way back out: Inform's `location` is the room, and a vehicle is in it.
    assert_eq!(turns[i - 1].0, "enter car");
    assert_eq!(
        turns[i - 1].1.location.as_ref().map(|l| l.number),
        Some(arrival.number),
        "`enter car` does not move the player between rooms"
    );
}

/// Contiguous `Subheader` (glk.h `style_Subheader` = 4) runs of a turn's transcript that
/// BEGIN at a line start — the shape `StoryScan` accepts as a room heading.
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
