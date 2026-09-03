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
                if let Some(gs) = session.as_any_mut().downcast_mut::<GlulxSession>() {
                    gs.resize(cur.0 as u32, cur.1 as u32);
                }
            }
        }
    }
    redraw
}

/// Report the story pane's REAL size to the Z-machine (ZMSD §8.4 — SQ-0532/A-F1).
///
/// §8.4: the interpreter "may change the exact dimensions whenever it likes but
/// must write the current height (in lines) and width (in characters) into bytes
/// $20 and $21 in the header." Uses last frame's story rect (one-frame lag is
/// fine) and runs BEFORE the draw so the frame that follows already renders the
/// upper window at the width the game was just told about.
///
/// Unlike [`poll_glulx_resize`] there is no settle timer: this writes two header
/// bytes rather than running the game's re-layout code, so it is free to track
/// every intermediate size during a drag. It also carries no cached "last
/// applied" size — it compares against what the header currently SAYS, so a fresh
/// boot after `@restart` (whose new `Machine` re-seeds the fallback) is corrected
/// on the next pass without the restart path having to know about any of this.
///
/// Skipped for v6, whose fixed 640×400 pixel screen is scaled into the pane
/// rather than measured from it, and for v1–3, which have no such header fields.
/// Returns `true` when the header changed (the upper window's width follows it).
///
/// What is declared is [`declared_story_screen_dims`], not the raw pane
/// measurement: the width is floored at the columns the story booted with
/// (SQ-0679) — whatever `GameSession::boot_screen_cols` says THIS session
/// actually booted at, 80 by default or a narrower/wider pre-boot-seeded pane
/// (SQ-0680) — so narrowing the pane can never move a v4/v5 status routine's
/// baked-in field columns outside the window.
///
/// [`declared_story_screen_dims`]: app::render::screen::declared_story_screen_dims
pub(crate) fn poll_zvm_screen_dims(
    session: &mut dyn Engine,
    state: &AppState,
    last_panes: &crate::PaneRects,
) -> bool {
    let Some(gs) = session.as_any().downcast_ref::<app::session::GameSession>() else {
        return false;
    };
    let version = gs.machine.mem.version();
    if version < 4 || version == 6 {
        return false;
    }
    let Some((rows, cols)) = app::render::screen::declared_story_screen_dims(
        last_panes.story,
        state,
        version,
        gs.boot_screen_cols,
    ) else {
        return false;
    };
    let current = (
        gs.machine.mem.read_byte(0x20) as u16,
        gs.machine.mem.read_byte(0x21) as u16,
    );
    if current == (rows.min(255), cols.min(255)) {
        return false;
    }
    session.set_screen_dims(rows, cols);
    true
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
    let Some(gs) = crate::engine_helpers::zvm_session_opt_mut(session) else {
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
    crate::engine_helpers::zvm_session_opt_mut(session)
        .is_some_and(|gs| gs.settle_paced_pictures())
}

/// Refresh the engine input-mode flags and re-arm the timed-input / Glk-timer
/// deadlines. Returns `true` only for a prompt-visibility transition (the timer
/// re-arm never forces a redraw).
pub(crate) fn refresh_engine_input(
    state: &mut AppState,
    session: &mut dyn Engine,
) -> bool {
    let mut redraw = false;

    // Each new Glulx line-input request says what the input line should now hold
    // (Glk spec §4.2 `initlen`): text the game pre-loaded, editable, or nothing.
    // advent.blb's toolbar needs both — clicking Examine must leave "Examine " at
    // the prompt for the player to finish, while clicking a verb over an
    // already-typed noun runs that command itself and asks again empty, which must
    // not strand the noun at the prompt. Either way the game has just consumed or
    // cancelled the previous line, so its answer is authoritative and replaces
    // whatever the app was showing. (SQ-0562, SQ-0565)
    if let Some(text) = crate::engine_helpers::glulx_session_opt_mut(session)
        .and_then(|gs| gs.take_line_seed())
    {
        if state.input.value != text {
            state.input.clear();
            state.input.insert_str(&text);
            redraw = true;
        }
    }
    // ...and keep the game's buffer holding what the player has actually typed.
    // Glk lends the interpreter that buffer for the whole request so a cancel can
    // report the partial input; advent.blb's toolbar cancels on every button press
    // and preserves what it finds there, so a stale buffer made every later button
    // re-insert the FIRST verb — text the player may have already deleted. Written
    // after the seed above so a fresh prefill lands in the buffer too. (SQ-0565)
    if let Some(gs) = crate::engine_helpers::glulx_session_opt_mut(session) {
        gs.sync_line_input(&state.input.value);
    }

    // Update char_mode flag so the renderer hides the prompt during read_char.
    let prev_char_mode = state.char_mode;
    let prev_event_wait = state.event_wait;
    state.char_mode = matches!(session.pending_input(), app::session::InputKind::Char);
    // A Glulx timer/mouse/hyperlink-only glk_select: hide the prompt too (no
    // typed input is requested), but unlike char_mode do NOT forward keys to
    // the game — the timer clock / click delivers the event instead.
    state.event_wait = matches!(session.pending_input(), app::session::InputKind::Event);
    // A prompt-visibility transition changes the frame even with no new input.
    if state.char_mode != prev_char_mode || state.event_wait != prev_event_wait {
        redraw = true;
    }

    // Re-arm the timed-input deadline each iteration. Only while the game is
    // actually awaiting input (no dialog/overlay/prompt covering the pane) and
    // honoring timers; `pending_timeout()` is `None` for an untimed read, so
    // this is a no-op for the vast majority of games (regression guard). Timed
    // input is a Z-machine-only concept (ZMSD): `zvm_session_opt` is `None` for
    // a Glulx engine, so the timer never arms there.
    let timer_interval = crate::engine_helpers::zvm_session_opt(session)
        .and_then(|s| s.pending_timeout())
        .map(|(t, _)| Duration::from_millis(t as u64 * 100));
    let should_arm = state.config.honor_timed_input
        && !state.any_overlay_open()
        && timer_interval.is_some();
    state.input_deadline = crate::turn::next_input_deadline(
        state.input_deadline,
        should_arm,
        timer_interval.unwrap_or(Duration::ZERO),
        std::time::Instant::now(),
    );

    // Re-arm the Glulx Glk timer-events clock (glk_request_timer_events) — the
    // Glulx analogue of `input_deadline`, and independent of it. Armed only
    // when a Glulx game has requested a timer interval and no overlay covers
    // the pane; uses the same arm-once semantics (`next_input_deadline`) so the
    // deadline holds steady until it fires (the fire path below re-arms fresh).
    let glk_timer_interval = crate::engine_helpers::glulx_session_opt(session).and_then(|s| s.timer_interval());
    let should_arm_glk_timer = !state.any_overlay_open() && glk_timer_interval.is_some();
    state.glulx_timer_next_fire = crate::turn::next_input_deadline(
        state.glulx_timer_next_fire,
        should_arm_glk_timer,
        glk_timer_interval.unwrap_or(Duration::ZERO),
        std::time::Instant::now(),
    );

    redraw
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
pub(crate) fn catch_up_deferred_map_layout(
    state: &mut AppState,
    mapper: &Mapper,
    bg_tidy_counter: &mut u32,
) -> bool {
    if !state.map_layout_deferred
        || state.layout != app::state::Layout::Split
        || state.tidy_job.is_some()
    {
        return false;
    }
    state.map_layout_deferred = false;
    // `new_room = true` so this asks for a FULL relayout rather than a cleanup:
    // the deferred stretch may have added many rooms, and dead reckoning is
    // exactly what a full pass exists to straighten out.
    crate::turn::schedule_map_maintenance(state, mapper, true, true, bg_tidy_counter);
    true
}

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
            crate::turn::schedule_map_maintenance(state, mapper, false, true, bg_tidy_counter);
            changed = true;
        } else if app::random_exit_probe::owns(state, answer.token)
            && app::random_exit_probe::deliver(state, mapper, &answer)
        {
            // SQ-1257 Phase 2: an edge was just DELETED (a random exit confirmed), which is a
            // geometry change exactly like a new one — the render memo and any in-flight tidy
            // must not go on describing the edge that is now gone.
            state.graph_gen = state.graph_gen.wrapping_add(1);
            crate::turn::schedule_map_maintenance(state, mapper, false, true, bg_tidy_counter);
            changed = true;
        }
        // An answer nobody owns is one whose asker has moved on — an aborted
        // search, or a vocabulary offer the player typed past. Dropping it is the
        // silence discipline both consumers already have.
    }
    app::return_probe::pump_return_search(state);
    changed
}
