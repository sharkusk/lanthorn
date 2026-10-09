//! A story built with ZILF + ZAPF gets an automap (SQ-1718).
//!
//! The SQ-1579 gate (`zmachine_story_is_menu_driven_v6`) switched mapping off on
//! ANY `Grammar::load` error, and a ZILF build made `load` fail with
//! `BadVerbTable` — its serial is the build date, ZAPF stamps `"ZAPF"` at $3C
//! and lays the verb tables out in a different order — so rooms were detected
//! and then never mapped. The gate now fires only for `Absent`, and the
//! grammar reader finds the ZAPF layout.
//!
//! `unit_tests/zork1-mit.z3` is committed (a ZILF 0.11.1 build of Zork I from
//! Microsoft's MIT-licensed source), so the main case runs on CI. The Infocom
//! original is the same walk as a cross-check and skips without `stories/`.

use std::path::PathBuf;

use app::engine_helpers::zmachine_story_is_menu_driven_v6;
use app::session::{apply_turn, DeathWatch, GameSession};
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;

/// The opening of Zork I: West of House, North of House, Behind House, then in
/// through the window to the Kitchen and the Living Room — five rooms.
const WALK: [&str; 7] =
    ["look", "open mailbox", "north", "east", "open window", "enter house", "west"];

fn session_for(bytes: Vec<u8>) -> GameSession {
    GameSession::new_with_trace(
        bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)),
    )
    .expect("the story boots without a ZError")
}

/// Play `WALK` the way the host does: gate the mapper on the grammar, then feed
/// every turn through `apply_turn`. Returns the mapped room names.
fn walk_and_map(bytes: Vec<u8>) -> Vec<String> {
    let mut session = session_for(bytes);
    let mut mapper = Mapper::default();
    if zmachine_story_is_menu_driven_v6(&session) {
        mapper.disable_mapping(); // exactly what `host::boot` does
    }
    let mut death = DeathWatch::default();
    for cmd in WALK {
        let result = session.submit(cmd);
        assert!(session.machine.fault_trace.is_none(), "faulted on {cmd:?}");
        apply_turn(&mut mapper, cmd, &result, &mut death);
    }
    mapper.graph.rooms().map(|r| r.name.clone()).collect()
}

#[test]
fn zilf_zork1_has_a_grammar_and_is_mapped() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../unit_tests/zork1-mit.z3");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert!(
        !zmachine_story_is_menu_driven_v6(&session_for(bytes.clone())),
        "a ZILF build has a grammar table, so the map gate must stay open"
    );
    let rooms = walk_and_map(bytes);
    assert_eq!(rooms.len(), 5, "expected West of House .. Living Room, got {rooms:?}");
}

/// The narrowed gate: a grammar table the reader does NOT recognise is not an
/// absent one. Shave one entry off the specimen's preposition count (the word
/// ending where the verb pointer table begins; the walk never says the last
/// preposition) and `Grammar::load` refuses with `BadVerbTable`, yet the story
/// is the same parser game and must still be mapped.
#[test]
fn an_unreadable_grammar_does_not_switch_the_map_off() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../unit_tests/zork1-mit.z3");
    let mut bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes[0x43CC..0x43CE], [0x00, 0x12], "the specimen's preposition count moved");
    bytes[0x43CD] = 0x11;
    let mem = zvm::memory::Memory::new(bytes.clone()).unwrap();
    assert_eq!(
        zvm::grammar::Grammar::load(&mem).err(),
        Some(zvm::grammar::GrammarError::BadVerbTable),
        "non-vacuity: the reader must refuse this story"
    );
    assert!(!zmachine_story_is_menu_driven_v6(&session_for(bytes.clone())));
    assert_eq!(walk_and_map(bytes).len(), 5);
}

#[test]
fn infocom_zork1_maps_the_same_walk() {
    let path = fixture_path("zork1-r88-s840726.z3");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return;
    };
    let rooms = walk_and_map(bytes);
    assert_eq!(rooms.len(), 5, "got {rooms:?}");
}

/// The other half of the gate: a Dialog story has no grammar table of any shape
/// (`Absent`) but is not menu-driven V6, so the gate stays open and it is mapped
/// (SQ-1742). (Journey, the SQ-1579 case, is `sq1579_journey_no_map`'s.)
#[test]
fn a_dialog_story_has_no_grammar_but_is_still_mapped() {
    let path = fixture_path("ImpossibleStairs.z8");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: fetched story missing at {}", path.display());
        return;
    };
    assert!(!zmachine_story_is_menu_driven_v6(&session_for(bytes)));
}
