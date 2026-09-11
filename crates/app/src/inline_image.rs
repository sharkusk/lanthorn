//! The value type for an image that flows inline with text-buffer output
//! (Glk `glk_image_draw` into a text-buffer window), plus its cell geometry.
//! Rendered as a full-width block; the raw `align` is retained for a future
//! margin-float renderer.

use std::sync::Arc;

/// Glk `imagealign_*` argument for a buffer-window `glk_image_draw`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ImageAlign {
    InlineUp,
    InlineDown,
    InlineCenter,
    MarginLeft,
    MarginRight,
}

impl ImageAlign {
    /// Decode a Glk `imagealign` constant. Unknown values default to `InlineUp`.
    pub fn from_glk(v: u32) -> ImageAlign {
        match v {
            1 => ImageAlign::InlineUp,
            2 => ImageAlign::InlineDown,
            3 => ImageAlign::InlineCenter,
            4 => ImageAlign::MarginLeft,
            5 => ImageAlign::MarginRight,
            _ => ImageAlign::InlineUp,
        }
    }
}

/// An image drawn into a text-buffer window, carrying its pixels (shared, like
/// `GraphicsWindow.canvas`), its alignment, and an optional scaled target size.
///
/// SQ-0461 also gave this an `ImageSource` provenance field, whose only job was
/// to mark a band as *already on screen as a window canvas* so that every render
/// mode but frameless would skip it. SQ-0895 removed frameless, which left every
/// mode skipping every such band — so they stopped being emitted, and the field
/// and its two-variant enum went with them. Everything anchored here now is
/// story content that every mode floats in the transcript.
#[derive(Clone, Debug)]
pub struct InlineImage {
    pub pixels: Arc<image::RgbaImage>,
    pub align: ImageAlign,
    pub scaled: Option<(u32, u32)>,
    /// A **standing** Glk 0.7.6 `imagerule` (SQ-1424), set only by
    /// `glk_image_draw_scaled_ext` into a text-buffer window. When present it
    /// SUPERSEDES `scaled`, because the two say different things: `scaled` is a
    /// size already settled, while a rule is a size that is not settled until
    /// the band width is known — and is re-settled every time that width moves.
    ///
    /// This is the field that makes `imagerule_WidthRatio` behave as the spec
    /// requires: "In a text buffer window, imagerule_WidthRatio is dynamically
    /// computed; the image width will always be relative to the *current*
    /// window width. If the text buffer window is resized (by the user or a
    /// window arrangement call), the image will resize too." Storing the
    /// resolved pixels instead would freeze a picture at the width it happened
    /// to be drawn at, and no later relayout could recover the ratio — the
    /// archive keeps the rule for the same reason ("persist the recipe, not the
    /// result").
    ///
    /// `None` for `glk_image_draw` / `glk_image_draw_scaled` and for every
    /// Z-machine v6 picture, none of which carry a rule.
    pub rule: Option<gvm::glk::ImageRule>,
    /// For a margin float: the pixel x where text should start beside the image
    /// (the v6 game's own `set_margins` value when it followed the draw). `None`
    /// = derive from the image width. Ignored for inline (non-margin) aligns.
    pub margin_px: Option<u32>,
    /// The Glk hyperlink value this picture carries (`glk_set_hyperlink` before
    /// `glk_image_draw`/`_scaled`/`_scaled_ext`; SQ-1503), 0 = no link. A Glulx
    /// game makes an inline picture clickable exactly the way it makes text
    /// clickable — Anchorhead: the Illustrated Edition's "click this thumbnail
    /// to view the full-size illustration" uses this. 0 for every Z-machine v6
    /// picture, which has no such concept.
    pub link: u32,
}

impl InlineImage {
    /// The `(cols, rows)` this image occupies at the given band `width` and
    /// terminal cell pixel size, aspect-preserved and capped to `width`.
    /// Both dimensions floor at 1.
    ///
    /// **This is the relayout hook for a standing `imagerule`** (SQ-1424). It
    /// is called from the transcript wrapper with the band's CURRENT width
    /// every time the transcript is laid out — so a terminal resize, a pane
    /// change, a font-size change, all re-enter here with a new `width` and
    /// re-resolve the rule against it. There is deliberately no separate
    /// "on resize, re-scale the images" path to forget to call: the one place
    /// that already knows the live width does the resolving.
    pub fn fitted_cells(&self, width: u16, char_px: (u16, u16)) -> (u16, u16) {
        let (cell_w, cell_h) = (char_px.0.max(1) as u32, char_px.1.max(1) as u32);
        let max_px_w = width.max(1) as u32 * cell_w;
        let natural = {
            let d = &self.pixels;
            (d.width().max(1), d.height().max(1))
        };
        let (pw, ph) = match self.rule {
            // A standing rule is resolved against the band width AS IT IS NOW.
            // `resolve_in_buffer` applies `maxwidth` too, which is the buffer
            // window's own bound; the further cap to `max_px_w` below is the
            // terminal's, and both are meant to hold.
            Some(r) => r.resolve_in_buffer(natural, max_px_w).unwrap_or(natural),
            None => self.scaled.unwrap_or(natural),
        };
        let (pw, ph) = (pw.max(1), ph.max(1));
        let (dw, dh) = if pw <= max_px_w {
            (pw, ph)
        } else {
            // Scale down to fit width, preserving aspect ratio.
            let dh = ((ph as u64 * max_px_w as u64) / pw as u64) as u32;
            (max_px_w, dh.max(1))
        };
        let cols = dw.div_ceil(cell_w).clamp(1, width.max(1) as u32) as u16;
        let rows = dh.div_ceil(cell_h).max(1) as u16;
        (cols, rows)
    }
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn img(w: u32, h: u32) -> InlineImage {
        InlineImage { pixels: Arc::new(image::RgbaImage::new(w, h)), align: ImageAlign::InlineUp, scaled: None , margin_px: None, rule: None, link: 0 }
    }

    #[test]
    fn align_decodes_all_glk_constants() {
        assert_eq!(ImageAlign::from_glk(1), ImageAlign::InlineUp);
        assert_eq!(ImageAlign::from_glk(2), ImageAlign::InlineDown);
        assert_eq!(ImageAlign::from_glk(3), ImageAlign::InlineCenter);
        assert_eq!(ImageAlign::from_glk(4), ImageAlign::MarginLeft);
        assert_eq!(ImageAlign::from_glk(5), ImageAlign::MarginRight);
        assert_eq!(ImageAlign::from_glk(999), ImageAlign::InlineUp); // unknown → default
    }

    #[test]
    fn fitted_cells_native_when_it_fits() {
        // 16x16 px, cell 8x8 → 2x2 cells; width 40 leaves it native.
        let (cols, rows) = img(16, 16).fitted_cells(40, (8, 8));
        assert_eq!((cols, rows), (2, 2));
    }

    #[test]
    fn fitted_cells_scales_down_to_width_preserving_aspect() {
        // 800x400 px, cell 8x8 → native 100x50 cells; width 40 → scale to 40 cols,
        // height scales by 40/100 → 20 cells.
        let (cols, rows) = img(800, 400).fitted_cells(40, (8, 8));
        assert_eq!(cols, 40);
        assert_eq!(rows, 20);
    }

    #[test]
    fn fitted_cells_uses_scaled_dims_when_present() {
        let mut i = img(16, 16);
        i.scaled = Some((80, 40)); // 80x40 px scaled request overrides native 16x16
        // 80x40 px, cell 8x8 → 10x5 cells; width 40 fits.
        assert_eq!(i.fitted_cells(40, (8, 8)), (10, 5));
    }

    /// SQ-1424 — a standing `imagerule` SUPERSEDES `scaled`, and is resolved
    /// against the band width handed in on THIS call, not the one the picture
    /// was drawn at. That is the whole of the Glk 0.7.6 text-buffer behaviour:
    /// the transcript wrapper calls this on every layout, so a resize is
    /// nothing more than the next call arriving with a different `width`.
    #[test]
    fn a_standing_rule_re_resolves_against_the_current_width() {
        let mut i = img(200, 100); // 2:1
        i.scaled = Some((999, 999)); // must be ignored while a rule is present
        i.rule = Some(gvm::glk::ImageRule {
            rule: gvm::glk::imagerule::WIDTH_RATIO | gvm::glk::imagerule::ASPECT_RATIO,
            width: 0x8000, // 50% of the window
            height: 0x1_0000, // original aspect
            maxwidth: 0,
        });
        // 1px cells → cell counts are pixel counts.
        assert_eq!(i.fitted_cells(40, (1, 1)), (20, 10), "half of a 40px band");
        assert_eq!(i.fitted_cells(80, (1, 1)), (40, 20), "…and half of an 80px one");
        // Half of 10 is 5 wide; the 2:1 aspect asks for 2.5 tall, which the
        // resolver rounds half-UP to 3 (matching garglk's `std::round`) rather
        // than truncating to 2.
        assert_eq!(i.fitted_cells(10, (1, 1)), (5, 3), "half of a 10px band, height rounded not truncated");
    }

    /// The precedence is one-way: with no rule, `scaled` still decides, which
    /// is what keeps `glk_image_draw_scaled` behaving exactly as before 0.7.6.
    #[test]
    fn without_a_rule_the_frozen_size_still_wins() {
        let mut i = img(200, 100);
        i.scaled = Some((60, 30));
        assert_eq!(i.rule, None);
        assert_eq!(i.fitted_cells(100, (1, 1)), (60, 30));
        assert_eq!(i.fitted_cells(200, (1, 1)), (60, 30), "a wider band does not stretch it");
    }

    #[test]
    fn fitted_cells_floor_is_one() {
        // Tiny image never disappears to 0 cells.
        assert_eq!(img(1, 1).fitted_cells(40, (8, 8)), (1, 1));
    }

    // The `frameless scaling policy` section (SQ-0461 decision 2) was deleted
    // here by SQ-0895 along with `InlineImage::frameless_scaled` and its
    // `dropcap_scale` / `band_upscale` helpers. All six tests pinned the sizing
    // policy of a mode that no longer exists — drop-caps to ~3.5 text rows, band
    // art to an integer 2x/3x under a 60%-of-viewport cap — and the function was
    // called from exactly one place, the frameless arm of the transcript's image
    // filter. Hybrid and raster never called it: they size from their own
    // letterbox factor, which `fitted_cells` (still tested above) applies.

    #[test]
    fn fitted_cells_pins_known_geometry() {
        // 100x60 px image, char_px 8x16, band width 40.
        // max_px_w = 40 * 8 = 320; native width 100 <= 320, so no downscale:
        // dw=100, dh=60. cols = ceil(100/8) = 13 (clamped to width 40 → 13).
        // rows = ceil(60/16) = 4.
        let (cols, rows) = img(100, 60).fitted_cells(40, (8, 16));
        assert_eq!((cols, rows), (13, 4));
    }
}
