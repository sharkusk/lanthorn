//! SQ-1283: Shogun's same-named rooms must not collapse onto one object.
//!
//! Shogun ships two rooms called `Bridge`, two called `Main Deck` and four
//! called `Ledge`. Its rooms are children of a `ROOMS` container rather than
//! top-level objects, and its global 0 holds a constant NPC rather than the
//! current room, so neither of `resolve_room_object`'s first two tie-breaks can
//! separate them: every one of the ten used to be handed to the lowest-numbered
//! twin. From the very first turn of the game the Erasmus's own bridge
//! (`BRIDGE-OF-ERASMUS`, object #57) was therefore reported as object #42 —
//! the stone bridge over Osaka castle's moat, thirteen scenes away.
//!
//! Silent until SQ-1268 taught `WorldModel` to read a V6 ZIL exit table. From
//! then on the story's own table said `Deck` `UP` leads to #57 while detection
//! said #42, and SQ-1269's declared-exit mismatch rule read that disagreement
//! as evidence of a RANDOM exit: climbing back up from below decks minted no
//! edge at all and raised a random-exit suspicion instead. That is what
//! `climbing_back_up_from_below_decks_mints_a_real_edge` pins.
//!
//! Room objects and exits verified against Infocom's own ZIL source
//! (<https://github.com/historicalsource/shogun>):
//!
//! ```text
//! erasmus.zil  <ROOM BRIDGE-OF-ERASMUS (DESC "Bridge")
//!                    (DOWN TO ON-DECK) (FORE TO ON-DECK)
//!                    (PORT SORRY "You would fall overboard.") …>
//!              <ROOM ON-DECK (DESC "Deck") (UP TO BRIDGE-OF-ERASMUS) …>
//! osaka.zil    <ROOM ON-BRIDGE (DESC "Bridge")
//!                    (NORTH TO GATEWAY) (SOUTH TO PORTCULLIS)>
//! ```
//!
//! Skips vacuously without the gitignored `stories/` fixture, per
//! `crates/app/tests/suites/fixture_paths.rs`.

use crate::fixture_paths::fixture_path;

use app::engine::{DeclaredExit, Engine};
use app::graphics::PictSource;
use app::session::{apply_turn, DeathWatch, GameSession, InputKind};
use mapper::direction::{parse_direction, Direction};
use mapper::mapper::Mapper;

const SHOGUN: &str = "shogun-r322-s890706.z6";

/// The Erasmus's bridge — `BRIDGE-OF-ERASMUS`, `erasmus.zil`.
const BRIDGE_OF_ERASMUS: mapper::graph::RoomId = 57;
/// Osaka castle's bridge over the moat — `ON-BRIDGE`, `osaka.zil`. Same `DESC`.
const ON_BRIDGE: mapper::graph::RoomId = 42;
/// `ON-DECK`, the Erasmus's main deck.
const ON_DECK: mapper::graph::RoomId = 10;

/// Boot Shogun the way `v6_shogun_gameplay` does — the picture source and the
/// archive's own standard window, so the game lays its windows out the way the
/// player sees them and the status band this suite reads is the real one.
fn boot() -> Option<GameSession> {
    let path = fixture_path(SHOGUN);
    let bytes = std::fs::read(&path).ok()?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session = GameSession::new_with_trace(
        bytes, true, false, None, false, picture_dims, picts.std_window(), None, None,
    )
    .expect("Shogun (v6) boots without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    Some(session)
}

/// Answer the boot menu and any [MORE]/event gate until the game wants a line.
fn advance_to_line(session: &mut GameSession, budget: usize) -> bool {
    for _ in 0..budget {
        match session.pending_input() {
            InputKind::Line => return true,
            InputKind::Char => {
                let _ = session.submit_char(13);
            }
            InputKind::Event => {
                let _ = session.submit("");
            }
        }
    }
    matches!(session.pending_input(), InputKind::Line)
}

/// One turn driven exactly the way `turn::finish_command_turn` drives one: the
/// origin room's declared exit for the direction typed is read BEFORE
/// `apply_turn` decides what the move meant, because that is the input
/// SQ-1269's mismatch rule reads.
fn play(session: &mut GameSession, mapper: &mut Mapper, death: &mut DeathWatch, cmd: &str) {
    assert!(advance_to_line(session, 8), "Shogun should be at a line prompt before {cmd:?}");
    let mut result = session.submit(cmd);
    if let (Some(origin), Some(dir)) = (mapper.graph.current(), parse_direction(cmd)) {
        result.declared_exit = Some(session.declared_exit(origin, dir));
    }
    apply_turn(mapper, cmd, &result, death);
}

#[test]
fn shogun_opens_on_the_erasmus_bridge_not_the_castle_bridge() {
    let Some(mut session) = boot() else {
        eprintln!("SKIP: gitignored stories/{SHOGUN} missing");
        return;
    };
    assert!(advance_to_line(&mut session, 12), "Shogun reaches an in-game prompt after its menu");
    let _ = session.submit("look");

    let here = session.current_location().expect("Shogun names a room on its status band");
    assert_eq!(here.name, "Bridge", "non-vacuity guard: the game must actually open on `Bridge`");
    assert_eq!(
        here.number, BRIDGE_OF_ERASMUS,
        "the opening room is the Erasmus's own bridge, not Osaka castle's twin (#{ON_BRIDGE})"
    );

    // The two are distinguished by the exit tables the ZIL source declares, so
    // the pick above cannot be luck: only one of them leads DOWN to the deck.
    assert_eq!(
        session.declared_exit(BRIDGE_OF_ERASMUS, Direction::Down),
        DeclaredExit::Room(ON_DECK),
        "erasmus.zil: (DOWN TO ON-DECK)"
    );
    assert_eq!(
        session.declared_exit(ON_BRIDGE, Direction::N),
        DeclaredExit::Room(20),
        "osaka.zil: (NORTH TO GATEWAY) — the twin, still its own room"
    );
    assert_eq!(
        session.declared_exit(BRIDGE_OF_ERASMUS, Direction::N),
        DeclaredExit::Absent,
        "the Erasmus's bridge declares no compass exits at all"
    );
}

#[test]
fn shogun_climbing_back_up_from_below_decks_mints_a_real_edge() {
    let Some(mut session) = boot() else {
        eprintln!("SKIP: gitignored stories/{SHOGUN} missing");
        return;
    };
    assert!(advance_to_line(&mut session, 12), "Shogun reaches an in-game prompt after its menu");

    let mut mapper = Mapper::default();
    let mut death = DeathWatch::default();
    // The ship words are Shogun's own directions (`defs.zil`'s <DIRECTIONS …>
    // gives FORE/AFT/PORT/STARBOARD properties 51/50/49/48); `parse_direction`
    // aliases them onto N/S/W/E, and on the Erasmus that alias is reciprocal.
    for cmd in ["look", "down", "fore", "aft", "aft", "port", "starboard", "fore", "up"] {
        play(&mut session, &mut mapper, &mut death, cmd);
    }

    let here = mapper.graph.current().expect("the walk ends somewhere");
    assert_eq!(here, BRIDGE_OF_ERASMUS, "the walk ends back on the Erasmus's bridge");

    let edges: Vec<_> = mapper
        .graph
        .connections()
        .iter()
        .map(|c| (c.origin, c.dir, c.dest))
        .collect();
    assert!(
        edges.contains(&(ON_DECK, Direction::Up, BRIDGE_OF_ERASMUS)),
        "climbing back up from the deck must be a real passage, not a random-exit \
         suspicion raised by a mismatch against the wrong room's exit table; got {edges:?}"
    );
    assert!(
        !mapper.graph.is_random_exit(ON_DECK, Direction::Up),
        "…and it must not be marked random either"
    );
    // Non-vacuity: the below-decks half of the walk really happened.
    for (origin, dir, dest) in [(ON_DECK, Direction::N, 13), (ON_DECK, Direction::S, 56)] {
        assert!(
            edges.contains(&(origin, dir, dest)),
            "the ship-word walk below decks must have minted {origin} -{dir:?}-> {dest}; got {edges:?}"
        );
    }
}
