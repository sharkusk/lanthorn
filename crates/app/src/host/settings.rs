//! Applying a changed config to a RUNNING session (SQ-1559): the bookkeeping
//! the settings screen's Save does, callable by a host that is not a terminal.
//!
//! It used to live inline in `Action::ConfigSave`'s arm in `input.rs` plus a
//! tail in `main.rs`'s run loop, so a host that let a player edit
//! `config.toml` mid-game had to copy it field by field — and would drift the
//! next time someone added a `_base` or a one-run hold to the TUI's copy. The
//! TUI now runs on these two functions too, so there is one copy.
//!
//! Two halves, because the TUI's settings-screen tests drive the first through
//! `apply_action` with no session and must not write the user's real
//! `config.toml`:
//!
//! - [`apply`] — the `AppState` half: the config itself, the `_base` values,
//!   the one-run holds on `AppState`, the sound sink, the render mirrors, and
//!   (for a host) this game's sidecar layered back over the top.
//! - [`commit`] — the half that must run after it: write `config.toml`, tell
//!   the engine, then re-resolve the live look. The ORDER is load-bearing — see
//!   the comment above `reload_style` below.
//!
//! What only a terminal can do stays with the caller, reported on [`Applied`]:
//! mouse capture, and the style file-watcher (`state.pending_watch_style`).

use crate::config::{keys, Config};
use crate::engine::Engine;
use crate::state::AppState;
use crate::styles::PerGameConfig;

/// What [`apply`] changed that a caller has to act on, and what [`commit`]
/// needs to finish the job.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Applied {
    /// `Some(new)` when the `mouse` setting changed. Only a terminal host has
    /// mouse capture to re-arm; any other host ignores it.
    pub mouse: Option<bool>,
    /// `Some(new)` when `command_bar` changed: [`commit`] re-applies the
    /// session's prompt stripping from it (inline mode keeps the game's `>`,
    /// command-bar mode strips it).
    pub strip_prompt: Option<bool>,
    /// The Glulx borderless-windows mode this game's sidecar asks for, when
    /// [`apply`] was handed one (`per_game.borderless_windows`, else the
    /// discovered `garglk.ini`'s, else off — boot's precedence). `None` when no
    /// sidecar was given: the settings screen has no such row, so the TUI's
    /// Save never touches borders.
    pub borderless: Option<bool>,
}

/// What [`commit`] did.
pub struct Committed {
    /// Writing `config.toml`. An error here is worth telling the player — Save
    /// that silently saves nothing is the worst place to swallow it (SQ-0580).
    pub config_write: std::io::Result<()>,
    /// Re-resolving the live look. `Reloaded` means the theme, `period_look`,
    /// the honour key and the rest were rebuilt and the whole frame needs a
    /// repaint; `Failed` means the look is untouched (and why).
    pub style: crate::reload::ReloadOutcome,
    /// True when the running Glulx session's window borders were switched and
    /// its window tree relaid out — the host re-reads the screen model.
    pub borders_changed: bool,
}

/// Apply `working` to the running session's `AppState` — everything the
/// settings screen's Save does before the file is written.
///
/// `working` is the WORKING copy, not a bare global file: the live
/// `state.config` with the player's edits applied, and `one_run.release(key)`
/// called for every key the player edited — exactly what the settings screen
/// builds. A one-run pin still on `working` means "the player did not touch
/// this key", and that is what keeps a CLI flag, a sidecar or the artwork's
/// force-off in force across a save of some unrelated setting (SQ-0807,
/// SQ-0860). A config freshly parsed from disk has no pins, so every one-run
/// choice would read as a deliberate edit — start from `state.config.clone()`
/// instead.
///
/// `per_game`, when given, is this game's sidecar (normally
/// `PerGameConfig::read(&state.game_dir)`), layered over the result the way
/// `host::boot` layers it at launch: the `_base`s are lowered from `working`
/// FIRST, then each key the sidecar names replaces the live value and is
/// pinned, so one game's choice never reaches the global `config.toml`. The
/// TUI passes `None` — its working copy already carries the sidecar's values
/// as the pins boot left. Keys that only mean something at boot (`show_map`,
/// `panel`, `pictures`, `interpreter_number`, the Scott options, `quick`) are
/// not applied live.
pub fn apply(state: &mut AppState, working: Config, per_game: Option<&PerGameConfig>) -> Applied {
    let mouse = (working.mouse != state.config.mouse).then_some(working.mouse);
    let strip_prompt = (working.command_bar != state.config.command_bar).then_some(working.command_bar);

    state.config = working;
    // The config screen edits the GLOBAL honor default; keep the
    // SQ-0318 base in sync so a later reload_style doesn't revert it
    // (a per-game override, if any, still wins on the next reload).
    state.honor_game_colours_base = state.config.honor_game_colours;
    // SQ-0860: the base alone is not enough when a one-run source is
    // holding this key, because `reload_style` only falls back to the
    // base when nothing per-story is speaking. Editing the row calls
    // `one_run.release` (see `one_run_key_for_row`), so a missing pin
    // on a key that had one IS the deliberate edit — end the holds that
    // live on `AppState` too, or the next style reload recomputes the
    // user's own choice straight back off. Untouched rows keep their
    // pin, so saving some unrelated setting changes nothing here.
    if !state.config.one_run.holds(keys::HONOR_GAME_COLOURS) {
        state.game_colours_cli = None;
        state.artwork_declines_colours = false;
    }
    if let Some(b) = state.audio.as_mut() {
        b.set_volume(state.config.volume);
    } else if state.config.enable_sound {
        state.audio = Some(crate::host::sound::default_sound_sink(state.config.volume));
    }
    if !state.config.enable_sound {
        state.reset_sound_sidecars();
    }
    // Sync the running Glulx VM's Sound gestalt (drained by `commit`, or by
    // the TUI's event loop, which drains it after every action).
    state.pending_vm_sound = Some(state.config.enable_sound);
    // Reconcile the style file-watcher live (the run loop owns it).
    state.pending_watch_style = Some(state.config.watch_style);
    // SQ-1161: two settings are mirrored onto `AppState` at boot and
    // read from THERE by render — `startup.rs` seeds both and the
    // toggle keys drive the mirror, not the config. Saving the row
    // without lowering it wrote config.toml and changed nothing on
    // screen until the next launch, which is exactly the silent
    // half-application the screen's contract forbids.
    state.show_status_bar = state.config.show_status_bar;
    state.show_room_numbers = state.config.show_room_numbers;
    // SQ-1161: and four more keys keep a `_base` on `AppState` — the
    // GLOBAL default a per-story source overrides for one launch, and
    // what `/set-guidance auto` (and its siblings) fall back to. The
    // honour row's base is lowered above for the same reason; without
    // these, saving the row moved the live value and left `auto`
    // pointing at the value the session started with.
    //
    // Only when nothing per-story is pinning the key: a pin means the
    // row was NOT edited (editing releases it, above), so `working`
    // still holds someone else's value for this run and lowering it
    // would turn one game's choice into everyone's (SQ-0807).
    if !state.config.one_run.holds(keys::GUIDANCE) {
        state.guidance_base = state.config.guidance;
    }
    if !state.config.one_run.holds(keys::RETURN_PROBE) {
        state.return_probe_base = state.config.return_probe;
    }
    if !state.config.one_run.holds(keys::V6_PIXEL_LOCK) {
        state.v6_pixel_lock_base = state.config.v6_pixel_lock;
    }
    if !state.config.one_run.holds(keys::V6_RENDER) {
        state.v6_render_base = state.config.v6_render;
    }

    let borderless = per_game.map(|pg| layer_per_game(state, pg));
    Applied { mouse, strip_prompt, borderless }
}

/// Layer this game's sidecar over the live config, with `host::boot`'s
/// precedence and pins (the `_base`s were already lowered from the global
/// value, which is the order boot captures them in). Returns the borderless
/// mode the sidecar resolves to.
///
/// A non-terminal host has no command line, so boot's `--game-colours` /
/// `--guidance` / … filters have nothing to filter; the honour key's two
/// launch-wide holds that DO live on `AppState` (the CLI flag and the
/// artwork's force-off) still outrank the sidecar, exactly as `reload_style`
/// ranks them.
fn layer_per_game(state: &mut AppState, pg: &PerGameConfig) -> bool {
    let cfg = &mut state.config;
    if let Some(v) = pg
        .honor_game_colours
        .filter(|_| state.game_colours_cli.is_none() && !state.artwork_declines_colours)
    {
        cfg.honor_game_colours = v;
        cfg.one_run.pin(keys::HONOR_GAME_COLOURS, v);
    }
    if let Some(v) = pg.colour_source {
        cfg.colour_source = v;
        if v == crate::config::ColourSource::Machine {
            cfg.system_colours = true;
            cfg.one_run.pin(keys::SYSTEM_COLOURS, true);
        }
    }
    if let Some(v) = pg.v6_pixel_lock {
        cfg.v6_pixel_lock = v;
        cfg.one_run.pin(keys::V6_PIXEL_LOCK, v);
    }
    if let Some(v) = pg.guidance {
        cfg.guidance = v;
        cfg.one_run.pin(keys::GUIDANCE, v);
    }
    if let Some(v) = pg.return_probe {
        cfg.return_probe = v;
        cfg.one_run.pin(keys::RETURN_PROBE, v);
    }
    if let Some(m) = pg.v6_render.as_deref().and_then(crate::config::v6_render_from_key) {
        cfg.v6_render = m;
        cfg.one_run.pin(keys::V6_RENDER, crate::config::v6_render_key(m));
    }
    pg.borderless_windows
        .or_else(|| state.garglk_overlay.as_ref().and_then(|o| o.borderless))
        .unwrap_or(false)
}

/// Finish what [`apply`] started, on the running `session`: write
/// `state.config` to `config.toml`, sync the engine, and re-resolve the live
/// look. Run it after [`apply`], once per save.
pub fn commit(state: &mut AppState, session: &mut dyn Engine, applied: &Applied) -> Committed {
    // The running Glulx VM's Sound gestalt, so games that re-check
    // gestalt_Sound per play (e.g. sensory.blorb's gong) honor the change.
    // (The TUI's event loop has usually drained this already.)
    if let Some(on) = state.pending_vm_sound.take() {
        if let Some(gs) = crate::engine_helpers::glulx_session_opt_mut(session) {
            gs.set_sound(on);
        }
    }
    let config_write = crate::config::write_config_file(&state.config);
    // Re-apply prompt stripping live so toggling the command bar takes
    // effect on the next turn without a restart.
    if let Some(on) = applied.strip_prompt {
        session.set_strip_prompt(on);
    }
    let mut borders_changed = false;
    if let Some(on) = applied.borderless {
        if let Some(gs) = crate::engine_helpers::glulx_session_opt_mut(session) {
            if gs.borderless() != on {
                gs.set_borderless(on);
                borders_changed = true;
            }
        }
    }
    // SQ-1161: and re-resolve the live look, AFTER the write above. This is
    // the single funnel the style watcher and `/reload-style` go through, so
    // it is what makes the `period_look` row (and the theme layers, and this
    // story's own style.toml and garglk.ini overlays) land on Save instead of
    // waiting for the next launch. It must run after `write_config_file`,
    // because it recomputes `honor_game_colours` from this story's sidecar and
    // re-pins the key — and a pinned key is skipped by the writer, so running
    // it first would drop the honour row's own edit out of the file.
    let style = crate::reload::reload_style(state);
    Committed { config_write, style, borders_changed }
}

#[cfg(all(test, feature = "t-session"))]
mod tests {
    use super::*;
    use crate::config::{ColourSource, V6RenderMode};

    /// The session double `commit` drives: records every prompt-stripping call,
    /// and is not a Glulx session, so the gestalt/borderless arms find nothing.
    #[derive(Default)]
    struct StripRecorder {
        strip_calls: Vec<bool>,
    }

    impl Engine for StripRecorder {
        fn submit(&mut self, _command: &str) -> crate::session::TurnResult {
            unreachable!("not exercised by this test")
        }
        fn submit_key(&mut self, _key: crate::engine::KeyInput) -> Option<crate::session::TurnResult> {
            unreachable!("not exercised by this test")
        }
        fn take_transcript(&mut self) -> String {
            String::new()
        }
        fn set_strip_prompt(&mut self, on: bool) {
            self.strip_calls.push(on);
        }
        fn drain_screen_clear(&mut self) -> bool {
            false
        }
        fn pending_input(&self) -> crate::session::InputKind {
            crate::session::InputKind::Line
        }
        fn resume_save(&mut self, _wrote_ok: bool) -> crate::session::TurnResult {
            unreachable!("not exercised by this test")
        }
        fn resume_restore(&mut self, _data: Option<&[u8]>) -> crate::session::TurnResult {
            unreachable!("not exercised by this test")
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

    /// The ORACLE: the settings screen's Save exactly as it ran before SQ-1559,
    /// transcribed from a64394b5 — `Action::ConfigSave`'s arm in `input.rs`
    /// (the first half) and `main.rs`'s post-dispatch tail (the second: the
    /// generic `pending_vm_sound` drain, `write_config_file`, the command-bar
    /// strip, `reload_style`). Frozen on purpose: it is what the new functions
    /// are held to, so it must NOT be edited to follow them.
    fn legacy_save(state: &mut AppState, working: Config, session: &mut dyn Engine) {
        let command_bar_before_save = state.config.command_bar;
        let cfg_to_write = working.clone();
        // ── input.rs, Action::ConfigSave ──
        state.config = working;
        state.honor_game_colours_base = state.config.honor_game_colours;
        if !state.config.one_run.holds(keys::HONOR_GAME_COLOURS) {
            state.game_colours_cli = None;
            state.artwork_declines_colours = false;
        }
        if let Some(b) = state.audio.as_mut() {
            b.set_volume(state.config.volume);
        } else if state.config.enable_sound {
            state.audio = Some(crate::host::sound::default_sound_sink(state.config.volume));
        }
        if !state.config.enable_sound {
            state.reset_sound_sidecars();
        }
        state.pending_vm_sound = Some(state.config.enable_sound);
        state.pending_watch_style = Some(state.config.watch_style);
        state.show_status_bar = state.config.show_status_bar;
        state.show_room_numbers = state.config.show_room_numbers;
        if !state.config.one_run.holds(keys::GUIDANCE) {
            state.guidance_base = state.config.guidance;
        }
        if !state.config.one_run.holds(keys::RETURN_PROBE) {
            state.return_probe_base = state.config.return_probe;
        }
        if !state.config.one_run.holds(keys::V6_PIXEL_LOCK) {
            state.v6_pixel_lock_base = state.config.v6_pixel_lock;
        }
        if !state.config.one_run.holds(keys::V6_RENDER) {
            state.v6_render_base = state.config.v6_render;
        }
        // ── main.rs, after apply_action ──
        if let Some(on) = state.pending_vm_sound.take() {
            if let Some(gs) = crate::engine_helpers::glulx_session_opt_mut(session) {
                gs.set_sound(on);
            }
        }
        if let Err(e) = crate::config::write_config_file(&state.config) {
            state.push_notice(&format!("[config not saved: {e}]"));
        }
        if cfg_to_write.command_bar != command_bar_before_save {
            session.set_strip_prompt(cfg_to_write.command_bar);
        }
        if let crate::reload::ReloadOutcome::Failed { msg } = crate::reload::reload_style(state) {
            state.push_notice(&format!("[style not reloaded: {msg}]"));
        }
    }

    /// The new path, as `main.rs` now runs it (minus the terminal-only mouse).
    fn new_save(
        state: &mut AppState,
        working: Config,
        per_game: Option<&PerGameConfig>,
        session: &mut dyn Engine,
    ) -> (Applied, Committed) {
        let applied = apply(state, working, per_game);
        let committed = commit(state, session, &applied);
        (applied, committed)
    }

    /// Every field either path writes, as one comparable value. `Config` has
    /// no `PartialEq`, so it is compared through `Debug` (which prints every
    /// field, `one_run` pins included). `colors` is left to [`same_look`]: its
    /// theme is a `HashMap`, whose `Debug` order differs run to run.
    fn touched(state: &AppState) -> String {
        format!(
            "config={:?}\nhonor_base={} guidance_base={} return_probe_base={} \
             v6_pixel_lock_base={} v6_render_base={:?}\ngame_colours_cli={:?} \
             artwork_declines={}\nshow_status_bar={} show_room_numbers={}\n\
             pending_vm_sound={:?} pending_watch_style={:?} audio={}\n\
             period_look={:?}",
            state.config,
            state.honor_game_colours_base,
            state.guidance_base,
            state.return_probe_base,
            state.v6_pixel_lock_base,
            state.v6_render_base,
            state.game_colours_cli,
            state.artwork_declines_colours,
            state.show_status_bar,
            state.show_room_numbers,
            state.pending_vm_sound,
            state.pending_watch_style,
            state.audio.is_some(),
            state.period_look,
        )
    }

    /// The live look (`ColorScheme`, theme included) is the same on both.
    fn same_look(a: &AppState, b: &AppState) -> bool {
        a.colors == b.colors
    }

    /// A running session whose machine CAN show a period look — a v3 story off
    /// its own medium, machine colours in force — with the look, the game's
    /// colours and the Guiding Light all off, and a one-run pin on the pixel
    /// lock (a key the saves below do not touch).
    fn seed(dir: &std::path::Path) -> AppState {
        std::fs::write(dir.join("style.toml"), "[colors]\n\"transcript\" = { fg = \"white\" }\n").unwrap();
        let mut s = AppState::default();
        s.config.user_dir = dir.to_path_buf();
        s.config.config_file = dir.join("config.toml");
        s.config.style = Some(dir.join("style.toml").to_string_lossy().to_string());
        s.story_zversion = Some(3);
        s.config.colour_source = ColourSource::Machine;
        s.config.interpreter_source = crate::interpreter::ProfileSource::Medium;
        s.config.period_look = false;
        s.config.honor_game_colours = false;
        s.honor_game_colours_base = false;
        s.config.guidance = false;
        s.guidance_base = false;
        s.config.show_room_numbers = false;
        s.show_room_numbers = false;
        // `--v6-pixel-lock on` (or a sidecar) over a global-off config.
        s.v6_pixel_lock_base = false;
        s.config.v6_pixel_lock = true;
        s.config.one_run.pin(keys::V6_PIXEL_LOCK, true);
        s.v6_render_base = V6RenderMode::Hybrid;
        s
    }

    /// What the settings screen builds when the player flips these rows: the
    /// live config with the edits applied and each edited key's pin released.
    fn flip_rows(s: &AppState) -> Config {
        let mut w = s.config.clone();
        w.honor_game_colours = true;
        w.one_run.release(keys::HONOR_GAME_COLOURS);
        w.guidance = true;
        w.one_run.release(keys::GUIDANCE);
        w.show_room_numbers = true;
        w.period_look = true;
        w.command_bar = !w.command_bar;
        w
    }

    /// SQ-1559's acceptance: `apply` + `commit` leave a running session in
    /// exactly the state the pre-refactor Save left it in, field for field —
    /// and the file on disk says the same thing.
    #[test]
    fn apply_and_commit_match_the_original_config_save_field_for_field() {
        let dir = crate::scratch_dir("settings-equiv");

        let mut old = seed(&dir);
        let working = flip_rows(&old);
        let mut old_eng = StripRecorder::default();
        legacy_save(&mut old, working.clone(), &mut old_eng);
        let old_file = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        std::fs::remove_file(dir.join("config.toml")).unwrap();

        let mut new = seed(&dir);
        let mut new_eng = StripRecorder::default();
        let (applied, committed) = new_save(&mut new, working, None, &mut new_eng);
        let new_file = std::fs::read_to_string(dir.join("config.toml")).unwrap();

        // Non-vacuity: the save really did move every field the brief names.
        assert!(old.config.honor_game_colours && old.guidance_base && old.show_room_numbers);
        assert!(old.period_look.is_some(), "the period look must actually come on, or this compares two Nones");
        assert_eq!(old_eng.strip_calls.len(), 1, "the command bar flip reached the old engine");

        assert_eq!(touched(&new), touched(&old));
        assert!(same_look(&new, &old), "the re-resolved look is the same");
        assert_eq!(new.transcript.len(), old.transcript.len(), "same notices");
        assert_eq!(new_file, old_file, "config.toml is written identically");
        assert_eq!(new_eng.strip_calls, old_eng.strip_calls);
        assert!(committed.config_write.is_ok());
        assert!(matches!(committed.style, crate::reload::ReloadOutcome::Reloaded { .. }));
        assert!(!committed.borders_changed);
        assert_eq!(applied.borderless, None, "no sidecar given, so borders are not in question");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The per-game layer: a host that hands `apply` the GLOBAL values plus
    /// this game's sidecar lands on the same state the TUI's Save lands on —
    /// the TUI carrying that sidecar as the pin boot left, the host re-layering
    /// it the way boot does. The per-game `guidance` wins live, is pinned, and
    /// the global base stays the global value.
    #[test]
    fn a_per_game_sidecar_layers_over_the_global_exactly_as_boot_left_it() {
        let dir = crate::scratch_dir("settings-per-game");
        // Boot of a story whose sidecar says `guidance = true` over a global off.
        let booted = |dir: &std::path::Path| {
            let mut s = seed(dir);
            s.config.guidance = true;
            s.config.one_run.pin(keys::GUIDANCE, true);
            s
        };
        let edits = |w: &mut Config| {
            w.honor_game_colours = true;
            w.one_run.release(keys::HONOR_GAME_COLOURS);
            w.show_room_numbers = true;
            w.period_look = true;
        };

        // The TUI: the guidance row untouched, so its pin rides in `working`.
        let mut old = booted(&dir);
        let mut working = old.config.clone();
        edits(&mut working);
        legacy_save(&mut old, working, &mut StripRecorder::default());
        std::fs::remove_file(dir.join("config.toml")).unwrap();

        // The host: the global value, no pin — and the sidecar beside it.
        let mut new = booted(&dir);
        let mut global = new.config.clone();
        edits(&mut global);
        global.guidance = false;
        global.one_run.release(keys::GUIDANCE);
        let pg = PerGameConfig { guidance: Some(true), ..Default::default() };
        new_save(&mut new, global, Some(&pg), &mut StripRecorder::default());

        assert!(new.config.guidance && new.config.one_run.holds(keys::GUIDANCE));
        assert!(!new.guidance_base, "the base is the GLOBAL default, not this game's");
        assert!(new.period_look.is_some() && new.show_room_numbers);
        assert_eq!(touched(&new), touched(&old));
        assert!(same_look(&new, &old), "the re-resolved look is the same");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A one-run hold on a key this save did not touch survives it — on the
    /// config (the pixel lock's pin, and its base) and on `AppState` (the
    /// artwork's force-off of the game's colours) — with or without a sidecar
    /// that names OTHER keys. `apply` ends only the holds the edit ended.
    #[test]
    fn a_hold_on_an_untouched_key_survives_the_apply() {
        for per_game in [None, Some(PerGameConfig { guidance: Some(true), ..Default::default() })] {
            let dir = crate::scratch_dir("settings-hold");
            let mut s = seed(&dir);
            // What a boot on a two-colour archive leaves (SQ-0806/SQ-0846).
            s.honor_game_colours_base = true;
            s.artwork_declines_colours = true;
            s.config.honor_game_colours = false;
            s.config.one_run.pin(keys::HONOR_GAME_COLOURS, false);

            let mut working = s.config.clone();
            working.show_room_numbers = true; // the only edit
            new_save(&mut s, working, per_game.as_ref(), &mut StripRecorder::default());

            assert!(s.show_room_numbers, "the edit itself landed");
            assert!(s.artwork_declines_colours, "the artwork's hold is not this save's to end");
            assert!(!s.config.honor_game_colours, "and the next reload kept the colours off");
            assert!(s.config.one_run.holds(keys::HONOR_GAME_COLOURS));
            assert!(s.config.one_run.holds(keys::V6_PIXEL_LOCK), "the pixel lock's pin survives");
            assert!(s.config.v6_pixel_lock);
            assert!(!s.v6_pixel_lock_base, "and its one-run value never became the global default");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// The sidecar's honour key does not outrank the artwork's force-off (the
    /// order `reload_style` ranks them in), and its borderless key is reported
    /// for the host to relay out — boot's precedence, per-game over garglk.ini.
    #[test]
    fn the_sidecar_yields_honour_to_the_artwork_and_reports_borders() {
        let mut s = AppState::default();
        s.artwork_declines_colours = true;
        s.config.honor_game_colours = false;
        s.config.one_run.pin(keys::HONOR_GAME_COLOURS, false);
        let working = s.config.clone();
        let pg = PerGameConfig {
            honor_game_colours: Some(true),
            borderless_windows: Some(true),
            ..Default::default()
        };
        let applied = apply(&mut s, working, Some(&pg));
        assert!(!s.config.honor_game_colours, "the artwork's force-off outranks the sidecar");
        assert_eq!(applied.borderless, Some(true));

        let mut s = AppState::default();
        let working = s.config.clone();
        let applied = apply(&mut s, working, Some(&PerGameConfig::default()));
        assert_eq!(applied.borderless, Some(false), "no sidecar key and no garglk.ini: bordered");
    }
}
