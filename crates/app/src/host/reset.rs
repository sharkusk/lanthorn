//! Game restart/reset: rebuild the engine from the original story bytes via the
//! same factory used at startup, optionally clearing the accumulated map and/or
//! deleting the game's AUTO persistent data. Extracted verbatim from `main.rs`
//! (SQ-0306) as a pure move — no behavior change — and into the library by
//! SQ-1539, so a host that is not a terminal restarts a story by the same rules
//! the TUI does. The one fact it asked the terminal for, the host pane a v1–v8
//! Z-machine story re-boots with (SQ-1061), is now the host's `terminal_size`.

use crate::engine::Engine;
use crate::glulx_session::GlulxSession;
use crate::hints;
use crate::session::{apply_turn, GameSession, TurnResult};
use crate::state::AppState;
use mapper::mapper::Mapper;

use crate::engine_helpers::zvm_session_mut;
use super::resolve_pict_blorb;

/// What a reset clears beyond the running game itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResetOptions {
    /// Wipe the accumulated map (as `/reset map` does), so only the start room
    /// remains after the re-seed.
    pub clear_map: bool,
    /// Delete the game's AUTO persistent data first (the Glk file VFS and the
    /// other per-story sidecars), so the fresh boot re-initialises from nothing.
    pub delete_data: bool,
}

/// Restart the story: rebuild the engine from the original story bytes the way
/// the launch built it, and reset the session state around it.
///
/// `terminal_size` is the host's `(cols, rows)` — the frame the story pane is
/// laid out in, exactly as [`BootRequest`](super::BootRequest)'s
/// `TerminalFacts::size` is at launch — or `None` for the constructor's 80x24
/// fallback.
#[allow(clippy::too_many_arguments)]
pub fn reset_game(
    session: &mut dyn Engine,
    mapper: &mut Mapper,
    state: &mut AppState,
    story_bytes: &[u8],
    story_path: &std::path::Path,
    game_dir: &std::path::Path,
    terminal_size: Option<(u16, u16)>,
    options: ResetOptions,
) {
    let ResetOptions { clear_map, delete_data } = options;
    // Delete the game's AUTO persistent data BEFORE rebuilding so the fresh boot
    // re-initializes: the on-disk sidecars go now, and the in-memory VFS carried
    // into the Glulx rebuild is suppressed below (an empty carry_vfs).
    if delete_data {
        crate::storage::delete_auto_persistent(game_dir);
    }
    // Rebuild the engine from the original story bytes via the same factory used
    // at startup: classify the executable, then replace the concrete session in
    // place (restart re-runs the SAME story, so the engine type is unchanged).
    let rebuilt: Result<(), String> = match hints::extract_story(story_bytes.to_vec()) {
        Ok(crate::hints::LoadedStory::ZCode(bytes)) => {
            // Rebuild through the SAME construction startup uses, not the bare
            // `GameSession::new` (SQ-0546). A v6 game needs all of it before and
            // after its boot run: the Pict dimension table (`picture_data` is
            // called DURING boot, inside the constructor), the Blorb `Reso`
            // standard window that sizes the 640×400 unit screen its windows and
            // hardcoded art align to, the host default colour pair, then the Pict
            // source and a boot-picture flush to drain the art the boot drew
            // before that source existed. Restarting without them left Shogun
            // with a mis-sized status band and no inline graphics at all.
            // SQ-0734: a restart re-resolves all three tiers, so the archive the
            // per-game sidecar names is still the one in force afterwards. The
            // profile is NOT re-derived from it — `state.config.interpreter_profile`
            // below is the one boot settled, and a restart re-runs the same story
            // on the same machine.
            // SQ-0789/0791: `pictures_override` carries a choice made for THIS
            // launch and never written down (`--pictures`, or the launch dialog
            // with its checkbox left clear). Without it a restart would re-read
            // only the sidecar and silently swap the art back to the Blorb.
            let over = if state.config.images {
                crate::graphics::PictureOverride::resolve_with_session(
                    story_path,
                    &state.game_dir,
                    state.config.pictures_override.as_deref(),
                )
            } else {
                crate::graphics::PictureOverride::Unset
            };
            let named_art_std_window = over.std_window();
            let mut picts = if state.config.images {
                // The story's own entry on the medium, carried by the config for
                // exactly this moment (SQ-0876).
                crate::graphics::PictSource::resolve_with_override(
                    story_path,
                    over,
                    state.config.disk_entry.clone().as_deref(),
                )
            } else {
                crate::graphics::PictSource::new(None)
            };
            // SQ-0719: a restart re-runs the same story on the same machine, so
            // the interpreter profile resolved at boot supplies the same three
            // answers it did there — the standard window a native Amiga archive
            // has no chunk to declare, the machine's own default colours, and its
            // interpreter number. IBM PC (every Blorb-sourced story) supplies
            // none of them and this is the prior code exactly.
            let profile = state.config.interpreter_profile;
            // SQ-0816: and the same dither preference the launch resolved, so a
            // restart does not quietly change what the artwork looks like.
            picts.set_fuse_dither(state.config.fuse_art_dither);
            let picture_dims = picts.all_pict_dims();
            // The same four links `startup.rs` resolves, in the same order, so a
            // restart comes back on the screen the launch settled — including
            // the archive's own picture space, which is the standard Macintosh's
            // 480×300 when the mono archive is the one mounted (SQ-0838).
            // SQ-0790: and the density the art arrives at, so a restart of a
            // story playing its EGA rendition comes back with the same geometry
            // it booted with. `None` for every Blorb-sourced story.
            let v6_art_scale = picts.art_scale();
            // SQ-0936: republish it for the render's magnification ladder, since a
            // restart may have re-resolved a DIFFERENT archive (the sidecar, or a
            // `--pictures` choice) and with it a different art density.
            if let Some(scale) = v6_art_scale {
                state.v6_art_scale = scale;
            }
            // SQ-0956: the CARD's pair when the archive this restart resolved is a
            // two-colour one, exactly as `startup.rs` resolves it — a restart may
            // have landed on a different rendition (the sidecar, or a `--pictures`
            // choice), so this is asked again rather than carried.
            let card = if state.config.honor_game_colours {
                picts.two_colour_card_screen(&state.config)
            } else {
                None
            };
            // SQ-1393: the machine's own colour table, re-derived here in the same
            // two steps `startup.rs` uses — the story's Version and this launch's
            // licence name a base table, and an archive that turns out to be a
            // two-colour CARD names its own over the top. Re-asked rather than
            // carried, for the reason the rest of this block is: a restart may
            // have landed on a different rendition, and the card is a fact about
            // the archive it mounted.
            let machine_palette = card
                .map(|(p, _)| p)
                .unwrap_or_else(|| state.config.machine_text_palette(bytes.first().copied()));
            let card = card.map(|(_, pair)| pair);
            // SQ-1082: and the chain the card heads is `colors::host_default_colours`
            // now, shared with `startup.rs` rather than copied here. This copy is
            // exactly the hand-maintained invariant across files the refactoring
            // policy names: `--colour` would have had to be added to both.
            let host_default_colours = crate::colors::host_default_colours(
                &state.config,
                card.or_else(|| state.config.machine_default_colours()),
                state.colors.theme.get("transcript").style,
                state.term_default_colors.fg.map(|c| (c.0[0], c.0[1], c.0[2])),
                state.term_default_colors.bg.map(|c| (c.0[0], c.0[1], c.0[2])),
                machine_palette,
            );
            // SQ-1022: every per-machine fact in one value, resolved the way
            // `startup.rs` resolves it rather than reproduced here. It HAD drifted
            // — this call passed `None` for the Version 6 cell, so restarting a
            // Macintosh game re-booted it on 8x16 where the launch gave it 7x15.
            // The comment above promised "the same four links" and by then there
            // were five facts, which is exactly how a recipe fails.
            // SQ-1009: and the release's own typeface, re-resolved off the medium
            // exactly as `startup.rs` resolves it. It is a link in the chain now
            // rather than a render detail, because the DECLARED cell follows the
            // face — omit it and an `@restart` of Arthur's Amiga floppy re-boots
            // the story on 8x16 where its launch gave it 8x20, which is SQ-1022's
            // defect with a different fact in the hole.
            // SQ-1037: the same cascade, including the system rung — a restart that
            // re-read only the release's medium would drop a Macintosh game's Geneva
            // and re-boot it in Monaco, which is SQ-1022's defect with a different
            // fact in the hole.
            let user_disks =
                crate::system_fonts::UserDisks::new(&state.config.system_font_disk);
            let faces = crate::native_font::resolve(&crate::native_font::FaceRequest {
                story_path,
                entry: state.config.disk_entry.as_deref(),
                profile,
                source: state.config.interpreter_source,
                art_scale: picts.art_scale(),
                disks: Some(&user_disks),
            });
            // **The number is `Config`'s cascade, not a second copy of it**
            // (SQ-1058). This read `interpreter_number.or_else(|| profile
            // .interpreter_number())` — rungs 1 and 3 of
            // `Config::advertised_interpreter_number`, with rung 2 missing.
            // That rung is SQ-0930's whole point: a DOS MEDIUM names the IBM PC,
            // whose own `interpreter_number()` is deliberately `None` because the
            // honest answer is version-dependent. Without it a restart fell
            // through to zvm's default rule — Frotz's, 6 for Version 6 and 1
            // otherwise — so a v5 story off `floppy1.ima` advertised `$1E = 6` at
            // launch and `$1E = 1` after `@restart`, and *Beyond Zork* silently
            // swapped its CP437 box graphics back to Font 3 arrows mid-session.
            // Version 6 masked it, because both roads reach 6.
            let boot = crate::machine_boot::MachineBoot::resolve(
                profile,
                &picts,
                named_art_std_window,
                state.config.advertised_interpreter_number(),
                host_default_colours,
                // SQ-1154: re-asked, not carried — `--colour` is a flag of this
                // run and `@restart` re-boots under the same one. This is the site
                // the required parameter exists to enumerate.
                state.config.machine_colours_licensed(),
                faces,
                // SQ-1393: and the two facts the process-wide statics used to
                // carry. `interpreter_version` is a flag of THIS run
                // (`--interpreter-version`), so it is the one the launch pinned.
                machine_palette,
                state.config.interpreter_version,
            );
            // Republish the render's copy for the same reason `v6_art_scale` is
            // republished above: a restart may have landed on a different archive,
            // and the pen's scale rides on that.
            state.v6_text = boot.text_face();
            // Republish the renderer's copy too (SQ-1393): the scheme resolves
            // Standard colour numbers through this table, and a restart that
            // re-resolved a two-colour card must not leave the cells reading the
            // launch's.
            state.colors.machine_palette = boot.palette;
            GameSession::new_for_machine(
                bytes,
                state.config.honor_game_colours,
                state.config.enable_sound,
                // Keep boot tracing across a restart in a `--debug` session, as
                // the Glulx arm below does.
                state.persist_debug_trace,
                picture_dims,
                // The host pane, as the launch seeds it (SQ-1061). This was a
                // bare `None` — the only argument in this call with no comment
                // above it — so the constructor took neither `set_screen_dims`
                // nor the `boot_screen_cols` branch, and a v3/v4/v5 story whose
                // status routine lays itself out once at boot (Zork 1, SQ-0680)
                // came back laid out for zvm's 80x24 fallback. Nothing re-seeds
                // it afterwards: `loop_tick::poll_zvm_screen_dims` runs
                // post-boot, which is exactly what SQ-0680 established is too
                // late. The v6 arm never saw it, because that arm takes its
                // screen from `boot.screen_px`.
                terminal_size.and_then(|size| super::story_screen_in(state, size)),
                // A restart re-draws the seed the same way the launch did
                // (SQ-0811): a pinned `random_seed` replays the same game, and an
                // unpinned one deals a fresh one — which is what restarting a
                // randomised game is FOR.
                Some(state.config.effective_random_seed()),
                &boot,
            )
            .map_err(|e| format!("{e:?}"))
            .map(|mut new_session| {
                new_session.machine.undo_cap = state.config.undo_levels;
                new_session.set_pict_source(Some(picts));
                new_session.flush_boot_pictures();
                *zvm_session_mut(session) = new_session;
            })
        }
        Ok(crate::hints::LoadedStory::Glulx(bytes)) => {
            // Restart re-resolves the Pict Blorb the same path-based way as launch
            // (self-contained blorb, same-stem sidecar, or dir scan), and reuses
            // the stored game Picker for char-cell size, so graphics come back
            // enabled per config.images — matching the initial launch even for a
            // bare .ulx with a sidecar .blorb.
            let char_px = state
                .game_picker
                .as_ref()
                .map(|p| {
                    let f = p.font_size();
                    (f.width as u32, f.height as u32)
                })
                .unwrap_or((8, 16));
            let pict_blorb = resolve_pict_blorb(story_path, state.config.images);
            // Carry the current in-memory Glk file VFS (e.g. CM's boot cache,
            // kept in sync with the sidecar) into the restarted session so the
            // fresh boot still sees it (SQ-0290). When delete_data is set, carry
            // an EMPTY VFS instead so the game boots with no cache and re-runs its
            // full initialization (deleting default.glkvfs on disk is not enough —
            // the cache also lives in memory and would otherwise be carried over).
            let carry_vfs = if delete_data { Vec::new() } else { session.vfs_bytes() };
            // Preserve the per-game borderless-windows override across @restart
            // (SQ-0341); an explicit per-game value wins over a garglk.ini
            // `wborder`, else garglk's, else off (SQ-0344).
            let borderless = crate::styles::read_per_game_borderless(game_dir)
                .or_else(|| state.garglk_overlay.as_ref().and_then(|o| o.borderless))
                .unwrap_or(false);
            GlulxSession::new_in(
                game_dir.to_path_buf(),
                bytes,
                state.config.virtual_screen_cols.unwrap_or(crate::config::FALLBACK_SCREEN_COLS) as u32,
                state.config.virtual_screen_rows.unwrap_or(crate::config::FALLBACK_SCREEN_ROWS) as u32,
                state.config.acceleration,
                state.config.images,
                state.config.enable_sound,
                borderless,
                char_px,
                pict_blorb,
                &carry_vfs,
                // The live theme's rendered colours, in place before the fresh
                // boot probes glk_style_measure (SQ-0315).
                crate::glk_backend::theme_style_colours(&state.colors),
                // Keep the debug inspector's boot-tracing across @restart when a
                // `--debug` session is active, so the restarted boot is captured too.
                state.persist_debug_trace,
                // Re-seeded exactly as the launch was (SQ-0811) — see the zvm arm.
                Some(state.config.effective_random_seed()),
            )
            .map_err(|e| format!("{e:?}"))
            .map(|new_session| {
                *session
                    .as_any_mut()
                    .downcast_mut::<GlulxSession>()
                    .expect("restart re-runs the same Glulx story") = new_session;
            })
        }
        Ok(crate::hints::LoadedStory::Scott(bytes)) => {
            // The four picture facts, resolved the one way both a launch and
            // an `@restart` resolve them (SQ-1485, `startup.rs`'s Scott arm
            // is the other caller). `story_bytes` above is the DATABASE, not
            // the container, so a restart cannot recover a S.A.G.A. release's
            // picture files from it — `resolve` re-reads them off the same
            // container the launch opened (the release disk, family C §8.3;
            // the Apple II release's companion side, family D, SQ-1476; or
            // the MS-DOS release's zip, family E §8.5, SQ-1477) rather than
            // carrying them in app state for the life of a session to save
            // one re-read of a 175 KB floppy.
            let pictures = crate::graphics::ScottPictureSources::resolve(
                story_path,
                &bytes,
                game_dir,
                resolve_pict_blorb(story_path, state.config.images),
                state.game_picker.as_ref(),
                state.config.scott_picture_resolution_override,
            );
            crate::scott_session::ScottSession::new_with_options(
                bytes,
                false,
                // Re-seeded exactly as the launch was (SQ-0811) — see the zvm arm.
                Some(state.config.effective_random_seed()),
                // Re-resolved exactly as the launch was (SQ-1413).
                crate::scott_session::resolve_options(game_dir),
                pictures,
            )
            .map(|new_session| {
                *session
                    .as_any_mut()
                    .downcast_mut::<crate::scott_session::ScottSession>()
                    .expect("restart re-runs the same Scott story") = new_session;
            })
        }
        Err(e) => Err(format!("{e}")),
    };
    match rebuilt {
        Ok(()) => {
            // The rebuilt session defaults strip_prompt=true; re-apply the config
            // choice so an in-game restart keeps the inline prompt in inline mode.
            session.set_strip_prompt(state.config.command_bar);
            // The rebuilt session carries a fresh sink, which knows nothing of
            // the game's directory: re-name the Z-machine stream files exactly
            // as `startup.rs` does, or a transcript restarted after a
            // `reset-game` would have nowhere to go. (The `@restart` OPCODE
            // needs no equivalent — it re-boots the machine in place and keeps
            // its sink.)
            session.set_stream_files(game_dir);
            let start_loc = session.current_location();
            state.reset_sound_sidecars();
            // A restart is a new game: the death the old one left unresolved died with it, and so
            // did the `tried` record a fatal move there might still owe. Carried across, an
            // outstanding death would swallow the first room change of the fresh game — which is
            // the seed below, or the first passage the player walks. (SQ-0671, SQ-0673)
            state.death_watch = crate::session::DeathWatch::default();
            state.turns = 0;
            state.unsaved_progress = false; // restart: fresh game, nothing to save
            state.vm_halted = false;
            state.game_ended = false;
            state.input.clear();
            state.suggestions.clear();
            state.suggestion_idx = 0;
            state.suggestion_active = false;
            state.transcript.clear();
            state.clear_anchor = None;
            state.transcript_kinds.clear();
            state.transcript_runs.clear();
            state.transcript_para.clear();
            state.transcript_scroll = 0;
            // Wipe any pager state the OLD game left behind — mid-[more]
            // catch-up (`active`), a stale pending arm, or a row-total baseline
            // describing a transcript that no longer exists — before the fresh
            // banner below is measured (SQ-1575). NOT `reset_transcript_
            // sidecars`'s `baseline_stale` flag: that tells `pager::apply_frame`
            // to CALIBRATE instead of measure on the next surfaced frame, which
            // is right for a restore/history-jump landing on scrollback the
            // player already read, and wrong here — a fresh banner is unread and
            // must still be measured so it can page.
            state.pager = crate::pager::Pager::default();
            state.last_transcript_total_rows = 0;
            if clear_map {
                *mapper = Mapper::default();
            }
            // Glulx returns ordered elements (text + any startup images); the
            // Z-machine returns empty and uses the flat string path.
            let banner_elems = session.take_transcript_elems();
            if banner_elems.is_empty() {
                let banner = session.take_transcript();
                state.push_transcript(&banner);
            } else {
                crate::state::apply_transcript_elems(state, &banner_elems);
            }
            // [more] pager for the restarted banner, exactly as a fresh boot arms
            // it — see `arm_opening_banner` (SQ-1575). Unlike boot's resumed-
            // transcript case, a restart has no "already read" scrollback to
            // skip: the fresh banner just landed above and is always unread.
            crate::pager::arm_opening_banner(state, &*session);
            if let Some(snap) = start_loc {
                let snap_number = snap.number;
                let seed_result = TurnResult {
                    transcript: String::new(),
                    transcript_runs: Vec::new(),
                    location: Some(snap),
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
                };
                apply_turn(mapper, "", &seed_result, &mut state.death_watch);
                let rid = snap_number as mapper::graph::RoomId;
                state.select_room(Some(rid));
            }
            // Reset cleared and/or re-seeded the mapper graph — invalidate the map
            // memo so the fresh map (not the previous game's) shows. Unconditional
            // (SQ-0305): when `clear_map` replaced `mapper` wholesale, the new
            // graph's `struct_gen` starts back at 0 (see its own doc comment) and
            // could coincidentally match whatever the OLD graph's cached render was
            // routed for, so this cannot rely on a generation-number comparison at
            // all (SQ-1544) — it drops the cache and any in-flight job outright.
            state.invalidate_map_render();
            state.push_notice("[Game reset]");
        }
        Err(e) => {
            state.push_notice(&format!("[Reset failed: {e}]"));
        }
    }
}

