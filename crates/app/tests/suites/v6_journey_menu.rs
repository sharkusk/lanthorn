//! Journey (Infocom v6) bottom command menu — SQ-0492.
//!
//! Journey drives all gameplay through a bottom command-menu window (win1:
//! "Proceed/Back/Game", the party names, verb columns), painted as absolute
//! pixel-positioned text runs at native rows 19–24. That window is FULL-WIDTH
//! but sized to HEIGHT 0 in the window property table — its menu is pure v6
//! PAINT (persists at its screen-absolute pixels regardless of the zero size).
//!
//! The pre-fix `v6_screen_model` skipped every window with `x_size==0 ||
//! y_size==0`, so the whole menu was dropped before it ever reached the
//! ScreenModel — no render mode showed it. The fix keeps a zero-size window
//! that still holds painted runs, and grows the native raster extent to cover
//! those runs so the menu rasterizes into the bottom band.
//!
//! Skip-if-missing pattern per the other gitignored-story smokes.
//!
//! **Colour mode: `honor_game_colours = true`** — the app's shipped config
//! default, so these render assertions are made in the mode real players run.
//! (Before SQ-0532 wave 4 every v6 smoke booted with the game's colours
//! DECLINED, which is exactly why three colour-driven render regressions
//! shipped unseen. The theme-only `false` path is covered by the paired cases
//! in `v6_game_colour_regression.rs`.)

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// Does the frame's side rule reach this cell?
///
/// SQ-0750: hybrid no longer uploads a bitmap of a border the game printed as a
/// CHARACTER — it stamps the character. So the divider's ink is a reverse-video space
/// (a solid block carried by the REVERSED modifier over the theme's own colours) or a
/// box-drawing glyph, and a cell-BACKGROUND luminance probe, which is what every case
/// below used, stopped seeing a divider that is drawn perfectly well. Ask about all
/// three media instead: a light background, a reversed cell, or a glyph.
fn is_divider(buf: &Buffer, x: u16, y: u16) -> bool {
    use ratatui::style::{Color, Modifier};
    let Some(c) = buf.cell((x, y)) else { return false };
    let lum = match c.bg {
        Color::Rgb(r, g, b) => r as u16 + g as u16 + b as u16,
        _ => 0,
    };
    // A frame rule is a box-drawing or block glyph (U+2500..U+259F) — the same class
    // `collapse_row_rules` treats as a divider. Accepting ANY non-blank symbol would
    // let a stray character in the flank column stand in for a rule that is missing.
    let frame_glyph = c.symbol().chars().next().is_some_and(|ch| ('\u{2500}'..='\u{259F}').contains(&ch));
    lum > 400 || c.style().add_modifier.contains(Modifier::REVERSED) || frame_glyph
}

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot Journey and drive the intro vignettes until it sits in `read_char` on
/// the Praxix command-menu page (the menu painted, awaiting a keypress).
fn journey_at_menu() -> Option<GameSession> {
    let story_path = stories_dir().join("journey-r83-s890706.z6");
    let story_bytes = match std::fs::read(&story_path) {
        Ok(b) => b,
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", story_path.display());
            return None;
        }
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Journey (v6) should load and boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();

    // Tap Enter through the intro vignettes until the command menu is up.
    for _ in 0..40 {
        let r = match session.pending_input() {
            InputKind::Line => session.submit(""),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        if r.transcript.contains("Praxix") || r.transcript.contains("magical resources") {
            break;
        }
    }
    Some(session)
}

/// All non-blank grid px-text runs in the model, with their native row.
fn deep_runs(model: &app::engine::ScreenModel) -> Vec<(u16, String)> {
    let WinNode::Layered(items) = &model.root else { return Vec::new() };
    let mut out = Vec::new();
    for pw in items {
        if let WinNode::Grid(g) = &pw.node {
            for t in &g.px_texts {
                if !t.text.trim().is_empty() {
                    out.push(((t.y.max(1) - 1) / 16, t.text.clone()));
                }
            }
        }
    }
    out
}

/// (a) The command menu reaches the ScreenModel as positioned grid runs at
/// native rows ≥ 19 — including the "Proceed" verb. Pre-fix this was empty.
#[test]
fn journey_menu_reaches_the_model_as_deep_grid_runs() {
    let Some(session) = journey_at_menu() else { return };
    assert_eq!(session.pending_input(), InputKind::Char, "menu sits in read_char");

    let model = session.screen();
    let runs = deep_runs(&model);
    let deep: Vec<&(u16, String)> = runs.iter().filter(|(row, _)| *row >= 19).collect();
    assert!(
        !deep.is_empty(),
        "the command menu must reach the model as deep grid runs (row ≥ 19); got {runs:?}"
    );
    assert!(
        deep.iter().any(|(_, t)| t.contains("Proceed")),
        "the 'Proceed' menu verb is a deep run; got {deep:?}"
    );
    // The party column ("Bergon"/"Praxix") and the "Game" verb are there too.
    assert!(deep.iter().any(|(_, t)| t.contains("Praxix")), "party name present: {deep:?}");
    assert!(deep.iter().any(|(_, t)| t.contains("Game")), "'Game' verb present: {deep:?}");
}

/// (b, Raster) The menu rasterizes into the bottom band of the native chrome
/// canvas — the exact canvas Raster mode uploads. Pre-fix the canvas was too
/// short (no window reached below native y≈304) so rows 19–24 were blank.
#[test]
fn journey_menu_rasterizes_into_the_bottom_band() {
    use app::render::v6_layout as v6;
    let Some(session) = journey_at_menu() else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };

    let native = v6::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    assert!(
        native.1 >= 385,
        "native extent must cover the menu runs (bottom at native y≈385); got {native:?}"
    );

    let layout = v6::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let colors = app::colors::ColorScheme::terminal_default();
    let canvas = v6::build_chrome_canvas(
        &layout.chrome,
        native,
        image::Rgba([220, 220, 220, 255]),
        image::Rgba([0, 0, 0, 255]),
        &colors,
        v6::TextLayer::All,
        &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT),
    );
    // Count opaque ink in the bottom band (native rows 19–24 → y 304..).
    let y0 = 19 * 16u32;
    let inked = (y0..canvas.height())
        .flat_map(|y| (0..canvas.width()).map(move |x| (x, y)))
        .filter(|&(x, y)| canvas.get_pixel(x, y)[3] >= 128)
        .count();
    assert!(
        inked > 0,
        "the menu must rasterize opaque ink into the bottom band (native rows ≥ 19); got {inked} inked pixels"
    );
}

/// (b, Hybrid) In HYBRID mode the menu is BELOW the story window (rows 0–18),
/// so the screen takes the pixel-chrome RING path (not the menu-screen text
/// gate, SQ-0494) and rasterizes the menu into the bottom terminal band. The
/// bottom rows must carry rendered ink (colored halfblock cells) — pre-fix they
/// were empty.
#[test]
fn journey_hybrid_ring_shows_the_menu_band() {
    let Some(session) = journey_at_menu() else { return };
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    let area = Rect::new(0, 0, 80, 25);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    // A cell has ink when its symbol is not a blank space or its style set a
    // concrete (non-Reset) colour — the halfblocks encoder writes '▀' cells with
    // fg/bg colours for the rasterized menu.
    let has_ink = |x: u16, y: u16| -> bool {
        let Some(c) = buf.cell((x, y)) else { return false };
        let sym = c.symbol();
        let blank = sym == " " || sym.is_empty();
        let default_style = c.fg == ratatui::style::Color::Reset && c.bg == ratatui::style::Color::Reset;
        !blank || !default_style
    };
    let band_ink: usize = (19..25)
        .flat_map(|y| (0..area.width).map(move |x| (x, y)))
        .filter(|&(x, y)| has_ink(x, y))
        .count();
    assert!(
        band_ink > 0,
        "the hybrid ring must rasterize the menu into the bottom band (rows 19–24); got {band_ink} inked cells"
    );
}

/// (d, SQ-0500) In HYBRID mode the bottom command menu is a PURE-TEXT chrome band
/// (no artwork behind it), so it paints as real terminal CELLS — "Proceed" is
/// findable as buffer text in the bottom rows — while the LEFT picture column
/// (win3 graphics) stays in the pixel ring (image half-block cells, not text).
#[test]
fn journey_hybrid_menu_is_terminal_cells_picture_stays_ring() {
    let Some(session) = journey_at_menu() else { return };
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    let area = Rect::new(0, 0, 80, 25);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    // "Proceed" is a real menu verb painted as terminal text in the bottom band.
    let menu_text: String = (18..area.height).map(|y| row_text(y) + "\n").collect();
    assert!(
        menu_text.contains("Proceed"),
        "the command menu renders as terminal CELLS ('Proceed' as buffer text); got:\n{menu_text}"
    );
    assert!(menu_text.contains("Praxix"), "party column as cells too:\n{menu_text}");

    // The left picture column stays a pixel-ring image: its cells are half-block
    // graphics (▀/▄) or coloured, NOT terminal text. Scan the upper-left region.
    let is_image_cell = |x: u16, y: u16| -> bool {
        let c = buf.cell((x, y)).unwrap();
        let sym = c.symbol();
        sym == "\u{2580}" || sym == "\u{2584}" || c.bg != ratatui::style::Color::Reset
    };
    let pic_ink = (1..17)
        .flat_map(|y| (0..24).map(move |x| (x, y)))
        .filter(|&(x, y)| is_image_cell(x, y))
        .count();
    assert!(pic_ink > 0, "the left picture column stays imaged in the pixel ring (got {pic_ink} image cells)");
}

/// (e, SQ-0504 raster) In the pixel canvas Raster mode uploads, the menu header
/// ("The Party" | "Individual Commands", both reverse-video) reads as ONE solid
/// bar spanning the ENTIRE screen width — the reverse block fills from the left
/// edge, between the two labels, and out to the right edge, not just the span
/// between them. The menu BODY row (reversed column dividers among NON-reversed
/// verb text) is left alone: the cells between its dividers stay bare, not a bar.
#[test]
fn journey_raster_reverse_header_bar_is_solid_body_untouched() {
    use app::render::v6_layout as v6;
    let Some(session) = journey_at_menu() else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = v6::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = v6::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let colors = app::colors::ColorScheme::terminal_default();
    let canvas = v6::build_chrome_canvas(
        &layout.chrome, native, image::Rgba([220, 220, 220, 255]), image::Rgba([0, 0, 0, 255]), &colors,
        v6::TextLayer::All,
        &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT),
    );
    let ncols = (canvas.width() / 8) as u32;
    // A whole 8-px cell is "filled" when every pixel column has opaque ink.
    let cell_filled = |cx: u32, row: u32| -> bool {
        let (x0, y0) = (cx * 8, row * 16);
        (x0..(x0 + 8).min(canvas.width())).all(|x| {
            (y0..(y0 + 16).min(canvas.height())).any(|y| canvas.get_pixel(x, y)[3] >= 128)
        })
    };
    // Header row 19 is a pure reverse bar: EVERY cell from the left edge to the
    // right edge is a filled block — the leading gap (cols 0.., before "The
    // Party"), the gap between the labels, and the trailing gap (past "Individual
    // Commands") all fill, so the whole native row is solid.
    let header_full_width = (0..ncols).all(|cx| cell_filled(cx, 19));
    assert!(header_full_width, "row-19 header fills into ONE solid reverse bar across the full screen width");
    // Body row 20 has reversed dividers at cols ~15/31/47/63 with normal verb text
    // between — the cells mid-column (e.g. 35..46, between two dividers, no text)
    // must NOT be block-filled (that row is mixed, not a pure reverse bar).
    let body_between_dividers_bare = (35..46).any(|cx| !cell_filled(cx, 20));
    assert!(body_between_dividers_bare, "row-20 menu body stays normal (not over-filled) between its dividers");
}

/// (c) The menu is live through the app layer: a keypress (arrow) changes the
/// painted runs and the game stays in `read_char` (still on the menu).
#[test]
fn journey_menu_is_live_through_the_app_layer() {
    let Some(mut session) = journey_at_menu() else { return };
    assert_eq!(session.pending_input(), InputKind::Char);

    let sig = |s: &GameSession| -> Vec<(u16, u16, u8, String)> {
        let mut v: Vec<_> = s.machine.screen.v6.as_ref().unwrap().windows[1]
            .texts
            .iter()
            .map(|t| (t.y, t.x, t.style, t.text.clone()))
            .collect();
        v.sort();
        v
    };
    let before = sig(&session);
    // ZSCII 130 = down-arrow (v6 read_char terminating key).
    let r = session.submit_char(130);
    assert!(!r.quit, "an arrow keypress must not quit the game");
    assert_eq!(session.pending_input(), InputKind::Char, "still on the menu after the arrow");
    assert_ne!(before, sig(&session), "the arrow moved the selection — the painted runs changed");
}

/// (f, SQ-0504 hybrid) The menu header reverse bar spans the ENTIRE pane width in
/// the hybrid cell path: on the terminal row carrying "The Party", every cell from
/// the left edge to the right edge is reverse-video — the bar reads edge to edge,
/// not just between the two labels.
#[test]
fn journey_hybrid_header_bar_spans_full_pane_width() {
    use ratatui::style::Modifier;
    let Some(session) = journey_at_menu() else { return };
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    let area = Rect::new(0, 0, 80, 25);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    let header_y = (0..area.height).find(|&y| row_text(y).contains("The Party")).expect("header row present");
    let holes: Vec<u16> = (0..area.width)
        .filter(|&x| !buf.cell((x, header_y)).unwrap().modifier.contains(Modifier::REVERSED))
        .collect();
    assert!(
        holes.is_empty(),
        "the header reverse bar spans the full pane [0,{}) with no gaps; non-reversed cols {holes:?}\nrow: {:?}",
        area.width,
        row_text(header_y)
    );
}

/// (h, SQ-0508 + SQ-0543) At a pane size whose letterbox scale USED to spread the
/// menu's native rows across MORE terminal rows (100×30 → scale 1.2, which mapped
/// six native rows onto seven terminal rows with a hole in the middle), the whole
/// menu panel reads as ONE solid black block, its body rows land on CONSECUTIVE
/// terminal rows, and the reversed vertical column dividers run continuously down
/// the panel — no broken lines, no theme backdrop through the gaps.
///
/// SQ-0543 removed the spreading itself: a chrome TEXT strip has no art behind it
/// to stay aligned with, so it lays its runs out by the game's own row structure
/// instead of by scaled device pixels.
#[test]
fn journey_hybrid_menu_full_black_panel_dividers_continuous() {
    use ratatui::style::{Color, Modifier};
    let Some(session) = journey_at_menu() else { return };
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    // 100×30 → pane_dev 800×480, native 640×400, scale = min(1.25, 1.2) = 1.2:
    // the menu's six native rows spread to seven terminal rows (one is a gap).
    let area = Rect::new(0, 0, 100, 30);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    let reversed = |x: u16, y: u16| buf.cell((x, y)).unwrap().modifier.contains(Modifier::REVERSED);

    let header_y = (0..area.height).find(|&y| row_text(y).contains("The Party")).expect("header row");
    let game_y = (header_y..area.height).find(|&y| row_text(y).contains("Game")).expect("last menu row (Game)");
    assert!(game_y > header_y + 4, "the menu spans several rows (header {header_y}, Game {game_y})");

    // (a) The ENTIRE menu panel is a black background — every cell in the strip
    // (reversed or not, the stored bg stays black under the REVERSED modifier) is
    // Color::Black, so no theme backdrop shows through the gaps between the runs.
    //
    // "The entire panel" is the game's own SCREEN, not the pane (SQ-0946). At this
    // size the halfblock cell is 1x2, so the pane is 100x60 device pixels against a
    // 640x400 native screen and the fit is HEIGHT-bound: `s = 0.15`, the screen is 96
    // device columns wide, and a two-column letterbox margin opens either side of it.
    // The menu's ground stops there, because a strip that floods the margin puts the
    // game's frame off centre by however wide the margin is — which is the whole of
    // SQ-0946, reported on this very game. The gaps this case is about are the ones
    // BETWEEN the runs, and they are all inside the screen.
    let (lo, hi) = {
        use app::render::v6_layout as v6;
        let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
        let native = v6::native_extent(items.as_slice(), &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        let (scale, _) = v6::FrameGeometry::new(native, (2, 2), zvm::screen::V6Cell::DEFAULT)
            .fitted_scale((area.width as u32, area.height as u32 * 2), false);
        v6::screen_cols(&scale, native.0, (1, 2), area)
    };
    assert_eq!((lo, hi), (2, 98), "the height-bound fit leaves two columns of letterbox either side");
    for y in header_y..=game_y {
        for x in lo..hi {
            assert_eq!(
                buf.cell((x, y)).unwrap().bg,
                Color::Black,
                "menu cell ({x},{y}) must be on the black panel bg; row: {:?}",
                row_text(y)
            );
        }
    }

    // (b) Divider continuity. Collect the reversed column dividers from a body row
    // (the "Proceed" row), then assert EVERY menu body row — including the blank
    // scale-introduced gap row — is reversed at each of those columns.
    let proceed_y = (header_y..=game_y).find(|&y| row_text(y).contains("Proceed")).expect("Proceed row");
    let dividers: Vec<u16> = (0..area.width)
        .filter(|&x| reversed(x, proceed_y) && row_text(proceed_y).chars().nth(x as usize) == Some(' '))
        .collect();
    assert!(dividers.len() >= 4, "the menu body has ≥4 reversed column dividers; got {dividers:?}");
    // SQ-0543: NO scale-introduced blank row may split the menu's body. The game
    // drew these rows adjacent, and a text strip now lays out by the game's own row
    // structure, so they arrive adjacent at every pane size. (The same spreading is
    // what put a hole through the middle of Shogun's two-line status band.)
    let gap_y = ((header_y + 1)..game_y).find(|&y| row_text(y).trim().is_empty());
    assert!(
        gap_y.is_none(),
        "menu body rows must be consecutive; found a blank row at {gap_y:?}\n{}",
        (header_y..=game_y).map(|y| format!("  {y}: {:?}\n", row_text(y))).collect::<String>()
    );
    for y in (header_y + 1)..=game_y {
        for &x in &dividers {
            assert!(
                reversed(x, y),
                "column divider at col {x} must stay reversed on row {y} (continuous vertical line); row: {:?}",
                row_text(y)
            );
        }
    }
}

/// (i, SQ-0509 regression guard) The menu items are SEPARATE runs with real pixel
/// gaps + reversed dividers between them — the SQ-0509 fragment-merging must NOT
/// blob them into one word. Even at scale 1.2 the body row still reads its verbs as
/// distinct, space-separated tokens ("Praxix", "Cast", "Examine"), and the dividers
/// keep them apart.
#[test]
fn journey_hybrid_menu_items_not_merged() {
    let Some(session) = journey_at_menu() else { return };
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    let area = Rect::new(0, 0, 100, 30);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    let body: String = (0..area.height).map(|y| row_text(y) + "\n").collect();
    // Each verb is present as its own token, and adjacent items never fuse.
    for verb in ["Proceed", "Bergon", "Praxix", "Cast", "Examine", "Inventory"] {
        assert!(body.contains(verb), "menu verb {verb:?} present as its own run; got:\n{body}");
    }
    assert!(!body.contains("PraxixCast") && !body.contains("CastExamine"), "adjacent menu items must NOT merge:\n{body}");
    // The Praxix row keeps its items spaced apart (a divider/space between them).
    let prow = (0..area.height).map(row_text).find(|r| r.contains("Praxix")).expect("Praxix row");
    let pi = prow.find("Praxix").unwrap();
    let ci = prow.find("Cast").unwrap();
    assert!(ci > pi + "Praxix".len() + 1, "Praxix and Cast stay separated by a gap; row: {prow:?}");
}

/// (g, SQ-0504) The pixel-band canvas EXCLUDES the pure-text menu rows: after the
/// text-strip rows are carved out (exactly what the hybrid path feeds its bands),
/// the menu's native rows carry NO ink — so the rasterized menu can never appear
/// behind the terminal cells — while Journey's authentic vertical picture/text
/// divider (reversed runs at native x≈233, beside the story box, NOT a text strip)
/// stays inked and thus keeps rendering in the left picture-column ring.
#[test]
fn journey_pixel_band_canvas_excludes_menu_keeps_divider() {
    use app::render::v6_layout as v6;
    let Some(session) = journey_at_menu() else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = v6::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = v6::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let story = layout.story.expect("story window");
    let story_bottom = (story.y_px + story.h_px) as u32;
    let colors = app::colors::ColorScheme::terminal_default();
    let mut canvas = v6::build_chrome_canvas(
        &layout.chrome, native, image::Rgba([220, 220, 220, 255]), image::Rgba([0, 0, 0, 255]), &colors,
        v6::TextLayer::All,
        &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT),
    );

    // The menu runs sit BELOW the story box; collect their native tops.
    let menu_tops: Vec<u16> = layout
        .chrome
        .iter()
        .filter_map(|it| match &it.node {
            WinNode::Grid(g) => Some(g.px_texts.iter()),
            _ => None,
        })
        .flatten()
        .filter(|t| (t.y.max(1) - 1) as u32 >= story_bottom)
        .map(|t| t.y.max(1) - 1)
        .collect();
    assert!(!menu_tops.is_empty(), "menu runs below the story box present");

    let row_has_ink = |c: &image::RgbaImage, y: u32| (0..c.width()).any(|x| c.get_pixel(x, y)[3] >= 128);
    // Before carving: the menu rows are inked (rasterized text).
    let a_menu_row = menu_tops.iter().map(|&t| t as u32 + 4).min().unwrap();
    assert!(row_has_ink(&canvas, a_menu_row), "menu row {a_menu_row} is rasterized before carving");
    // The divider column (native x≈233) has ink across the story rows.
    let divider_ink_before = (0..story_bottom).filter(|&y| canvas.get_pixel(233, y)[3] >= 128).count();
    assert!(divider_ink_before > 200, "the vertical divider is inked before carving ({divider_ink_before} rows)");

    // Carve the menu rows out — exactly what screen.rs feeds the pixel bands.
    // The menu band spans the pane, so its strip's native columns are the whole
    // screen — the span `screen.rs` passes for any full-width strip (SQ-0894).
    let carve: Vec<(u16, u32, u32)> = menu_tops.iter().map(|&t| (t, 0, u32::MAX)).collect();
    v6::clear_text_rows(&mut canvas, &carve, zvm::screen::V6Cell::DEFAULT);

    // Every carved menu row is now fully transparent.
    for &top in &menu_tops {
        for y in top as u32..(top as u32 + 16).min(canvas.height()) {
            assert!(!row_has_ink(&canvas, y), "menu native row {y} must be carved out of the band canvas");
        }
    }
    // The divider is UNTOUCHED — it is beside the story, not a text strip.
    let divider_ink_after = (0..story_bottom).filter(|&y| canvas.get_pixel(233, y)[3] >= 128).count();
    assert_eq!(divider_ink_before, divider_ink_after, "the picture/text divider survives the carve (stays in the ring)");
}

/// (SQ-0505 dynamic hybrid layout) At a TALL pane (90×40 — vertical letterbox
/// slack below the native 8:5 frame) Journey has BOTTOM TEXT CHROME: the command
/// menu. The menu strip anchors to the pane BOTTOM edge and the story viewport
/// fills the space between the top-anchored chrome and the menu at constant width.
#[test]
fn journey_hybrid_tall_pane_menu_pinned_to_bottom() {
    let Some(session) = journey_at_menu() else { return };
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    let area = Rect::new(0, 0, 90, 40);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    // The command menu renders as terminal CELLS pinned at the pane bottom: the
    // "Proceed" verb sits in the last few rows, and the menu reaches the very
    // bottom row (its dead space below the letterboxed menu is reclaimed).
    let proceed_y = (0..area.height).find(|&y| row_text(y).contains("Proceed")).expect("'Proceed' renders as terminal cells");
    assert!(proceed_y >= area.height - 8, "the menu is anchored to the pane bottom (Proceed at row {proceed_y} of {})", area.height);
    let last_text_y = (0..area.height).filter(|&y| !row_text(y).trim().is_empty()).max().unwrap();
    assert!(last_text_y >= area.height - 2, "the menu strip reaches the pane bottom edge (last text row {last_text_y})");

    // The story viewport fills the space ABOVE the menu, from near the pane top.
    let vp = state.transcript_geom.get().expect("hybrid renders the story as a transcript").area;
    assert!(vp.y <= 2, "the story is top-anchored (viewport top {} near the pane top)", vp.y);
    assert!(vp.bottom() <= proceed_y, "the story viewport stops above the bottom-anchored menu (vp bottom {}, menu at {proceed_y})", vp.bottom());
    assert!(vp.x > 0 && vp.right() < area.right(), "the story keeps its constant inset beside the picture column; got {vp:?}");
}

/// (SQ-0511 divider continuity) At a TALL pane the reclaimed space between the story
/// and the bottom-anchored menu is now spanned by the STRETCHED left flank band: the
/// picture column AND its authentic vertical divider (reversed text runs at native
/// x≈233, rasterized into the chrome canvas → part of the left side band, not a text
/// strip) run UNBROKEN from the frame top down to the menu strip. Probe first: the
/// divider is left-flank ring ART (image cells), not terminal text; then assert the
/// left flank carries a ring image cell on EVERY row from the top to the menu — no
/// clipped gap where the old reclaim blanked the flank below the picture's art.
#[test]
fn journey_hybrid_tall_pane_divider_reaches_menu() {
    use ratatui::style::Color;
    let Some(session) = journey_at_menu() else { return };
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    let area = Rect::new(0, 0, 90, 40);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    let vp = state.transcript_geom.get().expect("hybrid renders the story as a transcript").area;
    // The menu is bottom-anchored terminal cells; find its top ("Proceed" row).
    let proceed_y = (0..area.height).find(|&y| row_text(y).contains("Proceed")).expect("'Proceed' menu row");
    assert!(vp.bottom() <= proceed_y, "the reclaimed story stops above the menu (vp bottom {}, menu {proceed_y})", vp.bottom());

    // The divider/flank is chrome-ring ART, not terminal text: a cell is a ring image
    // when it is a halfblock glyph or carries a concrete background colour.
    let is_img = |x: u16, y: u16| -> bool {
        let c = buf.cell((x, y)).unwrap();
        let s = c.symbol();
        s == "\u{2580}" || s == "\u{2584}" || c.bg != Color::Reset
    };
    // Every row from the frame top down to the menu strip carries a left-flank ring
    // image cell — the picture column + divider run unbroken to the menu (the old
    // clip-to-art-extent reclaim left a bare-backdrop gap below the picture art).
    for y in 0..proceed_y {
        assert!(
            (0..vp.x).any(|x| is_img(x, y)),
            "the left picture-column/divider must be continuous to the menu — no ring image on row {y} (menu at {proceed_y}); row: {:?}",
            row_text(y)
        );
    }
    // And it abuts the story viewport with no seam at a deep reclaimed row.
    let dy = proceed_y.saturating_sub(1);
    let lmax = (0..vp.x).rev().find(|&x| is_img(x, dy)).expect("left flank ring cells present near the menu");
    assert_eq!(lmax, vp.x - 1, "the divider/flank abuts the story viewport with no seam (lmax {lmax}, vp.x {})", vp.x);

    // SQ-0511 fix: the left picture column keeps its ASPECT — it is NOT vertically
    // stretched to fill the reclaimed slack. A picture pixel is a real image glyph
    // (▀/▄ halfblock with per-cell colour), so the picture art's rendered rows are
    // exactly the rows carrying WIDE glyph coverage. Under the uniform scale those
    // rows stay within the story's native bottom mapped through that scale; the old
    // stretch filled the WHOLE column with picture glyphs right down to the menu.
    use app::engine::WinNode;
    use app::render::v6_layout as v6;
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = v6::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let story = v6::classify_windows(items, zvm::screen::V6Cell::DEFAULT).story.expect("story window");
    let fs = ratatui_image::picker::Picker::halfblocks().font_size();
    let (cw, ch) = (fs.width.max(1) as u32, fs.height.max(1) as u32);
    let s = v6::uniform_scale(native, (area.width as u32 * cw, area.height as u32 * ch)).s;
    let story_bottom = story.y_px as u32 + story.h_px as u32;
    // Top-anchored (off_y = 0): the story's native bottom maps to this device row.
    let story_bottom_row = (story_bottom as f32 * s / ch as f32).floor() as u16;
    let is_glyph = |x: u16, y: u16| {
        let s = buf.cell((x, y)).unwrap().symbol();
        s == "\u{2580}" || s == "\u{2584}"
    };
    let picture_row = |y: u16| (0..vp.x).filter(|&x| is_glyph(x, y)).count() as u16 > vp.x / 2;
    let picture_rows: Vec<u16> = (0..proceed_y).filter(|&y| picture_row(y)).collect();
    let picture_top = *picture_rows.first().expect("picture rows present");
    let picture_bottom = *picture_rows.last().expect("picture rows present");
    let picture_h = picture_bottom - picture_top + 1;
    // The picture's HEIGHT is what proves the aspect is kept: at the uniform scale
    // it can never exceed the story's native bottom mapped through that scale, and
    // the old stretch filled the whole column with picture glyphs down to the menu.
    assert!(
        picture_h <= story_bottom_row + 2,
        "the picture keeps its aspect (uniform scale): it occupies {picture_h} rows \
         ({picture_top}..={picture_bottom}), which must stay within the story's \
         uniform-scaled native height ({story_bottom_row} rows); NOT stretched to the menu {proceed_y}"
    );
    // SQ-0547: and it is CENTRED vertically in the flank column rather than
    // top-anchored, with the game's own panel background filling above and below.
    let above = picture_top.saturating_sub(vp.y);
    let below = (vp.y + vp.height).saturating_sub(1).saturating_sub(picture_bottom);
    assert!(
        above.abs_diff(below) <= 2,
        "the picture is centred in the flank column: {above} rows above vs {below} below \
         (picture {picture_top}..={picture_bottom}, column {}..{})",
        vp.y,
        vp.y + vp.height
    );
    // A genuine reclaimed gap remains below the aspect-correct picture, and NO wide
    // picture glyphs bleed into it (the old stretch put picture glyphs on every row).
    assert!(
        proceed_y > picture_bottom + 3,
        "a reclaimed gap remains below the aspect-correct picture (picture {picture_bottom}, menu {proceed_y})"
    );
    for y in (picture_bottom + 1)..proceed_y {
        assert!(
            !picture_row(y),
            "row {y} below the picture carries no stretched picture glyphs; row: {:?}",
            row_text(y)
        );
    }
    // The divider — NOT the dark gap fill — is carried on down through the reclaimed
    // gap to the menu: on a deep reclaimed row the flank cell abutting the story
    // carries the frame's rule and the interior does not.
    let deep = proceed_y.saturating_sub(2);
    assert!(deep > picture_bottom, "the deep probe row {deep} is inside the reclaimed gap (picture {picture_bottom})");
    assert!(
        is_divider(&buf, vp.x - 1, deep) && !is_divider(&buf, vp.x / 2, deep),
        "the divider reaches the reclaimed row {deep}: the flank's edge cell must carry the \
         frame's rule and the interior must not (edge {:?}, interior {:?})",
        buf.cell((vp.x - 1, deep)).unwrap(),
        buf.cell((vp.x / 2, deep)).unwrap()
    );
}

/// SQ-0547 follow-up: the panel fill must reach the flank's right edge, and the
/// divider must run the WHOLE column.
///
/// Two defects the first cut shipped, both visible at the TTY. (a) The art was
/// cropped to the flank's full native column width, so Journey's transparent side
/// margins (its picture occupies native x 5..226 of 240) were dragged into the
/// drawn image and rendered as dark cells ON TOP of the panel fill — a strip of
/// "missing background" down the right of the picture. The crop is now tight to
/// the art's bounding box on both axes. (b) Cropping tight then dropped the
/// divider on the picture's own rows, because the divider column lives at native
/// x≈233, outside that box — it used to ride along in the band image. The divider
/// extension now spans the whole column instead of starting below the art.
#[test]
fn journey_hybrid_tall_pane_panel_fill_reaches_the_divider() {
    use ratatui::style::Color;
    let Some(session) = journey_at_menu() else { return };
    let model = session.screen();
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    let area = Rect::new(0, 0, 90, 40);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    let vp = state.transcript_geom.get().expect("hybrid transcript").area;
    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    let proceed_y = (0..area.height).find(|&y| row_text(y).contains("Proceed")).expect("'Proceed' menu row");

    // (b) The divider reaches every row of the column — including the rows the
    // picture sits on, and the rows ABOVE it, not just the reclaimed gap below.
    for y in [vp.y + 1, vp.y + vp.height / 2, proceed_y.saturating_sub(2)] {
        assert!(
            is_divider(&buf, vp.x - 1, y),
            "the divider runs the whole flank column: row {y} col {} carries {:?} (row: {:?})",
            vp.x - 1,
            buf.cell((vp.x - 1, y)).unwrap(),
            row_text(y)
        );
    }

    // (c) A column of panel fill separates the picture from the divider, so the panel
    // frames the art on that side the way the art's own native left margin frames it
    // on the other. Walk in from the divider: the first cell left of it must be fill
    // (a plain space), not a halfblock of the picture.
    let mid = vp.y + vp.height / 2;
    let mut dx = vp.x;
    while dx > 1 && is_divider(&buf, dx - 1, mid) {
        dx -= 1;
    }
    let gap = buf.cell((dx - 1, mid)).unwrap();
    assert!(
        gap.symbol().trim().is_empty(),
        "a panel-fill column must sit between the picture and the divider: col {} on row {mid} \
         holds {:?} (divider starts at col {dx})",
        dx - 1,
        gap.symbol()
    );

    // (a) Every flank cell carries SOMETHING the game painted — panel fill, picture
    // or divider — with no untouched theme-backdrop cells left behind. A `Reset`
    // background is the tell: that is a cell nothing drew.
    for y in vp.y..proceed_y.min(vp.y + vp.height) {
        for x in 0..vp.x {
            assert_ne!(
                buf.cell((x, y)).unwrap().bg,
                Color::Reset,
                "flank cell ({x},{y}) kept the theme backdrop — the panel fill must cover \
                 the whole column (row: {:?})",
                row_text(y)
            );
        }
    }
}

/// SQ-0548: no width-dependent dark bar between the flank panel and the command menu.
///
/// The menu strip's top cell used to be the story's native bottom mapped through the
/// bottom-anchored menu scale and rounded DOWN. That scale and the story scale round
/// independently, so at some pane widths the floor landed one row ABOVE the first menu
/// row, and the leftover row joined the menu band carrying no runs. A run-less row is
/// classed `Empty` and coalesces into an ART strip, so it redrew a squashed slice of the
/// frame's bottom edge full-width across the pane — over the flank panel fill and its
/// divider. That was the dark bar under the left picture column that grew and shrank
/// with the window: at widths where the floor happened to land on the first menu row
/// there was no leftover row and no bar.
///
/// **This must be probed at REAL cell metrics (8×18).** `Picker::halfblocks()` uses 1×2
/// cells — a completely different layout regime that never reproduces the defect, and a
/// halfblocks sweep reported every width clean while the TTY showed the bar.
#[test]
#[allow(deprecated)]
fn journey_hybrid_flank_panel_meets_the_menu_at_every_width() {
    use ratatui::style::Color;
    let Some(session) = journey_at_menu() else { return };
    let model = session.screen();

    for w in [96u16, 104, 110, 116, 122, 128, 134, 138, 144, 150] {
        let mut state = app::state::AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        state.game_picker =
            Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
        state.config.v6_render = app::config::V6RenderMode::Hybrid;
        let area = Rect::new(0, 0, w, 51);
        let mut buf = Buffer::empty(area);
        let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

        let vp = state.transcript_geom.get().expect("hybrid transcript").area;
        let row_text = |y: u16| -> String {
            (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
        };
        // The menu's own first row — its header, above "Proceed".
        let header_y =
            (0..area.height).find(|&y| row_text(y).contains("The Party")).expect("the menu header row");

        // (a) The story viewport ends exactly where the menu begins: no leftover row
        // between them for an Empty→Art strip to claim.
        assert_eq!(
            vp.bottom(),
            header_y,
            "w={w}: the story viewport must meet the menu with no leftover row \
             (viewport bottom {}, menu header {header_y})",
            vp.bottom()
        );

        // (b) And the flank column is continuous into the menu: the three rows above
        // the header carry the SAME panel fill and the SAME divider colour. The bar
        // showed up as the bottom one of these differing from the two above it.
        let bg = |x: u16, y: u16| buf.cell((x, y)).unwrap().bg;
        // The divider is probed at ITS OWN column and compared as a whole CELL, not as
        // the background of the column beside the viewport. Since SQ-0750 the rule is
        // the game's own character, placed by the same mapping the frame's top rule
        // uses for its corner, so it stands one column wide where the game drew it —
        // which at some pane widths is not `vp.x - 1`. The viewport is quantized INWARD
        // to whole cells and the rule OUTWARD, and the column that can fall between
        // them belongs to neither (SQ-0747 item A): it keeps the story's own ground.
        let rule_x = state
            .v6_cell_map
            .borrow()
            .iter()
            .filter(|e| e.label.starts_with("flank-divider"))
            .map(|e| e.cells)
            .find(|c| c.0 < vp.x)
            .map(|c| c.0)
            .unwrap_or(vp.x - 1);
        let edge_cell = |y: u16| buf.cell((rule_x, y)).unwrap().clone();
        let (fill, edge) = (bg(2, header_y - 3), edge_cell(header_y - 3));
        assert!(
            matches!(fill, Color::Rgb(..)) && is_divider(&buf, rule_x, header_y - 3),
            "w={w}: the flank column carries the game's panel fill and divider at column \
             {rule_x} (fill {fill:?}, edge {edge:?})"
        );
        for y in (header_y - 2)..header_y {
            assert_eq!(
                bg(2, y),
                fill,
                "w={w}: the panel fill runs unbroken to the menu — row {y} interior is {:?}, \
                 not the {fill:?} of row {} (row: {:?})",
                bg(2, y),
                header_y - 3,
                row_text(y)
            );
            assert_eq!(
                edge_cell(y),
                edge,
                "w={w}: the divider runs unbroken to the menu — row {y} edge is {:?}, \
                 not the {edge:?} of row {} (row: {:?})",
                edge_cell(y),
                header_y - 3,
                row_text(y)
            );
        }
    }
}

/// SQ-0550: a click on a menu label must reach THAT label's game row.
///
/// The hybrid click map inverts the letterbox linearly, but the menu is a TEXT strip
/// and `draw_chrome_text_strip` does not place its runs through that scale — it buckets
/// them by game row and packs the buckets onto CONSECUTIVE terminal rows from the
/// strip's top (SQ-0543). The two row pitches differ, so the linear inverse drifted:
/// at a 138-column pane (scale 1.725) the letterbox spreads game rows 1.53 terminal
/// rows apart while the strip draws them 1 apart, and every command below the first
/// mapped a row high — "Game", five rows down, mapped two rows high. The player had to
/// click one line BELOW the command they wanted.
///
/// Drives the real geometry: render, find where each label is actually drawn, click its
/// first glyph cell, and require the recovered game pixel to fall inside that label's
/// own 16px game row. Swept across widths because the drift scales with the letterbox —
/// at w=96 (scale 1.2, ≈1 terminal row per game row) the old code was already correct,
/// which is exactly why this needs more than one width to catch.
#[test]
#[allow(deprecated)]
fn journey_hybrid_menu_clicks_hit_the_row_they_land_on() {
    use app::engine::WinNode;
    let Some(session) = journey_at_menu() else { return };
    let model = session.screen();

    // Every menu label the game painted, with the native row it belongs to. The
    // arrows repeat verbatim on four rows, so they cannot be located by text search.
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let mut labels: Vec<(String, u16)> = Vec::new();
    for pw in items {
        if let WinNode::Grid(g) = &pw.node {
            for t in &g.px_texts {
                let row = (t.y.max(1) - 1) / 16;
                if !t.text.trim().is_empty() && row >= 19 && t.text.trim() != "-->" {
                    labels.push((t.text.trim().to_string(), row));
                }
            }
        }
    }
    assert!(labels.len() >= 8, "the command menu paints its labels: {labels:?}");

    for w in [96u16, 110, 122, 138, 150] {
        let mut state = app::state::AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        state.game_picker =
            Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
        state.config.v6_render = app::config::V6RenderMode::Hybrid;
        let area = Rect::new(0, 0, w, 51);
        let mut buf = Buffer::empty(area);
        let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
        let map = state.graphics_render.borrow().last_v6_map.clone().expect("the hybrid path records a click map");
        let row_text = |y: u16| -> String {
            (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
        };

        for (label, native_row) in &labels {
            let Some(sy) = (0..area.height).find(|&y| row_text(y).contains(label.as_str())) else {
                panic!("w={w}: menu label {label:?} is drawn somewhere");
            };
            let sx = row_text(sy).find(label.as_str()).expect("column of the label") as u16;
            let (_, gy) = map.map_click(sx, sy).unwrap_or_else(|| {
                panic!("w={w}: clicking {label:?} at its drawn cell ({sx},{sy}) maps into the game")
            });
            assert_eq!(
                (gy.max(1) - 1) / 16,
                *native_row,
                "w={w}: clicking {label:?} where it is DRAWN (cell {sx},{sy}) must reach its own \
                 game row {native_row} — got game pixel y={gy} (row {})",
                (gy.max(1) - 1) / 16
            );
        }
    }
}

/// SQ-0540: Journey stamps its selected command-menu label ("Start", "Proceed",
/// "Combat", "Cast", …) as a painted run with §8.7.1 style bit 2 — the one v6
/// moment where a real game's BOLD reaches the raster path. The composite must
/// synthesize a bold face for it (double-strike, +1 px) instead of drawing the
/// same roman glyphs as an unstyled run.
#[test]
fn journey_bold_menu_label_rasterizes_emboldened() {
    use app::render::v6_layout as v6;
    let Some(session) = journey_at_menu() else { return };
    let mut model = session.screen();
    let WinNode::Layered(items) = &mut model.root else { panic!("v6 Layered root") };

    // The bold runs Journey paints this frame (the highlighted verb label).
    let bold: Vec<String> = items
        .iter()
        .filter_map(|it| match &it.node {
            WinNode::Grid(g) => Some(g.px_texts.iter().filter(|t| t.style & 2 != 0).map(|t| t.text.clone()).collect::<Vec<_>>()),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(!bold.is_empty(), "Journey's command menu must carry at least one bold painted run at the menu page");
    eprintln!("Journey bold menu run(s): {bold:?}");

    let native = v6::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let colors = app::colors::ColorScheme::terminal_default();
    let fg = image::Rgba([220, 220, 220, 255]);
    let bg = image::Rgba([0, 0, 0, 255]);
    let render = |items: &[app::engine::PositionedWindow]| {
        let layout = v6::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
        v6::build_chrome_canvas(&layout.chrome, native, fg, bg, &colors, v6::TextLayer::All, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT))
    };
    let with_bold = render(items);

    // Same frame with the bold bit cleared: the roman rendering of the same text.
    for it in items.iter_mut() {
        if let WinNode::Grid(g) = &mut it.node {
            for t in g.px_texts.iter_mut() {
                t.style &= !2;
            }
        }
    }
    let roman = render(items);

    assert_ne!(with_bold, roman, "the bold label must not rasterize identically to its roman self");
    let lit = |c: &image::RgbaImage| -> std::collections::BTreeSet<(u32, u32)> {
        c.enumerate_pixels().filter(|(_, _, p)| p[3] >= 128).map(|(x, y, _)| (x, y)).collect()
    };
    let (b, r) = (lit(&with_bold), lit(&roman));
    assert!(r.is_subset(&b), "double-strike is additive — bold keeps every roman pixel");
    for &(x, y) in b.difference(&r) {
        assert!(x > 0 && r.contains(&(x - 1, y)), "bold pixel ({x},{y}) is not a +1 double-strike of the roman face");
    }
}

/// (SQ-0491) FRAMELESS, both defects at once.
///
/// (a) Journey's half-screen LEFT PICTURE COLUMN (win3 graphics, native
///     0,0–232×304) rendered in raster and hybrid but was dropped entirely by the
///     frameless cell path, which only ever drew chrome TEXT. A chrome graphics
///     window that lies wholly BESIDE the story window is story content, not frame
///     art, so it now gets its own native-proportional column and the transcript is
///     inset beside it. (Frame art that spans or overlaps the story — Arthur's
///     header panel, every game's backdrop — is still dropped: that is what
///     frameless means.)
/// (b) The command MENU was stamped at its absolute native rows 19–24, so it landed
///     over the story text at any pane taller than the native 25 rows. It is now
///     packed against the pane's BOTTOM edge, with the story stopping above it.
///
/// Checked at the native height (80×25, where the old absolute stamp happened to
/// coincide) and at a taller pane (80×40, where it did not), in BOTH
/// `honor_game_colours` modes.
#[test]
fn journey_frameless_shows_picture_column_and_pins_menu_to_the_bottom() {
    use ratatui::style::Color;
    let Some(session) = journey_at_menu() else { return };
    let model = session.screen();

    for honor in [true, false] {
        for (w, h) in [(80u16, 25u16), (80, 40)] {
            let mut state = app::state::AppState::default();
            state.colors = app::colors::ColorScheme::terminal_default();
            state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
            // Force the CELL path: SQ-0895 removed frameless, which was the
            // deliberate route in. Dropping the picker is the substitute whose
            // ONLY effect is the one frameless contributed here — draw no game
            // image. (A modal overlay also lands on the cell path, but it
            // additionally suppresses the inlined input line, which shifts row
            // counts.)
            state.game_picker = None;
            state.config.honor_game_colours = honor;
            let area = Rect::new(0, 0, w, h);
            let mut buf = Buffer::empty(area);
            let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

            let row_text = |y: u16| -> String {
                (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
            };
            let ctx = format!("(honor={honor}, {w}x{h})");

            // (b) The menu is locked to the bottom: its header is `menu_rows` up
            // from the pane bottom and its last verb ("Game") is on the LAST row.
            let header_y = (0..area.height)
                .find(|&y| row_text(y).contains("The Party"))
                .unwrap_or_else(|| panic!("the menu header renders as terminal cells {ctx}"));
            assert_eq!(
                header_y,
                area.height - 6,
                "the 6-row menu band is packed against the pane bottom {ctx}"
            );
            assert!(
                row_text(area.height - 1).contains("Game"),
                "the menu's last row sits on the pane's last row {ctx}: {:?}",
                row_text(area.height - 1)
            );
            for verb in ["Proceed", "Praxix", "Inventory"] {
                let y = (0..area.height).find(|&y| row_text(y).contains(verb)).unwrap_or_else(|| panic!("{verb} present {ctx}"));
                assert!(y >= area.height - 6, "'{verb}' is inside the bottom menu band (row {y}) {ctx}");
            }

            // (a) The left picture column is drawn, and the story transcript is
            // inset to its RIGHT rather than spanning the whole pane.
            let vp = state.transcript_geom.get().expect("frameless publishes transcript geometry").area;
            assert!(vp.x >= 29, "the transcript is inset beside the picture column (x {}) {ctx}", vp.x);
            assert!(vp.bottom() <= header_y, "the story stops above the menu (bottom {}, menu at {header_y}) {ctx}", vp.bottom());
            let picture_ink = (0..header_y)
                .flat_map(|y| (0..29u16).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    let c = buf.cell((x, y)).unwrap();
                    let s = c.symbol();
                    s == "\u{2580}" || s == "\u{2584}" || c.bg != Color::Reset
                })
                .count();
            assert!(picture_ink > 100, "the left picture column renders (only {picture_ink} inked cells) {ctx}");
        }
    }
}
