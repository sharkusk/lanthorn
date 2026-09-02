//! Shared scrollbar drawing. The single place the ratatui `Scrollbar` idiom
//! lives; every linearly-scrollable surface calls [`draw_scrollbar`] (vertical,
//! right edge) or [`draw_hscrollbar`] (horizontal, bottom edge).
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget},
};

/// True when `total` units do not fit in `viewport` units (so a scrollbar — and
/// a reserved 1-cell gutter — are warranted). Orientation-agnostic: rows for the
/// vertical bar, columns for the horizontal one.
pub fn needs_scrollbar(total: usize, viewport: usize) -> bool {
    total > viewport
}

/// The resolved look of a scrollbar: two BACKGROUND fills, no glyphs (SQ-0782).
///
/// The bar used to be ratatui's default `█` thumb on a `│` track. A full block
/// fills its whole cell, so transcript text one column away had no visual
/// gutter. Drawing thumb and track as spaces carrying a background colour keeps
/// the bar legible without putting a second vertical rule beside the pane
/// border. Both colours are themeable: each selector's FOREGROUND is its fill
/// (`scrollbar` was already the thumb's colour, so an existing
/// `scrollbar = { fg = … }` keeps meaning "the colour of the bar").
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarLook {
    /// The moving thumb's fill colour.
    pub thumb: Color,
    /// The channel the thumb runs in.
    pub track: Color,
}

impl ScrollbarLook {
    /// Read the `scrollbar` (thumb) and `scrollbar_track` (channel) selectors.
    pub fn from_theme(theme: &crate::theme::resolve::Theme) -> Self {
        Self {
            thumb: theme.get("scrollbar").style.fg.unwrap_or(Color::Gray),
            track: theme.get("scrollbar_track").style.fg.unwrap_or(Color::DarkGray),
        }
    }

    /// Blend both fills toward `backdrop`, `opacity` of the way to fully
    /// visible (`1.0` = untouched, `0.0` = the backdrop itself). Used by the
    /// story pane's auto-hide fade.
    ///
    /// A colour with no canonical RGB here (`Reset`, `Indexed`) cannot be
    /// blended, so it is left at full strength and the bar pops instead of
    /// fading — the same degradation the inline-image page path makes.
    pub fn faded(self, opacity: f64, backdrop: Color) -> Self {
        let t = opacity.clamp(0.0, 1.0);
        Self { thumb: blend(self.thumb, backdrop, t), track: blend(self.track, backdrop, t) }
    }
}

/// `fill` mixed `t` of the way from `backdrop` (t = 0) to itself (t = 1).
/// Returns `fill` unchanged when either end has no canonical RGB.
fn blend(fill: Color, backdrop: Color, t: f64) -> Color {
    let (Some(f), Some(b)) = (fill_rgb(fill), fill_rgb(backdrop)) else { return fill };
    let mix = |a: u8, c: u8| (c as f64 + (a as f64 - c as f64) * t).round().clamp(0.0, 255.0) as u8;
    Color::Rgb(mix(f.0, b.0), mix(f.1, b.1), mix(f.2, b.2))
}

/// A colour's RGB, or `None` when it has none here (`Reset`/`Indexed`). Reuses
/// the raster path's table rather than transcribing a second copy: an alpha of
/// zero can only be the fallback, since every resolved colour is opaque.
fn fill_rgb(c: Color) -> Option<(u8, u8, u8)> {
    let px = crate::render::v6_layout::color_to_rgba(c, image::Rgba([0, 0, 0, 0]));
    (px[3] != 0).then_some((px[0], px[1], px[2]))
}

/// Draw a themed vertical scrollbar on the right edge of `area`. No-op when the
/// content fits (`total <= viewport`) or `area` is degenerate. `position` is the
/// index of the first visible row (0-based), ranging `0..=total-viewport`.
///
/// ratatui places the thumb at the track bottom only when
/// `position == content_length - 1`; since `position` ranges `0..=max_scroll`
/// (`max_scroll = total - viewport`), the state's content length is
/// `max_scroll + 1` so the thumb spans the full track, while
/// `viewport_content_length` keeps the thumb proportional.
pub fn draw_scrollbar(
    buf: &mut Buffer,
    area: Rect,
    total: usize,
    viewport: usize,
    position: usize,
    look: ScrollbarLook,
) {
    draw_bar(buf, area, ScrollbarOrientation::VerticalRight, total, viewport, position, look);
}

/// Draw a themed HORIZONTAL scrollbar along the bottom edge of `area`, for a
/// surface that pans sideways rather than scrolling down (the debug inspector's
/// Memory dump, SQ-0974). Same contract as [`draw_scrollbar`] with columns in
/// place of rows: no-op when the content fits, `position` is the leftmost
/// visible column.
///
/// The same [`ScrollbarLook`] — so the `scrollbar` / `scrollbar_track`
/// selectors dress every bar in the app, whichever way it runs. A caller whose
/// `position` may exceed `total - viewport` (the Memory pan clamps against the
/// widest row, not the pane) simply pins the thumb at the far end: ratatui
/// clamps the position into the track rather than drawing off it.
pub fn draw_hscrollbar(
    buf: &mut Buffer,
    area: Rect,
    total: usize,
    viewport: usize,
    position: usize,
    look: ScrollbarLook,
) {
    draw_bar(buf, area, ScrollbarOrientation::HorizontalBottom, total, viewport, position, look);
}

/// The one `Scrollbar` invocation both orientations share: background-only
/// thumb and track (SQ-0782), no arrow heads, position clamped by ratatui.
fn draw_bar(
    buf: &mut Buffer,
    area: Rect,
    orientation: ScrollbarOrientation,
    total: usize,
    viewport: usize,
    position: usize,
    look: ScrollbarLook,
) {
    if !needs_scrollbar(total, viewport) || area.height == 0 || area.width == 0 {
        return;
    }
    let content_len = total.saturating_sub(viewport) + 1;
    let mut sb_state = ScrollbarState::new(content_len)
        .viewport_content_length(viewport)
        .position(position);
    StatefulWidget::render(
        Scrollbar::new(orientation)
            .begin_symbol(None)
            .end_symbol(None)
            .thumb_symbol(" ")
            .track_symbol(Some(" "))
            .thumb_style(Style::default().bg(look.thumb))
            .track_style(Style::default().bg(look.track)),
        area,
        buf,
        &mut sb_state,
    );
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use ratatui::{buffer::Buffer, layout::Rect};

    fn look() -> ScrollbarLook {
        ScrollbarLook { thumb: Color::Cyan, track: Color::DarkGray }
    }

    #[test]
    fn needs_scrollbar_only_when_overflowing() {
        assert!(!needs_scrollbar(5, 10)); // fits
        assert!(!needs_scrollbar(10, 10)); // exactly fits
        assert!(needs_scrollbar(11, 10)); // overflows
    }

    #[test]
    fn draw_scrollbar_noop_when_fits_and_draws_when_overflowing() {
        let area = Rect::new(0, 0, 8, 4);
        // fits -> nothing drawn on the right edge
        let mut b1 = Buffer::empty(area);
        draw_scrollbar(&mut b1, area, 4, 4, 0, look());
        let right_col_plain = (0..area.height)
            .all(|y| b1.cell((area.right() - 1, y)).unwrap().bg == Color::Reset);
        assert!(right_col_plain, "no scrollbar when content fits");
        // overflows -> the right column is painted with the bar's backgrounds
        let mut b2 = Buffer::empty(area);
        draw_scrollbar(&mut b2, area, 40, 4, 0, look());
        let painted = (0..area.height)
            .any(|y| b2.cell((area.right() - 1, y)).unwrap().bg == Color::Cyan);
        assert!(painted, "scrollbar drawn when content overflows");
    }

    /// SQ-0782: the bar is background only — no glyph in any of its cells, so
    /// text one column away has a clear gutter instead of a `█` against it.
    #[test]
    fn draw_scrollbar_paints_backgrounds_and_writes_no_glyphs() {
        let area = Rect::new(0, 0, 8, 6);
        let mut buf = Buffer::empty(area);
        draw_scrollbar(&mut buf, area, 60, 6, 0, look());
        let mut thumb = 0;
        let mut track = 0;
        for y in 0..area.height {
            let cell = buf.cell((area.right() - 1, y)).unwrap();
            assert_eq!(cell.symbol(), " ", "row {y} must carry no glyph");
            match cell.bg {
                Color::Cyan => thumb += 1,
                Color::DarkGray => track += 1,
                other => panic!("row {y} painted with {other:?}"),
            }
        }
        assert!(thumb > 0, "the thumb is painted");
        assert!(track > 0, "the track is painted");
    }

    /// SQ-0974: the horizontal bar is the same idiom turned on its side — it
    /// paints the BOTTOM row of its area, and stays away entirely when the
    /// content fits.
    #[test]
    fn draw_hscrollbar_noop_when_fits_and_paints_the_bottom_row_when_overflowing() {
        let area = Rect::new(0, 0, 10, 3);
        let mut b1 = Buffer::empty(area);
        draw_hscrollbar(&mut b1, area, 10, 10, 0, look());
        let bottom_plain = (0..area.width)
            .all(|x| b1.cell((x, area.bottom() - 1)).unwrap().bg == Color::Reset);
        assert!(bottom_plain, "no scrollbar when the content fits");

        let mut b2 = Buffer::empty(area);
        draw_hscrollbar(&mut b2, area, 100, 10, 0, look());
        let painted = (0..area.width)
            .any(|x| b2.cell((x, area.bottom() - 1)).unwrap().bg == Color::Cyan);
        assert!(painted, "the thumb is painted on the bottom row");
        // …and only the bottom row: rows above it are untouched.
        for y in 0..area.bottom() - 1 {
            for x in 0..area.width {
                assert_eq!(b2.cell((x, y)).unwrap().bg, Color::Reset, "row {y} must be clear");
            }
        }
    }

    /// The whole track is background fill (no `█`, no `│`), exactly as the
    /// vertical bar has been since SQ-0782.
    #[test]
    fn draw_hscrollbar_paints_backgrounds_and_writes_no_glyphs() {
        let area = Rect::new(0, 0, 12, 1);
        let mut buf = Buffer::empty(area);
        draw_hscrollbar(&mut buf, area, 120, 12, 0, look());
        let (mut thumb, mut track) = (0, 0);
        for x in 0..area.width {
            let cell = buf.cell((x, 0)).unwrap();
            assert_eq!(cell.symbol(), " ", "column {x} must carry no glyph");
            match cell.bg {
                Color::Cyan => thumb += 1,
                Color::DarkGray => track += 1,
                other => panic!("column {x} painted with {other:?}"),
            }
        }
        assert!(thumb > 0, "the thumb is painted");
        assert!(track > 0, "the track is painted");
    }

    /// The thumb tracks `position`: hard left at 0, hard right at the end of the
    /// range — and an over-range position (the Memory pan clamps to the widest
    /// row, not the pane) pins there rather than vanishing.
    #[test]
    fn draw_hscrollbar_thumb_tracks_position_at_both_ends() {
        let area = Rect::new(0, 0, 20, 1);
        let thumb_cols = |pos: usize| -> Vec<u16> {
            let mut buf = Buffer::empty(area);
            draw_hscrollbar(&mut buf, area, 100, 20, pos, look());
            (0..area.width).filter(|&x| buf.cell((x, 0)).unwrap().bg == Color::Cyan).collect()
        };
        let left = thumb_cols(0);
        let right = thumb_cols(80);
        assert_eq!(left.first(), Some(&0), "at position 0 the thumb starts at the left edge");
        assert_eq!(
            right.last(),
            Some(&(area.width - 1)),
            "at the end of the range the thumb reaches the right edge",
        );
        assert!(left.last() < right.first(), "the thumb actually moved: {left:?} then {right:?}");
        assert_eq!(thumb_cols(999), right, "an over-range position pins at the far end");
    }

    #[test]
    fn faded_blends_both_fills_toward_the_backdrop() {
        let l = ScrollbarLook { thumb: Color::Rgb(200, 100, 0), track: Color::Rgb(100, 100, 100) };
        assert_eq!(l.faded(1.0, Color::Rgb(0, 0, 0)), l, "full opacity is untouched");
        assert_eq!(
            l.faded(0.5, Color::Rgb(0, 0, 0)),
            ScrollbarLook { thumb: Color::Rgb(100, 50, 0), track: Color::Rgb(50, 50, 50) },
            "half way to the backdrop"
        );
        assert_eq!(
            l.faded(0.0, Color::Rgb(20, 20, 20)),
            ScrollbarLook { thumb: Color::Rgb(20, 20, 20), track: Color::Rgb(20, 20, 20) },
            "zero opacity is the backdrop"
        );
    }

    #[test]
    fn faded_leaves_colours_with_no_canonical_rgb_alone() {
        let l = ScrollbarLook { thumb: Color::Indexed(6), track: Color::DarkGray };
        // Indexed thumb: unblendable. DarkGray track: blends fine, and an
        // unresolvable BACKDROP leaves everything alone.
        assert_eq!(l.faded(0.5, Color::Reset), l, "no backdrop RGB -> no fade");
        assert_eq!(l.faded(0.0, Color::Rgb(0, 0, 0)).thumb, Color::Indexed(6));
        assert_eq!(l.faded(0.0, Color::Rgb(0, 0, 0)).track, Color::Rgb(0, 0, 0));
    }
}
