//! The per-turn apply (SQ-1538): what a finished turn does to the transcript,
//! the map, the sound, the `[more]` pager and the save bookkeeping — for a
//! submitted command, a resumed in-game save/restore, and a game-driven turn
//! (a keypress read, a timed-input interrupt, a Glk timer, a sound routine).
//!
//! Moved out of the binary's `turn.rs` (itself extracted from `main.rs` by
//! SQ-0306) so a host that is not a terminal applies a turn by the same rules
//! the TUI does. None of it ever called crossterm; what tied it to the terminal
//! was a `ratatui::layout::Rect` for the map pane, used only to recenter the map
//! on the current room. That is now `map_view` — the pane's `(cols, rows)`, or
//! `None` for a host that draws no map — and playback goes through the state's
//! [`SoundSink`](super::sound::SoundSink) rather than straight to a device. The
//! Wave 1 invariant calls (the map memo's `Mapper::struct_gen` read after
//! `apply_turn` — SQ-1544, replacing a hand-bumped `graph_gen` — transcript
//! generation bumps inside `push_*`) move intact inside the bodies.
//!
//! Every entry point returns a [`TurnOutcome`]: whether the game ended, and the
//! pager's verdict on the output, so a host that paginates by its own viewport
//! knows where a pause belongs.

use std::time::Duration;

use mapper::mapper::Mapper;

use crate::archive::load_archive;
use crate::engine::Engine;
use crate::host::{flush_screen_trace, flush_v6_trace};
use crate::tidy::{cleanup_overlaps_layer_silent, should_bg_tidy, tidy_layer_silent};
use crate::session::{apply_turn, InputKind, TurnResult};
use crate::state::{AppState, SoundPulse, TidyJob, TidyKind, TranscriptKind};
use crate::storage::default_state_path;

use crate::engine_helpers::{restore_error_msg, zvm_session_opt, zvm_session_opt_mut};
use super::ingame_io::{open_filename_modal, open_ingame_saves};

/// What a turn left for the host to act on. `#[must_use]`: a dropped `quit` is
/// a game that ended while the host kept waiting for input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[must_use]
pub struct TurnOutcome {
    /// The game ended cleanly (`@quit`, `glk_exit`) and the session should close.
    /// A VM fault is NOT a quit — the game halts and the host stays up.
    pub quit: bool,
    /// The `[more]` pager's verdict on this turn's output.
    pub paging: Paging,
}

/// The host's per-session facts a player turn is applied against (SQ-1568): the
/// trailing arguments [`finish_command_turn`] takes, as one value, so an entry
/// point that may END in a command turn ([`crate::host::input::deliver_v6_click`])
/// is handed all of them together rather than a subset.
///
/// `map_view` is the map pane's `(cols, rows)`, or `None` for a host with no map;
/// `bg_tidy_counter` is the host's debounce counter for background map tidies.
pub struct TurnCtx<'a> {
    pub game_dir: &'a std::path::Path,
    pub ifid: &'a str,
    pub arc_file: &'a std::path::Path,
    pub map_view: Option<(u16, u16)>,
    pub bg_tidy_counter: &'a mut u32,
}

/// The `[more]` pager's verdict on one turn, for a host that paginates by its
/// own viewport (SQ-1538). The TUI's pager ([`crate::pager`]) measures wrapped
/// rows at the next frame; this is the same decision in transcript LINES, which
/// is what a host with a different layout can use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Paging {
    /// The pager would arm for this output ([`crate::pager::should_arm`]): the
    /// game is now waiting on a line or a key and nothing vetoes `[more]`. A
    /// host whose viewport cannot show everything from `first_line` on pauses
    /// there first.
    pub would_arm: bool,
    /// The v6 "never print [MORE]" veto is in force ([`crate::pager::more_suppressed`],
    /// ZMSD §8.8.3.2.6) — which is also why `would_arm` is false.
    pub more_suppressed: bool,
    /// The first transcript line (`AppState::transcript` index) this turn's
    /// output landed on: where a paginating host's pause belongs.
    pub first_line: usize,
}

impl Paging {
    /// The verdict for output that began at `first_line`, with the game now
    /// waiting on `pending`.
    fn after(first_line: usize, pending: InputKind, session: &dyn Engine) -> Paging {
        let more_suppressed = crate::pager::more_suppressed(session);
        Paging {
            would_arm: crate::pager::should_arm(pending, more_suppressed),
            more_suppressed,
            first_line,
        }
    }
}

/// Recenter the map pane on `rid` if the host shows a map (`map_view` is its
/// `(cols, rows)`) and the room has been placed.
fn recenter_on_room(state: &mut AppState, mapper: &Mapper, rid: mapper::graph::RoomId, map_view: Option<(u16, u16)>) {
    if let (Some(pos), Some((pw, ph))) = (mapper.graph.room(rid).and_then(|r| r.pos), map_view) {
        state.recenter_on(pos, pw, ph);
    }
}

/// Whether the game echoed the just-submitted command itself at the start of its
/// turn output (e.g. CounterfeitMonkey prints the command back in bold). A genuine
/// self-echo repeats the WHOLE command as its own first line — compared
/// case-insensitively — not merely a leading word: `north` from West of House
/// prints the room heading "North of House", which starts with "north" but is
/// not an echo of it, and treating it as one swallowed the player's typed command
/// from the transcript entirely (SQ-1546). An empty command never matches.
pub fn game_echoes_command(transcript: &str, cmd: &str) -> bool {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return false;
    }
    let first_line = transcript.trim_start().split('\n').next().unwrap_or("");
    first_line.trim_end().eq_ignore_ascii_case(cmd)
}

/// Format a Unix timestamp (seconds since epoch) as an RFC3339 UTC string.
pub fn format_rfc3339(secs: u64) -> String {
    let sec = secs % 60;
    let min = (secs / 60) % 60;
    let hour = (secs / 3600) % 24;
    let days = secs / 86400;
    let (year, month, day) = days_to_ymd(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", year, month, day, hour, min, sec)
}

/// Now, as [`format_rfc3339`] writes it — every save's `Meta::saved_at`.
pub fn now_rfc3339() -> String {
    format_rfc3339(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    )
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    days += 719468;
    let era = days / 146097;
    let doe = days % 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Re-observe the VM's current location after a restore/resume: fold the room into the
/// map, deselect the viewed layer, select the room, and recenter the map pane on it.
/// Produces no transcript output. Shared by every host restore/resume arm.
pub fn reobserve_location(
    state: &mut AppState,
    mapper: &mut Mapper,
    session: &dyn Engine,
    map_view: Option<(u16, u16)>,
) {
    // Every caller is a restore/resume/import: the live state now equals a saved
    // one, so there is no unsaved progress to warn about on quit.
    state.unsaved_progress = false;
    // The caller has just swapped in a restored/imported mapper (or is about to
    // re-observe into it); invalidate the map render memo so the loaded map shows
    // this frame instead of the pre-restore one. Unconditional so even the
    // no-current-location early-return below still invalidates. A wholesale
    // mapper swap starts `struct_gen` back at 0 (see its own doc comment), so a
    // generation-number comparison alone could coincidentally match the stale
    // cache's — this drops the cache and any in-flight job outright instead
    // (SQ-0305, SQ-1544).
    state.invalidate_map_render();
    // The restored game is not the one the death watch was watching: a death outstanding in the
    // live session says nothing about the saved one, and the re-observation below is itself a room
    // change with no passage behind it. Cleared before the early return, so a restore into a game
    // that reports no location does not carry the old one's death either. (SQ-0671, SQ-0673)
    state.death_watch = Default::default();
    let Some(snap) = session.current_location() else { return };
    let rid = snap.number as mapper::graph::RoomId;
    let restore_result = TurnResult::observation(snap);
    apply_turn(mapper, "", &restore_result, &mut state.death_watch);
    state.set_viewed_layer(None);
    state.select_room(Some(rid));
    recenter_on_room(state, mapper, rid, map_view);
}

/// **A line read that ended on a terminating character echoed no newline**, so
/// the host must not invent one (SQ-0881).
///
/// The newline ZMSD §7.1.1.1 has an interpreter echo after a `read` is the echo
/// of the newline the player TYPED. From Version 5 a read can instead end on a
/// character the game listed in its terminating-characters table (§10.7), and
/// then nothing was typed and nothing was echoed. Arthur lists the four arrows,
/// F1–F6 (ZSCII 133–138) and both mouse clicks, so pressing F2 for its map is a
/// read that ends with no newline anywhere in it.
///
/// The host adds a transcript line per turn, and that line IS its way of
/// supplying §7.1.1.1's newline — so on such a turn it supplies one the game
/// never echoed, and the player watches the cursor drop a line for every
/// function key they press.
///
/// True only when the turn has nothing whatsoever to show: no typed text to
/// echo, and no output. A terminator that ends a line the player DID type still
/// echoes that text, and a game that prints in response still gets its line —
/// this silences a turn that was already silent, and nothing else.
pub fn silent_terminator_turn(
    cmd: &str,
    ended_on_newline: bool,
    result: &TurnResult,
) -> bool {
    use crate::session::TranscriptElem;
    !ended_on_newline
        && cmd.is_empty()
        && result.transcript.is_empty()
        && result.info.is_none()
        && result
            .transcript_elems
            .iter()
            .all(|e| matches!(e, TranscriptElem::Text { text, .. } if text.is_empty()))
}

/// Apply a completed game-turn `result` from a submitted command line: echo the
/// command, push its transcript, advance the mapper, run post-turn bookkeeping /
/// auto-save / background tidy, and recenter on the current room. Shared by the
/// normal `SubmitCommand` path and the terminator-key submit gate (SQ-0188).
///
/// `ended_on_newline` is whether the read ended on Enter rather than a
/// terminating character (see [`silent_terminator_turn`]); `map_view` is the map
/// pane's `(cols, rows)` to recenter in, or `None` for a host with no map;
/// `bg_tidy_counter` is the host's debounce counter for background map tidies.
///
/// Per-command bookkeeping (SQ-1545) runs first, unconditionally: `cmd` is
/// recorded into shell-style command history (a no-op if a caller already
/// recorded the same line — see below), the turn counter advances, and
/// progress is marked unsaved (drives the quit prompt). This used to be three
/// lines every caller had to repeat immediately before calling this function;
/// a headless host that just does `session.submit` + `finish_command_turn` now
/// gets the same history/turn-count/autosave behaviour the TUI does, with no
/// extra lines of its own. The TUI's own slash-command branch still records a
/// command into history BEFORE routing it (a slash command never reaches this
/// function at all, so it has to record itself) — when that same line lands
/// here as an ordinary command, the record below is a harmless consecutive-
/// duplicate no-op (`AppState::record_command`'s own dedupe).
#[allow(clippy::too_many_arguments)]
pub fn finish_command_turn(
    cmd: &str,
    ended_on_newline: bool,
    mut result: TurnResult,
    state: &mut AppState,
    mapper: &mut Mapper,
    session: &mut dyn Engine,
    game_dir: &std::path::Path,
    ifid: &str,
    arc_file: &std::path::Path,
    map_view: Option<(u16, u16)>,
    bg_tidy_counter: &mut u32,
) -> TurnOutcome {
    if !cmd.is_empty() {
        state.record_command(cmd);
    }
    state.turns += 1;
    state.unsaved_progress = true;
    // The player has typed again, so anything the shadow is still working on for
    // an earlier turn is stale (SQ-1124).
    state.begin_turn();
    if result.erase_lower { state.mark_screen_clear(); }
    // A read that ended on a terminating character with nothing typed and
    // nothing printed adds nothing to the transcript — see
    // [`silent_terminator_turn`]. Everything below this point that is NOT the
    // transcript (the map, the turn events, the save bookkeeping) still runs.
    let silent = silent_terminator_turn(cmd, ended_on_newline, &result);
    // Some games echo the typed command themselves at the start of their turn
    // output (e.g. CounterfeitMonkey prints it back in bold). Adding our own echo
    // on top would show the command twice, so detect that and skip ours. Most
    // games don't self-echo, so they still get our echo below.
    let self_echo = silent || game_echoes_command(&result.transcript, cmd);
    // When the game self-echoes AND we're inline with the `>` as the last line,
    // fold the game's echo onto that prompt line (below) so it reads `>look` at
    // the prompt, with the game's own styling — instead of a detached line.
    let merge_echo = self_echo && !state.config.command_bar && state.last_transcript_line_is_story();
    if self_echo {
        // Game provides the echo; add nothing of our own.
    } else if state.config.command_bar || !state.last_transcript_line_is_story() {
        // Command-bar mode, or inline mode where the game's `>` is NOT the last
        // line (e.g. a `/help` Meta dump intervened): echo on its own line so we
        // never corrupt non-prompt scrollback.
        state.push_transcript_kind(&format!("> {}", cmd), TranscriptKind::Input);
    } else {
        // Inline mode: the game's own `>` is already the last transcript line;
        // append the typed command so `>look` persists in scrollback.
        state.append_to_last_transcript_line(cmd);
    }
    let before_push = state.transcript.len();
    if silent {
        // Nothing to push: the turn printed nothing and the read ate no newline.
    } else if result.transcript_elems.is_empty() {
        state.push_transcript_runs(&result.transcript, TranscriptKind::Story, &result.transcript_runs);
    } else {
        crate::state::apply_transcript_elems(state, &result.transcript_elems);
    }
    // Where this turn's output begins, for the pager report below.
    let mut first_line = before_push;
    if merge_echo && state.transcript.len() > before_push {
        // Fold the game's own echo (its first output line) onto the `>` prompt.
        // The game printed the echo in the default colour; preserve the current
        // page colours on the folded line rather than resetting it to the theme.
        let prevailing = state.prevailing_run_colour_before(before_push);
        state.merge_line_into_previous(before_push);
        if let Some((fg, bg)) = prevailing {
            state.fill_line_default_colours(before_push - 1, fg, bg);
        }
        first_line = before_push - 1;
    }
    apply_turn_events(state, &result);
    flush_screen_trace(&state.config.user_dir, session, state.config.trace.screen);
    flush_v6_trace(&state.config.user_dir, session, state.config.trace.v6);
    // [more] pager (SQ-0404, ruleset reworked in SQ-0539): arm for this command's
    // output whenever the game is now awaiting the player — LINE *or* CHAR — and
    // the v6 "never print [MORE]" veto is off. The old `!result.erase_lower`
    // exclusion is gone: a clear preserves scrollback and re-anchors, so the rows
    // this turn added already measure the post-clear repaint alone (fits → no
    // pager; overflows → page it). The next render measures the rows added and
    // engages if it overflowed. See `crate::pager` for the full table.
    state.pager.arm_after_turn(
        state.last_transcript_total_rows,
        session.pending_input(),
        crate::pager::more_suppressed(&*session),
        crate::pager::Driver::PlayerInput,
    );
    let paging = Paging::after(first_line, session.pending_input(), &*session);
    if let Some(note) = &result.info {
        state.push_transcript(note);
    }

    // The story's own vocabulary, when the command held a word the story cannot
    // have understood (SQ-1041). HERE, after the game's reply is in the
    // transcript, so the offer reads underneath the refusal it answers rather
    // than above it — and only for a turn that printed something, because a turn
    // that printed nothing rejected nothing. Silence is the usual outcome; see
    // `crate::vocab` for the four gates it has to pass first.
    let printed = !silent
        && (!result.transcript.trim().is_empty() || !result.transcript_elems.is_empty());
    crate::vocab::offer_vocabulary(state, &*session, cmd, printed);

    // Capture room + connection counts before apply_turn, for `new_room`/`new_conn`
    // below (tidy scheduling — a non-mutating command like "look" leaves both
    // unchanged). `struct_gen_before` is `post_turn_bookkeeping`'s own before/after
    // comparison (SQ-1544).
    let rooms_before = mapper.graph.rooms().count();
    let conns_before = mapper.graph.connections().len();
    let struct_gen_before = mapper.graph.struct_gen();

    // SQ-0526: the Glulx side identifies rooms by hashing their printed NAME until
    // it has worked out where the game keeps its `location` global, then switches
    // to the room's real object address. On the turn that switch happens it hands
    // back the rooms mapped during the learning window; re-key them so they are
    // the same nodes afterwards instead of duplicates the player walks back into.
    // Empty on every other turn, and always empty for the Z-machine.
    if let Some(g) = crate::engine_helpers::glulx_session_opt_mut(&mut *session) {
        for (name, addr) in g.take_room_remap() {
            let old_id = crate::roomid::synthetic_room_id(&name);
            let new_id = crate::roomid::glulx_room_id(addr);
            mapper.rekey_room(old_id, new_id); // Mapper-level: also re-keys arrived_via (SQ-0632)
        }
    }

    // What `apply_turn` is about to record as tried, captured while the room typed in is still
    // the current one — the rollback below needs to know which record is this turn's (SQ-0671).
    let attempted = crate::session::tried_record_for(mapper, cmd);
    // …and the room they are LEAVING, which is only knowable here for the same reason and which
    // the return probe needs as the room a way back has to lead to (SQ-0785).
    let room_before = mapper.graph.current();

    // SQ-1257: what the room being LEFT declares for the direction just typed, read before
    // `apply_turn` decides what this move means. Needs a live engine handle and the pre-move
    // room, which is why this lives here and not inside `apply_turn` itself (an engine-neutral
    // pure function with neither). SQ-1314: `None` for a move made off the compass — see the
    // function's own docs.
    result.declared_exit =
        crate::random_exit_probe::declared_exit_for_command(cmd, room_before, |o, d| session.declared_exit(o, d));

    apply_turn(mapper, cmd, &result, &mut state.death_watch);

    // A move that killed the player proved nothing about the passage, so its `tried` record is
    // taken back and the direction stays untried (`·`, not `×`). Fires for the turn that
    // CONTAINED the fatal move even when the death is only admitted a turn later, after the
    // player answers a resurrection prompt. (SQ-0671)
    crate::session::rollback_tried_on_death(
        mapper,
        &mut state.death_watch,
        attempted,
        crate::session::turn_reports_death(&result.transcript),
    );

    // Breadcrumb for the maze view (SQ-0666): where the player has just been. Recorded from the
    // room the mapper settled on, after `apply_turn`, so a suppressed or relocated location does
    // not leave a step the map never took.
    if let Some(here) = mapper.graph.current() {
        state.push_trail(here);
    }

    // ONE host snapshot for everything this finished turn wants one for — the
    // return probe here, the history capture and the auto-save in
    // `post_turn_bookkeeping` below (SQ-1178). Valid for all three because
    // nothing from here to the next command mutates the VM: every call into
    // the session in between reads through `&dyn Engine`.
    let mut turn_save = crate::engine::TurnSave::default();

    // Look for the way back, in a silent copy of the game (SQ-0785). Off by default; arms only
    // for a crossing the map has no return path for, and ends any search a move has outrun.
    crate::return_probe::arm_return_search(state, mapper, &*session, cmd, room_before, &mut turn_save);

    // SQ-1257 Phase 2: this move's own edge was minted (or not) already, by `apply_turn` above.
    // Which reseeded shadow this turn earns — a first walk, an upgrade, or a suspicion left
    // pending — is `random_exit_probe`'s own decision, and lives there in ONE place: five real-
    // story harnesses mirror this call and every one of them used to restate the gate by hand
    // (SQ-1314). `room_before` is the only fact this scope has that it does not.
    crate::random_exit_probe::arm_for_finished_turn(
        state,
        &*session,
        mapper,
        cmd,
        room_before,
        result.declared_exit,
    );

    // The map render memo (`AppState::cached_map_render`) reads `Mapper::struct_gen`
    // straight off the live graph, and that counter bumps itself ONLY for a mutation
    // that changes the routed geometry (SQ-1540/SQ-1544) — never for a step between
    // already-placed rooms. No separate invalidation is needed here: bumping on every
    // step (rather than only a real geometry change) is exactly what SQ-0378 fixed,
    // and `struct_gen` already keeps that guarantee for free. The current-room
    // highlight and any in-place relabel are refreshed cheaply at draw time (see
    // `cached_map_render`), with no re-route.

    // Game-initiated (v4+) save/restore: open the saves dialog in
    // in-game mode and defer auto-save/history capture until the
    // resume completes (the turn is still in flight).
    if let Some(io) = result.pending_io {
        open_ingame_saves(io, game_dir, state);
        return TurnOutcome { quit: false, paging };
    }

    // Game create_by_prompt: open the filename modal and defer bookkeeping until the
    // resume completes (the turn is still in flight, like the save/restore path).
    if let Some(req) = session.pending_filename() {
        open_filename_modal(req, &*session, state);
        return TurnOutcome { quit: false, paging };
    }

    // Computed here, BEFORE bookkeeping, so a clean game-driven quit
    // (`should_exit_on_turn`) can suppress this turn's own per-turn auto-save
    // (SQ-1342): a save from the quit turn enqueued after the exit path's
    // clearing write would leave a resume point behind again. `state.game_ended`
    // is the flag the exit path (`main.rs` §6) reads to choose between
    // `lifecycle::exit_auto_save` and clearing the archive; it is set ONLY here
    // and in `finish_resumed_turn`, and cleared on restart/restore.
    let should_exit = should_exit_on_turn(&result, state);
    state.game_ended = should_exit;

    // ── Post-turn bookkeeping (history / inventory / auto-save) ──
    post_turn_bookkeeping(
        state, mapper, &mut *session, &result, cmd,
        struct_gen_before, ifid, arc_file, &mut turn_save,
    );
    persist_aux_after_turn(session, state, game_dir);
    persist_vfs_after_turn(session, state, game_dir);

    // SQ-1257 Phase 2: keep the engine as it stands RIGHT NOW for next turn's possible probe —
    // the moment just before whatever command produces the NEXT move is typed. Gated on
    // `rng_seed` answering `Some` (Z-machine today): `save_state` is sub-millisecond there, but
    // ~100 ms on Glulx (SQ-1177/SQ-1178), and `declared_exit` never overrides `Unknown` for any
    // engine besides the Z-machine anyway, so a probe can never fire for one — paying for a
    // snapshot it will never use would be exactly the cost this seam's own laziness elsewhere
    // exists to avoid.
    state.random_exit_pre_move_save = session
        .rng_seed()
        .map(|_| (mapper.graph.current().unwrap_or(0), turn_save.get(&*session)));

    // Background map maintenance: a geometry change (new room/connection) is the
    // ONLY thing that can require re-layout, so all of it runs on a worker thread —
    // NO routing or overlap cleanup ever touches the interpreter thread (SQ-0379).
    // A bare "look"/"inventory" changes no geometry, so it schedules nothing and
    // pays nothing here. The `background_tidy` setting decides whether the job is a
    // FULL relayout (aesthetics) or overlap-cleanup-only; cleanup runs either way,
    // so the map is never left overlapping regardless of the setting. Only runs in
    // Auto layout mode.
    //
    // Abort-and-replace on rapid movement: if a job is already in flight when the
    // next room arrives, we DROP its handle (the thread detaches and finishes into
    // the void — its result is discarded by the gen check) and spawn a fresh job for
    // the new state, so the latest geometry always wins immediately instead of
    // queueing behind a stale computation. Threads can't be force-killed, but only
    // the newest job is tracked/joined; concurrent stragglers are bounded by the
    // input rate. (This job is spawned once per command; the per-frame render worker
    // still coalesces — see `spawn_render_job` — to avoid a per-frame thread storm.)
    let new_room = mapper.graph.rooms().count() > rooms_before;
    let new_conn = mapper.graph.connections().len() > conns_before;
    schedule_map_maintenance(state, mapper, new_room, new_conn, bg_tidy_counter);

    // Clear any manual layer browse override so the view follows the player.
    state.set_viewed_layer(None);

    // Select and recenter on the current room.
    if let Some(snap) = &result.location {
        let rid = snap.number as mapper::graph::RoomId;
        state.select_room(Some(rid));
        recenter_on_room(state, mapper, rid, map_view);
    }

    // Scott Adams games auto-terminate via the VM's quit (opcode 63) on win or
    // loss. Rather than let a clean Scott quit exit the whole app, keep it alive
    // and raise the game-over dialog (the final message stays in the transcript
    // behind it). Every other engine keeps exiting on a clean quit.
    // (`should_exit` was computed above, before bookkeeping — see there.)
    let is_scott = crate::engine_helpers::engine_tag(session) == "scott";

    // SQ-0439: the map may have something to say about the move just made — that a set of rooms
    // wants a layer of its own. Deliberately at the END of the turn and never mid-one: the two
    // early returns above are the in-flight cases (the game is waiting on a save/restore or on a
    // filename), and `offer_layer_suggestion` stands down for a modal the player asked for.
    if !should_exit {
        crate::input::offer_layer_suggestion(state, mapper);
    }

    // If the debug inspector is open, refresh its snapshot from the VM state
    // this turn just produced (globals/objects/PC may have moved).
    if let Some(p) = &mut state.debug {
        if let Some(dbg) = session.debugger() {
            p.refresh(dbg);
        }
    }

    TurnOutcome { quit: intercept_scott_game_over(should_exit, is_scott, state), paging }
}

/// Fold a Scott clean quit into the game-over overlay. When the turn would exit
/// the app (`should_exit`) AND the engine is Scott, open the game-over dialog and
/// keep the app alive (return `false`). For every other case return `should_exit`
/// unchanged, so Z-machine/Glulx keep exiting on a clean `@quit`/`glk_exit`.
/// Schedule whatever background map work a graph change has earned (SQ-0379).
///
/// Extracted from the turn path so that a passage the RETURN PROBE discovered
/// (SQ-0785) reaches the same relayout the player's own moves do. An edge that
/// is recorded but never laid out or redrawn is a discovery nobody sees, which
/// is indistinguishable from not making it — and two copies of this block would
/// be two places for "does the map need re-laying out?" to be answered
/// differently.
///
/// A maze layer's geometry is frozen (SQ-0671): no job is scheduled for it at
/// all, rather than one spawned and thrown away. The layer keeps growing — a new
/// room was already dead-reckoned into place by `apply_turn` — but nothing
/// re-derives where the rooms already there sit. See `tidy::layer_is_frozen`.
pub fn schedule_map_maintenance(
    state: &mut AppState,
    mapper: &Mapper,
    new_room: bool,
    new_conn: bool,
    bg_tidy_counter: &mut u32,
) {
    let changed = new_room || new_conn;
    if !crate::tidy::should_schedule_tidy(&mapper.graph, state.active_layer(&mapper.graph), changed) {
        return;
    }
    // Nobody can see the map, so nothing here is worth a main-thread cost (SQ-1136).
    // What follows is not free just because the RELAYOUT runs on a worker: the
    // overlap scan walks the layer, the distortion scan walks every connection,
    // and then the whole graph is CLONED and a thread spawned — all on the thread
    // running the story, every turn that adds a room or an edge.
    //
    // Skipping is safe because a tidy is cosmetic. `place_incremental` has already
    // dead-reckoned each new room into position back in `Mapper::observe`, so a
    // hidden map is CORRECT throughout; it is merely untidy, and one relayout on
    // the way back settles however many turns went by. This is the same trade
    // SQ-0671 made for a frozen maze layer, which schedules no job at all rather
    // than spawning one to throw away.
    if state.layout != crate::state::Layout::Split {
        state.map_layout_deferred = true;
        return;
    }
    let active_layer = state.active_layer(&mapper.graph);
    // Overlap/distortion signal → decides FULL relayout vs. cleanup-only.
    let cells = mapper::layout::occupied_cells_in_layer(&mapper.graph, active_layer);
    let total_rooms = mapper.graph.rooms_in_layer(active_layer).len();
    let has_overlap = cells.len() < total_rooms;
    let has_distorted = mapper.graph.connections().iter().any(|c| {
        c.distorted
            && mapper.graph.layer_of(c.origin) == active_layer
            && mapper.graph.layer_of(c.dest) == active_layer
    });
    let overlap = has_overlap || has_distorted;
    let full =
        should_bg_tidy(state.config.background_tidy, new_room, overlap, changed, bg_tidy_counter);
    let kind = if full { TidyKind::Full } else { TidyKind::Cleanup };
    let graph_clone = mapper.graph.clone();
    let gen = mapper.graph.struct_gen();
    let handle = std::thread::spawn(move || {
        let mut g = graph_clone;
        match kind {
            TidyKind::Full => tidy_layer_silent(&mut g, active_layer),
            TidyKind::Cleanup => cleanup_overlaps_layer_silent(&mut g, active_layer),
        }
        g
    });
    state.tidy_job =
        Some(TidyJob { handle, layer: active_layer, gen, started: std::time::Instant::now(), kind });
}

/// Pay off the layout debt a hidden map ran up, once its pane is back (SQ-1136).
///
/// The counterpart to `turn::schedule_map_maintenance`'s early return. One job
/// settles any number of deferred turns, because a relayout derives every
/// position from the graph and not from the turns that built it — which is what
/// makes deferring sound in the first place.
///
/// Two conditions beyond the flag. The pane must be **visible**, or this would
/// undo the whole optimisation on the very tick that set the flag. And no job may
/// already be in flight: `schedule_map_maintenance` assigns over `state.tidy_job`,
/// which drops the running handle and detaches its thread, so a catch-up that
/// barged in would cost the very work it is trying to schedule. The debt keeps
/// until the next tick instead — it is a flag, and waiting costs nothing.
///
/// Returns true when a job was scheduled, so the caller can redraw.
pub fn catch_up_deferred_map_layout(
    state: &mut AppState,
    mapper: &Mapper,
    bg_tidy_counter: &mut u32,
) -> bool {
    if !state.map_layout_deferred
        || state.layout != crate::state::Layout::Split
        || state.tidy_job.is_some()
    {
        return false;
    }
    state.map_layout_deferred = false;
    // `new_room = true` so this asks for a FULL relayout rather than a cleanup:
    // the deferred stretch may have added many rooms, and dead reckoning is
    // exactly what a full pass exists to straighten out.
    schedule_map_maintenance(state, mapper, true, true, bg_tidy_counter);
    true
}

fn intercept_scott_game_over(should_exit: bool, is_scott: bool, state: &mut AppState) -> bool {
    if should_exit && is_scott {
        state.overlays.game_over = true;
        state.overlays.dialog_focus = 0;
        false
    } else {
        should_exit
    }
}

/// Post-turn bookkeeping shared by the normal `submit` path and the resumed
/// in-game save/restore path: opt-in rewind/replay capture, inventory tracking,
/// and per-turn auto-save. `struct_gen_before` is the mapper's
/// [`mapper::mapper::Mapper::struct_gen`] captured before this turn's
/// `apply_turn` (to detect a map change, SQ-1544 — was a rooms/conns count
/// comparison before it). `cmd` is the player's command (empty string for a
/// resumed in-game I/O turn).
fn post_turn_bookkeeping(
    state: &mut AppState,
    mapper: &Mapper,
    session: &mut dyn Engine,
    result: &TurnResult,
    cmd: &str,
    struct_gen_before: u64,
    ifid: &str,
    arc_file: &std::path::Path,
    turn_save: &mut crate::engine::TurnSave,
) {
    // A background archive write from an earlier turn can fail after this
    // turn has already moved on (SQ-1184) — surface it now rather than lose
    // it, on whichever later tick first calls back in here.
    for msg in state.archive_worker.drain_failures() {
        state.push_notice(&format!("[Auto-save failed: {}]", msg));
    }

    // ── Rewind/replay capture (opt-in) ────────────────────────────
    // Skip the quit turn: the VM has terminated, so its snapshot has
    // no replayable state — recording it just adds a junk final turn.
    if state.config.record_turn_history && !result.quit {
        let map_changed = mapper.graph.struct_gen() != struct_gen_before;
        // The record owns its bytes — it outlives the turn and is serialized
        // into the archive — so it copies them out of the shared turn snapshot
        // (SQ-1178): a memcpy, where a second `save_state` was the cost.
        crate::history::record_turn(
            &mut state.history,
            state.turns,
            cmd,
            turn_save.get(&*session).bytes.clone(),
            mapper,
            map_changed,
            &result.transcript,
        );
        // Bound retained turns (SQ-1185): `TurnRecord::save` is a full VM
        // snapshot, so left uncapped this grows without limit over an
        // arbitrarily long session.
        crate::history::cap_history(&mut state.history, state.config.history_turns);
    }

    // ── Inventory tracking ────────────────────────────────────────
    {
        use crate::inventory::{detect_player_obj, parse_inventory_output};

        let current_loc = session.current_location()
            .map(|s| s.number)
            .unwrap_or(0);

        if current_loc != 0 {
            // Objects whose parent is the current room, via the engine's
            // introspection (the same object-tree walk as before).
            let objects_here: std::collections::BTreeSet<u16> = session
                .introspect()
                .map(|i| i.children_of(current_loc))
                .unwrap_or_default();

            // Lock the player object. Prefer the reliable name-based
            // lookup (the object short-named "you"/"yourself"/… — present
            // in most games incl. v3 Zork as obj #30) so the inventory
            // panel reads the LIVE object tree from turn one and reflects
            // take/drop immediately. Fall back to the movement heuristic
            // for games whose player object isn't named.
            if state.player_obj.is_none() {
                state.player_obj = session.introspect().and_then(|i| i.player_object())
                    .or_else(|| detect_player_obj(
                        state.prev_location,
                        &state.prev_objects_here,
                        current_loc,
                        &objects_here,
                    ));
            }

            // Update tracking for next turn.
            state.prev_location = Some(current_loc);
            state.prev_objects_here = objects_here;
        }

        // If the submitted command was an inventory command, parse the output.
        let cmd_norm = cmd.trim().to_lowercase();
        if cmd_norm == "i" || cmd_norm == "inv" || cmd_norm == "inventory" {
            state.inventory_fallback = parse_inventory_output(&result.transcript);
        }
    }

    // ── The story's own words in what it has just printed (SQ-1116) ───
    // Here rather than in the key handler that reads it, because only the engine
    // can split prose the way the story's parser does and say which of the pieces
    // its dictionary holds — and the transcript this reads changes once a turn,
    // which is exactly this often.
    crate::input::refresh_seen_words(state, session);
    // …and the words for the things that are actually here, which change with
    // the room and are the ones a player cannot guess (SQ-1042).
    crate::input::refresh_scope_words(state, session);

    // Per-turn auto-save (when enabled). The build-and-write happens on a
    // background worker thread (SQ-1184): everything gathered here is either
    // an `Arc` clone (this turn's engine snapshot, every inline image, every
    // retained history turn) or a small owned copy, never the JSON-serialize
    // + Deflate + PNG-encode work that used to run on this thread every turn.
    // A write failure is non-fatal and is drained (and shown) at the top of
    // THIS function on a later turn, since it can only be known after this
    // call returns.
    // Engine-neutral: the save routes through Engine::save_state (Quetzal for
    // zvm, the gvm snapshot for Glulx); screen.bin is written for zvm only.
    // Skipped on the quit turn itself when the exit is game-driven (SQ-1342):
    // `state.game_ended` was just set above, and enqueuing a save here would
    // race the exit path's clearing write — a background write that lands
    // AFTER it would silently put the resume point right back.
    if state.config.auto_save && !state.game_ended {
        let (location, score) = crate::engine_helpers::save_summary(session, state);
        let meta = crate::archive::Meta {
            format_version: crate::archive::CURRENT_FORMAT_VERSION,
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
            trigger: crate::archive::SaveTrigger::HostState,
        };
        // v6 graphics canvases ride along so a resumed v6 story's pictures redraw
        // (SQ-0516); empty for non-v6 sessions, leaving the archive layout unchanged.
        // Must run here, on the main thread: it needs `&mut dyn Engine`.
        let (v6_pics, v6_display, v6_ground, v6_diags) = crate::engine_helpers::v6_save_payload(session);
        for d in &v6_diags { state.note_v6_save(d); }
        // The same turn snapshot history and the return probe read (SQ-1178):
        // the word refreshers and inventory tracking above read through
        // `&dyn Engine`, so the VM here is byte-identical to the VM there.
        let save = turn_save.get(&*session);
        let screen = zvm_session_opt(session).map(|z| z.machine.screen.clone());
        let job = crate::archive_worker::ArchiveJob {
            path: arc_file.to_path_buf(),
            mapper_graph: mapper.graph.clone(),
            save,
            screen,
            aux: session.aux_data().clone(),
            meta,
            session: crate::archive::SessionRecord::of(state).snapshot(),
            pictures: v6_pics,
            display: v6_display,
            ground: v6_ground,
        };
        state.archive_worker.enqueue(job);
    }
}

/// After a turn, persist the VM's aux table if it changed.  Archive mode is
/// already covered by the per-turn auto-save (`save_archive_meta` embeds it);
/// global mode writes the per-game file here.  `Ask` opens the first-use
/// prompt dialog (Task 6) and leaves `aux_dirty` set for the dialog to resolve.
pub fn persist_aux_after_turn(
    session: &mut dyn Engine,
    state: &mut AppState,
    game_dir: &std::path::Path,
) {
    if !session.aux_dirty() {
        return;
    }
    match state.config.aux_storage {
        crate::config::AuxStorage::Global => {
            let _ = crate::aux_store::write_global_aux(game_dir, session.aux_data());
            session.clear_aux_dirty();
        }
        crate::config::AuxStorage::Archive => {
            session.clear_aux_dirty(); // archive auto-save already embedded it
        }
        crate::config::AuxStorage::Ask => {
            state.overlays.aux_prompt = true; // resolve in the dialog; leave aux_dirty set
            state.overlays.dialog_focus = 0;
        }
    }
}

/// Flush the Glulx Glk file VFS to its per-story sidecar when it changed this
/// turn. Dirty-gated; a no-op for the Z-machine (whose `vfs_dirty` default is
/// always false). Mirrors `persist_aux_after_turn`.
pub fn persist_vfs_after_turn(
    session: &mut dyn Engine,
    state: &AppState,
    game_dir: &std::path::Path,
) {
    if !session.vfs_dirty() {
        return;
    }
    let bytes = session.vfs_bytes();
    let _ = crate::vfs_store::write_vfs(game_dir, &bytes);
    session.clear_vfs_dirty();
    crate::trace::hostio(&state.config.user_dir, state.config.trace.hostio,
        format!("vfs_write({} bytes)", bytes.len()));
}

/// Post-process a TurnResult produced by `session.resume_*`: render output,
/// re-observe the location, recenter, run post-turn bookkeeping, and record a
/// *chained* in-game I/O if the resume itself suspended on another
/// `@save`/`@restore`. Returns true if the app should quit. Mirrors the
/// post-turn block in the `submit` path.
pub fn finish_resumed_turn(
    result: TurnResult,
    mapper: &mut Mapper,
    state: &mut AppState,
    session: &mut dyn Engine,
    game_dir: &std::path::Path,
    ifid: &str,
    map_view: Option<(u16, u16)>,
) -> TurnOutcome {
    state.begin_turn(); // see `finish_command_turn` (SQ-1124)
    let first_line = state.transcript.len();
    state.push_transcript(&result.transcript);
    apply_turn_events(state, &result);
    if let Some(note) = &result.info {
        state.push_transcript(note);
    }
    // Captured before apply_turn so post_turn_bookkeeping can detect a map change.
    let struct_gen_before = mapper.graph.struct_gen();
    let room_before = mapper.graph.current();
    apply_turn(mapper, "", &result, &mut state.death_watch);
    // The resumed half of a turn can be where the death lands; it names no direction of its own,
    // so only the move still held from the submit path can be rolled back. (SQ-0671)
    crate::session::rollback_tried_on_death(
        mapper,
        &mut state.death_watch,
        None,
        crate::session::turn_reports_death(&result.transcript),
    );
    // Unconditional, mirroring the pre-SQ-1544 hand-bump this replaces: the resumed
    // half of a turn always forces a redraw, whether or not this apply_turn actually
    // changed the map (see `invalidate_map_render`'s own doc for why a plain
    // generation-number check is not used here).
    state.invalidate_map_render();
    // The resumed half of a turn can be a crossing too, and it is certainly a place a search
    // can be outrun (SQ-0785). It names no direction, so the fallback order applies.
    // The snapshot it takes is the one `post_turn_bookkeeping` below shares (SQ-1178).
    let mut turn_save = crate::engine::TurnSave::default();
    crate::return_probe::arm_return_search(state, mapper, session, "", room_before, &mut turn_save);
    state.set_viewed_layer(None);
    if let Some(snap) = &result.location {
        let rid = snap.number as mapper::graph::RoomId;
        state.select_room(Some(rid));
        recenter_on_room(state, mapper, rid, map_view);
    }
    // Captured before the partial move below (of `result.pending_io`) makes a
    // subsequent whole-struct borrow of `result` a borrow-checker error.
    let should_exit = should_exit_on_turn(&result, state);
    // See the matching comment in `finish_command_turn` (SQ-1342): set before
    // `post_turn_bookkeeping` below so a game-driven quit on the resumed half of
    // a turn also suppresses its own per-turn auto-save.
    state.game_ended = should_exit;
    // A chained request: the resumed turn suspended on another @save/@restore.
    // Mirror the submit path, which defers bookkeeping until the chain resolves;
    // run bookkeeping only when this turn finished without chaining.
    if let Some(io) = result.pending_io {
        state.ingame_io = Some(io);
    } else if let Some(req) = session.pending_filename() {
        // The resumed turn chained straight into a create_by_prompt.
        open_filename_modal(req, session, state);
    } else {
        let arc_file = default_state_path(game_dir);
        post_turn_bookkeeping(state, mapper, &mut *session, &result, "", struct_gen_before, ifid, &arc_file, &mut turn_save);
    }
    // A resumed turn arms no pager (it never did); the report still says where
    // its output began and whether a pager could have armed.
    TurnOutcome { quit: should_exit, paging: Paging::after(first_line, session.pending_input(), &*session) }
}

/// Apply a pending resume: restore the VM save, set transcript, re-observe location.
///
/// The same sequence the live restore performs — `engine_helpers`' restore path:
/// restore_quetzal, set transcript, `apply_turn` to re-observe the current room,
/// `set_viewed_layer(None)`, `select_room`, recenter.
///
/// This used to cite `Action::RestoreGame` (SQ-1065). That arm's own first line
/// says it is "dead post-unification: keys now route through `SlashOutcome::Load`",
/// and it has itself already drifted from the helper — so the citation pointed at
/// code no key reaches.
#[allow(clippy::too_many_arguments)]
pub fn apply_launch_resume(
    save: &crate::engine::EngineSave,
    lines: Vec<String>,
    kinds: Vec<TranscriptKind>,
    screen: Option<zvm::screen::ScreenState>,
    session: &mut dyn Engine,
    mapper: &mut Mapper,
    state: &mut AppState,
    map_view: Option<(u16, u16)>,
    arc_file: &std::path::Path,
) {
    match session.restore_state(save) {
        Ok(()) => {
            // Inline transcript images from the same archive the stashed lines came
            // from (parallel to `lines`); re-attached after the sidecar reset below
            // so a resumed transcript renders its embedded art (SQ-0518).
            let mut resumed_images: Vec<Option<crate::inline_image::InlineImage>> = Vec::new();
            // What the archive is missing because it predates the screen (SQ-1401)
            // or paint-log (SQ-1403) format bump — SQ-1410.
            let mut restore_degradation: Option<crate::archive::RestoreDegradation> = None;
            // The resumed game's map is part of its archive state — load it alongside.
            if let Ok(ac) = load_archive(arc_file) {
                // The v6 screen: rebuilt from the archived display list under the
                // archived palette when there is one (SQ-0588), else from canvas
                // PNGs. Ahead of the map move below, which consumes `ac` in part.
                // No-op for non-v6 archives and for Glulx (SQ-0516).
                crate::engine_helpers::apply_v6_pictures(&mut *session, &ac);
                *mapper = ac.mapper;
                // Restore the turn counter from the same archive the map came from.
                // The launch-resume stash omits it, so without this the count would
                // reset to 0 on resume (SQ-0260) — mirrors the interactive restore.
                state.turns = ac.meta.turns;
                // Hand Glulx back the room it was saved in (SQ-0523); no-op for zvm.
                crate::engine_helpers::seed_resumed_location(&mut *session, &ac.meta);
                resumed_images = ac.transcript_images;
                restore_degradation = Some(crate::archive::RestoreDegradation::from_format_version(
                    ac.meta.format_version,
                    crate::engine_helpers::is_v6_session(&*session),
                ));
            }
            // Reinstate the saved screen too (mirrors the auto-load path, zvm-only),
            // so a once-split game's upper window/status line shows after resuming.
            if let Some(scr) = screen {
                if let Some(z) = zvm_session_opt_mut(&mut *session) { crate::session::restore_screen(z, scr); }
            }
            state.transcript = lines;
            state.clear_anchor = None;
            state.transcript_kinds = kinds;
            // The launch-resume stash carries no style runs; keep the parallel
            // vecs length-synced (unstyled, left/no-indent rows).
            state.transcript_runs = vec![Vec::new(); state.transcript.len()];
            state.transcript_para = vec![crate::state::ParaFmt::default(); state.transcript.len()];
            state.reset_transcript_sidecars();
            // Re-attach inline images after the sidecar reset. Guard length: the
            // archive's images parallel ac.transcript, which equals the stashed
            // `lines` (same arc_file) — but a mismatch would desync the renderer.
            if resumed_images.len() == state.transcript.len() {
                state.transcript_images = resumed_images;
            }
            // The scraped word set is derived from the transcript and never
            // archived, so rebuild it from the resumed one (SQ-1135).
            crate::input::refresh_seen_words(state, &*session);
            // Re-observe current location (same as Action::RestoreGame).
            reobserve_location(state, mapper, &*session, map_view);
            state.push_notice("[Game resumed from save.]");
            // After `state.transcript = lines` above, not before: this line must
            // survive as the last one on screen (SQ-1410).
            if let Some(degradation) = restore_degradation {
                crate::engine_helpers::push_restore_degradation_notice(state, degradation);
            }
        }
        Err(e) => {
            state.push_notice(&format!("[Resume failed: {}]", restore_error_msg(e)));
        }
    }
}

// ── Game-driven input helpers (char-mode keypress, timed-input interrupt) ──────

/// Append a gvm runtime fault (diagnostics + fault trace) to `user_dir/crash.log`.
/// A fault ends the game via a silent `Quit`, so this makes the failure durable
/// regardless of terminal state. IO errors are ignored (best-effort logging).
fn log_gvm_fault(user_dir: &std::path::Path, fault: &[String], diagnostics: &[String]) {
    use std::io::Write as _;
    let Ok(mut f) =
        std::fs::OpenOptions::new().create(true).append(true).open(user_dir.join("crash.log"))
    else {
        return;
    };
    let _ = writeln!(f, "\n=== gvm runtime fault (game halted) ===");
    for d in diagnostics {
        let _ = writeln!(f, "diag: {d}");
    }
    for line in fault {
        let _ = writeln!(f, "{line}");
    }
}

/// Whether a turn result should terminate the app: only a CLEAN game exit
/// (glk_exit) does. A VM fault halts the game but keeps the app alive.
fn should_exit_on_turn(result: &TurnResult, state: &AppState) -> bool {
    result.quit && result.fault.is_none() && !state.vm_halted
}

/// Route a turn's sound/diagnostic events: diagnostics become Warning transcript
/// lines; the latest beep arms a one-shot story-border pulse; the current room
/// name is tracked for the built-in location story rule.
fn apply_turn_events(state: &mut AppState, result: &TurnResult) {
    for line in &result.diagnostics {
        state.push_transcript_kind(line, crate::state::TranscriptKind::Warning);
    }
    if let Some(lines) = &result.fault {
        let crash = state.colors.theme.get("transcript_crash").style;
        for line in lines {
            state.push_transcript_styled(line, crate::state::TranscriptKind::Warning, crash);
        }
        state.push_transcript_styled("(game halted)", crate::state::TranscriptKind::Warning, crash);
        // A gvm runtime fault ends the game via a silent Quit; if the app then
        // exits before this transcript is rendered, the error would vanish. Record
        // it durably so a "silent" crash always leaves a trace.
        log_gvm_fault(&state.config.user_dir, lines, &result.diagnostics);
        // Keep the app alive: a VM fault is not a clean glk_exit. The run loop's
        // exit checks all gate on `should_exit_on_turn`, which consults this flag.
        state.vm_halted = true;
        state.set_status("VM fault — the game has halted; you can review the map/transcript or quit.");
    }
    if let Some(kind) = result.sounds.iter().rev().find_map(|ev| match ev.number {
        1 => Some(crate::state::BeepKind::High),
        2 => Some(crate::state::BeepKind::Low),
        _ => None,
    }) {
        state.sound_pulse = Some(SoundPulse { kind, started: std::time::Instant::now() });
    }
    // Audio is additive on top of the border pulse; gated inside play_turn_sounds.
    state.play_turn_sounds(&result.sounds);
    // Glulx Glk sound channels (empty for the Z-machine path).
    state.play_glulx_sound_ops(&result.glulx_sound_ops);
    state.loc_method = result.location_method.or(state.loc_method);
    // Retain the previous name when this turn has no location signal.
    if let Some(loc) = &result.location {
        state.current_room_name = Some(loc.name.clone());
    }
}

/// Apply a `TurnResult` produced by game-driven input that is not a full player
/// command submission — a char-mode (`read_char`) keypress or a timed-input
/// interrupt tick. Pushes transcript output (with style runs), routes
/// beep/location/diagnostic events, applies the mapper turn, opens a
/// game-initiated save/restore dialog if requested, and recenters on a location
/// change. Deliberately skips `post_turn_bookkeeping` (history/inventory/
/// auto-save): this is not a completed player turn. Returns `true` if the game
/// quit (the caller should break the event loop).
pub fn apply_game_driven_result(
    state: &mut AppState,
    mapper: &mut Mapper,
    result: &TurnResult,
    game_dir: &std::path::Path,
    map_view: Option<(u16, u16)>,
    session: &dyn Engine,
    driver: crate::pager::Driver,
) -> TurnOutcome {
    state.begin_turn(); // see `finish_command_turn` (SQ-1124)
    if result.erase_lower {
        // A game-driven screen clear is a menu redraw navigated by keystrokes —
        // Counterfeit Monkey's help menu clears the primary buffer and reprints
        // the whole menu on EVERY arrow press. Collapse the previous reprint by
        // truncating back to the prior clear anchor, so the reprints replace each
        // other instead of piling up invisibly in scrollback (hidden by the
        // SQ-0403 view-pin, then revealed when you exit the menu). (SQ-0407)
        if let Some(anchor) = state.clear_anchor {
            state.truncate_transcript(anchor);
        }
        state.mark_screen_clear();
    }
    // Whether this turn's output CONTINUED the transcript's last pre-turn row
    // instead of opening one below it — the pager needs it (below).
    let mut continued_row = false;
    let pre_len = state.transcript.len();
    if result.transcript_elems.is_empty() {
        // Output the game printed where the cursor already was stays on the line it
        // was already on (SQ-0726, generalised in SQ-0804) — see
        // `push_transcript_runs_char_echo`. Not after `erase_lower`: the line before
        // the push belongs to a screen the game has just wiped, and the truncate
        // above may have taken it away entirely.
        if result.erase_lower {
            state.push_transcript_runs(&result.transcript, TranscriptKind::Story, &result.transcript_runs);
        } else {
            let continues = session.output_continued_line();
            continued_row = state.push_transcript_runs_char_echo(
                &result.transcript,
                TranscriptKind::Story,
                &result.transcript_runs,
                continues,
            );
        }
    } else {
        crate::state::apply_transcript_elems(state, &result.transcript_elems);
    }
    apply_turn_events(state, result);
    if let Some(note) = &result.info {
        state.push_transcript(note);
    }
    // [more] pager (SQ-0539): a game-driven turn pages exactly like a command
    // turn. A `read_char` that dumps more than a screenful — a hint page, a "press
    // any key" dump, a menu repaint that overflows — must show its FIRST screenful
    // and let the player page, and the paging keys are swallowed by the pager
    // rather than answering the pending read (see `crate::pager` and the char-input
    // gate in main.rs). `driver` keeps a timed-input interrupt / Glk timer /
    // sound-finish routine from reloading the baseline or dismissing an active
    // pager — a timeout is not a keystroke.
    state.pager.arm_after_turn(
        crate::pager::baseline_before(state.last_transcript_total_rows, continued_row),
        session.pending_input(),
        crate::pager::more_suppressed(session),
        driver,
    );
    let first_line = if continued_row { pre_len.saturating_sub(1) } else { pre_len };
    let paging = Paging::after(first_line, session.pending_input(), session);
    // apply_turn: this input doesn't carry direction info (no text command to
    // parse), but we still observe any location change so the map stays in sync.
    let room_before = mapper.graph.current();
    apply_turn(mapper, "", result, &mut state.death_watch);
    // A keypress can be the turn a death is finally admitted on ("press any key" after the
    // banner). It names no direction of its own, so the rollback can only be for the move the
    // player is still standing on the wrong side of. (SQ-0671)
    crate::session::rollback_tried_on_death(
        mapper,
        &mut state.death_watch,
        None,
        crate::session::turn_reports_death(&result.transcript),
    );
    // Game-initiated (v4+) save/restore: open the saves dialog in in-game mode
    // and defer the rest of the turn.
    if let Some(io) = result.pending_io {
        open_ingame_saves(io, game_dir, state);
        return TurnOutcome { quit: false, paging };
    }
    // Game `create_by_prompt`: open the filename modal and defer the rest, exactly
    // as `finish_command_turn` and `finish_resumed_turn` do. A Glulx game can ask
    // for a fileref from ANY turn — a transcript-on from a char-input menu
    // keypress, a timer, a mouse/hyperlink click, a sound-notify routine — and
    // without this the request had no resolver at all: the VM stayed suspended on
    // NeedFilename, every later drive re-reported it, and keypresses were
    // discarded against a machine that could never advance. (SQ-0657)
    if let Some(req) = session.pending_filename() {
        open_filename_modal(req, session, state);
        return TurnOutcome { quit: false, paging };
    }
    // The map render memo reads `Mapper::struct_gen` straight off the live graph
    // (SQ-1540/SQ-1544), which bumps itself ONLY when this game-driven turn actually
    // changed the routed geometry (a room/connection added) — never for a char-input
    // keypress (menu navigation, a "press any key" prompt), so it does not re-route
    // the whole map on the main thread every keystroke (the Counterfeit Monkey
    // help-menu pause). Mirrors the line-input path's gate in `finish_command_turn`
    // (SQ-0378), which this path used to spell by hand (SQ-0406). The current-room
    // highlight + recenter below still update cheaply without a re-route.
    // A game-driven turn can move the player too — a timer, a menu selection, a teleport — so it
    // both arms a search and ends one that has been outrun (SQ-0785). This path deliberately
    // skips `post_turn_bookkeeping`, so there is nobody to share the turn snapshot with.
    crate::return_probe::arm_return_search(
        state, mapper, session, "", room_before, &mut crate::engine::TurnSave::default(),
    );
    // Select and recenter on the current room if it changed.
    if let Some(snap) = &result.location {
        let rid = snap.number as mapper::graph::RoomId;
        state.select_room(Some(rid));
        recenter_on_room(state, mapper, rid, map_view);
    }

    // If the debug inspector is open, refresh its snapshot from the VM state
    // this turn just produced (globals/objects/PC may have moved).
    if let Some(p) = &mut state.debug {
        if let Some(dbg) = session.debugger() {
            p.refresh(dbg);
        }
    }

    TurnOutcome { quit: should_exit_on_turn(result, state), paging }
}

/// Decide the timed-input deadline for this loop iteration. `should_arm` is true
/// while the game is awaiting timed input (honoring timers, no overlay covering
/// the pane, and a timed read pending). Arm ONCE at `now + interval` and KEEP the
/// existing deadline while still armed — re-arming every iteration would push the
/// deadline perpetually ahead of `now`, so `now >= deadline` could never become
/// true and the interrupt would never fire. Disarm (`None`) when not applicable;
/// the run loop also clears the deadline to `None` right after firing, so the next
/// armed iteration re-arms fresh at `now + interval`.
pub fn next_input_deadline(
    current: Option<std::time::Instant>,
    should_arm: bool,
    interval: Duration,
    now: std::time::Instant,
) -> Option<std::time::Instant> {
    if should_arm {
        Some(current.unwrap_or(now + interval))
    } else {
        None
    }
}

#[cfg(all(test, feature = "t-session"))]
mod tests {
    use super::silent_terminator_turn;
    use crate::session::{TranscriptElem, TurnResult};

    fn blank_turn() -> TurnResult {
        TurnResult::default()
    }

    /// SQ-0881: the four states this rule has to tell apart.
    ///
    /// Arthur lists F1–F6 among its terminating characters, so pressing F2 for
    /// the map is a `read` that ends with no newline in it. Measured on
    /// `arthur-r74-s890714.z6`: at an ordinary line prompt,
    /// `submit_line_with_terminator("", 134)` returns an EMPTY transcript and a
    /// single empty text element — the game draws its map into a v6 window and
    /// prints nothing — so the transcript line the host adds per turn is a
    /// newline nothing echoed.
    #[test]
    fn only_a_turn_with_nothing_to_show_is_silenced() {
        // The reported case: a function key, nothing typed, nothing printed.
        assert!(silent_terminator_turn("", false, &blank_turn()));
        // …including when the engine reports it as one empty text element,
        // which is the shape Arthur actually produces.
        let mut empty_elem = blank_turn();
        empty_elem.transcript_elems =
            vec![TranscriptElem::Text { text: String::new(), runs: Vec::new() }];
        assert!(silent_terminator_turn("", false, &empty_elem));

        // A newline-terminated read is untouched, however empty — pressing
        // Enter on a blank line is a turn the player took and the game answered.
        assert!(!silent_terminator_turn("", true, &blank_turn()));

        // A terminator that ended a line the player TYPED still echoes it.
        assert!(!silent_terminator_turn("look", false, &blank_turn()));

        // …and a game that printed something still gets its line.
        let mut printed = blank_turn();
        printed.transcript = "The map unfolds.".to_string();
        assert!(!silent_terminator_turn("", false, &printed));
        let mut elem = blank_turn();
        elem.transcript_elems =
            vec![TranscriptElem::Text { text: "x".to_string(), runs: Vec::new() }];
        assert!(!silent_terminator_turn("", false, &elem));
        // A screen clear is output too: it moves the transcript's anchor.
        let mut cleared = blank_turn();
        cleared.transcript_elems = vec![TranscriptElem::ScreenClear];
        assert!(!silent_terminator_turn("", false, &cleared));
    }

    // ── SQ-1136: a hidden map does not pay for a layout nobody can see ─────────

    /// Two rooms and a passage — enough that `should_schedule_tidy` says yes.
    fn walked() -> mapper::mapper::Mapper {
        let mut m = mapper::mapper::Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "North of House", Some(mapper::direction::Direction::N));
        m
    }

    /// The visible case, which is the control: this is what the deferred case must
    /// NOT do. Without it a broken guard that never schedules anything would pass
    /// the test below and take the map's layout with it.
    #[test]
    fn a_visible_map_schedules_its_layout_as_it_always_did() {
        use crate::state::{AppState, Layout};
        let mut s = AppState::default();
        s.layout = Layout::Split;
        let m = walked();
        let mut counter = 0u32;
        super::schedule_map_maintenance(&mut s, &m, true, true, &mut counter);
        assert!(s.tidy_job.is_some(), "a visible map still gets its background tidy");
        assert!(!s.map_layout_deferred, "and owes nothing");
    }

    /// The optimisation itself: no clone, no thread, just a note that one is owed.
    #[test]
    fn a_hidden_map_defers_its_layout_instead_of_cloning_the_graph() {
        use crate::state::{AppState, Layout};
        let mut s = AppState::default();
        s.layout = Layout::TranscriptFull;
        let m = walked();
        let mut counter = 0u32;
        super::schedule_map_maintenance(&mut s, &m, true, true, &mut counter);
        assert!(s.tidy_job.is_none(), "nothing is spawned for a map nobody can see");
        assert!(s.map_layout_deferred, "but the debt is recorded");
    }

    /// …and the debt is paid once, on the way back. Many deferred turns settle with
    /// a single relayout, because a relayout reads the graph and not the history.
    #[test]
    fn showing_the_map_again_settles_the_whole_deferred_stretch_with_one_job() {
        use crate::state::{AppState, Layout};
        let mut s = AppState::default();
        s.layout = Layout::TranscriptFull;
        let mut m = walked();
        let mut counter = 0u32;
        for (id, name) in [(3u16, "Behind House"), (4, "Kitchen"), (5, "Attic")] {
            m.observe(id.into(), name, Some(mapper::direction::Direction::E));
            super::schedule_map_maintenance(&mut s, &m, true, true, &mut counter);
        }
        assert!(s.tidy_job.is_none(), "three turns hidden, three jobs not spawned");
        assert!(s.map_layout_deferred);

        // Still hidden: the catch-up must not fire, or the optimisation undoes
        // itself on the very tick that deferred the work.
        assert!(
            !super::catch_up_deferred_map_layout(&mut s, &m, &mut counter),
            "a hidden pane collects no debt"
        );
        assert!(s.map_layout_deferred, "and the debt survives to be paid later");

        s.layout = Layout::Split;
        assert!(super::catch_up_deferred_map_layout(&mut s, &m, &mut counter));
        assert!(s.tidy_job.is_some(), "one job settles the lot");
        assert!(!s.map_layout_deferred, "and the debt is cleared");
        assert!(
            !super::catch_up_deferred_map_layout(&mut s, &m, &mut counter),
            "it is a debt marker, not a queue: it does not fire twice"
        );
    }

    /// A catch-up never barges in on a running job. `schedule_map_maintenance`
    /// assigns over `state.tidy_job`, dropping the live handle and detaching its
    /// thread — so firing here would cost the very work it means to schedule.
    #[test]
    fn a_catch_up_waits_for_an_in_flight_tidy_rather_than_clobbering_it() {
        use crate::state::{AppState, Layout};
        let mut s = AppState::default();
        let m = walked();
        let mut counter = 0u32;

        s.layout = Layout::TranscriptFull;
        super::schedule_map_maintenance(&mut s, &m, true, true, &mut counter);
        assert!(s.map_layout_deferred);

        // A job from some other route is already running.
        s.layout = Layout::Split;
        super::schedule_map_maintenance(&mut s, &m, true, true, &mut counter);
        assert!(s.tidy_job.is_some());

        assert!(
            !super::catch_up_deferred_map_layout(&mut s, &m, &mut counter),
            "the catch-up stands down while a job is in flight"
        );
        assert!(s.map_layout_deferred, "and keeps the debt for the next tick");
    }

    // ── screen-trace flush (drain-always, write-when-on) ────────────────────────

    /// A minimal `Engine` double whose `take_screen_trace` returns one line on
    /// its first call and is empty thereafter, so a test can observe both the
    /// write-when-on path and the always-drain (no regrowth) behavior. `v6` lets
    /// a test stand in for a v6-vs-non-v6 engine on `v6_snapshot`.
    struct TraceOnlyEngine {
        line: Option<String>,
        v6: Option<Vec<String>>,
        /// A suspended `glk_fileref_create_by_prompt`, as a Glulx session reports
        /// one after a turn's drive stopped on `NeedFilename` (SQ-0657).
        filename_req: Option<crate::session::FilenameReq>,
    }

    impl crate::engine::Engine for TraceOnlyEngine {
        fn submit(&mut self, _command: &str) -> crate::session::TurnResult {
            unreachable!("not exercised by this test")
        }
        fn submit_key(&mut self, _key: crate::engine::KeyInput) -> Option<crate::session::TurnResult> {
            unreachable!("not exercised by this test")
        }
        fn take_transcript(&mut self) -> String {
            String::new()
        }

        // No screen-clear channel: this double is not a game.
        fn drain_screen_clear(&mut self) -> bool {
            false
        }

        fn pending_input(&self) -> crate::session::InputKind {
            // A game-driven turn ends on a keypress read (menu navigation, "press
            // any key"), which is what `apply_game_driven_result` asks about when
            // it arms the [more] pager (SQ-0539).
            crate::session::InputKind::Char
        }
        fn resume_save(&mut self, _wrote_ok: bool) -> crate::session::TurnResult {
            unreachable!("not exercised by this test")
        }
        fn resume_restore(&mut self, _data: Option<&[u8]>) -> crate::session::TurnResult {
            unreachable!("not exercised by this test")
        }
        fn pending_filename(&self) -> Option<crate::session::FilenameReq> {
            self.filename_req
        }
        fn has_quit(&self) -> bool {
            false
        }
        fn screen(&self) -> crate::engine::ScreenModel {
            unreachable!("not exercised by this test")
        }
        fn save_state(&self) -> crate::engine::EngineSave {
            unreachable!("not exercised by this test")
        }
        fn restore_state(&mut self, _save: &crate::engine::EngineSave) -> Result<(), crate::engine::EngineError> {
            unreachable!("not exercised by this test")
        }
        fn restore_game_save(&mut self, _bytes: &[u8]) -> Result<(), crate::engine::EngineError> {
            unreachable!("not exercised by this test")
        }
        fn take_screen_trace(&mut self) -> Vec<String> {
            self.line.take().into_iter().collect()
        }
        fn v6_snapshot(&self) -> Option<Vec<String>> {
            self.v6.clone()
        }
        fn aux_data(&self) -> &std::collections::BTreeMap<String, Vec<u8>> {
            unreachable!("not exercised by this test")
        }
        fn set_aux_data(&mut self, _data: std::collections::BTreeMap<String, Vec<u8>>) {
            unreachable!("not exercised by this test")
        }
        fn aux_dirty(&self) -> bool {
            false
        }
        fn clear_aux_dirty(&mut self) {}
        fn current_location(&self) -> Option<crate::engine::LocationInfo> {
            None
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    #[test]
    fn flush_screen_trace_writes_when_on_and_drains() {
        let dir = std::env::temp_dir().join(format!("bm-flush-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut eng = TraceOnlyEngine { line: Some("@split_window(1)".to_string()), v6: None, filename_req: None };

        super::flush_screen_trace(&dir, &mut eng, true);
        let body = std::fs::read_to_string(dir.join("trace.log")).unwrap_or_default();
        assert!(body.contains("[screen] "), "{body:?}");

        // Off → no further write, but the buffer must still be drained (no growth
        // while the section is toggled off between calls). Since take_screen_trace
        // is already empty after the first drain, a second flush is a no-op either
        // way; assert the log is unchanged.
        super::flush_screen_trace(&dir, &mut eng, false);
        let body2 = std::fs::read_to_string(dir.join("trace.log")).unwrap_or_default();
        assert_eq!(body, body2, "off flush must not append");

        std::fs::remove_dir_all(&dir).ok();
    }

    // ── v6-trace flush (skip-when-off, snapshot-only-for-v6-engines) ────────────

    #[test]
    fn flush_v6_trace_writes_when_on_and_skips_when_off() {
        let dir = std::env::temp_dir().join(format!("bm-flush-v6-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut eng = TraceOnlyEngine {
            line: None,
            v6: Some(vec!["turn snapshot (current=7)".to_string()]),
            filename_req: None,
        };

        // Off: nothing built, nothing written.
        super::flush_v6_trace(&dir, &mut eng, false);
        let body = std::fs::read_to_string(dir.join("trace.log")).unwrap_or_default();
        assert!(body.is_empty(), "off flush must not append: {body:?}");

        // On: the snapshot lines land, tagged.
        super::flush_v6_trace(&dir, &mut eng, true);
        let body2 = std::fs::read_to_string(dir.join("trace.log")).unwrap_or_default();
        assert!(
            body2.contains("[v6]") && body2.contains("turn snapshot (current=7)"),
            "{body2:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn flush_v6_trace_writes_nothing_for_an_engine_with_no_v6_model() {
        let dir = std::env::temp_dir().join(format!("bm-flush-v6-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut eng = TraceOnlyEngine { line: None, v6: None, filename_req: None };

        super::flush_v6_trace(&dir, &mut eng, true);
        let body = std::fs::read_to_string(dir.join("trace.log")).unwrap_or_default();
        assert!(body.is_empty(), "no v6 model → nothing written: {body:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    // ── Timed-input deadline arming (F1 regression) ─────────────────────────────

    #[test]
    fn timed_input_deadline_arms_once_and_does_not_recede() {
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let iv = Duration::from_millis(3000);

        // First armed iteration, no existing deadline: arm at t0 + interval.
        let d1 = super::next_input_deadline(None, true, iv, t0);
        assert_eq!(d1, Some(t0 + iv));

        // Later armed iterations MUST keep the original deadline, not push it
        // forward. This is the whole bug: re-arming to `now + interval` every
        // ~50ms iteration meant `now >= deadline` was never reached.
        let d2 = super::next_input_deadline(d1, true, iv, t0 + Duration::from_millis(50));
        assert_eq!(d2, d1, "armed deadline must not recede on later iterations");
        let d3 = super::next_input_deadline(d2, true, iv, t0 + Duration::from_millis(2999));
        assert_eq!(d3, d1, "still the original deadline right up until it elapses");

        // Not armed (overlay opened, timers off, or read ended): disarm.
        assert_eq!(super::next_input_deadline(d3, false, iv, t0 + Duration::from_millis(2999)), None);
        // Re-arm after a fire (deadline cleared to None): fresh at the new `now`.
        let t_fire = t0 + Duration::from_millis(3000);
        assert_eq!(super::next_input_deadline(None, true, iv, t_fire), Some(t_fire + iv));
    }

    #[test]
    fn glulx_glk_timer_arms_once_and_refires_each_interval() {
        use std::time::{Duration, Instant};
        // The Glulx Glk timer-events clock reuses `next_input_deadline`, so it has
        // the same arm-once/hold/re-arm-after-fire behavior as timed input. A 100ms
        // timer arms once and holds until it elapses, then re-arms fresh after the
        // fire path clears `glulx_timer_next_fire` to None.
        let t0 = Instant::now();
        let iv = Duration::from_millis(100);

        let d1 = super::next_input_deadline(None, true, iv, t0);
        assert_eq!(d1, Some(t0 + iv), "armed once at t0 + interval");
        let d2 = super::next_input_deadline(d1, true, iv, t0 + Duration::from_millis(30));
        assert_eq!(d2, d1, "holds steady across iterations until it fires");

        // Fire path sets glulx_timer_next_fire = None; next armed iteration re-arms
        // fresh at the fire instant + interval (periodic ticking).
        let t_fire = t0 + iv;
        assert_eq!(super::next_input_deadline(None, true, iv, t_fire), Some(t_fire + iv));

        // Timer canceled (interval None → should_arm false): disarm.
        assert_eq!(super::next_input_deadline(d2, false, iv, t0 + Duration::from_millis(30)), None);
    }

    // SQ-0260: the launch-dialog auto-resume must restore the saved turn counter.
    // The stash it works from carries no turn count, so apply_launch_resume reads
    // it from the archive (like the interactive restore) instead of leaving it 0.
    #[test]
    fn launch_resume_restores_the_turn_counter_sq0260() {
        use crate::engine::Engine;
        use crate::session::GameSession;

        // A Save State (.lanthorn) written with a non-zero turn count.
        let sess = GameSession::new(crate::engine_helpers::tests::read_char_then_save_v4_story(), true, false, None).expect("new");
        let save = sess.save_state();
        let arc = std::env::temp_dir().join(format!("bm-sq260-{}.lanthorn", std::process::id()));
        let meta = crate::archive::Meta {
            format_version: crate::archive::CURRENT_FORMAT_VERSION,
            ifid: None,
            name: None,
            turns: 42,
            saved_at: String::new(),
            location: None,
            score: None,
            trigger: crate::archive::SaveTrigger::HostState,
        };
        crate::archive::save_archive_meta(
            &arc, &mapper::mapper::Mapper::default(), &save, None,
            &std::collections::BTreeMap::new(), meta, &[], &[], &[], &[], &[], &[],
        ).expect("write .lanthorn with turns=42");

        // Fresh session + default state (turns start at 0), then launch-resume.
        let mut fresh = GameSession::new(crate::engine_helpers::tests::read_char_then_save_v4_story(), true, false, None).expect("new");
        let mut state = crate::state::AppState::default();
        let mut mapper = mapper::mapper::Mapper::default();
        assert_eq!(state.turns, 0, "a fresh AppState starts at turn 0");

        super::apply_launch_resume(
            &save, Vec::new(), Vec::new(), None,
            &mut fresh, &mut mapper, &mut state, None, &arc,
        );

        assert_eq!(state.turns, 42, "launch resume restores the saved turn count (SQ-0260)");
        let _ = std::fs::remove_file(&arc);
    }

    // SQ-0516: the launch-dialog auto-resume must repopulate a v6 story's graphics
    // canvases (`pictures_canvas`) from the archive — otherwise the restored game
    // shows NO pictures in any render mode (the screen() adapter emits Graphics
    // leaves only for windows present in pictures_canvas). Guards two coupled fixes:
    // the arc now carries pictures (save_archive_meta_pics, as the auto-save/exit
    // paths do) AND apply_launch_resume calls load_pictures_png on restore.
    // Skips cleanly when the gitignored Zork0 asset is absent.
    #[test]
    fn launch_resume_restores_v6_pictures_sq0516() {
        use crate::engine::Engine;
        use crate::graphics::PictSource;
        use crate::session::GameSession;

        let story_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/zork0-r393-s890714.z6");
        let Ok(story_bytes) = std::fs::read(&story_path) else {
            eprintln!("SKIP: gitignored Zork0 story missing at {}", story_path.display());
            return;
        };
        let boot = |bytes: Vec<u8>| -> GameSession {
            let mut picts =
                PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
            let dims = picts.all_pict_dims();
            let mut s = GameSession::new_with_trace(
                bytes, false, false, None, false, dims, picts.std_window(), None, None
            )
            .expect("Zork0 (v6) boots");
            s.set_pict_source(Some(picts));
            s.flush_boot_pictures();
            s
        };

        // Source session with live graphics; write the arc EXACTLY as the fixed
        // auto-save / exit paths now do (save_archive_meta_pics + pictures_png()).
        let src = boot(story_bytes.clone());
        let src_canvas_count = src.pictures_canvas.len();
        assert!(src_canvas_count > 0, "Zork0 boot draws graphics canvases");
        let save = src.save_state();
        let arc = std::env::temp_dir().join(format!("bm-sq516-{}.lanthorn", std::process::id()));
        let meta = crate::archive::Meta {
            format_version: crate::archive::CURRENT_FORMAT_VERSION,
            ifid: None,
            name: None,
            turns: 0,
            saved_at: String::new(),
            location: None,
            score: None,
            trigger: crate::archive::SaveTrigger::HostState,
        };
        crate::archive::save_archive_meta_pics(
            &arc, &mapper::mapper::Mapper::default(), &save, Some(&src.machine.screen),
            &src.machine.aux_data, meta, &crate::archive::SessionRecord::empty(), &src.pictures_png(), None, None,
        )
        .expect("write v6 .lanthorn with pictures");

        // Fresh v6 session with an EMPTY canvas, then drive the REAL launch-resume.
        let mut fresh = boot(story_bytes);
        fresh.pictures_canvas.clear();
        assert!(fresh.pictures_canvas.is_empty(), "canvas cleared before resume");

        let mut state = crate::state::AppState::default();
        let mut mapper = mapper::mapper::Mapper::default();

        super::apply_launch_resume(
            &save, Vec::new(), Vec::new(), Some(src.machine.screen.clone()),
            &mut fresh, &mut mapper, &mut state, None, &arc,
        );

        assert_eq!(
            fresh.pictures_canvas.len(), src_canvas_count,
            "launch resume must repopulate v6 graphics canvases from the archive (SQ-0516)"
        );
        assert!(
            !fresh.pictures_png().is_empty(),
            "restored v6 session re-encodes its graphics canvases"
        );
        let _ = std::fs::remove_file(&arc);
    }

    // ── gvm-fault survival (app must not silently exit on a VM runtime fault) ──

    fn fault_test_result(quit: bool, fault: Option<Vec<String>>) -> super::TurnResult {
        super::TurnResult {
            transcript: String::new(),
            transcript_runs: Vec::new(),
            location: None,
            quit,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops: Vec::new(),
            diagnostics: vec![],
            fault,
            location_method: None,
            pending_io: None,
            timed_out: false,
            pictures: Vec::new(),
            transcript_elems: Vec::new(),
            prose_retired: None,
            declared_exit: None,
        }
    }

    fn game_driven_result(location: Option<crate::engine::LocationInfo>) -> super::TurnResult {
        super::TurnResult {
            transcript: String::new(),
            transcript_runs: Vec::new(),
            location,
            quit: false,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops: Vec::new(),
            diagnostics: vec![],
            fault: None,
            location_method: None,
            pending_io: None,
            timed_out: false,
            pictures: Vec::new(),
            transcript_elems: Vec::new(),
            prose_retired: None,
            declared_exit: None,
        }
    }

    fn clearing_result(text: &str) -> super::TurnResult {
        let mut r = game_driven_result(None);
        r.erase_lower = true;
        r.transcript = text.to_string();
        r
    }

    #[test]
    fn game_driven_screen_clear_collapses_menu_reprints() {
        // SQ-0407: a menu navigated by keystrokes (CM's help) clears + reprints the
        // whole menu into the primary buffer every keypress. Consecutive game-driven
        // clears must COLLAPSE — each reprint replaces the last instead of piling up
        // in scrollback — while pre-menu content is preserved.
        let tmp = std::env::temp_dir().join(format!("lanthorn-collapse-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let mut state = crate::state::AppState::default();
        state.config.user_dir = tmp.clone();
        let mut m = mapper::mapper::Mapper::default();
        let rect = Some((20u16, 20u16));
        let eng = TraceOnlyEngine { line: None, v6: None, filename_req: None };

        state.push_transcript("room description"); // pre-menu content

        // First menu draw (a clearing turn): appends MENU v1.
        let _ = super::apply_game_driven_result(&mut state, &mut m, &clearing_result("MENU v1"), &tmp, rect, &eng, crate::pager::Driver::PlayerInput);
        assert!(state.transcript.iter().any(|l| l.contains("MENU v1")));
        let len_after_v1 = state.transcript.len();

        // Second draw (an arrow keypress): collapses v1, appends MENU v2.
        let _ = super::apply_game_driven_result(&mut state, &mut m, &clearing_result("MENU v2"), &tmp, rect, &eng, crate::pager::Driver::PlayerInput);
        assert!(!state.transcript.iter().any(|l| l.contains("MENU v1")), "v1 must be collapsed, not stacked");
        assert!(state.transcript.iter().any(|l| l.contains("MENU v2")), "v2 present");
        assert!(state.transcript.iter().any(|l| l.contains("room description")), "pre-menu content preserved");
        assert!(state.transcript.len() <= len_after_v1, "transcript did not grow across reprints");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn game_driven_char_turn_arms_the_more_pager_even_when_it_clears() {
        // SQ-0539, per the directive "[more] should work any time output is larger
        // than what fits on the screen (including boot) … we should behave as the
        // original game intended". The v1 (SQ-0404) rule armed only for a LINE
        // read on a turn that did NOT clear, so BOTH of this turn's properties —
        // game-driven keypress, screen clear — used to disqualify it. Now the
        // clear is irrelevant (it preserves scrollback and re-anchors, so the rows
        // this turn added measure the post-clear repaint alone) and a keypress
        // read arms like a command line; only `activation_target`, at render time,
        // decides fits-vs-overflows.
        let tmp = std::env::temp_dir().join(format!("lanthorn-pagerarm-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let mut state = crate::state::AppState::default();
        state.config.user_dir = tmp.clone();
        let mut m = mapper::mapper::Mapper::default();
        let rect = Some((20u16, 20u16));
        let eng = TraceOnlyEngine { line: None, v6: None, filename_req: None }; // pending_input() == Char
        state.last_transcript_total_rows = 12;

        let _ = super::apply_game_driven_result(
            &mut state, &mut m, &clearing_result("MENU"), &tmp, rect, &eng,
            crate::pager::Driver::PlayerInput,
        );
        assert_eq!(
            state.pager.pending_before_rows, Some(12),
            "a clearing char-input turn must arm with the pre-turn row total"
        );

        // A timeout-driven turn on top must NOT reload that baseline (mirrors the
        // engine's v6 line-count rule: keystrokes reload, timeouts don't).
        state.last_transcript_total_rows = 40;
        let _ = super::apply_game_driven_result(
            &mut state, &mut m, &clearing_result("TICK"), &tmp, rect, &eng,
            crate::pager::Driver::Timeout,
        );
        assert_eq!(state.pager.pending_before_rows, Some(12), "a timeout is not a keystroke");

        // And an already-showing pager is never re-parked mid-catch-up.
        state.pager.active = true;
        state.pager.disarm();
        let _ = super::apply_game_driven_result(
            &mut state, &mut m, &clearing_result("TICK 2"), &tmp, rect, &eng,
            crate::pager::Driver::Timeout,
        );
        assert!(state.pager.pending_before_rows.is_none(), "no re-arm while [more] is up");
        assert!(state.pager.active, "and a timeout never dismisses it");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn game_driven_turn_bumps_struct_gen_only_on_new_geometry() {
        // SQ-0406, reworked for SQ-1544: a char-input keypress (menu navigation /
        // "press any key") is a game-driven turn that changes no geometry — it must
        // NOT bump `Mapper::struct_gen`, which would re-route the whole map on the
        // main thread every keystroke (the Counterfeit Monkey help-menu pause). A
        // turn that reveals a NEW room still must bump so the map updates. This is
        // the map's own counter now (not a mirror `AppState` field), so it is read
        // straight off `m` — production code no longer bumps anything by hand here.
        let tmp = std::env::temp_dir().join(format!("lanthorn-gdr-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let mut state = crate::state::AppState::default();
        state.config.user_dir = tmp.clone();
        let mut m = mapper::mapper::Mapper::default();
        m.observe(1, "Lab", None); // a known, placed room
        let gen0 = m.struct_gen();
        let rect = Some((20u16, 20u16));
        let eng = TraceOnlyEngine { line: None, v6: None, filename_req: None };

        // Re-reporting the SAME room (a menu keystroke) must not re-route.
        let same = game_driven_result(Some(crate::engine::LocationInfo { number: 1, parent: 0, name: "Lab".into() }));
        let _ = super::apply_game_driven_result(&mut state, &mut m, &same, &tmp, rect, &eng, crate::pager::Driver::PlayerInput);
        assert_eq!(m.struct_gen(), gen0, "re-reporting a known room must not bump struct_gen");

        // Revealing a NEW room must bump (the map has to update).
        let moved = game_driven_result(Some(crate::engine::LocationInfo { number: 2, parent: 0, name: "Hall".into() }));
        let _ = super::apply_game_driven_result(&mut state, &mut m, &moved, &tmp, rect, &eng, crate::pager::Driver::PlayerInput);
        assert_ne!(m.struct_gen(), gen0, "a new room on a game-driven turn must bump struct_gen");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn game_driven_turn_opens_the_filename_modal_for_a_create_by_prompt() {
        // SQ-0657: a Glulx game can issue `glk_fileref_create_by_prompt` from ANY
        // turn — TRANSCRIPT ON chosen from a char-input menu, a timer routine, a
        // mouse/hyperlink click, a sound-notify routine. `finish_command_turn` and
        // `finish_resumed_turn` both resolve that request; the game-driven path did
        // not, so the VM stayed suspended on NeedFilename with NO modal open. Every
        // later drive re-reported it and every keypress was discarded against a
        // machine that could not advance — permanently wedged, with only a quit out.
        let tmp = std::env::temp_dir().join(format!("lanthorn-gdfn-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let mut state = crate::state::AppState::default();
        state.config.user_dir = tmp.clone();
        let mut m = mapper::mapper::Mapper::default();
        let rect = Some((20u16, 20u16));
        // fmode Write → a name-entry prompt (the TRANSCRIPT ON shape).
        let eng = TraceOnlyEngine {
            line: None,
            v6: None,
            filename_req: Some(crate::session::FilenameReq { usage: 0x02, fmode: 0x01 }),
        };

        let quit = super::apply_game_driven_result(
            &mut state, &mut m, &game_driven_result(None), &tmp, rect, &eng,
            crate::pager::Driver::PlayerInput,
        ).quit;

        assert!(!quit, "a filename request defers the turn, it does not end the app");
        assert_eq!(
            state.pending_filename,
            Some(crate::session::FilenameReq { usage: 0x02, fmode: 0x01 }),
            "the request must be recorded so the run loop's resolver can answer it",
        );
        assert!(
            matches!(
                &state.overlays.text_entry,
                Some(d) if d.kind == crate::state::TextEntryKind::CreateFile
            ),
            "a create-file prompt must be open — without it nothing can ever call resume_filename",
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn should_exit_on_turn_gates_on_clean_quit_only() {
        let mut state = crate::state::AppState::default();

        // Clean glk_exit: quit, no fault, not already halted → exit.
        let clean = fault_test_result(true, None);
        assert!(super::should_exit_on_turn(&clean, &state));

        // VM fault: quit, fault present → do not exit.
        let fault = fault_test_result(true, Some(vec!["boom".to_string()]));
        assert!(!super::should_exit_on_turn(&fault, &state));

        // Already halted from a prior fault: even a fault-free quit (the VM is a
        // no-op once halted) must not re-trigger an exit.
        state.vm_halted = true;
        let post_halt = fault_test_result(true, None);
        assert!(!super::should_exit_on_turn(&post_halt, &state));

        // Not a quit at all → never exit regardless of vm_halted.
        state.vm_halted = false;
        let not_quit = fault_test_result(false, None);
        assert!(!super::should_exit_on_turn(&not_quit, &state));
    }

    // ── SQ-1342: a clean, game-driven quit leaves no resume point ──────────────

    /// `finish_resumed_turn` must set `state.game_ended` on a clean quit — set
    /// exactly where `should_exit_on_turn` answers true — and that flag must, in
    /// the SAME call, suppress this turn's own per-turn auto-save (the write the
    /// exit path's clearing write must not race). `TraceOnlyEngine.save_state`
    /// is `unreachable!()`, so this test would PANIC if the gate ever let the
    /// auto-save reach it; not panicking, plus the archive file never appearing,
    /// is the proof it did not.
    #[test]
    fn a_clean_quit_sets_game_ended_and_skips_its_own_per_turn_auto_save_sq1342() {
        let dir = crate::scratch_dir("sq1342-quit-skips-autosave");
        let mut state = crate::state::AppState::default();
        state.config.auto_save = true;
        let mut mapper = mapper::mapper::Mapper::default();
        let mut eng = TraceOnlyEngine { line: None, v6: None, filename_req: None };
        let rect = Some((20u16, 20u16));
        let arc_file = dir.join("default.lanthorn");

        let quit_result = fault_test_result(true, None); // clean glk_exit
        let should_exit =
            super::finish_resumed_turn(quit_result, &mut mapper, &mut state, &mut eng, &dir, "TEST-IFID", rect).quit;

        assert!(should_exit, "a clean quit must still signal exit");
        assert!(state.game_ended, "should_exit_on_turn true must set game_ended (SQ-1342)");

        state.archive_worker.flush();
        assert!(
            !arc_file.exists(),
            "the quit turn's own per-turn auto-save must be skipped entirely, not merely emptied"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The guard the fix must NOT widen: an ordinary game-driven turn that is
    /// NOT a clean quit must leave `state.game_ended` false and its per-turn
    /// auto-save running exactly as before — mirrors the SQ-0648
    /// `per_turn_auto_save_never_prompts…` harness but drives the real
    /// `finish_resumed_turn` entry point so this turn's new
    /// `state.game_ended = should_exit` line is exercised on its FALSE branch too.
    #[test]
    fn a_non_quit_game_driven_turn_leaves_game_ended_false_and_still_auto_saves_sq1342() {
        use crate::session::GameSession;

        let dir = crate::scratch_dir("sq1342-nonquit-still-autosaves");
        let arc_file = dir.join("default.lanthorn");
        let mut sess = GameSession::new(crate::engine_helpers::tests::read_char_then_save_v4_story(), true, false, None).expect("new");
        let mut state = crate::state::AppState::default();
        state.config.auto_save = true;
        let mut mapper = mapper::mapper::Mapper::default();
        let rect = Some((20u16, 20u16));

        let not_quit = game_driven_result(None);
        let should_exit =
            super::finish_resumed_turn(not_quit, &mut mapper, &mut state, &mut sess, &dir, "TEST-IFID", rect).quit;

        assert!(!should_exit, "a non-quit turn must not exit");
        assert!(!state.game_ended, "a host-driven session must never see game_ended set (SQ-1342)");

        state.archive_worker.flush();
        let ac = crate::archive::load_archive(&arc_file).expect("per-turn auto-save must still write");
        assert!(!ac.save.is_empty(), "the per-turn auto-save must still be a real, resumable save");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Scott-only game-over interception ────────────────────────────────────
    #[test]
    fn scott_clean_quit_raises_game_over_and_stays_alive() {
        let mut state = crate::state::AppState::default();

        // A Scott engine on a quitting turn: raise the overlay, keep the app alive.
        let stay = super::intercept_scott_game_over(true, true, &mut state);
        assert!(!stay, "a Scott clean quit must NOT exit the app");
        assert!(state.overlays.game_over, "a Scott clean quit opens the game-over dialog");
        assert_eq!(state.overlays.dialog_focus, 0, "focus starts on the first button");

        // A non-Scott engine on a quitting turn: exit as before, no overlay.
        let mut state2 = crate::state::AppState::default();
        let exit = super::intercept_scott_game_over(true, false, &mut state2);
        assert!(exit, "a non-Scott clean quit still exits the app");
        assert!(!state2.overlays.game_over, "non-Scott never opens the game-over dialog");

        // A Scott engine on a non-quitting turn: no exit, no overlay.
        let mut state3 = crate::state::AppState::default();
        let exit3 = super::intercept_scott_game_over(false, true, &mut state3);
        assert!(!exit3, "a non-quitting turn never exits");
        assert!(!state3.overlays.game_over, "a non-quitting turn never opens the dialog");
    }

    #[test]
    fn apply_turn_events_halts_and_logs_on_fault() {
        // The other half of the `lanthorn-test-<pid>` collision — see
        // `persist_files::tests::save_then_load_round_trips` (SQ-1131).
        let tmp = crate::scratch_dir("turn-fault-log");
        let mut state = crate::state::AppState::default();
        state.config.user_dir = tmp.clone();

        let result = fault_test_result(true, Some(vec!["some fault line".to_string()]));
        super::apply_turn_events(&mut state, &result);

        assert!(state.vm_halted, "a fault must set vm_halted");
        assert!(state.notifications.latest_text().is_some(), "a fault must set a user-visible notification");

        let log = std::fs::read_to_string(tmp.join("crash.log")).expect("crash.log written");
        assert!(log.contains("gvm runtime fault"), "crash.log must record the fault header");
        assert!(log.contains("some fault line"), "crash.log must record the fault line");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── Per-turn auto-save never prompts (SQ-0648) ──────────────────────────────

    /// The overwrite-confirm prompt exists for save-as (a name the PLAYER typed).
    /// The per-turn auto-save writes the fixed `default.lanthorn` slot every turn
    /// regardless of what is already there, and must keep doing that silently —
    /// it calls `save_archive_meta_pics` directly, never `save_named` /
    /// `handle_save_as`, so there is no typed name to collide on. This guards
    /// that architecture: even seeding the slot with a DIFFERENT save first (the
    /// exact shape that triggers the prompt on the save-as path) must not open
    /// the overlay here.
    #[test]
    fn per_turn_auto_save_never_prompts_even_when_the_slot_already_has_a_different_save() {
        use crate::engine::Engine;
        use crate::session::GameSession;

        let dir = std::env::temp_dir().join(format!("bm-sq0648-autosave-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let arc_file = dir.join("default.lanthorn");

        // Pre-seed the slot as if from an earlier session.
        let seed_sess = GameSession::new(crate::engine_helpers::tests::read_char_then_save_v4_story(), true, false, None).expect("new");
        let seed_meta = crate::archive::Meta {
            format_version: crate::archive::CURRENT_FORMAT_VERSION,
            ifid: None, name: None, turns: 1, saved_at: String::new(), location: None, score: None,
            trigger: crate::archive::SaveTrigger::HostState,
        };
        crate::archive::save_archive_meta(
            &arc_file, &mapper::mapper::Mapper::default(), &seed_sess.save_state(), None,
            &std::collections::BTreeMap::new(), seed_meta, &[], &[], &[], &[], &[], &[],
        ).expect("seed default.lanthorn");
        let before = std::fs::read(&arc_file).unwrap();

        let mut sess = GameSession::new(crate::engine_helpers::tests::read_char_then_save_v4_story(), true, false, None).expect("new");
        let mut state = crate::state::AppState::default();
        state.config.auto_save = true;
        let mapper = mapper::mapper::Mapper::default();
        let result = game_driven_result(None);

        super::post_turn_bookkeeping(&mut state, &mapper, &mut sess, &result, "look", mapper.struct_gen(), "TEST-IFID", &arc_file, &mut crate::engine::TurnSave::default());
        // The write now happens on the background archive worker (SQ-1184);
        // flush before asserting on disk.
        state.archive_worker.flush();

        assert!(
            state.overlays.confirm_overwrite_save.is_none(),
            "the auto-save path must never open the overwrite-confirm overlay"
        );
        assert!(state.archive_worker.drain_failures().is_empty(), "the auto-save must succeed");
        let after = std::fs::read(&arc_file).unwrap();
        assert_ne!(after, before, "the auto-save actually wrote over the existing slot, silently");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
