//! A restored v6 screen is rebuilt from the display list, not from pixels — SQ-0588.
//!
//! SQ-0587 stopped a restored canvas being wiped by the next palette change, but only
//! by marking it `unreplayable`: the same flag that protects it from the replay is the
//! one that stops it being RECOLOURED, so restored art kept its saved colours for the
//! rest of the session. Pixels cannot be recoloured; only replaying the ops under the
//! new palette can do it.
//!
//! So the archive stores what the story DID — the display list plus the Current Palette
//! (Blorb §11.3) it was drawn under — and a restore replays it. The strong form of that
//! claim is the first test here: a session that saved, restored, and moved must end up
//! with the same pixels as one that just moved.
//!
//! Skip-if-missing per the other gitignored-story smokes.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn picts() -> PictSource {
    let p = stories_dir().join("arthur-r74-s890714.z6");
    PictSource::new(blorb::resolve_resource_blorb(&p).map(|(b, _)| b))
}

fn boot() -> Option<GameSession> {
    let p = stories_dir().join("arthur-r74-s890714.z6");
    let bytes = std::fs::read(&p).ok()?;
    let mut pic = picts();
    let dims = pic.all_pict_dims();
    let mut s = GameSession::new_with_trace(bytes, true, false, None, false, dims, pic.std_window(), None, None)
        .expect("Arthur (v6) boots");
    s.set_pict_source(Some(pic));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    for _ in 0..12 {
        let r = match s.pending_input() {
            InputKind::Line => s.submit(""),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = s.submit_char(b'n');
        }
    }
    let _ = s.submit("look");
    let _ = s.take_transcript();
    Some(s)
}

fn meta() -> app::archive::Meta {
    app::archive::Meta {
        format_version: app::archive::CURRENT_FORMAT_VERSION,
        ifid: None, name: None, turns: 0, saved_at: String::new(),
        location: None, score: None, trigger: app::archive::SaveTrigger::HostState,
    }
}

/// Save through the real archive, with the display list when `with_display`, and read
/// it back. `with_display: false` writes what a pre-SQ-0588 build wrote: PNGs only.
fn round_trip(session: &mut GameSession, tag: &str, with_display: bool) -> app::archive::ArchiveContents {
    let mapper = mapper::mapper::Mapper::default();
    let es = Engine::save_state(session);
    let path = app::scratch_dir(&format!("arthur-{tag}")).join("save.lanthorn");
    let (pics, display) = if with_display {
        let (dto, fallback, _diags) = session.display_list();
        (session.pictures_png_for(&fallback), Some(dto))
    } else {
        (session.pictures_png(), None)
    };
    app::archive::save_archive_meta_pics(
        &path,
        &mapper,
        &es,
        Some(&session.machine.screen),
        &session.machine.aux_data,
        meta(),
        &app::archive::SessionRecord::empty(),
        &pics,
        display.as_ref(),
        None,
    )
    .expect("save archive");
    let ac = app::archive::load_archive(&path).expect("load archive");
    let _ = std::fs::remove_file(&path);
    ac
}

/// Mirrors the app's restore order (`engine_helpers::apply_v6_pictures`). Note that
/// the `PictSource` is NOT replaced here: `boot` already installed one, and
/// `load_display_list` reinstates the archived palette INTO it — handing the session a
/// fresh source afterwards would silently drop that palette again.
fn restore_into(fresh: &mut GameSession, ac: &app::archive::ArchiveContents) {
    Engine::restore_state(fresh, &ac.engine_save()).expect("restore");
    app::session::restore_screen(fresh, ac.screen.clone().expect("screen"));
    match &ac.display {
        Some(d) => fresh.load_display_list(d, &ac.pictures),
        None => fresh.load_pictures_png(&ac.pictures),
    }
}

/// Every window canvas, keyed by window, as raw pixels — the whole visible v6 screen.
fn canvases(s: &GameSession) -> std::collections::BTreeMap<u8, Vec<u8>> {
    s.pictures_canvas.iter().map(|(w, c)| (*w, c.img.as_raw().clone())).collect()
}

/// The acceptance case. A session that saved, restored and moved must reach the same
/// pixels as one that only moved: the move recolours the palette, and a restored screen
/// can only follow it if it was rebuilt from ops rather than loaded as pixels.
#[test]
fn a_restored_screen_is_recoloured_by_a_later_palette_change() {
    let Some(mut control) = boot() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let mut saved = boot().expect("second boot");
    let ac = round_trip(&mut saved, "recolour", true);
    assert!(ac.display.is_some(), "the archive carries a display list");

    let mut fresh = boot().expect("fresh boot");
    restore_into(&mut fresh, &ac);
    assert_eq!(
        canvases(&fresh),
        canvases(&control),
        "the restore reproduces the screen exactly, before anything else happens"
    );

    // The move that recolours the palette, made by both sessions.
    for s in [&mut control, &mut fresh] {
        let r = s.submit("e");
        assert!(!r.quit && r.fault.is_none(), "\"e\" faulted/quit");
        let _ = s.take_transcript();
    }

    assert_eq!(
        canvases(&fresh),
        canvases(&control),
        "after the palette change the restored screen matches one that never saved — \
         a screen restored as PIXELS cannot be recoloured and diverges here"
    );
}

/// The self-check that makes storing ops safe. A window that cannot be rebuilt from its
/// ops must be spotted AT SAVE TIME and carried as a PNG, with a diagnostic naming it —
/// not written as an op list that silently restores wrong.
///
/// A window restored from pixels is exactly that case (it has no draw history), so
/// restoring a pre-SQ-0588 archive and saving again drives the fallback through public
/// API, with no need to corrupt anything.
#[test]
fn the_save_time_self_check_falls_back_to_a_png_and_says_which_window() {
    let Some(mut session) = boot() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    // A pre-SQ-0588 archive: canvas PNGs, no display list.
    let old = round_trip(&mut session, "legacy", false);
    assert!(old.display.is_none(), "the legacy archive carries no display list");
    assert!(!old.pictures.is_empty(), "...and does carry canvas PNGs");

    let mut fresh = boot().expect("fresh boot");
    restore_into(&mut fresh, &old);

    // Saving now: every window came back as pixels, so none can be replayed —
    // `restore_into`'s `None` branch (a pre-SQ-0588 archive has no display list
    // at all) loads pixels only and never reinstates a paint log, so every
    // window restored this way is `unreplayable` and must fall back to its PNG,
    // whatever `zvm`'s log (still holding `fresh`'s own pre-restore boot
    // history — this helper does not touch it) happens to say.
    let (_dto, fallback, diags) = fresh.display_list();
    assert!(!fallback.is_empty(), "...it falls back to its PNG");
    assert_eq!(fallback.len(), diags.len(), "...and every fallback names itself");
    for win in &fallback {
        assert!(
            diags.iter().any(|d| d.contains(&format!("v6 window {win}"))),
            "window {win} fell back without a diagnostic naming it: {diags:?}"
        );
    }
    assert!(
        !fresh.pictures_png_for(&fallback).is_empty(),
        "the fallback windows are actually stored as PNGs"
    );
}

/// A pre-SQ-0588 archive keeps restoring exactly as it did: pixels, no display list, and
/// no recolouring. Old saves are not migrated — they have no ops to migrate — so this
/// pins the behaviour rather than pretending otherwise.
#[test]
fn a_legacy_archive_still_restores_from_its_canvas_pngs() {
    let Some(mut session) = boot() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let before = canvases(&session);
    let old = round_trip(&mut session, "legacy-restore", false);

    let mut fresh = boot().expect("fresh boot");
    restore_into(&mut fresh, &old);

    assert_eq!(canvases(&fresh), before, "a legacy archive restores its canvases unchanged");
}

/// The Current Palette is host state that no archive carried before SQ-0588 (Blorb
/// §11.3): it is established by the last non-adaptive draw, and every adaptive picture
/// decodes through it. Round-tripping the display list must round-trip the palette with
/// it, or the replay rebuilds the right shapes in the wrong colours.
#[test]
fn the_current_palette_round_trips_with_the_display_list() {
    let Some(mut session) = boot() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let live = session.pict_source_mut().and_then(|s| s.current_palette().map(<[u8]>::to_vec));
    let ac = round_trip(&mut session, "palette", true);
    let dto = ac.display.as_ref().expect("display list");
    assert_eq!(dto.palette, live, "the archived palette is the one that was live at save time");

    let mut fresh = boot().expect("fresh boot");
    restore_into(&mut fresh, &ac);
    let restored = fresh.pict_source_mut().and_then(|s| s.current_palette().map(<[u8]>::to_vec));
    assert_eq!(restored, live, "and the restore reinstates it");
}
