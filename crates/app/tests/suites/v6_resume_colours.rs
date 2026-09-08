//! SQ-0551: the story's own ink must survive a host Save State resume.
//!
//! Save State on quit + auto-resume brought Zork Zero back with its page colours
//! correct but its PROSE in the host theme — a cyan room name over light grey
//! body text on the story's white page — healing only once the game next called
//! `set_colour`.
//!
//! Cause: `ScreenState::current_fg`/`current_bg` are transient display state the
//! VM deliberately does not serialize (ZMSD §8.3 gives every v6 window its own
//! pair; these only MIRROR the current window's, so the prose stream can tag its
//! runs). The archive restored every window's colours faithfully — which is why
//! the white page came back — but handed these two fields back as `Default`, and
//! the prose stream reads them. `restore_screen` now re-derives the pair from the
//! restored window table, the same way the runtime maintains it.
//!
//! **Colour mode: `honor_game_colours = true`** — the shipped default and the
//! mode the bug appears in. With colours declined there is nothing to lose, so a
//! suite booted the other way cannot see this at all.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

use crate::fixture_paths::fixture_path;

fn story_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/zork0-r393-s890714.z6")
}

/// Boot Zork Zero with the game's colours HONOURED and drive it to a line prompt.
fn zork0_at_prompt() -> Option<(Vec<u8>, GameSession)> {
    let story_bytes = match std::fs::read(story_path()) {
        Ok(b) => b,
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", story_path().display());
            return None;
        }
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path()).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes.clone(), true, false, None, false, dims, picts.std_window(), None, None)
            .expect("Zork0 (v6) boots without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    for _ in 0..30 {
        if matches!(session.pending_input(), InputKind::Line) {
            break;
        }
        match session.pending_input() {
            InputKind::Char => session.submit_char(13),
            _ => session.submit(""),
        };
    }
    Some((story_bytes, session))
}

fn fresh_picts() -> PictSource {
    PictSource::new(blorb::resolve_resource_blorb(&story_path()).map(|(b, _)| b))
}

#[test]
fn story_ink_survives_a_save_state_resume() {
    use zvm::screen::ZColour;

    let Some((story_bytes, mut session)) = zork0_at_prompt() else { return };

    // The live game has set its own pair on window 0 — black ink on a white page.
    let live_fg = session.machine.screen.current_fg;
    let live_bg = session.machine.screen.current_bg;
    assert_ne!(
        live_fg,
        ZColour::Default,
        "Zork Zero sets its own foreground while playing (got {live_fg:?}) — without that there is nothing to lose"
    );
    // And its prose runs carry it, which is what actually reaches the renderer.
    let turn = session.submit("look");
    assert!(
        turn.transcript_runs.iter().any(|r| r.2 == live_fg),
        "the live game's prose runs carry its own ink {live_fg:?}: {:?}",
        turn.transcript_runs.iter().map(|r| r.2).collect::<Vec<_>>()
    );

    // Save through the REAL host Save State archive path.
    let mapper = mapper::mapper::Mapper::default();
    let es = Engine::save_state(&session);
    let pics = session.pictures_png();
    let path = std::env::temp_dir().join(format!("zork0-ink-{}.lanthorn", std::process::id()));
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

    let persisted = ac.screen.clone().expect("persisted screen");
    // A v6 archive deliberately does NOT carry the pair: the window table is its
    // one source of truth, and `restore_screen` re-derives from that. Asserting
    // it here keeps this test honest — if the pair ever starts riding along, the
    // derivation below would be covered for by the stored value and this test
    // would pass even with the fix removed.
    assert!(persisted.v6.is_some(), "a v6 archive carries the window table");
    assert_eq!(
        persisted.current_fg,
        ZColour::Default,
        "a v6 archive leaves the transient pair unwritten; it is re-derived on restore"
    );

    // Resume exactly as startup.rs does: fresh boot, restore_state, restore_screen.
    let mut fresh = GameSession::new_with_trace(
        story_bytes, true, false, None, false, fresh_picts().all_pict_dims(), fresh_picts().std_window(), None, None
    )
    .expect("fresh Zork0 boot");
    Engine::restore_state(&mut fresh, &ac.engine_save()).expect("restore_state");
    app::session::restore_screen(&mut fresh, persisted);
    fresh.load_pictures_png(&ac.pictures);
    fresh.set_pict_source(Some(fresh_picts()));

    // The resumed screen carries the story's pair, re-derived from the window table.
    assert_eq!(
        fresh.machine.screen.current_fg, live_fg,
        "the resumed screen must carry the story's own ink, not the host theme's"
    );
    assert_eq!(fresh.machine.screen.current_bg, live_bg, "and its own page colour with it");

    // End to end: the FIRST turn after the resume tags its prose with the story's
    // ink. This is the assertion that actually matches what the player saw — the
    // room name and body text came back in theme colours for exactly one turn.
    let after = fresh.submit("look");
    assert!(
        !after.transcript_runs.is_empty(),
        "the first resumed turn produced prose to colour"
    );
    assert!(
        after.transcript_runs.iter().all(|r| r.2 != ZColour::Default),
        "no run in the first turn after a resume may fall back to the theme ink: {:?}",
        after.transcript_runs.iter().map(|r| r.2).collect::<Vec<_>>()
    );
}

/// SQ-0551, the other half: versions 1–5/7/8 have no window table to re-derive
/// from, so their pair travels in `screen.bin` instead. Photopia (v5) sets black
/// on white exactly as Zork Zero does, and lost its ink for a turn the same way.
///
/// Driven through the real archive round trip, not a direct struct compare, so it
/// covers the serde mirror as well as `restore_screen`.
#[test]
fn story_ink_survives_a_resume_without_a_v6_window_table() {
    use zvm::screen::ZColour;

    let path = fixture_path("photopia.z5");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(bytes.clone(), true, false, None, false, dims, picts.std_window(), None, None)
            .expect("Photopia (v5) boots");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    for _ in 0..12 {
        if matches!(session.pending_input(), InputKind::Line) {
            break;
        }
        match session.pending_input() {
            InputKind::Char => session.submit_char(13),
            _ => session.submit(""),
        };
    }

    let live_fg = session.machine.screen.current_fg;
    let live_bg = session.machine.screen.current_bg;
    assert_ne!(live_fg, ZColour::Default, "Photopia sets its own foreground while playing");
    assert!(
        session.machine.screen.v6.is_none(),
        "this case is precisely the one with NO window table to derive from"
    );

    let mapper = mapper::mapper::Mapper::default();
    let es = Engine::save_state(&session);
    let arc = std::env::temp_dir().join(format!("photopia-ink-{}.lanthorn", std::process::id()));
    app::archive::save_archive_meta_pics(
        &arc,
        &mapper,
        &es,
        Some(&session.machine.screen),
        &session.machine.aux_data,
        app::archive::Meta {
            format_version: app::archive::CURRENT_FORMAT_VERSION,
            ifid: None, name: None, turns: 0, saved_at: String::new(), location: None, score: None, trigger: app::archive::SaveTrigger::HostState,
        },
        &app::archive::SessionRecord::empty(),
        &[],
        None,
        None,
    )
    .expect("save_archive_meta_pics");
    let ac = app::archive::load_archive(&arc).expect("load_archive");
    let _ = std::fs::remove_file(&arc);

    let mut fresh =
        GameSession::new_with_trace(bytes, true, false, None, false, Vec::new(), None, None, None).expect("fresh boot");
    Engine::restore_state(&mut fresh, &ac.engine_save()).expect("restore_state");
    app::session::restore_screen(&mut fresh, ac.screen.clone().expect("persisted screen"));

    assert_eq!(
        fresh.machine.screen.current_fg, live_fg,
        "with no window table the pair must come back from screen.bin, not reset to the theme"
    );
    assert_eq!(fresh.machine.screen.current_bg, live_bg, "and its page colour with it");
}
