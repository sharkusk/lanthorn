//! SQ-0805: one screen, one story window. sunburst.z6 published TWO primary
//! `Buffer`s at the same 640x400 rect, and `classify_windows` filed the second one
//! under CHROME.
//!
//! ```text
//! [0] cell 0x0   @(0,0)  px 640x400 @(0,0)  Buffer primary=true  px_runs=14
//! [1] cell 80x25 @(0,0)  px 640x400 @(0,0)  Buffer primary=true  px_runs=0
//! ```
//!
//! Entry [0] is the real window 0. Entry [1] is the SYNTHETIC full-screen buffer
//! `GameSession::v6_screen_model` adds for Inform 6's v6 library (SQ-0459), which
//! leaves every window at height 0 so the size-0 skip drops them all and raster mode
//! would otherwise ship a blank screen. That branch's own comment says "when nothing
//! survived" — but its guard tested `max_x == 0 || max_y == 0` over each entry's
//! CELL extent, and a v6 window's cell size is its char grid, which sunburst never
//! sizes because it never resizes window 0 off its boot rect. So window 0 arrived
//! with `w: 0, h: 0` and the guard fired with a real primary Buffer standing right
//! there.
//!
//! The same flag has a SECOND consumer that genuinely wants the zero-extent test:
//! `content_size` falls back to the header's 0x21/0x20 char dims (80x25 here)
//! precisely because the cell extent is 0. So the fix splits them — the synthetic
//! buffer asks whether anything survived at all, `content_size` keeps asking whether
//! the cell extent is degenerate — and this suite pins both halves.
//!
//! It was benign on screen, and by luck rather than design: window 0 carries the
//! whole 640x400 in PIXELS, both the hybrid and raster v6 paths measure the story
//! box in pixels, and the chrome twin drew nothing. A second primary buffer is still
//! not chrome.
//!
//! `stories/` is gitignored (CLAUDE.md), so every test here skips vacuously when the
//! story is absent.


use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::GameSession;

use crate::fixture_paths::fixture_path;


fn open(name: &str) -> Option<GameSession> {
    let path = fixture_path(name);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut s = GameSession::new_with_trace(
        bytes, true, false, None, false, dims, picts.std_window(), None, None,
    )
    .expect("a valid v6 story");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some(s)
}

/// sunburst booted and started: the boot menu is keyed by each item's lowercase
/// initial (SQ-0706), and `s` is "Start game." — the point the quest measured.
fn started() -> Option<GameSession> {
    let mut s = open("sunburst.z6")?;
    let _ = s.take_transcript();
    let _ = s.submit_char(b's');
    Some(s)
}

/// A window's rect as `(x, y, w, h)`, in whichever unit the caller asked for.
type Rect4 = (u16, u16, u16, u16);

/// Every primary `Buffer` in the model, as `(cell rect, pixel rect)`.
fn primaries(session: &GameSession) -> Vec<(Rect4, Rect4)> {
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 story builds a Layered root") };
    items
        .iter()
        .filter(|pw| matches!(&pw.node, WinNode::Buffer(b) if b.primary))
        .map(|pw| ((pw.x, pw.y, pw.w, pw.h), (pw.x_px, pw.y_px, pw.w_px, pw.h_px)))
        .collect()
}

/// The premise the defect rests on: sunburst never resizes window 0 off its boot
/// rect, so the window is published with a 640x400 PIXEL box and a 0x0 CELL box.
/// Without that the guard would never have fired here at all.
#[test]
fn sunburst_publishes_window_zero_with_a_zero_cell_extent() {
    let Some(session) = started() else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 story builds a Layered root") };
    let win0 = items
        .iter()
        .find(|pw| matches!(&pw.node, WinNode::Buffer(b) if b.primary))
        .expect("sunburst publishes a story window");
    assert_eq!(
        (win0.x_px, win0.y_px, win0.w_px, win0.h_px),
        (0, 0, 640, 400),
        "premise: window 0 covers the unit screen in pixels"
    );
    assert_eq!(
        (win0.w, win0.h),
        (0, 0),
        "premise: …and carries a 0x0 char grid, which is what made the degenerate guard fire"
    );
}

/// The report. One screen, one story window.
///
/// Falsified by restoring `if degenerate {` on the synthetic-buffer push in
/// `GameSession::v6_screen_model`:
///
/// ```text
/// sunburst publishes exactly ONE primary Buffer — a second one is the synthetic
/// full-screen buffer the degenerate branch adds for a story where NOTHING
/// survived the size-0 skip, and window 0 is standing right there (SQ-0805); got
/// [((0, 0, 0, 0), (0, 0, 640, 400)), ((0, 0, 80, 25), (0, 0, 640, 400))]
/// ```
#[test]
fn sunburst_publishes_exactly_one_primary_buffer() {
    let Some(session) = started() else { return };
    let found = primaries(&session);
    assert_eq!(
        found.len(),
        1,
        "sunburst publishes exactly ONE primary Buffer — a second one is the synthetic \
         full-screen buffer the degenerate branch adds for a story where NOTHING survived the \
         size-0 skip, and window 0 is standing right there (SQ-0805); got {found:?}"
    );
}

/// …and no primary buffer is filed under chrome. This is the consumer the quest is
/// titled for: `classify_windows` takes the FIRST primary `Buffer` as the story and
/// everything after it as chrome, so a twin lands in the frame furniture.
#[test]
fn classify_windows_files_no_primary_buffer_under_chrome() {
    let Some(session) = started() else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 story builds a Layered root") };
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    assert!(layout.story.is_some(), "the story window is the primary Buffer");
    let stray: Vec<_> = layout
        .chrome
        .iter()
        .filter(|pw| matches!(&pw.node, WinNode::Buffer(b) if b.primary))
        .map(|pw| (pw.x, pw.y, pw.w, pw.h, pw.x_px, pw.y_px, pw.w_px, pw.h_px))
        .collect();
    assert!(
        stray.is_empty(),
        "a second primary Buffer is not chrome (SQ-0805); chrome holds {stray:?}"
    );
}

/// The OTHER consumer of the same flag, which must not move: with the cell extent
/// still degenerate, `content_size` keeps falling back to the header's char dims
/// (0x21 cols / 0x20 rows). Splitting the two consumers is only a fix if this half
/// stays exactly where it was.
#[test]
fn content_size_still_falls_back_to_the_header_char_dims() {
    let Some(session) = started() else { return };
    assert_eq!(
        session.screen().content_size,
        (80, 25),
        "a zero cell extent still reads the header's 0x21/0x20 dims — that fallback is \
         load-bearing and is NOT what the synthetic buffer should have been asking (SQ-0805)"
    );
}

/// The branch the synthetic buffer exists for is still reachable. Its guard now asks
/// whether anything survived the size-0 skip at all, so a story that publishes no
/// window whatsoever still gets its full-screen primary Buffer — without one, raster
/// mode renders a blank screen (SQ-0459).
#[test]
fn a_story_with_no_surviving_window_still_gets_its_synthetic_buffer() {
    // advent.z6 is Inform 6's v6 library, the shape SQ-0459 was opened on.
    let Some(session) = open("advent.z6") else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 story builds a Layered root") };
    let real: Vec<_> = items
        .iter()
        .map(|pw| (pw.w, pw.h, pw.w_px, pw.h_px))
        .collect();
    assert!(
        !primaries(&session).is_empty(),
        "some primary Buffer reaches the model, synthetic or otherwise — entries {real:?}"
    );
    assert_ne!(model.content_size, (0, 0), "…and the v6 model always reports a nonzero content size");
    let _ = Engine::paint_surface(&session);
}
