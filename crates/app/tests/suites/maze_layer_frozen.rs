//! A maze layer's geometry is frozen, and its pane owes nothing to the layout pipeline (SQ-0671).
//!
//! The player reported the map pane cycling colours while walking a maze in the matrix view. The
//! cause was the tidy loop: a layer that is ~96% non-Euclidean has no compass layout to converge
//! on, so every turn produced a different one, every arriving result bumped the graph generation,
//! and the render worker that bump spawned pulsed the pane border red/green — over a table that
//! reads the graph and never looks at a routed model at all.
//!
//! Three things are pinned here: nothing schedules (or applies) a tidy for a maze layer, a table
//! pane asks for no render model and takes no pulse, and `tidy-map` says so instead of working.

use mapper::direction::Direction;
use mapper::graph::{MapGraph, RoomId};
use mapper::layer::{LayerId, MapView, MAIN_LAYER};
use mapper::mapper::Mapper;

use app::input::{apply_action, Action};
use app::session::{apply_turn, TurnResult};
use app::state::{AppState, TidyJob, TidyKind};
use app::tidy::{
    apply_tidy_result, cleanup_overlaps_layer_silent, layer_is_frozen, should_schedule_tidy,
    tidy_layer_silent, ApplyTidyOutcome,
};

/// A turn result that reports arriving in `(num, name)`, with the room heading printed — the
/// evidence a real move leaves.
fn turn(num: RoomId, name: &str) -> TurnResult {
    TurnResult {
        transcript: format!("{name}\n"),
        transcript_runs: Vec::new(),
        location: Some(app::engine::LocationInfo { number: num, parent: 0, name: name.into() }),
        quit: false,
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
    }
}

/// Two layers: a tangled maze (rooms 1–4, cross-linked so no compass layout satisfies it) peeled
/// onto its own layer and flagged, plus an ordinary corridor left on Main.
fn maze_and_corridor() -> (Mapper, LayerId) {
    let mut m = Mapper::default();
    // The corridor, on Main: A -E-> B -E-> C with reciprocals, deliberately placed badly so a
    // tidy has something to fix.
    for (id, n) in [(10u16, "Hall"), (11, "Study"), (12, "Attic")] {
        m.graph.upsert_room(id.into(), n.into());
    }
    for (a, b) in [(10, 11), (11, 12)] {
        m.graph.add_edge(a, Direction::E, b);
        m.graph.add_edge(b, Direction::W, a);
    }
    m.graph.set_pos(10, (0, 0));
    m.graph.set_pos(11, (7, 6));
    m.graph.set_pos(12, (7, 6)); // overlapping: cleanup has work to do

    // The maze, on its own layer: every room called "Maze", almost nothing reciprocal.
    let maze = m.graph.new_layer(Some(MAIN_LAYER), "Maze".into());
    for id in [1u16, 2, 3, 4] {
        m.graph.upsert_room(id.into(), "Maze".into());
        m.graph.set_room_layer(id.into(), maze);
    }
    for (o, d, dst) in [
        (1, Direction::N, 2),
        (2, Direction::N, 3),
        (3, Direction::N, 1),
        (1, Direction::W, 3),
        (3, Direction::W, 4),
        (4, Direction::E, 2),
        (2, Direction::S, 4),
    ] {
        m.graph.add_edge(o, d, dst);
    }
    for (id, p) in [(1u16, (0, 0)), (2, (0, -1)), (3, (0, -2)), (4, (-1, -1))] {
        m.graph.set_pos(id.into(), p);
    }
    m.graph.set_layer_maze(maze, true);
    m.graph.set_current(1);
    (m, maze)
}

fn positions(g: &MapGraph, layer: LayerId) -> Vec<(RoomId, Option<(i32, i32)>)> {
    g.rooms_in_layer(layer).into_iter().map(|id| (id, g.room(id).and_then(|r| r.pos))).collect()
}

// ── 1a: the freeze ────────────────────────────────────────────────────────────

#[test]
fn a_maze_layer_schedules_no_tidy_while_its_neighbours_still_do() {
    let (m, maze) = maze_and_corridor();
    assert!(layer_is_frozen(&m.graph, maze), "the maze flag is what freezes a layer");
    assert!(!layer_is_frozen(&m.graph, MAIN_LAYER));

    assert!(
        !should_schedule_tidy(&m.graph, maze, true),
        "a geometry change on a maze layer schedules nothing"
    );
    assert!(
        should_schedule_tidy(&m.graph, MAIN_LAYER, true),
        "the layers beside it are untouched by the freeze"
    );
    assert!(
        !should_schedule_tidy(&m.graph, MAIN_LAYER, false),
        "and an unchanged turn still schedules nothing, as before"
    );
}

/// The user-visible half: walk a maze layer for several turns and its rooms do not move a cell,
/// while the ordinary layer beside it is tidied as usual on the same turns.
#[test]
fn maze_positions_are_byte_stable_across_turns_while_the_other_layer_tidies() {
    let (mut m, maze) = maze_and_corridor();
    let before_maze = positions(&m.graph, maze);
    let before_main = positions(&m.graph, MAIN_LAYER);

    // Four turns of walking the maze, each one the loop the run loop runs: observe, then
    // schedule background maintenance for the layer the player is standing in.
    for (cmd, id) in [("north", 2u32), ("north", 3), ("west", 4), ("east", 2)] {
        apply_turn(&mut m, cmd, &turn(id, "Maze"), &mut Default::default());
        if should_schedule_tidy(&m.graph, maze, true) {
            tidy_layer_silent(&mut m.graph, maze);
        }
    }
    assert_eq!(positions(&m.graph, maze), before_maze, "no maze room moved a single cell");

    // The same turn loop on the ordinary layer does move rooms — otherwise this test would pass
    // with the whole tidy pipeline broken.
    assert!(should_schedule_tidy(&m.graph, MAIN_LAYER, true));
    tidy_layer_silent(&mut m.graph, MAIN_LAYER);
    assert_ne!(
        positions(&m.graph, MAIN_LAYER),
        before_main,
        "the non-maze layer is still tidied (it started with an overlap)"
    );
    assert_eq!(positions(&m.graph, maze), before_maze, "…and tidying it left the maze alone");
}

/// The freeze is on the OPTIMIZATION only. A room discovered on a frozen layer is still
/// dead-reckoned into place on the turn it is found — the map would otherwise grow rooms with no
/// position at all, and switching the layer back to the drawn view would show nothing.
#[test]
fn a_new_room_on_a_frozen_layer_is_still_placed_and_moves_nothing_else() {
    let (mut m, maze) = maze_and_corridor();
    let before = positions(&m.graph, maze);

    apply_turn(&mut m, "down", &turn(5, "Maze"), &mut Default::default());
    if should_schedule_tidy(&m.graph, maze, true) {
        tidy_layer_silent(&mut m.graph, maze);
    }

    let new = m.graph.room(5).expect("the new room joined the graph");
    assert_eq!(m.graph.layer_of(5), maze, "and joined the layer the player is standing on");
    assert!(new.pos.is_some(), "a newly discovered room is placed even on a frozen layer");
    assert!(
        m.graph.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::Down && c.dest == 5),
        "the edge is minted too: the freeze stops layout, not mapping"
    );
    let after: Vec<_> = positions(&m.graph, maze).into_iter().filter(|(id, _)| *id != 5).collect();
    assert_eq!(after, before, "the rooms already placed did not shift to make room for it");
}

/// Belt as well as braces: even called directly, the two silent entry points the background
/// worker runs refuse a frozen layer.
#[test]
fn the_silent_tidy_entry_points_refuse_a_frozen_layer() {
    let (mut m, maze) = maze_and_corridor();
    let before = positions(&m.graph, maze);
    tidy_layer_silent(&mut m.graph, maze);
    assert_eq!(positions(&m.graph, maze), before, "full tidy is a no-op on a maze layer");
    cleanup_overlaps_layer_silent(&mut m.graph, maze);
    assert_eq!(positions(&m.graph, maze), before, "so is overlap cleanup");
}

/// A job in flight when the player flags the layer must not land afterwards — `/mark-maze-layer`
/// during a background tidy is exactly when a player reaches for it.
#[test]
fn a_result_that_lands_after_the_flag_is_dropped_rather_than_applied() {
    let (mut m, maze) = maze_and_corridor();
    // The worker's output: the same graph, tidied while the layer was still ordinary.
    let mut tidied = m.graph.clone();
    tidied.set_layer_maze(maze, false);
    tidy_layer_silent(&mut tidied, maze);
    assert_ne!(
        positions(&tidied, maze),
        positions(&m.graph, maze),
        "the worker really did move rooms — otherwise this proves nothing"
    );

    let before = positions(&m.graph, maze);
    let outcome = apply_tidy_result(&mut m.graph, tidied, maze, 7, 7);
    assert!(
        matches!(outcome, ApplyTidyOutcome::Applied),
        "reported as handled, so nothing re-triggers a fresh job"
    );
    assert_eq!(positions(&m.graph, maze), before, "…but not one room was moved by it");
}

// ── 1b: the matrix pane is independent of the layout pipeline ─────────────────

/// A background tidy for some other layer is in flight. On a drawn map that pulses the border and
/// re-routes; on the matrix it must do neither.
fn state_on(layer: LayerId, graph: &MapGraph) -> AppState {
    let mut st = AppState::default();
    st.set_viewed_layer(Some(layer));
    assert_eq!(st.active_layer(graph), layer);
    st
}

fn with_tidy_job(st: &mut AppState, layer: LayerId) {
    let handle = std::thread::spawn(MapGraph::new);
    st.tidy_job = Some(TidyJob {
        handle,
        layer,
        gen: 0,
        started: std::time::Instant::now(),
        kind: TidyKind::Cleanup,
    });
}

#[test]
fn a_tidy_in_flight_does_not_pulse_or_re_route_a_matrix_pane() {
    let (m, maze) = maze_and_corridor();
    assert_eq!(m.graph.layer_view(maze), MapView::Matrix, "a maze layer defaults to the table");

    // The control: the same job, the same graph, viewed on the DRAWN layer.
    let mut drawn = state_on(MAIN_LAYER, &m.graph);
    with_tidy_job(&mut drawn, MAIN_LAYER);
    assert!(
        drawn.map_job_pulse_elapsed(&m.graph).is_some(),
        "a drawn map still says a layout job is running"
    );
    assert!(drawn.live_map_render(MAIN_LAYER, &m.graph).is_some(), "and still routes a model");

    // The matrix: same in-flight job — for the OTHER layer, as the run loop would have it.
    let mut matrix = state_on(maze, &m.graph);
    with_tidy_job(&mut matrix, MAIN_LAYER);
    assert!(
        matrix.map_job_pulse_elapsed(&m.graph).is_none(),
        "a table pane must not restyle its border for a layout job it never draws"
    );
    assert!(
        matrix.live_map_render(maze, &m.graph).is_none(),
        "and must ask for no routed model at all"
    );
    assert!(
        !matrix.map_render_in_flight(),
        "so no render worker is spawned — the source of the pulse the player saw"
    );
}

/// Switching the same layer back to the drawn view restores both.
#[test]
fn switching_the_view_back_puts_the_map_pane_back_on_the_pipeline() {
    let (mut m, maze) = maze_and_corridor();
    let mut st = state_on(maze, &m.graph);
    with_tidy_job(&mut st, MAIN_LAYER);
    assert!(st.live_map_render(maze, &m.graph).is_none());

    m.graph.set_layer_view(maze, Some(MapView::Drawn));
    assert!(st.live_map_render(maze, &m.graph).is_some(), "the drawn view needs its model back");
    assert!(st.map_job_pulse_elapsed(&m.graph).is_some(), "and the pulse means something again");
}

// ── 1c: tidy-map on a frozen layer ────────────────────────────────────────────

#[test]
fn tidy_map_on_a_maze_layer_says_so_instead_of_working() {
    let (mut m, maze) = maze_and_corridor();
    let mut st = state_on(maze, &m.graph);
    let before = positions(&m.graph, maze);

    apply_action(Action::Retidy, &mut st, &mut m);

    assert_eq!(
        st.notifications.latest_text(),
        Some("maze layer: geometry is frozen — the matrix is the view"),
    );
    assert!(st.anim_build_job.is_none(), "no tidy build was started");
    assert_eq!(positions(&m.graph, maze), before, "and nothing moved");

    // On an ordinary layer the command still works: a build is spawned and says so.
    let mut st2 = state_on(MAIN_LAYER, &m.graph);
    apply_action(Action::Retidy, &mut st2, &mut m);
    assert!(st2.anim_build_job.is_some(), "the command is unharmed elsewhere");
    assert_eq!(st2.notifications.latest_text(), Some("tidying map…"));
}
