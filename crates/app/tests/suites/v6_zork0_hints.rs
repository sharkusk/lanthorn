//! Zork Zero InvisiClues hint-menu regression (SQ-0456 follow-up).
//!
//! The hint menu clears window 0's WRAPPING attribute (window_style op 2) and
//! paints topics via set_cursor, one row per item. Before the win0 paint-mode
//! routing, that output went through the flat transcript: every topic strung
//! together on one line, and every menu navigation appended another copy.
//! Skip-if-missing (gitignored story).

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

#[test]
fn zork0_hint_menu_paints_topics_by_row_and_keeps_transcript_clean() {
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, false, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Zork0 (v6) should load and boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();

    let mut result = session.submit("hint");
    assert!(result.transcript.contains("Do you still want a hint?"), "hint warning prompt");
    assert_eq!(session.pending_input(), InputKind::Char);
    result = session.submit_char(b'y');
    assert!(result.fault.is_none(), "entering the hint menu faulted: {:?}", result.fault);

    // Topics paint into window 0 as positioned runs, one 8-px row per item —
    // NOT into the transcript (which strung them all together before).
    assert!(
        !result.transcript.contains("PROLOGUE"),
        "hint topics must not stream into the transcript: {:?}",
        result.transcript
    );
    let topic_rows = |session: &GameSession| -> std::collections::BTreeMap<u16, String> {
        let mut rows: std::collections::BTreeMap<u16, String> = Default::default();
        for t in &session.machine.screen.v6.as_ref().unwrap().windows[0].texts {
            rows.entry(t.y).or_default().push_str(&t.text);
        }
        rows
    };
    let rows = topic_rows(&session);
    assert!(rows.len() >= 15, "one painted row per topic, got {} rows: {rows:?}", rows.len());
    let all: String = rows.values().cloned().collect::<Vec<_>>().join("\n");
    for topic in ["PROLOGUE", "EAST WING", "GENERAL QUESTIONS", "THE JESTER"] {
        assert!(all.contains(topic), "missing topic {topic:?} in painted rows:\n{all}");
    }
    // Distinct topics live on distinct rows (the strung-together bug had
    // everything on one line).
    let prologue_row = rows.iter().find(|(_, s)| s.contains("PROLOGUE")).map(|(y, _)| *y);
    let jester_row = rows.iter().find(|(_, s)| s.contains("THE JESTER")).map(|(y, _)| *y);
    assert_ne!(prologue_row, jester_row, "topics must occupy different rows");

    // The instruction header paints into window 1.
    let header: String =
        session.machine.screen.v6.as_ref().unwrap().windows[1].texts.iter().map(|t| t.text.as_str()).collect();
    assert!(header.contains("InvisiClues"), "menu header missing: {header:?}");
    assert!(header.contains("N for next item."), "navigation help missing: {header:?}");

    // Menu navigation repaints in place: no transcript output, no fault.
    let nav = session.submit_char(b'n');
    assert!(nav.fault.is_none(), "menu navigation faulted: {:?}", nav.fault);
    assert!(
        nav.transcript.trim().is_empty(),
        "navigation must repaint, not stream: {:?}",
        nav.transcript
    );

    // Q resumes the story: wrapping restored, prose streams to the transcript
    // again at the game's prompt.
    let quit = session.submit_char(b'q');
    assert!(quit.fault.is_none(), "leaving the hint menu faulted: {:?}", quit.fault);
    assert_eq!(session.pending_input(), InputKind::Line, "back at the story prompt");
    let look = session.submit("look");
    assert!(
        look.transcript.contains("Banquet Hall"),
        "post-menu prose must stream to the transcript again: {:?}",
        look.transcript
    );
}

/// SQ-0934: the hint screen's HEADER is drawn as raster, and its topic list as
/// glyphs — the split that makes this screen a ring rather than a page of text.
///
/// **This case used to assert the opposite** and was right to, until the screen
/// changed destination. The menu reached the runs-only arm (SQ-0477), which draws
/// every run as terminal cells and discards every pixel, so the title really was a
/// full-width reverse bar in the cell buffer. That arm threw the game's frame away
/// with it, which is what SQ-0934 fixed: the screen is a ring — 78% of the top band
/// and 70% of each flank opaque, middle 0.0% — and it now takes one.
///
/// So the header moved into the art. `" InvisiClues (tm)"` and the key legend sit
/// ON the banner artwork at native y 1..33. The topic list is the opposite case —
/// the ring's middle is 0.0% opaque, nothing is under it, and it is drawn with
/// glyphs. That is CLAUDE.md's rule for hybrid, applied per strip and per frame.
///
/// **SQ-0944 revisited the header half, and it was half right.** The reason given
/// for rasterising it was "a terminal glyph cannot be drawn over a kitty
/// placement", stated generally. Measured (`pty_oracle.rs`), the true statement is
/// narrower and about lanthorn's placements rather than about kitty: they are
/// VIRTUAL (`U=1`), positioned by `U+10EEEE` placeholder characters, so the image
/// is the cell's content — printing a glyph into a covered cell deletes the image
/// and truncates the rest of that row's run. Under kitty the conclusion therefore
/// stands, and this case still pins it. It does NOT generalise to a backend with
/// no placements at all: on half-blocks the ring stamps these rows as glyphs, in a
/// ground sampled from the art, because a rasterised 8x16 glyph is 8x2 there and
/// the header is simply unreadable. Both are asserted below.
///
/// Skip-if-missing (gitignored story).
#[test]
fn zork0_hint_header_is_raster_on_the_banner_and_the_topics_are_glyphs() {
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, false, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Zork0 (v6) should load and boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    session.submit("hint");
    let entered = session.submit_char(b'y');
    assert!(entered.fault.is_none(), "entering the hint menu faulted: {:?}", entered.fault);

    let model = session.screen();
    for protocol in [ratatui_image::picker::ProtocolType::Kitty, ratatui_image::picker::ProtocolType::Halfblocks] {
        let glyphs_over_art = protocol == ratatui_image::picker::ProtocolType::Halfblocks;
        let mut state = app::state::AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        state.game_picker = Some({
            let mut p = ratatui_image::picker::Picker::halfblocks();
            p.set_protocol_type(protocol);
            p
        });
        state.config.v6_render = app::config::V6RenderMode::Hybrid;
        let area = Rect::new(0, 0, 100, 34);
        let mut buf = Buffer::empty(area);
        let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

        let screen: String = (0..area.height)
            .map(|y| (0..area.width).map(|x| buf.cell((x, y)).map_or(' ', |c| c.symbol().chars().next().unwrap_or(' '))).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");

        // The premise, so a frame that stopped being the menu cannot pass vacuously.
        assert_eq!(state.v6_path_log.borrow().last().map(|(l, _)| l.clone()), Some("hybrid-ring".into()),
            "{protocol:?}: the hint screen takes the ring:\n{screen}");

        // The TOPIC LIST is glyphs on BOTH — nothing is under it to argue with.
        for topic in ["GREAT HALL AREA", "SECRET WING", "GENERAL QUESTIONS"] {
            assert!(screen.contains(topic), "{protocol:?}: {topic:?} must be drawn with glyphs:\n{screen}");
        }

        // The HEADER is on the banner ARTWORK, so its medium follows the backend.
        for header in ["InvisiClues", "N for next item.", "Q to resume story."] {
            assert_eq!(
                screen.contains(header),
                glyphs_over_art,
                "{protocol:?}: {header:?} sits on the banner artwork — it must be rasterised \
                 into the band where a glyph in a covered cell would delete the placement, and \
                 stamped as glyphs where there is no placement to delete:\n{screen}"
            );
        }
    }
}


/// SQ-0934: the hint menu DRAWS the ring, so the ring is continuous across the
/// round trip into the menu and back.
///
/// **What this case used to pin, and where that guarantee lives now.** SQ-0637
/// found that the painted-screen branch dropped the ring for the menu's frames
/// while leaving `hybrid-ring` stamped in `v6_path_log`, so the resume read a stale
/// stamp, every band was a cache hit that sent nothing, and Zork Zero came back
/// from InvisiClues with its frame art missing. The fix was to stamp the painted
/// path; this case was the specimen.
///
/// The menu is not that frame any more — it takes the ring itself, so there is no
/// drop to recover from. The GUARANTEE is untouched and is pinned mechanically
/// rather than through this game:
/// `v6_kitty_graphics::evicting_one_band_makes_the_survivors_re_upload_too` is the
/// same rule without needing a specimen that drops the ring, and
/// `v6_band_placement_lifecycle::an_unchanged_menu_frame_re_uploads_no_band` holds
/// the other side of it.
///
/// What is pinned here instead is the stronger property the change bought: the ring
/// never goes away, so the bands stay warm and the menu costs no re-upload at all.
///
/// Pinned in BOTH `honor_game_colours` modes: the path taken must not depend on
/// whether the game's colours are honoured. Skip-if-missing (gitignored story).
#[test]
fn zork0_hint_menu_draws_the_ring_and_keeps_its_bands_across_the_round_trip() {
    for honor in [true, false] {
        let story_path = stories_dir().join("zork0-r393-s890714.z6");
        let Ok(story_bytes) = std::fs::read(&story_path) else {
            eprintln!("SKIP: gitignored story missing at {}", story_path.display());
            return;
        };
        let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
        let picture_dims = picts.all_pict_dims();
        let mut session =
            GameSession::new_with_trace(story_bytes, honor, false, None, false, picture_dims, picts.std_window(), None, None)
                .expect("Zork0 (v6) should load and boot");
        session.set_pict_source(Some(picts));
        session.flush_boot_pictures();
        let _ = session.take_transcript();

        let mut state = app::state::AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        // A real kitty cell size: the band cache and its uploads only exist on the
        // pixel path, and that is what the resume gate protects.
        state.game_picker = Some(app::render::graphics::kitty_picker(14, 28));
        state.config.v6_render = app::config::V6RenderMode::Hybrid;
        state.config.honor_game_colours = honor;
        let area = Rect::new(0, 0, 100, 40);
        let frame = |session: &GameSession, state: &app::state::AppState| {
            let model = session.screen();
            let mut buf = Buffer::empty(area);
            app::render::screen::render_story_pane(&model, false, None, state, area, &mut buf);
        };
        let last_path = |state: &app::state::AppState| -> String {
            state.v6_path_log.borrow().last().map(|(l, _)| l.clone()).unwrap_or_default()
        };
        let band_uploads = |state: &app::state::AppState| -> usize {
            state
                .graphics_render
                .borrow()
                .uploaded_targets()
                .iter()
                .filter(|t| matches!(t, app::render::graphics::GraphicsTarget::Band(..)))
                .count()
        };

        // Gameplay: the pixel ring runs and fills the band cache.
        frame(&session, &state);
        assert_eq!(last_path(&state), "hybrid-ring", "honor={honor}: gameplay draws the ring");
        assert!(band_uploads(&state) > 0, "honor={honor}: the ring uploaded its bands");

        // Into the hint menu: a ring frame of its own now (SQ-0934).
        session.submit("hint");
        let entered = session.submit_char(b'y');
        assert!(entered.fault.is_none(), "entering the hint menu faulted: {:?}", entered.fault);
        frame(&session, &state);
        let menu_path = last_path(&state);
        assert_eq!(
            menu_path, "hybrid-ring",
            "honor={honor}: the hint screen is a ring — banner and flanks are the game's own \
             artwork, and throwing them away is what SQ-0934 fixed"
        );

        let uploads_in_menu = band_uploads(&state);
        // Out again: the ring was never gone, so its bands are still live.
        let quit = session.submit_char(b'q');
        assert!(quit.fault.is_none(), "leaving the hint menu faulted: {:?}", quit.fault);
        frame(&session, &state);
        assert_eq!(last_path(&state), "hybrid-ring", "honor={honor}: still the ring");
        // …and it never left, so there is nothing to recover: the bands the menu
        // frame uploaded are still the ones on screen. A re-upload here would mean
        // the ring had been dropped after all.
        let after = band_uploads(&state);
        assert_eq!(
            after, uploads_in_menu,
            "honor={honor}: the ring was continuous across the menu, so leaving it must \
             re-upload NO band (uploaded {after}, had {uploads_in_menu})"
        );
    }
}

/// SQ-0934: the story surface on a hint screen is the GRID the game printed into
/// the ring's clear middle — and it is the same screen in two different games.
///
/// Zork Zero and Shogun share one InvisiClues subsystem: measured on both, the
/// frame artwork is a ring at 78% top band / 70% flanks / **0.0% middle**, and the
/// header prints the same four strings at the same native rows. What differs is
/// only the rect of the middle, and — across releases of the SAME game — whether
/// the primary buffer is withdrawn at all. Blorb r393 and Amiga r366 withdraw it;
/// the Macintosh r296 keeps it and reaches the ring by another road.
///
/// This pins the withdrawn case on real media, because the structural unit tests in
/// `render::v6_layout` prove the RULE and cannot prove that any shipped game
/// actually publishes this shape.
///
/// Skip-if-missing, and non-vacuous: a fixture that is present but yielded nothing
/// fails rather than passing quietly.
#[test]
fn a_withdrawn_buffer_leaves_the_menu_grid_as_the_story_surface() {
    let specimens: &[(&str, u16)] = &[("zork0-r393-s890714.z6", 393), ("shogun-r322-s890706.z6", 322)];
    let mut seen = 0;
    for (file, release) in specimens {
        let path = stories_dir().join(file);
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            continue;
        };
        seen += 1;
        assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), *release, "{file} is not the pinned release");
        let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
        let dims = picts.all_pict_dims();
        let mut s = GameSession::new_with_trace(bytes, true, false, None, false, dims, picts.std_window(), None, None)
            .expect("boots");
        s.set_pict_source(Some(picts));
        s.flush_boot_pictures();
        let _ = s.take_transcript();
        // Zork Zero asks for a LINE first; Shogun asks for a CHAR (its title
        // splash). Answer whatever is in the way rather than assuming either.
        for _ in 0..8 {
            match s.pending_input() {
                InputKind::Line => break,
                InputKind::Char => {
                    let _ = s.submit_char(13);
                }
                InputKind::Event => {
                    let _ = s.submit("");
                }
            }
        }
        s.submit("hint");
        let entered = s.submit_char(b'y');
        assert!(entered.fault.is_none(), "{file}: entering hints faulted: {:?}", entered.fault);

        let model = s.screen();
        let WinNode::Layered(items) = &model.root else { panic!("{file}: v6 publishes a layered composite") };
        // The premise: the game really did withdraw its buffer for this screen.
        assert!(
            !items.iter().any(|pw| matches!(&pw.node, WinNode::Buffer(b) if b.primary)),
            "{file}: this release is expected to withdraw its primary buffer for the menu"
        );
        let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
        let story = layout.story.unwrap_or_else(|| panic!("{file}: the middle grid must stand in for it"));
        assert!(matches!(&story.node, WinNode::Grid(_)), "{file}: the surface is that Grid");
        // …and it is still chrome, or the 22 topic runs it carries are never drawn.
        assert!(
            layout.chrome.iter().any(|c| std::ptr::eq(*c, story)),
            "{file}: the promoted grid must remain in chrome"
        );
    }
    let any = specimens.iter().any(|(f, _)| stories_dir().join(f).is_file());
    assert!(!any || seen > 0, "stories are present but none was read");
}

/// **A promoted menu grid is a RECT, not a transcript surface** (SQ-1026).
///
/// With no primary `Buffer` on the frame, `classify_windows` promotes the `Grid`
/// filling the clear middle of the ring — the case pinned directly above. It does
/// that for the RECT, so the ring has a viewport to lay out around, and the grid
/// stays in `chrome` so its own topic runs still reach the canvas. Its own doc
/// comment says as much: *"a `Grid` in this slot contributes its rect and nothing
/// else"*, and lists the readers that pattern-match for a `Buffer` and decline
/// otherwise.
///
/// `build_v6_raster_canvas` was not one of them. It took the promoted grid's box as
/// a prose box and stamped the host transcript into it, so the whole scrollback was
/// re-wrapped underneath the topics. Reported on Amiga Shogun **r295/890321** off
/// `James Clavell's Shogun.adf`: 78 rows of transcript inside the 500x330 topic
/// list at native (70, 70). The tell that settled it was the player's own
/// `/dump-windows` output appearing inside the menu — only the HOST transcript can
/// put that there, since the game never printed it.
///
/// **Hybrid was already right** and is the reason the rule is not invented here:
/// `render_node` dispatches the story surface on its node kind, sending a `Buffer`
/// to `render_transcript` and a `Grid` to `draw_grid`. This restores parity.
///
/// Asked by POISONING the transcript and requiring the canvas not to move: a
/// hundred rows of a marker string reach a frame that must not show them, so the
/// case says "the transcript does not reach this screen" rather than pinning any
/// particular pixel. Guarded against passing vacuously on a blank canvas by
/// requiring the menu's own topics to be on it — those come from `chrome` and must
/// survive.
///
/// FALSIFY by removing the `WinNode::Buffer` guard in `build_v6_raster_canvas`: the
/// two canvases diverge and `RasterMetrics` comes back `Some`, which is the report.
#[test]
fn a_promoted_menu_grid_is_not_a_transcript_surface_in_raster() {
    let specimens: &[(&str, u16)] = &[("zork0-r393-s890714.z6", 393), ("shogun-r322-s890706.z6", 322)];
    let mut seen = 0;
    for (file, release) in specimens {
        let path = stories_dir().join(file);
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            continue;
        };
        seen += 1;
        assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), *release, "{file} is not the pinned release");
        let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
        let dims = picts.all_pict_dims();
        let mut s = GameSession::new_with_trace(bytes, true, false, None, false, dims, picts.std_window(), None, None)
            .expect("boots");
        s.set_pict_source(Some(picts));
        s.flush_boot_pictures();
        let _ = s.take_transcript();
        for _ in 0..8 {
            match s.pending_input() {
                InputKind::Line => break,
                InputKind::Char => {
                    let _ = s.submit_char(13);
                }
                InputKind::Event => {
                    let _ = s.submit("");
                }
            }
        }
        s.submit("hint");
        let entered = s.submit_char(b'y');
        assert!(entered.fault.is_none(), "{file}: entering hints faulted: {:?}", entered.fault);

        let model = s.screen();
        let WinNode::Layered(items) = &model.root else { panic!("{file}: v6 publishes a layered composite") };
        let cell = zvm::screen::V6Cell::DEFAULT;
        let native = app::render::v6_layout::native_extent(
            items,
            &app::native_font::TextFace::cell_only(cell),
        );
        let layout = app::render::v6_layout::classify_windows(items, cell);
        // The premise, restated so this case cannot pass on a frame that never
        // promoted anything: the story slot holds a Grid, and it is still chrome.
        let story = layout.story.unwrap_or_else(|| panic!("{file}: a story surface"));
        assert!(matches!(&story.node, WinNode::Grid(_)), "{file}: the promoted surface is a Grid");
        let topics = match &story.node {
            WinNode::Grid(g) => g.px_texts.len(),
            _ => 0,
        };
        assert!(topics >= 8, "{file}: the menu carries its topics ({topics} runs)");

        // Both honour modes: the guard must not be colour-dependent, and the ring's
        // page IS.
        for honor in [true, false] {
            let state = |lines: usize| {
                let mut st = app::state::AppState::default();
                st.colors = app::colors::ColorScheme::terminal_default();
                st.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
                st.config.v6_render = app::config::V6RenderMode::Raster;
                st.config.honor_game_colours = honor;
                st.transcript = (0..lines).map(|i| format!("ZZ transcript row {i} ZZ")).collect();
                st.transcript_runs = vec![Vec::new(); lines];
                st.transcript_images = vec![None; lines];
                st
            };
            let (clean, m_clean) =
                app::render::screen::build_v6_raster_canvas(&layout, native, &state(0));
            let (poisoned, m_poisoned) =
                app::render::screen::build_v6_raster_canvas(&layout, native, &state(100));
            let where_ = format!("{file} honor={honor}");
            assert!(
                m_clean.is_none() && m_poisoned.is_none(),
                "{where_}: there is no transcript on this frame, so no scroll metrics                  (got {m_clean:?} / {m_poisoned:?})"
            );
            assert!(
                clean == poisoned,
                "{where_}: a hundred rows of host transcript changed the menu screen —                  the promoted grid was taken for a prose box"
            );
            // Non-vacuity: the topics the grid carries really are on that canvas, so
            // the equality above is not two blank images agreeing.
            let (mx, my) = (u32::from(story.x_px), u32::from(story.y_px));
            let (mw, mh) = (u32::from(story.w_px), u32::from(story.h_px));
            let page = *clean.get_pixel(mx + mw / 2, my + mh - 2);
            let rows: std::collections::BTreeSet<u32> = (my..(my + mh).min(clean.height()))
                .filter(|&y| {
                    (mx..(mx + mw).min(clean.width())).any(|x| *clean.get_pixel(x, y) != page)
                })
                .map(|y| (y - my) / u32::from(cell.h()))
                .collect();
            assert!(
                rows.len() >= 4,
                "{where_}: the menu's own topics are drawn — only {} text row(s) carry ink",
                rows.len(),
            );
        }
    }
    let any = specimens.iter().any(|(f, _)| stories_dir().join(f).is_file());
    assert!(!any || seen > 0, "stories are present but none was read");
}

/// SQ-0937: every topic the game prints inside the story box reaches the screen,
/// at every pane width — including the column that starts one pixel inside the
/// box's left edge.
///
/// The Macintosh press is the specimen because it is the release that KEEPS its
/// primary buffer for the hint screen, so its menu is chrome runs inside the box
/// and takes the ring's in-box packing. Blorb and Amiga withdraw the buffer and
/// have their menu grid promoted to the story surface instead (SQ-0934), drawn by
/// `render_node`, so they never exercise this path and never showed the defect.
///
/// What failed: the run's COLUMN was mapped through the device scale while its ROW
/// was box-relative. Zork Zero prints its left topic column at native x=87 against
/// a box whose left edge is x=86; at a 136x50 pane that rounded to `viewport.x - 1`
/// and the run was dropped, so the entire left column vanished while the right
/// column at native x=320 drew normally.
///
/// **Swept across widths, because the defect is a rounding boundary** — it appears
/// at some pane sizes and not others, which is exactly how it survived until a user
/// happened to resize.
#[test]
fn the_macintosh_hint_menu_keeps_its_leftmost_topic_column_at_every_width() {
    let path = stories_dir().join("Zork Zero Disk.image");
    if !path.exists() {
        eprintln!("SKIP: gitignored Macintosh medium missing at {}", path.display());
        return;
    }
    let Ok(app::hints::LoadedStory::ZCode(bytes)) = app::hints::load_story(&path) else {
        panic!("Story.data mounts off the HFS volume")
    };
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 296, "the Mac disk carries r296");
    // Boot the way startup.rs does — the profile from the medium, and the screen
    // size through the full chain. Skip a link and the game lays its own windows
    // out differently and every column measured afterwards is of another screen.
    let profile = app::interpreter::InterpreterProfile::resolve(&path, None, None, None);
    let mut picts = app::graphics::PictSource::resolve_with_override(&path, app::graphics::PictureOverride::Unset, None);
    let dims = picts.all_pict_dims();
    let honoured =
        !picts.declines_game_colours(profile.default_colours());
    // SQ-1021/SQ-1022: every per-machine fact in one value, so this
    // harness cannot omit one — it was omitting the CELL.
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        honoured.then(|| profile.default_colours()).flatten(),
        true,
        app::native_font::FaceSet::none(),
        profile.palette(),
        None,
    );
    let mut s = app::session::GameSession::new_for_machine(bytes, honoured, false, false, dims, None, None, &boot)
    .expect("Zork Zero boots off the Macintosh disk");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    for _ in 0..8 {
        match s.pending_input() {
            InputKind::Line => break,
            InputKind::Char => {
                let _ = s.submit_char(13);
            }
            InputKind::Event => {
                let _ = s.submit("");
            }
        }
    }
    s.submit("hint");
    let entered = s.submit_char(b'y');
    assert!(entered.fault.is_none(), "entering hints faulted: {:?}", entered.fault);

    let model = s.screen();
    // The premise: this release really does keep its buffer, so the menu really is
    // taking the in-box packing rather than SQ-0934's promoted-grid road.
    let WinNode::Layered(items) = &model.root else { panic!("v6 publishes a layered composite") };
    assert!(
        items.iter().any(|pw| matches!(&pw.node, WinNode::Buffer(b) if b.primary)),
        "r296 keeps its primary buffer for the hint screen; without that this proves nothing"
    );

    // 136 wide is the reported size; the others are a sweep either side of it, so a
    // fix that merely moved the boundary cannot pass.
    for (w, h) in [(136u16, 50u16), (100, 34), (114, 50), (90, 30), (160, 60)] {
        let mut state = app::state::AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.v6_render = app::config::V6RenderMode::Hybrid;
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
        let screen: String = (0..area.height)
            .map(|y| (0..area.width).map(|x| buf.cell((x, y)).map_or(' ', |c| c.symbol().chars().next().unwrap_or(' '))).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        // The LEFT column — the one that was being dropped — and the right column,
        // which always survived, so a frame that lost both fails differently.
        for topic in ["GREAT HALL AREA", "SECRET WING", "THE JESTER"] {
            assert!(screen.contains(topic), "{w}x{h}: the leftmost topic column must draw ({topic:?}):\n{screen}");
        }
        assert!(screen.contains("AS A LAST RESORT"), "{w}x{h}: the right column draws too");
    }
}
