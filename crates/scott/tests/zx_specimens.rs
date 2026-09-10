//! The ZX Spectrum *Mysterious Adventures* loader against the twenty published
//! snapshots and the reference-format oracle (SQ-1478).
//!
//! `docs/internals/scott-dialects-spec.md` §10.1 names the oracle: many Scott
//! Adams games exist both as a platform-native binary and as a plain-text
//! conversion, so decoding the native file and comparing the resulting tables
//! field for field against the text file is a black-box check that needs
//! nothing from the specification and nothing from any GPL implementation.
//!
//! # What this suite is really pinning
//!
//! `crates/scott/src/zx_mysterious.rs` reads these eleven with **no
//! per-release catalogue at all** — the field order and the verb/noun split
//! come out of the bytes — so the interesting assertions here are the ones
//! that could not come out right by accident:
//!
//! * §4.1's dictionary signature occurs **exactly once** in each of the eleven
//!   images, at the address §10.3 pins;
//! * §4.6's plausibility scan, run over all four of §4.5's candidate field
//!   orders at every address in the 48K image, yields **exactly one**
//!   (order, address) pair per title — and it is the **early** order in all
//!   eleven;
//! * the nine other snapshots in the same archive yield **none**, and each is
//!   refused by name rather than mis-read;
//! * the tables the pointer block implies tile the image exactly: every string
//!   block ends on the next table's pointer, and the action table lands on the
//!   room connections.
//!
//! # How strong the oracle is here — five titles of eleven
//!
//! §10.3 warns about exactly this: "Escape from Pulsar 7 matches exactly …
//! The Golden Baton does not … **Check the header numbers before assuming
//! table equality is the right test**". Measured, five of the eleven snapshots
//! carry the same eleven header numbers as their published conversion
//! (*The Time Machine*, *Escape from Pulsar 7*, *Circus*, *The Wizard of
//! Akyrz*, *Ten Little Indians*), and for four of those five every table comes
//! out **identical**; *Ten Little Indians* differs in three action records'
//! fifth condition word and nothing else. The other six are different releases
//! of the same games and this suite pins their overlaps as numbers.
//!
//! # Getting the corpus
//!
//! Both halves are commercial game files, neither is redistributable, and
//! neither is committed. Everything here **skips vacuously with an
//! explanation** when either half is absent — a silent skip reads exactly like
//! a pass, so the skip says why.
//!
//! ```text
//! <fixtures>/spectrum/{m1goldba,…,m11waxwo}.z80  plus the nine controls
//!     https://ifarchive.org/if-archive/games/spectrum/mystsoft.zip
//! <fixtures>/mysterious-dat/{1_baton,…,B_waxworks}.dat
//!     https://ifarchive.org/if-archive/scott-adams/games/scottfree/mysterious.tar.gz
//! ```
//!
//! where `<fixtures>` is `$SCOTT_DIALECT_FIXTURES` or `stories/scott-dialects`.

use std::path::PathBuf;

use scott::c64::HeaderShape;
use scott::zx_mysterious as zx;
use scott::{Database, Dialect, LoadError, Options, Presentation, StepResult, Vm};

/// The eleven, paired with the stem of the reference-format conversion of the
/// same title.
const ORACLE: [(&str, &str); 11] = [
    ("m1goldba", "1_baton"),
    ("m2tmachi", "2_timemachine"),
    ("m3arrow1", "3_arrow1"),
    ("m4arrow2", "4_arrow2"),
    ("m5pulsar", "5_pulsar7"),
    ("m6circus", "6_circus"),
    ("m7feasib", "7_feasibility"),
    ("m8akyrtz", "8_akyrz"),
    ("m9perseu", "9_perseus"),
    ("m10india", "A_tenlittleindians"),
    ("m11waxwo", "B_waxworks"),
];

/// Per title, as MEASURED and as §10.3 partly pins: the header address the
/// §4.6 scan finds, the action table's address (the connections less
/// (action count + 1) × 16), the dictionary address §4.1's signature lands on,
/// the picture pointer (slot 0), and how many Family B images decode.
///
/// **None of it is an input to the loader.** Every number here is derived, and
/// pinning them is what would catch a corpus that quietly became a different
/// release — and what records the one structural surprise of the family: the
/// series ships **two** drivers, one putting the header at `$6349` and its
/// action table at `$7B56` (the two 1982 titles measured, *The Golden Baton*
/// and *Circus*) and one at `$6351` / `$7B81` (the other nine). §10.3 pins
/// two of the dictionary addresses (`0x873A` for *The Golden Baton*, `0x8B1D`
/// for *Escape from Pulsar 7*) and this suite pins all eleven.
///
/// The image count is the room count in every one of the eleven, which is
/// §8.6's Family B identity rule ("room *n* shows vector image *n* − 1") seen
/// from the other side.
const PINNED: [(&str, u16, u16, u16, u16, usize); 11] = [
    ("m1goldba", 0x6349, 0x7B56, 0x873A, 0x9B8B, 31),
    ("m2tmachi", 0x6351, 0x7B81, 0x875F, 0x979C, 44),
    ("m3arrow1", 0x6351, 0x7B81, 0x86B3, 0x98E6, 52),
    ("m4arrow2", 0x6351, 0x7B81, 0x89B7, 0x9E89, 65),
    ("m5pulsar", 0x6351, 0x7B81, 0x8B1D, 0xA143, 45),
    ("m6circus", 0x6349, 0x7B56, 0x871A, 0x97C5, 36),
    ("m7feasib", 0x6351, 0x7B81, 0x87BF, 0x9917, 59),
    ("m8akyrtz", 0x6351, 0x7B81, 0x897D, 0x9F5B, 40),
    ("m9perseu", 0x6351, 0x7B81, 0x8823, 0x9E9D, 40),
    ("m10india", 0x6351, 0x7B81, 0x87B7, 0x99A6, 63),
    ("m11waxwo", 0x6351, 0x7B81, 0x88D3, 0x9DFC, 41),
];

/// The eleven header count sets, as the snapshots store them: items, actions,
/// words, rooms, max carried, word length, messages, and the lamp.
///
/// §6.1's third identification route says "every ZX release in the series
/// stores exactly **82** messages"; **measured, that is wrong** — only *Arrow
/// of Death part 1* stores 82, and the eleven spread from 65 to 99. The
/// numbers here are the record of it.
type Counts = (&'static str, u16, u16, u16, u16, u16, u16, u16, i32);

const COUNTS: [Counts; 11] = [
    ("m1goldba", 48, 171, 76, 31, 5, 4, 99, 200),
    ("m2tmachi", 62, 164, 87, 44, 6, 4, 73, 200),
    ("m3arrow1", 64, 150, 90, 52, 5, 4, 82, 32766),
    ("m4arrow2", 91, 190, 83, 65, 9, 4, 87, 400),
    ("m5pulsar", 90, 220, 145, 45, 6, 4, 75, 200),
    ("m6circus", 65, 165, 97, 36, 6, 4, 72, 150),
    ("m7feasib", 65, 164, 82, 59, 5, 4, 65, 200),
    ("m8akyrtz", 49, 201, 85, 40, 6, 4, 99, 500),
    ("m9perseu", 60, 178, 130, 40, 6, 4, 96, 200),
    ("m10india", 73, 161, 85, 63, 5, 4, 67, 500),
    ("m11waxwo", 57, 189, 106, 41, 6, 4, 91, 250),
];

/// The four titles whose every table is byte-identical to its published
/// conversion, so this suite can demand equality rather than overlap.
///
/// Items are compared on text, location and treasure flag rather than on the
/// whole [`scott::Item`]: these releases spell an item's noun as the
/// dictionary cell it names, `*` and all (`Small Bush/*BUSH/`), where the
/// conversions simply dropped the marker. See `AUTO_NOUN_EXTRAS`.
const IDENTICAL: [&str; 4] = ["m2tmachi", "m5pulsar", "m6circus", "m8akyrtz"];

/// The six different-release titles, with the overlap against their
/// conversions **as measured**, per table: actions, verbs, nouns, rooms,
/// messages, items (text + location + treasure).
///
/// Pinned rather than floored, exactly as `c64_specimens` and
/// `ti994a_specimens` pin their own numbers, because a floor either passes
/// everything or fails an honest release. Read them as a description of how
/// far apart the two editions are, not as a quality score: *The Golden
/// Baton*'s snapshot has five more action lines and two fewer words than its
/// conversion and shares 76 of 79 verb cells, which is the number that could
/// not come out right by accident, while its room texts are a complete
/// rewrite (0 of 32 match).
const OVERLAPS: [(&str, usize, usize, usize, usize, usize, usize); 6] = [
    ("m1goldba", 13, 76, 77, 0, 10, 20),
    ("m3arrow1", 150, 91, 89, 26, 62, 52),
    ("m4arrow2", 23, 83, 81, 21, 42, 53),
    ("m7feasib", 3, 80, 80, 8, 18, 55),
    ("m9perseu", 2, 130, 129, 31, 65, 50),
    ("m11waxwo", 183, 92, 103, 25, 41, 44),
];

/// The nine snapshots in the same archive that are **not** ZX Mysterious
/// releases, with the refusal each must produce (§11: name it rather than
/// guess).
///
/// Four are other Scott Adams games and must be refused by DIALECT — two
/// family-A releases (§8.1) that answer the plain dictionary signature, and
/// two that compress both their action table and their text (§5.1, §5.2) and
/// answer the mixed-case one. The remaining five are not Scott Adams games at
/// all and carry no signature in either their raw or their decompressed form,
/// so `detect_dialect` answers `None` and the text lexer's own error stands.
const CONTROLS: [(&str, Option<Dialect>); 9] = [
    ("gremlins", Some(Dialect::C64OrZxSnapshot)),
    ("supergra", Some(Dialect::C64OrZxSnapshot)),
    ("sherwood", Some(Dialect::CompressedActionTable)),
    ("seablood", Some(Dialect::CompressedActionTable)),
    ("rbplanet", None),
    ("blizzard", None),
    ("heman", None),
    ("kayleth", None),
    ("temple", None),
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

/// The `.z80` snapshot for `stem`, as distributed.
fn snapshot(stem: &str) -> Option<Vec<u8>> {
    std::fs::read(fixtures()?.join("spectrum").join(format!("{stem}.z80"))).ok()
}

/// The 48K memory image `stem`'s snapshot decompresses to (§7.1).
fn image(stem: &str) -> Option<Vec<u8>> {
    scott::decompress_z80(&snapshot(stem)?).ok()
}

fn dat(stem: &str) -> Option<Database> {
    let bytes = std::fs::read(fixtures()?.join("mysterious-dat").join(format!("{stem}.dat"))).ok()?;
    Database::parse(&bytes).ok()
}

/// Every Mysterious specimen that is present, as (stem, 48K image).
fn corpus() -> Vec<(&'static str, Vec<u8>)> {
    ORACLE.iter().filter_map(|(stem, _)| Some((*stem, image(stem)?))).collect()
}

/// A vacuous skip reads exactly like a pass, so say why.
fn skipped(what: &str) -> bool {
    eprintln!(
        "SKIP: {what} not found. Put the twenty snapshots under \
         stories/scott-dialects/spectrum/ and the eleven .dat conversions under \
         stories/scott-dialects/mysterious-dat/, or point SCOTT_DIALECT_FIXTURES \
         at a directory holding both."
    );
    true
}

// ── Locating, with no catalogue ───────────────────────────────────────────────

#[test]
fn the_dictionary_signature_occurs_exactly_once_where_the_specification_pins_it() {
    let files = corpus();
    if files.is_empty() {
        assert!(skipped("the ZX Spectrum snapshots"));
        return;
    }
    for (stem, image) in &files {
        let (_, _, _, dictionary, _, _) =
            PINNED.iter().find(|p| p.0 == *stem).copied().expect("a pinned row per title");
        let hits: Vec<u16> = image
            .windows(8)
            .enumerate()
            .filter(|(_, w)| *w == b"AUTO\0GO\0")
            .map(|(i, _)| u16::try_from(i).unwrap() + 0x4000)
            .collect();
        assert_eq!(hits, [dictionary], "{stem}: §4.1's signature, exactly once");
    }
    assert_eq!(files.len(), 11, "all eleven specimens present");
}

#[test]
fn the_scan_finds_exactly_one_early_shape_header_per_title() {
    let files = corpus();
    if files.is_empty() {
        assert!(skipped("the ZX Spectrum snapshots"));
        return;
    }
    for (stem, image) in &files {
        let (_, header, actions, dictionary, pictures, _) =
            PINNED.iter().find(|p| p.0 == *stem).copied().unwrap();
        let layout = zx::locate(image).unwrap_or_else(|e| panic!("{stem} locates: {e:?}"));
        assert_eq!(layout.header.addr, header, "{stem}: §4.6's scan address");
        assert_eq!(
            layout.header.shape,
            HeaderShape::Early,
            "{stem}: §4.5's early field order — the reference format's own"
        );
        assert_eq!(layout.tables.actions, actions, "{stem}: the action table");
        assert_eq!(layout.tables.verbs, dictionary, "{stem}: slot 16 IS the signature hit");
        assert_eq!(layout.tables.pictures, pictures, "{stem}: slot 0, one past the items");
        // The pointer block's own redundancy, and §4.3's early reading order
        // seen as physical adjacency: actions, connections, locations, the
        // driver's copy of them, verbs, nouns, room descriptions, messages,
        // item descriptions, pictures — in that order, with nothing between.
        let t = layout.tables;
        let h = layout.header;
        assert!(t.actions < t.connections, "{stem}");
        assert_eq!(t.locations - t.connections, 6 * (h.rooms + 1), "{stem}: §4.4 exits");
        assert_eq!(t.locations_copy - t.locations, h.items + 1, "{stem}: §4.4 locations");
        assert_eq!(t.verbs - t.locations_copy, h.items + 3, "{stem}: the copy, and two spare");
        let cells = (h.words + 1) * (h.word_length + 1);
        assert_eq!(t.nouns - t.verbs, cells, "{stem}: (words + 1) verb cells");
        assert_eq!(t.rooms - t.nouns, cells, "{stem}: (words + 1) noun cells");
        assert!(t.rooms < t.messages && t.messages < t.items && t.items < t.pictures, "{stem}");
    }
}

#[test]
fn every_header_count_is_the_one_measured() {
    let files = corpus();
    if files.is_empty() {
        assert!(skipped("the ZX Spectrum snapshots"));
        return;
    }
    for (stem, image) in &files {
        let (_, items, actions, words, rooms, carried, length, messages, lamp) =
            COUNTS.iter().find(|c| c.0 == *stem).copied().unwrap();
        let h = zx::locate(image).unwrap().header;
        assert_eq!(
            (h.items, h.actions, h.words, h.rooms, h.max_carry, h.word_length, h.messages),
            (items, actions, words, rooms, carried, length, messages),
            "{stem}: §4.5's early header, read at the scanned address"
        );
        let db = zx::parse_zx_mysterious(image).unwrap();
        assert_eq!(db.light_time, lamp, "{stem}: the lamp, sign-extended");
        assert_eq!(db.verbs.len(), usize::from(words) + 1, "{stem}: the verb block");
        assert_eq!(db.nouns.len(), usize::from(words) + 1, "{stem}: the noun block");
    }
    // §6.1's "every ZX release in the series stores exactly 82 messages" is
    // false; this is the count that is actually shared.
    let word_lengths: Vec<u16> = COUNTS.iter().map(|c| c.6).collect();
    assert!(word_lengths.iter().all(|&w| w == 4), "§6.1: word length 4 throughout the series");
    assert_eq!(
        COUNTS.iter().filter(|c| c.7 == 82).count(),
        1,
        "only one of the eleven stores 82 messages, contra §6.1"
    );
}

#[test]
fn the_nine_other_snapshots_are_refused_by_name_and_nothing_panics() {
    let present: Vec<_> =
        CONTROLS.iter().filter_map(|(stem, d)| Some((*stem, snapshot(stem)?, *d))).collect();
    if present.is_empty() {
        assert!(skipped("the ZX Spectrum control snapshots"));
        return;
    }
    for (stem, file, dialect) in &present {
        assert_eq!(scott::detect_dialect(file), *dialect, "{stem}: the dialect it is");
        // The loader must not claim it…
        assert!(!zx::looks_like_zx_mysterious_z80(file), "{stem} is not a ZX Mysterious release");
        if let Ok(image) = scott::decompress_z80(file) {
            assert!(
                matches!(zx::locate(&image), Err(LoadError::UnsupportedDialect(_))),
                "{stem}: §4.6's scan must find no header"
            );
        }
        // …and `Database::parse` must refuse by name rather than by whatever
        // token the text lexer hit first — for the four that ARE Scott games.
        let err = Database::parse(file).expect_err(&format!("{stem} must not load"));
        match dialect {
            Some(d) => assert_eq!(
                err,
                LoadError::UnsupportedDialect(*d),
                "{stem}: refused as the dialect it is"
            ),
            None => assert!(
                !matches!(err, LoadError::UnsupportedDialect(_)),
                "{stem} is no Scott Adams game at all, so no dialect may be claimed: {err:?}"
            ),
        }
    }
    assert_eq!(present.len(), 9, "all nine controls present");
}

// ── The oracle, table by table ────────────────────────────────────────────────

#[test]
fn four_titles_decode_identically_to_their_published_conversions() {
    let files = corpus();
    if files.is_empty() {
        assert!(skipped("the ZX Spectrum snapshots"));
        return;
    }
    let mut checked = 0;
    for (stem, oracle_stem) in ORACLE {
        if !IDENTICAL.contains(&stem) {
            continue;
        }
        let (Some(image), Some(want)) = (image(stem), dat(oracle_stem)) else {
            assert!(skipped(&format!("{stem}.z80 or {oracle_stem}.dat")));
            return;
        };
        let got = zx::parse_zx_mysterious(&image).unwrap();
        assert_eq!(got.max_carry, want.max_carry, "{stem}: max carried");
        assert_eq!(got.start_room, want.start_room, "{stem}: start room");
        assert_eq!(got.num_treasures, want.num_treasures, "{stem}: treasures");
        assert_eq!(got.treasure_room, want.treasure_room, "{stem}: treasure room");
        assert_eq!(got.word_length, want.word_length, "{stem}: word length");
        assert_eq!(got.light_time, want.light_time, "{stem}: lamp");
        assert_eq!(got.actions, want.actions, "{stem}: the whole action table");
        assert_eq!(got.verbs, want.verbs, "{stem}: every verb cell");
        assert_eq!(got.nouns, want.nouns, "{stem}: every noun cell");
        assert_eq!(got.rooms, want.rooms, "{stem}: every room text and exit");
        assert_eq!(got.messages, want.messages, "{stem}: every message");
        assert_eq!(got.items.len(), want.items.len(), "{stem}: item count");
        for (i, (g, w)) in got.items.iter().zip(&want.items).enumerate() {
            assert_eq!((&g.text, g.start_loc, g.treasure), (&w.text, w.start_loc, w.treasure),
                "{stem}: item {i}");
        }
        checked += 1;
    }
    assert_eq!(checked, IDENTICAL.len(), "every identical-table title was checked");
}

#[test]
fn ten_little_indians_differs_from_its_conversion_in_three_condition_words_only() {
    let (Some(image), Some(want)) = (image("m10india"), dat("A_tenlittleindians")) else {
        assert!(skipped("m10india.z80 or A_tenlittleindians.dat"));
        return;
    };
    let got = zx::parse_zx_mysterious(&image).unwrap();
    // Everything but the action table is identical.
    assert_eq!(got.verbs, want.verbs);
    assert_eq!(got.nouns, want.nouns);
    assert_eq!(got.rooms, want.rooms);
    assert_eq!(got.messages, want.messages);
    assert_eq!(got.actions.len(), want.actions.len());
    let differing: Vec<usize> = got
        .actions
        .iter()
        .zip(&want.actions)
        .enumerate()
        .filter(|(_, (g, w))| g != w)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(differing, [5, 69, 101], "the three lines the two releases disagree on");
    for i in differing {
        let (g, w) = (&got.actions[i], &want.actions[i]);
        assert_eq!((g.verb, g.noun), (w.verb, w.noun), "action {i}: same trigger");
        assert_eq!(g.commands, w.commands, "action {i}: same commands");
        assert_eq!(&g.conditions[..4], &w.conditions[..4], "action {i}: same first four");
        assert_ne!(
            g.conditions[4], w.conditions[4],
            "action {i}: the snapshot carries a fifth condition the conversion does not"
        );
    }
}

#[test]
fn the_other_six_titles_overlap_their_conversions_as_measured() {
    let files = corpus();
    if files.is_empty() {
        assert!(skipped("the ZX Spectrum snapshots"));
        return;
    }
    for (stem, actions, verbs, nouns, rooms, messages, items) in OVERLAPS {
        let oracle_stem = ORACLE.iter().find(|o| o.0 == stem).unwrap().1;
        let (Some(image), Some(want)) = (image(stem), dat(oracle_stem)) else {
            assert!(skipped(&format!("{stem}.z80 or {oracle_stem}.dat")));
            return;
        };
        let got = zx::parse_zx_mysterious(&image).unwrap();
        let eq = |a: usize| a;
        assert_eq!(
            eq(got.actions.iter().zip(&want.actions).filter(|(a, b)| a == b).count()),
            actions,
            "{stem}: action records shared with the conversion"
        );
        assert_eq!(
            got.verbs.iter().zip(&want.verbs).filter(|(a, b)| a == b).count(),
            verbs,
            "{stem}: verb cells shared"
        );
        assert_eq!(
            got.nouns.iter().zip(&want.nouns).filter(|(a, b)| a == b).count(),
            nouns,
            "{stem}: noun cells shared"
        );
        assert_eq!(
            got.rooms.iter().zip(&want.rooms).filter(|(a, b)| a == b).count(),
            rooms,
            "{stem}: rooms shared"
        );
        assert_eq!(
            got.messages.iter().zip(&want.messages).filter(|(a, b)| a == b).count(),
            messages,
            "{stem}: messages shared"
        );
        assert_eq!(
            got.items
                .iter()
                .zip(&want.items)
                .filter(|(a, b)| a.text == b.text
                    && a.start_loc == b.start_loc
                    && a.treasure == b.treasure)
                .count(),
            items,
            "{stem}: item text and locations shared"
        );
    }
}

#[test]
fn the_snapshots_spell_an_auto_noun_the_conversions_dropped() {
    // Both halves of §2.5's rule, on real data: where the conversion has a
    // plain `/SWOR/` the snapshot has the same, and where the conversion has
    // no marker at all the snapshot often has `/*NOUN/` — the dictionary cell
    // the item names, synonym star included. Such an auto-noun matches no
    // typed word (a `*` is not a letter), so the item behaves exactly as the
    // conversion's markerless one does, and this suite pins that rather than
    // "repairing" stored data.
    let (Some(image), Some(want)) = (image("m2tmachi"), dat("2_timemachine")) else {
        assert!(skipped("m2tmachi.z80 or 2_timemachine.dat"));
        return;
    };
    let got = zx::parse_zx_mysterious(&image).unwrap();
    let extra: Vec<&str> = got
        .items
        .iter()
        .zip(&want.items)
        .filter(|(g, w)| w.auto_noun.is_none() && g.auto_noun.is_some())
        .map(|(g, _)| g.auto_noun.as_deref().unwrap())
        .collect();
    assert_eq!(extra.len(), 41, "the markers this release carries and its conversion does not");
    assert!(extra.iter().all(|n| n.starts_with('*')), "every one of them is a synonym cell: {extra:?}");
    // And none of them can be typed, so nothing became gettable that was not.
    for noun in &extra {
        assert_eq!(got.match_noun(noun), None, "{noun} must match no vocabulary entry");
    }
}

// ── The dictionary grid (§4.2) ────────────────────────────────────────────────

#[test]
fn all_nul_cells_are_read_as_empty_cells_rather_than_skipped() {
    let files = corpus();
    if files.is_empty() {
        assert!(skipped("the ZX Spectrum snapshots"));
        return;
    }
    // §4.2: "on the ZX Spectrum releases … all-NUL cells are common, up to 27
    // in one dictionary — and the escape must not be applied there". Measured
    // here it is up to 47, and the proof the escape is off is that every
    // dictionary still holds exactly (word count + 1) cells per block and ends
    // on the pointer that follows it (asserted above).
    let mut most = 0;
    for (stem, image) in &files {
        let db = zx::parse_zx_mysterious(image).unwrap();
        let empty = db.verbs.iter().chain(&db.nouns).filter(|c| c.is_empty()).count();
        most = most.max(empty);
        // §5.3's direction-noun repair is NOT needed: the seven cells it would
        // install are already there, straight out of the grid.
        assert_eq!(
            &db.nouns[..7],
            ["ANY", "NORT", "SOUT", "EAST", "WEST", "UP", "DOWN"],
            "{stem}: noun cells 0-6 as stored"
        );
    }
    assert_eq!(most, 47, "the deepest run of all-NUL cells in the corpus");
}

// ── Pictures (§8.2, §8.6) ─────────────────────────────────────────────────────

#[test]
fn every_title_decodes_one_family_b_image_per_room() {
    let files = corpus();
    if files.is_empty() {
        assert!(skipped("the ZX Spectrum snapshots"));
        return;
    }
    for (stem, image) in &files {
        let (_, _, _, _, _, pictures) = PINNED.iter().find(|p| p.0 == *stem).copied().unwrap();
        let lists = zx::decode_picture_lists(image).unwrap_or_else(|e| panic!("{stem}: {e:?}"));
        let db = zx::parse_zx_mysterious(image).unwrap();
        assert_eq!(lists.len(), pictures, "{stem}: images decoded");
        assert_eq!(
            lists.len(),
            db.rooms.len() - 1,
            "{stem}: §8.6's identity — one image per room, room 0 excepted"
        );
        for (i, list) in lists.iter().enumerate() {
            assert!(list.background < 16, "{stem}: image {i} background is a palette index");
            // §8.2's derived line colour.
            assert_eq!(
                list.line,
                if list.background == 0 { 7 } else { 0 },
                "{stem}: image {i}'s line colour is derived, not stored"
            );
            assert!(!list.ops.is_empty(), "{stem}: image {i} draws nothing");
        }
        // And they rasterise, on §8.2's own canvas.
        let drawn = zx::decode_pictures(image).unwrap();
        assert_eq!(drawn.len(), pictures);
        assert!(drawn.iter().all(|p| p.width == 255 && p.height == 94), "{stem}: §8.2's canvas");
    }
}

// ── The host's own route (§7.1, Appendix A) ───────────────────────────────────

#[test]
fn database_parse_reads_a_snapshot_straight_from_its_file_bytes() {
    let files: Vec<_> = ORACLE.iter().filter_map(|(s, _)| Some((*s, snapshot(s)?))).collect();
    if files.is_empty() {
        assert!(skipped("the ZX Spectrum snapshots"));
        return;
    }
    for (stem, file) in &files {
        // The sniff a multi-engine host runs before it commits to an engine.
        assert!(scott::looks_like_scott_bytes(file), "{stem}: the host's own sniff");
        let via_parse =
            Database::parse(file).unwrap_or_else(|e| panic!("{stem} loads through parse: {e:?}"));
        let direct = zx::parse_zx_mysterious(&scott::decompress_z80(file).unwrap()).unwrap();
        assert_eq!(via_parse, direct, "{stem}: one entry point, one answer");
        // Appendix A: the runtime differences of §9 travel with the database.
        assert!(via_parse.mysterious, "{stem}: §9.2's two lamp options");
        assert!(via_parse.second_person, "{stem}: §9.3's ZX wording");
    }
    assert_eq!(files.len(), 11);
}

#[test]
fn a_real_release_boots_second_person_and_plays_a_turn() {
    let Some(file) = snapshot("m5pulsar") else {
        assert!(skipped("m5pulsar.z80"));
        return;
    };
    let db = Database::parse(&file).unwrap();
    // §9.3's black-box test: the room preamble must read "You are in a ", and
    // an inventory "You are carrying:". The host asks for neither.
    let mut vm =
        Vm::new_full(db, false, 7, Options::new().with_presentation(Presentation::ScottFree));
    assert_eq!(vm.step(), StepResult::NeedLine);
    // The opening is the game's own dedication card, printed from its message
    // pool; §9.3's wording shows up in the DRIVER's replies from the first
    // turn on.
    let opening = vm.take_output();
    assert!(opening.contains("ESCAPE FROM PULSAR 7"), "the release's own title card: {opening:?}");

    // A direction the room has no exit for: §6.4's table gives the ZX release
    // `You can't go in that direction. ` where Adventure International says
    // `I can't go in that direction. `.
    vm.supply_line("north");
    assert_eq!(vm.step(), StepResult::NeedLine);
    let blocked = vm.take_output();
    assert!(
        blocked.contains("You can't go in that direction"),
        "§9.3: a ZX Mysterious release is second person, got {blocked:?}"
    );
    assert!(!blocked.contains("I can't go"), "the first-person set must not appear: {blocked:?}");

    // And §6.4's inventory heading, which is the other half of §9.3's test.
    vm.supply_line("inventory");
    assert_eq!(vm.step(), StepResult::NeedLine);
    let inventory = vm.take_output();
    assert!(inventory.contains("You are carrying"), "got {inventory:?}");
    assert!(!inventory.contains("I'm carrying"), "got {inventory:?}");
}

// ── Robustness ────────────────────────────────────────────────────────────────

#[test]
fn two_hundred_byte_flips_of_a_real_image_load_or_refuse_and_never_panic() {
    let Some(base) = image("m1goldba") else {
        assert!(skipped("m1goldba.z80"));
        return;
    };
    let mut seed = 0x9E37_79B9u32;
    for _ in 0..200 {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let at = (seed >> 7) as usize % base.len();
        let mut image = base.clone();
        image[at] ^= (seed >> 3) as u8 | 1;
        let _ = zx::parse_zx_mysterious(&image);
        let _ = zx::decode_picture_lists(&image);
        let _ = zx::identify(&image);
    }
    // And a truncated image, which is the other half of the same question.
    for len in [0, 1, 0x100, 0x6000, base.len() - 1] {
        let _ = zx::parse_zx_mysterious(&base[..len]);
    }
}

#[test]
fn identify_names_every_one_of_the_eleven() {
    let files: Vec<_> = ORACLE.iter().filter_map(|(s, _)| Some((*s, snapshot(s)?))).collect();
    if files.is_empty() {
        assert!(skipped("the ZX Spectrum snapshots"));
        return;
    }
    for (stem, file) in &files {
        let release = zx::identify_z80(file).unwrap_or_else(|| panic!("{stem} is catalogued"));
        assert_eq!(release.file_name, format!("{stem}.z80"), "{stem}: the row is its own");
        assert!(!release.title.is_empty());
    }
    // Spot-check two titles by name, so a shuffled table cannot pass.
    assert_eq!(zx::identify_z80(&snapshot("m1goldba").unwrap()).unwrap().title, "The Golden Baton");
    assert_eq!(zx::identify_z80(&snapshot("m11waxwo").unwrap()).unwrap().title, "Waxworks");
}
