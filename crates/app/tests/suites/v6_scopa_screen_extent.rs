//! SQ-0710: a v6 window sized past the screen must not inflate the composite.
//!
//! scopa measures text by sizing a scratch window enormous so the string it is
//! about to print cannot wrap — straight from the game's own Inform source:
//!
//! ```text
//! textwidth [ txt ws tw;
//!     @window_size 5 1000 1000;
//!     ws = WinSet(5);
//! ```
//!
//! `native_extent` takes the union of every published window box, so once a hand
//! was dealt that one window pushed the composite to 1579×1370: the whole picture
//! scaled down inside the pane ("the screen zooms out"), crammed into the top-left,
//! with large black rectangles where the oversized window's page painted outside
//! the real 640×400 screen.
//!
//! The screen's real size is the header's (ZMSD §8.4.3, word $22 = width in units,
//! word $24 = height in units; v6 units are pixels), which `zvm::screen` seeds at
//! boot. `GameSession::v6_clip_box` clips each PUBLISHED box to it — and only the
//! published box: the VM keeps storing what the game wrote, so `get_wind_prop`
//! still answers 1000, which is exactly what scopa's own measurement depends on.
//!
//! The story is gitignored (CLAUDE.md), so these skip cleanly when it is absent.


use app::engine::Engine;
use app::graphics::PictSource;
use app::session::GameSession;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::fixture_paths::fixture_path;


/// Boot scopa and deal a hand. Its menu is mouse-driven: a click reaches
/// `read_char` as ZSCII 254 (ZMSD §3.8) with the coordinates already set, and
/// `Engine::set_mouse` takes them Y FIRST. The ace of hearts the title screen
/// invites you to click sits at roughly x 222..272, y 305..395; the deal follows,
/// and with it the `textwidth` call that oversizes window 5.
fn scopa_dealt() -> Option<GameSession> {
    let path = fixture_path("scopa.z6");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut s = GameSession::new_with_trace(
        bytes, true, false, None, false, dims, picts.std_window(), None, None,
    )
    .expect("scopa is a valid v6 story");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    for _ in 0..3 {
        match s.pending_input() {
            app::session::InputKind::Char => {
                Engine::set_mouse(&mut s, 350, 245);
                let _ = s.submit_char(254);
            }
            _ => {
                let _ = s.submit("");
            }
        }
        let _ = s.take_transcript();
    }
    Some(s)
}

/// scopa's card table is a flat green the game fills with `erase_window`
/// (ZMSD §8.3.1 green, `Rgb(0, 132, 0)` through the pixel path). Matched by shape
/// rather than by that exact triple so an anti-aliased edge cell still counts.
fn is_baize(c: Color) -> bool {
    matches!(c, Color::Rgb(r, g, b) if g >= 100 && r < 90 && b < 90)
}

fn items(model: &app::engine::ScreenModel) -> &[app::engine::PositionedWindow] {
    match &model.root {
        app::engine::WinNode::Layered(v) => v,
        other => panic!("v6 builds a Layered root, got {other:?}"),
    }
}

/// The published screen stops at the screen, and the VM's own table does not.
///
/// Falsified by returning `(x_size, y_size)` unclipped from `v6_clip_box`:
/// "a scratch measuring window must not enlarge the screen: left=1579x1370,
/// right=640x400".
#[test]
fn a_measuring_window_never_inflates_the_published_screen() {
    let Some(session) = scopa_dealt() else { return };
    let model = session.screen();
    let items = items(&model);

    assert_eq!(
        app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT)),
        (640, 400),
        "a scratch measuring window must not enlarge the screen"
    );
    for pw in items {
        assert!(
            pw.x_px as u32 + pw.w_px as u32 <= 640 && pw.y_px as u32 + pw.h_px as u32 <= 400,
            "every published box ends inside the 640x400 screen: ({},{}) {}x{}",
            pw.x_px, pw.y_px, pw.w_px, pw.h_px
        );
    }

    // …and the clip is PRESENTATION only. The game reads its measuring window's
    // size straight back through `get_wind_prop` (ZMSD §8.8.3.2) — clipping the
    // VM's stored size would make `textwidth` measure against a 640px window and
    // wrap the very string it is sizing. `/dump-windows` reports both halves.
    let dump = session.v6_window_dump(&[], None).join("\n");
    assert!(
        dump.contains("win5  1000x1000"),
        "zvm still stores the size the game wrote, so get_wind_prop is unaffected:\n{dump}"
    );
    assert!(
        dump.contains("clipped to 61x30"),
        "…and the dump says which part of it is on screen:\n{dump}"
    );
    assert!(
        dump.contains("win5") && !dump.split("win5").nth(1).unwrap().contains("not published"),
        "the clipped box must still match its model node, or /dump-windows loses the window:\n{dump}"
    );
}

/// The composite the player sees is the game's screen, at the game's scale, with
/// no page painted outside it.
///
/// Both `honor_game_colours` modes: the cards are the game's own painted ground,
/// not a palette preference, and the extent must not depend on the theme either.
/// Several pane sizes, because a scale fault can hide at one of them.
///
/// Only RASTER here. Hybrid publishes no story window for scopa and takes a
/// different arm entirely — that is SQ-0711, and `v6_scopa_hybrid_no_story.rs`
/// covers it end to end once it lands.
///
/// Falsified by returning `(x_size, y_size)` unclipped from `v6_clip_box`:
/// "honor=true: the composite IS the game's screen: left=1579x1370, right=640x400".
#[test]
fn the_dealt_table_composites_at_screen_scale_and_paints_no_black_bands() {
    let Some(session) = scopa_dealt() else { return };
    let model = session.screen();
    let items = items(&model);
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);

    for honor in [true, false] {
        // The composite is built once per colour mode: it is the game's own
        // screen either way.
        let mut state = app::state::AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.v6_render = app::config::V6RenderMode::Raster;
        state.config.honor_game_colours = honor;
        *state.v6_paint.borrow_mut() = session.paint_surface();

        let (canvas, _m) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
        assert_eq!(
            (canvas.width(), canvas.height()),
            (640, 400),
            "honor={honor}: the composite IS the game's screen"
        );

        // …and at the END of the pipeline, across pane sizes: what the drawn
        // image reports as its native extent is what a click is mapped through,
        // and what the pane is actually FULL of. Each size gets its own state —
        // the raster encode is cached per pane, and re-using one across sizes
        // measures the previous size's stale upload rather than this one.
        for (w, h) in [(100u16, 34u16), (80, 25), (132, 50), (64, 20)] {
            let mut state = app::state::AppState::default();
            state.colors = app::colors::ColorScheme::terminal_default();
            state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
            state.config.v6_render = app::config::V6RenderMode::Raster;
            state.config.honor_game_colours = honor;
            *state.v6_paint.borrow_mut() = session.paint_surface();

            let area = Rect::new(0, 0, w, h);
            let mut buf = Buffer::empty(area);
            let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
            let map = state
                .graphics_render
                .borrow()
                .last_v6_map
                .clone()
                .expect("the raster path records a click map");
            assert_eq!(
                (map.canvas, map.screen),
                ((640, 400), (640, 400)),
                "honor={honor} pane={w}x{h}: the drawn image spans the game's screen"
            );
            // Aspect: 640/400 = 1.60. The inflated extent was 1579/1370 = 1.15,
            // and that squarer picture is what left the table crammed and small.
            let aspect = map.img_w / map.img_h.max(1.0);
            assert!(
                (aspect - 1.6).abs() < 0.2,
                "honor={honor} pane={w}x{h}: the picture keeps the screen's shape, got {aspect:.3}"
            );
            // …and what the pane is FULL of. Before the clip it was the wrong
            // things: at 100x34 the pane came back 1125 white cells and 966 BLACK
            // ones — the oversized window's page, and the ground past the real
            // screen that nothing ever painted — with the felt down to 170 of 3400.
            //
            // How much felt is a COLOUR-MODE question, and deliberately so. The
            // table's background is window 1's page, which `fill_window_pages`
            // paints only when the game's colours are honoured (SQ-0704); the two
            // green bands the game fills with `erase_window` are drawing, and reach
            // the pane either way (SQ-0706). So honour the pair and the baize fills
            // the pane; decline it and the theme's page shows through between the
            // game's own fills. Both are pinned, at their own thresholds.
            let cells = (w as usize) * (h as usize);
            let green = buf.content().iter().filter(|c| is_baize(c.bg)).count();
            let floor = if honor { cells / 2 } else { cells / 20 };
            assert!(
                green > floor,
                "honor={honor} pane={w}x{h}: the table reaches the pane — {green} of {cells} \
                 cells are baize, wanted more than {floor}"
            );
            if honor {
                let black = buf.content().iter().filter(|c| c.bg == Color::Rgb(0, 0, 0)).count();
                assert!(
                    black * 20 < cells,
                    "honor={honor} pane={w}x{h}: no black band beside the table — {black} of {cells} cells"
                );
            }
        }
    }
}
