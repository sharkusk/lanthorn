//! Is `PAINT_LOG_CAP` big enough? Measured across every v6 game we have — SQ-0588 follow-up.
//!
//! A window whose display list hits the cap is dropped from replay: it still restores
//! (its canvas falls back to a PNG) but its colours stop following palette changes, and
//! the only signal is a `/dump-windows` line. So the cap being adequate is load-bearing,
//! and it was picked by reasoning rather than measurement.
//!
//! The number that matters is NOT the peak. It is whether the peak GROWS with play.
//! `zvm`'s own paint log (SQ-1403) resets a window's list to a single entry whenever a
//! whole-canvas `erase_window` supersedes everything before it — a game that swaps
//! screens therefore plateaus, and is safe for a session of any length. A game that only
//! ever appends would overflow eventually, and a larger cap would just be a bigger number
//! before the same failure. Sampling early and late tells those two apart; a single peak
//! cannot.
//!
//! Skip-if-missing per the other gitignored-story smokes.


use app::graphics::PictSource;
use app::session::{GameSession, InputKind};
use zvm::paint_log::PAINT_LOG_CAP;

use crate::fixture_paths::fixture_path;


/// Every v6 story in the fixture set, with a rotation of inputs that keeps it drawing.
/// The verbs are deliberately generic — an unrecognised one still costs a turn and a
/// redraw, which is all the measurement needs.
const GAMES: &[(&str, &str)] = &[
    ("Arthur", "arthur-r74-s890714.z6"),
    ("Zork Zero", "zork0-r393-s890714.z6"),
    ("Shogun", "shogun-r322-s890706.z6"),
    ("Journey", "journey-r83-s890706.z6"),
    ("advent", "advent.z6"),
];

const MOVES: &[&str] = &["look", "n", "s", "e", "w", "wait", "u", "d", "in", "out"];

fn boot(file: &str) -> Option<GameSession> {
    let p = fixture_path(file);
    let bytes = std::fs::read(&p).ok()?;
    let mut pic = PictSource::new(blorb::resolve_resource_blorb(&p).map(|(b, _)| b));
    let dims = pic.all_pict_dims();
    let mut s =
        GameSession::new_with_trace(bytes, true, false, None, false, dims, pic.std_window(), None, None).ok()?;
    s.set_pict_source(Some(pic));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    // Through the boot banner / "press a key" screens.
    for _ in 0..12 {
        let r = match s.pending_input() {
            InputKind::Line => s.submit(""),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = s.submit_char(b'n');
        }
    }
    Some(s)
}

/// Play `turns` turns, returning the longest display list seen along the way and the
/// number of windows sitting at the cap.
fn play(s: &mut GameSession, turns: usize) -> (usize, usize) {
    let (mut peak, mut at_cap) = s.display_ops_extent();
    for i in 0..turns {
        match s.pending_input() {
            InputKind::Line => {
                let _ = s.submit(MOVES[i % MOVES.len()]);
            }
            InputKind::Char => {
                let _ = s.submit_char(13);
            }
            InputKind::Event => {
                let _ = s.submit("");
            }
        }
        let _ = s.take_transcript();
        let (p, c) = s.display_ops_extent();
        peak = peak.max(p);
        at_cap = at_cap.max(c);
    }
    (peak, at_cap)
}

#[test]
fn no_v6_game_comes_close_to_the_display_list_cap() {
    let mut measured = 0;
    let mut worst = 0;
    for (name, file) in GAMES {
        let Some(mut s) = boot(file) else {
            eprintln!("SKIP {name}: gitignored story missing");
            continue;
        };
        measured += 1;

        // Three sample points, because the question is the SHAPE of the growth, not
        // the peak: a game that resets its list plateaus and is safe at any session
        // length, while one that accumulates would cross the cap eventually and a
        // bigger cap would only postpone it.
        let (early, early_cap) = play(&mut s, 10);
        let (mid, mid_cap) = play(&mut s, 30);
        let (late, late_cap) = play(&mut s, 160);
        let peak = early.max(mid).max(late);
        worst = worst.max(peak);

        eprintln!(
            "{name}: longest display list {early} ops @10 turns, {mid} @40, {late} @200 \
             (cap {PAINT_LOG_CAP}, {}% used)",
            peak * 100 / PAINT_LOG_CAP
        );

        assert_eq!(
            (early_cap, mid_cap, late_cap),
            (0, 0, 0),
            "{name}: a window hit the {PAINT_LOG_CAP}-op cap during ordinary play — it drops out of \
             replay, so its art stops following palette changes for the rest of the session."
        );
        assert!(
            peak * 4 < PAINT_LOG_CAP,
            "{name}: longest list {peak} is within 4x of the {PAINT_LOG_CAP}-op cap — not a failure \
             yet, but thin enough that a longer session could reach it."
        );
        // The one that actually matters. Shogun re-erased the same two regions every
        // turn and reached 266 ops by turn 200 (SQ-0592); pruning erases superseded by
        // a covering erase flattened it to 5. Twenty times the turns must not grow the
        // list — a game whose screen is stable should hold a near-constant list.
        assert!(
            late <= early.max(4) * 2,
            "{name}: the display list grew from {early} at turn 10 to {late} at turn 200 — that is \
             accumulation, not a plateau, so it would overflow in a long session however large the \
             cap is. Something this game repeats is not being pruned."
        );
    }
    if measured == 0 {
        eprintln!("SKIP: no v6 stories present");
        return;
    }
    eprintln!("measured {measured} v6 game(s); worst case {worst}/{PAINT_LOG_CAP} ops");
}
