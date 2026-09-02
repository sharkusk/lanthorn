//! Mouse drag-resize for the pane boundaries (SQ-0669).
//!
//! Resize mode (`/resize-panes`) moves the same three sizes with the arrow keys;
//! this is the direct-manipulation path for them. The two agree by construction:
//! both clamp to the limits in [`crate::layout`] and both mirror into
//! `state.config` through [`AppState::sync_pane_sizes_to_config`].
//!
//! **The Down owns the whole drag.** What the pointer was over when the left
//! button went down decides what the gesture means, and nothing re-decides until
//! the button comes back up: a Down on a boundary claims every following mouse
//! event (so it can never also start a transcript selection, a band click or a
//! map click), and a Down anywhere else leaves the drag machine alone (so a
//! selection that crosses a boundary keeps selecting). Only `Drag(Left)`
//! continues a claimed drag — every other event ends it, which is also how a
//! release the terminal never reported (button let go outside the window) or an
//! interrupting keypress recover instead of wedging the pointer to the splitter.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::layout::{
    boundary_at, dock_pct_for_rows, split_pct_for_story_width, Boundary, BoundaryZone, PaneLayout,
};
use crate::render::controls::{control_at, BorderControl};
use crate::render::command_band::{MAX_BAND_ROWS, MIN_BAND_ROWS};
use crate::state::AppState;

/// A drag in progress: which boundary is held, where the pointer grabbed it, and
/// the geometry the pointer delta is converted against.
///
/// The anchors are captured ONCE, at Down, and the live size is recomputed from
/// the pointer's absolute position each event — never accumulated. An
/// accumulating drag drifts as soon as one conversion rounds, and returning the
/// pointer to where it started would then not restore the size it started at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneDrag {
    pub boundary: Boundary,
    /// Pointer coordinate at Down: the column for the splitter, the row for the
    /// horizontal edges.
    pub origin: u16,
    /// The size the boundary had at Down, in cells: the story pane's width for
    /// the splitter, the band's height for the horizontal edges.
    pub start_cells: u16,
    /// The area the conversion inverts against: the story+map region for the
    /// splitter, the whole frame for the inventory dock (whose height is a
    /// percentage of it). Unused by the command band, which is sized in rows.
    pub area: Rect,
}

/// What the drag machine did with a mouse event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragOutcome {
    /// Not a boundary gesture — the caller routes the event as usual.
    Ignored,
    /// Claimed by a live drag; the caller must swallow the event.
    Consumed,
    /// Claimed, and the drag finished: the caller must swallow the event AND
    /// flush the pending config write.
    Committed,
}

/// Route one mouse event to the drag machine.
///
/// `pl` is the pane geometry of the last drawn frame (the drag's anchor), and
/// `zones` its grab zones (`PaneLayout::boundary_zones`, cached per frame
/// alongside the other hit-rects). `controls` are the story pane's border
/// toggles, which OVERLAP a grab zone and take priority inside their own cells —
/// see [`on_mouse`]'s Down arm.
pub fn on_mouse(
    state: &mut AppState,
    m: &MouseEvent,
    pl: &PaneLayout,
    zones: &[BoundaryZone],
    controls: &[(BorderControl, Rect)],
) -> DragOutcome {
    if state.pane_drag.is_some() {
        // A live drag owns the mouse. Only continued motion with the button
        // still held keeps it alive.
        return match m.kind {
            MouseEventKind::Drag(MouseButton::Left) => {
                track(state, m.column, m.row);
                DragOutcome::Consumed
            }
            MouseEventKind::Up(_) => {
                track(state, m.column, m.row);
                commit(state);
                DragOutcome::Committed
            }
            // Anything else means the button is no longer down (a plain `Moved`
            // is reported only with no button held, so it is how a release
            // outside the window comes back to us): keep the size the pointer
            // last asked for and end the drag.
            _ => {
                commit(state);
                DragOutcome::Committed
            }
        };
    }

    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // A modal owns the mouse; boundaries under it are not grabbable.
            if state.any_modal_overlay_open() {
                return DragOutcome::Ignored;
            }
            // A border control owns its own cell (SQ-1123). The bottom-border
            // cluster shares its row with the command band's and the inventory
            // dock's grab zone — the layout puts each band's first row directly
            // under the story pane, so `band.y - 1` IS the pane's bottom border —
            // and a click on a toggle has to toggle rather than start a one-row
            // resize it would then commit unchanged, swallowing the click whole.
            //
            // Declining here (rather than arming a click-vs-drag gesture) is the
            // deliberate trade: the zone is a whole pane-width row and the two
            // clusters take about eight columns out of the middle and right of
            // it, so the edge stays easy to grab either side of them. What is
            // lost is a drag STARTED on a toggle, which is a gesture nobody
            // needs when the same edge is grabbable one column over — unlike the
            // vertical splitter, where a control at the midpoint would have made
            // the midpoint itself undraggable. No control is placed on the
            // splitter's columns for exactly that reason.
            if control_at(state, controls, m.column, m.row).is_some() {
                return DragOutcome::Ignored;
            }
            match boundary_at(zones, m.column, m.row) {
                Some(boundary) => {
                    state.pane_drag = Some(anchor(boundary, pl, m.column, m.row));
                    state.pane_hover = Some(boundary);
                    DragOutcome::Consumed
                }
                None => DragOutcome::Ignored,
            }
        }
        // Hover: pointer motion with no button held just lights the boundary up
        // (see `AppState::boundary_active`). It never claims the event — the
        // debug panel's own hover tooltips still need it.
        MouseEventKind::Moved => {
            state.pane_hover = if state.any_modal_overlay_open()
                || control_at(state, controls, m.column, m.row).is_some()
            {
                // Over a control the boundary must not light up either, or the
                // pointer would advertise a drag the Down arm above declines.
                None
            } else {
                boundary_at(zones, m.column, m.row)
            };
            DragOutcome::Ignored
        }
        _ => DragOutcome::Ignored,
    }
}

/// End a live drag because something that is not a mouse event happened (a key,
/// a terminal resize, a game-driven repaint). Returns true when a drag was
/// actually committed, so the caller can flush the config write.
pub fn interrupt(state: &mut AppState) -> bool {
    if state.pane_drag.is_none() {
        return false;
    }
    commit(state);
    true
}

/// Capture the anchors for a drag that starts at `col`/`row`.
fn anchor(boundary: Boundary, pl: &PaneLayout, col: u16, row: u16) -> PaneDrag {
    match boundary {
        Boundary::StoryMapSplit => PaneDrag {
            boundary,
            origin: col,
            start_cells: pl.story.width,
            area: pl.panes_area(),
        },
        Boundary::InvDockTop => PaneDrag {
            boundary,
            origin: row,
            start_cells: pl.inv_dock.height,
            area: pl.frame,
        },
        Boundary::CommandBandTop => PaneDrag {
            boundary,
            origin: row,
            start_cells: pl.command_band.height,
            area: pl.frame,
        },
        // The room dock lives inside the map pane but is SIZED against the frame
        // (see `PaneSizes::room_dock_pct`), so the inversion area is the frame —
        // the same one `dock_pct_for_rows` is asked about at layout time.
        Boundary::RoomDockTop => PaneDrag {
            boundary,
            origin: row,
            start_cells: pl.room_dock.height,
            area: pl.frame,
        },
    }
}

/// Apply the pointer's current position to the held boundary's size.
fn track(state: &mut AppState, col: u16, row: u16) {
    let Some(d) = state.pane_drag else { return };
    match d.boundary {
        // Rightward pointer motion widens the story pane by that many columns;
        // the pct that lands the splitter closest to there is found by inverting
        // the layout's own split.
        Boundary::StoryMapSplit => {
            let want = (d.start_cells as i32 + (col as i32 - d.origin as i32))
                .clamp(0, d.area.width as i32) as u16;
            state.pane_sizes.split_ratio = split_pct_for_story_width(d.area, want);
        }
        // The docks grow UPWARD, so a pointer moving up (smaller row) adds rows.
        Boundary::InvDockTop => {
            let want = (d.start_cells as i32 + (d.origin as i32 - row as i32))
                .clamp(0, d.area.height as i32) as u16;
            state.pane_sizes.inv_dock_pct = dock_pct_for_rows(d.area.height, want);
        }
        Boundary::CommandBandTop => {
            let want = (d.start_cells as i32 + (d.origin as i32 - row as i32))
                .clamp(MIN_BAND_ROWS as i32, MAX_BAND_ROWS as i32) as u16;
            state.pane_sizes.band_height = want;
        }
        Boundary::RoomDockTop => {
            let want = (d.start_cells as i32 + (d.origin as i32 - row as i32))
                .clamp(0, d.area.height as i32) as u16;
            state.pane_sizes.room_dock_pct = dock_pct_for_rows(d.area.height, want);
        }
    }
    state.sync_pane_sizes_to_config();
}

/// Release the boundary and persist the size through resize mode's own path.
fn commit(state: &mut AppState) {
    state.pane_drag = None;
    state.sync_pane_sizes_to_config();
    state.pending_config_write = true;
}

#[cfg(all(test, feature = "t-input"))]
mod tests {
    use super::*;
    use crate::layout::{
        compute_pane_layout, split_story_map, Boundary, MAX_SPLIT_PCT, MIN_SPLIT_PCT,
    };
    use crate::render::command_band::{default_quick, default_verbs};
    use crate::state::{AppState, CommandBandState, Layout};
    use crossterm::event::KeyModifiers;
    use ratatui::layout::Rect;

    /// Most cases here have no border controls on screen; the ones that do say
    /// so explicitly (see `a_border_control_keeps_its_own_cell_out_of_the_drag`).
    const NO_CONTROLS: &[(BorderControl, Rect)] = &[];

    fn ev(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE }
    }
    fn down(col: u16, row: u16) -> MouseEvent {
        ev(MouseEventKind::Down(MouseButton::Left), col, row)
    }
    fn drag(col: u16, row: u16) -> MouseEvent {
        ev(MouseEventKind::Drag(MouseButton::Left), col, row)
    }
    fn up(col: u16, row: u16) -> MouseEvent {
        ev(MouseEventKind::Up(MouseButton::Left), col, row)
    }

    fn area(w: u16, h: u16) -> Rect {
        Rect::new(0, 0, w, h)
    }

    fn open_band(state: &mut AppState) {
        state.overlays.command_band =
            Some(CommandBandState::new(default_verbs(), default_quick()));
        state.band_dock.toggle_to(true, true);
    }

    fn open_dock(state: &mut AppState) {
        state.show_inventory = true;
        state.inv_dock.toggle_to(true, true);
    }

    /// Re-derive the frame's geometry + zones from the current state, the way
    /// the run loop does each frame.
    fn frame(state: &AppState, a: Rect, items: usize) -> (PaneLayout, Vec<BoundaryZone>) {
        let pl = compute_pane_layout(a, state, items);
        let zones = pl.boundary_zones();
        (pl, zones)
    }

    // ── Zone derivation ──────────────────────────────────────────────────────

    #[test]
    fn splitter_zone_is_the_two_drawn_border_columns() {
        let s = AppState::default();
        let (pl, zones) = frame(&s, area(80, 24), 0);
        let z = zones
            .iter()
            .find(|z| z.boundary == Boundary::StoryMapSplit)
            .expect("split layout has a splitter");
        assert_eq!(z.rect.x, pl.story.right() - 1, "story's right border column");
        assert_eq!(z.rect.width, 2, "plus the map's left border column");
        assert_eq!(z.rect.y, pl.story.y);
        assert_eq!(z.rect.height, pl.story.height, "the full height of the panes");
        // Both columns grab it; the columns either side do not.
        assert_eq!(boundary_at(&zones, pl.story.right() - 1, 5), Some(Boundary::StoryMapSplit));
        assert_eq!(boundary_at(&zones, pl.map.x, 5), Some(Boundary::StoryMapSplit));
        assert_eq!(boundary_at(&zones, pl.story.right() - 2, 5), None);
        assert_eq!(boundary_at(&zones, pl.map.x + 1, 5), None);
    }

    #[test]
    fn no_splitter_zone_without_a_split() {
        let mut s = AppState::default();
        s.layout = Layout::TranscriptFull;
        let (_, zones) = frame(&s, area(80, 24), 0);
        assert!(zones.iter().all(|z| z.boundary != Boundary::StoryMapSplit));
    }

    #[test]
    fn dock_zone_is_the_pane_border_plus_the_docks_own_border() {
        let mut s = AppState::default();
        open_dock(&mut s);
        let (pl, zones) = frame(&s, area(80, 24), 3);
        assert!(pl.inv_dock.height > 0);
        let z = zones
            .iter()
            .find(|z| z.boundary == Boundary::InvDockTop)
            .expect("an open dock has a top edge");
        assert_eq!(z.rect.y, pl.inv_dock.y - 1, "the story pane's bottom border row");
        assert_eq!(z.rect.height, 2, "plus the dock's own top border row");
        assert_eq!(z.rect.width, pl.inv_dock.width, "full width");
        assert_eq!(boundary_at(&zones, 10, pl.inv_dock.y), Some(Boundary::InvDockTop));
        assert_eq!(boundary_at(&zones, 10, pl.inv_dock.y - 1), Some(Boundary::InvDockTop));
        assert_eq!(boundary_at(&zones, 10, pl.inv_dock.y + 1), None, "inside the dock");
    }

    /// The band is borderless (SQ-0667), so its zone is only the pane-border row
    /// above it — the band's own first row belongs to its column headers.
    #[test]
    fn band_zone_is_the_single_row_above_it() {
        let mut s = AppState::default();
        open_band(&mut s);
        let (pl, zones) = frame(&s, area(80, 24), 0);
        let z = zones
            .iter()
            .find(|z| z.boundary == Boundary::CommandBandTop)
            .expect("an open band has a top edge");
        assert_eq!(z.rect, Rect::new(pl.command_band.x, pl.command_band.y - 1, pl.command_band.width, 1));
        assert_eq!(boundary_at(&zones, 10, pl.command_band.y), None, "the header row stays clickable");
    }

    #[test]
    fn closed_docks_have_no_zones() {
        let s = AppState::default();
        let (_, zones) = frame(&s, area(80, 24), 0);
        assert!(zones.iter().all(|z| z.boundary == Boundary::StoryMapSplit));
    }

    // ── Down claims the drag; other Downs do not ─────────────────────────────

    #[test]
    fn down_on_the_splitter_claims_the_drag_and_starts_no_selection() {
        let mut s = AppState::default();
        let (pl, zones) = frame(&s, area(80, 24), 0);
        let x = pl.story.right() - 1;
        assert_eq!(on_mouse(&mut s, &down(x, 6), &pl, &zones, NO_CONTROLS), DragOutcome::Consumed);
        assert!(s.pane_drag.is_some(), "the boundary is held");
        // The claimed Down never reaches the selection path, so nothing selects.
        assert!(s.selection.is_none());
        // …and every following drag stays claimed.
        assert_eq!(on_mouse(&mut s, &drag(x + 3, 6), &pl, &zones, NO_CONTROLS), DragOutcome::Consumed);
        assert_eq!(on_mouse(&mut s, &up(x + 3, 6), &pl, &zones, NO_CONTROLS), DragOutcome::Committed);
        assert!(s.pane_drag.is_none());
    }

    #[test]
    fn down_in_the_transcript_is_ignored_and_a_drag_across_the_boundary_stays_ignored() {
        let mut s = AppState::default();
        let (pl, zones) = frame(&s, area(80, 24), 0);
        // Down well inside the story pane: the drag machine passes.
        assert_eq!(on_mouse(&mut s, &down(4, 6), &pl, &zones, NO_CONTROLS), DragOutcome::Ignored);
        assert!(s.pane_drag.is_none());
        // The selection then drags straight THROUGH the splitter and out the
        // other side — still ignored, so ExtendSelection keeps running.
        for x in [pl.story.right() - 1, pl.map.x, pl.map.x + 5] {
            assert_eq!(on_mouse(&mut s, &drag(x, 6), &pl, &zones, NO_CONTROLS), DragOutcome::Ignored, "x={x}");
            assert!(s.pane_drag.is_none());
        }
        assert_eq!(on_mouse(&mut s, &up(pl.map.x + 5, 6), &pl, &zones, NO_CONTROLS), DragOutcome::Ignored);
    }

    #[test]
    fn a_modal_overlay_blocks_the_grab() {
        let mut s = AppState::default();
        s.overlays.hotkey_dialog = true;
        let (pl, zones) = frame(&s, area(80, 24), 0);
        let x = pl.story.right() - 1;
        assert_eq!(on_mouse(&mut s, &down(x, 6), &pl, &zones, NO_CONTROLS), DragOutcome::Ignored);
        assert!(s.pane_drag.is_none());
    }

    // ── Conversion: the splitter tracks the pointer ──────────────────────────

    /// Dragging the splitter one column moves the split one column — at every
    /// width where a whole-percent split ratio can express a single column
    /// (panes no wider than 100 cells).
    #[test]
    fn one_column_of_drag_moves_the_split_one_column() {
        for w in [60u16, 80, 100] {
            for step in [1i32, 2, 3, -1, -2, -3] {
                let mut s = AppState::default();
                let (pl, zones) = frame(&s, area(w, 24), 0);
                let start_x = pl.story.right() - 1;
                let start_w = pl.story.width;
                on_mouse(&mut s, &down(start_x, 6), &pl, &zones, NO_CONTROLS);
                let to = (start_x as i32 + step) as u16;
                on_mouse(&mut s, &drag(to, 6), &pl, &zones, NO_CONTROLS);
                let after = compute_pane_layout(area(w, 24), &s, 0);
                assert_eq!(
                    after.story.width as i32,
                    start_w as i32 + step,
                    "width {w}: a {step}-column drag must move the splitter {step} columns"
                );
                on_mouse(&mut s, &up(to, 6), &pl, &zones, NO_CONTROLS);
            }
        }
    }

    /// Wider than 100 columns a percent is coarser than a cell, so the exact
    /// width is not always reachable — the drag must land on the CLOSEST
    /// achievable split, never merely a plausible one.
    #[test]
    fn a_wide_split_lands_on_the_closest_achievable_column() {
        let panes = Rect::new(0, 0, 200, 30);
        for want in [60u16, 61, 99, 100, 101, 137] {
            let pct = split_pct_for_story_width(panes, want);
            let got = split_story_map(panes, pct).0.width as i32;
            let best = (MIN_SPLIT_PCT..=MAX_SPLIT_PCT)
                .map(|p| (split_story_map(panes, p).0.width as i32 - want as i32).abs())
                .min()
                .unwrap();
            assert_eq!((got - want as i32).abs(), best, "want={want} → pct={pct}");
        }
    }

    #[test]
    fn splitter_drag_clamps_at_the_resize_mode_limits() {
        let mut s = AppState::default();
        let (pl, zones) = frame(&s, area(80, 24), 0);
        let x = pl.story.right() - 1;
        // The literals are resize mode's own arrow-key limits: a drag may not
        // reach a split its keyboard twin cannot.
        assert_eq!((MIN_SPLIT_PCT, MAX_SPLIT_PCT), (20, 80));
        // Yank the pointer far past the left edge, then far past the right.
        on_mouse(&mut s, &down(x, 6), &pl, &zones, NO_CONTROLS);
        on_mouse(&mut s, &drag(0, 6), &pl, &zones, NO_CONTROLS);
        assert_eq!(s.pane_sizes.split_ratio, 20);
        on_mouse(&mut s, &drag(79, 6), &pl, &zones, NO_CONTROLS);
        assert_eq!(s.pane_sizes.split_ratio, 80);
        on_mouse(&mut s, &up(79, 6), &pl, &zones, NO_CONTROLS);
        // Both panes survive either extreme.
        for pct in [20u16, 80] {
            s.pane_sizes.split_ratio = pct;
            let pl = compute_pane_layout(area(80, 24), &s, 0);
            assert!(pl.story.width > 0 && pl.map.width > 0, "pct={pct}");
        }
    }

    // ── Border controls vs the grab zone (SQ-1123) ───────────────────────────

    /// The story pane's bottom-border controls sit ON the command band's grab
    /// row: the layout puts the band's first row directly under the pane, so
    /// `band.y - 1` IS the pane's bottom border. Without a rule, a click on the
    /// band toggle would start a one-row resize and commit it unchanged — the
    /// click swallowed, the button dead.
    ///
    /// The rule is that a control owns its own cell. What that costs is a drag
    /// STARTED on a toggle; the assertion below is that one column either side
    /// still grabs the edge, which is what makes that cost acceptable.
    #[test]
    fn a_border_control_keeps_its_own_cell_out_of_the_drag() {
        let mut s = AppState::default();
        open_band(&mut s);
        let (pl, zones) = frame(&s, area(80, 24), 0);
        let y = pl.command_band.y - 1;
        assert_eq!(y, pl.story.bottom() - 1, "the band's edge IS the pane's bottom border");
        // Well clear of the splitter, whose own zone spans the full pane height
        // and would otherwise answer for this row first.
        let cx = pl.story.x + 5;
        assert_eq!(
            boundary_at(&zones, cx, y),
            Some(Boundary::CommandBandTop),
            "…and that row is a grab zone",
        );

        let ctl: &[(BorderControl, Rect)] =
            &[(BorderControl::VerbPanel, Rect::new(cx, y, 1, 1))];

        // On the control: declined, so the click path downstream gets the event.
        assert_eq!(on_mouse(&mut s, &down(cx, y), &pl, &zones, ctl), DragOutcome::Ignored);
        assert!(s.pane_drag.is_none(), "no drag was started under the button");

        // One column either side: the edge is grabbed exactly as before.
        for x in [cx - 1, cx + 1] {
            assert_eq!(
                on_mouse(&mut s, &down(x, y), &pl, &zones, ctl),
                DragOutcome::Consumed,
                "x={x}: the band edge is still draggable beside the control",
            );
            on_mouse(&mut s, &up(x, y), &pl, &zones, ctl);
        }

        // And hovering the control does not light the boundary up, so the pointer
        // never advertises a drag the Down arm declines.
        on_mouse(&mut s, &ev(MouseEventKind::Moved, cx, y), &pl, &zones, ctl);
        assert_eq!(s.pane_hover, None, "the control's cell is not the boundary's");
        on_mouse(&mut s, &ev(MouseEventKind::Moved, cx + 1, y), &pl, &zones, ctl);
        assert_eq!(s.pane_hover, Some(Boundary::CommandBandTop));
    }

    /// A control is only ever declined where one actually IS: a zero-area rect
    /// (a group the pane was too narrow to draw) grabs nothing, and neither does
    /// a control with a modal over it.
    #[test]
    fn an_undrawn_control_does_not_shadow_the_grab_zone() {
        let mut s = AppState::default();
        open_band(&mut s);
        let (pl, zones) = frame(&s, area(80, 24), 0);
        let y = pl.command_band.y - 1;
        let cx = pl.story.x + 5;
        let ctl: &[(BorderControl, Rect)] = &[(BorderControl::VerbPanel, Rect::new(cx, y, 0, 0))];
        assert_eq!(on_mouse(&mut s, &down(cx, y), &pl, &zones, ctl), DragOutcome::Consumed);
        on_mouse(&mut s, &up(cx, y), &pl, &zones, ctl);
    }

    // ── Conversion: the horizontal edges ─────────────────────────────────────

    #[test]
    fn dragging_the_band_edge_moves_it_one_row_per_row() {
        let mut s = AppState::default();
        open_band(&mut s);
        let (pl, zones) = frame(&s, area(80, 24), 0);
        let y = pl.command_band.y - 1;
        let start = pl.command_band.height;
        on_mouse(&mut s, &down(10, y), &pl, &zones, NO_CONTROLS);
        on_mouse(&mut s, &drag(10, y - 2), &pl, &zones, NO_CONTROLS);
        assert_eq!(s.pane_sizes.band_height, start + 2, "up two rows grows it two rows");
        assert_eq!(compute_pane_layout(area(80, 24), &s, 0).command_band.height, start + 2);
        on_mouse(&mut s, &drag(10, y + 1), &pl, &zones, NO_CONTROLS);
        assert_eq!(s.pane_sizes.band_height, start - 1, "and back down shrinks it");
        on_mouse(&mut s, &up(10, y + 1), &pl, &zones, NO_CONTROLS);
    }

    #[test]
    fn band_drag_clamps_to_the_band_row_limits() {
        let mut s = AppState::default();
        open_band(&mut s);
        let (pl, zones) = frame(&s, area(80, 40), 0);
        let y = pl.command_band.y - 1;
        on_mouse(&mut s, &down(10, y), &pl, &zones, NO_CONTROLS);
        on_mouse(&mut s, &drag(10, 0), &pl, &zones, NO_CONTROLS);
        assert_eq!(s.pane_sizes.band_height, MAX_BAND_ROWS);
        on_mouse(&mut s, &drag(10, 39), &pl, &zones, NO_CONTROLS);
        assert_eq!(s.pane_sizes.band_height, MIN_BAND_ROWS);
        on_mouse(&mut s, &up(10, 39), &pl, &zones, NO_CONTROLS);
    }

    /// The dock's height is a percentage of the frame, so the drag inverts that
    /// percentage: pulling the edge up by n rows must show n more rows of dock
    /// (while the item list has rows left to show).
    #[test]
    fn dragging_the_dock_edge_grows_it_by_the_rows_dragged() {
        let items = 20; // plenty, so the cap binds rather than the content
        // Frame heights a percentage does NOT divide evenly, so a conversion
        // that rounded the wrong way would land a row short.
        for h in [24u16, 30, 40] {
            for step in [1u16, 2, 3] {
                let mut s = AppState::default();
                open_dock(&mut s);
                let (pl, zones) = frame(&s, area(80, h), items);
                let start = pl.inv_dock.height;
                let y = pl.inv_dock.y;
                on_mouse(&mut s, &down(10, y), &pl, &zones, NO_CONTROLS);
                on_mouse(&mut s, &drag(10, y - step), &pl, &zones, NO_CONTROLS);
                let after = compute_pane_layout(area(80, h), &s, items);
                assert_eq!(after.inv_dock.height, start + step, "height {h}, up {step}");
                on_mouse(&mut s, &up(10, y - step), &pl, &zones, NO_CONTROLS);
            }
        }
    }

    /// Holding the dock edge still must hold the DOCK still: the pct is
    /// recomputed from the pointer every event, so a conversion that rounded
    /// down would shave a row off a drag that went nowhere.
    #[test]
    fn a_dock_drag_that_goes_nowhere_changes_nothing() {
        for h in [24u16, 27, 30, 33, 40] {
            let mut s = AppState::default();
            open_dock(&mut s);
            let (pl, zones) = frame(&s, area(80, h), 20);
            let start = pl.inv_dock.height;
            let y = pl.inv_dock.y;
            on_mouse(&mut s, &down(10, y), &pl, &zones, NO_CONTROLS);
            on_mouse(&mut s, &drag(10, y), &pl, &zones, NO_CONTROLS);
            on_mouse(&mut s, &up(10, y), &pl, &zones, NO_CONTROLS);
            assert_eq!(
                compute_pane_layout(area(80, h), &s, 20).inv_dock.height,
                start,
                "height {h}"
            );
        }
    }

    #[test]
    fn dock_drag_clamps_to_the_dock_pct_limits() {
        let mut s = AppState::default();
        open_dock(&mut s);
        let (pl, zones) = frame(&s, area(80, 40), 40);
        let y = pl.inv_dock.y;
        // Again the literals are resize mode's arrow-key limits.
        assert_eq!(
            (crate::layout::MIN_INV_DOCK_PCT, crate::layout::MAX_INV_DOCK_PCT),
            (10, 80)
        );
        on_mouse(&mut s, &down(10, y), &pl, &zones, NO_CONTROLS);
        on_mouse(&mut s, &drag(10, 0), &pl, &zones, NO_CONTROLS);
        assert_eq!(s.pane_sizes.inv_dock_pct, 80);
        on_mouse(&mut s, &drag(10, 39), &pl, &zones, NO_CONTROLS);
        assert_eq!(s.pane_sizes.inv_dock_pct, 10);
        on_mouse(&mut s, &up(10, 39), &pl, &zones, NO_CONTROLS);
    }

    /// A drag that ends where it began leaves the layout where it began — the
    /// anchors are captured at Down, so no rounding accumulates.
    #[test]
    fn a_round_trip_drag_restores_the_starting_geometry() {
        let mut s = AppState::default();
        let (pl, zones) = frame(&s, area(80, 24), 0);
        let x = pl.story.right() - 1;
        on_mouse(&mut s, &down(x, 6), &pl, &zones, NO_CONTROLS);
        for to in [x + 7, x + 12, x - 9, x] {
            on_mouse(&mut s, &drag(to, 6), &pl, &zones, NO_CONTROLS);
        }
        on_mouse(&mut s, &up(x, 6), &pl, &zones, NO_CONTROLS);
        assert_eq!(compute_pane_layout(area(80, 24), &s, 0).story, pl.story);
    }

    // ── Commit / interrupt ───────────────────────────────────────────────────

    #[test]
    fn release_commits_the_size_to_config() {
        let mut s = AppState::default();
        let (pl, zones) = frame(&s, area(80, 24), 0);
        let x = pl.story.right() - 1;
        assert!(!s.pending_config_write);
        on_mouse(&mut s, &down(x, 6), &pl, &zones, NO_CONTROLS);
        on_mouse(&mut s, &drag(x + 8, 6), &pl, &zones, NO_CONTROLS);
        assert!(!s.pending_config_write, "nothing is persisted mid-drag");
        assert_eq!(on_mouse(&mut s, &up(x + 8, 6), &pl, &zones, NO_CONTROLS), DragOutcome::Committed);
        assert!(s.pane_drag.is_none(), "the boundary is released");
        assert!(s.pending_config_write, "the release asks for the config write");
        assert_eq!(s.config.split_ratio, s.pane_sizes.split_ratio, "mirrored to config");
        assert!(s.pane_sizes.split_ratio > 50, "…and the drag actually moved it");
    }

    #[test]
    fn band_and_dock_sizes_mirror_to_their_own_config_keys() {
        let mut s = AppState::default();
        open_band(&mut s);
        let (pl, zones) = frame(&s, area(80, 24), 0);
        let y = pl.command_band.y - 1;
        on_mouse(&mut s, &down(10, y), &pl, &zones, NO_CONTROLS);
        on_mouse(&mut s, &drag(10, y - 2), &pl, &zones, NO_CONTROLS);
        on_mouse(&mut s, &up(10, y - 2), &pl, &zones, NO_CONTROLS);
        assert_eq!(s.config.command_band.height, s.pane_sizes.band_height);
        assert!(s.pending_config_write);

        let mut s = AppState::default();
        open_dock(&mut s);
        let (pl, zones) = frame(&s, area(80, 40), 20);
        let y = pl.inv_dock.y;
        on_mouse(&mut s, &down(10, y), &pl, &zones, NO_CONTROLS);
        on_mouse(&mut s, &drag(10, y - 3), &pl, &zones, NO_CONTROLS);
        on_mouse(&mut s, &up(10, y - 3), &pl, &zones, NO_CONTROLS);
        assert_eq!(s.config.inv_dock_pct, s.pane_sizes.inv_dock_pct);
        assert!(s.pending_config_write);
    }

    // ── Room dock (SQ-0692) ──────────────────────────────────────────────────

    fn open_room_dock(state: &mut AppState) {
        state.room_dock.toggle_to(true, true);
    }

    /// Dragging the room dock's top edge upward grows the dock by that many
    /// rows out of the map pane — the same direct-manipulation contract the
    /// inventory dock has, on the boundary inside the map.
    #[test]
    fn dragging_the_room_dock_edge_up_grows_it_by_that_many_rows() {
        let mut s = AppState::default();
        open_room_dock(&mut s);
        let a = area(80, 40);
        let (pl0, _) = frame(&s, a, 0);
        let start = pl0.room_dock.height;
        let y = pl0.room_dock.y;
        let x = pl0.room_dock.x + 5;

        for step in [1u16, 2, 4] {
            let (pl, zones) = frame(&s, a, 0);
            let start = pl.room_dock.height;
            let y = pl.room_dock.y;
            on_mouse(&mut s, &down(x, y), &pl, &zones, NO_CONTROLS);
            on_mouse(&mut s, &drag(x, y - step), &pl, &zones, NO_CONTROLS);
            assert_eq!(
                compute_pane_layout(a, &s, 0).room_dock.height,
                start + step,
                "up {step}"
            );
            on_mouse(&mut s, &up(x, y - step), &pl, &zones, NO_CONTROLS);
        }

        // …and a drag that goes nowhere leaves it exactly where it was.
        let (pl, zones) = frame(&s, a, 0);
        let held = pl.room_dock.height;
        on_mouse(&mut s, &down(x, pl.room_dock.y), &pl, &zones, NO_CONTROLS);
        on_mouse(&mut s, &drag(x, pl.room_dock.y), &pl, &zones, NO_CONTROLS);
        on_mouse(&mut s, &up(x, pl.room_dock.y), &pl, &zones, NO_CONTROLS);
        assert_eq!(compute_pane_layout(a, &s, 0).room_dock.height, held);
        let _ = (start, y, zones);
    }

    /// The drag clamps to the same percentage limits resize mode's arrows use,
    /// and the map pane survives even at the maximum.
    #[test]
    fn room_dock_drag_clamps_and_leaves_the_map_alive() {
        let mut s = AppState::default();
        open_room_dock(&mut s);
        // A SHORT frame, so the map's own floor is what stops the drag rather than
        // the percentage ceiling — on a tall frame the two never disagree and the
        // floor assertion below would be vacuous.
        let a = area(80, 12);
        let (pl, zones) = frame(&s, a, 0);
        let y = pl.room_dock.y;
        let x = pl.room_dock.x + 5;
        assert_eq!(
            (crate::layout::MIN_ROOM_DOCK_PCT, crate::layout::MAX_ROOM_DOCK_PCT),
            (10, 80)
        );
        on_mouse(&mut s, &down(x, y), &pl, &zones, NO_CONTROLS);
        on_mouse(&mut s, &drag(x, 0), &pl, &zones, NO_CONTROLS);
        assert_eq!(s.pane_sizes.room_dock_pct, 80);
        assert_eq!(
            compute_pane_layout(a, &s, 0).map.height,
            crate::render::room_dock::MIN_MAP_ROWS,
            "the map pane keeps its floor even at the maximum dock"
        );
        on_mouse(&mut s, &drag(x, 11), &pl, &zones, NO_CONTROLS);
        assert_eq!(s.pane_sizes.room_dock_pct, 10);
        on_mouse(&mut s, &up(x, 11), &pl, &zones, NO_CONTROLS);
        assert_eq!(s.config.room_dock_pct, s.pane_sizes.room_dock_pct, "mirrored to config");
        assert!(s.pending_config_write);
    }

    /// A release the terminal never delivered (button let go off-window, so the
    /// next thing we see is plain motion) commits — it must not wedge the
    /// splitter to the pointer forever.
    #[test]
    fn a_lost_release_commits_rather_than_wedging() {
        let mut s = AppState::default();
        let (pl, zones) = frame(&s, area(80, 24), 0);
        let x = pl.story.right() - 1;
        on_mouse(&mut s, &down(x, 6), &pl, &zones, NO_CONTROLS);
        on_mouse(&mut s, &drag(x + 6, 6), &pl, &zones, NO_CONTROLS);
        let held = s.pane_sizes.split_ratio;
        assert_eq!(
            on_mouse(&mut s, &ev(MouseEventKind::Moved, x + 20, 6), &pl, &zones, NO_CONTROLS),
            DragOutcome::Committed
        );
        assert!(s.pane_drag.is_none(), "the drag ended");
        assert_eq!(s.pane_sizes.split_ratio, held, "at the last size the pointer asked for");
        // And the mouse is free again: a later motion only sets hover.
        assert_eq!(
            on_mouse(&mut s, &ev(MouseEventKind::Moved, 4, 6), &pl, &zones, NO_CONTROLS),
            DragOutcome::Ignored
        );
        assert_eq!(s.pane_hover, None);
    }

    #[test]
    fn a_non_mouse_interruption_commits_rather_than_wedging() {
        let mut s = AppState::default();
        let (pl, zones) = frame(&s, area(80, 24), 0);
        let x = pl.story.right() - 1;
        assert!(!interrupt(&mut s), "nothing to interrupt when idle");
        on_mouse(&mut s, &down(x, 6), &pl, &zones, NO_CONTROLS);
        on_mouse(&mut s, &drag(x + 5, 6), &pl, &zones, NO_CONTROLS);
        let held = s.pane_sizes.split_ratio;
        assert!(interrupt(&mut s), "a keypress mid-drag ends it");
        assert!(s.pane_drag.is_none());
        assert!(s.pending_config_write);
        assert_eq!(s.pane_sizes.split_ratio, held);
        // The mouse is free: a following Down starts a NEW gesture, not a resume.
        assert_eq!(on_mouse(&mut s, &down(4, 6), &pl, &zones, NO_CONTROLS), DragOutcome::Ignored);
    }

    // ── Affordance ───────────────────────────────────────────────────────────

    #[test]
    fn hover_lights_the_boundary_without_claiming_the_event() {
        let mut s = AppState::default();
        let (pl, zones) = frame(&s, area(80, 24), 0);
        let x = pl.story.right() - 1;
        assert_eq!(
            on_mouse(&mut s, &ev(MouseEventKind::Moved, x, 6), &pl, &zones, NO_CONTROLS),
            DragOutcome::Ignored
        );
        assert_eq!(s.pane_hover, Some(Boundary::StoryMapSplit));
        assert!(s.boundary_active(Boundary::StoryMapSplit));
        on_mouse(&mut s, &ev(MouseEventKind::Moved, 4, 6), &pl, &zones, NO_CONTROLS);
        assert_eq!(s.pane_hover, None);
        assert!(!s.boundary_active(Boundary::StoryMapSplit));
    }

    /// While a boundary is held, only IT is lit — hover on another boundary
    /// cannot steal the highlight mid-drag.
    #[test]
    fn the_held_boundary_owns_the_highlight() {
        let mut s = AppState::default();
        open_band(&mut s);
        let (pl, zones) = frame(&s, area(80, 24), 0);
        let y = pl.command_band.y - 1;
        on_mouse(&mut s, &down(10, y), &pl, &zones, NO_CONTROLS);
        assert!(s.boundary_active(Boundary::CommandBandTop));
        assert!(!s.boundary_active(Boundary::StoryMapSplit));
        on_mouse(&mut s, &drag(pl.story.right() - 1, y - 1), &pl, &zones, NO_CONTROLS);
        assert!(s.boundary_active(Boundary::CommandBandTop), "still the held one");
        assert!(!s.boundary_active(Boundary::StoryMapSplit));
        on_mouse(&mut s, &up(10, y - 1), &pl, &zones, NO_CONTROLS);
    }
}
