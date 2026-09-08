//! A per-window fold of the Version 6 paint stream — what a host replays to
//! rebuild a window's picture canvas without the story's help.
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
//! what is on screen right now, per window: everything since the window's
//! last whole-canvas clear, with the window's later moves not moving what
//! was already painted (ZMSD §8: "subsequent movements of the window do not
//! move what was printed").
//!
//! Folding the stream down to that set is pure Z-machine bookkeeping — it
//! reads only [`crate::cpu::exec::PaintEvent`] geometry, never a decoded
//! picture — so it belongs here rather than in every host that wants it, and
//! two embedders cannot fold the stream two different ways.
//!
//! # Feeding it
//!
//! [`PaintLog::apply`] is NOT called automatically as events are queued —
//! deliberately, and the reason is ordering. A cross-window erase
//! ([`PaintOp::HostErase`]) can only be computed by a host that models
//! per-window canvases, from state ([`PaintLog::apply`] cannot see) that only
//! exists once the host is walking its OWN drained turn; feeding engine
//! events into the log the instant an opcode runs would put every such
//! cross-window entry after every engine event from the SAME turn, however
//! interleaved they truly were, because the whole turn's engine events would
//! already be queued before the host ever computes its first cross-window
//! rect. The fix is not to feed early — it is to feed once, from the SAME
//! walk that computes the cross-window entries: call
//! [`crate::cpu::exec::Machine::take_paint_events`], and for each drained
//! [`crate::cpu::exec::PaintEvent`] call `paint_log_mut().apply(&event)`
//! immediately before (or after — anywhere in the same iteration works)
//! rasterizing it, so any [`PaintLog::append_host_erase`] call that
//! rasterizing triggers lands at exactly this point in this window's own
//! timeline. Two embedders still cannot fold the stream two different ways —
//! the fold algorithm is here — but a host that never drains
//! `take_paint_events` (and therefore never renders anything) also never
//! feeds the log, which costs it nothing it was going to use anyway.
//!
//! # What is deliberately NOT in it
//!
//! **No rasterization.** A [`PaintOp`] names a picture NUMBER, a window box,
//! and window-relative native-pixel coordinates — never a decoded image,
//! never a canvas, never a resolved colour. Turning that into pixels needs
//! the story's own resource archive, which is the host's to hold.
//!
//! **`zvm` never COMPUTES a cross-window erasure.** ZMSD §8 describes one
//! shared screen; a host that models eight independent per-window canvases
//! (as `lanthorn` does) must mirror an erase into every OTHER canvas it
//! overlaps, in that canvas's own local coordinates — which rect that is is a
//! fact about the host's own canvas model, not about the Z-machine, and this
//! module has no opinion on it. What it DOES do is hold the result in the
//! right place: [`PaintLog::append_host_erase`] lets a host record that
//! already-computed rect directly into the affected window's own op list, so
//! it lands in the SAME timeline as that window's own draws rather than a
//! side channel with no ordering relationship to them. Order is what makes
//! this correct rather than merely convenient — see
//! [`PaintOp::HostErase`]'s docs.
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
//! A window's op list is not append-only; four rules keep it bounded and
//! keep it standing for the CURRENT screen rather than the whole session:
//!
//! - **A whole-window clear resets the list to one entry.** `erase_window`
//!   (ZMSD §8.7.3.3, any target — a single window, `-1`, or `-2`) fills the
//!   window with its background, i.e. paints over everything drawn since the
//!   last clear; everything before it is dead. `Machine` emits BOTH an
//!   [`crate::cpu::exec::EraseFill`] and a `PictureEvent { number: 0, erase:
//!   true }` for the same call — [`PaintLog::apply`] folds both into the
//!   SAME single [`PaintOp::Clear`] entry (the second application is a no-op
//!   over the first), never two.
//! - **A window move strands the list.** ZMSD §8: pixels already plotted do
//!   not move when the window does. So the moment a draw or erase-picture
//!   event's window box reports an origin different from the log's
//!   currently-tracked one for that window, the window's list — which
//!   describes pixels at the OLD origin — is no longer a recipe for
//!   anything on screen and is dropped before the new op is recorded.
//! - **`Machine::restart` clears every window's list** (ZMSD §6.1.3's
//!   reboot), in the same breath as the paint queues themselves — no window
//!   survives a restart holding a pre-restart recipe.
//! - **A window's list is capped** at [`PAINT_LOG_CAP`] entries. A story
//!   that only ever draws and never clears would otherwise grow its list for
//!   the life of the session; once a window hits the cap, further ops for it
//!   are dropped until the next whole-window clear (which resets the count
//!   to one) or restart. A host reading [`PaintLog::ops`] sees exactly
//!   [`PAINT_LOG_CAP`] entries for a capped window and knows replay will not
//!   reproduce it — precisely the shape `lanthorn`'s own `V6_OPS_CAP` used to
//!   detect for its fallback-to-PNG decision.

use crate::cpu::exec::PaintEvent;
use crate::error::ZError;

/// Number of Version 6 windows (ZMSD §8.4).
const N_WINDOWS: usize = 8;

/// Longest op list kept per window before further ops are dropped (see the
/// module docs' retirement rules). Matches `lanthorn`'s prior `V6_OPS_CAP`,
/// comfortably above any real screen's redraw history.
pub const PAINT_LOG_CAP: usize = 512;

/// One entry in a window's folded paint history, in window-relative NATIVE
/// pixels (1-based, exactly as the Z-machine opcodes give them) — no cell,
/// canvas, or terminal coordinate of any kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PaintOp {
    /// A `draw_picture` (ZMSD §15). `x`/`y` are the pixel coordinates (of the
    /// picture's top-left corner) within the window; `win_box` is the
    /// window's own box — `(x, y, w, h)` in native pixels — at the moment of
    /// the call.
    Draw { number: u16, x: u16, y: u16, win_box: (u16, u16, u16, u16) },
    /// An `erase_picture` (ZMSD §15). Same fields as [`Self::Draw`]; the
    /// picture's footprint is not known here — see the module docs.
    ErasePicture { number: u16, x: u16, y: u16, win_box: (u16, u16, u16, u16) },
    /// The whole-window clear an `erase_window` paints, folded from the
    /// paired [`crate::cpu::exec::EraseFill`] and canvas-clear `PictureEvent`
    /// `Machine` emits for one call — see the module docs' first retirement
    /// rule. `win_box` is the window's box at erase time.
    Clear { win_box: (u16, u16, u16, u16) },
    /// NOT a Z-machine event — appended directly by the host via
    /// [`PaintLog::append_host_erase`], never via [`PaintLog::apply`]. ZMSD §8
    /// describes one shared screen; a host that models each window as an
    /// independent canvas (as `lanthorn` does) must mirror one window's erase
    /// into every OTHER window it overlaps, in whatever LOCAL rect that other
    /// window's own canvas uses. `zvm` has no opinion on that host canvas
    /// model and cannot compute the rect — but it is still the one place that
    /// holds this window's ops in the right ORDER relative to its own draws,
    /// which is what makes a later replay correct: an erase recorded out of
    /// order would erase pixels the window had not been drawn onto yet, or
    /// fail to erase ones it had. `dx`/`dy`/`w`/`h` are already the host's own
    /// local, canvas-relative rect — nothing here to re-derive.
    HostErase { dx: i32, dy: i32, w: u32, h: u32 },
}

/// One window's folded paint history.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct WindowLog {
    /// The window's origin `(x, y)` as of the most recently recorded op —
    /// `None` until the first op. Compared against each new event's window
    /// box to detect a move (the second retirement rule).
    origin: Option<(u16, u16)>,
    ops: Vec<PaintOp>,
}

/// The Version 6 paint log: one [`WindowLog`] per window (0..8), fed
/// automatically as `Machine` queues paint events. See the module docs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaintLog {
    windows: [WindowLog; N_WINDOWS],
}

impl PaintLog {
    /// Fold one [`PaintEvent`] into the log — a host calls this once per
    /// event drained from [`crate::cpu::exec::Machine::take_paint_events`],
    /// in the SAME walk it rasterizes each one; see the module docs' "Feeding
    /// it" section for why that walk, and not opcode-execution time, is where
    /// this belongs.
    pub fn apply(&mut self, ev: &PaintEvent) {
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
        let w = &mut self.windows[win];
        w.origin = Some((win_box.0, win_box.1));
        w.ops.clear();
        w.ops.push(PaintOp::Clear { win_box });
    }

    fn record(&mut self, win: usize, ev: &crate::cpu::exec::PictureEvent) {
        let w = &mut self.windows[win];
        let origin = (ev.win_box.0, ev.win_box.1);
        if w.origin != Some(origin) {
            // The window has moved since the log's last op for it (or this is
            // the first op ever) — what is recorded describes pixels at an
            // origin nothing on screen occupies any more (ZMSD §8).
            w.ops.clear();
            w.origin = Some(origin);
        }
        if w.ops.len() >= PAINT_LOG_CAP {
            return;
        }
        w.ops.push(if ev.erase {
            PaintOp::ErasePicture { number: ev.number, x: ev.x, y: ev.y, win_box: ev.win_box }
        } else {
            PaintOp::Draw { number: ev.number, x: ev.x, y: ev.y, win_box: ev.win_box }
        });
    }

    /// Append a host-computed cross-window erase to `win`'s own op list — see
    /// [`PaintOp::HostErase`] for why this exists and why order matters. A
    /// call for an out-of-range window is ignored, exactly as [`Self::apply`]
    /// ignores one; the window's own cap still applies, and a call past it is
    /// dropped exactly as a real event past the cap is.
    ///
    /// Does NOT touch the window's tracked origin: this is not a window-box
    /// event, so it cannot move the window and cannot itself trigger the
    /// stranding rule — but a LATER draw or erase-picture that does still
    /// clears it away with everything else, because it is stored in the same
    /// list.
    ///
    /// Prunes earlier [`PaintOp::HostErase`] entries the new rect fully
    /// contains before appending (SQ-0592's rule, ported): a rect painted
    /// with the SAME background twice contributes nothing the second time,
    /// and a host that re-erases the same region every turn — Shogun's
    /// caret, reached only through this cross-window path since it is
    /// `erase_window`, never `erase_picture` — would otherwise grow this
    /// list once per turn for the life of the session. Nothing else is
    /// pruned: an earlier Draw or ErasePicture under the same rect is dead
    /// too, but proving it needs a footprint this op does not carry, and
    /// keeping it is harmless — replay order is preserved, so this erase
    /// still covers it.
    pub fn append_host_erase(&mut self, win: u8, dx: i32, dy: i32, w: u32, h: u32) {
        let Some(win) = self.windows.get_mut(win as usize) else { return };
        win.ops.retain(|prev| match *prev {
            PaintOp::HostErase { dx: pdx, dy: pdy, w: pw, h: ph } => {
                let inside = pdx >= dx
                    && pdy >= dy
                    && pdx.saturating_add(pw as i32) <= dx.saturating_add(w as i32)
                    && pdy.saturating_add(ph as i32) <= dy.saturating_add(h as i32);
                !inside
            }
            _ => true,
        });
        if win.ops.len() >= PAINT_LOG_CAP {
            return;
        }
        win.ops.push(PaintOp::HostErase { dx, dy, w, h });
    }

    /// One window's folded op list, oldest first. Empty for a window nothing
    /// has drawn to (or restart/whole-window-clear last touched).
    pub fn ops(&self, win: u8) -> &[PaintOp] {
        match self.windows.get(win as usize) {
            Some(w) => &w.ops,
            None => &[],
        }
    }

    /// Whether `win`'s list has hit [`PAINT_LOG_CAP`] — a host's signal that
    /// replaying it will not reproduce every op the story issued, so it
    /// should fall back to a pixel snapshot for this window rather than
    /// trust the (necessarily incomplete) list.
    pub fn is_capped(&self, win: u8) -> bool {
        self.windows.get(win as usize).is_some_and(|w| w.ops.len() >= PAINT_LOG_CAP)
    }

    /// Drop one window's history — as if it had just been whole-canvas
    /// cleared, but with nothing left standing at all (not even a
    /// [`PaintOp::Clear`]). A host reaches for this only in the same
    /// situations it would have dropped its own replay state; ordinary play
    /// never needs it, since a real `erase_window` already folds to
    /// [`PaintOp::Clear`] via [`Self::apply`].
    pub fn clear(&mut self, win: u8) {
        if let Some(w) = self.windows.get_mut(win as usize) {
            *w = WindowLog::default();
        }
    }

    /// Drop every window's history. `Machine::restart` calls this in the
    /// same breath it clears the paint queues themselves (ZMSD §6.1.3).
    pub fn clear_all(&mut self) {
        self.windows = Default::default();
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
const OP_HOST_ERASE: u8 = 3;

/// Bytes one encoded op occupies at minimum (a `Clear`, the smallest
/// variant): tag + 4 x u16 win_box.
const MIN_OP_BYTES: usize = 1 + 4 * 2;

/// Serialise a paint log to a versioned byte buffer.
pub fn encode(log: &PaintLog) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    put_u16(&mut out, VERSION);
    for w in &log.windows {
        match w.origin {
            Some((x, y)) => {
                out.push(1);
                put_u16(&mut out, x);
                put_u16(&mut out, y);
            }
            None => out.push(0),
        }
        put_u32(&mut out, w.ops.len() as u32);
        for op in &w.ops {
            put_op(&mut out, op);
        }
    }
    out
}

/// Rebuild a paint log from a buffer written by [`encode`].
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
    let mut windows: [WindowLog; N_WINDOWS] = Default::default();
    for w in windows.iter_mut() {
        let has_origin = r.u8()?;
        w.origin = match has_origin {
            0 => None,
            _ => {
                let x = r.u16()?;
                let y = r.u16()?;
                Some((x, y))
            }
        };
        let n = r.count(MIN_OP_BYTES)?;
        let mut ops = Vec::with_capacity(n);
        for _ in 0..n {
            ops.push(r.op()?);
        }
        w.ops = ops;
    }
    Ok(PaintLog { windows })
}

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn put_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn put_win_box(out: &mut Vec<u8>, b: (u16, u16, u16, u16)) {
    put_u16(out, b.0);
    put_u16(out, b.1);
    put_u16(out, b.2);
    put_u16(out, b.3);
}

fn put_op(out: &mut Vec<u8>, op: &PaintOp) {
    match *op {
        PaintOp::Draw { number, x, y, win_box } => {
            out.push(OP_DRAW);
            put_u16(out, number);
            put_u16(out, x);
            put_u16(out, y);
            put_win_box(out, win_box);
        }
        PaintOp::ErasePicture { number, x, y, win_box } => {
            out.push(OP_ERASE_PICTURE);
            put_u16(out, number);
            put_u16(out, x);
            put_u16(out, y);
            put_win_box(out, win_box);
        }
        PaintOp::Clear { win_box } => {
            out.push(OP_CLEAR);
            put_win_box(out, win_box);
        }
        PaintOp::HostErase { dx, dy, w, h } => {
            out.push(OP_HOST_ERASE);
            put_i32(out, dx);
            put_i32(out, dy);
            put_u32(out, w);
            put_u32(out, h);
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

    fn i32(&mut self) -> Result<i32, ZError> {
        Ok(self.u32()? as i32)
    }

    fn win_box(&mut self) -> Result<(u16, u16, u16, u16), ZError> {
        Ok((self.u16()?, self.u16()?, self.u16()?, self.u16()?))
    }

    fn op(&mut self) -> Result<PaintOp, ZError> {
        let tag = self.u8()?;
        Ok(match tag {
            OP_DRAW => {
                let number = self.u16()?;
                let x = self.u16()?;
                let y = self.u16()?;
                let win_box = self.win_box()?;
                PaintOp::Draw { number, x, y, win_box }
            }
            OP_ERASE_PICTURE => {
                let number = self.u16()?;
                let x = self.u16()?;
                let y = self.u16()?;
                let win_box = self.win_box()?;
                PaintOp::ErasePicture { number, x, y, win_box }
            }
            OP_CLEAR => {
                let win_box = self.win_box()?;
                PaintOp::Clear { win_box }
            }
            OP_HOST_ERASE => {
                let dx = self.i32()?;
                let dy = self.i32()?;
                let w = self.u32()?;
                let h = self.u32()?;
                PaintOp::HostErase { dx, dy, w, h }
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
            &[PaintOp::Clear { win_box }, PaintOp::Draw { number: 3, x: 30, y: 30, win_box }]
        );
    }

    #[test]
    fn one_erase_window_call_folds_its_paired_events_into_one_entry() {
        let mut log = PaintLog::default();
        let win_box = (5, 5, 50, 60);
        for ev in window_clear_pair(win_box) {
            log.apply(&ev);
        }
        assert_eq!(log.ops(0), &[PaintOp::Clear { win_box }]);
    }

    #[test]
    fn erase_window_minus_one_clears_every_window() {
        let mut log = PaintLog::default();
        for win in 0u8..8 {
            let win_box = (1, 1, u16::from(win) + 1, u16::from(win) + 1);
            log.apply(&PaintEvent::Erase(EraseFill::new(win, win_box.0, win_box.1, win_box.2, win_box.3, ZColour::Default, 0)));
            log.apply(&PaintEvent::Picture(PictureEvent::new(0, win, 1, 1, true, 0, None, true, win_box)));
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
            &[PaintOp::Draw { number: 3, x: 5, y: 5, win_box: moved }],
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
        assert_eq!(log.ops(0), &[PaintOp::ErasePicture { number: 7, x: 12, y: 34, win_box }]);
    }

    #[test]
    fn other_windows_are_untouched_by_one_windows_events() {
        let mut log = PaintLog::default();
        let win_box = (1, 1, 10, 10);
        log.apply(&PaintEvent::Picture(PictureEvent::new(9, 3, 1, 1, false, 0, None, true, win_box)));
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
        assert_eq!(log.ops(0), &[PaintOp::Clear { win_box }]);
    }

    #[test]
    fn clear_all_empties_every_window() {
        let mut log = PaintLog::default();
        for win in 0u8..8 {
            log.apply(&draw(1, 1, 1, (1, 1, 10, 10)));
            let _ = win;
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

    #[test]
    fn a_host_erase_lands_in_the_target_windows_own_order() {
        let mut log = PaintLog::default();
        let win_box = (1, 1, 100, 100);
        log.apply(&draw(1, 10, 10, win_box));
        log.append_host_erase(0, 5, 5, 20, 20);
        log.apply(&draw(2, 30, 30, win_box));
        assert_eq!(
            log.ops(0),
            &[
                PaintOp::Draw { number: 1, x: 10, y: 10, win_box },
                PaintOp::HostErase { dx: 5, dy: 5, w: 20, h: 20 },
                PaintOp::Draw { number: 2, x: 30, y: 30, win_box },
            ],
            "the erase sits between the two draws, exactly where it was appended"
        );
    }

    #[test]
    fn a_host_erase_does_not_move_the_tracked_origin() {
        let mut log = PaintLog::default();
        let win_box = (1, 1, 100, 100);
        log.apply(&draw(1, 10, 10, win_box));
        log.append_host_erase(0, 5, 5, 20, 20);
        // A redraw at the SAME origin must not strand — the host erase is not a
        // window-box event and must not have touched the tracked origin.
        log.apply(&draw(2, 11, 11, win_box));
        assert_eq!(log.ops(0).len(), 3, "same origin — nothing was stranded");
    }

    #[test]
    fn a_later_whole_window_clear_drops_an_earlier_host_erase_too() {
        let mut log = PaintLog::default();
        let win_box = (1, 1, 100, 100);
        log.apply(&draw(1, 10, 10, win_box));
        log.append_host_erase(0, 5, 5, 20, 20);
        for ev in window_clear_pair(win_box) {
            log.apply(&ev);
        }
        assert_eq!(log.ops(0), &[PaintOp::Clear { win_box }], "the whole list resets, host erase included");
    }

    #[test]
    fn a_host_erase_past_the_cap_is_dropped() {
        let mut log = PaintLog::default();
        let win_box = (1, 1, 10, 10);
        for i in 0..PAINT_LOG_CAP {
            log.apply(&draw(i as u16, 1, 1, win_box));
        }
        assert!(log.is_capped(0));
        log.append_host_erase(0, 0, 0, 1, 1);
        assert_eq!(log.ops(0).len(), PAINT_LOG_CAP, "capped — the host erase is dropped like any other");
    }

    #[test]
    fn a_host_erase_subsumes_an_earlier_smaller_one_at_the_same_spot() {
        let mut log = PaintLog::default();
        let win_box = (1, 1, 100, 100);
        log.apply(&draw(1, 10, 10, win_box));
        log.append_host_erase(0, 5, 5, 20, 20);
        // Re-erasing the SAME (or a larger, covering) rect every turn — Shogun's
        // caret pattern — must not grow the list.
        log.append_host_erase(0, 5, 5, 20, 20);
        log.append_host_erase(0, 5, 5, 20, 20);
        log.append_host_erase(0, 0, 0, 30, 30); // covers the smaller rect too
        assert_eq!(
            log.ops(0),
            &[
                PaintOp::Draw { number: 1, x: 10, y: 10, win_box },
                PaintOp::HostErase { dx: 0, dy: 0, w: 30, h: 30 },
            ],
            "every earlier host erase the newest one fully covers is pruned"
        );
    }

    #[test]
    fn a_host_erase_does_not_prune_one_it_does_not_fully_cover() {
        let mut log = PaintLog::default();
        log.append_host_erase(0, 0, 0, 10, 10);
        log.append_host_erase(0, 5, 5, 10, 10); // overlaps but does not contain
        assert_eq!(
            log.ops(0),
            &[
                PaintOp::HostErase { dx: 0, dy: 0, w: 10, h: 10 },
                PaintOp::HostErase { dx: 5, dy: 5, w: 10, h: 10 },
            ],
            "a partial overlap prunes nothing — only full containment does"
        );
    }

    #[test]
    fn a_host_erase_for_an_out_of_range_window_is_ignored() {
        let mut log = PaintLog::default();
        log.append_host_erase(200, 0, 0, 1, 1); // no panic is the assertion
        for win in 0u8..8 {
            assert!(log.ops(win).is_empty());
        }
    }

    // -- encode/decode -------------------------------------------------------

    fn sample_log() -> PaintLog {
        let mut log = PaintLog::default();
        let win_box = (1, 1, 100, 100);
        log.apply(&draw(1, 10, 10, win_box));
        log.apply(&erase_pic(2, 20, 20, win_box));
        log.append_host_erase(0, 4, 4, 8, 8);
        for win in 1u8..4 {
            let wb = (u16::from(win), u16::from(win), 40, 50);
            for ev in window_clear_pair(wb) {
                log.apply(&ev);
            }
            log.apply(&draw(win as u16, 3, 3, wb));
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
    fn a_corrupt_op_count_is_an_error_rather_than_an_allocation() {
        // Window 0's op count is the four bytes right after its has-origin
        // flag: magic(4) + version(2) + has_origin(1).
        let mut blob = encode(&PaintLog::default());
        let at = 4 + 2 + 1;
        blob[at..at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(decode(&blob).unwrap_err(), ZError::BadPaintLog);
    }
}
