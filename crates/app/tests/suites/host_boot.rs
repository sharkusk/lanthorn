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
use app::host::{boot_story, BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts};
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
        data_base: data_base.to_path_buf(),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
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
        data_base: home.join("saves"),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
    };
    let err = boot_story(req, &mut QuietBoot).err().expect("a missing story does not boot");
    assert!(err.0.contains("cannot read"), "says why: {err}");
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
