//! The room dock: one panel, docked at the bottom of the map pane, describing
//! one room (SQ-0692).
//!
//! It replaced two floating corner dialogs — Room Info (left-click) and the
//! diagnostics Inspector (right-click / `/toggle-inspector`) — which each
//! obscured the map they described, counted as a modal overlay, and never
//! followed the player. The dock reserves its own rows out of the map pane
//! instead, so it covers nothing, and it has two BODIES rather than two panels:
//!
//! - **Info** — the room's notes, its exit card in the matrix vocabulary, and
//!   (for the current room only) the objects the engine can see there.
//! - **Diagnostics** — id, layer, grid position, edges with distortion flags,
//!   discovery method.
//!
//! **Follow by default, pin on click.** With no room selected the dock describes
//! `graph.current()` and updates every move; a selected room pins it. Pin state
//! IS the selection (`state.selected_room`), so the map highlight, the matrix
//! cross-highlight and the dock header always agree.
//!
//! Like the inventory dock, the caller sizes `area` from the animated
//! `PanelSlide` fraction, so `area` may be shorter than the target height while
//! a slide is in flight — everything here clips to `area`.

use mapper::graph::{MapGraph, RoomId};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::draw_str_clipped;
use super::paneframe::InsetSegment;
use crate::colors::ColorScheme;
use crate::render::panel::{draw_panel, PanelSpec, PanelStrip};
use crate::state::RoomDockView;
use crate::symbols::SymbolSet;

/// Rows the dock refuses to shrink below: 2 border rows, the header line and two
/// body lines. Below that it says nothing a glance can use.
pub const MIN_ROOM_DOCK_ROWS: u16 = 5;

/// Rows the MAP pane keeps no matter how tall the dock is asked to be: its two
/// border rows plus one row of map. A dock that can starve the pane it lives in
/// is a dock you cannot drag back.
pub const MIN_MAP_ROWS: u16 = 3;


/// The dock's fully-open target height in rows: `pct`% of the frame, floored at
/// [`MIN_ROOM_DOCK_ROWS`] and capped so the map pane keeps [`MIN_MAP_ROWS`].
///
/// A map pane too short to host both is left to the map entirely — the dock
/// reports zero rather than squeezing into a sliver.
pub fn room_dock_target_height(map_height: u16, frame_height: u16, pct: u16) -> u16 {
    if map_height <= MIN_MAP_ROWS + MIN_ROOM_DOCK_ROWS {
        return 0;
    }
    let want = ((frame_height as u32 * pct as u32) / 100) as u16;
    want.max(MIN_ROOM_DOCK_ROWS)
        .min(map_height.saturating_sub(MIN_MAP_ROWS))
}

/// The reserved dock band height: `target_h` scaled by the slide's current
/// `fraction` (0.0 closed .. 1.0 fully open), rounded to the nearest row.
pub fn room_dock_height(target_h: u16, fraction: f64) -> u16 {
    (target_h as f64 * fraction).round() as u16
}

/// Which room the dock describes: the selected (pinned) room, else the room the
/// player is standing in. `None` when neither exists — the map has not placed
/// the player anywhere yet.
pub fn dock_room(selected: Option<RoomId>, graph: &MapGraph) -> Option<RoomId> {
    selected.or_else(|| graph.current())
}

/// The header line for `room`: its display name, its layer, and which regime the
/// dock is in. The name is the matrix-NUMBERED form ("Maze 4") whenever the
/// layer numbers it, so the dock, the matrix table and the exit card all call
/// the same room the same thing.
/// The two regime markers come from the player's [`SymbolSet`] (`dock.following`
/// / `dock.pinned` in `style.toml`) rather than from constants here. They were
/// `U+2316` and `U+2299`, and the second is the glyph SQ-0989 took off the map
/// for being undrawable in Fira Code — the dock was the other place it lived.
pub fn header_line(
    graph: &MapGraph,
    room: Option<RoomId>,
    pinned: bool,
    symbols: &SymbolSet,
) -> String {
    let marker = if pinned {
        format!("{} pinned", symbols.dock_pinned)
    } else {
        format!("{} following", symbols.dock_following)
    };
    match room.and_then(|id| graph.room(id).map(|_| id)) {
        Some(id) => {
            let layer = graph.layer_of(id);
            format!(
                "{} \u{b7} {}  {}",
                crate::render::room_info::display_name(graph, id),
                graph.layer_name(layer),
                marker,
            )
        }
        // No current room and nothing pinned: say so rather than draw a blank
        // dock the player has to guess at.
        None => format!("nowhere yet  {marker}"),
    }
}

/// The dock's hit-rects from one draw: the title-strip tabs, and the close box
/// (SQ-1265) when the frame was wide enough to draw one.
pub struct RoomDockRects {
    /// A click on "Room"/"Diagnostics" switches the view the same way a click
    /// on a layer tab switches layers.
    pub tabs: Vec<(RoomDockView, Rect)>,
    /// A click here closes the dock — the same effect as `toggle-room-panel`
    /// while it is open. `None` when the frame was too narrow to draw one.
    pub close: Option<Rect>,
    /// The ACTIVE body's total row count this frame (SQ-1280) — 0 when no body was
    /// drawn (missing room, zero area). The caller syncs its `ListScroll` against
    /// this after the draw returns, the same way it already tracks
    /// `modal_list_viewport`.
    pub body_total: u16,
    /// The active body's own viewport height (rows below the header, borders
    /// excluded) this frame — 0 under the same conditions as `body_total`.
    pub body_viewport: u16,
}

/// Draw the room dock into `area`.
///
/// - `room` is the resolved room ([`dock_room`]); `pinned` is `selected_room.is_some()`.
/// - `room_objects` are the engine's live objects for the CURRENT room (empty
///   when introspection is unavailable); `current_room` gates their display.
/// - `highlighted` is true when resize mode targets this dock or the pointer is
///   on its top edge — the same accent every other pane boundary uses.
/// - `scroll_offset` is the ACTIVE body's scroll position in rows (SQ-1280),
///   already read from the caller's `ListScroll` for `view` — the body draw
///   clamps it defensively, so a stale offset (the room just changed, say)
///   never draws garbage.
///
/// Returns the title-strip hit-rects, so a click on "Room"/"Diagnostics" switches
/// the view the same way a click on a layer tab switches layers, plus the close
/// box's rect and the active body's row totals for the caller's scroll sync.
#[allow(clippy::too_many_arguments)]
pub fn draw_room_dock(
    graph: &MapGraph,
    room: Option<RoomId>,
    pinned: bool,
    view: RoomDockView,
    room_objects: &[String],
    current_room: Option<RoomId>,
    area: Rect,
    colors: &ColorScheme,
    symbols: &SymbolSet,
    highlighted: bool,
    scroll_offset: u16,
    buf: &mut Buffer,
) -> RoomDockRects {
    if area.width == 0 || area.height == 0 {
        return RoomDockRects { tabs: Vec::new(), close: None, body_total: 0, body_viewport: 0 };
    }
    let style = colors.theme.get("room_panel").style;
    let header_style = colors
        .theme
        .get(if pinned { "room_panel.header:pinned" } else { "room_panel.header" })
        .style;
    // Section headings inside the body always use the unpinned header selector:
    // the pinned variant marks ONE line — the header — and a body that changed
    // colour on pin would say nothing extra while shouting twice as loud.
    let heading_style = colors.theme.get("room_panel.header").style;
    let border_selector = if highlighted { "panel.border:active" } else { "panel.border" };
    let border_color =
        if highlighted { colors.theme.get("panel.border:active").style } else { style };

    // Fill first so the map never shows through a mid-slide (short) dock.
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(style);
            }
        }
    }

    // The two views are named in the title strip, active one accented — the same
    // vocabulary the map pane's layer tabs use, so "this panel has two of these"
    // reads the same way in both places.
    let segments = [
        InsetSegment { text: "Room", active: view == RoomDockView::Info },
        InsetSegment { text: "Diagnostics", active: view == RoomDockView::Diagnostics },
    ];
    let spec = PanelSpec {
        area,
        border_selector,
        border_color: Some(border_color),
        border_style: None,
        // The MAP pane's glyphs, not the default set: the dock is carved out of that pane and
        // sits flush under it, so a user who restyles the map's border must not end up with two
        // stacked panes drawn in two different hands (SQ-0694).
        glyphs: &colors.map_border_glyphs,
        header_on: true,
        strip: Some(PanelStrip {
            segments: &segments,
            base: colors.theme.get("panel.tab").style,
            active: colors.theme.get("panel.tab:active").style,
        }),
        body_fill: None,
    };
    let frame = draw_panel(buf, &spec, &colors.theme);
    let tabs: Vec<(RoomDockView, Rect)> = [RoomDockView::Info, RoomDockView::Diagnostics]
        .into_iter()
        .zip(frame.tab_rects)
        .collect();

    // The close box: same glyph, same "just inside the top-right border" spot,
    // and the same reused border style `draw_dialog`'s `show_close` uses rather
    // than a selector of its own (SQ-1265) — drawn LAST so it always wins the
    // corner cell over the tab strip's own border fill.
    let close = if area.width >= 3 {
        let cx = area.right().saturating_sub(2);
        let cy = area.y;
        if let Some(cell) = buf.cell_mut((cx, cy)) {
            cell.set_symbol("✕").set_style(border_color);
        }
        Some(Rect::new(cx, cy, 1, 1))
    } else {
        None
    };

    let content = frame.content;
    if content.height == 0 || content.width == 0 {
        return RoomDockRects { tabs, close, body_total: 0, body_viewport: 0 };
    }

    draw_str_clipped(
        buf,
        content.x,
        content.y,
        &header_line(graph, room, pinned, symbols),
        header_style,
        content,
    );

    let body = Rect::new(
        content.x,
        content.y + 1,
        content.width,
        content.height.saturating_sub(1),
    );
    if body.height == 0 {
        return RoomDockRects { tabs, close, body_total: 0, body_viewport: 0 };
    }

    let Some(id) = room.filter(|id| graph.room(*id).is_some()) else {
        draw_str_clipped(
            buf,
            body.x,
            body.y,
            "The map has not placed you in a room yet.",
            style,
            body,
        );
        return RoomDockRects { tabs, close, body_total: 0, body_viewport: 0 };
    };

    let body_total = match view {
        RoomDockView::Info => super::room_info::draw_room_info_body(
            graph,
            room_objects,
            id,
            current_room,
            body,
            buf,
            &colors.theme,
            style,
            heading_style,
            scroll_offset,
        ),
        RoomDockView::Diagnostics => super::inspector::room_diagnostics(graph, id)
            .map(|diag| {
                super::inspector::draw_diagnostics_body(
                    &diag,
                    body,
                    buf,
                    &colors.theme,
                    style,
                    heading_style,
                    scroll_offset,
                )
            })
            .unwrap_or(0),
    };

    RoomDockRects { tabs, close, body_total, body_viewport: body.height }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use mapper::direction::Direction;
    use ratatui::style::Color;

    fn buf_text(buf: &Buffer) -> String {
        buf.content().iter().map(|c| c.symbol().to_owned()).collect()
    }

    /// SQ-0989's other half: the dock's two regime markers are the player's, and
    /// the defaults are glyphs a shipped face can actually draw.
    ///
    /// They were `U+2316` and `U+2299`, hard-coded — the second being the very
    /// glyph that quest took off the map for being absent from Fira Code, which
    /// left it drawn in the dock two panes away. Measured with
    /// `fc-list ":charset=NNNN" file`: no FiraCode face carries 2316 or 2299;
    /// thirteen carry 25C6 and 25C7.
    #[test]
    fn the_docks_regime_markers_are_themeable_and_drawable() {
        let g = graph_with_current();

        let d = SymbolSet::default();
        assert!(
            ('\u{25A0}'..='\u{25FF}').contains(&d.dock_following)
                && ('\u{25A0}'..='\u{25FF}').contains(&d.dock_pinned),
            "both defaults come from Geometric Shapes, the block the map already needs",
        );
        assert_ne!(d.dock_following, d.dock_pinned, "the two regimes must read apart");

        // The player's own glyphs reach the header, both ways round.
        let mine = SymbolSet { dock_following: '~', dock_pinned: '!', ..SymbolSet::default() };
        let following = header_line(&g, Some(1), false, &mine);
        let pinned = header_line(&g, Some(1), true, &mine);
        assert!(following.contains("~ following"), "{following}");
        assert!(pinned.contains("! pinned"), "{pinned}");
        assert!(
            !following.contains(d.dock_following) && !pinned.contains(d.dock_pinned),
            "the shipped preset must not survive an override: {following} / {pinned}",
        );

        // …including the room-less header, which is a separate format arm.
        let nowhere = header_line(&MapGraph::new(), None, false, &mine);
        assert!(nowhere.contains("~ following"), "{nowhere}");
    }

    fn graph_with_current() -> MapGraph {
        let mut g = MapGraph::new();
        g.upsert_room(1, "West of House".into());
        g.upsert_room(2, "Forest Path".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        g.set_current(1);
        g
    }

    #[test]
    fn target_height_floors_at_min_rows_and_caps_so_the_map_survives() {
        // 33% of a 40-row frame is 13, and a 30-row map pane can spare it.
        assert_eq!(room_dock_target_height(30, 40, 33), 13);
        // A tiny percentage still gets the readable minimum.
        assert_eq!(room_dock_target_height(30, 40, 1), MIN_ROOM_DOCK_ROWS);
        // A greedy percentage is capped so the map keeps MIN_MAP_ROWS.
        assert_eq!(room_dock_target_height(20, 40, 80), 20 - MIN_MAP_ROWS);
        // A map pane with no room for both keeps every row.
        assert_eq!(room_dock_target_height(MIN_MAP_ROWS + MIN_ROOM_DOCK_ROWS, 40, 33), 0);
        assert_eq!(room_dock_target_height(0, 40, 33), 0);
    }

    #[test]
    fn height_scales_with_the_slide_fraction() {
        assert_eq!(room_dock_height(10, 0.0), 0);
        assert_eq!(room_dock_height(10, 0.5), 5);
        assert_eq!(room_dock_height(10, 1.0), 10);
    }

    #[test]
    fn dock_room_follows_current_until_a_selection_pins_it() {
        let g = graph_with_current();
        assert_eq!(dock_room(None, &g), Some(1), "no selection: follow the current room");
        assert_eq!(dock_room(Some(2), &g), Some(2), "a selection pins the dock to it");
        let empty = MapGraph::new();
        assert_eq!(dock_room(None, &empty), None, "nothing current, nothing pinned");
    }

    #[test]
    fn header_states_the_room_its_layer_and_the_regime() {
        let g = graph_with_current();
        let following = header_line(&g, Some(1), false, &SymbolSet::default());
        assert!(following.contains("West of House"), "{following}");
        assert!(following.contains("Main"), "the layer is named: {following}");
        assert!(following.contains("following"), "{following}");
        assert!(!following.contains("pinned"), "{following}");

        let pinned = header_line(&g, Some(2), true, &SymbolSet::default());
        assert!(pinned.contains("Forest Path") && pinned.contains("pinned"), "{pinned}");
        assert!(!pinned.contains("following"), "{pinned}");

        let nowhere = header_line(&MapGraph::new(), None, false, &SymbolSet::default());
        assert!(nowhere.contains("nowhere yet"), "{nowhere}");
    }

    #[test]
    fn info_body_draws_the_exit_card_diagnostics_body_draws_the_edges() {
        let g = graph_with_current();
        let area = Rect::new(0, 0, 50, 20);
        let colors = ColorScheme::default();

        let mut buf = Buffer::empty(area);
        draw_room_dock(&g, Some(1), false, RoomDockView::Info, &[], Some(1), area, &colors, &SymbolSet::default(), false, 0, &mut buf);
        let info = buf_text(&buf);
        assert!(info.contains("West of House"), "the header names the room: {info}");
        assert!(info.contains("Exits:"), "the Info body draws the exit card");
        assert!(info.contains("Forest Path"), "…naming where east goes");

        let mut buf = Buffer::empty(area);
        draw_room_dock(&g, Some(1), true, RoomDockView::Diagnostics, &[], Some(1), area, &colors, &SymbolSet::default(), false, 0, &mut buf);
        let diag = buf_text(&buf);
        assert!(diag.contains("Pos"), "the Diagnostics body draws the grid position: {diag}");
        assert!(diag.contains("edge"), "…and the edge summary");
        assert!(!diag.contains("Exits:"), "…and NOT the Info body");
    }

    #[test]
    fn an_empty_graph_says_nowhere_rather_than_drawing_a_blank_dock() {
        let g = MapGraph::new();
        let area = Rect::new(0, 0, 50, 10);
        let mut buf = Buffer::empty(area);
        draw_room_dock(&g, None, false, RoomDockView::Info, &[], None, area, &ColorScheme::default(), &SymbolSet::default(), false, 0, &mut buf);
        let text = buf_text(&buf);
        assert!(text.contains("nowhere yet"), "{text}");
        assert!(text.contains("not placed you in a room yet"), "{text}");
    }

    // ── The view tabs (SQ-0694) ──────────────────────────────────────────────

    /// The tabs are the SHARED strip — the same `draw_panel` + `PanelStrip` the map pane's layer
    /// tabs and the debug pane's section tabs use — so they come back as real hit-rects, one per
    /// view, inside the dock's header row.
    #[test]
    fn the_dock_returns_a_hit_rect_for_each_view_tab() {
        let g = graph_with_current();
        let area = Rect::new(0, 0, 60, 12);
        let mut buf = Buffer::empty(area);
        let rects = draw_room_dock(&g, Some(1), false, RoomDockView::Info, &[], Some(1), area,
            &ColorScheme::default(), &SymbolSet::default(), false, 0, &mut buf);
        let tabs = &rects.tabs;

        assert_eq!(tabs.len(), 2, "one rect per view");
        assert_eq!(tabs[0].0, RoomDockView::Info);
        assert_eq!(tabs[1].0, RoomDockView::Diagnostics);
        for (view, r) in tabs {
            assert!(r.width > 0 && r.height > 0, "{view:?} has a clickable rect");
            assert_eq!(r.y, area.y, "the strip sits on the panel's header row");
            assert!(r.x >= area.x && r.right() <= area.right(), "…inside the dock");
        }
        assert!(tabs[0].1.right() <= tabs[1].1.x, "the two tabs do not overlap");
    }

    /// The close box (SQ-1265) sits at the strip's right edge — the same spot
    /// `draw_dialog`'s `show_close` uses, "just inside the top-right border" —
    /// and is a real hit-rect on the header row, like the tabs beside it.
    #[test]
    fn the_dock_returns_a_hit_rect_for_the_close_box() {
        let g = graph_with_current();
        let area = Rect::new(0, 0, 60, 12);
        let mut buf = Buffer::empty(area);
        let rects = draw_room_dock(&g, Some(1), false, RoomDockView::Info, &[], Some(1), area,
            &ColorScheme::default(), &SymbolSet::default(), false, 0, &mut buf);

        let close = rects.close.expect("a 60-wide frame has room for the close box");
        assert_eq!(close.y, area.y, "the close box sits on the header row, like the tabs");
        assert_eq!(close.right(), area.right() - 1, "just inside the top-right border");
        assert!(
            close.x >= rects.tabs[1].1.right(),
            "the close box does not overlap the Diagnostics tab: {close:?} vs {:?}",
            rects.tabs[1].1,
        );
        assert_eq!(
            buf.cell((close.x, close.y)).unwrap().symbol(),
            "\u{2715}",
            "the close glyph is drawn where the rect says it is",
        );
    }

    /// A click on each tab rect flips the dock to that view — driven through the SAME routing
    /// function the run loop calls, on the SAME rects the draw returned, so this is the real
    /// gesture and not a restatement of the router's `match`.
    #[test]
    fn clicking_each_tab_switches_the_body() {
        use crate::input::{apply_action, room_dock_mouse_action, Action};
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

        let g = graph_with_current();
        let area = Rect::new(0, 0, 60, 12);
        let mut buf = Buffer::empty(area);
        let rects = draw_room_dock(&g, Some(1), false, RoomDockView::Info, &[], Some(1), area,
            &ColorScheme::default(), &SymbolSet::default(), false, 0, &mut buf);
        let tabs = &rects.tabs;

        let click = |col: u16, row: u16| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        };

        let mut st = crate::state::AppState::default();
        let mut m = mapper::mapper::Mapper::default();
        st.room_dock.toggle_to(true, true);
        assert_eq!(st.room_dock_view, RoomDockView::Info);

        // Every cell of each tab is a target, not just its first column.
        for (view, r) in tabs {
            for col in r.x..r.right() {
                st.room_dock_view = view.flipped();
                let action = room_dock_mouse_action(area, tabs, rects.close, &click(col, r.y), false)
                    .unwrap_or_else(|| panic!("a click inside the dock is always claimed"));
                assert_eq!(action, Action::SetRoomDockView(*view), "col {col} of the {view:?} tab");
                apply_action(action, &mut st, &mut m);
                assert_eq!(st.room_dock_view, *view, "clicking the {view:?} tab shows that body");
            }
        }

        // A click in the dock's BODY is claimed but changes nothing — the dock owns its rect, so
        // the click never falls through to the map or the story pane behind it.
        st.room_dock_view = RoomDockView::Info;
        let body = click(area.x + 3, area.bottom() - 2);
        assert_eq!(room_dock_mouse_action(area, tabs, rects.close, &body, false), Some(Action::None));
        assert_eq!(st.room_dock_view, RoomDockView::Info);

        // A click OUTSIDE the dock is not the dock's business at all.
        assert_eq!(
            room_dock_mouse_action(area, tabs, rects.close, &click(area.x, area.bottom() + 1), false),
            None,
            "an event outside the dock rect falls through to normal routing"
        );
    }

    /// A click on the close box closes the dock — the same effect
    /// `toggle-room-panel` has while it is open — and the "remembered" state
    /// (the animated slide's own open flag) flips with it, so the next frame's
    /// layout actually gives the rows back to the map. A click just outside the
    /// box (one column short) is claimed by the tab strip's own rect at most,
    /// never mistaken for the close box.
    #[test]
    fn clicking_the_close_box_closes_the_dock() {
        use crate::input::{apply_action, room_dock_mouse_action, Action};
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

        let g = graph_with_current();
        let area = Rect::new(0, 0, 60, 12);
        let mut buf = Buffer::empty(area);
        let rects = draw_room_dock(&g, Some(1), false, RoomDockView::Info, &[], Some(1), area,
            &ColorScheme::default(), &SymbolSet::default(), false, 0, &mut buf);
        let close = rects.close.expect("a 60-wide frame has room for the close box");

        let click = |col: u16, row: u16| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        };

        let mut st = crate::state::AppState::default();
        let mut m = mapper::mapper::Mapper::default();
        st.room_dock.toggle_to(true, true);
        assert!(st.room_dock.open, "starts open");

        let action = room_dock_mouse_action(area, &rects.tabs, rects.close, &click(close.x, close.y), false)
            .unwrap_or_else(|| panic!("a click on the close box is always claimed"));
        assert_eq!(action, Action::CloseRoomDock);
        apply_action(action, &mut st, &mut m);
        assert!(!st.room_dock.open, "the dock's remembered open flag flips");

        // One column short of the box: claimed by the dock (it is still inside
        // its rect) but never read as the close gesture.
        st.room_dock.toggle_to(true, true);
        let beside = room_dock_mouse_action(area, &rects.tabs, rects.close, &click(close.x - 1, close.y), false)
            .unwrap_or_else(|| panic!("still inside the dock's own rect"));
        assert_ne!(beside, Action::CloseRoomDock, "a miss beside the box is not the close gesture");
    }

    /// The strip is drawn by the shared component, so it wears the shared grammar: bracketed
    /// terminator caps and a divider between the tabs, exactly like the map pane's layer strip —
    /// and the active tab in `panel.tab:active`, the inactive one in `panel.tab`.
    #[test]
    fn the_tab_strip_matches_the_other_panes_strips() {
        let g = graph_with_current();
        let area = Rect::new(0, 0, 60, 12);
        let parsed = crate::theme::toml_schema::parse(
            "[panel]\ntab = { fg = \"blue\" }\n\"tab:active\" = { fg = \"green\" }\n",
        )
        .unwrap();
        let mut colors = ColorScheme::default();
        colors.theme = crate::theme::resolve::resolve_theme(
            &crate::colors::GhosttyScheme::default(),
            &parsed,
        );

        let mut buf = Buffer::empty(area);
        let tabs = draw_room_dock(&g, Some(1), false, RoomDockView::Info, &[], Some(1), area,
            &colors, &SymbolSet::default(), false, 0, &mut buf).tabs;

        let top: String = (0..area.width).map(|x| buf.cell((x, 0)).unwrap().symbol()).collect();
        assert!(top.contains("┤ Room "), "the shared left cap: {top:?}");
        assert!(top.contains("│"), "the shared tab divider: {top:?}");
        assert!(top.contains(" Diagnostics ├"), "the shared right cap: {top:?}");

        let fg_at = |r: Rect| buf.cell((r.x + 1, r.y)).and_then(|c| c.style().fg);
        assert_eq!(fg_at(tabs[0].1), Some(Color::Green), "the active view uses panel.tab:active");
        assert_eq!(fg_at(tabs[1].1), Some(Color::Blue), "the inactive one uses panel.tab");

        // …and it follows the view, not the tab order.
        let mut buf = Buffer::empty(area);
        let tabs = draw_room_dock(&g, Some(1), false, RoomDockView::Diagnostics, &[], Some(1), area,
            &colors, &SymbolSet::default(), false, 0, &mut buf).tabs;
        let fg_at = |r: Rect| buf.cell((r.x + 1, r.y)).and_then(|c| c.style().fg);
        assert_eq!(fg_at(tabs[0].1), Some(Color::Blue));
        assert_eq!(fg_at(tabs[1].1), Some(Color::Green));
    }

    /// The dock sits flush under the map pane and shares its columns, so it draws its frame in
    /// the MAP pane's glyphs — restyle one and the other follows (SQ-0694).
    #[test]
    fn the_dock_frame_follows_the_map_panes_glyphs() {
        let g = graph_with_current();
        let area = Rect::new(0, 0, 60, 12);
        let mut colors = ColorScheme::default();
        colors.map_border_glyphs.tl = Some("\u{2554}".to_string()); // ╔

        let mut buf = Buffer::empty(area);
        draw_room_dock(&g, Some(1), false, RoomDockView::Info, &[], Some(1), area,
            &colors, &SymbolSet::default(), false, 0, &mut buf);
        assert_eq!(
            buf.cell((0, 0)).unwrap().symbol(),
            "\u{2554}",
            "the dock takes the map pane's corner glyph, not the default set"
        );
    }

    #[test]
    fn zero_area_does_not_panic() {
        let g = graph_with_current();
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        draw_room_dock(&g, Some(1), false, RoomDockView::Info, &[], Some(1),
            Rect::new(0, 0, 0, 0), &ColorScheme::default(), &SymbolSet::default(), false, 0, &mut buf);
    }

    /// Every new visual element is styleable: `room_panel` paints the body and
    /// `room_panel.header` / `room_panel.header:pinned` the header line, and an
    /// override must actually reach the buffer.
    ///
    /// (The `honor_game_colours` pairing for this dock lives in
    /// `tests/room_dock_render.rs`, which drives it from a real `AppState`: the
    /// game's palette must not reach app chrome in EITHER mode, and only a state
    /// carrying that flag can show it.)
    #[test]
    fn the_dock_style_selectors_reach_the_buffer() {
        let g = graph_with_current();
        let area = Rect::new(0, 0, 50, 12);
        let parsed = crate::theme::toml_schema::parse(
            "[elements]\nroom_panel = { fg = \"magenta\" }\n\
             \"room_panel.header\" = { fg = \"blue\" }\n\
             \"room_panel.header:pinned\" = { fg = \"green\" }\n",
        )
        .unwrap();
        let scheme = crate::colors::GhosttyScheme::default();
        let mut colors = ColorScheme::default();
        colors.theme = crate::theme::resolve::resolve_theme(&scheme, &parsed);

        let fgs_of = |buf: &Buffer| -> Vec<Option<Color>> {
            (0..area.width)
                .flat_map(|x| (0..area.height).map(move |y| (x, y)))
                .map(|(x, y)| buf.cell((x, y)).and_then(|c| c.style().fg))
                .collect()
        };

        let mut buf = Buffer::empty(area);
        draw_room_dock(&g, Some(1), false, RoomDockView::Info, &[], Some(1), area, &colors, &SymbolSet::default(), false, 0, &mut buf);
        let fgs = fgs_of(&buf);
        assert!(fgs.contains(&Some(Color::Blue)), "the following header uses room_panel.header");
        assert!(fgs.contains(&Some(Color::Magenta)), "the body uses room_panel");
        assert!(!fgs.contains(&Some(Color::Green)), "…and not the pinned variant");

        let mut buf = Buffer::empty(area);
        draw_room_dock(&g, Some(1), true, RoomDockView::Info, &[], Some(1), area, &colors, &SymbolSet::default(), false, 0, &mut buf);
        assert!(
            fgs_of(&buf).contains(&Some(Color::Green)),
            "a pinned header uses room_panel.header:pinned"
        );
    }

    // ── Scrolling (SQ-1280) ───────────────────────────────────────────────────

    /// `draw_room_dock` threads `scroll_offset` through to the active body and reports its total
    /// row count and viewport back — the two numbers the caller syncs its `ListScroll` against
    /// after the draw returns.
    #[test]
    fn draw_room_dock_reports_the_active_bodys_totals_and_honours_the_offset() {
        let g = graph_with_current();
        // Narrow and short enough that the exit card alone overflows.
        let area = Rect::new(0, 0, 20, 6);
        let colors = ColorScheme::default();

        let mut buf = Buffer::empty(area);
        let at_top = draw_room_dock(&g, Some(1), false, RoomDockView::Info, &[], Some(1), area,
            &colors, &SymbolSet::default(), false, 0, &mut buf);
        assert!(at_top.body_total > at_top.body_viewport, "the card overflows this dock: {} vs {}", at_top.body_total, at_top.body_viewport);
        assert!(at_top.body_viewport > 0 && at_top.body_viewport < area.height, "the body sits below the header: {}", at_top.body_viewport);

        let max_offset = at_top.body_total - at_top.body_viewport;
        let mut buf2 = Buffer::empty(area);
        let at_end = draw_room_dock(&g, Some(1), false, RoomDockView::Info, &[], Some(1), area,
            &colors, &SymbolSet::default(), false, max_offset, &mut buf2);
        assert_eq!(at_end.body_total, at_top.body_total, "the same content reports the same total");
        // Scrolling actually changed what is on screen.
        let text_of = |b: &Buffer| b.content().iter().map(|c| c.symbol().to_owned()).collect::<String>();
        assert_ne!(text_of(&buf), text_of(&buf2), "a nonzero offset draws a different window of the body");
    }

    /// A wheel notch anywhere inside the dock scrolls the active body — mapped to
    /// `Action::RoomDockScroll` with the sign `wheel_delta` resolves, `mouse_wheel_invert` and
    /// all — and the tab/close routing above it is untouched.
    #[test]
    fn room_dock_mouse_action_maps_a_wheel_notch_to_room_dock_scroll() {
        use crate::input::{room_dock_mouse_action, Action};
        use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};

        let area = Rect::new(0, 0, 60, 12);
        let wheel = |kind: MouseEventKind| MouseEvent {
            kind,
            column: area.x + 3,
            row: area.y + 3,
            modifiers: KeyModifiers::NONE,
        };

        assert_eq!(
            room_dock_mouse_action(area, &[], None, &wheel(MouseEventKind::ScrollDown), false),
            Some(Action::RoomDockScroll(1)),
        );
        assert_eq!(
            room_dock_mouse_action(area, &[], None, &wheel(MouseEventKind::ScrollUp), false),
            Some(Action::RoomDockScroll(-1)),
        );
        // `mouse_wheel_invert` flips it, same as every other wheel handler.
        assert_eq!(
            room_dock_mouse_action(area, &[], None, &wheel(MouseEventKind::ScrollDown), true),
            Some(Action::RoomDockScroll(-1)),
        );

        // A wheel event OUTSIDE the dock is still not the dock's business.
        let outside = MouseEvent { kind: MouseEventKind::ScrollDown, column: area.x, row: area.bottom() + 1, modifiers: KeyModifiers::NONE };
        assert_eq!(room_dock_mouse_action(area, &[], None, &outside, false), None);
    }

    /// Applying `RoomDockScroll` actually moves the right body's `ListScroll` — Info and
    /// Diagnostics scroll independently, so a notch while one view is showing never touches the
    /// other's remembered position.
    #[test]
    fn applying_room_dock_scroll_moves_the_active_bodys_list_scroll_only() {
        use crate::input::{apply_action, Action};

        let mut st = crate::state::AppState::default();
        let mut m = mapper::mapper::Mapper::default();
        st.room_dock_view = RoomDockView::Info;
        st.room_dock_body_viewport = 4;
        st.room_dock_info_scroll.len(20);
        st.room_dock_diag_scroll.len(20);

        apply_action(Action::RoomDockScroll(1), &mut st, &mut m);
        assert_eq!(st.room_dock_info_scroll.target_offset(), 1, "the Info scroll moved");
        assert_eq!(st.room_dock_diag_scroll.target_offset(), 0, "…and the Diagnostics one did not");

        st.room_dock_view = RoomDockView::Diagnostics;
        apply_action(Action::RoomDockScroll(1), &mut st, &mut m);
        assert_eq!(st.room_dock_diag_scroll.target_offset(), 1, "now Diagnostics moves");
        assert_eq!(st.room_dock_info_scroll.target_offset(), 1, "…and Info stays where it was");
    }
}
