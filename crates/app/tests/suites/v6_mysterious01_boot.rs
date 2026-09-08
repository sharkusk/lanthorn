//! SQ-0722 / SQ-0725: mysterious01's boot draws two title cards and lanthorn showed
//! at most one of them, in the wrong place.
//!
//! The game's boot picture sequence, read off `Machine::pending_pictures`, is three
//! events on one turn:
//!
//! ```text
//! pic 0  win 0..7 at (1,1)   erase=true                  <- clear every window
//! pic 33 win 0    at (1,1)   erase=false at_cursor=true
//! pic 34 win 0    at (1,192) erase=false at_cursor=false
//! ```
//!
//! Both cards are 512x192. `Mysterious01.blb` has no `Reso` chunk, so Blorb §11
//! draws its art 1:1 (`art_scale` 1, SQ-0715/SQ-0718) — the two cards stack down
//! the LEFT of the 640x400 unit screen, filling rows 1..384 and leaving the
//! right-hand 128 px bare. That geometry is the whole of both defects.
//!
//! **SQ-0722 — the splash landed in the transcript.** `is_win0_inline_float` reads a
//! window-0 picture as an inline float when the game declares prose intent, and
//! disqualifies one that spans the window (a float flows BESIDE text; art that
//! leaves no column cannot be one). Picture 33 declares intent only by
//! `at_cursor` — `erase_window` had just put the cursor back at (1,1), so the match
//! is pure coincidence — and at 512 of 640 px it does not span the window. So it
//! read as a float and went down the transcript path, where it scrolled away with
//! the prose. It used to span: every v6 picture was doubled by `V6_ART_SCALE` and
//! the card measured 1024, comfortably over. Giving the story its true `art_scale`
//! halved the number and nothing noticed.
//!
//! The fix is not a wider threshold. `at_cursor` is a pixel-exact `y == y_cursor`,
//! and with NOTHING ever streamed to window 0 it compares against a cursor no text
//! ever moved — so it is not evidence of anything. `PictureEvent::out_chars` is
//! that count, already carried on the event. Measured over the corpus, every
//! genuine float is printed into a window that has already streamed text (Zork
//! Zero's drop-cap at 1 char, its room icon at 270, Shogun's ship at 526) and every
//! coincidental backdrop is not (mysterious01 at 0); `margin_after` — the game
//! saying "text flows beside this" in as many words — still counts unconditionally.
//!
//! **SQ-0725 — the second card never appeared at all.** With picture 33 diverted to
//! the transcript, window 0's canvas held picture 34 alone, and hybrid (the shipped
//! default) drew none of it. `blit_story_gfx` is reachable from the RASTER path
//! only: hybrid shows art as a ring of bands AROUND the story viewport, and a story
//! window covering the screen leaves no ring to carve. `picture_takeover` exists to
//! route such a frame to the composite instead, but it asked whether the art FILLED
//! or ENCLOSED the screen — and two cards down the left of the screen do neither.
//! So the card the player could see was the one that had leaked into the transcript,
//! and the card on the canvas was invisible. One geometry, two symptoms.
//!
//! Both are pinned here at BOOT, which is where the report is: the asymmetry the
//! quest opened on ("it shows on restart") is not the differential it looked like —
//! a restart re-runs the same three events — and the frame that works is no place
//! to test the frame that does not.
//!
//! Stories are gitignored (CLAUDE.md), so every case skips cleanly without one.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, TranscriptElem};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// Both cards are 512x192 at y=1 and y=192, one unit pixel per art pixel.
const CARD_H: u32 = 192;
const CARD_W: u32 = 512;

/// SQ-1015: NOT redirected to the tracked fixtures directory, unlike its
/// sibling suites. This test's assertions depend on `Mysterious01.blb`'s
/// actual picture bytes (the two boot title cards), and that blorb's
/// screenshot-sourced graphics have no established licence separate from the
/// text `Mysterious Adventures` permission (see `fixture_paths.rs`'s doc
/// comment) — so only `mysterious01.z6` moved, and `Mysterious01.blb` stays
/// commercial-only in `stories/`. Resolving the story from the tracked copy
/// while its Blorb companion stays put would silently boot it with NO
/// pictures at all (`resolve_resource_blorb` scans the story's own
/// directory), turning this suite's card-geometry assertions into failures
/// instead of the clean skip they are today.
fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// mysterious01 at the boot screen, in hybrid, with game colours honored.
fn boot(honor: bool) -> Option<(GameSession, app::state::AppState)> {
    let path = stories_dir().join("mysterious01.z6");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    // `std_window()` is None — no `Reso` chunk — which is exactly what gives this
    // story an `art_scale` of 1 and makes its cards 512 px rather than 1024.
    assert!(picts.std_window().is_none(), "premise: Mysterious01.blb declares no Reso standard window");
    let mut s = GameSession::new_with_trace(
        bytes, honor, false, None, false, dims, picts.std_window(), None, None,
    )
    .expect("a valid v6 story");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    app::state::apply_transcript_elems(&mut state, &Engine::take_transcript_elems(&mut s));
    *state.v6_paint.borrow_mut() = Engine::paint_surface(&s);
    Some((s, state))
}

/// The rows of window 0's canvas that carry any opaque pixel.
fn opaque_rows(session: &GameSession) -> Vec<u32> {
    let png = session
        .pictures_png()
        .into_iter()
        .find(|(w, _)| *w == 0)
        .map(|(_, p)| p)
        .expect("window 0 takes a canvas for its title cards");
    let img = image::load_from_memory(&png).expect("a decodable canvas").to_rgba8();
    (0..img.height())
        .filter(|&y| (0..img.width()).any(|x| img.get_pixel(x, y)[3] != 0))
        .collect()
}

/// SQ-0722: both title cards are PLACED ART on window 0's canvas, and neither one
/// is an inline transcript float.
///
/// Falsified by restoring `let intent = ev.at_cursor || ev.margin_after.is_some()`
/// in `is_win0_inline_float`: "the FIRST title card (picture 33, unit rows 0..192)
/// must be placed art on window 0's canvas — the canvas carries rows 191..383 only,
/// so the card went down the transcript-float path and scrolls away with the prose
/// (SQ-0722)".
#[test]
fn mysterious01_stacks_both_title_cards_on_window_zero() {
    let Some((session, _state)) = boot(true) else { return };

    // The cards are drawn at 1-based y=1 and y=192, so they OVERLAP by exactly one
    // row: card 1 covers rows 0..=191 and card 2 starts on row 191, card 1's last.
    // Together they run 0..=382, one row short of twice a card.
    const LAST: u32 = 2 * CARD_H - 2;
    let rows = opaque_rows(&session);
    let (first, last) = (rows.first().copied(), rows.last().copied());
    assert_eq!(
        (first, last),
        (Some(0), Some(LAST)),
        "the FIRST title card (picture 33, unit rows 0..{CARD_H}) must be placed art on window 0's \
         canvas — the canvas carries rows {first:?}..{last:?}, so the card went down the \
         transcript-float path and scrolls away with the prose (SQ-0722)"
    );
    assert_eq!(
        rows.len() as u32,
        LAST + 1,
        "the two cards stack with no gap: every row of 0..={LAST} is painted"
    );

    // …and nothing about them reaches the transcript as a float. Tightened by
    // SQ-0895: this used to allow the SQ-0461 `ContentSplash` band (emitted for
    // the frameless mode, ignored by every other), so it could only assert that
    // no image was sourced `Story`. With the mode and the band both gone, the
    // cards anchor no transcript image whatsoever.
    let elems = Engine::take_transcript_elems(&mut { session });
    let images = elems.iter().filter(|e| matches!(e, TranscriptElem::Image(_))).count();
    assert_eq!(
        images, 0,
        "a title card is not an inline float; it anchors no transcript image at all"
    );
}

/// SQ-0725: and hybrid — the shipped default — actually SHOWS them.
///
/// The premise and the assertion are separate on purpose. The premise is that the
/// boot frame has no ring to draw in, which is what makes the composite the only
/// route; the assertion is that art lands in BOTH card bands.
///
/// Falsified by removing the `art_paints_anything` arm from `picture_takeover`:
/// "the boot frame must take the composite: hybrid's ring is empty when the story
/// window covers the screen, so a plate that does not go to raster is never drawn
/// at all (SQ-0725) — this frame took hybrid-ring".
fn mysterious01_boot_shows_both_cards(honor: bool) {
    let Some((session, state)) = boot(honor) else { return };

    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let story = layout.story.expect("mysterious01 publishes a story window");
    assert_eq!(
        (story.x_px, story.y_px, story.w_px, story.h_px),
        (0, 0, 640, 400),
        "premise (honor={honor}): window 0 covers the whole screen, so there is no ring to carve"
    );
    let plate = layout.story_gfx.expect("the title cards are window 0's own plate");
    let WinNode::Graphics(g) = &plate.node else { panic!("story_gfx is a Graphics leaf") };
    assert_eq!(
        g.canvas.dimensions(),
        (640, 400),
        "premise (honor={honor}): the plate is window 0's own pixel box"
    );

    // Premise: the cards reach neither the right edge nor all four edges, so the
    // fill and enclosure arms of `picture_takeover` cannot be what saves them.
    assert!(
        (CARD_W..640).all(|x| (0..400).all(|y| g.canvas.get_pixel(x, y)[3] == 0)),
        "premise (honor={honor}): the right-hand {} px of the screen is bare — the cards neither \
         fill nor enclose it",
        640 - CARD_W
    );

    let area = Rect::new(0, 0, 100, 30);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    let path = state.v6_path_log.borrow().last().map(|(l, _)| l.clone()).unwrap_or_default();
    assert_eq!(
        path, "raster",
        "honor={honor}: the boot frame must take the composite: hybrid's ring is empty when the \
         story window covers the screen, so a plate that does not go to raster is never drawn at \
         all (SQ-0725) — this frame took {path}"
    );

    // …and the composite carries art in BOTH card bands. A card that failed to
    // arrive leaves its band a flat page fill.
    let (canvas, _metrics) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
    for (label, y0, y1) in [("first (picture 33)", 0, CARD_H), ("second (picture 34)", CARD_H, 2 * CARD_H)] {
        let mut colours = std::collections::BTreeSet::new();
        for y in y0..y1 {
            for x in 0..CARD_W {
                let p = canvas.get_pixel(x, y);
                if p[3] == 255 {
                    colours.insert((p[0], p[1], p[2]));
                }
            }
        }
        assert!(
            colours.len() > 3,
            "honor={honor}: the {label} title card must reach the composite; its band (rows \
             {y0}..{y1}) holds only {} distinct colour(s)",
            colours.len()
        );
    }
}

#[test]
fn mysterious01_boot_shows_both_cards_honoring_game_colours() {
    mysterious01_boot_shows_both_cards(true);
}

#[test]
fn mysterious01_boot_shows_both_cards_theme_only() {
    mysterious01_boot_shows_both_cards(false);
}
