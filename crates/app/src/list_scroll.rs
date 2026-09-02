//! `ListScroll`: a selection index plus an animated display offset — the single
//! state machine every selection-list modal (and the story picker) uses instead
//! of a bare `selected: usize` with a per-frame window recompute. Movement
//! clamps the selection, keeps it visible in the viewport with the minimal
//! offset change, and eases the displayed offset via the shared [`ScrollAnim`].
//!
//! [`ScrollAnim`]: crate::state::ScrollAnim

use crossterm::event::KeyCode;

use crate::config::AnimationConfig;
use crate::state::ScrollAnim;

/// One-page step with a 1-row overlap, floored at 1 so paging always advances.
/// `dir > 0` steps forward (down), `dir < 0` backward (up). The single paging
/// helper — `input::page_scroll` delegates here so the math lives in one place.
pub(crate) fn page_step(current: usize, dir: i32, viewport: usize) -> usize {
    let page = viewport.saturating_sub(1).max(1);
    if dir > 0 {
        current.saturating_add(page)
    } else {
        current.saturating_sub(page)
    }
}

/// Half-page step, the vim `Ctrl-U`/`Ctrl-D` convention (SQ-1228): `floor(viewport
/// / 2)`, floored at 1 so paging always advances. `dir > 0` steps forward (down),
/// `dir < 0` backward (up). No overlap row — unlike `page_step`, a half page is
/// exactly half the viewport.
pub(crate) fn half_page_step(current: usize, dir: i32, viewport: usize) -> usize {
    let page = (viewport / 2).max(1);
    if dir > 0 {
        current.saturating_add(page)
    } else {
        current.saturating_sub(page)
    }
}

/// Minimal offset so `selected` is within `[offset, offset + viewport)`.
fn ensure_visible(offset: usize, selected: usize, viewport: usize) -> usize {
    if viewport == 0 {
        offset
    } else if selected < offset {
        selected
    } else if selected >= offset + viewport {
        selected + 1 - viewport
    } else {
        offset
    }
}

/// A list's selection index plus an animated first-visible-row offset.
#[derive(Debug, Default, Clone)]
pub struct ListScroll {
    /// Highlighted item index.
    pub selected: usize,
    /// First visible row (the settled target the displayed offset eases toward).
    offset: usize,
    /// Item count, for clamping.
    total: usize,
    /// Eases the *displayed* offset toward `offset`; `None` when settled/instant.
    anim: Option<ScrollAnim>,
}

impl ListScroll {
    /// A fresh, empty list scroll (selection 0, offset 0).
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the current item count, clamping `selected`/`offset` into range.
    pub fn len(&mut self, total: usize) {
        self.total = total;
        let max = total.saturating_sub(1);
        if self.selected > max {
            self.selected = max;
        }
        if self.offset > max {
            self.offset = max;
        }
    }

    /// Select `idx` (clamped), keeping it visible in `viewport`.
    pub fn select(&mut self, idx: usize, viewport: usize, anim: &AnimationConfig) {
        self.selected = idx.min(self.total.saturating_sub(1));
        self.ensure_visible_and_arm(viewport, anim);
    }

    /// Move the selection by `delta` (clamped to `[0, total-1]`), keeping it visible.
    pub fn move_by(&mut self, delta: isize, viewport: usize, anim: &AnimationConfig) {
        let max = self.total.saturating_sub(1) as isize;
        self.selected = (self.selected as isize + delta).clamp(0, max.max(0)) as usize;
        self.ensure_visible_and_arm(viewport, anim);
    }

    /// Scroll the VIEWPORT by `delta` rows — the mouse wheel — pinning the
    /// selection to the visible window instead of dragging it along (SQ-0831).
    ///
    /// The inverse of [`Self::move_by`]: a key moves the cursor and the offset
    /// follows it, a wheel notch moves the offset and the cursor is clamped
    /// into `[offset, offset + viewport)`, so it rides the top or bottom edge
    /// of the window rather than being carried off it. Two clamps matter:
    ///
    /// * the offset stops at `total - viewport`, not `total - 1` — otherwise
    ///   the list scrolls into empty space past its last item; and
    /// * a list SHORTER than its viewport has nothing to scroll, so the wheel
    ///   is a no-op there — it must not move the cursor as a consolation.
    pub fn scroll_by(&mut self, delta: isize, viewport: usize, anim: &AnimationConfig) {
        if viewport == 0 || self.total == 0 {
            return;
        }
        let max_offset = self.total.saturating_sub(viewport);
        if max_offset == 0 {
            return; // the whole list fits: nothing to scroll, nothing to move
        }
        let new_offset = (self.offset as isize + delta).clamp(0, max_offset as isize) as usize;
        let last_visible = (new_offset + viewport - 1).min(self.total - 1);
        let from = self.display_offset();
        self.offset = new_offset;
        self.selected = self.selected.clamp(new_offset, last_visible);
        self.anim = if from == new_offset { None } else { ScrollAnim::to(from, new_offset, anim) };
    }

    /// Page the selection by ~one `viewport` (1-row overlap). `dir > 0` = PageDown.
    pub fn page(&mut self, dir: i32, viewport: usize, anim: &AnimationConfig) {
        let stepped = page_step(self.selected, dir, viewport);
        self.selected = stepped.min(self.total.saturating_sub(1));
        self.ensure_visible_and_arm(viewport, anim);
    }

    /// Page the selection by half a `viewport` (vim `Ctrl-U`/`Ctrl-D`, SQ-1228).
    /// `dir > 0` = half page down.
    pub fn half_page(&mut self, dir: i32, viewport: usize, anim: &AnimationConfig) {
        let stepped = half_page_step(self.selected, dir, viewport);
        self.selected = stepped.min(self.total.saturating_sub(1));
        self.ensure_visible_and_arm(viewport, anim);
    }

    /// Jump to the first item.
    pub fn home(&mut self, viewport: usize, anim: &AnimationConfig) {
        self.selected = 0;
        self.ensure_visible_and_arm(viewport, anim);
    }

    /// Jump to the last item (`total - 1`).
    pub fn end(&mut self, total: usize, viewport: usize, anim: &AnimationConfig) {
        self.total = total;
        self.selected = total.saturating_sub(1);
        self.ensure_visible_and_arm(viewport, anim);
    }

    /// The animated (rounded) first-visible row to render this frame.
    pub fn display_offset(&self) -> usize {
        self.anim
            .as_ref()
            .map_or(self.offset, |a| a.current().round() as usize)
    }

    /// The settled first-visible row (scrollbar position).
    pub fn target_offset(&self) -> usize {
        self.offset
    }

    /// True while the display offset is still easing toward the target.
    pub fn has_active_animation(&self) -> bool {
        self.anim.as_ref().is_some_and(|a| !a.done())
    }

    /// Drop a completed animation (the offset already holds the target). Called
    /// from the run loop once the tween finishes. Returns `true` iff a running
    /// animation was actually cleared this call — the run loop ORs that into its
    /// redraw flag so the frame at the settled `target_offset()` paints once
    /// (the last painted frame used an eased `display_offset()` at progress <1,
    /// and the redraw gate would otherwise skip the settle frame). (SQ-0305)
    pub fn finalize_if_done(&mut self) -> bool {
        if self.anim.as_ref().is_some_and(|a| a.done()) {
            self.anim = None;
            true
        } else {
            false
        }
    }

    /// Recompute the target offset to keep `selected` visible, then ease toward
    /// it (instant when animation is disabled).
    fn ensure_visible_and_arm(&mut self, viewport: usize, anim: &AnimationConfig) {
        let new_offset = ensure_visible(self.offset, self.selected, viewport);
        let from = self.display_offset();
        self.offset = new_offset;
        self.anim = if from == new_offset {
            None
        } else {
            ScrollAnim::to(from, new_offset, anim)
        };
    }
}

/// Apply a standard list-navigation key — arrows, `j`/`k`, `PageUp`/
/// `PageDown`, `Home`/`End` — to `scroll`, re-recording `total` first (the
/// list a key lands on may have been replaced wholesale since the last
/// keystroke, e.g. a fresh search's results). Returns whether `code` was a
/// nav key at all, so a caller with other bindings on the same keys (a
/// type-to-search fallback, gallery grid movement) can tell a handled key
/// from one it must still dispatch itself.
///
/// THE shared navigation mechanism (SQ-0682): the story picker's list view,
/// the IFDB search modal's hit/option lists, and the command band's per-
/// column `PageUp`/`PageDown`/`Home`/`End` all route through this one
/// function rather than each hand-rolling the same six-key dispatch. (The
/// band's `Up`/`Down` stay bespoke — a first press only arms the highlight
/// without moving, a deliberate quirk this plain step-by-one dispatch
/// doesn't have — but still finish by driving [`ListScroll::select`], the
/// same primitive this function uses, so the viewport follows there too.)
pub fn nav_key(scroll: &mut ListScroll, code: KeyCode, total: usize, viewport: usize, anim: &AnimationConfig) -> bool {
    scroll.len(total);
    match code {
        KeyCode::Up | KeyCode::Char('k') => scroll.move_by(-1, viewport, anim),
        KeyCode::Down | KeyCode::Char('j') => scroll.move_by(1, viewport, anim),
        KeyCode::PageUp => scroll.page(-1, viewport, anim),
        KeyCode::PageDown => scroll.page(1, viewport, anim),
        KeyCode::Home => scroll.home(viewport, anim),
        KeyCode::End => scroll.end(total, viewport, anim),
        _ => return false,
    }
    true
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use crate::anim::Easing;
    fn anim_off() -> AnimationConfig {
        AnimationConfig { enabled: false, easing: Easing::EaseOut, scroll_ms: 0, ..Default::default() }
    }

    #[test]
    fn move_clamps_and_keeps_selection_visible() {
        let mut l = ListScroll::new();
        l.len(20);
        l.move_by(5, 5, &anim_off()); // select 5, viewport 5
        assert_eq!(l.selected, 5);
        // offset advanced so 5 is visible in a 5-row viewport (rows 1..=5)
        assert!(l.target_offset() <= 5 && 5 < l.target_offset() + 5);
        l.move_by(-100, 5, &anim_off()); // clamp to 0
        assert_eq!(l.selected, 0);
        assert_eq!(l.target_offset(), 0);
    }

    #[test]
    fn page_moves_by_a_viewport() {
        let mut l = ListScroll::new();
        l.len(100);
        l.page(1, 10, &anim_off()); // PageDown
        assert!(l.selected >= 9); // ~one page (with overlap)
        let before = l.selected;
        l.page(-1, 10, &anim_off()); // PageUp
        assert!(l.selected < before);
    }

    #[test]
    fn half_page_moves_by_half_a_viewport() {
        let mut l = ListScroll::new();
        l.len(100);
        l.half_page(1, 10, &anim_off()); // Ctrl-D: half page down
        assert_eq!(l.selected, 5); // floor(10 / 2)
        l.half_page(-1, 10, &anim_off()); // Ctrl-U: half page up
        assert_eq!(l.selected, 0);
    }

    #[test]
    fn half_page_step_floors_and_minimum_is_one() {
        assert_eq!(half_page_step(10, 1, 10), 15); // floor(10/2) = 5
        assert_eq!(half_page_step(10, 1, 9), 14); // floor(9/2) = 4
        assert_eq!(half_page_step(10, -1, 10), 5);
        // A 1-row (or 0-row) viewport still steps by at least 1.
        assert_eq!(half_page_step(10, 1, 1), 11);
        assert_eq!(half_page_step(10, 1, 0), 11);
        assert_eq!(half_page_step(0, -1, 10), 0); // saturates at 0
    }

    #[test]
    fn home_end_jump_to_bounds() {
        let mut l = ListScroll::new();
        l.len(50);
        l.end(50, 10, &anim_off());
        assert_eq!(l.selected, 49);
        l.home(10, &anim_off());
        assert_eq!(l.selected, 0);
        assert_eq!(l.target_offset(), 0);
    }

    // ── `scroll_by` (SQ-0831): the wheel moves the WINDOW, and the cursor is
    // clamped into it. ───────────────────────────────────────────────────────

    #[test]
    fn wheel_scrolls_the_window_and_leaves_a_visible_selection_alone() {
        let mut l = ListScroll::new();
        l.len(100);
        // Selection at 0, viewport 10. One notch down scrolls the window; the
        // cursor is no longer at the top, so it is pulled to the new first row.
        l.scroll_by(1, 10, &anim_off());
        assert_eq!(l.target_offset(), 1, "the window moved by one row");
        assert_eq!(l.selected, 1, "the cursor rides the top edge, not off it");

        // A selection in the middle of the window is untouched by a notch.
        l.select(5, 10, &anim_off());
        assert_eq!(l.target_offset(), 1, "5 is already visible in rows 1..11");
        l.scroll_by(1, 10, &anim_off());
        assert_eq!(l.target_offset(), 2);
        assert_eq!(l.selected, 5, "still inside rows 2..12 — the cursor does not move");
    }

    #[test]
    fn wheel_pins_the_cursor_to_the_bottom_edge_scrolling_up() {
        let mut l = ListScroll::new();
        l.len(100);
        l.select(50, 10, &anim_off());
        let off = l.target_offset();
        assert_eq!(off, 41, "select(50) scrolled 50 onto the last row of the window");
        l.scroll_by(-1, 10, &anim_off());
        assert_eq!(l.target_offset(), 40);
        assert_eq!(l.selected, 49, "clamped to the window's LAST row, not carried up with it");
    }

    #[test]
    fn wheel_offset_stops_at_total_minus_viewport_not_total_minus_one() {
        let mut l = ListScroll::new();
        l.len(20);
        // Far more notches than there are rows: the window stops with the last
        // item on its bottom row, never scrolling into empty space past the end.
        l.scroll_by(1000, 5, &anim_off());
        assert_eq!(l.target_offset(), 15, "20 items, 5 rows → last window starts at 15");
        assert_eq!(l.selected, 15, "cursor pinned to the top of that window");
        // …and the other end.
        l.scroll_by(-1000, 5, &anim_off());
        assert_eq!(l.target_offset(), 0);
        assert_eq!(l.selected, 4, "cursor pinned to the bottom of the first window");
    }

    #[test]
    fn wheel_is_a_no_op_when_the_list_fits_the_viewport() {
        let mut l = ListScroll::new();
        l.len(4);
        l.select(2, 10, &anim_off());
        for d in [1, -1, 5, -5] {
            l.scroll_by(d, 10, &anim_off());
            assert_eq!(l.target_offset(), 0, "nothing to scroll: the whole list is on screen");
            assert_eq!(l.selected, 2, "…and the cursor is NOT moved as a consolation");
        }
        // Exactly-fitting is the same case (total == viewport → no scroll room).
        let mut l = ListScroll::new();
        l.len(10);
        l.select(3, 10, &anim_off());
        l.scroll_by(1, 10, &anim_off());
        assert_eq!((l.target_offset(), l.selected), (0, 3));
    }

    #[test]
    fn wheel_on_an_empty_or_zero_height_list_does_nothing() {
        let mut l = ListScroll::new();
        l.len(0);
        l.scroll_by(1, 10, &anim_off());
        assert_eq!((l.target_offset(), l.selected), (0, 0));
        l.len(100);
        l.scroll_by(1, 0, &anim_off()); // viewport not measured yet
        assert_eq!((l.target_offset(), l.selected), (0, 0));
    }

    fn anim_on() -> AnimationConfig {
        AnimationConfig { enabled: true, easing: Easing::EaseOut, scroll_ms: 40, ..Default::default() }
    }

    #[test]
    fn finalize_if_done_reports_cleared_anim() {
        // Idle: nothing to finalize.
        let mut l = ListScroll::new();
        l.len(100);
        assert!(!l.finalize_if_done(), "idle scroll clears nothing");

        // Arm a real (still-running) animation; finalize is a no-op until it ends.
        l.move_by(40, 10, &anim_on());
        assert!(l.has_active_animation(), "a moved list with anim enabled is easing");
        assert!(!l.finalize_if_done(), "a running anim is not cleared");
        assert!(l.has_active_animation(), "still easing after a no-op finalize");

        // Let the tween elapse, then finalize clears it exactly once.
        std::thread::sleep(std::time::Duration::from_millis(60));
        assert!(!l.has_active_animation(), "tween has elapsed");
        assert_eq!(l.display_offset(), l.target_offset(), "settled offset holds target");
        assert!(l.finalize_if_done(), "finishing anim is cleared and reported true");
        assert!(!l.finalize_if_done(), "already cleared → false (idempotent)");
    }

    #[test]
    fn instant_when_animation_disabled() {
        let mut l = ListScroll::new();
        l.len(100);
        l.move_by(40, 10, &anim_off());
        assert_eq!(l.display_offset(), l.target_offset(), "no easing when disabled");
        assert!(!l.has_active_animation());
    }

    // ── `nav_key` (SQ-0682): the shared dispatch every keyboard-driven list
    // surface routes through — the story picker's list view, the IFDB search
    // modal's hit/option lists, and the command band's PageUp/PageDown/Home/
    // End. ──────────────────────────────────────────────────────────────────

    #[test]
    fn nav_key_dispatches_every_bound_key_and_reports_unbound_ones() {
        let mut l = ListScroll::new();
        l.len(20);

        assert!(nav_key(&mut l, KeyCode::Down, 20, 5, &anim_off()));
        assert_eq!(l.selected, 1, "Down moves by one");
        assert!(nav_key(&mut l, KeyCode::Char('j'), 20, 5, &anim_off()));
        assert_eq!(l.selected, 2, "`j` is a Down alias");
        assert!(nav_key(&mut l, KeyCode::Up, 20, 5, &anim_off()));
        assert_eq!(l.selected, 1, "Up moves back by one");
        assert!(nav_key(&mut l, KeyCode::Char('k'), 20, 5, &anim_off()));
        assert_eq!(l.selected, 0, "`k` is an Up alias");

        assert!(nav_key(&mut l, KeyCode::PageDown, 20, 5, &anim_off()));
        assert!(l.selected >= 4, "PageDown pages ~a viewport");
        assert!(nav_key(&mut l, KeyCode::PageUp, 20, 5, &anim_off()));
        assert!(l.selected < 4, "PageUp pages back");

        assert!(nav_key(&mut l, KeyCode::End, 20, 5, &anim_off()));
        assert_eq!(l.selected, 19, "End jumps to the last item");
        assert!(nav_key(&mut l, KeyCode::Home, 20, 5, &anim_off()));
        assert_eq!(l.selected, 0, "Home jumps to the first item");

        assert!(!nav_key(&mut l, KeyCode::Char('x'), 20, 5, &anim_off()), "an unbound key reports false");
        assert!(!nav_key(&mut l, KeyCode::Left, 20, 5, &anim_off()), "…and leaves the selection untouched");
        assert_eq!(l.selected, 0);
    }

    #[test]
    fn nav_key_re_records_total_so_a_replaced_list_reclamps() {
        // Mirrors the IFDB modal's own note: a fresh search reply replaces
        // the list wholesale, and a scroll still clamping against the
        // previous (longer) list would let the selection run off the end of
        // the new, shorter one.
        let mut l = ListScroll::new();
        l.len(50);
        l.select(45, 5, &anim_off());
        assert_eq!(l.selected, 45);

        nav_key(&mut l, KeyCode::Down, 10, 5, &anim_off());
        assert_eq!(l.selected, 9, "clamped into the new, shorter list before moving");
    }
}
