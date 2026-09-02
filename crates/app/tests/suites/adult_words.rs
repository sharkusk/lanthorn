//! SQ-1122: the adult list — `hide_adult_words` and `adult_words`.
//!
//! SQ-1111 pointed the command band's VERB column at the running story's own
//! grammar, and Zork I r88 went from 23 generic verbs to 258 of its own. Infocom
//! shipped a swear block in every dictionary, so the panel started listing
//! `fuck`, `shit`, `rape` and `molest` to anybody who pressed F2.
//!
//! # The line this draws, and the one it does not
//!
//! This is **not** a reversal of SQ-1115's "faithful, we don't censor" ruling.
//! That governs the generated synonym data and the Guiding Light, where a word
//! can only ever surface because the player typed something that mapped to it
//! AND the story implements it. Here the words are simply *enumerated*, unbidden,
//! to somebody who opened a panel. The principle the two share:
//!
//! > unprompted enumeration gets a default; what the player reached for does not.
//!
//! Which is why the keys are TOP-LEVEL rather than `[command_panel]`'s: SQ-1107's
//! momentary reveal is another unprompted enumeration, and one setting they all
//! read beats each surface growing its own and drifting.
//!
//! # The specimens
//!
//! | fixture | why |
//! |---|---|
//! | a pocket grammar built here | the display-only pin, on a machine with no `stories/` |
//! | `stories/zork1-r88-s840726.z3` (r88, s840726) | the story that raised the quest; `fuck`/`shit`/`rape`/`molest` are real verbs in it, `damn` and `barf` are real verbs that stay |
//!
//! `stories/` is gitignored commercial media, so the Zork I cases skip
//! vacuously; the pocket ones are what CI actually runs.

use std::collections::{BTreeMap, BTreeSet};

use app::config::{Config, DEFAULT_ADULT_WORDS};
use app::engine::Engine;
use app::graphics::PictSource;
use app::render::command_band::{verbs_from_grammar, VerbSource, VerbTable};
use app::session::GameSession;
use app::state::{AppState, TranscriptKind};
use app::vocab::{Position, StoryVocabulary};
use grammar_model::{NounKind, Slot, SyntaxLine, Token, Verb, WordRoles};

use crate::fixture_paths::fixture_path;

// ── A pocket grammar that holds one of the words ─────────────────────────────

fn roles(verb: bool, noun: bool) -> WordRoles {
    let mut r = WordRoles::default();
    r.verb = verb;
    r.noun = noun;
    r
}

/// Three verbs, one of them on the default list. `molest` is the one Infocom put
/// in almost every game and the one the user named explicitly; `move` sits beside
/// it as the innocent word a filter must not touch.
fn pocket_verbs() -> Vec<Verb> {
    let noun = || Slot::one(Token::Noun(NounKind::Noun));
    vec![
        Verb::new(
            255,
            0,
            vec!["molest".into(), "move".into()],
            vec![SyntaxLine::new(1, false, vec![noun()])],
        ),
        Verb::new(
            254,
            0,
            vec!["take".into(), "get".into()],
            vec![SyntaxLine::new(2, false, vec![noun()])],
        ),
        Verb::new(253, 0, vec!["damn".into()], vec![SyntaxLine::new(3, false, vec![])]),
    ]
}

fn pocket_vocabulary() -> StoryVocabulary {
    let mut words = BTreeMap::new();
    for w in ["molest", "move", "take", "get", "damn"] {
        words.insert(w.to_string(), roles(true, false));
    }
    for w in ["lantern", "lamp"] {
        words.insert(w.to_string(), roles(false, true));
    }
    StoryVocabulary::new(pocket_verbs(), words, BTreeSet::new(), 9)
}

/// The band's column as the app assembles it: the story's grammar, `extra_verbs`
/// layered on, and then the adult list applied — `Config::layer_band_verbs`, the
/// one production route.
fn column(cfg: &Config, verbs: &[Verb]) -> Vec<String> {
    let table = VerbTable::new(verbs_from_grammar(verbs), VerbSource::Story);
    cfg.layer_band_verbs(table).entries.into_iter().map(|e| e.word).collect()
}

// ── The default ──────────────────────────────────────────────────────────────

/// The switch is on out of the box, and the list is the strong end only.
#[test]
fn the_default_is_on_and_holds_only_the_strong_end() {
    let cfg = Config::default();
    assert!(cfg.hide_adult_words, "the filter is the default, which is the whole quest");
    assert_eq!(
        cfg.adult_words,
        DEFAULT_ADULT_WORDS.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        "the shipped list is the constant, so the config template can state it"
    );

    // Infocom being Infocom stays visible — the user's own examples, plus the
    // coarse-but-not-obscene words the corpus is full of.
    for mild in ["damn", "barf", "hell", "crap", "screw", "suck", "piss", "pee", "sod", "fart"] {
        assert!(
            !cfg.adult_words.iter().any(|w| w == mild),
            "`{mild}` is coarse, not obscene, and stays in the column"
        );
    }
    // The two that are not swearing at all, and are on the list anyway.
    for named in ["rape", "molest"] {
        assert!(cfg.adult_words.iter().any(|w| w == named), "`{named}` is on the list");
    }
    assert!(cfg.adult_words.len() <= 20, "short enough to read at a glance");
    assert!(
        cfg.adult_words.iter().all(|w| w.chars().all(|c| c.is_ascii_lowercase())),
        "the list is matched case-insensitively, so it is written in one case: {:?}",
        cfg.adult_words
    );
}

// ── DISPLAY ONLY — the load-bearing property ─────────────────────────────────

/// The same word, hidden from the panel and still offered by the light.
///
/// This is the property a future lane will want to "unify" away: the band's
/// column and `vocab`'s suggestion both draw on the story's dictionary, so it
/// looks like one filter would do for both. It would not. A word the player
/// reached for is not an unprompted enumeration, and SQ-1115 rules the offer.
///
/// Falsify by having `StoryVocabulary::offer` consult `Config::adult_words`:
/// the second half of this case fails and nothing else in the suite moves.
#[test]
fn a_word_hidden_from_the_panel_is_still_offered_by_the_light() {
    let cfg = Config::default();

    // The panel: `molest` is gone, its innocent synonym `move` is not.
    let shown = column(&cfg, &pocket_verbs());
    assert!(!shown.contains(&"molest".to_string()), "hidden from the column: {shown:?}");
    assert!(shown.contains(&"move".to_string()), "its sibling spelling stays: {shown:?}");
    assert!(shown.contains(&"damn".to_string()), "and so does Infocom's own salt");

    // The light: the player typed `molst`, and the story does hold the word.
    let vocab = pocket_vocabulary();
    assert!(vocab.knows("molest"), "the story knows it — that never changed");
    let offer = vocab.offer("molst", Position::Opening, &[], &[]);
    assert!(
        offer.contains(&"molest".to_string()),
        "a word the player reached for is still offered: {offer:?}"
    );
}

/// The suggestion path takes no configuration at all, which is why the case
/// above holds structurally and not by luck: `StoryVocabulary::offer` has the
/// story's tables and the typed line, and nothing else to consult.
///
/// A source-level case, in the spirit of `palette_lock_discipline`: the next
/// person to wire a filter into `vocab.rs` has no reason to know any of this.
#[test]
fn the_suggestion_path_never_reads_the_adult_list() {
    let src = include_str!("../../src/vocab.rs");
    for needle in ["adult_words", "hide_adult_words", "hidden_display_words"] {
        assert!(
            !src.contains(needle),
            "`{needle}` reached crates/app/src/vocab.rs — the Guiding Light answers a word \
             the player REACHED FOR, and SQ-1115 rules that half. Filter the panel, not the offer."
        );
    }
}

// ── The one half of the offer that is lanthorn's own voice (SQ-1145) ─────────

/// The same word, refused as a PROPOSAL and offered as a CORRECTION.
///
/// This is the whole of SQ-1145 in one case, and it is one case on purpose:
/// either half read alone is the opposite decision. The pair is the rule.
///
/// * `molst` → `molest` is a **correction**. The player typed those letters; the
///   story holds that word; naming it is answering a question they asked. The
///   case above rules it, SQ-1115 rules it, and nothing here touches it.
/// * `assault` → `molest` is a **proposal**. `assault` is a word this story has
///   never heard, and the meaning table answers it with a DIFFERENT word the
///   player never typed and never nearly typed. That is lanthorn choosing the
///   word — the same act as the band enumerating a column — so it answers to the
///   same list.
///
/// The two are separated by [`app::vocab::Pick::proposed`] and by nothing else,
/// which is why the provenance has to leave `vocab.rs` for the judgement to be
/// made anywhere.
///
/// Falsified both ways. Drop the filter from `Config::spoken_offer` and the
/// proposal half fails with the symptom the quest was filed on; have it ignore
/// `proposed` and filter every pick, and the correction half fails instead.
/// (Only this half — the case above reads `StoryVocabulary::offer` directly and
/// cannot see a `Config` at all, which is the point of where the filter sits.)
#[test]
fn a_proposed_word_answers_to_the_list_and_a_corrected_one_never_does() {
    let cfg = Config::default();
    let vocab = pocket_vocabulary();
    assert!(
        cfg.adult_words.iter().any(|w| w == "molest"),
        "the one word both halves turn on is on the shipped list"
    );

    // The proposal: a word the player never typed, and lanthorn does not say it.
    let proposed = vocab.offer_picks("assault", Position::Opening, &["lantern"], &[]);
    assert!(
        proposed.iter().any(|p| p.word == "molest" && p.proposed),
        "the meaning table reaches it, and says so: {proposed:?}"
    );
    assert!(
        !cfg.spoken_offer(proposed).contains(&"molest".to_string()),
        "…and it is not in lanthorn's own voice"
    );

    // The correction: the SAME word, reached for, and untouched by any of this.
    let reached = vocab.offer_picks("molst", Position::Opening, &["lantern"], &[]);
    assert!(
        reached.iter().any(|p| p.word == "molest" && !p.proposed),
        "one keystroke wrong is evidence about the word TYPED: {reached:?}"
    );
    assert!(
        cfg.spoken_offer(reached).contains(&"molest".to_string()),
        "…so it is still offered, on the default config"
    );
}

/// Both off-switches reach the offer too, the same way they reach the column —
/// one question (`hidden_display_words`), asked once, honoured everywhere.
#[test]
fn either_off_switch_restores_the_proposal() {
    let vocab = pocket_vocabulary();
    let picks = || vocab.offer_picks("assault", Position::Opening, &["lantern"], &[]);
    assert!(
        !Config::default().spoken_offer(picks()).contains(&"molest".to_string()),
        "the default hides it — the baseline this case is measured against"
    );
    for cfg in [
        Config { hide_adult_words: false, ..Config::default() },
        Config { adult_words: Vec::new(), ..Config::default() },
    ] {
        assert!(
            cfg.spoken_offer(picks()).contains(&"molest".to_string()),
            "switch off or list empty, the proposal comes back: {:?}",
            cfg.hidden_display_words()
        );
    }
}

/// And an innocent proposal is never touched, which is what keeps this a
/// four-word default rather than a mute button on the meaning table.
#[test]
fn a_proposal_of_a_word_not_on_the_list_is_said_as_it_always_was() {
    let cfg = Config::default();
    let vocab = pocket_vocabulary();
    // `shift` means `move`, and `move` is `molest`'s innocent sibling spelling —
    // the same verb entry, the same story, and nothing on the list.
    let picks = vocab.offer_picks("shift", Position::Opening, &["lantern"], &[]);
    assert!(
        picks.iter().any(|p| p.word == "move" && p.proposed),
        "a proposal, and an entirely ordinary one: {picks:?}"
    );
    assert!(cfg.spoken_offer(picks).contains(&"move".to_string()));
}

// ── Both off-switches ────────────────────────────────────────────────────────

/// `hide_adult_words = false` restores the full column AND keeps the words,
/// which is the reason the switch is a second key rather than an empty list.
#[test]
fn the_switch_off_restores_the_full_column_and_keeps_the_list() {
    let cfg = Config { hide_adult_words: false, ..Config::default() };
    let shown = column(&cfg, &pocket_verbs());
    assert!(shown.contains(&"molest".to_string()), "nothing hidden: {shown:?}");
    assert_eq!(
        cfg.adult_words.len(),
        DEFAULT_ADULT_WORDS.len(),
        "…and the list survived being switched off, so switching back needs no retyping"
    );
    assert!(cfg.hidden_display_words().is_empty(), "the one question every surface asks");
}

/// An empty `adult_words` restores it too. Same answer, different key — a caller
/// asks `hidden_display_words()` once and cannot honour one switch and miss the
/// other.
#[test]
fn an_empty_list_restores_the_full_column() {
    let cfg = Config { adult_words: Vec::new(), ..Config::default() };
    assert!(cfg.hide_adult_words, "the switch is still on — the LIST is what emptied");
    let shown = column(&cfg, &pocket_verbs());
    assert!(shown.contains(&"molest".to_string()), "nothing hidden: {shown:?}");
    assert!(cfg.hidden_display_words().is_empty());
}

/// And the list is the player's: a word they add is hidden, whatever it is.
#[test]
fn the_list_is_the_players_own() {
    let cfg = Config { adult_words: vec!["DAMN".to_string()], ..Config::default() };
    let shown = column(&cfg, &pocket_verbs());
    assert!(!shown.contains(&"damn".to_string()), "matched case-insensitively: {shown:?}");
    assert!(shown.contains(&"molest".to_string()), "…and only what the list now names");
}

/// Whole words only. Old dictionaries truncate — a v6 story's four-character
/// keys hold `bast` for *bastard* — and a prefix rule wide enough to catch that
/// also eats `rap` and `who`, real verbs in forty and twenty-five corpus stories.
/// Under-filtering is the instruction.
#[test]
fn matching_is_whole_word_never_a_prefix() {
    let noun = || Slot::one(Token::Noun(NounKind::Noun));
    let verbs = vec![Verb::new(
        255,
        0,
        vec!["rap".into(), "who".into(), "shitty".into(), "rape".into()],
        vec![SyntaxLine::new(1, false, vec![noun()])],
    )];
    let shown = column(&Config::default(), &verbs);
    assert!(!shown.contains(&"rape".to_string()), "the exact word goes");
    for kept in ["rap", "who", "shitty"] {
        assert!(shown.contains(&kept.to_string()), "`{kept}` is not on the list: {shown:?}");
    }
}

// ── The configuration surface ────────────────────────────────────────────────

/// Both keys are top-level, not `[command_panel]`'s — the principle is about
/// unprompted enumeration, not about the band (SQ-1117's argument).
#[test]
fn the_keys_are_top_level_and_round_trip() {
    let cfg: Config = toml::from_str(
        "hide_adult_words = false\nadult_words = [\"xyzzy\"]\n",
    )
    .expect("both keys parse at the top level");
    assert!(!cfg.hide_adult_words);
    assert_eq!(cfg.adult_words, vec!["xyzzy".to_string()]);

    // …and under `[command_panel]` they are nothing, which is the point of the
    // placement: one setting every enumerating surface reads.
    let band: Config = toml::from_str("[command_panel]\nhide_adult_words = false\n")
        .expect("an unknown key in a section is ignored, as every other one is");
    assert!(band.hide_adult_words, "the band section has no say in it");
}

/// The list ships UNCOMMENTED in the seeded config.toml, and that is the whole
/// reason this is a default rather than censorship: a player can read exactly
/// which words are hidden, shorten the line, or delete it.
#[test]
fn the_seeded_config_states_the_list_in_plain_sight() {
    let template = app::config_template::commented_template();
    let line = template
        .lines()
        .find(|l| l.trim_start().starts_with("adult_words"))
        .expect("the template names the list");
    assert!(!line.trim_start().starts_with('#'), "written LIVE, not commented: {line:?}");
    for w in DEFAULT_ADULT_WORDS {
        assert!(line.contains(&format!("\"{w}\"")), "`{w}` is visible in the file: {line}");
    }
    // The switch beside it is an ordinary commented default row like every other
    // boolean, because it IS the default and uncommenting it changes nothing.
    assert!(
        template.contains("# hide_adult_words = true"),
        "the switch is documented as the default it is"
    );
}

// ── Zork I r88: the story that raised the quest ──────────────────────────────

fn boot_zork1() -> Option<GameSession> {
    let path = fixture_path("zork1-r88-s840726.z3");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let std_window = picts.std_window();
    let mut session =
        GameSession::new_with_trace(bytes, true, false, None, false, dims, std_window, None, None)
            .expect("Zork I should load and boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    Some(session)
}

/// The reported symptom, on the story that produced it: four words out of a
/// 250-odd verb column, and everything else — Infocom's own `damn` and `barf`
/// included — still there.
#[test]
fn zork_i_s_verb_column_loses_four_words_and_keeps_the_rest() {
    let Some(session) = boot_zork1() else { return };
    let vocab = session.story_vocabulary().expect("Zork I's grammar reads");
    let verbs = vocab.verbs().to_vec();

    let unfiltered = column(&Config { hide_adult_words: false, ..Config::default() }, &verbs);
    let filtered = column(&Config::default(), &verbs);

    for gone in ["fuck", "shit", "rape", "molest"] {
        assert!(unfiltered.contains(&gone.to_string()), "Zork I really holds `{gone}`");
        assert!(!filtered.contains(&gone.to_string()), "`{gone}` is not enumerated");
    }
    for kept in ["damn", "barf", "curse", "kill", "murder", "dig", "pray", "take"] {
        assert!(filtered.contains(&kept.to_string()), "`{kept}` stays: {}", filtered.len());
    }
    assert_eq!(
        unfiltered.len() - filtered.len(),
        4,
        "four words, out of {}: this is a default, not a scrub",
        unfiltered.len()
    );
    assert!(filtered.len() > 200, "the whole grammar minus four: {}", filtered.len());
}

/// DISPLAY ONLY, against the real parser: typing a hidden word behaves exactly
/// as it did before. Zork I answers `fuck` with a scolding of its own; what
/// matters is that it is not `I don't know the word`.
#[test]
fn typing_a_hidden_word_still_reaches_zork_i_s_parser() {
    let Some(mut session) = boot_zork1() else { return };
    let reply = session.submit("fuck").transcript.to_lowercase();
    assert!(!reply.is_empty(), "the turn produced a reply");
    assert!(
        !reply.contains("don't know the word"),
        "the story still knows the word it always knew: {reply:?}"
    );
}

/// Drive the story through the same two steps `finish_command_turn` takes — the
/// game's reply into the transcript, then the offer — under `cfg`, and hand back
/// every assist line it produced. The production route, so the filter is
/// measured where the player would meet it and not at the seam it lives on.
fn offered(session: &mut GameSession, cfg: Config, commands: &[&str]) -> Vec<String> {
    let mut state = AppState::default();
    state.config = cfg;
    state.assist_preamble_shown = true;
    let _ = session.take_transcript();
    for cmd in commands {
        let r = session.submit(cmd);
        state.push_transcript_kind(&format!("> {cmd}"), TranscriptKind::Input);
        state.push_transcript_kind(r.transcript.trim_end_matches('\n'), TranscriptKind::Story);
        let printed = !r.transcript.trim().is_empty();
        app::vocab::offer_vocabulary(&mut state, &*session, cmd, printed);
    }
    state
        .transcript
        .iter()
        .zip(&state.transcript_kinds)
        .filter(|(_, k)| **k == TranscriptKind::Assist)
        .map(|(l, _)| l.clone())
        .collect()
}

/// The three words the SQ-1144 lane measured when the three-letter band opened,
/// on the story it measured them on, through the app's own turn hook.
///
/// | typed | proposed, in full | offered |
/// |---|---|---|
/// | `sod` | `shit · fuck · damn` | `damn` |
/// | `bed` | `fuck · set · curse` | `set · curse` |
/// | `don` | `wear · put on` | `wear · put on` |
///
/// The first two rows are the quest, and they are better than silence: the group
/// `sod` belongs to holds Infocom's own `damn` beside the two words on the list,
/// so the answer narrows rather than vanishing. The `don` row is the
/// load-bearing one — same source, same shape, same turn hook, nothing on the
/// list — and a filter that silenced the meaning TABLE rather than four WORDS
/// would pass the first two and fail it.
///
/// **The `don` row gained `put on` at `bad3ac28` (SQ-1238), and that is the
/// change working** (SQ-1251). Before it, a multi-word member of the meaning
/// table was truncated as one string and could never resolve, so `put on` was
/// invisible however plainly the story implemented it; `wear don put on get
/// into assume hat` is one group, and Zork I's own grammar really does pair
/// `put` with `on`, which is the test `025bdab6` (SQ-1240) then narrowed it to.
/// `get into` fails that narrower test and is correctly absent. What the row
/// must NOT gain is a third word: `hide` rode in with `put on` until SQ-1251,
/// because the story-synonym aside resolved the phrase through its first word
/// and offered `put`'s other spellings — and hiding a sword is not wearing it.
///
/// **And the `sod` row reads `shit · fuck` since `d28ce9d2` (SQ-1233)**, whose
/// support-count ordering put the more widely implemented word first in the
/// group `shit fuck damn sod`. The offer says a group in the table's own order,
/// so this line is the TABLE's, not the offer's — and the row that carries the
/// quest's claim is the filtered one beside it, `damn`, which never moved.
///
/// Three turns, one session, one command each — the offer speaks once per word
/// per session, so a repeat would be swallowed by that rule rather than by
/// anything here.
#[test]
fn zork_i_stops_proposing_a_hidden_word_and_keeps_every_other_proposal() {
    let Some(mut session) = boot_zork1() else { return };
    let vocab = session.story_vocabulary().expect("Zork I's grammar reads");
    for held in ["fuck", "set", "curse", "wear", "put on"] {
        assert!(vocab.knows(held), "Zork I holds `{held}` — the offer only names its own");
    }
    for typed in ["sod", "bed", "don", "get into"] {
        assert!(!vocab.knows(typed), "`{typed}` is a word Zork I never heard, which is the setup");
    }

    let hidden = offered(&mut session, Config::default(), &["sod", "bed", "don sword"]);
    assert_eq!(
        hidden,
        vec![
            "this story knows — damn",
            "this story knows — set · curse",
            "this story knows — wear · put on",
        ]
    );

    // The same three turns with the switch off, on a fresh session because the
    // offer speaks once per word per session.
    let Some(mut session) = boot_zork1() else { return };
    let shown = offered(
        &mut session,
        Config { hide_adult_words: false, ..Config::default() },
        &["sod", "bed", "don sword"],
    );
    assert_eq!(
        shown,
        vec![
            "this story knows — shit · fuck · damn",
            "this story knows — fuck · set · curse",
            "this story knows — wear · put on",
        ],
        "the unfiltered lines the SQ-1144 lane measured, which the switch restores whole"
    );
}

// ── The assembly points ──────────────────────────────────────────────────────

/// Every `VerbTable` that reaches the screen goes through `Config`, which is
/// where the list is applied.
///
/// `CommandBandConfig::resolve_verbs` and `layer_extra_verbs` know nothing about
/// the adult list — they cannot, the keys are top-level — so a call site that
/// uses them directly ships an unfiltered column. There are two assembly points
/// today (`input.rs`'s open, `command_band.rs`'s refresh) and both go through
/// `Config::resolve_band_verbs` / `layer_band_verbs`. This fails if `src/` grows
/// a third that does not, which nothing else would catch: the column would look
/// entirely correct, and hold four words too many.
#[test]
fn every_verb_table_in_src_is_assembled_through_config() {
    let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    let mut files = 0usize;
    let mut stack = vec![src_dir.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("src/ is readable") {
            let path = entry.expect("a readable entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            // config.rs DEFINES the wrappers, so it is the one file that calls
            // the band's own resolution directly.
            if path.file_name().and_then(|f| f.to_str()) == Some("config.rs") {
                continue;
            }
            files += 1;
            let text = std::fs::read_to_string(&path).expect("a readable file");
            for (n, line) in text.lines().enumerate() {
                // Skip prose: these names are named in several doc comments.
                if line.trim_start().starts_with("//") {
                    continue;
                }
                for call in [".resolve_verbs(", ".resolve_verbs_with(", ".layer_extra_verbs("] {
                    if line.contains(call) {
                        offenders.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
                    }
                }
            }
        }
    }
    assert!(files > 50, "sanity: the walk found the source tree ({files} files)");
    assert!(
        offenders.is_empty(),
        "these assemble a VerbTable without the adult list — use \
         `Config::resolve_band_verbs` / `Config::layer_band_verbs` instead:\n{}",
        offenders.join("\n")
    );
}
