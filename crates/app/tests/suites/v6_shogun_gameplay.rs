//! Shogun (v6) gameplay smoke (SQ-0456).
//!
//! Two regressions covered:
//! 1. Custom-alphabet ZSCII leak: Shogun's alphabet table holds ZSCII 11 (the
//!    sentence gap); decoding it as a raw char put `\u{b}` in every prose
//!    string, panicking ratatui's `cell_width` debug assert in live play.
//! 2. Zero-width input: the game sizes its READ buffer from the current
//!    window's font width (window prop 13). Uninitialized font props made
//!    that 0, so every typed command arrived empty ("[I beg your pardon?]").
//!
//! Skip-if-missing pattern per the other gitignored-story smokes.
//!
//! **Colour mode: `honor_game_colours = true`** — the app's shipped config
//! default, so these render assertions are made in the mode real players run.
//! (Before SQ-0532 wave 4 every v6 smoke booted with the game's colours
//! DECLINED, which is exactly why three colour-driven render regressions
//! shipped unseen. The theme-only `false` path is covered by the paired cases
//! in `v6_game_colour_regression.rs`.)
//!
//! **Palette: `Standard`, set rather than assumed** (SQ-0958) — see
//! [`standard_palette`] for what reading an inherited one cost here.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// All control chars (except newline) in `s` — must always be empty for
/// anything the renderer will touch.
fn ctrl_chars(s: &str) -> Vec<char> {
    s.chars().filter(|c| c.is_control() && *c != '\n').collect()
}

#[test]
fn shogun_boots_plays_and_emits_no_control_chars() {
    let story_path = stories_dir().join("shogun-r322-s890706.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Shogun (v6) should load and boot without a ZError");
    assert!(!session.quit, "Shogun quit during boot");
    assert!(session.machine.fault_trace.is_none(), "Shogun faulted during boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();

    // Select START on the boot menu (Enter), then drive the opening: the intro
    // prose must arrive clean (regression 1) and typed commands must actually
    // reach the parser (regression 2) — "look" re-describes the Bridge and an
    // unknown word gets Shogun's word error, never the empty-input
    // "[I beg your pardon?]".
    let mut saw_bridge_look = false;
    let mut saw_unknown_word = false;
    for turn in 0..8 {
        let result = match session.pending_input() {
            InputKind::Line => session.submit(if turn % 2 == 0 { "look" } else { "xyzzy" }),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        assert!(!result.quit, "Shogun quit on turn {turn}");
        assert!(result.fault.is_none(), "Shogun faulted on turn {turn}: {:?}", result.fault);
        assert_eq!(
            ctrl_chars(&result.transcript),
            Vec::<char>::new(),
            "turn {turn}: control chars leaked into the transcript (custom-alphabet ZSCII bug): {:?}",
            result.transcript
        );
        assert!(
            !result.transcript.contains("I beg your pardon"),
            "turn {turn}: typed command arrived empty (v6 window font-prop init bug): {:?}",
            result.transcript
        );
        // ZMSD §3.8's v6 SPACING codes — tab (9), the invisible spacer (10) and
        // sentence space (11) — must reach the transcript as spacing, never as
        // the CP437 glyphs their byte values carry. Every v6 story defaults to
        // interpreter 6 (IBM PC), which turns CP437 translation on, and Shogun
        // prints 11 between sentences: its cabin once read "…sea chest here.♂
        // Sitting on the desk…". (SQ-0545)
        for g in ['\u{2642}', '\u{25D9}', '\u{25CB}'] {
            assert!(
                !result.transcript.contains(g),
                "turn {turn}: CP437 glyph {g:?} leaked in place of a v6 spacing code: {:?}",
                result.transcript
            );
        }
        if result.transcript.contains("Bridge") {
            saw_bridge_look = true;
        }
        if result.transcript.contains("know the word") {
            saw_unknown_word = true;
        }
        // The v6 window model's own text runs feed the raster/hybrid renderer;
        // they must be clean too.
        if let Some(v6) = session.machine.screen.v6.as_ref() {
            for (i, w) in v6.windows.iter().enumerate() {
                let joined: String = w.texts.iter().map(|t| t.text.as_str()).collect();
                assert_eq!(
                    ctrl_chars(&joined),
                    Vec::<char>::new(),
                    "turn {turn}: control chars in window {i} text runs"
                );
            }
        }
    }
    assert!(saw_bridge_look, "\"look\" never re-described the Bridge — parser not receiving input");
    assert!(saw_unknown_word, "\"xyzzy\" never got the unknown-word reply — parser not receiving input");

    // v6 paint semantics: the status window overprints in place each turn —
    // old glyphs must be REPLACED, not accumulated (the "status corrupting
    // itself" live report). One "Score:" label, ever.
    let v6 = session.machine.screen.v6.as_ref().unwrap();
    let status: String = v6.windows[1].texts.iter().map(|t| t.text.as_str()).collect();
    assert_eq!(
        status.matches("Score:").count(),
        1,
        "status runs accumulated instead of overprinting: {status:?}"
    );
}

/// SQ-0489: Shogun's opening scene is a v6 margin picture (ZMSD §15). The game
/// draws picture 7 at the RIGHT of window 0 and issues `set_margins(left=2,
/// right=328)` on the 548px window, so prose flows in the LEFT column beside the
/// art, then full width once it scrolls past. Pinned end-to-end from the real
/// game: (a) the engine stores the asymmetric margins (small left, large right),
/// (b) the session classifies the window-0 picture as a `MarginRight` story
/// float, and (c) `build_main_text` lays it out as a RIGHT float — prose flush
/// left in a narrowed column, the picture pinned near the right edge.
#[test]
fn shogun_opening_is_a_right_margin_float() {
    use app::inline_image::ImageAlign;
    use app::session::TranscriptElem;

    let story_path = stories_dir().join("shogun-r322-s890706.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Shogun (v6) should load and boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();

    // Leave the title splash (any key) and select START (Enter) — the opening
    // scene draws picture 7 into window 0 and issues set_margins(2, 328). Sample
    // immediately: the game later resets the margins via a newline interrupt as
    // the prose scrolls past the art, so the asymmetric state is transient. The
    // window-0 picture rides the turn's ordered `transcript_elems`.
    session.submit_char(b' ');
    let mut elems: Vec<TranscriptElem> = session.submit_char(13).transcript_elems;
    // A couple of settling steps in case the draw lands a step later — but stop
    // the moment the margin picture appears so a later reset can't erase it.
    for _ in 0..3 {
        if elems.iter().any(|e| matches!(e, TranscriptElem::Image(img) if img.align == ImageAlign::MarginRight)) {
            break;
        }
        let r = match session.pending_input() {
            InputKind::Char => session.submit_char(13),
            InputKind::Line => session.submit(""),
            InputKind::Event => session.submit(""),
        };
        elems.extend(r.transcript_elems);
    }

    // (a) The engine honoured the game's `set_margins`: an asymmetric right margin
    // that leaves a prose column on the left (the ZMSD §15 margin picture).
    let w0 = &session.machine.screen.v6.as_ref().expect("v6 screen").windows[0];
    assert!(
        w0.right_margin > w0.left_margin && w0.right_margin >= 64,
        "window 0 carries the opening's large right margin (L={}, R={})",
        w0.left_margin, w0.right_margin
    );
    let text_col = w0.x_size.saturating_sub(w0.right_margin).saturating_sub(w0.left_margin);
    assert!(text_col >= 64, "a prose-wide left text column survives the margin (px {text_col})");

    // (b) The session classified the window-0 picture as a MarginRight story float.
    let img = elems.iter().find_map(|e| match e {
        TranscriptElem::Image(img) if img.align == ImageAlign::MarginRight => Some(img.clone()),
        _ => None,
    }).expect("the opening picture is classified as a MarginRight float");

    // (c) build_main_text lays it out as a right float: prose flush left in a
    // narrowed column, the picture pinned near the right edge.
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    app::state::apply_transcript_elems(&mut state, &elems);
    let cols = 68u16; // ~548px / 8px cell — Shogun's window-0 width in cells
    let (main, _) = app::render::screen::build_main_text(&state, cols, 40);
    let f = main.floats.iter().find(|f| {
        std::sync::Arc::ptr_eq(&f.img, &img.pixels)
    }).expect("the margin picture became a right float (not a full-width band)");
    // build_main_text reserves the picture's own cell width (+1 gutter) on the
    // right and pins the art there; prose stays flush left in the remainder.
    let img_cols = (img.pixels.width().div_ceil(8)) as u16; // FONT_W = 8
    assert_eq!(f.text_col, 0, "prose stays flush left beside a right float");
    assert_eq!(f.reserve_cols, (img_cols + 1).min(cols), "reserve = picture cell width + gutter");
    assert_eq!(f.img_col, cols.saturating_sub(img_cols), "picture pinned flush to the right edge");
    assert!(f.img_col > 0, "the picture floats to the right, not at the left margin");
    assert!(cols.saturating_sub(f.reserve_cols) >= 8, "a prose column survives beside the picture");
}

/// Shogun's boot menu prints its items through a 1-px-wide caret window with
/// wrapping OFF — three horizontal runs on distinct pixel rows, NOT a vertical
/// column of glyphs (the live "nothing after 'You may choose to:'" report),
/// and the title-splash canvas must be gone once the menu screen draws its
/// border decorations (the game erases window 7 first — the "splash never
/// cleared" report).
#[test]
fn shogun_boot_menu_items_paint_horizontally_and_splash_clears() {
    let story_path = stories_dir().join("shogun-r322-s890706.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Shogun (v6) should load and boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let splash_seq = session.pictures_canvas.get(&7).map(|c| c.z_seq);
    assert!(splash_seq.is_some(), "boot draws the title splash into window 7's canvas");

    // Any key leaves the title; the menu screen draws.
    let _ = session.take_transcript();
    let result = session.submit_char(b' ');
    assert!(result.fault.is_none(), "menu draw faulted: {:?}", result.fault);
    assert!(result.transcript.contains("You may choose to:"), "menu prompt missing");

    // The splash was erased and the border decorations repainted the canvas:
    // same window, LATER draw sequence.
    let menu_seq = session.pictures_canvas.get(&7).map(|c| c.z_seq);
    assert!(
        menu_seq > splash_seq,
        "window 7's canvas must be erased + repainted for the menu (splash {splash_seq:?} → {menu_seq:?})"
    );

    // Menu items: one horizontal band per item, each a row of runs at ONE y.
    let v6 = session.machine.screen.v6.as_ref().unwrap();
    let mut lines: std::collections::BTreeMap<u16, String> = Default::default();
    for t in &v6.windows[2].texts {
        lines.entry(t.y).or_default().push_str(&t.text);
    }
    let joined: Vec<&str> = lines.values().map(|s| s.trim()).collect();
    assert_eq!(
        joined,
        vec!["START the game", "RESTORE a saved game", "QUIT the game"],
        "menu items must paint as horizontal per-row bands, got {lines:?}"
    );
}

/// SQ-0478: in frameless mode Shogun's BOOT MENU is a painted text screen — its
/// three items are absolutely positioned at native rows 21–23 through window 2,
/// DEEP below the status band. The painted-screen renderer must stamp them as
/// positioned terminal text (the old anchored-band path dropped every run below
/// row 4, leaving "nothing after 'You may choose to:'"), and the selected item's
/// reverse-video run must carry the REVERSED modifier (the visible caret).
///
/// They land on pane rows 18–20, not 21–23: the cell path packs the native screen
/// (SQ-0697), so the frozen title banner's nine inked rows (native 3–11) pack
/// against the pane top, and the menu — painted inside the story box the game moved
/// down to native row 21 — travels with that box, three rows up.
#[test]
fn shogun_frameless_boot_menu_paints_items_and_caret() {
    use ratatui::style::Modifier;
    let story_path = stories_dir().join("shogun-r322-s890706.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Shogun (v6) should load and boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    // Any key leaves the title splash; the boot menu draws.
    let _ = session.take_transcript();
    let result = session.submit_char(b' ');
    assert!(result.transcript.contains("You may choose to:"), "menu prompt missing");

    use app::engine::Engine;
    let model = session.screen();
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    // Force the CELL path: SQ-0895 removed frameless, which was the
    // deliberate route in. Dropping the picker is the substitute whose
    // ONLY effect is the one frameless contributed here — draw no game
    // image. (A modal overlay also lands on the cell path, but it
    // additionally suppresses the inlined input line, which shifts row
    // counts.)
    state.game_picker = None;
    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' ')).collect()
    };
    // The three items, on the packed rows the story box carried them to.
    let r0 = row_text(18);
    let r1 = row_text(19);
    let r2 = row_text(20);
    eprintln!("18|{r0}|\n19|{r1}|\n20|{r2}|");
    assert!(r0.contains("START the game"), "row 18 shows the START item: {r0:?}");
    assert!(r1.contains("RESTORE a saved game"), "row 19 shows the RESTORE item: {r1:?}");
    assert!(r2.contains("QUIT the game"), "row 20 shows the QUIT item: {r2:?}");
    // The selected item (START, style bit 0 = reverse) carries the REVERSED
    // modifier — the visible selection caret.
    let start_col = r0.find("START").unwrap() as u16;
    let cell = buf.cell((start_col, 18)).unwrap();
    assert!(cell.modifier.contains(Modifier::REVERSED), "START item is reverse-video (the selection caret)");
    // RESTORE (style 0) is NOT reversed.
    let restore_col = r1.find("RESTORE").unwrap() as u16;
    assert!(!buf.cell((restore_col, 19)).unwrap().modifier.contains(Modifier::REVERSED), "RESTORE is not reversed");
}

/// SQ-0484: Shogun's boot menu in HYBRID render mode. The menu keeps window 0
/// (the story buffer) open AND paints its three items as DEEP chrome runs (native
/// rows 21–23). The old ring+viewport hybrid path split that menu across the raster
/// pixel ring (items mapping above the terminal viewport) and the terminal overlay
/// (items inside it) — the "first option raster, rest terminal text" defect.
///
/// SQ-0886 and then SQ-0892 RETARGETED THIS, and the requirement it was written for
/// has never moved: the screen is ONE coherent thing, never half ring and half
/// overlay. What moved is which coherent thing.
///
/// SQ-0886 sent it to the COMPOSITE, because routing it to the all-text cell path
/// threw away every pixel Shogun had drawn — its two ornate side panels and the
/// machine's ground — and the player got a full-width black block where the frame
/// belongs. SQ-0892 sent it to the RING, which draws the panels as art and the menu
/// as glyphs; the composite could only ever draw the menu as pixels, which is what
/// SQ-0750 forbids wherever the runs account for the pixels themselves.
///
/// So the coherence requirement is asserted in its most direct form yet: all three
/// items are terminal GLYPHS, on CONSECUTIVE rows, in the order the game printed
/// them. That is precisely the shape the original defect broke — and the shape both
/// of SQ-0892's own defects broke too, one per axis: independently rounded columns
/// gave `SI(RT th e ga me`, and independently rounded rows put the three items on
/// terminal rows 26, 28 and 29 with the first clipped off the top of the viewport.
///
/// The solid selection bar (SQ-0487) is asserted on both paths: as terminal cells
/// carrying the reverse attribute on the ring, and as the highlight block the
/// composite paints in raster mode, gaps between the words included.
#[test]
fn shogun_hybrid_boot_menu_is_one_coherent_ring_screen_with_a_solid_selection_bar() {
    let story_path = stories_dir().join("shogun-r322-s890706.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Shogun (v6) should load and boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    let result = session.submit_char(b' ');
    assert!(result.transcript.contains("You may choose to:"), "menu prompt missing");

    let model = session.screen();
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    // One coherent screen, and it is the RING — never the split the report named,
    // and never the art-less cell path SQ-0886 measured.
    let path = state.v6_path_log.borrow().last().map(|(l, _)| l.clone()).unwrap_or_default();
    assert_eq!(
        path, "hybrid-ring",
        "the boot menu draws over Shogun's own side panels, so hybrid draws it on the ring: \
         panels as art, menu as glyphs (SQ-0892). The cell path draws no art and left the \
         player a black block where the frame belongs (SQ-0886)"
    );

    // …and the three items are glyphs, on consecutive rows, in the game's own order.
    let rows: Vec<String> = (0..area.height)
        .map(|y| (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect())
        .collect();
    let row_of = |s: &str| {
        rows.iter()
            .position(|r| r.contains(s))
            .unwrap_or_else(|| panic!("the menu item {s:?} reaches the pane as text:\n{}", rows.join("\n")))
    };
    let (start, restore, quit) = (row_of("START the game"), row_of("RESTORE a saved game"), row_of("QUIT the game"));
    assert_eq!(
        (restore, quit),
        (start + 1, start + 2),
        "the game printed its three items on consecutive native rows, so they occupy consecutive \
         terminal rows — no skipped row through the middle of the menu:\n{}",
        rows.join("\n")
    );
    // The selected item's cells carry the highlight across their whole span — the
    // inter-word gaps included, which is the moth-eaten defect SQ-0487 was about.
    // Measured against the SAME columns one row down, which is the unselected item:
    // a bar that painted only the glyph cells would leave the gaps agreeing with it.
    let start_col = rows[start].find("START the game").map(|b| rows[start][..b].chars().count()).expect("located above");
    // The game asks for the bar with reverse video (ZMSD §8.7.1 bit 1), which
    // `v6_run_style` carries into the cell as `REVERSED`.
    let barred = |y: usize, x: usize| {
        buf.cell((x as u16, y as u16))
            .unwrap()
            .style()
            .add_modifier
            .contains(ratatui::style::Modifier::REVERSED)
    };
    let span = start_col..start_col + "START the game".chars().count();
    assert!(
        span.clone().all(|x| barred(start, x)),
        "the selected item carries a solid highlight across all fourteen of its columns, \
         inter-word gaps included (SQ-0487):\n{}",
        rows.join("\n")
    );
    assert!(
        span.clone().all(|x| !barred(restore, x)),
        "RESTORE is not selected, so its row carries no highlight bar:\n{}",
        rows.join("\n")
    );

    // The composite still paints the same bar in RASTER mode, where it is pixels.
    state.config.v6_render = app::config::V6RenderMode::Raster;

    // …and the SELECTED item carries a solid highlight bar (SQ-0487). Shogun prints
    // its menu one glyph at a time through a 1px caret window, and the spaces
    // between the words are their own reversed runs — so the bar is only solid if
    // every one of them paints. Read off the composite's own canvas: every COLUMN of
    // the item's own 16px cell row, across all fourteen characters of "START the
    // game" from native x 234, must carry the highlight SOMEWHERE down its height.
    // Column-wise rather than row-wise because the bar reverses the pair — its ink
    // is the ground colour — so a glyph column and a dropped gap column are the same
    // colour on any single row, and only the column tells them apart.
    let items = match &model.root {
        WinNode::Layered(items) => items,
        _ => panic!("v6 builds a Layered root"),
    };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let (canvas, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
    let ground = canvas.get_pixel(180, 337).0; // the same row, left of the menu
    let lit = |x: u32, top: u32| (top..top + 16).any(|y| canvas.get_pixel(x, y).0 != ground);
    let gaps: Vec<u32> = (234..234 + 14 * 8).filter(|&x| !lit(x, 336)).collect();
    assert!(
        gaps.is_empty(),
        "the selection bar is solid across the whole item, inter-word gaps included — these \
         columns are bare ground {ground:?} down the item's whole height, which is the \
         moth-eaten defect: {gaps:?}"
    );
    // The UNSELECTED item below it is not barred: the ground shows between its words.
    let bare = (234..234 + 20 * 8).filter(|&x| !lit(x, 352)).count();
    assert!(bare > 0, "RESTORE is not selected, so its row carries no highlight bar");
}

/// SQ-0467 follow-up: the frameless status band fills its ENTIRE row(s) with the
/// upper_window background, not just the cells behind glyphs — the gaps between
/// the anchored groups read as one solid bar. Asserts a gap cell on the band row
/// carries a space with the band's background style (not a leftover/default cell).
#[test]
fn shogun_frameless_status_band_fills_row_background() {
    let story_path = stories_dir().join("shogun-r322-s890706.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Shogun (v6) should load and boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    for _turn in 0..6 {
        match session.pending_input() {
            InputKind::Line => { session.submit("look"); }
            InputKind::Char => { session.submit_char(13); }
            InputKind::Event => { session.submit(""); }
        }
    }
    let model = session.screen();
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    // Force the CELL path: SQ-0895 removed frameless, which was the
    // deliberate route in. Dropping the picker is the substitute whose
    // ONLY effect is the one frameless contributed here — draw no game
    // image. (A modal overlay also lands on the cell path, but it
    // additionally suppresses the inlined input line, which shifts row
    // counts.)
    state.game_picker = None;
    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    // Pre-dirty every cell with a sentinel glyph: a band that fills its whole
    // row(s) first overwrites the gaps between the anchored groups; the old
    // per-glyph stamp left them showing the sentinel.
    for y in 0..area.height {
        for x in 0..area.width {
            buf.cell_mut((x, y)).unwrap().set_symbol("X");
        }
    }
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    // Row 0 is a band row (Erasmus/SHOGUN/Score with gaps between). Every cell on
    // it must have been overwritten — no sentinel 'X' survives in the gaps.
    let row0: String = (0..area.width).map(|x| buf.cell((x, 0)).unwrap().symbol().chars().next().unwrap_or(' ')).collect();
    assert!(row0.contains("SHOGUN"), "row 0 is the band row: {row0:?}");
    assert!(!row0.contains('X'), "band row fully filled — no sentinel left in the gaps: {row0:?}");
}

/// SQ-0512: in HYBRID mode Shogun's in-game status band now carries the game's
/// explicit colours (black on white, non-reversed). The whole band strip must be
/// flooded with the game's background — every cell across the band row, INCLUDING
/// the gaps between the anchored groups, shares the glyphs' background (z-colour 9,
/// white), not the theme backdrop. Retargeted for SQ-0532/A-F5: the default palette
/// now resolves Standard colours to their ZMSD §8.3.1 true-colour equivalents
/// ("9 = white (true $7FFF)"), so white is the exact (255,255,255) the v6 pixel
/// paths already drew, no longer the dim ANSI `Color::Gray`.
/// The old whole-strip flood painted the theme `base` bg, so the background showed
/// only behind the glyphs. Pins that the band reads as one solid white panel.
#[test]
fn shogun_hybrid_status_band_floods_game_background() {
    use ratatui::style::Color;

    let story_path = stories_dir().join("shogun-r322-s890706.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Shogun (v6) should load and boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    // Enter past the boot menu (Char → 13) and settle into gameplay (Bridge), so the
    // status band carries the location + Score/Moves runs with explicit colours.
    for _turn in 0..6 {
        match session.pending_input() {
            InputKind::Line => { session.submit("look"); }
            InputKind::Char => { session.submit_char(13); }
            InputKind::Event => { session.submit(""); }
        }
    }

    let model = session.screen();
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    // honor_game_colours defaults true; assert so the pin below is meaningful.
    assert!(state.config.honor_game_colours, "game colours honoured by default");
    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    // The band row is the one carrying "SHOGUN" (native y=1: Erasmus / SHOGUN / Score).
    let band_y = (0..area.height)
        .find(|&y| row_text(y).contains("SHOGUN"))
        .expect("a terminal row carries the SHOGUN status title");
    let text = row_text(band_y);
    assert!(text.contains("Score:"), "band row also carries the Score label: {text:?}");

    // z-colour 9 (white) resolves through the theme palette to the §8.3.1 RGB.
    // Every cell from the first glyph to the last on the band row — INCLUDING the
    // gaps BETWEEN Erasmus, SHOGUN and Score — must carry that game background, not
    // the theme backdrop. Pre-fix the gaps kept the theme `base` bg.
    // SQ-0894: the band's OWN glyphs, not the row's first ink. The game's status
    // window is native x 46..594 — exactly the span BETWEEN the frame's two ornament
    // columns (0..46 and 594..640) — and a flank now owns those columns down every
    // row of art it may have, this one included, so the row begins and ends with the
    // ornament's half-blocks. Those cells are the frame; the band is what is between
    // them, and it is the band's flood this case is about. (`char` indices, not byte
    // offsets, for the same reason: a half-block is three bytes.)
    // Block Elements only (U+2580..U+259F): what the half-block backend emits for a
    // rasterised image. NOT `screen.rs`'s `is_box_glyph`, which begins at U+2500 and
    // would also swallow Box Drawing — the characters a game prints when it draws its
    // own frame with glyphs, which are the row's ink and not the ring's.
    let is_frame_art = |c: char| ('\u{2580}'..='\u{259F}').contains(&c);
    let own = |c: char| c != ' ' && !is_frame_art(c);
    let cells: Vec<char> = text.chars().collect();
    let first = cells.iter().position(|&c| own(c)).expect("band row has glyphs") as u16;
    let last = cells.iter().rposition(|&c| own(c)).expect("band row has glyphs") as u16;
    // Sanity: the glyph cells themselves are on the game's white bg.
    assert_eq!(
        buf.cell((first, band_y)).unwrap().bg,
        Color::Rgb(255, 255, 255),
        "the first status glyph is on the game's white (z-colour 9) bg: {text:?}"
    );
    for x in first..=last {
        assert_eq!(
            buf.cell((x, band_y)).unwrap().bg,
            Color::Rgb(255, 255, 255),
            "band cell ({x},{band_y}) — incl. inter-group gaps — must flood the game's white bg, \
             not the theme backdrop: {text:?}"
        );
    }
}

/// SQ-0544: the live input caret belongs on the game's `>` prompt row, even when
/// a tall margin float outlives the text beside it.
///
/// Shogun's opening floats the ship picture at the right margin and the prose
/// wraps down its left. The picture is far taller than that prose, so the wrapped
/// rows BELOW the `>` prompt are "leftover float rows" — they carry the float
/// geometry but an EMPTY text string. The inline-input path drew the typed command
/// and block cursor on `lines.last()` unconditionally, so the caret landed on one
/// of those empty leftover rows, near the picture's bottom, instead of after the
/// prompt (user report at the TTY, 2026-07-28). It must skip back over the
/// text-less float tail to the real prompt row.
#[test]
fn shogun_input_caret_sits_on_the_prompt_row_beside_a_tall_float() {
    let story_path = stories_dir().join("shogun-r322-s890706.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Shogun (v6) should load and boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    // Into the opening scene (splash → START), where the ship floats at the right
    // margin, and on to the `>` prompt.
    let mut elems: Vec<app::session::TranscriptElem> = session.submit_char(b' ').transcript_elems;
    for _ in 0..8 {
        if session.pending_input() == InputKind::Line {
            break;
        }
        let r = match session.pending_input() {
            InputKind::Char => session.submit_char(13),
            _ => session.submit(""),
        };
        elems.extend(r.transcript_elems);
    }
    assert_eq!(session.pending_input(), InputKind::Line, "Shogun reached its line prompt");
    assert!(
        elems.iter().any(|e| matches!(e, app::session::TranscriptElem::Image(_))),
        "the opening put its ship picture into the transcript as an inline image"
    );

    let model = session.screen();
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.focus = app::state::Focus::Game;
    // Feed the turn's ordered elements exactly as the run loop does, so the ship
    // arrives as a real inline float with the prose wrapped beside it.
    app::state::apply_transcript_elems(&mut state, &elems);
    // A string that cannot occur in Shogun's prose, so finding it locates the
    // live input unambiguously.
    state.input.value = "zzq".to_string();
    state.input.cursor = 3;

    let area = Rect::new(0, 0, 138, 49);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    // The live input is drawn flush after the prompt, so the row carrying the typed
    // text must be the row carrying the game's `>` — not a text-less float row.
    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    let typed_row = (0..area.height)
        .find(|&y| row_text(y).contains("zzq"))
        .expect("the live input is drawn somewhere in the pane");
    let text = row_text(typed_row);
    // The input is drawn FLUSH after the prompt row's own text, so the character
    // immediately before it must be story text. On a text-less leftover float row
    // the input starts at the left margin instead, preceded by blank (or by the
    // float's halfblock art) — which is the bug.
    let at = text.find("zzq").expect("located above");
    let before = text[..at].chars().next_back().expect("the input never starts at column 0");
    assert!(
        !before.is_whitespace() && !"▀▄█".contains(before),
        "the live input must sit flush after the prompt row's text, not at the left \
         margin of a text-less leftover float row; row {typed_row} preceded it with \
         {before:?}\n{}",
        (typed_row.saturating_sub(4)..(typed_row + 3).min(area.height))
            .map(|y| format!("  {y}: {:?}\n", row_text(y)))
            .collect::<String>()
    );
}

/// SQ-0543: at a LARGE pane the status band's two lines must stay ADJACENT.
///
/// Shogun's status grid is 2 cells / 32 native px tall, with its runs at native
/// y=1 ("Erasmus : … SHOGUN … Score:") and y=17 ("Bridge … Moves:") — 16px apart,
/// i.e. consecutive rows, no blank line. But chrome TEXT strips used to position
/// runs by scaled device pixels, and the ring's art scales with the pane while
/// terminal text does not: at 138×49 one 16px game row spans ~2.2 terminal rows,
/// so the second line landed two rows down and a blank row opened through the
/// middle of the white band (user report at the TTY, 2026-07-28). A text strip has
/// no art behind it to stay aligned with, so it now lays out by the game's own row
/// structure and the two lines arrive adjacent at any pane size.
#[test]
fn shogun_hybrid_status_band_rows_stay_adjacent_at_a_large_pane() {
    let story_path = stories_dir().join("shogun-r322-s890706.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Shogun (v6) should load and boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    for _turn in 0..6 {
        match session.pending_input() {
            InputKind::Line => { session.submit("look"); }
            InputKind::Char => { session.submit_char(13); }
            InputKind::Event => { session.submit(""); }
        }
    }

    let model = session.screen();
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    // The reported geometry: a big pane, where the old device-pixel mapping spread
    // the band. (Small panes never showed it — the scale was near 1.)
    let area = Rect::new(0, 0, 138, 49);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    let title_y = (0..area.height)
        .find(|&y| row_text(y).contains("SHOGUN"))
        .expect("a terminal row carries the SHOGUN status title");
    let moves_y = (0..area.height)
        .find(|&y| row_text(y).contains("Moves:"))
        .expect("a terminal row carries the Moves counter");
    assert_eq!(
        moves_y,
        title_y + 1,
        "the band's second line must sit directly under the first (no scale-introduced \
         blank row); got title at {title_y}, Moves at {moves_y}\n  {title_y}: {:?}\n  {}: {:?}\n  {moves_y}: {:?}",
        row_text(title_y),
        title_y + 1,
        row_text(title_y + 1),
        row_text(moves_y),
    );
}

/// SQ-0511: Shogun's frame ENCLOSES the story to the native bottom (story bottom
/// 400 of 400) and is flanked by full-height side art, so at a TALL pane it takes
/// the `Frame` reclaim plan: the status band stays uniform-scaled + pane-top-
/// anchored, the story viewport top-anchors just under it and extends to the pane
/// BOTTOM at constant width, and the side flanks stretch to fill the reclaimed
/// space. Pins the tall-pane viewport exactly (halfblocks 10×20, native 640×400,
/// scale 1.40625, off_y 0; story native x=46 y=32 w=548 → cols 7..83 (one reserved
/// for the scrollbar → 75), top row 3, extended to row 40).
#[test]
fn shogun_hybrid_tall_pane_frame_reclaim() {
    use ratatui::layout::Rect;
    use ratatui::style::Color;
    let story_path = stories_dir().join("shogun-r322-s890706.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Shogun (v6) should load and boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    // Enter past the boot menu and settle into gameplay (Bridge).
    for _turn in 0..6 {
        match session.pending_input() {
            InputKind::Line => { session.submit("look"); }
            InputKind::Char => { session.submit_char(13); }
            InputKind::Event => { session.submit(""); }
        }
    }

    let model = session.screen();
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    let area = Rect::new(0, 0, 90, 40);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    let vp = state.transcript_geom.get().expect("hybrid renders the story as a transcript").area;

    assert_eq!(vp, Rect::new(7, 3, 75, 37), "Shogun top-anchors under the status band + extends to the pane bottom (Frame reclaim)");
    assert!(vp.y > 0 && vp.y <= 3, "story top-anchored just under the status band, not centred: {vp:?}");
    assert_eq!(vp.bottom(), area.bottom(), "story viewport extends to the pane bottom (slack reclaimed): {vp:?}");

    let is_img = |x: u16, y: u16| -> bool {
        let c = buf.cell((x, y)).unwrap();
        let s = c.symbol();
        s == "\u{2580}" || s == "\u{2584}" || c.bg != Color::Reset
    };
    // The side flanks stretch into the reclaimed space; at a deep row both flanks
    // carry ring cells and the left flank abuts the viewport with no seam.
    for dy in [vp.bottom() - 3, vp.bottom() - 1] {
        let lmax = (0..vp.x).rev().find(|&x| is_img(x, dy)).expect("left flank stretched into reclaimed space");
        assert_eq!(lmax, vp.x - 1, "left flank abuts the story viewport with no seam at row {dy} (lmax {lmax}, vp.x {})", vp.x);
        assert!((vp.right()..area.width).any(|x| is_img(x, dy)), "right flank stretched down to row {dy}");
    }
}

/// SQ-0467: in frameless mode Shogun's in-game status band lays out as a classic
/// full-width status line — the character/location runs anchored flush LEFT, the
/// "SHOGUN" title CENTERED, and the Score/Moves runs flush RIGHT across the whole
/// 80-col pane — instead of the old per-run cell-quantization that bunched every
/// run into the left 40 columns.
#[test]
fn shogun_frameless_status_band_anchors_left_center_right() {
    let story_path = stories_dir().join("shogun-r322-s890706.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Shogun (v6) should load and boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    // Select START on the boot menu and settle into gameplay (Bridge), so the
    // status band carries the location + Score/Moves runs.
    for _turn in 0..6 {
        match session.pending_input() {
            InputKind::Line => { session.submit("look"); }
            InputKind::Char => { session.submit_char(13); }
            InputKind::Event => { session.submit(""); }
        }
    }

    let model = session.screen();
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    // Force the CELL path: SQ-0895 removed frameless, which was the
    // deliberate route in. Dropping the picker is the substitute whose
    // ONLY effect is the one frameless contributed here — draw no game
    // image. (A modal overlay also lands on the cell path, but it
    // additionally suppresses the inlined input line, which shifts row
    // counts.)
    state.game_picker = None;
    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let band: Vec<String> = (0..4)
        .map(|y| (0..area.width).map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' ')).collect())
        .collect();
    eprintln!("--- Shogun FRAMELESS status band (80 cols) ---");
    for (y, r) in band.iter().enumerate() {
        eprintln!("{y}|{r}|");
    }

    // Location/character run flush LEFT (col 0 printed).
    assert_ne!(band[0].chars().next(), Some(' '), "left run flush at col 0: {:?}", band[0]);
    // "SHOGUN" title CENTERED (±1 of the pane centre).
    let title_row = band.iter().find(|r| r.contains("SHOGUN")).expect("a band row carries the SHOGUN title");
    let start = title_row.find("SHOGUN").unwrap();
    let expected = (area.width as usize - "SHOGUN".len()) / 2;
    assert!((start as i32 - expected as i32).abs() <= 1, "SHOGUN centered (at {start}, want ~{expected}): {title_row:?}");
    // Score anchored flush RIGHT (last glyph within the final two columns).
    let score_row = band.iter().find(|r| r.contains("Score:")).expect("a band row carries the Score label");
    let last_glyph = score_row.rfind(|c: char| c != ' ').expect("score row has text");
    assert!(last_glyph >= area.width as usize - 2, "Score flush right (last glyph col {last_glyph}): {score_row:?}");
}

/// SQ-0519 (raster twin of SQ-0512): in RASTER mode Shogun's in-game status band
/// carries the game's explicit black-on-white colours (non-reversed). The native
/// chrome canvas must flood the band's whole WINDOW width with the game's white
/// background BEFORE the glyphs stamp, so the band reads as one solid bar in the
/// pixel composite — the gaps between "Erasmus :", "SHOGUN" and "Score:" carry the
/// explicit white, the same as under the glyphs, not the transparent page. Probes a
/// pixel in a bare gap between two band runs on native row 0.
#[test]
fn shogun_raster_status_band_floods_game_white() {
    use app::engine::{PxText, WinNode};
    use app::render::v6_layout as v6;
    use image::Rgba;

    let story_path = stories_dir().join("shogun-r322-s890706.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Shogun (v6) should load and boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    // Enter past the boot menu (Char → 13) and settle into gameplay (Bridge), so the
    // status band carries the location + Score/Moves runs with explicit colours.
    for _turn in 0..6 {
        match session.pending_input() {
            InputKind::Line => { session.submit("look"); }
            InputKind::Char => { session.submit_char(13); }
            InputKind::Event => { session.submit(""); }
        }
    }

    let screen = session.screen();
    let WinNode::Layered(items) = &screen.root else {
        panic!("v6 story's screen() root must be WinNode::Layered, got {:?}", screen.root);
    };
    let native = v6::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = v6::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let colors = app::colors::ColorScheme::default();
    let default_fg = Rgba([220, 220, 220, 255]);
    let default_bg = Rgba([0, 0, 0, 255]);
    let canvas = v6::build_chrome_canvas(&layout.chrome, native, default_fg, default_bg, &colors, v6::TextLayer::All, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));

    // Gather the status band's runs: chrome grid runs on native row 0 (px y=1), the
    // "SHOGUN" title row, a non-reverse black-on-white band (bg = z-colour 9,
    // packed 0x01000009).
    let mut band: Vec<&PxText> = Vec::new();
    for it in &layout.chrome {
        if let WinNode::Grid(g) = &it.node {
            for t in &g.px_texts {
                if t.y == 1 {
                    band.push(t);
                }
            }
        }
    }
    band.sort_by_key(|t| t.x);
    assert!(
        band.iter().any(|t| t.text.contains("SHOGUN")),
        "native row 0 carries the SHOGUN title: {:?}",
        band.iter().map(|t| &t.text).collect::<Vec<_>>()
    );
    let white_run = band
        .iter()
        .find(|t| t.bg == 0x0100_0009)
        .expect("a band run names the explicit white bg (z-colour 9, packed 0x01000009)");
    assert_eq!(white_run.style & 1, 0, "the status band runs are NON-reversed (SQ-0519 is a bg flood, not reverse)");

    // Find a bare GAP between two adjacent band runs and probe its midpoint. Runs
    // carry screen-absolute 1-based px; a run spans FONT_W(8) per char.
    let (gap_start, gap_end) = band
        .windows(2)
        .find_map(|w| {
            let end = (w[0].x.max(1) as u32 - 1) + w[0].text.chars().count() as u32 * 8;
            let start = w[1].x.max(1) as u32 - 1;
            (start > end + 4).then_some((end, start))
        })
        .expect("two band runs with a bare gap between them");
    let gx = (gap_start + gap_end) / 2;
    let gy = 8; // inside native row 0 (py 0..16)
    // z-colour 9 (white) resolves to spec white (255,255,255) on the pixel path
    // (see v6_layout's packed_standard_palette_colour_blits_its_own_rgb_not_default).
    // The gap — no glyph over it — must carry that flooded white, not the page.
    assert_eq!(
        *canvas.get_pixel(gx, gy),
        Rgba([255, 255, 255, 255]),
        "the inter-run gap on the status band floods the game's explicit white, same as under the glyphs (px {gx},{gy})"
    );
}

/// SQ-0540: Shogun emphasises its PROSE — the room name ("Bridge") is bold and
/// the ship's name ("Erasmus") italic — so the v6 raster story path must carry
/// those §8.7.1 bits per character from the transcript's style runs all the way
/// into the synthesized bitmap faces, instead of drawing everything roman.
#[test]
fn shogun_prose_emphasis_reaches_the_raster_faces() {
    use app::render::v6_layout as v6;

    let story_path = stories_dir().join("shogun-r322-s890706.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Shogun (v6) should load and boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();

    // Leave the title splash (any key), select START (Enter), then play a few
    // turns — the opening scene prints the emphasised room name / ship name.
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    session.submit_char(b' ');
    for turn in 0..6 {
        let r = match session.pending_input() {
            InputKind::Char => session.submit_char(13),
            InputKind::Line => session.submit("look"),
            InputKind::Event => session.submit(""),
        };
        state.push_transcript_runs(&r.transcript, app::state::TranscriptKind::Story, &r.transcript_runs);
        if turn >= 1 && state.transcript_runs.iter().any(|runs| runs.iter().any(|s| s.bits & 6 != 0)) {
            break;
        }
    }

    let cols = 68u16;
    let (main, _) = app::render::screen::build_main_text(&state, cols, 40);
    // The emphasised words, as the raster model sees them.
    let emphasised: Vec<(u8, String)> = main
        .lines
        .iter()
        .zip(main.styles.iter())
        .flat_map(|(line, styles)| {
            let chars: Vec<char> = line.chars().collect();
            let mut out: Vec<(u8, String)> = Vec::new();
            for (i, &b) in styles.iter().enumerate() {
                if b == 0 {
                    continue;
                }
                match out.last_mut() {
                    Some((prev, s)) if *prev == b && i > 0 && styles[i - 1] == b => s.push(chars[i]),
                    _ => out.push((b, chars[i].to_string())),
                }
            }
            out
        })
        .collect();
    eprintln!("Shogun emphasised prose runs: {emphasised:?}");
    assert!(
        emphasised.iter().any(|(bits, _)| bits & 2 != 0),
        "Shogun's prose must reach the raster model with BOLD chars, got {emphasised:?}"
    );

    // Those bits must actually change the pixels: the same lines drawn with the
    // style vector cleared are the roman rendering, and every extra bold pixel is
    // a +1 double-strike of it.
    let draw = |m: &v6::MainText| {
        let mut c = image::RgbaImage::new(cols as u32 * 8, 40 * 16);
        v6::draw_story_text(&mut c, m, 0, 0, cols, 40, image::Rgba([255, 255, 255, 255]), &[], &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT), None);
        c
    };
    let styled = draw(&main);
    let roman = draw(&v6::MainText { styles: Vec::new(), ..main.clone() });
    assert_ne!(styled, roman, "emphasised prose must not rasterize identically to roman prose");
    let lit = |c: &image::RgbaImage| -> std::collections::BTreeSet<(u32, u32)> {
        c.enumerate_pixels().filter(|(_, _, p)| p[3] >= 128).map(|(x, y, _)| (x, y)).collect()
    };
    let (s, r) = (lit(&styled), lit(&roman));
    for &(x, y) in s.difference(&r) {
        assert!(x > 0 && r.contains(&(x - 1, y)), "styled pixel ({x},{y}) is not a +1 shift of the roman face");
    }
}

/// SQ-0894: Shogun's two ornaments are one strip each and cover the SAME rows.
///
/// A frame's two side columns are one object drawn twice, so nothing about the pane
/// may make them disagree about where they start and stop. Two separate mechanisms
/// could, and this case guards both.
///
/// **One strip each.** Before this quest a flank's vertical extent was the story
/// viewport's by definition, so the rows above and below it in the same columns
/// belonged to the full-width top and bottom bands — one column composed of up to
/// three pieces, drawn by two different routines off two different canvases.
///
/// **The same rows.** The row rule is per side, and the two sides can disagree for a
/// reason that has nothing to do with the artwork. Shogun's status band sits at
/// native x 46..594 — exactly between the ornaments — and its first glyph is at
/// native 49. At a 98x37 pane the left ornament ends at 7.04 cells and that glyph
/// lands at 7.35: the SAME terminal column, so the text wins it and the left flank
/// yields the band's rows, while the right flank's last run ends clear of its columns
/// and it takes them. That put ornament in one top corner and bare ground under the
/// band's flood in the other. `content_ring_bands` intersects the two row sets for
/// exactly this reason.
///
/// FALSIFY by removing that intersection (the `if left_cols.1 > left_cols.0 && …`
/// block in `content_ring_bands`): the right strip comes back `8x37 at (90,0)`
/// against the left's `8x35 at (0,2)`, and the case reports the row spans differing.
#[test]
fn shogun_ornament_columns_are_one_strip_and_span_the_same_rows() {
    let story_path = stories_dir().join("shogun-r322-s890706.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Shogun (v6) should load and boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    for _ in 0..6 {
        match session.pending_input() {
            InputKind::Line => {
                session.submit("look");
            }
            InputKind::Char => {
                session.submit_char(13);
            }
            InputKind::Event => {
                session.submit("");
            }
        }
    }
    let model = session.screen();

    for honor in [true, false] {
        for &(w, h) in &[(98u16, 37u16), (100u16, 40u16), (120u16, 45u16)] {
            let mut state = app::state::AppState::default();
            state.colors = app::colors::ColorScheme::terminal_default();
            #[allow(deprecated)] // `from_fontsize`: a headless test has no terminal to query.
            let picker =
                ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18));
            state.game_picker = Some(picker);
            state.config.v6_render = app::config::V6RenderMode::Hybrid;
            state.config.honor_game_colours = honor;
            let area = Rect::new(0, 0, w, h);
            let mut buf = Buffer::empty(area);
            let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

            let map = state.v6_cell_map.borrow();
            // A flank is a NARROW side column; the full-width top/bottom bands are
            // art strips too and are not what this case is about. Each entry is the
            // strip's `(x, y, width, height)` in cells; only its row span is asserted.
            let rows = |right_hand: bool| -> Vec<(u16, u16)> {
                map.iter()
                    .filter(|r| r.label.starts_with("strip:art") && !r.label.contains("skipped"))
                    .map(|r| r.cells)
                    .filter(|c| c.2 * 3 < w && (c.0 * 2 > w) == right_hand)
                    .map(|c| (c.1, c.1 + c.3))
                    .collect()
            };
            let (left, right) = (rows(false), rows(true));
            assert_eq!(
                (left.len(), right.len()),
                (1, 1),
                "honor={honor} {w}x{h}: each ornament is ONE strip, not one piece per band edge; \
                 got left {left:?} right {right:?}"
            );
            assert_eq!(
                left, right,
                "honor={honor} {w}x{h}: the frame's two ornaments are one object drawn twice, so \
                 they start and stop on the same rows"
            );
        }
    }
}
