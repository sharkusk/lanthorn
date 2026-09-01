//! SQ-1212: a Glk grid window's unwritten ground reads as reversed chrome —
//! the status bar's own spelling — instead of page-on-page.
//!
//! Before the fix, `draw_grid`'s blank-fill path and `cell_style`'s Default-cell
//! base both grounded EVERY grid window (Glulx or Z-machine) on the theme's
//! `upper_window` selector, which deliberately follows the terminal page
//! (SQ-0510). That is correct for a Z-machine upper window (which paints its
//! own reversal when it wants one), but leaves a Glk text-grid window — a
//! mouse-driven menu, e.g. City of Secrets' `help` screen — completely
//! invisible against a terminal whose page matches the theme: no border rule
//! is drawn by default (SQ-0821), so the menu's own extent has no visible edge
//! at all.
//!
//! The fix grounds a Glk grid (`GridWindow::win != 0`, real per SQ-1203) on the
//! new `glk.grid.background` registry row — reversed chrome, parented exactly
//! like `status_bar`/`help_bar` — while a Z-machine/Scott grid (`win == 0`)
//! keeps `upper_window`, untouched.
//!
//! This suite drives the same CoS `help` menu
//! `glulx_mouse_hyperlink_drawn_rect.rs` already boots (SQ-1203), so the two
//! together prove both the click hit-test AND the ground fix work over the
//! identical real screen.

use app::engine::{Engine, KeyInput, WinKind};
use app::state::AppState;
use blorb::Blorb;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use crate::fixture_paths::fixture_path;

/// Boot City of Secrets to its `help` menu (3 blank keypresses past the title,
/// then `help`), or `None` when the gitignored fixture is absent. Mirrors
/// `glulx_mouse_hyperlink_drawn_rect::boot_cos_help_menu` — duplicated rather
/// than shared so this suite stays a self-contained file the way
/// `tests/suites/` files are added (one file, one `mod` line).
fn boot_cos_help_menu() -> Option<app::glulx_session::GlulxSession> {
    let path = fixture_path("CoS.blb");
    let raw = std::fs::read(&path).ok()?;
    let blorb1 = Blorb::parse(raw.clone()).ok()?;
    let (_, image) = blorb1.executable().ok()?;
    let image = image.to_vec();
    let blorb2 = Blorb::parse(raw).ok()?;
    let mut sess =
        app::glulx_session::GlulxSession::new(image, 80, 30, true, true, false, (8, 16), Some(blorb2), &[])
            .expect("CoS should load and boot");
    let _ = Engine::take_transcript(&mut sess);
    for _ in 0..3 {
        Engine::submit_key(&mut sess, KeyInput::Char(' '));
    }
    let _ = Engine::submit(&mut sess, "help");
    Some(sess)
}

fn render(
    sess: &app::glulx_session::GlulxSession,
    state: &AppState,
) -> (Buffer, Vec<(u32, WinKind, Rect)>) {
    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    let model = Engine::screen(sess);
    let m = app::render::screen::render_story_pane(&model, false, None, state, area, &mut buf);
    (buf, m.win_rects)
}

fn row_text(buf: &Buffer, y: u16, width: u16) -> String {
    (0..width)
        .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
        .collect()
}

fn dump(buf: &Buffer, w: u16, h: u16) -> String {
    (0..h).map(|y| row_text(buf, y, w) + "\n").collect()
}

/// Drive the help menu, find its Glk grid window's drawn rect, and assert:
/// (a) an unwritten ground cell inside that rect carries the REVERSED modifier
///     — the visible chrome band this quest adds — and
/// (b) the menu's own written text ("Tutorial") is still legible: its glyph is
///     drawn (not blanked by the new ground), proving the ground fill only
///     covers cells the game never wrote.
///
/// Falsify: reverting `draw_grid`'s `is_glk_grid` selection (both paths ground
/// on `upper_window`) reproduces the invisible-ground symptom — no ground cell
/// in the grid's rect carries REVERSED — and this test fails on assertion (a).
fn help_menu_ground_is_reversed(honor_game_colours: bool) {
    let Some(sess) = boot_cos_help_menu() else {
        eprintln!("SKIP: no CoS.blb");
        return;
    };
    let mut state = AppState::default();
    state.config.honor_game_colours = honor_game_colours;
    let (buf, win_rects) = render(&sess, &state);
    let area_width = 80u16;
    let area_height = 30u16;

    // The help menu is drawn through a Glk text-grid window (a real Glk id,
    // per SQ-1203's `win` field) — find its drawn rect.
    let (_, _, grid_rect) = win_rects
        .iter()
        .copied()
        .find(|&(_, kind, _)| kind == WinKind::Grid)
        .unwrap_or_else(|| panic!("help menu should draw a Grid window: {win_rects:?}"));

    // Locate the "Tutorial" menu item within the grid's own rect, to prove it
    // is still legible under the new ground.
    let row = (grid_rect.y..grid_rect.y + grid_rect.height)
        .find(|&y| row_text(&buf, y, area_width).contains("Tutorial"))
        .unwrap_or_else(|| {
            panic!("menu should show a Tutorial item:\n{}", dump(&buf, area_width, area_height))
        });
    assert!(
        row_text(&buf, row, area_width).contains("Tutorial"),
        "the menu's own text must still render legibly over the new ground"
    );

    // A ground cell: somewhere in the grid's rect the game never wrote — a
    // blank column past the menu's own text width, on the SAME row as the
    // legible "Tutorial" text, so the game-written cell above already proves
    // it isn't blanked. Scan the grid's rect for a space cell and require it
    // to carry REVERSED — the visible chrome band this quest adds.
    let mut found_reversed_ground = false;
    for y in grid_rect.y..grid_rect.y + grid_rect.height {
        for x in grid_rect.x..grid_rect.x + grid_rect.width {
            let Some(cell) = buf.cell((x, y)) else { continue };
            if cell.symbol() == " " && cell.modifier.contains(Modifier::REVERSED) {
                found_reversed_ground = true;
            }
        }
    }
    assert!(
        found_reversed_ground,
        "an unwritten cell inside the Glk grid's rect {grid_rect:?} must carry REVERSED \
         (glk.grid.background) — none did, the invisible-ground symptom:\n{}",
        dump(&buf, area_width, area_height)
    );
}

#[test]
fn help_menu_ground_is_reversed_honor_on() {
    help_menu_ground_is_reversed(true);
}

#[test]
fn help_menu_ground_is_reversed_honor_off() {
    help_menu_ground_is_reversed(false);
}
