//! SQ-1564: in command-bar mode the game's own read prompt is stripped, so the
//! bar's `> ` — and the host's `> cmd` echo above it — is the only prompt the
//! player sees. Before this, every prompt shape but a lone `>` on its own line
//! was left in the transcript, sitting right above the echoed command.
//!
//! | fixture | release | frame | prompt the game printed | read |
//! |---|---|---|---|---|
//! | `bureaucracy-r116-s870602.z4` | 116 / 870602 | boot | `\n>>` | line |
//! | `beyondzork-r57-s871221.z5` | 57 / 871221 | boot | `[Please type YES or NO.] >` | line |
//! | `Bronze.zblorb` | Inform 7 3K27 | boot | `…fiction before? >` | line |
//! | `CounterfeitMonkey-11.gblorb` | 11 | boot | `Can you hear me? >> ` | line |
//! | `Fairest.gblorb` | — | boot + SPACE | `…starting the game?>` | line |
//! | `borderzone-r9-s871008.z5` | 9 / 871008 | boot | `…(R)estore? >` | **char** |
//!
//! Border Zone is the deliberate exception: its chapter question is a
//! `read_char`, where the command bar draws no prompt and the host echoes
//! nothing, so the game's `>` is the only cue and is KEPT.
//!
//! Every case also boots inline-prompt mode (`command_bar` off), which must
//! keep the game's prompt exactly as printed. `stories/` is gitignored, so each
//! case skips vacuously without its fixture.

use std::path::PathBuf;

use app::config::Config;
use app::engine::KeyInput;
use app::host::{boot_story, BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts};
use app::launch_options::LaunchOverrides;
use app::session::InputKind;

fn story(name: &str) -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(name);
    if p.exists() {
        Some(p)
    } else {
        eprintln!("skipping: stories/{name} not present");
        None
    }
}

/// Boot `path` headlessly, fresh (no resume), with the command bar on or off.
fn boot(path: PathBuf, command_bar: bool) -> BootedStory {
    let home = app::scratch_dir("sq1564");
    let overrides = LaunchOverrides::default();
    let cfg = Config {
        user_dir: home.clone(),
        config_file: home.join("config.toml"),
        random_seed: Some(1),
        command_bar,
        ..Config::default()
    };
    let req = BootRequest {
        story_path: path,
        disk_entry: None,
        overrides: &overrides,
        cfg,
        data_base: home.join("saves"),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
    };
    boot_story(req, &mut QuietBoot).expect("the story boots headlessly")
}

/// The boot transcript's text, trailing spaces trimmed.
fn boot_text(b: &BootedStory) -> String {
    b.state.transcript.join("\n").trim_end_matches([' ', '\t']).to_owned()
}

/// Boot `name` both ways and check the tail of its boot transcript: command-bar
/// mode ends in `stripped`, inline mode in `stripped` + `prompt`.
fn check_boot(name: &str, pending: InputKind, stripped: &str, prompt: &str) {
    let Some(path) = story(name) else { return };
    let bar = boot(path.clone(), true);
    assert_eq!(bar.session.pending_input(), pending, "{name}: the boot stops at the read this case is about");
    let t = boot_text(&bar);
    assert!(t.ends_with(stripped), "{name} (command bar): expected …{stripped:?}, got …{:?}", tail(&t));
    let inline = boot(path, false);
    let t = boot_text(&inline);
    let kept = format!("{stripped}{prompt}");
    assert!(t.ends_with(&kept), "{name} (inline): expected …{kept:?}, got …{:?}", tail(&t));
}

fn tail(t: &str) -> String {
    let n = t.chars().count();
    t.chars().skip(n.saturating_sub(80)).collect()
}

#[test]
fn bureaucracy_run_alone_on_its_line() {
    check_boot("bureaucracy-r116-s870602.z4", InputKind::Line, "to start the game anew.", "\n>>");
}

#[test]
fn beyond_zork_yes_or_no() {
    check_boot("beyondzork-r57-s871221.z5", InputKind::Line, "[Please type YES or NO.]", " >");
}

#[test]
fn bronze_played_before() {
    check_boot("Bronze.zblorb", InputKind::Line, "Have you played interactive fiction before?", " >");
}

#[test]
fn counterfeit_monkey_can_you_hear_me() {
    check_boot("CounterfeitMonkey-11.gblorb", InputKind::Line, "Can you hear me?", " >>");
}

#[test]
fn border_zone_chapter_question_is_a_char_read_and_keeps_its_prompt() {
    // Kept in BOTH modes: the bar hides its own prompt at a read_char.
    check_boot(
        "borderzone-r9-s871008.z5",
        InputKind::Char,
        "Which chapter would you like to play: 1, 2, 3, or (R)estore? >",
        "",
    );
}

#[test]
fn fairest_helpful_information_question() {
    let Some(path) = story("Fairest.gblorb") else { return };
    let question = "Would you like to read some helpful information before starting the game?";
    for command_bar in [true, false] {
        let mut b = boot(path.clone(), command_bar);
        assert_eq!(b.session.pending_input(), InputKind::Char, "Fairest opens on 'press SPACE'");
        let r = b.session.submit_key(KeyInput::Char(' ')).expect("SPACE is taken");
        assert_eq!(b.session.pending_input(), InputKind::Line, "the question is a line read");
        let want = if command_bar { question.to_owned() } else { format!("{question}>") };
        assert!(
            r.transcript.trim_end().ends_with(&want),
            "Fairest (command_bar={command_bar}): expected …{want:?}, got …{:?}",
            tail(&r.transcript)
        );
    }
}
