//! Lanthorn's Guiding Light, and the things that keep it honest (SQ-1045).
//!
//! Five features print through this register — SQ-1041's vocabulary offer,
//! SQ-1042's completion, SQ-1043's irreversibility caution, SQ-1044's recap, and
//! whatever follows — so what it guarantees has to be a property of the CODE, not
//! a habit of the prose.
//!
//! The property is **attribution**: a player must never mistake lanthorn's help
//! for the story's own voice, because when the help is WRONG they will otherwise
//! blame the game. Infocom's parser speaks in brackets (`[I don't know the word
//! "illuminate".]`), so a helper in the same stream is confusable by default.
//!
//! Attribution is carried differently on each surface, and nobody is on two of
//! them at once. The cases below prove each rather than asserting it:
//!
//! * **`TranscriptKind::Assist`** — the line is told from the story by its KIND,
//!   which holds even against a story that prints our own words verbatim
//!   (`an_assist_is_told_from_the_story_by_its_kind_not_its_text`), and lets
//!   `/filter story` hide every assist for a player who wants 1982.
//! * **the mark, on screen** — the gutter glyph IS the icon
//!   (`symbols.assist_gutter`, `●` by default), and the text carries no prefix at
//!   all. The introduction shows the configured glyph, so a font that lacks it
//!   fails visibly in the one sentence that explains the mark.
//! * **the words, in a file** — `AppState::transcript_for_export` puts
//!   `Lanthorn: ` back on, because a saved transcript has no gutter, no colour and
//!   nothing a screen reader can voice.
//!
//! The source-level case at the bottom is the one that matters most, for the same
//! reason `scratch_path_discipline` exists: the next four features are written by
//! someone with no reason to know any of this, and a hand-built assist line would
//! look perfectly fine in review.

use app::assist::{Assist, AssistTone, EXPORT_PREFIX, preamble};
use app::state::{AppState, TranscriptFilter, TranscriptKind};

fn kinds_of(s: &AppState) -> Vec<TranscriptKind> {
    s.transcript_kinds.clone()
}

/// The whole quest in one case: a story that prints our exact words is STILL told
/// apart from an assist, because the separation is the kind and not the wording.
/// Falsify by tagging the assist `TranscriptKind::Meta` (or `Story`) in
/// `push_assist` — the two lines become indistinguishable and this fails.
#[test]
fn an_assist_is_told_from_the_story_by_its_kind_not_its_text() {
    let mut s = AppState::default();
    s.assist_preamble_shown = true; // the introduction has its own case
    s.push_assist(&Assist::help("this story knows — light · turn on · burn"));
    let ours = s.transcript.last().cloned().unwrap();

    // A hostile (or merely unlucky) game prints the identical line itself.
    s.push_transcript_kind(&ours, TranscriptKind::Story);

    assert_eq!(s.transcript[s.transcript.len() - 2], s.transcript[s.transcript.len() - 1]);
    let k = kinds_of(&s);
    assert_eq!(k[k.len() - 2], TranscriptKind::Assist);
    assert_eq!(k[k.len() - 1], TranscriptKind::Story);
}

/// On screen the line is the caller's words and NOTHING else — the mark in the
/// gutter is what says whose it is. Falsify by putting the prefix back into
/// `Assist::lines`: the assist stops matching what was asked for, and a
/// forty-column pane starts spending ten of them on furniture.
#[test]
fn on_screen_an_assist_wears_the_mark_and_not_the_words() {
    let mut s = AppState::default();
    s.assist_preamble_shown = true;
    s.push_assist(&Assist::help("this story knows — light"));
    assert_eq!(s.transcript.last().unwrap(), "this story knows — light");
    assert!(
        !s.transcript.iter().any(|l| l.starts_with(EXPORT_PREFIX)),
        "no prefix on screen: {:?}",
        s.transcript
    );

    // And the mark exists, and is not the glyph Infocom spends on footnotes.
    assert_eq!(s.symbols.assist_gutter, '●');
    assert_ne!(s.symbols.assist_gutter, '*', "asterisks are Infocom's footnote marker");
}

/// A file has no gutter and no colour, so the export puts the words back —
/// exactly once, on the assist lines and on nothing else.
#[test]
fn an_exported_transcript_says_who_was_speaking() {
    let mut s = AppState::default();
    s.assist_preamble_shown = true;
    s.push_transcript_kind("West of House", TranscriptKind::Story);
    s.push_assist(&Assist::help("this story knows:\nlight · turn on · burn"));
    s.push_transcript_kind("style reloaded", TranscriptKind::Meta);

    // By kind, not by index: an app-generated line is INSERTED above a trailing
    // story line in inline-prompt mode, so the order on screen is not the order
    // these were pushed in (`insert_above_prompt_at`).
    let out = s.transcript_for_export();
    assert_eq!(out.len(), s.transcript.len(), "export covers the visible transcript");
    let at = |k: TranscriptKind| -> Vec<String> {
        kinds_of(&s)
            .iter()
            .enumerate()
            .filter(|(_, x)| **x == k)
            .map(|(i, _)| out[i].clone())
            .collect()
    };
    assert_eq!(at(TranscriptKind::Story), vec!["West of House"], "the story's own line is untouched");
    assert_eq!(at(TranscriptKind::Meta), vec!["style reloaded"], "a slash dump is not ours to attribute this way");
    assert_eq!(
        at(TranscriptKind::Assist),
        vec![format!("{EXPORT_PREFIX}this story knows:"), "  light · turn on · burn".to_string()],
        "the first line is marked, the continuation hangs rather than being marked twice"
    );
}

/// `/filter story` is the player asking for 1982, and 1982 had no assists.
/// `/filter meta` is the opposite half. Both must partition the same transcript.
#[test]
fn the_transcript_filter_separates_assists_from_the_story() {
    let mut s = AppState::default();
    s.assist_preamble_shown = true;
    s.push_transcript_kind("West of House", TranscriptKind::Story);
    s.push_assist(&Assist::help("this story knows — light"));
    s.push_transcript_kind("style reloaded", TranscriptKind::Meta);

    let assist_row = kinds_of(&s)
        .iter()
        .position(|k| *k == TranscriptKind::Assist)
        .expect("the assist line is in the transcript");

    s.transcript_filter = TranscriptFilter::Story;
    let story_only = s.visible_transcript_indices_from(0);
    assert!(!story_only.contains(&assist_row), "filter story must hide assists");

    s.transcript_filter = TranscriptFilter::Meta;
    let meta_only = s.visible_transcript_indices_from(0);
    assert!(meta_only.contains(&assist_row), "filter meta must show assists");

    s.transcript_filter = TranscriptFilter::Both;
    assert_eq!(s.visible_transcript_indices_from(0).len(), s.transcript.len());
}

/// The introduction fires once a session, above the first assist, and it shows
/// the glyph ACTUALLY in force — which is what makes it a self-test for a font
/// that has no glyph for the mark. Falsify by hard-coding `●` in `preamble`: a
/// user who set `gutter.assist` to something else is then told about a mark they
/// will never see.
#[test]
fn the_introduction_fires_once_and_shows_the_mark_in_force() {
    let mut s = AppState::default();
    s.symbols.assist_gutter = '◈';
    s.push_assist(&Assist::help("first"));
    assert_eq!(s.transcript[0], preamble('◈'));
    assert!(s.transcript[0].contains('◈'), "the introduction shows the configured mark");
    assert!(s.transcript[0].contains("story"), "and whose the marked lines are not");
    assert_eq!(s.transcript[1], "first");

    for n in 0..20 {
        s.push_assist(&Assist::caution(format!("later {n}")));
    }
    assert_eq!(
        s.transcript.iter().filter(|l| l.contains("Guiding Light")).count(),
        1,
        "the introduction is once a session"
    );
    assert!(kinds_of(&s).iter().all(|k| *k == TranscriptKind::Assist));
}

/// The switch is real, and it is checked at the one door rather than at five call
/// sites: with guidance off, nothing reaches the transcript at all — not even the
/// introduction, which would otherwise be the one line the switch cannot silence.
#[test]
fn guidance_off_silences_the_whole_set_including_its_introduction() {
    let mut s = AppState::default();
    assert!(s.config.guidance, "the light is on by default");
    s.config.guidance = false;
    s.push_assist(&Assist::help("this story knows — light"));
    s.push_assist(&Assist::caution("that cannot be undone."));
    assert!(s.transcript.is_empty(), "guidance off means silence: {:?}", s.transcript);

    // And back on, the session still owes the player its introduction.
    s.config.guidance = true;
    s.push_assist(&Assist::help("this story knows — light"));
    assert_eq!(s.transcript.len(), 2, "introduction, then the assist: {:?}", s.transcript);
}

/// Nothing lanthorn says as an assist may wear the parser's `[…]`, and no line of
/// it may sit flush left where a skimming eye reads it as prose: a continuation
/// hangs under the first line instead.
#[test]
fn no_assist_line_is_bracketed_or_a_flush_left_continuation() {
    let mut s = AppState::default();
    s.assist_preamble_shown = true;
    s.push_assist(&Assist::help("this story knows:\nlight · turn on · burn"));
    for line in &s.transcript {
        assert!(!line.starts_with('['), "assist line wears the parser's brackets: {line:?}");
    }
    assert!(
        s.transcript[1..].iter().all(|l| l.starts_with("  ")),
        "a continuation sits flush left where it reads as prose: {:?}",
        s.transcript
    );
}

/// The two tones must actually be two, or SQ-1043's "you cannot undo this" looks
/// exactly like SQ-1042's completion. They share the yellow slot (`alert`) and
/// separate by WEIGHT, the way `transcript_crash` separates from
/// `transcript_warning`, so this pins that they still resolve apart.
#[test]
fn the_caution_tone_is_styled_apart_from_the_ordinary_light() {
    let s = AppState::default();
    let help = s.colors.theme.get(AssistTone::Help.selector()).style;
    let caution = s.colors.theme.get(AssistTone::Caution.selector()).style;
    assert_ne!(help, caution, "both tones resolved to the same style");
    assert_eq!(help.fg, caution.fg, "one slot — the light is yellow either way");
    assert!(
        caution.add_modifier.contains(ratatui::style::Modifier::BOLD),
        "the caution tone is the louder one: {caution:?}"
    );
    // And both are themeable rather than hard-coded: the selectors exist.
    assert!(help.fg.is_some() && caution.fg.is_some());
}

/// **The wording of the vocabulary offer, pinned against the register's own
/// rules** (SQ-1041's opening, SQ-1121's).
///
/// Two openings, because they make two different claims and only one of them is
/// earned by anything: naming the dictionary is a fact, recommending a command
/// is a recommendation. Both are checked here rather than in `vocab.rs`, because
/// what a helper may SAY is this module's rule and the next feature will copy
/// whichever line it finds.
///
/// Falsify by restoring "you may want to try one of these" — the second-person
/// check fails on `you`, and the one-item reading fails on `one of these`.
#[test]
fn both_openings_of_the_offer_obey_the_register() {
    use app::vocab::{LEAD_DICTIONARY, LEAD_VETTED};
    for lead in [LEAD_DICTIONARY, LEAD_VETTED] {
        assert!(!lead.starts_with('['), "the parser's brackets: {lead:?}");
        let words: Vec<&str> = lead.split_whitespace().collect();
        assert!(
            !words.iter().any(|w| ["you", "your", "you're", "yours"].contains(&w.to_lowercase().as_str())),
            "the story owns the second person: {lead:?}"
        );
        // One line, on a pane that may be forty columns wide, with room left for
        // the words it introduces.
        assert!(lead.chars().count() <= 20, "the opening crowds the answer: {lead:?}");
        assert!(lead.ends_with(' '), "the opening runs straight into the list: {lead:?}");
    }
    assert_ne!(LEAD_DICTIONARY, LEAD_VETTED, "one claim cannot serve for both");

    // Each reads at ONE suggestion and at four — which is what rules out "try
    // one of these", since Zork I offers exactly one for `illuminate`.
    let mut s = AppState::default();
    s.assist_preamble_shown = true;
    for lead in [LEAD_DICTIONARY, LEAD_VETTED] {
        for list in ["light", "light · turn on · burn"] {
            let line = format!("{lead}{list}");
            s.push_assist(&Assist::help(line.clone()));
            assert_eq!(s.transcript.last().unwrap(), &line);
            assert_eq!(Assist::help(line.clone()).lines().len(), 1, "{line:?} wrapped");
        }
    }
    assert!(kinds_of(&s).iter().all(|k| *k == TranscriptKind::Assist));
}

// ── The guard ────────────────────────────────────────────────────────────────

fn app_sources() -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
        for e in std::fs::read_dir(dir).expect("app/src is part of the checkout").flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                let rel = p.strip_prefix(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap_or(&p);
                out.push((
                    rel.to_string_lossy().replace('\\', "/"),
                    std::fs::read_to_string(&p).expect("a source file in the checkout is readable"),
                ));
            }
        }
    }
    let mut out = Vec::new();
    walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
    out.sort();
    out
}

/// **`AppState::push_assist` is the only door into the assist voice.**
///
/// A caller that tags a line `TranscriptKind::Assist` itself has skipped the
/// tone's style, the once-per-session introduction, the player's `guidance`
/// switch, and the export marker that is the only attribution a saved file
/// carries. The compiler cannot catch it (the variant has to be public for the
/// renderer and the filter to match on it), so it is caught here, at the moment
/// the file is written.
///
/// Three files may name the variant, and each for a reason that is not production
/// of a line: `state.rs` holds the door itself, the `/filter` bucketing and the
/// export marking, `assist.rs` documents the register, and `render/transcript.rs`
/// has to MATCH on it to draw the mark, reserve the indent and pick the style. A
/// fourth file naming it is a second producer.
///
/// Falsified by adding `TranscriptKind::Assist` to any other file under
/// `crates/app/src/` — done, during SQ-1045, in `input.rs`, which this reported
/// by name.
///
/// A substring scan cannot tell a call from a mention, so a comment that spells
/// the variant out is reported too. Reword the comment rather than relaxing the
/// case: the spelling is what the next author copies.
#[test]
fn push_assist_is_the_only_producer_of_an_assist_line() {
    const ALLOWED: &[&str] = &["src/state.rs", "src/assist.rs", "src/render/transcript.rs"];
    let offenders: Vec<String> = app_sources()
        .into_iter()
        .filter(|(name, src)| {
            src.contains("TranscriptKind::Assist") && !ALLOWED.iter().any(|a| name.ends_with(a))
        })
        .map(|(name, _)| name)
        .collect();
    assert!(
        offenders.is_empty(),
        "these files build an assist line without going through AppState::push_assist, so they \
         skip the tone's style, the introduction, the guidance switch and the \"{EXPORT_PREFIX}\" \
         export marker: {offenders:?}"
    );
}

/// The register's constants, checked where the next four lanes will copy them
/// from: the file marker names the app, the introduction names the FEATURE (it is
/// the one chance to say what the mark means), and neither is in the parser's
/// voice.
#[test]
fn the_register_says_who_is_speaking() {
    assert!(EXPORT_PREFIX.starts_with("Lanthorn"));
    let p = preamble('●');
    assert!(p.starts_with("Lanthorn's Guiding Light"));
    assert!(p.contains("story"), "the introduction says whose the marked lines are not: {p:?}");
    assert!(!p.starts_with('[') && !EXPORT_PREFIX.starts_with('['));
    // Short enough to ride on every exported assist line without reflowing it.
    assert!(EXPORT_PREFIX.len() <= 12, "{EXPORT_PREFIX:?}");
}
