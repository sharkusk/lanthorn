//! SQ-1410: a restore-time notice naming what an older-format archive doesn't
//! carry — the screen (`screen.bin`, format_version < 9, SQ-1401) or a v6
//! story's pictures (`display.bin`, format_version < 10, SQ-1403).
//!
//! `app::archive::RestoreDegradation` (unit-tested directly in `archive.rs`
//! under `t-persist`) is the pure detection: it reads only `meta.format_version`
//! — "the version is the fact" — plus whether the restored story is v6. This
//! suite proves the END-TO-END wiring: a real v6 story, restored from a
//! hand-aged archive, ends up with the notice in its transcript exactly once,
//! tagged `TranscriptKind::Warning` (the same mechanism/selector every other
//! host warning at startup/restore uses — see `startup.rs`'s broken-config and
//! theme-warning notices) — and a current-format restore shows nothing.
//!
//! `restore_and_notice` below mirrors `engine_helpers::apply_archive_state`'s
//! order by hand (transcript replace, THEN the notice, so it survives as the
//! last line) rather than calling it directly: `engine_helpers` is a binary-only
//! module (`main.rs`'s `mod engine_helpers`), unreachable from an integration
//! test — the same constraint `v6_display_list_restore.rs`'s `restore_into`
//! already works around for `apply_v6_pictures`.
//!
//! Skip-if-missing per the other gitignored-story smokes.

use std::path::PathBuf;

use app::archive::{ArchiveContents, Meta, RestoreDegradation, SaveTrigger, SessionRecord};
use app::engine::Engine;
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};
use app::state::{AppState, TranscriptKind};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn picts() -> PictSource {
    let p = stories_dir().join("arthur-r74-s890714.z6");
    PictSource::new(blorb::resolve_resource_blorb(&p).map(|(b, _)| b))
}

/// Boot Arthur (v6) far enough to sit at a normal `>` prompt, exactly as
/// `v6_display_list_restore.rs`'s `boot()` does.
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

fn meta_at(format_version: u32) -> Meta {
    Meta {
        format_version,
        ifid: None,
        name: None,
        turns: 0,
        saved_at: String::new(),
        location: None,
        score: None,
        trigger: SaveTrigger::HostState,
    }
}

/// Save an archive aged to `format_version`, with no display list — what an
/// archive older than SQ-1403 (format_version 10) actually carries: canvas
/// PNGs only, no `display.bin` paint log.
fn round_trip_aged(session: &mut GameSession, tag: &str, format_version: u32) -> ArchiveContents {
    let mapper = mapper::mapper::Mapper::default();
    let es = Engine::save_state(session);
    let path = app::scratch_dir(&format!("sq1410-{tag}")).join("save.lanthorn");
    app::archive::save_archive_meta_pics(
        &path,
        &mapper,
        &es,
        Some(&session.machine.screen),
        &session.machine.aux_data,
        meta_at(format_version),
        &SessionRecord::empty(),
        &session.pictures_png(),
        None, // no display list — the paint log SQ-1403 added
        None,
    )
    .expect("save archive");
    let ac = app::archive::load_archive(&path).expect("load archive");
    let _ = std::fs::remove_file(&path);
    ac
}

/// Restore `ac` into `fresh`/`state`, then compute and push the SQ-1410 notice
/// exactly as `engine_helpers::apply_archive_state` does: transcript replace
/// first, notice last, so it survives as the final line rather than being
/// overwritten by the restored transcript.
fn restore_and_notice(state: &mut AppState, fresh: &mut GameSession, ac: &ArchiveContents) {
    Engine::restore_state(fresh, &ac.engine_save()).expect("restore");
    if let Some(scr) = ac.screen.clone() {
        app::session::restore_screen(fresh, scr);
    }
    match &ac.display {
        Some(d) => fresh.load_display_list(d, &ac.pictures),
        None => fresh.load_pictures_png(&ac.pictures),
    }
    state.transcript = ac.transcript.clone();
    state.clear_anchor = None;
    state.transcript_kinds = ac.transcript_kinds.clone();
    state.transcript_runs = ac.transcript_runs.clone();
    state.transcript_para = ac.transcript_para.clone();
    state.reset_transcript_sidecars();
    state.transcript_images = ac.transcript_images.clone();

    let is_v6 = fresh.machine.mem.version() == 6;
    let degradation = RestoreDegradation::from_format_version(ac.meta.format_version, is_v6);
    if let Some(msg) = degradation.notice_text() {
        state.push_transcript_internal(&msg, TranscriptKind::Warning);
    }
}

/// The acceptance case: a format-9 archive (SQ-1403's paint log doesn't exist
/// yet, but SQ-1401's screen.bin does) of a v6 story restores with exactly one
/// Warning-tagged notice naming pictures, not the screen.
#[test]
fn restoring_a_format_9_v6_archive_notes_missing_pictures_once() {
    let Some(mut session) = boot() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let ac = round_trip_aged(&mut session, "fmt9", 9);
    assert!(ac.screen.is_some(), "format_version 9 still carries screen.bin");
    assert!(ac.display.is_none(), "this archive carries no display list");

    let mut fresh = boot().expect("fresh boot");
    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    restore_and_notice(&mut state, &mut fresh, &ac);

    let expected = "[Restored from an older save: pictures will repaint as you play.]";
    let hits: Vec<usize> = state
        .transcript
        .iter()
        .enumerate()
        .filter(|(_, line)| line.as_str() == expected)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(hits.len(), 1, "the notice must appear exactly once: {:?}", state.transcript);
    assert_eq!(
        state.transcript_kinds[hits[0]],
        TranscriptKind::Warning,
        "the notice must carry the same TranscriptKind every other host restore warning uses"
    );
    // The line is the LAST one: pushed after the restored transcript, not before.
    assert_eq!(hits[0], state.transcript.len() - 1, "the notice must survive as the final transcript line");
}

/// No notice at all for a current-format archive.
#[test]
fn restoring_a_current_format_archive_shows_no_notice() {
    let Some(mut session) = boot() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let ac = round_trip_aged(&mut session, "current", app::archive::CURRENT_FORMAT_VERSION);

    let mut fresh = boot().expect("fresh boot");
    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    restore_and_notice(&mut state, &mut fresh, &ac);

    assert!(
        !state.transcript.iter().any(|line| line.contains("Restored from an older save")),
        "a current-format restore must show no degradation notice: {:?}",
        state.transcript
    );
    assert!(
        !state.transcript_kinds.contains(&TranscriptKind::Warning),
        "no Warning line should have been pushed at all"
    );
}
