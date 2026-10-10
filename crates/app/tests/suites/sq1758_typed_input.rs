//! SQ-1758: in inline-prompt mode the typed command is a marked span of the game's
//! `>` prompt line, so it can be drawn apart from story text -- and the mark survives
//! everything that carries transcript lines.
//!
//! Photopia (a manifest fixture, so this runs on CI) is a game-COLOURED story that
//! colours all its text, so its typed spans carry no story ink of their own.
//! Every case pins both `honor_game_colours` modes.

use app::engine::{Engine, KeyInput};
use app::host::persist::{restore_file, resume_from_turn, save_state_now, ExitSave};
use app::host::{boot_story, finish_command_turn, BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts};
use app::launch_options::LaunchOverrides;
use app::session::InputKind;
use app::state::GLK_STYLE_TYPED_INPUT;

use crate::fixture_paths::fixture_path;

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

fn play(b: &mut BootedStory, cmd: &str) {
    assert!(matches!(b.session.pending_input(), InputKind::Line), "premise: a line is awaited for `{cmd}`");
    let result = b.session.submit(cmd);
    let mut tidy = 0u32;
    let _ = finish_command_turn(cmd, true, result, &mut b.state, &mut b.mapper, &mut *b.session, &b.game_dir, &b.ifid, &b.arc_file, None, &mut tidy);
}

/// A key through the path the TUI uses for a char event.
fn key(b: &mut BootedStory) {
    assert!(matches!(b.session.pending_input(), InputKind::Char), "premise: the game is paused on a key");
    let result = b.session.submit_key(KeyInput::Char(' ')).expect("a key turn");
    let _ = app::host::turn::apply_game_driven_result(
        &mut b.state, &mut b.mapper, &result, &b.game_dir, None, &*b.session, app::pager::Driver::PlayerInput,
    );
}

/// Photopia answers `no` to its instructions question three times around one
/// "press a key" pause (the same stretch SQ-1715 drives): 3 typed commands, 3 records.
fn three_turns(b: &mut BootedStory) {
    play(b, "no");
    key(b);
    play(b, "no");
    play(b, "no");
    assert_eq!(b.state.history.len(), 3, "premise: three recorded turns");
}

/// Every transcript line that carries a typed-input span, with the text it covers.
fn typed_spans(b: &BootedStory) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (i, runs) in b.state.transcript_runs.iter().enumerate() {
        for r in runs.iter().filter(|r| r.glk_style == GLK_STYLE_TYPED_INPUT) {
            let text: String = b.state.transcript[i].chars().skip(r.start).take(r.end - r.start).collect();
            out.push((i, text));
        }
    }
    out
}

#[test]
fn an_inline_command_is_a_typed_span_of_the_prompt_line_and_takes_the_theme_colour_when_the_story_colours_all_its_text() {
    for honor in [true, false] {
        let home = app::scratch_dir("sq1758-mark");
        let mut b = boot(&home, honor);
        three_turns(&mut b);
        let spans = typed_spans(&b);
        assert!(spans.len() >= 3, "all three commands are marked (honor={honor}): {spans:?}");
        assert!(spans.iter().all(|(_, t)| t == "no"), "each span covers exactly the typed text: {spans:?}");
        let (line, _) = spans[0];
        assert_eq!(b.state.transcript_kinds[line], app::state::TranscriptKind::Story, "the line stays a Story line");
        // Photopia colours ALL its text and prints the prompt in that same colour, so
        // it states no input style: the span carries no story ink and the theme's
        // `transcript_input` colour applies (contrast is the renderer's fallback).
        let run = b.state.transcript_runs[line].iter().find(|r| r.glk_style == GLK_STYLE_TYPED_INPUT).unwrap();
        assert_eq!((run.ink, run.bits), (0, 0), "a page colour equal to the prompt's is no opinion: {run:?}");
        let _ = std::fs::remove_dir_all(&home);
    }
}

/// Restore, then make another move, then assert (CLAUDE.md: restore bugs surface one
/// action after the restore).
#[test]
fn save_state_restore_then_a_move_keeps_earlier_commands_marked() {
    for honor in [true, false] {
        let home = app::scratch_dir("sq1758-restore");
        let mut b = boot(&home, honor);
        three_turns(&mut b);
        let before = typed_spans(&b);
        assert!(before.len() >= 3, "premise");
        let slot = home.join("slot.lanthorn");
        assert!(matches!(save_state_now(&mut *b.session, &b.mapper, &b.state, &b.ifid, &slot), ExitSave::Saved));

        let home2 = app::scratch_dir("sq1758-restore-dst");
        let mut c = boot(&home2, honor);
        restore_file(&mut *c.session, &mut c.mapper, &mut c.state, &slot, None).expect("restore");
        key(&mut c); // the game is paused on a key (SQ-1715 drives the same two)
        key(&mut c);
        play(&mut c, "no"); // the perturbing move: input after the restore
        let after = typed_spans(&c);
        assert!(
            after.len() > before.len(),
            "the restored spans are still there and the new move added one (honor={honor}): {before:?} -> {after:?}"
        );
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&home2);
    }
}

#[test]
fn a_rewind_keeps_the_commands_before_the_target_and_drops_the_undone_ones() {
    for honor in [true, false] {
        let home = app::scratch_dir("sq1758-rewind");
        let mut b = boot(&home, honor);
        three_turns(&mut b);
        let full = typed_spans(&b).len();
        assert!(full >= 3, "premise");
        let out = resume_from_turn(&mut *b.session, &mut b.mapper, &mut b.state, 1, None);
        assert!(out.ok);
        let kept = typed_spans(&b).len();
        assert!(kept >= 1 && kept < full, "the earlier command(s) stay marked, the undone one's mark is gone: {kept} of {full}");
        let _ = std::fs::remove_dir_all(&home);
    }
}
