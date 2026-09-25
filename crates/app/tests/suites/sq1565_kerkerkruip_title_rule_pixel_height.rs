//! SQ-1565: a fixed/proportional graphics split's pixel-exact footprint
//! (gvm's `window_pixel_size`) reaches the app's `WinNode::Pair`/`Split`
//! (`Split::fixed_px`) and a graphics window's canvas allocation, instead of
//! being discarded at the cell-rounding gvm's own layout does for the TUI.
//!
//! Reported symptom: Kerkerkruip's 2-3px coloured divider rules under its
//! panel titles rendered as a solid full-text-row-high block (240x16 at the
//! shipped 8x16 cell) instead of a thin rule, because the pixel height the
//! game actually requested got lost on the way to the canvas the game draws
//! into — it was allocated `cells × char_px` tall (a whole row) rather than
//! the handful of pixels the split itself was fixed to.
//!
//! `scratch_dump_kerkerkruip_tree` (throwaway, not committed) confirmed the
//! real specimen: at `CELL_PX = (13, 29)`, Kerkerkruip's window tree carries
//! several `Pair`s whose FIRST child is a graphics window with
//! `split.fixed = 1` (one cell reserved for layout) and `split.fixed_px`
//! `Some(1)`, `Some(2)` or `Some(7)` — real title-rule/divider windows a
//! fraction of the 29px cell tall. Window ids drift across a real session
//! (see `sq1515_kerkerkruip_restore_arrange.rs`'s module doc), so this finds
//! its specimen by SHAPE (a thin fixed graphics split), not by a hard-coded id.

use std::path::{Path, PathBuf};
use std::time::Duration;

use app::colors::ColorScheme;
use app::engine::{Engine, GraphicsWindow, KeyInput, Split, WinNode};
use app::glk_backend::GlkStylePairs;
use app::glulx_session::GlulxSession;
use app::session::InputKind;

use crate::fixture_paths::fixture_path;

const STORY: &str = "Kerkerkruip.gblorb";
const COLS: u16 = 225;
const ROWS: u16 = 51;
const CELL_PX: (u32, u32) = (13, 29);
const POLL_CAP: usize = 8000;
const TURN_BUDGET: Duration = Duration::from_secs(600);

fn story_path() -> Option<PathBuf> {
    let p = fixture_path(STORY);
    if !p.is_file() {
        eprintln!("SKIP: fixture missing at {}", p.display());
        return None;
    }
    if !p.with_file_name("Kerkerkruip.ini").is_file() {
        eprintln!("SKIP: needs the shipped Kerkerkruip.ini beside the story");
        return None;
    }
    Some(p)
}

fn theme_pairs_for(path: &Path) -> GlkStylePairs {
    let mut cs = ColorScheme::default();
    if let Some(ov) = app::garglk_ini::discover(path) {
        ov.apply(&mut cs);
    }
    app::glk_backend::theme_style_colours(&cs)
}

fn boot(path: &Path) -> GlulxSession {
    let bytes = std::fs::read(path).expect("read the story");
    let blorb = blorb::Blorb::parse(bytes).expect("Kerkerkruip is a Blorb");
    let image = blorb.executable().expect("Glulx exec chunk").1.to_vec();
    let mut sess = GlulxSession::new_in(
        PathBuf::new(),
        image,
        COLS as u32,
        ROWS as u32,
        true,
        true,
        false,
        false,
        CELL_PX,
        Some(blorb),
        &[],
        theme_pairs_for(path),
        false,
        None,
    )
    .expect("Kerkerkruip boots");
    sess.set_turn_budget(TURN_BUDGET);
    sess
}

fn settle(sess: &mut GlulxSession, done: impl Fn(&GlulxSession) -> bool) -> bool {
    for _ in 0..POLL_CAP {
        if done(sess) {
            return true;
        }
        let can_advance =
            sess.timer_interval().is_some() || Engine::pending_input(sess) == InputKind::Event;
        if !can_advance {
            return done(sess);
        }
        let _ = sess.deliver_timer();
    }
    done(sess)
}

fn into_gameplay() -> Option<GlulxSession> {
    let mut sess = boot(&story_path()?);
    settle(&mut sess, |s| Engine::pending_input(s) != InputKind::Event);
    let _ = Engine::submit_key(&mut sess, KeyInput::Char(' '));
    settle(&mut sess, |s| Engine::pending_input(s) == InputKind::Line);
    assert_eq!(
        Engine::pending_input(&sess),
        InputKind::Line,
        "reached the parser prompt"
    );
    let _ = Engine::submit(&mut sess, "look");
    settle(&mut sess, |s| Engine::pending_input(s) != InputKind::Event);
    assert_eq!(
        Engine::pending_input(&sess),
        InputKind::Line,
        "survives a `look` turn"
    );
    Some(sess)
}

/// Every `(vertical, enclosing Split, first-child Graphics window)` triple
/// anywhere in the tree, found by SHAPE — a fixed/proportional split whose
/// first (top/left) child is a graphics window, exactly Kerkerkruip's
/// title-rule/divider shape (see module doc). Window ids drift across a
/// session, so nothing here is hard-coded. `vertical` matters because
/// `Split::fixed_px` is a SCALAR on the split axis: an Above/Below split
/// (`vertical`) constrains the graphics window's HEIGHT, a Left/Right split
/// constrains its WIDTH — Kerkerkruip has both kinds of thin divider.
fn graphics_first_splits(sess: &GlulxSession) -> Vec<(bool, Split, GraphicsWindow)> {
    fn walk(n: &WinNode, out: &mut Vec<(bool, Split, GraphicsWindow)>) {
        if let WinNode::Pair {
            vertical,
            split,
            first,
            second,
            ..
        } = n
        {
            if let WinNode::Graphics(g) = first.as_ref() {
                out.push((*vertical, *split, g.clone()));
            }
            walk(first, out);
            walk(second, out);
        }
    }
    let mut v = Vec::new();
    walk(&Engine::screen(sess).root, &mut v);
    v
}

/// The mechanism check: Kerkerkruip's thin title-rule/divider graphics
/// windows must report their real, sub-cell pixel footprint through
/// `Split::fixed_px` — not the whole cell `Split::fixed` reserves for layout
/// — and the CANVAS the game actually painted into (its own
/// `GraphicsWindow::canvas`, not a synthetic one) must be allocated at that
/// same pixel-exact size on the split axis, not a rounded-up whole-cell run.
#[test]
fn kerkerkruip_title_rule_reports_pixel_exact_height_not_a_rounded_up_cell() {
    let Some(sess) = into_gameplay() else { return };
    let splits = graphics_first_splits(&sess);
    assert!(
        !splits.is_empty(),
        "expected at least one fixed/proportional split led by a graphics window"
    );

    // Every thin divider the game actually painted (`fixed_px` known and
    // strictly narrower than the whole cell on its own axis; a canvas
    // dimension of exactly 1 on that axis is an unpainted 1×1 placeholder,
    // not a live specimen) — not just one: the fix must hold for both the
    // Above/Below title rules (height-constrained) and the Left/Right
    // dividers (width-constrained) this real game has of each kind.
    let cell_on_axis = |vertical: bool| if vertical { CELL_PX.1 } else { CELL_PX.0 };
    let rules: Vec<_> = splits
        .iter()
        .filter(|(vertical, split, g)| {
            let axis_px = if *vertical {
                g.canvas.height()
            } else {
                g.canvas.width()
            };
            split
                .fixed_px
                .is_some_and(|px| px > 0 && px < cell_on_axis(*vertical))
                && axis_px > 1
        })
        .collect();
    assert!(
        !rules.is_empty(),
        "expected a painted graphics window whose split reports a sub-cell pixel footprint; found: {:?}",
        splits.iter().map(|(v, s, g)| (g.win, v, s.fixed, s.fixed_px, g.canvas.dimensions())).collect::<Vec<_>>()
    );

    for (vertical, split, g) in &rules {
        let px = split.fixed_px.expect("filtered on is_some above");
        // The cell-based layout figure is unaffected by this fix — it still
        // reserves a whole cell for the split (`fixed`, not zero).
        assert!(
            split.fixed >= 1,
            "window {}: the cell-rounded layout footprint is unchanged by this fix",
            g.win
        );

        // The real bug: the window's own canvas — what the game actually
        // draws into and what the renderer actually displays — must be
        // exactly `px` pixels on the split axis, not
        // `split.fixed as u32 * cell_on_axis` (the rounded-up whole-cell
        // run that produced the reported "solid block").
        let rounded_up_cell = split.fixed as u32 * cell_on_axis(*vertical);
        assert_ne!(
            px, rounded_up_cell,
            "window {}: non-vacuity — pixel figure must differ from the cell one",
            g.win
        );
        let axis_px = if *vertical {
            g.canvas.height()
        } else {
            g.canvas.width()
        };
        assert_eq!(
            axis_px,
            px,
            "window {} ({}): canvas must be allocated at its exact {px}px request on the {} axis, not the rounded-up {rounded_up_cell}px cell run",
            g.win,
            if *vertical { "Above/Below title rule" } else { "Left/Right divider" },
            if *vertical { "height" } else { "width" },
        );
    }
}
