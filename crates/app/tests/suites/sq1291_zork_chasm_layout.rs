//! SQ-1291: Zork I's Chasm pinned SOUTH of the East-West Passage.
//!
//! The player's map (`stories/zork1-invclues-r52-s871125.z5`, host save `default.lanthorn`)
//! put `Chasm` #112 at (0, 1) with `East-West Passage` #136 at (-1, 0) — the chasm
//! south-EAST of the passage — and drew the chasm's own `SW` exit back to the passage
//! `distorted`. The game says otherwise: the passage describes "a stairway leading down
//! at the NORTH end of the room", and the return leg the player walked is `southwest`.
//!
//! # Why the chasm went south
//!
//! `direction::layout_offset` gives `Up` the offset of north and `Down` the offset of
//! south, so `build_axis_constraints` could not tell a stairwell from a compass bearing:
//! the reciprocated `136 Down 112` / `112 Up 136` pair looked like a reciprocated N/S
//! pair, and under SQ-1287's ordering — reciprocated evidence first — it claimed the Y
//! axis ahead of the one-way `112 SW 136`. The chasm was pinned a row BELOW the passage,
//! and the SW edge, arriving at a Y axis that already contradicted it, closed a cycle and
//! was dropped (→ `distorted`).
//!
//! # The rule
//!
//! Up and Down carry LESS placement weight than any compass edge, one-way ones included.
//! North-for-up is a drawing convention; a compass word is the game's own statement of
//! where the room IS. So `build_axis_constraints` now takes the directional edges in three
//! tiers — reciprocated compass pairs, then one-way compass edges, then Up/Down — with
//! insertion order breaking ties inside a tier. The chasm's `SW` claims both axes, the
//! stairwell yields, and the chasm lands north-east of the passage where the player found
//! it. (A dropped Up/Down constraint costs nothing on screen: `mark_distorted` gates on
//! `grid_offset`, which is `None` for Up/Down, so a stairwell is never drawn distorted.)

use mapper::direction::Direction as D;
use mapper::graph::{MapGraph, RoomId};
use mapper::layer::LayerId;

const PASSAGE: RoomId = 136; // East-West Passage
const CHASM: RoomId = 112;
const TROLL: RoomId = 133; // The Troll Room
const ROUND: RoomId = 16; // Round Room
const CELLAR_LAYER: LayerId = 1;

/// The twenty-five rooms of the reported map, in the order the player found them
/// (`seq`), with the layer each was filed under.
const ROOMS: &[(RoomId, &str, LayerId)] = &[
    (68, "West of House", 0),
    (143, "North of House", 0),
    (89, "Behind House", 0),
    (134, "Clearing", 0),
    (33, "Forest", 0),
    (247, "Forest Path", 0),
    (167, "Clearing", 0),
    (91, "Forest", 0),
    (230, "Forest", 0),
    (217, "South of House", 0),
    (23, "Canyon View", 0),
    (22, "Rocky Ledge", 0),
    (78, "Canyon Bottom", 0),
    (131, "End of Rainbow", 0),
    (175, "Forest", 0),
    (28, "Kitchen", 0),
    (79, "Living Room", 0),
    (195, "Attic", 0),
    (5, "Up a Tree", 0),
    (34, "Cellar", 1),
    (133, "The Troll Room", 1),
    (109, "Maze", 2),
    (136, "East-West Passage", 1),
    (112, "Chasm", 1),
    (16, "Round Room", 1),
];

/// The sixty-two connections verbatim from the player's `map.json`, in the order it lists
/// them — which is the order they were minted: down the stairwell BEFORE the chasm's own
/// `southwest` was walked.
const STAIRS_FIRST: &[(RoomId, D, RoomId)] = &[
    (68, D::N, 143),
    (143, D::W, 68),
    (143, D::E, 89),
    (89, D::N, 143),
    (89, D::E, 134),
    (134, D::W, 89),
    (134, D::N, 33),
    (33, D::S, 134),
    (33, D::W, 247),
    (247, D::E, 33),
    (247, D::N, 167),
    (167, D::S, 247),
    (167, D::W, 91),
    (91, D::E, 247),
    (91, D::N, 167),
    (91, D::S, 230),
    (230, D::N, 134),
    (230, D::W, 91),
    (247, D::S, 143),
    (143, D::N, 247),
    (68, D::W, 91),
    (134, D::S, 230),
    (89, D::S, 217),
    (217, D::W, 68),
    (217, D::E, 89),
    (68, D::S, 217),
    (167, D::E, 33),
    (134, D::E, 23),
    (23, D::W, 230),
    (23, D::NW, 134),
    (230, D::NW, 217),
    (23, D::Down, 22),
    (22, D::Up, 23),
    (22, D::Down, 78),
    (78, D::Up, 22),
    (78, D::N, 131),
    (131, D::SW, 78),
    (23, D::E, 22),
    (33, D::E, 175),
    (175, D::W, 33),
    (175, D::N, 33),
    (247, D::W, 91),
    (89, D::W, 28),
    (28, D::E, 89),
    (28, D::W, 79),
    (79, D::E, 28),
    (28, D::Up, 195),
    (195, D::Down, 28),
    (247, D::Up, 5),
    (5, D::Down, 247),
    (79, D::Down, 34),
    (34, D::N, 133),
    (133, D::S, 34),
    (133, D::W, 109),
    (109, D::E, 133),
    (133, D::E, 136),
    (136, D::W, 133),
    (136, D::Down, 112),
    (112, D::Up, 136),
    (112, D::SW, 136),
    (136, D::E, 16),
    (16, D::W, 136),
];

/// The SAME sixty-two edges with the chasm's three legs reordered: the `southwest` return
/// minted BEFORE the stairwell pair. Same graph, different history — and the map must not
/// be able to tell, which is the second half of SQ-1287's guarantee.
fn chasm_first() -> Vec<(RoomId, D, RoomId)> {
    let mut v: Vec<(RoomId, D, RoomId)> = STAIRS_FIRST.to_vec();
    let three = [(PASSAGE, D::Down, CHASM), (CHASM, D::Up, PASSAGE), (CHASM, D::SW, PASSAGE)];
    let at = v.iter().position(|e| *e == three[0]).expect("the stairwell leg is in the dump");
    v.retain(|e| !three.contains(e));
    for (i, e) in [three[2], three[0], three[1]].into_iter().enumerate() {
        v.insert(at + i, e);
    }
    v
}

fn build(edges: &[(RoomId, D, RoomId)]) -> MapGraph {
    let mut g = MapGraph::new();
    g.new_layer(Some(0), "Cellar".into());
    g.new_layer(Some(1), "Maze".into());
    g.set_layer_maze(2, true);
    for &(id, name, layer) in ROOMS {
        g.upsert_room(id, name.to_string());
        g.set_room_layer(id, layer);
    }
    for &(o, d, t) in edges {
        if o == t {
            g.add_self_loop(o, d);
        } else {
            g.add_edge(o, d, t);
        }
    }
    g.set_current(CHASM);
    g
}

/// The layout the shipped app runs on every turn that discovers a room
/// (`config::BackgroundTidy::EveryRoom`, the default).
fn tidy(g: &mut MapGraph) {
    app::tidy::tidy_layer_silent(g, CELLAR_LAYER);
}

fn pos(g: &MapGraph, id: RoomId) -> (i32, i32) {
    g.room(id).and_then(|r| r.pos).unwrap_or_else(|| panic!("room #{id} is placed"))
}

fn distorted(g: &MapGraph) -> Vec<(RoomId, D, RoomId)> {
    g.connections().iter().filter(|c| c.distorted).map(|c| (c.origin, c.dir, c.dest)).collect()
}

/// The cellar as the player walked it, asserted by ROOM rather than by cell: the troll room
/// due west of the passage, the round room due east, and the chasm up and to the right —
/// north-east — because that is the only thing its `southwest` return can mean.
fn assert_cellar_reads_right(g: &MapGraph, what: &str) {
    let (px, py) = pos(g, PASSAGE);
    let (cx, cy) = pos(g, CHASM);
    let (tx, ty) = pos(g, TROLL);
    let (rx, ry) = pos(g, ROUND);
    assert!(cy < py, "{what}: the chasm must lie NORTH of the passage (passage y={py}, chasm y={cy})");
    assert!(cx > px, "{what}: the chasm must lie EAST of the passage (passage x={px}, chasm x={cx})");
    assert!(tx < px, "{what}: the troll room must lie WEST of the passage (passage x={px}, troll x={tx})");
    assert_eq!(ty, py, "{what}: the troll room shares the passage's row");
    assert!(rx > px, "{what}: the round room must lie EAST of the passage (passage x={px}, round x={rx})");
    assert_eq!(ry, py, "{what}: the round room shares the passage's row");
}

/// The reported map, laid out again from the reported edge order. Before SQ-1291 this put
/// the chasm SOUTH of the passage and drew `112 SW 136` distorted.
#[test]
fn sq1291_the_reported_zork_cellar_puts_the_chasm_north_east_of_the_passage() {
    let mut g = build(STAIRS_FIRST);
    // Non-vacuity: the shape the defect needs must actually be present — the stairwell
    // walked from BOTH ends, and a one-way compass bearing disagreeing with it.
    let has = |o: RoomId, d: D, t: RoomId| {
        g.connections().iter().any(|c| c.origin == o && c.dir == d && c.dest == t)
    };
    assert!(has(PASSAGE, D::Down, CHASM) && has(CHASM, D::Up, PASSAGE), "the stairwell is reciprocated");
    assert!(has(CHASM, D::SW, PASSAGE), "and the chasm's one-way southwest return is on the map");
    assert!(!has(PASSAGE, D::NE, CHASM), "which nothing reciprocates — that is the whole contest");

    tidy(&mut g);
    assert_cellar_reads_right(&g, "stairs-first");

    let d = distorted(&g);
    assert!(
        !d.contains(&(CHASM, D::SW, PASSAGE)),
        "the chasm's own bearing must be honoured, not dropped, got {d:?}"
    );
    assert!(
        !d.contains(&(TROLL, D::E, PASSAGE)) && !d.contains(&(PASSAGE, D::W, TROLL)),
        "neither leg of the reciprocal troll/passage pair may be distorted, got {d:?}"
    );
    assert!(
        !d.contains(&(PASSAGE, D::E, ROUND)) && !d.contains(&(ROUND, D::W, PASSAGE)),
        "neither leg of the reciprocal passage/round-room pair may be distorted, got {d:?}"
    );
}

/// The same sixty-two edges with the chasm's three legs minted the other way round must
/// lay the cellar out identically. The layout is a function of the graph, not of the order
/// the player happened to walk it.
#[test]
fn sq1291_the_cellar_layout_does_not_depend_on_which_leg_was_walked_first() {
    let mut stairs_first = build(STAIRS_FIRST);
    let mut chasm_first_g = build(&chasm_first());
    tidy(&mut stairs_first);
    tidy(&mut chasm_first_g);
    assert_cellar_reads_right(&chasm_first_g, "chasm-first");

    for &(id, name, layer) in ROOMS {
        if layer != CELLAR_LAYER {
            continue;
        }
        assert_eq!(
            pos(&stairs_first, id),
            pos(&chasm_first_g, id),
            "#{id} {name:?} must land in the same cell whichever leg was walked first"
        );
    }
}
