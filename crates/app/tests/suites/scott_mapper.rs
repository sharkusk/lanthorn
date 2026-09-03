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
