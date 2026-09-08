//! Infocom Version 6 dictionary roles, checked against the games' own parsers
//! (SQ-1153).
//!
//! `zvm::grammar` used to read the V6 flag byte with Inform's layout — `$01`
//! verb, `$80` noun — and Infocom's V6 games do not keep the bits there. The
//! symptom was not a crash but a plausible wrong answer, different on each
//! title: `$80` selected `are is was were will` on Arthur, six function words
//! on Zork Zero and nothing at all on Shogun, while `crystal`, `torque` and
//! `sword` were not nouns anywhere. Arthur's bytes on their own fit a tidy
//! `$01`/`$02`/`$04`/`$40` reading that the other two falsify, so **one title
//! cannot settle this** and every case below drives all three.
//!
//! **The oracle is the story's own parser, never this reader.** A unit test
//! that asks `dictionary_words` what it thinks and compares that against the
//! same bit mask shares the implementation's assumption and passes however
//! wrong the mask is — which is exactly the defect class that produced the
//! Inform reading. So each case boots the real story headless, reaches its
//! first prompt, and types `x <word>` for every dictionary word, classifying
//! what the game answers:
//!
//!   * **names a thing** — "You can't see any spyglass right here." The parser
//!     took the word as the name of an object.
//!   * **generic object** — "You can't see any elongated *object* right here."
//!     The parser's own wording for a descriptor typed with no noun after it.
//!   * **refused** — "Sorry, but I don't understand."
//!
//! The three titles separate perfectly: every word answered as the name of a
//! thing carries `$02`, every word answered as a bare descriptor carries `$04`,
//! and no word without `$02` is ever taken as the name of a thing.
//!
//! **Specimens.** `arthur-r74-s890714.z6` (release 74, serial 890714),
//! `zork0-r393-s890714.z6` (release 393, serial 890714) and
//! `shogun-r322-s890706.z6` (release 322, serial 890706), each driven to the
//! **first** command prompt — Arthur answering `N` to its restore question,
//! the other two a bare RETURN through their title cards — and every probe run
//! from a Quetzal snapshot of that prompt, so each is turn 1 of a fresh game
//! rather than turn 200 of one long walk. All three are gitignored commercial
//! media; the cases SKIP vacuously without them, and each carries a
//! non-vacuity floor so an empty sweep cannot pass as success.

use std::collections::BTreeMap;
use std::path::PathBuf;

use zvm::cpu::exec::{Machine, StepResult};
use zvm::grammar::{self, GrammarFormat};
use zvm::memory::Memory;

/// A runaway guard for the boot run; Zork Zero's is the longest at a few
/// million opcodes.
const MAX_STEPS: u64 = 40_000_000;

fn story(name: &str) -> Option<Memory> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(name);
    if !path.exists() {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    }
    Memory::new(std::fs::read(&path).ok()?).ok()
}

fn run(m: &mut Machine) -> StepResult {
    let mut n = 0u64;
    loop {
        match m.step() {
            StepResult::Continue => {
                n += 1;
                assert!(n < MAX_STEPS, "runaway: no input request within {MAX_STEPS} steps");
            }
            other => return other,
        }
    }
}

/// What the game answered a one-word probe with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    /// "Sorry, but I don't understand" — the word cannot stand there.
    Refused,
    /// "You can't see any X **object** right here" — a descriptor with no noun.
    GenericObject,
    /// "You can't see any X right here" — the word names a thing.
    NamesAThing,
    /// Anything else: an in-scope description, a buzzword joke, a nudge.
    Other,
}

/// Everything output stream 2 has taken so far. `Machine::new` gives the machine
/// a [`zvm::io::BufferOutput`], whose `transcript` field is where the ZMSD §7.1.1
/// transcript stream lands — the machine itself keeps no copy, since a real host
/// is writing the text to a file as it arrives.
fn transcript(m: &Machine) -> &str {
    &m.buffer_output().expect("Probe boots with the default BufferOutput sink").transcript
}

/// A booted story parked at its first prompt, with a snapshot to return to.
struct Probe {
    machine: Machine,
    snapshot: Vec<u8>,
}

impl Probe {
    /// Boot `mem` the way a host does and drive to the first line prompt,
    /// answering any `read_char` with the next of `keys`.
    ///
    /// `init_caps` is not optional here and its absence is silent: without it
    /// Zork Zero's read buffer carries a maximum length of **zero**, so every
    /// `supply_line` writes nothing, the game answers "[I beg your pardon?]" to
    /// each probe, and a sweep asserting "no noun is refused" passes without
    /// having asked the parser anything at all.
    fn boot(mem: Memory, keys: &[u8]) -> Probe {
        let mut machine = Machine::new(mem);
        machine.init_caps();
        machine.set_screen_dims(24, 80);
        // Stream 2 is the transcript: the only text sink that sees a v6 game's
        // prose, which paints rather than streams (see `Machine::print_text`).
        machine.set_transcript(true);

        let mut i = 0;
        for _ in 0..60 {
            match run(&mut machine) {
                StepResult::NeedLine { .. } => {
                    let snapshot = machine.save_quetzal();
                    return Probe { machine, snapshot };
                }
                StepResult::NeedChar => {
                    let k = *keys.get(i).unwrap_or(keys.last().unwrap_or(&13));
                    i += 1;
                    machine.supply_char(k);
                }
                other => panic!("story stopped at {other:?} before reaching a command prompt"),
            }
        }
        panic!("story never reached a command prompt");
    }

    /// Type `x <word>` at turn 1 of a fresh game and classify the answer.
    fn examine(&mut self, word: &str) -> Verdict {
        self.machine.restore_quetzal(&self.snapshot).expect("snapshot restores");
        let _ = run(&mut self.machine);
        let before = transcript(&self.machine).len();
        self.machine.supply_line(&format!("x {word}"), 13);
        let result = run(&mut self.machine);
        // The games wrap their prose, so a phrase can straddle a newline.
        let reply = transcript(&self.machine)[before..]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        if !matches!(result, StepResult::NeedLine { .. }) {
            return Verdict::Other;
        }
        if reply.contains("don't understand") || reply.contains("don't know the word") {
            Verdict::Refused
        } else if reply.contains("object right here") || reply.contains("object here") {
            Verdict::GenericObject
        } else if reply.contains(&format!("see any {word} right here"))
            || reply.contains(&format!("see {word} right here"))
            || reply.contains(&format!("see any {word} here"))
        {
            Verdict::NamesAThing
        } else {
            Verdict::Other
        }
    }
}

/// How each class of word fared over one story's whole dictionary.
#[derive(Debug, Default)]
struct Tally {
    probed: usize,
    refused: Vec<String>,
    generic: usize,
    names_a_thing: usize,
}

/// The whole sweep for one story: probe every dictionary word that is three or
/// more plain letters and is not a verb, split by the roles **this reader**
/// reports, and let the game grade it.
fn sweep(mem: Memory, keys: &[u8]) -> (Tally, Tally, Tally) {
    assert_eq!(
        grammar::detect_format(&mem),
        GrammarFormat::InfocomV6,
        "this case only means anything for the Infocom V6 flag layout"
    );
    // The reader under test, asked once, before any probing.
    let words: Vec<(String, zvm::grammar::WordRoles)> = grammar::dictionary_words(&mem)
        .into_iter()
        .filter(|w| {
            w.text.len() >= 3 && w.text.chars().all(|c| c.is_ascii_lowercase()) && !w.roles.verb
        })
        .map(|w| (w.text, w.roles))
        .collect();

    let mut probe = Probe::boot(mem, keys);
    let (mut nouns, mut adjectives, mut neither) = (Tally::default(), Tally::default(), Tally::default());
    for (word, roles) in &words {
        let tally = if roles.noun {
            &mut nouns
        } else if roles.adjective {
            &mut adjectives
        } else if roles.raw & !0x07 == 0 {
            // No role bit AND none of this game's own class bits either: a word
            // carrying a direction or preposition bit is a different question,
            // and this quest deliberately does not claim to read those.
            &mut neither
        } else {
            continue;
        };
        tally.probed += 1;
        match probe.examine(word) {
            Verdict::Refused => tally.refused.push(word.clone()),
            Verdict::GenericObject => tally.generic += 1,
            Verdict::NamesAThing => tally.names_a_thing += 1,
            Verdict::Other => {}
        }
    }
    (nouns, adjectives, neither)
}

/// The shared shape of all three cases. `noun_floor`/`adjective_floor` are
/// non-vacuity guards: a sweep that probed nothing, or one whose probes all
/// bounced off an unwritten input buffer, cannot reach them.
fn check(
    file: &str,
    keys: &[u8],
    allowed_noun_refusals: &[&str],
    noun_floor: usize,
    adjective_floor: usize,
) {
    let Some(mem) = story(file) else { return };
    let (nouns, adjectives, neither) = sweep(mem, keys);

    eprintln!(
        "{file}: nouns {}/{} named a thing ({} refused); adjectives {}/{} generic-object ({} refused); \
         role-less {} probed, {} named a thing",
        nouns.names_a_thing,
        nouns.probed,
        nouns.refused.len(),
        adjectives.generic,
        adjectives.probed,
        adjectives.refused.len(),
        neither.probed,
        neither.names_a_thing,
    );

    // 1. The noun bit selects words the parser takes as the name of a thing.
    assert!(
        nouns.names_a_thing >= noun_floor,
        "{file}: only {} of {} words this reader calls nouns were answered as the name of a \
         thing (floor {noun_floor}) — the probes are not reaching the parser",
        nouns.names_a_thing,
        nouns.probed,
    );
    let unexpected: Vec<&String> =
        nouns.refused.iter().filter(|w| !allowed_noun_refusals.contains(&w.as_str())).collect();
    assert!(
        unexpected.is_empty(),
        "{file}: the parser refused {unexpected:?} in a noun position, though this reader calls \
         them nouns",
    );

    // 2. The adjective bit selects words the parser answers as bare descriptors
    //    — "you can't see any <word> OBJECT right here" is the game saying so.
    assert!(
        adjectives.generic >= adjective_floor,
        "{file}: only {} of {} words this reader calls adjectives drew the parser's bare-\
         descriptor answer (floor {adjective_floor})",
        adjectives.generic,
        adjectives.probed,
    );
    assert_eq!(
        adjectives.names_a_thing, 0,
        "{file}: an adjective-only word was answered as the name of a thing",
    );
    assert!(
        adjectives.refused.len() <= 1,
        "{file}: the parser refused {:?} though this reader calls them adjectives",
        adjectives.refused,
    );

    // 3. The strong half, and the one the Inform reading could never satisfy:
    //    a word this reader gives no role to is NEVER taken as the name of a
    //    thing. Measured zero on all three stories.
    assert_eq!(
        neither.names_a_thing, 0,
        "{file}: {} role-less words were answered as the name of a thing",
        neither.names_a_thing,
    );
    assert!(neither.probed >= 50, "{file}: only {} role-less words probed", neither.probed);
}

#[test]
fn arthur_r74_roles_match_its_own_parser() {
    // 'N' declines the "Would you like to restore a saved position?" question
    // Arthur asks before its first prompt; anything else re-asks forever.
    check("arthur-r74-s890714.z6", b"n", &[], 350, 100);
}

#[test]
fn zork_zero_r393_roles_match_its_own_parser() {
    check("zork0-r393-s890714.z6", &[13], &[], 580, 220);
}

#[test]
fn shogun_r322_roles_match_its_own_parser() {
    // Two words carry the noun bit and are still refused after `x`: "the",
    // which the parser wants a noun after, and "gonzale", a name the opening
    // scene will not resolve. Named rather than tolerated by a threshold, so a
    // third one cannot slip in behind them.
    check("shogun-r322-s890706.z6", &[13], &["the", "gonzale"], 380, 120);
}

/// The specific wrong answer this quest was opened on, kept as its own case so
/// a regression names itself rather than moving a count.
#[test]
fn arthur_r74_no_longer_reads_the_inform_layout() {
    let Some(mem) = story("arthur-r74-s890714.z6") else { return };
    let roles: BTreeMap<String, zvm::grammar::WordRoles> =
        grammar::dictionary_words(&mem).into_iter().map(|w| (w.text, w.roles)).collect();

    // What Inform's $80 selected on Arthur, and what its parser thinks of them.
    // `was` is the one of the seven that is not simply refused: Arthur gives it
    // $84, so it is a DESCRIPTOR as well, and the parser answers it with the
    // bare-descriptor wording. Nothing here makes it a noun.
    let mut probe = Probe::boot(mem, b"n");
    for word in ["am", "are", "is", "shall", "was", "were", "will"] {
        let r = roles.get(word).unwrap_or_else(|| panic!("{word} is in Arthur's dictionary"));
        assert!(!r.noun, "{word} carries $80 and is not a noun (raw ${:02x})", r.raw);
        let verdict = probe.examine(word);
        assert_ne!(verdict, Verdict::NamesAThing, "Arthur's parser does not take {word} as a thing");
        if !r.adjective {
            assert_eq!(
                verdict,
                Verdict::Refused,
                "Arthur's parser refuses {word} in a noun position",
            );
        }
    }
    // …and the three the old reading missed.
    for word in ["crystal", "torque", "sword"] {
        let r = roles.get(word).unwrap_or_else(|| panic!("{word} is in Arthur's dictionary"));
        assert!(r.noun, "{word} is a noun (raw ${:02x})", r.raw);
        assert_ne!(
            probe.examine(word),
            Verdict::Refused,
            "Arthur's parser takes {word} in a noun position",
        );
    }
    // "jewelled" is a descriptor of the sword, and the story keeps it in the
    // adjective half of the byte the Inform reading never decoded at all.
    let jewelled = roles.get("jewelled").expect("jewelled is in Arthur's dictionary");
    assert!(jewelled.adjective && !jewelled.noun, "raw ${:02x}", jewelled.raw);
}

/// A second, cheap witness that needs no boot: the words each story files under
/// its own objects, against the bits this reader gives them.
///
/// The noun half is total — every one of the 1,700-odd `SYNONYM` words across
/// the three stories carries `$02`, which no other bit in the byte comes close
/// to. The adjective half is total on Arthur and Shogun and has **four**
/// exceptions on Zork Zero, all of them size words with an empty flag byte:
/// `huge`, `mighty`, `smaller` and `tiny`. Those are not a hole in the DESC
/// bit. Zork Zero rewrites a size word to its canonical spelling before the
/// dictionary is consulted — typing `x big` (also `$00`) is answered "you can't
/// see any **large** object right here" — so the flag byte on the alias is
/// genuinely empty and the object property still lists it. Named here rather
/// than tolerated by a ratio, so a fifth one would fail.
#[test]
fn v6_noun_bit_covers_every_word_the_objects_are_named_by() {
    for (file, adjective_aliases) in [
        ("arthur-r74-s890714.z6", &[][..]),
        ("zork0-r393-s890714.z6", &["huge", "mighty", "smaller", "tiny"][..]),
        ("shogun-r322-s890706.z6", &[][..]),
    ] {
        let Some(mem) = story(file) else { continue };
        let roles: BTreeMap<String, zvm::grammar::WordRoles> =
            grammar::dictionary_words(&mem).into_iter().map(|w| (w.text, w.roles)).collect();
        let names = zvm::objects::ParseNames::detect(&mem).expect("V6 stories keep parse names");
        let (mut nouns, mut adjectives) = (0usize, 0usize);
        for object in names.all(&mem) {
            for word in &object.words {
                assert!(
                    roles.get(word).is_some_and(|r| r.noun),
                    "{file}: {word} names an object and does not carry the noun bit",
                );
                nouns += 1;
            }
            for word in object.adjectives.words() {
                assert!(
                    roles.get(word).is_some_and(|r| r.adjective)
                        || adjective_aliases.contains(&word.as_str()),
                    "{file}: {word} is an object's adjective and does not carry the DESC bit",
                );
                adjectives += 1;
            }
        }
        assert!(nouns > 500 && adjectives > 200, "{file}: {nouns} nouns, {adjectives} adjectives");
    }
}
