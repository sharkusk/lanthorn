//! The matrix view rendered against REAL player data (SQ-0666).
//!
//! `unit_tests/advent_maze_map.json` is a verbatim copy of the `map.json` from a lanthorn
//! archive: one player's partial mapping of Colossal Cave, with the "all alike" maze hand-peeled
//! onto layer 1. Twelve rooms, forty-seven in-layer edges, two of them reciprocal.
//!
//! Every colour assertion runs in BOTH `honor_game_colours` modes, per CLAUDE.md: single-mode
//! render suites have masked regressions here before.

use mapper::direction::Direction;
use mapper::graph::{MapGraph, RoomId};
use mapper::layer::{LayerId, MapView};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use app::render::map::render_map_layered;
use app::state::AppState;

const MAZE: LayerId = 1;
/// Wide enough for the full form (~83 columns) with room to spare.
const WIDE: Rect = Rect { x: 0, y: 0, width: 100, height: 24 };

fn advent() -> mapper::mapper::Mapper {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../unit_tests/advent_maze_map.json");
    let json = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("fixture {} must be readable: {e}", path.display()));
    mapper::persist::from_json(&json).expect("the fixture is a valid map file")
}

/// A state showing the maze layer as a matrix, with `honor_game_colours` pinned.
fn maze_state(honor: bool) -> (mapper::mapper::Mapper, AppState) {
    let mut m = advent();
    m.graph.set_layer_maze(MAZE, true);
    let mut st = AppState::default();
    st.config.honor_game_colours = honor;
    st.set_viewed_layer(Some(MAZE));
    (m, st)
}

fn draw(g: &MapGraph, st: &AppState, area: Rect) -> (Buffer, Vec<(RoomId, Rect)>) {
    let rm = mapper::render::render_layer(g, st.active_layer(g));
    let mut buf = Buffer::empty(area);
    let hits = render_map_layered(&rm, g, st, area, &mut buf);
    (buf, hits)
}

fn lines(buf: &Buffer, area: Rect) -> Vec<String> {
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" ").to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

/// The rendered line for a row, found by its label. Matched to the END of the label so
/// `"Maze 1"` does not find `"Maze 11"`.
fn row_for(buf: &Buffer, area: Rect, label: &str) -> String {
    lines(buf, area)
        .into_iter()
        .find(|l| {
            let body = l.trim_start_matches([' ', '▸']);
            body.strip_prefix(label).is_some_and(|rest| !rest.starts_with(|c: char| c.is_ascii_digit()))
        })
        .unwrap_or_else(|| panic!("no row labelled {label:?}"))
}

/// Room id by matrix row label.
fn id_of(g: &MapGraph, label: &str) -> RoomId {
    mapper::matrix::labels(g, MAZE)
        .row
        .into_iter()
        .find(|(_, l)| l == label)
        .unwrap_or_else(|| panic!("no room labelled {label:?}"))
        .0
}

/// The cells the design document names, read off a real render of a real map.
#[test]
fn the_cells_the_spec_names_are_on_screen_in_both_colour_modes() {
    for honor in [true, false] {
        let (m, st) = maze_state(honor);
        let (buf, _) = draw(&m.graph, &st, WIDE);
        let text = lines(&buf, WIDE).join("\n");

        // All twelve direction columns, always.
        let header = &lines(&buf, WIDE)[0];
        for d in ["N", "S", "E", "W", "NE", "NW", "SE", "SW", "U", "D", "I", "O"] {
            assert!(header.contains(d), "column {d} missing from header {header:?} (honor={honor})");
        }

        // `×` — east was typed in the Dead End and there is no path that way.
        let dead_end = row_for(&buf, WIDE, "Dead End");
        let cells: Vec<&str> = dead_end.split_whitespace().skip(2).collect();
        assert_eq!(cells[2], "×", "Dead End's E cell is `×`: {dead_end:?} (honor={honor})");
        assert!(cells[0].starts_with('⇄'), "and its N cell is reciprocal: {dead_end:?}");

        // The other end of the same passage — the layer's only other reciprocal cell.
        let maze4 = row_for(&buf, WIDE, "Maze 4");
        let m4: Vec<&str> = maze4.split_whitespace().skip(2).collect();
        assert_eq!(m4[1], "⇄DE", "Maze 4's S cell returns to the Dead End: {maze4:?}");
        assert_eq!(
            text.matches('⇄').count(),
            2,
            "exactly two reciprocal cells in the whole table (honor={honor})"
        );

        // `⇱out` — down from Maze 11 leaves the layer, footnoted with where it goes.
        let maze11 = row_for(&buf, WIDE, "Maze 11");
        assert!(maze11.contains("⇱out"), "Maze 11 leaves the layer: {maze11:?}");
        assert!(
            text.contains("⇱out: D from 11 → At West End of Long Hall"),
            "…and the footnote says where to (honor={honor}):\n{text}"
        );

        // `▸` on the room the save was taken in, and nowhere else.
        assert!(maze11.starts_with('▸'), "the here-marker is on Maze 11: {maze11:?}");
        assert_eq!(text.matches('▸').count(), 1, "exactly one here-marker (honor={honor})");

        // The other two cells of the vocabulary, from Maze 1's row.
        let maze1 = row_for(&buf, WIDE, "Maze 1");
        assert!(maze1.contains("→5⇠w"), "N goes to 5 and W comes back: {maze1:?}");
        assert!(maze1.contains("⇢9"), "S is one-way to 9: {maze1:?}");
        assert!(maze1.contains('·'), "and the diagonals are untried frontier: {maze1:?}");
    }
}

/// The one door INTO the maze in the fixture: "At West End of Long Hall" leads back in via `S`,
/// landing on Maze 11 — which happens to also be the room the save was taken standing in, so this
/// doubles as the real-data proof that `▸` wins over `⇲` (see the precedence test below for the
/// synthetic case where they are on different rows).
#[test]
fn the_inbound_door_gets_a_footnote_in_both_colour_modes() {
    for honor in [true, false] {
        let (m, st) = maze_state(honor);
        let (buf, _) = draw(&m.graph, &st, WIDE);
        let text = lines(&buf, WIDE).join("\n");

        assert!(
            text.contains("⇲ in:  At West End of Long Hall —S→ Maze 11"),
            "the inbound-edge footnote is missing (honor={honor}):\n{text}"
        );

        // Maze 11 is ALSO the here-room in this save, so `▸` must win over `⇲` on its row.
        let maze11 = row_for(&buf, WIDE, "Maze 11");
        assert!(maze11.starts_with('▸'), "here wins over entrance on Maze 11's row: {maze11:?}");
        // The ONLY line beginning with `⇲` must be the footnote's own prose, never a row marker —
        // the entry room's row is `▸`, and nothing else is an entry room in this fixture.
        assert_eq!(
            text.lines().filter(|l| l.starts_with('⇲')).count(),
            1,
            "only the footnote line should start with `⇲`, no row marker (honor={honor}):\n{text}"
        );
    }
}

/// Selecting a room BOLDS every cell elsewhere that arrives at it — the answer to "how do I get
/// back here", which is the one thing a row cannot tell you about itself.
#[test]
fn selecting_a_room_bolds_its_entrances_in_both_colour_modes() {
    for honor in [true, false] {
        let (m, mut st) = maze_state(honor);
        let maze4 = id_of(&m.graph, "Maze 4");
        st.select_room(Some(maze4));
        let (buf, _) = draw(&m.graph, &st, WIDE);

        let entrance = st.colors.theme.get("map.matrix.cell:entrance").style;
        assert!(
            entrance.add_modifier.contains(Modifier::BOLD),
            "the default entrance style must be BOLD, or this test proves nothing (honor={honor})"
        );

        // Every one of Maze 4's six entrances is bold, wherever it sits.
        let mut bold_cells = 0;
        for (from, dir) in mapper::matrix::entrances(&m.graph, maze4) {
            let Some(row_i) = mapper::matrix::build(&m.graph, MAZE).index_of(from) else { continue };
            let y = WIDE.y + 2 + row_i as u16;
            let styles: Vec<Style> =
                (0..WIDE.width).filter_map(|x| buf.cell((x, y)).map(|c| c.style())).collect();
            let bold = styles.iter().filter(|s| s.add_modifier.contains(Modifier::BOLD)).count();
            assert!(bold > 0, "the {dir:?} cell of row {row_i} (into Maze 4) is not bold (honor={honor})");
            bold_cells += 1;
        }
        assert_eq!(bold_cells, 6, "all six ways into Maze 4 are marked (honor={honor})");

        // Bold is STYLE, never a glyph: the cell text is identical selected or not.
        let (plain, _) = draw(&m.graph, &maze_state(honor).1, WIDE);
        assert_eq!(
            lines(&buf, WIDE),
            lines(&plain, WIDE),
            "the cross-highlight must not change one character of the table (honor={honor})"
        );
    }
}

/// The whole point of the view mode: the same layer draws two completely different things, and
/// the choice is per-layer and survives the map file.
#[test]
fn the_view_mode_is_per_layer_and_persists() {
    let mut m = advent();
    let mut st = AppState::default();
    st.set_viewed_layer(Some(MAZE));

    // Drawn by default: room boxes, no direction table.
    let (buf, _) = draw(&m.graph, &st, WIDE);
    let drawn = lines(&buf, WIDE).join("\n");
    assert!(drawn.contains('╭') || drawn.contains('┏'), "the drawn view draws boxes");
    assert!(!drawn.contains('⇢'), "and no matrix cells");

    m.graph.set_layer_view(MAZE, Some(MapView::Matrix));
    let (buf, _) = draw(&m.graph, &st, WIDE);
    assert!(lines(&buf, WIDE).join("\n").contains('⇢'), "the matrix took over");

    // The OTHER layer is untouched — the mode is per-layer, not global.
    st.set_viewed_layer(Some(0));
    let (buf, _) = draw(&m.graph, &st, WIDE);
    let main = lines(&buf, WIDE).join("\n");
    assert!(main.contains('╭') || main.contains('┏'), "Main still draws a map:\n{main}");

    // …and both survive a save/load round trip.
    let m2 = mapper::persist::from_json(&mapper::persist::to_json(&m)).expect("round trip");
    assert_eq!(m2.graph.layer_view(MAZE), MapView::Matrix);
    assert_eq!(m2.graph.layer_view(0), MapView::Drawn);
}

/// Narrow the pane and the table gives up detail before it gives up being a table: first the
/// `⇠x` return suffixes, and only then horizontal scrolling with the label column pinned.
#[test]
fn the_table_degrades_before_it_scrolls() {
    let (m, mut st) = maze_state(true);

    let (buf, _) = draw(&m.graph, &st, WIDE);
    assert!(row_for(&buf, WIDE, "Maze 1").contains("→5⇠w"), "wide: the return is spelled out");

    // The ladder itself, not just its symptoms: at each width the table must be in the density
    // the design settled on. Asserting only on the rendered text cannot tell "compact" from
    // "scrolling with compact cells" at a width where nothing has yet been cut off.
    use app::render::matrix::Density;
    let resolved = |w: u16| app::render::matrix::layout(&m.graph, MAZE, w).density;
    assert_eq!(resolved(WIDE.width), Density::Full);
    assert_eq!(resolved(70), Density::Compact, "narrower: drop the return suffix, keep the table");
    assert_eq!(resolved(40), Density::Scroll, "narrower still: scroll, as a last resort");

    let mid = Rect { width: 70, ..WIDE };
    let (buf, _) = draw(&m.graph, &st, mid);
    let row = row_for(&buf, mid, "Maze 1");
    assert!(!row.contains('⇠'), "narrower: the return suffix is dropped: {row:?}");
    assert!(row.contains("→5"), "but the destination survives: {row:?}");
    assert!(
        lines(&buf, mid)[0].contains('O'),
        "and all twelve columns are still on screen: {:?}",
        lines(&buf, mid)[0]
    );

    // Narrower still: columns scroll, and the label column stays pinned.
    let thin = Rect { width: 40, ..WIDE };
    let (buf, _) = draw(&m.graph, &st, thin);
    assert!(row_for(&buf, thin, "Maze 1").starts_with(" Maze 1"), "the label column is pinned");
    let first = lines(&buf, thin)[0].clone();
    st.matrix_scroll.0 = 4;
    let (buf, _) = draw(&m.graph, &st, thin);
    let scrolled = lines(&buf, thin)[0].clone();
    assert_ne!(first, scrolled, "scrolling moves the columns: {first:?} vs {scrolled:?}");
    assert!(
        row_for(&buf, thin, "Maze 1").starts_with(" Maze 1"),
        "…and the label column does not move with them"
    );
}

/// A click anywhere on a row selects that room; a click on a DESTINATION cell selects the room it
/// names. Both ride the drawn view's existing `room_rects` → `ShowRoomInfo` path.
#[test]
fn rows_and_destination_cells_are_both_click_targets() {
    let (m, st) = maze_state(true);
    let (_, hits) = draw(&m.graph, &st, WIDE);

    let maze1 = id_of(&m.graph, "Maze 1");
    let maze5 = id_of(&m.graph, "Maze 5");
    let label_row = hits.iter().find(|(id, r)| *id == maze1 && r.x == WIDE.x).expect("Maze 1's label");
    assert_eq!(label_row.1.height, 1);

    // Maze 1's N cell names Maze 5, so clicking it must reach Maze 5 — on Maze 1's row.
    let y = label_row.1.y;
    assert!(
        hits.iter().any(|(id, r)| *id == maze5 && r.y == y && r.x > WIDE.x + 5),
        "the `→5⇠w` cell on Maze 1's row is a click target for Maze 5: {hits:?}"
    );

    // A `⇱out` cell names a room with no row here, so it is NOT a target: there is nothing in
    // this table to jump to.
    let out_dest = m
        .graph
        .connections()
        .iter()
        .find(|c| m.graph.layer_of(c.origin) == MAZE && m.graph.layer_of(c.dest) != MAZE)
        .map(|c| c.dest)
        .expect("the maze has an exit");
    assert!(!hits.iter().any(|(id, _)| *id == out_dest), "no jump to a room with no row");
}

/// Hovering a truncated row label shows the full room name (SQ-1246). "Dead End, near Vending
/// Machine" is `LABEL_W` cannot hold whatever the pane width, so its row is truncated and
/// footnoted on screen exactly as `matrix.rs`'s own unit test fixture is — this is the same fact
/// against a real recorded map instead of a hand-built one.
#[test]
fn hovering_a_truncated_row_label_shows_the_full_room_name() {
    let (m, mut st) = maze_state(true);
    let (mut buf, hits) = draw(&m.graph, &st, WIDE);

    let dead_end = id_of(&m.graph, "Dead End, near Vending Machine");
    let label_rect = *hits
        .iter()
        .find(|(id, r)| *id == dead_end && r.x == WIDE.x)
        .map(|(_, r)| r)
        .expect("Dead End's row label is a hit target");

    st.matrix_hover = Some((dead_end, label_rect));
    let painted = app::render::matrix::draw_hover_tip(&m.graph, MAZE, &st, WIDE, &mut buf)
        .expect("a tip was painted");
    let joined = (painted.y..painted.bottom())
        .map(|y| {
            (painted.x..painted.right())
                .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" ").to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("Dead End, near Vending Machine"),
        "the tip must spell out the full name: {joined:?}"
    );
}

/// Self-loops: recordable, and rendered as `↩` where they belong.
#[test]
fn an_observed_self_loop_shows_as_a_return_arrow() {
    let (mut m, st) = maze_state(true);
    let maze1 = id_of(&m.graph, "Maze 1");
    let before = lines(&draw(&m.graph, &st, WIDE).0, WIDE).join("\n");
    assert!(!before.contains('↩'), "nothing loops yet");

    assert!(m.graph.add_self_loop(maze1, Direction::NE));
    let after = lines(&draw(&m.graph, &st, WIDE).0, WIDE).join("\n");
    assert!(after.contains('↩'), "the loop shows in the matrix:\n{after}");
    // And it is the NE cell of Maze 1's row, not some other cell.
    let (buf, _) = draw(&m.graph, &st, WIDE);
    let row = row_for(&buf, WIDE, "Maze 1");
    let cells: Vec<&str> = row.split_whitespace().skip(2).collect();
    assert_eq!(cells[4], "↩", "NE is the fifth column: {row:?}");
}
