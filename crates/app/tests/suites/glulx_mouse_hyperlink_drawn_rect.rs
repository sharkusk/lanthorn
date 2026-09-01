//! SQ-1203: a Glk mouse/hyperlink click is hit-tested against the DRAWN rect,
//! not gvm's own layout rect.
//!
//! gvm's `WinTree` reserves a 1-cell border gutter per bordered split
//! (`split_border`, `crates/gvm/src/glk.rs`) whether or not anything is drawn
//! there. The renderer only reserves that gutter when the THEME actually draws
//! a rule (`upper_window_border`, SQ-0821) — and the shipped default is
//! `BorderStyle::None`, so a real Glulx layout skews from gvm's rect by one cell
//! per collapsed gutter between the pane origin and the window. City of
//! Secrets' `help` menu is the reproduction that surfaced it: a click on the
//! menu's own drawn top row/left column landed outside every gvm-reported
//! rect and was silently dropped.
//!
//! Two layers, per the project's escalate-only-when-needed convention: a
//! CoS-free geometry case pins the mechanism (a bordered split with no rule
//! drawn ⇒ the recorded `win_rects` entry is the DRAWN rect, not a
//! gutter-inflated one), and a real-game case drives the whole chain — boot,
//! render, locate the drawn text, hit-test through the exact functions
//! `main.rs` calls, deliver — and asserts the game's own answer (the menu's `>`
//! selection marker moves to the clicked row).

use app::engine::{BorderPref, Engine, GridCell, GridWindow, KeyInput, ScreenModel, Split, StatusModel, WinKind, WinNode};
use app::glulx_session::GlulxSession;
use app::state::AppState;
use blorb::Blorb;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::fixture_paths::fixture_path;

// ── Geometry unit test: the collapsed-gutter case, without a real game ────────

/// A bordered `Pair` split (the game's `winmethod_Border` hint) under the
/// shipped default theme, which draws no rule for it (`upper_window_border` is
/// `BorderStyle::None`, SQ-0821). The renderer must draw the second child
/// FLUSH against the first — no gap — and `win_rects` must record exactly that,
/// not the 1-cell-inflated origin gvm's own layout would report.
#[test]
fn collapsed_gutter_split_records_the_flush_drawn_rect() {
    fn grid(win: u32, cols: u16, rows: u16) -> GridWindow {
        GridWindow {
            win,
            fill: None,
            cols,
            rows,
            cells: vec![GridCell::default(); cols as usize * rows as usize],
            active_rows: rows,
            cursor: (1, 1),
            cursor_active: false,
            border: BorderPref::Unspecified,
            bg: None,
            fg: None,
            reverse: false,
            px_texts: Vec::new(),
        }
    }

    let model = ScreenModel {
        root: WinNode::Pair {
            vertical: true,
            split: Split { fixed: 3 },
            // The game's own border request — gvm reserves a gutter cell for
            // this in its layout regardless of what the theme does with it.
            border: true,
            key_bg: None,
            key_fg: None,
            first: Box::new(WinNode::Grid(grid(7, 40, 3))),
            second: Box::new(WinNode::Grid(grid(8, 40, 7))),
        },
        status: StatusModel::HostManaged,
        bg: 0,
        fg: 0,
        // Nonzero content_size is the Glulx marker (`is_simple` routes a
        // zero-size model through the byte-identical Z-machine path, which
        // records no `win_rects` at all — this test wants the generic path).
        content_size: (40, 10),
    };

    let area = Rect::new(0, 0, 40, 10);
    let mut buf = Buffer::empty(area);
    let state = AppState::default();
    // Sanity: the shipped default really does draw no separator rule (SQ-0821),
    // or this test would not be exercising the collapsed-gutter case at all.
    assert!(
        !state.colors.upper_window_border_sides.any_on(),
        "this case pins the no-rule default; a themed default would need a different fixture"
    );

    let m = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    assert_eq!(
        m.win_rects.iter().find(|&&(id, _, _)| id == 7).map(|&(_, k, r)| (k, r)),
        Some((WinKind::Grid, Rect::new(0, 0, 40, 3))),
        "the first (fixed) child is drawn at its full budget, win_rects: {:?}",
        m.win_rects
    );
    assert_eq!(
        m.win_rects.iter().find(|&&(id, _, _)| id == 8).map(|&(_, k, r)| (k, r)),
        // FLUSH against the first child — row 3, not row 4. gvm's own layout
        // would have reserved a gutter row here and reported this window's
        // origin one row later than what was actually drawn.
        Some((WinKind::Grid, Rect::new(0, 3, 40, 7))),
        "the second child is drawn FLUSH — no gutter the theme never drew, win_rects: {:?}",
        m.win_rects
    );
}

// ── Real-game falsifying test: City of Secrets' `help` menu ───────────────────

/// Boot City of Secrets to its `help` menu (3 blank keypresses past the title,
/// then `help`), or `None` when the gitignored fixture is absent.
fn boot_cos_help_menu() -> Option<GlulxSession> {
    let path = fixture_path("CoS.blb");
    let raw = std::fs::read(&path).ok()?;
    let blorb1 = Blorb::parse(raw.clone()).ok()?;
    let (_, image) = blorb1.executable().ok()?;
    let image = image.to_vec();
    let blorb2 = Blorb::parse(raw).ok()?;
    let mut sess = GlulxSession::new(image, 80, 30, true, true, false, (8, 16), Some(blorb2), &[])
        .expect("CoS should load and boot");
    let _ = Engine::take_transcript(&mut sess);
    for _ in 0..3 {
        Engine::submit_key(&mut sess, KeyInput::Char(' '));
    }
    let _ = Engine::submit(&mut sess, "help");
    Some(sess)
}

/// Render the current screen at 80x30 and return `(buffer, win_rects)`.
fn render(sess: &GlulxSession, state: &AppState) -> (Buffer, Vec<(u32, WinKind, Rect)>) {
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

/// SQ-1203's own reproduction: a click on the menu's drawn "Tutorial" row must
/// select THAT row — driven through the exact functions `main.rs` calls
/// (`mouse_windows` + `glk_mouse_target` + `deliver_mouse`), against the
/// `win_rects` this same render recorded.
///
/// Falsified as instructed: reverting the `glk_mouse_target`/`win_rects` fix
/// (hit-testing gvm's own layout rect instead) reproduces exactly the reported
/// symptom — the click resolves one row/column short of the drawn text and the
/// menu's selection never moves.
#[test]
fn click_on_drawn_menu_row_selects_it() {
    let Some(mut sess) = boot_cos_help_menu() else {
        eprintln!("SKIP: no CoS.blb");
        return;
    };
    let state = AppState::default();
    let (buf, win_rects) = render(&sess, &state);
    let area_width = 80u16;
    let area_height = 30u16;

    // Locate the drawn "Tutorial" menu item — the row and its own starting column.
    let row = (0..area_height)
        .find(|&y| row_text(&buf, y, area_width).contains("Tutorial"))
        .unwrap_or_else(|| panic!("menu should show a Tutorial item:\n{}", dump(&buf, area_width, area_height)));
    let col = (0..area_width)
        .find(|&x| {
            let s: String = (x..(x + 8).min(area_width))
                .map(|xx| buf.cell((xx, row)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect();
            s.starts_with("Tutorial")
        })
        .expect("Tutorial's own drawn column");

    // Not yet selected.
    assert!(
        !row_text(&buf, row, area_width).trim_start().starts_with('>'),
        "Tutorial is not the menu's initial selection"
    );

    // The SAME path main.rs's mouse handler takes: ids currently watching,
    // hit-tested against the DRAWN win_rects this render recorded.
    let watching = sess.mouse_windows();
    assert!(!watching.is_empty(), "the menu grid is watching for clicks");
    let story = (0u16, 0u16, area_width, area_height);
    let target = app::glulx_session::glk_mouse_target(
        false,
        col,
        row,
        story,
        &watching,
        &win_rects,
        sess.char_pixels(),
        None,
    );
    let (win, vx, vy) = target.unwrap_or_else(|| {
        panic!(
            "the drawn Tutorial cell ({col},{row}) should resolve to the menu window; \
             watching={watching:?} win_rects={win_rects:?}"
        )
    });
    let _ = sess.deliver_mouse(win, vx, vy);

    // Re-render and confirm the click landed on the row it was aimed at.
    let (buf2, _) = render(&sess, &state);
    let selected = row_text(&buf2, row, area_width);
    assert!(
        selected.trim_start().starts_with('>') && selected.contains("Tutorial"),
        "the click on the drawn Tutorial row should select it, got row {row}: {selected:?}\n{}",
        dump(&buf2, area_width, area_height)
    );
}

fn dump(buf: &Buffer, w: u16, h: u16) -> String {
    (0..h).map(|y| row_text(buf, y, w) + "\n").collect()
}
