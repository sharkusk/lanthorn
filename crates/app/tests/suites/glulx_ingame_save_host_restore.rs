//! SQ-0556: a REAL Glulx game's in-game `@save` archive must load from the saves
//! manager, exactly like the Z-machine's and Scott's do.
//!
//! The unit tests pin the mechanism on hand-built images; this is the oracle that
//! it survives contact with an actual Inform-compiled Glulx story — its own SAVE
//! verb, its own `glk_fileref_create_by_prompt` plumbing, its own post-restore
//! tail. Save/persistence is precisely where a unit test passes while the real
//! thing corrupts state, so the assertions are behavioural: play PAST the save
//! point (picking things up), restore through the HOST path, and require the game
//! to replay the reference run move for move — including an inventory that has
//! forgotten everything taken after the save.


use app::archive::SaveTrigger;
use app::engine::Engine;
use app::glulx_session::GlulxSession;
use app::session::PendingIo;

use crate::fixture_paths::fixture_path;

/// The Glulx executable inside `advent.blb` (gitignored; the test skips loudly
/// when it is absent).
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

#[test]
fn a_real_glulx_ingame_save_archive_restores_through_the_host_path() {
    let Some(image) = advent_image() else { return };
    let mut sess = GlulxSession::new(image, 80, 24, true, false, false, (1.0, 1.0), None, &[])
        .expect("Adventure (Glulx) boots");
    let _ = sess.take_transcript(); // drain the banner

    // ── Walk into the well house, take the lamp, and SAVE there ──────────────
    sess.submit("in");
    sess.submit("get lamp");
    let saved_room = sess.current_location().map(|l| l.name).unwrap_or_default();
    assert_eq!(saved_room, "Inside Building", "walked to the save point");

    // Adventure's SAVE verb goes through `glk_fileref_create_by_prompt`, so it
    // bubbles to the host rather than being serviced silently.
    let r = sess.submit("save");
    assert_eq!(r.pending_io, Some(PendingIo::Save), "the game's SAVE verb bubbles a host Save request");

    // Seal the archive exactly as the app's `handle_save_as` does for an in-game
    // trigger — the same writer, the same trigger, a real `.lanthorn` on disk.
    let ingame = app::persist_files::game_save_bytes(&sess, SaveTrigger::Ingame);
    let dir = std::env::temp_dir().join(format!("bm-sq0556-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp save dir");
    app::persist_files::save_named(
        &dir, "GLULX-ADVENT-0556", "slot", SaveTrigger::Ingame, &mapper::mapper::Mapper::default(),
        &ingame, None, &[], None, None, sess.aux_data(), 3, Some(saved_room), None,
        &app::archive::SessionRecord::empty(),
    )
    .expect("the @save archive is written");
    let path = dir.join("slot.lanthorn");
    assert!(path.exists(), "an in-game @save produces a .lanthorn the saves manager can list");
    assert_eq!(
        app::archive::read_archive_meta(&path).expect("meta").trigger,
        SaveTrigger::Ingame,
        "it is recorded as an in-game save"
    );

    assert_eq!(sess.resume_save(true).pending_io, None, "@save completes and the game runs on");

    // ── The reference continuation, captured from the live run ───────────────
    let expected_out = sess.submit("out").transcript;
    let expected_inv = sess.submit("inventory").transcript;
    assert!(expected_out.contains("End Of Road"), "the reference move leaves the building: {expected_out:?}");
    assert!(expected_inv.contains("lantern"), "the reference inventory holds the lamp: {expected_inv:?}");

    // ── Play well past the save point, picking up things it never saw ────────
    sess.submit("in");
    sess.submit("get keys");
    sess.submit("get food");
    let drifted_inv = sess.submit("inventory").transcript;
    assert!(drifted_inv.contains("keys"), "the run really did move on: {drifted_inv:?}");
    assert_ne!(drifted_inv, expected_inv, "the live state has diverged from the save point");

    // ── The host restore: the saves-manager path, on this same live session ──
    let bytes = app::archive::read_quetzal_from_file(&path).expect("the archive's game.glksave reads back");
    sess.restore_game_save(&bytes)
        .expect("a real Glulx @save archive loads through the host restore path (SQ-0556)");
    assert!(!sess.has_quit(), "the restore left the session alive");

    // Move for move with the reference run. The first assertion also proves the
    // session was re-armed at a clean prompt — a restore that left the
    // pre-restore `glk_select` suspension in place would swallow this command.
    assert_eq!(sess.submit("out").transcript, expected_out, "the restored game replays the reference move");
    assert_eq!(
        sess.submit("inventory").transcript,
        expected_inv,
        "the restored inventory is the save point's — the keys and food taken afterwards are gone"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
