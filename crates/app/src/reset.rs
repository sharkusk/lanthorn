//! Game restart/reset. The rebuild itself is the library's
//! [`app::host::reset::reset_game`] (SQ-1539); this is the TUI's call into it,
//! which supplies the one fact only a terminal has — its size, from which the
//! restarted story's host pane is measured (SQ-1061).

use app::engine::Engine;
use app::host::reset::ResetOptions;
use app::state::AppState;
use mapper::mapper::Mapper;

#[allow(clippy::too_many_arguments)]
pub(crate) fn reset_game(
    session: &mut dyn Engine,
    mapper: &mut Mapper,
    state: &mut AppState,
    story_bytes: &[u8],
    story_path: &std::path::Path,
    game_dir: &std::path::Path,
    clear_map: bool,
    delete_data: bool,
) {
    app::host::reset::reset_game(
        session,
        mapper,
        state,
        story_bytes,
        story_path,
        game_dir,
        // The LIVE terminal, the same question the launch asks (SQ-1061).
        crossterm::terminal::size().ok(),
        ResetOptions { clear_map, delete_data },
    );
}

#[cfg(all(test, feature = "t-session"))]
mod tests {
    /// SQ-0546: restarting a v6 story must rebuild it the way LAUNCH does.
    ///
    /// **A restart advertises the number the LAUNCH advertised** (SQ-1058).
    ///
    /// `Config::advertised_interpreter_number` is a three-rung cascade and this
    /// file reproduced rungs 1 and 3 by hand, omitting rung 2 — SQ-0930's
    /// `IbmPc + ProfileSource::Medium -> 6`. `IbmPc::interpreter_number()` is
    /// deliberately `None`, so a restart fell through to zvm's own default rule
    /// (Frotz's: 6 for Version 6, **1** otherwise) and a **v5** story off a DOS
    /// medium came back claiming to be a DECSystem-20.
    ///
    /// `floppy1.ima` is the frame the rule was written for: *Beyond Zork* swaps
    /// Font 3's arrows for CP437 character graphics when it believes it is on an
    /// IBM PC, and zvm gates that on this byte. Version 6 masks the defect
    /// entirely — both roads reach 6 — so the specimen has to be a v5.
    ///
    /// FALSIFY by restoring
    /// `state.config.interpreter_number.or_else(|| profile.interpreter_number())`:
    /// the launch reads 6 and the restart reads 1.
    #[test]
    fn a_restart_advertises_the_interpreter_number_the_launch_advertised() {
        use app::interpreter::{InterpreterProfile, ProfileSource};
        let story = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/floppy1.ima");
        let Ok((loaded, medium)) = app::hints::load_mounted_story(&story) else {
            eprintln!("SKIP: gitignored medium missing at {}", story.display());
            return;
        };
        let app::hints::LoadedStory::ZCode(bytes) = loaded else {
            panic!("floppy1.ima carries Z-code");
        };
        let (profile, source) =
            InterpreterProfile::resolve_with_source(&story, None, None, medium);
        // The premise, stated rather than assumed: this really is the DOS medium
        // that names the IBM PC, and the story really is the version that can
        // show the difference.
        assert_eq!(profile, InterpreterProfile::IbmPc, "a DOS medium names the IBM PC");
        assert_eq!(source, ProfileSource::Medium, "and it is the MEDIUM that named it");
        assert_ne!(bytes[0], 6, "a Version 6 story would mask this — both roads reach 6");

        let mut state = app::state::AppState::default();
        state.config.interpreter_profile = profile;
        state.config.interpreter_source = source;
        let advertised = state.config.advertised_interpreter_number();
        assert_eq!(advertised, Some(6), "the launch advertises the IBM PC");

        let boot = app::machine_boot::MachineBoot::resolve(
            profile,
            &app::graphics::PictSource::new(None),
            None,
            advertised,
            None,
            state.config.machine_colours_licensed(),
            app::native_font::FaceSet::none(),
            zvm::screen::Palette::Standard,
            None,
        );
        let s = app::session::GameSession::new_for_machine(
            bytes.clone(), true, false, false, Default::default(), None, None, &boot,
        )
        .expect("Beyond Zork boots off the DOS medium");
        let mut engine: Box<dyn app::engine::Engine> = Box::new(s);
        assert_eq!(
            app::engine_helpers::zvm_session_mut(&mut *engine).machine.mem.read_byte(0x1e),
            6,
            "launch: header $1E is the IBM PC",
        );

        let mut mapper = mapper::mapper::Mapper::default();
        super::reset_game(
            &mut *engine, &mut mapper, &mut state, &bytes, &story,
            std::path::Path::new(""), false, false,
        );
        assert_eq!(
            app::engine_helpers::zvm_session_mut(&mut *engine).machine.mem.read_byte(0x1e),
            6,
            "restart: and so is it after @restart — not zvm's DECSystem-20 fallback",
        );
    }

    /// **The host pane reaches the boot, and a restart seeds it too** (SQ-1061).
    ///
    /// Two halves, because the second cannot be driven headlessly.
    ///
    /// The MECHANISM is pinned here: `host_screen` is what makes a v4/v5 story's
    /// boot-time status layout come out at the pane's width rather than zvm's
    /// 80-column fallback (SQ-0680), and `GameSession::new_for_machine` writes it
    /// into header `$21`.
    ///
    /// That a restart SUPPLIES it is a source-level guard, for the reason
    /// CLAUDE.md gives for `scratch_path_discipline`: the wrong spelling cannot be
    /// made unreachable here. The TUI asks the LIVE terminal, which a test has
    /// none of — it answers `None` in this process whatever the call passes, so a
    /// behavioural restart case would be green either way and would prove
    /// nothing. The omission it replaced was a bare `None`, so what the guard
    /// watches is that both halves still name the shared path: the library's
    /// restart measures the pane through `story_screen_in`, the same helper the
    /// launch uses, and this wrapper hands it the live terminal size (SQ-1539
    /// split the one call into those two halves).
    #[test]
    fn the_host_pane_reaches_a_boot_and_reset_still_seeds_one() {
        // A **v5** specimen, because §8.4 writes the screen size into `$20`/`$21`
        // from Version 4 on; Zork 1 is SQ-0680's own report but its v3 header has
        // no width to read, and the fact under test is that the pane reaches the
        // boot at all.
        let story = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/beyondzork-r57-s871221.z5");
        if let Ok(bytes) = std::fs::read(&story) {
            assert_eq!(bytes[0], 5, "the specimen is the version that records $21");
            let boot = app::machine_boot::MachineBoot::resolve(
                app::interpreter::InterpreterProfile::IbmPc,
                &app::graphics::PictSource::new(None),
                None,
                None,
                None,
                false,
                app::native_font::FaceSet::none(),
                zvm::screen::Palette::Standard,
                None,
            );
            let seeded = app::session::GameSession::new_for_machine(
                bytes.clone(), true, false, false, Default::default(), Some((24, 60)), None, &boot,
            )
            .expect("Beyond Zork boots");
            let bare = app::session::GameSession::new_for_machine(
                bytes, true, false, false, Default::default(), None, None, &boot,
            )
            .expect("Beyond Zork boots");
            assert_eq!(seeded.machine.mem.read_byte(0x21), 60, "the pane's own width reaches $21");
            assert_eq!(
                bare.machine.mem.read_byte(0x21),
                zvm::screen::DEFAULT_SCREEN_COLS,
                "and without it the story is told zvm's fallback — the argument is not inert",
            );
        } else {
            eprintln!("SKIP: gitignored story missing at {}", story.display());
        }

        // The guard: a restart still asks the same question the launch asks.
        // Matched on the whole file rather than on a sliced argument list, because
        // the arguments carry comments and a paren-balanced slice of prose is its
        // own small parser to get wrong. Each needle is assembled from two pieces
        // so this test's own text is not itself a match.
        let lib = include_str!("host/reset.rs");
        let measured = format!("super::story_screen_in{}", "(state, size)");
        assert_eq!(
            lib.matches(measured.as_str()).count(),
            1,
            "the library's restart must seed the host pane through `story_screen_in`, the \
             same helper the launch uses. It passed a bare `None` here until SQ-1061, and no \
             behavioural test can catch that: the TUI asks the LIVE terminal and this \
             process has none, so a restart case is green either way."
        );
        let src = include_str!("reset.rs");
        let live = format!("crossterm::terminal::size{}", "().ok()");
        assert_eq!(
            src.matches(live.as_str()).count(),
            1,
            "and the TUI's restart must hand it the live terminal's size"
        );
    }

    /// `reset_game`'s Z-machine arm used the bare `GameSession::new`, which
    /// carries no Pict dimension table, no Blorb `Reso` standard window and no
    /// host colour pair, and never attached a Pict source or flushed the boot
    /// pictures. A v6 game needs every one of those: `picture_data` is answered
    /// DURING the boot run inside the constructor, the `Reso` window sizes the
    /// 640×400 unit screen its windows and hardcoded art align to, and the art
    /// drawn during boot has to be drained once afterwards. So `/reset-game` on
    /// Shogun came back with a mis-sized status band and no graphics at all
    /// (user report at the TTY, 2026-07-28).
    ///
    /// Pins the observable half: after a reset the v6 screen model is present at
    /// its native size and the boot art has been rasterized.
    #[test]
    fn reset_game_rebuilds_a_v6_story_with_its_pictures_and_screen() {
        use app::engine::{Engine, WinNode};
        let story = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/shogun-r322-s890706.z6");
        let Ok(bytes) = std::fs::read(&story) else {
            eprintln!("SKIP: gitignored story missing at {}", story.display());
            return;
        };
        // Build the session the way launch does, so the "before" state is real.
        let mut picts = app::graphics::PictSource::new(
            blorb::resolve_resource_blorb(&story).map(|(b, _)| b),
        );
        let dims = picts.all_pict_dims();
        let std_window = picts.std_window();
        let mut s = app::session::GameSession::new_with_trace(
            bytes.clone(), true, false, None, false, dims, std_window, None, None
        )
        .expect("Shogun boots");
        s.set_pict_source(Some(picts));
        s.flush_boot_pictures();
        let mut engine: Box<dyn Engine> = Box::new(s);

        let native_before = match &engine.screen().root {
            WinNode::Layered(items) => app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT)),
            other => panic!("expected a v6 Layered root before reset, got {other:?}"),
        };
        assert_eq!(native_before, (640, 400), "launch sizes the v6 unit screen");

        let mut mapper = mapper::mapper::Mapper::default();
        let mut state = app::state::AppState::default();
        state.config.images = true;
        state.turns = 5;
        super::reset_game(
            &mut *engine, &mut mapper, &mut state, &bytes, &story,
            std::path::Path::new(""), false, false,
        );

        assert_eq!(state.turns, 0, "restart resets the turn counter");
        let model = engine.screen();
        let items = match &model.root {
            WinNode::Layered(items) => items,
            other => panic!("v6 story must still present a Layered root after reset, got {other:?}"),
        };
        assert_eq!(
            app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT)),
            native_before,
            "the restarted v6 screen keeps its native size (the Reso standard window \
             reached the constructor)"
        );
        // The boot art was rasterized again: at least one graphics window carries
        // opaque pixels. Without the Pict source + boot flush every canvas is empty.
        fn painted(node: &WinNode) -> bool {
            match node {
                WinNode::Graphics(g) => g.canvas.pixels().any(|p| p.0[3] != 0),
                WinNode::Pair { first, second, .. } => painted(first) || painted(second),
                WinNode::Layered(items) => items.iter().any(|i| painted(&i.node)),
                _ => false,
            }
        }
        assert!(
            painted(&model.root),
            "the restarted boot re-drew its graphics (Pict source attached + boot flush)"
        );
    }

    /// SQ-0673: a restart is a new game, so the death watch goes with the old one.
    ///
    /// It carries two things across turns — the `tried` record a fatal move may still have to
    /// take back, and an unresolved death waiting for a resurrection — and both are claims about
    /// a game that no longer exists. Left set, the outstanding death would swallow the first
    /// passage the fresh game walked, which is exactly the move that seeds the new map.
    #[test]
    fn reset_game_clears_the_death_watch() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let Ok(bytes) = std::fs::read(&fixture) else { return };
        let session = app::session::GameSession::new(bytes.clone(), true, false, None)
            .expect("zcode session");
        let mut engine: Box<dyn app::engine::Engine> = Box::new(session);
        let mut mapper = mapper::mapper::Mapper::default();
        let mut state = app::state::AppState::default();
        state.death_watch = app::session::DeathWatch {
            pending_tried: Some((7, mapper::direction::Direction::N)),
            unresolved: true,
        };
        super::reset_game(
            &mut *engine,
            &mut mapper,
            &mut state,
            &bytes,
            &fixture,
            std::path::Path::new(""),
            false,
            false,
        );
        assert_eq!(
            state.death_watch,
            app::session::DeathWatch::default(),
            "the restarted game inherits no outstanding death"
        );
    }

    #[test]
    fn reset_game_rebuilds_zcode_engine() {
        // Restart rebuilds a working Z-machine engine via the story factory and
        // resets the turn counter.
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let Ok(bytes) = std::fs::read(&fixture) else { return };
        let mut engine: Box<dyn app::engine::Engine> =
            Box::new(app::session::GameSession::new(bytes.clone(), true, false, None).expect("zcode session"));
        let mut mapper = mapper::mapper::Mapper::default();
        let mut state = app::state::AppState::default();
        // Inline-prompt mode (command_bar off): the rebuilt session must inherit
        // strip_prompt=false so @restart doesn't revert to stripping the game's `>`.
        state.config.command_bar = false;
        state.turns = 5;
        super::reset_game(&mut *engine, &mut mapper, &mut state, &bytes, &fixture, std::path::Path::new(""), false, false);
        assert_eq!(state.turns, 0, "restart resets the turn counter");
        assert!(engine.as_any().is::<app::session::GameSession>(),
            "still a Z-machine session after restart");
        assert!(
            !engine.as_any().downcast_ref::<app::session::GameSession>().unwrap().strip_prompt(),
            "restart re-applies inline-prompt mode (strip_prompt stays false)"
        );
    }

    #[test]
    fn reset_game_shows_the_fresh_map_not_the_previous_games() {
        // Reset re-seeds the mapper graph via the production path (a wholesale
        // mapper replacement when `clear_map`); the render cache must show the
        // FRESH map afterwards, never a stale routed model left over from the
        // previous game. (SQ-0305) Since SQ-1544 this can no longer rely on a
        // generation-number comparison alone — a freshly loaded graph's
        // `struct_gen` starts back at 0 (see its own doc comment) and could
        // coincidentally equal whatever the stale cache was routed for — so
        // `reset_game` drops the cache unconditionally via
        // `AppState::invalidate_map_render` (only `map_render`/`map_derived`/
        // `render_job`/`tidy_job`/`anim_build_job` are `pub(crate)` to the
        // library, so this drives it through the public `cached_map_render`/
        // `poll_render_job` API rather than inspecting the cache directly, this
        // file being part of the binary crate rather than the library).
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let Ok(bytes) = std::fs::read(&fixture) else { return };
        let mut engine: Box<dyn app::engine::Engine> =
            Box::new(app::session::GameSession::new(bytes.clone(), true, false, None).expect("zcode session"));
        let mut mapper = mapper::mapper::Mapper::default();
        // Seed a stale cached model for a room no fresh reset can ever re-create.
        mapper.observe(999_999, "Stale Room From The Old Game", None);
        mapper.graph.set_pos(999_999, (0, 0));
        let mut state = app::state::AppState::default();
        let drain = |state: &mut app::state::AppState, graph: &mapper::graph::MapGraph| {
            let _ = state.cached_map_render(mapper::layer::MAIN_LAYER, graph);
            while state.map_render_in_flight() {
                state.poll_render_job(graph);
                std::thread::yield_now();
            }
        };
        drain(&mut state, &mapper.graph);
        {
            let rm = state.cached_map_render(mapper::layer::MAIN_LAYER, &mapper.graph);
            assert!(
                rm.rooms.iter().any(|r| r.label.contains("Stale Room")),
                "fixture: the cache must show the seeded room before reset"
            );
        }

        super::reset_game(&mut *engine, &mut mapper, &mut state, &bytes, &fixture, std::path::Path::new(""), true, false);
        assert!(mapper.graph.room(999_999).is_none(), "fixture: clear_map must actually replace the graph");

        drain(&mut state, &mapper.graph);
        let rm = state.cached_map_render(mapper::layer::MAIN_LAYER, &mapper.graph);
        assert!(
            !rm.rooms.iter().any(|r| r.label.contains("Stale Room")),
            "reset must not go on showing the previous game's stale map: {:?}",
            rm.rooms.iter().map(|r| &r.label).collect::<Vec<_>>()
        );
    }

    #[test]
    fn reset_game_rebuilds_glulx_engine() {
        // Restart routes Glulx through the factory too (no "not supported"): a
        // fresh GlulxSession replaces the old one and the turn counter resets.
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../gvm-cli/tests/fixtures/glulxercise.ulx");
        let Ok(bytes) = std::fs::read(&fixture) else { return };
        let mut engine: Box<dyn app::engine::Engine> = Box::new(
            app::glulx_session::GlulxSession::new(bytes.clone(), 80, 24, true, false, false, (1.0, 1.0), None, &[])
                .expect("glulx session"),
        );
        let mut mapper = mapper::mapper::Mapper::default();
        let mut state = app::state::AppState::default();
        // config.images defaults true, so restart drives the graphics-enabled
        // rebuild branch: the fixture .ulx has no sidecar .blorb, so
        // resolve_pict_blorb resolves to None and graphics_enabled = true is
        // threaded in — the rebuild must succeed without panicking.
        assert!(state.config.images, "default config enables images");
        state.turns = 5;
        super::reset_game(&mut *engine, &mut mapper, &mut state, &bytes, &fixture, std::path::Path::new(""), false, false);
        assert_eq!(state.turns, 0, "restart resets the turn counter for Glulx");
        assert!(engine.as_any().is::<app::glulx_session::GlulxSession>(),
            "still a Glulx session after restart");
    }

    /// SQ-1504: reproduced at a 100x70 terminal, as reported. `reset_game`
    /// rebuilds Glulx at the fallback width (`FALLBACK_SCREEN_COLS`/`ROWS`, 80x24)
    /// exactly as a fresh launch's own constructor does — `state.config.
    /// virtual_screen_cols`/`rows` are unset by default in both paths, per
    /// `startup.rs`'s Glulx arm. A LAUNCH still ends up at the real pane width
    /// because `main.rs`'s per-frame `loop_tick::poll_glulx_resize` sees its
    /// `vm_story_size` tracker at its initial `None`, treats the real pane as new,
    /// and calls `GlulxSession::resize` once its settle timer elapses. A restart
    /// leaves that tracker exactly as it was before the restart — untouched by
    /// `reset_game`, which has no access to it — so if the terminal itself hasn't
    /// moved, the tracker already equals the (unchanged) pane and the poll never
    /// re-measures the freshly rebuilt (narrower) session against it. The story
    /// panel then stays at the fallback width until an actual terminal resize
    /// event forces the comparison to differ, which is exactly the reported
    /// symptom ("until the window is resized").
    ///
    /// Observed here through the status Grid window's own `cols`, which gvm
    /// derives from whatever `(cols, rows)` the session is CURRENTLY told —
    /// booting Anchorhead at 80x24 vs 100x70 reports `cols=80` vs `cols=100` on
    /// that same grid (checked with a throwaway probe against the real fixture).
    ///
    /// Falsified as instructed: skipping the `vm_story_size`/`story_size_seen`/
    /// `resize_dirty` reset that `main.rs`'s `OverlayAct::ResetConfirm` and
    /// `OverlayAct::GameOverPlayAgain` arms now perform after `reset_game`
    /// reproduces exactly the reported symptom below — the poll reports no
    /// redraw and the status grid stays at the fallback width.
    #[test]
    fn reset_game_glulx_requires_the_resize_trackers_cleared_to_reach_the_real_pane_width() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories/Anchorhead.gblorb");
        let Ok(raw) = std::fs::read(&fixture) else {
            eprintln!("SKIP: no stories/Anchorhead.gblorb");
            return;
        };
        let pane = (100u32, 70u32);

        // Boot the way a real launch at a 100x70 terminal would: the same two
        // parses `startup.rs` and `reset.rs` both do (one for the executable, one
        // kept whole for the Pict resources).
        let blorb1 = blorb::Blorb::parse(raw.clone()).expect("parse blorb (executable)");
        let (_, image) = blorb1.executable().expect("Anchorhead carries an executable chunk");
        let image = image.to_vec();
        let blorb2 = blorb::Blorb::parse(raw.clone()).expect("parse blorb (pictures)");
        let mut engine: Box<dyn app::engine::Engine> = Box::new(
            app::glulx_session::GlulxSession::new(image, pane.0, pane.1, true, true, false, (8.0, 16.0), Some(blorb2), &[])
                .expect("Anchorhead should boot"),
        );

        fn status_grid_cols(engine: &dyn app::engine::Engine) -> Option<u16> {
            fn find(node: &app::engine::WinNode) -> Option<u16> {
                match node {
                    app::engine::WinNode::Grid(g) => Some(g.cols),
                    app::engine::WinNode::Pair { first, second, .. } => find(first).or_else(|| find(second)),
                    _ => None,
                }
            }
            find(&app::engine::Engine::screen(engine).root)
        }
        assert_eq!(
            status_grid_cols(&*engine),
            Some(pane.0 as u16),
            "booted at the real pane, the status grid spans it"
        );

        let mut mapper = mapper::mapper::Mapper::default();
        let mut state = app::state::AppState::default();
        super::reset_game(&mut *engine, &mut mapper, &mut state, &raw, &fixture, std::path::Path::new(""), false, false);
        assert_eq!(
            status_grid_cols(&*engine),
            Some(app::config::FALLBACK_SCREEN_COLS),
            "reset_game rebuilds at the fallback width, exactly as a fresh launch's constructor does"
        );

        // Stale trackers: the terminal hasn't moved, so both still read the pane
        // size reported BEFORE the reset — the exact state `main.rs`'s locals are
        // left in when nothing clears them.
        let last_panes = crate::PaneRects {
            story: ratatui::layout::Rect::new(0, 0, pane.0 as u16, pane.1 as u16),
            ..Default::default()
        };
        let mut vm_story_size = Some((pane.0 as u16, pane.1 as u16));
        let mut story_size_seen = Some((pane.0 as u16, pane.1 as u16));
        let mut resize_dirty: Option<std::time::Instant> = None;

        let redraw = crate::loop_tick::poll_glulx_resize(
            &mut *engine, &last_panes, &mut story_size_seen, &mut resize_dirty, &mut vm_story_size,
        );
        assert!(!redraw, "stale trackers read the fresh session as already matching the pane");
        assert_eq!(
            status_grid_cols(&*engine),
            Some(app::config::FALLBACK_SCREEN_COLS),
            "BUG reproduced: without clearing the trackers the story panel stays at the fallback width"
        );

        // THE FIX: the exact call `main.rs`'s `ResetConfirm`/`GameOverPlayAgain`
        // arms now make right after `reset_game`.
        crate::loop_tick::reset_glulx_resize_trackers(&mut vm_story_size, &mut story_size_seen, &mut resize_dirty);
        let redraw = crate::loop_tick::poll_glulx_resize(
            &mut *engine, &last_panes, &mut story_size_seen, &mut resize_dirty, &mut vm_story_size,
        );
        assert!(!redraw, "the settle timer has not elapsed on this very first pass");
        // A later pass, once the 150ms settle window has elapsed.
        resize_dirty = std::time::Instant::now().checked_sub(std::time::Duration::from_millis(200));
        let redraw = crate::loop_tick::poll_glulx_resize(
            &mut *engine, &last_panes, &mut story_size_seen, &mut resize_dirty, &mut vm_story_size,
        );
        assert!(redraw, "once settled, the poll must re-measure the fresh session against the real pane");
        assert_eq!(
            status_grid_cols(&*engine),
            Some(pane.0 as u16),
            "the fix restores the story panel to the real pane width"
        );
    }

    #[test]
    fn reset_game_with_delete_data_removes_auto_sidecars() {
        // delete_data = true wipes the three AUTO sidecars in game_dir before the
        // rebuild, while keeping the player's named/in-game saves.
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let Ok(bytes) = std::fs::read(&fixture) else { return };

        let game_dir = std::env::temp_dir()
            .join(format!("lanthorn-reset-delete-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&game_dir);
        std::fs::create_dir_all(&game_dir).unwrap();
        for f in ["default.glkvfs", "default.aux", "default.lanthorn"] {
            std::fs::write(game_dir.join(f), b"x").unwrap();
        }
        std::fs::write(game_dir.join("myslot.lanthorn"), b"x").unwrap();
        std::fs::write(game_dir.join("quick.qzl"), b"x").unwrap();

        let mut engine: Box<dyn app::engine::Engine> =
            Box::new(app::session::GameSession::new(bytes.clone(), true, false, None).expect("zcode session"));
        let mut mapper = mapper::mapper::Mapper::default();
        let mut state = app::state::AppState::default();
        super::reset_game(&mut *engine, &mut mapper, &mut state, &bytes, &fixture, &game_dir, false, true);

        for f in ["default.glkvfs", "default.aux", "default.lanthorn"] {
            assert!(!game_dir.join(f).exists(), "{f} should be deleted by delete_data");
        }
        assert!(game_dir.join("myslot.lanthorn").exists(), "named save kept");
        assert!(game_dir.join("quick.qzl").exists(), "in-game save kept");

        let _ = std::fs::remove_dir_all(&game_dir);
    }
}
