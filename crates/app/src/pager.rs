//! `[more]` pager (SQ-0404).
//!
//! After a game command whose output overflows the story pane, instead of
//! auto-jumping to the newest line the view stops at the FIRST new screenful and
//! shows a `[more]` prompt; each keypress pages down one screen through the new
//! output until it catches up to the bottom, then the pager exits.
//!
//! The story pane's scroll offset (`AppState.transcript_scroll`) is measured in
//! rows-from-bottom (`0` = newest at bottom). Wrapped-row counts are only known
//! at render time, so engaging the pager is a two-phase arm/activate:
//!   1. **arm** — a qualifying command turn records the transcript's wrapped-row
//!      total *before* its output (from the last rendered frame).
//!   2. **activate** — the next render knows the *after* total and the viewport
//!      height, so [`activation_target`] decides whether to engage and where to
//!      park the scroll offset.
//!
//! # Measuring against what the reader actually gets (SQ-0823)
//!
//! The viewport in that arithmetic has to be the rows of PROSE the pane shows,
//! and the pane rect is not that number: the status line, the input bar, a
//! suggestion strip and the `[more]` bar itself each take one. Counting any of
//! them as readable parks the view low and slips a line past the pause — the
//! reported symptom, *"it scrolls one line too many before the [more] prompt is
//! shown"*. The renderer therefore reports its true body height, and reports the
//! `[more]` bar's row separately (`prompt_rows`) because the park is computed one
//! frame BEFORE the bar exists: the screenful being aimed at is a row shorter than
//! the frame doing the aiming. On the v6 raster path the prompt is stamped over
//! the tail of the last prose row instead of reserving one, so there
//! `prompt_rows` is 0 and nothing moves.
//!
//! # A turn that CONTINUES the row above it (SQ-0823)
//!
//! The baseline is a row count, and rows are atomic to [`activation_target`] —
//! every row below `before_rows` is old, every row at or above it is new. That
//! holds only while a turn's output OPENS a line. It does not have to: a
//! `read_char` turn whose game printed where the cursor already was is folded onto
//! the line it was already on ([`push_transcript_runs_char_echo`](crate::state::
//! AppState::push_transcript_runs_char_echo)), and then the last pre-turn row is
//! PARTLY OLD AND PARTLY NEW. Counted as old, its new half is exactly what slips
//! past the pause.
//!
//! Arthur's InvisiClues are the report. Its hint pages print a `1> ` prompt and
//! wait; the key that answers appends the whole page after that prompt, on that
//! row. Measured on the Amiga floppy at 80x48, `before_rows` 79 and 124 rows after,
//! the park put absolute row 79 on top — leaving `1> SENIOR PROGRAMMER`, the
//! section heading its list of names belongs to, one row above the fold.
//!
//! So [`baseline_before`] steps the baseline back onto that row. Note it steps back
//! exactly ONE row, never the whole logical line: only the pre-turn wrap's LAST row
//! is shared, and any rows the appended text wraps onto are wholly new and already
//! counted. What is on screen is what is measured.
//!
//! # Arming ruleset (SQ-0539)
//!
//! The v1 (SQ-0404) pager only armed behind a LINE read on a turn that did not
//! clear the screen. That is narrower than the games expect: an authentic Infocom
//! interpreter paged whenever the output since the last player interaction
//! exceeded the screen, *regardless* of what input came next. So the rule is now
//! simply "the game is waiting for the player, and the player would otherwise
//! miss text":
//!
//! | turn kind (pending input after the turn) | screen cleared | added rows > viewport | v6 veto | behavior |
//! |---|---|---|---|---|
//! | `Line` | no  | yes | no  | **page** |
//! | `Line` | no  | no  | no  | no pager (nothing past the fold) |
//! | `Line` | yes | yes | no  | **page** — the fresh screenful itself overflows |
//! | `Line` | yes | no  | no  | no pager — the repaint fits, nothing was missed |
//! | `Char` | any | yes | no  | **page** (keys page until caught up — see below) |
//! | `Char` | any | no  | no  | no pager |
//! | `Event` (Glulx timer/mouse-only `glk_select`) | any | any | any | no pager — out of scope |
//! | any | any | any | **yes** | no pager (ZMSD §8.8.3.2.6 "never print [MORE]") |
//! | lanthorn meta output (`/help`, …) | — | — | — | no pager (never reaches an arm site) |
//!
//! A screen clear needs no special case: the clear preserves scrollback and marks
//! an anchor, so *rows added by this turn* already measures exactly the post-clear
//! repaint — which is precisely "what the player can still scroll to see".
//!
//! **Char input.** While the pager is showing with a `read_char` pending, the
//! paging keys are consumed by the pager and NOT delivered to the game: the first
//! key pages instead of answering the read, and only once the view has caught up
//! to the bottom does the next key reach the game. Any key pages (not just
//! Space/PgDn/↓/Enter) — jumping to the bottom would skip text the player never
//! saw, which is the one thing the pager exists to prevent.
//!
//! **Timeouts.** Mirroring the engine's v6 line-count rule (a real keystroke
//! reloads the count, a timeout does not — see `Machine::v6_reload_line_counts`),
//! a turn driven by a timed-input interrupt / Glk timer / sound-finish routine
//! never reloads the pager baseline and never dismisses an active pager; its
//! output simply accumulates toward the baseline the last real interaction set.

use crate::session::InputKind;

/// Pager runtime state, held on `AppState`.
#[derive(Debug, Default, Clone)]
pub struct Pager {
    /// True while the `[more]` prompt is showing and keypresses page the pane.
    pub active: bool,
    /// Set by a qualifying command turn: the transcript's wrapped-row total
    /// BEFORE this turn's output, awaiting the next render to decide whether to
    /// engage. `None` when not armed.
    pub pending_before_rows: Option<u16>,
}

impl Pager {
    /// Arm for the turn that just appended output: record the pre-turn wrapped-row
    /// count so the next render can measure how much was added.
    pub fn arm(&mut self, before_rows: u16) {
        self.pending_before_rows = Some(before_rows);
    }

    /// Clear any pending arm (e.g. the added output fit in one screen).
    pub fn disarm(&mut self) {
        self.pending_before_rows = None;
    }

    /// Apply the [module ruleset](self) to the turn that just appended output.
    ///
    /// `before_rows` is the transcript's wrapped-row total from the last rendered
    /// frame, `pending` the input the game is waiting on *now*, `more_suppressed`
    /// the v6 "never print [MORE]" veto (see [`more_suppressed`]) and `driver`
    /// what caused the turn.
    pub fn arm_after_turn(
        &mut self,
        before_rows: u16,
        pending: InputKind,
        more_suppressed: bool,
        driver: Driver,
    ) {
        // Already paging: never re-park the view or reload the baseline
        // mid-catch-up. (Real keystrokes can't reach the game while the pager is
        // up, so only timer/sound-driven turns normally land here.)
        if self.active {
            return;
        }
        if !should_arm(pending, more_suppressed) {
            // A real interaction that doesn't qualify cancels a stale arm; a
            // timeout leaves the standing baseline alone.
            if driver == Driver::PlayerInput {
                self.disarm();
            }
            return;
        }
        match driver {
            Driver::PlayerInput => self.arm(before_rows),
            // A timeout is not a keystroke: keep the baseline the last real
            // interaction set, so this output accumulates toward it instead of
            // resetting the "since the player last acted" measurement.
            Driver::Timeout => {
                self.pending_before_rows.get_or_insert(before_rows);
            }
        }
    }
}

/// The baseline to arm with, given the last rendered frame's wrapped-row total and
/// whether this turn's output CONTINUED the last of those rows rather than opening
/// a line below it (SQ-0823 — see the [module docs](self)).
///
/// A continued row is partly this turn's, so it is new and the baseline steps back
/// onto it. Exactly one row: the pre-turn wrap's last row is the only shared one,
/// and rows the appended text wraps onto are wholly new and lie above the baseline
/// already.
pub fn baseline_before(last_frame_rows: u16, continued_row: bool) -> u16 {
    last_frame_rows.saturating_sub(u16::from(continued_row))
}

/// What drove the turn whose output the pager is about to measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Driver {
    /// A real player interaction — a submitted command line or a keypress.
    PlayerInput,
    /// No player input: a timed-input interrupt, a Glk timer tick, or a
    /// sound-finish routine.
    Timeout,
}

/// Does a turn that left the game waiting on `pending` qualify for the pager?
///
/// `Line` and `Char` both do — the original interpreters paged before *any* read,
/// not only a `>` prompt. `Event` (a Glulx timer/mouse-only `glk_select`) does
/// not: no typed input is requested and the Z-machine transcript pager is out of
/// scope there. `more_suppressed` vetoes everything (ZMSD §8.8.3.2.6).
pub fn should_arm(pending: InputKind, more_suppressed: bool) -> bool {
    if more_suppressed {
        return false;
    }
    matches!(pending, InputKind::Line | InputKind::Char)
}

/// The running story's "never print [MORE]" veto: true only for a v6 Z-machine
/// story whose current window has parked its line count at -999 (ZMSD §8.8.3.2.6
/// — the device Zork Zero's demonstration mode uses). Safely false for every
/// non-v6 Z-machine story and for a Glulx/Scott engine, which expose no such
/// counter. (SQ-0534, app half.)
pub fn more_suppressed(engine: &dyn crate::engine::Engine) -> bool {
    engine
        .as_any()
        .downcast_ref::<crate::session::GameSession>()
        .is_some_and(|z| z.machine.v6_suppress_more())
}

/// Given the transcript's wrapped-row totals before/after a turn and the story
/// pane's viewport height, decide whether the pager should engage and, if so, the
/// initial rows-from-bottom scroll offset that shows the FIRST new screenful.
///
/// Returns `None` when the new output fits within one screen (no pager needed).
///
/// `viewport_rows` is the rows of PROSE the pane shows with no prompt up, and
/// `prompt_rows` how many of them the `[more]` bar takes once it appears (1 on the
/// cell paths, which reserve it a row; 0 on the raster path, which overlays it).
/// Both matter, and to different halves of the decision:
///
/// - **Whether to engage** is measured against the full `viewport_rows`. With no
///   pager there is no prompt bar, so a turn that adds exactly a screenful is
///   entirely on screen and nothing was missed.
/// - **Where to park** is measured against `viewport_rows - prompt_rows`, because
///   that is the screenful the reader will actually be looking at.
///
/// Derivation: the paged viewport shows absolute rows `[after - scroll - body ..
/// after - scroll]` for `body = viewport - prompt`. To put the first new row
/// (`before`) at its top: `after - scroll - body = before` → `scroll = added -
/// body`, where `added = after - before`. Getting `prompt_rows` wrong here is a
/// row of prose scrolling past unread before the pause — the SQ-0823 report.
pub fn activation_target(
    before_rows: u16,
    after_rows: u16,
    viewport_rows: u16,
    prompt_rows: u16,
) -> Option<u16> {
    if viewport_rows == 0 {
        return None;
    }
    let added = after_rows.saturating_sub(before_rows);
    if added <= viewport_rows {
        return None; // fits in one screen — leave the view at the bottom
    }
    Some(added - viewport_rows.saturating_sub(prompt_rows))
}

/// Post-frame transcript bookkeeping, run once per drawn frame with the story
/// pane's [`StoryPaneMetrics`](crate::render::screen::StoryPaneMetrics) numbers:
/// clamp scrollback to what the frame can actually show (so an over-scroll past
/// the top doesn't accumulate), resolve a pending pager arm now that the frame
/// knows the after-total and the viewport, and cache this frame's total as the
/// next turn's baseline.
///
/// A frame with NO transcript surface — a v6 full-screen picture (the splash,
/// Zork Zero's map/rebus takeovers) — is skipped wholesale (SQ-0578): its zeroed
/// metrics are a property of the picture, not of the transcript. Clamping
/// against them wiped the reader's scrollback offset, and caching its zero total
/// made the next keypress measure the ENTIRE backlog as "new output", re-parking
/// the view at the top with a [more] to drain. A pending arm simply survives
/// until the next frame that really lays the transcript out.
pub fn apply_frame(
    state: &mut crate::state::AppState,
    max_scroll: u16,
    viewport_rows: u16,
    prompt_rows: u16,
    total_rows: u16,
    transcript_surface: bool,
) {
    if !transcript_surface {
        return;
    }
    state.transcript_scroll = state.transcript_scroll.min(max_scroll);
    if let Some(before) = state.pager.pending_before_rows.take() {
        match activation_target(before, total_rows, viewport_rows, prompt_rows) {
            Some(target) => {
                // `max_scroll` came from THIS frame, which has no prompt bar on it;
                // the frame the target is for gives `prompt_rows` of its viewport to
                // the bar, so it can scroll that much further back. Clamping to the
                // prompt-less ceiling would pin a whole-transcript overflow (a boot
                // banner, `before == 0`) one row short and drop its first line —
                // exactly the row this quest is about, on the one turn where the
                // clamp binds (SQ-0823).
                state.scroll_transcript_to(target.min(max_scroll.saturating_add(prompt_rows)));
                state.pager.active = true;
            }
            None => state.pager.active = false,
        }
    }
    state.last_transcript_total_rows = total_rows;
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;

    /// SQ-0578: a v6 full-screen picture frame (Zork Zero's rebus) reports no
    /// transcript surface. Its zeroed metrics must not clamp the reader's
    /// scrollback or replace the pager baseline — pre-guard, the keypress that
    /// dismissed the picture measured the WHOLE backlog against the picture
    /// frame's zero total and re-parked the view at the top with a [more].
    #[test]
    fn picture_frames_do_not_reset_scroll_or_pager_baseline() {
        let mut state = crate::state::AppState::default();

        // A settled session: 500-row backlog, reader scrolled 100 rows up.
        apply_frame(&mut state, 476, 24, 0, 500, true);
        assert_eq!(state.last_transcript_total_rows, 500);
        state.transcript_scroll = 100;

        // `look at rebus` arms the pager with the real pre-turn total, then the
        // picture frame draws: no transcript surface, all metrics zero.
        state.pager.arm(state.last_transcript_total_rows);
        apply_frame(&mut state, 0, 40, 0, 0, false);
        assert_eq!(state.transcript_scroll, 100, "a picture frame must not clamp scrollback away");
        assert_eq!(state.last_transcript_total_rows, 500, "a picture frame must not become the pager baseline");
        assert_eq!(state.pager.pending_before_rows, Some(500), "a pending arm survives until a real transcript frame");

        // The dismissing keypress re-arms from the (unpoisoned) baseline; the
        // normal frame returns with the same 500 rows: nothing new, no pager.
        state.pager.arm(state.last_transcript_total_rows);
        apply_frame(&mut state, 476, 24, 0, 500, true);
        assert!(!state.pager.active, "returning from the picture pages nothing (no rows were added)");
        assert_eq!(state.transcript_scroll, 100, "the reader's position survives the picture round-trip");
    }

    #[test]
    fn no_pager_when_output_fits_one_screen() {
        // 8 rows added, viewport 10 → fits, no pager.
        assert_eq!(activation_target(20, 28, 10, 0), None);
        // Exactly one screen added is still "fits" (nothing past the fold).
        assert_eq!(activation_target(20, 30, 10, 0), None);
        // Degenerate viewport never engages.
        assert_eq!(activation_target(0, 100, 0, 0), None);
    }

    #[test]
    fn pager_parks_at_first_new_screenful() {
        // 25 rows added, viewport 10 → engage; first screenful sits 15 rows up.
        assert_eq!(activation_target(2, 27, 10, 0), Some(15));
        // The target never exceeds max_scroll (= after - viewport = 17).
        let t = activation_target(2, 27, 10, 0).unwrap();
        assert!(t <= 27u16.saturating_sub(10));
    }

    /// SQ-0823: the `[more]` bar takes a row of the viewport for itself on the
    /// cell paths, and the park is computed on the frame BEFORE it appears — so
    /// the screenful being aimed at is one row shorter than the frame doing the
    /// aiming. Ignore that and the top row of the first new screenful is exactly
    /// the row the bar displaces: one line of prose scrolled past, unread, which
    /// is the report.
    #[test]
    fn the_prompt_row_comes_out_of_the_parked_screenful() {
        // Same 25-row overflow as above, on a path that reserves a prompt row: the
        // reader gets 9 rows of prose, so the park is one row further back.
        assert_eq!(activation_target(2, 27, 10, 1), Some(16));
        // …and the row it puts on top is the first NEW row: with the parked frame
        // showing `viewport - prompt` rows ending at `after - scroll`, the top row
        // is `after - scroll - (viewport - prompt)` = `before`.
        let scroll = activation_target(2, 27, 10, 1).unwrap();
        assert_eq!(27 - scroll - (10 - 1), 2, "the first new row lands at the viewport top");
        // The raster path overlays its prompt instead (`prompt_rows == 0`), so its
        // park must not move.
        let scroll = activation_target(2, 27, 10, 0).unwrap();
        assert_eq!(27 - scroll - 10, 2);
    }

    /// The prompt row changes where to park, never WHETHER to. With no pager up
    /// there is no bar, so a turn that adds exactly a screenful is entirely on
    /// screen — engaging would raise a `[more]` over output nobody missed.
    #[test]
    fn the_prompt_row_does_not_lower_the_activation_threshold() {
        assert_eq!(activation_target(20, 30, 10, 1), None, "exactly one screen still fits");
        assert_eq!(activation_target(20, 31, 10, 1), Some(2), "one row past the fold pages");
    }

    /// The clamp has to allow for the prompt row too. `max_scroll` is measured on
    /// the frame that decides — which has no bar on it — while the target is for
    /// the frame that does, whose shorter body can scroll one row further back.
    /// The clamp only ever binds when the whole transcript is new (a boot banner,
    /// `before == 0`), and that is precisely where clamping to the prompt-less
    /// ceiling drops the banner's first line (SQ-0823).
    #[test]
    fn the_activation_clamp_allows_for_the_prompt_row() {
        let mut state = crate::state::AppState::default();
        // A 40-row boot banner into a 10-row viewport, nothing before it.
        state.pager.arm(0);
        apply_frame(&mut state, 30, 10, 1, 40, true);
        assert!(state.pager.active);
        assert_eq!(state.transcript_scroll, 31, "parked one row past the prompt-less ceiling");
        // Which is exactly the offset that shows row 0 at the top of a 9-row body.
        assert_eq!(40 - state.transcript_scroll - (10 - 1), 0);
    }

    /// SQ-0823: a turn whose output continued the last pre-turn row makes that row
    /// new, so the baseline steps back onto it — and by exactly one row however far
    /// the appended text wraps, because only that row is shared.
    #[test]
    fn a_continued_row_steps_the_baseline_back_onto_itself() {
        assert_eq!(baseline_before(79, false), 79, "a turn that opens a line measures from the top of it");
        assert_eq!(baseline_before(79, true), 78, "a continued row is this turn's too");
        assert_eq!(baseline_before(0, true), 0, "nothing to step back onto at the very start");

        // Arthur's credits page, measured on the Amiga floppy at 80x48: 79 rows
        // before, 124 after, a 44-row viewport whose [more] bar takes one. Parked
        // from the unadjusted baseline the top row was 79 — the row carrying
        // `1> SENIOR PROGRAMMER` sat one above the fold.
        let (before, after, viewport, prompt) = (79u16, 124u16, 44u16, 1u16);
        let stale = activation_target(before, after, viewport, prompt).unwrap();
        assert_eq!(after - stale - (viewport - prompt), 79, "the reported symptom");
        let fixed = activation_target(baseline_before(before, true), after, viewport, prompt).unwrap();
        assert_eq!(after - fixed - (viewport - prompt), 78, "the continued row is the top row now");

        // The wrap boundary: the appended text turns that one row into three. Two of
        // them are wholly new and already above the baseline, so the step back is
        // still one row and the park still lands on the shared row.
        let target = activation_target(baseline_before(20, true), 60, 10, 1).unwrap();
        assert_eq!(60 - target - (10 - 1), 19);
    }

    #[test]
    fn arm_disarm_roundtrip() {
        let mut p = Pager::default();
        assert!(p.pending_before_rows.is_none());
        p.arm(42);
        assert_eq!(p.pending_before_rows, Some(42));
        p.disarm();
        assert!(p.pending_before_rows.is_none());
        assert!(!p.active);
    }

    #[test]
    fn char_input_turns_arm_like_line_input_turns() {
        // SQ-0539, per the directive "[more] should work any time output is larger
        // than what fits on the screen (including boot) … we should behave as the
        // original game intended". The v1 (SQ-0404) rule armed only behind a LINE
        // read, so a game that dumped three screens and then waited on a keypress
        // scrolled the first two away unread. Both reads arm now; only a
        // non-input Glulx `glk_select` (Event) stays out of scope.
        assert!(should_arm(InputKind::Line, false));
        assert!(should_arm(InputKind::Char, false));
        assert!(!should_arm(InputKind::Event, false));
    }

    #[test]
    fn v6_never_print_more_vetoes_every_turn_kind() {
        // SQ-0534 (app half) / ZMSD §8.8.3.2.6: a line count of -999 means "never
        // print [MORE]" — Zork Zero's demonstration mode. It outranks everything.
        for pending in [InputKind::Line, InputKind::Char, InputKind::Event] {
            assert!(!should_arm(pending, true), "{pending:?} must not arm under the v6 veto");
        }
        let mut p = Pager::default();
        p.arm_after_turn(7, InputKind::Char, true, Driver::PlayerInput);
        assert!(p.pending_before_rows.is_none(), "the veto leaves the pager unarmed");
    }

    #[test]
    fn screen_clearing_turns_are_no_longer_excluded() {
        // SQ-0539: v1 refused to arm on any turn that cleared the screen, so a
        // menu/hint page that CLEARS and then paints more than a screenful showed
        // only its tail. `arm_after_turn` no longer looks at the clear at all —
        // the clear preserves scrollback and re-anchors, so "rows added by this
        // turn" already measures the post-clear repaint by itself, and
        // `activation_target` makes the fits/overflows call:
        //   clear + fits     → no pager (nothing was missed)
        //   clear + overflows→ page it
        let mut p = Pager::default();
        p.arm_after_turn(100, InputKind::Char, false, Driver::PlayerInput);
        assert_eq!(p.pending_before_rows, Some(100), "a clearing turn arms like any other");
        // 6 post-clear rows into a 10-row viewport: fits, nothing to page.
        assert_eq!(activation_target(100, 106, 10, 0), None);
        // 26 post-clear rows into the same viewport: page from the first screenful.
        assert_eq!(activation_target(100, 126, 10, 0), Some(16));
    }

    #[test]
    fn a_timeout_never_reloads_or_dismisses_the_pager() {
        // SQ-0539, mirroring the engine's v6 line-count rule (`Machine::
        // v6_reload_line_counts` — Frotz reloads on a real keystroke, never on a
        // timeout): a timed-input interrupt / Glk timer / sound-finish routine
        // must not re-baseline the pager, and must never dismiss an active one.
        let mut p = Pager::default();
        p.arm_after_turn(10, InputKind::Char, false, Driver::PlayerInput);
        // A timeout lands on top: the baseline stays where the real input set it,
        // so the timeout's own output accumulates toward the same measurement.
        p.arm_after_turn(30, InputKind::Char, false, Driver::Timeout);
        assert_eq!(p.pending_before_rows, Some(10), "a timeout is not a keystroke");
        // A timeout that leaves the game in a non-arming state must not disarm
        // either (a real interaction would).
        p.arm_after_turn(30, InputKind::Event, false, Driver::Timeout);
        assert_eq!(p.pending_before_rows, Some(10));
        p.arm_after_turn(30, InputKind::Event, false, Driver::PlayerInput);
        assert!(p.pending_before_rows.is_none(), "a real interaction cancels a stale arm");

        // While the pager is showing, NOTHING re-arms or re-parks it — the player
        // is mid-catch-up and a fresh baseline would jump the view.
        let mut p = Pager { active: true, pending_before_rows: None };
        p.arm_after_turn(50, InputKind::Char, false, Driver::Timeout);
        assert!(p.pending_before_rows.is_none());
        p.arm_after_turn(50, InputKind::Line, false, Driver::PlayerInput);
        assert!(p.pending_before_rows.is_none());
        assert!(p.active, "and it stays up until the view catches up");
    }

    #[test]
    fn an_unarmed_timeout_still_measures_its_own_output() {
        // The other half of the rule: with no standing baseline (the player has
        // been idle), a timed interrupt that dumps a screenful is still paged —
        // it just takes the current row count as its baseline.
        let mut p = Pager::default();
        p.arm_after_turn(12, InputKind::Char, false, Driver::Timeout);
        assert_eq!(p.pending_before_rows, Some(12));
    }
}
