//! Clicking a room in the matrix highlights the way there (SQ-0693), against REAL player data.
//!
//! `unit_tests/advent_maze_map.json` and `unit_tests/zork1_underground_map.json` are verbatim
//! `map.json` files out of lanthorn archives — one player's partial Colossal Cave with the "all
//! alike" maze hand-peeled onto its own layer, and one player's Zork I with a maze layer and a
//! Cellar layer. Nothing here needs a story file.
//!
//! The route itself is `mapper::path::route`'s job and is unit-tested there. What these tests pin
//! is the PRESENTATION: which cells light up, what happens to the steps that fall outside the shown
//! layer, what is said when there is no route, and how Esc walks back out.
//!
//! Every colour assertion runs in BOTH `honor_game_colours` modes, per CLAUDE.md.

use mapper::direction::Direction;
use mapper::graph::{MapGraph, RoomId};
use mapper::layer::{LayerId, MapView};
use mapper::mapper::Mapper;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use app::input::{apply_action, key_to_action, Action};
use app::render::map::render_map_layered;
use app::state::{AppState, RoomDockView};

/// Colossal Cave: layer 1 is the peeled maze, and the save was taken standing in Maze 11.
const ADVENT_MAZE: LayerId = 1;
const ADVENT_MAIN: LayerId = 0;
/// Zork I: layer 2 is the maze, layer 6 the Cellar region around it.
const ZORK_CELLAR: LayerId = 6;

/// Wide enough for the full form, tall enough for a twenty-row table plus footnotes.
const WIDE: Rect = Rect { x: 0, y: 0, width: 110, height: 40 };

fn fixture(name: &str) -> Mapper {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../unit_tests/").join(name);
    let json = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("fixture {} must be readable: {e}", path.display()));
    mapper::persist::from_json(&json).expect("the fixture is a valid map file")
}

/// A mapper + state showing `layer` as a MATRIX, with `honor_game_colours` pinned.
fn matrix_state(name: &str, layer: LayerId, honor: bool) -> (Mapper, AppState) {
    let mut m = fixture(name);
    m.graph.set_layer_view(layer, Some(MapView::Matrix));
    let mut st = AppState::default();
    st.config.honor_game_colours = honor;
    st.set_viewed_layer(Some(layer));
    assert!(st.map_shows_matrix(&m.graph), "the pane must be drawing the table, or nothing here holds");
    (m, st)
}

fn draw(g: &MapGraph, st: &AppState, area: Rect) -> Buffer {
    let rm = mapper::render::render_layer(g, st.active_layer(g));
    let mut buf = Buffer::empty(area);
    render_map_layered(&rm, g, st, area, &mut buf);
    buf
}

/// Room id by matrix row label, e.g. `"Maze 11"`.
fn id_of(g: &MapGraph, layer: LayerId, label: &str) -> RoomId {
    mapper::matrix::labels(g, layer)
        .row
        .into_iter()
        .find(|(_, l)| l == label)
        .unwrap_or_else(|| panic!("no room labelled {label:?} in layer {layer}"))
        .0
}

/// The screen rect of one matrix cell: the row for `room`, the column for `dir`.
///
/// Recomputed from the layout rather than guessed, so a change to column widths moves the probe
/// with the table instead of silently sampling the wrong cell.
fn cell_rect(g: &MapGraph, layer: LayerId, area: Rect, room: RoomId, dir: Direction) -> Rect {
    use app::render::matrix::layout;
    let ml = layout(g, layer, area.width);
    let row = ml.matrix.index_of(room).unwrap_or_else(|| panic!("room {room} has no row here"));
    let col = mapper::matrix::MATRIX_DIRS
        .iter()
        .position(|&d| d == dir)
        .unwrap_or_else(|| panic!("{dir:?} has no column"));
    let y = area.y + 2 + row as u16;
    // `ml.label_w`, not the `LABEL_W` floor (SQ-1247): the label column's ACTUAL drawn width can
    // grow past the floor when the pane has room to spare, and this probe has to land on the same
    // cell `render_matrix` drew.
    let x = area.x + ml.label_w + ml.cell_w * col as u16;
    Rect::new(x, y, ml.cell_w, 1)
}

/// The styles of the non-blank glyphs inside a rect — what a cell is actually wearing.
fn styles_in(buf: &Buffer, r: Rect) -> Vec<Style> {
    (r.x..r.right())
        .filter_map(|x| buf.cell((x, r.y)))
        .filter(|c| c.symbol() != " ")
        .map(|c| c.style())
        .collect()
}

/// Is `worn` (read back off the buffer) the theme style `want`?
///
/// Not `==`: the buffer's cells carry the pane's own background and underline colour merged in, so
/// a resolved registry style never compares equal to what was actually drawn. The colour and the
/// emphasis are what the selector decides and what the reader sees, so those are what is checked —
/// and `the_path_style_is_its_own_thing_in_both_colour_modes` pins that no two matrix selectors
/// agree on both, which is what makes this test sound.
fn wears(worn: Style, want: Style) -> bool {
    worn.fg == want.fg && worn.add_modifier.contains(want.add_modifier)
}

/// The same `(room, direction)` cells, in the order the TABLE lays them out: row order, then
/// column order. [`highlighted`] reads the screen and so comes back in that order rather than in
/// walking order, and comparing the two needs both speaking the same language.
fn in_table_order(
    g: &MapGraph,
    layer: LayerId,
    mut cells: Vec<(RoomId, Direction)>,
) -> Vec<(RoomId, Direction)> {
    let m = mapper::matrix::build(g, layer);
    cells.sort_by_key(|&(room, dir)| {
        let col = mapper::matrix::MATRIX_DIRS.iter().position(|&d| d == dir);
        (m.index_of(room).unwrap_or(usize::MAX), col.unwrap_or(usize::MAX))
    });
    cells
}

/// Every cell of the table wearing the path style — the highlight, read back off the screen as
/// `(room, direction)` pairs, which is exactly the "leave-by cell" the design specifies.
fn highlighted(
    g: &MapGraph,
    st: &AppState,
    layer: LayerId,
    area: Rect,
) -> Vec<(RoomId, Direction)> {
    let want = st.colors.theme.get("map.matrix.cell:path").style;
    let buf = draw(g, st, area);
    let m = mapper::matrix::build(g, layer);
    let mut out = Vec::new();
    for row in &m.rows {
        for dir in mapper::matrix::MATRIX_DIRS {
            let r = cell_rect(g, layer, area, row.room, dir);
            if !r.intersects(area) {
                continue;
            }
            if styles_in(&buf, r).iter().any(|&s| wears(s, want)) {
                out.push((row.room, dir));
            }
        }
    }
    out
}

/// The style must be distinguishable from everything else the table wears, or a test that finds it
/// proves nothing. Pinned in both colour modes, since a scheme could collapse two roles into one.
#[test]
fn the_path_style_is_its_own_thing_in_both_colour_modes() {
    for honor in [true, false] {
        let (_, st) = matrix_state("advent_maze_map.json", ADVENT_MAZE, honor);
        let path = st.colors.theme.get("map.matrix.cell:path").style;
        for other in [
            "map.matrix.cell:entrance",
            "map.matrix.cell:frontier",
            "map.matrix.row:selected",
            "map.matrix.row:here",
            "map.room",
        ] {
            let other_style = st.colors.theme.get(other).style;
            assert_ne!(
                path, other_style,
                "`map.matrix.cell:path` must not look like `{other}` (honor={honor})"
            );
            // Stronger, and the exact condition `wears` needs to be a sound probe: nothing else
            // the table draws can be mistaken for the route by colour + emphasis alone.
            assert!(
                !wears(other_style, path),
                "`{other}` would be read back as a route cell (honor={honor})"
            );
        }
    }
}

/// The headline behaviour, on real data: standing in Maze 11 of Colossal Cave, click the Dead End
/// and the three cells you would leave by light up — S out of Maze 11, E out of Maze 7, S out of
/// Maze 4. One cell per step, on the row of the room you are IN, in the column you go OUT by.
#[test]
fn clicking_a_room_marks_the_leave_by_cell_of_every_step_in_both_colour_modes() {
    for honor in [true, false] {
        let (mut m, mut st) = matrix_state("advent_maze_map.json", ADVENT_MAZE, honor);
        let here = m.graph.current().expect("the save was taken somewhere");
        assert_eq!(here, id_of(&m.graph, ADVENT_MAZE, "Maze 11"));
        let dead_end = id_of(&m.graph, ADVENT_MAZE, "Dead End, near Vending Machine");
        let (maze7, maze4) =
            (id_of(&m.graph, ADVENT_MAZE, "Maze 7"), id_of(&m.graph, ADVENT_MAZE, "Maze 4"));

        assert!(highlighted(&m.graph, &st, ADVENT_MAZE, WIDE).is_empty(), "nothing yet (honor={honor})");

        apply_action(Action::PinRoomDock(dead_end, RoomDockView::Info), &mut st, &mut m);

        assert_eq!(
            st.room_path.iter().map(|s| (s.room, s.dir)).collect::<Vec<_>>(),
            vec![(here, Direction::S), (maze7, Direction::E), (maze4, Direction::S)],
            "the route walked is S, E, S (honor={honor})"
        );
        assert_eq!(
            highlighted(&m.graph, &st, ADVENT_MAZE, WIDE),
            in_table_order(
                &m.graph,
                ADVENT_MAZE,
                vec![(here, Direction::S), (maze7, Direction::E), (maze4, Direction::S)]
            ),
            "…and those exact three cells are the ones marked, and no others (honor={honor})"
        );
        // The DESTINATION's own row is not marked: you arrive there, you do not leave by it.
        assert!(
            highlighted(&m.graph, &st, ADVENT_MAZE, WIDE).iter().all(|&(r, _)| r != dead_end),
            "the target room's row carries no step (honor={honor})"
        );
        assert_eq!(st.selected_room, Some(dead_end), "and the click still selects, as it always did");
    }
}

/// Style, never a glyph. The highlight must not change one character of the table — the cell's
/// text is the only thing saying what KIND of passage each step is, and a step number written over
/// it would cost more than it bought.
#[test]
fn the_highlight_never_rewrites_a_cell() {
    let (mut m, mut st) = matrix_state("advent_maze_map.json", ADVENT_MAZE, true);
    let plain = draw(&m.graph, &st, WIDE);
    let dead_end = id_of(&m.graph, ADVENT_MAZE, "Dead End, near Vending Machine");
    apply_action(Action::PinRoomDock(dead_end, RoomDockView::Info), &mut st, &mut m);
    assert!(!st.room_path.is_empty(), "there is a route to mark");

    let marked = draw(&m.graph, &st, WIDE);
    let text = |b: &Buffer| {
        (0..WIDE.height)
            .map(|y| {
                (0..WIDE.width)
                    .map(|x| b.cell((x, y)).map(|c| c.symbol()).unwrap_or(" ").to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(text(&plain), text(&marked), "the route is a style, not a redraw");
}

/// A step of the route beats the entrance bolding on the same cell. The LAST step necessarily
/// arrives at the selected room, so it is always an entrance too; the answer the player just asked
/// for wins the cell.
#[test]
fn the_last_step_wears_the_path_style_not_the_entrance_style() {
    for honor in [true, false] {
        let (mut m, mut st) = matrix_state("advent_maze_map.json", ADVENT_MAZE, honor);
        // Maze 4 is the room to ask about: six different cells arrive at it, so there is plenty of
        // entrance bolding for the one route step to have to beat.
        let maze4 = id_of(&m.graph, ADVENT_MAZE, "Maze 4");
        let maze7 = id_of(&m.graph, ADVENT_MAZE, "Maze 7");
        apply_action(Action::PinRoomDock(maze4, RoomDockView::Info), &mut st, &mut m);

        // The final step, Maze 7 —E→ Maze 4, IS one of the selected room's entrances.
        assert_eq!(st.room_path.last().map(|s| (s.room, s.dir)), Some((maze7, Direction::E)));
        assert!(
            mapper::matrix::entrances(&m.graph, maze4).contains(&(maze7, Direction::E)),
            "the premise: that cell is an entrance too (honor={honor})"
        );
        let buf = draw(&m.graph, &st, WIDE);
        let r = cell_rect(&m.graph, ADVENT_MAZE, WIDE, maze7, Direction::E);
        let worn = styles_in(&buf, r);
        assert!(
            worn.iter().all(|&s| wears(s, st.colors.theme.get("map.matrix.cell:path").style)),
            "the route wins the shared cell (honor={honor}): {worn:?}"
        );
        // Entrances that are NOT on the route keep their bolding — the two facts coexist.
        let other = mapper::matrix::entrances(&m.graph, maze4)
            .into_iter()
            .find(|&(r, d)| (r, d) != (maze7, Direction::E) && m.graph.layer_of(r) == ADVENT_MAZE)
            .expect("Maze 4 has more than one way in");
        let r = cell_rect(&m.graph, ADVENT_MAZE, WIDE, other.0, other.1);
        assert!(
            styles_in(&buf, r)
                .iter()
                .any(|&s| wears(s, st.colors.theme.get("map.matrix.cell:entrance").style)),
            "an entrance off the route is still bold (honor={honor})"
        );
    }
}

/// Layers group rooms for READING; they are not a wall the search has to respect. The route is
/// found across the whole graph, and the table draws the steps that have a row here and silently
/// drops the ones that do not — no layer switch, no half-answer.
///
/// Zork I, Cellar layer on screen, player standing deep in the maze on another layer: the way to
/// the Gallery runs Dead End 1 → Maze 2 → Maze 1 (all on the maze layer, no rows here), then Troll
/// Room → Cellar → East of Chasm → Gallery, of which three steps have rows.
#[test]
fn a_route_found_across_layers_draws_only_the_steps_with_rows_here() {
    for honor in [true, false] {
        let (mut m, mut st) = matrix_state("zork1_underground_map.json", ZORK_CELLAR, honor);
        m.graph.set_current(id_of(&m.graph, 2, "Dead End 1"));
        let gallery = id_of(&m.graph, ZORK_CELLAR, "Gallery");
        let before = st.active_layer(&m.graph);

        apply_action(Action::PinRoomDock(gallery, RoomDockView::Info), &mut st, &mut m);

        let in_layer: Vec<(RoomId, Direction)> = st
            .room_path
            .iter()
            .filter(|s| m.graph.layer_of(s.room) == ZORK_CELLAR)
            .map(|s| (s.room, s.dir))
            .collect();
        assert!(
            st.room_path.len() > in_layer.len(),
            "the premise: the route starts outside this layer (honor={honor}): {:?}",
            st.room_path
        );
        assert_eq!(
            in_layer,
            vec![
                (id_of(&m.graph, ZORK_CELLAR, "The Troll Room"), Direction::S),
                (id_of(&m.graph, ZORK_CELLAR, "Cellar"), Direction::S),
                (id_of(&m.graph, ZORK_CELLAR, "East of Chasm"), Direction::E),
            ],
            "the Cellar-layer tail of the route (honor={honor})"
        );
        assert_eq!(
            highlighted(&m.graph, &st, ZORK_CELLAR, WIDE),
            in_table_order(&m.graph, ZORK_CELLAR, in_layer),
            "exactly the steps with a row here are drawn, and nothing is invented for the rest"
        );
        assert_eq!(st.active_layer(&m.graph), before, "and the view does not jump layers (honor={honor})");
    }
}

/// Where the route walks OUT of the layer on screen, the cell it leaves by is an `⇱out` cell —
/// which already exists, already footnotes where it goes, and is exactly the right thing to mark.
///
/// Colossal Cave, maze layer on screen, standing in Maze 11: the one way out of the maze is `D`,
/// down to "At West End of Long Hall" on the Main layer. (A room with no row here is reached from
/// the drawn map or by the dock following the player, not by a click on this table — the point is
/// that the departure is still marked when it happens.)
#[test]
fn a_route_leaving_the_layer_marks_the_out_of_layer_cell_in_both_colour_modes() {
    for honor in [true, false] {
        let (mut m, mut st) = matrix_state("advent_maze_map.json", ADVENT_MAZE, honor);
        let here = m.graph.current().expect("the save was taken somewhere");
        let hall = id_of(&m.graph, ADVENT_MAIN, "At West End of Long Hall");

        assert_eq!(
            mapper::matrix::classify(&m.graph, here, Direction::Down),
            mapper::matrix::MatrixCell::LeavesLayer { dest: hall },
            "the premise: D out of Maze 11 is the `⇱out` cell (honor={honor})"
        );

        apply_action(Action::PinRoomDock(hall, RoomDockView::Info), &mut st, &mut m);
        assert_eq!(
            st.room_path.iter().map(|s| (s.room, s.dir)).collect::<Vec<_>>(),
            vec![(here, Direction::Down)]
        );
        assert_eq!(
            highlighted(&m.graph, &st, ADVENT_MAZE, WIDE),
            vec![(here, Direction::Down)],
            "the departure is marked on the `⇱out` cell (honor={honor})"
        );
        // …and the cell still says `⇱out`, with the footnote naming where it goes.
        let buf = draw(&m.graph, &st, WIDE);
        let r = cell_rect(&m.graph, ADVENT_MAZE, WIDE, here, Direction::Down);
        let text: String =
            (r.x..r.right()).filter_map(|x| buf.cell((x, r.y))).map(|c| c.symbol()).collect();
        assert!(text.contains('⇱'), "the glyph is untouched (honor={honor}): {text:?}");
    }
}

/// No route: select the room as normal and SAY so. Falling silent reads as a broken click, and a
/// partial route to somewhere nearer answers a question nobody asked.
#[test]
fn a_room_with_no_known_route_selects_and_says_so() {
    // Main layer on screen as a table, player still down in the peeled maze: the fixture's map has
    // no walked way from the maze to the surface buildings at all.
    let (mut m, mut st) = matrix_state("advent_maze_map.json", ADVENT_MAIN, true);
    let here = m.graph.current().expect("the save was taken somewhere");
    let building = id_of(&m.graph, ADVENT_MAIN, "Inside Building");
    assert_eq!(mapper::path::route(&m.graph, here, building), None, "the premise: no known way");

    apply_action(Action::PinRoomDock(building, RoomDockView::Info), &mut st, &mut m);

    assert_eq!(st.selected_room, Some(building), "the room is selected exactly as usual");
    assert!(st.room_dock.open, "and the dock still opens on it");
    assert!(st.room_path.is_empty(), "there is no route to draw");
    assert!(highlighted(&m.graph, &st, ADVENT_MAIN, WIDE).is_empty(), "so nothing is highlighted");
    assert!(
        st.notifications.history().iter().any(|n| n == "no known route from here"),
        "the refusal is spoken aloud: {:?}",
        st.notifications.history()
    );

    // A room that IS reachable says nothing — the message is a refusal, not a running commentary.
    let hall = id_of(&m.graph, ADVENT_MAIN, "At West End of Long Hall");
    let before = st.notifications.history().len();
    apply_action(Action::PinRoomDock(hall, RoomDockView::Info), &mut st, &mut m);
    assert!(!st.room_path.is_empty(), "that one has a route");
    assert_eq!(st.notifications.history().len(), before, "and nothing new is said");
}

/// Clicking the room you are already standing in is "you are already there", not "no route": the
/// empty route draws nothing and says nothing.
#[test]
fn clicking_the_room_you_are_standing_in_is_not_a_refusal() {
    let (mut m, mut st) = matrix_state("advent_maze_map.json", ADVENT_MAZE, true);
    let here = m.graph.current().expect("the save was taken somewhere");
    apply_action(Action::PinRoomDock(here, RoomDockView::Info), &mut st, &mut m);
    assert!(st.room_path.is_empty());
    assert!(
        !st.notifications.history().iter().any(|n| n == "no known route from here"),
        "you are standing in it: {:?}",
        st.notifications.history()
    );
}

/// Esc walks back out the way you came in: the route first, then the pin, then the dock — the same
/// ladder the room dock already had, with one rung added ahead of it.
#[test]
fn esc_clears_the_route_first_and_the_selection_second() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);

    let (mut m, mut st) = matrix_state("advent_maze_map.json", ADVENT_MAZE, true);
    let dead_end = id_of(&m.graph, ADVENT_MAZE, "Dead End, near Vending Machine");
    apply_action(Action::PinRoomDock(dead_end, RoomDockView::Info), &mut st, &mut m);
    assert!(!st.room_path.is_empty() && st.selected_room == Some(dead_end));

    // Rung 1: the route goes, the selection and its entrance bolding stay.
    assert!(matches!(key_to_action(&st, esc), Action::ClearRoomPath));
    apply_action(key_to_action(&st, esc), &mut st, &mut m);
    assert!(st.room_path.is_empty(), "the route is cleared");
    assert_eq!(st.selected_room, Some(dead_end), "…but the room is still selected");
    assert!(st.room_dock.open, "…and the dock is still up");
    let buf = draw(&m.graph, &st, WIDE);
    let maze4 = id_of(&m.graph, ADVENT_MAZE, "Maze 4");
    let r = cell_rect(&m.graph, ADVENT_MAZE, WIDE, maze4, Direction::S);
    assert!(
        styles_in(&buf, r)
            .iter()
            .any(|&s| wears(s, st.colors.theme.get("map.matrix.cell:entrance").style)),
        "the entrance bolding survives the first Esc"
    );

    // Rung 2: the selection goes.
    assert!(matches!(key_to_action(&st, esc), Action::UnpinRoomDock));
    apply_action(key_to_action(&st, esc), &mut st, &mut m);
    assert_eq!(st.selected_room, None, "the second Esc unpins");
    assert!(st.room_dock.open, "…and still leaves the dock up");

    // Rung 3: unchanged from before — the dock closes.
    assert!(matches!(key_to_action(&st, esc), Action::CloseRoomDock));
}

/// Unpinning by any other route — a click on empty map space, a second click on the pinned room —
/// takes the route with it. A highlight describing a selection that no longer exists is a lie.
#[test]
fn unpinning_drops_the_route_too() {
    let (mut m, mut st) = matrix_state("advent_maze_map.json", ADVENT_MAZE, true);
    let dead_end = id_of(&m.graph, ADVENT_MAZE, "Dead End, near Vending Machine");
    apply_action(Action::PinRoomDock(dead_end, RoomDockView::Info), &mut st, &mut m);
    assert!(!st.room_path.is_empty());
    apply_action(Action::UnpinRoomDock, &mut st, &mut m);
    assert!(st.room_path.is_empty(), "unpinning clears the route");
    assert!(highlighted(&m.graph, &st, ADVENT_MAZE, WIDE).is_empty());
}

/// Arrow-key row nav moves the SELECTION, so it drops a route that described the row you just
/// stepped off. Recomputing per keypress instead would fire the refusal toast for every
/// unreachable row the selection merely passed over.
#[test]
fn stepping_the_selection_with_the_keyboard_drops_the_route() {
    let (mut m, mut st) = matrix_state("advent_maze_map.json", ADVENT_MAZE, true);
    let dead_end = id_of(&m.graph, ADVENT_MAZE, "Dead End, near Vending Machine");
    apply_action(Action::PinRoomDock(dead_end, RoomDockView::Info), &mut st, &mut m);
    assert!(!st.room_path.is_empty());

    apply_action(Action::MatrixMove(-1), &mut st, &mut m);
    assert_ne!(st.selected_room, Some(dead_end), "the selection moved");
    assert!(st.room_path.is_empty(), "so the old route went with it");
    assert!(
        !st.notifications.history().iter().any(|n| n == "no known route from here"),
        "and nothing was said about the new row: {:?}",
        st.notifications.history()
    );
}

/// The drawn view has no leave-by cell to mark, so a click there computes no route and — crucially
/// — raises no refusal toast for a room the player was only pointing at.
#[test]
fn a_click_in_the_drawn_view_neither_routes_nor_complains() {
    let mut m = fixture("advent_maze_map.json");
    let mut st = AppState::default();
    st.set_viewed_layer(Some(ADVENT_MAIN)); // Drawn: the fixture ships no Matrix view mode
    assert!(!st.map_shows_matrix(&m.graph));

    let building = id_of(&m.graph, ADVENT_MAIN, "Inside Building");
    apply_action(Action::PinRoomDock(building, RoomDockView::Info), &mut st, &mut m);
    assert_eq!(st.selected_room, Some(building), "the click still pins");
    assert!(st.room_path.is_empty());
    assert!(st.notifications.history().is_empty(), "{:?}", st.notifications.history());
}
