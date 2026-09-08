//! A fold of the Version 6 paint stream, in true issue order — what a host
//! replays to rebuild every window's picture canvas without the story's help.
//!
//! # Why this exists
//!
//! [`crate::cpu::exec::Machine::take_paint_events`] hands a host the RAW,
//! ordered stream of `draw_picture`/`erase_picture`/`erase_window` events a
//! turn produced. A host that wants to redraw a window later — after a
//! palette change, or after a Save State restore, neither of which the
//! Z-machine standard gives the story any way to notice — cannot just replay
//! that stream from the top: it is unbounded (a session-long game has drawn
//! thousands of pictures by the end) and most of it is superseded, not
//! current. What a redraw needs is the SMALL SET OF OPS still explaining
//! what is on screen right now: everything since each window's last
//! whole-canvas clear, with a window's later moves not moving what was
//! already painted (ZMSD §8: "subsequent movements of the window do not move
//! what was printed") — and, because ZMSD §8 describes ONE shared screen, in
//! the SAME relative order across every window, not just within each one:
//! a host that models the screen as independent per-window canvases (as
//! `lanthorn` does) must mirror one window's erase into every other window it
//! overlaps, and getting that mirror's position wrong relative to each
//! window's own draws is exactly as wrong as getting a draw's position wrong.
//!
//! Folding the stream down to that set is pure Z-machine bookkeeping — it
//! reads only [`crate::cpu::exec::PaintEvent`] geometry, never a decoded
//! picture — so it belongs here rather than in every host that wants it, and
//! two embedders cannot fold the stream two different ways.
//!
//! # Feeding it
//!
//! [`PaintLog::apply`] is called by `Machine` itself, at the exact points
//! [`crate::cpu::exec::Machine`]'s `pending_pictures` / `pending_erase_fills`
//! are queued — so the log always reflects every event this `Machine` has
//! ever issued, regardless of whether (or how often) a host drains
//! [`crate::cpu::exec::Machine::take_paint_events`]. A host never calls
//! [`PaintLog::apply`] itself; [`crate::cpu::exec::Machine::paint_log`] is
//! read-only for exactly that reason.
//!
//! # What is deliberately NOT in it
//!
//! **No rasterization.** A [`PaintOp`] names a picture NUMBER, a window box,
//! and window-relative native-pixel coordinates — never a decoded image,
//! never a canvas, never a resolved colour. Turning that into pixels needs
//! the story's own resource archive, which is the host's to hold.
//!
//! **No cross-window erasure.** Which OTHER canvas an erase overlaps, and
//! where in THAT canvas's own coordinates, is a fact about the host's canvas
//! model (`lanthorn` models eight independent ones; ZMSD §8 describes one
//! shared screen) — not about the Z-machine, and this module has no opinion
//! on it. What it DOES guarantee is the one thing a host cannot compute for
//! itself: that replaying [`PaintLog::ops_in_order`] through the SAME
//! functions that render a live turn reproduces every cross-window mirror at
//! its true position, because the walk is in true issue order and each op
//! still carries its own window and its own `win_box`. A host derives the
//! mirror at replay time, from that order — it does not store it.
//!
//! **No erase-picture footprint.** `erase_picture` (ZMSD §15) names a
//! picture number and a position but never a size — the ENGINE has no way to
//! know how large the erased region is without decoding the picture, which
//! is exactly the thing this module does not do. [`PaintOp::ErasePicture`]
//! therefore carries the same `(number, x, y)` the opcode does; a host
//! resolves the footprint from its own picture archive when it replays the
//! op, exactly as it resolves a [`PaintOp::Draw`]'s pixels.
//!
//! # Retirement
//!
//! The log is not append-only; rules keep it bounded and keep it standing
//! for the CURRENT screen rather than the whole session:
//!
//! - **A whole-window clear resets that window's entries to one.**
//!   `erase_window` (ZMSD §8.7.3.3, any target — a single window, `-1`, or
//!   `-2`) fills the window with its background, i.e. paints over everything
//!   drawn since the last clear; everything before it is dead FOR THAT
//!   WINDOW (other windows' entries, including any issued in between, are
//!   untouched — order across windows is exactly what
//!   [`PaintLog::ops_in_order`] preserves). `Machine` emits BOTH an
//!   [`crate::cpu::exec::EraseFill`] and a `PictureEvent { number: 0, erase:
//!   true }` for the same call — [`PaintLog::apply`] folds both into the
//!   SAME single [`PaintOp::Clear`] entry (the second application is a no-op
//!   over the first), never two.
//! - **A window move strands its entries.** ZMSD §8: pixels already plotted
//!   do not move when the window does. So the moment a draw or erase-picture
//!   event's window box reports an origin different from the log's
//!   currently-tracked one for that window, that window's entries — which
//!   describe pixels at the OLD origin — are no longer a recipe for anything
//!   on screen and are dropped before the new op is recorded.
//! - **`Machine::restart` clears the whole log** (ZMSD §6.1.3's reboot), in
//!   the same breath as the paint queues themselves — no window survives a
//!   restart holding a pre-restart recipe.
//! - **A window's entries are capped** at [`PAINT_LOG_CAP`]. A story that
//!   only ever draws into one window and never clears it would otherwise grow
//!   that window's entries for the life of the session; once a window hits
//!   the cap, further ops for it are dropped until the next whole-window
//!   clear (which resets its count to one) or restart. A host reading
//!   [`PaintLog::ops`] sees exactly [`PAINT_LOG_CAP`] entries for a capped
//!   window and knows replay will not reproduce it — precisely the shape
//!   `lanthorn`'s own prior `V6_OPS_CAP` used to detect for its
//!   fallback-to-PNG decision ([`PaintLog::is_capped`]).

use crate::cpu::exec::PaintEvent;
use crate::error::ZError;

/// Number of Version 6 windows (ZMSD §8.4).
const N_WINDOWS: usize = 8;

/// Longest op list kept per window before further ops are dropped (see the
/// module docs' retirement rules). Matches `lanthorn`'s prior `V6_OPS_CAP`,
/// comfortably above any real screen's redraw history.
pub const PAINT_LOG_CAP: usize = 512;

/// One entry in the paint log, in window-relative NATIVE pixels (1-based,
/// exactly as the Z-machine opcodes give them) — no cell, canvas, or
/// terminal coordinate of any kind, and no host canvas model of any kind
/// (see the module docs' "What is deliberately NOT in it").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PaintOp {
    /// A `draw_picture` (ZMSD §15). `x`/`y` are the pixel coordinates (of the
    /// picture's top-left corner) within the window; `win_box` is the
    /// window's own box — `(x, y, w, h)` in native pixels — at the moment of
    /// the call. `at_cursor`, `margin_after` and `out_chars` are
    /// [`crate::cpu::exec::PictureEvent`]'s own fields of the same name,
    /// carried through unchanged — genuine facts about THIS call (whether it
    /// landed on the window's text cursor, whether a `set_margins` followed
    /// it, and how many characters window 0 had ever streamed at the moment
    /// of the call), not a host's opinion about them. A host that wants to
    /// tell an inline text float from a window-canvas picture (ZMSD says
    /// nothing about the difference; it is a host rendering decision) needs
    /// exactly these three alongside the geometry above — `out_chars` in
    /// particular must be the value AT THE CALL, not whatever a host's own
    /// running counter reads by the time it gets around to replaying this op:
    /// `Machine`'s own counter has typically moved on by then (a whole turn's
    /// text streams before its pictures are drained), and standing in the
    /// live count for the historical one misclassifies exactly the picture
    /// this field exists to classify correctly.
    Draw {
        number: u16,
        x: u16,
        y: u16,
        win_box: (u16, u16, u16, u16),
        at_cursor: bool,
        margin_after: Option<u16>,
        out_chars: u64,
    },
    /// An `erase_picture` (ZMSD §15). Same fields as [`Self::Draw`]; the
    /// picture's footprint is not known here — see the module docs.
    ErasePicture {
        number: u16,
        x: u16,
        y: u16,
        win_box: (u16, u16, u16, u16),
        at_cursor: bool,
        margin_after: Option<u16>,
        out_chars: u64,
    },
    /// The whole-window clear an `erase_window` paints, folded from the
    /// paired [`crate::cpu::exec::EraseFill`] and canvas-clear `PictureEvent`
    /// `Machine` emits for one call — see the module docs' first retirement
    /// rule. `win_box` is the window's box at erase time.
    Clear { win_box: (u16, u16, u16, u16) },
}

impl PaintOp {
    /// The window box every variant carries, used both to detect a window
    /// move (retirement) and, on decode, to re-derive each window's current
    /// origin without storing it separately (see [`decode`]).
    fn win_box(&self) -> (u16, u16, u16, u16) {
        match *self {
            PaintOp::Draw { win_box, .. }
            | PaintOp::ErasePicture { win_box, .. }
            | PaintOp::Clear { win_box } => win_box,
        }
    }
}

/// The Version 6 paint log: every window's folded picture/erase history, in
/// ONE ordered stream so a host can replay it in true issue order across
/// windows — see the module docs. Fed by `Machine` at the same points it
/// queues [`PaintEvent`]s; a host never calls [`PaintLog::apply`] itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaintLog {
    /// Append-only within a window's own retirement lifetime: [`Self::apply`]
    /// only ever pushes to the end or removes a whole window's entries
    /// (`retain`, which preserves the relative order of what survives), so
    /// this Vec's own order IS issue order — no separate sequence number
    /// needed.
    entries: Vec<(u8, PaintOp)>,
    /// Each window's origin as of its most recently recorded entry — `None`
    /// until that window's first op. Compared against each new event's
    /// window box to detect a move (the retirement rule). Not part of the
    /// encoded format: [`decode`] rebuilds it by scanning.
    origin: [Option<(u16, u16)>; N_WINDOWS],
}

impl PaintLog {
    /// Fold one [`PaintEvent`] into the log. `Machine`'s own doing — see the
    /// module docs' "Feeding it".
    pub(crate) fn apply(&mut self, ev: &PaintEvent) {
        match ev {
            PaintEvent::Erase(fill) => {
                let win = fill.window as usize;
                if win >= N_WINDOWS {
                    return;
                }
                self.set_clear(win, (fill.x, fill.y, fill.w, fill.h));
            }
            PaintEvent::Picture(p) => {
                let win = p.window as usize;
                if win >= N_WINDOWS {
                    return;
                }
                if p.erase && p.number == 0 {
                    // The paired canvas-clear signal for the SAME erase_window
                    // call the EraseFill above already folded (or will fold) —
                    // idempotent, so whichever of the two arrives first wins and
                    // the second changes nothing.
                    self.set_clear(win, p.win_box);
                    return;
                }
                self.record(win, p);
            }
        }
    }

    fn set_clear(&mut self, win: usize, win_box: (u16, u16, u16, u16)) {
        self.entries.retain(|(w, _)| *w as usize != win);
        self.entries.push((win as u8, PaintOp::Clear { win_box }));
        self.origin[win] = Some((win_box.0, win_box.1));
    }

    fn record(&mut self, win: usize, ev: &crate::cpu::exec::PictureEvent) {
        let new_origin = (ev.win_box.0, ev.win_box.1);
        if self.origin[win] != Some(new_origin) {
            // The window has moved since the log's last op for it (or this is
            // the first op ever) — what is recorded describes pixels at an
            // origin nothing on screen occupies any more (ZMSD §8).
            self.entries.retain(|(w, _)| *w as usize != win);
            self.origin[win] = Some(new_origin);
        }
        if self.count(win as u8) >= PAINT_LOG_CAP {
            return;
        }
        let op = if ev.erase {
            PaintOp::ErasePicture {
                number: ev.number,
                x: ev.x,
                y: ev.y,
                win_box: ev.win_box,
                at_cursor: ev.at_cursor,
                margin_after: ev.margin_after,
                out_chars: ev.out_chars,
            }
        } else {
            PaintOp::Draw {
                number: ev.number,
                x: ev.x,
                y: ev.y,
                win_box: ev.win_box,
                at_cursor: ev.at_cursor,
                margin_after: ev.margin_after,
                out_chars: ev.out_chars,
            }
        };
        self.entries.push((win as u8, op));
    }

    fn count(&self, win: u8) -> usize {
        self.entries.iter().filter(|(w, _)| *w == win).count()
    }

    /// One window's folded op list, oldest first. Empty for a window nothing
    /// has drawn to (or restart/whole-window-clear last touched). Equivalent
    /// to filtering [`Self::ops_in_order`] to one window.
    pub fn ops(&self, win: u8) -> Vec<PaintOp> {
        self.entries.iter().filter(|(w, _)| *w == win).map(|(_, op)| *op).collect()
    }

    /// Every surviving op, across every window, in the TRUE order the story
    /// issued them — the one thing a per-window view cannot give a host: a
    /// cross-window erase mirrored while replaying window 2's entry must
    /// land relative to window 7's OWN draws exactly where it truly occurred,
    /// not before all of them or after all of them. A host rebuilding canvas
    /// state walks this, in order, through the SAME functions live rendering
    /// uses. See the module docs.
    pub fn ops_in_order(&self) -> &[(u8, PaintOp)] {
        &self.entries
    }

    /// Whether `win`'s entries have hit [`PAINT_LOG_CAP`] — a host's signal
    /// that replaying them will not reproduce every op the story issued, so
    /// it should fall back to a pixel snapshot for this window rather than
    /// trust the (necessarily incomplete) list.
    pub fn is_capped(&self, win: u8) -> bool {
        self.count(win) >= PAINT_LOG_CAP
    }

    /// Drop every window's entries. `Machine::restart` calls this in the
    /// same breath it clears the paint queues themselves (ZMSD §6.1.3).
    pub(crate) fn clear_all(&mut self) {
        self.entries.clear();
        self.origin = [None; N_WINDOWS];
    }

    /// Mirror, on the log's own copy of the last `draw_picture` for `win`, the
    /// SAME retroactive `margin_after` attachment `Machine` makes to its
    /// pending picture queue when a `set_margins` follows a `draw_picture`
    /// directly (ZMSD §15's inline-picture idiom): the log's entry for that
    /// draw was already fed by the time this runs, so without this call it
    /// would keep the `None` it had at draw time forever, and a host trying
    /// to tell an inline float from a canvas picture at replay time needs the
    /// SAME `margin_after` a live drain sees. Only the log's LAST entry is
    /// eligible, and only a `Draw` whose `margin_after` is still unset — the
    /// exact condition the pending-queue mutation uses, so the two can never
    /// disagree about which draw a `set_margins` was really about.
    pub(crate) fn set_margin_after(&mut self, win: u8, margin: u16) {
        let Some((w, op)) = self.entries.last_mut() else { return };
        if *w != win {
            return;
        }
        if let PaintOp::Draw { margin_after, .. } = op {
            if margin_after.is_none() {
                *margin_after = Some(margin);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Encoding — hand-rolled and versioned, in `screen_snapshot.rs`'s style:
// dependency-free, big-endian, a magic + version header, a GREATER version
// refused outright, and every declared count checked against the remaining
// buffer before it is allocated for.
// ---------------------------------------------------------------------------

const MAGIC: &[u8; 4] = b"ZPNT";
/// The format version this build writes, and the highest it can read.
pub const VERSION: u16 = 1;

const OP_DRAW: u8 = 0;
const OP_ERASE_PICTURE: u8 = 1;
const OP_CLEAR: u8 = 2;

/// Bytes one encoded entry occupies at minimum (a `Clear`, the smallest
/// variant): window tag + op tag + 4 x u16 win_box.
const MIN_ENTRY_BYTES: usize = 1 + 1 + 4 * 2;

fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_be_bytes());
}

/// Serialise a paint log to a versioned byte buffer. Entries travel in
/// [`PaintLog::ops_in_order`]'s order — the format IS that order, so
/// [`decode`] needs nothing beyond it to rebuild everything, including each
/// window's origin (see [`decode`]).
pub fn encode(log: &PaintLog) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    put_u16(&mut out, VERSION);
    put_u32(&mut out, log.entries.len() as u32);
    for (win, op) in &log.entries {
        out.push(*win);
        put_op(&mut out, op);
    }
    out
}

/// Rebuild a paint log from a buffer written by [`encode`].
///
/// Each window's origin is NOT stored — it is re-derived by scanning the
/// entries in order and taking the last `win_box` seen for that window,
/// which is exactly what [`PaintLog::apply`] would have left it at.
///
/// # Errors
///
/// [`ZError::PaintLogVersion`] when the buffer was written by a NEWER format
/// version than this build understands, and [`ZError::BadPaintLog`] when it
/// is not a paint log at all, is truncated, or is otherwise unreadable.
pub fn decode(bytes: &[u8]) -> Result<PaintLog, ZError> {
    let mut r = Reader { b: bytes, i: 0 };
    if r.take(4)? != MAGIC {
        return Err(ZError::BadPaintLog);
    }
    let version = r.u16()?;
    if version > VERSION {
        return Err(ZError::PaintLogVersion { found: version, supported: VERSION });
    }
    let n = r.count(MIN_ENTRY_BYTES)?;
    let mut entries = Vec::with_capacity(n);
    let mut origin = [None; N_WINDOWS];
    for _ in 0..n {
        let win = r.u8()?;
        let op = r.op()?;
        if (win as usize) < N_WINDOWS {
            let wb = op.win_box();
            origin[win as usize] = Some((wb.0, wb.1));
        }
        entries.push((win, op));
    }
    Ok(PaintLog { entries, origin })
}

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn put_win_box(out: &mut Vec<u8>, b: (u16, u16, u16, u16)) {
    put_u16(out, b.0);
    put_u16(out, b.1);
    put_u16(out, b.2);
    put_u16(out, b.3);
}

fn put_opt_u16(out: &mut Vec<u8>, v: Option<u16>) {
    match v {
        Some(v) => {
            out.push(1);
            put_u16(out, v);
        }
        None => out.push(0),
    }
}

fn put_op(out: &mut Vec<u8>, op: &PaintOp) {
    match *op {
        PaintOp::Draw { number, x, y, win_box, at_cursor, margin_after, out_chars } => {
            out.push(OP_DRAW);
            put_u16(out, number);
            put_u16(out, x);
            put_u16(out, y);
            put_win_box(out, win_box);
            out.push(u8::from(at_cursor));
            put_opt_u16(out, margin_after);
            put_u64(out, out_chars);
        }
        PaintOp::ErasePicture { number, x, y, win_box, at_cursor, margin_after, out_chars } => {
            out.push(OP_ERASE_PICTURE);
            put_u16(out, number);
            put_u16(out, x);
            put_u16(out, y);
            put_win_box(out, win_box);
            out.push(u8::from(at_cursor));
            put_opt_u16(out, margin_after);
            put_u64(out, out_chars);
        }
        PaintOp::Clear { win_box } => {
            out.push(OP_CLEAR);
            put_win_box(out, win_box);
        }
    }
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ZError> {
        let end = self.i.checked_add(n).ok_or(ZError::BadPaintLog)?;
        let s = self.b.get(self.i..end).ok_or(ZError::BadPaintLog)?;
        self.i = end;
        Ok(s)
    }

    fn remaining(&self) -> usize {
        self.b.len().saturating_sub(self.i)
    }

    /// A declared element count, refused before it is allocated for when the
    /// rest of the buffer could not possibly hold that many (SQ-0647's
    /// discipline, ported from `screen_snapshot.rs`).
    fn count(&mut self, min_element_bytes: usize) -> Result<usize, ZError> {
        let n = self.u32()? as usize;
        if n.saturating_mul(min_element_bytes) > self.remaining() {
            return Err(ZError::BadPaintLog);
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

    fn u64(&mut self) -> Result<u64, ZError> {
        let s = self.take(8)?;
        Ok(u64::from_be_bytes(s.try_into().expect("take(8) returns 8 bytes")))
    }

    fn win_box(&mut self) -> Result<(u16, u16, u16, u16), ZError> {
        Ok((self.u16()?, self.u16()?, self.u16()?, self.u16()?))
    }

    fn opt_u16(&mut self) -> Result<Option<u16>, ZError> {
        Ok(match self.u8()? {
            0 => None,
            _ => Some(self.u16()?),
        })
    }

    fn op(&mut self) -> Result<PaintOp, ZError> {
        let tag = self.u8()?;
        Ok(match tag {
            OP_DRAW => {
                let number = self.u16()?;
                let x = self.u16()?;
                let y = self.u16()?;
                let win_box = self.win_box()?;
                let at_cursor = self.u8()? != 0;
                let margin_after = self.opt_u16()?;
                let out_chars = self.u64()?;
                PaintOp::Draw { number, x, y, win_box, at_cursor, margin_after, out_chars }
            }
            OP_ERASE_PICTURE => {
                let number = self.u16()?;
                let x = self.u16()?;
                let y = self.u16()?;
                let win_box = self.win_box()?;
                let at_cursor = self.u8()? != 0;
                let margin_after = self.opt_u16()?;
                let out_chars = self.u64()?;
                PaintOp::ErasePicture { number, x, y, win_box, at_cursor, margin_after, out_chars }
            }
            OP_CLEAR => {
                let win_box = self.win_box()?;
                PaintOp::Clear { win_box }
            }
            _ => return Err(ZError::BadPaintLog),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::exec::{EraseFill, PictureEvent};
    use crate::screen::ZColour;

    fn draw(number: u16, x: u16, y: u16, win_box: (u16, u16, u16, u16)) -> PaintEvent {
        PaintEvent::Picture(PictureEvent::new(number, 0, x, y, false, 0, None, false, win_box))
    }

    fn draw_win(win: u8, number: u16, x: u16, y: u16, win_box: (u16, u16, u16, u16)) -> PaintEvent {
        PaintEvent::Picture(PictureEvent::new(number, win, x, y, false, 0, None, false, win_box))
    }

    fn erase_pic(number: u16, x: u16, y: u16, win_box: (u16, u16, u16, u16)) -> PaintEvent {
        PaintEvent::Picture(PictureEvent::new(number, 0, x, y, true, 0, None, false, win_box))
    }

    fn window_clear_pair(win_box: (u16, u16, u16, u16)) -> [PaintEvent; 2] {
        let (x, y, w, h) = win_box;
        [
            PaintEvent::Erase(EraseFill::new(0, x, y, w, h, ZColour::Default, 0)),
            PaintEvent::Picture(PictureEvent::new(0, 0, 1, 1, true, 0, None, true, win_box)),
        ]
    }

    fn window_clear_pair_win(win: u8, win_box: (u16, u16, u16, u16)) -> [PaintEvent; 2] {
        let (x, y, w, h) = win_box;
        [
            PaintEvent::Erase(EraseFill::new(win, x, y, w, h, ZColour::Default, 0)),
            PaintEvent::Picture(PictureEvent::new(0, win, 1, 1, true, 0, None, true, win_box)),
        ]
    }

    #[test]
    fn fill_draw_draw_clear_draw_folds_to_clear_plus_one_draw() {
        let mut log = PaintLog::default();
        let win_box = (1, 1, 100, 100);
        // "fill" — a whole-window clear opens the sequence.
        for ev in window_clear_pair(win_box) {
            log.apply(&ev);
        }
        log.apply(&draw(1, 10, 10, win_box));
        log.apply(&draw(2, 20, 20, win_box));
        // "whole-window clear" mid-sequence.
        for ev in window_clear_pair(win_box) {
            log.apply(&ev);
        }
        log.apply(&draw(3, 30, 30, win_box));

        assert_eq!(
            log.ops(0),
            vec![
                PaintOp::Clear { win_box },
                PaintOp::Draw { number: 3, x: 30, y: 30, win_box, at_cursor: false, margin_after: None, out_chars: 0 },
            ]
        );
    }

    #[test]
    fn one_erase_window_call_folds_its_paired_events_into_one_entry() {
        let mut log = PaintLog::default();
        let win_box = (5, 5, 50, 60);
        for ev in window_clear_pair(win_box) {
            log.apply(&ev);
        }
        assert_eq!(log.ops(0), vec![PaintOp::Clear { win_box }]);
    }

    #[test]
    fn erase_window_minus_one_clears_every_window() {
        let mut log = PaintLog::default();
        for win in 0u8..8 {
            let win_box = (1, 1, u16::from(win) + 1, u16::from(win) + 1);
            for ev in window_clear_pair_win(win, win_box) {
                log.apply(&ev);
            }
        }
        for win in 0u8..8 {
            assert_eq!(log.ops(win).len(), 1, "window {win}");
        }
    }

    #[test]
    fn a_window_move_strands_the_earlier_ops() {
        let mut log = PaintLog::default();
        let here = (1, 1, 100, 100);
        log.apply(&draw(1, 10, 10, here));
        log.apply(&draw(2, 20, 20, here));
        assert_eq!(log.ops(0).len(), 2);

        let moved = (50, 50, 100, 100);
        log.apply(&draw(3, 5, 5, moved));
        assert_eq!(
            log.ops(0),
            vec![PaintOp::Draw { number: 3, x: 5, y: 5, win_box: moved, at_cursor: false, margin_after: None, out_chars: 0 }],
            "the pre-move draws described pixels nothing on screen occupies any more"
        );
    }

    #[test]
    fn a_redraw_in_place_does_not_strand() {
        let mut log = PaintLog::default();
        let win_box = (1, 1, 100, 100);
        log.apply(&draw(1, 10, 10, win_box));
        log.apply(&draw(2, 10, 10, win_box));
        assert_eq!(log.ops(0).len(), 2, "same origin — both draws survive");
    }

    #[test]
    fn erase_picture_is_recorded_without_a_footprint() {
        let mut log = PaintLog::default();
        let win_box = (1, 1, 100, 100);
        log.apply(&erase_pic(7, 12, 34, win_box));
        assert_eq!(
            log.ops(0),
            vec![PaintOp::ErasePicture { number: 7, x: 12, y: 34, win_box, at_cursor: false, margin_after: None, out_chars: 0 }]
        );
    }

    #[test]
    fn other_windows_are_untouched_by_one_windows_events() {
        let mut log = PaintLog::default();
        let win_box = (1, 1, 10, 10);
        log.apply(&draw_win(3, 9, 1, 1, win_box));
        assert_eq!(log.ops(3).len(), 1);
        for win in (0u8..8).filter(|&w| w != 3) {
            assert!(log.ops(win).is_empty(), "window {win}");
        }
    }

    #[test]
    fn the_cap_stops_growth_until_the_next_clear() {
        let mut log = PaintLog::default();
        let win_box = (1, 1, 10, 10);
        for i in 0..PAINT_LOG_CAP + 50 {
            log.apply(&draw(i as u16, 1, 1, win_box));
        }
        assert_eq!(log.ops(0).len(), PAINT_LOG_CAP, "further ops are dropped once capped");
        assert!(log.is_capped(0));

        for ev in window_clear_pair(win_box) {
            log.apply(&ev);
        }
        assert!(!log.is_capped(0), "a whole-window clear resets the cap");
        assert_eq!(log.ops(0), vec![PaintOp::Clear { win_box }]);
    }

    #[test]
    fn clear_all_empties_every_window() {
        let mut log = PaintLog::default();
        for win in 0u8..8 {
            log.apply(&draw_win(win, 1, 1, 1, (1, 1, 10, 10)));
        }
        log.clear_all();
        for win in 0u8..8 {
            assert!(log.ops(win).is_empty(), "window {win}");
        }
    }

    #[test]
    fn an_out_of_range_window_is_ignored_rather_than_panicking() {
        let mut log = PaintLog::default();
        log.apply(&PaintEvent::Erase(EraseFill::new(200, 1, 1, 10, 10, ZColour::Default, 0)));
        // No panic is the assertion; every real window is still empty.
        for win in 0u8..8 {
            assert!(log.ops(win).is_empty());
        }
    }

    /// The property the flat, globally-ordered log exists for: a whole-window
    /// clear on window 2, issued BETWEEN two draws into window 7, must land
    /// between them in [`PaintLog::ops_in_order`] — not before both (as
    /// feeding-at-drain-time with a per-window store would put it) and not
    /// after both.
    #[test]
    fn ops_in_order_preserves_true_cross_window_interleave() {
        let mut log = PaintLog::default();
        let wb7 = (1, 1, 640, 400);
        let wb2 = (29, 1, 584, 192);
        log.apply(&draw_win(7, 54, 5, 12, wb7)); // 1: window 7 draws its backdrop
        for ev in window_clear_pair_win(2, wb2) {
            log.apply(&ev); // 2: window 2 clears (between the two window-7 draws)
        }
        log.apply(&draw_win(7, 137, 1, 1, wb7)); // 3: window 7 draws again

        let order = log.ops_in_order();
        let pos = |pred: &dyn Fn(&(u8, PaintOp)) -> bool| {
            order.iter().position(pred).unwrap_or_else(|| panic!("missing in {order:?}"))
        };
        let first_draw = pos(&|(w, op)| *w == 7 && matches!(op, PaintOp::Draw { number: 54, .. }));
        let clear2 = pos(&|(w, op)| *w == 2 && matches!(op, PaintOp::Clear { .. }));
        let second_draw = pos(&|(w, op)| *w == 7 && matches!(op, PaintOp::Draw { number: 137, .. }));
        assert!(
            first_draw < clear2 && clear2 < second_draw,
            "window 2's clear must sit strictly between window 7's two draws: {order:?}"
        );
    }

    // -- encode/decode -------------------------------------------------------

    fn sample_log() -> PaintLog {
        let mut log = PaintLog::default();
        let win_box = (1, 1, 100, 100);
        log.apply(&draw(1, 10, 10, win_box));
        log.apply(&erase_pic(2, 20, 20, win_box));
        for win in 1u8..4 {
            let wb = (u16::from(win), u16::from(win), 40, 50);
            for ev in window_clear_pair_win(win, wb) {
                log.apply(&ev);
            }
            log.apply(&draw_win(win, win as u16, 3, 3, wb));
        }
        log
    }

    #[test]
    fn a_log_round_trips() {
        let src = sample_log();
        let back = decode(&encode(&src)).expect("decodes");
        assert_eq!(src, back);
    }

    #[test]
    fn at_cursor_and_margin_after_round_trip() {
        let mut log = PaintLog::default();
        let win_box = (1, 1, 100, 100);
        log.apply(&PaintEvent::Picture(PictureEvent::new(
            1, 0, 5, 5, false, 0, Some(96), true, win_box,
        )));
        log.apply(&PaintEvent::Picture(PictureEvent::new(
            2, 0, 6, 6, true, 0, None, false, win_box,
        )));
        assert_eq!(
            log.ops(0),
            vec![
                PaintOp::Draw { number: 1, x: 5, y: 5, win_box, at_cursor: true, margin_after: Some(96), out_chars: 0 },
                PaintOp::ErasePicture { number: 2, x: 6, y: 6, win_box, at_cursor: false, margin_after: None, out_chars: 0 },
            ]
        );
        let back = decode(&encode(&log)).expect("decodes");
        assert_eq!(log, back, "at_cursor and margin_after must survive encode/decode");
    }

    /// `out_chars` is the count AT THE CALL, frozen into the entry the moment
    /// it is recorded — not a value a host recomputes later from its own
    /// running counter. Two draws with the SAME `win`/geometry but different
    /// `out_chars` must keep their own counts distinct, and the value must
    /// survive encode/decode exactly (mysterious01's title card carries 0
    /// here while the session's live counter has already moved past it by
    /// the time anything reads the log — conflating the two misclassifies
    /// the very picture this field exists to classify correctly).
    #[test]
    fn out_chars_is_the_value_at_the_call_not_a_running_count() {
        let mut log = PaintLog::default();
        let win_box = (1, 1, 640, 400);
        log.apply(&PaintEvent::Picture(PictureEvent::new(33, 0, 1, 1, false, 0, None, true, win_box)));
        log.apply(&PaintEvent::Picture(PictureEvent::new(34, 0, 1, 192, false, 24, None, false, win_box)));
        assert_eq!(
            log.ops(0),
            vec![
                PaintOp::Draw { number: 33, x: 1, y: 1, win_box, at_cursor: true, margin_after: None, out_chars: 0 },
                PaintOp::Draw { number: 34, x: 1, y: 192, win_box, at_cursor: false, margin_after: None, out_chars: 24 },
            ],
            "each entry keeps its OWN out_chars, not the log's most recent one"
        );
        let back = decode(&encode(&log)).expect("decodes");
        assert_eq!(log, back, "out_chars must survive encode/decode");
    }

    /// `Machine`'s own `set_margins` opcode handler retroactively attaches
    /// `margin_after` to the pending picture it directly follows — AFTER that
    /// picture's log entry was already fed. Without `set_margin_after`
    /// mirroring the same attachment on the log, the log's copy would keep
    /// `None` forever, silently different from what a live drain of the same
    /// turn sees on the SAME event.
    #[test]
    fn set_margin_after_reaches_the_logs_own_copy_of_the_draw() {
        let mut log = PaintLog::default();
        let win_box = (1, 1, 100, 100);
        log.apply(&draw(1, 5, 5, win_box));
        log.set_margin_after(0, 96);
        assert_eq!(
            log.ops(0),
            vec![PaintOp::Draw { number: 1, x: 5, y: 5, win_box, at_cursor: false, margin_after: Some(96), out_chars: 0 }],
        );
    }

    #[test]
    fn set_margin_after_does_not_overwrite_an_already_set_margin() {
        let mut log = PaintLog::default();
        let win_box = (1, 1, 100, 100);
        log.apply(&PaintEvent::Picture(PictureEvent::new(1, 0, 5, 5, false, 0, Some(10), false, win_box)));
        log.set_margin_after(0, 96);
        assert_eq!(
            log.ops(0),
            vec![PaintOp::Draw { number: 1, x: 5, y: 5, win_box, at_cursor: false, margin_after: Some(10), out_chars: 0 }],
            "a margin the draw already carried is not replaced"
        );
    }

    #[test]
    fn set_margin_after_ignores_a_different_windows_last_entry() {
        let mut log = PaintLog::default();
        let win_box = (1, 1, 100, 100);
        log.apply(&draw_win(3, 1, 5, 5, win_box));
        log.set_margin_after(0, 96); // window 0 has no entries at all
        assert!(log.ops(0).is_empty());
        assert_eq!(
            log.ops(3),
            vec![PaintOp::Draw { number: 1, x: 5, y: 5, win_box, at_cursor: false, margin_after: None, out_chars: 0 }],
            "window 3's draw is untouched by a set_margins naming window 0"
        );
    }

    #[test]
    fn set_margin_after_does_not_touch_an_erase_picture() {
        let mut log = PaintLog::default();
        let win_box = (1, 1, 100, 100);
        log.apply(&erase_pic(1, 5, 5, win_box));
        log.set_margin_after(0, 96);
        assert_eq!(
            log.ops(0),
            vec![PaintOp::ErasePicture { number: 1, x: 5, y: 5, win_box, at_cursor: false, margin_after: None, out_chars: 0 }],
            "the pending-queue mutation this mirrors only ever touches a draw (`!ev.erase`)"
        );
    }

    #[test]
    fn issue_order_survives_encode_decode() {
        let src = sample_log();
        let back = decode(&encode(&src)).expect("decodes");
        assert_eq!(
            src.ops_in_order(),
            back.ops_in_order(),
            "the encoded form IS issue order, so decode must reproduce it exactly"
        );
    }

    #[test]
    fn an_empty_log_round_trips() {
        let src = PaintLog::default();
        let back = decode(&encode(&src)).expect("decodes");
        assert_eq!(src, back);
    }

    #[test]
    fn every_truncation_is_an_error_and_not_a_panic() {
        let full = encode(&sample_log());
        for n in 0..full.len() {
            assert_eq!(decode(&full[..n]).unwrap_err(), ZError::BadPaintLog, "truncated to {n}");
        }
        assert!(decode(&full).is_ok(), "the untruncated blob still decodes");
    }

    #[test]
    fn a_foreign_buffer_is_refused() {
        assert_eq!(decode(b"").unwrap_err(), ZError::BadPaintLog);
        assert_eq!(decode(b"ZSCR\x00\x01").unwrap_err(), ZError::BadPaintLog);
    }

    #[test]
    fn a_newer_version_is_refused_and_names_both_numbers() {
        let mut blob = encode(&sample_log());
        blob[4..6].copy_from_slice(&(VERSION + 3).to_be_bytes());
        assert_eq!(
            decode(&blob).unwrap_err(),
            ZError::PaintLogVersion { found: VERSION + 3, supported: VERSION }
        );
    }

    #[test]
    fn a_corrupt_entry_count_is_an_error_rather_than_an_allocation() {
        // The entry count is the four bytes right after magic(4) + version(2).
        let mut blob = encode(&PaintLog::default());
        let at = 4 + 2;
        blob[at..at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(decode(&blob).unwrap_err(), ZError::BadPaintLog);
    }
}
