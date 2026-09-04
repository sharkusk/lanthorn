//! Per-room diagnostics: the room dock's second body.
//!
//! `room_diagnostics` gathers layout info for one room from the public `MapGraph` API (app-side
//! only — mapper internals are not touched). `draw_diagnostics_body` draws it into a plain `Rect`.
//!
//! SQ-0692 retired the floating corner dialog this used to be (`draw_inspector`): it is a BODY of
//! the room dock now, sharing that dock's chrome, room selection and follow/pin regime with the
//! Info body. `/toggle-inspector` flips between the two rather than opening a second panel.
//!
//! # Future extension
//! "Corrections made during cleanup" (which rooms were nudged by the overlap cleanup pass, and
//! in which direction) are not recorded anywhere today. Surfacing a per-room cleanup history would
//! require the mapper to emit events or keep a log; that is left as a future extension.

use mapper::graph::{MapGraph, RoomId};
use mapper::direction::Direction;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::draw_str_clipped;
use crate::theme::resolve::Theme;

// ── Data ──────────────────────────────────────────────────────────────────────

/// One outgoing edge of the selected room as seen by the inspector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeInfo {
    pub dir: Direction,
    pub neighbour_id: RoomId,
    pub neighbour_name: String,
    pub distorted: bool,
}

/// All layout diagnostics for one room, computed app-side from `MapGraph`.
#[derive(Debug, Clone)]
pub struct RoomDiagnostics {
    pub id: RoomId,
    pub name: String,
    pub layer_id: mapper::layer::LayerId,
    pub layer_name: String,
    pub pos: Option<(i32, i32)>,
    pub edges: Vec<EdgeInfo>,
    /// Total outgoing edges (length of `edges`).
    pub edge_count: usize,
    /// Number of outgoing edges with `distorted == true`.
    pub distorted_count: usize,
    /// How the mapper first worked out the player was here (SQ-0527), recorded on
    /// the room at discovery. `None` for rooms mapped before it was recorded.
    pub loc_method: Option<String>,
}

/// Gather layout diagnostics for `id` from the public `MapGraph` API.
///
/// Returns `None` if the room does not exist in the graph.
pub fn room_diagnostics(graph: &MapGraph, id: RoomId) -> Option<RoomDiagnostics> {
    let room = graph.room(id)?;
    let layer_id = graph.layer_of(id);
    let layer_name = graph.layer_name(layer_id).to_owned();
    let pos = room.pos;
    let name = room.label().to_owned();

    let edges: Vec<EdgeInfo> = graph
        .connections()
        .iter()
        .filter(|c| c.origin == id)
        .map(|c| {
            let neighbour_name = graph
                .room(c.dest)
                .map(|r| r.label().to_owned())
                .unwrap_or_else(|| format!("#{}", c.dest));
            EdgeInfo {
                dir: c.dir,
                neighbour_id: c.dest,
                neighbour_name,
                distorted: c.distorted,
            }
        })
        .collect();

    let edge_count = edges.len();
    let distorted_count = edges.iter().filter(|e| e.distorted).count();

    let loc_method = room.loc_method.clone();
    Some(RoomDiagnostics { id, name, layer_id, layer_name, pos, edges, edge_count, distorted_count, loc_method })
}

// ── Render ────────────────────────────────────────────────────────────────────

/// Build the Diagnostics body's full content as logical rows, top to bottom — independent of how
/// many of them a scrolled dock can actually show (SQ-1280). [`draw_diagnostics_body`] windows
/// this by `scroll_offset`; the row count is also the total this body needs, for the caller's
/// scrollbar and [`crate::list_scroll::ListScroll`].
fn build_diagnostics_rows(diag: &RoomDiagnostics, theme: &Theme, body: Style, heading: Style) -> Vec<(String, Style)> {
    let distorted_style = theme.get("inspector_edge_distorted").style;
    let ok_style = theme.get("inspector_edge_ok").style;
    let label_style = heading;
    let value_style = body;

    let pos_str = match diag.pos {
        Some((px, py)) => format!("({}, {})", px, py),
        None => "unplaced".to_owned(),
    };

    let mut rows = Vec::new();
    rows.push((format!("#{} {}", diag.id, diag.name), label_style));
    rows.push((format!("Layer {} \"{}\"", diag.layer_id, diag.layer_name), value_style));
    rows.push((format!("Pos {}", pos_str), value_style));
    // How this room was first detected (SQ-0527). Kept on the room, so it is
    // still here long after the turn that discovered it — which the old map-corner
    // indicator never was.
    if let Some(m) = diag.loc_method.as_deref() {
        rows.push((format!("Found by {m}"), value_style));
    }
    rows.push((String::new(), value_style)); // blank separator

    for edge in &diag.edges {
        let flag = if edge.distorted { "!" } else { " " };
        let line = format!("{} {:?} {} {}", flag, edge.dir, edge.neighbour_id, edge.neighbour_name);
        let style = if edge.distorted { distorted_style } else { ok_style };
        rows.push((line, style));
    }

    rows.push((String::new(), value_style)); // blank separator
    let summary = format!(
        "{} edge{}, {} distorted",
        diag.edge_count,
        if diag.edge_count == 1 { "" } else { "s" },
        diag.distorted_count,
    );
    rows.push((summary, label_style));

    rows
}

/// Draw the diagnostics body into `area` — no chrome, no borders: the caller (the room dock) owns
/// those.
///
/// `theme` supplies the shared `inspector_edge_ok` / `inspector_edge_distorted` selectors;
/// `body` / `heading` are the styles for ordinary lines and for the id/summary lines.
/// `scroll_offset` is rows of content already scrolled past (SQ-1280), clamped here so a stale or
/// out-of-range offset can never draw garbage or leave a trailing gap; a themed scrollbar
/// (`scrollbar` / `scrollbar_track`, the same selectors every other scrollable list in the app
/// already uses) takes the rightmost column when the content overflows `area`.
///
/// It no longer draws a compass rose (SQ-0666): per-direction exploration is one fact, and the
/// room-info card and the matrix view's `×`/`·` cells both say it, in a form that also names where
/// each direction goes. The rose was the third dialect for the same knowledge and the least
/// informative of the three.
///
/// Returns the body's total row count, for the caller to keep its `ListScroll` in sync — a room
/// with many edges can run well past `area`'s height.
pub fn draw_diagnostics_body(
    diag: &RoomDiagnostics,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    body: Style,
    heading: Style,
    scroll_offset: u16,
) -> u16 {
    if area.width == 0 || area.height == 0 {
        return 0;
    }

    let rows = build_diagnostics_rows(diag, theme, body, heading);
    let total = rows.len() as u16;
    let viewport = area.height;

    let scrollbar_visible =
        crate::render::scroll::needs_scrollbar(total as usize, viewport as usize) && area.width >= 2;
    let text_w = if scrollbar_visible { area.width - 1 } else { area.width };
    let clip = Rect::new(area.x, area.y, text_w, area.height);
    let offset = scroll_offset.min(total.saturating_sub(viewport));

    for (i, (text, style)) in rows.iter().enumerate().skip(offset as usize).take(viewport as usize) {
        let y = area.y + (i as u16 - offset);
        draw_str_clipped(buf, area.x, y, text, *style, clip);
    }

    if scrollbar_visible {
        let sb_area = Rect::new(area.right() - 1, area.y, 1, area.height);
        let look = crate::render::scroll::ScrollbarLook::from_theme(theme);
        crate::render::scroll::draw_scrollbar(buf, sb_area, total as usize, viewport as usize, offset as usize, look);
    }

    total
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use mapper::graph::MapGraph;
    use mapper::layout::relayout_auto;
    use mapper::direction::Direction;
    use ratatui::style::{Color, Style};

    fn test_theme() -> Theme {
        crate::colors::ColorScheme::terminal_default().theme
    }

    // ── room_diagnostics tests ────────────────────────────────────────────────

    #[test]
    fn diagnostics_none_for_missing_room() {
        let g = MapGraph::new();
        assert!(room_diagnostics(&g, 99).is_none());
    }

    #[test]
    fn diagnostics_id_name_layer_pos() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "West of House".into());
        g.set_pos(1, (3, -2));
        let d = room_diagnostics(&g, 1).unwrap();
        assert_eq!(d.id, 1);
        assert_eq!(d.name, "West of House");
        assert_eq!(d.layer_id, mapper::layer::MAIN_LAYER);
        assert_eq!(d.layer_name, "Main");
        assert_eq!(d.pos, Some((3, -2)));
        assert_eq!(d.edge_count, 0);
        assert_eq!(d.distorted_count, 0);
    }

    #[test]
    fn diagnostics_unplaced_room_has_none_pos() {
        let mut g = MapGraph::new();
        g.upsert_room(5, "Nowhere".into());
        let d = room_diagnostics(&g, 5).unwrap();
        assert_eq!(d.pos, None);
    }

    #[test]
    fn diagnostics_edges_and_distorted_flag() {
        // Build a two-room graph where the E edge from 1 to 2 becomes distorted
        // by placing 2 *west* of 1 (violating the eastward hint) then calling mark_distorted.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::E, 2);
        // Place 2 to the WEST of 1 so the E edge is geometrically violated.
        g.set_pos(1, (5, 0));
        g.set_pos(2, (0, 0)); // 2 is west of 1 → E edge is distorted
        // Force the distorted flag manually (as mark_distorted / relayout_auto would).
        g.set_conn_distorted(0, true);

        let d = room_diagnostics(&g, 1).unwrap();
        assert_eq!(d.edge_count, 1);
        assert_eq!(d.distorted_count, 1);
        let e = &d.edges[0];
        assert_eq!(e.dir, Direction::E);
        assert_eq!(e.neighbour_id, 2);
        assert_eq!(e.neighbour_name, "B");
        assert!(e.distorted);
    }

    #[test]
    fn diagnostics_after_relayout_marks_correct_distorted() {
        // A two-room reciprocal N/S graph: after relayout_auto the edge must NOT be distorted
        // (2 ends up north of 1 as expected).
        let mut g = MapGraph::new();
        g.upsert_room(1, "Root".into());
        g.upsert_room(2, "North Room".into());
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1);
        relayout_auto(&mut g);

        let d = room_diagnostics(&g, 1).unwrap();
        // The N edge from 1 to 2 should be satisfied → not distorted.
        let north_edge = d.edges.iter().find(|e| e.dir == Direction::N).unwrap();
        assert!(!north_edge.distorted, "satisfied N edge must not be distorted after relayout");
        assert_eq!(d.distorted_count, 0);
    }

    #[test]
    fn diagnostics_distorted_loop_marks_at_least_one() {
        // An impossible 3-room northward loop: at least one edge must be distorted.
        let mut g = MapGraph::new();
        for id in 1u16..=3 { g.upsert_room(id.into(), "r".into()); }
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::N, 3);
        g.add_edge(3, Direction::N, 1); // closes an impossible loop
        relayout_auto(&mut g);
        // At least one of the three rooms must report a distorted outgoing edge.
        let any_distorted = [1u16, 2, 3].iter().any(|&id| {
            room_diagnostics(&g, id.into()).map(|d| d.distorted_count > 0).unwrap_or(false)
        });
        assert!(any_distorted, "impossible loop must leave at least one distorted edge");
    }

    #[test]
    fn diagnostics_layer_name_for_non_main_layer() {
        let mut g = MapGraph::new();
        let l = g.new_layer(Some(mapper::layer::MAIN_LAYER), "Basement".into());
        g.upsert_room(7, "Cellar".into());
        g.set_room_layer(7, l);
        let d = room_diagnostics(&g, 7).unwrap();
        assert_eq!(d.layer_id, l);
        assert_eq!(d.layer_name, "Basement");
    }


    // ── draw_diagnostics_body render tests ────────────────────────────────────

    fn make_diag(id: RoomId, name: &str, edges: Vec<EdgeInfo>) -> RoomDiagnostics {
        let edge_count = edges.len();
        let distorted_count = edges.iter().filter(|e| e.distorted).count();
        RoomDiagnostics {
            id,
            name: name.to_owned(),
            layer_id: 0,
            layer_name: "Main".to_owned(),
            pos: Some((1, 2)),
            edges,
            edge_count,
            distorted_count,
            loc_method: Some("via status variable".to_owned()),
        }
    }

    fn buf_contains(buf: &ratatui::buffer::Buffer, s: &str) -> bool {
        let all: String = buf.content().iter().map(|c| c.symbol().to_owned()).collect();
        all.contains(s)
    }

    /// Draw the body into a plain rect the way the room dock does.
    fn render_body(diag: &RoomDiagnostics, w: u16, h: u16) -> Buffer {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let theme = test_theme();
        draw_diagnostics_body(diag, area, &mut buf, &theme, Style::default(), Style::default(), 0);
        buf
    }

    /// Like [`render_body`], but at a given scroll offset — for the SQ-1280 scroll tests. Returns
    /// the buffer alongside the total row count `draw_diagnostics_body` reported.
    fn render_body_scrolled(diag: &RoomDiagnostics, w: u16, h: u16, scroll_offset: u16) -> (Buffer, u16) {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let theme = test_theme();
        let total = draw_diagnostics_body(diag, area, &mut buf, &theme, Style::default(), Style::default(), scroll_offset);
        (buf, total)
    }

    #[test]
    fn diagnostics_body_renders_room_id_name_and_detection_method() {
        let diag = make_diag(42, "Clearing", vec![]);
        let buf = render_body(&diag, 60, 20);
        assert!(buf_contains(&buf, "42"), "should contain room id");
        assert!(buf_contains(&buf, "Clearing"), "should contain room name");
        // SQ-0527: and how the mapper first found the room. This used to be a
        // transient corner indicator describing only the LAST detection, gone by
        // the time you wanted to know about a given room.
        assert!(
            buf_contains(&buf, "Found by via status variable"),
            "the diagnostics body names the detection method recorded on the room"
        );
        assert!(buf_contains(&buf, "Pos (1, 2)"), "and the grid position");
        assert!(buf_contains(&buf, "0 edges, 0 distorted"), "and the edge summary");
    }

    #[test]
    fn diagnostics_body_in_a_tiny_area_does_not_panic() {
        let diag = make_diag(1, "A", vec![]);
        render_body(&diag, 3, 1);
        // Zero-area is a no-op, not a panic.
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        let theme = test_theme();
        draw_diagnostics_body(&diag, Rect::new(0, 0, 0, 0), &mut buf, &theme, Style::default(), Style::default(), 0);
    }

    /// SQ-0391's compass rose was retired by SQ-0666, and this is the test that used to pin
    /// it — flipped rather than deleted, because the FACT it covered is still displayed, just
    /// somewhere better. "Which ways out of this room have I explored?" is now answered by the
    /// room-info card and the matrix's `×`/`·` cells, both of which also say where each direction
    /// goes; the rose said only "some letter is capitalised".
    #[test]
    fn the_diagnostics_body_no_longer_draws_a_compass_rose() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "West of House".into());
        g.upsert_room(2, "North of House".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::N, 2); // walked north
        g.mark_tried(1, Direction::E); // typed east, went nowhere
        let diag = room_diagnostics(&g, 1).unwrap();
        let buf = render_body(&diag, 44, 24);

        assert!(!buf_contains(&buf, "nw"), "the rose's diagonal row is gone");
        assert!(!buf_contains(&buf, "sw"), "and its bottom row");
        assert!(!buf_contains(&buf, "\u{2299} \u{2297}"), "and its in/out portal pair");
        // What the body is FOR is untouched: the room, and where its edges go.
        assert!(buf_contains(&buf, "West of House"));
        assert!(buf_contains(&buf, "North of House"), "the N edge is still listed");

        // And the underlying knowledge is not lost — the matrix reads it straight from the graph.
        use mapper::matrix::{classify, MatrixCell};
        assert_eq!(classify(&g, 1, Direction::E), MatrixCell::Probed, "east: tried, no path");
        assert_eq!(classify(&g, 1, Direction::W), MatrixCell::Untried, "west: never tried");
    }

    #[test]
    fn diagnostics_body_shows_distorted_edge() {
        let edges = vec![EdgeInfo {
            dir: Direction::E,
            neighbour_id: 99,
            neighbour_name: "Maze".into(),
            distorted: true,
        }];
        let diag = make_diag(1, "Start", edges);
        let buf = render_body(&diag, 60, 20);
        assert!(buf_contains(&buf, "!"), "distorted edge should show '!'");
        assert!(buf_contains(&buf, "1 edge, 1 distorted"), "and be counted in the summary");
    }

    /// SQ-0643: `distorted_style`/`ok_style` used to be bare `Style::default().fg(Red/Green)`
    /// literals no `style.toml` could reach (and colorblind-hostile as a bonus). They must
    /// read `inspector_edge_distorted`/`inspector_edge_ok`, and an override must actually
    /// change what's drawn — still true now the body draws into the dock (SQ-0692).
    #[test]
    fn diagnostics_edge_colours_follow_style_overrides() {
        let scheme = crate::colors::GhosttyScheme::default();
        let parsed = crate::theme::toml_schema::parse(
            "[elements]\ninspector_edge_ok = { fg = \"magenta\" }\ninspector_edge_distorted = { fg = \"blue\" }\n",
        ).unwrap();
        let theme = crate::theme::resolve::resolve_theme(&scheme, &parsed);
        let edges = vec![
            EdgeInfo { dir: Direction::E, neighbour_id: 2, neighbour_name: "OK Room".into(), distorted: false },
            EdgeInfo { dir: Direction::W, neighbour_id: 3, neighbour_name: "Bad Room".into(), distorted: true },
        ];
        let diag = make_diag(1, "Start", edges);
        let area = Rect::new(0, 0, 60, 24);
        let mut buf = Buffer::empty(area);
        draw_diagnostics_body(&diag, area, &mut buf, &theme, Style::default(), Style::default(), 0);

        let has = |c: Color| (0..area.width).flat_map(|x| (0..area.height).map(move |y| (x, y)))
            .any(|(x, y)| buf.cell((x, y)).is_some_and(|cell| cell.style().fg == Some(c)));
        assert!(has(Color::Magenta), "an OK edge row must render in the overridden inspector_edge_ok colour");
        assert!(has(Color::Blue), "a distorted edge row must render in the overridden inspector_edge_distorted colour");
    }

    // ── Scrolling (SQ-1280) ───────────────────────────────────────────────────

    /// A room with many edges draws the first N rows at offset 0, and scrolling reveals the LAST
    /// row (the summary line) at the maximum offset — never past it, and a too-large offset
    /// clamps rather than drawing garbage or leaving a blank window.
    #[test]
    fn scrolling_reveals_edges_below_the_fold_and_clamps_at_the_end() {
        let edges: Vec<EdgeInfo> = (0..20)
            .map(|i| EdgeInfo {
                dir: Direction::E,
                neighbour_id: i,
                neighbour_name: format!("Room {i:02}"),
                distorted: false,
            })
            .collect();
        let diag = make_diag(1, "Hub", edges);
        let (width, height) = (40u16, 4u16);

        let (top, total) = render_body_scrolled(&diag, width, height, 0);
        assert!(total > height, "twenty edges overflow a 4-row body: {total}");
        assert!(buf_contains(&top, "Hub"), "the header rows are visible at offset 0");
        assert!(!buf_contains(&top, "Room 00"), "the first edge is well below the fold at offset 0");
        assert!(!buf_contains(&top, "20 edges, 0 distorted"), "the summary is far below the fold at offset 0");

        let max_offset = total - height;
        let (bottom, total_again) = render_body_scrolled(&diag, width, height, max_offset);
        assert_eq!(total_again, total, "the same content reports the same total");
        assert!(buf_contains(&bottom, "20 edges, 0 distorted"), "the summary reaches the bottom at the max offset");
        assert!(!buf_contains(&bottom, "Room 00"), "the first edge has scrolled off the top");

        // Scrolling PAST the end (or requesting an offset larger than the content allows) clamps
        // to the same maximum — it is a no-op past the end, not a blank window.
        let (past_end, _) = render_body_scrolled(&diag, width, height, max_offset + 50);
        assert_eq!(
            past_end.content().iter().map(|c| c.symbol().to_owned()).collect::<String>(),
            bottom.content().iter().map(|c| c.symbol().to_owned()).collect::<String>(),
            "an over-large offset clamps to the maximum, not past it",
        );
    }

    /// A body that fits entirely in the dock never shows a scrollbar column, however far past
    /// the end a (meaningless) offset is requested.
    #[test]
    fn a_body_that_fits_draws_no_scrollbar_and_ignores_any_offset() {
        let diag = make_diag(1, "Start", vec![]);
        let (fits, total) = render_body_scrolled(&diag, 60, 20, 0);
        assert!(total < 20, "the content fits well inside 20 rows: {total}");
        let (scrolled, _) = render_body_scrolled(&diag, 60, 20, 99);
        assert_eq!(
            fits.content().iter().map(|c| c.symbol().to_owned()).collect::<String>(),
            scrolled.content().iter().map(|c| c.symbol().to_owned()).collect::<String>(),
            "there is nothing to scroll, so an offset changes nothing",
        );
    }

    /// The indicator itself: a themed scrollbar (the SAME `scrollbar`/`scrollbar_track`
    /// selectors every other scrollable list in the app already uses — SQ-1280 adds no new
    /// selector) appears in the rightmost column only when the body actually overflows.
    #[test]
    fn the_scrollbar_appears_only_when_the_body_overflows_and_uses_the_shared_selectors() {
        let parsed = crate::theme::toml_schema::parse(
            "[elements]\nscrollbar = { fg = \"magenta\" }\nscrollbar_track = { fg = \"yellow\" }\n",
        )
        .unwrap();
        let theme = crate::theme::resolve::resolve_theme(&crate::colors::GhosttyScheme::default(), &parsed);
        let edges: Vec<EdgeInfo> = (0..20)
            .map(|i| EdgeInfo { dir: Direction::E, neighbour_id: i, neighbour_name: "R".into(), distorted: false })
            .collect();
        let diag = make_diag(1, "Hub", edges);

        let has_bg = |buf: &Buffer, area: Rect, c: Color| {
            (0..area.width).flat_map(|x| (0..area.height).map(move |y| (x, y)))
                .any(|(x, y)| buf.cell((x, y)).is_some_and(|cell| cell.bg == c))
        };

        // Overflowing: the rightmost column carries the themed bar.
        let area = Rect::new(0, 0, 40, 4);
        let mut buf = Buffer::empty(area);
        let total = draw_diagnostics_body(&diag, area, &mut buf, &theme, Style::default(), Style::default(), 0);
        assert!(total > area.height, "twenty edges overflow a 4-row body: {total}");
        assert!(has_bg(&buf, area, Color::Magenta), "the thumb uses the shared `scrollbar` selector");
        assert!(has_bg(&buf, area, Color::Yellow), "the track uses the shared `scrollbar_track` selector");

        // Fits: no bar anywhere.
        let area = Rect::new(0, 0, 40, 40);
        let mut buf = Buffer::empty(area);
        let total = draw_diagnostics_body(&diag, area, &mut buf, &theme, Style::default(), Style::default(), 0);
        assert!(total <= area.height, "content fits a 40-row body: {total}");
        assert!(!has_bg(&buf, area, Color::Magenta), "no thumb when nothing overflows");
        assert!(!has_bg(&buf, area, Color::Yellow), "no track when nothing overflows");
    }
}
