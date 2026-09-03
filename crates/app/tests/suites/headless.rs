//! Headless end-to-end smoke test for the `app` crate.
//!
//! Drives a scripted 3-room walk through state + render + export + persistence
//! without a real TTY, using `ratatui::backend::TestBackend` buffers.
#![allow(clippy::field_reassign_with_default)]

use app::export_svg::render_svg;
use app::persist_files::load_map;
use app::render::map::render_map;
use app::render::transcript::render_transcript;
use app::session::{apply_turn, TurnResult};
use app::state::AppState;
use mapper::mapper::Mapper;
use mapper::render::render as render_map_data;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use zvm::ObjectSnapshot;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Build a minimal valid v3 Z-machine (same approach as render::transcript tests).
fn minimal_machine() -> zvm::cpu::exec::Machine {
    use zvm::memory::Memory;

    let mut buf = vec![0u8; 0x0800];
    buf[0x00] = 3; // version = 3
    buf[0x04] = 0x00; buf[0x05] = 0x40; // high_mem_base = 0x0040
    buf[0x06] = 0x00; buf[0x07] = 0x40; // initial_pc = 0x0040
    buf[0x0A] = 0x00; buf[0x0B] = 0x80; // dict = 0x0080
    buf[0x0C] = 0x01; buf[0x0D] = 0x00; // object table = 0x0100
    buf[0x0E] = 0x03; buf[0x0F] = 0x00; // global var table = 0x0300
    buf[0x08] = 0x04; buf[0x09] = 0x00; // static mem = 0x0400
    buf[0x18] = 0x00; buf[0x19] = 0x60; // abbrev table = 0x0060
    buf[0x0080] = 0; // word-sep count = 0
    buf[0x0081] = 4; // entry size = 4
    buf[0x0082] = 0; buf[0x0083] = 0; // entry count = 0
    buf[0x0040] = 0xba; // quit opcode at initial PC

    let mem = Memory::new(buf).expect("minimal v3 story");
    zvm::cpu::exec::Machine::new(mem)
}

/// Build a synthetic `TurnResult` for room `number` / `name`, no quit.
fn turn(number: u16, name: &str) -> TurnResult {
    TurnResult {
        transcript: String::new(),
        transcript_runs: Vec::new(),
        location: Some(ObjectSnapshot { number, parent: 0, name: name.to_owned() }),
        quit: false,
        erase_lower: false,
        info: None,
        sounds: Vec::new(),
        glulx_sound_ops: Vec::new(),
        diagnostics: vec![],
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

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Simulate a 3-room walk (Room1 → Room2 via E → Room3 via N), then assert:
///
/// 1. The map buffer has the current-room REVERSED highlight.
/// 2. The transcript buffer shows a pushed line.
/// 3. The SVG contains all 3 room labels.
/// 4. A saved map file round-trips the connections through load_map.
#[test]
fn headless_e2e_smoke() {
    // ── Step 1: Build state + mapper, simulate 3-room walk ────────────────────

    let mut state = AppState::default();
    let mut mapper = Mapper::default();

    // Room 1: starting room (no command/direction).
    let r1 = turn(1, "Entrance Hall");
    apply_turn(&mut mapper, "", &r1, &mut Default::default());

    // Room 2: move East.
    let r2 = turn(2, "East Corridor");
    apply_turn(&mut mapper, "east", &r2, &mut Default::default());

    // Room 3: move North.
    let r3 = turn(3, "North Gallery");
    apply_turn(&mut mapper, "north", &r3, &mut Default::default());

    // Room 3 is now current.
    assert_eq!(mapper.graph.current(), Some(3));
    // Two connections: 1→E→2 and 2→N→3.
    assert_eq!(mapper.graph.connections().len(), 2);

    // Select room 3 and push a transcript line.
    state.select_room(Some(3));
    state.push_transcript("You are in the North Gallery.");

    // ── Step 2: Render map into TestBackend buffer ────────────────────────────

    let rm = render_map_data(&mapper.graph);

    // The current room (room 3) should be in rm.rooms.
    let current_render_room = rm.rooms.iter().find(|r| r.is_current)
        .expect("render must have a current room");
    assert_eq!(current_render_room.id, 3);

    let area = Rect::new(0, 0, 80, 24);
    let mut map_buf = Buffer::empty(area);

    // Recenter on the current room's cell so it's on screen.
    // scroll is in cell-grid units; at Boxes zoom (step 8×4), a pane of 80×24
    // cols/rows shows 10×6 cells, so pass pane dims in cell units.
    state.recenter_on(current_render_room.cell, 10, 6);
    render_map(&rm, &state, area, &mut map_buf);

    // Find where the current room was rendered and verify REVERSED modifier.
    let (cx, cy) = app::render::map::cell_to_screen(
        current_render_room.cell,
        state.zoom,
        state.scroll,
        area,
    )
    .expect("current room must be on screen after recentering");

    // The current room reverses only its interior: the top-left border cell is NOT
    // reversed, while an interior cell (one in, one down) IS.
    let border = map_buf.cell((cx, cy)).expect("border cell must exist");
    assert!(
        !border.modifier.contains(Modifier::REVERSED),
        "current room border cell must NOT have REVERSED modifier; got {:?}",
        border.modifier
    );
    let interior = map_buf.cell((cx + 1, cy + 1)).expect("interior cell must exist");
    assert!(
        interior.modifier.contains(Modifier::REVERSED),
        "current room interior cell must have REVERSED modifier; got {:?}",
        interior.modifier
    );

    // ── Step 3: Render transcript into TestBackend buffer ─────────────────────

    let machine = minimal_machine();
    let status = app::session::status_model_from_machine(&machine);
    let transcript_area = Rect::new(0, 0, 80, 10);
    let mut trans_buf = Buffer::empty(transcript_area);
    render_transcript(&status, None, &state, transcript_area, &mut trans_buf, None);

    // Check that the pushed line appears somewhere in the buffer.
    let buf_text: String = trans_buf.content.iter()
        .flat_map(|c| c.symbol().chars())
        .collect();
    assert!(
        buf_text.contains("North Gallery"),
        "transcript buffer must contain the pushed line; buf='{}'",
        &buf_text[..buf_text.len().min(200)]
    );

    // ── Step 4: SVG export contains all 3 room labels ─────────────────────────

    let svg = render_svg(&rm);
    assert!(svg.contains("Entrance Hall"), "SVG must contain 'Entrance Hall'");
    assert!(svg.contains("East Corridor"), "SVG must contain 'East Corridor'");
    assert!(svg.contains("North Gallery"), "SVG must contain 'North Gallery'");

    // ── Step 5: map file round-trips connections through load_map ─────────────

    let tmp_dir = std::env::temp_dir().join(format!("lanthorn-headless-{}", std::process::id()));
    std::fs::create_dir_all(&tmp_dir).expect("create tmp dir");
    let map_file = tmp_dir.join("test.map.json");

    std::fs::write(&map_file, mapper::persist::to_json(&mapper)).expect("write map file must succeed");
    let loaded = load_map(&map_file).expect("load_map must return Some");

    // Connections round-trip.
    let orig_conns = mapper.graph.connections();
    let load_conns = loaded.graph.connections();
    assert_eq!(
        load_conns.len(), orig_conns.len(),
        "loaded mapper must have same connection count"
    );
    for (orig, loaded) in orig_conns.iter().zip(load_conns.iter()) {
        assert_eq!(orig.origin, loaded.origin, "connection origin must match");
        assert_eq!(orig.dir, loaded.dir, "connection dir must match");
        assert_eq!(orig.dest, loaded.dest, "connection dest must match");
    }

    // Clean up.
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

/// Back-compat: a frame rendered with default (no-scheme) config uses today's exact ANSI
/// colors.  The connector arrowhead's fg must be Cyan, matching the pre-refactor constant.
#[test]
fn colors_default_config_connector_is_cyan() {
    use mapper::graph::MapGraph;
    use mapper::direction::Direction;

    let mut g = MapGraph::new();
    g.upsert_room(1, "R1".into());
    g.upsert_room(2, "R2".into());
    g.set_pos(1, (0, 0));
    g.set_pos(2, (1, 0));
    g.add_edge(1, Direction::E, 2);

    let rm = render_map_data(&g);

    // Default state has terminal_default() colors (no config).
    let state = AppState::default();
    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    render_map(&rm, &state, area, &mut buf);

    // The arrowhead embedded in room1's right border (col 10, row 2) must be Cyan.
    let cell = buf.cell((10, 2)).expect("arrow cell must exist");
    assert_eq!(
        cell.fg, Color::Cyan,
        "with terminal_default colors, arrowhead fg must be Cyan (back-compat); got {:?}",
        cell.fg
    );
}

/// Scheme-swap: resolving a built-in scheme (tomorrow-night) changes the connector color
/// to the scheme's palette[6] (0x70c0ba), not Cyan.
#[test]
fn colors_scheme_swap_changes_connector_color() {
    use app::style::{StyleDoc, StyleColors};
    use mapper::graph::MapGraph;
    use mapper::direction::Direction;

    let mut g = MapGraph::new();
    g.upsert_room(1, "A".into());
    g.upsert_room(2, "B".into());
    g.set_pos(1, (0, 0));
    g.set_pos(2, (1, 0));
    g.add_edge(1, Direction::E, 2);

    let rm = render_map_data(&g);

    let mut doc = StyleDoc::default();
    doc.colors = StyleColors { scheme: Some("tomorrow-night".to_string()), selectors: Default::default() };
    let (colors, _set, warnings) = app::style::resolve(&doc, std::path::Path::new("/tmp"));
    assert!(warnings.is_empty(), "tomorrow-night should resolve without warnings");

    let mut state = AppState::default();
    state.colors = colors;

    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    render_map(&rm, &state, area, &mut buf);

    // Tomorrow Night palette[6] = #70c0ba.
    let expected = Color::Rgb(0x70, 0xc0, 0xba);

    let cell = buf.cell((10, 2)).expect("arrow cell must exist");
    assert_eq!(
        cell.fg, expected,
        "with tomorrow-night scheme, arrowhead fg must be Rgb(0x70,0xc0,0xba); got {:?}",
        cell.fg
    );
    // Confirm it is different from the default Cyan (sanity check the test is meaningful).
    assert_ne!(cell.fg, Color::Cyan, "scheme color should differ from default Cyan");
}
