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
    /// For a margin float: the pixel x where text should start beside the image
    /// (the v6 game's own `set_margins` value when it followed the draw). `None`
    /// = derive from the image width. Ignored for inline (non-margin) aligns.
    pub margin_px: Option<u32>,
}

impl InlineImage {
    /// The `(cols, rows)` this image occupies at the given band `width` and
    /// terminal cell pixel size, aspect-preserved and capped to `width`.
    /// Both dimensions floor at 1.
    pub fn fitted_cells(&self, width: u16, char_px: (u16, u16)) -> (u16, u16) {
        let (cell_w, cell_h) = (char_px.0.max(1) as u32, char_px.1.max(1) as u32);
        let (pw, ph) = self.scaled.unwrap_or_else(|| {
            let d = &self.pixels;
            (d.width().max(1), d.height().max(1))
        });
        let (pw, ph) = (pw.max(1), ph.max(1));
        let max_px_w = width.max(1) as u32 * cell_w;
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
        InlineImage { pixels: Arc::new(image::RgbaImage::new(w, h)), align: ImageAlign::InlineUp, scaled: None , margin_px: None }
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
