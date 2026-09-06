//! The Zork I house makes room for its `Attic` and its `Studio` ghost (SQ-1375).
//!
//! The user, looking at the mapgen Zork I map (`docs/zork1-map.svg`): *"the zork house is not
//! making room for the ghost rooms and attic in the mapgen app. did that rule not apply there?"*
//! The rule is `mapper::layout::seat` — SQ-1356's line-opening, SQ-1358's chain push, SQ-1363's
//! dependant party, SQ-1367's bare-column fallback — and it did apply. It was being vetoed.
//!
//! # What was actually wrong
//!
//! `seat::adjacent_reciprocals` is the one invariant the seating pass will not trade away: a pair
//! of rooms joined by a reciprocal passage and standing exactly one cell apart stays exactly one
//! cell apart. Its own doc comment, this module's docs and `layout::edge_is_satisfied` (SQ-1364)
//! all say the same thing about which passages get to make that claim — a CARDINAL one does, and
//! *"a passage that is one-way, diagonal, gated or already stretched may lengthen"*, because a
//! diagonal only ever pinned its far end to a quadrant. But the code asked `grid_offset`, which
//! answers `Some` for all eight compass points, so a DIAGONAL pair was every bit as binding.
//!
//! On Zork I that one line decided the house. `Attic #195` is a portal-only leaf hanging off
//! `Kitchen #28` by a staircase and wants the cell directly above it, where `North of House #143`
//! stands. Opening that row would have moved `North of House` off the diagonal it sits on from
//! `Behind House #89` — a diagonal **the layout itself wrote**, as the repair for a walked `E` that
//! came out distorted — and that vetoed the whole slide. The chain push was then vetoed twice
//! over, by the same diagonal and by `Forest #91` sitting due west of `Forest Path #247` in the
//! column being pushed. So seating fell all the way through to its last resort, walking out along
//! the bearing: the `Attic` was drawn at `(1, -1)`, four cells up a column of its own, past
//! `North of House`, `Forest Path` and `Clearing`.
//!
//! The `Studio` ghost — `Studio #229` on the Cellar layer, reached by `#229 U #28` — wanted the
//! cell below the `Kitchen`, where `South of House #217` stands, and was walked out to `(1, 5)`
//! past it by the same last resort.
//!
//! # The fix, and what each half does
//!
//! Two changes in `mapper::layout::seat`, and it is worth knowing which one moved which room:
//!
//! * **A diagonal reciprocal claims no cell count.** This is what the `Attic` needed. With the
//!   `Behind House` diagonal free to stretch, the row simply OPENS (seating's step 2) — every
//!   cardinal pair on the map keeps its cell, the northern half of the layer slides up one, and
//!   the `Attic` takes the doorstep. It is also what let the `Studio` ghost's push succeed:
//!   `South of House` is joined to `Behind House` by another layout-written diagonal, and that was
//!   the only thing vetoing the bare-column push.
//! * **A kept adjacency travels with the party rather than vetoing it** (`stranded_partners`).
//!   The user's standing rule — when a room gets pushed, the rooms that depend on it get pushed
//!   the same way — extended from SQ-1363's "downwind" neighbours to the side neighbour whose
//!   adjacency the map is promising to keep. On this map it changes nothing, because the diagonal
//!   fix reaches both specimens first; the synthetic cases in `seat.rs` are where it is pinned.
//!
//! Falsified: with the `grid_offset(...).filter(...)` in `adjacent_reciprocals` reverted,
//! [`the_attic_sits_on_the_kitchens_doorstep`] fails with `Attic #195 at (1, -1)` — the field
//! symptom, to the cell.
//!
//! Zork I lives under the gitignored `stories/`, so every case here skips vacuously off CI and
//! says so. Run `cargo nextest run -p lanthorn sq1375` against a checkout with `stories/`.

use std::path::{Path, PathBuf};

use mapper::graph::RoomId;
use mapper::layer::LayerId;

/// Zork I release 52 / serial 871125 — the InvisiClues edition, the fixture the quest was
/// reported on and the one `docs/zork1-map.svg` is generated from.
const ZORK1: &str = "zork1-invclues-r52-s871125.z5";

/// `Kitchen`, the room both specimens hang off: `#28 U #195` upstairs to the `Attic`, and
/// `#229 U #28` downstairs from the `Studio` on the Cellar layer.
const KITCHEN: RoomId = 28;
/// `Attic` — a PORTAL-ONLY leaf (`#195 D #28` is its only passage), so the stress solve has no
/// compass bearing to seat it by and `seat_portal_leaves` places it last.
const ATTIC: RoomId = 195;
/// `Studio`, on the Cellar layer: what the Main layer's ghost box stands for.
const STUDIO: RoomId = 229;

fn story() -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(ZORK1);
    p.is_file().then_some(p)
}

/// The layer the house is on, found by the `Kitchen` rather than by name or index — a layer's
/// display name is the user's to change and its id is an allocation order.
fn main_layer(map: &app::mapgen::GeneratedMap) -> LayerId {
    map.graph.layer_of(KITCHEN)
}

/// **The reported symptom, at the cell.** The `Attic` sits directly above the `Kitchen` on the
/// Main layer instead of four cells up a column of its own.
///
/// Both halves matter. The equality is the fix; the inequality names the exact cell the field
/// report was looking at, so a future change that moves the `Attic` somewhere else entirely still
/// has to come here and say what it did.
#[test]
fn the_attic_sits_on_the_kitchens_doorstep() {
    let Some(path) = story() else {
        eprintln!("SKIP the_attic_sits_on_the_kitchens_doorstep: stories/{ZORK1} absent");
        return;
    };
    let map = app::mapgen::generate(&path, true).expect("mapgen");
    let kitchen = map.graph.room(KITCHEN).and_then(|r| r.pos).expect("the Kitchen must be placed");
    let attic = map.graph.room(ATTIC).and_then(|r| r.pos).expect("the Attic must be placed");
    assert_eq!(
        map.graph.layer_of(ATTIC),
        map.graph.layer_of(KITCHEN),
        "the Attic is on the house's own layer, so this is a seating question, not a ghost one"
    );
    assert_eq!(
        attic,
        (kitchen.0, kitchen.1 - 1),
        "Attic #195 at {attic:?} is not on Kitchen #28's doorstep ({kitchen:?})"
    );
    assert_ne!(attic, (kitchen.0, kitchen.1 - 4), "…and specifically not the reported (1, -1)");
}

/// **The other half of the report: the `Studio` ghost.** `render_layer` seats a cross-layer ghost
/// in the layer's own grid, so this reads the RENDER rather than the graph — the graph has no cell
/// for a room that lives somewhere else.
///
/// The ghost is found by the room id it stands for, not by label: `GhostRoom::label_for` spells a
/// one-way crossing `to Studio` and a two-way one `Studio`, and which of those applies is not this
/// case's business.
///
/// Note the anchor is re-read from the render too. Seating a ghost may OPEN a line in the plane,
/// which moves the layer's real rooms — the `Kitchen`'s cell in `map.graph` is the one before that
/// happened, and comparing against it would be measuring two different maps against each other.
#[test]
fn the_studio_ghost_sits_below_the_kitchen() {
    let Some(path) = story() else {
        eprintln!("SKIP the_studio_ghost_sits_below_the_kitchen: stories/{ZORK1} absent");
        return;
    };
    let map = app::mapgen::generate(&path, true).expect("mapgen");
    let rm = mapper::render::render_layer(&map.graph, main_layer(&map));
    let kitchen = rm
        .rooms
        .iter()
        .find(|r| r.id == KITCHEN)
        .expect("non-vacuity: the Kitchen must be on the rendered Main layer")
        .cell;
    let studio = rm
        .rooms
        .iter()
        .find(|r| r.id == STUDIO)
        .expect("the Cellar crossing must draw a Studio ghost on Main");
    assert!(studio.ghost.is_some(), "#{STUDIO} is on another layer: it is a ghost here");
    assert_eq!(
        studio.cell,
        (kitchen.0, kitchen.1 + 1),
        "the Studio ghost at {:?} is not on Kitchen #28's doorstep ({kitchen:?})",
        studio.cell
    );
    assert_ne!(studio.cell, (kitchen.0, kitchen.1 + 2), "…and not past South of House");
}

/// The invariant the fix is only allowed to be right BECAUSE of: every cardinal reciprocal pair
/// that the layout brought together is still exactly one cell apart on the rendered Main layer.
///
/// The seating pass opens lines and pushes columns to make room for the two boxes above, and this
/// is what says it did not pay for them out of the map's own adjacencies. Diagonals are excluded
/// deliberately — stretching one is the layout's ordinary currency (SQ-1364), and that is the very
/// permission this quest restored.
#[test]
fn no_cardinal_pair_on_the_main_layer_was_pulled_apart() {
    let Some(path) = story() else {
        eprintln!("SKIP no_cardinal_pair_on_main_was_pulled_apart: stories/{ZORK1} absent");
        return;
    };
    let map = app::mapgen::generate(&path, true).expect("mapgen");
    let layer = main_layer(&map);
    let rm = mapper::render::render_layer(&map.graph, layer);
    let cell = |id: RoomId| rm.rooms.iter().find(|r| r.id == id).map(|r| r.cell);
    let laid = |id: RoomId| map.graph.room(id).and_then(|r| r.pos);
    let conns = map.graph.connections();
    let mut checked = 0usize;
    for c in conns {
        if c.is_self_loop() || map.graph.layer_of(c.origin) != layer {
            continue;
        }
        let Some((dx, dy)) = mapper::direction::grid_offset(c.dir) else { continue };
        if dx != 0 && dy != 0 {
            continue; // a diagonal claims a quadrant, never a cell count
        }
        if !conns.iter().any(|o| o.origin == c.dest && o.dest == c.origin) {
            continue; // one-way: no cell-count claim either
        }
        let (Some(la), Some(lb)) = (laid(c.origin), laid(c.dest)) else { continue };
        if (lb.0 - la.0, lb.1 - la.1) != (dx, dy) {
            continue; // already stretched before the ghosts were seated: nothing to preserve
        }
        let (Some(a), Some(b)) = (cell(c.origin), cell(c.dest)) else { continue };
        assert_eq!(
            (b.0 - a.0, b.1 - a.1),
            (dx, dy),
            "seating the ghosts pulled #{} {:?} #{} apart: {a:?} → {b:?}",
            c.origin,
            c.dir,
            c.dest
        );
        checked += 1;
    }
    // Non-vacuity: the house alone contributes several, so a run that checked none has stopped
    // measuring anything.
    assert!(checked > 10, "only {checked} cardinal pairs held on Main — the map has changed shape");
}
