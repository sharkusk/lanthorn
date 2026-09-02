//! v6 layout classification: split the engine's flat window list into the
//! single scrolling story window (a primary `Buffer`) and everything else
//! (chrome — frame graphics, status grids, etc.). Pure classification, no
//! rendering (Phase 1a).

use image::{Rgba, RgbaImage};

use crate::colors::ColorScheme;
use crate::engine::{PositionedWindow, PxText, WinNode};
use zvm::screen::V6Cell;

/// Resolve a packed z-colour (see `crate::state::pack_zcolour`) to an opaque
/// RGBA. `0` (Default) → `fallback`. True24 → its RGB. Palette/standard colours
/// resolve through the theme; anything that doesn't reduce to a concrete RGB
/// falls back (v1 — richer palette handling is SQ-0450).
/// A packed z-colour (see [`crate::state::pack_zcolour`]) is EXPLICIT only when
/// the game named a real colour. `ZColour::Default` (0) and Standard 0/1
/// ("current"/"default", ZMSD §8.3.1) are not choices — they're inheritance —
/// so they are NOT explicit and the theme keeps the channel. Standard 2-9 and
/// every True/True24 value ARE explicit. Shared by the raster block-paint
/// decision and the cell colour paths so both gate identically. (SQ-0487/0488)
pub(crate) fn packed_explicit(packed: u32) -> bool {
    packed != 0 && !((packed >> 24) == 1 && (packed & 0xFF) <= 1)
}

/// The ZMSD §8.3.1 recommended true-colour equivalents for the Standard palette
/// colours 2..=9. On the pixel/canvas paths we resolve Standard colours to these
/// DOS/spec-authentic RGBs directly rather than routing them through the theme's
/// ANSI palette, which a user theme may remap arbitrarily (SQ-0506). Greys
/// (10..=12) still go through `resolve_zcolour`/`grey_rgb`, which already carry
/// their own fixed RGB.
fn standard_pixel_rgb(n: u8) -> Option<Rgba<u8>> {
    // SQ-0532/A-F5: the table itself now lives in `colors::STANDARD_COLOUR_RGB15`
    // so the terminal cell palette resolves Standard colours to the SAME §8.3.1
    // RGBs this pixel path uses (they used to disagree — e.g. white).
    let (r, g, b) = crate::colors::standard_colour_rgb(n)?;
    Some(Rgba([r, g, b, 255]))
}

/// The pixel colour a packed z-colour names OUTRIGHT, or `None` when it names
/// none (SQ-0706).
///
/// Every case here resolves without a [`ColorScheme`]: true-colour is arithmetic,
/// and Standard 2..=9 have fixed RGB in ZMSD §8.3.1 (see [`standard_pixel_rgb`]),
/// which is what the pixel path already uses so that white is real white rather
/// than VGA grey. `Default`, and the "current"/"default" sentinels 0 and 1, name
/// no colour: they mean "inherit", which is the host's business, not a painted
/// rectangle's.
///
/// This is what lets `GameSession` rasterize `erase_window` fills into a bounded
/// surface as they arrive, instead of hoarding an unbounded list of rects to
/// resolve later against a theme it cannot see.
pub(crate) fn explicit_pixel_rgba(packed: u32) -> Option<Rgba<u8>> {
    match packed >> 24 {
        3 => {
            let v = packed & 0x00FF_FFFF;
            Some(Rgba([(v >> 16) as u8, (v >> 8) as u8, v as u8, 255]))
        }
        2 => {
            let v = (packed & 0xFFFF) as u16;
            let (r, g, b) = ((v & 0x1F) as u8, ((v >> 5) & 0x1F) as u8, ((v >> 10) & 0x1F) as u8);
            // 5 bits per channel → 8, replicating the high bits (0x1F → 0xFF).
            Some(Rgba([(r << 3) | (r >> 2), (g << 3) | (g >> 2), (b << 3) | (b >> 2), 255]))
        }
        1 => standard_pixel_rgb((packed & 0xFF) as u8),
        _ => None,
    }
}

pub(crate) fn packed_to_rgba(packed: u32, fallback: Rgba<u8>, colors: &ColorScheme) -> Rgba<u8> {
    if packed == 0 {
        return fallback;
    }
    let tag = packed >> 24;
    if tag == 3 {
        let v = packed & 0x00FF_FFFF;
        return Rgba([(v >> 16) as u8, (v >> 8) as u8, v as u8, 255]);
    }
    // Standard(n)=tag 1, True(v)=tag 2 → reconstruct the ZColour and resolve via
    // the scheme; use the concrete RGB when the theme yields one, else fallback.
    let z = match tag {
        1 => zvm::screen::ZColour::Standard((packed & 0xFF) as u8),
        2 => zvm::screen::ZColour::True((packed & 0xFFFF) as u16),
        _ => return fallback,
    };
    // Pixel path: Standard 2..=9 resolve to their ZMSD §8.3.1 true-colour RGB,
    // bypassing the theme ANSI palette so white is real white, not VGA grey.
    if let zvm::screen::ZColour::Standard(n) = z {
        if let Some(rgb) = standard_pixel_rgb(n) {
            return rgb;
        }
    }
    color_to_rgba(crate::render::resolve_zcolour(z, colors), fallback)
}

/// Resolve a ratatui [`Color`] to an opaque RGBA for the pixel canvas. The cell
/// path renders NAMED ANSI colours (the terminal_default palette maps Standard
/// 2–9 to `Color::Red`/`Color::Blue`/… — the terminal draws them directly), but
/// the raster canvas needs concrete bytes: mapping only `Color::Rgb` dropped
/// every palette colour to the fallback, so Zork Zero's compass-direction
/// letters blitted in the default ink instead of their own colour (SQ-0480). The
/// 16 base ANSI colours resolve to the standard VGA RGB values; `Reset` and
/// `Indexed` (no canonical RGB here) fall back.
pub(crate) fn color_to_rgba(c: ratatui::style::Color, fallback: Rgba<u8>) -> Rgba<u8> {
    use ratatui::style::Color;
    let (r, g, b) = match c {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Black => (0, 0, 0),
        Color::Red => (170, 0, 0),
        Color::Green => (0, 170, 0),
        Color::Yellow => (170, 85, 0),
        Color::Blue => (0, 0, 170),
        Color::Magenta => (170, 0, 170),
        Color::Cyan => (0, 170, 170),
        Color::Gray => (170, 170, 170),
        Color::DarkGray => (85, 85, 85),
        Color::LightRed => (255, 85, 85),
        Color::LightGreen => (85, 255, 85),
        Color::LightYellow => (255, 255, 85),
        Color::LightBlue => (85, 85, 255),
        Color::LightMagenta => (255, 85, 255),
        Color::LightCyan => (85, 255, 255),
        Color::White => (255, 255, 255),
        Color::Reset | Color::Indexed(_) => return fallback,
    };
    Rgba([r, g, b, 255])
}

/// 1:1 opaque-over blit of `src` into `dst` at `(dx, dy)`, clipped to the
/// `max_w × max_h` box anchored at `(dx, dy)` (a v6 window's pixel box).
pub(crate) fn blit_clipped(dst: &mut RgbaImage, src: &RgbaImage, dx: u32, dy: u32, max_w: u32, max_h: u32) {
    let w = src.width().min(max_w);
    let h = src.height().min(max_h);
    let (dstw, dsth) = (dst.width(), dst.height());
    for oy in 0..h {
        let ty = dy + oy;
        if ty >= dsth {
            break;
        }
        for ox in 0..w {
            let tx = dx + ox;
            if tx >= dstw {
                break;
            }
            let p = *src.get_pixel(ox, oy);
            if p[3] >= 128 {
                dst.put_pixel(tx, ty, Rgba([p[0], p[1], p[2], 255]));
            }
        }
    }
}

/// Like [`blit_clipped`], but starts reading `src` at row `src_y` — for a
/// margin float partially scrolled off the top of the story view.
pub(crate) fn blit_clipped_src(dst: &mut RgbaImage, src: &RgbaImage, dx: u32, dy: u32, src_y: u32, max_w: u32, max_h: u32) {
    let w = src.width().min(max_w);
    let h = src.height().saturating_sub(src_y).min(max_h);
    let (dstw, dsth) = (dst.width(), dst.height());
    for oy in 0..h {
        let ty = dy + oy;
        if ty >= dsth {
            break;
        }
        for ox in 0..w {
            let tx = dx + ox;
            if tx >= dstw {
                break;
            }
            let p = *src.get_pixel(ox, src_y + oy);
            if p[3] >= 128 {
                dst.put_pixel(tx, ty, Rgba([p[0], p[1], p[2], 255]));
            }
        }
    }
}

/// The DEFAULT v6 font cell in game pixels — `zvm::screen::V6Cell::DEFAULT`
/// restated for the cases below, which build canvases in whole cells.
///
/// Production code does NOT read these. Since SQ-0917 the cell is per-session
/// state that a profile can change (the Macintosh's is 7x15), so every renderer
/// takes a [`V6Cell`] and quantizes by IT — a module constant here would be a
/// second answer to a question the engine has already answered, and the two
/// would disagree the moment a profile declares its own. The cases keep them
/// because a test that builds a `10 * FONT_W` canvas is describing its own
/// fixture, not asserting the machine's cell.
#[cfg(all(test, feature = "t-render"))]
pub(crate) const FONT_W: u32 = 8;
#[cfg(all(test, feature = "t-render"))]
pub(crate) const FONT_H: u32 = 16;

/// A window-0 inline picture floated beside the story text: anchored to a
/// wrapped display row, reserving columns for the picture and narrowing the rows
/// beside it. `row` is relative to the visible window and may be negative when
/// the float has partially scrolled off the top.
///
/// The float side is expressed by the column fields (not an enum): a LEFT float
/// (Zork Zero's drop-cap) blits at `img_col == 0` with text pushed right
/// (`text_col == reserve_cols`); a RIGHT float (Shogun's opening picture, ZMSD
/// §15 margin picture) blits at `img_col` near the right edge with text flush
/// left (`text_col == 0`). Either way the wrap width on covered rows is
/// `cols - reserve_cols`.
#[derive(Debug, Clone)]
pub struct RasterFloat {
    pub row: i32,
    pub rows: u16,
    /// Columns removed from the text width on the rows this float covers.
    pub reserve_cols: u16,
    /// Column where each covered row's text begins.
    pub text_col: u16,
    /// Column where the picture is blitted.
    pub img_col: u16,
    pub img: std::sync::Arc<RgbaImage>,
}

/// The story (primary) window's rasterizable content: visible wrapped lines
/// (oldest-first), the live input line, and the caret column. `awaiting` gates
/// the input line + block cursor (drawn while the view sits at the bottom of the
/// transcript; deliberately independent of which pane holds keyboard focus).
/// `floats` carries the window-0 inline pictures anchored within the visible
/// rows — blitted at the left margin with text indented beside them.
#[derive(Debug, Default, Clone)]
pub struct MainText {
    pub lines: Vec<String>,
    /// Per-character ZMSD §8.7.1 style bytes for `lines`, parallel to it
    /// (SQ-0540). An EMPTY inner vec — the common case, and what a short vec's
    /// missing tail means — is an all-roman row, so a transcript with no
    /// emphasis costs nothing. Only bold (2) and italic (4) are carried; the
    /// raster prose path has no reverse-video block to draw into.
    pub styles: Vec<Vec<u8>>,
    pub input: String,
    pub cursor_col: u16,
    pub awaiting: bool,
    pub floats: Vec<RasterFloat>,
}

/// The native screen extent (max window bottom-right) in game pixels; min 1×1.
///
/// A window whose `w_px`/`h_px` is an unresolved size sentinel — a small
/// negative value stored as a large `u16` (Shogun leaks `0xFFFE` ≈ −2 into a
/// window's `x_size`, ballooning the extent to 65534×200 and the raster canvas
/// allocation with it, SQ-0481) — must not drive the extent. Any dimension with
/// the high bit set (`>= 0x8000`, i.e. negative as `i16`) is far past any real
/// v6 screen (~640 px) so it's treated as unresolved and skipped for that axis;
/// clamping here (presentation) keeps zvm storing window props verbatim for the
/// game to read back (ZMSD §8.8.3.2).
pub fn native_extent(items: &[PositionedWindow], tf: &crate::native_font::TextFace) -> (u16, u16) {
    let cell = tf.cell();
    let font_h = u32::from(cell.h());
    let mut w = 1u16;
    let mut h = 1u16;
    let resolved = |px: u16| (px as i16) >= 0; // high bit clear ⇒ a real size
    for it in items {
        if resolved(it.w_px) {
            w = w.max(it.x_px.saturating_add(it.w_px));
        }
        if resolved(it.h_px) {
            h = h.max(it.y_px.saturating_add(it.h_px));
        }
        // A window sized to zero can still hold painted text runs at their
        // screen-absolute pixel positions (Journey's height-0 command menu,
        // SQ-0492): its w_px/h_px don't reach the runs, so grow the extent to
        // cover them directly, or the chrome canvas clips the menu off the
        // bottom. Runs carry 1-based top-left coords; a glyph spans FONT×FONT.
        if let WinNode::Grid(g) = &it.node {
            for t in &g.px_texts {
                // **The reach of a run is the PEN's** (SQ-1066). This was
                // `chars * cell.w` — `V6Cell::run_px` written longhand — while
                // `build_chrome_canvas` draws the very same run by stepping
                // `advance_styled`, so the unit screen was sized by a measure
                // nothing draws with. SQ-1054 was this one stage later: a
                // declared-cell rect a pen-drawn run had to fit INSIDE; here it is
                // one a pen-drawn run has to GROW.
                //
                // It is not a clip and loses no ink — it moves the whole frame.
                // `native_extent` is the unit screen every downstream stage divides
                // by, so an answer 18 px wide of the machine's own screen scales and
                // letterboxes everything against a screen the machine never
                // declared, which is the SQ-0901 shape. Measured on the Macintosh
                // Zork Zero hint frame: 658x400 where the machine says 640x400.
                //
                // Nor is the direction fixed — Geneva 12 advances 3-11 px against a
                // declared 7, so `chars * 7` can be narrower OR wider than the pen.
                // The defect is not a size, it is that the extent was not the extent
                // of what is drawn. Every fixed pen answers the declared cell for
                // every style, so no other press moves by a pixel.
                let right = (t.x.max(1) as u32 - 1) + tf.run_px_styled(&t.text, t.style);
                let bottom = (t.y.max(1) as u32 - 1) + font_h;
                w = w.max(right.min(u16::MAX as u32) as u16);
                h = h.max(bottom.min(u16::MAX as u32) as u16);
            }
        }
    }
    (w, h)
}

/// The v6 window list split into the story window, the story window's own
/// picture (the room illustration — story content, NOT chrome), and everything
/// else (chrome), in input order.
pub struct V6Layout<'a> {
    pub story: Option<&'a PositionedWindow>,
    /// The primary window's Graphics entry (window 0's picture canvas — a room
    /// illustration). It belongs to the story, so it is rendered inside the story
    /// region rather than composited as absolute chrome over the frame.
    pub story_gfx: Option<&'a PositionedWindow>,
    pub chrome: Vec<&'a PositionedWindow>,
}

/// How much of a candidate's own rect the frame art may cover before it stops
/// being a "clear middle" (SQ-0934). Measured 0.0% on every hint frame in the
/// corpus — Zork Zero and Shogun both leave the middle perfectly transparent —
/// so this is slack for a future frame with a stray pixel, not a threshold any
/// known screen sits near.
const RING_MIDDLE_MAX_INK: f32 = 0.005;

/// How much of the art OUTSIDE a candidate must be opaque for the candidate to be
/// framed rather than merely sitting on a blank screen. The hint frames measure
/// 30.5% over the whole canvas (78% top band, 70% flanks); gameplay frames run
/// 12–30%. A screen with nothing around the text is not a ring and must keep
/// today's behaviour.
const RING_MIDDLE_MIN_FRAME_INK: f32 = 0.05;

/// The chrome `Grid` filling the clear middle of a ring of artwork (SQ-0934).
///
/// # The screen this exists for
///
/// Zork Zero's and Shogun's **InvisiClues screen is one screen**, shared by both
/// games and identical on every medium: a full-screen graphics window whose
/// opaque pixels form a ring — 78% of the top band, 70% of each flank, and a
/// middle that is **0.0%** opaque — with the topic list printed into that clear
/// middle and the title and key legend printed onto the banner art above it.
///
/// The games publish it by WITHDRAWING the primary buffer and printing the list
/// into a `Grid` of the same rect. That made [`classify_windows`] answer
/// `story: None`, and a no-story frame routes to the runs-only arm in
/// `render::screen`, whose contract is that it draws the runs and discards every
/// pixel on the screen. So the frame the game drew around the menu was thrown
/// away and the menu came out as bare text on the player's theme.
///
/// Recognising the middle restores the ring, and nothing downstream needs to
/// change: the bands come from content (SQ-0894), the banner text stays
/// RASTERISED because its strip is art (`ChromeStrip::Art` contributes no
/// `glyph_rows`, SQ-0903 — a glyph cannot sit over a kitty placement), and the
/// topic list is packed onto the viewport as crisp glyphs (SQ-0892), which is
/// what the Macintosh release — the one that keeps its buffer — already does.
///
/// **The classification has to be per-frame, not per-window.** Shogun's top band
/// is 14% opaque during gameplay and 78% on the hint screen: the same window slot
/// is a text strip in one frame and artwork in the next.
///
/// # What it deliberately does not match
///
/// The candidate must be framed by a chrome GRAPHICS window. scopa publishes no
/// buffer either — its screen is three `Grid`s and it draws its card table with
/// `erase_window` fills (SQ-0711) — but those fills are a paint ground, not a
/// graphics window, so it finds no frame here and keeps the arm SQ-0711 chose.
///
/// A candidate spanning the whole screen is rejected too: a full-screen grid is
/// not in a middle, it IS the screen.
fn ring_middle_grid<'a>(chrome: &[&'a PositionedWindow], native: (u16, u16)) -> Option<&'a PositionedWindow> {
    let resolved = |px: u16| (px as i16) > 0;
    // The frame: every chrome graphics window that has ink in it. Asked as one
    // surface, because a game is free to draw its border in more than one.
    let art: Vec<&PositionedWindow> = chrome
        .iter()
        .copied()
        .filter(|pw| matches!(&pw.node, WinNode::Graphics(g) if g.win != 0))
        .collect();
    if art.is_empty() {
        return None;
    }
    let opaque_at = |x: u32, y: u32| -> bool {
        art.iter().any(|pw| {
            let WinNode::Graphics(g) = &pw.node else { return false };
            let (ox, oy) = (u32::from(pw.x_px), u32::from(pw.y_px));
            x >= ox
                && y >= oy
                && x - ox < g.canvas.width()
                && y - oy < g.canvas.height()
                && g.canvas.get_pixel(x - ox, y - oy)[3] >= 128
        })
    };
    let (nw, nh) = (u32::from(native.0), u32::from(native.1));
    let mut best: Option<(&PositionedWindow, u32)> = None;
    for pw in chrome.iter().copied() {
        let WinNode::Grid(g) = &pw.node else { continue };
        // It must be carrying text — an empty grid is not a menu.
        if !g.px_texts.iter().any(|t| !t.text.trim().is_empty()) {
            continue;
        }
        if !resolved(pw.w_px) || !resolved(pw.h_px) {
            continue;
        }
        let (x0, y0) = (u32::from(pw.x_px), u32::from(pw.y_px));
        let (x1, y1) = ((x0 + u32::from(pw.w_px)).min(nw), (y0 + u32::from(pw.h_px)).min(nh));
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        // Not the whole screen.
        if x0 == 0 && y0 == 0 && x1 >= nw && y1 >= nh {
            continue;
        }
        let inside_area = (x1 - x0) * (y1 - y0);
        let mut inside_ink = 0u32;
        for y in y0..y1 {
            for x in x0..x1 {
                if opaque_at(x, y) {
                    inside_ink += 1;
                }
            }
        }
        if inside_ink as f32 > inside_area as f32 * RING_MIDDLE_MAX_INK {
            continue;
        }
        // …and there has to be a frame AROUND it.
        let mut outside_ink = 0u32;
        let mut outside_area = 0u32;
        for y in 0..nh {
            for x in 0..nw {
                if x >= x0 && x < x1 && y >= y0 && y < y1 {
                    continue;
                }
                outside_area += 1;
                if opaque_at(x, y) {
                    outside_ink += 1;
                }
            }
        }
        if outside_ink as f32 <= outside_area as f32 * RING_MIDDLE_MIN_FRAME_INK {
            continue;
        }
        if best.is_none_or(|(_, a)| inside_area > a) {
            best = Some((pw, inside_area));
        }
    }
    best.map(|(pw, _)| pw)
}

/// Classify `items`: the first primary `Buffer` becomes `story`; window 0's own
/// `Graphics` entry becomes `story_gfx` (story content); every other entry (in
/// input order) goes into `chrome`.
///
/// With no primary `Buffer`, `story` falls back to the `Grid` filling the clear
/// middle of a ring of artwork, if there is one — see [`ring_middle_grid`] for
/// the screen that needs it and why. **That grid stays in `chrome` as well**,
/// which is not an oversight: `story` is wanted for its RECT (the viewport the
/// ring lays out around), while the grid's runs must still reach `chrome_runs`
/// or the menu they carry would not be drawn at all. The Macintosh release
/// already renders in exactly that arrangement — an empty story buffer whose
/// runs live on a chrome grid — so this is the shape the ring path is known to
/// handle, not a new one.
///
/// Every reader that wants a `Buffer` specifically already pattern-matches for
/// one and declines otherwise ([`story_bg_rgba`], [`story_fg_rgba`],
/// [`story_pair_packed`], [`draw_story_canvas_runs`]), so a `Grid` in this slot
/// contributes its rect and nothing else.
pub fn classify_windows(items: &[PositionedWindow], cell: V6Cell) -> V6Layout<'_> {
    let mut story = None;
    let mut story_gfx = None;
    let mut chrome = Vec::new();
    for pw in items {
        if story.is_none() && matches!(&pw.node, WinNode::Buffer(b) if b.primary) {
            story = Some(pw);
        } else if story_gfx.is_none() && matches!(&pw.node, WinNode::Graphics(g) if g.win == 0) {
            story_gfx = Some(pw);
        } else {
            chrome.push(pw);
        }
    }
    if story.is_none() {
        story = ring_middle_grid(&chrome, native_extent(items, &crate::native_font::TextFace::cell_only(cell)));
    }
    V6Layout { story, story_gfx, chrome }
}

/// One chrome graphics window the v6 CELL path places beside the story, and the
/// pane columns it places it in.
pub struct SideColumn<'a> {
    /// The window itself — the renderer draws `win.node`, the dialog walk only
    /// needs to know which columns it covers.
    pub win: &'a PositionedWindow,
    /// Left of the story box (`true`) or right of it (`false`) — which edge the
    /// story has to be inset from.
    pub left: bool,
    /// First pane column, absolute (already offset from the pane's `x`).
    pub x: u16,
    /// Width in pane columns. Never zero, never more than half the pane.
    pub w: u16,
}

/// The chrome graphics windows the v6 CELL path actually places, and where.
///
/// **THE single statement of that rule** (SQ-1092). It had two, and they measured
/// the half-pane guard on different bases: the renderer in PANE-PROPORTIONAL
/// columns (`area.width * px / native_w`), `screen::collect_graphics_rects` in the
/// game's own NATIVE cells (`PositionedWindow::w`, 80 across a 640-px screen). At
/// an 82-column pane against an ~80-cell screen those agree to the column, which
/// is exactly why nothing caught it; at 160 columns the drawn column is twice as
/// wide as the stamp and they can reach opposite verdicts on the same window. A
/// modal is then centred over pixels the terminal draws on top of it — SQ-0203, in
/// the one place the exclusion was still meant to apply.
///
/// The rule itself, unchanged from the cell path that owned it: a chrome GRAPHICS
/// window entirely BESIDE the story (Journey's half-screen picture column) is
/// story content, not frame art — this path drops the surrounding chrome, but
/// dropping this lost the illustration the raster and hybrid paths both show.
/// Frame art that spans or overlaps the story (Arthur's header panel, every game's
/// full-screen backdrop) is NOT beside it and stays dropped: that is what drawing
/// no game image means.
///
/// `layout` supplies the story window and the chrome, so window 0's own picture
/// (`story_gfx`) is filed by [`classify_windows`] and cannot be classified twice;
/// `native_w` is [`native_extent`]'s width for the same items. Empty when the frame
/// has no story window at all — that frame goes to the painted-screen branch, which
/// places no image.
pub fn cell_path_side_columns<'a>(
    layout: &V6Layout<'a>,
    area: ratatui::layout::Rect,
    native_w: u16,
) -> Vec<SideColumn<'a>> {
    let Some(story) = layout.story else { return Vec::new() };
    let col_of = |px: u16| (area.width as u32 * px as u32 / native_w.max(1) as u32) as u16;
    let story_l = story.x_px;
    let story_r = story.x_px.saturating_add(story.w_px);
    layout
        .chrome
        .iter()
        .filter(|pw| matches!(&pw.node, WinNode::Graphics(_)))
        .filter(|pw| {
            pw.y_px < story.y_px.saturating_add(story.h_px)
                && pw.y_px.saturating_add(pw.h_px) > story.y_px
        })
        .filter_map(|pw| {
            let right_edge = pw.x_px.saturating_add(pw.w_px);
            let left = if right_edge <= story_l {
                true
            } else if pw.x_px >= story_r {
                false
            } else {
                return None;
            };
            let x = col_of(pw.x_px);
            let w = col_of(right_edge).saturating_sub(x);
            // A side column never takes more than half the pane — the story stays
            // the larger half whatever the game declares.
            (w > 0 && w.saturating_mul(2) <= area.width).then_some(SideColumn {
                win: pw,
                left,
                x: area.x.saturating_add(x),
                w,
            })
        })
        .collect()
}

/// The story window's own background colour (set by the game via
/// `set_colour`), resolved to an opaque RGBA for filling the story rect
/// before floats/text. `None` when the game set no colour — the caller then
/// falls back to its resolved default page (SQ-0510); either way the rect ends
/// up opaque, never left for a compositor to colour in.
pub fn story_bg_rgba(story: Option<&PositionedWindow>, colors: &ColorScheme) -> Option<Rgba<u8>> {
    // SQ-0934: a `Grid` reaches this slot when the game withdrew its buffer and
    // printed into the clear middle of a ring instead (see `ring_middle_grid`). It
    // carries `bg`/`fg` exactly as a buffer does, and it IS the story surface for
    // that frame, so its page must fill the same rect — otherwise the middle keeps
    // the host's backdrop and the page stops short of the frame.
    let (bg, _) = story_surface_pair(story?)?;
    // `bg`, when `Some`, always packs a non-Default channel (see
    // `state::pack_zcolour`), so the fallback here is never actually used —
    // it exists only to satisfy `packed_to_rgba`'s signature.
    Some(packed_to_rgba(bg?, Rgba([0, 0, 0, 255]), colors))
}

/// The `(bg, fg)` a story surface declares, whichever kind of window it is.
///
/// One place, because the three readers below must not disagree about what a
/// promoted grid means — and because a node that is neither is not a story
/// surface at all, which is the case that keeps `Graphics` and `Blank` out.
fn story_surface_pair(it: &PositionedWindow) -> Option<(Option<u32>, Option<u32>)> {
    match &it.node {
        WinNode::Buffer(b) => Some((b.bg, b.fg)),
        WinNode::Grid(g) => Some((g.bg, g.fg)),
        _ => None,
    }
}

/// The story window's own FOREGROUND colour (set by the game via `set_colour`),
/// resolved to an opaque RGBA for the ink the story prose is rasterized in.
/// `None` when the game set no colour — the caller then falls back to its
/// resolved default ink.
///
/// The exact mirror of [`story_bg_rgba`], and for the same reason (SQ-0532
/// wave-5): the pair is the game's, so it has to be honoured as a pair. Zork
/// Zero boots `set_colour(fg=2 black, bg=9 white)` on window 0; taking its white
/// page but keeping the host's own (light) default ink rasterized white-on-white
/// prose that could not be read at all.
pub fn story_fg_rgba(story: Option<&PositionedWindow>, colors: &ColorScheme) -> Option<Rgba<u8>> {
    let (_, fg) = story_surface_pair(story?)?;
    // `fg`, when `Some`, always packs a non-Default channel (see
    // `state::pack_zcolour`), so the fallback here is never actually used —
    // it exists only to satisfy `packed_to_rgba`'s signature.
    Some(packed_to_rgba(fg?, Rgba([255, 255, 255, 255]), colors))
}

/// The story window's explicit `(fg, bg)` pair as PACKED z-colours (`0` when the
/// game set none), for the cell-side callers — the live input line resolves them
/// through `resolve_zcolour`, exactly as the transcript's prose runs do, rather
/// than through the pixel path's [`story_fg_rgba`]/[`story_bg_rgba`]. Same
/// source, same window, one resolution per path. (SQ-0532 wave-6)
pub fn story_pair_packed(story: Option<&PositionedWindow>) -> (u32, u32) {
    match story.and_then(story_surface_pair) {
        Some((bg, fg)) => (fg.unwrap_or(0), bg.unwrap_or(0)),
        None => (0, 0),
    }
}

/// Whether two positioned windows' native pixel boxes intersect at all.
fn boxes_overlap(a: &PositionedWindow, b: &PositionedWindow) -> bool {
    let (ax0, ay0) = (a.x_px as u32, a.y_px as u32);
    let (bx0, by0) = (b.x_px as u32, b.y_px as u32);
    let (ax1, ay1) = (ax0 + a.w_px as u32, ay0 + a.h_px as u32);
    let (bx1, by1) = (bx0 + b.w_px as u32, by0 + b.h_px as u32);
    ax0 < bx1 && bx0 < ax1 && ay0 < by1 && by0 < ay1
}

/// SQ-0704: resolve each chrome window's still-UNPAINTED area to that window's
/// OWN background colour.
///
/// ZMSD §8.8.3.2 gives every Version 6 window its own foreground/background pair
/// (property 11). [`build_chrome_canvas`] resolves everything against a SINGLE
/// host `default_fg`/`default_bg`, and consults a window's own pair only for its
/// text runs (`fill_explicit_bg_rows`, SQ-0519) — it never paints a window's own
/// page, and the graphics pass blits alpha untouched. So a window whose art is
/// mostly transparent reached the terminal as transparency, and whatever the
/// protocol composited it over became the backdrop. Zork Zero's room/compass
/// icons (pictures 9/10/11/13, 45×40, ~95 % alpha-0 line art drawn into its
/// 640×78 banner window) hang below the banner artwork, and there the clear
/// ground rendered as an opaque BLACK box where the DOS original shows the
/// window's white page.
///
/// Only pixels no layer has touched (`alpha == 0`) are painted, so frame art,
/// status bands, glyphs and the icons' own lit strokes are left byte-for-byte
/// alone — the faithful alpha compositing is preserved, and a window the game
/// gave no colour is skipped entirely and keeps today's behaviour (its holes
/// still fall through to the caller's page).
///
/// A window that overlaps the STORY box is skipped too: Zork Zero's window 7
/// carries the same white page across the whole 640×400 screen, and both the
/// hybrid transcript viewport and [`story_clear_native`]'s clear-interior probe
/// need that region to stay transparent. The story window's page is painted by
/// the story paths (`fill_pane_page` in hybrid, the story-rect fill plus
/// [`flatten_onto_page`] in raster), which already honour its own colour.
///
/// Callers gate this on `honor_game_colours`: with the game's colours declined
/// the host page governs everywhere, exactly as before — except for the windows
/// [`fill_painted_window_pages`] carves back out, which are the game's own canvas
/// rather than its colour preference.
/// `text` is the same claim [`build_chrome_canvas`] was built with, and it governs
/// this fill for the same reason (SQ-0948). A page is not artwork: on a row the
/// ring draws with GLYPHS, the strip stamps the window's own background into its
/// cells, so a page painted here is a second rendering of one ribbon — at the
/// window's true native height instead of the strip's whole cells, and reaching
/// whatever native columns the strip's cell boundary rounded away.
///
/// MEASURED on `stories/shogun-r322-s890706.z6` (release 322, IBM PC, six taps
/// into play) at a 117x40 kitty terminal: the status window is `548x32` at native
/// `(46, 0)` and the left flank band's last terminal cell inverts to native
/// `44.5..50.1`, so four columns of that page sat inside the FLANK. The strip drew
/// the ribbon 36 device px tall (two whole cells) and the band drew the same page
/// 46 px tall (32 native rows), and the 10-pixel difference reached the screen as a
/// 6x10 white block hanging below each end of the score bar — SQ-0948, and the same
/// boundary SQ-0902/0903 fixed for the glyph strokes and their flood while leaving
/// the page behind them untouched.
pub fn fill_window_pages(
    canvas: &mut RgbaImage,
    chrome: &[&PositionedWindow],
    story: Option<&PositionedWindow>,
    colors: &ColorScheme,
    text: TextLayer<'_>,
    cell: V6Cell,
) {
    fill_pages_where(canvas, chrome, story, colors, text, |_| true, cell);
}

/// SQ-0716: the half of [`fill_window_pages`] that survives `honor_game_colours
/// = off` — a window the game has DRAWN INTO keeps its declared page.
///
/// scopa's felt table is why. Measured from the screen ops rather than the model,
/// it boots `@set_true_colour(fg=true(0x0000), bg=true(0x0200), window=1)` — an
/// explicit green — sizes window 1 to the full 640×400 screen and issues
/// `@erase_window`. That is a FILL, the same drawing operation SQ-0706 declared
/// ungatable when it made the cards survive a declined palette; the cards and the
/// table come out of the identical opcode. It reaches us as a window page only
/// because `drain_erase_fills` classifies a fill spanning the whole screen as a
/// screen clear and drops it, leaving window 1's background as the sole surviving
/// record of the paint. Gating that record on the colour flag therefore deleted
/// half of one drawing: declining game colours left a BLACK table carrying the
/// green stripes and cards the game had drawn onto it — worse than either
/// honouring the game or ignoring it.
///
/// The discriminator is the painted ground, exactly as in SQ-0711: a window with
/// the game's own pixels inside it is a canvas, and its page is the ground those
/// pixels were drawn on. A window with none is presentation, and the host page
/// governs it as before. Zork Zero, Arthur, Shogun, Journey and advent paint no
/// ground at all, so none of them can reach this path.
///
/// The STORY window is deliberately NOT included (and `fill_pages_where` skips
/// anything overlapping it anyway): its page and ink are the surface prose is read
/// on, they have to be honoured or declined as a PAIR (SQ-0532 wave-5), and that
/// pair is precisely what `honor_game_colours` exists to govern.
pub fn fill_painted_window_pages(
    canvas: &mut RgbaImage,
    chrome: &[&PositionedWindow],
    story: Option<&PositionedWindow>,
    colors: &ColorScheme,
    paint: Option<&RgbaImage>,
    cell: V6Cell,
) {
    let Some(paint) = paint else { return };
    // `TextLayer::All`: this path is the game's own CANVAS, not a presentation
    // colour — the window it fills has the game's pixels in it, and no ring strip
    // stamps that ground into cells. Nothing here has a second rendering to agree
    // with, so there is no row to skip.
    fill_pages_where(canvas, chrome, story, colors, TextLayer::All, |it| window_has_paint(it, paint), cell);
}

/// Whether the game's painted ground has any pixel inside `it`'s native box.
fn window_has_paint(it: &PositionedWindow, paint: &RgbaImage) -> bool {
    let (x0, y0) = (it.x_px as u32, it.y_px as u32);
    let x1 = (x0 + it.w_px as u32).min(paint.width());
    let y1 = (y0 + it.h_px as u32).min(paint.height());
    (y0..y1).any(|y| (x0..x1).any(|x| paint.get_pixel(x, y)[3] > 0))
}

/// The shared body of [`fill_window_pages`] and [`fill_painted_window_pages`]:
/// paint each `keep`-approved chrome window's own page into its untouched pixels.
fn fill_pages_where(
    canvas: &mut RgbaImage,
    chrome: &[&PositionedWindow],
    story: Option<&PositionedWindow>,
    colors: &ColorScheme,
    text: TextLayer<'_>,
    keep: impl Fn(&PositionedWindow) -> bool,
    cell: V6Cell,
) {
    for it in chrome {
        let bg = match &it.node {
            WinNode::Grid(g) => g.bg,
            WinNode::Buffer(b) => b.bg,
            _ => None,
        };
        // Only a colour the game actually NAMED counts (`packed_explicit`):
        // "current"/"default" are inheritance, not a page choice.
        let Some(bg) = bg.filter(|&p| packed_explicit(p)) else { continue };
        // A size sentinel (negative read as i16) is not a real box (SQ-0481).
        if it.w_px == 0 || it.h_px == 0 || (it.w_px as i16) < 0 || (it.h_px as i16) < 0 {
            continue;
        }
        if story.is_some_and(|s| boxes_overlap(it, s)) {
            continue;
        }
        if !keep(it) {
            continue;
        }
        // `bg` is explicit here, so the fallback can never be reached.
        let page = packed_to_rgba(bg, Rgba([0, 0, 0, 255]), colors);
        let (x0, y0) = (it.x_px as u32, it.y_px as u32);
        let x1 = (x0 + it.w_px as u32).min(canvas.width());
        let y1 = (y0 + it.h_px as u32).min(canvas.height());
        for y in y0..y1 {
            if text.skips_line(y, cell) {
                continue;
            }
            for x in x0..x1 {
                if canvas.get_pixel(x, y)[3] == 0 {
                    canvas.put_pixel(x, y, page);
                }
            }
        }
    }
}

/// Fill the STORY window's still-unpainted pixels with its own declared page
/// (SQ-0704, hybrid half).
///
/// [`fill_window_pages`] deliberately skips any window overlapping the story box,
/// because in RASTER mode the story page is painted separately by
/// `build_v6_raster_canvas` and the whole canvas is flattened opaque before it
/// ships. HYBRID has no such flatten: it draws the story as terminal text and
/// ships only the ring bands as images — and those bands overlap the story box,
/// both in the one-row sliver under a top banner and along the flanks. Every pixel
/// left transparent there is resolved by the TERMINAL, not by us, so Zork Zero's
/// room icons came out sitting on the terminal background instead of the white page
/// the game declared for the window they live in.
///
/// Only pixels no layer has touched are filled, and only when the story window
/// named a page explicitly — a game that set none keeps today's behaviour.
pub fn fill_story_page_clear(
    canvas: &mut RgbaImage,
    story: Option<&PositionedWindow>,
    colors: &ColorScheme,
) {
    let Some(it) = story else { return };
    let Some(page) = story_bg_rgba(Some(it), colors) else { return };
    if it.w_px == 0 || it.h_px == 0 || (it.w_px as i16) < 0 || (it.h_px as i16) < 0 {
        return;
    }
    let (x0, y0) = (it.x_px as u32, it.y_px as u32);
    let x1 = (x0 + it.w_px as u32).min(canvas.width());
    let y1 = (y0 + it.h_px as u32).min(canvas.height());
    for y in y0..y1 {
        for x in x0..x1 {
            if canvas.get_pixel(x, y)[3] == 0 {
                canvas.put_pixel(x, y, page);
            }
        }
    }
}

/// The native pixel rects the game's own chrome TEXT occupies — one per painted
/// `px_texts` run, one per non-blank cell of a plain character grid (SQ-0728).
///
/// It is deliberately the runs the GAME printed, not every opaque pixel the text
/// pass left: `fill_reverse_row_gaps` also paints, and its screen-wide fill is a
/// host device for closing the bare cells inside a bar (SQ-0504), not something
/// the game drew. Journey draws a one-cell reversed divider on each of nineteen
/// rows, which qualifies every one of them as a "pure reverse row" and floods the
/// gap either side — right across window 0's text panel. That flood must yield to
/// the story page; the labels a game deliberately printed inside window 0's box
/// must not.
pub fn chrome_text_rects(
    chrome: &[&PositionedWindow],
    // The cell, the face and the PEN as one value (SQ-1054) — a rect that spares
    // what the draw claimed has to be measured the way the draw measures.
    tf: &crate::native_font::TextFace,
) -> Vec<(u32, u32, u32, u32)> {
    let cell = tf.cell();
    let font_w = u32::from(cell.w());
    let font_h = u32::from(cell.h());
    let mut rects = Vec::new();
    for it in chrome {
        // A secondary prose window's lines are drawn onto the composite too
        // (SQ-0729), so the story page must spare them exactly as it spares a
        // grid's runs — else fmvpoker's menu bar, printed inside window 0's box,
        // is painted out the moment it is painted in.
        rects.extend(buffer_line_rects(it, tf));
        let WinNode::Grid(g) = &it.node else { continue };
        if !g.px_texts.is_empty() {
            for t in &g.px_texts {
                let x = t.x.max(1) as u32 - 1;
                let y = t.y.max(1) as u32 - 1;
                // **The PEN's span, not the declared one** (SQ-1054). The glyph
                // loop in `build_chrome_canvas` steps `advance_styled`, so that is
                // the width of ink standing on the canvas; sparing
                // `chars * font_w` instead spares the box the GAME reserved, and
                // where the face is wider than the cell the page fill then lands
                // on the tail of the run and slices whichever glyph straddles the
                // boundary. Macintosh Zork Zero's hint menu is the report: its
                // topics stop dead at `x + chars * 7` with a half-drawn letter at
                // the cut — `GREAT HALL AREA` inked to 195 against a pen ending at
                // 203, `FOR YOUR AMUSEMENT` to 433 against 443.
                let w = tf.run_px_styled(&t.text, t.style).max(font_w);
                rects.push((x, y, x + w, y + font_h));
            }
            continue;
        }
        let (ox, oy) = (it.x_px as u32, it.y_px as u32);
        for row in 0..g.rows {
            for col in 0..g.cols {
                let cell = g.cell(row + 1, col + 1);
                if cell.ch == '\0' || (cell.ch == ' ' && cell.bg == 0) {
                    continue;
                }
                let (x, y) = (ox + col as u32 * font_w, oy + row as u32 * font_h);
                rects.push((x, y, x + font_w, y + font_h));
            }
        }
    }
    rects
}

/// Paint the story window's clear interior with its `page`, sparing every pixel a
/// chrome text run claimed (SQ-0728).
///
/// The page has to be opaque — raster ships one image, and a transparent pixel is
/// resolved by whoever composites it rather than by us (SQ-0510) — but it is also
/// the OLDEST thing in the box: the game filled window 0, then other windows
/// printed on top of it. Shogun's title is the measured case. Its menu window sits
/// inside window 0's 548x64 box and prints "START the game" there while window 0
/// prints "You may choose to:" beside it; both are on the screen at once on a real
/// interpreter. A flat fill of the box erased the menu.
///
/// `paint` is the game's own PAINTED GROUND — the rectangles its `erase_window`
/// calls filled (SQ-0706) — and is spared for exactly the same reason as the chrome
/// text: those pixels are NEWER than window 0's page, not older. fmvpoker is the
/// report (SQ-0729). It parks window 1 over the "Double Fanucci" banner its frame
/// art carries — the art is Zork Zero's, shipped renamed as FMVPOKER.EG1 — and
/// erases it to the blue it declared for that window, which is how the game hides a
/// title that is not its own. The erase reached us correctly, but the story page
/// then flooded window 0's whole box on top of it, so the banner rendered as a white
/// gash across the top of the frame rather than a plain blue tab.
pub fn fill_story_page_under_chrome_text(
    canvas: &mut RgbaImage,
    (bx, by, bw, bh): (u32, u32, u32, u32),
    page: Rgba<u8>,
    chrome: &[&PositionedWindow],
    paint: Option<&RgbaImage>,
    tf: &crate::native_font::TextFace,
) {
    let text: Vec<(u32, u32, u32, u32)> = chrome_text_rects(chrome, tf)
        .into_iter()
        .filter(|&(x0, y0, x1, y1)| x0 < bx + bw && bx < x1 && y0 < by + bh && by < y1)
        .collect();
    let painted = |x: u32, y: u32| -> bool {
        paint.is_some_and(|p| x < p.width() && y < p.height() && p.get_pixel(x, y)[3] != 0)
    };
    let (cw, ch) = (canvas.width(), canvas.height());
    for y in by..(by + bh).min(ch) {
        let row: Vec<(u32, u32)> =
            text.iter().filter(|&&(_, y0, _, y1)| y >= y0 && y < y1).map(|&(x0, _, x1, _)| (x0, x1)).collect();
        for x in bx..(bx + bw).min(cw) {
            if row.iter().any(|&(x0, x1)| x >= x0 && x < x1) || painted(x, y) {
                continue;
            }
            canvas.put_pixel(x, y, page);
        }
    }
}

/// Whether any pixel in the `w × h` box at `(px, py)` of `canvas` is opaque
/// (alpha ≥ 128). Used to tell a reverse-video run sitting ON frame art from one
/// over a clear background, so the art is preserved but a bare selection bar still
/// gets its highlight block (SQ-0487). Out-of-bounds pixels count as transparent.
pub(crate) fn region_has_opaque(canvas: &RgbaImage, px: u32, py: u32, w: u32, h: u32) -> bool {
    let (cw, ch) = (canvas.width(), canvas.height());
    for y in py..(py + h).min(ch) {
        for x in px..(px + w).min(cw) {
            if canvas.get_pixel(x, y)[3] >= 128 {
                return true;
            }
        }
    }
    false
}

pub(crate) fn fill_cell(canvas: &mut RgbaImage, px: u32, py: u32, cw: u32, ch: u32, color: Rgba<u8>) {
    let (w, h) = (canvas.width(), canvas.height());
    for y in py..(py + ch).min(h) {
        for x in px..(px + cw).min(w) {
            canvas.put_pixel(x, y, color);
        }
    }
}

/// Flatten a FULLY COMPOSED raster canvas onto an opaque `page` (SQ-0510):
/// every pixel the composite left completely transparent (`alpha == 0`) becomes
/// `page`; every pixel any layer touched (`alpha > 0` — frame art, status bands,
/// the story page fill, glyphs, inline drop-caps) is left exactly as it was.
///
/// Why: raster mode ships the whole canvas as ONE image, and a transparent pixel
/// is then resolved by whoever composites it — not by us. The kitty encoder
/// (`ratatui_image`'s `transmit_virtual`, `f=32`) keeps the alpha channel and
/// lets the terminal decide; the halfblocks encoder flattens with `to_rgb8()`
/// and maps an untouched cell's `Color::Reset` to **white**. So "transparent"
/// renders differently per protocol and per terminal, and is never safe in
/// raster mode. Painting the leftovers ourselves makes the composite
/// self-contained and identical everywhere.
///
/// Only ever called on the raster path's finished canvas. The HYBRID path must
/// NOT use this — there transparency is load-bearing (the chrome ring's clear
/// middle is what lets the terminal transcript show through).
pub(crate) fn flatten_onto_page(canvas: &mut RgbaImage, page: Rgba<u8>) {
    for px in canvas.pixels_mut() {
        if px[3] == 0 {
            *px = page;
        }
    }
}

/// Build the CHROME image: one native-resolution RGBA canvas containing only
/// the frame graphics and status text (everything `classify_windows` put in
/// `chrome`). The story region and any gaps stay fully transparent — a later
/// task scales this canvas to the pane and layers it over the story text.
///
/// Two passes, in list order, frame graphics behind status text: Graphics
/// entries are blitted first (later entries draw over earlier ones only where
/// opaque, giving correct z-order for overlapping frame art like Zork Zero's
/// compass); Grid entries are rasterized second, one glyph per `FONT × FONT`
/// native-pixel cell, drawing every row regardless of the window's pixel
/// height (a v6 status grid can legitimately exceed its pixel box).
///
/// A `px_texts` run's `style` bit 1 (reverse) swaps its resolved fg/bg: the
/// glyph ink is drawn in the run's (window) background colour and a solid
/// block in the run's foreground colour is painted behind it — reverse always
/// paints an opaque block (there is no "transparent ink"), so a run whose
/// colours are unset falls back to `default_bg`/`default_fg` respectively
/// rather than leaving the swapped-in channel transparent.
/// Blit every chrome Graphics window onto `canvas`, in list order (later entries
/// draw over earlier ones only where opaque). The window canvas is authored in
/// native game pixels (pictures at their native size/coords), so blit it 1:1 at
/// the window origin — never scaled — clipped to the window's pixel box (ZMSD §8:
/// plotting is always clipped to the window; a canvas can be larger than the
/// current box when the window has since shrunk). Shared by [`build_chrome_canvas`]
/// (pass 1) and [`build_graphics_canvas`].
fn blit_chrome_graphics(canvas: &mut RgbaImage, chrome: &[&PositionedWindow]) {
    for it in chrome {
        if let WinNode::Graphics(gwn) = &it.node {
            let src = &gwn.canvas;
            blit_clipped(canvas, src, it.x_px as u32, it.y_px as u32, it.w_px.max(1) as u32, it.h_px.max(1) as u32);
        }
    }
}

/// Composite the v6 PAINTED GROUND onto `canvas` — the filled rectangles an
/// `erase_window` left behind (SQ-0706), at their absolute native positions.
///
/// It is GROUND: it goes UNDER everything already drawn, and is itself drawn
/// before the window pages claim the rest.
///
/// A painted fill is the oldest thing on the screen — the game filled a rectangle,
/// then printed its label on top. Compositing the surface OVER the chrome canvas
/// erased exactly those labels: scopa's menu came out as white buttons with no
/// text, because its button fills landed on top of the glyphs that had already
/// been rasterized. So only pixels no layer has touched take paint, and the order
/// is: chrome art and glyphs, then this ground beneath them, then the window pages
/// filling whatever neither claimed.
/// `text` is the claim [`build_chrome_canvas`] was built with, and it governs this
/// blit for the same reason (SQ-0948). A ground the game filled UNDER a status
/// ribbon is not artwork either: the strip stamps that ribbon into whole cells, so a
/// band carrying the fill draws it a second time at the window's own native height.
///
/// MEASURED on `stories/shogun-r322-s890706.z6` two turns into play (`cr`, then
/// `look`) at a 117x40 kitty terminal. The game erases its `548x32` status window at
/// native `(46, 0)`, which reaches the app as a painted rectangle; the ring's left
/// flank band ends at native 50, so four columns of that fill sat inside its image
/// and hung ten device pixels below the two-cell ribbon. It showed only on the LEFT
/// because the paint surface is sized to the story window (548x368) while the fill is
/// recorded in SCREEN coordinates, so the same rectangle is clipped at native 548 and
/// never reaches the right flank at 590 — one white block, not two, which is why the
/// page half of this fix appeared to cure one side and not the other.
pub fn blit_paint_ground(canvas: &mut RgbaImage, paint: Option<&RgbaImage>, text: TextLayer<'_>, cell: V6Cell) {
    let Some(src) = paint else { return };
    let (w, h) = (src.width().min(canvas.width()), src.height().min(canvas.height()));
    for y in 0..h {
        if text.skips_line(y, cell) {
            continue;
        }
        for x in 0..w {
            let p = *src.get_pixel(x, y);
            if p[3] > 0 && canvas.get_pixel(x, y)[3] == 0 {
                canvas.put_pixel(x, y, p);
            }
        }
    }
}

/// Blit the STORY window's own absolutely-placed artwork ([`V6Layout::story_gfx`])
/// onto `canvas` at its native origin (SQ-0695).
///
/// `classify_windows` has always set this entry aside — a `WinNode::Graphics` whose
/// `win` is 0 is story content, not chrome — but nothing ever drew it, so it was
/// classified and dropped. Arthur's intro is what needs it: each illustrated screen
/// centres a 584×392 plate in window 0, so the plate is a BACKDROP occupying the
/// story window rather than part of the frame ring.
///
/// Callers blit it after the story page fill and before the story text, which is
/// the painter's order the game itself used: page, then plate, then prose — see
/// [`story_prose_box`] for whether any prose belongs on this frame at all.
pub fn blit_story_gfx(canvas: &mut RgbaImage, story_gfx: Option<&PositionedWindow>) {
    let Some(it) = story_gfx else { return };
    let WinNode::Graphics(gwn) = &it.node else { return };
    blit_clipped(canvas, &gwn.canvas, it.x_px as u32, it.y_px as u32, it.w_px.max(1) as u32, it.h_px.max(1) as u32);
}

/// A prose column narrower than this (cells) is not a text box — it is a sliver.
/// Mirrors the identical floor `build_main_text` applies before wrapping prose
/// beside an inline float, and the SQ-0578 lesson that a one-column story box
/// re-wraps the whole transcript a character per line.
const MIN_PROSE_COLS: u32 = 8;

/// The largest axis-aligned rectangle inside `clear` (native game pixels) that the
/// `story_gfx` plate painted no pixel of. `None` when the plate leaves nothing.
/// With no plate, or an unpainted one, the whole of `clear` is free.
///
/// Standard largest-rectangle-under-a-histogram sweep over the plate's alpha mask:
/// row by row, each column carries the run of consecutive free pixels above it, and
/// the monotone stack reads off every maximal rectangle ending at that row.
fn plate_free_box(
    clear: (u32, u32, u32, u32),
    story_gfx: Option<&PositionedWindow>,
) -> Option<(u32, u32, u32, u32)> {
    let (cx, cy, cw, chh) = clear;
    if cw == 0 || chh == 0 {
        return None;
    }
    let mut blocked = vec![false; (cw * chh) as usize];
    if let Some(it) = story_gfx {
        if let WinNode::Graphics(gwn) = &it.node {
            let (ox, oy) = (it.x_px as u32, it.y_px as u32);
            for (x, y, px) in gwn.canvas.enumerate_pixels() {
                if px.0[3] == 0 {
                    continue;
                }
                let (sx, sy) = (ox + x, oy + y);
                if sx < cx || sy < cy || sx >= cx + cw || sy >= cy + chh {
                    continue;
                }
                blocked[((sy - cy) * cw + (sx - cx)) as usize] = true;
            }
        }
    }
    let mut heights = vec![0u32; cw as usize];
    let mut best: Option<(u32, u32, u32, u32)> = None;
    let mut stack: Vec<(u32, u32)> = Vec::new(); // (start column, height)
    for r in 0..chh {
        for c in 0..cw {
            heights[c as usize] = if blocked[(r * cw + c) as usize] { 0 } else { heights[c as usize] + 1 };
        }
        stack.clear();
        for c in 0..=cw {
            let h = if c == cw { 0 } else { heights[c as usize] };
            let mut start = c;
            while let Some(&(s, sh)) = stack.last() {
                if sh <= h {
                    break;
                }
                stack.pop();
                let area = (c - s) as u64 * sh as u64;
                if best.is_none_or(|(_, _, bw, bh)| (bw as u64) * (bh as u64) < area) {
                    best = Some((cx + s, cy + r + 1 - sh, c - s, sh));
                }
                start = s;
            }
            stack.push((start, h));
        }
    }
    best.filter(|&(_, _, w, h)| w > 0 && h > 0)
}

/// Where the story window's prose goes once its absolutely-placed plate has the
/// floor — `None` when the plate owns the screen and no prose belongs on the
/// frame at all (SQ-0707).
///
/// An absolutely-placed window-0 picture is a BACKDROP the game draws INSTEAD of
/// prose, not underneath it. Arthur's intro is the measured case: each screen
/// `@erase_window(-1)`s, draws its plate, hides the cursor with `@set_cursor(-1)`
/// and waits on a `read_char` — the narration is a separate, picture-less screen
/// that the game erases before printing. The whole graveyard→Merlin turn is 31
/// instructions and prints not one character. So rasterizing the app's scrollback
/// onto the plate (which is what SQ-0695 shipped, on the mistaken premise that the
/// game "narrates over it") painted the previous screen's prose across the art.
///
/// The rule is the SQ-0578 one — "no room for text → the picture owns the screen"
/// — applied to a plate that blocks the MIDDLE rather than one that outgrew the
/// window. `story_clear_native` cannot see this: it insets from the EDGES, and a
/// centred plate touches none of them. So the free area is measured directly, as
/// the largest rectangle of `clear` the plate painted no pixel of
/// ([`plate_free_box`]); one too narrow to wrap into ([`MIN_PROSE_COLS`]) or too
/// short for one line means there is no prose box. A plate that leaves a genuine
/// column — a corner logo, a margin illustration — still gets prose beside it.
///
/// The free area is measured against what the plate PAINTED, never its bounding
/// box (SQ-0729). fmvpoker draws its poker table as a 640x400 frame with a hollow
/// middle: the ring's bounding box is the whole screen, so the bbox rule read the
/// game's own backdrop as a plate that owns the screen and the title dropped every
/// line of text it prints inside that frame. Only 17% of the picture is opaque, and
/// the hole in it is exactly where the game puts its prose.
pub fn story_prose_box(
    clear: (u32, u32, u32, u32),
    story_gfx: Option<&PositionedWindow>,
    cell: V6Cell,
) -> Option<(u32, u32, u32, u32)> {
    let font_w = u32::from(cell.w());
    let font_h = u32::from(cell.h());
    plate_free_box(clear, story_gfx).filter(|&(_, _, w, h)| w >= MIN_PROSE_COLS * font_w && h >= font_h)
}

/// Build a native-resolution canvas containing ONLY the chrome frame graphics —
/// no status/menu text. Used by the hybrid band decomposition to tell a band
/// strip that sits over real artwork (keeps the pixel ring) from a pure-text
/// strip (paints as terminal cells), via [`region_has_opaque`] — the full chrome
/// canvas can't answer that because rasterized text is itself opaque (SQ-0500).
pub fn build_graphics_canvas(chrome: &[&PositionedWindow], native: (u16, u16)) -> RgbaImage {
    let mut canvas = RgbaImage::new(native.0 as u32, native.1 as u32);
    blit_chrome_graphics(&mut canvas, chrome);
    canvas
}

/// SQ-0499: fill the unpainted interior cells of a PURE reverse-video row (one
/// whose every painted run is reversed) so a status/menu bar the game drew as
/// separate runs with bare gaps between them reads as one solid block. Games paint
/// a reversed bar as its text runs plus, sometimes, reversed spacer spaces — but
/// leave odd cells unpainted (Arthur's status skips one cell before "St Anne's
/// Day"; Journey's menu header leaves a wide gap between its two labels), and the
/// per-run block painting can't fill a cell no run covers. Only PURE reverse rows
/// qualify: a row carrying any NON-reversed run is a mixed layout (Journey's menu
/// BODY — reversed column dividers among normal verb text) and its gaps are real
/// background, left alone. Inherited reverse over opaque frame art still paints no
/// block (Zork0's ribbon labels sit ON the banner), matching the per-run over-art
/// rule so `region_has_opaque` gates each filled cell.
fn fill_reverse_row_gaps(
    canvas: &mut RgbaImage,
    art: &RgbaImage,
    texts: &[&PxText],
    default_fg: Rgba<u8>,
    colors: &ColorScheme,
    tf: &crate::native_font::TextFace,
) {
    let font_w = u32::from(tf.cell().w());
    let font_h = u32::from(tf.cell().h());
    use std::collections::BTreeMap;
    let full_w = canvas.width();
    let mut rows: BTreeMap<u32, Vec<&PxText>> = BTreeMap::new();
    for t in texts {
        rows.entry(t.y.max(1) as u32 - 1).or_default().push(t);
    }
    for (py, runs) in rows {
        // Pure reverse-video row only: every run reversed (and at least one run).
        if runs.is_empty() || runs.iter().any(|t| t.style & 1 == 0) {
            continue;
        }
        // A pure reverse-video row is a bar the game draws edge to edge, so the
        // fill spans the ENTIRE screen width (SQ-0504): the runs the game painted,
        // plus every bare cell around AND between them. A row that named real
        // colours fills unconditionally; an inherited row defers to the over-art
        // rule per gap (so Zork0's ribbon labels on the banner never gain a bar).
        let mut explicit_block: Option<Rgba<u8>> = None;
        let mut spans: Vec<(u32, u32)> = runs
            .iter()
            .map(|t| {
                if explicit_block.is_none() && (packed_explicit(t.fg) || packed_explicit(t.bg)) {
                    explicit_block = Some(packed_to_rgba(t.fg, default_fg, colors));
                }
                let s = t.x.max(1) as u32 - 1;
                // The PEN's span (SQ-1009), so the bare stretches this fills are
                // the ones the glyph loop really leaves bare. Identical to
                // `chars * font_w` for every face that is not proportional; with one
                // it is the difference between a bar and a bar full of holes.
                (s, s + tf.run_px_styled(&t.text, t.style).max(font_w))
            })
            .collect();
        spans.sort_unstable();
        // The bare stretches: from x=0 to the first run, between the runs, and from
        // the last run to the screen edge. Filled at EXACT pixel extent (not cell-
        // quantized): a run's start is `x - 1`, rarely 8-aligned, so a quantized
        // fill cell would bleed a pixel into the next run — harmless to the over-art
        // test (SQ-0487), which reads the ART layer, but still the game's geometry.
        let mut gaps: Vec<(u32, u32)> = Vec::new();
        let mut cursor = 0u32;
        for &(s, e) in &spans {
            if s > cursor {
                gaps.push((cursor, s));
            }
            cursor = cursor.max(e);
        }
        if cursor < full_w {
            gaps.push((cursor, full_w));
        }
        // **A pure reverse-video row is only a BAND if there is something to make it
        // one** (SQ-1026). Two frames go through here and the routine cannot tell them
        // apart by their runs, because their runs are nearly identical:
        //
        //   * Journey's IbmPc menu paints ONE reversed space, at x=233, on every row.
        //     Its frame's side borders are NOT runs at all — they are these gaps,
        //     reaching the screen edges while the over-art test below suppresses the
        //     middle because the picture is there. `journey_amiga_flank_border_is_a_
        //     stroke_not_a_filled_block` pins that border, and it is the behaviour to
        //     preserve.
        //   * Arthur's F3 inventory paints TWO reversed spaces, at x=213 and x=413, on
        //     every row — its column rules. There is no picture on that page, so no gap
        //     is suppressed and the same code floods it white, seven rows of it, against
        //     a capture (`machine-screenshots/amiga-arthur-inventory.png`) showing a
        //     bare page with two thin rules down it.
        //
        // So the runs do not separate them and the ARTWORK does. A row that carries
        // TEXT is a real band and fills regardless — Arthur's own status row, one window
        // below, is all-reversed AND holds `Churchyard`, and the same capture shows it
        // filled edge to edge. A row with no text fills only where some part of it sits
        // over a picture, which is the case the fill was built for and the only case it
        // can be right about. A textless, pictureless row is furniture on a bare page:
        // whatever the game painted there is already exactly what it wanted.
        let over_art = gaps
            .iter()
            .any(|&(gs, ge)| region_has_opaque(art, gs, py, ge.saturating_sub(gs), font_h));
        if !crate::render::screen::row_is_reverse_bar(runs.iter().copied()) && !over_art {
            continue;
        }
        for (gs, ge) in gaps {
            let block = match explicit_block {
                Some(b) => Some(b),
                None if region_has_opaque(art, gs, py, ge - gs, font_h) => None,
                None => Some(default_fg),
            };
            if let Some(b) = block {
                fill_cell(canvas, gs, py, ge - gs, font_h, b);
            }
        }
    }
}

/// SQ-0519: the window-wide background-flood colour for a chrome grid row, or
/// `None` when the row must not flood. Mirrors SQ-0512's hybrid per-row flood at
/// the raster canvas level: a NON-reverse row that names an explicit background
/// (first-explicit-wins per channel — Shogun's in-game status band prints
/// black-on-white, non-reversed) floods its whole window width with that bg so the
/// band reads as one solid bar in the pixel composite, not just behind the glyph
/// runs (the gaps between "Erasmus :", "SHOGUN", "Score:" otherwise showed the page
/// through). Two kinds of row return `None`, keeping the canvas byte-identical to
/// before: a row with no explicit background (Zork0's compass letters — explicit
/// FG only, no bg — so their windows never paint an opaque box over the banner
/// art), and a PURE reverse-video row (every run reversed — Zork0's on-banner ribbon
/// labels), which [`fill_reverse_row_gaps`] already handles edge to edge with the
/// over-art gate that leaves the art untouched. A mixed row (some reversed runs,
/// some not) still floods when it names an explicit bg, first-explicit-wins.
fn row_flood_bg(runs: &[&PxText], default_bg: Rgba<u8>, colors: &ColorScheme) -> Option<Rgba<u8>> {
    // Pure reverse-video (or empty) rows are owned by `fill_reverse_row_gaps`.
    if runs.is_empty() || runs.iter().all(|t| t.style & 1 != 0) {
        return None;
    }
    let bg = runs.iter().map(|t| t.bg).find(|&p| packed_explicit(p))?;
    Some(packed_to_rgba(bg, default_bg, colors))
}

/// SQ-0519: flood the window-width background of each explicit-bg chrome grid row
/// (see [`row_flood_bg`]) BEFORE its glyphs stamp, so an explicitly-coloured status
/// band (Shogun's black-on-white location/score bar) reads as one solid bar across
/// the whole window — not just behind each run. `ox`/`win_w` are the window's own
/// native pixel extent: the flood spans only THIS window (unlike the screen-wide
/// pure-reverse SQ-0504 fill and unlike the hybrid full-width title-bar rule
/// SQ-0515, which are look decisions on other paths). Runs carry screen-absolute
/// pixel rows, so each row floods at its own run `y` (`y - 1`, one `FONT_H` tall).
fn fill_explicit_bg_rows(
    canvas: &mut RgbaImage,
    texts: &[&PxText],
    ox: u32,
    win_w: u32,
    default_bg: Rgba<u8>,
    colors: &ColorScheme,
    tf: &crate::native_font::TextFace,
) {
    let font_w = u32::from(tf.cell().w());
    let font_h = u32::from(tf.cell().h());
    use std::collections::BTreeMap;
    let mut rows: BTreeMap<u32, Vec<&PxText>> = BTreeMap::new();
    for t in texts {
        rows.entry(t.y.max(1) as u32 - 1).or_default().push(t);
    }
    for (py, runs) in rows {
        if let Some(bg) = row_flood_bg(&runs, default_bg, colors) {
            // The point of this flood is to close the GAPS BETWEEN runs, so a bar
            // the game painted as several runs reads as one solid block — for a row
            // that IS a bar. The runs' hull answers that question below; what the
            // answer then licenses is both the reach past the outermost runs and the
            // bridging between them (SQ-0784).
            let lo = runs.iter().map(|t| u32::from(t.x.max(1)) - 1).min().unwrap_or(ox);
            let hi = runs
                .iter()
                .map(|t| (u32::from(t.x.max(1)) - 1) + tf.run_px_styled(&t.text, t.style).max(font_w))
                .max()
                .unwrap_or(ox + win_w);
            // A window is the bar only when its runs REACH BOTH OF ITS EDGES —
            // within one character cell, the padding a game leaves at the ends of a
            // band it filled. Shogun's status band is that: runs 49..592 in a 46..594
            // window, three pixels of slack at one end and two at the other, so the
            // flood rounds it out edge to edge and the gaps between "Erasmus :",
            // "SHOGUN" and "Score:" close.
            //
            // Anything else is a label parked in a scratch window whose box describes
            // nothing, and flooding that box smears the label's background across the
            // screen. scopa positions its "abort"/"OK" button labels with one window 5
            // it moves and resizes for every draw — and whose size its `measure`
            // routine leaves at a 1000×1000 sentinel, clamped to the screen. Its
            // "abort" run lands at 567..607 while the box reads 579..640, outside on
            // the left (SQ-0706); selecting a card redraws the same button's label as
            // "OK" at 579..595, which starts exactly ON that left edge and stops 45 px
            // short of the right one — inside the box, but 45 px is not padding. That
            // flooded a white tab from the button's rounded outline out to the screen
            // edge, which is what the player saw as the OK label spreading rightwards
            // (SQ-0721). There, flood only what the runs occupy.
            //
            // SQ-0784: and that same answer decides whether the row BRIDGES. A bar is
            // one continuous band, so its gaps are seams to close; anything else is a
            // set of independent labels, and the ground between two of them belongs to
            // the window, not to either label. Flooding a non-bar row's hull painted
            // straight across whatever the game had put in the gap: scopa's end-of-hand
            // score screen prints `Denari` (native 154) and `Primiera` (466) on one row
            // of its full-screen grid, and the two pairs of totals beneath them (native
            // 14 and 370) on two more, with its two blue card panels either side of a
            // green divider at native 350..360 — and the hull flood ran 153..529 and
            // 13..385 through the
            // divider, three blue bridges across a gap the game had deliberately left
            // open. Filling each run's own cells instead leaves the divider alone;
            // Shogun's status band, which reaches both window edges, still closes the
            // gaps between "Erasmus :", "SHOGUN" and "Score:" through the branch above.
            let spans_window = lo <= ox + font_w && hi + font_w >= ox + win_w;
            if spans_window {
                let (fx, fe) = (lo.min(ox), hi.max(ox + win_w));
                fill_cell(canvas, fx, py, fe.saturating_sub(fx), font_h, bg);
            } else {
                for t in &runs {
                    let x0 = u32::from(t.x.max(1)) - 1;
                    let w = tf.run_px_styled(&t.text, t.style).max(font_w);
                    fill_cell(canvas, x0, py, w, font_h, bg);
                }
            }
        }
    }
}

/// SQ-0779: the COLUMN analogue of [`clear_text_rows`] — erase the native columns of
/// a border the ring stamps as a CHARACTER, over `rows` (`[y0, y1)`).
///
/// `clear_text_rows` has always carved a text strip's native rows out of this canvas
/// so a band cannot rasterise a glyph the cells already draw. A border COLUMN the
/// hybrid ring stamps (SQ-0750) had no such carve, and one is exactly as necessary:
/// a flank band's source crop is its DESTINATION rect mapped back through the
/// letterbox scale, so trimming the destination by whole terminal columns moves the
/// crop's left edge to a native column that is still inside the border's own 8-pixel
/// text cell. Journey's `│` inks native x 3 of the cell at x 0..8; at a 234-column
/// pane the trimmed band began at native x 2 and carried that stroke into the
/// picture — the game's own rule, rasterised, standing beside the font glyph we
/// stamped for it. Scale-dependent, because at a smaller scale the band's first
/// native column lands past the stroke and it vanishes: native 5 at a 119-column
/// pane, native 2 at 234.
///
/// `cols` are native `[x0, x1)` spans — the character CELL, not the inked stroke,
/// since the whole cell is what the stamped glyph stands for.
pub fn clear_text_columns(canvas: &mut RgbaImage, cols: &[(u32, u32)], rows: (u32, u32)) {
    let (w, h) = (canvas.width(), canvas.height());
    let (y0, y1) = (rows.0.min(h), rows.1.min(h));
    for &(x0, x1) in cols {
        for y in y0..y1 {
            for x in x0.min(w)..x1.min(w) {
                canvas.put_pixel(x, y, Rgba([0, 0, 0, 0]));
            }
        }
    }
}

/// SQ-0504/SQ-0902, **retired by SQ-0903 and kept only as history.**
///
/// This carved a text strip's native rows out of the chrome canvas, keeping every
/// pixel the ART canvas accounted for, because [`build_chrome_canvas`] had already
/// rasterised glyphs the hybrid ring was about to draw as crisp cells. The rule it
/// enforced still holds — on a row the ring draws with GLYPHS the canvas keeps
/// artwork and nothing else (SQ-0750) — but it is enforced by not painting those
/// rows in the first place ([`TextLayer::SkipGlyphRows`]) rather than by erasing
/// them afterwards.
///
/// Rasterise-then-erase was never an oversight; it looked like an ordering
/// constraint, because the strip classification seemed to need the canvas. It did
/// not: of the 701 lines between the canvas build and the first read of it, seven
/// touched `canvas` and all seven were the construction. Moving it down past the
/// classification was the whole fix.
///
/// **What it cost while it existed is worth recording**, because it is the reason
/// the boundary bug SQ-0902 fixed could hide at all: a sequence that paints and
/// then unpaints has two places to be wrong about where the boundary is, and they
/// were wrong differently. It also cost real work — Journey's carve removed 61,440
/// pixels per frame, 90% of them outside any run's glyph span, in
/// `fill_explicit_bg_rows`' full-window flood.
pub fn clear_text_rows(canvas: &mut RgbaImage, runs: &[(u16, u32, u32)], cell: V6Cell) {
    let font_h = u32::from(cell.h());
    let (w, h) = (canvas.width(), canvas.height());
    for &(top, x0, x1) in runs {
        let y0 = top as u32;
        let y1 = (y0 + font_h).min(h);
        for y in y0..y1 {
            for x in x0.min(w)..x1.min(w) {
                canvas.put_pixel(x, y, Rgba([0, 0, 0, 0]));
            }
        }
    }
}

/// The ink a chrome `px_texts` run is drawn in, and the block it sits on — `None`
/// for "no block", meaning whatever is already behind the run shows through.
///
/// Extracted from [`build_chrome_canvas`]'s glyph loop (SQ-0944) because a SECOND
/// caller now needs the identical answer: on a backend that can put a terminal
/// glyph in a cell its artwork covers, `screen::stamp_runs_over_art` draws these
/// same runs as glyphs instead of pixels, and it has to reach the same colours or
/// the two renderings of one frame disagree. §6 of the pipeline document lists
/// "two places deciding the same thing by different rules" as a defect class this
/// file already suffers from, so there is one rule and both paths call it.
///
/// `over_art` is deferred because the answer costs a region scan and only the
/// inherited-colours-plus-reverse branch needs it — the same laziness the inline
/// code had.
pub(crate) fn chrome_run_ink(
    t: &PxText,
    default_fg: Rgba<u8>,
    default_bg: Rgba<u8>,
    colors: &ColorScheme,
    over_art: impl FnOnce() -> bool,
) -> (Rgba<u8>, Option<Rgba<u8>>) {
    // A packed colour is EXPLICIT only when the game named a real colour (see
    // `packed_explicit`): inherited colours + reverse over frame art (Zork0's
    // ribbon labels) must NOT paint an opaque block — the original renders dark
    // ink directly ON the art. A block is painted only when the game chose colours.
    if t.style & 1 == 0 {
        return (
            packed_to_rgba(t.fg, default_fg, colors),
            packed_explicit(t.bg).then(|| packed_to_rgba(t.bg, default_bg, colors)),
        );
    }
    if packed_explicit(t.fg) || packed_explicit(t.bg) {
        // Real colour pair: swap and paint the block.
        return (packed_to_rgba(t.bg, default_bg, colors), Some(packed_to_rgba(t.fg, default_fg, colors)));
    }
    // Inherited colours + reverse: whether to paint a block depends on what's
    // BEHIND the run (SQ-0487). Over opaque frame art (Zork0's ribbon labels) a
    // block would erase the art, so draw dark ink (default_bg) directly on it, no
    // block. Over a CLEAR background (Shogun's boot-menu selection bar — no art
    // behind it) the highlight must be visible, so paint the swapped block: a
    // solid default_fg bar with default_bg ink, INCLUDING the blank gap runs the
    // game paints between the item's words (a reversed space then fills its cell
    // — no more moth-eaten bar).
    if over_art() { (default_bg, None) } else { (default_bg, Some(default_fg)) }
}

/// Rasterise every chrome window into one native-sized canvas.
///
/// How much of the TEXT layer this canvas owes pixels for (SQ-0903).
///
/// A parameter rather than a flag, and an enum rather than a bare set, because
/// the two render paths want opposite things and a call site should say which it
/// is: passing an empty set would be indistinguishable from forgetting.
#[derive(Debug, Clone, Copy)]
pub enum TextLayer<'a> {
    /// Image every run. The **raster** composite has no cells to draw text with,
    /// so every glyph it will ever show has to be a pixel in this canvas.
    All,
    /// Skip these native row TOPS. The **hybrid** ring has already decided to
    /// draw them with terminal glyphs, so every pixel painted there is one it
    /// throws away — which is what it used to do, by carving them back out.
    SkipGlyphRows(&'a std::collections::HashSet<u16>),
}

impl TextLayer<'_> {
    fn skips(&self, native_top: u16) -> bool {
        match self {
            TextLayer::All => false,
            TextLayer::SkipGlyphRows(rows) => rows.contains(&native_top),
        }
    }

    /// The same question asked of a native SCAN LINE rather than of a run.
    ///
    /// A skipped run owns the whole `FONT_H` cell under its top, so a caller that
    /// walks pixels — [`fill_window_pages`] does — needs to know whether the line
    /// it is on falls inside one. Kept beside [`skips`](TextLayer::skips) so the
    /// two can never disagree about how tall a claimed row is.
    fn skips_line(&self, y: u32, cell: V6Cell) -> bool {
        match self {
            TextLayer::All => false,
            TextLayer::SkipGlyphRows(rows) => {
                let h = u32::from(cell.h());
                rows.iter().any(|&top| (top as u32..top as u32 + h).contains(&y))
            }
        }
    }
}

/// Rasterise every chrome window into one native-sized canvas.
///
/// `text` says how much of the text layer to image; see [`TextLayer`]. When it
/// skips a row it skips the row *entirely* — glyph strokes, reverse-row gap fill
/// and explicit-background flood alike — because two of those three flood a whole
/// row and Journey's carve was 90% flood.
///
/// **Pass 1 — the artwork — is never skipped.** A row the ring draws as glyphs may
/// still carry art beneath them, and the bands crop that art out of this canvas.
/// Zork Zero is the case that proves it matters: every one of its chrome runs sits
/// on the banner ribbon, so it is an Art strip throughout, no row of it is ever a
/// glyph row, and nothing here is skipped for it (SQ-0750).
/// Re-join the runs the ENGINE's own pen laid down side by side (SQ-1009).
///
/// # Why this exists
///
/// A v6 grid window publishes ONE RUN PER CHARACTER. Arthur's score bar arrives as
/// 73 of them — `(29, "C"), (39, "h"), (49, "u"), …` — each where `zvm`'s own
/// cursor stopped, which since SQ-1009 is the machine's PEN rather than the
/// declared cell: the engine and the renderer measure through one
/// [`zvm::screen::V6Metric`], handed over at boot, so the origins already step the
/// face's advances.
///
/// But it means a proportional pen cannot be applied one run at a time. Each glyph
/// would be stamped at its engine column and drawn at its own narrower width, and
/// the difference would open as a gap before every letter — which is exactly what
/// `Church` looked like, and exactly what `machine-screenshots/amiga-arthur-church.png`
/// does NOT show: there the glyph origins step 12, 10, 10, 10, 10 device px, which
/// is `C h u r c` from the face's own advance table and nothing else.
///
/// So a run whose origin is EXACTLY where the engine's pen left the previous one is
/// a continuation of it, and the two are drawn as one run with one pen. A run the
/// game positioned somewhere else — `set_cursor` for a right-hand field, a second
/// column — breaks the chain and keeps its own origin, which is what makes this
/// safe: nothing moves except the spacing INSIDE a run the engine already
/// considered contiguous.
///
/// Contiguity is asked in the PEN's units, because that is what the engine
/// advanced by. Asked in declared cells it would answer no for every glyph on a
/// proportional machine and the chain would never form.
///
/// **Identity when the face is not proportional**, so every other configuration
/// gets back exactly the list it passed in, in the order it passed it.
pub(crate) fn pen_chains(runs: &[&PxText], tf: &crate::native_font::TextFace) -> Vec<PxText> {
    if !tf.proportional() {
        return runs.iter().map(|t| (*t).clone()).collect();
    }
    // By row, then by column — the engine emits a window's cells in whatever order
    // the game printed them, and Arthur pads its bar to the right BEFORE writing
    // the location at the left. Stable, so two runs at one origin keep the order
    // that decided which of them overdraws the other.
    let mut order: Vec<&PxText> = runs.to_vec();
    order.sort_by_key(|t| (t.y, t.x));
    let mut out: Vec<PxText> = Vec::with_capacity(order.len());
    for t in order {
        let joins = out.last().is_some_and(|p: &PxText| {
            p.y == t.y
                && p.style == t.style
                && p.fg == t.fg
                && p.bg == t.bg
                && u32::from(p.x) + tf.run_px_styled(&p.text, p.style) == u32::from(t.x)
        });
        if joins {
            out.last_mut().expect("just checked").text.push_str(&t.text);
        } else {
            out.push(t.clone());
        }
    }
    out
}

pub fn build_chrome_canvas(
    chrome: &[&PositionedWindow],
    native: (u16, u16),
    default_fg: Rgba<u8>,
    default_bg: Rgba<u8>,
    colors: &ColorScheme,
    text: TextLayer<'_>,
    // The cell, the release's face and the pen, as one value (SQ-1009).
    tf: &crate::native_font::TextFace,
) -> RgbaImage {
    let cell = tf.cell();
    let font_w = u32::from(cell.w());
    let font_h = u32::from(cell.h());
    let mut canvas = RgbaImage::new(native.0 as u32, native.1 as u32);

    // Pass 1 — Graphics entries.
    blit_chrome_graphics(&mut canvas, chrome);
    // The ART layer, frozen (SQ-0727). Every "is this run sitting on artwork?"
    // question below (SQ-0487's per-run block rule, SQ-0499's gap fill) is asked
    // of THIS canvas, never of the live one: rasterized text is itself opaque, so
    // a live probe answers "yes, artwork" for a run whose span another run's own
    // highlight block already claimed — the lesson `build_graphics_canvas` records
    // for the hybrid side (SQ-0500).
    //
    // advent.z6's help screen is the case that needed it. Its navigation bar is a
    // pure reverse-video row painted as one run per label plus reversed spacer
    // spaces, and the spacer at x=289 lands INSIDE "About Adventure" (248..368).
    // The spacer draws first, so by the time the label's turn came the probe saw
    // the spacer's own white block, concluded the label sat on frame art, and drew
    // it as dark ink with no block — black ink that `flatten_onto_page` then
    // resolved onto a black page. The whole navigation bar was invisible in the
    // raster composite while rendering correctly as cells.
    let art = canvas.clone();

    // Pass 2 — Grid (status) entries, in list order. A v6 grid with
    // pixel-positioned runs draws those at their EXACT game pixel positions
    // (Zork Zero's banner text sits at rows 6/14, on the ribbon art — cell
    // quantization would snap it to the banner's top edge); the cell grid is
    // the fallback for grids without them.
    for it in chrome {
        if let WinNode::Grid(g) = &it.node {
            let ox = it.x_px as u32;
            let oy = it.y_px as u32;
            // SQ-0903: the runs this canvas still owes pixels for. A run whose row
            // the ring draws with glyphs is skipped here rather than painted and
            // carved back out — and the filter is applied ONCE, before all three
            // painters below, because two of them flood a whole ROW. Journey is
            // why that matters: 90% of what its carve used to remove lay outside
            // any run's glyph span, in `fill_explicit_bg_rows`' full-window flood.
            // Skipping only the glyphs would have left it behind.
            //
            // Asked of `g.px_texts`, not of the filtered list (SQ-0944): a grid
            // that carries pixel runs is drawn from its RUNS, and whether any of
            // them survived the skip does not change that. Gating the `continue`
            // on the survivors let a grid whose runs the ring took ALL of fall
            // through to the cell-grid painter below, which redraws the same
            // characters at `oy + row * font_h` — positions a set keyed on the
            // runs' own tops can never match. Zork Zero's banner is the case:
            // runs at native 10 and 26, both skipped, both painted straight back
            // in at 0 and 16, a text row above where the ring's glyphs land. That
            // is the ghost the half-block capture showed beside crisp letters.
            if !g.px_texts.is_empty() {
                let kept: Vec<&PxText> =
                    g.px_texts.iter().filter(|t| !text.skips(t.y.max(1) - 1)).collect();
                if kept.is_empty() {
                    continue;
                }
                // SQ-1009: one run per character is how a grid publishes a line, and
                // a per-glyph pen has to see the whole line to place it. Identity for
                // every face that is not proportional.
                let joined = pen_chains(&kept, tf);
                let px_texts: Vec<&PxText> = joined.iter().collect();
                // **The window's own right edge bounds the pen** (SQ-1026).
                //
                // ZMSD §8.8's window property 7 is a RIGHT MARGIN, and `zvm` lays
                // its own prose out against `x_size - right_margin` — but nothing
                // under `render/` had ever consulted either, and the renderer drew
                // rightward from a run's origin with no bound at all. At a fixed
                // cell that was invisible: our text was NARROWER than the machine's
                // proportional face, so a run always finished inside the box the
                // game had reserved for it. The pen removed the slack and the
                // omission surfaced — the pen exposed this, it did not cause it.
                //
                // Applied only to a PROPORTIONAL face, deliberately. A run's
                // coordinates are stamped where it was PAINTED and the window may
                // have moved or shrunk since (Shogun turns its menu window into a
                // 1-px caret after printing), so bounding every run by its window's
                // current box would erase text the game means to be on screen. On
                // the one press that has a proportional face those windows are
                // stable, and the alternative is glyphs drawn across the frame art.
                let bound = (tf.proportional() && (it.w_px as i16) >= 0)
                    .then(|| (ox + it.w_px as u32).saturating_sub(u32::from(it.right_margin)));
                // The run colour rule itself now lives in `chrome_run_ink`, which the
                // glyph loop below calls and so does the cell path that draws these
                // same runs as terminal glyphs (SQ-0944).
                //
                // Fill pure-reverse-row gaps FIRST, so the glyph loop paints the run
                // cells on top of them (SQ-0499). Both this fill and the glyph loop
                // put their over-art question to `art`, never to `canvas`.
                fill_reverse_row_gaps(&mut canvas, &art, &px_texts, default_fg, colors, tf);
                // SQ-0519: then flood the full WINDOW width of each explicit-bg,
                // non-reverse row with its own background, so an explicitly-coloured
                // status band (Shogun's black-on-white location/score bar) reads as
                // one solid bar in the pixel composite rather than showing the page
                // in the gaps between its runs. Only when the window's width is
                // resolved (a size sentinel would balloon the flood, SQ-0481). The
                // glyph loop then stamps the runs on top.
                if (it.w_px as i16) >= 0 {
                    fill_explicit_bg_rows(&mut canvas, &px_texts, ox, it.w_px as u32, default_bg, colors, tf);
                }
                for t in &px_texts {
                    let px0 = t.x.max(1) as u32 - 1;
                    let py = t.y.max(1) as u32 - 1;
                    // Run coords are SCREEN-absolute 1-based pixels stamped at
                    // paint time (v6 paint semantics) — no window-origin
                    // offset: the window may have moved/shrunk since (Shogun
                    // turns its menu window into a 1-px caret after printing).
                    // The run's own §8.7.1 style byte rides along: the raster
                    // font synthesizes bold/italic (SQ-0540). Reverse (bit 1) is
                    // already resolved into the fg/bg pair above and fixed-pitch
                    // (bit 8) is a no-op in a bitmap font, so `blit_glyph_styled`
                    // ignores both — passing the raw byte can't double-apply.
                    // A running pen (SQ-1009): fixed-pitch faces step by the cell
                    // exactly as `px0 + i * font_w` did, and a proportional one steps
                    // by each glyph's own advance. The run's ORIGIN is the game's, so
                    // a machine that positioned every label by pixel — Arthur's
                    // status line, its inventory columns, its map captions — comes
                    // out right without the engine's cursor moving at all.
                    //
                    // **The over-art question is the GLYPH's, not the run's**
                    // (SQ-1052). `region_has_opaque` answers "is ANY pixel here
                    // opaque?", which is a fair question about one character cell
                    // and a meaningless one about a long run: a single stray pixel
                    // anywhere beneath it condemns the whole thing. That was
                    // harmless while a v6 grid published ONE RUN PER CHARACTER —
                    // every probe was one cell wide — and stopped being harmless
                    // the moment `pen_chains` began joining those runs into lines.
                    //
                    // Macintosh Arthur is the report. Its score bar arrives as 123
                    // inherited-reverse runs which join into three: ` Churchyard`,
                    // eighty-eight padding spaces, and `St Anne's Day, Compline `.
                    // The padding chain's declared span is 616 px — the bar, the
                    // poles at both ends and 80 px past the screen — so it found
                    // frame art, took SQ-0487's "draw dark ink on the artwork, no
                    // block" arm, and eighty-eight cells of what should have been a
                    // white ribbon came out as page. The date chain overshot into
                    // the right-hand pole and went the same way. Only the location,
                    // whose chain is short enough to clear the art, survived — the
                    // reported "only the location, reversed, and the rest blank".
                    //
                    // Asked per glyph it is the same question the unjoined runs
                    // asked, at the same coordinates: the chain walks the engine's
                    // own pen, which is where those runs were. So this restores the
                    // pre-SQ-1009 answer for a proportional face and leaves every
                    // fixed one — where nothing joins — byte-identical, except that
                    // a run half over artwork now resolves per cell instead of
                    // letting its first opaque pixel speak for all of it.
                    //
                    // `art` is pass 1 frozen, so the question sees the real artwork
                    // (or transparency) and never another run's own block.
                    let mut pen = px0;
                    for ch in t.text.chars() {
                        let adv = tf.advance_styled(ch, t.style);
                        if let Some(right) = bound {
                            if pen + adv > right {
                                break;
                            }
                        }
                        // The cell this glyph reserves: its own advance, never
                        // narrower than the declared cell, so a proportional face
                        // probes at least the rectangle the game laid out.
                        let span_w = adv.max(font_w);
                        let (fg, bg) = chrome_run_ink(t, default_fg, default_bg, colors, || {
                            region_has_opaque(&art, pen, py, span_w, font_h)
                        });
                        crate::render::bitfont::blit_glyph_styled(&mut canvas, ch, pen, py, font_w, font_h, fg, bg, t.style, Some(tf));
                        pen += adv;
                    }
                }
                continue;
            }
            for row in 0..g.rows {
                let py = oy + row as u32 * font_h;
                // SQ-0903, the cell-grid half. Same rule, same reason: the ring
                // draws this row with glyphs, so imaging it here is work whose
                // only consumer is the carve that used to follow.
                if text.skips(py as u16) {
                    continue;
                }
                for col in 0..g.cols {
                    let idx = row as usize * g.cols as usize + col as usize;
                    let Some(cell) = g.cells.get(idx) else { continue };
                    let px = ox + col as u32 * font_w;
                    if cell.ch == '\0' || cell.ch == ' ' {
                        if cell.bg != 0 {
                            let b = packed_to_rgba(cell.bg, Rgba([0, 0, 0, 255]), colors);
                            fill_cell(&mut canvas, px, py, font_w, font_h, b);
                        }
                        continue;
                    }
                    let fg = packed_to_rgba(cell.fg, default_fg, colors);
                    let cellbg = (cell.bg != 0).then(|| packed_to_rgba(cell.bg, Rgba([0, 0, 0, 255]), colors));
                    // A grid CELL is addressed by column and stays on the grid —
                    // the game's own `set_cursor` counted these columns, so a pen
                    // here would place a character where nothing asked for it.
                    crate::render::bitfont::blit_glyph_styled(&mut canvas, cell.ch, px, py, font_w, font_h, fg, cellbg, cell.style, Some(tf));
                }
            }
        }
    }

    canvas
}

/// Draw every SECONDARY PROSE window's lines onto the pixel composite (SQ-0729).
///
/// A v6 game's second flowing-text window is published as a non-primary `Buffer`
/// (SQ-0585), and [`build_chrome_canvas`] draws Graphics and Grid windows and
/// nothing else — so every line such a window carried was absent from the raster
/// screen while both cell paths showed it. fmvpoker is the report: it prints its
/// menu bar and "Select an option with your mouse or by typing the first letter."
/// into one, and the composite showed neither. It matters more since the same
/// quest routed fmvpoker's hybrid frames here.
///
/// Separate from `build_chrome_canvas` because the ink is `honor_game_colours`-
/// gated and that function is not: `ink` is the caller's already-resolved page ink
/// (the game's own where honored, else the host's), and the window's OWN colour is
/// consulted only when the player is honoring game colours. Painting fmvpoker's
/// declared black regardless put black glyphs on the host's black page.
///
/// Placement — and therefore what [`fill_story_page_under_chrome_text`] must spare
/// — is [`buffer_line_rects`].
///
/// `input` is the host's live input line, drawn (with its caret) after the last line
/// of the window the game is reading through — `BufferWindow::reads_input`, SQ-0746.
/// A game may read through a panel it has declared is not the transcript, and the
/// echo of what the player types has to appear where they are typing it: fmvpoker's
/// "Enter the new bet: " is printed into its bottom panel and read from there, and
/// every digit typed was invisible. `None` when the view is scrolled back, matching
/// the transcript's own rule for the live line.
pub fn draw_secondary_prose(
    canvas: &mut RgbaImage,
    chrome: &[&PositionedWindow],
    ink: Rgba<u8>,
    honor: bool,
    colors: &ColorScheme,
    input: Option<&str>,
    tf: &crate::native_font::TextFace,
) {
    let cell = tf.cell();
    let font_w = u32::from(cell.w());
    let font_h = u32::from(cell.h());
    for it in chrome {
        let WinNode::Buffer(b) = &it.node else { continue };
        let fg = match b.fg.filter(|_| honor) {
            Some(p) => packed_to_rgba(p, ink, colors),
            None => ink,
        };
        let right = it.x_px as u32 + it.w_px as u32;
        let rects = buffer_line_rects(it, tf);
        for (line, &(x0, y0, _, _)) in b.lines.iter().zip(&rects) {
            let mut pen = x0;
            for ch in line.chars() {
                let adv = tf.advance(ch);
                if pen + adv > right {
                    break;
                }
                crate::render::bitfont::blit_glyph(canvas, ch, pen, y0, font_w, font_h, fg, None, Some(tf));
                pen += adv;
            }
        }
        // The live input line, when the player is typing into THIS window
        // (SQ-0746). It continues the window's last line — the prompt the game
        // printed and then read after, "Enter the new bet: " — exactly as
        // `draw_story_text` continues the transcript's kept prompt row, with the
        // caret one cell past what has been typed.
        let Some(input) = input.filter(|_| b.reads_input) else { continue };
        // With nothing in the window yet the read starts at its own top-left, the
        // same place the window's first line would have gone.
        let (x0, y0) = rects.last().map_or(
            (it.x_px as u32 + it.left_margin as u32, it.y_px as u32),
            |&(x0, y0, _, _)| (x0, y0),
        );
        let start = x0
            + rects.len().checked_sub(1).map_or(0, |i| tf.run_px(&b.lines[i]));
        let mut pen = start;
        for (i, ch) in input.chars().chain(std::iter::once(' ')).enumerate() {
            let adv = tf.advance(ch);
            if pen + adv > right {
                break;
            }
            // The caret is the cell after the input, drawn as the block the
            // transcript's own caret uses.
            if i == input.chars().count() {
                fill_cell(canvas, pen, y0, font_w, font_h, fg);
            } else {
                crate::render::bitfont::blit_glyph(canvas, ch, pen, y0, font_w, font_h, fg, None, Some(tf));
            }
            pen += adv;
        }
    }
}

/// Draw the STORY window's own streamed runs where they are sitting on the v6
/// screen (SQ-0729) — [`crate::engine::BufferWindow::px_runs`].
///
/// For a story window that is a CANVAS rather than a page (see
/// `screen::story_window_is_a_canvas`) this replaces the transcript entirely: the
/// window's live screen state is these runs, at the coordinates the game's own
/// `set_cursor` named, and a scrolling re-render of everything it ever printed is
/// the wrong reading of it. fmvpoker is the shape — it prints "HOLD" under each
/// card it is holding at `set_cursor(row=203, col=70/183/296/409/522)` and its
/// running totals at (76,247), (76,265), (420,247), (420,265), and every one of
/// them arrived in the story scroll instead.
///
/// Colour follows the same gate as [`draw_secondary_prose`]: the game's own pair
/// only when the player is honoring game colours, else the host's `ink` on its
/// `page`. Reverse (§8.7.1 bit 1) swaps the pair, which is how fmvpoker marks a
/// held card; a blank run in the game's own background is how it un-marks one, so
/// an explicit background is painted as a block rather than skipped.
pub fn draw_story_canvas_runs(
    canvas: &mut RgbaImage,
    story: Option<&PositionedWindow>,
    ink: Rgba<u8>,
    page: Rgba<u8>,
    honor: bool,
    colors: &ColorScheme,
    tf: &crate::native_font::TextFace,
) {
    let cell = tf.cell();
    let font_w = u32::from(cell.w());
    let font_h = u32::from(cell.h());
    let Some(it) = story else { return };
    let WinNode::Buffer(b) = &it.node else { return };
    // SQ-1009: a canvas window publishes one run per character just as a grid does.
    let refs: Vec<&PxText> = b.px_runs.iter().collect();
    let runs = pen_chains(&refs, tf);
    // The story window's own right edge, on the same rule and for the same reason
    // as the grid path above (SQ-1026).
    let bound = (tf.proportional() && (it.w_px as i16) >= 0)
        .then(|| (it.x_px as u32 + it.w_px as u32).saturating_sub(u32::from(it.right_margin)));
    for t in &runs {
        let (mut fg, mut bg) = if honor {
            (
                packed_to_rgba(t.fg, ink, colors),
                packed_explicit(t.bg).then(|| packed_to_rgba(t.bg, page, colors)),
            )
        } else {
            (ink, None)
        };
        if t.style & 1 != 0 {
            let block = bg.unwrap_or(page);
            bg = Some(fg);
            fg = block;
        }
        // Screen-absolute 1-based pixels, stamped where the run was printed —
        // no window-origin offset, exactly like a grid window's `px_texts`.
        let (px0, py) = (t.x.max(1) as u32 - 1, t.y.max(1) as u32 - 1);
        let mut pen = px0;
        for ch in t.text.chars() {
            if let Some(right) = bound {
                if pen + tf.advance_styled(ch, t.style) > right {
                    break;
                }
            }
            crate::render::bitfont::blit_glyph_styled(canvas, ch, pen, py, font_w, font_h, fg, bg, t.style, Some(tf));
            pen += tf.advance_styled(ch, t.style);
        }
    }
}

/// Where a SECONDARY prose window's lines land on the pixel composite (SQ-0729),
/// one `(x0, y0, x1, y1)` per line it carries, in the order of `lines`.
///
/// A `Buffer` is flowing prose with no pixel runs to place, so its lines stack from
/// the window's origin (plus the game's own left margin), one 16px text row each,
/// and stop at the bottom of the box the game declared — which is where the cell
/// paths put them too. Shared by the draw in [`build_chrome_canvas`] and by
/// [`chrome_text_rects`], whose caller must spare exactly the pixels the draw
/// claims; measuring them twice is how Shogun's menu got erased once already.
///
/// A PRIMARY buffer is the transcript and is not drawn here at all — it yields
/// nothing.
fn buffer_line_rects(it: &PositionedWindow, tf: &crate::native_font::TextFace) -> Vec<(u32, u32, u32, u32)> {
    let font_h = u32::from(tf.cell().h());
    let WinNode::Buffer(b) = &it.node else { return Vec::new() };
    if b.primary {
        return Vec::new();
    }
    let x0 = it.x_px as u32 + it.left_margin as u32;
    let bottom = it.y_px as u32 + it.h_px as u32;
    let right = it.x_px as u32 + it.w_px as u32;
    let mut out = Vec::new();
    for (row, line) in b.lines.iter().enumerate() {
        let y0 = it.y_px as u32 + row as u32 * font_h;
        if y0 + font_h > bottom {
            break;
        }
        // The PEN again (SQ-1054): `draw_secondary_prose` steps `tf.advance` down
        // this very list, so the rect and the draw must agree. The doc above
        // already said "spare exactly the pixels the draw claims" — they were
        // measured twice, in two different units.
        let x1 = (x0 + tf.run_px(line)).min(right);
        out.push((x0, y0, x1, y0 + font_h));
    }
    out
}

/// A uniform (aspect-preserving) letterbox scale from native game pixels to
/// pane device pixels, plus the device-pixel offset of the letterboxed area.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale {
    pub s: f32,
    pub off_x: u32,
    pub off_y: u32,
}

/// Compute the uniform letterbox scale that fits `native` game-pixel
/// dimensions into `pane_dev` device-pixel dimensions, centering the result.
pub fn uniform_scale(native: (u16, u16), pane_dev: (u32, u32)) -> Scale {
    let nw = if native.0 == 0 { 1 } else { native.0 as u32 } as f32;
    let nh = if native.1 == 0 { 1 } else { native.1 as u32 } as f32;
    let s = (pane_dev.0 as f32 / nw).min(pane_dev.1 as f32 / nh);
    centred(native, pane_dev, s)
}

/// Centre a screen scaled by `s` inside `pane_dev`. The one place the letterbox
/// offsets are computed, so the free and locked scales cannot drift apart.
fn centred(native: (u16, u16), pane_dev: (u32, u32), s: f32) -> Scale {
    let nw = if native.0 == 0 { 1 } else { native.0 as u32 } as f32;
    let nh = if native.1 == 0 { 1 } else { native.1 as u32 } as f32;
    let off_x = ((pane_dev.0 as f32 - nw * s) / 2.0).max(0.0) as u32;
    let off_y = ((pane_dev.1 as f32 - nh * s) / 2.0).max(0.0) as u32;
    Scale { s, off_x, off_y }
}

/// Greatest common divisor, for [`scale_ladder_step`]. Both inputs are clamped to
/// at least 1 by the caller, so this never sees a zero.
fn gcd(a: u32, b: u32) -> u32 {
    let (mut a, mut b) = (a, b);
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

/// The step of the magnification LADDER implied by `art_scale` — the smallest
/// increment of the uniform letterbox scale `s` that keeps one ART pixel a whole
/// number of device pixels (SQ-0936).
///
/// This is DERIVED, not chosen, and that is the whole point. What has to land on
/// a device-pixel boundary is the ART pixel, because that is where
/// nearest-neighbour crispness lives — the unit-screen pixel is an interpreter
/// fiction that no artist ever drew. An art pixel is `art_scale` unit pixels
/// (`crate::graphics::PictSource::art_scale`, per axis, computed at boot from the
/// archive's own declared picture space) and a unit pixel is `s` device pixels, so
/// both axes need `art_scale.N · s ∈ ℤ`, and the coarsest `s` satisfying both is
///
/// ```text
///     step = 1 / gcd(art_scale.0, art_scale.1)
/// ```
///
/// Every press the corpus has falls out of that one line:
///
/// ```text
///   press                      art space  art_scale  gcd  step  ladder
///   most v6 (Blorb/Amiga/DOS)   320x200    (2, 2)      2   1/2   0.5, 1, 1.5, 2 …
///   Macintosh mono Pic.data     480x300    (1, 1)      1   1     1, 2, 3 …
///   Macintosh colour CPic.data  320x200    (2, 2)      2   1/2   half-steps
///   EGA/CGA .eg1/.cg1           640x200    (1, 2)      1   1     1, 2, 3 …
///   Apple II                    140x192    (4, 2)      2   1/2   half-steps
/// ```
///
/// So the familiar "1x, 1.5x, 2x" intuition is right for the common case and
/// arrives as a consequence rather than an assumption — while the standard
/// Macintosh's mono plate and the 640-wide EGA/CGA renditions get whole steps
/// only, which is correct for them and which a hardcoded half-step ladder would
/// get wrong on both.
pub fn scale_ladder_step(art_scale: (u32, u32)) -> f32 {
    1.0 / gcd(art_scale.0.max(1), art_scale.1.max(1)) as f32
}

/// The uniform letterbox scale QUANTIZED down to the ladder
/// [`scale_ladder_step`] derives from `art_scale`, centred in the pane exactly as
/// [`uniform_scale`] centres the free one.
///
/// `None` when the pane cannot fit even the smallest step — the caller falls back
/// to free scaling rather than blocking or clipping (SQ-0936), which is what every
/// other too-small decision in this app does.
///
/// One GLOBAL factor for the whole native screen, never one per picture. Journey
/// is what settles that: its picture sits in its OWN window beside a drawn divider
/// rule, so quantizing per-picture would stop the art short of its own frame and
/// open a gap between picture and rule. Quantizing the screen's one factor moves
/// the window rect and the artwork in it together, and the art still fills it
/// exactly.
///
/// Tiling needs nothing here. Vertical tiling happens in the already-scaled space
/// and cuts at whole ART-pixel boundaries, so an integral art pixel makes every
/// tile height integral for free — crisp art and seamless flanks are the same
/// constraint, not two knobs.
/// What one Version 6 frame is made of, for anything that has to QUANTIZE it
/// (SQ-1024).
///
/// Three facts that must be considered together and were being passed
/// positionally: the unit screen, how dense the ARTWORK on it is, and the
/// character cell the TEXT on it is drawn at. The ladder needs all three, and a
/// caller that supplied two of them got a plausible answer — which is the same
/// failure shape as [`crate::machine_boot::MachineBoot`] one layer up, so it gets
/// the same treatment. A fourth fact will not touch a single call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameGeometry {
    /// The unit screen in native game pixels.
    pub native: (u16, u16),
    /// Unit pixels per ART pixel, per axis — the archive's density (SQ-0790).
    pub art_scale: (u32, u32),
    /// The character cell raster TEXT is drawn on — the machine's (SQ-0917).
    pub cell: zvm::screen::V6Cell,
}

impl FrameGeometry {
    pub fn new(
        native: (u16, u16),
        art_scale: (u32, u32),
        cell: zvm::screen::V6Cell,
    ) -> FrameGeometry {
        FrameGeometry { native, art_scale, cell }
    }

    /// Valid magnifications are the multiples of `1 / step()`.
    ///
    /// **The ladder serves the ARTWORK and the TEXT, and they are not the same
    /// constraint.** An art pixel is `art_scale` unit pixels, so art needs
    /// `art_scale · s` whole and admits steps of `1 / gcd(art_scale)`. Raster text
    /// is drawn on the character cell, also in unit pixels, so it needs
    /// `cell.w · s` and `cell.h · s` whole and admits steps of
    /// `1 / gcd(cell.w, cell.h)`. A rung has to satisfy both.
    ///
    /// On an 8x16 cell `gcd(8, 16)` is 8, so this changes nothing — `8 · 1.5 = 12`
    /// and `16 · 1.5 = 24` are both whole, which is why the half rungs have always
    /// been fine everywhere else and why nobody noticed. On the Macintosh's
    /// **7x15** cell `gcd(7, 15)` is **1**, so a half rung gives a 7-wide glyph
    /// 10.5 device pixels: its strokes alternate one and two, `l` and `i` go
    /// ragged, and the compass rose in the same frame stays perfectly crisp. That
    /// contrast inside one image is the signature (SQ-1012).
    ///
    /// So this is arithmetic about the cell, not a per-machine exception — a
    /// machine that declares some other cell gets the right answer without anyone
    /// adding a case for it.
    pub fn step(self) -> u32 {
        let g_art = gcd(self.art_scale.0.max(1), self.art_scale.1.max(1));
        let g_cell = gcd(u32::from(self.cell.w()), u32::from(self.cell.h()));
        gcd(g_art, g_cell).max(1)
    }

    /// The coarsest rung at or below the free scale, or `None` when the pane
    /// cannot hold even the smallest.
    pub fn locked_scale(self, pane_dev: (u32, u32)) -> Option<Scale> {
        locked_scale_inner(self, pane_dev)
    }

    /// The scale to draw this frame at: on the ladder when the player asked for it
    /// and a rung fits, free otherwise. The flag says which, for the diagnostic.
    ///
    /// A pane too small for the smallest rung degrades silently on the game screen,
    /// so the flag is what a caller publishes as a diagnostic (SQ-0936).
    pub fn fitted_scale(self, pane_dev: (u32, u32), lock: bool) -> (Scale, bool) {
        if !lock {
            return (uniform_scale(self.native, pane_dev), false);
        }
        match self.locked_scale(pane_dev) {
            Some(s) => (s, false),
            None => (uniform_scale(self.native, pane_dev), true),
        }
    }
}

/// SQ-1032: how tall the raster composite is BUILT, and the magnification it is
/// pinned to.
///
/// Two facts that are one decision, so they travel together rather than
/// positionally (the refactoring policy in CLAUDE.md, and the same shape as
/// [`FrameGeometry`] beside it): a canvas height, and the whole-pixel scale that
/// height was derived from. A caller handed only the height would letterbox the
/// taller canvas at a fractional scale and undo the whole point.
///
/// [`V6RenderMode::Raster`] builds the game's own screen and lets the pane
/// letterbox it. [`V6RenderMode::Extended`] grows the canvas DOWNWARD — the game's
/// screen is the top of it, untouched — so the pane's surplus height becomes whole
/// text rows of prose instead of empty margin. **Nothing is told a taller screen**:
/// `native` is what the game laid its windows out on and stays exactly that.
///
/// [`V6RenderMode::Raster`]: crate::config::V6RenderMode::Raster
/// [`V6RenderMode::Extended`]: crate::config::V6RenderMode::Extended
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RasterFrame {
    /// The game's own screen in native pixels — never changed by the extension.
    pub native: (u16, u16),
    /// The canvas height built, in native pixels. `native.1` unless extended.
    pub canvas_h: u32,
    /// The magnification the composite is pinned to, in device pixels per NATIVE
    /// pixel. `None` = the fitted letterbox every non-extended frame keeps.
    pub lock: Option<f32>,
}

impl RasterFrame {
    /// The game's screen and nothing more — today's letterboxed composite.
    pub fn native(native: (u16, u16)) -> RasterFrame {
        RasterFrame { native, canvas_h: u32::from(native.1), lock: None }
    }

    /// The extension this pane can afford: the largest magnification that fits
    /// the game's screen in `pane_dev` device pixels — whole, or fractional to
    /// match what `Raster`/`Hybrid` draw at the same pane, per `lock` — and as
    /// many whole text rows of surplus height as that scale leaves under it.
    ///
    /// `lock` is `v6_pixel_lock`, threaded through rather than defaulted: SQ-1239
    /// found Extended always taking the whole-magnification branch below,
    /// ignoring the toggle raster and hybrid both obey (`FrameGeometry::fitted_scale`).
    /// **Whole device pixels per NATIVE pixel** when `lock` is set, which is
    /// stricter than `v6_pixel_lock`'s whole-device-per-ART rung wherever
    /// `art_scale` is 2 — and stricter is what a locked extension needs, because
    /// its text is the thing being sized: raster text is drawn on the machine's
    /// cell in native pixels, so a half-native rung gives a 7-wide Macintosh glyph
    /// 10.5 device pixels and its strokes alternate one and two (SQ-1012, SQ-1024).
    /// A whole native rung cannot produce that on any cell. With `lock` clear the
    /// player has accepted that risk already in raster/hybrid, so Extended takes
    /// the same fractional scale rather than pretending the lock is always on.
    ///
    /// The surplus is measured in whole `cell.h` rows so the extension is a whole
    /// number of text rows of the game's own face — the raster prose box already
    /// floors its row count, so adding a multiple of the cell adds exactly that many
    /// rows and leaves whatever sub-row remainder the unextended box already had.
    ///
    /// Degrades to [`Self::native`] — today's letterbox, exactly — when the pane
    /// cannot hold the game's screen at 1:1. There is no extension to be had there,
    /// and a magnification below 1 is not a whole one.
    pub fn extended(
        native: (u16, u16),
        pane_dev: (u32, u32),
        cell: zvm::screen::V6Cell,
        cap: Option<f64>,
        lock: bool,
    ) -> RasterFrame {
        let plain = RasterFrame::native(native);
        if native.0 == 0 || native.1 == 0 || cell.h() == 0 {
            return plain;
        }
        let fit = (f64::from(pane_dev.0) / f64::from(native.0))
            .min(f64::from(pane_dev.1) / f64::from(native.1));
        let capped = cap.map_or(fit, |c| fit.min(c));
        let s = if lock { capped.floor() } else { capped };
        // NaN is impossible above (both divisors are guarded non-zero) but is stated
        // rather than assumed, because "not at least 1" and "less than 1" differ on it
        // and only one of them is safe to build a canvas from.
        if !s.is_finite() || s < 1.0 {
            return plain;
        }
        // The native rows the pane shows at `s`, and the whole text rows of them
        // that lie below the game's own screen.
        let rows_px = (f64::from(pane_dev.1) / s).floor() as u32;
        let extra = rows_px.saturating_sub(u32::from(native.1)) / u32::from(cell.h());
        RasterFrame {
            native,
            canvas_h: u32::from(native.1) + extra * u32::from(cell.h()),
            lock: Some(s as f32),
        }
    }

    /// The native rows the extension added below the game's own screen; 0 when the
    /// frame is the game's screen.
    pub fn extension(self) -> u32 {
        self.canvas_h.saturating_sub(u32::from(self.native.1))
    }
}

fn locked_scale_inner(geom: FrameGeometry, pane_dev: (u32, u32)) -> Option<Scale> {
    let free = uniform_scale(geom.native, pane_dev).s;
    // **The ladder serves the ARTWORK and the TEXT, and they are not the same
    // constraint** (SQ-1024). An art pixel is `art_scale` unit pixels, so art
    // needs `art_scale · s` whole and admits steps of `1 / gcd(art_scale)`.
    // Raster TEXT is drawn on the machine's character cell, which is in UNIT
    // pixels, so it needs `cell.w · s` and `cell.h · s` whole and admits steps of
    // `1 / gcd(cell.w, cell.h)`.
    //
    // A valid rung satisfies both, so it is a multiple of
    // `1 / gcd(g_art, g_cell)`.
    //
    // On an 8x16 cell `g_cell` is 8 and this changes nothing — 8 · 1.5 = 12 and
    // 16 · 1.5 = 24 are both whole, which is why the half rungs have always been
    // fine everywhere else and why nobody noticed. On the Macintosh's **7x15**
    // cell `gcd(7, 15)` is **1**, so a half rung gives a 7-wide glyph 10.5 device
    // pixels: its strokes come out alternating one and two, `l` and `i` go ragged,
    // and the compass rose in the same frame stays perfectly crisp. That contrast
    // inside one image is the signature, and it is why this is arithmetic about
    // the CELL rather than a per-machine exception (SQ-1012).
    let g = geom.step() as f32;
    // Count whole steps in units of the step itself (`free · g`), so the floor is
    // taken on an integer-valued quantity and a 2.9999996 that should be 3 does not
    // drop a whole rung. `1e-4` is far below one step and far above f32's error at
    // these magnitudes.
    let steps = (free * g + 1e-4).floor();
    if steps < 1.0 {
        return None;
    }
    Some(centred(geom.native, pane_dev, steps / g))
}

/// The story window's clear-interior rect in NATIVE game pixels: its native rect
/// inset (interleaved per-edge) until no edge overlaps an opaque chrome pixel.
/// `None` when there is no story window. May be zero-size if fully occluded.
///
/// Inset one native pixel at a time per edge, banner first then columns, but
/// *interleaved* round by round (rather than each edge run to completion before
/// the next starts): a story window can overlap chrome on both axes at once
/// (e.g. a banner AND side columns), and letting the top/bottom scan run to
/// completion against the still-full width would never see a "clear" row while
/// side-band columns persist down the whole height. Shrinking left/right a step
/// at a time alongside top/bottom lets each edge's scan range narrow in
/// lockstep, converging on the true clear interior.
pub fn story_clear_native(
    story: Option<&PositionedWindow>,
    chrome_canvas: &RgbaImage,
) -> Option<(u32, u32, u32, u32)> {
    let story = story?;
    let (cw, ch) = chrome_canvas.dimensions();
    let opaque = |x: u32, y: u32| -> bool { x < cw && y < ch && chrome_canvas.get_pixel(x, y)[3] >= 128 };
    let mut left = story.x_px as u32;
    let mut top = story.y_px as u32;
    let mut right = (story.x_px as u32 + story.w_px as u32).min(cw);
    let mut bottom = (story.y_px as u32 + story.h_px as u32).min(ch);
    loop {
        let mut changed = false;
        if top < bottom && (left..right).any(|x| opaque(x, top)) {
            top += 1;
            changed = true;
        }
        if bottom > top && (left..right).any(|x| opaque(x, bottom - 1)) {
            bottom -= 1;
            changed = true;
        }
        if left < right && (top..bottom).any(|y| opaque(left, y)) {
            left += 1;
            changed = true;
        }
        if right > left && (top..bottom).any(|y| opaque(right - 1, y)) {
            right -= 1;
            changed = true;
        }
        if !changed {
            break;
        }
    }
    Some((left, top, right.saturating_sub(left), bottom.saturating_sub(top)))
}

// `story_viewport` — the cell-space shrink-until-clear-then-quantize wrapper — was
// DELETED by SQ-0894, on the instruction SQ-0893 left when it kept it: "It is
// retained because it is exactly the shrink-until-clear-then-quantize step that
// SQ-0894 needs… If SQ-0894 lands without adopting it, delete it then."
//
// SQ-0894 measured adopting it and it is a NO-OP. Driven over the v6 corpus —
// Zork Zero, Arthur, Shogun, Journey, mysterious01, fmvpoker, scopa and advent —
// at boot and through six further turns each, at a 98x37 pane against the ART-ONLY
// canvas (the oracle §3(b) says it needs; against the full chrome canvas, which
// carries rasterised text as opaque pixels, Shogun's declared 548x64 comes back
// 548x16), the clear region equalled the declared window-0 box on EVERY frame.
// These games place window 0 to fit their own frame and do not draw art into it;
// a margin picture goes to the transcript as a float (SQ-0888), not onto the canvas.
//
// That is a property of the CORPUS, not of the function: its own unit tests proved
// it insets correctly on a synthetic canvas whose bands overlap the story window.
// `story_clear_native` above — the NATIVE-space sibling, which raster really does
// call — is untouched. Reinstate this wrapper from the commit that removed it if
// hybrid ever needs the cell-space answer.

/// The NATIVE rect hybrid cuts its story viewport out of — the story window
/// reduced to what the ART leaves it (SQ-0896).
///
/// This is step (b) of the user's ordering — *"determine the valid text region from
/// what the panels leave"* — for the HYBRID path, and it is raster's own two-step
/// composition rather than a new rule:
///
/// 1. [`story_clear_native`] insets the window edge by edge past the FRAME art. Its
///    oracle is `frame_art`, which must be the art-only chrome canvas
///    ([`build_graphics_canvas`]) and not the full chrome canvas — rasterised glyphs
///    are opaque too, and measured against them Shogun's declared 548x64 box comes
///    back 548x16 (SQ-0728).
/// 2. [`story_prose_box`] then takes the largest rectangle of what is left that the
///    story window's OWN plate painted no pixel of. Edge insetting cannot do this
///    job: a centred plate touches no edge, and fmvpoker's hollow 640x400 table
///    touches all four and insets to width 0 — MEASURED, `clear (320,54,0,322)`.
///
/// The inset is ADVISORY and the plate is AUTHORITATIVE, and the floor between them
/// is raster's own (`w >= FONT_W && h >= FONT_H`, one full text cell — `screen.rs`
/// applies it to the same call). An inset that leaves less than one cell is not a
/// measurement of the text region: it is the frame art being a BACKDROP rather than
/// a border, and the four edges converging on nothing from all sides at once.
/// MEASURED on `hybrid_v6_model` — a 320x200 fully opaque chrome window with window
/// 0 at (43,39,234,160) — where the interleaved inset runs to width 0 and would take
/// a story region hybrid has drawn prose into since Lane H. Such a window keeps its
/// declared box, exactly as it does today.
///
/// `None` means the PLATE owns the screen and no prose belongs on this frame at all
/// (SQ-0707: the game erases, draws and waits on a `read_char`, so the narration is
/// its own picture-less screen). For the ring that means the whole pane is chrome:
/// there is no viewport to carve around, so `pane − viewport` is the pane.
///
/// Why this is the capability hybrid was missing: `classify_windows` sets a `win == 0`
/// Graphics aside as `story_gfx` precisely so the chrome ring does NOT carry it, and
/// hybrid then opened its transcript viewport over the raw window box — straight over
/// the plate, which no band covered and no viewport could show. Deriving the viewport
/// from what the art leaves puts that art OUTSIDE the viewport, and everything outside
/// the viewport is already the ring's, drawn by machinery that has worked since
/// SQ-0505. Nothing new draws pixels; the ring simply gets given the ones it was
/// structurally denied.
pub fn story_text_native(
    story: Option<&PositionedWindow>,
    frame_art: &RgbaImage,
    story_gfx: Option<&PositionedWindow>,
    cell: V6Cell,
) -> Option<(u32, u32, u32, u32)> {
    let font_w = u32::from(cell.w());
    let font_h = u32::from(cell.h());
    let s = story?;
    let declared = (s.x_px as u32, s.y_px as u32, s.w_px as u32, s.h_px as u32);
    let inset = story_clear_native(story, frame_art)
        .filter(|&(_, _, w, h)| w >= font_w && h >= font_h)
        .unwrap_or(declared);
    story_prose_box(inset, story_gfx, cell)
}

/// The pane COLUMNS the game's screen covers — `[off_x, off_x + native_w · s)` in
/// device pixels, quantized OUTWARD to whole cells and clamped to `pane` (SQ-0946).
///
/// The letterbox margin beside it belongs to nobody: the ring's art bands are
/// transparent there by construction, and a cell path that paints the GAME's own
/// ground into it puts the frame off-centre by however wide the margin is. Journey's
/// IBM PC press (`journey-r83-s890706.z6`, release 83) is the report — its left
/// picture panel and its bottom command strip are both cell fills, and both ran to
/// the pane edge. At a 98x37 pane with `v6_pixel_lock` on (`s = 1`, `off_x = 72`) the
/// screen covers columns 9..89, and the panel was flooding 0..38: nine columns of
/// game-coloured ground down the left of a pane whose right margin was bare, which is
/// exactly the "not centred horizontally" the user sees. The ART itself was centred
/// throughout — measured symmetric to the cell at every pane width from 80 to 160.
///
/// OUTWARD, not inward, and that is `screen::edge_glyph_col`'s rounding rather than
/// [`native_viewport_box`]'s: an edge glyph is stamped in the column its
/// native cell's outer edge falls in, so the ground beneath it has to be there. The
/// viewport rounds the other way because it is carving a region out, not filling one.
///
/// Zero-cost where the art already fills the pane (`off_x == 0` and a width-bound
/// fit), which is every frame the lock is off and the pane is wide — this only ever
/// removes ink from a margin the game's screen does not reach.
pub fn screen_cols(
    scale: &Scale,
    native_w: u16,
    cell_px: (u16, u16),
    pane: ratatui::layout::Rect,
) -> (u16, u16) {
    let cw = if cell_px.0 == 0 { 1 } else { cell_px.0 } as f32;
    let dev0 = pane.x as f32 * cw + scale.off_x as f32;
    let dev1 = dev0 + native_w as f32 * scale.s;
    let lo = (dev0 / cw).floor().clamp(pane.x as f32, pane.right() as f32) as u16;
    let hi = (dev1 / cw).ceil().clamp(lo as f32, pane.right() as f32) as u16;
    (lo, hi)
}

/// The story viewport cell rect (relative to the pane's top-left cell) for the
/// HYBRID render mode: the win0 box (`story` x_px/y_px/w_px/h_px, native game
/// pixels) mapped through the letterbox [`Scale`] to device pixels, then quantized
/// to whole cells rounding INWARD (ceil the top-left, floor the bottom-right) so
/// no surrounding chrome cell overlaps the terminal story region. This does NOT
/// inset around opaque chrome pixels — see [`story_text_native`], which is what the
/// ring feeds [`native_viewport_box`] with. Falls back to the full pane when there
/// is no story window.
pub fn story_viewport_box(
    story: Option<&PositionedWindow>,
    scale: &Scale,
    pane_cells: (u16, u16),
    cell_px: (u16, u16),
) -> ratatui::layout::Rect {
    native_viewport_box(
        story.map(|s| (s.x_px as u32, s.y_px as u32, s.w_px as u32, s.h_px as u32)),
        scale,
        pane_cells,
        cell_px,
    )
}

/// [`story_viewport_box`] for a native rect that is not a window's own box — the
/// text region [`story_text_native`] derived from what the art leaves (SQ-0896).
///
/// Quantizes exactly as the window-box form does, because it IS the same step: the
/// only thing that changed is which native rectangle the terminal viewport is cut
/// from. `None` falls back to the full pane, matching "no story window".
pub fn native_viewport_box(
    rect: Option<(u32, u32, u32, u32)>,
    scale: &Scale,
    pane_cells: (u16, u16),
    cell_px: (u16, u16),
) -> ratatui::layout::Rect {
    let Some((rx, ry, rw, rh)) = rect else {
        return ratatui::layout::Rect { x: 0, y: 0, width: pane_cells.0, height: pane_cells.1 };
    };
    let left = rx as f32;
    let top = ry as f32;
    let right = (rx + rw) as f32;
    let bottom = (ry + rh) as f32;

    let dev_left = scale.off_x as f32 + left * scale.s;
    let dev_top = scale.off_y as f32 + top * scale.s;
    let dev_right = scale.off_x as f32 + right * scale.s;
    let dev_bottom = scale.off_y as f32 + bottom * scale.s;

    let cw_px = if cell_px.0 == 0 { 1 } else { cell_px.0 } as f32;
    let ch_px = if cell_px.1 == 0 { 1 } else { cell_px.1 } as f32;

    // Round INWARD: ceil the top-left, floor the bottom-right, so the viewport is
    // the largest whole-cell rect fully inside the win0 box.
    let cell_left = (dev_left / cw_px).ceil() as u16;
    let cell_top = (dev_top / ch_px).ceil() as u16;
    let cell_right = (dev_right / cw_px).floor() as u16;
    let cell_bottom = (dev_bottom / ch_px).floor() as u16;

    let width = cell_right.saturating_sub(cell_left).max(1);
    let height = cell_bottom.saturating_sub(cell_top).max(1);

    let cell_left = cell_left.min(pane_cells.0.saturating_sub(1));
    let cell_top = cell_top.min(pane_cells.1.saturating_sub(1));
    let width = width.min(pane_cells.0.saturating_sub(cell_left));
    let height = height.min(pane_cells.1.saturating_sub(cell_top));

    ratatui::layout::Rect { x: cell_left, y: cell_top, width, height }
}

/// Which side of the ring a chrome band is — its IDENTITY, carried alongside its
/// rect (SQ-0894).
///
/// Before this existed, downstream stages recovered the answer by measuring:
/// `band.width < pane.width` meant "a flank" in `decompose_chrome_strips` and in
/// the Extend clip, and `width == pane.width && y == viewport.bottom()` meant "the
/// menu band". Those tests are only true while the top and bottom bands span the
/// full pane width, which is the very definition SQ-0894 is replacing — and a
/// width test that silently becomes wrong is how Shogun's eight-run header and
/// Arthur's seventy-two-run status bar would have rasterised, in violation of the
/// SQ-0750 rule that hybrid never rasterises what the game printed as a character.
///
/// A band knows what it is. Nothing downstream measures to find out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BandRole {
    /// Above the story viewport.
    Top,
    /// Below the story viewport — under the `Menu` plan this is the game's own
    /// command strip.
    Bottom,
    /// Left of the story viewport.
    LeftFlank,
    /// Right of the story viewport.
    RightFlank,
}

impl BandRole {
    /// A flank carries the frame's side art: one continuous column, tiled or
    /// extended, never split row-by-row into text.
    pub fn is_flank(self) -> bool {
        matches!(self, BandRole::LeftFlank | BandRole::RightFlank)
    }
}

/// The chrome RING cell rects around a story `viewport` inside a `pane`: up to
/// four non-overlapping rects (top, bottom, left, right) that exactly tile
/// `pane − viewport`, each tagged with its [`BandRole`]. The top and bottom bands
/// span the pane's full width (and so own the corners); the left and right bands
/// span only the viewport's vertical extent. An edge-flush viewport omits that
/// side's band; `viewport == pane` yields an empty list. `viewport` is assumed to
/// lie within `pane`; it is clamped defensively. Both rects share one coordinate
/// space (both absolute, or both pane-relative).
pub fn chrome_bands(
    pane: ratatui::layout::Rect,
    viewport: ratatui::layout::Rect,
) -> Vec<(BandRole, ratatui::layout::Rect)> {
    use ratatui::layout::Rect;
    // Clamp the viewport within the pane so the band arithmetic can't underflow.
    let vx = viewport.x.clamp(pane.x, pane.right());
    let vy = viewport.y.clamp(pane.y, pane.bottom());
    let vr = viewport.right().clamp(vx, pane.right());
    let vb = viewport.bottom().clamp(vy, pane.bottom());

    let mut out = vec![
        // Top band: full pane width, from the pane top down to the viewport top.
        (BandRole::Top, Rect::new(pane.x, pane.y, pane.width, vy - pane.y)),
        // Bottom band: full pane width, from the viewport bottom to the pane bottom.
        (BandRole::Bottom, Rect::new(pane.x, vb, pane.width, pane.bottom() - vb)),
        // Left band: the viewport's vertical span, from the pane left to the viewport left.
        (BandRole::LeftFlank, Rect::new(pane.x, vy, vx - pane.x, vb - vy)),
        // Right band: the viewport's vertical span, from the viewport right to the pane right.
        (BandRole::RightFlank, Rect::new(vr, vy, pane.right() - vr, vb - vy)),
    ];
    out.retain(|(_, r)| r.width > 0 && r.height > 0);
    out
}

/// Rasterize `main`'s wrapped lines (then the input line + block cursor when
/// `main.awaiting`) into `canvas` starting at native px `(ox, oy)`, one glyph per
/// FONT×FONT cell, transparent glyph bg (draws over chrome/background art).
/// Clipped to `rows` lines and `cols` columns.
///
/// `spare` is the native-pixel rects another window's own text already claimed
/// inside this box — [`chrome_text_rects`] — and no story glyph is drawn in a cell
/// that meets one (SQ-0729). The transcript is the HOST's re-render of everything
/// window 0 ever printed; a label another window has on the screen right now is
/// live, so where they collide the live one wins. Without it fmvpoker's dealt hand
/// wrote its boot banner straight through "You draw (a) an Eight, (b) a Three, …",
/// the line the player needs in order to see their draw: once the five cards fill
/// the frame's interior, window 0's clear rectangle moves DOWN onto the box the
/// game gave its bottom prose window, and both wanted the same rows.
/// [`fill_story_page_under_chrome_text`] already spared those pixels from the page
/// FILL; nothing spared them from the GLYPHS. Pass `&[]` for no sparing.
///
/// `reveal` is the momentary word reveal, when one is lit (SQ-1107 / SQ-1138), and
/// `None` otherwise. The cell path re-styles its drawn cells afterwards; a canvas
/// has no cells to re-style, so the light is applied HERE, as each glyph is
/// blitted — which is also why it is a parameter of the draw rather than a field
/// of [`MainText`]. The reveal is a property of the moment, not of the text: the
/// wrapped rows are the game's own output and get persisted in the archive, and a
/// decoration folded into them would have to be taken back out again.
pub fn draw_story_text(canvas: &mut RgbaImage, main: &MainText, ox: u32, oy: u32, cols: u16, rows: u16, fg: Rgba<u8>, spare: &[(u32, u32, u32, u32)], tf: &crate::native_font::TextFace, reveal: Option<&crate::reveal::RasterReveal<'_>>) {
    let cell = tf.cell();
    let font_w = u32::from(cell.w());
    let font_h = u32::from(cell.h());
    let region_h = rows as u32 * font_h;
    // The reveal's underline, in the geometry SQ-1028 gives an emphasised run:
    // ONE MASTER ROW at the bottom of the cell — two native rows on a 16-row cell,
    // one on the Macintosh's fifteen. Derived from `font_h`, the declared TEXT
    // cell, and so from the same space every other number in this function is
    // stated in; a thickness resolved from `art_scale` instead would be double on
    // the presses where one art pixel is two native pixels and correct nowhere
    // else (SQ-0917 / SQ-1039).
    let rule_h = (font_h / 8).max(1);
    // A cell is spared when any pixel of it belongs to a chrome text run.
    let blocked = |px: u32, py: u32| -> bool {
        spare.iter().any(|&(x0, y0, x1, y1)| px < x1 && x0 < px + font_w && py < y1 && y0 < py + font_h)
    };
    // Floats first (text draws over/beside them). A float that has partially
    // scrolled off the top (row < 0) is drawn cropped from its own top. Blitted
    // at `img_col` (0 = left float; near the right edge = right float), clamped
    // to the columns from there to the region's right edge.
    for f in &main.floats {
        let src = &*f.img;
        let crop_top = if f.row < 0 { (-f.row) as u32 * font_h } else { 0 };
        if crop_top >= src.height() {
            continue;
        }
        let dy = oy + (f.row.max(0) as u32) * font_h;
        let max_h = region_h.saturating_sub(dy - oy);
        let img_x = ox + f.img_col as u32 * font_w;
        let max_w = (cols as u32).saturating_sub(f.img_col as u32) * font_w;
        blit_clipped_src(canvas, src, img_x, dy, crop_top, max_w, max_h);
    }
    // The active float's (reserved cols, text start col) for a given row — one
    // float is active at a time; when several overlap take the widest reserve.
    let float_at = |row: u32| -> (u32, u32) {
        main.floats
            .iter()
            .filter(|f| f.row <= row as i32 && (row as i32) < f.row + f.rows as i32)
            .map(|f| (f.reserve_cols as u32, f.text_col as u32))
            .max_by_key(|(reserve, _)| *reserve)
            .unwrap_or((0, 0))
    };
    let mut row = 0u32;
    // Where the pen finished on the last drawn line, in native px from `ox` — the
    // column count this used to be cannot express a proportional row (SQ-1009).
    let mut last_row_px = 0u32;
    for line in &main.lines {
        if row >= rows as u32 {
            return;
        }
        let (reserve, text_col) = float_at(row);
        let avail = (cols as u32).saturating_sub(reserve);
        // Per-char emphasis for this row (SQ-0540): the raster font synthesizes
        // bold/italic, so a game's emphasised prose (Zork Zero's bold room
        // names, Shogun's italic "Erasmus") reads as emphasis here too. A row
        // with no `styles` entry — or a char past its end — is roman.
        let row_styles = main.styles.get(row as usize);
        // The pen starts at the float's text column and advances per glyph
        // (SQ-1009), and the row is CLIPPED BY PIXELS — the same budget
        // `build_main_text` wrapped it to.
        //
        // This used to be `line.chars().take(avail)`, a CHARACTER cap, on the
        // reasoning that the wrap had already fitted the row so the cap could only
        // ever catch a row nothing wrapped. That holds exactly while one character
        // costs one column. It stops holding the moment the pen is proportional: the
        // wrap packs whatever FITS in `avail * font_w` pixels, and Geneva at about
        // 6.4px against a declared 7 fits SEVENTY-TWO characters into a 66-column
        // box — so the cap threw the last six away and the line lost its tail with
        // the space still visibly there. Measured on Zork Zero's Banquet Hall off
        // `stories/Zork Zero Disk.image`, raster, where `…the thousands of reveling`
        // was drawn as `…the thousands of reveli` and `to the west` as `to th`
        // (SQ-1051).
        //
        // A pixel cap keeps the guarantee the character cap was there for — a row
        // that was never wrapped still cannot run past its box — without discarding
        // glyphs the wrap correctly fitted.

        // While a reveal is lit, where on THIS row the story printed each of its
        // own things (SQ-1138, SQ-1207). Char ranges into `line`, which
        // is exactly what `line.chars().enumerate()` below counts in — the same
        // `lit_spans` the cell path calls on the same wrapped row, so the two
        // surfaces cannot disagree about which words light.
        let lit = reveal.map(|r| crate::reveal::lit_spans(line, r.words)).unwrap_or_default();
        let mut pen = ox + text_col * font_w;
        let row_limit = pen + avail * font_w;
        let py = oy + row * font_h;
        for (col, glyph) in line.chars().enumerate() {
            let style = row_styles.and_then(|s| s.get(col)).copied().unwrap_or(0);
            let adv = tf.advance_styled(glyph, style);
            if pen + adv > row_limit {
                break;
            }
            let hit = reveal.filter(|_| lit.iter().any(|&(s, e)| col >= s && col < e));
            if !blocked(pen, py) {
                crate::render::bitfont::blit_glyph_styled(canvas, glyph, pen, py, font_w, font_h, hit.map_or(fg, |r| r.ink), None, style, Some(tf));
                // …and the rule under it, AFTER the glyph so it reads as one line
                // rather than as a row the descenders punch holes in — the same
                // order `blit_metric_glyph` draws SQ-1028's in. Spanning the whole
                // ADVANCE, so consecutive lit glyphs join and the player sees one
                // underlined word instead of seven underlined letters.
                if let Some(r) = hit.filter(|r| r.rule) {
                    fill_cell(canvas, pen, py + font_h - rule_h, adv, rule_h, r.ink);
                }
            }
            pen += adv;
        }
        last_row_px = pen - ox;
        row += 1;
    }
    if main.awaiting {
        // The live input continues the game's kept prompt line (the last drawn
        // row — Zork Zero's "…HINT): >"), NOT a fresh row below it (SQ-0470a):
        // the caret sits right after the prompt. When the transcript ended on a
        // newline the last line is empty (`last_row_end == 0`) so the input
        // starts a clean row of its own, matching the terminal inline prompt.
        let input_row = row.saturating_sub(1);
        let right = cols as u32 * font_w;
        if input_row < rows as u32 {
            let py = oy + input_row * font_h;
            let mut pen = last_row_px;
            for glyph in main.input.chars() {
                let adv = tf.advance(glyph);
                if pen + adv > right {
                    break;
                }
                if !blocked(ox + pen, py) {
                    crate::render::bitfont::blit_glyph(canvas, glyph, ox + pen, py, font_w, font_h, fg, None, Some(tf));
                }
                pen += adv;
            }
            // The caret sits where the pen would be after `cursor_col` characters
            // of the input, which is the same cell the column arithmetic gave for
            // every fixed-pitch face.
            let caret = (last_row_px
                + main.input.chars().take(main.cursor_col as usize).map(|c| tf.advance(c)).sum::<u32>())
            .min(right.saturating_sub(font_w));
            if !blocked(ox + caret, py) {
                fill_cell(canvas, ox + caret, py, font_w, font_h, fg);
            }
        }
    }
}

#[cfg(all(test, feature = "t-render"))]
mod tests {

    // ── the text layer the ring claims (SQ-0902 → SQ-0903) ───────────────────

    /// Shogun's measured geometry, on `stories/shogun-r322-s890706.z6` two turns in:
    /// the frame art ends at native x **45**, the status window begins at native x
    /// **46**, and at a 129x60 pane the strip's first whole cell inverts to native
    /// **49** — so `clear_text_rows`, whose span is that cell rect, left native 46..48
    /// carrying the rasterised cell backgrounds `build_chrome_canvas` paints for the
    /// status window. The flank band's source reaches native 50 at that pane, sampled
    /// them, and drew a sliver of the status line inside the frame.
    ///
    /// SQ-0903, porting SQ-0902's evidence to the mechanism that replaced it.
    ///
    /// On a row the ring draws with GLYPHS the canvas keeps artwork and nothing
    /// else (SQ-0750). That used to be enforced by rasterising the status window
    /// and carving it back out; it is enforced now by never painting the row.
    ///
    /// **And the subtlety the carve existed for disappears with it.** Shogun's
    /// frame art ends at native 45 and its status window begins at 46, three
    /// columns inside the flank's last terminal cell — so a carve scoped to the
    /// strip's own cell rect left them rasterised and the flank's band sampled a
    /// sliver of the status line. There are no columns here at all: a row is
    /// painted or it is not, and a boundary inside a cell has nothing to fall on.
    #[test]
    fn a_glyph_row_is_never_painted_and_the_art_under_it_survives() {
        const ART_END: u32 = 46; // exclusive — the frame slab occupies 0..45
        let native = (640u16, 400u16);
        let art_win = PositionedWindow {
            x: 0, y: 0, w: 6, h: 25, x_px: 0, y_px: 0, w_px: ART_END as u16, h_px: 400,
            left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(GraphicsWindow {
                win: 7,
                canvas: {
                    let mut c = RgbaImage::new(ART_END, 400);
                    for px in c.pixels_mut() {
                        *px = Rgba([9, 9, 9, 255]);
                    }
                    Arc::new(c)
                },
                version: 0,
                upscale: false,
            }),
        };
        // A status window rasterising two text rows from its own left edge to the
        // screen's right — Shogun's is 32px, and only the FIRST is a glyph row.
        let bar = |y: u16| {
            crate::engine::PxText::derived(
                y,
                ART_END as u16 + 1,
                "X".repeat(74),
                0,
                0x0300_0000,
                0x03FF_FFFF,
                zvm::screen::V6Cell::DEFAULT,
            )
        };
        let status = PositionedWindow {
            x: 0, y: 0, w: 74, h: 2, x_px: ART_END as u16, y_px: 0, w_px: 592, h_px: 32,
            left_margin: 0, right_margin: 0,
            node: WinNode::Grid(GridWindow {
                win: 0,
                fill: None,
                cols: 74, rows: 2, cells: vec![], active_rows: 2, cursor: (0, 0),
                cursor_active: false, border: BorderPref::Unspecified,
                bg: None, fg: None, reverse: false,
                px_texts: vec![bar(1), bar(FONT_H as u16 + 1)],
            }),
        };
        let chrome: Vec<&PositionedWindow> = vec![&art_win, &status];
        let skip: std::collections::HashSet<u16> = [0u16].into_iter().collect();
        let canvas = build_chrome_canvas(
            &chrome, native, Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255]),
            &colors(), TextLayer::SkipGlyphRows(&skip),
            &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT),
        );
        let op = |x: u32, y: u32| canvas.get_pixel(x, y)[3] >= 128;

        assert!(
            (ART_END..native.0 as u32).all(|x| !op(x, 4)),
            "the glyph row was never painted — including native {ART_END}..49, the \
             columns a cell-rect carve could not reach",
        );
        assert!(
            (0..ART_END).all(|x| op(x, 4)),
            "and Pass 1's artwork is untouched: a glyph row may still carry art",
        );
        assert!(
            (ART_END..native.0 as u32).any(|x| op(x, FONT_H + 4)),
            "the row BELOW it is not a glyph row and is imaged as always",
        );
    }

    /// A grid whose runs the ring took ALL of paints NOTHING — it does not fall
    /// back to its cell grid and redraw the same characters (SQ-0944).
    ///
    /// The `continue` used to be gated on the runs that SURVIVED the skip, so a
    /// grid that lost every run fell through to the cell-grid painter below,
    /// which places a row at `oy + row * FONT_H`. A skip set keyed on the runs'
    /// own tops can never match those, so the text came back — cell-quantised,
    /// a text row off where the ring's glyphs land.
    ///
    /// Zork Zero's banner is the frame that showed it: runs at native 10 and 26,
    /// both handed to the ring under half-blocks, both painted straight back in
    /// at 0 and 16, so a crisp "Banquet Hall" arrived with a blurred copy of
    /// itself sitting one row above. The rows here are the same shape — 10 is
    /// not a multiple of `FONT_H`, which is the whole reason the two painters
    /// disagree about where the row is.
    #[test]
    fn a_grid_whose_runs_are_all_skipped_does_not_fall_back_to_its_cell_grid() {
        let native = (640u16, 400u16);
        let cell = |ch: char| GridCell { ch, style: 0, fg: 0, bg: 0, link: 0, glk_style: 0 };
        let status = PositionedWindow {
            x: 0, y: 0, w: 12, h: 2, x_px: 0, y_px: 0, w_px: 640, h_px: 32,
            left_margin: 0, right_margin: 0,
            node: WinNode::Grid(GridWindow {
                win: 0,
                fill: None,
                cols: 12, rows: 2, active_rows: 2, cursor: (0, 0),
                cursor_active: false, border: BorderPref::Unspecified,
                bg: None, fg: None, reverse: false,
                // The SAME text in both representations, as a v6 status window
                // carries it: the runs say where it really is, the cell grid says
                // where it would be if rows were 16 pixels apart.
                cells: "Banquet Hall".chars().chain("Moves:     1".chars()).map(cell).collect(),
                px_texts: vec![
                    crate::engine::PxText::derived(11, 1, "Banquet Hall".into(), 0, 0x0300_0000, 0, zvm::screen::V6Cell::DEFAULT),
                    crate::engine::PxText::derived(27, 1, "Moves:     1".into(), 0, 0x0300_0000, 0, zvm::screen::V6Cell::DEFAULT),
                ],
            }),
        };
        let chrome: Vec<&PositionedWindow> = vec![&status];
        let skip: std::collections::HashSet<u16> = [10u16, 26].into_iter().collect();
        let canvas = build_chrome_canvas(
            &chrome, native, Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255]),
            &colors(), TextLayer::SkipGlyphRows(&skip),
            &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT),
        );

        let painted = canvas.pixels().filter(|p| p[3] > 0).count();
        assert_eq!(
            painted, 0,
            "every run was skipped, so this grid owes the canvas no pixels at all — \
             {painted} were painted, which is the cell grid drawing the banner again",
        );

        // …and the skip is still the only reason. Ask for the same canvas with
        // nothing skipped: the runs appear, and they appear at the RUNS' rows.
        // Native 32..42 is the discriminating span — inside run 2 (26..42) and
        // past the cell grid's second and last row (16..32) — so ink there can
        // only have come from the run painter.
        let all = build_chrome_canvas(
            &chrome, native, Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255]),
            &colors(), TextLayer::All,
            &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT),
        );
        assert!(
            (32..42).any(|y| (0..native.0 as u32).any(|x| all.get_pixel(x, y)[3] > 0)),
            "the runs really are paintable, and only the run painter reaches native 32..42 — \
             so the empty canvas above is the skip doing its job, not an inert fixture",
        );
    }

    /// Artwork the game DREW survives, and it survives for a structural reason
    /// rather than by being spared (SQ-0902 → SQ-0903).
    ///
    /// `build_graphics_canvas` is `blit_chrome_graphics` and nothing else, so it
    /// knows only about art published as a `Graphics` window's display list.
    /// `stories/scopa.z6` has NO Graphics window at all — its start page draws
    /// three cards with `draw_picture`, which reaches the ring as the paint
    /// surface (SQ-0706) — so its art canvas is empty while its paint surface
    /// carries 28,296 opaque pixels, 8,256 of them on rows carrying chrome text.
    /// The old carve had to be told about the paint surface explicitly or it
    /// erased the cards, which is the regression the user caught within the hour.
    ///
    /// **Nothing has to be told anything now.** Skipping removes only pixels
    /// `build_chrome_canvas` would have painted, and `blit_paint_ground` runs
    /// afterwards and is never skipped — so a painting game's artwork is not
    /// spared from an erasure, it is simply never at risk.
    #[test]
    fn a_painting_games_artwork_is_not_at_risk_because_it_lands_after_the_skip() {
        let native = (640u16, 400u16);
        let bar = crate::engine::PxText::derived(1, 1, "X".repeat(80), 0, 0x0300_0000, 0x03FF_FFFF, zvm::screen::V6Cell::DEFAULT);
        let status = PositionedWindow {
            x: 0, y: 0, w: 80, h: 1, x_px: 0, y_px: 0, w_px: 640, h_px: 16,
            left_margin: 0, right_margin: 0,
            node: WinNode::Grid(GridWindow {
                win: 0,
                fill: None,
                cols: 80, rows: 1, cells: vec![], active_rows: 1, cursor: (0, 0),
                cursor_active: false, border: BorderPref::Unspecified,
                bg: None, fg: None, reverse: false,
                px_texts: vec![bar],
            }),
        };
        // A painted card overlapping that row — scopa's shape, no Graphics window.
        let mut paint = RgbaImage::new(native.0 as u32, native.1 as u32);
        for y in 0..40 {
            for x in 100..180 {
                paint.put_pixel(x, y, Rgba([200, 30, 30, 255]));
            }
        }
        let chrome: Vec<&PositionedWindow> = vec![&status];
        let skip: std::collections::HashSet<u16> = [0u16].into_iter().collect();
        let mut canvas = build_chrome_canvas(
            &chrome, native, Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255]),
            &colors(), TextLayer::SkipGlyphRows(&skip),
            &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT),
        );
        // The ring's own order: the painted ground goes on after the chrome text
        // (SQ-0706), which is exactly why the skip cannot reach it.
        blit_paint_ground(&mut canvas, Some(&paint), TextLayer::All, zvm::screen::V6Cell::DEFAULT);
        let op = |x: u32, y: u32| canvas.get_pixel(x, y)[3] >= 128;

        assert!(
            (100..180).all(|x| op(x, 4)),
            "the painted card is on the canvas, on a row that also carries chrome text",
        );
        assert!(
            (0..100).all(|x| !op(x, 4)) && (180..native.0 as u32).all(|x| !op(x, 4)),
            "and the status cells beside it were never painted",
        );
        // The art canvas is empty here, as scopa's is — and it no longer matters.
        assert!(
            build_graphics_canvas(&chrome, native).pixels().all(|p| p[3] == 0),
            "no Graphics window: the oracle the old carve depended on sees nothing",
        );
    }

    use super::*;
    use crate::engine::{BorderPref, BufferWindow, GraphicsWindow, GridCell, GridWindow, PxText};
    use std::sync::Arc;

    /// **The unit screen grows by the PEN, not the declared cell** (SQ-1066).
    ///
    /// `native_extent`'s answer is the screen every downstream stage divides by, and
    /// it grew a grid run by `chars * cell.w` — `V6Cell::run_px` written longhand —
    /// while `build_chrome_canvas` draws that same run by stepping
    /// `advance_styled`. So the frame was scaled and letterboxed against a screen
    /// the machine never declared: the Macintosh Zork Zero hint frame measured
    /// **658x400** where the machine says 640x400, and the whole picture shrank and
    /// sat off-centre. Not a clip — no ink is lost — which is why it read as
    /// nothing in particular. SQ-0901 is the same shape.
    ///
    /// Measured after the fix, across the presses that have a face: Macintosh Zork
    /// Zero's hint frame 640x400 (was 658x400), Arthur's Amiga floppy 640x400
    /// unchanged, Shogun's Amiga floppy 640x400 unchanged. Every fixed pen answers
    /// the declared cell for every style, so no other press moves by a pixel — which
    /// is also why the ninety-odd suites pinning `(640, 400)` through a
    /// `cell_only` face needed no re-measuring.
    ///
    /// FALSIFY by restoring `n * font_w`: the extent comes back as the declared
    /// answer, which this case computes alongside so the two are named together.
    #[test]
    fn native_extent_grows_a_grid_run_by_the_pen_not_the_declared_cell() {
        let profile = crate::interpreter::InterpreterProfile::Macintosh; // 7x15
        let glyph = |w: u8| blorb::bitmap_font::Glyph {
            width: w,
            rows: (0..15).map(|r| if r == 12 { 0xFF } else { 0x00 }).collect(),
        };
        let font = blorb::bitmap_font::BitmapFont {
            width: 11,
            height: 15,
            baseline: 12,
            bold_smear: 0,
            proportional: true,
            lo: b' ',
            // 7..11 against a declared 7, so the pen reaches FURTHER than the cell.
            glyphs: (b'\x20'..=b'\x7e').map(|c| glyph(7 + (c % 5))).collect(),
        };
        let tf = crate::native_font::TextFace::new(
            profile,
            crate::native_font::FaceSet::release(font, profile, Some((1, 1))),
            Some((1, 1)),
        );
        const TOPIC: &str = "GENERAL QUESTIONS";
        let pen = tf.run_px_styled(TOPIC, 0);
        let declared = u32::from(tf.cell().w()) * TOPIC.chars().count() as u32;
        assert!(
            pen != declared,
            "non-vacuity: the two measures must differ ({pen} vs {declared})",
        );

        // A window of no width, so the RUN is what the extent has to grow for.
        let mut grid = grid_item(0);
        grid.w_px = 1;
        grid.h_px = 1;
        match &mut grid.node {
            WinNode::Grid(g) => {
                g.px_texts = vec![PxText {
                    y: 1,
                    x: 1,
                    text: TOPIC.to_string(),
                    style: 0,
                    fg: 0,
                    bg: 0,
                    grow: 0,
                    gcol: 0,
                }]
            }
            _ => unreachable!(),
        }
        assert_eq!(
            native_extent(&[&grid].map(|g| g.clone()), &tf).0,
            pen as u16,
            "the screen reaches as far as the run is DRAWN",
        );
        assert_eq!(
            native_extent(
                &[&grid].map(|g| g.clone()),
                &crate::native_font::TextFace::cell_only(tf.cell()),
            )
            .0,
            declared as u16,
            "…and the declared measure is the other answer, named here so the two cannot \
             be confused for one",
        );
    }

    /// **A chrome run's spare rect is the PEN's span, not the declared one**
    /// (SQ-1054).
    ///
    /// `chrome_text_rects` names the pixels `fill_story_page_under_chrome_text`
    /// must not paint over, and the glyph loop that put them there steps
    /// `advance_styled`. Measured as `chars * cell.w` instead, the rect is the box
    /// the GAME reserved — and where the face is wider than the cell the page fill
    /// lands on the tail of the run and slices whichever glyph straddles the edge.
    ///
    /// Macintosh Zork Zero's InvisiClues menu is the report, drawn in Geneva 12
    /// (advances 3–11 against a declared 7): every topic stopped dead at
    /// `x + chars * 7` with a half-drawn letter at the cut. Measured on
    /// `stories/Zork Zero Disk.image` — `GREAT HALL AREA` inked to 195 against a
    /// pen ending at 203, `AS A LAST RESORT` to 419 against 425,
    /// `FOR YOUR AMUSEMENT` to 433 against 443.
    ///
    /// A unit case rather than a real-media one because that press draws with
    /// Geneva, which ships with the MACHINE and with no game (SQ-1036) — a case
    /// resolving the real cascade would answer out of whatever the person running
    /// it keeps in `~/.lanthorn/`, which is the trap SQ-1052's first case fell
    /// into. Only the pen matters here and a synthetic one states it exactly.
    ///
    /// FALSIFY by restoring `t.text.chars().count().max(1) as u32 * font_w`.
    #[test]
    fn a_chrome_runs_spare_rect_is_the_pens_span_not_the_declared_one() {
        let profile = crate::interpreter::InterpreterProfile::Macintosh;
        let glyph = |w: u8| blorb::bitmap_font::Glyph {
            width: w,
            rows: (0..15).map(|r| if r == 12 { 0xFF } else { 0x00 }).collect(),
        };
        let font = blorb::bitmap_font::BitmapFont {
            width: 11,
            height: 15,
            baseline: 12,
            bold_smear: 0,
            proportional: true,
            lo: b' ',
            // Advances 7..11 against a declared 7, so a run is reliably WIDER than
            // the box the game reserved — which is the direction Geneva 12 runs on
            // the Macintosh's own cell and the only direction that clips.
            glyphs: (b'\x20'..=b'\x7e').map(|c| glyph(7 + (c % 5))).collect(),
        };
        let tf = crate::native_font::TextFace::new(
            profile,
            crate::native_font::FaceSet::release(font, profile, Some((1, 1))),
            Some((1, 1)),
        );
        assert!(tf.proportional(), "the precondition: a pen that varies");

        const TOPIC: &str = "GREAT HALL AREA";
        let pen = tf.run_px_styled(TOPIC, 0);
        let declared = u32::from(tf.cell().w()) * TOPIC.chars().count() as u32;
        assert!(
            pen > declared,
            "non-vacuity: this face must be WIDER than the cell here ({pen} vs {declared}),              or the two measures agree and the case proves nothing",
        );

        let mut grid = grid_item(0);
        grid.w_px = 640;
        grid.h_px = 400;
        match &mut grid.node {
            WinNode::Grid(g) => {
                g.px_texts = vec![PxText {
                    y: 1,
                    x: 1,
                    text: TOPIC.to_string(),
                    style: 0,
                    fg: 0,
                    bg: 0,
                    grow: 0,
                    gcol: 0,
                }]
            }
            _ => unreachable!(),
        }
        let rects = chrome_text_rects(&[&grid], &tf);
        assert_eq!(rects.len(), 1, "one run, one rect: {rects:?}");
        let (x0, _, x1, _) = rects[0];
        assert_eq!(
            x1 - x0,
            pen,
            "the spared width is the pen's, so the page fill stops where the ink does",
        );
    }

    /// **The over-art question belongs to a GLYPH, not to a joined chain** (SQ-1052).
    ///
    /// `region_has_opaque` answers *"is ANY pixel in this rectangle opaque?"* — a
    /// fair question about one character cell and a meaningless one about a long
    /// run. It was safe while a v6 grid published ONE RUN PER CHARACTER, because
    /// every probe was then one cell wide, and stopped being safe when
    /// [`pen_chains`] began joining those runs into lines for a proportional pen.
    ///
    /// This is the RULE, on a canvas built here so it holds whatever any shipped
    /// game happens to publish. The real-media half is
    /// `v6_arthur_status::mac_arthur_raster_score_bar_is_one_ribbon_not_the_location_alone`,
    /// which needs the Macintosh's own Geneva and therefore a boot disk the repo
    /// cannot carry — so this case is the one that runs everywhere, and the two
    /// together are the pair CLAUDE.md asks for.
    ///
    /// The frame: one inherited-reverse row of single-character runs laid at the
    /// pen's own advances, so they join into a single chain, over frame art that is
    /// opaque in ONE ten-pixel patch near the right end.
    ///
    /// FALSIFY by hoisting the probe back out of the glyph loop and asking it once
    /// per run over `cell.run_px(&t.text).max(font_w)`: the patch condemns the whole
    /// chain, every cell takes SQ-0487's no-block arm, and the bar comes out as
    /// page — which is the reported screen.
    #[test]
    fn a_joined_reverse_chain_resolves_its_block_per_glyph_not_per_run() {
        let profile = crate::interpreter::InterpreterProfile::Macintosh;
        // A varying pen is the whole precondition: without one nothing joins and
        // the defect cannot exist. Synthetic, because the Macintosh's own body face
        // ships with the machine and with no game (SQ-1036).
        // Inked on ONE row, not solid: a glyph must carry SOME ink to count as
        // defined (a blank one measures as a zero advance), and a solid one would
        // paint over the very block this case is about.
        let glyph = |w: u8| blorb::bitmap_font::Glyph {
            width: w,
            rows: (0..15).map(|r| if r == 12 { 0xFF } else { 0x00 }).collect(),
        };
        let font = blorb::bitmap_font::BitmapFont {
            width: 11,
            height: 15,
            baseline: 12,
            bold_smear: 0,
            proportional: true,
            lo: b' ',
            glyphs: (b'\x20'..=b'\x7e').map(|c| glyph(3 + (c % 9))).collect(),
        };
        let tf = crate::native_font::TextFace::new(
            profile,
            crate::native_font::FaceSet::release(font, profile, Some((1, 1))),
            Some((1, 1)),
        );
        assert!(tf.proportional(), "the precondition: a pen that varies");
        let (cw, ch) = (u32::from(tf.cell().w()), u32::from(tf.cell().h()));

        // The art: transparent everywhere but one patch, which is what a frame's
        // own rule or a pole looks like to the probe.
        const ART_X: u32 = 500;
        const PY: u32 = 195;
        let mut art = image::RgbaImage::new(640, 400);
        for y in PY..PY + ch {
            for x in ART_X..ART_X + 10 {
                art.put_pixel(x, y, Rgba([9, 9, 9, 255]));
            }
        }
        let frame = PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 0, y_px: 0, w_px: 640, h_px: 400,
            left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(GraphicsWindow { win: 7, canvas: Arc::new(art), version: 0, upscale: false }),
        };

        // The bar: one reversed run per character at the pen's own positions, so
        // `pen_chains` joins them into ONE chain reaching past the patch.
        let bar = "Churchyard                                                        Compline";
        let mut px_texts = Vec::new();
        let mut pen = 28u32;
        for c in bar.chars() {
            px_texts.push(PxText {
                y: PY as u16 + 1,
                x: pen as u16 + 1,
                text: c.to_string(),
                style: 1, // reverse, inherited colours — SQ-0487's arm
                fg: 0,
                bg: 0,
                grow: 13,
                gcol: ((pen - 28) / cw) as u16,
            });
            pen += tf.advance_styled(c, 1);
        }
        assert!(pen > ART_X + 10, "the chain must reach past the patch (ended at {pen})");
        let mut grid = grid_item(28);
        grid.y_px = PY as u16;
        grid.w_px = 584;
        grid.h_px = ch as u16;
        match &mut grid.node {
            WinNode::Grid(g) => g.px_texts = px_texts,
            _ => unreachable!(),
        }

        let chrome: Vec<&PositionedWindow> = vec![&frame, &grid];
        let fg = Rgba([220, 220, 220, 255]);
        let canvas = build_chrome_canvas(
            &chrome, (640, 400), fg, Rgba([0, 0, 0, 255]), &colors(), TextLayer::All, &tf,
        );

        // Counted rather than sampled: a proportional pen can advance further than
        // the declared cell a block is drawn at, so the ribbon inside a chain is
        // blocks with hairlines between them rather than one solid fill. What
        // separates the two answers is not a pixel, it is whether the blocks are
        // there at all.
        let mid = PY + ch / 2;
        let block_px = |xs: std::ops::Range<u32>| xs.filter(|&x| *canvas.get_pixel(x, mid) == fg).count();
        let clear = block_px(100..400);
        assert!(
            clear > 150,
            "glyphs clear of the artwork paint their reversed blocks — one opaque patch 500 px away must not speak for them (only {clear} of 300 px)",
        );
        // …and the cells ON the patch still do not, which is the rule SQ-0487 added
        // and this must not undo.
        let over = block_px(ART_X..ART_X + 10);
        assert_eq!(over, 0, "a glyph over the artwork still draws ink on it rather than a block");
    }

    fn grid_item(x_px: u16) -> PositionedWindow {
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Grid(GridWindow {
                win: 0,
                fill: None,
                cols: 1, rows: 1, cells: vec![], active_rows: 1, cursor: (0, 0), cursor_active: false,
                border: BorderPref::Unspecified, bg: None, fg: None, reverse: false,
                px_texts: Vec::new(),
            }),
        }
    }

    fn graphics_item(x_px: u16) -> PositionedWindow {
        graphics_item_win(x_px, 7)
    }

    fn graphics_item_win(x_px: u16, win: u32) -> PositionedWindow {
        let canvas = Arc::new(image::RgbaImage::new(1, 1));
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(GraphicsWindow { win, canvas, version: 0, upscale: false }),
        }
    }

    fn buffer_item(x_px: u16, primary: bool) -> PositionedWindow {
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary, ..Default::default() }),
        }
    }

    /// A window-0 plate `w`×`h` painted opaque at native `(x, y)`.
    fn plate_at(x: u16, y: u16, w: u32, h: u32) -> PositionedWindow {
        let canvas = Arc::new(image::RgbaImage::from_pixel(w, h, image::Rgba([1, 2, 3, 255])));
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: x, y_px: y, w_px: w as u16, h_px: h as u16,
            left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(GraphicsWindow { win: 0, canvas, version: 0, upscale: false }),
        }
    }

    // ── story_prose_box (SQ-0707) ────────────────────────────────────────────

    #[test]
    fn story_prose_box_without_a_plate_is_the_whole_clear_interior() {
        assert_eq!(story_prose_box((0, 0, 640, 400), None, zvm::screen::V6Cell::DEFAULT), Some((0, 0, 640, 400)));
    }

    /// Arthur's real geometry: a 584×392 plate centred in window 0's 640×400 box
    /// leaves 28px side margins (3 cells — below the 8-column floor) and 4px
    /// top/bottom (under one 16px line). Nothing survives, so the plate owns the
    /// screen and no prose is drawn. This is the SQ-0707 symptom in one line.
    #[test]
    fn story_prose_box_yields_the_screen_to_a_centred_full_bleed_plate() {
        let plate = plate_at(28, 4, 584, 392);
        assert_eq!(
            story_prose_box((0, 0, 640, 400), Some(&plate), zvm::screen::V6Cell::DEFAULT),
            None,
            "a plate leaving only a 3-cell side margin is not a prose box — the picture owns \
             the screen exactly as a window-filling one does (SQ-0578)"
        );
    }

    /// Graceful degradation: a plate that leaves a genuine column still gets
    /// prose beside it, in the widest strip it left. A 240px-wide plate down the
    /// left of a 640px box leaves 400px (50 cells) on the right.
    #[test]
    fn story_prose_box_keeps_the_column_a_margin_illustration_leaves() {
        let plate = plate_at(0, 0, 240, 400);
        assert_eq!(
            story_prose_box((0, 0, 640, 400), Some(&plate), zvm::screen::V6Cell::DEFAULT),
            Some((240, 0, 400, 400)),
            "prose wraps in the column beside a margin illustration"
        );
    }

    /// A plate wholly outside the story's clear interior changes nothing.
    #[test]
    fn story_prose_box_ignores_a_plate_that_misses_the_text_box() {
        let plate = plate_at(0, 0, 100, 100);
        assert_eq!(story_prose_box((200, 200, 400, 200), Some(&plate), zvm::screen::V6Cell::DEFAULT), Some((200, 200, 400, 200)));
    }

    /// Only PAINTED pixels count: a plate whose canvas is fully transparent (a
    /// window-0 graphics leaf that has drawn nothing yet) never takes the screen.
    #[test]
    fn story_prose_box_ignores_an_unpainted_plate() {
        let canvas = Arc::new(image::RgbaImage::new(584, 392));
        let plate = PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 28, y_px: 4, w_px: 584, h_px: 392,
            left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(GraphicsWindow { win: 0, canvas, version: 0, upscale: false }),
        };
        assert_eq!(story_prose_box((0, 0, 640, 400), Some(&plate), zvm::screen::V6Cell::DEFAULT), Some((0, 0, 640, 400)));
    }

    // ── story_text_native (SQ-0896) ──────────────────────────────────────────

    /// A story window at a native box, for the clear/prose composition.
    fn story_box(x: u16, y: u16, w: u16, h: u16) -> PositionedWindow {
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: x, y_px: y, w_px: w, h_px: h,
            left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
        }
    }

    /// A native canvas with `rect` painted opaque, standing in for frame art.
    fn art_canvas(native: (u32, u32), rect: (u32, u32, u32, u32)) -> RgbaImage {
        let mut c = RgbaImage::new(native.0, native.1);
        for y in rect.1..(rect.1 + rect.3).min(native.1) {
            for x in rect.0..(rect.0 + rect.2).min(native.0) {
                c.put_pixel(x, y, Rgba([9, 9, 9, 255]));
            }
        }
        c
    }

    /// The no-art case, which is every corpus frame that reaches the ring today:
    /// nothing to inset past and no plate, so the derived rect IS the declared
    /// window box and the hybrid viewport does not move.
    #[test]
    fn story_text_native_is_the_declared_box_when_no_art_touches_it() {
        let story = story_box(86, 78, 468, 320);
        let empty = RgbaImage::new(640, 400);
        assert_eq!(story_text_native(Some(&story), &empty, None, zvm::screen::V6Cell::DEFAULT), Some((86, 78, 468, 320)));
    }

    /// Frame art overlapping the window's top edge insets it, exactly as raster's
    /// `story_clear_native` call does — this is the step the ring never took.
    ///
    /// The side columns come in too, and that is [`story_clear_native`]'s documented
    /// INTERLEAVED inset rather than a surprise: each edge advances one pixel per
    /// round, so while the top is still walking down through a 32-row banner the left
    /// and right edges are walking inward through the same banner's own row range.
    /// They stop the round the top clears it. Pinned here because hybrid now shares
    /// the rule with raster, and a full-width banner OVERLAPPING window 0 would cost
    /// the prose 31 columns a side — no corpus frame does that (every v6 title places
    /// window 0 to fit its own frame; SQ-0894 measured clear == declared on all of
    /// them), but the next one that does will land here first.
    #[test]
    fn story_text_native_insets_past_frame_art_on_an_edge() {
        let story = story_box(0, 0, 640, 400);
        let banner = art_canvas((640, 400), (0, 0, 640, 32));
        assert_eq!(story_text_native(Some(&story), &banner, None, zvm::screen::V6Cell::DEFAULT), Some((31, 32, 578, 368)));
    }

    /// The half edge insetting cannot reach: a plate CENTRED in the window touches
    /// no edge, so `story_clear_native` returns the whole box and only
    /// `story_prose_box` can see it. Arthur's intro geometry — the plate wins.
    #[test]
    fn story_text_native_yields_to_a_centred_plate_edge_insetting_cannot_see() {
        let story = story_box(0, 0, 640, 400);
        let empty = RgbaImage::new(640, 400);
        let plate = plate_at(28, 4, 584, 392);
        assert_eq!(
            story_clear_native(Some(&story), &empty),
            Some((0, 0, 640, 400)),
            "the inset is blind to a plate that touches no edge — that is the gap"
        );
        assert_eq!(story_text_native(Some(&story), &empty, Some(&plate), zvm::screen::V6Cell::DEFAULT), None);
    }

    /// …and the other half it cannot reach: a HOLLOW plate touching all four edges
    /// insets to nothing (fmvpoker's 640x400 poker table — MEASURED as
    /// `clear (320,54,0,322)` against an oracle carrying the plate), while the
    /// largest-free-rectangle sweep finds the hole the game prints in. This is why
    /// the plate belongs to `story_prose_box` and never to the inset's oracle.
    #[test]
    fn story_text_native_finds_the_hole_in_a_frame_shaped_plate() {
        let story = story_box(0, 0, 640, 400);
        let empty = RgbaImage::new(640, 400);
        let mut canvas = RgbaImage::from_pixel(640, 400, Rgba([1, 2, 3, 255]));
        for y in 100..300 {
            for x in 40..600 {
                canvas.put_pixel(x, y, Rgba([0, 0, 0, 0]));
            }
        }
        let plate = PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 0, y_px: 0, w_px: 640, h_px: 400,
            left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(GraphicsWindow {
                win: 0, canvas: Arc::new(canvas), version: 0, upscale: false,
            }),
        };
        assert_eq!(story_text_native(Some(&story), &empty, Some(&plate), zvm::screen::V6Cell::DEFAULT), Some((40, 100, 560, 200)));
    }

    /// Both steps at once, and in the right order: art insets the window, then the
    /// plate is measured against what the inset LEFT — not against the raw box. The
    /// plate's right edge is at native 200 and the prose starts there; the box's own
    /// right edge is 541 because the banner pulled it in (see the note above), which
    /// is what "measured against the inset box" means.
    #[test]
    fn story_text_native_measures_the_plate_against_the_inset_box() {
        let story = story_box(0, 0, 640, 400);
        let banner = art_canvas((640, 400), (0, 0, 640, 100));
        let plate = plate_at(0, 100, 200, 300);
        assert_eq!(story_text_native(Some(&story), &banner, Some(&plate), zvm::screen::V6Cell::DEFAULT), Some((200, 100, 341, 300)));
    }

    /// Frame art that swallows the WHOLE window is a backdrop, not a border: the
    /// interleaved inset converges on nothing, and the window keeps its declared box
    /// rather than losing a story region hybrid has always drawn prose into. This is
    /// `hybrid_v6_model`'s geometry — the synthetic frame three render tests are
    /// built on — and it is what separates the advisory inset from the authoritative
    /// plate.
    #[test]
    fn story_text_native_keeps_the_declared_box_when_art_swallows_the_whole_window() {
        let story = story_box(43, 39, 234, 160);
        let solid = art_canvas((320, 200), (0, 0, 320, 200));
        assert_eq!(
            story_clear_native(Some(&story), &solid),
            Some((122, 119, 76, 0)),
            "the inset converges on a zero-height sliver when every edge is opaque"
        );
        assert_eq!(story_text_native(Some(&story), &solid, None, zvm::screen::V6Cell::DEFAULT), Some((43, 39, 234, 160)));
    }

    /// `native_viewport_box` quantizes a derived rect exactly as the window-box
    /// form quantizes a window's own — same step, different rectangle.
    #[test]
    fn native_viewport_box_matches_story_viewport_box_on_the_same_rect() {
        let story = story_box(86, 78, 468, 320);
        let scale = Scale { s: 1.225, off_x: 0, off_y: 88 };
        assert_eq!(
            native_viewport_box(Some((86, 78, 468, 320)), &scale, (98, 37), (8, 18)),
            story_viewport_box(Some(&story), &scale, (98, 37), (8, 18)),
        );
    }

    #[test]
    fn story_is_the_primary_buffer_and_chrome_preserves_order() {
        let items = vec![graphics_item(1), grid_item(2), buffer_item(3, true)];
        let layout = classify_windows(&items, zvm::screen::V6Cell::DEFAULT);
        let story = layout.story.expect("primary buffer found");
        assert!(matches!(&story.node, WinNode::Buffer(b) if b.primary));
        assert_eq!(story.x_px, 3);
        assert_eq!(layout.chrome.len(), 2);
        assert_eq!(layout.chrome[0].x_px, 1);
        assert_eq!(layout.chrome[1].x_px, 2);
    }

    #[test]
    fn no_primary_buffer_means_no_story_and_all_chrome() {
        let items = vec![grid_item(1), graphics_item(2), buffer_item(3, false)];
        let layout = classify_windows(&items, zvm::screen::V6Cell::DEFAULT);
        assert!(layout.story.is_none());
        assert!(layout.story_gfx.is_none());
        assert_eq!(layout.chrome.len(), items.len());
    }

    // ── ring_middle_grid (SQ-0934) ───────────────────────────────────────────
    //
    // The rule is STRUCTURAL and names no game: a chrome grid carrying text,
    // whose rect the frame art leaves clear, with frame art around it. These
    // cases are built from synthetic windows for exactly that reason — if the
    // rule needed to know it was looking at InvisiClues, it could not be written
    // without one.

    /// A chrome graphics window `win` holding `native`-sized art with `rect`
    /// painted opaque — a stand-in for any game's frame.
    fn frame_art(native: (u32, u32), rects: &[(u32, u32, u32, u32)]) -> PositionedWindow {
        let mut c = RgbaImage::new(native.0, native.1);
        for &(rx, ry, rw, rh) in rects {
            for y in ry..(ry + rh).min(native.1) {
                for x in rx..(rx + rw).min(native.0) {
                    c.put_pixel(x, y, Rgba([9, 9, 9, 255]));
                }
            }
        }
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 0, y_px: 0,
            w_px: native.0 as u16, h_px: native.1 as u16,
            left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(GraphicsWindow { win: 7, canvas: Arc::new(c), version: 0, upscale: false }),
        }
    }

    /// A chrome grid at a native rect, carrying `text` if non-empty.
    fn text_grid(x: u16, y: u16, w: u16, h: u16, text: &str) -> PositionedWindow {
        let px_texts = if text.is_empty() {
            Vec::new()
        } else {
            vec![crate::engine::PxText::derived(y + 1, x + 1, text.into(), 0, 0, 0, zvm::screen::V6Cell::DEFAULT)]
        };
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: x, y_px: y, w_px: w, h_px: h,
            left_margin: 0, right_margin: 0,
            node: WinNode::Grid(GridWindow {
                win: 0,
                fill: None, cols: 1, rows: 1, cells: vec![], active_rows: 1,
                cursor: (0, 0), cursor_active: false, border: BorderPref::Unspecified,
                bg: None, fg: None, reverse: false, px_texts,
            }),
        }
    }

    /// A ring — banner across the top, a flank down each side, clear middle —
    /// with a text grid filling the middle. This is the corpus shape: Zork Zero's
    /// and Shogun's hint screen measure 78% of the top band and 70% of each flank
    /// opaque with the middle at 0.0%.
    fn ringed_menu() -> Vec<PositionedWindow> {
        vec![
            frame_art((640, 400), &[(0, 0, 640, 78), (0, 78, 86, 322), (554, 78, 86, 322)]),
            text_grid(86, 78, 468, 322, "GREAT HALL AREA"),
        ]
    }

    #[test]
    fn a_text_grid_in_a_rings_clear_middle_becomes_the_story_surface() {
        let items = ringed_menu();
        let layout = classify_windows(&items, zvm::screen::V6Cell::DEFAULT);
        let story = layout.story.expect("the middle grid stands in for the withdrawn buffer");
        assert!(matches!(&story.node, WinNode::Grid(_)), "it is a Grid, not a Buffer");
        assert_eq!((story.x_px, story.y_px, story.w_px, story.h_px), (86, 78, 468, 322));
    }

    /// **It stays in `chrome` as well**, which is the load-bearing half: `story`
    /// is wanted for its RECT, while the runs it carries must still reach
    /// `chrome_runs` or the menu would not be drawn at all.
    #[test]
    fn the_promoted_grid_remains_in_chrome_so_its_runs_are_still_drawn() {
        let items = ringed_menu();
        let layout = classify_windows(&items, zvm::screen::V6Cell::DEFAULT);
        let story = layout.story.expect("promoted");
        assert!(
            layout.chrome.iter().any(|c| std::ptr::eq(*c, story)),
            "the promoted grid must still be chrome, or its 22 menu runs vanish",
        );
    }

    /// A frame with no artwork around it is not a ring. This is the guard that
    /// keeps every ordinary no-story frame on the arm it already had.
    #[test]
    fn a_text_grid_with_no_frame_around_it_is_not_promoted() {
        let items = vec![
            frame_art((640, 400), &[]), // a graphics window, but empty
            text_grid(86, 78, 468, 322, "GREAT HALL AREA"),
        ];
        assert!(classify_windows(&items, zvm::screen::V6Cell::DEFAULT).story.is_none(), "no ink outside means no ring");
    }

    /// scopa's shape (SQ-0711): grids and no graphics window at all. Its card
    /// table is an `erase_window` paint ground, which is not a frame, so it keeps
    /// the arm SQ-0711 chose for it.
    #[test]
    fn a_screen_of_grids_with_no_graphics_window_is_not_promoted() {
        let items = vec![text_grid(0, 0, 320, 200, "abort"), text_grid(86, 78, 468, 322, "OK")];
        assert!(classify_windows(&items, zvm::screen::V6Cell::DEFAULT).story.is_none(), "no graphics window, no frame");
    }

    /// Art THROUGH the candidate means it is not a clear middle — the game is
    /// drawing over that rect, so the pixels there are somebody's artwork and
    /// only the composite can show them.
    #[test]
    fn a_grid_with_art_behind_its_own_rect_is_not_promoted() {
        let items = vec![
            frame_art((640, 400), &[(0, 0, 640, 78), (0, 78, 86, 322), (554, 78, 86, 322), (86, 78, 468, 322)]),
            text_grid(86, 78, 468, 322, "GREAT HALL AREA"),
        ];
        assert!(classify_windows(&items, zvm::screen::V6Cell::DEFAULT).story.is_none(), "opaque under the rect is not a clear middle");
    }

    /// A grid spanning the whole screen is not IN a middle, it IS the screen.
    /// Zork Zero's Macintosh release publishes exactly such a grid alongside a
    /// live buffer, and must not have it mistaken for a viewport.
    #[test]
    fn a_full_screen_grid_is_not_a_middle() {
        let items = vec![
            frame_art((640, 400), &[(0, 0, 640, 78), (0, 78, 86, 322), (554, 78, 86, 322)]),
            text_grid(0, 0, 640, 400, "GREAT HALL AREA"),
        ];
        assert!(classify_windows(&items, zvm::screen::V6Cell::DEFAULT).story.is_none(), "the whole screen is not a middle");
    }

    /// An EMPTY grid in the middle is not a menu. Shogun publishes a 169×48 grid
    /// with no runs on every frame; promoting that would hand the ring a viewport
    /// with nothing in it.
    #[test]
    fn an_empty_grid_in_the_middle_is_not_promoted() {
        let items = vec![
            frame_art((640, 400), &[(0, 0, 640, 78), (0, 78, 86, 322), (554, 78, 86, 322)]),
            text_grid(86, 78, 468, 322, ""),
        ];
        assert!(classify_windows(&items, zvm::screen::V6Cell::DEFAULT).story.is_none(), "a grid with no text is not a menu");
    }

    /// A real primary buffer always wins — the promotion is a fallback, so no
    /// frame that has a story window today can change behaviour.
    #[test]
    fn a_primary_buffer_still_wins_over_a_ring_middle_grid() {
        let mut items = ringed_menu();
        items.push(story_box(86, 78, 468, 320));
        let layout = classify_windows(&items, zvm::screen::V6Cell::DEFAULT);
        let story = layout.story.expect("the buffer");
        assert!(matches!(&story.node, WinNode::Buffer(_)), "a published buffer is never displaced");
    }

    /// The LARGEST qualifying grid wins, so a game that puts a caption in the
    /// clear middle beside its menu does not hand the ring the caption.
    #[test]
    fn the_largest_clear_middle_grid_wins() {
        let mut items = ringed_menu();
        items.push(text_grid(100, 90, 60, 16, "caption"));
        let story = classify_windows(&items, zvm::screen::V6Cell::DEFAULT).story.expect("promoted");
        assert_eq!(story.w_px, 468, "the menu, not the caption");
    }

    #[test]
    fn window_zero_graphics_is_story_content_not_chrome() {
        // The primary window's own picture (window 0) is the room illustration —
        // story content, kept out of chrome so it renders inside the story region.
        let items = vec![
            graphics_item_win(1, 0), // window 0's illustration
            graphics_item_win(2, 7), // window 7 frame → chrome
            buffer_item(3, true),    // story
        ];
        let layout = classify_windows(&items, zvm::screen::V6Cell::DEFAULT);
        assert_eq!(layout.story.expect("story").x_px, 3);
        assert_eq!(layout.story_gfx.expect("story_gfx").x_px, 1);
        assert_eq!(layout.chrome.len(), 1, "only window 7 graphics is chrome");
        assert_eq!(layout.chrome[0].x_px, 2);
    }

    fn colors() -> ColorScheme {
        ColorScheme::default()
    }

    #[test]
    fn story_text_wraps_right_of_float_and_blits_it() {
        // Rows covered by a float are inset by its indent (text flows beside the
        // picture); rows past it are flush left; the float's pixels are blitted
        // at its anchored row.
        let cell_has_ink = |c: &RgbaImage, col: u32, row: u32| -> bool {
            (0..FONT_H).any(|dy| (0..FONT_W).any(|dx| c.get_pixel(col * FONT_W + dx, row * FONT_H + dy)[3] > 0))
        };
        // A 16×32 opaque red image → float of 2 rows (32px / FONT_H(16) = 2).
        let img = RgbaImage::from_pixel(16, 32, Rgba([200, 20, 20, 255]));
        let main = MainText {
            lines: vec!["AAAA".into(), "BBBB".into(), "CCCC".into()],
            styles: Vec::new(),
            input: String::new(), cursor_col: 0, awaiting: false,
            floats: vec![RasterFloat { row: 0, rows: 2, reserve_cols: 3, text_col: 3, img_col: 0, img: Arc::new(img) }],
        };
        let mut canvas = RgbaImage::new(10 * FONT_W, 5 * FONT_H);
        draw_story_text(&mut canvas, &main, 0, 0, 10, 5, Rgba([255, 255, 255, 255]), &[], &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT), None);
        // Rows 0-1 (beside float): glyph ink starts at column 3.
        assert!(cell_has_ink(&canvas, 0, 0), "float pixels occupy row 0 col 0");
        assert_eq!(*canvas.get_pixel(4, 20), Rgba([200, 20, 20, 255]), "float blitted at its row (spans y 0..32)");
        assert!(cell_has_ink(&canvas, 3, 0), "row 0 col 3 inked (text beside the float)");
        assert!(cell_has_ink(&canvas, 3, 1), "row 1 col 3 inked (text beside the float)");
        // Row 2 (past the float): ink flush left.
        assert!(cell_has_ink(&canvas, 0, 2), "row 2 col 0 inked (flush left below float)");
    }

    #[test]
    fn story_text_wraps_left_of_right_float_and_blits_it_right() {
        // A RIGHT float (Shogun's opening picture): text stays flush LEFT and is
        // narrowed to `cols - reserve_cols`; the picture blits at `img_col` near
        // the right edge; rows past the picture reclaim full width.
        let cell_has_ink = |c: &RgbaImage, col: u32, row: u32| -> bool {
            (0..FONT_H).any(|dy| (0..FONT_W).any(|dx| c.get_pixel(col * FONT_W + dx, row * FONT_H + dy)[3] > 0))
        };
        // 10-col region; a 32×32 image → 4 cols wide, 2 rows tall; reserve 5 cols
        // (image + gutter), text confined to cols 0..5, image blits at col 6.
        let img = RgbaImage::from_pixel(32, 32, Rgba([20, 200, 20, 255]));
        let main = MainText {
            lines: vec!["AAAAAAAA".into(), "BBBB".into(), "CCCCCCCC".into()],
            styles: Vec::new(),
            input: String::new(), cursor_col: 0, awaiting: false,
            floats: vec![RasterFloat { row: 0, rows: 2, reserve_cols: 5, text_col: 0, img_col: 6, img: Arc::new(img) }],
        };
        let mut canvas = RgbaImage::new(10 * FONT_W, 5 * FONT_H);
        draw_story_text(&mut canvas, &main, 0, 0, 10, 5, Rgba([255, 255, 255, 255]), &[], &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT), None);
        // Row 0 text is flush left but clipped to the narrowed column (cols 0..5).
        assert!(cell_has_ink(&canvas, 0, 0), "row 0 col 0 inked (text flush left)");
        assert!(!cell_has_ink(&canvas, 5, 0), "row 0 col 5 blank (text narrowed away from the picture)");
        // The picture blits at col 6 (img_col), on the right.
        assert_eq!(*canvas.get_pixel(6 * FONT_W, 0), Rgba([20, 200, 20, 255]), "float blitted at img_col 6");
        // Row 2 (past the float) reclaims full width.
        assert!(cell_has_ink(&canvas, 6, 2), "row 2 col 6 inked (full width below the float)");
    }

    #[test]
    fn packed_standard_palette_colour_blits_its_own_rgb_not_default() {
        // SQ-0480/SQ-0506: a run coloured with a Standard palette colour (the
        // compass letters) must blit in that colour, not the default ink. On the
        // PIXEL path, Standard 2..=9 resolve to the ZMSD §8.3.1 true-colour RGB
        // (DOS/spec-authentic) rather than the theme's dim VGA ANSI values — so
        // red is the spec red $001D → (239,0,0), NOT the old VGA base-red
        // (170,0,0). White(9) likewise becomes real white (255,255,255).
        let colors = ColorScheme::terminal_default();
        let fallback = Rgba([1, 2, 3, 255]);
        // Standard(3): packed tag 1, value 3 (see state::pack_zcolour).
        let packed_std3 = (1u32 << 24) | 3;
        let got = packed_to_rgba(packed_std3, fallback, &colors);
        assert_ne!(got, fallback, "a palette colour must NOT fall back to the default ink");
        assert_eq!(got, Rgba([239, 0, 0, 255]), "Standard(3) → spec red $001D on the pixel path");
        // Standard(9) white must be TRUE white, not the VGA base-grey it used to be.
        let packed_std9 = (1u32 << 24) | 9;
        assert_eq!(
            packed_to_rgba(packed_std9, fallback, &colors),
            Rgba([255, 255, 255, 255]),
            "Standard(9) → true white 255,255,255 (ZMSD $7FFF), not VGA grey 170,170,170"
        );
        // Standard(2) black stays black.
        assert_eq!(
            packed_to_rgba((1u32 << 24) | 2, fallback, &colors),
            Rgba([0, 0, 0, 255]),
            "Standard(2) → black 0,0,0 (ZMSD $0000)"
        );
        // And the full blit through build_chrome_canvas carries it: a space-only
        // run has no ink, so probe an inked glyph's fg by asserting SOME cell pixel
        // is the run's red.
        let win = px_text_grid_item("N", 0, packed_std3, 0);
        let c = build_chrome_canvas(&[&win], (8, 8), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors, TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        assert!(
            (0..8).any(|x| (0..8).any(|y| *c.get_pixel(x, y) == Rgba([239, 0, 0, 255]))),
            "the compass glyph blits in its own spec red, not the default fg"
        );
    }

    #[test]
    fn native_extent_ignores_unresolved_size_sentinel() {
        // SQ-0481: a real 320×200 window plus a bogus window whose x_size leaked
        // the -2 sentinel (0xFFFE ≈ 65534). The sentinel must NOT balloon the
        // native extent (and thus the raster canvas allocation) — the real
        // 320×200 screen size stands.
        let real = || PositionedWindow { x_px: 0, y_px: 0, w_px: 320, h_px: 200, ..buffer_item(0, true) };
        let bogus = PositionedWindow { x_px: 0, y_px: 0, w_px: 0xFFFE, h_px: 200, ..grid_item(0) };
        assert_eq!(native_extent(&[real(), bogus], &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT)), (320, 200), "sentinel width excluded");
        // A sentinel HEIGHT is likewise ignored on its axis.
        let bogus_h = PositionedWindow { x_px: 0, y_px: 0, w_px: 320, h_px: 0xFFFD, ..grid_item(0) };
        assert_eq!(native_extent(&[real(), bogus_h], &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT)), (320, 200), "sentinel height excluded");
    }

    #[test]
    fn story_text_input_continues_the_prompt_row() {
        // SQ-0470a: the live input sits on the game's kept ">" prompt row,
        // appended right after it — NOT a fresh row below it.
        let cell_has_ink = |c: &RgbaImage, col: u32, row: u32| -> bool {
            (0..FONT_H).any(|dy| (0..FONT_W).any(|dx| c.get_pixel(col * FONT_W + dx, row * FONT_H + dy)[3] > 0))
        };
        let main = MainText {
            lines: vec!["Room desc.".into(), ">".into()],
            styles: Vec::new(),
            input: "go".into(),
            cursor_col: 2,
            awaiting: true,
            floats: vec![],
        };
        let mut canvas = RgbaImage::new(20 * FONT_W, 5 * FONT_H);
        draw_story_text(&mut canvas, &main, 0, 0, 20, 5, Rgba([255, 255, 255, 255]), &[], &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT), None);
        // ">" is on row 1; input "go" appends after it at cols 1 and 2.
        assert!(cell_has_ink(&canvas, 1, 1), "input 'g' on the prompt row, after '>'");
        assert!(cell_has_ink(&canvas, 2, 1), "input 'o' on the prompt row");
        // Caret block after the input: col = 1 (\">\".len) + 2 (cursor) = 3.
        assert!(cell_has_ink(&canvas, 3, 1), "caret after the input on the prompt row");
        // The row BELOW the prompt is empty — input no longer drops a row.
        assert!(!(0..20).any(|col| cell_has_ink(&canvas, col, 2)), "nothing on the row below the prompt");
    }

    #[test]
    fn story_text_input_after_newline_starts_a_clean_row() {
        // When the transcript ended on a newline the last line is empty, so the
        // input starts a clean row of its own (col 0) — the universal rule that
        // makes SQ-0470a correct for both prompt and non-prompt endings.
        let cell_has_ink = |c: &RgbaImage, col: u32, row: u32| -> bool {
            (0..FONT_H).any(|dy| (0..FONT_W).any(|dx| c.get_pixel(col * FONT_W + dx, row * FONT_H + dy)[3] > 0))
        };
        let main = MainText {
            lines: vec!["Prose line.".into(), String::new()],
            styles: Vec::new(),
            input: "x".into(),
            cursor_col: 1,
            awaiting: true,
            floats: vec![],
        };
        let mut canvas = RgbaImage::new(20 * FONT_W, 5 * FONT_H);
        draw_story_text(&mut canvas, &main, 0, 0, 20, 5, Rgba([255, 255, 255, 255]), &[], &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT), None);
        assert!(cell_has_ink(&canvas, 0, 1), "input on the empty last row at col 0");
        assert!(!(0..20).any(|col| cell_has_ink(&canvas, col, 2)), "not the row below");
    }

    #[test]
    fn story_text_scrolled_float_is_cropped_not_pinned() {
        // A float whose anchor scrolled above the view (row = -1) draws only its
        // remaining rows, cropped from its own top (one FONT_H = 16px row).
        let mut img = RgbaImage::new(8, 32);
        for y in 0..32 {
            // Top row (y<16) green, bottom row (y>=16) blue — the visible part,
            // after cropping the scrolled-off top FONT_H row, must be blue.
            let c = if y < 16 { Rgba([0, 200, 0, 255]) } else { Rgba([0, 0, 200, 255]) };
            for x in 0..8 { img.put_pixel(x, y, c); }
        }
        let main = MainText {
            lines: vec!["XXXX".into()],
            styles: Vec::new(),
            input: String::new(), cursor_col: 0, awaiting: false,
            floats: vec![RasterFloat { row: -1, rows: 2, reserve_cols: 2, text_col: 2, img_col: 0, img: Arc::new(img) }],
        };
        let mut canvas = RgbaImage::new(10 * FONT_W, 3 * FONT_H);
        draw_story_text(&mut canvas, &main, 0, 0, 10, 3, Rgba([255, 255, 255, 255]), &[], &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT), None);
        assert_eq!(*canvas.get_pixel(4, 4), Rgba([0, 0, 200, 255]), "visible slice is the float's BOTTOM half");
    }

    #[test]
    fn chrome_graphics_blits_native_and_clips_to_window_box() {
        // The window canvas is authored in native game pixels: build_chrome_canvas
        // blits it 1:1 at the window origin (never scaled to the declared box) and
        // clips at the box edge (ZMSD §8: plotting is always clipped to the window).
        let mut src = image::RgbaImage::new(48, 43);
        src.put_pixel(40, 38, Rgba([10, 200, 30, 255])); // marker low in the canvas
        src.put_pixel(2, 2, Rgba([200, 10, 30, 255])); // marker near the top-left
        let win = |h_px: u16, canvas: image::RgbaImage| PositionedWindow {
            x: 0, y: 0, w: 40, h: 1,
            x_px: 4, y_px: 4, // window origin
            w_px: 320, h_px,
            left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(GraphicsWindow {
                win: 1, canvas: Arc::new(canvas), version: 0, upscale: false,
            }),
        };
        // Box tall enough (40): both markers land 1:1 — never squashed.
        let tall = win(40, src.clone());
        let canvas = build_chrome_canvas(&[&tall], (100, 100), Rgba([0, 0, 0, 255]), Rgba([0, 0, 0, 255]), &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        assert_eq!(canvas.get_pixel(6, 6)[3], 255, "top-left marker at native (6,6)");
        assert_eq!(canvas.get_pixel(44, 42)[3], 255, "low marker 1:1 at native (44,42)");
        // Box only 5 tall: content past the box clips; nothing squashes into it.
        let short = win(5, src);
        let canvas = build_chrome_canvas(&[&short], (100, 100), Rgba([0, 0, 0, 255]), Rgba([0, 0, 0, 255]), &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        assert_eq!(canvas.get_pixel(6, 6)[3], 255, "top-left marker inside the box survives");
        assert_eq!(canvas.get_pixel(44, 42)[3], 0, "content below the 5px box is clipped");
        for y in 4..9 {
            assert_eq!(canvas.get_pixel(44, y)[3], 0, "no squashed copy inside the box (y={y})");
        }
    }

    fn graphics_window(x_px: u16, y_px: u16, w: u16, h: u16, canvas: image::RgbaImage) -> PositionedWindow {
        PositionedWindow {
            x: 0, y: 0, w, h, x_px, y_px, w_px: w, h_px: h, left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(GraphicsWindow { win: 0, canvas: Arc::new(canvas), version: 0, upscale: false }),
        }
    }

    #[test]
    fn frame_opaque_border_transparent_interior_and_outside_stays_transparent() {
        // 20x20 native canvas, one chrome Graphics window covering it whose
        // source canvas has an opaque 1px border ring and a transparent
        // center. The built chrome canvas should mirror that: opaque at the
        // border, transparent at the center, and transparent outside the
        // window (there is none here, but the whole canvas is checked).
        let mut src = image::RgbaImage::new(20, 20);
        for x in 0..20u32 {
            src.put_pixel(x, 0, Rgba([255, 255, 255, 255]));
            src.put_pixel(x, 19, Rgba([255, 255, 255, 255]));
        }
        for y in 0..20u32 {
            src.put_pixel(0, y, Rgba([255, 255, 255, 255]));
            src.put_pixel(19, y, Rgba([255, 255, 255, 255]));
        }
        let win = graphics_window(0, 0, 20, 20, src);
        let chrome: Vec<&PositionedWindow> = vec![&win];
        let c = build_chrome_canvas(&chrome, (20, 20), Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255]), &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        assert_eq!(c.get_pixel(0, 0)[3], 255, "border pixel is opaque");
        assert_eq!(c.get_pixel(10, 10)[3], 0, "center is transparent");
    }

    #[test]
    fn later_graphics_entry_draws_over_earlier_through_its_transparent_margin() {
        // Two overlapping chrome Graphics entries at the same native spot
        // (4,4), 8x8 each: "base" solid colour A, then "indicator" solid
        // colour B on its left half and transparent on its right half.
        // Later-drawn wins where opaque; the base shows through the
        // indicator's transparent right half.
        let color_a = Rgba([200, 0, 0, 255]);
        let color_b = Rgba([0, 200, 0, 255]);
        let base = image::RgbaImage::from_pixel(8, 8, color_a);
        let mut indicator = image::RgbaImage::new(8, 8);
        for y in 0..8u32 {
            for x in 0..4u32 {
                indicator.put_pixel(x, y, color_b);
            }
        }
        let base_win = graphics_window(4, 4, 8, 8, base);
        let indicator_win = graphics_window(4, 4, 8, 8, indicator);
        let chrome: Vec<&PositionedWindow> = vec![&base_win, &indicator_win];
        let c = build_chrome_canvas(&chrome, (20, 20), Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255]), &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        assert_eq!(*c.get_pixel(5, 8), color_b, "left half shows the indicator (last-drawn wins)");
        assert_eq!(*c.get_pixel(10, 8), color_a, "right half shows the base through the transparent margin");
    }

    #[test]
    fn status_grid_glyph_paints_fg_in_its_native_pixel_cell() {
        let mut cells = vec![GridCell { ch: ' ', style: 0, fg: 0, bg: 0, link: 0, glk_style: 0 }; 6];
        // row 1, col 2 in a 3-col grid.
        cells[3 + 2] = GridCell { ch: 'A', style: 0, fg: 0, bg: 0, link: 0, glk_style: 0 };
        let win = PositionedWindow {
            x: 0, y: 0, w: 3, h: 2, x_px: 10, y_px: 4, w_px: 24, h_px: 32, left_margin: 0, right_margin: 0,
            node: WinNode::Grid(GridWindow {
                win: 0,
                fill: None,
                cols: 3, rows: 2, cells, active_rows: 2, cursor: (0, 0), cursor_active: false,
                border: BorderPref::Unspecified, bg: None, fg: None, reverse: false,
                px_texts: Vec::new(),
            }),
        };
        let chrome: Vec<&PositionedWindow> = vec![&win];
        let fg = Rgba([0, 255, 255, 255]);
        let c = build_chrome_canvas(&chrome, (40, 40), fg, Rgba([0, 0, 0, 255]), &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        // cell (col=2,row=1) native px box: x = 10 + 2·FONT_W(8) = 26..34,
        // y = 4 + 1·FONT_H(16) = 20..36 (non-square 8×16 cell, SQ-0479).
        assert!(
            (26..34).any(|x| (20..36).any(|y| *c.get_pixel(x, y) == fg)),
            "glyph fg pixels appear within the status cell's native box"
        );
    }

    // ── px_text colour + reverse-video (Lane C) ─────────────────────────────
    //
    // These probe the SOLID FILL colour behind a run, not individual glyph
    // pixels: a run whose text is a single space has no ink bits set, so its
    // whole FONT×FONT cell is exactly `blit_glyph`'s `bg` fill colour (or
    // fully transparent when `bg` is `None`) — a robust way to assert which
    // colour the resolver chose without depending on font-bitmap geometry.
    const RED: u32 = 0x03FF_0000; // True24 packed
    const BLUE: u32 = 0x0300_00FF; // True24 packed

    fn px_text_grid_item(text: &str, style: u8, fg: u32, bg: u32) -> PositionedWindow {
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 0, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Grid(GridWindow {
                win: 0,
                fill: None,
                cols: 1, rows: 1, cells: vec![], active_rows: 1, cursor: (0, 0), cursor_active: false,
                border: BorderPref::Unspecified, bg: None, fg: None, reverse: false,
                px_texts: vec![crate::engine::PxText::derived(1, 1, text.into(), style, fg, bg, zvm::screen::V6Cell::DEFAULT)],
            }),
        }
    }

    /// Lit-`fg` pixel coordinates of a rendered canvas — the shape the raster
    /// font actually drew, independent of where the cells sit.
    fn ink(c: &RgbaImage, fg: Rgba<u8>) -> std::collections::BTreeSet<(u32, u32)> {
        c.enumerate_pixels().filter(|(_, _, p)| **p == fg).map(|(x, y, _)| (x, y)).collect()
    }

    #[test]
    fn px_text_bold_run_double_strikes_the_raster_glyphs() {
        // SQ-0540: a painted run carrying style bit 2 (Journey stamps its command
        // menu labels — "Proceed", "Combat", "Cast" — exactly this way) renders
        // emboldened in the pixel composite, not roman.
        let fg = Rgba([255, 0, 0, 255]);
        let canvas = |style: u8| {
            let win = px_text_grid_item("Ab", style, RED, 0);
            let chrome: Vec<&PositionedWindow> = vec![&win];
            build_chrome_canvas(&chrome, (24, 16), Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255]), &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT))
        };
        let roman = ink(&canvas(0), fg);
        let bold = ink(&canvas(2), fg);
        assert_ne!(bold, roman, "a bold run must not render identically to a roman one");
        assert!(roman.is_subset(&bold), "bold keeps every roman pixel");
        for &(x, y) in bold.difference(&roman) {
            assert!(x > 0 && roman.contains(&(x - 1, y)), "bold pixel ({x},{y}) is not a +1 double-strike");
        }
        // Italic leans the top half; bold-italic is heavier still.
        let italic = ink(&canvas(4), fg);
        assert_ne!(italic, roman, "an italic run must not render roman");
        assert!(ink(&canvas(6), fg).len() > italic.len(), "bold-italic is heavier than italic");
    }

    #[test]
    fn px_text_reverse_only_run_keeps_the_roman_face() {
        // Zork Zero's banner/ribbon chrome is style-REVERSE with no emphasis: the
        // reverse bit is resolved into the fg/bg pair before the blit, so the
        // glyphs must be the same roman shapes a plain run with the swapped pair
        // draws — SQ-0540's faces must not touch it (nor may fixed-pitch, bit 8).
        let render = |style: u8, fg: u32, bg: u32| {
            let win = px_text_grid_item("Ab", style, fg, bg);
            let chrome: Vec<&PositionedWindow> = vec![&win];
            build_chrome_canvas(&chrome, (24, 16), Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255]), &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT))
        };
        let blue = Rgba([0, 0, 255, 255]);
        // Reversed: the run's fg becomes the block, its bg becomes the ink. The
        // INK pixels (blue) must be the same roman glyph shapes a plain blue-on-
        // transparent run draws. (The two canvases differ elsewhere — reverse
        // also floods the row gaps — so compare the ink, not the whole image.)
        let reversed = render(1, RED, BLUE);
        assert_eq!(ink(&reversed, blue), ink(&render(0, BLUE, 0), blue), "reverse ink keeps the roman face");
        assert_eq!(render(1 | 8, RED, BLUE), reversed, "fixed-pitch changes nothing in a bitmap font");
    }

    #[test]
    fn status_grid_cell_carries_bold() {
        // The cell-grid fallback path (no pixel-positioned runs) gets faces too:
        // a v6 game can `set_text_style` bold in any window.
        let cells = |style: u8| vec![GridCell { ch: 'A', style, fg: 0, bg: 0, link: 0, glk_style: 0 }];
        let canvas = |style: u8| {
            let win = PositionedWindow {
                x: 0, y: 0, w: 1, h: 1, x_px: 0, y_px: 0, w_px: 8, h_px: 16, left_margin: 0, right_margin: 0,
                node: WinNode::Grid(GridWindow {
                    win: 0,
                    fill: None,
                    cols: 1, rows: 1, cells: cells(style), active_rows: 1, cursor: (0, 0), cursor_active: false,
                    border: BorderPref::Unspecified, bg: None, fg: None, reverse: false,
                    px_texts: Vec::new(),
                }),
            };
            let chrome: Vec<&PositionedWindow> = vec![&win];
            build_chrome_canvas(&chrome, (8, 16), Rgba([0, 255, 255, 255]), Rgba([0, 0, 0, 255]), &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT))
        };
        let fg = Rgba([0, 255, 255, 255]);
        let roman = ink(&canvas(0), fg);
        let bold = ink(&canvas(2), fg);
        assert!(roman.is_subset(&bold) && bold.len() > roman.len(), "a bold grid cell is emboldened");
    }

    #[test]
    fn story_text_applies_per_char_emphasis() {
        // The prose path (Zork Zero's bold room names, Shogun's italic "Erasmus")
        // takes per-char style bytes parallel to its lines; chars with no entry
        // stay roman, and emphasis never spills into the neighbouring cells.
        let fg = Rgba([255, 255, 255, 255]);
        let draw = |styles: Vec<Vec<u8>>| {
            let main = MainText { lines: vec!["AAAA".into()], styles, input: String::new(), cursor_col: 0, awaiting: false, floats: vec![] };
            let mut c = RgbaImage::new(6 * FONT_W, 2 * FONT_H);
            draw_story_text(&mut c, &main, 0, 0, 6, 2, fg, &[], &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT), None);
            c
        };
        let roman = ink(&draw(Vec::new()), fg);
        // Bold only the two middle chars (cols 1..3).
        let mixed = ink(&draw(vec![vec![0, 2, 2, 0]]), fg);
        assert_ne!(mixed, roman, "an emphasised row must differ from the roman one");
        assert!(roman.is_subset(&mixed), "double-strike is additive");
        for &(x, y) in mixed.difference(&roman) {
            let col = x / FONT_W;
            assert!((1..3).contains(&col), "only the bold columns changed, got a new pixel in col {col} at ({x},{y})");
            assert!(roman.contains(&(x - 1, y)), "new pixel ({x},{y}) is a +1 double-strike");
        }
        // A short/absent style row is all-roman.
        assert_eq!(ink(&draw(vec![Vec::new()]), fg), roman, "an empty style row renders roman");
        assert_eq!(ink(&draw(vec![vec![0, 0]]), fg), roman, "a short style row's tail renders roman");
    }

    #[test]
    fn px_text_run_fills_its_cell_with_the_explicit_background() {
        let win = px_text_grid_item(" ", 0, RED, BLUE);
        let chrome: Vec<&PositionedWindow> = vec![&win];
        let c = build_chrome_canvas(&chrome, (8, 8), Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255]), &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(*c.get_pixel(x, y), Rgba([0, 0, 255, 255]), "cell filled with the run's bg (blue) at ({x},{y})");
            }
        }
    }

    #[test]
    fn px_text_reverse_swaps_the_fill_to_the_foreground_colour() {
        // Same run as above but with style bit 1 (reverse) set: the swap makes
        // the run's FOREGROUND (red) the fill colour instead of its background.
        let win = px_text_grid_item(" ", 1, RED, BLUE);
        let chrome: Vec<&PositionedWindow> = vec![&win];
        let c = build_chrome_canvas(&chrome, (8, 8), Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255]), &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(*c.get_pixel(x, y), Rgba([255, 0, 0, 255]), "reverse fill is the run's fg (red) at ({x},{y})");
            }
        }
    }

    #[test]
    fn px_text_reverse_inherited_over_art_draws_dark_ink_no_block() {
        // The run never chose an explicit colour (fg=bg=0/Default) and sits OVER
        // opaque frame art: reverse video must NOT paint a block — Zork0's ribbon
        // labels print in reverse with inherited colours and the original shows dark
        // ink directly ON the banner art (a block would erase it, the black-box
        // regression the user hit). A blank glyph therefore leaves the art
        // untouched; an inked glyph draws in default_bg (dark) on the art. (SQ-0487
        // keeps this by testing the canvas is opaque behind the run.)
        let default_fg = Rgba([10, 20, 30, 255]);
        let default_bg = Rgba([40, 50, 60, 255]);
        let art_color = Rgba([200, 150, 100, 255]);
        // An opaque 8×8 art window behind the run (pass 1), then the reverse run.
        let art = graphics_window(0, 0, 8, 8, image::RgbaImage::from_pixel(8, 8, art_color));
        let blank = px_text_grid_item(" ", 1, 0, 0);
        let chrome: Vec<&PositionedWindow> = vec![&art, &blank];
        let c = build_chrome_canvas(&chrome, (8, 8), default_fg, default_bg, &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        assert_eq!(*c.get_pixel(4, 4), art_color, "blank reverse glyph over art leaves the art (no block)");
        let inked = px_text_grid_item("X", 1, 0, 0);
        let chrome: Vec<&PositionedWindow> = vec![&art, &inked];
        let c = build_chrome_canvas(&chrome, (8, 8), default_fg, default_bg, &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        assert!(
            (0..8).any(|x| (0..8).any(|y| *c.get_pixel(x, y) == default_bg)),
            "reverse ink over art draws in the themed default_bg (dark on the art)"
        );
    }

    #[test]
    fn px_text_reverse_inherited_over_clear_bg_paints_the_highlight_block() {
        // SQ-0487: the same inherited-colour reverse run over a CLEAR background
        // (Shogun's boot-menu selection bar — no frame art behind it) MUST paint the
        // swapped highlight block: a solid default_fg bar with default_bg ink. A
        // blank gap run between words fills its whole cell with the bar colour, so
        // the selection bar reads solid (not moth-eaten).
        let default_fg = Rgba([210, 210, 210, 255]);
        let default_bg = Rgba([12, 12, 12, 255]);
        // A blank reverse run (an inter-word gap) over the transparent canvas fills
        // its cell with the bar colour (default_fg).
        let gap = px_text_grid_item(" ", 1, 0, 0);
        let chrome: Vec<&PositionedWindow> = vec![&gap];
        let c = build_chrome_canvas(&chrome, (8, 8), default_fg, default_bg, &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(*c.get_pixel(x, y), default_fg, "gap cell filled with the bar colour at ({x},{y})");
            }
        }
        // An inked reverse glyph paints the bar (default_fg) with dark (default_bg) ink.
        let glyph = px_text_grid_item("X", 1, 0, 0);
        let chrome: Vec<&PositionedWindow> = vec![&glyph];
        let c = build_chrome_canvas(&chrome, (8, 8), default_fg, default_bg, &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        assert!(
            (0..8).any(|x| (0..8).any(|y| *c.get_pixel(x, y) == default_fg)),
            "the highlight bar (default_fg) is painted behind the glyph"
        );
        assert!(
            (0..8).any(|x| (0..8).any(|y| *c.get_pixel(x, y) == default_bg)),
            "the glyph ink is drawn in default_bg (dark on the bright bar)"
        );
    }

    #[test]
    fn px_text_reverse_with_explicit_colours_paints_the_swapped_block() {
        // A run whose game explicitly chose colours DOES paint the swap block.
        let win = px_text_grid_item(" ", 1, RED, BLUE);
        let chrome: Vec<&PositionedWindow> = vec![&win];
        let c = build_chrome_canvas(&chrome, (8, 8), Rgba([1, 1, 1, 255]), Rgba([2, 2, 2, 255]), &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        assert_eq!(c.get_pixel(4, 4)[3], 255, "explicit reverse paints an opaque block");
    }

    #[test]
    fn px_text_no_bg_stays_transparent_without_reverse() {
        // Regression guard: a run with no explicit bg (0/Default) and no
        // reverse style stays transparent — unchanged from before colour
        // handling existed, so frame art under status text still shows through.
        let win = px_text_grid_item(" ", 0, RED, 0);
        let chrome: Vec<&PositionedWindow> = vec![&win];
        let c = build_chrome_canvas(&chrome, (8, 8), Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255]), &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(c.get_pixel(x, y)[3], 0, "no bg, no reverse ⇒ transparent at ({x},{y})");
            }
        }
    }

    // ── explicit-bg status-band flood (SQ-0519) ─────────────────────────────

    #[test]
    fn row_flood_bg_predicate_first_explicit_wins_and_skips_reverse() {
        // The window-wide flood predicate (raster twin of SQ-0512's hybrid per-row
        // flood): a NON-reverse row that names an explicit bg floods with it; a pure
        // reverse-video row and a row with no explicit bg do NOT (byte-identical).
        let colors = colors();
        let default_bg = Rgba([9, 9, 9, 255]);
        let z_black = (1u32 << 24) | 2; // Standard 2 (explicit)
        let z_white = (1u32 << 24) | 9; // Standard 9 (explicit) → spec white
        let run = |x: u16, style: u8, fg: u32, bg: u32| {
            crate::engine::PxText::derived(1, x, "AB".into(), style, fg, bg, zvm::screen::V6Cell::DEFAULT)
        };
        // (a) explicit-bg non-reverse row → floods the resolved white.
        let a = run(1, 0, z_black, z_white);
        let b = run(50, 0, z_black, z_white);
        assert_eq!(
            row_flood_bg(&[&a, &b], default_bg, &colors),
            Some(Rgba([255, 255, 255, 255])),
            "explicit-bg row floods z-colour 9 white"
        );
        // (b) pure reverse-video, non-explicit row (Zork0's on-art ribbon) → None:
        // fill_reverse_row_gaps owns it (with the over-art gate).
        let rev = run(1, 1, 0, 0);
        assert_eq!(row_flood_bg(&[&rev], default_bg, &colors), None, "reverse row: no window flood");
        // (c) mixed partial-explicit row → first-explicit-wins (the second run's white).
        let plain = run(1, 0, 0, 0);
        let white = run(50, 0, z_black, z_white);
        assert_eq!(
            row_flood_bg(&[&plain, &white], default_bg, &colors),
            Some(Rgba([255, 255, 255, 255])),
            "mixed row floods the first explicit bg"
        );
        // (d) explicit-FG-only, non-reverse row (Zork0's compass letters) → None: no
        // explicit bg means no opaque box painted over the banner art.
        let fg_only = run(1, 0, z_black, 0);
        assert_eq!(row_flood_bg(&[&fg_only], default_bg, &colors), None, "explicit-fg-only row: no window flood");
    }

    fn band_grid(w_px: u16, runs: Vec<PxText>) -> PositionedWindow {
        PositionedWindow {
            x: 0, y: 0, w: (w_px / 8).max(1), h: 1, x_px: 0, y_px: 0, w_px, h_px: 16, left_margin: 0, right_margin: 0,
            node: WinNode::Grid(GridWindow {
                win: 0,
                fill: None,
                cols: (w_px / 8).max(1), rows: 1, cells: vec![], active_rows: 1, cursor: (0, 0), cursor_active: false,
                border: BorderPref::Unspecified, bg: None, fg: None, reverse: false, px_texts: runs,
            }),
        }
    }

    #[test]
    fn explicit_bg_status_row_floods_the_whole_window_width() {
        // SQ-0519: two explicit black-on-white runs with a bare gap between them —
        // the gap (and the whole window width) floods the explicit white, so the band
        // reads as one solid bar rather than showing the page between the runs.
        let z_black = (1u32 << 24) | 2;
        let z_white = (1u32 << 24) | 9;
        let win = band_grid(64, vec![
            crate::engine::PxText::derived(1, 1, "AB".into(), 0, z_black, z_white, zvm::screen::V6Cell::DEFAULT),
            crate::engine::PxText::derived(1, 41, "CD".into(), 0, z_black, z_white, zvm::screen::V6Cell::DEFAULT),
        ]);
        let c = build_chrome_canvas(&[&win], (64, 16), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        // px 24 is a gap between run A (px 0..16) and run C (px 40..): flooded white.
        assert_eq!(*c.get_pixel(24, 8), Rgba([255, 255, 255, 255]), "the inter-run gap floods the explicit white");
        // The window's far edge is flooded too — the whole window width is one bar.
        assert_eq!(*c.get_pixel(60, 8), Rgba([255, 255, 255, 255]), "the flood spans the full window width");
    }

    /// SQ-0784: the other half of the same decision — a row whose runs do NOT reach
    /// the window's edges is not a bar, so the ground BETWEEN two of its runs is the
    /// window's and must survive.
    ///
    /// The pattern is scopa's end-of-hand score screen, in its own coordinates: one
    /// full-screen 640x400 grid printing `Denari` at native x 154 and `Primiera` at
    /// 466 on the row at y 86, and `8`/`72` and `2`/`84` on the rows at 145 and 239 —
    /// with the game's two blue card panels drawn either side of a green divider at
    /// native 350..360. The hull flood bridged all three rows straight through that
    /// divider (153..529 and 13..385), which is the report.
    ///
    /// FALSIFY by restoring the hull flood (`let (fx, fe) = if spans_window { .. }
    /// else { (lo, hi) };` and one `fill_cell`): every one of the three rows fails
    /// with "the divider between scopa's two card panels keeps the window's own
    /// ground" — the gap painted the panel blue. The contrast that keeps this from
    /// being a blanket "never bridge" is the test above, which must keep passing:
    /// a row that DOES reach both window edges still floods gap and edges alike.
    #[test]
    fn separated_runs_keep_the_window_ground_between_them() {
        // scopa's white-on-blue: true-colour packed, 15-bit 0x59A0 -> Rgb(0,107,181).
        let ink = 0x0200_7FFF;
        let panel = 0x0200_59A0;
        let blue = Rgba([0, 107, 181, 255]);
        let px = |x: u16, y: u16, s: &str| {
            crate::engine::PxText::derived(y, x, s.into(), 0, ink, panel, zvm::screen::V6Cell::DEFAULT)
        };
        let win = PositionedWindow {
            x: 0, y: 0, w: 80, h: 25, x_px: 0, y_px: 0, w_px: 640, h_px: 400,
            left_margin: 0, right_margin: 0,
            node: WinNode::Grid(GridWindow {
                win: 0,
                fill: None,
                cols: 80, rows: 25, cells: vec![], active_rows: 25, cursor: (0, 0), cursor_active: false,
                border: BorderPref::Unspecified, bg: None, fg: None, reverse: false,
                px_texts: vec![
                    px(154, 86, "Denari"), px(466, 86, "Primiera"),
                    px(14, 145, "8"), px(370, 145, "72"),
                    px(14, 239, "2"), px(370, 239, "84"),
                ],
            }),
        };
        let c = build_chrome_canvas(&[&win], (640, 400), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        for (row, (left, lx), (right, rx)) in [
            (86u32, ("Denari", 153u32), ("Primiera", 465u32)),
            (145, ("8", 13), ("72", 369)),
            (239, ("2", 13), ("84", 369)),
        ] {
            // The last scanline of the 16-pixel text cell: below every glyph in this
            // row, so what it shows is the background alone.
            let y = row - 1 + 15;
            assert_eq!(
                c.get_pixel(355, y)[3], 0,
                "row {row} ({left} -> {right}): the divider between scopa's two card panels keeps \
                 the window's own ground, so the flood must not bridge the gap"
            );
            // …and the runs themselves still carry the panel colour they named.
            assert_eq!(*c.get_pixel(lx, y), blue, "row {row}: the left run's own cells still flood {left}'s bg");
            assert_eq!(*c.get_pixel(rx, y), blue, "row {row}: the right run's own cells still flood {right}'s bg");
        }
    }

    #[test]
    fn explicit_fg_only_run_over_art_is_not_flooded() {
        // SQ-0519 byte-identity guard: Zork0's compass letters are explicit-FG-only,
        // non-reverse, ON opaque banner art. With no explicit bg the flood must NOT
        // fire — an art pixel beside the letter keeps its value (no black box).
        let z_red = (1u32 << 24) | 3;
        let art_color = Rgba([180, 140, 90, 255]);
        let art = graphics_window(0, 0, 16, 16, image::RgbaImage::from_pixel(16, 16, art_color));
        let letter = band_grid(16, vec![crate::engine::PxText::derived(1, 1, "N".into(), 0, z_red, 0, zvm::screen::V6Cell::DEFAULT)]);
        let c = build_chrome_canvas(&[&art, &letter], (16, 16), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        // px 12 is the second cell (no ink, no run) — the banner art shows through.
        assert_eq!(*c.get_pixel(12, 8), art_color, "explicit-fg-only run leaves the banner art (no bg flood)");
    }

    // ── per-window page fill (SQ-0704, ZMSD §8.8.3.2) ───────────────────────

    /// A chrome grid window at `(0,0)` covering `w × h` native pixels, carrying
    /// `bg` as its own Normal-style background.
    fn page_grid(w: u16, h: u16, bg: Option<u32>) -> PositionedWindow {
        PositionedWindow {
            x: 0, y: 0, w: (w / 8).max(1), h: (h / 16).max(1), x_px: 0, y_px: 0, w_px: w, h_px: h,
            left_margin: 0, right_margin: 0,
            node: WinNode::Grid(GridWindow {
                win: 0,
                fill: None,
                cols: (w / 8).max(1), rows: (h / 16).max(1), cells: vec![], active_rows: 1,
                cursor: (0, 0), cursor_active: false, border: BorderPref::Unspecified,
                bg, fg: None, reverse: false, px_texts: Vec::new(),
            }),
        }
    }

    #[test]
    fn window_page_fills_only_the_holes_and_leaves_art_alone() {
        // A window whose art covers the top half only: the untouched bottom half
        // becomes the window's own page, the art stays byte-for-byte.
        let art_color = Rgba([180, 140, 90, 255]);
        let art = graphics_window(0, 0, 16, 8, image::RgbaImage::from_pixel(16, 8, art_color));
        let win = page_grid(16, 16, Some(BLUE));
        let chrome = [&art, &win];
        let mut c = build_chrome_canvas(&chrome, (16, 16), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        assert_eq!(c.get_pixel(4, 12)[3], 0, "precondition: the window's lower half is unpainted");
        fill_window_pages(&mut c, &chrome, None, &colors(), TextLayer::All, zvm::screen::V6Cell::DEFAULT);
        assert_eq!(*c.get_pixel(4, 12), Rgba([0, 0, 255, 255]), "an unpainted pixel takes the window's own page");
        assert_eq!(*c.get_pixel(4, 4), art_color, "artwork is never repainted");
    }

    #[test]
    fn window_with_no_page_of_its_own_keeps_todays_transparency() {
        let win = page_grid(16, 16, None);
        let chrome = [&win];
        let mut c = build_chrome_canvas(&chrome, (16, 16), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        let before = c.as_raw().clone();
        fill_window_pages(&mut c, &chrome, None, &colors(), TextLayer::All, zvm::screen::V6Cell::DEFAULT);
        assert_eq!(*c.as_raw(), before, "a window the game gave no colour is left exactly as before");
    }

    #[test]
    fn a_window_overlapping_the_story_box_is_skipped() {
        // Zork Zero's window 7 carries the same page across the WHOLE screen;
        // filling it would flood the hybrid transcript viewport and defeat
        // `story_clear_native`'s clear-interior probe.
        let full = page_grid(16, 16, Some(BLUE));
        let story = PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 4, y_px: 4, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
        };
        let chrome = [&full];
        let mut c = build_chrome_canvas(&chrome, (16, 16), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        fill_window_pages(&mut c, &chrome, Some(&story), &colors(), TextLayer::All, zvm::screen::V6Cell::DEFAULT);
        assert_eq!(c.get_pixel(8, 8)[3], 0, "the story box stays clear for the transcript");
        assert_eq!(c.get_pixel(0, 0)[3], 0, "and the covering window is skipped whole, not clipped");
    }

    #[test]
    fn an_inherited_colour_is_not_a_page_choice() {
        // Standard 0/1 ("current"/"default", ZMSD §8.3.1) are inheritance, not a
        // colour the game named — `packed_explicit` rejects them.
        let win = page_grid(16, 16, Some(1u32 << 24)); // Standard(0)
        let chrome = [&win];
        let mut c = build_chrome_canvas(&chrome, (16, 16), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        fill_window_pages(&mut c, &chrome, None, &colors(), TextLayer::All, zvm::screen::V6Cell::DEFAULT);
        assert_eq!(c.get_pixel(4, 4)[3], 0, "an inherited colour leaves the window's page to the host");
    }

    // ── declined colours: a PAINTED window keeps its page (SQ-0716) ─────────

    /// A painted ground with one opaque pixel at `(px, py)` of a `w × h` surface.
    fn ground(w: u32, h: u32, px: u32, py: u32) -> image::RgbaImage {
        let mut g = image::RgbaImage::new(w, h);
        g.put_pixel(px, py, Rgba([255, 0, 0, 255]));
        g
    }

    #[test]
    fn a_painted_window_keeps_its_page_with_colours_declined() {
        // scopa's shape: the game drew inside this window, so its declared page is
        // the ground of that drawing rather than a palette preference.
        let win = page_grid(16, 16, Some(BLUE));
        let chrome = [&win];
        let mut c = build_chrome_canvas(&chrome, (16, 16), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        fill_painted_window_pages(&mut c, &chrome, None, &colors(), Some(&ground(16, 16, 4, 4)), zvm::screen::V6Cell::DEFAULT);
        assert_eq!(*c.get_pixel(10, 10), Rgba([0, 0, 255, 255]), "the painted window's page arrives anyway");
    }

    #[test]
    fn an_unpainted_window_still_declines_its_page() {
        // The flag keeps its meaning for every window the game only coloured:
        // Zork Zero, Arthur, Shogun, Journey and advent paint no ground at all.
        let win = page_grid(16, 16, Some(BLUE));
        let chrome = [&win];
        let mut c = build_chrome_canvas(&chrome, (16, 16), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        let before = c.as_raw().clone();
        // A ground that exists but lies entirely outside this window's box.
        let mut g = image::RgbaImage::new(64, 64);
        g.put_pixel(40, 40, Rgba([255, 0, 0, 255]));
        fill_painted_window_pages(&mut c, &chrome, None, &colors(), Some(&g), zvm::screen::V6Cell::DEFAULT);
        assert_eq!(*c.as_raw(), before, "a window the game never drew into keeps the host page");
    }

    #[test]
    fn no_painted_ground_at_all_changes_nothing() {
        let win = page_grid(16, 16, Some(BLUE));
        let chrome = [&win];
        let mut c = build_chrome_canvas(&chrome, (16, 16), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        let before = c.as_raw().clone();
        fill_painted_window_pages(&mut c, &chrome, None, &colors(), None, zvm::screen::V6Cell::DEFAULT);
        assert_eq!(*c.as_raw(), before, "no ground, no exception");
    }

    #[test]
    fn a_painted_window_over_the_story_box_is_still_skipped() {
        // The story window's page and ink are the reading surface: they are the
        // pair `honor_game_colours` governs, painted ground or not.
        let full = page_grid(16, 16, Some(BLUE));
        let story = PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 4, y_px: 4, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
        };
        let chrome = [&full];
        let mut c = build_chrome_canvas(&chrome, (16, 16), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors(), TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        fill_painted_window_pages(&mut c, &chrome, Some(&story), &colors(), Some(&ground(16, 16, 4, 4)), zvm::screen::V6Cell::DEFAULT);
        assert_eq!(c.get_pixel(0, 0)[3], 0, "the story-overlapping window is skipped whole, exactly as when colours are honoured");
    }

    // ── story region background fill (Lane C) ───────────────────────────────

    #[test]
    fn story_bg_rgba_resolves_the_windows_own_colour() {
        let story = PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 0, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, bg: Some(BLUE), ..Default::default() }),
        };
        let color = story_bg_rgba(Some(&story), &colors()).expect("win0 set a bg colour");
        assert_eq!(color, Rgba([0, 0, 255, 255]));
    }

    #[test]
    fn story_bg_rgba_is_none_when_the_game_set_no_colour() {
        let story = PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 0, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
        };
        assert!(story_bg_rgba(Some(&story), &colors()).is_none(), "no game colour ⇒ None (caller leaves it transparent)");
    }

    #[test]
    fn story_bg_rgba_fills_the_clear_interior_rect() {
        // End-to-end through the same calls screen.rs makes: resolve the colour,
        // then fill_cell the story_clear_native rect with it.
        let story = PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 2, y_px: 2, w_px: 4, h_px: 4, left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, bg: Some(RED), ..Default::default() }),
        };
        let mut canvas = RgbaImage::new(8, 8);
        let (sx, sy, sw, sh) = story_clear_native(Some(&story), &canvas).expect("story window present");
        let color = story_bg_rgba(Some(&story), &colors()).expect("bg set");
        fill_cell(&mut canvas, sx, sy, sw, sh, color);
        for y in 2..6 {
            for x in 2..6 {
                assert_eq!(*canvas.get_pixel(x, y), Rgba([255, 0, 0, 255]), "story rect filled red at ({x},{y})");
            }
        }
        assert_eq!(canvas.get_pixel(0, 0)[3], 0, "outside the story rect stays transparent");
    }

    #[test]
    fn flatten_onto_page_only_repaints_fully_transparent_pixels() {
        // SQ-0510: the raster composite's leftover holes become the page, but any
        // pixel a layer touched — however faintly — is left byte-for-byte alone,
        // so frame art, status bands, glyphs and drop-caps can never be covered.
        let page = Rgba([26, 26, 26, 255]);
        let art = Rgba([102, 34, 0, 255]);
        let faint = Rgba([1, 2, 3, 1]); // alpha 1: touched, so untouchable
        let mut canvas = RgbaImage::new(3, 1);
        canvas.put_pixel(0, 0, Rgba([0, 0, 0, 0])); // an untouched hole
        canvas.put_pixel(1, 0, art);
        canvas.put_pixel(2, 0, faint);

        flatten_onto_page(&mut canvas, page);

        assert_eq!(*canvas.get_pixel(0, 0), page, "a fully transparent pixel becomes the page");
        assert_eq!(*canvas.get_pixel(1, 0), art, "an opaque art pixel is never repainted");
        assert_eq!(*canvas.get_pixel(2, 0), faint, "even alpha==1 counts as drawn and survives");
        assert!(canvas.pixels().all(|p| p[3] > 0), "no fully transparent pixel is left behind");
    }

    #[test]
    fn uniform_scale_letterboxes() {
        let scale = uniform_scale((320, 200), (640, 480));
        assert_eq!(scale.s, 2.0);
        assert_eq!(scale.off_x, 0);
        assert_eq!(scale.off_y, 40);
    }

    // ── SQ-0936: the magnification ladder, derived per press ──────────────────

    /// One row of the ladder table, straight out of the corpus: the press, the
    /// art scale `PictSource::art_scale` computes for it, the step that implies,
    /// the native screen the archive's picture space times that scale gives, and
    /// what a 1024x600 device pane resolves to.
    ///
    /// The point of driving all four is that they are NOT the same ladder. A
    /// hardcoded "1x / 1.5x / 2x" is right for the two `(2, 2)` presses and wrong
    /// for the standard Macintosh's mono plate and for EGA, both of which may only
    /// take whole steps — the Mac because its art is already 1:1 on the unit
    /// screen, EGA because its pixels are half-width and a half-step would put an
    /// art pixel on a half device pixel horizontally.
    struct Rung {
        press: &'static str,
        art_scale: (u32, u32),
        step: f32,
        native: (u16, u16),
        /// What [`LADDER_PANE`] resolves to: the free scale, then the rung below
        /// it. Every press is measured at the SAME pane, which is what makes the
        /// four answers a demonstration rather than four unrelated sums — and note
        /// the 320-wide and EGA rows, whose unit screen is the same 640x400 and
        /// whose rungs are 1.5 and 1.0 because their ARTWORK differs.
        free_and_locked: (f32, f32),
    }

    /// One device pane, 1100x700 — a 137x38 terminal at an 8x18 cell, near enough
    /// the panes the other v6 suites sweep, and chosen because all four presses
    /// land strictly between two rungs on it.
    const LADDER_PANE: (u32, u32) = (1100, 700);

    const LADDER: &[Rung] = &[
        // 320x200 art doubled onto the 640x400 unit screen — Blorb, Amiga
        // `Pic.data`, DOS MCGA `.mg1`. free = min(1100/640, 700/400) = 1.71875,
        // and the half-step ladder puts the rung below it at 1.5.
        Rung { press: "320x200 (Blorb/Amiga/MCGA)", art_scale: (2, 2), step: 0.5, native: (640, 400), free_and_locked: (1.71875, 1.5) },
        // The standard Macintosh's monochrome `Pic.data`: a 480x300 picture space
        // drawn 1:1, so the unit screen IS 480x300 and only WHOLE steps are on the
        // ladder. free = min(1100/480, 700/300) = 2.2916667 → 2.
        Rung { press: "Macintosh mono Pic.data", art_scale: (1, 1), step: 1.0, native: (480, 300), free_and_locked: (2.2916667, 2.0) },
        // EGA/CGA `.eg1`/`.cg1`: 640x200 art with half-width pixels, so (1, 2) onto
        // the SAME 640x400 unit screen as the first row. gcd(1, 2) = 1 — whole steps
        // only, because a half step would leave one art pixel half a device pixel
        // wide — so the same free 1.71875 locks to 1.0 here and 1.5 there.
        Rung { press: "EGA/CGA 640x200", art_scale: (1, 2), step: 1.0, native: (640, 400), free_and_locked: (1.71875, 1.0) },
        // Apple II: 140x192 art at (4, 2) — see `PictSource::art_scale` for where
        // that pair comes from — onto a 560x384 screen. gcd(4, 2) = 2, half steps.
        // free = min(1100/560, 700/384) = 1.8229167 → 1.5.
        Rung { press: "Apple II 140x192", art_scale: (4, 2), step: 0.5, native: (560, 384), free_and_locked: (1.8229167, 1.5) },
    ];

    /// The step is `1 / gcd(art_scale)` and nothing else, and each press in the
    /// corpus lands where its own artwork puts it — never on a shared hardcoded
    /// list. FALSIFY by returning a constant `0.5`: the Mac and EGA rows fail.
    #[test]
    fn the_ladder_step_is_derived_from_the_art_scale() {
        for r in LADDER {
            assert_eq!(
                scale_ladder_step(r.art_scale),
                r.step,
                "{}: art_scale {:?} implies step {} — 1/gcd, not a chosen ladder",
                r.press,
                r.art_scale,
                r.step,
            );
        }
    }

    /// Locking snaps the free scale DOWN to the rung below it, per press.
    #[test]
    fn locking_quantizes_the_free_scale_down_to_its_own_rung() {
        let pane = LADDER_PANE;
        for r in LADDER {
            let (free, want) = r.free_and_locked;
            assert!(
                (uniform_scale(r.native, pane).s - free).abs() < 1e-4,
                "{}: {:?} into {pane:?} scales freely by {free}",
                r.press,
                r.native,
            );
            let got = FrameGeometry::new(r.native, r.art_scale, zvm::screen::V6Cell::DEFAULT).locked_scale(pane).expect("1024x600 fits a rung").s;
            assert!(
                (got - want).abs() < 1e-6,
                "{}: free {free} must lock to {want}, got {got}",
                r.press,
            );
        }
    }

    /// **A rung must put the CELL on whole device pixels too, not only an art
    /// pixel** (SQ-1024).
    ///
    /// Stated as a relation over presses and cells rather than as pinned rungs, so
    /// it holds for a machine nobody has declared yet. The Macintosh is the case
    /// that motivated it and it is not special-cased anywhere: `gcd(7, 15) == 1`
    /// falls out of the same arithmetic that gives `gcd(8, 16) == 8`.
    #[test]
    fn a_rung_puts_the_character_cell_on_whole_device_pixels() {
        let cell = |w, h| zvm::screen::V6Cell::new(w, h);
        // The step each combination admits, which is the whole claim in one line.
        let cases = [
            // press,        art_scale, cell,        step
            ("most v6",      (2u32, 2u32), cell(8, 16), 2u32),
            ("Macintosh colour", (2, 2), cell(7, 15), 1),
            ("Macintosh mono",   (1, 1), cell(7, 15), 1),
            ("EGA / CGA",         (1, 2), cell(8, 16), 1),
            ("Apple II",          (4, 2), cell(8, 16), 2),
        ];
        for (who, art, c, want) in cases {
            let geom = FrameGeometry::new((640, 400), art, c);
            assert_eq!(geom.step(), want, "{who}: rungs are multiples of 1/{want}");
        }

        // And the property the step exists for: at every rung a pane can hold, one
        // art pixel AND one cell land on whole device pixels.
        for (who, art, c, _) in cases {
            let geom = FrameGeometry::new((640, 400), art, c);
            for pane in [(640u32, 400u32), (800, 500), (960, 600), (1600, 1200), (900, 337)] {
                let Some(sc) = geom.locked_scale(pane) else { continue };
                for (what, n) in [
                    ("art x", art.0),
                    ("art y", art.1),
                    ("cell w", u32::from(c.w())),
                    ("cell h", u32::from(c.h())),
                ] {
                    let dev = n as f32 * sc.s;
                    assert!(
                        (dev - dev.round()).abs() < 1e-4,
                        "{who} at {pane:?}: s={} puts {what} on {dev} device pixels",
                        sc.s,
                    );
                }
            }
        }
    }

    /// The property the whole mode exists for: at the locked scale one ART pixel
    /// is a whole number of device pixels on BOTH axes. `art_scale.N · s ∈ ℤ` is
    /// the constraint the step was derived from, so this is the derivation checked
    /// from the other end.
    #[test]
    fn a_locked_scale_puts_an_art_pixel_on_whole_device_pixels() {
        for r in LADDER {
            for pane in [(640u32, 400u32), LADDER_PANE, (800, 500), (1600, 1200), (900, 337)] {
                let Some(sc) =
                    FrameGeometry::new(r.native, r.art_scale, zvm::screen::V6Cell::DEFAULT)
                        .locked_scale(pane)
                else {
                    continue;
                };
                for (axis, n) in [("x", r.art_scale.0), ("y", r.art_scale.1)] {
                    let dev = n as f32 * sc.s;
                    assert!(
                        (dev - dev.round()).abs() < 1e-4 && dev >= 1.0,
                        "{} at {pane:?}: s={} puts one art pixel on {dev} device pixels along {axis}",
                        r.press,
                        sc.s,
                    );
                }
            }
        }
    }

    /// A rung never overflows the pane, and the screen it leaves is centred — the
    /// margin is the story's own page (`fill_pane_page`), not dead space.
    #[test]
    fn a_locked_screen_fits_the_pane_and_stays_centred() {
        let pane = LADDER_PANE;
        for r in LADDER {
            let sc = FrameGeometry::new(r.native, r.art_scale, zvm::screen::V6Cell::DEFAULT).locked_scale(pane).expect("a rung fits");
            let (w, h) = (r.native.0 as f32 * sc.s, r.native.1 as f32 * sc.s);
            assert!(w <= pane.0 as f32 && h <= pane.1 as f32, "{}: {w}x{h} overflows {pane:?}", r.press);
            assert_eq!(sc.off_x, ((pane.0 as f32 - w) / 2.0) as u32, "{}: centred horizontally", r.press);
            assert_eq!(sc.off_y, ((pane.1 as f32 - h) / 2.0) as u32, "{}: centred vertically", r.press);
        }
    }

    /// Too small for even the smallest rung → free scaling, never a block and
    /// never a message on the game screen. A 320x200 press's smallest rung is
    /// 0.5, so a pane under 320x200 device pixels has none; `fitted_scale` says
    /// so through its flag, which is what a diagnostic reads.
    #[test]
    fn a_pane_too_small_for_the_smallest_rung_falls_back_to_free_scaling() {
        let native = (640u16, 400u16);
        let tiny = (240u32, 150u32); // free s = 0.375, below the 0.5 rung
        assert!(
            FrameGeometry::new(native, (2, 2), zvm::screen::V6Cell::DEFAULT).locked_scale(tiny).is_none(),
            "no rung fits a 240x150 pane",
        );

        let free = uniform_scale(native, tiny);
        let (got, fell_back) =
            FrameGeometry::new(native, (2, 2), zvm::screen::V6Cell::DEFAULT).fitted_scale(tiny, true);
        assert!(fell_back, "the fallback is reported, not silent");
        assert_eq!(got.s, free.s, "and it IS the free scale — degrade, never block");
        assert_eq!((got.off_x, got.off_y), (free.off_x, free.off_y));

        // The Mac's whole-step ladder has a coarser floor: anything under 1.0.
        let mac = FrameGeometry::new((480, 300), (1, 1), zvm::screen::V6Cell::DEFAULT);
        assert!(mac.locked_scale((470, 600)).is_none(), "0.979 is below the Mac's floor");
        assert!(mac.fitted_scale((470, 600), true).1, "and reports the fallback");
    }

    /// With the mode off nothing changes at all — this is opt-in, and the default
    /// path must stay byte-for-byte `uniform_scale`.
    #[test]
    fn the_mode_is_opt_in_and_off_is_the_free_scale() {
        for r in LADDER {
            for pane in [LADDER_PANE, (784, 666), (240, 150)] {
                let free = uniform_scale(r.native, pane);
                let (got, fell_back) = FrameGeometry::new(r.native, r.art_scale, zvm::screen::V6Cell::DEFAULT).fitted_scale(pane, false);
                assert_eq!(got.s, free.s, "{} at {pane:?}", r.press);
                assert_eq!((got.off_x, got.off_y), (free.off_x, free.off_y));
                assert!(!fell_back, "a mode that was never asked for cannot fall back");
            }
        }
    }

    /// A rung is exactly a rung: `(free · gcd).floor() / gcd` must not lose one to
    /// float error when the free scale is already on the ladder. 1280x800 into
    /// 640x400 is exactly 2.0 and must stay 2.0, not drop to 1.5.
    #[test]
    fn a_free_scale_already_on_the_ladder_is_left_alone() {
        for (pane, want) in [((1280u32, 800u32), 2.0f32), ((640, 400), 1.0), ((320, 200), 0.5), ((960, 600), 1.5)] {
            let got = FrameGeometry::new((640, 400), (2, 2), zvm::screen::V6Cell::DEFAULT)
                .locked_scale(pane)
                .expect("a rung fits")
                .s;
            assert_eq!(got, want, "{pane:?} is exactly {want} and must not round down a step");
        }
    }



    // ── Hybrid render mode: story_viewport_box + chrome_bands ──────────────────

    #[test]
    fn story_viewport_box_maps_win0_box_inward_to_cells() {
        // Native 320×200 game, win0 box (43,39,234,160). Scale 1:1 (native px ==
        // device px), 8 px/cell. Rounding INWARD: left ceil(43/8)=6, top
        // ceil(39/8)=5, right floor((43+234)/8)=floor(277/8)=34,
        // bottom floor((39+160)/8)=floor(199/8)=24 → 28×19 cells at (6,5).
        let story = PositionedWindow { x_px: 43, y_px: 39, w_px: 234, h_px: 160, ..buffer_item(0, true) };
        let scale = uniform_scale((320, 200), (320, 200)); // s = 1.0, no offset
        assert_eq!(scale.s, 1.0);
        let rect = story_viewport_box(Some(&story), &scale, (40, 25), (8, 8));
        assert_eq!(rect, ratatui::layout::Rect { x: 6, y: 5, width: 28, height: 19 });
    }

    #[test]
    fn story_viewport_box_no_story_is_full_pane() {
        let scale = uniform_scale((320, 200), (320, 200));
        let rect = story_viewport_box(None, &scale, (40, 25), (8, 8));
        assert_eq!(rect, ratatui::layout::Rect { x: 0, y: 0, width: 40, height: 25 });
    }

    #[test]
    fn chrome_bands_tile_pane_minus_viewport_without_overlap() {
        use ratatui::layout::Rect;
        let pane = Rect::new(0, 0, 40, 25);
        let viewport = Rect::new(6, 5, 28, 19); // interior, all four edges inset
        let bands = chrome_bands(pane, viewport);
        assert_eq!(bands.len(), 4, "all four edges produce a band");
        // Each band carries its own identity (SQ-0894), so no downstream stage has
        // to infer "is this a flank?" from its width.
        let roles: Vec<BandRole> = bands.iter().map(|(r, _)| *r).collect();
        for want in [BandRole::Top, BandRole::Bottom, BandRole::LeftFlank, BandRole::RightFlank] {
            assert!(roles.contains(&want), "{want:?} missing from {roles:?}");
        }
        assert!(bands.iter().filter(|(r, _)| r.is_flank()).count() == 2, "exactly two flanks");
        // Non-overlap + exact tiling: every pane cell OUTSIDE the viewport is
        // covered exactly once; every viewport cell is covered zero times.
        let mut cover = vec![0u8; (pane.width as usize) * (pane.height as usize)];
        for (_, b) in &bands {
            for y in b.y..b.bottom() {
                for x in b.x..b.right() {
                    cover[y as usize * pane.width as usize + x as usize] += 1;
                }
            }
        }
        for y in 0..pane.height {
            for x in 0..pane.width {
                let inside_vp = (viewport.x..viewport.right()).contains(&x) && (viewport.y..viewport.bottom()).contains(&y);
                let c = cover[y as usize * pane.width as usize + x as usize];
                if inside_vp {
                    assert_eq!(c, 0, "viewport cell ({x},{y}) untouched by chrome bands");
                } else {
                    assert_eq!(c, 1, "chrome cell ({x},{y}) covered exactly once");
                }
            }
        }
    }

    #[test]
    fn chrome_bands_omit_flush_edges() {
        use ratatui::layout::Rect;
        let pane = Rect::new(0, 0, 40, 25);
        // Viewport flush to the left and top edges → only bottom + right bands.
        let viewport = Rect::new(0, 0, 30, 20);
        let bands = chrome_bands(pane, viewport);
        assert_eq!(bands.len(), 2, "left+top flush → those bands omitted");
        assert!(bands.iter().all(|(_, b)| b.x >= 30 || b.y >= 20), "remaining bands are the right/bottom ring");
        // …and they say which sides they are, rather than leaving it to be measured.
        let roles: Vec<BandRole> = bands.iter().map(|(r, _)| *r).collect();
        assert!(roles.contains(&BandRole::Bottom) && roles.contains(&BandRole::RightFlank), "roles: {roles:?}");
    }

    #[test]
    fn chrome_bands_full_viewport_is_empty() {
        use ratatui::layout::Rect;
        let pane = Rect::new(0, 0, 40, 25);
        assert!(chrome_bands(pane, pane).is_empty(), "viewport == pane → no chrome");
    }

    #[test]
    fn chrome_bands_absolute_coords_offset_pane() {
        use ratatui::layout::Rect;
        // A pane not anchored at the origin: bands must tile pane − viewport in the
        // same absolute space (the hybrid path passes absolute rects).
        let pane = Rect::new(10, 4, 20, 12);
        let viewport = Rect::new(13, 6, 12, 6);
        let bands = chrome_bands(pane, viewport);
        assert_eq!(bands.len(), 4);
        for (_, b) in &bands {
            assert!(b.x >= pane.x && b.right() <= pane.right() && b.y >= pane.y && b.bottom() <= pane.bottom(),
                "band {b:?} stays inside the pane");
        }
    }
}
