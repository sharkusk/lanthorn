//! advent.blb's clickable graphical toolbar — a detailed graphics window at the
//! top of the screen, whose buttons the game hit-tests itself in canvas pixels.
//!
//! - SQ-0520: the toolbar must reach the image protocol. At common pane widths it
//!   lands 2 cells tall, and the thin-strip rule heuristic (SQ-0332) used to claim
//!   it and shred it into colour-averaged ─ glyphs instead of drawing the image.
//! - SQ-0562: its noun-taking verb buttons prime the input line via Glk's
//!   pre-filled line input rather than running a command.
//! - SQ-0563: the compass rose's W/E buttons are unreachable at cell granularity.
//!
//! Boots the real gitignored story; skips cleanly when absent.

use app::engine::{Engine, WinNode};
use app::glulx_session::GlulxSession;

use crate::fixture_paths::fixture_path;

/// Boot the real gitignored story at the user-report geometry: a 138×51 pane at
/// 8×18 char cells → the toolbar window comes out 138×2 cells with a fully
/// painted 1104×36 canvas. `None` when the story is absent.
fn boot_advent() -> Option<GlulxSession> {
    let path = fixture_path("advent.blb");
    let raw = std::fs::read(&path).ok()?;
    let blorb = blorb::Blorb::parse(raw.clone()).expect("valid blorb");
    let bytes = match app::hints::extract_story(raw).expect("extract") {
        app::hints::LoadedStory::Glulx(b) => b,
        _ => panic!("expected Glulx"),
    };
    Some(
        GlulxSession::new(bytes, 138, 51, true, true, false, (8, 18), Some(blorb), &[])
            .expect("session"),
    )
}

#[test]
fn advent_toolbar_reaches_the_image_protocol() {
    let Some(mut sess) = boot_advent() else {
        eprintln!("SKIP: no advent.blb");
        return;
    };
    let _ = sess.take_transcript();

    fn find_graphics(node: &WinNode) -> Option<&app::engine::GraphicsWindow> {
        match node {
            WinNode::Graphics(gw) => Some(gw),
            WinNode::Pair { first, second, .. } => {
                find_graphics(first).or_else(|| find_graphics(second))
            }
            _ => None,
        }
    }
    let model = sess.screen();
    let gw = find_graphics(&model.root).expect("advent opens a graphics toolbar window");

    // The game painted the whole toolbar: every canvas pixel opaque.
    assert!(gw.canvas.pixels().all(|p| p.0[3] != 0), "toolbar canvas fully painted");

    // The 2-cell-tall toolbar must NOT be claimed by the thin-rule cells path —
    // it falls through to the image protocol (SQ-0520).
    let area = ratatui::layout::Rect::new(0, 0, 138, 2);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    assert!(
        !app::render::graphics::render_graphics_as_cells(gw, area, &mut buf, false),
        "detailed toolbar must not be averaged into rule glyphs"
    );
}

/// SQ-0562 regression: the toolbar's noun-taking verbs (Examine, Take, Drop, Open,
/// Close, Read) don't run a command — they re-request line input with the verb
/// ALREADY in the game's buffer (Glk §4.2 `initlen`). The app must take that
/// prefill and start its input line with it; ignoring it left the prompt empty, so
/// Enter submitted a blank line and the game answered with a bare newline.
///
/// The buttons live at fixed canvas pixels (from the boot draw trace) and the press
/// is animated: the click arms a 50ms timer and the command only lands a few ticks
/// later, so drive the timer until the request appears.
#[test]
fn advent_toolbar_verb_buttons_prefill_the_input_line() {
    if boot_advent().is_none() {
        eprintln!("SKIP: no advent.blb");
        return;
    }
    // (button canvas pixel, the verb it primes) — six buttons, 32px apart. A fresh
    // boot per button so one press can't colour the next.
    for (px, want) in [
        (295u32, "Examine "),
        (327, "Take "),
        (359, "Drop "),
        (391, "Open "),
        (423, "Close "),
        (455, "Read "),
    ] {
        let mut sess = boot_advent().expect("story present");
        let _ = sess.take_transcript();
        let _ = sess.take_line_seed();
        let windows = sess.mouse_windows();
        let win = *windows.first().expect("the toolbar watches for clicks");
        sess.deliver_mouse(win, px, 8);
        let mut got = None;
        for _ in 0..8 {
            sess.deliver_timer();
            if let Some(p) = sess.take_line_seed() {
                got = Some(p);
                break;
            }
        }
        assert_eq!(got.as_deref(), Some(want), "button at canvas x={px} primes {want:?}");
    }
}

/// SQ-0565 regression: the toolbar cancels line input on every button press and
/// PRESERVES whatever partial input it finds in the buffer. The app used to write
/// that buffer only at submit time, so it still held the previous prefill — and
/// every later verb button re-inserted the FIRST verb, text the player may have
/// already deleted. ("Click Examine, delete the word, click Take → Examine comes
/// back.") Mirroring the input line into the buffer each pass fixes it.
///
/// Also covers the flip side the same rule delivers: with a noun already typed, a
/// verb button runs the whole command itself and asks for a fresh empty line, which
/// must not leave the noun stranded at the prompt.
#[test]
fn toolbar_verbs_follow_the_edited_input_line_not_a_stale_buffer() {
    let Some(mut sess) = boot_advent() else {
        eprintln!("SKIP: no advent.blb");
        return;
    };
    let _ = sess.take_transcript();
    let _ = sess.take_line_seed();

    /// What the app's run loop does each pass: adopt any new request's seed as the
    /// input line, then mirror the (possibly player-edited) line back. Returns the
    /// line the player would now see, plus anything the game printed.
    fn settle(sess: &mut GlulxSession, input: &mut String) -> String {
        let mut printed = String::new();
        for _ in 0..8 {
            printed.push_str(&sess.deliver_timer().transcript);
            if let Some(seed) = sess.take_line_seed() {
                *input = seed;
                sess.sync_line_input(input);
                break;
            }
            sess.sync_line_input(input);
        }
        printed
    }

    fn click(sess: &mut GlulxSession, px: u32) {
        let windows = sess.mouse_windows();
        let win = *windows.first().expect("the toolbar watches for clicks");
        sess.deliver_mouse(win, px, 8);
    }

    const EXAMINE: u32 = 295;
    let mut input = String::new();

    // Prime Examine, then erase it exactly as the player did.
    click(&mut sess, EXAMINE);
    settle(&mut sess, &mut input);
    assert_eq!(input, "Examine ", "the button primes its verb");
    input.clear();
    sess.sync_line_input(&input);

    // Every other verb button must now prime ITS OWN verb.
    for (px, want) in [(327u32, "Take "), (359, "Drop "), (391, "Open "), (423, "Close "), (455, "Read ")] {
        click(&mut sess, px);
        settle(&mut sess, &mut input);
        assert_eq!(input, want, "after erasing, the button at x={px} primes {want:?}");
        input.clear();
        sess.sync_line_input(&input);
    }

    // With a noun typed, the verb button runs the command and leaves a clean prompt.
    input.push_str("lamp");
    sess.sync_line_input(&input);
    click(&mut sess, EXAMINE);
    let printed = settle(&mut sess, &mut input);
    assert!(printed.contains("Examine lamp"), "the click ran the whole command: {printed:?}");
    assert_eq!(input, "", "and the noun is not left stranded at the prompt");
}

/// SQ-0564: the premise behind caching kitty uploads by canvas CONTENT. Pressing a
/// toolbar button makes advent repaint the whole bar (pressed), and releasing it
/// repaints the whole bar again — and that release is pixel-for-pixel the resting
/// bar it started from, even though the canvas version has moved on twice. So the
/// release costs a re-place, not a second 155 KiB upload.
#[test]
fn advent_toolbar_returns_to_a_pixel_identical_canvas_after_a_press() {
    let Some(mut sess) = boot_advent() else {
        eprintln!("SKIP: no advent.blb");
        return;
    };
    let _ = sess.take_transcript();

    fn toolbar(sess: &mut GlulxSession) -> (Vec<u8>, u64) {
        fn find(node: &WinNode) -> Option<&app::engine::GraphicsWindow> {
            match node {
                WinNode::Graphics(gw) => Some(gw),
                WinNode::Pair { first, second, .. } => find(first).or_else(|| find(second)),
                _ => None,
            }
        }
        let model = sess.screen();
        let gw = find(&model.root).expect("toolbar window");
        (gw.canvas.as_raw().clone(), gw.version)
    }

    let (resting, v_resting) = toolbar(&mut sess);
    let windows = sess.mouse_windows();
    let win = *windows.first().expect("the toolbar watches for clicks");

    // Press "North" (canvas pixel 28,6) — a different picture, same size.
    sess.deliver_mouse(win, 28, 6);
    let (pressed, v_pressed) = toolbar(&mut sess);
    assert_ne!(pressed, resting, "the pressed bar is a genuinely different picture");
    assert!(v_pressed > v_resting, "and the repaint bumped the canvas version");

    // Release: advent holds the press for a few 50ms ticks, then repaints.
    let mut released = None;
    for _ in 0..8 {
        sess.deliver_timer();
        let (pixels, version) = toolbar(&mut sess);
        if version > v_pressed && pixels != pressed {
            released = Some((pixels, version));
            break;
        }
    }
    let (released, v_released) = released.expect("the button releases within a few ticks");
    assert!(v_released > v_pressed, "the release is a fresh repaint, not the same version");
    assert_eq!(released, resting, "yet its pixels are the resting bar's, byte for byte");
}

/// SQ-0563: the compass rose's W and E buttons sit in a canvas band that
/// cell-granular clicks cannot reach. The toolbar is two 18px cell rows, so a
/// cell-CENTRE click only ever names canvas y 9 or 27, while W/E occupy y 12..24 —
/// the buttons were unreachable by construction, however carefully you aimed.
/// Under pixel mouse reporting the click's offset within its cell is known, and
/// the same physical click reaches them.
///
/// Drives the real game both ways and compares what it actually does: the moves
/// the buttons produce ("West"/"East") only appear with the offset supplied.
#[test]
fn pixel_offsets_make_the_compass_w_e_buttons_clickable() {
    if boot_advent().is_none() {
        eprintln!("SKIP: no advent.blb");
        return;
    }
    // Canvas positions from the boot draw trace: W is image 9 at (14, 12) and E is
    // image 10 at (36, 12), each ~12px square. Aim at the middle of each.
    for (label, canvas_x, canvas_y) in [("West", 20u16, 17u16), ("East", 42, 17)] {
        // The same physical click, expressed as the terminal would report it:
        // a cell plus the offset inside that cell.
        let (cell_col, cell_row) = (canvas_x / 8, canvas_y / 18);
        let sub = (canvas_x % 8, canvas_y % 18);
        assert_eq!(cell_row, 0, "{label} lies in the toolbar's FIRST cell row");

        let mut moves = Vec::new();
        for sub_px in [None, Some(sub)] {
            let mut sess = boot_advent().expect("story present");
            let _ = sess.take_transcript();
            let windows = sess.mouse_windows();
            let story = (0u16, 0u16, 138u16, 51u16);
            // SQ-1203: the hit-test is against the rendered DRAWN rect, not gvm's
            // own layout rect, so render a frame the same way the app does and
            // hand glk_mouse_target the recorded win_rects.
            let state = app::state::AppState::default();
            let model = sess.screen();
            let area = ratatui::layout::Rect::new(0, 0, 138, 51);
            let mut buf = ratatui::buffer::Buffer::empty(area);
            let m = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
            let target = app::glulx_session::glk_mouse_target(
                false,
                cell_col,
                cell_row,
                story,
                &windows,
                &m.win_rects,
                sess.char_pixels(),
                sub_px,
            );
            let (win, vx, vy) = target.expect("the click lands in the toolbar window");
            sess.deliver_mouse(win, vx, vy);
            // The press is animated: the command lands a few 50ms ticks later, and
            // each turn's output arrives on that turn's result.
            let mut echoed = String::new();
            for _ in 0..8 {
                echoed.push_str(&sess.deliver_timer().transcript);
                if !echoed.trim().is_empty() {
                    break;
                }
            }
            moves.push(echoed);
        }
        let (without, with) = (&moves[0], &moves[1]);
        assert!(
            !without.contains(label),
            "cell-centre reporting cannot reach {label} (it hit: {without:?})"
        );
        assert!(
            with.contains(label),
            "the pixel offset reaches {label} (got: {with:?})"
        );
    }
}
