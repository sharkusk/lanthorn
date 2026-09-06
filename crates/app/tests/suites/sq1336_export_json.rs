//! `/export-json` (SQ-1336): the app's own walked-map JSON, in the SAME
//! `lanthorn-map` schema `lanthorn-mapgen` writes for a static map
//! (`app::mapgen::render_json`) — one serialiser
//! (`app::mapgen::render_json_view`) feeds both.
//!
//! `app::export_json`'s own in-crate tests (`crates/app/src/export_json.rs`)
//! cover the schema on a synthetic graph; this suite is the real-game
//! counterpart, walking Zork I a few rooms and checking what the JSON says
//! about the rooms it actually visited. Skips vacuously without the
//! gitignored `stories/zork1-r88-s840726.z3` — see
//! `crates/app/tests/suites/fixture_paths.rs` for the CI-safe pattern.

use crate::fixture_paths::fixture_path;

use app::engine::Engine;
use app::export_json::{build_walked_story, render_walked_json};
use app::session::{apply_turn, DeathWatch, GameSession};
use mapper::mapper::Mapper;

fn boot(bytes: Vec<u8>) -> GameSession {
    let mut s = GameSession::new_with_trace(
        bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)),
    )
    .expect("story boots without a ZError");
    s.set_strip_prompt(false);
    s
}

/// Walk West of House -> north -> North of House -> east -> Behind House ->
/// north -> North of House, and check the JSON names all three rooms by their
/// real object number, draws North of House's east passage to Behind House
/// `declared` and `reciprocal` (Behind House's own north walk back declares
/// the reverse — verified against the real game with `zvm-cli` before writing
/// this: West/North/South of House's own N/S "exits" are all NEXIT refusal
/// jokes, "the windows are all boarded", never real moves — East/North around
/// the corner at Behind House is the one genuinely two-way pair this close to
/// the start), and leaves `via`/`note` null — a walked map never learns a
/// door's own name or a CEXIT's condition.
#[test]
fn zork1_walked_json_names_its_rooms_with_kind_and_reciprocal_right() {
    let path = fixture_path("zork1-r88-s840726.z3");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored stories/zork1-r88-s840726.z3 missing");
        return;
    };
    let mut s = boot(bytes.clone());
    let mut mapper = Mapper::default();
    let mut death = DeathWatch::default();

    let r = s.submit("look");
    apply_turn(&mut mapper, "look", &r, &mut death);
    let west = mapper.graph.current().expect("West of House seeded");

    let r = s.submit("north");
    apply_turn(&mut mapper, "north", &r, &mut death);
    let north_of_house = mapper.graph.current().expect("North of House");
    assert_ne!(north_of_house, west, "non-vacuity: the walk actually moved");

    let r = s.submit("east");
    apply_turn(&mut mapper, "east", &r, &mut death);
    let behind_house = mapper.graph.current().expect("Behind House");
    assert_ne!(behind_house, north_of_house, "non-vacuity: the walk actually moved");

    let r = s.submit("north"); // back to North of House, declaring the reverse
    apply_turn(&mut mapper, "north", &r, &mut death);
    assert_eq!(mapper.graph.current(), Some(north_of_house), "north returns to North of House");

    let walked = build_walked_story(&s, &bytes, std::path::Path::new("zork1-r88-s840726.z3"));
    let json = render_walked_json(&mapper.graph, &walked);
    let v: serde_json::Value = serde_json::from_str(&json).expect("the JSON must round-trip");

    assert_eq!(v["story"]["source"], "walked");
    assert_eq!(v["story"]["engine"], "z-machine");
    assert_eq!(v["story"]["file"], "zork1-r88-s840726.z3");
    assert_eq!(v["story"]["release"], 88);
    assert_eq!(v["generator"]["name"], "lanthorn");

    let rooms = v["rooms"].as_array().expect("rooms is an array");
    let north_json = rooms
        .iter()
        .find(|r| r["name"] == "North of House")
        .expect("North of House named in the JSON");
    let behind_json =
        rooms.iter().find(|r| r["name"] == "Behind House").expect("Behind House named in the JSON");
    assert_eq!(north_json["engine_ref"]["kind"], "z-object");
    assert_eq!(
        north_json["engine_ref"]["number"], north_of_house as u64,
        "a Z-machine RoomId IS the object number"
    );
    assert_eq!(behind_json["engine_ref"]["kind"], "z-object");
    assert_eq!(behind_json["engine_ref"]["number"], behind_house as u64);

    let edges = v["edges"].as_array().expect("edges is an array");
    let east_edge = edges
        .iter()
        .find(|e| e["from"] == north_json["id"] && e["dir"] == "east")
        .expect("North of House's east edge");
    assert_eq!(east_edge["to"], behind_json["id"]);
    assert_eq!(east_edge["kind"], "declared");
    assert_eq!(east_edge["reciprocal"], true, "Behind House's own north walk back declares the reverse");
    assert!(east_edge["via"].is_null(), "a walked map never learns a door's own name");
    assert!(east_edge["note"].is_null());
}
