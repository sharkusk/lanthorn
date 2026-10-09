//! SQ-1537: a story boots through the library with no terminal.
//!
//! `app::host::boot_story` is the per-story build the TUI's `startup.rs` used to
//! do inline, stopping where the terminal begins. These cases drive it the way a
//! host that is not a terminal would — `TerminalFacts::default()` (no picker, no
//! OSC colours, no size), `QuietBoot` hooks, a scratch home — and hold it to what
//! a player sees on the TUI's first frame: a banner, and the story waiting for a
//! line.
//!
//! | fixture | engine | where |
//! |---|---|---|
//! | `Tangle.z5` | Z-machine v5 | fetched (`fixture_path`) |
//! | `chlorophyll.gblorb` | Glulx | fetched (`fixture_path`) |
//! | `crates/scott/tests/tiny_cave.dat` | Scott Adams | committed |
//!
//! A fetched fixture that is absent skips, as every real-media suite does.

use std::path::{Path, PathBuf};

use app::config::Config;
use app::engine::Engine;
use app::host::persist::exit_clear_resume_save;
use app::host::{
    boot_story, resume_source, BootRequest, BootedStory, LaunchFlags, QuietBoot, ResumeSlot,
    TerminalFacts,
};
use app::launch_options::LaunchOverrides;
use app::session::InputKind;

use crate::fixture_paths::fixture_path;

/// A config rooted in a scratch home, so nothing here reads or writes the real
/// `~/.lanthorn`. `random_seed` is pinned so two boots of one story are the same
/// run.
fn headless_config(home: &Path) -> Config {
    Config {
        user_dir: home.to_path_buf(),
        config_file: home.join("config.toml"),
        random_seed: Some(1),
        ..Config::default()
    }
}

/// Boot `story` exactly as a headless host would.
fn boot(story: PathBuf, cfg: Config, data_base: &Path) -> BootedStory {
    let overrides = LaunchOverrides::default();
    let req = BootRequest {
        story_path: story,
        disk_entry: None,
        overrides: &overrides,
        cfg,
        roots: app::data_roots::DataRoots::single(data_base.to_path_buf()),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
        fresh_start: false,
    };
    boot_story(req, &mut QuietBoot).expect("the story boots headlessly")
}

/// [`boot`], but with `fresh_start: true` (SQ-1626): the host-requested boot
/// that must attempt no resume at all, from either reserved slot.
fn boot_fresh_start(story: PathBuf, cfg: Config, data_base: &Path) -> BootedStory {
    let overrides = LaunchOverrides::default();
    let req = BootRequest {
        story_path: story,
        disk_entry: None,
        overrides: &overrides,
        cfg,
        roots: app::data_roots::DataRoots::single(data_base.to_path_buf()),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
        fresh_start: true,
    };
    boot_story(req, &mut QuietBoot).expect("the story boots headlessly")
}

/// The story's own text on screen, joined.
fn banner(b: &BootedStory) -> String {
    b.state.transcript.join("\n")
}

fn assert_ready(b: &BootedStory, what: &str) {
    let text = banner(b);
    assert!(!text.trim().is_empty(), "{what}: the banner is on screen");
    assert_eq!(b.session.pending_input(), InputKind::Line, "{what}: the story waits for a command");
    assert!(!b.session.has_quit(), "{what}: the story is running");
}

#[test]
fn a_zmachine_story_boots_with_no_terminal() {
    let story = fixture_path("Tangle.z5");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-boot-z");
    let b = boot(story, headless_config(&home), &home.join("saves"));
    assert_ready(&b, "Spider and Web");
    assert!(banner(&b).contains("Spider And Web"), "the Spider and Web banner: {}", banner(&b));
    // The starting room is on the map from the seed turn, as on the TUI's first frame.
    assert!(b.mapper.graph.current().is_some(), "the starting room is observed");
    assert!(b.game_dir.starts_with(home.join("saves")), "per-story storage under the data base");
    assert!(!b.resumed, "a first boot with no archive on disk did not resume anything (SQ-1545)");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_glulx_story_boots_with_no_terminal() {
    let story = fixture_path("chlorophyll.gblorb");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-boot-glulx");
    let b = boot(story, headless_config(&home), &home.join("saves"));
    assert_ready(&b, "Chlorophyll");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_scott_adams_story_boots_with_no_terminal() {
    let story = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scott/tests/tiny_cave.dat");
    let home = app::scratch_dir("host-boot-scott");
    let b = boot(story, headless_config(&home), &home.join("saves"));
    assert_ready(&b, "tiny_cave");
    let _ = std::fs::remove_dir_all(&home);
}

/// SQ-1556: a host's explicit "None, text only" launch choice
/// (`LaunchOverrides.images = Some(false)`) suppresses the whole picture
/// pipeline for that one boot — the same effect `--images off` has globally,
/// scoped to a single launch instead. `cfg.images` is the one gate every
/// engine's picture resolution reads (`PictureOverride::resolve_with_session`,
/// `PictSource::resolve_with_override` for Z-code, `resolve_pict_blorb` for
/// Glulx and Scott — see `host/boot.rs`), so asserting it is off after boot is
/// a direct proof that no picture source is in effect, for any engine.
///
/// Arthur is a real graphical Version 6 release with its own native/Blorb
/// artwork (`stories/`-only, so this skips vacuously without it, like every
/// other real-media suite).
#[test]
fn the_text_only_override_boots_a_picture_bearing_story_with_no_picture_source() {
    let story = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories/arthur-r74-s890714.z6");
    if !story.is_file() {
        eprintln!("SKIP: {} absent (stories/ not populated in this checkout)", story.display());
        return;
    }
    let home = app::scratch_dir("host-boot-text-only");
    let overrides = LaunchOverrides { images: Some(false), ..LaunchOverrides::default() };
    let req = BootRequest {
        story_path: story,
        disk_entry: None,
        overrides: &overrides,
        cfg: headless_config(&home),
        roots: app::data_roots::DataRoots::single(home.join("saves")),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
        fresh_start: false,
    };
    let b = boot_story(req, &mut QuietBoot).expect("boots with the text-only override");
    // Not `assert_ready`: a v6 game's intro screen does not necessarily land
    // its first text in `state.transcript`, and the point of this case is the
    // override, not v6's window model — `has_quit` is the engine-neutral proof
    // the boot actually ran the story rather than erroring out some other way.
    assert!(!b.session.has_quit(), "the story is running, not stuck at an error");
    assert!(
        !b.state.config.images,
        "LaunchOverrides.images forced pictures off for this launch, even though \
         the config default (and Arthur's own artwork) would otherwise draw them"
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// The baseline the case above is falsified against: with no override, the
/// same story boots with pictures still on (the config default).
#[test]
fn without_the_override_the_config_default_for_images_is_unchanged() {
    let story = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories/arthur-r74-s890714.z6");
    if !story.is_file() {
        eprintln!("SKIP: {} absent (stories/ not populated in this checkout)", story.display());
        return;
    }
    let home = app::scratch_dir("host-boot-images-default");
    let b = boot(story, headless_config(&home), &home.join("saves"));
    assert!(!b.session.has_quit(), "the story is running, not stuck at an error");
    assert!(b.state.config.images, "no override: the config default (on) is untouched");
    let _ = std::fs::remove_dir_all(&home);
}

/// A story that cannot be read is an error the host gets back, not a process exit.
#[test]
fn an_unreadable_story_is_an_error_not_an_exit() {
    let home = app::scratch_dir("host-boot-missing");
    let overrides = LaunchOverrides::default();
    let req = BootRequest {
        story_path: home.join("no-such-story.z5"),
        disk_entry: None,
        overrides: &overrides,
        cfg: headless_config(&home),
        roots: app::data_roots::DataRoots::single(home.join("saves")),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
        fresh_start: false,
    };
    let err = boot_story(req, &mut QuietBoot).err().expect("a missing story does not boot");
    assert!(err.0.contains("cannot read"), "says why: {err}");
    let _ = std::fs::remove_dir_all(&home);
}

// ── SQ-1563: a launch-time "Game colours" override survives a later reload ─────

/// `LaunchOverrides.honor_game_colours` is the launch-options dialog's
/// un-persisted per-launch choice (SQ-1532) — the same field the TUI's own
/// launch dialog fills in. The bug: `boot_story` applied it to `cfg` and then
/// called `reload::reload_style` itself (the post-IFID reload, before ever
/// returning), whose honour-key recompute knows nothing about a dialog choice
/// that never touched disk and silently overwrites it — so the override was
/// already gone by the time the player saw their first frame. Checked twice
/// to prove the fix holds up: right after `boot_story` returns (where the
/// bug already bites, since boot's own reload already ran), and after one
/// MORE explicit `reload_style` call standing in for a later mid-session
/// recompute (a live `/reload-style`, the style watcher, …) — the case a
/// fix that only patches the boot-time snapshot, rather than the hold
/// `reload_style` itself consults, would still fail.
#[test]
fn a_launch_dialog_honour_override_survives_a_second_style_reload() {
    let story = fixture_path("Tangle.z5");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-boot-honour-override");
    let cfg = headless_config(&home);
    assert!(cfg.honor_game_colours, "premise: the config default is on");

    let overrides = LaunchOverrides { honor_game_colours: Some(false), ..LaunchOverrides::default() };
    let req = BootRequest {
        story_path: story,
        disk_entry: None,
        overrides: &overrides,
        cfg,
        roots: app::data_roots::DataRoots::single(home.join("saves")),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
        fresh_start: false,
    };
    let mut b = boot_story(req, &mut QuietBoot).expect("boots with the launch override");
    assert!(
        !b.state.config.honor_game_colours,
        "the launch dialog's off choice is in force right after boot"
    );

    // The regression: a SECOND reload (anything that calls `reload_style` mid
    // session) must not recompute the key back to the config default.
    app::reload::reload_style(&mut b.state);
    assert!(
        !b.state.config.honor_game_colours,
        "the launch-dialog override must survive a later style reload, not just the boot-time one"
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// A subsequent settings-screen edit of the honour row must still end the
/// hold — exactly as it already does for the `--game-colours` CLI flag
/// (`host::settings::apply`, SQ-1559) — whether the hold came from a CLI flag
/// or from a launch-dialog override.
#[test]
fn a_settings_edit_ends_the_launch_dialog_honour_hold() {
    let story = fixture_path("Tangle.z5");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-boot-honour-settings");
    let overrides = LaunchOverrides { honor_game_colours: Some(false), ..LaunchOverrides::default() };
    let req = BootRequest {
        story_path: story,
        disk_entry: None,
        overrides: &overrides,
        cfg: headless_config(&home),
        roots: app::data_roots::DataRoots::single(home.join("saves")),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
        fresh_start: false,
    };
    let mut b = boot_story(req, &mut QuietBoot).expect("boots with the launch override");
    assert!(!b.state.config.honor_game_colours, "the override is in force after boot");
    assert!(
        b.state.game_colours_cli.is_some(),
        "the launch override rides the same hold the CLI flag uses"
    );

    // The player opens settings and turns the row back on: the working copy
    // carries the edit and releases the pin, exactly what the settings screen
    // builds (see `host::settings::apply`'s doc comment).
    let mut working = b.state.config.clone();
    working.honor_game_colours = true;
    working.one_run.release(app::config::keys::HONOR_GAME_COLOURS);
    app::host::settings::apply(&mut b.state, working, None);

    assert!(b.state.config.honor_game_colours, "the deliberate edit takes effect");
    assert!(
        b.state.game_colours_cli.is_none(),
        "the edit ends the hold, exactly as it already does for a CLI flag"
    );

    // And the edit survives a later reload, rather than the now-absent hold
    // letting the config default creep back in some other way.
    app::reload::reload_style(&mut b.state);
    assert!(b.state.config.honor_game_colours, "the edit is not undone by a later reload");
    let _ = std::fs::remove_dir_all(&home);
}

// ── Resume ───────────────────────────────────────────────────────────────────

/// One move, applied the way a host applies it: submit, push the reply, and feed
/// the map.
fn play(b: &mut BootedStory, cmd: &str) {
    let result = b.session.submit(cmd);
    b.state.push_transcript_kind(&format!("> {cmd}"), app::state::TranscriptKind::Input);
    b.state
        .push_transcript_runs(&result.transcript, app::state::TranscriptKind::Story, &result.transcript_runs);
    app::session::apply_turn(&mut b.mapper, cmd, &result, &mut b.state.death_watch);
    b.state.turns += 1;
}

/// Write the resume archive the way the TUI's exit save does.
fn write_resume_archive(b: &mut BootedStory) {
    let meta = app::archive::Meta {
        format_version: app::archive::CURRENT_FORMAT_VERSION,
        ifid: Some(b.ifid.clone()),
        name: None,
        turns: b.state.turns,
        saved_at: String::new(),
        location: b.state.current_room_name.clone(),
        score: None,
        trigger: app::archive::SaveTrigger::HostState,
        source: b.state.source.clone(),
    };
    let screen = app::engine_helpers::zvm_session_opt(&*b.session).map(|z| z.machine.screen.clone());
    app::archive::save_archive_meta_pics(
        &b.arc_file,
        &b.mapper,
        &b.session.save_state(),
        screen.as_ref(),
        b.session.aux_data(),
        meta,
        &app::archive::SessionRecord::of(&b.state),
        &[],
        None,
        None,
    )
    .expect("the resume archive writes");
}

/// [`write_resume_archive`] to an arbitrary path with an explicit `saved_at`
/// (SQ-1624): boot compares the two reserved slots by this timestamp, so a
/// case proving "the newer one wins" needs to set it directly rather than
/// relying on wall-clock ordering between two fast in-process writes.
fn write_resume_archive_to(b: &mut BootedStory, path: &Path, saved_at: &str) {
    let meta = app::archive::Meta {
        format_version: app::archive::CURRENT_FORMAT_VERSION,
        ifid: Some(b.ifid.clone()),
        name: None,
        turns: b.state.turns,
        saved_at: saved_at.to_string(),
        location: b.state.current_room_name.clone(),
        score: None,
        trigger: app::archive::SaveTrigger::HostState,
        source: b.state.source.clone(),
    };
    let screen = app::engine_helpers::zvm_session_opt(&*b.session).map(|z| z.machine.screen.clone());
    app::archive::save_archive_meta_pics(
        path,
        &b.mapper,
        &b.session.save_state(),
        screen.as_ref(),
        b.session.aux_data(),
        meta,
        &app::archive::SessionRecord::of(&b.state),
        &[],
        None,
        None,
    )
    .expect("the resume archive writes");
}

fn tail(b: &BootedStory, n: usize) -> Vec<String> {
    let t = &b.state.transcript;
    t[t.len().saturating_sub(n)..].to_vec()
}

/// The room the MAP stands in — what a player sees highlighted.
fn here(b: &BootedStory) -> Option<mapper::graph::RoomId> {
    b.mapper.graph.current()
}

/// Boot, play two moves, write the resume archive, boot again: the second boot
/// resumes where the first left off — same room, same transcript tail, same map —
/// and, because a restore bug surfaces one move AFTER the restore, the next move
/// from the resumed game reads exactly as it does from the original.
#[test]
fn a_second_boot_resumes_the_first_and_plays_on_identically() {
    let story = fixture_path("Tangle.z5");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-boot-resume");
    let data_base = home.join("saves");

    let mut first = boot(story.clone(), headless_config(&home), &data_base);
    assert!(!first.resumed, "the first boot found no archive to resume from (SQ-1545)");
    let start = here(&first);
    play(&mut first, "look");
    play(&mut first, "south");
    assert_ne!(here(&first), start, "premise: the two moves went somewhere");
    write_resume_archive(&mut first);

    let mut second = boot(story, headless_config(&home), &data_base);
    assert!(second.resumed, "the second boot restored the archive the first one wrote (SQ-1545)");
    assert_eq!(here(&second), here(&first), "the resumed game stands where the first one stopped");
    assert_eq!(tail(&second, 6), tail(&first, 6), "the resumed transcript ends where the first one did");
    assert_eq!(second.state.turns, first.state.turns, "the turn counter comes back with it");
    assert_eq!(
        second.mapper.graph.rooms().count(),
        first.mapper.graph.rooms().count(),
        "the map comes back with it"
    );
    assert_eq!(second.session.pending_input(), InputKind::Line);

    // Perturb before trusting it: one more move from each.
    play(&mut first, "north");
    play(&mut second, "north");
    assert_eq!(here(&second), here(&first), "the next move lands in the same room");
    assert_eq!(tail(&second, 4), tail(&first, 4), "and prints the same reply");

    let _ = std::fs::remove_dir_all(&home);
}

/// SQ-1624: two reserved slots can each hold a resume point now — the
/// auto-save's `default.lanthorn` (`arc_file`) and the manual quick-save's
/// `quick-save.lanthorn` (`quick_save_file`). Boot picks whichever was saved
/// more recently (`Meta::saved_at`), so a quick-save that happens to be NEWER
/// than the auto-save wins, rather than the auto-save slot winning
/// unconditionally just because it existed first / is checked first.
#[test]
fn boot_auto_load_prefers_the_newer_of_the_two_reserved_slots() {
    let story = fixture_path("Tangle.z5");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-boot-newer-slot");
    let data_base = home.join("saves");

    let mut first = boot(story.clone(), headless_config(&home), &data_base);
    play(&mut first, "look");
    // The OLDER write lands in the auto-save slot.
    let arc_path = first.arc_file.clone();
    write_resume_archive_to(&mut first, &arc_path, "2020-01-01T00:00:00Z");
    let room_at_older = here(&first);

    play(&mut first, "south");
    // The NEWER write lands in the quick-save slot, at a DIFFERENT room, so
    // the two are distinguishable.
    let quick_save_path = first.quick_save_file.clone();
    write_resume_archive_to(&mut first, &quick_save_path, "2030-01-01T00:00:00Z");
    let room_at_newer = here(&first);
    assert_ne!(room_at_older, room_at_newer, "premise: the two saves differ");

    let second = boot(story, headless_config(&home), &data_base);
    assert!(second.resumed, "a save exists to resume from (SQ-1545)");
    assert_eq!(
        second.resume_source_file, quick_save_path,
        "the NEWER slot (quick-save) is the one boot actually read from, not arc_file unconditionally"
    );
    assert_eq!(here(&second), room_at_newer, "and its room is the newer save's room, not the older auto-save's");

    let _ = std::fs::remove_dir_all(&home);
}

/// The single-file case (SQ-1624): with only the auto-save slot present (no
/// quick-save.lanthorn at all — today's shape, before this quest, and still
/// the common case for a player who never hits Ctrl+S), boot must still
/// auto-load it exactly as before. `a_second_boot_resumes_the_first_and_plays_on_identically`
/// above already covers this end-to-end; this case additionally pins
/// `resume_source_file` directly, the fact the newer-of-two comparison reads.
#[test]
fn boot_auto_load_falls_back_to_the_only_slot_present() {
    let story = fixture_path("Tangle.z5");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-boot-single-slot");
    let data_base = home.join("saves");

    let mut first = boot(story.clone(), headless_config(&home), &data_base);
    play(&mut first, "look");
    write_resume_archive(&mut first);
    assert!(!first.quick_save_file.exists(), "premise: no quick-save slot was ever written");

    let second = boot(story, headless_config(&home), &data_base);
    assert!(second.resumed, "the only slot present is still auto-loaded (SQ-1545)");
    assert_eq!(
        second.resume_source_file, second.arc_file,
        "with no quick-save file, the auto-save slot is the resume source"
    );

    let _ = std::fs::remove_dir_all(&home);
}

/// SQ-1626 Fix 1: `exit_clear_resume_save` (a clean, game-driven quit) rewrites
/// `default.lanthorn` with a fresh `Meta::saved_at` but an EMPTY resume point
/// (see its own doc). Comparing `saved_at` alone — the pre-SQ-1626 rule — would
/// let that freshly-cleared, resume-less autosave always beat an
/// older-but-real quick save, defeating the entire point of giving Ctrl+S its
/// own slot. A slot that actually HAS a resume point must win regardless of
/// timestamp.
#[test]
fn boot_resumes_the_quick_save_that_survives_a_clean_quit() {
    let story = fixture_path("Tangle.z5");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-boot-quicksave-survives-quit");
    let data_base = home.join("saves");

    let mut first = boot(story.clone(), headless_config(&home), &data_base);
    play(&mut first, "look");
    let quick_save_room = here(&first);
    let quick_save_path = first.quick_save_file.clone();
    // The quick save is OLD, but it is the only slot with a real resume point.
    write_resume_archive_to(&mut first, &quick_save_path, "2020-01-01T00:00:00Z");

    // A clean, game-driven quit: `default.lanthorn` gets rewritten with a
    // brand-new `saved_at` (now) and no resume point.
    let outcome = exit_clear_resume_save(&mut *first.session, &first.mapper, &first.state, &first.ifid, &first.arc_file);
    assert!(matches!(outcome, app::host::persist::ExitSave::Saved), "the clean-quit rewrite itself succeeds: {outcome:?}");
    assert!(first.arc_file.exists(), "premise: the clean quit left a (resume-less) archive behind");

    let second = boot(story, headless_config(&home), &data_base);
    assert!(second.resumed, "the quick save's real resume point is still there to resume (SQ-1626)");
    assert_eq!(
        second.resume_source_file, quick_save_path,
        "the quick save wins despite being OLDER, because the autosave slot has no resume point at all"
    );
    assert_eq!(here(&second), quick_save_room, "and its room is the quick save's room");

    let _ = std::fs::remove_dir_all(&home);
}

/// SQ-1626 Fix 2: a host-requested `fresh_start` boot must not resume from
/// EITHER reserved slot, must not even load the mapper/aux/command-history
/// from whichever slot would otherwise have won, and must leave a quick save
/// on disk completely untouched — ready to resume on the NEXT ordinary boot.
#[test]
fn fresh_start_resumes_nothing_and_leaves_the_quick_save_untouched() {
    let story = fixture_path("Tangle.z5");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-boot-fresh-start");
    let data_base = home.join("saves");

    let mut first = boot(story.clone(), headless_config(&home), &data_base);
    play(&mut first, "look");
    // Explore a SECOND room before saving, so the quick save's map is
    // distinguishable from the one room boot always observes on its own
    // (`Observe the starting room…`, `boot_story`) — a fresh boot with no
    // resume attempted still shows that one room; the question is whether it
    // ALSO carries this save's second one.
    play(&mut first, "south");
    let explored_rooms = first.mapper.graph.rooms().count();
    assert!(explored_rooms >= 2, "premise: the quick save's map has more than just the starting room");
    let quick_save_path = first.quick_save_file.clone();
    write_resume_archive_to(&mut first, &quick_save_path, "2020-01-01T00:00:00Z");
    assert!(!first.arc_file.exists(), "premise: no autosave was ever written, only the quick save");
    let quick_save_bytes_before = std::fs::read(&quick_save_path).expect("the quick save is readable before the fresh-start boot");

    let second = boot_fresh_start(story, headless_config(&home), &data_base);
    assert!(!second.resumed, "fresh_start must not resume ANYTHING, even though a real resume point exists (SQ-1626)");
    assert!(
        second.mapper.graph.rooms().count() < explored_rooms,
        "fresh_start must not load the mapper from whichever slot would otherwise have won: got {} rooms, same as the explored quick save",
        second.mapper.graph.rooms().count()
    );

    let quick_save_bytes_after = std::fs::read(&quick_save_path).expect("the quick save is still readable after the fresh-start boot");
    assert_eq!(
        quick_save_bytes_before, quick_save_bytes_after,
        "fresh_start must leave the quick save byte-for-byte untouched on disk"
    );
    assert!(!first.arc_file.exists(), "fresh_start must not write the autosave slot either");

    let _ = std::fs::remove_dir_all(&home);
}

/// SQ-1626 Fix 3: `resume_source` is the ONE place the resume-slot decision is
/// made, and `boot_story` itself calls it — so the two can never disagree.
/// Checked in the plain "no saves at all" case, and in the "clean quit
/// clears the autosave but the quick save survives" case from
/// `boot_resumes_the_quick_save_that_survives_a_clean_quit` above.
#[test]
fn resume_source_agrees_with_what_boot_story_actually_does() {
    let story = fixture_path("Tangle.z5");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }

    // Case 1: nothing saved at all.
    {
        let home = app::scratch_dir("host-boot-resume-source-none");
        let data_base = home.join("saves");
        let first = boot(story.clone(), headless_config(&home), &data_base);
        assert!(!first.resumed, "premise: a first-ever boot has nothing to resume");
        assert_eq!(
            resume_source(&first.game_dir),
            None,
            "resume_source agrees there is nothing to offer"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    // Case 2: a clean quit clears the autosave, but the quick save survives.
    {
        let home = app::scratch_dir("host-boot-resume-source-quicksave");
        let data_base = home.join("saves");
        let mut first = boot(story.clone(), headless_config(&home), &data_base);
        play(&mut first, "look");
        let quick_save_path = first.quick_save_file.clone();
        write_resume_archive_to(&mut first, &quick_save_path, "2020-01-01T00:00:00Z");
        let outcome = exit_clear_resume_save(&mut *first.session, &first.mapper, &first.state, &first.ifid, &first.arc_file);
        assert!(matches!(outcome, app::host::persist::ExitSave::Saved), "the clean-quit rewrite itself succeeds: {outcome:?}");

        assert_eq!(
            resume_source(&first.game_dir),
            Some(ResumeSlot::QuickSave),
            "resume_source picks the quick save, matching Fix 1"
        );

        let second = boot(story.clone(), headless_config(&home), &data_base);
        assert!(second.resumed);
        assert_eq!(
            second.resume_source_file,
            ResumeSlot::QuickSave.path(&first.game_dir),
            "and boot_story's own choice is exactly what resume_source predicted"
        );
        let _ = std::fs::remove_dir_all(&home);
    }
}

/// SQ-1626 Fix 4: `Meta::saved_at` is an RFC3339 string with one-second
/// resolution, so two saves in the same second are indistinguishable by
/// timestamp alone. On an exact tie, the quick save wins — it is the more
/// deliberate of the two saves.
#[test]
fn resume_source_breaks_a_same_second_tie_toward_the_quick_save() {
    let story = fixture_path("Tangle.z5");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-boot-same-second-tie");
    let data_base = home.join("saves");

    let mut first = boot(story.clone(), headless_config(&home), &data_base);
    play(&mut first, "look");
    let arc_path = first.arc_file.clone();
    write_resume_archive_to(&mut first, &arc_path, "2025-06-15T12:00:00Z");

    play(&mut first, "south");
    let room_at_quick_save = here(&first);
    let quick_save_path = first.quick_save_file.clone();
    write_resume_archive_to(&mut first, &quick_save_path, "2025-06-15T12:00:00Z");

    let second = boot(story, headless_config(&home), &data_base);
    assert!(second.resumed);
    assert_eq!(
        second.resume_source_file, quick_save_path,
        "on an exact same-second tie, the quick save wins"
    );
    assert_eq!(here(&second), room_at_quick_save);

    let _ = std::fs::remove_dir_all(&home);
}

/// SQ-1633: `Meta::source` is informational-only display metadata and must
/// have ZERO effect on which slot a boot resumes from. Mirrors
/// `resume_source_breaks_a_same_second_tie_toward_the_quick_save` exactly,
/// except the two writes carry DIFFERENT (and non-default) `source` values —
/// the tie-break must still land on the quick save, precisely as it does
/// with no `source` in play at all.
#[test]
fn resume_selection_ignores_source_even_on_a_same_second_tie() {
    let story = fixture_path("Tangle.z5");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-boot-source-ignored-tie");
    let data_base = home.join("saves");

    let mut first = boot(story.clone(), headless_config(&home), &data_base);
    play(&mut first, "look");
    first.state.source = app::archive::SaveSource {
        story_file: Some("Tangle (copy A).z5".to_string()),
        disk_entry: None,
        machine: Some(app::archive::MachineDto::Amiga),
    };
    let arc_path = first.arc_file.clone();
    write_resume_archive_to(&mut first, &arc_path, "2025-06-15T12:00:00Z");

    play(&mut first, "south");
    let room_at_quick_save = here(&first);
    first.state.source = app::archive::SaveSource {
        story_file: Some("Tangle (copy B).z5".to_string()),
        disk_entry: None,
        machine: None,
    };
    let quick_save_path = first.quick_save_file.clone();
    write_resume_archive_to(&mut first, &quick_save_path, "2025-06-15T12:00:00Z");

    assert_eq!(
        resume_source(&first.game_dir),
        Some(ResumeSlot::QuickSave),
        "the tie-break must still favour the quick save, exactly as with no source recorded at all"
    );

    let second = boot(story, headless_config(&home), &data_base);
    assert!(second.resumed);
    assert_eq!(
        second.resume_source_file, quick_save_path,
        "on an exact same-second tie, the quick save still wins regardless of either slot's source"
    );
    assert_eq!(here(&second), room_at_quick_save);

    let _ = std::fs::remove_dir_all(&home);
}

/// SQ-1634: `state.turns` isn't restored from the resumed archive until AFTER the
/// boot drain that re-observes the starting room's description and items (see
/// `host/boot.rs`'s own comment at that call site) — so on a resume, that drain
/// used to stamp everything it captured with the pre-restore `state.turns`
/// (always 0 on a fresh process) instead of the save's real turn count, losing
/// "last seen at move N" for the starting room and everything in it.
///
/// Real-game repro from the quest: Zork I r88, open the mailbox, take the
/// leaflet, four more no-op moves (six total) without leaving West of House,
/// save, exit, resume — the mailbox and the front door (both re-observed by
/// the resumed boot's own drain, independent of anything the transcript says)
/// must come back stamped with turn 6, not 0.
#[test]
fn a_resumed_boots_own_drain_stamps_the_restored_turn_not_zero() {
    let story = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/zork1-r88-s840726.z3");
    if !story.is_file() {
        eprintln!("SKIP: gitignored story missing at {}", story.display());
        return;
    }
    let home = app::scratch_dir("host-boot-resume-turn-stamp");
    let data_base = home.join("saves");

    let mut first = boot(story.clone(), headless_config(&home), &data_base);
    assert!(!first.resumed, "the first boot found no archive to resume from");
    let start_room = here(&first);
    play(&mut first, "open mailbox");
    play(&mut first, "take leaflet");
    play(&mut first, "wait");
    play(&mut first, "wait");
    play(&mut first, "wait");
    play(&mut first, "wait");
    assert_eq!(first.state.turns, 6, "premise: six moves played, all inside West of House");
    assert_eq!(here(&first), start_room, "premise: none of the six moves left the starting room");
    write_resume_archive(&mut first);

    let second = boot(story, headless_config(&home), &data_base);
    assert!(second.resumed, "the second boot restored the archive the first one wrote");
    assert_eq!(second.state.turns, 6, "the restored turn counter carries over");

    let mailbox = second
        .mapper
        .graph
        .items()
        .map(|(_, rec)| rec)
        .find(|rec| rec.name.to_lowercase().contains("mailbox"))
        .expect("the resumed boot's own drain re-observes the mailbox");
    assert_eq!(
        mailbox.last_seen_turn, 6,
        "the mailbox must come back stamped with the save's real turn count, not 0 (SQ-1634)"
    );

    let door = second
        .mapper
        .graph
        .items()
        .map(|(_, rec)| rec)
        .find(|rec| rec.name.to_lowercase().contains("door"))
        .expect("the resumed boot's own drain re-observes the front door");
    assert_eq!(
        door.last_seen_turn, 6,
        "the front door must come back stamped with the save's real turn count, not 0 (SQ-1634)"
    );

    let _ = std::fs::remove_dir_all(&home);
}

// ── SQ-1635: a loose known release shares a disk-mounted copy's game_dir ────

/// The end-to-end proof that `save_key_media.rs`'s unit-level pins do not by
/// themselves give: `boot_story` — the real chain `startup.rs` drives, not
/// `story_key_for` called in isolation — resolves a loose `zork1-r88-s840726.z3`
/// to the SAME `game_dir` as an Amiga floppy pressing the identical release
/// (`Zork I - The Great Underground Empire.adf`), when both are booted against
/// the same `data_base`. `stories/`-only, so this skips vacuously without it.
#[test]
fn a_loose_known_release_boots_into_the_same_game_dir_as_a_disk_mounted_copy() {
    let loose = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories/zork1-r88-s840726.z3");
    let disk =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories/Zork I - The Great Underground Empire.adf");
    if !loose.is_file() || !disk.is_file() {
        eprintln!("SKIP: stories/ not populated with both Zork I fixtures in this checkout");
        return;
    }
    let home = app::scratch_dir("host-boot-sq1635-unify");
    let data_base = home.join("saves");

    let b_loose = boot(loose, headless_config(&home), &data_base);
    assert_ready(&b_loose, "Zork I (loose)");
    let b_disk = boot(disk, headless_config(&home), &data_base);
    assert_ready(&b_disk, "Zork I (Amiga floppy)");

    assert_eq!(
        b_loose.game_dir, b_disk.game_dir,
        "a loose copy and a disk-mounted copy of the identical release must share one game_dir"
    );
    // And it is the DISK-STYLE key, not either fixture's own basename — proof
    // this really is the unification and not an accidental basename collision.
    let dir_name = b_loose.game_dir.file_name().and_then(|s| s.to_str()).unwrap_or_default();
    assert!(dir_name.starts_with("zork-i-r88-s840726"), "got {dir_name:?}");

    let _ = std::fs::remove_dir_all(&home);
}

/// The v6 half of the same proof, in the other direction: a loose, KNOWN
/// Version 6 release (`arthur-r74-s890714.z6`) still boots into its OWN
/// basename-keyed `game_dir`, exactly as before SQ-1635 — never unified with
/// the Amiga floppy's build-keyed directory, per this feature's documented v6
/// exclusion (`cli_host::storage`'s module docs).
#[test]
fn a_loose_known_version_six_release_keeps_its_own_basename_game_dir() {
    let loose = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories/arthur-r74-s890714.z6");
    if !loose.is_file() {
        eprintln!("SKIP: stories/arthur-r74-s890714.z6 absent in this checkout");
        return;
    }
    let home = app::scratch_dir("host-boot-sq1635-v6-excluded");
    let b = boot(loose, headless_config(&home), &home.join("saves"));
    assert!(!b.session.has_quit(), "the story is running");

    let dir_name = b.game_dir.file_name().and_then(|s| s.to_str()).unwrap_or_default();
    assert_eq!(
        dir_name, "arthur-r74-s890714.z6.save",
        "a loose Version 6 file keeps its basename game_dir, unaffected by SQ-1635"
    );

    let _ = std::fs::remove_dir_all(&home);
}

/// SQ-1743: resuming at Beyond Zork's character sheet (a room-shaped status line
/// that names the player, "Frank Booth") must not seed that name as the map's first
/// room — the seed on resume gets the same NameOnly corroboration check a live turn
/// gets. `stories/`-only; skips without it.
#[test]
fn resuming_at_the_beyond_zork_character_sheet_maps_no_non_room() {
    let story = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories/beyondzork-r57-s871221.z5");
    if !story.is_file() {
        eprintln!("SKIP: stories/beyondzork-r57-s871221.z5 absent in this checkout");
        return;
    }
    let home = app::scratch_dir("host-boot-sq1743");
    let data_base = home.join("saves");
    let mut first = boot(story.clone(), headless_config(&home), &data_base);
    let mut at_sheet = false;
    for cmd in ["yes", "begin", "", "", "", "", "", ""] {
        // SQ-1752: the sheet is no longer a LOCATION, so recognise it by the status line itself.
        let on_sheet = |b: &BootedStory| {
            app::engine_helpers::zvm_session_opt(&*b.session).is_some_and(|z| {
                zvm::location::status_line_room_name(&z.machine.screen.upper, z.machine.screen.upper_window_rows)
                    .as_deref()
                    == Some("Frank Booth")
            })
        };
        if on_sheet(&first) {
            at_sheet = true;
            break;
        }
        match first.session.pending_input() {
            InputKind::Char => {
                if let Some(r) = first.session.submit_key(app::engine::KeyInput::Enter) {
                    app::session::apply_turn(&mut first.mapper, "", &r, &mut first.state.death_watch);
                }
            }
            InputKind::Line => play(&mut first, cmd),
            _ => break,
        }
    }
    assert!(at_sheet, "premise: the probe reached the character sheet");
    assert!(first.session.current_location().is_none(), "SQ-1752: the sheet is not a location");
    assert_eq!(first.state.current_room_name, None, "SQ-1752: the host names no room at the sheet");
    assert_eq!(first.mapper.graph.rooms().count(), 0, "premise: the live game rejected the name");
    write_resume_archive(&mut first);

    // A fresh boot in its own home (so it auto-resumes nothing), then the TUI's
    // launch-resume (`main.rs` -> `apply_launch_resume`) applies the saved state.
    let home2 = app::scratch_dir("host-boot-sq1743-b");
    let mut second = boot(story, headless_config(&home2), &home2.join("saves"));
    let save = first.session.save_state();
    // The status line the name-only detector reads lives in the screen state.
    let screen = app::engine_helpers::zvm_session_opt(&*first.session).map(|z| z.machine.screen.clone());
    app::host::turn::apply_launch_resume(
        &save,
        Vec::new(),
        Vec::new(),
        screen,
        &mut *second.session,
        &mut second.mapper,
        &mut second.state,
        None,
        &first.arc_file,
    );
    assert!(
        second.session.current_location().is_none(),
        "the resumed game stands at the character sheet, which names no location (SQ-1752)"
    );
    let rooms: Vec<String> = second.mapper.graph.rooms().map(|r| r.name.clone()).collect();
    assert!(rooms.is_empty(), "the resumed map holds no non-room: {rooms:?}");
    let _ = std::fs::remove_dir_all(&home2);
    let _ = std::fs::remove_dir_all(&home);
}

/// SQ-1749: a fresh boot seeds the mapper only; `current_room_name` must be set
/// too, or a Save State / exit autosave before the first turn records no location.
#[test]
fn a_fresh_boot_sets_current_room_name_before_any_turn() {
    let story = fixture_path("Tangle.z5");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-boot-sq1749");
    let b = boot(story, headless_config(&home), &home.join("saves"));
    assert_eq!(b.state.turns, 0, "premise: no turn has been played");
    let room = b.state.current_room_name.clone();
    assert!(room.is_some(), "the opening room name is set on a fresh boot");
    assert_eq!(app::engine_helpers::save_summary(&*b.session, &b.state).0, room);
    let _ = std::fs::remove_dir_all(&home);
}

/// SQ-1742: a Dialog story has no Inform grammar table, and the SQ-1579 no-map
/// gate used to read that as "menu-driven", so `The Impossible Bottle` never
/// mapped through the real boot path (a bare `Mapper::default()` in
/// `nameonly_room_corroboration` could not see the gate). Answer the opening menu
/// with `1` five times, then `look`, `south`: Kitchen and the room south of it
/// must reach the map.
#[test]
fn a_dialog_story_maps_through_boot_story() {
    let story = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories/the-impossible-bottle.zblorb.blorb");
    if !story.is_file() {
        eprintln!("SKIP: stories/the-impossible-bottle.zblorb.blorb absent in this checkout");
        return;
    }
    let home = app::scratch_dir("host-boot-sq1742-dialog");
    let mut b = boot(story, headless_config(&home), &home.join("saves"));
    for cmd in ["1", "1", "1", "1", "1", "look", "south"] {
        play(&mut b, cmd);
    }
    let rooms: Vec<String> = b.mapper.graph.rooms().map(|r| r.name.clone()).collect();
    assert!(rooms.contains(&"Kitchen".to_string()), "the Bottle's opening room is mapped: {rooms:?}");
    assert!(
        rooms.contains(&"Smooth surface".to_string()),
        "walking south out of the Kitchen maps the next room: {rooms:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}
