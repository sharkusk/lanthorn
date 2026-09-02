/// Render the engine's text-grid (upper) window atop the story pane transcript.
///
/// Public entry point: `draw_upper_window` — reads a neutral [`GridWindow`] from
/// the engine's `ScreenModel` and delegates to the testable `draw_grid` helper.
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::colors::ColorScheme;
use crate::engine::{BorderPref, GridCell, GridWindow};
use crate::render::paneframe::{draw_framed, BorderStyle, PaneSides};

/// Resolve a grid cell's game colour into a ratatui [`Style`], mirroring the
/// mechanism used by `draw_str_runs` in the transcript renderer.
///
/// Reverse video (style bit `0x01`) is realised via the ratatui `REVERSED`
/// modifier so the terminal performs exactly one swap. fg/bg are applied in
/// logical order (pre-reverse) for non-Default channels when
/// `honor_game_colours` is true; Default channels inherit from the theme base.
fn cell_style(cell: zvm::screen::Cell, glk_style: u8, scheme: &ColorScheme, honor_game_colours: bool, bg_override: Option<u32>, is_glk_grid: bool) -> Style {
    use zvm::screen::ZColour;
    // Use the theme's grid content style as the base (consistent with the
    // blank-fill path in draw_grid, and with how transcript.rs draws styled runs).
    // A Glk grid (win != 0) grounds on `glk.grid.background` — reversed chrome,
    // the status bar's own spelling (SQ-1212) — so an unwritten cell and a
    // written Default-colour cell agree; a Z-machine/Scott grid (win == 0) keeps
    // `upper_window`, which follows the terminal page (SQ-0510) untouched.
    // A per-window background override (Glulx window colour, SQ-0328) replaces the
    // theme bg here so a Default-bg game cell shows the window's own colour.
    let mut base = scheme.theme.get(if is_glk_grid { "glk.grid.background" } else { "upper_window" }).style;
    if let Some(rgb) = bg_override {
        base = window_ground(base, rgb, scheme);
    }
    // Per-channel colour resolution (SQ-0331): game-set cell colour (gated by
    // honor_game_colours), then the theme's per-Glk-style slot (grid = row 1),
    // then the element base. Mirrors draw_str_runs in transcript.rs exactly.
    let glk = scheme.glk_styles[1].get(glk_style as usize).copied().unwrap_or_default();
    let game = |c: ZColour| -> Option<ratatui::style::Color> {
        (!matches!(c, ZColour::Default)).then(|| crate::render::resolve_zcolour(c, scheme))
    };
    let game_fg = honor_game_colours.then(|| game(cell.fg)).flatten();
    let game_bg = honor_game_colours.then(|| game(cell.bg)).flatten();
    // SQ-1219: a Glk grid's ground is realised via `Modifier::REVERSED` (SQ-1212),
    // so an explicit `.fg()`/`.bg()` laid on top of `base` as-is would be swapped
    // by the terminal along with the rest — a colour meant as a foreground lands
    // as a background patch instead. Only when the GAME actually set a colour for
    // this cell does the ground need resolving to concrete, un-reversed fg/bg
    // first, so the explicit channel means what it says. A cell with no game
    // colour is untouched: the ground's own REVERSED stays, which the cursor XOR
    // toggle (in `draw_grid`/`draw_grid_transparent`) depends on for "exactly one
    // terminal swap".
    if is_glk_grid && (game_fg.is_some() || game_bg.is_some()) {
        base = concretize_reversed(base);
    }
    // apply_text_style adds REVERSED for bit 0x01, BOLD for 0x02, ITALIC for 0x04.
    // The terminal performs exactly one swap for the REVERSED modifier — no manual
    // fg/bg swap here (which would be a no-op for Default/Reset channels, C1 bug).
    let mut s = crate::render::apply_text_style(base, cell.style);
    if let Some(c) = crate::render::resolve_glk_channel(game_fg, glk.fg, base.fg, honor_game_colours) {
        s = s.fg(c);
    }
    // The theme's per-Glk-style slot never paints a Glk grid cell's BACKGROUND
    // (SQ-1219): only the ground (`base.bg`, above) or a colour the GAME itself
    // set may replace it, so an unstyled cell always blends into the reversed
    // band — a themed slot bg would paint its own patch the same way the
    // hyperlink accent used to. The slot's fg is unaffected (styling text colour
    // doesn't break the blend the ground exists for).
    let glk_bg = if is_glk_grid { None } else { glk.bg };
    if let Some(c) = crate::render::resolve_glk_channel(game_bg, glk_bg, base.bg, honor_game_colours) {
        s = s.bg(c);
    }
    let glk_mods =
        crate::render::glk_theme_modifiers(scheme, true, glk_style as usize) | glk.add_modifier;
    if !glk_mods.is_empty() {
        s = s.add_modifier(glk_mods);
    }
    s
}

/// Resolve a `Modifier::REVERSED` style to concrete, already-swapped fg/bg with
/// the modifier dropped — so a channel written on top afterwards (`.fg()`,
/// `.bg()`, `.patch()`) means what it says instead of being swapped a second
/// time by the terminal. A style with no REVERSED bit is returned unchanged.
///
/// SQ-1219: a Glk grid's ground (`glk.grid.background`, SQ-1212) is realised via
/// this modifier. City of Secrets' `help` menu hyperlinks patch the themed
/// `hyperlink` accent colour (an explicit fg, no bg) onto that ground; without
/// this, the terminal's single swap put the accent colour on the WRONG side of
/// the character — a teal background patch behind black text — instead of teal
/// link text on the ground's own background.
fn concretize_reversed(style: Style) -> Style {
    if !style.add_modifier.contains(Modifier::REVERSED) {
        return style;
    }
    Style { fg: style.bg, bg: style.fg, ..style }.remove_modifier(Modifier::REVERSED)
}

/// The ground of a grid whose game gave it a window background of its own
/// (SQ-0328): the theme's ground with `rgb` behind it. A Glk grid's ground is
/// reversed chrome (SQ-1212), and `.bg()` laid on a REVERSED style is what the
/// terminal swaps into the FOREGROUND — the window's colour became the ink and
/// the chrome's ink the visible background, so every unwritten cell of
/// Kerkerkruip's light-grey status window was black. Resolve the reversal
/// first, so the colour lands where the game put it.
fn window_ground(ground: Style, rgb: u32, scheme: &ColorScheme) -> Style {
    concretize_reversed(ground).bg(crate::render::resolve_zcolour(zvm::screen::ZColour::True24(rgb), scheme))
}

/// Convert a neutral [`GridCell`] (packed colour) into a `zvm::screen::Cell`
/// (typed `ZColour`) for [`cell_style`].
fn grid_cell_to_zvm(cell: GridCell) -> zvm::screen::Cell {
    zvm::screen::Cell {
        ch: cell.ch,
        style: cell.style,
        fg: crate::state::unpack_zcolour(cell.fg),
        bg: crate::state::unpack_zcolour(cell.bg),
    }
}

/// The per-side border styles `draw_grid` will actually use for `grid`, after
/// applying the game's border PRESENCE over the theme's glyph/colour (SQ-0286):
/// - `NoBorder` (an explicit Glulx `winmethod_NoBorder`) draws no sides, whatever
///   the theme;
/// - `Border` (an explicit Glulx `winmethod_Border`) uses the theme's sides, or
///   falls back to a single-line full frame if the theme disabled every side, so
///   the game's request is visibly honored;
/// - `Unspecified` (Z-machine, default, parentless root) defers entirely to the
///   theme — frameless when the theme turns the border off.
pub fn resolved_grid_sides(grid: &GridWindow, colors: &ColorScheme) -> PaneSides {
    match grid.border {
        BorderPref::NoBorder => PaneSides::all(BorderStyle::None),
        BorderPref::Border => {
            if colors.upper_window_border_sides.any_on() {
                colors.upper_window_border_sides
            } else {
                PaneSides::all(BorderStyle::Single)
            }
        }
        BorderPref::Unspecified => colors.upper_window_border_sides,
    }
}

/// Terminal rows the grid's border chrome adds on top of its content rows
/// (top + bottom borders), honoring the game's border presence (SQ-0286). The
/// generic multi-window path widens a stacked grid's allotment by this much so
/// the chrome isn't squished into the grid's exact Glk split (SQ-0200);
/// `draw_grid` sizes its own frame with it too.
pub fn grid_border_overhead(grid: &GridWindow, colors: &ColorScheme) -> u16 {
    let sides = resolved_grid_sides(grid, colors);
    (if sides.top != BorderStyle::None { 1 } else { 0 })
        + (if sides.bottom != BorderStyle::None { 1 } else { 0 })
}

/// The pane columns [`draw_grid`] draws `upper`'s own grid columns on inside
/// `area`: the column its FIRST grid column lands on, and how many of them fit.
///
/// The grid is the GAME's screen (`upper.cols` wide), not the pane, so a pane
/// wider than the game centres it — see the placement in `draw_grid`, which is
/// this function. A pane narrower than the game left-aligns and clips.
///
/// Public because a caller that has to **invert** the placement must ask the
/// drawing where it put the text rather than restate the arithmetic. The v6
/// hybrid click map is that caller (SQ-0951): it resolves a click by the cell
/// the character under it was drawn in, and it had been assuming the game's
/// first column sat on the story viewport's first column. It does not when the
/// viewport is wider than the game — Zork Zero's InvisiClues grid is 58 columns
/// against a 138-column viewport at a 190x60 pane, so every topic is drawn forty
/// columns right of where the map looked for it, and the player had to click far
/// to the left of a topic to select it.
pub fn grid_content_x_span(upper: &GridWindow, colors: &ColorScheme, area: Rect) -> (u16, u16) {
    let sides = resolved_grid_sides(upper, colors);
    let left = u16::from(sides.left != BorderStyle::None);
    let border_cols = left + u16::from(sides.right != BorderStyle::None);
    let uw_w = upper.cols.saturating_add(border_cols).min(area.width).max(1);
    let x_off = area.width.saturating_sub(uw_w) / 2;
    (area.x + x_off + left, uw_w.saturating_sub(border_cols))
}

// ── Core grid renderer ────────────────────────────────────────────────────────

/// Draw the upper-window grid into `area`.
///
/// - `upper`: the neutral grid to render (the `ScreenModel`'s text-grid window).
/// - `upper_rows`: the active row count (`GridWindow::active_rows`).
/// - `cursor`: 1-based (row, col) of the grid cursor.
/// - `show_cursor`: when true, mark the cursor cell (e.g. while the game is
///   awaiting input in the upper window) so forms show where typing lands.
/// - `colors`: resolved color scheme.
/// - `area`: target rectangle in the buffer.
///
/// Returns the number of terminal rows consumed (0 when `upper_rows == 0`).
pub fn draw_grid(
    upper: &GridWindow,
    upper_rows: u16,
    cursor: (u16, u16),
    show_cursor: bool,
    colors: &ColorScheme,
    area: Rect,
    buf: &mut Buffer,
    honor_game_colours: bool,
    links: &mut Vec<((u16, u16), u32)>,
) -> u16 {
    if upper_rows == 0 || area.height == 0 || area.width == 0 {
        return 0;
    }

    // A Glk grid (a real Glk window id, `win != 0`) grounds on `glk.grid.background`
    // — reversed chrome, the status bar's own spelling — so the game's own grid
    // reads as a visible chrome band instead of page-on-page (SQ-1212). A
    // Z-machine/Scott grid (`win == 0`) keeps `upper_window`, which follows the
    // terminal page (SQ-0510) and is untouched: those games paint their own
    // reversal, and a default reverse here would double it up.
    let is_glk_grid = upper.win != 0;
    let ground_selector = if is_glk_grid { "glk.grid.background" } else { "upper_window" };

    // Per-window background override (Glulx window colour, SQ-0328): when the grid
    // carries its own `bg`, the content fill and each cell's default background use
    // it instead of the theme's ground bg. `None` (Z-machine simple path,
    // default) leaves the behaviour byte-identical.
    let uw = colors.theme.get(ground_selector).style;
    let mut content_style = match upper.bg {
        Some(rgb) => window_ground(uw, rgb, colors),
        None => uw,
    };
    // If the game reversed the grid's Normal style with no explicit colours
    // (Counterfeit Monkey's menu sets ReverseColor on every grid style), the empty
    // fill must invert the theme base too — otherwise the unwritten cells show the
    // non-reversed base (white) while the reverse-video text is dark. (SQ-0403)
    if upper.reverse {
        content_style = content_style.add_modifier(ratatui::style::Modifier::REVERSED);
    }
    let border_color = colors.theme.get("upper_window_border").style;

    // Resolve which sides to frame. The game controls border PRESENCE (SQ-0286);
    // the theme controls only the glyph + colour (see `resolved_grid_sides`).
    let sides = resolved_grid_sides(upper, colors);

    // How many terminal rows does the border frame consume? (top + bottom sides).
    let border_overhead: u16 = grid_border_overhead(upper, colors);

    // Total terminal rows needed: grid rows + optional border.
    // Clamp to the available area height.
    let needed = upper_rows.saturating_add(border_overhead).min(area.height);

    // The upper window is the game's screen (`upper.cols` wide) — NOT the pane.
    // Size the region to the game screen width (+ side borders) and CENTER it in
    // the pane, so a game that centers its own content (e.g. Bureaucracy's
    // full-width forms / status) lines up under our border instead of being
    // stretched to the pane edge. When the pane is narrower than the game screen,
    // use the full pane width and left-align (the col-offset scroll below handles
    // the overflow).
    //
    // The horizontal placement itself is [`grid_content_x_span`] — one piece of
    // arithmetic, so the v6 click map inverts exactly what is drawn here instead
    // of restating it (SQ-0951).
    let left_border: u16 = if sides.left != BorderStyle::None { 1 } else { 0 };
    let border_cols: u16 =
        left_border + (if sides.right != BorderStyle::None { 1 } else { 0 });
    let uw_w = upper.cols.saturating_add(border_cols).min(area.width).max(1);
    let (content_x, _) = grid_content_x_span(upper, colors, area);

    // Carve out the centered top region for the upper window.
    let uw_area = Rect::new(content_x - left_border, area.y, uw_w, needed);

    // Draw the optional border and get the inner content rect.
    let frame = draw_framed(buf, uw_area, sides, &colors.upper_window_border_glyphs, border_color, false);
    let content = frame.content;

    if content.height == 0 || content.width == 0 {
        return needed;
    }

    // Viewport auto-follow: scroll so cursor stays visible.
    // cursor is 1-based; convert to 0-based for arithmetic.
    let (crow, ccol) = (
        cursor.0.saturating_sub(1),
        cursor.1.saturating_sub(1),
    );
    // Row viewport offset: scroll down so cursor row is visible.
    let row_offset: u16 = if crow >= content.height {
        crow.saturating_sub(content.height - 1)
    } else {
        0
    };
    // Col viewport offset: scroll right so the cursor column is visible — but
    // ONLY while the cursor is the player's (`show_cursor`: the game is waiting
    // on a keypress in this grid, an in-place form). (SQ-0679)
    //
    // Otherwise the left of the row wins. A grid wider than the pane is almost
    // always a status bar, and its content — the room name — starts at column 1
    // while the cursor is parked wherever the game's last print left it, out to
    // the right. Following that parked cursor scrolled the room name off the
    // left edge and showed the score/moves fields floating alone. Nobody is
    // typing there; there is no caret to keep on screen.
    let col_offset: u16 = if show_cursor && ccol >= content.width {
        ccol.saturating_sub(content.width - 1)
    } else {
        0
    };

    // Fill content area with background style.
    for dy in 0..content.height {
        for dx in 0..content.width {
            let bx = content.x + dx;
            let by = content.y + dy;
            if let Some(cell) = buf.cell_mut((bx, by)) {
                cell.set_symbol(" ").set_style(content_style);
            }
        }
    }

    // Render each visible grid cell.
    // Grid rows/cols are 1-based; viewport offsets are 0-based.
    for dy in 0..content.height {
        let grid_row = dy + row_offset + 1; // 1-based
        if grid_row > upper_rows {
            break;
        }
        for dx in 0..content.width {
            let grid_col = dx + col_offset + 1; // 1-based
            if grid_col > upper.cols {
                break;
            }
            let cell = upper.cell(grid_row, grid_col);
            let bx = content.x + dx;
            let by = content.y + dy;
            if let Some(buf_cell) = buf.cell_mut((bx, by)) {
                let mut style = cell_style(grid_cell_to_zvm(cell), cell.glk_style, colors, honor_game_colours, upper.bg, is_glk_grid);
                // Glk hyperlink affordance: layer the themeable `hyperlink` colour
                // and an underline on top, and record the cell for click hit-testing.
                // Mirrors the transcript path in `draw_str_runs`. (SQ-0258)
                if cell.link != 0 {
                    if honor_game_colours {
                        // SQ-1219: on a Glk grid, `style` may still carry the
                        // ground's REVERSED (cell_style only drops it for a
                        // GAME-set colour, and a hyperlink has none) — resolve to
                        // concrete fg/bg first so patching the accent fg on top
                        // means text colour, not a swapped background patch.
                        if is_glk_grid {
                            style = concretize_reversed(style);
                        }
                        style = style.patch(colors.theme.get("hyperlink").style);
                    }
                    style = style.add_modifier(ratatui::style::Modifier::UNDERLINED);
                    links.push(((bx, by), cell.link));
                }
                let mut ch_buf = [0u8; 4];
                // Control chars would trip ratatui's cell_width debug assert.
                let ch = if cell.ch.is_control() { ' ' } else { cell.ch };
                buf_cell.set_symbol(ch.encode_utf8(&mut ch_buf)).set_style(style);
            }
        }
    }

    // Cursor: XOR bit 0x01 into the cell's style before calling cell_style, so
    // apply_text_style reflects the toggled reverse. Cursor on a normal cell adds
    // REVERSED (inverts, visible); cursor on an already-reverse cell removes it
    // (contrasts its reversed neighbours, still visually distinct). Exactly one
    // terminal swap in every case.
    //
    // ratatui's set_style uses insert-semantics for modifiers, so we reset the
    // buffer cell's modifier first to make the cursor's style authoritative —
    // otherwise REVERSED painted by the game-cell loop above would persist when
    // the XOR removes it (cursor on an already-reverse cell).
    if show_cursor && crow >= row_offset && ccol >= col_offset {
        let cur_dy = crow - row_offset;
        let cur_dx = ccol - col_offset;
        if cur_dy < content.height && cur_dx < content.width {
            let grid_row = cur_dy + row_offset + 1; // 1-based
            let grid_col = cur_dx + col_offset + 1; // 1-based
            let cur_cell = upper.cell(grid_row, grid_col);
            let mut cur_zvm = grid_cell_to_zvm(cur_cell);
            cur_zvm.style ^= 0x01; // toggle reverse bit
            let style = cell_style(cur_zvm, cur_cell.glk_style, colors, honor_game_colours, upper.bg, is_glk_grid);
            if let Some(c) = buf.cell_mut((content.x + cur_dx, content.y + cur_dy)) {
                c.modifier = ratatui::style::Modifier::empty(); // clear before re-apply
                c.set_style(style);
            }
        }
    }

    needed
}

/// Draw `grid`'s cells directly into `area` at a 1:1 mapping — no border, no
/// centering, no viewport scroll; `area` IS the grid's absolute cell rect
/// (the v6 layered composite's [`PositionedWindow`](crate::engine::PositionedWindow)
/// already carries the exact placement). Paints ONLY non-blank cells: a cell
/// is "blank" when its glyph is a space with the default (unset) background
/// (`ch == ' ' && bg == 0`); those cells are left UNTOUCHED so an earlier
/// composited layer (e.g. a v6 graphics window) shows through the gaps
/// ("cell-text-wins", Phase 1b).
///
/// `show_cursor` draws the caret the same way [`draw_grid`] does — XOR the reverse
/// bit of the cell the game's cursor sits in. It exists because the hybrid ring
/// reaches this function for a v6 story window that is a Grid (SQ-1074), and that
/// window is one the player is often reading a keypress in: InvisiClues parks a
/// caret after its `6>` prompt. The v6 layered composite passes `false` and is
/// unchanged.
pub fn draw_grid_transparent(
    grid: &GridWindow,
    area: Rect,
    buf: &mut Buffer,
    honor_game_colours: bool,
    colors: &ColorScheme,
    links: &mut Vec<((u16, u16), u32)>,
    show_cursor: bool,
) {
    let rows = area.height.min(grid.rows);
    let cols = area.width.min(grid.cols);
    for dy in 0..rows {
        let grid_row = dy + 1; // 1-based
        for dx in 0..cols {
            let grid_col = dx + 1; // 1-based
            let cell = grid.cell(grid_row, grid_col);
            // **A REVERSED space is ink, not a gap** (SQ-1074). A highlight block is
            // built out of reversed spaces — the run's own §8.7.1 style bit, with no
            // background colour set — so testing only `ch`/`bg` reads the gaps between
            // a selected item's words as blank and leaves the layer beneath showing
            // through them. Shogun's InvisiClues selection is the report: `What must I
            // do to survive?` came out highlighted a word at a time, five separate
            // blocks with the page between them, where the machine draws one bar.
            // Same rule `draw_painted_screen` already states for the run path
            // ("a reversed space fills its cell of the selection bar", SQ-0484).
            if cell.ch == ' ' && cell.bg == 0 && cell.style & 0x01 == 0 {
                continue; // blank: leave the layer beneath showing through
            }
            let bx = area.x + dx;
            let by = area.y + dy;
            if let Some(buf_cell) = buf.cell_mut((bx, by)) {
                let is_glk_grid = grid.win != 0;
                let mut style = cell_style(grid_cell_to_zvm(cell), cell.glk_style, colors, honor_game_colours, grid.bg, is_glk_grid);
                if cell.link != 0 {
                    if honor_game_colours {
                        // SQ-1219: see the identical comment in `draw_grid`.
                        if is_glk_grid {
                            style = concretize_reversed(style);
                        }
                        style = style.patch(colors.theme.get("hyperlink").style);
                    }
                    style = style.add_modifier(ratatui::style::Modifier::UNDERLINED);
                    links.push(((bx, by), cell.link));
                }
                let mut ch_buf = [0u8; 4];
                // Control chars would trip ratatui's cell_width debug assert.
                let ch = if cell.ch.is_control() { ' ' } else { cell.ch };
                buf_cell.set_symbol(ch.encode_utf8(&mut ch_buf)).set_style(style);
            }
        }
    }
    // The caret, on the same XOR rule as `draw_grid` — and reached through the same
    // 1:1 mapping as the cells above, since there is no border, centering or scroll
    // offset here to account for.
    if show_cursor {
        let (crow, ccol) = (grid.cursor.0.saturating_sub(1), grid.cursor.1.saturating_sub(1));
        if crow < rows && ccol < cols {
            let cur = grid.cell(crow + 1, ccol + 1);
            let mut z = grid_cell_to_zvm(cur);
            z.style ^= 0x01;
            let style = cell_style(z, cur.glk_style, colors, honor_game_colours, grid.bg, grid.win != 0);
            if let Some(c) = buf.cell_mut((area.x + ccol, area.y + crow)) {
                c.modifier = ratatui::style::Modifier::empty();
                c.set_style(style);
            }
        }
    }
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Draw the engine's text-grid (upper) window into the top of `area`, returning
/// the number of story-pane rows consumed (0 when the grid is inactive).
///
/// `char_mode` is true when the game is awaiting a keypress; combined with the
/// grid being the engine's currently selected window (`GridWindow::cursor_active`),
/// it decides whether to show the cursor (so in-place forms reveal where typed
/// characters land).
pub fn draw_upper_window(
    grid: &GridWindow,
    char_mode: bool,
    colors: &ColorScheme,
    area: Rect,
    buf: &mut Buffer,
    honor_game_colours: bool,
    links: &mut Vec<((u16, u16), u32)>,
) -> u16 {
    let show_cursor = char_mode && grid.cursor_active;
    draw_grid(
        grid,
        grid.active_rows,
        grid.cursor,
        show_cursor,
        colors,
        area,
        buf,
        honor_game_colours,
        links,
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Modifier};
    use ratatui::{buffer::Buffer, layout::Rect};

    #[test]
    fn upper_cell_colour_resolves_and_reverse_uses_modifier() {
        use zvm::screen::{Cell, ZColour};
        let mut scheme = ColorScheme::default();
        scheme.palette[1] = Color::Rgb(200, 0, 0); // red   (Standard(3) -> palette[1])
        scheme.palette[4] = Color::Rgb(0, 0, 200); // blue  (Standard(6) -> palette[4])
        // no reverse: fg=red, bg=blue (logical order, no REVERSED modifier)
        let s = cell_style(
            Cell { ch: 'x', style: 0, fg: ZColour::Standard(3), bg: ZColour::Standard(6) },
            0,
            &scheme,
            true,
            None,
            false,
        );
        assert_eq!(s.fg, Some(Color::Rgb(200, 0, 0)));
        assert_eq!(s.bg, Some(Color::Rgb(0, 0, 200)));
        assert!(!s.add_modifier.contains(Modifier::REVERSED), "no REVERSED for style=0");
        // reverse (style 0x01): REVERSED modifier set, fg/bg stay in logical order —
        // the terminal performs the single swap via the modifier.
        let r = cell_style(
            Cell { ch: 'x', style: 0x01, fg: ZColour::Standard(3), bg: ZColour::Standard(6) },
            0,
            &scheme,
            true,
            None,
            false,
        );
        assert!(r.add_modifier.contains(Modifier::REVERSED), "REVERSED modifier for style=0x01");
        assert_eq!(r.fg, Some(Color::Rgb(200, 0, 0)), "fg stays logical (not swapped)");
        assert_eq!(r.bg, Some(Color::Rgb(0, 0, 200)), "bg stays logical (not swapped)");
    }

    /// A grid cell's Glk style class selects the theme's per-style colour slot
    /// from the GRID row (row 1) — applying in both gate states, while a Normal
    /// cell (unseeded row 1) inherits the `upper_window` element base (SQ-0331).
    #[test]
    fn cell_style_grid_glk_style_slot_uses_row1() {
        use zvm::screen::{Cell, ZColour};
        let mut scheme = ColorScheme::default();
        scheme.glk_styles[1][4] = Style::default().fg(Color::Green);
        // Subheader (glk_style 4) grid cell, no game colour → slot green, honor OFF.
        let s = cell_style(
            Cell { ch: 'x', style: 0, fg: ZColour::Default, bg: ZColour::Default },
            4, &scheme, false, None, false,
        );
        assert_eq!(s.fg, Some(Color::Green), "grid Subheader → row-1 slot");
        // Normal (glk_style 0) cell → element base (upper_window fg, None by default).
        let n = cell_style(
            Cell { ch: 'x', style: 0, fg: ZColour::Default, bg: ZColour::Default },
            0, &scheme, false, None, false,
        );
        assert_eq!(n.fg, scheme.theme.get("upper_window").style.fg, "grid Normal → upper_window element base");
    }

    /// Alert (glk_style 5) in a grid window renders bold — the registry theme's
    /// canonical Alert modifier, applied unconditionally (SQ-0309 §3).
    #[test]
    fn glk_grid_alert_renders_bold() {
        use zvm::screen::{Cell, ZColour};
        let scheme = ColorScheme::default();
        let s = cell_style(
            Cell { ch: 'x', style: 0, fg: ZColour::Default, bg: ZColour::Default },
            5, &scheme, false, None, false,
        );
        assert!(s.add_modifier.contains(ratatui::style::Modifier::BOLD), "Alert grid cell renders bold");
    }

    /// SQ-1212: a Glk grid cell (`is_glk_grid = true`) with no game colour grounds
    /// on `glk.grid.background`, not `upper_window` — the fix's whole point, that
    /// an unwritten cell and a written Default-colour cell agree on the same
    /// reversed-chrome ground instead of one showing page-on-page.
    #[test]
    fn cell_style_glk_grid_grounds_on_glk_grid_background() {
        use zvm::screen::{Cell, ZColour};
        let scheme = ColorScheme::default();
        let s = cell_style(
            Cell { ch: 'x', style: 0, fg: ZColour::Default, bg: ZColour::Default },
            0, &scheme, true, None, true,
        );
        let ground = scheme.theme.get("glk.grid.background").style;
        assert_eq!(s.fg, ground.fg, "glk grid Normal → glk.grid.background fg");
        assert_eq!(s.bg, ground.bg, "glk grid Normal → glk.grid.background bg");
        assert_eq!(
            s.add_modifier.contains(Modifier::REVERSED),
            ground.add_modifier.contains(Modifier::REVERSED),
            "glk grid Normal inherits the ground's REVERSED bit"
        );
        // And a Z-machine grid cell with the identical colours stays on `upper_window`.
        let z = cell_style(
            Cell { ch: 'x', style: 0, fg: ZColour::Default, bg: ZColour::Default },
            0, &scheme, true, None, false,
        );
        assert_eq!(z.bg, scheme.theme.get("upper_window").style.bg, "Z-machine grid Normal → upper_window bg, untouched");
    }

    /// C1 regression guard: a reverse cell with DEFAULT colours (fg==bg==ZColour::Default)
    /// must carry Modifier::REVERSED even when honor_game_colours is ON. The previous code
    /// used mem::swap(Reset, Reset) = no-op, then masked bit 0x01, making the inversion
    /// invisible for the most common case (status bars that invert without set_colour).
    #[test]
    fn reverse_cell_with_default_colours_carries_reversed() {
        use zvm::screen::{Cell, ZColour};
        let scheme = ColorScheme::default();
        let s = cell_style(
            Cell { ch: ' ', style: 0x01, fg: ZColour::Default, bg: ZColour::Default },
            0,
            &scheme,
            true,
            None,
            false,
        );
        assert!(
            s.add_modifier.contains(Modifier::REVERSED),
            "C1: reverse cell with default colours must carry REVERSED modifier"
        );
    }

    /// **A REVERSED space is ink, not a gap** (SQ-1074). `draw_grid_transparent`
    /// skips blanks so an earlier composited layer shows through, and a highlight
    /// block is built out of spaces carrying the §8.7.1 reverse bit with NO
    /// background colour — so a blank test that reads only `ch`/`bg` punches holes
    /// through a selection bar wherever its words are separated.
    ///
    /// Amiga Shogun's InvisiClues selection is the report: `What must I do to
    /// survive?` highlighted a word at a time, five blocks with the page showing
    /// between them, where `machine-screenshots/amiga-shogun-hint.png` draws one bar.
    /// The reporter noticed the tell that pins the mechanism — it came out correct on
    /// the SECOND visit to the menu, because by then those cells had inherited a real
    /// background from the clue screen's erase and so were no longer "blank".
    #[test]
    fn draw_grid_transparent_paints_a_reversed_space_and_skips_a_plain_one() {
        let mut upper = GridWindow::default();
        upper.resize(1, 3);
        upper.put(1, 1, 'A', 0x01); // reversed glyph
        upper.put(1, 2, ' ', 0x01); // reversed SPACE — the gap inside a highlight bar
        upper.put(1, 3, ' ', 0x00); // a genuinely blank cell

        let colors = make_colors();
        let area = Rect::new(0, 0, 3, 1);
        let mut buf = Buffer::empty(area);
        // A sentinel under the grid: whatever this function declines to paint keeps it.
        for x in 0..3 {
            buf.cell_mut((x, 0)).unwrap().set_symbol("~");
        }
        draw_grid_transparent(&upper, area, &mut buf, true, &colors, &mut Vec::new(), false);

        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "A", "a reversed glyph is painted");
        assert_eq!(
            buf.cell((1, 0)).unwrap().symbol(),
            " ",
            "a REVERSED space must be painted — it is one cell of a highlight bar, and \
             leaving it transparent breaks the bar at every word gap (SQ-1074)"
        );
        assert!(
            buf.cell((1, 0)).unwrap().modifier.contains(Modifier::REVERSED),
            "…and it must carry the reverse the bar is made of"
        );
        assert_eq!(
            buf.cell((2, 0)).unwrap().symbol(),
            "~",
            "a plain blank stays transparent so the layer beneath shows through"
        );
    }

    /// `draw_grid_transparent` places 1:1 from the rect it is given — no centring —
    /// and touches nothing outside the cells the game wrote (SQ-1074).
    ///
    /// Both halves are the Amiga Shogun hint screen. `draw_grid` centres by column
    /// COUNT and floods its region with the theme's `upper_window` page, which for a
    /// v6 window put the topic list thirteen columns right of its own left edge and
    /// left a 62x9 black rectangle over the rows the game had not written since the
    /// clue screen.
    #[test]
    fn draw_grid_transparent_does_not_centre_or_flood() {
        let mut upper = GridWindow::default();
        upper.resize(2, 4);
        upper.put(1, 1, 'Z', 0x00); // one written cell, row 1 column 1

        let colors = make_colors();
        let area = Rect::new(0, 0, 20, 5); // far wider/taller than the 4x2 grid
        let mut buf = Buffer::empty(area);
        for y in 0..5 {
            for x in 0..20 {
                buf.cell_mut((x, y)).unwrap().set_symbol("~");
            }
        }
        draw_grid_transparent(&upper, area, &mut buf, true, &colors, &mut Vec::new(), false);

        assert_eq!(
            buf.cell((0, 0)).unwrap().symbol(),
            "Z",
            "the grid is drawn from the rect's own first column — centring a v6 window, \
             which has an absolute native origin, moves it off its own left edge (SQ-1074)"
        );
        let untouched = (0..5)
            .flat_map(|y| (0..20).map(move |x| (x, y)))
            .filter(|&(x, y)| (x, y) != (0, 0))
            .all(|(x, y)| buf.cell((x, y)).unwrap().symbol() == "~");
        assert!(
            untouched,
            "every cell the game did not write must be left alone — flooding them with the \
             theme's grid page is the black box of SQ-1074"
        );
    }

    /// C1 regression guard (draw_grid level): the REVERSED modifier must reach the
    /// buffer cell for a grid cell that is reverse-video with default colours.
    #[test]
    fn draw_grid_reverse_default_cell_has_reversed_in_buffer() {
        let mut upper = GridWindow::default(); // border Unspecified → theme decides
        upper.resize(1, 3);
        upper.put(1, 2, 'X', 0x01); // reverse, default colors

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        // cols=3 centered in 10 (no border): x_off=(10-3)/2=3; col 2 -> buf x=4.
        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 1, (1, 1), false, &colors, area, &mut buf, true, &mut Vec::new());

        let c = buf.cell((4, 0)).unwrap();
        assert!(
            c.modifier.contains(Modifier::REVERSED),
            "draw_grid: reverse cell with default colours must carry REVERSED in the buffer"
        );
    }

    /// A grid cell carrying a Glk hyperlink must render underlined AND be recorded
    /// in the cell→link map so a click can be hit-tested to it. (SQ-0258)
    #[test]
    fn draw_grid_hyperlinked_cell_underlines_and_maps_to_link() {
        let mut upper = GridWindow::default(); // border Unspecified → theme decides
        upper.resize(1, 3);
        // Put a plain char, then stamp a link on cell (1,2) directly.
        upper.put(1, 2, 'L', 0);
        upper.cells[1].link = 77;

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        // cols=3 centered in 10 (no border): x_off=(10-3)/2=3; col 2 -> buf x=4.
        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        let mut links: Vec<((u16, u16), u32)> = Vec::new();
        draw_grid(&upper, 1, (1, 1), false, &colors, area, &mut buf, true, &mut links);

        assert!(
            buf.cell((4, 0)).unwrap().modifier.contains(Modifier::UNDERLINED),
            "a linked grid cell must render underlined"
        );
        assert_eq!(links, vec![((4, 0), 77)], "the linked cell is recorded at its buffer position");
    }

    /// SQ-0328: a grid carrying its own `bg` (a Glulx per-window background)
    /// fills its content cells with that colour instead of the theme's
    /// `upper_window` bg. A grid with `bg = None` is byte-identical to today.
    #[test]
    fn draw_grid_window_bg_fills_override_colour() {
        let mut upper = GridWindow::default();
        upper.resize(1, 3);
        upper.bg = Some(0x0012_3456); // packed 0x00RRGGBB window background

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        // cols=3 centered in 10 (no border): x_off=(10-3)/2=3; a content cell at x=4,y=0.
        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 1, (1, 1), false, &colors, area, &mut buf, true, &mut Vec::new());

        assert_eq!(
            buf.cell((4, 0)).unwrap().style().bg,
            Some(Color::Rgb(0x12, 0x34, 0x56)),
            "the grid's own window bg fills the content cells"
        );
    }

    /// SQ-0286 (a): a `BorderPref::NoBorder` grid draws NO frame even when the
    /// theme has every border side on — the content sits flush at row 0 and no
    /// border row is consumed.
    #[test]
    fn draw_grid_noborder_suppresses_frame_despite_theme() {
        let mut upper = make_upper_hi(); // 2×5 grid, "HI" at (1,1)
        upper.border = BorderPref::NoBorder;

        // Theme with ALL border sides on — normally this frames the grid.
        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::Single;
        colors.upper_window_border_sides = PaneSides::all(BorderStyle::Single);

        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        let consumed = draw_grid(&upper, 2, (1, 1), false, &colors, area, &mut buf, true, &mut Vec::new());

        // No border overhead: exactly the 2 grid rows, content flush at row 0.
        assert_eq!(consumed, 2, "a NoBorder grid consumes only its content rows");
        // cols=5 centered in 20 with no border cols: x_off = (20-5)/2 = 7.
        assert_eq!(buf.cell((7, 0)).unwrap().symbol(), "H", "content sits at row 0, no top border");
        assert_eq!(buf.cell((8, 0)).unwrap().symbol(), "I");
        // No box-drawing corner anywhere on the top row.
        for x in 0..20 {
            assert_ne!(buf.cell((x, 0)).unwrap().symbol(), "┌", "no frame corner drawn for a NoBorder grid");
        }
    }

    /// SQ-0286 (b): a `BorderPref::Border` grid (an explicit Glulx `winmethod_Border`)
    /// whose theme disabled every side still renders a frame — the fallback
    /// single-line box — so the game's border request is visibly honored.
    #[test]
    fn draw_grid_bordered_forces_frame_when_theme_off() {
        let mut upper = make_upper_hi();
        upper.border = BorderPref::Border; // game explicitly requested a border

        // Theme with EVERY side off.
        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = PaneSides::all(BorderStyle::None);

        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        let consumed = draw_grid(&upper, 2, (1, 1), false, &colors, area, &mut buf, true, &mut Vec::new());

        // Fallback single frame: top+bottom overhead added (2 grid rows + 2).
        assert_eq!(consumed, 4, "the forced fallback frame adds top+bottom border rows");
        // border_cols = 2 → uw_w = 7, x_off = (20-7)/2 = 6; top-left corner at (6,0).
        assert_eq!(buf.cell((6, 0)).unwrap().symbol(), "┌", "fallback single-line corner is drawn");
        // Content is pushed inside the frame: 'H' now at (7,1), not (7,0).
        assert_eq!(buf.cell((7, 1)).unwrap().symbol(), "H", "content sits inside the forced frame");
    }

    fn make_colors() -> ColorScheme {
        ColorScheme::terminal_default()
    }

    /// Build a 2-row × 5-col grid with "HI" starting at (1,1).
    fn make_upper_hi() -> GridWindow {
        let mut w = GridWindow::default();
        w.resize(2, 5);
        w.put(1, 1, 'H', 0);
        w.put(1, 2, 'I', 0);
        w
    }

    #[test]
    fn draws_grid_cells_and_consumes_rows() {
        let upper = make_upper_hi(); // border Unspecified → theme decides
        let colors = make_colors();
        // Area taller than the grid so no scrolling needed.
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);

        // Use BorderStyle::None to avoid border overhead for simplicity.
        let mut colors_no_border = colors.clone();
        colors_no_border.virtual_window_border = BorderStyle::None;
        colors_no_border.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        let consumed = draw_grid(&upper, 2, (1, 1), false, &colors_no_border, area, &mut buf, true, &mut Vec::new());

        // Should consume exactly 2 rows (grid height, no border).
        assert_eq!(consumed, 2, "consumed rows should equal upper_window_rows");

        // cols=5 is centered in the 20-wide pane (no border): x_off = (20-5)/2 = 7.
        assert_eq!(buf.cell((7, 0)).unwrap().symbol(), "H");
        assert_eq!(buf.cell((8, 0)).unwrap().symbol(), "I");
    }

    #[test]
    fn upper_window_centered_at_game_screen_width_not_pane_width() {
        // Regression (bug #79): a game-screen-width upper window must render at its
        // own width centered in a wider pane — not stretched to the pane, which
        // made Bureaucracy's border too wide and its centered content off-place.
        let mut upper = GridWindow::default();
        upper.resize(1, 10); // game screen is 10 cols wide
        upper.put(1, 1, 'A', 0);
        upper.put(1, 10, 'Z', 0); // content spans the full game screen

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        // Pane is 30 wide; the 10-col upper window should center: x_off=(30-10)/2=10.
        let area = Rect::new(0, 0, 30, 5);
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 1, (1, 1), false, &colors, area, &mut buf, true, &mut Vec::new());
        assert_eq!(buf.cell((10, 0)).unwrap().symbol(), "A", "left edge of the game screen at x=10");
        assert_eq!(buf.cell((19, 0)).unwrap().symbol(), "Z", "right edge at x=19 (10..19)");
        // Nothing drawn outside the centered 10-col region.
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), " ", "no content stretched to the pane left edge");
    }

    /// SQ-0403: a grid whose Normal style is reverse-video with no explicit
    /// colours (Counterfeit Monkey's menu sets ReverseColor on every grid style)
    /// must fill its EMPTY cells reversed too, so the whole window reads dark to
    /// match the reverse-video text — not a white fill with dark text islands.
    #[test]
    fn reverse_grid_fills_empty_cells_reversed() {
        let mut upper = GridWindow::default();
        upper.resize(2, 5);
        upper.reverse = true; // game reversed the grid's Normal style
        upper.put(1, 1, 'X', 0); // one written non-reverse cell; the rest are empty fill

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        let area = Rect::new(0, 0, 10, 3);
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 2, (1, 1), false, &colors, area, &mut buf, true, &mut Vec::new());

        // An empty fill cell (row 1, col 5 → cols=5 centered in 10: x_off=2, so x=6) is REVERSED.
        assert!(
            buf.cell((6, 0)).unwrap().modifier.contains(Modifier::REVERSED),
            "a reverse-Normal grid must fill empty cells reversed so the window reads dark"
        );
    }

    /// SQ-1212: a Glk grid window (a real Glk id, `win != 0`) fills its unwritten
    /// GROUND reversed — the status bar's own spelling — so it reads as a
    /// visible chrome band instead of page-on-page. A Z-machine/Scott grid
    /// (`win == 0`, the default) is byte-identical to before: no default reverse,
    /// because those games paint their own reversal and a default one would
    /// double it up. Falsifying this (reverting the `is_glk_grid` selection so
    /// both paths ground on `upper_window`) makes this fail with the
    /// invisible-ground symptom: an unwritten Glk grid cell carries no REVERSED
    /// bit and is indistinguishable from the terminal page.
    #[test]
    fn glk_grid_fills_empty_ground_reversed_but_zmachine_grid_does_not() {
        let mut glk_grid = GridWindow { win: 5, ..GridWindow::default() };
        glk_grid.resize(1, 3);
        // No game colours, no game reverse — an entirely unwritten ground.

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        let area = Rect::new(0, 0, 3, 1);
        let mut glk_buf = Buffer::empty(area);
        draw_grid(&glk_grid, 1, (1, 1), false, &colors, area, &mut glk_buf, true, &mut Vec::new());
        assert!(
            glk_buf.cell((0, 0)).unwrap().modifier.contains(Modifier::REVERSED),
            "an unwritten Glk grid cell must ground on reversed chrome — glk.grid.background"
        );

        // The identical grid, but win == 0 (Z-machine/Scott): untouched, no default reverse.
        let mut z_grid = GridWindow::default();
        z_grid.resize(1, 3);
        let mut z_buf = Buffer::empty(area);
        draw_grid(&z_grid, 1, (1, 1), false, &colors, area, &mut z_buf, true, &mut Vec::new());
        assert!(
            !z_buf.cell((0, 0)).unwrap().modifier.contains(Modifier::REVERSED),
            "a Z-machine upper window must NOT default-reverse — the game paints its own reversal"
        );
    }

    /// A Glk grid carrying its own window `bg` (SQ-0328) must show that colour
    /// behind its UNWRITTEN cells too — not only behind the cells the game wrote.
    /// The ground is reversed chrome (SQ-1212), so laying `.bg(rgb)` on it as-is
    /// puts the window's colour on the side the terminal swaps INTO the
    /// foreground, and the chrome's ink becomes the visible background: black
    /// gaps between light-grey status fields, which is what Kerkerkruip's status
    /// window looked like with its panels off (every field a run of written
    /// cells, every gap an unwritten one).
    #[test]
    fn glk_grid_window_bg_grounds_the_unwritten_cells_in_the_windows_own_colour() {
        use zvm::screen::ZColour;
        let grey = 0x00C0_C0C0;
        let mut grid = GridWindow { win: 5, ..GridWindow::default() };
        grid.resize(1, 3);
        grid.bg = Some(grey);
        // One written cell, coloured the way the game's Normal hints colour it.
        grid.put(0, 0, 'H', 0);
        grid.cells[0].fg = crate::state::pack_zcolour(ZColour::True24(0x00_0000));
        grid.cells[0].bg = crate::state::pack_zcolour(ZColour::True24(grey));

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);
        assert!(
            colors.theme.get("glk.grid.background").style.add_modifier.contains(Modifier::REVERSED),
            "premise: the Glk grid ground is reversed chrome, or this case tests nothing"
        );

        let area = Rect::new(0, 0, 3, 1);
        let mut buf = Buffer::empty(area);
        draw_grid(&grid, 1, (1, 1), false, &colors, area, &mut buf, true, &mut Vec::new());

        // What the terminal will actually paint behind a cell, after its one swap.
        let shown_bg = |x: u16| {
            let st = buf.cell((x, 0)).unwrap().style();
            if st.add_modifier.contains(Modifier::REVERSED) { st.fg } else { st.bg }
        };
        let want = Some(Color::Rgb(0xC0, 0xC0, 0xC0));
        assert_eq!(shown_bg(0), want, "the written cell shows the window's own background");
        assert_eq!(shown_bg(1), want, "so must the unwritten cell beside it");
    }

    /// SQ-1212 precedence: a Glk grid's new reversed ground is the FILL only —
    /// a cell the game DID write with its own colour must still show that
    /// colour, not the ground's reversed chrome. Mirrors SQ-0328's
    /// `draw_grid_window_bg_fills_override_colour` but on the Glk-grid ground.
    #[test]
    fn glk_grid_game_set_cell_colour_still_wins_over_the_new_ground() {
        use zvm::screen::ZColour;
        let mut upper = GridWindow { win: 5, ..GridWindow::default() };
        upper.resize(1, 3);
        upper.put(1, 1, 'X', 0); // written cell, no explicit style bits yet
        // Stamp an explicit game colour directly (mirrors how gvm reports a
        // cell's own fg/bg): pack green.
        let idx = 0usize;
        upper.cells[idx].fg = crate::state::pack_zcolour(ZColour::True24(0x00FF00));

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        let area = Rect::new(0, 0, 3, 1);
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 1, (1, 1), false, &colors, area, &mut buf, true, &mut Vec::new());

        assert_eq!(
            buf.cell((0, 0)).unwrap().style().fg,
            Some(Color::Rgb(0, 0xFF, 0)),
            "a game-set cell colour must still win over the new reversed ground"
        );
    }

    #[test]
    fn returns_zero_when_upper_window_inactive() {
        let upper = make_upper_hi();
        let colors = make_colors();
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        // upper_rows = 0 means inactive.
        let consumed = draw_grid(&upper, 0, (1, 1), false, &colors, area, &mut buf, true, &mut Vec::new());
        assert_eq!(consumed, 0);
    }

    #[test]
    fn border_adds_overhead() {
        let upper = make_upper_hi();
        let mut colors = make_colors();
        // The frame is off by default (SQ-0700), so a test about the frame's cost
        // has to ask for one — and it is the SIDES the renderer measures.
        colors.virtual_window_border = BorderStyle::Single;
        colors.upper_window_border_sides = PaneSides::all(BorderStyle::Single);
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);

        let consumed = draw_grid(&upper, 2, (1, 1), false, &colors, area, &mut buf, true, &mut Vec::new());

        // 2 grid rows + 2 border rows = 4 total.
        assert_eq!(consumed, 4);
        // cols=5 + 2 side borders = 7, centered in 20: x_off=(20-7)/2=6, content.x=7.
        // Grid content starts at row 1 (inside the top border).
        assert_eq!(buf.cell((7, 1)).unwrap().symbol(), "H");
        assert_eq!(buf.cell((8, 1)).unwrap().symbol(), "I");
    }

    #[test]
    fn viewport_scrolls_when_cursor_exceeds_height() {
        let mut upper = GridWindow::default();
        upper.resize(5, 5);
        // Put 'A' at row 5 (last row, 1-based).
        upper.put(5, 1, 'A', 0);

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        // Only 3 rows available, but cursor is at row 5.
        let area = Rect::new(0, 0, 10, 3);
        let mut buf = Buffer::empty(area);

        let consumed = draw_grid(&upper, 5, (5, 1), false, &colors, area, &mut buf, true, &mut Vec::new());
        assert_eq!(consumed, 3);

        // Row offset = cursor_row-1 - (height-1) = 4 - 2 = 2.
        // 'A' is at grid row 5, displayed at terminal row 2 (0-based within content).
        // cols=5 centered in 10 (no border): x_off=(10-5)/2=2, so col 2.
        assert_eq!(buf.cell((2, 2)).unwrap().symbol(), "A");
    }

    /// SQ-0679: a grid WIDER than the pane renders from its first column when
    /// nobody is typing in it — the room name at the left of a status bar must
    /// not be scrolled away by a cursor the game parked out to the right after
    /// painting. The horizontal follow is for a live caret only, so it comes
    /// back the moment the grid is the input target.
    #[test]
    fn wide_grid_anchors_left_unless_the_cursor_is_the_players() {
        let mut upper = GridWindow::default();
        upper.resize(1, 20);
        for (i, ch) in "ROOM NAME".chars().enumerate() {
            upper.put(1, i as u16 + 1, ch, 0);
        }
        upper.put(1, 20, '#', 0); // the far right, where the cursor was left

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        // Pane narrower than the grid; cursor parked past its right edge.
        let area = Rect::new(0, 0, 10, 2);
        let row = |buf: &Buffer| -> String {
            (0..10).map(|x| buf.cell((x, 0)).unwrap().symbol().to_string()).collect()
        };

        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 1, (1, 20), false, &colors, area, &mut buf, true, &mut Vec::new());
        assert_eq!(row(&buf), "ROOM NAME ", "a parked cursor does not scroll the row");

        // The same grid while the game awaits a keypress here: the caret wins.
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 1, (1, 20), true, &colors, area, &mut buf, true, &mut Vec::new());
        assert_eq!(
            row(&buf).chars().last(),
            Some('#'),
            "with a live caret the viewport follows it to the grid's last column"
        );
    }

    #[test]
    fn bold_and_reverse_style_applied() {
        use zvm::screen::ZColour;
        let mut upper = GridWindow::default();
        upper.resize(1, 3);
        // ZMSD §8.7.2 operand values: 1 = reverse-video, 2 = bold
        upper.put(1, 1, 'X', 0x02); // bold
        upper.put(1, 2, 'Y', 0x01); // reverse-video
        // Give Y distinct logical fg/bg so the colour-handling is observable.
        upper.cells[1].fg = crate::state::pack_zcolour(ZColour::Standard(3)); // -> palette[1]
        upper.cells[1].bg = crate::state::pack_zcolour(ZColour::Standard(6)); // -> palette[4]

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);
        colors.palette[1] = Color::Rgb(200, 0, 0);
        colors.palette[4] = Color::Rgb(0, 0, 200);

        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 1, (1, 1), false, &colors, area, &mut buf, true, &mut Vec::new());

        // cols=3 centered in 10 (no border): x_off=(10-3)/2=3.
        let x_cell = buf.cell((3, 0)).unwrap();
        assert!(x_cell.modifier.contains(Modifier::BOLD), "X should be bold");

        // Reverse video uses the REVERSED modifier (not a manual fg/bg swap).
        // The terminal performs exactly one swap via the modifier. fg/bg remain
        // in logical order in the buffer.
        let y_cell = buf.cell((4, 0)).unwrap();
        assert!(
            y_cell.modifier.contains(Modifier::REVERSED),
            "reverse uses the REVERSED modifier, not a manual fg/bg swap"
        );
        assert_eq!(y_cell.fg, Color::Rgb(200, 0, 0), "Y fg stays logical (not swapped)");
        assert_eq!(y_cell.bg, Color::Rgb(0, 0, 200), "Y bg stays logical (not swapped)");
    }

    /// Cursor on a normal (non-reverse) cell: XOR toggles bit 0x01 ON, producing
    /// a REVERSED modifier with logical fg/bg order preserved for the terminal to swap.
    #[test]
    fn cursor_on_nonreverse_cell_adds_reversed_modifier() {
        use zvm::screen::ZColour;
        let mut upper = GridWindow::default();
        upper.resize(2, 5);
        // Give the cell under the cursor distinct game colours so the logical
        // ordering (fg/bg not swapped in buffer) can be verified.
        upper.put(2, 3, 'C', 0); // style=0 (normal, non-reverse)
        let idx = 5 + (3 - 1);
        upper.cells[idx].fg = crate::state::pack_zcolour(ZColour::Standard(3)); // -> palette[1]
        upper.cells[idx].bg = crate::state::pack_zcolour(ZColour::Standard(6)); // -> palette[4]

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);
        colors.palette[1] = Color::Rgb(200, 0, 0);
        colors.palette[4] = Color::Rgb(0, 0, 200);
        let area = Rect::new(0, 0, 10, 3);

        // cols=5 centered in 10 (no border): x_off=2; cursor (row 2, col 3) →
        // content (1,2) → buffer (4,1).
        // With show_cursor=false the cursor cell shows its logical fg/bg order.
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 2, (2, 3), false, &colors, area, &mut buf, true, &mut Vec::new());
        let c = buf.cell((4, 1)).unwrap();
        assert_eq!(c.fg, Color::Rgb(200, 0, 0), "no cursor: fg is logical");
        assert_eq!(c.bg, Color::Rgb(0, 0, 200), "no cursor: bg is logical");

        // With show_cursor=true: XOR 0^1=1 → REVERSED modifier, fg/bg remain logical.
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 2, (2, 3), true, &colors, area, &mut buf, true, &mut Vec::new());
        let c = buf.cell((4, 1)).unwrap();
        assert!(c.modifier.contains(Modifier::REVERSED), "cursor on normal cell adds REVERSED modifier");
        assert_eq!(c.fg, Color::Rgb(200, 0, 0), "cursor fg stays logical (terminal swaps via REVERSED)");
        assert_eq!(c.bg, Color::Rgb(0, 0, 200), "cursor bg stays logical (terminal swaps via REVERSED)");
    }

    /// Cursor on an already-reverse cell: XOR toggles bit 0x01 OFF, removing the
    /// REVERSED modifier so the cursor cell appears normal while its reversed neighbours
    /// remain inverted — the cursor is still visually distinct.
    #[test]
    fn cursor_on_reverse_cell_toggles_reverse_off() {
        let mut upper = GridWindow::default();
        upper.resize(1, 3);
        upper.put(1, 2, 'R', 0x01); // style=reverse (0x01)

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        // cols=3 centered in 10 (no border): x_off=(10-3)/2=3; col 2 -> buf x=4.
        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 1, (1, 2), true, &colors, area, &mut buf, true, &mut Vec::new());

        let c = buf.cell((4, 0)).unwrap();
        assert!(
            !c.modifier.contains(Modifier::REVERSED),
            "cursor on an already-reverse cell must XOR-toggle reverse OFF"
        );
    }

    /// Cursor on a DEFAULT cell (fg == bg == ZColour::Default) must be visible.
    /// XOR toggles bit 0x01 ON (style 0→1), producing REVERSED so the terminal
    /// inverts whatever colours the cell inherits — the cursor is always visible.
    #[test]
    fn cursor_on_default_cell_carries_reversed_modifier() {
        let mut upper = GridWindow::default();
        upper.resize(1, 3);
        // Cell (1,2) keeps its default colours (ZColour::Default -> Color::Reset).
        upper.put(1, 2, ' ', 0);

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        // cols=3 centered in 10 (no border): x_off=(10-3)/2=3; col 2 -> buf x=4.
        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 1, (1, 2), true, &colors, area, &mut buf, true, &mut Vec::new());

        let c = buf.cell((4, 0)).unwrap();
        assert!(
            c.modifier.contains(Modifier::REVERSED),
            "cursor on a default (Reset/Reset) cell must carry REVERSED so it stays visible"
        );
    }
}
