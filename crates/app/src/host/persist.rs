//! Host saves and restores (SQ-1539): the exit save that leaves a resume point,
//! the clean-quit clear that leaves none, a Save State on demand, and loading a
//! save file or archive back in.
//!
//! The per-turn auto-save already runs inside the per-turn apply
//! ([`super::turn`]); these are the other doors to the same archive. What only
//! the TUI has — the termination watchdog it tells a write is in flight, the
//! stderr it reports the outcome on — stays in its `lifecycle.rs`, which wraps
//! these.

use mapper::mapper::Mapper;

use crate::engine::Engine;
use crate::engine_helpers::{
    apply_archive_state, engine_tag, restore_error_msg, restore_from_file, zvm_session_opt, RestoreOutcome,
};
use crate::state::AppState;

use super::turn::{now_rfc3339, reobserve_location, TurnOutcome};

/// What an exit-time save did.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub enum ExitSave {
    /// Nothing written: auto-save is off, or an in-game `@save`/`@restore` is
    /// suspended mid-flight (a snapshot then would capture the un-popped call
    /// stub — SQ-0283).
    Skipped,
    Saved,
    Failed(String),
}

/// Write a host Save State of the running session to `arc_file`: map, engine
/// snapshot, Z-machine screen, v6 display list, aux table, transcript, history.
/// Lands any in-flight background auto-save first (SQ-1184) so a stale one
/// cannot finish after it and overwrite it.
fn write_save_state(
    session: &mut dyn Engine,
    mapper: &Mapper,
    state: &AppState,
    ifid: &str,
    arc_file: &std::path::Path,
) -> Result<(), String> {
    state.archive_worker.flush();
    let (location, score) = crate::engine_helpers::save_summary(session, state);
    let meta = crate::archive::Meta {
        format_version: crate::archive::CURRENT_FORMAT_VERSION,
        ifid: Some(ifid.to_string()),
        name: None,
        turns: state.turns,
        saved_at: now_rfc3339(),
        location,
        score,
        trigger: crate::archive::SaveTrigger::HostState,
        source: state.source.clone(),
    };
    let (v6_pics, v6_display, v6_ground, v6_diags) = crate::engine_helpers::v6_save_payload(session);
    for d in &v6_diags {
        state.note_v6_save(d);
    }
    crate::archive::save_archive_meta_pics(
        arc_file,
        mapper,
        &session.save_state(),
        zvm_session_opt(session).map(|z| &z.machine.screen),
        session.aux_data(),
        meta,
        &crate::archive::SessionRecord::of(state),
        &v6_pics,
        v6_display.as_ref(),
        v6_ground.as_deref(),
    )
    .map_err(|e| e.to_string())
}

/// Save on exit ONLY when auto_save is enabled (the default, since SQ-1624).
/// With auto_save off, nothing is saved automatically on exit — the user
/// controls saving via the quit prompt's "Save State & quit", the
/// /save-state command, or named save slots. This keeps "Quit without
/// saving" honest and avoids silently overwriting an explicit save point on
/// exit.
///
/// Exit auto-save is engine-neutral: the save routes through Engine::save_state
/// (Quetzal for zvm, the gvm snapshot for Glulx); screen.bin is written for
/// zvm only.
///
/// Skip while a Glulx in-game @save/@restore is suspended, awaiting host I/O:
/// snapshotting mid-suspension would capture the un-popped @save call stub,
/// and restore_state never pops it -> a corrupted stack on a later Save State
/// restore (SQ-0283 carry-forward fix). The in-game save the player was
/// already making is the relevant persistence in that case.
///
/// The next [`boot_story`](super::boot_story) of the same story resumes from
/// what this writes (with `auto_load` on).
pub fn exit_auto_save(
    session: &mut dyn Engine,
    mapper: &Mapper,
    state: &AppState,
    ifid: &str,
    arc_file: &std::path::Path,
) -> ExitSave {
    if !state.config.auto_save || session.is_saveload_pending() {
        return ExitSave::Skipped;
    }
    match write_save_state(session, mapper, state, ifid, arc_file) {
        Ok(()) => ExitSave::Saved,
        Err(e) => ExitSave::Failed(e),
    }
}

/// Exit path for a CLEAN, game-driven quit (SQ-1342): rewrite the archive with
/// no resume point instead of [`exit_auto_save`]'s snapshot, so reopening the
/// story starts it fresh rather than one turn before the player typed `quit`.
///
/// "No resume point" means an [`crate::engine::EngineSave`] with empty `bytes`
/// (`ArchiveContents::save.is_empty()` is what the boot's auto-load check
/// reads) and no screen/transcript/history — but the mapper (the player's own
/// knowledge of the map), the aux table, and the command history all survive a
/// clean finish exactly as they do today, because nothing about THOSE is
/// specific to the turn the player quit on.
///
/// Called instead of, never alongside, [`exit_auto_save`] — when
/// `state.game_ended` is set.
pub fn exit_clear_resume_save(
    session: &mut dyn Engine,
    mapper: &Mapper,
    state: &AppState,
    ifid: &str,
    arc_file: &std::path::Path,
) -> ExitSave {
    if !state.config.auto_save || session.is_saveload_pending() {
        return ExitSave::Skipped;
    }
    // Land any in-flight per-turn auto-save BEFORE this write (see the matching
    // comment in `write_save_state`): a background write for an earlier turn that
    // lands AFTER this one would silently put the resume point right back.
    state.archive_worker.flush();
    // A full save just to discard its bytes looks wasteful, but this runs once,
    // at exit, and it is the only way to get the CORRECT engine tag/format
    // version stamped on an empty save — the same ones a real save on this
    // engine would carry, so a hand-rolled shortcut can't drift from them.
    // `write_cleared_resume_archive` (`crate::archive`) is the shared, testable
    // core: it discards `save.bytes` itself and writes no transcript/history/
    // screen, keeping only the mapper/aux (passed through) and command history.
    let save = session.save_state();
    match crate::archive::write_cleared_resume_archive(
        arc_file,
        mapper,
        &save,
        session.aux_data(),
        ifid,
        now_rfc3339(),
        &state.command_history,
        &state.source,
    ) {
        Ok(()) => ExitSave::Saved,
        Err(e) => ExitSave::Failed(e.to_string()),
    }
}

/// The session's exit: a clean game-driven quit clears the resume point
/// ([`exit_clear_resume_save`]), anything else leaves one
/// ([`exit_auto_save`]) — the choice the TUI makes when its loop ends.
pub fn exit_save(
    session: &mut dyn Engine,
    mapper: &Mapper,
    state: &AppState,
    ifid: &str,
    arc_file: &std::path::Path,
) -> ExitSave {
    if state.game_ended {
        exit_clear_resume_save(session, mapper, state, ifid, arc_file)
    } else {
        exit_auto_save(session, mapper, state, ifid, arc_file)
    }
}

/// A host Save State to `arc_file` NOW, whatever `auto_save` says — the quit
/// dialog's "Save State & quit". Skipped only while an in-game `@save`/`@restore`
/// is suspended, for the reason [`exit_auto_save`] gives.
pub fn save_state_now(
    session: &mut dyn Engine,
    mapper: &Mapper,
    state: &AppState,
    ifid: &str,
    arc_file: &std::path::Path,
) -> ExitSave {
    if session.is_saveload_pending() {
        return ExitSave::Skipped;
    }
    match write_save_state(session, mapper, state, ifid, arc_file) {
        Ok(()) => ExitSave::Saved,
        Err(e) => ExitSave::Failed(e),
    }
}

/// What [`restore_file`] brought back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restored {
    /// A game save (an in-game `@save` archive, a bare `.qzl`, a Scott save)
    /// completed the pending `@save` descriptor. `with_session` when it was an
    /// archive that carried the map/transcript/screen alongside its bytes
    /// (SQ-0531), which were reinstated too.
    GameSave { with_session: bool },
    /// A host Save State resumed the whole session.
    Resumed,
}

/// Restore `path` — a `.lanthorn` archive or a bare game save — into the running
/// session and reinstate everything the archive carries, then re-observe where
/// the player now stands. The shared core of `/restore-state` and the saves
/// manager's Load.
pub fn restore_file(
    session: &mut dyn Engine,
    mapper: &mut Mapper,
    state: &mut AppState,
    path: &std::path::Path,
    map_view: Option<(u16, u16)>,
) -> Result<Restored, String> {
    let restored = match restore_from_file(path, session)? {
        RestoreOutcome::DescriptorCompleted(ac) => {
            // An in-game @save archive carries the whole session alongside its
            // game bytes (SQ-0531); a bare .qzl has nothing but the bytes.
            let with_session = ac.is_some();
            if let Some(ac) = ac {
                apply_archive_state(*ac, session, mapper, state);
            }
            Restored::GameSave { with_session }
        }
        RestoreOutcome::Resumed(ac) => {
            apply_archive_state(*ac, session, mapper, state);
            Restored::Resumed
        }
    };
    reobserve_location(state, mapper, &*session, map_view);
    Ok(restored)
}

/// Answer the game's own pending `@restore` with the game save at `path` —
/// or, with `None`, tell it the restore failed.
///
/// A `.lanthorn` written by lanthorn's own `@save` also reinstates the session
/// it carries BEFORE the game resumes, so the game's own post-restore output
/// lands at the end of the restored scrollback instead of being wiped by it.
/// `name` is what the notice calls the save.
#[allow(clippy::too_many_arguments)]
pub fn answer_ingame_restore(
    session: &mut dyn Engine,
    mapper: &mut Mapper,
    state: &mut AppState,
    save: Option<(&std::path::Path, &str)>,
    game_dir: &std::path::Path,
    ifid: &str,
    map_view: Option<(u16, u16)>,
) -> TurnOutcome {
    state.overlays.saves = None;
    state.ingame_io = None;
    let result = match save.map(|(path, name)| (path, name, crate::archive::read_quetzal_from_file(path))) {
        Some((path, name, Ok(bytes))) => {
            if !crate::persist_files::is_game_save(path) {
                match crate::archive::load_archive(path) {
                    Ok(ac) => apply_archive_state(ac, session, mapper, state),
                    Err(e) => state.push_notice(&format!("[Save State sidecars unreadable: {}]", e)),
                }
            } else {
                // A bare .qzl carries no screen, so the restored
                // game's layout width has to be assumed
                // (`note_bare_quetzal_width`, SQ-0681). Raised on
                // the attempt: `resume_restore` reports a refused
                // save only as the game's own "Failed.", and the
                // guard only ever widens the declared screen.
                crate::engine_helpers::note_bare_quetzal_width(session);
            }
            state.push_notice(&format!("[Game restored from {}]", name));
            session.resume_restore(Some(&bytes))
        }
        Some((_, _, Err(e))) => {
            state.push_notice(&format!("[Restore failed: {}]", e));
            session.resume_restore(None)
        }
        None => session.resume_restore(None),
    };
    let out = super::turn::finish_resumed_turn(result, mapper, state, session, game_dir, ifid, map_view);
    super::turn::persist_aux_after_turn(session, state, game_dir);
    super::turn::persist_vfs_after_turn(session, state, game_dir);
    if let Some(io) = state.ingame_io {
        super::ingame_io::open_ingame_saves(io, game_dir, state);
    }
    out
}

/// What [`load_save`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[must_use]
pub struct Loaded {
    /// The game ended on the turn the load resumed.
    pub quit: bool,
    /// A pending in-game `@restore` was answered, so a game turn ran.
    pub answered_game: bool,
}

/// Load a save the player picked from this story's saves (`path`, shown as
/// `name`, written by `trigger`) — the saves manager's Load.
///
/// While the game itself is waiting on a `@restore`, a GAME save (a bare `.qzl`
/// from another interpreter, or a `.lanthorn` that lanthorn's own `@save` wrote,
/// SQ-0531) answers it: its descriptor-PC bytes go back into the suspended VM
/// ([`answer_ingame_restore`]). Anything else — or no pending `@restore` — is a
/// host load ([`restore_file`]): a Save State picked while a `@restore` is
/// pending fully resumes and abandons the pending call, and on failure the
/// pending `@restore` is still answered with a failure so the VM is not left
/// blocked waiting for a result (SQ-0227).
#[allow(clippy::too_many_arguments)]
pub fn load_save(
    session: &mut dyn Engine,
    mapper: &mut Mapper,
    state: &mut AppState,
    (path, name, trigger): (&std::path::Path, &str, crate::archive::SaveTrigger),
    game_dir: &std::path::Path,
    ifid: &str,
    map_view: Option<(u16, u16)>,
) -> Loaded {
    let ingame_restore_pending = state.ingame_io == Some(crate::session::PendingIo::Restore);
    if ingame_restore_pending && trigger.is_portable() {
        let out = answer_ingame_restore(session, mapper, state, Some((path, name)), game_dir, ifid, map_view);
        return Loaded { quit: out.quit, answered_game: true };
    }
    match restore_file(session, mapper, state, path, map_view) {
        Ok(Restored::GameSave { with_session }) => {
            state.overlays.saves = None;
            if with_session {
                state.ingame_io = None;
                state.pending_filename = None;
            }
            state.push_notice(&format!("[Game restored from {}]", name));
        }
        Ok(Restored::Resumed) => {
            state.ingame_io = None;
            // A restore abandons any suspended create_by_prompt in the
            // session, so the host-side request must not outlive it and
            // fire a spurious resume_filename turn.
            state.pending_filename = None;
            state.push_notice(&format!("[Loaded save: {}]", name));
            state.overlays.saves = None;
        }
        Err(e) => {
            state.push_notice(&format!("[Load failed: {}]", e));
            if ingame_restore_pending {
                let out = answer_ingame_restore(session, mapper, state, None, game_dir, ifid, map_view);
                return Loaded { quit: out.quit, answered_game: true };
            }
        }
    }
    Loaded::default()
}

/// What [`resume_from_turn`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[must_use]
pub struct Rewound {
    /// The resume completed: the engine, transcript, map and history all now
    /// reflect the picked turn. `false` when `idx` was out of range for
    /// `state.history` (nothing was touched) or the engine refused the restore
    /// (a `[Resume failed: …]` notice was already pushed either way).
    pub ok: bool,
    /// Something on screen changed, for a host that doesn't blanket-redraw
    /// every action — mirrors [`super::clock::Fired::redraw`]. The TUI's own
    /// event loop already forces a redraw unconditionally before dispatching
    /// any action, so this matters mainly to a non-terminal host. `false` only
    /// when `idx` was out of range and nothing happened at all.
    pub redraw: bool,
}

/// Resume play from a recorded rewind/replay turn (`state.history[idx]`): restore
/// the engine to that turn's snapshot, swap in the map as it stood at-or-before
/// it, truncate history to `[0..=idx]` (a linear rewind — turns after it are
/// discarded, so playing on overwrites them, the way typing over a rewound point
/// works in any other IF interpreter), rebuild the transcript to match, reset the
/// transcript sidecars and the scraped word set, and re-observe the resumed room.
/// The shared core of the rewind/replay modal's Resume — every host reaches
/// `state.history` through the same picker, so this is the one place that reads
/// it back out.
///
/// Re-observing the room is [`reobserve_location`]'s job, not hand-rolled here:
/// it also recenters the map pane on the resumed room, which the TUI's own
/// former inline version of this code did NOT do — a genuine improvement that
/// falls out of sharing the helper every other restore/resume arm already uses,
/// not something added deliberately beyond what this dedup was asked for.
///
/// **Does not autosave.** Resuming only changes in-memory state and pushes a
/// notice; nothing is written to the archive, matching the TUI's existing
/// behaviour today. A host that wants the resumed turn to survive a crash or
/// relaunch has to trigger its own Save State afterwards — every host now gets
/// this same (non-autosaving) behaviour uniformly, by calling this one function.
///
/// `idx` out of range for `state.history` is a no-op: returns `Rewound::default()`
/// with nothing touched and no notice pushed.
pub fn resume_from_turn(
    session: &mut dyn Engine,
    mapper: &mut Mapper,
    state: &mut AppState,
    idx: usize,
    map_view: Option<(u16, u16)>,
) -> Rewound {
    if idx >= state.history.len() {
        return Rewound::default();
    }
    let plan = crate::history::resume_plan(&state.history, idx);
    // History snapshots come from the running engine; wrap them with its tag so
    // restore_state accepts them (both engines).
    let es = crate::engine::EngineSave::new(engine_tag(&*session), 1, plan.save.clone());
    if let Err(e) = session.restore_state(&es) {
        state.push_notice(&format!("[Resume failed: {}]", restore_error_msg(e)));
        return Rewound { ok: false, redraw: true };
    }
    if let Some(json) = &plan.map_json {
        if let Ok(m) = mapper::persist::from_json(json) {
            *mapper = m;
        }
    }
    // Linear: discard later turns.
    state.history.truncate(idx + 1);
    if !cut_back_transcript(state, idx) {
        let (lines, kinds) = crate::history::rebuild_transcript(&state.history, idx);
        state.transcript = lines;
        // History replay carries the game's clears as a flag per turn, so the rewound
        // transcript keeps the same scrollback hidden (SQ-1713).
        let anchor = crate::history::rebuild_clear_anchor(&state.history, idx);
        state.clear_anchor = anchor;
        state.top_anchor = anchor;
        state.pending_clear = false;
        state.transcript_kinds = kinds;
        // The rebuild carries no style runs; keep the parallel vecs length-synced
        // (unstyled, left rows).
        state.transcript_runs = vec![Vec::new(); state.transcript.len()];
        state.transcript_para = vec![crate::state::ParaFmt::default(); state.transcript.len()];
        state.reset_transcript_sidecars();
    }
    // Rebuilt from the replayed transcript (SQ-1135): a rewind to turn 4 offers
    // the words turn 4 had printed.
    crate::input::refresh_seen_words(state, &*session);
    state.turns = plan.turn;
    reobserve_location(state, mapper, &*session, map_view);
    state.push_notice(&format!("[Resumed from turn {}]", plan.turn));
    Rewound { ok: true, redraw: true }
}

/// Cut the LIVE transcript back to what record `idx` left behind (SQ-1715), so a
/// rewind keeps the styled lines the player saw -- including the output of
/// game-driven turns (a key, a timer) that record nothing and that a rebuild from
/// the records cannot know. Valid only while the record's epoch is still the
/// live one, i.e. nothing has shortened the transcript or moved an earlier line
/// since; otherwise (and for a record without a mark, e.g. loaded from an
/// archive) returns `false` having touched nothing, and the caller rebuilds.
fn cut_back_transcript(state: &mut AppState, idx: usize) -> bool {
    let rec = state.history[idx].clone();
    let (Some(len), Some(epoch), Some(tail), Some(anchors)) =
        (rec.transcript_len, rec.transcript_epoch, rec.transcript_tail_chars, rec.transcript_anchors)
    else {
        return false;
    };
    if epoch != state.transcript_epoch || len > state.transcript.len() {
        return false;
    }
    state.truncate_transcript(len);
    // The line the player was prompted on has since been grown (the echoed
    // command, a folded game echo); trim it back to the prompt it was.
    state.shrink_last_transcript_line(tail);
    // Everything up to `len` is exactly as it was at the record, so the records
    // up to `idx` stay comparable: truncating bumped the epoch, put it back.
    state.transcript_epoch = epoch;
    state.clear_anchor = anchors.clear;
    state.top_anchor = anchors.top;
    state.pending_clear = false;
    state.reset_transcript_derived();
    true
}
