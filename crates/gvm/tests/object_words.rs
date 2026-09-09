//! What a Glulx object can be CALLED — SQ-1118.
//!
//! A Glulx image names the object tree's address nowhere, exactly as it names
//! the grammar table's nowhere, so [`gvm::objects::ParseNames`] derives it: a
//! `$70`-tagged linked list whose every link is exact, verified by its objects'
//! `name` arrays landing on real dictionary records. See that module's header
//! for the sources and the derivation.
//!
//! **The oracle is the game's own parser.** Every word pinned below was
//! confirmed by driving `gvm-cli` through the real story. From `advent.blb`,
//! Inside Building, where "There is a shiny brass lamp nearby":
//!
//! ```text
//!   take lamp           → Taken.      drop headlamp → Dropped.
//!   take headlight      → Taken.      drop shiny lantern → Dropped.
//!   take brass          → Taken.      i → a brass lantern
//! ```
//!
//! `stories/` is gitignored commercial media, so those cases skip vacuously;
//! the committed fixture case is a refusal, which is the one worth having in CI
//! — a reader that finds objects in a story that has none would poison every
//! consumer downstream.

use std::path::PathBuf;

use gvm::grammar::GrammarError;
use gvm::memory::Memory;
use gvm::objects::{ObjectError, ParseNames, NAME_PROPERTY};

/// Pull the `GLUL` chunk out of a Blorb, or pass a bare Glulx image through.
/// Hand-rolled so this suite adds no dependency to a zero-dependency crate.
fn glulx_image(bytes: Vec<u8>) -> Option<Vec<u8>> {
    if bytes.starts_with(b"Glul") {
        return Some(bytes);
    }
    if !(bytes.starts_with(b"FORM") && bytes.get(8..12) == Some(b"IFRS")) {
        return None;
    }
    let be32 = |a: usize| -> usize {
        u32::from_be_bytes([bytes[a], bytes[a + 1], bytes[a + 2], bytes[a + 3]]) as usize
    };
    let mut i = 12;
    while i + 8 <= bytes.len() {
        let len = be32(i + 4);
        if &bytes[i..i + 4] == b"GLUL" {
            return bytes.get(i + 8..i + 8 + len).map(<[u8]>::to_vec);
        }
        i += 8 + len + (len & 1);
    }
    None
}

fn story(name: &str) -> Option<Memory> {
    let local = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(name);
    let path = if local.is_file() {
        local
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../app/tests/fixtures/stories").join(name)
    };
    if !path.exists() {
        eprintln!("SKIP: {} absent", path.display());
        return None;
    }
    Memory::new(glulx_image(std::fs::read(&path).ok()?)?).ok()
}

// ── The committed fixture, so CI sees something ─────────────────────────────

#[test]
fn a_story_with_no_dictionary_chain_is_refused_and_says_which_link_failed() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../gvm-cli/tests/fixtures/glulxercise.ulx");
    let bytes = std::fs::read(&path).expect("glulxercise fixture is committed");
    let mem = Memory::new(glulx_image(bytes).expect("bare Glulx image")).unwrap();

    // glulxercise is a VM conformance suite with a dictionary and no parser.
    // Without the dictionary's bounds a `name` array is a list of addresses
    // that cannot be turned back into words, so the refusal is that one and it
    // names the grammar reader's own reason rather than swallowing it.
    assert_eq!(
        ParseNames::detect(&mem).err(),
        Some(ObjectError::NoDictionary(GrammarError::TablesNotFound))
    );
}

// ── Real commercial media (skips vacuously without `stories/`) ──────────────

#[test]
fn adventure_answers_the_words_its_parser_accepts() {
    let Some(mem) = story("advent.blb") else { return };
    let pn = ParseNames::detect(&mem).expect("advent.blb has an object tree");

    // Derived, not looked up: no header field names any of this.
    assert_eq!(pn.head(), 0x1fbcc);
    assert_eq!(pn.attr_bytes(), 7); // Inform's NUM_ATTR_BYTES default
    assert_eq!(pn.len(), 273);

    let lantern = pn.of(&mem, 0x2008c).expect("the brass lantern");
    assert_eq!(lantern.printed_name, "brass lantern");
    assert_eq!(
        lantern.words,
        ["lamp", "headlamp", "headlight", "lantern", "light", "shiny", "brass"]
    );
    assert_eq!(lantern.property, Some(NAME_PROPERTY));
    // Inform keeps ADJECTIVES in the same array as the nouns, which is why
    // `brass` is here and is absent from the Z-machine Infocom reader.
    assert!(lantern.refers_to("brass") && lantern.refers_to("shiny"));
    assert_eq!(pn.find(&mem, "headlamp").map(|o| o.id), Some(0x2008c));

    let snake = pn.of(&mem, 0x206ac).expect("the snake");
    assert_eq!(snake.printed_name, "snake");
    assert!(snake.refers_to("cobra") && snake.refers_to("venomous") && snake.refers_to("asp"));

    assert_eq!(pn.all(&mem).len(), 127);
}

/// Pinned to release 11 (`stories/CounterfeitMonkey-11.gblorb`) on purpose: the exact
/// object-tree head address and count are properties of that specific compile
/// (SQ-1454's disposition table). `stories/`-only, skips vacuously on CI.
#[test]
fn an_inform_seven_story_has_no_printed_names_and_words_are_all_there_is() {
    let Some(mem) = story("CounterfeitMonkey-11.gblorb") else { return };
    let pn = ParseNames::detect(&mem).expect("Counterfeit Monkey has an object tree");
    assert_eq!(pn.head(), 0x53f973);
    assert_eq!(pn.len(), 2494);

    let named = pn.all(&mem);
    assert_eq!(named.len(), 2222);
    // Inform 7 prints objects through a rule rather than a hardware short name,
    // so the vast majority have none at all — which is exactly why a caller
    // needs the word list and not the printed name.
    let unnamed = named.iter().filter(|o| o.printed_name.is_empty()).count();
    assert!(unnamed > named.len() / 2, "{unnamed} of {} unnamed", named.len());
    assert!(named.iter().all(|o| !o.words.is_empty()));
}

#[test]
fn a_non_object_address_is_refused_rather_than_decoded() {
    let Some(mem) = story("advent.blb") else { return };
    let pn = ParseNames::detect(&mem).unwrap();
    // Between two objects, before the tree, and past its end.
    assert!(pn.of(&mem, pn.head() + 1).is_none());
    assert!(pn.of(&mem, pn.head() - 32).is_none());
    assert!(pn.of(&mem, pn.head() + 273 * 32).is_none());
    // The head itself is Inform's `Class` metaclass and has no name array.
    assert!(pn.of(&mem, pn.head()).is_none());
}

#[test]
fn every_glulx_story_here_derives_a_tree_whose_words_are_real_dictionary_records() {
    // The verification that separates an object tree from a run of bytes that
    // walks like one: every entry of every array must land on a `$60` record.
    let expected = [
        ("cragne.gblorb", 0x6471eeu32, 2545usize),
        ("Kerkerkruip.gblorb", 0x205a26, 805),
        ("photo201.blb", 0x35ddb, 219),
    ];
    let mut checked = 0;
    for (name, head, count) in expected {
        let Some(mem) = story(name) else { continue };
        let pn = ParseNames::detect(&mem).unwrap_or_else(|e| panic!("{name} has an object tree: {e:?}"));
        assert_eq!(pn.head(), head, "{name}");
        assert_eq!(pn.len(), count, "{name}");
        let named = pn.all(&mem);
        assert!(named.len() > count / 4, "{name}: only {} named", named.len());
        assert!(named.iter().all(|o| o.words.iter().all(|w| !w.is_empty())), "{name}");
        checked += 1;
    }
    if checked == 0 {
        eprintln!("SKIP: no Glulx media present");
    }
}
