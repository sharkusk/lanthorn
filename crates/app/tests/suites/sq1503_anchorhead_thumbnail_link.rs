//! SQ-1503: clicking an illustration THUMBNAIL in Anchorhead: the Illustrated
//! Edition (the commercial `stories/Anchorhead.gblorb`) does not open the
//! full-screen view of the picture, while typing the VIEW command does.
//!
//! # What the game itself says the mechanic is
//!
//! `ILLUSTRATIONS` (in real gameplay) answers: "As you explore the game, you
//! will periodically see a small thumbnail image appear on the right side of
//! the screen, alongside the text. Click on this thumbnail to view the
//! full-size illustration; click again on the illustration or press any key to
//! return to the game." That is Glk's documented "hyperlink on a picture"
//! feature (`glk_set_hyperlink()` immediately before `glk_image_draw()`), the
//! same mechanism linked TEXT uses — not the cell-granular graphics-window
//! click SQ-0563 covers (a real Glk graphics window, mouse-watched directly).
//! `anchorhead_illustrations_help_describes_the_clickable_thumbnail` below pins
//! that this archive really does describe (and therefore rely on) exactly this.
//!
//! # The root cause, traced end to end
//!
//! THREE gaps, all silent (no panic, no wrong answer — just nothing). The
//! first two were fixed, the quest closed, and the user's mouse reopened it on
//! the third — which is the one every real Glulx thumbnail actually goes
//! through:
//!
//! 1. `crates/gvm/src/exec.rs`'s `glk_image_draw`/`_scaled`/`_scaled_ext` opcode
//!    handlers never read the window's current stream's hyperlink value
//!    (`glk_set_hyperlink`) the way text output already does — so gvm handed
//!    every picture to the host with link 0, however the game had drawn it.
//!    Fixed by `Machine::current_hyperlink` and threading it through
//!    `GlkBackend::graphics_draw_image`/`buffer_draw_image_ext` (a new trailing
//!    `link: u32` argument on both); proven directly at the opcode level by
//!    `gvm::exec::tests::glk_set_hyperlink_tags_a_subsequently_drawn_image`.
//! 2. `crates/app/src/render/transcript.rs`'s draw loop hits an inline-image
//!    band row and `continue`s straight past the "record cell→link for this
//!    row" code below it — which would have found nothing to record anyway,
//!    since an image line carries no styled runs at all
//!    (`glk_backend::log_to_lines` starts and ends every image on a fresh,
//!    empty run vec). So even with gap 1 fixed, a click anywhere on a linked
//!    picture found no entry in the per-frame cell→link map `main.rs` hit-tests
//!    against, and silently did nothing. Fixed by recording link cells for the
//!    band's own drawn rect before the `continue`; proven directly by
//!    `render::transcript::tests::render_transcript_builds_cell_link_map_for_an_inline_image_band`.
//! 3. …but a real thumbnail is never a BAND. "Alongside the text" is
//!    `imagealign_MarginRight`, and the main transcript wraps with
//!    `left_float = true`, so `FloatState::start` claims every margin picture
//!    that leaves a usable prose column and emits `WrappedRow`s carrying
//!    `float: Some(strip)` with `band: None`. `try_blit_band_row` declines such
//!    a row — it is a prose row with a picture laid over its margin, not an
//!    image row — so gap 2's fix, which lives inside that arm, never ran for
//!    the picture the player sees. The click still found nothing. Fixed by
//!    `render::transcript::record_band_links`, one function now called from
//!    BOTH arms so the recorded cells cannot depend on which route the picture
//!    took; proven generically by
//!    `render::transcript::tests::render_transcript_builds_cell_link_map_for_a_margin_float_picture`.
//!
//! Typing VIEW worked throughout because it is driven entirely by the game's
//! own logic, never by any of these paths.
//!
//! # What this suite adds on top of those unit tests
//!
//! The unit tests above prove each seam generically. What only a suite against
//! the REAL commercial archive can add is that Anchorhead genuinely relies on
//! this (not a synthetic scenario) and that the FULL click chain — render,
//! `glk_hyperlink_window`'s hit test, `GlulxSession::deliver_hyperlink`, and
//! the game's own answer to it — works at a moment the game really produces.
//!
//! Two cases do that, and the difference between them is the whole of why this
//! quest was reopened:
//!
//! - `a_hyperlinked_anchorhead_picture_resolves_to_a_click` pushes this
//!   archive's own boot artwork as an `InlineUp` picture under a synthetic
//!   link. That is a real chain, but through the BAND route — so it passed
//!   while the reported bug stood.
//! - `the_real_anchorhead_thumbnail_is_a_margin_float_whose_cells_carry_its_link`
//!   restores the user's own host snapshot of the moment a thumbnail is on
//!   screen (`stories/Anchorhead-thumbnail.lanthorn`) and clicks the picture
//!   the game itself drew: `MarginRight`, link 102, on a restored, live,
//!   hyperlink-armed session, at the user's own 123x68 / 8x18 px terminal. The
//!   game answers by opening the full-size illustration. That is the case that
//!   reproduces the report, and it fails with the reported symptom (an EMPTY
//!   cell→link map) the moment gap 3's fix is reverted.
//!
//! Skips vacuously without the gitignored `stories/Anchorhead.gblorb` (and, for
//! the snapshot case, `stories/Anchorhead-thumbnail.lanthorn` beside it).

use app::engine::{Engine, GraphicsWindow, KeyInput, WinNode};
use app::glulx_session::GlulxSession;
use app::inline_image::{ImageAlign, InlineImage};
use app::state::{AppState, Focus, TranscriptKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::fixture_paths::fixture_path;

/// Boot the real, gitignored Anchorhead.gblorb, or `None` when absent.
fn boot_anchorhead() -> Option<GlulxSession> {
    let path = fixture_path("Anchorhead.gblorb");
    let raw = std::fs::read(&path).ok()?;
    let blorb1 = blorb::Blorb::parse(raw.clone()).ok()?;
    let (_, image) = blorb1.executable().ok()?;
    let image = image.to_vec();
    let blorb2 = blorb::Blorb::parse(raw).ok()?;
    GlulxSession::new(image, 100, 60, true, true, false, (8, 16), Some(blorb2), &[]).ok()
}

/// Drive past the opening title/banner sequence into real gameplay ("Outside
/// the Real Estate Office"), the way pressing through the intro at the
/// keyboard would.
fn boot_into_gameplay() -> Option<GlulxSession> {
    let mut sess = boot_anchorhead()?;
    let _ = Engine::take_transcript(&mut sess);
    for _ in 0..3 {
        let _ = Engine::submit_key(&mut sess, KeyInput::Char(' '));
    }
    Some(sess)
}

fn find_graphics(node: &WinNode) -> Option<&GraphicsWindow> {
    match node {
        WinNode::Graphics(g) => Some(g),
        WinNode::Pair { first, second, .. } => find_graphics(first).or_else(|| find_graphics(second)),
        _ => None,
    }
}

#[test]
fn anchorhead_illustrations_help_describes_the_clickable_thumbnail() {
    let Some(mut sess) = boot_into_gameplay() else {
        eprintln!("SKIP: no Anchorhead.gblorb");
        return;
    };
    let r = Engine::submit(&mut sess, "illustrations");
    assert!(
        r.transcript.contains("thumbnail") && r.transcript.contains("Click on this thumbnail"),
        "Anchorhead's own ILLUSTRATIONS text should describe the click-the-thumbnail \
         mechanic this quest is about; got:\n{}",
        r.transcript
    );
}

#[test]
fn anchorhead_keeps_a_standing_hyperlink_watch_on_the_primary_window_during_play() {
    let Some(mut sess) = boot_into_gameplay() else {
        eprintln!("SKIP: no Anchorhead.gblorb");
        return;
    };
    let _ = Engine::submit(&mut sess, "look");
    assert!(
        sess.hyperlink_windows().contains(&1),
        "the primary window should stay armed for a hyperlink click throughout ordinary \
         play — that is what lets a thumbnail's link reach the game the instant it \
         appears, mid-turn, with no separate VIEW-style request; got {:?}",
        sess.hyperlink_windows()
    );
}

/// The BAND route's end-to-end chain: a linked picture that reaches the screen
/// as an `InlineUp` image row, clicked through `glk_hyperlink_window` and
/// `deliver_hyperlink`. Falsified against gap 2's fix (the link-cell recording
/// inside the band arm, ahead of its `continue`).
///
/// **This case is not the reported bug, and reading it as one is what closed
/// SQ-1503 early.** Anchorhead's thumbnail is `MarginRight`, which never enters
/// the band arm at all — see
/// `the_real_anchorhead_thumbnail_is_a_margin_float_whose_cells_carry_its_link`
/// below for the route the player's click actually takes. Kept because the band
/// route is real too: an inline picture, or a margin picture too wide to float,
/// still arrives this way and must stay clickable.
#[test]
fn a_hyperlinked_anchorhead_picture_resolves_to_a_click() {
    let Some(mut sess) = boot_anchorhead() else {
        eprintln!("SKIP: no Anchorhead.gblorb");
        return;
    };

    // Anchorhead's own boot/title artwork — real pixels from THIS archive,
    // captured before the title sequence moves the game past it.
    let model = Engine::screen(&sess);
    let cover = find_graphics(&model.root)
        .unwrap_or_else(|| panic!("Anchorhead should draw a title illustration at boot"))
        .canvas
        .clone();

    // Drive into real gameplay; the cover's own graphics window is gone by
    // then (the title sequence toggles to plain text) — this is only where the
    // pixels were captured, not where they get pushed below.
    let _ = Engine::take_transcript(&mut sess);
    for _ in 0..3 {
        let _ = Engine::submit_key(&mut sess, KeyInput::Char(' '));
    }
    let _ = Engine::submit(&mut sess, "look");

    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.focus = Focus::Game;
    state.push_transcript_kind("Outside the Real Estate Office", TranscriptKind::Story);
    const LINK: u32 = 4242; // an arbitrary nonzero link value, as any real game picks one
    state.push_transcript_image(InlineImage {
        pixels: cover,
        align: ImageAlign::InlineUp,
        scaled: None,
        margin_px: None,
        rule: None,
        link: LINK,
    });
    state.push_transcript("after the thumbnail");

    let area = Rect::new(0, 0, 100, 40);
    let mut buf = Buffer::empty(area);
    let m = app::render::screen::render_story_pane(&Engine::screen(&sess), false, None, &state, area, &mut buf);

    let &((col, row), link) = m
        .links
        .iter()
        .find(|&&(_, v)| v == LINK)
        .unwrap_or_else(|| panic!("the drawn picture's cells must be in the click map; got {:?}", m.links));
    assert_eq!(link, LINK);

    // The SAME two functions `main.rs`'s hyperlink click handler calls, in
    // order: which window owns the click, then deliver the event to it.
    let windows = sess.hyperlink_windows();
    let story = (0u16, 0u16, area.width, area.height);
    let win = app::glulx_session::glk_hyperlink_window(false, col, row, story, &windows, &m.win_rects).unwrap_or_else(|| {
        panic!(
            "the click at ({col},{row}) should resolve to the hyperlink-watching primary \
             window; windows={windows:?} win_rects={:?}",
            m.win_rects
        )
    });
    assert_eq!(win, 1, "the primary window owns the click");

    // Deliver it — same call `main.rs` makes. Anchorhead does not know this
    // synthetic link value, so it is free to ignore it; what matters is that
    // the chain reaches the game at all instead of stopping dead at the
    // click-has-no-recorded-link step this quest is about.
    let _ = sess.deliver_hyperlink(win, link);
}

// ── The reopened bug: the real thumbnail is a MARGIN FLOAT, not a band ───────
//
// SQ-1503 was closed on the case above and the user's mouse reopened it. The
// case above is synthetic in the one way that mattered: it pushes the picture
// as `ImageAlign::InlineUp`, which `wrap_lines_kinded_extend` expands into an
// N-row `WrappedRow::band`. Anchorhead's thumbnail is
// `ImageAlign::MarginRight` — "a small thumbnail image … on the right side of
// the screen, ALONGSIDE the text", exactly as its own ILLUSTRATIONS text says —
// and the main transcript wraps with `left_float = true`, so a margin picture
// takes `FloatState::start`'s path instead: `WrappedRow::float`, with
// `band: None` and the prose narrowed beside it. `try_blit_band_row` declines a
// float row, so the band arm's link recording never runs for the picture the
// player actually sees.
//
// The case below drives the real archive at the real moment — the user's own
// host snapshot, restored — rather than a picture this suite pushed itself.

/// The user's `.lanthorn` host snapshot of the moment a thumbnail is on screen
/// (`stories/Anchorhead-thumbnail.lanthorn`, gitignored beside the story).
/// `None` when absent, so this skips vacuously exactly as the story fixture does.
fn thumbnail_snapshot() -> Option<app::archive::ArchiveContents> {
    app::archive::load_archive(&fixture_path("Anchorhead-thumbnail.lanthorn")).ok()
}

/// Rebuild the snapshot's transcript into an `AppState` the way a restore does:
/// each line in order, an image line pushed as an image unit.
// `from_fontsize` is deprecated in favour of a live stdio query, which a
// headless test has no terminal to make — the same exemption every other
// fixed-cell render harness here takes.
#[allow(deprecated)]
fn state_from(ar: &app::archive::ArchiveContents, cell: (u16, u16)) -> AppState {
    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker =
        Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(cell.0, cell.1)));
    state.focus = Focus::Game;
    for (i, line) in ar.transcript.iter().enumerate() {
        match ar.transcript_images.get(i) {
            Some(Some(img)) => state.push_transcript_image(img.clone()),
            _ => state.push_transcript_kind(
                line,
                ar.transcript_kinds.get(i).copied().unwrap_or(TranscriptKind::Story),
            ),
        }
    }
    state
}

#[test]
fn the_real_anchorhead_thumbnail_is_a_margin_float_whose_cells_carry_its_link() {
    let (Some(ar), Some(mut sess)) = (thumbnail_snapshot(), boot_anchorhead()) else {
        eprintln!("SKIP: no Anchorhead.gblorb and/or stories/Anchorhead-thumbnail.lanthorn");
        return;
    };

    // Non-vacuity, and the whole point of using the real archive: this really is
    // a thumbnail moment, the picture really is a RIGHT-MARGIN float, and gvm
    // really did stamp it with the game's own `glk_set_hyperlink` value (the
    // half of SQ-1503 that was already fixed and is not re-fixed here).
    let (idx, img) = ar
        .transcript_images
        .iter()
        .enumerate()
        .find_map(|(i, o)| o.as_ref().map(|im| (i, im.clone())))
        .expect("the snapshot must hold the thumbnail moment — one inline picture in the transcript");
    assert_eq!(
        img.align,
        ImageAlign::MarginRight,
        "Anchorhead draws its thumbnail alongside the text as a right-margin picture, \
         which is the align the float path (not the band path) serves"
    );
    assert_ne!(img.link, 0, "gvm must stamp the game's glk_set_hyperlink value onto the picture");

    // The live game at that same moment, so the click is delivered to a real
    // suspended `glk_select`, not to a freshly booted one.
    Engine::restore_state(&mut sess, &ar.engine_save()).expect("restore the host snapshot");
    assert!(
        sess.hyperlink_windows().contains(&1),
        "the restored moment must have the primary window armed for a hyperlink click; got {:?}",
        sess.hyperlink_windows()
    );

    // The user's own terminal: 123x68 cells at 8x18 px (their /dump-windows).
    let state = state_from(&ar, (8, 18));
    let area = Rect::new(0, 0, 123, 68);
    let mut buf = Buffer::empty(area);
    let m = app::render::screen::render_story_pane(&Engine::screen(&sess), false, None, &state, area, &mut buf);

    let &((col, row), link) = m.links.iter().find(|&&(_, v)| v == img.link).unwrap_or_else(|| {
        panic!(
            "a click anywhere on the drawn thumbnail (transcript line {idx}, link {}) must find \
             that link in the frame's cell→link map; got {:?}",
            img.link, m.links
        )
    });

    // The two calls `main.rs`'s hyperlink arm makes, in order.
    let windows = sess.hyperlink_windows();
    let win =
        app::glulx_session::glk_hyperlink_window(false, col, row, (0, 0, area.width, area.height), &windows, &m.win_rects)
            .unwrap_or_else(|| {
                panic!("the click at ({col},{row}) must resolve to a hyperlink-watching window; windows={windows:?}")
            });
    assert_eq!(win, 1, "the primary window owns the click");

    // And the game must ACT on it: Anchorhead answers a thumbnail click by
    // opening the full-size illustration in a graphics window. Asserted as a
    // CHANGE the click caused, not as a state that merely holds afterwards —
    // "a graphics window exists" would also be true of a moment that already
    // had one, which is a pass this case must not be able to buy.
    assert!(
        find_graphics(&Engine::screen(&sess).root).is_none(),
        "the restored moment is the thumbnail-in-the-margin one: the full-size view is \
         not open yet, so what the click opens below is the click's own doing"
    );
    let _ = Engine::take_transcript(&mut sess);
    let result = sess.deliver_hyperlink(win, link);
    assert!(
        find_graphics(&Engine::screen(&sess).root).is_some(),
        "clicking the thumbnail must open the full-size illustration the way typing VIEW \
         does; the game produced: {:?}",
        result.transcript
    );
}
