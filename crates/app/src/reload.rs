//! Live style reload: re-resolve style.toml from disk and swap the live
//! ColorScheme / SymbolSet, keeping the current look on a parse error.

use std::path::Path;

use crate::state::AppState;

/// Result of a reload attempt.
pub enum ReloadOutcome {
    /// Applied; carries any non-fatal resolve warnings — both `style::resolve`'s
    /// (legacy scheme-path warnings) and, appended after (SQ-0663), the
    /// registry theme's own [`crate::theme::resolve::Theme::warnings`]
    /// (formatted via [`crate::theme::resolve::describe_theme_warnings`]), so a
    /// `parent` typo or cycle in style.toml surfaces through the exact same
    /// notice path every other reload warning already uses.
    Reloaded { warnings: Vec<String> },
    /// Not applied (read/parse error); the current look is untouched.
    Failed { msg: String },
}

/// Resolve the `style` pointer to its on-disk path, if it names a real file.
/// Returns `None` for the built-in `"default"` or when `None` resolves to a
/// missing `user_dir/style.toml` (those parse the embedded default — no file).
pub fn resolved_style_path(style: Option<&str>, user_dir: &Path) -> Option<std::path::PathBuf> {
    match style {
        Some("default") => None,
        Some(p) => Some(crate::colors::expand_path(p, user_dir)),
        None => {
            let cand = user_dir.join("style.toml");
            if cand.is_file() { Some(cand) } else { None }
        }
    }
}

/// Re-read and apply `style.toml`. On a real-file read/parse error, the current
/// `state.colors`/`state.symbols` are left in place.
pub fn reload_style(state: &mut AppState) -> ReloadOutcome {
    let user_dir = state.config.user_dir.clone();
    let pointer = state.config.style.clone();

    // Build the StyleDoc: a real file parses directly (error → Failed); the
    // default/missing cases use the embedded default via load_style.
    let doc = match resolved_style_path(pointer.as_deref(), &user_dir) {
        Some(path) => match std::fs::read_to_string(&path) {
            Ok(text) => match crate::style::parse_style_toml(&text) {
                Ok(doc) => doc,
                Err(e) => return ReloadOutcome::Failed { msg: format!("{}: {}", path.display(), e) },
            },
            Err(e) => return ReloadOutcome::Failed { msg: format!("{}: {}", path.display(), e) },
        },
        None => {
            let (doc, _w) = crate::style::load_style(pointer.as_deref(), &user_dir);
            doc
        }
    };

    // Layer the per-game override (<game_dir>/style.toml) over the global.
    let doc = if !state.game_dir.as_os_str().is_empty() {
        let pg_path = crate::styles::per_game_style_path(&state.game_dir);
        if pg_path.is_file() {
            match std::fs::read_to_string(&pg_path) {
                Ok(text) => match crate::style::parse_style_toml(&text) {
                    Ok(over) => crate::style::merge(&doc, &over),
                    Err(e) => return ReloadOutcome::Failed { msg: format!("{}: {}", pg_path.display(), e) },
                },
                Err(e) => return ReloadOutcome::Failed { msg: format!("{}: {}", pg_path.display(), e) },
            }
        } else {
            doc
        }
    } else {
        doc
    };

    let (cs, set, warnings) = crate::style::resolve(&doc, &user_dir);
    state.colors = cs;
    state.symbols = set;
    // Re-apply the per-game garglk.ini overlay (SQ-0319): the resolve above
    // rebuilds `colors` from style.toml + <game_dir>/style.toml and would drop it,
    // so fold it back on so the imported look survives every reload path.
    if let Some(ov) = state.garglk_overlay.clone() {
        ov.apply(&mut state.colors);
    }
    // Honor-game-colours precedence (SQ-0318): the user's explicit per-game
    // override wins over the garglk.ini `stylehint` gate, which in turn wins over
    // the global config default. Recompute from disk each reload so `auto` (no
    // per-game override) falls back to garglk/global, and an explicit per-game
    // choice is never clobbered by the garglk gate.
    let per_game_honor = if state.game_dir.as_os_str().is_empty() {
        None
    } else {
        crate::styles::read_per_game_honor(&state.game_dir)
    };
    let garglk_honor = state.garglk_overlay.as_ref().and_then(|o| o.honor_game_colours);
    // SQ-0855: …unless `--game-colours` was typed on this launch, which outranks
    // both per-story sources — see `AppState::game_colours_cli`. Still expressed
    // as a per-story value rather than by lowering the base, so it is PINNED below
    // and cannot reach the global config.toml. SQ-1082 made it symmetric, so
    // `--game-colours on` now overrules a sidecar that turned them off, which is
    // what a flag typed on this launch ought to have done all along.
    // SQ-0860: …or unless the artwork in hand has no colours to give, which is the
    // same shape of fact and gets the same treatment. `startup.rs` forces the key
    // off and pins it before the engine is built (SQ-0806/SQ-0846); this recompute
    // runs a few lines later and would otherwise land on the global base — captured
    // BEFORE that force-off — releasing the pin and turning the colours back on for
    // every consumer that reads `config.honor_game_colours` after boot. Both of
    // these say "off, for this run" and neither may reach the user's file, so both
    // ride the per-story value rather than lowering the base.
    // The artwork's force-off is checked FIRST and is not overridable here: it is
    // a fact about the rendition in hand — a two-colour stencil has no colours to
    // give — rather than a preference, and honouring colours over it paints the
    // stencil out (SQ-0860). `/set-game-colours` clears both holds together, which
    // is still the way to say "I know, do it anyway".
    let per_story_honor = if state.artwork_declines_colours {
        Some(false)
    } else {
        state.game_colours_cli.or(per_game_honor).or(garglk_honor)
    };
    state.config.honor_game_colours = per_story_honor.unwrap_or(state.honor_game_colours_base);
    // SQ-0807: whatever this recompute lands on, say where it came from. A per-game
    // (or garglk) value is in force for THIS story and must not reach the global
    // config.toml — `/game-colours off` followed by a settings save would otherwise
    // turn one game's choice into everyone's. Falling back to the global base is the
    // opposite event: nothing is overriding the key any more, so any earlier pin
    // (the boot-time artwork force included) no longer describes the live value.
    match per_story_honor {
        Some(v) => state.config.one_run.pin(crate::config::keys::HONOR_GAME_COLOURS, v),
        None => state.config.one_run.release(crate::config::keys::HONOR_GAME_COLOURS),
    }

    // SQ-0309: rebuild the registry theme from the same global + per-game files, with
    // provenance. Render still reads the legacy fields above; the theme is consumed by
    // the Wave-5 render migration. New-schema sections drive it; an old-schema file
    // (only [colors]/[symbols]) parses to empty new-schema decls → registry defaults.
    {
        use crate::theme::{resolve::resolve_theme_layered, toml_schema};
        let parse_file = |p: Option<std::path::PathBuf>| -> toml_schema::ParsedStyle {
            p.and_then(|p| std::fs::read_to_string(&p).ok())
                .and_then(|t| toml_schema::parse(&t).ok())
                .unwrap_or_default()
        };
        let global = parse_file(resolved_style_path(pointer.as_deref(), &user_dir));
        let per_game = if state.game_dir.as_os_str().is_empty() {
            toml_schema::ParsedStyle::default()
        } else {
            parse_file(Some(crate::styles::per_game_style_path(&state.game_dir)))
        };
        let (_, gs, _) = crate::colors::resolve_base(global.scheme.as_deref(), &user_dir);
        // SQ-0510: with no scheme configured, `resolve_base` returns an all-Reset
        // scheme and `Roles::from_scheme` falls back to a hard-coded white-on-BLACK
        // `chrome` — a black band across every chrome-derived selector on a
        // terminal that is not black. Seed the scheme from the OSC 10/11 probe
        // taken at startup so the theme follows the real terminal. This is the ONE
        // place the theme is built (startup, `/reload-style`, the style watcher and
        // the per-game overlay all funnel through `reload_style`), so seeding here
        // reaches all of them. A terminal that did not answer both channels leaves
        // the scheme untouched and today's behaviour intact.
        let gs = crate::colors::seed_scheme_from_terminal(gs, &state.term_default_colors);
        // SQ-0309: a discovered garglk.ini's colours ride the theme's garglk layer
        // (between global and per-game) so they survive the migration off the legacy
        // ColorScheme fields (`ov.apply` still seeds the glk override array, which the
        // render glk path reads for per-style colours).
        let garglk_decls = state.garglk_overlay.as_ref().map(|o| o.color_decls()).unwrap_or_default();
        state.colors.theme = resolve_theme_layered(&gs, &global, &garglk_decls, &per_game);
    }

    // SQ-0873: and under all of it, the machine's own screen — but only for a
    // v1-v4 story, which has no colour concept for the theme to be overriding.
    // Recomputed here rather than at startup because both of its inputs move: the
    // theme was just rebuilt (so a `style.toml` edit that claims the transcript
    // must take the page back from the machine on the very next reload), and
    // `honor_game_colours` was recomputed a few lines above. `crate::period` holds
    // the gate and the composition rules.
    // SQ-0917: the render path's copy of the machine's Version 6 cell. Recomputed
    // here for the same reason `period_look` is — the profile is a config value the
    // settings screen can change under a running session — and from the SAME knob
    // the session was constructed with, so the renderer divides by the number the
    // engine multiplied by. They disagreeing is every run landing in the wrong
    // column, silently, because both answers look plausible.
    //
    // SQ-1009: through `TextFace`, because the cell FOLLOWS the face where the
    // release shipped a proportional one. Assigning the machine table's here is
    // what would clobber Arthur's 20-row line back to 16 on the next style reload,
    // hours into a session, with nothing on screen to say why. The face itself is
    // kept — only the medium can produce one and this scope has no medium — and
    // re-tested against the new profile, so a face that no longer fits declines
    // exactly as it would have at launch. BOTH of them (SQ-1036): a machine's
    // fixed-pitch alternate is as much a resolved fact as its body face, and
    // dropping it here would put a Macintosh status bar back into Geneva on the
    // next style reload.
    state.v6_text = crate::native_font::TextFace::new(
        state.config.interpreter_profile,
        state.v6_text.faces().clone(),
        Some(state.v6_art_scale),
    );
    state.period_look = crate::period::resolve(
        state.config.interpreter_profile,
        state.config.period_look,
        state.config.honor_game_colours,
        // SQ-0928's media rule, applied here too: a period look is what a machine's
        // screen LOOKED LIKE, so being on that machine is what makes it true.
        state.config.machine_colours_licensed(),
        state.story_zversion,
    );
    if let Some(look) = state.period_look {
        crate::period::apply_to_theme(&mut state.colors.theme, &look, state.story_zversion);
    }

    // SQ-0700/SQ-0703: the frame STYLES still live on `ColorScheme` (the renderers
    // frame and reserve from them), but the file they are written in is the new
    // schema, where the selectors sit in `[elements]` and land in the theme just
    // rebuilt above. Lower them back so `upper_window_border = { style = "none" }`,
    // `input_line = { style = "single" }` and `suggestion_line = { style = "single" }`
    // reach their frames instead of dying in the theme.
    crate::style::lower_element_borders(&mut state.colors);

    // SQ-0663: append the freshly-rebuilt theme's own diagnostics (a `parent`
    // re-root naming an unknown selector, or a cycle) after style::resolve's.
    // Every consumer of `ReloadOutcome::Reloaded` already loops over `warnings`
    // and pushes each as a transcript Warning, so folding these in here — rather
    // than adding a second warnings channel — reuses that path for free and
    // keeps the "quiet when clean" property: a reload whose theme resolves
    // cleanly appends nothing, so nothing is re-announced.
    let mut warnings = warnings;
    warnings.extend(crate::theme::resolve::describe_theme_warnings(state.colors.theme.warnings()));

    ReloadOutcome::Reloaded { warnings }
}

#[cfg(all(test, feature = "t-session"))]
mod tests {
    use super::*;
    use crate::state::AppState;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        crate::scratch_dir(&format!("reload-{tag}"))
    }

    #[test]
    fn reload_applies_style_file_and_keeps_current_on_error() {
        // SQ-0309: the legacy `[colors]` selector table no longer patches
        // `ColorScheme` fields directly (that now flows through the theme —
        // see `reload_builds_theme_from_new_schema_files`), so this exercises
        // the new-schema `[elements]` table instead. The behaviour under test
        // — reload keeps the current look when the file fails to parse — is
        // unrelated to which schema drives the colour.
        use ratatui::style::Color;
        let dir = temp_dir("ok");
        let path = dir.join("style.toml");
        std::fs::write(&path, "[elements]\ntranscript = { fg = \"green\" }\n").unwrap();

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.style = Some(path.to_string_lossy().to_string());

        let outcome = reload_style(&mut state);
        assert!(matches!(outcome, ReloadOutcome::Reloaded { .. }));
        assert_eq!(state.colors.theme.get("transcript").style.fg, Some(Color::Green));

        // Now break the file: reload keeps the current (green) look and reports Failed.
        std::fs::write(&path, "this is not valid = = toml [[[").unwrap();
        let outcome2 = reload_style(&mut state);
        assert!(matches!(outcome2, ReloadOutcome::Failed { .. }));
        assert_eq!(
            state.colors.theme.get("transcript").style.fg, Some(Color::Green),
            "current look preserved on error"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A style.toml is required for reload_style's file path; seed a minimal one.
    fn seed_style(dir: &std::path::Path) -> std::path::PathBuf {
        let global = dir.join("style.toml");
        std::fs::write(&global, "[colors]\n\"transcript\" = { fg = \"white\" }\n").unwrap();
        global
    }

    #[test]
    fn per_game_honor_override_beats_global_and_auto_falls_back() {
        let dir = temp_dir("honor");
        let global = seed_style(&dir);
        let game_dir = dir.join("game.save");
        std::fs::create_dir_all(&game_dir).unwrap();

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.style = Some(global.to_string_lossy().to_string());
        state.game_dir = game_dir.clone();
        // Global default (base) is honor = true.
        state.honor_game_colours_base = true;
        state.config.honor_game_colours = true;

        // A per-game override of false wins over the global true.
        crate::styles::write_per_game_honor(&game_dir, Some(false)).unwrap();
        reload_style(&mut state);
        assert!(!state.config.honor_game_colours, "per-game false wins over global true");

        // `auto` (override cleared) falls back to the global base (true).
        crate::styles::write_per_game_honor(&game_dir, None).unwrap();
        reload_style(&mut state);
        assert!(state.config.honor_game_colours, "auto falls back to global base");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-0807: `/game-colours off` writes THIS game's sidecar and reload_style folds
    /// the result into `state.config` — where the next settings save used to write it
    /// straight into the global config.toml, so one game's choice became every game's.
    /// The recompute pins it as one-run instead, and clearing the override (`auto`)
    /// releases the pin because nothing is overriding the key any more.
    #[test]
    fn a_per_game_honor_override_never_reaches_the_global_config() {
        let dir = temp_dir("honor-global");
        let global = seed_style(&dir);
        let game_dir = dir.join("game.save");
        std::fs::create_dir_all(&game_dir).unwrap();
        let cfg_path = dir.join("config.toml");
        std::fs::write(&cfg_path, "honor_game_colours = true\n").unwrap();

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.config_file = cfg_path.clone();
        state.config.style = Some(global.to_string_lossy().to_string());
        state.game_dir = game_dir.clone();
        state.honor_game_colours_base = true;
        state.config.honor_game_colours = true;

        crate::styles::write_per_game_honor(&game_dir, Some(false)).unwrap();
        reload_style(&mut state);
        assert!(!state.config.honor_game_colours, "the live session honours the choice");

        crate::config::write_config_file(&state.config).unwrap();
        let back = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(
            toml::from_str::<crate::config::Config>(&back).unwrap().honor_game_colours,
            "the GLOBAL file still says true — one game spoke, not all of them: {back}"
        );

        // Back to `auto`: the key is nobody's override now, so it persists as usual.
        crate::styles::write_per_game_honor(&game_dir, None).unwrap();
        reload_style(&mut state);
        state.config.honor_game_colours = false; // as if the settings panel turned it off
        crate::config::write_config_file(&state.config).unwrap();
        let back = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(
            !toml::from_str::<crate::config::Config>(&back).unwrap().honor_game_colours,
            "a global choice persists as always: {back}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn per_game_honor_wins_over_garglk_and_auto_falls_back_to_garglk() {
        let dir = temp_dir("honor-garglk");
        let global = seed_style(&dir);
        let game_dir = dir.join("game.save");
        std::fs::create_dir_all(&game_dir).unwrap();

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.style = Some(global.to_string_lossy().to_string());
        state.game_dir = game_dir.clone();
        // Global base true; a garglk.ini stylehint set honor = true.
        state.honor_game_colours_base = true;
        state.garglk_overlay = Some(crate::garglk_ini::GarglkOverlay {
            honor_game_colours: Some(true),
            ..Default::default()
        });

        // A per-game override of false wins over the garglk-set true.
        crate::styles::write_per_game_honor(&game_dir, Some(false)).unwrap();
        reload_style(&mut state);
        assert!(!state.config.honor_game_colours, "per-game false wins over garglk true");

        // `auto` falls back to the garglk value (true), not just the base.
        crate::styles::write_per_game_honor(&game_dir, None).unwrap();
        reload_style(&mut state);
        assert!(state.config.honor_game_colours, "auto falls back to garglk stylehint");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-0855: `--game-colours` outranks BOTH per-story sources a reload re-reads
    /// from disk. Lowering `honor_game_colours_base` alone is not enough — the base is
    /// only the fallback, so a `garglk.ini` beside the story or a sidecar `honor` key
    /// left by an earlier run would turn the game's colours back on at the boot
    /// `reload_style`, and the flag would do nothing on exactly the stories that have
    /// an opinion about colour.
    #[test]
    fn a_cli_game_colours_flag_outranks_garglk_and_the_per_game_sidecar() {
        let dir = temp_dir("honor-cli-flag");
        let global = seed_style(&dir);
        let game_dir = dir.join("game.save");
        std::fs::create_dir_all(&game_dir).unwrap();

        let cfg_path = dir.join("config.toml");
        std::fs::write(&cfg_path, "honor_game_colours = true\n").unwrap();

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.config_file = cfg_path.clone();
        state.config.style = Some(global.to_string_lossy().to_string());
        state.game_dir = game_dir.clone();
        // As `resolve()` leaves a `--game-colours off` launch: base lowered, flag noted.
        state.honor_game_colours_base = false;
        state.game_colours_cli = Some(false);
        // Both per-story sources ask loudly for the game's colours.
        state.garglk_overlay = Some(crate::garglk_ini::GarglkOverlay {
            honor_game_colours: Some(true),
            ..Default::default()
        });
        crate::styles::write_per_game_honor(&game_dir, Some(true)).unwrap();

        reload_style(&mut state);
        assert!(
            !state.config.honor_game_colours,
            "the flag was typed on THIS launch; a file beside the story was not"
        );
        // …and it is still one-run through the reload, so a settings save — the story
        // browser's "remember this directory?" prompt is enough — cannot bake one
        // launch's flag into the global config.
        crate::config::write_config_file(&state.config).unwrap();
        let back = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(
            toml::from_str::<crate::config::Config>(&back).unwrap().honor_game_colours,
            "the GLOBAL file must still say true — the flag was for one run: {back}"
        );

        // `/set-game-colours` is the user overriding their own flag, and clears it.
        state.game_colours_cli = None;
        reload_style(&mut state);
        assert!(
            state.config.honor_game_colours,
            "an explicit in-session choice ends the flag's hold (SQ-0646's rule)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The other honor mode, per the project's colour-render convention: with the
    /// flag absent, everything above must behave exactly as it always did — the
    /// shipped default is `honor_game_colours = true` and the per-story sources rule.
    #[test]
    fn without_the_flag_the_per_story_sources_still_rule_both_ways() {
        let dir = temp_dir("honor-noflag");
        let global = seed_style(&dir);
        let game_dir = dir.join("game.save");
        std::fs::create_dir_all(&game_dir).unwrap();

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.style = Some(global.to_string_lossy().to_string());
        state.game_dir = game_dir.clone();
        state.honor_game_colours_base = true;
        state.game_colours_cli = None;

        // The shipped default (true) with a sidecar asking for off.
        crate::styles::write_per_game_honor(&game_dir, Some(false)).unwrap();
        reload_style(&mut state);
        assert!(!state.config.honor_game_colours, "sidecar off wins over base true");

        // And a config that persisted false, with a sidecar asking for on.
        state.honor_game_colours_base = false;
        crate::styles::write_per_game_honor(&game_dir, Some(true)).unwrap();
        reload_style(&mut state);
        assert!(state.config.honor_game_colours, "sidecar on wins over base false");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── SQ-0860: the artwork force-off survives the boot reload ──────────────

    /// `startup.rs`'s honour sequence, up to the point the post-IFID
    /// `reload_style` runs: the base is captured FIRST, and the monochrome-artwork
    /// force-off lands on `Config` after it. `base` is the user's global file.
    fn state_after_the_artwork_force_off(
        tag: &str,
        base: bool,
    ) -> (AppState, std::path::PathBuf, std::path::PathBuf) {
        let dir = temp_dir(tag);
        let global = seed_style(&dir);
        let game_dir = dir.join("game.save");
        std::fs::create_dir_all(&game_dir).unwrap();
        let cfg_path = dir.join("config.toml");
        std::fs::write(&cfg_path, format!("honor_game_colours = {base}\n")).unwrap();

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.config_file = cfg_path.clone();
        state.config.style = Some(global.to_string_lossy().to_string());
        state.game_dir = game_dir.clone();
        state.honor_game_colours_base = base;
        state.game_colours_cli = None;
        state.config.honor_game_colours = base;
        if base {
            // The archive declared the interpreter colourless (SQ-0806/SQ-0846).
            state.artwork_declines_colours = true;
            state.config.honor_game_colours = false;
            state.config.one_run.pin(crate::config::keys::HONOR_GAME_COLOURS, false);
        }
        (state, dir, cfg_path)
    }

    /// The defect: the boot `reload_style` runs a few lines after the artwork
    /// force-off, finds no `garglk.ini` and no sidecar, and lands on the global
    /// base — which was captured BEFORE the force-off. Measured before the fix, the
    /// flag came back `true` and the pin was released in the same breath, so from
    /// the boot reload onward every consumer of `config.honor_game_colours` read
    /// the opposite of what the artwork asked for: `poll_zvm_default_colours`
    /// starts writing header $2C/$2D that §8.3.2 says to leave alone, and an
    /// `@restart` (`reset.rs`) rebuilds the session honouring the very colours that
    /// paint a two-colour stencil out.
    #[test]
    fn the_artwork_force_off_survives_the_boot_reload() {
        let (mut state, dir, _cfg) = state_after_the_artwork_force_off("mono-boot", true);
        reload_style(&mut state);
        assert!(
            !state.config.honor_game_colours,
            "the archive has no colours to give; a reload that reads two files it is \
             not written in must not overrule it"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// …and it survives as a ONE-RUN value. The pin is what keeps a fact about
    /// this launch's artwork out of the user's global file, so a settings save
    /// afterwards — the story browser's "remember this directory?" prompt is
    /// enough — must leave the key exactly as the user wrote it.
    #[test]
    fn the_artwork_force_off_never_reaches_the_global_config() {
        let (mut state, dir, cfg_path) = state_after_the_artwork_force_off("mono-global", true);
        reload_style(&mut state);
        assert!(state.config.one_run.holds(crate::config::keys::HONOR_GAME_COLOURS));

        crate::config::write_config_file(&state.config).unwrap();
        let back = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(
            toml::from_str::<crate::config::Config>(&back).unwrap().honor_game_colours,
            "the GLOBAL file must still say true — one archive spoke, not the user: {back}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The other honour mode: a user whose global file already says `false` gets no
    /// force-off at all (`startup.rs` guards on the live value), nothing is pinned,
    /// and their own `false` persists as it always has.
    #[test]
    fn a_global_false_is_untouched_by_the_artwork_path() {
        let (mut state, dir, cfg_path) = state_after_the_artwork_force_off("mono-base-off", false);
        reload_style(&mut state);
        assert!(!state.config.honor_game_colours, "the user's own base stands");
        assert!(
            !state.config.one_run.holds(crate::config::keys::HONOR_GAME_COLOURS),
            "nothing one-run is in force, so nothing is pinned"
        );
        crate::config::write_config_file(&state.config).unwrap();
        let back = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(
            !toml::from_str::<crate::config::Config>(&back).unwrap().honor_game_colours,
            "a global choice persists as always: {back}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The pin must never become a blanket ban on writing the key. A deliberate
    /// choice — `/set-game-colours`, or the settings panel's row edit, both of
    /// which clear `artwork_declines_colours` — outranks a guess about a machine,
    /// stays put across the next reload, AND reaches the file.
    #[test]
    fn a_deliberate_choice_outranks_the_artwork_and_still_persists() {
        let (mut state, dir, cfg_path) = state_after_the_artwork_force_off("mono-user", true);
        reload_style(&mut state);
        assert!(!state.config.honor_game_colours);

        // `/set-game-colours on`: the sidecar is written and the guess ends.
        crate::styles::write_per_game_honor(&state.game_dir, Some(true)).unwrap();
        state.artwork_declines_colours = false;
        reload_style(&mut state);
        assert!(state.config.honor_game_colours, "the player settled it by hand");

        // And the settings panel's global edit: the row edit releases the pin, so
        // the value it leaves is the user's and persists like any other setting.
        crate::styles::write_per_game_honor(&state.game_dir, None).unwrap();
        reload_style(&mut state);
        state.config.one_run.release(crate::config::keys::HONOR_GAME_COLOURS);
        state.config.honor_game_colours = false;
        state.honor_game_colours_base = false;
        crate::config::write_config_file(&state.config).unwrap();
        let back = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(
            !toml::from_str::<crate::config::Config>(&back).unwrap().honor_game_colours,
            "a deliberate global choice must still reach the file: {back}"
        );
        // …and a later reload keeps it, rather than recomputing it back on.
        reload_style(&mut state);
        assert!(!state.config.honor_game_colours, "the user's choice is not undone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_builds_theme_from_new_schema_files() {
        use ratatui::style::Color;
        let dir = temp_dir("theme-newschema");
        let global = dir.join("style.toml");
        std::fs::write(&global, "[roles]\naccent = { fg = \"green\" }\n").unwrap();
        let game_dir = dir.join("game.save");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(game_dir.join("style.toml"), "[roles]\naccent = { fg = \"red\" }\n").unwrap();

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.style = Some(global.to_string_lossy().to_string());
        state.game_dir = game_dir.clone();

        let outcome = reload_style(&mut state);
        assert!(matches!(outcome, ReloadOutcome::Reloaded { .. }));
        let r = state.colors.theme.get("accent");
        assert_eq!(r.style.fg, Some(Color::Red), "per-game [roles] wins");
        assert_eq!(r.provenance, crate::theme::resolve::Provenance::PerGame);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_layers_per_game_map_glyph_presets_over_the_global() {
        // The `[map]` glyph knobs must layer through real files exactly like
        // colours do (SQ-0557/0558): the per-game file wins where it speaks, the
        // global stands where it is silent.
        let dir = temp_dir("map-presets");
        let global = dir.join("style.toml");
        std::fs::write(&global, "[map]\nbox_style = \"double\"\ndiagonal_corners = false\n").unwrap();
        let game_dir = dir.join("game.save");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(game_dir.join("style.toml"), "[map]\nbox_style = \"ascii\"\n").unwrap();

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.style = Some(global.to_string_lossy().to_string());
        state.game_dir = game_dir.clone();

        assert!(matches!(reload_style(&mut state), ReloadOutcome::Reloaded { .. }));
        assert_eq!(
            state.symbols.room_normal,
            crate::symbols::BoxStyle::preset("ascii").unwrap(),
            "per-game box_style wins"
        );
        assert!(!state.symbols.diagonal_corners, "global diagonal_corners stands");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── SQ-0510: the probed terminal page reaches the theme's chrome roles ───

    /// The user's reported configuration exactly: no `style.toml`, no scheme —
    /// just an empty user dir and whatever the OSC 10/11 probe answered.
    fn state_with_probe(
        tag: &str,
        probe: crate::term_colors::TermDefaultColors,
    ) -> (AppState, std::path::PathBuf) {
        let dir = temp_dir(tag);
        let _ = std::fs::remove_file(dir.join("style.toml"));
        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.style = None; // no style.toml, no [colors] scheme
        state.term_default_colors = probe;
        match reload_style(&mut state) {
            ReloadOutcome::Reloaded { .. } => {}
            ReloadOutcome::Failed { msg } => panic!("the default look must load: {msg}"),
        }
        (state, dir)
    }

    fn rgba(r: u8, g: u8, b: u8) -> image::Rgba<u8> {
        image::Rgba([r, g, b, 255])
    }

    #[test]
    fn reload_seeds_the_chrome_roles_from_an_answered_terminal_probe() {
        // The bug: Anchorhead's status bar painted a BLACK band on a terminal
        // whose background is not black. The story sets no colours at all — the
        // black came from `Roles::terminal_default`'s hard-coded chrome, because
        // the OSC probe's answer was stored in AppState and never handed to the
        // theme. `reload_style` is the one place the theme is built (startup,
        // /reload-style, the style watcher and the per-game overlay all funnel
        // here), so the seeding has to land — and be observable — here.
        use ratatui::style::Color;
        let (state, dir) = state_with_probe(
            "probe-on",
            crate::term_colors::TermDefaultColors {
                fg: Some(rgba(0x58, 0x6e, 0x75)),
                bg: Some(rgba(0xfd, 0xf6, 0xe3)), // Solarized Light: emphatically not black
            },
        );

        let uw = state.colors.theme.get("upper_window").style;
        assert_ne!(uw.bg, Some(Color::Black), "no black band on a non-black terminal");
        assert_eq!(uw.bg, Some(Color::Rgb(0xfd, 0xf6, 0xe3)), "the upper window follows the terminal page");
        assert_eq!(uw.fg, Some(Color::Rgb(0x58, 0x6e, 0x75)));
        assert_eq!(state.colors.theme.get("chrome").style.bg, Some(Color::Rgb(0xfd, 0xf6, 0xe3)));
        assert_eq!(state.colors.theme.get("status_bar").style.bg, Some(Color::Rgb(0xfd, 0xf6, 0xe3)));
        // The out-of-box accents are untouched — this is a page fix, not a repaint.
        assert_eq!(state.colors.theme.get("panel.border").style.fg, Some(Color::Cyan));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_keeps_todays_behaviour_when_the_terminal_answers_nothing_or_half() {
        // A terminal that never answers (or answers only one channel) must be
        // byte-for-byte what it is today: the concrete terminal-default roles,
        // chrome included. A probed ink is never mixed onto a guessed page.
        use ratatui::style::Color;
        let cases = [
            ("probe-off", crate::term_colors::TermDefaultColors::default()),
            (
                "probe-fg-only",
                crate::term_colors::TermDefaultColors { fg: Some(rgba(0x58, 0x6e, 0x75)), bg: None },
            ),
            (
                "probe-bg-only",
                crate::term_colors::TermDefaultColors { fg: None, bg: Some(rgba(0xfd, 0xf6, 0xe3)) },
            ),
        ];
        for (tag, probe) in cases {
            let (state, dir) = state_with_probe(tag, probe);
            let uw = state.colors.theme.get("upper_window").style;
            assert_eq!(uw.bg, Some(Color::Black), "{tag}: unchanged fallback");
            assert_eq!(uw.fg, Some(Color::White), "{tag}: unchanged fallback");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_configured_scheme_still_wins_over_the_probe() {
        // The probe only fills the gap left by "no scheme configured". A user who
        // picked a scheme gets that scheme's page, whatever the terminal says.
        use ratatui::style::Color;
        let dir = temp_dir("probe-scheme");
        let path = dir.join("style.toml");
        std::fs::write(&path, "version = 1\nscheme = \"tomorrow-night\"\n").unwrap();

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.style = Some(path.to_string_lossy().to_string());
        state.term_default_colors = crate::term_colors::TermDefaultColors {
            fg: Some(rgba(0, 0, 0)),
            bg: Some(rgba(255, 255, 255)),
        };
        assert!(matches!(reload_style(&mut state), ReloadOutcome::Reloaded { .. }));

        let bg = state.colors.theme.get("upper_window").style.bg;
        assert_ne!(bg, Some(Color::Rgb(255, 255, 255)), "the probe must not overwrite a chosen scheme");
        assert_eq!(bg, Some(Color::Rgb(0x1d, 0x1f, 0x21)), "tomorrow-night's own page");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── SQ-0663: theme resolution warnings ride the same reload notice path ──

    #[test]
    fn reload_surfaces_a_theme_warning_for_a_typo_d_parent() {
        // Falsification (a)/(b): with the SQ-0663 surfacing removed, this fails
        // because `warnings` never carries a line naming the typo.
        let dir = temp_dir("theme-warn");
        let global = dir.join("style.toml");
        std::fs::write(&global, "[panel]\ntitle = { parent = \"acent\" }\n").unwrap();

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.style = Some(global.to_string_lossy().to_string());

        match reload_style(&mut state) {
            ReloadOutcome::Reloaded { warnings } => {
                assert!(
                    warnings.iter().any(|w| w.contains("panel.title") && w.contains("acent")),
                    "theme warning must ride the reload's warnings: {:?}",
                    warnings
                );
            }
            ReloadOutcome::Failed { msg } => panic!("expected Reloaded, got Failed({msg})"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_stays_quiet_when_the_theme_resolves_cleanly() {
        // The other half of the falsification: a reload whose style.toml has no
        // `parent` issues must not manufacture a warning out of nothing, and a
        // SECOND clean reload must not re-announce anything either.
        let dir = temp_dir("theme-warn-clean");
        let global = seed_style(&dir);

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.style = Some(global.to_string_lossy().to_string());

        for _ in 0..2 {
            match reload_style(&mut state) {
                ReloadOutcome::Reloaded { warnings } => {
                    assert!(warnings.is_empty(), "clean style.toml must carry no warnings: {:?}", warnings);
                }
                ReloadOutcome::Failed { msg } => panic!("expected Reloaded, got Failed({msg})"),
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
