//! SQ-1220: a Glk proportional split divides its parent the way a GUI
//! interpreter divides it, so the window tree covers the whole story pane.
//!
//! City of Secrets splits its graphics column off the LEFT with
//! `winmethod_Left | winmethod_Proportional`, size 15. gvm used to make such a
//! split land on whole cells by shrinking the WHOLE SCREEN until every
//! proportional split in the tree divided exactly — 15 % wants content ≡ 0
//! (mod 20), so an 80-column pane was laid out at 61 and `/dump-windows` read
//! "Window layout (61x30)" with nineteen columns covered by no window at all.
//!
//! Gargoyle and Spatterlight have no such problem because they split in PIXELS
//! and each text window reports `floor(px / char_px)` characters, absorbing the
//! sub-character slack as a gutter. gvm now does the same per split: divide the
//! parent's content in virtual pixels, floor each child to whole cells
//! independently, and leave the at-most-one-cell remainder in the split. Two
//! consequences this suite pins against real games:
//!
//!   * City of Secrets covers its pane — the fix, and the falsifier: restore
//!     "the other child takes the remainder" and the text stops at 61 of 80.
//!   * Counterfeit Monkey — the Inform 7 status/graphics layout whose rounding
//!     the old snapping was built for — still reaches an input request at odd
//!     pane sizes and across resizes, i.e. the turn watchdog never fires.
//!
//! (Read at 79 columns with the snap disabled, CM's arrange handler is a
//! straight repaint: `glk_window_get_size` on its status grid, `move_cursor`,
//! done. It never calls `glk_window_set_arrangement` and never compares a child
//! against a percentage, and its only proportional split is the figure window at
//! 0 %, exact at every size. The equal-halves property below is what the
//! original report was about, and it now holds at every parent size.)

use app::engine::{Engine, KeyInput, WinKind};
use app::state::AppState;
use blorb::Blorb;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::fixture_paths::fixture_path;

/// Boot a Glulx blorb into a `cols`x`rows` pane, or `None` when the gitignored
/// fixture is absent. Three blank keypresses past any title card, then each
/// command in `cmds`.
fn boot(name: &str, cols: u32, rows: u32, cmds: &[&str]) -> Option<app::glulx_session::GlulxSession> {
    let raw = std::fs::read(fixture_path(name)).ok()?;
    let image = Blorb::parse(raw.clone()).ok()?.executable().ok()?.1.to_vec();
    let resources = Blorb::parse(raw).ok()?;
    let mut sess = app::glulx_session::GlulxSession::new(
        image,
        cols,
        rows,
        true,
        true,
        false,
        (8, 16),
        Some(resources),
        &[],
    )
    .expect("the story should load and boot");
    let _ = Engine::take_transcript(&mut sess);
    for _ in 0..3 {
        Engine::submit_key(&mut sess, KeyInput::Char(' '));
    }
    for c in cmds {
        let _ = Engine::submit(&mut sess, c);
    }
    Some(sess)
}

/// The rects the RENDER actually painted, which is what the player sees — gvm's
/// own layout rect reserves a border gutter the theme may not draw (SQ-1203) and
/// stops at the snap margin (SQ-1220).
fn win_rects(sess: &app::glulx_session::GlulxSession, cols: u16, rows: u16) -> Vec<(u32, WinKind, Rect)> {
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let model = Engine::screen(sess);
    let state = AppState::default();
    app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf).win_rects
}

/// City of Secrets, 80x30 pane, 3 keypresses + `help` (the menu frame SQ-1203
/// and SQ-1212 also drive): the text windows are drawn out to column 80, the
/// graphics column keeps the width its own pixel share bought, and the layout
/// leaves no more than the separator and one padding cell uncovered.
#[test]
fn city_of_secrets_covers_its_pane() {
    let Some(sess) = boot("CoS.blb", 80, 30, &["help"]) else {
        return; // gitignored fixture absent (CI): skip vacuously
    };
    let rects = win_rects(&sess, 80, 30);

    // Non-vacuity: the frame must actually have the shape this case is about —
    // a graphics column on the left and text windows beside it.
    let (gfx, text): (Vec<_>, Vec<_>) =
        rects.iter().partition(|(_, k, _)| *k == WinKind::Graphics);
    assert_eq!(gfx.len(), 1, "CoS's help frame should carry one graphics column: {rects:?}");
    assert!(!text.is_empty(), "CoS's help frame should carry text windows: {rects:?}");

    for (id, kind, r) in &text {
        assert_eq!(
            r.x + r.width,
            80,
            "text window {id} ({kind:?}) is drawn only to column {} of an 80-column pane: {rects:?}",
            r.x + r.width
        );
    }

    // The graphics column is not stretched to fill anything: 15 % of the 79-cell
    // content is 94 virtual pixels, which floors to 11 cells at an 8px cell.
    let (_, _, g) = gfx[0];
    assert_eq!(g.x, 0, "the graphics column stays at the left edge: {rects:?}");
    assert_eq!(g.width, 11, "the graphics column is floor(15% of the content px): {rects:?}");

    // And what the GAME is told covers the pane too, but for the separator and
    // the single cell the split could not divide. `/dump-windows` prints gvm's
    // own rects — the field `glk_window_get_size` answers from.
    let dump = Engine::window_dump(&sess).join("\n");
    assert!(
        dump.contains("Window layout (80x30):"),
        "the whole pane must be laid out, not a snapped sub-rect:\n{dump}"
    );
    // 15 % of 79 content cells is 94 px -> 11 cells; the other 617 px -> 67 cells.
    // 11 + 67 = 78 of the 79 content cells, so the separator and one padding cell
    // are all that is left uncovered of the 80.
    assert!(
        dump.contains("11x30") && dump.contains("67x24") && dump.contains("67x5"),
        "each child should hold the floor of its own pixel share (11 | 67):\n{dump}"
    );
}

/// Counterfeit Monkey at odd pane sizes still reaches an input request. The
/// watchdog aborts a runaway turn by quitting the session (`drive`'s budget), so
/// a live session after several turns is the assertion that no layout loop was
/// hit — this is the hang the whole-screen snapping was built for, and which
/// SQ-1220 must not reopen.
///
/// Deliberately NOT setting `LANTHORN_TURN_BUDGET_MS` to shorten that budget:
/// it is read from the process environment, which under `cargo test` is one
/// process shared by every case in this group binary, and a 2 s budget imposed
/// on a sibling booting a large story is exactly the kind of process-global
/// cross-talk `palette_lock_discipline` exists to prevent. The quit flag IS the
/// watchdog's signal; the default budget only costs time when this assertion is
/// already about to fail.
#[test]
fn counterfeit_monkey_does_not_loop_at_odd_pane_sizes() {
    let mut booted = 0;
    for &(cols, rows) in &[(79u32, 30u32), (81, 31), (101, 33), (80, 30)] {
        let Some(mut sess) = boot("CounterfeitMonkey-11.gblorb", cols, rows, &["look", "x me"]) else {
            return; // gitignored fixture absent (CI): skip vacuously
        };
        booted += 1;
        assert!(
            !Engine::has_quit(&sess),
            "Counterfeit Monkey's turn watchdog fired at {cols}x{rows} — a layout loop"
        );
        // And a further turn after a resize, since a resize is a fresh relayout
        // at a size the game never chose.
        sess.resize(cols + 1, rows);
        let _ = Engine::submit(&mut sess, "look");
        assert!(
            !Engine::has_quit(&sess),
            "Counterfeit Monkey's turn watchdog fired after a resize to {}x{rows}",
            cols + 1
        );
    }
    assert_eq!(booted, 4, "every pane size should have been exercised");
}
