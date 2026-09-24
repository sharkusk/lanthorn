//! Per-iteration housekeeping pollers for the event loop. These run at the top
//! of every loop pass, BEFORE the draw/poll, draining the independent pollable
//! subsystems (style-watch, Glulx re-arrange, background tidy/anim jobs, engine
//! input-mode + timer re-arm, sound-pulse + verb-dock settle). Extracted verbatim
//! from `main()`'s event loop (SQ-0306) as a pure move — no behavior change, same
//! order, same predicates. Each poller RETURNS its redraw contribution; the loop
//! OR-s it into its loop-local `needs_redraw` at the call site. Helper fns these
//! rely on stay in `main.rs` (referenced via `crate::`).

use std::time::Duration;

use mapper::mapper::Mapper;

use app::engine::Engine;
use app::glulx_session::GlulxSession;
use app::tidy::{apply_tidy_result, cleanup_overlaps_layer_silent, tidy_layer_silent, ApplyTidyOutcome};
use app::render::map::SOUND_PULSE_MS;
use app::state::{AppState, TidyJob, TidyKind, TidyAnim, TranscriptKind};

/// Style watch: drain events, debounce, then reload. Returns `true` if a reload
/// happened (colours/status changed → repaint).
pub(crate) fn poll_style_watch(
    state: &mut AppState,
    style_watcher: &Option<app::watch::StyleWatcher>,
    watch_dirty: &mut Option<std::time::Instant>,
) -> bool {
    let mut redraw = false;
    if let Some(w) = style_watcher {
        let mut saw = false;
        while w.rx.try_recv().is_ok() { saw = true; }
        if saw { *watch_dirty = Some(std::time::Instant::now()); }
    } else {
        // Watch turned off: drop any pending debounce so it can't fire later.
        *watch_dirty = None;
    }
    if app::watch::due(*watch_dirty, std::time::Instant::now(), Duration::from_millis(200)) {
        *watch_dirty = None;
        redraw = true; // style reload changes colours/status → repaint
        match app::reload::reload_style(state) {
            app::reload::ReloadOutcome::Reloaded { warnings } => {
                for wn in &warnings {
                    state.push_transcript_internal(wn, TranscriptKind::Warning);
                }
                state.set_status("style reloaded (watch)");
            }
            app::reload::ReloadOutcome::Failed { msg } => {
                state.push_transcript_internal(
                    &format!("style reload failed: {}", msg),
                    TranscriptKind::Warning,
                );
            }
        }
    }
    redraw
}

/// Keep the Glulx backend's theme colours in sync with the live ColorScheme
/// (SQ-0315): a style reload (watch, /reload-style, the per-game override) swaps
/// `state.colors`, and glk_style_measure must answer with what is now rendered.
/// Pushing the derived pairs every pass is cheap (four `Option<u32>` writes) and
/// needs no reload-site plumbing. Z-machine sessions are untouched (not Glk).
/// No redraw contribution.
pub(crate) fn sync_theme_colours(state: &AppState, session: &mut dyn Engine) {
    if let Some(gs) = session.as_any_mut().downcast_mut::<GlulxSession>() {
        gs.set_theme_colours(app::glk_backend::theme_style_colours(&state.colors));
    }
}

/// Glulx re-arrange on settled story-pane size (SQ-0201).
/// Uses last frame's story rect (one-frame lag is fine). Runs BEFORE the
/// draw so the resized graphics show on the next frame. Glulx-only; the
/// Z-machine renders its own fixed virtual screen into the pane.
pub(crate) fn poll_glulx_resize(
    session: &mut dyn Engine,
    last_panes: &crate::PaneRects,
    story_size_seen: &mut Option<(u16, u16)>,
    resize_dirty: &mut Option<std::time::Instant>,
    vm_story_size: &mut Option<(u16, u16)>,
) -> bool {
    let mut redraw = false;
    if session.as_any().is::<GlulxSession>() {
        let now = std::time::Instant::now();
        let cur = (last_panes.story.width, last_panes.story.height);
        if cur.0 > 0 && cur.1 > 0 {
            if Some(cur) != *story_size_seen {
                *story_size_seen = Some(cur);
                *resize_dirty = Some(now); // size moved; (re)start the settle timer
            }
            if Some(cur) != *vm_story_size
                && app::watch::due(*resize_dirty, now, Duration::from_millis(150))
            {
                *resize_dirty = None;
                *vm_story_size = Some(cur);
                redraw = true; // Glulx graphics repaint at the new size
                // The resize itself is the library's (SQ-1539); the settle timer
                // above is the TUI's, because only a drag needs one.
                app::host::screen::resize_glulx(session, cur);
            }
        }
    }
    redraw
}

/// SQ-1504: reset [`poll_glulx_resize`]'s three trackers to the same starting
/// point (`None`/`None`/`None`) a fresh launch begins from.
///
/// A restarted Glulx session (`reset::reset_game`) is rebuilt at the same
/// fallback width a launch's own constructor uses — `state.config.
/// virtual_screen_cols`/`rows` are unset by default in both paths. A launch
/// still ends up at the real pane width because `poll_glulx_resize` sees
/// `vm_story_size` at its initial `None`, treats the real pane as new, and
/// resizes once its settle timer elapses. `reset_game` has no access to
/// `main.rs`'s tracker locals and cannot touch them, so left alone they carry
/// whatever the OLD session last settled on — and if the terminal itself
/// hasn't moved since then, that already equals the (unchanged) pane, so the
/// poll reads the freshly rebuilt (narrower) session as already matching it
/// and never re-measures. The story panel then stays at the fallback width
/// until an actual terminal resize forces the comparison to differ — the
/// reported symptom. Called right after every `reset_game` (`main.rs`'s
/// `OverlayAct::ResetConfirm` and `OverlayAct::GameOverPlayAgain` arms) so the
/// very next `poll_glulx_resize` pass re-measures for real, exactly as a
/// launch's first pass does. A no-op for a Z-machine/Scott session — these
/// trackers are read only by the Glulx-gated code above — so it is safe to
/// call unconditionally after every reset.
pub(crate) fn reset_glulx_resize_trackers(
    vm_story_size: &mut Option<(u16, u16)>,
    story_size_seen: &mut Option<(u16, u16)>,
    resize_dirty: &mut Option<std::time::Instant>,
) {
    *vm_story_size = None;
    *story_size_seen = None;
    *resize_dirty = None;
}

/// Settle-and-requery the in-game graphics `Picker`'s cell size after a resize
/// (SQ-1511), replacing the ioctl-based `picker_ui::refresh_cell_size` this used
/// to call for `state.game_picker` specifically. SQ-1520 moved
/// `picker_ui::run_story_picker`'s own cover-art preview picker onto the same
/// settle-and-requery core (see [`requery_picker_if_settled`]) and retired
/// `refresh_cell_size` for good, once nothing called it any more.
///
/// **Why settle-and-requery instead of `TIOCGWINSZ`.** `refresh_cell_size`
/// re-derived the cell every resize with no round trip, by dividing the
/// window's reported pixel size by its cell count — cheap, but provably wrong
/// at some window sizes (the division isn't always exact), which is why
/// `ratatui-image`'s maintainer rejected a `set_font_size` call driven off it
/// upstream. The fix that shipped upstream instead (SQ-1519) is a poll-based
/// `Picker::from_query_stdio` — a real stdio round trip, safe to repeat
/// mid-session now that it no longer risks racing a blocking read against the
/// app's own input loop. A round trip is not free, so it is paid at most once
/// per settled resize BURST rather than once per `Event::Resize` — the same
/// shape as [`poll_glulx_resize`]'s settle timer, reused rather than
/// reinvented.
///
/// **Why every resize re-triggers it, not only a suspected font change.**
/// `Event::Resize(cols, rows)` is IDENTICAL whether the user dragged a window
/// corner or zoomed their font — crossterm hands back the same shape either
/// way, and there is no signal at that layer to tell them apart. The only
/// terminal-side signal that could — comparing against a derived pixel cell —
/// is exactly the disputed `TIOCGWINSZ` arithmetic this function exists to
/// stop trusting, so reaching for it here to decide WHETHER to requery would
/// just move the same rejected assumption one step earlier. Every settled
/// resize therefore requeries; see the commit message for the measured
/// real-world cost of that choice.
///
/// `dirty` is set by the caller (`main.rs`, on every `Event::Resize`) and
/// cleared here; `query` performs the actual requery and is a parameter
/// purely so a test can substitute a counting stub for the real stdio round
/// trip. Returns `true` (redraw needed) only when the requery both ran and
/// found a different cell.
///
/// A thin wrapper over [`requery_picker_if_settled`], which holds the actual
/// settle/requery/compare logic with no `AppState` dependency (SQ-1520
/// extraction) — so `picker_ui::run_story_picker`'s own cover-art preview loop
/// (its own local `Option<Picker>`, not `state.game_picker`) can drive the
/// same settle timer instead of the ioctl-based `picker_ui::refresh_cell_size`
/// it used to call.
pub(crate) fn poll_picker_requery(
    state: &mut AppState,
    dirty: &mut Option<std::time::Instant>,
    query: impl FnOnce() -> Option<ratatui_image::picker::Picker>,
) -> bool {
    let changed = requery_picker_if_settled(
        &mut state.game_picker,
        state.game_picker_query_answered,
        dirty,
        query,
    );
    if changed {
        state.graphics_render.borrow_mut().invalidate_cell_geometry();
    }
    changed
}

/// The settle-and-requery core [`poll_picker_requery`] wraps for `AppState`
/// (SQ-1520 extraction — see that fn's doc). Takes every fact it needs as a
/// parameter rather than reading `AppState`, so a second caller with its own
/// local `Option<Picker>` (`picker_ui::run_story_picker`'s cover-art preview)
/// can drive it too. `query_answered` is the caller's own
/// `game_picker_query_answered`-shaped bool (SQ-1511's guard: a launch-time
/// query that got no answer at all never will, so don't pay its timeout again
/// on every future resize). Returns `true` only when the requery both ran and
/// found a different cell — callers that keep a separate cell-geometry cache
/// invalidate it on `true`, same as [`poll_picker_requery`] does for
/// `state.graphics_render`.
pub(crate) fn requery_picker_if_settled(
    picker: &mut Option<ratatui_image::picker::Picker>,
    query_answered: bool,
    dirty: &mut Option<std::time::Instant>,
    query: impl FnOnce() -> Option<ratatui_image::picker::Picker>,
) -> bool {
    if !app::watch::due(*dirty, std::time::Instant::now(), Duration::from_millis(150)) {
        return false;
    }
    *dirty = None;

    if !query_answered {
        return false;
    }
    // `FontSize` has no `PartialEq` (it's a foreign type), so compare the
    // fields it exposes.
    let Some(was) = picker.as_ref().map(|p| (p.font_size().width, p.font_size().height)) else {
        return false;
    };
    let Some(new_picker) = query() else { return false };
    let now = (new_picker.font_size().width, new_picker.font_size().height);
    if now == was {
        return false; // same measurement; don't churn the picker for nothing
    }
    *picker = Some(new_picker);
    true
}

/// Report the story pane's REAL size to the Z-machine (ZMSD §8.4 — SQ-0532/A-F1),
/// from last frame's story rect (one-frame lag is fine). Runs BEFORE the draw so
/// the frame that follows already renders the upper window at the width the game
/// was just told about. The rule — which versions, the SQ-0679 width floor, the
/// compare against what the header says — is the library's
/// [`app::host::screen::sync_zvm_screen_dims`] (SQ-1539); this only measures.
/// Returns `true` when the header changed.
pub(crate) fn poll_zvm_screen_dims(
    session: &mut dyn Engine,
    state: &AppState,
    last_panes: &crate::PaneRects,
) -> bool {
    app::host::screen::sync_zvm_screen_dims(session, state, (last_panes.story.width, last_panes.story.height))
}

/// Keep header bytes $2C/$2D describing the colours the player actually sees
/// (ZMSD §8.3.3 — SQ-0532/A-F2).
///
/// The pair is resolved at startup and passed into the session before boot; this
/// poller exists for what happens AFTER: a `/reload-style`, a style-watch reload,
/// or a per-game theme switch changes the app's default page/ink, and
/// `reload_style` only sees `AppState` — it has no engine to tell. Comparing
/// against the machine's stored pair makes the poll a no-op on every pass but the
/// one right after a change (and re-arms it after an `@restart` rebuild).
///
/// `honor_game_colours = false` declares the interpreter colourless to the story
/// (§8.3.2), so the VM's own black-on-white seed is left alone. No redraw
/// contribution — this changes header bytes, not the screen.
pub(crate) fn poll_zvm_default_colours(session: &mut dyn Engine, state: &AppState) {
    if !state.config.honor_game_colours {
        return;
    }
    // SQ-0719: …and nothing to do when the interpreter profile pins the pair.
    // The Amiga profile reports the Amiga's default page and ink, which is the
    // whole point of claiming to be one; letting a style reload overwrite them
    // with the user's terminal colours would undo it on the next tick.
    //
    // SQ-0956: which is also what keeps a two-colour CARD's pair out of this
    // poller's reach, and it is worth saying rather than leaving to be noticed.
    // `startup.rs` decides that pair ONCE, from the archive, before the session
    // constructor runs the story — `PictSource::two_colour_card_screen`, whose only
    // other caller is the `@restart` rebuild — and a launch that reaches it is a
    // licensed one BY CONSTRUCTION, since the card's pair comes through
    // `machine_two_colour_colours` and that is gated on the same licence. So the
    // line below has already returned by the time any CGA launch gets here, and the
    // header keeps the card's black under white for the whole run. Anyone loosening
    // this guard has to give the card its own.
    if state.config.machine_default_colours().is_some() {
        return;
    }
    let Some(gs) = session.as_any().downcast_ref::<app::session::GameSession>() else {
        return;
    };
    let Some((bg, fg)) = app::colors::host_default_colour_pair(
        gs.machine.palette(),
        state.colors.theme.get("transcript").style,
        state.term_default_colors.fg.map(|c| (c.0[0], c.0[1], c.0[2])),
        state.term_default_colors.bg.map(|c| (c.0[0], c.0[1], c.0[2])),
    ) else {
        return;
    };
    if (gs.machine.default_bg_colour, gs.machine.default_fg_colour) == (bg, fg) {
        return;
    }
    session.set_default_colours(bg, fg);
}

/// Background tidy job + tidy-animation build job: poll and apply/install.
/// Runs BEFORE the draw so the first fully-drawn frame after completion shows the
/// new layout. Returns `true` if either job finished (map changed → repaint).
pub(crate) fn poll_tidy_jobs(
    state: &mut AppState,
    mapper: &mut Mapper,
    last_panes: &crate::PaneRects,
) -> bool {
    let mut redraw = false;

    // ── Background tidy job: poll and apply ───────────────────────────────
    // Check whether the in-flight tidy job has finished. Do this BEFORE the
    // draw so the first fully-drawn frame after completion shows the new layout.
    if state.tidy_job.as_ref().is_some_and(|j| j.handle.is_finished()) {
        redraw = true; // tidy result applied (or re-triggered) → map changes
        let job = state.tidy_job.take().unwrap();
        let current_gen = state.graph_gen;
        let active_layer = job.layer;
        match job.handle.join() {
            Ok(tidied) => {
                match apply_tidy_result(&mut mapper.graph, tidied, active_layer, job.gen, current_gen) {
                    ApplyTidyOutcome::Applied => {
                        state.bump_graph_gen(); // tidied layout applied → invalidate map memo (SQ-0305)
                        // Re-center on the current room if it moved.
                        if let Some(rid) = mapper.graph.current() {
                            if let Some(room) = mapper.graph.room(rid) {
                                if let Some(pos) = room.pos {
                                    let (pw, ph) = crate::map_pane_dims(last_panes.map);
                                    state.recenter_on(pos, pw, ph);
                                }
                            }
                        }
                    }
                    ApplyTidyOutcome::Stale => {
                        // Graph changed mid-tidy: re-trigger the SAME kind of job
                        // (full relayout vs. cleanup-only) for the current state.
                        // …but never onto a maze layer, whose geometry is frozen: re-triggering
                        // there is exactly the churn loop the freeze exists to end (SQ-0671).
                        let active_layer2 = state.active_layer(&mapper.graph);
                        if !app::tidy::layer_is_frozen(&mapper.graph, active_layer2) {
                            let kind = job.kind;
                            let graph_clone = mapper.graph.clone();
                            let gen2 = state.graph_gen;
                            let handle2 = std::thread::spawn(move || {
                                let mut g = graph_clone;
                                match kind {
                                    TidyKind::Full => tidy_layer_silent(&mut g, active_layer2),
                                    TidyKind::Cleanup => cleanup_overlaps_layer_silent(&mut g, active_layer2),
                                }
                                g
                            });
                            state.tidy_job = Some(TidyJob {
                                handle: handle2,
                                layer: active_layer2,
                                gen: gen2,
                                started: std::time::Instant::now(),
                                kind,
                            });
                        }
                    }
                }
            }
            Err(_) => {
                // Worker panicked: discard result, leave graph as-is. Do not crash.
            }
        }
    }

    // ── Tidy-animation build job: poll and install ────────────────────────
    // The `animate-tidy` command builds its frames off-thread. When the worker
    // finishes, apply the tidied graph (staleness-guarded) and install the anim.
    // Unlike the background tidy above, a stale result is simply discarded — the
    // user asked for one animation, so we do NOT re-trigger a fresh build.
    if state.anim_build_job.as_ref().is_some_and(|j| j.handle.is_finished()) {
        redraw = true; // anim build installed / graph applied → repaint
        let job = state.anim_build_job.take().unwrap();
        let current_gen = state.graph_gen;
        if let Ok((frames, tidied)) = job.handle.join() {
            match apply_tidy_result(&mut mapper.graph, tidied, job.layer, job.gen, current_gen) {
                ApplyTidyOutcome::Applied => {
                    // Instant re-tidy (animate=false) and the anim's final settle both
                    // land the tidied graph here — invalidate the map memo so the live
                    // path shows it (and does not SNAP BACK when the anim ends). (SQ-0305)
                    state.bump_graph_gen();
                    // `animate-tidy` plays the captured frames; the instant `tidy-map`
                    // re-tidy (animate=false) applies the tidied graph without an
                    // animation — it only used the off-thread build for the progress
                    // bar. (SQ-0261)
                    if job.animate {
                        state.tidy_anim = Some(TidyAnim::new(frames, job.layer));
                    }
                    // Re-center on the current room if it moved (mirrors the tidy_job path).
                    if let Some(rid) = mapper.graph.current() {
                        if let Some(room) = mapper.graph.room(rid) {
                            if let Some(pos) = room.pos {
                                let (pw, ph) = crate::map_pane_dims(last_panes.map);
                                state.recenter_on(pos, pw, ph);
                            }
                        }
                    }
                }
                ApplyTidyOutcome::Stale => {
                    // Graph changed during the build: discard the frames and the
                    // tidied result. Do not install an animation or apply a stale graph.
                }
            }
        }
    }

    redraw
}

/// Play out a v6 turn's picture sequence, one frame per hold (SQ-0708).
///
/// A v6 turn can queue several `draw_picture`s — Arthur's intro paints the
/// graveyard plate and then Merlin fourteen instructions later, in ONE turn — and
/// compositing them all before anything renders hands the player the finished
/// screen instantly. The real machines blitted each picture as its opcode ran, so
/// you watched the graveyard paint and then Merlin paint onto it. The session
/// snapshots the screens the turn passed through; this walks them.
///
/// The turn itself already ran to completion, so nothing here blocks the story
/// interpreter, and nothing sleeps: the deadline joins the loop's other clocks in
/// `next_deadline`, which is what keeps the poll waking in time and the keyboard
/// live all the way through. Returns `true` when a frame actually advanced.
pub(crate) fn poll_picture_pacing(state: &mut AppState, session: &mut dyn Engine) -> bool {
    let Some(gs) = app::engine_helpers::zvm_session_opt_mut(session) else {
        // Not a Z-machine engine: no sequence can be in flight, so make sure a
        // stale deadline from a previous session cannot linger.
        state.picture_pace_next = None;
        return false;
    };
    let Some(hold) = gs.paced_picture_hold() else {
        state.picture_pace_next = None;
        return false;
    };
    let now = std::time::Instant::now();
    match state.picture_pace_next {
        // First sight of this frame: start its clock. It is already on screen —
        // the turn that produced it forced a redraw — so nothing changes yet.
        None => {
            state.picture_pace_next = Some(now + hold);
            false
        }
        Some(due) if now >= due => {
            gs.advance_paced_pictures();
            // Arm the next frame's hold from THIS instant rather than from the
            // deadline just missed, so a slow frame cannot compound into a
            // sequence that races to catch up.
            state.picture_pace_next = gs.paced_picture_hold().map(|h| now + h);
            true
        }
        Some(_) => false,
    }
}

/// Collapse any in-flight picture sequence to its settled composite (SQ-0708) —
/// the player pressed a key, or the pane resized under it.
///
/// The player outranks paced output, the same rule the `[more]` pager runs on.
/// Unlike the pager this never CONSUMES the key: the sequence is decoration over
/// a turn that has already finished, so eating a keystroke to dismiss it would
/// swallow a character the player meant for the story. The two cannot deadlock
/// for the same reason — pacing blocks nothing and waits for nothing, so a key
/// settles the pictures and goes on to whatever else wanted it, `[more]` included.
///
/// Returns `true` when frames were dropped (the screen jumps to the final state).
pub(crate) fn settle_picture_pacing(state: &mut AppState, session: &mut dyn Engine) -> bool {
    state.picture_pace_next = None;
    app::engine_helpers::zvm_session_opt_mut(session)
        .is_some_and(|gs| gs.settle_paced_pictures())
}

/// Refill the command band from the engine, once per loop tick: its object
/// columns whenever the VM has run since the last fill (`turn_epoch`-gated,
/// SQ-1175 — objects cannot move while the VM is parked at a read), and its
/// VERB column once per open, from the story's own grammar (SQ-1111).
///
/// Thin wrapper over `app::render::command_band`'s two refreshers, which live in
/// the lib so the integration tests can drive them against a real story. Both
/// run — `||` would short-circuit the objects refresh on the one tick the verbs
/// change.
pub(crate) fn refresh_command_band(state: &mut AppState, session: &dyn Engine) -> bool {
    let verbs = app::render::command_band::refresh_verbs(state, session);
    app::render::command_band::refresh_objects(state, session) || verbs
}

/// Expire a finished sound pulse and settle the command band's slide-out.
/// Returns `true` if the border reset or the drawer content dropped
/// (→ repaint once).
pub(crate) fn expire_sound_and_settle_dock(state: &mut AppState) -> bool {
    let mut redraw = false;

    // Expire a finished sound pulse so the story border returns to normal.
    if let Some(p) = &state.sound_pulse {
        if p.started.elapsed().as_millis() as u64 >= SOUND_PULSE_MS {
            state.sound_pulse = None;
            redraw = true; // border returns to normal → repaint once
        }
    }

    // Reap expired notification toasts (they slide out, then vanish). (SQ-0176)
    if state.notifications.expire() {
        redraw = true;
    }

    // Put out a word reveal whose hold is up (SQ-1107). Here rather than at the
    // draw, because the hold is a WALL CLOCK: a player who presses the key and
    // then does nothing at all must still watch it go out, and nothing else in an
    // otherwise idle loop would notice the deadline pass.
    if app::reveal::expire(state) {
        redraw = true;
    }

    // Clear the command band's content once its slide-out has fully settled
    // (drawer pattern: content persists during the close animation).
    let had_band = state.overlays.command_band.is_some();
    state.settle_command_band();
    if had_band && state.overlays.command_band.is_none() {
        redraw = true; // drawer content dropped → repaint the cleared pane
    }

    redraw
}

/// Collect one answer from the shared shadow and give it to whoever asked for it,
/// then let the return search hand out its next question (SQ-0785).
///
/// **One collector, because there is one channel.** [`app::probe::ShadowProbe`]
/// serves two consumers — the vocabulary offer and the return search — and
/// [`app::probe::ShadowProbe::poll`] takes whatever has arrived without knowing
/// who wanted it. Two consumers each polling for themselves would mean the first
/// one to look takes the other's answer off the channel and drops it, silently
/// and only sometimes. So the answer is collected here, once, and routed by the
/// token it carries.
///
/// The pump runs after the route, so a search whose attempt has just come back
/// asks its next question on the same pass rather than idling a frame per
/// direction.
pub(crate) fn poll_shadow_answers(
    state: &mut AppState,
    mapper: &mut Mapper,
    bg_tidy_counter: &mut u32,
) -> bool {
    let mut changed = false;
    if let Some(answer) = state.probe.poll() {
        if app::vocab::owns(state, answer.token) {
            changed |= app::vocab::deliver_answer(state, answer);
        } else if app::return_probe::owns(state, answer.token)
            && app::return_probe::deliver(state, mapper, &answer).is_some()
        {
            // A new passage is a geometry change, so it gets everything a walked
            // one gets: the render memo invalidated, the layout rescheduled, and
            // a redraw. An edge nobody lays out or draws is a discovery the
            // player never sees.
            state.graph_gen = state.graph_gen.wrapping_add(1);
            app::host::turn::schedule_map_maintenance(state, mapper, false, true, bg_tidy_counter);
            changed = true;
        } else if app::random_exit_probe::owns(state, answer.token)
            && app::random_exit_probe::deliver(state, mapper, &answer)
        {
            // SQ-1257 Phase 2: an edge was just DELETED (a random exit confirmed), which is a
            // geometry change exactly like a new one — the render memo and any in-flight tidy
            // must not go on describing the edge that is now gone.
            state.graph_gen = state.graph_gen.wrapping_add(1);
            app::host::turn::schedule_map_maintenance(state, mapper, false, true, bg_tidy_counter);
            changed = true;
        }
        // An answer nobody owns is one whose asker has moved on — an aborted
        // search, or a vocabulary offer the player typed past. Dropping it is the
        // silence discipline both consumers already have.
    }
    app::return_probe::pump_return_search(state);
    changed
}

#[cfg(all(test, feature = "t-misc"))]
mod poll_picker_requery_tests {
    use super::*;
    use std::cell::Cell;
    use std::time::Instant;

    use ratatui_image::picker::Picker;
    use ratatui_image::FontSize;

    /// A picker whose launch query got an answer, at `font`. Real `Picker`s
    /// with non-empty `capabilities()` can only come from a real stdio round
    /// trip (its fields are private outside `ratatui-image`), which is exactly
    /// why `game_picker_query_answered` is a plain `AppState` bool rather than
    /// something derived from the picker itself — see that field's doc.
    // `Picker::set_font_size` was a fork-only addition (SQ-0992); upstream has
    // no setter for an existing `Picker`, only the deprecated `from_fontsize`
    // constructor — fine here, since these tests only need a `Picker` at a
    // chosen font size, not a live capability-queried one (SQ-1510).
    #[allow(deprecated)]
    fn answered_state(font: (u16, u16)) -> AppState {
        let mut state = AppState::default();
        let p = Picker::from_fontsize(FontSize::new(font.0, font.1));
        state.game_picker = Some(p);
        state.game_picker_query_answered = true;
        state
    }

    fn font_of(state: &AppState) -> (u16, u16) {
        let f = state.game_picker.as_ref().expect("picker still present").font_size();
        (f.width, f.height)
    }

    /// (1) A resize that just arrived has not settled — the very next poll
    /// pass must not query at all, not even to find "no change".
    #[test]
    fn an_unsettled_resize_never_queries() {
        let mut state = answered_state((10, 20));
        let calls = Cell::new(0u32);
        let mut dirty = Some(Instant::now()); // a Resize "just" arrived
        let redraw = poll_picker_requery(&mut state, &mut dirty, || {
            calls.set(calls.get() + 1);
            Some(Picker::halfblocks())
        });
        assert!(!redraw, "the settle window has not elapsed yet");
        assert_eq!(calls.get(), 0, "no requery before a burst settles");
        assert!(dirty.is_some(), "still pending — not consumed early");
    }

    /// (1) An ordinary resize with no font change: once settled, the requery
    /// runs (there is no cheaper signal that distinguishes a font change from
    /// a plain resize — see the fn's own docs) but finds the same measurement
    /// and leaves the picker alone.
    #[test]
    fn a_settled_resize_with_no_font_change_does_not_swap() {
        let mut state = answered_state((10, 20));
        let calls = Cell::new(0u32);
        let mut dirty = Some(Instant::now() - std::time::Duration::from_millis(200));
        let redraw = poll_picker_requery(&mut state, &mut dirty, || {
            calls.set(calls.get() + 1);
            Some(Picker::halfblocks()) // same (10, 20) as `answered_state`
        });
        assert!(!redraw, "same measurement — nothing to redraw for");
        assert_eq!(calls.get(), 1, "the settled requery still runs once");
        assert_eq!((10, 20), font_of(&state), "unchanged measurement leaves the picker alone");
        assert!(dirty.is_none(), "consumed once due() fires");
    }

    /// (2) A font change: once settled, the requery's result replaces
    /// `game_picker` and asks for a redraw. FALSIFY by hard-coding this fn to
    /// always `return false` after the swap and watch `font_of` stay stale.
    #[test]
    #[allow(deprecated)]
    fn a_settled_font_change_swaps_the_picker_and_redraws() {
        let mut state = answered_state((10, 20));
        let mut dirty = Some(Instant::now() - std::time::Duration::from_millis(200));
        let redraw = poll_picker_requery(&mut state, &mut dirty, || {
            Some(Picker::from_fontsize(FontSize::new(7, 15)))
        });
        assert!(redraw, "a font-size change must ask for a redraw");
        assert_eq!((7, 15), font_of(&state));
        assert!(dirty.is_none());
    }

    /// (3) A resize BURST — several `Resize` events arriving faster than the
    /// settle window — must cost at most one requery, not one per event.
    /// Driven the way `main.rs` drives it: every event just re-marks `dirty`
    /// to "now", so a burst keeps re-arming the timer and never lets it fire
    /// until the events stop. FALSIFY by removing the `due()` gate (always
    /// query) and watch `calls` climb past 1 during the burst loop below.
    #[test]
    #[allow(deprecated)]
    fn a_resize_burst_queries_at_most_once() {
        let mut state = answered_state((10, 20));
        let calls = Cell::new(0u32);
        let mut dirty: Option<Instant>;

        // Four "Resize" events in a row, each re-arming the settle timer
        // before it can fire — exactly a drag delivering a burst.
        for _ in 0..4 {
            dirty = Some(Instant::now());
            let redraw = poll_picker_requery(&mut state, &mut dirty, || {
                calls.set(calls.get() + 1);
                Some(Picker::halfblocks())
            });
            assert!(!redraw, "still inside the burst — never settled");
        }
        assert_eq!(calls.get(), 0, "no requery fired during the burst itself");

        // The burst stops; the settle window has now genuinely elapsed.
        dirty = Some(Instant::now() - std::time::Duration::from_millis(200));
        let redraw = poll_picker_requery(&mut state, &mut dirty, || {
            calls.set(calls.get() + 1);
            Some(Picker::from_fontsize(FontSize::new(7, 15)))
        });
        assert!(redraw);
        assert_eq!(calls.get(), 1, "settling a burst costs exactly one requery");

        // A further idle poll pass (no new Resize) must not re-fire.
        let redraw2 = poll_picker_requery(&mut state, &mut dirty, || {
            calls.set(calls.get() + 1);
            Some(Picker::halfblocks())
        });
        assert!(!redraw2);
        assert_eq!(calls.get(), 1, "consumed — an idle pass after settling must not re-query");
    }

    /// The guard: a picker whose launch query never got an answer at all must
    /// never pay a requery's stdio round trip on a resize, however long the
    /// settle window has elapsed — this is what keeps a terminal that never
    /// answers DSR from paying that timeout on every resize.
    #[test]
    fn an_unanswered_launch_query_never_requeries() {
        let mut state = AppState::default();
        state.game_picker = Some(Picker::halfblocks());
        state.game_picker_query_answered = false; // e.g. --image-protocol halfblocks, or a silent tty
        let calls = Cell::new(0u32);
        let mut dirty = Some(Instant::now() - std::time::Duration::from_millis(200));
        let redraw = poll_picker_requery(&mut state, &mut dirty, || {
            calls.set(calls.get() + 1);
            Some(Picker::halfblocks())
        });
        assert!(!redraw);
        assert_eq!(calls.get(), 0, "never pay the query's timeout for a terminal that answers nothing");
        assert!(dirty.is_none(), "still consumed — no point re-arming for the same terminal");
    }

    /// SQ-1520: the core the AppState-shaped tests above exercise through
    /// [`poll_picker_requery`] must also work driven directly, with no
    /// `AppState` in sight — exactly how `picker_ui::run_story_picker`'s own
    /// local cover-art preview picker drives it. FALSIFY by hard-coding
    /// `requery_picker_if_settled` to always `return false` right after
    /// building `new_picker` (skipping the `*picker = Some(new_picker)`
    /// assignment) and watch `cover_picker`'s font stay at its stale (10, 20)
    /// below.
    #[test]
    #[allow(deprecated)]
    fn requery_picker_if_settled_drives_a_bare_option_with_no_appstate() {
        let mut cover_picker = Some(Picker::from_fontsize(FontSize::new(10, 20)));
        let calls = Cell::new(0u32);
        let mut dirty = Some(Instant::now() - std::time::Duration::from_millis(200));

        let changed = requery_picker_if_settled(&mut cover_picker, true, &mut dirty, || {
            calls.set(calls.get() + 1);
            Some(Picker::from_fontsize(FontSize::new(7, 15)))
        });

        assert!(changed, "a settled requery finding a different cell reports true");
        assert_eq!(calls.get(), 1);
        let f = cover_picker.as_ref().expect("picker still present").font_size();
        assert_eq!((7, 15), (f.width, f.height));
        assert!(dirty.is_none(), "consumed once due() fires");
    }
}
