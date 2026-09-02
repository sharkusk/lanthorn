//! SlashOutcome side-effect dispatch: the single switch that applies a parsed
//! `SlashOutcome` (from typed input or a key binding) against the live app
//! state, engine, and mapper. Extracted verbatim from `main.rs` (SQ-0306) as a
//! pure move — no behavior change. Touches binary-only helpers (save/restore
//! plumbing, reset, hints, transcript export), all reached via `crate::`.

use app::archive::save_archive_meta_pics;
use app::engine::Engine;
use app::export::export_transcript;
use app::input::{apply_action, Action};
use app::persist_files::{load_map, save_named};
use app::slash::{self, SlashOutcome, TranscriptFilterArg};
use app::state::{AppState, ExitTarget, Focus, SavesState, TranscriptFilter, TranscriptKind};
use mapper::mapper::Mapper;
use ratatui::layout::Rect;

use crate::engine_helpers::{apply_archive_state, restore_from_file, zvm_session_opt, RestoreOutcome};
use crate::reset::reset_game;
use crate::{
    combined_saves, format_rfc3339, handle_map_export, open_hints, reobserve_location,
    scroll_for_match, should_prompt_save_on_quit, toggle_style_watch,
};

/// Handle a parsed `SlashOutcome` from either typed input or a key dispatch.
///
/// Both the typed-command path and the keybinding path resolve to a
/// `SlashOutcome` and funnel through here so the two share one behaviour. The
/// run loop owns the actual loop, so the `Quit` outcome cannot `break` directly:
/// this returns `true` when the loop should break (a non-dialog quit).
#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_slash_outcome(
    outcome: SlashOutcome,
    state: &mut AppState,
    mapper: &mut Mapper,
    session: &mut dyn Engine,
    style_watcher: &mut Option<app::watch::StyleWatcher>,
    game_dir: &std::path::Path,
    ifid: &str,
    arc_file: &std::path::Path,
    story_bytes: &[u8],
    story_path: &std::path::Path,
    map_rect: Rect,
    story_rect: Rect,
    from_key: bool,
) -> bool {
    match outcome {
        SlashOutcome::Action(a) => {
            if matches!(a, Action::OpenSaves) {
                // OpenSaves needs `game_dir` to populate the saves list, which
                // `apply_action` (state + mapper only) can't do — it just resets
                // the modal's focus. Build the populated dialog here so the
                // command/slash path (e.g. `restore-state`) actually opens it.
                let entries = combined_saves(game_dir);
                apply_action(Action::OpenSaves, state, mapper);
                state.overlays.saves = Some(SavesState { entries, scroll: Default::default() });
            } else if handle_map_export(&a, game_dir, mapper, state) {
                // handled
            } else if matches!(a, Action::ToggleWatch) {
                toggle_style_watch(state, style_watcher);
            } else {
                apply_action(a, state, mapper);
            }
        }
        SlashOutcome::Message(m) | SlashOutcome::Error(m) => {
            state.set_status(m);
        }
        SlashOutcome::Help => {
            for line in slash::help_text(state.config.command_prefix) {
                state.push_transcript_internal(&line, TranscriptKind::Meta);
            }
        }
        SlashOutcome::PrintColors { actual } => {
            for (line, style_opt) in app::theme::describe_theme(&state.colors.theme) {
                match (actual, style_opt) {
                    (true, Some(style)) => state.push_transcript_internal_styled(&line, TranscriptKind::Meta, style),
                    _ => state.push_transcript_internal(&line, TranscriptKind::Meta),
                }
            }
            // SQ-0510 diagnostics: what the OSC 10/11 startup probe actually
            // captured — the input to the v6 raster ink/page pair resolution.
            let fmt = |c: Option<image::Rgba<u8>>| match c {
                Some(image::Rgba([r, g, b, _])) => format!("rgb({r},{g},{b})"),
                None => "unanswered".to_string(),
            };
            let td = state.term_default_colors;
            state.push_transcript_internal(
                &format!("terminal defaults (OSC 10/11 probe): fg {} · bg {}", fmt(td.fg), fmt(td.bg)),
                TranscriptKind::Meta,
            );
        }
        SlashOutcome::DumpWindows => {
            // A v6 story reports one block per window, merging the game's window
            // table, the model, and where the last frame put each on the terminal —
            // the three things that have to agree (SQ-0585). The engine owns the
            // first two; the render mapping lives here, so hand it over.
            //
            // The mapping is the LAST GAME FRAME's, not the live one (SQ-0756). This
            // command is reached through the palette or a hotkey dialog, so the frame
            // in `v6_cell_map` is always the modal's, in which the game's windows are
            // all "NOT DRAWN this frame" — the one thing a reader is here to learn.
            // The game halves stay live: a modal runs no game code, so the window
            // table and the model still describe the frame being reported.
            let (frame, frame_line) = state.v6_dump_frame();
            let cells = frame.as_ref().map(|f| f.cells.clone()).unwrap_or_default();
            let history: Vec<String> = state
                .v6_path_log
                .borrow()
                .iter()
                .map(|(label, n)| if *n > 1 { format!("{label} x{n}") } else { label.clone() })
                .collect();
            let mut out: Vec<String> = match session.as_any().downcast_ref::<app::session::GameSession>() {
                Some(gs) if !gs.v6_window_dump(&cells, Some(&state.v6_text)).is_empty() => {
                    gs.v6_window_dump(&cells, Some(&state.v6_text))
                }
                _ => session.window_dump(),
            };
            out.push(frame_line);
            // The ring's plan and clip belong to the frame that drew them, so they
            // ride the same snapshot; with no game frame recorded there is nothing
            // honest to print (SQ-0756).
            let clip = match frame.as_ref() {
                None => "unavailable — no game frame recorded".to_string(),
                Some(f) => match f.ring_clip {
                    Some((art, row)) if art == u16::MAX => {
                        format!("plan {}, ring clipped at row {row} — NO opaque art found in the canvas", f.ring_plan)
                    }
                    Some((art, row)) => format!(
                        "plan {}, ring clipped at row {row} (art opaque down to native y={art})",
                        f.ring_plan
                    ),
                    None => format!("plan {}, ring not clipped", f.ring_plan),
                },
            };
            out.push(format!("  ring: {clip}"));
            // SQ-0588: windows whose recorded ops did not reproduce their canvas at
            // save time. Each is a draw path we are not recording — the archive fell
            // back to a PNG for it, so it restores correctly but cannot be recoloured
            // by a later palette change.
            let save_diags: Vec<String> = state.v6_save_log.borrow().clone();
            for msg in &save_diags {
                out.push(format!("  save: {msg}"));
            }
            // SQ-0593: the character-cell pixel size we hand the game. EVERYTHING a
            // Glulx game sizes in pixels is derived from it — advent.blb's toolbar
            // among them — so a wrong value here makes the game's own artwork come out
            // the wrong size, with nothing downstream to blame. Two ways it can be
            // wrong, and this line tells them apart: with no image protocol we fall
            // back to a hardcoded 8x16 regardless of the real font, and where there IS
            // one the Picker reports LOGICAL points, which on a 2x HiDPI display are
            // half the device pixels the terminal actually paints (the same mismatch
            // that keeps pixel-precise mouse reporting switched off, see startup.rs).
            // Compare `implies` against the real window size to tell which.
            let (raw_w, raw_h, src) = match state.game_picker.as_ref() {
                Some(p) => {
                    let f = p.font_size();
                    (f.width as u32, f.height as u32, "terminal-reported")
                }
                None => (8, 16, "FALLBACK — no image protocol, real font size unknown"),
            };
            // What the game actually sees, after SQ-0593 scaling. Reporting the raw
            // terminal value alone was misleading the moment the divisor existed.
            let (cw, ch) = state.config.glk_pixel_scale.apply((raw_w, raw_h));
            out.push(format!(
                "  cell size: terminal says {raw_w}x{raw_h} px ({src}); glk_pixel_scale \
                 → game sees {cw}x{ch}; pane {}x{} cells implies {}x{} px to the game",
                story_rect.width, story_rect.height,
                cw * story_rect.width as u32, ch * story_rect.height as u32
            ));
            let encodes = state.graphics_render.borrow().band_encodes;
            out.push(format!("  band uploads since launch: {encodes}"));
            let bands = state.graphics_render.borrow().band_log.clone();
            for b in bands {
                out.push(format!("  {b}"));
            }
            if !history.is_empty() {
                out.push(format!("  recent render paths (oldest first): {}", history.join(" · ")));
            }
            for line in &out {
                state.push_transcript_internal(line, TranscriptKind::Meta);
            }
            // …and the same text to a file, because the on-screen copy cannot be
            // copied: a v6 pane is drawn out of kitty unicode placeholder glyphs, and
            // a terminal selection over the dump takes them with it — the user's paste
            // came back placeholder-dense and truncated mid-field (SQ-0756). The log
            // is readable from another terminal while the game is still running.
            let msg = match app::export::append_window_dump(&state.config.user_dir, &out) {
                Ok(p) => format!("  [dump appended to {} — copy it from there, not off the screen]", crate::abbreviate_home(&p)),
                Err(e) => format!("  [dump log failed: {e}]"),
            };
            state.push_transcript_internal(&msg, TranscriptKind::Meta);
        }
        SlashOutcome::DumpCells => {
            // The rendered cells, glyphs AND styling, as plain text (SQ-0761). Only
            // the file gets the grid: it is two lines per terminal row, so echoing it
            // into the transcript would scroll the very frame the next capture is
            // meant to describe — and the transcript cannot be copied off a v6 pane
            // anyway, which is the whole reason SQ-0756 started writing files.
            let snapshot = state.last_frame_cells.borrow().clone();
            let msg = match snapshot {
                None => "[dump-cells] no frame has been drawn yet — nothing to dump".to_string(),
                Some(frame) => {
                    let lines = frame.lines();
                    match app::export::append_cell_dump(&state.config.user_dir, &lines) {
                        Ok(p) => format!(
                            "[dump-cells] {} rows x {} cols of glyphs + styling appended to {} \
                             — copy it from there, not off the screen",
                            frame.buf.area.height,
                            frame.buf.area.width,
                            crate::abbreviate_home(&p)
                        ),
                        Err(e) => format!("[dump-cells] log failed: {e}"),
                    }
                }
            };
            state.push_transcript_internal(&msg, TranscriptKind::Meta);
        }
        SlashOutcome::DumpTerminal => {
            // What was DETECTED about this terminal, what lanthorn GUESSED, and the
            // traffic those two together explain (SQ-0994). Everything below is read
            // from state something already tracked for its own reasons; nothing here
            // instruments the frame path.
            let snap = terminal_snapshot(state, session, story_rect);
            for line in app::terminal_dump::dump_lines(&snap) {
                let style = match line.kind {
                    app::terminal_dump::DumpKind::Heading => Some("terminal_dump_heading"),
                    app::terminal_dump::DumpKind::Assumed => Some("terminal_dump_assumed"),
                    app::terminal_dump::DumpKind::Value => None,
                };
                match style {
                    Some(sel) => state.push_transcript_internal_styled(
                        &line.text,
                        TranscriptKind::Meta,
                        state.colors.theme.get(sel).style,
                    ),
                    None => state.push_transcript_internal(&line.text, TranscriptKind::Meta),
                }
            }
            // …and the same report to a file, for the reason `/dump-windows` writes
            // one (SQ-0756): a v6 pane is drawn out of kitty placeholder glyphs and a
            // terminal selection over the transcript takes them with it, so the
            // on-screen copy is exactly the thing that cannot be pasted into a bug
            // report. This is the report most likely to be wanted in one.
            let text = app::terminal_dump::dump_text(&snap);
            let msg = match app::export::append_terminal_dump(&state.config.user_dir, &text) {
                Ok(p) => format!(
                    "  [report appended to {} — copy it from there, not off the screen]",
                    crate::abbreviate_home(&p)
                ),
                Err(e) => format!("  [dump log failed: {e}]"),
            };
            state.push_transcript_internal(&msg, TranscriptKind::Meta);
        }
        SlashOutcome::ToggleDebug => toggle_debug(state, session),
        SlashOutcome::DumpNotifications => {
            let history = state.notifications.history().to_vec();
            state.push_transcript_internal("[notifications]", TranscriptKind::Meta);
            if history.is_empty() {
                state.push_transcript_internal("  (none)", TranscriptKind::Meta);
            } else {
                for line in history {
                    state.push_transcript_internal(&format!("  {line}"), TranscriptKind::Meta);
                }
            }
        }
        SlashOutcome::PlaySound(None) => {
            for line in app::state::format_sound_resource_list(state.sound_blorb.as_ref(), &state.disk_sounds) {
                state.push_transcript_internal(&line, TranscriptKind::Meta);
            }
        }
        SlashOutcome::PlaySound(Some(n)) => {
            let mut report = app::state::PlaySoundReport {
                number: n,
                enable_sound: state.config.enable_sound,
                backend_present: state.audio.is_some(),
                blorb_present: state.sound_blorb.is_some(),
                disk_sounds: state.disk_sounds.len(),
                ..Default::default()
            };
            // The DIAGNOSTIC must resolve exactly as the play path does or it
            // answers a question nobody asked, so both go through `resolve_sound`
            // (SQ-0914). Copied to owned bytes so the borrow of `state` ends before
            // the mutable one for playback.
            let picked: Option<(Vec<u8>, blorb::SoundKind, Option<String>)> = u16::try_from(n)
                .ok()
                .and_then(|e| {
                    app::state::resolve_sound(&state.disk_sounds, state.sound_blorb.as_ref(), e)
                })
                .map(|(bytes, kind, name)| (bytes.to_vec(), kind, name.map(str::to_string)));
            if let Some((bytes, kind, from_medium)) = picked {
                report.resource = Some((kind, bytes.len()));
                report.from_medium = from_medium;
                if let Some(fmt) = app::state::sound_kind_to_format(kind) {
                    report.format = Some(fmt);
                    if let Some(backend) = state.audio.as_mut() {
                        report.sound_id = backend.play_sample(&bytes, fmt, 8, 1);
                    }
                }
            }
            for line in app::state::format_play_sound_report(&report) {
                state.push_transcript_internal(&line, TranscriptKind::Meta);
            }
        }
        SlashOutcome::Save(name_opt) => match name_opt {
            Some(ref name) => match app::persist_files::named_save_path(game_dir, name) {
                // SQ-0648: `/save <name>` clobbering an existing file with no
                // warning is exactly the bug — including a cross-name slugify
                // collision, which is why the prompt names the EXISTING
                // save's display name rather than echoing what was just typed.
                Ok(path) => match app::persist_files::existing_save_display_name(&path) {
                    Some(existing_name) => {
                        state.overlays.confirm_overwrite_save = Some(app::state::ConfirmOverwriteSave {
                            path,
                            existing_name,
                            pending: app::state::PendingOverwrite::Slash(name.clone()),
                        });
                        state.overlays.dialog_focus = 1; // Cancel default
                    }
                    None => {
                        let result = write_named_save(game_dir, ifid, name, mapper, session, state);
                        apply_slash_save_result(result, session, state);
                    }
                },
                Err(e) => state.set_status(format!("save failed: {}", e)),
            },
            None => {
                // The default archive slot is the auto/quick-save equivalent —
                // never a name the player typed — so it never prompts (SQ-0648).
                // SQ-0588: the display list travels with this save too — this is
                // the interactive Save State path, and an archive written
                // without it restores art that can never be recoloured.
                // Land any in-flight background per-turn auto-save first (SQ-1184):
                // this writes the same default slot, so an explicit /save that
                // reports "saved" must not race a background write for an
                // earlier turn onto the same file.
                state.archive_worker.flush();
                let (v6_pics, v6_display, v6_ground, v6_diags) = crate::engine_helpers::v6_save_payload(&mut *session);
                for d in &v6_diags { state.note_v6_save(d); }
                let (location, score) = crate::engine_helpers::save_summary(&*session, state);
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
                let result = save_archive_meta_pics(arc_file, &*mapper, &session.save_state(), zvm_session_opt(&*session).map(|z| &z.machine.screen), session.aux_data(), meta, &app::archive::SessionRecord::of(state), &v6_pics, v6_display.as_ref(), v6_ground.as_deref())
                    .map(|()| "saved".to_string())
                    .map_err(|e| format!("save failed: {}", e));
                apply_slash_save_result(result, session, state);
            }
        },
        SlashOutcome::Load(name_opt) => {
            // Named-slot load or default archive load. Named slots may be a
            // .lanthorn Save State or a .qzl game save (SQ-0227 Task 3).
            let archive_to_load = match name_opt {
                None => Some(arc_file.to_path_buf()),
                Some(ref name) => {
                    // Find the first named save whose display name matches.
                    let saves = combined_saves(game_dir);
                    saves.into_iter()
                        .find(|e| !e.is_default && e.name.to_lowercase() == name.to_lowercase())
                        .map(|e| e.path)
                }
            };
            match archive_to_load {
                None => {
                    state.set_status("load failed: no save found with that name");
                }
                Some(ref path) => {
                    let restore_outcome = restore_from_file(path, &mut *session);
                    app::trace::hostio(&state.config.user_dir, state.config.trace.hostio, format!("restore_state({})", path.display()));
                    match restore_outcome {
                        Ok(RestoreOutcome::DescriptorCompleted(ac)) => {
                            // An in-game @save archive carries the whole session
                            // alongside its game bytes (SQ-0531); a bare .qzl has
                            // nothing but the bytes.
                            if let Some(ac) = ac {
                                apply_archive_state(*ac, &mut *session, mapper, state);
                            }
                            reobserve_location(state, mapper, &*session, map_rect);
                            state.set_status("restored");
                        }
                        Ok(RestoreOutcome::Resumed(ac)) => {
                            apply_archive_state(*ac, &mut *session, mapper, state);
                            reobserve_location(state, mapper, &*session, map_rect);
                            state.set_status("loaded");
                        }
                        Err(e) => state.set_status(format!("load failed: {}", e)),
                    }
                }
            }
        }
        SlashOutcome::LoadMap(path) => {
            let full = app::colors::expand_path(&path, &std::env::current_dir().unwrap_or_default());
            match load_map(&full) {
                Some(m) => {
                    *mapper = m;
                    state.bump_graph_gen(); // imported map replaced the graph → invalidate memo (SQ-0305)
                    state.set_viewed_layer(None);
                    // A whole new graph switches the active layer to whatever the loaded map's
                    // current room sits on — route it through the same layer-switch recenter as
                    // cycling/tab-clicking/peel/merge, so a loaded map with no `pos` on its
                    // current room (or none at all) still lands somewhere sane (SQ-0672).
                    app::input::recenter_for_active_layer(state, &mapper.graph);
                    state.set_status(format!("loaded map: {}", full.display()));
                }
                None => state.set_status(format!("load-map failed: {}", full.display())),
            }
        }
        SlashOutcome::Reset { map: reset_map, data: reset_data } => {
            // A key press (e.g. F5) or a bare `/reset-game` opens the confirmation
            // dialog with its map/data checkboxes; an explicit-token form
            // (`/reset-game map`, `data`, or both) acts immediately as typed.
            if from_key || (!reset_map && !reset_data) {
                apply_action(Action::ResetGame, state, mapper);
            } else {
                reset_game(session, mapper, state, story_bytes, story_path, game_dir, reset_map, reset_data);
                let mut status_msg = String::from("reset");
                if reset_map { status_msg.push_str(" (map cleared)"); }
                if reset_data { status_msg.push_str(" (data deleted)"); }
                state.set_status(&status_msg);
            }
        }
        SlashOutcome::Quit => {
            // A plain quit resolves the loop to Exit. Set it explicitly so a
            // prior `/quit-to-library` that opened (then was superseded by) this
            // path can't leave the target pointing at the library. (SQ-0435)
            state.exit_target = ExitTarget::Exit;
            if should_prompt_save_on_quit(state) {
                state.overlays.quit_dialog = true;
                state.overlays.dialog_focus = 0;
            } else {
                return true;
            }
        }
        SlashOutcome::QuitToLibrary => {
            // Only meaningful when launched from a directory (a picker exists).
            if !state.launched_from_library {
                state.set_status("No story library — launched with a single file");
            } else {
                // Return-to-library mirrors Quit's save handling, but resolves
                // the loop to the library instead of exiting. The break sites
                // read `state.exit_target`. (SQ-0435)
                state.exit_target = ExitTarget::Library;
                if should_prompt_save_on_quit(state) {
                    state.overlays.quit_dialog = true;
                    state.overlays.dialog_focus = 0;
                } else {
                    return true;
                }
            }
        }
        SlashOutcome::Search(q_opt) => {
            let query_to_run: Option<String> = match q_opt {
                Some(q) => Some(q),
                None => state.search_query.clone(),
            };
            match query_to_run {
                None => {
                    state.set_status("search: no previous search");
                }
                Some(query) => {
                    let count = state.run_search(&query, state.config.search.start_backward);
                    if count == 0 {
                        state.set_status("search: no matches");
                    } else {
                        state.set_status(format!("search: {} match{}", count, if count == 1 { "" } else { "es" }));
                        // Scroll to the current match.
                        let pos = state.search_matches[state.search_idx];
                        let total_vis = state.visible_transcript_indices().len();
                        let pane_rows = if story_rect.height > 0 {
                            story_rect.height as usize
                        } else {
                            24
                        };
                        state.transcript_scroll = scroll_for_match(pos, total_vis, pane_rows);
                    }
                }
            }
        }
        SlashOutcome::Filter(arg) => {
            state.transcript_filter = match arg {
                TranscriptFilterArg::Both  => TranscriptFilter::Both,
                TranscriptFilterArg::Story => TranscriptFilter::Story,
                TranscriptFilterArg::Meta  => TranscriptFilter::Meta,
            };
            let label = match state.transcript_filter {
                TranscriptFilter::Both  => "both",
                TranscriptFilter::Story => "story",
                TranscriptFilter::Meta  => "meta",
            };
            // If a search is active, recompute it against the new filter
            // so highlights and the [i/N] hint stay consistent.
            if let Some(query) = state.search_query.clone() {
                let count = state.run_search(&query, state.config.search.start_backward);
                if count > 0 {
                    let pos = state.search_matches[state.search_idx];
                    let total_vis = state.visible_transcript_indices().len();
                    let pane_rows = if story_rect.height > 0 {
                        story_rect.height as usize
                    } else {
                        24
                    };
                    state.transcript_scroll = scroll_for_match(pos, total_vis, pane_rows);
                }
            }
            state.set_status(format!("filter: {}", label));
        }
        SlashOutcome::Export(dest) => {
            // The VISIBLE transcript as a FILE should carry it: an assist
            // identifies itself on screen with the mark in its gutter, and a file
            // has no gutter, no colour and nothing a screen reader can voice, so
            // `Lanthorn: ` goes back on the front of every line that is ours
            // (SQ-1045). `AppState` owns that rule, next to the door the lines
            // came through.
            let lines: Vec<String> = state.transcript_for_export();
            match export_transcript(&lines, dest.as_deref(), game_dir) {
                Ok(path) => state.set_status(format!("exported: {}", path.display())),
                Err(e)   => state.set_status(format!("export failed: {}", e)),
            }
        }
        SlashOutcome::OpenHints => {
            let ud = state.config.user_dir.clone();
            open_hints(state, story_path, ifid, &ud);
        }
        SlashOutcome::HelpCommand(name) => {
            for line in slash::help_for_command(state.config.command_prefix, &name) {
                state.push_transcript_internal(&line, TranscriptKind::Meta);
            }
        }
        SlashOutcome::SetGameColours(opt) => {
            // Persist the per-game override (or clear it on `auto`), then recompute
            // the live look + honor precedence from disk (SQ-0318). reload_style
            // applies `per_game > garglk.ini > global`, so an explicit choice wins
            // and `auto` falls back to garglk/global.
            match app::styles::write_per_game_honor(game_dir, opt) {
                Ok(()) => {
                    // SQ-0855: an explicit choice here ends a `--game-colours`
                    // launch's hold — the user is overriding their own flag, which
                    // is the same event as a settings-panel edit ending
                    // `--interpreter`'s (SQ-0646). Without this the command would
                    // report success and change nothing.
                    state.game_colours_cli = None;
                    // SQ-0860: and the artwork's force-off, for the same reason and
                    // then some — the archive's half of `declines_game_colours` is
                    // expressly a GUESS about a machine, and a player who typed this
                    // command has just settled the question by hand.
                    state.artwork_declines_colours = false;
                    let _ = app::reload::reload_style(state);
                    // Follow through to the running Z-machine: future set_colour
                    // ops and the Flags1 colour capability track the new setting.
                    // (Render-side gates handle colours already recorded.)
                    if let Some(zs) = session.as_any_mut().downcast_mut::<app::session::GameSession>() {
                        zs.machine.set_honor_game_colours(state.config.honor_game_colours);
                    }
                    let label = match opt {
                        Some(true) => "on",
                        Some(false) => "off",
                        None => "auto",
                    };
                    state.push_transcript_internal(
                        &format!(
                            "game colours: {label} (honor_game_colours = {})",
                            state.config.honor_game_colours
                        ),
                        TranscriptKind::Meta,
                    );
                }
                Err(e) => state.set_status(format!("set-game-colours failed: {e}")),
            }
        }
        SlashOutcome::SetGameBorderless(opt) => {
            // Persist the per-game borderless override (or clear it on `auto`),
            // then apply it live: the running Glulx session relayouts so windows
            // abut (or regain their border gutters) immediately. (SQ-0341)
            match app::styles::write_per_game_borderless(game_dir, opt) {
                Ok(()) => {
                    let effective =
                        app::styles::read_per_game_borderless(game_dir).unwrap_or(false);
                    if let Some(gs) = session.as_any_mut().downcast_mut::<app::glulx_session::GlulxSession>() {
                        gs.set_borderless(effective);
                    }
                    let label = match opt {
                        Some(true) => "off (borderless)",
                        Some(false) => "on",
                        None => "auto (on)",
                    };
                    state.push_transcript_internal(
                        &format!("window borders: {label}"),
                        TranscriptKind::Meta,
                    );
                }
                Err(e) => state.set_status(format!("set-game-borders failed: {e}")),
            }
        }
        SlashOutcome::RunFontCheck => {
            // SQ-1104: open the same modal the first run raises. Focus starts on
            // the second button — the answer that changes nothing — matching the
            // dialog's declared default, so Enter without reading is not a
            // decision to install glyphs the font may not have.
            state.overlays.dialog_focus = 1;
            state.overlays.font_check = true;
        }
        SlashOutcome::SetGuidance(arg) => {
            // SQ-1045 put the Guiding Light on a switch; SQ-1123 made the switch
            // stick to the GAME. Whether you want help is a standing preference
            // about the story in front of you — off for the one you know by
            // heart, on for the one you just opened — so it belongs in the
            // per-game sidecar, exactly as `set-v6-pixel-lock` already does. The
            // settings screen still owns the global default new games inherit.
            use app::slash::GuidanceArg;
            let want = match arg {
                GuidanceArg::On => Some(true),
                GuidanceArg::Off => Some(false),
                GuidanceArg::Auto => None,
                GuidanceArg::Toggle => Some(!state.config.guidance),
            };
            match app::styles::write_per_game_guidance(game_dir, want) {
                Ok(()) => {
                    // `auto` falls back to the global value captured at boot —
                    // the one the sidecar overrode, and the only place it survives.
                    state.config.guidance = want.unwrap_or(state.guidance_base);
                    // A per-game choice must never reach the user's global
                    // config.toml: pin it while it is in force, release on `auto`.
                    match want {
                        Some(v) => state.config.one_run.pin(app::config::keys::GUIDANCE, v),
                        None => state.config.one_run.release(app::config::keys::GUIDANCE),
                    }
                    let label = match want {
                        Some(true) => "on",
                        Some(false) => "off",
                        None => "auto",
                    };
                    // Said as META, not as an assist: this is a report of something
                    // lanthorn did, and an assist announcing that assists are now off
                    // would be the one line the switch could not silence.
                    state.push_transcript_internal(
                        &format!(
                            "Lanthorn's Guiding Light: {label} (for this game — guidance = {})",
                            state.config.guidance
                        ),
                        TranscriptKind::Meta,
                    );
                }
                Err(e) => state.set_status(format!("set-guidance failed: {e}")),
            }
        }
        SlashOutcome::SetReturnProbe(arg) => {
            // The same four-state shape every persisted control has (SQ-1123):
            // bare flips the LIVE value, `auto` clears the override so the global
            // setting decides again. Off by default globally, which makes `auto`
            // meaningfully different from `off` here rather than a synonym for it.
            use app::slash::ReturnProbeArg;
            let want = match arg {
                ReturnProbeArg::On => Some(true),
                ReturnProbeArg::Off => Some(false),
                ReturnProbeArg::Auto => None,
                ReturnProbeArg::Toggle => Some(!state.config.return_probe),
            };
            match app::styles::write_per_game_return_probe(game_dir, want) {
                Ok(()) => {
                    state.config.return_probe = want.unwrap_or(state.return_probe_base);
                    match want {
                        Some(v) => state.config.one_run.pin(app::config::keys::RETURN_PROBE, v),
                        None => state.config.one_run.release(app::config::keys::RETURN_PROBE),
                    }
                    // Switching it off ends whatever is in flight. The shadow's
                    // answer would still be true, but a player who has just turned
                    // the feature off is entitled to it stopping now rather than
                    // one edge later.
                    if !state.config.return_probe {
                        state.return_search = None;
                    }
                    let label = match want {
                        Some(true) => "on",
                        Some(false) => "off",
                        None => "auto",
                    };
                    state.push_transcript_internal(
                        &format!(
                            "return probe: {label} (for this game — return_probe = {})",
                            state.config.return_probe
                        ),
                        TranscriptKind::Meta,
                    );
                }
                Err(e) => state.set_status(format!("set-return-probe failed: {e}")),
            }
        }
        SlashOutcome::RevealWords => {
            // A trigger: nothing to write, nothing to inherit, no transcript line.
            // A META line here would be worse than useless — it is an insert above
            // the prompt, so it would scroll the very screenful the reveal was
            // asked about, and the words would light on text that had moved.
            //
            // What is said out loud is what the reveal could NOT do, and its own
            // claim — through a toast, which floats over the pane without
            // touching it.
            use app::reveal::Armed;
            match app::reveal::arm(state, &*session) {
                // A lit reveal says nothing at all (user decision, SQ-1214): the
                // words lighting up IS the answer, and the caveat legend that used
                // to ride the status line on every press was one more thing to
                // read over the thing being read. The claim it stated — words the
                // story KNOWS, not necessarily things that are here — lives in
                // the control's own description now, said once where the feature
                // is discovered instead of on every use. The arms below still
                // speak, because each names the reason nothing lit.
                Armed::Lit { .. } => {}
                Armed::Nothing => {
                    state.set_status("[nothing on screen is a word this story takes]")
                }
                // A different sentence, because it is a different claim: the one
                // above is about the ROOM, and this one is about US (SQ-1150).
                Armed::NoVocabulary => {
                    state.set_status("[lanthorn cannot read this story's words]")
                }
                Armed::NoText => state.set_status("[no story text on screen to read]"),
                Armed::GuidanceOff => state.set_status(
                    "[the Guiding Light is out — /set-guidance on to use the reveal]",
                ),
            }
        }
        SlashOutcome::SetV6Render(arg) => {
            // Session-only until SQ-1123, and that was the right design for what
            // this once was: raster began as a FALLBACK — the mode you escaped to
            // when hybrid could not cope — and a temporary escape hatch should not
            // outlive the session. Raster is a destination now, with `extended`
            // beside it (SQ-1032), and a player may genuinely prefer raster for one
            // game and hybrid for another. So the mode sticks to the game it was
            // chosen for, in the per-game sidecar, exactly as the pixel lock does.
            //
            // Bare CYCLES — with three modes there is no "the other one" to
            // toggle to, and the cycle is the order the settings screen's own row
            // already walks. The cycle never visits `auto`: "inherit" has no look
            // of its own to show, so returning to the global default is the
            // command's `auto` argument rather than a fourth step nobody could see.
            use app::config::V6RenderMode;
            use app::slash::V6RenderArg;
            let want = match arg {
                V6RenderArg::Mode(m) => Some(m),
                V6RenderArg::Auto => None,
                V6RenderArg::Cycle => Some(match state.config.v6_render {
                    V6RenderMode::Hybrid => V6RenderMode::Raster,
                    V6RenderMode::Raster => V6RenderMode::Extended,
                    V6RenderMode::Extended => V6RenderMode::Hybrid,
                }),
            };
            let key = want.map(|m| app::config::v6_render_key(m).to_string());
            match app::styles::write_per_game_v6_render(game_dir, key.clone()) {
                Ok(()) => {
                    // Live from the next frame: the render reads this field afresh
                    // every draw. `auto` falls back to the global mode captured at
                    // boot, which is the value the sidecar overrode.
                    state.config.v6_render = want.unwrap_or(state.v6_render_base);
                    match key {
                        Some(k) => state.config.one_run.pin(app::config::keys::V6_RENDER, k),
                        None => state.config.one_run.release(app::config::keys::V6_RENDER),
                    }
                    let label = match want {
                        Some(m) => app::config::v6_render_key(m),
                        None => "auto",
                    };
                    state.push_transcript_internal(
                        &format!(
                            "v6 render: {label} (for this game — v6_render = {})",
                            app::config::v6_render_key(state.config.v6_render)
                        ),
                        TranscriptKind::Meta,
                    );
                }
                Err(e) => state.set_status(format!("set-v6-render failed: {e}")),
            }
        }
        SlashOutcome::SetV6PixelLock(arg) => {
            // SQ-0945: the runtime switch for SQ-0936's magnification ladder.
            // Per-game, not global: the ladder's step is derived from the
            // artwork's own density, so whether locking is worth its wider margin
            // is a question about this story's press rather than about lanthorn.
            use app::slash::V6PixelLockArg;
            let want = match arg {
                V6PixelLockArg::On => Some(true),
                V6PixelLockArg::Off => Some(false),
                V6PixelLockArg::Auto => None,
                V6PixelLockArg::Toggle => Some(!state.config.v6_pixel_lock),
            };
            match app::styles::write_per_game_v6_pixel_lock(game_dir, want) {
                Ok(()) => {
                    // Live from the next frame: the render reads this field afresh
                    // every draw, so there is nothing to relayout by hand. `auto`
                    // falls back to the global default captured at boot, which is
                    // the value the sidecar overrode and the only place it survives.
                    state.config.v6_pixel_lock = want.unwrap_or(state.v6_pixel_lock_base);
                    // A per-game choice must never reach the user's global
                    // config.toml — pin it while it is in force, and release the
                    // pin on `auto`, when nothing is overriding the key any more.
                    match want {
                        Some(v) => state.config.one_run.pin(app::config::keys::V6_PIXEL_LOCK, v),
                        None => state.config.one_run.release(app::config::keys::V6_PIXEL_LOCK),
                    }
                    let label = match want {
                        Some(true) => "on",
                        Some(false) => "off",
                        None => "auto",
                    };
                    state.push_transcript_internal(
                        &format!(
                            "v6 pixel lock: {label} (v6_pixel_lock = {})",
                            state.config.v6_pixel_lock
                        ),
                        TranscriptKind::Meta,
                    );
                }
                Err(e) => state.set_status(format!("set-v6-pixel-lock failed: {e}")),
            }
        }
        SlashOutcome::Trace(arg) => {
            match arg {
                None => {
                    state.set_status(format!("[trace: {}]", state.config.trace.active_list()));
                }
                Some(list) => {
                    let (sections, unknown) = app::trace::TraceSections::parse(&list);
                    state.config.trace = sections;
                    session.set_trace_screen(sections.screen);
                    if sections.any() && !unknown.is_empty() {
                        state.set_status(format!("[trace: {} — ignored: {}]", sections.active_list(), unknown.join(",")));
                    } else if !unknown.is_empty() {
                        state.set_status(format!("[trace: unknown section(s): {}]", unknown.join(",")));
                    } else {
                        state.set_status(format!("[trace: {}]", sections.active_list()));
                    }
                }
            }
        }
        // The story browser's own verbs (SQ-0796). `parse_in_context` only emits
        // these in `Context::Browser`, which the running game never is, and the
        // picker loop applies them itself — so reaching here means a binding was
        // resolved in the wrong context, and the honest answer is to say so
        // rather than silently swallow the key.
        SlashOutcome::Browser(_) => {
            state.set_status("[that command belongs to the story browser]".to_string());
        }
    }
    false
}

/// Gather everything `/dump-terminal` reports (SQ-0994).
///
/// The one place that can: the live `Picker` (protocol, cell size, capability
/// list), a `TIOCGWINSZ` ioctl, the render's published v6 facts, and the byte
/// counters hanging off the ratatui backend. Everything downstream — the
/// transcript, the log mirror, the tests — reads only the returned snapshot, so
/// the report itself stays a pure function and can be asserted with no terminal
/// at all.
///
/// Costs one ioctl, plus one `Engine::screen()` on a v6 session and none
/// otherwise — both at command time. Nothing here is sampled per frame.
fn terminal_snapshot(
    state: &AppState,
    session: &dyn Engine,
    story_rect: Rect,
) -> app::terminal_dump::TerminalSnapshot {
    use app::terminal_dump::{CellSource, OpCounts, Probe, RenderFacts, TerminalSnapshot, TrafficStats};
    use ratatui_image::picker::{Capability, ProtocolType};

    let picker = state.game_picker.as_ref();
    let protocol = picker.map(|p| {
        match p.protocol_type() {
            ProtocolType::Halfblocks => "halfblocks",
            ProtocolType::Sixel => "sixel",
            ProtocolType::Kitty => "kitty",
            ProtocolType::Iterm2 => "iterm2",
        }
        .to_string()
    });
    // `Auto` is detection; anything else was named on the command line, and the
    // report must not let a forced answer read as a detected one.
    let forced_protocol = match state.config.image_protocol {
        app::config::ImageProtocol::Auto => None,
        app::config::ImageProtocol::Halfblocks => Some("halfblocks".to_string()),
        app::config::ImageProtocol::Kitty => Some("kitty".to_string()),
        app::config::ImageProtocol::Sixel => Some("sixel".to_string()),
        app::config::ImageProtocol::Iterm2 => Some("iterm2".to_string()),
    };
    // An empty capability list is three different facts. Only
    // `build_cover_picker`'s `Auto`/named arms run `Picker::from_query_stdio`;
    // `halfblocks()` asks nothing, and `--images off` builds no picker at all.
    // Read off the CONFIG rather than off `picker.is_none()`, because a picker is
    // also absent when a forced protocol's query failed — which is a terminal that
    // was asked and could not answer, the opposite of never having been asked.
    let probe = if !state.config.images {
        Probe::NotAskedImagesOff
    } else if state.config.image_protocol == app::config::ImageProtocol::Halfblocks {
        Probe::NotAskedHalfblocksForced
    } else {
        Probe::Asked
    };

    let no_caps: Vec<Capability> = Vec::new();
    let caps: &[Capability] = picker.map_or(&no_caps, |p| p.capabilities());
    let capabilities: Vec<String> = caps
        .iter()
        .map(|c| match c {
            Capability::Kitty => "Kitty — the kitty graphics protocol".to_string(),
            Capability::Sixel => "Sixel".to_string(),
            Capability::RectangularOps => "RectangularOps".to_string(),
            Capability::KittyCompression => {
                "KittyCompression — the terminal can inflate an o=z transmission".to_string()
            }
            Capability::CellSize(Some((w, h))) => format!("CellSize({w}x{h} px, from CSI 16 t)"),
            Capability::CellSize(None) => "CellSize (answered, but named no size)".to_string(),
            Capability::TextSizingProtocol => "TextSizingProtocol".to_string(),
            Capability::Background(r, g, b) => format!("Background(rgb({r},{g},{b}))"),
        })
        .collect();
    let kitty_compression = caps.contains(&Capability::KittyCompression);

    let cell = picker.map(|p| {
        let f = p.font_size();
        (f.width, f.height)
    });
    // What `CSI 16 t` said, if it said anything — the direct measurement.
    let reported_cell = caps.iter().find_map(|c| match c {
        Capability::CellSize(Some((w, h))) => Some((*w, *h)),
        _ => None,
    });
    // …and what the tty says right NOW. Asked live rather than remembered,
    // because `refresh_cell_size` re-derives from exactly this on every resize
    // (SQ-0988) — so a remembered boot-time answer could be stale in a way the
    // live one cannot.
    let ioctl_cell = crate::picker_ui::terminal_cell_size().map(|f| (f.width, f.height));
    // Ordered by directness: the CSI answer if it is still the value in force,
    // then the ioctl, then the crate's hardcoded 10x20 — which is the one that
    // has to be called a guess, because it is one.
    let cell_source = match cell {
        None => CellSource::None,
        Some(c) if reported_cell == Some(c) => CellSource::Measured,
        Some(c) if ioctl_cell == Some(c) => CellSource::Derived,
        Some((10, 20)) => CellSource::Assumed,
        Some(_) => CellSource::Unexplained,
    };

    // The v6 facts, and only for a session that has them. `v6_path_log` is
    // written by the pixel path and by nothing else, so an empty one means there
    // is no v6 geometry to report — and the engine is not asked for its window
    // tree at all, which keeps a Glulx or v3 dump down to the terminal half.
    let render = if state.v6_path_log.borrow().is_empty() {
        None
    } else {
        match session.screen().root {
            app::engine::WinNode::Layered(items) => {
                let native = app::render::v6_layout::native_extent(&items, &state.v6_text);
                let hybrid = state.config.v6_render == app::config::V6RenderMode::Hybrid;
                Some(RenderFacts {
                    mode: match state.config.v6_render {
                        app::config::V6RenderMode::Hybrid => "hybrid",
                        app::config::V6RenderMode::Raster => "raster",
                        app::config::V6RenderMode::Extended => "extended",
                    },
                    takeover: state.v6_takeover_reason.get(),
                    // The hatch is a hybrid-mode test; in raster the cell holds
                    // whatever some earlier hybrid frame left, which is not a verdict
                    // about this session.
                    takeover_evaluated: hybrid,
                    native,
                    art_scale: state.v6_art_scale,
                    magnification: state.v6_image_scale.get(),
                    pixel_lock: state.config.v6_pixel_lock,
                    pixel_lock_fell_back: state.v6_scale_lock_fallback.get(),
                    pixel_lock_inapplicable: state.v6_scale_lock_inapplicable.get(),
                    paths: state
                        .v6_path_log
                        .borrow()
                        .iter()
                        .map(|(label, n)| if *n > 1 { format!("{label} x{n}") } else { label.clone() })
                        .collect(),
                })
            }
            _ => None,
        }
    };

    // The APC command count, taken from the render's own op log rather than off
    // the wire: the ops ARE the commands, and reading them is free, where finding
    // `\x1b_G` in the stream would mean scanning every byte of every frame.
    let gr = state.graphics_render.borrow();
    let mut ops = OpCounts::default();
    for op in gr.ops() {
        use app::render::graphics::GraphicsOp;
        match op {
            GraphicsOp::Upload { .. } => ops.uploads += 1,
            GraphicsOp::Reuse { .. } => ops.reuses += 1,
            GraphicsOp::Place { at: (_, _, w, h), .. } => {
                ops.places += 1;
                ops.placed_cells += u64::from(*w) * u64::from(*h);
            }
            GraphicsOp::Drop { .. } => ops.drops += 1,
        }
    }

    TerminalSnapshot {
        protocol,
        forced_protocol,
        probe,
        cell,
        cell_source,
        reported_cell,
        ioctl_cell,
        capabilities,
        kitty_compression,
        pane_cells: (story_rect.width, story_rect.height),
        render,
        traffic: state.term_traffic.as_ref().map(|t| TrafficStats {
            total_bytes: t.total_bytes(),
            flushes: t.flushes(),
            last_flush_bytes: t.last_flush_bytes(),
        }),
        band_encodes: gr.band_encodes,
        uploads: gr.uploads,
        ops,
    }
}

/// Write a named Save State via the slash `/save <name>` path (SQ-0648).
///
/// Factored out of the `SlashOutcome::Save(Some(name))` arm so the
/// overwrite-confirm resolver (`main.rs`'s `OverlayAct::ConfirmOverwrite`
/// handler, reached once the player answers the prompt) can perform the SAME
/// write without re-deriving it. Always writes — the existence check that
/// decides whether to prompt lives in the caller, not here.
pub(crate) fn write_named_save(
    game_dir: &std::path::Path,
    ifid: &str,
    name: &str,
    mapper: &Mapper,
    session: &mut dyn Engine,
    state: &mut AppState,
) -> Result<String, String> {
    // SQ-0588: the display list travels with every host save — an archive
    // written without it restores art that can never be recoloured.
    let (v6_pics, v6_display, v6_ground, v6_diags) = crate::engine_helpers::v6_save_payload(session);
    for d in &v6_diags { state.note_v6_save(d); }
    let (location, score) = crate::engine_helpers::save_summary(&*session, state);
    save_named(
        game_dir, ifid, name, app::archive::SaveTrigger::HostState, mapper, &session.save_state(),
        zvm_session_opt(&*session).map(|z| &z.machine.screen), &v6_pics, v6_display.as_ref(),
        v6_ground.as_deref(), session.aux_data(), state.turns, location, score,
        &app::archive::SessionRecord::of(state),
    )
        .map(|()| format!("saved as \"{}\"", name))
        .map_err(|e| format!("save failed: {}", e))
}

/// Apply the result of a slash-path save write (named or default archive) to
/// `state`: clear the unsaved-progress flag and post the status line on
/// success, trace it, or just post the error on failure. Shared by the direct
/// write and the overwrite-confirm resolver (SQ-0648).
pub(crate) fn apply_slash_save_result(result: Result<String, String>, session: &mut dyn Engine, state: &mut AppState) {
    match result {
        Ok(msg) => {
            // Progress is now captured in a Save State — quitting is safe.
            state.unsaved_progress = false;
            if state.config.trace.hostio {
                app::trace::hostio(&state.config.user_dir, true, format!("save_state({} bytes)", session.save_state().bytes.len()));
            }
            state.set_status(msg);
        }
        Err(e) => state.set_status(e),
    }
}

/// Toggle the Z-machine debug inspector pane: activate it at the engine's current PC
/// (focusing the region), or deactivate it (map returns, focus back to the game).
/// Factored out of the `ToggleDebug` arm so it's testable without a full
/// `dispatch_slash_outcome` call.
fn toggle_debug(state: &mut AppState, session: &mut dyn Engine) {
    if state.debug.is_some() {
        state.debug = None;
        state.focus = Focus::Game;
        session.set_debug_trace(false);
    } else if session.debugger().is_some() {
        session.set_debug_trace(true);
        // Seed cumulative coverage from a prior `--debug` run's sidecar so a casual
        // `/debug` immediately shows the blue "ever executed" lines (SQ-0449).
        // Idempotent — re-seeding an already-populated set is harmless.
        let loaded = app::pcset_store::read_pcs(&state.game_dir);
        if !loaded.is_empty() {
            session.seed_executed_pcs(&loaded);
        }
        let dbg = session.debugger().expect("checked above");
        let mut panel = app::debug_panel::DebugPanelState::new(dbg.pc());
        panel.apply_engine_layout(dbg);
        panel.refresh(dbg);
        state.debug = Some(panel);
        state.focus = Focus::Map;
    } else {
        state.push_transcript_internal(
            "debugger not available for this engine", TranscriptKind::Meta);
    }
}

#[cfg(all(test, feature = "t-input"))]
mod debug_dispatch_tests {
    use super::*;
    use app::engine::{Debugger, EngineError, EngineSave, KeyInput, LocationInfo, ScreenModel};
    use app::session::{InputKind, TurnResult};
    use std::any::Any;
    use std::collections::BTreeMap;

    /// A minimal `Engine` double: every method the debug-open path doesn't touch
    /// panics if called, so a wiring mistake fails loudly instead of silently.
    struct MockEngine {
        has_debugger: bool,
        aux: BTreeMap<String, Vec<u8>>,
    }

    struct MockDebugger;
    impl Debugger for MockDebugger {
        fn pc(&self) -> u32 { 0x1234 }
        fn disassemble(&self, _addr: u32, _lines: usize) -> Vec<String> { vec!["1234  nop".into()] }
        fn disassemble_raw(&self, _addr: u32, _lines: usize) -> Vec<String> { vec!["1234: b4  0OP:0x04".into()] }
        fn disassemble_basic(&self, _addr: u32, _lines: usize) -> Vec<String> { vec!["1234  loadw #0abc".into()] }
        fn next_instr(&self, addr: u32) -> u32 { addr + 1 }
        fn prev_instr(&self, addr: u32) -> u32 { addr.saturating_sub(1) }
        fn executed_pcs(&self) -> std::collections::HashSet<u32> { std::collections::HashSet::new() }
        fn stack_lines(&self) -> Vec<String> { Vec::new() }
        fn eval_stack_lines(&self) -> Vec<String> { Vec::new() }
        fn locals_lines(&self) -> Vec<String> { Vec::new() }
        fn globals_lines(&self) -> Vec<String> { Vec::new() }
        fn object_tree_lines(&self) -> Vec<String> { Vec::new() }
        fn dictionary_lines(&self) -> Vec<String> { Vec::new() }
        fn memory_hex(&self, _addr: u32, _rows: usize) -> Vec<String> { Vec::new() }
        fn memory_len(&self) -> u32 { 0x10000 }
        fn object_detail(&self, _obj: u16) -> Vec<String> { Vec::new() }
        fn frame_locals(&self, _idx: usize) -> Vec<String> { Vec::new() }
        fn var_value(&self, _var: u8) -> Option<u16> { None }
    }

    impl Engine for MockEngine {
        fn submit(&mut self, _command: &str) -> TurnResult { unimplemented!() }
        fn submit_key(&mut self, _key: KeyInput) -> Option<TurnResult> { unimplemented!() }
        fn take_transcript(&mut self) -> String { unimplemented!() }
        // No screen-clear channel: this double is not a game.
        fn drain_screen_clear(&mut self) -> bool { false }
        fn pending_input(&self) -> InputKind { InputKind::Line }
        fn resume_save(&mut self, _wrote_ok: bool) -> TurnResult { unimplemented!() }
        fn resume_restore(&mut self, _data: Option<&[u8]>) -> TurnResult { unimplemented!() }
        fn has_quit(&self) -> bool { false }
        fn screen(&self) -> ScreenModel { unimplemented!() }
        fn save_state(&self) -> EngineSave { EngineSave::new("mock", 1, Vec::new()) }
        fn restore_state(&mut self, _save: &EngineSave) -> Result<(), EngineError> { Ok(()) }
        fn restore_game_save(&mut self, _bytes: &[u8]) -> Result<(), EngineError> { Ok(()) }
        fn aux_data(&self) -> &BTreeMap<String, Vec<u8>> { &self.aux }
        fn set_aux_data(&mut self, data: BTreeMap<String, Vec<u8>>) { self.aux = data; }
        fn aux_dirty(&self) -> bool { false }
        fn clear_aux_dirty(&mut self) {}
        fn current_location(&self) -> Option<LocationInfo> { None }
        fn as_any(&self) -> &dyn Any { self }
        fn as_any_mut(&mut self) -> &mut dyn Any { self }
        fn debugger(&self) -> Option<&dyn Debugger> {
            if self.has_debugger { Some(&MockDebugger) } else { None }
        }
        /// The engine's half of `/dump-windows`. Overridden because the trait's
        /// default derives it from `screen()`, which this double does not model —
        /// what the SQ-0777 cases are about is the frame half, which the app owns.
        fn window_dump(&self) -> Vec<String> {
            vec!["Window layout: mock engine".to_string()]
        }
    }

    #[test]
    fn toggle_debug_opens_panel_when_engine_has_debugger() {
        let mut state = AppState::default();
        let mut engine = MockEngine { has_debugger: true, aux: BTreeMap::new() };
        toggle_debug(&mut state, &mut engine);
        assert!(state.debug.is_some());
        let panel = state.debug.as_ref().unwrap();
        assert_eq!(panel.disasm_addr, 0x1234);
        assert_eq!(state.focus, Focus::Map);
    }

    #[test]
    fn toggle_debug_closes_panel_when_already_open() {
        let mut state = AppState::default();
        let mut engine = MockEngine { has_debugger: true, aux: BTreeMap::new() };
        toggle_debug(&mut state, &mut engine);
        assert!(state.debug.is_some());
        toggle_debug(&mut state, &mut engine);
        assert!(state.debug.is_none());
        assert_eq!(state.focus, Focus::Game);
    }

    #[test]
    fn toggle_debug_reports_when_engine_has_no_debugger() {
        let mut state = AppState::default();
        let mut engine = MockEngine { has_debugger: false, aux: BTreeMap::new() };
        toggle_debug(&mut state, &mut engine);
        assert!(state.debug.is_none());
        assert!(state.transcript.iter().any(|l| l.contains("debugger not available")));
    }

    /// Drive `dispatch_slash_outcome` for a quit-like outcome against a minimal
    /// environment (the `QuitToLibrary`/`Quit` paths touch only `state`), and
    /// return the "should break" signal. (SQ-0435)
    fn dispatch_quit_like(state: &mut AppState, outcome: SlashOutcome) -> bool {
        let mut mapper = Mapper::default();
        let mut engine = MockEngine { has_debugger: false, aux: BTreeMap::new() };
        let mut style_watcher: Option<app::watch::StyleWatcher> = None;
        let dir = std::path::Path::new("/tmp/lanthorn-sq0435-test");
        dispatch_slash_outcome(
            outcome, state, &mut mapper, &mut engine, &mut style_watcher,
            dir, "IFIDTEST", dir, &[], dir,
            Rect::default(), Rect::default(), false,
        )
    }

    /// Drive `dispatch_slash_outcome` against a real per-game directory — the
    /// sidecar writers need one — for outcomes that touch only `state` + `game_dir`.
    fn dispatch_in(state: &mut AppState, outcome: SlashOutcome, dir: &std::path::Path) {
        let mut mapper = Mapper::default();
        let mut engine = MockEngine { has_debugger: false, aux: BTreeMap::new() };
        let mut style_watcher: Option<app::watch::StyleWatcher> = None;
        dispatch_slash_outcome(
            outcome, state, &mut mapper, &mut engine, &mut style_watcher,
            dir, "IFIDTEST", dir, &[], dir,
            Rect::default(), Rect::default(), false,
        );
    }

    /// SQ-0945: the runtime switch for SQ-0936's magnification ladder. On/off/toggle
    /// all land in THIS game's `config.toml` sidecar and take effect on the next
    /// frame; `auto` clears the key (absent = inherit) and puts the live value back
    /// to the global default boot captured. The pin is what keeps a per-game answer
    /// out of the user's global config — see the config-side case for the bleed.
    #[test]
    fn set_v6_pixel_lock_persists_per_game_and_auto_falls_back_to_the_global() {
        use app::config::keys;
        use app::slash::V6PixelLockArg;
        let dir = std::env::temp_dir().join(format!("bm-sq0945-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut state = AppState::default();
        // The global says off, and that is what boot recorded as the base.
        state.config.v6_pixel_lock = false;
        state.v6_pixel_lock_base = false;

        dispatch_in(&mut state, SlashOutcome::SetV6PixelLock(V6PixelLockArg::On), &dir);
        assert!(state.config.v6_pixel_lock, "on applies live");
        assert_eq!(app::styles::read_per_game_v6_pixel_lock(&dir), Some(true), "and is written down");
        assert!(state.config.one_run.holds(keys::V6_PIXEL_LOCK), "a per-game value is pinned");

        // Bare toggles whatever is in force, and persists the result.
        dispatch_in(&mut state, SlashOutcome::SetV6PixelLock(V6PixelLockArg::Toggle), &dir);
        assert!(!state.config.v6_pixel_lock, "toggle flips it");
        assert_eq!(
            app::styles::read_per_game_v6_pixel_lock(&dir), Some(false),
            "an explicit off is a choice, not an absence — it has to be written"
        );

        // `auto` clears the override and falls back to the global base.
        state.v6_pixel_lock_base = true;
        dispatch_in(&mut state, SlashOutcome::SetV6PixelLock(V6PixelLockArg::Auto), &dir);
        assert_eq!(app::styles::read_per_game_v6_pixel_lock(&dir), None, "the key is gone");
        assert!(state.config.v6_pixel_lock, "and the live value is the global one again");
        assert!(!state.config.one_run.holds(keys::V6_PIXEL_LOCK), "nothing overrides it now");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-1123: `set-guidance` was session-only and now sticks to the game.
    ///
    /// Whether you want help is a standing preference about the story in front of
    /// you — off for the one you know by heart, on for the one you just opened —
    /// so it belongs beside the pixel lock in the per-game sidecar. The settings
    /// screen still owns the global default new games inherit, which is what the
    /// pin protects.
    #[test]
    fn set_guidance_persists_per_game_and_auto_falls_back_to_the_global() {
        use app::config::keys;
        use app::slash::GuidanceArg;
        let dir = std::env::temp_dir().join(format!("bm-sq1123-g-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut state = AppState::default();
        state.config.guidance = true;
        state.guidance_base = true;

        dispatch_in(&mut state, SlashOutcome::SetGuidance(GuidanceArg::Off), &dir);
        assert!(!state.config.guidance, "off applies live");
        assert_eq!(app::styles::read_per_game_guidance(&dir), Some(false), "and is written down");
        assert!(state.config.one_run.holds(keys::GUIDANCE), "a per-game value is pinned");

        // A bare toggle is what the border control sends, and it persists too.
        dispatch_in(&mut state, SlashOutcome::SetGuidance(GuidanceArg::Toggle), &dir);
        assert!(state.config.guidance);
        assert_eq!(app::styles::read_per_game_guidance(&dir), Some(true));

        // `auto` is the way back to the global default — the one thing a button
        // that only ever writes a concrete value cannot reach.
        state.guidance_base = false;
        dispatch_in(&mut state, SlashOutcome::SetGuidance(GuidanceArg::Auto), &dir);
        assert_eq!(app::styles::read_per_game_guidance(&dir), None, "the key is gone");
        assert!(!state.config.guidance, "and the live value is the global one again");
        assert!(!state.config.one_run.holds(keys::GUIDANCE));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-1123: `set-v6-render` was session-only, and that was right for what it
    /// then was — raster began as a FALLBACK, and an escape hatch should not
    /// outlive the session. Raster is a destination now, with `extended` beside
    /// it, so the mode sticks to the game it was chosen for.
    ///
    /// The bare CYCLE walks the three concrete modes and never visits `auto`:
    /// "inherit" has no look of its own to show, so it is the command's argument
    /// rather than a fourth step nobody could tell from the third.
    #[test]
    fn set_v6_render_persists_per_game_and_cycles_without_visiting_auto() {
        use app::config::{keys, V6RenderMode};
        use app::slash::V6RenderArg;
        let dir = std::env::temp_dir().join(format!("bm-sq1123-r-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut state = AppState::default();
        state.config.v6_render = V6RenderMode::Hybrid;
        state.v6_render_base = V6RenderMode::Hybrid;

        // Three bare cycles return to where they started, writing every step.
        for want in [V6RenderMode::Raster, V6RenderMode::Extended, V6RenderMode::Hybrid] {
            dispatch_in(&mut state, SlashOutcome::SetV6Render(V6RenderArg::Cycle), &dir);
            assert_eq!(state.config.v6_render, want, "the cycle steps to {want:?}");
            assert_eq!(
                app::styles::read_per_game_v6_render(&dir).as_deref(),
                Some(app::config::v6_render_key(want)),
                "…and writes it down, so it survives the session",
            );
            assert!(state.config.one_run.holds(keys::V6_RENDER), "pinned out of the global file");
        }

        // Naming a mode is the same thing without the walk.
        dispatch_in(
            &mut state,
            SlashOutcome::SetV6Render(V6RenderArg::Mode(V6RenderMode::Raster)),
            &dir,
        );
        assert_eq!(state.config.v6_render, V6RenderMode::Raster);

        // …and `auto` is how a game gets back to inheriting.
        state.v6_render_base = V6RenderMode::Extended;
        dispatch_in(&mut state, SlashOutcome::SetV6Render(V6RenderArg::Auto), &dir);
        assert_eq!(app::styles::read_per_game_v6_render(&dir), None, "the key is gone");
        assert_eq!(state.config.v6_render, V6RenderMode::Extended, "the global mode is back");
        assert!(!state.config.one_run.holds(keys::V6_RENDER));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quit_to_library_without_library_sets_status_and_does_not_break() {
        let mut state = AppState::default();
        state.launched_from_library = false;
        let should_break = dispatch_quit_like(&mut state, SlashOutcome::QuitToLibrary);
        assert!(!should_break, "no library → the loop must not break");
        assert_eq!(state.exit_target, ExitTarget::Exit, "target must stay Exit");
        assert!(!state.overlays.quit_dialog, "no save prompt should open");
        assert!(
            state.notifications.history().iter().any(|m| m.contains("No story library")),
            "a status explaining the missing library should be shown"
        );
    }

    #[test]
    fn quit_to_library_with_library_and_no_unsaved_progress_breaks_to_library() {
        let mut state = AppState::default();
        state.launched_from_library = true;
        // Default config: auto_save off, no unsaved progress → no save prompt.
        state.unsaved_progress = false;
        let should_break = dispatch_quit_like(&mut state, SlashOutcome::QuitToLibrary);
        assert!(should_break, "no unsaved progress → break straight to the library");
        assert_eq!(state.exit_target, ExitTarget::Library, "target must be Library");
        assert!(!state.overlays.quit_dialog, "no save prompt with no unsaved progress");
    }

    #[test]
    fn quit_to_library_with_unsaved_progress_opens_prompt_targeting_library() {
        let mut state = AppState::default();
        state.launched_from_library = true;
        // Force the save prompt: prompt_save_on_quit defaults on; add progress.
        state.config.auto_save = false;
        state.config.prompt_save_on_quit = true;
        state.unsaved_progress = true;
        let should_break = dispatch_quit_like(&mut state, SlashOutcome::QuitToLibrary);
        assert!(!should_break, "opening the save prompt does not break yet");
        assert!(state.overlays.quit_dialog, "the save prompt should open");
        assert_eq!(state.exit_target, ExitTarget::Library, "prompt resolution must target Library");
    }

    #[test]
    fn quit_sets_exit_target_to_exit() {
        let mut state = AppState::default();
        // Even after a prior quit-to-library set the target, a plain Quit resets it.
        state.launched_from_library = true;
        state.exit_target = ExitTarget::Library;
        state.unsaved_progress = false;
        let should_break = dispatch_quit_like(&mut state, SlashOutcome::Quit);
        assert!(should_break, "no unsaved progress → quit breaks immediately");
        assert_eq!(state.exit_target, ExitTarget::Exit, "quit must resolve to Exit");
    }

    // ── SQ-0648: overwrite confirmation on the slash `/save <name>` path ───────

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        app::scratch_dir(&format!("sq0648-slash-{tag}"))
    }

    /// A save-as target that already exists must open the overwrite-confirm
    /// overlay INSTEAD of writing — same rule as the dialog path, reached here
    /// through `/save <name>`. The prompt names the EXISTING save, so a
    /// cross-name slugify collision ("Before Troll" / "before, troll!" both
    /// land on `before-troll.lanthorn`) is visible rather than reading as a
    /// harmless same-name re-save.
    #[test]
    fn slash_save_named_prompts_before_overwriting_an_existing_target() {
        let dir = temp_dir("prompt");
        let mut mapper = Mapper::default();

        // Seed "Before Troll" through the very writer `/save` itself uses.
        let mut seed_engine = MockEngine { has_debugger: false, aux: BTreeMap::new() };
        let mut seed_state = AppState::default();
        super::write_named_save(&dir, "IFIDTEST", "Before Troll", &mapper, &mut seed_engine, &mut seed_state)
            .expect("seed write succeeds");
        let path = dir.join("before-troll.lanthorn");
        let original_bytes = std::fs::read(&path).expect("seed archive written");

        // `/save "before, troll!"` — a DIFFERENT typed name, same target file.
        let mut state = AppState::default();
        let mut engine = MockEngine { has_debugger: false, aux: BTreeMap::new() };
        let mut style_watcher: Option<app::watch::StyleWatcher> = None;
        let arc_file = dir.join("default.lanthorn");
        let should_break = dispatch_slash_outcome(
            SlashOutcome::Save(Some("before, troll!".to_string())),
            &mut state, &mut mapper, &mut engine, &mut style_watcher,
            &dir, "IFIDTEST", &arc_file, &[], &dir,
            Rect::default(), Rect::default(), false,
        );
        assert!(!should_break);

        // Nothing was written — byte-identical to the seed.
        let after_bytes = std::fs::read(&path).expect("archive still there");
        assert_eq!(after_bytes, original_bytes, "no write until the overwrite is confirmed");

        let pending = state.overlays.confirm_overwrite_save.as_ref().expect("confirm overlay opens");
        assert_eq!(pending.path, path);
        assert_eq!(
            pending.existing_name, "Before Troll",
            "the prompt must name the save ALREADY there, not the name just typed"
        );
        assert_eq!(pending.pending, app::state::PendingOverwrite::Slash("before, troll!".to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `write_named_save` is exactly what the run loop's
    /// `OverlayAct::ConfirmOverwrite(true)` handler calls once the player
    /// confirms — calling it directly here mirrors that resumed write and
    /// proves it actually replaces the file.
    #[test]
    fn slash_write_named_save_overwrites_when_the_overwrite_is_confirmed() {
        let dir = temp_dir("confirm");
        let mapper = Mapper::default();

        let mut seed_engine = MockEngine { has_debugger: false, aux: BTreeMap::new() };
        let mut seed_state = AppState::default();
        super::write_named_save(&dir, "IFIDTEST", "Before Troll", &mapper, &mut seed_engine, &mut seed_state)
            .expect("seed write succeeds");
        let path = dir.join("before-troll.lanthorn");
        let original_bytes = std::fs::read(&path).expect("seed archive written");

        let mut engine = MockEngine { has_debugger: false, aux: BTreeMap::new() };
        let mut state = AppState::default();
        let result = super::write_named_save(&dir, "IFIDTEST", "before, troll!", &mapper, &mut engine, &mut state);
        assert!(result.is_ok(), "{result:?}");

        let new_bytes = std::fs::read(&path).expect("archive overwritten");
        assert_ne!(new_bytes, original_bytes, "the confirmed overwrite actually wrote");
        let meta = app::archive::read_archive_meta(&path).expect("meta");
        assert_eq!(meta.name.as_deref(), Some("before, troll!"), "the file now belongs to the new name");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The default archive slot (`/save` with no name) is the quick-save
    /// equivalent, never a name the player typed — it must keep overwriting
    /// silently even when the slot already holds an earlier save, exactly like
    /// the per-turn auto-save (SQ-0648).
    #[test]
    fn slash_save_default_archive_never_prompts_even_when_it_already_exists() {
        let dir = temp_dir("default-slot");
        let arc_file = dir.join("default.lanthorn");

        let seed_meta = app::archive::Meta {
            format_version: app::archive::CURRENT_FORMAT_VERSION,
            ifid: None, name: None, turns: 1, saved_at: String::new(), location: None, score: None,
            trigger: app::archive::SaveTrigger::HostState,
        };
        app::archive::save_archive_meta(
            &arc_file, &Mapper::default(), &EngineSave::new("mock", 1, vec![1, 2, 3]), None,
            &BTreeMap::new(), seed_meta, &[], &[], &[], &[], &[], &[],
        ).expect("seed default.lanthorn");
        let before = std::fs::read(&arc_file).expect("seed archive written");

        let mut state = AppState::default();
        let mut mapper = Mapper::default();
        let mut engine = MockEngine { has_debugger: false, aux: BTreeMap::new() };
        let mut style_watcher: Option<app::watch::StyleWatcher> = None;
        let should_break = dispatch_slash_outcome(
            SlashOutcome::Save(None),
            &mut state, &mut mapper, &mut engine, &mut style_watcher,
            &dir, "IFIDTEST", &arc_file, &[], &dir,
            Rect::default(), Rect::default(), false,
        );
        assert!(!should_break);
        assert!(
            state.overlays.confirm_overwrite_save.is_none(),
            "the default archive slot must never open the overwrite-confirm overlay"
        );
        let after = std::fs::read(&arc_file).expect("archive still there");
        assert_ne!(after, before, "it wrote silently, straight over the existing slot");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── SQ-0777: the `DumpWindows` arm itself ────────────────────────────────
    //
    // Everything else about `/dump-windows` is covered from the supplier side —
    // `tests/window_dump_last_game_frame.rs` drives real frames through
    // `AppState::v6_dump_frame`, `tests/window_dump_bound_key.rs` drives the key
    // resolver — and both stop one call short of the consumer. Nothing reached
    // this arm, so SQ-0756's one-line change could be reverted without breaking
    // a test. These two go through the arm.

    use app::state::{V6CellRect, V6GameFrame};

    /// A state in exactly the situation the command runs in: the game's frame is
    /// on record, and the frame standing in `v6_cell_map` is the modal one the
    /// palette drew over it. The two share no number.
    fn dump_windows_state(dir: &std::path::Path) -> AppState {
        let mut state = AppState::default();
        state.config.user_dir = dir.to_path_buf();
        *state.v6_last_game_frame.borrow_mut() = Some(V6GameFrame {
            cells: vec![
                V6CellRect { label: "path:hybrid-ring".into(), native: (0, 0, 0, 0), cells: (0, 0, 40, 25) },
                V6CellRect { label: "pane".into(), native: (0, 0, 0, 0), cells: (0, 0, 40, 25) },
                V6CellRect { label: "viewport".into(), native: (43, 39, 234, 160), cells: (5, 4, 29, 20) },
            ],
            ring_plan: "tall",
            ring_clip: Some((120, 15)),
            modal_frames_since: 2,
        });
        *state.v6_cell_map.borrow_mut() = vec![
            V6CellRect {
                label: "path:cell — modal overlay open: palette".into(),
                native: (0, 0, 0, 0),
                cells: (0, 0, 30, 20),
            },
            V6CellRect { label: "pane".into(), native: (0, 0, 0, 0), cells: (0, 0, 30, 20) },
        ];
        state
    }

    fn dispatch_dump_windows(state: &mut AppState, dir: &std::path::Path) {
        let mut mapper = Mapper::default();
        let mut engine = MockEngine { has_debugger: false, aux: BTreeMap::new() };
        let mut style_watcher: Option<app::watch::StyleWatcher> = None;
        let should_break = dispatch_slash_outcome(
            SlashOutcome::DumpWindows,
            state, &mut mapper, &mut engine, &mut style_watcher,
            dir, "IFIDTEST", dir, &[], dir,
            Rect::default(), Rect::default(), true,
        );
        assert!(!should_break, "a diagnostic dump never breaks the run loop");
    }

    /// SQ-0756, through the arm: the lines the command emits describe the GAME's
    /// frame — the snapshot's ring plan and clip, under a header saying which
    /// frame it is and how stale — and they name the log file, because the
    /// on-screen copy is the one thing a v6 pane cannot hand back.
    ///
    /// FALSIFY by restoring the pre-fix source in the `DumpWindows` arm —
    /// `let cells = state.v6_cell_map.borrow().clone();` with the ring read from
    /// the live `state.v6_ring_plan`/`v6_ring_clip` and no `frame_line` pushed.
    /// The `frame described:` line disappears entirely and the ring line reports
    /// the live cells' default plan instead of the recorded frame's.
    #[test]
    fn dump_windows_reports_the_game_frame_and_names_its_log() {
        let dir = temp_dir("dumpwin-arm");
        let mut state = dump_windows_state(&dir);
        dispatch_dump_windows(&mut state, &dir);

        let text = state.transcript.join("\n");
        assert!(
            text.contains("frame described: the last frame the game drew"),
            "the dump says which frame it describes: {text}"
        );
        assert!(
            text.contains("2 modal frame(s) ago"),
            "…and how stale it is, so the modal frames are disclaimed: {text}"
        );
        assert!(
            text.contains("ring: plan tall, ring clipped at row 15 (art opaque down to native y=120)"),
            "the ring plan and clip are the recorded GAME frame's: {text}"
        );
        assert!(
            !text.contains("modal overlay open"),
            "nothing from the modal frame standing in v6_cell_map may reach the dump: {text}"
        );

        // The log path is surfaced, and the file it names really holds the dump.
        let log = app::export::window_dump_path(&dir);
        assert!(
            text.contains(&format!("dump appended to {}", crate::abbreviate_home(&log))),
            "the transcript names the copyable log: {text}"
        );
        let written = std::fs::read_to_string(&log).expect("the log the transcript names exists");
        assert!(written.contains("ring: plan tall"), "and carries the same dump: {written}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-0759, through the arm: taking the dump must not churn the very state
    /// it is diagnosing. The band-upload count and the render-path history are
    /// snapshotted either side of the dispatch and must be untouched — a dump
    /// that invalidated the band cache or filed a render path would be
    /// measuring its own footprint.
    ///
    /// FALSIFY by making the arm touch either: a `state.note_v6_path("dump")`
    /// or a `state.graphics_render.borrow_mut().band_encodes += 1` anywhere in
    /// it fails one of these two assertions.
    #[test]
    fn dump_windows_moves_neither_the_band_count_nor_the_path_history() {
        let dir = temp_dir("dumpwin-quiet");
        let mut state = dump_windows_state(&dir);
        // Seed both counters so "unchanged" is a real value, not zero-vs-zero.
        state.graphics_render.borrow_mut().band_encodes = 7;
        state.note_v6_path("hybrid-ring");
        state.note_v6_path("hybrid-ring");

        let encodes_before = state.graphics_render.borrow().band_encodes;
        let paths_before = state.v6_path_log.borrow().clone();
        assert_eq!(encodes_before, 7, "precondition: some bands have been uploaded");
        assert_eq!(paths_before, vec![("hybrid-ring".to_string(), 2)], "precondition: a history exists");

        dispatch_dump_windows(&mut state, &dir);

        assert_eq!(
            state.graphics_render.borrow().band_encodes, encodes_before,
            "taking a dump must upload no bands"
        );
        assert_eq!(
            *state.v6_path_log.borrow(), paths_before,
            "taking a dump must add no render path to the history"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── SQ-0994: the `DumpTerminal` arm ─────────────────────────────────────
    //
    // The report itself is pinned by `terminal_dump`'s own unit tests, which
    // need no terminal at all. What is only testable HERE is the arm: that the
    // snapshot it gathers actually reflects the state handed to it, and that
    // the file half of "here and to ~/.lanthorn/dump-terminal.log" is real.

    fn dispatch_dump_terminal(state: &mut AppState, dir: &std::path::Path, pane: Rect) {
        let mut mapper = Mapper::default();
        let mut engine = MockEngine { has_debugger: false, aux: BTreeMap::new() };
        let mut style_watcher: Option<app::watch::StyleWatcher> = None;
        let should_break = dispatch_slash_outcome(
            SlashOutcome::DumpTerminal,
            state, &mut mapper, &mut engine, &mut style_watcher,
            dir, "IFIDTEST", dir, &[], dir,
            Rect::default(), pane, true,
        );
        assert!(!should_break, "a diagnostic dump never breaks the run loop");
    }

    /// The command's own promise: the same report on screen AND in the log it
    /// names. `/dump-windows` writes a file because a v6 pane's placeholder
    /// glyphs make the on-screen copy unpastable (SQ-0756); this one writes it
    /// because the capability list and the byte counts are precisely what goes
    /// into a bug report, and a file is easier to attach than scrollback.
    ///
    /// FALSIFY by dropping the `append_terminal_dump` call: the transcript loses
    /// the path line and `dump-terminal.log` never appears.
    #[test]
    fn dump_terminal_reports_the_live_picker_and_names_its_log() {
        let dir = temp_dir("dumpterm-arm");
        let log = app::export::terminal_dump_path(&dir);
        let _ = std::fs::remove_file(&log);

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        // A picker that was never asked anything — `--image-protocol halfblocks`
        // builds exactly this, and its empty capability list is the "nobody
        // asked" kind of empty rather than the "the terminal said no" kind.
        state.config.image_protocol = app::config::ImageProtocol::Halfblocks;
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        let traffic: app::terminal_dump::TrafficHandle = Default::default();
        state.term_traffic = Some(std::sync::Arc::clone(&traffic));

        dispatch_dump_terminal(&mut state, &dir, Rect::new(0, 0, 115, 61));

        let text = state.transcript.join("\n");
        assert!(text.contains("graphics protocol: halfblocks"), "the live picker's protocol: {text}");
        assert!(
            text.contains("FORCED by --image-protocol halfblocks"),
            "a forced protocol is not a detected one: {text}"
        );
        assert!(
            text.contains("capabilities: NOT ASKED"),
            "an empty list under a forced half-blocks picker is the not-asked kind: {text}"
        );
        assert!(text.contains("115x61 cells = 7,015"), "the story pane it was handed: {text}");

        // The log path is surfaced, and the file it names really holds the report.
        assert!(
            text.contains(&format!("report appended to {}", crate::abbreviate_home(&log))),
            "the transcript names the copyable log: {text}"
        );
        let written = std::fs::read_to_string(&log).expect("the log the transcript names exists");
        assert!(written.contains("=== /dump-terminal "), "stamped like every other dump: {written}");
        assert!(written.contains("graphics protocol: halfblocks"), "and carries the report: {written}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The counters are the writer's, not a fresh sample — a report that read
    /// zero while the session had emitted megabytes would be worse than no
    /// report. And with no writer at all the arm must say so rather than print a
    /// zero, which is the same measured-versus-assumed rule the whole command is
    /// built on.
    #[test]
    fn dump_terminal_reads_the_writer_counters_and_disclaims_their_absence() {
        let dir = temp_dir("dumpterm-traffic");
        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());

        let traffic: app::terminal_dump::TrafficHandle = Default::default();
        {
            use std::io::Write as _;
            let mut w = app::terminal_dump::CountingWriter::new(Vec::new(), std::sync::Arc::clone(&traffic));
            w.write_all(&vec![b'x'; 4096]).unwrap();
            w.flush().unwrap();
        }
        state.term_traffic = Some(std::sync::Arc::clone(&traffic));
        dispatch_dump_terminal(&mut state, &dir, Rect::new(0, 0, 80, 24));
        let text = state.transcript.join("\n");
        assert!(text.contains("4,096 in 1 frame flush(es)"), "the writer's own totals: {text}");
        assert!(text.contains("last drawn frame: 4,096 bytes"), "{text}");

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.term_traffic = None;
        dispatch_dump_terminal(&mut state, &dir, Rect::new(0, 0, 80, 24));
        let text = state.transcript.join("\n");
        assert!(
            text.contains("bytes written: unavailable"),
            "no writer means no counts, and saying so beats printing a zero: {text}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
