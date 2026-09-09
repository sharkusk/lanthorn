//! The US S.A.G.A. binary-database loader against the reference-format oracle
//! (SQ-1414, SQ-1464).
//!
//! `docs/internals/scott-dialects-spec.md` §12.13 names the oracle and calls it
//! a good one: every title here has a reference-format twin in the §10.1
//! archive — `adv01` through `adv06` for the six S.A.G.A. numbers, `adv13` for
//! *Claymorgue*, and `quest1` for the *Hulk*. Decode the binary database and
//! compare table for table against the twin; nothing from any implementation is
//! needed, and nothing from the specification either once the comparison is
//! running.
//!
//! # What the comparison should be expected to produce
//!
//! §12.13's own list, which [`ORACLE`] pins release by release:
//!
//! * the eleven header numbers match **exactly** on all fifteen databases;
//! * item start locations and room connections match exactly on fourteen of
//!   the fifteen — all but the damaged specimen below;
//! * the dictionary matches, truncated to the release's word length;
//! * room, message and item texts match except where the graphic release
//!   genuinely differs, the Atari *Count* being the widest such gap;
//! * actions match to within a handful of records per title, and *Voodoo
//!   Castle* and *The Count* are **exactly identical on both platforms** —
//!   "a column-major reader that gets either of them wrong is wrong about the
//!   format, not about the release".
//!
//! # The damaged specimen
//!
//! §12.13: the Atari *Mission Impossible* side A carries about fifty corrupt
//! bytes in the middle of its room-description block. Its header and dictionary
//! decode correctly; everything after the sixteenth room string does not, so
//! §12.7's pointer tables do not resolve and §12.14 says to stop. This suite
//! asserts that it is **refused**, with [`scott::LoadError::BadDialectData`] —
//! not that it decodes.
//!
//! # Getting the corpus
//!
//! Both halves are commercial game files, neither is redistributable, and
//! neither is committed. Everything here **skips vacuously with an
//! explanation** when either half is absent — a silent skip reads exactly like
//! a pass, so the skip says why.
//!
//! ```text
//! <fixtures>/atari/db/{adventureland,pirate,mission,voodoo,count,odyssey,claymorgue}.bin
//!     MountedDisk::mount(side A .atr).read_named(blorb::atr::IMAGE_ENTRY)
//! <fixtures>/apple/db/{same seven}.bin
//!     MountedDisk::mount(boot-side .dsk).read_named("A1.DAT" | … | "DATABASE")
//! <fixtures>/c64/db/hulk.bin
//!     MountedDisk::mount(QUESTPR1.D64).read_named("SHULK.DB")
//! <fixtures>/../{adv01..adv06,adv13,quest1}.dat   the reference-format twins
//! ```
//!
//! where `<fixtures>` is `$SCOTT_DIALECT_FIXTURES` or `stories/scott-dialects`,
//! and each `db/README.txt` names the source image, the exact `MountedDisk`
//! call and the sha256 of what it produced.

use std::path::PathBuf;

use scott::{
    detect_saga_us, looks_like_saga_us, parse_saga_us, Database, Dialect, LoadError, SagaPlatform,
    SagaUs,
};

/// One specimen: platform, the `db/` stem, the reference-format twin's stem,
/// and §12.12's tabulated facts, every one of which that section says is
/// "recovered from the bytes" — so this table is an identification checksum
/// rather than an input.
///
/// The fifteenth fact, the database's memory base, is not pinned here because
/// the loader consumes it: §12.7's three pointer tables are resolved against
/// it, entry for entry, and a wrong base cannot survive that.
struct Specimen {
    platform: SagaPlatform,
    stem: &'static str,
    twin: &'static str,
    version: u16,
    adventure: u16,
    /// word length, words, actions, items, messages, rooms, max carried,
    /// start room, treasures, lamp turns, treasure room — §4.5's US order.
    header: [i64; 11],
    /// The array offset §12.5's `ANY` scan lands on.
    dictionary_at: usize,
    /// Per-table disagreement with the twin, as measured: verbs, nouns, room
    /// texts, exits, messages, item texts, item start locations, actions.
    /// Pinned rather than floored — a floor either passes everything or fails
    /// an honest release.
    diffs: Diffs,
}

#[derive(Debug, PartialEq, Eq)]
struct Diffs {
    verbs: usize,
    nouns: usize,
    rooms: usize,
    exits: usize,
    messages: usize,
    items: usize,
    locations: usize,
    actions: usize,
}

const NONE: Diffs =
    Diffs { verbs: 0, nouns: 0, rooms: 0, exits: 0, messages: 0, items: 0, locations: 0, actions: 0 };

/// §12.12's two tables plus the Commodore 64 row, and §12.13's measured
/// agreement per title.
///
/// The Atari *Mission Impossible* is absent on purpose: it is the damaged
/// specimen (§12.13) and is asserted separately, as a refusal.
const ORACLE: &[Specimen] = &[
    Specimen {
        platform: SagaPlatform::Atari8Bit,
        stem: "adventureland",
        twin: "adv01",
        version: 416,
        adventure: 1,
        header: [3, 69, 169, 65, 75, 33, 6, 11, 13, 125, 3],
        dictionary_at: 0x108,
        diffs: Diffs { messages: 1, actions: 4, ..NONE },
    },
    Specimen {
        platform: SagaPlatform::Atari8Bit,
        stem: "pirate",
        twin: "adv02",
        version: 408,
        adventure: 2,
        header: [3, 79, 177, 66, 88, 26, 6, 1, 2, 150, 1],
        dictionary_at: 0x108,
        diffs: Diffs { items: 1, actions: 4, ..NONE },
    },
    Specimen {
        platform: SagaPlatform::Atari8Bit,
        stem: "voodoo",
        twin: "adv04",
        version: 119,
        adventure: 4,
        header: [3, 89, 189, 65, 99, 25, 9, 1, 0, 15000, 0],
        dictionary_at: 0x108,
        diffs: NONE,
    },
    Specimen {
        platform: SagaPlatform::Atari8Bit,
        stem: "count",
        twin: "adv05",
        version: 115,
        adventure: 5,
        header: [3, 79, 219, 72, 88, 22, 7, 1, 0, 175, 0],
        dictionary_at: 0x108,
        // §12.13: "the Atari *Count* is the widest such gap, 3 rooms, 8
        // messages and 7 items away from `adv05.dat`, where the Apple II
        // *Count* is identical to it".
        diffs: Diffs { rooms: 3, messages: 8, items: 7, ..NONE },
    },
    Specimen {
        platform: SagaPlatform::Atari8Bit,
        stem: "odyssey",
        twin: "adv06",
        version: 119,
        adventure: 6,
        header: [4, 79, 223, 55, 94, 35, 6, 1, 5, 10000, 22],
        dictionary_at: 0x108,
        diffs: Diffs { actions: 3, ..NONE },
    },
    Specimen {
        platform: SagaPlatform::Atari8Bit,
        stem: "claymorgue",
        twin: "adv13",
        version: 125,
        adventure: 13,
        header: [5, 109, 267, 75, 79, 32, 10, 1, 13, 3000, 19],
        dictionary_at: 0x108,
        diffs: Diffs { actions: 2, ..NONE },
    },
    Specimen {
        platform: SagaPlatform::AppleII,
        stem: "adventureland",
        twin: "adv01",
        version: 416,
        adventure: 1,
        header: [3, 69, 169, 65, 75, 33, 6, 11, 13, 125, 3],
        dictionary_at: 0x108,
        diffs: Diffs { messages: 1, actions: 4, ..NONE },
    },
    Specimen {
        platform: SagaPlatform::AppleII,
        stem: "pirate",
        twin: "adv02",
        version: 408,
        adventure: 2,
        header: [3, 79, 177, 66, 88, 26, 6, 1, 2, 150, 1],
        dictionary_at: 0x108,
        diffs: Diffs { items: 1, actions: 4, ..NONE },
    },
    Specimen {
        platform: SagaPlatform::AppleII,
        stem: "mission",
        twin: "adv03",
        version: 306,
        adventure: 3,
        header: [3, 64, 161, 53, 81, 23, 7, 2, 0, 10000, 1],
        dictionary_at: 0x108,
        diffs: Diffs { messages: 2, actions: 7, ..NONE },
    },
    Specimen {
        platform: SagaPlatform::AppleII,
        stem: "voodoo",
        twin: "adv04",
        version: 119,
        adventure: 4,
        header: [3, 89, 189, 65, 99, 25, 9, 1, 0, 15000, 0],
        dictionary_at: 0x108,
        diffs: NONE,
    },
    Specimen {
        platform: SagaPlatform::AppleII,
        stem: "count",
        twin: "adv05",
        version: 115,
        adventure: 5,
        header: [3, 79, 219, 72, 88, 22, 7, 1, 0, 175, 0],
        dictionary_at: 0x108,
        diffs: NONE,
    },
    Specimen {
        platform: SagaPlatform::AppleII,
        stem: "odyssey",
        twin: "adv06",
        version: 119,
        adventure: 6,
        header: [4, 79, 223, 55, 94, 35, 6, 1, 5, 10000, 22],
        dictionary_at: 0x108,
        diffs: Diffs { actions: 3, ..NONE },
    },
    Specimen {
        platform: SagaPlatform::AppleII,
        stem: "claymorgue",
        twin: "adv13",
        version: 122,
        adventure: 13,
        header: [5, 109, 267, 75, 79, 32, 10, 1, 13, 3000, 19],
        dictionary_at: 0x108,
        // §12.5/§12.8: version 122 where the Atari release is 125, and the
        // dictionary and action gaps are that release difference. §12.5 says
        // "six verbs and two nouns"; measured here it is NINETEEN verbs and
        // two nouns, of which six are mid-table synonyms the v122 release
        // spells `.` and eleven are trailing entries (`LIGHT` and its
        // synonyms) it does not carry at all. The Atari v125 release matches
        // its twin exactly, which is what makes this a release difference and
        // not a decoding one.
        diffs: Diffs { verbs: 19, nouns: 2, messages: 5, actions: 15, ..NONE },
    },
    Specimen {
        platform: SagaPlatform::Commodore64,
        stem: "hulk",
        twin: "quest1",
        version: 127,
        adventure: 1,
        header: [4, 128, 261, 54, 99, 20, 10, 1, 17, 150, 16],
        dictionary_at: 0x55,
        diffs: Diffs { actions: 10, ..NONE },
    },
];

fn fixtures() -> Option<PathBuf> {
    [
        std::env::var_os("SCOTT_DIALECT_FIXTURES").map(PathBuf::from),
        Some(PathBuf::from("stories/scott-dialects")),
        Some(PathBuf::from("../../stories/scott-dialects")),
    ]
    .into_iter()
    .flatten()
    .find(|p| p.is_dir())
}

fn platform_dir(platform: SagaPlatform) -> &'static str {
    match platform {
        SagaPlatform::Atari8Bit => "atari",
        SagaPlatform::AppleII => "apple",
        _ => "c64",
    }
}

/// The extracted database file, exactly as the container handed it over.
fn database_file(platform: SagaPlatform, stem: &str) -> Option<Vec<u8>> {
    let path = fixtures()?.join(platform_dir(platform)).join("db").join(format!("{stem}.bin"));
    std::fs::read(path).ok()
}

/// The reference-format twin, parsed through this crate's own text reader.
fn twin(stem: &str) -> Option<Database> {
    let root = fixtures()?;
    let bytes = std::fs::read(root.join("..").join(format!("{stem}.dat"))).ok()?;
    Database::parse(&bytes).ok()
}

/// A vacuous skip reads exactly like a pass, so say why.
fn skipped(what: &str) -> bool {
    eprintln!(
        "SKIP: {what} not found. Extract each platform's database with \
         blorb::medium::MountedDisk (see this suite's module doc and each \
         stories/scott-dialects/*/db/README.txt) and put the reference-format \
         .dat twins beside stories/, or point SCOTT_DIALECT_FIXTURES at a \
         directory holding both."
    );
    true
}

/// Every specimen that is present, with its bytes.
fn corpus() -> Vec<(&'static Specimen, Vec<u8>)> {
    ORACLE.iter().filter_map(|s| Some((s, database_file(s.platform, s.stem)?))).collect()
}

/// §12.5: compare a dictionary entry truncated to the release's word length,
/// keeping the `*` synonym marker, which is what the twin's untruncated words
/// have to be cut down to.
fn cut(word: &str, width: usize) -> String {
    match word.strip_prefix('*') {
        Some(rest) => format!("*{}", rest.chars().take(width).collect::<String>()),
        None => word.chars().take(width).collect(),
    }
}

/// §12.6's zero-length placeholder is the reference format's own `.`, and a
/// `.dat` conversion writes it as an empty string — the same thing spelled two
/// ways, so it is not a difference.
fn text(s: &str) -> &str {
    if s.is_empty() {
        "."
    } else {
        s
    }
}

fn measure(db: &Database, twin: &Database) -> Diffs {
    let width = db.word_length;
    let count = |a: &[String], b: &[String]| {
        a.iter().zip(b).filter(|(x, y)| cut(x, width) != cut(y, width)).count()
    };
    Diffs {
        verbs: count(&db.verbs, &twin.verbs),
        nouns: count(&db.nouns, &twin.nouns),
        rooms: db
            .rooms
            .iter()
            .zip(&twin.rooms)
            .filter(|(a, b)| text(&a.desc) != text(&b.desc) || a.literal != b.literal)
            .count(),
        exits: db.rooms.iter().zip(&twin.rooms).filter(|(a, b)| a.exits != b.exits).count(),
        messages: db
            .messages
            .iter()
            .zip(&twin.messages)
            .filter(|(a, b)| text(a) != text(b))
            .count(),
        items: db
            .items
            .iter()
            .zip(&twin.items)
            .filter(|(a, b)| {
                text(&a.text) != text(&b.text)
                    || a.treasure != b.treasure
                    || a.auto_noun != b.auto_noun
            })
            .count(),
        locations: db
            .items
            .iter()
            .zip(&twin.items)
            .filter(|(a, b)| a.start_loc != b.start_loc)
            .count(),
        actions: db.actions.iter().zip(&twin.actions).filter(|(a, b)| a != b).count(),
    }
}

// ── Identification and layout (§12.2, §12.4, §12.5, §12.7, §12.12) ────────────

#[test]
fn every_specimen_is_detected_at_its_own_platform_and_no_other() {
    let files = corpus();
    if files.is_empty() {
        assert!(skipped("the extracted S.A.G.A. databases"));
        return;
    }
    for (spec, bytes) in &files {
        let name = format!("{} {}", platform_dir(spec.platform), spec.stem);
        assert_eq!(detect_saga_us(bytes), Some(spec.platform), "{name}: §12.2/§12.3 detection");
        assert!(looks_like_saga_us(bytes, spec.platform), "{name}");
        for other in SagaPlatform::ALL {
            if other != spec.platform {
                assert!(!looks_like_saga_us(bytes, other), "{name}: false positive as {other:?}");
            }
        }
        // §12.14: this is a Scott Adams file a host should recognise, and it
        // must reach `Database::parse`'s own entry point.
        assert!(scott::looks_like_scott_bytes(bytes), "{name}: the byte sniff answers for it");
        assert_eq!(scott::detect_dialect(bytes), Some(Dialect::SagaUsDatabase), "{name}");
    }
    eprintln!("{} extracted databases detected", files.len());
}

#[test]
fn every_specimen_reproduces_the_facts_section_12_12_tabulates() {
    let files = corpus();
    if files.is_empty() {
        assert!(skipped("the extracted S.A.G.A. databases"));
        return;
    }
    for (spec, bytes) in &files {
        let name = format!("{} {}", platform_dir(spec.platform), spec.stem);
        let db = parse_saga_us(bytes, spec.platform).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let release = db.saga_us.expect("a S.A.G.A. database carries its release");
        assert_eq!(
            release,
            SagaUs { version: spec.version, adventure: spec.adventure, platform: spec.platform },
            "{name}: §12.2's version/adventure pair"
        );
        // §12.12's eleven header numbers, in §4.5's US field order.
        let measured: [i64; 11] = [
            db.word_length as i64,
            (db.verbs.len() - 1) as i64,
            (db.actions.len() - 1) as i64,
            (db.items.len() - 1) as i64,
            (db.messages.len() - 1) as i64,
            (db.rooms.len() - 1) as i64,
            db.max_carry as i64,
            db.start_room as i64,
            db.num_treasures as i64,
            db.light_time as i64,
            db.treasure_room as i64,
        ];
        assert_eq!(measured, spec.header, "{name}: §12.12's header row");
        assert_eq!(db.nouns.len(), db.verbs.len(), "{name}: (words + 1) cells in each block");
        assert_eq!(db.adventure_number, i64::from(spec.adventure) as i32, "{name}");
        // §12.11: these are Adventure International releases and force neither
        // lamp option, unlike the Mysterious ones.
        assert!(!db.mysterious, "{name}: §12.11 forces no lamp option");
        assert!(db.ti99.is_none(), "{name}");
        // §12.5: the dictionary begins on the `A` of the first noun cell.
        let array = &bytes[spec.platform.array_offset()..];
        assert_eq!(&array[spec.dictionary_at..spec.dictionary_at + 3], b"ANY", "{name}: §12.5");
        assert_eq!(db.nouns[0], "ANY", "{name}: `ANY` is noun cell 0");
        eprintln!("{name}: v{} adv {}", spec.version, spec.adventure);
    }
}

#[test]
fn the_item_back_reference_byte_is_the_items_own_index() {
    // §12.6: "An item whose description carries an auto-get word is followed by
    // one extra byte, outside the length prefix, and that byte is the item's
    // own index." The loader consumes it without checking (see
    // `read_item_strings`); this is where the measurement is pinned, over every
    // specimen and every item.
    let files = corpus();
    if files.is_empty() {
        assert!(skipped("the extracted S.A.G.A. databases"));
        return;
    }
    for (spec, bytes) in &files {
        let name = format!("{} {}", platform_dir(spec.platform), spec.stem);
        let db = parse_saga_us(bytes, spec.platform).expect("parses");
        let array = &bytes[spec.platform.array_offset()..];
        // Reach the item block WITHOUT re-implementing the loader: anchor on a
        // long, uniquely-occurring message (§12.6's strings are stored plain,
        // one length byte and that many bytes of text) and walk the rest of
        // the message block from there. The item block begins where it ends.
        let anchor = db
            .messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.len() >= 12 && m.is_ascii())
            .find_map(|(index, message)| {
                let mut needle = vec![message.len() as u8];
                needle.extend_from_slice(message.as_bytes());
                let hits: Vec<usize> =
                    array.windows(needle.len()).enumerate().filter(|(_, w)| *w == needle).map(|(i, _)| i).collect();
                (hits.len() == 1).then(|| (index, hits[0]))
            });
        let Some((from, mut at)) = anchor else {
            panic!("{name}: no message long and unique enough to anchor on");
        };
        for message in &db.messages[from..] {
            let len = usize::from(array[at]);
            let stored = if len == 0 { "." } else { std::str::from_utf8(&array[at + 1..at + 1 + len]).unwrap_or_default() };
            assert_eq!(stored, message, "{name}: the message walk lost alignment");
            at += 1 + len;
        }
        let mut carried = 0usize;
        for index in 0..db.items.len() {
            let len = usize::from(array[at]);
            at += 1;
            let raw = std::str::from_utf8(&array[at..at + len]).unwrap_or_default();
            at += len;
            if raw.contains('/') {
                assert_eq!(
                    usize::from(array[at]),
                    index,
                    "{name}: item {index} ({raw:?}) back-reference"
                );
                at += 1;
                carried += 1;
            }
        }
        // §12.7: the item block is followed by one separator byte, 0x00 on
        // every specimen — which is also the proof this walk stayed aligned.
        assert_eq!(array[at], 0, "{name}: §12.7's separator byte");
        eprintln!("{name}: {carried} of {} items carry the byte", db.items.len());
    }
}

// ── The oracle (§12.13) ───────────────────────────────────────────────────────

#[test]
fn every_specimen_agrees_with_its_reference_format_twin_as_measured() {
    let files = corpus();
    if files.is_empty() || twin("adv01").is_none() {
        assert!(skipped("the extracted S.A.G.A. databases and their .dat twins"));
        return;
    }
    let mut checked = 0;
    let mut wrong: Vec<String> = Vec::new();
    for (spec, bytes) in &files {
        let name = format!("{} {}", platform_dir(spec.platform), spec.stem);
        let Some(reference) = twin(spec.twin) else {
            eprintln!("SKIP {name}: no {}.dat", spec.twin);
            continue;
        };
        let db = parse_saga_us(bytes, spec.platform).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        // §12.13: "the eleven header numbers match exactly on all fifteen".
        assert_eq!(db.word_length, reference.word_length, "{name}: word length");
        assert_eq!(db.verbs.len(), reference.verbs.len(), "{name}: word count");
        assert_eq!(db.actions.len(), reference.actions.len(), "{name}: action count");
        assert_eq!(db.items.len(), reference.items.len(), "{name}: item count");
        assert_eq!(db.messages.len(), reference.messages.len(), "{name}: message count");
        assert_eq!(db.rooms.len(), reference.rooms.len(), "{name}: room count");
        assert_eq!(db.max_carry, reference.max_carry, "{name}: max carried");
        assert_eq!(db.start_room, reference.start_room, "{name}: start room");
        assert_eq!(db.num_treasures, reference.num_treasures, "{name}: treasures");
        assert_eq!(db.light_time, reference.light_time, "{name}: lamp turns");
        assert_eq!(db.treasure_room, reference.treasure_room, "{name}: treasure room");

        let diffs = measure(&db, &reference);
        eprintln!("{name} vs {}.dat: {diffs:?}", spec.twin);
        if diffs != spec.diffs {
            wrong.push(format!("{name}: measured {diffs:?}, pinned {:?}", spec.diffs));
        }
        checked += 1;
    }
    assert!(checked > 0, "no specimen had a twin to compare against");
    // Every title at once, so one release drifting does not hide the rest.
    assert!(wrong.is_empty(), "agreement changed:\n{}", wrong.join("\n"));
    eprintln!("{checked} specimens compared against their twins");
}

#[test]
fn voodoo_castle_and_the_count_are_exact_on_every_table_on_both_platforms() {
    // §12.13 names these two first: "a column-major reader that gets either of
    // them wrong is wrong about the format, not about the release." Voodoo
    // Castle's 190 records and The Count's 220 are identical to their
    // conversions on both platforms — with the one exception §12.13 itself
    // records, the Atari Count's release text.
    let mut checked = 0;
    for spec in ORACLE.iter().filter(|s| ["voodoo", "count"].contains(&s.stem)) {
        let (Some(bytes), Some(reference)) =
            (database_file(spec.platform, spec.stem), twin(spec.twin))
        else {
            continue;
        };
        let name = format!("{} {}", platform_dir(spec.platform), spec.stem);
        let db = parse_saga_us(&bytes, spec.platform).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(db.actions, reference.actions, "{name}: every action record, §12.8");
        assert_eq!(
            db.rooms.iter().map(|r| r.exits).collect::<Vec<_>>(),
            reference.rooms.iter().map(|r| r.exits).collect::<Vec<_>>(),
            "{name}: every connection, §12.9"
        );
        assert_eq!(
            db.items.iter().map(|i| i.start_loc).collect::<Vec<_>>(),
            reference.items.iter().map(|i| i.start_loc).collect::<Vec<_>>(),
            "{name}: every item start location, §12.7"
        );
        eprintln!("{name}: {} action records identical", db.actions.len());
        checked += 1;
    }
    if checked == 0 {
        assert!(skipped("Voodoo Castle and The Count"));
    }
}

#[test]
fn the_scrambled_apple_titles_decode_their_databases_unchanged() {
    // §12.3: "*Voodoo Castle*, *The Count* and *Claymorgue* are the three Apple
    // II titles whose `M2` file carries `COPYRIGHT 1983 NORMAN L. SAILER` …
    // and all three decode their **databases** by the rules of this section
    // unchanged, with no transform of any kind." Scrambling is a PICTURE
    // matter, so all three must load exactly like the others.
    let mut checked = 0;
    for stem in ["voodoo", "count", "claymorgue"] {
        let Some(bytes) = database_file(SagaPlatform::AppleII, stem) else { continue };
        let db = parse_saga_us(&bytes, SagaPlatform::AppleII)
            .unwrap_or_else(|e| panic!("apple {stem}: {e:?}"));
        assert!(db.rooms.len() > 10 && db.actions.len() > 100, "apple {stem}");
        assert_eq!(db.nouns[0], "ANY", "apple {stem}");
        checked += 1;
    }
    if checked == 0 {
        assert!(skipped("the three scrambled Apple II titles"));
    }
}

// ── Refusals (§12.13, §12.14) ─────────────────────────────────────────────────

#[test]
fn the_damaged_atari_mission_impossible_is_refused_rather_than_decoded() {
    // §12.13: about fifty corrupt bytes from file offset 0x912, and everything
    // after the sixteenth room string is wrong. §12.14: "if [the pointer
    // tables] do not resolve onto the string starts already read, stop."
    let Some(bytes) = database_file(SagaPlatform::Atari8Bit, "mission") else {
        assert!(skipped("the Atari Mission Impossible side A"));
        return;
    };
    // It IS recognised as this format — its header and dictionary are fine —
    // so the report a host gets must be "damaged", not "unknown".
    assert_eq!(detect_saga_us(&bytes), Some(SagaPlatform::Atari8Bit));
    match parse_saga_us(&bytes, SagaPlatform::Atari8Bit) {
        Err(LoadError::BadDialectData(Dialect::SagaUsDatabase, why)) => {
            eprintln!("atari mission: refused — {why}");
        }
        other => panic!("expected a damaged-database refusal, got {other:?}"),
    }
}

#[test]
fn questprobe_3_fantastic_four_stays_refused() {
    // §12.14: "Questprobe 3, *Fantastic Four*, remains unidentified. Neither
    // its Commodore 64 release (`QUESTPR3.D64`) nor its MS-DOS one
    // (`SPL53P.DAT`) is this format … Refuse it by name."
    let Some(root) = fixtures() else {
        assert!(skipped("the dialect fixtures"));
        return;
    };
    let mut checked = 0;
    for name in ["c64/QUESTPR3.D64", "c64/QUESTPR1.D64", "c64/MYSTADV1.D64", "c64/MYSTADV2.D64"] {
        let Ok(bytes) = std::fs::read(root.join(name)) else { continue };
        // A raw disk image is a container, never a database: the host must
        // open it first. None of the four may be mistaken for one.
        assert_eq!(detect_saga_us(&bytes), None, "{name} is a container, not an array");
        checked += 1;
    }
    if checked == 0 {
        assert!(skipped("the Commodore 64 disk images"));
    }
}

#[test]
fn no_other_dialects_specimen_is_mistaken_for_this_format() {
    // The detector is structural rather than signature-based (§12.2), so the
    // thing worth measuring is that it stays silent on every other dialect in
    // the corpus — and on the reference text format, which `Database::parse`
    // tries first anyway.
    let Some(root) = fixtures() else {
        assert!(skipped("the dialect fixtures"));
        return;
    };
    let mut checked = 0;
    for entry in std::fs::read_dir(root.join("ti99")).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "fiad") {
            let bytes = std::fs::read(&path).expect("a listed file reads");
            assert_eq!(detect_saga_us(&bytes), None, "{}", path.display());
            checked += 1;
        }
    }
    for stem in ["adv01", "adv05", "quest1"] {
        let Ok(bytes) = std::fs::read(root.join("..").join(format!("{stem}.dat"))) else {
            continue;
        };
        assert_eq!(detect_saga_us(&bytes), None, "{stem}.dat is the TEXT format");
        checked += 1;
    }
    if checked == 0 {
        assert!(skipped("the other dialects' specimens"));
    }
    eprintln!("{checked} non-S.A.G.A. files correctly not detected");
}

// ── The one entry point (Appendix A) ──────────────────────────────────────────

#[test]
fn database_parse_reads_a_saga_database_from_its_container_bytes() {
    // Appendix A: "a dialect loader belongs behind the same entry point as the
    // text parser and must produce the same model, not a parallel one."
    let mut checked = 0;
    for spec in ORACLE {
        let Some(bytes) = database_file(spec.platform, spec.stem) else { continue };
        let name = format!("{} {}", platform_dir(spec.platform), spec.stem);
        let through_parse = Database::parse(&bytes).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let direct = parse_saga_us(&bytes, spec.platform).expect("parses");
        assert_eq!(through_parse, direct, "{name}: one entry point, one model");
        checked += 1;
    }
    if checked == 0 {
        assert!(skipped("the extracted S.A.G.A. databases"));
    }
    eprintln!("{checked} databases reached through Database::parse");
}

#[test]
fn a_specimen_plays_a_few_turns_through_the_vm() {
    // The model has to be one `Vm` can actually run, not merely one that
    // compares well: boot a real release and take a couple of turns.
    let Some(bytes) = database_file(SagaPlatform::Commodore64, "hulk")
        .or_else(|| database_file(SagaPlatform::AppleII, "adventureland"))
    else {
        assert!(skipped("a S.A.G.A. database to play"));
        return;
    };
    let db = Database::parse(&bytes).expect("parses");
    let start = db.start_room;
    let mut vm = scott::Vm::new(db);
    assert_eq!(vm.step(), scott::StepResult::NeedLine);
    let opening = vm.take_output();
    assert!(!opening.is_empty(), "the opening room description is printed");
    assert_eq!(vm.current_room(), start);
    for line in ["look", "inventory", "north"] {
        vm.supply_line(line);
        assert_eq!(vm.step(), scott::StepResult::NeedLine, "after {line:?}");
    }
    eprintln!("opening frame:\n{opening}");
}
