//! SQ-0714: a full-screen picture drawn into window 0 is a BACKDROP, not a float.
//!
//! SQ-0695 taught the engine to mark a window-0 picture drawn on the window's
//! current text line (`PictureEvent::at_cursor`), and the host routes those down
//! the inline-transcript-float path: Zork Zero's drop-caps and room icons flow
//! beside the paragraph they belong to, Shogun's opening ship reserves a right
//! margin and the prose runs down the left column.
//!
//! `erase_window` puts window 0's cursor back at (1,1), so a game that clears the
//! screen and then paints a backdrop at (1,1) satisfies `at_cursor` by pure
//! coincidence. fmvpoker draws its 640x400 poker table exactly that way, Journey
//! its title illustration, mysterious01 its title card. All three were read as
//! floats, so `pictures_canvas` stayed empty, the model published no `Graphics`
//! leaf for window 0, and the art only ever reached the screen as an image in the
//! text flow — "some graphics but the outline is missing" in the field report.
//!
//! The discriminator this pins is what a float actually IS: **a float flows
//! beside text**. A picture spanning window 0's full width leaves no column for
//! text, so it cannot be one. Measured, in unit pixels:
//!
//! | game         | picture      | draw x | picture w | window w |
//! |--------------|--------------|--------|-----------|----------|
//! | Zork Zero    | 2 (drop-cap) | 1      | 84        | 468      |
//! | Shogun       | 7 (ship)     | 229    | 320       | 548      |
//! | fmvpoker     | 99 (table)   | 1      | 640       | 640      |
//!
//! Both halves are asserted here: fmvpoker's backdrop must reach a canvas AND the
//! composite, and Zork Zero's drop-cap and Shogun's ship must stay in the
//! transcript. Stories are gitignored (CLAUDE.md), so each case skips cleanly.
//!
//! Both `honor_game_colours` modes are pinned for the render assertions: the
//! backdrop is the game's own art, not a palette preference, so declining game
//! colours must not lose it.


use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind, TranscriptElem};

use crate::fixture_paths::fixture_path;


fn boot(name: &str) -> Option<GameSession> {
    let path = fixture_path(name);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut s = GameSession::new_with_trace(
        bytes,
        true,
        false,
        None,
        false,
        dims,
        picts.std_window(),
        None,
        None,
    )
    .expect("a valid v6 story");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    Some(s)
}

/// Window 0's `Graphics` leaf in the published model, if it has one.
fn win0_graphics(session: &GameSession) -> Option<(u32, u32)> {
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { return None };
    items.iter().find_map(|pw| match &pw.node {
        WinNode::Graphics(g) if g.win == 0 => Some((g.canvas.width(), g.canvas.height())),
        _ => None,
    })
}

/// fmvpoker's backdrop reaches a window canvas and the published model.
///
/// Falsified by restoring the bare `ev.window == 0 && ev.at_cursor` test:
/// "fmvpoker's 640x400 backdrop must land on window 0's canvas, not float in the
/// transcript — pictures_canvas holds []".
#[test]
fn fmvpoker_backdrop_lands_on_window_zero() {
    let Some(mut session) = boot("fmvpoker.z6") else { return };
    // One keypress past the "press any key" splash: the game erases every window,
    // then draws picture 99 (640x400) into window 0 at (1,1).
    let result = match session.pending_input() {
        InputKind::Char => session.submit_char(b' '),
        _ => session.submit(""),
    };

    let canvases: Vec<u8> = session.pictures_png().into_iter().map(|(w, _)| w).collect();
    assert!(
        canvases.contains(&0),
        "fmvpoker's 640x400 backdrop must land on window 0's canvas, not float in the \
         transcript — pictures_canvas holds {canvases:?}"
    );

    let (cw, ch) = win0_graphics(&session)
        .expect("window 0 with a canvas publishes a Graphics leaf the renderer can composite");
    assert_eq!(
        (cw, ch),
        (640, 400),
        "the canvas is window 0's own pixel box, so the backdrop is not clipped"
    );

    // …and it is not a transcript float at all. This used to assert only that no
    // image was sourced `Story`, because the backdrop DID anchor one band — the
    // SQ-0461 `ContentSplash` entry that existed so frameless mode, which drops
    // graphics windows, could still show it. SQ-0895 removed that mode and the
    // band with it, so the assertion tightens: the backdrop anchors NOTHING here,
    // and every mode reads it off the window canvas asserted above.
    let images = result
        .transcript_elems
        .iter()
        .filter(|e| matches!(e, TranscriptElem::Image(_)))
        .count();
    assert_eq!(
        images, 0,
        "a backdrop is not an inline float; it anchors no transcript image at all"
    );
}

/// …and those pixels survive into what the player actually sees, in both v6
/// render modes and both colour modes.
#[test]
fn fmvpoker_backdrop_reaches_the_composite() {
    let Some(mut session) = boot("fmvpoker.z6") else { return };
    match session.pending_input() {
        InputKind::Char => session.submit_char(b' '),
        _ => session.submit(""),
    };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    assert!(
        layout.story_gfx.is_some(),
        "window 0's backdrop is story graphics — classify_windows reported story_gfx = None"
    );

    for honor in [true, false] {
        let mut state = app::state::AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.honor_game_colours = honor;

        let (canvas, _metrics) =
            app::render::screen::build_v6_raster_canvas(&layout, native, &state);

        // The backdrop is a drawn frame, not a flat fill: count distinct opaque
        // colours over the whole composite. An empty story rect gives one.
        let mut colours = std::collections::BTreeSet::new();
        for (_, _, p) in canvas.enumerate_pixels() {
            if p[3] == 255 {
                colours.insert((p[0], p[1], p[2]));
            }
        }
        assert!(
            colours.len() > 3,
            "honor={honor}: the backdrop must reach the composite; only {} distinct colour(s) \
             were painted",
            colours.len()
        );
    }
}

/// Zork Zero's drop-cap is still a float: no window-0 canvas, and the picture
/// arrives as a transcript image anchored to its paragraph.
#[test]
fn zork0_drop_cap_stays_an_inline_float() {
    let Some(session) = boot("zork0-r393-s890714.z6") else { return };
    // The drop-cap (picture 2, 84x70 unit px in a 468-wide window 0) and the room
    // icon (216, 42x42) are both drawn during boot.
    let canvases: Vec<u8> = session.pictures_png().into_iter().map(|(w, _)| w).collect();
    assert!(
        !canvases.contains(&0),
        "Zork Zero's drop-cap must stay an inline float — window 0 gained a canvas: {canvases:?}"
    );
}

/// Shogun's opening ship is the widest float in the corpus (320 px of a 548 px
/// window) and must keep flowing beside the prose in its left column.
#[test]
fn shogun_ship_stays_an_inline_float() {
    let Some(mut session) = boot("shogun-r322-s890706.z6") else { return };
    let mut saw_float = false;
    for turn in 0..6 {
        let result = match session.pending_input() {
            InputKind::Char => session.submit_char(13),
            InputKind::Line => session.submit("wait"),
            InputKind::Event => session.submit(""),
        };
        if result.transcript_elems.iter().any(|e| matches!(e, TranscriptElem::Image(_))) {
            saw_float = true;
            let canvases: Vec<u8> = session.pictures_png().into_iter().map(|(w, _)| w).collect();
            assert!(
                !canvases.contains(&0),
                "turn {turn}: Shogun's ship floats beside the prose; window 0 must not take a \
                 canvas for it: {canvases:?}"
            );
            break;
        }
    }
    assert!(saw_float, "Shogun never floated its opening ship into the transcript");
}
