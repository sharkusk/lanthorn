//! Lane H (hybrid render) end-to-end acceptance: boot the real Zork0 (v6/
//! graphical), build its `Layered` screen model, and drive the story-pane
//! renderer in BOTH v6 render modes.
//!
//! Hybrid mode draws the chrome as a scaled pixel ring around a terminal story
//! viewport, then renders the story window as REAL terminal text (crisp,
//! selectable) inside it — so the transcript appears as ordinary buffer cells and
//! the story-pane render reports real scroll metrics. Raster mode keeps today's
//! behavior: the whole pane is one rasterized pixel image, no terminal transcript.
//!
//! The story asset is gitignored (large, local-only), so this test **skips
//! cleanly** when absent — mirrors `zork0_v6_windows.rs`.
//!
//! **Colour mode: `honor_game_colours = true`** — the app's shipped config
//! default, so these render assertions are made in the mode real players run.
//! (Before SQ-0532 wave 4 every v6 smoke booted with the game's colours
//! DECLINED, which is exactly why three colour-driven render regressions
//! shipped unseen. The theme-only `false` path is covered by the paired cases
//! in `v6_game_colour_regression.rs`.)
//!
//! **Palette: `Standard`, set rather than assumed** (SQ-0956). Every case here
//! resolves colour numbers through the process-global palette, and until now the
//! suite neither set it nor took the lock — it inherited whatever the last suite
//! in this group binary left behind, which was harmless only for as long as every
//! one of them happened to leave `Standard` there. `v6_cga_stencil_page` boots a
//! DOS press, whose palette is the IBM PC's YZIP table, and under `cargo test` —
//! one process, parallel threads — this suite started reading it: measured, a
//! chrome band that must survive a status change came back re-hashed, and the
//! failure appeared in THIS file for a change made in another. That is the
//! reader's half of SQ-0905. Since SQ-1393 the palette is a `Machine` field and
//! `new_with_trace` presents §8.3.1's own table outright, so a bare story's colour
//! numbers resolve through the table this suite has always assumed and no sibling
//! can move them.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::GameSession;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot Zork0 through boot + boot-picture flush, exactly like `zork0_v6_windows.rs`.
/// Returns `None` (with a SKIP note) when the gitignored story is absent.
fn boot_zork0() -> Option<GameSession> {
    // The bare story file names no machine, so its colour numbers resolve through
    // ZMSD §8.3.1's own table — which is what every assertion below was written
    // against, and what `new_with_trace` presents (SQ-1393).
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Zork0 (v6) should load and boot without a ZError");
    assert!(!session.quit, "Zork0 quit during boot");
    assert!(session.machine.fault_trace.is_none(), "Zork0 faulted during boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    Some(session)
}

/// Build an AppState wired for a v6 headless render: a halfblocks picker (no
/// terminal query), terminal-default theme, and the given render mode.
fn render_state(mode: app::config::V6RenderMode) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = mode;
    state
}

/// The same state, forced onto the CELL path. SQ-0895 removed `frameless`, which
/// was the deliberate way to ask for it; dropping the picker is the substitute
/// whose only effect is the one frameless contributed — draw no game image.
fn cell_path_state() -> app::state::AppState {
    let mut state = render_state(app::config::V6RenderMode::Hybrid);
    state.game_picker = None;
    state
}

#[test]
fn zork0_hybrid_renders_story_as_terminal_text() {
    let Some(session) = boot_zork0() else { return };
    let model = session.screen();
    assert!(matches!(model.root, WinNode::Layered(_)), "v6 root is a layered composite");

    let mut state = render_state(app::config::V6RenderMode::Hybrid);
    // Seed a couple of transcript lines so the story viewport has real text to draw.
    state.push_transcript("West of House");
    state.push_transcript("You are standing in an open field west of a white house.");

    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    let metrics = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    // Hybrid renders the story window as terminal cells → the transcript publishes
    // its geometry into an inset viewport (strictly inside the pane, so the chrome
    // ring surrounds it).
    let geom = state
        .transcript_geom
        .get()
        .expect("hybrid renders the story as a terminal transcript");
    let vp = geom.area;
    assert!(vp.width < area.width && vp.height < area.height, "story viewport is inset in the chrome ring: {vp:?}");
    assert!(metrics.viewport_rows > 0, "story pane reports real viewport rows");

    // Cell-dump of the hybrid render (the Task H3 deliverable). Chrome-ring cells
    // are painted by the image protocol (halfblock glyphs here); the inset
    // viewport carries the crisp terminal transcript.
    eprintln!("--- Zork0 HYBRID cell-dump ({}x{}) — viewport {:?} ---", area.width, area.height, vp);
    for y in area.y..area.bottom() {
        let mut row = String::new();
        for x in area.x..area.right() {
            let s = buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default();
            row.push(s.chars().next().unwrap_or(' '));
        }
        eprintln!("{y:2}|{row}|");
    }

    // The seeded story text lands somewhere in the viewport as real cells.
    let found = (vp.y..vp.bottom()).any(|y| {
        let row: String = (vp.x..vp.right())
            .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        row.contains("West of House") || row.contains("white house")
    });
    assert!(found, "seeded story text renders as terminal cells inside the viewport");
}

#[test]
fn zork0_raster_mode_publishes_scroll_geometry() {
    let Some(session) = boot_zork0() else { return };
    let model = session.screen();

    let mut state = render_state(app::config::V6RenderMode::Raster);
    state.push_transcript("West of House");

    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    let metrics = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    // Raster mode still rasterizes the whole pane as one pixel image, but it now
    // REPORTS the story box's scroll geometry (SQ-0455) so the shared scroll
    // keybindings and the [more] pager (SQ-0404) engage. The reported viewport is
    // the story box's raster body rows (never the default full-pane height), and
    // geometry is published (approximate mouse mapping over the pixel-scaled text).
    assert!(state.transcript_geom.get().is_some(), "raster mode publishes scroll geometry");
    assert!(metrics.viewport_rows > 0, "raster reports real story-box viewport rows");
    assert!(
        metrics.viewport_rows < area.height,
        "the story box is smaller than the full pane (chrome ring reserved): {}",
        metrics.viewport_rows
    );
}

/// SQ-0467: frameless mode lays the v6 status band out as a classic full-width
/// status line (the "anchored bar"), not a 40-cell postage stamp squatting in the
/// left half of an 80-col pane. Zork0's banner (native 320px / 40 cells) carries a
/// left location run and right-side Score/Moves runs, so on an 80-col pane the
/// location must sit flush at col 0 and the Score/Moves must land flush right — not
/// bunched mid-pane as the old per-run cell-quantization produced.
#[test]
fn zork0_frameless_status_band_is_anchored_full_width() {
    let Some(session) = boot_zork0() else { return };
    let model = session.screen();

    let state = cell_path_state();
    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    // The band occupies the top rows; capture them as text.
    let band: Vec<String> = (0..4)
        .map(|y| (0..area.width).map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' ')).collect())
        .collect();
    eprintln!("--- Zork0 FRAMELESS status band (80 cols) ---");
    for (y, r) in band.iter().enumerate() {
        eprintln!("{y}|{r}|");
    }

    // Location is flush LEFT: row 0 begins with a printed glyph at col 0.
    assert_ne!(band[0].chars().next(), Some(' '), "location run is flush at col 0, not indented: {:?}", band[0]);

    // Score/Moves are anchored flush RIGHT: the row carrying "Score:" ends its
    // painted text within the last two columns of the 80-col pane (old quantized
    // layout left them mid-pane with dead space to the right).
    let score_row = band.iter().find(|r| r.contains("Score:")).expect("a status row carries the Score label");
    let last_glyph = score_row.rfind(|c: char| c != ' ').expect("score row has printed text");
    assert!(last_glyph >= area.width as usize - 2, "Score/Moves flush right (last glyph at col {last_glyph} of {}): {score_row:?}", area.width - 1);
    // And it did NOT squat in the left 40 columns (the reported bug).
    assert!(!score_row[41..].trim().is_empty(), "right-side status reaches past the old 40-cell stamp: {score_row:?}");
}

/// (SQ-0500 pin) Zork0's status ("Moves:"/"Score:") sits ON opaque banner art, so
/// in HYBRID mode its chrome band stays the pixel RING — the strip is classified
/// Art and the artwork goes up as an image, unlike Arthur's clear-interior status
/// row, which becomes a Text strip. Guards the "art behind → keep the ring" branch
/// of the band decomposition.
///
/// SQ-0944 made the MEDIUM of the text on that band depend on the backend, and
/// this pin now says so on both. The strip is an Art strip on either — that is the
/// branch being guarded, and it has not moved. What changed is that half-blocks
/// draws the labels sitting on it as glyphs, because a rasterised 8x16 glyph is
/// 8x2 there and unreadable, while kitty keeps them in the raster, which is both
/// faithful and the only thing that works: a glyph printed into a cell a virtual
/// placement covers deletes the image rather than layering over it.
#[test]
fn zork0_hybrid_status_on_art_stays_in_the_ring() {
    use ratatui_image::picker::ProtocolType;
    let Some(session) = boot_zork0() else { return };
    let model = session.screen();

    for protocol in [ProtocolType::Kitty, ProtocolType::Halfblocks] {
        let glyphs_over_art = protocol == ProtocolType::Halfblocks;
        let mut state = render_state(app::config::V6RenderMode::Hybrid);
        if let Some(p) = state.game_picker.as_mut() {
            p.set_protocol_type(protocol);
        }
        state.push_transcript("West of House");
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

        let screen: String = (area.y..area.bottom())
            .map(|y| {
                (area.x..area.right())
                    .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                    .collect::<String>()
                    + "\n"
            })
            .collect();
        for label in ["Moves:", "Score:"] {
            assert_eq!(
                screen.contains(label),
                glyphs_over_art,
                "{protocol:?}: {label} rides the banner's ART band either way — rasterised into \
                 it where a glyph cannot sit over a placement, stamped as glyphs where one \
                 can:\n{screen}"
            );
        }
    }
}

/// (SQ-0514 chrome-band freshness pin) Zork0 rasterizes its Score/Moves status into
/// the chrome canvas every turn, so a turn changes only the TOP status band's native
/// pixels. In HYBRID mode the flank (side/bottom) ring bands must then stay FRESH —
/// their cached uploads reused, not re-encoded. Before the fix the freshness hash
/// covered the WHOLE canvas, so any status change staled every band (the ~377ms
/// per-turn stall). Here we render, take a turn (mutating the banner), render again,
/// and assert at least one band re-encoded while at least one other stayed fresh.
#[test]
fn zork0_hybrid_status_change_keeps_flank_bands_fresh() {
    let Some(mut session) = boot_zork0() else { return };
    let mut state = render_state(app::config::V6RenderMode::Hybrid);
    state.push_transcript("West of House");
    let area = Rect::new(0, 0, 80, 30);

    // Frame 1: populate the per-band cache.
    let model = session.screen();
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    let before = state.graphics_render.borrow().chrome_band_hashes();
    assert!(before.len() >= 2, "hybrid Zork0 draws multiple chrome bands, got {}", before.len());

    // Take a turn so the banner (Score/Moves) re-rasterizes into the chrome canvas.
    let _ = session.submit("look");
    let model = session.screen();
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    let after = state.graphics_render.borrow().chrome_band_hashes();

    // Every band that survived the turn (same rect) is compared; the fix guarantees
    // at least one flank band kept its hash (fresh) even though the banner changed.
    let common: Vec<_> = before.keys().filter(|k| after.contains_key(*k)).collect();
    assert!(!common.is_empty(), "at least one band rect persists across the turn");
    let fresh = common.iter().filter(|k| before[**k] == after[**k]).count();
    assert!(
        fresh > 0,
        "at least one chrome band stays fresh across a status change (SQ-0514): \
         {fresh}/{} bands unchanged",
        common.len()
    );
}

/// (SQ-0511 enclosed-frame reclaim — Zork0 pin) Zork0's frame ENCLOSES the story to
/// the native screen bottom (story bottom 398 of 400) and is flanked by full-height
/// side art. At a TALL pane the `Frame` plan now RECLAIMS the letterbox slack: the
/// top banner stays uniform-scaled and pane-top-anchored, the story viewport grows
/// from just under it to the pane BOTTOM at constant width, and the ornate side
/// flanks STRETCH vertically to fill the reclaimed space (no longer a centred
/// letterbox). Pins the new geometry exactly (halfblocks 10×20 cells, native
/// 640×400, scale 1.40625, off_y 0). One column is reserved for the scrollbar.
#[test]
fn zork0_hybrid_tall_pane_frame_reclaim() {
    use ratatui::style::Color;
    let Some(session) = boot_zork0() else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };

    let mut state = render_state(app::config::V6RenderMode::Hybrid);
    state.push_transcript("West of House");
    let area = Rect::new(0, 0, 90, 40);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    let vp = state.transcript_geom.get().expect("hybrid renders the story as a transcript").area;
    let _ = items;

    // Top-anchored under the banner band (native y=78 → dev 109.7 → row 6), extended
    // to the pane bottom (row 40), constant width (native 86..554 → cols 13..77,
    // one reserved for the scrollbar → width 63).
    assert_eq!(vp, Rect::new(13, 6, 63, 34), "Zork0 top-anchors + extends to the pane bottom (Frame reclaim)");
    assert!(vp.y > 0 && vp.y <= 6, "story top-anchored just under the banner band, not centred: {vp:?}");
    assert_eq!(vp.bottom(), area.bottom(), "story viewport extends to the pane bottom (slack reclaimed): {vp:?}");

    // A cell carries a chrome-ring image when it is a halfblock glyph or has a
    // concrete (non-Reset) background — the flank bands upload as image protocols.
    let is_img = |x: u16, y: u16| -> bool {
        let c = buf.cell((x, y)).unwrap();
        let s = c.symbol();
        s == "\u{2580}" || s == "\u{2584}" || c.bg != Color::Reset
    };
    // The side flanks STRETCH into the reclaimed space: at a deep row well below the
    // old (centred) art extent, both flanks still carry ring image cells — the OLD
    // clip-to-art-extent path left these rows the bare backdrop.
    for dy in [vp.bottom() - 3, vp.bottom() - 1] {
        assert!((0..vp.x).any(|x| is_img(x, dy)), "left flank stretched down to row {dy} (reclaimed space)");
        assert!((vp.right()..area.width).any(|x| is_img(x, dy)), "right flank stretched down to row {dy}");
        // Squares-and-seams at the fractional scale 1.40625: the left flank's
        // rightmost image column abuts the viewport (== vp.x-1) — no gap, no overlap
        // into the story columns — and the right flank stays at/after vp.right().
        let lmax = (0..vp.x).rev().find(|&x| is_img(x, dy)).expect("left flank has ring cells");
        assert_eq!(lmax, vp.x - 1, "left flank abuts the story viewport with no seam at row {dy} (lmax {lmax}, vp.x {})", vp.x);
        let rmin = (vp.right()..area.width).find(|&x| is_img(x, dy)).expect("right flank has ring cells");
        assert!(rmin >= vp.right(), "right flank does not overlap the story viewport at row {dy} (rmin {rmin}, vp.right {})", vp.right());
    }
}

/// Drive Zork0 to the Great Hall: survive Megaboz's curse under the table, then
/// advance through the intro until the room announces itself. Returns `None`
/// when the story is absent.
fn zork0_in_great_hall() -> Option<GameSession> {
    use app::session::InputKind;
    let mut session = boot_zork0()?;
    let _ = session.take_transcript();
    let mut lines = ["get under table", "wait", "wait", "wait", "wait", "wait"].into_iter();
    for _ in 0..16 {
        let _ = match session.pending_input() {
            InputKind::Line => session.submit(lines.next().unwrap_or("wait")),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
    }
    for _ in 0..6 {
        let r = match session.pending_input() {
            InputKind::Char => session.submit_char(13),
            _ => session.submit("look"),
        };
        if r.transcript.contains("Great Hall") {
            break;
        }
    }
    Some(session)
}

/// Drive Zork0 to its on-screen MAP: from the Great Hall, `map` and answer its
/// "Are you ready to see the map (y or n)?" prompt with `y`. Returns `None` when
/// the story is absent.
fn zork0_showing_map() -> Option<GameSession> {
    let mut session = zork0_in_great_hall()?;
    let r = session.submit("map");
    assert!(
        r.transcript.contains("ready to see the map"),
        "expected the map's confirmation prompt, got: {:?}",
        r.transcript
    );
    let _ = session.submit_char(b'y');
    Some(session)
}

/// SQ-0570: Zork0's on-screen map must actually be VISIBLE in hybrid mode.
///
/// The map is the exact inverse of the title splash. The splash calls
/// `split_window(400)`, which makes window 1 the whole screen and COLLAPSES window
/// 0, so hybrid carves no story viewport over it (SQ-0497). The map instead GROWS
/// window 0 to the full screen `(0,0) 640×400` and paints the map into the
/// full-screen graphics window beneath it. Hybrid therefore made the story viewport
/// the ENTIRE pane, which leaves `chrome_bands` empty — the map was never uploaded
/// at all and the transcript painted over the whole screen. The reported symptom was
/// a sudden drop into frameless mode: no frame, no map, only story text. Raster mode
/// was unaffected, which is the tell: the composite was right, the hybrid overlay
/// was covering it.
///
/// Such a frame has no ring, so hybrid now falls through to the raster composite for
/// it. Asserts the model precondition, that the pane is fully imaged with no
/// transcript stamped over it, and that hybrid agrees with raster — which the
/// reporter confirmed already renders this screen correctly.
#[test]
fn zork0_hybrid_shows_the_full_screen_map() {
    let Some(session) = zork0_showing_map() else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };

    // Precondition: window 0 grown to the whole screen, opaque graphics behind it.
    let story = items
        .iter()
        .find(|pw| matches!(&pw.node, WinNode::Buffer(_)))
        .expect("the map screen keeps a story window (it does NOT collapse it)");
    assert_eq!(
        (story.x_px, story.y_px, story.w_px, story.h_px),
        (0, 0, 640, 400),
        "the map grows window 0 to the full screen — the inverse of the splash's collapse"
    );
    let coverage = items
        .iter()
        .filter_map(|pw| match &pw.node {
            WinNode::Graphics(gw) if pw.x_px == 0 && pw.y_px == 0 && gw.canvas.width() >= 640 && gw.canvas.height() >= 400 => {
                let total = gw.canvas.pixels().count();
                let opaque = gw.canvas.pixels().filter(|p| p.0[3] >= 128).count();
                Some(opaque as f32 / total.max(1) as f32)
            }
            _ => None,
        })
        .fold(0.0f32, f32::max);
    eprintln!("full-screen graphics coverage: {coverage:.4}");
    assert!(coverage > 0.95, "the map is painted across a full-screen graphics window (coverage {coverage:.4})");

    let area = Rect::new(0, 0, 96, 40);
    let mut rendered = Vec::new();
    for mode in [app::config::V6RenderMode::Hybrid, app::config::V6RenderMode::Raster] {
        let mut state = render_state(mode);
        // Real transcript lines — the text that used to be painted over the map.
        state.push_transcript("Great Hall");
        state.push_transcript("You are in the great hall of the castle.");
        let mut buf = Buffer::empty(area);
        let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

        let text: String = (0..area.height)
            .map(|y| {
                (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !text.contains("great hall of the castle"),
            "{mode:?}: the transcript must not be stamped over the map\n{text}"
        );
        let painted = (0..area.height)
            .flat_map(|y| (0..area.width).map(move |x| (x, y)))
            .filter(|&(x, y)| buf.cell((x, y)).unwrap().bg != ratatui::style::Color::Reset)
            .count();
        assert_eq!(
            painted,
            area.width as usize * area.height as usize,
            "{mode:?}: the map fills the whole pane"
        );
        rendered.push(buf);
    }
    assert!(
        rendered[0] == rendered[1],
        "hybrid must render the map frame exactly as raster does (raster was already correct)"
    );
}

/// Drive Zork0 to the Gallery and `look at rebus`: the game grows window 0 to
/// the full screen and paints the rebus picture across virtually all of it,
/// then waits for a keypress. Returns `None` when the story is absent.
fn zork0_looking_at_rebus() -> Option<GameSession> {
    let mut session = zork0_in_great_hall()?;
    walk_to_rebus(&mut session);
    Some(session)
}

/// From the Great Hall (or anywhere the Balcony stairs are reachable), walk to
/// the Gallery and `look at rebus`, leaving the game on its keypress wait.
fn walk_to_rebus(session: &mut GameSession) {
    let r = session.submit("up");
    assert!(r.transcript.contains("Balcony"), "up from the Great Hall is the Balcony: {:?}", r.transcript);
    let r = session.submit("south");
    assert!(r.transcript.contains("Gallery"), "south from the Balcony is the Gallery: {:?}", r.transcript);
    let _ = session.submit("look at rebus");
    assert!(
        matches!(session.pending_input(), app::session::InputKind::Char),
        "the rebus screen waits for a keypress"
    );
}

/// SQ-0578: a full-screen picture must not squeeze the transcript into a
/// one-character column.
///
/// `look at rebus` grows window 0 over the whole screen (the map's takeover
/// shape) but its art leaves a degenerate 0x80 sliver unpainted, so
/// `story_clear_native` returns a rect too small to hold a single 8x16 text
/// cell. The raster composite pinned that zero width to ONE column
/// (`cols = (sw/8).max(1)`), re-wrapped the whole transcript a character per
/// line, and armed the [more] pager with a 4-row page — draining it took
/// hundreds of keypresses. The reported symptom: picture redraws, a thin
/// single column of text appears with [more], and a key must be held for a
/// very long time to progress.
///
/// The composite now skips the story stamp when no full cell fits: the picture
/// ships alone with NO scroll metrics, so the pager treats the screen like the
/// no-story-window case (nothing to page) and one keypress returns to the
/// normal frame.
#[test]
fn zork0_rebus_picture_shows_without_a_text_column() {
    let Some(session) = zork0_looking_at_rebus() else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };

    // Precondition: the takeover shape — window 0 grown to the whole screen.
    let story = items
        .iter()
        .find(|pw| matches!(&pw.node, WinNode::Buffer(_)))
        .expect("the rebus screen keeps a story window");
    assert_eq!(
        (story.x_px, story.y_px, story.w_px, story.h_px),
        (0, 0, 640, 400),
        "the rebus grows window 0 to the full screen"
    );

    // Precondition that separates this from the map: the art leaves a clear
    // sliver too small for even one 8x16 text cell. (The map paints edge to
    // edge; the rebus is what exposed the degenerate-sliver stamp.)
    use app::render::v6_layout as v6;
    let native = v6::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = v6::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let state = render_state(app::config::V6RenderMode::Hybrid);
    let canvas = v6::build_chrome_canvas(
        &layout.chrome,
        native,
        image::Rgba([220, 220, 220, 255]),
        image::Rgba([0, 0, 0, 255]),
        &state.colors,
        v6::TextLayer::All,
        &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT),
    );
    let clear = v6::story_clear_native(layout.story, &canvas).expect("story window present");
    eprintln!("story_clear_native: {clear:?}");
    assert!(
        clear.2 < 8 || clear.3 < 16,
        "the rebus art occludes the story interior down to a sub-cell sliver: {clear:?}"
    );

    // The composite must ship the picture alone: no story stamp, no metrics.
    let (_, rm) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
    assert!(
        rm.is_none(),
        "a sub-cell story interior must not produce raster scroll metrics (a 1-column \
         transcript stamp armed a 4-row [more] pager): {rm:?}"
    );

    // End to end through the pane render, with a real transcript backlog (the
    // text that used to wrap a character per line): the pager must see the
    // full-pane fallback viewport, not a sliver, and nothing to page.
    let mut state = render_state(app::config::V6RenderMode::Hybrid);
    for i in 0..40 {
        state.push_transcript(&format!("transcript backlog line {i} with enough words to wrap"));
    }
    let area = Rect::new(0, 0, 96, 40);
    let mut buf = Buffer::empty(area);
    let metrics = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    assert_eq!(
        (metrics.viewport_rows, metrics.total_rows),
        (area.height, 0),
        "the rebus screen reports the no-story fallback metrics — nothing for the [more] pager to drain"
    );
    assert!(
        !metrics.transcript_surface,
        "a picture-takeover frame reports NO transcript surface, so the frame loop skips the \
         scroll clamp and pager-baseline bookkeeping (a zero baseline re-paged the whole \
         backlog on return)"
    );
    let text: String = (0..area.height)
        .map(|y| {
            (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !text.contains("backlog"),
        "the transcript must not be stamped over the rebus picture\n{text}"
    );
}

/// SQ-0578 (flash): entering one full-screen picture must never show a stale
/// raster composite from an EARLIER screen while the new encode runs.
///
/// The raster path encodes off-thread and redraws "the last-ready composite
/// until the worker lands" — right for a burst of the same screen, but the
/// hybrid path only falls through to raster for takeovers, so its last-ready
/// composite could be minutes old: booting showed the title splash for a split
/// second when the rebus came up, and `map` → gameplay → `look at rebus`
/// flashed the map. Hybrid band frames now drop the cached composite, and a
/// raster frame with no composite encodes synchronously — the new picture is
/// on screen the same frame it is entered.
///
/// One shared render state across three frames, exactly like the live pane:
/// the map (raster), the restored gameplay frame (hybrid bands), the rebus
/// (raster). The rebus frame must not reproduce the map frame's cells.
#[test]
fn zork0_rebus_after_map_never_flashes_the_stale_composite() {
    let Some(mut session) = zork0_showing_map() else { return };
    let mut state = render_state(app::config::V6RenderMode::Hybrid);
    let area = Rect::new(0, 0, 96, 40);
    let render = |session: &GameSession, state: &app::state::AppState| {
        let model = session.screen();
        let mut buf = Buffer::empty(area);
        let _ = app::render::screen::render_story_pane(&model, false, None, state, area, &mut buf);
        buf
    };

    // Frame 1: the map — raster fall-through; with no prior composite the
    // encode is synchronous, so the map is really in this buffer.
    let map_buf = render(&session, &state);
    let img_cells = |buf: &Buffer| {
        (0..area.height)
            .flat_map(|y| (0..area.width).map(move |x| (x, y)))
            .filter(|&(x, y)| buf.cell((x, y)).unwrap().symbol() == "\u{2580}")
            .count()
    };
    assert!(img_cells(&map_buf) > 0, "the map composite really rendered (halfblock cells present)");

    // Frame 2: dismiss the map — an ordinary gameplay frame takes the hybrid
    // band path (story window back inside the frame), invalidating the cache.
    let _ = session.submit_char(b' ');
    {
        let model = session.screen();
        let app::engine::WinNode::Layered(items) = &model.root else { panic!("layered") };
        let story = items.iter().find(|pw| matches!(&pw.node, WinNode::Buffer(_))).expect("story window");
        assert!(story.w_px < 640, "gameplay restores the framed story window: {}px wide", story.w_px);
    }
    state.push_transcript("Great Hall");
    let _ = render(&session, &state);

    // Frame 3: the rebus. Its composite must be freshly encoded — not the map.
    walk_to_rebus(&mut session);
    let rebus_buf = render(&session, &state);
    assert!(img_cells(&rebus_buf) > 0, "the rebus composite really rendered (halfblock cells present)");
    assert!(
        rebus_buf != map_buf,
        "the rebus frame must not redraw the stale map composite while its own encode runs"
    );
}
