//! Inventory dock: a bordered multi-row list panel docked at the very bottom
//! of the screen (full width, under the input line, above the help row),
//! reserving layout space and sliding up/down via `state.inv_dock`.
//!
//! The caller (`main.rs`) sizes `area` from the animated `PanelSlide` fraction
//! (see `inventory_dock_height`), so `area` may be shorter than the panel's
//! target height while a slide is in flight — everything here clips to `area`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::draw_str_clipped;
use super::paneframe::{InsetSegment, PaneGlyphs};
use crate::colors::ColorScheme;
use crate::render::panel::{draw_panel, PanelSpec, PanelStrip};
use crate::state::AppState;

/// Click targets emitted while drawing the inventory dock, for the event
/// loop to hit-test — the panel's own counterpart of
/// [`crate::render::command_band::CommandBandHits`] (SQ-1244): a left-click
/// on an item composes its word into the prompt the same way a click on the
/// command band's WHAT column does.
#[derive(Default, Clone)]
pub struct InventoryDockHits {
    /// The dock's whole rect — clicks inside it belong to the panel and must
    /// not reach the story pane behind it. Zero-area whenever the dock isn't
    /// drawn this frame.
    pub area: Rect,
    /// Item rows, as `(index into the drawn item list, rect)`. Published for
    /// every row actually drawn this frame — never for a row scrolled/clipped
    /// past `content.bottom()`, and never outside the panel.
    pub rows: Vec<(usize, Rect)>,
}

/// Refill the inventory dock's clickable words from the engine, once per loop
/// tick (SQ-1244) — the command band's `refresh_objects` sibling for the
/// panel that shows exactly when the band is closed (`SidePanel`), so it
/// cannot piggyback on the band's own `carried` list.
///
/// Reuses the WHAT column's own noun derivation
/// (`render::transcript::inventory_click_words`, which wraps
/// `crate::vocab::typeable_name`) over the same one-level contents list
/// `inventory_items` draws, so a click composes the word the story's parser
/// actually accepts. Gated on the panel actually being visible or sliding —
/// same test `main.rs` uses to decide whether to compute `inv_items` at all
/// — so a closed dock costs nothing.
///
/// Pure bookkeeping for the click path: unlike `refresh_objects`, this never
/// changes what is drawn (the dock re-derives its own display list fresh
/// every frame in `main.rs`, same as it always has), so it reports nothing
/// for `needs_redraw` to OR in.
pub fn refresh_inventory_click_words(state: &mut AppState, engine: &dyn crate::engine::Engine) {
    if !(state.show_inventory || state.inv_dock.active()) {
        state.inventory_click_words.clear();
        return;
    }
    let vocab = state.vocab.get(engine);
    state.inventory_click_words = super::transcript::inventory_click_words(
        state.player_obj,
        &state.inventory_fallback,
        engine.introspect(),
        vocab,
    );
}

/// Compute the dock's fully-open target height in rows: one row per item
/// (minimum 1, for the "(empty)" line) plus 2 border rows, capped at
/// `cap_pct`% of the screen height so the dock never swallows the whole
/// terminal (default 33, ≈ the old fixed 1/3 cap).
pub fn inventory_dock_target_height(item_count: usize, full_height: u16, cap_pct: u16) -> u16 {
    let cap = ((full_height as u32 * cap_pct as u32) / 100) as u16;
    ((item_count.max(1) as u16) + 2).min(cap)
}

/// Compute the reserved dock band height in rows: `target_h` scaled by the
/// slide's current `fraction` (0.0 closed .. 1.0 fully open), rounded to the
/// nearest row. Extracted from the layout split so the arithmetic is testable
/// without a full terminal/main-loop harness.
pub fn inventory_dock_height(target_h: u16, fraction: f64) -> u16 {
    (target_h as f64 * fraction).round() as u16
}

/// Draw the inventory dock panel into `area`: a bordered box titled
/// " Inventory ", listing one item per row (or "(empty)" when there are none).
///
/// `area` is the currently-animated band height, which may be shorter than
/// the full target while mid-slide; content simply clips to whatever fits.
///
/// `highlighted` is true when interactive resize mode has this dock as its
/// target (draws the border with the `focused_border` accent instead).
///
/// Publishes `hits` (SQ-1244): the dock's own rect, and one row rect per item
/// actually drawn, in the exact `items` index the caller must resolve a click
/// against (`AppState::inventory_click_words`, the SAME order).
pub fn draw_inventory_dock(
    items: &[String],
    area: Rect,
    colors: &ColorScheme,
    highlighted: bool,
    buf: &mut Buffer,
    hits: &mut InventoryDockHits,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    hits.area = area;
    let style = colors.theme.get("inventory_panel").style;
    // Focus drives the border STYLE selector; the resize accent (or the dock's
    // own style) is preserved as the border COLOUR via `border_color`.
    let border_selector = if highlighted { "panel.border:active" } else { "panel.border" };
    let border_color = if highlighted { colors.theme.get("panel.border:active").style } else { style };

    // Fill the band's background first so panes behind it never show through
    // while it's mid-slide (shorter than its final bordered content needs).
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(style);
            }
        }
    }

    // Frame + title strip via the shared themed panel. The border style now
    // follows `panel.border` (so `[panel] border = { style = "double" }` reaches
    // the dock) and the title caps track that style; the border colour and the
    // "Inventory" strip (drawn in the dock's own style) are preserved exactly.
    let spec = PanelSpec {
        area,
        border_selector,
        border_color: Some(border_color),
        border_style: None,
        glyphs: &PaneGlyphs::default(),
        header_on: true,
        strip: Some(PanelStrip {
            segments: &[InsetSegment { text: "Inventory", active: false }],
            base: style,
            active: style,
        }),
        body_fill: None,
    };
    let frame = draw_panel(buf, &spec, &colors.theme);

    let content = frame.content;
    if content.height == 0 || content.width == 0 {
        return;
    }

    if items.is_empty() {
        draw_str_clipped(buf, content.x, content.y, "(empty)", style, content);
        return;
    }

    for (i, item) in items.iter().enumerate() {
        let y = content.y + i as u16;
        if y >= content.bottom() {
            break;
        }
        let row_area = Rect::new(content.x, y, content.width, 1);
        hits.rows.push((i, row_area));
        draw_str_clipped(buf, content.x, y, item, style, content);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn buf_contains(buf: &Buffer, s: &str) -> bool {
        let all: String = buf.content().iter().map(|c| c.symbol().to_owned()).collect();
        all.contains(s)
    }

    /// Build a `Theme` with the given selectors' fg overridden (like a
    /// `style.toml` decl), so tests exercising render code migrated to
    /// `theme.get("<selector>")` (SQ-0309) can still inject a custom colour
    /// instead of mutating the (no-longer-read) legacy `ColorScheme` field.
    fn theme_with_overrides(overrides: &[(&str, Color)]) -> crate::theme::resolve::Theme {
        let mut decls = std::collections::HashMap::new();
        for &(sel, fg) in overrides {
            decls.insert(sel.to_string(), crate::theme::registry::Delta { fg: Some(fg), ..crate::theme::registry::Delta::EMPTY });
        }
        crate::theme::resolve::resolve(
            &crate::theme::resolve::Roles::terminal_default(),
            &decls,
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
        )
    }

    #[test]
    fn draw_inventory_dock_shows_items_and_border() {
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        let colors = ColorScheme::default();
        let items = vec!["lamp".to_string(), "sword".to_string()];
        draw_inventory_dock(&items, area, &colors, false, &mut buf, &mut InventoryDockHits::default());

        assert!(buf_contains(&buf, "┌"), "top-left border corner");
        assert!(buf_contains(&buf, "┐"), "top-right border corner");
        assert!(buf_contains(&buf, "└"), "bottom-left border corner");
        assert!(buf_contains(&buf, "┘"), "bottom-right border corner");
        assert!(buf_contains(&buf, "Inventory"), "title");
        assert!(buf_contains(&buf, "lamp"), "first item");
        assert!(buf_contains(&buf, "sword"), "second item");
    }

    #[test]
    fn draw_inventory_dock_title_uses_shared_bracketed_header() {
        // The title comes from the shared panel header, so the top border row is
        // bracketed. With the default single `panel.border`, the caps now track
        // that style: "┤ Inventory ├" (single), not the old hardcoded thick
        // "┫ … ┣".
        let area = Rect::new(0, 0, 24, 5);
        let mut buf = Buffer::empty(area);
        let colors = ColorScheme::default();
        draw_inventory_dock(&["lamp".to_string()], area, &colors, false, &mut buf, &mut InventoryDockHits::default());
        let top: String = (0..area.width).map(|x| buf.cell((x, 0)).unwrap().symbol().to_owned()).collect();
        assert!(top.contains("┤ Inventory ├"), "single-cap title strip, got {top:?}");
    }

    #[test]
    fn draw_inventory_dock_follows_panel_border_style() {
        // A user's `[panel] border = { style = "double" }` must now reach the
        // dock: the top-left corner is the double corner ╔ and the title-strip
        // left cap tracks it (╡), proving the dock renders `panel.border` (not the
        // old hardcoded Single) and the caps follow that style.
        let scheme = crate::colors::GhosttyScheme::default();
        let parsed =
            crate::theme::toml_schema::parse("[panel]\nborder = { style = \"double\" }\n").unwrap();
        let mut colors = ColorScheme::default();
        colors.theme = crate::theme::resolve::resolve_theme(&scheme, &parsed);

        let area = Rect::new(0, 0, 24, 5);
        let mut buf = Buffer::empty(area);
        draw_inventory_dock(&["lamp".to_string()], area, &colors, false, &mut buf, &mut InventoryDockHits::default());

        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "╔", "double top-left corner");
        let top: String = (0..area.width).map(|x| buf.cell((x, 0)).unwrap().symbol().to_owned()).collect();
        assert!(top.contains("╡ Inventory ╞"), "double-cap title strip, got {top:?}");
    }

    #[test]
    fn draw_inventory_dock_empty_shows_placeholder() {
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        let colors = ColorScheme::default();
        draw_inventory_dock(&[], area, &colors, false, &mut buf, &mut InventoryDockHits::default());

        assert!(buf_contains(&buf, "(empty)"), "empty placeholder text");
    }

    #[test]
    fn draw_inventory_dock_zero_area_does_not_panic() {
        let area = Rect::new(0, 0, 0, 0);
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        let colors = ColorScheme::default();
        draw_inventory_dock(&["lamp".to_string()], area, &colors, false, &mut buf, &mut InventoryDockHits::default());
        // No assertion beyond "did not panic".
    }

    #[test]
    fn draw_inventory_dock_applies_theme_style() {
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        let mut colors = ColorScheme::default();
        colors.theme = theme_with_overrides(&[("inventory_panel", Color::Rgb(1, 2, 3))]);
        draw_inventory_dock(&["lamp".to_string()], area, &colors, false, &mut buf, &mut InventoryDockHits::default());
        assert_eq!(buf.cell((0, 0)).unwrap().style().fg, Some(Color::Rgb(1, 2, 3)));
    }

    #[test]
    fn draw_inventory_dock_publishes_a_hit_rect_per_drawn_row() {
        // SQ-1244: the panel's own rect plus one row rect per item actually
        // drawn, indexed exactly like `items` — nothing published for a row
        // clipped past the content area.
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        let colors = ColorScheme::default();
        let items = vec!["lamp".to_string(), "sword".to_string()];
        let mut hits = InventoryDockHits::default();
        draw_inventory_dock(&items, area, &colors, false, &mut buf, &mut hits);

        assert_eq!(hits.area, area, "the panel's own rect");
        assert_eq!(hits.rows.len(), 2, "one rect per drawn item");
        assert_eq!(hits.rows[0].0, 0, "first row is index 0");
        assert_eq!(hits.rows[1].0, 1, "second row is index 1");
        // Every row rect sits strictly inside the panel's own bordered area.
        for (_, r) in &hits.rows {
            assert!(r.x > area.x && r.right() <= area.right(), "row {r:?} outside {area:?}");
            assert!(r.y > area.y && r.bottom() < area.bottom(), "row {r:?} outside {area:?}");
        }
    }

    #[test]
    fn draw_inventory_dock_clips_rows_past_content_height_and_publishes_none_for_them() {
        // Content height is 1 (area height 3 minus 2 border rows); 3 items
        // offered, only the first can be drawn, so only one row rect.
        let area = Rect::new(0, 0, 20, 3);
        let mut buf = Buffer::empty(area);
        let colors = ColorScheme::default();
        let items = vec!["lamp".to_string(), "sword".to_string(), "rope".to_string()];
        let mut hits = InventoryDockHits::default();
        draw_inventory_dock(&items, area, &colors, false, &mut buf, &mut hits);

        assert_eq!(hits.rows.len(), 1, "only the row that actually fit is published");
        assert_eq!(hits.rows[0].0, 0);
    }

    #[test]
    fn draw_inventory_dock_empty_publishes_area_but_no_rows() {
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        let colors = ColorScheme::default();
        let mut hits = InventoryDockHits::default();
        draw_inventory_dock(&[], area, &colors, false, &mut buf, &mut hits);

        assert_eq!(hits.area, area);
        assert!(hits.rows.is_empty(), "the placeholder \"(empty)\" line is not a clickable row");
    }

    #[test]
    fn draw_inventory_dock_zero_area_publishes_nothing() {
        let area = Rect::new(0, 0, 0, 0);
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        let colors = ColorScheme::default();
        let mut hits = InventoryDockHits::default();
        draw_inventory_dock(&["lamp".to_string()], area, &colors, false, &mut buf, &mut hits);

        assert_eq!(hits.area, Rect::default());
        assert!(hits.rows.is_empty());
    }

    #[test]
    fn inventory_dock_height_scales_with_fraction() {
        assert_eq!(inventory_dock_height(4, 0.0), 0);
        assert_eq!(inventory_dock_height(4, 1.0), 4);
        assert_eq!(inventory_dock_height(4, 0.5), 2);
    }

    #[test]
    fn inventory_dock_target_height_is_items_plus_borders_capped() {
        // 2 items + 2 border rows = 4, well under a 30-row screen's 33% cap (9).
        assert_eq!(inventory_dock_target_height(2, 30, 33), 4);
        // Empty list still reserves 1 row (for "(empty)") + 2 borders = 3.
        assert_eq!(inventory_dock_target_height(0, 30, 33), 3);
        // Capped at 33% of full_height for a very long inventory: 30*33/100 = 9.
        assert_eq!(inventory_dock_target_height(100, 30, 33), 9);
    }

    #[test]
    fn inventory_dock_target_height_content_binds_when_cap_is_generous() {
        // Cap = 90*33/100 = 29; content (10 items + 2 = 12) is smaller, so
        // content binds.
        assert_eq!(inventory_dock_target_height(10, 90, 33), 12);
    }

    #[test]
    fn inventory_dock_target_height_cap_binds_when_items_overflow() {
        // Cap = 90*33/100 = 29; content (100 items + 2 = 102) overflows, so the
        // cap binds.
        assert_eq!(inventory_dock_target_height(100, 90, 33), 29);
    }

    #[test]
    fn dock_band_closed_is_zero_open_reserves_items_plus_borders() {
        // Mirrors the layout split in main.rs: closed (show_inventory=false,
        // inv_dock inactive) reserves 0 rows; fully open with 2 items reserves
        // item_count + 2 border rows.
        let full_height = 30u16;
        let closed_target = 0u16; // inv_visible == false path in main.rs
        assert_eq!(inventory_dock_height(closed_target, 0.0), 0);

        let open_target = inventory_dock_target_height(2, full_height, 33);
        assert_eq!(open_target, 4);
        assert_eq!(inventory_dock_height(open_target, 1.0), 4);
    }
}
