//! Arthur (Infocom v6) top status bar — SQ-0500 / SQ-0499 / SQ-0504.
//!
//! Arthur paints a reverse-video status bar (location + "St Anne's Day, Compline"
//! date) as pixel-positioned runs at native row 12, ABOVE the story buffer (which
//! starts at native row 13) and BELOW a graphics panel (native rows 0–11). The
//! panel's frame is a full-screen graphics window, but its INTERIOR — where the
//! status text sits — is transparent, so the status row is a pure-TEXT strip
//! sandwiched between art strips.
//!
//! HYBRID mode (SQ-0500): the status row decomposes into its own terminal-CELL
//! strip (crisp reverse bar) while the graphics panel above stays the pixel ring.
//! The bar reads solid — no unreversed gap anywhere INSIDE it (SQ-0504), including
//! the lone hole before the date the original SQ-0499 report was about.
//!
//! **It is solid across its own WINDOW and not across the pane** (SQ-0949). Arthur's
//! status window is native `28..612` of 640, and the 28 native columns it leaves at
//! each edge are where his poles stand. Both reference machines show the ribbon inset
//! with the frame's rule running past it, unbroken from the panel's foot to the
//! bottom of the screen: `machine-screenshots/dos-arthur.png` (the EGA press at the
//! Churchyard, "Merlin disappears as suddenly as he came") puts the white ribbon at
//! native **28..610** and the grey rule beside it at native **6.5..8.7**;
//! `machine-screenshots/mac-arthur.png` is the same frame with the black ribbon inset
//! and the green poles at a constant x above and below it. Reading SQ-0504's "a pure
//! reverse-video row fills edge to edge" as "fills the PANE" flooded the strip's
//! ground straight over both poles and cut each flank into a piece above the bar and
//! a piece below it — the step the SQ-0949 report describes as the side strip not
//! lining up with the panel above it.
//!
//! Skip-if-missing pattern per the other gitignored-story smokes.
//!
//! **Colour mode: `honor_game_colours = true`** — the app's shipped config
//! default, so these render assertions are made in the mode real players run.
//! (Before SQ-0532 wave 4 every v6 smoke booted with the game's colours
//! DECLINED, which is exactly why three colour-driven render regressions
//! shipped unseen. The theme-only `false` path is covered by the paired cases
//! in `v6_game_colour_regression.rs`.)

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot Arthur past the sword-in-the-stone intro to ordinary gameplay, where the
/// top status bar (location + date) is painted and the story buffer is streaming.
fn arthur_at_status() -> Option<GameSession> {
    let story_path = stories_dir().join("arthur-r74-s890714.z6");
    let story_bytes = std::fs::read(&story_path).ok().or_else(|| {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        None
    })?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Arthur (v6) should load and boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();

    // Tap through the intro; answer 'n' to the "restore a saved position?" prompt.
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
    Some(session)
}

/// The status runs reach the model as reverse-video grid runs at native row 12,
/// carrying the "St Anne's Day" date text — a sanity gate for the harness.
#[test]
fn arthur_status_reaches_the_model() {
    let Some(session) = arthur_at_status() else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let mut date = false;
    let mut reversed = false;
    for pw in items {
        if let WinNode::Grid(g) = &pw.node {
            for t in &g.px_texts {
                if (t.y.max(1) - 1) / 16 == 12 {
                    reversed |= t.style & 1 != 0;
                    if t.text.contains('A') || t.text.contains('n') {
                        date = true;
                    }
                }
            }
        }
    }
    assert!(date, "status runs present at native row 12");
    assert!(reversed, "status bar is reverse-video");
}

/// A cell the hybrid RING drew rather than the status strip: a kitty virtual
/// placeholder, a half-block the image encoder emitted, or any cell carrying a
/// background the ring painted. Used to say what stands BESIDE the ribbon — see the
/// module header: the answer must be the frame's poles, not bare theme backdrop.
fn is_ring_art(c: &ratatui::buffer::Cell) -> bool {
    let g = c.symbol().chars().next().unwrap_or(' ');
    g == '\u{10eeee}'
        || ('\u{2580}'..='\u{259f}').contains(&g)
        || c.bg != ratatui::style::Color::Reset
}

/// (SQ-0500 + SQ-0499 + SQ-0504, bounded by SQ-0949) HYBRID: the status row renders
/// as terminal CELLS — the "St Anne's Day, Compline" date is real buffer text — and
/// its reverse bar is solid with no unreversed gap INSIDE it, while the columns
/// outside it belong to the ring's flank art. The graphics panel above the status
/// row stays the pixel ring (half-block image cells).
///
/// FALSIFY by dropping the `row_spans` clause from `ChromeRowOracle::blocked`: the
/// bar floods the whole pane again and `beside` comes back empty at both ends,
/// which is the pole the report says the ribbon paints over.
#[test]
fn arthur_hybrid_status_row_is_solid_terminal_bar() {
    let Some(session) = arthur_at_status() else { return };
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    let area = Rect::new(0, 0, 80, 25);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    // Locate the terminal row carrying the status date as real cell text.
    let status_y = (0..area.height)
        .find(|&y| row_text(y).contains("Anne"))
        .expect("status date renders as terminal cells");

    // The bar is SOLID over its own span: every cell between its first and its last
    // reverse-video cell is reverse-video too, including the lone gap before the date
    // that was the original SQ-0499 hole.
    let rev: Vec<u16> = (0..area.width)
        .filter(|&x| buf.cell((x, status_y)).unwrap().modifier.contains(Modifier::REVERSED))
        .collect();
    assert!(!rev.is_empty(), "the status row carries a reverse bar\nrow: {:?}", row_text(status_y));
    let (first, last) = (rev[0], *rev.last().unwrap());
    let holes: Vec<u16> = (first..=last)
        .filter(|&x| !buf.cell((x, status_y)).unwrap().modifier.contains(Modifier::REVERSED))
        .collect();
    assert!(
        holes.is_empty(),
        "the reverse status bar is solid over [{first},{last}] with no unreversed gap; holes at \
         {holes:?}\nrow: {:?}",
        row_text(status_y)
    );

    // …and what stands beside it is the frame, not backdrop. Arthur's status window
    // is native 28..612 of 640, so at any pane there are columns at each edge the
    // ribbon must not have taken — that is where his poles are (see the module
    // header, and the DOS press capture it names).
    let beside: Vec<u16> = (0..area.width).filter(|&x| x < first || x > last).collect();
    assert!(
        !beside.is_empty(),
        "the ribbon reaches as far as its window and no further, so the flank keeps \
         columns at both edges of the status row; it took the whole pane [0,{})\nrow: {:?}",
        area.width,
        row_text(status_y)
    );
    let bare: Vec<u16> =
        beside.iter().copied().filter(|&x| !is_ring_art(buf.cell((x, status_y)).unwrap())).collect();
    assert!(
        bare.is_empty(),
        "every column the ribbon left is the flank's own art — the poles run through \
         this row unbroken; bare cells at {bare:?}\nrow: {:?}",
        row_text(status_y)
    );

    // The graphics panel above the status row stays the pixel ring.
    let is_image_cell = |x: u16, y: u16| -> bool {
        let c = buf.cell((x, y)).unwrap();
        let s = c.symbol();
        s == "\u{2580}" || s == "\u{2584}" || c.bg != ratatui::style::Color::Reset
    };
    let panel_ink = (0..status_y.saturating_sub(1))
        .flat_map(|y| (0..area.width).map(move |x| (x, y)))
        .filter(|&(x, y)| is_image_cell(x, y))
        .count();
    assert!(panel_ink > 0, "the graphics panel above the status stays imaged in the ring (got {panel_ink} image cells)");
}

/// (SQ-0509) At a pane size whose letterbox scale is not 1.0 (100×30 → scale 1.2),
/// Arthur's status text — emitted as single-glyph runs at fixed 8-px pixel starts —
/// must still read "Churchyard" as ONE unbroken word: the strip merges the abutting
/// fragments before mapping, instead of rounding each independently into stray cell
/// gaps ("Chu rch yard"). The right-hand date field stays in the right portion of
/// the bar (its runs are separated from the location by a real pixel gap, so they
/// do NOT merge into the location text).
#[test]
fn arthur_hybrid_status_churchyard_is_one_word_date_right() {
    let Some(session) = arthur_at_status() else { return };
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    // 100×30 → scale 1.2: at 1.0 the fixed 8-px runs already lined up; only a
    // non-unit scale exposed the per-run rounding that split the word.
    let area = Rect::new(0, 0, 100, 30);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    let status_y = (0..area.height).find(|&y| row_text(y).contains("Anne")).expect("status row present");
    let row = row_text(status_y);
    // The location reads as one unbroken word — no stray gap inside it.
    assert!(row.contains("Churchyard"), "location must read 'Churchyard' as one word; got: {row:?}");
    // The date field is intact and sits in the RIGHT portion of the bar (well past
    // the location on the left), never merged into the location run.
    let date_at = row.find("St Anne's Day, Compline").expect("date field intact and contiguous");
    let loc_at = row.find("Churchyard").unwrap();
    assert!(date_at > loc_at + "Churchyard".len(), "the date stays to the right of the location");
    assert!(date_at * 2 > area.width as usize, "the date sits in the right half of the {}-col bar (at {date_at})", area.width);
}

/// (SQ-0505 dynamic hybrid layout) At a TALL pane (90×40 — taller than the 8:5
/// native aspect, so there is vertical letterbox dead space), Arthur has no bottom
/// ART: header art + status bar on top, side borders, nothing painted below the
/// story. The ring is top-anchored (no vertical centering) and the story text
/// viewport extends to the pane BOTTOM at its constant inset width. Where the
/// side art runs out, the poles are TILED down the rest of the flank (SQ-0698) —
/// never stretched, which would elongate them by whatever the slack happens to be.
///
/// **RE-BLESSED, SQ-1008: "to the pane bottom" is one row short of it here, and
/// always was.** This frame is not the empty-below-the-story frame the case was
/// written for. `arthur-r74-s890714.z6` reaches gameplay by tapping blank lines,
/// and Arthur answers the last one in a BOX — window 3 at native
/// `(28, 384, 584, 16)`, the last text row of his 640x400 screen, carrying
/// *"I beg your pardon?"*. At 80x25 (no slack, letterbox plan) the ring drew it
/// all along; here the reclaim grew the viewport over it and it was on no screen
/// at all, which is the whole of SQ-1008. The reclaim itself is untouched — the
/// viewport is still 24 rows against window 0's 11 native ones — so this case
/// still measures what it was written to measure, one row higher.
#[test]
fn arthur_hybrid_tall_pane_extends_story_to_bottom() {
    use ratatui::style::Color;
    let Some(session) = arthur_at_status() else { return };
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    // 90×40 → scale 1.40625 (width-limited), ~238 px of vertical letterbox slack.
    let area = Rect::new(0, 0, 90, 40);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    // SQ-1008: the game's own bottom row is drawn, on the pane's last row…
    let boxed = (0..area.height).find(|&y| row_text(y).contains("I beg your pardon?"));
    assert_eq!(
        boxed,
        Some(area.bottom() - 1),
        "window 3's boxed message is bottom-anchored to the pane's last row; pane:\n{}",
        (0..area.height).map(row_text).collect::<Vec<_>>().join("\n")
    );
    let vp = state.transcript_geom.get().expect("hybrid renders the story as a transcript").area;
    // …and the story viewport reclaims every dead row above it.
    assert_eq!(
        vp.bottom(),
        area.bottom() - 1,
        "story viewport extends to the pane bottom, less window 3's own row; got {vp:?}"
    );
    assert!(vp.height > 11, "the reclaim survives — {vp:?} against window 0's 11 native rows");
    // It keeps its side insets — the story column did NOT reflow to full width.
    assert!(vp.x > 0 && vp.right() < area.right(), "story keeps its constant inset width beside the side borders; got {vp:?}");
    // The top chrome ring (header art + status bar) is still imaged above the story.
    let is_image = |x: u16, y: u16| -> bool {
        let c = buf.cell((x, y)).unwrap();
        let s = c.symbol();
        s == "\u{2580}" || s == "\u{2584}" || c.bg != Color::Reset
    };
    let top_ring = (0..vp.y).flat_map(|y| (0..area.width).map(move |x| (x, y))).filter(|&(x, y)| is_image(x, y)).count();
    assert!(top_ring > 0, "the header/status ring stays imaged above the top-anchored story ({top_ring} cells)");
    // A flank cell in a LEFT-band row beside the top of the story carries side
    // art — and so does one deep below it.
    //
    // RE-BLESSED, SQ-0698. This assertion used to read the other way: the flank
    // below the poles had to be `Color::Reset`, the theme backdrop, because
    // SQ-0511 clipped the Extend plan's ring to the artwork's own lowest opaque
    // row and left the rest bare. The user reported the consequence — "the side
    // columns for arthur does not stretch all the way down" — and it is
    // measurable: at this 90x40 pane Arthur's poles stop at native row 379 of
    // 400, which is terminal row 31 of 40, so the frame stood open down its
    // whole lower quarter. The poles are now TILED to the band's full height
    // (a 4-row texture cut at 90% of the pole's height, then its own foot —
    // Bocfel's `draw_arthur_side_images`), so the same cell is painted.
    let flank_art = (vp.y..vp.y + 6).any(|y| buf.cell((vp.x - 1, y)).unwrap().bg != Color::Reset);
    assert!(flank_art, "the side border art shows in the flank beside the top of the story");
    let deep = buf.cell((vp.x - 1, area.height - 2)).unwrap();
    assert_ne!(deep.bg, Color::Reset, "the flank keeps its border art all the way down the pane: {deep:?}");
}

/// (SQ-0549) FRAMELESS: Arthur's status bar must ANCHOR TO THE TOP of the pane.
///
/// Frameless drops the pixel chrome, so the 12-row graphics panel above the bar is
/// never drawn — but the bar itself used to be stamped absolutely at its NATIVE row
/// 12, leaving it floating about a quarter of the way down an otherwise blank pane.
/// The frameless band split is now a RELATION ("the chrome text above the story
/// window"), not the old `native row < 4` constant, so the bar lands on row 0 with
/// the transcript starting beneath it. Pinned at two pane sizes so the position
/// can't be re-derived from the pane geometry, and in BOTH `honor_game_colours`
/// modes — the reverse bar is a style bit, not a game colour, so it must read solid
/// either way.
#[test]
fn arthur_frameless_status_bar_anchors_to_the_top() {
    let Some(session) = arthur_at_status() else { return };
    let model = session.screen();

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
            let area = Rect::new(0, 0, w, h);
            let mut buf = Buffer::empty(area);
            let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

            let row_text = |y: u16| -> String {
                (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
            };
            let status_y = (0..area.height)
                .find(|&y| row_text(y).contains("Anne"))
                .unwrap_or_else(|| panic!("status date renders as terminal cells (honor={honor}, {w}x{h})"));
            assert_eq!(status_y, 0, "the status bar is the TOP row, not floating at its native row 12 (honor={honor}, {w}x{h})");

            // A classic anchored status line: location flush left, date flush right.
            let row = row_text(0);
            assert!(row.starts_with("Churchyard"), "location flush at col 0 (honor={honor}, {w}x{h}): {row:?}");
            // The SQ-0509 fragment merge reaches the band too: Arthur emits one run
            // per GLYPH, so without it every letter became its own anchor group.
            let date = "St Anne's Day, Compline";
            let date_at = row
                .find(date)
                .unwrap_or_else(|| panic!("the date reads as one contiguous field (honor={honor}, {w}x{h}): {row:?}"));
            assert_eq!(date_at + date.len(), area.width as usize, "the date is flush RIGHT: {row:?}");

            // The bar reads solid edge to edge, and nothing is stranded at row 12.
            let holes: Vec<u16> = (0..area.width)
                .filter(|&x| !buf.cell((x, 0)).unwrap().modifier.contains(Modifier::REVERSED))
                .collect();
            assert!(holes.is_empty(), "the reverse bar spans the pane (honor={honor}, {w}x{h}); holes at {holes:?}");
            assert!(
                row_text(12).trim().is_empty(),
                "nothing is stamped at the old absolute native row 12 (honor={honor}, {w}x{h}): {:?}",
                row_text(12)
            );
        }
    }
}

/// Boot Arthur, then issue the in-game `map` command. `after_enter` additionally
/// dismisses the map with a bare Enter. The two states differ ONLY in the story
/// window's native height: `map` grows win0 from 584×128 to 584×192 (so its bottom
/// reaches the native screen bottom, 400), and dismissing it shrinks it back.
fn arthur_showing_map(after_enter: bool) -> Option<GameSession> {
    let mut session = arthur_at_status()?;
    let _ = session.submit("map");
    if after_enter {
        let _ = session.submit("");
    }
    Some(session)
}

/// A hybrid-mode render at a real terminal cell size (8×17, the reporter's
/// Ghostty) with the Kitty protocol — the mode and geometry players actually run.
fn render_hybrid(model: &app::engine::ScreenModel, cols: u16, rows: u16) -> (Rect, Buffer) {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    // from_fontsize is deprecated in favour of a live stdio query a headless test
    // can't do; the fixed cell is the point here — the defect is cell-size driven.
    #[allow(deprecated)]
    let mut picker = ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 17));
    picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
    state.game_picker = Some(picker);
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
    (area, buf)
}

/// SQ-0571(a): Arthur's header picture must not MOVE when the in-game `map`
/// command runs.
///
/// `map` grows win0 to 584×192, putting its bottom at the native screen bottom
/// (400). `hybrid_bottom_plan` read that as "an enclosing frame" and — finding no
/// full-height side ART to stretch (Arthur's side borders are GRID windows, so the
/// graphics-only canvas is empty there) — fell back to `Letterbox`, whose CENTRED
/// vertical offset dropped the header art and the map drawn into it half the
/// letterbox slack down the pane: a band of blank rows opened above the picture,
/// and dismissing the map jumped it back to the top. A story window's height must
/// not decide where the header art lands, so a header panel above the story now
/// keeps the ring top-anchored (`Extend`) in both states.
#[test]
fn arthur_map_does_not_move_the_header_art() {
    let Some(shown) = arthur_showing_map(false) else { return };
    let Some(dismissed) = arthur_showing_map(true) else { return };

    for (cols, rows) in [(95u16, 51u16), (96, 51), (99, 51), (100, 51), (138, 51)] {
        let mut seen = Vec::new();
        for (label, session) in [("map shown", &shown), ("map dismissed", &dismissed)] {
            let model = session.screen();
            let (area, buf) = render_hybrid(&model, cols, rows);
            let ink = |x: u16, y: u16| -> bool {
                let c = buf.cell((x, y)).unwrap();
                c.symbol() != " " || c.bg != ratatui::style::Color::Reset
            };
            let first_ink_row = (0..area.height)
                .find(|&y| (0..area.width).any(|x| ink(x, y)))
                .unwrap_or_else(|| panic!("{label} at {cols}x{rows} drew nothing at all"));
            assert_eq!(
                first_ink_row, 0,
                "{label} at {cols}x{rows}: the header art starts at the pane TOP, with no blank band above it"
            );
            let status_y = (0..area.height)
                .find(|&y| {
                    (0..area.width)
                        .map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' '))
                        .collect::<String>()
                        .contains("Anne")
                })
                .unwrap_or_else(|| panic!("{label} at {cols}x{rows}: status bar renders as terminal cells"));
            seen.push((label, status_y));
        }
        assert_eq!(
            seen[0].1, seen[1].1,
            "the status bar sits on the SAME terminal row whether the map is shown or dismissed \
             at {cols}x{rows} ({} → row {}, {} → row {})",
            seen[0].0, seen[0].1, seen[1].0, seen[1].1
        );
    }
}

/// SQ-0571(b): Arthur's status bar must render as crisp terminal CELLS at EVERY
/// pane width — the width-dependent "corrupted location bar".
///
/// Under the `Extend` plan the chrome ring bands are clipped to the header art's
/// own vertical extent, at `ceil(art_bottom · s / cell_h)`, so the flanks below the
/// art stay theme backdrop. But `run_cell` maps a chrome run's native top by
/// ROUNDing, and Arthur's art ends at native y=192 exactly where its status row
/// begins. Whenever `192·s / cell_h` had a fraction ≥ 0.5 the ceil and the round
/// agreed, the clip landed exactly ON the status row and evicted it from the band:
/// no `Text` strip covered it, so `clear_text_rows` never carved it out of the band
/// canvas and the bar painted as a squashed raster slice of the frame instead of
/// cells — half-height, sliced glyphs. On an 8×17 cell that broke widths 96..=99
/// and left 95 and 100 clean, which is exactly how it was reported.
///
/// Sweeps every width across two full periods of that fraction and requires the bar
/// to be real cells, solid over its own window's span (SQ-0504/SQ-0949), with the
/// frame's poles standing in the columns it leaves, at all of them.
#[test]
fn arthur_status_bar_is_terminal_cells_at_every_pane_width() {
    let Some(session) = arthur_showing_map(true) else { return };
    let model = session.screen();

    let mut broken = Vec::new();
    for cols in 88u16..=112 {
        let (area, buf) = render_hybrid(&model, cols, 51);
        let row_text = |y: u16| -> String {
            (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
        };
        let Some(status_y) = (0..area.height).find(|&y| row_text(y).contains("Anne")) else {
            broken.push((cols, "status bar never reached the cell layer".to_string()));
            continue;
        };
        let row = row_text(status_y);
        if !row.contains("Churchyard") {
            broken.push((cols, format!("location text broken up: {row:?}")));
            continue;
        }
        // SQ-0504, bounded by SQ-0949: a pure reverse-video row reads solid over its
        // own window's span, and the columns outside it are the flank's pole art.
        let rev: Vec<u16> = (0..area.width)
            .filter(|&x| buf.cell((x, status_y)).unwrap().modifier.contains(Modifier::REVERSED))
            .collect();
        let Some((&first, &last)) = rev.first().zip(rev.last()) else {
            broken.push((cols, format!("no reverse bar on the status row: {row:?}")));
            continue;
        };
        let holes: Vec<u16> = (first..=last)
            .filter(|&x| !buf.cell((x, status_y)).unwrap().modifier.contains(Modifier::REVERSED))
            .collect();
        if !holes.is_empty() {
            broken.push((cols, format!("unreversed gaps at {holes:?} in {row:?}")));
            continue;
        }
        // On THIS frame the flank may legitimately be absent — `map` grows win0 to
        // 584x192 and Arthur's side borders are GRID windows, so the graphics-only
        // canvas is empty beside it and no flank is carved at all. The invariant that
        // holds either way is that the ribbon never takes a column the flank owns; the
        // pole itself is asserted on the gameplay frame, below.
        let bare: Vec<u16> = (0..area.width)
            .filter(|&x| x < first || x > last)
            .filter(|&x| !is_ring_art(buf.cell((x, status_y)).unwrap()))
            .collect();
        if !bare.is_empty() {
            broken.push((cols, format!("bare cells beside the ribbon at {bare:?} in {row:?}")));
        }
    }
    assert!(broken.is_empty(), "the status bar must be crisp cells at every width; failures: {broken:#?}");
}

/// A hybrid render at 8×17/Kitty that reports the recorded letterbox anchor: the
/// device-pixel top of the drawn game image inside the pane, rounded to a whole
/// pixel. This is what "the frame moved" means numerically.
fn hybrid_anchor(model: &app::engine::ScreenModel, cols: u16, rows: u16) -> i32 {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    #[allow(deprecated)]
    let mut picker = ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 17));
    picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
    state.game_picker = Some(picker);
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
    let anchor = state.graphics_render.borrow().last_v6_map.as_ref().map(|m| m.img_y.round() as i32);
    anchor.expect("the hybrid path records a v6 click map")
}

/// SQ-0571(c): none of Arthur's full-screen takeovers may MOVE the frame when they
/// open or close.
///
/// Each of them resizes win0 so its bottom reaches the native screen bottom (400):
/// `map` grows it to 584×192, and the F6 text page opens it at 640×384.
/// `hybrid_bottom_plan` read a story window reaching the bottom as an enclosing
/// frame and — with no full-height side ART to stretch, since Arthur's borders are
/// drawn into the full-screen window 7 and erased by these very screens — fell back
/// to `Letterbox`, whose CENTRED vertical offset pushed the whole screen half the
/// letterbox slack down the pane (F6: 193 device px, ~11 rows). Dismissing the
/// screen shrank win0 back below the bottom, flipped the plan to `Extend`, and
/// everything jumped to the top. Where the frame sits must not depend on the story
/// window's height, so that arm now top-anchors too.
///
/// Asserts the recorded letterbox anchor is unchanged across the transition, and
/// that it is the pane top — a centred offset is exactly the defect.
#[test]
fn arthur_screen_swaps_do_not_move_the_frame() {
    // `None` → the `map` command; `Some(t)` → a line read terminated by function
    // key ZSCII `t` (F1 = 133, so F3..F6 are 135..138) — how Arthur's keypad
    // screens are actually invoked.
    for (label, term) in [
        ("map command", None),
        ("F3", Some(135u8)),
        ("F4", Some(136)),
        ("F5", Some(137)),
        ("F6", Some(138)),
    ] {
        let Some(mut session) = arthur_at_status() else { return };
        match term {
            None => {
                let _ = session.submit("map");
            }
            Some(t) => {
                let _ = session.submit_line_with_terminator("", t);
            }
        }
        let shown = hybrid_anchor(&session.screen(), 96, 51);
        let _ = session.submit("");
        let dismissed = hybrid_anchor(&session.screen(), 96, 51);
        assert_eq!(
            shown, dismissed,
            "{label}: the frame sits at the same place shown and dismissed \
             (shown img_y={shown}, dismissed img_y={dismissed})"
        );
        assert_eq!(shown, 0, "{label}: the frame is anchored to the pane TOP, not centred (img_y={shown})");
    }
}

// ── the MACINTOSH press, in RASTER (SQ-1052) ─────────────────────────────────

/// Arthur off `InfocomMasterpieces.img`, the Macintosh compilation volume, drawn
/// with the PROPORTIONAL body face — the configuration that reproduces SQ-1052.
/// The bare `.z6` under `--interpreter 3` renders the same bar correctly and
/// cannot reproduce it at all.
///
/// # This case cannot run everywhere, and says so rather than pretending
///
/// The volume's OWN face is `FONT` 524, a fixed 7x15 — a fixed pen joins nothing,
/// so the defect does not exist under it. What the machine actually drew with is
/// **Geneva**, and Geneva ships with the machine and with no game (SQ-1036): it
/// lives in a System file that no medium under `stories/` carries and the repo
/// cannot redistribute. So the pen this needs comes from a boot disk in the
/// player's own `~/.lanthorn/`, and where there is none the case **skips with a
/// message** instead of asserting something untrue about the volume.
///
/// A synthetic face is not a substitute here and was tried: with different
/// advances the GAME lays its bar out differently — 76 runs against the real 123,
/// and a padding chain that stops short of the frame art — so the frame under test
/// is no longer the frame that was reported. The RULE is pinned unconditionally
/// instead, by
/// `render::v6_layout::tests::a_joined_reverse_chain_resolves_its_block_per_glyph_not_per_run`,
/// which builds its own canvas and runs on every machine. This is that pair's
/// real-media half: it proves a shipped game publishes the shape.
///
/// Booted the way `startup.rs` boots (SQ-0901) — the profile from the medium, the
/// face cascade off that medium and the player's disks, and every per-machine fact
/// through one [`app::machine_boot::MachineBoot`].
///
/// **Turn count: 12** blank lines / returns from cold, which answers the restore
/// question and lands in the Churchyard with the bar painted (SQ-0883 — say how you
/// got to a frame, because a frame is a fixture).
fn mac_arthur_at_status(honor: bool) -> Option<(app::session::GameSession, app::native_font::TextFace)> {
    const ENTRY: &str = "InfocomMasterpieces/ARTHUR FOLDER/STORY.DATA";
    let path = stories_dir().join("InfocomMasterpieces.img");
    if !path.is_file() {
        eprintln!("SKIP: gitignored compilation volume missing at {}", path.display());
        return None;
    }
    let (profile, source) =
        app::interpreter::InterpreterProfile::resolve_with_source(&path, None, None, None);
    assert_eq!(profile, app::interpreter::InterpreterProfile::Macintosh, "the volume names the machine");
    let bytes = match app::hints::load_mounted_story_from(&path, Some(ENTRY)).ok()?.0 {
        app::hints::LoadedStory::ZCode(b) => b,
        other => panic!("Arthur is Z-code on this volume, got {other:?}"),
    };
    let mut picts = PictSource::resolve_with_override(&path, app::graphics::PictureOverride::Unset, Some(ENTRY));
    let picture_dims = picts.all_pict_dims();
    let honoured = honor && !picts.declines_game_colours(profile.default_colours());
    // The player's own media, deliberately: see the header. `UserDisks::new` reads
    // `~/.lanthorn/` whatever key it is given, so this is the real cascade.
    let disks = app::system_fonts::UserDisks::new("");
    let faces = app::native_font::resolve(&app::native_font::FaceRequest {
        story_path: &path,
        entry: Some(ENTRY),
        profile,
        source,
        art_scale: picts.art_scale(),
        disks: Some(&disks),
    });
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        honoured.then(|| profile.default_colours()).flatten(),
        true,
        faces,
        profile.palette(),
        None,
    );
    assert_eq!(boot.cell, zvm::interpreter::MACINTOSH_V6_CELL, "the Macintosh's 7x15 cell");
    let face = boot.text_face();
    if !face.proportional() {
        eprintln!(
            "SKIP: no Macintosh System disk in ~/.lanthorn/, so the body face is the \
             volume's fixed FONT 524 and no run chain forms. The rule this case covers \
             is pinned unconditionally by render::v6_layout::tests::\
             a_joined_reverse_chain_resolves_its_block_per_glyph_not_per_run.",
        );
        return None;
    }
    let mut session = app::session::GameSession::new_for_machine(
        bytes, honoured, false, false, picture_dims, None, None, &boot,
    )
    .expect("Arthur boots off the Macintosh volume");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
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
    Some((session, face))
}

/// (SQ-1052) RASTER, Macintosh: the score bar is one unbroken reversed ribbon
/// across its own window — not the location alone with the rest of the row blank.
///
/// The defect was a granularity one. A v6 grid publishes ONE RUN PER CHARACTER and
/// SQ-1009 joins those runs into lines for the pen; `region_has_opaque` — "is ANY
/// pixel under this run opaque?" — then stopped being asked about a character cell
/// and started being asked about an 88-character chain, which found the frame art
/// and took SQ-0487's no-block arm for the whole bar.
///
/// FALSIFY by putting the probe back on the run (`region_has_opaque(&art, px0, py,
/// cell.run_px(&t.text).max(font_w), font_h)`, hoisted out of the glyph loop): the
/// ribbon collapses to 13% of its window — ` Churchyard` and a two-pixel sliver —
/// which is the reported screen.
///
/// Both `honor_game_colours` modes, per the standing rule: the ribbon is the
/// resolved page INK either way, and only one of them was ever exercised.
#[test]
fn mac_arthur_raster_score_bar_is_one_ribbon_not_the_location_alone() {
    for honor in [true, false] {
        let Some((session, face)) = mac_arthur_at_status(honor) else { return };
        let model = session.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };

        // Non-vacuity: this frame really is the score bar — one grid window whose
        // runs are ALL reverse-video, carrying both the location and the date.
        let bar = items
            .iter()
            .find(|pw| matches!(&pw.node, WinNode::Grid(g) if g.px_texts.len() > 10))
            .expect("the score bar window is on this frame");
        let WinNode::Grid(g) = &bar.node else { unreachable!() };
        let line: String = {
            let mut v: Vec<_> = g.px_texts.iter().collect();
            v.sort_by_key(|t| (t.y, t.x));
            v.iter().map(|t| t.text.as_str()).collect()
        };
        assert!(line.contains("Churchyard"), "honor={honor}: the location is on the bar: {line:?}");
        assert!(line.contains("Compline"), "honor={honor}: and the date: {line:?}");
        assert!(
            g.px_texts.iter().all(|t| t.style & 1 != 0),
            "honor={honor}: every run is reversed — this is the pure-reverse bar the fill is about"
        );

        let mut state = app::state::AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.v6_render = app::config::V6RenderMode::Raster;
        state.config.honor_game_colours = honor;
        state.v6_text = face;
        let cell = state.v6_text.cell();
        let native = app::render::v6_layout::native_extent(items, &state.v6_text);
        let layout = app::render::v6_layout::classify_windows(items, cell);
        let (canvas, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);

        // The bar's own top scan line, across the bar's own window. The ribbon is
        // whatever colour the location's block is — asked of the pixels rather than
        // pinned, so the case says "one bar" and not "this theme".
        let py = u32::from(g.px_texts.iter().map(|t| t.y.max(1)).min().expect("runs")) - 1;
        let x0 = u32::from(bar.x_px);
        let x1 = x0 + u32::from(bar.w_px);
        let ribbon = *canvas.get_pixel(x0, py);
        let solid = (x0..x1).filter(|&x| *canvas.get_pixel(x, py) == ribbon).count();
        let width = (x1 - x0) as usize;
        assert!(
            solid * 100 >= width * 95,
            "honor={honor}: the bar is one ribbon across its window — {solid}/{width} px are \
             {ribbon:?} at native row {py} (pre-SQ-1052 this was 13%: the location and a sliver)"
        );
    }
}
