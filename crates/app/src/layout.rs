//! Single source of truth for pane geometry: the vertical split that carves
//! out the command band, the inventory dock and the help row, the per-`Layout`
//! split of the remaining panes area between the story and map panes, and the
//! room dock carved off the map pane's own bottom (SQ-0692).
//!
//! Extracted from the inline `.constraints(...)` splits that used to live in
//! `main.rs`'s `terminal.draw` closure so the geometry is testable without a
//! full terminal/render stack. Behavior-identical to that inline code.

use ratatui::layout::{Constraint, Direction, Layout as RatatuiLayout, Rect};

use crate::render::command_band::{band_height, band_target_height};
use crate::render::inventory_dock::{inventory_dock_height, inventory_dock_target_height};
use crate::state::{AppState, Layout};

/// The resolved pane rects for one frame. `story`/`map` are the OUTER
/// (pre-frame) rects passed to `draw_framed`; they are `Rect::default()`
/// (zero area) when that pane is hidden for the current `Layout`.
/// `command_band`/`inv_dock`/`room_dock` are zero-area when closed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PaneLayout {
    /// The whole frame this layout was computed from. Kept because the
    /// inventory dock's height is a PERCENTAGE of it — a drag that moves the
    /// dock edge by rows has to invert that percentage against the same
    /// height the layout used (SQ-0669).
    pub frame: Rect,
    pub story: Rect,
    pub map: Rect,
    pub command_band: Rect,
    pub inv_dock: Rect,
    /// The room dock, carved out of the MAP pane's bottom (SQ-0692). Zero-area
    /// when closed, when the layout hides the map, or while the debug inspector
    /// owns the map slot. `map` is already the SHRUNK pane rect when this has
    /// rows, so nothing downstream has to subtract it again.
    pub room_dock: Rect,
    pub help_row: Rect,
    /// How wide/tall each draggable boundary's grab zone reaches, in cells —
    /// `Config::grab_zone_cells` clamped to `MIN_GRAB_ZONE_CELLS..=
    /// MAX_GRAB_ZONE_CELLS` (SQ-1327). `boundary_zones` reads this; it does not
    /// affect `CommandBandTop`, which always keeps a single-row zone — see its
    /// doc comment.
    pub grab_zone_cells: u16,
}

// ── Draggable pane boundaries (SQ-0669) ───────────────────────────────────────

/// Smallest / largest story share of the story-map split, in percent. Resize
/// mode's arrows and the mouse drag clamp to the SAME limits — one definition,
/// so the two ways of moving the splitter can never disagree about its range.
pub const MIN_SPLIT_PCT: u16 = 20;
pub const MAX_SPLIT_PCT: u16 = 80;
/// Smallest / largest inventory-dock cap, in percent of the frame height.
pub const MIN_INV_DOCK_PCT: u16 = 10;
pub const MAX_INV_DOCK_PCT: u16 = 80;
/// Smallest / largest room-dock height, in percent of the frame height. The same
/// range as the inventory dock's, deliberately: both are bottom docks sized as a
/// share of the frame, and [`dock_pct_for_rows`] inverts either one. The map's
/// own floor (`MIN_MAP_ROWS`) is what actually stops a tall dock from eating the
/// pane — a percentage cannot know how many rows the map pane has.
pub const MIN_ROOM_DOCK_PCT: u16 = MIN_INV_DOCK_PCT;
pub const MAX_ROOM_DOCK_PCT: u16 = MAX_INV_DOCK_PCT;
/// Smallest / largest width (splitter) or height (dock edges) of a draggable
/// boundary's grab zone, in cells. `Config::grab_zone_cells` is clamped to this
/// range when [`compute_pane_layout`] resolves it — a mouse click is a point but
/// a finger is not, so a touch session (the Docker web image on a tablet) wants
/// a wider target than the default (SQ-1327).
pub const MIN_GRAB_ZONE_CELLS: u16 = 1;
pub const MAX_GRAB_ZONE_CELLS: u16 = 6;

/// A pane boundary the mouse can grab and drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boundary {
    /// The vertical splitter between the story and map panes (moves
    /// `split_ratio`).
    StoryMapSplit,
    /// The inventory dock's top edge (moves `inv_dock_pct`).
    InvDockTop,
    /// The command band's top edge (moves `command_band.height`).
    CommandBandTop,
    /// The room dock's top edge (moves `room_dock_pct`). Spans the MAP pane's
    /// columns only — the dock sits inside that pane, not across the frame.
    RoomDockTop,
}

/// One boundary plus the screen rect that grabs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundaryZone {
    pub boundary: Boundary,
    pub rect: Rect,
}

impl PaneLayout {
    /// The combined story+map region before the per-layout split — reconstructs
    /// what was previously called `panes_area` in `main.rs`. Used as a last-resort
    /// overlay target when both panes report zero content height (e.g. a terminal
    /// so small the pane's border consumes all its rows).
    pub fn panes_area(&self) -> Rect {
        let story_empty = self.story.width == 0 && self.story.height == 0;
        let map_empty = self.map.width == 0 && self.map.height == 0;
        match (story_empty, map_empty) {
            (true, true) => Rect::default(),
            (true, false) => self.map,
            (false, true) => self.story,
            (false, false) => self.story.union(self.map),
        }
    }

    /// The draggable boundaries of this frame, with their grab zones.
    ///
    /// A one-cell target is hard to hit with a mouse and harder still with a
    /// finger, so each zone straddles the divider that is actually DRAWN there,
    /// reaching `self.grab_zone_cells` cells out from it in total (default 2,
    /// raised from `Config::grab_zone_cells` for touch — SQ-1327):
    ///
    /// - the splitter straddles the story pane's right border and the map
    ///   pane's left border, which abut (`story.right() == map.x`);
    /// - the inventory and room docks' top edges straddle the pane border above
    ///   them and their own top border, the same way.
    ///
    /// At the default of 2 that is exactly one cell either side, which is the
    /// whole divider these three actually draw and matches lanthorn's original
    /// (unconfigurable) zones.
    ///
    /// `CommandBandTop` is the one exception: it has no border of its own
    /// (SQ-0667 made it a borderless strip), so its zone stays the single
    /// pane-border row above it REGARDLESS of `grab_zone_cells`. Widening it
    /// down into the band would swallow clicks on the band's column headers,
    /// which is a worse trade than a narrow grab — so a touch session widens
    /// every other boundary and leaves this one alone.
    ///
    /// The splitter comes first, so the corner cell where it meets a horizontal
    /// edge grabs the splitter (`boundary_at` takes the first match).
    pub fn boundary_zones(&self) -> Vec<BoundaryZone> {
        let mut zones = Vec::new();
        let reach = self.grab_zone_cells;

        // Splitter: only when both panes are actually on screen and adjacent.
        let split_live = self.story.width > 0
            && self.story.height > 0
            && self.map.width > 0
            && self.map.height > 0
            && self.story.right() == self.map.x;
        if split_live {
            let into_story = reach / 2;
            zones.push(BoundaryZone {
                boundary: Boundary::StoryMapSplit,
                rect: Rect::new(
                    self.story.right().saturating_sub(into_story),
                    self.story.y,
                    reach,
                    self.story.height,
                ),
            });
        }

        // Horizontal edges: the band and the dock are never both up (the band
        // subsumes the dock while it is open), but derive them independently
        // anyway — a zone only exists when its band has rows on screen.
        for (boundary, band) in [
            (Boundary::CommandBandTop, self.command_band),
            (Boundary::InvDockTop, self.inv_dock),
            (Boundary::RoomDockTop, self.room_dock),
        ] {
            if band.width == 0 || band.height == 0 {
                continue;
            }
            // CommandBandTop ignores `grab_zone_cells` — see the doc comment
            // above — and keeps the single pane-border row above the band;
            // the other two split `reach` across the border they actually draw.
            let (rows, above) = match boundary {
                Boundary::CommandBandTop => (1, 1),
                _ => (reach, reach / 2),
            };
            let top = band.y.saturating_sub(above);
            zones.push(BoundaryZone {
                boundary,
                rect: Rect::new(band.x, top, band.width, rows),
            });
        }

        zones
    }
}

/// Which boundary (if any) the cell at `col`/`row` grabs.
pub fn boundary_at(zones: &[BoundaryZone], col: u16, row: u16) -> Option<Boundary> {
    zones
        .iter()
        .find(|z| {
            z.rect.width > 0
                && z.rect.height > 0
                && col >= z.rect.x
                && col < z.rect.right()
                && row >= z.rect.y
                && row < z.rect.bottom()
        })
        .map(|z| z.boundary)
}

/// Split `panes_area` between the story and map panes at `split_ratio` percent.
///
/// The single definition of that split: `compute_pane_layout` draws with it and
/// the mouse drag INVERTS it (`split_pct_for_story_width`), so the splitter can
/// track the pointer without re-deriving ratatui's rounding by hand.
pub fn split_story_map(panes_area: Rect, split_ratio: u16) -> (Rect, Rect) {
    let chunks = RatatuiLayout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(split_ratio),
            Constraint::Percentage(100u16.saturating_sub(split_ratio)),
        ])
        .split(panes_area);
    (chunks[0], chunks[1])
}

/// The `split_ratio` whose resulting story pane sits closest to `want` columns
/// wide — the inverse of [`split_story_map`], found by asking it.
///
/// A percentage is coarser than a column whenever the panes are wider than 100
/// cells, so the exact width is not always reachable; this returns the closest
/// achievable one (lowest ratio on a tie), which is what makes a drag track the
/// pointer as tightly as the persisted unit allows.
pub fn split_pct_for_story_width(panes_area: Rect, want: u16) -> u16 {
    (MIN_SPLIT_PCT..=MAX_SPLIT_PCT)
        .min_by_key(|p| {
            let (story, _) = split_story_map(panes_area, *p);
            (story.width as i32 - want as i32).abs()
        })
        .unwrap_or(MIN_SPLIT_PCT)
}

/// The `inv_dock_pct` whose cap admits `rows` rows of dock in a `frame_height`
/// frame — the inverse of `inventory_dock_target_height`'s
/// `cap = frame_height * pct / 100`.
///
/// Rounded UP so the cap is never a row short of what was asked for: the dock's
/// height is `min(items + 2, cap)`, so a cap that floors below the current
/// height would shrink the dock on a drag that meant to hold it still. (When
/// the item list is the binding constraint, dragging the edge further up is a
/// no-op — the same ceiling resize mode's arrows hit.)
pub fn dock_pct_for_rows(frame_height: u16, rows: u16) -> u16 {
    if frame_height == 0 {
        return MIN_INV_DOCK_PCT;
    }
    let pct = (rows as u32 * 100).div_ceil(frame_height as u32);
    (pct as u16).clamp(MIN_INV_DOCK_PCT, MAX_INV_DOCK_PCT)
}

/// Compute this frame's pane geometry. `inv_item_count` is passed in (rather
/// than computed here) so this stays free of `engine.introspect()`/rendering
/// dependencies.
pub fn compute_pane_layout(area: Rect, state: &AppState, inv_item_count: usize) -> PaneLayout {
    // ── Inventory dock: reserve a bottom band (above the help row) that
    // slides up when toggled, sized from the item list + slide fraction.
    let inv_visible = state.show_inventory || state.inv_dock.active();
    let inv_target_h = if inv_visible {
        inventory_dock_target_height(inv_item_count, area.height, state.pane_sizes.inv_dock_pct)
    } else {
        0
    };
    let inv_dock_h = inventory_dock_height(inv_target_h, state.inv_dock.fraction());

    // ── Command band: a bottom band under the story pane, above the help row
    // and above the inventory dock, sliding up when opened (SQ-0664). While
    // it is open it SUBSUMES the inventory dock — the "carried" column IS the
    // inventory — so the dock is not reserved at all and returns on close.
    let band_visible = state.command_band_visible();
    let band_target_h = band_target_height(band_visible, area.height, state.pane_sizes.band_height);
    let band_h = band_height(band_target_h, state.band_dock.fraction());
    let inv_dock_h = if band_visible { 0 } else { inv_dock_h };

    // ── Reserve bottom 1 row for help bar, the command band and the inventory
    // dock band above it ─────────────────────────────────────────────────────
    let vert = RatatuiLayout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(band_h),
            Constraint::Length(inv_dock_h),
            Constraint::Length(1),
        ])
        .split(area);
    let panes_area = vert[0];
    let band_area = vert[1];
    let inv_dock_area = vert[2];
    let help_row = vert[3];

    // The debug inspector tiles into the map slot; make sure a right-slot rect
    // exists for it even when the current layout is TranscriptFull (map hidden).
    let effective_layout = if state.debug.is_some() { Layout::Split } else { state.layout };
    let (story, map) = match effective_layout {
        Layout::TranscriptFull => (panes_area, Rect::default()),
        Layout::Split => split_story_map(panes_area, state.pane_sizes.split_ratio),
    };

    // ── Room dock: carved off the MAP pane's bottom (SQ-0692), so it docks
    // under whatever the layer draws as — the drawn map or the matrix table —
    // and the map pane above it keeps its own frame. Never while the debug
    // inspector owns that slot (it is not a map), and never when the layout has
    // no map pane to dock inside.
    let dock_visible = state.room_dock_visible() && state.debug.is_none();
    let room_dock_target = if dock_visible {
        crate::render::room_dock::room_dock_target_height(
            map.height,
            area.height,
            state.pane_sizes.room_dock_pct,
        )
    } else {
        0
    };
    let room_dock_h = crate::render::room_dock::room_dock_height(
        room_dock_target,
        state.room_dock.fraction(),
    );
    let (map, room_dock_area) = if room_dock_h > 0 {
        (
            Rect::new(map.x, map.y, map.width, map.height - room_dock_h),
            Rect::new(map.x, map.bottom() - room_dock_h, map.width, room_dock_h),
        )
    } else {
        (map, Rect::default())
    };

    PaneLayout {
        frame: area,
        story,
        map,
        command_band: band_area,
        inv_dock: inv_dock_area,
        room_dock: room_dock_area,
        help_row,
        grab_zone_cells: state.config.grab_zone_cells.clamp(MIN_GRAB_ZONE_CELLS, MAX_GRAB_ZONE_CELLS),
    }
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use crate::render::command_band::{default_quick, default_verbs};
    use crate::state::CommandBandState;

    fn open_band(state: &mut AppState) {
        state.overlays.command_band =
            Some(CommandBandState::new(default_verbs(), default_quick()));
        state.band_dock.toggle_to(true, true); // instant open → fraction() == 1.0
    }

    fn area80x24() -> Rect {
        Rect::new(0, 0, 80, 24)
    }

    #[test]
    fn split_layout_halves_panes() {
        let state = AppState::default();
        assert_eq!(state.layout, Layout::Split);
        let pl = compute_pane_layout(area80x24(), &state, 0);

        // Docks closed → zero area.
        assert_eq!(pl.inv_dock.width * pl.inv_dock.height, 0);
        assert_eq!(pl.command_band.width * pl.command_band.height, 0);

        // Help row is the bottom single row.
        assert_eq!(pl.help_row, Rect::new(0, 23, 80, 1));

        // Story + map fill the remaining 23 rows and split the 80 columns ~evenly.
        assert_eq!(pl.story.height, 23);
        assert_eq!(pl.map.height, 23);
        assert_eq!(pl.story.y, 0);
        assert_eq!(pl.map.y, 0);
        assert_eq!(pl.story.width + pl.map.width, 80);
        assert!((pl.story.width as i32 - pl.map.width as i32).abs() <= 1);
    }

    #[test]
    fn split_matches_manual_split_of_panes_area() {
        // Parity check: reproduce the exact old inline computation (panes_area =
        // area minus the 1-row help row, docks closed) and assert the pure
        // function agrees exactly.
        let area = area80x24();
        let state = AppState::default();
        let pl = compute_pane_layout(area, &state, 0);

        let panes_area = Rect::new(0, 0, 80, 23);
        let chunks = RatatuiLayout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(panes_area);

        assert_eq!(pl.story, chunks[0]);
        assert_eq!(pl.map, chunks[1]);
    }

    #[test]
    fn transcript_full_hides_map() {
        let mut state = AppState::default();
        state.layout = Layout::TranscriptFull;
        let pl = compute_pane_layout(area80x24(), &state, 0);

        assert_eq!(pl.map.width * pl.map.height, 0);
        assert_eq!(pl.story, Rect::new(0, 0, 80, 23));
    }

    #[test]
    fn help_row_always_bottom_single_row() {
        for layout in [Layout::Split, Layout::TranscriptFull] {
            let mut state = AppState::default();
            state.layout = layout;
            let pl = compute_pane_layout(area80x24(), &state, 0);
            assert_eq!(pl.help_row, Rect::new(0, 23, 80, 1), "{layout:?}");
        }
    }

    #[test]
    fn inv_dock_open_reserves_bottom_band() {
        let mut state = AppState::default();
        state.show_inventory = true;
        state.inv_dock.toggle_to(true, true); // instant open → fraction() == 1.0
        let pl = compute_pane_layout(area80x24(), &state, 3);

        // target height = item_count(3) + 2 borders = 5, capped at height/3 = 8 → 5.
        assert_eq!(pl.inv_dock.height, 5);
        assert_eq!(pl.help_row, Rect::new(0, 23, 80, 1));
        // Story/map shrink to make room for the dock band above the help row.
        assert_eq!(pl.story.height + pl.inv_dock.height + pl.help_row.height, 24);
    }

    /// The band is a BOTTOM band now (SQ-0664): full width, above the help row,
    /// with the story/map panes shrinking to make room.
    #[test]
    fn command_band_open_reserves_a_bottom_band() {
        let mut state = AppState::default();
        open_band(&mut state);
        let pl = compute_pane_layout(area80x24(), &state, 0);

        assert_eq!(pl.command_band.width, 80, "full width");
        assert_eq!(pl.command_band.x, 0);
        assert_eq!(
            pl.command_band.height,
            crate::render::command_band::DEFAULT_BAND_ROWS,
            "the default-height band"
        );
        assert_eq!(pl.help_row, Rect::new(0, 23, 80, 1), "help row stays the bottom row");
        assert_eq!(pl.command_band.y + pl.command_band.height, pl.help_row.y);
        assert_eq!(pl.story.height + pl.command_band.height + pl.help_row.height, 24);
        // Story and map keep the FULL width — no left carve any more.
        assert_eq!(pl.story.x, 0);
        assert_eq!(pl.story.width + pl.map.width, 80);
    }

    /// Decision 1: while the band is open it subsumes the inventory dock (the
    /// carried column IS the inventory), which returns when the band closes.
    #[test]
    fn open_band_subsumes_the_inventory_dock() {
        let mut state = AppState::default();
        state.show_inventory = true;
        state.inv_dock.toggle_to(true, true);

        let before = compute_pane_layout(area80x24(), &state, 3);
        assert!(before.inv_dock.height > 0, "the dock is up to begin with");

        open_band(&mut state);
        let during = compute_pane_layout(area80x24(), &state, 3);
        assert_eq!(during.inv_dock.height, 0, "the band subsumes the inventory panel");
        assert!(during.command_band.height > 0);

        // Closing the band brings it back.
        state.overlays.command_band = None;
        state.band_dock.toggle_to(false, true);
        let after = compute_pane_layout(area80x24(), &state, 3);
        assert_eq!(after.inv_dock.height, before.inv_dock.height, "the dock returns on close");
    }

    /// The configured height drives the band, clamped so it can never starve
    /// the story pane.
    #[test]
    fn band_height_follows_config_and_clamps() {
        let mut state = AppState::default();
        open_band(&mut state);
        state.pane_sizes.band_height = 10;
        assert_eq!(compute_pane_layout(area80x24(), &state, 0).command_band.height, 10);

        state.pane_sizes.band_height = 99;
        let pl = compute_pane_layout(area80x24(), &state, 0);
        assert_eq!(
            pl.command_band.height,
            crate::render::command_band::MAX_BAND_ROWS,
            "clamped to MAX_BAND_ROWS"
        );
        assert!(pl.story.height > 0, "the story pane always survives");

        // A tiny terminal wins over the configured height.
        state.pane_sizes.band_height = 14;
        let tiny = compute_pane_layout(Rect::new(0, 0, 80, 10), &state, 0);
        assert!(tiny.command_band.height <= 6, "band shrinks on a short screen");
        assert!(tiny.story.height > 0);
    }

    #[test]
    fn split_ratio_configurable_matches_manual_percentage_split() {
        // A non-default split_ratio (70/30) must match a manual
        // Percentage(70)/Percentage(30) split of the same panes_area exactly.
        let area = area80x24();
        let mut state = AppState::default();
        state.pane_sizes.split_ratio = 70;
        let pl = compute_pane_layout(area, &state, 0);

        let panes_area = Rect::new(0, 0, 80, 23);
        let chunks = RatatuiLayout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
            .split(panes_area);

        assert_eq!(pl.story, chunks[0]);
        assert_eq!(pl.map, chunks[1]);
    }

    // ── Room dock (SQ-0692) ───────────────────────────────────────────────────

    fn open_room_dock(state: &mut AppState) {
        state.room_dock.toggle_to(true, true); // instant open → fraction() == 1.0
    }

    /// The dock is carved out of the MAP pane's bottom, not the frame's: the story
    /// pane keeps every row it had, and the map above the dock shrinks by exactly
    /// the dock's height.
    #[test]
    fn room_dock_carves_the_map_pane_not_the_frame() {
        let mut state = AppState::default();
        let before = compute_pane_layout(area80x24(), &state, 0);
        assert_eq!(before.room_dock, Rect::default(), "closed → zero area");

        open_room_dock(&mut state);
        let after = compute_pane_layout(area80x24(), &state, 0);
        assert!(after.room_dock.height > 0, "open → rows reserved");
        assert_eq!(after.story, before.story, "the story pane is untouched");
        assert_eq!(after.map.x, before.map.x);
        assert_eq!(after.map.width, before.map.width, "and the map keeps its columns");
        assert_eq!(
            after.map.height + after.room_dock.height,
            before.map.height,
            "the dock's rows come straight out of the map pane"
        );
        assert_eq!(after.room_dock.y, after.map.bottom(), "…off its bottom");
        assert_eq!(after.room_dock.x, after.map.x, "spanning the map's columns only");
        assert_eq!(after.room_dock.width, after.map.width);
    }

    /// A layout with no map pane has nowhere to dock, and the debug inspector owns
    /// that slot when it is up — neither may sprout a room dock.
    #[test]
    fn room_dock_needs_a_map_pane_to_dock_in() {
        let mut state = AppState::default();
        open_room_dock(&mut state);
        state.layout = Layout::TranscriptFull;
        assert_eq!(
            compute_pane_layout(area80x24(), &state, 0).room_dock,
            Rect::default(),
            "no map pane, no dock"
        );
    }

    /// The dock's height follows `room_dock_pct`, is floored at a readable
    /// minimum, and is capped so the map pane always survives.
    #[test]
    fn room_dock_height_follows_config_and_clamps() {
        let mut state = AppState::default();
        open_room_dock(&mut state);
        let tall = Rect::new(0, 0, 80, 40);

        state.pane_sizes.room_dock_pct = 25;
        assert_eq!(compute_pane_layout(tall, &state, 0).room_dock.height, 10, "25% of 40 rows");

        state.pane_sizes.room_dock_pct = MIN_ROOM_DOCK_PCT;
        let pl = compute_pane_layout(tall, &state, 0);
        assert_eq!(
            pl.room_dock.height,
            crate::render::room_dock::MIN_ROOM_DOCK_ROWS,
            "a tiny percentage still gets the readable minimum"
        );

        // On a SHORT frame the percentage would swallow the pane, so the map's own
        // floor binds instead — the assertion is only meaningful where the two
        // limits actually disagree (80% of 12 rows = 9, but an 11-row map pane can
        // only spare 8).
        state.pane_sizes.room_dock_pct = MAX_ROOM_DOCK_PCT;
        let short = Rect::new(0, 0, 80, 12);
        let pl = compute_pane_layout(short, &state, 0);
        assert_eq!(
            pl.room_dock.height,
            11 - crate::render::room_dock::MIN_MAP_ROWS,
            "the map's floor binds before the percentage does"
        );
        assert_eq!(pl.map.height, crate::render::room_dock::MIN_MAP_ROWS, "the map pane survives");
        assert_eq!(pl.map.height + pl.room_dock.height, 11, "and the two still tile the pane");
    }

    /// The dock's top edge is a draggable boundary — the pane border above it plus
    /// its own top border, spanning the MAP pane's columns only.
    #[test]
    fn room_dock_top_is_a_grab_zone_over_the_map_columns() {
        let mut state = AppState::default();
        open_room_dock(&mut state);
        let pl = compute_pane_layout(area80x24(), &state, 0);
        let zones = pl.boundary_zones();

        let z = zones
            .iter()
            .find(|z| z.boundary == Boundary::RoomDockTop)
            .expect("room panel top zone");
        assert_eq!(z.rect.y, pl.room_dock.y - 1, "the map pane's bottom border row");
        assert_eq!(z.rect.height, 2, "plus the dock's own top border");
        assert_eq!(z.rect.x, pl.map.x, "the map's columns, not the frame's");
        assert_eq!(z.rect.width, pl.map.width);
        // Inside the map's columns the edge grabs the dock; the splitter still wins
        // its own two columns (it is pushed first).
        let mid = pl.map.x + pl.map.width / 2;
        assert_eq!(boundary_at(&zones, mid, pl.room_dock.y), Some(Boundary::RoomDockTop));
        assert_eq!(boundary_at(&zones, mid, pl.room_dock.y + 1), None, "inside the dock");
    }

    #[test]
    fn panes_area_reconstructs_union_across_layouts() {
        for layout in [Layout::Split, Layout::TranscriptFull] {
            let mut state = AppState::default();
            state.layout = layout;
            let pl = compute_pane_layout(area80x24(), &state, 0);
            assert_eq!(pl.panes_area(), Rect::new(0, 0, 80, 23), "{layout:?}");
        }
    }

    // ── grab_zone_cells (SQ-1327) ─────────────────────────────────────────────

    /// The shipped default must reproduce lanthorn's original, unconfigurable
    /// zones exactly — a mouse user sees no change.
    #[test]
    fn default_grab_zone_cells_matches_todays_pinned_zones() {
        let state = AppState::default();
        assert_eq!(state.config.grab_zone_cells, 2, "the shipped default");
        let pl = compute_pane_layout(area80x24(), &state, 0);
        assert_eq!(pl.grab_zone_cells, 2);

        let zones = pl.boundary_zones();
        let split = zones.iter().find(|z| z.boundary == Boundary::StoryMapSplit).unwrap();
        assert_eq!(
            split.rect,
            Rect::new(pl.story.right() - 1, pl.story.y, 2, pl.story.height),
            "one column either side of the splitter, same as before this knob existed"
        );
    }

    /// A wider `grab_zone_cells` reaches further from the splitter on both
    /// sides — a press two cells away, unreachable at the default of 2, now
    /// grabs it.
    #[test]
    fn wider_grab_zone_cells_reaches_further_from_the_splitter() {
        let mut state = AppState::default();
        state.config.grab_zone_cells = 4;
        let pl = compute_pane_layout(area80x24(), &state, 0);
        let zones = pl.boundary_zones();

        assert_eq!(boundary_at(&zones, pl.story.right() - 2, 5), Some(Boundary::StoryMapSplit));
        assert_eq!(boundary_at(&zones, pl.map.x + 1, 5), Some(Boundary::StoryMapSplit));
        // Still bounded: three cells out either way is past a 4-wide zone.
        assert_eq!(boundary_at(&zones, pl.story.right() - 3, 5), None);
        assert_eq!(boundary_at(&zones, pl.map.x + 2, 5), None);
    }

    /// A wider zone also reaches further above the inventory dock's own edge.
    #[test]
    fn wider_grab_zone_cells_reaches_further_above_a_dock_edge() {
        let mut state = AppState::default();
        state.show_inventory = true;
        state.inv_dock.toggle_to(true, true);
        state.config.grab_zone_cells = 4;
        let pl = compute_pane_layout(area80x24(), &state, 3);
        let zones = pl.boundary_zones();

        assert_eq!(boundary_at(&zones, 10, pl.inv_dock.y - 2), Some(Boundary::InvDockTop));
        assert_eq!(boundary_at(&zones, 10, pl.inv_dock.y - 3), None, "past the 4-wide zone");
    }

    /// Out-of-range config values clamp rather than producing a zero-width or
    /// runaway zone.
    #[test]
    fn grab_zone_cells_clamps_to_a_sane_range() {
        let mut state = AppState::default();
        state.config.grab_zone_cells = 0;
        assert_eq!(
            compute_pane_layout(area80x24(), &state, 0).grab_zone_cells,
            MIN_GRAB_ZONE_CELLS
        );

        state.config.grab_zone_cells = 99;
        assert_eq!(
            compute_pane_layout(area80x24(), &state, 0).grab_zone_cells,
            MAX_GRAB_ZONE_CELLS
        );
    }

    /// The borderless command band (SQ-0667) keeps its snug one-row grab no
    /// matter how wide the knob is set — widening it would swallow clicks on
    /// the band's own column headers.
    #[test]
    fn command_band_top_grab_zone_ignores_grab_zone_cells() {
        let mut state = AppState::default();
        open_band(&mut state);
        state.config.grab_zone_cells = MAX_GRAB_ZONE_CELLS;
        let pl = compute_pane_layout(area80x24(), &state, 0);
        let zones = pl.boundary_zones();

        let z = zones
            .iter()
            .find(|z| z.boundary == Boundary::CommandBandTop)
            .expect("command panel top zone");
        assert_eq!(z.rect.height, 1, "stays a single row regardless of the knob");
        assert_eq!(z.rect.y, pl.command_band.y - 1);
        assert_eq!(
            boundary_at(&zones, 10, pl.command_band.y),
            None,
            "the header row stays clickable even at the widest setting"
        );
    }
}
