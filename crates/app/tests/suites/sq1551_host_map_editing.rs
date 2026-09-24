//! SQ-1551: map-editing functions a non-terminal host needs, made reachable from outside `app`'s
//! own binary.
//!
//! A headless host building the layer UI on top of `AppState`/`Mapper` had to: (1) mirror
//! `input.rs`'s private `choose_region`/`move_targets` ordering using only `mapper`'s public API
//! for its own "Move to another layer…" action, which can silently drift from what the TUI's
//! `move-region` command actually does; (2) bypass `apply_region_prompt`'s accept entirely to
//! avoid the maze flag it bundles in for name-triggered suggestions, since the host shows such a
//! layer in its own matrix view rather than using maze flags at all; and (3) hash tried directions
//! itself because `MapGraph::mark_tried` did not bump `struct_gen`, so a host that re-sends its
//! layout only on `struct_gen` changes missed the matrix view's tried-direction markers.
//!
//! These tests exercise the fix from outside the crate's own binary, the way such a host would.

use mapper::direction::Direction;
use mapper::layer::MoveTarget;
use mapper::mapper::Mapper;
use mapper::suggest::Trigger;

use app::input::{
    accept_layer_suggestion, apply_region_prompt, choose_region, move_targets,
    offer_layer_suggestion, suggestion_sets_maze_flag, SeamRefusal,
};
use app::state::{AppState, RegionOption, RegionPromptAct, RegionPromptKind};

// ── Fixtures, mirroring `input.rs`'s own private ones (SQ-0439's three seam tiers) ────────────

/// TIER 1: a portal-bounded region needs no seam search at all. The Maze's only ways out are a
/// one-way portal DOWN back to the hall and an Unknown portal to Inside Building, so the compass
/// walk from room 3 stops at those portals and the region is a proper subset of its layer.
fn advent_maze() -> Mapper {
    let mut m = Mapper::default();
    m.observe(1, "Inside Building", None);
    m.observe(2, "At West End of Long Hall", Some(Direction::Down));
    m.observe(3, "Maze", Some(Direction::S));
    m.graph.upsert_room(4, "Maze".into());
    for (a, d, b) in [
        (3, Direction::Down, 2),
        (3, Direction::N, 4),
        (4, Direction::S, 3),
        (4, Direction::Unknown, 1),
    ] {
        m.graph.add_edge(a, d, b);
    }
    m
}

/// TIER 2: the only way into the maze is a one-way SOUTH passage, with the way back a portal — so
/// the compass walk covers the whole layer (no portal edge to stop at) and there is exactly one
/// inbound bridge to auto-pick.
fn one_way_maze() -> Mapper {
    let mut m = Mapper::default();
    for (id, n) in [(1, "At West End of Long Hall"), (2, "Maze"), (3, "Maze"), (4, "Maze")] {
        m.graph.upsert_room(id, n.into());
    }
    for (a, d, b) in [
        (1, Direction::S, 2),
        (2, Direction::Down, 1),
        (2, Direction::N, 3),
        (3, Direction::S, 2),
        (2, Direction::E, 4),
        (4, Direction::N, 3),
    ] {
        m.graph.add_edge(a, d, b);
    }
    m.graph.set_current(2);
    m
}

/// TIER 3: a straight compass chain with no portal in it, so two different edges both cut a valid
/// boundary and neither can be auto-picked — cutting at room 1-2 leaves {1} apart from {2,3,4},
/// cutting at 2-3 leaves {1,2} apart from {3,4}.
fn corridor() -> Mapper {
    let mut m = Mapper::default();
    for (id, n) in [(1, "A"), (2, "B"), (3, "C"), (4, "D")] {
        m.graph.upsert_room(id, n.into());
    }
    for (a, b) in [(1, 2), (2, 3), (3, 4)] {
        m.graph.add_edge(a, Direction::E, b);
        m.graph.add_edge(b, Direction::W, a);
    }
    m.graph.set_current(1);
    m
}

/// A hall with a maze through its south door — the semantic (name) trigger's shape, mirroring
/// `input.rs`'s own `maze_doorway` fixture. Stops one step short so the caller walks in.
fn maze_doorway() -> Mapper {
    let mut m = Mapper::default();
    m.observe(1, "At West End of Long Hall", None);
    m.observe(7, "Storeroom", Some(Direction::N));
    m.observe(1, "At West End of Long Hall", Some(Direction::S));
    m
}

// ── choose_region / move_targets, called the way a host's own layer UI would ───────────────────

#[test]
fn choose_region_tier1_takes_the_portal_bounded_region_with_no_seam() {
    let m = advent_maze();
    let (region, seam) =
        choose_region(&m.graph, 3, None).expect("a portal-bounded region needs no refusal");
    assert_eq!(
        region.rooms.iter().copied().collect::<Vec<_>>(),
        vec![2, 3, 4],
        "the compass walk stops at the portals and takes everything inside them"
    );
    assert!(seam.is_none(), "nothing was cut to get here, so nothing to report");
    assert_eq!(
        move_targets(&m.graph, &region),
        vec![MoveTarget::New],
        "Main is the only other layer and it cannot take a region that isn't its own whole self"
    );
}

#[test]
fn choose_region_tier2_finds_the_unique_one_way_bridge() {
    let m = one_way_maze();
    let (region, seam) =
        choose_region(&m.graph, 2, None).expect("exactly one inbound bridge auto-picks");
    assert_eq!(
        region.rooms.iter().copied().collect::<Vec<_>>(),
        vec![2, 3, 4],
        "the whole maze leaves, since the compass walk covered the whole layer"
    );
    assert_eq!(seam, Some((1, Direction::S)), "the one-way S passage from the hall is the cut");
}

#[test]
fn choose_region_tier3_refuses_ambiguous_seams_by_name() {
    let m = corridor();
    match choose_region(&m.graph, 2, None) {
        Err(SeamRefusal::Ambiguous(seams)) => {
            assert_eq!(seams.len(), 2, "both the passage from A and the one from C are valid cuts");
        }
        other => panic!("expected an ambiguous refusal, got {other:?}"),
    }
}

#[test]
fn move_targets_offers_only_a_new_layer_when_main_is_the_sole_layer() {
    let m = advent_maze();
    let (region, _) = choose_region(&m.graph, 3, None).unwrap();
    assert_eq!(move_targets(&m.graph, &region), vec![MoveTarget::New]);
}

// ── accept_layer_suggestion: the maze flag is now a caller's choice ────────────────────────────

/// A host that does not use maze flags accepts the very same name-triggered suggestion the TUI
/// would, but tells `accept_layer_suggestion` to skip the flag — mirroring `input.rs`'s own
/// in-crate `accepting_a_maze_suggestion_flags_the_layer`, but asserting the opposite outcome.
#[test]
fn accepting_a_name_triggered_suggestion_without_the_flag_leaves_maze_unset() {
    let mut s = AppState::default();
    let mut m = maze_doorway();
    m.observe(2, "Maze", Some(Direction::S));
    offer_layer_suggestion(&mut s, &mut m);

    let (trigger, region, target) = {
        let p = s.overlays.region_prompt.as_ref().expect("the map has something to say");
        let RegionPromptKind::Suggest { trigger, region, .. } = &p.kind else {
            panic!("expected a Suggest prompt: {:?}", p.kind)
        };
        let Some(RegionOption::Dest { target, .. }) = p.chosen() else {
            panic!("expected a Dest option")
        };
        (*trigger, region.clone(), *target)
    };
    assert_eq!(trigger, Trigger::Name, "the room's own name is the trigger");
    assert!(
        suggestion_sets_maze_flag(trigger),
        "the TUI's own accept path would set the flag for this trigger"
    );

    let landed = accept_layer_suggestion(&mut s, &mut m, &region, target, false)
        .expect("the move itself must still happen");
    assert!(
        !m.graph.layer_is_maze(landed),
        "accepting WITHOUT the flag must leave the layer's maze bit unset"
    );
}

/// The TUI's own default path — `apply_region_prompt` — is unchanged: a name-triggered accept
/// still sets the flag.
#[test]
fn the_tuis_default_accept_path_still_sets_the_maze_flag() {
    let mut s = AppState::default();
    let mut m = maze_doorway();
    m.observe(2, "Maze", Some(Direction::S));
    offer_layer_suggestion(&mut s, &mut m);
    apply_region_prompt(&mut s, &mut m, RegionPromptAct::Accept);

    let landed = s.viewed_layer.expect("the maze moved to a layer of its own");
    assert!(m.graph.layer_is_maze(landed), "the TUI's own path is unchanged: it still sets the flag");
}

/// A STRUCTURAL suggestion never sets the flag, with or without the caller opting in — there is
/// nothing to opt into, since `suggestion_sets_maze_flag` says no for this trigger.
#[test]
fn suggestion_sets_maze_flag_is_false_for_a_structural_trigger() {
    assert!(!suggestion_sets_maze_flag(Trigger::Structural));
    assert!(suggestion_sets_maze_flag(Trigger::Name));
}
