//! Text that never reached the v6 RASTER composite — SQ-0727, SQ-0728, SQ-0729.
//!
//! Three field reports, three games, and — measured — two mistakes:
//!
//! **"Opaque" is not "artwork."** Twice over. `build_chrome_canvas` decides whether
//! an inherited-colour reverse-video run should paint its highlight block by asking
//! whether anything opaque is behind it (SQ-0487): over Zork Zero's banner artwork a
//! block would erase the art, so the run draws dark ink straight onto it. It asked
//! the LIVE canvas, which by then also carried the blocks of every run drawn before
//! it. advent.z6's help screen paints its navigation bar as one run per label plus
//! reversed spacer spaces, and the spacer at x=289 lands inside "About Adventure"
//! (248..368) — so the label saw the spacer's own white block, called it artwork,
//! and drew itself in the page colour on the page. The whole bar was invisible in
//! raster while rendering correctly as cells. The same confusion, one layer up, ate
//! Shogun's prose: `story_clear_native` shrinks the story window until no edge
//! touches an opaque pixel, which is right for the frame art it exists to seat prose
//! inside and wrong for a menu the game deliberately printed INSIDE window 0.
//! Shogun's declared 548x64 box measured 548x16 — one row, which `build_main_text`
//! reports as ZERO visible rows — and Journey's 392x304 text panel measured 392x0.
//!
//! **A bounding box is not a picture.** `story_prose_box` yields the screen to a
//! window-0 plate that leaves no room for prose (Arthur's illustrated intro screens,
//! SQ-0707), and measured the plate by its bounding box. fmvpoker's poker table is a
//! 640x400 FRAME with a hollow middle — 17% of its pixels opaque — so its bbox is
//! the whole screen and the game's own backdrop was read as a plate that owns it.
//! Every line fmvpoker prints inside that frame was dropped.
//!
//! Two more findings fell out of the same measurements and are pinned here:
//! Shogun's menu is painted inside window 0's box, so the story page must go UNDER
//! it rather than flat over it; and the raster story text now top-anchors a
//! post-screen-clear screen exactly as the cell path does (`window_wrapped_rows`),
//! without which Shogun's four-row box redrew the tail of the banner the SQ-0697
//! freeze had just retired up top.
//!
//! Both `honor_game_colours` modes are pinned throughout: none of these games sets a
//! colour on the runs involved, so a mode-specific regression would otherwise hide.
//! Stories are gitignored (CLAUDE.md), so every case skips cleanly.

use std::collections::HashSet;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

use crate::fixture_paths::fixture_path;


fn boot(name: &str, honor: bool) -> Option<GameSession> {
    let path = fixture_path(name);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut s =
        GameSession::new_with_trace(bytes, honor, false, None, false, dims, picts.std_window(), None, None)
            .expect("a valid v6 story");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some(s)
}

fn raster_state(honor: bool) -> app::state::AppState {
    let mut st = app::state::AppState::default();
    st.colors = app::colors::ColorScheme::terminal_default();
    st.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    st.config.v6_render = app::config::V6RenderMode::Raster;
    st.config.honor_game_colours = honor;
    st
}

/// The v6 raster composite in native game pixels, plus its scroll metrics — the
/// render's own canvas step, so these are the shipped pixels.
fn composite(
    session: &GameSession,
    state: &app::state::AppState,
) -> (image::RgbaImage, Option<app::render::screen::RasterMetrics>) {
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    app::render::screen::build_v6_raster_canvas(&layout, native, state)
}

/// The distinct colours the composite carries inside `(x, y, w, h)` native pixels.
/// A run that reached the composite legibly shows TWO — its ink and the ground it
/// is read against. One means the region flattened to a single flat colour, which
/// is exactly what invisible text looks like.
fn colours_in(img: &image::RgbaImage, (x, y, w, h): (u32, u32, u32, u32)) -> HashSet<[u8; 4]> {
    let mut set = HashSet::new();
    for py in y..(y + h).min(img.height()) {
        for px in x..(x + w).min(img.width()) {
            set.insert(img.get_pixel(px, py).0);
        }
    }
    set
}

/// The share of `(x, y, w, h)` still painted in `page`. A reverse-video run that
/// reached the composite is mostly its HIGHLIGHT BLOCK with the glyph ink cut out
/// of it, so the page shows through only a minority of the span; a run whose block
/// was dropped leaves the page everywhere but its (page-coloured, invisible) ink.
fn page_share(img: &image::RgbaImage, page: [u8; 4], (x, y, w, h): (u32, u32, u32, u32)) -> f64 {
    let (mut hit, mut all) = (0usize, 0usize);
    for py in y..(y + h).min(img.height()) {
        for px in x..(x + w).min(img.width()) {
            all += 1;
            if img.get_pixel(px, py).0 == page {
                hit += 1;
            }
        }
    }
    hit as f64 / all.max(1) as f64
}

/// Pixels of `(x, y, w, h)` where the two composites differ.
fn differing(a: &image::RgbaImage, b: &image::RgbaImage, (x, y, w, h): (u32, u32, u32, u32)) -> usize {
    let mut n = 0;
    for py in y..(y + h).min(a.height()) {
        for px in x..(x + w).min(a.width()) {
            if a.get_pixel(px, py).0 != b.get_pixel(px, py).0 {
                n += 1;
            }
        }
    }
    n
}

/// The same render with NO transcript, so a diff against it isolates the story text
/// from everything else the composite draws.
fn without_transcript(state: &app::state::AppState) -> app::state::AppState {
    raster_state(state.config.honor_game_colours)
}

// ── SQ-0728: Shogun's title ──────────────────────────────────────────────────

/// Shogun's story window and its menu window, in native game pixels. The game
/// moves window 0 to a four-row box level with, and to the LEFT of, the
/// START/RESTORE/QUIT menu it prints into window 2, then prints "You may choose
/// to:" into window 0. Both belong on the screen at once.
const SHOGUN_STORY: (u32, u32, u32, u32) = (46, 336, 548, 64);
const SHOGUN_PROSE: (u32, u32, u32, u32) = (46, 336, 186, 64);
const SHOGUN_MENU_FIRST_ITEM: (u32, u32, u32, u32) = (234, 336, 120, 16);

fn shogun_title(honor: bool) -> Option<(GameSession, app::state::AppState)> {
    let mut s = boot("shogun-r322-s890706.z6", honor)?;
    let mut state = raster_state(honor);
    app::state::apply_transcript_elems(&mut state, &Engine::take_transcript_elems(&mut s));
    let r = match s.pending_input() {
        InputKind::Char => s.submit_char(13),
        _ => s.submit(""),
    };
    assert!(r.fault.is_none(), "Shogun faulted reaching its title: {:?}", r.fault);
    app::state::apply_transcript_elems(&mut state, &r.transcript_elems);
    Some((s, state))
}

fn shogun_title_shows_its_prose(honor: bool) {
    let Some((session, state)) = shogun_title(honor) else { return };
    assert!(
        state.transcript.iter().any(|l| l.contains("You may choose to")),
        "harness sanity (honor={honor}): the game printed its prompt into the new box"
    );
    // Premise: the box the game declared really is four rows beside the menu.
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let story = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT).story.expect("Shogun has a story window");
    assert_eq!(
        (story.x_px as u32, story.y_px as u32, story.w_px as u32, story.h_px as u32),
        SHOGUN_STORY,
        "premise (honor={honor}): window 0's declared box"
    );

    let (img, metrics) = composite(&session, &state);
    let m = metrics.expect("Shogun has a story window, so the raster reports scroll metrics");
    assert!(
        m.viewport_rows >= 3,
        "honor={honor}: the story box measures {} visible row(s); the menu painted inside window 0 \
         is eating it (SQ-0728) — the declared box is four rows",
        m.viewport_rows
    );

    // The prose reached the composite: the column left of the menu differs from the
    // same frame rendered with no transcript at all.
    let (blank, _) = composite(&session, &without_transcript(&state));
    let changed = differing(&img, &blank, SHOGUN_PROSE);
    assert!(
        changed > 50,
        "honor={honor}: only {changed} pixel(s) of window 0's text column carry story text — \
         Shogun's title shows no prose in raster (SQ-0728)"
    );

    // …and the menu the game painted inside that same box survived the story page
    // fill: its first item still reads as ink on a highlight block, not one flat
    // colour.
    let menu = colours_in(&img, SHOGUN_MENU_FIRST_ITEM);
    assert!(
        menu.len() >= 2,
        "honor={honor}: the menu inside window 0's box flattened to {} colour(s) — the story \
         page was painted OVER the chrome text the game printed into that box",
        menu.len()
    );
}

#[test]
fn shogun_title_shows_its_prose_honoring_game_colours() {
    shogun_title_shows_its_prose(true);
}

#[test]
fn shogun_title_shows_its_prose_theme_only() {
    shogun_title_shows_its_prose(false);
}

/// The four-row box shows the line the game printed into it, not the tail of the
/// banner the SQ-0697 freeze retired as paint up top. The cell path top-anchors a
/// post-screen-clear screen (`window_wrapped_rows`); the raster path now does too.
fn shogun_title_top_anchors_its_cleared_screen(honor: bool) {
    let Some((_session, state)) = shogun_title(honor) else { return };
    state.clear_anchor.expect("the freeze marks a screen clear");
    // The box the game declared: 548x64 native = 68 columns of four 8x16 cells.
    let (main, _) = app::render::screen::build_main_text(&state, 68, 4);
    assert_eq!(
        main.lines,
        vec!["You may choose to:".to_string()],
        "honor={honor}: the four-row box must show the line the game printed into it, not the \
         tail of the banner the SQ-0697 freeze already retired as paint up top"
    );
}

#[test]
fn shogun_title_top_anchors_its_cleared_screen_honoring_game_colours() {
    shogun_title_top_anchors_its_cleared_screen(true);
}

#[test]
fn shogun_title_top_anchors_its_cleared_screen_theme_only() {
    shogun_title_top_anchors_its_cleared_screen(false);
}

// ── SQ-0727: advent's help navigation bar ────────────────────────────────────

/// "About Adventure" and "N = next subject", in native game pixels. The game paints
/// them as reverse-video runs at these exact positions (`px_texts` at 1-based
/// (249,1) and (9,17)); a reversed spacer space at x=289 overlaps the first.
const ADVENT_TITLE_RUN: (u32, u32, u32, u32) = (248, 0, 120, 16);
const ADVENT_NEXT_RUN: (u32, u32, u32, u32) = (8, 16, 128, 16);

fn advent_help_bar_reaches_the_composite(honor: bool) {
    let Some(mut session) = boot("advent.z6", honor) else { return };
    let mut state = raster_state(honor);
    for _ in 0..10 {
        match session.pending_input() {
            InputKind::Line => break,
            InputKind::Char => {
                session.submit_char(13);
            }
            InputKind::Event => {
                session.submit("");
            }
        }
    }
    app::state::apply_transcript_elems(&mut state, &Engine::take_transcript_elems(&mut session));
    let r = session.submit("help");
    assert!(r.fault.is_none(), "advent faulted opening help: {:?}", r.fault);
    app::state::apply_transcript_elems(&mut state, &r.transcript_elems);

    // Premise: the bar really is painted as reverse-video runs at these positions.
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let runs: Vec<(u16, u16, String, u8)> = items
        .iter()
        .filter_map(|pw| match &pw.node {
            WinNode::Grid(g) => Some(g.px_texts.iter()),
            _ => None,
        })
        .flatten()
        .map(|t| (t.x, t.y, t.text.clone(), t.style))
        .collect();
    assert!(
        runs.contains(&(249, 1, "About Adventure".into(), 9)),
        "premise (honor={honor}): the help header is a reverse-video run at (249,1): {runs:?}"
    );
    assert!(
        runs.iter().any(|(x, y, t, s)| (*x, *y, s & 1) == (289, 1, 1) && t.trim().is_empty()),
        "premise (honor={honor}): a reversed spacer sits INSIDE that run's span"
    );

    let (img, _) = composite(&session, &state);
    // The page: advent's help screen leaves the bottom of window 0 untouched.
    let page = img.get_pixel(600, 380).0;
    for (label, rect) in [("About Adventure", ADVENT_TITLE_RUN), ("N = next subject", ADVENT_NEXT_RUN)] {
        let seen = colours_in(&img, rect);
        let share = page_share(&img, page, rect);
        assert!(
            seen.len() >= 2 && share < 0.5,
            "honor={honor}: \"{label}\" is {:.0}% bare page across {} colour(s) in the raster \
             composite — the row is missing entirely (SQ-0727). The reversed spacer run inside \
             its span made the over-art probe call it artwork, so it dropped its highlight block \
             and drew the ink in the page colour.",
            share * 100.0,
            seen.len()
        );
    }
}

#[test]
fn advent_help_bar_reaches_the_composite_honoring_game_colours() {
    advent_help_bar_reaches_the_composite(true);
}

#[test]
fn advent_help_bar_reaches_the_composite_theme_only() {
    advent_help_bar_reaches_the_composite(false);
}

// ── SQ-0729: fmvpoker's hollow frame ─────────────────────────────────────────

fn fmvpoker_text_reaches_the_composite(honor: bool) {
    let Some(mut session) = boot("fmvpoker.z6", honor) else { return };
    let mut state = raster_state(honor);
    app::state::apply_transcript_elems(&mut state, &Engine::take_transcript_elems(&mut session));
    let r = match session.pending_input() {
        InputKind::Char => session.submit_char(13),
        _ => session.submit(""),
    };
    assert!(r.fault.is_none(), "fmvpoker faulted: {:?}", r.fault);
    app::state::apply_transcript_elems(&mut state, &r.transcript_elems);
    assert!(
        state.transcript.iter().any(|l| l.contains("FROBOZZ MAGIC VIDEOPOKER")),
        "harness sanity (honor={honor}): the title prints its banner"
    );

    // Premise: window 0 carries the poker table as a backdrop, and it is a FRAME —
    // its bounding box is the whole screen, its painted pixels a small minority.
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let plate = layout.story_gfx.expect("fmvpoker draws a backdrop into window 0 (SQ-0714)");
    let WinNode::Graphics(g) = &plate.node else { panic!("story_gfx is a Graphics leaf") };
    let (pw, ph) = g.canvas.dimensions();
    let opaque = g.canvas.pixels().filter(|p| p.0[3] != 0).count();
    assert_eq!((pw, ph), (640, 400), "premise (honor={honor}): a full-screen backdrop");
    assert!(
        opaque * 2 < (pw * ph) as usize,
        "premise (honor={honor}): the table is a hollow frame — {opaque} of {} pixels painted",
        pw * ph
    );

    // The hollow frame leaves a real text box: `story_prose_box` measures the
    // largest rectangle of the clear interior the plate painted no pixel of, and a
    // frame's BOUNDING box (which is the whole screen) used to make it answer "the
    // plate owns the screen" — which is why not one line of text was drawn.
    let prose = app::render::v6_layout::story_prose_box((0, 0, 640, 400), layout.story_gfx, zvm::screen::V6Cell::DEFAULT)
        .expect("the hollow frame leaves a prose box");

    assert!(
        prose.2 >= 8 * 8 && prose.3 >= 16,
        "honor={honor}: the frame's interior measured {}x{} — a hollow frame's BOUNDING box is \
         the whole screen, and measuring it that way answered \"the plate owns the screen\", which \
         is why not one line of text was drawn (SQ-0729)",
        prose.2,
        prose.3
    );

    // …and the game's own text reaches the composite inside it. It arrives as
    // PAINT, not as transcript (SQ-0729 rule (d): this story window's art ENCLOSES
    // it, so it is a canvas and its runs are drawn where the game's own set_cursor
    // put them) — so the probe is for ink at those coordinates, which is where a
    // real interpreter shows them too. "Current Bet: / 10 / Total Winnings: / 1000"
    // are printed into window 0 at (76,247), (76,265), (420,247) and (420,265),
    // 1-based, all of them inside the frame.
    let (img, _) = composite(&session, &state);
    for (x, y, label) in
        [(76u32, 247u32, "Current Bet:"), (76, 265, "10"), (420, 247, "Total Winnings:"), (420, 265, "1000")]
    {
        let rect = (x - 1, y - 1, label.chars().count() as u32 * 8, 16);
        let seen = colours_in(&img, rect);
        assert!(
            seen.len() >= 2,
            "honor={honor}: {label:?} is one flat colour at the ({x},{y}) the game named — \
             fmvpoker's raster shows no text at all (SQ-0729)"
        );
    }
}

#[test]
fn fmvpoker_text_reaches_the_composite_honoring_game_colours() {
    fmvpoker_text_reaches_the_composite(true);
}

#[test]
fn fmvpoker_text_reaches_the_composite_theme_only() {
    fmvpoker_text_reaches_the_composite(false);
}

// ── Corpus guard: the same shrink was eating Journey's text panel ────────────

/// Journey draws a one-cell reversed divider on each of nineteen rows, and the
/// screen-wide pure-reverse gap fill (SQ-0504) that closes the bare cells of a bar
/// then paints across window 0's 392x304 text panel. Measured against the full
/// canvas the panel came back 392x0 and Journey's raster carried no prose at all;
/// measured against the ART it is the panel the game declared.
fn journey_text_panel_survives_the_menu_fill(honor: bool) {
    let Some(mut session) = boot("journey-r83-s890706.z6", honor) else { return };
    let mut state = raster_state(honor);
    let mut metrics = None;
    let mut reached = false;
    for _ in 0..4 {
        let r = match session.pending_input() {
            InputKind::Char => session.submit_char(13),
            _ => session.submit(""),
        };
        assert!(r.fault.is_none(), "Journey faulted: {:?}", r.fault);
        app::state::apply_transcript_elems(&mut state, &r.transcript_elems);
        let model = session.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
        let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
        let Some(story) = layout.story else { continue };
        if (story.x_px, story.y_px, story.w_px, story.h_px) != (240, 0, 392, 304) {
            continue;
        }
        reached = true;
        metrics = composite(&session, &state).1;
        break;
    }
    assert!(reached, "harness sanity: Journey reaches its 392x304 text panel within four frames");
    let rows = metrics.map(|m| m.viewport_rows).unwrap_or(0);
    assert!(
        rows >= 15,
        "honor={honor}: Journey's 392x304 text panel measures {rows} visible row(s) — the menu's \
         screen-wide gap fill is being counted as artwork and the panel shrinks to nothing, so \
         the raster carries no prose at all (SQ-0728)"
    );
}

#[test]
fn journey_text_panel_survives_the_menu_fill_honoring_game_colours() {
    journey_text_panel_survives_the_menu_fill(true);
}

#[test]
fn journey_text_panel_survives_the_menu_fill_theme_only() {
    journey_text_panel_survives_the_menu_fill(false);
}

// ── SQ-1056: a window's own erase is its PAGE, not an obstruction of itself ──

/// The game's painted ground (`erase_window` fills, SQ-0706) must not shrink the
/// story window's clear interior, because the probe that measures that interior
/// only ever reads pixels INSIDE that same window.
///
/// `stories/Shogun.toast` (Macintosh, release 292 / serial 890314) is the report,
/// on the frame after leaving InvisiClues with `q`: the game erases window 0 on the
/// way out, 548x370 at native (46, 30) — the story window to the pixel — and that
/// fill went into the obstruction the interior is measured against. Measured under
/// `examples/pty_capture` at the reporter's own 172x68 pane:
///
/// ```text
///   against the art          (46, 30) 548x370   the box the game declared
///   + the painted ground    (230, 215) 180x0    degenerate
/// ```
///
/// A degenerate interior trips SQ-0578's `w >= 8 && h >= 16` floor, which ships the
/// chrome and no prose — the score bar and both ornaments on screen with an empty
/// page between them, recoverable only by `restart`. Hybrid never saw it: it measures
/// against `build_graphics_canvas` alone (SQ-0896).
///
/// Driven here on a TRACKED fixture rather than the commercial press, because the
/// mechanism needs no particular game — any v6 story whose ground covers its own
/// story window reproduces it, and CI has no `stories/`.
fn a_ground_over_the_story_window_is_not_artwork(honor: bool) {
    let Some(mut session) = boot("advent.z6", honor) else { return };
    let mut state = raster_state(honor);
    for _ in 0..10 {
        match session.pending_input() {
            InputKind::Line => break,
            InputKind::Char => {
                session.submit_char(13);
            }
            InputKind::Event => {
                session.submit("");
            }
        }
    }
    app::state::apply_transcript_elems(&mut state, &Engine::take_transcript_elems(&mut session));

    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(
        items,
        &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT),
    );
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let story = layout.story.expect("advent publishes a story window");
    let (bx, by, bw, bh) =
        (story.x_px as u32, story.y_px as u32, story.w_px as u32, story.h_px as u32);
    assert!(
        bw >= 8 && bh >= 16,
        "harness sanity: advent's story window is {bw}x{bh} at ({bx},{by}) — too small to measure"
    );

    // Premise: with no ground at all the composite carries prose.
    let before = composite(&session, &state)
        .1
        .unwrap_or_else(|| panic!("honor={honor}: advent reports no story metrics even before the ground"));
    assert!(
        before.viewport_rows > 0,
        "harness sanity (honor={honor}): advent's clean frame measures 0 visible row(s)"
    );

    // …now the game erases its OWN story window, exactly as Shogun does leaving
    // InvisiClues. An opaque fill covering the box, and nothing else on the screen
    // changed.
    let mut ground = image::RgbaImage::new(u32::from(native.0), u32::from(native.1));
    for y in by..(by + bh).min(ground.height()) {
        for x in bx..(bx + bw).min(ground.width()) {
            ground.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
        }
    }
    *state.v6_paint.borrow_mut() = Some(std::sync::Arc::new(ground));

    let after = composite(&session, &state).1;
    let rows = after.map(|m| m.viewport_rows).unwrap_or(0);
    assert_eq!(
        rows, before.viewport_rows,
        "honor={honor}: the game's own erase of its {bw}x{bh} story window at ({bx},{by}) took the \
         story box from {} visible row(s) to {rows} — a window's ground is its PAGE, and the \
         clear-interior probe reads only pixels inside that window, so counting it obstructs the \
         box against itself and the raster ships chrome with no prose (SQ-1056)",
        before.viewport_rows
    );
}

#[test]
fn a_ground_over_the_story_window_is_not_artwork_honoring_game_colours() {
    a_ground_over_the_story_window_is_not_artwork(true);
}

#[test]
fn a_ground_over_the_story_window_is_not_artwork_theme_only() {
    a_ground_over_the_story_window_is_not_artwork(false);
}
