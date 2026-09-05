/// SVG map export: render a `RenderMap` to a standalone SVG document.
///
/// # Coordinate mapping
///
/// Logical cell `(c, r)` → SVG pixels via:
///   `px_x = MARGIN + (c - min_col) * CELL_W`
///   `px_y = MARGIN + (r - min_row) * CELL_H`
///
/// Fine-grid point `(fx, fy)` → SVG pixels via half-cell steps:
///   `px_x = MARGIN + (fx - fine_min_x) * HALF_W`   where `HALF_W = CELL_W / 2`
///   `px_y = MARGIN + (fy - fine_min_y) * HALF_H`   where `HALF_H = CELL_H / 2`
///
/// Because a room cell `(c,r)` maps to fine cell `(2c, 2r)`, multiplying a fine coord
/// by `HALF_W` is the same as multiplying its logical coord by `CELL_W` — the two
/// mappings are identical and consistent.
use mapper::graph::MapGraph;
use mapper::render::RenderMap;
use std::fmt::Write as FmtWrite;
use std::path::Path;

/// Fixed cell size in SVG pixels.
const CELL_W: i32 = 64;
const CELL_H: i32 = 40;

/// Half-cell sizes for fine-grid → px mapping.
const HALF_W: i32 = CELL_W / 2;
const HALF_H: i32 = CELL_H / 2;

/// Margin around the rendered map in SVG pixels.
const MARGIN: i32 = 20;

/// Room box size (slightly smaller than cell so gutters are visible).
const ROOM_W: i32 = CELL_W - 8;
const ROOM_H: i32 = CELL_H - 8;

/// Escape XML special characters in a text value.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Render a `RenderMap` to a standalone SVG document string.
///
/// Empty map (no rooms): returns a minimal valid `<svg></svg>`.
pub fn render_svg(rm: &RenderMap) -> String {
    let Some((body, width, height)) = render_svg_body(rm) else {
        return "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"></svg>".to_string();
    };
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\">\
         <rect width=\"{width}\" height=\"{height}\" fill=\"#1a1a2e\"/>{body}</svg>"
    )
}

/// The room/edge markup for one `RenderMap`, with no outer `<svg>` tag and no
/// background rect — just the pieces [`render_svg`] wraps directly, and
/// [`render_svg_layered`] wraps once per layer inside a translated `<g>`.
/// `None` for an empty map, matching `render_svg`'s own empty-map case.
///
/// Returns the markup plus the `(width, height)` a caller needs to size its
/// own canvas around it.
fn render_svg_body(rm: &RenderMap) -> Option<(String, i32, i32)> {
    if rm.rooms.is_empty() {
        return None;
    }

    let ((min_col, min_row), (max_col, max_row)) = rm.bounds;

    // Fine-grid origin for connector coordinate mapping.
    let fine_min_x = 2 * min_col;
    let fine_min_y = 2 * min_row;

    // Total SVG canvas size.
    let cols = max_col - min_col + 1;
    let rows = max_row - min_row + 1;
    let width = 2 * MARGIN + cols * CELL_W;
    let height = 2 * MARGIN + rows * CELL_H;

    // Map a logical cell to the center pixel of that cell.
    let cell_center = |c: i32, r: i32| -> (i32, i32) {
        let x = MARGIN + (c - min_col) * CELL_W + CELL_W / 2;
        let y = MARGIN + (r - min_row) * CELL_H + CELL_H / 2;
        (x, y)
    };

    // Map a logical cell to the top-left pixel of the room rect.
    let cell_topleft = |c: i32, r: i32| -> (i32, i32) {
        let x = MARGIN + (c - min_col) * CELL_W + (CELL_W - ROOM_W) / 2;
        let y = MARGIN + (r - min_row) * CELL_H + (CELL_H - ROOM_H) / 2;
        (x, y)
    };

    // Map a fine-grid point to SVG pixels.
    let fine_to_px = |fx: i32, fy: i32| -> (i32, i32) {
        let x = MARGIN + (fx - fine_min_x) * HALF_W + HALF_W;
        let y = MARGIN + (fy - fine_min_y) * HALF_H + HALF_H;
        (x, y)
    };

    let mut svg = String::new();

    // ── Edges ────────────────────────────────────────────────────────────────

    for edge in &rm.edges {
        if edge.is_stub {
            // Draw a short stub line + label near origin room.
            if edge.points.len() >= 2 {
                let (x1, y1) = fine_to_px(edge.points[0].0, edge.points[0].1);
                let (x2, y2) = fine_to_px(edge.points[1].0, edge.points[1].1);
                let _ = write!(
                    svg,
                    "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"#888\" stroke-width=\"1\" stroke-dasharray=\"2 2\"/>",
                    x1, y1, x2, y2
                );
                if let Some(lbl) = &edge.label {
                    let lx = x2 + 2;
                    let ly = y2;
                    let _ = write!(
                        svg,
                        "<text x=\"{}\" y=\"{}\" font-size=\"8\" fill=\"#aaa\" font-family=\"monospace\">{}</text>",
                        lx, ly, xml_escape(lbl)
                    );
                }
            }
        } else {
            // Normal or distorted polyline.
            let pts: Vec<String> = edge
                .points
                .iter()
                .map(|&(fx, fy)| {
                    let (px, py) = fine_to_px(fx, fy);
                    format!("{},{}", px, py)
                })
                .collect();
            let pts_str = pts.join(" ");

            let (stroke, extra_attrs) = if edge.distorted {
                ("#e88", " stroke-dasharray=\"4 3\"")
            } else {
                ("#8bf", "")
            };

            let _ = write!(
                svg,
                "<polyline points=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"2\"{}/> ",
                pts_str, stroke, extra_attrs
            );
        }
    }

    // ── Rooms ────────────────────────────────────────────────────────────────

    for room in &rm.rooms {
        let (rx, ry) = cell_topleft(room.cell.0, room.cell.1);
        let (cx, cy) = cell_center(room.cell.0, room.cell.1);

        let (fill, stroke) = if room.is_current {
            ("#ffe", "#c00")
        } else {
            ("#2a2a4a", "#8bf")
        };

        let _ = write!(
            svg,
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"1\" rx=\"3\"/>",
            rx, ry, ROOM_W, ROOM_H, fill, stroke
        );

        // Label (XML-escaped, centered).
        let text_color = if room.is_current { "#c00" } else { "#ddd" };
        let _ = write!(
            svg,
            "<text x=\"{}\" y=\"{}\" text-anchor=\"middle\" dominant-baseline=\"middle\" font-size=\"10\" fill=\"{}\" font-family=\"monospace\">{}</text>",
            cx, cy, text_color, xml_escape(&room.label)
        );

        // Notes marker: small circle in top-right corner.
        if room.has_notes {
            let nx = rx + ROOM_W - 4;
            let ny = ry + 4;
            let _ = write!(
                svg,
                "<circle cx=\"{}\" cy=\"{}\" r=\"3\" fill=\"#fc0\"/>",
                nx, ny
            );
        }
    }

    Some((svg, width, height))
}

/// Render every non-empty layer of `graph` as one standalone SVG document, each
/// layer its own coordinate plane stacked top-to-bottom under a heading naming
/// it (SQ-1308) — the same rule [`crate::map_dump::render_dump`] draws its ASCII
/// map by.
///
/// [`render_svg`] draws a single [`RenderMap`] on one shared canvas with no
/// notion of layer at all, which [`mapper::render::render`] (as opposed to
/// [`mapper::render::render_layer`]) never distinguishes either — safe only
/// when there is exactly one layer. A room peeled onto a fresh layer keeps
/// whatever cell it already had on the layer it left
/// ([`mapper::layer::move_region`]'s doc comment), so two rooms on different
/// layers can and routinely do share a cell; drawing every layer on one canvas
/// would then draw them on top of each other. Stacking each layer's own
/// [`mapper::render::render_layer`] output avoids that by construction, since
/// each one gets its own canvas.
///
/// A single-layer graph renders exactly as `render_svg(&mapper::render::render(graph))`
/// always has — no heading, no change from before SQ-1308.
pub fn render_svg_layered(graph: &MapGraph) -> String {
    let mut layers: Vec<mapper::layer::LayerId> = graph
        .layers()
        .keys()
        .copied()
        .filter(|&l| !graph.rooms_in_layer(l).is_empty())
        .collect();
    layers.sort_unstable();
    if layers.len() <= 1 {
        return render_svg(&mapper::render::render(graph));
    }

    const HEADING_H: i32 = 24;
    const GAP: i32 = 16;
    let mut y = 0i32;
    let mut max_w = MARGIN * 2;
    let mut body = String::new();
    for &l in &layers {
        let rm = mapper::render::render_layer(graph, l);
        let heading = format!(
            "{}{} ({} rooms)",
            graph.layer_name(l),
            if graph.layer_is_maze(l) { " [maze]" } else { "" },
            graph.rooms_in_layer(l).len()
        );
        let _ = write!(
            body,
            "<text x=\"{}\" y=\"{}\" font-size=\"14\" fill=\"#ddd\" font-family=\"monospace\">{}</text>",
            MARGIN,
            y + 16,
            xml_escape(&heading)
        );
        y += HEADING_H;
        if let Some((frag, w, h)) = render_svg_body(&rm) {
            let _ = write!(body, "<g transform=\"translate(0,{y})\">{frag}</g>");
            y += h;
            max_w = max_w.max(w);
        }
        y += GAP;
    }
    let total_h = y.max(1);
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{max_w}\" height=\"{total_h}\">\
         <rect width=\"{max_w}\" height=\"{total_h}\" fill=\"#1a1a2e\"/>{body}</svg>"
    )
}

/// Write `render_svg(rm)` to the file at `path`.
pub fn export_svg(path: &Path, rm: &RenderMap) -> std::io::Result<()> {
    crate::storage::atomic_write(path, render_svg(rm).as_bytes())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-state"))]
mod tests {
    use super::*;
    use mapper::direction::Direction;
    use mapper::mapper::Mapper;
    use mapper::render::render;

    #[test]
    fn svg_contains_rooms_and_edges() {
        let mut m = Mapper::default();
        m.observe(1, "Start <Room>", None); // XML-special char in label
        m.observe(2, "North", Some(Direction::N));
        let svg = render_svg(&render(&m.graph));
        assert!(svg.starts_with("<svg") || svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
        assert!(svg.contains("<rect")); // at least one room
        assert!(svg.contains("&lt;Room&gt;")); // label XML-escaped
        assert!(svg.contains("polyline") || svg.contains("<line")); // a connector
    }

    #[test]
    fn empty_map_returns_valid_svg() {
        use mapper::graph::MapGraph;
        let g = MapGraph::new();
        let rm = render(&g);
        let svg = render_svg(&rm);
        assert!(svg.contains("<svg"), "must open svg tag");
        assert!(svg.contains("</svg>"), "must close svg tag");
        assert!(!svg.contains("<rect"), "no rooms expected");
    }

    #[test]
    fn current_room_has_distinct_style() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        // Room 2 is current (last observed).
        let svg = render_svg(&render(&m.graph));
        // Current room uses fill="#ffe" stroke="#c00"
        assert!(svg.contains("#ffe"), "current room fill must be #ffe");
        assert!(svg.contains("#c00"), "current room stroke must be #c00");
    }

    #[test]
    fn xml_escape_covers_all_specials() {
        assert_eq!(xml_escape("a & b"), "a &amp; b");
        assert_eq!(xml_escape("<tag>"), "&lt;tag&gt;");
        assert_eq!(xml_escape("\"quoted\""), "&quot;quoted&quot;");
        assert_eq!(xml_escape("it's"), "it&#39;s");
        assert_eq!(xml_escape("plain"), "plain");
    }
}
