//! SQ-1289: the random forest room stood between the road and the valley.
//!
//! SQ-1287 got Adventure's opening the right way up — the valley south of the road
//! instead of north of it — and left one thing crooked. The valley came to rest TWO
//! rows below the road rather than one, with `In Forest` #55642 parked in the row in
//! between. Nothing connects the road to that forest going south; it simply took a
//! row on the way past, and the streambed and everything hanging off the valley moved
//! down with it.
//!
//! # Why a row it has no claim to
//!
//! Look at what #55642 says about itself in the reported map. It puts `In A Valley`
//! **west** of it (`FOREST_2 W VALLEY`) and it puts the same valley **east** of it
//! (`FOREST_2 E VALLEY`). Both cannot be true. Adventure's forest is random (SQ-1264):
//! `In Forest` scatters arrivals between two rooms of that name, so exits recorded
//! "from" #55642 were in fact walked from whichever forest the player was really
//! standing in, and the bundle that lands on one room id is a mixture.
//!
//! `build_axis_constraints` believed all of it anyway. `HILL S FOREST_2` put the
//! forest one row below the hill (= the road's row, the two being welded by their
//! reciprocal E/W pair), and `VALLEY Up FOREST_2` put the valley one row below the
//! forest. Neither constraint contradicts anything, so nothing was dropped and
//! `creates_cycle` never got a say — the forest just quietly claimed a row and
//! pushed the valley off the road's doorstep. Ordering the edges differently cannot
//! help: SQ-1287's sort only decides which of two CONTRADICTING edges gives way, and
//! these two agree with everybody.
//!
//! The fix is `layout::positionally_unreliable` (see its own cases in
//! `crates/mapper/src/layout/constraints.rs`): a room whose own outgoing compass edges
//! put one neighbour on two sides of itself, and which has no reciprocated pair to
//! anchor it, contributes no separation constraints at all — in or out — and takes a
//! free cell LAST, after every room with real geometry has claimed one. It still
//! appears on the map, beside its neighbours; it just stops shoving them around on the
//! strength of a position it does not have.

use mapper::direction::Direction as D;
use mapper::graph::{MapGraph, RoomId};

/// The seven rooms of the reported map (`/export-map`, `advent.blb`), as
/// `sq1287_advent_map_layout` transcribes them.
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

/// The reported twenty-three edges, in the order the player minted them.
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

/// The same graph as a player who went SOUTH first would have minted it. The layout is a
/// function of the graph, never of the route, so both must come out the same (SQ-1287).
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

/// The layout the shipped app runs on every turn that finds a room
/// (`config::BackgroundTidy::EveryRoom`, the default).
fn tidy(g: &mut MapGraph) {
    app::tidy::tidy_layer_silent(g, mapper::layer::MAIN_LAYER);
}

fn pos(g: &MapGraph, id: RoomId) -> (i32, i32) {
    g.room(id).and_then(|r| r.pos).unwrap_or_else(|| panic!("room #{id} is placed"))
}

/// Non-vacuity for every case below: the graph must actually carry the shape the defect
/// needs — exactly ONE room whose own compass claims contradict themselves, and it must be
/// the second forest. If the definition ever stopped naming it, the row assertions would
/// pass for the wrong reason (there would be nothing left to get out of the way).
fn assert_the_second_forest_is_the_only_muddled_room(g: &MapGraph) {
    let muddled: Vec<RoomId> = mapper::layout::positionally_unreliable(g).into_iter().collect();
    assert_eq!(
        muddled,
        vec![FOREST_2],
        "only #{FOREST_2} puts the valley on two sides of itself; the rest of the map is walked evidence"
    );
}

/// The whole quest, said as the player would say it: the valley is the road's next room
/// south, not its second. Relations only — never a cell — so the case survives any
/// repacking of the map as a whole.
fn assert_nothing_stands_between_the_road_and_the_valley(g: &MapGraph, what: &str) {
    let (rx, ry) = pos(g, ROAD);
    let (vx, vy) = pos(g, VALLEY);
    let (sx, sy) = pos(g, SLIT);
    assert_eq!(vx, rx, "{what}: the valley shares the road's column");
    assert_eq!(vy, ry + 1, "{what}: the valley is the road's next row south, not its second");
    assert_eq!(sx, vx, "{what}: the streambed shares the valley's column");
    assert_eq!(sy, vy + 1, "{what}: and the streambed is the valley's next row south");
    // …and nothing at all is parked in the road's own column on the way down. Stated
    // separately from the two rows above because THIS is the symptom the player reported:
    // a room standing in the gap, whichever room it turned out to be.
    for r in g.rooms() {
        let (x, y) = r.pos.expect("every room is placed");
        assert!(
            !(x == rx && y > ry && y < sy && r.id != VALLEY),
            "{what}: #{} {:?} stands in the road's column between the road and the streambed",
            r.id,
            r.label()
        );
    }
}

/// The rest of the opening must be exactly where SQ-1287 left it: hill due west of the
/// road, building due east, both on the road's row.
fn assert_the_road_still_runs_east_west(g: &MapGraph, what: &str) {
    let (rx, ry) = pos(g, ROAD);
    let (hx, hy) = pos(g, HILL);
    let (bx, by) = pos(g, BUILDING);
    assert!(hx < rx, "{what}: the hill lies WEST of the road (road x={rx}, hill x={hx})");
    assert_eq!(hy, ry, "{what}: the hill shares the road's row");
    assert!(bx > rx, "{what}: the building lies EAST of the road (road x={rx}, building x={bx})");
    assert_eq!(by, ry, "{what}: the building shares the road's row");
}

/// The reported map, laid out again. Before SQ-1289 the valley came to rest at the road's
/// row + 2, with `In Forest` #55642 holding the row between them.
#[test]
fn sq1289_the_random_forest_no_longer_holds_a_row_between_the_road_and_the_valley() {
    let mut g = build(FOREST_FIRST);
    assert_the_second_forest_is_the_only_muddled_room(&g);
    tidy(&mut g);
    assert_nothing_stands_between_the_road_and_the_valley(&g, "forest-first");
    assert_the_road_still_runs_east_west(&g, "forest-first");
}

/// The muddled room is not thrown away — it is still on the map, and still beside one of
/// the rooms it connects to. "Out of the way" must not become "off in a corner".
#[test]
fn sq1289_the_muddled_forest_still_lands_next_to_a_room_it_connects_to() {
    let mut g = build(FOREST_FIRST);
    tidy(&mut g);
    let (fx, fy) = pos(&g, FOREST_2);
    let neighbours: Vec<RoomId> = g
        .connections()
        .iter()
        .filter(|c| !c.is_self_loop())
        .filter_map(|c| match (c.origin, c.dest) {
            (FOREST_2, other) | (other, FOREST_2) => Some(other),
            _ => None,
        })
        .collect();
    assert!(!neighbours.is_empty(), "the forest has neighbours to be placed beside");
    let touching = neighbours.iter().any(|&n| {
        let (nx, ny) = pos(&g, n);
        (nx - fx).abs() <= 1 && (ny - fy).abs() <= 1
    });
    assert!(touching, "#{FOREST_2} at ({fx}, {fy}) must sit adjacent to one of {neighbours:?}");
}

/// A room with no geometry must never take a cell from one that has geometry, so it claims
/// last. Nothing else may have moved to make room for it: the five rooms the player can
/// actually place — and `In Forest` #42746, welded to the valley's row by its reciprocal
/// E/W pair — are all still exactly where the walked evidence puts them.
#[test]
fn sq1289_no_room_with_real_geometry_gave_up_its_cell() {
    let mut g = build(FOREST_FIRST);
    tidy(&mut g);
    let (rx, ry) = pos(&g, ROAD);
    let (f1x, f1y) = pos(&g, FOREST_1);
    let (vx, vy) = pos(&g, VALLEY);
    assert_eq!(f1y, vy, "the near forest shares the valley's row — their reciprocal E/W weld");
    assert!(f1x < vx, "and sits west of it, as both ends of that passage agree");
    assert_ne!((f1x, f1y), (rx, ry), "no two rooms share a cell");
    let mut cells: Vec<(i32, i32)> = g.rooms().map(|r| r.pos.expect("placed")).collect();
    cells.sort_unstable();
    let before = cells.len();
    cells.dedup();
    assert_eq!(cells.len(), before, "every room has a cell of its own");
}

/// Both mint orders, still one map — SQ-1287's invariant, re-checked now that a room is
/// placed by a different route than the rest.
#[test]
fn sq1289_the_layout_still_does_not_depend_on_which_way_the_player_went_first() {
    let mut forest_first = build(FOREST_FIRST);
    let mut valley_first = build(VALLEY_FIRST);
    assert_the_second_forest_is_the_only_muddled_room(&valley_first);
    tidy(&mut forest_first);
    tidy(&mut valley_first);
    assert_nothing_stands_between_the_road_and_the_valley(&valley_first, "valley-first");
    assert_the_road_still_runs_east_west(&valley_first, "valley-first");
    for &(id, name) in ROOMS {
        assert_eq!(
            pos(&forest_first, id),
            pos(&valley_first, id),
            "#{id} {name:?} must land in the same cell whichever way the player went first"
        );
    }
}
