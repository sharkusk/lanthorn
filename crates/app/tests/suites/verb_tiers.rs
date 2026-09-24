//! SQ-1554: the command band's verbs arrive RANKED — a handful of core verbs,
//! then what this story talks about or the player has typed, then the rest —
//! one row per verb, through the host API a non-terminal front end reads.
//!
//! | fixture | release | turns in | what it shows |
//! |---|---|---|---|
//! | `minizork-r34-s871124.z3` (fetched) | r34/s871124 | 0, then 1 | the whole contract, in CI |
//! | `stories/zork1-r88-s840726.z3` | r88/s840726 | 0, then 1 | the same on the full game |

use app::engine::Engine;
use app::render::command_band::{VerbEntry, VerbSource, VerbTier};
use app::session::GameSession;
use app::state::AppState;

use crate::fixture_paths::fixture_path;

fn boot(file: &str) -> Option<GameSession> {
    let path = fixture_path(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: story missing at {}", path.display());
        return None;
    };
    let mut s = GameSession::new_with_trace(bytes, true, false, None, false, Vec::new(), None, None, None)
        .expect("a Version 3 story boots");
    let _ = s.take_transcript();
    Some(s)
}

fn tier(verbs: &[VerbEntry], t: VerbTier) -> Vec<&str> {
    verbs.iter().filter(|e| e.tier == t).map(|e| e.word.as_str()).collect()
}

fn the_band_is_ranked(file: &str, what: &str) {
    let Some(session) = boot(file) else { return };
    let mut state = AppState::default();
    let data = app::host::refresh_band_data(&mut state, &session);
    assert_eq!(data.verb_source, VerbSource::Story, "{what}: the story's own grammar");
    let verbs = &data.verbs;

    // A small core that holds what every player needs.
    let core = tier(verbs, VerbTier::Core);
    assert!((10..=20).contains(&core.len()), "{what}: core is small: {core:?}");
    for want in ["take", "drop", "open", "examine", "look"] {
        assert!(core.contains(&want), "{what}: `{want}` is core: {core:?}");
    }
    assert!(verbs.windows(2).all(|p| p[0].tier <= p[1].tier), "{what}: tiers arrive in order");

    // Synonyms collapse: one `take` row, `get` folded behind it.
    let take = verbs.iter().find(|e| e.word == "take").expect("take");
    assert!(take.answers_to("get"), "{what}: `get` is the take row's: {:?}", take.synonyms);
    assert!(!verbs.iter().any(|e| e.word == "get"), "{what}: …and not a row of its own");
    let mut words: Vec<&str> = verbs.iter().map(|e| e.word.as_str()).collect();
    words.sort();
    words.dedup();
    assert_eq!(words.len(), verbs.len(), "{what}: no verb listed twice");

    // The story tier is exactly the verbs the story's own text uses.
    let vocab = state.vocab.get(&session).expect("a grammar").clone();
    let text = vocab.text_words();
    let mentioned =
        |e: &VerbEntry| std::iter::once(&e.word).chain(&e.synonyms).any(|s| text.contains(s));
    let story = tier(verbs, VerbTier::Story);
    let more = tier(verbs, VerbTier::More);
    eprintln!("{what}: core {core:?}\n  story ({}) {story:?}\n  more ({}) {more:?}", story.len(), more.len());
    assert!(!story.is_empty() && !more.is_empty(), "{what}: both lower tiers are populated");
    for e in verbs.iter().filter(|e| e.tier != VerbTier::Core) {
        assert_eq!(
            e.tier == VerbTier::Story,
            mentioned(e),
            "{what}: `{}` is Story exactly when the story's text uses it",
            e.word
        );
    }

    // Nothing is dropped: every grammar spelling still reaches a row, bar the
    // two display filters (sigils, the adult list) that always applied.
    let hidden = state.config.hidden_display_words();
    for v in vocab.verbs() {
        for w in v.words.iter().filter(|w| w.chars().count() >= 2) {
            let w = w.to_lowercase();
            if w.starts_with(['#', '$']) || hidden.iter().any(|h| h.eq_ignore_ascii_case(&w)) {
                continue;
            }
            let shown = vocab.spell(&w);
            assert!(
                verbs.iter().any(|e| e.answers_to(shown))
                    // A core row is named by the curated word, which may reach
                    // the verb through truncation rather than equal a key.
                    || verbs.iter().any(|e| vocab.verb_named(&e.word).is_some_and(|x| x.words.contains(&w))),
                "{what}: `{w}` still reaches a row"
            );
        }
    }

    // A verb the player types moves up, on the next turn.
    let typed = more[0].to_string();
    state.record_command(&format!("{typed} it"));
    state.begin_turn();
    let data = app::host::refresh_band_data(&mut state, &session);
    let row = data.verbs.iter().find(|e| e.word == typed).expect("still listed");
    assert_eq!(row.tier, VerbTier::Story, "{what}: `{typed}` was typed, so it ranks up");
}

#[test]
fn minizork_verbs_arrive_ranked() {
    the_band_is_ranked("minizork-r34-s871124.z3", "Mini-Zork r34");
}

#[test]
fn zork1_verbs_arrive_ranked() {
    the_band_is_ranked("zork1-r88-s840726.z3", "Zork I r88");
}

/// A story with no grammar keeps today's fallback: the built-in table, whole,
/// in its own order, all of it shown up front.
#[test]
fn no_grammar_keeps_the_fallback() {
    let Some(session) = boot("journey-r83-s890706.z6") else { return };
    let mut state = AppState::default();
    let data = app::host::refresh_band_data(&mut state, &session);
    assert_eq!(data.verb_source, VerbSource::Builtin);
    let builtin = app::render::command_band::default_verbs();
    assert_eq!(data.verbs, builtin.entries, "unchanged, entry for entry");
    assert!(data.verbs.iter().all(|e| e.tier == VerbTier::Core && e.synonyms.is_empty()));
}
