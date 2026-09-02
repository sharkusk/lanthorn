//! Transient top-right notification popups (SQ-0176).
//!
//! The app's short feedback messages — save/restore/export banners, slash
//! `Message`/`Error` results, VM faults, map-command refusals — used to hijack
//! the status/score bar (`set_status`) or clutter the transcript (`push_notice`).
//! SQ-0176 gives them a home of their own: a stack of toasts that slide in at the
//! top-right, hold for a few seconds, slide out, and vanish — never touching the
//! score bar and never written to the story. Nothing is lost: every message is
//! also kept in a bounded `history`, which `/dump-notifications` replays into the
//! transcript "in case some are missed".
//!
//! SQ-0415: the toasts render inside the story pane (top-right of that rect,
//! sliding in from the pane's right edge) rather than over the full frame, so
//! they never cover the map. If the story pane is absent or too small to hold
//! anything, rendering falls back to the full frame so a toast is never lost.
//!
//! Timing is clock-based (`Instant`), mirroring the `sound_pulse` flash: the run
//! loop's redraw gate ([`Notifications::needs_tick`]) keeps frames stepping only
//! while a toast is sliding in/out or waiting to be reaped — the multi-second
//! hold costs no redraws. Rendering lives in `render/transcript.rs`.

use std::time::Instant;

use crate::anim::{ease, Easing};

/// Slide-in duration (ms).
pub const NOTIFY_IN_MS: u64 = 220;
/// How long a toast holds fully shown (ms) — "a few seconds".
pub const NOTIFY_HOLD_MS: u64 = 3600;
/// Slide-out duration (ms).
pub const NOTIFY_OUT_MS: u64 = 320;
/// Cap on retained history entries (oldest dropped past this).
pub const NOTIFY_MAX_HISTORY: usize = 200;
/// Cap on concurrently *visible* toasts (oldest still-active dropped past this).
pub const NOTIFY_MAX_VISIBLE: usize = 5;

/// A single toast: its text and the instant it was born.
#[derive(Debug)]
pub struct Notification {
    pub text: String,
    born: Instant,
}

impl Notification {
    fn age_ms(&self) -> u64 {
        self.born.elapsed().as_millis() as u64
    }

    fn total_ms() -> u64 {
        NOTIFY_IN_MS + NOTIFY_HOLD_MS + NOTIFY_OUT_MS
    }

    /// True once the toast has lived out its full in+hold+out lifetime.
    pub fn expired(&self) -> bool {
        self.age_ms() >= Self::total_ms()
    }

    /// True while the toast is mid-slide (in or out) — the display is changing,
    /// so the run loop must keep redrawing. The hold phase returns `false`.
    fn animating(&self) -> bool {
        let a = self.age_ms();
        a < NOTIFY_IN_MS || (a >= NOTIFY_IN_MS + NOTIFY_HOLD_MS && a < Self::total_ms())
    }

    /// Reveal fraction in `[0,1]`: `0` = fully hidden off the right edge, `1` =
    /// fully shown at rest. With `animate` off it snaps to `1` for the whole
    /// lifetime (no slide, but the toast still expires on the same clock).
    pub fn reveal(&self, animate: bool, easing: Easing) -> f64 {
        if !animate {
            return 1.0;
        }
        let a = self.age_ms();
        if a < NOTIFY_IN_MS {
            ease(easing, a as f64 / NOTIFY_IN_MS as f64)
        } else if a < NOTIFY_IN_MS + NOTIFY_HOLD_MS {
            1.0
        } else {
            let t = (a - NOTIFY_IN_MS - NOTIFY_HOLD_MS) as f64 / NOTIFY_OUT_MS as f64;
            1.0 - ease(easing, t.clamp(0.0, 1.0))
        }
    }
}

/// The live toast stack plus a bounded message history for `/dump-notifications`.
#[derive(Debug, Default)]
pub struct Notifications {
    active: Vec<Notification>,
    history: Vec<String>,
}

impl Notifications {
    /// Push a new toast (newest is drawn on top) and record it in history.
    ///
    /// A single surrounding `[…]` pair — the old transcript-notice convention —
    /// is stripped: it reads as clutter inside a bordered toast (SQ-0176).
    pub fn push(&mut self, text: impl Into<String>) {
        let mut text = text.into();
        if text.len() >= 2 && text.starts_with('[') && text.ends_with(']') {
            text = text[1..text.len() - 1].to_string();
        }
        self.history.push(text.clone());
        if self.history.len() > NOTIFY_MAX_HISTORY {
            let drop = self.history.len() - NOTIFY_MAX_HISTORY;
            self.history.drain(0..drop);
        }
        self.active.push(Notification { text, born: Instant::now() });
        // Bound the visible stack: drop the oldest still-active toasts.
        if self.active.len() > NOTIFY_MAX_VISIBLE {
            let drop = self.active.len() - NOTIFY_MAX_VISIBLE;
            self.active.drain(0..drop);
        }
    }

    /// Reap expired toasts. Returns `true` if any were removed (→ repaint once).
    pub fn expire(&mut self) -> bool {
        let before = self.active.len();
        self.active.retain(|n| !n.expired());
        before != self.active.len()
    }

    /// True while any toast is sliding or waiting to be reaped — the run loop's
    /// redraw gate. Deliberately `false` during a toast's static hold so the
    /// several-second wait costs no redraws.
    pub fn needs_tick(&self) -> bool {
        self.active.iter().any(|n| n.animating() || n.expired())
    }

    /// The toasts to draw (oldest first; render newest-on-top by reversing).
    pub fn active(&self) -> &[Notification] {
        &self.active
    }

    /// The bounded message history, oldest first.
    pub fn history(&self) -> &[String] {
        &self.history
    }

    /// Text of the most recently pushed toast still on screen (tests/helpers).
    pub fn latest_text(&self) -> Option<&str> {
        self.active.last().map(|n| n.text.as_str())
    }
}

#[cfg(all(test, feature = "t-misc"))]
mod tests {
    use super::*;

    #[test]
    fn push_records_active_and_history_stripping_brackets() {
        let mut n = Notifications::default();
        n.push("[Saved as: foo]");
        assert_eq!(n.active().len(), 1);
        // The surrounding [...] pair is stripped for a clean toast.
        assert_eq!(n.history(), &["Saved as: foo".to_string()]);
        assert_eq!(n.latest_text(), Some("Saved as: foo"));
        // Text without brackets is left untouched.
        n.push("move-region: no room selected");
        assert_eq!(n.latest_text(), Some("move-region: no room selected"));
    }

    #[test]
    fn visible_stack_is_capped_but_history_is_not() {
        let mut n = Notifications::default();
        for i in 0..(NOTIFY_MAX_VISIBLE + 3) {
            n.push(format!("msg {i}"));
        }
        assert_eq!(n.active().len(), NOTIFY_MAX_VISIBLE, "visible stack capped");
        assert_eq!(n.history().len(), NOTIFY_MAX_VISIBLE + 3, "history keeps them all");
        // The newest survives in the visible stack.
        assert_eq!(n.latest_text(), Some(&*format!("msg {}", NOTIFY_MAX_VISIBLE + 2)));
    }

    #[test]
    fn history_is_bounded() {
        let mut n = Notifications::default();
        for i in 0..(NOTIFY_MAX_HISTORY + 10) {
            n.push(format!("m{i}"));
        }
        assert_eq!(n.history().len(), NOTIFY_MAX_HISTORY);
        // Oldest were dropped; the newest remains last.
        assert_eq!(n.history().last().map(String::as_str), Some(&*format!("m{}", NOTIFY_MAX_HISTORY + 9)));
    }

    #[test]
    fn fresh_toast_is_animating_and_not_expired() {
        let mut n = Notifications::default();
        n.push("hi");
        assert!(n.needs_tick(), "a just-pushed toast is sliding in");
        assert!(!n.expire(), "nothing to reap yet");
        assert_eq!(n.active().len(), 1);
    }

    #[test]
    fn reveal_snaps_to_one_when_animation_disabled() {
        let mut n = Notifications::default();
        n.push("hi");
        let note = &n.active()[0];
        assert_eq!(note.reveal(false, Easing::EaseOut), 1.0);
        // Animated: a fresh toast is partway (or at most) into its slide-in.
        let r = note.reveal(true, Easing::EaseOut);
        assert!((0.0..=1.0).contains(&r));
    }

    #[test]
    fn expired_toast_is_reaped() {
        // A zero-lifetime toast: fake age by constructing past its total.
        let mut n = Notifications::default();
        n.active.push(Notification {
            text: "old".into(),
            born: Instant::now() - std::time::Duration::from_millis(Notification::total_ms() + 50),
        });
        assert!(n.active()[0].expired());
        assert!(n.needs_tick(), "an expired-but-unreaped toast keeps the loop alive");
        assert!(n.expire(), "reaping removes it");
        assert!(!n.needs_tick());
        assert!(n.active().is_empty());
    }
}
