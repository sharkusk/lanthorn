//! Renders `WinNode::Graphics` canvases via ratatui-image, caching the built
//! protocol per (window, canvas version, area size).

use ratatui::buffer::Buffer;
use ratatui::layout::{Rect, Size};
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use ratatui_image::{Image, Resize};

use crate::engine::GraphicsWindow;
use crate::render::v6_layout::RasterFrame;

/// Two RGBA samples are "the same colour" within a small tolerance (anti-alias slack).
fn close(a: image::Rgba<u8>, b: image::Rgba<u8>) -> bool {
    (0..4).all(|i| a[i].abs_diff(b[i]) <= 8)
}

/// The native source pixel a scaled-canvas pixel `sp` samples under the `image`
/// crate's Nearest resize from `np` native pixels to `sp_max` scaled pixels:
/// `floor((sp + 0.5) · np / sp_max)`, clamped into `[0, np)`. Used only to bound
/// a band's native footprint for its freshness hash (SQ-0514); it mirrors, and
/// never replaces, the crate's own resize.
fn scaled_to_native(sp: u32, np: u32, sp_max: u32) -> u32 {
    let v = ((sp as f32 + 0.5) * np as f32 / sp_max.max(1) as f32).floor() as i64;
    v.clamp(0, np as i64 - 1) as u32
}

/// Resample `src` to `tw × th`, picking the filter per axis by the DIRECTION that
/// axis moves in (SQ-0824).
///
/// Nearest is what keeps pixel art crisp when an axis GROWS: it replicates whole
/// source pixels and invents no colours — a 1.48× magnification of Journey's canyon
/// plate comes back with the same 14 colours it went in with, where Triangle returns
/// 1636. It is exactly the wrong filter when an axis SHRINKS, because the same rule
/// that replicates a pixel on the way up DROPS one on the way down: at the smallest
/// pane swept, 54 of the plate's 222 columns and 56 of its 254 rows were never
/// sampled at all, and a dithered foreground is precisely where "some pixels survive
/// and their neighbours don't" reads as noise.
///
/// Measured against the per-axis ideal (an area average where the axis shrinks,
/// replication where it grows), on that same plate: Nearest scores an RMS of
/// 9.9–10.7 on a minification, Triangle 0.4–1.6. CatmullRom (2.1–2.6) and Lanczos3
/// (3.8–4.1) over-sharpen dithered art — they push adjacent-pixel contrast ABOVE the
/// ideal rather than fusing the dither — and Gaussian (2.4–3.5) over-blurs. Triangle,
/// whose kernel `image` widens to the resampling ratio, is the area filter here.
///
/// The axes go in separate passes because a band can grow on one and shrink on the
/// other — that is what [`GraphicsRender::draw_chrome_band_stretched`] exists for —
/// while `image` takes one filter for the pair. A pass at a 1:1 ratio is a bit-exact
/// identity under either filter, so the ordinary uniform case still costs one resize.
///
/// A BLENDING pass runs on ASSOCIATED (premultiplied) colour, and that is not a
/// refinement (SQ-0827). `image` filters the four channels independently, so where an
/// opaque pixel meets a transparent one it averages the transparent pixel's RGB —
/// which is `(0,0,0)`, a colour nothing on screen ever had — into its neighbour, and
/// drops the alpha to match. Composited, the pair reads as a dark fringe one pixel
/// wide: a Zork Zero flank whose art ends in the story page abutting clear canvas
/// emitted `(38,38,38,57)` at the seam, which over that page draws
/// `38·57/255 + 173·198/255 = 142` against the page's own 173 — the reported line down
/// both edges of the story pane. Nearest never blended, so the defect arrived with the
/// area filter above and not with anything that touched it.
///
/// Associating first makes the average one of light-with-nothing rather than
/// light-with-black, and the seam comes back out as page. It costs nothing where it is
/// not needed: at `a = 255` both directions round-trip exactly (`(255v+127)/255 = v`),
/// so a fully opaque plate — Journey's canyon, every RMS figure above — is bit-identical
/// either way, and a magnifying pass skips the conversion entirely.
///
/// # What it guarantees, to any caller
///
/// This is deliberately GENERAL, not v6-private: it takes an image and a target size and
/// assumes nothing about unit space, chrome bands or the Z-machine. Two guarantees, and
/// they were the two nothing else in the app had until SQ-0829 brought the rest of the
/// art scaling here — Glulx's `glk_image_draw_scaled`
/// ([`Canvas::draw_image`](crate::graphics::Canvas::draw_image)), inline transcript
/// pictures ([`super::inline_image::fit_preserving_aspect`]) and, through
/// [`fit_for_protocol`], cover art, gallery tiles, the resource preview and the
/// non-kitty graphics-window blit:
///
/// * **Direction.** Each axis is filtered by the way it moves — replication when it grows
///   (pixel art stays crisp and gains no colours) and an area average when it shrinks
///   (nothing is dropped). A band that grows on one axis and shrinks on the other gets
///   both, in separate passes.
/// * **Alpha.** A blending pass runs on associated colour, so a transparent neighbour
///   contributes its coverage and not its `(0,0,0)`.
///
/// An axis that does not move is a bit-exact identity, so the common uniform case still
/// costs one resize.
///
/// `pub` because it is also the ORACLE for SQ-0973: "the half-blocks composite is one
/// resample of the native canvas" is a claim an integration test has to be able to
/// state, and one application of this function to the canvas is what states it.
pub fn resize_directional(src: &image::RgbaImage, tw: u32, th: u32) -> image::RgbaImage {
    use image::imageops::FilterType;
    let pick = |t: u32, s: u32| if t < s { FilterType::Triangle } else { FilterType::Nearest };
    let (sw, sh) = src.dimensions();
    let (fx, fy) = (pick(tw, sw), pick(th, sh));
    // Only a filter that AVERAGES neighbours can smear a transparent pixel's colour
    // into an opaque one; Nearest picks one source pixel whole.
    let blends = fx == FilterType::Triangle || fy == FilterType::Triangle;
    let associated;
    let src = if blends {
        associated = associate_alpha(src);
        &associated
    } else {
        src
    };
    let mut out = if fx == fy {
        image::imageops::resize(src, tw, th, fx)
    } else {
        let mid = image::imageops::resize(src, tw, sh, fx);
        image::imageops::resize(&mid, tw, th, fy)
    };
    if blends {
        unassociate_alpha(&mut out);
    }
    out
}

/// The size an aspect-preserving fit of `w × h` into `nw × nh` lands on — the
/// arithmetic `ratatui-image` calls `fit_area_proportionally`, reproduced here so
/// [`fit_for_protocol`] can land on exactly the pixels the crate would have chosen
/// and differ from it ONLY in the filter.
fn fit_proportionally(w: u32, h: u32, nw: u32, nh: u32) -> (u32, u32) {
    let ratio = (nw as f64 / w.max(1) as f64).min(nh as f64 / h.max(1) as f64);
    ((((w as f64) * ratio).round() as u32).max(1), (((h as f64) * ratio).round() as u32).max(1))
}

/// Pre-scale `img` for [`Picker::new_protocol`] through [`resize_directional`],
/// and return it with the cell [`Size`] to hand the picker (SQ-0829).
///
/// Hand the pair to `new_protocol` with `Resize::Fit(None)`, which is then a
/// no-op: the returned image is already exactly `size × font_size` pixels, so the
/// crate's own `needs_resize` short-circuits and never resamples. That is the whole
/// point — `Resize::Fit(None)` means "resize with the DEFAULT filter", and the
/// default is `FilterType::Nearest`. Every delegation to it was therefore a Nearest
/// resample chosen by omission rather than on purpose, and `Fit` is overwhelmingly
/// a MINIFICATION: a 1200×1600 cover into a 20-cell panel is a 7× reduction in
/// which Nearest keeps one source row in seven and throws the rest away.
///
/// `upscale` picks between the crate's two aspect-preserving modes — `false` is
/// `Resize::Fit` (clamped to the image's own size, so a picture smaller than the
/// box is not blown up to fill it) and `true` is `Resize::Scale` (fills the box in
/// both directions). Aspect is preserved either way, so both axes always travel the
/// same way and [`resize_directional`]'s per-axis split costs nothing here; what it
/// brings is the direction itself, plus the alpha association a cut-out PNG needs.
///
/// The leftover strip below/right of the fitted picture is transparent padding, laid
/// down top-left — byte for byte what the crate does with a picker that was never
/// given a `background_color`, which is every picker lanthorn builds.
pub fn fit_for_protocol(
    picker: &Picker,
    img: &image::DynamicImage,
    target: Size,
    upscale: bool,
) -> (image::DynamicImage, Size) {
    if target.width == 0 || target.height == 0 {
        return (img.clone(), target);
    }
    let g = fit_geometry(picker.font_size(), (img.width(), img.height()), target, upscale);
    let (pw, ph) = g.boxed;
    let (tw, th) = g.pic;
    let scaled = resize_directional(&img.to_rgba8(), tw, th);
    let out = if (tw, th) == (pw, ph) {
        scaled
    } else {
        let mut padded = image::RgbaImage::new(pw, ph);
        image::imageops::replace(&mut padded, &scaled, 0, 0);
        padded
    };
    (image::DynamicImage::ImageRgba8(out), g.cells)
}

/// Where an aspect-preserving fit lands, in cells and in pixels — the whole of
/// [`fit_for_protocol`]'s geometry, kept apart from its resample so the half-blocks
/// arm of [`fitted_protocol`] can reach the same cells by a different route (SQ-0979).
///
/// The two answers cannot be allowed to differ. `Protocol::size()` is what every
/// caller centres against — [`GraphicsRender::render`]'s letterbox, the resource
/// preview's, and the gallery's own `fitted_tile_rect` — so a cell rect that moved
/// when the backend changed would be a layout change wearing a performance fix's
/// clothes. `fitted_cells_match_the_prescale` pins them against each other.
struct FitGeometry {
    /// The cell rect the protocol will report.
    cells: Size,
    /// The fitted picture, in device pixels.
    pic: (u32, u32),
    /// `cells` in device pixels — what `pic` is padded out to, top-left.
    boxed: (u32, u32),
}

fn fit_geometry(
    fs: ratatui_image::FontSize,
    src: (u32, u32),
    target: Size,
    upscale: bool,
) -> FitGeometry {
    let (fw, fh) = (fs.width.max(1) as u32, fs.height.max(1) as u32);
    let (sw, sh) = (src.0.max(1), src.1.max(1));
    let (bw, bh) = (target.width as u32 * fw, target.height as u32 * fh);
    // Which cell rect the protocol will report: the fit, rounded UP to whole cells.
    let (cw, ch) = if upscale { (bw, bh) } else { (bw.min(sw), bh.min(sh)) };
    let (aw, ah) = fit_proportionally(sw, sh, cw, ch);
    let cells = Size::new(aw.div_ceil(fw) as u16, ah.div_ceil(fh) as u16);
    // Then the pixels inside it. The second fit is not redundant: the ceil above can
    // add up to a cell on each axis, and the crate resamples into that whole box.
    let boxed = (cells.width as u32 * fw, cells.height as u32 * fh);
    FitGeometry { cells, pic: fit_proportionally(sw, sh, boxed.0, boxed.1), boxed }
}

/// The protocol a fitted picture goes on screen as: cover art, gallery tiles, the
/// resource preview and the non-kitty graphics-window blit (SQ-0979).
///
/// One call in place of the [`fit_for_protocol`] + `new_protocol(.., Resize::Fit(None))`
/// pair those four sites each wrote out, because on HALF-BLOCKS that pair resamples
/// twice. `fit_for_protocol` pre-scales to the pane's device pixels, and
/// `Halfblocks::encode` then takes whatever it is handed straight back down to
/// `cols x 2·rows` samples — one per column, two per row, `font_size` thrown away. At
/// a 20x11 gallery tile on a 10x20 font that is a 200x220 intermediate built to reach
/// a 20x22 grid, and up-then-down through two filters is blurrier than one pass down.
/// So half-blocks resamples ONCE, onto the sample grid itself, exactly as the v6
/// composite has since SQ-0973.
///
/// **Only half-blocks.** Kitty, sixel and iTerm2 genuinely encode pixels, so the
/// pre-scale is the right shape for them and is byte for byte where SQ-0829 left it —
/// and `GraphicsRender::render` never reaches here under kitty at all, which places
/// its own canvas. `only_halfblocks_leaves_the_fit_prescale_behind` is that claim.
///
/// The CELL RECT is the same either way: [`fit_geometry`] answers it once, and the
/// half-blocks arm maps the picture onto its grid rather than choosing a new box. So
/// nothing centred against `Protocol::size()` moves.
///
/// `None` when the protocol fails to build — every caller draws nothing, which is what
/// each of them did with the `Err` this replaces.
pub fn fitted_protocol(
    picker: &Picker,
    img: &image::DynamicImage,
    target: Size,
    upscale: bool,
) -> Option<Protocol> {
    if picker.protocol_type() == ratatui_image::picker::ProtocolType::Halfblocks
        && target.width > 0
        && target.height > 0
    {
        return halfblocks_fitted_protocol(picker, img, target, upscale);
    }
    let (img, size) = fit_for_protocol(picker, img, target, upscale);
    picker.new_protocol(img, size, Resize::Fit(None)).ok()
}

/// A fitted picture as half-blocks, resampled EXACTLY ONCE (SQ-0979).
///
/// The grid is `cells.width x 2·cells.height` samples, and the picture's own extent on
/// it is the device-pixel fit scaled into that grid — `pic.0 · cols / boxed.0` across
/// and `pic.1 · 2·rows / boxed.1` down. Those two ratios are not the same number: a
/// cell is one sample wide and two tall whatever the font's aspect is, so the mapping
/// is anisotropic and the picture's own aspect is carried by `pic`, which was fitted in
/// square device pixels. The leftover — under a cell's worth on the axis the fit did
/// not bind, so at most one column or two rows here — stays transparent padding laid
/// down top-left, as [`fit_for_protocol`] leaves it, because `Halfblocks::encode`
/// resolves alpha to black and that black margin is what the letterbox already showed.
///
/// [`resize_directional`] does the one resample, so SQ-0829's two guarantees hold on
/// the grid that is actually drawn rather than on device pixels the backend discards:
/// each axis filtered by the direction IT moves (and the two can differ here, where
/// they never could in device space — a photograph reduced into a tile shrinks on both,
/// while a small Scott room picture magnified to fill its window can still shrink
/// vertically onto the sample grid), and a blending pass on associated colour so a
/// cut-out edge is not averaged toward `(0,0,0)`.
fn halfblocks_fitted_protocol(
    picker: &Picker,
    img: &image::DynamicImage,
    target: Size,
    upscale: bool,
) -> Option<Protocol> {
    use ratatui_image::protocol::halfblocks::Halfblocks;
    let g = fit_geometry(picker.font_size(), (img.width(), img.height()), target, upscale);
    let (gw, gh) = (u32::from(g.cells.width), u32::from(g.cells.height) * 2);
    let map = |v: u32, from: u32, to: u32| {
        ((f64::from(v) * f64::from(to) / f64::from(from.max(1))).round() as u32).clamp(1, to.max(1))
    };
    let (sx, sy) = (map(g.pic.0, g.boxed.0, gw), map(g.pic.1, g.boxed.1, gh));
    let scaled = resize_directional(&img.to_rgba8(), sx, sy);
    let grid = if (sx, sy) == (gw, gh) {
        scaled
    } else {
        let mut padded = image::RgbaImage::new(gw, gh);
        image::imageops::replace(&mut padded, &scaled, 0, 0);
        padded
    };
    let hb = Halfblocks::new(image::DynamicImage::ImageRgba8(grid), g.cells).ok()?;
    Some(Protocol::Halfblocks(hb))
}

/// Straight (unassociated) RGBA → premultiplied, for [`resize_directional`].
fn associate_alpha(src: &image::RgbaImage) -> image::RgbaImage {
    let mut out = src.clone();
    for p in out.pixels_mut() {
        let a = u32::from(p.0[3]);
        for c in 0..3 {
            p.0[c] = ((u32::from(p.0[c]) * a + 127) / 255) as u8;
        }
    }
    out
}

/// Premultiplied RGBA → straight, undoing [`associate_alpha`] after the resample.
///
/// Triangle's weights are non-negative and sum to one, so no channel can come back
/// above its own alpha and the division cannot overflow; the `min` is belt and braces
/// against a future filter with negative lobes.
fn unassociate_alpha(img: &mut image::RgbaImage) {
    for p in img.pixels_mut() {
        let a = u32::from(p.0[3]);
        if a == 0 {
            p.0 = [0, 0, 0, 0];
            continue;
        }
        for c in 0..3 {
            p.0[c] = ((u32::from(p.0[c]) * 255 + a / 2) / a).min(255) as u8;
        }
    }
}

/// The image the v6 raster composite goes to the protocol as, and the fit mode that
/// finishes it — the whole of [`GraphicsRender::encode_v6`]'s resampling decision,
/// kept pure so it can be measured (SQ-0824).
///
/// `Resize::Fit` only ever SHRINKS, so a pane bigger than the composite needs the
/// magnification done here: Nearest, capped at `max_upscale`, after which the
/// protocol's own (also Nearest) fit at most nudges the result onto the cell grid.
/// The ceiling is the BACKEND's, not this function's — [`v6_upscale_cap`] answers
/// it, and `None` means the backend has no encode to budget for (SQ-0964).
///
/// A pane SMALLER than the composite needs no pre-scale at all, and that is the fix.
/// The scale used to be clamped at 1.0, which turned this branch into a full identity
/// copy of the canvas that bought nothing — and then left the actual shrink to the
/// protocol's DEFAULT filter, Nearest, which drops whole rows and columns exactly
/// where Journey's dithered foreground keeps its detail. Naming the area filter makes
/// it one resample, from the best source there is, in the right direction.
pub fn v6_fit_source(
    canvas: &image::RgbaImage,
    box_w: u32,
    box_h: u32,
    lock: Option<f32>,
    max_upscale: Option<f64>,
) -> (image::RgbaImage, Resize) {
    let (cw, ch) = canvas.dimensions();
    // The backend's ceiling on magnification, or none at all (SQ-0964). Whatever it
    // allows, the BOX still decides how big the composite gets: every scale below is
    // derived from `box_w`/`box_h` (the locked one via the same pane, see below), so
    // lifting the ceiling lets the composite reach the pane and never past it.
    let capped = |s: f64| match max_upscale {
        Some(c) => s.min(c),
        None => s,
    };
    // SQ-0936: the LOCKED magnification, when the pane has one. The raster arm used
    // to compute its own free scale here and so never saw `v6_pixel_lock` at all —
    // which is not the "the setting does nothing in raster mode" caveat it looks
    // like, because a title that publishes no primary Buffer falls through to this
    // arm in HYBRID mode too. scopa (three Grids and `erase_window` fills, SQ-0711)
    // and fmvpoker both do, and both measured 0 differing pixels with the lock on
    // and off until this existed.
    //
    // A locked scale is always <= the free one, so the pre-scaled image always fits
    // its box and the protocol's `Fit` (which only ever shrinks) leaves it alone and
    // centres it.
    if let Some(s) = lock.filter(|s| s.is_finite() && *s > 0.0) {
        let s = capped(f64::from(s));
        let (tw, th) = (((cw as f64 * s) as u32).max(1), ((ch as f64 * s) as u32).max(1));
        // Nearest in BOTH directions here, unlike the free path below, and that is
        // the point rather than an oversight: a locked scale puts one art pixel on a
        // whole number of device pixels, so nearest is exact — it duplicates or
        // drops whole pixels and invents no intermediate colour. Area-averaging an
        // exact 1/2 would blend pairs that the original never blended.
        let scaled = image::imageops::resize(canvas, tw, th, image::imageops::FilterType::Nearest);
        return (scaled, Resize::Fit(None));
    }
    let scale = capped((box_w as f64 / cw as f64).min(box_h as f64 / ch as f64));
    if scale < 1.0 {
        return (canvas.clone(), Resize::Fit(Some(image::imageops::FilterType::Triangle)));
    }
    let (tw, th) = ((cw as f64 * scale) as u32, (ch as f64 * scale) as u32);
    let scaled =
        image::imageops::resize(canvas, tw.max(cw), th.max(ch), image::imageops::FilterType::Nearest);
    (scaled, Resize::Fit(None))
}

/// The composite as the protocol must HOLD it — padded out to the whole cells it
/// will be placed over — and the picture's own device-pixel rect inside that box
/// (SQ-1081).
///
/// **Kitty scales a virtual placement's image to the cell rectangle it covers.**
/// lanthorn's own transmit says so in as many words (`r=`/`c=` in
/// [`kitty_transmit_virtual`]), `ratatui-image`'s says it by omission, and the
/// placement oracle — a port of Ghostty's core — resolves it that way, which is why
/// `tests/pty_stream/raster.rs` composites by scaling a placement's source rect onto
/// its destination. So wherever the image and the cells it is placed over disagree,
/// the pixels the player sees are the protocol's image RESAMPLED BY THE TERMINAL,
/// through whatever filter that terminal smooths with — and against artwork with no
/// intermediate tones in it (`machine-screenshots/amiga-journey.png`, the Amiga
/// release at the party menu) that is pure loss: every edge in the frame softened to
/// reconstruct detail the source never had. Sixel and iTerm2 draw at their own pixel
/// size instead, so for them the pad is inert — a transparent margin that merely makes
/// the picture centre in the cells the placement already reserved.
///
/// They disagree because [`Picker::new_protocol`] SHORT-CIRCUITS. Its `needs_resize`
/// returns `None` the moment the pre-scaled image already fits, handing the protocol
/// the image untouched under a cell rect rounded UP — so the composite is placed over
/// a box up to a cell taller and wider than itself. Measured on a 640x400 press at
/// the gallery's 16x32 kitty cell:
///
/// ```text
///   pane     s      composite     placed over        terminal stretch
///   82x28    2.00   1280x800   →  80x25 = 1280x800     1.00000   (exact)
///   60x24    1.50    960x600   →  60x19 = 960x608      1.01333   ← every row resampled
///   76x40    1.90   1216x760   →  76x24 = 1216x768     1.01053
/// ```
///
/// That is the whole of "a fractional magnification interpolates every edge", and it
/// is why the 0.3 gallery had to pin its raster shots to a whole `s`: at `s = 2` a
/// 640x400 press lands on `16s x 32s` exactly and the stretch is 1. It is not about
/// the fraction as such — `s = 1.2` on an 8x16 cell reaches 768x480, whole cells on
/// both axes, and is already exact — it is about landing on the CELL GRID, which a
/// whole `s` on a matched cell does by luck and a fractional one essentially never does.
///
/// Padding costs nothing anyone can see: the cell rect is `ceil(pixels / cell)` either
/// way, so the composite occupies the SAME cells and nothing centred against
/// `Protocol::size()` moves. What changes is that the terminal has a 1:1 blit to do
/// instead of a resample. The pad is split evenly around the picture, because this
/// image is a game screen being letterboxed rather than a picture in a box — one more
/// sub-cell of margin on each side, not a whole cell of it below. (Only on the arm
/// this pads; the shrinking one belongs to the crate, which lays its picture top-left.)
///
/// Nothing here touches the SHRINKING arm, which never had the defect: there the
/// protocol still has a resize to do, `Resize::resize` runs, and it pads its own
/// output onto the cell grid already. All this wants from that arm is where the
/// picture will land inside it, which is [`fit_proportionally`] — the same arithmetic
/// the crate's `needs_resize_pixels` performs, reproduced here for exactly this
/// purpose.
fn v6_pad_to_cells(
    img: image::RgbaImage,
    box_w: u32,
    box_h: u32,
    fs: ratatui_image::FontSize,
) -> (image::RgbaImage, (u32, u32, u32, u32)) {
    let (fw, fh) = (u32::from(fs.width.max(1)), u32::from(fs.height.max(1)));
    let (iw, ih) = img.dimensions();
    if iw > box_w || ih > box_h {
        let (dw, dh) = fit_proportionally(iw, ih, box_w.min(iw), box_h.min(ih));
        return (img, (0, 0, dw, dh));
    }
    let (pw, ph) = (iw.div_ceil(fw) * fw, ih.div_ceil(fh) * fh);
    if (pw, ph) == (iw, ih) {
        return (img, (0, 0, iw, ih));
    }
    let (ox, oy) = ((pw - iw) / 2, (ph - ih) / 2);
    let mut padded = image::RgbaImage::new(pw, ph);
    image::imageops::replace(&mut padded, &img, i64::from(ox), i64::from(oy));
    (padded, (ox, oy, iw, ih))
}

/// How [`resize_directional`] will treat a resample, for the band log and
/// `/dump-windows` (SQ-0824). Which filter a band went through is not inferable from
/// its cell rect — the direction depends on the band's own native extent against its
/// device box, and a band that magnifies sits beside one that shrinks — and "is this
/// art being minified?" is the question a report of aliasing in fine detail is
/// answered by.
fn resample_note(sw: u32, sh: u32, tw: u32, th: u32) -> String {
    let axis = |t: u32, s: u32| if t < s { "area" } else { "nearest" };
    format!("resample {sw}x{sh}->{tw}x{th} x:{} y:{}", axis(tw, sw), axis(th, sh))
}

/// How many native pixels beyond a band's own footprint can alter its scaled pixels
/// (SQ-0824). Nearest samples exactly one native pixel per scaled pixel, so a
/// magnifying (or 1:1) letterbox has no halo at all; a minifying one goes through
/// Triangle, whose kernel `image` widens to the resampling ratio, so a scaled pixel
/// averages roughly `1/s` native pixels either side of its centre. The band freshness
/// hash in [`GraphicsRender::draw_chrome_band`] covers the footprint plus this halo,
/// or a change just outside a band's own native rect could alter its image without
/// altering its key.
fn scale_halo(s: f32) -> u32 {
    if s >= 1.0 || !s.is_finite() || s <= 0.0 {
        0
    } else {
        (1.0 / s).ceil() as u32 + 1
    }
}

/// Fold the canvas pixels of the native rect `[x0, x1) × [y0, y1)` into `h` —
/// the content half of every band freshness hash. One definition so the crop
/// and stretch draws cannot disagree about what "the footprint's pixels"
/// means, and so the walk has one place to be made fast.
///
/// SQ-1189: one hasher write per ROW over `as_raw()`, not one `Hash` call per
/// pixel. `get_pixel().0.hash()` cost four bounds-checked sample reads plus a
/// SipHash round per pixel — ~1M rounds for a 640x400 footprint — where the
/// backing buffer is already one contiguous RGBA byte run per row. Semantics
/// are equivalent by construction: the same canvas hashes the same bytes in
/// the same order, and any changed pixel inside the footprint changes the byte
/// stream. The footprint bounds (and their SQ-0824 halo) are unchanged — the
/// callers still hash the coords themselves, so a moved footprint over
/// identical bytes still misses.
///
/// The finer generation this cannot become: the chrome canvas is a per-frame
/// COMPOSITE of many windows, the paint ground and the theme's pages, so no
/// single `GraphicsWindow::version` describes it — the version-shaped gate
/// exists one level up instead, where SQ-1187's whole-frame key skips this
/// hash entirely on a replay frame.
fn hash_canvas_rows(
    h: &mut std::collections::hash_map::DefaultHasher,
    canvas: &image::RgbaImage,
    x0: u32,
    x1: u32,
    y0: u32,
    y1: u32,
) {
    use std::hash::Hasher;
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let w = canvas.width() as usize;
    let raw = canvas.as_raw();
    let len = (x1 - x0) as usize * 4;
    for ny in y0..y1 {
        let base = (ny as usize * w + x0 as usize) * 4;
        h.write(&raw[base..base + len]);
    }
}

/// Render a graphics window directly as per-cell background colours when it is a
/// solid fill or a thin strip — the shape games use for chrome: panel dividers,
/// colour bars, backgrounds (e.g. Kerkerkruip draws its rules as 1×N / N×1 solid
/// graphics windows). Returns `true` when it painted the window this way.
///
/// Why not the image protocol: a thin 1-cell strip rendered as a kitty/sixel image
/// sits on a separate compositing layer that doesn't align to the character grid
/// and clobbers adjacent text. Sampling the canvas into cell backgrounds is exact,
/// grid-aligned, cheap, and needs no image-capable terminal. A detailed (non-thin,
/// non-uniform) canvas returns `false` so the caller falls back to the protocol.
/// `force` (v6 layered composite): skip the thin/uniform gate and always paint
/// every cell as the average of its opaque pixels (transparent cells left
/// untouched). This gives a low-res but grid-aligned, letterbox-free composite
/// for overlapping v6 background windows — the image-protocol path would paint a
/// solid grey letterbox over each mostly-empty canvas and clobber the layers
/// beneath. Non-v6 (Glulx) callers pass `false` to keep the detailed-image path.
pub fn render_graphics_as_cells(gw: &GraphicsWindow, area: Rect, buf: &mut Buffer, force: bool) -> bool {
    paint_cell_plan(&classify_graphics_as_cells(gw, area, force), area, buf)
}

/// The outcome of classifying a graphics window for [`render_graphics_as_cells`]:
/// either it stays an image (the caller falls through to the protocol path), or
/// it is drawn as cells — an optional rule glyph, plus each cell's colour
/// (`None` = no opaque pixels there, so the underlying cell is left untouched).
///
/// Split out from the classify/paint walk below so [`GraphicsRender::render_as_cells`]
/// can memoize this outcome (SQ-1200) instead of recomputing it on every redraw.
enum CellPlan {
    Unhandled,
    Handled { line_glyph: Option<&'static str>, colors: Vec<Option<image::Rgba<u8>>> },
}

/// The blank/uniform/rule_like scans and the per-cell region-averaging that used
/// to run inline in [`render_graphics_as_cells`] on every call. Pulled out on its
/// own (SQ-1200) so [`GraphicsRender::render_as_cells`] can memoize the result on
/// `(gw.version, area, force)` and skip re-walking an unchanged canvas.
fn classify_graphics_as_cells(gw: &GraphicsWindow, area: Rect, force: bool) -> CellPlan {
    if area.width == 0 || area.height == 0 {
        return CellPlan::Unhandled;
    }
    let (cw, ch) = (gw.canvas.width(), gw.canvas.height());
    if cw == 0 || ch == 0 {
        return CellPlan::Unhandled;
    }
    // A window with no opaque pixel anywhere is blank — the game opened it but
    // never painted it (narco frames its story with graphics windows it leaves
    // empty). Report it HANDLED (painting nothing) so it does NOT fall through to
    // the image protocol, which would garble a transparent image into stray
    // chars/lines over the neighbouring windows. The scan short-circuits on the
    // first opaque pixel, so a real image pays almost nothing. (SQ-0338)
    if !gw.canvas.pixels().any(|p| p[3] >= 128) {
        let colors = vec![None; area.width as usize * area.height as usize];
        return CellPlan::Handled { line_glyph: None, colors };
    }
    // A cell's colour is the AVERAGE of the OPAQUE pixels in its canvas region, or
    // `None` if the region has none. Scanning the whole region (not just the centre
    // pixel) is essential: games draw their rules as 1–2px lines that rarely sit at
    // a cell's centre — a centre sample would miss them and render nothing. Any
    // opaque pixel in the cell surfaces the line's colour. (SQ-0332)
    let cell_color = |cx: u16, cy: u16| -> Option<image::Rgba<u8>> {
        let px0 = cx as u32 * cw / area.width as u32;
        let px1 = (((cx as u32 + 1) * cw / area.width as u32).max(px0 + 1)).min(cw);
        let py0 = cy as u32 * ch / area.height as u32;
        let py1 = (((cy as u32 + 1) * ch / area.height as u32).max(py0 + 1)).min(ch);
        let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
        for py in py0..py1 {
            for px in px0..px1 {
                let p = gw.canvas.get_pixel(px, py);
                if p[3] >= 128 {
                    r += p[0] as u64;
                    g += p[1] as u64;
                    b += p[2] as u64;
                    n += 1;
                }
            }
        }
        (n > 0).then(|| image::Rgba([(r / n) as u8, (g / n) as u8, (b / n) as u8, 255]))
    };
    // Handle a thin strip (a rule/divider) or a solid uniform fill as cells;
    // otherwise leave it to the image protocol. The uniform scan short-circuits on
    // the first differing/transparent cell, so a detailed image bails fast.
    //
    // A thin window is a RULE only when every opaque cell agrees on one colour
    // (gaps allowed — a partial-length rule). A DETAILED thin canvas — advent.blb's
    // clickable toolbar lands at 2 cells tall — is an image, not a divider:
    // averaging it into ─ glyphs shredded it into "two thin strips of pixels"
    // while the real icons never reached the image protocol. (SQ-0520)
    let thin = area.width.min(area.height) <= 2;
    let first = cell_color(0, 0);
    let uniform = first.is_some()
        && (0..area.height).all(|cy| (0..area.width).all(|cx| cell_color(cx, cy).is_some_and(|c| close(c, first.unwrap()))));
    let rule_like = thin && {
        // All opaque cells close to the first opaque cell's colour.
        let mut reference: Option<image::Rgba<u8>> = None;
        (0..area.height).all(|cy| {
            (0..area.width).all(|cx| match cell_color(cx, cy) {
                None => true,
                Some(c) => match reference {
                    None => {
                        reference = Some(c);
                        true
                    }
                    Some(r) => close(c, r),
                },
            })
        })
    };
    if !(force || rule_like || uniform) {
        return CellPlan::Unhandled;
    }
    // A window ≤2 cells in one dimension IS a rule/divider (Kerkerkruip's panel
    // borders). Draw it as a thin line GLYPH (fg = the rule colour, background
    // untouched) so it reads like a real rule at any width — not a full-cell colour
    // block that looks far thicker than a pixel interpreter's 1–2px bar. Like a
    // pixel interpreter, a white rule on a matching page then stays invisibly
    // subtle. Only larger (background) fills paint the whole cell. (SQ-0332)
    let line_glyph = if thin {
        // Vertical rule (tall & narrow) → │, horizontal rule → ─.
        Some(if area.height >= area.width { "\u{2502}" } else { "\u{2500}" })
    } else {
        None
    };
    let mut colors = Vec::with_capacity(area.width as usize * area.height as usize);
    for cy in 0..area.height {
        for cx in 0..area.width {
            colors.push(cell_color(cx, cy));
        }
    }
    CellPlan::Handled { line_glyph, colors }
}

/// Apply a [`CellPlan`] to `buf` — the write half of what
/// [`render_graphics_as_cells`] used to do inline. Returns whether the window
/// was drawn as cells (`Handled`) or left for the caller's image-protocol
/// fallback (`Unhandled`). Never approximates: the plan already carries the
/// exact per-cell colours [`classify_graphics_as_cells`] computed, so a memoized
/// replay (SQ-1200) paints byte-identically to a fresh classify+paint.
fn paint_cell_plan(plan: &CellPlan, area: Rect, buf: &mut Buffer) -> bool {
    let CellPlan::Handled { line_glyph, colors } = plan else {
        return false;
    };
    for cy in 0..area.height {
        for cx in 0..area.width {
            let idx = cy as usize * area.width as usize + cx as usize;
            let Some(p) = colors[idx] else {
                continue; // no opaque pixels here → leave the underlying cell
            };
            if let Some(c) = buf.cell_mut((area.x + cx, area.y + cy)) {
                let fg = Color::Rgb(p[0], p[1], p[2]);
                match line_glyph {
                    Some(g) => {
                        // Preserve the underlying background; only the glyph + fg change.
                        let mut s = Style::default().fg(fg);
                        if let Some(bg) = c.style().bg {
                            s = s.bg(bg);
                        }
                        c.set_symbol(g).set_style(s);
                    }
                    None => {
                        c.set_symbol(" ").set_style(Style::default().bg(fg));
                    }
                }
            }
        }
    }
    true
}

/// The geometry needed to invert the v6 letterbox mapping: given a terminal
/// cell click, recover the game-pixel coordinate under the pointer. Recorded by
/// the last v6 draw path (single-canvas [`GraphicsRender::draw_v6_canvas`] or the
/// hybrid chrome ring) so [`map_click`](GraphicsRender::map_click) can be a pure
/// inverse of the forward scale-and-centre placement.
///
/// All fields are in the terminal's device-pixel space, measured relative to the
/// v6 pane's top-left cell (`pane_x`, `pane_y`). The drawn game image occupies the
/// device-pixel rect (`img_x`, `img_y`, `img_w`, `img_h`) inside that pane, and
/// maps a `native_w × native_h` game-pixel canvas across it, aspect-preserved.
// SQ-0938: no longer `Copy` — it carries a list of packed regions now, because a
// frame can pack more than one and each publishes its own mapping.
#[derive(Clone, Debug, PartialEq)]
pub struct V6ClickMap {
    pub pane_x: u16,
    pub pane_y: u16,
    pub cell_w: u16,
    pub cell_h: u16,
    pub img_x: f32,
    pub img_y: f32,
    pub img_w: f32,
    pub img_h: f32,
    /// The game-pixel canvas the drawn image maps across. This is the game's own
    /// screen on every path but one: an EXTENDED raster frame (SQ-1032) draws a
    /// taller canvas, whose lower rows are lanthorn's, not the game's.
    pub canvas: (u16, u16),
    /// The game's own screen inside that canvas.
    ///
    /// **The inverse is stated over `canvas`, because that is what was drawn; the
    /// ANSWER is bounded by `screen`, because that is what the game has** (SQ-1032).
    /// A click below an extended frame's screen is in lanthorn's own scrollback and
    /// is dropped — not clamped onto the game's last row, which would hand the game
    /// a plausible coordinate the player did not click. Equal to `canvas` on every
    /// other path, so that rejection is unreachable there by construction.
    pub screen: (u16, u16),
    /// Every region of the pane that is drawn as PACKED CELLS rather than through
    /// the letterbox scale — see [`PackedText`]. Empty when the whole pane is the
    /// scaled image, which is the common case.
    pub packed_text: Vec<PackedText>,
}

/// A rectangle the renderer draws at ONE TERMINAL CELL PER NATIVE TEXT CELL,
/// instead of placing it through the letterbox scale.
///
/// # Why the proportional inverse is wrong inside one
///
/// SQ-0550, on rows, and the reasoning is the general case: a packed region does
/// not inherit the scale's gaps, so "inside this span the linear inverse is wrong,
/// and wrong by a growing amount — at a scale of 1.725 the letterbox spreads game
/// rows 1.53 terminal rows apart while the strip draws them 1 apart, so by the
/// fifth menu row the click lands two game rows high."
///
/// # The rule this expresses
///
/// **A click is resolved the way the pixel under it was drawn.** Proportionally
/// where the frame is an image; by cell index where the text is glyphs. That is
/// why this is a LIST rather than the single strip it began as: any number of
/// regions can be packed on one frame, each publishes its own mapping, and a
/// region nobody has met yet inverts correctly the moment the renderer records it.
/// Nothing here knows which game it is looking at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackedText {
    /// First terminal row, its row count, and the native PIXEL y of the TOP of
    /// the text row drawn at `top` — a pixel rather than a row index for the same
    /// reason `cols` carries one below: a story box does not begin on a multiple
    /// of the cell.
    ///
    /// SQ-0951 measured what the row index cost. Zork Zero's InvisiClues box
    /// starts at native y=78 and prints its first topic at y=79, so its rows are
    /// drawn at 79..94, 95..110, … while row index `(79-1)/16 = 4`'s grid slot is
    /// 64..79. Inverting through the index returned the middle of the SLOT (72),
    /// which lands in the row ABOVE the one under the pointer — so clicking
    /// GENERAL QUESTIONS selected THE JESTER, on every pane width, and only
    /// Shogun's luckier phase (box at y=70, text at 71) hid it there.
    pub rows: (u16, u16, u16),
    /// First terminal column, its column count, and the native PIXEL x drawn at
    /// `left` — a pixel rather than a column index because a story box does not
    /// begin on a multiple of the cell (Zork Zero's starts at native x=86).
    ///
    /// `None` where the region packs only its rows and still places columns
    /// through the scale, which is what a chrome text strip does.
    pub cols: Option<(u16, u16, u16)>,
}

impl PackedText {
    /// Whether this region packs `row` — asked WITHOUT the column, because the
    /// rows a region packs are the pane's row layout across its whole width.
    ///
    /// SQ-0951: a region that packs its columns too used to publish its row
    /// mapping only inside them, so a click one column left of a topic — in the
    /// ring's flank, which is a TILED band and not on the letterbox grid at all —
    /// fell through to the proportional inverse and reported a native y from a
    /// completely different part of the screen. At a 50x60 pane that turned a
    /// click beside GLACIER into GENERAL QUESTIONS, thirteen items lower: "click
    /// to the left of a selection and it misclicks several items lower". A
    /// column-less region (a chrome text strip) has always behaved this way; this
    /// is only the same rule reaching the regions that also pack columns.
    fn holds_row(&self, row: u16) -> bool {
        let (top, count, _) = self.rows;
        row >= top && row < top.saturating_add(count)
    }

    fn holds(&self, col: u16, row: u16) -> bool {
        self.holds_row(row)
            && self.cols.is_none_or(|(left, n, _)| col >= left && col < left.saturating_add(n))
    }
}

impl V6ClickMap {
    /// Map a terminal cell click at `(col, row)` to a 1-based game pixel
    /// `(x, y)`, or `None` when the cell lies outside the drawn game image
    /// (the letterbox margin, or another pane). This is the exact inverse of the
    /// forward letterbox: the forward map places game pixel `p` (0-based) at
    /// device offset `img_origin + (p + ½)·(img_size/native)`; inverting, a click
    /// at device position `d` recovers `p = ⌊(d − img_origin)/img_size · native⌋`.
    /// The click's device position is taken at the clicked cell's centre, giving
    /// a subcell estimate finer than the cell grid.
    pub fn map_click(&self, col: u16, row: u16) -> Option<(u16, u16)> {
        if self.img_w <= 0.0 || self.img_h <= 0.0 || self.canvas.0 == 0 || self.canvas.1 == 0 {
            return None;
        }
        if col < self.pane_x || row < self.pane_y {
            return None;
        }
        // Device-pixel position of the clicked cell's centre, relative to pane.
        let dx = (col - self.pane_x) as f32 * self.cell_w as f32 + self.cell_w as f32 / 2.0;
        let dy = (row - self.pane_y) as f32 * self.cell_h as f32 + self.cell_h as f32 / 2.0;
        let fx = (dx - self.img_x) / self.img_w;
        if !(0.0..1.0).contains(&fx) {
            return None;
        }
        // A PACKED region inverts by cell index, not by the letterbox — on whichever
        // axes it packs. Its own cells are drawn, so they need no range check. The
        // two axes are asked SEPARATELY (SQ-0951): a region's rows govern the pane's
        // whole width, its columns only the span it packs them into.
        let row_packed = self.packed_text.iter().find(|p| p.holds_row(row));
        let packed = self.packed_text.iter().find(|p| p.holds(col, row));
        let gx = match packed.and_then(|p| p.cols) {
            // The clicked column's game pixel is that cell's HORIZONTAL middle, so
            // the whole terminal cell selects the character the player sees on it —
            // the same rule the row mapping below uses vertically.
            Some((left, _, native_x0)) => {
                // A packed region's overshoot is a ROUNDING TAIL of cell arithmetic
                // over cells the renderer drew, not a click outside the frame, so it
                // is clamped exactly as it always was. (Packed regions belong to the
                // hybrid ring, which never extends — `redraw_v6` passes none — so
                // this clamp and the bound below can never disagree.)
                (u32::from(native_x0) + u32::from(col - left) * 8 + 4).min(u32::from(self.screen.0))
            }
            None => {
                let gx = (fx * self.canvas.0 as f32).floor() as u32 + 1;
                // Outside the game's own screen → not the game's click (SQ-1032).
                // Inert until a frame extends sideways, which none does; stated
                // anyway because `screen` and `canvas` are one subject and an
                // asymmetric bound is how the next axis gets forgotten.
                if gx > u32::from(self.screen.0) {
                    return None;
                }
                gx
            }
        };
        let gy = match row_packed {
            Some(p) => {
                // The clicked row's game pixel is that row's VERTICAL middle,
                // counted in native pixels from the top of the row drawn first —
                // the whole terminal row therefore selects the line the player
                // sees on it, whatever sub-cell phase the region begins on.
                let (top, _, native_y0) = p.rows;
                (u32::from(native_y0) + u32::from(row - top) * 16 + 8).min(u32::from(self.screen.1))
            }
            None => {
                let fy = (dy - self.img_y) / self.img_h;
                if !(0.0..1.0).contains(&fy) {
                    return None;
                }
                let gy = (fy * self.canvas.1 as f32).floor() as u32 + 1;
                // The rejection this quest is actually about: a click in the rows an
                // EXTENDED frame added below the game's screen. Those rows carry
                // lanthorn's scrollback, drawn in the game's face; the game never had
                // them and must not be told it was clicked on its last one.
                if gy > u32::from(self.screen.1) {
                    return None;
                }
                gy
            }
        };
        Some((gx as u16, gy as u16))
    }
}

/// Largest integer upscale the v6 raster composite is encoded at (SQ-0469). A
/// native 320×200 game therefore encodes at most 1280×800 instead of the full
/// pane device resolution (which could be ~1920×1200 = 9 MB and cost hundreds of
/// ms to resize+PNG-encode). The protocol still fits/centres the smaller image in
/// the pane, so the only visible effect is that pixel art stops growing past this
/// multiple of its native size — crisp Nearest scaling either way.
///
/// SQ-0479 doubled the native canvas (320×200 → 640×400), so 2× here reaches the
/// SAME 1280×800 output ceiling the old 4× cap gave over the 320×200 canvas — the
/// encoded-pixel budget is unchanged, not quadrupled.
///
/// **It is a budget, so it only binds a backend that spends one** — see
/// [`v6_upscale_cap`], which is what the cap is read through.
const MAX_V6_UPSCALE: f64 = 2.0;

/// How far this backend may magnify the v6 raster composite before the protocol
/// fits it into the pane: [`MAX_V6_UPSCALE`], or **no ceiling at all** (SQ-0964).
///
/// The cap is a PNG-encode budget and nothing else. Kitty and iTerm2 ship the
/// composite down the wire as encoded pixels every frame it changes, and sixel
/// encodes one too, so every extra factor of magnification is bytes to build and
/// bytes to write — there the ceiling earns its keep and stays exactly where it is.
///
/// Half-blocks encodes nothing. `ratatui-image` resolves the image straight into
/// terminal cells at one pixel per column and two per row, so the budget it is
/// protecting does not exist — while the COST is entirely real, because under
/// `Resize::Fit` (which only ever shrinks) the pre-scale here is what decides how
/// many CELLS the composite occupies. Capped at 2×, a 640×400 canvas reaches a
/// fixed 1280×800 nominal pixels — a fixed number of cells — while shrinking the
/// terminal font goes on giving the pane more of them. So the picture that should
/// have grown sharper as the grid got finer visibly SHRANK instead, worst on the
/// titles that fall through to the composite whatever the mode (scopa and fmvpoker,
/// which publish no primary Buffer — SQ-0711).
///
/// Removing the ceiling does not remove a bound: the free scale is derived from the
/// pane box and the locked one from the same pane's ladder (SQ-0945), so the
/// composite still stops at the pane. It may simply climb the whole way there.
pub fn v6_upscale_cap(picker: &Picker) -> Option<f64> {
    match picker.protocol_type() {
        ratatui_image::picker::ProtocolType::Halfblocks => None,
        _ => Some(MAX_V6_UPSCALE),
    }
}

/// Whether the v6 magnification LOCK has a rung on this backend to snap to at all
/// (SQ-0978).
///
/// `v6_pixel_lock` promises **one art pixel on a whole number of device pixels**, and
/// [`crate::render::v6_layout::locked_scale`] delivers it by quantizing the letterbox
/// factor `s` in device pixels — `pane_dev = cells x picker.font_size()`. That is exact
/// for every backend that ships pixels: kitty, iTerm2 and sixel each put the composite
/// on the screen at the device resolution the pane really has.
///
/// Half-blocks does not have one. `Picker::halfblocks()` — and `from_query_stdio`'s
/// default when a terminal answers no cell size — hardcodes `FontSize::new(10, 20)`
/// whatever the real font is, and the encoder then **throws the font size away**:
/// `Halfblocks::encode` resamples whatever it is handed to exactly `width x 2·height`
/// SAMPLES, one per column and two per row. So the grid the picture actually resolves
/// onto is a property of the CELL grid, and the device pixels `s` was quantized in are
/// a number the picker invented.
///
/// The honest analogue would be to quantize onto that sample grid instead — one art
/// pixel on a whole number of half-block samples, which is `cols` and `2·rows` and no
/// font size at all. **It was measured and it buys nothing**, and the reason is that
/// half-blocks does not magnify: a 640x400 canvas has more unit pixels than a terminal
/// has samples until the pane reaches 640x200 CELLS, so the composite is minified at
/// every size anyone runs, and [`resize_directional`] minifies through `Triangle`.
/// Measured on a 640x400 canvas of 2x2 art pixels in hard black/white stripes:
///
/// ```text
///   sample grid   ratio   samples that are a PURE art-pixel colour
///   640x400        1:1    640 / 640     (Nearest — the target is not below the source)
///   458x288       1.4:1     50 / 458
///   320x200         2:1      0 / 320    ← an EXACT rung: one art pixel per sample
///   160x100         4:1      0 / 160
/// ```
///
/// The 320x200 row is the whole finding. It is the honest ladder's own rung — one art
/// pixel onto exactly one sample — and Triangle still lands every sample on a 25/75
/// blend of two art pixels, because a separable Triangle at ratio 2 has support 2 in
/// source space and reaches across the art pixel's edge. The rung delivers nothing.
///
/// And below that rung there is nothing to reach for either, because free scaling is
/// never worse: at `s >= 10` nominal device pixels the sample grid is at or above the
/// canvas, `resize_directional` picks `Nearest`, and the art comes out pure ALREADY —
/// while the lock could only move `s` DOWN, off that plateau and into Triangle. Below
/// it every `s` blends, rung or no rung. So on half-blocks the lock has no reachable
/// pane size at which it improves a frame, and a measured 17-20% of linear resolution
/// to lose where it acts: at a 120x40 pane a 640x400 canvas free-scales to 120x38
/// cells and the old device-pixel rung cut it to 96x30.
///
/// So the lock is INERT here, and `/dump-terminal` says so in those words rather than
/// reporting a snap that did not happen. Not a ceiling — SQ-0964 removed the one
/// half-blocks used to carry and nothing here puts it back; the free scale still climbs
/// the whole way to the pane.
///
/// The picker is the thing that knows, exactly as it is for [`v6_upscale_cap`].
pub fn v6_pixel_lock_applies(picker: &Picker) -> bool {
    picker.protocol_type() != ratatui_image::picker::ProtocolType::Halfblocks
}

/// Whether this terminal may be sent a DEFLATED kitty transmission (SQ-0997).
///
/// `o=z` is not a hint a terminal may ignore. One that cannot inflate refuses the
/// transmission outright: the image is never stored, and every placement naming
/// it draws nothing — so a graphics window simply has no picture in it, with no
/// error, no fallback, and nothing on screen to suggest anything went wrong.
///
/// That is why SQ-0991 made `ratatui-image` probe before compressing. Lanthorn's
/// OWN transmit — [`kitty_transmit_virtual`], which SQ-0976 taught to compress
/// before the capability existed — stated `o=z` whatever the probe said, so on
/// such a terminal every graphics-window image (Glulx toolbars, Scott room
/// pictures, v6 graphics windows) vanished silently while the chrome ring beside
/// them, encoded by the crate, drew fine. Two encoders, one wire, one answer.
///
/// The picker is the thing that knows, exactly as it is for [`v6_upscale_cap`].
/// An EMPTY capability list means raw, and it has to: that is what
/// `Picker::halfblocks()`, the deprecated `from_fontsize`, the default picker
/// returned when a query fails, tmux without passthrough and the terminal
/// blacklist all leave behind, and none of them is evidence that this terminal
/// can inflate anything. Fail safe — a raw upload is merely slower, an
/// uninflatable one is invisible.
pub fn kitty_compression(picker: &Picker) -> bool {
    picker.capabilities().contains(&ratatui_image::picker::Capability::KittyCompression)
}

/// The cell rect the v6 composite occupies under HALF-BLOCKS, without building a
/// pixel of it (SQ-0973).
///
/// Same answer [`v6_fit_source`] and the protocol's own `Fit` arrive at together —
/// the composite fitted into the pane's device box at the magnification the pane (or
/// the lock) allows, then rounded up onto the cell grid — reached by arithmetic instead
/// of by a 52 MB intermediate. `v6_halfblocks_grid_matches_the_protocol` pins the two
/// against each other over a pane sweep, in every branch, so the mirror cannot drift.
///
/// No upscale ceiling appears here because half-blocks has none: [`v6_upscale_cap`]
/// answers `None` for it, and this function is reachable only from that arm.
pub fn v6_halfblocks_grid(
    canvas: (u32, u32),
    box_w: u32,
    box_h: u32,
    fs: ratatui_image::FontSize,
    lock: Option<f32>,
) -> Size {
    let (cw, ch) = (canvas.0.max(1), canvas.1.max(1));
    let (fw, fh) = (u32::from(fs.width.max(1)), u32::from(fs.height.max(1)));
    // What `v6_fit_source` hands over: the canvas at the pane's (or the lock's)
    // magnification, truncated exactly as it truncates, and the canvas untouched where
    // it declines to pre-scale at all.
    let (tw, th) = match lock.filter(|s| s.is_finite() && *s > 0.0) {
        // The artwork's own integer ladder (SQ-0945/0936).
        Some(s) => {
            let s = f64::from(s);
            (((cw as f64 * s) as u32).max(1), ((ch as f64 * s) as u32).max(1))
        }
        None => {
            let s = (box_w as f64 / cw as f64).min(box_h as f64 / ch as f64);
            if s < 1.0 {
                (cw, ch)
            } else {
                (((cw as f64 * s) as u32).max(cw), ((ch as f64 * s) as u32).max(ch))
            }
        }
    };
    // Then what `Resize::Fit` does to it, which is the crate's own
    // `fit_area_proportionally(tw, th, min(box_w, tw), min(box_h, th))` — the shrink the
    // pre-scale deliberately leaves to the protocol, and the ONLY thing standing between
    // an over-large locked rung and a composite that overruns its pane. It ROUNDS where
    // the magnification above truncates, and it is an exact identity whenever the
    // pre-scale already fits.
    let ratio = (f64::from(tw.min(box_w)) / f64::from(tw)).min(f64::from(th.min(box_h)) / f64::from(th));
    let (tw, th) = (
        ((f64::from(tw) * ratio).round() as u32).max(1),
        ((f64::from(th) * ratio).round() as u32).max(1),
    );
    Size::new(tw.div_ceil(fw).max(1) as u16, th.div_ceil(fh).max(1) as u16)
}

/// The v6 composite as half-blocks, resampled EXACTLY ONCE (SQ-0973).
///
/// The encoding backends need [`v6_fit_source`]'s pre-scale because `Resize::Fit`
/// never grows: hand it the bare canvas and `needs_resize_pixels`' `min(box, image)`
/// collapses the composite to a native-sized cell footprint. Half-blocks needs none
/// of it, because half-blocks does not encode pixels at all — `Halfblocks::encode`
/// throws away `font_size` and resamples whatever it is given to exactly
/// `width x 2·height` samples, one per column and two per row. The pre-scale was
/// therefore magnifying the canvas to device pixels the backend was about to discard:
/// a 640x400 canvas in a 458x144-cell pane went up to 4580x2862 RGBA (52 MB, Nearest)
/// so that the crate could take it straight back down to 458x288 (Triangle). Two
/// resamples in opposite directions to land BELOW where the canvas started, and
/// Nearest-up-then-Triangle-down is the blur that combination always is.
///
/// So resample once, onto the sample grid itself, through [`resize_directional`] —
/// which picks its filter per axis by the direction that axis moves, and associates
/// alpha before it blends (SQ-0824/0827). The crate's own `resize_exact` then runs at
/// a 1:1 ratio, which is a bit-exact identity, so that filter choice is what reaches
/// the screen rather than being overwritten by the crate's unconditional Triangle.
///
/// This is half-blocks ONLY. Sixel, iTerm2 and kitty keep [`v6_fit_source`] exactly
/// as it was: they ship encoded pixels down the wire, the cap is a real budget for
/// them, and `Fit` plus a pre-scale is the right shape.
fn v6_halfblocks_protocol(
    canvas: &image::RgbaImage,
    box_w: u32,
    box_h: u32,
    fs: ratatui_image::FontSize,
    lock: Option<f32>,
) -> Option<Protocol> {
    use ratatui_image::protocol::halfblocks::Halfblocks;
    let cells = v6_halfblocks_grid(canvas.dimensions(), box_w, box_h, fs, lock);
    let grid = resize_directional(canvas, u32::from(cells.width), u32::from(cells.height) * 2);
    let hb = Halfblocks::new(image::DynamicImage::ImageRgba8(grid), cells).ok()?;
    Some(Protocol::Halfblocks(hb))
}

/// A completed v6 raster encode (SQ-0469): the uploaded protocol plus the key it
/// was built for (`gen` + pane cell size) and the native canvas extent (for the
/// click map). Always rendered once present — a stale entry is shown until a
/// fresher encode lands, so the pane never flickers to blank on a change.
struct V6Ready {
    gen: u64,
    area_w: u16,
    area_h: u16,
    proto: Protocol,
    /// The canvas encoded, and the game's own screen inside it — see
    /// [`V6ClickMap::canvas`] / [`V6ClickMap::screen`]. They differ only for an
    /// extended frame (SQ-1032).
    canvas: (u16, u16),
    screen: (u16, u16),
    /// Where the composite itself lies inside the cell rect the protocol reports,
    /// in device pixels: `(off_x, off_y, w, h)` (SQ-1081).
    ///
    /// It is not the whole of that rect, because the rect is `ceil`ed onto the cell
    /// grid and the picture is centred in what that leaves — see [`v6_pad_to_cells`].
    /// The click map has to invert through the PICTURE and not through the box, or a
    /// click reads a game pixel up to a cell out. (It used to invert through the box
    /// and be right anyway, on the magnifying arm only, because the terminal was
    /// stretching the picture to fill it. On the shrinking arm, where the protocol
    /// has always padded, it was quietly wrong by the same amount.)
    pic: (u32, u32, u32, u32),
    /// The kitty image id this composite was last PLACED as, read back off the
    /// placement (SQ-0753). `None` under a non-kitty protocol, and until the first
    /// [`GraphicsRender::redraw_v6`]. Without it the full-frame composite — the
    /// single largest upload lanthorn makes, 2.8 MB on Journey — can only be
    /// forgotten, never freed.
    placed_id: Option<u32>,
}

/// The worker-thread handle for an in-flight v6 raster encode (SQ-0469). The
/// heavy resize + PNG/protocol encode (tens–hundreds of ms) runs off the UI
/// thread and yields a ready-to-render [`V6Ready`]. Coalesced: only one runs at a
/// time, and `poll_v6_job` installs its result (see `spawn_v6_encode`).
type V6Job = std::thread::JoinHandle<Option<V6Ready>>;

/// SQ-0898: whether a band claims to be showing the game's screen ON the frame's
/// letterbox grid, or is a picture deliberately fitted to a box of its own.
///
/// The distinction exists because "one frame, one magnification" is a property of
/// the FIRST kind and a category error about the second. Every band that shows the
/// v6 screen — a full-width banner tile, a flank crop, a tiled flank extension — is
/// a window onto one canvas scaled by one factor, and two of them at two factors is
/// a seam. The other kind is not on that grid at all and its magnification is
/// chosen for other reasons; there are exactly two, both deliberate and both
/// documented where they are made.
///
/// **The exemption is keyed on the SITE, not on the drawing function** (SQ-0898,
/// second round). It used to be the latter: [`GraphicsRender::draw_chrome_band_stretched`]
/// recorded `Fitted` unconditionally, so every caller of it — present and future —
/// was exempt from the gate by construction. Two of its callers were the deliberate
/// exceptions below; the THIRD was the Frame-plan flank stretch, which inherited
/// their exemption silently and drew Arthur's banner-row poles at 0.60 vertical
/// against the frame's 1.35 for as long as it lived. A caller now names which of
/// these it is, and anything that does not name an exception is held to the frame's
/// magnification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BandFit {
    /// On the letterbox grid: `device == native · s`, to within the half native
    /// pixel that whole-pixel sources cost. Asserted across the corpus and a pane
    /// sweep by `v6_band_tiling::every_band_draws_at_the_frames_one_magnification`.
    Letterbox,
    /// The Menu-plan flank PANEL, whose picture is re-centred and quantized to whole
    /// CELLS by `aspect_cells` (SQ-0547). It is a picture in a panel, not a window
    /// onto the screen, so the frame's scale is not what decides its size.
    MenuPanel,
    /// The divider EXTENSION, which replicates ONE native row down a reclaimed gap
    /// (SQ-0511) and so has no meaningful vertical factor at all.
    DividerExtension,
}

impl BandFit {
    /// Is this band claiming to show the game's screen on the frame's letterbox
    /// grid — i.e. is it one the "one frame, one magnification" gate applies to?
    pub fn on_the_letterbox_grid(self) -> bool {
        matches!(self, BandFit::Letterbox)
    }
}

/// Where a band's source belongs INSIDE that band, in device pixels from the
/// band's own top-left: `(x, y, w, h)` (SQ-0898). The rest of the band is
/// transparent, exactly as a crop leaves the letterbox margin transparent.
pub type BandDest = (u32, u32, u32, u32);

/// One entry of [`GraphicsRender::band_mags`]: the band's cell rect, what it
/// claims to be, its source in NATIVE pixels, and its destination in DEVICE
/// pixels. The magnification is `dst / src`.
pub type BandMag = (Rect, BandFit, (u32, u32), (u32, u32));

/// Which of a frame's draws a cached chrome band belongs to (SQ-0755).
///
/// A band's cache key is the cell rect it is drawn at — but one rect can legitimately
/// be drawn twice on a single frame. Journey's right flank IS its border column, so the
/// flank's own art and the divider extension replicated down the reclaimed gap land on
/// exactly the same cells. With the rect alone as the key each overwrote the other's
/// entry, every frame, so neither was ever a cache hit and both re-encoded forever.
/// Skipping one of the draws is not the answer: they carry different pixels — the flank
/// the column's true native extent, the extension one native row replicated past where
/// the canvas ends — and dropping either loses ink.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BandSlot {
    /// The band's own artwork, at the letterbox scale or stretched to a flank.
    Art = 0,
    /// A flank border column replicated down the gap between the art and the menu.
    DividerExtension = 1,
}

/// A chrome-band cache key: its [`BandSlot`] plus the cell rect it is drawn at.
pub type BandKey = (u8, u16, u16, u16, u16);

/// One band encode staged for the background worker (SQ-1188): the finished
/// (sealed) band image, the cache key it will land under, the content hash it
/// answers for, and the kitty id it re-transmits to (SQ-0996).
struct PendingBand {
    key: BandKey,
    band: Rect,
    hash: u64,
    img: image::DynamicImage,
    reuse: Option<u32>,
}

/// One band the worker finished encoding (SQ-1188). `proto` is `None` when the
/// encode failed — the staging is then un-marked so the next frame retries.
struct BandEncoded {
    key: BandKey,
    band: Rect,
    hash: u64,
    reuse: Option<u32>,
    proto: Option<Protocol>,
}

/// SQ-1188: whether this backend's band encodes are worth a worker thread.
/// Kitty and iTerm2 pay zlib-L6 + base64 per encode and sixel pays icy_sixel's
/// quantization; half-blocks "encodes" straight into cells with no compression
/// stage at all, so the worker would buy a frame of latency and save nothing —
/// it (and therefore every cell-buffer test harness) stays synchronous.
fn band_encode_offthread(picker: &Picker) -> bool {
    !matches!(picker.protocol_type(), ratatui_image::picker::ProtocolType::Halfblocks)
}

#[derive(Default)]
pub struct GraphicsRender {
    cache: std::collections::HashMap<u32, (u64, u16, u16, Protocol)>,
    /// Per-window memo of [`classify_graphics_as_cells`] (SQ-1200): the
    /// blank/uniform/rule_like scans and their region-averaging `cell_color`
    /// walk the whole canvas, so a redraw of an unchanged window — the common
    /// uniform backdrop, `force` included — otherwise pays roughly two full
    /// passes over it for nothing. Keyed and invalidated exactly like `cache`
    /// above — see [`Self::retain_live`] and [`Self::invalidate_cell_geometry`]
    /// — with `force` added to the freshness tuple because the classification
    /// depends on it exactly as directly as it depends on the canvas: the
    /// image-protocol call site asks the SAME window/version/area once with
    /// `force = false` (falls through to the image protocol on a detailed
    /// canvas) and, on its no-picker fallback, once with `force = true`
    /// (always painted as cells) — two different outcomes that must not share
    /// one cached answer.
    cell_memo: std::collections::HashMap<u32, (u64, Rect, bool, CellPlan)>,
    /// Count of [`classify_graphics_as_cells`] calls (`cell_memo` misses), for
    /// a test to assert an unchanged redraw reuses the memo instead of
    /// rescanning (SQ-1200).
    #[cfg(all(test, feature = "t-render"))]
    classify_calls: u64,
    /// Letterbox geometry recorded by the most recent v6 draw, for inverting a
    /// terminal click back to a game pixel (Lane M mouse input). `None` until a
    /// v6 frame has been drawn.
    pub last_v6_map: Option<V6ClickMap>,
    /// Last-ready v6 pixel composite (Phase 1c / SQ-0469), keyed on a change
    /// generation + area rather than a full-buffer pixel hash. Built off-thread.
    v6: Option<V6Ready>,
    /// The in-flight background encode, if any (SQ-0469).
    v6_job: Option<V6Job>,
    /// Per-band cache for the v6 HYBRID chrome ring (Lane H): one uploaded
    /// protocol per band cell rect, keyed on the band rect with a stored
    /// hash of ONLY that band's own native sub-rect (plus scale/offset/cell/rect)
    /// so a change confined to one band leaves the other bands' uploads fresh
    /// (SQ-0514). Pruned each frame to the live band set by
    /// [`GraphicsRender::retain_chrome_bands`].
    /// Keyed on the rect a band is DRAWN at, plus a [`BandSlot`]: one rect can carry
    /// two different images on one frame — a flank's own art and the divider extension
    /// replicated over it — and keying on the rect alone made each overwrite the
    /// other's cache entry, so both re-encoded on every frame forever (SQ-0755).
    /// The third element is the kitty image id the band was last PLACED as, read
    /// back off the placement (SQ-0753) — `None` under any non-kitty protocol, and
    /// until the band's first place. It is the only handle we have on a
    /// `ratatui-image` upload, and it is what lets an abandoned band be freed in the
    /// terminal instead of merely forgotten here.
    chrome_bands: std::collections::HashMap<BandKey, (u64, Protocol, Option<u32>)>,
    /// What happened to each chrome band on the last v6 frame, for `/dump-windows`
    /// (SQ-0587). Whether a band was a cache hit, whether it encoded, and what size
    /// the protocol reported — the questions that decide whether a missing image is a
    /// stale placement, a failed upload, or a band that was never drawn at all. The
    /// answers cannot be inferred from the geometry, which is why this exists.
    pub band_log: Vec<String>,
    /// SQ-0898: what each band of the last v6 frame was actually resampled from and
    /// to — `(band cell rect, what it claims to be, source WxH in NATIVE px,
    /// destination WxH in DEVICE px)`. The magnification is `dst / src`; the sizes
    /// are kept rather than the ratio so a reader can also ask "how far from where
    /// the frame's scale puts it does this land?", which is `|dst − src·s|` and the
    /// only form of the question that is in pixels a viewer could see.
    ///
    /// Taken from the numbers that went INTO the resample. The band log's `native`
    /// field cannot answer this: on a crop it is a hash footprint carrying the area
    /// filter's halo (see [`scale_halo`]), so it reads several pixels wider than the
    /// crop and neighbouring bands appear to overlap when they partition the canvas
    /// exactly.
    ///
    /// **One frame, one magnification.** Every piece of the game's screen — a
    /// full-width banner tile, a flank crop, a tiled flank extension — lands at the
    /// frame's letterbox scale on both axes. The extension changes WHAT is drawn
    /// (rows of art past the ones the game painted, per the per-title recipe); it
    /// must never change the magnification it is drawn at. Two pieces of one column
    /// at two magnifications is exactly the seam SQ-0894 removed, and the corner
    /// fragment SQ-0898 is about.
    ///
    /// A `/dump-windows` reader can see this in the band log, but only by dividing
    /// numbers in their head, and the two defects this exists for both shipped
    /// because nobody did. Recorded structurally so a test can assert the whole
    /// class at once.
    pub band_mags: Vec<BandMag>,
    /// How many chrome bands have been ENCODED (uploaded) since launch (SQ-0587).
    /// The band log only shows the latest frame; this shows whether an upload ever
    /// happened across an event like a restore. Dump it either side: if the number
    /// has not moved, every band was a cache hit and the terminal was sent nothing.
    pub band_encodes: u64,
    /// What every kitty upload since launch has cost the wire, and what the same
    /// pixels would have cost raw (SQ-1005). Measured off the transmits themselves,
    /// so it covers `ratatui-image`'s encoder as well as ours.
    ///
    /// Its `deletes`/`freed_pixels`/`stranded_uploads`/`stranded_pixels` (SQ-1201)
    /// are kept in sync with [`Self::outstanding`] below rather than measured off
    /// text: every id this struct ever transmits or deletes is already known as a
    /// typed value at the call site (`entry.id`, the id [`reseat_kitty_placement`]
    /// hands back, the id a `queue_*` method takes), so pairing it against
    /// `outstanding` is a `HashMap` lookup, cheaper and more exact than re-parsing
    /// the escape a second time. [`crate::render::graphics::measure_traffic`] is
    /// the byte-scanning sibling, for a caller (the pty-stream harness) that only
    /// has the wire and no such call sites to hook.
    pub uploads: UploadBytes,
    /// Kitty ids transmitted by THIS struct that no later delete (`queue_kitty_deletes`,
    /// `queue_protocol_delete`, `queue_protocol_delete_after_place`) has named yet —
    /// `id → pixel bytes` of the upload currently resident under it (SQ-1201). A
    /// re-transmit to an id already here OVERWRITES the entry rather than adding to
    /// it, matching the kitty spec's own rule that re-transmitting to an id replaces
    /// what it held; [`Self::note_upload_id`]/[`Self::note_delete_id`] are the only
    /// writers, and [`Self::sync_stranded`] mirrors its live size/sum into
    /// `uploads.stranded_uploads`/`stranded_pixels` after each change.
    ///
    /// Scoped to this struct's own traffic — the picker's `KittyDeleteQueue` caches
    /// (`cover.rs`/`picker_ui.rs`) run before any `GraphicsRender` exists and keep
    /// no ledger of their own; see [`measure_traffic`] for the whole-wire answer.
    outstanding: std::collections::HashMap<u32, u64>,
    /// The whole native chrome canvas scaled to device pixels, shared across all
    /// bands of a frame so the expensive Nearest resize runs at most ONCE per
    /// changed frame instead of once per band (SQ-0514). Keyed on the canvas
    /// content + scale + scaled dimensions; each band crops its sub-rect from it,
    /// so band output stays byte-identical to a per-band whole-canvas resize.
    chrome_scaled: Option<(u64, image::RgbaImage)>,
    /// SQ-1187: the cached hybrid chrome-ring frame — canvases, strips, band
    /// placements, viewport — replayed whole when `v6_hybrid_gen`'s key holds
    /// still. Lives here (not on `AppState`) because this is the object whose
    /// band caches the replay leans on, and the two must be invalidated
    /// together on a font-size change.
    pub(crate) hybrid: Option<crate::render::screen::HybridFrame>,
    /// How many times the hybrid ring frame has been COMPUTED since launch
    /// (SQ-1187). The gate's oracle: an unchanged frame replayed from cache
    /// leaves it still, any input change bumps it. Public for the gate's
    /// falsification suite, and cheap enough to keep forever.
    pub hybrid_builds: u64,
    /// SQ-1187: true while the current frame REPLAYS an unchanged hybrid
    /// frame. The whole-frame generation key has already proven every input to
    /// the per-band content hashes unchanged, so the three band draws reuse
    /// their stored hashes instead of re-walking canvas pixels. Reset by
    /// [`Self::begin_band_log`] so it can never leak across frames or into the
    /// raster path.
    band_replay: bool,
    /// SQ-1188: band encodes staged this frame for the background worker —
    /// content that changed while an older upload is still placed. Drained by
    /// [`Self::spawn_band_jobs`] at the end of the hybrid draw.
    band_pending: Vec<PendingBand>,
    /// The in-flight background band-encode batch, if any (SQ-1188) — one at a
    /// time, coalesced exactly like the raster worker's `v6_job`.
    band_job: Option<std::thread::JoinHandle<Vec<BandEncoded>>>,
    /// What the in-flight batch carries, `key → content hash` — so a staging
    /// dropped while the worker runs knows whether its content is already on
    /// the way (kept dirty) or superseded (un-marked, restaged next frame).
    band_inflight: std::collections::HashMap<BandKey, u64>,
    /// Bands whose cached upload LAGS the canvas: staged or in flight,
    /// `key → the content hash on its way`. A dirty band is excluded from the
    /// SQ-1187 replay fast path (its stored hash is known-stale) and skips
    /// re-staging while the same content is already queued.
    band_dirty: std::collections::HashMap<BandKey, u64>,
    /// Kitty-protocol graphics windows: one transmitted image per window,
    /// placed with an EXPLICIT r×c grid so the terminal scales the canvas to
    /// exactly the window's cell rect (SQ-0520 — see `render_kitty_virtual`).
    kitty_wins: std::collections::HashMap<u32, KittyWindowImage>,
    /// Monotonic id source for `kitty_wins` (offset into a private id range).
    next_kitty_id: u32,
    /// Kitty `a=d` delete escapes for uploads abandoned WHOLESALE — a window that
    /// closed ([`GraphicsRender::retain_live`]) or a window whose area changed, which
    /// restarts its cache (SQ-0637). Dropping the [`KittyWindowImage`] only forgets
    /// the ids on OUR side; the terminal keeps every transmitted generation (up to
    /// [`KITTY_CACHE`] per window) until it is told to free them, so a closed window
    /// or a sequence of resizes leaked terminal image memory until the terminal's own
    /// quota forced an eviction. The escapes are queued rather than written here
    /// because they must ride the SAME output batch as the rest of our kitty traffic
    /// (buffer cell symbols, not a direct stdout write that would interleave with
    /// ratatui's diff): they are flushed by the next placement, or by
    /// [`GraphicsRender::flush_kitty_deletes`] when no window places again.
    pending_deletes: String,
    /// Kitty `a=d` deletes for an upload that is still COVERING ITS RECT — a chrome
    /// band re-encoded into its own cache slot, the raster composite superseded by
    /// the next encode (SQ-0817). These cannot ride ahead of the frame's traffic the
    /// way [`Self::pending_deletes`] does: the id being freed is the one the terminal
    /// is showing at that very rect, and its replacement is up to 618 KB of image data
    /// away. Freeing it first leaves the cells with nothing to draw for the length of
    /// that transfer — which is exactly the flicker Zork Zero's compass, its map and
    /// Arthur's Merlin composite all showed, once per frame the game changed.
    ///
    /// So they ride as a SUFFIX on the placement that supersedes them, in the same
    /// batch: transmit new, place new, free old. The rect is covered throughout, and
    /// the memory is still freed on the same frame — deferring by a frame would work
    /// too, but there is no reason to hold 618 KB longer than the width of one
    /// placement.
    deletes_after_place: String,
    /// The ground this frame's chrome bands are flattened onto before they are
    /// encoded, or `None` to ship their alpha as they always have (SQ-0944).
    ///
    /// The ring's bands are the third surface in this renderer to ship alpha to a
    /// compositor, after the raster composite (`flatten_onto_page`, SQ-0510) and
    /// the inline story floats (`inline_image::flatten_onto`, SQ-0704), and both
    /// of those record the same rule: whoever composites must not be left to pick
    /// the colour for us. Half-blocks picks BLACK, so Zork Zero's pillars arrived
    /// with a black gutter down either side where kitty shows the story page.
    ///
    /// A frame property rather than an argument on all three band entry points:
    /// the answer cannot differ between two bands of one frame, and reset by
    /// [`Self::begin_band_log`] so it can never outlive the frame that set it. It
    /// rides each band's hash, so a frame that changes the ground re-encodes
    /// rather than placing a band flattened onto the old one.
    band_ground: Option<image::Rgba<u8>>,
    /// Machine-readable record of the protocol traffic this frame produced
    /// (SQ-0590) — see [`GraphicsOp`]. `band_log` above is the human dump for
    /// `/dump-windows`; this is the same events in a form a test can assert on,
    /// which is what lets the pixel path be exercised headlessly at all.
    ops: Vec<GraphicsOp>,
}

/// One unit of graphics-protocol traffic (SQ-0590).
///
/// The pixel path's failure modes are all about WHICH of these happened: an image
/// that vanished is a stale [`Place`](GraphicsOp::Place) with no re-[`Upload`](GraphicsOp::Upload),
/// a slow frame is an `Upload` that should have been a [`Reuse`](GraphicsOp::Reuse),
/// and a leak is an `Upload` with no matching [`Drop`](GraphicsOp::Drop). None of
/// that is visible in the rendered `Buffer` — under the kitty protocol the cells
/// carry only placeholder glyphs — so it has to be recorded as it happens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphicsOp {
    /// Pixels were encoded and handed to the terminal.
    Upload { target: GraphicsTarget, id: Option<u32>, cells: (u16, u16) },
    /// The terminal already had these exact pixels; an existing upload was re-placed
    /// instead of re-sent (the kitty per-window cache, or a fresh band).
    Reuse { target: GraphicsTarget, id: Option<u32> },
    /// An image was placed on screen at a cell rect `(x, y, w, h)`.
    Place { target: GraphicsTarget, at: (u16, u16, u16, u16) },
    /// An upload was released — a kitty `a=d` delete, or a cached band dropped.
    /// The terminal frees the image AND its placements, so anything dropped must
    /// be re-uploaded before it can be seen again (SQ-0587).
    Drop { target: GraphicsTarget },
}

/// What a [`GraphicsOp`] acted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GraphicsTarget {
    /// A v6 graphics window, by window number.
    Window(u32),
    /// One chrome-ring band, by its cell rect `(x, y, w, h)`.
    Band(u16, u16, u16, u16),
    /// The v6 RASTER path's full-frame composite (SQ-0747). There is only ever one,
    /// and it covers the whole pane — so a frame that stops drawing it without
    /// dropping it strands an image over everything the next path draws. Journey
    /// boots through this path (`raster x2 · hybrid-ring x27`) before its menu
    /// switches to the ring, and the transition was invisible to the op log until
    /// this target existed.
    Raster,
}

impl GraphicsOp {
    pub fn target(&self) -> GraphicsTarget {
        match *self {
            GraphicsOp::Upload { target, .. }
            | GraphicsOp::Reuse { target, .. }
            | GraphicsOp::Place { target, .. }
            | GraphicsOp::Drop { target } => target,
        }
    }
    pub fn is_upload(&self) -> bool {
        matches!(self, GraphicsOp::Upload { .. })
    }
    pub fn is_place(&self) -> bool {
        matches!(self, GraphicsOp::Place { .. })
    }
}

/// Cap on the retained op log. A v6 frame records a few dozen ops and clears at
/// the top of the next one ([`GraphicsRender::begin_band_log`]); this only bounds
/// the non-v6 paths, which have no frame boundary to clear on.
const OPS_MAX: usize = 512;

/// Build a [`Picker`] that speaks the kitty graphics protocol at a fixed cell
/// size, without querying a terminal (SQ-0590).
///
/// Test harnesses otherwise use `Picker::halfblocks()`, which reports 10×20 and
/// draws cells — so every band cache, upload, eviction and placement in this file
/// was unreachable from the suite, and the whole class of v6 graphics regressions
/// could only be found by playing. Pass the cell size the case needs (14×28 is a
/// typical kitty terminal); the pixel path is sensitive to it, since art scales by
/// pixel while text places by cell.
///
/// `from_fontsize` is deprecated upstream in favour of `from_query_stdio`, which
/// needs a real terminal — exactly what a headless test does not have.
pub fn kitty_picker(cell_w: u16, cell_h: u16) -> Picker {
    #[allow(deprecated)]
    let mut picker = Picker::from_fontsize(ratatui_image::FontSize::new(cell_w, cell_h));
    picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
    picker
}

/// The placement id every graphics-window transmit names (SQ-0995).
///
/// Placement ids are scoped to their image, so one constant serves every window.
/// It has to be stated rather than left at the protocol's default of 0, because
/// `p=0` means "assign me an internal id" and this path now re-transmits to the
/// SAME image id on every content change: an unnamed placement would be a fresh
/// internal placement each time, piling up unreachable duplicates for the life of
/// the window. Naming it makes each re-transmit REPLACE the one placement the
/// window owns. The placeholder cells still encode placement 0 — "any virtual
/// placement of this image" — which resolves to it precisely because it is the
/// only one.
const KITTY_PLACEMENT: u32 = 1;

/// The kitty image backing one graphics window (SQ-0520/SQ-0995).
struct KittyWindowImage {
    /// Canvas version the upload was last reconciled against. Hashing a canvas
    /// costs a pass over every pixel, so it only happens when the game actually
    /// drew something — not on every frame.
    version: u64,
    w: u16,
    h: u16,
    /// The id this window's pixels live under in the terminal, allocated once
    /// when the entry is created and STABLE for as long as the window keeps this
    /// cell size (SQ-0995).
    ///
    /// The id is a PER-CELL value — `kitty_place_rows` writes its low 24 bits into
    /// every placeholder cell's foreground and its high byte into the third
    /// diacritic — so changing it dirties the whole grid. Keying an id on the
    /// canvas's CONTENT, as this did before, therefore made one changed pixel
    /// repaint every cell of the window: measured on golden_baton.blb at 230×64,
    /// a 228×16 window cost 42,207 bytes for a frame whose compressed image was
    /// 2,208. Holding the id still and replacing the data behind it costs the
    /// image and nothing else.
    id: u32,
    /// The transmit escape, prepended to the first placeholder row of the next
    /// emitted frame, then dropped — the terminal keeps the image by id after that.
    pending_transmit: Option<String>,
    /// Hash of the pixels currently uploaded under [`Self::id`], or `None` while
    /// nothing has been transmitted yet. A repaint that lands on identical pixels
    /// (advent.blb's toolbar redraws itself from scratch to release a button)
    /// re-places the id and sends nothing; `None` also tells
    /// [`GraphicsRender::queue_kitty_deletes`] there is nothing in the terminal to
    /// free.
    uploaded: Option<u64>,
}

impl std::fmt::Debug for GraphicsRender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphicsRender").field("cached", &self.cache.len()).finish()
    }
}

impl GraphicsRender {
    /// Memoized [`render_graphics_as_cells`] (SQ-1200): reuses the last
    /// classification for this window when `(gw.version, area, force)` are
    /// unchanged, instead of rescanning `gw.canvas`. See [`Self::cell_memo`]'s
    /// docs for the keying and invalidation this mirrors.
    pub fn render_as_cells(&mut self, gw: &GraphicsWindow, area: Rect, buf: &mut Buffer, force: bool) -> bool {
        let fresh = matches!(self.cell_memo.get(&gw.win),
            Some((v, a, f, _)) if *v == gw.version && *a == area && *f == force);
        if !fresh {
            let plan = classify_graphics_as_cells(gw, area, force);
            self.cell_memo.insert(gw.win, (gw.version, area, force, plan));
            #[cfg(all(test, feature = "t-render"))]
            {
                self.classify_calls += 1;
            }
        }
        let (.., plan) = self.cell_memo.get(&gw.win).expect("just inserted above, or already fresh");
        paint_cell_plan(plan, area, buf)
    }

    pub fn render(&mut self, picker: &Picker, gw: &GraphicsWindow, area: Rect, letterbox: Style, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        // Letterbox fill behind the fitted canvas.
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(c) = buf.cell_mut((x, y)) {
                    c.set_symbol(" ").set_style(letterbox);
                }
            }
        }
        // Kitty terminals: place the canvas ourselves with an explicit r×c grid,
        // so the terminal scales it to exactly the window's cell rect. The
        // ratatui-image placement omits r/c, leaving the on-screen extent up to
        // the terminal's own cell-pixel accounting — on displays where images
        // render at device pixels but cell metrics report logical points
        // (Ghostty/macOS 2×), the image covered only part of the window and
        // mouse clicks mapped to the wrong game pixels. (SQ-0520)
        if picker.protocol_type() == ratatui_image::picker::ProtocolType::Kitty {
            self.render_kitty_virtual(gw, area, kitty_compression(picker), buf);
            return;
        }
        let fresh = matches!(self.cache.get(&gw.win),
            Some((v, w, h, _)) if *v == gw.version && *w == area.width && *h == area.height);
        if !fresh {
            let img = image::DynamicImage::ImageRgba8((*gw.canvas).clone());
            // `upscale` blows a small canvas up to fill the window (aspect
            // preserved → crisp pixel art); otherwise the canvas stays at native
            // size, centered. Scott room pictures want the former.
            //
            // Only the NON-kitty backends reach here — kitty placed the canvas
            // itself and returned, above — so this is the sixel/iTerm2/halfblocks
            // resample, and it went through the crate's default Nearest in BOTH
            // directions. A canvas larger than its window (advent.blb's 1104-px
            // toolbar in a narrow pane) was minified by dropping columns (SQ-0829).
            // Half-blocks then does it ONCE, onto its own sample grid (SQ-0979).
            match fitted_protocol(picker, &img, Size::new(area.width, area.height), gw.upscale) {
                Some(p) => { self.cache.insert(gw.win, (gw.version, area.width, area.height, p)); }
                None => return,
            }
        }
        if let Some((_, _, _, proto)) = self.cache.get(&gw.win) {
            let sz = proto.size();
            let w = sz.width.min(area.width);
            let h = sz.height.min(area.height);
            let dest = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h);
            place_protocol(proto, dest, buf);
        }
    }

    /// Drop cache entries for windows no longer live (evicts on close; bounds growth).
    ///
    /// A dropped kitty entry's uploads are DELETED in the terminal, not merely
    /// forgotten (SQ-0637) — see [`GraphicsRender::queue_kitty_deletes`].
    pub fn retain_live(&mut self, live: &std::collections::HashSet<u32>) {
        self.cache.retain(|win, _| live.contains(win));
        self.cell_memo.retain(|win, _| live.contains(win));
        let dead: Vec<u32> = self.kitty_wins.keys().copied().filter(|w| !live.contains(w)).collect();
        for win in dead {
            if let Some(entry) = self.kitty_wins.remove(&win) {
                self.queue_kitty_deletes(&entry, win);
            }
        }
    }

    /// How many times [`Self::render_as_cells`] has recomputed a classification
    /// (a `cell_memo` miss), for a test to assert an unchanged redraw hits the
    /// memo instead (SQ-1200).
    #[cfg(all(test, feature = "t-render"))]
    pub(crate) fn classify_calls(&self) -> u64 {
        self.classify_calls
    }

    /// Record a transmit of `pixels` bytes under `id` in [`Self::outstanding`]
    /// (SQ-1201) and mirror the map's live size/sum into `uploads.stranded_*`.
    /// A no-op for `pixels == 0` — a re-place of already-uploaded content, which
    /// leaves whatever the id already holds (or does not) exactly as it was.
    fn note_upload_id(&mut self, id: Option<u32>, pixels: u64) {
        if let Some(id) = id {
            if pixels > 0 {
                self.outstanding.insert(id, pixels);
            }
        }
        self.sync_stranded();
    }

    /// Record an `a=d` delete for `id` in [`Self::outstanding`] (SQ-1201):
    /// `uploads.deletes` counts the command regardless, and `uploads.freed_pixels`
    /// is credited only when `id` was still outstanding — a delete for an id this
    /// struct never transmitted, or already freed, is counted but not credited.
    fn note_delete_id(&mut self, id: Option<u32>) {
        if let Some(id) = id {
            self.uploads.deletes += 1;
            if let Some(px) = self.outstanding.remove(&id) {
                self.uploads.freed_pixels += px;
            }
        }
        self.sync_stranded();
    }

    /// Mirror [`Self::outstanding`]'s current size/sum into `uploads`. A snapshot,
    /// not a running total — called after every [`Self::note_upload_id`]/
    /// [`Self::note_delete_id`] so `stranded_uploads`/`stranded_pixels` always read
    /// "as of now" rather than something `UploadBytes::add` accumulated.
    fn sync_stranded(&mut self) {
        self.uploads.stranded_uploads = self.outstanding.len() as u64;
        self.uploads.stranded_pixels = self.outstanding.values().sum();
    }

    /// Queue an `a=d,d=I` delete for the id an abandoned [`KittyWindowImage`] still
    /// holds in the terminal, and record a [`GraphicsOp::Drop`] for it (SQ-0637).
    /// `d=I` frees the image data AND its placements, so nothing deleted here can be
    /// re-placed — which is correct: the entry that knew this id is gone.
    ///
    /// One id, not a cache generation: since SQ-0995 a window owns exactly one
    /// upload at a given cell size and replaces the data behind it in place. An
    /// entry that never transmitted (`uploaded` is `None`) names nothing the
    /// terminal is holding, so it queues nothing.
    fn queue_kitty_deletes(&mut self, entry: &KittyWindowImage, win: u32) {
        use std::fmt::Write as _;
        if entry.uploaded.is_none() {
            return;
        }
        let id = entry.id;
        write!(self.pending_deletes, "\x1b_Gq=2,a=d,d=I,i={id}\x1b\\").expect("write to String");
        self.note_delete_id(Some(id));
        self.note_op(GraphicsOp::Drop { target: GraphicsTarget::Window(win) });
    }

    /// Queue an `a=d,d=I` delete for ONE abandoned `ratatui-image` upload (SQ-0753).
    ///
    /// Everything drawn through a [`Protocol`] — every chrome-ring band, the v6
    /// raster composite — was uploaded by `ratatui-image`, which never deletes:
    /// dropping the struct forgets the id on our side and leaves the pixels in the
    /// terminal for ever. Only the graphics WINDOWS, whose ids this file allocates
    /// itself, were ever freed (SQ-0637). Measured on Journey release 30 over five
    /// keystrokes: 4.1 MB uploaded, 0 bytes freed, because a band that re-encodes
    /// strands its predecessor and the boot raster frame is abandoned wholesale when
    /// the ring takes over. Kitty evicts by LRU and evicts images that are CURRENTLY
    /// PLACED, so an unbounded pile of orphans can blank a live one.
    ///
    /// `None` (a non-kitty protocol, or a cache entry never placed) is a no-op: there
    /// is nothing the terminal is holding that we could name. `d=I` frees the data
    /// and its placements together, which is right — the entry that knew this id is
    /// gone, so nothing can re-place it.
    fn queue_protocol_delete(&mut self, id: Option<u32>) {
        use std::fmt::Write as _;
        if let Some(id) = id {
            write!(self.pending_deletes, "\x1b_Gq=2,a=d,d=I,i={id}\x1b\\").expect("write to String");
        }
        self.note_delete_id(id);
    }

    /// Queue `a=d,d=I` deletes for uploads a SIBLING cache owns (SQ-1190): the
    /// inline-image transcript bands and the picker's cover/gallery tiles are
    /// placed through [`place_protocol`], exactly like a chrome band, but they
    /// live in `InlineImageRender`/`cover.rs`, not here — `queue_protocol_delete`
    /// is private, so this is the seam that lets their eviction share this queue
    /// and [`Self::flush_kitty_deletes`]'s no-flicker sequencing instead of
    /// duplicating either.
    pub(crate) fn queue_external_deletes(&mut self, ids: impl IntoIterator<Item = u32>) {
        for id in ids {
            self.queue_protocol_delete(Some(id));
        }
    }

    /// Queue a delete for an upload that is being REPLACED IN PLACE — one still
    /// covering the rect its successor is about to take (SQ-0817).
    ///
    /// [`Self::queue_protocol_delete`] is right for an upload nothing on screen
    /// depends on: a closed window, a band evicted from the ring, the composite
    /// abandoned at the raster→ring transition. Freeing those early is free. This one
    /// is the opposite case — the terminal is drawing these pixels right now — so the
    /// escape goes to [`Self::deletes_after_place`] and rides out behind the
    /// placement that covers the rect again.
    fn queue_protocol_delete_after_place(&mut self, id: Option<u32>) {
        use std::fmt::Write as _;
        if let Some(id) = id {
            write!(self.deletes_after_place, "\x1b_Gq=2,a=d,d=I,i={id}\x1b\\")
                .expect("write to String");
        }
        self.note_delete_id(id);
    }

    /// Flush any queued kitty deletes into `buf` when no graphics window will place
    /// this frame (SQ-0637) — closing the LAST graphics window is exactly the case
    /// that leaks, and it leaves no placement to piggyback on.
    ///
    /// The escapes are prepended to a cell's existing symbol (keeping its glyph, with
    /// the width forced to 1 so the escape's own "width" never shifts the row) — the
    /// same trick `kitty_place_rows` uses for a transmit, so the bytes ride the normal
    /// ratatui diff instead of a racing stdout write. Only a plain single-column ASCII
    /// cell is used: one already carrying an escape (a kitty placeholder row), one
    /// marked `Skip`, or a wide glyph belongs to something whose width/placement must
    /// not be disturbed. If no such cell exists the deletes stay queued for a later
    /// frame — a delete is never dropped, only deferred.
    pub fn flush_kitty_deletes(&mut self, area: Rect, buf: &mut Buffer) {
        use ratatui::buffer::CellDiffOption;
        if self.pending_deletes.is_empty() || area.width == 0 || area.height == 0 {
            return;
        }
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                let Some(cell) = buf.cell_mut((x, y)) else { continue };
                let plain = cell.diff_option == CellDiffOption::None
                    && cell.symbol().len() == 1
                    && cell.symbol().is_ascii()
                    && !cell.symbol().starts_with(char::is_control);
                if !plain {
                    continue;
                }
                let symbol = format!("{}{}", self.pending_deletes, cell.symbol());
                cell.set_symbol(&symbol)
                    .set_diff_option(CellDiffOption::ForcedWidth(std::num::NonZeroU16::new(1).unwrap()));
                self.pending_deletes.clear();
                return;
            }
        }
    }

    /// Render a graphics window's canvas through kitty unicode placeholders
    /// with an EXPLICIT `r×c` grid on the virtual placement (SQ-0520): the
    /// terminal scales the image to exactly the placeholder rect, so the
    /// visible image always fills the window's cells — independent of any
    /// logical-vs-device pixel mismatch — and the cell→game-pixel mouse
    /// mapping in `glk_mouse_target` stays truthful.
    ///
    /// THE ID IS STABLE AND THE DATA BEHIND IT IS REPLACED (SQ-0995). A window
    /// takes one image id when its entry is created and keeps it for as long as
    /// the window keeps that cell size; a changed canvas re-transmits to the SAME
    /// id. That is the whole reason the placeholder design pays off: the id lives
    /// in every cell (low 24 bits in the foreground, high byte in the third
    /// diacritic), so an id that changes dirties every cell of the window and
    /// ratatui's diff emits all of them. Allocating a fresh id per canvas — which
    /// this did until SQ-0995 — meant one changed pixel repainted the whole grid,
    /// and it did so on a CACHE HIT too, since re-placing a previously uploaded
    /// canvas also swaps the id back. Measured on golden_baton.blb at 230×64
    /// cells, a 228×16-cell room picture: 42,207 bytes for a changed frame whose
    /// compressed image was 2,208, and 39,859 bytes for a frame that transmitted
    /// nothing at all. With the id held still, both collapse to the image.
    ///
    /// The protocol licenses it: *"When re-transmitting image data for a specific
    /// id, the existing image and all its placements must be deleted"* — the data
    /// is replaced wholesale, and our `a=T,U=1,r,c,p=1` re-creates the window's one
    /// placement in the same command, so the cells never stop resolving. The old
    /// image also stays on screen throughout, because a chunked transmit commits
    /// only on its final chunk: nothing blanks mid-transfer.
    ///
    /// A repaint that lands on identical pixels still costs nothing — advent.blb's
    /// toolbar redraws itself from scratch to press and release a button, bumping
    /// the version each time, and the release hashes equal to what is already
    /// uploaded (SQ-0564's insight, kept; SQ-0564's LRU of alternative uploads is
    /// gone, because an id you might place is an id in the cells).
    fn render_kitty_virtual(
        &mut self,
        gw: &GraphicsWindow,
        area: Rect,
        compress: bool,
        buf: &mut Buffer,
    ) {
        // A resize invalidates this window's upload: the placement's r×c grid is
        // baked into the transmission, so what the terminal holds cannot be
        // re-placed at the new size. Start the window over — and DELETE the upload
        // being abandoned, or every resize would strand a generation in the
        // terminal (SQ-0637).
        let previous = self.kitty_wins.remove(&gw.win);
        let mut entry = match previous {
            Some(e) if e.w == area.width && e.h == area.height => e,
            stale => {
                if let Some(stale) = stale {
                    self.queue_kitty_deletes(&stale, gw.win);
                }
                // Private id range, disjoint from ratatui-image's random ids in
                // practice. The top byte stays 0, which is what keeps every id this
                // path allocates nameable by `kitty_place_rows` — it now encodes the
                // byte rather than assuming it (SQ-0772), but the diacritic table
                // tops out at 296 and an id above that could not be placed at all.
                self.next_kitty_id = self.next_kitty_id.wrapping_add(1) & 0x000F_FFFF;
                KittyWindowImage {
                    version: 0,
                    w: area.width,
                    h: area.height,
                    id: 0x00B0_0000 | self.next_kitty_id,
                    pending_transmit: None,
                    uploaded: None,
                }
            }
        };
        if entry.uploaded.is_none() || entry.version != gw.version {
            entry.version = gw.version;
            let hash = canvas_hash(&gw.canvas);
            if entry.uploaded == Some(hash) {
                // The terminal already holds these exact pixels under this id:
                // re-place it and transmit nothing.
                self.note_op(GraphicsOp::Reuse {
                    target: GraphicsTarget::Window(gw.win),
                    id: Some(entry.id),
                });
            } else {
                let transmit =
                    kitty_transmit_virtual(&gw.canvas, entry.id, area.height, area.width, compress);
                let measured = measure_transmit(&transmit);
                self.note_upload_id(Some(entry.id), measured.pixels);
                self.uploads.add(measured);
                entry.pending_transmit = Some(transmit);
                entry.uploaded = Some(hash);
                self.note_op(GraphicsOp::Upload {
                    target: GraphicsTarget::Window(gw.win),
                    id: Some(entry.id),
                    cells: (area.width, area.height),
                });
            }
        }
        // Queued deletes (a closed window, or this window's pre-resize generation)
        // ride ahead of this frame's transmit, in the same batch (SQ-0637). They only
        // ever name ids no entry can place any more, so freeing them before the
        // placement below cannot blank anything still on screen.
        let mut batch = std::mem::take(&mut self.pending_deletes);
        batch.push_str(entry.pending_transmit.take().as_deref().unwrap_or(""));
        let transmit = (!batch.is_empty()).then_some(batch);
        kitty_place_rows(entry.id, transmit.as_deref(), area, buf);
        self.note_op(GraphicsOp::Place {
            target: GraphicsTarget::Window(gw.win),
            at: (area.x, area.y, area.width, area.height),
        });
        self.kitty_wins.insert(gw.win, entry);
    }

    /// How many images this window is holding in the terminal (0 before its first
    /// transmit, 1 thereafter) and the id they live under (observability hook,
    /// SQ-0564/SQ-0995). The count is the thing worth asserting: an id that is
    /// replaced in place can never grow it.
    #[cfg(all(test, feature = "t-render"))]
    fn kitty_uploads(&self, win: u32) -> Option<(usize, u32)> {
        self.kitty_wins.get(&win).map(|e| (usize::from(e.uploaded.is_some()), e.id))
    }

    /// The kitty delete escapes waiting to be written to the terminal (SQ-0637).
    #[cfg(all(test, feature = "t-render"))]
    fn queued_deletes(&self) -> &str {
        &self.pending_deletes
    }

    /// The deletes waiting to ride out BEHIND the placement that supersedes them
    /// (SQ-0817). Separate from [`Self::queued_deletes`] because "nothing was
    /// freed" is only true when both are empty (SQ-0996).
    #[cfg(all(test, feature = "t-render"))]
    fn queued_deletes_after_place(&self) -> &str {
        &self.deletes_after_place
    }

    /// What each cache keyed on cell geometry is currently holding, for SQ-0988:
    /// `(non-kitty window protocols, chrome bands, kitty window uploads,
    /// a raster composite?)`. One accessor rather than four because the whole
    /// question is which of them survive an invalidation and which must not.
    #[cfg(all(test, feature = "t-render"))]
    fn cell_keyed_cache_sizes(&self) -> (usize, usize, usize, bool) {
        (self.cache.len(), self.chrome_bands.len(), self.kitty_wins.len(), self.v6.is_some())
    }

    /// Resize + encode a native v6 canvas into a terminal image protocol,
    /// upscaled (Nearest → crisp pixel art) to fill `area`'s device pixels with
    /// aspect preserved, capped at whatever this backend's [`v6_upscale_cap`] is.
    /// Pure/self-contained so it can run on a worker thread (SQ-0469). Returns
    /// `None` if the protocol encode fails.
    ///
    /// The backend is threaded no further than this: the `picker` is already the
    /// thing that knows it, and is already here (SQ-0964).
    ///
    /// Half-blocks takes its own arm (SQ-0973) and lands on the same cell rect by a
    /// single resample — see [`v6_halfblocks_protocol`] for why the pre-scale every
    /// other backend needs is pure waste there.
    ///
    /// `reuse` is the kitty image id the composite currently on screen lives under,
    /// and the new encode goes out under it (SQ-0996). The composite covers the
    /// whole pane — 3,680 cells at 117x64 — and the id is written into every one of
    /// them, so an encode under a fresh id repaints the pane in cells on top of
    /// sending the picture. Measured on Journey r83 in raster mode, one changed
    /// frame: 48,742 bytes for a 7,668-byte image. `None` on the first encode after
    /// boot or after [`Self::invalidate_v6`] (nothing to reuse — and the delete that
    /// abandonment queued means there had better not be), and under every backend
    /// with no addressable id, where `placed_id` is never anything else.
    fn encode_v6(
        picker: &Picker,
        canvas: &image::RgbaImage,
        gen: u64,
        area: Rect,
        frame: RasterFrame,
        reuse: Option<u32>,
    ) -> Option<V6Ready> {
        let lock = frame.lock;
        let fs = picker.font_size();
        let box_w = area.width as u32 * fs.width.max(1) as u32;
        let box_h = area.height as u32 * fs.height.max(1) as u32;
        let (proto, pic) = if picker.protocol_type() == ratatui_image::picker::ProtocolType::Halfblocks {
            // Half-blocks never had the stretch to fix: `Halfblocks::encode` resolves
            // the image onto the cell grid itself, so there is no pixel size for a
            // terminal to disagree with. Its own arm builds the grid exactly, and its
            // picture therefore IS the whole cell rect.
            let proto = v6_halfblocks_protocol(canvas, box_w, box_h, fs, lock)?;
            let sz = proto.size();
            let box_px = (
                0,
                0,
                u32::from(sz.width) * u32::from(fs.width.max(1)),
                u32::from(sz.height) * u32::from(fs.height.max(1)),
            );
            (proto, box_px)
        } else {
            let (img, fit) = v6_fit_source(canvas, box_w, box_h, lock, v6_upscale_cap(picker));
            // Out to whole cells, so the terminal blits the composite 1:1 instead of
            // resampling it into a box up to a cell bigger than itself (SQ-1081).
            let (img, pic) = v6_pad_to_cells(img, box_w, box_h, fs);
            let img = image::DynamicImage::ImageRgba8(img);
            let size = Size::new(area.width, area.height);
            let proto = match reuse {
                Some(id) => picker.new_protocol_with_id(img, size, fit, id).ok()?,
                None => picker.new_protocol(img, size, fit).ok()?,
            };
            (proto, pic)
        };
        Some(V6Ready {
            pic,
            gen,
            area_w: area.width,
            area_h: area.height,
            proto,
            canvas: (canvas.width() as u16, canvas.height() as u16),
            screen: frame.native,
            // The id this composite is already placed under, carried across the
            // re-encode: `redraw_v6` re-confirms it off the placement it writes.
            placed_id: reuse,
        })
    }

    /// Whether the caller should (re)build the native v6 canvas and spawn an
    /// encode for `(gen, area)` this frame (SQ-0469). False when the last-ready
    /// encode already matches (nothing changed) OR a background encode is already
    /// in flight (coalesced — one at a time; the next frame after it lands
    /// respawns for the current generation). The expensive canvas BUILD is thus
    /// skipped entirely on an unchanged frame — no rebuild, no pixel hash.
    pub fn v6_wants_build(&self, gen: u64, area: Rect) -> bool {
        if area.width == 0 || area.height == 0 {
            return false;
        }
        if self.v6_job.is_some() {
            return false;
        }
        !matches!(&self.v6, Some(r) if r.gen == gen && r.area_w == area.width && r.area_h == area.height)
    }

    /// Spawn the background resize+encode for a freshly built native `canvas`
    /// (SQ-0469). Caller must have checked [`v6_wants_build`]; this replaces any
    /// (already-absent) job. The UI thread never blocks on the encode — the
    /// worker's result is installed by [`poll_v6_job`] — with one exception:
    /// when there is NO last-ready composite at all (first raster frame after
    /// boot or after [`invalidate_v6`]), the encode runs synchronously so the
    /// new screen shows this frame. Redrawing "the previous encode until the
    /// worker lands" is right during a burst of the same screen, but at a
    /// transition there is no honest previous frame — the pane would blank, or
    /// worse flash whatever the raster path last showed (SQ-0578: entering the
    /// rebus flashed the title splash or the on-screen map for a split second).
    pub fn spawn_v6_encode(
        &mut self,
        picker: &Picker,
        canvas: image::RgbaImage,
        gen: u64,
        area: Rect,
        // SQ-1032: the frame, not the bare lock. It carries the canvas height, the
        // magnification that height was derived from, and the GAME's own screen —
        // one subject, so a caller cannot supply the scale and omit the bound a
        // click has to be judged against (CLAUDE.md's refactoring policy).
        frame: RasterFrame,
    ) {
        if area.width == 0 || area.height == 0 || canvas.width() == 0 || canvas.height() == 0 {
            return;
        }
        // The id the composite on screen lives under, so the re-encode replaces its
        // pixels rather than moving to a new id and repainting the pane's cells
        // (SQ-0996). `None` when there is no composite yet, which is the same
        // branch that has to encode synchronously.
        let reuse = self.v6.as_ref().and_then(|r| r.placed_id);
        if self.v6.is_none() {
            self.v6 = Self::encode_v6(picker, &canvas, gen, area, frame, None);
            return;
        }
        let picker = picker.clone();
        self.v6_job =
            Some(std::thread::spawn(move || Self::encode_v6(&picker, &canvas, gen, area, frame, reuse)));
    }

    /// Drop the last-ready v6 raster composite (and detach any in-flight encode,
    /// discarding its result). Called by the HYBRID band path on every frame it
    /// renders WITHOUT the raster composite: once the pane has shown chrome-band
    /// frames, the cached composite is stale content from another screen, and a
    /// later fall-through to raster (a full-screen picture takeover, SQ-0570)
    /// must not flash it while the new encode is in flight (SQ-0578). The next
    /// raster frame re-encodes synchronously (see [`spawn_v6_encode`]).
    pub fn invalidate_v6(&mut self) {
        // The composite the terminal was last pointed at is being abandoned; record
        // it, so "placed on the previous frame, neither re-placed nor dropped on this
        // one" is answerable for the raster→ring transition too (SQ-0747).
        if let Some(old) = self.v6.take() {
            // …and free it in the terminal, which forgetting it here does not do
            // (SQ-0753). This is the transition Journey makes two frames into its
            // boot, and the abandoned composite is 2.8 MB.
            self.queue_protocol_delete(old.placed_id);
            self.note_op(GraphicsOp::Drop { target: GraphicsTarget::Raster });
        }
        self.v6_job = None;
    }

    /// SQ-0988: the terminal's CELL changed size. Throw away everything fitted
    /// against the old one.
    ///
    /// A resize normally changes the cell GRID, and every cache here is keyed on
    /// a rect in cells, so a resize invalidates them by not matching. A font-size
    /// change is the case that slips through: the pane can keep the same
    /// `width × height` in cells while every one of those cells becomes a
    /// different box in pixels, and a cache keyed in cells sees no change at all.
    ///
    /// What that leaves behind, cache by cache:
    ///
    /// * `cache` — the non-kitty per-window protocol, keyed `(version, cols,
    ///   rows)`. STALE: the protocol holds an image encoded for the old device
    ///   box, and nothing about the window changed to dislodge it.
    /// * `v6` — the raster composite, keyed `(gen, area_w, area_h)` in cells, so
    ///   [`Self::v6_wants_build`] answers "already have it" and the pane keeps
    ///   showing a picture resampled to the old pixel size. STALE, and the most
    ///   visible of the two.
    ///
    /// And four that are already safe — checked, not assumed, because most of
    /// them share the suspect key shape and only some of them are safe for the
    /// same reason:
    ///
    /// * `chrome_bands` — the key IS in cells, but the freshness HASH mixes in
    ///   `(bw, bh)` in device pixels, so a font change makes every band miss and
    ///   re-encode on its own.
    /// * `chrome_scaled` — keyed on the scaled dimensions, likewise device
    ///   pixels.
    /// * `kitty_wins` — keyed `(version, w, h)` in cells, exactly like `cache`,
    ///   and nonetheless immune: [`Self::render_kitty_virtual`] transmits the
    ///   canvas at its NATIVE size with an explicit `r×c` grid and lets the
    ///   terminal scale it to the cell rect (SQ-0520). It never reads
    ///   `picker.font_size()` at all, so there is nothing in that cache fitted
    ///   to a cell, and re-uploading would spend a full canvas to arrive at the
    ///   same pixels.
    /// * `cell_memo` (SQ-1200) — keyed `(version, area, force)` in cells, and
    ///   immune for the same reason as `kitty_wins`: [`classify_graphics_as_cells`]
    ///   reads only `gw.canvas`'s NATIVE pixel dimensions and `area`'s cell
    ///   count, never a device-pixel size or `picker.font_size()`, so its answer
    ///   for an unchanged `(version, area, force)` is unchanged by a font-size
    ///   change too.
    ///
    /// The two device-pixel caches are dropped anyway even though they would
    /// re-encode by themselves: they are cheap to rebuild, and a ring whose
    /// bands survived a font change while the composite behind them did not is a
    /// seam waiting to happen. `kitty_wins` and `cell_memo` are deliberately KEPT.
    ///
    /// Every drop frees its upload in the terminal rather than merely forgetting
    /// it (SQ-0753), which is why this delegates instead of clearing the maps.
    pub fn invalidate_cell_geometry(&mut self) {
        self.cache.clear();
        self.invalidate_v6();
        self.invalidate_chrome_bands();
        self.chrome_scaled = None;
        // The hybrid frame's generation key covers the font size, so a font
        // change would rebuild it anyway — dropped here too so a frame fitted
        // against the old cell can never outlive the band caches it replays
        // (SQ-1187).
        self.hybrid = None;
    }

    /// Poll the background v6 encode: if it finished, install its protocol as the
    /// new last-ready composite and return true (the caller should redraw). The
    /// result is always installed (even if the generation has since advanced) so
    /// the display keeps converging to the latest encoded frame during a burst;
    /// an out-of-date entry is still rendered until the next encode lands, so the
    /// pane never blanks. (SQ-0469)
    pub fn poll_v6_job(&mut self) -> bool {
        // SQ-1188: the chrome-band worker is polled on the same tick — one
        // caller (`AppState::poll_v6_encode_job`), two workers, either can
        // warrant the redraw.
        let bands = self.poll_band_job();
        let done = self.v6_job.as_ref().is_some_and(|j| j.is_finished());
        if !done {
            return bands;
        }
        let job = self.v6_job.take().expect("checked above");
        if let Ok(Some(ready)) = job.join() {
            // The composite being replaced is a whole-pane upload; free it in the
            // terminal rather than letting the assignment orphan it (SQ-0753). A
            // raster-mode game re-encodes on every visible change, so this is the
            // heaviest leak in the app — scopa strands one full frame per move.
            //
            // It is also the composite the terminal is showing RIGHT NOW, and the
            // replacement is a whole pane of image data away, so the delete rides
            // BEHIND the placement that covers the pane again (SQ-0817).
            //
            // …but since SQ-0996 the replacement is usually the SAME id, re-transmitted
            // — and deleting that would free the image this frame is about to place.
            // Written as the comparison rather than as "never delete" because the
            // reuse can be absent (a non-kitty backend, a composite never placed) and
            // then the old rule still applies exactly.
            let reused = ready.placed_id;
            let stale = self.v6.replace(ready);
            let stale_id = stale.and_then(|r| r.placed_id);
            if stale_id != reused {
                self.queue_protocol_delete_after_place(stale_id);
            }
        }
        true
    }

    /// True while a background v6 encode is in flight (SQ-0469).
    pub fn v6_encode_in_flight(&self) -> bool {
        self.v6_job.is_some()
    }

    /// Render the last-ready v6 composite into `area`, centred/letterboxed, and
    /// record the click map (SQ-0469). No-op (leaves the pane blank) until the
    /// first encode has landed. Never re-encodes — the heavy work happened on the
    /// worker.
    pub fn redraw_v6(&mut self, picker: &Picker, area: Rect, buf: &mut Buffer) {
        let Some(ready) = &self.v6 else { return };
        let proto = &ready.proto;
        let sz = proto.size();
        let w = sz.width.min(area.width);
        let ht = sz.height.min(area.height);
        let dest = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - ht) / 2, w, ht);
        let (canvas, screen) = (ready.canvas, ready.screen);
        let pic = ready.pic;
        // Queued deletes ride out on this placement, in the same batch (SQ-0637's
        // rule); deferred back to the queue if this backend has no row to carry them.
        // The supersede-deletes ride BEHIND it instead (SQ-0817) — see
        // [`Self::deletes_after_place`].
        let pending = std::mem::take(&mut self.pending_deletes);
        let after = std::mem::take(&mut self.deletes_after_place);
        let (placed_id, placed_bytes) = place_protocol_with(proto, dest, buf, &pending, &after);
        self.uploads.add(placed_bytes);
        self.note_upload_id(placed_id, placed_bytes.pixels);
        if placed_id.is_none() {
            self.pending_deletes = pending;
            // Nothing was placed, so nothing on screen depends on these either:
            // hand them to the ordinary queue rather than stranding them.
            self.pending_deletes.push_str(&after);
        }
        // Learn the id this composite lives under in the terminal, so whoever
        // abandons it can free it (SQ-0753).
        if let Some(r) = &mut self.v6 {
            r.placed_id = placed_id;
        }
        // Record the letterbox geometry so a click in the pane can be mapped
        // back to a game pixel (Lane M). `dest`'s cells are the box the composite is
        // PLACED over; `pic` is where the composite itself lies inside that box, which
        // is what a click has to invert through (SQ-1081) — the ceil onto the cell grid
        // leaves up to a cell of margin, and the terminal no longer stretches the
        // picture across it.
        let fs = picker.font_size();
        let (cw, ch) = (fs.width.max(1), fs.height.max(1));
        self.note_op(GraphicsOp::Place {
            target: GraphicsTarget::Raster,
            at: (dest.x, dest.y, dest.width, dest.height),
        });
        self.last_v6_map = Some(V6ClickMap {
            pane_x: area.x,
            pane_y: area.y,
            cell_w: cw,
            cell_h: ch,
            img_x: (dest.x - area.x) as f32 * cw as f32 + pic.0 as f32,
            img_y: (dest.y - area.y) as f32 * ch as f32 + pic.1 as f32,
            img_w: pic.2 as f32,
            img_h: pic.3 as f32,
            canvas,
            screen,
            packed_text: Vec::new(),
        });
    }

    /// Record the letterbox click map for the HYBRID draw path (Lane H), where the
    /// game image is drawn as a chrome ring around a terminal viewport rather than
    /// one canvas. `scale` is the same [`uniform_scale`](crate::render::v6_layout::uniform_scale)
    /// the chrome bands were placed through, so the recovered game pixel matches
    /// what the player sees. `pane` is the whole v6 pane's cell rect; `native` is
    /// the chrome canvas's game-pixel extent; `cell_px` is the font cell size.
    /// `packed_text` carries every region drawn as packed cells rather than through
    /// the scale — see [`PackedText`].
    pub fn record_hybrid_click_map(
        &mut self,
        pane: Rect,
        scale: &crate::render::v6_layout::Scale,
        native: (u16, u16),
        cell_px: (u16, u16),
        packed_text: Vec<PackedText>,
    ) {
        let (cw, ch) = (cell_px.0.max(1), cell_px.1.max(1));
        self.last_v6_map = Some(V6ClickMap {
            pane_x: pane.x,
            pane_y: pane.y,
            cell_w: cw,
            cell_h: ch,
            img_x: scale.off_x as f32,
            img_y: scale.off_y as f32,
            img_w: native.0 as f32 * scale.s,
            img_h: native.1 as f32 * scale.s,
            // The hybrid ring draws the game's screen and nothing below it — the
            // SQ-1032 extension is the raster composite's alone — so the canvas IS
            // the screen here and the bound in `map_click` is unreachable.
            canvas: native,
            screen: native,
            packed_text,
        });
    }

    /// Record the click map for the v6 CELL path — a terminal with no image
    /// protocol, a modal overlay over the story pane, or a painted menu takeover.
    ///
    /// That path draws no game image at all: it re-lays the v6 screen out as
    /// ordinary terminal text filling the whole pane. There is therefore no
    /// letterbox to invert — but the pane still stands for the game's screen, so
    /// a click is mapped by proportion: the pane's full cell rect covers the
    /// native `(width, height)` game-pixel canvas, and a click at a given
    /// fraction across/down the pane yields the game pixel at the same fraction.
    ///
    /// Without this, clicks on the cell path were simply dead — the raster and
    /// hybrid paths each recorded a map and this one recorded none, so
    /// `map_click` returned a stale (or missing) geometry while games that ask
    /// for mouse input (the capability bit is advertised) got nothing.
    /// (SQ-0532/A-F4; named for the frameless mode until SQ-0895 removed it, of
    /// which it was only ever one of the callers.)
    pub fn record_cell_path_click_map(&mut self, pane: Rect, native: (u16, u16), cell_px: (u16, u16)) {
        let (cw, ch) = (cell_px.0.max(1), cell_px.1.max(1));
        self.last_v6_map = Some(V6ClickMap {
            pane_x: pane.x,
            pane_y: pane.y,
            cell_w: cw,
            cell_h: ch,
            img_x: 0.0,
            img_y: 0.0,
            img_w: pane.width as f32 * cw as f32,
            img_h: pane.height as f32 * ch as f32,
            // The cell path draws no game image at all and never extends anything:
            // the pane stands for the game's screen, so canvas and screen are one.
            canvas: native,
            screen: native,
            packed_text: Vec::new(),
        });
    }

    /// Drop cached chrome-band protocols whose band rect is not in `live` — called
    /// once per hybrid frame so a resize/layout change can't leave stale band
    /// uploads accumulating.
    /// Drop every cached chrome band, so the next draw re-encodes and re-PLACES all
    /// of them (SQ-0587). The cache's job is to skip re-uploading a band whose pixels
    /// have not changed — which is exactly wrong when the terminal has lost the
    /// placement rather than the band having changed: an overlay covered the pane, the
    /// v6 pixel path stood down while it was up, and on the way back every band is a
    /// cache HIT and nothing is sent. The image is gone from the screen and the cache
    /// believes it is still there.
    /// Start a fresh band log — and op log (SQ-0590) — for this frame.
    pub fn begin_band_log(&mut self) {
        self.band_log.clear();
        self.band_mags.clear();
        self.ops.clear();
        self.band_ground = None;
        self.band_replay = false;
    }

    /// SQ-1187: declare whether this frame REPLAYS an unchanged hybrid frame —
    /// see the field. Set by the hybrid draw half right after the frame gate,
    /// never anywhere else.
    pub fn set_band_replay(&mut self, on: bool) {
        self.band_replay = on;
    }

    /// Declare the ground this frame's chrome bands resolve their transparency
    /// onto — see [`Self::band_ground`]. `None` ships the alpha.
    pub fn set_band_ground(&mut self, ground: Option<image::Rgba<u8>>) {
        self.band_ground = ground;
    }

    /// Hand a finished band image to the encoder, resolving its transparency
    /// first when this frame named a ground (SQ-0944).
    fn seal_band(&self, mut img: image::RgbaImage) -> image::DynamicImage {
        if let Some(page) = self.band_ground {
            crate::render::inline_image::flatten_onto(&mut img, page);
        }
        image::DynamicImage::ImageRgba8(img)
    }

    /// Record what magnification one band drew at (SQ-0898) — see [`Self::band_mags`].
    /// `src`/`dst` are the sizes that went into the resample, in native and device
    /// pixels; a zero on either side means there was nothing to measure.
    fn note_band_mag(&mut self, band: Rect, fit: BandFit, src: (u32, u32), dst: (u32, u32)) {
        if src.0 == 0 || src.1 == 0 {
            return;
        }
        self.band_mags.push((band, fit, src, dst));
    }

    /// Record one unit of protocol traffic (SQ-0590), bounded by [`OPS_MAX`] so a
    /// path with no frame boundary cannot grow it without limit.
    fn note_op(&mut self, op: GraphicsOp) {
        if self.ops.len() >= OPS_MAX {
            self.ops.remove(0);
        }
        self.ops.push(op);
    }

    /// The protocol traffic recorded since the last [`begin_band_log`] (SQ-0590).
    pub fn ops(&self) -> &[GraphicsOp] {
        &self.ops
    }

    /// Every target that was UPLOADED (not merely re-placed) since the last frame
    /// boundary — the question "was the terminal actually sent these pixels?".
    pub fn uploaded_targets(&self) -> std::collections::BTreeSet<GraphicsTarget> {
        self.ops.iter().filter(|o| o.is_upload()).map(|o| o.target()).collect()
    }

    /// Every target PLACED on screen since the last frame boundary — the question
    /// "what should be visible right now?", which a stale cache can answer wrongly
    /// only by omission (SQ-0587).
    pub fn placed_targets(&self) -> std::collections::BTreeSet<GraphicsTarget> {
        self.ops.iter().filter(|o| o.is_place()).map(|o| o.target()).collect()
    }

    pub fn invalidate_chrome_bands(&mut self) {
        let dropped: Vec<_> = self.chrome_bands.drain().map(|(k, (_, _, id))| (k, id)).collect();
        for ((_, x, y, w, h), id) in dropped {
            // Free the upload in the terminal, not merely the struct here (SQ-0753).
            self.queue_protocol_delete(id);
            self.note_op(GraphicsOp::Drop { target: GraphicsTarget::Band(x, y, w, h) });
        }
        // SQ-1188: staged and in-flight encodes answered for entries that no
        // longer exist — cancel them (an in-flight batch is left to finish; its
        // results find their dirty marks gone and are dropped on install).
        self.band_pending.clear();
        self.band_dirty.clear();
        self.band_inflight.clear();
    }

    pub fn retain_chrome_bands(&mut self, live: &std::collections::HashSet<BandKey>) {
        let before: Vec<_> = self.chrome_bands.iter().map(|(k, (_, _, id))| (*k, *id)).collect();
        self.chrome_bands.retain(|k, _| live.contains(k));
        // SQ-0587: dropping a cached band drops its image PROTOCOL, and a graphics
        // protocol releases its placement when it goes (kitty deletes the image).
        // A band that merely survived is a cache HIT, so it would not re-upload and
        // its placement can go with the deleted ones — Arthur loses its header art
        // for a frame whenever the ring's band set changes shape, which is what a
        // restore and its first move do. Cheap and certain: when anything was
        // evicted, evict the rest too, so every surviving band re-encodes and
        // re-places on this frame. Costs one re-encode on the rare frames where the
        // ring's shape changes, and nothing at all on the common path.
        if self.chrome_bands.len() != before.len() {
            self.chrome_bands.clear();
            // Everything that was cached is gone — the ones `retain` evicted and
            // the survivors cleared with them — so every one of them must
            // re-upload before it can be seen again, and every one of them must be
            // freed in the terminal first (SQ-0753).
            for ((_, x, y, w, h), id) in before {
                self.queue_protocol_delete(id);
                self.note_op(GraphicsOp::Drop { target: GraphicsTarget::Band(x, y, w, h) });
            }
        }
    }

    /// The kitty image id a cached chrome band is currently placed as, if any
    /// (observability hook, SQ-0753).
    #[cfg(all(test, feature = "t-render"))]
    fn chrome_band_id(&self, key: BandKey) -> Option<u32> {
        self.chrome_bands.get(&key).and_then(|(_, _, id)| *id)
    }

    /// Snapshot the current chrome-band freshness hashes, keyed by band cell rect
    /// (observability hook, SQ-0514). A band whose hash is unchanged across two
    /// frames did NOT re-encode — used to confirm that a change confined to one
    /// band leaves the other bands' uploads fresh.
    pub fn chrome_band_hashes(&self) -> std::collections::HashMap<BandKey, u64> {
        self.chrome_bands.iter().map(|(k, (h, _, _))| (*k, *h)).collect()
    }

    /// The whole native `chrome_canvas` scaled to `sw × sh` device pixels
    /// (Nearest → crisp), memoised and shared across every band of a frame
    /// (SQ-0514). Rebuilt only when the canvas content or the scaled size
    /// changes, so the heavy resize runs at most once per changed frame; each
    /// band then crops its sub-rect from the returned image. The cropped pixels
    /// are byte-identical to resizing the whole canvas per band (it IS the same
    /// resize) — the only change is that the resize is no longer repeated.
    fn scaled_chrome(&mut self, chrome_canvas: &image::RgbaImage, s: f32, sw: u32, sh: u32) -> &image::RgbaImage {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        chrome_canvas.as_raw().hash(&mut h);
        s.to_bits().hash(&mut h);
        (sw, sh).hash(&mut h);
        let key = h.finish();
        if !matches!(&self.chrome_scaled, Some((k, _)) if *k == key) {
            let scaled = resize_directional(chrome_canvas, sw, sh);
            self.chrome_scaled = Some((key, scaled));
        }
        &self.chrome_scaled.as_ref().expect("just inserted").1
    }

    /// Draw ONE chrome ring band (Lane H hybrid mode): the crop of the letterbox-
    /// scaled `chrome_canvas` lying under `band`'s device region, placed as a
    /// single image at the band's cell rect. `chrome_canvas` is the native
    /// game-pixel chrome composite; `scale` is the same [`uniform_scale`] the story
    /// viewport was mapped through, so the ring lines up pixel-exactly with the
    /// terminal story region it surrounds. `pane` is the whole v6 pane's cell rect
    /// (the band's coordinate origin). Cached per band on a content+scale hash.
    pub fn draw_chrome_band(
        &mut self,
        picker: &Picker,
        chrome_canvas: &image::RgbaImage,
        scale: &crate::render::v6_layout::Scale,
        pane: Rect,
        band: Rect,
        buf: &mut Buffer,
    ) {
        if band.width == 0 || band.height == 0 || chrome_canvas.width() == 0 || chrome_canvas.height() == 0 {
            return;
        }
        let fs = picker.font_size();
        let (cw, ch) = (fs.width.max(1) as u32, fs.height.max(1) as u32);
        // The band's device-pixel region, measured from the pane's top-left pixel.
        let rel_x0 = band.x.saturating_sub(pane.x) as u32 * cw;
        let rel_y0 = band.y.saturating_sub(pane.y) as u32 * ch;
        let bw = band.width as u32 * cw;
        let bh = band.height as u32 * ch;
        // The scaled chrome canvas occupies [off_x, off_x + native_w·s) ×
        // [off_y, off_y + native_h·s) in that same pane-relative device space.
        let (nw, nh) = (chrome_canvas.width(), chrome_canvas.height());
        let sw = ((nw as f32 * scale.s).round() as u32).max(1);
        let sh = ((nh as f32 * scale.s).round() as u32).max(1);

        // The band reads scaled-canvas pixels [sx_lo, sx_hi) × [sy_lo, sy_hi)
        // (the crop loop below only touches sx/sy in-bounds of the scaled canvas).
        let sx_lo = (rel_x0 as i64 - scale.off_x as i64).clamp(0, sw as i64);
        let sx_hi = (rel_x0 as i64 + bw as i64 - scale.off_x as i64).clamp(sx_lo, sw as i64);
        let sy_lo = (rel_y0 as i64 - scale.off_y as i64).clamp(0, sh as i64);
        let sy_hi = (rel_y0 as i64 + bh as i64 - scale.off_y as i64).clamp(sy_lo, sh as i64);

        use std::hash::{Hash, Hasher};
        // Hash ONLY the band's own native footprint (not the whole canvas): a
        // change confined to another band's native pixels then leaves this band's
        // hash — and its cached upload — untouched (SQ-0514). The footprint is the
        // native bounding box of the scaled pixels this band samples; the Nearest
        // scaled→native map is monotone, so it covers exactly the native pixels
        // that can alter the band.
        // …and the same footprint is what this band PAINTS, so the dump reports it
        // (SQ-0747): a band's cell rect says where an image lands, never which rows
        // of the game's screen were rasterized into it, and "which canvas rows is
        // this band showing?" is the question a band painting the wrong region of
        // the screen is answered by.
        let mut footprint = None;
        if sx_lo < sx_hi && sy_lo < sy_hi {
            // A minifying letterbox resamples through an area filter whose kernel is
            // as wide as the ratio, so a scaled pixel reads native pixels either side
            // of the one Nearest would have picked. The footprint carries that halo
            // (zero when magnifying, where Nearest is exact) — SQ-0824.
            let halo = scale_halo(scale.s);
            let nx0 = scaled_to_native(sx_lo as u32, nw, sw).saturating_sub(halo);
            let nx1 = (scaled_to_native(sx_hi as u32 - 1, nw, sw) + 1 + halo).min(nw);
            let ny0 = scaled_to_native(sy_lo as u32, nh, sh).saturating_sub(halo);
            let ny1 = (scaled_to_native(sy_hi as u32 - 1, nh, sh) + 1 + halo).min(nh);
            footprint = Some((nx0, ny0, nx1 - nx0, ny1 - ny0));
        }
        let key = (BandSlot::Art as u8, band.x, band.y, band.width, band.height);
        let cached_hash = self.chrome_bands.get(&key).map(|(v, _, _)| *v);
        // SQ-1187: on a replay frame the whole-frame generation key has already
        // proven every input to this hash unchanged, so the stored hash IS the
        // hash and no pixel is walked. Any frame that could have moved an input
        // rebuilt the HybridFrame, and that frame carries `band_replay = false`.
        let hash = match cached_hash.filter(|_| self.band_replay && !self.band_dirty.contains_key(&key)) {
            Some(v) => v,
            None => {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                match footprint {
                    Some((nx0, ny0, fw, fh)) => {
                        (nx0, nx0 + fw, ny0, ny0 + fh).hash(&mut h);
                        hash_canvas_rows(&mut h, chrome_canvas, nx0, nx0 + fw, ny0, ny0 + fh);
                    }
                    // Fully in the letterbox margin — no native pixels feed it.
                    None => 0u8.hash(&mut h),
                }
                self.band_ground.map(|p| p.0).hash(&mut h);
                scale.s.to_bits().hash(&mut h);
                (scale.off_x, scale.off_y).hash(&mut h);
                (cw, ch).hash(&mut h);
                (rel_x0, rel_y0, bw, bh).hash(&mut h);
                h.finish()
            }
        };
        let fresh = cached_hash == Some(hash);
        let status = if fresh {
            self.note_op(GraphicsOp::Reuse {
                target: GraphicsTarget::Band(key.1, key.2, key.3, key.4),
                id: None,
            });
            "cache HIT"
        } else if self.band_queued(key, hash) {
            // SQ-1188: exactly this content is already on its way to the worker
            // — keep placing the old upload below until the result lands.
            "encode queued (worker)"
        } else {
            // Copy the sub-rect under this band out of the frame-shared scaled
            // chrome into a band-sized image (letterbox area outside the scaled
            // chrome stays transparent). The whole-canvas resize happens at most
            // once per changed frame (cached in `chrome_scaled`), not per band.
            let band_img = {
                let scaled = self.scaled_chrome(chrome_canvas, scale.s, sw, sh);
                // SQ-1188: the in-bounds sx range is one contiguous byte run per
                // row — copy rows, not pixels. `sx = rel_x0 + bx - off_x`, so bx
                // is in-bounds on [off_x - rel_x0, off_x - rel_x0 + sw).
                let lo = (scale.off_x as i64 - rel_x0 as i64).max(0);
                let hi = ((scale.off_x as i64 - rel_x0 as i64) + sw as i64).min(bw as i64);
                let mut data = vec![0u8; bw as usize * bh as usize * 4];
                if lo < hi {
                    let len = (hi - lo) as usize * 4;
                    let sx0 = (rel_x0 as i64 + lo - scale.off_x as i64) as usize;
                    let raw = scaled.as_raw();
                    for by in 0..bh {
                        let sy = rel_y0 as i64 + by as i64 - scale.off_y as i64;
                        if sy < 0 || sy as u32 >= sh {
                            continue;
                        }
                        let src = (sy as usize * sw as usize + sx0) * 4;
                        let dst = (by as usize * bw as usize + lo as usize) * 4;
                        data[dst..dst + len].copy_from_slice(&raw[src..src + len]);
                    }
                }
                image::RgbaImage::from_raw(bw, bh, data).expect("sized above")
            };
            let img = self.seal_band(band_img);
            // Under the id this band is already placed as, when it has one — see
            // [`Self::band_encode`] (SQ-0996); off the main thread when the old
            // upload can cover for it (SQ-1188).
            let Some(status) = self.stage_band_encode(picker, img, key, band, hash) else {
                return;
            };
            status
        };
        // The band image is exactly band-sized, so it places at the band's
        // top-left (no centering — the crop is already positioned).
        //
        // Any deletes queued above (this band's own predecessor, or another band's)
        // ride out on this placement, in the same batch (SQ-0637's rule). Restored
        // to the queue when nothing was placed, or when the backend has no
        // placeholder row to carry them — a delete is deferred, never dropped.
        let pending = std::mem::take(&mut self.pending_deletes);
        // …and this band's own predecessor rides BEHIND the placement, because it is
        // still covering the rect that placement is about to take (SQ-0817).
        let after = std::mem::take(&mut self.deletes_after_place);
        let placed = self.chrome_bands.get(&key).map(|(_, proto, _)| {
            let sz = proto.size();
            let dest = Rect::new(band.x, band.y, sz.width.min(band.width), sz.height.min(band.height));
            (dest, sz, place_protocol_with(proto, dest, buf, &pending, &after))
        });
        if !matches!(placed, Some((_, _, (Some(_), _)))) {
            self.pending_deletes = pending;
            // Nothing was placed, so nothing on screen depends on the supersede
            // deletes either — hand them to the ordinary queue rather than strand them.
            self.pending_deletes.push_str(&after);
        }
        if let Some((_, _, (id, bytes))) = placed {
            self.uploads.add(bytes);
            self.note_upload_id(id, bytes.pixels);
        }
        match placed {
            Some((dest, sz, (id, _))) => {
                self.band_log.push(format!(
                    "band {}x{}@({},{}): {} · proto {}x{} · placed {}x{} at ({},{}) · native {} · {}",
                    band.width, band.height, band.x, band.y,
                    status,
                    sz.width, sz.height, dest.width, dest.height, dest.x, dest.y,
                    match footprint {
                        Some((x, y, w, h)) => format!("{w}x{h}@({x},{y})"),
                        None => "— (entirely in the letterbox margin)".to_string(),
                    },
                    resample_note(nw, nh, sw, sh),
                ));
                // The crop's magnification is the whole canvas's, on both axes, and
                // it cannot be anything else: this band copies device pixels 1:1 out
                // of the ONE frame-shared `sw x sh` scaled canvas (SQ-0514). Anything
                // outside the canvas stays transparent, so the letterbox margin is
                // margin here rather than art stretched into it. Recorded from the
                // resize's own sizes so the reading is a measurement, not a promise.
                self.note_band_mag(band, BandFit::Letterbox, (nw, nh), (sw, sh));
                self.remember_band_id(key, id);
                self.note_op(GraphicsOp::Place {
                    target: GraphicsTarget::Band(key.1, key.2, key.3, key.4),
                    at: (dest.x, dest.y, dest.width, dest.height),
                });
            }
            None => self.band_log.push(format!(
                "band {}x{}@({},{}): NO PROTOCOL (encode failed)",
                band.width, band.height, band.x, band.y
            )),
        }
    }

    /// Record the kitty image id a band was just placed as, so the upload can be
    /// freed when the entry is abandoned (SQ-0753) — and so the band's NEXT encode
    /// can go out under the same id (SQ-0996; see [`Self::band_encode`]). A no-op
    /// under a protocol with no addressable id (half-blocks, sixel).
    fn remember_band_id(&mut self, key: BandKey, id: Option<u32>) {
        if let Some((_, _, slot)) = self.chrome_bands.get_mut(&key) {
            *slot = id;
        }
    }

    /// Encode one chrome band, UNDER THE ID IT IS ALREADY PLACED AS when it has
    /// one (SQ-0996), and record the upload. `None` if the encode failed.
    ///
    /// A kitty virtual placement writes the image id into EVERY cell of its rect —
    /// low 24 bits as the foreground colour, high byte as the third diacritic — so
    /// the id is a per-cell value, and a band that re-encodes under a fresh id
    /// dirties every one of those cells. `ratatui-image` draws its ids at random
    /// (`rand::random()` per `Protocol`) and this path builds a new `Protocol` on
    /// every content change, so until now a chrome band that changed by one pixel
    /// repainted its whole placeholder rect. Measured on Journey r83 at 117x64
    /// under a pty, the 39x20-cell illustration band: 26,968 bytes for a frame
    /// whose image was 15,136 — the rest was cells.
    ///
    /// Handing the previous id back to the crate replaces the DATA behind an
    /// unchanged placement instead. The placeholder cells come out byte-identical
    /// to the last frame's except the first, which carries the transmit, so
    /// ratatui's diff emits one cell and the picture.
    ///
    /// The id comes from the placement we last WROTE, not from an allocator: it is
    /// read back off the cells (`place_protocol_with`), so it is `None` under
    /// half-blocks and sixel — which have no ids and want none — and `None` before
    /// a band's first place, where the crate's random draw is exactly right. That
    /// also means an EVICTED band gets a fresh id, which it must: eviction queues
    /// an `a=d` for the old one, and a delete riding out on another band's cell
    /// could otherwise be emitted after the re-transmit that revived the id.
    fn band_encode(
        &mut self,
        picker: &Picker,
        img: image::DynamicImage,
        key: BandKey,
        band: Rect,
        hash: u64,
        reuse: Option<u32>,
    ) -> Option<()> {
        let size = Size::new(band.width, band.height);
        let encoded = match reuse {
            Some(id) => picker.new_protocol_with_id(img, size, Resize::Fit(None), id),
            None => picker.new_protocol(img, size, Resize::Fit(None)),
        };
        let p = encoded.ok()?;
        self.band_encodes += 1;
        // The id carries forward with the new protocol, so the placement's cells
        // are the ones already on screen and `remember_band_id` re-confirms it.
        let stale = self.chrome_bands.insert(key, (hash, p, reuse));
        let stale_id = stale.and_then(|(_, _, id)| id);
        // Whatever this key held is being replaced: free it in the terminal before
        // the only record of its id is overwritten (SQ-0753) — UNLESS it is the id
        // we just re-transmitted to, which is the whole point and would delete the
        // image this frame is about to show.
        if stale_id != reuse {
            self.queue_protocol_delete_after_place(stale_id);
        }
        self.note_op(GraphicsOp::Upload {
            target: GraphicsTarget::Band(key.1, key.2, key.3, key.4),
            id: reuse,
            cells: (band.width, band.height),
        });
        Some(())
    }

    /// SQ-1188: is an encode for exactly this band content already staged or in
    /// flight? Then the caller skips rebuilding the band image and keeps
    /// placing the old upload until the worker's result lands.
    fn band_queued(&self, key: BandKey, hash: u64) -> bool {
        self.band_dirty.get(&key) == Some(&hash)
    }

    /// SQ-1188: hand a changed band to the encoder — the background worker when
    /// this backend's encode is worth a thread AND an older upload is still
    /// placed to keep showing; synchronously otherwise. The three outcomes are
    /// the band log's status words.
    ///
    /// The synchronous cases are deliberate, not leftovers:
    /// * **no cached upload** — a first appearance or a post-resume re-upload
    ///   has no honest previous image to keep placed, so deferring it would
    ///   blank the band for a frame. This is exactly `spawn_v6_encode`'s rule
    ///   for the raster composite's first frame (SQ-0578).
    /// * **half-blocks** — see [`band_encode_offthread`].
    fn stage_band_encode(
        &mut self,
        picker: &Picker,
        img: image::DynamicImage,
        key: BandKey,
        band: Rect,
        hash: u64,
    ) -> Option<&'static str> {
        let reuse = self.chrome_bands.get(&key).and_then(|(_, _, id)| *id);
        if !band_encode_offthread(picker) || !self.chrome_bands.contains_key(&key) {
            self.band_dirty.remove(&key);
            return self.band_encode(picker, img, key, band, hash, reuse).map(|()| "encoded");
        }
        self.band_dirty.insert(key, hash);
        self.band_pending.push(PendingBand { key, band, hash, img, reuse });
        Some("encode queued (worker)")
    }

    /// SQ-1188: hand this frame's staged band encodes to one background worker.
    /// Called at the end of the hybrid draw, once per frame. Coalesced exactly
    /// like the raster worker: one batch at a time, and stagings that arrive
    /// while a batch runs are dropped and re-staged by a later frame (their
    /// dirty marks are lifted so the re-stage actually happens — except where
    /// the in-flight batch already carries the same content).
    pub fn spawn_band_jobs(&mut self, picker: &Picker) {
        let pending = std::mem::take(&mut self.band_pending);
        if pending.is_empty() {
            return;
        }
        if self.band_job.is_some() {
            for p in pending {
                if self.band_inflight.get(&p.key) != Some(&p.hash) {
                    self.band_dirty.remove(&p.key);
                }
            }
            return;
        }
        self.band_inflight = pending.iter().map(|p| (p.key, p.hash)).collect();
        let picker = picker.clone();
        self.band_job = Some(std::thread::spawn(move || {
            pending
                .into_iter()
                .map(|p| {
                    let size = Size::new(p.band.width, p.band.height);
                    // The id-reuse discipline rides into the worker unchanged
                    // (SQ-0996): the encode goes out under the id the band is
                    // already placed as, so the placeholder cells stay stable.
                    let proto = match p.reuse {
                        Some(id) => picker.new_protocol_with_id(p.img, size, Resize::Fit(None), id),
                        None => picker.new_protocol(p.img, size, Resize::Fit(None)),
                    }
                    .ok();
                    BandEncoded { key: p.key, band: p.band, hash: p.hash, reuse: p.reuse, proto }
                })
                .collect()
        }));
    }

    /// SQ-1188: reap the background band batch. Installs each result whose band
    /// still expects exactly that content — a band that changed again while the
    /// worker ran, or was evicted, drops its result on the floor (its staging
    /// mark is lifted so the next frame re-stages the CURRENT content instead).
    /// Returns true whenever a batch was reaped, so the caller schedules the
    /// redraw that places the new uploads (and re-stages whatever is still
    /// stale) — the raster worker's last-ready shape.
    fn poll_band_job(&mut self) -> bool {
        let done = self.band_job.as_ref().is_some_and(|j| j.is_finished());
        if !done {
            return false;
        }
        let job = self.band_job.take().expect("checked above");
        if let Ok(results) = job.join() {
            for r in results {
                if self.band_dirty.get(&r.key) != Some(&r.hash) {
                    continue; // superseded or cancelled — a newer staging owns this band
                }
                self.band_dirty.remove(&r.key);
                let Some(proto) = r.proto else { continue }; // failed encode: retried by the next frame
                let Some(entry) = self.chrome_bands.get_mut(&r.key) else { continue }; // evicted
                let stale_id = entry.2;
                *entry = (r.hash, proto, r.reuse);
                self.band_encodes += 1;
                if stale_id != r.reuse {
                    self.queue_protocol_delete_after_place(stale_id);
                }
                self.note_op(GraphicsOp::Upload {
                    target: GraphicsTarget::Band(r.key.1, r.key.2, r.key.3, r.key.4),
                    id: r.reuse,
                    cells: (r.band.width, r.band.height),
                });
            }
        }
        self.band_inflight.clear();
        true
    }

    /// True while any band encode is staged or in flight (SQ-1188) — the test
    /// harness's settle probe, mirroring [`Self::v6_encode_in_flight`].
    pub fn band_encode_in_flight(&self) -> bool {
        self.band_job.is_some() || !self.band_pending.is_empty() || !self.band_dirty.is_empty()
    }

    /// SQ-0511: draw ONE side flank band VERTICALLY STRETCHED — sample the native
    /// `crop` sub-rect of `chrome_canvas` (x, y, w, h in game pixels) and resize it
    /// to fill the whole `band` device region (Nearest → crisp). The horizontal
    /// resize factor is the uniform letterbox scale (the caller derives `crop`'s
    /// width from `band.width · cell_w / s`), so columns keep their true width; only
    /// the vertical factor grows, elongating an ornate frame column to span the
    /// reclaimed dead space between the top-anchored chrome and a bottom-anchored
    /// band/menu (enclosed-frame Zork0/Shogun flanks; Journey's picture-column
    /// divider continuing to its menu strip).
    ///
    /// Shares the per-band [`chrome_bands`](Self) cache with [`draw_chrome_band`]
    /// (one entry per band rect), so a band drawn stretched still participates in
    /// [`retain_chrome_bands`]/[`chrome_band_hashes`]. The freshness hash covers ONLY
    /// the crop's own native pixels + its coords + the target size, so a status tick
    /// (whose pixels sit ABOVE the flank crop) leaves the flank's cached upload fresh
    /// (SQ-0514 property preserved), and the crop native rect is folded in so the
    /// stretch factor itself is part of the key.
    ///
    /// `fit` is the caller's own claim about what this band is, and it is a
    /// parameter rather than a constant because that is what SQ-0898's second round
    /// turned on: recorded here as `Fitted` for everybody, it exempted every caller
    /// of this function from the one-magnification gate, including the one that was
    /// wrong. See [`BandFit`].
    pub fn draw_chrome_band_stretched(
        &mut self,
        picker: &Picker,
        chrome_canvas: &image::RgbaImage,
        band: Rect,
        crop: (u32, u32, u32, u32),
        slot: BandSlot,
        fit: BandFit,
        buf: &mut Buffer,
    ) {
        let (cx, cy, cw_n, ch_n) = crop;
        if band.width == 0 || band.height == 0 || cw_n == 0 || ch_n == 0 {
            return;
        }
        let (canvas_w, canvas_h) = (chrome_canvas.width(), chrome_canvas.height());
        if cx >= canvas_w || cy >= canvas_h {
            return;
        }
        let fs = picker.font_size();
        let (cw, ch) = (fs.width.max(1) as u32, fs.height.max(1) as u32);
        let bw = band.width as u32 * cw;
        let bh = band.height as u32 * ch;

        use std::hash::{Hash, Hasher};
        let x1 = (cx + cw_n).min(canvas_w);
        let y1 = (cy + ch_n).min(canvas_h);
        let key = (slot as u8, band.x, band.y, band.width, band.height);
        let cached_hash = self.chrome_bands.get(&key).map(|(v, _, _)| *v);
        // SQ-1187: on a replay frame the stored hash is the hash — see
        // `draw_chrome_band`.
        let hash = match cached_hash.filter(|_| self.band_replay && !self.band_dirty.contains_key(&key)) {
            Some(v) => v,
            None => {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                // Hash ONLY the crop's native footprint (coords + pixels) plus the target
                // device size — the stretch factor is (bw,bh)/(cw_n,ch_n), so both ends are
                // covered. A change outside this native rect (e.g. the banner's Score/Moves)
                // never alters the hash, keeping the flank's cached upload fresh (SQ-0514).
                (slot as u8).hash(&mut h); // discriminator vs. draw_chrome_band keys on the same map
                (cx, cy, cw_n, ch_n).hash(&mut h);
                hash_canvas_rows(&mut h, chrome_canvas, cx, x1, cy, y1);
                self.band_ground.map(|p| p.0).hash(&mut h);
                (bw, bh).hash(&mut h);
                h.finish()
            }
        };
        let fresh = cached_hash == Some(hash);
        let status = if fresh {
            self.note_op(GraphicsOp::Reuse {
                target: GraphicsTarget::Band(key.1, key.2, key.3, key.4),
                id: None,
            });
            "cache HIT"
        } else if self.band_queued(key, hash) {
            // SQ-1188: this content is already on its way to the worker — keep
            // placing the old upload below until the result lands.
            "encode queued (worker)"
        } else {
            // Copy the native crop (clamped to the canvas) into its own image, then
            // resize it to the band's device box. Transparent native pixels stay
            // transparent, so the theme backdrop shows through gaps in the flank.
            // SQ-1188: rows, not pixels — each crop row is one contiguous byte run.
            let mut data = vec![0u8; cw_n as usize * ch_n as usize * 4];
            let cols = (x1 - cx) as usize * 4;
            let raw = chrome_canvas.as_raw();
            for oy in 0..(y1 - cy) {
                let srow = ((cy + oy) as usize * canvas_w as usize + cx as usize) * 4;
                let drow = oy as usize * cw_n as usize * 4;
                data[drow..drow + cols].copy_from_slice(&raw[srow..srow + cols]);
            }
            let src = image::RgbaImage::from_raw(cw_n, ch_n, data).expect("sized above");
            let stretched = resize_directional(&src, bw, bh);
            let img = self.seal_band(stretched);
            let Some(status) = self.stage_band_encode(picker, img, key, band, hash) else {
                return;
            };
            status
        };
        // SQ-0747: a STRETCHED band goes in the band log too. It never did, and that
        // is why every `/dump-windows` capture of Journey's menu listed the right-hand
        // flank and the bottom strip and no left flank at all — the picture column is
        // drawn by this function, at a dest rect derived from the panel rather than
        // from the strip, so the one band an investigation most wanted to see was the
        // one band the dump could not name. Two passes were spent inferring it from
        // the strip rect beside it. The crop rides along for the same reason it does
        // above: it says which rows of the game's screen this image is showing.
        // Any deletes queued above (this band's own predecessor, or another band's)
        // ride out on this placement, in the same batch (SQ-0637's rule). Restored
        // to the queue when nothing was placed, or when the backend has no
        // placeholder row to carry them — a delete is deferred, never dropped.
        let pending = std::mem::take(&mut self.pending_deletes);
        // …and this band's own predecessor rides BEHIND the placement, because it is
        // still covering the rect that placement is about to take (SQ-0817).
        let after = std::mem::take(&mut self.deletes_after_place);
        let placed = self.chrome_bands.get(&key).map(|(_, proto, _)| {
            let sz = proto.size();
            let dest = Rect::new(band.x, band.y, sz.width.min(band.width), sz.height.min(band.height));
            (dest, sz, place_protocol_with(proto, dest, buf, &pending, &after))
        });
        if !matches!(placed, Some((_, _, (Some(_), _)))) {
            self.pending_deletes = pending;
            // Nothing was placed, so nothing on screen depends on the supersede
            // deletes either — hand them to the ordinary queue rather than strand them.
            self.pending_deletes.push_str(&after);
        }
        if let Some((_, _, (id, bytes))) = placed {
            self.uploads.add(bytes);
            self.note_upload_id(id, bytes.pixels);
        }
        match placed {
            Some((dest, sz, (id, _))) => {
                self.band_log.push(format!(
                    "band {}x{}@({},{}) [{slot:?}, stretched]: {} · proto {}x{} · placed {}x{} at ({},{}) · native {cw_n}x{ch_n}@({cx},{cy}) · {}",
                    band.width, band.height, band.x, band.y,
                    status,
                    sz.width, sz.height, dest.width, dest.height, dest.x, dest.y,
                    resample_note(cw_n, ch_n, bw, bh),
                ));
                self.note_band_mag(band, fit, (cw_n, ch_n), (bw, bh));
                self.remember_band_id(key, id);
                self.note_op(GraphicsOp::Place {
                    target: GraphicsTarget::Band(key.1, key.2, key.3, key.4),
                    at: (dest.x, dest.y, dest.width, dest.height),
                });
            }
            None => self.band_log.push(format!(
                "band {}x{}@({},{}) [{slot:?}, stretched]: NO PROTOCOL (encode failed)",
                band.width, band.height, band.x, band.y
            )),
        }
    }

    /// SQ-0698: draw ONE band from a source image the CALLER composed in native
    /// pixels, rather than from a sub-rect of the chrome canvas.
    ///
    /// The side border extension ([`crate::render::v6_border`]) tiles a flank's
    /// art downward past the native screen bottom, so its source does not exist
    /// in the canvas and cannot be named as a crop.
    ///
    /// `dest` is where inside the band, in device pixels relative to the band's
    /// own top-left, `src` belongs — **the letterbox image of `src`'s native box
    /// at the frame's scale**, computed by the caller from the same mapping the
    /// plain crop uses ([`crate::render::screen`]'s `flank_native_box`). The rest
    /// of the band stays transparent, exactly as a crop leaves the letterbox
    /// margin transparent.
    ///
    /// It used to be the whole band, and that was SQ-0898's defect. The caller
    /// clipped the SOURCE to the art it has — a flank cannot read native columns
    /// left of zero, and there are none to its left when the pane is wider than
    /// the scaled screen — and then handed the clipped source to a destination
    /// nobody had clipped, so the resize made up the difference by changing the
    /// magnification. MEASURED on Arthur at a 70x19 pane (`off_x = 6`, scale
    /// 0.855): the pole's 30 native columns were resized into the band's full
    /// 32 device px — 1.067 px per native px against the canvas's 0.855, and
    /// shifted 6 px left of the crop directly above it, which is the corner
    /// fragment the user reported as "not scaled to match". At `off_x = 0` the
    /// clip is a no-op and the same code looks perfect, which is why this
    /// survived a corpus checked at one pane size.
    ///
    /// Shares the per-band cache with the other two draws (one entry per band
    /// rect + slot), so it participates in [`Self::retain_chrome_bands`]. The
    /// freshness hash is the source's own pixels plus the target size, so a band
    /// whose art did not change is not re-encoded.
    pub fn draw_chrome_band_image(
        &mut self,
        picker: &Picker,
        src: &image::RgbaImage,
        band: Rect,
        dest: BandDest,
        slot: BandSlot,
        buf: &mut Buffer,
    ) {
        if band.width == 0 || band.height == 0 || src.width() == 0 || src.height() == 0 {
            return;
        }
        let fs = picker.font_size();
        let (cw, ch) = (fs.width.max(1) as u32, fs.height.max(1) as u32);
        let bw = band.width as u32 * cw;
        let bh = band.height as u32 * ch;
        // Clamped rather than trusted: a destination outside the band would panic
        // the blit, and the caller's rounding is device-pixel arithmetic.
        let (dx, dy) = (dest.0.min(bw), dest.1.min(bh));
        let (dw, dh) = (dest.2.min(bw - dx), dest.3.min(bh - dy));
        if dw == 0 || dh == 0 {
            return;
        }

        use std::hash::{Hash, Hasher};
        let key = (slot as u8, band.x, band.y, band.width, band.height);
        let cached_hash = self.chrome_bands.get(&key).map(|(v, _, _)| *v);
        // SQ-1187: on a replay frame the stored hash is the hash — see
        // `draw_chrome_band`.
        let hash = match cached_hash.filter(|_| self.band_replay && !self.band_dirty.contains_key(&key)) {
            Some(v) => v,
            None => {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                (slot as u8).hash(&mut h);
                (src.width(), src.height()).hash(&mut h);
                src.as_raw().hash(&mut h);
                (bw, bh).hash(&mut h);
                self.band_ground.map(|p| p.0).hash(&mut h);
                (dx, dy, dw, dh).hash(&mut h);
                h.finish()
            }
        };
        let fresh = cached_hash == Some(hash);
        let status = if fresh {
            self.note_op(GraphicsOp::Reuse {
                target: GraphicsTarget::Band(key.1, key.2, key.3, key.4),
                id: None,
            });
            "cache HIT"
        } else if self.band_queued(key, hash) {
            // SQ-1188: this content is already on its way to the worker — keep
            // placing the old upload below until the result lands.
            "encode queued (worker)"
        } else {
            let scaled = resize_directional(src, dw, dh);
            // The band is `dest` and transparent everywhere else. When `dest` IS the
            // band — every pane where the letterbox leaves no margin beside this
            // flank — the copy is skipped and the bytes are what they always were.
            let scaled = if (dx, dy, dw, dh) == (0, 0, bw, bh) {
                scaled
            } else {
                let mut band_img = image::RgbaImage::new(bw, bh);
                image::imageops::replace(&mut band_img, &scaled, dx as i64, dy as i64);
                band_img
            };
            let img = self.seal_band(scaled);
            let Some(status) = self.stage_band_encode(picker, img, key, band, hash) else {
                return;
            };
            status
        };
        // Any deletes queued above (this band's own predecessor, or another band's)
        // ride out on this placement, in the same batch (SQ-0637's rule). Restored
        // to the queue when nothing was placed, or when the backend has no
        // placeholder row to carry them — a delete is deferred, never dropped.
        let pending = std::mem::take(&mut self.pending_deletes);
        // …and this band's own predecessor rides BEHIND the placement, because it is
        // still covering the rect that placement is about to take (SQ-0817).
        let after = std::mem::take(&mut self.deletes_after_place);
        let placed = self.chrome_bands.get(&key).map(|(_, proto, _)| {
            let sz = proto.size();
            let at = Rect::new(band.x, band.y, sz.width.min(band.width), sz.height.min(band.height));
            (at, sz, place_protocol_with(proto, at, buf, &pending, &after))
        });
        if !matches!(placed, Some((_, _, (Some(_), _)))) {
            self.pending_deletes = pending;
            // Nothing was placed, so nothing on screen depends on the supersede
            // deletes either — hand them to the ordinary queue rather than strand them.
            self.pending_deletes.push_str(&after);
        }
        if let Some((_, _, (id, bytes))) = placed {
            self.uploads.add(bytes);
            self.note_upload_id(id, bytes.pixels);
        }
        match placed {
            Some((placed_at, sz, (id, _))) => {
                let (blank, run, run_at) = blank_rows(src);
                self.band_log.push(format!(
                    "band {}x{}@({},{}) [{slot:?}, tiled]: {} · proto {}x{} · placed {}x{} at ({},{}) · source {}x{} native px · into {dw}x{dh}px at ({dx},{dy}) of {bw}x{bh} · blank rows {}, longest run {} at {} · {}",
                    band.width, band.height, band.x, band.y,
                    status,
                    sz.width, sz.height, placed_at.width, placed_at.height, placed_at.x, placed_at.y,
                    src.width(), src.height(),
                    blank, run, run_at,
                    resample_note(src.width(), src.height(), dw, dh),
                ));
                self.note_band_mag(band, BandFit::Letterbox, (src.width(), src.height()), (dw, dh));
                self.remember_band_id(key, id);
                self.note_op(GraphicsOp::Place {
                    target: GraphicsTarget::Band(key.1, key.2, key.3, key.4),
                    at: (placed_at.x, placed_at.y, placed_at.width, placed_at.height),
                });
            }
            None => self.band_log.push(format!(
                "band {}x{}@({},{}) [{slot:?}, tiled]: NO PROTOCOL (encode failed)",
                band.width, band.height, band.x, band.y
            )),
        }
    }
}

// ── Delete queues detached from a `GraphicsRender` instance (SQ-1190) ────────
//
// `GraphicsRender` lives on `AppState`, but two other `Protocol` caches place
// uploads through the same `place_protocol` and have no `AppState` to share it
// with: the pre-game picker's cover/tile/preview caches (`cover.rs`,
// `picker_ui.rs`) run their own event loop before any `AppState` exists. The
// two pieces below are `GraphicsRender::queue_protocol_delete` and
// `flush_kitty_deletes`, detached from `self` so a queue with no instance in
// common can still emit the identical bytes and no-flicker sequencing instead
// of duplicating either by hand.

/// The literal escape that frees one kitty image id — `a=d,d=I` deletes the
/// image data and every placement of it at once, exactly what
/// `GraphicsRender::queue_protocol_delete` writes.
pub(crate) fn kitty_delete_escape(id: u32) -> String {
    format!("\x1b_Gq=2,a=d,d=I,i={id}\x1b\\")
}

/// [`GraphicsRender::flush_kitty_deletes`], operating on a caller-owned `pending`
/// string instead of `self.pending_deletes`. Kept in sync with that method by
/// hand — the two have no instance to share, so this is the alternative to
/// duplicating the caller-facing queue itself.
fn flush_kitty_deletes_into(pending: &mut String, area: Rect, buf: &mut Buffer) {
    use ratatui::buffer::CellDiffOption;
    if pending.is_empty() || area.width == 0 || area.height == 0 {
        return;
    }
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let Some(cell) = buf.cell_mut((x, y)) else { continue };
            let plain = cell.diff_option == CellDiffOption::None
                && cell.symbol().len() == 1
                && cell.symbol().is_ascii()
                && !cell.symbol().starts_with(char::is_control);
            if !plain {
                continue;
            }
            let symbol = format!("{}{}", pending, cell.symbol());
            cell.set_symbol(&symbol)
                .set_diff_option(CellDiffOption::ForcedWidth(std::num::NonZeroU16::new(1).unwrap()));
            pending.clear();
            return;
        }
    }
}

/// A delete queue for a `Protocol` cache with no `GraphicsRender` to share
/// (SQ-1190) — see the section header above. Same two-step shape as
/// `GraphicsRender`'s own queue: [`Self::queue`] an id an eviction/replacement
/// abandoned, [`Self::flush`] it into a frame's buffer once a plain cell can
/// carry it.
#[derive(Default)]
pub struct KittyDeleteQueue(String);

impl KittyDeleteQueue {
    /// Queue a delete for an upload nothing on screen depends on any more. A
    /// no-op for `None` — a non-kitty protocol, or an entry never placed.
    pub fn queue(&mut self, id: Option<u32>) {
        if let Some(id) = id {
            self.0.push_str(&kitty_delete_escape(id));
        }
    }

    /// Flush queued deletes into `buf`, riding on the first plain cell found in
    /// `area` (never dropped — deferred to a later flush if none is found).
    pub fn flush(&mut self, area: Rect, buf: &mut Buffer) {
        flush_kitty_deletes_into(&mut self.0, area, buf);
    }
}

/// Rows of `src` with no opaque pixel anywhere across them: how many, the
/// longest contiguous run of them, and where that run starts (SQ-0698).
///
/// Reported on every tiled band's log line, because a hole in a tiled flank is
/// invisible in the band's RECT — the placement is the full height either way —
/// and shows up only as a black stripe on the user's screen. The RUN and WHERE
/// are the numbers that matter. A run at row 0 is the band's own top edge, where
/// the renderer clears the rows a chrome text strip draws as crisp cells (a v6
/// text row is 16 native px, so the leading run is small); an INTERIOR run is a
/// hole, and Shogun's tiled pieces were separated by one of 64 native rows.
fn blank_rows(src: &image::RgbaImage) -> (u32, u32, u32) {
    let (mut total, mut run, mut longest, mut at) = (0, 0, 0, 0);
    for y in 0..src.height() {
        if (0..src.width()).all(|x| src.get_pixel(x, y)[3] == 0) {
            total += 1;
            run += 1;
            if run > longest {
                longest = run;
                at = y + 1 - run;
            }
        } else {
            run = 0;
        }
    }
    (total, longest, at)
}

// ── Kitty virtual-placement emission (SQ-0520) ────────────────────────────────

/// Kitty's `rowcolumn-diacritics.txt`, complete: the index into this table IS
/// the value the diacritic encodes. A placeholder cell carries up to three of
/// them — image ROW, image COLUMN, and the image id's HIGH BYTE — and every one
/// of those three needs an arbitrary index, so the table has to be whole. It
/// held only the first 140 entries while the emitter used exactly two values
/// (the row, and zero for the other two); that cap silently made a wide
/// placement and a high-byte id inexpressible (SQ-0772).
///
/// Transcribed from `ratatui-image` 11.0.6's `DIACRITICS` and cross-checked
/// entry-for-entry against `qwertty-term-vt` 0.4.0's independent copy — two
/// implementations that agree on all 297 values, rather than one recalled table.
const KITTY_DIACRITICS: [char; 297] = [
    '\u{305}', '\u{30D}', '\u{30E}', '\u{310}', '\u{312}', '\u{33D}', '\u{33E}', '\u{33F}',
    '\u{346}', '\u{34A}', '\u{34B}', '\u{34C}', '\u{350}', '\u{351}', '\u{352}', '\u{357}',
    '\u{35B}', '\u{363}', '\u{364}', '\u{365}', '\u{366}', '\u{367}', '\u{368}', '\u{369}',
    '\u{36A}', '\u{36B}', '\u{36C}', '\u{36D}', '\u{36E}', '\u{36F}', '\u{483}', '\u{484}',
    '\u{485}', '\u{486}', '\u{487}', '\u{592}', '\u{593}', '\u{594}', '\u{595}', '\u{597}',
    '\u{598}', '\u{599}', '\u{59C}', '\u{59D}', '\u{59E}', '\u{59F}', '\u{5A0}', '\u{5A1}',
    '\u{5A8}', '\u{5A9}', '\u{5AB}', '\u{5AC}', '\u{5AF}', '\u{5C4}', '\u{610}', '\u{611}',
    '\u{612}', '\u{613}', '\u{614}', '\u{615}', '\u{616}', '\u{617}', '\u{657}', '\u{658}',
    '\u{659}', '\u{65A}', '\u{65B}', '\u{65D}', '\u{65E}', '\u{6D6}', '\u{6D7}', '\u{6D8}',
    '\u{6D9}', '\u{6DA}', '\u{6DB}', '\u{6DC}', '\u{6DF}', '\u{6E0}', '\u{6E1}', '\u{6E2}',
    '\u{6E4}', '\u{6E7}', '\u{6E8}', '\u{6EB}', '\u{6EC}', '\u{730}', '\u{732}', '\u{733}',
    '\u{735}', '\u{736}', '\u{73A}', '\u{73D}', '\u{73F}', '\u{740}', '\u{741}', '\u{743}',
    '\u{745}', '\u{747}', '\u{749}', '\u{74A}', '\u{7EB}', '\u{7EC}', '\u{7ED}', '\u{7EE}',
    '\u{7EF}', '\u{7F0}', '\u{7F1}', '\u{7F3}', '\u{816}', '\u{817}', '\u{818}', '\u{819}',
    '\u{81B}', '\u{81C}', '\u{81D}', '\u{81E}', '\u{81F}', '\u{820}', '\u{821}', '\u{822}',
    '\u{823}', '\u{825}', '\u{826}', '\u{827}', '\u{829}', '\u{82A}', '\u{82B}', '\u{82C}',
    '\u{82D}', '\u{951}', '\u{953}', '\u{954}', '\u{F82}', '\u{F83}', '\u{F86}', '\u{F87}',
    '\u{135D}', '\u{135E}', '\u{135F}', '\u{17DD}', '\u{193A}', '\u{1A17}', '\u{1A75}', '\u{1A76}',
    '\u{1A77}', '\u{1A78}', '\u{1A79}', '\u{1A7A}', '\u{1A7B}', '\u{1A7C}', '\u{1B6B}', '\u{1B6D}',
    '\u{1B6E}', '\u{1B6F}', '\u{1B70}', '\u{1B71}', '\u{1B72}', '\u{1B73}', '\u{1CD0}', '\u{1CD1}',
    '\u{1CD2}', '\u{1CDA}', '\u{1CDB}', '\u{1CE0}', '\u{1DC0}', '\u{1DC1}', '\u{1DC3}', '\u{1DC4}',
    '\u{1DC5}', '\u{1DC6}', '\u{1DC7}', '\u{1DC8}', '\u{1DC9}', '\u{1DCB}', '\u{1DCC}', '\u{1DD1}',
    '\u{1DD2}', '\u{1DD3}', '\u{1DD4}', '\u{1DD5}', '\u{1DD6}', '\u{1DD7}', '\u{1DD8}', '\u{1DD9}',
    '\u{1DDA}', '\u{1DDB}', '\u{1DDC}', '\u{1DDD}', '\u{1DDE}', '\u{1DDF}', '\u{1DE0}', '\u{1DE1}',
    '\u{1DE2}', '\u{1DE3}', '\u{1DE4}', '\u{1DE5}', '\u{1DE6}', '\u{1DFE}', '\u{20D0}', '\u{20D1}',
    '\u{20D4}', '\u{20D5}', '\u{20D6}', '\u{20D7}', '\u{20DB}', '\u{20DC}', '\u{20E1}', '\u{20E7}',
    '\u{20E9}', '\u{20F0}', '\u{2CEF}', '\u{2CF0}', '\u{2CF1}', '\u{2DE0}', '\u{2DE1}', '\u{2DE2}',
    '\u{2DE3}', '\u{2DE4}', '\u{2DE5}', '\u{2DE6}', '\u{2DE7}', '\u{2DE8}', '\u{2DE9}', '\u{2DEA}',
    '\u{2DEB}', '\u{2DEC}', '\u{2DED}', '\u{2DEE}', '\u{2DEF}', '\u{2DF0}', '\u{2DF1}', '\u{2DF2}',
    '\u{2DF3}', '\u{2DF4}', '\u{2DF5}', '\u{2DF6}', '\u{2DF7}', '\u{2DF8}', '\u{2DF9}', '\u{2DFA}',
    '\u{2DFB}', '\u{2DFC}', '\u{2DFD}', '\u{2DFE}', '\u{2DFF}', '\u{A66F}', '\u{A67C}', '\u{A67D}',
    '\u{A6F0}', '\u{A6F1}', '\u{A8E0}', '\u{A8E1}', '\u{A8E2}', '\u{A8E3}', '\u{A8E4}', '\u{A8E5}',
    '\u{A8E6}', '\u{A8E7}', '\u{A8E8}', '\u{A8E9}', '\u{A8EA}', '\u{A8EB}', '\u{A8EC}', '\u{A8ED}',
    '\u{A8EE}', '\u{A8EF}', '\u{A8F0}', '\u{A8F1}', '\u{AAB0}', '\u{AAB2}', '\u{AAB3}', '\u{AAB7}',
    '\u{AAB8}', '\u{AABE}', '\u{AABF}', '\u{AAC1}', '\u{FE20}', '\u{FE21}', '\u{FE22}', '\u{FE23}',
    '\u{FE24}', '\u{FE25}', '\u{FE26}', '\u{10A0F}', '\u{10A38}', '\u{1D185}', '\u{1D186}', '\u{1D187}',
    '\u{1D188}', '\u{1D189}', '\u{1D1AA}', '\u{1D1AB}', '\u{1D1AC}', '\u{1D1AD}', '\u{1D242}', '\u{1D243}',
    '\u{1D244}',
];

/// Plain-Rust base64 (standard alphabet, padded) — only used for kitty image
/// transmission, so no dependency is worth it.
fn kitty_b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Deflate a transmit payload for the kitty protocol's `o=z`.
///
/// RFC 1950 (zlib), which is the ONLY compression the protocol defines. Level 6
/// (`Compression::default()`) rather than 1 or 9 — measured on real v6 canvases
/// in SQ-0976, where the three levels are not close:
///
/// | canvas (turns)                | b64 raw | L1 b64 | L6 b64 | L9 b64 | L1 / L6 / L9 ms |
/// |-------------------------------|--------:|-------:|-------:|-------:|-----------------|
/// | Zork Zero r393 win 7, 640x400 | 1365336 |  35068 |   6580 |   6024 | 0.40 / 1.43 / 2.40 |
/// | Shogun r322 win 7, 640x400    | 1365336 |  20276 |   6532 |   6040 | 0.27 / 1.77 / 3.30 |
/// | Journey r83 win 3, 232x304    |  376152 |  27168 |  10884 |  10016 | 0.35 / 1.88 / 5.34 |
/// | Zork Zero r393 win 1, 640x78  |  266240 |   3180 |    960 |    964 | 0.05 / 0.26 / 0.37 |
///
/// L1 leaves three to five times as much on the wire for a millisecond saved;
/// L9 buys 5–8% more for two to four times L6's cost, and on one canvas it is
/// *larger*. Sixteen-colour flat artwork is what deflate is best at, and 1.4–3.3
/// ms on the render worker is nothing beside 1.37 MB of base64.
///
/// Writing into a `Vec` has no failure mode; the `expect` documents that rather
/// than inventing a fallback path no test could reach.
fn zlib_deflate(raw: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut enc = flate2::write::ZlibEncoder::new(
        Vec::with_capacity(raw.len() / 32 + 64),
        flate2::Compression::default(),
    );
    enc.write_all(raw).expect("a zlib encoder writing into a Vec cannot fail");
    enc.finish().expect("a zlib encoder writing into a Vec cannot fail")
}

/// The kitty transmit sequence for `canvas` as image `id`: a VIRTUAL placement
/// (`U=1`) declaring an explicit `r×c` grid, so the terminal scales the image
/// to exactly the placeholder rect (SQ-0520). RGBA, chunked per the protocol's
/// 4096-encoded-byte limit, and zlib-compressed when `compress` says the terminal
/// can inflate it. (No tmux passthrough — matches the app's existing kitty
/// support, which targets direct terminals.)
///
/// **`compress` is a capability, not a preference** (SQ-0997). It comes from
/// [`kitty_compression`], and this function stated `o=z` unconditionally until it
/// did: a terminal that answers the kitty query but not the `o=z` one refuses a
/// deflated transmission, stores no image, and draws nothing for every placement
/// naming it — with no error and nothing on screen to explain the missing picture.
/// See [`kitty_compression`] for why an unasked terminal counts as "cannot".
///
/// **`o=z` is the payload's encoding and nothing else** (SQ-0976). Per the kitty
/// graphics protocol: *"the payload is now compressed using deflate (this occurs
/// prior to base64 encoding)"*, so `f=32` still names the format the terminal
/// finds after inflating, and `s=`/`v=` still name the **uncompressed** image's
/// pixel dimensions — the terminal sizes its buffer from `s*v*4` and the inflated
/// payload must be exactly that long. The `S` key is not involved: the spec
/// requires it only for PNG-plus-compression, where the decompressed length is
/// not implied by the geometry.
///
/// Chunking therefore applies to the COMPRESSED stream, because it is the thing
/// being base64-encoded — *"the pixel data must first be base64 encoded then
/// chunked up into chunks no larger than 4096 bytes"*. 3072 compressed bytes make
/// exactly 4096 base64 characters and satisfy "all chunks except the last must
/// have a size that is a multiple of 4"; continuation chunks carry only `m` and
/// `q`, as the spec demands, so `o=z` is stated once on the first chunk and
/// governs the reassembled whole.
///
/// **`p=` names the placement** (SQ-0995), because `id` is now stable across a
/// window's whole life and this command is re-issued whenever the canvas changes.
/// The protocol says *"When re-transmitting image data for a specific id, the
/// existing image and all its placements must be deleted"*, so on a conforming
/// terminal this command replaces both; but Ghostty's storage replaces only the
/// image and leaves placements alone, and an unnamed placement (`p=0`) is
/// *"assign me an internal id"* — so a hundred re-transmits would leave a hundred
/// duplicate placements. A named one is replaced in the map. The placeholder cells
/// still encode placement 0, which resolves to "the first virtual placement of
/// this image" and therefore to the only one.
fn kitty_transmit_virtual(
    canvas: &image::RgbaImage,
    id: u32,
    rows: u16,
    cols: u16,
    compress: bool,
) -> String {
    use std::fmt::Write as _;
    let (w, h) = (canvas.width(), canvas.height());
    let deflated;
    // SQ-0997: `compress` is [`kitty_compression`]'s answer for the picker in
    // force. Raw when it is false — the geometry keys are untouched either way,
    // because `o=z` describes the payload's encoding and nothing about the image.
    let (payload, encoding): (&[u8], &str) = if compress {
        deflated = zlib_deflate(canvas.as_raw());
        (&deflated, "o=z,")
    } else {
        (canvas.as_raw(), "")
    };
    let chunks: Vec<&[u8]> = payload.chunks(3072).collect();
    let n = chunks.len();
    let mut out = String::with_capacity(payload.len() / 3 * 4 + n * 24);
    for (i, chunk) in chunks.into_iter().enumerate() {
        let more = u8::from(i + 1 < n);
        if i == 0 {
            write!(
                out,
                "\x1b_Gq=2,i={id},p={KITTY_PLACEMENT},a=T,U=1,f=32,{encoding}t=d,\
                 s={w},v={h},r={rows},c={cols},m={more};"
            )
            .unwrap();
        } else {
            write!(out, "\x1b_Gq=2,m={more};").unwrap();
        }
        out.push_str(&kitty_b64(chunk));
        out.push_str("\x1b\\");
    }
    out
}

/// What kitty uploads have cost, and what the same pixels would have cost with no
/// compression at all (SQ-1005).
///
/// Read off the WIRE rather than out of an encoder, which is why it can speak for
/// both of them: lanthorn emits its graphics-window transmits itself
/// ([`kitty_transmit_virtual`]) while every band, composite, inline picture and
/// cover tile is encoded by `ratatui-image`, and neither hands back the two
/// lengths. The transmit declares its own geometry — `f=32` with `s=W,v=H` is
/// `W · H · 4` bytes of RGBA — so the uncompressed size is known without inflating
/// anything, or even reading the payload.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct UploadBytes {
    /// Bytes actually written for the transmit: control blocks, base64 and the
    /// `ESC \` terminators of every chunk.
    pub wire: u64,
    /// The image's own pixel bytes, from the geometry the transmit declares.
    pub pixels: u64,
    /// Transmits counted — a first chunk each, so this is images and not chunks.
    pub uploads: u64,
    /// `a=d` delete commands seen, whether or not they named an id this
    /// particular measurement can account for (SQ-1201). Always incremented on
    /// sight — pairing is what decides [`Self::freed_pixels`], not this.
    pub deletes: u64,
    /// Pixel bytes a delete freed: credited only when the delete's `i=` named an
    /// id this measurement had already seen transmitted (and no LATER delete in
    /// between had already freed it). A delete for an unknown or already-freed id
    /// still counts toward [`Self::deletes`] above, just not here — SQ-1190's bug
    /// was exactly a delete that never got SENT, which this cannot see either;
    /// what it catches is the transmit whose delete never arrives at all.
    pub freed_pixels: u64,
    /// Uploads measured here whose id no LATER delete in the same measurement
    /// named — still resident in the terminal (or, for [`GraphicsRender::uploads`],
    /// resident as of the last id this struct's own traffic touched) when the
    /// measurement ended. Zero for [`measure_transmit`] on a single small
    /// fragment, where a transmit and the delete that eventually frees it are
    /// almost always in two DIFFERENT fragments — see [`measure_traffic`] for the
    /// function built to pair them, and [`GraphicsRender::note_upload_id`] for the
    /// live equivalent kept as typed state instead of re-scanned text.
    pub stranded_uploads: u64,
    /// Pixel bytes those stranded uploads account for.
    pub stranded_pixels: u64,
}

impl UploadBytes {
    /// Sums every genuinely cumulative field. `stranded_uploads`/`stranded_pixels`
    /// are deliberately NOT summed: they are a snapshot of "what is outstanding
    /// right now", and adding two snapshots together does not mean anything — a
    /// caller that wants them kept current across many `add` calls (as
    /// [`GraphicsRender`] does) assigns them separately, from state that persists
    /// across the calls this discards.
    fn add(&mut self, other: UploadBytes) {
        self.wire += other.wire;
        self.pixels += other.pixels;
        self.uploads += other.uploads;
        self.deletes += other.deletes;
        self.freed_pixels += other.freed_pixels;
    }

    /// What [`Self::pixels`] would have occupied on the wire uncompressed: base64
    /// is 4 bytes per 3, and the control blocks are the same either way.
    ///
    /// This is the honest comparison for "what did `o=z` buy", because `wire`
    /// already includes base64 — comparing a deflated-and-encoded stream against
    /// raw pixel bytes would credit compression with the 4/3 expansion it never
    /// removed.
    pub fn wire_uncompressed(&self) -> u64 {
        self.pixels.div_ceil(3) * 4
    }
}

/// One kitty APC chunk's control block (everything up to its first `;`, or its
/// whole body if it has none) and the chunk's total wire length (control block +
/// base64 + the `ESC \` terminator). Shared by [`measure_transmit`] and
/// [`measure_traffic`] so the two ways of reading this file's own emitted bytes
/// agree on where one chunk ends and the next begins.
fn kitty_chunks(text: &str) -> Vec<(&str, u64)> {
    let b = text.as_bytes();
    let mut i = 0usize;
    let mut out = Vec::new();
    while let Some(rel) = b[i..].windows(3).position(|w| w == b"\x1b_G") {
        let start = i + rel;
        // The chunk runs to its `ESC \`; a truncated one is measured to the end.
        let term = b[start..].windows(2).position(|w| w == b"\x1b\\");
        let end = term.map_or(b.len(), |p| start + p + 2);
        // `content_end` excludes the terminator itself, so a chunk with no `;` —
        // every delete escape, which has no payload to introduce one — does not
        // read its own `ESC \` as part of the last param's value (SQ-1201: that
        // silently broke `i=<id>` parsing on a delete, which has no other
        // separator after it).
        let content_end = term.map_or(b.len(), |p| start + p);
        let head_end = b[start..content_end].iter().position(|&c| c == b';').map_or(content_end, |p| start + p);
        out.push((&text[start + 3..head_end], (end - start) as u64));
        i = end;
        if i >= b.len() {
            break;
        }
    }
    out
}

fn kitty_param(params: &str, key: &str) -> Option<u64> {
    params.split(',').find_map(|kv| kv.strip_prefix(key)?.strip_prefix('=')?.parse().ok())
}

/// Whether a chunk's own params (never its neighbours') are a transmit's — `a=T`
/// (transmit and display) or `a=t` (transmit only), the two spellings
/// `kitty_transmit_virtual` and `ratatui-image`'s encoder emit.
///
/// The gate `measure_traffic` needed and `measure_transmit` never did: lanthorn's
/// own kitty capability PROBE (`a=q`, sent once at startup to ask whether the
/// terminal answers at all) transmits a throwaway 1x1 `s=1,v=1` image too — s/v
/// alone is not "this is an upload", it is "this chunk names pixel geometry",
/// and a query names it for the same reason a transmit does. `measure_transmit`
/// was never fed that probe (only its own already-known-good transmit text), so
/// this never manifested there; `measure_traffic` reads the WHOLE wire, probe
/// included, and without this gate counted it as two extra uploads and two
/// falsely-stranded ids (SQ-1201).
fn kitty_is_upload_action(params: &str) -> bool {
    params.split(',').any(|kv| kv == "a=T" || kv == "a=t")
}

/// Measure one transmit's cost off its own bytes. See [`UploadBytes`].
///
/// Cheap on purpose: it reads each chunk's control block — the few dozen bytes
/// before the `;` — and the chunk's length, never its payload. A 14 MB upload
/// costs the same to measure as a 200-byte one, which is what lets this sit on the
/// frame path where [`crate::terminal_dump::Traffic`] deliberately would not scan.
///
/// A chunk with no `s`/`v` is a continuation (`m=1` carries no geometry) and adds
/// wire without adding pixels, so a chunked upload is counted once.
///
/// `deletes` is counted here too (an `a=d` chunk, by `,a=d,` appearing in the
/// params — cheap, no allocation) but `freed_pixels`/`stranded_*` are always zero:
/// pairing a delete against the transmit it frees needs to have seen BOTH within
/// one measurement, and every call site that feeds this function a small
/// per-frame fragment (SQ-1005) never has both in the same fragment — the delete
/// for THIS id rides on a placement several frames later. [`measure_traffic`] is
/// the whole-capture sibling that does the pairing.
pub fn measure_transmit(transmit: &str) -> UploadBytes {
    let mut out = UploadBytes::default();
    for (params, wire) in kitty_chunks(transmit) {
        out.wire += wire;
        if params.split(',').any(|kv| kv == "a=d") {
            out.deletes += 1;
            continue;
        }
        if !kitty_is_upload_action(params) {
            continue;
        }
        // `S` is the kitty spec's own "size of the uncompressed data" and only ever
        // accompanies a compressed PNG; for the `f=32` RGBA we and the crate emit,
        // the declared geometry is the same fact and is always present.
        if let Some(size) = kitty_param(params, "S") {
            out.pixels += size;
            out.uploads += 1;
        } else if let (Some(w), Some(h)) = (kitty_param(params, "s"), kitty_param(params, "v")) {
            out.pixels += w * h * 4;
            out.uploads += 1;
        }
    }
    out
}

/// Measure a WHOLE capture's kitty traffic — every transmit and every delete in
/// `text`, paired by `i=<id>` (SQ-1201).
///
/// This is [`measure_transmit`] with the one thing a single small fragment can
/// never show it: a delete's OWN id, matched against a transmit the same text
/// also contains. A transmit sets/overwrites a local `id → pixels` ledger — a
/// re-transmit to an id already held REPLACES it in the terminal, per the kitty
/// spec's own re-transmit rule, so the ledger does too, rather than accumulating
/// both sizes — and a delete removes its id from the ledger, crediting
/// `freed_pixels`. Whatever the ledger still holds when `text` runs out is
/// `stranded_uploads`/`stranded_pixels`: transmitted in this capture, never freed
/// in it.
///
/// A delete naming an id nothing in `text` transmitted (freed by an EARLIER
/// capture not included here, or already freed once and named again) still counts
/// toward `deletes`, just not `freed_pixels` — there is nothing in this text to
/// credit it against.
///
/// A transmit with no `i=` is invisible to the ledger — neither freed nor
/// stranded — rather than assumed safe: `kitty_transmit_virtual` and
/// `ratatui-image`'s own kitty encoder both always state one (every id lanthorn
/// emits is meant to be freed later), so this is unreached on lanthorn's own
/// traffic today, and an id-less transmit still counts toward `pixels`/`uploads`
/// above, just not toward stranding.
///
/// Whole-capture, not per-frame: meant for a caller holding the ENTIRE emitted
/// stream at once (the pty-stream harness), where the cost of one `HashMap` is
/// nothing beside the megabytes of image data already in hand. [`measure_transmit`]
/// stays the frame-path measurer, unchanged, for exactly that reason.
pub fn measure_traffic(text: &str) -> UploadBytes {
    let mut out = UploadBytes::default();
    let mut outstanding: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();
    for (params, wire) in kitty_chunks(text) {
        out.wire += wire;
        let id = kitty_param(params, "i").map(|v| v as u32);
        if params.split(',').any(|kv| kv == "a=d") {
            out.deletes += 1;
            if let Some(id) = id {
                if let Some(px) = outstanding.remove(&id) {
                    out.freed_pixels += px;
                }
            }
            continue;
        }
        if !kitty_is_upload_action(params) {
            continue;
        }
        let size = kitty_param(params, "S").or_else(|| {
            let (w, h) = (kitty_param(params, "s")?, kitty_param(params, "v")?);
            Some(w * h * 4)
        });
        if let Some(size) = size {
            out.pixels += size;
            out.uploads += 1;
            if let Some(id) = id {
                outstanding.insert(id, size);
            }
        }
    }
    out.stranded_uploads = outstanding.len() as u64;
    out.stranded_pixels = outstanding.values().sum();
    out
}

/// Hash a canvas's pixels, so two canvases that look identical share one upload
/// (SQ-0564). Only ever compared against other hashes from this same function —
/// a collision would place the wrong cached image, which SipHash makes a
/// non-concern at these sizes.
fn canvas_hash(canvas: &image::RgbaImage) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    canvas.as_raw().hash(&mut h);
    canvas.dimensions().hash(&mut h);
    h.finish()
}

/// Every placeholder cell is a real buffer cell of forced width 1: the escape it
/// carries prints exactly one column, so ratatui's cursor bookkeeping and the
/// terminal's agree.
const PLACEHOLDER_WIDTH: ratatui::buffer::CellDiffOption =
    ratatui::buffer::CellDiffOption::ForcedWidth(std::num::NonZeroU16::MIN);

/// Write ONE row of a kitty virtual placement into `buf`: `width` self-describing
/// placeholder cells starting at `(x0, y)`, each naming its own image row, image
/// column and id high byte, with the id's low 24 bits as the cell's foreground.
///
/// SELF-DESCRIBING IS THE POINT (SQ-0772). The protocol lets a placeholder with no
/// diacritics inherit its row/column/id-high-byte from the cell to its left, and
/// both this app and `ratatui-image` used to emit exactly one anchored cell per row
/// followed by bare continuations. That is legal until a later frame overpaints the
/// row's left edge: the anchor dies, the survivors keep only the foreground's low 24
/// bits, and the run either names an image the terminal does not hold (drawing
/// nothing) or — for lanthorn's own `0x00B0_xxxx` ids, whose high byte is zero —
/// resolves to the art's FIRST row, redrawn on every row and shifted a column right.
/// A cell that carries its own three diacritics cannot be orphaned by anything that
/// happens to its neighbours.
///
/// BUFFER-VISIBLE IS THE OTHER HALF. The old shape squeezed the whole row into the
/// first cell and marked the rest `Skip`, which made the placement invisible to
/// ratatui's damage model: a later frame that simply stopped drawing there produced
/// no diff, so nothing unpainted the placeholders and they outlived the frame that
/// made them (SQ-0763). Real cells diff like any other content — a frame that stops
/// placing writes ordinary spaces over them, and a frame that re-places writes the
/// identical symbols, so the steady state still costs nothing.
///
/// `prefix` (an upload, or queued deletes) rides on the first cell, as before.
/// Columns past the table's 297 entries fall back to bare continuation placeholders:
/// the protocol cannot express a larger explicit index either.
fn kitty_place_row(
    buf: &mut Buffer,
    (x0, y): (u16, u16),
    width: u16,
    fg: Color,
    (row_d, extra_d): (char, char),
    prefix: Option<&str>,
) {
    let mut symbol = String::new();
    let mut prefix = prefix;
    for x in 0..width {
        symbol.clear();
        if let Some(seq) = prefix.take() {
            symbol.push_str(seq);
        }
        symbol.push('\u{10EEEE}');
        if let Some(&col_d) = KITTY_DIACRITICS.get(usize::from(x)) {
            symbol.push(row_d);
            symbol.push(col_d);
            symbol.push(extra_d);
        }
        let Some(cell) = buf.cell_mut((x0 + x, y)) else { continue };
        cell.set_symbol(&symbol).set_fg(fg).set_diff_option(PLACEHOLDER_WIDTH);
    }
}

/// Write the placeholder rows for image `id` into `buf` over `area`. `transmit`
/// (the first frame after an encode, plus any queued deletes) rides on the very
/// first cell.
fn kitty_place_rows(id: u32, transmit: Option<&str>, area: Rect, buf: &mut Buffer) {
    let [id_hi, id_r, id_g, id_b] = id.to_be_bytes();
    // The third diacritic carries the id's HIGH byte; the foreground carries the
    // other 24 bits. Emitting the byte instead of a hardcoded zero is what makes
    // the placement agree with `kitty_transmit_virtual`'s full 32-bit `i={id}` for
    // any id, not merely for the `0x00B0_xxxx` range this file happens to allocate
    // from (SQ-0772). An id above the table's 296 is unplaceable in this protocol at
    // all; drawing nothing beats pointing the terminal at a truncated id, which names
    // a DIFFERENT image and is exactly the failure this quest is about. Unreachable
    // from the allocation above, which never sets the top byte.
    let Some(&extra_d) = KITTY_DIACRITICS.get(usize::from(id_hi)) else { return };
    let fg = Color::Rgb(id_r, id_g, id_b);
    let rows = area.height.min(KITTY_DIACRITICS.len() as u16);
    let mut transmit = transmit;
    for y in 0..rows {
        kitty_place_row(
            buf,
            (area.left(), area.top() + y),
            area.width,
            fg,
            (KITTY_DIACRITICS[usize::from(y)], extra_d),
            transmit.take(),
        );
    }
}

/// Render a `ratatui-image` protocol at `dest`, then re-lay any kitty virtual
/// placement it just wrote into the same self-describing, buffer-visible cells
/// [`kitty_place_row`] emits (SQ-0772).
///
/// WHY THIS WRAPPER EXISTS AT ALL. lanthorn has two kitty emitters and only owns
/// one. Everything drawn through a [`Protocol`] — the v6 raster composite, every
/// chrome-ring band, inline story art, the picker's cover tiles — is placed by
/// `ratatui-image`, whose kitty backend squeezes each row into its first cell and
/// marks the rest `Skip`, with an explicit "use inherited diacritic values"
/// comment. So a fix confined to our own emitter would fix none of the art the
/// player actually looks at: the Journey capture that opened SQ-0772 has the main
/// 920x575 composite orphaned by a later frame trimming its left edge, and that
/// composite is a `ratatui-image` placement. Re-laying the row afterwards makes
/// both emitters produce the same thing without forking the crate or hand-rolling
/// its encoders.
///
/// It reads back what the protocol wrote rather than being told: a `Protocol`
/// keeps its image id, its transmit sequence and its row diacritics private. The
/// three things needed are all fixed by the kitty protocol, not by the crate's
/// taste — the foreground SGR carrying the id's low 24 bits, the placeholder
/// character, and the (row, column, id-high-byte) diacritic triple after it. A row
/// that does not parse into exactly that shape is left exactly as the protocol
/// wrote it, so a half-block or sixel protocol (no placeholders at all) and any
/// future change to the crate's row format degrade to today's behaviour rather
/// than to garbage.
/// Returns the kitty image id it just placed, when the protocol was a kitty one —
/// `None` for half-blocks, sixel, iterm2 or a failed parse. That id is the only
/// handle we ever get on a `ratatui-image` upload, and without it the image can
/// never be freed in the terminal (SQ-0753): the crate keeps the id private, so
/// reading it back off the placement it just wrote is how we learn to name it.
pub fn place_protocol(proto: &Protocol, dest: Rect, buf: &mut Buffer) -> Option<u32> {
    place_protocol_with(proto, dest, buf, "", "").0
}

/// [`place_protocol`], with `prefix` (queued `a=d` deletes) riding on the very first
/// placeholder cell — ahead of the protocol's own upload, in the SAME output batch —
/// and `suffix` (deletes for uploads this placement SUPERSEDES) on the very last, so
/// they are freed only once the rect they cover has been covered again (SQ-0817).
///
/// That is the rule SQ-0637 set for graphics windows and the reason `pending_deletes`
/// is a queue rather than a direct write: escapes handed straight to stdout interleave
/// unpredictably with ratatui's diff, while a cell's symbol is emitted exactly where
/// the frame puts it. Bands and the raster composite are placed by `ratatui-image`
/// rather than by our own emitter, so they had no way to carry anything until the
/// re-seat gave them one (SQ-0753). Returns `None` — and carries nothing — for a
/// protocol with no placeholder rows, which is every non-kitty backend.
fn place_protocol_with(
    proto: &Protocol,
    dest: Rect,
    buf: &mut Buffer,
    prefix: &str,
    suffix: &str,
) -> (Option<u32>, UploadBytes) {
    Image::new(proto).render(dest, buf);
    reseat_kitty_placement(dest, buf, prefix, suffix)
}

/// Rewrite each row of a `ratatui-image` kitty placement at `area` in place, and
/// return the image id the rows name. See [`place_protocol`]; a no-op returning
/// `None` for anything that is not a kitty placement.
fn reseat_kitty_placement(
    area: Rect,
    buf: &mut Buffer,
    prefix: &str,
    suffix: &str,
) -> (Option<u32>, UploadBytes) {
    let mut id = None;
    // The image data rides on the row that carries it, ahead of that row's own
    // escapes — `row.prefix`. Measured here because this is the ONE funnel every
    // `ratatui-image` upload passes through (SQ-1005).
    let mut bytes = UploadBytes::default();
    let mut prefix = (!prefix.is_empty()).then_some(prefix);
    // The last cell this re-seat writes — where a `suffix` delete goes, so it is
    // emitted AFTER every byte of the placement it supersedes (SQ-0817).
    let mut last: Option<(u16, u16)> = None;
    for y in area.top()..area.bottom() {
        let Some(symbol) = buf.cell((area.left(), y)).map(|c| c.symbol().to_string()) else { continue };
        let Some(row) = parse_placement_row(&symbol) else { continue };
        // The prefix is only ever consumed on a row we could also NAME, so the
        // caller's "did this carry my deletes?" question is answered by the id
        // coming back — never half-answered.
        let this_id = placement_id(row.fg, row.extra_d);
        id = id.or(this_id);
        bytes.add(measure_transmit(row.prefix));
        let width = row.cells.min(area.width);
        let head = match this_id.and(prefix.take()) {
            Some(p) => format!("{p}{}", row.prefix),
            None => row.prefix.to_string(),
        };
        kitty_place_row(buf, (area.left(), y), width, row.fg, (row.row_d, row.extra_d), Some(&head));
        if width > 0 {
            last = Some((area.left() + width - 1, y));
        }
        // Anything the protocol marked `Skip` past the run we just rewrote (a
        // shorter row than the rect, which `full_width` clamping can produce)
        // would otherwise stay invisible to the diff for ever.
        for x in width..area.width {
            if let Some(c) = buf.cell_mut((area.left() + x, y)) {
                if c.diff_option == ratatui::buffer::CellDiffOption::Skip {
                    c.set_diff_option(ratatui::buffer::CellDiffOption::None);
                }
            }
        }
    }
    // The supersede-deletes go last, appended to the final placeholder cell — after
    // the transmit that rides on the first cell and after every placeholder row, so
    // the rect this delete frees is already covered by its replacement (SQ-0817).
    // Only ever attached to a placement we could NAME, for the same reason the prefix
    // is: the caller reads the returned id as "my escapes went out".
    if let (false, Some(pos), Some(_)) = (suffix.is_empty(), last, id) {
        if let Some(cell) = buf.cell_mut(pos) {
            let symbol = format!("{}{suffix}", cell.symbol());
            cell.set_symbol(&symbol);
        }
    }
    (id, bytes)
}

/// Reassemble a kitty image id from the two halves a placeholder row carries: the
/// low 24 bits ride as the cell foreground, the high byte as the third diacritic
/// (SQ-0753). The inverse of what [`kitty_place_rows`] writes, and of what
/// `ratatui-image` writes — both split the id exactly this way, because the kitty
/// protocol says to.
fn placement_id(fg: Color, extra_d: char) -> Option<u32> {
    let Color::Rgb(r, g, b) = fg else { return None };
    let hi = KITTY_DIACRITICS.iter().position(|&c| c == extra_d)?;
    Some(u32::from_be_bytes([u8::try_from(hi).ok()?, r, g, b]))
}

/// One `ratatui-image` placeholder row, read back off the cell it was written to.
struct PlacementRow<'a> {
    /// Everything before the row's own escapes — the image upload, when this is
    /// the row that carries it. Passed through untouched.
    prefix: &'a str,
    /// The id's low 24 bits, as the foreground the protocol chose.
    fg: Color,
    /// The image row and id-high-byte diacritics, verbatim: the row index is the
    /// protocol's to decide (it slices images across rows) and the high byte is
    /// the only part of the id the foreground cannot carry.
    row_d: char,
    extra_d: char,
    /// Placeholder cells in the row.
    cells: u16,
}

fn parse_placement_row(symbol: &str) -> Option<PlacementRow<'_>> {
    let at = symbol.find('\u{10EEEE}')?;
    let (head, tail) = symbol.split_at(at);
    // `ESC[38;2;r;g;bm` immediately before the first placeholder is the id colour.
    let sgr = head.rfind("\x1b[38;2;")?;
    let rgb = head.get(sgr + 7..)?.strip_suffix('m')?;
    let mut parts = rgb.split(';');
    let mut byte = || parts.next()?.parse::<u8>().ok();
    let fg = Color::Rgb(byte()?, byte()?, byte()?);
    if parts.next().is_some() {
        return None;
    }
    // The protocol's own cursor-save sits between the upload and the id colour.
    let prefix = &head[..head[..sgr].rfind("\x1b[s")?];

    let mut diacritics = tail.chars().skip(1).take_while(|c| KITTY_DIACRITICS.contains(c));
    let row_d = diacritics.next()?;
    let _col_d = diacritics.next()?;
    let extra_d = diacritics.next()?;
    let cells = u16::try_from(tail.chars().filter(|&c| c == '\u{10EEEE}').count()).ok()?;
    Some(PlacementRow { prefix, fg, row_d, extra_d, cells })
}

/// SQ-0824: the resampler picks its filter by direction, so a pane smaller than the
/// artwork stops dropping the rows and columns a dithered picture keeps its detail in.
///
/// Judged against the per-axis single-resample ideal — an area average where an axis
/// shrinks, replication where it grows — which is the thing "resample once, from the
/// best source, with the right filter" is trying to be. Nearest cannot come close on a
/// minification: it is off by an RMS of ~10 on a dithered plate where the directional
/// resampler is off by ~1, and that gap IS the reported aliasing.
///
/// FALSIFY by restoring `image::imageops::resize(src, tw, th, FilterType::Nearest)` as
/// the body of `resize_directional`: every minifying case fails on its RMS bound.
#[cfg(all(test, feature = "t-render"))]
mod resample_tests {
    // SQ-0973's cases below drive the shipped half-blocks composite end to end, so this
    // module now needs the render types (`Picker`, `Protocol`, `Buffer`, …) alongside
    // the resampler it was written for.
    use super::*;
    use image::{Rgba, RgbaImage};

    /// A plate in the shape of the artwork this quest is about: a four-ink palette laid
    /// down as broad flat regions, joined by checkerboard-dithered transition bands
    /// (the shadow gradients Journey's canyon is built from), with hard one-pixel edges
    /// cutting across (the foreground rocks). Synthetic, so the case runs on a machine
    /// without the gitignored story — the shipped floppy's real plate is measured by
    /// `v6_art_resample.rs`, which skips when the fixture is absent.
    fn dithered_plate(w: u32, h: u32) -> RgbaImage {
        let mut img = RgbaImage::new(w, h);
        let inks = [
            Rgba([0x20, 0x18, 0x10, 0xff]),
            Rgba([0xc8, 0x70, 0x28, 0xff]),
            Rgba([0x48, 0x38, 0x60, 0xff]),
            Rgba([0xf0, 0xe0, 0xa0, 0xff]),
        ];
        for y in 0..h {
            for x in 0..w {
                // Four horizontal regions; the middle third of each boundary dithers
                // between the two inks either side of it rather than stepping.
                let t = y as f64 * 4.0 / h.max(1) as f64;
                let band = (t.floor() as usize).min(3);
                let frac = t - t.floor();
                let ink = if frac > 0.85 && band < 3 && (x + y) % 2 == 0 {
                    inks[band + 1]
                } else if x % 37 == 0 || y % 41 == 0 || (x / 8 + y / 8) % 11 == 0 {
                    inks[(band + 2) % inks.len()]
                } else {
                    inks[band]
                };
                img.put_pixel(x, y, ink);
            }
        }
        img
    }

    /// The area-weighted average — the correct answer for a shrinking axis.
    fn area_average(src: &RgbaImage, tw: u32, th: u32) -> RgbaImage {
        let (sw, sh) = src.dimensions();
        let (fx, fy) = (sw as f64 / tw as f64, sh as f64 / th as f64);
        let mut out = RgbaImage::new(tw, th);
        for y in 0..th {
            let (y0, y1) = (y as f64 * fy, (y as f64 + 1.0) * fy);
            for x in 0..tw {
                let (x0, x1) = (x as f64 * fx, (x as f64 + 1.0) * fx);
                let (mut acc, mut wsum) = ([0f64; 4], 0f64);
                for sy in (y0.floor() as u32)..(y1.ceil() as u32).min(sh) {
                    let wy = (y1.min(sy as f64 + 1.0) - y0.max(sy as f64)).max(0.0);
                    for sx in (x0.floor() as u32)..(x1.ceil() as u32).min(sw) {
                        let w = wy * (x1.min(sx as f64 + 1.0) - x0.max(sx as f64)).max(0.0);
                        if w <= 0.0 {
                            continue;
                        }
                        let p = src.get_pixel(sx, sy).0;
                        (0..4).for_each(|c| acc[c] += p[c] as f64 * w);
                        wsum += w;
                    }
                }
                let mut px = [0u8; 4];
                (0..4).for_each(|c| px[c] = (acc[c] / wsum).round().clamp(0.0, 255.0) as u8);
                out.put_pixel(x, y, Rgba(px));
            }
        }
        out
    }

    /// Nearest replication along x — the correct answer for a growing axis.
    fn nearest_x(src: &RgbaImage, tw: u32) -> RgbaImage {
        let (sw, sh) = src.dimensions();
        let mut out = RgbaImage::new(tw, sh);
        for y in 0..sh {
            for x in 0..tw {
                let sx = (((x as f64 + 0.5) * sw as f64 / tw as f64).floor() as u32).min(sw - 1);
                out.put_pixel(x, y, *src.get_pixel(sx, y));
            }
        }
        out
    }

    fn transpose(src: &RgbaImage) -> RgbaImage {
        let (w, h) = src.dimensions();
        let mut out = RgbaImage::new(h, w);
        for y in 0..h {
            for x in 0..w {
                out.put_pixel(y, x, *src.get_pixel(x, y));
            }
        }
        out
    }

    fn ideal(src: &RgbaImage, tw: u32, th: u32) -> RgbaImage {
        let (sw, sh) = src.dimensions();
        let mid = if tw < sw { area_average(src, tw, sh) } else { nearest_x(src, tw) };
        let t = transpose(&mid);
        let t = if th < sh { area_average(&t, th, tw) } else { nearest_x(&t, th) };
        transpose(&t)
    }

    fn rms(a: &RgbaImage, b: &RgbaImage) -> f64 {
        assert_eq!(a.dimensions(), b.dimensions());
        let (mut s, mut n) = (0f64, 0f64);
        for (pa, pb) in a.pixels().zip(b.pixels()) {
            for c in 0..3 {
                let d = pa.0[c] as f64 - pb.0[c] as f64;
                s += d * d;
                n += 1.0;
            }
        }
        (s / n).sqrt()
    }

    /// Every direction regime the three art paths can put a resample in, at the ratios
    /// the pane sweep actually produces on Journey's 222×254 canyon plate. The bound is
    /// 2.0 everywhere; Nearest measures 9.9–10.7 on each of the shrinking cases.
    #[test]
    fn resampling_tracks_the_single_resample_ideal_in_every_direction() {
        let src = dithered_plate(222, 254);
        for (tw, th, regime) in [
            (168u32, 198u32, "both axes shrink, hard"),
            (200, 234, "both axes shrink"),
            (212, 244, "both axes shrink, barely"),
            (212, 256, "x shrinks while y grows"),
            (217, 259, "x shrinks barely while y grows"),
            (224, 270, "both axes grow, barely"),
            (328, 378, "both axes grow"),
            (222, 254, "no change at all"),
        ] {
            let got = resize_directional(&src, tw, th);
            assert_eq!(got.dimensions(), (tw, th), "222x254 -> {tw}x{th} ({regime})");
            let ideal = ideal(&src, tw, th);
            let err = rms(&got, &ideal);
            let nearest =
                rms(&image::imageops::resize(&src, tw, th, image::imageops::FilterType::Nearest), &ideal);
            if tw < 222 || th < 254 {
                assert!(
                    err < 4.0 && err * 2.0 < nearest,
                    "222x254 -> {tw}x{th} ({regime}): RMS {err:.3} against the per-axis \
                     single-resample ideal, where a plain Nearest resample scores {nearest:.3}. \
                     A filter chosen by direction must stay under 4 AND beat Nearest by more \
                     than 2x — Nearest on a shrinking axis IS the reported aliasing."
                );
            } else {
                assert_eq!(
                    err, 0.0,
                    "222x254 -> {tw}x{th} ({regime}): a resample that only magnifies must BE \
                     the ideal, pixel for pixel — that is the crisp look at native size and \
                     above, and it is not negotiable"
                );
            }
        }
    }

    /// The other half of the rule, and the one that must NOT regress: magnification is
    /// still bit-exact pixel replication, so art at or above its native size keeps the
    /// crisp look `MAX_V6_UPSCALE` and the corpus tests exist to protect. A smoothing
    /// filter would show up here instantly — Triangle turns this plate's four inks into
    /// hundreds of blends.
    #[test]
    fn magnification_invents_no_colours() {
        let src = dithered_plate(222, 254);
        let inks = |img: &RgbaImage| {
            img.pixels().map(|p| p.0).collect::<std::collections::HashSet<_>>().len()
        };
        for (tw, th) in [(444u32, 508u32), (328, 378), (224, 270), (222, 508)] {
            let got = resize_directional(&src, tw, th);
            assert_eq!(
                inks(&got),
                inks(&src),
                "222x254 -> {tw}x{th} magnifies, so every pixel must be a replicated \
                 source pixel — a growing axis is exactly what Nearest is for"
            );
        }
    }

    /// SQ-0824, second pass: **the `V6_ART_SCALE` pre-double is not a resample the final
    /// one compounds with — it composes away exactly.**
    ///
    /// The premise under investigation was that v6 art is "pre-scaled 2x and then scaled
    /// again off the pre-scale", so the picture the player sees has been through two
    /// samplers. It has been through two *calls*, and that is not the same thing. The
    /// doubling is `image`'s Nearest at an integer ratio, which is pure replication:
    /// output pixel `i` takes source `floor(i/2)`. A second Nearest then takes
    /// `floor((o+0.5)·2N/T)` of that, and `floor(floor(2u)/2) = floor(u)` for every real
    /// `u ≥ 0` — so the pair IS `floor((o+0.5)·N/T)`, the single Nearest resample straight
    /// from the artwork's own resolution. Not approximately: bit for bit, which is what
    /// this case asserts, in both directions and at the ratios the pane sweep produces.
    ///
    /// The consequence is worth stating plainly, because it decides a fix: sampling the
    /// native artwork instead of the unit-space replica **cannot change a single pixel**
    /// anywhere the final resample magnifies — which is every pane at or above ~80
    /// columns, including the 166x44 the defect was reported at. Whatever is wrong there
    /// is not double sampling. (Journey's plate at 166x44 is one art pixel per 3.69 device
    /// pixels, so Nearest emits it as runs of 3 and 4 — the unevenness is the non-integer
    /// magnification itself, and it survives any change of source.)
    ///
    /// Where the two DO differ is the direction decision above, and only there: a target
    /// between the artwork's size and its double magnifies from native while it minifies
    /// from the replica, so the same target picks Nearest one way and the area filter the
    /// other. That is the `(168, 198)` row below, and it is a policy question — this
    /// quest's own shrink win — not an arithmetic one.
    #[test]
    fn the_art_scale_predouble_composes_away_under_nearest() {
        use image::imageops::FilterType;
        let native = dithered_plate(111, 127);
        // Exactly what `session::v6_scaled_art` does at `V6_ART_SCALE`.
        let doubled = image::imageops::resize(&native, 222, 254, FilterType::Nearest);
        for (tw, th) in
            [(444u32, 508u32), (456, 522), (410, 468), (328, 378), (224, 270), (222, 254), (168, 198)]
        {
            assert_eq!(
                image::imageops::resize(&doubled, tw, th, FilterType::Nearest).as_raw(),
                image::imageops::resize(&native, tw, th, FilterType::Nearest).as_raw(),
                "111x127 doubled to 222x254 and then Nearest-resampled to {tw}x{th} must be \
                 BIT-IDENTICAL to one Nearest resample from the native artwork — an integer \
                 replication composes with the sampler that follows it, so the pre-double is \
                 not a second sampling and cannot be what distorts the picture"
            );
        }
        // …and therefore the shipped resampler is a single resample from native wherever
        // it magnifies, which is the regime the defect was reported in.
        for (tw, th) in [(444u32, 508u32), (456, 522), (410, 468), (328, 378), (224, 270)] {
            assert_eq!(
                resize_directional(&doubled, tw, th).as_raw(),
                image::imageops::resize(&native, tw, th, FilterType::Nearest).as_raw(),
                "a magnifying {tw}x{th} band already samples the native artwork exactly once"
            );
        }
    }

    /// The two-pass form leans on a 1:1 pass being a true identity, or a band that grows
    /// on one axis and shrinks on the other would be resampled twice over on the axis
    /// that did not move.
    #[test]
    fn an_unmoved_axis_is_untouched() {
        let src = dithered_plate(64, 64);
        assert_eq!(resize_directional(&src, 64, 64).as_raw(), src.as_raw(), "1:1 is identity");
        let narrowed = resize_directional(&src, 48, 64);
        let twice = resize_directional(&narrowed, 48, 64);
        assert_eq!(twice.as_raw(), narrowed.as_raw(), "an unmoved axis re-resamples to itself");
    }

    /// A minifying letterbox reads native pixels either side of the one Nearest would
    /// have picked, so the band freshness hash has to cover a halo; a magnifying one
    /// does not, and must not pay for one.
    #[test]
    fn only_a_shrinking_letterbox_carries_a_halo() {
        assert_eq!(super::scale_halo(2.0), 0, "magnifying: Nearest samples one pixel");
        assert_eq!(super::scale_halo(1.0), 0, "1:1: Nearest samples one pixel");
        assert_eq!(super::scale_halo(0.5), 3, "half size: a kernel two native pixels wide");
        assert_eq!(super::scale_halo(0.0), 0, "degenerate scales are not a panic");
    }

    /// The RASTER composite's half of the same rule, measured through the protocol's own
    /// `Resize` rather than around it — a pane smaller than the composite must land on
    /// the area-averaged ideal, and one bigger must still be exact pixel replication.
    ///
    /// FALSIFY by restoring `.clamp(1.0, MAX_V6_UPSCALE)` and `Resize::Fit(None)` in
    /// `v6_fit_source`: the shrinking cases fail on their RMS bound, because the pane
    /// then gets an identity copy of the canvas followed by a Nearest shrink.
    ///
    /// SQ-0964: this measures the ENCODED backends, and now says so — the cap it fits
    /// under belongs to kitty/sixel/iTerm2, and half-blocks has none. The panes swept
    /// here all land at or under 2x on the limiting axis, so the two backends would
    /// answer identically anyway; naming the cap is about what the case is claiming,
    /// not about a number that moved. `halfblocks_climbs_past_the_encode_cap_and_kitty_does_not`
    /// below is the other backend's case.
    #[test]
    fn the_raster_composite_takes_one_resample_in_the_right_direction() {
        use ratatui_image::FontSize;
        // Journey's composite: its 320x200 screen at the uniform `V6_ART_SCALE`.
        let canvas = dithered_plate(640, 400);
        let fs = FontSize::new(8, 18);
        for (cols, rows) in [(60u16, 24u16), (70, 30), (76, 28), (100, 40), (160, 60)] {
            let (box_w, box_h) = (cols as u32 * 8, rows as u32 * 18);
            let (src, fit) =
                super::v6_fit_source(&canvas, box_w, box_h, None, Some(super::MAX_V6_UPSCALE));
            let dyn_src = image::DynamicImage::ImageRgba8(src.clone());
            let cells = fit.size_for(&dyn_src, fs, ratatui::layout::Size::new(cols, rows));
            let got = fit.resize(&dyn_src, fs, cells, None).to_rgba8();
            // The protocol pads its output out to the cell grid with a TRANSPARENT
            // background; the composite itself is opaque, so the drawn extent is
            // exactly the opaque part. Measuring the padding would drown the signal.
            let dw = (0..got.width()).filter(|&x| got.get_pixel(x, 0).0[3] != 0).count() as u32;
            let dh = (0..got.height()).filter(|&y| got.get_pixel(0, y).0[3] != 0).count() as u32;
            let drawn = image::imageops::crop_imm(&got, 0, 0, dw, dh).to_image();
            let ideal = ideal(&canvas, dw, dh);
            let err = rms(&drawn, &ideal);
            assert!(dw > 0 && dh > 0, "raster pane {cols}x{rows}: nothing opaque was drawn");
            if dw < canvas.width() {
                let nearest = rms(
                    &image::imageops::resize(&canvas, dw, dh, image::imageops::FilterType::Nearest),
                    &ideal,
                );
                assert!(
                    err < 6.0 && err * 2.0 < nearest,
                    "raster pane {cols}x{rows}: the composite reached {dw}x{dh} with an RMS of \
                     {err:.3} against the single-resample ideal, where the Nearest shrink it \
                     used to get scores {nearest:.3}. A pane smaller than the composite must be \
                     ONE area-filtered shrink, not an identity copy followed by a Nearest one."
                );
            } else {
                assert_eq!(
                    err, 0.0,
                    "raster pane {cols}x{rows}: at or above native size the composite must be \
                     replicated pixel for pixel"
                );
            }
        }
    }

    // -- SQ-1081: a fractional magnification and the terminal's own resample -----

    /// Every press this sweep can put on screen, at its unit-screen size. Three real
    /// ones, because the pad is arithmetic about `native · s` against the CELL and a
    /// press whose height happens to divide the cell would prove nothing about the
    /// others (the 640x400 rows are the whole modern corpus, 480x304 the Macintosh's
    /// monochrome plate, 560x384 Arthur's Apple II and Journey r77).
    const PRESSES: [(u32, u32); 3] = [(640, 400), (480, 304), (560, 384)];

    /// The pixel dimensions a placed protocol DECLARES on the wire, read back off the
    /// bytes it wrote into the buffer rather than out of the encoder — `f=32` with
    /// `s=W,v=H` is `W · H · 4` bytes of RGBA, which is what [`measure_transmit`]
    /// counts. `None` when nothing was transmitted.
    fn wire_pixels(proto: &Protocol, dest: Rect) -> Option<u64> {
        let mut buf = Buffer::empty(Rect::new(0, 0, dest.right(), dest.bottom()));
        super::place_protocol(proto, dest, &mut buf)?;
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        let m = measure_transmit(&text);
        (m.uploads > 0).then_some(m.pixels)
    }

    /// **The composite fills the cells it is placed over, at every magnification.**
    ///
    /// A terminal scales a placement's image to the cell rectangle the placement
    /// names. `Picker::new_protocol` short-circuits its own resize the moment the
    /// pre-scaled image already fits, so the raster composite used to be handed over
    /// at `round(native · s)` pixels under a cell rect rounded UP — and the terminal
    /// made up the difference by resampling the whole frame. At the gallery's 16x32
    /// kitty cell over a 640x400 press that is a 1.013x vertical stretch at `s = 1.5`
    /// and a 1.011x one at `s = 1.9`, against artwork (see
    /// `machine-screenshots/amiga-journey.png`) that has no intermediate tones to
    /// reconstruct — which is the whole of "a fractional scale interpolates every edge
    /// in the frame", and why the 0.3 gallery had to pin its raster shots to a whole
    /// magnification.
    ///
    /// The claim is read OFF THE WIRE: the pixels the transmit declares must be exactly
    /// the pixels the placement covers. Both halves of the sweep matter — the rows
    /// where the pad is zero are the ones that must not have moved, and the rows where
    /// it is not are what the case is for. `a_whole_magnification_on_a_matched_cell_is_untouched`
    /// below pins the first half on its own.
    ///
    /// FALSIFY by returning `(img, (0, 0, iw, ih))` unconditionally from the magnifying
    /// branch of `v6_pad_to_cells`: every row whose guard reports a non-zero pad fails,
    /// declaring fewer pixels than its placement covers by exactly that pad.
    #[test]
    fn the_raster_composite_fills_the_cells_it_is_placed_over() {
        let mut padded_rows = 0usize;
        for (nw, nh) in PRESSES {
            let canvas = dithered_plate(nw, nh);
            for (fw, fh) in [(16u16, 32u16), (8, 16), (8, 18)] {
                let picker = super::kitty_picker(fw, fh);
                for (cols, rows) in [(82u16, 28u16), (60, 24), (76, 40), (100, 30), (122, 41), (92, 32)] {
                    let area = Rect::new(0, 0, cols, rows);
                    let (bw, bh) = (u32::from(cols) * u32::from(fw), u32::from(rows) * u32::from(fh));
                    let (raw, _) =
                        super::v6_fit_source(&canvas, bw, bh, None, super::v6_upscale_cap(&picker));
                    let (rw, rh) = raw.dimensions();
                    let ready = GraphicsRender::encode_v6(&picker, &canvas, 0, area, RasterFrame::native((canvas.width() as u16, canvas.height() as u16)), None)
                        .expect("kitty always builds a protocol");
                    let sz = ready.proto.size();
                    let (box_w, box_h) =
                        (u32::from(sz.width) * u32::from(fw), u32::from(sz.height) * u32::from(fh));
                    let dest = Rect::new(0, 0, sz.width, sz.height);
                    let got = wire_pixels(&ready.proto, dest).expect("kitty transmits its pixels");
                    assert_eq!(
                        got,
                        u64::from(box_w) * u64::from(box_h) * 4,
                        "{nw}x{nh} press, {fw}x{fh} cell, {cols}x{rows} pane: the composite is \
                         placed over {}x{} cells = {box_w}x{box_h} device pixels but declares \
                         {got} bytes of image. A placement whose image is smaller than its own \
                         cell rect is one the TERMINAL resamples — every edge in the frame \
                         interpolated, which is precisely what a fractional magnification must \
                         stop doing (SQ-1081).",
                        sz.width, sz.height
                    );
                    // Where the composite lies inside that box, and whether this row had
                    // anything to prove. `rw x rh` is the magnifying arm's own output;
                    // the shrinking arm hands the canvas over whole and the protocol
                    // resizes it, so only the pad is asserted there.
                    if rw <= bw && rh <= bh {
                        let (ox, oy) = ((box_w - rw) / 2, (box_h - rh) / 2);
                        assert_eq!(
                            ready.pic,
                            (ox, oy, rw, rh),
                            "{nw}x{nh} press, {fw}x{fh} cell, {cols}x{rows} pane: the click map \
                             inverts through the PICTURE, centred in the cells it was padded out to"
                        );
                        if (rw, rh) != (box_w, box_h) {
                            padded_rows += 1;
                        }
                    }
                }
            }
        }
        assert!(
            padded_rows >= 20,
            "non-vacuity: only {padded_rows} rows of this sweep landed off the cell grid, so the \
             case would pass with no padding at all. The sweep has to keep reaching the \
             magnifications the defect lives at."
        );
    }

    /// The other half, stated on its own so a regression cannot hide in an aggregate:
    /// **the pad is zero where the composite already lands on whole cells, so those
    /// frames are what they were.**
    ///
    /// That is every case the corpus is tested at and every raster frame the gallery
    /// ships: a 640x400 press at `s = 2` on the 16x32 cell chosen to match it
    /// (`gallery.toml`'s `8s x 16s` rule) reaches 1280x800, exactly 80x25 cells. It is
    /// NOT a property of the fraction — `s = 1.2` on an 8x16 cell reaches 768x480 and
    /// is exact too — it is a property of landing on the grid.
    #[test]
    fn a_whole_magnification_on_a_matched_cell_is_untouched() {
        use ratatui_image::FontSize;
        let canvas = dithered_plate(640, 400);
        for (fw, fh, cols, rows, want) in [
            (16u16, 32u16, 82u16, 28u16, (1280u32, 800u32)), // the gallery's own rung, s = 2
            (8, 16, 162, 53, (1280, 800)),                   // s = 2 on the ordinary cell
            (8, 16, 82, 25, (640, 400)),                     // s = 1
            (8, 16, 100, 30, (768, 480)),                    // s = 1.2, fractional and still exact
        ] {
            let fs = FontSize::new(fw, fh);
            let (bw, bh) = (u32::from(cols) * u32::from(fw), u32::from(rows) * u32::from(fh));
            let (raw, _) = super::v6_fit_source(&canvas, bw, bh, None, Some(super::MAX_V6_UPSCALE));
            assert_eq!(raw.dimensions(), want, "{fw}x{fh} cell, {cols}x{rows} pane: the pre-scale");
            let (padded, pic) = super::v6_pad_to_cells(raw.clone(), bw, bh, fs);
            assert_eq!(
                padded.as_raw(),
                raw.as_raw(),
                "{fw}x{fh} cell, {cols}x{rows} pane: {}x{} is whole cells already, so the image \
                 handed to the protocol must be byte for byte the one that was scaled",
                want.0, want.1
            );
            assert_eq!(pic, (0, 0, want.0, want.1), "…and it occupies all of them");
        }
    }

    /// The shrinking arm never had the defect and must not acquire one: the protocol
    /// still has a resize to do there, and `Resize::resize` pads its own output onto
    /// the cell grid. All [`v6_pad_to_cells`] does is say where the picture lands.
    ///
    /// It is the same claim as above read from the other side — the wire pixels in
    /// `the_raster_composite_fills_the_cells_it_is_placed_over` cover the shrinking
    /// panes too — but the PICTURE rect is what the click map reads, and inverting a
    /// click through the padded box instead of the picture was already wrong here
    /// before this quest existed.
    #[test]
    fn a_shrinking_pane_reports_the_picture_and_not_the_padded_box() {
        use ratatui_image::FontSize;
        let canvas = dithered_plate(640, 400);
        // 60x24 at 8x16: a 480x384 box, so the fit lands on 480x300 inside 60x19 cells
        // (480x304) — four rows of padding the protocol lays down itself.
        let (bw, bh) = (480u32, 384u32);
        let (raw, _) = super::v6_fit_source(&canvas, bw, bh, None, Some(super::MAX_V6_UPSCALE));
        assert_eq!(raw.dimensions(), (640, 400), "a pane below native size pre-scales nothing");
        let (out, pic) = super::v6_pad_to_cells(raw.clone(), bw, bh, FontSize::new(8, 16));
        assert_eq!(out.as_raw(), raw.as_raw(), "the canvas goes to the protocol untouched");
        assert_eq!(
            pic,
            (0, 0, 480, 300),
            "the picture inside the protocol's 480x304 box is the aspect-preserving fit, laid \
             down top-left — which is where `Resize::resize` puts it"
        );
    }

    // -- SQ-0964: the upscale cap is a budget, and half-blocks spends none ------

    /// Who the encode budget is charged to. Kitty (and sixel, and iTerm2) build and
    /// write encoded pixels for every composite; half-blocks resolves the image into
    /// terminal cells and encodes nothing at all.
    #[test]
    fn only_an_encoding_backend_spends_the_upscale_budget() {
        use ratatui_image::picker::Picker;
        assert_eq!(
            super::v6_upscale_cap(&Picker::halfblocks()),
            None,
            "half-blocks ships no encoded image, so there is no PNG budget for a cap to protect"
        );
        assert_eq!(
            super::v6_upscale_cap(&super::kitty_picker(8, 18)),
            Some(super::MAX_V6_UPSCALE),
            "kitty re-encodes the whole composite every time it changes - its ceiling stands"
        );
    }

    /// The fix itself: at one and the same pane, the two backends now magnify to
    /// DIFFERENT sizes, and that difference is the whole of SQ-0964.
    ///
    /// The pane is 200x60 cells at half-blocks' own nominal 10x20 - the grid a small
    /// terminal font gives you - so the box wants 3x out of a 640x400 composite where
    /// the cap allowed 2x. Below the cap the two agree exactly, which is the other half
    /// of the claim: nothing changed for a pane that never wanted the ceiling.
    ///
    /// FALSIFY by restoring the unconditional `.min(MAX_V6_UPSCALE)` in
    /// `v6_fit_source`: the two sizes become equal, which is the reported symptom - a
    /// finer grid that does not make the picture any bigger.
    #[test]
    fn halfblocks_climbs_past_the_encode_cap_and_kitty_does_not() {
        let canvas = dithered_plate(640, 400);
        // A box that wants 3x: 2000x1200 device pixels over a 640x400 composite.
        let (box_w, box_h) = (2000u32, 1200u32);
        let (free, _) = super::v6_fit_source(&canvas, box_w, box_h, None, None);
        let (capped, _) =
            super::v6_fit_source(&canvas, box_w, box_h, None, Some(super::MAX_V6_UPSCALE));
        assert_eq!(capped.dimensions(), (1280, 800), "the encoded backends stop at 2x, as before");
        assert_eq!(
            free.dimensions(),
            (1920, 1200),
            "half-blocks takes the whole 3x the box offers - the picture grows with the grid"
        );
        assert_ne!(
            free.dimensions(),
            capped.dimensions(),
            "the two backends must ANSWER DIFFERENTLY here; that difference is the fix"
        );
        // ...and the box is still the bound. Only the flat ceiling went.
        assert!(
            free.width() <= box_w && free.height() <= box_h,
            "an uncapped magnification is still a fit: {:?} must sit inside {box_w}x{box_h}",
            free.dimensions()
        );
        // Nearest, still: a 3x magnification replicates whole pixels and invents no
        // colour, exactly as `magnification_invents_no_colours` demands of the ring.
        let inks = |img: &RgbaImage| {
            img.pixels().map(|p| p.0).collect::<std::collections::HashSet<_>>().len()
        };
        assert_eq!(
            inks(&free),
            inks(&canvas),
            "uncapped magnification must stay nearest - a smoothing filter would show up as \
             hundreds of blends nobody painted"
        );
        // A pane under the ceiling never noticed it, and still does not.
        for (bw, bh) in [(1000u32, 800u32), (640, 400), (400, 300)] {
            let (a, _) = super::v6_fit_source(&canvas, bw, bh, None, None);
            let (b, _) = super::v6_fit_source(&canvas, bw, bh, None, Some(super::MAX_V6_UPSCALE));
            assert_eq!(
                a.dimensions(),
                b.dimensions(),
                "a {bw}x{bh} box wants less than {}x, so both backends must answer the same",
                super::MAX_V6_UPSCALE
            );
        }
    }

    /// The reported symptom, measured the way the player meets it: the terminal stays
    /// the same size and the FONT shrinks, so the pane gains cells. Half-blocks reports
    /// a fixed nominal 10x20 whatever the real font is (that is what the protocol is -
    /// one pixel per column, two per row), so more cells is more room, and the picture
    /// should fill the pane's short axis at every grid size.
    ///
    /// Under the cap it stops at 40 rows and the pane goes on growing around it, which
    /// is precisely "shrinking the font just makes the game window smaller".
    #[test]
    fn a_finer_cell_grid_grows_the_halfblocks_picture_and_the_capped_one_stalls() {
        use ratatui::layout::Size;
        use ratatui_image::FontSize;
        let canvas = dithered_plate(640, 400);
        let fs = FontSize::new(10, 20);
        let cells_of = |cap: Option<f64>, cols: u16, rows: u16| {
            let (box_w, box_h) = (u32::from(cols) * 10, u32::from(rows) * 20);
            let (src, fit) = super::v6_fit_source(&canvas, box_w, box_h, None, cap);
            fit.size_for(&image::DynamicImage::ImageRgba8(src), fs, Size::new(cols, rows))
        };
        let mut capped_rows = Vec::new();
        for (cols, rows) in [(100u16, 30u16), (140, 42), (200, 60)] {
            let free = cells_of(None, cols, rows);
            let capped = cells_of(Some(super::MAX_V6_UPSCALE), cols, rows);
            assert_eq!(
                free.height, rows,
                "{cols}x{rows}: uncapped, the composite fills the pane's short axis - a finer \
                 grid is more picture, which is the point of shrinking the font"
            );
            capped_rows.push(capped.height);
        }
        assert_eq!(
            capped_rows,
            vec![30u16, 40, 40],
            "capped, the composite stops at 40 rows and the pane grows around it: that stall \
             IS the defect, and it is what the encoded backends still (deliberately) do"
        );
    }

    /// SQ-0964 composes with SQ-0945's pixel lock rather than fighting it: with the lock
    /// on, the ladder still governs - half-blocks may simply climb higher up it.
    ///
    /// The pane is 2000x1150 device pixels, which would freely take 2.875x. `art_scale`
    /// (2, 2) puts the rungs on half-steps, so the lock quantizes down to 2.5x - and 2.5
    /// unit pixels per art pixel of 2 unit pixels is 5 whole device pixels, which is the
    /// entire point of the lock. Uncapped that rung is reached; capped, 2x is as far as
    /// the composite gets and a whole rung is left on the table.
    #[test]
    fn the_pixel_lock_ladder_still_governs_when_the_cap_is_gone() {
        use crate::render::v6_layout::FrameGeometry;
        let canvas = dithered_plate(640, 400);
        let (box_w, box_h) = (2000u32, 1150u32);
        let s = FrameGeometry::new((640, 400), (2, 2), zvm::screen::V6Cell::DEFAULT)
            .locked_scale((box_w, box_h))
            .expect("the pane holds a rung")
            .s;
        assert_eq!(s, 2.5, "the free 2.875x quantizes down to the ladder's 2.5x");
        let (free, _) = super::v6_fit_source(&canvas, box_w, box_h, Some(s), None);
        let (capped, _) =
            super::v6_fit_source(&canvas, box_w, box_h, Some(s), Some(super::MAX_V6_UPSCALE));
        assert_eq!(
            free.dimensions(),
            (1600, 1000),
            "half-blocks reaches the locked rung itself - 5 device pixels per art pixel"
        );
        assert_eq!(capped.dimensions(), (1280, 800), "the cap keeps the encoded backends at 2x");
        assert!(
            free.width() <= box_w && free.height() <= box_h,
            "a locked scale is still a scale the pane can hold"
        );
    }

    // -- SQ-0973: half-blocks resamples ONCE, and the pre-scale is gone ---------

    /// The sample grid a cell rect stands for: half-blocks resolves one sample per
    /// COLUMN and two per ROW, and `font_size` never enters into it.
    fn sample_grid(cells: Size) -> (u32, u32) {
        (u32::from(cells.width), u32::from(cells.height) * 2)
    }

    /// What a half-blocks composite actually puts ON SCREEN, read back out of the
    /// terminal buffer: two samples per cell, the upper half and the lower half, in the
    /// grid order they occupy. This is the only honest place to measure a backend that
    /// resolves into cells rather than encoding pixels — and it needs no guess about
    /// which branch of the crate's `needs_resize` a call took.
    fn screen_grid(proto: &Protocol, cells: Size) -> RgbaImage {
        let rect = Rect::new(0, 0, cells.width, cells.height);
        let mut buf = Buffer::empty(rect);
        Image::new(proto).render(rect, &mut buf);
        let (gw, gh) = sample_grid(cells);
        let mut out = RgbaImage::new(gw, gh);
        for y in 0..cells.height {
            for x in 0..cells.width {
                let cell = buf.cell((x, y)).expect("a cell");
                // `pick_side` swaps the halves and flips the glyph when the lower is the
                // brighter of the two; the glyph says which way round they are.
                let (upper, lower) =
                    if cell.symbol() == "\u{2584}" { (cell.bg, cell.fg) } else { (cell.fg, cell.bg) };
                for (half, colour) in [(0u32, upper), (1, lower)] {
                    let Color::Rgb(r, g, b) = colour else { panic!("half-blocks emits Rgb cells") };
                    out.put_pixel(u32::from(x), u32::from(y) * 2 + half, Rgba([r, g, b, 255]));
                }
            }
        }
        out
    }

    /// The composite the OLD path put on screen: pre-scale to the pane's device pixels,
    /// then hand that to the crate, which resamples it back down to the sample grid.
    fn double_resampled(canvas: &RgbaImage, picker: &Picker, cols: u16, rows: u16) -> (RgbaImage, Size) {
        let fs = picker.font_size();
        let (box_w, box_h) =
            (u32::from(cols) * u32::from(fs.width), u32::from(rows) * u32::from(fs.height));
        let (pre, fit) = super::v6_fit_source(canvas, box_w, box_h, None, super::v6_upscale_cap(picker));
        let proto = picker
            .new_protocol(image::DynamicImage::ImageRgba8(pre), Size::new(cols, rows), fit)
            .expect("the old path encodes");
        let cells = proto.size();
        (screen_grid(&proto, cells), cells)
    }

    /// Nothing on screen moved: [`super::v6_halfblocks_grid`] is the cell rect the old
    /// pre-scale-then-`Fit` pair landed on, in every branch it has — free magnification,
    /// free shrink, and a locked rung — over a pane sweep and two font sizes.
    ///
    /// This is the "agree where they must" half of SQ-0973. The composite's relationship
    /// to the pane is what the click map (`V6ClickMap`, built from `proto.size()`) and the
    /// letterbox centring in `redraw_v6` are both derived from, so a cell rect that moved
    /// would be a geometry change wearing a performance fix's clothes.
    #[test]
    fn v6_halfblocks_grid_matches_the_protocol() {
        use ratatui_image::FontSize;
        let canvas = dithered_plate(640, 400);
        for fs in [FontSize::new(10, 20), FontSize::new(8, 18)] {
            for (cols, rows) in
                [(458u16, 144u16), (200, 60), (100, 40), (240, 80), (60, 24), (40, 10), (20, 4)]
            {
                let (box_w, box_h) =
                    (u32::from(cols) * u32::from(fs.width), u32::from(rows) * u32::from(fs.height));
                for lock in [None, Some(2.5f32), Some(1.0)] {
                    let (pre, fit) = super::v6_fit_source(&canvas, box_w, box_h, lock, None);
                    let want =
                        fit.size_for(&image::DynamicImage::ImageRgba8(pre), fs, Size::new(cols, rows));
                    let got = super::v6_halfblocks_grid(canvas.dimensions(), box_w, box_h, fs, lock);
                    assert_eq!(
                        got, want,
                        "{cols}x{rows} at {}x{} font, lock {lock:?}: the arithmetic must land on \
                         the cell rect the pre-scale-then-Fit pair landed on, or the composite \
                         has quietly changed size and the click map with it",
                        fs.width, fs.height
                    );
                    assert!(
                        got.width <= cols && got.height <= rows,
                        "{cols}x{rows}: the pane is still the bound, got {got:?}"
                    );
                }
            }
        }
    }

    /// The regression this quest is about, pinned so it cannot come back quietly: the
    /// half-blocks composite is ONE resample from the native canvas onto the sample
    /// grid, and the crate's own `resize_exact` on top of it is a bit-exact identity.
    ///
    /// The pin is the reference image — `resize_directional(canvas, w, 2h)` and nothing
    /// else. Restore the pre-scale (hand `v6_fit_source`'s output to `new_protocol` on
    /// the half-blocks arm of `encode_v6`) and the rendered cells stop matching it,
    /// because Nearest-up-then-Triangle-down is not the same picture as one Triangle
    /// down — as the RMS half below measures.
    #[test]
    fn the_halfblocks_composite_is_one_resample_onto_the_sample_grid() {
        use ratatui_image::protocol::halfblocks::Halfblocks;
        let picker = Picker::halfblocks();
        let fs = picker.font_size();
        let canvas = dithered_plate(640, 400);
        // The pane the defect was reported at, plus a magnifying one and a coarse one.
        for (cols, rows) in [(458u16, 144u16), (200, 60), (60, 24)] {
            let area = Rect::new(0, 0, cols, rows);
            let (box_w, box_h) =
                (u32::from(cols) * u32::from(fs.width), u32::from(rows) * u32::from(fs.height));
            let cells = super::v6_halfblocks_grid(canvas.dimensions(), box_w, box_h, fs, None);
            let (gw, gh) = sample_grid(cells);

            let ready = GraphicsRender::encode_v6(&picker, &canvas, 1, area, RasterFrame::native((canvas.width() as u16, canvas.height() as u16)), None).expect("encode");
            assert_eq!(ready.proto.size(), cells, "{cols}x{rows}: the protocol reports the grid");

            // One resample, straight from the canvas — then the crate, whose resample
            // at 1:1 cannot change a pixel.
            let once = resize_directional(&canvas, gw, gh);
            let reference = Protocol::Halfblocks(
                Halfblocks::new(image::DynamicImage::ImageRgba8(once), cells).expect("reference"),
            );
            let rect = Rect::new(0, 0, cells.width, cells.height);
            let (mut a, mut b) = (Buffer::empty(rect), Buffer::empty(rect));
            Image::new(&ready.proto).render(rect, &mut a);
            Image::new(&reference).render(rect, &mut b);
            assert_eq!(
                a, b,
                "{cols}x{rows}: the shipped composite must BE a single {gw}x{gh} resample of \
                 the canvas — any pre-scale in between shows up here as different cells"
            );
        }
    }

    /// …and the reason the filter choice survives: `Halfblocks::encode` resamples
    /// whatever it is given to `width x 2·height` with Triangle, unconditionally and
    /// ignoring `font_size` — so handing it exactly that size makes its resample a
    /// 1:1 identity, and what reaches the screen is [`resize_directional`]'s answer
    /// rather than the crate's.
    #[test]
    fn the_crates_own_resample_is_an_identity_on_the_sample_grid() {
        use ratatui_image::protocol::halfblocks::Halfblocks;
        let cells = Size::new(97, 31);
        let (gw, gh) = sample_grid(cells);
        let grid = dithered_plate(gw, gh);
        let rect = Rect::new(0, 0, cells.width, cells.height);

        let proto = Protocol::Halfblocks(
            Halfblocks::new(image::DynamicImage::ImageRgba8(grid.clone()), cells).expect("encode"),
        );
        let mut buf = Buffer::empty(rect);
        Image::new(&proto).render(rect, &mut buf);

        // Every cell's two halves must be the two grid rows above and below it, exactly.
        // (`pick_side` may swap fg/bg and flip the glyph; the PAIR is what is pinned.)
        for y in 0..cells.height {
            for x in 0..cells.width {
                let cell = buf.cell((x, y)).expect("a cell");
                let px = |row: u32| {
                    let p = grid.get_pixel(u32::from(x), row).0;
                    Color::Rgb(p[0], p[1], p[2])
                };
                let (upper, lower) = (px(u32::from(y) * 2), px(u32::from(y) * 2 + 1));
                let got = if cell.symbol() == "\u{2584}" {
                    (cell.bg, cell.fg)
                } else {
                    (cell.fg, cell.bg)
                };
                assert_eq!(
                    got,
                    (upper, lower),
                    "cell ({x},{y}) must carry grid rows {} and {} untouched — a resample at \
                     1:1 has to be an identity, or the second pass is still there",
                    u32::from(y) * 2,
                    u32::from(y) * 2 + 1
                );
            }
        }
    }

    /// Quality, measured rather than claimed — and measured on what reaches the screen,
    /// through [`screen_grid`], because half-blocks has no encoded image to compare.
    ///
    /// The pair is never better than the single pass, and how much worse it is depends
    /// entirely on whether its magnification happened to be an integer. Measured against
    /// the per-axis single-resample ideal on a 640x400 dithered plate:
    ///
    /// | pane | sample grid | one resample | the old pair |
    /// |---|---|---|---|
    /// | 458x144 (the reported one) | 458x288 | 2.472 | 3.615 |
    /// | 200x60 | 192x120 | 3.884 | 4.006 |
    /// | 60x24 | 60x38 | 4.056 | 16.372 |
    ///
    /// 200x60 is the honest middle: the pre-scale there is an exact 3x Nearest
    /// replication, which composes cleanly with the shrink that follows it, so the pair
    /// lands almost where one pass does and the win is purely the 8.79 MB it built to
    /// get there. 458x144 magnifies by 7.16x, an integer ratio it is not, so every edge
    /// is quantized onto a ragged replication grid before being averaged — and 60x24
    /// never magnified at all: the pair is a 0.94x Triangle followed by a 0.1x Triangle
    /// over a transparent pad row, which is the blur those numbers say it is.
    #[test]
    fn one_resample_beats_the_pair_against_the_ideal() {
        let picker = Picker::halfblocks();
        let canvas = dithered_plate(640, 400);
        for (cols, rows) in [(458u16, 144u16), (200, 60), (60, 24)] {
            let area = Rect::new(0, 0, cols, rows);
            let ready = GraphicsRender::encode_v6(&picker, &canvas, 1, area, RasterFrame::native((canvas.width() as u16, canvas.height() as u16)), None).expect("encode");
            let cells = ready.proto.size();
            let (gw, gh) = sample_grid(cells);
            let ideal = ideal(&canvas, gw, gh);

            let once = rms(&screen_grid(&ready.proto, cells), &ideal);
            let (old, old_cells) = double_resampled(&canvas, &picker, cols, rows);
            assert_eq!(old_cells, cells, "{cols}x{rows}: the same cell rect, either way");
            let twice = rms(&old, &ideal);
            eprintln!("{cols}x{rows} -> {gw}x{gh}: once {once:.3}, twice {twice:.3}");
            assert!(
                once < 4.5 && once < twice,
                "{cols}x{rows} -> a {gw}x{gh} sample grid: one resample scores an RMS of \
                 {once:.3} against the single-resample ideal where the old pair scores \
                 {twice:.3}. The single pass must stay under 4.5 and must never be the \
                 WORSE of the two — see the table above for what was measured."
            );
        }
        // …and at the pane the defect was reported at, by a margin worth having.
        let area = Rect::new(0, 0, 458, 144);
        let ready = GraphicsRender::encode_v6(&picker, &canvas, 1, area, RasterFrame::native((canvas.width() as u16, canvas.height() as u16)), None).expect("encode");
        let cells = ready.proto.size();
        let ideal = ideal(&canvas, sample_grid(cells).0, sample_grid(cells).1);
        let once = rms(&screen_grid(&ready.proto, cells), &ideal);
        let twice = rms(&double_resampled(&canvas, &picker, 458, 144).0, &ideal);
        assert!(
            once * 1.3 < twice,
            "458x144: {once:.3} against {twice:.3} — a 7.16x pre-scale is not an integer \
             ratio, so dropping it has to buy more than a rounding difference"
        );
    }

    /// The blast radius, stated as a test: half-blocks takes the new arm and every
    /// backend that actually encodes pixels is byte-for-byte where SQ-0964 left it.
    ///
    /// This is the "differ where they should" half. Kitty's composite is still the
    /// capped pre-scale handed to `new_protocol` under `Resize::Fit`, because kitty
    /// ships those pixels down the wire and the cap is a budget it genuinely spends.
    #[test]
    fn only_halfblocks_leaves_the_prescale_behind() {
        let area = Rect::new(0, 0, 200, 60);
        let canvas = dithered_plate(640, 400);
        for picker in [kitty_picker(10, 20), {
            let mut p = Picker::halfblocks();
            p.set_protocol_type(ratatui_image::picker::ProtocolType::Sixel);
            p
        }] {
            let fs = picker.font_size();
            let (box_w, box_h) =
                (u32::from(area.width) * u32::from(fs.width), u32::from(area.height) * u32::from(fs.height));
            let (img, fit) =
                super::v6_fit_source(&canvas, box_w, box_h, None, super::v6_upscale_cap(&picker));
            let want = picker
                .new_protocol(image::DynamicImage::ImageRgba8(img), Size::new(area.width, area.height), fit)
                .expect("encode")
                .size();
            let got = GraphicsRender::encode_v6(&picker, &canvas, 1, area, RasterFrame::native((canvas.width() as u16, canvas.height() as u16)), None).expect("encode");
            assert_eq!(
                got.proto.size(),
                want,
                "{:?} still goes through v6_fit_source + new_protocol, unchanged",
                picker.protocol_type()
            );
            assert_eq!(
                want,
                Size::new(128, 40),
                "{:?}: and that is still the 2x cap, 1280x800 device pixels",
                picker.protocol_type()
            );
        }
    }

    // ── SQ-0827: the seam where art ends and the canvas is clear ────────────────

    /// A flank in the shape Zork Zero's is: a column of art, then the story page it
    /// abuts, then CLEAR canvas past the crop's inner edge. The alpha step is the whole
    /// specimen — the art either side of it is flat, so anything dark that comes out of
    /// the resample was invented by the filter.
    fn flank_with_a_clear_edge(w: u32, h: u32, art_to: u32, page_to: u32) -> RgbaImage {
        let mut img = RgbaImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let p = if x < art_to {
                    Rgba([0x44, 0x00, 0x00, 0xff]) // the pillar
                } else if x < page_to {
                    Rgba([0xad, 0xad, 0xad, 0xff]) // the game's page, opaque
                } else {
                    Rgba([0, 0, 0, 0]) // clear: the cell background shows through
                };
                img.put_pixel(x, y, p);
            }
        }
        img
    }

    /// What a terminal puts on screen for one emitted pixel: straight-alpha compositing
    /// over the cell background behind the band.
    fn over(px: Rgba<u8>, bg: [u8; 3]) -> [u8; 3] {
        let a = f64::from(px.0[3]) / 255.0;
        let mut out = [0u8; 3];
        for c in 0..3 {
            out[c] = (f64::from(px.0[c]) * a + f64::from(bg[c]) * (1.0 - a)).round() as u8;
        }
        out
    }

    /// SQ-0827, the reported symptom: a one-pixel darker column down the story pane's
    /// edge. Where the flank's opaque page meets clear canvas, no emitted pixel may
    /// composite darker than the page it is part of — the band is drawn OVER a cell
    /// flooded with that same page, so a correct seam is invisible.
    ///
    /// FALSIFY by dropping the `associate_alpha`/`unassociate_alpha` pair from
    /// `resize_directional`: the seam pixel comes back as `(38,38,38,57)`, which over
    /// the page draws 142 against 173, and this fails naming that column.
    #[test]
    fn a_shrinking_band_leaves_no_dark_fringe_where_its_art_meets_clear_canvas() {
        const PAGE: [u8; 3] = [0xad, 0xad, 0xad];
        // 95 native columns into an 84px band is Zork Zero's own ratio on the Amiga
        // floppy at an 83-column terminal; the rest sweep the regime either side of it.
        for (nw, bw) in [(95u32, 84u32), (94, 70), (95, 91), (128, 64), (100, 99)] {
            let src = flank_with_a_clear_edge(nw, 40, nw * 3 / 4, nw - 7);
            let got = resize_directional(&src, bw, 34);
            let y = got.height() / 2;
            for x in 0..got.width() {
                let px = *got.get_pixel(x, y);
                let seen = over(px, PAGE);
                assert!(
                    // Only a PARTLY TRANSPARENT pixel is judged: an opaque one that
                    // reads darker is the pillar itself, which is meant to be dark.
                    // The one level of slack is the 8-bit premultiply round trip — the
                    // page's own 173 comes back as 172 — against the 31 levels the
                    // reported line was worth.
                    px.0[3] == 0xff || (0..3).all(|c| seen[c] + 1 >= PAGE[c]),
                    "{nw}->{bw}: emitted pixel x={x} is {:?}, which over the page behind the \
                     band draws {seen:?} against the page's own {PAGE:?} — a partly \
                     transparent pixel whose colour was averaged with the (0,0,0) of clear \
                     canvas IS the one-pixel dark line down the story pane's edge",
                    px.0
                );
            }
        }
    }

    /// …and the same specimen at the same ratios under a GROWING axis, which takes the
    /// Nearest arm and must not pay for the conversion at all: every output pixel is a
    /// source pixel, alpha included.
    #[test]
    fn a_growing_band_still_replicates_whole_pixels_across_an_alpha_edge() {
        let src = flank_with_a_clear_edge(84, 20, 60, 77);
        let got = resize_directional(&src, 95, 30);
        let inks: std::collections::HashSet<_> = got.pixels().map(|p| p.0).collect();
        assert_eq!(
            inks,
            src.pixels().map(|p| p.0).collect::<std::collections::HashSet<_>>(),
            "a magnifying pass invents no colour, so an alpha edge stays a step"
        );
    }

    /// The reason the conversion is free where it is not needed, and the reason every
    /// RMS figure in this module is untouched by it: on FULLY OPAQUE art it is the
    /// identity, bit for bit. Journey's canyon plate is exactly that.
    #[test]
    fn associating_a_fully_opaque_plate_is_the_identity() {
        let src = dithered_plate(222, 254);
        assert!(src.pixels().all(|p| p.0[3] == 0xff), "the specimen must be opaque to prove this");
        assert_eq!(super::associate_alpha(&src).as_raw(), src.as_raw(), "premultiply by 1");
        let mut back = src.clone();
        super::unassociate_alpha(&mut back);
        assert_eq!(back.as_raw(), src.as_raw(), "and divide by 1");
        // Both axes shrink: the single-pass Triangle arm.
        for (tw, th) in [(200u32, 234u32), (168, 198)] {
            assert_eq!(
                resize_directional(&src, tw, th).as_raw(),
                image::imageops::resize(&src, tw, th, image::imageops::FilterType::Triangle)
                    .as_raw(),
                "222x254 -> {tw}x{th}: opaque art resamples exactly as it did before SQ-0827"
            );
        }
        // …and the mixed arm, whose two passes are equally untouched.
        let mid = image::imageops::resize(&src, 212, 254, image::imageops::FilterType::Triangle);
        assert_eq!(
            resize_directional(&src, 212, 256).as_raw(),
            image::imageops::resize(&mid, 212, 256, image::imageops::FilterType::Nearest).as_raw(),
            "222x254 -> 212x256 shrinks on x and grows on y, and is unchanged too"
        );
    }

    /// The band log names the direction, because a cell rect never could.
    #[test]
    fn the_band_log_names_the_direction() {
        assert_eq!(super::resample_note(222, 254, 200, 234), "resample 222x254->200x234 x:area y:area");
        assert_eq!(
            super::resample_note(222, 254, 212, 256),
            "resample 222x254->212x256 x:area y:nearest"
        );
        assert_eq!(
            super::resample_note(222, 254, 328, 378),
            "resample 222x254->328x378 x:nearest y:nearest"
        );
    }

    // ── SQ-0979: the four FITTED sites resample once under half-blocks too ──────
    //
    // `fit_for_protocol` + `new_protocol(.., Resize::Fit(None))` was written out at
    // four call sites — Glulx graphics windows, the picker's cover panel, the gallery
    // tiles and the resource preview — and on half-blocks that pair pre-scales to
    // device pixels the backend immediately throws away. `fitted_protocol` is the one
    // call all four now make; on every encoding backend it IS the pair.

    /// A PHOTOGRAPH, which is what cover art and IFDB jackets are: smooth tonal ramps
    /// with grain on top, no palette and no hard edges.
    ///
    /// It fails differently from `dithered_plate`, and that is why both are swept here.
    /// A second resample costs pixel art its colours and its edges — the failure SQ-0824
    /// measured — while what it costs a photograph is CONTRAST: the grain and the fine
    /// tonal steps fuse into flatness, and nothing about the colour count says so.
    fn photograph(w: u32, h: u32) -> RgbaImage {
        let mut img = RgbaImage::new(w, h);
        // A deterministic LCG, so the grain is the same on every run and on every
        // machine — a quality bound measured against a random image is not a bound.
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut noise = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) & 0xff) as i32 - 128
        };
        for y in 0..h {
            for x in 0..w {
                let (fx, fy) = (x as f64 / w as f64, y as f64 / h as f64);
                // Two overlapping lobes and a sky ramp: the low-frequency content a
                // jacket scan has, before the grain.
                let r = 200.0 * (1.0 - fy) + 40.0 * ((fx * 9.0).sin() * 0.5 + 0.5);
                let g = 120.0 + 90.0 * ((fx * 5.0 + fy * 3.0).cos() * 0.5 + 0.5);
                let b = 60.0 + 160.0 * fy * (1.0 - 0.6 * fx);
                let px = [r, g, b].map(|c| {
                    (c + f64::from(noise()) * 0.10).round().clamp(0.0, 255.0) as u8
                });
                img.put_pixel(x, y, Rgba([px[0], px[1], px[2], 0xff]));
            }
        }
        img
    }

    /// The fitted picture's extent inside the padded pixel image the OTHER path builds,
    /// read off the padding rather than recomputed: [`fit_for_protocol`] lays the
    /// picture down top-left and leaves the remainder fully transparent, so on an opaque
    /// source the opaque region IS the fit. Independent of the half-blocks arm's own
    /// arithmetic, which is the point — that arithmetic is what is under test.
    fn opaque_extent(img: &RgbaImage) -> (u32, u32) {
        let (w, h) = img.dimensions();
        let cols = (0..w).filter(|&x| (0..h).any(|y| img.get_pixel(x, y).0[3] != 0)).count() as u32;
        let rows = (0..h).filter(|&y| (0..w).any(|x| img.get_pixel(x, y).0[3] != 0)).count() as u32;
        (cols.max(1), rows.max(1))
    }

    /// The composite the OLD pair put on screen at one of these sites, and the cell rect
    /// it reported: the device-pixel pre-scale handed to the crate, which resamples it
    /// back down to the sample grid.
    fn fit_pair(
        picker: &Picker,
        img: &image::DynamicImage,
        target: Size,
        upscale: bool,
    ) -> (Protocol, Size, (u32, u32)) {
        let (pre, size) = super::fit_for_protocol(picker, img, target, upscale);
        let extent = opaque_extent(&pre.to_rgba8());
        let (pw, ph) = (pre.width(), pre.height());
        let proto = picker.new_protocol(pre, size, Resize::Fit(None)).expect("the old pair encodes");
        // The extent, carried into sample space: a cell is one sample wide and two tall.
        let (gw, gh) = sample_grid(size);
        let map = |v: u32, from: u32, to: u32| {
            ((f64::from(v) * f64::from(to) / f64::from(from.max(1))).round() as u32).clamp(1, to)
        };
        (proto, size, (map(extent.0, pw, gw), map(extent.1, ph, gh)))
    }

    /// Half-blocks and the encoding backends put the picture in exactly the same CELLS.
    ///
    /// This is the "agree where they must" half. `Protocol::size()` is what every one of
    /// the four sites centres against — `GraphicsRender::render`'s letterbox, the
    /// resource preview's, and the gallery's own `fitted_tile_rect` — so a cell rect
    /// that moved when the backend changed would be a layout change wearing a
    /// performance fix's clothes. Swept over both `upscale` modes, three font sizes,
    /// and sources on either side of their box.
    #[test]
    fn fitted_cells_match_the_prescale() {
        // Aspects, not megapixels: the arithmetic is a ratio and a ceil, and a sweep of
        // full-size jacket scans through a sixel ENCODER is minutes of test time for
        // nothing. `1104x36` is advent.blb's toolbar (SQ-0829) and `24x15` a canvas
        // small enough that `upscale` has somewhere to go.
        let sources =
            [(240u32, 279u32), (150, 200), (320, 200), (32, 32), (1104, 36), (24, 15)];
        for (sw, sh) in sources {
            let src = image::DynamicImage::ImageRgba8(dithered_plate(sw, sh));
            for fs in [ratatui_image::FontSize::new(10, 20), ratatui_image::FontSize::new(8, 18)] {
                let mk = |proto| {
                    #[allow(deprecated)]
                    let mut p = Picker::from_fontsize(fs);
                    p.set_protocol_type(proto);
                    p
                };
                let hb = mk(ratatui_image::picker::ProtocolType::Halfblocks);
                let sx = mk(ratatui_image::picker::ProtocolType::Sixel);
                for (cols, rows) in [(20u16, 11u16), (24, 20), (40, 30), (5, 3), (1, 1)] {
                    for upscale in [false, true] {
                        let target = Size::new(cols, rows);
                        let want = super::fit_for_protocol(&hb, &src, target, upscale).1;
                        let got = super::fitted_protocol(&hb, &src, target, upscale)
                            .expect("half-blocks builds")
                            .size();
                        assert_eq!(
                            got, want,
                            "{sw}x{sh} into {cols}x{rows} at {}x{} font, upscale {upscale}: \
                             half-blocks must report the cell rect the pre-scale landed on",
                            fs.width, fs.height
                        );
                        assert!(
                            got.width <= cols && got.height <= rows,
                            "{sw}x{sh} into {cols}x{rows}: the box is still the bound, got {got:?}"
                        );
                        let enc = super::fitted_protocol(&sx, &src, target, upscale)
                            .expect("sixel builds")
                            .size();
                        assert_eq!(
                            enc, want,
                            "{sw}x{sh} into {cols}x{rows}: and so does every backend that encodes"
                        );
                    }
                }
            }
        }
        // …and once at the size a real jacket arrives at, half-blocks only.
        let big = image::DynamicImage::ImageRgba8(dithered_plate(1200, 1600));
        let hb = Picker::halfblocks();
        for (cols, rows) in [(20u16, 11u16), (24, 20)] {
            let target = Size::new(cols, rows);
            assert_eq!(
                super::fitted_protocol(&hb, &big, target, false).expect("builds").size(),
                super::fit_for_protocol(&hb, &big, target, false).1,
                "a 1200x1600 jacket into {cols}x{rows}"
            );
        }
    }

    /// The regression this quest is about, pinned so it cannot come back quietly: what
    /// half-blocks draws is ONE resample of the source onto its own sample grid, with
    /// the padding [`fit_for_protocol`] would have left still where it was.
    ///
    /// The reference is built from the source and nothing else — `resize_directional`
    /// once, onto the extent read out of the OTHER path's padded image. Restore the pair
    /// (hand `fit_for_protocol`'s output to `new_protocol` in `fitted_protocol`) and the
    /// rendered cells stop matching it, because Triangle-down-then-Triangle-down is not
    /// the same picture as one pass down — as the RMS case below measures.
    #[test]
    fn a_fitted_halfblocks_picture_is_one_resample_onto_the_sample_grid() {
        use ratatui_image::protocol::halfblocks::Halfblocks;
        let picker = Picker::halfblocks();
        for (label, src) in
            [("photograph", photograph(640, 744)), ("pixel art", dithered_plate(320, 200))]
        {
            let dyn_src = image::DynamicImage::ImageRgba8(src.clone());
            for (cols, rows, upscale) in
                [(20u16, 11u16, false), (24, 20, false), (100, 40, true), (100, 40, false)]
            {
                let target = Size::new(cols, rows);
                let (_, cells, (sx, sy)) = fit_pair(&picker, &dyn_src, target, upscale);
                let (gw, gh) = sample_grid(cells);
                let shipped = super::fitted_protocol(&picker, &dyn_src, target, upscale)
                    .expect("half-blocks builds");
                assert_eq!(shipped.size(), cells, "{label} {cols}x{rows}: the grid it reports");

                let once = resize_directional(&src, sx, sy);
                let grid = if (sx, sy) == (gw, gh) {
                    once
                } else {
                    let mut padded = RgbaImage::new(gw, gh);
                    image::imageops::replace(&mut padded, &once, 0, 0);
                    padded
                };
                let reference = Protocol::Halfblocks(
                    Halfblocks::new(image::DynamicImage::ImageRgba8(grid), cells)
                        .expect("reference"),
                );
                let rect = Rect::new(0, 0, cells.width, cells.height);
                let (mut a, mut b) = (Buffer::empty(rect), Buffer::empty(rect));
                Image::new(&shipped).render(rect, &mut a);
                Image::new(&reference).render(rect, &mut b);
                assert_eq!(
                    a, b,
                    "{label} into {cols}x{rows} (upscale {upscale}): what reaches the screen \
                     must BE a single {sx}x{sy} resample of the source on a {gw}x{gh} grid — \
                     any pre-scale in between shows up here as different cells"
                );
            }
        }
    }

    /// Quality, measured rather than claimed, and measured on both KINDS of content
    /// because they fail differently.
    ///
    /// Judged over the picture's own sample extent (the padding is not picture) against
    /// the per-axis single-resample ideal. Measured, RMS, one pass against the pair:
    ///
    /// | content | site | picture grid | one resample | the old pair |
    /// |---|---|---|---|---|
    /// | photograph 640x744 | gallery tile 20x11 | 19x22 | 0.480 | 2.249 |
    /// | photograph 640x744 | cover panel 24x20 | 24x28 | 0.434 | 1.797 |
    /// | photograph 640x744 | window 100x40, upscale | 69x80 | 0.579 | 2.554 |
    /// | pixel art 320x200 | gallery tile 20x11 | 20x13 | 4.674 | 31.233 |
    /// | pixel art 320x200 | cover panel 24x20 | 24x15 | 4.482 | 7.994 |
    /// | pixel art 320x200 | window 100x40, upscale | 100x63 | 3.996 | 14.808 |
    ///
    /// The photograph's absolute numbers are small either way — there is no palette to
    /// lose and no hard edge to ragged — and the pair still costs it four to five times
    /// the error, which is the fine tonal detail and grain a second pass fuses. Pixel art
    /// is where it is loud: a 16x reduction of a dithered plate through a device-pixel
    /// intermediate scores 31.2 against 4.7, because the pre-scale quantizes the dither
    /// onto an intermediate grid that has nothing to do with the one drawn, and the
    /// second pass then averages THAT.
    #[test]
    fn one_resample_beats_the_pair_at_the_fitted_sites() {
        let picker = Picker::halfblocks();
        let crop = |img: &RgbaImage, w: u32, h: u32| {
            image::imageops::crop_imm(img, 0, 0, w, h).to_image()
        };
        for (label, src) in
            [("photograph", photograph(640, 744)), ("pixel art", dithered_plate(320, 200))]
        {
            let dyn_src = image::DynamicImage::ImageRgba8(src.clone());
            for (cols, rows, upscale) in [(20u16, 11u16, false), (24, 20, false), (100, 40, true)] {
                let target = Size::new(cols, rows);
                let (pair, cells, (sx, sy)) = fit_pair(&picker, &dyn_src, target, upscale);
                let shipped = super::fitted_protocol(&picker, &dyn_src, target, upscale)
                    .expect("half-blocks builds");
                let ideal = ideal(&src, sx, sy);
                let once = rms(&crop(&screen_grid(&shipped, cells), sx, sy), &ideal);
                let twice = rms(&crop(&screen_grid(&pair, cells), sx, sy), &ideal);
                eprintln!(
                    "{label} into {cols}x{rows} (upscale {upscale}) -> {sx}x{sy}: \
                     once {once:.3}, twice {twice:.3}"
                );
                assert!(
                    once < twice && once < 5.0,
                    "{label} into {cols}x{rows} (upscale {upscale}), a {sx}x{sy} extent: one \
                     resample scores an RMS of {once:.3} against the single-resample ideal \
                     where the old pair scores {twice:.3}. The single pass must stay under \
                     5.0 and must never be the WORSE of the two — see the table above."
                );
            }
        }
    }

    /// SQ-0829's own specimen, on the grid half-blocks draws: advent.blb's 1104-px
    /// toolbar in a narrow pane. The guarantee that quest exists for is that a canvas
    /// LARGER than its window is minified by an area average and not by dropping
    /// columns, and moving the resample off device pixels must not quietly hand it back.
    ///
    /// Column-dropping is measured here rather than assumed — Nearest onto the same grid
    /// — because "we call `resize_directional`" is a claim about the source, and this is
    /// a claim about the screen.
    #[test]
    fn a_toolbar_wider_than_its_pane_is_averaged_on_the_sample_grid_too() {
        let picker = Picker::halfblocks();
        let src = dithered_plate(1104, 36);
        let dyn_src = image::DynamicImage::ImageRgba8(src.clone());
        for (cols, rows) in [(40u16, 3u16), (80, 4)] {
            let target = Size::new(cols, rows);
            let proto = super::fitted_protocol(&picker, &dyn_src, target, false).expect("builds");
            let cells = proto.size();
            let (_, pair_cells, (sx, sy)) = fit_pair(&picker, &dyn_src, target, false);
            assert_eq!(cells, pair_cells);
            assert!(sx < 1104, "{cols}x{rows}: the point is a MINIFICATION, got {sx} columns");
            let got = image::imageops::crop_imm(&screen_grid(&proto, cells), 0, 0, sx, sy).to_image();
            let ideal = ideal(&src, sx, sy);
            let dropped = image::imageops::resize(&src, sx, sy, image::imageops::FilterType::Nearest);
            let (mine, near) = (rms(&got, &ideal), rms(&dropped, &ideal));
            assert!(
                mine * 2.0 < near,
                "{cols}x{rows} -> {sx}x{sy}: the toolbar scores {mine:.2} against the area \
                 average, column-dropping {near:.2} — SQ-0829's guarantee has to hold on \
                 the grid that is drawn, not only on the device pixels that are not"
            );
        }
    }

    /// `upscale` is the one thing this path has that the v6 composite does not, and both
    /// of its states must survive: `true` blows a small canvas up to fill its window
    /// (a Scott room picture), `false` leaves it at native size for the caller to centre.
    ///
    /// The flag decides the CELL RECT and nothing else — the sample grid follows from the
    /// rect — so the case pins that the two rects still differ, that each is the one the
    /// pre-scale reached, and that the magnified one gained no colours it did not have.
    /// Replication is what makes a blown-up room picture crisp, and it is the axis
    /// direction on the SAMPLE grid that decides it, not the direction in device pixels:
    /// 320x200 into a 100x40-cell window magnifies 3.1x across the screen while
    /// SHRINKING onto the 100x64 grid that is drawn.
    #[test]
    fn an_upscaled_window_and_a_native_one_both_reach_their_own_grid() {
        let picker = Picker::halfblocks();
        let src = dithered_plate(320, 200);
        let dyn_src = image::DynamicImage::ImageRgba8(src.clone());
        let target = Size::new(100, 40);
        let up = super::fitted_protocol(&picker, &dyn_src, target, true).expect("scaled");
        let native = super::fitted_protocol(&picker, &dyn_src, target, false).expect("fitted");
        assert_eq!(up.size(), super::fit_for_protocol(&picker, &dyn_src, target, true).1);
        assert_eq!(native.size(), super::fit_for_protocol(&picker, &dyn_src, target, false).1);
        assert!(
            up.size().width > native.size().width && up.size().height > native.size().height,
            "upscale must still fill the window ({:?}) where Fit leaves it native ({:?})",
            up.size(),
            native.size()
        );
        assert_eq!(up.size(), Size::new(100, 32), "aspect preserved: 320x200 into a 1000x800 box");

        // A magnification on the sample grid — one big enough that both axes grow — is
        // replication, and mints no colour. 32x20 cells is a 32x40 grid over a 320x200
        // canvas, so take a small source to a big grid instead.
        let tiny = image::DynamicImage::ImageRgba8(dithered_plate(24, 15));
        let grown = super::fitted_protocol(&picker, &tiny, Size::new(100, 40), true).expect("grown");
        let cells = grown.size();
        // The picture's own extent on the grid; the strip past it is padding, which
        // half-blocks resolves to black and which is not part of the picture.
        let (_, pair_cells, (sx, sy)) = fit_pair(&picker, &tiny, Size::new(100, 40), true);
        assert_eq!(cells, pair_cells);
        assert!(sx > 24 && sy > 15, "the picture's grid {sx}x{sy} must actually magnify 24x15");
        let on_screen = screen_grid(&grown, cells);
        let mut seen = std::collections::HashSet::new();
        for p in image::imageops::crop_imm(&on_screen, 0, 0, sx, sy).to_image().pixels() {
            seen.insert([p.0[0], p.0[1], p.0[2]]);
        }
        let mut source_colours = std::collections::HashSet::new();
        for p in tiny.to_rgba8().pixels() {
            source_colours.insert([p.0[0], p.0[1], p.0[2]]);
        }
        assert!(
            seen.len() <= source_colours.len(),
            "a magnified window minted {} colours from {} — that is the blur replication \
             exists to avoid",
            seen.len(),
            source_colours.len()
        );
    }

    /// The blast radius, stated as a test: half-blocks takes the new arm and every
    /// backend that actually encodes pixels is byte-for-byte where SQ-0829 left it.
    ///
    /// This is the "differ where they should" half. Sixel and iTerm2 ship encoded pixels
    /// down the wire, so `Resize::Fit` plus a device-pixel pre-scale is the right shape
    /// for them; kitty never reaches `GraphicsRender::render`'s fit at all, but the cover
    /// and preview sites do build kitty protocols and those are unchanged too. The sixel
    /// arm is compared as RENDERED BYTES, not merely as a cell rect — kitty's ids are
    /// random, so only its rect can be pinned.
    #[test]
    fn only_halfblocks_leaves_the_fit_prescale_behind() {
        let src = image::DynamicImage::ImageRgba8(dithered_plate(240, 320));
        let target = Size::new(20, 10);
        let mut sixel = kitty_picker(8, 16);
        sixel.set_protocol_type(ratatui_image::picker::ProtocolType::Sixel);
        for picker in [sixel, kitty_picker(8, 16)] {
            let (pre, size) = super::fit_for_protocol(&picker, &src, target, false);
            let want = picker.new_protocol(pre, size, Resize::Fit(None)).expect("the old pair");
            let got = super::fitted_protocol(&picker, &src, target, false).expect("builds");
            assert_eq!(
                got.size(),
                want.size(),
                "{:?} still goes through fit_for_protocol + new_protocol, unchanged",
                picker.protocol_type()
            );
            if picker.protocol_type() == ratatui_image::picker::ProtocolType::Sixel {
                let rect = Rect::new(0, 0, want.size().width, want.size().height);
                let (mut a, mut b) = (Buffer::empty(rect), Buffer::empty(rect));
                Image::new(&got).render(rect, &mut a);
                Image::new(&want).render(rect, &mut b);
                assert_eq!(a, b, "sixel's encoded bytes are the pre-scale's, to the byte");
            }
        }
    }
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;

    // A 100×100 native image drawn 1:1 (scale 1) into a pane at the origin with
    // 10×10-pixel cells — one cell == 10 game pixels.
    fn unit_map() -> V6ClickMap {
        V6ClickMap {
            pane_x: 0,
            pane_y: 0,
            cell_w: 10,
            cell_h: 10,
            img_x: 0.0,
            img_y: 0.0,
            img_w: 100.0,
            img_h: 100.0,
            canvas: (100, 100),
            screen: (100, 100),
            packed_text: Vec::new(),
        }
    }

    #[test]
    fn map_click_inverts_letterbox_at_cell_centre() {
        let m = unit_map();
        // Cell (0,0) centre is device px (5,5) -> game px floor(5/100*100)+1 = 6.
        assert_eq!(m.map_click(0, 0), Some((6, 6)));
        // Cell (3,4) centre is device px (35,45) -> game px (36, 46).
        assert_eq!(m.map_click(3, 4), Some((36, 46)));
        // Last in-image cell (9,9): centre (95,95) -> (96, 96), within native.
        assert_eq!(m.map_click(9, 9), Some((96, 96)));
    }

    #[test]
    fn map_click_rejects_clicks_outside_the_image() {
        let m = unit_map();
        // Cell 10 starts at device px 100 == native width -> outside (letterbox).
        assert_eq!(m.map_click(10, 0), None);
        assert_eq!(m.map_click(0, 10), None);
    }

    #[test]
    fn map_click_honours_pane_and_letterbox_offset() {
        // Pane origin at cell (4,2); image centred with an 8px/4px letterbox
        // offset inside a 5×5-pixel cell grid; native 40×40 scaled 2×.
        let m = V6ClickMap {
            pane_x: 4,
            pane_y: 2,
            cell_w: 5,
            cell_h: 5,
            img_x: 8.0,
            img_y: 4.0,
            img_w: 80.0, // 40 native * 2
            img_h: 80.0,
            packed_text: Vec::new(),
            canvas: (40, 40),
            screen: (40, 40),
        };
        // A click left of the pane origin is rejected outright.
        assert_eq!(m.map_click(3, 2), None);
        // Cell (6,4) rel to pane is (2,2): centre device px (12.5, 12.5);
        // (12.5-8)/80 * 40 = 2.25 -> floor+1 = 3 in x; (12.5-4)/80*40=4.25 -> 5 in y.
        assert_eq!(m.map_click(6, 4), Some((3, 5)));
        // A click in the top-left letterbox margin (before img_x/img_y) → None.
        assert_eq!(m.map_click(4, 2), None);
    }

    fn window(win: u32) -> GraphicsWindow {
        GraphicsWindow {
            win,
            canvas: std::sync::Arc::new(image::RgbaImage::new(1, 1)),
            version: 1,
            upscale: false,
        }
    }

    fn populate(gr: &mut GraphicsRender, picker: &Picker, wins: &[u32]) {
        let area = Rect::new(0, 0, 4, 2);
        let mut buf = Buffer::empty(area);
        for &win in wins {
            gr.render(picker, &window(win), area, Style::default(), &mut buf);
        }
    }

    #[test]
    fn retain_live_drops_closed_windows() {
        // halfblocks() needs no terminal query — deterministic in tests.
        let picker = Picker::halfblocks();
        let mut gr = GraphicsRender::default();
        populate(&mut gr, &picker, &[1, 2]);
        assert_eq!(gr.cache.len(), 2);

        gr.retain_live(&std::collections::HashSet::from([1]));
        assert_eq!(gr.cache.len(), 1);
        assert!(gr.cache.contains_key(&1));
    }

    fn solid(win: u32, wpx: u32, hpx: u32, rgba: [u8; 4]) -> GraphicsWindow {
        GraphicsWindow {
            win,
            canvas: std::sync::Arc::new(image::RgbaImage::from_pixel(wpx, hpx, image::Rgba(rgba))),
            version: 1,
            upscale: false,
        }
    }

    #[test]
    fn thin_divider_renders_as_line_glyph_in_its_colour() {
        // A 1×3-cell divider (solid red canvas) renders as a │ rule in red — thin,
        // not a full-cell block.
        let gw = solid(1, 9, 57, [156, 31, 0, 255]);
        let area = Rect::new(2, 0, 1, 3);
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 3));
        assert!(render_graphics_as_cells(&gw, area, &mut buf, false), "thin → cells");
        for cy in 0..3 {
            assert_eq!(buf.cell((2, cy)).unwrap().symbol(), "\u{2502}", "cell (2,{cy}) is a │ rule");
            assert_eq!(buf.cell((2, cy)).unwrap().style().fg, Some(Color::Rgb(156, 31, 0)), "rule colour on fg");
        }
    }

    #[test]
    fn thin_sparse_rule_renders_as_line_glyph() {
        // Kerkerkruip's real case: a 1px-tall rule at the TOP of a 1-cell-tall
        // (19px) window. A centre sample would miss it; the region scan catches the
        // opaque line. Because it's SPARSE (a pixel-thin rule), it renders as a thin
        // ─ glyph in the rule colour (fg), NOT a full-cell block — matching a pixel
        // interpreter's thin bar.
        let mut img = image::RgbaImage::new(90, 19); // 10 cells × 1 cell, transparent
        for x in 0..90 {
            img.put_pixel(x, 0, image::Rgba([200, 40, 60, 255])); // top row only
        }
        let gw = GraphicsWindow { win: 1, canvas: std::sync::Arc::new(img), version: 1, upscale: false };
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        assert!(render_graphics_as_cells(&gw, area, &mut buf, false), "thin strip → cells");
        let cell = buf.cell((5, 0)).unwrap();
        assert_eq!(cell.symbol(), "\u{2500}", "sparse horizontal rule → ─ glyph");
        assert_eq!(cell.style().fg, Some(Color::Rgb(200, 40, 60)), "rule colour on the glyph fg");
    }

    #[test]
    fn thin_vertical_sparse_rule_renders_vertical_glyph() {
        // A 2px-wide vertical rule in a 1-cell-wide × 3-tall window → │ glyph.
        let mut img = image::RgbaImage::new(9, 57); // 1 cell × 3 cells, transparent
        for y in 0..57 {
            img.put_pixel(3, y, image::Rgba([255, 255, 255, 255]));
            img.put_pixel(4, y, image::Rgba([255, 255, 255, 255]));
        }
        let gw = GraphicsWindow { win: 1, canvas: std::sync::Arc::new(img), version: 1, upscale: false };
        let area = Rect::new(0, 0, 1, 3);
        let mut buf = Buffer::empty(area);
        assert!(render_graphics_as_cells(&gw, area, &mut buf, false));
        assert_eq!(buf.cell((0, 1)).unwrap().symbol(), "\u{2502}", "sparse vertical rule → │ glyph");
    }

    #[test]
    fn kitty_graphics_place_with_explicit_grid_and_transmit_once() {
        // SQ-0520: on a kitty terminal a graphics window transmits its canvas as
        // a virtual placement with an EXPLICIT r×c grid (the terminal scales the
        // image to exactly the window's cells) and re-transmits only when the
        // canvas version or area changes — a replaced transmit deletes its
        // predecessor's image id.
        let mut img = image::RgbaImage::new(1104, 36);
        for (x, _y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgba([(x % 256) as u8, 40, 200, 255]);
        }
        let gw = GraphicsWindow { win: 7, canvas: std::sync::Arc::new(img), version: 3, upscale: false };
        // from_fontsize is deprecated in favor of a live stdio query, which a
        // headless test can't do — the fixed 8×18 mirrors the SQ-0520 report.
        #[allow(deprecated)]
        let mut picker = Picker::from_fontsize(ratatui_image::FontSize::new(8, 18));
        picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
        let area = Rect::new(0, 0, 138, 2);
        let mut gr = GraphicsRender::default();

        let mut buf = Buffer::empty(area);
        gr.render(&picker, &gw, area, Style::default(), &mut buf);
        let first = buf.cell((0, 0)).unwrap().symbol().to_string();
        assert!(first.contains(",r=2,c=138,"), "transmit declares the explicit cell grid");
        // No `o=z`: `from_fontsize` asks the terminal nothing, so its capability
        // list is empty and the transmit must go out raw (SQ-0997).
        assert!(first.contains("a=T,U=1,f=32,t=d"), "virtual placement transmit present");
        assert!(first.contains(",p=1,"), "and names the placement it owns (SQ-0995)");
        assert!(first.contains('\u{10EEEE}'), "placeholder run present");
        assert!(!first.contains("a=d"), "first transmit deletes nothing");
        assert!(buf.cell((0, 1)).unwrap().symbol().contains('\u{10EEEE}'), "second row placed");

        // Same version + area: no re-transmit, placeholders only.
        let mut buf2 = Buffer::empty(area);
        gr.render(&picker, &gw, area, Style::default(), &mut buf2);
        let second = buf2.cell((0, 0)).unwrap().symbol().to_string();
        assert!(!second.contains("a=T"), "unchanged canvas is not re-transmitted");
        assert!(second.contains('\u{10EEEE}'), "placeholders still placed every frame");

        // Version bump with UNCHANGED pixels (a game that repaints its whole
        // window every turn): nothing is re-uploaded and nothing is deleted —
        // the same id is simply re-placed. (SQ-0564)
        let gw2 = GraphicsWindow { win: 7, canvas: gw.canvas.clone(), version: 4, upscale: false };
        let mut buf3 = Buffer::empty(area);
        gr.render(&picker, &gw2, area, Style::default(), &mut buf3);
        let third = buf3.cell((0, 0)).unwrap().symbol().to_string();
        assert!(!third.contains("a=T"), "a repaint of identical pixels is not re-uploaded");
        assert!(!third.contains("a=d"), "and frees nothing — the image is still wanted");
        assert_eq!(gr.kitty_uploads(7), Some((1, id_of(&first))), "still one upload, still placed");
    }

    /// Recover the image id from a transmit escape (`i=<id>`), so a test can assert
    /// WHICH cached upload a frame placed.
    fn id_of(transmit: &str) -> u32 {
        let after = transmit.split("i=").nth(1).expect("transmit carries an id");
        after
            .split(|c: char| !c.is_ascii_digit())
            .find(|s| !s.is_empty())
            .expect("digits after i=")
            .parse()
            .expect("numeric id")
    }

    /// THE PROPERTY SQ-0995 IS ABOUT. A changed canvas keeps the window's image id,
    /// so ratatui's diff carries the transmit and NOTHING ELSE: one cell out of the
    /// whole grid.
    ///
    /// The id is a per-cell value — `kitty_place_rows` writes its low 24 bits into
    /// every placeholder's foreground and its high byte into the third diacritic —
    /// so an id that changes dirties every cell of the window. This is asserted on
    /// the DIFF rather than on the buffer because the buffer looks the same either
    /// way: both builds write `w*h` placeholders every frame, and the only question
    /// is how many of them ratatui then has to emit. Restore the per-hash id
    /// allocation and this case fails with 276 cells instead of 1.
    #[test]
    fn a_changed_canvas_keeps_the_id_so_the_diff_is_one_cell_not_the_grid() {
        let picker = kitty_picker(8, 18);
        let area = Rect::new(0, 0, 138, 2);
        let cells = usize::from(area.width) * usize::from(area.height);
        let mut gr = GraphicsRender::default();

        // A frame of the same window, one pixel different from the last.
        let frame = |gr: &mut GraphicsRender, version: u64, tint: u8| {
            let mut img = image::RgbaImage::from_pixel(1104, 36, image::Rgba([220, 220, 220, 255]));
            img.put_pixel(0, 0, image::Rgba([tint, 0, 0, 255]));
            let gw =
                GraphicsWindow { win: 2, canvas: std::sync::Arc::new(img), version, upscale: false };
            let mut buf = Buffer::empty(area);
            gr.render(&picker, &gw, area, Style::default(), &mut buf);
            buf
        };

        let a = frame(&mut gr, 1, 1);
        let b = frame(&mut gr, 2, 2);
        let lead_a = a.cell((0, 0)).unwrap().symbol().to_string();
        let lead_b = b.cell((0, 0)).unwrap().symbol().to_string();
        assert!(lead_b.contains("a=T"), "the changed pixels are transmitted");

        // The symptom first: this is the count that was `cells` before SQ-0995.
        let diff = a.diff(&b);
        assert_eq!(
            diff.len(),
            1,
            "a changed canvas costs the transmit and nothing else, not {cells} placeholder cells"
        );
        assert_eq!(diff[0].0, area.x, "and it is the lead cell, which carries the transmit");
        assert_eq!(diff[0].1, area.y);
        // …and the reason for it.
        assert_eq!(id_of(&lead_a), id_of(&lead_b), "the id is what holds still");

        // The frame AFTER an upload drops the escape from that lead cell, so it
        // diffs once more and then the window is free again.
        let c = frame(&mut gr, 3, 2);
        assert!(
            !c.cell((0, 0)).unwrap().symbol().contains("a=T"),
            "identical pixels are not re-uploaded (SQ-0564's insight, kept)"
        );
        assert_eq!(b.diff(&c).len(), 1, "only the lead cell, shedding the escape");
        let d = frame(&mut gr, 4, 2);
        assert_eq!(c.diff(&d).len(), 0, "a settled window emits nothing at all");
        assert_eq!(gr.kitty_uploads(2), Some((1, id_of(&lead_a))), "one upload, still placed");
    }

    /// A window animating through many canvases never holds more than ONE image in
    /// the terminal, and never queues a delete to get there: the id is stable and
    /// the data behind it is replaced (SQ-0995). This is what the SQ-0564 LRU used
    /// to buy with a cap and an eviction — and could not, because re-placing a
    /// cached id swapped the id in every cell and repainted the grid anyway.
    #[test]
    fn an_animating_window_holds_exactly_one_upload_and_evicts_nothing() {
        let picker = kitty_picker(8, 18);
        let area = Rect::new(0, 0, 10, 2);
        let mut gr = GraphicsRender::default();
        let mut ids = std::collections::BTreeSet::new();
        for i in 0..24u32 {
            let mut img = image::RgbaImage::from_pixel(80, 36, image::Rgba([10, 20, 30, 255]));
            img.put_pixel(0, 0, image::Rgba([i as u8, 0, 0, 255]));
            let gw = GraphicsWindow {
                win: 4,
                canvas: std::sync::Arc::new(img),
                version: u64::from(i) + 1,
                upscale: false,
            };
            let mut buf = Buffer::empty(area);
            gr.render(&picker, &gw, area, Style::default(), &mut buf);
            let sym = buf.cell((0, 0)).unwrap().symbol().to_string();
            assert!(sym.contains("a=T"), "frame {i} is a new picture → uploaded");
            assert!(!sym.contains("a=d"), "frame {i} replaces its data; nothing is abandoned");
            ids.insert(id_of(&sym));
        }
        assert_eq!(ids.len(), 1, "24 canvases, one id: {ids:?}");
        assert_eq!(
            gr.kitty_uploads(4).map(|(n, _)| n),
            Some(1),
            "and one image in the terminal, with no cap needed to bound it"
        );
    }

    #[test]
    fn thin_but_detailed_toolbar_is_not_a_rule() {
        // SQ-0520: advent.blb's clickable toolbar is a DETAILED graphics window
        // that lands at 2 cells tall on common pane widths (its ~36px request /
        // an 18px cell). The thin-strip shortcut must not claim it as a rule and
        // paint colour-averaged ─ glyphs ("two thin strips of pixels") — a thin
        // window whose cells disagree in colour is an image for the protocol.
        let mut img = image::RgbaImage::new(1104, 36); // 138×2 cells at 8×18
        for (x, _y, p) in img.enumerate_pixels_mut() {
            // Blocks of strongly different hues, like toolbar icons.
            *p = match (x / 48) % 4 {
                0 => image::Rgba([200, 60, 40, 255]),
                1 => image::Rgba([40, 160, 60, 255]),
                2 => image::Rgba([50, 80, 200, 255]),
                _ => image::Rgba([220, 210, 200, 255]),
            };
        }
        let gw = GraphicsWindow { win: 1, canvas: std::sync::Arc::new(img), version: 1, upscale: false };
        let area = Rect::new(0, 0, 138, 2);
        let mut buf = Buffer::empty(area);
        assert!(
            !render_graphics_as_cells(&gw, area, &mut buf, false),
            "a thin-but-detailed canvas must fall through to the image protocol"
        );
        // force=true (the no-picker fallback) still paints the approximation.
        assert!(render_graphics_as_cells(&gw, area, &mut buf, true), "forced → approximated as cells");
    }

    #[test]
    fn thin_fully_transparent_paints_nothing() {
        // A thin window the game hasn't drawn (all transparent) leaves cells alone.
        let img = image::RgbaImage::new(90, 19);
        let gw = GraphicsWindow { win: 1, canvas: std::sync::Arc::new(img), version: 1, upscale: false };
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        buf.cell_mut((5, 0)).unwrap().set_style(Style::default().bg(Color::Rgb(1, 2, 3)));
        assert!(render_graphics_as_cells(&gw, area, &mut buf, false), "thin → handled");
        assert_eq!(buf.cell((5, 0)).unwrap().style().bg, Some(Color::Rgb(1, 2, 3)), "transparent → underlying kept");
    }

    #[test]
    fn large_uniform_graphics_paints_cells() {
        // A big but uniform canvas is still cheap-and-exact as cells.
        let gw = solid(1, 90, 190, [10, 20, 30, 255]);
        let area = Rect::new(0, 0, 10, 10);
        let mut buf = Buffer::empty(area);
        assert!(render_graphics_as_cells(&gw, area, &mut buf, false), "uniform → cells");
        assert_eq!(buf.cell((5, 5)).unwrap().style().bg, Some(Color::Rgb(10, 20, 30)));
    }

    #[test]
    fn render_as_cells_memoizes_the_classification_on_version_and_area() {
        // SQ-1200: a redraw with the SAME (version, area, force) must reuse the
        // last classification instead of re-running the blank/uniform/rule_like
        // scans and the region-averaging cell_color over the whole canvas —
        // roughly two full passes for a window that has not repainted.
        let mut gr = GraphicsRender::default();
        let gw = solid(1, 90, 190, [10, 20, 30, 255]);
        let area = Rect::new(0, 0, 10, 10);

        let mut buf = Buffer::empty(area);
        assert!(gr.render_as_cells(&gw, area, &mut buf, false), "uniform → cells");
        assert_eq!(gr.classify_calls(), 1, "first draw classifies");
        assert_eq!(buf.cell((5, 5)).unwrap().style().bg, Some(Color::Rgb(10, 20, 30)));

        // Same window, same version, same area: must hit the memo.
        let mut buf2 = Buffer::empty(area);
        assert!(gr.render_as_cells(&gw, area, &mut buf2, false));
        assert_eq!(gr.classify_calls(), 1, "an unchanged redraw reuses the memo");
        assert_eq!(
            buf2.cell((5, 5)).unwrap().style().bg,
            Some(Color::Rgb(10, 20, 30)),
            "a memo replay paints the same colour a fresh classify+paint would"
        );

        // A version bump (a repaint) must recompute, and reflect the new pixels.
        let mut gw2 = gw.clone();
        gw2.version = 2;
        gw2.canvas = std::sync::Arc::new(image::RgbaImage::from_pixel(90, 190, image::Rgba([200, 0, 0, 255])));
        let mut buf3 = Buffer::empty(area);
        assert!(gr.render_as_cells(&gw2, area, &mut buf3, false));
        assert_eq!(gr.classify_calls(), 2, "a version bump recomputes");
        assert_eq!(buf3.cell((5, 5)).unwrap().style().bg, Some(Color::Rgb(200, 0, 0)), "the new colour is painted");
    }

    #[test]
    fn detailed_graphics_falls_back_to_protocol() {
        // A non-thin, non-uniform canvas (checker) must NOT be handled as cells.
        let mut img = image::RgbaImage::new(90, 190);
        for (x, y, p) in img.enumerate_pixels_mut() {
            let on = ((x / 9) + (y / 19)) % 2 == 0;
            *p = if on { image::Rgba([255, 255, 255, 255]) } else { image::Rgba([0, 0, 0, 255]) };
        }
        let gw = GraphicsWindow { win: 1, canvas: std::sync::Arc::new(img), version: 1, upscale: false };
        let area = Rect::new(0, 0, 10, 10);
        let mut buf = Buffer::empty(area);
        assert!(!render_graphics_as_cells(&gw, area, &mut buf, false), "detailed image → protocol, not cells");
    }

    #[test]
    fn large_fully_transparent_is_handled_not_sent_to_protocol() {
        // narco opens big border frames around its story but never paints them.
        // A blank (all-transparent) window must be reported HANDLED (painting
        // nothing), NOT bounced to the image protocol — a transparent image gets
        // garbled into artifacts (stray chars/lines) over the neighbouring
        // windows in a real terminal. (SQ-0338)
        let img = image::RgbaImage::new(90, 190); // 10×10 cells, all transparent
        let gw = GraphicsWindow { win: 1, canvas: std::sync::Arc::new(img), version: 1, upscale: false };
        let area = Rect::new(0, 0, 10, 10);
        let mut buf = Buffer::empty(area);
        buf.cell_mut((5, 5)).unwrap().set_style(Style::default().bg(Color::Rgb(1, 2, 3)));
        assert!(render_graphics_as_cells(&gw, area, &mut buf, false), "blank window → handled, not protocol");
        assert_eq!(buf.cell((5, 5)).unwrap().style().bg, Some(Color::Rgb(1, 2, 3)), "blank → underlying kept");
    }

    /// Drive the background v6 encode to completion (test helper, SQ-0469).
    fn drain_v6_job(gr: &mut GraphicsRender) {
        while gr.v6_encode_in_flight() {
            std::thread::sleep(std::time::Duration::from_millis(1));
            gr.poll_v6_job();
        }
    }

    #[test]
    fn v6_encode_gates_on_generation_off_thread() {
        let picker = Picker::halfblocks();
        let mut gr = GraphicsRender::default();
        let area = Rect::new(0, 0, 4, 2);
        let canvas = image::RgbaImage::from_pixel(32, 32, image::Rgba([1, 2, 3, 255]));

        // First frame for gen 7: nothing ready, no job → wants a build. With no
        // last-ready composite the encode is SYNCHRONOUS (SQ-0578: a transition
        // frame has no honest previous image to show), so it is ready this frame.
        assert!(gr.v6_wants_build(7, area), "cold start wants the first build");
        gr.spawn_v6_encode(&picker, canvas.clone(), 7, area, RasterFrame::native((canvas.width() as u16, canvas.height() as u16)));
        assert!(gr.v6_job.is_none(), "a cold-start encode runs synchronously, no worker");
        assert!(gr.v6.is_some(), "the cold-start encode installed immediately");
        assert_eq!(gr.v6.as_ref().unwrap().gen, 7);

        // With a composite ready, a NEW generation encodes off-thread. While the
        // encode is in flight, no second build is requested — coalesced, even
        // for a newer generation (newest wins: the in-flight one finishes, then
        // the next frame builds whatever the current generation is).
        assert!(gr.v6_wants_build(8, area), "a changed generation wants a fresh build");
        gr.spawn_v6_encode(&picker, canvas.clone(), 8, area, RasterFrame::native((canvas.width() as u16, canvas.height() as u16)));
        assert!(gr.v6_job.is_some(), "a warm re-encode runs on the worker");
        assert!(!gr.v6_wants_build(8, area), "an in-flight encode suppresses a rebuild");
        assert!(!gr.v6_wants_build(9, area), "an in-flight encode suppresses even a newer generation");
        drain_v6_job(&mut gr);
        assert!(gr.v6.is_some(), "the worker installed the encoded protocol");
        assert_eq!(gr.v6.as_ref().unwrap().gen, 8);

        // `invalidate_v6` (the hybrid band path ran): back to cold — the next
        // raster frame wants a build and will encode synchronously again.
        gr.invalidate_v6();
        assert!(gr.v6.is_none() && gr.v6_job.is_none(), "invalidation drops composite and job");
        assert!(gr.v6_wants_build(8, area), "an invalidated cache wants a rebuild");
        gr.spawn_v6_encode(&picker, canvas.clone(), 7, area, RasterFrame::native((canvas.width() as u16, canvas.height() as u16)));
        assert!(gr.v6_job.is_none() && gr.v6.is_some(), "post-invalidation encode is synchronous");

        // Same generation → no rebuild (the gate is the win: idle frames skip
        // build + encode entirely).
        assert!(!gr.v6_wants_build(7, area), "unchanged generation reuses the ready encode");
        // A new generation → wants a rebuild.
        assert!(gr.v6_wants_build(9, area), "a changed generation wants a fresh build");
        // A pane resize → wants a rebuild even at the same generation.
        assert!(gr.v6_wants_build(7, Rect::new(0, 0, 5, 2)), "a resize wants a fresh build");
    }

    /// A 32×32 native canvas in a huge pane encodes at [`MAX_V6_UPSCALE`], not at the
    /// full device box: a 200×100-cell pane at 10×20 is 2000×2000 device pixels, which
    /// would otherwise scale ~62×.
    ///
    /// **This asks the question of an ENCODING backend, and now says so** (SQ-0964).
    /// It used to run on `Picker::halfblocks()` — the deterministic test picker,
    /// reached for because it needs no terminal query rather than because half-blocks
    /// was the subject. That is precisely the backend the cap no longer applies to, so
    /// a case whose whole claim is "the cap engaged" has to name a backend that spends
    /// the budget the cap protects. `kitty_picker` at the same 10×20 cell keeps every
    /// number below unchanged, and the half-blocks half of the contrast is asserted
    /// alongside rather than dropped.
    #[test]
    fn v6_encode_caps_upscale_on_an_encoding_backend() {
        let area = Rect::new(0, 0, 200, 100);
        let canvas = image::RgbaImage::from_pixel(32, 32, image::Rgba([1, 2, 3, 255]));
        let kitty = kitty_picker(10, 20);
        let ready = GraphicsRender::encode_v6(&kitty, &canvas, 1, area, RasterFrame::native((canvas.width() as u16, canvas.height() as u16)), None).expect("encode");
        // The encoded protocol reports its device size; 2× of 32 = 64 px = at most
        // ceil(64/20)=4 cells tall / ceil(64/10)=7 wide. Assert it is far smaller than
        // the pane (the cap engaged), not the full 200×100.
        let sz = ready.proto.size();
        assert!(sz.width <= 14 && sz.height <= 8, "capped image is ~2× native, got {sz:?}");
        // …and the same canvas on half-blocks, which encodes nothing, reaches the pane.
        let hb = Picker::halfblocks();
        let hb_ready = GraphicsRender::encode_v6(&hb, &canvas, 1, area, RasterFrame::native((canvas.width() as u16, canvas.height() as u16)), None).expect("encode");
        let hb_sz = hb_ready.proto.size();
        assert!(
            hb_sz.width > sz.width && hb_sz.height > sz.height,
            "half-blocks spends no encode budget, so it takes the pane: {hb_sz:?} must beat \
             the capped {sz:?} on both axes"
        );
        assert!(
            hb_sz.width <= area.width && hb_sz.height <= area.height,
            "…and stops AT the pane: {hb_sz:?} inside {}x{}",
            area.width,
            area.height
        );
    }

    /// SQ-0996, on the two `ratatui-image` paths the v6 pane is actually drawn
    /// through: a CHANGED chrome band and a CHANGED raster composite each cost the
    /// picture and ONE cell, not the whole placeholder rect.
    ///
    /// The id is a per-cell value — `kitty_place_rows` writes its low 24 bits into
    /// every placeholder's foreground and its high byte into the third diacritic —
    /// so an id that moves dirties every cell of the placement. `ratatui-image`
    /// draws a fresh random id for every `Protocol`, and both of these paths build a
    /// new `Protocol` on every content change, so one changed pixel repainted the
    /// rect. Measured under a pty on Journey r83 in raster mode at 117x64, one
    /// changed frame: 3,680 cells and 48,742 bytes for a 7,668-byte image, against
    /// 1 cell and 7,806 bytes.
    ///
    /// Asserted as a BUFFER DIFF because that is what ratatui emits — the property
    /// is not "the id is equal" (a fix that held the id and rewrote the cells
    /// anyway would pass that) but "the frame is one cell wide".
    ///
    /// FALSIFY by dropping the `reuse` argument at either encode site — hand
    /// `picker.new_protocol` the image and let it draw its own id: the band case
    /// fails with all 80 of its cells in the diff and the composite case with all
    /// 40 of its.
    mod stable_image_ids {
        use super::*;

        const CELL: (u16, u16) = (8, 18);

        fn tinted(w: u32, h: u32, green: u8) -> image::RgbaImage {
            image::RgbaImage::from_fn(w, h, |x, y| {
                image::Rgba([((x + y) % 251) as u8, green, 0x40, 255])
            })
        }

        #[test]
        fn a_changed_chrome_band_keeps_its_id_so_the_diff_is_one_cell_not_the_rect() {
            use crate::render::v6_layout::uniform_scale;
            let picker = kitty_picker(CELL.0, CELL.1);
            let mut gr = GraphicsRender::default();
            let pane = Rect::new(0, 0, 20, 10);
            let native = (pane.width as u32 * CELL.0 as u32, pane.height as u32 * CELL.1 as u32);
            let scale = uniform_scale((native.0 as u16, native.1 as u16), (native.0, native.1));
            let band = Rect::new(pane.x, pane.y, pane.width, 4);
            let key = (BandSlot::Art as u8, band.x, band.y, band.width, band.height);

            let draw = |gr: &mut GraphicsRender, green: u8| {
                let mut buf = Buffer::empty(pane);
                gr.draw_chrome_band(&picker, &tinted(native.0, native.1, green), &scale, pane, band, &mut buf);
                buf
            };
            // Two frames of the first art, so the band is settled — the second sheds
            // the transmit escape and is otherwise identical to the first.
            let _ = draw(&mut gr, 40);
            let settled = draw(&mut gr, 40);
            let id = gr.chrome_band_id(key).expect("a kitty placement names its image");

            // The change frame stages the encode for the worker and keeps the old
            // upload placed (SQ-1188); the frame after the result lands is the one
            // that carries the transmit, and it is the one whose diff is measured.
            let _ = draw(&mut gr, 90);
            gr.spawn_band_jobs(&picker);
            settle_bands(&mut gr);
            let changed = draw(&mut gr, 90);
            // The DIFF first: it is the symptom, and a fix that held the id while
            // rewriting the cells anyway would sail past the id assertion below.
            let diff = settled.diff(&changed);
            let cells = usize::from(band.width) * usize::from(band.height);
            assert_eq!(
                diff.len(),
                1,
                "a changed band costs the lead cell (which carries the transmit) and nothing \
                 else, not all {cells} placeholders"
            );
            assert_eq!(gr.chrome_band_id(key), Some(id), "and its id did not move");
            assert!(
                diff[0].2.symbol().contains("a=T,"),
                "and that one cell IS the new upload — one cell and no transmit would be a \
                 frame that changed nothing"
            );
            assert!(
                gr.queued_deletes().is_empty() && gr.queued_deletes_after_place().is_empty(),
                "nothing is freed: the id we re-transmitted to is the id on screen, and \
                 deleting it would take the picture with it"
            );
        }

        #[test]
        fn a_changed_raster_composite_keeps_its_id_so_the_diff_is_one_cell_not_the_pane() {
            let picker = kitty_picker(CELL.0, CELL.1);
            let mut gr = GraphicsRender::default();
            let area = Rect::new(0, 0, 10, 4);
            let native = (area.width as u32 * CELL.0 as u32, area.height as u32 * CELL.1 as u32);

            let draw = |gr: &mut GraphicsRender, gen: u64, green: u8| {
                gr.spawn_v6_encode(&picker, tinted(native.0, native.1, green), gen, area, RasterFrame::native((native.0 as u16, native.1 as u16)));
                drain_v6_job(gr);
                let mut buf = Buffer::empty(area);
                gr.redraw_v6(&picker, area, &mut buf);
                buf
            };
            let _ = draw(&mut gr, 1, 40);
            let settled = draw(&mut gr, 2, 40);
            let id = gr.v6.as_ref().and_then(|r| r.placed_id).expect("the composite is placed");

            let changed = draw(&mut gr, 3, 90);
            let diff = settled.diff(&changed);
            let cells = usize::from(area.width) * usize::from(area.height);
            assert_eq!(
                diff.len(),
                1,
                "a changed composite costs one cell, not the pane's {cells}"
            );
            assert_eq!(
                gr.v6.as_ref().and_then(|r| r.placed_id),
                Some(id),
                "and its id did not move"
            );
            assert!(diff[0].2.symbol().contains("a=T,"), "and that cell carries the new upload");
            assert!(
                gr.queued_deletes().is_empty() && gr.queued_deletes_after_place().is_empty(),
                "the composite being replaced IS the one re-transmitted to; freeing it would \
                 blank the pane"
            );
        }

        /// SQ-0637 is untouched by SQ-0996, which is the half that is easy to lose:
        /// an upload the app can no longer re-place must still be DELETED in the
        /// terminal, or every abandoned band and every abandoned composite leaks a
        /// cache generation until the terminal's own quota evicts something live.
        ///
        /// The id must not survive the abandonment either. Reviving a deleted id
        /// would emit `a=d` for it (queued at eviction, riding out on whichever
        /// placement goes next) possibly AFTER the transmit that revived it, and
        /// the picture would be freed the moment it arrived.
        #[test]
        fn an_abandoned_upload_is_still_freed_and_never_comes_back_under_the_same_id() {
            use crate::render::v6_layout::uniform_scale;
            let picker = kitty_picker(CELL.0, CELL.1);
            let mut gr = GraphicsRender::default();
            let pane = Rect::new(0, 0, 20, 10);
            let native = (pane.width as u32 * CELL.0 as u32, pane.height as u32 * CELL.1 as u32);
            let scale = uniform_scale((native.0 as u16, native.1 as u16), (native.0, native.1));
            let band = Rect::new(pane.x, pane.y, pane.width, 4);
            let key = (BandSlot::Art as u8, band.x, band.y, band.width, band.height);
            let art = tinted(native.0, native.1, 40);

            let mut buf = Buffer::empty(pane);
            gr.draw_chrome_band(&picker, &art, &scale, pane, band, &mut buf);
            let first = gr.chrome_band_id(key).expect("a kitty placement names its image");

            gr.retain_chrome_bands(&std::collections::HashSet::new());
            assert!(
                gr.queued_deletes().contains(&format!("a=d,d=I,i={first}")),
                "an evicted band is freed in the terminal, not merely forgotten (SQ-0637): {:?}",
                gr.queued_deletes()
            );

            let mut buf = Buffer::empty(pane);
            gr.draw_chrome_band(&picker, &art, &scale, pane, band, &mut buf);
            assert_ne!(
                gr.chrome_band_id(key),
                Some(first),
                "the revived band takes a NEW id — the old one has an `a=d` in flight for it"
            );

            // …and the same for the whole-pane composite, whose abandonment is the
            // raster→ring transition Journey makes two frames into its boot.
            let area = Rect::new(0, 0, 10, 4);
            let canvas = tinted(area.width as u32 * CELL.0 as u32, area.height as u32 * CELL.1 as u32, 40);
            let dims = (canvas.width() as u16, canvas.height() as u16);
            gr.spawn_v6_encode(&picker, canvas, 1, area, RasterFrame::native(dims));
            drain_v6_job(&mut gr);
            let mut buf = Buffer::empty(area);
            gr.redraw_v6(&picker, area, &mut buf);
            let composite = gr.v6.as_ref().and_then(|r| r.placed_id).expect("placed");
            gr.invalidate_v6();
            assert!(
                gr.queued_deletes().contains(&format!("a=d,d=I,i={composite}")),
                "an abandoned composite is freed too: {:?}",
                gr.queued_deletes()
            );
        }
    }

    #[test]
    fn draw_chrome_band_caches_and_retain_prunes() {
        use crate::render::v6_layout::uniform_scale;
        let picker = Picker::halfblocks();
        let mut gr = GraphicsRender::default();
        // Native 32×20 chrome (opaque), scaled 1:1 into a 32×20-device pane.
        let chrome = image::RgbaImage::from_pixel(32, 20, image::Rgba([10, 20, 30, 255]));
        let fs = picker.font_size();
        let pane = Rect::new(0, 0, 32 / fs.width.max(1), 20 / fs.height.max(1));
        let scale = uniform_scale((32, 20), (pane.width as u32 * fs.width as u32, pane.height as u32 * fs.height as u32));
        let band = Rect::new(pane.x, pane.y, pane.width, 1); // a top ring band
        let mut buf = Buffer::empty(pane);

        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        assert_eq!(gr.chrome_bands.len(), 1, "first draw uploads + caches the band protocol");
        let key = (BandSlot::Art as u8, band.x, band.y, band.width, band.height);
        let hash0 = gr.chrome_bands.get(&key).unwrap().0;
        // Same content + band → cache hit, no rebuild.
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        assert_eq!(gr.chrome_bands.get(&key).unwrap().0, hash0, "identical band keeps the cached upload");

        // retain_chrome_bands drops any band not in the live set.
        gr.retain_chrome_bands(&std::collections::HashSet::new());
        assert!(gr.chrome_bands.is_empty(), "empty live set clears the band cache");
    }

    /// A transparent band pixel reaches a half-block screen as the PAGE, not as
    /// the encoder's black (SQ-0944).
    ///
    /// `ratatui-image`'s primitive half-block encoder calls `to_rgb8()`, so a
    /// fully transparent pixel arrives at RGB 0,0,0 — and `pick_side` then
    /// collapses the two equal halves to a SPACE, which is why the symptom on
    /// screen is space cells on a black background rather than anything that
    /// looks like an image. That is the black gutter that ran down both sides of
    /// Zork Zero's pillars where kitty shows the white page the story declared.
    ///
    /// Asserted on the CELLS the band wrote, because that is the whole distance
    /// between the two backends: the same image, the same call, and only the
    /// encoder in between.
    #[test]
    fn a_declared_ground_replaces_the_encoders_black_under_halfblocks() {
        use crate::render::v6_layout::uniform_scale;
        let picker = Picker::halfblocks();
        let fs = picker.font_size();
        let (cw, ch) = (fs.width.max(1) as u32, fs.height.max(1) as u32);
        let pane = Rect::new(0, 0, 4, 2);
        let native = (pane.width as u32 * cw, pane.height as u32 * ch);
        // A canvas that is entirely hole — the frame art leaves exactly this
        // beside a flank, and it is the only thing the ground can be read off.
        let chrome = image::RgbaImage::from_pixel(native.0, native.1, image::Rgba([0, 0, 0, 0]));
        let scale = uniform_scale(
            (native.0 as u16, native.1 as u16),
            (pane.width as u32 * cw, pane.height as u32 * ch),
        );
        let band = Rect::new(pane.x, pane.y, pane.width, 1);
        let page = image::Rgba([255, 255, 255, 255]);

        let cells_of = |ground: Option<image::Rgba<u8>>| -> Vec<(ratatui::style::Color, ratatui::style::Color)> {
            let mut gr = GraphicsRender::default();
            gr.set_band_ground(ground);
            let mut buf = Buffer::empty(pane);
            gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
            (band.x..band.right()).map(|x| {
                let c = buf.cell((x, band.y)).expect("in the pane");
                (c.fg, c.bg)
            }).collect()
        };

        let black = ratatui::style::Color::Rgb(0, 0, 0);
        let white = ratatui::style::Color::Rgb(255, 255, 255);
        assert!(
            cells_of(None).iter().all(|&(fg, bg)| fg == black && bg == black),
            "with no ground declared the encoder picks black, which is the defect",
        );
        assert!(
            cells_of(Some(page)).iter().all(|&(fg, bg)| fg == white && bg == white),
            "and a declared ground reaches the screen instead of it",
        );
    }

    /// …and the ground rides the band's freshness hash, so a frame that changes
    /// it re-encodes rather than placing a band flattened onto the old one.
    #[test]
    fn a_changed_ground_re_encodes_the_band() {
        use crate::render::v6_layout::uniform_scale;
        let picker = Picker::halfblocks();
        let fs = picker.font_size();
        let (cw, ch) = (fs.width.max(1) as u32, fs.height.max(1) as u32);
        let pane = Rect::new(0, 0, 4, 2);
        let native = (pane.width as u32 * cw, pane.height as u32 * ch);
        let chrome = image::RgbaImage::from_pixel(native.0, native.1, image::Rgba([0, 0, 0, 0]));
        let scale = uniform_scale(
            (native.0 as u16, native.1 as u16),
            (pane.width as u32 * cw, pane.height as u32 * ch),
        );
        let band = Rect::new(pane.x, pane.y, pane.width, 1);
        let key = (BandSlot::Art as u8, band.x, band.y, band.width, band.height);
        let mut gr = GraphicsRender::default();
        let mut buf = Buffer::empty(pane);

        gr.set_band_ground(Some(image::Rgba([255, 255, 255, 255])));
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        let hash0 = gr.chrome_bands.get(&key).expect("cached").0;
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        assert_eq!(gr.chrome_bands.get(&key).unwrap().0, hash0, "same ground, same pixels: a cache hit");

        gr.set_band_ground(Some(image::Rgba([0, 0, 128, 255])));
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        assert_ne!(
            gr.chrome_bands.get(&key).unwrap().0, hash0,
            "a changed ground is a changed band — the cache must not serve the old flatten",
        );
    }

    #[test]
    fn draw_chrome_band_isolates_bands_by_native_footprint() {
        // SQ-0514: a change confined to the TOP band's native rows must re-encode
        // only that band; a disjoint BOTTOM band stays fresh (its stored hash and
        // cached upload unchanged). Before the fix, the freshness hash covered the
        // WHOLE canvas, so any pixel change staled every band.
        use crate::render::v6_layout::uniform_scale;
        let picker = Picker::halfblocks();
        let fs = picker.font_size();
        let (cw, ch) = (fs.width.max(1) as u32, fs.height.max(1) as u32);
        let mut gr = GraphicsRender::default();
        // Native canvas exactly the pane's device size → scale 1:1 (s=1, off=0),
        // so a band's native footprint equals its device rows (no letterbox slop).
        let (cols, rows) = (2u16, 4u16);
        let (nw, nh) = (cols as u32 * cw, rows as u32 * ch);
        let mut chrome = image::RgbaImage::from_pixel(nw, nh, image::Rgba([10, 20, 30, 255]));
        let pane = Rect::new(0, 0, cols, rows);
        let scale = uniform_scale((nw as u16, nh as u16), (nw, nh));
        assert_eq!((scale.s, scale.off_x, scale.off_y), (1.0, 0, 0), "test canvas must map 1:1");
        let top = Rect::new(0, 0, cols, 1); // device rows [0, ch)
        let bottom = Rect::new(0, rows - 1, cols, 1); // device rows [nh-ch, nh)
        let mut buf = Buffer::empty(pane);

        gr.draw_chrome_band(&picker, &chrome, &scale, pane, top, &mut buf);
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, bottom, &mut buf);
        let before = gr.chrome_band_hashes();
        let top_key = (BandSlot::Art as u8, top.x, top.y, top.width, top.height);
        let bot_key = (BandSlot::Art as u8, bottom.x, bottom.y, bottom.width, bottom.height);
        assert!(before.contains_key(&top_key) && before.contains_key(&bot_key), "both bands cached");

        // Change a pixel that lives ONLY in the top band's native footprint.
        chrome.put_pixel(0, 1, image::Rgba([200, 0, 0, 255]));
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, top, &mut buf);
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, bottom, &mut buf);
        let after = gr.chrome_band_hashes();

        assert_ne!(before[&top_key], after[&top_key], "the changed top band re-encodes (hash changed)");
        assert_eq!(before[&bot_key], after[&bot_key], "the disjoint bottom band stays fresh (hash unchanged)");
    }

    #[test]
    fn hash_canvas_rows_tracks_footprint_pixels_exactly() {
        // SQ-1189: the row-slice walk must be semantically equivalent to the
        // per-pixel walk it replaced — same canvas → same key, any changed
        // pixel INSIDE the footprint → changed key, any change strictly
        // OUTSIDE it → same key.
        use std::hash::Hasher;
        let hash = |c: &image::RgbaImage| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            hash_canvas_rows(&mut h, c, 2, 12, 1, 9);
            h.finish()
        };
        let mut canvas = image::RgbaImage::new(20, 10);
        for (x, y, p) in canvas.enumerate_pixels_mut() {
            *p = image::Rgba([(x * 13) as u8, (y * 7) as u8, ((x + y) * 5) as u8, 255]);
        }
        let base = hash(&canvas);
        assert_eq!(base, hash(&canvas.clone()), "same canvas → same key");

        let mut inside = canvas.clone();
        inside.put_pixel(11, 8, image::Rgba([1, 2, 3, 4])); // last col/row of the footprint
        assert_ne!(base, hash(&inside), "a changed pixel inside the footprint changes the key");
        let mut corner = canvas.clone();
        corner.put_pixel(2, 1, image::Rgba([9, 9, 9, 9])); // first col/row of the footprint
        assert_ne!(base, hash(&corner), "the footprint's first pixel is covered too");

        let mut outside = canvas.clone();
        outside.put_pixel(12, 8, image::Rgba([1, 2, 3, 4])); // one column past x1
        outside.put_pixel(1, 5, image::Rgba([1, 2, 3, 4])); // one column before x0
        outside.put_pixel(5, 0, image::Rgba([1, 2, 3, 4])); // one row above y0
        outside.put_pixel(5, 9, image::Rgba([1, 2, 3, 4])); // one row below y1
        assert_eq!(base, hash(&outside), "a change strictly outside the footprint leaves the key alone");

        // A degenerate footprint hashes nothing and does not panic.
        let mut h = std::collections::hash_map::DefaultHasher::new();
        hash_canvas_rows(&mut h, &canvas, 5, 5, 2, 8);
        hash_canvas_rows(&mut h, &canvas, 3, 9, 6, 6);
    }

    /// A kitty picker at a stated cell size — the backend whose band encodes go
    /// to the background worker (SQ-1188).
    fn kitty_picker(w: u16, h: u16) -> Picker {
        #[allow(deprecated)]
        let mut p = Picker::from_fontsize(ratatui_image::FontSize::new(w, h));
        p.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
        p
    }

    /// Reap the band worker, driving `poll_v6_job` the way the app's loop tick
    /// does. Panics rather than hangs when nothing ever lands.
    fn settle_bands(gr: &mut GraphicsRender) {
        for _ in 0..500 {
            if gr.poll_v6_job() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        panic!("band worker never landed");
    }

    /// SQ-1188: a 1:1 kitty band fixture — canvas the pane's device size, one
    /// band covering the top row. Returns (gr, picker, chrome, scale, pane, band).
    fn kitty_band_fixture() -> (GraphicsRender, Picker, image::RgbaImage, crate::render::v6_layout::Scale, Rect, Rect)
    {
        use crate::render::v6_layout::uniform_scale;
        let picker = kitty_picker(4, 7);
        let fs = picker.font_size();
        let (cw, ch) = (fs.width.max(1) as u32, fs.height.max(1) as u32);
        let (cols, rows) = (2u16, 4u16);
        let (nw, nh) = (cols as u32 * cw, rows as u32 * ch);
        let chrome = image::RgbaImage::from_pixel(nw, nh, image::Rgba([10, 20, 30, 255]));
        let pane = Rect::new(0, 0, cols, rows);
        let scale = uniform_scale((nw as u16, nh as u16), (nw, nh));
        assert_eq!((scale.s, scale.off_x, scale.off_y), (1.0, 0, 0), "fixture must map 1:1");
        let band = Rect::new(0, 0, cols, 1);
        (GraphicsRender::default(), picker, chrome, scale, pane, band)
    }

    #[test]
    fn a_changed_band_keeps_its_old_upload_until_the_worker_lands() {
        // SQ-1188: on kitty, a band's FIRST encode is synchronous (no honest
        // previous image), a CHANGED band's encode runs on the worker while the
        // old upload stays placed, and the result installs via poll_v6_job.
        let (mut gr, picker, mut chrome, scale, pane, band) = kitty_band_fixture();
        let key = (BandSlot::Art as u8, band.x, band.y, band.width, band.height);
        let mut buf = Buffer::empty(pane);

        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        assert_eq!(gr.band_encodes, 1, "first appearance encodes synchronously");
        assert!(!gr.band_encode_in_flight(), "nothing staged after a sync encode");
        let h0 = gr.chrome_band_hashes()[&key];

        chrome.put_pixel(1, 1, image::Rgba([200, 0, 0, 255]));
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        assert_eq!(gr.band_encodes, 1, "the change frame encodes nothing on this thread");
        assert_eq!(gr.chrome_band_hashes()[&key], h0, "the OLD upload (old hash) is still what is placed");
        assert!(gr.band_encode_in_flight(), "the encode is staged for the worker");
        assert!(
            gr.band_log.last().is_some_and(|l| l.contains("encode queued (worker)")),
            "the band log says what happened: {:?}",
            gr.band_log.last()
        );

        gr.spawn_band_jobs(&picker);
        settle_bands(&mut gr);
        assert_eq!(gr.band_encodes, 2, "the worker's install counts the encode");
        assert_ne!(gr.chrome_band_hashes()[&key], h0, "the installed entry answers for the NEW content");
        assert!(!gr.band_encode_in_flight(), "nothing left staged");

        // The next frame's draw is a plain cache hit on the new content.
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        assert_eq!(gr.band_encodes, 2, "the settled band is a cache hit");
        assert!(
            gr.band_log.last().is_some_and(|l| l.contains("cache HIT")),
            "the settled band logs a hit: {:?}",
            gr.band_log.last()
        );
    }

    #[test]
    fn a_stale_band_result_is_dropped_when_the_band_changed_again() {
        // SQ-1188: results are keyed by the content they answer for — a band
        // that changed AGAIN while its encode ran drops the stale result and
        // re-stages the current content on the next frame.
        let (mut gr, picker, mut chrome, scale, pane, band) = kitty_band_fixture();
        let key = (BandSlot::Art as u8, band.x, band.y, band.width, band.height);
        let mut buf = Buffer::empty(pane);

        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        let h0 = gr.chrome_band_hashes()[&key];

        chrome.put_pixel(1, 1, image::Rgba([200, 0, 0, 255])); // content B
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        gr.spawn_band_jobs(&picker); // B's encode is now in flight
        chrome.put_pixel(1, 1, image::Rgba([0, 200, 0, 255])); // content C, superseding B
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        gr.spawn_band_jobs(&picker); // coalesced: C is dropped and un-marked
        settle_bands(&mut gr);
        assert_eq!(gr.chrome_band_hashes()[&key], h0, "B's stale result must not install over a superseded band");
        assert_eq!(gr.band_encodes, 1, "nothing installed");

        // The redraw poll_v6_job's true return schedules re-stages C and lands it.
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        gr.spawn_band_jobs(&picker);
        settle_bands(&mut gr);
        assert_ne!(gr.chrome_band_hashes()[&key], h0, "the current content lands on the retry");
        assert_eq!(gr.band_encodes, 2);
    }

    #[test]
    fn an_invalidation_cancels_staged_band_encodes() {
        // SQ-1188: a resume/font-change invalidation must not let an in-flight
        // result resurrect an entry the invalidation just freed.
        let (mut gr, picker, mut chrome, scale, pane, band) = kitty_band_fixture();
        let key = (BandSlot::Art as u8, band.x, band.y, band.width, band.height);
        let mut buf = Buffer::empty(pane);

        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        chrome.put_pixel(1, 1, image::Rgba([200, 0, 0, 255]));
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        gr.spawn_band_jobs(&picker);
        gr.invalidate_chrome_bands();
        settle_bands(&mut gr);
        assert!(!gr.chrome_band_hashes().contains_key(&key), "the result found no entry and was dropped");
        assert!(!gr.band_encode_in_flight(), "the cancellation cleared every mark");
    }

    #[test]
    fn halfblocks_band_encodes_stay_synchronous() {
        // SQ-1188: half-blocks builds cells with no compression stage — the
        // worker would buy a frame of latency and save nothing, so a changed
        // band installs on the calling thread exactly as before (and every
        // cell-buffer harness stays deterministic).
        use crate::render::v6_layout::uniform_scale;
        let picker = Picker::halfblocks();
        let fs = picker.font_size();
        let (cw, ch) = (fs.width.max(1) as u32, fs.height.max(1) as u32);
        let (nw, nh) = (2 * cw, 4 * ch);
        let mut chrome = image::RgbaImage::from_pixel(nw, nh, image::Rgba([10, 20, 30, 255]));
        let pane = Rect::new(0, 0, 2, 4);
        let scale = uniform_scale((nw as u16, nh as u16), (nw, nh));
        let band = Rect::new(0, 0, 2, 1);
        let key = (BandSlot::Art as u8, band.x, band.y, band.width, band.height);
        let mut gr = GraphicsRender::default();
        let mut buf = Buffer::empty(pane);

        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        let h0 = gr.chrome_band_hashes()[&key];
        chrome.put_pixel(1, 1, image::Rgba([200, 0, 0, 255]));
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        assert_ne!(gr.chrome_band_hashes()[&key], h0, "half-blocks installs the change immediately");
        assert!(!gr.band_encode_in_flight(), "nothing is staged for a worker");
        assert_eq!(gr.band_encodes, 2);
    }

    #[test]
    fn scaled_chrome_shares_one_resize_and_matches_direct_resize() {
        // SQ-0514: the shared scaled-canvas cache returns pixels byte-identical to
        // a direct whole-canvas Nearest resize (so band output is unchanged), and
        // it memoises — reused for the same content/scale, rebuilt on a change.
        let mut gr = GraphicsRender::default();
        // A detailed (non-uniform) canvas so the resize actually samples pixels.
        let mut canvas = image::RgbaImage::new(17, 11);
        for (x, y, p) in canvas.enumerate_pixels_mut() {
            *p = image::Rgba([(x * 13) as u8, (y * 7) as u8, ((x + y) * 5) as u8, 255]);
        }
        let (s, sw, sh) = (2.5f32, 42u32, 27u32); // fractional scale → floor-map matters
        let expected = image::imageops::resize(&canvas, sw, sh, image::imageops::FilterType::Nearest);
        {
            let got = gr.scaled_chrome(&canvas, s, sw, sh);
            assert_eq!(got.as_raw(), expected.as_raw(), "shared scaled == direct whole-canvas resize");
        }
        let key0 = gr.chrome_scaled.as_ref().unwrap().0;
        // Same args → same cached key (memoised, no rebuild).
        let _ = gr.scaled_chrome(&canvas, s, sw, sh);
        assert_eq!(gr.chrome_scaled.as_ref().unwrap().0, key0, "identical content/scale reuses the cache");
        // A content change → rebuilt (key changes).
        canvas.put_pixel(0, 0, image::Rgba([1, 2, 3, 255]));
        let _ = gr.scaled_chrome(&canvas, s, sw, sh);
        assert_ne!(gr.chrome_scaled.as_ref().unwrap().0, key0, "a content change rebuilds the shared scaled canvas");
    }

    #[test]
    fn retain_live_empty_clears_all() {
        let picker = Picker::halfblocks();
        let mut gr = GraphicsRender::default();
        populate(&mut gr, &picker, &[1, 2]);
        assert_eq!(gr.cache.len(), 2);

        gr.retain_live(&std::collections::HashSet::new());
        assert_eq!(gr.cache.len(), 0);
    }

    // ── Kitty upload lifetime (SQ-0637) ──────────────────────────────────────

    /// Render `win` at `area` through the kitty path with `n` DISTINCT canvases,
    /// returning the id the terminal was told to keep.
    ///
    /// One id however many canvases pass through, since SQ-0995: each is
    /// transmitted, and each replaces the data behind the same id.
    fn upload_generations(gr: &mut GraphicsRender, picker: &Picker, win: u32, area: Rect, n: u8) -> u32 {
        let mut ids = std::collections::BTreeSet::new();
        for i in 0..n {
            let mut img = image::RgbaImage::from_pixel(64, 32, image::Rgba([9, 9, 9, 255]));
            img.put_pixel(0, 0, image::Rgba([i, 0, 0, 255]));
            let gw = GraphicsWindow { win, canvas: std::sync::Arc::new(img), version: i as u64 + 1, upscale: false };
            let mut buf = Buffer::empty(area);
            gr.render(picker, &gw, area, Style::default(), &mut buf);
            let sym = buf.cell((area.x, area.y)).unwrap().symbol().to_string();
            assert!(sym.contains("a=T"), "generation {i} is a new picture → uploaded");
            ids.insert(id_of(&sym));
        }
        assert_eq!(ids.len(), 1, "one stable id across {n} canvases: {ids:?}");
        ids.into_iter().next().expect("n >= 1")
    }

    #[test]
    fn closing_a_kitty_window_deletes_its_uploads() {
        // SQ-0637: `retain_live` used to forget a closed window's `KittyWindowImage`
        // silently. The terminal keeps a transmitted image until told to free it, so
        // a Glulx game that closes its graphics window (or reopens one under a new
        // id) leaked it.
        let picker = kitty_picker(8, 18);
        let area = Rect::new(0, 0, 8, 2);
        let mut gr = GraphicsRender::default();
        let id = upload_generations(&mut gr, &picker, 2, area, 3);

        gr.begin_band_log(); // frame boundary: only the close's ops below
        gr.retain_live(&std::collections::HashSet::new());
        let queued = gr.queued_deletes().to_string();
        assert!(
            queued.contains(&format!("a=d,d=I,i={id}")),
            "the closed window's upload is freed (missing i={id}): {queued:?}"
        );
        assert_eq!(
            gr.ops().iter().filter(|o| matches!(o, GraphicsOp::Drop { .. })).count(),
            1,
            "one Drop op per freed upload"
        );

        // The escapes reach the terminal through the buffer, like every other bit of
        // our kitty traffic — and the cell they ride on keeps its glyph and width.
        let mut buf = Buffer::empty(area);
        buf.cell_mut((0, 0)).unwrap().set_symbol("X");
        gr.flush_kitty_deletes(area, &mut buf);
        let sym = buf.cell((0, 0)).unwrap().symbol().to_string();
        assert!(sym.starts_with('\x1b') && sym.ends_with('X'), "deletes prepended, glyph kept: {sym:?}");
        assert_eq!(
            buf.cell((0, 0)).unwrap().diff_option,
            ratatui::buffer::CellDiffOption::ForcedWidth(std::num::NonZeroU16::new(1).unwrap()),
            "the escape must not be measured as visible width"
        );
        assert!(gr.queued_deletes().is_empty(), "flushed once, not re-emitted every frame");
    }

    #[test]
    fn resizing_a_kitty_window_deletes_the_uploads_it_abandons() {
        // A size change restarts the window's upload (the r×c grid is baked into the
        // transmission), so the old image can never be re-placed at the new size — it
        // must be freed, in the SAME batch as the new upload (SQ-0637). SQ-0995 made
        // the id stable ACROSS CONTENT, not across geometry: a resize still abandons
        // what it cannot use, and still names it.
        let picker = kitty_picker(8, 18);
        let mut gr = GraphicsRender::default();
        let old_id = upload_generations(&mut gr, &picker, 2, Rect::new(0, 0, 8, 2), 2);

        let bigger = Rect::new(0, 0, 12, 3);
        let mut buf = Buffer::empty(bigger);
        let gw = GraphicsWindow {
            win: 2,
            canvas: std::sync::Arc::new(image::RgbaImage::from_pixel(64, 32, image::Rgba([9, 9, 9, 255]))),
            version: 9,
            upscale: false,
        };
        gr.render(&picker, &gw, bigger, Style::default(), &mut buf);
        let sym = buf.cell((0, 0)).unwrap().symbol().to_string();
        assert!(
            sym.contains(&format!("a=d,d=I,i={old_id}")),
            "pre-resize upload i={old_id} freed: {sym:?}"
        );
        assert!(sym.contains("a=T"), "and the new size is transmitted in the same batch");
        assert!(
            !sym.contains(&format!("i={old_id},p=")),
            "under a NEW id — the transmit must not name the id it just freed: {sym:?}"
        );
        let delete_at = sym.find("a=d").expect("a delete");
        let transmit_at = sym.find("a=T").expect("a transmit");
        assert!(delete_at < transmit_at, "frees precede the transmit they make room for");
        assert!(gr.queued_deletes().is_empty(), "the placement carried them; nothing left queued");
        assert_eq!(gr.kitty_uploads(2).map(|(n, _)| n), Some(1), "one live upload after the resize");
    }

    #[test]
    fn queued_deletes_wait_for_a_safe_cell_and_are_never_dropped() {
        // The flush never disturbs a cell that belongs to a live placement (an
        // escape-carrying placeholder, or one marked Skip): the deletes stay queued
        // for a later frame instead (SQ-0637).
        let picker = kitty_picker(8, 18);
        let area = Rect::new(0, 0, 2, 1);
        let mut gr = GraphicsRender::default();
        upload_generations(&mut gr, &picker, 5, area, 1);
        gr.retain_live(&std::collections::HashSet::new());
        assert!(!gr.queued_deletes().is_empty(), "the close queued a delete");

        let mut buf = Buffer::empty(area);
        buf.cell_mut((0, 0)).unwrap().set_symbol("\x1b_Gplaceholder\x1b\\");
        buf.cell_mut((1, 0)).unwrap().set_diff_option(ratatui::buffer::CellDiffOption::Skip);
        let queued = gr.queued_deletes().to_string();
        gr.flush_kitty_deletes(area, &mut buf);
        assert_eq!(gr.queued_deletes(), queued, "no safe cell this frame → deferred, not lost");
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "\x1b_Gplaceholder\x1b\\", "placement untouched");

        // A later frame with an ordinary cell carries them.
        let mut buf2 = Buffer::empty(area);
        gr.flush_kitty_deletes(area, &mut buf2);
        assert!(gr.queued_deletes().is_empty(), "flushed on the next frame that has room");
    }

    /// The half of SQ-0772 that is not ours to encode: everything drawn through a
    /// `ratatui-image` [`Protocol`] — the v6 raster composite, every chrome band,
    /// inline art, the picker's covers — is placed by the crate's own kitty backend,
    /// one anchored cell per row followed by bare `Skip` continuations. [`place_protocol`]
    /// re-lays that row into the same self-describing, buffer-visible cells our own
    /// emitter writes, and this pins that it actually happens on a REAL protocol
    /// rather than on a hand-written imitation of one.
    #[test]
    fn a_ratatui_image_placement_is_reseated_into_self_describing_cells() {
        let picker = kitty_picker(8, 18);
        let (cols, rows) = (7u16, 3u16);
        let mut img = image::RgbaImage::new(u32::from(cols) * 8, u32::from(rows) * 18);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgba([(x % 256) as u8, (y % 256) as u8, 90, 255]);
        }
        let proto = picker
            .new_protocol(image::DynamicImage::ImageRgba8(img), Size::new(cols, rows), Resize::Fit(None))
            .expect("the kitty backend encodes a plain RGBA image");

        let dest = Rect::new(2, 1, cols, rows);
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 5));
        place_protocol(&proto, dest, &mut buf);

        let mut row_diacritics = Vec::new();
        let mut fg = None;
        for y in 0..rows {
            for x in 0..cols {
                let cell = buf.cell((dest.x + x, dest.y + y)).expect("inside the buffer");
                assert_eq!(
                    cell.diff_option, PLACEHOLDER_WIDTH,
                    "cell ({x},{y}) must be a width-1 placeholder, not a Skip the diff cannot see",
                );
                let sym = cell.symbol();
                let at = sym.find('\u{10EEEE}').unwrap_or_else(|| panic!("cell ({x},{y}): {sym:?}"));
                let mut d = sym[at..].chars().skip(1);
                let (row_d, col_d, _extra) = (d.next(), d.next(), d.next());
                assert_eq!(
                    col_d,
                    Some(KITTY_DIACRITICS[usize::from(x)]),
                    "cell ({x},{y}) must name its OWN column, not lean on its neighbour",
                );
                if x == 0 {
                    row_diacritics.push(row_d);
                }
                let seen = *fg.get_or_insert(cell.fg);
                assert_eq!(cell.fg, seen, "every cell of one image carries the same id colour");
            }
        }
        row_diacritics.dedup();
        assert_eq!(row_diacritics.len(), usize::from(rows), "each row names a different image row");

        // And the whole 32-bit id is readable back off the cells: the low 24 bits
        // in the foreground, the high byte as the third diacritic's index. That is
        // the thing SQ-0753 recorded as impossible ("the image id lives inside
        // ratatui-image's `Kitty` struct with no accessor, so we cannot name what to
        // delete") — worth pinning here, since a delete built on it would be silently
        // aiming at the wrong image if this ever stopped holding.
        let lead = buf.cell((dest.x, dest.y)).unwrap();
        let sym = lead.symbol().to_string();
        let Some(Color::Rgb(r, g, b)) = fg else { panic!("the id colour is truecolour") };
        let extra_d = sym[sym.find('\u{10EEEE}').unwrap()..].chars().nth(3).expect("the third diacritic");
        let high = KITTY_DIACRITICS.iter().position(|&c| c == extra_d).expect("a table entry");
        let recovered = ((high as u32) << 24) | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
        assert_eq!(recovered, id_of(&sym), "the id the upload declared, read back off the screen");
    }

    /// A kitty chrome ring at a fixed 1:1 scale: a native canvas exactly the pane's
    /// device size, so a band's crop is its own device rows and a pixel edit inside
    /// one band re-encodes only that band.
    fn band_fixture() -> (Picker, image::RgbaImage, crate::render::v6_layout::Scale, Rect, Rect, Buffer) {
        use crate::render::v6_layout::uniform_scale;
        let picker = kitty_picker(8, 18);
        let (cols, rows) = (4u16, 3u16);
        let (nw, nh) = (u32::from(cols) * 8, u32::from(rows) * 18);
        let mut chrome = image::RgbaImage::new(nw, nh);
        for (x, y, p) in chrome.enumerate_pixels_mut() {
            *p = image::Rgba([(x % 256) as u8, (y % 256) as u8, 40, 255]);
        }
        let pane = Rect::new(0, 0, cols, rows);
        let scale = uniform_scale((nw as u16, nh as u16), (nw, nh));
        let band = Rect::new(0, 0, cols, 1);
        let buf = Buffer::empty(pane);
        (picker, chrome, scale, pane, band, buf)
    }

    /// Every `a=d,d=I,i=` id in some emitted text, in order.
    fn delete_ids(text: &str) -> Vec<u32> {
        text.split("\x1b_Gq=2,a=d,d=I,i=")
            .skip(1)
            .filter_map(|s| s.split('\x1b').next()?.parse().ok())
            .collect()
    }

    /// Every `a=d,d=I,i=` id currently queued, in order.
    fn queued_delete_ids(gr: &GraphicsRender) -> Vec<u32> {
        delete_ids(gr.queued_deletes())
    }

    /// Every id this frame actually frees: the deletes riding out on a placement in
    /// `buf`, then anything still queued for a later frame. A delete lives in one
    /// place or the other and never in neither — which is the property worth
    /// asserting, since "queued" and "written" are both correct outcomes.
    fn freed_ids(gr: &GraphicsRender, buf: &Buffer) -> Vec<u32> {
        let cells: String = buf.content().iter().map(|c| c.symbol()).collect();
        let mut ids = delete_ids(&cells);
        ids.extend(queued_delete_ids(gr));
        ids
    }

    /// SQ-0753: a chrome band that goes away must be FREED in the terminal, not
    /// merely forgotten here.
    ///
    /// `retain_chrome_bands`/`invalidate_chrome_bands` recorded a `Drop` and dropped
    /// the `Protocol`, which releases nothing — `ratatui-image` has no output channel
    /// and never deletes. The terminal kept every band lanthorn ever encoded, and
    /// kitty evicts by LRU including images that are CURRENTLY PLACED, so a big
    /// enough pile can blank a live one. Naming the abandoned upload was the blocker
    /// the quest recorded ("the image id lives inside ratatui-image's `Kitty` struct
    /// with no accessor"); [`place_protocol`] now reads it back off the placement.
    #[test]
    fn abandoning_a_chrome_band_deletes_its_upload() {
        let (picker, chrome, scale, pane, band, mut buf) = band_fixture();
        let mut gr = GraphicsRender::default();
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);

        let key = (BandSlot::Art as u8, band.x, band.y, band.width, band.height);
        let id = gr.chrome_band_id(key).expect("a placed kitty band knows the id it lives under");
        assert!(gr.queued_deletes().is_empty(), "a band still on screen is not freed");

        gr.retain_chrome_bands(&std::collections::HashSet::new());
        assert!(gr.chrome_bands.is_empty(), "the band left the live set");
        assert_eq!(queued_delete_ids(&gr), vec![id], "the abandoned upload is freed by id");

        // …and the wholesale invalidation (a terminal clear, SQ-0587) does the same.
        let mut gr = GraphicsRender::default();
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        let id = gr.chrome_band_id(key).expect("placed");
        gr.invalidate_chrome_bands();
        assert_eq!(queued_delete_ids(&gr), vec![id], "invalidate frees what it drops");
    }

    /// SQ-0753's per-frame leak, and how SQ-0996 closed it for good: a band whose
    /// pixels changed re-encodes into the same cache slot, and the `insert` used to
    /// be the last anyone ever heard of its predecessor's id. Journey's picture
    /// column re-encodes on each menu step, so this is where the megabytes went.
    ///
    /// SQ-0753 answered it by DELETING the predecessor. SQ-0996 answers it by not
    /// creating one: the re-encode goes out under the id the band is already placed
    /// as, replacing the data behind it. There is no predecessor to orphan, and no
    /// delete — deleting that id would take the picture the frame just sent.
    ///
    /// A band therefore holds exactly ONE image in the terminal for as long as it
    /// keeps its slot, which is a tighter bound than "free the last one each time"
    /// and costs the whole placeholder rect less per frame.
    #[test]
    fn re_encoding_a_chrome_band_replaces_its_upload_in_place() {
        let (picker, mut chrome, scale, pane, band, mut buf) = band_fixture();
        let mut gr = GraphicsRender::default();
        let key = (BandSlot::Art as u8, band.x, band.y, band.width, band.height);

        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        let first = gr.chrome_band_id(key).expect("placed");

        // A cache HIT sends nothing and frees nothing — the upload is still live.
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        assert!(freed_ids(&gr, &buf).is_empty(), "a cache hit must not free the image it is re-placing");
        assert_eq!(gr.chrome_band_id(key), Some(first), "and it stays the same image");

        // Change a pixel inside this band's native footprint → re-encode. On
        // kitty the encode runs on the worker (SQ-1188): the change frame keeps
        // the old upload placed, and the next frame after the result lands is
        // the one that transmits the new pixels — under the SAME id.
        chrome.put_pixel(1, 2, image::Rgba([255, 0, 0, 255]));
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        assert_eq!(gr.chrome_band_id(key), Some(first), "the change frame still shows the old upload's id");
        gr.spawn_band_jobs(&picker);
        settle_bands(&mut gr);
        let mut buf = Buffer::empty(pane);
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        assert_eq!(
            gr.chrome_band_id(key),
            Some(first),
            "a re-encode re-transmits to the id the band already lives under (SQ-0996)"
        );
        assert!(
            freed_ids(&gr, &buf).is_empty(),
            "and frees nothing: that id is the one on screen, so an `a=d` for it would blank \
             the rect the frame just repainted"
        );
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            text.contains(&format!("i={first},")),
            "the new pixels did go out, under the unmoved id"
        );
    }

    /// SQ-0817: …and it rides BEHIND that placement, never ahead of it.
    ///
    /// The id being freed here is the one the terminal is drawing at this very rect,
    /// and its replacement is up to 618 KB of image data away (Zork Zero's banner
    /// band, measured). Freed first, those cells have nothing to draw for the length
    /// of that transfer — which is the flicker the compass, the on-screen map and
    /// Arthur's Merlin composite all showed, once per frame the game changed
    /// anything. SQ-0753 introduced it by putting every delete in the placement's
    /// PREFIX; before that nothing was ever freed, so nothing could blink.
    ///
    /// A frame's cells are emitted in row-major order, so their concatenated symbols
    /// are the frame's byte order: the assertion is simply that this delete is the
    /// last escape in it.
    ///
    /// **Driven at the mechanism, because SQ-0996 removed its last producer.** The
    /// two paths that used to supersede an upload — a chrome band re-encoding into
    /// its own slot, the composite replaced by the next encode — now re-transmit to
    /// the id already on screen and free nothing at all, so nothing reaches
    /// `deletes_after_place` through the ordinary v6 frame any more. The RULE is
    /// unchanged and the machinery is kept for whatever queues into it next: a
    /// delete for an id that is still covering its rect must be emitted after every
    /// byte of the placement that covers it again. Queuing one directly is the only
    /// way left to say so, and saying so is worth more than deleting the guard.
    #[test]
    fn the_upload_being_replaced_is_freed_only_after_its_replacement_is_placed() {
        let (picker, mut chrome, scale, pane, band, mut buf) = band_fixture();
        let mut gr = GraphicsRender::default();
        let key = (BandSlot::Art as u8, band.x, band.y, band.width, band.height);

        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        let first = gr.chrome_band_id(key).expect("placed");

        // The change is encoded on the worker (SQ-1188); the frame AFTER it lands
        // is the one that transmits the replacement, so that is the frame the
        // supersede delete is queued onto.
        chrome.put_pixel(1, 2, image::Rgba([255, 0, 0, 255]));
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        gr.spawn_band_jobs(&picker);
        settle_bands(&mut gr);
        // The replacement frame gets a clean buffer, so what it carries is its own —
        // and an upload that is still covering this very rect is queued to be freed
        // on it.
        let mut buf = Buffer::empty(pane);
        gr.queue_protocol_delete_after_place(Some(first));
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);

        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        let del = format!("\x1b_Gq=2,a=d,d=I,i={first}\x1b\\");
        let at = text.find(&del).expect("the superseded upload is freed in this frame");
        assert!(
            text[..at].contains("a=T,"),
            "the replacement was still un-transmitted when its predecessor was freed — the \
             rect draws nothing until the upload lands, which is the flicker"
        );
        assert_eq!(
            at,
            text.rfind("\x1b_G").expect("the frame carries APC traffic"),
            "the supersede delete must be the LAST escape of the frame: anything emitted after \
             it is traffic the freed image was still covering for"
        );
    }

    /// SQ-0753 for the biggest upload lanthorn makes: the v6 raster composite is the
    /// whole pane in one image (2.8 MB on Journey at 117x64). It is abandoned twice
    /// over — wholesale when the hybrid ring takes the screen (`invalidate_v6`), and
    /// once per visible change in a raster-mode game, where `poll_v6_job` installs
    /// the new encode over the old.
    #[test]
    fn abandoning_the_v6_raster_composite_deletes_its_upload() {
        let picker = kitty_picker(8, 18);
        let area = Rect::new(0, 0, 4, 3);
        let mut buf = Buffer::empty(area);
        let mut gr = GraphicsRender::default();
        let canvas = image::RgbaImage::from_pixel(32, 54, image::Rgba([9, 8, 7, 255]));

        gr.spawn_v6_encode(&picker, canvas.clone(), 1, area, RasterFrame::native((canvas.width() as u16, canvas.height() as u16)));
        gr.redraw_v6(&picker, area, &mut buf);
        let first = gr.v6.as_ref().and_then(|r| r.placed_id).expect("the composite knows its id");

        // A second generation replaces it on the worker thread — and since SQ-0996 it
        // is the SAME upload, re-transmitted: the composite's id is written into all
        // 12 of this pane's placeholder cells (3,680 at 117x64), so moving it would
        // repaint the pane to change the picture. Nothing is freed, because the id
        // being replaced is the id the frame is about to place.
        gr.spawn_v6_encode(&picker, canvas.clone(), 2, area, RasterFrame::native((canvas.width() as u16, canvas.height() as u16)));
        drain_v6_job(&mut gr);
        assert!(
            queued_delete_ids(&gr).is_empty() && gr.deletes_after_place.is_empty(),
            "a re-encode frees nothing: it replaces the data behind an id that never moved"
        );
        let mut frame = Buffer::empty(area);
        gr.redraw_v6(&picker, area, &mut frame);
        let second = gr.v6.as_ref().and_then(|r| r.placed_id).expect("placed");
        assert_eq!(second, first, "the new generation lives under the same id");

        let text: String = frame.content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains(&format!("i={first},")), "the new pixels went out under it");
        assert!(!text.contains("a=d,"), "and took nothing with them");

        // …but the raster→ring transition abandons the composite outright — there is
        // no re-transmit to replace it, so SQ-0637's delete still has to happen, and
        // nothing places after it (that is what the transition IS), so it waits for
        // the frame's closing flush.
        gr.invalidate_v6();
        assert_eq!(queued_delete_ids(&gr), vec![second], "invalidation frees the live composite");
        assert_eq!(freed_ids(&gr, &frame), vec![second], "which is the one upload there was");
    }

    /// The deletes above are worthless unless they reach the terminal, and the v6
    /// pixel paths have no placement of their own to piggyback on — the frame's
    /// closing flush is what carries them (SQ-0753). Pinned here as the property
    /// `main`'s end-of-frame flush relies on: a queued delete lands in the buffer
    /// as a real, diffable change.
    #[test]
    fn a_queued_protocol_delete_rides_out_on_the_frame() {
        let (picker, chrome, scale, pane, band, mut buf) = band_fixture();
        let mut gr = GraphicsRender::default();
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        let key = (BandSlot::Art as u8, band.x, band.y, band.width, band.height);
        let id = gr.chrome_band_id(key).expect("placed");
        gr.retain_chrome_bands(&std::collections::HashSet::new());

        // The next frame draws no band at all — ordinary cells everywhere.
        let mut next = Buffer::empty(pane);
        gr.flush_kitty_deletes(pane, &mut next);
        assert!(gr.queued_deletes().is_empty(), "the frame carried them");
        let carried: String = next.content().iter().map(|c| c.symbol()).collect();
        assert!(
            carried.contains(&format!("a=d,d=I,i={id}")),
            "the delete must be IN the frame's cells, or it is never written: {carried:?}"
        );
    }

    /// SQ-0976: what the terminal is handed for a kitty window upload.
    ///
    /// The claim under test is not "we wrote `o=z`" — that is a substring — but
    /// that the bytes behind it are a well-formed zlib stream of EXACTLY the
    /// canvas, reassembled from the chunks in the order they were emitted. A
    /// transmit that compresses each chunk separately, or that chunks the raw
    /// bytes and compresses after, or that declares the compressed length in `s`
    /// and `v`, all still contain `o=z` and all draw nothing.
    mod compressed_upload {
        use super::*;

        /// Standard-alphabet base64, decoded. The emitter hand-rolls the encoder
        /// (`kitty_b64`) rather than take a dependency, so the check has to be
        /// able to undo it independently — a shared codec proves nothing about
        /// either half.
        fn unb64(s: &str) -> Vec<u8> {
            const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut acc = 0u32;
            let mut bits = 0u32;
            let mut out = Vec::new();
            for c in s.bytes().filter(|&c| c != b'=') {
                let v = T.iter().position(|&t| t == c).unwrap_or_else(|| panic!("base64 alphabet: {c:?}"));
                acc = (acc << 6) | v as u32;
                bits += 6;
                if bits >= 8 {
                    bits -= 8;
                    out.push((acc >> bits) as u8);
                }
            }
            out
        }

        /// Split a transmit into `(first chunk's control keys, every payload
        /// concatenated in emission order)`.
        fn parse(transmit: &str) -> (String, Vec<u8>) {
            let mut keys = String::new();
            let mut payload = Vec::new();
            for (i, cmd) in transmit.split("\x1b_G").skip(1).enumerate() {
                let body = cmd.strip_suffix("\x1b\\").expect("every APC command is ST-terminated");
                let (params, b64) = body.split_once(';').expect("every transmit chunk has a payload");
                if i == 0 {
                    keys = params.to_string();
                } else {
                    assert!(
                        params.split(',').all(|kv| kv.starts_with("m=") || kv.starts_with("q=")),
                        "a continuation chunk may carry only `m` and `q`: {params}"
                    );
                }
                assert!(b64.len() <= 4096, "chunk of {} base64 bytes exceeds the protocol's 4096", b64.len());
                let last = params.contains("m=0");
                assert!(last || b64.len() % 4 == 0, "every chunk but the last must be a multiple of 4");
                payload.extend_from_slice(&unb64(b64));
            }
            (keys, payload)
        }

        /// A canvas whose pixels are not all one colour, or a codec bug that
        /// dropped everything after the first chunk would still round-trip.
        fn canvas(w: u32, h: u32) -> image::RgbaImage {
            image::RgbaImage::from_fn(w, h, |x, y| {
                image::Rgba([(x % 251) as u8, (y % 241) as u8, ((x * 7 + y * 13) % 239) as u8, 255])
            })
        }

        /// SQ-1005: the transmit measures itself, off its own control block.
        ///
        /// The point of measuring the WIRE rather than an encoder is that one
        /// number then speaks for both encoders — lanthorn emits its
        /// graphics-window uploads and `ratatui-image` encodes everything else, and
        /// neither hands back the two lengths. So the parser has to hold against a
        /// chunked transmit (a continuation chunk carries no geometry and must not
        /// be counted as a second image) and against a raw one.
        #[test]
        fn a_transmit_reports_its_wire_cost_and_the_pixels_it_stands_for() {
            let img = canvas(640, 400);
            let pixels = 640u64 * 400 * 4;

            let raw = kitty_transmit_virtual(&img, 7, 25, 80, false);
            let m = measure_transmit(&raw);
            assert_eq!(m.uploads, 1, "one image, however many chunks it took");
            assert_eq!(m.pixels, pixels, "s x v x 4 for f=32 RGBA");
            assert_eq!(m.wire, raw.len() as u64, "an uncompressed transmit IS its wire cost");
            assert!(
                m.wire >= m.wire_uncompressed(),
                "base64 alone cannot come out smaller than base64: {} vs {}",
                m.wire,
                m.wire_uncompressed(),
            );

            let zipped = kitty_transmit_virtual(&img, 7, 25, 80, true);
            let z = measure_transmit(&zipped);
            assert_eq!(z.uploads, 1, "still one image");
            assert_eq!(z.pixels, pixels, "the declared geometry is the IMAGE, not the payload");
            assert_eq!(z.wire, zipped.len() as u64);
            assert_eq!(
                z.wire_uncompressed(),
                m.wire_uncompressed(),
                "the same pixels stand for the same uncompressed wire either way",
            );

            // The measurement's whole purpose, and the number worth quoting — but
            // NOT on `canvas()`, whose pixels are pseudo-random on purpose so that
            // a codec bug dropping every chunk but the first would still be caught.
            // Noise does not deflate: that canvas measures 1,369,725 raw against
            // 973,792 compressed, a ratio of 1.4 that says nothing about a game.
            //
            // Infocom v6 art is a 16-colour palette in flat runs, which is what
            // `o=z` is worth having for, so the ratio is asserted on a canvas
            // shaped like one.
            let art = image::RgbaImage::from_fn(640, 400, |x, y| {
                let band = ((y / 24) + (x / 80)) % 16;
                image::Rgba([band as u8 * 17, 40 + band as u8 * 9, 90, 255])
            });
            let (flat_raw, flat_z) = (
                measure_transmit(&kitty_transmit_virtual(&art, 7, 25, 80, false)),
                measure_transmit(&kitty_transmit_virtual(&art, 7, 25, 80, true)),
            );
            assert!(
                flat_z.wire * 20 < flat_raw.wire,
                "on artwork-shaped pixels compression must be worth an order of magnitude: \
                 {} vs {}",
                flat_z.wire,
                flat_raw.wire,
            );
            eprintln!(
                "640x400 flat art: {} pixel bytes, {} on the wire raw, {} compressed ({:.1}x)",
                flat_z.pixels,
                flat_raw.wire,
                flat_z.wire,
                flat_raw.wire as f64 / flat_z.wire.max(1) as f64,
            );
            eprintln!(
                "640x400 noise:    {} pixel bytes, {} on the wire raw, {} compressed ({:.1}x)",
                z.pixels,
                m.wire,
                z.wire,
                m.wire as f64 / z.wire.max(1) as f64,
            );
        }

        /// Two transmits in one string are two images, and a stream with none is
        /// zero — the accumulator adds these up across a session, so a parser that
        /// double-counted or fell off the end would drift silently.
        #[test]
        fn measuring_counts_images_and_not_chunks() {
            assert_eq!(measure_transmit(""), UploadBytes::default());
            assert_eq!(measure_transmit("just some cells\u{10eeee}"), UploadBytes::default());

            let a = kitty_transmit_virtual(&canvas(64, 32), 1, 2, 8, true);
            let b = kitty_transmit_virtual(&canvas(16, 16), 2, 1, 2, false);
            let both = measure_transmit(&format!("{a}{b}"));
            assert_eq!(both.uploads, 2);
            assert_eq!(both.pixels, 64 * 32 * 4 + 16 * 16 * 4);
            assert_eq!(both.wire, (a.len() + b.len()) as u64);
        }

        // ── measure_traffic: pairing a whole capture's deletes against its
        // transmits (SQ-1201) ──────────────────────────────────────────────────

        /// A transmit followed by the `a=d` that frees it: `freed_pixels` credits
        /// the id, and it no longer counts as stranded.
        ///
        /// Falsified by hand: commenting out the `outstanding.remove(&id)` credit
        /// in `measure_traffic` (crediting nothing and leaving the id stranded)
        /// turns this into `freed_pixels: 0, stranded_uploads: 1` and fails both
        /// assertions below — the pairing is load-bearing, not a tautology of
        /// `deletes == 1`.
        #[test]
        fn a_transmit_and_its_later_delete_pair_into_freed_pixels() {
            let pixels = 64u64 * 32 * 4;
            let transmit = kitty_transmit_virtual(&canvas(64, 32), 0x00B0_0001, 2, 8, false);
            let text = format!("{transmit}{}", kitty_delete_escape(0x00B0_0001));
            let m = measure_traffic(&text);
            assert_eq!(m.uploads, 1);
            assert_eq!(m.pixels, pixels);
            assert_eq!(m.deletes, 1);
            assert_eq!(m.freed_pixels, pixels, "the delete named the transmit's own id");
            assert_eq!(m.stranded_uploads, 0, "freed, not stranded");
            assert_eq!(m.stranded_pixels, 0);
        }

        /// A transmit with no delete anywhere in the capture is stranded: still
        /// resident in the terminal as far as this measurement can tell.
        #[test]
        fn a_transmit_with_no_delete_is_stranded() {
            let pixels = 64u64 * 32 * 4;
            let transmit = kitty_transmit_virtual(&canvas(64, 32), 0x00B0_0002, 2, 8, false);
            let m = measure_traffic(&transmit);
            assert_eq!(m.deletes, 0);
            assert_eq!(m.freed_pixels, 0);
            assert_eq!(m.stranded_uploads, 1);
            assert_eq!(m.stranded_pixels, pixels);
        }

        /// A delete naming an id nothing in this capture transmitted still counts
        /// as a delete COMMAND, but frees nothing — there is no pixel size in this
        /// text to credit it against (the transmit that set it happened earlier,
        /// outside this capture, or it was already freed once).
        #[test]
        fn a_delete_for_an_unknown_id_is_counted_but_not_credited() {
            let m = measure_traffic(&kitty_delete_escape(0x00B0_00FF));
            assert_eq!(m.deletes, 1);
            assert_eq!(m.freed_pixels, 0);
            assert_eq!(m.stranded_uploads, 0, "nothing was transmitted here to strand");
        }

        /// The measurer keys on `a=d` alone; the `d=` value (`I` frees the image
        /// data and every placement, `i` frees one placement) never enters the
        /// classification, so a hand-built `d=i` pairs exactly like `d=I` does.
        /// lanthorn itself only ever emits `d=I` (`kitty_delete_escape`) — this
        /// documents that the OTHER spelling the kitty spec allows is not silently
        /// mis-measured if it is ever emitted, without inventing a form nobody
        /// sends today.
        #[test]
        fn d_lowercase_i_deletes_pair_exactly_like_d_uppercase_i() {
            let pixels = 16u64 * 16 * 4;
            let transmit = kitty_transmit_virtual(&canvas(16, 16), 0x00B0_0003, 1, 2, false);
            let lowercase_delete = "\x1b_Gq=2,a=d,d=i,i=11534339\x1b\\"; // 0x00B0_0003
            let m = measure_traffic(&format!("{transmit}{lowercase_delete}"));
            assert_eq!(m.deletes, 1);
            assert_eq!(m.freed_pixels, pixels, "d=i pairs by id exactly like d=I");
            assert_eq!(m.stranded_uploads, 0);
        }

        /// A re-transmit to an id already held REPLACES it (the kitty spec's own
        /// rule for re-transmitting to an existing id) — the ledger holds the
        /// LATEST size under that id, not the sum of every transmit to it, so a
        /// window re-transmitting three times and never being deleted is one
        /// stranded upload at its last size, not three.
        #[test]
        fn a_retransmit_to_the_same_id_replaces_the_ledger_entry_not_adds_to_it() {
            let id = 0x00B0_0004;
            let first = kitty_transmit_virtual(&canvas(8, 8), id, 1, 1, false);
            let second = kitty_transmit_virtual(&canvas(64, 64), id, 2, 2, false);
            let m = measure_traffic(&format!("{first}{second}"));
            assert_eq!(m.uploads, 2, "both transmits still cost the wire");
            assert_eq!(m.stranded_uploads, 1, "one id, held once");
            assert_eq!(m.stranded_pixels, 64 * 64 * 4, "the LATEST size, not 8x8 + 64x64");
        }

        /// SQ-1201: the kitty capability PROBE lanthorn sends at startup (`a=q`) also
        /// declares `s=1,v=1` — a tiny throwaway image, never placed and never meant
        /// to be freed — and a whole-capture measurement sees it right beside the
        /// real transmits. `s`/`v` alone cannot be "this is an upload"; the action
        /// has to be `a=T`/`a=t`, or a capability probe on a real capture inflates
        /// `uploads` and manufactures a phantom stranded id for every query sent.
        #[test]
        fn a_capability_query_is_not_counted_as_an_upload() {
            let query = "\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\";
            let m = measure_traffic(query);
            assert_eq!(m.uploads, 0, "a query is not an upload");
            assert_eq!(m.stranded_uploads, 0, "and nothing here to strand");

            // A real transmit alongside it is still counted normally.
            let transmit = kitty_transmit_virtual(&canvas(16, 16), 0x00B0_0005, 1, 2, false);
            let m = measure_traffic(&format!("{query}{transmit}"));
            assert_eq!(m.uploads, 1, "the query still does not count");
            assert_eq!(m.stranded_uploads, 1, "only the real transmit is stranded");
        }

        #[test]
        fn the_transmit_declares_o_z_and_the_canvas_own_uncompressed_dimensions() {
            let img = canvas(640, 400);
            let (keys, _) = parse(&kitty_transmit_virtual(&img, 0x00B0_0001, 25, 80, true));
            assert!(keys.contains(",o=z"), "the payload is compressed and must say so: {keys}");
            assert!(keys.contains(",f=32"), "`o=z` is the encoding; `f` is still the format: {keys}");
            // s/v name the image, not the payload. 640x400 is 1,024,000 raw bytes
            // and a few thousand compressed, so a transmit that confused the two
            // would be caught here and nowhere else.
            assert!(keys.contains(",s=640,v=400,"), "s/v are the UNCOMPRESSED pixel dimensions: {keys}");
            assert!(keys.contains(",r=25,c=80,"), "the explicit placeholder grid survives (SQ-0520)");
            assert!(!keys.contains("S="), "`S` is for PNG-plus-compression only, and this is f=32");
        }

        #[test]
        fn inflating_the_reassembled_payload_reproduces_the_canvas_byte_for_byte() {
            for (w, h) in [(640u32, 400u32), (232, 304), (1104, 36), (1, 1)] {
                let img = canvas(w, h);
                let (_, payload) = parse(&kitty_transmit_virtual(&img, 7, 2, 8, true));
                let mut raw = Vec::new();
                std::io::copy(&mut flate2::read::ZlibDecoder::new(&payload[..]), &mut raw)
                    .unwrap_or_else(|e| panic!("{w}x{h}: the payload must be one zlib stream: {e}"));
                assert_eq!(raw.len(), (w * h * 4) as usize, "{w}x{h}: kitty sizes its buffer from s*v*4");
                assert_eq!(&raw, img.as_raw(), "{w}x{h}: the inflated payload is the canvas");
            }
        }

        /// The point of the exercise, kept as a number so a regression that
        /// silently stops compressing is a failure rather than a slow terminal.
        #[test]
        fn a_flat_artwork_canvas_costs_a_fraction_of_its_raw_upload() {
            // Sixteen colours in horizontal bands — the shape of every v6 frame.
            let img = image::RgbaImage::from_fn(640, 400, |_x, y| {
                let c = ((y / 25) * 17) as u8;
                image::Rgba([c, c / 2, 255 - c, 255])
            });
            let transmit = kitty_transmit_virtual(&img, 7, 25, 80, true);
            let raw_b64 = 1024000usize.div_ceil(3) * 4;
            assert!(
                transmit.len() * 20 < raw_b64,
                "a 16-colour 640x400 frame must cost under a twentieth of its raw upload: \
                 {} bytes against {raw_b64}",
                transmit.len()
            );
        }

        /// SQ-0997: a terminal that did not answer the `o=z` probe is sent the
        /// pixels RAW, and the transmission is otherwise the same command.
        ///
        /// The claim is not "we omitted `o=z`" but that the bytes behind the
        /// omission are the canvas itself: a transmit that dropped the key while
        /// still deflating the payload contains no `o=z` just the same, and a real
        /// terminal would read `s*v*4` bytes of image out of a few thousand bytes
        /// of zlib and store nothing. Every geometry key is unchanged, because
        /// `o=z` describes the payload's encoding and nothing about the image.
        #[test]
        fn a_terminal_that_cannot_inflate_is_sent_the_canvas_raw() {
            let img = canvas(232, 304);
            let (keys, payload) = parse(&kitty_transmit_virtual(&img, 0x00B0_0001, 19, 29, false));
            assert!(!keys.contains("o=z"), "nothing may claim to be compressed: {keys}");
            assert!(keys.contains(",f=32"), "the format is still RGBA: {keys}");
            assert!(keys.contains(",s=232,v=304,"), "the pixel dimensions do not move: {keys}");
            assert!(keys.contains(",r=19,c=29,"), "nor does the explicit placeholder grid: {keys}");
            assert_eq!(payload, *img.as_raw(), "the payload IS the canvas, undeflated");
        }
    }

    /// SQ-0997: the graphics-window encoder asks the picker whether this terminal
    /// can inflate, exactly as `ratatui-image` has since SQ-0991.
    ///
    /// A capability list can only be filled by `Picker::from_query_stdio`, which
    /// needs a terminal — so every picker a headless test can build reports "no",
    /// and "no" is the answer that matters: an `o=z` transmission such a terminal
    /// cannot inflate is REFUSED, the image is never stored, and every placeholder
    /// cell naming it draws nothing at all. The window is simply empty, silently.
    ///
    /// FALSIFY by restoring the unconditional `o=z` in `kitty_transmit_virtual`:
    /// this fails on the first assertion, with the frame claiming a compression
    /// nobody agreed to.
    #[test]
    fn a_graphics_window_transmit_is_raw_when_the_picker_knows_of_no_compression() {
        let img = image::RgbaImage::from_fn(64, 36, |x, y| {
            image::Rgba([(x % 251) as u8, (y % 241) as u8, 0x40, 255])
        });
        let gw = GraphicsWindow { win: 4, canvas: std::sync::Arc::new(img), version: 1, upscale: false };
        let picker = kitty_picker(8, 18);
        assert!(
            !kitty_compression(&picker),
            "an unqueried picker carries no capabilities, so it cannot promise inflation"
        );

        let area = Rect::new(0, 0, 8, 2);
        let mut gr = GraphicsRender::default();
        let mut buf = Buffer::empty(area);
        gr.render(&picker, &gw, area, Style::default(), &mut buf);

        let first = buf.cell((0, 0)).unwrap().symbol().to_string();
        assert!(first.contains("a=T,U=1,f=32,t=d"), "the transmit is there and states no encoding: {first:?}");
        assert!(
            !first.contains("o=z"),
            "the terminal never said it could inflate, so the transmit must not say it did: {first:?}"
        );
    }

    /// SQ-0988: the terminal's cell size is measured once at launch, so a font
    /// change mid-session leaves every fit running on the launch cell.
    ///
    /// The absolute size does not matter — geometry multiplies by `fw`/`fh` to
    /// reach a device box and divides by them again, so a uniform scale error
    /// cancels. The ASPECT RATIO is what survives, and it genuinely moves between
    /// adjacent font sizes: a cell is `round(advance_em · px)` by
    /// `round(line_em · px)`, and the two round at different rates.
    mod cell_size_change {
        use super::*;

        /// A kitty picker at a stated cell size, the way the app's other headless
        /// paths already build one.
        fn kitty(w: u16, h: u16) -> Picker {
            #[allow(deprecated)]
            let mut p = Picker::from_fontsize(ratatui_image::FontSize::new(w, h));
            p.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
            p
        }

        /// The claim the whole quest rests on, in arithmetic: the SAME cell rect
        /// at a different cell SHAPE fits a different box.
        ///
        /// 4x7 and 4x9 are FiraCode at 6 px and 7 px — a face whose design ratio
        /// is 2.002, yielding real cells of 1.750 and 2.250. Two adjacent font
        /// sizes, one keystroke apart.
        #[test]
        fn the_same_cell_rect_at_a_different_cell_shape_fits_a_different_box() {
            let target = Size::new(80, 25);
            let src = (640u32, 400u32);
            let tall = fit_geometry(ratatui_image::FontSize::new(4, 7), src, target, false);
            let taller = fit_geometry(ratatui_image::FontSize::new(4, 9), src, target, false);
            assert_ne!(
                (tall.cells.width, tall.cells.height),
                (taller.cells.width, taller.cells.height),
                "80x25 cells fits {}x{} at a 1.750 cell and {}x{} at a 2.250 one — were these \
                 equal there would be nothing to fix",
                tall.cells.width,
                tall.cells.height,
                taller.cells.width,
                taller.cells.height
            );
        }

        /// And the raster composite is encoded FOR the cell it was built against:
        /// two pickers, one area, two different pictures.
        #[test]
        fn the_raster_composite_is_encoded_for_the_cell_it_was_built_against() {
            let canvas = image::RgbaImage::from_fn(320, 200, |x, y| {
                image::Rgba([(x % 256) as u8, (y % 256) as u8, 90, 255])
            });
            let area = Rect::new(0, 0, 80, 25);
            let a = GraphicsRender::encode_v6(&kitty(4, 7), &canvas, 1, area, RasterFrame::native((canvas.width() as u16, canvas.height() as u16)), None)
                .expect("a kitty encode of a real canvas");
            let b = GraphicsRender::encode_v6(&kitty(4, 9), &canvas, 1, area, RasterFrame::native((canvas.width() as u16, canvas.height() as u16)), None)
                .expect("a kitty encode of a real canvas");
            assert_ne!(
                (a.proto.size().width, a.proto.size().height),
                (b.proto.size().width, b.proto.size().height),
                "the composite is fitted to the cell, so it cannot be the same picture at both"
            );
        }

        /// …and yet the key that decides whether to rebuild it cannot see the
        /// difference. The defect, stated as a test.
        #[test]
        fn the_composite_survives_a_font_change_until_the_caches_are_told() {
            let canvas = image::RgbaImage::from_pixel(320, 200, image::Rgba([10, 20, 30, 255]));
            let area = Rect::new(0, 0, 80, 25);
            let mut gr = GraphicsRender::default();
            assert!(gr.v6_wants_build(1, area), "nothing is built yet");
            let dims = (canvas.width() as u16, canvas.height() as u16);
            gr.spawn_v6_encode(&kitty(4, 7), canvas, 1, area, RasterFrame::native(dims));
            assert!(!gr.v6_wants_build(1, area), "the first encode installs a composite");

            // The pane is still 80x25 cells; only the cells changed shape. Nothing
            // in the key moved, so the composite fitted to a 1.750 cell is what the
            // pane would go on drawing at 2.250.
            assert!(
                !gr.v6_wants_build(1, area),
                "the key is (gen, cols, rows) and a font change moves none of the three — \
                 which is precisely why the invalidation below has to be explicit"
            );

            gr.invalidate_cell_geometry();
            assert!(gr.v6_wants_build(1, area), "once the cell moved, the composite must be rebuilt");
        }

        /// Which caches the invalidation drops, and — just as load-bearing — which
        /// it must NOT.
        ///
        /// `kitty_wins` shares `cache`'s key shape and is nonetheless immune: a
        /// virtual placement sends the canvas at native size and names an `r×c`
        /// grid, so the terminal rescales to the new cell rect on its own.
        /// Dropping it would re-upload a whole canvas to arrive at the same
        /// pixels, which on a v6 window is megabytes for nothing.
        #[test]
        fn invalidating_drops_what_was_fitted_to_the_cell_and_keeps_what_was_not() {
            let img = image::RgbaImage::from_pixel(64, 32, image::Rgba([7, 7, 7, 255]));
            let gw =
                GraphicsWindow { win: 7, canvas: std::sync::Arc::new(img), version: 1, upscale: false };
            let area = Rect::new(0, 0, 20, 4);
            let mut gr = GraphicsRender::default();

            // A kitty window upload…
            let mut buf = Buffer::empty(area);
            gr.render(&kitty(8, 18), &gw, area, Style::default(), &mut buf);
            // …a non-kitty window protocol, on a second window…
            let gw2 = GraphicsWindow { win: 8, ..gw.clone() };
            let mut sixel = kitty(8, 18);
            sixel.set_protocol_type(ratatui_image::picker::ProtocolType::Sixel);
            let mut buf2 = Buffer::empty(area);
            gr.render(&sixel, &gw2, area, Style::default(), &mut buf2);
            // …and a raster composite.
            gr.spawn_v6_encode(&kitty(8, 18), (*gw.canvas).clone(), 1, area, RasterFrame::native((gw.canvas.width() as u16, gw.canvas.height() as u16)));

            let (cache, _bands, kitty_wins, v6) = gr.cell_keyed_cache_sizes();
            assert_eq!(
                (cache, kitty_wins, v6),
                (1, 1, true),
                "the fixture must actually populate all three, or the assertions below pass \
                 vacuously"
            );

            gr.invalidate_cell_geometry();
            let (cache, bands, kitty_wins, v6) = gr.cell_keyed_cache_sizes();
            assert_eq!(cache, 0, "the non-kitty window protocol was encoded at the old device box");
            assert_eq!(bands, 0, "chrome bands go with it, so the ring cannot outlive the composite");
            assert!(!v6, "the raster composite was resampled to the old device box");
            assert_eq!(
                kitty_wins, 1,
                "a virtual placement is scaled to its r×c grid BY THE TERMINAL, so its upload is \
                 still correct — re-sending it would spend a whole canvas for nothing"
            );
        }
    }
}
