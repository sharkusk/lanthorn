//! SQ-0706: scopa draws with `erase_window`, and those fills must reach the screen.
//!
//! scopa.z6 uses no `draw_picture` for its deck at all. From the game's own Inform
//! source, `drawpic` sends anything numbered 40 or below to `HardPic`, which
//! decodes run-length card data and emits each run as a filled rectangle:
//!
//! ```text
//! [ fastsimplebox x y w h c;
//!     @window_size 3 h w; @move_window 3 y x; @set_colour 2 c 3; @erase_window 3; ];
//! ```
//!
//! So one card is hundreds of moves-and-erases of a single window, and the
//! rectangle IS the erase. Reading only the canvas-clear sentinel left one empty
//! canvas at the window's last position and drew no cards at all.
//!
//! The story is gitignored (CLAUDE.md), so these skip cleanly when it is absent.


use app::engine::Engine;
use app::graphics::PictSource;
use app::session::GameSession;

use crate::fixture_paths::fixture_path;


/// Boot scopa and click into a game — its menu is mouse-driven, and a click
/// reaches `read_char` as ZSCII 254 (ZMSD §3.8) with the coordinates already set.
fn scopa_in_play() -> Option<GameSession> {
    let path = fixture_path("scopa.z6");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut s =
        GameSession::new_with_trace(bytes, true, false, None, false, dims, picts.std_window(), None, None)
            .expect("scopa is a valid v6 story");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    for _ in 0..6 {
        match s.pending_input() {
            app::session::InputKind::Char => {
                Engine::set_mouse(&mut s, 230, 320);
                let _ = s.submit_char(254);
            }
            app::session::InputKind::Line => {
                let _ = s.submit("");
            }
            app::session::InputKind::Event => {
                let _ = s.submit("");
            }
        }
        let _ = s.take_transcript();
        if s.paint_surface().is_some() {
            break;
        }
    }
    Some(s)
}

/// The engine's fills become real pixels on a real surface.
#[test]
fn scopa_paints_a_ground_the_renderer_can_composite() {
    let Some(session) = scopa_in_play() else { return };
    let paint = session
        .paint_surface()
        .expect("scopa paints its table with erase_window fills — before SQ-0706 nothing accumulated");

    assert_eq!((paint.width(), paint.height()), (640, 400), "the ground is the v6 screen");

    let opaque = paint.pixels().filter(|p| p[3] > 0).count();
    assert!(
        opaque > 5_000,
        "the fills must cover a real part of the screen, not a stray box: {opaque} painted pixels"
    );

    // A card is runs of DIFFERENT colours (face, pips, border). One flat colour
    // would mean the fills' own `bg` was being ignored.
    let mut colours = std::collections::BTreeSet::new();
    for p in paint.pixels().filter(|p| p[3] > 0) {
        colours.insert((p[0], p[1], p[2]));
    }
    assert!(
        colours.len() > 2,
        "the painted ground carries the colours the game filled with; got {} distinct",
        colours.len()
    );
}

/// …and those pixels survive into what the player actually sees.
///
/// scopa publishes no Buffer window at all — its screen is three Grids — so
/// hybrid's `if let Some(story)` ring arm is skipped. This file used to claim the
/// frame then "falls through" to the raster composite. It did not: the next arm
/// down is the painted-screen text takeover (SQ-0477), which drew scopa's two
/// button labels and discarded the table. SQ-0711 sends a screen with a painted
/// ground to the composite instead, so both render modes now really do reach it —
/// asserted end to end in `v6_scopa_hybrid_no_story.rs`, which is why this test
/// can drive the composite directly.
///
/// Both colour modes ARE pinned: painted pixels are the game's own drawing, not a
/// palette preference, so declining game colours must not erase its cards.
///
/// Falsified by dropping `blit_paint_ground`: "Raster/honor=true: the cards the game
/// painted must survive into the composite — only 44 of 254 sampled pixels did"
/// (the 44 being coincidental matches against the ground beneath).
#[test]
fn the_painted_cards_reach_the_composite_in_both_modes() {
    let Some(session) = scopa_in_play() else { return };
    let Some(paint) = session.paint_surface() else { return };
    let model = session.screen();
    let app::engine::WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);

    // Sample where the game actually painted, so the test cannot pass vacuously.
    let samples: Vec<(u32, u32, image::Rgba<u8>)> = (0..paint.height())
        .step_by(7)
        .flat_map(|y| (0..paint.width()).step_by(11).map(move |x| (x, y)))
        .filter_map(|(x, y)| {
            let p = *paint.get_pixel(x, y);
            (p[3] == 255).then_some((x, y, p))
        })
        .take(400)
        .collect();
    assert!(samples.len() > 50, "the ground must give us plenty to check: {}", samples.len());

    assert!(
        layout.story.is_none(),
        "scopa publishes no Buffer window, so hybrid's ring arm is skipped — which arm it lands \
         on instead is SQ-0711's business (v6_scopa_hybrid_no_story.rs); both end at this composite"
    );

    for honor in [true, false] {
        let mut state = app::state::AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.honor_game_colours = honor;
        *state.v6_paint.borrow_mut() = Some(paint.clone());

        let (canvas, _metrics) =
            app::render::screen::build_v6_raster_canvas(&layout, native, &state);

        let hits = samples
            .iter()
            .filter(|&&(x, y, p)| canvas.get_pixel_checked(x, y).is_some_and(|c| *c == p))
            .count();
        assert!(
            hits * 100 >= samples.len() * 80,
            "Raster/honor={honor}: the cards the game painted must survive into the composite \
             — only {hits} of {} sampled pixels did",
            samples.len()
        );
    }
}
