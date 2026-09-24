//! Exit / quit persistence paths: exit auto-save, the quit-dialog "Save State &
//! quit" snapshot, and the pending config-write flush. Extracted verbatim from
//! `main.rs` (SQ-0306). The saves themselves — the gates, the archive each one
//! writes — are the library's [`app::host::persist`] (SQ-1539); what stays here
//! is the TUI's half: telling the termination watchdog a write is in flight
//! (SQ-0651), and saying on stderr how it went.

use mapper::mapper::Mapper;

use app::engine::Engine;
use app::host::persist::ExitSave;
use app::state::AppState;

/// Save on exit when auto_save is enabled — see
/// [`app::host::persist::exit_auto_save`] for the rules (the auto_save gate, and
/// the SQ-0283 skip while an in-game @save/@restore is suspended).
pub(crate) fn exit_auto_save(
    session: &mut dyn Engine,
    mapper: &Mapper,
    state: &app::state::AppState,
    ifid: &str,
    arc_file: &std::path::Path,
) {
    // Tell the termination watchdog a save is actively running so its fixed grace
    // does not kill the process mid-write and lose it (SQ-0651 / partial SQ-0644).
    // Held for the whole snapshot+write; cleared on drop, unwind included.
    let _writing = crate::ExitSaveGuard::new();
    match app::host::persist::exit_auto_save(session, mapper, state, ifid, arc_file) {
        ExitSave::Skipped => {}
        ExitSave::Saved => eprintln!("lanthorn: map saved to {}", arc_file.display()),
        ExitSave::Failed(e) => {
            eprintln!("lanthorn: warning: could not save to {}: {}", arc_file.display(), e)
        }
    }
}

/// Exit path for a CLEAN, game-driven quit (SQ-1342) — see
/// [`app::host::persist::exit_clear_resume_save`]. Called instead of, never
/// alongside, `exit_auto_save` — see the call site in `main.rs` §6 gated on
/// `state.game_ended`.
pub(crate) fn exit_clear_resume_save(
    session: &mut dyn Engine,
    mapper: &Mapper,
    state: &app::state::AppState,
    ifid: &str,
    arc_file: &std::path::Path,
) {
    let _writing = crate::ExitSaveGuard::new();
    match app::host::persist::exit_clear_resume_save(session, mapper, state, ifid, arc_file) {
        ExitSave::Skipped => {}
        ExitSave::Saved => eprintln!(
            "lanthorn: map saved to {} (story finished — no resume point)",
            arc_file.display()
        ),
        ExitSave::Failed(e) => {
            eprintln!("lanthorn: warning: could not save to {}: {}", arc_file.display(), e)
        }
    }
}

/// Quit-dialog "Save State & quit" host snapshot, extracted from the quit-dialog
/// keyboard and mouse handlers so the guard it relies on is unit-testable — see
/// [`app::host::persist::save_state_now`] (skipped while an in-game
/// @save/@restore is suspended, SQ-0283; the dialog still proceeds to quit).
///
/// Returns the failure message when the save the user explicitly asked for did
/// not happen, so the caller can print it AFTER the terminal is restored (SQ-0651
/// — this used to be `let _ =`, and "Save State & quit" quit silently with
/// nothing saved). `None` on success and on the pending-save skip above, which is
/// a deliberate no-op rather than a failure.
#[must_use = "a failed Save State & quit must be reported to the user"]
pub(crate) fn quit_dialog_save(
    session: &mut dyn Engine,
    mapper: &Mapper,
    state: &app::state::AppState,
    ifid: &str,
    arc_file: &std::path::Path,
) -> Option<String> {
    match app::host::persist::save_state_now(session, mapper, state, ifid, arc_file) {
        ExitSave::Skipped | ExitSave::Saved => None,
        ExitSave::Failed(e) => Some(format!(
            "lanthorn: warning: \"Save State & quit\" could not save to {}: {}",
            arc_file.display(),
            e
        )),
    }
}

// ── Pending config-write flush ────────────────────────────────────────────────

/// Write `state.config` to `config.toml` if `pending_config_write` is set, then
/// clear the flag. Called after both key-dispatch paths (`KeyResolve::Action`
/// and `KeyResolve::Command`, the latter via `dispatch_slash_outcome`) so a
/// resize-reset/exit persists regardless of which path handled the key.
pub(crate) fn flush_pending_config_write(state: &mut AppState) {
    if state.pending_config_write {
        // A save can legitimately fail — a read-only home, or a config.toml the user
        // has broken, which `write_config_file` refuses to overwrite (SQ-0580). Say so
        // rather than dropping the setting on the floor.
        if let Err(e) = app::config::write_config_file(&state.config) {
            state.push_notice(&format!("[config not saved: {e}]"));
        }
        state.pending_config_write = false;
    }
}

#[cfg(all(test, feature = "t-misc"))]
mod tests {
    /// Engine stand-in whose in-game @save/@restore never resolves (mirrors a
    /// mid-suspension Glulx session). `save_state`/`aux_data` are left
    /// `unreachable!()`: the exit auto-save guard (SQ-0283 Task 6 carry-forward
    /// fix) must never reach them while a save/restore is pending -- reaching
    /// either would be the very bug (a snapshot capturing the un-popped @save
    /// call stub) the guard exists to prevent.
    struct SaveloadPendingEngine;

    impl app::engine::Engine for SaveloadPendingEngine {
        fn submit(&mut self, _command: &str) -> app::session::TurnResult { unreachable!() }
        fn submit_key(&mut self, _key: app::engine::KeyInput) -> Option<app::session::TurnResult> { unreachable!() }
        fn take_transcript(&mut self) -> String { unreachable!() }
        // No screen-clear channel: this double is not a game.
        fn drain_screen_clear(&mut self) -> bool { false }
        fn pending_input(&self) -> app::session::InputKind { unreachable!() }
        fn resume_save(&mut self, _wrote_ok: bool) -> app::session::TurnResult { unreachable!() }
        fn resume_restore(&mut self, _data: Option<&[u8]>) -> app::session::TurnResult { unreachable!() }
        fn has_quit(&self) -> bool { false }
        fn screen(&self) -> app::engine::ScreenModel { unreachable!() }
        fn save_state(&self) -> app::engine::EngineSave {
            unreachable!("exit_auto_save must not snapshot while a save/restore is pending")
        }
        fn restore_state(&mut self, _save: &app::engine::EngineSave) -> Result<(), app::engine::EngineError> { unreachable!() }
        fn restore_game_save(&mut self, _bytes: &[u8]) -> Result<(), app::engine::EngineError> { unreachable!() }
        fn is_saveload_pending(&self) -> bool { true }
        fn aux_data(&self) -> &std::collections::BTreeMap<String, Vec<u8>> {
            unreachable!("exit_auto_save must not read aux data while a save/restore is pending")
        }
        fn set_aux_data(&mut self, _data: std::collections::BTreeMap<String, Vec<u8>>) { unreachable!() }
        fn aux_dirty(&self) -> bool { false }
        fn clear_aux_dirty(&mut self) {}
        fn current_location(&self) -> Option<app::engine::LocationInfo> { None }
        fn as_any(&self) -> &dyn std::any::Any { self }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    }

    /// Engine stand-in that CAN be snapshotted, so the archive write is actually
    /// attempted. `saw_in_progress` records the exit-save flag as observed from
    /// *inside* the save (the archive writer reads `aux_data` mid-write).
    struct SnapshotableEngine {
        aux: std::collections::BTreeMap<String, Vec<u8>>,
        saw_in_progress: std::cell::Cell<bool>,
    }

    impl app::engine::Engine for SnapshotableEngine {
        fn submit(&mut self, _command: &str) -> app::session::TurnResult { unreachable!() }
        fn submit_key(&mut self, _key: app::engine::KeyInput) -> Option<app::session::TurnResult> { unreachable!() }
        fn take_transcript(&mut self) -> String { unreachable!() }
        // No screen-clear channel: this double is not a game.
        fn drain_screen_clear(&mut self) -> bool { false }
        fn pending_input(&self) -> app::session::InputKind { unreachable!() }
        fn resume_save(&mut self, _wrote_ok: bool) -> app::session::TurnResult { unreachable!() }
        fn resume_restore(&mut self, _data: Option<&[u8]>) -> app::session::TurnResult { unreachable!() }
        fn has_quit(&self) -> bool { false }
        fn screen(&self) -> app::engine::ScreenModel { unreachable!() }
        fn save_state(&self) -> app::engine::EngineSave {
            app::engine::EngineSave::new("test", 1, vec![1, 2, 3])
        }
        fn restore_state(&mut self, _save: &app::engine::EngineSave) -> Result<(), app::engine::EngineError> { unreachable!() }
        fn restore_game_save(&mut self, _bytes: &[u8]) -> Result<(), app::engine::EngineError> { unreachable!() }
        fn is_saveload_pending(&self) -> bool { false }
        fn aux_data(&self) -> &std::collections::BTreeMap<String, Vec<u8>> {
            if crate::exit_save_in_progress() {
                self.saw_in_progress.set(true);
            }
            &self.aux
        }
        fn set_aux_data(&mut self, _data: std::collections::BTreeMap<String, Vec<u8>>) {}
        fn aux_dirty(&self) -> bool { false }
        fn clear_aux_dirty(&mut self) {}
        fn current_location(&self) -> Option<app::engine::LocationInfo> { None }
        fn as_any(&self) -> &dyn std::any::Any { self }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    }

    impl SnapshotableEngine {
        fn new() -> SnapshotableEngine {
            SnapshotableEngine { aux: Default::default(), saw_in_progress: std::cell::Cell::new(false) }
        }
    }

    /// One lock for every test that runs `exit_auto_save` (or asserts on the
    /// process-global exit-save flag): the flag is one static for the whole
    /// process, so under `cargo test`'s shared-process model two of these
    /// tests on parallel threads see each other's saves — the watchdog test's
    /// "nothing running before" raced exactly that on CI's Linux runner while
    /// nextest's per-test processes structurally could not show it (the
    /// SQ-0904 class, SQ-1184's flush test being the new second writer).
    /// Poison-proof: a panicking holder must not fail its neighbours twice.
    static EXIT_SAVE_FLAG: std::sync::Mutex<()> = std::sync::Mutex::new(());
    fn exit_save_lock() -> std::sync::MutexGuard<'static, ()> {
        EXIT_SAVE_FLAG.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn quit_dialog_save_reports_a_failed_write() {
        // SQ-0651: "Save State & quit" is a save the user explicitly asked for.
        // The call site used to be `let _ =`, so a failed write quit the app with
        // no message and no save. The failure must come back as a message the run
        // loop can print once the terminal is restored.
        let mut engine = SnapshotableEngine::new();
        let state = app::state::AppState::default();
        let mapper = mapper::mapper::Mapper::default();
        // A path whose parent is a FILE, not a directory: the write cannot succeed.
        let blocker = std::env::temp_dir().join(format!("bm-quitsave-blocker-{}", std::process::id()));
        std::fs::write(&blocker, b"not a directory").unwrap();
        let arc_file = blocker.join("save.lanthorn");

        let warn = super::quit_dialog_save(&mut engine, &mapper, &state, "ZCODE-1", &arc_file)
            .expect("a failed Save State & quit must report why");
        assert!(
            warn.contains("could not save"),
            "the message must say the save failed: {warn}"
        );
        let _ = std::fs::remove_file(&blocker);
    }

    #[test]
    fn quit_dialog_save_reports_nothing_when_the_write_succeeds() {
        let mut engine = SnapshotableEngine::new();
        let state = app::state::AppState::default();
        let mapper = mapper::mapper::Mapper::default();
        let arc_file = std::env::temp_dir().join(format!("bm-quitsave-ok-{}.lanthorn", std::process::id()));
        let _ = std::fs::remove_file(&arc_file);

        let warn = super::quit_dialog_save(&mut engine, &mapper, &state, "ZCODE-1", &arc_file);
        assert!(warn.is_none(), "a successful save reports nothing: {warn:?}");
        assert!(arc_file.exists(), "the archive was written");
        let _ = std::fs::remove_file(&arc_file);
    }

    /// SQ-0651 / partial SQ-0644: the termination watchdog's fixed 600ms grace
    /// could kill the process mid exit-save. `exit_auto_save` must publish "a save
    /// is running" for the whole write — observed here from INSIDE the save, via
    /// the `aux_data` the archive writer calls mid-write — and clear it after.
    ///
    /// One test, not three: the flag is process-global, so separate tests
    /// asserting "not running" would race each other under the parallel harness.
    #[test]
    fn exit_auto_save_publishes_its_progress_to_the_termination_watchdog() {
        let _flag = exit_save_lock();
        assert!(!crate::exit_save_in_progress(), "nothing running before");
        {
            let _g = crate::ExitSaveGuard::new();
            assert!(crate::exit_save_in_progress(), "the watchdog must see the save running");
        }
        assert!(!crate::exit_save_in_progress(), "cleared on drop");

        let mut engine = SnapshotableEngine::new();
        let mut state = app::state::AppState::default();
        state.config.auto_save = true;
        let mapper = mapper::mapper::Mapper::default();
        let arc_file = std::env::temp_dir().join(format!("bm-exitsave-flag-{}.lanthorn", std::process::id()));
        let _ = std::fs::remove_file(&arc_file);

        super::exit_auto_save(&mut engine, &mapper, &state, "ZCODE-1", &arc_file);
        assert!(
            engine.saw_in_progress.get(),
            "the flag must be set for the whole write, not just around it"
        );
        assert!(!crate::exit_save_in_progress(), "cleared once the save returns");
        assert!(arc_file.exists());
        let _ = std::fs::remove_file(&arc_file);
    }

    #[test]
    fn exit_auto_save_skips_snapshot_while_a_save_is_pending() {
        let _flag = exit_save_lock();
        // SQ-0283 carry-forward fix: a host save_state() snapshot captured while
        // a Glulx in-game @save is suspended would embed the un-popped @save call
        // stub; restore_state never pops it, corrupting the stack on a later Save
        // State restore. exit_auto_save must skip entirely (not call save_state)
        // when Engine::is_saveload_pending() is true, even with auto_save on.
        let mut engine = SaveloadPendingEngine;
        let mut state = app::state::AppState::default();
        state.config.auto_save = true;
        let mapper = mapper::mapper::Mapper::default();
        let arc_file = std::env::temp_dir().join(format!("bm-t6-pending-{}.lanthorn", std::process::id()));
        let _ = std::fs::remove_file(&arc_file);

        // Must not panic (save_state()/aux_data() are unreachable!()) and must not
        // write the archive file.
        super::exit_auto_save(&mut engine, &mapper, &state, "ZCODE-1", &arc_file);

        assert!(!arc_file.exists(), "exit auto-save must not write while a save/restore is pending");
        let _ = std::fs::remove_file(&arc_file);
    }

    #[test]
    fn quit_dialog_save_skips_snapshot_while_a_save_is_pending() {
        // SQ-0283 review fix: the quit-dialog "Save State & quit" path was an
        // unguarded save_state() reachable while a Glulx in-game @save is
        // suspended (Ctrl+Q wins even over an open SaveAs prompt). Mirrors
        // exit_auto_save_skips_snapshot_while_a_save_is_pending above but for the
        // extracted quit_dialog_save helper, which has no auto_save gate.
        let mut engine = SaveloadPendingEngine;
        let state = app::state::AppState::default();
        let mapper = mapper::mapper::Mapper::default();
        let arc_file = std::env::temp_dir().join(format!("bm-t6-quit-pending-{}.lanthorn", std::process::id()));
        let _ = std::fs::remove_file(&arc_file);

        // Must not panic (save_state()/aux_data() are unreachable!()) and must not
        // write the archive file.
        let warn = super::quit_dialog_save(&mut engine, &mapper, &state, "ZCODE-1", &arc_file);
        assert!(warn.is_none(), "a deliberate skip is not a failure to report");

        assert!(!arc_file.exists(), "quit-dialog save must not write while a save/restore is pending");
        let _ = std::fs::remove_file(&arc_file);
    }

    /// SQ-1184: `exit_auto_save` must FLUSH any in-flight background per-turn
    /// auto-save before doing its own synchronous write to the same path — or a
    /// slow background write for an EARLIER turn can land AFTER the exit write
    /// and silently overwrite it with stale data, exactly the "quit loses the
    /// last turn" bug the flush exists to prevent.
    ///
    /// Falsifies deterministically rather than by luck: the background job
    /// below carries several MB of incompressible bytes so its Deflate pass
    /// takes measurably longer than `exit_auto_save`'s own tiny synchronous
    /// write. Comment out the `state.archive_worker.flush()` call in
    /// `exit_auto_save` and this test reliably fails with `turns == 999` (the
    /// stale job landing last) instead of `0` (the exit write).
    #[test]
    fn exit_auto_save_flushes_a_pending_background_write_before_its_own_write() {
        let _flag = exit_save_lock();
        let dir = app::scratch_dir("lifecycle-exit-flush");
        let arc_file = dir.join("default.lanthorn");

        let mut engine = SnapshotableEngine::new();
        let mut state = app::state::AppState::default();
        state.config.auto_save = true;
        state.turns = 0; // exit_auto_save's own write carries this turn count
        let mapper = mapper::mapper::Mapper::default();

        // Several MB of incompressible bytes: a real Deflate pass, not a
        // near-instant run of zeros, so this job reliably outlasts
        // exit_auto_save's own write when NOT flushed first.
        let mut noise = vec![0u8; 6 * 1024 * 1024];
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        for b in noise.iter_mut() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *b = x as u8;
        }
        let mut aux = std::collections::BTreeMap::new();
        aux.insert("noise".to_string(), noise);
        let stale_job = app::archive_worker::ArchiveJob {
            path: arc_file.clone(),
            mapper_graph: mapper::mapper::Mapper::default().graph,
            save: std::sync::Arc::new(app::engine::EngineSave::new("test", 1, vec![9, 9, 9])),
            screen: None,
            aux,
            meta: app::archive::Meta {
                format_version: app::archive::CURRENT_FORMAT_VERSION,
                ifid: None,
                name: None,
                turns: 999, // the STALE marker this test must NOT see win
                saved_at: String::new(),
                location: None,
                score: None,
                trigger: app::archive::SaveTrigger::HostState,
            },
            session: app::archive::SessionRecord::empty().snapshot(),
            pictures: Vec::new(),
            display: None,
            ground: None,
        };
        state.archive_worker.enqueue(stale_job);

        super::exit_auto_save(&mut engine, &mapper, &state, "ZCODE-1", &arc_file);

        let meta = app::archive::read_archive_meta(&arc_file).expect("archive readable");
        assert_eq!(
            meta.turns, 0,
            "exit's own write must be the one left on disk, not the stale background job (SQ-1184)"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── SQ-1342: a clean, game-driven quit leaves no resume point ──────────────

    /// The exit path this quest adds: `exit_clear_resume_save` must empty the
    /// save (what `startup.rs`'s auto-load check reads) and drop the
    /// transcript/rewind-history, while the mapper, aux table, and command
    /// history — none of which is specific to the turn the player quit on —
    /// survive exactly as a normal exit leaves them.
    #[test]
    fn exit_clear_resume_save_empties_the_save_but_keeps_mapper_aux_and_command_history_sq1342() {
        let dir = app::scratch_dir("lifecycle-clear-resume");
        let arc_file = dir.join("default.lanthorn");

        let mut engine = SnapshotableEngine::new();
        engine.aux.insert("k".to_string(), vec![7, 7]);
        let mut state = app::state::AppState::default();
        state.config.auto_save = true;
        state.command_history = vec!["look".to_string(), "north".to_string()];
        let mut mapper = mapper::mapper::Mapper::default();
        mapper.observe(1, "Lab", None);

        // Seed the slot exactly as a prior per-turn auto-save would have left it
        // (non-empty save, some transcript/history) — the state a clean quit's
        // OWN per-turn save is now skipped from ever adding to (turn.rs), but
        // an EARLIER turn's write is exactly what must be cleared here.
        let seed_save = app::engine::EngineSave::new("test", 1, vec![1, 2, 3]);
        let seed_meta = app::archive::Meta {
            format_version: app::archive::CURRENT_FORMAT_VERSION,
            ifid: Some("ZCODE-1".to_string()),
            name: None,
            turns: 5,
            saved_at: String::new(),
            location: Some("Lab".to_string()),
            score: Some(10),
            trigger: app::archive::SaveTrigger::HostState,
        };
        let lines = vec!["You are in a lab.".to_string()];
        let kinds = vec![app::state::TranscriptKind::Story];
        let runs = vec![Vec::new()];
        let para = vec![app::state::ParaFmt::default()];
        let images = vec![None];
        let seed_session = app::archive::SessionRecord {
            transcript: &lines,
            kinds: &kinds,
            runs: &runs,
            para: &para,
            images: &images,
            history: &[],
            command_history: &state.command_history,
        };
        app::archive::save_archive_meta_pics(
            &arc_file, &mapper, &seed_save, None, &engine.aux, seed_meta, &seed_session, &[], None, None,
        ).expect("seed a non-empty archive");
        let before = app::archive::load_archive(&arc_file).expect("seeded archive readable");
        assert!(!before.save.is_empty(), "sanity: the seeded save is non-empty");
        assert!(!before.transcript.is_empty(), "sanity: the seeded transcript is non-empty");

        super::exit_clear_resume_save(&mut engine, &mapper, &state, "ZCODE-1", &arc_file);

        let after = app::archive::load_archive(&arc_file).expect("cleared archive readable");
        assert!(after.save.is_empty(), "a clean game quit must leave no resume point (SQ-1342)");
        assert!(after.screen.is_none(), "no screen state after a clean quit");
        assert!(after.transcript.is_empty(), "no transcript after a clean quit");
        assert!(after.history.is_empty(), "no rewind/replay history after a clean quit");
        assert_eq!(after.mapper.graph.rooms().count(), 1, "the mapper survives a clean quit");
        assert_eq!(after.aux.get("k"), Some(&vec![7, 7]), "the aux table survives a clean quit");
        assert_eq!(
            after.command_history, state.command_history,
            "command history survives a clean quit"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Mirrors `exit_auto_save_flushes_a_pending_background_write_before_its_own_write`:
    /// the clearing write must also land any in-flight per-turn auto-save first,
    /// or a slow background write for an earlier turn can overwrite the clearing
    /// write with stale (non-empty) data — silently putting the resume point
    /// right back (SQ-1342).
    #[test]
    fn exit_clear_resume_save_flushes_a_pending_background_write_before_its_own_write_sq1342() {
        let dir = app::scratch_dir("lifecycle-clear-resume-flush");
        let arc_file = dir.join("default.lanthorn");

        let mut engine = SnapshotableEngine::new();
        let mut state = app::state::AppState::default();
        state.config.auto_save = true;
        let mapper = mapper::mapper::Mapper::default();

        // Several MB of incompressible bytes: a real Deflate pass, so this job
        // reliably outlasts the clearing write's own write when NOT flushed first.
        let mut noise = vec![0u8; 6 * 1024 * 1024];
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        for b in noise.iter_mut() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *b = x as u8;
        }
        let mut aux = std::collections::BTreeMap::new();
        aux.insert("noise".to_string(), noise);
        let stale_job = app::archive_worker::ArchiveJob {
            path: arc_file.clone(),
            mapper_graph: mapper::mapper::Mapper::default().graph,
            save: std::sync::Arc::new(app::engine::EngineSave::new("test", 1, vec![9, 9, 9])),
            screen: None,
            aux,
            meta: app::archive::Meta {
                format_version: app::archive::CURRENT_FORMAT_VERSION,
                ifid: None,
                name: None,
                turns: 999, // the STALE marker this test must NOT see win
                saved_at: String::new(),
                location: None,
                score: None,
                trigger: app::archive::SaveTrigger::HostState,
            },
            session: app::archive::SessionRecord::empty().snapshot(),
            pictures: Vec::new(),
            display: None,
            ground: None,
        };
        state.archive_worker.enqueue(stale_job);

        super::exit_clear_resume_save(&mut engine, &mapper, &state, "ZCODE-1", &arc_file);

        let ac = app::archive::load_archive(&arc_file).expect("archive readable");
        assert!(
            ac.save.is_empty(),
            "the clearing write must be the one left on disk, not the stale (non-empty) background job"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
