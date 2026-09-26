//! SQ-0523: a resumed Glulx game must come back in the room it was saved in.
//!
//! Glulx has no object tree to interrogate, so the current room is recovered from
//! the room HEADING the game prints and cached in `GlulxSession::last_room` —
//! host-side state that lives outside the VM snapshot. A resume restored the VM
//! correctly but left that cache holding whatever the FRESH BOOT printed, so the
//! map selected and centred on the game's opening room until the next turn
//! produced a heading and quietly corrected it. Save deep in Adventure's maze,
//! resume, and the map jumped to "At End Of Road" until you typed `look`.
//!
//! The fix seeds the cache from the room name the archive already records
//! (`Meta::location`) — no new persisted field — rebuilding the id with the same
//! `heading_to_room` the live path uses, so the seeded room is identical to the
//! one the next heading would produce.
//!
//! The Z-machine needs no equivalent: its `current_location` reads the restored
//! object tree, which is inside the snapshot.


use app::engine::Engine;
use app::glulx_session::GlulxSession;

use crate::fixture_paths::fixture_path;

/// The Glulx executable inside `advent.blb`.
fn advent_image() -> Option<Vec<u8>> {
    let p = fixture_path("advent.blb");
    let bytes = match std::fs::read(&p) {
        Ok(b) => b,
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", p.display());
            return None;
        }
    };
    let blorb = blorb::Blorb::parse(bytes).expect("advent.blb parses as a Blorb");
    let (_kind, exec) = blorb.executable().expect("advent.blb carries an executable chunk");
    Some(exec.to_vec())
}

fn boot(image: Vec<u8>) -> GlulxSession {
    GlulxSession::new(image, 80, 24, true, false, false, (1.0, 1.0), None, &[]).expect("Adventure (Glulx) boots")
}

#[test]
fn a_resumed_glulx_game_reports_the_room_it_was_saved_in() {
    let Some(image) = advent_image() else { return };

    let mut session = boot(image.clone());
    let boot_room = session.current_location().expect("Adventure prints its opening room heading");

    // Walk somewhere else, so "the saved room" and "the boot room" differ at all.
    for cmd in ["in", "take lamp", "down", "west", "west"] {
        let _ = Engine::submit(&mut session, cmd);
    }
    let saved = session.current_location().expect("a room heading after moving");
    assert_ne!(
        saved.number, boot_room.number,
        "the walk must leave the opening room, or this test proves nothing (still in {:?})",
        boot_room.name
    );

    // Save through the real host Save State archive path. `Meta::location` is what
    // `save_summary` records at every save site: the current room's name.
    let mapper = mapper::mapper::Mapper::default();
    let es = Engine::save_state(&session);
    let path = std::env::temp_dir().join(format!("advent-resume-{}.lanthorn", std::process::id()));
    app::archive::save_archive_meta_pics(
        &path,
        &mapper,
        &es,
        None,
        &Default::default(),
        app::archive::Meta {
            format_version: app::archive::CURRENT_FORMAT_VERSION,
            ifid: None,
            name: None,
            turns: 5,
            saved_at: String::new(),
            location: Some(saved.name.clone()),
            score: None,
            trigger: app::archive::SaveTrigger::HostState,
        },
        &app::archive::SessionRecord::empty(),
        &[],
        None,
        None,
    )
    .expect("save_archive_meta_pics");
    let ac = app::archive::load_archive(&path).expect("load_archive");
    let _ = std::fs::remove_file(&path);
    assert_eq!(
        ac.meta.location.as_deref(),
        Some(saved.name.as_str()),
        "the archive already records the room name the seed is rebuilt from"
    );

    // Resume as the host does: fresh boot, restore_state, then seed the location
    // cache from the archive's recorded room.
    let mut fresh = boot(image);
    Engine::restore_state(&mut fresh, &ac.engine_save()).expect("restore_state");
    if let Some(name) = ac.meta.location.as_deref() {
        fresh.seed_last_room(name);
    }

    let resumed = fresh.current_location().expect("a resumed session reports a location");
    assert_eq!(
        (resumed.number, resumed.name.as_str()),
        (saved.number, saved.name.as_str()),
        "a resumed game must report the room it was saved in, not the one the fresh boot printed \
         ({:?})",
        boot_room.name
    );

    // And the id is the one the live path would produce, so the map selects the
    // node already in the restored graph rather than creating a second one: the
    // next turn's own heading must agree with the seed, with no room change.
    let after = Engine::submit(&mut fresh, "look");
    let looked = after.location.expect("`look` reprints the room heading");
    assert_eq!(
        (looked.number, looked.name.as_str()),
        (resumed.number, resumed.name.as_str()),
        "the seeded room must equal what the next heading resolves to — otherwise the map \
         would move again on the first turn, which is the bug in a subtler form"
    );
}
