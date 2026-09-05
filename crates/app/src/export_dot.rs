//! Graphviz DOT map export: emit the raw room graph as DOT so a Graphviz engine
//! (neato/fdp/dot) lays it out.
//!
//! Unlike the SVG export, this does NOT use lanthorn's own positions or routed
//! connectors. It hands Graphviz the topology — rooms as nodes, directed
//! connections as edges labelled with their compass/stub direction — and lets
//! Graphviz solve the layout. This sidesteps lanthorn's in-app layout/routing
//! entirely, which is the point: Graphviz's engines handle dense graphs well.
//!
//! The emitted graph is a `digraph` because connections are directed and not
//! assumed reciprocal: `A -> B [label="N"]` and `B -> A [label="W"]` are two
//! independent edges, exactly as observed.

use std::collections::BTreeMap;
use std::path::Path;

use mapper::direction::Direction;
use mapper::graph::MapGraph;
use mapper::layer::LayerId;

/// Short label string for a connection direction, matching the TUI conventions.
fn dir_label(d: Direction) -> &'static str {
    match d {
        Direction::N => "N",
        Direction::S => "S",
        Direction::E => "E",
        Direction::W => "W",
        Direction::NE => "NE",
        Direction::NW => "NW",
        Direction::SE => "SE",
        Direction::SW => "SW",
        Direction::Up => "U",
        Direction::Down => "D",
        Direction::In => "IN",
        Direction::Out => "OUT",
        Direction::Unknown => "?",
    }
}

/// Escape a string for use inside a DOT double-quoted value.
fn dot_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// Render a `MapGraph` to a Graphviz DOT document string.
///
/// A room's DOT node identifier: `r` plus [`crate::roomid::display_room_id`]'s
/// digits, minus the `#` — a bare DOT identifier cannot contain one — so a real
/// object number reads as `r57` and a synthetic id as `r8000ABCD` rather than a
/// 10-digit decimal, the same real/synthetic distinction every other room-id
/// display uses (SQ-1297).
///
/// Deliberately NOT [`crate::roomid::room_label_no`]'s per-map ordinal (SQ-1300): the ordinal is
/// discovery order for THIS playthrough, so two exports of the same story explored in a different
/// order would number the same physical room differently, where a DOT file's whole point is to be
/// compared — against another export, against notes, against a previous run. The raw id (a real
/// object number, or a hash of the room's name/Glulx address) has no such dependency on how the
/// player got there. The exported graph's node LABELS still show the room's name either way.
fn node_id(id: mapper::graph::RoomId) -> String {
    format!("r{}", crate::roomid::display_room_id(id).trim_start_matches('#'))
}

/// The node id is `r<object-number>` (or `r<hex>` for a synthetic id — see
/// [`node_id`]), so same-named rooms stay distinct. The current room is filled
/// gold; rooms with notes carry their notes as a tooltip and a `●` marker in
/// the label.
pub fn render_dot(graph: &MapGraph) -> String {
    let mut out = String::new();

    out.push_str("// lanthorn map export (Graphviz DOT)\n");
    out.push_str("// Render with:  dot -Tsvg map.dot -o map.svg\n");
    out.push_str("//   (the layout=neato attribute below picks the engine; fdp/sfdp also work)\n");
    out.push_str("digraph lanthorn {\n");
    out.push_str("  layout=\"neato\";\n");
    out.push_str("  overlap=false;\n");
    out.push_str("  splines=true;\n");
    out.push_str("  node [shape=box, style=\"rounded\", fontname=\"monospace\"];\n");
    out.push_str("  edge [fontname=\"monospace\", fontsize=10];\n");

    // Nodes, in ascending id order for deterministic output, grouped into one
    // Graphviz cluster per map layer (SQ-1308) — a single-layer graph groups
    // everything under one (unlabelled, undrawn) cluster, so this reproduces
    // the un-clustered output byte for byte until a second layer exists.
    let mut rooms: Vec<&mapper::graph::Room> = graph.rooms().collect();
    rooms.sort_by_key(|r| r.id);
    let mut by_layer: BTreeMap<LayerId, Vec<&mapper::graph::Room>> = BTreeMap::new();
    for room in &rooms {
        by_layer.entry(room.layer).or_default().push(room);
    }
    let multi_layer = by_layer.len() > 1;

    let current = graph.current();

    for (layer, layer_rooms) in &by_layer {
        if multi_layer {
            out.push_str(&format!("  subgraph cluster_{layer} {{\n"));
            out.push_str(&format!("    label=\"{}\";\n", dot_escape(graph.layer_name(*layer))));
        }
        let indent = if multi_layer { "    " } else { "  " };
        for room in layer_rooms {
            let has_notes = !room.notes.is_empty();
            let mut label = dot_escape(room.label());
            if has_notes {
                label.push_str(" ●");
            }

            let mut attrs = format!("label=\"{}\"", label);
            if Some(room.id) == current {
                attrs.push_str(", style=\"rounded,filled\", fillcolor=\"gold\"");
            }
            if has_notes {
                attrs.push_str(&format!(", tooltip=\"{}\"", dot_escape(&room.notes)));
            }
            out.push_str(&format!("{indent}{} [{}];\n", node_id(room.id), attrs));
        }
        if multi_layer {
            out.push_str("  }\n");
        }
    }

    // Edges, in connection order.
    for conn in graph.connections() {
        out.push_str(&format!(
            "  {} -> {} [label=\"{}\"];\n",
            node_id(conn.origin),
            node_id(conn.dest),
            dir_label(conn.dir)
        ));
    }

    out.push_str("}\n");
    out
}

/// Write `render_dot(graph)` to the file at `path`.
pub fn export_dot(path: &Path, graph: &MapGraph) -> std::io::Result<()> {
    crate::storage::atomic_write(path, render_dot(graph).as_bytes())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-state"))]
mod tests {
    use super::*;
    use mapper::direction::Direction;
    use mapper::mapper::Mapper;

    #[test]
    fn dot_contains_nodes_and_directed_edges() {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "Forest", Some(Direction::N));
        m.observe(1, "West of House", Some(Direction::S)); // back south — non-reciprocal pair
        let dot = render_dot(&m.graph);

        assert!(dot.contains("digraph lanthorn {"));
        assert!(dot.contains("}\n"));
        // Nodes keyed by object number.
        assert!(dot.contains("r1 ["));
        assert!(dot.contains("r2 ["));
        assert!(dot.contains("label=\"West of House"));
        assert!(dot.contains("label=\"Forest"));
        // Directed edge with a compass label.
        assert!(dot.contains("r1 -> r2 [label=\"N\"]"));
        assert!(dot.contains("r2 -> r1 [label=\"S\"]"));
    }

    #[test]
    fn current_room_is_filled() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E)); // room 2 is current
        let dot = render_dot(&m.graph);
        // The current room (r2) line must carry the filled-gold style.
        let r2_line = dot.lines().find(|l| l.trim_start().starts_with("r2 [")).unwrap();
        assert!(r2_line.contains("filled"), "current room must be filled: {r2_line}");
        assert!(r2_line.contains("gold"), "current room fill must be gold: {r2_line}");
        // The non-current room must NOT be filled.
        let r1_line = dot.lines().find(|l| l.trim_start().starts_with("r1 [")).unwrap();
        assert!(!r1_line.contains("filled"), "non-current room must not be filled: {r1_line}");
    }

    #[test]
    fn notes_become_tooltip_and_marker() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Cellar".into());
        g.set_notes(1, "trophy case here".into());
        let dot = render_dot(&g);
        let line = dot.lines().find(|l| l.trim_start().starts_with("r1 [")).unwrap();
        assert!(line.contains("tooltip=\"trophy case here\""), "notes → tooltip: {line}");
        assert!(line.contains('●'), "notes → ● marker in label: {line}");
    }

    #[test]
    fn labels_are_escaped() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall of \"Mirrors\"\\Doom".into());
        let dot = render_dot(&g);
        assert!(dot.contains("Hall of \\\"Mirrors\\\"\\\\Doom"), "quotes/backslashes escaped: {dot}");
    }

    #[test]
    fn stub_and_unknown_directions_labelled() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.upsert_room(3, "C".into());
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(1, Direction::Unknown, 3);
        let dot = render_dot(&g);
        assert!(dot.contains("r1 -> r2 [label=\"U\"]"));
        assert!(dot.contains("r1 -> r3 [label=\"?\"]"));
    }

    #[test]
    fn empty_map_is_valid_digraph() {
        use mapper::graph::MapGraph;
        let g = MapGraph::new();
        let dot = render_dot(&g);
        assert!(dot.contains("digraph lanthorn {"));
        assert!(dot.trim_end().ends_with('}'));
        // No room/edge lines.
        assert!(!dot.contains("->"));
        assert!(!dot.lines().any(|l| l.trim_start().starts_with('r')));
    }
}
