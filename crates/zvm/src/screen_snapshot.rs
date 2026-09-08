//! A versioned, dependency-free binary snapshot of the Z-machine SCREEN.
//!
//! # Why this exists at all
//!
//! Quetzal ([`crate::quetzal`]) saves no screen state, and that is deliberate:
//! the standard assumes the *story* repaints after an in-game `@restore`. A HOST
//! snapshot ("Save State" — an emulator-style capture taken between turns) does
//! not get that assumption. It swaps dynamic memory under a game that never
//! learns it happened, so nothing repaints and everything the screen needs is
//! the host's to carry.
//!
//! Before this module every embedder carried it by hand: `lanthorn`'s archive
//! held a serde mirror of `ScreenState`, `ZWindow`, `V6Windows`, `V6Text`,
//! `Cell` and `ZColour`, six types that had to move in lockstep with the ones
//! here across a crate boundary with nothing checking that they still agreed,
//! and a second embedder would have written the whole thing again.
//!
//! # What it is
//!
//! [`encode`] turns a [`ScreenState`] into bytes and [`decode`] turns them back;
//! [`Machine::screen_snapshot`](crate::cpu::exec::Machine::screen_snapshot) and
//! [`Machine::restore_screen_snapshot`](crate::cpu::exec::Machine::restore_screen_snapshot)
//! are the same pair spelled against a live machine. Hand-rolled and versioned,
//! exactly as `quetzal.rs` is, because `zvm` takes no external dependencies.
//!
//! # What it is NOT
//!
//! **Backend- and terminal-neutral.** Version 6 geometry travels in zvm's own
//! native pixels, never in terminal cells; there are no font metrics and no host
//! state of any kind in here. A snapshot therefore moves between graphics
//! backends and between terminal sizes, which is the whole point: a restore into
//! a different pane is a resize the game never saw, and the host reconciles it
//! afterwards (see [`Machine::restore_screen_snapshot`]).
//!
//! **Not the display list.** What pictures were drawn where is the host's own
//! record — `zvm` reports draws as events and keeps no canvas — so a host that
//! wants its art back composes this blob with a picture record of its own.
//!
//! **Not a save file.** It carries no story identity and validates against none:
//! it is one half of a host snapshot whose other half is a Quetzal buffer, and
//! that half already checks release/serial/checksum.
//!
//! # Format
//!
//! All integers big-endian, matching `quetzal.rs`.
//!
//! ```text
//! "ZSCR"                        magic, 4 bytes
//! u16   version                 = 1; a GREATER version is refused
//! u16   upper_window_rows
//! u8    current_window
//! u8    text_style
//! u16   cursor_row
//! u16   cursor_col
//! u8    buffer_mode             0 | 1
//! u8    show_status_requested   0 | 1
//! u8    v6_input_window
//! colour current_fg             see below
//! colour current_bg
//! grid  upper                   see below
//! u8    has_v6                  0 | 1
//!   u8    v6.current            present only when has_v6 = 1
//!   8 x window                  "
//!
//! colour := u8 tag + u32 value  0 Default (value 0), 1 Standard, 2 True, 3 True24
//! cell   := u32 ch (Unicode scalar) + u8 style + colour fg + colour bg
//! grid   := u16 cols + u16 rows + u32 count + count x cell
//! run    := u16 y + u16 x + str text + u8 style + colour fg + colour bg
//!           + u16 grow + u16 gcol
//! runs   := u32 count + count x run
//! str    := u32 byte length + UTF-8 bytes
//! window := 16 x u16 props (ZMSD 1.1 8.8.3.2, property number = index)
//!           + grid + colour fg + colour bg
//!           + runs texts + u32 count + count x str prose
//!           + runs streamed + runs retired
//! ```
//!
//! # What is deliberately NOT in it
//!
//! Every one of these is either a REQUEST the host drains within the turn, or a
//! memo about a derivation the restore redoes. Persisting a derived result
//! instead of its inputs is how a restore comes back looking right and stops
//! being recomputable (SQ-0587/0588).
//!
//! - [`ScreenState::erase_lower_requested`] — a request the host drains each
//!   turn; a snapshot is taken between turns with none outstanding, and a
//!   replayed one would erase the lower window the restore just brought back.
//! - [`ScreenState::upper_rows_stranded_by_split`] — transient, and `false` is
//!   the answer that keeps whatever is on screen on screen.
//! - [`ScreenState::current_font`] — transient display state Quetzal does not
//!   carry either; the story re-selects its font.
//! - [`ScreenState::v6_generation`] — a change COUNTER, meaningless without the
//!   history it counted. A host that cached anything against the old value must
//!   drop it, which is exactly what a restored zero forces.
//! - [`ZWindow::stream_origin`] — per-burst state that only lives between a
//!   clear and the read that follows it.
//! - [`ZWindow::grid_pen`] — a memo about a derivation ([`GridPen`]), and
//!   [`ZWindow::grid_cursor`] re-derives whenever it is absent.
//!
//! One field is written but derived rather than carried: for a **Version 6**
//! story `current_fg`/`current_bg` are encoded as `Default`, because ZMSD §8.3
//! gives every v6 window its own pair and the window table below IS the
//! authority — they only mirror the current window's so the prose stream can tag
//! its runs. Versions 1–5/7/8 have no window table and nothing else that holds
//! the game's selected colour, so for them the pair travels for real.

use crate::error::ZError;
use crate::screen::{Cell, ScreenState, UpperWindow, V6Text, V6Windows, ZColour, ZWindow};

/// Magic bytes at the head of every snapshot: "Z-machine SCReen".
const MAGIC: &[u8; 4] = b"ZSCR";

/// The format version this build writes, and the highest it can read.
pub const VERSION: u16 = 1;

/// Upper bound on a decoded grid's dimensions (SQ-0647). Well past any real
/// terminal — ZMSD §11.1 gives the header only a BYTE each for screen height and
/// width in characters — and low enough that a corrupt `65535 x 65535` cannot
/// ask for a four-billion-cell allocation on the way in. A host reconciles the
/// restored screen against its current pane anyway, so clamping here costs a
/// restore nothing it was not about to recompute.
const MAX_GRID_COLS: u16 = 1024;
const MAX_GRID_ROWS: u16 = 1024;

/// Bytes one encoded [`Cell`] occupies, used to reject a declared cell count that
/// the remaining buffer cannot possibly hold before allocating for it.
const CELL_BYTES: usize = 4 + 1 + COLOUR_BYTES * 2;
/// Bytes one encoded [`ZColour`] occupies.
const COLOUR_BYTES: usize = 1 + 4;
/// The smallest an encoded run or prose line can be, for the same guard.
const MIN_RUN_BYTES: usize = 2 + 2 + 4 + 1 + COLOUR_BYTES * 2 + 2 + 2;
const MIN_STR_BYTES: usize = 4;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Serialise a screen to a versioned byte buffer.
///
/// See the module docs for the layout and for the handful of fields deliberately
/// left out of it.
pub fn encode(screen: &ScreenState) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    put_u16(&mut out, VERSION);
    put_u16(&mut out, screen.upper_window_rows);
    out.push(screen.current_window);
    out.push(screen.text_style);
    put_u16(&mut out, screen.cursor_row);
    put_u16(&mut out, screen.cursor_col);
    out.push(u8::from(screen.buffer_mode));
    out.push(u8::from(screen.show_status_requested));
    out.push(screen.v6_input_window);
    // For v6 the window table below is the single source of truth for the ink
    // (module docs); writing the mirror as well would give one screen two
    // sources that could quietly disagree.
    let (fg, bg) = match screen.v6 {
        Some(_) => (ZColour::Default, ZColour::Default),
        None => (screen.current_fg, screen.current_bg),
    };
    put_colour(&mut out, fg);
    put_colour(&mut out, bg);
    put_grid(&mut out, &screen.upper);
    match &screen.v6 {
        None => out.push(0),
        Some(v6) => {
            out.push(1);
            out.push(v6.current);
            for w in &v6.windows {
                put_window(&mut out, w);
            }
        }
    }
    out
}

/// Rebuild a screen from a buffer written by [`encode`].
///
/// # Errors
///
/// [`ZError::ScreenSnapshotVersion`] when the buffer was written by a NEWER
/// format version than this build understands (it names both numbers), and
/// [`ZError::BadScreenSnapshot`] when it is not a snapshot at all, is truncated,
/// or is otherwise unreadable.
///
/// A snapshot is a file on the player's disk, not a value this run produced, so
/// what CAN be repaired is repaired rather than refused: a grid whose cell count
/// disagrees with its own `cols x rows` is padded or truncated to fit (SQ-0647 —
/// zvm's grid code indexes straight into `cells`, so a mismatch is a panic
/// waiting for the first repaint after the restore), and a v6 `current` outside
/// 0..=7 is clamped (an archived 9 panicked on the first frame, not on the load).
/// Both keep text that is genuinely there rather than throwing a whole screen
/// away over an arithmetic slip.
pub fn decode(bytes: &[u8]) -> Result<ScreenState, ZError> {
    let mut r = Reader { b: bytes, i: 0 };
    if r.take(4)? != MAGIC {
        return Err(ZError::BadScreenSnapshot);
    }
    let version = r.u16()?;
    if version > VERSION {
        return Err(ZError::ScreenSnapshotVersion { found: version, supported: VERSION });
    }
    let upper_window_rows = r.u16()?;
    let current_window = r.u8()?;
    let text_style = r.u8()?;
    let cursor_row = r.u16()?;
    let cursor_col = r.u16()?;
    let buffer_mode = r.u8()? != 0;
    let show_status_requested = r.u8()? != 0;
    let v6_input_window = r.u8()?;
    let current_fg = r.colour()?;
    let current_bg = r.colour()?;
    let upper = r.grid()?;
    let v6 = match r.u8()? {
        0 => None,
        _ => {
            // ZMSD §8.4 has exactly eight v6 windows, and `windows[current]` is a
            // fixed array index every host screen read performs.
            let current = r.u8()?.min(7);
            let mut windows: [ZWindow; 8] = Default::default();
            for w in windows.iter_mut() {
                *w = r.window()?;
            }
            Some(V6Windows::new(windows, current))
        }
    };
    Ok(ScreenState {
        upper_window_rows,
        current_window,
        text_style,
        cursor_row,
        cursor_col,
        buffer_mode,
        show_status_requested,
        upper,
        current_fg,
        current_bg,
        v6,
        v6_input_window,
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn put_colour(out: &mut Vec<u8>, c: ZColour) {
    let (tag, value) = match c {
        ZColour::Standard(n) => (1u8, u32::from(n)),
        ZColour::True(v) => (2u8, u32::from(v)),
        ZColour::True24(v) => (3u8, v),
        // `ZColour` is `#[non_exhaustive]`: anything this build does not know is
        // the interpreter default, which is what an unset pen means anyway.
        _ => (0u8, 0u32),
    };
    out.push(tag);
    put_u32(out, value);
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

fn put_cell(out: &mut Vec<u8>, c: &Cell) {
    put_u32(out, c.ch as u32);
    out.push(c.style);
    put_colour(out, c.fg);
    put_colour(out, c.bg);
}

fn put_grid(out: &mut Vec<u8>, g: &UpperWindow) {
    put_u16(out, g.cols);
    put_u16(out, g.rows);
    put_u32(out, g.cells.len() as u32);
    for c in &g.cells {
        put_cell(out, c);
    }
}

fn put_run(out: &mut Vec<u8>, t: &V6Text) {
    put_u16(out, t.y);
    put_u16(out, t.x);
    put_str(out, &t.text);
    out.push(t.style);
    put_colour(out, t.fg);
    put_colour(out, t.bg);
    put_u16(out, t.grow);
    put_u16(out, t.gcol);
}

fn put_runs(out: &mut Vec<u8>, runs: &[V6Text]) {
    put_u32(out, runs.len() as u32);
    for t in runs {
        put_run(out, t);
    }
}

fn put_window(out: &mut Vec<u8>, w: &ZWindow) {
    for n in 0..16u16 {
        put_u16(out, w.get_prop(n));
    }
    put_grid(out, &w.grid);
    put_colour(out, w.fg);
    put_colour(out, w.bg);
    put_runs(out, &w.texts);
    put_u32(out, w.prose.len() as u32);
    for line in &w.prose {
        put_str(out, line);
    }
    put_runs(out, &w.streamed);
    put_runs(out, &w.retired);
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ZError> {
        let end = self.i.checked_add(n).ok_or(ZError::BadScreenSnapshot)?;
        let s = self.b.get(self.i..end).ok_or(ZError::BadScreenSnapshot)?;
        self.i = end;
        Ok(s)
    }

    fn remaining(&self) -> usize {
        self.b.len().saturating_sub(self.i)
    }

    /// A declared element count, refused before it is allocated for when the rest
    /// of the buffer could not possibly hold that many. A snapshot is a file on
    /// disk: a corrupt `count` of four billion must be an error, not an OOM.
    fn count(&mut self, min_element_bytes: usize) -> Result<usize, ZError> {
        let n = self.u32()? as usize;
        if n.saturating_mul(min_element_bytes) > self.remaining() {
            return Err(ZError::BadScreenSnapshot);
        }
        Ok(n)
    }

    fn u8(&mut self) -> Result<u8, ZError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, ZError> {
        let s = self.take(2)?;
        Ok(u16::from_be_bytes([s[0], s[1]]))
    }

    fn u32(&mut self) -> Result<u32, ZError> {
        let s = self.take(4)?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn colour(&mut self) -> Result<ZColour, ZError> {
        let tag = self.u8()?;
        let v = self.u32()?;
        Ok(match tag {
            1 => ZColour::Standard(v as u8),
            2 => ZColour::True(v as u16),
            3 => ZColour::True24(v),
            // Tag 0 is `Default`; an unknown tag from a same-version writer is a
            // corrupt byte, and the interpreter default is the safe reading.
            _ => ZColour::Default,
        })
    }

    fn string(&mut self) -> Result<String, ZError> {
        let n = self.u32()? as usize;
        let s = self.take(n)?;
        // Lossy rather than an error: the run's POSITION is the fact a restore
        // needs, and one bad byte in a caption is not worth discarding a screen.
        Ok(String::from_utf8_lossy(s).into_owned())
    }

    fn cell(&mut self) -> Result<Cell, ZError> {
        let ch = char::from_u32(self.u32()?).unwrap_or(' ');
        let style = self.u8()?;
        let fg = self.colour()?;
        let bg = self.colour()?;
        Ok(Cell::new(ch, style, fg, bg))
    }

    fn grid(&mut self) -> Result<UpperWindow, ZError> {
        let cols = self.u16()?.min(MAX_GRID_COLS);
        let rows = self.u16()?.min(MAX_GRID_ROWS);
        let n = self.count(CELL_BYTES)?;
        let mut cells = Vec::with_capacity(n);
        for _ in 0..n {
            cells.push(self.cell()?);
        }
        // `from_cells` holds the invariant every consumer assumes —
        // `cells.len() == cols * rows` — by padding or truncating to fit.
        Ok(UpperWindow::from_cells(cols, rows, cells))
    }

    fn run(&mut self) -> Result<V6Text, ZError> {
        let y = self.u16()?;
        let x = self.u16()?;
        let text = self.string()?;
        let style = self.u8()?;
        let fg = self.colour()?;
        let bg = self.colour()?;
        let grow = self.u16()?;
        let gcol = self.u16()?;
        Ok(V6Text::at_cell(y, x, text, style, fg, bg, grow, gcol))
    }

    fn runs(&mut self) -> Result<Vec<V6Text>, ZError> {
        let n = self.count(MIN_RUN_BYTES)?;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(self.run()?);
        }
        Ok(out)
    }

    fn window(&mut self) -> Result<ZWindow, ZError> {
        let mut w = ZWindow::default();
        for n in 0..16u16 {
            let v = self.u16()?;
            w.put_prop(n, v);
        }
        w.grid = self.grid()?;
        w.fg = self.colour()?;
        w.bg = self.colour()?;
        w.texts = self.runs()?;
        let n = self.count(MIN_STR_BYTES)?;
        let mut prose = Vec::with_capacity(n);
        for _ in 0..n {
            prose.push(self.string()?);
        }
        w.prose = prose;
        w.streamed = self.runs()?;
        w.retired = self.runs()?;
        Ok(w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Byte offset of the `has_v6` flag in a snapshot whose upper grid is EMPTY:
    /// magic + version + the nine scalar bytes + the two colours + an empty grid.
    const EMPTY_UPPER_HAS_V6_AT: usize =
        4 + 2 + (2 + 1 + 1 + 2 + 2 + 1 + 1 + 1) + COLOUR_BYTES * 2 + (2 + 2 + 4);

    fn sample_run(y: u16, x: u16, text: &str) -> V6Text {
        V6Text::at_cell(y, x, text.to_string(), 2, ZColour::Standard(4), ZColour::True(0x1234), y / 16, x / 8)
    }

    fn sample_v5_screen() -> ScreenState {
        let mut s = ScreenState {
            upper_window_rows: 2,
            current_window: 1,
            text_style: 5,
            cursor_row: 2,
            cursor_col: 7,
            buffer_mode: false,
            show_status_requested: true,
            current_fg: ZColour::Standard(9),
            current_bg: ZColour::True(0x03E0),
            ..Default::default()
        };
        s.upper.resize(2, 6);
        s.upper.put(1, 1, 'W', 2, ZColour::Standard(3), ZColour::Standard(2));
        s.upper.put(2, 6, 'z', 0, ZColour::True24(0x00FF7700), ZColour::Default);
        s
    }

    fn sample_v6_screen() -> ScreenState {
        let mut windows: [ZWindow; 8] = Default::default();
        for (i, w) in windows.iter_mut().enumerate() {
            w.put_prop(0, 16 * i as u16 + 1);
            w.put_prop(1, 1);
            w.put_prop(2, 32);
            w.put_prop(3, 640);
            w.put_prop(4, 5);
            w.put_prop(5, 9);
            w.put_prop(10, 3);
            w.put_prop(14, 15);
            w.put_prop(15, 4);
            w.fg = ZColour::Standard(2 + i as u8);
            w.bg = ZColour::True(0x7C00);
            w.grid.resize(2, 3);
            w.grid.put(1, 2, 'q', 1, ZColour::Standard(5), ZColour::Standard(6));
            w.texts = vec![sample_run(16 * i as u16 + 1, 9, "painted")];
            w.prose = vec!["one".to_string(), "two".to_string()];
            w.streamed = vec![sample_run(48, 1, "streamed")];
            w.retired = vec![sample_run(64, 1, "retired")];
        }
        ScreenState {
            v6_input_window: 3,
            v6: Some(V6Windows::new(windows, 5)),
            ..Default::default()
        }
    }

    /// Field-by-field equality of everything the blob is documented to carry.
    /// Spelled out rather than `PartialEq` so a field ADDED to `ScreenState`
    /// without being added here is a review question, not a silent pass.
    fn assert_same(a: &ScreenState, b: &ScreenState) {
        assert_eq!(a.upper_window_rows, b.upper_window_rows, "upper_window_rows");
        assert_eq!(a.current_window, b.current_window, "current_window");
        assert_eq!(a.text_style, b.text_style, "text_style");
        assert_eq!(a.cursor_row, b.cursor_row, "cursor_row");
        assert_eq!(a.cursor_col, b.cursor_col, "cursor_col");
        assert_eq!(a.buffer_mode, b.buffer_mode, "buffer_mode");
        assert_eq!(a.show_status_requested, b.show_status_requested, "show_status_requested");
        assert_eq!(a.v6_input_window, b.v6_input_window, "v6_input_window");
        assert_grid_same(&a.upper, &b.upper, "upper window");
        match (&a.v6, &b.v6) {
            (None, None) => {}
            (Some(x), Some(y)) => {
                assert_eq!(x.current, y.current, "v6.current");
                for (i, (wx, wy)) in x.windows.iter().zip(y.windows.iter()).enumerate() {
                    for n in 0..16u16 {
                        assert_eq!(wx.get_prop(n), wy.get_prop(n), "window {i} prop {n}");
                    }
                    assert_eq!((wx.fg, wx.bg), (wy.fg, wy.bg), "window {i} colours");
                    assert_grid_same(&wx.grid, &wy.grid, &format!("window {i}"));
                    assert_eq!(wx.texts, wy.texts, "window {i} texts");
                    assert_eq!(wx.prose, wy.prose, "window {i} prose");
                    assert_eq!(wx.streamed, wy.streamed, "window {i} streamed");
                    assert_eq!(wx.retired, wy.retired, "window {i} retired");
                }
            }
            _ => panic!("one screen has a v6 window table and the other does not"),
        }
    }

    fn assert_grid_same(a: &UpperWindow, b: &UpperWindow, what: &str) {
        assert_eq!(a.cols, b.cols, "{what} grid cols");
        assert_eq!(a.rows, b.rows, "{what} grid rows");
        assert_eq!(a.cells.len(), b.cells.len(), "{what} grid cell count");
        for (i, (x, y)) in a.cells.iter().zip(b.cells.iter()).enumerate() {
            assert_eq!((x.ch, x.style, x.fg, x.bg), (y.ch, y.style, y.fg, y.bg), "{what} cell {i}");
        }
    }

    #[test]
    fn a_classic_screen_round_trips() {
        let src = sample_v5_screen();
        let back = decode(&encode(&src)).expect("decodes");
        assert_same(&src, &back);
        // The classic pair travels for real: there is no window table to derive
        // it from below Version 6.
        assert_eq!(back.current_fg, ZColour::Standard(9));
        assert_eq!(back.current_bg, ZColour::True(0x03E0));
    }

    #[test]
    fn a_v6_window_table_round_trips() {
        let src = sample_v6_screen();
        let back = decode(&encode(&src)).expect("decodes");
        assert_same(&src, &back);
    }

    #[test]
    fn a_v6_screen_leaves_the_mirrored_pair_to_the_window_table() {
        // Documented above: for v6 the window table is the single source of truth
        // for the ink, so the mirror is written as `Default` and the host
        // re-derives it. Two sources that can disagree is the defect.
        let src = ScreenState {
            current_fg: ZColour::Standard(6),
            current_bg: ZColour::Standard(2),
            ..sample_v6_screen()
        };
        let back = decode(&encode(&src)).expect("decodes");
        assert_eq!(back.current_fg, ZColour::Default);
        assert_eq!(back.current_bg, ZColour::Default);
        assert_eq!(back.v6.as_ref().unwrap().windows[5].fg, ZColour::Standard(7),
            "the window table it is re-derived from is intact");
    }

    #[test]
    fn an_empty_default_screen_round_trips() {
        let back = decode(&encode(&ScreenState::default())).expect("decodes");
        assert_same(&ScreenState::default(), &back);
    }

    #[test]
    fn every_truncation_is_an_error_and_not_a_panic() {
        let full = encode(&sample_v6_screen());
        for n in 0..full.len() {
            assert_eq!(decode(&full[..n]).unwrap_err(), ZError::BadScreenSnapshot, "truncated to {n}");
        }
        assert!(decode(&full).is_ok(), "the untruncated blob still decodes");
    }

    #[test]
    fn a_foreign_buffer_is_refused() {
        assert_eq!(decode(b"").unwrap_err(), ZError::BadScreenSnapshot);
        assert_eq!(decode(b"FORM\0\0\0\0IFZS").unwrap_err(), ZError::BadScreenSnapshot);
    }

    #[test]
    fn a_newer_version_is_refused_and_names_both_numbers() {
        let mut blob = encode(&sample_v5_screen());
        blob[4..6].copy_from_slice(&(VERSION + 7).to_be_bytes());
        assert_eq!(
            decode(&blob).unwrap_err(),
            ZError::ScreenSnapshotVersion { found: VERSION + 7, supported: VERSION }
        );
    }

    #[test]
    fn a_corrupt_count_is_an_error_rather_than_an_allocation() {
        // The upper grid's cell count is the four bytes straight after its
        // cols/rows, i.e. the four before an EMPTY grid's `has_v6` flag.
        let mut blob = encode(&sample_v5_screen());
        let at = EMPTY_UPPER_HAS_V6_AT - 4;
        blob[at..at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(decode(&blob).unwrap_err(), ZError::BadScreenSnapshot);
    }

    #[test]
    fn a_grid_whose_cells_disagree_with_its_own_size_is_repaired() {
        // SQ-0647: zvm indexes straight into `cells`, so a short vector is a panic
        // waiting for the first repaint after the restore. Claim 40x4 and supply
        // the 12 cells of a 6x2 grid.
        let mut src = sample_v5_screen();
        src.upper.cols = 40;
        src.upper.rows = 4;
        let back = decode(&encode(&src)).expect("decodes");
        assert_eq!(back.upper.cells.len(), 160);
        assert_eq!(back.upper.cell(1, 1).ch, 'W', "the cells that were there are kept");
    }

    #[test]
    fn an_out_of_range_v6_current_window_is_clamped() {
        // SQ-0647 again: `windows[current]` is a fixed array index every host
        // screen read performs, so an archived 9 panicked on the first frame after
        // the restore rather than on the load. Poked into the bytes because
        // `V6Windows::new` clamps at the door.
        let mut blob = encode(&sample_v6_screen());
        assert_eq!(blob[EMPTY_UPPER_HAS_V6_AT], 1, "the has_v6 flag is where the arithmetic says");
        blob[EMPTY_UPPER_HAS_V6_AT + 1] = 9;
        let back = decode(&blob).expect("decodes");
        assert_eq!(back.v6.as_ref().unwrap().current, 7, "clamped to the last window");
    }
}
