//! SQ-1268: SQ-1260's ZIL exit-table derivation, widened from V3-only to
//! V4/V5/V6 (`crates/zvm/src/world.rs`'s "Declared exits: ZIL" module docs
//! carry the full citations and byte-length tables this suite proves).
//!
//! Real-game cases skip vacuously without `stories/` (gitignored), the
//! CI-safe pattern documented in `crates/app/tests/suites/fixture_paths.rs`.

use crate::fixture_paths::fixture_path;

use app::engine::{DeclaredExit, Engine};
use app::session::GameSession;
use mapper::direction::Direction;

fn story(name: &str) -> Option<Vec<u8>> {
    std::fs::read(fixture_path(name)).ok()
}

fn boot_narrow(bytes: Vec<u8>) -> GameSession {
    let mut s = GameSession::new_with_trace(
        bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)),
    )
    .expect("story boots without a ZError");
    s.set_strip_prompt(false);
    let _ = s.submit("look");
    s
}

// ── V4: Trinity — Palace Gate ────────────────────────────────────────────────
//
// `stories/trinity-r12-s860926.z4`'s starting room, Palace Gate, checked
// byte-for-byte against `places.zil`
// (<https://github.com/historicalsource/trinity>):
//
//   (NORTH TO BROAD-WALK) (NE TO WABE) (EAST TO FLOWER-WALK)
//   (SE PER IFENCE-BLOCKS) (SOUTH PER EXIT-GARDEN) (SW PER IFENCE-BLOCKS)
//   (WEST PER IFENCE-BLOCKS) (NW PER IFENCE-BLOCKS) (OUT PER EXIT-GARDEN)
//   (IN PER WHICH-WAY-IN)
//
// — three UEXITs (Room) and seven FEXITs (Code), never a NEXIT/DEXIT here;
// `trinity_v4_bluff_covers_uexit_nexit_fexit_and_dexit_in_one_room` below
// covers the other three shapes on a second room (Bluff).

#[test]
fn trinity_v4_palace_gate_matches_the_real_zil_source_and_a_real_move() {
    let Some(bytes) = story("trinity-r12-s860926.z4") else {
        eprintln!("SKIP: gitignored stories/trinity-r12-s860926.z4 missing");
        return;
    };
    let s = boot_narrow(bytes);
    let start = s.current_location().expect("Trinity names Palace Gate at boot");
    assert_eq!(start.name, "Palace Gate", "non-vacuity guard: must actually open on Palace Gate");

    let DeclaredExit::Room(north_dest) = s.declared_exit(start.number, Direction::N) else {
        panic!("north must be a plain declared UEXIT room (Broad Walk)");
    };
    assert_eq!(
        s.declared_exit(start.number, Direction::NE),
        DeclaredExit::Room(79),
        "NE is a UEXIT to The Wabe"
    );
    assert_eq!(
        s.declared_exit(start.number, Direction::E),
        DeclaredExit::Room(250),
        "east is a UEXIT to Flower Walk"
    );
    for dir in [Direction::S, Direction::SW, Direction::W, Direction::NW, Direction::Out, Direction::In] {
        assert_eq!(
            s.declared_exit(start.number, dir),
            DeclaredExit::Code,
            "{dir:?} is an FEXIT (PER IFENCE-BLOCKS / PER EXIT-GARDEN / PER WHICH-WAY-IN) — computed, not a fixed room"
        );
    }

    // A real move confirms the declared UEXIT: north actually lands in Broad Walk.
    let mut s = s;
    let after = s.submit("north");
    let arrived = after.location.expect("a room after walking north");
    assert_ne!(arrived.number, start.number, "the move actually crossed something");
    assert_eq!(arrived.number, north_dest, "the declared UEXIT's room number matches the real move's destination");
    assert_eq!(arrived.name, "Broad Walk", "and by name too");
}

/// A second Trinity room (Bluff, reached by walking away from Palace Gate)
/// whose own exit table exercises all FIVE ZIL shapes at once — checked
/// against `places.zil`'s `ON-BLUFF`:
///
///   (NORTH PER YOUD-FALL) (NE PER YOUD-FALL) (WEST PER YOUD-FALL)
///   (NW PER YOUD-FALL)                                              — FEXIT
///   (EAST TO IN-COTTAGE IF COTTAGE-DOOR IS OPEN)
///   (IN TO IN-COTTAGE IF COTTAGE-DOOR IS OPEN)                      — DEXIT
///   (SE TO AT-CRATER) (SW TO AT-CHASM)                              — UEXIT
///   (SOUTH SORRY "A sudden cliff blocks your path.")                — NEXIT
///
/// Bluff is not reachable from Palace Gate in one move in the shipped game
/// (the two rooms are in different regions of the map), so this reads the
/// room's compiled table directly by object number (#213, found by
/// `short_name` off the booted machine — see the module docs' citation) —
/// still a REAL room's REAL compiled bytes, not synthetic data.
#[test]
fn trinity_v4_bluff_covers_uexit_nexit_fexit_and_dexit_in_one_room() {
    let Some(bytes) = story("trinity-r12-s860926.z4") else {
        eprintln!("SKIP: gitignored stories/trinity-r12-s860926.z4 missing");
        return;
    };
    let s = boot_narrow(bytes);
    const BLUFF: u16 = 213;
    assert_eq!(
        zvm::objects::short_name(&s.machine.mem, BLUFF),
        "Bluff",
        "non-vacuity guard: object #213 must actually be Bluff"
    );

    assert_eq!(s.declared_exit(BLUFF.into(), Direction::SE), DeclaredExit::Room(441), "SE: a UEXIT to At Crater");
    assert_eq!(s.declared_exit(BLUFF.into(), Direction::SW), DeclaredExit::Room(306), "SW: a UEXIT to At Chasm");
    assert_eq!(s.declared_exit(BLUFF.into(), Direction::S), DeclaredExit::Message, "SOUTH: a NEXIT — a refusal, never a passage");
    for dir in [Direction::N, Direction::NE, Direction::W, Direction::NW] {
        assert_eq!(s.declared_exit(BLUFF.into(), dir), DeclaredExit::Code, "{dir:?}: an FEXIT (PER YOUD-FALL)");
    }
    for dir in [Direction::E, Direction::In] {
        assert_eq!(
            s.declared_exit(BLUFF.into(), dir),
            DeclaredExit::Room(327),
            "{dir:?}: a DEXIT (TO IN-COTTAGE IF COTTAGE-DOOR IS OPEN) — a STATIC destination room, matching the \
             module docs' \"UEXIT and DEXIT both resolve to Room\"; whether the door actually lets a live move \
             through this turn is a separate question `resolve_zil`'s own doc comment addresses"
        );
    }
}

// ── V5: Sherlock — 221-B Baker Street ────────────────────────────────────────
//
// A NARROW-width V4+ story (SQ-1268's `infer_zil_room_width` finds 1 byte,
// not 2): checked byte-for-byte against the compiled table (Sherlock's own
// ZIL source is not published on `historicalsource`, so this is corroborated
// by real-geography cross-reference — object number AND name — the same
// discipline `check_zil_exits.rs` established for SQ-1260's Mini-Zork case).
//
// North and south are both plain UEXITs (`Room(71)`/`Room(61)`, to York
// Place and Orchard Street respectively — real London street names either
// side of Baker Street), but the game's own opening puzzle refuses BOTH of
// them without a light source ("You start off into the fog, but think
// better of it when you realize you have no light to guide your way.") —
// there is no lamp in the starting inventory to fetch quickly, so this does
// NOT attempt a live walk in either of those two directions and says why
// rather than silently dropping the "real move" half of the proof. West/In
// (`Code`, an FEXIT — the game decides at run time) is the one direction
// that DOES have a real, immediate move: knocking gets you let in, and the
// declared `Code` here is proven correct in the OTHER direction Phase 1
// cares about — it genuinely leads somewhere (the entry hall, object #111,
// matching the byte-level dump exactly), which is exactly what `Code` means
// ("unresolvable statically, but maybe real") as opposed to `Message`.

#[test]
fn sherlock_v5_baker_street_matches_the_real_geography_and_its_code_exit_really_leads_somewhere() {
    let Some(bytes) = story("sherlock-r26-s880127.z5") else {
        eprintln!("SKIP: gitignored stories/sherlock-r26-s880127.z5 missing");
        return;
    };
    let s = boot_narrow(bytes);
    let start = s.current_location().expect("Sherlock names 221-B Baker Street at boot");
    assert_eq!(start.name, "221-B Baker Street", "non-vacuity guard");

    assert_eq!(s.declared_exit(start.number, Direction::N), DeclaredExit::Room(71), "north: a one-byte UEXIT to York Place");
    assert_eq!(s.declared_exit(start.number, Direction::S), DeclaredExit::Room(61), "south: a one-byte UEXIT to Orchard Street");
    assert_eq!(s.declared_exit(start.number, Direction::W), DeclaredExit::Code, "west: computed (routine-decided)");
    assert_eq!(s.declared_exit(start.number, Direction::In), DeclaredExit::Code, "in: the SAME computed exit as west");
    assert_eq!(zvm::objects::short_name(&s.machine.mem, 71), "York Place", "the declared north destination really is York Place, by name");
    assert_eq!(zvm::objects::short_name(&s.machine.mem, 61), "Orchard Street", "and the declared south destination really is Orchard Street");

    // The one real move available at the very start: knocking gets Mrs Hudson
    // to let you in, landing in the entry hall — proving the `Code`-declared
    // west/in exit is a genuine, working passage.
    let mut s = s;
    let after = s.submit("knock on door");
    let arrived = after.location.expect("a room after knocking");
    assert_eq!(arrived.number, 111, "the Code-declared west/in exit really does lead to the entry hall");
    assert_eq!(arrived.name, "entry hall");
}

// ── V6: Zork Zero — Banquet Hall ─────────────────────────────────────────────
//
// V6 has no `DIR` flag at all (ztools' `showdict.c` skips flag decoding for
// Version 6 outright) — the exit-property number is the dictionary entry's
// first data byte, directly. Checked against `prologue.zil`
// (<https://github.com/historicalsource/zorkzero>)'s `BANQUET-HALL`:
//
//   (WEST TO ENTRANCE-HALL) (SOUTH TO COURTYARD) (EAST TO KITCHEN)
//
// Booted the way the `v6_zork0_*` suites do (`PictSource::std_window()` for
// the screen size, per CLAUDE.md's v6 harness rule) — this is a BARE story
// file with no medium named, so the profile is the default (Generic
// interpreter, `Palette::Standard`), which the printed boot facts below
// confirm rather than assume.

use app::graphics::PictSource;
use std::path::PathBuf;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot Zork Zero and play far enough in to reach the Banquet Hall — the
/// same six-`look` opening `v6_zork0_icon_backdrop.rs`'s `zork0_in_play`
/// uses, reached via the SAME boot chain (`picts.std_window()` for the v6
/// screen size). Prints the profile/release/screen-size boot facts CLAUDE.md
/// asks every v6 harness to print.
fn zork0_reach_banquet_hall() -> Option<GameSession> {
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session = GameSession::new_with_trace(
        story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None,
    )
    .expect("Zork Zero (v6) should load and boot without a ZError");
    eprintln!(
        "BOOT: zork0-r393-s890714.z6 v{} release={} std_window={:?}",
        session.machine.mem.version(),
        session.machine.mem.read_word(2), // header release number (ZMSD §11.1.1)
        picts.std_window(),
    );
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    // Adaptive, not a fixed loop count: the Banquet Hall's own scripted
    // Prologue cutscene (`I-PROLOGUE`, `zork0_prologue.zil`) kills the player
    // outright at its FIFTH turn standing there unless they have hidden under
    // a table — so this stops on the FIRST turn `current_location` reports
    // Banquet Hall, spending as few turns there as possible before the
    // caller's own single confirming move.
    for _ in 0..10 {
        if session.current_location().map(|l| l.name) == Some("Banquet Hall".to_string()) {
            return Some(session);
        }
        match session.pending_input() {
            app::session::InputKind::Line => {
                let _ = session.submit("look");
            }
            app::session::InputKind::Char => {
                let _ = session.submit_char(b' ');
            }
            app::session::InputKind::Event => {
                let _ = session.submit("");
            }
        }
        let _ = session.take_transcript();
    }
    Some(session)
}

#[test]
fn zork0_v6_banquet_hall_matches_the_real_zil_source_and_a_real_move() {
    let _g = app::v6_palette(zvm::screen::Palette::Standard);
    let Some(mut session) = zork0_reach_banquet_hall() else { return };
    let start = session.current_location().expect("Zork Zero names a room after the opening");
    assert_eq!(start.name, "Banquet Hall", "non-vacuity guard: the opening must actually reach the Banquet Hall");

    assert_eq!(session.declared_exit(start.number, Direction::W), DeclaredExit::Room(56), "west: a one-byte UEXIT to Entrance Hall");
    assert_eq!(session.declared_exit(start.number, Direction::S), DeclaredExit::Room(8), "south: a one-byte UEXIT to Courtyard");
    assert_eq!(session.declared_exit(start.number, Direction::E), DeclaredExit::Room(59), "east: a one-byte UEXIT to Kitchen");
    assert_eq!(zvm::objects::short_name(&session.machine.mem, 59), "Kitchen");

    let after = session.submit("east");
    let arrived = after.location.expect("a room after walking east");
    assert_eq!(arrived.number, 59, "the declared UEXIT's room number matches the real move's destination");
    assert_eq!(arrived.name, "Kitchen");
}

/// A second V6 story, off a WIDER dictionary entry (`entry_length` 10, vs
/// Zork Zero's 9) — proving `infer_zil_exits_v6` reads the exit-property
/// number off the entry's first data byte regardless of how many trailing
/// bytes the rest of the entry carries. Read statically (no live boot;
/// Shogun's own castle scenes are gated behind hours of the game the way
/// Zork Zero's Prologue is behind a handful of turns, so this proves the
/// derivation against the compiled table directly rather than risk another
/// scripted event) — `ON-BRIDGE`'s own ZIL source
/// (<https://github.com/historicalsource/shogun>, `osaka.zil`):
///
///   (NORTH TO GATEWAY) (SOUTH TO AT-PORTCULLIS)
#[test]
fn shogun_v6_on_bridge_matches_the_real_zil_source() {
    let Some(bytes) = story("shogun-r322-s890706.z6") else {
        eprintln!("SKIP: gitignored stories/shogun-r322-s890706.z6 missing");
        return;
    };
    let mem = zvm::memory::Memory::new(bytes).unwrap();
    let model = zvm::world::WorldModel::discover(&mem);
    assert!(
        model.zil_exit_props.iter().filter(|p| p.is_some()).count() >= 6,
        "at least six of the twelve compass words must be direction-shaped"
    );

    const ON_BRIDGE: u16 = 42;
    assert_eq!(zvm::objects::short_name(&mem, ON_BRIDGE), "Bridge", "non-vacuity guard: object #42 must actually be Bridge");
    assert_eq!(
        model.declared_exit(&mem, ON_BRIDGE, zvm::world::Compass::N),
        zvm::world::DeclaredExit::Room(20),
        "north: a one-byte UEXIT to Gateway"
    );
    assert_eq!(
        model.declared_exit(&mem, ON_BRIDGE, zvm::world::Compass::S),
        zvm::world::DeclaredExit::Room(72),
        "south: a one-byte UEXIT to Portcullis"
    );
    assert_eq!(zvm::objects::short_name(&mem, 20), "Gateway");
    assert_eq!(zvm::objects::short_name(&mem, 72), "Portcullis");
}

/// Journey has NO compass parser at all — its dictionary (27 entries total)
/// carries none of the twelve compass words, so the derivation must find too
/// few `DIR`-shaped words to trust and answer `Unknown` for every direction,
/// never misfiring on some coincidental byte match.
#[test]
fn journey_v6_has_no_compass_parser_and_stays_unknown_everywhere() {
    let Some(bytes) = story("journey-r83-s890706.z6") else {
        eprintln!("SKIP: gitignored stories/journey-r83-s890706.z6 missing");
        return;
    };
    let mem = zvm::memory::Memory::new(bytes).unwrap();
    let model = zvm::world::WorldModel::discover(&mem);
    assert!(
        model.zil_exit_props.iter().all(Option::is_none),
        "Journey's dictionary has no compass words at all — no ZIL exit-property numbers to find"
    );
    for dir in zvm::world::Compass::ALL {
        assert_eq!(
            model.declared_exit(&mem, 1, dir),
            zvm::world::DeclaredExit::Unknown,
            "{dir:?}: no convention at all means Unknown, not a guessed answer"
        );
    }
}
