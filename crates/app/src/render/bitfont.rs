//! Embedded bitmap fonts, rasterized into an RGBA canvas for the v6 pixel
//! composite (Phase 1c). Glyphs are scaled to fill a `cw × ch` device-pixel
//! cell so text stays legible at terminal cell sizes (~9×19).
//!
//! ## Three faces, and which one draws
//!
//! [`crate::render::misc7x14`] — X11 misc-fixed, natively **7×14** — draws a cell
//! that is 7 wide, which is the Macintosh's and no other machine's (SQ-1016).
//! [`crate::render::vga16`] — Uni-VGA, natively **8×16** — draws every other cell,
//! and [`glyph_bits`]'s 8×8 chain below is the fallback for what neither carries
//! (they carry the same 194 text codepoints, and no font 3 at all).
//! The cell is **the machine's** (SQ-0917): 7×15 on a Macintosh, 8×16 elsewhere,
//! and a release's own proportional face states its own line height on top of that
//! (SQ-1009). At 8×16 the 16-row face samples **1:1** while an 8×8 master has to
//! double each row into it. This paragraph used to say 8×16 was the cell "at every
//! production call site", which was true until the Macintosh declared its own and
//! is contradicted 200 lines below by [`blit_glyph_styled`]'s own account of
//! SQ-0917. `v6_layout`'s `FONT_W`/`FONT_H` are still bare 8/16 and documented
//! there as a test convenience — they are not the cell. Left standing, a sentence
//! like that is how `py + 16` gets written next, which is SQ-1020 exactly. That doubling was the state of things from SQ-0450
//! until SQ-0932, and this module's opening line used to call a taller font
//! "future work".
//!
//! The split is TEXT versus **font 3**. Uni-VGA carries letters — ASCII, Latin-1,
//! `Œ`/`œ` — and nothing else; box drawing, block elements, the cursor arrows, the
//! APL quad and the runes all stay on the 8×8 masters, because font 3 is a
//! graphics character set rather than a typeface and no story prints it as text.
//! [`crate::render::vga16`]'s docs carry the full argument, including the pixel it
//! costs Journey's frame border to get this wrong.
//!
//! **The two faces pack their bits in opposite directions** — `font8x8` is
//! LSB-leftmost, `vga16` is MSB-leftmost (BDF's order, and `blorb`'s). Sampling
//! one with the other's rule mirrors the glyph rather than corrupting it, so it
//! still looks like a font; every place that reads a row says which order it is
//! reading, and [`synthesize_face`] / [`synthesize_face16`] are a pair for the
//! same reason.
//!
//! ## Provenance
//!
//! The 8×16 face is Uni-VGA, under the X licence, and the 7×14 one is Markus
//! Kuhn's public-domain X11 misc-fixed — see `crates/app/assets/README.md` for
//! their origins, their exact terms and how the subsets beside them were cut. Both
//! were drawn from scratch rather than traced off silicon; no dumped ROM font is
//! used anywhere in lanthorn.
//!
//! The 8×8 chain is the `font8x8` crate (`BASIC_FONTS`, `LATIN_FONTS`,
//! `BOX_FONTS`, `BLOCK_FONTS`) — a CC0/public-domain font ported from
//! `https://github.com/dhepper/font8x8`, itself extracted from a public-domain
//! `.asm` source.
//!
//! `font8x8` doesn't cover every codepoint v6 titles actually print, though:
//! BeyondZork's "font 3" (see `zvm::cpu::exec::font3_translate`) emits cursor
//! arrows, an APL quad, and ~26 decorative "atmosphere" runic codepoints, and
//! the ZSCII default Unicode table (155–223, ZMSD §3.8.5) includes `œ`/`Œ`
//! which fall outside `font8x8`'s Latin-1-only extended-Latin set.
//! Note that this file is reached only from the **v6** paths (`screen.rs` and
//! `v6_layout.rs`). BeyondZork — the game that actually prints runes — is v5, so
//! ITS runes come from the terminal's own font via the codepoints
//! `font3_translate` returns and never from here. Font 3 does still reach this
//! file, though: *Journey* is v6 and ships `Char.data`, which is **byte-identical**
//! to BeyondZork's `Graphic.Data` (sha1 `ae8977231608`) — the same 8×8 font-3
//! font — so its box-drawing half is live in the raster path. The runic half is
//! the part no known title exercises here (SQ-0915).
//!
//! `EXTRA_GLYPHS` below supplies ORIGINAL 8×8 bitmaps for exactly those gaps,
//! hand-drawn for this pass — not sourced from any font, ROM, or existing
//! artwork. The runic entries are stylised angular stem-and-branch
//! placeholders (a full-height vertical stave plus a distinct combination of
//! diagonal/bar strokes per codepoint): each reads as "a rune" and is
//! visually distinct from the others, but this is NOT a claim of scholarly
//! Elder Futhark letterform accuracy — treat them as decorative placeholders,
//! matching their in-game use as unreadable "atmosphere" text.

use font8x8::UnicodeFonts;
use image::{Rgba, RgbaImage};

/// Original 8×8 bitmaps for codepoints `font8x8` doesn't ship (see module
/// docs for provenance and design notes). Checked only after `font8x8`'s
/// built-in sets in [`glyph_bits`], since this is a short, hand-authored
/// fallback list, not a general-purpose font.
static EXTRA_GLYPHS: &[(char, [u8; 8])] = &[
    ('\u{2190}', [0x00, 0x08, 0x0C, 0x7E, 0x0C, 0x08, 0x00, 0x00]), // ← cursor left (BeyondZork font 3)
    ('\u{2192}', [0x00, 0x10, 0x30, 0x7E, 0x30, 0x10, 0x00, 0x00]), // → cursor right
    ('\u{2191}', [0x08, 0x1C, 0x3E, 0x08, 0x08, 0x08, 0x08, 0x00]), // ↑ cursor up
    ('\u{2193}', [0x00, 0x08, 0x08, 0x08, 0x08, 0x3E, 0x1C, 0x08]), // ↓ cursor down
    ('\u{2195}', [0x08, 0x1C, 0x08, 0x08, 0x08, 0x08, 0x1C, 0x08]), // ↕ cursor up/down
    ('\u{2395}', [0x00, 0x7E, 0x42, 0x42, 0x42, 0x42, 0x7E, 0x00]), // ⎕ APL quad (font 3 code 95)
    ('\u{FFFD}', [0xFF, 0x81, 0xBD, 0xA5, 0xA5, 0xBD, 0x81, 0xFF]), // unknown-glyph placeholder box
    ('\u{0153}', [0x00, 0x00, 0x36, 0x49, 0x79, 0x09, 0x76, 0x00]), // œ (ZSCII default table)
    ('\u{0152}', [0x00, 0x76, 0x09, 0x39, 0x09, 0x09, 0x76, 0x00]), // Œ (ZSCII default table)
    // BeyondZork font-3 "atmosphere" runic placeholders (codes 97–122).
    ('\u{16AA}', [0x14, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04]),
    ('\u{16D2}', [0x05, 0x06, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04]),
    ('\u{16C7}', [0x14, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0C, 0x14]),
    ('\u{16DE}', [0x05, 0x06, 0x04, 0x04, 0x04, 0x04, 0x06, 0x05]),
    ('\u{16D6}', [0x04, 0x04, 0x04, 0x3C, 0x04, 0x04, 0x04, 0x04]),
    ('\u{16A0}', [0x04, 0x04, 0x04, 0x07, 0x04, 0x04, 0x04, 0x04]),
    ('\u{16B7}', [0x14, 0x0C, 0x04, 0x1C, 0x04, 0x04, 0x04, 0x04]),
    ('\u{16BB}', [0x04, 0x04, 0x0C, 0x14, 0x0C, 0x04, 0x04, 0x04]),
    ('\u{16C1}', [0x04, 0x04, 0x06, 0x05, 0x06, 0x04, 0x04, 0x04]),
    ('\u{16C4}', [0x04, 0x0C, 0x04, 0x14, 0x04, 0x04, 0x0C, 0x04]),
    ('\u{16E6}', [0x04, 0x06, 0x04, 0x05, 0x04, 0x04, 0x06, 0x04]),
    ('\u{16DA}', [0x1C, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x1C]),
    ('\u{16D7}', [0x07, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x07]),
    ('\u{16BE}', [0x04, 0x04, 0x1C, 0x04, 0x04, 0x1C, 0x04, 0x04]),
    ('\u{16A9}', [0x04, 0x0C, 0x14, 0x04, 0x14, 0x0C, 0x04, 0x04]),
    ('\u{15BE}', [0x04, 0x0C, 0x14, 0x0C, 0x14, 0x0C, 0x04, 0x04]),
    ('\u{16B3}', [0x04, 0x04, 0x04, 0x7C, 0x04, 0x04, 0x04, 0x04]),
    ('\u{16B1}', [0x14, 0x0C, 0x04, 0x14, 0x04, 0x04, 0x0C, 0x14]),
    ('\u{16CB}', [0x04, 0x04, 0x0E, 0x14, 0x0E, 0x04, 0x04, 0x04]),
    ('\u{16CF}', [0x15, 0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04]),
    ('\u{16A2}', [0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E, 0x15]),
    ('\u{16E0}', [0x14, 0x0C, 0x04, 0x1C, 0x04, 0x04, 0x0C, 0x14]),
    ('\u{16B9}', [0x04, 0x14, 0x04, 0x14, 0x04, 0x14, 0x04, 0x04]),
    ('\u{16C9}', [0x14, 0x04, 0x14, 0x04, 0x14, 0x04, 0x14, 0x04]),
    ('\u{16A5}', [0x04, 0x06, 0x04, 0x06, 0x04, 0x06, 0x04, 0x04]),
    ('\u{16DF}', [0x14, 0x0C, 0x04, 0x3C, 0x04, 0x04, 0x04, 0x04]),
];

/// Look up the 8×8 bitmap for `glyph`. Tries `font8x8`'s basic-Latin,
/// Latin-1-supplement, box-drawing, and block-element sets (all CC0,
/// covering ASCII, the ZSCII default accented-character table, and
/// BeyondZork's font-3 box/block glyphs) in order, then falls back to
/// [`EXTRA_GLYPHS`] for the handful of codepoints those sets don't cover.
pub(crate) fn glyph_bits(glyph: char) -> Option<[u8; 8]> {
    font8x8::BASIC_FONTS
        .get(glyph)
        .or_else(|| font8x8::LATIN_FONTS.get(glyph))
        .or_else(|| font8x8::BOX_FONTS.get(glyph))
        .or_else(|| font8x8::BLOCK_FONTS.get(glyph))
        .or_else(|| EXTRA_GLYPHS.iter().find(|&&(c, _)| c == glyph).map(|&(_, bits)| bits))
}

/// Whether either master in this module carries `glyph` — the 8×16 face or the
/// 8×8 one [`blit_glyph`] falls back to.
///
/// **The 7×14 face is deliberately not a third term** (SQ-1016). It carries the
/// same 194 codepoints as [`crate::render::vga16`] — the two subsets were cut to
/// the same ranges, and `the_narrow_face_can_never_widen_this_answer` pins that
/// they are equal — so asking it could not change any answer, only suggest to a
/// reader that it might. Should a future regeneration widen it, that case fails
/// and this is the decision to revisit.
///
/// [`blit_glyph`] paints a blank cell for anything it does not have, which is the
/// right thing to DRAW and a terrible thing to be unable to ASK about: a caller
/// that hands this module the glyphs a text face declined has no other way to
/// learn that the fallback declined them too, and a missing glyph is silent
/// either way (SQ-0963). The gallery tool asks, and names what fell through both.
pub fn has_glyph(glyph: char) -> bool {
    crate::render::vga16::glyph(glyph).is_some() || glyph_bits(glyph).is_some()
}

/// Read one bit from an 8×8 bitmap, clamping out-of-range `row`/`col` to the
/// nearest edge pixel (so [`scale2x`]'s neighbour rule degrades to plain
/// replication at the glyph's border instead of treating off-canvas space as
/// background).
fn bit_at(bits: &[u8; 8], row: i32, col: i32) -> bool {
    let r = row.clamp(0, 7) as usize;
    let c = col.clamp(0, 7) as usize;
    bits[r] & (1 << c) != 0
}

/// Upscale an 8×8 monochrome bitmap to 16×16 using the Scale2x/AdvMAME2x edge
/// rule: each destination corner takes its diagonal source neighbour's value
/// only when that neighbour agrees with one adjacent side and disagrees with
/// the other. That rounds the re-entrant corners nearest-neighbour leaves
/// stair-stepped on diagonal strokes (box-drawing diagonals, letter serifs),
/// while flat/orthogonal regions fall back to the source pixel unchanged.
/// Pixel-deterministic — no blending, no alpha, same input always yields the
/// same output.
fn scale2x(bits: &[u8; 8]) -> [[bool; 16]; 16] {
    let mut out = [[false; 16]; 16];
    for row in 0..8i32 {
        for col in 0..8i32 {
            let p = bit_at(bits, row, col);
            let a = bit_at(bits, row - 1, col); // north
            let b = bit_at(bits, row, col + 1); // east
            let c = bit_at(bits, row, col - 1); // west
            let d = bit_at(bits, row + 1, col); // south
            let e0 = if c == a && c != d && a != b { a } else { p };
            let e1 = if a == b && a != c && b != d { b } else { p };
            let e2 = if d == c && c != a && d != b { c } else { p };
            let e3 = if d == b && b != a && d != c { b } else { p };
            let (r0, c0) = (row as usize * 2, col as usize * 2);
            out[r0][c0] = e0;
            out[r0][c0 + 1] = e1;
            out[r0 + 1][c0] = e2;
            out[r0 + 1][c0 + 1] = e3;
        }
    }
    out
}

/// ZMSD §8.7.1 style bit for bold, as the model packs it (see
/// [`crate::engine::PxText::style`] / [`crate::engine::GridCell::style`] and
/// `screen::v6_run_style`, which reads the same numbering for the cell paths).
pub(crate) const STYLE_BOLD: u8 = 2;
/// ZMSD §8.7.1 style bit for italic (see [`STYLE_BOLD`]).
const STYLE_ITALIC: u8 = 4;

/// Synthesize a bold / italic / bold-italic face from the roman 8×8 master
/// (SQ-0540). The raster path has ONE face, so emphasis is faked the way bitmap
/// terminals always faked it — and it is done here, in FONT space, BEFORE the
/// glyph is scaled into its `cw × ch` cell, for two reasons:
///
/// * **Scale.** A v6 cell is the machine's — 8×16 on most, 7×15 on a Macintosh
///   (SQ-0917) — and
///   [`blit_glyph`] maps font column `c` to device column `dx` with `col = dx*8/cw`
///   — at `cw == 8` that is 1:1, so one font pixel IS one device pixel horizontally
///   and a one-column shift is exactly the classic one-device-pixel double-strike.
///   (Vertically the same cell doubles each font row, which the shifts don't touch.)
///   At any other cell width the shift scales with the glyph's own stroke width,
///   which is what you want: a 2× cell gets a 2 px double-strike over 2 px strokes,
///   not a hairline. Shifting in device space instead would need per-`cw` tuning to
///   stay proportional.
/// * **Clipping.** Each row is a `u8` with bit `n` = column `n`, so shifting LEFT
///   moves the glyph RIGHT and the `u8` truncation drops anything past column 7 —
///   the transform physically cannot bleed into the neighbouring cell, whatever
///   the caller's geometry. The 16×16 [`scale2x`] path then smooths the
///   already-emboldened/sheared shape rather than fighting it.
///
/// **Bold** double-strikes: `row | row << 1`, i.e. the glyph OR'd with itself one
/// pixel right. Purely additive — a bold glyph is a superset of its roman self, so
/// nothing (not even a full-width box-drawing row) can be thinned or lost.
///
/// **Italic** shears one step: the top half (font rows 0–3) moves one pixel right,
/// the bottom half (rows 4–7) stays put, so the stem leans forward across the
/// x-height. One step is the maximum the cell affords — the Latin masters fill
/// columns 0–6, leaving exactly one column of headroom, and a 2-step shear would
/// push their right edge out of the 8 px cell (a wide box-drawing glyph, columns
/// 0–7, does lose its last column under italic; italic box art isn't a thing any
/// v6 title does).
///
/// Bits 1 (reverse) and 8 (fixed-pitch) are deliberately IGNORED: reverse is a
/// fg/bg swap the caller has already resolved (see `build_chrome_canvas`'s
/// px-run branch), and the bitmap font is fixed-pitch by construction. Passing a
/// run's raw style byte through is therefore safe and never double-applies.
fn synthesize_face(bits: [u8; 8], style: u8) -> [u8; 8] {
    let mut out = bits;
    if style & STYLE_ITALIC != 0 {
        for row in out.iter_mut().take(4) {
            *row <<= 1;
        }
    }
    if style & STYLE_BOLD != 0 {
        for row in out.iter_mut() {
            *row |= *row << 1;
        }
    }
    out
}

/// [`synthesize_face`] for the 16-row master (SQ-0932). Same two transforms and
/// the same reasoning — read that first; this documents only what differs.
///
/// **The shifts go the other way.** [`crate::render::vga16`] packs rows
/// MSB-leftmost, so moving a glyph one column RIGHT is `>> 1`, not `<< 1`. The
/// clipping guarantee survives the flip intact: the bit shifted out of a `u8` is
/// column 7's, the rightmost, so an emboldened or sheared glyph still physically
/// cannot bleed into the next cell.
///
/// **Italic shears at row 8**, the midpoint of the 16-row box, where the 8-row
/// master shears at row 4 — the same fraction of the glyph, and it lands near the
/// x-height on a face whose baseline is row 12 (`FONT_ASCENT 12`, `FONT_DESCENT 4`).
///
/// **What the shear clips is worth naming, because this face is wider.** The 8×8
/// masters fill columns 0–6 and always have a spare column to lean into. Uni-VGA
/// reserves column 7 as its inter-character gap too, but four Latin glyphs do
/// reach it — `*`, `_`, `©` and `¶` — and so do 95 of the 150 box/block glyphs.
/// Under italic those lose their rightmost column. That is the same trade the
/// 8-row path already makes for box art, on a handful more glyphs, and italic
/// box art is not something any v6 title does.
fn synthesize_face16(bits: [u8; 16], style: u8) -> [u8; 16] {
    let mut out = bits;
    if style & STYLE_ITALIC != 0 {
        for row in out.iter_mut().take(8) {
            *row >>= 1;
        }
    }
    if style & STYLE_BOLD != 0 {
        for row in out.iter_mut() {
            *row |= *row >> 1;
        }
    }
    out
}

/// [`synthesize_face16`] for the 14-row, 7-wide face (SQ-1016). Same two
/// transforms, same MSB-leftmost direction — read that first; this documents only
/// what differs, which is what happens at the right-hand edge.
///
/// **There is no spare column here.** The 8×8 masters fill columns 0–6 and lean
/// into column 7; Uni-VGA keeps column 7 as its gap. This face IS 7 wide, and bit 0
/// of each row is BDF's byte padding rather than a column — so a shift that pushes
/// ink past column 6 puts it in a bit [`blit_glyph_styled`] never samples. Measured
/// over the whole subset:
///
/// * **Bold cannot lose a stroke**, because `row | row >> 1` is additive: every
///   roman pixel survives and the smear off column 6 lands in the padding bit,
///   which is a clip the cell never sees rather than a bleed into the next cell.
///   What it does cost is a one-pixel COUNTER: 21 letters (`b d g h k m n p q r u
///   v w y G K M N Q W Y`) have a 1 px gap that the double-strike closes. That is
///   not a property of this face — the same measurement over Uni-VGA at its own
///   8×16 cell closes 30, `m` among them — it is what a one-pixel double-strike
///   does to any bitmap face with one-pixel counters, and the alternatives are
///   worse: declining to embolden makes bold indistinguishable from roman, and
///   falling back to `vga16` for emphasised runs reinstates the touching letters
///   this arm exists to fix, blob and all.
/// * **Italic loses ink on exactly four glyphs** — `T`, `Ð`, `×` and `æ`, the only
///   ones in the subset that ink column 6 at all — and what they lose is one pixel
///   of a horizontal bar whose right end still reaches column 6. No letter loses
///   its rightmost stroke, because the shear moves ink RIGHT and only column-6 ink
///   can fall off the edge.
///
/// Sheared at row 7, the midpoint of the 14-row box, exactly as
/// [`synthesize_face16`] shears at the midpoint of its 16.
fn synthesize_face7(bits: [u8; 14], style: u8) -> [u8; 14] {
    let mut out = bits;
    if style & STYLE_ITALIC != 0 {
        for row in out.iter_mut().take(7) {
            *row >>= 1;
        }
    }
    if style & STYLE_BOLD != 0 {
        for row in out.iter_mut() {
            *row |= *row >> 1;
        }
    }
    out
}

/// Blit one glyph into `canvas`, top-left at device pixel `(px, py)`, scaled to
/// `cw × ch` device px. Set bits paint `fg`; clear bits paint `bg` when `Some`
/// (skipped when `None`, leaving the canvas — transparent text over graphics).
/// Unprintable / out-of-font chars paint only `bg` (a blank cell). Blits are
/// clipped to the canvas bounds.
///
/// When `cw`/`ch` request an exact 2× cell (16×16), edges are smoothed via
/// [`scale2x`] instead of plain nearest-neighbour doubling. Any other size
/// (including the native 8×8 used by the current v6 pixel-canvas call sites)
/// uses nearest-neighbour, unchanged from before.
///
/// Draws the ROMAN face; [`blit_glyph_styled`] is the same blit with a v6 style
/// byte applied.
pub fn blit_glyph(
    canvas: &mut RgbaImage,
    glyph: char,
    px: u32,
    py: u32,
    cw: u32,
    ch: u32,
    fg: Rgba<u8>,
    bg: Option<Rgba<u8>>,
    tf: Option<&crate::native_font::TextFace>,
) {
    blit_glyph_styled(canvas, glyph, px, py, cw, ch, fg, bg, 0, tf);
}

/// Whether a glyph must TILE EDGE TO EDGE with its neighbours, so its rightmost
/// source column has to survive into the cell's rightmost column (SQ-1027).
///
/// Box drawing (U+2500..) and block elements (U+2580..) — a game's chrome, and the
/// only glyphs whose correctness depends on meeting the cell next door. A LETTER is
/// the opposite case: `crate::render::vga16` inks 76 of its 94 printable glyphs out
/// to column 6 and keeps column 7 as the inter-character gap, so dropping that
/// column is exactly right for text and exactly wrong for a rule.
///
/// Same range as `render::screen::is_box_glyph`, which delegates here rather than
/// restating it — one predicate, since a second copy of a set like this drifts.
pub(crate) fn must_tile(c: char) -> bool {
    ('\u{2500}'..='\u{259F}').contains(&c)
}

/// The source column of an 8-pixel master that destination column `dx` samples, in
/// a cell `cw` wide.
///
/// # Why a tiling glyph needs a different map
///
/// `dx * 8 / cw` is the identity at `cw == 8`, which is every machine but one. At
/// the Macintosh's `cw == 7` (SQ-0917) `dx` runs 0..=6 and the quotient takes
/// 0,1,2,3,4,5,6 — **source column 7 is never sampled** and the master's rightmost
/// column is dropped.
///
/// For text that is the right column to lose. For a glyph that has to meet the cell
/// beside it, it is not, and measuring which glyphs actually suffer is worth
/// recording because the arithmetic alone predicts far more damage than occurs: the
/// corners `└ ┌ ┐ ┘ ├ ┤ ┬ ┴ ┼` all SURVIVE, because their arms are ink across
/// columns 3..7 and dropping column 7 still leaves ink at column 6. Rendering the
/// whole U+2500..U+25FF range at both widths finds exactly three casualties —
/// `▕` (U+2595, right one-eighth block), whose only ink IS column 7, so it vanishes
/// outright; and `┄`/`┅`, whose dashes leave column 6 blank, so they lose contact
/// with the cell to their right.
///
/// So a tiling glyph maps its ENDPOINTS instead: `dx * 7 / (cw - 1)` sends
/// destination 0 to source 0 and destination `cw - 1` to source 7, dropping an
/// INTERIOR column. At `cw == 8` that is the identity too, so no machine but the
/// Macintosh moves. At `cw > 8` the endpoint map is NOT taken — the gate below is
/// `(2..8)` — and the plain `dx * 8 / cw` runs instead; the outcome coincides,
/// because `(cw - 1) * 8 / cw` floors to 7 for every `cw > 8`, so the rightmost
/// column still survives and the tiling guarantee holds. This sentence used to say
/// the wide case "spreads the master the same way", describing a branch the body
/// does not take (SQ-1065). An all-ink arm stays contiguous either way; a stem
/// mid-cell stays one pixel wide.
fn source_col(dx: u32, cw: u32, tiling: bool) -> u32 {
    let col = if tiling && (2..8).contains(&cw) {
        dx.saturating_mul(7) / (cw - 1)
    } else {
        dx.saturating_mul(8) / cw.max(1)
    };
    col.min(7)
}

/// [`blit_glyph`], with the run's ZMSD §8.7.1 `style` byte applied as a
/// synthesized face (bit 2 bold, bit 4 italic — see [`synthesize_face`], which
/// also documents why reverse/fixed-pitch are ignored here). `style == 0` is
/// byte-for-byte the old roman blit.
pub fn blit_glyph_styled(
    canvas: &mut RgbaImage,
    glyph: char,
    px: u32,
    py: u32,
    cw: u32,
    ch: u32,
    fg: Rgba<u8>,
    bg: Option<Rgba<u8>>,
    style: u8,
    // The cell, the face the RELEASE shipped and the pen (SQ-1011, SQ-1009).
    // `None` — and a `TextFace` with no face on it — draw exactly as before.
    tf: Option<&crate::native_font::TextFace>,
) {
    // The 8x16 face first (SQ-0932): at the 8x16 cell every production call site
    // uses it samples 1:1, where an 8x8 master has to double every row. The 8x8
    // chain stays as the fallback for what it doesn't carry — the quadrant blocks,
    // the APL quad, the runes — so `tall` and `short` are never both `Some`.
    //
    // A taller face is only an improvement in a cell that can show its rows.
    // Sample sixteen rows into eight and half of them are simply discarded, which
    // is worse than an 8x8 master doubled — a thinner, more broken glyph, not a
    // smaller one. So there is a floor, and `blit_glyph` is public enough to need
    // one: its own sample-sheet test renders square cells at 1x.
    //
    // **The floor was `ch >= 16` and that was a latent assumption, not a rule**
    // (SQ-0917). It read "16 is the only cell any production call site asks for",
    // which was true while every v6 machine shared one cell and stopped being true
    // the moment the Macintosh declared its own 7x15. At `ch == 15` the old guard
    // sent the Macintosh silently back to the 8x8 chain — and that chain has NO
    // descender below the baseline (see `crate::render::vga16`'s header, which is
    // the defect SQ-0932 introduced this face to fix). The symptom on screen is
    // clipped tails on `g`, `j`, `p`, `q` and `y` in raster, on one machine.
    //
    // 15 is in fact the ideal case rather than a marginal one: `dy * 16 / 15` maps
    // rows 0..=14 one-for-one and reaches row 15 never, and **no glyph in the table
    // inks row 15** — so the Macintosh cell samples the face losslessly. `y`'s tail
    // lives in rows 12..=14 and arrives whole.
    //
    // 12 is the floor because three quarters of the source rows survive there,
    // which comfortably covers a descender sitting in the bottom quarter. Below it
    // the resample starts skipping source rows outright and the 8x8 master — drawn
    // to be legible at its own size — is the better source.
    // **The release's own face first, when it IS the cell** (SQ-1011).
    //
    // `vga16` is drawn for an 8-pixel advance: 76 of its 94 printable glyphs ink
    // out to column 6, so column 7 is their whole inter-character gap and a
    // 7-wide cell drops it — letters touch. The Macintosh floppy carries `FONT`
    // 524 at exactly 7x15, which is the cell SQ-0917 declares, so it samples 1:1
    // and keeps its own spacing and its own left side bearings.
    //
    // The dimensions must match EXACTLY. A face that has to be resampled into the
    // cell is the defect this replaces wearing different clothes, so a mismatch
    // declines here and `vga16` answers as before.
    // Dimensions only. **Fitness was already decided** by `native_font::fits`,
    // which is the single authority on whether a face may be used — it checks the
    // advance across printable ASCII, because `BitmapFont::proportional` counts
    // the accented high range and answers `true` for `FONT` 524 (SQ-0916).
    //
    // This kept its own copy of that test, including the `!proportional` clause,
    // and that duplicate is why the feature shipped inert a second time: the
    // resolver was corrected and the renderer went on rejecting the same face on
    // the same wrong condition. What survives here is the cheap structural check
    // that the face is this cell — a guard against a mismatched pair reaching the
    // sampler, not a second opinion on fitness.
    // **§8.7.1's Italic bit is a RULE on the machines that shipped Version 6**
    // (SQ-1028). The standard offers "rendering italic with underlining" as its own
    // example, so neither answer is a compliance question — and both machines with a
    // capture to measure underline. `machine-screenshots/amiga-shogun-game.png` and
    // `mac-shogun.jpg` rule under `Erasmus` in "the Erasmus, a Dutch merchant" and
    // under nothing beside it; the Macintosh had real italics and used them anyway
    // not at all. Where a machine has no capture, the synthesised slope stands.
    //
    // The shear is REMOVED rather than added to, because sloping and ruling are two
    // renderings of one bit and drawing both would be neither machine.
    let rule = tf.is_some_and(|t| t.underlines_emphasis()) && style & STYLE_ITALIC != 0;
    let style = if rule { style & !STYLE_ITALIC } else { style };

    // **A PROPORTIONAL face is drawn at its OWN size, not stretched to the cell**
    // (SQ-1009). It has no single advance to match `cw` against — that is what
    // makes it proportional — so the `Cell` test below can never admit one and the
    // filter would silently discard Arthur's whole typeface. Its bitmap is scaled
    // by the TEXT scale instead (each face pixel becomes a `scale.0` x `scale.1`
    // block, 2x2 on Arthur's Amiga floppy), which is what makes `face.height *
    // text_scale.1` the declared cell height in the first place. `TextFace` holds
    // that scale, and it is NOT always the art scale — see
    // `zvm::interpreter::V6FaceSpace` (SQ-1039).
    // `draws_proportionally` and not `proportional()`: a §8.7.1 FIXED-PITCH run on
    // a machine that has an alternate to draw it with is stamped into the declared
    // cell instead, which is how a Macintosh status bar keeps its columns while the
    // prose beside it steps Geneva's own advances (SQ-1036). The rule is asked of
    // `TextFace` rather than tested here, because `zvm`'s pen has to answer it the
    // same way and two copies of one rule is SQ-1026/SQ-1035.
    //
    // A FIXED face at a scale is drawn here too (SQ-1053): the Amiga's system topaz
    // is 8x8 on an 8x16 cell and needs each face row twice, which the cell blit
    // below cannot do — its `f.height == ch` filter would decline it and quietly
    // fall back to `vga16`. `draws_scaled` is that whole question, in one place.
    if let Some(t) = tf.filter(|t| t.draws_scaled(style)) {
        if let Some((f, g)) = t.face_for(style).and_then(|f| {
            u8::try_from(u32::from(glyph)).ok().and_then(|c| f.glyph(c)).map(|g| (f, g))
        }) {
            let row_bytes = g.row_bytes(f.height);
            blit_metric_glyph(canvas, g, row_bytes, px, py, ch, t.scale(), fg, bg, style, t.bold_smear(style), rule);
            return;
        }
        // A code the typeface does not carry falls through to the masters below,
        // drawn in the cell exactly as it would be with no face at all.
    }
    let native_face = tf
        .and_then(|t| t.face_for(style))
        .filter(|f| u32::from(f.width) == cw && u32::from(f.height) == ch);
    // `rows.len() as u32 == ch` (== `f.height`) is what makes this single-byte-per-
    // row read below safe: it holds only when this GLYPH's own row is exactly one
    // byte (`Glyph::row_bytes(f.height) == 1`), which every real face reaching this
    // path satisfies — `f.width == cw` above already restricts it to a font whose
    // nominal cell fits the (always ≤8px) v6 grid, and a glyph is never wider than
    // its font's nominal cell. A glyph that DID need more than one byte here would
    // fail this filter and fall through to the masters below, not misread (SQ-1038).
    let native_rows: Option<&[u8]> = native_face
        .and_then(|f| u8::try_from(u32::from(glyph)).ok().and_then(|c| f.glyph(c)))
        .map(|g| g.rows.as_slice())
        .filter(|rows| rows.len() as u32 == ch);

    // **A 7-WIDE CELL GETS A 7-WIDE FACE** (SQ-1016). `vga16` is drawn for an
    // 8-pixel advance and the Macintosh cell is 7 (SQ-0917), so the column it drops
    // is the one holding its whole inter-character gap: measured over all 52x52
    // ordered pairs of ASCII letters at 7x15, **1649 pairs touch** their neighbour
    // out of 2704, against **19** from the face below. `FONT` 524 off the release's
    // own floppy is the real answer and wins above (SQ-1011) — but a Macintosh cell
    // is reachable with no volume behind it (`--interpreter 3` on a bare `.z6`, and
    // CI, where every disk font lives on gitignored media), so the path that most
    // needs a 7-wide face is the one that cannot have the disk's.
    //
    // **`cw == 7` exactly, because this face has no horizontal resampler.** Its
    // rows are 7 bits with bit 0 as BDF's byte padding, so column `c` is drawn at
    // `dx == c` and nowhere else — one face pixel, one device pixel. At `cw == 8`
    // nothing here is consulted and no machine but the Macintosh can move.
    //
    // **`ch >= 14`, because that is where every row of a 14-row face survives.**
    // At `ch == 15`, `dy * 14 / 15` over 0..=14 gives 0,0,1,2,…,13: every source row
    // used, row 0 twice, so the glyph body below it is 1:1. That doubled row is
    // blank in 174 of the 194 glyphs; the 20 that ink it are accented capitals
    // (`À`–`Ý`), where it draws the top row of the ACCENT twice and shifts nothing.
    // At `ch == 13` source row 13 is never reached, and 28 glyphs ink it — the tails
    // of `g j p q y`, the comma and the semicolon, `Ç`'s cedilla — which is the
    // clipped-descender symptom of SQ-0917 arriving from the other direction. So
    // below 14 this declines and `vga16` answers exactly as it did before.
    //
    // Font 3 cannot come from here: the subset carries no box drawing, no block
    // elements and no cursor arrows (`misc7x14::font_three_is_not_in_this_face`),
    // so a tiling glyph gets `None` and falls through to the masters and to
    // `source_col`'s endpoint map (SQ-1027), untouched.
    let narrow = (native_rows.is_none() && cw == 7 && ch >= 14)
        .then(|| crate::render::misc7x14::glyph(glyph).map(|b| synthesize_face7(b, style)))
        .flatten();
    let tall = (native_rows.is_none() && narrow.is_none() && ch >= 12)
        .then(|| crate::render::vga16::glyph(glyph).map(|b| synthesize_face16(b, style)))
        .flatten();
    let short = if tall.is_some() || narrow.is_some() || native_rows.is_some() {
        None
    } else {
        glyph_bits(glyph).map(|b| synthesize_face(b, style))
    };
    let smoothed = (cw == 16 && ch == 16).then(|| short.map(|b| scale2x(&b))).flatten();
    // Whether this glyph has to meet the cell beside it — see `source_col`, which
    // samples a tiling glyph's rightmost column and a letter's inter-character gap
    // from two different places (SQ-1027).
    let tiling = must_tile(glyph);
    let (cwidth, cheight) = (canvas.width(), canvas.height());
    for dy in 0..ch {
        let oy = py + dy;
        if oy >= cheight {
            break;
        }
        let tall_row = (dy * 16 / ch) as usize; // nearest source row, 16-row face
        let narrow_row = (dy * 14 / ch) as usize; // nearest source row, 14-row face
        let row = (dy * 8 / ch) as usize; // nearest source row (non-smoothed path)
        for dx in 0..cw {
            let ox = px + dx;
            if ox >= cwidth {
                break;
            }
            let on = if let Some(rows) = native_rows {
                // 1:1 on both axes — the face IS the cell, which is the whole
                // point of preferring it. MSB = leftmost, as `vga16` packs too.
                rows[dy as usize] & (0x80 >> dx) != 0
            } else if let Some(g) = &narrow {
                // 1:1 horizontally — `dx` IS the source column, since the cell and
                // the face are both 7 wide (SQ-1016). MSB = leftmost, as `vga16`
                // packs too, and `dx < cw == 7` never reads bit 0, which is BDF's
                // byte padding rather than a column.
                g[narrow_row] & (0x80 >> dx) != 0
            } else if let Some(g) = &tall {
                let col = source_col(dx, cw, tiling);
                // vga16 packs each row MSB = leftmost column — the OPPOSITE of
                // font8x8 below. Both orders are live in this function.
                g[tall_row] & (0x80 >> col) != 0
            } else if let Some(grid) = &smoothed {
                grid[dy as usize][dx as usize]
            } else {
                let col = source_col(dx, cw, tiling);
                // font8x8 packs each row LSB = leftmost column.
                short.is_some_and(|g| g[row] & (1 << col) != 0)
            };
            // The rule, at the bottom of the cell and across its full width so
            // neighbouring cells join into an unbroken line (SQ-1028). One MASTER row
            // thick — `ch / 8` for the 8-row masters this chain draws, which is the
            // two native rows `amiga-shogun-game.png` measures against its
            // sixteen-row line, and one row on the Macintosh's fifteen-row cell.
            let on = on || (rule && dy + (ch / 8).max(1) >= ch);
            if on {
                canvas.put_pixel(ox, oy, fg);
            } else if let Some(b) = bg {
                canvas.put_pixel(ox, oy, b);
            }
        }
    }
}

/// One glyph of a PROPORTIONAL disk face, at the face's own size (SQ-1009).
///
/// Split out because every assumption the cell blit above makes is wrong here: the
/// glyph's width is its own advance rather than `cw`, its rows are the face's
/// rather than a fixed 8 or 16, and the only resampling wanted is a whole-number
/// block per face pixel — `scale` is the TEXT scale
/// ([`crate::native_font::TextFace::scale`]), so on an AMIGA press whose 320-wide
/// rendition doubles onto the unit screen each face pixel is a 2x2 native block and
/// the ten-row face fills the twenty-row line exactly. On a MACINTOSH it is `(1, 1)`
/// however dense the artwork is, because that machine paints text at one native
/// pixel per face pixel — `zvm::interpreter::V6FaceSpace` carries the split
/// (SQ-1039).
///
/// Rows are MSB-leftmost, as [`crate::render::vga16`] packs them and as
/// [`blorb::bitmap_font`] documents, so the style shears go `>>`. `row_bytes` is
/// [`blorb::bitmap_font::Glyph::row_bytes`]'s result for `g` — 1 for a glyph up to
/// 8px wide (bearing included), more for a wider one (SQ-1038) — computed once by
/// the caller, which already has the font `g` came from and so its `height`.
fn blit_metric_glyph(
    canvas: &mut RgbaImage,
    g: &blorb::bitmap_font::Glyph,
    row_bytes: usize,
    px: u32,
    py: u32,
    ch: u32,
    scale: (u32, u32),
    fg: Rgba<u8>,
    bg: Option<Rgba<u8>>,
    style: u8,
    smear: u8,
    // §8.7.1's Italic bit as a RULE rather than a slope (SQ-1028).
    rule: bool,
) {
    let (sx, sy) = (scale.0.max(1), scale.1.max(1));
    let rows = synthesize_rows(&g.rows, row_bytes, style, smear);
    // The pen's own advance, which is where the NEXT glyph starts — so a painted
    // background covers exactly the run and never a neighbour's column. Bold adds
    // the face's own `tf_BoldSmear` to it, exactly as the machine does: the smear
    // needs a column to live in or it eats the inter-character gap (SQ-1009).
    let adv = (u32::from(g.width) + u32::from(smear)) * sx;
    let row_cols = row_bytes * 8;
    let (cwidth, cheight) = (canvas.width(), canvas.height());
    for dy in 0..ch {
        let oy = py + dy;
        if oy >= cheight {
            break;
        }
        let src_row = (dy / sy) as usize;
        for dx in 0..adv {
            let ox = px + dx;
            if ox >= cwidth {
                break;
            }
            let col = (dx / sx) as usize;
            // The rule is ONE FACE ROW thick at the bottom of the cell, spanning the
            // whole advance so consecutive glyphs join into an unbroken line — which
            // is what the captures show under `Erasmus` and what makes it read as
            // one underlined WORD rather than seven underlined letters (SQ-1028).
            let ruled = rule && dy + sy >= ch;
            let on = ruled
                || (col < row_cols
                    && blorb::bitmap_font::row_bit(&rows, row_bytes, src_row, col));
            if on {
                canvas.put_pixel(ox, oy, fg);
            } else if let Some(b) = bg {
                canvas.put_pixel(ox, oy, b);
            }
        }
    }
}

/// [`synthesize_face16`]'s two transforms for a face of any height and, since
/// SQ-1038, any row width — `row_bytes` is [`blorb::bitmap_font::Glyph::row_bytes`],
/// 1 for a glyph up to 8px wide (bearing included), more for a wider one.
///
/// Same reasoning, same direction (MSB-leftmost, so right is a shift toward higher
/// columns), sheared at the midpoint of whatever height the face has rather than at
/// a fixed row — and emboldened by the FACE's `tf_BoldSmear` rather than by a fixed
/// one pixel. A row wider than one byte shifts as ONE unit across the whole row
/// (ink can cross a byte boundary), never each byte independently — shifting bytes
/// independently would wrap a bit that crossed a boundary back to the SAME byte's
/// column 0 instead of carrying it into the next byte, which is a corrupted glyph
/// wearing a font's clothes exactly the way this module's header warns about.
fn synthesize_rows(rows: &[u8], row_bytes: usize, style: u8, smear: u8) -> Vec<u8> {
    let mut out = rows.to_vec();
    if row_bytes == 0 {
        return out;
    }
    let height = out.len() / row_bytes;
    let half = height / 2;
    if style & STYLE_ITALIC != 0 {
        for r in 0..half {
            let shifted = shift_row_right(&out[r * row_bytes..(r + 1) * row_bytes], 1);
            out[r * row_bytes..(r + 1) * row_bytes].copy_from_slice(&shifted);
        }
    }
    if style & STYLE_BOLD != 0 {
        // The face's OWN smear, not a fixed one pixel — and the caller has already
        // widened the advance by the same amount.
        let n = u32::from(smear.max(1));
        for r in 0..height {
            let slice = r * row_bytes..(r + 1) * row_bytes;
            let shifted = shift_row_right(&out[slice.clone()], n);
            for (o, s) in out[slice].iter_mut().zip(shifted.iter()) {
                *o |= s;
            }
        }
    }
    out
}

/// Shift one MSB-leftmost row right by `n` bits, AS A WHOLE ROW: ink at column `c`
/// moves to column `c + n`, carrying across a byte boundary when `row.len() > 1`,
/// and a bit shifted past the last byte is dropped rather than wrapping — the same
/// clipping guarantee [`synthesize_face`]'s single-byte `<<`/`>>` gives for free,
/// generalised because a multi-byte row cannot get it for free (SQ-1038).
fn shift_row_right(row: &[u8], n: u32) -> Vec<u8> {
    let total_bits = row.len() * 8;
    let mut out = vec![0u8; row.len()];
    if row.is_empty() {
        return out;
    }
    let n = n as usize;
    for col in 0..total_bits {
        let new_col = col + n;
        if new_col >= total_bits {
            continue;
        }
        if row[col / 8] & (0x80 >> (col % 8)) != 0 {
            out[new_col / 8] |= 0x80 >> (new_col % 8);
        }
    }
    out
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;

    fn assert_has_glyph(c: char) {
        let bits = glyph_bits(c);
        assert!(bits.is_some(), "{c:?} (U+{:04X}) has no glyph", c as u32);
        if c != ' ' {
            assert_ne!(bits.unwrap(), [0u8; 8], "{c:?} (U+{:04X}) resolves to a blank glyph", c as u32);
        }
    }

    /// **A descender still descends in a cell that is not 16 tall** (SQ-0917).
    ///
    /// The 8x16 face is chosen over the 8x8 master by a height floor, and that
    /// floor used to be `ch >= 16` — which read as a quality rule and was really
    /// an assumption that 16 is the only cell in play. The Macintosh declares
    /// 7x15, fell under it, and silently got the 8x8 chain: a face with **no
    /// descender below the baseline at all**, so `g`, `j`, `p`, `q` and `y` came
    /// out with their tails clipped in raster on that one machine.
    ///
    /// # The property, and the one this case first got wrong
    ///
    /// "There is ink low in the cell" does NOT distinguish the two faces: stretch
    /// an 8-row glyph over 15 rows and it inks the bottom of the cell too. The
    /// first version of this case asserted exactly that and passed with the bug
    /// restored, which is the whole reason to falsify a test rather than trust a
    /// green one.
    ///
    /// The property that separates them is RELATIVE — a descender hangs below a
    /// letter that has none. In the 16-row face `x` inks rows 5..=11 and `y` inks
    /// 5..=14, so `y` reaches three rows lower. In the 8x8 master both sit inside
    /// the same band, so they end together however the cell is scaled.
    ///
    /// Falsified by restoring `ch >= 16`, which fails here at 7x15.
    ///
    /// The 7x15 row measures [`crate::render::misc7x14`] since SQ-1016, and that
    /// face puts `y` two rows below `x` where Uni-VGA puts it three. Two is still
    /// the number that separates a real descender from the 8x8 master stretched,
    /// which clears the baseline by ONE duplicated scanline — so the threshold
    /// stands and the case still asks the question it was written to ask.
    #[test]
    fn a_descender_still_descends_at_the_macintoshs_fifteen_row_cell() {
        let fg = Rgba([255, 0, 0, 255]);
        let lowest = |g: char, cw: u32, ch: u32| -> Option<u32> {
            let mut c = RgbaImage::from_pixel(cw, ch, Rgba([0, 0, 0, 255]));
            blit_glyph(&mut c, g, 0, 0, cw, ch, fg, None, None);
            (0..ch).rev().find(|&y| (0..cw).any(|x| *c.get_pixel(x, y) == fg))
        };
        for (cw, ch) in [(7u32, 15u32), (8, 16)] {
            let x = lowest('x', cw, ch).expect("`x` inks something");
            let y = lowest('y', cw, ch).expect("`y` inks something");
            assert!(
                y >= x + 2,
                "{cw}x{ch}: `y` bottoms out at row {y} against `x` at {x} — a depth of {}, \
                 where the 8x16 face gives 3. A depth of ONE is the 8x8 fallback stretched: \
                 its tail clears the baseline by a single duplicated scanline, which is why \
                 `y > x` alone does not tell the two faces apart.",
                y - x,
            );
        }
    }

    #[test]
    fn space_paints_only_bg() {
        let mut c = RgbaImage::from_pixel(8, 8, Rgba([0, 0, 0, 255]));
        blit_glyph(&mut c, ' ', 0, 0, 8, 8, Rgba([255, 0, 0, 255]), Some(Rgba([9, 9, 9, 255])), None);
        // No set bits → every pixel is the bg fill, none is fg.
        assert!(c.pixels().all(|p| *p == Rgba([9, 9, 9, 255])));
    }

    #[test]
    fn glyph_sets_some_fg_pixels() {
        let mut c = RgbaImage::from_pixel(8, 8, Rgba([0, 0, 0, 0]));
        blit_glyph(&mut c, 'A', 0, 0, 8, 8, Rgba([255, 0, 0, 255]), None, None);
        // 'A' has set bits → at least one fg pixel, and transparent bg elsewhere.
        assert!(c.pixels().any(|p| *p == Rgba([255, 0, 0, 255])), "A has fg pixels");
        assert!(c.pixels().any(|p| p[3] == 0), "unset bits stay transparent (bg=None)");
    }

    #[test]
    fn transparent_bg_leaves_canvas_on_clear_bits() {
        let mut c = RgbaImage::from_pixel(8, 8, Rgba([1, 2, 3, 255]));
        blit_glyph(&mut c, '.', 0, 0, 8, 8, Rgba([255, 255, 255, 255]), None, None);
        // A '.' is mostly clear; those cells keep the original canvas colour.
        assert!(c.pixels().any(|p| *p == Rgba([1, 2, 3, 255])), "clear bits keep canvas");
    }

    #[test]
    fn out_of_range_char_is_blank() {
        // U+2588 (a block glyph) used to be the "out of range" probe here,
        // back when only BASIC_FONTS was consulted; it's covered now (via
        // BLOCK_FONTS, per the coverage audit), so this uses a CJK ideograph
        // instead — genuinely outside every set `glyph_bits` checks.
        let mut c = RgbaImage::from_pixel(8, 8, Rgba([0, 0, 0, 0]));
        blit_glyph(&mut c, '\u{4E2D}', 0, 0, 8, 8, Rgba([255, 0, 0, 255]), None, None);
        assert!(c.pixels().all(|p| p[3] == 0), "unknown glyph paints nothing with bg=None");
    }

    #[test]
    fn scales_up_to_fill_cell() {
        // 8×8 glyph blitted into a 16×16 cell must touch the lower-right quadrant.
        let mut c = RgbaImage::from_pixel(16, 16, Rgba([0, 0, 0, 0]));
        blit_glyph(&mut c, 'M', 0, 0, 16, 16, Rgba([255, 0, 0, 255]), None, None);
        assert!(
            (8..16).any(|y| (0..16).any(|x| c.get_pixel(x, y)[3] == 255)),
            "scaled glyph reaches the lower half of the cell"
        );
    }

    #[test]
    fn glyph_coverage_ascii_printable() {
        for b in 32u8..=126 {
            assert_has_glyph(b as char);
        }
    }

    #[test]
    fn glyph_coverage_zscii_default_unicode_table() {
        // ZSCII 155-223 (ZMSD §3.8.5) is the full set of accented characters
        // v6 titles can print via the default Unicode translation table.
        for zscii in 155u16..=223 {
            assert_has_glyph(zvm::text::decode::zscii_to_char(zscii));
        }
    }

    #[test]
    fn glyph_coverage_font3_box_and_cursor_symbols() {
        // BeyondZork's font-3 box-drawing/block/cursor codepoints
        // (zvm::cpu::exec::font3_translate, codes 32-96 minus the unassigned
        // 71-74 gap which maps to U+FFFD).
        let symbols = [
            '\u{2190}', '\u{2191}', '\u{2192}', '\u{2193}', '\u{2195}',
            '\u{2500}', '\u{2502}', '\u{2534}', '\u{252C}', '\u{251C}', '\u{2524}',
            '\u{2514}', '\u{250C}', '\u{2510}', '\u{2518}', '\u{253C}',
            '\u{2571}', '\u{2572}', '\u{2573}',
            '\u{2588}', '\u{2580}', '\u{2584}', '\u{258C}', '\u{2590}',
            '\u{259D}', '\u{2597}', '\u{2596}', '\u{2598}',
            '\u{2594}', '\u{2581}', '\u{258F}', '\u{2595}',
            '\u{258E}', '\u{258D}', '\u{258B}', '\u{258A}', '\u{2589}',
            '\u{2395}', '\u{FFFD}',
        ];
        for c in symbols {
            assert_has_glyph(c);
        }
    }

    #[test]
    fn glyph_coverage_font3_runic_placeholders() {
        // The 26 BeyondZork "atmosphere" runic codepoints (font-3 codes 97-122).
        for &(c, _) in EXTRA_GLYPHS {
            assert_has_glyph(c);
        }
    }

    #[test]
    fn extra_glyphs_are_pairwise_distinct() {
        // Sanity check that the hand-drawn runic placeholders don't collapse
        // onto duplicate bitmaps (which would make two different in-game
        // codepoints look identical).
        for (i, (ci, gi)) in EXTRA_GLYPHS.iter().enumerate() {
            for (cj, gj) in EXTRA_GLYPHS.iter().skip(i + 1) {
                assert_ne!(gi, gj, "{:?} and {:?} render identically", ci, cj);
            }
        }
    }

    /// The lit (fg) pixel coordinates of a glyph blitted into its own 8×16 v6
    /// cell at the origin — the geometry every raster call site uses.
    fn lit_cell(glyph: char, style: u8) -> std::collections::BTreeSet<(u32, u32)> {
        let fg = Rgba([255, 0, 0, 255]);
        let mut c = RgbaImage::from_pixel(8, 16, Rgba([0, 0, 0, 0]));
        blit_glyph_styled(&mut c, glyph, 0, 0, 8, 16, fg, None, style, None);
        c.enumerate_pixels().filter(|(_, _, p)| **p == fg).map(|(x, y, _)| (x, y)).collect()
    }

    #[test]
    fn roman_face_is_unchanged_by_reverse_and_fixed_pitch_bits() {
        // SQ-0540: only bits 2/4 synthesize a face. Reverse (1) is the caller's
        // fg/bg swap and fixed-pitch (8) is a no-op in a bitmap font, so a run
        // carrying them — Zork Zero's whole reverse-video banner/ribbon chrome —
        // must stay pixel-identical to the roman blit.
        for glyph in ['A', 'g', '0', ' ', '\u{2500}'] {
            let roman = lit_cell(glyph, 0);
            for bits in [1, 8, 1 | 8] {
                assert_eq!(lit_cell(glyph, bits), roman, "{glyph:?} with style {bits} must render roman");
            }
        }
    }

    #[test]
    fn bold_double_strikes_one_pixel_right() {
        // Emboldening is additive: bold ⊇ roman, and every added pixel is a roman
        // pixel shifted one column right (the classic bitmap double-strike).
        for glyph in ['A', 'm', 'W', '5'] {
            let roman = lit_cell(glyph, 0);
            let bold = lit_cell(glyph, 2);
            assert!(roman.is_subset(&bold), "{glyph:?}: bold must keep every roman pixel");
            assert!(bold.len() > roman.len(), "{glyph:?}: bold must light more pixels than roman");
            // Each extra pixel is its left neighbour from the roman face.
            for &(x, y) in bold.difference(&roman) {
                assert!(x > 0 && roman.contains(&(x - 1, y)), "{glyph:?}: bold pixel ({x},{y}) is not a +1 double-strike");
            }
        }
    }

    #[test]
    fn italic_shears_the_top_half_only() {
        // The 8×16 cell doubles each font row, so the shear boundary (font row 4)
        // lands at device row 8: rows 0..8 move one pixel right, rows 8..16 stay.
        for glyph in ['A', 'l', 'H'] {
            let roman = lit_cell(glyph, 0);
            let italic = lit_cell(glyph, 4);
            let split = |s: &std::collections::BTreeSet<(u32, u32)>, top: bool| -> std::collections::BTreeSet<(u32, u32)> {
                s.iter().copied().filter(|&(_, y)| (y < 8) == top).collect()
            };
            let want_top: std::collections::BTreeSet<(u32, u32)> =
                split(&roman, true).into_iter().filter(|&(x, _)| x < 7).map(|(x, y)| (x + 1, y)).collect();
            assert_eq!(split(&italic, true), want_top, "{glyph:?}: top half must be the roman top shifted +1");
            assert_eq!(split(&italic, false), split(&roman, false), "{glyph:?}: bottom half must not move");
            assert_ne!(italic, roman, "{glyph:?}: italic must differ from roman");
        }
    }

    #[test]
    fn bold_italic_applies_both_transforms() {
        let roman = lit_cell('A', 0);
        let bold = lit_cell('A', 2);
        let italic = lit_cell('A', 4);
        let both = lit_cell('A', 2 | 4);
        assert!(italic.is_subset(&both), "bold-italic keeps the sheared shape");
        assert!(both.len() > italic.len(), "bold-italic is heavier than italic");
        for face in [&roman, &bold, &italic] {
            assert_ne!(&both, face, "bold-italic must differ from every single face");
        }
    }

    #[test]
    fn styled_faces_never_bleed_into_neighbouring_cells() {
        // Blit into the middle cell of a 3×3 grid of 8×16 cells and assert every
        // pixel outside that cell is untouched — the u8 row shift drops anything
        // past column 7, so no synthesized face can spill sideways.
        let fg = Rgba([255, 0, 0, 255]);
        for style in [0u8, 2, 4, 6, 1 | 2 | 4 | 8] {
            for glyph in ['W', '\u{2588}', '\u{2500}', 'j'] {
                let mut c = RgbaImage::from_pixel(24, 48, Rgba([0, 0, 0, 0]));
                blit_glyph_styled(&mut c, glyph, 8, 16, 8, 16, fg, Some(Rgba([9, 9, 9, 255])), style, None);
                for (x, y, p) in c.enumerate_pixels() {
                    let inside = (8..16).contains(&x) && (16..32).contains(&y);
                    if !inside {
                        assert_eq!(*p, Rgba([0, 0, 0, 0]), "{glyph:?} style {style} painted ({x},{y}) outside its cell");
                    }
                }
            }
        }
    }

    #[test]
    fn scale2x_path_is_deterministic() {
        let mut c1 = RgbaImage::from_pixel(16, 16, Rgba([0, 0, 0, 0]));
        let mut c2 = RgbaImage::from_pixel(16, 16, Rgba([0, 0, 0, 0]));
        blit_glyph(&mut c1, 'M', 0, 0, 16, 16, Rgba([255, 0, 0, 255]), None, None);
        blit_glyph(&mut c2, 'M', 0, 0, 16, 16, Rgba([255, 0, 0, 255]), None, None);
        assert_eq!(c1, c2, "same glyph/size blitted twice must produce identical pixels");
    }

    #[test]
    fn scale2x_reshapes_a_diagonal_versus_naive_doubling() {
        // '╱' is a pure diagonal stroke — the textbook case scale2x smooths. It is
        // font 3, so it stays on the 8×8 master the smoothing belongs to (SQ-0932).
        let bits = glyph_bits('\u{2571}').expect("box diagonal glyph must exist");
        let mut smoothed = RgbaImage::from_pixel(16, 16, Rgba([0, 0, 0, 0]));
        blit_glyph(&mut smoothed, '\u{2571}', 0, 0, 16, 16, Rgba([255, 0, 0, 255]), None, None);

        // Naive nearest-neighbour doubling: the pre-smoothing behaviour,
        // each source pixel expands into an exact 2×2 block.
        let mut naive = RgbaImage::from_pixel(16, 16, Rgba([0, 0, 0, 0]));
        for row in 0..8u32 {
            for col in 0..8u32 {
                if bits[row as usize] & (1 << col) != 0 {
                    for dy in 0..2 {
                        for dx in 0..2 {
                            naive.put_pixel(col * 2 + dx, row * 2 + dy, Rgba([255, 0, 0, 255]));
                        }
                    }
                }
            }
        }
        assert_ne!(smoothed, naive, "scale2x should reshape corners vs naive doubling on a diagonal");
    }

    /// Read the rendered 8×16 cell back as sixteen row bitmaps, MSB-leftmost, so
    /// a test can compare a blit against a face's own rows.
    fn rendered_rows_with(
        glyph: char,
        style: u8,
        tf: Option<&crate::native_font::TextFace>,
    ) -> [u8; 16] {
        let fg = Rgba([255, 0, 0, 255]);
        let mut canvas = RgbaImage::from_pixel(8, 16, Rgba([0, 0, 0, 0]));
        blit_glyph_styled(&mut canvas, glyph, 0, 0, 8, 16, fg, None, style, tf);
        let mut rows = [0u8; 16];
        for (y, row) in rows.iter_mut().enumerate() {
            for x in 0..8u32 {
                if *canvas.get_pixel(x, y as u32) == fg {
                    *row |= 0x80 >> x;
                }
            }
        }
        rows
    }

    fn rendered_rows(glyph: char, style: u8) -> [u8; 16] {
        let fg = Rgba([255, 0, 0, 255]);
        let mut canvas = RgbaImage::from_pixel(8, 16, Rgba([0, 0, 0, 0]));
        blit_glyph_styled(&mut canvas, glyph, 0, 0, 8, 16, fg, None, style, None);
        let mut rows = [0u8; 16];
        for (y, row) in rows.iter_mut().enumerate() {
            for x in 0..8u32 {
                if *canvas.get_pixel(x, y as u32) == fg {
                    *row |= 0x80 >> x;
                }
            }
        }
        rows
    }

    /// At the cell every production call site uses — 8×16, `v6_layout`'s
    /// `FONT_W`/`FONT_H` — an ordinary letter is the 16-row face's own bitmap,
    /// pixel for pixel. No resampling on either axis (SQ-0932).
    #[test]
    fn the_tall_face_blits_one_to_one_at_the_v6_cell() {
        for ch in ['A', 'g', 'L', '~', 'É', 'ß', 'œ'] {
            let face = crate::render::vga16::glyph(ch).expect("all of these are in the subset");
            assert_eq!(rendered_rows(ch, 0), face, "{ch:?} is not its own bitmap on the canvas");
        }
    }

    /// The regression this replaced, stated as the property that used to hold and
    /// now must not: an 8×8 master doubled into a 16-row cell makes every row an
    /// exact copy of its partner, so rows `2k` and `2k+1` were ALWAYS equal. If
    /// this ever passes vacuously again, the 8×8 path has silently won.
    #[test]
    fn the_tall_face_is_not_a_doubled_eight_row_master() {
        let rows = rendered_rows('A', 0);
        let odd = (0..8).filter(|k| rows[2 * k] != rows[2 * k + 1]).count();
        assert!(odd > 0, "every row pair is identical — 'A' is being doubled, not drawn: {rows:02X?}");
    }

    /// A descender is the thing an 8×8 master cannot express in this cell at all.
    /// Uni-VGA's baseline is row 12 (`FONT_ASCENT 12`), so `g` must put ink below
    /// it — and the tail must differ from the bowl above, or it is just doubling.
    #[test]
    fn a_descender_reaches_below_the_baseline() {
        let rows = rendered_rows('g', 0);
        assert!(rows[12..16].iter().any(|&r| r != 0), "g has no tail below the baseline: {rows:02X?}");
        assert_ne!(rows[12], rows[11], "the tail is a copy of the bowl's last row");
    }

    /// A cell too short to show sixteen rows keeps the 8×8 master. Halving a
    /// 16-row face throws away every other row, which breaks a glyph rather than
    /// shrinking it — see the `ch >= 16` gate in [`blit_glyph_styled`].
    #[test]
    fn a_short_cell_keeps_the_eight_row_master() {
        let fg = Rgba([255, 0, 0, 255]);
        let mut canvas = RgbaImage::from_pixel(8, 8, Rgba([0, 0, 0, 0]));
        blit_glyph(&mut canvas, 'A', 0, 0, 8, 8, fg, None, None);
        let eight = glyph_bits('A').expect("font8x8 has 'A'");
        for (y, &src) in eight.iter().enumerate() {
            let mut row = 0u8;
            for x in 0..8u32 {
                if *canvas.get_pixel(x, y as u32) == fg {
                    row |= 0x80 >> x;
                }
            }
            // font8x8 is LSB-leftmost; the canvas was read back MSB-leftmost.
            assert_eq!(row, src.reverse_bits(), "row {y} of an 8x8 'A' is not the 8x8 master's");
        }
    }

    /// Font 3 still draws, out of the 8×8 chain, and draws EXACTLY as it did
    /// before the 16-row face existed — each master row doubled, nothing resampled.
    /// That is the guarantee that keeps v6 rule geometry where SQ-0750 and
    /// SQ-0755 left it: `│` is still one native pixel wide, not Uni-VGA's two.
    #[test]
    fn font_three_falls_back_and_doubles_exactly() {
        for ch in ['\u{2500}', '\u{2502}', '\u{250C}', '\u{2588}', '\u{2591}', '\u{2596}', '\u{2190}', '\u{2395}'] {
            assert!(crate::render::vga16::glyph(ch).is_none(), "the fallback is what's under test");
            let rows = rendered_rows(ch, 0);
            let eight = glyph_bits(ch).expect("the 8x8 chain carries every font-3 glyph");
            for (k, &src) in eight.iter().enumerate() {
                // font8x8 is LSB-leftmost; the canvas was read back MSB-leftmost.
                let mirrored = src.reverse_bits();
                assert_eq!(rows[2 * k], mirrored, "U+{:04X} row {k} top half", ch as u32);
                assert_eq!(rows[2 * k + 1], mirrored, "U+{:04X} row {k} bottom half", ch as u32);
            }
        }
    }

    /// The specific pixel that decided the subset's boundary (SQ-0932). Uni-VGA
    /// draws `│` as CP437 does, two columns wide; Journey's frame border is that
    /// glyph, and `v6_journey_prose_containment` reads it as a hairline.
    #[test]
    fn the_vertical_rule_stays_one_pixel_wide() {
        let rows = rendered_rows('\u{2502}', 0);
        let ink = rows.iter().copied().find(|&r| r != 0).expect("the rule has ink");
        assert_eq!(ink.count_ones(), 1, "`|` must stay a one-pixel hairline, not CP437's two: {ink:#010b}");
    }

    /// Bold on the 16-row face is additive and cannot bleed into the next cell —
    /// the same two guarantees [`synthesize_face`] gives the 8-row master, checked
    /// through the shift that runs the other way (SQ-0932).
    #[test]
    fn tall_bold_is_a_superset_and_stays_inside_the_cell() {
        for ch in ['A', 'g', 'W', 'M'] {
            let roman = rendered_rows(ch, 0);
            let bold = rendered_rows(ch, STYLE_BOLD);
            for (k, (&r, &b)) in roman.iter().zip(bold.iter()).enumerate() {
                assert_eq!(b & r, r, "{ch:?} row {k}: bold dropped ink the roman face had");
                assert!(b.count_ones() >= r.count_ones(), "{ch:?} row {k}: bold is not heavier");
            }
        }
    }

    /// Italic leans the top of the glyph one column RIGHT. On an MSB-leftmost row
    /// that is `>> 1`, which is the easiest thing in this module to get backwards —
    /// and a backwards shear still looks like a slanted font, just the wrong way.
    #[test]
    fn tall_italic_leans_forward_not_backward() {
        let roman = rendered_rows('L', 0);
        let italic = rendered_rows('L', STYLE_ITALIC);
        let top = (0..8).find(|&k| roman[k] != 0).expect("L has ink in its top half");
        assert_eq!(italic[top], roman[top] >> 1, "the top of L must move right, not left");
        let bottom = (8..16).find(|&k| roman[k] != 0).expect("L has ink in its bottom half");
        assert_eq!(italic[bottom], roman[bottom], "the bottom of L stays put");
    }

    #[test]
    fn bitfont_sample_png_renders_1x_2x_3x() {
        // Composite pangram + digits + box/arrow glyphs at 1x/2x/3x scale, so
        // the controller can eyeball the CC0 base font, the coverage fixes,
        // and the scale2x smoothing side-by-side. Written to target/ (not
        // committed) — a visual oracle, not a pass/fail assertion.
        let text_rows = [
            "THE QUICK BROWN FOX JUMPS",
            "the quick brown fox jumps",
            "0123456789 !?.,;:'\"-+=/\\",
            "\u{2190}\u{2191}\u{2192}\u{2193}\u{2195} \u{2395} \u{FFFD}",
            "\u{250C}\u{2500}\u{2500}\u{2510} \u{2588}\u{2580}\u{2584}\u{258C}\u{2590}",
            "\u{2514}\u{2500}\u{2500}\u{2518} \u{16A0}\u{16A2}\u{16B1}\u{16C9}\u{16DF}",
            "\u{00E4}\u{00F6}\u{00FC}\u{00DF} \u{0153}\u{0152} \u{00E9}\u{00E8}\u{00E7}\u{00F1}",
        ];
        let scales: [u32; 3] = [1, 2, 3];
        let cols = text_rows.iter().map(|r| r.chars().count()).max().unwrap_or(0) as u32;
        let rows = text_rows.len() as u32;

        // Lay the three scales out left-to-right with a gutter between them.
        let gutter = 8u32;
        let mut x_offset = 0u32;
        let mut panel_w = [0u32; 3];
        for (i, &s) in scales.iter().enumerate() {
            panel_w[i] = cols * 8 * s;
        }
        let total_w: u32 = panel_w.iter().sum::<u32>() + gutter * (scales.len() as u32 - 1);
        let total_h: u32 = rows * 8 * scales.iter().max().copied().unwrap_or(1);

        let mut canvas = RgbaImage::from_pixel(total_w.max(1), total_h.max(1), Rgba([16, 16, 20, 255]));
        let fg = Rgba([230, 230, 200, 255]);

        for (i, &s) in scales.iter().enumerate() {
            let cell = 8 * s;
            // blit_glyph only smooths at exactly 16x16 (s == 2); other
            // scales fall back to nearest, which is expected/documented.
            for (row, line) in text_rows.iter().enumerate() {
                for (col, ch) in line.chars().enumerate() {
                    let px = x_offset + col as u32 * cell;
                    let py = row as u32 * cell;
                    blit_glyph(&mut canvas, ch, px, py, cell, cell, fg, None, None);
                }
            }
            x_offset += panel_w[i] + gutter;
        }

        let out_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/bitfont_sample.png");
        if let Some(dir) = out_path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        canvas.save(&out_path).expect("write bitfont sample PNG");
    }
    /// Ink columns of `glyph` rendered into a `cw`-wide cell, bit `1 << x` per
    /// column — the shape a tiling glyph's correctness is judged on.
    fn cell_rows(glyph: char, cw: u32) -> Vec<u16> {
        let fg = Rgba([255, 0, 0, 255]);
        let mut canvas = RgbaImage::from_pixel(cw, 16, Rgba([0, 0, 0, 0]));
        blit_glyph_styled(&mut canvas, glyph, 0, 0, cw, 16, fg, None, 0, None);
        (0..16u32)
            .map(|y| {
                let mut r = 0u16;
                for x in 0..cw {
                    if *canvas.get_pixel(x, y) == fg {
                        r |= 1 << x;
                    }
                }
                r
            })
            .collect()
    }

    /// **A glyph that must tile keeps its rightmost column in a 7-wide cell**
    /// (SQ-1027).
    ///
    /// `dx * 8 / cw` never reaches source column 7 when `cw` is 7, so the master's
    /// rightmost column was dropped on the Macintosh (SQ-0917 is where that cell
    /// came from). The quest predicted this would break every corner; rendering the
    /// whole U+2500..U+25FF range at both widths says otherwise, and the real
    /// casualties are pinned below rather than the predicted ones — the corners are
    /// ink across columns 3..7, so losing column 7 still leaves ink at column 6 and
    /// the arm reaches the cell edge anyway.
    ///
    /// Falsified by restoring `dx * 8 / cw` for tiling glyphs: `▕` comes back blank
    /// and the two dashed rules come back a pixel short.
    #[test]
    fn a_tiling_glyph_keeps_its_right_edge_in_a_narrow_cell() {
        // `▕` U+2595 RIGHT ONE EIGHTH BLOCK — its ONLY ink is source column 7, so
        // this is the glyph that vanished outright.
        let narrow = cell_rows('\u{2595}', 7);
        assert!(
            narrow.iter().any(|&r| r != 0),
            "`▕` must survive a 7-wide cell; it is nothing but its rightmost column",
        );
        assert!(
            narrow.iter().all(|&r| r == 0 || r == 1 << 6),
            "`▕` is the RIGHTMOST column and nothing else, got {narrow:?}",
        );
        // `┄`/`┅` — dashed, so column 6 is a gap and only column 7 touches the edge.
        for ch in ['\u{2504}', '\u{2505}'] {
            assert!(
                cell_rows(ch, 7).iter().any(|&r| r & (1 << 6) != 0),
                "{ch:?} must reach the right edge of a 7-wide cell to meet the cell beside it",
            );
        }
    }

    /// The glyphs the narrow cell was never breaking must not start breaking now —
    /// the fix drops an INTERIOR source column instead of the last one, and these
    /// are the shapes that could notice (SQ-1027).
    #[test]
    fn narrowing_a_cell_keeps_arms_contiguous_and_stems_hairline() {
        // A corner's arm runs from the stem to the edge and must stay unbroken.
        for ch in ['\u{2514}', '\u{250C}', '\u{2500}', '\u{2588}'] {
            for (y, &r) in cell_rows(ch, 7).iter().enumerate() {
                if r == 0 {
                    continue;
                }
                let lo = r.trailing_zeros();
                let hi = 15 - r.leading_zeros();
                let span = (lo..=hi).fold(0u16, |a, b| a | 1 << b);
                assert_eq!(r, span, "{ch:?} row {y} has a hole: {r:#09b}");
            }
        }
        // `│`'s stem sits mid-cell and must stay one pixel — the case SQ-0932 chose
        // the 16-row subset's boundary on.
        for cw in [7u32, 8] {
            let rows = cell_rows('\u{2502}', cw);
            let ink = rows.iter().copied().find(|&r| r != 0).expect("the rule has ink");
            assert_eq!(ink.count_ones(), 1, "`│` at cw={cw} must stay a hairline: {ink:#010b}");
        }
    }

    /// **Every machine but the Macintosh is untouched**, because both column maps
    /// are the identity at an 8-wide cell (SQ-1027).
    #[test]
    fn an_eight_wide_cell_is_the_identity_for_tiling_and_text_alike() {
        for c in 0x2500u32..=0x25FFu32 {
            let ch = char::from_u32(c).expect("a valid scalar");
            assert_eq!(
                (0..8).map(|dx| source_col(dx, 8, must_tile(ch))).collect::<Vec<_>>(),
                (0..8).collect::<Vec<_>>(),
                "{ch:?}: an 8-wide cell samples its master 1:1",
            );
        }
        // And a LETTER keeps the old map at 7 wide: vga16 inks out to column 6 and
        // column 7 is the inter-character gap, so dropping column 7 is correct there.
        assert_eq!(
            (0..7).map(|dx| source_col(dx, 7, false)).collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4, 5, 6],
        );
        assert_eq!(
            (0..7).map(|dx| source_col(dx, 7, true)).collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4, 5, 7],
            "a tiling glyph maps its endpoints and drops an interior column instead",
        );
    }

    /// The rendered 7×15 Macintosh cell (SQ-0917) read back as fifteen row
    /// bitmaps, MSB-leftmost over its seven columns — the shape a 7-wide face's
    /// own rows can be compared against.
    fn rendered_rows_7x15(glyph: char, style: u8, tf: Option<&crate::native_font::TextFace>) -> [u8; 15] {
        let fg = Rgba([255, 0, 0, 255]);
        let mut canvas = RgbaImage::from_pixel(7, 15, Rgba([0, 0, 0, 0]));
        blit_glyph_styled(&mut canvas, glyph, 0, 0, 7, 15, fg, None, style, tf);
        let mut rows = [0u8; 15];
        for (y, row) in rows.iter_mut().enumerate() {
            for x in 0..7u32 {
                if *canvas.get_pixel(x, y as u32) == fg {
                    *row |= 0x80 >> x;
                }
            }
        }
        rows
    }

    /// **The Macintosh cell draws the 7-wide face, row for row** (SQ-1016).
    ///
    /// Pins the SOURCE and the row map together: `dy * 14 / 15` sends device row 0
    /// and row 1 both to source row 0 and every row after that 1:1, so the whole
    /// glyph body is the face's own bitmap with its top row drawn twice. Source row
    /// 0 is blank in 174 of the 194 glyphs; `À` is one of the twenty that ink it,
    /// which is why it is here — its accent's top row is the doubled one.
    #[test]
    fn a_seven_wide_cell_draws_the_seven_wide_face() {
        for ch in ['A', 'm', 'g', 'y', 'T', '\u{00C0}', '\u{0153}', '\u{FFFD}'] {
            let face = crate::render::misc7x14::glyph(ch).expect("all of these are in the subset");
            let rows = rendered_rows_7x15(ch, 0, None);
            for (dy, &got) in rows.iter().enumerate() {
                let want = face[dy.saturating_sub(1)];
                assert_eq!(got, want, "{ch:?} device row {dy} is not source row {}", dy.saturating_sub(1));
            }
        }
    }

    /// **Letters stop touching in a 7-wide cell** — the defect this face was
    /// embedded to fix, measured on pixels (SQ-1016).
    ///
    /// `vga16` is drawn for an 8-pixel advance: 76 of its 94 printable glyphs ink
    /// out to column 6, so column 7 is their entire inter-character gap, and a
    /// 7-wide cell drops it. Census over all 52 × 52 ordered pairs of ASCII letters
    /// blitted into adjacent 7×15 cells, counting pairs with ink in both the left
    /// cell's last column and the right cell's first on the same row:
    ///
    /// | face | touching pairs | touching rows |
    /// |---|---|---|
    /// | `vga16` sampled into 7 wide (the old behaviour) | 1649 / 2704 | 4810 |
    /// | `misc7x14` at 7 wide (now) | 19 / 2704 | 19 |
    ///
    /// The 19 that remain are `T` before a letter whose first column reaches the
    /// crossbar's row — `T` is the ONLY glyph in the subset that inks column 6.
    ///
    /// Falsified by removing the `narrow` arm from `blit_glyph_styled`: this comes
    /// back at 1649 and `mimic` comes back with no gaps at all.
    #[test]
    fn letters_gain_their_inter_character_gap_in_a_seven_wide_cell() {
        let fg = Rgba([255, 0, 0, 255]);
        let pair = |a: char, b: char| -> usize {
            let mut c = RgbaImage::from_pixel(14, 15, Rgba([0, 0, 0, 0]));
            blit_glyph_styled(&mut c, a, 0, 0, 7, 15, fg, None, 0, None);
            blit_glyph_styled(&mut c, b, 7, 0, 7, 15, fg, None, 0, None);
            (0..15).filter(|&y| *c.get_pixel(6, y) == fg && *c.get_pixel(7, y) == fg).count()
        };
        let letters: Vec<char> = ('a'..='z').chain('A'..='Z').collect();
        let touching: Vec<(char, char)> = letters
            .iter()
            .flat_map(|&a| letters.iter().map(move |&b| (a, b)))
            .filter(|&(a, b)| pair(a, b) > 0)
            .collect();
        assert!(
            touching.len() <= 20,
            "{} of 2704 letter pairs touch their neighbour — the 8-wide face sampled into a \
             7-wide cell gives 1649, this face gives 19: {:?}",
            touching.len(),
            &touching[..touching.len().min(8)],
        );
        assert!(
            touching.iter().all(|&(a, _)| a == 'T'),
            "only `T` inks column 6 of this face, so only `T` can touch: {touching:?}",
        );
        // And the specimen: every adjacent pair in `mimic` is separated by a blank
        // column, which is the whole of what a reader sees.
        let word = "mimic";
        let mut c = RgbaImage::from_pixel(7 * word.len() as u32, 15, Rgba([0, 0, 0, 0]));
        for (i, ch) in word.chars().enumerate() {
            blit_glyph_styled(&mut c, ch, i as u32 * 7, 0, 7, 15, fg, None, 0, None);
        }
        for i in 1..word.len() as u32 {
            let seam = i * 7;
            assert!(
                (0..15).all(|y| *c.get_pixel(seam - 1, y) != fg || *c.get_pixel(seam, y) != fg),
                "`mimic` letters {i} and {} run into each other at column {seam}",
                i + 1,
            );
        }
    }

    /// **Font 3 still comes from the masters in a 7-wide cell, and still tiles**
    /// (SQ-1016, guarding SQ-1027).
    ///
    /// A character-graphics set has to meet the cell beside it, which no text face
    /// satisfies — so the 7-wide subset carries none of it (`misc7x14::tests::
    /// font_three_is_not_in_this_face`) and every such glyph falls through to
    /// `source_col`'s endpoint map exactly as before.
    #[test]
    fn a_tiling_glyph_still_comes_from_the_masters_in_a_seven_wide_cell() {
        let fg = Rgba([255, 0, 0, 255]);
        let rows = |ch: char| -> Vec<u8> {
            assert!(crate::render::misc7x14::glyph(ch).is_none(), "{ch:?} must not be in a text face");
            let mut c = RgbaImage::from_pixel(7, 15, Rgba([0, 0, 0, 0]));
            blit_glyph_styled(&mut c, ch, 0, 0, 7, 15, fg, None, 0, None);
            (0..15)
                .map(|y| (0..7).fold(0u8, |a, x| a | if *c.get_pixel(x, y) == fg { 1 << x } else { 0 }))
                .collect()
        };
        // `▕` is nothing BUT its master's rightmost column, so it is the glyph that
        // vanishes if the endpoint map ever stops running here.
        let bar = rows('\u{2595}');
        assert!(bar.iter().any(|&r| r != 0), "`▕` must survive a 7-wide cell");
        assert!(bar.iter().all(|&r| r == 0 || r == 1 << 6), "`▕` is the rightmost column only: {bar:?}");
        // A dashed rule leaves column 6 blank, so only the endpoint map reaches the edge.
        for ch in ['\u{2504}', '\u{2505}'] {
            assert!(cell_rows(ch, 7).iter().any(|&r| r & (1 << 6) != 0), "{ch:?} must reach the cell edge");
        }
        // A corner's arm stays unbroken and `│` stays a hairline.
        for ch in ['\u{2514}', '\u{250C}', '\u{2500}', '\u{2588}'] {
            for (y, &r) in rows(ch).iter().enumerate() {
                if r == 0 {
                    continue;
                }
                let span = (r.trailing_zeros()..=7 - r.leading_zeros()).fold(0u8, |a, b| a | 1 << b);
                assert_eq!(r, span, "{ch:?} row {y} has a hole: {r:#09b}");
            }
        }
        let stem = rows('\u{2502}').into_iter().find(|&r| r != 0).expect("the rule has ink");
        assert_eq!(stem.count_ones(), 1, "`│` must stay a one-pixel hairline: {stem:#010b}");
    }

    /// **No 8-wide machine moves** — the 7-wide face is admitted at `cw == 7` and
    /// nowhere else, because it has no horizontal resampler (SQ-1016).
    ///
    /// Every machine but the Macintosh declares an 8-wide cell (SQ-0917), so this
    /// is the case that says the arm cannot reach them. Checked at three heights,
    /// including the two that pass its own `ch >= 14` floor.
    #[test]
    fn an_eight_wide_cell_never_reaches_the_seven_wide_face() {
        let fg = Rgba([255, 0, 0, 255]);
        for ch in [12u32, 14, 15, 16] {
            for glyph in ['A', 'm', 'y'] {
                let face = crate::render::vga16::glyph(glyph).expect("in the 8x16 subset");
                let mut c = RgbaImage::from_pixel(8, ch, Rgba([0, 0, 0, 0]));
                blit_glyph_styled(&mut c, glyph, 0, 0, 8, ch, fg, None, 0, None);
                for dy in 0..ch {
                    let got = (0..8u32)
                        .fold(0u8, |a, x| a | if *c.get_pixel(x, dy) == fg { 0x80 >> x } else { 0 });
                    assert_eq!(
                        got,
                        face[(dy * 16 / ch) as usize],
                        "{glyph:?} at 8x{ch} row {dy} is not the 8x16 face's",
                    );
                }
            }
        }
    }

    /// **A cell too short for the 7-wide face keeps `vga16`** (SQ-1016).
    ///
    /// The floor is `ch >= 14` — where every row of a 14-row face survives. At
    /// `ch == 13`, `dy * 14 / 13` never reaches source row 13, and 28 glyphs ink it:
    /// the tails of `g j p q y`, the comma, the semicolon, `Ç`'s cedilla. So below
    /// the floor this declines rather than clipping them.
    #[test]
    fn a_cell_too_short_for_the_seven_wide_face_keeps_the_eight_wide_one() {
        let fg = Rgba([255, 0, 0, 255]);
        let mut c = RgbaImage::from_pixel(7, 13, Rgba([0, 0, 0, 0]));
        blit_glyph_styled(&mut c, 'A', 0, 0, 7, 13, fg, None, 0, None);
        let face = crate::render::vga16::glyph('A').expect("in the 8x16 subset");
        for dy in 0..13u32 {
            let got = (0..7u32).fold(0u8, |a, x| a | if *c.get_pixel(x, dy) == fg { 0x80 >> x } else { 0 });
            // `vga16` sampled into 7 columns, which is `source_col`'s text map.
            let want = (0..7u32).fold(0u8, |a, dx| {
                a | if face[(dy * 16 / 13) as usize] & (0x80 >> source_col(dx, 7, false)) != 0 {
                    0x80 >> dx
                } else {
                    0
                }
            });
            assert_eq!(got, want, "row {dy} of a 7x13 `A` is not the 8x16 face's");
        }
    }

    /// **The release's own face still wins.** `FONT` 524 off a Macintosh floppy is
    /// the real answer at this cell (SQ-1011) and the embedded face is what stands
    /// in when there is no volume; a stand-in that outranked the disk would be a
    /// regression wearing a fix's clothes.
    #[test]
    fn the_releases_own_face_outranks_the_embedded_one() {
        // A synthetic 7x15 fixed face whose row `y` of code `c` is `(c + y) as u8`,
        // MSB-leftmost — nothing like any real glyph, so its presence is unmistakable.
        let glyphs: Vec<blorb::bitmap_font::Glyph> = (0x20u8..=0x7E)
            .map(|c| blorb::bitmap_font::Glyph {
                width: 7,
                rows: (0..15u8).map(|y| c.wrapping_add(y)).collect(),
            })
            .collect();
        let font = blorb::bitmap_font::BitmapFont {
            width: 7,
            height: 15,
            baseline: 12,
            bold_smear: 0,
            proportional: false,
            lo: 0x20,
            glyphs,
        };
        let profile = crate::interpreter::InterpreterProfile::Macintosh;
        let faces = crate::native_font::FaceSet::release(font, profile, None);
        assert!(faces.body().is_some(), "non-vacuity: the synthetic face was admitted");
        let tf = crate::native_font::TextFace::new(profile, faces, None);
        assert_eq!((tf.cell().w(), tf.cell().h()), (7, 15), "non-vacuity: this IS the Macintosh cell");

        // Masked to the cell's seven columns: the face is 7 wide, so bit 0 of each
        // row is past its right edge and the blit never reads it.
        let want: Vec<u8> = (0..15u8).map(|y| b'A'.wrapping_add(y) & 0xFE).collect();
        assert_eq!(rendered_rows_7x15('A', 0, Some(&tf)).to_vec(), want, "the disk face draws, 1:1");
        assert_ne!(
            rendered_rows_7x15('A', 0, Some(&tf)),
            rendered_rows_7x15('A', 0, None),
            "and it is not the embedded face",
        );
    }

    /// **Bold on the 7-wide face keeps every roman pixel and stays in its cell.**
    ///
    /// The two guarantees `synthesize_face`/`synthesize_face16` give, checked on the
    /// face that has NO spare column: bit 0 of each row is BDF padding rather than a
    /// column, so the smear off column 6 lands somewhere the blit never samples —
    /// a clip, not a bleed (SQ-1016).
    #[test]
    fn narrow_bold_is_a_superset_and_stays_inside_the_cell() {
        for ch in ['m', 'A', 'W', 'g', 'T'] {
            let roman = rendered_rows_7x15(ch, 0, None);
            let bold = rendered_rows_7x15(ch, STYLE_BOLD, None);
            for (k, (&r, &b)) in roman.iter().zip(bold.iter()).enumerate() {
                assert_eq!(b & r, r, "{ch:?} row {k}: bold dropped ink the roman face had");
            }
            assert!(bold != roman, "{ch:?}: bold must differ from roman");
        }
        // Nothing lands outside the cell, at any style.
        let fg = Rgba([255, 0, 0, 255]);
        for style in [0u8, STYLE_BOLD, STYLE_ITALIC, STYLE_BOLD | STYLE_ITALIC] {
            for ch in ['m', 'T', 'W', 'y'] {
                let mut c = RgbaImage::from_pixel(21, 45, Rgba([0, 0, 0, 0]));
                blit_glyph_styled(&mut c, ch, 7, 15, 7, 15, fg, Some(Rgba([9, 9, 9, 255])), style, None);
                for (x, y, p) in c.enumerate_pixels() {
                    if !((7..14).contains(&x) && (15..30).contains(&y)) {
                        assert_eq!(*p, Rgba([0, 0, 0, 0]), "{ch:?} style {style} painted ({x},{y}) outside its cell");
                    }
                }
            }
        }
    }

    /// **Italic leans the top forward and no letter loses its rightmost stroke**
    /// (SQ-1016).
    ///
    /// The shear moves ink RIGHT, so only ink in column 6 can fall off — and `T`,
    /// `Ð`, `×` and `æ` are the only glyphs in the whole subset that ink column 6.
    /// What `T` loses is one pixel of its crossbar, whose right end still reaches
    /// the cell edge; every letter, `y` among them, keeps all of its ink.
    #[test]
    fn narrow_italic_leans_forward_and_keeps_the_right_edge() {
        let roman = rendered_rows_7x15('l', 0, None);
        let italic = rendered_rows_7x15('l', STYLE_ITALIC, None);
        let top = (0..8).find(|&k| roman[k] != 0).expect("`l` has ink in its top half");
        assert_eq!(italic[top], roman[top] >> 1, "the top of `l` must move right, not left");
        let bottom = (8..15).find(|&k| roman[k] != 0).expect("`l` has ink in its bottom half");
        assert_eq!(italic[bottom], roman[bottom], "the bottom of `l` stays put");

        // No LETTER loses ink to the edge — `y` is the one the tail makes interesting.
        for ch in ('a'..='z').chain('A'..='Z') {
            let (r, i) = (rendered_rows_7x15(ch, 0, None), rendered_rows_7x15(ch, STYLE_ITALIC, None));
            let ink = |rows: [u8; 15]| rows.iter().map(|r| r.count_ones()).sum::<u32>();
            if ch == 'T' {
                continue;
            }
            assert_eq!(ink(i), ink(r), "{ch:?}: the shear dropped a stroke off the right edge");
        }
        // `T` is the one that pays, and it pays exactly one pixel per crossbar row
        // while still reaching column 6.
        let (r, i) = (rendered_rows_7x15('T', 0, None), rendered_rows_7x15('T', STYLE_ITALIC, None));
        let bar = r.iter().position(|&x| x != 0).expect("`T` has a crossbar");
        assert_eq!(i[bar].count_ones() + 1, r[bar].count_ones(), "`T`'s crossbar loses exactly one pixel");
        assert!(i[bar] & 0x02 != 0, "and still reaches column 6: {:#010b}", i[bar]);
    }

    /// The 7-wide face carries the same 194 codepoints as the 8-wide one, which is
    /// what makes leaving it out of [`has_glyph`] a no-op rather than an omission
    /// (SQ-1016). A regeneration that widened it would change that, and this is
    /// where it would say so.
    #[test]
    fn the_narrow_face_can_never_widen_this_answer() {
        for c in 0u32..=0xFFFF {
            let Some(ch) = char::from_u32(c) else { continue };
            if crate::render::misc7x14::glyph(ch).is_some() {
                assert!(
                    has_glyph(ch),
                    "U+{c:04X} is in the 7-wide face and in neither master — `has_glyph` now under-reports",
                );
            }
        }
    }

    /// A `TextFace` with no face behind it, on `profile` — the cell path with that
    /// machine's emphasis rule.
    fn face_for(profile: crate::interpreter::InterpreterProfile) -> crate::native_font::TextFace {
        crate::native_font::TextFace::new(profile, crate::native_font::FaceSet::none(), None)
    }

    /// **§8.7.1's Italic bit is a RULE on the machines that shipped Version 6, and
    /// the rule is the bottom of the cell** (SQ-1028).
    ///
    /// The standard offers "rendering italic with underlining" as its own example, so
    /// this is a fidelity question rather than a compliance one, and the two machines
    /// Infocom wrote a v6 interpreter for both answer it the same way. Measured on
    /// `machine-screenshots/amiga-shogun-game.png`: `Erasmus` in "This is the bridge
    /// of the Erasmus, a Dutch merchant" carries a solid rule and the words beside it
    /// carry none. Row by row, the glyph ink runs 336..349 and the rule 350..351
    /// against a sixteen-row line pitch — the cell's last row, abutting the letters
    /// with no gap. `mac-shogun.jpg` rules under the same word on the same frame.
    ///
    /// Falsified by restoring the shear: the bottom row comes back empty and the top
    /// half comes back displaced.
    #[test]
    fn an_emphasised_run_is_ruled_not_sloped_on_the_machines_that_shipped_v6() {
        for profile in
            [crate::interpreter::InterpreterProfile::Amiga, crate::interpreter::InterpreterProfile::Macintosh]
        {
            let tf = face_for(profile);
            assert!(tf.underlines_emphasis(), "{profile:?} rules under an emphasised run");
            let fg = Rgba([255, 0, 0, 255]);
            let mut canvas = RgbaImage::from_pixel(8, 16, Rgba([0, 0, 0, 0]));
            blit_glyph_styled(&mut canvas, 'n', 0, 0, 8, 16, fg, None, STYLE_ITALIC, Some(&tf));
            // The rule spans the cell's full width, so the glyph beside it joins on.
            let bottom: Vec<bool> = (0..8).map(|x| *canvas.get_pixel(x, 15) == fg).collect();
            assert!(bottom.iter().all(|&b| b), "{profile:?}: the rule must span the cell, got {bottom:?}");
            // …and it is ONE MASTER ROW thick against a sixteen-row line, which is
            // the two rows the Amiga capture measures.
            assert!(
                (0..8).all(|x| *canvas.get_pixel(x, 14) == fg),
                "{profile:?}: the rule is one master row (two native rows at ch=16)",
            );
            assert!(
                !(0..8).all(|x| *canvas.get_pixel(x, 13) == fg),
                "{profile:?}: and no thicker than that",
            );
            // The glyph itself is ROMAN — a sloped-and-ruled glyph is neither machine.
            let roman = rendered_rows('n', 0);
            let ruled = rendered_rows_with('n', STYLE_ITALIC, Some(&tf));
            for row in 0..13usize {
                assert_eq!(
                    ruled[row], roman[row],
                    "{profile:?} row {row}: an emphasised glyph must not ALSO be sheared",
                );
            }
        }
    }

    /// A machine with no capture keeps the synthesised slope — `machine-screenshots/`
    /// has no PC frame with an emphasised run in it, so the IBM PC is UNMEASURED
    /// rather than known, and a bare story file has no machine to be faithful to
    /// (SQ-1028).
    #[test]
    fn an_unmeasured_machine_keeps_the_slope() {
        let tf = face_for(crate::interpreter::InterpreterProfile::IbmPc);
        assert!(!tf.underlines_emphasis(), "the IBM PC has no capture and does not move");
        let fg = Rgba([255, 0, 0, 255]);
        let mut canvas = RgbaImage::from_pixel(8, 16, Rgba([0, 0, 0, 0]));
        blit_glyph_styled(&mut canvas, 'n', 0, 0, 8, 16, fg, None, STYLE_ITALIC, Some(&tf));
        assert!(
            !(0..8).all(|x| *canvas.get_pixel(x, 15) == fg),
            "no rule on a machine that slopes",
        );
        assert_eq!(
            rendered_rows_with('L', STYLE_ITALIC, Some(&tf)),
            rendered_rows('L', STYLE_ITALIC),
            "and the slope is exactly what it always was",
        );
    }

}



