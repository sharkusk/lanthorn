//! SQ-0711: hybrid mode has to draw a v6 game that publishes NO story window.
//!
//! Hybrid's ring arm is gated on `layout.story` — the primary `Buffer` the
//! transcript streams through. scopa has none: its screen is three Grids, and its
//! card table is `erase_window` fills rather than prose. What happens next was
//! assumed to be a fall-through to the raster composite. It was not. The next arm
//! down is the painted-screen takeover (the InvisiClues/boot-menu path, SQ-0477),
//! which presents a story-less screen as positioned terminal text — and it draws
//! the text runs and NOTHING else. scopa's runs are its two buttons, so hybrid
//! rendered SEVEN non-blank cells out of a 100x34 pane while raster drew the
//! whole table.
//!
//! The discriminator is the painted ground. A screen with pixels on it can only be
//! shown by the composite, and the composite draws the runs over them anyway; a
//! screen with no pixels (Zork Zero's hint menu, Shogun's boot menu — both
//! story-less, neither painting a ground) is exactly what the text takeover is for
//! and keeps it. `v6_zork0_hints.rs` pins that side.
//!
//! SQ-0709 is the same seam seen from the other end, and this file is its guard.
//! It was filed against the arm ABOVE: hybrid's pane flood reads
//! `story_bg_rgba`, which only looks at a `WinNode::Buffer`, so scopa — whose page
//! is declared by a full-screen GRID — flooded nothing and its green table came
//! out as the terminal background while raster drew it correctly. Routing a
//! painted story-less screen to the composite closed that: hybrid no longer runs
//! the flood at all for scopa, and `hybrid_draws_the_table_instead_of_two_button_labels`
//! pins both halves — that the table reaches the pane, and that hybrid and raster
//! agree byte for byte. Re-measured across the corpus afterwards, no other v6
//! title declares its page from a grid while its story Buffer declares none
//! (Zork Zero and fmvpoker declare both; Arthur, Shogun, Journey and advent
//! declare neither), so the original mechanism has no remaining reproducer.
//!
//! The story is gitignored (CLAUDE.md), so these skip cleanly when it is absent.


use app::engine::Engine;
use app::graphics::PictSource;
use app::session::GameSession;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::fixture_paths::fixture_path;


/// Boot scopa and deal a hand — `Engine::set_mouse` takes the coordinates Y
/// FIRST, and the ace of hearts the title screen invites you to click sits at
/// roughly x 222..272, y 305..395. A click reaches `read_char` as ZSCII 254
/// (ZMSD §3.8) with the coordinates already set.
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

/// The premise: no story window, and pixels the game painted itself.
#[test]
fn scopa_publishes_no_story_window_but_does_paint_a_ground() {
    let Some(session) = scopa_dealt() else { return };
    let model = session.screen();
    let app::engine::WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    assert!(
        layout.story.is_none(),
        "scopa publishes no primary Buffer — its screen is Grids, so hybrid's ring arm is skipped"
    );
    let paint = session
        .paint_surface()
        .expect("…and its table is a painted ground (SQ-0706), which is what decides the arm below");
    assert!(
        paint.pixels().filter(|p| p[3] > 0).count() > 5_000,
        "the ground has to be real pixels, not a stray box"
    );
}

/// Hybrid draws the game. Same pane as raster, because a game with no story
/// window has no terminal viewport to ring and nothing for the ring to surround.
///
/// Both `honor_game_colours` modes and four pane sizes: the arm taken must not
/// depend on either.
///
/// Falsified by dropping the `!painted_ground` guard:
/// "Hybrid honor=true pane=100x34: the table reaches the pane — 0 of 3400 cells
/// are baize, wanted more than 1700", and the path stamp comes back
/// "painted (hint/menu takeover)" with 7 non-blank cells.
#[test]
fn hybrid_draws_the_table_instead_of_two_button_labels() {
    let Some(session) = scopa_dealt() else { return };
    let model = session.screen();

    for honor in [true, false] {
        for (w, h) in [(100u16, 34u16), (80, 25), (132, 50), (64, 20)] {
            let mut panes = Vec::new();
            for mode in [app::config::V6RenderMode::Hybrid, app::config::V6RenderMode::Raster] {
                // A fresh state per case: the raster encode is cached per pane, so
                // re-using one across sizes measures the previous size's upload.
                let mut state = app::state::AppState::default();
                state.colors = app::colors::ColorScheme::terminal_default();
                state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
                state.config.v6_render = mode;
                state.config.honor_game_colours = honor;
                *state.v6_paint.borrow_mut() = session.paint_surface();

                let area = Rect::new(0, 0, w, h);
                let mut buf = Buffer::empty(area);
                let _ =
                    app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

                let path = state.v6_path_log.borrow().last().map(|(l, _)| l.clone()).unwrap_or_default();
                assert_eq!(
                    path, "raster",
                    "{mode:?} honor={honor} pane={w}x{h}: a painted v6 screen with no story window \
                     goes to the composite, not to the text takeover"
                );

                // The table reaches the pane, in either colour mode. It used to be
                // a colour-mode question — the felt is window 1's page, painted only
                // when the game's colours were honoured (SQ-0704), while the bands
                // the game fills with `erase_window` are drawing and arrived either
                // way (SQ-0706) — which left a black table with green stripes on it.
                // SQ-0716 made a window the game DREW INTO keep its page regardless;
                // `v6_scopa_declined_colours.rs` pins that side in detail.
                let cells = (w as usize) * (h as usize);
                let green = buf.content().iter().filter(|c| is_baize(c.bg)).count();
                let floor = cells / 2;
                assert!(
                    green > floor,
                    "{mode:?} honor={honor} pane={w}x{h}: the table reaches the pane — {green} of \
                     {cells} cells are baize, wanted more than {floor}"
                );
                panes.push(buf);
            }
            assert_eq!(
                panes[0], panes[1],
                "honor={honor} pane={w}x{h}: with no story window to ring, hybrid and raster are \
                 the same frame"
            );
        }
    }
}
