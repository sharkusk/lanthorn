//! `lanthorn-mapgen`: a story's complete map, read out of the story file with
//! nothing played (SQ-1306).
//!
//! Every case drives [`app::mapgen::generate`] — the same library function the
//! `lanthorn-mapgen` binary calls — rather than shelling out, so a failure
//! points at a line of Rust instead of at a process exit code.
//!
//! **Three of the four sources run on CI.** `minizork.z3` (ZIL) and
//! `tiny_cave.dat` (Scott Adams) are tracked fixtures, and `czech.z5` is the
//! negative case: a real Z-machine story that declares no map at all. The two
//! Inform sources — `i7-world` and `i6-library` — have no tracked fixture, so
//! their cases resolve out of the gitignored `stories/` and skip vacuously
//! without it. A skip reads exactly like a pass, so each of those cases prints
//! what it skipped.

use std::path::{Path, PathBuf};

use app::mapgen::{self, EdgeKind, SourceKind};

// Declared once per GROUP BINARY, not per suite — these suites are modules of
// one crate, so a `#[path]` module here would be the same file loaded twice
// (clippy::duplicate_mod). See the header of `tests/mapper_ui.rs`.
use crate::fixture_paths::fixture_path;

/// A story under the gitignored `stories/`, or `None` when this checkout has no
/// copy — the CI-safe vacuous-skip pattern.
fn story(name: &str) -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(name);
    p.is_file().then_some(p)
}

/// A fixture tracked in the repository, so CI reaches it.
fn tracked(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// Every room name in a generated map.
fn names(map: &mapgen::GeneratedMap) -> Vec<String> {
    map.graph.rooms().map(|r| r.label().to_string()).collect()
}

/// True when `map` has an edge from a room named `from`, in direction `dir`, to
/// a room named `to`. Names rather than ids on purpose: an id is an
/// implementation detail of whichever engine read the story, and a test that
/// pins one breaks when the reader improves without the MAP being wrong.
fn has_edge(map: &mapgen::GeneratedMap, from: &str, dir: mapper::direction::Direction, to: &str) -> bool {
    map.facts.iter().any(|f| {
        f.dir == dir
            && map.graph.room(f.origin).map(|r| r.label() == from).unwrap_or(false)
            && map.graph.room(f.dest).map(|r| r.label() == to).unwrap_or(false)
    })
}

// ---------------------------------------------------------------------------
// CI-runnable: Scott Adams
// ---------------------------------------------------------------------------

/// A Scott Adams database lists its whole map explicitly, so the generated map
/// is exactly the database's own room table with nothing derived.
///
/// `tiny_cave.dat` is three rooms in a vertical chain — clearing, cave, grotto —
/// plus the format's room-0 "no room" sentinel, which is not a place and is not
/// in the map.
#[test]
fn scott_database_maps_completely() {
    let map = mapgen::generate(&tracked("../scott/tests/tiny_cave.dat"), true)
        .expect("tiny_cave.dat is a tracked fixture and must map");

    assert_eq!(map.source, SourceKind::Scott);
    assert_eq!(map.story.engine, "scott");
    assert_eq!(map.graph.rooms().count(), 3, "three rooms; index 0 is the sentinel, not a place");

    // A Scott Adams database has no release, serial or checksum field at all
    // (SQ-1306) — unlike the Z-machine and an Inform-compiled Glulx image,
    // which both self-identify. All three stay null rather than substituting
    // the trailer's adventure number, which is a title id, not a build identity.
    assert_eq!(map.story.release, None);
    assert_eq!(map.story.serial, None);
    assert_eq!(map.story.checksum, None);

    // Four edges: down the chain twice, and back up twice. The grotto's DOWN
    // and the clearing's UP are both 0 in the database, which means "no exit".
    assert_eq!(map.facts.len(), 4, "edges: {:?}", map.facts);

    let names = names(&map);
    let clearing = names.iter().find(|n| n.contains("clearing")).expect("the sunlit clearing");
    let cave = names.iter().find(|n| n.contains("cave")).expect("the damp cave");
    assert!(
        has_edge(&map, clearing, mapper::direction::Direction::Down, cave),
        "the clearing's path leads DOWN into the cave"
    );
    assert!(
        has_edge(&map, cave, mapper::direction::Direction::Up, clearing),
        "and back up again"
    );
}

// ---------------------------------------------------------------------------
// CI-runnable: ZIL
// ---------------------------------------------------------------------------

/// Mini-Zork I (r34/s871124) is Infocom's own ZIL, and is tracked — so the ZIL
/// reader has CI coverage that does not depend on `stories/`.
///
/// The white-house corner is the part of Zork every reader knows, and it is
/// four UEXITs: West of House leads north and northeast to North of House, and
/// south and southeast to South of House.
#[test]
fn zil_story_maps_from_its_exit_properties() {
    use mapper::direction::Direction;
    let map = mapgen::generate(&fixture_path("minizork-r34-s871124.z3"), true)
        .expect("minizork.z3 is a tracked fixture and must map");

    assert_eq!(map.source, SourceKind::Zil);
    assert_eq!(map.story.engine, "z-machine");
    assert_eq!(map.story.release, Some(34), "Mini-Zork I release 34");
    assert_eq!(map.story.serial.as_deref(), Some("871124"));

    // ZMSD §11.1: checksum is the word at $1C, reported as a lowercase
    // `0x`-prefixed hex string. Read straight out of the fixture's own bytes
    // rather than pinning a magic number, so this doesn't silently start
    // testing the wrong fixture if minizork.z3 is ever replaced.
    let bytes = std::fs::read(fixture_path("minizork-r34-s871124.z3")).unwrap();
    let want_checksum = format!("0x{:04x}", u16::from_be_bytes([bytes[0x1C], bytes[0x1D]]));
    assert_eq!(map.story.checksum.as_deref(), Some(want_checksum.as_str()));

    assert!(has_edge(&map, "West of House", Direction::N, "North of House"));
    assert!(has_edge(&map, "West of House", Direction::NE, "North of House"));
    assert!(has_edge(&map, "West of House", Direction::S, "South of House"));
    assert!(has_edge(&map, "West of House", Direction::SE, "South of House"));

    // A ZIL CEXIT is a real passage behind a condition, and it is drawn. West
    // of House leads southwest to the barrow only in the endgame; a reader that
    // dropped conditional exits would lose it silently.
    assert!(
        map.facts.iter().any(|f| f.kind == EdgeKind::Conditional),
        "Mini-Zork declares conditional exits and they must be on the map"
    );

    // Sanity on the whole map rather than on one corner: a real story has far
    // more rooms than the handful this case names, and every room a Z-machine
    // reader mints is an object number it can point at.
    assert!(map.graph.rooms().count() > 40, "rooms: {}", map.graph.rooms().count());
    for r in map.graph.rooms() {
        assert!(
            matches!(map.engine_refs.get(&r.id), Some(mapgen::EngineRef::ZObject(_))),
            "every Z-machine room carries its object number: {:?}",
            r.label()
        );
    }
}

/// A story with no map anywhere in it is refused, and says so — this is the
/// exit-2 path, and `czech.z5` (a Z-machine conformance test, not a game) is a
/// tracked example of it.
#[test]
fn a_story_that_declares_no_map_is_refused() {
    let err = mapgen::generate(&tracked("../zvm/tests/fixtures/czech.z5"), true)
        .expect_err("czech.z5 is a conformance test with no rooms and must be refused");
    assert!(
        matches!(err, mapgen::GenError::NoStaticSource(_)),
        "refusal must be NoStaticSource (the binary's exit 2), got {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("no static map source"),
        "the message has to say what happened: {msg}"
    );
}

// ---------------------------------------------------------------------------
// CI-runnable: the JSON map's shape
// ---------------------------------------------------------------------------

/// The JSON is a published format read by tools that have never seen lanthorn,
/// so its required keys are pinned here: a change to any of them has to be a
/// deliberate edit to this case, not a side effect.
#[test]
fn json_map_pins_its_format_and_required_keys() {
    let map = mapgen::generate(&tracked("../scott/tests/tiny_cave.dat"), true).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&mapgen::render_json(&map)).expect("the JSON must round-trip");

    assert_eq!(v["format"], "lanthorn-map");
    assert_eq!(v["version"], 1);
    for key in ["format", "version", "generator", "story", "directions", "rooms", "edges", "layers"] {
        assert!(!v[key].is_null(), "top-level key {key:?} is required");
    }
    for key in ["file", "engine", "source", "generated_at"] {
        assert!(!v["story"][key].is_null(), "story.{key} is required");
    }
    assert_eq!(v["story"]["source"], "scott");
    assert!(v["generator"]["name"] == "lanthorn-mapgen");

    // The direction vocabulary: the eight compass points carry a bearing and
    // the four portals do not, which is what tells a consumer which of them it
    // can lay out on a grid.
    let dirs = v["directions"].as_array().expect("directions is an array");
    assert_eq!(dirs.len(), 12);
    let north = dirs.iter().find(|d| d["word"] == "north").expect("north");
    assert_eq!(north["bearing"], 0);
    assert_eq!(north["short"], "n");
    let up = dirs.iter().find(|d| d["word"] == "up").expect("up");
    assert!(up["bearing"].is_null(), "up is not a compass bearing");

    // Every room the text dump lists is in the JSON, by the same id.
    let dump = app::map_dump::render_dump(&map.graph, &app::symbols::SymbolSet::default());
    let rooms = v["rooms"].as_array().expect("rooms is an array");
    assert_eq!(rooms.len(), map.graph.rooms().count());
    for r in rooms {
        let name = r["name"].as_str().unwrap();
        assert!(dump.contains(name), "the dump must list {name:?} too");
        for key in ["id", "raw_id", "name", "ordinal", "layer", "flags", "engine_ref"] {
            assert!(!r[key].is_null(), "room key {key:?} is required");
        }
        assert!(r["pos"]["x"].is_number(), "a laid-out map gives every room a position");
        assert_eq!(r["engine_ref"]["kind"], "scott-room");
    }

    for e in v["edges"].as_array().expect("edges is an array") {
        for key in ["from", "to", "dir", "kind", "reciprocal"] {
            assert!(!e[key].is_null(), "edge key {key:?} is required");
        }
        assert!(e["reciprocal"].is_boolean());
    }
}

/// With no layout there are no positions, and the JSON says so with `null`
/// rather than by omitting the key or inventing an origin.
#[test]
fn json_map_reports_no_position_when_layout_was_skipped() {
    let map = mapgen::generate(&tracked("../scott/tests/tiny_cave.dat"), false).unwrap();
    assert!(map.layout_time.is_none());
    let v: serde_json::Value = serde_json::from_str(&mapgen::render_json(&map)).unwrap();
    for r in v["rooms"].as_array().unwrap() {
        assert!(r["pos"].is_null(), "no layout means no position: {r}");
    }
}

// ---------------------------------------------------------------------------
// Real-game: Inform 7 (Counterfeit Monkey)
// ---------------------------------------------------------------------------

/// The Inform 7 reader against the story it was built for.
///
/// The comparison set is the map a PLAYER built by walking Counterfeit Monkey —
/// 84 distinct room names. A static map that misses any of them is missing a
/// room the game really has, which is the failure this case exists to catch;
/// having MORE is expected and fine, since the player never finished the game.
#[test]
fn counterfeit_monkey_static_map_covers_every_walked_room() {
    use mapper::direction::Direction;
    let Some(path) = story("CounterfeitMonkey-11.gblorb") else {
        eprintln!("SKIP: stories/CounterfeitMonkey-11.gblorb absent");
        return;
    };
    let map = mapgen::generate(&path, true).expect("Counterfeit Monkey must map");
    assert_eq!(map.source, SourceKind::I7World, "CM is an Inform 7 build");

    // Glulx-Inform-Tech.html §1 "Static Data": CM's own `Info` block reports
    // release 11, serial "230220" — verified directly against the bytes at
    // 0x24 in its embedded Glulx chunk. Every Glulx image also carries a
    // whole-image checksum (Glulx spec §1.4, offset 0x20), which is never
    // absent the way release/serial can be for a non-Inform build.
    assert_eq!(map.story.release, Some(11), "Counterfeit Monkey release 11");
    assert_eq!(map.story.serial.as_deref(), Some("230220"));
    assert!(
        matches!(map.story.checksum.as_deref(), Some(s) if s.starts_with("0x") && s.len() == 10),
        "checksum must be a non-null 0x-prefixed 32-bit hex string: {:?}",
        map.story.checksum
    );

    let generated = names(&map);
    for walked in WALKED_COUNTERFEIT_MONKEY_ROOMS {
        assert!(
            generated.iter().any(|n| n == walked),
            "the static map is missing {walked:?}, which a player walked into"
        );
    }

    assert!(
        has_edge(&map, "Sigil Street", Direction::E, "Ampersand Bend"),
        "Sigil Street leads east to Ampersand Bend"
    );

    // I7 resolves a two-sided door's far side statically, so CM's doors are
    // ordinary passages carrying the door's name rather than dead ends.
    let doors: Vec<_> = map.facts.iter().filter(|f| f.kind == EdgeKind::Door).collect();
    assert!(!doors.is_empty(), "Counterfeit Monkey has doors");
    assert!(
        doors.iter().all(|f| f.via.is_some()),
        "every door edge names the door it goes through"
    );
}

/// Layout on a real, large map: it finishes, and it satisfies the invariant
/// `mapper::layout::relayout_auto` guarantees — no two rooms in one layer share
/// a grid cell (`rooms_never_overlap_random_walk` in `mapper::layout` is the
/// same assertion on synthetic graphs).
#[test]
fn counterfeit_monkey_layout_places_every_room_in_its_own_cell() {
    use std::collections::BTreeSet;
    let Some(path) = story("CounterfeitMonkey-11.gblorb") else {
        eprintln!("SKIP: stories/CounterfeitMonkey-11.gblorb absent");
        return;
    };
    let map = mapgen::generate(&path, true).expect("Counterfeit Monkey must map");
    assert!(map.layout_time.is_some(), "layout was asked for and must have run");
    assert!(map.graph.rooms().count() > 90, "a hundred-room graph is the point of this case");

    for layer in map.graph.layers().keys() {
        let cells: Vec<(i32, i32)> = map
            .graph
            .rooms_in_layer(*layer)
            .into_iter()
            .filter_map(|id| map.graph.room(id).and_then(|r| r.pos))
            .collect();
        let unique: BTreeSet<(i32, i32)> = cells.iter().copied().collect();
        assert_eq!(cells.len(), unique.len(), "two rooms share a cell on layer {layer}");
    }

    // And the dump renders the whole thing without falling over — the artefact
    // is the deliverable, not the graph.
    let dump = app::map_dump::render_dump(&map.graph, &app::symbols::SymbolSet::default());
    assert!(dump.contains("Sigil Street"), "the dump draws the map it was given");
}

// ---------------------------------------------------------------------------
// Real-game: ZIL (Zork I) and the Inform 6 library on Glulx (Adventure)
// ---------------------------------------------------------------------------

/// Zork I's own release, as opposed to Mini-Zork's: the same white-house
/// geography, from a much larger story, and a room count in a sane range.
#[test]
fn zork1_static_map_reads_its_zil_exits() {
    use mapper::direction::Direction;
    let Some(path) = story("zork1-invclues-r52-s871125.z5") else {
        eprintln!("SKIP: stories/zork1-invclues-r52-s871125.z5 absent");
        return;
    };
    let map = mapgen::generate(&path, true).expect("Zork I must map");
    assert_eq!(map.source, SourceKind::Zil);
    assert_eq!(map.story.release, Some(52));
    assert_eq!(map.story.serial.as_deref(), Some("871125"));

    assert!(has_edge(&map, "West of House", Direction::N, "North of House"));
    assert!(has_edge(&map, "West of House", Direction::S, "South of House"));

    // Zork I has on the order of a hundred rooms. The range is deliberately
    // loose — this is a guard against a reader that finds two rooms or two
    // thousand, not a pin on the exact count.
    let rooms = map.graph.rooms().count();
    assert!((80..=200).contains(&rooms), "room count out of range: {rooms}");

    // The endgame barrow is a CEXIT, and the whole reason `ExitDetail` exists:
    // `declared_exit` alone would report it as unresolvable code and the map
    // would simply not have it.
    assert!(
        has_edge(&map, "West of House", Direction::SW, "Stone Barrow"),
        "the conditional passage to the Stone Barrow is a real passage"
    );
}

/// The Inform 6 library on Glulx — a different reader from Inform 7's, reached
/// only when no `Map_Storage` table is found.
#[test]
fn adventure_static_map_reads_the_inform6_library() {
    use mapper::direction::Direction;
    let Some(path) = story("advent.blb") else {
        eprintln!("SKIP: stories/advent.blb absent");
        return;
    };
    let map = mapgen::generate(&path, true).expect("Adventure must map");
    assert_eq!(map.source, SourceKind::I6Library, "advent.blb is an Inform 6 build");
    assert_eq!(map.story.engine, "glulx");

    // The Inform 6 library build carries its own `Info` block too — reading
    // it does not depend on the I7 world model, which this build has none of.
    assert_eq!(map.story.release, Some(5), "advent.blb release 5");
    assert_eq!(map.story.serial.as_deref(), Some("961209"));
    assert!(map.story.checksum.is_some());

    let generated = names(&map);
    assert!(
        generated.iter().any(|n| n == "At End Of Road"),
        "the road outside the wellhouse is where Colossal Cave starts"
    );
    assert!(has_edge(&map, "At End Of Road", Direction::E, "Inside Building"));
    assert!(has_edge(&map, "At End Of Road", Direction::W, "At Hill In Road"));
    assert!(has_edge(&map, "At End Of Road", Direction::Down, "In A Valley"));
}

/// A story the Inform 7 reader refuses AND that declares no Inform 6 exits
/// either is refused outright — the binary's exit 2. Kerkerkruip is one of the
/// eight the I7 reader declines.
#[test]
fn kerkerkruip_has_no_static_map_source() {
    let Some(path) = story("Kerkerkruip.gblorb") else {
        eprintln!("SKIP: stories/Kerkerkruip.gblorb absent");
        return;
    };
    let err = mapgen::generate(&path, true).expect_err("Kerkerkruip declares no static map");
    assert!(
        matches!(err, mapgen::GenError::NoStaticSource(_)),
        "must be the exit-2 refusal, got {err:?}"
    );
    let msg = err.to_string();
    assert!(msg.contains("Map_Storage"), "the message names what was looked for: {msg}");
}

/// The 84 distinct room names a player reached walking Counterfeit Monkey,
/// taken from a real `.map.txt` dump of that session. Not the game's whole map
/// — the player never finished it — which is why the assertion above is
/// "every one of these is present" rather than an equality.
const WALKED_COUNTERFEIT_MONKEY_ROOMS: &[&str] = &[
    "Abandoned Park",
    "Abandoned Shore",
    "Ampersand Bend",
    "Antechamber",
    "Apartment Bathroom",
    "Aquarium Bookstore",
    "Arbot Maps & Antiques",
    "Babel Café",
    "Back Alley",
    "Bureau Basement Middle",
    "Bureau Basement Secret Section",
    "Bureau Basement South",
    "Bureau Hallway",
    "Bus Station",
    "Cathedral Gift Shop",
    "Church Forecourt",
    "Cinema Lobby",
    "Cold Storage",
    "Counterfeit Monkey",
    "Crew Cabin",
    "Crumbling Wall Face",
    "Customs House",
    "Deep Street",
    "Display Reloading Room",
    "Docks",
    "Dormitory Room",
    "Equipment Archive",
    "Fair",
    "Fish Market",
    "Fleur d'Or Drinks Club",
    "Fleur d'Or Lobby",
    "Foredeck",
    "Galley",
    "Generator Room",
    "Heritage Corner",
    "Hesychius Street",
    "Higgate's office",
    "High Street",
    "Hostel",
    "Language Studies Department Office",
    "Long Street North",
    "Long Street South",
    "Midway",
    "Monumental Staircase",
    "My Apartment",
    "Navigation Area",
    "New Church",
    "Old City Walls",
    "Open Sea",
    "Oracle Project",
    "Outdoor Café",
    "Palm Square",
    "Park Center",
    "Patriotic Chard-Garden",
    "Personal Apartment",
    "Precarious Perch",
    "Private Solarium",
    "Projection Booth",
    "Public Convenience",
    "Rectification Room",
    "Roget Close",
    "Rotunda",
    "Roundabout",
    "Samuel Johnson Basement",
    "Samuel Johnson Hall",
    "Screening Room",
    "Sensitive Equipment Testing Room",
    "Sigil Street",
    "Slango's Bunk",
    "Slango's Head",
    "Sunning Deck",
    "Surveillance Room",
    "Tall Street",
    "Tin Hut",
    "Traffic Circle",
    "Tunnel through Chalk",
    "University Oval",
    "Waterstone's Office",
    "Webster Court",
    "Winding Footpath",
    "Wonderland",
    "Workshop",
    "Your Bunk",
    "Your Head",
];
