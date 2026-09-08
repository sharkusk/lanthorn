//! SQ-0704 — a v6 chrome window's UNPAINTED area must resolve to that window's
//! OWN background, not to a backdrop the graphics protocol picks for us.
//!
//! The reported symptom: Zork Zero's room icons render on an opaque BLACK box
//! where other interpreters (and the DOS original) show the banner window's own
//! white page. Measured cause — the icons are pictures 9/10/11/13, 45×40 line
//! art that is ~95 % `alpha == 0`, drawn into window 1, Zork Zero's 640×78
//! banner. The banner ARTWORK (window 7's frame) only reaches native row 67, so
//! the bottom of every icon hangs over rows 68..77, where nothing was painted at
//! all. `build_chrome_canvas` resolves everything against one host
//! `default_fg`/`default_bg` pair and consults a window's own colours only for
//! its text runs (SQ-0519), so those pixels left the compositor transparent —
//! and a transparent chrome pixel becomes whatever the protocol decides
//! (halfblocks' `to_rgb8()` flattens it to black; the reporter saw the same
//! black under kitty). ZMSD §8.8.3.2 is explicit that a Version 6 window has its
//! OWN foreground/background pair, and window 1's is `set_colour(fg=2 black,
//! bg=9 white)` — white.
//!
//! RASTER mode never showed the bug: it ships one image and already flattens its
//! holes onto the story page (`flatten_onto_page`, SQ-0510), and Zork Zero's
//! story page is the same white. HYBRID — the default, and what the reporter
//! runs — must NOT flatten (the ring's clear middle is what lets the terminal
//! transcript show through), so the icons' clear ground travelled all the way to
//! the terminal. `v6_layout::fill_window_pages` closes that gap.
//!
//! Both `honor_game_colours` modes are pinned: `true` (the shipped default and
//! primary baseline) gets the window's page; `false` declines the game's colours
//! and must keep today's behaviour byte for byte.
//!
//! The story asset is gitignored, so every case **skips cleanly** when absent.

use std::path::PathBuf;

use app::engine::{Engine, PositionedWindow, WinNode};
use app::graphics::PictSource;
use app::session::GameSession;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot Zork Zero and play far enough in that the banner carries a live room
/// name, its move/score readouts and the compass/room icons.
fn zork0_in_play(honor_game_colours: bool) -> Option<GameSession> {
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session = GameSession::new_with_trace(
        story_bytes,
        honor_game_colours,
        false,
        None,
        false,
        picture_dims,
        picts.std_window(),
        None,
        None,
    )
    .expect("Zork0 (v6) should load and boot without a ZError");
    assert!(!session.quit && session.machine.fault_trace.is_none(), "Zork0 booted cleanly");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    for _ in 0..6 {
        match session.pending_input() {
            app::session::InputKind::Line => {
                let _ = session.submit("look");
            }
            app::session::InputKind::Char => {
                let _ = session.submit_char(b' ');
            }
            app::session::InputKind::Event => {
                let _ = session.submit("");
            }
        }
        let _ = session.take_transcript();
    }
    Some(session)
}

fn render_state(mode: app::config::V6RenderMode, honor_game_colours: bool) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = mode;
    state.config.honor_game_colours = honor_game_colours;
    state.push_transcript("Banquet Hall");
    state.push_transcript("The hall is filled to capacity.");
    state
}

/// The chrome window carrying an explicit background that is NOT the story box —
/// Zork Zero's 640×78 banner (window 1).
fn banner<'a>(chrome: &[&'a PositionedWindow]) -> Option<&'a PositionedWindow> {
    chrome
        .iter()
        .copied()
        .find(|pw| matches!(&pw.node, WinNode::Grid(g) if g.bg.is_some()) && pw.h_px < 200)
}

#[test]
fn zork0_room_icons_rest_on_the_banner_windows_own_white_page() {
    let Some(session) = zork0_in_play(true) else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 publishes a layered composite") };

    let state = render_state(app::config::V6RenderMode::Hybrid, true);
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let (default_fg, default_bg) = app::render::screen::v6_host_pair(&state);

    let win1 = banner(&layout.chrome).expect("Zork0's banner window publishes its own background");
    assert_eq!((win1.x_px, win1.y_px, win1.w_px, win1.h_px), (0, 0, 640, 78), "Zork Zero's banner window");

    // The composite exactly as the hybrid ring builds it, before the SQ-0704 pass.
    let before = app::render::v6_layout::build_chrome_canvas(
        &layout.chrome,
        native,
        default_fg,
        default_bg,
        &state.colors,
        app::render::v6_layout::TextLayer::All,
        &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT),
    );

    // Precondition (the symptom): inside the banner window, below the frame art,
    // the icons' clear ground is FULLY TRANSPARENT — nothing of ours decided what
    // colour the player sees there.
    let holes: Vec<(u32, u32)> = (68..78)
        .flat_map(|y| (276..321).map(move |x| (x, y)))
        .filter(|&(x, y)| before.get_pixel(x, y).0[3] == 0)
        .collect();
    assert!(
        holes.len() > 300,
        "the icons' clear ground under the banner art is unpainted: only {} of 450 pixels are holes",
        holes.len()
    );

    // The icons' LIT strokes over that same strip — these must survive untouched.
    let strokes: Vec<((u32, u32), [u8; 4])> = (68..78)
        .flat_map(|y| (276..321).map(move |x| (x, y)))
        .filter(|&(x, y)| before.get_pixel(x, y).0[3] != 0)
        .map(|(x, y)| ((x, y), before.get_pixel(x, y).0))
        .collect();
    assert!(!strokes.is_empty(), "the icons paint real ink into the strip below the banner art");

    let mut after = before.clone();
    app::render::v6_layout::fill_window_pages(
        &mut after,
        &layout.chrome,
        layout.story,
        &state.colors,
        app::render::v6_layout::TextLayer::All,
        zvm::screen::V6Cell::DEFAULT,
    );

    // (1) Every hole now reads the BANNER WINDOW's own page: opaque white
    // (ZMSD §8.3.1's true-colour equivalent of Standard 9), never black.
    for &(x, y) in &holes {
        assert_eq!(
            after.get_pixel(x, y).0,
            [255, 255, 255, 255],
            "({x},{y}) behind a room icon must be window 1's white page, not a protocol-chosen backdrop"
        );
    }

    // (2) The icons' own ink is byte-identical — the fix decides what UNPAINTED
    // pixels mean and never repaints art.
    for &((x, y), px) in &strokes {
        assert_eq!(after.get_pixel(x, y).0, px, "icon ink at ({x},{y}) is untouched");
    }

    // (3) The banner ARTWORK above (window 7's frame, opaque to row 67) is
    // untouched too — no white box is painted over it.
    for y in 0..68 {
        for x in 276..321 {
            assert_eq!(
                after.get_pixel(x, y).0,
                before.get_pixel(x, y).0,
                "frame art at ({x},{y}) is untouched"
            );
        }
    }

    // (4) The story box stays CLEAR. Window 7 carries the same white page across
    // the whole 640×400 screen; painting it would fill the hybrid transcript
    // viewport and defeat `story_clear_native`'s clear-interior probe.
    let story = layout.story.expect("Zork0 has a story window");
    let (sx, sy) = (story.x_px as u32 + 40, story.y_px as u32 + 40);
    assert_eq!(after.get_pixel(sx, sy).0[3], 0, "the story box is left transparent for the transcript");
}

#[test]
fn zork0_hybrid_ring_ships_no_black_behind_the_room_icons() {
    let Some(session) = zork0_in_play(true) else { return };
    let model = session.screen();

    // Halfblocks is the honest oracle for "a transparent chrome pixel becomes
    // whatever the protocol decides": `ratatui_image`'s halfblocks encoder calls
    // `to_rgb8()`, so an `alpha == 0` band pixel flattens to pure BLACK — the
    // same black the reporter saw under kitty. Pre-fix this render put 85 pure
    // black cells in the chrome rows above the story viewport (the strip under
    // the banner art, right where the icons hang); the fix leaves none.
    let state = render_state(app::config::V6RenderMode::Hybrid, true);
    let area = Rect::new(0, 0, 120, 40);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    assert_eq!(state.v6_ring_plan.get(), "frame", "Zork0's enclosed frame takes the hybrid ring path");

    let viewport = state.transcript_geom.get().expect("hybrid renders the story as a transcript").area;
    let black = |c: ratatui::style::Color| matches!(c, ratatui::style::Color::Rgb(0, 0, 0));
    let mut offenders = Vec::new();
    for y in 0..viewport.y {
        for x in viewport.x..viewport.right() {
            let cell = buf.cell((x, y)).expect("cell inside the pane");
            // A BACKDROP is what this case is about, and which of a cell's two
            // colours is backdrop depends on what the cell is. In a half-block ART
            // cell (`▀`/`▄`) both are picture samples, so either being black is the
            // flattened-transparency bug. Everywhere else the foreground is INK —
            // since SQ-0944 the ring stamps the banner's labels as glyphs, and Zork
            // Zero's chrome ink is `Standard(2)`, black on purpose — so only the
            // background can be a backdrop there. Checking `fg` unconditionally
            // flagged all 47 cells of "Banquet Hall"/"Flatheadia" as black
            // backdrops; they are black letters on the ribbon.
            let art = matches!(cell.symbol(), "▀" | "▄");
            if black(cell.bg) || (art && black(cell.fg)) {
                offenders.push((x, y));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the banner ring must never ship a black backdrop: {} black cells, first {:?}",
        offenders.len(),
        &offenders[..offenders.len().min(8)]
    );
}

#[test]
fn zork0_declined_game_colours_keep_the_hosts_backdrop() {
    // `honor_game_colours = false`: the game's pair is declined at the engine
    // boundary (the model publishes no window background) AND at the render
    // gate, so the SQ-0704 pass is a no-op and the composite is byte-identical
    // to today's. The host page governs everywhere, as before.
    let Some(session) = zork0_in_play(false) else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 publishes a layered composite") };

    let state = render_state(app::config::V6RenderMode::Hybrid, false);
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let (default_fg, default_bg) = app::render::screen::v6_host_pair(&state);

    assert!(
        banner(&layout.chrome).is_none(),
        "with colours declined no chrome window claims a background of its own"
    );

    let before = app::render::v6_layout::build_chrome_canvas(
        &layout.chrome,
        native,
        default_fg,
        default_bg,
        &state.colors,
        app::render::v6_layout::TextLayer::All,
        &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT),
    );
    let mut after = before.clone();
    app::render::v6_layout::fill_window_pages(
        &mut after,
        &layout.chrome,
        layout.story,
        &state.colors,
        app::render::v6_layout::TextLayer::All,
        zvm::screen::V6Cell::DEFAULT,
    );
    assert_eq!(before.as_raw(), after.as_raw(), "declined colours leave the composite byte-identical");

    // And the icons' ground is still the transparency the caller's page resolves.
    assert_eq!(after.get_pixel(300, 70).0[3], 0, "the clear ground stays the caller's to colour in");
}

/// SQ-0704 follow-up — the mechanism that resolves the icons' clear ground,
/// pinned directly.
///
/// Reported after SQ-0704 shipped: the icons no longer render black, but were
/// said to show the *terminal* background rather than the banner window's white
/// page. The theory offered for it — that the icons hang off the window's
/// `Graphics` entry, which carries no colour pair, so `fill_window_pages` skips
/// them — is not the operative mechanism. `classify_windows` lists Zork Zero's
/// banner TWICE: once as `Graphics` (the canvas the icons live in) and once as
/// `Grid` at the IDENTICAL `(0,0) 640x78` rect, and it is the Grid entry that
/// carries the explicit `Standard(9)` white. Both are chrome, so
/// `fill_window_pages` iterates both and the Grid arm resolves the whole rect.
///
/// Probed after the fill rather than before it (the distinction that makes the
/// question answerable at all): **0 of 450** pixels in the icon strip are still
/// clear. Nothing is left for a protocol to colour in, so no `Graphics`-side
/// change is called for.
///
/// The symptom DID recur, and this case correctly said it was not here: it was in
/// the hybrid ring, which never paints the story box at all — see
/// `zork0_hybrid_ring_ships_the_story_page_under_the_banner` below.
#[test]
fn zork0_icon_strip_is_fully_resolved_by_the_grid_entry_at_the_same_rect() {
    let Some(session) = zork0_in_play(true) else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 publishes a layered composite") };
    let state = render_state(app::config::V6RenderMode::Hybrid, true);
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let (default_fg, default_bg) = app::render::screen::v6_host_pair(&state);

    // The banner really is listed twice — a Graphics entry with no colour of its
    // own, and a Grid entry at the identical rect that carries the white page.
    let at_banner_rect =
        |pw: &&PositionedWindow| (pw.x_px, pw.y_px, pw.w_px, pw.h_px) == (0, 0, 640, 78);
    assert!(
        layout.chrome.iter().any(|pw| at_banner_rect(pw) && matches!(&pw.node, WinNode::Graphics(_))),
        "the icons' canvas is a Graphics entry at the banner rect"
    );
    assert!(
        layout
            .chrome
            .iter()
            .any(|pw| at_banner_rect(pw) && matches!(&pw.node, WinNode::Grid(g) if g.bg.is_some())),
        "a Grid entry at the SAME rect carries the banner's own background — that is what \
         `fill_window_pages` resolves the icons' clear ground with"
    );

    let mut canvas = app::render::v6_layout::build_chrome_canvas(
        &layout.chrome,
        native,
        default_fg,
        default_bg,
        &state.colors,
        app::render::v6_layout::TextLayer::All,
        &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT),
    );
    app::render::v6_layout::fill_window_pages(
        &mut canvas,
        &layout.chrome,
        layout.story,
        &state.colors,
        app::render::v6_layout::TextLayer::All,
        zvm::screen::V6Cell::DEFAULT,
    );
    let clear: Vec<(u32, u32)> = (68..78)
        .flat_map(|y| (276..321).map(move |x| (x, y)))
        .filter(|&(x, y)| canvas.get_pixel(x, y).0[3] == 0)
        .collect();
    assert!(
        clear.is_empty(),
        "after the fill nothing in the icon strip may be left unpainted: {} of 450 still clear",
        clear.len()
    );
}

/// SQ-0704, HYBRID half — the ring's bands must ship the story window's page, not
/// leave it for the terminal.
///
/// Reported after both earlier passes: "in v6-render mode the background is
/// correct white; in hybrid the background hasn't been fixed." That split is the
/// whole diagnosis. RASTER flattens its finished canvas onto the page before
/// shipping, so every pixel it sends is opaque. HYBRID draws the story as terminal
/// text and ships only the ring bands as images — and those bands OVERLAP the
/// story box: the sliver between the banner's bottom edge (native y=78) and the
/// first viewport row, plus the flanks. `fill_window_pages` deliberately skips any
/// window overlapping the story box, so nothing ever painted that sliver, and a
/// band pixel left transparent is resolved by the TERMINAL — which is exactly what
/// "the icons sit on the terminal background" looks like.
///
/// The oracle is POSITIVE — the sliver must carry the game's own white — because
/// the sibling case above only forbids BLACK, and that is precisely why this
/// survived it: pre-fix at 120x40 and larger those cells are neither white nor
/// black but `Reset`, a cell with no background at all. Sizes are swept because
/// the sliver is a cell-quantization artifact: it does not exist at every pane
/// size (at 80x24 and 90x30 the banner art consumes it), and a single-size test
/// would have been a coin flip.
///
/// Falsified by dropping `fill_story_page_clear`: white goes to 0 of 71 at
/// 100x34 (71 pure BLACK, the halfblocks flattening of transparent), and 0 of 85,
/// 0 of 101, 0 of 115 at the larger sizes (`Reset` — the reported symptom).
#[test]
fn zork0_hybrid_ring_ships_the_story_page_under_the_banner() {
    let Some(session) = zork0_in_play(true) else { return };
    let model = session.screen();
    let state = render_state(app::config::V6RenderMode::Hybrid, true);

    let mut checked = 0;
    for (w, h) in [(100u16, 34u16), (120, 40), (140, 45), (160, 50)] {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
        let vp = state.transcript_geom.get().expect("hybrid publishes a transcript viewport").area;
        assert!(vp.y > 0, "the banner ring reserves rows above the story viewport at {w}x{h}");

        // The band row directly above the viewport is the sliver in question.
        let row = vp.y - 1;
        let width = vp.width as usize;
        let page = (vp.x..vp.right())
            .filter(|&x| {
                matches!(
                    buf.cell((x, row)).expect("cell inside the pane").style().bg,
                    Some(ratatui::style::Color::Rgb(255, 255, 255))
                )
            })
            .count();
        assert!(
            page * 100 >= width * 95,
            "at {w}x{h} the ring row under the banner must carry the story window's own white page, \
             not the terminal's: {page} of {width} cells"
        );
        checked += 1;
    }
    assert_eq!(checked, 4, "every swept pane size was actually asserted");
}
