// Screen model — ZMSD §7, §8, §11.
//
// `ScreenState` tracks window layout and text attributes the host needs to
// render.  `StatusLine` is the v3 status bar computed on demand from globals.
// `StreamState` manages output-stream routing including stream-3 memory
// redirection.
//
// Stream-3 can nest up to 16 deep (ZMSD §7.1.2.5).  Each frame holds a
// table base address; the first word of the table is the byte-count written.

use crate::memory::Memory;
use crate::objects;

// ---------------------------------------------------------------------------
// Status line
// ---------------------------------------------------------------------------

/// The right-hand portion of a v3 status line (ZMSD §8.2.3.1).
/// Flags1 bit 1: 0 = score/turns, 1 = time (hours:minutes).
#[derive(Debug, PartialEq)]
#[non_exhaustive]
pub enum StatusRight {
    ScoreTurns { score: i16, turns: u16 },
    Time { hours: u8, minutes: u8 },
}

/// A fully computed v3 status line (location name + right field).
#[derive(Debug, PartialEq)]
pub struct StatusLine {
    pub location: String,
    pub right: StatusRight,
}

// ---------------------------------------------------------------------------
// Screen state (window model)
// ---------------------------------------------------------------------------

/// A Z-machine colour channel value (logical, pre-reverse-swap).
///
/// Transient display state — NOT serialised into Quetzal saves (like
/// `current_font`). The host resolves `Default` to the terminal/scheme
/// default, `Standard(2..=9)` to the scheme palette, `Standard(10..=12)` to
/// fixed grey RGB, `True` to an exact 15-bit RGB colour (Z-machine
/// `set_true_colour`), and `True24` to an exact 24-bit `0xRRGGBB` colour (used
/// by the Glulx host, whose Glk stylehint colours are 24-bit — carried at full
/// fidelity rather than downsampled to 15-bit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(Default)]
#[non_exhaustive]
pub enum ZColour {
    #[default]
    Default,
    Standard(u8),
    True(u16),
    True24(u32),
}

impl ZColour {
    /// This channel's colour as a 15-bit true-colour value (ZMSD §8.3.7), given
    /// the interpreter's own default colour number for the channel.
    ///
    /// Standard numbers map through the §8.3.1 table; `Default` resolves to the
    /// interpreter default the header publishes in $2C/$2D (which is what the
    /// player actually sees); `True` is already a 15-bit value; `True24` is a
    /// 24-bit host colour rounded down to 15 bits — §8.8.3.2.8 anticipates
    /// exactly that ("the value shown may be a 15-bit rounding of a more precise
    /// colour").
    ///
    /// There is no `-4` (transparent) answer here because the model has no
    /// transparent state: §8.3.6 lets an interpreter without transparency
    /// "ignore any attempt to select colour 15", and this one does.
    ///
    /// `palette` is the machine's own table ([`crate::cpu::Machine::palette`]).
    /// It is a parameter and not a global because resolving a colour number
    /// without saying WHICH machine's table you mean is the bug (SQ-1393).
    pub fn true_value(self, palette: Palette, interpreter_default: u8) -> u16 {
        match self {
            ZColour::Default => true_colour_in(palette, interpreter_default).unwrap_or(0),
            ZColour::Standard(n) => true_colour_in(palette, n).unwrap_or(0),
            ZColour::True(v) => v & 0x7FFF,
            ZColour::True24(rgb) => {
                let (r, g, b) = ((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
                (((b >> 3) << 10) | ((g >> 3) << 5) | (r >> 3)) as u16
            }
        }
    }
}

/// Expand a 15-bit RGB (0bbbbbgggggrrrrr) to 8-bit `(r, g, b)`. Shared by the
/// CLI (SGR) and app (ratatui) renderers so the expansion is defined once.
pub fn rgb15_to_888(v: u16) -> (u8, u8, u8) {
    let exp = |c: u16| -> u8 { ((c << 3) | (c >> 2)) as u8 };
    (exp(v & 0x1F), exp((v >> 5) & 0x1F), exp((v >> 10) & 0x1F))
}

/// RGB for the v6 greys (Standard 10/11/12). Defined once here so both
/// renderers agree. Any other value falls back to dark grey (12).
///
/// ZMSD §8.3.1 gives the true-colour values for these three entries —
/// 10 = light grey ($5AD6), 11 = medium grey ($4631), 12 = dark grey ($2D6B) —
/// so they are just [`rgb15_to_888`] of the spec table, not an invented ramp.
/// Under [`Palette::Amiga`] they come from Infocom's Amiga table instead
/// ([`amiga_true_colour`]), which is the one place the greys genuinely differ —
/// which is why `palette` is a parameter here (SQ-1393).
pub fn grey_rgb(palette: Palette, n: u8) -> (u8, u8, u8) {
    // Anything outside the three grey numbers reads as dark grey, as it always
    // has — the callers guard on 10..=12, so this is belt and braces.
    let n = if matches!(n, 10 | 11) { n } else { 12 };
    rgb15_to_888(true_colour_in(palette, n).unwrap_or(0x2D6B))
}

/// One character cell in the upper window.
#[derive(Debug, Clone, Copy)]
pub struct Cell {
    pub ch: char,
    pub style: u8,
    pub fg: ZColour,
    pub bg: ZColour,
}
impl Default for Cell {
    fn default() -> Self {
        Cell { ch: ' ', style: 0, fg: ZColour::Default, bg: ZColour::Default }
    }
}

/// Upper (status) window character grid.
#[derive(Debug, Default, Clone)]
pub struct UpperWindow {
    pub cols: u16,
    pub rows: u16,
    pub cells: Vec<Cell>,
}
impl UpperWindow {
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.rows = rows;
        self.cols = cols;
        self.cells = vec![Cell::default(); rows as usize * cols as usize];
    }
    /// Blank every cell, keeping the grid's size.
    ///
    /// For a restore that brings game memory WITHOUT a screen to go with it
    /// (Quetzal archives none by design). The story repaints its own status line
    /// on the next turn; until it does, an empty grid is the only honest thing to
    /// show, because what is there belongs to a different moment. Size is kept
    /// because the restored game's field columns were baked at that width — see
    /// `resize_preserving`'s note and SQ-0681.
    pub fn blank(&mut self) {
        self.cells.iter_mut().for_each(|c| *c = Cell::default());
    }
    /// Resize the grid **preserving** whatever cells survive the new extent
    /// (growing adds blank rows/cols, shrinking truncates).
    ///
    /// ZMSD §15 `split_window`: "In Version 3 (only) the upper window should be
    /// cleared after the split" — so from Version 4 on a re-split must leave the
    /// existing upper-window contents on screen. [`resize`] (which reallocates
    /// blank) is the Version 3 behaviour.
    pub fn resize_preserving(&mut self, rows: u16, cols: u16) {
        if cols == self.cols {
            // Row-major with an unchanged stride: truncate/extend in place.
            self.cells.resize(rows as usize * cols as usize, Cell::default());
            self.rows = rows;
            return;
        }
        let mut next = vec![Cell::default(); rows as usize * cols as usize];
        for r in 0..rows.min(self.rows) as usize {
            for c in 0..cols.min(self.cols) as usize {
                next[r * cols as usize + c] = self.cells[r * self.cols as usize + c];
            }
        }
        self.cells = next;
        self.rows = rows;
        self.cols = cols;
    }
    /// [`resize_preserving`](Self::resize_preserving), but a WIDENING continues
    /// each row's trailing appearance (style + colours, blanked to a space) into
    /// the columns that appear — instead of leaving them at the interpreter
    /// default. (SQ-0679)
    ///
    /// This is for a width change the HOST forces on the game (the screen grew
    /// under it, `refit_upper_window_width`), never for one the game asked for.
    /// A v4/v5 status line is painted once — a run of reverse-video spaces the
    /// game fills at whatever width byte $21 held when it laid out — and the
    /// fields are then updated in place, so nothing ever repaints the columns a
    /// later widen adds. Defaulting them punched an unstyled hole in the game's
    /// band from its old right edge to the new one: the reverse-video bar
    /// stopped short of its own box. Continuing the row's own trailing cell is
    /// the only extension that cannot introduce an appearance the game did not
    /// already have on that row.
    ///
    /// Not an erase, so ZMSD §8.7.3.4 ("Even if the text style is Reverse Video
    /// the new blank space should not have reversed colours") does not apply —
    /// that rule governs `erase_window`/`erase_line`, where the GAME asked for
    /// blank space. Here the game asked for nothing at all.
    pub fn resize_continuing_row_style(&mut self, rows: u16, cols: u16) {
        let old_cols = self.cols;
        // The appearance each surviving row ends in, captured before the move.
        let tail: Vec<Cell> = if cols > old_cols && old_cols > 0 {
            (0..rows.min(self.rows))
                .map(|r| Cell { ch: ' ', ..self.cell(r + 1, old_cols) })
                .collect()
        } else {
            Vec::new()
        };
        self.resize_preserving(rows, cols);
        for (r, t) in tail.iter().enumerate() {
            for c in old_cols..cols {
                self.cells[r * cols as usize + c as usize] = *t;
            }
        }
    }
    pub fn clear(&mut self) {
        self.clear_to(ZColour::Default);
    }
    /// Blank every cell to `bg`.
    ///
    /// ZMSD §8.7.3.2: a window is erased "to background colour", and §8.7.3.4
    /// adds "Even if the text style is Reverse Video the new blank space should
    /// not have reversed colours" — hence style 0 (no reverse bit) on the blank.
    pub fn clear_to(&mut self, bg: ZColour) {
        self.cells.fill(Cell { ch: ' ', style: 0, fg: ZColour::Default, bg });
    }
    /// The 1-based index of the deepest row that has anything on it — 0 when the
    /// grid is entirely blank. (SQ-1355)
    ///
    /// This is how far down the screen the game's own printing actually reaches,
    /// which is a different question from how many rows the grid has been
    /// ALLOCATED: rows below the split exist only to hold what was painted there
    /// (`grow_rows`, and `split_window`'s shrink), and once nothing is painted in
    /// them they describe no pixel a real interpreter would still be showing.
    ///
    /// A cell counts as painted if it carries a glyph or a style bit. Colour is
    /// deliberately not consulted: [`clear_to`](Self::clear_to) blanks the window
    /// TO the background colour (ZMSD §8.7.3.2, "the specified window can be
    /// cleared to background colour"), so a cell's background says what the ERASE
    /// left, not what the game drew — counting it would make a freshly erased
    /// window read as fully painted. The style bit is consulted because that is
    /// what a reverse-video bar is made of, and such a bar IS paint.
    pub fn last_painted_row(&self) -> u16 {
        let cols = self.cols as usize;
        if cols == 0 {
            return 0;
        }
        for r in (0..self.rows as usize).rev() {
            let row = &self.cells[r * cols..(r + 1) * cols];
            if row.iter().any(|c| c.ch != ' ' || c.style != 0) {
                return r as u16 + 1;
            }
        }
        0
    }
    /// Grow the grid to at least `new_rows` rows, preserving existing content.
    /// No-op when the grid is already tall enough. Used when a game draws in the
    /// upper window at rows beyond the current split height (Frotz keeps such
    /// writes on screen instead of clipping them to the split).
    pub fn grow_rows(&mut self, new_rows: u16) {
        if new_rows <= self.rows {
            return;
        }
        // Cells are row-major; appending blank cells adds new rows at the bottom
        // without disturbing existing rows.
        self.cells
            .resize(new_rows as usize * self.cols as usize, Cell::default());
        self.rows = new_rows;
    }
    /// Widen the grid, keeping every cell already in it (SQ-1072).
    ///
    /// The column counterpart of [`grow_rows`](Self::grow_rows), and it exists
    /// for the same reason: a **proportional** pen fits more characters on a line
    /// than the declared cell counts, so the row a v6 window prints can be wider
    /// than `x_size / cell.w`. Bounding the LINE by that count instead — which is
    /// what the print path used to do — ends it before the pixels do and draws a
    /// screen the machine never drew.
    ///
    /// Row-major, so a widen cannot append in place the way `grow_rows` can; it
    /// re-lays through [`resize_preserving`](Self::resize_preserving).
    pub fn grow_cols(&mut self, new_cols: u16) {
        if new_cols > self.cols {
            self.resize_preserving(self.rows, new_cols);
        }
    }
    /// Scroll the grid vertically by whole rows (used by `scroll_window`,
    /// EXT:0x14, quantized to the character grid): positive shifts content
    /// forward/up (drops the top `rows`, appends blank rows at the bottom);
    /// negative shifts backward/down (drops the bottom `rows`, inserts blank
    /// rows at the top). `rows` at or beyond the grid's extent clears it.
    pub fn scroll_rows(&mut self, rows: i16) {
        if rows == 0 || self.rows == 0 {
            return;
        }
        let total = self.rows as usize;
        let cols = self.cols as usize;
        let n = (rows.unsigned_abs() as usize).min(total);
        if n == total {
            self.clear();
            return;
        }
        if rows > 0 {
            self.cells.drain(0..n * cols);
            self.cells.resize(total * cols, Cell::default());
        } else {
            self.cells.truncate((total - n) * cols);
            self.cells.splice(0..0, vec![Cell::default(); n * cols]);
        }
    }
    fn idx(&self, row: u16, col: u16) -> Option<usize> {
        if row == 0 || col == 0 || row > self.rows || col > self.cols {
            return None;
        }
        Some(((row - 1) as usize) * self.cols as usize + (col - 1) as usize)
    }
    pub fn cell(&self, row: u16, col: u16) -> Cell {
        self.idx(row, col)
            .and_then(|i| self.cells.get(i).copied())
            .unwrap_or_default()
    }
    pub fn put(&mut self, row: u16, col: u16, ch: char, style: u8, fg: ZColour, bg: ZColour) {
        if let Some(i) = self.idx(row, col) {
            if let Some(c) = self.cells.get_mut(i) {
                *c = Cell { ch, style, fg, bg };
            }
        }
    }
}

/// One v6 window; its fields ARE the ZMSD window-property array (index =
/// property number, ZMSD 1.1 §8.8.3.2).
#[derive(Debug, Clone, Default)]
pub struct ZWindow {
    pub y_coord: u16,          // prop 0  (pixels)
    pub x_coord: u16,          // prop 1
    pub y_size: u16,           // prop 2  (height, pixels)
    pub x_size: u16,           // prop 3  (width, pixels)
    /// Cursor in UNITS (pixels), 1-based within the window (ZMSD §8.8.3.2 —
    /// window props are measured in units, so `get_wind_prop` 4/5 read these
    /// verbatim). The char-cell the grid writes at derives as `(px-1)/font + 1`.
    pub y_cursor: u16,         // prop 4  (pixels)
    pub x_cursor: u16,         // prop 5  (pixels)
    pub left_margin: u16,      // prop 6
    pub right_margin: u16,     // prop 7
    pub interrupt_routine: u16,// prop 8
    pub interrupt_countdown: u16, // prop 9
    pub text_style: u16,       // prop 10
    pub colour_data: u16,      // prop 11 (high byte bg, low byte fg — ZMSD)
    pub font_number: u16,      // prop 12
    pub font_size: u16,        // prop 13 (high byte height, low byte width)
    pub attributes: u16,       // prop 14 (bit0 wrap, bit1 scroll, bit2 copy-to-transcript, bit3 buffered)
    pub line_count: u16,       // prop 15
    /// Character grid for this window (grid windows 1–7). Window 0 scrolls (buffered),
    /// its text goes to the transcript stream, not a grid.
    pub grid: UpperWindow,
    pub fg: ZColour,
    pub bg: ZColour,
    /// Pixel-positioned text runs (grid windows 1–7): each print records the
    /// exact 1-based pixel position it painted at, so a pixel-faithful raster
    /// can draw text where the game put it (e.g. Zork Zero's status text at
    /// rows 6/14, ON the banner ribbons) instead of snapping to the char grid.
    /// The char grid above remains the cell-mode fallback.
    pub texts: Vec<V6Text>,
    /// Flowing PROSE currently displayed in this window, as logical lines
    /// (SQ-0585). Only a wrap+scroll window that is not the one the game reads
    /// input through fills this: a v6 game may run several scrolling text windows
    /// at once — advent.z6's `style` opens one across the top and keeps playing in
    /// another below — and their streams must not be spliced into one transcript.
    ///
    /// This is LIVE SCREEN STATE, not history: no scrollback, bounded to
    /// [`PROSE_MAX_LINES`], and cleared by `erase_window` exactly as `texts` is.
    /// The window the game reads input through streams to the host transcript as
    /// before and leaves this empty.
    pub prose: Vec<String>,
    /// Where on the screen the prose this window has streamed to the host
    /// transcript is currently sitting (SQ-0697), as runs in the same
    /// screen-absolute space as [`ZWindow::texts`].
    ///
    /// ZMSD §15 is explicit that `move_window`/`window_size` "do not change the
    /// current display": text already printed stays as pixels where it was
    /// drawn. A scrolling window is no exception — Shogun prints its whole title
    /// header while window 0 is the full 640x400 screen, then moves window 0 down
    /// to a 548x64 box beside its menu and prints "You may choose to:" there. On a
    /// real interpreter the header stays painted up top; we stream both halves
    /// into one transcript, so they end up adjacent.
    ///
    /// So the stream is shadowed here as it goes out, and
    /// [`ZWindow::retire_streamed`] hands it over to `texts` — real paint — the
    /// moment the window's box changes. This is the ONLY use: nothing renders
    /// from this buffer directly, and while the window stays put the host
    /// transcript remains the single source of the prose.
    ///
    /// Bounded by the window itself: [`ZWindow::prose_new_line`] scrolls the runs
    /// up with the text and drops whatever leaves the top, so a window that never
    /// moves holds at most a screenful. `erase_window` empties it, exactly as it
    /// empties `texts` and `prose` — the pixels are gone, so their record is too.
    pub streamed: Vec<V6Text>,
    /// Prose this window streamed that has since been FROZEN in place (SQ-0697):
    /// the window was moved or resized, and ZMSD §15 keeps what was already
    /// printed exactly where it was drawn.
    ///
    /// Kept apart from [`ZWindow::texts`] rather than folded into it, even though
    /// both are paint in the same absolute space: `texts` is the window's live
    /// painted layer and several renderers reason about it as such (its rows are
    /// the window's own status/menu bars), while this is a layer the window has
    /// LEFT BEHIND, at coordinates it no longer covers. An erase trims and clears
    /// the two together — the pixels are shared, so their fates are.
    pub retired: Vec<V6Text>,
    /// Screen-absolute `(y, x)` in pixels where the FIRST glyph streamed since
    /// this was last cleared landed (SQ-0804). Set by
    /// [`ZWindow::record_streamed`] and cleared by
    /// [`ZWindow::clear_stream_origin`]; nothing in the VM reads it.
    ///
    /// It answers one question the printed TEXT cannot: did this burst of output
    /// continue the line the last one left the pen on, or did the game reposition
    /// first? A `read_char` echoes nothing (ZMSD §10.7), so a keypress turn
    /// inherits no newline and the host must decide for itself whether to open a
    /// transcript line for the turn's output — and games that redraw a menu
    /// `set_cursor` back to the top with no newline in sight, so the text alone is
    /// not decidable. Compare this against the cursor the window HAD before the
    /// turn and the answer is exact.
    ///
    /// Live per-burst state, not history: transient, not archived, and
    /// meaningless outside the window between a clear and the read that follows.
    pub stream_origin: Option<(u16, u16)>,
    /// Where the character GRID's pen sits, and the pixel cursor it belongs to
    /// (SQ-1009). See [`GridPen`].
    pub grid_pen: Option<GridPen>,
}

/// The character-grid pen, remembered against the pixel cursor it was reached
/// from (SQ-1009).
///
/// # Why the grid needs its own pen at all
///
/// A v6 window carries two representations of the same text — [`ZWindow::grid`]
/// for the hybrid backend's terminal cells, [`ZWindow::texts`] for the raster's
/// pixel-positioned runs — and until SQ-1009 the column was simply DERIVED,
/// `(x_cursor - 1) / cell.w + 1`. That is exact while the pen advances by one
/// declared cell per character and false the moment it does not: at Arthur's
/// ~10.4 native pixels against a declared 8, the derived column steps 1.3 per
/// character and the grid grows holes.
///
/// So the column advances by ONE per character while the pixel cursor advances
/// by the face's advance, and this records the pair they were last in step at.
///
/// # Why it is remembered against the cursor rather than simply stored
///
/// Every other route to the cursor — `set_cursor`, `put_prop` 4/5, a resize
/// re-homing it, `erase_window` — moves the pixel cursor and knows nothing about
/// a grid pen. Stored plainly, the column would go stale at each of them, and a
/// stale column is exactly the silent, self-consistent defect this codebase keeps
/// meeting. Recording the pixel cursor alongside makes going stale detectable:
/// [`ZWindow::grid_cursor`] falls back to the derivation whenever the pixel
/// cursor has moved by any other hand, so no site outside the print loop has to
/// know this exists.
///
/// Not archived, for the same reason: it is a memo about a derivation, and a
/// restore re-derives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridPen {
    pub y_cursor: u16,
    pub x_cursor: u16,
    pub row: u16,
    pub col: u16,
}

/// One pixel-positioned text run in a v6 grid window: `(y, x)` are the 1-based
/// **screen-absolute** pixel coords of the run's first glyph's top-left,
/// captured at paint time. v6 text is PAINT — once drawn, pixels stay where
/// they were put regardless of later `move_window`/`window_size` calls
/// ("window_size does not change the current display", ZMSD §15; Shogun
/// shrinks its menu window to a 1-px caret AFTER printing the menu items).
/// A run is only removed or trimmed by later paint over the same pixels
/// ([`V6Windows::paint_run`]) or an erase ([`V6Windows::erase_screen_rect`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V6Text {
    pub y: u16,
    pub x: u16,
    pub text: String,
    pub style: u8,
    pub fg: ZColour,
    pub bg: ZColour,
    /// The SCREEN character cell this run's first glyph was written at — 0-based
    /// row and column in the same space [`V6Cell::row_of`] and [`V6Cell::col_of`]
    /// answer in (SQ-1009).
    ///
    /// # Why a run carries its cell as well as its pixel
    ///
    /// Because on a proportional machine the two are no longer the same fact, and
    /// a cell backend cannot recover one from the other. `(x - 1) / cell.w` is the
    /// column only while the pen advances by exactly one declared cell per
    /// character; at Arthur's ~10.4 native pixels against a declared 8 that
    /// quotient climbs 1.3 per glyph, so a renderer deriving columns skips them
    /// and the drift compounds along the line — `Churchyard` reads `Ch urc  hy ard`,
    /// and the wider the pane the worse it gets.
    ///
    /// The engine already maintains the dense grid ([`ZWindow::grid`], see
    /// [`GridPen`]), so the answer exists and only needed carrying. A run never
    /// spans more than one grid row: the print loop breaks a run at EITHER
    /// measure's line break, so this cell plus the run's own characters address
    /// every glyph in it.
    ///
    /// For a fixed pen these are exactly `row_of(y)` and `col_of(x)`, so every
    /// machine but Arthur's Amiga press is unchanged.
    pub grow: u16,
    pub gcol: u16,
}

impl V6Text {
    /// A painted run whose grid cell is the DERIVATION `(row_of(y), col_of(x))`.
    ///
    /// Correct for every fixed-pen machine — the pen and the cell agree there —
    /// and the honest answer wherever no grid stands behind the run at all: the
    /// prose shadow, and test fixtures that place paint by hand. The print path
    /// does NOT use it: on a proportional machine the grid pen is a separate fact
    /// and it carries that instead.
    pub fn derived(y: u16, x: u16, text: String, style: u8, fg: ZColour, bg: ZColour, cell: V6Cell) -> V6Text {
        V6Text { y, x, text, style, fg, bg, grow: cell.row_of(y), gcol: cell.col_of(x) }
    }

    /// Pixel width of this run as the machine DREW it (SQ-0917, SQ-1009).
    ///
    /// The run's own style byte is part of the measurement: a bold run on a
    /// machine that emboldens by smearing is genuinely wider than the same
    /// letters in roman, and this width is what decides whether a later paint
    /// covers it.
    fn px_w(&self, metric: &V6Metric) -> u32 {
        metric.run_px(&self.text, self.style)
    }
}

/// ZMSD §8.8.3.2.6: "A line count of -999 means 'never print [MORE]'."
/// Also the floor §8.8.3.2.2.3 clamps to ("A line count is never decremented
/// below -999"), so once a window reaches the sentinel it stays there.
pub const NEVER_MORE: i16 = -999;

impl ZWindow {
    /// ZMSD §8.8.3.1 attribute 0 ("wrapping").
    pub fn wrapping(&self) -> bool {
        self.attributes & 0b0001 != 0
    }
    /// A flowing-PROSE window: wrapping AND scrolling, the pair that sends a v6
    /// window's output down the stream-1 text path rather than painting it at
    /// pixel coordinates (see `cpu::exec`'s print routing).
    pub fn prose_window(&self) -> bool {
        self.attributes & 0b0011 == 0b0011
    }
    /// ZMSD §8.8.3.1 attribute 2: "text copied to output stream 2 (the transcript,
    /// if selected)". A game that runs more than one prose window marks the one
    /// carrying the narrative with it — advent.z6 sets it on the window the player
    /// types into and clears it on the display window it opens above (SQ-0585).
    pub fn copy_to_transcript(&self) -> bool {
        self.attributes & 0b0100 != 0
    }
    /// Append streamed prose to this window's live line buffer (SQ-0585), starting
    /// a new logical line at each `\n` and dropping the oldest lines past
    /// [`PROSE_MAX_LINES`]. Wrapping is the host's job — it knows the font and the
    /// pane — so lines are stored logically, exactly as the host transcript keeps
    /// them.
    ///
    /// `at_col` is a COLUMN this window's own `set_cursor` declared for `s`
    /// (SQ-0729), in characters from the window's left margin — `None` when the
    /// text simply flowed on from the last print. A v6 prose window is still a
    /// pixel surface, and a game may place runs across it: fmvpoker prints its five
    /// menu labels into one at x = 1, 178, 372, 454 and 557, and dropping those
    /// columns butted the runs against each other as
    /// `PLAY CURRENT BETCHANGE CURRENT BETSAVERESTOREQUIT`.
    ///
    /// The line is PADDED OUT TO the column, not indented BY it: the run has to
    /// land at the column the game named, not that far past wherever the previous
    /// run happened to end. A column already behind the line's end cannot be
    /// reached by appending, so it is ignored — a line buffer can only move right,
    /// and only the streaming shadow ([`ZWindow::record_streamed`]) can express a
    /// true backwards jump.
    ///
    /// `at_row` is that same `set_cursor`'s ROW, in text lines from the window's
    /// top, and is honoured the same way: the buffer is PADDED OUT TO it with blank
    /// lines, and a row already behind the buffer's end is ignored. fmvpoker needs
    /// both halves, and needed the row half the moment its story window became a
    /// canvas: it prints its menu bar and its CONTINUE button at `set_cursor(row=80,
    /// …)`, five text lines down its 156px bottom panel, and prints its running
    /// totals into WINDOW 0 at absolute y = 247 and 265 — which land in that panel's
    /// first two lines. Stacking the panel's own prose from its top edge put the two
    /// on the same rows, so the game's own layout collided with itself.
    pub fn push_prose(&mut self, s: &str, at_col: Option<usize>, at_row: Option<usize>) {
        // `row` is a 0-based line index, so the buffer needs `row + 1` lines for
        // the run to land ON it. A buffer already that long has moved past the
        // declaration and keeps its own last line.
        if let Some(row) = at_row {
            while self.prose.len() <= row {
                self.prose.push(String::new());
            }
        }
        for (i, part) in s.split('\n').enumerate() {
            if i > 0 || self.prose.is_empty() {
                self.prose.push(String::new());
            }
            if let Some(last) = self.prose.last_mut() {
                // Only the first fragment sits at the declared column; everything
                // after a '\n' starts a line of its own at the left margin.
                if i == 0 {
                    if let Some(col) = at_col {
                        let have = last.chars().count();
                        last.extend(std::iter::repeat_n(' ', col.saturating_sub(have)));
                    }
                }
                last.push_str(part);
            }
        }
        if self.prose.len() > PROSE_MAX_LINES {
            let drop = self.prose.len() - PROSE_MAX_LINES;
            self.prose.drain(..drop);
        }
    }
    /// ZMSD §8.8.3.1 attribute 1 ("scrolling").
    pub fn scrolling(&self) -> bool {
        self.attributes & 0b0010 != 0
    }
    /// ZMSD §8.8.3.1 attribute 2 ("text copied to output stream 2 (the
    /// transcript, if selected)").
    pub fn scripting(&self) -> bool {
        self.attributes & 0b0100 != 0
    }
    /// ZMSD §8.8.3.1 attribute 3 ("buffered printing").
    pub fn buffered(&self) -> bool {
        self.attributes & 0b1000 != 0
    }

    /// Property 15 read as the signed number the spec talks about (window
    /// properties are "standard Z-machine numbers", i.e. signed 16-bit).
    pub fn line_count_signed(&self) -> i16 {
        self.line_count as i16
    }

    /// How many lines this window prints before "[MORE]" falls due — its
    /// height in text lines less one, matching frotz's `screen_new_line`
    /// threshold (`above + below - 1`). Degenerate (zero-height) windows
    /// report 1 rather than 0 so the count never starts already-due.
    pub fn more_interval(&self, cell: V6Cell) -> i16 {
        let lines = (self.y_size / cell.h()) as i32;
        (lines - 1).clamp(1, i16::MAX as i32) as i16
    }

    /// One new-line happened in this window: ZMSD §8.8.3.2.2 "the line count
    /// is decremented on each new-line", §8.8.3.2.2.3 "A line count is never
    /// decremented below -999". The sentinel is sticky — a window the game
    /// parked at -999 to suppress "[MORE]" (§8.8.3.2.6) stays there.
    pub fn tick_line_count(&mut self) {
        let lc = self.line_count_signed();
        if lc == NEVER_MORE {
            return;
        }
        self.line_count = lc.saturating_sub(1).max(NEVER_MORE) as u16;
    }

    /// Reload the line count to a full window's worth of lines. Frotz does the
    /// equivalent (`line_count = 0`, counting the other way) for all eight
    /// windows whenever a keystroke actually arrives — see
    /// `console_read_input`/`console_read_key` — which is what stops the count
    /// drifting down to the -999 floor over a long game.
    pub fn reload_line_count(&mut self, cell: V6Cell) {
        self.line_count = self.more_interval(cell) as u16;
    }

    /// One new-line in the *scrolling prose* regime (v6 window 0, or an Inform
    /// v6 library's wrap+scroll main window): the cursor returns to the left
    /// margin and drops a line, except on the bottom line where the window
    /// scrolls under a stationary cursor. Mirrors frotz `screen_new_line`
    /// (`if (y_cursor + 2 * font_height - 1 > y_size) scroll else y_cursor +=
    /// font_height`), and ticks the line count (§8.8.3.2.2).
    ///
    /// The *paint* regime deliberately does not use this: painted text keeps
    /// running past the bottom of its window (runs are screen-absolute), so
    /// clamping there would move glyphs the games expect to stay put.
    pub fn prose_new_line(&mut self, cell: V6Cell) {
        self.x_cursor = self.left_margin.saturating_add(1);
        if self.scrolling() {
            self.tick_line_count();
        }
        let fh = cell.h() as u32;
        if self.y_cursor as u32 + 2 * fh - 1 <= self.y_size as u32 {
            self.y_cursor += cell.h();
        } else {
            // The window scrolled under a stationary cursor, so everything
            // already on screen moved up one line and the top line left the
            // window (SQ-0697). Move the shadow with it — a record that claimed
            // the old rows would freeze the prose in places it no longer is.
            let top = self.y_coord;
            for t in self.streamed.iter_mut() {
                t.y = t.y.saturating_sub(cell.h());
            }
            self.streamed.retain(|t| t.y >= top);
        }
    }

    /// The grid `(row, col)` the next printed character belongs in — see
    /// [`GridPen`].
    ///
    /// Answers the remembered pen when the pixel cursor is still the one that pen
    /// was left at, and otherwise re-derives from the cursor, which is what every
    /// caller did before SQ-1009.
    pub fn grid_cursor(&self, cell: V6Cell) -> (u16, u16) {
        match self.grid_pen {
            Some(p) if (p.y_cursor, p.x_cursor) == (self.y_cursor, self.x_cursor) => (p.row, p.col),
            _ => (cell.row_of(self.y_cursor) + 1, cell.col_of(self.x_cursor) + 1),
        }
    }

    /// Remember the grid pen against the pixel cursor as it stands NOW — so call
    /// this after the print has finished moving both.
    pub fn set_grid_cursor(&mut self, row: u16, col: u16) {
        self.grid_pen =
            Some(GridPen { y_cursor: self.y_cursor, x_cursor: self.x_cursor, row, col });
    }

    /// The screen-absolute `(y, x)` the window's cursor is at right now, in the
    /// same space [`ZWindow::stream_origin`] records — so the two compare directly.
    pub fn pen(&self) -> (u16, u16) {
        (
            self.y_coord.saturating_add(self.y_cursor).saturating_sub(1),
            self.x_coord.saturating_add(self.x_cursor).saturating_sub(1),
        )
    }

    /// Forget where the last burst of prose started (SQ-0804), so the next glyph
    /// this window streams records a fresh [`ZWindow::stream_origin`].
    pub fn clear_stream_origin(&mut self) {
        self.stream_origin = None;
    }

    /// Shadow one streamed glyph at the window cursor's current screen position
    /// (SQ-0697) — see [`ZWindow::streamed`]. Extends the run in progress when the
    /// glyph continues it in the same style at the next cell; starts a new one
    /// otherwise (a new line, a `set_cursor` jump, a style or colour change).
    pub fn record_streamed(&mut self, ch: char, style: u8, fg: ZColour, bg: ZColour, metric: &V6Metric) {
        // Window coords and cursors are both 1-based, so the absolute position is
        // origin + cursor - 1 (ZMSD §8.8.1/§8.8.3.2).
        let x = self.x_coord.saturating_add(self.x_cursor).saturating_sub(1);
        let y = self.y_coord.saturating_add(self.y_cursor).saturating_sub(1);
        // Where this burst of prose STARTED (SQ-0804) — see `stream_origin`.
        self.stream_origin.get_or_insert((y, x));
        if let Some(last) = self.streamed.last_mut() {
            let end = last.x as u32 + last.px_w(metric);
            if last.y == y
                && end == x as u32
                && last.style == style
                && last.fg == fg
                && last.bg == bg
            {
                last.text.push(ch);
                return;
            }
        }
        // Insurance only: the scroll in `prose_new_line` already bounds this to a
        // screenful for any window that stays put, and a window that never
        // scrolls is one whose prose fits. A story that defeats both still can't
        // grow the buffer without bound.
        if self.streamed.len() >= STREAMED_MAX_RUNS {
            self.streamed.drain(..self.streamed.len() - STREAMED_MAX_RUNS / 2);
        }
        // Streamed PROSE has no grid behind it — the host wraps it — so the cell
        // is the derivation, which is what every consumer used before SQ-1009.
        self.streamed.push(V6Text::derived(y, x, ch.to_string(), style, fg, bg, metric.cell()));
    }

    /// Hand the shadowed prose over to real paint (SQ-0697): the window's box is
    /// about to become `(x, y, w, h)`, and what it already printed does not move
    /// with it (ZMSD §15, "does not change the current display").
    ///
    /// Only the prose the NEW box no longer covers is frozen. That distinction is
    /// the whole safety of this feature. Arthur resizes and moves window 0 around
    /// its narration on almost every turn — the box changes, but it still covers
    /// the text, which is therefore still the window's own live content and still
    /// belongs to the streaming transcript. Shogun's title header is the other
    /// case: window 0 drops from the full 640x400 screen to a 548x64 box at the
    /// bottom, leaving nine lines of header stranded at rows the window no longer
    /// reaches. Freezing on the box CHANGING rather than on the box LEAVING froze
    /// Arthur's prose seven turns running and stacked its stale prompts up as paint.
    ///
    /// Returns `true` when anything was frozen, which is the host's cue to restart
    /// its transcript at the window's new origin.
    pub fn retire_streamed(&mut self, x: u16, y: u16, w: u16, h: u16, metric: &V6Metric) -> bool {
        if self.streamed.is_empty() {
            return false;
        }
        let cell = metric.cell();
        let (left, top) = (x.max(1) as i32, y.max(1) as i32);
        let (right, bottom) = (left + w as i32, top + h as i32);
        let mut kept = Vec::with_capacity(self.streamed.len());
        let mut froze = false;
        for run in std::mem::take(&mut self.streamed) {
            let rx = run.x.max(1) as i32;
            let ry = run.y.max(1) as i32;
            let covered = rx >= left
                && ry >= top
                && rx + run.px_w(metric) as i32 <= right
                && ry + cell.h() as i32 <= bottom;
            if covered {
                kept.push(run);
            } else {
                self.retired.push(run);
                froze = true;
            }
        }
        self.streamed = kept;
        froze
    }

    /// Read property `n` (0–15, ZMSD 1.1 §8.8.3.2). Out-of-range → 0.
    pub fn get_prop(&self, n: u16) -> u16 {
        match n {
            0 => self.y_coord,
            1 => self.x_coord,
            2 => self.y_size,
            3 => self.x_size,
            4 => self.y_cursor,
            5 => self.x_cursor,
            6 => self.left_margin,
            7 => self.right_margin,
            8 => self.interrupt_routine,
            9 => self.interrupt_countdown,
            10 => self.text_style,
            11 => self.colour_data,
            12 => self.font_number,
            13 => self.font_size,
            14 => self.attributes,
            15 => self.line_count,
            _ => 0,
        }
    }
    /// Write property `n` (0–15, ZMSD 1.1 §8.8.3.2). Out-of-range → ignored.
    ///
    /// 16/17 fall in that ignored range on purpose: §8.8.3.2 ends "The true
    /// foreground and true background properties must not be written by
    /// put_wind_prop." They are read-derived from the window's channels in the
    /// `get_wind_prop` arm instead.
    ///
    /// # The one clamp
    ///
    /// This is the crate's ONLY writer of properties 0–7 from a story operand —
    /// `move_window`, `window_size`, `set_cursor`, `set_margins` and `split_window`
    /// all route through here rather than assigning the fields — so
    /// [`WINDOW_PX_CAP`] is enforced once instead of at every consumer. The
    /// consumers are half a dozen plain `+`s in the print path, and a story that
    /// wrote `0xFFFF` here aborted a debug-built host four instructions into `main`
    /// (SQ-1030). Six saturating additions would have fixed the same six sites; a
    /// hand-maintained invariant spread across call sites is the symptom, and the
    /// seventh consumer is written by someone with no reason to know any of this.
    ///
    /// Properties 8–15 are a routine address, a countdown, a style, a colour, a
    /// font and a line count — none of them a pixel, none of them added to another
    /// — so they are stored verbatim. In particular property 15 is SIGNED and has
    /// its own floor at -999 (§8.8.3.2.2.3); clamping it here would break it.
    pub fn put_prop(&mut self, n: u16, v: u16) {
        let v = if n <= 7 { v.min(WINDOW_PX_CAP) } else { v };
        match n {
            0 => self.y_coord = v,
            1 => self.x_coord = v,
            2 => self.y_size = v,
            3 => self.x_size = v,
            4 => self.y_cursor = v,
            5 => self.x_cursor = v,
            6 => self.left_margin = v,
            7 => self.right_margin = v,
            8 => self.interrupt_routine = v,
            9 => self.interrupt_countdown = v,
            10 => self.text_style = v,
            11 => self.colour_data = v,
            12 => self.font_number = v,
            13 => self.font_size = v,
            14 => self.attributes = v,
            15 => self.line_count = v,
            _ => {}
        }
    }

    /// Scroll this grid window's content by `pixels` (ZMSD 1.1 §15
    /// `scroll_window`: "Scrolls the given window by the given number of
    /// pixels (a negative value scrolls backwards, i.e., down) writing in
    /// blank (background colour) pixels in the new lines."). Shifts each
    /// pixel-positioned text run's `y` by `-pixels` (dropping runs that land
    /// fully outside the window's visible height `[1, y_size]`), and shifts
    /// the cell-grid fallback by whole rows (`pixels / V6_FONT_HEIGHT`,
    /// truncated toward zero).
    pub fn scroll_pixels(&mut self, pixels: i16, cell: V6Cell) {
        // Runs are screen-absolute: the scroll region is this window's CURRENT
        // screen rect; runs shift within it and drop when they leave it.
        let top = self.y_coord.max(1) as i32;
        let bottom_edge = top + self.y_size.max(1) as i32 - 1;
        let delta = pixels as i32;
        self.texts.retain_mut(|t| {
            let new_y = t.y as i32 - delta;
            let bottom = new_y + cell.h() as i32 - 1;
            if bottom < top || new_y > bottom_edge {
                false
            } else {
                t.y = new_y.clamp(1, u16::MAX as i32) as u16;
                true
            }
        });
        let rows = pixels / cell.h() as i16;
        self.grid.scroll_rows(rows);
    }
}

/// The v6 8-window table (ZMSD §8.4): windows 0–7, addressed in pixels.
#[derive(Debug, Clone, Default)]
pub struct V6Windows {
    pub windows: [ZWindow; 8],
    pub current: u8, // 0–7
}

/// Where each glyph of `run` starts, as offsets from the run's own origin, with
/// the run's total width appended — so glyph `i` covers `edges[i]..edges[i+1]`.
///
/// One list rather than `i * cell.w` at each site, because on a proportional pen
/// the glyphs are not the same width and the arithmetic is no longer a
/// multiplication (SQ-1009). For a fixed pen every step is `cell.w` and this is
/// exactly what the multiplication gave.
fn glyph_edges(run: &V6Text, metric: &V6Metric) -> Vec<i32> {
    let mut edges = Vec::with_capacity(run.text.chars().count() + 1);
    let mut acc = 0i32;
    for c in run.text.chars() {
        edges.push(acc);
        acc += i32::from(metric.advance(c, run.style));
    }
    edges.push(acc);
    edges
}

/// The contiguous run of GLYPH INDICES `(first, last)` that the screen rect
/// `(top, left)..(top+h, left+w)` covers, or `None` when it covers none.
///
/// Glyph `i` occupies `[rx + edges[i], rx + edges[i+1])` — the PEN's cumulative
/// offsets, not `i * cell.w` — and is covered when that span meets the rect at
/// all, because paint replaces whole glyphs and sub-glyph residue cannot be
/// represented as text. The covered glyphs are always contiguous.
///
/// Extracted so [`trim_run_against_rect`] and the blanks-only erase ask the
/// question once (SQ-1054): two copies of this arithmetic, one of them
/// hand-maintained beside the other, is the shape CLAUDE.md's refactoring policy
/// exists to refuse.
fn covered_glyphs(
    run: &V6Text,
    top: i32,
    left: i32,
    h: i32,
    w: i32,
    metric: &V6Metric,
) -> Option<(usize, usize)> {
    let cell = metric.cell();
    let ry = run.y as i32;
    if ry + cell.h() as i32 <= top || ry >= top + h {
        return None;
    }
    let rx = run.x as i32;
    let edges = glyph_edges(run, metric);
    let n = edges.len() - 1;
    if rx + edges[n] <= left || rx >= left + w {
        return None;
    }
    let mut span: Option<(usize, usize)> = None;
    for i in 0..n {
        if rx + edges[i + 1] > left && rx + edges[i] < left + w {
            span = Some(span.map_or((i, i), |(f, _)| (f, i)));
        }
    }
    span
}

/// The GROUND a run paints: its video state and its colour pair. Two runs sharing
/// one ground put the same pixels in a blank cell, whoever printed it.
type Ground = (bool, ZColour, ZColour);

fn ground_of(run: &V6Text) -> Ground {
    (run.style & STYLE_REVERSE != 0, run.fg, run.bg)
}

/// Whether every glyph `rect` covers in `run` is a BLANK — a cell carrying a
/// background and no letter — **on a ground other than `by`**.
///
/// The ground test is what keeps this from erasing a blank the covering run would
/// have painted identically. See [`V6Windows::erase_blank_cells_in_rect`].
fn covers_only_blanks(
    run: &V6Text,
    by: Ground,
    top: i32,
    left: i32,
    h: i32,
    w: i32,
    metric: &V6Metric,
) -> bool {
    ground_of(run) != by
        && covered_glyphs(run, top, left, h, w, metric)
            .is_some_and(|(a, b)| run.text.chars().skip(a).take(b - a + 1).all(|c| c == ' '))
}

/// Trim `run` against the screen rect `(top, left)..(top+h, left+w)` in pixels:
/// drop it entirely, keep it, or split it into up-to-two remnants. A glyph is
/// erased when its cell intersects the rect at all (paint replaces whole
/// glyphs; sub-glyph residue can't be represented as text).
fn trim_run_against_rect(
    run: V6Text,
    top: i32,
    left: i32,
    h: i32,
    w: i32,
    metric: &V6Metric,
) -> Vec<V6Text> {
    let cell = metric.cell();
    let ry = run.y as i32;
    // Vertical band overlap?
    if ry + cell.h() as i32 <= top || ry >= top + h {
        return vec![run];
    }
    let Some((first_erased, last_erased)) = covered_glyphs(&run, top, left, h, w, metric) else {
        return vec![run];
    };
    let rx = run.x as i32;
    let edges = glyph_edges(&run, metric);
    let chars: Vec<char> = run.text.chars().collect();
    let mut out = Vec::new();
    if first_erased > 0 {
        out.push(V6Text {
            y: run.y,
            x: run.x,
            text: chars[..first_erased].iter().collect(),
            style: run.style,
            fg: run.fg,
            bg: run.bg,
            grow: run.grow,
            gcol: run.gcol,
        });
    }
    if last_erased + 1 < chars.len() {
        out.push(V6Text {
            y: run.y,
            x: (rx + edges[last_erased + 1]) as u16,
            text: chars[last_erased + 1..].iter().collect(),
            style: run.style,
            fg: run.fg,
            bg: run.bg,
            // A run never spans a grid row, so the surviving tail is that many
            // columns further along the same one.
            grow: run.grow,
            gcol: run.gcol.saturating_add(last_erased as u16 + 1),
        });
    }
    out
}

impl V6Windows {
    /// Paint one text run: erase whatever earlier runs its pixels cover (in
    /// EVERY window — the screen is one shared raster), then store it on
    /// window `win`. This is what keeps overprinted status lines legible:
    /// Shogun re-prints its location/score at the same pixel cursor each turn
    /// and relies on the new glyphs replacing the old ones.
    ///
    /// A glyph only erases underneath where it deposits OPAQUE pixels: any
    /// glyph over an opaque background paints its whole cell, but a SPACE on a
    /// transparent background paints nothing — Shogun pads its status fields
    /// with such spaces, and erasing under them would eat the neighbouring
    /// labels. (Non-space ink on transparent bg is approximated as covering
    /// its cell: latest-wins per cell, since a text-run model can't
    /// overstrike.)
    pub fn paint_run(&mut self, win: usize, run: V6Text, metric: &V6Metric) {
        let cell = metric.cell();
        if run.text.is_empty() {
            return;
        }
        // Inherited colours (Default / Standard "current"/"default") are
        // transparent; a real chosen colour paints an opaque block.
        let bg_opaque = !matches!(run.bg, ZColour::Default | ZColour::Standard(0) | ZColour::Standard(1));
        // A run that is ENTIRELY blanks is a CLEARING run: the game printing
        // spaces to wipe a region. Zork Zero blanks the old, LONGER location
        // name ("Banquet Hall") with such runs before repainting the shorter
        // "Great Hall" — those blanks must erase the covered glyphs, or the old
        // tail survives as "Great Hall" + a stale "ll" ("Great Hallll", SQ-0498).
        // A space WITHIN a mixed run stays non-erasing: those are field-padding
        // gaps (Shogun pads its status fields with spaces) and erasing under
        // them would eat a neighbouring label painted in the same row.
        let clearing = run.text.chars().all(|c| c == ' ');
        // Segment bounds come from the PEN, not from `i * cell.w`: the glyphs of
        // a proportional run are not the same width, so the pixels an erasing
        // segment covers are the pen's cumulative offsets (SQ-1009).
        let edges = glyph_edges(&run, metric);
        let chars: Vec<char> = run.text.chars().collect();
        let erases = |i: usize| bg_opaque || clearing || chars[i] != ' ';
        // Walk the run in segments of equal opacity. An OPAQUE segment erases
        // everything under it; a transparent one — a padding space — erases only
        // the BLANK cells under it (SQ-1054), which is the whole of the
        // distinction below.
        let mut i = 0usize;
        while i < chars.len() {
            let e = erases(i);
            let mut j = i + 1;
            while j < chars.len() && erases(j) == e {
                j += 1;
            }
            let (top, left) = (run.y as i32, run.x as i32 + edges[i]);
            let (h, w) = (cell.h() as i32, edges[j] - edges[i]);
            if e {
                self.erase_screen_rect(top, left, h, w, metric);
            } else {
                self.erase_blank_cells_in_rect(top, left, h, w, metric, ground_of(&run));
            }
            i = j;
        }
        if let Some(w) = self.windows.get_mut(win) {
            w.texts.push(run);
        }
    }

    /// Erase only the BLANK cells a rect covers — a cell carrying a background
    /// and no letter (SQ-1054).
    ///
    /// # The two frames this sits between
    ///
    /// A SPACE printed with inherited colours deposits no pixels, so
    /// [`Self::paint_run`] has always let it pass over whatever is beneath it.
    /// That is right for Shogun, which pads its status fields with spaces whose
    /// span reaches a neighbouring label painted earlier in the same row: erasing
    /// there would eat a label that is on the screen.
    ///
    /// It is wrong for a cell whose only content IS a background. Macintosh Zork
    /// Zero's InvisiClues menu highlights the selected topic in reverse video —
    /// `GREAT HALL AREA` plus a trailing reversed space — and deselects it by
    /// re-printing the same characters in normal video. The letters overwrite
    /// their own cells, but the two INTER-WORD spaces do not, so the reversed
    /// blocks the old highlight left at native x=132 and x=167 outlived the
    /// highlight itself and stood on the row as stray marks. Arthur's hint page
    /// shows the same thing, for the same reason.
    ///
    /// A letter cannot be overstruck by a space in a text-run model, and that
    /// approximation stays. A BLANK has no letter to preserve: two spaces cannot
    /// both own one pixel span, so the later one wins and the earlier one goes.
    /// Shogun's neighbours are letters; Zork Zero's leftovers are not.
    ///
    /// # …and the erase is gated on the GROUND, because of a third frame
    ///
    /// `advent.z6`'s help bar is a pure reverse-video row painted as reversed
    /// SPACERS first and the labels over them, so its spacers at native x=17, 33
    /// and 73 sit inside `N = next subject`'s span and the label's own spaces cover
    /// them. Those spacers are the bar: erase them and it comes out moth-eaten,
    /// which is SQ-0504's defect returning. `v6_advent_help_bar` failed on exactly
    /// that when this erase was unconditional.
    ///
    /// What separates it from Zork Zero is not the blank, it is what is printing
    /// over it. advent's label is REVERSED, like the spacer beneath it, so the two
    /// runs would put identical pixels in that cell and the record may as well
    /// stand. Zork Zero's deselected topic is NOT reversed while the blank beneath
    /// it is, so the old block is a ground the new run does not paint and has to
    /// go. Hence [`Ground`]: same ground, same pixels, leave it alone.
    fn erase_blank_cells_in_rect(
        &mut self,
        top: i32,
        left: i32,
        h: i32,
        w: i32,
        metric: &V6Metric,
        by: Ground,
    ) {
        if h <= 0 || w <= 0 {
            return;
        }
        for win in self.windows.iter_mut() {
            // The same three layers `erase_screen_rect` walks, for the same
            // reason: an erase is about pixels and does not care which of them
            // recorded the glyph.
            for layer in [&mut win.texts, &mut win.retired, &mut win.streamed] {
                if !layer.iter().any(|t| covers_only_blanks(t, by, top, left, h, w, metric)) {
                    continue;
                }
                let old = std::mem::take(layer);
                *layer = old
                    .into_iter()
                    .flat_map(|t| {
                        if covers_only_blanks(&t, by, top, left, h, w, metric) {
                            trim_run_against_rect(t, top, left, h, w, metric)
                        } else {
                            vec![t]
                        }
                    })
                    .collect();
            }
        }
    }

    /// Erase a screen-absolute pixel rect: every stored run (any window) loses
    /// the glyphs the rect covers. Backs both `paint_run` and `erase_window`
    /// (which erases the target window's CURRENT screen rect — Shogun erases
    /// its 1-px caret window without disturbing the menu items painted around
    /// it earlier).
    pub fn erase_screen_rect(&mut self, top: i32, left: i32, h: i32, w: i32, metric: &V6Metric) {
        if h <= 0 || w <= 0 {
            return;
        }
        let cell = metric.cell();
        for win in self.windows.iter_mut() {
            // Every layer that records where glyphs are SITTING: what the window
            // is showing now, the prose it left frozen behind when it moved
            // (SQ-0697), and the prose it is still streaming (SQ-0729 — the
            // shadow of the live stream, which is the same pixels seen from the
            // other side). The erase covers pixels, and does not care which layer
            // put them there: fmvpoker erases its bottom window and reprints the
            // running total there, and a shadow that kept the old figure would
            // have "990" standing on the tail of "1000".
            for layer in [&mut win.texts, &mut win.retired, &mut win.streamed] {
                if layer.iter().any(|t| {
                    let ty = t.y as i32;
                    let tx = t.x as i32;
                    ty + (cell.h() as i32) > top
                        && ty < top + h
                        && tx + (t.px_w(metric) as i32) > left
                        && tx < left + w
                }) {
                    let old = std::mem::take(layer);
                    *layer = old
                        .into_iter()
                        .flat_map(|t| trim_run_against_rect(t, top, left, h, w, metric))
                        .collect();
                }
            }
        }
    }
}

/// Structured screen model the host (TUI etc.) reads to render.
///
/// For v3 the host derives the status line by calling `Machine::status_line()`.
/// For v4+ the host reads `upper_window_rows`, `current_window`, `text_style`,
/// and `cursor` to manage windows.
#[derive(Debug, Clone)]
pub struct ScreenState {
    /// Number of rows in the upper (status) window; 0 means no upper window.
    pub upper_window_rows: u16,
    /// Currently selected window: 0 = lower, 1 = upper.
    pub current_window: u8,
    /// Current text-style bitmask (ZMSD §8.7.2):
    ///   value 1 = reverse video, 2 = bold, 4 = italic, 8 = fixed-pitch (ZMSD §8.7.2).
    pub text_style: u8,
    /// Cursor position in the upper window (1-based row, col).
    pub cursor_row: u16,
    pub cursor_col: u16,
    /// Whether output should be buffered (lower window).
    pub buffer_mode: bool,
    /// Whether `show_status` (v3 0OP:0x0C) was requested since last read.
    pub show_status_requested: bool,
    /// Whether the lower window should be cleared (set by `erase_window` 0/-1/-2;
    /// ZMSD §8.7.3). The engine does not model the scrolling lower window's
    /// contents, so it records the request here for the host to drain and act on.
    pub erase_lower_requested: bool,
    /// Upper window character grid (v4+).
    pub upper: UpperWindow,
    /// Were the rows of `upper` BELOW `upper_window_rows` stranded there by a
    /// shrinking `split_window` (an Inform quote box), rather than printed there
    /// deliberately by the game (a menu)? Only the first kind is retired when the
    /// player next acts — see `Machine::retire_stranded_upper_rows` (SQ-0696,
    /// SQ-1088). Transient display state, like `current_fg`: a host Save State
    /// restores it `false`, which keeps whatever is on screen on screen.
    pub upper_rows_stranded_by_split: bool,
    /// Active font number (ZMSD §16): 1 = normal (default), 3 = character-graphics.
    /// This is transient display state — NOT serialised into Quetzal saves.
    pub current_font: u8,
    /// Current logical foreground/background colour (ZMSD §8.3). Transient
    /// display state — NOT serialised into Quetzal saves.
    pub current_fg: ZColour,
    pub current_bg: ZColour,
    /// The v6 8-window table; `Some` only when the loaded story is v6
    /// (v1–5/v7/v8 keep the classic 2-window model above and this stays `None`).
    pub v6: Option<V6Windows>,
    /// The v6 window the game last asked for INPUT through, when that window was a
    /// flowing-prose one (SQ-0585). It is the game's main text window by definition
    /// — the one the player types into — so its output is what the host mirrors as
    /// the transcript. Any OTHER prose window is a display panel, and its text goes
    /// to that window's own `prose` buffer instead of being spliced into the same
    /// stream. `0` until the first input request, which is right for boot: window 0
    /// is the classic main window, and text printed before any read (the banner)
    /// belongs to the transcript.
    ///
    /// Lives on `ScreenState`, not `Machine`, for the same reason as `current_fg`/
    /// `current_bg`: it is an input to what the screen must show, and archiving the
    /// whole `ScreenState` (SQ-0749) is what carries it through a host Save State —
    /// a restore taken mid-read through a secondary panel used to come back with
    /// this at 0, so the panel's typed-input echo went dark until the next read
    /// re-established it.
    pub v6_input_window: u8,
    /// Change GENERATION of the v6 window table (SQ-1191): advanced by
    /// [`ScreenState::v6_mut`], the one door to `&mut V6Windows`, so a host can
    /// tell "the screen may look different" from "nothing moved" with a single
    /// integer compare instead of re-reading eight windows' runs and grids.
    /// Read it through [`ScreenState::v6_generation`].
    ///
    /// A `pub` field on the same terms as a canvas `version` stamp: writers go
    /// through `v6_mut` (the `v6_generation_discipline` test holds that door),
    /// and the only legitimate direct write is a wholesale swap keeping the
    /// counter monotone — `Machine::restart` carries it across the reboot's
    /// fresh `ScreenState` for exactly that reason. Transient display state,
    /// like `current_fg`: never serialised, so a host that installs a restored
    /// `ScreenState` gets a counter with no history and must drop anything it
    /// cached against the old one.
    pub v6_generation: u64,
}

impl Default for ScreenState {
    fn default() -> Self {
        ScreenState {
            upper_window_rows: 0,
            current_window: 0,
            text_style: 0,
            cursor_row: 0,
            cursor_col: 0,
            // ZMSD §8.7.2.5: the lower window is buffered (word-wrapped) by
            // default; a game turns buffering off explicitly via buffer_mode 0.
            buffer_mode: true,
            show_status_requested: false,
            erase_lower_requested: false,
            upper: UpperWindow::default(),
            upper_rows_stranded_by_split: false,
            current_font: 1,
            current_fg: ZColour::Default,
            current_bg: ZColour::Default,
            v6: None,
            v6_input_window: 0,
            v6_generation: 0,
        }
    }
}

/// Is ZMSD §8.3's **Amiga rule** in force for this story? (SQ-0740)
///
/// §8.3 gives each Version 6 window its own foreground/background pair, and then
/// carves out one machine:
///
/// > "Note that a Version 6 interpreter going under the Amiga interpreter number
/// > must use the same pair of colours for all windows when running Infocom's
/// > games. If either is changed, then the interpreter must change the colour of
/// > all text on the screen to match. This simulates the Amiga hardware, which
/// > used two logical colours for text and switched palette to change their
/// > physical colour. This behaviour should not occur when running non-Infocom
/// > games, and modern games should never expect it. An interpreter that does
/// > not wish to handle this behaviour at all should avoid using the Amiga
/// > interpreter number when running Infocom's Version 6 games."
///
/// The test is [`machine_rule`] asked of
/// [`MachineProfile::global_colour_pens`](crate::interpreter::MachineProfile::global_colour_pens),
/// which the Amiga row sets and no other does — Version 6, colours available,
/// and `$1E` naming the machine, every term read back out of the HEADER so the
/// rule survives a `@restart`, a `@restore` and a host Save State without anybody
/// carrying it.
///
/// **"When running Infocom's games."** §8.3.1.1 asks the same question of the
/// palette knob — an interpreter may substitute its own colour values "if and
/// only if they can detect they are running an original Infocom story file" —
/// and gives no mechanism. lanthorn answers both the same way, because the same
/// thing answers them: interpreter 4 is only ever advertised by
/// [`InterpreterProfile::Amiga`](../../app/interpreter/enum.InterpreterProfile.html),
/// which is selected by an Amiga release floppy, by a native Amiga `Pic.data`
/// archive, or by the player naming the number outright — and Infocom is the only
/// publisher who ever shipped a Version 6 story on Amiga media. The third route
/// is the player asking for an Amiga, which is the standard's own framing: the
/// escape hatch it offers is to "avoid using the Amiga interpreter number", so
/// choosing it *is* the opt-in.
pub fn amiga_global_colour_pair(m: &crate::cpu::exec::Machine) -> bool {
    machine_rule(m, |p| p.global_colour_pens)
}

/// The shared shape of every per-machine v6 screen rule: Version 6, colours
/// available, the machine header `$1E` names claims the rule, and the LAUNCH is
/// licensed to present that machine at all (SQ-0872, SQ-1154).
///
/// Three of the four terms are read back out of the HEADER rather than held as a
/// field, which is what makes the rules survive a `@restart`, a Quetzal
/// `@restore` and a host Save State without anybody carrying them:
///
/// - **Version 6** — every rule here is scoped to a Version 6 screen, and below
///   v6 there is one screen pair anyway.
/// - **The machine** ($1E) — the byte the story reads to decide what it is
///   running on, so it is exactly the condition the standard names. `claims`
///   picks which member of [`crate::interpreter::MachineProfile`] is being asked
///   about; a number no row models answers `false`, never a substitute.
/// - **Colours available** (Flags 1 bit 0, §8.3.2/§8.3.3) — with
///   `honor_game_colours` off lanthorn declares itself colourless, the host theme
///   owns the screen, and there is no pair for the windows to share.
///
/// The fourth is [`crate::cpu::exec::Machine::machine_colours_licensed`], and it
/// is a field precisely because it is not a fact about the story: it is the
/// host's colour REGIME for this run, which no story can reach, so carrying it on
/// the `Machine` is what makes it survive all three of those the same way. See
/// that field for the whole argument.
///
/// **The fourth term belongs HERE and not at a call site** (SQ-1154). The
/// symptom was reported on the Amiga's shared pens, and gating
/// [`amiga_global_colour_pair`] would have fixed the Amiga and left the
/// Macintosh's screen page — the other caller — broken and undiscovered, because
/// no Macintosh case asserts a GROUND. They assert pairs, and under a host regime
/// the pair is already correct: it is the snapped host pair. Measured on
/// `arthur-r74-s890714.z6`, `--interpreter 3`, `--colour terminal`: a pure black
/// ground where the terminal asked for `#1A1B26`, by a different rule and the
/// same route.
fn machine_rule(
    m: &crate::cpu::exec::Machine,
    claims: fn(&crate::interpreter::MachineProfile) -> bool,
) -> bool {
    m.machine_colours_licensed
        && m.mem.version() == 6
        && m.mem.read_byte(0x01) & 0x01 != 0
        && crate::interpreter::machine_of(&m.mem).is_some_and(claims)
}

/// ZMSD §11.1.3's interpreter number for the Amiga, from the machine table.
pub use crate::interpreter::AMIGA_INTERPRETER_NUMBER;

/// The pair of pens the whole screen is painted with under
/// [`amiga_global_colour_pair`], as `(foreground, background)`; `None` when the
/// rule is not in force. (SQ-0740)
///
/// This is the pair the machine BOOTS with, and — because Infocom's window-0 gate
/// means Journey's only `set_colour` never lands — the pair Journey is played on:
/// header bytes `$2D`/`$2C`, which under the Amiga row carry `DEF_FORE 9` (white)
/// over `DEF_BACK 12` (**dark** grey `$444`), read out of the release floppies'
/// own interpreters rather than out of `amiga/yzip.h` — see
/// [`crate::interpreter::AMIGA_DEFAULT_BACKGROUND`] for the disassembly (SQ-0822).
///
/// **Why the host needs it, and why §8.3.3 alone was not enough.** Those two
/// bytes are what §8.3.3 tells the *story* about the interpreter's defaults, and
/// lanthorn wrote them faithfully — but nothing ever PAINTED them, so a v6 window
/// left at [`ZColour::Default`] rendered in the host terminal's theme and an Amiga
/// looked exactly like an IBM PC on screen. On the real machine the two registers
/// are not advice, they are the screen: every pixel no picture and no `set_colour`
/// claimed is the background pen. Returning the pair here lets the host paint the
/// page it has been advertising all along.
///
/// A window-0 `set_colour` still wins over this wherever the game made one — it
/// moves the pens, and the model carries the moved pair on the window itself
/// (Zork Zero's black-on-light-grey page). This is the ground beneath that.
pub fn amiga_screen_pair(m: &crate::cpu::exec::Machine) -> Option<(ZColour, ZColour)> {
    amiga_global_colour_pair(m).then(|| crate::interpreter::header_pair(&m.mem))
}

/// The MACHINE's own screen pair for a Version 6 frame, `(foreground,
/// background)` — the ground every window that names no colour of its own is read
/// on — or `None` on a machine that has no such thing (SQ-0846, SQ-0872).
///
/// Two machines answer, and they answer for the same reason: their §8.3.3 default
/// colours are not advice about a terminal, they are the screen.
/// [`MachineProfile::v6_screen_page`](crate::interpreter::MachineProfile::v6_screen_page)
/// is the flag, and both pairs arrive here the same way — through header bytes
/// `$2D`/`$2C`, which is where §8.3.3 already had the interpreter publishing them
/// to the story.
///
/// - The **Amiga** (interpreter 4), for §8.3's own reason: one pair of pens for
///   the whole screen, shared and unmoving. [`amiga_screen_pair`] carries the full
///   rule, because on that machine the pens also govern `set_colour`; this is the
///   strictly weaker half, and the Amiga answers both.
/// - The **Macintosh** (interpreter 3), for a plainer one: a white page under
///   black ink was what a Mac window WAS, and `mac/xzip.lst` states it outright
///   (`SetColor := (zWHITE*256) + zBLACK; { Mac defaults: white under black }`).
///   Nothing about the pens is claimed — a Mac `set_colour` behaves exactly as it
///   did — only the ground beneath a window that asked for nothing.
///
/// **This is the function that used to be two.** The Amiga's half lived here and
/// the Macintosh's in `app::session::machine_screen_pair`, gated on a constant
/// `blorb` happened to carry — one concept in two crates, and the reason `zvm-cli`
/// could see one machine's rule and not the other's (SQ-0872). Which machines
/// answer is now a column of the table rather than a chain of `if`s, so adding a
/// third is a `true` and not a new function.
///
/// **This is what SQ-0846 was reported as**, on `stories/Zork Zero Disk.image`
/// (release 296, serial 881019): the status banner's location and score text came
/// out grey on the white artwork and read as missing. Zork Zero on the Macintosh
/// **never calls `set_colour` at all**, so with nothing painting `$2C`/`$2D` the
/// ink fell all the way through to the host theme's grey, while the white it sat
/// on was the game's own two-colour plate. That is SQ-0740's Amiga finding
/// exactly, one machine later.
///
/// The colour bit (`$01` bit 0) gates every arm, which is what makes
/// `honor_game_colours = false` a no-op here: a colourless interpreter is never
/// given the machine's pair to publish, and the header then carries zvm's own
/// §8.3.2 seed, which is nobody's machine.
pub fn machine_screen_pair(m: &crate::cpu::exec::Machine) -> Option<(ZColour, ZColour)> {
    machine_rule(m, |p| p.v6_screen_page).then(|| crate::interpreter::header_pair(&m.mem))
}

impl ScreenState {
    /// The v6 window table's change generation (SQ-1191).
    ///
    /// Moves whenever the table MAY have changed — every `&mut V6Windows` is
    /// handed out by [`ScreenState::v6_mut`], which advances it — so two equal
    /// readings promise the table reads back identically, while two different
    /// readings promise nothing beyond "look again". Deliberately conservative
    /// in that direction: a mutable borrow that ends up writing nothing costs a
    /// caching reader one rebuild, never a stale screen.
    ///
    /// What it does NOT cover, on purpose: facts a v6 frame reads from
    /// elsewhere — header memory (screen dims `$20`–`$25`, the §8.3.3 default
    /// pair `$2C`/`$2D`), [`Machine::v6_win0_out_chars`], `v6_input_window` —
    /// are cheap scalar reads a caching reader keys on directly, and stamping
    /// them here would mean bumping from inside `Memory` writes. Monotone
    /// across `Machine::restart`; a `ScreenState` a host installs wholesale
    /// (a restored save) carries a counter with no history, so any cache keyed
    /// on the old screen's numbers must be dropped with the old screen.
    ///
    /// [`Machine::v6_win0_out_chars`]: crate::cpu::exec::Machine
    pub fn v6_generation(&self) -> u64 {
        self.v6_generation
    }

    /// The one door to mutable v6 window state (SQ-1191): the 8-window table,
    /// with [`ScreenState::v6_generation`] advanced on the way through.
    ///
    /// Every mutation path — paint, erase, moves and resizes, scrolls, cursor
    /// and margin writes, the Amiga pens repaint — reaches the table through
    /// this borrow, which is what lets one counter stand in for all of them.
    /// The `v6_generation_discipline` test keeps it the one door: the only
    /// `.v6.as_mut(` in the crate is the line below, so the next mutator
    /// cannot forget the bump by never being offered a bumpless spelling.
    pub fn v6_mut(&mut self) -> Option<&mut V6Windows> {
        if self.v6.is_some() {
            self.v6_generation = self.v6_generation.wrapping_add(1);
        }
        self.v6.as_mut()
    }

    /// Apply a `set_colour` under [`amiga_global_colour_pair`]: move the
    /// machine's two text "pens" and repaint the screen through them. (SQ-0740)
    ///
    /// `fg`/`bg` are the decoded channel requests (`None` = "leave this channel
    /// alone", the opcode's 0 sentinel). `fg_under_cursor`/`bg_under_cursor` flag
    /// the §8.3.1 colour **-1**, "the colour of the pixel under the cursor".
    ///
    /// **The window-0 gate, and why it beats the letter of §8.3.** Infocom's own
    /// Amiga interpreter (`amiga/yzip3.c`) says the two text colours "are now
    /// 'global', meaning they *can't* be changed for a single word on the screen,
    /// or for a certain window", and then states the rule outright: "we allow text
    /// colors to be changed only in window 0, and ignore requests in other windows
    /// (except for the special case of bg = -1)". §8.3 does not mention that gate.
    /// It is nonetheless what this implements, for two reasons that point the same
    /// way:
    ///
    /// - §8.3's stated purpose is to *"simulate the Amiga hardware"*. A reading of
    ///   it that makes lanthorn diverge from that hardware defeats the rule's own
    ///   reason for existing, and Infocom's shipped interpreter is the better
    ///   authority on how Infocom's own games looked on it.
    /// - It is what the machine actually did. `Journey - The Quest Begins.adf`
    ///   (release 30, serial 890322) makes exactly one `set_colour(9, 2)` — white
    ///   ink, black page — and makes it on **window 3**. Applying it globally
    ///   paints Journey black; contemporary Amiga walkthrough material shows the
    ///   game on its *default* pair instead, light grey page with white text,
    ///   which is `DEF_BACK 11` / `DEF_FORE 9` from Infocom's released
    ///   `amiga/yzip.h`. The real machine ignored the call, exactly as yzip3.c
    ///   says it would.
    ///
    /// If a later reader is tempted to "fix" this back to the letter of the
    /// standard: that is the change, and this is why it was not made.
    ///
    /// **Why -1 is not a pen move.** Infocom's carve-out for `bg = -1` is not an
    /// exception to the sharing rule, it is a different request altogether —
    /// "-1" names no colour,
    /// so there is nothing to load into a register; it asks for the glyph to be
    /// drawn *over what is already there*. Zork Zero prints its banner labels
    /// under `COLOR 2 -1` precisely so the ribbon art shows through them, and
    /// loading a real background there would paint opaque boxes over the art. So
    /// a -1 channel stays a per-window paint request, exactly as it is on every
    /// other machine, and leaves the pens where they were.
    ///
    /// **What "change the colour of all text on the screen" rewrites.** Every
    /// glyph the v6 screen model holds — the character grids, the pixel-positioned
    /// runs (`texts`), the prose a window has streamed (`streamed`) and the prose
    /// it has left frozen behind it (`retired`) — takes the new pens, along with
    /// every window's own pair and the current pair the prose stream tags its runs
    /// from. The colours are REWRITTEN rather than resolved late on purpose: on
    /// the Amiga this is a hardware repaint with no Z-machine event behind it, so
    /// after it happens the model must simply *be* the new screen. Every consumer
    /// — the render path, `/dump-windows`, a host Save State — then sees one
    /// truth without knowing this rule exists.
    ///
    /// **Except a transparent background**, which is not repainted because it was
    /// never painted: a glyph whose stored background is [`ZColour::Default`] put
    /// nothing behind itself (that is how -1 and an inherited channel render), so
    /// there is no pen-0 pixel on the screen for a register move to reach. Its
    /// foreground still follows, which is the whole of the reported symptom — the
    /// prose that stayed white while the game had asked for black.
    ///
    /// The same rule governs a WINDOW's own background, and it has to: a window's
    /// `bg` is the page the renderer fills its box with, and Journey never gives
    /// windows 0–2 one — they are transparent over the illustration behind them.
    /// Loading black into all four anyway paints the frame's own picture out of
    /// existence (measured on `Journey - The Quest Begins.adf`, release 30, serial
    /// 890322: a 115×61 hybrid frame collapsed from 730 distinct cell styles to 8).
    /// Nothing was ever drawn in pen 0 there, so there is nothing for the pen to
    /// change — one rule, applied to glyphs and to the pages under them alike.
    pub fn set_amiga_colour_pair(
        &mut self,
        win: u8,
        fg: Option<ZColour>,
        bg: Option<ZColour>,
        fg_under_cursor: bool,
        bg_under_cursor: bool,
    ) {
        if self.v6.is_none() {
            return;
        }
        // 0. **Infocom's window-0 gate.** `amiga/yzip3.c`, above `set_color`:
        //    "We allow text colors to be changed only in window 0, and ignore
        //    requests in other windows (except for the special case of bg = -1)."
        //    A request from any other window is dropped whole — it moves no pen and
        //    does not reach the window that made it. See the doc comment for why
        //    Infocom's interpreter outranks the letter of §8.3 here.
        if win != 0 && !bg_under_cursor {
            return;
        }
        // 1. The window that asked takes the request whole, exactly as it would on
        //    any other machine — including a -1 channel, which is how it says
        //    "draw over what is already here" and is the one thing the sharing
        //    rule must not take away from it.
        let mut mirror = None;
        if let Some(v6) = self.v6_mut() {
            let current = v6.current;
            if let Some(w) = v6.windows.get_mut(win as usize) {
                if let Some(c) = fg {
                    w.fg = c;
                }
                if let Some(c) = bg {
                    w.bg = c;
                }
                w.colour_data = crate::cpu::exec::pack_colour_data(w.fg, w.bg);
                if win == current {
                    mirror = Some((w.fg, w.bg));
                }
            }
        }
        // 2. §8.3: whatever the request moved the PENS to is shared by every
        //    window, and the text already on the screen changes with it. A -1
        //    channel names no colour, so it moves no pen (see the doc comment) —
        //    and neither does a request from a window other than 0, which reaches
        //    here only through Infocom's `bg = -1` exception and is that window's
        //    own transparency, not a register move.
        let pen_fg = if fg_under_cursor || win != 0 { None } else { fg };
        let pen_bg = if bg_under_cursor || win != 0 { None } else { bg };
        if pen_fg.is_some() || pen_bg.is_some() {
            self.repaint_amiga_pens(pen_fg, pen_bg);
        }
        // 3. The prose stream tags its runs from the current pair, which follows
        //    the current window exactly as it does on every other machine.
        if let Some((fg, bg)) = mirror {
            self.current_fg = fg;
            self.current_bg = bg;
        }
    }

    /// The repaint half of [`Self::set_amiga_colour_pair`]: load the pens that
    /// MOVED and change every glyph already on the screen to match.
    ///
    /// Per channel, because a `set_colour` moves one, the other or both: the
    /// opcode's 0 sentinel means "leave this channel alone", and a channel left
    /// alone is a pen that has not moved and so has nothing to repaint. Reading a
    /// stationary channel back off the current pair instead is what silently
    /// dragged Zork Zero's light-grey page to transparent — the pair follows the
    /// current WINDOW, which is not where the pens live.
    fn repaint_amiga_pens(&mut self, fg: Option<ZColour>, bg: Option<ZColour>) {
        let Some(v6) = self.v6_mut() else { return };
        for w in v6.windows.iter_mut() {
            if let Some(fg) = fg {
                w.fg = fg;
            }
            if let (Some(bg), false) = (bg, matches!(w.bg, ZColour::Default)) {
                w.bg = bg;
            }
            w.colour_data = crate::cpu::exec::pack_colour_data(w.fg, w.bg);
            for c in w.grid.cells.iter_mut() {
                if let Some(fg) = fg {
                    c.fg = fg;
                }
                if let (Some(bg), false) = (bg, matches!(c.bg, ZColour::Default)) {
                    c.bg = bg;
                }
            }
            for t in w.texts.iter_mut().chain(w.streamed.iter_mut()).chain(w.retired.iter_mut()) {
                if let Some(fg) = fg {
                    t.fg = fg;
                }
                if let (Some(bg), false) = (bg, matches!(t.bg, ZColour::Default)) {
                    t.bg = bg;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Output stream state
// ---------------------------------------------------------------------------

/// One frame of nested stream-3 redirection.
struct Stream3Frame {
    /// Base address of the table in dynamic memory.
    table_addr: u32,
    /// Bytes written so far into this frame (accumulated before we flush).
    buf: Vec<u8>,
    /// Resolved box width in pixels for v6's optional 3rd `output_stream`
    /// operand (ZMSD §15 `output_stream`: "In Version 6, a width field may
    /// optionally be given: text will then be justified as if it were in the
    /// window with that number (if width is zero or positive) or a box
    /// -width pixels wide (if negative)."). `None` means the operand was
    /// omitted — text is stored verbatim, unwrapped (pre-existing behaviour).
    width_px: Option<u16>,
}

/// Manages all four Z-machine output streams plus the selected input stream.
///
/// Streams 1 (screen) and 2 (transcript) are on/off flags; only stream 1
/// defaults to on.  Stream 3 redirects text to a memory table and can nest.
/// Stream 4 (command log) is flag-only.  The input stream (`input_stream`
/// opcode) is recorded here too; the engine drives all input through the host,
/// so this field only remembers the game's selection.
pub struct StreamState {
    /// Stream 1 (screen) active.
    pub stream1: bool,
    /// Stream 2 (transcript) active.
    pub stream2: bool,
    /// Stream 4 (command log) active.
    pub stream4: bool,
    /// Selected input stream: 0 = keyboard (default), 1 = command file.
    /// Recorded for the host; the engine never reads input from a file itself.
    pub input_stream: u8,
    /// Stack of active stream-3 frames (nested up to 16).
    stream3_stack: Vec<Stream3Frame>,
    /// Everything routed to stream 2 while it was selected (ZMSD §7.1.2:
    /// stream 2 is "the game transcript"). Writing it to a FILE is a host
    /// concern the app does not implement (§7.6.5 lets an interpreter decline
    /// external files, and `output_stream 2` warns the player); the model
    /// still has to route text here so the routing is correct the day a file
    /// sink exists — in particular the v6 per-window "copy to stream 2"
    /// attribute (§8.8.3.1 attribute 2), which decides *which* windows'
    /// text a transcript would contain.
    stream2_buf: String,
}

impl Default for StreamState {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamState {
    pub fn new() -> Self {
        StreamState {
            stream1: true,
            stream2: false,
            stream4: false,
            input_stream: 0,
            stream3_stack: Vec::new(),
            stream2_buf: String::new(),
        }
    }

    /// Append `s` to the transcript sink (see [`StreamState::stream2_buf`]).
    /// Callers gate this on stream 2 being selected AND — in v6 — on the
    /// printing window carrying attribute 2.
    pub fn write_stream2(&mut self, s: &str) {
        self.stream2_buf.push_str(s);
    }

    /// The transcript text accumulated so far.
    pub fn stream2_text(&self) -> &str {
        &self.stream2_buf
    }

    /// True when stream 3 is active (text goes to memory, not screen).
    pub fn stream3_active(&self) -> bool {
        !self.stream3_stack.is_empty()
    }

    /// Select (push) stream 3 with a table at `table_addr` (ZMSD §7.1.2.5).
    /// `width_px` is the resolved box width in pixels for v6's optional 3rd
    /// operand (the caller resolves a window-number-vs-negative-pixel-width
    /// operand into pixels before calling, since that needs the v6 window
    /// table which lives outside `StreamState`); `None` when the operand was
    /// omitted.
    pub fn push_stream3(&mut self, table_addr: u32, width_px: Option<u16>) {
        if self.stream3_stack.len() < 16 {
            self.stream3_stack.push(Stream3Frame { table_addr, buf: Vec::new(), width_px });
        }
    }

    /// Deselect (pop) stream 3: write accumulated bytes into memory, update
    /// the length word, and return.
    ///
    /// **Two layouts, and the width operand chooses between them.**
    ///
    /// Plain (no width): ZMSD §7.1.2.1 — "When the stream is deselected, the
    /// initial word of the table holds the number of characters printed and
    /// subsequent bytes hold those characters."
    ///
    /// Formatted (a v6 width was given): ZMSD §15 `output_stream` — "Then the
    /// table will contain not ordinary text but formatted text: see
    /// print_form", and §15 `print_form` says what that is: "It is a sequence
    /// of lines, terminated with a zero word. Each line is a word containing
    /// the number of characters, followed by that many bytes which hold the
    /// characters concerned."
    ///
    /// So a width does not merely insert newlines into the plain layout — it
    /// changes the layout, and the reader is [`Machine`]'s `print_form`
    /// (EXT:0x1A) rather than the game's own byte walk. Arthur release 54 is
    /// the game that proves it: its box messages are `output_stream 3, table,
    /// 0` (justify to window 0) followed by `print_form table` into window 3,
    /// and against the plain layout the whole box came out empty (SQ-1006).
    ///
    /// The wrap itself happens here, at close, on the whole accumulated buffer
    /// (splitting on ASCII spaces) rather than incrementally per printed word
    /// the way Frotz's `memory_word` does it.
    pub fn pop_stream3(&mut self, mem: &mut Memory, metric: &V6Metric) {
        if let Some(frame) = self.stream3_stack.pop() {
            let total_width = match frame.width_px {
                Some(w) => {
                    let (bytes, total_width) = wrap_stream3_text(&frame.buf, w, metric);
                    // One (count word, bytes) record per line, then a zero word.
                    // `wrap_stream3_text` marks the breaks with ZSCII 13 (§7.1.2.2.1),
                    // which is how the lines are recovered — the separator itself is
                    // not stored, because a line's count covers its characters only.
                    let mut at = frame.table_addr;
                    for line in bytes.split(|&b| b == 13) {
                        mem.write_word(at, line.len() as u16);
                        at += 2;
                        for &b in line {
                            mem.write_byte(at, b);
                            at += 1;
                        }
                    }
                    mem.write_word(at, 0);
                    total_width
                }
                None => {
                    let n = frame.buf.len() as u16;
                    mem.write_word(frame.table_addr, n);
                    for (i, &b) in frame.buf.iter().enumerate() {
                        mem.write_byte(frame.table_addr + 2 + i as u32, b);
                    }
                    metric.run_px(&zscii_run(&frame.buf), 0)
                }
            };
            // ZMSD §7.1.2.1: in v6, deselecting stream 3 stores "the total
            // width of printing (in units)" in header word $30. Infocom games
            // MEASURE string widths this way — Shogun prints its status
            // fields to stream 3 and reads $30 back to right-align them; an
            // unwritten $30 collapses that math to garbage columns.
            if mem.version() == 6 {
                mem.write_word(0x30, total_width.min(u16::MAX as u32) as u16);
            }
        }
    }

    /// Append raw ZSCII bytes to the current stream-3 buffer (ZMSD §7.1.2.5:
    /// each output character is stored as a single byte, not UTF-8). Callers
    /// must convert chars to ZSCII themselves (`Memory::zscii_from_unicode`) —
    /// `StreamState` has no access to the story's custom Unicode table.
    pub fn write_stream3_bytes(&mut self, bytes: &[u8]) {
        if let Some(frame) = self.stream3_stack.last_mut() {
            frame.buf.extend_from_slice(bytes);
        }
    }
}

/// The characters a slice of stream-3 ZSCII bytes stands for, for MEASUREMENT
/// only.
///
/// Stream 3 stores one byte per output character (ZMSD §7.1.2.5) and the pen is
/// keyed by the same byte, so this is the identity mapping written down rather
/// than a text codec — nothing decoded here is ever printed.
fn zscii_run(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// Word-wrap a stream-3 buffer to `width_px` pixels at the machine's own PEN,
/// replacing the space at each wrap point with a
/// ZSCII 13 newline (ZMSD §7.1.2.2.1: "Newlines are written to output stream
/// 3 as ZSCII 13") — mirrors Frotz's `memory_word`/`memory_close`
/// (`redirect.c`): a word that would overflow the current line drops its
/// leading space and starts a fresh line instead. Existing embedded ZSCII 13
/// bytes are treated as hard breaks: they end the current line without being
/// counted as printable width, and line-width accounting restarts after them.
/// Returns the rewritten bytes and the total width (sum of every completed
/// line's pixel width, hard-broken or wrapped) for header $30.
///
/// **The pen, not the declared cell** (SQ-1009). Header $30 is how an Infocom v6
/// game MEASURES a string it is about to right-align, and the machine measured
/// what it was going to draw. Arthur's Amiga press right-aligns its date field at
/// `x_size - $30`: told the declared 8 per character it puts the field 25 px left
/// of where the machine did and the proportional glyphs then run off the end of
/// the bar. Measured roman, because a stream-3 buffer accumulates across style
/// changes and carries none of them.
fn wrap_stream3_text(buf: &[u8], width_px: u16, metric: &V6Metric) -> (Vec<u8>, u32) {
    let px = |w: &[u8]| metric.run_px(&zscii_run(w), 0);
    let space = metric.run_px(" ", 0);
    let mut out = Vec::with_capacity(buf.len());
    let mut total: u32 = 0;
    for segment in buf.split(|&b| b == 13) {
        let mut line_width: u32 = 0;
        let mut first = true;
        for word in segment.split(|&b| b == b' ') {
            if first {
                first = false;
                line_width = px(word);
                out.extend_from_slice(word);
                continue;
            }
            let candidate = line_width + space + px(word);
            if line_width > 0 && candidate > width_px as u32 {
                total += line_width;
                out.push(13);
                line_width = px(word);
                out.extend_from_slice(word);
            } else {
                out.push(b' ');
                out.extend_from_slice(word);
                line_width = candidate;
            }
        }
        total += line_width;
        out.push(13); // restore the hard break consumed by `split`
    }
    out.pop(); // the loop always adds one trailing 13 too many
    (out, total)
}

// ---------------------------------------------------------------------------
// Header capability bits (ZMSD §11.1)
// ---------------------------------------------------------------------------

/// Default interpreter number (header 0x1E) per Frotz's rule (ux_init.c): IBM PC
/// (6) for v6 story files, DECSystem-20 (1) otherwise. v6 is rejected at load,
/// so in practice every loaded game defaults to 1.
pub fn default_interpreter_number(version: u8) -> u8 {
    if version == 6 { 6 } else { 1 }
}

/// Set interpreter capability bits in the story header at machine startup.
///
/// The bit meanings are ZMSD §11.1's "Flags 1" / "Flags 2" tables; the per-bit
/// reasoning lives beside each mask below. In outline:
///   - Flags1 (v1–3): clear "status line not available" and "variable-pitch
///     font default"; set "screen-splitting available". Bit 1 is the game's
///     status-line kind — left alone.
///   - Flags1 (v4+): advertise bold, italic, fixed-space and timed keyboard
///     input; advertise pictures for v6. Colour (bit 0) and sound (bit 5) are
///     capability-driven — see `advertise_colour` / `advertise_sound`.
///   - Flags2: these are the GAME's requests, so we only clear what we cannot
///     honour — menus (bit 8, `make_menu` is a stub). Font 3 / pictures (bit 3)
///     and mouse (bit 5) are provided and stay as the game left them; undo
///     (bit 4) is advertised for v5+; colour (bit 6) and sound (bit 7) are
///     capability-driven. Transcript (bit 0) and fixed-pitch (bit 1) are the
///     game's own state.
///   - 0x1E: interpreter number — override, else Frotz's default (6 for v6, else 1).
///   - 0x1F: interpreter version — 'A' (ASCII 0x41), standard v1.1 era.
///   - 0x32/0x33: standard revision number (1.1 → 1, 1).
///
/// Only modifies bytes inside dynamic memory (below static_mem_base); if the
/// header region is read-only (static_mem_base ≤ 0x40) we skip silently.
pub fn init_header_caps(mem: &mut Memory, honor_game_colours: bool, sound_available: bool, interpreter_number: Option<u8>, interpreter_version: Option<u8>, palette: Palette) {
    let version = mem.version();

    // Guard: only write if the header sits in dynamic memory.
    // All story files should have static_mem_base > 0x40 (ZMSD §1.1), but be safe.
    // We check individual addresses before each write via the fact that
    // `write_byte` debug-asserts the range; to avoid panics we only call it
    // if we know memory is writable.  In practice all well-formed stories have
    // dynamic memory covering the header, so this is always fine.

    // Flags1 (byte 0x01): interpreter-writable bits.
    let f1 = mem.read_byte(0x01);
    let new_f1 = if version <= 3 {
        // v3 Flags1 bits (ZMSD §11.1.1):
        //   bit 1: time game (0 = score/turns, set by game — don't touch)
        //   bit 4: status line not available — clear (we support it)
        //   bit 5: screen-splitting available — set
        //   bit 6: variable-pitch font default — clear (use fixed)
        f1 & !((1 << 4) | (1 << 6))   // clear "status line not available" + variable-pitch default
          | (1 << 5)      // screen-splitting available
    } else {
        // v4+ Flags1 bits (ZMSD §11.1, "Flags 1" Version 4+ table):
        //   bit 0: "Colours available?" (V5) — handled separately (advertise_colour)
        //   bit 1: "Picture displaying available?" (V6) — set for v6: pictures are
        //          implemented end to end (draw_picture/picture_data/erase_picture
        //          over the blorb Pict resources). Clear below v6, where the bit
        //          has no meaning.
        //   bit 2: "Boldface available?" — set (rendered via SGR / style spans)
        //   bit 3: "Italic available?" — set (rendered via SGR / style spans)
        //   bit 4: "Fixed-space style available?" — set
        //   bit 5: "Sound effects available?" (V6) — handled separately (advertise_sound)
        //   bit 7: "Timed keyboard input available?" — set: timed `read` and
        //          `read_char` (the time/routine operands) are implemented.
        let base = f1 | (1 << 2) | (1 << 3) | (1 << 4) | (1 << 7);
        if version == 6 { base | (1 << 1) } else { base & !(1 << 1) }
    };
    mem.write_byte(0x01, new_f1);

    // Flags2 (word 0x10–0x11). ZMSD §11.1 "Flags 2": bits 3–8 are the GAME's
    // requests ("If set, game wants to use …"); the interpreter clears the ones
    // it cannot honour and otherwise leaves the request standing.
    //   bit 3: V5 = character-graphics font wanted, V6 = "game wants to use
    //          pictures". PRESERVE either way — §8.1.5.1 says only "an
    //          interpreter which cannot provide the character graphics font
    //          should clear bit 3", and we render font 3 (font3_translate) and
    //          v6 pictures both.
    //   bit 4: "game wants to use the UNDO opcodes" — set for v5+ (save_undo/
    //          restore_undo implemented); pre-v5 has no undo opcodes, so clear.
    //   bit 5: "game wants to use a mouse" — PRESERVE: mouse input is
    //          implemented (read_mouse / mouse_window, `Machine::set_mouse`,
    //          and the host delivers clicks).
    //   bit 6: "game wants to use colours" — handled separately (advertise_colour).
    //   bit 7: "game wants to use sound effects" — handled separately (advertise_sound).
    //   bit 8: "game wants to use menus" — CLEAR: `make_menu` (EXT:0x1B) is a
    //          stub that always branches false, so menus are not available.
    let f2 = mem.read_word(0x10);
    let mut new_f2 = f2 & !(1 << 8);
    if version >= 5 {
        new_f2 |= 1 << 4; // undo available
    } else {
        new_f2 &= !(1 << 4); // pre-v5: no undo
    }
    mem.write_word(0x10, new_f2);

    // Interpreter number (0x1E): explicit override, else Frotz's default
    // (6 for v6, else 1 = DEC-20). `version` was read at the top of this fn.
    let interp = interpreter_number.unwrap_or_else(|| default_interpreter_number(version));
    mem.write_byte(0x1E, interp);

    // Interpreter version (0x1F). `b'A'` = 0x41 is the default and has NO
    // PROVENANCE: it arrived in this function's first commit beside `$1E`'s
    // "6, a common neutral value", which SQ-0872 has since replaced with a
    // sourced machine table, and it was never revisited. A story can PRINT this
    // byte — Shogun r295 renders it as a decimal, so 'A' shows as 65 — so
    // `Machine::set_interpreter_version` exists to override it while SQ-0885
    // works out what each machine actually wrote.
    mem.write_byte(0x1F, interpreter_version.unwrap_or(b'A'));

    // Standard revision (0x32 = major, 0x33 = minor): 1.1 — the only published
    // Z-Machine Standards Document revision (ZMSD 1.1); no "1.2" exists.
    mem.write_byte(0x32, 1);
    mem.write_byte(0x33, 1);

    // Screen dimensions (ZMSD §11.1). Without these the header keeps the story
    // file's defaults (usually 0), and size-sensitive games (notably Bureaucracy)
    // read "0 lines", print "[Screen too small.]" and abort on the first turn.
    // Seed a generous default; the host refines it to the real pane size via
    // `write_screen_dims` once known (and on resize).
    // The boot seed always uses the DEFAULT cell: the host has not resolved a
    // profile yet, and `Machine::set_screen_dims` rewrites all of this with the
    // real one the moment it does (SQ-0917).
    write_screen_dims(mem, DEFAULT_SCREEN_ROWS, DEFAULT_SCREEN_COLS, V6Cell::DEFAULT);

    // Default colours (ZMSD §8.3.2/§8.3.3). Bytes $2C (default background) and
    // $2D (default foreground) exist in V5+. §8.3.3: a colour-capable interpreter
    // "should ... write its default background and foreground colours into bytes
    // $2c and $2d"; §8.3.2: a non-colour interpreter should "write colours 2 and
    // 9 (black and white) ... into the default background and foreground". Both
    // cases are satisfied by black-background / white-foreground, our default
    // presentation. Infocom's own V6 games ship $2C/$2D = 0 (an invalid "current"
    // sentinel); games that read the header defaults to build a colour scheme —
    // Beyond Zork (V5) among them — compute garbage colour numbers from 0/0 and
    // their set_colour calls get ignored, leaving the game monochrome. Seeding
    // valid numbers here makes such games colour correctly.
    write_default_colours(mem, DEFAULT_BG_COLOUR, DEFAULT_FG_COLOUR, palette);

    advertise_colour(mem, honor_game_colours);
    advertise_sound(mem, sound_available);
}

/// The palette a standard colour NUMBER resolves to actual colour through.
///
/// ZMSD §8.3.1.1 makes this an interpreter choice, not a fixed law: "The
/// equivalences between the colour numbers and true colours are *recommended*.
/// The interpreter may allow the user to change the mapping, but the given
/// values should be the default." A colour number is a name for "whatever this
/// machine shows for red", so a host claiming to *be* a particular machine owes
/// the game that machine's colours.
///
/// [`Palette::Standard`] is the §8.3.1 table verbatim and the default — it is
/// what every lanthorn session has always used. [`Palette::Amiga`] is the
/// sibling, for the Amiga interpreter profile (SQ-0719).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Palette {
    /// ZMSD §8.3.1's recommended true-colour table.
    #[default]
    Standard,
    /// The palette Infocom's own Amiga interpreter loaded.
    Amiga,
    /// The IBM PC's EGA colours as Infocom's **XZIP** mapped them — the v1–v5
    /// interpreter. See [`ega_true_colour`].
    IbmXzip,
    /// The same, as **YZIP** mapped them — the Version 6 interpreter. It differs
    /// from [`Palette::IbmXzip`] in exactly one entry, and that entry is white.
    IbmYzip,
    /// The same machine again, showing a **CGA** card: two colours, black and
    /// light grey, and no third (SQ-0956).
    ///
    /// The card is not the machine. An IBM PC running the EGA or MCGA rendition
    /// of a Version 6 game resolves numbers through [`Palette::IbmYzip`], where
    /// white is `#FFFFFF`; put the CGA plates in the same machine and the screen
    /// has two states, `#000000` and `#AAAAAA` — EGA entry 7, which is the value
    /// the XZIP table already gives for white and the value
    /// `machine-screenshots/dos-zorkzero-cga.png` measures for every lit pixel in
    /// the frame, text and artwork alike. So the numbers resolve exactly as XZIP's
    /// do; what makes this a variant of its own is [`Palette::two_colour_card`],
    /// which is a claim about the DISPLAY rather than about a table.
    IbmCga,
}

impl Palette {
    /// Is this palette a **two-state display** — one that has an ink and a page
    /// and nothing else (SQ-0956)?
    ///
    /// Only the CGA card answers. The Macintosh's monochrome plate is two-colour
    /// art on a machine whose screen is not: its interpreter names ordinary
    /// §8.3.1 colours and `mac/xzip.lst` sets a white page under black ink like
    /// any other pair, so nothing about it collapses.
    ///
    /// [`two_colour_card_pair`] is the pair it shows, and
    /// `Machine::set_colour`'s CGA arm is the one caller that matters — see there
    /// for what a story's request means on a display with one bit.
    pub fn two_colour_card(self) -> bool {
        matches!(self, Self::IbmCga)
    }

    /// Does a `HLIGHT` **bold** run light the EGA intensity bit on this display
    /// (SQ-1354)?
    ///
    /// **Only [`Palette::IbmXzip`]**, and every clause of that is read out of
    /// Infocom's own interpreters rather than reasoned from the hardware:
    ///
    /// - **XZIP and EZIP** — the v1–v5 DOS interpreters, which is what this
    ///   palette IS — paint into a text cell's attribute byte and OR bit 3 into
    ///   its foreground nibble for bold. See [`ega_intense`] for the lines.
    /// - **YZIP declines.** The Version 6 interpreter draws its text into a
    ///   graphics screen, and its `md_hlite` (`ibmzip/yzip/sysdep1.c`) reads
    ///   `attrib` only to swap the pair for `REVERSE`; `BOLD` appears nowhere in
    ///   its screen code, in that file or in any of releases 65–71 beside it. A
    ///   bold run on that machine was simply not distinguished, so brightening one
    ///   here would be inventing a behaviour rather than reproducing it.
    /// - **The CGA card declines** for a different reason: it is a *two-state*
    ///   display ([`Palette::two_colour_card`]) whose whole content is `#000000`
    ///   and `#AAAAAA`. A third value is exactly what that variant exists to rule
    ///   out.
    /// - The Amiga and the §8.3.1 table have no intensity bit to light.
    pub fn bold_lights_the_intensity_bit(self) -> bool {
        matches!(self, Self::IbmXzip)
    }
}

/// The `(foreground, background)` a two-colour card shows, as §8.3.1 colour
/// numbers: **white 9 over black 2** (SQ-0956).
///
/// Stated here rather than read from the machine table because it is a fact about
/// the CARD: `zvm::interpreter::IBM_PC_DEFAULT_BACKGROUND` is the PC's blue, which
/// is what the EGA and MCGA renditions of the same machine show, and
/// [`crate::interpreter::IBM_PC_TWO_COLOUR_BACKGROUND`] carries the census that
/// says the CGA plate's is black instead. The ink does not move: white 9 both
/// times, which this palette resolves to the card's `#AAAAAA`.
pub const CGA_CARD_PAIR: (u8, u8) =
    (crate::interpreter::IBM_PC_DEFAULT_FOREGROUND, crate::interpreter::IBM_PC_TWO_COLOUR_BACKGROUND);

/// What a story's `@set_colour(fg, bg)` MEANS on a two-colour card — the pair as
/// requested on every other display, and the card's own two states on that one
/// (SQ-0956).
///
/// # A display with one bit cannot take a pair of colours, but it can take a side
///
/// A CGA card in the 640-wide mode a `.CG1` was drawn for has two states and no
/// third: the page, `#000000`, and the ink, `#AAAAAA` ([`CGA_CARD_PAIR`], resolved
/// through [`Palette::IbmCga`]). A story naming blue is naming something that is
/// not there. What a story CAN say is which of its two channels wants the lit
/// state, and that is one bit — which is exactly what the machine offers back:
/// Zork Zero's own in-game `color` command on a CGA machine presents a **swap** of
/// the two states and nothing else (observed on the emulator).
///
/// So this maps a request onto the card: whichever channel the story named for its
/// INK gets the card's page, and the other gets the card's ink.
///
/// # Why that is a swap and not "the pair as named", which is the surprise
///
/// `machine-screenshots/dos-zorkzero-cga.png` — Zork Zero r393 at the Banquet
/// Hall, a DOS emulator in CGA mode running `zork0.cg1` — censuses **48.3%
/// `#000000`** page under **8.8% `#A0A0A0`** ink, with no second hue in the frame.
/// The story asked for the opposite: r393 issues `set_colour(fg=2, bg=9)` — BLACK
/// ink on a WHITE page — for every video card alike, measured identical across
/// `.cg1`, `.eg1` and `.mg1`, and `dos-zorkzero.png` shows that honoured on the
/// colour rendition at 25.7% `#FFFFFF`. Same story, same release, same machine,
/// opposite polarity; the card is the only thing that moved.
///
/// The user's second visit to the emulator pins the other side of the bit, which
/// is what makes this a rule rather than a fudge fitted to one frame: choosing the
/// light ground from that `color` menu washes the plates out **on the real machine
/// too**, because `.cg1` art is light line work authored for a black ground. Both
/// states of the card's one bit are therefore accounted for by a capture or by the
/// machine, and this function reproduces both.
///
/// # Why it is not `honor_game_colours = false`
///
/// Declining §8.3's palette to the story produces the same boot frame — the game
/// checks the colour flag and issues no `set_colour` at all when it is clear
/// (measured on this press) — and it costs the `color` command, which needs the
/// flag set to do anything. Two states is not no colours. SQ-0806's rule survives
/// for the launch that genuinely has no machine to speak for; see
/// `app::graphics::PictSource::declines_game_colours`.
///
/// A `None` channel is "keep this one" (the opcode's 0 sentinel) and a
/// [`ZColour::Default`] is the -1 "colour under the cursor" carve-out; neither
/// names a colour, so neither can carry the bit, and a request with fewer than two
/// named channels is passed through untouched.
pub fn two_colour_card_request(
    palette: Palette,
    fg: Option<ZColour>,
    bg: Option<ZColour>,
) -> (Option<ZColour>, Option<ZColour>) {
    if !palette.two_colour_card() {
        return (fg, bg);
    }
    let named = |c: Option<ZColour>| match c {
        Some(ZColour::Standard(n)) => true_colour_in(palette, n),
        _ => None,
    };
    let (Some(want_ink), Some(want_page)) = (named(fg), named(bg)) else {
        return (fg, bg);
    };
    // The two channels have to differ for either to be a side of the bit; a story
    // asking for one colour twice is asking for a blank screen, and the card has
    // nothing to say about that.
    if want_ink == want_page {
        return (fg, bg);
    }
    let (card_ink, card_page) = CGA_CARD_PAIR;
    let (ink, page) = if luma15(want_ink) < luma15(want_page) {
        // The story wants the DARKER of its two colours as ink — the polarity the
        // card boots in, page under ink.
        (card_ink, card_page)
    } else {
        (card_page, card_ink)
    };
    (Some(ZColour::Standard(ink)), Some(ZColour::Standard(page)))
}

/// A 15-bit colour's brightness, for ordering two of them. Rec. 601 weights on
/// the 5-bit channels, which is enough to say which of two colours is the darker
/// and is never asked anything finer.
fn luma15(c: u16) -> u32 {
    let (r, g, b) = ((c >> 10) & 31, (c >> 5) & 31, c & 31);
    u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114
}

/// Interpreter default background colour written to header $2C when the host
/// never says otherwise: 2 = black (ZMSD §8.3.1).
pub const DEFAULT_BG_COLOUR: u8 = 2;
/// Interpreter default foreground colour written to header $2D when the host
/// never says otherwise: 9 = white (ZMSD §8.3.1).
pub const DEFAULT_FG_COLOUR: u8 = 9;

/// Clamp a host-supplied default colour to a standard colour number a machine
/// of this `version` could actually have as its own default.
///
/// ZMSD §8.3.1 defines 2..=9 as the true colour names; 0/1 are the "current"/
/// "default" sentinels, 13–14 reserved and 15 transparent — none of which are
/// meaningful as *the interpreter's own* default, so they fall back.
///
/// The greys 10–12 exist "only in Version 6", and there they are perfectly
/// meaningful defaults: Infocom's own Amiga Version 6 interpreter booted with
/// `DEF_BACK 12` — dark grey — as its default page (SQ-0822). So they
/// are accepted for Version 6 and rejected everywhere else, which is exactly
/// what the spec's "only in Version 6" says. (SQ-0719; before that nothing ever
/// offered a grey here, so no existing session's answer moves.)
pub(crate) fn clamp_default_colour(c: u8, fallback: u8, version: u8) -> u8 {
    let ok = (2..=9).contains(&c) || (version == 6 && (10..=12).contains(&c));
    if ok { c } else { fallback }
}

/// Write the interpreter's default background/foreground colours into header
/// bytes $2C and $2D (V5+ only; those bytes have no meaning before V5).
///
/// ZMSD §8.3.3: "If the interpreter can produce colours, it should set bit 0 of
/// 'Flags 1' in the header, and write its default background and foreground
/// colours into bytes $2c and $2d of the header." (§8.3.2 asks a non-colour
/// interpreter for 2 and 9 "either way round", which the 2/9 default satisfies.)
/// Values outside 2..=9 fall back to [`DEFAULT_BG_COLOUR`]/[`DEFAULT_FG_COLOUR`].
pub fn write_default_colours(mem: &mut Memory, bg: u8, fg: u8, palette: Palette) {
    if mem.version() < 5 {
        return;
    }
    let bg = clamp_default_colour(bg, DEFAULT_BG_COLOUR, mem.version());
    let fg = clamp_default_colour(fg, DEFAULT_FG_COLOUR, mem.version());
    mem.write_byte(0x2C, bg);
    mem.write_byte(0x2D, fg);
    write_header_ext_colours(mem, bg, fg, palette);
}

/// The true-colour equivalent of standard colour number `n` (2..=12) IN palette
/// `p`, as a 15-bit RGB value. `None` for the sentinels (0 current, 1 default,
/// -1 pixel-under-cursor), the reserved 13/14 and 15 (transparent, which §8.3.7
/// gives the special value -4 rather than an RGB triple).
///
/// [`Palette::Standard`] is the spec table below verbatim; [`Palette::Amiga`] is
/// the palette Infocom's own Amiga interpreter loaded, which §8.3.1.1 explicitly
/// permits an interpreter to substitute.
///
/// **The palette is asked for by value and never read out of shared state.** A
/// *table* presents as all of them at once — [`crate::machines`] prints every
/// machine's page and ink side by side — and until SQ-1393 the VM's own resolver
/// reached a process-wide atomic instead, so printing a table meant writing state
/// every other thread in the process could see. Under `cargo test`, where a whole
/// crate's cases share one process, that is the SQ-0904 race exactly: a
/// borrow-and-hand-back is atomic to nobody. Asking by value cannot race with
/// anything, and the machine's own answer is [`crate::cpu::Machine::palette`].
pub fn true_colour_in(p: Palette, n: u8) -> Option<u16> {
    match p {
        Palette::Standard => zmsd_true_colour(n),
        Palette::Amiga => amiga_true_colour(n),
        Palette::IbmXzip => ega_true_colour(n, false).or_else(|| zmsd_true_colour(n)),
        Palette::IbmYzip => ega_true_colour(n, true).or_else(|| zmsd_true_colour(n)),
        // The card's two states are black and EGA 7, which is exactly where the
        // XZIP table sends white — so one table serves both (SQ-0956).
        Palette::IbmCga => ega_true_colour(n, false).or_else(|| zmsd_true_colour(n)),
    }
}

/// The IBM PC's colours for standard numbers 2..=9, as 15-bit RGB — Infocom's own
/// mapping of Z-machine colour numbers onto EGA attributes.
///
/// # Source
///
/// The tables compiled into Infocom's IBM interpreters, which are the programs
/// that painted these colours. `yzip/data.c`, the **Version 6** interpreter — the
/// one Shogun's banner names as "IBM Interpreter version 6.68":
///
/// ```text
///   char Zip_to_ega[] = {        /* map ZIP colors to EGA */
///      0,       /* ZIP_BLACK */    4,       /* ZIP_RED */
///      2,       /* ZIP_GREEN */    14,      /* ZIP_YELLOW */
///      1,       /* ZIP_BLUE */     5,       /* ZIP_MAGENTA */
///      3,       /* ZIP_CYAN */     15,      /* ZIP_WHITE */
///      9,       /* ZIP_GREY */     7        /* ZIP_BROWN */ };
/// ```
///
/// and `xzip/data.c`, the v1–v5 one:
///
/// ```text
///   char zip_to_ibm_color[] = {-1, -2, 0, 4, /* nc, def, black, red */
///                                    2, 14, 1, 5, /* green, ylw, blue, mag */
///                                    3, 7};       /* cyan, white */
/// ```
///
/// The RGB for each attribute is `Mcga_palette` in the same YZIP file, whose
/// 6-bit DAC values (`0x00`, `0x15`, `0x2a`, `0x3f`) are the familiar
/// `0x00`/`0x55`/`0xAA`/`0xFF`, and which its own comment says "maps to the same
/// as the EGA colors".
///
/// # The two tables differ in exactly one entry, and it is WHITE
///
/// XZIP sends colour 9 to attribute **7**, `#AAAAAA`, EGA's light grey. YZIP sends
/// it to **15**, `#FFFFFF`. Every other colour is identical between them.
///
/// Both are corroborated by a capture, which is what makes this a finding rather
/// than a transcription: `machine-screenshots/dos-hitchhiker.png` is Version 3 and
/// measures its ink at `#A0A0A0`, and `dos-shogun.png` is Version 6 and measures
/// `#FDFFFF`, with 59% of its text pixels above `#C8C8C8`. Same machine, same
/// colour number, two generations of interpreter, two whites.
///
/// # What is deliberately NOT modelled
///
/// Colours 10..=12 — §8.3.1's light, medium and dark greys, which are v6-only.
/// YZIP's list ends `ZIP_GREY, ZIP_BROWN` against attributes 9 and 7, and 9 is
/// `#5555FF`, a light BLUE. Whatever those two slots are, they are not the
/// standard's three greys, and guessing which of 10/11/12 they answer would be
/// inventing a mapping rather than reading one. They fall through to
/// [`zmsd_true_colour`] until someone reads the YZIP screen code that indexes
/// this table, or measures a frame that uses one.
pub fn ega_true_colour(n: u8, yzip: bool) -> Option<u16> {
    Some(match n {
        2 => 0x0000,                            // black   EGA 0  #000000
        3 => 0x0015,                            // red     EGA 4  #AA0000
        4 => 0x02A0,                            // green   EGA 2  #00AA00
        5 => 0x2BFF,                            // yellow  EGA 14 #FFFF55
        6 => 0x5400,                            // blue    EGA 1  #0000AA
        7 => 0x5415,                            // magenta EGA 5  #AA00AA
        8 => 0x56A0,                            // cyan    EGA 3  #00AAAA
        // The one entry the two interpreters disagree on.
        9 if yzip => 0x7FFF,                    // white   EGA 15 #FFFFFF
        9 => 0x56B5,                            // white   EGA 7  #AAAAAA
        _ => return None,
    })
}

/// The **high-intensity sibling** of an EGA/CGA colour, as 15-bit RGB — what a
/// bold run looked like on an IBM PC (SQ-1354).
///
/// # The rule, and where it is read from
///
/// A DOS text cell is one attribute byte: `bbbbffff`, the low nibble the
/// foreground. Bit 3 of that nibble is the **intensity** bit, so the sixteen
/// text colours are eight base colours and the same eight lit. Infocom's own IBM
/// interpreters set `HLIGHT` bold (§8.7.1's bit `0x02`, `BOLD 2` in their
/// `zipdefs.h`) by ORing that bit into whatever the foreground already was —
/// `md_hlite` in `ibmzip/sysdep.c`, the EZIP (Version 4) interpreter, and
/// verbatim again in `ibmzip/xzip/sysdep.c`:
///
/// ```text
///     if (graphics < 0) THEN {            /* text mode */
///       if (docolor > 0) THEN {
///         if (attrib & REVERSE) THEN
///           curattr = ((curattr >> 4) & 7) | ((curattr & 7) << 4);
///         if (attrib & BOLD) THEN
///           curattr = curattr | 8;        /* intense foreground */
///         }                               /* underlining done manually */
///        else {                           /* black and white */
///         if (attrib & REVERSE) THEN
///           curattr = 0x70;
///         if (attrib & BOLD) THEN
///           curattr = curattr | 8;
/// ```
///
/// (`ibmzip.zip` from Andrew Plotkin's Infocom catalogue,
/// <https://eblong.com/infocom/#terps>, the "IBM PC, C" package.) Both branches
/// light the same bit, so the brightening is the machine's, not the colour
/// mode's — which is why it applies to whatever put the low three bits there: a
/// story's own `set_colour`, or the machine's default ink.
///
/// | base | | lit | |
/// |---|---|---|---|
/// | 0 black `#000000` | → | 8 dark grey | `#555555` |
/// | 1 blue `#0000AA` | → | 9 light blue | `#5555FF` |
/// | 2 green `#00AA00` | → | 10 light green | `#55FF55` |
/// | 3 cyan `#00AAAA` | → | 11 light cyan | `#55FFFF` |
/// | 4 red `#AA0000` | → | 12 light red | `#FF5555` |
/// | 5 magenta `#AA00AA` | → | 13 light magenta | `#FF55FF` |
/// | 6 brown `#AA5500` | → | 14 yellow | `#FFFF55` |
/// | 7 light grey `#AAAAAA` | → | 15 white | `#FFFFFF` |
///
/// Anything already lit — and anything that is not an EGA colour at all — comes
/// back unchanged, because there is no second intensity bit to set.
///
/// # What is deliberately NOT modelled
///
/// **The reverse-then-brighten order.** The excerpt above swaps the nibbles
/// *before* ORing bit 3, so on the real machine a run that is both reversed and
/// bold lights the colour that ends up in front — the pair's old background. Here
/// the caller intensifies the run's logical foreground and lets the terminal
/// perform the single swap, so a REVERSE+BOLD run lights its ground instead. The
/// case is rare enough that carrying the swap through the cell renderer would cost
/// more than it buys; it is named here so it is a known gap rather than a
/// surprise.
pub fn ega_intense(v15: u16) -> u16 {
    match v15 {
        0x0000 => 0x294A, // 0 black       → 8  dark grey    #555555
        0x5400 => 0x7D4A, // 1 blue        → 9  light blue   #5555FF
        0x02A0 => 0x2BEA, // 2 green       → 10 light green  #55FF55
        0x56A0 => 0x7FEA, // 3 cyan        → 11 light cyan   #55FFFF
        0x0015 => 0x295F, // 4 red         → 12 light red    #FF5555
        0x5415 => 0x7D5F, // 5 magenta     → 13 light magenta #FF55FF
        0x0155 => 0x2BFF, // 6 brown       → 14 yellow       #FFFF55
        0x56B5 => 0x7FFF, // 7 light grey  → 15 white        #FFFFFF
        already => already,
    }
}

/// [`ega_true_colour`] for a run the story printed with `HLIGHT` **bold**: the
/// same table with the EGA intensity bit lit (SQ-1354).
///
/// [`ega_intense`] carries the rule and Infocom's own code for it. Which
/// PALETTES actually apply it is a separate question, and
/// [`Palette::bold_lights_the_intensity_bit`] is where that is decided — this
/// function answers for the table alone.
pub fn ega_true_colour_styled(n: u8, yzip: bool, bold: bool) -> Option<u16> {
    ega_true_colour(n, yzip).map(|v| if bold { ega_intense(v) } else { v })
}

/// The Amiga palette for standard colour numbers 2..=12, as 15-bit RGB.
///
/// Source: the `colortable[]` compiled into the Amiga Version 6 interpreter on
/// Infocom's own **release floppies** — the one program that ever painted these
/// colours — read through `colormap[]`, which maps a Z-machine colour number to a
/// table slot. The Amiga's `SetRGB4` takes 4 bits per channel, so the raw entries
/// are `0x0RGB`; they are widened to the Z-machine's 5-bit channels by bit
/// replication (`n << 1 | n >> 3`), the expansion that keeps `$F` at full
/// intensity and `$0` at zero.
///
/// ```text
/// colour        slot            raw     → 15-bit
/// 2  black      colortable[2]  $0000     $0000
/// 3  red        colortable[4]  $0E00     $001D
/// 4  green      colortable[3]  $00C0     $0320
/// 5  yellow     colortable[5]  $0FD0     $037F
/// 6  blue       colortable[0]  $005A     $5540
/// 7  magenta    colortable[6]  $0F0F     $7C1F
/// 8  cyan       colortable[7]  $00EE     $77A0
/// 9  white      colortable[1]  $0FFF     $7FFF
/// 10 light grey colortable[8]  $0AAA     $56B5
/// 11 medium     colortable[9]  $x777     $39CE
/// 12 dark grey  colortable[10] $0444     $2108
/// ```
///
/// (Slot 9 is written `0x7777`; `SetRGB4` uses only the low 12 bits, so the stray
/// high nibble never reached the hardware.)
///
/// **The floppies outrank `amiga/yzip1.c`** (SQ-0822). The leaked development
/// source gives slot 5 as `0x0EE0`, and it is the ONE entry where the source and
/// the shipped program disagree: the byte string
/// `00 5A 0F FF 00 00 00 C0 0E 00 0F D0 0F 0F 00 EE 0A AA 77 77 04 44` appears,
/// identically and once, in every Amiga Version 6 interpreter in `stories/` —
/// `Arthur` (release 54 floppy), `Journey` (release 30), `Zork Zero` (release 366)
/// and `Shogun` (release 295) — with `0F D0` where the source has `0E E0`. Ten
/// entries match the source exactly, which is what makes the eleventh a shipped
/// correction rather than a mis-transcription.
///
/// Five of the eleven entries — black, red, magenta, cyan, white — come out
/// bit-for-bit identical to ZMSD §8.3.1's table, which is a strong sign the
/// standard's "recommended" values were themselves read off an Amiga. Green,
/// blue, yellow and the three greys are where the two genuinely differ.
pub fn amiga_true_colour(n: u8) -> Option<u16> {
    Some(match n {
        2 => 0x0000,  // black       $000
        3 => 0x001D,  // red         $E00
        4 => 0x0320,  // green       $0C0
        5 => 0x037F,  // yellow      $FD0
        6 => 0x5540,  // blue        $05A
        7 => 0x7C1F,  // magenta     $F0F
        8 => 0x77A0,  // cyan        $0EE
        9 => 0x7FFF,  // white       $FFF
        10 => 0x56B5, // light grey  $AAA [V6 only]
        11 => 0x39CE, // medium grey $777 [V6 only]
        12 => 0x2108, // dark grey   $444 [V6 only]
        _ => return None,
    })
}

/// ZMSD §8.3.1's true-colour table for standard colour numbers 2..=12, as
/// 15-bit RGB. The spec's own table, transcribed verbatim; §8.3.1.1 calls these
/// equivalences "recommended" and the interpreter default, and they are
/// lanthorn's default too ([`Palette::Standard`]).
pub fn zmsd_true_colour(n: u8) -> Option<u16> {
    Some(match n {
        2 => 0x0000,  // black
        3 => 0x001D,  // red
        4 => 0x0340,  // green
        5 => 0x03BD,  // yellow
        6 => 0x59A0,  // blue
        7 => 0x7C1F,  // magenta
        8 => 0x77A0,  // cyan
        9 => 0x7FFF,  // white
        10 => 0x5AD6, // light grey  [V6 only]
        11 => 0x4631, // medium grey [V6 only]
        12 => 0x2D6B, // dark grey   [V6 only]
        _ => return None,
    })
}

/// Publish the interpreter's side of the header extension table (ZMSD §11.1.7.3):
/// word 4 = Flags 3, word 5 = true default FOREGROUND, word 6 = true default
/// BACKGROUND (note the fg-before-bg order — the reverse of $2C/$2D).
///
/// All three are marked "Int" and "Rst" in the §11.1.7.3 table, i.e. written by
/// the interpreter and re-stamped on restart/restore, which is why this rides
/// along with every `write_default_colours`.
///
/// Flags 3 is cleared outright: §11.1.7.4 — "The bits in Flags 3 are set by the
/// game to request use of a feature. If the interpreter cannot provide a
/// feature, it must clear the relevant bit" — and §11.1.7.4.1 — "All unused bits
/// in Flags 3 must be cleared by the interpreter." Its only defined bit is 0
/// ("game wants to use transparency"), which we do not provide (§8.3.6 lets a
/// non-transparent interpreter ignore colour 15), so every bit goes to 0.
///
/// Writes are skipped for any word past the table's length, per §11.1.7.2: "If
/// the interpreter needs to write a word which is beyond the length of the
/// extension table, or the extension table doesn't exist at all, then the result
/// is that nothing happens."
fn write_header_ext_colours(mem: &mut Memory, bg: u8, fg: u8, palette: Palette) {
    let ext = mem.read_word(0x36) as u32;
    if ext == 0 {
        return;
    }
    let count = mem.read_word(ext); // word 0 = number of further words
    if count >= 4 {
        mem.write_word(ext + 8, 0); // word 4: Flags 3 — no features provided
    }
    if count >= 5 {
        let true_fg = true_colour_in(palette, fg).unwrap_or(0x7FFF);
        mem.write_word(ext + 10, true_fg); // word 5: true default foreground
    }
    if count >= 6 {
        let true_bg = true_colour_in(palette, bg).unwrap_or(0x0000);
        mem.write_word(ext + 12, true_bg); // word 6: true default background
    }
}

/// Set or clear the Flags1 "colour available" bit (bit 0). No-op for v3, which
/// has no colour capability bit. Re-applied on every header init and whenever
/// the host toggles `honor_game_colours`.
pub fn advertise_colour(mem: &mut Memory, on: bool) {
    if mem.version() < 4 {
        return;
    }
    let f1 = mem.read_byte(0x01);
    let f1 = if on { f1 | 1 } else { f1 & !1 };
    mem.write_byte(0x01, f1);

    // Flags2 bit 6 (word 0x10) is the game's "wants colours" request bit. When
    // colour is off, clear it so a game doesn't proceed believing colour was
    // granted; when on, leave the game's request untouched. Render gates colour
    // regardless, so this is strict-correctness hygiene (ZMSD §11.1.4).
    if !on {
        let f2 = mem.read_word(0x10);
        mem.write_word(0x10, f2 & !(1 << 6));
    }
}

/// Set or clear the sound-effects capability bits: Flags1 bit 5 (v4+ ONLY —
/// in v3 that bit means "screen-splitting available", a different capability,
/// and is left untouched) and Flags2 bit 7 (all versions). Re-applied on
/// every header init and whenever the host toggles `sound_available`.
pub fn advertise_sound(mem: &mut Memory, on: bool) {
    if mem.version() >= 4 {
        let f1 = mem.read_byte(0x01);
        let f1 = if on { f1 | (1 << 5) } else { f1 & !(1 << 5) };
        mem.write_byte(0x01, f1);
    }
    let f2 = mem.read_word(0x10);
    let f2 = if on { f2 | (1 << 7) } else { f2 & !(1 << 7) };
    mem.write_word(0x10, f2);
}

/// Default screen size seeded at header init, before the host reports the real
/// pane size. Generous enough that size-sensitive v4+ games run.
pub const DEFAULT_SCREEN_ROWS: u8 = 24;
pub const DEFAULT_SCREEN_COLS: u8 = 80;

/// v6 font cell size in pixels. Reference interpreters present Infocom v6 on a
/// **non-square 8×16 cell** — the Amiga/DOS profile Frotz uses for every v6 game
/// (`src/dos/bcinit.c` mode table `{0x12, 640, 400, 8, 16}`; `restart_header`
/// seeds `h_font_width=8, h_font_height=16`). 8 wide × 16 tall over a 640×400
/// screen gives the authentic **80 cols × 25 rows** that makes text read at the
/// period-screenshot size relative to the 2×-scaled 320×200 art (SQ-0479). v6
/// addresses everything in pixels; the app quantizes to character cells by
/// dividing X by WIDTH and Y by HEIGHT.
pub const V6_FONT_WIDTH: u16 = 8;
pub const V6_FONT_HEIGHT: u16 = 16;

/// [`V6Cell`] lives in its own module so its private fields are invisible to
/// the REST OF THIS FILE too, not merely to other crates — Rust scopes a
/// private field to the defining module and its children, and `screen.rs` is
/// four thousand lines of exactly the code most likely to write a new cell.
/// The whole workspace therefore has one `V6Cell` literal, inside
/// [`V6Cell::new`], and the zero-axis guard cannot be walked past (SQ-1031).
mod v6_cell {
    use super::{V6_FONT_HEIGHT, V6_FONT_WIDTH};

    /// The Version 6 character cell in native pixels, as a value rather than a pair
    /// of constants (SQ-0917).
    ///
    /// # Why this is state and not a constant
    ///
    /// The two constants above are one machine's cell, and for years they were read
    /// as every machine's. They are not. Infocom's Macintosh interpreter set
    /// `colWidth := 7` and `lineHeight := 15 {16}`; the Apple IIgs YZIP set
    /// `MFONT_W EQU 3` and `FONT_H EQU 9`. `v6_font_cell` in the app declined to
    /// express any of that three times — EGA (SQ-0790), Macintosh (SQ-0838), Apple
    /// IIgs (SQ-0857) — each time on the reasoning that nothing depended on it.
    ///
    /// Something does now. On the black-and-white Macintosh press the archive puts
    /// the story on a 480x300 screen; at 8 wide that is 60 columns, where the
    /// machine's own 7 gives 68. The story lays its hint banner out for the columns
    /// it is told it has, so eight columns of it have nowhere to go and the glyphs
    /// overrun the banner.
    ///
    /// # It is a DECLARED metric, not a drawn advance
    ///
    /// Worth stating here because it is the trap: `machine-screenshots/mac-zorkzero-hint.png`
    /// shows the Macintosh drawing **proportionally** — `WING` is the same glyph run
    /// at character index 5 in both `EAST WING` and `WEST WING` and starts at x=137
    /// in one and x=139 in the other, which no fixed pitch permits. Infocom's
    /// `stdFont := geneva` is a proportional System font and 7 is simply its average
    /// advance. So `colWidth := 7` is exactly the quantity header `$27` carries: what
    /// the story is TOLD its cell is, which is a different thing from how the
    /// interpreter chose to paint. Matching the declared metric is both the smaller
    /// change and the faithful one; matching the drawn face would mean proportional
    /// layout, which the Z-machine's own column arithmetic cannot express.
    ///
    /// # DECLARED, not DRAWN — the boundary that keeps this type honest
    ///
    /// This is what the STORY IS TOLD: header `$26`/`$27`, the character grid, window
    /// property 13, and every coordinate the interpreter computes by advancing its
    /// cursor. **It is fixed on every machine, including ones that painted
    /// proportionally** — the Macintosh declares `colWidth := 7` while drawing Geneva
    /// 12, and a host that declared anything else would be lying to the story.
    ///
    /// Where the ink actually goes is a different question, and one this type must not
    /// be asked. A proportional renderer — the Macintosh's own, or a future GUI —
    /// needs per-glyph advances, which the host supplies through
    /// [`V6Metric::proportional`] (SQ-1009). Both are true at once, and were, on real
    /// hardware: the pen is what the cursor advances by and what a printed run
    /// measures, the cell is still what the story was told.
    ///
    /// So: interpreting a coordinate the story produced is [`Self::row_of`],
    /// [`Self::col_of`], [`Self::run_px`]. Deciding where to put a pixel is
    /// [`V6Metric::advance`]'s business, not this type's.
    ///
    /// # Not a global
    ///
    /// The cell is per-session — resolved once at boot from the medium's profile and
    /// never changed — so it lives on [`crate::cpu::Machine`] and is threaded to the
    /// handful of places that quantize by it. It deliberately does NOT live on
    /// [`ScreenState`], which the host archives: the cell is derived from the
    /// profile, so a restore must re-derive it rather than replay a stored copy
    /// (CLAUDE.md, "persist the recipe, not the result"). And it is emphatically not
    /// process-global. The palette was, for two years, on the same "one machine per
    /// run" premise this type rejected; SQ-1393 moved it here beside the cell, so
    /// [`crate::cpu::Machine::palette`] is now the second fact of this shape rather
    /// than the counter-example.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct V6Cell {
        w: u16,
        h: u16,
    }

    impl V6Cell {
        /// The 8x16 cell described above — every machine's until a profile says
        /// otherwise, and the value a bare `Machine` (every unit test) carries.
        pub const DEFAULT: V6Cell = V6Cell::new(V6_FONT_WIDTH, V6_FONT_HEIGHT);

        /// Guard against a zero axis reaching the divisions below. A profile that
        /// stated `0` would otherwise panic somewhere far from the mistake — and
        /// in RELEASE as well as debug, because integer division by zero is not a
        /// debug-only overflow check.
        ///
        /// **This is the only `V6Cell` literal in the workspace**, which is what
        /// the module wrapper buys: the fields are private, so `V6Cell { w: 0, h: 0 }`
        /// is a compile error everywhere else — in other crates, and in the rest of
        /// this file. Until SQ-1031 the guard was a convention a public field walked
        /// straight past.
        ///
        /// `const` so that `DEFAULT` and `interpreter::MACINTOSH_V6_CELL` can be
        /// consts without a second literal; written out longhand because `Ord::max`
        /// is not a `const fn`.
        pub const fn new(w: u16, h: u16) -> Self {
            V6Cell { w: if w == 0 { 1 } else { w }, h: if h == 0 { 1 } else { h } }
        }

        /// The cell's width in native pixels. Never zero — see [`Self::new`].
        pub const fn w(self) -> u16 {
            self.w
        }

        /// The cell's height in native pixels. Never zero — see [`Self::new`].
        pub const fn h(self) -> u16 {
            self.h
        }

        /// The text ROW a 1-based native pixel Y falls in.
        ///
        /// ZMSD §8.8.1: v6 window and cursor coordinates are 1-based pixels, so the
        /// `- 1` is the origin correction and not an off-by-one. It lived at three
        /// dozen call sites before SQ-0917, hand-written every time, which is exactly
        /// how a site that forgot it read one row high without anything complaining.
        pub fn row_of(self, y_px: u16) -> u16 {
            (y_px.max(1) - 1) / self.h
        }

        /// The row a **zero-based** native pixel Y falls in.
        ///
        /// Two conventions are live in this codebase and they look identical written
        /// out longhand, which is why both are named here. A `PxText` run's `y` is
        /// ZMSD's 1-based pixel and wants [`Self::row_of`]; a `PositionedWindow`'s
        /// `y_px` was built as `t.y.saturating_sub(1)` and is already corrected, so
        /// passing it to `row_of` would subtract one twice and land a row high.
        ///
        /// Bare `y_px / cell.h` said nothing about which it held. This says it.
        pub fn row_of_origin0(self, y_px: u16) -> u16 {
            y_px / self.h
        }

        /// The COLUMN a 1-based native pixel X falls in. See [`Self::row_of`].
        pub fn col_of(self, x_px: u16) -> u16 {
            (x_px.max(1) - 1) / self.w
        }

        /// The pixel width the STORY believes `s` occupies.
        ///
        /// **Uniform on purpose, even for a machine that painted proportionally.**
        /// This answers what the story was told, and the interpreter's own cursor
        /// arithmetic advances by a fixed cell — so a proportional answer here would
        /// disagree with the coordinates the engine produced. See the type's own
        /// documentation for the declared-versus-drawn split.
        ///
        /// Takes the text rather than a count because every call site has the text in
        /// hand and was counting characters itself.
        pub fn run_px(self, s: &str) -> u32 {
            s.chars().count() as u32 * u32::from(self.w)
        }

        /// The native pixel rows a run whose 1-based top is `y_px` occupies.
        ///
        /// The EXTENT half of this type, and the half SQ-0917's sweep did not have.
        /// That quest named the three DIVISIONS — [`Self::row_of`], [`Self::col_of`],
        /// [`Self::run_px`] — and its own follow-up recorded what it was leaving
        /// behind: "a THRESHOLD compared against a cell dimension is not arithmetic
        /// and is not fixed by any of this". Those thresholds are written `py + 16`,
        /// which is a bare number no grep for a constant can see, and they went on
        /// meaning 16 after one machine stopped being 16.
        ///
        /// SQ-1020 is what that cost: a status bar sitting directly above the story
        /// satisfies `py + h == story_top`, so testing it with a hardcoded 16 fails by
        /// exactly one pixel on the Macintosh's 15-tall cell — and the bar drops out
        /// of the text band and rasterises, on one machine, silently.
        pub fn rows_px(self, y_px: u16) -> core::ops::Range<u32> {
            let top = u32::from(y_px.max(1) - 1);
            top..top + u32::from(self.h)
        }

        /// The native pixel row just PAST a run whose 1-based top is `y_px`.
        ///
        /// [`Self::rows_px`]'s end, defined from it so the two cannot drift — which is
        /// the failure this whole area keeps having.
        pub fn bottom_px(self, y_px: u16) -> u32 {
            self.rows_px(y_px).end
        }
    }

    impl Default for V6Cell {
        fn default() -> Self {
            V6Cell::DEFAULT
        }
    }
}

pub use v6_cell::V6Cell;

/// ZMSD §8.7.1 style bit 0 — reverse video. Named here because a blank cell's
/// only content is its ground, and reverse is half of what a ground IS (SQ-1054).
const STYLE_REVERSE: u8 = 1;

/// ZMSD §8.7.1 style bit 1 — bold. Named here because the pen has to know:
/// the Amiga emboldens by smearing a glyph right and advancing by the same
/// amount, so a bold run is genuinely WIDER than the same letters in roman.
const STYLE_BOLD: u8 = 2;

/// ZMSD §8.7.1 style bit 3 — fixed pitch.
///
/// A run carries it two ways and they mean one thing: the story asked for it with
/// `@set_text_style 8`, or it selected **font 4**, which `exec.rs` folds in here so
/// that everything downstream has a single question to ask. On a machine drawing a
/// proportional body face this is not decoration — it is the difference between
/// Geneva and Monaco, and *Zork Zero*'s Macintosh press brackets its whole status
/// bar in `@set_font 4` / `@set_font 1` (measured on
/// `machine-screenshots/mac-zorkzero-game.png`, where `Banquet Hall` steps a
/// uniform 7 px per character while the prose two lines below advances 7, 7, 5).
pub const STYLE_FIXED_PITCH: u8 = 8;

/// What the story is TOLD about its cell, together with the pen the machine
/// actually drew with (SQ-1009).
///
/// # Why these are one value
///
/// They are the same subject measured two ways, and every place that quantizes
/// text needs both: [`V6Cell`] is the DECLARED metric — header `$26`/`$27`, the
/// grid a window divides into, what `@get_wind_prop 13` reports — while the pen
/// is what the interpreter's own cursor arithmetic advanced by. On every machine
/// but one they are the same number and this type is a cell in a wrapper. On
/// Arthur's Amiga floppy they are not: the release ships a proportional
/// `char.data` whose glyphs advance 2..=8 face pixels (4..=16 native, the art
/// scale doubling them) against a declared width of 8.
///
/// CLAUDE.md's refactoring policy is the reason they travel together rather than
/// as `(cell, pen)` at each of the dozen call sites: a caller who supplies one
/// and not the other gets a plausible answer rather than an error, which is the
/// exact shape of SQ-0901/SQ-1020/SQ-1021/SQ-1022.
///
/// # Declared still wins where the STORY is doing the arithmetic
///
/// [`Self::cell`] is not deprecated by [`Self::advance`]. A window's character
/// grid, the row a pixel falls in, `more_interval` — all of those are the
/// story's own units and must stay on the declared cell, because the story laid
/// its windows out from it. The pen governs only where the NEXT glyph goes and
/// how wide a printed run came out, which is what the machine measured and what
/// header `$30` reports back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V6Metric {
    cell: V6Cell,
    /// Native pixels of advance per ZSCII byte, or `None` for a fixed pen —
    /// which is every machine whose release did not ship a proportional face,
    /// and every configuration that existed before SQ-1009.
    advances: Option<Box<[u16; 256]>>,
    /// Native pixels a **bold** glyph adds to its own advance, so the smeared
    /// column has somewhere to live. Zero for a fixed pen.
    bold_extra: u16,
    /// The advance of one character in a §8.7.1 [`STYLE_FIXED_PITCH`] run, where
    /// the machine has a fixed-pitch face to draw such a run WITH (SQ-1036).
    ///
    /// `None` — every machine before this, and every machine today whose media
    /// carry no fixed alternate — means the fixed-pitch bit does not move the pen,
    /// which is what it has always meant. It is not a licence to invent a second
    /// pitch: a host sets this only when it has admitted a face that *is* the
    /// declared cell, so the number is the cell's width by construction rather
    /// than by choice.
    fixed_pitch: Option<u16>,
}

impl V6Metric {
    /// A machine that advances by its declared cell — every one but Arthur's
    /// Amiga press, and every path that existed before SQ-1009.
    pub fn fixed(cell: V6Cell) -> V6Metric {
        V6Metric { cell, advances: None, bold_extra: 0, fixed_pitch: None }
    }

    /// A machine drawing a proportional face: `advances[b]` is the native pixel
    /// advance of ZSCII byte `b`, and `bold_extra` what bold adds to each.
    ///
    /// The host builds the table, because the face lives on the release's own
    /// medium and `zvm` takes no dependencies. Every byte must carry a usable
    /// number — a glyph the face does not cover is the caller's to fill with the
    /// cell width, so that this type never has to guess.
    pub fn proportional(cell: V6Cell, advances: Box<[u16; 256]>, bold_extra: u16) -> V6Metric {
        V6Metric { cell, advances: Some(advances), bold_extra, fixed_pitch: None }
    }

    /// State that a §8.7.1 [`STYLE_FIXED_PITCH`] run advances by the DECLARED
    /// cell, because the machine has a fixed-pitch face that is that cell.
    ///
    /// Only meaningful on a proportional pen — a fixed one already answers the
    /// cell for everything — and it takes no width, because a face admitted as
    /// the machine's fixed alternate has already been tested against the cell and
    /// there is no other number it could be. Pairing the two here is what stops a
    /// caller inventing a pitch to go with a face it never checked.
    pub fn with_fixed_alternate(mut self) -> V6Metric {
        self.fixed_pitch = Some(self.cell.w());
        self
    }

    /// The DECLARED cell — what the story was told.
    pub fn cell(&self) -> V6Cell {
        self.cell
    }

    /// Whether the pen varies per glyph.
    pub fn is_proportional(&self) -> bool {
        self.advances.is_some()
    }

    /// Native pixels the pen moves for one character printed in §8.7.1 `style`.
    ///
    /// A fixed pen answers the cell width for everything, which is what every
    /// machine but one does and what this crate did before SQ-1009.
    pub fn advance(&self, ch: char, style: u8) -> u16 {
        let Some(advances) = self.advances.as_ref() else { return self.cell.w() };
        // A fixed-pitch run is drawn with the machine's fixed ALTERNATE, so it
        // advances by that face's pitch rather than the body face's — the whole
        // reason *Zork Zero*'s Macintosh status bar lines its columns up
        // (SQ-1036). Bold still widens it: the smear needs a column on any face.
        let base = match self.fixed_pitch {
            Some(w) if style & STYLE_FIXED_PITCH != 0 => w,
            _ => {
                let Ok(b) = u8::try_from(u32::from(ch)) else { return self.cell.w() };
                advances[usize::from(b)]
            }
        };
        let extra = if style & STYLE_BOLD != 0 { self.bold_extra } else { 0 };
        base.saturating_add(extra).max(1)
    }

    /// Native pixels a whole run occupies at this pen — the width that WRAPS,
    /// and the width header `$30` reports when a game measures through stream 3.
    pub fn run_px(&self, s: &str, style: u8) -> u32 {
        if self.advances.is_none() {
            return self.cell.run_px(s);
        }
        s.chars().map(|c| u32::from(self.advance(c, style))).sum()
    }
}

impl Default for V6Metric {
    fn default() -> Self {
        V6Metric::fixed(V6Cell::DEFAULT)
    }
}

/// One line break [`wrap_text`] introduced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrapBreak {
    /// Index, in CHARACTERS of the printed string, of the first character on the
    /// new line.
    pub at: usize,
    /// Whether the break CONSUMED the space at `at`. Frotz buffers a word with
    /// its leading blank and drops that blank at the break (`screen_word`), so a
    /// wrapped line never begins with the space that caused it — and that
    /// character is drawn on neither line.
    pub consumed: bool,
}

/// Where `s` breaks — measured in PIXELS and in COLUMNS at once, as character
/// indices into the printed string.
///
/// # Why one pass and not two
///
/// A v6 print has two measures of the same text. The PIXEL measure is the game's
/// truth: it is what `@get_cursor` reports, what header `$30` records and what the
/// raster backend draws. The COLUMN measure is what a terminal backend can
/// actually place, one glyph per cell. On every machine whose pen IS the declared
/// cell the two are the same statement twice; on Arthur's Amiga press, where the
/// pen advances the face's own 3-to-11 pixels against a declared 8, they are not.
///
/// Running them as two independent passes and correlating the answers afterwards
/// does not work, and the reason is worth stating because it looks like it should
/// (SQ-1009). Each pass assumes it is the only one breaking, so its indices are
/// only valid until the other one breaks first — and they disagree about which
/// blank a soft break swallows, so the two line-sets are not even the same
/// character sequence. The observable result on Arthur's F5 description: the pixel
/// measure fills the 584px window before the column measure fills its 73 columns,
/// so every wrapped line ends with a word painted on the next row while still
/// tagged with the previous row's cell, at columns 68, 64, 55 that run off the
/// window — and `churchyard`'s `d` is overwritten by the `.` that follows it,
/// because one measure dropped a blank the other kept.
///
/// So there is ONE pass, one break list, and a break moves both pens. A line ends
/// at whichever limit is reached first, which on a proportional face is nearly
/// always the pixel one — so hybrid wraps where the machine wrapped, and its lines
/// are honestly shorter than the window rather than filled to a column count the
/// game never used. A run's grid cell and its pixel origin then agree character
/// for character, which is the invariant a cell backend needs and could not get.
///
/// * `at` — the 1-based `(pixel, column)` position of the first character.
/// * `margin` — the 1-based `(pixel, column)` a fresh line begins at
///   (`left_margin + 1`, and its column).
/// * `limit` — the last usable `(pixel, column)` on a line, inclusive.
///   [`u32::MAX`] in both for a window with wrapping off, which never breaks.
/// * `word_wrap` — ZMSD §8.8.3.1.2.2, wrapping AND buffered printing.
/// * `advance` — one character's `(pixel, column)` cost. The column term is 1 for
///   every glyph a grid holds; it is a parameter so the pair stays one value.
///
/// Hard newlines are NOT reported: they are in the string, both measures honour
/// them identically, and the caller sees them itself. They are still obeyed here,
/// because a new line resets the positions every later break depends on.
///
/// The word-wrap probe fires at a space, and once at the very first character:
/// the buffer spans one print call, exactly as it did before this was extracted.
pub fn wrap_text(
    s: &str,
    at: (u32, u32),
    margin: (u32, u32),
    limit: (u32, u32),
    word_wrap: bool,
    advance: &mut dyn FnMut(char) -> (u32, u32),
) -> Vec<WrapBreak> {
    let chars: Vec<char> = s.chars().collect();
    let mut breaks = Vec::new();
    let (mut x, mut col) = at;
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        i += 1;
        if ch == '\n' {
            (x, col) = margin;
            continue;
        }
        // Measure the word about to start and break ahead of it if it cannot fit
        // under EITHER measure. Frotz buffers a word together with its leading
        // space and drops that space at the break (`screen_word`), so a wrapped
        // line does not begin with a stray blank. A word longer than a whole line
        // is left to the character wrap below.
        if word_wrap {
            let at_space = ch == ' ';
            if at_space || i == 1 {
                let start = if at_space { i } else { i - 1 };
                let (mut word, mut word_cols) = (0u32, 0u32);
                for c in chars[start..].iter().take_while(|c| **c != ' ' && **c != '\n') {
                    let (a, ac) = advance(*c);
                    word += a;
                    word_cols += ac;
                }
                let (sp, sp_cols) = if at_space { advance(' ') } else { (0, 0) };
                let need = word + sp;
                let need_cols = word_cols + sp_cols;
                // Each measure judges itself against its own limit, and either one
                // that cannot fit the word ends the line for both.
                let px_full =
                    word > 0 && x > margin.0 && word <= limit.0 && x.saturating_add(need).saturating_sub(1) > limit.0;
                let col_full = word_cols > 0
                    && col > margin.1
                    && word_cols <= limit.1
                    && col.saturating_add(need_cols).saturating_sub(1) > limit.1;
                if px_full || col_full {
                    // `i - 1` is this character's own index. At a space the break
                    // consumes it and the new line starts on the word behind it;
                    // at the call's first character nothing is consumed.
                    breaks.push(WrapBreak { at: i - 1, consumed: at_space });
                    (x, col) = margin;
                    if at_space {
                        continue; // the break consumes the space
                    }
                }
            }
        }
        let (a, ac) = advance(ch);
        x = x.saturating_add(a);
        col = col.saturating_add(ac);
        // The character wrap is a POST-check and deliberately so: a glyph that
        // ends exactly ON the limit is drawn on this line, and the break happens
        // after it. Checked ahead of the glyph instead, a run that exactly fills
        // its line would leave the cursor at the end of that line rather than at
        // the start of the next — which a story reads back through `@get_cursor`.
        if x > limit.0 || col > limit.1 {
            breaks.push(WrapBreak { at: i, consumed: false });
            (x, col) = margin;
        }
    }
    breaks
}

/// Upper bound on any character-grid dimension a story operand can request
/// (`split_window`, EXT `window_size`). A hostile/buggy story passing 0xFFFF
/// would otherwise force a rows×cols cell allocation in the hundreds of
/// megabytes — an OOM abort, where the VM promises graceful faults. 1024
/// far exceeds any real terminal (a 4K screen at 8 px/cell is ~480×270
/// cells) yet caps worst-case storage at ~1M cells per window.
pub const GRID_CELL_CAP: u16 = 1024;

/// Upper bound, in pixels, on every window coordinate a STORY can write:
/// properties 0-7 of ZMSD 1.1 §8.8.3.2 (`y coordinate`, `x coordinate`, `y size`,
/// `x size`, `y cursor`, `x cursor`, `left margin size`, `right margin size`),
/// whether they arrive through `put_wind_prop`, `move_window`, `window_size`,
/// `set_cursor` or `set_margins`.
///
/// # Why a cap at all
///
/// The print path combines these fields with plain `+` — `x_coord + x_cursor - 1`,
/// then `+ font_width - 1`, and `x_cursor = left_margin + 1` on every new-line.
/// Stored verbatim, a story writing `0xFFFF` into one of them aborts a debug-built
/// host with "attempt to add with overflow" four instructions into `main`, and
/// wraps silently in a release one. A library cannot choose its embedder's
/// profile, so neither outcome is acceptable (SQ-1030).
///
/// # Why 8192 and not `u16::MAX`
///
/// Saturating at `u16::MAX` would trade a panic for a window whose geometry is
/// absurd but still self-consistent, which is the harder bug to see. This is the
/// same ceiling [`GRID_CELL_CAP`] already states, in the other unit: 1024 cells at
/// the 8-pixel cell this crate defaults to. The tallest, widest v6 screen anyone
/// shipped is 640x400, so no real story comes within an order of magnitude of it —
/// and two capped values plus a font cell stay far inside `u16`, which is exactly
/// what the additions above need.
pub const WINDOW_PX_CAP: u16 = 8192;

/// Cap on [`ZWindow::prose`] (SQ-0585). A secondary prose window shows what is on
/// screen and nothing more — the tallest v6 screen is 400px, 25 text rows, and a
/// game that prints past its window's bottom without erasing has scrolled the
/// earlier lines off. Twice the tallest screen leaves room for that overshoot while
/// keeping a runaway printer from growing the buffer without bound.
pub const PROSE_MAX_LINES: usize = 50;

/// Cap on [`ZWindow::streamed`] (SQ-0697). The scroll in
/// [`ZWindow::prose_new_line`] already bounds the shadow to a screenful for any
/// window that stays put; this is the backstop for one that never scrolls. A run
/// per glyph on the tallest v6 screen (25 rows of 80 cells) is 2000, so four
/// times that is far past anything real while still bounded.
pub const STREAMED_MAX_RUNS: usize = 8000;

/// Write the screen-dimension header fields for the loaded story's version.
///
/// v4+: byte 0x20 = height in lines, byte 0x21 = width in chars (ZMSD §11.1).
/// v5+: also word 0x22 = width in units, word 0x24 = height in units, and font
/// size bytes 0x26/0x27 = 1 (one unit per char cell, since we render a fixed
/// character grid). `rows`/`cols` of 0 are clamped to 1 to avoid a zero size.
pub fn write_screen_dims(mem: &mut Memory, rows: u8, cols: u8, cell: V6Cell) {
    let version = mem.version();
    if version < 4 {
        return; // v1-3 have no settable screen-size header fields.
    }
    let rows = rows.max(1);
    let cols = cols.max(1);
    if version == 6 {
        // A character grid is not a v6 screen, so recover the pixels this grid
        // stands for and go through the pixel path — see `write_screen_dims_px`.
        write_screen_dims_px(mem, cols as u16 * cell.w(), rows as u16 * cell.h(), cell);
        return;
    }
    mem.write_byte(0x20, rows);
    mem.write_byte(0x21, cols);
    if version >= 5 {
        mem.write_word(0x22, cols as u16); // screen width in units
        mem.write_word(0x24, rows as u16); // screen height in units
        mem.write_byte(0x26, 1); // font width in units
        mem.write_byte(0x27, 1); // font height in units
    }
}

/// The Version 6 screen, stated the way Version 6 means it: **in pixels, with the
/// character grid derived** (SQ-0917).
///
/// # Why the direction matters
///
/// [`write_screen_dims`] takes a character grid and multiplies back up, which is
/// exact only when the cell divides the screen. It always did while the cell was
/// 8x16 and every v6 screen was a multiple of it. It stops being exact the moment
/// a profile declares its own: the black-and-white Macintosh window is 480 px
/// wide, `480 / 7` is 68 columns, and `68 * 7` is **476** — four pixels the story
/// would never be told about, on the axis its hint banner is laid out along.
///
/// Infocom's own Macintosh interpreter states the direction outright, in
/// `mac/xzip.lst`:
///
/// ```text
///   totRows := (bottom - top) {DIV lineheight};
///   totCols := ((right - left) - (2 * wMarg)) {DIV colWidth};
/// ```
///
/// The window is the truth and the grid is a quotient of it. So `$22`/`$24` carry
/// the pixels verbatim and `$20`/`$21` carry what fits, which is what a story
/// reading either one expects to find.
///
/// # The grid TRUNCATES where the pixels round
///
/// `$20`/`$21` answer "how many whole characters fit", so a partial cell at the
/// edge is not one — `480 / 7 = 68`, not 69. That is the opposite of the caller's
/// rounding when it turns an art canvas into a screen (`session.rs` rounds a
/// 300-pixel plate to the NEAREST whole cell so a game is not told its own
/// artwork is clipped), and both are right: one is asking what the screen IS, the
/// other what fits ON it.
pub fn write_screen_dims_px(mem: &mut Memory, width_px: u16, height_px: u16, cell: V6Cell) {
    if mem.version() != 6 {
        return;
    }
    let (w, h) = (width_px.max(1), height_px.max(1));
    // Whole characters only, and at least one: a screen narrower than a cell is
    // degenerate, but a zero in $20/$21 makes a size-sensitive story abort.
    let cols = (w / cell.w().max(1)).clamp(1, 255) as u8;
    let rows = (h / cell.h().max(1)).clamp(1, 255) as u8;
    mem.write_byte(0x20, rows);
    mem.write_byte(0x21, cols);
    // ZMSD §8.4.3: word $22 = screen width in units, word $24 = screen height in
    // units, and v6 units are pixels — so these are the window itself, not the
    // grid multiplied back up.
    mem.write_word(0x22, w);
    mem.write_word(0x24, h);
    // ZMSD §11.1 header table (verified against the spec): byte $26 = "Font width
    // in V5, or font HEIGHT in V6"; byte $27 = "Font height in V5, or font WIDTH
    // in V6" — the famous V5<->V6 swap (§8.1.1: "in Version 6 the width and
    // height are stored the other way round"). So in V6: $26 = HEIGHT, $27 =
    // WIDTH. Latent while the cell was square; load-bearing since SQ-0479, and
    // per-machine since SQ-0917.
    mem.write_byte(0x26, cell.h() as u8);
    mem.write_byte(0x27, cell.w() as u8);
}

// ---------------------------------------------------------------------------
// Status-line computation (v3)
// ---------------------------------------------------------------------------

/// Compute the current v3 status line from memory globals and header.
///
/// G0 (global var 0) = location object number.
/// G1 = score (signed) or hours (unsigned).
/// G2 = turns or minutes.
/// Flags1 bit 1: 0 = score/turns, 1 = time.
/// Does this story keep a clock rather than a score on the status line?
///
/// ZMSD §8.2.1: "In Versions 1 and 2, all games are 'score games'. In Version 3,
/// if bit 1 of 'Flags 1' is clear then the game is a 'score game'; if it is set,
/// then the game is a 'time game'." Flags 1 bit 1 only carries that meaning from
/// v3 on, so it must not be consulted below it. (Belt and braces today: the
/// header parser refuses to load a v1/v2 story at all — see
/// [`crate::header::parse_header`] — so the guard only matters if that ever
/// changes.)
fn is_time_game(version: u8, flags1: u8) -> bool {
    version >= 3 && (flags1 & (1 << 1)) != 0
}

pub fn compute_status_line(mem: &Memory) -> StatusLine {
    let gbase = mem.global_vars() as u32;
    let loc_obj = mem.read_word(gbase);
    let g1 = mem.read_word(gbase + 2);
    let g2 = mem.read_word(gbase + 4);

    let location = if loc_obj == 0 {
        String::new()
    } else {
        objects::short_name(mem, loc_obj)
    };

    let time_mode = is_time_game(mem.version(), mem.read_byte(0x01));

    let right = if time_mode {
        StatusRight::Time { hours: g1 as u8, minutes: g2 as u8 }
    } else {
        StatusRight::ScoreTurns { score: g1 as i16, turns: g2 }
    };

    StatusLine { location, right }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {

    /// A run at `(x, y)` in `style`, with inherited colours and its declared grid
    /// cell — what a game's own `print` deposits.
    fn run_at(x: u16, y: u16, text: &str, style: u8) -> V6Text {
        V6Text {
            y,
            x,
            text: text.to_string(),
            style,
            fg: ZColour::Default,
            bg: ZColour::Default,
            grow: (y.max(1) - 1) / 15,
            gcol: (x.max(1) - 1) / 7,
        }
    }

    /// **A space erases a BLANK cell under it, and still spares a letter**
    /// (SQ-1054).
    ///
    /// A space printed with inherited colours deposits no pixels, so `paint_run`
    /// has always let one pass over whatever is beneath. That is right when a
    /// letter is beneath — Shogun pads its status fields with spaces whose span
    /// reaches a neighbouring label painted earlier in the same row, and a text-run
    /// model cannot overstrike. It is wrong when the thing beneath is a cell whose
    /// only content IS a background: two spaces cannot both own one pixel span.
    ///
    /// Macintosh Zork Zero's InvisiClues menu is the report. It highlights a topic
    /// in reverse video and deselects it by re-printing the same characters in
    /// normal video; the letters overwrote their own cells and the INTER-WORD
    /// spaces did not, so the old highlight's reversed blocks outlived it. Measured
    /// on `stories/Zork Zero Disk.image` with Geneva 12: after two `n` presses the
    /// deselected row still carried reversed spaces at native x=132 and x=167.
    ///
    /// FALSIFY by routing the transparent segment of `paint_run` to nothing again
    /// (drop the `erase_blank_cells_in_rect` arm): the reversed blank survives the
    /// second print and the first assertion fails.
    /// SQ-1031. The zero-axis clamp is the whole reason `V6Cell::new` exists, and
    /// until the fields went private a struct literal walked straight past it into
    /// `row_of`'s `/ self.h` — which panics in RELEASE as well as debug, because
    /// integer division by zero is not a debug-only overflow check.
    ///
    /// FALSIFY by restoring `w`/`h` to `pub`: the literal below stops being a
    /// compile error, and `V6Cell { w: 0, h: 0 }.row_of(1)` panics.
    #[test]
    fn a_zero_axis_cannot_reach_the_divisions() {
        // `new` is the only way in, and it clamps both axes.
        assert_eq!(V6Cell::new(0, 0), V6Cell::new(1, 1));
        assert_eq!(V6Cell::new(0, 15).w(), 1);
        assert_eq!(V6Cell::new(7, 0).h(), 1);
        // A non-zero axis is passed through untouched — the clamp is a floor, not
        // a substitution.
        let mac = V6Cell::new(7, 15);
        assert_eq!((mac.w(), mac.h()), (7, 15));
        // And the divisions the guard exists for terminate on the clamped cell.
        let zero = V6Cell::new(0, 0);
        assert_eq!(zero.row_of(1), 0);
        assert_eq!(zero.col_of(1), 0);
        assert_eq!(zero.row_of_origin0(9), 9);
    }

    #[test]
    fn a_printed_space_clears_a_blank_cell_but_not_a_letter() {
        let metric = V6Metric::fixed(V6Cell::new(7, 15));
        // The highlight: a reversed space sitting alone at x=8 (native 7..14).
        let mut w = V6Windows::default();
        w.paint_run(0, run_at(8, 1, " ", 1), &metric);
        assert_eq!(w.windows[0].texts.len(), 1, "the highlight is on the screen");

        // Deselecting prints `A A` over it — the letters land either side and the
        // SPACE lands exactly on the reversed blank.
        w.paint_run(0, run_at(1, 1, "A A", 0), &metric);
        let left: Vec<&V6Text> = w.windows[0].texts.iter().collect();
        assert!(
            !left.iter().any(|t| t.style & 1 != 0),
            "the reversed blank the space covered is gone: {:?}",
            left.iter().map(|t| (t.x, t.style, &t.text)).collect::<Vec<_>>(),
        );

        // …and the same space does NOT eat a LETTER, which is Shogun's padding.
        let mut w2 = V6Windows::default();
        w2.paint_run(0, run_at(8, 1, "L", 0), &metric);
        w2.paint_run(0, run_at(1, 1, "A A", 0), &metric);
        assert!(
            w2.windows[0].texts.iter().any(|t| t.text.contains('L')),
            "a padding space still spares a neighbouring label: {:?}",
            w2.windows[0].texts.iter().map(|t| (t.x, &t.text)).collect::<Vec<_>>(),
        );

        // …and it spares a blank on its OWN ground, which is advent.z6's help bar:
        // reversed spacers painted first, reversed labels over them, and the
        // spacers ARE the bar. Same pixels either way, so the record stands.
        // `v6_advent_help_bar` failed on exactly this when the erase was
        // unconditional — the third frame, and the one that fixes the rule's shape.
        let mut w3 = V6Windows::default();
        w3.paint_run(0, run_at(8, 1, " ", 1), &metric);
        w3.paint_run(0, run_at(1, 1, "A A", 1), &metric);
        assert!(
            w3.windows[0].texts.iter().any(|t| t.x == 8 && t.text == " "),
            "a reversed spacer under a REVERSED label survives — it is the bar: {:?}",
            w3.windows[0].texts.iter().map(|t| (t.x, t.style, &t.text)).collect::<Vec<_>>(),
        );
    }

    // ── SQ-1030: the one clamp ───────────────────────────────────────────────

    /// `put_prop` is the crate's only writer of the eight geometry properties
    /// from a story operand, so the clamp belongs to it and this case pins it.
    /// Properties 8-15 are not pixels and are stored verbatim — property 15 in
    /// particular is a SIGNED line count whose own floor is -999, and clamping it
    /// to a positive pixel ceiling would silently disable "[MORE]".
    #[test]
    fn put_prop_clamps_the_geometry_and_leaves_the_rest_alone() {
        let mut w = super::ZWindow::default();
        for n in 0..=7u16 {
            w.put_prop(n, 0xFFFF);
            assert_eq!(w.get_prop(n), super::WINDOW_PX_CAP, "property {n} is a pixel and clamps");
            w.put_prop(n, 300);
            assert_eq!(w.get_prop(n), 300, "property {n} under the cap is untouched");
        }
        for n in 8..=15u16 {
            w.put_prop(n, 0xFFFF);
            assert_eq!(w.get_prop(n), 0xFFFF, "property {n} is not a pixel and is verbatim");
        }
        // §8.8.3.2 ends "The true foreground and true background properties must
        // not be written by put_wind_prop" — 16/17 are still ignored.
        w.put_prop(16, 1);
        w.put_prop(17, 1);
        assert_eq!(w.get_prop(16), 0);
        assert_eq!(w.get_prop(17), 0);
    }

    // ── SQ-0956: the two-colour card ─────────────────────────────────────────
    //
    // Every case here names its table by VALUE (SQ-1393). There is no process-wide
    // palette left to set, so there is nothing to hold, nothing to restore, and no
    // way for one case to decide what another resolves.

    /// The card's table is XZIP's — one entry from YZIP's, and that entry is the
    /// one the capture measures.
    #[test]
    fn the_cga_card_resolves_white_to_the_cards_light_grey() {
        assert_eq!(true_colour_in(Palette::IbmCga, 9), Some(0x56B5), "white 9 is EGA entry 7, #AAAAAA");
        assert_eq!(true_colour_in(Palette::IbmCga, 2), Some(0x0000), "and black 2 is black");
        assert_eq!(
            true_colour_in(Palette::IbmYzip, 9),
            Some(0x7FFF),
            "…where the same machine's EGA is #FFFFFF",
        );
    }

    /// Only the card is a two-state display, and a `Machine` carries whichever
    /// table it was told (SQ-1393).
    #[test]
    fn only_the_cga_card_is_a_two_state_display() {
        let mut m = crate::cpu::exec::Machine::new(Memory::new(sample_story(5)).expect("story"));
        assert_eq!(m.palette(), Palette::Standard, "a fresh machine resolves through §8.3.1");
        for p in [Palette::Standard, Palette::Amiga, Palette::IbmXzip, Palette::IbmYzip, Palette::IbmCga] {
            m.set_palette(p);
            assert_eq!(m.palette(), p, "{p:?} survives the round trip");
            assert_eq!(p.two_colour_card(), p == Palette::IbmCga, "{p:?}");
        }
        assert_eq!(CGA_CARD_PAIR, (9, 2), "white ink over a black page");
    }

    // ── SQ-1354: bold is the EGA intensity bit ───────────────────────────────

    /// Every colour number the IBM tables answer for, bold, on BOTH interpreters
    /// — the eight attributes `Zip_to_ega`/`zip_to_ibm_color` reach.
    ///
    /// The pairs are `attr` → `attr | 8` read off the EGA/CGA text palette; see
    /// [`ega_intense`] for the `curattr | 8` that produces them.
    #[test]
    fn bold_lights_every_ibm_colour_on_both_tables() {
        // (colour number, plain, lit) — identical for XZIP and YZIP except white.
        let common: [(u8, u16, u16); 7] = [
            (2, 0x0000, 0x294A), // black   EGA 0  → 8  dark grey
            (3, 0x0015, 0x295F), // red     EGA 4  → 12 light red
            (4, 0x02A0, 0x2BEA), // green   EGA 2  → 10 light green
            (5, 0x2BFF, 0x2BFF), // yellow  EGA 14 is already lit
            (6, 0x5400, 0x7D4A), // blue    EGA 1  → 9  light blue
            (7, 0x5415, 0x7D5F), // magenta EGA 5  → 13 light magenta
            (8, 0x56A0, 0x7FEA), // cyan    EGA 3  → 11 light cyan
        ];
        for yzip in [false, true] {
            for (n, plain, lit) in common {
                assert_eq!(ega_true_colour_styled(n, yzip, false), Some(plain), "{n} plain (yzip={yzip})");
                assert_eq!(ega_true_colour_styled(n, yzip, true), Some(lit), "{n} bold (yzip={yzip})");
            }
        }
        // White is the one entry the two tables disagree on, and bold closes the
        // gap: XZIP's EGA 7 lights to 15, which is where YZIP already was.
        assert_eq!(ega_true_colour_styled(9, false, false), Some(0x56B5), "xzip white: EGA 7, #AAAAAA");
        assert_eq!(ega_true_colour_styled(9, false, true), Some(0x7FFF), "…lit: EGA 15, #FFFFFF");
        assert_eq!(ega_true_colour_styled(9, true, false), Some(0x7FFF), "yzip white is already EGA 15");
        assert_eq!(ega_true_colour_styled(9, true, true), Some(0x7FFF), "…and has no second bit to set");
        // The sentinels and the greys answer nothing, bold or not.
        for n in [0u8, 1, 10, 11, 12, 13, 14, 15] {
            assert_eq!(ega_true_colour_styled(n, false, true), None, "{n} is not in the IBM table");
        }
    }

    /// Brown is the one EGA base colour no Z-machine number reaches, and
    /// [`ega_intense`] still has to answer for it: the table is the DISPLAY's, and
    /// the machine's own default ink resolves through it too.
    #[test]
    fn the_intensity_table_covers_the_display_not_just_the_colour_numbers() {
        assert_eq!(ega_intense(0x0155), 0x2BFF, "6 brown #AA5500 → 14 yellow #FFFF55");
        // Already-lit entries and non-EGA values are returned untouched — there is
        // no second intensity bit.
        for lit in [0x294A, 0x7D4A, 0x2BEA, 0x7FEA, 0x295F, 0x7D5F, 0x2BFF, 0x7FFF] {
            assert_eq!(ega_intense(lit), lit, "{lit:#06X} is already intense");
        }
        assert_eq!(ega_intense(0x39CE), 0x39CE, "the Amiga's medium grey is not an EGA colour");
    }

    /// Which displays apply it — and, just as much, which do not.
    #[test]
    fn only_the_xzip_display_lights_bold() {
        for p in [Palette::Standard, Palette::Amiga, Palette::IbmXzip, Palette::IbmYzip, Palette::IbmCga] {
            assert_eq!(
                p.bold_lights_the_intensity_bit(),
                p == Palette::IbmXzip,
                "{p:?}: only the v1–v5 DOS text display ORs bit 3 for HLIGHT",
            );
        }
    }

    /// **The rule, both ways round.** A pair carries one bit for a two-state
    /// display: which channel wants the lit state. Zork Zero's boot pair asks for
    /// dark ink on a light page and the card shows the opposite polarity —
    /// `machine-screenshots/dos-zorkzero-cga.png`, black page under light ink —
    /// and the pair its own `color` menu offers gives that page back.
    #[test]
    fn a_two_colour_card_takes_one_bit_from_a_pair() {
        let card = Palette::IbmCga;
        let std = |n: u8| Some(ZColour::Standard(n));
        assert_eq!(
            two_colour_card_request(card, std(2), std(9)),
            (std(9), std(2)),
            "black ink on a white page is the card's own polarity: light ink, black page",
        );
        assert_eq!(
            two_colour_card_request(card, std(9), std(2)),
            (std(2), std(9)),
            "…and the swap the game's `color` menu offers is the other side of the bit",
        );
        // A channel that names no colour cannot carry a bit.
        assert_eq!(two_colour_card_request(card, std(2), None), (std(2), None), "one channel kept");
        assert_eq!(
            two_colour_card_request(card, std(2), Some(ZColour::Default)),
            (std(2), Some(ZColour::Default)),
            "the -1 carve-out names no colour either",
        );
        assert_eq!(two_colour_card_request(card, std(9), std(9)), (std(9), std(9)), "one colour twice");

        // …and on every other display the request is what it says it is.
        for p in [Palette::Standard, Palette::Amiga, Palette::IbmXzip, Palette::IbmYzip] {
            assert_eq!(
                two_colour_card_request(p, std(2), std(9)),
                (std(2), std(9)),
                "{p:?}: a screen with colours takes the pair as named",
            );
        }
    }
    use super::*;
    use crate::header::tests_support::sample_story;
    use crate::memory::Memory;
    use crate::text::encode::encode_word;

    /// SQ-0804: `stream_origin` is where the FIRST glyph of a burst landed — set
    /// once and then left alone until it is cleared, so the host can compare it
    /// against the cursor the window had before the burst.
    /// A fixed-pitch run advances by the DECLARED cell, once the host has paired
    /// it with a face that is that cell (SQ-1036).
    ///
    /// The pairing is the rule: `with_fixed_alternate` takes no width, because a
    /// face admitted as the machine's fixed alternate has already been tested
    /// against the cell and there is no other number it could be. Without the
    /// pairing the bit is the no-op it has always been.
    #[test]
    fn a_fixed_pitch_run_advances_by_the_cell_only_where_an_alternate_exists() {
        let cell = V6Cell::new(7, 15);
        let mut advances = Box::new([9u16; 256]);
        advances[usize::from(b'i')] = 3;
        let bare = V6Metric::proportional(cell, advances.clone(), 1);
        let paired = V6Metric::proportional(cell, advances, 1).with_fixed_alternate();

        // Roman: both pens are the body face's, and both are proportional.
        for m in [&bare, &paired] {
            assert_eq!(m.advance('i', 0), 3);
            assert_eq!(m.advance('W', 0), 9);
        }
        // Fixed pitch: the paired pen answers the cell for everything, and the bare
        // one goes on answering the body face's advances.
        assert_eq!(paired.advance('i', STYLE_FIXED_PITCH), 7, "the declared cell");
        assert_eq!(paired.advance('W', STYLE_FIXED_PITCH), 7, "for every character alike");
        assert_eq!(bare.advance('i', STYLE_FIXED_PITCH), 3, "no alternate, no second pitch");

        // Bold still widens a fixed-pitch run: the smear needs a column on any face.
        assert_eq!(paired.advance('i', STYLE_FIXED_PITCH | STYLE_BOLD), 8);

        // And a run measures the same way, which is what wraps.
        assert_eq!(paired.run_px("iiii", STYLE_FIXED_PITCH), 28);
        assert_eq!(paired.run_px("iiii", 0), 12);

        // A machine with no face at all is untouched by any of it.
        let fixed = V6Metric::fixed(cell);
        assert_eq!(fixed.advance('i', STYLE_FIXED_PITCH), 7);
        assert_eq!(fixed.advance('i', 0), 7);
    }

    #[test]
    fn stream_origin_records_the_first_glyph_of_a_burst_only() {
        let mut w = ZWindow { x_coord: 5, y_coord: 9, x_cursor: 3, y_cursor: 1, ..Default::default() };
        assert_eq!(w.stream_origin, None, "a window that has printed nothing has no origin");
        assert_eq!(w.pen(), (9, 7), "the pen is origin + cursor - 1, screen-absolute (§8.8.1)");

        w.record_streamed('a', 0, ZColour::Default, ZColour::Default, &V6Metric::default());
        assert_eq!(w.stream_origin, Some((9, 7)), "the first glyph lands at the pen");

        // A second glyph, wherever it goes, must not move the origin.
        w.x_cursor += V6_FONT_WIDTH;
        w.record_streamed('b', 0, ZColour::Default, ZColour::Default, &V6Metric::default());
        w.y_cursor += V6_FONT_HEIGHT;
        w.record_streamed('c', 0, ZColour::Default, ZColour::Default, &V6Metric::default());
        assert_eq!(w.stream_origin, Some((9, 7)), "…and only the first");

        w.clear_stream_origin();
        assert_eq!(w.stream_origin, None, "cleared, ready for the next burst");
        w.record_streamed('d', 0, ZColour::Default, ZColour::Default, &V6Metric::default());
        assert_eq!(w.stream_origin, Some(w.pen()), "the next burst records its own start");
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    /// Build a minimal v3 story with one object whose short name is "West of House".
    /// Object 1 is placed at the v3 entries base.
    /// G0 = 1 (location object), G1, G2 = supplied.
    fn build_v3_status_story(g1: u16, g2: u16, time_mode: bool) -> Vec<u8> {
        let mut buf = sample_story(3);

        // Object table is at 0x0100 (set by sample_story).
        // v3 property-defaults: 31 words = 62 bytes → entries at 0x013E.
        let obj1_entry: usize = 0x013E;
        let prop_tbl: u16 = 0x0200;

        // Object 1 entry (9 bytes): no attrs, no tree, prop_tbl pointer.
        for i in 0..7 { buf[obj1_entry + i] = 0; }
        buf[obj1_entry + 7] = (prop_tbl >> 8) as u8;
        buf[obj1_entry + 8] = (prop_tbl & 0xFF) as u8;

        // Property table: short name = "west" (2 Z-words).
        let name = encode_word("west", 3); // 4 bytes
        assert_eq!(name.len(), 4);
        buf[prop_tbl as usize] = 2; // 2 name-words
        buf[prop_tbl as usize + 1..prop_tbl as usize + 5].copy_from_slice(&name);
        buf[prop_tbl as usize + 5] = 0x00; // sentinel

        // Set G0=1, G1=g1, G2=g2 in global vars table (0x0300).
        let gbase: usize = 0x0300;
        buf[gbase]     = 0; buf[gbase + 1] = 1;  // G0 = 1
        buf[gbase + 2] = (g1 >> 8) as u8; buf[gbase + 3] = (g1 & 0xFF) as u8;
        buf[gbase + 4] = (g2 >> 8) as u8; buf[gbase + 5] = (g2 & 0xFF) as u8;

        // Flags1: bit 1 controls time mode.
        if time_mode {
            buf[0x01] |= 1 << 1;
        } else {
            buf[0x01] &= !(1 << 1);
        }

        buf
    }

    // ── (a) v3 status line: score/turns mode ─────────────────────────────────

    #[test]
    fn v3_status_line_score_turns() {
        let buf = build_v3_status_story(42u16, 7, false);
        let mem = Memory::new(buf).unwrap();
        let sl = compute_status_line(&mem);
        assert!(
            sl.location.starts_with("west"),
            "location should start with 'west', got {:?}", sl.location
        );
        assert_eq!(sl.right, StatusRight::ScoreTurns { score: 42, turns: 7 });
    }

    // ── (b) v3 status line: time mode ────────────────────────────────────────

    #[test]
    fn v3_status_line_time_mode() {
        let buf = build_v3_status_story(10, 30, true);
        let mem = Memory::new(buf).unwrap();
        let sl = compute_status_line(&mem);
        assert!(sl.location.starts_with("west"), "location should start with 'west'");
        assert_eq!(sl.right, StatusRight::Time { hours: 10, minutes: 30 });
    }

    // ── (b2) v1/v2 are always score games (§8.2.1) ───────────────────────────

    #[test]
    fn v1_v2_status_line_is_always_score() {
        // §8.2.1: "In Versions 1 and 2, all games are 'score games'" — the
        // Flags 1 bit 1 "time game" bit must not be consulted below v3.
        // (Tested on the predicate: `parse_header` refuses v1/v2 story files
        // outright, so no Memory can be built at those versions.)
        let flags1_with_time_bit = 1u8 << 1;
        assert!(!is_time_game(1, flags1_with_time_bit), "v1 is always a score game");
        assert!(!is_time_game(2, flags1_with_time_bit), "v2 is always a score game");
        assert!(is_time_game(3, flags1_with_time_bit), "v3 honours the bit");
        assert!(!is_time_game(3, 0), "v3 without the bit is a score game");
    }

    // ── (c) header capability bits ───────────────────────────────────────────

    #[test]
    fn header_caps_v3_clears_no_status_line() {
        let mut mem = Memory::new(sample_story(3)).unwrap();
        // Set "status line not available" bit before init.
        let f1 = mem.read_byte(0x01) | (1 << 4);
        mem.write_byte(0x01, f1);
        init_header_caps(&mut mem, false, false, None, None, Palette::Standard);
        // Bit 4 should be cleared.
        assert_eq!(mem.read_byte(0x01) & (1 << 4), 0, "bit 4 (no status line) should be clear");
        // Screen-splitting available (bit 5) should be set.
        assert_ne!(mem.read_byte(0x01) & (1 << 5), 0, "bit 5 (screen-split) should be set");
    }

    #[test]
    fn header_caps_v5_clears_unsupported_bits() {
        let mut mem = Memory::new(sample_story(5)).unwrap();
        init_header_caps(&mut mem, false, false, None, None, Palette::Standard);
        let f1 = mem.read_byte(0x01);
        // Colour (bit 0) should be clear.
        assert_eq!(f1 & (1 << 0), 0, "colour bit should be clear");
        // Pictures (bit 1) should be clear.
        assert_eq!(f1 & (1 << 1), 0, "pictures bit should be clear");
        // Fixed-space font (bit 4) should be set.
        assert_ne!(f1 & (1 << 4), 0, "fixed-space font bit should be set");
        // Interpreter number set.
        assert_eq!(mem.read_byte(0x1E), 1, "interpreter number defaults to DEC-20 (1)");
        assert_eq!(mem.read_byte(0x1F), b'A', "interpreter version = 'A'");
    }

    #[test]
    fn header_caps_v4_seeds_nonzero_screen_dims() {
        // Regression: without seeded screen dims the header keeps 0, and v4 games
        // such as Bureaucracy abort with "[Screen too small.]" on the first turn.
        let mut mem = Memory::new(sample_story(4)).unwrap();
        init_header_caps(&mut mem, false, false, None, None, Palette::Standard);
        assert_eq!(mem.read_byte(0x20), DEFAULT_SCREEN_ROWS, "screen height (lines) seeded");
        assert_eq!(mem.read_byte(0x21), DEFAULT_SCREEN_COLS, "screen width (chars) seeded");
        assert_ne!(mem.read_byte(0x20), 0, "height must not be zero");
        assert_ne!(mem.read_byte(0x21), 0, "width must not be zero");
    }

    #[test]
    fn header_caps_v5_seeds_unit_words_and_font_size() {
        let mut mem = Memory::new(sample_story(5)).unwrap();
        init_header_caps(&mut mem, false, false, None, None, Palette::Standard);
        assert_eq!(mem.read_byte(0x20), DEFAULT_SCREEN_ROWS);
        assert_eq!(mem.read_byte(0x21), DEFAULT_SCREEN_COLS);
        assert_eq!(mem.read_word(0x22), DEFAULT_SCREEN_COLS as u16, "width in units");
        assert_eq!(mem.read_word(0x24), DEFAULT_SCREEN_ROWS as u16, "height in units");
        assert_eq!(mem.read_byte(0x26), 1, "font width = 1 unit");
        assert_eq!(mem.read_byte(0x27), 1, "font height = 1 unit");
    }

    /// The extent operations agree with the divisions, at every cell (SQ-1020).
    ///
    /// Stated as a RELATION rather than as pinned numbers, so it holds for a cell
    /// no one has declared yet — which is the property the bare `py + 16` sites
    /// lacked. A run's last inked row must be the row its top is in whenever the
    /// run is one cell tall, and its bottom must be where the NEXT row starts.
    #[test]
    fn the_extent_of_a_run_agrees_with_the_row_it_is_in() {
        for h in 1u16..=32 {
            let cell = V6Cell::new(8, h);
            for y_px in 1u16..=200 {
                let rows = cell.rows_px(y_px);
                assert_eq!(rows.end, cell.bottom_px(y_px), "bottom is the end of the span");
                assert_eq!(rows.end - rows.start, u32::from(h), "a run is one cell tall");
                // The row the top is in, and the row the bottom lands in, are
                // consecutive — the invariant `py + 16 <= story_top` was really
                // asserting, and the reason a wrong `h` moved a bar by one row.
                assert_eq!(
                    u32::from(cell.row_of(y_px)) + 1,
                    rows.end / u32::from(h),
                    "cell {h}, y {y_px}: the run ends where the next row begins",
                );
            }
        }
    }

    /// A 1-based pixel of 0 is clamped, not wrapped (ZMSD §8.8.1 has no row 0).
    #[test]
    fn the_extent_of_a_run_at_the_origin_starts_at_zero() {
        let cell = V6Cell::new(7, 15);
        assert_eq!(cell.rows_px(0), 0..15, "y=0 is treated as y=1");
        assert_eq!(cell.rows_px(1), 0..15);
        assert_eq!(cell.rows_px(16), 15..30);
    }

    #[test]
    fn write_screen_dims_is_noop_for_v3() {
        // v1-3 use bytes 0x20+ for other header data; never clobber them.
        let mut mem = Memory::new(sample_story(3)).unwrap();
        let before = mem.read_byte(0x20);
        write_screen_dims(&mut mem, 30, 60, V6Cell::DEFAULT);
        assert_eq!(mem.read_byte(0x20), before, "v3 header byte 0x20 must be untouched");
    }

    #[test]
    fn write_screen_dims_clamps_zero_to_one() {
        let mut mem = Memory::new(sample_story(4)).unwrap();
        write_screen_dims(&mut mem, 0, 0, V6Cell::DEFAULT);
        assert_eq!(mem.read_byte(0x20), 1, "zero rows clamped to 1");
        assert_eq!(mem.read_byte(0x21), 1, "zero cols clamped to 1");
    }

    #[test]
    fn v6_advertises_pixel_screen_and_font() {
        let mut m = Memory::new(sample_story(6)).unwrap();
        write_screen_dims(&mut m, 24, 80, V6Cell::DEFAULT);
        assert_eq!(m.read_byte(0x20), 24, "rows");
        assert_eq!(m.read_byte(0x21), 80, "cols");
        assert_eq!(m.read_word(0x22), 80 * V6_FONT_WIDTH, "screen width in pixels");
        assert_eq!(m.read_word(0x24), 24 * V6_FONT_HEIGHT, "screen height in pixels");
        // ZMSD §11.1/§8.1.1: in V6 the font-size bytes are the swap of V5 —
        // $26 = font HEIGHT (16), $27 = font WIDTH (8). Non-square now exercises it.
        assert_eq!(m.read_byte(0x26), V6_FONT_HEIGHT as u8, "$26 = font height in V6");
        assert_eq!(m.read_byte(0x27), V6_FONT_WIDTH as u8, "$27 = font width in V6");
        assert_eq!((m.read_byte(0x26), m.read_byte(0x27)), (16, 8), "8×16 non-square cell");
    }

    #[test]
    fn header_caps_v3_clears_variable_pitch_default() {
        // We render fixed-pitch; Flags1 v3 bit 6 (variable-pitch default) must be
        // explicitly cleared rather than inheriting the story file's value.
        let mut mem = Memory::new(sample_story(3)).unwrap();
        let f1 = mem.read_byte(0x01) | (1 << 6); // pre-set variable-pitch default
        mem.write_byte(0x01, f1);
        init_header_caps(&mut mem, false, false, None, None, Palette::Standard);
        assert_eq!(mem.read_byte(0x01) & (1 << 6), 0, "bit 6 (variable-pitch) should be clear");
    }

    #[test]
    fn header_caps_writes_standard_revision_1_1() {
        // ZMSD 1.1 is the only published standard revision; advertise major=1,
        // minor=1 (bytes 0x32/0x33), not a non-existent "1.2".
        let mut mem = Memory::new(sample_story(5)).unwrap();
        init_header_caps(&mut mem, false, false, None, None, Palette::Standard);
        assert_eq!(mem.read_byte(0x32), 1, "standard revision major = 1");
        assert_eq!(mem.read_byte(0x33), 1, "standard revision minor = 1");
    }

    #[test]
    fn header_caps_v5_advertises_styles_and_undo() {
        // Bold/italic are rendered (SGR / style spans) and multi-level undo
        // (save_undo/restore_undo, EXT:0x09/0x0A) is implemented, so the header
        // must advertise them or games skip the features at startup.
        let mut mem = Memory::new(sample_story(5)).unwrap();
        init_header_caps(&mut mem, false, false, None, None, Palette::Standard);
        let f1 = mem.read_byte(0x01);
        assert_ne!(f1 & (1 << 2), 0, "Flags1 bit 2 (bold available) should be set");
        assert_ne!(f1 & (1 << 3), 0, "Flags1 bit 3 (italic available) should be set");
        let f2 = mem.read_word(0x10);
        assert_ne!(f2 & (1 << 4), 0, "Flags2 bit 4 (undo available) should be set");
    }

    #[test]
    fn header_caps_flags2_preserves_font3_and_picture_request() {
        // ZMSD §8.1.5.1: "In Version 5 (only), an interpreter which cannot
        // provide the character graphics font should clear bit 3 of 'Flags 2'."
        // We CAN provide font 3, so the game's request must survive. In V6 the
        // same bit is "game wants to use pictures" (§11.1) — also provided, so
        // also preserved. (This test previously pinned the opposite.)
        for v in [5u8, 6] {
            let mut mem = Memory::new(sample_story(v)).unwrap();
            let f2 = mem.read_word(0x10) | (1 << 3);
            mem.write_word(0x10, f2);
            init_header_caps(&mut mem, false, false, None, None, Palette::Standard);
            assert_ne!(
                mem.read_word(0x10) & (1 << 3),
                0,
                "v{v}: Flags2 bit 3 (font 3 / pictures wanted) must be preserved"
            );
        }
    }

    #[test]
    fn header_caps_flags1_advertises_timed_input_and_v6_pictures() {
        // ZMSD §11.1 "Flags 1" (Version 4+): bit 1 "Picture displaying
        // available?" (Version 6), bit 7 "Timed keyboard input available?".
        // Timed `read`/`read_char` and v6 pictures are both implemented, so both
        // must be advertised — they used to be cleared unconditionally.
        for v in [4u8, 5, 6, 7, 8] {
            let mut mem = Memory::new(sample_story(v)).unwrap();
            init_header_caps(&mut mem, false, false, None, None, Palette::Standard);
            let f1 = mem.read_byte(0x01);
            assert_ne!(f1 & (1 << 7), 0, "v{v}: Flags1 bit 7 (timed input) must be set");
            if v == 6 {
                assert_ne!(f1 & (1 << 1), 0, "v6: Flags1 bit 1 (pictures available) must be set");
            } else {
                assert_eq!(f1 & (1 << 1), 0, "v{v}: Flags1 bit 1 is a v6-only capability");
            }
        }
    }

    #[test]
    fn header_caps_flags2_preserves_mouse_request_and_clears_menus() {
        // ZMSD §11.1 "Flags 2": bit 5 "If set, game wants to use a mouse" —
        // preserved, mouse input is implemented (read_mouse / Machine::set_mouse
        // and the host delivers clicks). Bit 8 "If set, game wants to use menus"
        // — cleared, `make_menu` is a stub that always branches false.
        for v in [5u8, 6] {
            let mut mem = Memory::new(sample_story(v)).unwrap();
            mem.write_word(0x10, mem.read_word(0x10) | (1 << 5) | (1 << 8));
            init_header_caps(&mut mem, false, false, None, None, Palette::Standard);
            let f2 = mem.read_word(0x10);
            assert_ne!(f2 & (1 << 5), 0, "v{v}: Flags2 bit 5 (mouse wanted) must be preserved");
            assert_eq!(f2 & (1 << 8), 0, "v{v}: Flags2 bit 8 (menus wanted) must be cleared");
        }
    }

    #[test]
    fn header_caps_flags2_leaves_unrequested_bits_unset() {
        // The interpreter only ever CLEARS a game request it cannot honour; it
        // never invents one. With the game asking for nothing, bits 3 and 5 stay
        // clear (bit 4, UNDO, is the interpreter's own advertisement and is set).
        let mut mem = Memory::new(sample_story(5)).unwrap();
        mem.write_word(0x10, 0);
        init_header_caps(&mut mem, false, false, None, None, Palette::Standard);
        let f2 = mem.read_word(0x10);
        assert_eq!(f2 & (1 << 3), 0, "bit 3 not requested, not invented");
        assert_eq!(f2 & (1 << 5), 0, "bit 5 not requested, not invented");
        assert_ne!(f2 & (1 << 4), 0, "bit 4 (undo available) is ours to set");
    }

    #[test]
    fn write_default_colours_clamps_and_skips_pre_v5() {
        // ZMSD §8.3.3: the interpreter writes ITS default background ($2C) and
        // foreground ($2D). §8.3.1 only names 2..=9 as real colours, so anything
        // else falls back to black-on-white.
        let mut mem = Memory::new(sample_story(5)).unwrap();
        write_default_colours(&mut mem, 6, 5, Palette::Standard);
        assert_eq!((mem.read_byte(0x2C), mem.read_byte(0x2D)), (6, 5), "valid pair lands as given");
        for bad in [0u8, 1, 10, 12, 15, 200] {
            write_default_colours(&mut mem, bad, bad, Palette::Standard);
            assert_eq!(
                (mem.read_byte(0x2C), mem.read_byte(0x2D)),
                (DEFAULT_BG_COLOUR, DEFAULT_FG_COLOUR),
                "colour {bad} is not a standard colour number — falls back to 2/9"
            );
        }
        // $2C/$2D are not colour bytes before V5.
        let mut mem3 = Memory::new(sample_story(3)).unwrap();
        mem3.write_byte(0x2C, 0x11);
        mem3.write_byte(0x2D, 0x22);
        write_default_colours(&mut mem3, 6, 5, Palette::Standard);
        assert_eq!((mem3.read_byte(0x2C), mem3.read_byte(0x2D)), (0x11, 0x22), "v3 untouched");
    }

    #[test]
    fn default_colours_publish_the_header_extension_words() {
        // ZMSD §11.1.7.3: word 4 = Flags 3, word 5 = true default FOREGROUND,
        // word 6 = true default BACKGROUND — all three marked "Int"/"Rst", so
        // the interpreter writes them alongside $2C/$2D.
        let mut mem = Memory::new(sample_story(5)).unwrap();
        let ext: u32 = 0x0180; // dynamic memory
        mem.write_word(0x36, ext as u16);
        mem.write_word(ext, 6); // 6 further words
        mem.write_word(ext + 8, 0x0001); // game asked for transparency
        write_default_colours(&mut mem, 6, 5, Palette::Standard); // bg = blue, fg = yellow

        assert_eq!(mem.read_word(ext + 8), 0, "Flags 3 cleared — we provide none of its features");
        assert_eq!(
            mem.read_word(ext + 10),
            0x03BD,
            "word 5 = true default foreground (yellow, §8.3.1)"
        );
        assert_eq!(
            mem.read_word(ext + 12),
            0x59A0,
            "word 6 = true default background (blue, §8.3.1)"
        );
    }

    #[test]
    fn header_extension_writes_stop_at_the_table_length() {
        // ZMSD §11.1.7.2: writing past the table's length must do nothing.
        let mut mem = Memory::new(sample_story(5)).unwrap();
        let ext: u32 = 0x0180;
        mem.write_word(0x36, ext as u16);
        mem.write_word(ext, 4); // only 4 further words: Flags 3 is the last one
        mem.write_word(ext + 10, 0xDEAD);
        mem.write_word(ext + 12, 0xBEEF);
        write_default_colours(&mut mem, 6, 5, Palette::Standard);
        assert_eq!(mem.read_word(ext + 8), 0, "word 4 is in range and gets cleared");
        assert_eq!(mem.read_word(ext + 10), 0xDEAD, "word 5 out of range → untouched");
        assert_eq!(mem.read_word(ext + 12), 0xBEEF, "word 6 out of range → untouched");

        // No table at all → nothing happens (and no panic).
        let mut bare = Memory::new(sample_story(5)).unwrap();
        bare.write_word(0x36, 0);
        write_default_colours(&mut bare, 6, 5, Palette::Standard);
        assert_eq!(bare.read_byte(0x2C), 6, "the $2C/$2D half still lands");
    }

    #[test]
    fn sound_bit_tracks_sound_available_flag_v5() {
        let mut mem = Memory::new(sample_story(5)).unwrap();
        init_header_caps(&mut mem, false, false, None, None, Palette::Standard);
        assert_eq!(mem.read_byte(0x01) & (1 << 5), 0, "Flags1 sound bit clear when sound_available=false");
        assert_eq!(mem.read_word(0x10) & (1 << 7), 0, "Flags2 sound bit clear when sound_available=false");

        init_header_caps(&mut mem, false, true, None, None, Palette::Standard);
        assert_ne!(mem.read_byte(0x01) & (1 << 5), 0, "Flags1 sound bit set when sound_available=true");
        assert_ne!(mem.read_word(0x10) & (1 << 7), 0, "Flags2 sound bit set when sound_available=true");

        advertise_sound(&mut mem, false);
        assert_eq!(mem.read_byte(0x01) & (1 << 5), 0, "advertise_sound(false) clears Flags1 bit again");
        assert_eq!(mem.read_word(0x10) & (1 << 7), 0, "advertise_sound(false) clears Flags2 bit again");
    }

    #[test]
    fn sound_bit_v3_flags1_untouched_but_flags2_tracks() {
        let mut mem = Memory::new(sample_story(3)).unwrap();
        init_header_caps(&mut mem, false, true, None, None, Palette::Standard);
        // v3 Flags1 bit 5 means "screen-splitting available", NOT sound — must
        // stay set regardless of sound_available (it's set unconditionally by
        // init_header_caps for v3, see header_caps_v3_clears_no_status_line).
        assert_ne!(mem.read_byte(0x01) & (1 << 5), 0, "v3 Flags1 bit5 (screen-split) stays set");
        assert_ne!(mem.read_word(0x10) & (1 << 7), 0, "v3 Flags2 sound bit set when sound_available=true");

        init_header_caps(&mut mem, false, false, None, None, Palette::Standard);
        assert_ne!(mem.read_byte(0x01) & (1 << 5), 0, "v3 Flags1 bit5 (screen-split) still set");
        assert_eq!(mem.read_word(0x10) & (1 << 7), 0, "v3 Flags2 sound bit clear when sound_available=false");
    }

    #[test]
    fn colour_bit_tracks_honor_flag() {
        let mut mem = Memory::new(sample_story(5)).unwrap();
        init_header_caps(&mut mem, false, false, None, None, Palette::Standard);
        assert_eq!(mem.read_byte(0x01) & 1, 0, "colour bit clear when honor=false");
        init_header_caps(&mut mem, true, false, None, None, Palette::Standard);
        assert_eq!(mem.read_byte(0x01) & 1, 1, "colour bit set when honor=true");
        advertise_colour(&mut mem, false);
        assert_eq!(mem.read_byte(0x01) & 1, 0, "advertise_colour clears it again");
    }

    #[test]
    fn default_colours_seeded_in_header_v5plus() {
        // ZMSD §8.3.2/§8.3.3: the interpreter writes default bg/fg into $2C/$2D.
        // Infocom stories ship 0/0 (invalid "current"); we overwrite with black
        // (2) bg / white (9) fg so games that read the header defaults compute
        // valid colour numbers.
        for v in [5u8, 6, 7, 8] {
            let mut mem = Memory::new(sample_story(v)).unwrap();
            mem.write_byte(0x2C, 0); // simulate Infocom's 0/0
            mem.write_byte(0x2D, 0);
            init_header_caps(&mut mem, true, false, None, None, Palette::Standard);
            assert_eq!(mem.read_byte(0x2C), 2, "v{v} default background = black(2)");
            assert_eq!(mem.read_byte(0x2D), 9, "v{v} default foreground = white(9)");
        }
    }

    #[test]
    fn default_colours_not_written_pre_v5() {
        // $2C/$2D are not colour-default bytes before V5; leave them alone.
        let mut mem = Memory::new(sample_story(3)).unwrap();
        mem.write_byte(0x2C, 0x11);
        mem.write_byte(0x2D, 0x22);
        init_header_caps(&mut mem, true, false, None, None, Palette::Standard);
        assert_eq!(mem.read_byte(0x2C), 0x11, "v3 $2C untouched");
        assert_eq!(mem.read_byte(0x2D), 0x22, "v3 $2D untouched");
    }

    #[test]
    fn flags2_colour_request_bit_cleared_when_colour_off() {
        let mut mem = Memory::new(sample_story(5)).unwrap();
        // Game requests colours (Flags2 bit 6).
        let f2 = mem.read_word(0x10) | (1 << 6);
        mem.write_word(0x10, f2);
        // Honour OFF: the request bit is cleared (colour not granted).
        init_header_caps(&mut mem, false, false, None, None, Palette::Standard);
        assert_eq!(mem.read_word(0x10) & (1 << 6), 0, "bit 6 cleared when colour off");
        // Honour ON: the game's request bit is left untouched.
        let f2 = mem.read_word(0x10) | (1 << 6);
        mem.write_word(0x10, f2);
        init_header_caps(&mut mem, true, false, None, None, Palette::Standard);
        assert_eq!(mem.read_word(0x10) & (1 << 6), 1 << 6, "bit 6 preserved when colour on");
    }

    #[test]
    fn default_interpreter_number_follows_frotz_rule() {
        // Frotz: DEC-20 (1) for non-v6, IBM PC (6) for v6.
        assert_eq!(default_interpreter_number(3), 1);
        assert_eq!(default_interpreter_number(5), 1);
        assert_eq!(default_interpreter_number(8), 1);
        assert_eq!(default_interpreter_number(6), 6);
    }

    /// SQ-0885: `$1F` is overridable, and `None` restores the default.
    ///
    /// The byte has no provenance (see [`init_header_caps`]) and a story can
    /// PRINT it — Shogun r295 renders it as a decimal, so the default `'A'` (65)
    /// makes its Amiga banner read "version 6.65" against the real machine's
    /// "6.8". This is the knob that lets that be tried.
    ///
    /// The override rides on the `Machine` since SQ-1393, so there is nothing
    /// process-wide left to save and restore around this case: each call below
    /// states its own byte and nothing outside can observe it.
    #[test]
    fn the_interpreter_version_byte_is_overridable() {
        let byte_after = |v: Option<u8>| {
            let mut mem = Memory::new(sample_story(5)).unwrap();
            init_header_caps(&mut mem, true, false, None, v, Palette::Standard);
            mem.read_byte(0x1F)
        };
        assert_eq!(byte_after(None), b'A', "the default, unchanged");
        assert_eq!(byte_after(Some(8)), 8, "the Amiga's own, per Shogun's banner");
        assert_eq!(byte_after(Some(0)), 0, "zero is a value, not 'unset'");
        // …and the same fact through the door a host actually uses.
        let mut m = crate::cpu::exec::Machine::new(Memory::new(sample_story(5)).unwrap());
        m.set_interpreter_version(Some(8));
        m.init_caps();
        assert_eq!(m.mem.read_byte(0x1F), 8, "latched to init_caps, like $1E");
        m.set_interpreter_version(None);
        m.init_caps();
        assert_eq!(m.mem.read_byte(0x1F), b'A', "…and None restores the default");
    }

    #[test]
    fn init_header_caps_default_interpreter_is_dec20_for_v5() {
        let mut mem = Memory::new(sample_story(5)).unwrap();
        init_header_caps(&mut mem, false, false, None, None, Palette::Standard);
        assert_eq!(mem.read_byte(0x1E), 1, "v5 default interpreter = DEC-20 (1)");
    }

    #[test]
    fn init_header_caps_interpreter_override_wins() {
        let mut mem = Memory::new(sample_story(5)).unwrap();
        init_header_caps(&mut mem, false, false, Some(6), None, Palette::Standard);
        assert_eq!(mem.read_byte(0x1E), 6, "override forces IBM PC (6)");
    }

    #[test]
    fn v6_screen_state_has_window_table() {
        let m = crate::cpu::exec::Machine::new(Memory::new(sample_story(6)).unwrap());
        let v6 = m.screen.v6.as_ref().expect("v6 story has a window table");
        assert_eq!(v6.windows.len(), 8);
        assert_eq!(v6.current, 0);
    }

    #[test]
    fn non_v6_has_no_window_table() {
        let m = crate::cpu::exec::Machine::new(Memory::new(sample_story(5)).unwrap());
        assert!(m.screen.v6.is_none(), "v5 keeps the classic 2-window model");
    }

    // ── Task 6: get_prop / put_prop over the ZMSD property array ────────────

    #[test]
    fn zwindow_prop_round_trip_all_16() {
        let mut w = ZWindow::default();
        for n in 0..16u16 {
            w.put_prop(n, 1000 + n);
        }
        for n in 0..16u16 {
            assert_eq!(w.get_prop(n), 1000 + n, "prop {n} round-trips");
        }
    }

    #[test]
    fn zwindow_prop_out_of_range_get_is_zero_and_put_is_ignored() {
        // prop 0, untouched by an out-of-range write below
        let mut w = ZWindow { y_coord: 42, ..Default::default() };
        assert_eq!(w.get_prop(16), 0, "prop 16+ not modeled here — reads 0");
        assert_eq!(w.get_prop(255), 0);
        w.put_prop(16, 999); // ignored — must not alias into any real field
        w.put_prop(255, 999);
        assert_eq!(w.get_prop(0), 42, "out-of-range put left prop 0 untouched");
    }

    #[test]
    fn zwindow_prop_indices_match_zmsd_1_1_8_8_3_2() {
        // Direct field <-> index mapping, verified against ZMSD 1.1 §8.8.3.2.
        let w = ZWindow {
            y_coord: 1, x_coord: 2, y_size: 3, x_size: 4,
            y_cursor: 5, x_cursor: 6, left_margin: 7, right_margin: 8,
            interrupt_routine: 9, interrupt_countdown: 10, text_style: 11, colour_data: 12,
            font_number: 13, font_size: 14, attributes: 15, line_count: 16,
            ..Default::default()
        };
        let expected = [1u16, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        for (n, exp) in expected.into_iter().enumerate() {
            assert_eq!(w.get_prop(n as u16), exp, "prop {n}");
        }
    }

    // ── (d) ScreenState defaults ──────────────────────────────────────────────

    #[test]
    fn screen_state_defaults() {
        let s = ScreenState::default();
        assert_eq!(s.upper_window_rows, 0);
        assert_eq!(s.current_window, 0);
        assert_eq!(s.text_style, 0);
        // The lower window is buffered (word-wrapped) by default (ZMSD §8.7.2.5).
        assert!(s.buffer_mode, "buffer_mode defaults to on (buffered)");
        assert_eq!(s.current_font, 1, "default font is 1 (normal)");
    }

    // ── (f) UpperWindow: resize, put, cell, clear ───────────────────────────

    #[test]
    fn upper_window_resize_put_and_cell() {
        let mut w = UpperWindow::default();
        w.resize(2, 4);
        assert_eq!(w.rows, 2);
        assert_eq!(w.cols, 4);
        assert_eq!(w.cell(1, 1).ch, ' ');
        w.put(2, 3, 'X', 0b0001, ZColour::Default, ZColour::Default);
        assert_eq!(w.cell(2, 3).ch, 'X');
        assert_eq!(w.cell(2, 3).style, 0b0001);
        w.put(9, 9, 'Z', 0, ZColour::Default, ZColour::Default); // out of range -> ignored, no panic
        w.clear();
        assert_eq!(w.cell(2, 3).ch, ' ');
    }

    // ── Lane Z: scroll_window (EXT:0x14) helpers ─────────────────────────────

    #[test]
    fn upper_window_scroll_rows_up_shifts_content_and_blanks_bottom() {
        let mut w = UpperWindow::default();
        w.resize(3, 2);
        w.put(1, 1, 'A', 0, ZColour::Default, ZColour::Default);
        w.put(2, 1, 'B', 0, ZColour::Default, ZColour::Default);
        w.put(3, 1, 'C', 0, ZColour::Default, ZColour::Default);
        w.scroll_rows(1); // positive: scroll forward/up
        assert_eq!(w.cell(1, 1).ch, 'B', "row 2 moved up to row 1");
        assert_eq!(w.cell(2, 1).ch, 'C', "row 3 moved up to row 2");
        assert_eq!(w.cell(3, 1).ch, ' ', "new bottom row is blank");
    }

    #[test]
    fn upper_window_scroll_rows_down_shifts_content_and_blanks_top() {
        let mut w = UpperWindow::default();
        w.resize(3, 2);
        w.put(1, 1, 'A', 0, ZColour::Default, ZColour::Default);
        w.put(2, 1, 'B', 0, ZColour::Default, ZColour::Default);
        w.put(3, 1, 'C', 0, ZColour::Default, ZColour::Default);
        w.scroll_rows(-1); // negative: scroll backward/down
        assert_eq!(w.cell(1, 1).ch, ' ', "new top row is blank");
        assert_eq!(w.cell(2, 1).ch, 'A', "row 1 moved down to row 2");
        assert_eq!(w.cell(3, 1).ch, 'B', "row 2 moved down to row 3");
    }

    #[test]
    fn upper_window_scroll_rows_beyond_extent_clears() {
        let mut w = UpperWindow::default();
        w.resize(2, 2);
        w.put(1, 1, 'A', 0, ZColour::Default, ZColour::Default);
        w.scroll_rows(5);
        assert_eq!(w.cell(1, 1).ch, ' ');
        assert_eq!(w.cell(2, 1).ch, ' ');
    }

    #[test]
    fn zwindow_scroll_pixels_shifts_text_runs_and_drops_out_of_range() {
        let mut w = ZWindow { y_size: 24, ..Default::default() };
        w.texts.push(V6Text::derived(9, 1, "far".into(), 0, ZColour::Default, ZColour::Default, V6Cell::DEFAULT));
        w.texts.push(V6Text::derived(1, 1, "near".into(), 0, ZColour::Default, ZColour::Default, V6Cell::DEFAULT));
        // Scroll forward by 32px (two 16px lines):
        //   y=9  -> new_y=-23, bottom=-23+16-1=-8 < 1 -> fully above, dropped.
        //   y=1  -> new_y=-31, bottom=-31+16-1=-16 < 1 -> fully above, dropped.
        w.scroll_pixels(32, V6Cell::DEFAULT);
        assert!(w.texts.is_empty(), "both runs fully scrolled above the window");
    }

    #[test]
    fn zwindow_scroll_pixels_keeps_run_still_partially_visible() {
        let mut w = ZWindow { y_size: 24, ..Default::default() };
        w.texts.push(V6Text::derived(9, 1, "keep".into(), 0, ZColour::Default, ZColour::Default, V6Cell::DEFAULT));
        // Scroll forward by 8px: y=9 -> 1, bottom=1+16-1=16 >= 1, still kept.
        w.scroll_pixels(8, V6Cell::DEFAULT);
        assert_eq!(w.texts.len(), 1, "run still overlapping the window is kept");
        assert_eq!(w.texts[0].y, 1, "kept run shifted by -pixels");
    }

    #[test]
    fn zwindow_scroll_pixels_negative_scrolls_down() {
        let mut w = ZWindow { y_size: 24, ..Default::default() };
        w.texts.push(V6Text::derived(5, 1, "a".into(), 0, ZColour::Default, ZColour::Default, V6Cell::DEFAULT));
        w.scroll_pixels(-3, V6Cell::DEFAULT);
        assert_eq!(w.texts[0].y, 8, "negative pixels shift y downward (y - (-3) = y+3)");
    }

    #[test]
    fn zwindow_scroll_pixels_also_shifts_cell_grid_by_whole_rows() {
        let mut w = ZWindow { y_size: 24, ..Default::default() };
        w.grid.resize(3, 2);
        w.grid.put(1, 1, 'A', 0, ZColour::Default, ZColour::Default);
        w.grid.put(2, 1, 'B', 0, ZColour::Default, ZColour::Default);
        w.scroll_pixels(V6_FONT_HEIGHT as i16, V6Cell::DEFAULT); // exactly one row
        assert_eq!(w.grid.cell(1, 1).ch, 'B', "grid shifted one row up");
    }

    // ── (e) StreamState: stream-3 push/pop/write ─────────────────────────────

    #[test]
    fn stream3_push_write_pop() {
        let buf = sample_story(5);
        // Reserve a table at 0x0050 (within dynamic memory, safely away from header).
        let table_addr: u32 = 0x0050;

        let mut mem = Memory::new(buf.clone()).unwrap();
        let mut ss = StreamState::new();

        assert!(!ss.stream3_active());
        ss.push_stream3(table_addr, None);
        assert!(ss.stream3_active());

        ss.write_stream3_bytes(b"Hello");
        ss.pop_stream3(&mut mem, &V6Metric::default());

        assert!(!ss.stream3_active());

        // Check table: word at table_addr = 5 (length), then "Hello".
        assert_eq!(mem.read_word(table_addr), 5, "length word should be 5");
        assert_eq!(mem.read_byte(table_addr + 2), b'H');
        assert_eq!(mem.read_byte(table_addr + 3), b'e');
        assert_eq!(mem.read_byte(table_addr + 4), b'l');
        assert_eq!(mem.read_byte(table_addr + 5), b'l');
        assert_eq!(mem.read_byte(table_addr + 6), b'o');
    }

    #[test]
    fn stream3_write_bytes_stores_single_byte_per_char() {
        // A high ZSCII char (e.g. 195 = 'û') must be stored as ONE byte, not
        // multi-byte UTF-8 (SQ-0240).
        let buf = sample_story(5);
        let table_addr: u32 = 0x0050;

        let mut mem = Memory::new(buf).unwrap();
        let mut ss = StreamState::new();

        ss.push_stream3(table_addr, None);
        ss.write_stream3_bytes(&[195]);
        ss.pop_stream3(&mut mem, &V6Metric::default());

        assert_eq!(mem.read_word(table_addr), 1, "length word should be 1");
        assert_eq!(mem.read_byte(table_addr + 2), 195);
    }

    #[test]
    fn zcolour_defaults_and_cell_carries_colour() {
        assert_eq!(ZColour::default(), ZColour::Default);
        let c = Cell::default();
        assert_eq!(c.fg, ZColour::Default);
        assert_eq!(c.bg, ZColour::Default);

        let mut w = UpperWindow::default();
        w.resize(1, 4);
        w.put(1, 1, 'X', 0x01, ZColour::Standard(3), ZColour::Standard(6));
        let cell = w.cell(1, 1);
        assert_eq!(cell.ch, 'X');
        assert_eq!(cell.style, 0x01);
        assert_eq!(cell.fg, ZColour::Standard(3));
        assert_eq!(cell.bg, ZColour::Standard(6));
    }

    #[test]
    fn rgb15_expansion_and_greys() {
        assert_eq!(rgb15_to_888(0x7FFF), (255, 255, 255));
        assert_eq!(rgb15_to_888(0x001F), (255, 0, 0)); // red = low 5 bits
        // ZMSD §8.3.1 fixes the true-colour value of each grey; expanding those
        // 15-bit values is what `grey_rgb` must return (it used to return an
        // invented #B0/#80/#50 ramp).
        assert_eq!(grey_rgb(Palette::Standard, 10), rgb15_to_888(0x5AD6), "10 = light grey ($5AD6)");
        assert_eq!(grey_rgb(Palette::Standard, 11), rgb15_to_888(0x4631), "11 = medium grey ($4631)");
        assert_eq!(grey_rgb(Palette::Standard, 12), rgb15_to_888(0x2D6B), "12 = dark grey ($2D6B)");
        assert_eq!(grey_rgb(Palette::Standard, 11), (0x8C, 0x8C, 0x8C));
    }

    #[test]
    fn stream3_nested() {
        let buf = sample_story(5);
        let table1: u32 = 0x0050;
        let table2: u32 = 0x0060;

        let mut mem = Memory::new(buf).unwrap();
        let mut ss = StreamState::new();

        ss.push_stream3(table1, None);
        ss.write_stream3_bytes(b"ab");
        ss.push_stream3(table2, None);
        ss.write_stream3_bytes(b"cd");
        ss.pop_stream3(&mut mem, &V6Metric::default()); // finalise table2
        ss.write_stream3_bytes(b"ef");
        ss.pop_stream3(&mut mem, &V6Metric::default()); // finalise table1

        // table2: "cd" (2 bytes)
        assert_eq!(mem.read_word(table2), 2);
        assert_eq!(mem.read_byte(table2 + 2), b'c');
        assert_eq!(mem.read_byte(table2 + 3), b'd');

        // table1: "ab" + "ef" = "abef" (4 bytes)
        assert_eq!(mem.read_word(table1), 4);
        assert_eq!(mem.read_byte(table1 + 2), b'a');
        assert_eq!(mem.read_byte(table1 + 3), b'b');
        assert_eq!(mem.read_byte(table1 + 4), b'e');
        assert_eq!(mem.read_byte(table1 + 5), b'f');
    }

    // ── (f) v6 output_stream 3 width operand: word-wrap on close ─────────────
    // ZMSD §15 output_stream: "In Version 6, a width field may optionally be
    // given: text will then be justified as if it were in the window with
    // that number (if width is zero or positive) or a box -width pixels wide
    // (if negative). Then the table will contain not ordinary text but
    // formatted text: see print_form."

    /// Read a formatted-text table back as its lines — ZMSD §15 `print_form`:
    /// "a sequence of lines, terminated with a zero word. Each line is a word
    /// containing the number of characters, followed by that many bytes which
    /// hold the characters concerned."
    fn read_formatted(mem: &Memory, table_addr: u32) -> Vec<String> {
        let mut at = table_addr;
        let mut lines = Vec::new();
        loop {
            let count = mem.read_word(at);
            at += 2;
            if count == 0 {
                return lines;
            }
            let bytes: Vec<u8> = (0..count as u32).map(|i| mem.read_byte(at + i)).collect();
            at += count as u32;
            lines.push(String::from_utf8_lossy(&bytes).into_owned());
        }
    }

    #[test]
    fn stream3_width_wraps_overflowing_word_onto_new_line() {
        // "AAAA BBBB" at a 40px box (V6_FONT_WIDTH=8 -> 5 chars) doesn't fit
        // "AAAA BBBB" (72px) on one line; the wrap point starts a new LINE of
        // the formatted table and drops the space from the width tally (Frotz
        // redirect.c:memory_word skips the leading space of the overflowing
        // word).
        //
        // A width operand also changes the LAYOUT, not just where the breaks
        // fall — ZMSD §15 `output_stream`: "Then the table will contain not
        // ordinary text but formatted text: see print_form" (SQ-1006).
        let buf = sample_story(6);
        let table_addr: u32 = 0x0050;
        let mut mem = Memory::new(buf).unwrap();
        let mut ss = StreamState::new();

        ss.push_stream3(table_addr, Some(40));
        ss.write_stream3_bytes(b"AAAA BBBB");
        ss.pop_stream3(&mut mem, &V6Metric::default());

        assert_eq!(read_formatted(&mem, table_addr), ["AAAA", "BBBB"], "one record per line");
        // Total width = 4 chars + 4 chars (the dropped space isn't printable width).
        assert_eq!(mem.read_word(0x30), 8 * V6_FONT_WIDTH, "header $30 excludes the wrap newline");
    }

    #[test]
    fn stream3_width_no_wrap_when_text_fits() {
        // Text that fits within the box is one line — but still a formatted
        // table with a terminating zero word, because the width operand is what
        // selects that layout (ZMSD §15 `output_stream`), not the wrapping.
        let buf = sample_story(6);
        let table_addr: u32 = 0x0050;
        let mut mem = Memory::new(buf).unwrap();
        let mut ss = StreamState::new();

        ss.push_stream3(table_addr, Some(200));
        ss.write_stream3_bytes(b"Score:");
        ss.pop_stream3(&mut mem, &V6Metric::default());

        assert_eq!(read_formatted(&mem, table_addr), ["Score:"]);
        assert_eq!(mem.read_word(table_addr), 6, "the one line's own character count");
        assert_eq!(mem.read_word(0x30), 6 * V6_FONT_WIDTH);
    }

    /// The other half of the pair: NO width operand keeps the plain layout of
    /// ZMSD §7.1.2.1 — "the initial word of the table holds the number of
    /// characters printed and subsequent bytes hold those characters" — with no
    /// terminating zero word. Pinned beside the formatted cases so a future
    /// change cannot quietly give every table the same shape.
    #[test]
    fn stream3_without_a_width_keeps_the_plain_layout() {
        let buf = sample_story(6);
        let table_addr: u32 = 0x0050;
        let mut mem = Memory::new(buf).unwrap();
        let mut ss = StreamState::new();

        ss.push_stream3(table_addr, None);
        ss.write_stream3_bytes(b"AAAA BBBB");
        ss.pop_stream3(&mut mem, &V6Metric::default());

        assert_eq!(mem.read_word(table_addr), 9, "word 0 is the whole character count");
        let bytes: Vec<u8> = (0..9).map(|i| mem.read_byte(table_addr + 2 + i)).collect();
        assert_eq!(bytes, b"AAAA BBBB", "unformatted: the text verbatim, no line records");
    }

    /// SQ-0679: when the HOST widens the grid, the columns that appear continue
    /// the appearance their row already ended in — so a status bar the game
    /// painted as a run of reverse-video spaces reaches the new right edge
    /// instead of stopping at the old one. Shrinking is still plain truncation,
    /// and a row that ended in default cells is byte-identical to before.
    #[test]
    fn widening_continues_each_rows_trailing_appearance() {
        let mut u = UpperWindow::default();
        u.resize(2, 4);
        // Row 1: a reverse-video bar with text in it, the whole row.
        for (c, ch) in " Hi ".chars().enumerate() {
            u.cells[c] = Cell { ch, style: 0x01, fg: ZColour::Default, bg: ZColour::Standard(4) };
        }
        // Row 2 is left entirely default.
        u.resize_continuing_row_style(2, 7);

        assert_eq!(u.cols, 7);
        assert_eq!((1..=4).map(|c| u.cell(1, c).ch).collect::<String>(), " Hi ", "old columns verbatim");
        for c in 5..=7 {
            let cell = u.cell(1, c);
            assert_eq!(cell.ch, ' ', "a grown column is blank space, never a copied glyph");
            assert_eq!(cell.style, 0x01, "…carrying the row's trailing style (col {c})");
            assert!(matches!(cell.bg, ZColour::Standard(4)), "…and its colours (col {c})");
        }
        for c in 5..=7 {
            let cell = u.cell(2, c);
            assert_eq!(cell.style, 0, "a default row grows default (col {c})");
            assert!(matches!(cell.bg, ZColour::Default));
        }

        // A shrink truncates and continues nothing.
        u.resize_continuing_row_style(2, 2);
        assert_eq!(u.cols, 2);
        assert_eq!((1..=2).map(|c| u.cell(1, c).ch).collect::<String>(), " H");
    }

    // ── ZMSD §8.3, the Amiga rule (SQ-0740) ──────────────────────────────────

    /// A story header describing `version` on interpreter `interp`, with colour
    /// either advertised or withdrawn (Flags 1 bit 0, §8.3.2/§8.3.3).
    fn header_for(version: u8, interp: u8, colour: bool) -> crate::cpu::exec::Machine {
        let mut m = crate::cpu::exec::Machine::new(Memory::new(sample_story(version)).unwrap());
        m.mem.write_byte(0x1E, interp);
        advertise_colour(&mut m.mem, colour);
        m
    }

    #[test]
    fn the_amiga_rule_is_read_out_of_the_header_and_nothing_else() {
        // §8.3 scopes it to "a Version 6 interpreter going under the Amiga
        // interpreter number", and §11.1.3 numbers the Amiga 4.
        assert!(amiga_global_colour_pair(&header_for(6, 4, true)), "v6 on interpreter 4");
        // Every other machine keeps the per-window model §8.3 gives it. 6 is the
        // IBM PC — the number the whole existing v6 corpus runs under, so this
        // row is the pin that says the corpus cannot move.
        for interp in [1u8, 2, 3, 5, 6, 7, 8, 9, 10, 11] {
            assert!(
                !amiga_global_colour_pair(&header_for(6, interp, true)),
                "v6 on interpreter {interp} keeps one pair per window",
            );
        }
        // Below Version 6 there is one screen pair anyway; the rule names v6.
        for v in [3u8, 4, 5, 7, 8] {
            assert!(!amiga_global_colour_pair(&header_for(v, 4, true)), "version {v}");
        }
        // …and with `honor_game_colours` off lanthorn declares itself colourless
        // (§8.3.2), so the host theme owns the screen and there is no pair to
        // share. The Amiga rule must not reach past that switch.
        assert!(
            !amiga_global_colour_pair(&header_for(6, 4, false)),
            "colours withdrawn: the theme owns the screen, Amiga or not",
        );
    }

    /// The pair the host PAINTS with, as opposed to the pair §8.3.3 advertises to
    /// the story: on the Amiga they are the same two bytes, and before SQ-0740
    /// lanthorn wrote them and then painted the terminal's colours instead.
    #[test]
    fn the_amiga_screen_pair_is_the_headers_own_default_colours() {
        let mut mem = header_for(6, 4, true);
        // What `InterpreterProfile::Amiga` publishes: `DEF_BACK 12` (dark grey)
        // and `DEF_FORE 9` (white), read out of the release floppies' own Amiga
        // interpreters (SQ-0822).
        write_default_colours(&mut mem.mem, 12, 9, Palette::Standard);
        assert_eq!(
            amiga_screen_pair(&mem),
            Some((ZColour::Standard(9), ZColour::Standard(12))),
            "(foreground, background), straight off $2D/$2C",
        );
        // Every machine that is not an Amiga has no such thing — each window
        // carries its own pair and the host theme owns everything else.
        let mut ibm = header_for(6, 6, true);
        write_default_colours(&mut ibm.mem, 12, 9, Palette::Standard);
        assert_eq!(amiga_screen_pair(&ibm), None, "interpreter 6 publishes no screen pair");
        // …and neither does a colourless interpreter, Amiga or not.
        let mut off = header_for(6, 4, false);
        write_default_colours(&mut off.mem, 12, 9, Palette::Standard);
        assert_eq!(amiga_screen_pair(&off), None, "colours withdrawn: nothing to paint with");
        // …and neither does a launch that declines to present its machine at all
        // (SQ-1154): the fourth term of `machine_rule`, and the only one a story
        // cannot reach. This is `--colour theme|terminal` on Amiga media.
        let mut unlicensed = header_for(6, 4, true);
        write_default_colours(&mut unlicensed.mem, 12, 9, Palette::Standard);
        unlicensed.machine_colours_licensed = false;
        assert_eq!(
            amiga_screen_pair(&unlicensed),
            None,
            "an unlicensed launch presents no machine, so there is no screen pair to paint",
        );
        assert!(
            !amiga_global_colour_pair(&unlicensed),
            "…and the pens rule is off with it, so a set_colour stays on its own window",
        );
    }

    /// A v6 screen with something already drawn on it: window 1 holds a grid cell
    /// and a painted run in an explicit pair, window 2 holds a run drawn over
    /// whatever was underneath it (a `-1`/inherited background), and window 3 is a
    /// window the game has never coloured.
    fn screen_with_text_on_it() -> ScreenState {
        let mut s = ScreenState { v6: Some(V6Windows::default()), ..Default::default() };
        let v6 = s.v6_mut().unwrap();
        let w1 = &mut v6.windows[1];
        w1.fg = ZColour::Standard(9);
        w1.bg = ZColour::Standard(2);
        w1.grid.resize(1, 1);
        w1.grid.cells[0] = Cell {
            ch: 'A',
            style: 0,
            fg: ZColour::Standard(9),
            bg: ZColour::Standard(2),
        };
        w1.texts.push(V6Text::derived(1, 1, "banner".into(), 0, ZColour::Standard(9), ZColour::Standard(2), V6Cell::DEFAULT));
        w1.streamed.push(V6Text::derived(9, 1, "prose".into(), 0, ZColour::Standard(9), ZColour::Standard(2), V6Cell::DEFAULT));
        w1.retired.push(V6Text::derived(17, 1, "frozen".into(), 0, ZColour::Standard(9), ZColour::Standard(2), V6Cell::DEFAULT));
        v6.windows[2].texts.push(V6Text::derived(1, 1, "over the art".into(), 0, ZColour::Standard(9), ZColour::Default, V6Cell::DEFAULT));
        s
    }

    /// The heart of §8.3: "If either is changed, then the interpreter must change
    /// the colour of ALL TEXT ON THE SCREEN to match."
    ///
    /// So this prints first and changes the colour second, and asserts the text
    /// that was already there moved. A test that only checked newly printed text
    /// would pass without the rule implemented at all.
    ///
    /// FALSIFY by deleting the `w.texts`/`streamed`/`retired`/`grid` loops from
    /// `repaint_amiga_pens`: every "already on the screen" assertion below fails
    /// with the reported symptom — glyphs still white (standard 9) after the game
    /// asked for black (standard 2).
    #[test]
    fn changing_either_colour_repaints_the_text_already_on_the_screen() {
        let mut s = screen_with_text_on_it();
        // …and the change is made from window 0, which owns none of that text.
        s.set_amiga_colour_pair(
            0,
            Some(ZColour::Standard(2)),
            Some(ZColour::Standard(10)),
            false,
            false,
        );

        let v6 = s.v6.as_ref().unwrap();
        let w1 = &v6.windows[1];
        assert_eq!(w1.grid.cells[0].fg, ZColour::Standard(2), "the grid cell's ink follows the pen");
        assert_eq!(w1.grid.cells[0].bg, ZColour::Standard(10), "…and its page");
        for (what, run) in
            [("texts", &w1.texts[0]), ("streamed", &w1.streamed[0]), ("retired", &w1.retired[0])]
        {
            assert_eq!(run.fg, ZColour::Standard(2), "{what}: already-painted ink follows the pen");
            assert_eq!(run.bg, ZColour::Standard(10), "{what}: and its page");
        }
        // "The same pair of colours for all windows" — including windows that did
        // not ask and windows that hold nothing.
        for (i, w) in v6.windows.iter().enumerate() {
            assert_eq!(w.fg, ZColour::Standard(2), "window {i} shares the foreground");
        }
        // A run drawn over what was underneath it painted no background at all, so
        // a pen carrying a new background has nothing there to change. Its INK
        // still follows — that is the reported symptom, prose left white while the
        // game had asked for black.
        let over_art = &v6.windows[2].texts[0];
        assert_eq!(over_art.fg, ZColour::Standard(2), "ink follows even over artwork");
        assert_eq!(over_art.bg, ZColour::Default, "…but a transparent background stays transparent");
        assert_eq!(v6.windows[3].bg, ZColour::Default, "an uncoloured window gains no opaque page");
    }

    /// The `0` sentinel means "leave this channel alone" (§8.3), and a channel
    /// left alone is a pen that has not moved — so it repaints nothing.
    #[test]
    fn a_channel_the_game_left_alone_moves_no_pen() {
        let mut s = screen_with_text_on_it();
        s.set_amiga_colour_pair(0, Some(ZColour::Standard(2)), None, false, false);
        let w1 = &s.v6.as_ref().unwrap().windows[1];
        assert_eq!(w1.texts[0].fg, ZColour::Standard(2), "the channel that moved repaints");
        assert_eq!(w1.texts[0].bg, ZColour::Standard(2), "the one that did not is untouched");
    }

    /// Colour **-1** is "the colour of the pixel under the cursor" (§8.3.1): it
    /// names no colour, so there is nothing to load into a pen. Infocom's own
    /// Amiga interpreter carves it out of the window-0 gate for exactly that reason
    /// (`amiga/yzip3.c`), and Zork Zero depends on it — it prints its banner
    /// labels over the ribbon artwork under `COLOR 2 -1`.
    ///
    /// So the request reaches the window that made it, and stops there: a window
    /// other than 0 moves no pen, whatever channels it names.
    #[test]
    fn the_pixel_under_the_cursor_is_a_paint_request_not_a_pen() {
        let mut s = screen_with_text_on_it();
        // Window 1 asks for black on whatever is beneath it.
        s.set_amiga_colour_pair(
            1,
            Some(ZColour::Standard(2)),
            Some(ZColour::Default),
            false,
            true,
        );
        let v6 = s.v6.as_ref().unwrap();
        assert_eq!(v6.windows[1].bg, ZColour::Default, "the asking window draws over the art");
        assert_eq!(v6.windows[1].fg, ZColour::Standard(2), "…in the ink it asked for");
        // No pen moved, so nothing already on the screen changed — not its page,
        // and not its ink either.
        assert_eq!(
            v6.windows[1].texts[0].bg,
            ZColour::Standard(2),
            "a -1 request repaints no existing background",
        );
        assert_eq!(
            v6.windows[1].texts[0].fg,
            ZColour::Standard(9),
            "…and a window that is not window 0 moves no ink pen either",
        );
        assert_eq!(v6.windows[0].fg, ZColour::Default, "no other window hears about it at all");
    }

    /// Infocom's window-0 gate: "We allow text colors to be changed only in window
    /// 0, and ignore requests in other windows (except for the special case of
    /// bg = -1)" (`amiga/yzip3.c`). A plain request from any other window is
    /// dropped whole — it does not move the pens AND it does not reach the window
    /// that made it.
    ///
    /// This is what makes `Journey - The Quest Begins.adf` (release 30, serial
    /// 890322) play on the Amiga's own light-grey default rather than the black
    /// page its single `set_colour(9, 2)` — issued on window 3 — asks for.
    ///
    /// FALSIFY by deleting the `win != 0` early return: window 3 takes the pair and
    /// every other window follows it.
    #[test]
    fn a_request_from_a_window_other_than_0_is_dropped_whole() {
        let mut s = screen_with_text_on_it();
        s.set_amiga_colour_pair(
            3,
            Some(ZColour::Standard(9)),
            Some(ZColour::Standard(2)),
            false,
            false,
        );
        let v6 = s.v6.as_ref().unwrap();
        assert_eq!(v6.windows[3].fg, ZColour::Default, "the asking window is not coloured");
        assert_eq!(v6.windows[3].bg, ZColour::Default, "…on either channel");
        assert_eq!(v6.windows[0].fg, ZColour::Default, "no pen moved");
        assert_eq!(
            v6.windows[1].texts[0].fg,
            ZColour::Standard(9),
            "nothing already on the screen was repainted",
        );
        assert_eq!(s.current_fg, ZColour::Default, "and the prose stream's pair is untouched");
    }

    /// Nothing here may touch a machine that is not an Amiga: the entry point is
    /// only ever reached through [`amiga_global_colour_pair`], and a v6 screen
    /// with no window table is left entirely alone.
    #[test]
    fn a_screen_with_no_v6_window_table_is_untouched() {
        let mut s = ScreenState::default();
        s.set_amiga_colour_pair(0, Some(ZColour::Standard(2)), Some(ZColour::Standard(10)), false, false);
        assert_eq!(s.current_fg, ZColour::Default);
        assert_eq!(s.current_bg, ZColour::Default);
    }

    /// SQ-1191: every mutation class a screen-model reader cares about moves
    /// [`ScreenState::v6_generation`] — paint, erase, move/resize, scroll, and
    /// the Amiga pens repaint — because each one reaches the table through
    /// [`ScreenState::v6_mut`], the one door. And the counter moves ONLY for
    /// v6 mutation: a read borrow and a v1–5 screen leave it alone.
    #[test]
    fn v6_generation_moves_with_every_mutation_class() {
        let metric = V6Metric::fixed(V6Cell::DEFAULT);
        let mut s = ScreenState { v6: Some(V6Windows::default()), ..Default::default() };
        let mut last = s.v6_generation();
        let moved = |s: &ScreenState, what: &str, last: &mut u64| {
            assert!(s.v6_generation() > *last, "{what} must advance the v6 generation");
            *last = s.v6_generation();
        };

        // Paint: a run deposited on window 1.
        s.v6_mut().unwrap().paint_run(1, run_at(15, 31, "banner", 0), &metric);
        moved(&s, "paint_run", &mut last);

        // Erase: a screen rect wiped across the shared raster.
        s.v6_mut().unwrap().erase_screen_rect(1, 1, 32, 64, &metric);
        moved(&s, "erase_screen_rect", &mut last);

        // Move/resize: window props written the way the opcodes write them.
        s.v6_mut().unwrap().windows[1].put_prop(0, 33); // y position
        moved(&s, "put_prop (move)", &mut last);
        s.v6_mut().unwrap().windows[1].put_prop(2, 64); // height
        moved(&s, "put_prop (resize)", &mut last);

        // Scroll: pixels through a window.
        s.v6_mut().unwrap().windows[0].scroll_pixels(15, V6Cell::DEFAULT);
        moved(&s, "scroll_pixels", &mut last);

        // Colour: the §8.3 Amiga pens repaint bumps WITHOUT the caller ever
        // borrowing the table — its own inner borrows go through the door.
        s.set_amiga_colour_pair(0, Some(ZColour::Standard(9)), Some(ZColour::Standard(2)), false, false);
        moved(&s, "set_amiga_colour_pair", &mut last);

        // A read borrow moves nothing…
        let _ = s.v6.as_ref().unwrap().windows[0].texts.len();
        assert_eq!(s.v6_generation(), last, "a read borrow must not advance the generation");

        // …and neither does a v1–5 screen, which has no v6 table to change.
        let mut classic = ScreenState::default();
        assert!(classic.v6_mut().is_none());
        assert_eq!(classic.v6_generation(), 0, "no v6 table, no generation to move");
    }

    /// SQ-1191: the generation moves through the `Machine` seams a host drives —
    /// printed prose, a host resize — and stays MONOTONE across `@restart`'s
    /// wholesale screen swap, so a model cached against the pre-restart screen
    /// can never match the rebooted one.
    #[test]
    fn v6_generation_moves_through_the_machine_seams() {
        let mem = crate::memory::Memory::new(crate::header::tests_support::sample_story(6)).unwrap();
        let mut m = crate::cpu::exec::Machine::new(mem);
        let g0 = m.screen.v6_generation();

        m.print_text("hello\n");
        let g1 = m.screen.v6_generation();
        assert!(g1 > g0, "prose printed through window 0 must advance the generation");

        m.set_v6_screen_px(560, 384);
        let g2 = m.screen.v6_generation();
        assert!(g2 > g1, "a host resize must advance the generation");

        m.restart();
        assert!(
            m.screen.v6_generation() > g2,
            "@restart swaps in a fresh screen; its generation must continue past the old one, never restart behind it"
        );
    }

    /// SQ-1191 discipline: [`ScreenState::v6_mut`] is the ONE DOOR to
    /// `&mut V6Windows`. The only `.v6.as_mut(` in the crate's source is the
    /// line inside `v6_mut` itself — any other spelling is a mutation path the
    /// generation counter cannot see, written by someone with no reason to know
    /// the counter exists (the `scratch_path_discipline` shape, SQ-1163).
    ///
    /// Deliberately NOT scanned: `.v6 = Some(…)` installs. Those are boot-shaped
    /// — a fresh table on a fresh or local `ScreenState` (boot, fixtures) — and
    /// the one production swap of a LIVE screen, `Machine::restart`, carries the
    /// counter across explicitly. Outlawing the spelling would forbid legitimate
    /// fixture setup to guard a path the restart carry and the field's own docs
    /// already cover.
    #[test]
    fn v6_generation_discipline() {
        // Assembled so this test's own source is not a hit.
        let needle = format!(".v6.as_mut{}", '(');
        fn scan(dir: &std::path::Path, needle: &str, hits: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let p = entry.unwrap().path();
                if p.is_dir() {
                    scan(&p, needle, hits);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    for (i, line) in std::fs::read_to_string(&p).unwrap().lines().enumerate() {
                        if line.trim_start().starts_with("//") {
                            continue; // prose may name the spelling; code may not
                        }
                        if line.contains(needle) {
                            hits.push(format!("{}:{}", p.display(), i + 1));
                        }
                    }
                }
            }
        }
        let mut hits = Vec::new();
        scan(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &needle, &mut hits);
        assert_eq!(
            hits.len(),
            1,
            "`{needle}` may appear exactly once in zvm — inside ScreenState::v6_mut. \
             Route any other mutable borrow through `v6_mut()` so the v6 generation moves with it. Found: {hits:?}"
        );
        assert!(
            hits[0].contains("screen.rs"),
            "the one sanctioned `{needle}` lives in screen.rs's v6_mut, found {hits:?}"
        );
    }
}
