//! The floating tooltip box: a few lines of text, drawn opaquely on top of
//! whatever is underneath, anchored to the thing the pointer is on.
//!
//! Lifted out of `render::debug_panel` when the border controls needed the same
//! box (SQ-1123). One implementation, so the debugger's value tip and a border
//! control's hint clamp, frame and paint identically — and both read the same
//! `tooltip.background` / `tooltip.border` selectors (§2d).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::draw_str_clipped;
use super::paneframe::{draw_pane_frame, BorderStyle, PaneGlyphs};
use crate::symbols::SymbolSet;
use crate::theme::resolve::Theme;

/// Which side of the anchor a tip prefers to sit on (SQ-1123).
///
/// The preference is about which way is INTO the thing the anchor sits on: a
/// control in a pane's top border wants its hint to drop into the pane, and one
/// in the bottom border wants it to rise into the pane. Either way the box never
/// covers the cell being pointed at, and either way it flips to the other side
/// when the preferred one would run off `area`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipSide {
    Below,
    Above,
}

/// [`draw_tip_on`] with the default preference: one row BELOW the anchor.
pub fn draw_tip(
    buf: &mut Buffer,
    area: Rect,
    col: u16,
    row: u16,
    lines: &[String],
    theme: &Theme,
    symbols: &SymbolSet,
) -> Option<Rect> {
    draw_tip_on(buf, area, col, row, lines, theme, symbols, TipSide::Below)
}

/// Draw `lines` as a floating box anchored at `(col, row)`, on `side` of it.
///
/// **Placement.** The box is CENTRED on the anchor's column and sits two rows
/// off it on the preferred side, with the pointer in the row between (SQ-1139).
/// Centring is what puts the pointer near the middle of the box: the pointer
/// itself cannot move — it has to aim at the icon — so the box is what moves.
/// Neither ever covers the cell being pointed at.
///
/// **The pointer is two cells wide and its apex therefore lands on a cell
/// BOUNDARY**, half a cell from the centre of the one-cell control it aims at.
/// That is inherent, not an oversight: no glyph in a range a patched font
/// guarantees draws a centred apex meeting a flat edge, and the one-cell
/// alternatives are worse (see [`crate::symbols::TipGlyphs`]). It straddles the
/// anchor's left edge, which is the arrangement that was judged by eye.
///
/// **Edges.** The box is clamped inside `area`: it slides when it would overrun
/// a side, and FLIPS to the other side of the anchor when the preferred one
/// would overrun. The POINTER is then clamped into the box's own span, so a box
/// pushed off-centre by an edge keeps a pointer attached to it rather than one
/// floating in the gap — the pointer drifts off the box's middle exactly as far
/// as the edge forced the box to move, and no further. A box that cannot fit in
/// `area` at all is skipped rather than drawn partially, so this never panics on
/// a small pane.
///
/// Returns the rect painted — the BOX, not counting the pointer row, since that
/// is the rect callers reason about. `None` when nothing was drawn.
pub fn draw_tip_on(
    buf: &mut Buffer,
    area: Rect,
    col: u16,
    row: u16,
    lines: &[String],
    theme: &Theme,
    symbols: &SymbolSet,
    side: TipSide,
) -> Option<Rect> {
    let inner = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as u16;
    let n = lines.len() as u16;
    if n == 0 {
        return None;
    }

    let style = theme.get("tooltip.background").style;
    // Optional frame (§2d): borderless by default; a themed `tooltip.border`
    // style wraps the box in a frame (colour + glyphs from that selector).
    let border = theme.get("tooltip.border");
    let box_style = border.border.unwrap_or(BorderStyle::None);
    let bordered = !matches!(box_style, BorderStyle::None);

    // Content is `inner` wide with one space of padding each side; a frame (when
    // set) adds one more cell all around.
    let pad_w = inner + 2;
    let (w, h) = if bordered { (pad_w + 2, n + 2) } else { (pad_w, n) };
    // The pointer costs one row on top of the box, and it is not optional: a box
    // that fits only without its spur would jump between pointed and unpointed as
    // the pane resized.
    let h_total = h + 1;
    if area.width < w || area.height < h_total {
        return None;
    }

    // CENTRED on the anchor, not started at it: that is what puts the pointer
    // near the middle of the box, since the pointer cannot move off the icon.
    let mut x = col.saturating_sub(w / 2);
    if x + w > area.right() {
        x = area.right().saturating_sub(w);
    }
    x = x.max(area.x);

    // Which side has room for the box AND its pointer row. The preference wins
    // whenever it fits; otherwise it flips; when neither fits the preference
    // stands and the clamp below keeps the box whole and inside.
    let fits_below = row + 1 + h_total <= area.bottom();
    let fits_above = row >= area.y + h_total;
    let below = match side {
        TipSide::Below => fits_below || !fits_above,
        TipSide::Above => !fits_above && fits_below,
    };

    let mut y = if below { row + 2 } else { row.saturating_sub(1 + h) };
    y = y.max(area.y);
    if y + h > area.bottom() {
        y = area.bottom().saturating_sub(h).max(area.y);
    }

    let box_rect = Rect::new(x, y, w, h);
    // Reset every cell the box covers before drawing: draw_char_clipped PATCHES
    // cell styles, so a modifier already on what is underneath (e.g. the
    // UNDERLINED on a clickable operand) would otherwise bleed through the
    // tooltip. A clean reset makes the box fully opaque.
    for yy in box_rect.y..box_rect.bottom() {
        for xx in box_rect.x..box_rect.right() {
            if let Some(cell) = buf.cell_mut((xx, yy)) {
                cell.reset();
            }
        }
    }
    // Fill the whole box with the tooltip background.
    let pad: String = " ".repeat(w as usize);
    for ry in box_rect.y..box_rect.bottom() {
        draw_str_clipped(buf, x, ry, &pad, style, box_rect);
    }
    // Frame in tooltip.border's colour, then position the text inside it.
    let (tx, ty) = if bordered {
        draw_pane_frame(buf, box_rect, box_style, &PaneGlyphs::default(), border.style);
        (x + 1, y + 1)
    } else {
        (x, y)
    };
    for (i, line) in lines.iter().enumerate() {
        draw_str_clipped(buf, tx + 1, ty + i as u16, line, style, box_rect);
    }
    draw_pointer(buf, area, box_rect, col, below, style, symbols);
    Some(box_rect)
}

/// The wedge that aims the box at its anchor (SQ-1139).
///
/// Drawn in the box's own BACKGROUND as its foreground, so it reads as the box
/// growing a spur rather than as two characters parked above one. That trick is
/// the whole design, and it has one hard requirement: the box must HAVE a
/// background. A theme that leaves `tooltip.background` without a `bg` gets no
/// pointer rather than a pair of glyphs in some inherited colour pointing at
/// nothing — there is no visible box for them to belong to.
fn draw_pointer(
    buf: &mut Buffer,
    area: Rect,
    box_rect: Rect,
    col: u16,
    below: bool,
    style: ratatui::style::Style,
    symbols: &SymbolSet,
) {
    /// Both presets are two cells; see `symbols::TipGlyphs`.
    const PTR_W: u16 = 2;
    let Some(fill) = style.bg else { return };
    if box_rect.width < PTR_W {
        return;
    }
    // Derived from the box rather than from `row`, so the two are adjacent by
    // construction however far the clamp above had to move the box.
    let y = if below { box_rect.y.checked_sub(1) } else { Some(box_rect.bottom()) };
    let Some(y) = y.filter(|y| *y >= area.y && *y < area.bottom()) else { return };

    // Straddle the anchor's LEFT edge — the arrangement judged by eye — then keep
    // the wedge inside the box's span so an edge-clamped box never grows a
    // detached spur.
    let px = col
        .saturating_sub(1)
        .clamp(box_rect.x, box_rect.right().saturating_sub(PTR_W));
    let (left, right) = symbols.tip.wedge(below);
    let ptr = ratatui::style::Style::default().fg(fill);
    for (i, ch) in [left, right].into_iter().enumerate() {
        let cx = px + i as u16;
        if let Some(cell) = buf.cell_mut((cx, y)) {
            // A pointer cell is HALF box and half whatever it lies on, and only
            // the box half is ours to paint. The wedge is a foreground glyph: the
            // colour outside its diagonal is the cell's BACKGROUND, so that
            // background has to stay the surface the tip is floating over — the
            // story pane's fill, the map's canvas, a dialog's face.
            //
            // Hence read-then-reset rather than reset alone. `reset` is still
            // needed, because a modifier already on what is underneath (an
            // underlined operand in the disassembly, say) would otherwise bleed
            // through the spur exactly as it would through the box; but it clears
            // the background too, and leaving it cleared drops the cell to the
            // TERMINAL's default. That reads as a notch of bare terminal punched
            // through the pane above the tip, which is what it is.
            //
            // The underlying cell's VISIBLE background is not always its `bg`:
            // the selected room box is painted REVERSED, so its `fg`/`bg` are
            // swapped on screen (SQ-1279). Read the cell's *visible* colours —
            // `fg` under REVERSED, `bg` otherwise — so the room's actual ground
            // runs through the pointer cell instead of showing the dark block a
            // literal `bg` read would give. The pointer keeps the tip's own
            // colour as its ink and is drawn without REVERSED regardless of what
            // was underneath, since `ptr` sets both colours explicitly.
            let under_style = cell.style();
            let visible_bg = if under_style.add_modifier.contains(ratatui::style::Modifier::REVERSED) {
                under_style.fg
            } else {
                under_style.bg
            };
            cell.reset();
            let style = match visible_bg {
                Some(bg) => ptr.bg(bg),
                // Nothing was painted here, so the terminal's own background IS
                // what is behind the pointer, and leaving it unset is correct.
                None => ptr,
            };
            cell.set_symbol(&ch.to_string()).set_style(style);
        }
    }
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use crate::state::AppState;

    /// Draw one tip and hand back the buffer, the box, and the symbols used.
    fn tip(area: Rect, col: u16, row: u16, text: &str, side: TipSide) -> (Buffer, Rect, SymbolSet) {
        let st = AppState::default();
        let mut buf = Buffer::empty(area);
        let lines = vec![text.to_string()];
        let r = draw_tip_on(&mut buf, area, col, row, &lines, &st.colors.theme, &st.symbols, side)
            .expect("the tip fits");
        (buf, r, st.symbols)
    }

    /// SQ-1139's headline: the box is a SURFACE, not the page it floats on.
    ///
    /// `tooltip.background` used to derive from `chrome`, whose bg *is* the page
    /// background — so a borderless tip was painted in exactly the colours it lay
    /// over, and the report was that it "blends into the background". It did not
    /// blend; it was the same colour. Nothing else in this module can be judged
    /// until this holds, because the pointer is drawn in this very colour.
    #[test]
    fn the_tip_is_painted_in_something_other_than_the_page() {
        let st = AppState::default();
        let tipbg = st.colors.theme.get("tooltip.background").style;
        let page = st.colors.theme.get("chrome").style;
        assert!(tipbg.bg.is_some(), "a tip with no background cannot be a surface");
        assert_ne!(tipbg.bg, page.bg, "the tip must not be painted in the page's own colour");
        assert_ne!(tipbg.fg, tipbg.bg, "and its ink must not be its own background");
    }

    /// The pointer is the box's background used as a foreground — that is what
    /// makes it read as one shape rather than as a character above a rectangle.
    #[test]
    fn the_pointer_is_drawn_in_the_boxs_own_fill() {
        let area = Rect::new(0, 0, 60, 20);
        let (buf, r, syms) = tip(area, 30, 4, "Toggle the map", TipSide::Below);
        let fill = AppState::default().colors.theme.get("tooltip.background").style.bg;

        let py = r.y - 1;
        let (l, rg) = syms.tip.wedge(true);
        let found: Vec<(u16, String)> = (r.x..r.right())
            .filter_map(|x| {
                let c = buf.cell((x, py))?;
                (c.symbol() == l.to_string() || c.symbol() == rg.to_string())
                    .then(|| (x, c.symbol().to_string()))
            })
            .collect();
        assert_eq!(found.len(), 2, "a two-cell wedge sits on the row between anchor and box");
        for (x, _) in &found {
            assert_eq!(
                buf.cell((*x, py)).unwrap().style().fg,
                fill,
                "the wedge is drawn in the box's fill colour, not an inherited one"
            );
        }
    }

    /// The pointer keeps the background of whatever it lies ON.
    ///
    /// Reported against the first cut: the wedge showed the TERMINAL's background
    /// outside its diagonal instead of the story pane's, so a notch of bare
    /// terminal appeared punched through the pane just above the tip. The box
    /// never had the bug because it repaints its own fill across every cell it
    /// covers; a pointer cell is half box and half pane, and only the box half is
    /// ours — the rest is the cell's background, which `Cell::reset` had cleared.
    #[test]
    fn the_pointer_keeps_the_surface_it_lies_on_behind_it() {
        use ratatui::style::{Color, Modifier, Style};
        let area = Rect::new(0, 0, 60, 20);
        let st = AppState::default();
        let pane = Color::Rgb(20, 30, 40); // stand in for the story pane's fill
        let mut buf = Buffer::empty(area);
        for cell in buf.content.iter_mut() {
            cell.set_style(Style::new().bg(pane).add_modifier(Modifier::UNDERLINED));
        }
        let lines = vec!["Toggle the map".to_string()];
        let r = draw_tip_on(
            &mut buf,
            area,
            30,
            4,
            &lines,
            &st.colors.theme,
            &st.symbols,
            TipSide::Below,
        )
        .expect("the tip fits");

        let (l, rg) = st.symbols.tip.wedge(true);
        let py = r.y - 1;
        let mut seen = 0;
        for x in r.x..r.right() {
            let cell = buf.cell((x, py)).unwrap();
            if cell.symbol() != l.to_string() && cell.symbol() != rg.to_string() {
                continue;
            }
            seen += 1;
            assert_eq!(cell.style().bg, Some(pane), "the wedge sits ON the pane, not on bare terminal");
            assert!(
                !cell.style().add_modifier.contains(Modifier::UNDERLINED),
                "…but a modifier underneath must still not bleed through it"
            );
        }
        assert_eq!(seen, 2, "both wedge cells were found");
    }

    /// The point of the whole placement rule: the pointer lands as near the
    /// middle of the box as a two-cell wedge can, because the box is centred on
    /// the anchor rather than started at it.
    #[test]
    fn the_pointer_sits_at_the_middle_of_an_unclamped_box() {
        let area = Rect::new(0, 0, 60, 20);
        let (buf, r, syms) = tip(area, 30, 4, "Toggle the map", TipSide::Below);
        let (l, _) = syms.tip.wedge(true);
        let px = (r.x..r.right())
            .find(|x| buf.cell((*x, r.y - 1)).unwrap().symbol() == l.to_string())
            .expect("the wedge is drawn");
        // Two cells cannot straddle a centre exactly; one off is the whole error.
        let middle = r.x + r.width / 2;
        assert!(
            px.abs_diff(middle) <= 1,
            "wedge at {px} against a box {r:?} whose middle is {middle}"
        );
        assert!(px >= r.x && px + 1 < r.right(), "and it stays inside the box's span");
    }

    /// An edge cannot detach the pointer from the box. The box is clamped inside
    /// `area`, so the wedge follows it rather than staying under an anchor the
    /// box can no longer reach — a spur floating in the gap would be worse than
    /// no spur at all.
    #[test]
    fn a_box_shoved_off_centre_by_an_edge_keeps_its_pointer_attached() {
        let area = Rect::new(0, 0, 24, 20);
        for col in [0u16, 1, 22, 23] {
            let (buf, r, syms) = tip(area, col, 4, "Toggle the map", TipSide::Below);
            let (l, _) = syms.tip.wedge(true);
            let px = (r.x..r.right())
                .find(|x| buf.cell((*x, r.y - 1)).unwrap().symbol() == l.to_string())
                .unwrap_or_else(|| panic!("no wedge for an anchor at {col}, box {r:?}"));
            assert!(
                px >= r.x && px + 1 < r.right(),
                "anchor {col}: wedge at {px} escapes box {r:?}"
            );
        }
    }

    /// A pointer cell over a plain highlighted cell (e.g. an explicit light `bg`,
    /// not REVERSED) shows that colour, not the terminal default (SQ-1279).
    #[test]
    fn the_pointer_shows_a_plain_cells_background() {
        use ratatui::style::{Color, Style};
        let area = Rect::new(0, 0, 60, 20);
        let st = AppState::default();
        let light = Color::Rgb(220, 220, 220);
        let mut buf = Buffer::empty(area);
        for cell in buf.content.iter_mut() {
            cell.set_style(Style::new().bg(light));
        }
        let tipcol = st.colors.theme.get("tooltip.background").style.bg;
        let lines = vec!["Toggle the map".to_string()];
        let r = draw_tip_on(&mut buf, area, 30, 4, &lines, &st.colors.theme, &st.symbols, TipSide::Below)
            .expect("the tip fits");

        let (l, rg) = st.symbols.tip.wedge(true);
        let py = r.y - 1;
        let mut seen = 0;
        for x in r.x..r.right() {
            let cell = buf.cell((x, py)).unwrap();
            if cell.symbol() != l.to_string() && cell.symbol() != rg.to_string() {
                continue;
            }
            seen += 1;
            assert_eq!(cell.style().bg, Some(light), "the pointer shows the plain cell's bg");
            assert_eq!(cell.style().fg, tipcol, "the pointer's ink stays the tip's own colour");
            assert!(
                !cell.style().add_modifier.contains(ratatui::style::Modifier::REVERSED),
                "the pointer never inherits REVERSED"
            );
        }
        assert_eq!(seen, 2, "both wedge cells were found");
    }

    /// A pointer cell over a REVERSED cell (the selected room box's ground, SQ-1279)
    /// shows that cell's *visible* background — its `fg`, since REVERSED swaps the
    /// two on screen — not a dark block punched through the light box.
    #[test]
    fn the_pointer_shows_a_reversed_cells_visible_background() {
        use ratatui::style::{Color, Modifier, Style};
        let area = Rect::new(0, 0, 60, 20);
        let st = AppState::default();
        let light = Color::Rgb(220, 220, 220);
        let dark = Color::Rgb(10, 10, 10);
        let mut buf = Buffer::empty(area);
        for cell in buf.content.iter_mut() {
            cell.set_style(Style::new().fg(light).bg(dark).add_modifier(Modifier::REVERSED));
        }
        let tipcol = st.colors.theme.get("tooltip.background").style.bg;
        let lines = vec!["Toggle the map".to_string()];
        let r = draw_tip_on(&mut buf, area, 30, 4, &lines, &st.colors.theme, &st.symbols, TipSide::Below)
            .expect("the tip fits");

        let (l, rg) = st.symbols.tip.wedge(true);
        let py = r.y - 1;
        let mut seen = 0;
        for x in r.x..r.right() {
            let cell = buf.cell((x, py)).unwrap();
            if cell.symbol() != l.to_string() && cell.symbol() != rg.to_string() {
                continue;
            }
            seen += 1;
            assert_eq!(
                cell.style().bg,
                Some(light),
                "the pointer takes the REVERSED cell's fg as its visible ground"
            );
            assert_eq!(cell.style().fg, tipcol, "the pointer's ink stays the tip's own colour");
            assert!(
                !cell.style().add_modifier.contains(Modifier::REVERSED),
                "the pointer is drawn with the colours set explicitly, not the inherited REVERSED"
            );
        }
        assert_eq!(seen, 2, "both wedge cells were found");
    }

    /// A pointer cell over a plain, unstyled cell (today's dark-terminal case)
    /// keeps today's look: no background set, i.e. the terminal's own default
    /// shows through, exactly as before this fix.
    #[test]
    fn the_pointer_over_an_unstyled_cell_is_unchanged() {
        let area = Rect::new(0, 0, 60, 20);
        let (buf, r, syms) = tip(area, 30, 4, "Toggle the map", TipSide::Below);
        let (l, rg) = syms.tip.wedge(true);
        let py = r.y - 1;
        let mut seen = 0;
        for x in r.x..r.right() {
            let cell = buf.cell((x, py)).unwrap();
            if cell.symbol() != l.to_string() && cell.symbol() != rg.to_string() {
                continue;
            }
            seen += 1;
            assert_eq!(
                cell.style().bg,
                Some(ratatui::style::Color::Reset),
                "an unstyled cell's default bg (Reset) is what shows through, unchanged from before this fix"
            );
        }
        assert_eq!(seen, 2, "both wedge cells were found");
    }

    /// Only the pointer row changes here — the tip BODY still carries the
    /// tooltip selector's own style, untouched by the pointer's background fix.
    #[test]
    fn the_tip_body_keeps_its_own_style() {
        let area = Rect::new(0, 0, 60, 20);
        let st = AppState::default();
        let (buf, r, _) = tip(area, 30, 4, "Toggle the map", TipSide::Below);
        let body_style = st.colors.theme.get("tooltip.background").style;
        let cell = buf.cell((r.x + 1, r.y)).expect("a body cell inside the tip");
        assert_eq!(cell.style().bg, body_style.bg, "the body cell keeps the tip selector's bg");
        assert_eq!(cell.style().fg, body_style.fg, "the body cell keeps the tip selector's fg");
    }

    /// The box never covers the cell it is explaining, on either side — the whole
    /// reason the pointer row exists rather than the box simply abutting.
    #[test]
    fn neither_box_nor_pointer_ever_covers_the_anchor() {
        let area = Rect::new(0, 0, 60, 20);
        for (side, row) in [(TipSide::Below, 4u16), (TipSide::Above, 15)] {
            let (_, r, _) = tip(area, 30, row, "Toggle the map", side);
            assert!(
                !(r.y..r.bottom()).contains(&row),
                "{side:?}: box {r:?} covers the anchor row {row}"
            );
            let py = if r.y > row { r.y - 1 } else { r.bottom() };
            assert_ne!(py, row, "{side:?}: the pointer covers the anchor row {row}");
        }
    }
}
