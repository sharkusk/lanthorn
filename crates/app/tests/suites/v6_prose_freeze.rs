//! SQ-0697: prose freezes where it was painted when its window moves.
//!
//! ZMSD §15 is explicit that `move_window`/`window_size` "do not change the
//! current display": text already printed stays as pixels where it was drawn. A
//! scrolling window is no exception, and Shogun's opening turns on it — the game
//! prints its whole title header while window 0 is the full 640x400 screen, then
//! moves window 0 down to a 548x64 box beside its menu and prints "You may choose
//! to:" there. A real interpreter leaves the header painted up top. lanthorn
//! streamed both halves into one transcript, so they came out adjacent and the
//! header scrolled out of a four-row box.
//!
//! Measured from the screen ops (`trace_screen`), one turn, in order:
//!
//! ```text
//!   @set_cursor(row=49, col=297, window=0)   … nine centred header lines
//!   @move_window(win=0, y=33,  x=47)         <- the freeze fires here
//!   @window_size(win=0, y=368, x=548)
//!   @move_window(win=0, y=337, x=47)
//!   @window_size(win=0, y=64,  x=548)
//!   @erase_window(lower)                     … clears the NEW box only
//!   @set_cursor(row=1, col=1, window=0)
//!   "You may choose to:"
//! ```
//!
//! So the engine shadows what a wrap+scroll window streams, and hands that shadow
//! to `ZWindow::texts` — real paint — the moment the window's box changes. The
//! host answers with a `TranscriptElem::ScreenClear` at the same point in the
//! turn's output: the frozen half stays in scrollback, the live screen restarts
//! at the window's new origin.
//!
//! The corpus guard matters as much as the fix: Zork Zero, Arthur and Journey all
//! move window 0 during play, and freezing on every move would eat their
//! transcripts. They do it right after an `erase_window`, which empties the
//! shadow, so nothing is ever retired for them — asserted below over 25 turns of
//! real play each.
//!
//! Stories are gitignored (CLAUDE.md), so each case skips cleanly.


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

/// Window 0's painted runs, as `(y, x, text)`.
fn win0_runs(session: &GameSession) -> Vec<(u16, u16, String)> {
    session
        .machine
        .screen
        .v6
        .as_ref()
        .map(|v6| v6.windows[0].retired.iter().map(|t| (t.y, t.x, t.text.clone())).collect())
        .unwrap_or_default()
}

/// How many runs window 0 still holds as LIVE streamed prose — the other side of
/// [`win0_runs`], and what tells a partial retirement from a whole one.
fn win0_streamed(session: &GameSession) -> usize {
    session.machine.screen.v6.as_ref().map(|v6| v6.windows[0].streamed.len()).unwrap_or(0)
}

/// Shogun's title header freezes at the pixel columns the game declared.
///
/// Falsified by making `ZWindow::retire_streamed` a no-op: "Shogun's title header
/// must freeze where it was painted when window 0 moves down to the menu — window
/// 0 carries 0 painted run(s)".
#[test]
fn shogun_title_header_freezes_where_it_was_painted() {
    let Some(mut session) = boot("shogun-r322-s890706.z6") else { return };
    let result = match session.pending_input() {
        InputKind::Char => session.submit_char(13),
        _ => session.submit(""),
    };

    let runs = win0_runs(&session);
    assert!(
        !runs.is_empty(),
        "Shogun's title header must freeze where it was painted when window 0 moves down to \
         the menu — window 0 carries {} painted run(s)",
        runs.len()
    );

    // The game centres every line itself, by cursor position: it reads window 0's
    // width (640) and its own row, then set_cursors to the exact column. Those
    // columns are what the freeze has to preserve — a transcript cannot carry them.
    let header: Vec<(u16, u16, &str)> =
        runs.iter().map(|(y, x, t)| (*y, *x, t.as_str())).collect();
    assert!(
        header.contains(&(49, 297, "SHOGUN")),
        "the title lands at its declared column (8px/char centring of 6 chars in 640px = 297): \
         {header:?}"
    );
    assert!(
        header.contains(&(65, 257, "A Story of Japan")),
        "…and every line below it keeps its own: {header:?}"
    );

    // Row spacing is one 16px text row per line, so the block reads as a paragraph
    // and not as nine independently-placed labels.
    let mut rows: Vec<u16> = header.iter().map(|(y, _, _)| *y).collect();
    rows.sort_unstable();
    rows.dedup();
    assert_eq!(
        rows,
        vec![49, 65, 81, 97, 113, 129, 145, 161, 177],
        "the frozen header keeps the game's own 16px row grid"
    );

    // The boundary is still marked, and the prompt the game printed at the
    // window's new origin is still after it — but the frozen half is no longer
    // emitted at all (SQ-0890). It is PAINT now, asserted run by run above, and
    // the story box renders the transcript: keeping a copy there drew the header
    // a second time into the four-row box, across the game's own menu. Nothing
    // above the boundary reaches the host, so `before` is empty.
    let mut before = String::new();
    let mut after = String::new();
    let mut seen_clear = false;
    for e in &result.transcript_elems {
        match e {
            TranscriptElem::ScreenClear => seen_clear = true,
            TranscriptElem::Text { text, .. } => {
                if seen_clear {
                    after.push_str(text)
                } else {
                    before.push_str(text)
                }
            }
            TranscriptElem::Image(_) => {}
        }
    }
    assert!(seen_clear, "the turn carries a screen-clear boundary at the freeze");
    assert!(
        before.is_empty(),
        "the frozen half is paint, not transcript — nothing above the boundary is \
         emitted: {before:?}"
    );
    assert_eq!(
        after.trim(),
        "You may choose to:",
        "and the live screen restarts with what the game printed at the new origin"
    );
    // The reported offset is still the boundary in the FLAT transcript, which
    // keeps every character the turn printed (the mapper reads it) — so it counts
    // the header the elems channel dropped.
    assert_eq!(
        result.prose_retired,
        Some(result.transcript.chars().count() - after.chars().count()),
        "the reported offset is the boundary itself"
    );
}

/// …and the header reaches the composite, up top, clear of the story box — in
/// both colour modes (the text is the game's own, not a palette preference).
#[test]
fn shogun_frozen_header_reaches_the_composite() {
    let Some(mut session) = boot("shogun-r322-s890706.z6") else { return };
    match session.pending_input() {
        InputKind::Char => session.submit_char(13),
        _ => session.submit(""),
    };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);

    // The frozen prose publishes as its own paint layer, at the prose's extent —
    // NOT at the window's new box, which would read as an overlay strip sitting
    // inside the story and shove the live transcript out of it.
    let frozen = layout
        .chrome
        .iter()
        .find(|pw| matches!(&pw.node, WinNode::Grid(g) if g.px_texts.iter().any(|t| t.text == "SHOGUN")))
        .expect("the frozen header is published as a paint layer");
    assert!(
        frozen.y_px + frozen.h_px <= layout.story.expect("story window").y_px,
        "the frozen layer sits clear above the story box: frozen y={} h={}, story y={}",
        frozen.y_px,
        frozen.h_px,
        layout.story.unwrap().y_px
    );

    for honor in [true, false] {
        let mut state = app::state::AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.honor_game_colours = honor;
        if let Some(p) = Engine::paint_surface(&session) {
            *state.v6_paint.borrow_mut() = Some(p);
        }
        let (canvas, _metrics) =
            app::render::screen::build_v6_raster_canvas(&layout, native, &state);

        // Ink on the header's own rows, in the middle third of the screen where
        // the frame art never reaches — so this cannot pass on the border alone.
        let inked = (40..190)
            .flat_map(|y| (200..440).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                canvas.get_pixel_checked(x, y).is_some_and(|p| p[3] == 255 && (p[0], p[1], p[2]) != (0, 0, 0))
            })
            .count();
        assert!(
            inked > 500,
            "honor={honor}: the frozen header must be painted in the composite's upper half; \
             only {inked} inked pixel(s) there"
        );
    }
}

/// SQ-0717 — reported by a player as "some of shogun intro text is no longer
/// centered", minutes after the freeze landed. The centring survives in EVERY
/// render path, which is the assertion that would have caught it at merge time:
/// the previous guard checked only the raster composite, and raster is the one path
/// that was never wrong.
///
/// Measured, Shogun's title, before the fix — lines losing their centring:
///   raster     0 of 9   (the composite paints the frozen layer as pixels)
///   hybrid     6 of 9   ← the shipped default, and what the player saw
///   frameless  6 of 9   (identical to hybrid)
///
/// Both cell paths route text above the story through the frameless status band,
/// which sorts each run into a LEFT/CENTER/RIGHT anchor group by where it STARTS —
/// the right question for a status field, the wrong one for a paragraph. Shogun's
/// nine lines are centred by the game's own cursor arithmetic, so the five longest
/// begin left of the left-third boundary (208px of a 640px screen) and the shortest
/// ends past the right two-thirds: five flushed to the left margin, one to the
/// right. A run with equal margins on the game's screen was centred on purpose and
/// stays centred in the pane.
///
/// Falsified by reverting the `centred` exemption in `draw_anchored_status_band`:
/// "hybrid honor=true 80x25: \"Copyright (c) 1988 by Infocom\" keeps the centring
/// the game gave it (at col 0, want ~25)".
#[test]
fn shogun_frozen_header_stays_centred_in_every_render_path() {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    let Some(mut session) = boot("shogun-r322-s890706.z6") else { return };
    match session.pending_input() {
        InputKind::Char => session.submit_char(13),
        _ => session.submit(""),
    };
    let model = session.screen();

    // Every line of the header, exactly as the game centred it.
    let lines = [
        "SHOGUN",
        "A Story of Japan",
        "Copyright (c) 1988 by Infocom",
        "All rights reserved.",
        "SHOGUN is a trademark of James Clavell",
        "Original Literary Work Copyright 1975 by James Clavell",
        "Licensed by Noble House Trading Limited, London.",
        "Release 322 / Pix 322 / Serial number 890706",
        "IBM Interpreter version 6.65",
    ];

    // The paths that draw this header as GLYPHS in the cell buffer.
    //
    // SQ-0886 took hybrid out of this loop: the boot menu is a painted takeover over
    // Shogun's own side panels, and hybrid sent it to the composite rather than to
    // the art-less cell path, which drew the screen as a full-width black block with
    // no frame on it at all. SQ-0892 put hybrid back, and on the RING — which draws
    // the panels as art and the header as text, so it belongs in this loop and not in
    // the composite arm below. It is also the arm that would have caught SQ-0892's
    // own defect: driven here before the fix, the ring mapped each line's own native
    // pixel through the scale and the nine centres came out five columns apart.
    for tag in ["cell", "hybrid"] {
        for honor in [true, false] {
            for (w, h) in [(80u16, 25u16), (120, 40)] {
                let mut state = app::state::AppState::default();
                state.colors = app::colors::ColorScheme::terminal_default();
                state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
                if tag == "cell" {
                    // Force the CELL path: SQ-0895 removed frameless, which was the
                    // deliberate route in. Dropping the picker is the substitute whose
                    // ONLY effect is the one frameless contributed here — draw no game
                    // image. (A modal overlay also lands on the cell path, but it
                    // additionally suppresses the inlined input line, which shifts row
                    // counts.)
                    state.game_picker = None;
                } else {
                    state.config.v6_render = app::config::V6RenderMode::Hybrid;
                }
                state.config.honor_game_colours = honor;
                let area = Rect::new(0, 0, w, h);
                let mut buf = Buffer::empty(area);
                let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
                let row_text = |y: u16| -> String {
                    (0..w).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
                };
                let rows: Vec<String> = (0..h).map(row_text).collect();
                let ctx = format!("{tag} honor={honor} {w}x{h}");

                // CHAR index, not byte: on the ring the row begins with the frame's
                // half-blocks, three bytes each, and `str::find` would report a column
                // eight past the one the glyph is in (SQ-0894 fixed the same defect in
                // `v6_shogun_status_alignment`). The cell path's rows are all-ASCII, so
                // the two agreed there and only ever there.
                let find_col = |r: &String, s: &str| r.find(s).map(|b| r[..b].chars().count());
                for line in lines {
                    let at = rows
                        .iter()
                        .find_map(|r| find_col(r, line))
                        .unwrap_or_else(|| panic!("{ctx}: the frozen header line {line:?} reaches the pane:\n{}", rows.join("\n")));
                    let want = (w as usize - line.chars().count()) / 2;
                    assert!(
                        (at as i32 - want as i32).abs() <= 1,
                        "{ctx}: {line:?} keeps the centring the game gave it (at col {at}, want ~{want}):\n{}",
                        rows.join("\n")
                    );
                }

                // The block still reads as one paragraph: consecutive rows, in order.
                let first = rows.iter().position(|r| r.contains("SHOGUN")).expect("header rows located above");
                for (i, line) in lines.iter().enumerate() {
                    assert!(
                        rows[first + i].contains(line),
                        "{ctx}: the header keeps the game's own row order at row {}: {:?}",
                        first + i,
                        rows[first + i]
                    );
                }
            }
        }
    }

    // …and the COMPOSITE, which is the raster MODE's answer for this frame. It paints
    // the frozen layer as pixels at the game's own coordinates and was never wrong,
    // but it is pinned here so the invariant is "centred in every path" rather than
    // "centred in the ones we happened to fix". Measured as the INK extent of each
    // 16px text row, inside the middle band where the frame art never reaches.
    //
    // Which path each MODE takes is asserted first, so an arm that quietly stopped
    // covering what it names cannot pass by covering nothing. SQ-0892 moved hybrid off
    // the composite and onto the ring; raster is unmoved, and still the only mode that
    // draws this screen as pixels.
    for honor in [true, false] {
        for (mode, want) in [
            (app::config::V6RenderMode::Hybrid, "hybrid-ring"),
            (app::config::V6RenderMode::Raster, "raster"),
        ] {
            let mut state = app::state::AppState::default();
            state.colors = app::colors::ColorScheme::terminal_default();
            state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
            state.config.v6_render = mode;
            state.config.honor_game_colours = honor;
            let area = Rect::new(0, 0, 80, 25);
            let mut buf = Buffer::empty(area);
            let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
            let path = state.v6_path_log.borrow().last().map(|(l, _)| l.clone()).unwrap_or_default();
            assert_eq!(
                path, want,
                "{mode:?} honor={honor}: Shogun's boot menu is a painted takeover over the game's \
                 own artwork. Hybrid draws it on the RING — panels as art, header and menu as \
                 glyphs (SQ-0892); raster draws every pixel of it as the composite"
            );
        }
    }

    let layout = app::render::v6_layout::classify_windows(match &model.root {
        WinNode::Layered(items) => items,
        _ => panic!("v6 builds a Layered root"),
    }, zvm::screen::V6Cell::DEFAULT);
    let native = match &model.root {
        WinNode::Layered(items) => app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT)),
        _ => unreachable!(),
    };
    for honor in [true, false] {
        let mut state = app::state::AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.honor_game_colours = honor;
        if let Some(p) = Engine::paint_surface(&session) {
            *state.v6_paint.borrow_mut() = Some(p);
        }
        let (canvas, _metrics) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
        // The header occupies native text rows 3..12 (y = 49, 65, … 177).
        for row in 3..12u32 {
            let (mut lo, mut hi) = (u32::MAX, 0u32);
            for y in row * 16..(row * 16 + 16).min(canvas.height()) {
                for x in 80..560u32.min(canvas.width()) {
                    let ink = canvas
                        .get_pixel_checked(x, y)
                        .is_some_and(|p| p[3] == 255 && (p[0] as u16 + p[1] as u16 + p[2] as u16) > 200);
                    if ink {
                        lo = lo.min(x);
                        hi = hi.max(x);
                    }
                }
            }
            assert_ne!(lo, u32::MAX, "raster honor={honor}: header row {row} is painted");
            let right = native.0 as u32 - (hi + 1);
            // Equal margins to within one 8px cell — the glyph's ink stops short of
            // its advance, so the right margin runs a few pixels wide.
            assert!(
                lo.abs_diff(right) <= 8,
                "raster honor={honor}: header row {row} stays centred (ink {lo}..{hi}, \
                 left margin {lo}px, right margin {right}px of {}px)",
                native.0
            );
        }
    }
}

/// SQ-0697, second half — the freeze was right and the RESUME was unplaced.
/// Reported by a player, clean boot, no overlay: "'You may choose to:' appears the
/// line below the last centred line of intro text."
///
/// The freeze leaves the nine banner lines painted across native rows 3–11; the
/// game then moves window 0 to a 548x64 box at (47,337) — four rows, level with and
/// to the LEFT of its START/RESTORE/QUIT menu at (235,337) — and prints the prompt
/// there. `/dump-windows` confirmed we hold the right box. The cell path simply
/// started the transcript flush under the band and let it flow, so the prompt came
/// out nine rows above the menu it belongs beside.
///
/// The story window's box is authoritative for where its transcript renders: the
/// transcript begins at the box's own declared offset below the chrome above it,
/// and everything painted INSIDE the box — the menu's glyphs and the erased ground
/// under them — moves with it.
///
/// Falsified by reverting `story_row` to `top_used`: "hybrid honor=true 80x25:
/// 'You may choose to:' renders beside the menu, not above it (prompt row 9, menu
/// row 21)".
#[test]
fn shogun_resumed_prompt_lands_beside_the_menu() {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    let Some(mut session) = boot("shogun-r322-s890706.z6") else { return };
    let result = match session.pending_input() {
        InputKind::Char => session.submit_char(13),
        _ => session.submit(""),
    };
    let model = session.screen();

    // SQ-0886: the CELL paths. Hybrid left them for this frame — its takeover
    // escape now routes a menu screen with the game's ARTWORK behind it to the
    // composite, which places both windows at the coordinates the game declared
    // (window 0 at x=47, window 2 at x=235, both on native row 21) and so satisfies
    // this relation by construction. Asserted as pixels at the foot of the case.
    for tag in ["cell"] {
        for honor in [true, false] {
            for (w, h) in [(80u16, 25u16), (100, 40)] {
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
                state.config.honor_game_colours = honor;
                app::state::apply_transcript_elems(&mut state, &result.transcript_elems);
                let area = Rect::new(0, 0, w, h);
                let mut buf = Buffer::empty(area);
                let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
                let rows: Vec<String> = (0..h)
                    .map(|y| {
                        (0..w).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
                    })
                    .collect();
                let ctx = format!("{tag} honor={honor} {w}x{h}");

                let prompt = rows
                    .iter()
                    .position(|r| r.contains("You may choose to:"))
                    .unwrap_or_else(|| panic!("{ctx}: the prompt reaches the pane:\n{}", rows.join("\n")));
                let menu = rows
                    .iter()
                    .position(|r| r.contains("START the game"))
                    .unwrap_or_else(|| panic!("{ctx}: the menu reaches the pane:\n{}", rows.join("\n")));
                assert_eq!(
                    prompt, menu,
                    "{ctx}: \"You may choose to:\" renders beside the menu, not above it \
                     (prompt row {prompt}, menu row {menu}):\n{}",
                    rows.join("\n")
                );
                // …and to its LEFT, which is the layout the game declared: window 0
                // at x=47, window 2 at x=235.
                let prompt_col = rows[prompt].find("You may choose to:").expect("located above");
                let menu_col = rows[menu].find("START the game").expect("located above");
                assert!(
                    prompt_col < menu_col,
                    "{ctx}: the prompt sits left of the menu (prompt col {prompt_col}, menu col {menu_col}):\n{}",
                    rows.join("\n")
                );
                // The frozen banner still owns the top of the pane, unchanged, with
                // the game's own gap between it and the box it moved down to.
                assert!(
                    rows[0].contains("SHOGUN"),
                    "{ctx}: the frozen banner still starts at the pane top: {:?}",
                    rows[0]
                );
                // Row 18, at every pane size, because the cell path packs the native
                // screen and not the pane: the banner's nine inked native rows (3–11)
                // pack to pane rows 0–8, then the nine rows of empty screen the game
                // left between its banner and its story box (native 12–20) carry
                // through, putting the box's first row at 9 + 9 = 18. Pinning the row
                // itself is what separates "the prompt is beside the menu" from "the
                // prompt and the menu drifted up together".
                assert_eq!(
                    prompt, 18,
                    "{ctx}: the story box lands its own declared distance below the \
                     banner — nine packed banner rows, then the game's nine-row gap:\n{}",
                    rows.join("\n")
                );
            }
        }
    }

    // …and HYBRID, which since SQ-0892 draws this frame on the RING: the prompt is
    // transcript text in the story viewport and the menu is chrome runs stamped over
    // it, so the relation is now visible in the cell buffer and is asserted there,
    // on the path hybrid actually takes.
    for honor in [true, false] {
        let mut state = app::state::AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.v6_render = app::config::V6RenderMode::Hybrid;
        state.config.honor_game_colours = honor;
        app::state::apply_transcript_elems(&mut state, &result.transcript_elems);
        let area = Rect::new(0, 0, 100, 40);
        let mut buf = Buffer::empty(area);
        let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
        let path = state.v6_path_log.borrow().last().map(|(l, _)| l.clone()).unwrap_or_default();
        assert_eq!(
            path, "hybrid-ring",
            "hybrid honor={honor}: the ring draws this frame — panels as art, menu as glyphs (SQ-0892)"
        );
        let rows: Vec<String> = (0..area.height)
            .map(|y| (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect())
            .collect();
        // CHAR index, not byte — the ring's rows begin with the frame's half-blocks.
        let col_of = |r: &String, s: &str| r.find(s).map(|b| r[..b].chars().count());
        let prompt = rows
            .iter()
            .position(|r| r.contains("You may choose to:"))
            .unwrap_or_else(|| panic!("hybrid honor={honor}: the prompt reaches the pane:\n{}", rows.join("\n")));
        let menu = rows
            .iter()
            .position(|r| r.contains("START the game"))
            .unwrap_or_else(|| panic!("hybrid honor={honor}: the menu reaches the pane:\n{}", rows.join("\n")));
        assert_eq!(
            prompt, menu,
            "hybrid honor={honor}: the prompt lands on the menu's OWN row, not nine above it:\n{}",
            rows.join("\n")
        );
        assert!(
            col_of(&rows[prompt], "You may choose to:") < col_of(&rows[menu], "START the game"),
            "hybrid honor={honor}: and to its left, as the game placed them (window 0 at native \
             x=47, window 2 at x=235):\n{}",
            rows.join("\n")
        );
    }

    // …and the COMPOSITE, which is what raster MODE still draws. The same relation,
    // measured where the composite states it: on the game's own native row 21
    // (y 336..351), ink stands both left of native x=235 — the prompt, in window 0
    // at x=47 — and at or right of it, which is the menu in window 2.
    let items = match &model.root {
        WinNode::Layered(items) => items,
        _ => panic!("v6 builds a Layered root"),
    };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    for honor in [true, false] {
        let mut state = app::state::AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.v6_render = app::config::V6RenderMode::Raster;
        state.config.honor_game_colours = honor;
        app::state::apply_transcript_elems(&mut state, &result.transcript_elems);
        let area = Rect::new(0, 0, 100, 40);
        let mut buf = Buffer::empty(area);
        let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
        let path = state.v6_path_log.borrow().last().map(|(l, _)| l.clone()).unwrap_or_default();
        assert_eq!(path, "raster", "raster honor={honor}: this frame is drawn by the composite");

        let (canvas, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
        let ground = canvas.get_pixel(20, 340).0;
        let ink = |x0: u32, x1: u32| {
            (336..352).any(|y| (x0..x1).any(|x| canvas.get_pixel_checked(x, y).is_some_and(|p| p.0 != ground)))
        };
        assert!(
            ink(47, 235),
            "hybrid honor={honor}: the prompt renders on the menu's own row, in the box the game \
             moved window 0 to (native x 47..235 of row 21) — not nine rows above it"
        );
        assert!(ink(235, native.0 as u32), "hybrid honor={honor}: the menu renders on that same row");
    }
}

/// The corpus guard for the placement rule: every other v6 game puts its story
/// window flush under its chrome, so honouring the box must move NOTHING.
///
/// Measured native geometry — the story window's declared top, and the chrome box
/// above it, in 16px text rows:
///
/// ```text
///   advent   status window 640x20  at row 0 → box bottom row 2; story at row 1
///   Arthur   status window 584x16  at row 12 → box bottom row 13; story at row 13
///   Zork0    status panel 640x78   at row 0 → box bottom row 5; story at row 4
///   Journey  nothing above the story at all;                    story at row 0
/// ```
///
/// Zork Zero is the case that forced the gap to be measured against the chrome's
/// declared BOX rather than its ink: only two of that 78px panel's five rows carry
/// runs, the band has already compressed it to those two, and re-counting its own
/// slack as empty screen pushed the whole transcript down two rows for art frameless
/// had deliberately dropped.
#[test]
fn a_story_window_flush_under_its_chrome_keeps_the_pane_top() {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    // (story file, driver command, the pane row its transcript starts on)
    for (name, cmd, want_row) in [
        ("advent.z6", "look", 1u16),
        ("arthur-r74-s890714.z6", "look", 1),
        ("zork0-r393-s890714.z6", "look", 2),
        ("journey-r83-s890706.z6", "", 0),
    ] {
        let Some(mut session) = boot(name) else { continue };
        for turn in 0..8 {
            let r = match session.pending_input() {
                InputKind::Char => session.submit_char(13),
                InputKind::Line => session.submit(if turn % 2 == 0 { cmd } else { "" }),
                InputKind::Event => session.submit(""),
            };
            if r.transcript.to_lowercase().contains("y or n") {
                let _ = session.submit_char(b'n');
            }
        }
        let model = session.screen();
        for honor in [true, false] {
            let mut state = app::state::AppState::default();
            state.colors = app::colors::ColorScheme::terminal_default();
            state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
            // FRAMELESS forces the cell path for a game that would otherwise take
            // the hybrid pixel ring — that path is where the placement rule lives.
            // Force the CELL path: SQ-0895 removed frameless, which was the
            // deliberate route in. Dropping the picker is the substitute whose
            // ONLY effect is the one frameless contributed here — draw no game
            // image. (A modal overlay also lands on the cell path, but it
            // additionally suppresses the inlined input line, which shifts row
            // counts.)
            state.game_picker = None;
            state.config.honor_game_colours = honor;
            let area = Rect::new(0, 0, 80, 25);
            let mut buf = Buffer::empty(area);
            let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
            let map = state.v6_cell_map.borrow();
            let story = map
                .iter()
                .find(|r| r.label.starts_with("path:cell"))
                .unwrap_or_else(|| panic!("{name} honor={honor}: frameless takes the cell path: {map:?}"));
            assert_eq!(
                story.cells.1, want_row,
                "{name} honor={honor}: its story window sits flush under its chrome, so the \
                 transcript keeps the row it has always started on (got {}, want {want_row})",
                story.cells.1
            );
        }
    }
}

/// The corpus guard, and the reason the freeze is scoped to what the window's new
/// box LEAVES rather than to the box merely changing.
///
/// Zork Zero moves window 0 at boot and again after its title splash, Journey
/// after `split_window(400)`, Arthur on almost every turn of play — its story
/// window is resized around the narration it has just printed. Arthur is the one
/// that bites: freezing whenever the box changed froze its prose seven turns
/// running and stacked its stale `>` prompts up as paint over the churchyard,
/// which broke `v6_arthur_status`'s hybrid cases outright. The window still
/// covers that text, so the text is still the window's own and still belongs to
/// the streaming transcript.
///
/// Both drivers are run: the plain play loop, and the 'n'-at-the-restore-prompt
/// path `v6_arthur_status` uses — the first never reaches Arthur's resize at all,
/// which is exactly how the regression got through the first time.
#[test]
fn a_window_that_still_covers_its_prose_freezes_nothing() {
    for name in [
        "zork0-r393-s890714.z6",
        "arthur-r74-s890714.z6",
        "journey-r83-s890706.z6",
        "advent.z6",
    ] {
        for answer_n in [false, true] {
            let Some(mut session) = boot(name) else { continue };
            for turn in 0..25 {
                let result = match session.pending_input() {
                    InputKind::Char => session.submit_char(13),
                    InputKind::Line if answer_n => session.submit(""),
                    InputKind::Line => session.submit(if turn % 2 == 0 { "look" } else { "wait" }),
                    InputKind::Event => session.submit(""),
                };
                if answer_n && result.transcript.to_lowercase().contains("y or n") {
                    let _ = session.submit_char(b'n');
                }
                assert_eq!(
                    result.prose_retired, None,
                    "{name} turn {turn} (answer_n={answer_n}): nothing may freeze here — this \
                     game's window 0 still covers the prose it printed, so freezing would \
                     restart the transcript under a live session"
                );
                assert!(
                    win0_runs(&session).is_empty(),
                    "{name} turn {turn} (answer_n={answer_n}): window 0 must carry no frozen \
                     paint: {:?}",
                    win0_runs(&session)
                );
            }
        }
    }
}

/// SQ-1155: a PARTIAL retirement is not a screen clear.
///
/// Specimen: `arthur-r74-s890714.z6` (bare story file, so §8.3.1 is the table),
/// twelve intro taps answering `n` at the restore prompt — `v6_arthur_status`'s
/// own route to the first playable frame — then `look` (turn 13) and `wa` (turn
/// 14). `wa` is not in Arthur's dictionary, and his rejection does not go to
/// window 0 at all. Measured from `trace_screen`, the turn is
///
/// ```text
///   @scroll_window(win=0, pixels=16)
///   @window_size(win=0, y=176, x=584)     <- window 0 loses ONE row, 192 -> 176
///   @move_window(win=3, y=385, x=29)      <- his one-line message window opens
///   @window_size(win=3, y=16, x=584)         in the row window 0 just gave up
///   @erase_window(win3)
///   @set_window(win3)                     … "You don't need to use the word 'wa.'"
/// ```
///
/// Window 0 is never erased and the transcript is empty for the whole turn. The
/// row it gave up strands exactly one of its sixteen streamed runs, which is a
/// real retirement — that run is outside the box, becomes paint, and window 3's
/// erase paints over it. What it is NOT is a screen restart: the other fifteen
/// runs are still on screen and still the window's own. Announcing the boundary
/// anyway anchored the transcript past every one of them, and since the rejection
/// prints into window 3 the player was left with a blank pane and the message
/// alone on the bottom line — the reported symptom, bottom placement included.
///
/// A second `wa` in a row does not reproduce it and never could: the leading
/// `scroll_window` has already lifted the content clear of the new bottom, so
/// nothing is stranded. That is why the report reads as "conditional on there
/// being something to clear".
///
/// FALSIFY by dropping the `.filter(|_| froze_whole)` from `prose_cleared_at` in
/// `GameSession::finish_turn`: the `wa` turn comes back carrying a `ScreenClear`
/// and nothing else.
#[test]
fn arthur_shrinking_one_row_for_his_message_window_is_not_a_screen_clear() {
    let Some(mut session) = boot("arthur-r74-s890714.z6") else { return };
    for _ in 0..12 {
        let r = match session.pending_input() {
            InputKind::Line => session.submit(""),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = session.submit_char(b'n');
        }
    }

    let look = session.submit("look");
    assert!(
        look.transcript.contains("churchyard"),
        "turn 13 reaches the churchyard description, which is the content the wipe ate: {:?}",
        look.transcript
    );

    let before = win0_streamed(&session);
    let wa = session.submit("wa");
    let after = win0_streamed(&session);

    // Non-vacuity: the shape this case is about must actually occur. A turn that
    // retired nothing would pass the assertion below for the wrong reason, and a
    // turn that retired EVERYTHING is the Shogun case, where the boundary is
    // correct and the cases above already pin it.
    assert!(
        wa.prose_retired.is_some(),
        "the turn must RETIRE something — window 0 gives up its bottom row to \
         window 3 — or this case is vacuous"
    );
    assert!(
        after > 0 && after < before,
        "…and the retirement must be PARTIAL: window 0 streamed {before} run(s) \
         before the turn and {after} after, and this case only means anything \
         while some are stranded and some are kept"
    );
    assert!(
        wa.transcript.trim().is_empty(),
        "Arthur prints the rejection into window 3, not the transcript: {:?}",
        wa.transcript
    );
    assert!(
        !wa.transcript_elems.iter().any(|e| matches!(e, TranscriptElem::ScreenClear)),
        "a partial retirement must not announce a screen clear — the runs window 0 \
         kept are still on screen, and anchoring the transcript past them leaves \
         the player looking at a blank pane"
    );
}
