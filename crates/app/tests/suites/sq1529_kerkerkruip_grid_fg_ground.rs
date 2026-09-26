//! SQ-1529: a Glulx window that declared its own Normal-style foreground
//! (`GridWindow::fg`, from `glk_stylehint_set(wintype_TextGrid, style_Normal,
//! stylehint_TextColor, rgb)`) must ground an unwritten cell's fg exactly the
//! way `GridWindow::bg` already grounds its bg (SQ-0328) — see the fix in
//! `render::upper_window::window_ground`/`cell_style`.
//!
//! Reported symptom: after a host Save State restore, Kerkerkruip's panel
//! text read washed-out (theme default fg on the game's own bg) instead of
//! the game's own dark ink, on cells the game hadn't happened to reprint
//! since the window was rebuilt.
//!
//! # Why this drives a real cell but SYNTHESIZES the gap
//!
//! `kerkerkruip_status_grid_declares_both_bg_and_fg` drives a real,
//! freshly-booted Kerkerkruip session into gameplay and confirms its status
//! grid (a real Glk `TextGrid`, found by kind, ids drift — see
//! `sq1515_kerkerkruip_restore_arrange.rs`'s module doc) declares its own
//! `bg` AND `fg`, both real values off the shipped `Kerkerkruip.ini`.
//!
//! It does NOT also assert a naturally-unwritten (`Default`/`Default`) cell
//! sitting in that live grid — checked while writing this suite, there isn't
//! one: Kerkerkruip's status-bar routine prints its FULL row width every
//! turn, including trailing padding spaces, and the Glk backend bakes the
//! window's own colour into every printed cell explicitly (this grid's 103
//! blank cells all carried an explicit fg/bg equal to the window's own, not
//! `0`/`0`). That is a real, useful negative finding, not a shortcut: it
//! means the gap this fix closes is not reachable through this window in
//! *ordinary* play at a stable size. Where it IS reachable —
//! `GridWindow::resize`'s reallocation to all-`GridCell::default()` — is
//! exactly `sq1515_kerkerkruip_restore_arrange.rs`'s restore-into-a-fresh-
//! session territory (or a resize into a wider grid than what the game
//! happens to reprint), which this suite must NOT re-drive (SQ-1515's
//! restore delivery is out of scope for this fix and already covered there).
//!
//! So `real_grid_window_fg_grounds_a_cell_no_repaint_has_reached_yet` takes
//! the SAME real, live grid — genuine `bg`/`fg` off the real story, not
//! invented colours — and clears exactly one of its already-painted cells
//! back to `GridCell::default()` before rendering. That is precisely the
//! shape `GridWindow::resize` leaves for any cell a post-rebuild repaint
//! hasn't reached yet: a real window's declared colour, over a cell nobody
//! has printed a colour to. The render path under test
//! (`render::upper_window::draw_grid` → `cell_style`) cannot tell a
//! synthesized gap from an organic one — both are exactly
//! "`GridCell::fg == 0 && GridCell::bg == 0` inside a window with its own
//! declared colour" — so this exercises the real fix through its real
//! production entry point against a real specimen's real colours.
//!
//! Boot/settle plumbing mirrors `sq1515_kerkerkruip_restore_arrange.rs`
//! (same constants, same turn-budget reasoning: Kerkerkruip deals its whole
//! dungeon inside the first `glk_select`-to-`glk_select` span).

use std::path::{Path, PathBuf};
use std::time::Duration;

use app::colors::ColorScheme;
use app::engine::{Engine, GridCell, GridWindow, KeyInput, WinNode};
use app::glk_backend::GlkStylePairs;
use app::glulx_session::GlulxSession;
use app::render::paneframe::{BorderStyle, PaneSides};
use app::render::upper_window::draw_grid;
use app::session::InputKind;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

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
        (CELL_PX.0 as f64, CELL_PX.1 as f64),
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
    assert_eq!(Engine::pending_input(&sess), InputKind::Line, "reached the parser prompt");
    // The status grid is only painted on a full game turn, not merely on
    // opening (`sq1515_kerkerkruip_restore_arrange.rs`'s `assert_panels_populated`
    // doc comment) — take one real turn so this suite inspects a grid the
    // game has actually printed into.
    let _ = Engine::submit(&mut sess, "look");
    settle(&mut sess, |s| Engine::pending_input(s) != InputKind::Event);
    assert_eq!(Engine::pending_input(&sess), InputKind::Line, "survives a `look` turn");
    Some(sess)
}

/// Every Grid window anywhere in the tree, found by kind — not a hard-coded
/// id (Glk ids drift across a real session; see `sq1515`'s module doc for why
/// that suite's finder is content-based too).
fn grid_windows(sess: &GlulxSession) -> Vec<GridWindow> {
    fn walk(node: &WinNode, out: &mut Vec<GridWindow>) {
        match node {
            WinNode::Grid(g) => out.push(g.clone()),
            WinNode::Pair { first, second, .. } => {
                walk(first, out);
                walk(second, out);
            }
            _ => {}
        }
    }
    let mut v = Vec::new();
    walk(&Engine::screen(sess).root, &mut v);
    v
}

/// Non-vacuity precondition for the mechanism test below: a live Kerkerkruip
/// session must actually contain a Glk grid window that declared BOTH its own
/// `bg` and `fg` — the shape the fix's mechanism needs a specimen of. If a
/// future Kerkerkruip release (or a change to how lanthorn drives it) stops
/// declaring one or the other, this must fail loudly rather than let the case
/// below quietly stop meaning anything.
#[test]
fn kerkerkruip_status_grid_declares_both_bg_and_fg() {
    let Some(_p) = story_path() else { return };
    let sess = into_gameplay().expect("Kerkerkruip reaches the parser prompt");
    let grids = grid_windows(&sess);
    assert!(!grids.is_empty(), "expected at least one Glk grid window in Kerkerkruip's tree");
    let found = grids.iter().find(|g| g.bg.is_some() && g.fg.is_some());
    assert!(
        found.is_some(),
        "expected a Glk grid window declaring both bg and fg; found: {:?}",
        grids.iter().map(|g| (g.win, g.bg, g.fg)).collect::<Vec<_>>()
    );
    // And it must actually have printed content (a real, live grid, not one
    // that has never been touched at all).
    let g = found.unwrap();
    assert!(g.cells.iter().any(|c| c.ch != ' ' && c.ch != '\0'), "expected the grid to carry real printed text");
}

/// The mechanism check: take the real, live status grid's genuine `bg`/`fg`
/// (off the real Kerkerkruip.ini, via a real boot), clear one already-painted
/// cell back to `GridCell::default()` — the shape a `GridWindow::resize`
/// reallocation leaves for any cell a post-rebuild repaint hasn't reached
/// yet (see module doc) — and confirm the production `draw_grid` entry point
/// grounds that cell's fg on the WINDOW's own declared colour, not the theme
/// default.
#[test]
fn real_grid_window_fg_grounds_a_cell_no_repaint_has_reached_yet() {
    let Some(_p) = story_path() else { return };
    let sess = into_gameplay().expect("Kerkerkruip reaches the parser prompt");
    let grids = grid_windows(&sess);
    let mut g = grids
        .into_iter()
        .find(|g| g.bg.is_some() && g.fg.is_some())
        .expect("kerkerkruip_status_grid_declares_both_bg_and_fg should have caught a missing specimen first");

    let fg_rgb = g.fg.expect("filtered on fg.is_some() above");
    let expected_fg = Color::Rgb(((fg_rgb >> 16) & 0xFF) as u8, ((fg_rgb >> 8) & 0xFF) as u8, (fg_rgb & 0xFF) as u8);

    // Non-vacuity: the window's declared fg must actually differ from the
    // theme's own default grid fg, or grounding-vs-not would be unobservable.
    let mut colors = ColorScheme::default();
    let theme_default_fg = colors.theme.get("glk.grid.background").style.fg;
    assert_ne!(
        Some(expected_fg),
        theme_default_fg,
        "the window's declared fg coincides with the theme default by chance — cannot falsify the fix with this specimen"
    );

    // Pick a real cell the game DID paint (proof the window is genuinely
    // live), then blank it to `GridCell::default()` — an unwritten cell in
    // a window that still declares its own colour.
    let idx = g
        .cells
        .iter()
        .position(|c| c.ch != ' ' && c.ch != '\0')
        .expect("kerkerkruip_status_grid_declares_both_bg_and_fg confirmed real printed text exists");
    g.cells[idx] = GridCell::default();
    assert_eq!(g.cells[idx].fg, 0, "sanity: cleared cell carries no per-cell fg");
    assert_eq!(g.cells[idx].bg, 0, "sanity: cleared cell carries no per-cell bg");
    let (row, col) = ((idx / g.cols as usize) as u16 + 1, (idx % g.cols as usize) as u16 + 1);

    // Disable borders so the render area maps 1:1 onto the grid with no
    // centering/offset arithmetic to account for (mirrors
    // `draws_grid_cells_and_consumes_rows` in upper_window.rs's own tests).
    colors.virtual_window_border = BorderStyle::None;
    colors.upper_window_border_sides = PaneSides::all(BorderStyle::None);

    let area = Rect::new(0, 0, g.cols, g.rows);
    let mut buf = Buffer::empty(area);
    let consumed = draw_grid(&g, g.rows, (1, 1), false, &colors, area, &mut buf, true, &mut Vec::new());
    assert!(consumed > 0, "draw_grid consumed no rows");

    let cell = buf.cell((col - 1, row - 1)).unwrap_or_else(|| panic!("no buffer cell at ({col}, {row})"));
    assert_eq!(
        cell.fg, expected_fg,
        "unwritten grid cell ({row}, {col}) of window {} should ground on the window's own declared fg \
         (0x{fg_rgb:06x}), not the theme default",
        g.win
    );
}
