//! SQ-0715: scopa offers three decks, and all three must be on the opening screen.
//!
//! The player reported seeing only one card at startup. scopa's menu invites you to
//! "click on a card type to begin" and lays out one sample card per deck it can
//! find: the Milanese deck hardwired into the z-code as vector data, plus whatever
//! picture decks the Blorb supplies (Neapolitan at base 100, Sicilian at base 300 —
//! the French deck at base 200 is not in the shipped `scopa.blb`, and the game's own
//! `measurepic` drops it). Only the vector deck ever reached the screen.
//!
//! Two things were wrong, and the game's own Inform source names both. Its `drawpic`
//! borrows window 3 as a scratch pad for every picture:
//!
//! ```text
//! if (pic > 40) { @move_window 3 y x;  @window_size 3 1000 1000;
//!                 ws = WinSet(3);  @draw_picture pic 1 1;  WinSet(ws); }
//! else HardPic(pic,x,y);
//! ```
//!
//! so by the time the host drained the queue that window had been moved and shrunk
//! to an 80×1 sliver somewhere else entirely — the card was clipped out of existence
//! and its canvas erased by the next fill. And `scopa.blb` carries no Blorb `Reso`
//! chunk, which makes its images non-scalable (Blorb §11: "always displayed at their
//! actual size"), so reporting doubled sizes from `picture_data` laid the two picture
//! decks out at 104×168 — overlapping each other and hanging off the screen.
//!
//! The story is gitignored (CLAUDE.md), so these skip cleanly when it is absent.


use app::engine::Engine;
use app::graphics::PictSource;
use app::session::GameSession;

use crate::fixture_paths::fixture_path;


/// The three sample cards on the opening menu, as the game lays them out:
/// `locateims` centres them 70 px apart at `x` = 250/320/390 and `y` = 350, each
/// card 52×84, so their 0-based screen rects start here. All three are the same
/// size — which is the point: the Blorb art is authored for the same 640×400
/// screen as the hardwired vector deck.
const CARD_X: [(&str, u32); 3] = [("Milanese (vector)", 224), ("Neapolitan", 294), ("Sicilian", 364)];
const CARD_Y: u32 = 308;
const CARD_W: u32 = 52;
const CARD_H: u32 = 84;

/// Boot scopa to its opening menu with its Blorb resolved.
fn scopa_at_the_menu() -> Option<GameSession> {
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
    Some(s)
}

/// Every deck the game offers is on the opening screen, each in its own 52×84 slot.
///
/// Falsified by reverting the win_box fix (`apply_picture_event` reading the
/// window's current box again): "the Neapolitan sample card must be painted where
/// the game laid it out — only 0 of 4368 pixels are".
#[test]
fn all_three_sample_decks_are_painted_at_the_menu() {
    let Some(session) = scopa_at_the_menu() else { return };
    let paint = session
        .paint_surface()
        .expect("scopa paints its menu with erase_window fills and card pictures");
    assert_eq!((paint.width(), paint.height()), (640, 400), "the ground is the v6 screen");

    for (name, x0) in CARD_X {
        let opaque = (CARD_Y..CARD_Y + CARD_H)
            .flat_map(|y| (x0..x0 + CARD_W).map(move |x| (x, y)))
            .filter(|&(x, y)| paint.get_pixel(x, y)[3] == 255)
            .count();
        assert!(
            opaque * 100 >= (CARD_W * CARD_H) as usize * 95,
            "the {name} sample card must be painted where the game laid it out — \
             only {opaque} of {} pixels are",
            CARD_W * CARD_H
        );
    }

    // The two picture decks are photographs of real cards, so each slot carries far
    // more colours than the flat-filled vector deck beside it. Without this the test
    // would still pass if all three slots showed the same drawn Ace of Hearts.
    for (name, x0) in [CARD_X[1], CARD_X[2]] {
        let colours: std::collections::BTreeSet<[u8; 3]> = (CARD_Y..CARD_Y + CARD_H)
            .flat_map(|y| (x0..x0 + CARD_W).map(move |x| (x, y)))
            .map(|(x, y)| {
                let p = paint.get_pixel(x, y);
                [p[0], p[1], p[2]]
            })
            .collect();
        assert!(
            colours.len() > 40,
            "the {name} slot must hold its photographic card, not a redraw of the \
             vector one; got {} distinct colours",
            colours.len()
        );
    }
}

/// …at the size the game laid out for, not twice it.
///
/// `scopa.blb` has no `Reso` chunk, so Blorb §11 makes every image in it
/// non-scalable — one image pixel per screen pixel. Doubling them (the Infocom
/// profile, right for every `Reso`-bearing v6 blorb) told the game its cards were
/// 104×168, so it centred them 70 px apart at a size that overlaps its neighbours
/// and runs 34 px off the bottom of a 400 px screen.
///
/// Falsified by restoring the unconditional `V6_ART_SCALE`: "nothing may be painted
/// in the gap between the Milanese and Neapolitan cards — a doubled card spills into
/// it".
#[test]
fn the_decks_are_drawn_at_their_natural_size() {
    let Some(session) = scopa_at_the_menu() else { return };

    let paint = session.paint_surface().expect("the menu is painted");
    // Between two 52-wide cards centred 70 apart there is an 18 px gap, and nothing
    // above or below the single card row. A doubled card covers all three.
    for (name, x, y) in [
        ("gap between the Milanese and Neapolitan cards", 285u32, 350u32),
        ("gap between the Neapolitan and Sicilian cards", 355, 350),
        ("the row above the cards", 320, 300),
        ("the row below the cards", 320, 396),
    ] {
        assert_eq!(
            paint.get_pixel(x, y)[3],
            0,
            "nothing may be painted in {name} — a doubled card spills into it \
             (at {x},{y})"
        );
    }

    // …and the mechanism behind the layout: what the game itself was told.
    let dims = &session.machine.picture_dims;
    for want in [(101u16, 52u16, 84u16), (301, 52, 84)] {
        assert!(
            dims.contains(&want),
            "picture_data must report the art's own size for a Reso-less Blorb; \
             {want:?} missing from the injected table"
        );
    }
}

/// …and all three reach what the player actually sees, in both colour modes.
///
/// Painted pixels are the game's own drawing, not a palette preference, so declining
/// game colours must not erase a deck.
#[test]
fn the_decks_reach_the_composite_in_both_modes() {
    let Some(session) = scopa_at_the_menu() else { return };
    let Some(paint) = session.paint_surface() else { return };
    let model = session.screen();
    let app::engine::WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);

    // Sample inside each card slot only, so a composite that drops one deck fails.
    let samples: Vec<(u32, u32, image::Rgba<u8>)> = CARD_X
        .iter()
        .flat_map(|&(_, x0)| {
            (CARD_Y + 6..CARD_Y + CARD_H - 6)
                .step_by(5)
                .flat_map(move |y| (x0 + 6..x0 + CARD_W - 6).step_by(5).map(move |x| (x, y)))
        })
        .filter_map(|(x, y)| {
            let p = *paint.get_pixel(x, y);
            (p[3] == 255).then_some((x, y, p))
        })
        .collect();
    assert!(samples.len() > 300, "three card faces give us plenty to check: {}", samples.len());

    for honor in [true, false] {
        let mut state = app::state::AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.honor_game_colours = honor;
        *state.v6_paint.borrow_mut() = Some(paint.clone());

        let (canvas, _metrics) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);

        let hits = samples
            .iter()
            .filter(|&&(x, y, p)| canvas.get_pixel_checked(x, y).is_some_and(|c| *c == p))
            .count();
        assert!(
            hits * 100 >= samples.len() * 95,
            "honor={honor}: every deck the game painted must survive into the \
             composite — only {hits} of {} sampled pixels did",
            samples.len()
        );
    }
}
