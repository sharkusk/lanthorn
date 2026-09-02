//! Pixel-precise mouse reporting — SGR-Pixels, DEC private mode 1016 (SQ-0563).
//!
//! A terminal normally reports a click as a text CELL, which is all the app's own
//! UI needs but discards everything a game's graphics window cares about.
//! advent.blb's toolbar is the worst case: it draws ~12px buttons into a window
//! two 18px text rows tall, stacking THREE compass-rose rows inside those two
//! cells. A cell-centre click can only ever name canvas row 9 or 27, so the
//! middle row — W and E — occupies a pixel band no click can reach, and those two
//! buttons are unclickable however carefully you aim.
//!
//! Mode 1016 switches the SGR mouse report from cells to pixels ("Enable SGR
//! Mouse PixelMode", xterm ctlseqs). Coordinates then arrive as pixels, so the
//! app divides back to cells for every hit test it already had — byte-identical
//! behaviour — and keeps the remainder as the position WITHIN the cell, which is
//! exactly what a Glk graphics window's pixel hit test wants.
//!
//! Support is confirmed, never assumed. Guessing wrong is not a cosmetic failure:
//! dividing already-cell coordinates by the cell size would land every click in
//! the app at roughly an eighth of its true position. So the probe sets the mode,
//! asks DECRQM whether it took, and resets it again unless the terminal answers
//! "set". Terminals that don't recognise the mode answer 0 (or don't answer at
//! all) and keep today's cell-granular behaviour.

use std::sync::OnceLock;

use crossterm::event::MouseEvent;

/// Set mode 1016, ask whether it took (DECRQM `CSI ? 1016 $ p`), and end with a
/// DSR that every responding terminal answers so the reply drain terminates.
const QUERY: &[u8] = b"\x1b[?1016h\x1b[?1016$p\x1b[5n";

/// Turn pixel reporting back off. Mode 1016 is terminal state that outlives the
/// process, so this belongs on every exit path — a shell left in PixelMode would
/// hand pixel coordinates to the next program that reads the mouse.
pub const RESET: &str = "\x1b[?1016l";

/// The terminal's cell size in pixels, set once when pixel reporting has been
/// confirmed. `None` — the default, and the only possibility on a terminal that
/// declined or on Windows, where crossterm drives the mouse through WinAPI rather
/// than these escapes — leaves every coordinate a cell coordinate.
static CELL_PX: OnceLock<(u16, u16)> = OnceLock::new();

/// Probe the terminal for SGR-Pixels support, leaving the mode ON when supported
/// and OFF otherwise. Must run in the pre-UI query window (alongside the
/// image-protocol Picker and the OSC 10/11 colour probe), before anything reads
/// terminal events. Never hangs; never leaks reply bytes into the app's input.
pub fn query_pixel_mouse_support() -> bool {
    let supported = crate::term_colors::query_with(QUERY)
        .map(|reply| parse_decrpm_set(&reply.text, 1016))
        .unwrap_or(false);
    if !supported {
        use std::io::Write as _;
        let mut out = std::io::stdout();
        let _ = out.write_all(RESET.as_bytes()).and_then(|_| out.flush());
    }
    supported
}

/// Record the cell size and start translating pixel coordinates. Call only after
/// [`query_pixel_mouse_support`] returned true AND the cell size is really known
/// (a guessed cell size would misplace clicks exactly like a guessed mode).
pub fn activate(cell_px: (u16, u16)) {
    if cell_px.0 > 0 && cell_px.1 > 0 {
        let _ = CELL_PX.set(cell_px);
    }
}

/// The confirmed cell size, or `None` while coordinates are plain cells.
pub fn cell_px() -> Option<(u16, u16)> {
    CELL_PX.get().copied()
}

/// Read a DECRPM reply (`CSI ? Ps ; Pm $ y`) for `mode` and report whether it is
/// set. Per xterm's ctlseqs, `Pm` is 0 = not recognised, 1 = set, 2 = reset,
/// 3 = permanently set, 4 = permanently reset — so 1 and 3 both mean the terminal
/// is reporting pixels, and everything else (including a missing reply) does not.
pub fn parse_decrpm_set(reply: &str, mode: u16) -> bool {
    let marker = format!("\x1b[?{mode};");
    let Some(start) = reply.find(&marker) else { return false };
    let rest = &reply[start + marker.len()..];
    let Some(end) = rest.find("$y") else { return false };
    matches!(rest[..end].trim().parse::<u16>(), Ok(1 | 3))
}

/// Convert a pixel-reported mouse event to cells, returning the event with cell
/// coordinates plus the click's offset WITHIN that cell in pixels.
///
/// Pure and total: `cell_px` components are treated as at least 1, so no division
/// by zero is possible however the terminal answered.
pub fn to_cells(m: MouseEvent, cell_px: (u16, u16)) -> (MouseEvent, (u16, u16)) {
    let (cw, ch) = (cell_px.0.max(1), cell_px.1.max(1));
    let (px, py) = (m.column, m.row);
    let cells = MouseEvent { column: px / cw, row: py / ch, ..m };
    (cells, (px % cw, py % ch))
}

/// Normalise an incoming mouse event for the rest of the app: in cell mode it is
/// returned untouched with no sub-cell offset; in pixel mode its coordinates
/// become cells and the sub-cell pixel offset rides alongside.
///
/// Every mouse consumer downstream keeps working on cells; only a Glk graphics
/// window's hit test looks at the offset.
pub fn normalise(m: MouseEvent) -> (MouseEvent, Option<(u16, u16)>) {
    match cell_px() {
        Some(cell) => {
            let (m, sub) = to_cells(m, cell);
            (m, Some(sub))
        }
        None => (m, None),
    }
}

#[cfg(all(test, feature = "t-input"))]
mod tests {
    use super::*;
    use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};

    fn down(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn decrpm_reports_set_and_permanently_set_as_supported() {
        assert!(parse_decrpm_set("\x1b[?1016;1$y", 1016), "1 = set");
        assert!(parse_decrpm_set("\x1b[?1016;3$y", 1016), "3 = permanently set");
    }

    #[test]
    fn decrpm_reports_everything_else_as_unsupported() {
        assert!(!parse_decrpm_set("\x1b[?1016;0$y", 1016), "0 = not recognised");
        assert!(!parse_decrpm_set("\x1b[?1016;2$y", 1016), "2 = reset");
        assert!(!parse_decrpm_set("\x1b[?1016;4$y", 1016), "4 = permanently reset");
        assert!(!parse_decrpm_set("\x1b[0n", 1016), "a bare DSR reply says nothing about 1016");
        assert!(!parse_decrpm_set("", 1016), "no reply at all → unsupported");
        // A reply about a DIFFERENT mode must never be read as this one's answer.
        assert!(!parse_decrpm_set("\x1b[?2026;1$y", 1016), "another mode's reply is not ours");
        // Truncated / malformed replies decline rather than guess.
        assert!(!parse_decrpm_set("\x1b[?1016;1", 1016), "no terminator → unsupported");
        assert!(!parse_decrpm_set("\x1b[?1016;x$y", 1016), "non-numeric → unsupported");
    }

    #[test]
    fn decrpm_finds_the_answer_among_other_replies() {
        // Terminals batch replies; ours may arrive after another query's answer.
        let reply = "\x1b]11;rgb:0000/0000/0000\x07\x1b[?1016;1$y\x1b[0n";
        assert!(parse_decrpm_set(reply, 1016), "the 1016 answer is found mid-stream");
    }

    #[test]
    fn pixels_become_the_containing_cell_plus_an_offset() {
        // 8×18 cells (advent.blb's report geometry). Pixel 0 is cell 0, offset 0.
        assert_eq!(to_cells(down(0, 0), (8, 18)).0.column, 0);
        assert_eq!(to_cells(down(0, 0), (8, 18)).1, (0, 0));
        // Last pixel of the first cell, then the first of the second.
        let (m, sub) = to_cells(down(7, 17), (8, 18));
        assert_eq!((m.column, m.row), (0, 0), "still inside the first cell");
        assert_eq!(sub, (7, 17), "at its far corner");
        let (m, sub) = to_cells(down(8, 18), (8, 18));
        assert_eq!((m.column, m.row), (1, 1), "one pixel further is the next cell");
        assert_eq!(sub, (0, 0), "at its origin");
    }

    /// The whole point: the sub-cell offset distinguishes the three compass rows
    /// advent.blb stacks inside two text rows. A click on the W button (canvas
    /// y 12..24) lands in cell row 0 with an offset that names y 12 — a coordinate
    /// cell-centre reporting could never produce (it only ever says 9 or 27).
    #[test]
    fn sub_cell_offsets_reach_a_band_cell_centres_cannot() {
        let cell = (8u16, 18u16);
        let centres: Vec<u16> = (0..2).map(|r| r * cell.1 + cell.1 / 2).collect();
        assert_eq!(centres, vec![9, 27], "what cell-granular clicks can name");

        // A real pixel click 14px down the toolbar: cell row 0, offset 14.
        let (m, sub) = to_cells(down(18, 14), cell);
        assert_eq!(m.row, 0, "cell row unchanged for every other hit test");
        let canvas_y = m.row * cell.1 + sub.1;
        assert_eq!(canvas_y, 14, "and the game hears the pixel actually clicked");
        assert!((12..24).contains(&canvas_y), "inside the W/E band");
        assert!(!centres.contains(&canvas_y), "which no cell centre reaches");
    }

    #[test]
    fn a_zero_cell_size_cannot_divide_by_zero() {
        let (m, sub) = to_cells(down(5, 7), (0, 0));
        assert_eq!((m.column, m.row), (5, 7));
        assert_eq!(sub, (0, 0));
    }

    #[test]
    fn normalise_is_a_no_op_until_pixel_mode_is_confirmed() {
        // `activate` is process-wide and never called in this test binary, so this
        // pins the default: coordinates pass through as cells, no offset.
        let (m, sub) = normalise(down(42, 7));
        assert_eq!((m.column, m.row), (42, 7));
        assert_eq!(sub, None, "no offset means every consumer behaves as before");
    }
}
