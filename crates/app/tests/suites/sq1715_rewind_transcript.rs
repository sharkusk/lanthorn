//! SQ-1715: a rewind keeps the live transcript's text and styling, including the
//! output of game-driven turns (a key press) that record nothing.
//!
//! Before: `resume_from_turn` rebuilt the transcript from the per-command records,
//! so everything printed after a "press any key" -- and every style run -- was
//! gone from the rewound screen. Now a record stamps the transcript's length and
//! rewrite epoch, and a rewind under an unchanged epoch cuts the real transcript
//! back to it.
//!
//! Photopia (a manifest fixture, so this runs on CI) is driven with `no` to its
//! instructions question, then past two "press a key" pauses. Input counts are
//! named in each case. Every case pins both `honor_game_colours` modes.

use app::engine::{Engine, KeyInput};
use app::host::persist::{resume_from_turn, restore_file, save_state_now, ExitSave};
use app::host::{boot_story, finish_command_turn, BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts};
use app::launch_options::LaunchOverrides;
use app::session::InputKind;
use app::state::{StyleRun, TranscriptKind};
use std::sync::Arc;

use crate::fixture_paths::fixture_path;

/// Post-keypress text of the first pause (printed by a game-driven turn).
const AFTER_FIRST_KEY: &str = "Speeding down Montgomery Boulevard";
/// Post-keypress text of the second pause.
const AFTER_SECOND_KEY: &str = "You are Wendy Mackaye, first girl on the red planet.";

fn boot(home: &std::path::Path, honor: bool) -> BootedStory {
    let overrides = LaunchOverrides::default();
    let req = BootRequest {
        story_path: fixture_path("photopia.z5"),
        disk_entry: None,
        overrides: &overrides,
        cfg: app::config::Config {
            user_dir: home.to_path_buf(),
            config_file: home.join("config.toml"),
            random_seed: Some(1),
            enable_sound: false,
            auto_save: false,
            record_turn_history: true,
            honor_game_colours: honor,
            ..app::config::Config::default()
        },
        roots: app::data_roots::DataRoots::single(home.join("saves")),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
        fresh_start: true,
    };
    boot_story(req, &mut QuietBoot).expect("photopia boots headlessly")
}

/// A typed command through the path the TUI uses.
fn play(b: &mut BootedStory, cmd: &str) {
    assert!(matches!(b.session.pending_input(), InputKind::Line), "premise: a line is awaited for `{cmd}`");
    let result = b.session.submit(cmd);
    let mut tidy = 0u32;
    let _ = finish_command_turn(cmd, true, result, &mut b.state, &mut b.mapper, &mut *b.session, &b.game_dir, &b.ifid, &b.arc_file, None, &mut tidy);
}

/// A key through the path the TUI uses for a char event: no `TurnRecord`.
fn key(b: &mut BootedStory) {
    assert!(matches!(b.session.pending_input(), InputKind::Char), "premise: the game is paused on a key");
    let before = b.state.history.len();
    let result = b.session.submit_key(KeyInput::Char(' ')).expect("a key turn");
    let _ = app::host::turn::apply_game_driven_result(
        &mut b.state, &mut b.mapper, &result, &b.game_dir, None, &*b.session, app::pager::Driver::PlayerInput,
    );
    assert_eq!(b.state.history.len(), before, "a game-driven turn records nothing");
}

/// 4 records, two key pauses between them. Returns the history index of the 4th.
fn first_stretch(b: &mut BootedStory) -> usize {
    play(b, "no"); // input 1 -> record 1
    key(b); // input 2: "Speeding down Montgomery Boulevard"
    play(b, "no"); // input 3 -> record 2
    play(b, "no"); // input 4 -> record 3
    key(b); // input 5
    key(b); // input 6: "You are Wendy Mackaye"
    play(b, "no"); // input 7 -> record 4
    assert_eq!(b.state.history.len(), 4, "premise: four recorded turns");
    3
}

#[derive(Debug, PartialEq)]
struct Shape {
    lines: Vec<String>,
    kinds: Vec<TranscriptKind>,
    runs: Vec<Vec<StyleRun>>,
    anchors: (Option<usize>, Option<usize>),
}

fn shape(b: &BootedStory) -> Shape {
    Shape {
        lines: b.state.transcript.clone(),
        kinds: b.state.transcript_kinds.clone(),
        runs: b.state.transcript_runs.clone(),
        anchors: (b.state.clear_anchor, b.state.top_anchor),
    }
}

fn line_with(b: &BootedStory, needle: &str) -> Option<usize> {
    b.state.transcript.iter().position(|l| l.contains(needle))
}

fn rewind(b: &mut BootedStory, idx: usize) {
    let out = resume_from_turn(&mut *b.session, &mut b.mapper, &mut b.state, idx, None);
    assert!(out.ok, "the rewind succeeds");
}

#[test]
fn rewind_keeps_post_keypress_text_and_styling() {
    for honor in [true, false] {
        let home = app::scratch_dir("sq1715-keep");
        let mut b = boot(&home, honor);
        let at = first_stretch(&mut b);
        let seen = shape(&b); // the screen the player had at record 4's prompt
        play(&mut b, "no"); // input 8 -> record 5
        play(&mut b, "no"); // input 9 -> record 6
        assert_eq!(b.state.history.len(), 6, "premise: two more turns recorded");
        assert_ne!(shape(&b), seen, "premise: the transcript moved on");

        // Non-vacuity: the text only a game-driven turn printed, and styling on it.
        let i = line_with(&b, AFTER_FIRST_KEY).expect("premise: the first pause's follow-on text is on screen");
        assert!(line_with(&b, AFTER_SECOND_KEY).is_some(), "premise: the second pause's too");
        assert!(
            b.state.transcript_runs[i].iter().any(|r| r.fg != 0 && r.bg != 0),
            "premise: that line is game-coloured (fg and bg set) (honor={honor}): {:?}",
            b.state.transcript_runs[i]
        );

        rewind(&mut b, at);
        let i = line_with(&b, AFTER_FIRST_KEY).expect("post-keypress text survives the rewind");
        assert!(line_with(&b, AFTER_SECOND_KEY).is_some(), "so does the second pause's");
        assert!(b.state.transcript_runs[i].iter().any(|r| r.fg != 0 && r.bg != 0), "its colour runs survive (honor={honor})");
        assert_eq!(shape(&b), seen, "the rewound screen is exactly the one the player saw (honor={honor})");
        assert_eq!(b.state.history.len(), at + 1, "later turns discarded");

        // Records up to the target stay comparable: a second rewind cuts back too.
        rewind(&mut b, 1);
        assert!(line_with(&b, AFTER_FIRST_KEY).is_some(), "an earlier rewind still keeps the first pause's text");
        assert!(line_with(&b, AFTER_SECOND_KEY).is_none(), "and has not yet reached the second");
        let _ = std::fs::remove_dir_all(&home);
    }
}

#[test]
fn a_reshaped_transcript_falls_back_to_the_rebuild() {
    for honor in [true, false] {
        let home = app::scratch_dir("sq1715-epoch");
        let mut b = boot(&home, honor);
        let at = first_stretch(&mut b);
        let rebuilt = app::history::rebuild_transcript(&b.state.history, at);
        // Something shortens the transcript after the record (a menu reprint
        // collapse does exactly this).
        let epoch = b.state.transcript_epoch;
        let len = b.state.transcript.len();
        b.state.truncate_transcript(len - 1);
        assert_ne!(b.state.transcript_epoch, epoch, "premise: a truncate bumps the epoch");

        rewind(&mut b, at);
        assert_eq!((b.state.transcript.clone(), b.state.transcript_kinds.clone()), rebuilt, "today's rebuild, unchanged");
        assert!(b.state.transcript_runs.iter().all(Vec::is_empty), "the rebuild carries no runs");
        assert!(line_with(&b, AFTER_FIRST_KEY).is_none(), "and has no game-driven text (the old behaviour)");
        let _ = std::fs::remove_dir_all(&home);
    }
}

#[test]
fn an_insert_above_the_prompt_also_falls_back() {
    let home = app::scratch_dir("sq1715-insert");
    let mut b = boot(&home, true);
    let at = first_stretch(&mut b);
    let epoch = b.state.transcript_epoch;
    // An app-internal line inserted above the trailing `>` shifts the prompt.
    b.state.push_transcript_internal("[note]", TranscriptKind::Meta);
    assert_ne!(b.state.transcript_epoch, epoch, "premise: the insert bumped the epoch");
    let rebuilt = app::history::rebuild_transcript(&b.state.history, at);
    rewind(&mut b, at);
    assert_eq!((b.state.transcript.clone(), b.state.transcript_kinds.clone()), rebuilt);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_record_without_a_mark_rebuilds_as_before() {
    for honor in [true, false] {
        let home = app::scratch_dir("sq1715-old");
        let mut b = boot(&home, honor);
        let at = first_stretch(&mut b);
        // Records from an older save: no mark at all.
        b.state.history = b
            .state
            .history
            .iter()
            .map(|r| {
                Arc::new(app::history::TurnRecord {
                    transcript_len: None,
                    transcript_epoch: None,
                    transcript_tail_chars: None,
                    transcript_anchors: None,
                    ..(**r).clone()
                })
            })
            .collect();
        let rebuilt = app::history::rebuild_transcript(&b.state.history, at);
        rewind(&mut b, at);
        assert_eq!((b.state.transcript.clone(), b.state.transcript_kinds.clone()), rebuilt);
        assert!(line_with(&b, AFTER_FIRST_KEY).is_none());
        let _ = std::fs::remove_dir_all(&home);
    }
}

/// Save State then restore: records read back from the archive carry no mark (the
/// archive filters the transcript, so their lengths would not index the live one)
/// and rewind by rebuilding; records made AFTER the restore are stamped under the
/// restored transcript and cut back again.
#[test]
fn a_round_trip_through_the_archive_rebuilds_old_records_and_cuts_back_new_ones() {
    for honor in [true, false] {
        let home = app::scratch_dir("sq1715-roundtrip");
        let mut b = boot(&home, honor);
        first_stretch(&mut b);
        let slot = home.join("slot.lanthorn");
        assert!(matches!(save_state_now(&mut *b.session, &b.mapper, &b.state, &b.ifid, &slot), ExitSave::Saved));

        let home2 = app::scratch_dir("sq1715-roundtrip-dst");
        let mut c = boot(&home2, honor);
        restore_file(&mut *c.session, &mut c.mapper, &mut c.state, &slot, None).expect("restore");
        assert_eq!(c.state.history.len(), 4, "premise: the history came back");
        assert!(c.state.history.iter().all(|r| r.transcript_len.is_none()), "archive-loaded records carry no mark");

        // A turn after the restore is stamped under the restored transcript.
        play(&mut c, "no"); // record 5
        let at = c.state.history.len() - 1;
        assert!(c.state.history[at].transcript_len.is_some());
        let seen = shape(&c);
        play(&mut c, "no"); // record 6
        rewind(&mut c, at);
        assert_eq!(shape(&c), seen, "a post-restore record cuts back exactly (honor={honor})");

        // And an archive-loaded record rebuilds, as before.
        let rebuilt = app::history::rebuild_transcript(&c.state.history, 1);
        rewind(&mut c, 1);
        assert_eq!((c.state.transcript.clone(), c.state.transcript_kinds.clone()), rebuilt);
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&home2);
    }
}
