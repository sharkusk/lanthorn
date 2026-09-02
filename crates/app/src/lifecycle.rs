//! Exit / quit persistence paths: exit auto-save, the quit-dialog "Save State &
//! quit" snapshot, and the pending config-write flush. Extracted verbatim from
//! `main.rs` (SQ-0306) as a pure move — no behavior change. The SQ-0283
//! save/restore-pending guards and the auto-save gate move intact inside the
//! bodies. Helper fns these rely on stay in `main.rs` (referenced via `crate::`).

use mapper::mapper::Mapper;

use app::engine::Engine;
use app::state::AppState;

use crate::engine_helpers::zvm_session_opt;
use crate::format_rfc3339;

/// Save on exit ONLY when auto_save is enabled. With auto_save off (the default),
/// nothing is saved automatically — the user controls saving via the quit prompt's
/// "Save State & quit", the /save-state command, or named save slots. This keeps
/// "Quit without saving" honest and avoids silently overwriting an explicit save
/// point on exit.
/// Exit auto-save is engine-neutral: the save routes through Engine::save_state
/// (Quetzal for zvm, the gvm snapshot for Glulx); screen.json is written for
/// zvm only.
/// Skip while a Glulx in-game @save/@restore is suspended, awaiting host I/O:
/// snapshotting mid-suspension would capture the un-popped @save call stub,
/// and restore_state never pops it -> a corrupted stack on a later Save State
/// restore (SQ-0283 carry-forward fix). The in-game save the player was
/// already making is the relevant persistence in that case.
pub(crate) fn exit_auto_save(
    session: &mut dyn Engine,
    mapper: &Mapper,
    state: &app::state::AppState,
    ifid: &str,
    arc_file: &std::path::Path,
) {
    if !state.config.auto_save || session.is_saveload_pending() {
        return;
    }
    // Tell the termination watchdog a save is actively running so its fixed grace
    // does not kill the process mid-write and lose it (SQ-0651 / partial SQ-0644).
    // Held for the whole snapshot+write; cleared on drop, unwind included.
    let _writing = crate::ExitSaveGuard::new();
    // Land any in-flight background auto-save write (SQ-1184) BEFORE this
    // function does its own synchronous write to the same path — otherwise a
    // background write still catching up on the last turn could finish AFTER
    // this one and overwrite the exit save with a stale turn, or the two
    // could interleave onto the file. See `archive_worker::ArchiveWorker::flush`.
    state.archive_worker.flush();
    let (location, score) = crate::engine_helpers::save_summary(session, state);
    let exit_meta = app::archive::Meta {
        format_version: app::archive::CURRENT_FORMAT_VERSION,
        ifid: Some(ifid.to_string()),
        name: None,
        turns: state.turns,
        saved_at: format_rfc3339(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        ),
        location,
        score,
        trigger: app::archive::SaveTrigger::HostState,
    };
    let (v6_pics, v6_display, v6_ground, v6_diags) = crate::engine_helpers::v6_save_payload(session);
    for d in &v6_diags { state.note_v6_save(d); }
    match app::archive::save_archive_meta_pics(arc_file, mapper, &session.save_state(), zvm_session_opt(session).map(|z| &z.machine.screen), session.aux_data(), exit_meta, &app::archive::SessionRecord::of(state), &v6_pics, v6_display.as_ref(), v6_ground.as_deref()) {
        Ok(()) => {
            eprintln!("lanthorn: map saved to {}", arc_file.display());
        }
        Err(e) => {
            eprintln!("lanthorn: warning: could not save to {}: {}", arc_file.display(), e);
        }
    }
}

/// Quit-dialog "Save State & quit" host snapshot, extracted from the quit-dialog
/// keyboard and mouse handlers so the guard below is unit-testable.
/// Skip while a Glulx in-game @save/@restore is suspended, awaiting host I/O:
/// snapshotting mid-suspension would capture the un-popped @save call stub, and
/// restore_state never pops it -> a corrupted stack on a later Save State
/// restore (SQ-0283 carry-forward fix). The in-game save the player was already
/// making is the relevant persistence in that case; the dialog still proceeds
/// to quit either way.
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
    if session.is_saveload_pending() {
        return None;
    }
    // See the matching comment in `exit_auto_save` (SQ-1184): this writes the
    // same path a background per-turn auto-save may still be catching up on.
    state.archive_worker.flush();
    let (location, score) = crate::engine_helpers::save_summary(session, state);
    let meta = app::archive::Meta {
        format_version: app::archive::CURRENT_FORMAT_VERSION,
        ifid: Some(ifid.to_string()),
        name: None,
        turns: state.turns,
        saved_at: format_rfc3339(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        ),
        location,
        score,
        trigger: app::archive::SaveTrigger::HostState,
    };
    let (v6_pics, v6_display, v6_ground, v6_diags) = crate::engine_helpers::v6_save_payload(session);
    for d in &v6_diags { state.note_v6_save(d); }
    match app::archive::save_archive_meta_pics(arc_file, mapper, &session.save_state(), zvm_session_opt(session).map(|z| &z.machine.screen), session.aux_data(), meta, &app::archive::SessionRecord::of(state), &v6_pics, v6_display.as_ref(), v6_ground.as_deref()) {
        Ok(()) => None,
        Err(e) => Some(format!(
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
}
