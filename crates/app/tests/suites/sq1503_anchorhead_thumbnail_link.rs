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
//! Two gaps, both silent (no panic, no wrong answer — just nothing):
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
//!
//! Typing VIEW worked throughout because it is driven entirely by the game's
//! own logic, never by either of these paths.
//!
//! # What this suite adds on top of those two
//!
//! The two unit tests above already prove the mechanism, generically, at each
//! seam. What only a suite against the REAL commercial archive can add is that
//! Anchorhead genuinely relies on it (not a synthetic scenario) and that the
//! FULL click chain — render, `glk_hyperlink_window`'s hit test, then
//! `GlulxSession::deliver_hyperlink` — resolves to the right window using this
//! archive's own boot artwork. Anchorhead's early puzzles gate every
//! *in-story* thumbnail behind solving them (there is no cheap turn count that
//! reaches one), so `a_hyperlinked_anchorhead_picture_resolves_to_a_click`
//! below threads the one picture reachable with zero puzzle-solving — this
//! archive's own boot/title illustration, real pixels captured off the booted
//! session — through `AppState::push_transcript_image` under a hyperlink,
//! exactly the shape `glk_backend::graphics_draw_image`'s buffer-window arm now
//! produces for any picture the game itself draws under `glk_set_hyperlink`.
//!
//! Skips vacuously without the gitignored `stories/Anchorhead.gblorb`.

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

/// SQ-1503's own falsifying reproduction. Falsified as instructed: reverting
/// the render-loop fix in `render::transcript`'s `render_middle` (the link-cell
/// recording for an image band row, ahead of its `continue`) reproduces the
/// reported symptom exactly — `glk_hyperlink_window` finds no window no matter
/// where on the picture the click lands, so the click silently does nothing.
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
