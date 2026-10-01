//! Inventory dock: a bordered multi-row list panel docked at the very bottom
//! of the screen (full width, under the input line, above the help row),
//! reserving layout space and sliding up/down via `state.inv_dock`.
//!
//! The caller (`main.rs`) sizes `area` from the animated `PanelSlide` fraction
//! (see `inventory_dock_height`), so `area` may be shorter than the panel's
//! target height while a slide is in flight — everything here clips to `area`.
//!
//! Two sections (SQ-1630), built by [`build_inventory_dock_rows`]: "Carrying:"
//! — the player's LIVE carried items, cross-referenced against
//! [`mapper::graph::MapGraph`]'s whole-game item registry (SQ-1627 and
//! friends) for where each was first found — and "Elsewhere:", every OTHER
//! item the registry has ever seen, with where it was last confirmed. A
//! section with nothing in it draws no header at all, the same rule
//! `render::room_info` follows for its own optional sections.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use mapper::graph::{ItemKey, ItemLocation, ItemRecord, MapGraph};

use super::draw_str_clipped;
use super::paneframe::{InsetSegment, PaneGlyphs};
use crate::colors::ColorScheme;
use crate::render::panel::{draw_panel, PanelSpec, PanelStrip};
use crate::state::AppState;

/// Click targets emitted while drawing the inventory dock, for the event
/// loop to hit-test — the panel's own counterpart of
/// [`crate::render::command_band::CommandBandHits`] (SQ-1244): a left-click
/// on a CARRIED item composes its word into the prompt the same way a click
/// on the command band's WHAT column does. An "Elsewhere" row is never
/// clickable — the item is not in scope, so there is no word the parser
/// would currently accept for it.
#[derive(Default, Clone)]
pub struct InventoryDockHits {
    /// The dock's whole rect — clicks inside it belong to the panel and must
    /// not reach the story pane behind it. Zero-area whenever the dock isn't
    /// drawn this frame.
    pub area: Rect,
    /// Carried-item rows, as `(index into `AppState::inventory_click_words`,
    /// rect)`. Published for every CARRIED row actually drawn this frame —
    /// never for a row scrolled/clipped past the content area, and never for
    /// a header or an "Elsewhere" row.
    pub rows: Vec<(usize, Rect)>,
    /// This frame's total content row count (SQ-1630) — headers, carried
    /// rows and elsewhere rows all counted, same as
    /// [`crate::render::room_info::draw_room_info_body`]'s return value.
    /// Zero whenever the dock wasn't drawn. Synced into
    /// `AppState::inv_dock_scroll`/`inv_dock_body_viewport` by the run loop
    /// the same way `room_dock_body_total`/`room_dock_body_viewport` are.
    pub body_total: u16,
    /// This frame's measured content viewport height (rows), for the run
    /// loop to clamp a wheel scroll against between frames.
    pub body_viewport: u16,
}

/// Refill the inventory dock's clickable words from the engine, once per loop
/// tick (SQ-1244) — the command band's `refresh_objects` sibling for the
/// panel that shows exactly when the band is closed (`SidePanel`), so it
/// cannot piggyback on the band's own object refresh.
///
/// Reuses the WHAT column's own noun derivation
/// (`render::transcript::inventory_click_words`, which wraps
/// `crate::vocab::typeable_name`) over the same one-level contents list
/// [`build_inventory_dock_rows`]'s `carried` argument is built from, so a
/// click composes the word the story's parser actually accepts. Gated on the
/// panel actually being visible or sliding — same test `main.rs` uses to
/// decide whether to compute the dock's content at all — so a closed dock
/// costs nothing.
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

/// Compute the dock's fully-open target height in rows: one row per content
/// line (minimum 1, for the "(empty)" line) plus 2 border rows, capped at
/// `cap_pct`% of the screen height so the dock never swallows the whole
/// terminal (default 33, ≈ the old fixed 1/3 cap).
///
/// `line_count` is the number of LINES the dock will actually draw this frame
/// — [`build_inventory_dock_rows`]'s result length, headers included, not a
/// bare item count (SQ-1630 widened this from "one row per carried item" once
/// the dock grew a second section and section headers of its own).
pub fn inventory_dock_target_height(line_count: usize, full_height: u16, cap_pct: u16) -> u16 {
    let cap = ((full_height as u32 * cap_pct as u32) / 100) as u16;
    ((line_count.max(1) as u16) + 2).min(cap)
}

/// Compute the reserved dock band height in rows: `target_h` scaled by the
/// slide's current `fraction` (0.0 closed .. 1.0 fully open), rounded to the
/// nearest row. Extracted from the layout split so the arithmetic is testable
/// without a full terminal/main-loop harness.
pub fn inventory_dock_height(target_h: u16, fraction: f64) -> u16 {
    (target_h as f64 * fraction).round() as u16
}

// ── Content rows (SQ-1630) ──────────────────────────────────────────────────

/// One line of the dock's content, before scrolling picks a window of it —
/// the dock's own analogue of `render::room_info::Row`.
#[derive(Debug, Clone, PartialEq)]
pub enum ItemDockRow {
    /// A section header ("Carrying:" / "Elsewhere:") or the blank separator
    /// between two non-empty sections.
    Header(String),
    /// A live carried item. `click_idx` is this item's position in the
    /// `carried` slice [`build_inventory_dock_rows`] was called with — the
    /// SAME index `AppState::inventory_click_words` uses, so a click
    /// resolves to the right word regardless of how filtering has rearranged
    /// what's actually drawn. `meta_start` is the CHAR count (not byte count —
    /// [`draw_str_clipped`] advances one column per `char`) of the prefix that
    /// stays at the row's ordinary style; the rest (the "found Room, turn N"
    /// clause, when there is one) draws dimmed. Equal to `text.chars().count()`
    /// when there's nothing to dim.
    Carried { text: String, click_idx: usize, meta_start: usize },
    /// A tracked item the registry has ever seen that is not currently
    /// carried — a `Room` or `Vanished` sighting. Never clickable (see
    /// [`InventoryDockHits::rows`]'s own doc). `meta_start` is the same split
    /// point [`Carried`]'s field is.
    Elsewhere { text: String, meta_start: usize },
}

/// Case-insensitive substring match — the filter's whole rule (`filter-items
/// <query>`, SQ-1630). `None` (no filter active) matches everything.
fn filter_matches(filter: Option<&str>, name: &str) -> bool {
    match filter {
        Some(f) => name.to_lowercase().contains(&f.to_lowercase()),
        None => true,
    }
}

/// One "Carrying:" row's text: the item's display name, cross-referenced
/// against the mapper's registry (by `key`) for where it was first found.
///
/// The NAME shown prefers the registry's own `rec.name` over the `name`
/// parameter whenever a record exists (SQ-1659): `name` is derived from
/// [`grammar_model::ObjectWords::display_name`] (every parser word for the
/// object joined together — the raw `o.display_name()` call in
/// `render::transcript::inventory_items_with_keys`), which is the right
/// fallback when there is no better source but is exactly the word-salad
/// shape `session::item_tracker_display_name` (SQ-1648) exists to avoid for
/// text the player reads — and the registry's `rec.name` is already that
/// resolved, clean name, stamped by `apply_item_observations` from the very
/// same per-turn observation this live contents list is built from. Only
/// falls back to `name` when there is no record yet to prefer.
///
/// `key` is `None` (no live object tree — the inventory-fallback path) or
/// `Some` of an id the registry never captured — rare, since the registry is
/// built from the very same per-turn observations, but possible: an item
/// picked up before the mapper started tracking, or a story whose "take"
/// phrasing the structural signal `apply_item_observations` reads never
/// recognised. Either way this shows a name ALONE, gracefully, rather than
/// omitting the item or panicking — an unrecorded provenance is not a reason
/// to hide an item the player is plainly holding right now.
///
/// Returns the line alongside the CHAR count of its name-prefix — the point
/// where the "found …" clause begins and dimmed drawing should take over
/// (`draw_inventory_dock`). Splitting on a count computed here, rather than
/// re-finding `" — "` at draw time, is deliberate: an item's own name could
/// itself contain that substring, which would misplace a naive re-parse.
fn carried_line(name: &str, key: Option<ItemKey>, graph: &MapGraph) -> (String, usize) {
    let record = key.and_then(|k| graph.item(k));
    match record {
        Some(rec) => {
            let prefix = format!("  {}", rec.name);
            let meta_start = prefix.chars().count();
            let line = format!(
                "{prefix} — found {}, turn {}",
                crate::render::room_info::display_name(graph, rec.origin_room),
                rec.origin_turn
            );
            (line, meta_start)
        }
        None => {
            let line = format!("  {name}");
            let meta_start = line.chars().count();
            (line, meta_start)
        }
    }
}

/// One "Elsewhere:" row's text: last confirmed room/turn, `(fixed in place)`
/// when the registry has ever seen an unambiguous take fail against it, and
/// `[vanished] ` when the last confirmed fact is that it went missing.
///
/// The `Vanished` arm reads `room`/`turn` off [`ItemLocation::Vanished`]
/// itself, never `rec.last_seen_turn` — they hold the same value today (see
/// that variant's own doc), but the enum's own fields are what the type
/// actually promises, and reading them directly can never drift from what is
/// displayed even if that stops being true.
///
/// `ItemLocation::Carried` is the one arm that should be structurally
/// unreachable here — every carried key is excluded from the "Elsewhere" set
/// by [`build_inventory_dock_rows`] before this is ever called — but it is
/// handled rather than left to panic, in case the mapper's carried record
/// and the engine's live contents list ever disagree for a frame (e.g. the
/// turn the registry hasn't caught up on a drop yet).
///
/// Returns the line alongside the CHAR count of its name-prefix (the item's
/// own name, plus `[vanished] ` when present) — where the dimmed metadata
/// clause begins. See [`carried_line`] for why this is computed here rather
/// than re-parsed at draw time.
fn elsewhere_line(rec: &ItemRecord, graph: &MapGraph) -> (String, usize) {
    let fixed = if rec.fixed_in_place { " (fixed in place)" } else { "" };
    match rec.last_seen {
        ItemLocation::Room { room, .. } => {
            let prefix = format!("  {}", rec.name);
            let meta_start = prefix.chars().count();
            let line = format!(
                "{prefix} — last seen {}, turn {}{fixed}",
                crate::render::room_info::display_name(graph, room),
                rec.last_seen_turn
            );
            (line, meta_start)
        }
        ItemLocation::Vanished { room, turn } => {
            let prefix = format!("  [vanished] {}", rec.name);
            let meta_start = prefix.chars().count();
            let line = format!(
                "{prefix} — last seen {}, turn {turn}{fixed}",
                crate::render::room_info::display_name(graph, room)
            );
            (line, meta_start)
        }
        ItemLocation::Carried => {
            let prefix = format!("  {}", rec.name);
            let meta_start = prefix.chars().count();
            let line = format!("{prefix} — carried{fixed}");
            (line, meta_start)
        }
    }
}

/// Build the dock's full content as logical rows, top to bottom (SQ-1630) —
/// independent of how many of them a scrolled/capped dock can actually show.
/// [`draw_inventory_dock`] windows this by `scroll_offset`; the row count is
/// also what [`inventory_dock_target_height`] sizes the band from.
///
/// - `carried`: the player's LIVE carried items — `(id, display name)` pairs,
///   in the SAME order as `AppState::inventory_click_words`
///   (`render::transcript::inventory_items_with_keys`'s own contract). `id`
///   is `None` in the inventory-fallback path (no live object tree to ask).
/// - `graph`: the mapper's whole-game item registry, for cross-referencing
///   `carried` and for every "Elsewhere" entry.
/// - `filter`: an active `filter-items` query (SQ-1630), or `None` to show
///   everything. Applies to both sections independently — matching an
///   "Elsewhere" item does not require anything in "Carrying" to also match,
///   and a section left empty by the filter draws no header, same as an
///   ordinarily-empty one.
///
/// "Elsewhere" items are excluded from "Carrying" by KEY, not by their
/// recorded `last_seen` — a carried item still tracks its own history in the
/// registry (`ItemLocation::Carried`), and the live list is the definitive
/// answer to "carried right now". Sorted by most-recently-seen turn first,
/// among items in the same vanished/not-vanished group; VANISHED items sort
/// to the bottom of the section regardless of turn (the user's explicit
/// instruction, SQ-1630) — a player scanning "Elsewhere" for something to go
/// get should see live leads before dead ones.
pub fn build_inventory_dock_rows(
    carried: &[(Option<ItemKey>, String)],
    graph: &MapGraph,
    filter: Option<&str>,
) -> Vec<ItemDockRow> {
    let mut carried_keys = std::collections::BTreeSet::new();
    let mut carried_rows = Vec::new();
    for (idx, (key, name)) in carried.iter().enumerate() {
        if let Some(k) = key {
            carried_keys.insert(*k);
        }
        if !filter_matches(filter, name) {
            continue;
        }
        let (text, meta_start) = carried_line(name, *key, graph);
        carried_rows.push(ItemDockRow::Carried { text, click_idx: idx, meta_start });
    }

    let mut elsewhere: Vec<(&ItemKey, &ItemRecord)> =
        graph.items().filter(|(k, _)| !carried_keys.contains(*k)).collect();
    elsewhere.retain(|(_, rec)| filter_matches(filter, &rec.name));
    elsewhere.sort_by(|(_, a), (_, b)| {
        let a_vanished = matches!(a.last_seen, ItemLocation::Vanished { .. });
        let b_vanished = matches!(b.last_seen, ItemLocation::Vanished { .. });
        a_vanished.cmp(&b_vanished).then_with(|| b.last_seen_turn.cmp(&a.last_seen_turn))
    });
    let elsewhere_rows: Vec<ItemDockRow> = elsewhere
        .iter()
        .map(|(_, rec)| {
            let (text, meta_start) = elsewhere_line(rec, graph);
            ItemDockRow::Elsewhere { text, meta_start }
        })
        .collect();

    let mut rows = Vec::new();
    if !carried_rows.is_empty() {
        rows.push(ItemDockRow::Header("Carrying:".to_string()));
        rows.extend(carried_rows);
    }
    if !elsewhere_rows.is_empty() {
        if !rows.is_empty() {
            rows.push(ItemDockRow::Header(String::new())); // blank separator, mirroring the mockup
        }
        rows.push(ItemDockRow::Header("Elsewhere:".to_string()));
        rows.extend(elsewhere_rows);
    }
    rows
}

/// Draw one row's `text` in two style runs: the name-prefix (`text`'s first
/// `meta_start` CHARS) at `style`, and the rest — the "found …"/"last seen …"
/// metadata clause, when there is one — dimmed at `meta_style`. Both runs
/// share `clip`, so the existing right-edge clipping still applies to each
/// independently. `meta_start == text.chars().count()` (nothing to dim) draws
/// the second call with an empty string, a no-op.
fn draw_row_with_dimmed_meta(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    text: &str,
    meta_start: usize,
    style: ratatui::style::Style,
    meta_style: ratatui::style::Style,
    clip: Rect,
) {
    let name_part: String = text.chars().take(meta_start).collect();
    let meta_part: String = text.chars().skip(meta_start).collect();
    draw_str_clipped(buf, x, y, &name_part, style, clip);
    let meta_x = x.saturating_add(u16::try_from(meta_start).unwrap_or(u16::MAX));
    draw_str_clipped(buf, meta_x, y, &meta_part, meta_style, clip);
}

/// Draw the inventory dock panel into `area`: a bordered box titled
/// " Inventory ", listing `rows` (or "(empty)" when there are none).
///
/// `area` is the currently-animated band height, which may be shorter than
/// the full target while mid-slide; content simply clips to whatever fits.
///
/// `highlighted` is true when interactive resize mode has this dock as its
/// target (draws the border with the `focused_border` accent instead).
///
/// `scroll_offset` is rows of content already scrolled past (SQ-1630) —
/// clamped here so a stale or out-of-range offset can never draw garbage or
/// leave a trailing gap. When the content overflows `area`, a themed
/// scrollbar (the same `scrollbar`/`scrollbar_track` selectors every other
/// scrollable list in the app uses) takes the rightmost column.
///
/// Publishes `hits` (SQ-1244, widened by SQ-1630): the dock's own rect, one
/// row rect per CARRIED row actually drawn (indexed by `click_idx`, never for
/// a header or an "Elsewhere" row), and this frame's total/viewport row
/// counts for the run loop to sync its scroll state against.
pub fn draw_inventory_dock(
    rows: &[ItemDockRow],
    area: Rect,
    colors: &ColorScheme,
    highlighted: bool,
    scroll_offset: u16,
    buf: &mut Buffer,
    hits: &mut InventoryDockHits,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    hits.area = area;
    let style = colors.theme.get("inventory_panel").style;
    let header_style = colors.theme.get("inventory_panel.header").style;
    let meta_style = colors.theme.get("inventory_panel.meta").style;
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

    if rows.is_empty() {
        draw_str_clipped(buf, content.x, content.y, "(empty)", style, content);
        return;
    }

    let total = rows.len() as u16;
    let viewport = content.height;
    hits.body_total = total;
    hits.body_viewport = viewport;

    let scrollbar_visible = crate::render::scroll::needs_scrollbar(total as usize, viewport as usize) && content.width >= 2;
    let text_w = if scrollbar_visible { content.width - 1 } else { content.width };
    let clip = Rect::new(content.x, content.y, text_w, content.height);
    let offset = scroll_offset.min(total.saturating_sub(viewport));

    for (i, row) in rows.iter().enumerate().skip(offset as usize).take(viewport as usize) {
        let y = content.y + (i as u16 - offset);
        let row_area = Rect::new(content.x, y, content.width, 1);
        match row {
            ItemDockRow::Header(text) => draw_str_clipped(buf, content.x, y, text, header_style, clip),
            ItemDockRow::Carried { text, click_idx, meta_start } => {
                hits.rows.push((*click_idx, row_area));
                draw_row_with_dimmed_meta(buf, content.x, y, text, *meta_start, style, meta_style, clip);
            }
            ItemDockRow::Elsewhere { text, meta_start } => {
                draw_row_with_dimmed_meta(buf, content.x, y, text, *meta_start, style, meta_style, clip);
            }
        }
    }

    if scrollbar_visible {
        let sb_area = Rect::new(content.right() - 1, content.y, 1, content.height);
        let look = crate::render::scroll::ScrollbarLook::from_theme(&colors.theme);
        crate::render::scroll::draw_scrollbar(buf, sb_area, total as usize, viewport as usize, offset as usize, look);
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

    fn rows_for(text: &[&str]) -> Vec<ItemDockRow> {
        text.iter()
            .enumerate()
            .map(|(i, t)| ItemDockRow::Carried { text: t.to_string(), click_idx: i, meta_start: t.chars().count() })
            .collect()
    }

    #[test]
    fn draw_inventory_dock_shows_items_and_border() {
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        let colors = ColorScheme::default();
        let rows = rows_for(&["  lamp", "  sword"]);
        draw_inventory_dock(&rows, area, &colors, false, 0, &mut buf, &mut InventoryDockHits::default());

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
        draw_inventory_dock(&rows_for(&["  lamp"]), area, &colors, false, 0, &mut buf, &mut InventoryDockHits::default());
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
        draw_inventory_dock(&rows_for(&["  lamp"]), area, &colors, false, 0, &mut buf, &mut InventoryDockHits::default());

        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "╔", "double top-left corner");
        let top: String = (0..area.width).map(|x| buf.cell((x, 0)).unwrap().symbol().to_owned()).collect();
        assert!(top.contains("╡ Inventory ╞"), "double-cap title strip, got {top:?}");
    }

    #[test]
    fn draw_inventory_dock_empty_shows_placeholder() {
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        let colors = ColorScheme::default();
        draw_inventory_dock(&[], area, &colors, false, 0, &mut buf, &mut InventoryDockHits::default());

        assert!(buf_contains(&buf, "(empty)"), "empty placeholder text");
    }

    #[test]
    fn draw_inventory_dock_zero_area_does_not_panic() {
        let area = Rect::new(0, 0, 0, 0);
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        let colors = ColorScheme::default();
        draw_inventory_dock(&rows_for(&["  lamp"]), area, &colors, false, 0, &mut buf, &mut InventoryDockHits::default());
        // No assertion beyond "did not panic".
    }

    #[test]
    fn draw_inventory_dock_applies_theme_style() {
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        let mut colors = ColorScheme::default();
        colors.theme = theme_with_overrides(&[("inventory_panel", Color::Rgb(1, 2, 3))]);
        draw_inventory_dock(&rows_for(&["  lamp"]), area, &colors, false, 0, &mut buf, &mut InventoryDockHits::default());
        assert_eq!(buf.cell((0, 0)).unwrap().style().fg, Some(Color::Rgb(1, 2, 3)));
    }

    #[test]
    fn draw_inventory_dock_dims_the_metadata_clause() {
        // The "found Room, turn N" clause draws in `inventory_panel.meta`'s
        // colour, distinct from the name-prefix's ordinary `inventory_panel`
        // colour — the user-requested dimming (SQ-1630 follow-up).
        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);
        let mut colors = ColorScheme::default();
        colors.theme = theme_with_overrides(&[
            ("inventory_panel", Color::Rgb(10, 20, 30)),
            ("inventory_panel.meta", Color::Rgb(40, 50, 60)),
        ]);
        let text = "  lamp — found Living Room, turn 4".to_string();
        let meta_start = "  lamp".chars().count();
        let rows = vec![ItemDockRow::Carried { text, click_idx: 0, meta_start }];
        let mut hits = InventoryDockHits::default();
        draw_inventory_dock(&rows, area, &colors, false, 0, &mut buf, &mut hits);

        let row_rect = hits.rows[0].1;
        assert_eq!(
            buf.cell((row_rect.x, row_rect.y)).unwrap().style().fg,
            Some(Color::Rgb(10, 20, 30)),
            "the name-prefix cell keeps the ordinary inventory_panel style"
        );
        let meta_x = row_rect.x + meta_start as u16;
        assert_eq!(
            buf.cell((meta_x, row_rect.y)).unwrap().style().fg,
            Some(Color::Rgb(40, 50, 60)),
            "the metadata clause cell carries the dimmed inventory_panel.meta style"
        );
    }

    #[test]
    fn draw_inventory_dock_publishes_a_hit_rect_per_carried_row() {
        // SQ-1244/SQ-1630: the panel's own rect plus one row rect per CARRIED
        // item actually drawn, indexed by `click_idx` — nothing published for a
        // header or an Elsewhere row.
        let area = Rect::new(0, 0, 20, 6);
        let mut buf = Buffer::empty(area);
        let colors = ColorScheme::default();
        let rows = vec![
            ItemDockRow::Header("Carrying:".to_string()),
            ItemDockRow::Carried { text: "  lamp".to_string(), click_idx: 0, meta_start: "  lamp".chars().count() },
            ItemDockRow::Carried { text: "  sword".to_string(), click_idx: 1, meta_start: "  sword".chars().count() },
        ];
        let mut hits = InventoryDockHits::default();
        draw_inventory_dock(&rows, area, &colors, false, 0, &mut buf, &mut hits);

        assert_eq!(hits.area, area, "the panel's own rect");
        assert_eq!(hits.rows.len(), 2, "one rect per carried item, not the header");
        assert_eq!(hits.rows[0].0, 0);
        assert_eq!(hits.rows[1].0, 1);
        for (_, r) in &hits.rows {
            assert!(r.x > area.x && r.right() <= area.right(), "row {r:?} outside {area:?}");
            assert!(r.y > area.y && r.bottom() < area.bottom(), "row {r:?} outside {area:?}");
        }
    }

    #[test]
    fn draw_inventory_dock_elsewhere_rows_publish_no_hit() {
        let area = Rect::new(0, 0, 20, 6);
        let mut buf = Buffer::empty(area);
        let colors = ColorScheme::default();
        let rows = vec![
            ItemDockRow::Header("Elsewhere:".to_string()),
            ItemDockRow::Elsewhere {
                text: "  whistle — last seen Attic, turn 3".to_string(),
                meta_start: "  whistle".chars().count(),
            },
        ];
        let mut hits = InventoryDockHits::default();
        draw_inventory_dock(&rows, area, &colors, false, 0, &mut buf, &mut hits);
        assert!(hits.rows.is_empty(), "an Elsewhere row is not clickable");
        assert!(buf_contains(&buf, "whistle"), "but it still draws");
    }

    #[test]
    fn draw_inventory_dock_clips_rows_past_content_height_and_publishes_none_for_them() {
        // Content height is 1 (area height 3 minus 2 border rows); 3 items
        // offered, only the first can be drawn, so only one row rect.
        let area = Rect::new(0, 0, 20, 3);
        let mut buf = Buffer::empty(area);
        let colors = ColorScheme::default();
        let rows = rows_for(&["  lamp", "  sword", "  rope"]);
        let mut hits = InventoryDockHits::default();
        draw_inventory_dock(&rows, area, &colors, false, 0, &mut buf, &mut hits);

        assert_eq!(hits.rows.len(), 1, "only the row that actually fit is published");
        assert_eq!(hits.rows[0].0, 0);
    }

    #[test]
    fn draw_inventory_dock_empty_publishes_area_but_no_rows() {
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        let colors = ColorScheme::default();
        let mut hits = InventoryDockHits::default();
        draw_inventory_dock(&[], area, &colors, false, 0, &mut buf, &mut hits);

        assert_eq!(hits.area, area);
        assert!(hits.rows.is_empty(), "the placeholder \"(empty)\" line is not a clickable row");
    }

    #[test]
    fn draw_inventory_dock_zero_area_publishes_nothing() {
        let area = Rect::new(0, 0, 0, 0);
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        let colors = ColorScheme::default();
        let mut hits = InventoryDockHits::default();
        draw_inventory_dock(&rows_for(&["  lamp"]), area, &colors, false, 0, &mut buf, &mut hits);

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
        // 2 lines + 2 border rows = 4, well under a 30-row screen's 33% cap (9).
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
        // inv_dock inactive) reserves 0 rows; fully open with 2 lines reserves
        // line_count + 2 border rows.
        let full_height = 30u16;
        let closed_target = 0u16; // inv_visible == false path in main.rs
        assert_eq!(inventory_dock_height(closed_target, 0.0), 0);

        let open_target = inventory_dock_target_height(2, full_height, 33);
        assert_eq!(open_target, 4);
        assert_eq!(inventory_dock_height(open_target, 1.0), 4);
    }

    // ── build_inventory_dock_rows (SQ-1630) ─────────────────────────────────

    fn text_of(row: &ItemDockRow) -> &str {
        match row {
            ItemDockRow::Header(t) => t,
            ItemDockRow::Carried { text, .. } => text,
            ItemDockRow::Elsewhere { text, .. } => text,
        }
    }

    #[test]
    fn carried_item_cross_references_its_origin_room_and_turn() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Living Room".into());
        g.note_item_seen(10, "brass lantern".to_string(), 1, true, None, 4);
        let carried = vec![(Some(10u32), "brass lantern".to_string())];
        g.note_item_carried(10, "brass lantern".to_string(), 1, 4);

        let rows = build_inventory_dock_rows(&carried, &g, None);
        let texts: Vec<&str> = rows.iter().map(text_of).collect();
        assert!(texts.contains(&"Carrying:"));
        assert!(
            texts.iter().any(|t| t.contains("brass lantern") && t.contains("found Living Room") && t.contains("turn 4")),
            "carried row cross-references origin room/turn: {texts:?}"
        );
        assert!(!texts.iter().any(|t| t == &"Elsewhere:"), "nothing else tracked");
    }

    #[test]
    fn carried_item_with_no_registry_record_falls_back_to_name_alone() {
        let g = MapGraph::new();
        let carried = vec![(Some(99u32), "mysterious coin".to_string())];
        let rows = build_inventory_dock_rows(&carried, &g, None);
        let texts: Vec<&str> = rows.iter().map(text_of).collect();
        assert!(
            texts.iter().any(|t| t.trim() == "mysterious coin"),
            "an uncaptured item still shows by name alone: {texts:?}"
        );
    }

    /// SQ-1659: the reported defect, reduced to a synthetic fixture. A carried item WITH a
    /// registry record must show the registry's already-clean `rec.name` — the exact name
    /// `session::item_tracker_display_name` resolved when the observation was first applied —
    /// even though the LIVE name passed in through `carried` is the raw joined word-salad
    /// `grammar_model::ObjectWords::display_name()` produces for an Inform 7 object with no
    /// hardware short name (the real shape: Anchorhead's green umbrella read "umbrella things
    /// green handle brolly bumbersho" instead of "umbrella").
    #[test]
    fn carried_item_with_a_registry_record_prefers_the_registrys_clean_name_over_a_word_salad_live_name() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Outside the Real Estate Office".into());
        g.note_item_carried(10, "umbrella".to_string(), 1, 2);
        let carried = vec![(Some(10u32), "umbrella things green handle brolly bumbersho".to_string())];

        let rows = build_inventory_dock_rows(&carried, &g, None);
        let (text, meta_start) = rows
            .iter()
            .find_map(|r| match r {
                ItemDockRow::Carried { text, meta_start, .. } => Some((text.clone(), *meta_start)),
                _ => None,
            })
            .expect("the carried row is drawn");
        let name_prefix: String = text.chars().take(meta_start).collect();
        assert_eq!(
            name_prefix.trim(),
            "umbrella",
            "the registry's clean name must win over the live word-salad name: {text:?}"
        );
    }

    /// Non-regression half of the same fix: with NO registry record yet (the very first frame
    /// before any observation has landed for this specific item), the live name passed in is
    /// still shown exactly as before — there is nothing better to prefer.
    #[test]
    fn carried_item_with_no_registry_record_still_shows_the_live_name_even_when_it_is_a_word_salad() {
        let g = MapGraph::new();
        let carried = vec![(Some(10u32), "umbrella things green handle brolly bumbersho".to_string())];
        let rows = build_inventory_dock_rows(&carried, &g, None);
        let texts: Vec<&str> = rows.iter().map(text_of).collect();
        assert!(
            texts.iter().any(|t| t.trim() == "umbrella things green handle brolly bumbersho"),
            "no registry record yet, so the live name is the only source available: {texts:?}"
        );
    }

    #[test]
    fn elsewhere_item_shows_last_seen_room_and_turn() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Attic".into());
        g.note_item_seen(20, "silver whistle".to_string(), 1, true, None, 31);
        let rows = build_inventory_dock_rows(&[], &g, None);
        let texts: Vec<&str> = rows.iter().map(text_of).collect();
        assert!(texts.contains(&"Elsewhere:"));
        assert!(
            texts.iter().any(|t| t.contains("silver whistle") && t.contains("last seen Attic") && t.contains("turn 31")),
            "{texts:?}"
        );
    }

    #[test]
    fn elsewhere_item_fixed_in_place_gets_the_suffix() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Forest".into());
        g.note_item_seen(30, "jeweled egg".to_string(), 1, true, None, 20);
        g.note_item_fixed_in_place(30);
        let rows = build_inventory_dock_rows(&[], &g, None);
        let texts: Vec<&str> = rows.iter().map(text_of).collect();
        assert!(
            texts.iter().any(|t| t.contains("jeweled egg") && t.contains("(fixed in place)")),
            "{texts:?}"
        );
    }

    #[test]
    fn vanished_item_shows_its_own_room_and_turn_and_sorts_last() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Kitchen".into());
        g.upsert_room(2, "Attic".into());
        // The vanished item's own last-seen turn (50) is LATER than the live item's
        // (10) — a naive most-recent-first sort would put it first. The vanished
        // rule must still send it to the bottom regardless of how recent it is;
        // this is what makes the test able to fail if that rule regresses to a
        // bare turn-descending sort (falsified: reverting the `a_vanished.cmp`
        // priority in `build_inventory_dock_rows` to a bare
        // `b.last_seen_turn.cmp(&a.last_seen_turn)` makes this assertion fail,
        // confirmed 2026-09-29, then restored).
        g.note_item_seen(40, "tarnished coin".to_string(), 1, true, None, 49);
        g.note_items_absent(1, 50, &std::collections::BTreeSet::new());
        g.note_item_seen(50, "silver whistle".to_string(), 2, true, None, 10);

        let rows = build_inventory_dock_rows(&[], &g, None);
        let elsewhere_texts: Vec<&str> = rows
            .iter()
            .skip_while(|r| text_of(r) != "Elsewhere:")
            .skip(1)
            .map(text_of)
            .collect();
        assert_eq!(elsewhere_texts.len(), 2);
        assert!(
            elsewhere_texts[0].contains("silver whistle"),
            "the live item sorts first despite its OLDER turn: {elsewhere_texts:?}"
        );
        assert!(
            elsewhere_texts[1].contains("[vanished]") && elsewhere_texts[1].contains("tarnished coin")
                && elsewhere_texts[1].contains("Kitchen") && elsewhere_texts[1].contains("turn 50"),
            "the vanished item sorts last, despite its NEWER turn, and reports its OWN room/turn: {elsewhere_texts:?}"
        );
    }

    #[test]
    fn empty_sections_draw_no_bare_header() {
        // Nothing carried, nothing tracked at all: no headers, no content rows.
        let g = MapGraph::new();
        let rows = build_inventory_dock_rows(&[], &g, None);
        assert!(rows.is_empty(), "{rows:?}");
    }

    #[test]
    fn only_carrying_present_shows_no_elsewhere_header() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Room".into());
        g.note_item_carried(10, "lamp".to_string(), 1, 1);
        let carried = vec![(Some(10u32), "lamp".to_string())];
        let rows = build_inventory_dock_rows(&carried, &g, None);
        let texts: Vec<&str> = rows.iter().map(text_of).collect();
        assert!(texts.contains(&"Carrying:"));
        assert!(!texts.contains(&"Elsewhere:"), "{texts:?}");
    }

    #[test]
    fn filter_matches_both_sections_independently() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Room".into());
        g.note_item_seen(20, "silver whistle".to_string(), 1, true, None, 5);
        let carried = vec![(Some(10u32), "brass lantern".to_string())];
        g.note_item_carried(10, "brass lantern".to_string(), 1, 1);

        // "whistle" matches only the Elsewhere item.
        let rows = build_inventory_dock_rows(&carried, &g, Some("whistle"));
        let texts: Vec<&str> = rows.iter().map(text_of).collect();
        assert!(!texts.contains(&"Carrying:"), "no carried match, no header: {texts:?}");
        assert!(texts.contains(&"Elsewhere:"));
        assert!(texts.iter().any(|t| t.contains("silver whistle")));
    }

    #[test]
    fn clearing_the_filter_restores_the_full_list() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Room".into());
        g.note_item_seen(20, "silver whistle".to_string(), 1, true, None, 5);
        let carried = vec![(Some(10u32), "brass lantern".to_string())];
        g.note_item_carried(10, "brass lantern".to_string(), 1, 1);

        let filtered = build_inventory_dock_rows(&carried, &g, Some("whistle"));
        assert!(filtered.iter().map(text_of).all(|t| !t.contains("brass lantern")));

        let cleared = build_inventory_dock_rows(&carried, &g, None);
        let texts: Vec<&str> = cleared.iter().map(text_of).collect();
        assert!(texts.iter().any(|t| t.contains("brass lantern")));
        assert!(texts.iter().any(|t| t.contains("silver whistle")));
    }

    #[test]
    fn a_body_taller_than_the_dock_shows_a_scrollbar_and_scrolling_reaches_the_rest() {
        let rows: Vec<ItemDockRow> = (0..8)
            .map(|i| {
                let text = format!("  item-{i:02}");
                let meta_start = text.chars().count();
                ItemDockRow::Carried { text, click_idx: i, meta_start }
            })
            .collect();
        let area = Rect::new(0, 0, 20, 6); // content height 4, 8 rows offered
        let colors = ColorScheme::default();

        let mut buf = Buffer::empty(area);
        let mut hits = InventoryDockHits::default();
        draw_inventory_dock(&rows, area, &colors, false, 0, &mut buf, &mut hits);
        assert_eq!(hits.body_total, 8);
        assert_eq!(hits.body_viewport, 4);
        assert!(buf_contains(&buf, "item-00"), "top of the list visible at offset 0");
        assert!(!buf_contains(&buf, "item-07"), "the last row is below the fold");

        let mut buf2 = Buffer::empty(area);
        let mut hits2 = InventoryDockHits::default();
        let max_offset = hits.body_total - hits.body_viewport;
        draw_inventory_dock(&rows, area, &colors, false, max_offset, &mut buf2, &mut hits2);
        assert!(buf_contains(&buf2, "item-07"), "scrolling to the max offset reaches the last row");
        assert!(!buf_contains(&buf2, "item-00"), "the top has scrolled off");
    }
}
