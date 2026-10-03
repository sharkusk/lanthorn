//! Single source of truth for pane geometry: the vertical split that carves
//! out the command band and the help row, and the split of the remaining panes
//! area between the story pane and the Journal (SQ-1684) — the Journal being
//! the tabbed right-hand panel (Map · Room · Inventory) that the map pane, the
//! room dock and the full-width inventory dock were folded into.
//!
//! Extracted from the inline `.constraints(...)` splits that used to live in
//! `main.rs`'s `terminal.draw` closure so the geometry is testable without a
//! full terminal/render stack.

use ratatui::layout::{Constraint, Direction, Layout as RatatuiLayout, Rect};

use crate::journal::JournalTab;
use crate::render::command_band::{band_height, band_target_height};
use crate::state::{AppState, Layout};

/// The resolved pane rects for one frame. `story`/`journal` are the OUTER
/// (pre-frame) rects; they are `Rect::default()` (zero area) when that pane is
/// hidden for the current `Layout`. `command_band` is zero-area when closed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PaneLayout {
    /// The whole frame this layout was computed from. Kept so a drag that
    /// moves the command band's edge by rows can invert against the same
    /// height the layout used (SQ-0669).
    pub frame: Rect,
    pub story: Rect,
    /// The Journal's whole rect (SQ-1684): tab bar plus body. Zero-area when
    /// the layout hides it.
    pub journal: Rect,
    /// The one-row tab bar along the Journal's top. Zero-area while the debug
    /// inspector owns the Journal's slot, or when the Journal is too short to
    /// spare a row.
    pub journal_tabs: Rect,
    /// What is left of `journal` under the tab bar: the rect the active tab
    /// draws into.
    pub journal_body: Rect,
    /// The map pane's outer rect: `journal_body` while the Map tab is up (or
    /// the debug inspector holds the slot), zero-area on every other tab, so
    /// nothing that hit-tests the map has to ask which tab is showing.
    pub map: Rect,
    /// The Map tab's room card (SQ-1688): the strip under `map` while a room is
    /// pinned, zero-area otherwise. `map` has already been shortened by it.
    pub map_card: Rect,
    pub command_band: Rect,
    pub help_row: Rect,
    /// How wide/tall each draggable boundary's grab zone reaches, in cells —
    /// `Config::grab_zone_cells` clamped to `MIN_GRAB_ZONE_CELLS..=
    /// MAX_GRAB_ZONE_CELLS` (SQ-1327). `boundary_zones` reads this; it does not
    /// affect `CommandBandTop`, which always keeps a single-row zone — see its
    /// doc comment.
    pub grab_zone_cells: u16,
}

// ── Draggable pane boundaries (SQ-0669) ───────────────────────────────────────

/// Smallest / largest story share of the story/Journal split, in percent. Resize
/// mode's arrows and the mouse drag clamp to the SAME limits — one definition,
/// so the two ways of moving the splitter can never disagree about its range.
pub const MIN_SPLIT_PCT: u16 = 20;
pub const MAX_SPLIT_PCT: u16 = 80;
/// Smallest / largest width (splitter) or height (band edge) of a draggable
/// boundary's grab zone, in cells. `Config::grab_zone_cells` is clamped to this
/// range when [`compute_pane_layout`] resolves it — a mouse click is a point but
/// a finger is not, so a touch session (the Docker web image on a tablet) wants
/// a wider target than the default (SQ-1327).
pub const MIN_GRAB_ZONE_CELLS: u16 = 1;
pub const MAX_GRAB_ZONE_CELLS: u16 = 6;

/// A pane boundary the mouse can grab and drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boundary {
    /// The vertical splitter between the story pane and the Journal (moves
    /// `split_ratio`).
    StoryMapSplit,
    /// The command band's top edge (moves `command_band.height`).
    CommandBandTop,
}

/// One boundary plus the screen rect that grabs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundaryZone {
    pub boundary: Boundary,
    pub rect: Rect,
}

impl PaneLayout {
    /// The combined story+Journal region before the per-layout split — reconstructs
    /// what was previously called `panes_area` in `main.rs`. Used as a last-resort
    /// overlay target when both panes report zero content height (e.g. a terminal
    /// so small the pane's border consumes all its rows).
    pub fn panes_area(&self) -> Rect {
        let story_empty = self.story.width == 0 && self.story.height == 0;
        let journal_empty = self.journal.width == 0 && self.journal.height == 0;
        match (story_empty, journal_empty) {
            (true, true) => Rect::default(),
            (true, false) => self.journal,
            (false, true) => self.story,
            (false, false) => self.story.union(self.journal),
        }
    }

    /// The draggable boundaries of this frame, with their grab zones.
    ///
    /// A one-cell target is hard to hit with a mouse and harder still with a
    /// finger, so each zone straddles the divider that is actually DRAWN there,
    /// reaching `self.grab_zone_cells` cells out from it in total (default 2,
    /// raised from `Config::grab_zone_cells` for touch — SQ-1327): the splitter
    /// straddles the story pane's right border and the Journal's left edge, which
    /// abut (`story.right() == journal.x`).
    ///
    /// `CommandBandTop` is the exception: it has no border of its own
    /// (SQ-0667 made it a borderless strip), so its zone stays the single
    /// pane-border row above it REGARDLESS of `grab_zone_cells`. Widening it
    /// down into the band would swallow clicks on the band's column headers,
    /// which is a worse trade than a narrow grab.
    ///
    /// The splitter comes first, so a corner cell where it meets the band edge
    /// grabs the splitter (`boundary_at` takes the first match).
    pub fn boundary_zones(&self) -> Vec<BoundaryZone> {
        let mut zones = Vec::new();
        let reach = self.grab_zone_cells;

        // Splitter: only when both panes are actually on screen and adjacent.
        let split_live = self.story.width > 0
            && self.story.height > 0
            && self.journal.width > 0
            && self.journal.height > 0
            && self.story.right() == self.journal.x;
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

        // The band's top edge exists only when the band has rows on screen.
        let band = self.command_band;
        if band.width > 0 && band.height > 0 {
            zones.push(BoundaryZone {
                boundary: Boundary::CommandBandTop,
                rect: Rect::new(band.x, band.y.saturating_sub(1), band.width, 1),
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

/// Split `panes_area` between the story pane and the Journal at `split_ratio`
/// percent.
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

/// Compute this frame's pane geometry.
///
/// The inventory dock that used to reserve a full-width band above the help row
/// is gone (SQ-1684): the story pane keeps those rows, and the inventory is a
/// Journal tab.
pub fn compute_pane_layout(area: Rect, state: &AppState) -> PaneLayout {
    compute_pane_layout_with_card(area, state, 0)
}

/// [`compute_pane_layout`] with the Map tab's room card (SQ-1688) carved off the
/// bottom of the Journal body: `card_rows` is `map_card::rows_for` (0 = no card).
/// The map rect shrinks by that many rows and `map_card` is the strip under it;
/// `compute_pane_layout` is the card-less form, which is what every caller that
/// only wants the STORY geometry uses (the card never touches it).
pub fn compute_pane_layout_with_card(area: Rect, state: &AppState, card_rows: u16) -> PaneLayout {
    // ── Command band: a bottom band under the story pane, above the help row,
    // sliding up when opened (SQ-0664).
    let band_visible = state.command_band_visible();
    let band_target_h = band_target_height(band_visible, area.height, state.pane_sizes.band_height);
    let band_h = band_height(band_target_h, state.band_dock.fraction());

    // ── Reserve the bottom row for the help bar and the command band above it ─
    let vert = RatatuiLayout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(band_h), Constraint::Length(1)])
        .split(area);
    let panes_area = vert[0];
    let band_area = vert[1];
    let help_row = vert[2];

    // The debug inspector tiles into the Journal's slot; make sure a right-slot
    // rect exists for it even when the current layout is TranscriptFull (Journal
    // hidden).
    let debugging = state.debug.is_some();
    let effective_layout = if debugging { Layout::Split } else { state.layout };
    let (story, journal) = match effective_layout {
        Layout::TranscriptFull => (panes_area, Rect::default()),
        Layout::Split => split_story_map(panes_area, state.pane_sizes.split_ratio),
    };

    // ── Journal: a one-row tab bar over the active tab's body. The inspector is
    // not a tab, so while it holds the slot there is no bar and it gets all of
    // it. A Journal too short to spare a row for the bar gives the body
    // everything rather than showing a bar and no body.
    let (journal_tabs, journal_body) = if debugging || journal.height < 2 {
        (Rect::default(), journal)
    } else {
        (
            Rect::new(journal.x, journal.y, journal.width, 1),
            Rect::new(journal.x, journal.y + 1, journal.width, journal.height - 1),
        )
    };
    let mut map = if debugging || state.journal_tab == JournalTab::Map {
        journal_body
    } else {
        Rect::default()
    };
    let mut map_card = Rect::default();
    if !debugging && state.journal_tab == JournalTab::Map && card_rows > 0 && card_rows < map.height {
        let h = map.height - card_rows;
        map_card = Rect::new(map.x, map.y + h, map.width, card_rows);
        map.height = h;
    }

    PaneLayout {
        frame: area,
        story,
        journal,
        journal_tabs,
        journal_body,
        map,
        map_card,
        command_band: band_area,
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
        let pl = compute_pane_layout(area80x24(), &state);

        assert_eq!(pl.command_band.width * pl.command_band.height, 0);

        // Help row is the bottom single row.
        assert_eq!(pl.help_row, Rect::new(0, 23, 80, 1));

        // Story + Journal fill the remaining 23 rows and split the 80 columns ~evenly.
        assert_eq!(pl.story.height, 23);
        assert_eq!(pl.journal.height, 23);
        assert_eq!(pl.story.y, 0);
        assert_eq!(pl.journal.y, 0);
        assert_eq!(pl.story.width + pl.journal.width, 80);
        assert!((pl.story.width as i32 - pl.journal.width as i32).abs() <= 1);
    }

    #[test]
    fn split_matches_manual_split_of_panes_area() {
        // Parity check: reproduce the plain inline computation (panes_area =
        // area minus the 1-row help row) and assert the pure function agrees.
        let area = area80x24();
        let state = AppState::default();
        let pl = compute_pane_layout(area, &state);

        let panes_area = Rect::new(0, 0, 80, 23);
        let chunks = RatatuiLayout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(panes_area);

        assert_eq!(pl.story, chunks[0]);
        assert_eq!(pl.journal, chunks[1]);
    }

    #[test]
    fn transcript_full_hides_the_journal() {
        let mut state = AppState::default();
        state.layout = Layout::TranscriptFull;
        let pl = compute_pane_layout(area80x24(), &state);

        assert_eq!(pl.journal.width * pl.journal.height, 0);
        assert_eq!(pl.journal_tabs, Rect::default());
        assert_eq!(pl.map, Rect::default());
        assert_eq!(pl.story, Rect::new(0, 0, 80, 23));
    }

    #[test]
    fn help_row_always_bottom_single_row() {
        for layout in [Layout::Split, Layout::TranscriptFull] {
            let mut state = AppState::default();
            state.layout = layout;
            let pl = compute_pane_layout(area80x24(), &state);
            assert_eq!(pl.help_row, Rect::new(0, 23, 80, 1), "{layout:?}");
        }
    }

    /// The Journal is a one-row tab bar over a body, and the body is what the
    /// active tab draws into (SQ-1684).
    #[test]
    fn journal_is_a_tab_bar_over_a_body() {
        let state = AppState::default();
        let pl = compute_pane_layout(area80x24(), &state);
        assert_eq!(pl.journal_tabs, Rect::new(pl.journal.x, pl.journal.y, pl.journal.width, 1));
        assert_eq!(pl.journal_body.y, pl.journal.y + 1);
        assert_eq!(pl.journal_body.height, pl.journal.height - 1);
        assert_eq!(pl.journal_body.width, pl.journal.width);
        assert_eq!(pl.journal_body.x, pl.journal.x);
    }

    /// `map` is the Journal body on the Map tab and nothing on any other, so
    /// every map hit-test goes inert when another tab is up.
    #[test]
    fn map_rect_exists_only_on_the_map_tab() {
        let mut state = AppState::default();
        for tab in JournalTab::ALL {
            state.journal_tab = tab;
            let pl = compute_pane_layout(area80x24(), &state);
            assert_eq!(pl.journal_body.height, pl.journal.height - 1, "{tab:?}");
            if tab == JournalTab::Map {
                assert_eq!(pl.map, pl.journal_body);
            } else {
                assert_eq!(pl.map, Rect::default(), "{tab:?}");
            }
        }
    }

    /// The debug inspector owns the Journal's slot outright: no tab bar, all of it.
    #[test]
    fn the_debug_inspector_takes_the_whole_journal_slot() {
        let mut state = AppState::default();
        state.debug = Some(crate::debug_panel::DebugPanelState::new(0x1000));
        state.journal_tab = JournalTab::Inventory;
        let pl = compute_pane_layout(area80x24(), &state);
        assert_eq!(pl.journal_tabs, Rect::default());
        assert_eq!(pl.journal_body, pl.journal);
        assert_eq!(pl.map, pl.journal);
    }

    /// The inventory dock's rows went back to the story pane (SQ-1684): the
    /// Inventory tab changes nothing about the story's height.
    #[test]
    fn the_story_pane_keeps_the_rows_the_inventory_dock_used_to_take() {
        let mut state = AppState::default();
        let before = compute_pane_layout(area80x24(), &state);
        state.journal_tab = JournalTab::Inventory;
        let after = compute_pane_layout(area80x24(), &state);
        assert_eq!(after.story, before.story);
        assert_eq!(after.story.height + after.help_row.height, 24, "only the help row is spent");
    }

    /// The band is a BOTTOM band (SQ-0664): full width, above the help row,
    /// with the story/Journal panes shrinking to make room.
    #[test]
    fn command_band_open_reserves_a_bottom_band() {
        let mut state = AppState::default();
        open_band(&mut state);
        let pl = compute_pane_layout(area80x24(), &state);

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
        // Story and Journal keep the FULL width — no left carve any more.
        assert_eq!(pl.story.x, 0);
        assert_eq!(pl.story.width + pl.journal.width, 80);
    }

    /// The configured height drives the band, clamped so it can never starve
    /// the story pane.
    #[test]
    fn band_height_follows_config_and_clamps() {
        let mut state = AppState::default();
        open_band(&mut state);
        state.pane_sizes.band_height = 10;
        assert_eq!(compute_pane_layout(area80x24(), &state).command_band.height, 10);

        state.pane_sizes.band_height = 99;
        let pl = compute_pane_layout(area80x24(), &state);
        assert_eq!(
            pl.command_band.height,
            crate::render::command_band::MAX_BAND_ROWS,
            "clamped to MAX_BAND_ROWS"
        );
        assert!(pl.story.height > 0, "the story pane always survives");

        // A tiny terminal wins over the configured height.
        state.pane_sizes.band_height = 14;
        let tiny = compute_pane_layout(Rect::new(0, 0, 80, 10), &state);
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
        let pl = compute_pane_layout(area, &state);

        let panes_area = Rect::new(0, 0, 80, 23);
        let chunks = RatatuiLayout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
            .split(panes_area);

        assert_eq!(pl.story, chunks[0]);
        assert_eq!(pl.journal, chunks[1]);
    }

    #[test]
    fn panes_area_reconstructs_union_across_layouts() {
        for layout in [Layout::Split, Layout::TranscriptFull] {
            let mut state = AppState::default();
            state.layout = layout;
            let pl = compute_pane_layout(area80x24(), &state);
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
        let pl = compute_pane_layout(area80x24(), &state);
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
        let pl = compute_pane_layout(area80x24(), &state);
        let zones = pl.boundary_zones();

        assert_eq!(boundary_at(&zones, pl.story.right() - 2, 5), Some(Boundary::StoryMapSplit));
        assert_eq!(boundary_at(&zones, pl.journal.x + 1, 5), Some(Boundary::StoryMapSplit));
        // Still bounded: three cells out either way is past a 4-wide zone.
        assert_eq!(boundary_at(&zones, pl.story.right() - 3, 5), None);
        assert_eq!(boundary_at(&zones, pl.journal.x + 2, 5), None);
    }

    /// Out-of-range config values clamp rather than producing a zero-width or
    /// runaway zone.
    #[test]
    fn grab_zone_cells_clamps_to_a_sane_range() {
        let mut state = AppState::default();
        state.config.grab_zone_cells = 0;
        assert_eq!(compute_pane_layout(area80x24(), &state).grab_zone_cells, MIN_GRAB_ZONE_CELLS);

        state.config.grab_zone_cells = 99;
        assert_eq!(compute_pane_layout(area80x24(), &state).grab_zone_cells, MAX_GRAB_ZONE_CELLS);
    }

    /// The borderless command band (SQ-0667) keeps its snug one-row grab no
    /// matter how wide the knob is set — widening it would swallow clicks on
    /// the band's own column headers.
    #[test]
    fn command_band_top_grab_zone_ignores_grab_zone_cells() {
        let mut state = AppState::default();
        open_band(&mut state);
        state.config.grab_zone_cells = MAX_GRAB_ZONE_CELLS;
        let pl = compute_pane_layout(area80x24(), &state);
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
