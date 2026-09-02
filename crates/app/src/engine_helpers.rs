//! Engine downcast/utility helpers: the escape-hatch downcasts to the concrete
//! Z-machine / Glulx sessions behind a `dyn Engine`, plus the host restore
//! dispatch and save-support guard. Extracted verbatim from `main.rs` (SQ-0306)
//! as a pure move — no behavior change. Shared across the binary's modules
//! (main.rs, loop_tick.rs, turn.rs, startup.rs, lifecycle.rs) via `crate::`.

use app::archive::load_archive;
use app::engine::Engine;
use app::glulx_session::GlulxSession;
use app::scott_session::ScottSession;
use app::session::GameSession;

/// Escape hatch: borrow the concrete Z-machine `GameSession` behind a
/// `dyn Engine`, mutably.
///
/// Used ONLY by the persistence layer — archive save/restore and the
/// saved-screen snapshot — because the on-disk archive format serializes the
/// Z-machine `ScreenState` and cannot change without breaking compatibility
/// with existing saves (a no-behavior-change requirement). Everything else
/// (gameplay, render, input, introspection, `save_state`/`restore_state`,
/// `current_location`, aux) goes through the neutral `Engine` trait.
pub(crate) fn zvm_session_mut(engine: &mut dyn Engine) -> &mut GameSession {
    engine
        .as_any_mut()
        .downcast_mut::<GameSession>()
        .expect("lanthorn drives a Z-machine GameSession")
}

/// Non-panicking downcast to the Z-machine session: `Some` for a Z-code game,
/// `None` for Glulx. The archive-save paths use it to source the **zvm-only**
/// `screen.json` (`Some(&z.machine.screen)` for the Z-machine, `None` for Glulx —
/// whose display lives inside its `EngineSave`); the save itself routes through
/// the engine-neutral `Engine::save_state` for both engines.
pub(crate) fn zvm_session_opt(engine: &dyn Engine) -> Option<&GameSession> {
    engine.as_any().downcast_ref::<GameSession>()
}

/// Mutable non-panicking downcast to the Z-machine session: `Some` for a Z-code
/// game, `None` for Glulx. Used to reinstate the zvm-only `screen.json` after an
/// archive restore without panicking on a Glulx engine.
pub(crate) fn zvm_session_opt_mut(engine: &mut dyn Engine) -> Option<&mut GameSession> {
    engine.as_any_mut().downcast_mut::<GameSession>()
}

/// What [`v6_save_payload`] hands a save site: the fallback canvas PNGs
/// `(window, png_bytes)`, the display list + palette, the painted ground PNG, and
/// the self-check's diagnostics.
pub(crate) type V6SavePayload =
    (Vec<(u8, Vec<u8>)>, Option<app::archive::DisplayListDto>, Option<Vec<u8>>, Vec<String>);

/// Everything a host Save State needs to reproduce a v6 screen (SQ-0588): the
/// display list + Current Palette, the fallback canvas PNGs, the painted ground
/// (SQ-0787), and any diagnostics the save-time self-check raised.
///
/// The list is authoritative and the PNGs are the exception — `display_list`
/// replays each window and only asks for a PNG where the replay does not reproduce
/// the live canvas. Empty/`None` for non-v6 stories and non-Z-machine engines, so
/// the archive layout is unchanged for them.
///
/// `&mut` because the self-check replays, and replaying decodes pictures.
pub(crate) fn v6_save_payload(session: &mut dyn Engine) -> V6SavePayload {
    let Some(z) = zvm_session_opt_mut(session) else {
        return (Vec::new(), None, None, Vec::new());
    };
    if z.machine.screen.v6.is_none() {
        return (Vec::new(), None, None, Vec::new());
    }
    let (dto, fallback, diags) = z.display_list();
    let pics = z.pictures_png_for(&fallback);
    let ground = z.paint_ground_png();
    (pics, Some(dto), ground, diags)
}

/// Reinstate a restored v6 screen: rebuild from the display list when the archive
/// has one (SQ-0588), else from the canvas PNGs alone (pre-SQ-0588 archives, and
/// non-v6 stories, where both are empty). Shared by every restore site so the two
/// paths cannot drift.
pub(crate) fn apply_v6_pictures(session: &mut dyn Engine, ac: &app::archive::ArchiveContents) {
    let Some(z) = zvm_session_opt_mut(session) else { return };
    match &ac.display {
        Some(d) => z.load_display_list(d, &ac.pictures),
        None => z.load_pictures_png(&ac.pictures),
    }
    // The painted ground under all of them (SQ-0787). UNCONDITIONAL, including
    // when the archive carries none: `auto_load` restores after the story has
    // booted and painted its own opening screen, and a Save State swaps VM memory
    // under a game that never learns it happened — so nothing repaints and the
    // pre-restore ground would otherwise survive into the restored frame. scopa
    // resumed a dealt hand with its main menu still underneath.
    z.load_paint_ground(ac.ground.as_deref());
    // …and the last two layers beside the window tree (SQ-0814): the `erase_window`
    // fills and the canvas anchors. Unconditional for the same reason as the ground,
    // and it reaches the `ac.display == None` branch above too, where there is no
    // recipe to install and the reset is the whole of the fix.
    z.load_v6_screen_layers(ac.display.as_ref().map(|d| &d.layers));
}

/// The `(location, score)` save summary captured into a save's `Meta` at save time
/// (SQ-0411), shared by every Meta-building save site (auto-save, exit/quit-dialog
/// snapshots, named saves).
///
/// Location is the app-detected room name (`state.current_room_name`). Score comes
/// from the Z-machine v1–3 automatic status line; it is `None` for v4+ Z-machine and
/// Glulx games, which expose no engine-provided score.
pub(crate) fn save_summary(engine: &dyn Engine, state: &app::state::AppState) -> (Option<String>, Option<i32>) {
    use app::engine::{StatusField, StatusModel};
    let location = state.current_room_name.clone();
    let score = zvm_session_opt(engine).and_then(|z| {
        match app::session::status_model_from_machine(&z.machine) {
            StatusModel::Classic { right: StatusField::ScoreTurns { score, .. }, .. } => Some(score as i32),
            _ => None,
        }
    });
    (location, score)
}

/// Non-panicking downcast to the Glulx session: `Some` for a Glulx game, `None`
/// for Z-code. Used to read the armed Glk timer interval.
pub(crate) fn glulx_session_opt(engine: &dyn Engine) -> Option<&GlulxSession> {
    engine.as_any().downcast_ref::<GlulxSession>()
}

/// Mutable non-panicking downcast to the Glulx session: `Some` for a Glulx
/// game, `None` for Z-code. Used to deliver Glk sound-notify events.
pub(crate) fn glulx_session_opt_mut(engine: &mut dyn Engine) -> Option<&mut GlulxSession> {
    engine.as_any_mut().downcast_mut::<GlulxSession>()
}

/// Non-panicking downcast to the Scott Adams session: `Some` for a Scott
/// Adams game, `None` otherwise.
#[allow(dead_code)] // not yet called by any gameplay path (later Scott task)
pub(crate) fn scott_session_opt(engine: &dyn Engine) -> Option<&ScottSession> {
    engine.as_any().downcast_ref::<ScottSession>()
}

/// Mutable non-panicking downcast to the Scott Adams session: `Some` for a
/// Scott Adams game, `None` otherwise.
#[allow(dead_code)] // not yet called by any gameplay path (later Scott task)
pub(crate) fn scott_session_opt_mut(engine: &mut dyn Engine) -> Option<&mut ScottSession> {
    engine.as_any_mut().downcast_mut::<ScottSession>()
}

/// The engine tag (`"zmachine"` / `"glulx"` / `"scott"`) of the running engine,
/// for wrapping raw same-engine save bytes (e.g. a rewind/replay snapshot) into
/// an [`app::engine::EngineSave`] before `restore_state`.
pub(crate) fn engine_tag(engine: &dyn Engine) -> &'static str {
    if engine.as_any().is::<GlulxSession>() {
        app::glulx_session::GLULX_ENGINE
    } else if engine.as_any().is::<ScottSession>() {
        app::scott_session::SCOTT_ENGINE
    } else {
        app::session::ZMACHINE_ENGINE
    }
}

/// Convert an [`app::engine::EngineError`] from `restore_state` into a graceful
/// player-facing message (no panic): a foreign-engine save names both engines, a
/// bad Z-machine save keeps the historical "different story" wording.
pub(crate) fn restore_error_msg(e: app::engine::EngineError) -> String {
    use app::engine::EngineError;
    match e {
        EngineError::EngineMismatch { expected, found } => format!(
            "this save was written by the \"{found}\" engine, but lanthorn is running the \"{expected}\" engine"
        ),
        EngineError::BadSave(msg) if msg.contains("SaveMismatch") => {
            "save is for a different story".to_string()
        }
        EngineError::BadSave(msg) => format!("restore failed: {msg}"),
    }
}

/// Outcome of [`restore_from_file`]: either the pending `@save`/`@restore`
/// descriptor was completed from game-save bytes, or a full session was resumed
/// from a host Save State archive (`Engine::restore_state`). Either way the
/// caller re-observes the current location; when an `ArchiveContents` comes back
/// it also applies the archive's map/screen/transcript/aux via
/// [`apply_archive_state`].
pub(crate) enum RestoreOutcome {
    /// Descriptor-PC bytes were completed. `None` for a bare `.qzl` carried in
    /// from another interpreter (it has no sidecars); `Some` for an in-game
    /// `@save` `.lanthorn`, which carries the full session alongside them.
    DescriptorCompleted(Option<Box<app::archive::ArchiveContents>>),
    Resumed(Box<app::archive::ArchiveContents>),
}

/// Restore `path` into `session`, dispatching on which PC convention its game
/// bytes follow (SQ-0227, retargeted by SQ-0531).
///
/// A bare `.qzl` and an `@save`-triggered `.lanthorn` both hold descriptor-PC
/// bytes, so both complete the pending `@save` descriptor
/// (`Engine::restore_game_save`); a host-Save-State `.lanthorn` resumes the whole
/// session (`Engine::restore_state`). The convention now comes from the archive's
/// `Meta::trigger`, not from the file extension — `@save` writes a `.lanthorn`
/// too. This is still the fix for the SQ-0163 regression: every host restore path
/// used to call `restore_state` unconditionally, landing the VM on the descriptor
/// instead of past it. Shared by every host load/restore site (saves-manager Load,
/// `/restore-state`, and a `.lanthorn` picked from the in-game restore picker).
pub(crate) fn restore_from_file(path: &std::path::Path, session: &mut dyn Engine) -> Result<RestoreOutcome, String> {
    if app::persist_files::is_game_save(path) {
        let bytes = app::archive::read_quetzal_from_file(path).map_err(|e| e.to_string())?;
        session.restore_game_save(&bytes).map_err(restore_error_msg)?;
        note_bare_quetzal_width(session);
        return Ok(RestoreOutcome::DescriptorCompleted(None));
    }
    let tag = engine_tag(&*session);
    let ac = load_archive(path).map_err(|e| e.to_string())?;
    if ac.meta.trigger.is_portable() {
        // An in-game `@save`: the same guard `restore_state` applies internally,
        // done by hand because `restore_game_save` takes bare bytes.
        app::archive::restore_engine_allowed(&ac.engine, tag)?;
        session.restore_game_save(&ac.save).map_err(restore_error_msg)?;
        Ok(RestoreOutcome::DescriptorCompleted(Some(Box::new(ac))))
    } else {
        session.restore_state(&ac.engine_save()).map_err(restore_error_msg)?;
        Ok(RestoreOutcome::Resumed(Box::new(ac)))
    }
}

/// Floor the declared-width guard for a restore that brought game memory in
/// WITHOUT any screen state: a bare Quetzal (`.qzl`/`.sav`), from another
/// interpreter or an older lanthorn (SQ-0681).
///
/// [`app::session::restore_screen`] can raise the floor to the restored game's
/// own frame of reference because a `.lanthorn` archives the screen the game was
/// laid out on. A bare Quetzal, by design, archives none: ZMSD/Quetzal treat the
/// header's screen dimensions as the interpreter's, to be re-stamped on restore,
/// so the width whose columns the restored status routine baked in is genuinely
/// unknowable here.
///
/// The conservative choice is [`zvm::screen::DEFAULT_SCREEN_COLS`] (80), not the
/// current session's floor. 80 is the Z-machine's conventional screen width, the
/// width zvm falls back to, and the width every lanthorn session booted at before
/// SQ-0680 seeded the real pane — so it is what a foreign or older `.qzl` was
/// overwhelmingly laid out for. Getting it wrong in this direction costs at most
/// a bar clipped at the right of a narrower pane (what any interpreter shows for
/// an 80-column game in a 60-column window); not flooring at all re-manifests the
/// SQ-0679 garble — digits printed over the room name — on every turn after the
/// restore. It only ever grows the floor, so a session already wider than 80 is
/// untouched, and it is a no-op for the versions the floor exempts (v1–3, v6).
///
/// Not applied to an in-game `@save` `.lanthorn`, which travels the same
/// descriptor-completion path but DOES carry its screen: there
/// [`apply_archive_state`] hands `restore_screen` the real width, and guessing 80
/// first would pin a genuinely 60-column save to 80.
pub(crate) fn note_bare_quetzal_width(session: &mut dyn Engine) {
    if let Some(z) = zvm_session_opt_mut(session) {
        z.note_restored_screen_cols(zvm::screen::DEFAULT_SCREEN_COLS as u16);
    }
}

/// Reinstate everything an archive carries OUTSIDE the VM: the map, the
/// transcript (styles, paragraph layout, and inline art), the Z-machine screen,
/// the v6 graphics canvases, aux data, turn history, and the turn counter.
///
/// The VM-side restore has already happened by the time this runs — either a full
/// `restore_state` resume or a descriptor completion from an in-game `@save`
/// archive (SQ-0531). Shared by every `.lanthorn` restore site so the two never
/// drift; the caller still owns its own notices and overlay bookkeeping.
pub(crate) fn apply_archive_state(
    ac: app::archive::ArchiveContents,
    session: &mut dyn Engine,
    mapper: &mut mapper::mapper::Mapper,
    state: &mut app::state::AppState,
) {
    // A restore swaps the world out from under everything derived from it, so it
    // is a turn boundary like any other (SQ-1175): the epoch bump is what tells
    // the epoch-gated readers (the command band's object columns) to re-read the
    // engine, and `begin_turn` also retires anything the previous world was
    // still waiting on (a pending vocab offer, a lit reveal).
    state.begin_turn();
    if let Some(scr) = ac.screen.clone() {
        if let Some(z) = zvm_session_opt_mut(session) {
            app::session::restore_screen(z, scr);
        }
    }
    // The v6 screen: rebuilt from the archived display list under the archived
    // palette when there is one (SQ-0588), else from canvas PNGs alone. A no-op for
    // non-v6 archives and for Glulx (SQ-0516).
    apply_v6_pictures(session, &ac);
    // Hand Glulx back the room it was saved in (SQ-0523); no-op for zvm.
    seed_resumed_location(session, &ac.meta);
    if state.config.aux_storage != app::config::AuxStorage::Global {
        session.set_aux_data(ac.aux.clone());
    }
    *mapper = ac.mapper;
    state.transcript = ac.transcript;
    state.clear_anchor = None;
    state.transcript_kinds = ac.transcript_kinds;
    state.transcript_runs = ac.transcript_runs;
    state.transcript_para = ac.transcript_para;
    state.reset_transcript_sidecars();
    // Re-attach inline images AFTER the sidecar reset so a restored transcript
    // renders its embedded art, not just its text (SQ-0518).
    state.transcript_images = ac.transcript_images;
    state.history = ac.history;
    // Named-slot archives carry no command history; only adopt it when present
    // so loading a slot doesn't wipe the per-game history.
    if !ac.command_history.is_empty() {
        state.command_history = ac.command_history;
    }
    state.turns = ac.meta.turns;
    // The scraped word set is derived from the transcript and nothing else, so it
    // is REBUILT here rather than carried in the archive (SQ-1135). The sidecar
    // reset above already dropped the pre-restore set; this fills it from the
    // transcript that replaced it, so the noun columns and Tab completion are
    // right on the very first frame after a restore instead of one turn later.
    // Restoring to before a word was printed therefore takes the word away,
    // which is the correct per-save answer and comes free from deriving it.
    app::input::refresh_seen_words(state, &*session);
}

/// Hand a resumed session the room the archive recorded (SQ-0523).
///
/// A no-op for the Z-machine, whose `current_location` reads the restored object
/// tree. Glulx has no object tree: it recovers the room from the heading the game
/// prints and caches it OUTSIDE the VM snapshot, so a resume left that cache
/// holding the fresh boot's opening room and the map selected the wrong room
/// until the next turn reprinted a heading. Call at every site that resumes a
/// `.lanthorn`, beside `restore_screen` — before `reobserve_location`, which is
/// what reads the location back out.
pub(crate) fn seed_resumed_location(session: &mut dyn Engine, meta: &app::archive::Meta) {
    if let (Some(name), Some(g)) = (meta.location.as_deref(), glulx_session_opt_mut(session)) {
        g.seed_last_room(name);
    }
}

/// Whether the active engine is the Z-machine `GameSession` required by the
/// standard `.qzl`/`.sav` Quetzal **import** path.
///
/// That path reaches the concrete session via [`zvm_session_mut`]/[`zvm_session_opt`]
/// (which PANIC / return `None` on any other engine) and reads raw Quetzal saves
/// from other interpreters — Z-machine-only until cross-interpreter Glulx Quetzal exists.
/// They check this first and bail gracefully when it returns `false` (a Glulx
/// game). The `.lanthorn` archive save/restore/restart paths no longer need it:
/// they route through the engine-neutral `Engine::save_state`/`restore_state`
/// and work for both engines.
pub(crate) fn engine_supports_save(engine: &dyn Engine) -> bool {
    engine.as_any().downcast_ref::<GameSession>().is_some()
}

#[cfg(all(test, feature = "t-session"))]
mod tests {
    // ── SQ-0227 Task 3: restore dispatch on file extension ──────────────────────
    //
    // `restore_from_file` is the dispatch shared by every host restore site
    // (saves-manager Load, `/restore-state`, and a `.lanthorn` picked from the
    // in-game restore picker). Regression proof for SQ-0163: every host
    // restore path used to call `restore_state` (resume) unconditionally, so
    // a host restore of an in-game `@save` (`.qzl`) landed the VM on the
    // descriptor instead of past it.

    use crate::read_char_then_save_v4_story;

    /// SQ-0681. A bare Quetzal carries game memory but no screen, so the width
    /// its status layout was baked at is unknowable — `note_bare_quetzal_width`
    /// floors the declared width at the conventional 80 rather than leaving a
    /// narrow session's own boot width to make every baked field column illegal.
    ///
    /// The contrast in the second half is the reason the guess is scoped to this
    /// one branch: a `.lanthorn` restore must NOT take the guess, because an
    /// archive knows the real width (`restore_screen`) and pinning a genuinely
    /// 60-column session to 80 would undo SQ-0680's pre-boot seed.
    #[test]
    fn a_bare_qzl_restore_floors_the_declared_width_at_the_default_sq0681() {
        use app::engine::Engine;
        use app::session::{GameSession, PendingIo};

        // A narrow session: booted with the real 60-column pane seeded (SQ-0680).
        let narrow = || {
            GameSession::new_with_trace(
                read_char_then_save_v4_story(), true, false, None, false, Vec::new(), None, None,
                Some((24, 60)),
            )
            .expect("new")
        };

        // Bare .qzl bytes, captured while the @save descriptor is still pending.
        let mut donor = GameSession::new(read_char_then_save_v4_story(), true, false, None).expect("new");
        assert_eq!(donor.submit_char(b'x').pending_io, Some(PendingIo::Save));
        let qzl_bytes = donor.machine.save_quetzal();
        let qzl_path = std::env::temp_dir().join(format!("bm-sq0681-{}.qzl", std::process::id()));
        std::fs::write(&qzl_path, &qzl_bytes).unwrap();

        let mut sess = narrow();
        assert_eq!(sess.boot_screen_cols, 60, "the session booted at its real narrow pane");
        let outcome = super::restore_from_file(&qzl_path, &mut sess).expect("restore .qzl game save");
        assert!(matches!(outcome, super::RestoreOutcome::DescriptorCompleted(None)));
        assert_eq!(
            sess.boot_screen_cols,
            zvm::screen::DEFAULT_SCREEN_COLS as u16,
            "a screenless restore floors the declared width at the conventional 80"
        );
        let _ = std::fs::remove_file(&qzl_path);

        // Contrast: a Save State .lanthorn. Its width comes from the archived
        // screen (here: none archived), never from the bare-Quetzal guess.
        let save = Engine::save_state(&narrow());
        let lanthorn_path = std::env::temp_dir().join(format!("bm-sq0681-{}.lanthorn", std::process::id()));
        app::archive::save_archive(&lanthorn_path, &mapper::mapper::Mapper::default(), &save, None,
            &std::collections::BTreeMap::new(), &[], &[], &[], &[], &[], &[]).expect("write .lanthorn");

        let mut sess2 = narrow();
        let outcome2 = super::restore_from_file(&lanthorn_path, &mut sess2).expect("restore .lanthorn");
        assert!(matches!(outcome2, super::RestoreOutcome::Resumed(_)));
        assert_eq!(
            sess2.boot_screen_cols, 60,
            "an archive restore never takes the bare-Quetzal guess — SQ-0680's narrow seed stands"
        );
        let _ = std::fs::remove_file(&lanthorn_path);
    }

    /// SQ-0787. Every restore site funnels through [`super::apply_v6_pictures`], and
    /// the painted ground (SQ-0706) is the layer that used to ride straight past it:
    /// `auto_load` restores after the story has painted its own opening screen, so a
    /// resumed scopa came back with its MAIN MENU under the restored hand.
    ///
    /// The funnel must reset the ground even when the archive carries none, which is
    /// what makes the fix reach every v6 story rather than only the ones that paint.
    /// The real-story end of this — the ground actually surviving a round trip and a
    /// move after it — is `crates/app/tests/suites/v6_restore_paint_ground.rs`.
    ///
    /// Falsified by deleting the `load_paint_ground` call from `apply_v6_pictures`:
    ///
    /// ```text
    /// the restore funnel replaces the painted ground from the archive — an archive
    /// with none leaves an EMPTY ground, not the pre-restore screen's
    /// ```
    #[test]
    fn a_restore_resets_the_painted_ground_it_found_sq0787() {
        use app::engine::Engine;
        use app::session::GameSession;

        let mut sess =
            GameSession::new(read_char_then_save_v4_story(), true, false, None).expect("new");

        // Stand a painted ground up the only way a test can — the same setter the
        // restore path uses — so "the funnel cleared it" is a real observation.
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            4, 4, image::Rgba([1, 2, 3, 255]),
        ))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("encode");
        sess.load_paint_ground(Some(&png));
        assert!(Engine::paint_surface(&sess).is_some(), "premise: a ground is on screen");

        let path =
            std::env::temp_dir().join(format!("bm-sq0787-{}.lanthorn", std::process::id()));
        let save = Engine::save_state(&sess);
        app::archive::save_archive(&path, &mapper::mapper::Mapper::default(), &save, None,
            &std::collections::BTreeMap::new(), &[], &[], &[], &[], &[], &[]).expect("write");
        let ac = app::archive::load_archive(&path).expect("load");
        let _ = std::fs::remove_file(&path);
        assert!(ac.ground.is_none(), "a non-v6 archive carries no painted ground");

        super::apply_v6_pictures(&mut sess, &ac);
        assert!(
            Engine::paint_surface(&sess).is_none(),
            "the restore funnel replaces the painted ground from the archive — an \
             archive with none leaves an EMPTY ground, not the pre-restore screen's"
        );
    }

    /// SQ-0814, the same shape as SQ-0787's test above and for the same reason: the
    /// `erase_window` fills and the canvas anchors rode past every restore site too.
    ///
    /// The half that reaches every v6 story is the RESET, so that is what this pins —
    /// an archive carrying no layers (a non-v6 game, or the `ac.display == None`
    /// branch of the funnel) must leave the screen wearing NONE, not the ones the
    /// pre-restore session left. The real-story end — the layers surviving a round
    /// trip and a move after it, on four stories and both colour modes — is
    /// `crates/app/tests/suites/v6_restore_screen_layers.rs`.
    ///
    /// Falsified by deleting the `load_v6_screen_layers` call from `apply_v6_pictures`:
    ///
    /// ```text
    /// the restore funnel replaces the screen layers from the archive — an archive
    /// with none leaves NO fills and NO anchors, not the pre-restore screen's
    /// ```
    #[test]
    fn a_restore_resets_the_screen_layers_it_found_sq0814() {
        use app::engine::Engine;
        use app::session::GameSession;

        let mut sess =
            GameSession::new(read_char_then_save_v4_story(), true, false, None).expect("new");

        // Stand a fill and an anchor up the only way a test outside the crate can — the
        // same setter the restore path uses — so "the funnel cleared them" is a real
        // observation.
        sess.load_v6_screen_layers(Some(&app::archive::V6LayersDto {
            fills: vec![app::archive::V6FillDto { win: 3, x: 0, y: 0, w: 64, h: 48, bg: 0x00FF_00FF }],
            anchors: vec![app::archive::V6AnchorDto {
                win: 3, origin_x: 1, origin_y: 1, x: 0, y: 0, w: 64, h: 48,
            }],
        }));
        assert_eq!(
            sess.v6_screen_layers().fills.len(), 1,
            "premise: a fill and an anchor are on the screen"
        );

        let path =
            std::env::temp_dir().join(format!("bm-sq0814-{}.lanthorn", std::process::id()));
        let save = Engine::save_state(&sess);
        app::archive::save_archive(&path, &mapper::mapper::Mapper::default(), &save, None,
            &std::collections::BTreeMap::new(), &[], &[], &[], &[], &[], &[]).expect("write");
        let ac = app::archive::load_archive(&path).expect("load");
        let _ = std::fs::remove_file(&path);
        assert!(ac.display.is_none(), "a non-v6 archive carries no display list to hold them");

        super::apply_v6_pictures(&mut sess, &ac);
        assert_eq!(
            sess.v6_screen_layers(),
            app::archive::V6LayersDto::default(),
            "the restore funnel replaces the screen layers from the archive — an archive \
             with none leaves NO fills and NO anchors, not the pre-restore screen's"
        );
    }

    #[test]
    fn restore_from_file_completes_qzl_descriptor_and_resumes_lanthorn_sq0163() {
        use app::engine::Engine;
        use app::session::{GameSession, InputKind, PendingIo};

        // In-game @save: suspend with pending_save set (descriptor PC), and
        // capture the .qzl bytes exactly as save_game_named does (Task 2) --
        // while pending_save is still set, before resume_save runs.
        let mut sess = GameSession::new(read_char_then_save_v4_story(), true, false, None).expect("new");
        let r = sess.submit_char(b'x');
        assert_eq!(r.pending_io, Some(PendingIo::Save));
        let qzl_bytes = sess.machine.save_quetzal();
        let _ = sess.resume_save(true); // host "wrote" the .qzl; @save completes, VM runs to quit.

        let qzl_path = std::env::temp_dir().join(format!("bm-t3-{}.qzl", std::process::id()));
        std::fs::write(&qzl_path, &qzl_bytes).unwrap();

        // HOST restore of that .qzl (the SQ-0163 regression scenario): must
        // dispatch to descriptor completion, not a resume.
        let mut fresh = GameSession::new(read_char_then_save_v4_story(), true, false, None).expect("new");
        let outcome = super::restore_from_file(&qzl_path, &mut fresh).expect("restore .qzl game save");
        assert!(matches!(outcome, super::RestoreOutcome::DescriptorCompleted(None)));
        assert_eq!(fresh.machine.global(0), 2, "descriptor completion stores 2 into G0 (SQ-0163 fix)");
        // SQ-0233: the host .qzl restore now runs FORWARD past the @save
        // descriptor to the game's next input (like the in-game @restore),
        // instead of parking on the save-verb tail (which dropped the first
        // typed command). This minimal story quits right after @save, so it runs
        // to quit; a real game lands at its next read (covered by
        // session::tests::game_save_restore_via_manager_accepts_next_command).
        assert_ne!(fresh.machine.state.pc, 0x46,
            "restore runs forward past the @save descriptor, not parked on it (SQ-0233)");
        let _ = std::fs::remove_file(&qzl_path);

        // Contrast: a Save State (.lanthorn) is resume-PC convention --
        // captured at an input prompt, no pending @save. The dispatch must
        // instead do a full session resume, landing exactly at the saved PC.
        let sess2 = GameSession::new(read_char_then_save_v4_story(), true, false, None).expect("new");
        assert_eq!(sess2.pending_input(), InputKind::Char);
        let pc_before_restore = sess2.machine.state.pc;
        let save = sess2.save_state();

        let lanthorn_path = std::env::temp_dir().join(format!("bm-t3-{}.lanthorn", std::process::id()));
        app::archive::save_archive(&lanthorn_path, &mapper::mapper::Mapper::default(), &save, None,
            &std::collections::BTreeMap::new(), &[], &[], &[], &[], &[], &[]).expect("write .lanthorn");

        let mut fresh2 = GameSession::new(read_char_then_save_v4_story(), true, false, None).expect("new");
        let outcome2 = super::restore_from_file(&lanthorn_path, &mut fresh2).expect("restore .lanthorn Save State");
        assert!(matches!(outcome2, super::RestoreOutcome::Resumed(_)));
        assert_eq!(fresh2.machine.state.pc, pc_before_restore, "resume convention: lands exactly at the saved PC, not the @save descriptor");
        assert_eq!(fresh2.machine.global(0), 0, "resume: @save never ran, G0 untouched (contrast with descriptor completion's 2 above)");
        let _ = std::fs::remove_file(&lanthorn_path);
    }

    // ── Graceful no-panic guards for non-Z-machine (Glulx) engines ──────────────

    /// Minimal non-Z-machine `Engine` stand-in. The guard helper only inspects
    /// `as_any`, so every gameplay/persistence method is left `unreachable!()`:
    /// a guarded path that reaches one would be the very panic we are preventing.
    struct NotZmachineEngine;

    impl app::engine::Engine for NotZmachineEngine {
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
        fn save_state(&self) -> app::engine::EngineSave { unreachable!() }
        fn restore_state(&mut self, _save: &app::engine::EngineSave) -> Result<(), app::engine::EngineError> { unreachable!() }
        fn restore_game_save(&mut self, _bytes: &[u8]) -> Result<(), app::engine::EngineError> { unreachable!() }
        fn aux_data(&self) -> &std::collections::BTreeMap<String, Vec<u8>> { unreachable!() }
        fn set_aux_data(&mut self, _data: std::collections::BTreeMap<String, Vec<u8>>) { unreachable!() }
        fn aux_dirty(&self) -> bool { false }
        fn clear_aux_dirty(&mut self) {}
        fn current_location(&self) -> Option<app::engine::LocationInfo> { None }
        fn as_any(&self) -> &dyn std::any::Any { self }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    }

    #[test]
    fn engine_supports_save_false_for_non_zmachine() {
        let engine: Box<dyn app::engine::Engine> = Box::new(NotZmachineEngine);
        assert!(
            !super::engine_supports_save(&*engine),
            "a non-Z-machine engine must report no save support so guards short-circuit"
        );
    }

    #[test]
    fn engine_default_screen_trace_is_empty_and_toggle_is_noop() {
        use app::engine::Engine as _;
        let mut engine = NotZmachineEngine;
        engine.set_trace_screen(true); // default no-op must not panic
        assert!(engine.take_screen_trace().is_empty());
    }

    #[test]
    fn tags_scott_engine() {
        let s = app::scott_session::ScottSession::new(
            include_bytes!("../../scott/tests/tiny_cave.dat").to_vec(),
            None,
        )
        .unwrap();
        let boxed: Box<dyn app::engine::Engine> = Box::new(s);
        assert_eq!(super::engine_tag(&*boxed), "scott");
    }
}
