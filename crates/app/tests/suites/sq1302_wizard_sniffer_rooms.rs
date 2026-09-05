//! SQ-1302: a Glulx story that names its rooms only on the STATUS LINE.
//!
//! # The report
//!
//! Against 0.4.3, on *The Wizard Sniffer*: *"doesn't seem to detect any rooms at
//! all"*. Literally none — not a wrong room, not a late one. The map stayed empty
//! for the whole game.
//!
//! # Why
//!
//! Glulx has no object tree, so `GlulxSession` recovers the room from the
//! `Subheader` room HEADING an Inform story prints, which `glk_backend`'s
//! `StoryScan` captures. **This story prints no heading, in any style, ever.** Its
//! presentation is a custom two-row status grid — `" Atop a Mountain"` over
//! `" Exit: north"` — and a buffer that carries the description alone:
//!
//! ```text
//! The Wizard Sniffer                                       <- style_Header
//! An Interactive Fiction by Buster Hudson                  <- style_Normal
//! Release 1 / Serial number 171007 / Inform 7 build 6L38 (I6/v6.33 lib 6/12N)
//!
//! You stand before the raised drawbridge of an evil fortress. …
//! ```
//!
//! So `take_room_heading` had nothing to capture, and every fallback that exists
//! is downstream of one. `glulx_roomlock` scores RAM words against observed
//! heading CHANGES, so with no heading every turn reads `Unchanged` and the lock
//! can never reach its three changes — it cannot learn its way out. And SQ-1293's
//! `silent_look` asks the story by typing `look`, which here prints the
//! description and, again, no heading: the one answer that means "stop asking".
//!
//! The name was on the screen the whole time; nothing on the Glulx path had ever
//! looked at the grid. `AppGlk::status_room_name` now reads it — the Glk twin of
//! `zvm::location::status_line_room_name`, which has read the Z-machine's upper
//! window this way for as long as there has been a Z-machine path — and
//! `GlulxSession::name_this_room` gates it so it can only fire for a story that
//! has printed no heading at all.
//!
//! # The fixture and the route
//!
//! `stories/The_Wizard_Sniffer.gblorb` — **release 1 / serial 171007 / Inform 7
//! build 6L38 (I6/v6.33 lib 6/12N)**, Buster Hudson, IFComp 2017. Gitignored, so
//! this skips vacuously without it.
//!
//! Two keypresses clear the prologue card and the banner and land on the first
//! command prompt, standing Atop a Mountain. Then the opening puzzle, from David
//! Welbourn's walkthrough: `sniff rope` drops the drawbridge, and `n`, `n` walks
//! into the Southern Bailey and the Centre Bailey. Five events in all, and the
//! room the report says is missing is already on the map after the second key —
//! before the player types anything.

use app::engine::{Engine, KeyInput};
use app::glulx_session::GlulxSession;
use app::session::{apply_turn, DeathWatch, InputKind, TurnResult};
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;

const STORY: &str = "The_Wizard_Sniffer.gblorb";

/// The three rooms this route visits, in order.
const MOUNTAIN: &str = "Atop a Mountain";
const SOUTHERN: &str = "Southern Bailey";
const CENTRE: &str = "Centre Bailey";

/// The prologue's two keypresses, then the opening puzzle and two moves north.
/// `None` is a keypress rather than a command.
const ROUTE: &[Option<&str>] = &[None, None, Some("sniff rope"), Some("n"), Some("n")];

fn boot() -> Option<GlulxSession> {
    let path = fixture_path(STORY);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let pict_blorb = blorb::Blorb::parse(bytes.clone()).ok();
    let app::hints::LoadedStory::Glulx(image) =
        app::hints::extract_story(bytes).expect("The_Wizard_Sniffer.gblorb is a readable container")
    else {
        panic!("{STORY} is a Glulx story");
    };
    let mut s = GlulxSession::new(image, 80, 30, true, false, false, (8, 16), pict_blorb, &[])
        .expect("The Wizard Sniffer boots");
    let _ = s.take_transcript();
    Some(s)
}

/// One route, every assertion: play [`ROUTE`] and check the map followed the
/// story's own status line into all three rooms.
#[test]
fn the_status_line_names_every_room_this_story_never_prints_a_heading_for() {
    let Some(mut s) = boot() else { return };

    let mut turns: Vec<TurnResult> = Vec::new();
    for (i, step) in ROUTE.iter().enumerate() {
        let r = match step {
            Some(cmd) => {
                assert_eq!(s.pending_input(), InputKind::Line, "route step {i} ({cmd:?}) wants a line");
                s.submit(cmd)
            }
            None => {
                assert_eq!(s.pending_input(), InputKind::Char, "route step {i} wants a keypress");
                s.submit_key(KeyInput::Enter).expect("Glulx takes keys")
            }
        };
        turns.push(r);
    }

    // ── Non-vacuity: the route really did reach the game, and this really is the
    //    fixture the diagnosis was made on ─────────────────────────────────────
    let banner = &turns[1].transcript;
    assert!(
        banner.contains("Release 1 / Serial number 171007 / Inform 7 build 6L38"),
        "expected release 1 / serial 171007 / build 6L38; the fixture has changed: {banner:?}"
    );
    assert!(
        turns[2].transcript.contains("Down comes the drawbridge"),
        "the route only reaches a second room if `sniff rope` drops the drawbridge: {:?}",
        turns[2].transcript
    );

    // ── …and the shape the whole quest is about: not one turn of this route puts
    //    a room NAME in the buffer. If a future release starts printing headings,
    //    this case stops testing the status-line path and must be told so.
    for (i, t) in turns.iter().enumerate() {
        for room in [MOUNTAIN, SOUTHERN, CENTRE] {
            assert!(
                !t.transcript.contains(room),
                "turn {i} printed the room name {room:?} in the buffer; this story is supposed \
                 to name its rooms only on the status line: {:?}",
                t.transcript
            );
        }
    }

    // ── The report: the opening room is known before the player types anything ─
    assert_eq!(
        turns[1].location.as_ref().map(|l| l.name.as_str()),
        Some(MOUNTAIN),
        "the turn that hands the player the command prompt is standing Atop a Mountain"
    );

    // ── …and the first two moves land where the story says they do ────────────
    assert_eq!(turns[3].location.as_ref().map(|l| l.name.as_str()), Some(SOUTHERN));
    assert_eq!(turns[4].location.as_ref().map(|l| l.name.as_str()), Some(CENTRE));
    assert_eq!(s.current_location().map(|l| l.name), Some(CENTRE.to_string()));

    // ── The map's half: three rooms, and the edges the player walked ──────────
    let mut mapper = Mapper::default();
    let mut death = DeathWatch::default();
    for (step, r) in ROUTE.iter().zip(&turns) {
        apply_turn(&mut mapper, step.unwrap_or("<key>"), r, &mut death);
    }
    let names: Vec<String> = mapper.graph.rooms().map(|r| r.label().to_string()).collect();
    for room in [MOUNTAIN, SOUTHERN, CENTRE] {
        assert_eq!(names.iter().filter(|n| n.as_str() == room).count(), 1, "one {room}: {names:?}");
    }
    let id_of = |room: &str| {
        mapper.graph.rooms().find(|r| r.label() == room).map(|r| r.id).unwrap_or_else(|| panic!("{room} is mapped"))
    };
    for (from, to) in [(MOUNTAIN, SOUTHERN), (SOUTHERN, CENTRE)] {
        let (a, b) = (id_of(from), id_of(to));
        assert!(
            mapper
                .graph
                .connections()
                .iter()
                .any(|c| c.origin == a && c.dest == b && c.dir == mapper::direction::Direction::N),
            "the walk {from} -> N -> {to} must be on the map: {:?}",
            mapper.graph.connections()
        );
    }
}
