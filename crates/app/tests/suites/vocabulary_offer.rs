//! SQ-1041: the story's own vocabulary, offered when the parser cannot have
//! understood the player — the first feature to speak in Lanthorn's Guiding
//! Light.
//!
//! `assist_voice.rs` pins the REGISTER (whose line it is, on which surface, and
//! that `push_assist` is the only door). This pins the FEATURE: when it speaks,
//! what it says, and — more of these cases than any other kind — when it does
//! not.
//!
//! # What the cases are really guarding
//!
//! **That the detection never reads the game's prose.** Every family words its
//! refusal differently and a story may reword it entirely; Dr Ludwig answers an
//! unknown verb with "Why, I don't even know what that verb means!". The offer
//! fires there exactly as it does under Infocom's `I don't know the word "…".`,
//! because it is looking at the story's dictionary and not at its output.
//!
//! **That silence is the ordinary answer.** A suggestion on every failed turn is
//! wallpaper, and the register's own test is the twentieth firing. Most cases
//! below assert that nothing was said.
//!
//! # The specimens
//!
//! | fixture | engine | dictionary | what it shows |
//! |---|---|---|---|
//! | a pocket story built here | stub | 6 chars | the wiring, on a machine with no `stories/` |
//! | `crates/scott/tests/tiny_cave.dat` | Scott | 3 chars | the two-word adapter, in-repo |
//! | `stories/zork1-r88-s840726.z3` | Z-machine v3 | 6 Z-chars | truncation, and the prose that undoes it |
//! | `stories/Dr Ludwig and the Devil.gblorb` | Glulx | 9 chars | a story that rewords the refusal |
//! | `stories/adv14a.dat` | Scott | 4 chars | an offer from a two-word parser |
//!
//! `stories/` is gitignored commercial media, so the last three skip vacuously.
//! The first two do not, and they are what CI actually runs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use app::engine::Engine;
use app::state::{AppState, TranscriptKind};
use app::vocab::{Position, StoryVocabulary};
use grammar_model::{NounKind, Slot, SyntaxLine, Token, Verb, WordRoles};

// ── A story with no engine under it ─────────────────────────────────────────

fn roles(verb: bool, noun: bool) -> WordRoles {
    let mut r = WordRoles::default();
    r.verb = verb;
    r.noun = noun;
    r
}

/// A pocket Version 3 story: `light`/`burn`, `take`/`get`, and a lantern whose
/// dictionary key is the six characters such a story can hold.
fn pocket_vocabulary() -> StoryVocabulary {
    let noun = || Slot::one(Token::Noun(NounKind::Noun));
    let verbs = vec![
        Verb::new(
            255,
            0,
            vec!["light".into(), "burn".into()],
            vec![SyntaxLine::new(1, false, vec![noun()])],
        ),
        Verb::new(
            254,
            0,
            vec!["take".into(), "get".into()],
            vec![SyntaxLine::new(2, false, vec![noun()])],
        ),
    ];
    let mut words = BTreeMap::new();
    for w in ["light", "burn", "take", "get"] {
        words.insert(w.to_string(), roles(true, false));
    }
    for w in ["lanter", "lamp", "the"] {
        words.insert(w.to_string(), roles(false, true));
    }
    StoryVocabulary::new(verbs, words, BTreeSet::new(), 6)
}

/// An engine that is nothing but a vocabulary. Everything a turn would need is
/// `unreachable!` — this double is never driven, only asked what its story knows.
struct PocketStory;

impl Engine for PocketStory {
    fn story_vocabulary(&self) -> Option<StoryVocabulary> {
        Some(pocket_vocabulary())
    }
    fn submit(&mut self, _command: &str) -> app::session::TurnResult {
        unreachable!("this double is asked about its dictionary, never driven")
    }
    fn submit_key(&mut self, _key: app::engine::KeyInput) -> Option<app::session::TurnResult> {
        unreachable!("this double is asked about its dictionary, never driven")
    }
    fn take_transcript(&mut self) -> String {
        String::new()
    }
    fn pending_input(&self) -> app::session::InputKind {
        app::session::InputKind::Line
    }
    fn resume_save(&mut self, _ok: bool) -> app::session::TurnResult {
        unreachable!("no save path here")
    }
    fn resume_restore(&mut self, _data: Option<&[u8]>) -> app::session::TurnResult {
        unreachable!("no restore path here")
    }
    fn has_quit(&self) -> bool {
        false
    }
    fn screen(&self) -> app::engine::ScreenModel {
        unreachable!("this double draws nothing")
    }
    fn save_state(&self) -> app::engine::EngineSave {
        unreachable!("no save path here")
    }
    fn restore_state(&mut self, _s: &app::engine::EngineSave) -> Result<(), app::engine::EngineError> {
        unreachable!("no restore path here")
    }
    fn restore_game_save(&mut self, _b: &[u8]) -> Result<(), app::engine::EngineError> {
        unreachable!("no restore path here")
    }
    fn aux_data(&self) -> &BTreeMap<String, Vec<u8>> {
        unreachable!("no aux data here")
    }
    fn set_aux_data(&mut self, _d: BTreeMap<String, Vec<u8>>) {}
    fn aux_dirty(&self) -> bool {
        false
    }
    fn clear_aux_dirty(&mut self) {}
    fn current_location(&self) -> Option<app::engine::LocationInfo> {
        None
    }
    fn drain_screen_clear(&mut self) -> bool {
        false
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// A state with the story's last reply already in the transcript, which is where
/// `finish_command_turn` calls the offer from.
fn after(reply: &str) -> AppState {
    let mut s = AppState::default();
    s.assist_preamble_shown = true; // the introduction has its own case in assist_voice
    s.push_transcript_kind(reply, TranscriptKind::Story);
    s
}

fn assists(s: &AppState) -> Vec<String> {
    s.transcript
        .iter()
        .zip(&s.transcript_kinds)
        .filter(|(_, k)| **k == TranscriptKind::Assist)
        .map(|(l, _)| l.clone())
        .collect()
}

/// The feature, on a machine with no `stories/`: an unknown word, and the story's
/// own words underneath it.
#[test]
fn an_unknown_word_is_answered_with_what_the_story_knows() {
    let mut s = after("I don't know the word \"lanturn\".");
    app::vocab::offer_vocabulary(&mut s, &PocketStory, "take lanturn", true);
    assert_eq!(assists(&s), vec!["this story knows — lanter"]);
}

/// One line, in lanthorn's own words, and never in the parser's brackets or the
/// story's second person — the register, checked at the one place that writes it.
#[test]
fn the_offer_is_one_unbracketed_line_that_speaks_of_the_story() {
    let mut s = after("I don't know the word \"tkae\".");
    app::vocab::offer_vocabulary(&mut s, &PocketStory, "tkae lamp", true);
    let lines = assists(&s);
    assert_eq!(lines.len(), 1, "one line, on a pane that may be forty columns: {lines:?}");
    let line = &lines[0];
    assert!(!line.starts_with('['), "the brackets are the parser's voice: {line:?}");
    assert!(line.starts_with("this story knows — "), "{line:?}");
    assert!(!line.contains("You "), "the second person is the story's voice: {line:?}");
    assert_eq!(line.matches('·').count() + 1, line["this story knows — ".len()..].split(" · ").count());
    assert!(
        line["this story knows — ".len()..].split(" · ").count() <= 3,
        "three at most, or the player reads instead of playing: {line:?}"
    );
}

/// A word the story DOES hold is not a vocabulary problem. It may be out of
/// scope, or not a verb here, and answering either with near-misses is what makes
/// interactive-fiction help feel stupid.
#[test]
fn a_word_the_story_knows_is_never_answered() {
    let mut s = after("You can't see any lamp here!");
    app::vocab::offer_vocabulary(&mut s, &PocketStory, "take lamp", true);
    assert!(assists(&s).is_empty(), "{:?}", assists(&s));
}

/// Two unknown words is not a command with one word wrong in it — it is a
/// sentence about things this story has never heard of, or a name typed at a
/// prompt. Speaking into one of those is the expensive mistake.
#[test]
fn two_unknown_words_are_never_answered() {
    let mut s = after("I don't know the word \"lanturn\".");
    app::vocab::offer_vocabulary(&mut s, &PocketStory, "tkae lanturn", true);
    assert!(assists(&s).is_empty(), "{:?}", assists(&s));
}

/// Once a session. The twentieth `lanturn` is the register's own test, and a line
/// that fires every time is furniture.
#[test]
fn a_word_is_answered_once_a_session() {
    let mut s = after("I don't know the word \"lanturn\".");
    for _ in 0..20 {
        app::vocab::offer_vocabulary(&mut s, &PocketStory, "take lanturn", true);
    }
    assert_eq!(assists(&s).len(), 1, "{:?}", assists(&s));
    // …and a DIFFERENT unknown word still gets its answer.
    app::vocab::offer_vocabulary(&mut s, &PocketStory, "tkae lamp", true);
    assert_eq!(assists(&s).len(), 2, "{:?}", assists(&s));
}

/// A turn that printed nothing rejected nothing.
#[test]
fn a_silent_turn_is_never_answered() {
    let mut s = after("");
    app::vocab::offer_vocabulary(&mut s, &PocketStory, "take lanturn", false);
    assert!(assists(&s).is_empty(), "{:?}", assists(&s));
}

/// The player's switch reaches this feature, and reaches it before the story's
/// tables are read — so switching the light back on later still owes them the
/// answer rather than finding the word already marked as given.
#[test]
fn guidance_off_silences_the_offer_and_forgets_nothing() {
    let mut s = after("I don't know the word \"lanturn\".");
    s.config.guidance = false;
    app::vocab::offer_vocabulary(&mut s, &PocketStory, "take lanturn", true);
    assert!(assists(&s).is_empty(), "{:?}", assists(&s));

    s.config.guidance = true;
    app::vocab::offer_vocabulary(&mut s, &PocketStory, "take lanturn", true);
    assert_eq!(assists(&s), vec!["this story knows — lanter"]);
}

/// The story printed the whole word, so the truncated key is shown whole. The
/// same command answered `lanter` above, with nothing in the transcript to spell
/// it out of.
#[test]
fn a_truncated_key_is_spelled_out_of_the_storys_own_prose() {
    let mut s = after("A battery-powered brass lantern is on the trophy case.");
    app::vocab::offer_vocabulary(&mut s, &PocketStory, "take lanturn", true);
    assert_eq!(assists(&s), vec!["this story knows — lantern"]);
}

// ── The Scott Adams adapter, on an in-repo database ─────────────────────────

fn tiny_cave() -> Vec<u8> {
    include_bytes!("../../../scott/tests/tiny_cave.dat").to_vec()
}

/// A two-word parser has no grammar module and needs none: the adapter builds the
/// same neutral value the other two engines answer with. Its vocabulary lists pad
/// unused slots with `.`, which is a placeholder and not a word anybody could
/// type — falsified by keeping them, when `.` appears among the story's words.
#[test]
fn the_scott_adapter_answers_in_the_same_neutral_shape() {
    let session = app::scott_session::ScottSession::new(tiny_cave(), None)
        .expect("tiny_cave.dat is in the checkout");
    let v = session.story_vocabulary().expect("a Scott database always has a vocabulary");
    assert!(!v.is_empty());

    // Every verb reaches its own record, and a synonym reaches the same one.
    let take = v.verb_named("take").expect("tiny_cave knows TAKE");
    assert_eq!(take.words.first().map(String::as_str), Some("get"), "{:?}", take.words);
    assert!(take.words.iter().any(|w| w == "take"), "the synonym joins its canonical verb: {:?}", take.words);
    assert!(take.takes_bare() && take.max_nouns() == 1, "a two-word grammar is VERB [NOUN]");

    for verb in v.verbs() {
        for w in &verb.words {
            assert!(w.chars().any(char::is_alphanumeric), "`{w}` is a padding slot, not a word");
        }
    }

    // The database truncates to its own word length, and so does the snapshot.
    assert!(v.knows("score"), "the story's own verb");
    assert!(v.knows("scoreboard"), "and anything that truncates to it, as the parser sees it");
    assert!(!v.knows("xyzzy"));
}

/// Nothing is offered that the parser would reject — checked against a real
/// database rather than a hand-built one, across both positions.
#[test]
fn a_scott_offer_never_names_a_word_its_parser_would_refuse() {
    let session = app::scott_session::ScottSession::new(tiny_cave(), None)
        .expect("tiny_cave.dat is in the checkout");
    let v = session.story_vocabulary().expect("a Scott database always has a vocabulary");
    for typed in ["scoer", "taek", "pushing", "lanturn", "xyzzy"] {
        for pos in [Position::Opening, Position::Inside] {
            for w in v.offer(typed, pos, &[], &[]) {
                assert!(v.knows(&w), "{w:?} is not in tiny_cave's vocabulary");
            }
        }
    }
}

// ── Real stories. `stories/` is gitignored; these skip vacuously. ───────────

fn story(name: &str) -> Option<Vec<u8>> {
    let path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(name);
    match std::fs::read(&path) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            None
        }
    }
}

/// Drive a real story through the same two steps `finish_command_turn` takes —
/// the game's reply into the transcript, then the offer — and hand back every
/// assist line it produced.
fn play(session: &mut dyn Engine, commands: &[&str]) -> (AppState, Vec<String>) {
    let mut state = AppState::default();
    state.assist_preamble_shown = true;
    let _ = session.take_transcript();
    for cmd in commands {
        let r = session.submit(cmd);
        state.push_transcript_kind(&format!("> {cmd}"), TranscriptKind::Input);
        state.push_transcript_kind(r.transcript.trim_end_matches('\n'), TranscriptKind::Story);
        let printed = !r.transcript.trim().is_empty();
        app::vocab::offer_vocabulary(&mut state, &*session, cmd, printed);
    }
    let lines = assists(&state);
    (state, lines)
}

fn zork1() -> Option<app::session::GameSession> {
    let bytes = story("zork1-r88-s840726.z3")?;
    let mut s = app::session::GameSession::new_with_trace(
        bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)),
    )
    .expect("zork1-r88-s840726.z3 boots without a ZError");
    s.set_strip_prompt(false); // inline-prompt mode, the shipped default
    Some(s)
}

/// Zork I release 88 / serial 840726, eight turns in: walk to the Living Room so
/// the story has PRINTED `lantern`, then mistype it. The dictionary holds
/// `lanter` and the offer says `lantern`, which is the word to type and the word
/// the parser matches.
///
/// The walk is the fixture. Falsify by asking before the Living Room — the offer
/// is still right and reads `lanter`, which is the case above.
#[test]
fn zork1_answers_a_mistyped_lantern_with_the_word_it_printed() {
    let Some(mut s) = zork1() else { return };
    let (_state, lines) = play(
        &mut s,
        &["north", "east", "open window", "enter window", "west", "take lanturn"],
    );
    assert_eq!(lines, vec!["this story knows — lantern"]);
}

/// Three shapes of miss on one story, and the four kinds of turn that must stay
/// quiet — including `xyzzy`, which Zork answers with a hollow voice rather than
/// a refusal, and `unlock chest with key`, whose words the story all knows.
#[test]
fn zork1_answers_the_misses_it_can_and_stays_quiet_otherwise() {
    let Some(mut s) = zork1() else { return };
    let (_state, lines) = play(
        &mut s,
        &[
            "opne mailbox",  // a transposition in the opening word
            "smel mailbox",  // a dropped letter, and a verb with its own synonym
            "opening mailbox", // an ending the story does not inflect
            "xyzzy",         // known, and not a refusal at all
            "take leaflet",  // understood
            "unlock mailbox with key", // every word known; the failure is not vocabulary
            "marcus",        // a name: near nothing, so nothing is said
        ],
    );
    assert_eq!(
        lines,
        vec![
            "this story knows — open",
            "this story knows — smell · sniff",
            "this story knows — open",
        ],
        "three offers and four silences"
    );
}

/// **What the whole synonym effort was for**, on the game everybody meets first.
/// `illuminate` is eight keystrokes from `light`, stems to nothing, and Zork's
/// grammar relates them not at all — every source that reads FORM is blind to
/// it, and the shipped table (SQ-1110, SQ-1115) is what closes the gap. This is
/// the wire SQ-1041 left hanging and SQ-1119 ran.
///
/// `inspect` is the one the player reported. `doff` is the case that says the
/// answer is not a fragment: `remove` is exactly the six characters a Version 3
/// dictionary keeps, and the aside rule discarded it until a word's WHOLENESS
/// travelled with it — without that, this reads `carry · remove · catch`, an
/// answer that has thrown away its own reason for existing.
///
/// Falsify by removing `by_meaning` from `candidates`: all four fall silent.
///
/// **SQ-1238 briefly changed the first and third lines** to `light · light
/// up` and `hold in · hold back · hide` — `light up` and `hold in`/`hold
/// back` are genuine WordNet members of `illuminate`'s and `conceal`'s
/// groups, and every one of their WORDS is a genuine Zork dictionary entry,
/// so the dictionary-only check that quest shipped credited them.
///
/// **SQ-1240 closes `light up`**, because `light`'s only grammar line
/// (`light OBJ with OBJ`) never pairs it with `up` — verified against
/// `infodump`-shaped output (`cargo run -p zvm --example grammar_dump --
/// stories/zork1-r88-s840726.z3`): `191. 2 entries, verb = "light" / light
/// OBJ with OBJ / light OBJ`.
///
/// **It does NOT close `hold in`**, and this is not a hole SQ-1240 leaves —
/// it is what "this story knows" honestly means. `hold` reaches `carry`
/// (`238. 8 entries, verb = "carry", synonyms = catch, get, grab, hold,
/// remove, take`), and Zork's own grammar for it includes a bare `carry in
/// OBJ` line beside `carry OBJ in OBJ` — so `carry in the trophy` truly
/// parses on this release, exactly the shape `hold in <noun>` would need.
/// The grammar genuinely pairs `carry` with `in`; it simply pairs it for an
/// unrelated sense of "carry" than the one `hold in` (conceal) means, which
/// is precisely the gap "this story knows" (a fact about the dictionary and
/// grammar) leaves for `try instead` (a fact about behaviour, from the
/// shadow probe) to close. `hold back`, by contrast, stays gone: `back` is
/// nowhere in `carry`'s literal list.
#[test]
fn zork1_answers_a_word_it_never_heard_with_what_that_word_means() {
    let Some(mut s) = zork1() else { return };
    let (_state, lines) = play(
        &mut s,
        &["illuminate lamp", "inspect lamp", "conceal lamp", "doff sword"],
    );
    assert_eq!(
        lines,
        vec![
            "this story knows — light",
            "this story knows — examine · describe · see",
            "this story knows — hold in · hide · carry",
            "this story knows — remove · carry · catch",
        ]
    );
}

/// SQ-1113: an IRREGULAR inflection, on the game everybody meets first. `took`
/// is `take` by no rule at all — there is nothing to strip off it — which is why
/// `stems` reached nothing here until WordNet's exception list shipped, and why
/// the near miss cannot stand in: `took` is three keystrokes from `take` and
/// that threshold is one, on purpose.
///
/// Zork I then adds its own synonyms for the verb once it is identified, which
/// is the aside source doing its usual work on top.
///
/// Every word here is out of reach of the near miss as well as of the rule —
/// `threw` was tried and dropped, because a single substitution reaches `throw`
/// and the case would have passed with the table removed.
///
/// Falsify by dropping the `irregular_bases` loop from `vocab::stems`: all three
/// lines fall silent.
#[test]
fn zork1_answers_an_irregular_inflection_with_the_verb_it_knows() {
    let Some(mut s) = zork1() else { return };
    let (_state, lines) = play(&mut s, &["took lamp", "broke lamp", "caught rope"]);
    assert_eq!(
        lines,
        vec![
            "this story knows — take · look · carry",
            "this story knows — break · block · smash",
            "this story knows — catch · carry · get",
        ]
    );
}

/// And `lit lamp` — the line the quest was NAMED for (SQ-1144).
///
/// This case is the INVERSION of the one SQ-1113 left here, which pinned the
/// same command as silent. Nothing about the table changed: `irregular_bases`
/// reached `light` then and reaches it now, and Zork has always held the word.
/// What stood between them was `MIN_LEN`, applied at the top of `offer` to every
/// source alike — an argument about EDIT DISTANCE (at three letters every
/// dictionary has a neighbour one keystroke away) deciding the fate of a lookup
/// that measures no distance at all. `lit` → `light` is a morphological fact
/// from WordNet's own exception list, and it is exactly as much a fact at three
/// letters as `caught` → `catch` is at six.
///
/// The gate now lives in `by_near_miss`, which is the only source it was ever
/// reasoning about; the silence half of that move is pinned three cases below.
///
/// Falsify by putting the length test back at the top of `StoryVocabulary::offer`:
/// this falls silent again, which is precisely the state SQ-1113 recorded.
#[test]
fn zork1_answers_a_three_letter_irregular_now_that_length_gates_only_the_near_miss() {
    let Some(mut s) = zork1() else { return };
    let (_state, lines) = play(&mut s, &["lit lamp"]);
    assert_eq!(lines, vec!["this story knows — light"]);

    assert_eq!(verb_synonyms::irregular_bases("lit"), ["light"], "the table reaches it");
    let v = <app::session::GameSession as Engine>::story_vocabulary(&s).expect("zork1 has one");
    assert!(v.knows("light"), "and the story holds the word it reaches");
}

/// The other three-letter irregulars the quest was filed on, on the same story:
/// `ate`, `saw`, `won`, `got`. One command each, because the offer speaks once
/// per word per session and a second `ate` would be swallowed by that rule
/// rather than by anything this quest touched.
///
/// Each is a form no suffix rule can produce and the near miss cannot reach —
/// `saw` is two keystrokes from `see`, `won` two from `win` — so every line here
/// is WordNet's exception list and the story's own dictionary, and nothing else.
#[test]
fn zork1_answers_the_rest_of_the_three_letter_irregulars() {
    let Some(mut s) = zork1() else { return };
    let (_state, lines) = play(&mut s, &["ate lamp", "saw lamp", "won lamp", "got lamp"]);
    assert_eq!(
        lines,
        vec![
            "this story knows — eat · bite · taste",
            "this story knows — see · find · seek",
            "this story knows — win",
            "this story knows — get · carry · catch",
        ]
    );
}

/// **The silence half, and the half that mattered more.** Three letters is where
/// noise is cheapest to generate, so SQ-1144 had to show what stayed quiet as
/// well as what started speaking.
///
/// * `take lam` — `lam` is one keystroke from the `lamp` this story holds, and
///   equally from `jam`, `ram`, `lab` and `am`. That is what `MIN_LEN` is *for*,
///   and it is untouched: the near miss still refuses anything under four.
/// * `oaf lamp` — a word in no table at all, near nothing. The ordinary answer.
/// * `sum lamp` — the table PROPOSES (`sum up`, `summarize`, `tally`) and the
///   story DISPOSES: Zork holds none of them. That rule is not relaxed at three
///   letters either, which is the whole reason no censorship of the table is
///   needed.
///
/// Falsify the first by deleting the `MIN_LEN` guard from `by_near_miss`: `take
/// lam` starts answering `lamp`, and every three-letter word in the game becomes
/// a suggestion.
#[test]
fn zork1_stays_quiet_at_three_letters_where_the_evidence_is_a_distance_or_absent() {
    let Some(mut s) = zork1() else { return };
    let (_state, lines) = play(&mut s, &["take lam", "oaf lamp", "sum lamp"]);
    assert!(lines.is_empty(), "{lines:?}");

    let v = <app::session::GameSession as Engine>::story_vocabulary(&s).expect("zork1 has one");
    assert!(v.knows("lamp"), "the near miss `lam` is one keystroke from a word right here");
    assert!(verb_synonyms::suggest("oaf", |_| true, 3).is_empty(), "`oaf` is in no group");
    assert_eq!(
        verb_synonyms::suggest("sum", |_| true, 3),
        ["sum up", "summarize", "tally"],
        "`sum` IS in a group"
    );
    assert!(
        !v.knows("summarize") && !v.knows("tally"),
        "and Zork holds nothing that group proposes"
    );
}

/// And the silences the meaning source must keep, on the same story — the ones
/// it is MOST able to erode, because a table of three thousand groups can always
/// find something.
///
/// `purchase` and `hint` are in the table and answered by nothing, because Zork
/// I's dictionary holds neither `buy` nor `help`: the intersection in `offer` is
/// what makes the feature honest, and it is the whole reason no censorship of
/// the table is needed. `marcus` is a name and reaches the table not at all.
///
/// `don sword` stood in this list until SQ-1144 and has moved to the case below,
/// because it never belonged with these: `don` means `wear` and Zork DOES hold
/// `wear`. It was silent on LENGTH, not on evidence — the one refusal here that
/// was not the story disposing of what the table proposed.
#[test]
fn zork1_stays_quiet_where_meaning_reaches_nothing_the_story_implements() {
    let Some(mut s) = zork1() else { return };
    let (_state, lines) = play(&mut s, &["purchase lamp", "hint", "marcus"]);
    assert!(lines.is_empty(), "{lines:?}");

    let v = <app::session::GameSession as Engine>::story_vocabulary(&s).expect("zork1 has one");
    assert!(!v.knows("buy") && !v.knows("help"), "the two the table proposes and Zork lacks");
}

/// And `don sword` is answered (SQ-1144), on the same reasoning as `lit`: an
/// exact hit in the synonym table is a lookup, and a lookup does not get weaker
/// as the word gets shorter. Zork holds `wear`; the table says `don` means it;
/// there was never anything between them but the length gate.
///
/// Its unit-test twin — `the_canonical_meanings_reach_the_word_the_story_holds`
/// in `vocab.rs` — pinned the same refusal on a synthetic story and is inverted
/// with it.
///
/// **SQ-1238 briefly added `get into` and `put on` to the line** — both are
/// members of `don`'s group, and Zork's dictionary genuinely holds every one
/// of `get`, `into`, `put` and `on`, which is all SQ-1238's per-word
/// dictionary check asked. **SQ-1240 closes `get into`**: `get` reaches
/// `carry`, and `carry`'s grammar (`carry OBJ from OBJ` / `off` / `out` /
/// `up` / `on` / `in`, and bare `carry up|on|out|in OBJ`) never pairs it with
/// `into` at all. **It does not close `put on`**, for the same honest reason
/// as `hold in` on `zork1_answers_a_word_it_never_heard_with_what_that_word_
/// means`: `put` reaches `hide`, and `hide`'s own grammar has a bare `hide on
/// OBJ` line beside `hide OBJ on OBJ`, so `hide on the rug` truly parses —
/// Zork's grammar genuinely pairs `hide` with `on`, just not for the sense
/// `put on` (wear) means. That is the dictionary-and-grammar fact "this
/// story knows" states; whether typing `put on` actually dresses the player
/// is exactly what the vetted `try instead` line is for.
#[test]
fn zork1_answers_a_three_letter_synonym_the_story_holds() {
    let Some(mut s) = zork1() else { return };
    let (_state, lines) = play(&mut s, &["don sword"]);
    assert_eq!(lines, vec!["this story knows — wear · put on · hide"]);

    let v = <app::session::GameSession as Engine>::story_vocabulary(&s).expect("zork1 has one");
    assert!(v.knows("wear"), "the word the table reaches, and Zork's own");
}

/// **The case the whole detection design is for.** Dr Ludwig and the Devil
/// rewords Inform's refusal completely — "Why, I don't even know what that verb
/// means!" — and the offer fires anyway, because it asked the dictionary and
/// never read the reply.
///
/// Falsify by matching on the printed text instead: this story says none of the
/// things any such matcher would look for.
#[test]
fn a_glulx_story_that_rewords_the_refusal_is_answered_all_the_same() {
    let Some(bytes) = story("Dr Ludwig and the Devil.gblorb") else { return };
    let b = blorb::Blorb::parse(bytes).expect("a gblorb parses");
    let exec = b.executable().expect("an Exec chunk").1.to_vec();
    let mut s =
        app::glulx_session::GlulxSession::new(exec, 80, 24, true, false, false, (1, 1), Some(b), &[])
            .expect("Dr Ludwig boots");
    s.set_strip_prompt(false);
    // Its opening runs on keypresses; step past them to the first line prompt.
    for _ in 0..12 {
        if s.pending_input() != app::session::InputKind::Char {
            break;
        }
        let _ = s.submit_key(app::engine::KeyInput::Enter);
    }
    let (state, lines) = play(&mut s, &["opne door", "examien me"]);
    assert!(
        state.transcript.iter().any(|l| l.contains("I don't even know what that verb means")),
        "the specimen is this story's OWN wording of the refusal"
    );
    assert!(
        !state.transcript.iter().any(|l| l.contains("I don't know the word")),
        "and it never uses the wording a text matcher would have looked for"
    );
    assert_eq!(
        lines,
        vec!["this story knows — open · uncover · unwrap", "this story knows — examine · check · describe"],
        "the story's own synonym groups, free, once a verb is identified"
    );
}

/// A two-word parser, end to end. `adv14a.dat` keeps four characters of a word,
/// which is enough for a near miss to mean something — three keeps so little that
/// the parser refuses almost nothing, and the offer correctly never speaks there.
#[test]
fn a_scott_story_answers_a_mistyped_verb() {
    let Some(bytes) = story("adv14a.dat") else { return };
    let mut s = app::scott_session::ScottSession::new(bytes, None).expect("adv14a.dat loads");
    let (_state, lines) = play(&mut s, &["quti", "loko"]);
    assert_eq!(
        lines,
        vec!["this story knows — quit", "this story knows — look"],
        "a fragment (`exam`, `desc`) is fit to be the answer and not fit to be an aside"
    );
}

/// **SQ-1238.** The shipped synonym table groups `hasten` with `rush`,
/// `hurry` and the phrasal `look sharp`. `ten_indians.blb`'s Scott Adams
/// dictionary keeps four characters and implements `look` but none of `rush`,
/// `hurry` or `sharp` — and before the fix, truncating the whole PHRASE
/// `"look sharp"` to four characters landed on the very same key `"look"`
/// truncates to, so the offer named `look sharp` (and `look`'s own aliases,
/// riding along through `by_story_synonym`) though no release of this game
/// implements any of them.
///
/// `adv03.dat` is the fixture the quest was filed on, but it is not the
/// specimen here: its dictionary truncates at THREE characters, where it
/// happens to hold an unrelated verb (`SHA`) that `sharp` also truncates to —
/// a second, independent truncation collision genuinely present in that
/// story's own dictionary (`stored("sharp")` truly resolves there), which was
/// not the mechanism SQ-1238 fixed and was not closed by it. SQ-1240 closes
/// it anyway, from an entirely different direction: see
/// [`adv03_credits_no_phrasal_member_because_scott_adams_has_no_prepositions`]
/// just below.
///
/// Falsify by reverting the `stored` fix: `hasten north` on `ten_indians.blb`
/// starts naming `look sharp` again.
#[test]
fn a_scott_story_does_not_credit_a_phrasal_synonym_through_truncation() {
    let Some(bytes) = story("ten_indians.blb") else { return };
    let loaded = app::hints::extract_story(bytes).expect("ten_indians.blb extracts a Scott exec");
    let app::hints::LoadedStory::Scott(data) = loaded else {
        panic!("ten_indians.blb is a Scott Adams blorb")
    };
    let mut s = app::scott_session::ScottSession::new(data, None).expect("ten_indians.blb loads");
    let (_state, lines) = play(&mut s, &["hasten north"]);
    assert!(
        lines.is_empty(),
        "no release of this game implements `rush`, `hurry` or `look sharp`: {lines:?}"
    );
}

/// **SQ-1240, on the fixture the quest was actually filed on.** `adv03.dat`
/// truncates at THREE characters, and `sharp` truncates to `sha` — a real but
/// unrelated verb in this story's own dictionary, so SQ-1238's per-word
/// dictionary check alone could not tell `look sharp` apart from a story that
/// genuinely implements it: every word of the phrase "resolves". What closes
/// it is that a Scott Adams database has NO prepositions at all — its
/// `SyntaxLine`s are always `VERB` or `VERB noun`, never `VERB word noun`
/// (see `scott_session::story_vocabulary`) — so `Verb::prepositions()` is
/// empty for every verb this format can produce, and no multi-word synonym
/// member can ever pair with one. `look` on its own is still offered nowhere
/// near this: `hasten` never matches `look` directly, only through the
/// `rush`/`hurry`/`look sharp` group, and every member of that group is
/// closed to this story.
///
/// Falsify by reverting the SQ-1240 grammar-pairing check: `hasten north`
/// starts naming `look sharp` again, exactly as it did on `ten_indians.blb`
/// before SQ-1238.
#[test]
fn adv03_credits_no_phrasal_member_because_scott_adams_has_no_prepositions() {
    let Some(bytes) = story("adv03.dat") else { return };
    let mut s = app::scott_session::ScottSession::new(bytes, None).expect("adv03.dat loads");
    let (_state, lines) = play(&mut s, &["hasten north"]);
    assert!(
        lines.is_empty(),
        "no Scott Adams game can implement a multi-word synonym member: {lines:?}"
    );
}

// ── Words no player can type (SQ-1151) ──────────────────────────────────────

/// The seam drops nothing when the engine lends no tokeniser, which is the whole
/// safety property of the rule: without the story's own splitter there is no
/// authority to drop a word on, so `PocketStory` — which answers `None` to
/// `split_like_parser`, as Glulx and Scott Adams do — keeps every word it had.
///
/// This is the CI-safe half. The half that shows a word actually going is below,
/// on the two stories that hold one.
#[test]
fn a_story_that_lends_no_tokeniser_keeps_every_word() {
    assert!(
        PocketStory.split_like_parser("anything").is_none(),
        "the premise: this double has no tokeniser to lend"
    );
    let kept = pocket_vocabulary().without_untypeable_words(&PocketStory);
    for w in ["light", "burn", "take", "get", "lanter", "lamp", "the"] {
        assert!(kept.roles(w).is_some(), "{w} survives an engine with no opinion");
    }
    let spellings: Vec<&str> =
        kept.verbs().iter().flat_map(|v| v.words.iter().map(String::as_str)).collect();
    assert_eq!(spellings, vec!["light", "burn", "take", "get"], "and so does every verb");
}

/// **Moonmist release 9 / serial 861022 is the case that proves the separator set
/// has to come from the story.** It declares `'` an input separator, so its own
/// tokeniser cuts `dee's` into `dee`, `'`, `s` before the parser looks anything
/// up — nineteen possessive entries in its dictionary that no sequence of
/// keystrokes can reach.
///
/// Enchanter release 29 / serial 860820 is the control: a Version 3 story of the
/// same vintage that does NOT declare `'`, where `dee's` would be one word. A
/// fixed table of separators gets one of these two wrong whichever way it is
/// written, which is why the rule asks [`Engine::split_like_parser`] — the code
/// path `read` itself calls — instead.
///
/// Falsify by dropping `without_untypeable_words` from `VocabState::get`: the
/// raw snapshot below still holds every one of these, and the assertion on the
/// seam is what fails.
#[test]
fn moonmist_declares_the_apostrophe_a_separator_so_its_possessives_never_reach_a_player() {
    let Some(bytes) = story("moonmist-r9-s861022.z3") else { return };
    let session = app::session::GameSession::new_with_trace(
        bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)),
    )
    .expect("moonmist-r9-s861022.z3 boots without a ZError");

    // The story's own answer, first — this is the fact the rule is reading.
    assert_eq!(
        session.split_like_parser("dee's"),
        Some(vec!["dee".to_string(), "'".to_string(), "s".to_string()]),
        "Moonmist's dictionary header declares `'` a separator"
    );
    // Non-vacuity: the entries really are in the dictionary, un-filtered.
    let raw = <app::session::GameSession as Engine>::story_vocabulary(&session)
        .expect("a readable Version 3 dictionary");
    for w in ["dee's", "iris'", "b's"] {
        assert!(raw.roles(w).is_some(), "{w:?} is a real entry in Moonmist's dictionary");
    }

    // …and the seam every surface reads has none of them.
    let mut state = AppState::default();
    let seam = state.vocab.get(&session).expect("the seam still has a vocabulary");
    for w in ["dee's", "iris'", "b's"] {
        assert!(seam.roles(w).is_none(), "{w:?} cannot be typed, so it is never offered");
    }
    // The rule is CONTAINS, not IS: a word that is itself a separator tokenises
    // to itself and stays. Moonmist files the bare apostrophe as a real token.
    assert_eq!(session.split_like_parser("'"), Some(vec!["'".to_string()]));
    assert_eq!(
        raw.roles("'").is_some(),
        seam.roles("'").is_some(),
        "a word that IS a separator is not a word that CONTAINS one"
    );
    // And an ordinary word is untouched, so this is not a filter that ate the
    // dictionary.
    for w in ["lamp", "north", "tamara"] {
        assert_eq!(raw.roles(w).is_some(), seam.roles(w).is_some(), "{w:?} is unaffected");
    }
}

/// Enchanter, the control for the case above: the SAME spelling is one word here,
/// because this story declares no `'`. Nothing is dropped on its account.
#[test]
fn enchanter_does_not_declare_the_apostrophe_and_keeps_the_same_spellings() {
    let Some(bytes) = story("enchanter-r29-s860820.z3") else { return };
    let session = app::session::GameSession::new_with_trace(
        bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)),
    )
    .expect("enchanter-r29-s860820.z3 boots without a ZError");
    assert_eq!(
        session.split_like_parser("dee's"),
        Some(vec!["dee's".to_string()]),
        "release 29 declares no apostrophe, so the same spelling is one token"
    );
}
