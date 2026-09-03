//! Map dump: a self-contained, human-readable text snapshot of the explored map
//! for offline evaluation and regression fixtures.
//!
//! The dump has three parts:
//!   1. a ROOM legend (`id`, name, grid pos, notes),
//!   2. an EDGE list (directed `origin DIR dest`, marked `distorted` when the
//!      layout could not satisfy the compass direction),
//!   3. an ASCII rendering of the actual map — produced by rendering the real
//!      `render_map` into an off-screen buffer at Boxes zoom and copying each
//!      cell's symbol directly (blank cells become spaces) — so it faithfully
//!      shows the routing (clearances, crossings) the TUI draws as box-drawing line-art.
//!
//! Lines starting with `#` are comments: the file is meant to be annotated (mark
//! which room ids look wrong) and handed back for analysis.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use mapper::direction::{grid_offset, Direction};
use mapper::graph::MapGraph;
use mapper::layout::detect_chains;
use mapper::render::render_layer;

use crate::render::map::{arrow_for_direction, boxes_axes, render_map};
use crate::state::{AppState, Zoom};
use crate::symbols::SymbolSet;

/// Max dump buffer dimension (cells) to bound memory on very large maps.
const MAX_DIM: i32 = 4000;

/// Short label for a connection direction (matches the TUI / DOT conventions).
fn dir_str(d: Direction) -> &'static str {
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

/// Render the map to a line-art ASCII string (rooms as boxes, connectors as
/// `─│┼` lines, exits as arrowheads).
#[allow(clippy::field_reassign_with_default)]
fn ascii_map(graph: &MapGraph, layer: mapper::layer::LayerId, symbols: &SymbolSet) -> String {
    // `render_layer`, not `render` on a pre-sliced subgraph: only `render_layer` flags rooms that
    // own a cross-layer portal and appends the interlayer badges. Rendering a subgraph through
    // plain `render` silently dropped BOTH, so the dump showed a room with a staircase to another
    // layer as an ordinary room — the one view where you'd most want to see it (SQ-0223).
    let rm = render_layer(graph, layer);
    if rm.rooms.is_empty() {
        return "(empty map)".to_string();
    }
    let ((min_col, min_row), _) = rm.bounds;

    // Build the lane-aware position tables so the buffer is sized from the ACTUAL
    // non-uniform column/row widths, not a uniform estimate.  A busy channel that
    // carries ≥5 lanes grows wider than the old STEP_W constant, so the old
    // formula would undercount and clip connectors on the right/bottom.
    let (col_table, row_table) = boxes_axes(&rm.plan, rm.bounds);

    // Pad by 2 default-stride steps each side so connectors detouring outside the
    // room bounding box are captured rather than clipped.  The default step for an
    // empty channel is BOX_W + MIN_GUTTER (13) / BOX_H + MIN_GUTTER (7); we use
    // the table's extrapolation via room_pixel on indices outside the map.
    let default_step_w = col_table.room_pixel(min_col) - col_table.room_pixel(min_col - 1);
    let default_step_h = row_table.room_pixel(min_row) - row_table.room_pixel(min_row - 1);
    let pad_w = col_table.room_pixel(min_col) - col_table.room_pixel(min_col - 2);
    let pad_h = row_table.room_pixel(min_row) - row_table.room_pixel(min_row - 2);
    // Left/top padding: from scroll origin to map start (pad_w / pad_h).
    // Right/bottom padding: 2 extra default steps beyond the real map extent.
    let area_w = (col_table.total_pixels() + pad_w + 2 * default_step_w).clamp(1, MAX_DIM);
    let area_h = (row_table.total_pixels() + pad_h + 2 * default_step_h).clamp(1, MAX_DIM);
    let area = Rect::new(0, 0, area_w as u16, area_h as u16);

    // Render at Boxes zoom (AppState default) with scroll set to pad the map.
    // Always show room numbers in the dump (diagnostic context).
    // The player's OWN glyph set, not `AppState::default()`'s: a dump is a picture of
    // the map on screen, and one drawn with somebody else's box style and portal icons
    // is a picture of a map nobody is looking at (SQ-0989).
    let mut state = AppState::default();
    state.symbols = symbols.clone();
    state.zoom = Zoom::Boxes;
    state.scroll = (min_col - 2, min_row - 2);
    state.show_room_numbers = true;

    let mut buf = Buffer::empty(area);
    render_map(&rm, &state, area, &mut buf);

    // Serialize: copy each cell's symbol directly (blank → space).
    let mut lines: Vec<String> = Vec::with_capacity(area_h as usize);
    for y in 0..area_h {
        let mut line = String::new();
        for x in 0..area_w {
            let sym = buf.cell((x as u16, y as u16)).map(|c| c.symbol()).unwrap_or(" ");
            line.push_str(if sym.is_empty() { " " } else { sym });
        }
        lines.push(line.trim_end().to_string());
    }

    // Trim leading/trailing all-blank rows.
    let first = lines.iter().position(|l| !l.is_empty()).unwrap_or(0);
    let last = lines.iter().rposition(|l| !l.is_empty()).unwrap_or(0);
    lines[first..=last].join("\n")
}

/// Produce the full map dump string for `graph`, drawn with the player's `symbols`.
pub fn render_dump(graph: &MapGraph, symbols: &SymbolSet) -> String {
    let mut rooms: Vec<&mapper::graph::Room> = graph.rooms().collect();
    rooms.sort_by_key(|r| r.id);
    let conns = graph.connections();

    let mut out = String::new();
    out.push_str("# lanthorn map dump\n");
    let current = graph.current().map(|id| format!("#{id}")).unwrap_or_else(|| "none".into());
    out.push_str(&format!(
        "# rooms: {}, edges: {}, current: {}\n#\n",
        rooms.len(),
        conns.len(),
        current
    ));

    if rooms.is_empty() {
        out.push_str("# (empty map)\n");
        return out;
    }

    let chains = detect_chains(graph);
    out.push_str("# === ROOMS (id  name  pos  notes) ===\n");
    for r in &rooms {
        let pos = r.pos.map(|(x, y)| format!("{x},{y}")).unwrap_or_else(|| "?".into());
        let notes = if r.notes.is_empty() {
            String::new()
        } else {
            format!("  notes={:?}", r.notes)
        };
        // Other names the story has printed for this room (SQ-1257 Phase 3) — Lost Pig's gnome
        // tunnels are the specimen — so an exported map still says what the room was called
        // before its label last changed.
        let aka = if r.aliases.is_empty() {
            String::new()
        } else {
            format!("  aka={:?}", r.aliases)
        };

        // Every `?` random-exit direction (SQ-1261), with the distinct rooms it has actually
        // been seen to land in, by id and name — the same evidence the room card and the
        // matrix's superscript count draw from, so a dump can be cross-checked against either.
        let random = if r.random_exits.is_empty() {
            String::new()
        } else {
            let dirs: Vec<String> = r
                .random_exits
                .iter()
                .map(|&d| {
                    let dests: Vec<String> = graph
                        .random_destinations(r.id, d)
                        .iter()
                        .map(|&id| {
                            let name =
                                graph.room(id).map(|rr| rr.label().to_string()).unwrap_or_default();
                            format!("#{id} {name:?}")
                        })
                        .collect();
                    format!("{}→({})", dir_str(d), dests.join(", "))
                })
                .collect();
            format!("  random=[{}]", dirs.join(", "))
        };

        // Build align= annotation.
        let mut align_parts: Vec<String> = Vec::new();
        if let Some(&cid) = chains.ew.get(&r.id) {
            let members: Vec<String> = chains.ew_members[cid].iter().map(|id| id.to_string()).collect();
            align_parts.push(format!("row[{}]", members.join(",")));
        }
        if let Some(&cid) = chains.ns.get(&r.id) {
            let members: Vec<String> = chains.ns_members[cid].iter().map(|id| id.to_string()).collect();
            align_parts.push(format!("col[{}]", members.join(",")));
        }
        let align = if align_parts.is_empty() {
            "none".to_string()
        } else {
            align_parts.join(" ")
        };

        // Build dropped= annotation (outgoing distorted compass edges).
        let dropped: Vec<String> = conns
            .iter()
            .filter(|c| c.origin == r.id && c.distorted && grid_offset(c.dir).is_some())
            .map(|c| format!("{}→{}→{}", c.origin, dir_str(c.dir), c.dest))
            .collect();
        let dropped_str = if dropped.is_empty() {
            String::new()
        } else {
            format!(" dropped=[{}]", dropped.join(", "))
        };

        // Tag the room's layer when it is not the base layer, so a multi-layer dump
        // says which plane each room lives on. Single-layer dumps stay unchanged.
        let layer_str = if r.layer != mapper::layer::MAIN_LAYER {
            format!(" layer={}", r.layer)
        } else {
            String::new()
        };
        out.push_str(&format!(
            "ROOM {} {:?} pos={}{}{}{} align={}{}{}\n",
            r.id, r.label(), pos, notes, aka, random, align, dropped_str, layer_str
        ));
    }

    out.push_str("#\n# === EDGES (origin DIR dest) ===\n");
    for c in conns {
        let dist = if c.distorted { "  distorted" } else { "" };
        out.push_str(&format!("EDGE {} {} {}{}\n", c.origin, dir_str(c.dir), c.dest, dist));
    }

    let portals: Vec<String> = conns
        .iter()
        .filter(|c| grid_offset(c.dir).is_none())
        .map(|c| {
            let name = graph.room(c.dest).map(|r| r.label().to_string()).unwrap_or_default();
            let glyph = arrow_for_direction(c.dir, &symbols.arrows, &symbols.portal);
            format!("PORTAL {} {} #{} {}", c.origin, glyph, c.dest, name)
        })
        .collect();
    if !portals.is_empty() {
        out.push_str("#\n# === PORTALS (origin glyph #target name) ===\n");
        for line in &portals {
            out.push_str(line);
            out.push('\n');
        }
    }

    out.push_str("#\n# === MAP (#id = room, lines = connectors, ▶◀▲▼ = exits) ===\n");
    // Each layer is its own coordinate plane — render them separately so they don't
    // overlap. A single-layer map renders exactly as before (one map, no layer header).
    let map_layers: Vec<mapper::layer::LayerId> = graph
        .layers()
        .keys()
        .copied()
        .filter(|&l| !graph.rooms_in_layer(l).is_empty())
        .collect();
    if map_layers.len() <= 1 {
        // Keep rendering whichever layer actually holds the rooms — it is not always MAIN_LAYER.
        let only = map_layers.first().copied().unwrap_or(mapper::layer::MAIN_LAYER);
        out.push_str(&ascii_map(graph, only, symbols));
    } else {
        for (i, &l) in map_layers.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&format!("# --- layer {} ({}) ---\n", l, graph.layer_name(l)));
            out.push_str(&ascii_map(graph, l, symbols));
        }
    }
    out.push_str("\n#\n# Annotate problems below — lines starting with # are comments:\n#\n");

    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-state"))]
mod tests {
    use super::*;
    use mapper::direction::Direction;
    use mapper::mapper::Mapper;

    #[test]
    fn dump_separates_layers_and_tags_membership() {
        // Hall (1) on MAIN; Cellar (2) reached by Down, peeled into its own layer.
        let mut g = mapper::graph::MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Cellar".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 0)); // SAME cell as Hall — only safe because they're different layers
        g.add_edge(1, Direction::Down, 2);
        g.add_edge(2, Direction::Up, 1);
        let region = mapper::layer::planar_region(&g, 2);
        let l = mapper::layer::move_region(&mut g, &region, mapper::layer::MoveTarget::New)
            .expect("peel cellar");
        let dump = render_dump(&g, &SymbolSet::default());
        // ROOM legend tags the peeled room's layer; the base room carries no tag.
        assert!(dump.contains(&format!("ROOM 2 \"Cellar\" pos=0,0 align=none layer={l}")), "dump:\n{dump}");
        assert!(dump.lines().any(|ln| ln.starts_with("ROOM 1 \"Hall\"") && !ln.contains("layer=")));
        // The MAP renders each layer under its own header (no single merged plane).
        assert!(dump.contains("# --- layer 0 (Main) ---"), "dump:\n{dump}");
        assert!(dump.contains(&format!("# --- layer {l} (Cellar) ---")), "dump:\n{dump}");
    }

    /// A dump is drawn with the PLAYER's glyphs, in both places it names a portal:
    /// the PORTALS legend and the badge inside the room box. It used to reach for a
    /// pair of hard-coded constants for the legend and `AppState::default()` for the
    /// map, so `/export-map` showed the shipped icons to a player who had chosen
    /// others — and matching a dump against the screen is the whole point of it
    /// (SQ-0989).
    #[test]
    fn dump_portals_use_the_configured_icon_preset() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Vault".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::In, 2);

        let nerd = SymbolSet::from_preset_names("light", "filled", "nerdfont", "light");
        let dump = render_dump(&g, &nerd);
        // nf-fa-sign_in (U+F090) — the "nerdfont" preset's In icon.
        assert!(
            dump.contains(&format!("PORTAL 1 {} #2 Vault", '\u{F090}')),
            "legend must use the configured preset:\n{dump}"
        );
        assert!(dump.contains('\u{F090}'), "map badge must use it too:\n{dump}");
        // And the default preset's In icon appears nowhere: the dump committed to one set.
        let ascii_in = SymbolSet::default().portal.in_;
        assert!(!dump.contains(ascii_in), "default icon {ascii_in:?} leaked into a themed dump:\n{dump}");

        // The default set still reads as it always did.
        let plain = render_dump(&g, &SymbolSet::default());
        assert!(
            plain.contains(&format!("PORTAL 1 {ascii_in} #2 Vault")),
            "default dump keeps the default icon:\n{plain}"
        );
    }

    #[test]
    fn dump_lists_rooms_edges_and_ids() {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "Forest", Some(Direction::N));
        let dump = render_dump(&m.graph, &SymbolSet::default());

        assert!(dump.contains("# lanthorn map dump"));
        assert!(dump.contains("ROOM 1 \"West of House\""), "room legend: {dump}");
        assert!(dump.contains("ROOM 2 \"Forest\""));
        assert!(dump.contains("EDGE 1 N 2"), "edge list: {dump}");
        // The ASCII map shows room ids.
        assert!(dump.contains("#1"));
        assert!(dump.contains("#2"));
    }

    /// SQ-1257 Phase 3: a room the story has renamed carries its other names on the ROOM line
    /// as `aka=[...]`, so `/export-map` keeps them. A room with no rename carries no `aka=`
    /// segment at all — the field must not clutter every ordinary room.
    #[test]
    fn dump_room_line_carries_aka_only_when_the_room_has_aliases() {
        let mut m = Mapper::default();
        m.observe_moved(183, "Twisty Cave", None);
        m.observe_moved(183, "Confusing Passage", Some(Direction::N));
        m.observe(2, "Plain Room", None); // never renamed
        let dump = render_dump(&m.graph, &SymbolSet::default());

        assert!(
            dump.contains(r#"aka=["Twisty Cave"]"#),
            "the tunnel room's old name appears as aka=[...]:\n{dump}"
        );
        let plain_room_line =
            dump.lines().find(|l| l.starts_with("ROOM 2 ")).expect("room 2's line");
        assert!(!plain_room_line.contains("aka="), "an unrenamed room carries no aka= at all: {plain_room_line}");
    }

    /// SQ-1261: a `?` random exit carries the rooms it has actually landed in on the ROOM line
    /// as `random=[...]`, by id and name — so an exported map keeps the same evidence the room
    /// card and the matrix's superscript count show. A room with no random exit carries no
    /// `random=` segment at all.
    #[test]
    fn dump_room_line_carries_random_destinations_only_when_recorded() {
        let mut m = Mapper::default();
        m.observe(1, "Windy Cave", None);
        m.observe(2, "Plain Room", None); // never marked
        assert!(m.record_random_exit(1, Direction::N));
        m.graph.note_random_destination(1, Direction::N, 2);
        let dump = render_dump(&m.graph, &SymbolSet::default());

        let random_room_line =
            dump.lines().find(|l| l.starts_with("ROOM 1 ")).expect("room 1's line");
        assert!(
            random_room_line.contains(r#"random=[N→(#2 "Plain Room")]"#),
            "the marked direction names its recorded destination by id and name: {random_room_line}"
        );
        let plain_room_line =
            dump.lines().find(|l| l.starts_with("ROOM 2 ")).expect("room 2's line");
        assert!(
            !plain_room_line.contains("random="),
            "a room with no random exit carries no random= at all: {plain_room_line}"
        );
    }

    #[test]
    fn dump_ascii_has_line_art_connector() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        let dump = render_dump(&m.graph, &SymbolSet::default());
        // A horizontal connector run should serialize to box-drawing line-art.
        let has_line = dump.contains('─') || dump.contains('│') || dump.contains('┼');
        assert!(has_line, "expected line-art connectors in:\n{dump}");
        // And an exit arrow somewhere.
        let has_arrow = dump.contains('▶') || dump.contains('◀') || dump.contains('▲') || dump.contains('▼');
        assert!(has_arrow, "expected an exit arrow in:\n{dump}");
    }

    #[test]
    fn empty_map_dump_is_safe() {
        let g = MapGraph::new();
        let dump = render_dump(&g, &SymbolSet::default());
        assert!(dump.contains("# lanthorn map dump"));
        assert!(dump.contains("(empty map)"));
    }

    #[test]
    fn dump_copies_glyphs_directly_no_ribbon_mask() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        let dump = render_dump(&m.graph, &SymbolSet::default());
        // A connector line-art glyph appears, and the legend no longer advertises ▒.
        assert!(dump.contains('─') || dump.contains('│'), "line-art connector expected:\n{dump}");
        assert!(!dump.contains("▒ = unrouted"), "unrouted concept removed from legend");
    }

    #[test]
    fn dump_legend_shows_alignment_rules() {
        use mapper::direction::Direction;
        let mut m = mapper::mapper::Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        // make it a reciprocal E/W pair so a chain forms
        m.graph.add_edge(2, Direction::W, 1);
        let dump = render_dump(&m.graph, &SymbolSet::default());
        assert!(dump.contains("align=row[1,2]"), "reciprocal pair annotated as a row chain:\n{dump}");
    }

    #[test]
    fn dump_legend_marks_ungrouped_room() {
        let mut m = mapper::mapper::Mapper::default();
        m.observe(1, "A", None);
        let dump = render_dump(&m.graph, &SymbolSet::default());
        assert!(dump.contains("align=none"), "lone room shows align=none:\n{dump}");
    }

    /// Regression: the dump buffer must NOT truncate when a channel carries many lanes.
    ///
    /// With the OLD uniform sizing (`(cols+4)*STEP_W` where STEP_W=19), a busy vertical
    /// channel carrying ≥5 lanes grows wider than the 19-cell estimate (channel_width(5)
    /// = LANE_BASE + 4*LANE_SPACING + 1 = 10 which, added to BOX_W=11, gives stride 21
    /// vs the estimated 19), so the cumulative map width overflows the buffer and
    /// `render_map` silently clips connectors on the right side. The new sizing derives
    /// the buffer dimensions from the actual lane-aware `boxes_axes` tables.
    ///
    /// Setup: hub room at (1,0), plus 5 spoke rooms at (0,1)..(0,5) each connecting
    /// East to the hub. The spoke connectors from rows 1–5 all route through V(0) at
    /// overlapping doubled-coord extents (each extends from its row down to y=1), so the
    /// lane router must assign them 5 separate lanes in V(0), widening that channel
    /// beyond the STEP_W constant.
    #[test]
    fn dump_buffer_not_truncated_on_busy_channel() {
        use mapper::graph::MapGraph;

        let mut g = MapGraph::new();
        // Hub at column 1, row 0.
        g.upsert_room(1, "Hub".into());
        g.set_pos(1, (1, 0));
        // Five spoke rooms at column 0, rows 1..5.
        for row in 1i32..=5 {
            let id = (row + 1) as u16;
            g.upsert_room(id, format!("S{row}"));
            g.set_pos(id, (0, row));
            g.add_edge(id, Direction::E, 1);
        }

        let dump = render_dump(&g, &SymbolSet::default());

        // Every room's id label must appear in the dump.
        for id in 1u16..=6 {
            assert!(
                dump.contains(&format!("#{id}")),
                "room #{id} must appear in dump (not clipped); dump:\n{dump}",
            );
        }

        // Every room box's right border must be present: look for the '╯' or '╮'
        // closing corners (bottom-right / top-right of a normal room box).
        let right_corners = dump.matches('╯').count() + dump.matches('╮').count();
        assert!(
            right_corners >= 6,
            "expected ≥6 right-border corners (one per room), got {right_corners}; dump may be clipped:\n{dump}",
        );
    }

    #[test]
    fn dump_portal_legend_shows_glyph_id_name() {
        let mut m = Mapper::default();
        m.observe(1, "Cellar", None);
        m.observe(2, "Attic", Some(Direction::Up)); // edge 1 →Up→ 2, both placed
        let dump = render_dump(&m.graph, &SymbolSet::default());
        assert!(dump.contains("# === PORTALS"), "portal legend section present:\n{dump}");
        assert!(
            dump.contains("PORTAL 1 ↑ #2 Attic"),
            "portal line shows origin, glyph, target id and full name:\n{dump}"
        );
    }
}
