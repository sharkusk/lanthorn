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

use app::colors::ColorScheme;
use app::engine::{Engine, GridWindow, KeyInput, WinKind};
use app::render::paneframe::{BorderStyle, PaneSides};
use app::render::upper_window::draw_grid;
use app::state::{pack_zcolour, AppState};
use blorb::Blorb;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use zvm::screen::ZColour;

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
    // The assertions below name colours, so the palette they resolve through is
    // stated rather than inherited from the last suite in this binary (SQ-0958).
    let _g = app::v6_palette(zvm::screen::Palette::Standard);
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

// ── SQ-1219: styled Glk grid cells keep the ground's own background ───────────
//
// SQ-1212 grounded a Glk grid on reversed chrome (`Modifier::REVERSED`). Laying
// an explicit colour on top of a REVERSED style with `.fg()`/`.bg()`/`.patch()`
// sets the literal channel, and the terminal's single swap then puts it on the
// OTHER side of the character than intended — City of Secrets' `help` menu
// patches the themed `hyperlink` accent fg onto that ground, and it landed as a
// teal BACKGROUND patch behind unreadable text instead of teal link text on the
// ground's own background.

/// A cell's *effective* background — what the terminal actually paints, after
/// performing the swap `Modifier::REVERSED` asks for. Ratatui's `Style::bg` is
/// the literal, pre-swap field, so a "does it blend with the ground" assertion
/// has to resolve through the modifier the way a real terminal would, exactly
/// like the bug this quest fixes.
fn effective_bg(cell: &ratatui::buffer::Cell) -> Option<Color> {
    let s = cell.style();
    if s.add_modifier.contains(Modifier::REVERSED) { s.fg } else { s.bg }
}

/// The real-game regression: the help menu's "Tutorial" hyperlink must blend
/// into the reversed ground exactly like an unwritten cell beside it — same
/// effective background — rather than showing a patch of its own.
///
/// Falsify: reverting the `concretize_reversed` call at the hyperlink-patch
/// site in `draw_grid` reproduces the reported symptom — the linked cell's
/// effective background becomes the theme's `hyperlink` accent colour (a teal
/// patch) instead of the ground's, and this test fails.
fn help_menu_hyperlink_blends_with_ground(honor_game_colours: bool) {
    // The assertions below name colours, so the palette they resolve through is
    // stated rather than inherited from the last suite in this binary (SQ-0958).
    let _g = app::v6_palette(zvm::screen::Palette::Standard);
    let Some(sess) = boot_cos_help_menu() else {
        eprintln!("SKIP: no CoS.blb");
        return;
    };
    let mut state = AppState::default();
    state.config.honor_game_colours = honor_game_colours;
    let (buf, win_rects) = render(&sess, &state);
    let area_width = 80u16;
    let area_height = 30u16;

    let (_, _, grid_rect) = win_rects
        .iter()
        .copied()
        .find(|&(_, kind, _)| kind == WinKind::Grid)
        .unwrap_or_else(|| panic!("help menu should draw a Grid window: {win_rects:?}"));

    let row = (grid_rect.y..grid_rect.y + grid_rect.height)
        .find(|&y| row_text(&buf, y, area_width).contains("Tutorial"))
        .unwrap_or_else(|| {
            panic!("menu should show a Tutorial item:\n{}", dump(&buf, area_width, area_height))
        });
    let text: Vec<char> = row_text(&buf, row, area_width).chars().collect();
    let link_x = text.iter().position(|&c| c == 'T').expect("Tutorial should start with T") as u16;
    let linked_cell = buf.cell((link_x, row)).expect("linked cell must be in the buffer");
    assert!(
        linked_cell.modifier.contains(Modifier::UNDERLINED),
        "sanity: the 'T' of Tutorial must be the linked, styled cell"
    );

    // A ground cell on the SAME row, past the item's own text — proves the
    // comparison is apples-to-apples (same row, same theme resolution).
    let ground_x = (grid_rect.x..grid_rect.x + grid_rect.width)
        .find(|&x| {
            let i = x as usize;
            i >= text.len() || (text[i] == ' ' && x > link_x)
        })
        .unwrap_or_else(|| {
            panic!("row should have a blank ground cell past the item text: {text:?}")
        });
    let ground_cell = buf.cell((ground_x, row)).expect("ground cell must be in the buffer");

    assert_eq!(
        effective_bg(linked_cell),
        effective_bg(ground_cell),
        "the hyperlinked 'Tutorial' item must blend into the ground's own background \
         (linked cell at x={link_x}, ground cell at x={ground_x}, row {row}), not show a \
         patch of its own:\n{}",
        dump(&buf, area_width, area_height)
    );
}

#[test]
fn help_menu_hyperlink_blends_with_ground_honor_on() {
    help_menu_hyperlink_blends_with_ground(true);
}

#[test]
fn help_menu_hyperlink_blends_with_ground_honor_off() {
    help_menu_hyperlink_blends_with_ground(false);
}

/// A synthetic Glk grid, no border, centred at (0,0) — the minimal setup a
/// `draw_grid` cell-level test needs.
fn make_glk_grid_colors() -> ColorScheme {
    let mut colors = ColorScheme::terminal_default();
    colors.virtual_window_border = BorderStyle::None;
    colors.upper_window_border_sides = PaneSides::all(BorderStyle::None);
    colors
}

/// The theme's per-Glk-style slot must never paint a Glk grid cell's own
/// BACKGROUND — only the ground itself, or a colour the GAME set, may. A
/// themed slot bg (the same shape a `garglk.ini` import can produce) would
/// otherwise paint its own patch exactly like the hyperlink accent did.
fn glk_grid_theme_slot_bg_does_not_paint(honor_game_colours: bool) {
    // The assertions below name colours, so the palette they resolve through is
    // stated rather than inherited from the last suite in this binary (SQ-0958).
    let _g = app::v6_palette(zvm::screen::Palette::Standard);
    let mut upper = GridWindow { win: 5, ..GridWindow::default() };
    upper.resize(1, 3);
    upper.put(1, 1, 'X', 0);
    upper.cells[0].glk_style = 6; // "note" — distinct from Normal(0)

    let mut colors = make_glk_grid_colors();
    // A theme (or garglk.ini import) setting the grid row's Note slot bg.
    colors.glk_styles[1][6] = ratatui::style::Style::default().bg(Color::Rgb(0, 128, 128));

    let area = Rect::new(0, 0, 3, 1);
    let mut buf = Buffer::empty(area);
    draw_grid(&upper, 1, (1, 1), false, &colors, area, &mut buf, honor_game_colours, &mut Vec::new());

    let ground = colors.theme.get("glk.grid.background").style;
    assert_eq!(
        buf.cell((0, 0)).unwrap().style().bg,
        ground.bg,
        "a Glk grid cell with no GAME colour must keep the ground's own bg \
         (honor_game_colours = {honor_game_colours}), not the theme's per-style slot bg"
    );
}

#[test]
fn glk_grid_theme_slot_bg_does_not_paint_honor_on() {
    glk_grid_theme_slot_bg_does_not_paint(true);
}

#[test]
fn glk_grid_theme_slot_bg_does_not_paint_honor_off() {
    glk_grid_theme_slot_bg_does_not_paint(false);
}

/// A GAME-set background is the one colour allowed to replace the ground —
/// and it must land as a literal background, not get swapped into the
/// foreground by the ground's REVERSED modifier. Bold (a modifier, not a
/// colour channel) must survive untouched alongside it.
fn glk_grid_explicit_game_bg_is_bg_not_swapped(honor_game_colours: bool) {
    // The assertions below name colours, so the palette they resolve through is
    // stated rather than inherited from the last suite in this binary (SQ-0958).
    let _g = app::v6_palette(zvm::screen::Palette::Standard);
    let mut upper = GridWindow { win: 5, ..GridWindow::default() };
    upper.resize(1, 3);
    upper.put(1, 1, 'X', 0x02); // bold
    let teal = 0x00_11_88_88;
    upper.cells[0].bg = pack_zcolour(ZColour::True24(teal));

    let colors = make_glk_grid_colors();
    let area = Rect::new(0, 0, 3, 1);
    let mut buf = Buffer::empty(area);
    draw_grid(&upper, 1, (1, 1), false, &colors, area, &mut buf, honor_game_colours, &mut Vec::new());

    let style = buf.cell((0, 0)).unwrap().style();
    let teal_rgb = Color::Rgb(0x11, 0x88, 0x88);
    if honor_game_colours {
        assert_eq!(style.bg, Some(teal_rgb), "an explicit game bg must land as bg, not fg");
        assert_ne!(style.fg, Some(teal_rgb), "…and must not ALSO land as fg via an unswapped REVERSED");
        assert!(
            !style.add_modifier.contains(Modifier::REVERSED),
            "a concrete game bg must not be carried under a REVERSED modifier — \
             that would swap it again at render time"
        );
    } else {
        // honor_game_colours off: the game's own colour is ignored entirely — the
        // cell stays on the ground, same as an unwritten cell (SQ-0331's gate).
        let ground = colors.theme.get("glk.grid.background").style;
        assert_eq!(style.bg, ground.bg, "honor off: the game bg must be ignored, ground bg wins");
    }
    assert!(style.add_modifier.contains(Modifier::BOLD), "bold must survive alongside the colour fix");
}

#[test]
fn glk_grid_explicit_game_bg_is_bg_not_swapped_honor_on() {
    glk_grid_explicit_game_bg_is_bg_not_swapped(true);
}

#[test]
fn glk_grid_explicit_game_bg_is_bg_not_swapped_honor_off() {
    glk_grid_explicit_game_bg_is_bg_not_swapped(false);
}
