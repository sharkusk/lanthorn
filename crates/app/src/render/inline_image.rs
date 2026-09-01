//! Blits inline-image bands (one terminal-row strip per call) via ratatui-image,
//! mirroring `render/graphics.rs`. Each band row renders the corresponding
//! horizontal strip of the fitted image, so partial-scroll degrades cleanly.
//!
//! The built per-row protocol is cached, keyed by
//! `(Arc::as_ptr(&band.image.pixels) as usize, band.cols, band.rows, band.row)`,
//! so a stable band (unchanged image/geometry/row) reuses the resized strip
//! across frames instead of rebuilding it every time. The full image is also
//! fit-resized only ONCE per band (keyed by `(ptr, cols, rows)`) and shared by
//! every row's crop, so the first scroll that reveals a tall image doesn't
//! resample the whole picture once per band row (SQ-0513).

use ratatui::buffer::Buffer;
use ratatui::layout::{Rect, Size};
use ratatui::style::{Color, Style};
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use ratatui_image::Resize;

use crate::render::transcript::ImageBand;
use crate::state::AppState;

/// If `wr` is an inline-image band row, blit it and return true (caller does
/// `continue`). A band row is consumed (no text drawn over it) even when no
/// game picker is present — matches the prior duplicated behavior at both call
/// sites (transcript draw loop and non-primary buffer windows).
pub(crate) fn try_blit_band_row(
    state: &AppState,
    wr: &super::transcript::WrappedRow,
    area_x: u16,
    area_width: u16,
    row_y: u16,
    buf: &mut Buffer,
) -> bool {
    if let Some(band) = &wr.band {
        if let Some(picker) = state.game_picker.as_ref() {
            let suppress = sixel_scroll_suppress(state, picker);
            blit_band(&state.inline_image_render, picker, band, area_x, area_width, row_y, float_page(state), suppress, buf);
        }
        return true;
    }
    false
}

/// True while a sixel image band should render as a background-filled footprint
/// instead of its full payload (SQ-1198): the transcript viewport is still in
/// motion from a recent scroll, and the active backend is sixel — the one
/// protocol with no image ids, so scrolling a placed image past its anchor cell
/// re-emits the whole payload rather than re-placing an existing upload the way
/// kitty does. Kitty and half-blocks are untouched: neither pays that cost.
fn sixel_scroll_suppress(state: &AppState, picker: &Picker) -> bool {
    picker.protocol_type() == ratatui_image::picker::ProtocolType::Sixel && state.transcript_scroll_in_motion()
}

/// Blit a left-margin float's picture strip (`x_off == 0`) for one row. Unlike
/// [`try_blit_band_row`] this does NOT consume the row — the caller has already
/// drawn the row's text to the right of the picture, so this only lays the image
/// down over the left `cols` columns. No-op without a game picker. (SQ-0454)
pub(crate) fn blit_float_row(
    state: &AppState,
    band: &ImageBand,
    area_x: u16,
    area_width: u16,
    row_y: u16,
    buf: &mut Buffer,
) {
    if let Some(picker) = state.game_picker.as_ref() {
        let suppress = sixel_scroll_suppress(state, picker);
        blit_band(&state.inline_image_render, picker, band, area_x, area_width, row_y, float_page(state), suppress, buf);
    }
}

/// Compute the clamped 1-row `dest` for an image band within a body area and
/// blit its strip. Shared by the transcript draw loop (Task 8) and non-primary
/// buffer windows (Task 9): both offset by `band.x_off`, clamp the width to the
/// drawable body, and render the strip via `InlineImageRender::render_row`, so a
/// game-supplied band can never exceed the area.
pub(crate) fn blit_band(
    render: &std::cell::RefCell<InlineImageRender>,
    picker: &Picker,
    band: &ImageBand,
    area_x: u16,
    area_width: u16,
    row_y: u16,
    page: Option<image::Rgba<u8>>,
    suppress: bool,
    buf: &mut Buffer,
) {
    let dest = Rect::new(
        area_x + band.x_off.min(area_width),
        row_y,
        band.cols.min(area_width.saturating_sub(band.x_off)),
        1,
    );
    render.borrow_mut().render_row(picker, band, dest, page, suppress, buf);
}

/// Cache key for one band row's built protocol: the image's pixel-buffer
/// identity, the band geometry that determines the resized strip, the page
/// the strip was flattened onto (see [`flatten_onto`]) so a theme or game-colour
/// change re-encodes instead of serving a strip baked over the old page, and the
/// CELL SIZE that geometry is measured in.
///
/// The cell size is in the key because `cols`/`rows` are a count and the resample
/// is in pixels: `box_w = cols · cell_w`. A terminal font-size change moves the
/// cell without necessarily moving the count — `InlineImage::fitted_cells` rounds
/// native pixels UP to whole cells, so adjacent font sizes routinely land on the
/// same one — and without it here, every key hit and the strips served were the
/// ones resampled for the old cell. Zork Zero's drop-cap and room icons came back
/// as misaligned bands at some font sizes and not others, and only until the game
/// was restarted, which rebuilt the pictures behind fresh `Arc`s and missed every
/// pointer key (SQ-1003). SQ-0988 fixed the same defect for the OTHER cache —
/// `draw_chrome_band` folds `(cw, ch)` into its freshness hash, and a resize
/// clears `GraphicsRender` outright — but `InlineImageRender` is a sibling field
/// on `AppState` and got neither. It is a key rather than a second invalidation
/// call so there is nothing for a future resize path to forget.
type BandCacheKey = (usize, u16, u16, u16, u32, u16, u16);

/// Cache key for a band's shared fit: [`BandCacheKey`] without the row or the
/// page, since the fit is the whole picture and predates both. It carries the
/// cell size for the same reason (SQ-1003).
type FittedKey = (usize, u16, u16, u16, u16);

/// Resize `src` to sit inside a `box_w × box_h` pixel box WITHOUT distorting it,
/// centred, with the leftover margin left transparent (SQ-0704).
///
/// The band's cell footprint is computed by rounding the image UP to whole cells
/// on each axis independently (`InlineImage::fitted_cells`), and the two axes
/// round by different amounts — so the box is very rarely the image's own shape.
/// Resizing straight onto it (`resize_exact`) stretched the picture to match: a
/// 40×40 icon in an 8×16 cell becomes 5 cols × 3 rows = 40×48 px, i.e. 20% too
/// tall. Zork Zero's square room icons came out visibly tall that way.
///
/// The margin stays transparent here and is resolved by [`flatten_onto`] against
/// the story's page, so the padding matches the paper the icon sits on.
///
/// The resample is [`crate::render::graphics::resize_directional`] (SQ-0829). This
/// site runs in BOTH directions — a picture wider than the transcript body is
/// shrunk to it, while one that already fits is nudged UP to its ceil-to-cells box,
/// and the removed frameless mode deliberately asked for a whole 2×/3×
/// enlargement ("an integer 2×/3× for pixel-art crispness", SQ-0461/SQ-0895).
/// Triangle at every size served neither: it blurred away the very crispness the
/// integer factor was chosen for, and, filtering the four channels independently,
/// averaged the `(0, 0, 0)` behind a transparent pixel into its neighbours — which
/// is the whole population here, since Zork Zero's drop caps and room icons are
/// cut-out PNGs.
pub fn fit_preserving_aspect(
    src: &image::RgbaImage,
    box_w: u32,
    box_h: u32,
) -> image::RgbaImage {
    let (sw, sh) = (src.width().max(1), src.height().max(1));
    // The largest whole-pixel size with the source's aspect that still fits.
    let dw = (box_w).min((sw as u64 * box_h as u64 / sh as u64).max(1) as u32).max(1);
    let dh = (box_h).min((sh as u64 * dw as u64 / sw as u64).max(1) as u32).max(1);
    let resized = crate::render::graphics::resize_directional(src, dw, dh);
    if dw == box_w && dh == box_h {
        return resized;
    }
    let mut out = image::RgbaImage::new(box_w, box_h);
    image::imageops::replace(&mut out, &resized, ((box_w - dw) / 2) as i64, ((box_h - dh) / 2) as i64);
    out
}

/// Composite every pixel of `strip` over an opaque `page`, in place (SQ-0704).
///
/// An inline story picture is handed to the image protocol WITH ITS ALPHA. Kitty
/// keeps that alpha and composites the image against the terminal's own
/// background — not against the cell colours underneath, which the protocol never
/// consults. So Zork Zero's room icons (transparent PNGs drawn as transcript
/// floats, exactly like its drop-caps) came out sitting on the terminal
/// background instead of the white page the story window declared, no matter what
/// colour the cells behind them carried.
///
/// Flattening here is the same move `flatten_onto_page` makes for the raster
/// composite, applied to the one surface that still shipped alpha: whoever
/// composites must not be left to pick the colour for us. It also resolves the
/// margin [`fit_preserving_aspect`] leaves, so the padding around a picture whose
/// shape differs from its cell box is the story's page, not a hole.
pub(crate) fn flatten_onto(strip: &mut image::RgbaImage, page: image::Rgba<u8>) {
    for px in strip.pixels_mut() {
        let a = px[3] as u32;
        if a == 255 {
            continue;
        }
        // Straight `over`: src·α + dst·(1−α), with the destination fully opaque.
        for c in 0..3 {
            px[c] = ((px[c] as u32 * a + page[c] as u32 * (255 - a)) / 255) as u8;
        }
        px[3] = 255;
    }
}

/// The ground THIS frame's inline story floats are flattened onto: the three
/// layers of [`page_for`], resolved from the state the render published.
///
/// Public so a test can assert on the page the render actually resolved instead
/// of re-deriving it, exactly as `render::screen::v6_host_pair` is.
pub fn float_page(state: &AppState) -> Option<image::Rgba<u8>> {
    page_for(
        state.v6_story_page.get(),
        crate::render::screen::v6_machine_page_rgba(state),
        state.colors.theme.get("inline_image").style,
    )
}

/// The opaque page an inline image should be flattened onto — three layers, most
/// specific first.
///
/// 1. The STORY window's own page (`AppState::v6_story_page`, published by the
///    render's Layered arm from the window the game declared with `set_colour`).
///    A colour the game named for the very window the picture floats in wins
///    outright.
/// 2. The MACHINE's page (`AppState::v6_page_pair`, via
///    [`crate::render::screen::v6_machine_page_rgba`]) — SQ-0848. Reported by eye
///    on `stories/Zork Zero Disk.image`, **release 296 / serial 881019**, the
///    Macintosh disk: *"the room icon background is terminal default, rather than
///    the white story pane background"*. Zork Zero on the Macintosh **never calls
///    `set_colour` at all** (measured — see `session::machine_screen_pair`), so
///    layer 1 is `None` on every frame of it and the drop-caps and room icons fell
///    straight through to the theme's `chrome` black while the pane around them
///    was the machine's white. The machine's page is not a theme preference: it is
///    the paper the prose beside the picture is being read on
///    (`screen::v6_machine_page` lays the same pair under the transcript's cells),
///    so the picture's ground and the prose's ground are one thing.
/// 3. The theme's `inline_image` style, for a frame that declares neither.
///    Flattening onto it any earlier would paint the icons with the very
///    terminal-following colour the game overrode, since `transcript` and
///    `upper_window` follow the terminal's own background as of SQ-0510.
///
/// Deliberately NOT read back out of the destination cell. The band's cells hold
/// whatever the previous frame drew there — including the picture itself — so
/// sampling them feeds the image its own colours and mints a fresh cache entry
/// every frame.
///
/// `None` when none of the three names a background we can resolve (`Reset` is
/// the terminal's own colour and `Indexed` has no canonical RGB here): there is
/// then no page we could claim is right, so the alpha ships as before rather than
/// being flattened onto a guess.
pub(crate) fn page_for(
    story_page: Option<(u8, u8, u8)>,
    machine_page: Option<image::Rgba<u8>>,
    letterbox: Style,
) -> Option<image::Rgba<u8>> {
    if let Some((r, g, b)) = story_page {
        return Some(image::Rgba([r, g, b, 255]));
    }
    if let Some(p) = machine_page {
        return Some(p);
    }
    match letterbox.bg {
        None | Some(Color::Reset) | Some(Color::Indexed(_)) => None,
        Some(c) => Some(crate::render::v6_layout::color_to_rgba(c, image::Rgba([0, 0, 0, 255]))),
    }
}

#[derive(Default)]
pub struct InlineImageRender {
    /// Value pins the source `Arc` alongside the built protocol: holding the
    /// `Arc` keeps its pixel-buffer address reserved while cached, so the
    /// pointer-based key can never collide with a later image that reuses a
    /// freed address (the ABA bug). Same shared allocation the live image holds.
    ///
    /// The third element is the kitty image id `place_protocol` returned the
    /// last time this entry was placed (`None` off-kitty, or before the first
    /// placement) — one id per cache entry, since each key already names a
    /// distinct row of a distinct band at a distinct page/cell size, so no two
    /// entries are ever the same upload (SQ-1190). `retain_live` reads it back
    /// out to free the upload a dropped entry still owns in the terminal,
    /// mirroring `GraphicsRender::chrome_bands` (SQ-0753).
    cache: std::collections::HashMap<BandCacheKey, (std::sync::Arc<image::RgbaImage>, Protocol, Option<u32>)>,
    /// The whole source image fitted to a band's cell box, cached per
    /// [`FittedKey`] and SHARED by every row of that band. The
    /// fit `resize_exact` is by far the expensive step (a full-image resample);
    /// doing it once per band instead of once per row is what keeps the first
    /// scroll that brings a tall image into view smooth — otherwise all N band
    /// rows resample the whole image in a single paint frame (SQ-0513). The
    /// paired `Arc` pins the SOURCE buffer so the pointer key can't ABA-collide,
    /// mirroring `cache`.
    fitted: std::collections::HashMap<FittedKey, (std::sync::Arc<image::RgbaImage>, image::DynamicImage)>,
}

impl std::fmt::Debug for InlineImageRender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InlineImageRender").field("cached", &self.cache.len()).finish()
    }
}

impl InlineImageRender {
    /// Blit the strip for `band.row` (of `band.rows`) into the 1-row `dest`.
    ///
    /// `suppress` (SQ-1198) skips the protocol build/placement entirely and
    /// leaves the destination as the letterbox fill below — the image's
    /// background-filled footprint — so a sixel image mid-scroll costs no
    /// payload at all instead of re-emitting its full data every step. Set only
    /// by [`sixel_scroll_suppress`]; kitty and half-blocks always pass `false`.
    pub(crate) fn render_row(&mut self, picker: &Picker, band: &ImageBand, dest: Rect, page: Option<image::Rgba<u8>>, suppress: bool, buf: &mut Buffer) {
        if dest.width == 0 || dest.height == 0 {
            return;
        }
        // Letterbox the destination first (padding when the image is narrower).
        let letterbox = match page {
            Some(p) => Style::default().bg(Color::Rgb(p[0], p[1], p[2])),
            None => Style::default(),
        };
        for y in dest.top()..dest.bottom() {
            for x in dest.left()..dest.right() {
                if let Some(c) = buf.cell_mut((x, y)) {
                    c.set_symbol(" ").set_style(letterbox);
                }
            }
        }
        if suppress {
            return;
        }
        let src_ptr = std::sync::Arc::as_ptr(&band.image.pixels) as usize;
        // The page joins the key: the same strip over a different page is a
        // different image, and serving the cached one would keep the old ground
        // after a theme switch or `/set-game-colours`.
        let page_key = page.map_or(0, |p| {
            u32::from_be_bytes([1, p[0], p[1], p[2]])
        });
        // Cell pixel size comes from the picker font, and joins both keys: it is
        // what the resample below is measured in, and it moves under a terminal
        // font-size change that leaves `cols`/`rows` alone (SQ-1003).
        let fs = picker.font_size();
        let (cw, ch) = (fs.width.max(1), fs.height.max(1));
        let key: BandCacheKey = (src_ptr, band.cols, band.rows, band.row, page_key, cw, ch);
        if !self.cache.contains_key(&key) {
            // Fit the whole image to the band's cell box in pixels, then crop
            // the strip for this row.
            let (fw, fh) = (cw as u32, ch as u32);
            let box_w = band.cols as u32 * fw;
            let box_h = band.rows as u32 * fh;
            if box_w == 0 || box_h == 0 {
                return;
            }
            let strip_y = band.row as u32 * fh;
            if strip_y >= box_h {
                return;
            }
            let strip_h = fh.min(box_h - strip_y);
            // Build (or reuse) the band's fitted full image ONCE, then crop this
            // row's strip from it. Sharing the resample across the band's rows is
            // the SQ-0513 fix — the crop is cheap; the resize is not.
            let strip = {
                let fit_key: FittedKey = (src_ptr, band.cols, band.rows, cw, ch);
                let (_pin, full) = self.fitted.entry(fit_key).or_insert_with(|| {
                    // Fit, do not stretch: the cell box is rounded up per axis and
                    // is almost never the picture's own shape (SQ-0704).
                    let fitted = image::DynamicImage::ImageRgba8(fit_preserving_aspect(
                        &band.image.pixels,
                        box_w,
                        box_h,
                    ));
                    (band.image.pixels.clone(), fitted)
                });
                full.crop_imm(0, strip_y, box_w, strip_h)
            };
            // Flatten the strip onto its page BEFORE the protocol is built, so the
            // encoder never sees an alpha channel it would hand to the terminal to
            // resolve (SQ-0704). Done on the crop rather than the cached `fitted`
            // full image, which is shared across pages.
            let strip = match page {
                Some(p) => {
                    let mut rgba = strip.to_rgba8();
                    flatten_onto(&mut rgba, p);
                    image::DynamicImage::ImageRgba8(rgba)
                }
                None => strip,
            };
            if let Ok(proto) = picker.new_protocol(strip, Size::new(band.cols, 1), Resize::Fit(None)) {
                self.cache.insert(key, (band.image.pixels.clone(), proto, None));
            }
        }
        // Placed every frame this row is drawn, not only on a cache miss — the id
        // is stable for a given `Protocol`, so re-recording it is idempotent, and
        // it is how a freshly-built entry (still `None` above) learns the id it
        // was just placed under (SQ-1190).
        let placed = self.cache.get(&key).map(|(_, proto, _)| crate::render::graphics::place_protocol(proto, dest, buf));
        if let Some(id) = placed {
            if let Some(entry) = self.cache.get_mut(&key) {
                entry.2 = id;
            }
        }
    }

    /// Drop cache entries for bands no longer live, keyed by source Arc-ptr
    /// (`live` holds the currently-visible bands' pointers). Bounds growth and,
    /// with the pinned Arc in the value, releases addresses only once truly gone.
    ///
    /// Returns the kitty image ids the evicted entries were placed under, so the
    /// caller can free them in the terminal (`GraphicsRender::queue_external_deletes`)
    /// rather than merely forgetting the struct that named them (SQ-1190,
    /// mirroring SQ-0753's `GraphicsRender::retain_live`/`retain_chrome_bands`).
    /// One id per evicted entry: each `BandCacheKey` names a distinct row of a
    /// distinct band at a distinct page/cell size, so eviction here can never
    /// free an id another surviving entry still places.
    pub fn retain_live(&mut self, live: &std::collections::HashSet<usize>) -> Vec<u32> {
        let dropped: Vec<u32> = self
            .cache
            .iter()
            .filter(|(key, _)| !live.contains(&key.0))
            .filter_map(|(_, (_, _, id))| *id)
            .collect();
        self.cache.retain(|key, _| live.contains(&key.0));
        self.fitted.retain(|key, _| live.contains(&key.0));
        dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui_image::picker::Picker;

    #[test]
    fn renders_band_row_without_panic() {
        let mut px = image::RgbaImage::new(16, 16);
        for p in px.pixels_mut() {
            *p = image::Rgba([200, 0, 0, 255]);
        }
        let img = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(px),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None,
        };
        let band = crate::render::transcript::ImageBand { image: img, cols: 2, rows: 2, row: 0, x_off: 0 };
        let picker = Picker::halfblocks();
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
        let mut r = InlineImageRender::default();
        r.render_row(&picker, &band, Rect::new(0, 0, 2, 1), None, false, &mut buf);
        // No panic == pass; the halfblock protocol writes into (0,0)..(2,1).
    }

    /// SQ-0704: an inline picture's transparent pixels must be resolved by US,
    /// against the page the row already shows, never handed to the terminal.
    ///
    /// Zork Zero's room icons are transcript floats like its drop-caps, and their
    /// PNGs carry alpha. The protocol keeps that alpha — kitty composites the
    /// image against the TERMINAL's background and never consults the cell colours
    /// underneath — so the icons sat on the terminal background instead of the
    /// white page their story window declared.
    ///
    /// Halfblocks is the honest oracle: its encoder calls `to_rgb8()`, so an
    /// unflattened transparent pixel becomes pure BLACK. Falsified by dropping the
    /// flatten — every asserted cell comes back `Rgb(0, 0, 0)`.
    #[test]
    fn a_transparent_inline_picture_is_flattened_onto_the_rows_own_page() {
        // Fully transparent: every pixel is the terminal's to resolve, pre-fix.
        let px = image::RgbaImage::new(16, 16);
        let img = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(px),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None,
        };
        let band = crate::render::transcript::ImageBand { image: img, cols: 2, rows: 2, row: 0, x_off: 0 };
        let picker = Picker::halfblocks();
        let white = Some(image::Rgba([255, 255, 255, 255]));

        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
        let mut r = InlineImageRender::default();
        r.render_row(&picker, &band, Rect::new(0, 0, 2, 1), white, false, &mut buf);
        for x in 0..2 {
            let cell = buf.cell((x, 0)).expect("the band wrote this cell");
            assert_eq!(
                cell.style().fg,
                Some(ratatui::style::Color::Rgb(255, 255, 255)),
                "cell {x} must carry the page the picture was flattened onto, not the terminal's"
            );
        }

        // A different page is a different image: the cache must not serve the
        // strip baked over the old one after a theme or game-colour change.
        let black = Some(image::Rgba([0, 0, 0, 255]));
        r.render_row(&picker, &band, Rect::new(0, 0, 2, 1), black, false, &mut buf);
        assert_eq!(r.cache.len(), 2, "the page is part of the cache key");
    }

    /// A style with no resolvable background leaves the alpha alone — there is no
    /// colour we could claim is right, so we do not invent one.
    #[test]
    fn an_unresolvable_page_leaves_the_picture_unflattened() {
        let none = ratatui::style::Style::default();
        assert_eq!(page_for(None, None, none), None, "no page published and no theme background");
        assert_eq!(page_for(None, None, none.bg(Color::Reset)), None, "Reset is the terminal's own colour");
        assert_eq!(page_for(None, None, none.bg(Color::Indexed(9))), None, "an indexed colour has no canonical RGB here");
        assert_eq!(
            page_for(None, None, none.bg(Color::White)),
            Some(image::Rgba([255, 255, 255, 255])),
            "a named ANSI theme colour does resolve as the fallback"
        );
        // The GAME's page wins over the theme: the theme's inline_image colour
        // follows the terminal since SQ-0510, which is exactly what must not win
        // when the story window declared a page of its own.
        let themed = none.bg(Color::Rgb(26, 26, 26));
        assert_eq!(
            page_for(Some((255, 255, 255)), None, themed),
            Some(image::Rgba([255, 255, 255, 255])),
            "the story window's declared page beats the theme"
        );
        assert_eq!(
            page_for(None, None, themed),
            Some(image::Rgba([26, 26, 26, 255])),
            "with no declared page the theme is the fallback"
        );
    }

    /// SQ-0848: the MACHINE's page is the middle layer — under a window that
    /// declared one, over the theme.
    ///
    /// A machine whose §8.3.3 defaults ARE its screen (the Macintosh's white page,
    /// the Amiga's grey) is not stating a preference the theme may outvote: it is
    /// the paper the prose beside the picture is read on. But it is still less
    /// specific than a colour the game named for that very window, so it must not
    /// displace layer 1.
    #[test]
    fn the_machines_page_sits_between_the_window_and_the_theme() {
        let themed = ratatui::style::Style::default().bg(Color::Rgb(26, 26, 26));
        let white = image::Rgba([255, 255, 255, 255]);
        // The Macintosh case as reported: no window colour anywhere, a machine
        // page of white, and a theme that would otherwise have supplied its own.
        assert_eq!(
            page_for(None, Some(white), themed),
            Some(white),
            "with no window page the machine's own beats the theme — SQ-0848",
        );
        // …and it is a layer, not an override: `zork0-r393-s890714.z6` boots
        // `set_colour(fg=2 black, bg=9 white)` on window 0, and that must still win.
        let grey = image::Rgba([66, 66, 66, 255]);
        assert_eq!(
            page_for(Some((255, 255, 255)), Some(grey), themed),
            Some(white),
            "an explicit window background still beats the machine's page",
        );
        // No machine pair (every profile but the Amiga and the Macintosh, and
        // either of those with the game's colours declined) is byte-identical to
        // the two-layer behaviour that shipped before.
        assert_eq!(page_for(None, None, themed), Some(image::Rgba([26, 26, 26, 255])));
    }

    /// SQ-0704: a square picture must stay square in a cell box that is not.
    ///
    /// `fitted_cells` rounds the image up to whole cells on each axis
    /// independently, so the box rarely matches the picture's shape: a 40x40 icon
    /// with an 8x16 cell lands in a 5x3 cell box = 40x48 px. Stretching to fill
    /// that made Zork Zero's room icons 20% too tall.
    ///
    /// Falsified by restoring `resize_exact`: the drawn height becomes 48, not 40.
    #[test]
    fn a_square_picture_keeps_its_shape_in_a_taller_cell_box() {
        let mut src = image::RgbaImage::new(40, 40);
        for p in src.pixels_mut() {
            *p = image::Rgba([10, 20, 30, 255]);
        }
        let out = fit_preserving_aspect(&src, 40, 48);
        assert_eq!((out.width(), out.height()), (40, 48), "the fitted image fills the whole box");

        // The opaque content inside it is still square, and centred.
        let opaque_rows: Vec<u32> = (0..out.height())
            .filter(|&y| (0..out.width()).any(|x| out.get_pixel(x, y)[3] == 255))
            .collect();
        let opaque_cols: Vec<u32> = (0..out.width())
            .filter(|&x| (0..out.height()).any(|y| out.get_pixel(x, y)[3] == 255))
            .collect();
        assert_eq!(opaque_rows.len(), 40, "the picture keeps its own height, not the box's");
        assert_eq!(opaque_cols.len(), 40, "and its own width");
        assert_eq!(opaque_rows[0], 4, "the leftover margin is split evenly above and below");

        // A box that already matches needs no padding at all.
        let exact = fit_preserving_aspect(&src, 40, 40);
        assert!(exact.pixels().all(|p| p[3] == 255), "an exact box is filled edge to edge");
    }

    #[test]
    fn flatten_onto_composites_partial_alpha_over_the_page() {
        let mut img = image::RgbaImage::new(3, 1);
        img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255])); // opaque — untouched
        img.put_pixel(1, 0, image::Rgba([255, 0, 0, 0])); // clear — becomes the page
        img.put_pixel(2, 0, image::Rgba([0, 0, 0, 128])); // half — blends
        flatten_onto(&mut img, image::Rgba([255, 255, 255, 255]));
        assert_eq!(img.get_pixel(0, 0), &image::Rgba([255, 0, 0, 255]), "opaque pixels are left alone");
        assert_eq!(img.get_pixel(1, 0), &image::Rgba([255, 255, 255, 255]), "clear pixels take the page");
        assert_eq!(img.get_pixel(2, 0), &image::Rgba([127, 127, 127, 255]), "half alpha blends toward the page");
        assert!(img.pixels().all(|p| p[3] == 255), "nothing is left for a compositor to resolve");
    }

    #[test]
    fn render_row_caches_built_protocol() {
        let px = image::RgbaImage::new(16, 16);
        let img = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(px),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None,
        };
        let band = crate::render::transcript::ImageBand { image: img, cols: 2, rows: 2, row: 0, x_off: 0 };
        let picker = Picker::halfblocks();
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
        let mut r = InlineImageRender::default();
        assert_eq!(r.cache.len(), 0);
        r.render_row(&picker, &band, Rect::new(0, 0, 2, 1), None, false, &mut buf);
        assert_eq!(r.cache.len(), 1);
        // A second render of the same band/row reuses the cached protocol
        // rather than inserting a new entry.
        r.render_row(&picker, &band, Rect::new(0, 0, 2, 1), None, false, &mut buf);
        assert_eq!(r.cache.len(), 1);
        // A different row of the same band gets its own cache entry.
        let band_row1 = crate::render::transcript::ImageBand { row: 1, ..band };
        r.render_row(&picker, &band_row1, Rect::new(0, 0, 2, 1), None, false, &mut buf);
        assert_eq!(r.cache.len(), 2);
    }

    #[test]
    fn a_font_size_change_re_resamples_instead_of_serving_the_old_cell() {
        // SQ-1003. Both caches are keyed in CELLS, and the cell's pixel size is
        // what decides the resample — so a font-size change that leaves `cols`
        // and `rows` alone must still miss. It did not, and Zork Zero's drop-cap
        // and room icons came back as misaligned bands the moment the terminal
        // font moved: pixels fitted to the old cell, placed into the new one.
        // Restarting the game cleared it, because the pictures came back behind
        // fresh `Arc`s and every pointer key missed — which is what says cache
        // rather than geometry.
        let px = image::RgbaImage::new(32, 32);
        let img = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(px),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None,
        };
        let (cols, rows) = (4u16, 2u16);
        let band = crate::render::transcript::ImageBand { image: img, cols, rows, row: 0, x_off: 0 };
        let mut picker = Picker::halfblocks();
        picker.set_font_size(ratatui_image::FontSize::new(8, 16));
        let mut buf = Buffer::empty(Rect::new(0, 0, cols + 2, rows + 2));
        let mut r = InlineImageRender::default();
        r.render_row(&picker, &band, Rect::new(0, 0, cols, 1), None, false, &mut buf);
        let small = r.fitted.values().next().expect("the band was fitted").1.clone();
        assert_eq!((small.width(), small.height()), (32, 32), "fitted to the 8x16 cell");

        // The same band, the same cell COUNT, a bigger cell.
        picker.set_font_size(ratatui_image::FontSize::new(16, 32));
        r.render_row(&picker, &band, Rect::new(0, 0, cols, 1), None, false, &mut buf);
        assert_eq!(r.fitted.len(), 2, "the new cell size is a new fit, not a hit on the old one");
        let big = r
            .fitted
            .values()
            .map(|(_, f)| (f.width(), f.height()))
            .max()
            .expect("two fits");
        assert_eq!(big, (64, 64), "fitted to the 16x32 cell — the picture is resampled, not rescaled by the terminal");
        assert_eq!(r.cache.len(), 2, "and the row's built protocol is rebuilt with it");
    }

    #[test]
    fn fit_resize_is_shared_across_a_bands_rows() {
        // SQ-0513: the first scroll that reveals a tall image paints every band
        // row in one frame. The expensive full-image fit-resize must happen ONCE
        // per band, not once per row — so rendering all N rows leaves exactly one
        // `fitted` entry while producing N per-row protocol entries. (A per-row
        // resize is what froze the first scroll for ~N× the single-resize cost.)
        let px = image::RgbaImage::new(64, 64);
        let img = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(px),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None,
        };
        let picker = Picker::halfblocks();
        let (cols, rows) = (6u16, 8u16);
        let mut buf = Buffer::empty(Rect::new(0, 0, cols + 2, rows + 2));
        let mut r = InlineImageRender::default();
        for row in 0..rows {
            let band = crate::render::transcript::ImageBand { image: img.clone(), cols, rows, row, x_off: 0 };
            r.render_row(&picker, &band, Rect::new(0, row, cols, 1), None, false, &mut buf);
        }
        assert_eq!(r.fitted.len(), 1, "the whole image is fit-resized once per band, shared by every row");
        assert_eq!(r.cache.len(), rows as usize, "each row still gets its own cheap cropped protocol");
        // Eviction of the band releases BOTH the protocols and the shared fit.
        r.retain_live(&std::collections::HashSet::new());
        assert_eq!(r.fitted.len(), 0, "retain_live evicts the fitted-image cache too");
        assert_eq!(r.cache.len(), 0);
    }

    fn band_for(pixels: std::sync::Arc<image::RgbaImage>) -> crate::render::transcript::ImageBand {
        let img = crate::inline_image::InlineImage {
            pixels,
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None,
        };
        crate::render::transcript::ImageBand { image: img, cols: 2, rows: 2, row: 0, x_off: 0 }
    }

    #[test]
    fn cache_pins_source_arc_blocking_aba() {
        // Building a protocol pins the source Arc in the cache value, so the
        // image's pixel-buffer address cannot be freed and reused while cached.
        // A NEW image therefore always gets a distinct pointer key — the stale
        // protocol can never be served for the wrong picture (the ABA bug).
        let picker = Picker::halfblocks();
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
        let mut r = InlineImageRender::default();

        let arc_a = std::sync::Arc::new(image::RgbaImage::new(16, 16));
        let ptr_a = std::sync::Arc::as_ptr(&arc_a) as usize;
        let band_a = band_for(arc_a.clone());
        r.render_row(&picker, &band_a, Rect::new(0, 0, 2, 1), None, false, &mut buf);
        assert_eq!(r.cache.len(), 1);
        // Drop every strong reference to A that this test holds; only the cache
        // still pins it. Its address stays reserved and un-reusable.
        drop(band_a);
        drop(arc_a);

        let arc_b = std::sync::Arc::new(image::RgbaImage::new(16, 16));
        let ptr_b = std::sync::Arc::as_ptr(&arc_b) as usize;
        // The pin guarantees B cannot land on A's still-reserved address.
        assert_ne!(ptr_b, ptr_a, "cached Arc must keep A's address reserved");
        let band_b = band_for(arc_b);
        r.render_row(&picker, &band_b, Rect::new(0, 0, 2, 1), None, false, &mut buf);
        // B is a fresh, distinct entry — it never reuses A's cached protocol.
        assert_eq!(r.cache.len(), 2);
    }

    #[test]
    fn retain_live_evicts_absent_bands_keeps_present() {
        let picker = Picker::halfblocks();
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
        let mut r = InlineImageRender::default();

        let arc1 = std::sync::Arc::new(image::RgbaImage::new(16, 16));
        let arc2 = std::sync::Arc::new(image::RgbaImage::new(16, 16));
        let ptr1 = std::sync::Arc::as_ptr(&arc1) as usize;
        let ptr2 = std::sync::Arc::as_ptr(&arc2) as usize;
        r.render_row(&picker, &band_for(arc1.clone()), Rect::new(0, 0, 2, 1), None, false, &mut buf);
        r.render_row(&picker, &band_for(arc2.clone()), Rect::new(0, 0, 2, 1), None, false, &mut buf);
        assert_eq!(r.cache.len(), 2);

        // Only band 1 is still live: band 2's entry is evicted, band 1's kept.
        r.retain_live(&std::collections::HashSet::from([ptr1]));
        assert_eq!(r.cache.len(), 1);
        assert!(r.cache.keys().any(|k| k.0 == ptr1));
        assert!(!r.cache.keys().any(|k| k.0 == ptr2));
    }

    /// SQ-1190: an evicted band's kitty upload must be freed in the terminal,
    /// not merely forgotten here. `place_protocol` is the only place that ever
    /// learns the id `render_row` placed a band's `Protocol` under — this
    /// asserts `retain_live` reads it back out of the entry it drops and hands
    /// it to the caller, rather than discarding it along with the struct.
    ///
    /// Falsified by reverting the `entry.2 = id` write-back in `render_row`:
    /// every entry then stays `None` and this returns an empty `Vec` instead
    /// of the id, exactly the leak this quest fixes.
    #[test]
    fn retain_live_returns_the_evicted_kitty_ids_to_delete() {
        let mut px = image::RgbaImage::new(16, 16);
        for p in px.pixels_mut() {
            *p = image::Rgba([200, 0, 0, 255]);
        }
        let img = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(px),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None,
        };
        let band = crate::render::transcript::ImageBand { image: img, cols: 2, rows: 2, row: 0, x_off: 0 };
        // A real kitty picker (not `Picker::halfblocks()`), so `place_protocol`
        // actually has an id to hand back.
        let picker = crate::render::graphics::kitty_picker(8, 16);
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
        let mut r = InlineImageRender::default();
        r.render_row(&picker, &band, Rect::new(0, 0, 2, 1), None, false, &mut buf);
        let id = r
            .cache
            .values()
            .next()
            .and_then(|(_, _, id)| *id)
            .expect("a kitty placement must have named an id");

        let evicted = r.retain_live(&std::collections::HashSet::new());
        assert_eq!(evicted, vec![id], "the evicted entry's id must be handed back for deletion");

        // A second eviction of the same (now-empty) cache frees nothing new.
        assert!(r.retain_live(&std::collections::HashSet::new()).is_empty());
    }

    // ── SQ-1198: sixel scroll-settle debounce ─────────────────────────────────
    //
    // `pty_stream` (crates/app/tests/pty_stream) can only pose convincingly as
    // kitty — its responder's DA1 deliberately omits sixel's `4` "so a fallback
    // path never looks like a success" — so it cannot honestly capture a sixel
    // byte stream. These two layers are the honest ones instead: the suppression
    // DECISION (`sixel_scroll_suppress`, pure function of protocol type + motion)
    // and the render OUTCOME (`render_row`'s `suppress` arm, asserted on the
    // buffer cells it writes — the same oracle `render_row_caches_built_protocol`
    // above already uses for the un-suppressed path).

    /// `sixel_scroll_suppress` gates on BOTH the backend and the motion window:
    /// only sixel, and only while `transcript_scroll_in_motion()`. Kitty
    /// re-places an existing upload by id for free and half-blocks are ordinary
    /// cells, so neither pays the cost this debounce exists to avoid, and the
    /// design commits to leaving both untouched.
    #[test]
    fn sixel_scroll_suppress_gates_on_protocol_and_motion() {
        let mut state = AppState::default();
        let mut sixel = crate::render::graphics::kitty_picker(8, 16);
        sixel.set_protocol_type(ratatui_image::picker::ProtocolType::Sixel);
        let kitty = crate::render::graphics::kitty_picker(8, 16);
        let halfblocks = Picker::halfblocks();

        // A still screen (no scroll in flight): never suppressed, whatever the
        // backend — first render of an image is unchanged.
        assert!(!sixel_scroll_suppress(&state, &sixel), "no motion yet");
        assert!(!sixel_scroll_suppress(&state, &kitty));
        assert!(!sixel_scroll_suppress(&state, &halfblocks));

        // In motion: sixel alone is suppressed.
        state.sixel_scroll_motion_at = Some(std::time::Instant::now());
        assert!(sixel_scroll_suppress(&state, &sixel), "sixel mid-scroll must suppress");
        assert!(!sixel_scroll_suppress(&state, &kitty), "kitty is untouched by the debounce");
        assert!(!sixel_scroll_suppress(&state, &halfblocks), "half-blocks is untouched by the debounce");
    }

    /// Falsification target for case (1): while suppressed, a sixel band row
    /// renders as its background-filled footprint — no protocol is built, no
    /// entry is cached, and the anchor cell carries no payload — where an
    /// un-suppressed render of the exact same row places a real sixel protocol,
    /// whose fork-patched encoder (`ratatui-image`'s `src/protocol/sixel.rs`)
    /// writes the WHOLE sixel data string into the anchor cell's symbol
    /// (SQ-1198). Falsified by deleting the `if suppress { return; }` early
    /// return in `render_row`: the cache then gains an entry and the anchor
    /// carries the payload on the suppressed call too — confirmed by hand before
    /// trusting this test.
    #[test]
    fn suppressed_render_leaves_only_the_footprint_no_payload() {
        let mut px = image::RgbaImage::new(16, 16);
        for p in px.pixels_mut() {
            *p = image::Rgba([200, 0, 0, 255]);
        }
        let img = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(px),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None,
        };
        let band = crate::render::transcript::ImageBand { image: img, cols: 2, rows: 2, row: 0, x_off: 0 };
        let mut picker = crate::render::graphics::kitty_picker(8, 16);
        picker.set_protocol_type(ratatui_image::picker::ProtocolType::Sixel);
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
        let mut r = InlineImageRender::default();

        // Case (1): mid-scroll, no sixel payload is emitted at all.
        r.render_row(&picker, &band, Rect::new(0, 0, 2, 1), None, true, &mut buf);
        assert_eq!(r.cache.len(), 0, "a suppressed render must not build (or cache) a sixel protocol");
        let anchor = buf.cell((0, 0)).expect("the band wrote this cell");
        assert_eq!(anchor.symbol(), " ", "no sixel payload rides the anchor cell during motion");

        // Case (2): settled (suppress = false), exactly one full emit lands.
        r.render_row(&picker, &band, Rect::new(0, 0, 2, 1), None, false, &mut buf);
        assert_eq!(r.cache.len(), 1, "the settled render builds and caches the protocol");
        let anchor = buf.cell((0, 0)).expect("the band wrote this cell");
        assert!(
            anchor.symbol().len() > 16,
            "the settled anchor cell carries the real sixel payload, not a bare space"
        );

        // A second settled render of the SAME row is a cache hit, not a rebuild —
        // exactly one full emit per settle, not one per subsequent frame.
        r.render_row(&picker, &band, Rect::new(0, 0, 2, 1), None, false, &mut buf);
        assert_eq!(r.cache.len(), 1, "a settled re-render of the same row reuses the cached protocol");
    }
}
