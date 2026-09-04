//! `/view-map` and `/mark-maze-layer` — flagging a maze is a manual act (SQ-0666).

use mapper::direction::Direction;
use mapper::layer::{LayerId, MapView, MAIN_LAYER};
use mapper::mapper::Mapper;

use app::input::{apply_action, Action};
use app::slash::{parse, SlashOutcome};
use app::state::AppState;

fn advent() -> Mapper {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../unit_tests/advent_maze_map.json");
    mapper::persist::from_json(&std::fs::read_to_string(&path).expect("fixture")).expect("valid map")
}

const MAZE: LayerId = 1;

fn on_maze() -> (Mapper, AppState) {
    let m = advent();
    let mut st = AppState::default();
    st.set_viewed_layer(Some(MAZE));
    (m, st)
}

// ── Parsing ───────────────────────────────────────────────────────────────────

#[test]
fn view_map_cycles_bare_and_takes_a_named_view() {
    assert!(matches!(parse("view-map", '/'), SlashOutcome::Action(Action::ViewMap(None))));
    assert!(matches!(
        parse("view-map matrix", '/'),
        SlashOutcome::Action(Action::ViewMap(Some(MapView::Matrix)))
    ));
    assert!(matches!(
        parse("view-map DRAWN", '/'),
        SlashOutcome::Action(Action::ViewMap(Some(MapView::Drawn)))
    ));
    // A refusal has to SAY it is a refusal, not silently do nothing.
    match parse("view-map sideways", '/') {
        SlashOutcome::Error(e) => assert!(e.contains("drawn") && e.contains("matrix"), "{e}"),
        other => panic!("expected an error naming the two views, got {other:?}"),
    }
}

#[test]
fn mark_maze_layer_is_a_bare_command() {
    assert!(matches!(parse("mark-maze-layer", '/'), SlashOutcome::Action(Action::MarkMazeLayer)));
}

// ── Behaviour ─────────────────────────────────────────────────────────────────

#[test]
fn view_map_cycles_from_what_is_on_screen_and_stays_on_its_own_layer() {
    let (mut m, mut st) = on_maze();
    assert_eq!(m.graph.layer_view(MAZE), MapView::Drawn);

    apply_action(Action::ViewMap(None), &mut st, &mut m);
    assert_eq!(m.graph.layer_view(MAZE), MapView::Matrix);
    assert_eq!(m.graph.layer_view(MAIN_LAYER), MapView::Drawn, "the other layer is untouched");

    apply_action(Action::ViewMap(None), &mut st, &mut m);
    assert_eq!(m.graph.layer_view(MAZE), MapView::Drawn, "and back again");

    apply_action(Action::ViewMap(Some(MapView::Matrix)), &mut st, &mut m);
    assert_eq!(m.graph.layer_view(MAZE), MapView::Matrix, "a named view sets it outright");
}

/// A bare cycle must start from what the player is LOOKING at. On a maze-flagged layer with no
/// explicit choice that is the matrix, so the first `/view-map` has to go to drawn — reading the
/// stored `None` and cycling from the drawn default would leave the screen unchanged and the
/// command looking broken.
#[test]
fn a_bare_cycle_on_a_maze_layer_moves_off_the_matrix_not_onto_it() {
    let (mut m, mut st) = on_maze();
    m.graph.set_layer_maze(MAZE, true);
    assert_eq!(m.graph.layer_view(MAZE), MapView::Matrix, "flagged → matrix by default");
    assert_eq!(m.graph.layer_view_choice(MAZE), None, "…without an explicit choice");

    apply_action(Action::ViewMap(None), &mut st, &mut m);
    assert_eq!(m.graph.layer_view(MAZE), MapView::Drawn, "the cycle moved the screen");
}

/// The flag moves a DEFAULT. It never overwrites a view the player chose, and unflagging puts an
/// unchosen layer straight back to drawn instead of stranding it on the matrix.
#[test]
fn the_maze_flag_defaults_the_view_without_overriding_a_choice() {
    let (mut m, mut st) = on_maze();

    apply_action(Action::MarkMazeLayer, &mut st, &mut m);
    assert!(m.graph.layer_is_maze(MAZE));
    assert_eq!(m.graph.layer_view(MAZE), MapView::Matrix);
    assert_eq!(m.graph.layer_view_choice(MAZE), None, "no choice was invented on the player's behalf");

    apply_action(Action::MarkMazeLayer, &mut st, &mut m);
    assert!(!m.graph.layer_is_maze(MAZE));
    assert_eq!(m.graph.layer_view(MAZE), MapView::Drawn, "unflagging restores the drawn default");

    // Now with an explicit choice in place.
    apply_action(Action::ViewMap(Some(MapView::Drawn)), &mut st, &mut m);
    apply_action(Action::MarkMazeLayer, &mut st, &mut m);
    assert!(m.graph.layer_is_maze(MAZE));
    assert_eq!(
        m.graph.layer_view(MAZE),
        MapView::Drawn,
        "flagging a maze must not silently undo an explicit /view-map"
    );
}

/// Both commands change what the pane draws, so both must invalidate the render memo — a missed
/// bump paints a stale map (SQ-0305).
#[test]
fn both_commands_invalidate_the_map_render_memo() {
    let (mut m, mut st) = on_maze();
    let g0 = st.graph_gen;
    apply_action(Action::ViewMap(None), &mut st, &mut m);
    assert_ne!(st.graph_gen, g0, "/view-map bumps the generation");
    let g1 = st.graph_gen;
    apply_action(Action::MarkMazeLayer, &mut st, &mut m);
    assert_ne!(st.graph_gen, g1, "/mark-maze-layer bumps it too");
}

// ── Matrix navigation ─────────────────────────────────────────────────────────

#[test]
fn arrows_step_the_selection_only_while_the_matrix_is_showing() {
    let (mut m, mut st) = on_maze();

    // Drawn layer: the arrows are inert, exactly as they were before the matrix existed.
    apply_action(Action::MatrixMove(1), &mut st, &mut m);
    assert_eq!(st.selected_room, None, "no selection is invented on a drawn layer");

    apply_action(Action::ViewMap(Some(MapView::Matrix)), &mut st, &mut m);
    apply_action(Action::MatrixMove(1), &mut st, &mut m);
    let first = st.selected_room.expect("the first press selects");
    assert_eq!(first, m.graph.current().unwrap(), "…the room you are standing in");

    apply_action(Action::MatrixMove(-1), &mut st, &mut m);
    let up = st.selected_room.unwrap();
    assert_ne!(up, first, "and the next press actually moves");

    // Home/End are spelled as saturating extremes — they must not overflow.
    apply_action(Action::MatrixMove(i32::MIN), &mut st, &mut m);
    assert_eq!(st.selected_room, Some(m.graph.rooms_in_layer(MAZE)[0]));
    apply_action(Action::MatrixMove(i32::MAX), &mut st, &mut m);
    assert_eq!(st.selected_room, Some(*m.graph.rooms_in_layer(MAZE).last().unwrap()));
}

#[test]
fn column_scrolling_is_clamped_and_only_applies_to_the_matrix() {
    let (mut m, mut st) = on_maze();
    apply_action(Action::MatrixPanColumns(3), &mut st, &mut m);
    assert_eq!(st.matrix_scroll.0, 0, "a drawn layer does not scroll columns");

    apply_action(Action::ViewMap(Some(MapView::Matrix)), &mut st, &mut m);
    apply_action(Action::MatrixPanColumns(3), &mut st, &mut m);
    assert_eq!(st.matrix_scroll.0, 3);
    apply_action(Action::MatrixPanColumns(-99), &mut st, &mut m);
    assert_eq!(st.matrix_scroll.0, 0, "never before the first column");
    apply_action(Action::MatrixPanColumns(99), &mut st, &mut m);
    assert_eq!(st.matrix_scroll.0, 11, "nor past the last of the twelve");
}

/// The wheel and Shift+arrows are the pane-scroll conventions; on a matrix layer they must scroll
/// the TABLE, not a grid viewport nobody is looking at.
#[test]
fn panning_scrolls_the_table_when_the_matrix_is_showing() {
    let (mut m, mut st) = on_maze();
    apply_action(Action::Pan(0, 3), &mut st, &mut m);
    assert_eq!(st.matrix_scroll, (0, 0), "a drawn layer pans its viewport instead");
    assert_eq!(st.scroll, (0, 3), "…which is exactly what it did before");

    apply_action(Action::ViewMap(Some(MapView::Matrix)), &mut st, &mut m);
    let viewport = st.scroll;
    apply_action(Action::Pan(0, 2), &mut st, &mut m);
    assert_eq!(st.matrix_scroll.1, 2, "the table scrolled");
    assert_eq!(st.scroll, viewport, "and the drawn view's viewport was left alone");

    apply_action(Action::Pan(0, -99), &mut st, &mut m);
    assert_eq!(st.matrix_scroll.1, 0, "never above the first row");
    apply_action(Action::Pan(0, 99), &mut st, &mut m);
    assert_eq!(st.matrix_scroll.1, 11, "nor past the last of the twelve rooms");
}

// ── Trail ─────────────────────────────────────────────────────────────────────

#[test]
fn the_trail_remembers_the_last_eight_steps_and_ignores_standing_still() {
    let mut st = AppState::default();
    for id in 1..=12u16 {
        st.push_trail(id.into());
    }
    assert_eq!(st.map_trail.len(), app::state::MAP_TRAIL_LEN);
    assert_eq!(st.map_trail.back(), Some(&12), "the newest step is last");
    assert_eq!(st.trail_age(12), Some(0), "…and is age 0");
    assert_eq!(st.trail_age(5), Some(7), "the eighth-oldest step is still on the trail");
    assert_eq!(st.trail_age(4), None, "the ones before it have fallen off");

    st.push_trail(12); // a `look`, or a move that failed
    assert_eq!(st.map_trail.len(), app::state::MAP_TRAIL_LEN, "standing still is not a step");
    assert_eq!(st.trail_age(11), Some(1), "so the trail behind it is undisturbed");
}

/// A maze self-loop and a wall look identical to the mapper. Only a turn whose output proves the
/// player arrived may mint the loop.
#[test]
fn a_self_loop_needs_proof_of_arrival() {
    let mut m = Mapper::default();
    m.observe(1, "Maze", None);
    m.observe(1, "Maze", Some(Direction::E)); // bounced
    assert!(m.graph.self_loops(1).is_empty());
    m.observe_moved(1, "Maze", Some(Direction::W)); // arrived
    assert_eq!(m.graph.self_loops(1), vec![Direction::W]);
}
