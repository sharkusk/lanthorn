//! The room dock rendered end to end, against real player data (SQ-0692).
//!
//! `unit_tests/advent_maze_map.json` is a verbatim copy of the `map.json` from a lanthorn
//! archive: one player's partial mapping of Colossal Cave. Driving the dock from a real graph is
//! what makes "does the header name the room the way the matrix does" a meaningful question.
//!
//! Every colour assertion runs in BOTH `honor_game_colours` modes, per CLAUDE.md: the dock is app
//! chrome, so the game's palette must never reach it — and a single-mode suite could not show it.

use mapper::graph::{MapGraph, RoomId};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use app::layout::compute_pane_layout;
use app::render::room_dock::{dock_room, draw_room_dock};
use app::state::{AppState, RoomDockView};

use crate::fixture_paths::fixture_path;

const FRAME: Rect = Rect { x: 0, y: 0, width: 120, height: 40 };

fn advent() -> mapper::mapper::Mapper {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../unit_tests/advent_maze_map.json");
    let json = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("fixture {} must be readable: {e}", path.display()));
    mapper::persist::from_json(&json).expect("the fixture is a valid map file")
}

fn id_of(g: &MapGraph, label: &str) -> RoomId {
    g.rooms()
        .find(|r| r.label() == label)
        .map(|r| r.id)
        .unwrap_or_else(|| panic!("no room labelled {label:?}"))
}

/// A state with the dock open (instantly, no slide), `honor_game_colours` pinned.
fn dock_state(honor: bool, view: RoomDockView) -> AppState {
    let mut st = AppState::default();
    st.config.honor_game_colours = honor;
    st.room_dock.toggle_to(true, true);
    st.room_dock_view = view;
    st
}

/// Render the dock exactly where the frame layout puts it, and return the buffer
/// plus the dock's rect.
fn draw(g: &MapGraph, st: &AppState) -> (Buffer, Rect) {
    let pl = compute_pane_layout(FRAME, st, 0);
    assert!(pl.room_dock.height > 0, "the layout must reserve dock rows for this test");
    let mut buf = Buffer::empty(FRAME);
    let room = dock_room(st.selected_room, g);
    draw_room_dock(
        g,
        room,
        st.room_dock_pinned(),
        st.room_dock_view,
        &[],
        g.current(),
        pl.room_dock,
        &st.colors,
        &st.symbols,
        false,
        &mut buf,
    );
    (buf, pl.room_dock)
}

fn text_in(buf: &Buffer, r: Rect) -> String {
    (r.y..r.bottom())
        .map(|y| {
            (r.x..r.right())
                .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The dock FOLLOWS by default: with nothing pinned it describes the room the player is standing
/// in, and it changes when the player moves — no click, no command, no state of its own. This is
/// the property neither floating popup had: they described the room you last clicked, forever.
#[test]
fn the_dock_follows_the_player_across_a_move() {
    for honor in [true, false] {
        let mut m = advent();
        let st = dock_state(honor, RoomDockView::Info);

        let start = id_of(&m.graph, "Inside Building");
        let next = id_of(&m.graph, "In Cobble Crawl");
        m.graph.set_current(start);

        let (buf, r) = draw(&m.graph, &st);
        let before = text_in(&buf, r);
        assert!(before.contains("Inside Building"), "honor={honor}: {before}");
        assert!(before.contains("following"), "…and says it is following: {before}");

        // The player walks. Nothing about the dock is touched.
        m.graph.set_current(next);
        let (buf, r) = draw(&m.graph, &st);
        let after = text_in(&buf, r);
        assert!(after.contains("In Cobble Crawl"), "honor={honor}: the dock moved with the player: {after}");
        assert!(!after.contains("Inside Building"), "…and left the old room behind: {after}");
        assert!(after.contains("following"), "still following: {after}");
    }
}

/// Pinned, the dock stays on the selected room while the player walks away, and the header says
/// so. Pin state IS `selected_room`, so this is the same fact the map highlight draws.
#[test]
fn a_pinned_dock_holds_its_room_while_the_player_walks_away() {
    for honor in [true, false] {
        let mut m = advent();
        let mut st = dock_state(honor, RoomDockView::Info);

        let pinned = id_of(&m.graph, "Inside Building");
        let elsewhere = id_of(&m.graph, "In Cobble Crawl");
        m.graph.set_current(pinned);
        st.selected_room = Some(pinned);

        m.graph.set_current(elsewhere);
        let (buf, r) = draw(&m.graph, &st);
        let text = text_in(&buf, r);
        assert!(text.contains("Inside Building"), "honor={honor}: the dock held its pin: {text}");
        assert!(text.contains("pinned"), "…and says so: {text}");
        assert!(!text.contains("following"), "{text}");

        // Unpinning (the click/Esc gesture, at state level) hands it back to the player.
        st.selected_room = None;
        let (buf, r) = draw(&m.graph, &st);
        let text = text_in(&buf, r);
        assert!(text.contains("In Cobble Crawl"), "honor={honor}: unpinned, it follows again: {text}");
        assert!(text.contains("following"), "{text}");
    }
}

/// One dock, two bodies. Flipping the view redraws the SAME room with different content — it does
/// not open a second panel, and it does not change which room is described.
#[test]
fn flipping_the_view_redraws_the_same_room_with_the_other_body() {
    for honor in [true, false] {
        let mut m = advent();
        let here = id_of(&m.graph, "Inside Building");
        m.graph.set_current(here);

        let info = dock_state(honor, RoomDockView::Info);
        let (buf, r) = draw(&m.graph, &info);
        let info_text = text_in(&buf, r);
        assert!(info_text.contains("Exits:"), "honor={honor}: Info draws the exit card: {info_text}");

        let diag = dock_state(honor, RoomDockView::Diagnostics);
        let (buf, r) = draw(&m.graph, &diag);
        let diag_text = text_in(&buf, r);
        assert!(diag_text.contains("Pos "), "honor={honor}: Diagnostics draws the grid position: {diag_text}");
        assert!(!diag_text.contains("Exits:"), "…and not the exit card: {diag_text}");

        // Same room, both times.
        assert!(info_text.contains("Inside Building") && diag_text.contains("Inside Building"));
    }
}

/// The dock's own style selectors reach the screen in both colour modes: the game's palette has
/// no say over app chrome, so an override must land identically either way.
#[test]
fn the_dock_selectors_apply_in_both_colour_modes() {
    let parsed = app::theme::toml_schema::parse(
        "[elements]\nroom_panel = { fg = \"magenta\" }\n\
         \"room_panel.header\" = { fg = \"blue\" }\n\
         \"room_panel.header:pinned\" = { fg = \"green\" }\n",
    )
    .expect("the override parses");

    for honor in [true, false] {
        let mut m = advent();
        let here = id_of(&m.graph, "Inside Building");
        m.graph.set_current(here);

        let mut st = dock_state(honor, RoomDockView::Info);
        st.colors.theme = app::theme::resolve::resolve_theme(
            &app::colors::GhosttyScheme::default(),
            &parsed,
        );

        let fgs = |buf: &Buffer, r: Rect| -> Vec<Option<Color>> {
            (r.y..r.bottom())
                .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
                .map(|(x, y)| buf.cell((x, y)).and_then(|c| c.style().fg))
                .collect()
        };

        let (buf, r) = draw(&m.graph, &st);
        let following = fgs(&buf, r);
        assert!(following.contains(&Some(Color::Blue)), "honor={honor}: room_panel.header applies");
        assert!(following.contains(&Some(Color::Magenta)), "honor={honor}: room_panel applies to the body");
        assert!(!following.contains(&Some(Color::Green)), "honor={honor}: not the pinned variant");

        st.selected_room = Some(here);
        let (buf, r) = draw(&m.graph, &st);
        assert!(
            fgs(&buf, r).contains(&Some(Color::Green)),
            "honor={honor}: room_panel.header:pinned applies once pinned"
        );
    }
}

/// The dock docks below the map WHATEVER the layer draws as — including the matrix table, which
/// has its own full-pane geometry. The map pane simply gets fewer rows.
#[test]
fn the_dock_docks_below_the_matrix_view_too() {
    use mapper::layer::MapView;

    let m = advent();
    let mut st = AppState::default();
    st.room_dock.toggle_to(true, true);
    st.set_viewed_layer(Some(1));

    let closed = {
        let mut s = AppState::default();
        s.set_viewed_layer(Some(1));
        compute_pane_layout(FRAME, &s, 0)
    };
    let open = compute_pane_layout(FRAME, &st, 0);

    assert!(open.room_dock.height > 0);
    assert_eq!(open.room_dock.y, open.map.bottom(), "directly under the map pane");
    assert_eq!(
        open.map.height + open.room_dock.height,
        closed.map.height,
        "the rows come out of the map pane, whatever it is drawing"
    );

    // And the matrix simply renders into the shorter pane.
    let mut m2 = m;
    m2.graph.set_layer_view(1, Some(MapView::Matrix));
    let rm = mapper::render::render_layer(&m2.graph, 1);
    let mut buf = Buffer::empty(FRAME);
    let hits = app::render::map::render_map_layered(&rm, &m2.graph, &st, open.map, &mut buf);
    assert!(!hits.room_rects.is_empty(), "the matrix still publishes click targets in the shortened pane");
    for (_, r) in &hits.room_rects {
        assert!(r.bottom() <= open.room_dock.y, "nothing the matrix draws reaches into the dock");
    }
}

/// SQ-0694: the whole Info body — header, objects, the twelve-direction card — fits inside the
/// dock at its SHIPPED default height, at the map width a split-pane layout actually gives it.
///
/// This is the test that sets `room_dock_pct`'s default. The card used to be a fixed thirteen
/// rows, which no default a 40-row terminal could spare would admit; spending columns instead
/// brings the natural body down to about nine, so the default went back to the inventory dock's
/// 33 rather than the 40 it needed before.
#[test]
fn the_whole_info_body_fits_at_the_default_dock_height() {
    let mut m = advent();
    let here = id_of(&m.graph, "Inside Building");
    m.graph.set_current(here);

    let st = dock_state(true, RoomDockView::Info);
    assert_eq!(
        st.pane_sizes.room_dock_pct,
        app::config::Config::default().room_dock_pct,
        "this test is about the DEFAULT height"
    );

    let pl = compute_pane_layout(FRAME, &st, 0);
    let mut buf = Buffer::empty(FRAME);
    let objects = ["a brass lantern".to_string(), "a small mat".to_string()];
    draw_room_dock(
        &m.graph,
        Some(here),
        false,
        RoomDockView::Info,
        &objects,
        Some(here),
        pl.room_dock,
        &st.colors,
        &st.symbols,
        false,
        &mut buf,
    );
    let text = text_in(&buf, pl.room_dock);

    assert!(text.contains("Inside Building"), "the header: {text}");
    assert!(text.contains("Here:") && text.contains("a brass lantern"), "the objects: {text}");
    assert!(text.contains("Exits:"), "the card's label: {text}");
    for d in ["N ", "S ", "E ", "W ", "NE", "NW", "SE", "SW", "Up", "Dn", "In", "Out"] {
        assert!(text.contains(d), "every direction is on screen at the default height; {d} is not:\n{text}");
    }

    // …and it is a GRID, not twelve rows: three directions from three different groups share one.
    assert!(
        text.lines().any(|l| l.contains("N ") && l.contains("NE") && l.contains("Up")),
        "the card lays out three across at this width:\n{text}"
    );

    // Nothing spilled past the dock: the body ends inside its own rows.
    let used = text.lines().filter(|l| !l.trim_matches(['│', ' ']).is_empty()).count();
    assert!(used <= pl.room_dock.height as usize, "the body fits its rect");
}

// ── Real-game smoke (gitignored fixture; skips vacuously) ────────────────────

/// The Info body's object list is LIVE, read from the running engine's object tree — not a
/// transcript scrape and not the map's own record, which knows nothing about what is lying on the
/// floor. Only a booted story can show that, so this drives one.
///
/// It also pins the gating rule the dock inherited: objects are shown for the CURRENT room only.
/// The engine cannot introspect a room the player is not standing in beyond what the map recorded,
/// so a pinned dock pointed elsewhere must not invent contents for it.
#[test]
fn the_info_body_lists_the_current_rooms_objects_from_a_real_engine() {
    use app::engine::Engine;
    use app::session::GameSession;

    let path = fixture_path("minizork-r34-s871124.z3");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return;
    };
    let mut session =
        GameSession::new_with_trace(bytes, true, false, None, false, Default::default(), None, None, None)
            .expect("story should load and boot");
    let _ = session.take_transcript();
    session.submit("look");

    let loc = session.current_location().expect("the starting room is detected at boot");
    let here = loc.number;
    let objects = session
        .introspect()
        .expect("a Z-machine story has an object tree")
        .room_objects(here)
        .iter()
        .filter_map(|o| o.display_name())
        .collect::<Vec<String>>();
    assert!(!objects.is_empty(), "the opening room has objects in it");

    // Map the room the way the app does, then draw the dock over it.
    let mut m = mapper::mapper::Mapper::default();
    m.observe(here, &loc.name, None);
    let st = dock_state(true, RoomDockView::Info);

    let pl = compute_pane_layout(FRAME, &st, 0);
    let mut buf = Buffer::empty(FRAME);
    draw_room_dock(
        &m.graph,
        dock_room(st.selected_room, &m.graph),
        false,
        RoomDockView::Info,
        &objects,
        Some(here),
        pl.room_dock,
        &st.colors,
        &st.symbols,
        false,
        &mut buf,
    );
    let text = text_in(&buf, pl.room_dock);
    assert!(text.contains("Here:"), "the current room gets an objects section: {text}");
    assert!(
        objects.iter().any(|o| text.contains(o.as_str())),
        "…listing what the engine can actually see: {objects:?}\n{text}"
    );

    // Pinned to a DIFFERENT room, the same objects must not be attributed to it.
    let elsewhere = here.wrapping_add(1);
    m.observe(elsewhere, "Somewhere Else", Some(mapper::direction::Direction::N));
    let mut buf = Buffer::empty(FRAME);
    draw_room_dock(
        &m.graph,
        Some(elsewhere),
        true,
        RoomDockView::Info,
        &objects,
        Some(here),
        pl.room_dock,
        &st.colors,
        &st.symbols,
        false,
        &mut buf,
    );
    let text = text_in(&buf, pl.room_dock);
    assert!(!text.contains("Here:"), "a room the player is not in gets no object list: {text}");
}
