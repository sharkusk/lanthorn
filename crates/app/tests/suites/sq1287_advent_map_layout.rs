//! SQ-1287: Adventure's opening laid out the wrong way up.
//!
//! The player walked the first few rooms of `advent.blb` and the automap put
//! `In A Valley` NORTH of `At End Of Road` — with `At Slit In Streambed`
//! south of the road, so the road sat between the valley and its own streambed
//! and the two reciprocal legs (`road S valley` / `valley N road`) both drew
//! `distorted`. Everything else on that map was right.
//!
//! # Why the valley went north
//!
//! Nothing about the valley. `background_tidy` defaults to `EveryRoom`, so the
//! shipped app re-runs the full layout (`tidy::tidy_layer_silent` →
//! `mapper::layout::relayout_auto`) on every turn that discovers a room, and
//! `layout::constraints::build_axis_constraints` is where the geometry is
//! decided: chain equalities first (a reciprocal E/W pair shares a ROW, a
//! reciprocal N/S pair shares a COLUMN), then one separation constraint per
//! compass edge, each skipped — and its edge flagged `distorted` — when it
//! would close a cycle on its axis.
//!
//! Adventure's forest is genuinely random (SQ-1264): `In Forest` redirects
//! arrivals to a second `In Forest` half the time, so the two forest rooms sit
//! on the map with self-loops and random pools and their positions mean very
//! little. But `In Forest` and `In A Valley` are joined by a reciprocal E/W
//! pair, which is a hard **equal-Y** equality — the valley is welded to the
//! forest's row. The player's very first move was `north` into that forest, and
//! that one-way edge (`road N forest`) claimed the Y axis first purely because
//! it was minted first. Welded to the forest, the valley went north with it, and
//! the reciprocal `road S valley` / `valley N road` pair — walked from BOTH
//! ends, two observations in agreement — arrived at a Y axis that already
//! contradicted them and was dropped.
//!
//! The fix is an ordering, not a special case: `build_axis_constraints` now
//! takes the directional edges **reciprocated first**, insertion order breaking
//! ties. A passage walked from both ends outranks a single one-way crossing, so
//! the valley pair claims the column and the lone road→forest edge is the one
//! that gives way. The layout stops depending on which way the player happened
//! to go first, which is what the two mint orders below assert.

use app::engine::Engine;
use app::glulx_session::GlulxSession;
use app::session::{apply_turn, DeathWatch};
use mapper::direction::Direction as D;
use mapper::graph::{MapGraph, RoomId};
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;

/// The seven rooms and twenty-three edges of the reported map, verbatim from the
/// player's `/export-map` dump (ids are `roomid::glulx_room_id` hashes, stable per
/// compile of `advent.blb`).
const ROOMS: &[(RoomId, &str)] = &[
    (34441, "Inside Building"),
    (42746, "In Forest"),
    (49722, "In A Valley"),
    (55642, "In Forest"),
    (61289, "At Hill In Road"),
    (61562, "At Slit In Streambed"),
    (63776, "At End Of Road"),
];

const ROAD: RoomId = 63776;
const VALLEY: RoomId = 49722;
const SLIT: RoomId = 61562;
const HILL: RoomId = 61289;
const BUILDING: RoomId = 34441;
const FOREST_1: RoomId = 42746;
const FOREST_2: RoomId = 55642;

/// The edges in the order the reported dump lists them — which is the order the player
/// minted them, north into the forest before south into the valley.
const FOREST_FIRST: &[(RoomId, D, RoomId)] = &[
    (ROAD, D::N, FOREST_1),
    (FOREST_1, D::E, VALLEY),
    (VALLEY, D::W, FOREST_1),
    (VALLEY, D::N, ROAD),
    (ROAD, D::S, VALLEY),
    (FOREST_1, D::S, FOREST_1),
    (FOREST_1, D::W, FOREST_1),
    (FOREST_1, D::N, FOREST_1),
    (VALLEY, D::S, SLIT),
    (SLIT, D::N, VALLEY),
    (FOREST_2, D::W, VALLEY),
    (VALLEY, D::Up, FOREST_2),
    (ROAD, D::E, BUILDING),
    (BUILDING, D::W, ROAD),
    (ROAD, D::W, HILL),
    (HILL, D::E, ROAD),
    (HILL, D::S, FOREST_2),
    (FOREST_2, D::S, FOREST_1),
    (FOREST_2, D::E, VALLEY),
    (FOREST_2, D::N, ROAD),
    (ROAD, D::Up, HILL),
    (ROAD, D::Down, VALLEY),
    (ROAD, D::In, BUILDING),
];

/// The SAME twenty-three edges, minted in the order a player who went SOUTH into the
/// valley before NORTH into the forest would produce. Same graph, different history.
const VALLEY_FIRST: &[(RoomId, D, RoomId)] = &[
    (ROAD, D::S, VALLEY),
    (VALLEY, D::N, ROAD),
    (VALLEY, D::S, SLIT),
    (SLIT, D::N, VALLEY),
    (VALLEY, D::W, FOREST_1),
    (FOREST_1, D::E, VALLEY),
    (FOREST_1, D::S, FOREST_1),
    (FOREST_1, D::W, FOREST_1),
    (FOREST_1, D::N, FOREST_1),
    (ROAD, D::N, FOREST_1),
    (FOREST_2, D::W, VALLEY),
    (VALLEY, D::Up, FOREST_2),
    (ROAD, D::E, BUILDING),
    (BUILDING, D::W, ROAD),
    (ROAD, D::W, HILL),
    (HILL, D::E, ROAD),
    (HILL, D::S, FOREST_2),
    (FOREST_2, D::S, FOREST_1),
    (FOREST_2, D::E, VALLEY),
    (FOREST_2, D::N, ROAD),
    (ROAD, D::Up, HILL),
    (ROAD, D::Down, VALLEY),
    (ROAD, D::In, BUILDING),
];

fn build(edges: &[(RoomId, D, RoomId)]) -> MapGraph {
    let mut g = MapGraph::new();
    for &(id, name) in ROOMS {
        g.upsert_room(id, name.to_string());
    }
    for &(o, d, t) in edges {
        if o == t {
            g.add_self_loop(o, d);
        } else {
            g.add_edge(o, d, t);
        }
    }
    g.set_current(ROAD);
    g
}

/// The layout the shipped app runs on every turn that discovers a room
/// (`config::BackgroundTidy::EveryRoom`, the default).
fn tidy(g: &mut MapGraph) {
    app::tidy::tidy_layer_silent(g, mapper::layer::MAIN_LAYER);
}

fn pos(g: &MapGraph, id: RoomId) -> (i32, i32) {
    g.room(id).and_then(|r| r.pos).unwrap_or_else(|| panic!("room #{id} is placed"))
}

/// The geography of Adventure's opening, asserted by room rather than by cell so the
/// case says what it means: the valley is below the road, the streambed below the
/// valley, the hill due west and the building due east.
fn assert_opening_reads_right(g: &MapGraph, what: &str) {
    let (rx, ry) = pos(g, ROAD);
    let (vx, vy) = pos(g, VALLEY);
    let (sx, sy) = pos(g, SLIT);
    let (hx, hy) = pos(g, HILL);
    let (bx, by) = pos(g, BUILDING);
    assert!(vy > ry, "{what}: the valley must lie SOUTH of the road (road y={ry}, valley y={vy})");
    assert_eq!(vx, rx, "{what}: the valley shares the road's column (road x={rx}, valley x={vx})");
    assert!(sy > vy, "{what}: the streambed must lie SOUTH of the valley (valley y={vy}, slit y={sy})");
    assert_eq!(sx, vx, "{what}: the streambed shares the valley's column");
    assert!(hx < rx, "{what}: the hill must lie WEST of the road (road x={rx}, hill x={hx})");
    assert_eq!(hy, ry, "{what}: the hill shares the road's row");
    assert!(bx > rx, "{what}: the building must lie EAST of the road (road x={rx}, building x={bx})");
    assert_eq!(by, ry, "{what}: the building shares the road's row");
}

fn distorted(g: &MapGraph) -> Vec<(RoomId, D, RoomId)> {
    g.connections().iter().filter(|c| c.distorted).map(|c| (c.origin, c.dir, c.dest)).collect()
}

/// The reported map, laid out again from the reported edge order. Before SQ-1287 this
/// put the valley at the road's north and flagged both legs of the road/valley pair
/// `distorted`; the whole of the reproduction is the mint ORDER, which is why the
/// sibling case below lays the identical graph out the other way round.
#[test]
fn sq1287_the_reported_advent_graph_puts_the_valley_south_of_the_road() {
    let mut g = build(FOREST_FIRST);
    // Non-vacuity: the shape the defect needs must actually be present — the valley
    // welded to a forest room by a reciprocal E/W pair, and the road/valley pair
    // reciprocal on N/S. Without both, this graph could not reproduce anything.
    let chains = mapper::layout::detect_chains(&g);
    assert_eq!(
        chains.ew.get(&VALLEY),
        chains.ew.get(&FOREST_1),
        "the valley and the forest share an E/W chain — the equal-Y weld the defect rode in on"
    );
    assert_eq!(chains.ns.get(&VALLEY), chains.ns.get(&ROAD), "road and valley share an N/S chain");

    tidy(&mut g);
    assert_opening_reads_right(&g, "forest-first");

    let d = distorted(&g);
    assert!(
        !d.contains(&(ROAD, D::S, VALLEY)) && !d.contains(&(VALLEY, D::N, ROAD)),
        "neither leg of the reciprocal road/valley pair may be distorted, got {d:?}"
    );
    assert!(
        !d.contains(&(VALLEY, D::S, SLIT)) && !d.contains(&(SLIT, D::N, VALLEY)),
        "neither leg of the reciprocal valley/streambed pair may be distorted, got {d:?}"
    );
    assert!(
        d.contains(&(ROAD, D::N, FOREST_1)),
        "the ONE-WAY edge into the random forest is the one that gives way, got {d:?}"
    );
}

/// The same seven rooms and twenty-three edges, minted in the other order, must lay out
/// identically. The layout is a function of the graph, not of the player's route through
/// it — before SQ-1287 these two orders produced two different maps.
#[test]
fn sq1287_the_layout_does_not_depend_on_which_way_the_player_went_first() {
    let mut forest_first = build(FOREST_FIRST);
    let mut valley_first = build(VALLEY_FIRST);
    tidy(&mut forest_first);
    tidy(&mut valley_first);
    assert_opening_reads_right(&valley_first, "valley-first");

    for &(id, name) in ROOMS {
        assert_eq!(
            pos(&forest_first, id),
            pos(&valley_first, id),
            "#{id} {name:?} must land in the same cell whichever way the player went first"
        );
    }
}

// ── The same thing on the real story ─────────────────────────────────────────

/// A live `advent.blb` session driven the way `turn::finish_command_turn` drives one, minus the
/// shadow probe: a suspicion no probe can decide is resolved on the spot
/// (`Mapper::resolve_suspicion_as_random`), which is what the real app does when none can run.
/// Adventure's forest is random (SQ-1264), so a walk through it is only a fixture if the rolls
/// are pinned. Every tracked turn below reseeds to this value, found by trial (see the seed probe
/// in the SQ-1287 investigation): it is the roll on which `north` out of `At End Of Road` lands
/// in the forest the room's own exit table DECLARES, so the walk mints `road N forest` — the
/// one-way edge into a random room that decided the reported map — instead of a random mark.
const LUCKY_SEED: u32 = 1;

struct Walk {
    mapper: Mapper,
    session: GlulxSession,
    death: DeathWatch,
}

impl Walk {
    fn advent() -> Option<Walk> {
        let bytes = match std::fs::read(fixture_path("advent.blb")) {
            Ok(b) => b,
            Err(_) => {
                eprintln!("SKIP: gitignored story missing at {}", fixture_path("advent.blb").display());
                return None;
            }
        };
        let blorb = blorb::Blorb::parse(bytes).ok()?;
        let (_kind, exec) = blorb.executable().ok()?;
        let store = app::scratch_dir("sq1287-advent-walk");
        let session = GlulxSession::new_in(
            store, exec.to_vec(), 80, 24, true, false, false, false, (1, 1), None, &[],
            [[(None, None); 11]; 2], false, None,
        )
        .expect("Adventure (Glulx) boots");
        Some(Walk { mapper: Mapper::default(), session, death: DeathWatch::default() })
    }

    /// An UNTRACKED submit, for the room-lock warmup only (SQ-0526, and see
    /// `sq1264_forest_randomization`'s `g_reach_hill`): tracking starts once the id has settled.
    fn raw(&mut self, cmd: &str) {
        let _ = Engine::submit(&mut self.session, cmd);
    }

    fn turn(&mut self, cmd: &str) {
        Engine::reseed_random(&mut self.session, LUCKY_SEED);
        let before = self.mapper.graph.current();
        let mut result = Engine::submit(&mut self.session, cmd);
        let dir = mapper::direction::parse_direction(cmd);
        if let (Some(o), Some(d)) = (before, dir) {
            result.declared_exit = Some(self.session.declared_exit(o, d));
        }
        apply_turn(&mut self.mapper, cmd, &result, &mut self.death);
        if let Some(susp) = self.mapper.take_random_exit_suspicion() {
            self.mapper.resolve_suspicion_as_random(susp);
        }
    }

    fn here(&self) -> String {
        self.session.current_location().map(|l| l.name).unwrap_or_default()
    }

    fn id_of(&self, name: &str) -> Option<RoomId> {
        self.mapper.graph.rooms().find(|r| r.label() == name).map(|r| r.id)
    }
}

/// Walk `At End Of Road` and its four neighbours on the real story, forest first — the order the
/// reported map was made in — and check the opening reads geographically after the same
/// background tidy the app runs on every new room.
///
/// The forest legs are the random ones (SQ-1264: `In Forest` redirects arrivals to a second
/// `In Forest` half the time), so every tracked turn reseeds to [`LUCKY_SEED`] and the walk is a
/// fixture rather than a coin toss. It mints eleven edges — the reported map's core shape: the
/// one-way `road N forest`, the reciprocal forest/valley E/W weld, and the reciprocal road/valley
/// and valley/streambed pairs — and before SQ-1287 that laid the valley out north of the road,
/// exactly as reported. Assertions are by room NAME and never on a cell, so the case says what
/// the player would say. Skips vacuously without the gitignored fixture; guards non-vacuity below.
#[test]
fn sq1287_a_scripted_advent_walk_lays_the_opening_out_geographically() {
    let Some(mut p) = Walk::advent() else { return };
    // Room-lock warmup, untracked: four room changes and then east/west until the id stops
    // moving, exactly as `sq1264_forest_randomization::g_reach_hill` does.
    for cmd in ["in", "take lamp", "down", "west"] {
        p.raw(cmd);
    }
    let mut prev = p.session.current_location().map(|l| l.number);
    for _ in 0..8 {
        p.raw("east");
        p.raw("west");
        let now = p.session.current_location().map(|l| l.number);
        if now == prev {
            break;
        }
        prev = now;
    }
    assert_eq!(p.here(), "At End Of Road", "the warmup ends where the walk starts");

    // Tracking starts here, at turn 0 of the scripted walk.
    p.turn("look");
    for cmd in [
        "north", // into the forest FIRST — the move that decided the reported map
        "east", "north", // back out: forest 1 goes east to the valley, the valley north to the road
        "south", "west", "east", // road → valley → forest → valley (the reciprocal E/W weld)
        "north", "south", // valley ↔ road, both ways: the reciprocal N/S pair
        "south", "north", // valley ↔ streambed, both ways
        "north", "west", "east", // road ↔ hill
        "east", "west", // road ↔ building
    ] {
        p.turn(cmd);
    }

    // Non-vacuity: the walk must actually have reached the five deterministic rooms, both legs
    // of the two reciprocal pairs, and at least one forest. A story that booted and refused
    // every move would otherwise pass.
    let road = p.id_of("At End Of Road").expect("the road is on the map");
    let valley = p.id_of("In A Valley").expect("the valley is on the map");
    let slit = p.id_of("At Slit In Streambed").expect("the streambed is on the map");
    let hill = p.id_of("At Hill In Road").expect("the hill is on the map");
    let building = p.id_of("Inside Building").expect("the building is on the map");
    assert!(p.mapper.graph.rooms().any(|r| r.label() == "In Forest"), "a forest room was reached");
    let has = |o: RoomId, d: D, t: RoomId| {
        p.mapper.graph.connections().iter().any(|c| c.origin == o && c.dir == d && c.dest == t)
    };
    assert!(has(road, D::S, valley) && has(valley, D::N, road), "the road/valley pair was walked both ways");
    assert!(has(valley, D::S, slit) && has(slit, D::N, valley), "the valley/streambed pair was walked both ways");

    let mut g = p.mapper.graph.clone();
    tidy(&mut g);

    let (rx, ry) = pos(&g, road);
    let (vx, vy) = pos(&g, valley);
    let (sx, sy) = pos(&g, slit);
    let (hx, hy) = pos(&g, hill);
    let (bx, by) = pos(&g, building);
    assert!(vy > ry, "the valley lies SOUTH of the road (road y={ry}, valley y={vy})");
    assert_eq!(vx, rx, "the valley shares the road's column");
    assert!(sy > vy, "the streambed lies SOUTH of the valley (valley y={vy}, slit y={sy})");
    assert_eq!(sx, vx, "the streambed shares the valley's column");
    assert!(hx < rx, "the hill lies WEST of the road (road x={rx}, hill x={hx})");
    assert_eq!(hy, ry, "the hill shares the road's row");
    assert!(bx > rx, "the building lies EAST of the road (road x={rx}, building x={bx})");
    assert_eq!(by, ry, "the building shares the road's row");
}
