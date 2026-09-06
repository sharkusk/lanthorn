//! SQ-0943: the asciinema serialiser, and the committed cast recipe.
//!
//! Two halves, both cheap and both in CI. The serialiser is pure — a
//! `Capture` in, a v2 cast out — so it is tested against hand-built captures
//! whose expected file can be stated exactly, and the assertions go through a
//! real JSON parser rather than substring-matching, because "it looks like
//! JSON" is exactly the failure a hand-rolled writer has. The manifest half
//! mirrors `gallery_manifest.rs`: the recipe is the only committed artefact, so
//! it is the thing that can go stale in silence.
//!
//! No case here boots a story or wants a gitignored medium.

#![cfg(unix)]

use std::time::Duration;

use super::pty_stream::cast::{self, CastManifest, Header, Program};
use super::pty_stream::driver::{Capture, Flush, Spec};

/// A capture with the given flushes, for the serialiser cases.
fn capture(bytes: &[u8], flushes: &[(f64, usize, usize)]) -> Capture {
    let mut spec = Spec::new("/nonexistent/lanthorn", "/nonexistent/story.z5", "/nonexistent/home");
    spec.cols = 80;
    spec.rows = 24;
    Capture {
        bytes: bytes.to_vec(),
        flushes: flushes
            .iter()
            .map(|&(at, offset, len)| Flush { at: Duration::from_secs_f64(at), offset, len })
            .collect(),
        answered: Vec::new(),
        resizes: Vec::new(),
        typed: Vec::new(),
        spec,
        duration: Duration::from_secs(1),
        timed_out: false,
        exit: None,
    }
}

fn header() -> Header {
    Header {
        width: 80,
        height: 24,
        title: "A \"quoted\" title".to_string(),
        idle_time_limit: 2.0,
        timestamp: 1_700_000_000,
        term: "xterm-256color".to_string(),
        note: cast::NO_KITTY_NOTE.to_string(),
    }
}

/// Header line and event lines, each parsed as its own JSON value.
fn parse(text: &str) -> (serde_json::Value, Vec<serde_json::Value>) {
    let mut lines = text.lines();
    let head: serde_json::Value =
        serde_json::from_str(lines.next().expect("a cast has a header line")).expect("the header must be JSON");
    let events = lines
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("event line is not JSON: {e}\n{l}")))
        .collect();
    (head, events)
}

#[test]
fn the_header_is_a_v2_header_and_carries_the_size_the_harness_drove() {
    let cap = capture(b"hello", &[(0.5, 0, 5)]);
    let (head, _) = parse(&cast::to_cast(&cap, &header()));
    assert_eq!(head["version"], 2);
    assert_eq!(head["width"], 80);
    assert_eq!(head["height"], 24);
    assert_eq!(head["timestamp"], 1_700_000_000i64);
    assert_eq!(head["title"], "A \"quoted\" title", "a quote in a title must survive being written by hand");
    assert_eq!(head["env"]["TERM"], "xterm-256color");
}

/// The one question a reader will ask of a lanthorn cast with no artwork in it,
/// answered inside the file rather than beside it.
#[test]
fn the_header_says_why_there_is_no_kitty_artwork() {
    let cap = capture(b"x", &[(0.0, 0, 1)]);
    let (head, _) = parse(&cast::to_cast(&cap, &header()));
    let note = head["lanthorn"]["note"].as_str().expect("a note");
    assert!(note.contains("kitty"), "{note}");
    assert!(note.contains("half-block"), "{note}");
    assert!(head["lanthorn"]["build"].is_string(), "the build that made the recording is half of its identity");
}

#[test]
fn one_event_per_flush_at_the_flush_s_own_time() {
    let cap = capture(b"onetwothree", &[(0.25, 0, 3), (1.5, 3, 3), (4.0, 6, 5)]);
    let (_, events) = parse(&cast::to_cast(&cap, &header()));
    assert_eq!(events.len(), 3);
    let times: Vec<f64> = events.iter().map(|e| e[0].as_f64().unwrap()).collect();
    assert_eq!(times, vec![0.25, 1.5, 4.0]);
    assert!(events.iter().all(|e| e[1] == "o"), "every event is output");
    let data: Vec<&str> = events.iter().map(|e| e[2].as_str().unwrap()).collect();
    assert_eq!(data, vec!["one", "two", "three"]);
}

/// Escapes are what a cast is MADE of, so the writer has to survive its own
/// content: ESC, CR, LF, tab, quote and backslash all appear in one stream.
#[test]
fn control_bytes_and_quotes_round_trip_through_the_writer() {
    let raw = b"\x1b[2J\"a\\b\"\r\n\tc\x07";
    let cap = capture(raw, &[(0.1, 0, raw.len())]);
    let (_, events) = parse(&cast::to_cast(&cap, &header()));
    assert_eq!(
        events[0][2].as_str().unwrap(),
        std::str::from_utf8(raw).unwrap(),
        "what comes back out of a JSON parser must be the bytes that went in"
    );
}

/// THE correctness property of the recorder. Half-block output is nothing but
/// multi-byte characters, and a flush boundary lands wherever the kernel put it
/// — so `▀` gets cut in half routinely. A lossy conversion would drop U+FFFD
/// into the middle of somebody's frame; the writer holds the tail instead.
#[test]
fn a_multibyte_glyph_split_across_two_flushes_is_reassembled_not_replaced() {
    let raw = "a▀b".as_bytes(); // 'a' | E2 96 80 | 'b'
    assert_eq!(raw.len(), 5);
    // Cut the ▀ after its first byte.
    let cap = capture(raw, &[(0.0, 0, 2), (1.0, 2, 3)]);
    let text = cast::to_cast(&cap, &header());
    let (_, events) = parse(&text);
    let joined: String = events.iter().map(|e| e[2].as_str().unwrap()).collect();
    assert_eq!(joined, "a▀b", "the split glyph must come back whole");
    assert!(!joined.contains('\u{FFFD}'), "no replacement character may appear: {joined:?}");
    assert_eq!(events[0][2], "a", "the incomplete tail waits rather than being closed off");
    assert_eq!(events[1][2], "▀b");
}

#[test]
fn a_trailing_incomplete_sequence_is_still_emitted_rather_than_lost() {
    // A truncated write at the very end of a run: the last flush ends mid-glyph
    // and there is no next flush to carry it into.
    let raw = b"ok\xe2\x96";
    let cap = capture(raw, &[(0.0, 0, 4)]);
    let (_, events) = parse(&cast::to_cast(&cap, &header()));
    let joined: String = events.iter().map(|e| e[2].as_str().unwrap()).collect();
    assert!(joined.starts_with("ok"), "the good bytes survive: {joined:?}");
    assert!(
        joined.chars().count() > 2,
        "the run is not silently shorter than the capture; the unusable tail is marked, not dropped"
    );
}

/// The capability probe is app→terminal traffic like everything else, so a cast
/// that correctly drew nothing graphical still contains one `ESC _ G`. Counting
/// it would refuse every good recording this tool makes.
#[test]
fn the_kitty_capability_probe_is_not_a_graphics_command() {
    let probe = b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\";
    assert_eq!(cast::graphics_commands(probe), 0, "`a=q` is a question, not a picture");

    let transmit = b"\x1b_Ga=T,f=32,s=4,v=4,i=7;AAAABBBB\x1b\\";
    assert_eq!(cast::graphics_commands(transmit), 1);
    let both = [probe.as_slice(), transmit.as_slice()].concat();
    assert_eq!(cast::graphics_commands(&both), 1, "one of these two would be dropped by the player");
    assert_eq!(cast::graphics_commands(b"no escapes here at all"), 0);
}

// ── The committed recipe ──────────────────────────────────────────────────────

fn manifest() -> CastManifest {
    let path = CastManifest::default_path();
    CastManifest::load(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

const GOOD: &str = r#"
id = "a-cast"
title = "A Recording"
caption = "What it is for."
program = "zvm-cli"
media = "stories/a.z5"
args = ["{media}"]
keys = "wait:900,text:look,cr"
size = "100x30"
expect = ["something"]
"#;

fn parse_one(body: &str) -> Result<CastManifest, String> {
    CastManifest::parse(&format!("[[casts]]\n{body}"))
}

#[test]
fn the_committed_manifest_parses_and_validates() {
    assert!(manifest().casts.len() >= 5, "the site plan wants the automapper, the machine table and one cast per engine");
}

#[test]
fn the_specimen_cast_is_valid() {
    assert!(parse_one(GOOD).is_ok(), "the specimen must parse, or every negative case below proves nothing");
}

/// The site plan's list, checked against the recipe rather than against memory:
/// the automapper, the machine table, a period look, a pinned status line, and
/// one cast per engine.
#[test]
fn the_recipe_covers_every_engine_and_the_showpiece() {
    let m = manifest();
    for want in [Program::Lanthorn, Program::ZvmCli, Program::GvmCli, Program::ScottCli] {
        assert!(
            m.casts.iter().any(|c| c.program == want),
            "nothing records `{}` — the CLI clients answer an objection the TUI cannot",
            want.binary()
        );
    }
    assert!(
        m.casts.iter().any(|c| c.args.iter().any(|a| a == "--machines")),
        "no `zvm-cli --machines` cast: the whole fidelity argument in about four seconds"
    );
    assert!(m.casts.iter().any(|c| c.show_map), "no automapper cast, which is the project's differentiator");
}

#[test]
fn ids_are_unique_and_survive_being_filenames() {
    let mut seen = std::collections::BTreeSet::new();
    for c in &manifest().casts {
        assert!(seen.insert(c.id.clone()), "duplicate cast id `{}`", c.id);
        assert!(
            c.id.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-'),
            "`{}` is not a safe filename",
            c.id
        );
    }
}

#[test]
fn every_cast_names_its_media_consistently_and_parses_its_keys() {
    for c in &manifest().casts {
        c.keys().unwrap_or_else(|e| panic!("{e}"));
        c.size_cells().unwrap_or_else(|e| panic!("{e}"));
        if c.media.is_empty() {
            assert!(
                !c.args.iter().any(|a| a.contains("{media}")),
                "`{}` substitutes a medium it does not name",
                c.id
            );
        } else {
            assert!(c.media_path().is_some(), "`{}` names a medium that resolves to nothing", c.id);
        }
        assert!(c.max_bytes > 0 && c.idle_time_limit > 0.0, "`{}`", c.id);
    }
}

/// `{media}` is the only way a CLI cast can name its story, because the tool
/// gives a CLI client its argument list verbatim.
#[test]
fn a_cli_cast_with_a_medium_substitutes_an_absolute_path() {
    let m = parse_one(GOOD).expect("valid");
    let argv = m.casts[0].cli_argv();
    assert_eq!(argv.len(), 1);
    assert!(argv[0].ends_with("stories/a.z5"), "{argv:?}");
    assert!(!argv[0].contains("{media}"), "the placeholder must be gone: {argv:?}");
    assert!(std::path::Path::new(&argv[0]).is_absolute(), "a CLI client is spawned with no cwd of ours: {argv:?}");
}

#[test]
fn a_cast_with_no_guard_is_refused() {
    let e = parse_one(&GOOD.replace(r#"expect = ["something"]"#, "")).expect_err("guards are required");
    assert!(e.contains("expect"), "{e}");
}

#[test]
fn a_cast_may_not_force_a_graphics_protocol_or_a_home() {
    for owned in ["--image-protocol", "--user-dir"] {
        let e = parse_one(&format!("{GOOD}\n")
            .replace(r#"args = ["{media}"]"#, &format!("args = [\"{owned}\", \"x\"]")))
        .expect_err("the tool owns these");
        assert!(e.contains(owned), "{e}");
    }
}

#[test]
fn a_lanthorn_cast_must_name_a_story_and_a_cli_cast_need_not() {
    let e = parse_one(&GOOD.replace(r#"program = "zvm-cli""#, r#"program = "lanthorn""#).replace(r#"media = "stories/a.z5""#, ""))
        .expect_err("the TUI has nothing to show without a story");
    assert!(e.contains("media"), "{e}");

    let ok = parse_one(
        &GOOD.replace(r#"media = "stories/a.z5""#, "").replace(r#"args = ["{media}"]"#, r#"args = ["--machines"]"#),
    );
    assert!(ok.is_ok(), "`zvm-cli --machines` takes no story at all: {ok:?}");
}

#[test]
fn a_cli_cast_may_not_ask_for_the_map_pane() {
    let e = parse_one(&format!("{GOOD}show_map = true\n")).expect_err("the CLI clients have no map pane");
    assert!(e.contains("map"), "{e}");
}

#[test]
fn a_duplicate_id_is_refused() {
    let e = CastManifest::parse(&format!("[[casts]]{GOOD}[[casts]]{GOOD}")).expect_err("ids are filenames");
    assert!(e.contains("duplicate"), "{e}");
}

#[test]
fn an_unknown_field_is_refused() {
    assert!(parse_one(&format!("{GOOD}duration = 12\n")).is_err(), "a cast's length is measured, never declared");
}
