//! What an object can be CALLED, read out of the story — SQ-1118.
//!
//! `objects::short_name` answers what a game PRINTS for a thing.
//! [`zvm::objects::ParseNames`] answers what a player may TYPE for it, and the
//! two sets barely overlap: Zork I prints "brass lantern" and accepts `lamp`,
//! `lanter` and `light`, none of which is in the printed name.
//!
//! **The oracle for every word below is the game's own parser, not this
//! reader.** Each was confirmed by driving `zvm-cli` through the real story and
//! watching the command land on the right object. From Zork I r88, in the
//! Living Room after `open mailbox` at West of House:
//!
//! ```text
//!   take advertisement → Taken.      drop booklet   → Dropped.
//!   take mail          → Taken.      take glamdring → Taken.
//!   drop orcrist       → Dropped.    take blade     → Taken.
//!   take light         → Taken.      drop lantern   → Dropped.
//!   take lamp          → Taken.
//!   inventory          → A brass lantern / A sword / A leaflet
//! ```
//!
//! The cases against `crates/zvm/tests/fixtures/` are the ones CI can see —
//! most of it committed, `minizork.z3` fetched instead (SQ-1453) and reached
//! the same way through `zvm::fixtures::load`, so CI still always has it;
//! `stories/` is gitignored commercial media and those skip vacuously.

use zvm::memory::Memory;
use zvm::objects::{self, Adjectives, ParseNames};

fn fixture(name: &str) -> Memory {
    Memory::new(zvm::fixtures::load(name).expect("fixture CI always has")).unwrap()
}

/// A gitignored commercial story, or `None` so the case can skip.
fn story(name: &str) -> Option<Memory> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(name);
    if !path.exists() {
        eprintln!("SKIP: {} absent", path.display());
        return None;
    }
    Memory::new(std::fs::read(&path).ok()?).ok()
}

fn words_of(pn: &ParseNames, mem: &Memory, obj: u16) -> Vec<String> {
    pn.of(mem, obj).unwrap_or_else(|| panic!("object {obj} has no parse names")).words
}

// ── Committed fixtures: what CI sees ─────────────────────────────────────────

#[test]
fn an_infocom_story_keeps_its_words_in_a_property_of_its_own_choosing() {
    let mem = fixture("minizork.z3");
    let pn = ParseNames::detect(&mem).expect("the Zork I sampler has parse names");

    // NOT property 1. ZIL numbers `SYNONYM` per game and this one landed on 17.
    assert_eq!(pn.property(), 17);

    let lantern = pn.of(&mem, 102).unwrap();
    assert_eq!(lantern.printed_name, "brass lantern");
    assert_eq!(lantern.words, ["lamp", "lanter", "light"]);
    assert_eq!(lantern.property, Some(17));
    // v1-3 keys hold six Z-characters (ZMSD §13.3), so the player's whole word
    // still matches the story's truncation of it.
    assert_eq!(lantern.truncated_at, Some(6));
    assert!(lantern.refers_to("lantern") && lantern.refers_to("LAMP"));
    assert!(!lantern.refers_to("sword"));

    assert_eq!(words_of(&pn, &mem, 89), ["leafle", "mail"]);
    assert_eq!(words_of(&pn, &mem, 164), ["sword", "blade"]);
    assert_eq!(words_of(&pn, &mem, 167), ["mailbo", "box"]);
    assert_eq!(pn.of(&mem, 167).unwrap().printed_name, "small mailbox");

    // The printed name is not the word list, which is the whole point: `lamp`
    // and `light` appear nowhere in "brass lantern", and `box` appears nowhere
    // in "small mailbox".
    assert!(lantern.words.iter().filter(|w| !lantern.printed_name.contains(w.as_str())).count() >= 2);
    let mailbox = pn.of(&mem, 167).unwrap();
    assert!(!mailbox.printed_name.contains("box "));
    assert!(mailbox.refers_to("box"));

    // And the reader can be asked the other way round.
    let found = pn.find(&mem, "glamdring");
    assert!(found.is_none(), "the sampler's sword has no elvish names: {found:?}");
    assert_eq!(pn.find(&mem, "mailbox").unwrap().id, 167);
}

#[test]
fn an_inform_story_uses_property_one_and_says_so() {
    let mem = fixture("praxix.z5");
    let pn = ParseNames::detect(&mem).expect("praxix has parse names");
    assert_eq!(pn.property(), objects::INFORM_NAME_PROPERTY);

    let look = pn.of(&mem, 6).unwrap();
    assert_eq!(look.words, ["look", "l", "help", "?"]);
    // v4+ keys hold nine Z-characters (§13.4).
    assert_eq!(look.truncated_at, Some(9));
    assert_eq!(pn.of(&mem, 10).unwrap().words, ["comarith", "comparith"]);
}

#[test]
fn a_story_with_no_object_words_is_refused_rather_than_guessed_at() {
    // czech is a VM conformance suite: ten objects, no vocabulary, no game.
    let mem = fixture("czech.z5");
    assert!(ParseNames::detect(&mem).is_none());
}

// ── Falsification: it must refuse, not invent ────────────────────────────────

#[test]
fn asking_the_wrong_property_yields_nothing_instead_of_plausible_words() {
    let mem = fixture("minizork.z3");
    // Property 1 is Inform's answer and is wrong here; the sampler's property 1
    // holds something else entirely on the objects that have it at all.
    let wrong = ParseNames::with_property(&mem, objects::INFORM_NAME_PROPERTY);
    let wrong = wrong.expect("the reader still builds; it is the objects that refuse");
    assert_eq!(wrong.property(), 1);
    assert_eq!(
        wrong.all(&mem).len(),
        0,
        "reading the wrong property must produce no words at all, not fewer words"
    );
    assert!(wrong.of(&mem, 102).is_none());
}

#[test]
fn one_corrupted_entry_refuses_the_whole_object() {
    let mut mem = fixture("minizork.z3");
    let pn = ParseNames::detect(&mem).unwrap();
    assert_eq!(words_of(&pn, &mem, 102).len(), 3);

    // Point the lantern's second word at an address that is not a dictionary
    // entry. A reader that decoded whatever is there would return three
    // plausible-looking words, one of them nonsense.
    let data = objects::get_prop_addr(&mem, 102, pn.property()) as u32;
    mem.write_word(data + 2, 0x0123);
    assert!(
        pn.of(&mem, 102).is_none(),
        "one entry that is not a dictionary word must refuse the object"
    );
    // Its neighbours are untouched, so the refusal is the object's, not the
    // reader's giving up.
    assert_eq!(words_of(&pn, &mem, 167), ["mailbo", "box"]);
}

#[test]
fn an_object_without_the_property_answers_none() {
    let mem = fixture("minizork.z3");
    let pn = ParseNames::detect(&mem).unwrap();
    // Object 0 is the null object and object 2 is one of the many that carry no
    // synonym list; neither may be answered with a guess.
    assert!(pn.of(&mem, 0).is_none());
    let without: Vec<u16> = (1..=objects::object_count(&mem))
        .filter(|&o| pn.of(&mem, o).is_none())
        .collect();
    assert!(!without.is_empty(), "the sampler has objects with no synonyms");
    assert_eq!(pn.all(&mem).len(), 105);
}

#[test]
fn the_object_count_stops_where_the_property_tables_begin() {
    let mem = fixture("minizork.z3");
    let n = objects::object_count(&mem);
    assert_eq!(n, 179);
    // The last object is real and the one past it is not addressable as an
    // entry, which is what the count means.
    assert!(!objects::short_name(&mem, n).is_empty());
}

// ── Real commercial media (skips vacuously without `stories/`) ───────────────

#[test]
fn zork_one_answers_the_words_its_parser_accepts() {
    let Some(mem) = story("zork1-r88-s840726.z3") else { return };
    let pn = ParseNames::detect(&mem).expect("Zork I has parse names");
    // Zork I's SYNONYM is 18 where the sampler's is 17 — the same game, a
    // different compilation, a different property number.
    assert_eq!(pn.property(), 18);

    let sword = pn.of(&mem, 110).unwrap();
    assert_eq!(sword.printed_name, "sword");
    assert_eq!(sword.words, ["sword", "orcris", "glamdr", "blade"]);
    assert!(sword.refers_to("glamdring") && sword.refers_to("orcrist"));

    assert_eq!(words_of(&pn, &mem, 164), ["lamp", "lanter", "light"]);
    assert_eq!(words_of(&pn, &mem, 161), ["advert", "leafle", "bookle", "mail"]);
    assert_eq!(words_of(&pn, &mem, 160), ["mailbo", "box"]);
    assert_eq!(pn.all(&mem).len(), 136);
}

#[test]
fn a_version_six_story_is_settled_by_containment_where_counts_are_not() {
    // Zork Zero's word arrays lead 432 objects to 306, which no margin
    // separates; its V6 dictionary flags mark 24 of 1624 words a noun, which no
    // part-of-speech filter separates either. The adjectives being a strict
    // subset of the nouns is what settles it.
    let Some(mem) = story("zork0-r393-s890714.z6") else { return };
    let pn = ParseNames::detect(&mem).expect("Zork Zero has parse names");
    assert_eq!(pn.property(), 51);
    let hangar = pn.of(&mem, 5).unwrap();
    assert_eq!(hangar.printed_name, "Dirigible Hangar");
    assert_eq!(hangar.words, ["hangar"]);
    assert_eq!(pn.all(&mem).len(), 432);
}

#[test]
fn a_runner_up_that_is_not_the_adjectives_is_beaten_on_count_instead() {
    // Planetfall's property 14 is a word array on twelve objects sharing just
    // one with property 19's 146, so containment cannot settle it and the
    // margin does.
    let Some(mem) = story("planetfall-r37-s851003.z3") else { return };
    let pn = ParseNames::detect(&mem).expect("Planetfall has parse names");
    assert_eq!(pn.property(), 19);
    assert_eq!(pn.all(&mem).len(), 146);
}

#[test]
fn stories_with_no_vocabulary_of_their_own_are_refused() {
    // Journey is menu-driven and Scopa is a card game: neither has a parser,
    // and neither keeps word arrays on its objects.
    for name in ["journey-r83-s890706.z6", "scopa.z6"] {
        let Some(mem) = story(name) else { continue };
        assert!(ParseNames::detect(&mem).is_none(), "{name} should be refused");
    }
    // `advent.z8` boots and plays, and tokenises with its own word table: its
    // Z-machine dictionary declares zero entries, so there is nothing for a
    // parse-name property to point at.
    if let Some(mem) = story("advent.z8") {
        assert!(ParseNames::detect(&mem).is_none());
    }
}

#[test]
fn every_infocom_story_here_answers_with_a_property_of_its_own() {
    // The numbers move from game to game, which is the reason detection exists.
    let expected = [
        ("zork2-r48-s840904.z3", 17u8),
        ("zork3-r17-s840727.z3", 17),
        ("seastalker-r16-s850603.z3", 14),
        ("hitchhiker-r59-s851108.z3", 31),
        ("plunderedhearts-r26-s870730.z3", 29),
        ("nordandbert-r19-s870722.z4", 63),
        ("amfv-r77-s850814.z4", 52),
        ("shogun-r322-s890706.z6", 45),
    ];
    let mut checked = 0;
    for (name, property) in expected {
        let Some(mem) = story(name) else { continue };
        let pn = ParseNames::detect(&mem).unwrap_or_else(|| panic!("{name} has parse names"));
        assert_eq!(pn.property(), property, "{name}");
        assert!(pn.all(&mem).len() > 100, "{name}");
        checked += 1;
    }
    if checked == 0 {
        eprintln!("SKIP: no Infocom media present");
    }
}

// ── Adjectives: read from V4, refused before it — SQ-1120 ───────────────────
//
// **The oracle here is the parser too.** Driving `zvm-cli` through the real
// stories:
//
// ```text
//   zork1-r88-s840726.z3          take brass lantern      → Taken.
//   zork1-invclues-r52-s871125.z5 take brass lantern      → Taken.
//   trinity-r12-s860926.z4        examine baby prams      → They're probably
//                                                            full of British
//                                                            babies.
// ```
//
// `brass` is a word both Zork I releases accept and neither keeps in its
// SYNONYM property. The v5 release answers it out of a second property; the v3
// release keeps it as a one-byte adjective NUMBER that nothing here can locate,
// and says so rather than reporting an empty list.

#[test]
fn a_story_that_cannot_be_asked_says_so_instead_of_answering_none() {
    // A v1–3 Infocom story: the adjectives exist (its parser takes `brass`)
    // and are stored as numbers this reader cannot find. `Unavailable` is the
    // honest answer and is NOT the same value as "this object has none".
    let mem = fixture("minizork.z3");
    let pn = ParseNames::detect(&mem).unwrap();
    assert_eq!(pn.adjective_property(), None);
    let lantern = pn.of(&mem, 102).unwrap();
    assert_eq!(lantern.adjectives, Adjectives::Unavailable);
    assert!(!lantern.adjectives.is_available());
    assert_eq!(lantern.adjectives.property(), None);
    assert_eq!(lantern.adjectives.words(), [] as [String; 0]);
    // The nouns are untouched by any of it.
    assert_eq!(lantern.words, ["lamp", "lanter", "light"]);

    // Inform is unaffected for a different reason — its adjectives are already
    // IN the name array — and reports the same refusal rather than pretending
    // to a second list.
    let inform = fixture("praxix.z5");
    let ipn = ParseNames::detect(&inform).unwrap();
    assert_eq!(ipn.adjective_property(), None);
    assert!(!ipn.of(&inform, 6).unwrap().adjectives.is_available());
}

#[test]
fn the_same_game_on_two_releases_answers_brass_only_where_it_can() {
    // Zork I twice: r88 is v3 and r52 (Solid Gold) is v5. The parser accepts
    // `take brass lantern` on BOTH — the difference is entirely in what can be
    // read back, and the two answers are told apart by their type rather than
    // by an empty list that means two things.
    let Some(v3) = story("zork1-r88-s840726.z3") else { return };
    let Some(v5) = story("zork1-invclues-r52-s871125.z5") else { return };

    let pn3 = ParseNames::detect(&v3).expect("Zork I r88 has parse names");
    let lantern3 = pn3.of(&v3, 164).unwrap();
    assert_eq!(lantern3.printed_name, "brass lantern");
    assert_eq!(lantern3.words, ["lamp", "lanter", "light"]);
    assert_eq!(lantern3.adjectives, Adjectives::Unavailable);
    assert!(!lantern3.refers_to("brass"), "v3 cannot know it, and must not claim to");

    let pn5 = ParseNames::detect(&v5).expect("Zork I r52 has parse names");
    assert_eq!(pn5.property(), 46);
    assert_eq!(pn5.adjective_property(), Some(44));
    let lantern5 = pn5.of(&v5, 153).unwrap();
    assert_eq!(lantern5.printed_name, "brass lantern");
    assert_eq!(lantern5.words, ["lamp", "lantern", "light"], "nouns stay nouns");
    assert_eq!(
        lantern5.adjectives,
        Adjectives::Read { words: vec!["brass".into()], property: 44 }
    );
    assert!(lantern5.refers_to("brass") && lantern5.refers_to("lamp"));
    assert_eq!(lantern5.describe(), "brass lantern [lamp, lantern, light + adj: brass]");

    // And the adjectives are what tell three lanterns apart, which is what a
    // player needs them for.
    let burned = pn5.of(&v5, 95).unwrap();
    assert_eq!(burned.adjectives.words(), ["rusty", "burned", "dead", "useless"]);
    assert_eq!(pn5.of(&v5, 171).unwrap().adjectives.words(), ["broken"]);
}

#[test]
fn an_object_with_no_adjectives_in_a_story_that_has_them_answers_an_empty_list() {
    // The distinction the whole feature turns on: `Read { words: [] }` is an
    // object with none, in a story that would have said so if it had any.
    let Some(mem) = story("zork0-r393-s890714.z6") else { return };
    let pn = ParseNames::detect(&mem).expect("Zork Zero has parse names");
    assert_eq!(pn.property(), 51);
    assert_eq!(pn.adjective_property(), Some(46));

    // The quest's worked example, straight from the story.
    let hangar = pn.of(&mem, 5).unwrap();
    assert_eq!(hangar.printed_name, "Dirigible Hangar");
    assert_eq!(hangar.words, ["hangar"]);
    assert_eq!(hangar.adjectives.words(), ["dirigible", "large"]);
    assert_eq!(hangar.adjectives.property(), Some(46));

    let all = pn.all(&mem);
    assert_eq!(all.len(), 432);
    let bare: Vec<&zvm::objects::ObjectWords> =
        all.iter().filter(|o| o.adjectives.words().is_empty()).collect();
    assert!(!bare.is_empty(), "Zork Zero has objects with no adjectives");
    assert!(
        bare.iter().all(|o| o.adjectives.is_available()),
        "an object with none must still report that the STORY could be asked"
    );
    assert_eq!(all.iter().filter(|o| !o.adjectives.words().is_empty()).count(), 306);
}

#[test]
fn every_version_four_and_up_infocom_story_here_names_an_adjective_property() {
    // Measured across `stories/`: the runner-up in the same ranking that picks
    // the nouns, on every V4–V6 title, and never on a V1–3 one.
    let v4_up = [
        ("amfv-r77-s850814.z4", 52u8, 51u8),
        ("bureaucracy-r116-s870602.z4", 55, 47),
        ("nordandbert-r19-s870722.z4", 63, 50),
        ("trinity-r12-s860926.z4", 51, 49),
        ("beyondzork-r57-s871221.z5", 49, 48),
        ("borderzone-r9-s871008.z5", 51, 38),
        ("sherlock-r26-s880127.z5", 44, 43),
        ("wishbringer-invclues-r23-s880706.z5", 50, 40),
        ("arthur-r74-s890714.z6", 51, 45),
        ("shogun-r322-s890706.z6", 45, 32),
    ];
    let v1_3 = [
        "zork1-r88-s840726.z3",
        "zork2-r48-s840904.z3",
        "zork3-r17-s840727.z3",
        "seastalker-r16-s850603.z3",
        "hitchhiker-r59-s851108.z3",
        "moonmist-r9-s861022.z3",
        "spellbreaker-r87-s860904.z3",
        "starcross-r17-s821021.z3",
    ];
    let mut checked = 0;
    for (name, nouns, adjectives) in v4_up {
        let Some(mem) = story(name) else { continue };
        let pn = ParseNames::detect(&mem).unwrap_or_else(|| panic!("{name} has parse names"));
        assert_eq!(pn.property(), nouns, "{name} nouns");
        assert_eq!(pn.adjective_property(), Some(adjectives), "{name} adjectives");
        assert!(
            pn.all(&mem).iter().filter(|o| !o.adjectives.words().is_empty()).count() > 100,
            "{name} should answer adjectives for most of its objects"
        );
        checked += 1;
    }
    for name in v1_3 {
        let Some(mem) = story(name) else { continue };
        let pn = ParseNames::detect(&mem).unwrap_or_else(|| panic!("{name} has parse names"));
        // Every one of these has a contained runner-up covering one to four
        // objects — noise, and reading it would answer `win` for Zork I's
        // kitchen window. The version gate is what refuses it.
        assert_eq!(pn.adjective_property(), None, "{name} must not guess");
        checked += 1;
    }
    if checked == 0 {
        eprintln!("SKIP: no Infocom media present");
    }
}

#[test]
fn asking_the_wrong_adjective_property_yields_nothing_instead_of_plausible_words() {
    // Falsification for the adjective half, the way
    // `asking_the_wrong_property_yields_nothing_instead_of_plausible_words`
    // does for the nouns: point it at the NOUN property of a v1–3 story, where
    // the bytes are adjective numbers and not addresses at all.
    let Some(mem) = story("zork1-r88-s840726.z3") else { return };
    let wrong = ParseNames::with_properties(&mem, 18, Some(16)).unwrap();
    let lantern = wrong.of(&mem, 164).unwrap();
    assert_eq!(lantern.words, ["lamp", "lanter", "light"], "the nouns are still right");
    assert_eq!(
        lantern.adjectives.words(),
        [] as [String; 0],
        "property 16 holds one-byte numbers, not dictionary addresses, and must decode to nothing"
    );
    // …and it still reports that the story was ASKED, which is the honest
    // answer for a reader that was told where to look and found nothing.
    assert!(lantern.adjectives.is_available());
}

#[test]
fn a_property_table_that_repeats_a_number_is_walked_once_and_not_forever() {
    // SQ-1143. Sherlock r26 object 308 lists property 43 twice, so
    // `get_next_prop` — which answers with the entry AFTER the one it is given
    // — returns 43 again, and an unguarded walk never terminates. Detection
    // walks every object, so this case simply RETURNING is the assertion; it
    // hung indefinitely before `objects::property_numbers` existed.
    let Some(mem) = story("sherlock-r26-s880127.z5") else { return };
    let chain = objects::property_numbers(&mem, 308);
    assert_eq!(chain, [51, 50, 47, 46, 45, 44, 43], "the walk stops where it stops descending");
    let pn = ParseNames::detect(&mem).expect("Sherlock has parse names");
    assert_eq!(pn.property(), 44);
    assert_eq!(pn.adjective_property(), Some(43));
    assert_eq!(pn.of(&mem, 1).unwrap().printed_name, "Chamber of Horrors");
}
