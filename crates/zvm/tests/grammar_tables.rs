// Grammar-table extraction against real stories — SQ-1040.
//
// Two halves, and they fail differently. The synthetic tables in
// `zvm::grammar`'s own unit tests exercise every token type of every format
// against an oracle we build byte by byte; this file drives the parser over
// stories somebody else compiled, where the assumption we did not know we were
// making is the one that breaks.
//
// The cases that use `crates/zvm/tests/fixtures/` run everywhere, CI included:
// `minizork.z3` is a real Infocom ZIL game with a real syntax table, and every
// number asserted about it below was read out of `infodump -g`'s dump of the
// same file (ztools 7/3, built from <https://github.com/ecliptik/ztools>)
// rather than out of this implementation. The cases that reach `stories/` skip
// vacuously, because that directory is gitignored commercial media — each of
// those carries a guard naming a count or a verb, so a parser that returned an
// empty table could not pass as success.
//
// Cross-checked at SQ-1040 against `infodump -g` over 97 story files in
// `stories/`: 94 rendered identically sentence for sentence, and the three
// Infocom Version 6 games differ only in that infodump prints a bare-verb line
// for every verb, including the ones whose record says $FFFF ("cannot be used
// as a sentence in itself") — its `show_verb_parse_table` reads an
// uninitialised `verb_entry` when deciding, so the check it documents never
// runs. Filtering those lines out, all three match exactly.

use std::path::PathBuf;

use zvm::grammar::{Grammar, GrammarError, GrammarFormat, NounKind, Token};
use zvm::memory::Memory;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Load a gitignored commercial story, or `None` so the case can skip.
fn story(name: &str) -> Option<Memory> {
    let path = stories_dir().join(name);
    if !path.exists() {
        eprintln!("SKIP: {} absent", path.display());
        return None;
    }
    Memory::new(std::fs::read(&path).ok()?).ok()
}

/// Load a fixture this crate can always reach on CI — most are committed;
/// `minizork.z3` is fetched instead (SQ-1453) and `zvm::fixtures::load`
/// falls back to the app crate's fetched copy for that one name. Either way
/// CI always has it, so a missing one is a failure rather than a skip.
fn fixture(name: &str) -> Memory {
    let bytes = zvm::fixtures::load(name).unwrap_or_else(|| panic!("fixture {name} is missing"));
    Memory::new(bytes).expect("fixture is a valid story")
}

// ── minizork.z3 — a fetched Infocom fixture (SQ-1453), so CI sees this ──────

#[test]
fn minizork_grammar_matches_infodump() {
    let g = Grammar::load(&fixture("minizork.z3")).expect("minizork has a grammar table");

    // infodump: "Verb entries = 101", and the later games' variable-length
    // syntax lines rather than the fixed 8-byte form.
    assert_eq!(g.format(), GrammarFormat::InfocomVariable);
    assert_eq!(g.verbs().len(), 101);

    // Non-vacuity: infodump's dump holds 212 syntax lines across those verbs.
    let lines: usize = g.verbs().iter().map(|v| v.lines.len()).sum();
    assert_eq!(lines, 212);

    // infodump: `245. 1 entry, verb = "unlock"` → `"unlock OBJ with OBJ"`.
    let unlock = g.verb_for_word("unlock").expect("minizork knows 'unlock'");
    assert_eq!(unlock.number, 245);
    assert_eq!(unlock.lines.len(), 1);
    assert_eq!(unlock.lines[0].noun_count(), 2);
    assert_eq!(unlock.lines[0].literals(), vec!["with"]);
}

#[test]
fn minizork_answers_the_questions_a_rejection_needs() {
    let g = Grammar::load(&fixture("minizork.z3")).unwrap();

    // "take" is a synonym; infodump prints the verb under its first dictionary
    // spelling, `239. 8 entries, verb = "carry", synonyms = ... take`.
    let take = g.verb_for_word("take").expect("minizork knows 'take'");
    assert_eq!(take.number, 239);
    assert_eq!(take.word(), Some("carry"));
    assert_eq!(take.lines.len(), 8);

    // The distinction the dictionary alone cannot make: this verb takes a
    // second noun after "from", and does not after "with".
    assert!(take.accepts(2, &["from"]));
    assert!(!take.accepts(2, &["with"]));
    // …while "open" does take "with", so the difference is the story's and not
    // an artefact of how we ask.
    assert!(g.verb_for_word("open").unwrap().accepts(2, &["with"]));

    // Parts of speech, which the flat dictionary does not carry.
    assert!(g.is_verb("carry") && g.is_verb("take") && g.is_verb("grab"));
    assert!(!g.is_verb("lamp"));
    assert!(g.roles("take").unwrap().verb);
    // infodump's dictionary dump: `lamp [80 01 00] <noun>`.
    assert!(g.roles("lamp").is_some_and(|r| r.noun && !r.verb));

    // infodump: "Prepositions … Table entries = 18". Distinct spellings are
    // fewer than table entries, which include synonyms of one index.
    for word in ["with", "from", "under", "around", "off"] {
        assert!(g.is_preposition(word), "{word} should be a preposition");
    }
    assert!(!g.is_preposition("carry"));

    // The shape query, over the whole grammar rather than one verb.
    let with_two: Vec<&str> =
        g.verbs_accepting(2, &["with"]).iter().filter_map(|v| v.word()).collect();
    assert!(with_two.contains(&"unlock"), "got {with_two:?}");
}

// ── Committed fixtures with no grammar at all ────────────────────────────────

#[test]
fn parser_less_test_stories_report_absent() {
    // czech and praxix are Z-machine conformance suites, not adventures: they
    // have dictionaries but no verb table. Refusing with `Absent` rather than
    // inventing one is the whole contract.
    for name in ["czech.z5", "praxix.z5"] {
        assert_eq!(Grammar::load(&fixture(name)).err(), Some(GrammarError::Absent), "{name}");
    }
}

// ── Real commercial media (skips vacuously without `stories/`) ───────────────

#[test]
fn zork1_uses_the_fixed_infocom_form() {
    let Some(mem) = story("zork1-r88-s840726.z3") else { return };
    let g = Grammar::load(&mem).expect("Zork I has a grammar table");
    assert_eq!(g.format(), GrammarFormat::InfocomFixed);
    assert_eq!(g.verbs().len(), 134); // infodump: "Verb entries = 134"

    let take = g.verb_for_word("take").expect("Zork I knows 'take'");
    assert!(take.max_nouns() >= 1);
    assert!(g.is_preposition("with"));
}

#[test]
fn beyond_zork_v5_reads_as_a_whole() {
    let Some(mem) = story("beyondzork-r57-s871221.z5") else { return };
    let g = Grammar::load(&mem).expect("Beyond Zork has a grammar table");
    assert_eq!(g.format(), GrammarFormat::InfocomFixed);
    assert_eq!(g.verbs().len(), 233);
    // Non-vacuity: infodump renders 753 syntax lines for this release.
    let lines: usize = g.verbs().iter().map(|v| v.lines.len()).sum();
    assert_eq!(lines, 753);
}

#[test]
fn photopia_reads_as_inform_grammar_version_2() {
    let Some(mem) = story("photopia.z5") else { return };
    let g = Grammar::load(&mem).expect("Photopia has a grammar table");
    assert_eq!(g.format(), GrammarFormat::InformGv2);
    assert_eq!(g.verbs().len(), 101);

    // infodump: `get out / off / up`, `get in / into / on / onto noun`. The
    // alternative list is a GV2-only shape and the reason a slot holds a
    // vector: flattening it would claim the story wants four words in a row.
    let get = g.verb_for_word("get").expect("Photopia knows 'get'");
    let entering = get
        .lines
        .iter()
        .flat_map(|l| l.slots.iter())
        .find(|s| s.accepts_word("in"))
        .expect("'get' has a line beginning with 'in'");
    let words: Vec<&str> = entering.alternatives.iter().filter_map(Token::word).collect();
    assert!(words.contains(&"into") && words.contains(&"onto"), "got {words:?}");
    // `get out / off / up` is a second alternative list on another line.
    assert_eq!(
        get.lines.iter().flat_map(|l| l.slots.iter()).filter(|s| s.alternatives.len() > 1).count(),
        2
    );

    // GV2's elementary tokens are named, which the Infocom formats never are.
    assert!(g
        .verbs()
        .iter()
        .flat_map(|v| v.lines.iter())
        .flat_map(|l| l.slots.iter())
        .any(|s| s.alternatives.contains(&Token::Noun(NounKind::MultiExcept))));
}

#[test]
fn mysterious_adventures_read_as_inform_grammar_version_1() {
    let Some(mem) = story("mysterious01.z6") else { return };
    let g = Grammar::load(&mem).expect("Mysterious Adventures 01 has a grammar table");
    assert_eq!(g.format(), GrammarFormat::InformGv1);
    assert_eq!(g.verbs().len(), 42); // infodump: "Verb entries = 42"
    assert!(g.is_verb("go"));
}

#[test]
fn zork_zero_reads_the_version_6_shape() {
    let Some(mem) = story("zork0-r393-s890714.z6") else { return };
    let g = Grammar::load(&mem).expect("Zork Zero has a grammar table");
    assert_eq!(g.format(), GrammarFormat::InfocomV6);
    assert_eq!(g.verbs().len(), 153); // infodump: "Verb entries = 153"

    // infodump: `verb = "unlock"` → `"unlock OBJ with OBJ"`.
    let unlock = g.verb_for_word("unlock").expect("Zork Zero knows 'unlock'");
    assert!(unlock.accepts(2, &["with"]));
    assert!(!unlock.accepts(2, &["from"]));

    // The V6 object slot carries the attribute the game's own suggestion
    // helper uses, which no other format has.
    assert!(unlock
        .lines
        .iter()
        .flat_map(|l| l.slots.iter())
        .flat_map(|s| s.alternatives.iter())
        .any(|t| matches!(t, Token::InfocomObject { .. })));
}

#[test]
fn journey_has_no_grammar_and_says_so() {
    let Some(mem) = story("journey-r83-s890706.z6") else { return };
    // Journey is driven entirely by menus; infodump agrees — "There are no
    // parse tables". A wrong-but-well-formed table here is exactly the failure
    // this parser must not have.
    assert_eq!(Grammar::load(&mem).err(), Some(GrammarError::Absent));
}

#[test]
fn every_readable_story_either_parses_or_refuses() {
    // A sweep rather than a pin: whatever is on this machine, no story may
    // produce a table the parser cannot vouch for. The alternative to an error
    // is not "an empty grammar" — it is a plausible one, which is worse.
    let Ok(entries) = std::fs::read_dir(stories_dir()) else {
        eprintln!("SKIP: stories/ absent");
        return;
    };
    let mut parsed = 0;
    let mut refused = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_zcode = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e, "z3" | "z4" | "z5" | "z6" | "z7" | "z8"));
        if !is_zcode {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Ok(mem) = Memory::new(bytes) else { continue };
        match Grammar::load(&mem) {
            Ok(g) => {
                parsed += 1;
                // Every verb the table names must be one the dictionary can
                // reach, or the two halves disagree about what a verb is.
                for verb in g.verbs() {
                    for word in &verb.words {
                        assert!(g.is_verb(word), "{}: {word} unreachable", path.display());
                    }
                }
            }
            Err(_) => refused += 1,
        }
    }
    assert!(parsed >= 10, "expected a corpus, parsed {parsed} and refused {refused}");
}
