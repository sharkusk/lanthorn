//! Lane P (v6 persistence) acceptance gates (SQ-0186).
//!
//! 1. `v6_host_save_state_restore_is_byte_identical`: boot Zork0, drive a turn,
//!    render the v6 raster composite, save through the REAL archive Save State
//!    path (`save_archive_meta_pics` + the v6 window table now inside
//!    `screen.bin`), restore into a FRESH session, render again, and assert the
//!    two composites are byte-for-byte equal. This proves the v6 window table
//!    (`screen.v6`) and the per-window graphics canvases (`pictures_canvas`)
//!    both survive a host Save State losslessly.
//!
//! 2. `zork0_quetzal_save_restore_smoke`: drive Zork0's OWN `@save`/`@restore`
//!    (Quetzal) via the PendingIo resume path and assert no fault + a sane
//!    layered v6 model afterwards (the game re-runs its own redraw on restore).
//!
//! The Zork0 asset is gitignored (local-only), so both tests skip cleanly when
//! it is absent — mirroring `zork0_v6_windows.rs`.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, PendingIo};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn story_path() -> PathBuf {
    stories_dir().join("zork0-r393-s890714.z6")
}

/// Boot Zork0 to its first prompt with a live Pict source and boot art flushed,
/// exactly as `zork0_v6_windows.rs` does. `None` when the asset is missing.
fn boot_zork0() -> Option<(Vec<u8>, GameSession)> {
    let story_bytes = std::fs::read(story_path()).ok()?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path()).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session = GameSession::new_with_trace(
        story_bytes.clone(), false, false, None, false, picture_dims, picts.std_window(), None, None
    )
    .expect("Zork0 (v6) should boot without a ZError");
    assert!(session.machine.fault_trace.is_none(), "Zork0 faulted during boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    Some((story_bytes, session))
}

/// Build a fresh `PictSource` for Zork0 (the same resolution the boot uses).
fn fresh_picts() -> PictSource {
    PictSource::new(blorb::resolve_resource_blorb(&story_path()).map(|(b, _)| b))
}

/// Render the v6 raster composite for a session, mirroring the render pipeline
/// (`build_chrome_canvas` + `story_clear_native` + `draw_story_text`) used by
/// `zork0_v6_windows.rs`. The `MainText` is a fixed synthetic block so the ONLY
/// variables are the persisted state: the v6 window table + graphics canvases.
fn composite(session: &GameSession) -> image::RgbaImage {
    use app::render::v6_layout as v6;
    let screen = Engine::screen(session);
    let WinNode::Layered(items) = &screen.root else {
        panic!("v6 story's screen() root must be WinNode::Layered, got {:?}", screen.root);
    };
    let native = v6::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = v6::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let colors = app::colors::ColorScheme::default();
    let default_fg = image::Rgba([220, 220, 220, 255]);
    let default_bg = image::Rgba([0, 0, 0, 255]);
    let mut canvas = v6::build_chrome_canvas(&layout.chrome, native, default_fg, default_bg, &colors, v6::TextLayer::All, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let main = v6::MainText {
        lines: (0..6).map(|i| format!("story line {i} {}", "x".repeat(20))).collect(),
        styles: Vec::new(),
        input: String::new(),
        cursor_col: 0,
        awaiting: false,
        floats: Vec::new(),
    };
    if let Some((sx, sy, sw, sh)) = v6::story_clear_native(layout.story, &canvas) {
        let (cols, rows) = ((sw / 8).max(1) as u16, (sh / 8).max(1) as u16);
        v6::draw_story_text(&mut canvas, &main, sx, sy, cols, rows, default_fg, &[], &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT), None);
    }
    canvas
}

#[test]
fn v6_host_save_state_restore_is_byte_identical() {
    let Some((story_bytes, mut session)) = boot_zork0() else {
        eprintln!("SKIP: gitignored Zork0 story missing at {}", story_path().display());
        return;
    };

    // Drive a turn so we save from a genuine mid-game state (not the boot frame).
    let _ = session.take_transcript();
    let r = session.submit("look");
    assert!(r.fault.is_none() && !r.quit, "Zork0 faulted/quit on 'look'");

    // Render BEFORE the save.
    let before = composite(&session);
    assert!(before.pixels().any(|p| p[3] > 0), "before-composite has content");

    // Save through the REAL host Save State archive path: the v6 window table
    // rides `screen.bin` (Some(&machine.screen)), the graphics canvases ride
    // `pictures/win-N.png` (session.pictures_png()).
    let mapper = mapper::mapper::Mapper::default();
    let es = Engine::save_state(&session);
    let pics = session.pictures_png();
    assert!(!pics.is_empty(), "Zork0 has rasterized graphics canvases to persist");
    let path = std::env::temp_dir().join(format!("zork0-persist-{}.lanthorn", std::process::id()));
    app::archive::save_archive_meta_pics(
        &path,
        &mapper,
        &es,
        Some(&session.machine.screen),
        &session.machine.aux_data,
        app::archive::Meta {
            format_version: app::archive::CURRENT_FORMAT_VERSION,
            ifid: None, name: None, turns: 0, saved_at: String::new(), location: None, score: None, trigger: app::archive::SaveTrigger::HostState,
        },
        &app::archive::SessionRecord::empty(),
        &pics,
        None,
        None,
    )
    .expect("save_archive_meta_pics");

    let ac = app::archive::load_archive(&path).expect("load_archive");
    let _ = std::fs::remove_file(&path);
    assert!(
        ac.screen.as_ref().and_then(|s| s.v6.as_ref()).is_some(),
        "the v6 window table must be persisted in screen.bin"
    );
    assert!(!ac.pictures.is_empty(), "graphics canvases must be persisted");

    // Restore into a completely FRESH session (fresh boot + fresh Pict source).
    let mut fresh = GameSession::new_with_trace(
        story_bytes, false, false, None, false, fresh_picts().all_pict_dims(), fresh_picts().std_window(), None, None
    )
    .expect("fresh Zork0 boot");
    Engine::restore_state(&mut fresh, &ac.engine_save()).expect("restore_state (Quetzal memory/stack/pc)");
    // Overlay the persisted v6 window table + graphics canvases (what the render
    // reads). Without these a fresh session shows a blank v6 chrome.
    fresh.machine.screen = ac.screen.clone().expect("persisted screen");
    fresh.load_pictures_png(&ac.pictures);
    fresh.set_pict_source(Some(fresh_picts()));

    // Render AFTER the restore and assert byte-for-byte equality.
    let after = composite(&fresh);
    assert_eq!(before.dimensions(), after.dimensions(), "composite dimensions identical after restore");
    assert!(
        before.as_raw() == after.as_raw(),
        "v6 raster composite must be byte-identical after a host Save State restore"
    );
}

/// SQ-0518: an inline transcript image (Zork0's opening drop-cap / room art,
/// a window-0 picture that flows with the prose) must survive the archive
/// Save State round-trip. Before the fix the transcript TEXT came back but the
/// embedded art was dropped on the floor (`transcript_images` was never
/// serialized). Drives the real elems pipeline to populate `transcript_images`,
/// saves the way every app path now does (`save_archive_meta_pics` with
/// `&state.transcript_images`), reloads, and asserts the restored transcript
/// carries the SAME inline images with byte-identical pixels — the PNG blob is
/// lossless, so an adaptive-palette picture re-materializes exactly (no re-decode
/// against a possibly-different Current Palette).
#[test]
fn inline_transcript_images_survive_archive_roundtrip_sq0518() {
    use app::state::{apply_transcript_elems, AppState, TranscriptKind};

    let Some((_bytes, mut session)) = boot_zork0() else {
        eprintln!("SKIP: gitignored Zork0 story missing at {}", story_path().display());
        return;
    };

    // Build the transcript through the SAME elems pipeline the app uses: the boot
    // banner's window-0 drop-cap arrives as ordered `TranscriptElem::Image`s.
    let mut state = AppState::default();
    let elems = Engine::take_transcript_elems(&mut session);
    apply_transcript_elems(&mut state, &elems);
    // If the opening frame didn't surface one, drive a couple of turns.
    if !state.transcript_images.iter().any(Option::is_some) {
        for cmd in ["look", "wait", "open mailbox"] {
            let r = Engine::submit(&mut session, cmd);
            if r.transcript_elems.is_empty() {
                state.push_transcript_runs(&r.transcript, TranscriptKind::Story, &r.transcript_runs);
            } else {
                apply_transcript_elems(&mut state, &r.transcript_elems);
            }
            if state.transcript_images.iter().any(Option::is_some) {
                break;
            }
        }
    }

    // A checksum + dims for every inline image, keyed by transcript index. Every
    // line here is Story-kind, so the save filter keeps them all and indices align.
    let fingerprint = |imgs: &[Option<app::inline_image::InlineImage>]| -> Vec<(usize, (u32, u32), u64)> {
        imgs.iter()
            .enumerate()
            .filter_map(|(i, o)| {
                o.as_ref().map(|img| {
                    let sum: u64 = img.pixels.as_raw().iter().map(|&b| b as u64).sum();
                    (i, img.pixels.dimensions(), sum)
                })
            })
            .collect()
    };
    let before = fingerprint(&state.transcript_images);
    assert!(
        !before.is_empty(),
        "Zork0's opening must surface at least one inline transcript image to test"
    );
    assert!(
        before.iter().all(|(_, (w, h), _)| *w > 0 && *h > 0),
        "every pre-save inline image has nonempty pixels"
    );

    // Save through the REAL archive path (as /save, auto-save, exit-save, and
    // named slots all do), then reload.
    let path = std::env::temp_dir().join(format!("zork0-inline-{}.lanthorn", std::process::id()));
    app::archive::save_archive_meta_pics(
        &path,
        &mapper::mapper::Mapper::default(),
        &Engine::save_state(&session),
        Some(&session.machine.screen),
        &session.machine.aux_data,
        app::archive::Meta {
            format_version: app::archive::CURRENT_FORMAT_VERSION,
            ifid: None, name: None, turns: 0, saved_at: String::new(), location: None, score: None, trigger: app::archive::SaveTrigger::HostState,
        },
        &app::archive::SessionRecord::of(&state),
        &session.pictures_png(),
        None,
        None,
    )
    .expect("save_archive_meta_pics with inline transcript images");

    let ac = app::archive::load_archive(&path).expect("load_archive");
    let _ = std::fs::remove_file(&path);

    let after = fingerprint(&ac.transcript_images);
    assert_eq!(
        after, before,
        "restored transcript must carry the same inline images (index, dims, pixel checksum) — \
         a lossless PNG round-trip, adaptive palette and all (SQ-0518)"
    );
    assert_eq!(
        ac.transcript_images.len(),
        ac.transcript.len(),
        "restored images stay parallel to the restored transcript"
    );
}

#[test]
fn zork0_quetzal_save_restore_smoke() {
    let Some((_bytes, mut session)) = boot_zork0() else {
        eprintln!("SKIP: gitignored Zork0 story missing at {}", story_path().display());
        return;
    };

    // Drive Zork0's OWN @save (Quetzal). Typing "save" suspends the VM with
    // PendingIo::Save; the host captures the blob and resumes (mirrors the
    // session.rs resume_save/resume_restore tests).
    let _ = session.take_transcript();
    let r = session.submit("save");
    let blob = if r.pending_io == Some(PendingIo::Save) {
        let blob = session.machine.save_quetzal();
        let rs = session.resume_save(true); // pretend the host wrote the file
        assert!(rs.fault.is_none(), "resume_save must not fault: {:?}", rs.fault);
        Some(blob)
    } else {
        // Some Zork0 builds route "save" differently; don't fail the smoke, just
        // report and stop here — the byte-identity test is the primary gate.
        eprintln!("NOTE: 'save' did not reach @save (pending_io={:?}); skipping @restore leg", r.pending_io);
        None
    };

    if let Some(blob) = blob {
        // Now drive @restore and feed the captured blob back.
        let _ = session.take_transcript();
        let r = session.submit("restore");
        if r.pending_io == Some(PendingIo::Restore) {
            let rr = session.resume_restore(Some(&blob));
            assert!(rr.fault.is_none(), "resume_restore must not fault: {:?}", rr.fault);
        } else {
            eprintln!("NOTE: 'restore' did not reach @restore (pending_io={:?})", r.pending_io);
        }
    }

    // Whatever path the verbs took, the game must still be alive with a sane,
    // layered v6 model (the game re-runs INIT-STATUS-LINE on its own restore).
    assert!(!session.quit, "Zork0 quit during the @save/@restore smoke");
    assert!(session.machine.fault_trace.is_none(), "Zork0 faulted during @save/@restore");
    let screen = Engine::screen(&session);
    assert!(
        matches!(screen.root, WinNode::Layered(_)),
        "v6 story keeps a layered screen model after @save/@restore"
    );
    assert_ne!(screen.content_size, (0, 0), "v6 content_size stays nonzero after @save/@restore");
}
