//! Pure timing/easing primitives for TUI animations.
//!
//! No rendering and no app state — just the math (`ease`, `lerp`,
//! `parse_easing`) plus a thin clock-backed [`Tween`]. Consumers (e.g. smooth
//! transcript scroll) drive their own from/to values through these helpers and
//! the run loop advances them each frame.

use std::time::{Duration, Instant};

/// An easing curve mapping linear progress `t` in `[0,1]` to an eased `[0,1]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Easing {
    /// Identity: `f(t) = t`.
    Linear,
    /// Accelerating (quadratic): below the diagonal until the end.
    EaseIn,
    /// Decelerating (quadratic): above the diagonal until the end. The default.
    #[default]
    EaseOut,
    /// Accelerate then decelerate (symmetric quadratic).
    EaseInOut,
}

/// Clamp `t` to `[0,1]`.
fn clamp01(t: f64) -> f64 {
    t.clamp(0.0, 1.0)
}

/// Map progress `t` (clamped to `[0,1]`) through the easing `curve`; returns `[0,1]`.
pub fn ease(curve: Easing, t: f64) -> f64 {
    let t = clamp01(t);
    match curve {
        Easing::Linear => t,
        Easing::EaseIn => t * t,
        Easing::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
        Easing::EaseInOut => {
            if t < 0.5 {
                2.0 * t * t
            } else {
                let u = -2.0 * t + 2.0;
                1.0 - u * u / 2.0
            }
        }
    }
}

/// Parse a config token into an [`Easing`].
///
/// Recognizes `"linear"`, `"ease-in"`, `"ease-out"`, `"ease-in-out"`. Any
/// unknown value maps to [`Easing::EaseOut`] (the default).
pub fn parse_easing(s: &str) -> Easing {
    match s {
        "linear" => Easing::Linear,
        "ease-in" => Easing::EaseIn,
        "ease-out" => Easing::EaseOut,
        "ease-in-out" => Easing::EaseInOut,
        _ => Easing::EaseOut,
    }
}

/// The canonical config token for an [`Easing`] (inverse of [`parse_easing`]).
pub fn easing_token(e: Easing) -> &'static str {
    match e {
        Easing::Linear => "linear",
        Easing::EaseIn => "ease-in",
        Easing::EaseOut => "ease-out",
        Easing::EaseInOut => "ease-in-out",
    }
}

/// Linear interpolation: `from + (to - from) * t`.
pub fn lerp(from: f64, to: f64, t: f64) -> f64 {
    from + (to - from) * t
}

/// A clock-backed animation timer. Constructed with a duration and easing; its
/// [`progress`](Tween::progress) eases the elapsed fraction and clamps past the end.
#[derive(Debug, Clone)]
pub struct Tween {
    started: Instant,
    duration: Duration,
    easing: Easing,
}

impl Tween {
    /// Start a tween of `duration` using `easing`, beginning now.
    pub fn new(duration: Duration, easing: Easing) -> Self {
        Self { started: Instant::now(), duration, easing }
    }

    /// Eased progress in `[0,1]`: `ease(easing, clamp01(elapsed / duration))`.
    /// A zero duration reports `1.0` immediately.
    pub fn progress(&self) -> f64 {
        let frac = if self.duration.is_zero() {
            1.0
        } else {
            self.started.elapsed().as_secs_f64() / self.duration.as_secs_f64()
        };
        ease(self.easing, frac)
    }

    /// True once the elapsed time has reached the duration.
    pub fn done(&self) -> bool {
        self.started.elapsed() >= self.duration
    }
}

/// Session-only slide state for a slide-in panel/dock. Holds a target fraction
/// and an optional tween easing the displayed fraction toward it (so a
/// mid-slide reverse starts from the current position).
#[derive(Debug)]
pub struct PanelSlide {
    pub open: bool,
    from: f64,
    to: f64,
    tween: Option<Tween>,
}

impl PanelSlide {
    pub fn closed() -> Self {
        Self { open: false, from: 0.0, to: 0.0, tween: None }
    }

    /// The displayed fraction right now (tween-eased), given a raw progress.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn fraction_at(&self, progress: f64) -> f64 {
        lerp(self.from, self.to, progress)
    }

    /// Current displayed fraction from the live tween (or the settled `to`).
    pub fn fraction(&self) -> f64 {
        match &self.tween {
            Some(t) => lerp(self.from, self.to, t.progress()),
            None => self.to,
        }
    }

    pub fn active(&self) -> bool {
        self.tween.as_ref().is_some_and(|t| !t.done())
    }

    /// Drop a finished slide tween (the displayed `fraction()` already holds the
    /// settled `to`). Returns `true` iff a completed tween was cleared this call
    /// — the run loop ORs that into its redraw flag so the settled slide frame
    /// paints once. Without it a dock that finishes its slide during the poll
    /// wait goes inactive before the settle frame is drawn, leaving e.g. a
    /// ~1-row sliver of a closing inv_dock. (SQ-0305)
    pub fn finalize_if_done(&mut self) -> bool {
        if self.tween.as_ref().is_some_and(|t| t.done()) {
            self.tween = None;
            self.from = self.to;
            true
        } else {
            false
        }
    }

    /// Toggle to `open`, arming a tween unless `instant`.
    pub fn toggle_to(&mut self, open: bool, instant: bool) {
        self.open = open;
        let target = if open { 1.0 } else { 0.0 };
        let current = self.fraction();
        self.from = current;
        self.to = target;
        self.tween = None; // set by caller with duration; see arm()
        if instant {
            self.from = target;
        }
    }

    /// Arm the tween with the configured duration/easing (call after toggle_to).
    pub fn arm(&mut self, cfg: &crate::config::AnimationConfig) {
        if !cfg.enabled || cfg.scroll_ms == 0 || (self.from - self.to).abs() < f64::EPSILON {
            self.from = self.to;
            self.tween = None;
        } else {
            self.tween = Some(Tween::new(
                std::time::Duration::from_millis(cfg.scroll_ms),
                cfg.easing,
            ));
        }
    }
}

#[cfg(all(test, feature = "t-misc"))]
mod tests {
    use super::*;

    #[test]
    fn ease_endpoints_are_fixed() {
        for curve in [Easing::Linear, Easing::EaseIn, Easing::EaseOut, Easing::EaseInOut] {
            assert!((ease(curve, 0.0) - 0.0).abs() < 1e-9, "{curve:?} at t=0");
            assert!((ease(curve, 1.0) - 1.0).abs() < 1e-9, "{curve:?} at t=1");
        }
    }

    #[test]
    fn ease_clamps_out_of_range_t() {
        assert_eq!(ease(Easing::Linear, -1.0), 0.0);
        assert_eq!(ease(Easing::Linear, 2.0), 1.0);
    }

    #[test]
    fn linear_is_identity() {
        assert!((ease(Easing::Linear, 0.5) - 0.5).abs() < 1e-9);
        assert!((ease(Easing::Linear, 0.25) - 0.25).abs() < 1e-9);
    }

    #[test]
    fn ease_in_below_diagonal_ease_out_above_at_half() {
        // Non-vacuous: EaseIn lags the diagonal, EaseOut leads it.
        assert!(ease(Easing::EaseIn, 0.5) < 0.5, "ease-in must be below diagonal at t=0.5");
        assert!(ease(Easing::EaseOut, 0.5) > 0.5, "ease-out must be above diagonal at t=0.5");
    }

    #[test]
    fn ease_in_out_is_symmetric_midpoint() {
        assert!((ease(Easing::EaseInOut, 0.5) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn parse_easing_known_and_unknown() {
        assert_eq!(parse_easing("linear"), Easing::Linear);
        assert_eq!(parse_easing("ease-in"), Easing::EaseIn);
        assert_eq!(parse_easing("ease-out"), Easing::EaseOut);
        assert_eq!(parse_easing("ease-in-out"), Easing::EaseInOut);
        // Unknown falls back to the default.
        assert_eq!(parse_easing("bogus"), Easing::EaseOut);
        assert_eq!(parse_easing(""), Easing::EaseOut);
    }

    #[test]
    fn easing_token_round_trips() {
        for curve in [Easing::Linear, Easing::EaseIn, Easing::EaseOut, Easing::EaseInOut] {
            assert_eq!(parse_easing(easing_token(curve)), curve);
        }
    }

    #[test]
    fn lerp_interpolates() {
        assert_eq!(lerp(0.0, 10.0, 0.0), 0.0);
        assert_eq!(lerp(0.0, 10.0, 1.0), 10.0);
        assert_eq!(lerp(0.0, 10.0, 0.5), 5.0);
        assert_eq!(lerp(4.0, 8.0, 0.25), 5.0);
    }

    #[test]
    fn tween_progress_starts_at_zero_and_completes() {
        let t = Tween::new(Duration::from_millis(20), Easing::Linear);
        assert!(t.progress() < 0.5, "progress near 0 right after start");
        assert!(!t.done());
        std::thread::sleep(Duration::from_millis(40));
        assert!(t.done(), "tween should be done after its duration");
        assert!((t.progress() - 1.0).abs() < 1e-9, "progress clamps to 1 past the end");
    }

    #[test]
    fn tween_zero_duration_is_immediately_done() {
        let t = Tween::new(Duration::ZERO, Easing::EaseOut);
        assert!(t.done());
        assert_eq!(t.progress(), 1.0);
    }

    #[test]
    fn panel_slide_closed_is_inactive_at_zero() {
        let s = PanelSlide::closed();
        assert!(!s.active());
        assert_eq!(s.fraction(), 0.0);
    }

    #[test]
    fn panel_slide_arm_enabled_is_active() {
        let cfg_enabled = crate::config::AnimationConfig {
            enabled: true,
            easing: Easing::Linear,
            scroll_ms: 100,
            ..Default::default()
        };
        let mut s = PanelSlide::closed();
        s.toggle_to(true, false);
        s.arm(&cfg_enabled);
        assert!(s.active(), "tween should not be done immediately after arming");
        let f = s.fraction();
        assert!((0.0..1.0).contains(&f), "fraction {f} should be between from (0.0) and to (1.0)");
    }

    #[test]
    fn panel_slide_arm_zero_ms_snaps() {
        let cfg_instant = crate::config::AnimationConfig {
            enabled: true,
            easing: Easing::Linear,
            scroll_ms: 0,
            ..Default::default()
        };
        let mut s = PanelSlide::closed();
        s.toggle_to(true, false);
        s.arm(&cfg_instant);
        assert!(!s.active());
        assert_eq!(s.fraction(), 1.0);
    }
}
