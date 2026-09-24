//! SQ-1553: a truncated dictionary key is SHOWN as the whole word.
//!
//! A Version 3 dictionary keeps six Z-characters (ZMSD 1.1 §13.3), so Zork I
//! stores `examin`, `lanter` and `brandi`, and every surface that lists
//! dictionary entries — completion, the command band's VERB column, the
//! vocabulary offer — used to show those fragments. Matching was already right;
//! this is about what the player reads.
//!
//! | fixture | release | turns in | what it shows |
//! |---|---|---|---|
//! | `minizork-r34-s871124.z3` (fetched) | r34/s871124 | 0 | the whole path, in CI |
//! | `stories/zork1-r88-s840726.z3` | r88/s840726 | 0 | the same on the full game, and its cost |
//!
//! Booted through `app::host::boot_story` — the per-story build the TUI's own
//! startup runs — so `dict_words` is the list completion really reads.

use std::path::{Path, PathBuf};
use std::time::Instant;

use app::config::Config;
use app::engine::Engine;
use app::host::{boot_story, BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts};
use app::launch_options::LaunchOverrides;

use crate::fixture_paths::fixture_path;

fn boot(story: PathBuf, home: &Path) -> BootedStory {
    let overrides = LaunchOverrides::default();
    let cfg = Config {
        user_dir: home.to_path_buf(),
        config_file: home.join("config.toml"),
        random_seed: Some(1),
        ..Config::default()
    };
    let req = BootRequest {
        story_path: story,
        disk_entry: None,
        overrides: &overrides,
        cfg,
        data_base: home.join("saves"),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
    };
    boot_story(req, &mut QuietBoot).expect("the story boots headlessly")
}

fn minizork() -> Option<BootedStory> {
    let path = fixture_path("minizork-r34-s871124.z3");
    if !path.is_file() {
        eprintln!("SKIP: {} absent", path.display());
        return None;
    }
    Some(boot(path, &app::scratch_dir("spell-mini")))
}

fn zork1() -> Option<BootedStory> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/zork1-r88-s840726.z3");
    if !path.is_file() {
        eprintln!("SKIP: {} absent", path.display());
        return None;
    }
    Some(boot(path, &app::scratch_dir("spell-zork1")))
}

fn complete(b: &BootedStory, line: &str) -> Vec<String> {
    app::complete::completion_candidates(
        line,
        line.chars().count(),
        '/',
        &[],
        &b.state.dict_words,
        &[],
        &[],
    )
}

/// The shared acceptance for both fixtures.
fn spells_the_story_out(mut b: BootedStory, what: &str) {
    // Non-vacuity: this really is a six-character dictionary.
    assert!(
        b.state.dict_words.iter().all(|w| w != "lanter"),
        "{what}: no dictionary word is still shown cut short where the story spells it"
    );
    assert!(b.state.dict_words.iter().any(|w| w == "lantern"), "{what}: `lanter` → `lantern`");
    assert!(b.state.dict_words.iter().any(|w| w == "examine"), "{what}: `examin` → `examine`");

    // Completion offers the whole word, and applying it still reaches the
    // dictionary entry the parser matches.
    let hits = complete(&b, "activ");
    assert_eq!(hits, vec!["activate"], "{what}: `activ` completes to the whole verb");
    let line = app::complete::apply_completion_to_line("activ", '/', &hits[0]);
    assert_eq!(line, "activate");
    assert_eq!(b.session.knows_word(&line), Some(true), "{what}: the parser takes it");
    assert_eq!(
        app::complete::completion_ghost_tail(&complete(&b, "exami")[0], "exami").as_deref(),
        Some("ne"),
        "{what}: the ghost adds the rest of the WHOLE word"
    );

    // An ambiguous key stays as stored. Mini-Zork's `descri` is its
    // describe-verb; its text prints `descriptions` and never `describe`.
    let v = b.state.vocab.get(b.session.as_ref()).expect("a Z-machine grammar");
    assert_eq!(v.spell("descri"), "descri", "{what}: two whole words reach it");

    // The band's VERB column, through the host API.
    let band = app::host::refresh_band_data(&mut b.state, b.session.as_ref());
    let verbs: Vec<&str> = band.verbs.iter().map(|e| e.word.as_str()).collect();
    for want in ["examine", "brandish", "activate"] {
        assert!(verbs.contains(&want), "{what}: VERB column holds `{want}`: {verbs:?}");
    }
    for gone in ["examin", "brandi", "activa"] {
        assert!(!verbs.contains(&gone), "{what}: VERB column no longer shows `{gone}`");
    }
    assert!(verbs.windows(2).all(|p| p[0] <= p[1]), "{what}: still alphabetical");
}

#[test]
fn minizork_shows_whole_words() {
    let Some(b) = minizork() else { return };
    spells_the_story_out(b, "Mini-Zork r34");
}

#[test]
fn zork1_shows_whole_words() {
    let Some(b) = zork1() else { return };
    let v = {
        let mut b = b;
        let v = b.state.vocab.get(b.session.as_ref()).expect("a Z-machine grammar").clone();
        spells_the_story_out(b, "Zork I r88");
        v
    };
    assert_eq!(v.spell("machin"), "machin", "`machine` and `machinery` both reach it");
    assert_eq!(v.spell("mailbo"), "mailbox");
}

/// The one-time cost, printed for the record rather than pinned (a debug
/// build on a loaded machine is no stopwatch): the text scan alone, and the
/// whole vocabulary read it rides with.
#[test]
fn the_one_time_decode_cost_is_reported() {
    let Some(b) = zork1().or_else(minizork) else { return };
    let t = Instant::now();
    let words = b.session.story_text_words().expect("the Z-machine reads its text");
    let scan = t.elapsed();
    let t = Instant::now();
    let mut vs = app::vocab::VocabState::default();
    assert!(vs.get(b.session.as_ref()).is_some());
    let whole = t.elapsed();
    eprintln!(
        "story text: {} words, scan {:?}; whole vocabulary read incl. scan {:?}",
        words.len(),
        scan,
        whole
    );
    assert!(words.len() > 1000, "a real game's text is thousands of words, got {}", words.len());
}
