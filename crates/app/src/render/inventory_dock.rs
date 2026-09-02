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
pub fn draw_inventory_dock(items: &[String], area: Rect, colors: &ColorScheme, highlighted: bool, buf: &mut Buffer) {
    if area.width == 0 || area.height == 0 {
        return;
    }
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
        draw_inventory_dock(&items, area, &colors, false, &mut buf);

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
        draw_inventory_dock(&["lamp".to_string()], area, &colors, false, &mut buf);
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
        draw_inventory_dock(&["lamp".to_string()], area, &colors, false, &mut buf);

        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "╔", "double top-left corner");
        let top: String = (0..area.width).map(|x| buf.cell((x, 0)).unwrap().symbol().to_owned()).collect();
        assert!(top.contains("╡ Inventory ╞"), "double-cap title strip, got {top:?}");
    }

    #[test]
    fn draw_inventory_dock_empty_shows_placeholder() {
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        let colors = ColorScheme::default();
        draw_inventory_dock(&[], area, &colors, false, &mut buf);

        assert!(buf_contains(&buf, "(empty)"), "empty placeholder text");
    }

    #[test]
    fn draw_inventory_dock_zero_area_does_not_panic() {
        let area = Rect::new(0, 0, 0, 0);
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        let colors = ColorScheme::default();
        draw_inventory_dock(&["lamp".to_string()], area, &colors, false, &mut buf);
        // No assertion beyond "did not panic".
    }

    #[test]
    fn draw_inventory_dock_applies_theme_style() {
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        let mut colors = ColorScheme::default();
        colors.theme = theme_with_overrides(&[("inventory_panel", Color::Rgb(1, 2, 3))]);
        draw_inventory_dock(&["lamp".to_string()], area, &colors, false, &mut buf);
        assert_eq!(buf.cell((0, 0)).unwrap().style().fg, Some(Color::Rgb(1, 2, 3)));
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
