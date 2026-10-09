//! End-to-end smoke test: a Scott Adams (`ScottFree` `.dat`) walk drives the
//! automapper exactly as the app's boot/turn loop does — seed the starting
//! room via `apply_turn` with no direction, then feed each submitted command's
//! `TurnResult` through `apply_turn` (mirrors `crates/app/src/startup.rs`'s
//! room-seed block and `crates/app/src/turn.rs`'s `session.submit` ->
//! `apply_turn` per-turn flow).

use app::engine::Engine;
use app::scott_session::ScottSession;
use app::session::{apply_turn, TurnResult};
use mapper::direction::Direction;
use mapper::mapper::Mapper;

fn tiny_cave() -> Vec<u8> {
    include_bytes!("../../../scott/tests/tiny_cave.dat").to_vec()
}

#[test]
fn scott_walk_drives_the_automapper() {
    let mut session = ScottSession::new(tiny_cave(), None).expect("tiny_cave.dat loads");
    let mut mapper = Mapper::default();

    // Startup seed: observe the starting room with no direction, mirroring
    // startup.rs's "Observe the starting room so it appears on the map
    // immediately" block.
    let start_loc = session.current_location().expect("starting room known");
    let seed_result = TurnResult {
        transcript: String::new(),
        transcript_runs: Vec::new(),
        location: Some(start_loc),
        quit: session.has_quit(),
        erase_lower: false,
        info: None,
        sounds: Vec::new(),
        glulx_sound_ops: Vec::new(),
        diagnostics: Vec::new(),
        fault: None,
        location_method: None,
        pending_io: None,
        timed_out: false,
        pictures: Vec::new(),
        transcript_elems: Vec::new(),
        prose_retired: None,
        declared_exit: None,
        description: None,
        items: Vec::new(),
    };
    apply_turn(&mut mapper, "", &seed_result, &mut Default::default());

    assert_eq!(mapper.graph.rooms().count(), 1, "seed observes only the starting room");
    assert_eq!(mapper.graph.current(), Some(1));

    // Walk: tiny_cave's room 1 has a scripted "down" exit to room 2 (see
    // crates/scott/tests/golden.rs); "up" returns to room 1.
    for cmd in ["down", "up"] {
        let result = session.submit(cmd);
        apply_turn(&mut mapper, cmd, &result, &mut Default::default());
    }

    // The walk visited two distinct rooms.
    assert!(mapper.graph.rooms().count() >= 2, "walk should have discovered a second room");
    assert_eq!(mapper.graph.current(), Some(1), "up returned to the starting room");

    // A directional Down edge from room 1 to room 2 was recorded.
    let conns = mapper.graph.connections();
    assert!(
        conns.iter().any(|c| c.origin == 1 && c.dir == Direction::Down && c.dest == 2),
        "expected a Down edge 1 -> 2 in {conns:?}"
    );
}

// ── SQ-1744: a `*` room's map label is short, its description stays whole ────

#[test]
fn map_label_strips_the_lead_in_of_a_literal_room() {
    use app::scott_session::map_label;
    assert_eq!(map_label("I'm on the shore of a lake"), "shore of a lake");
    assert_eq!(map_label("I am in a dark cave."), "dark cave");
    assert_eq!(map_label("You are at the edge"), "edge");
    assert_eq!(map_label("You're by an old well"), "old well");
    assert_eq!(map_label("I'm in cellar"), "cellar");
    assert_eq!(map_label("Outside a large gothic looking building."), "Outside a large gothic looking building");
    // Not a lead-in: left alone.
    assert_eq!(map_label("I'm lost"), "I'm lost");
    assert_eq!(map_label("In the forest"), "In the forest");
}

/// Every `*` room of adv01..adv14 gets a non-empty, no-longer, lead-in-free label.
/// `stories/` is gitignored: skips vacuously per file when absent.
#[test]
fn every_literal_room_of_the_adventure_set_has_a_short_map_label() {
    use app::scott_session::map_label;
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories");
    let (mut seen, mut leads) = (0, 0);
    for n in 1..=14 {
        let path = dir.join(format!("adv{n:02}.dat"));
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("SKIP: {} absent", path.display());
            continue;
        };
        let db = scott::Database::parse(&bytes).unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
        for room in db.rooms.iter().filter(|r| r.literal && !r.desc.trim().is_empty()) {
            let label = map_label(&room.desc);
            seen += 1;
            assert!(!label.is_empty(), "{n}: empty label for {:?}", room.desc);
            assert!(label.len() <= room.desc.len(), "{n}: {label:?} grew from {:?}", room.desc);
            // Rooms that open with a real lead-in ("I'm in a ...") must lose it. A `*` room
            // that is some other sentence entirely ("I think I'm in real trouble ...") has
            // no lead-in to strip and is only trimmed.
            let d = room.desc.to_ascii_lowercase();
            let has_lead = ["i'm", "i am", "you are", "you're"].iter().any(|l| {
                d.strip_prefix(l).and_then(|r| r.strip_prefix(' ')).is_some_and(|r| {
                    ["on ", "in ", "at ", "by "].iter().any(|p| r.starts_with(p))
                })
            });
            if has_lead {
                leads += 1;
                assert!(label.len() < room.desc.len(), "{n}: {label:?} is not shorter than {:?}", room.desc);
                for lead in ["I'm", "I am", "You are", "You're"] {
                    assert!(!label.starts_with(lead), "{n}: {label:?} still leads with {lead:?}");
                }
            }
        }
    }
    eprintln!("checked {seen} literal rooms, {leads} with a lead-in");
}
