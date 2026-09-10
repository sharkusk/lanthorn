//! The Commodore 64 *Mysterious Adventures* loader against the
//! reference-format oracle (SQ-1414).
//!
//! `docs/internals/scott-dialects-spec.md` §10.1 names the oracle: many Scott
//! Adams games exist both as a platform-native binary and as a plain-text
//! conversion, so decoding the native file and comparing the resulting tables
//! field for field against the text file is a black-box check that needs
//! nothing from the specification and nothing from any GPL implementation.
//!
//! # How strong an oracle it is here — stronger than for the TI-99/4A
//!
//! For **six** of the eleven titles the agreement is total: the action table,
//! the room descriptions and connections, the messages, the item descriptions
//! and locations and both vocabulary columns come out of the Commodore 64
//! program file **identical** to the same tables built from the published
//! `.dat` conversion. Those six are *The Golden Baton*, *Arrow of Death* parts
//! 1 and 2, *Feasibility Experiment*, *Perseus and Andromeda* and *Waxworks*,
//! and this suite asserts that equality per table, not merely a resemblance.
//!
//! The other five are **different releases of the same games**, exactly as
//! §10.3 records for the ZX *Golden Baton*: *Circus* is a different text
//! revision (its actions, connections and item locations still match), and
//! *The Time Machine*, *Escape from Pulsar 7*, *The Wizard of Akyrz* and *Ten
//! Little Indians* have different header counts from their conversions. For
//! those this suite asserts the order-independent overlaps the TI-99/4A suite
//! uses — a loader that mis-read the header shape, the pointer block or the
//! cell width could not produce the room, item and vocabulary text of a
//! different release of the same game by accident.
//!
//! # Getting the corpus
//!
//! Both halves are commercial game files, neither is redistributable, and
//! neither is committed. Everything here **skips vacuously with an
//! explanation** when either half is absent — a silent skip reads exactly like
//! a pass, so the skip says why.
//!
//! ```text
//! <fixtures>/c64/prg/MYSTADV1.D64/{BATON,TIME MACHINE,ARROW I,ARROW II,PULSAR 7,CIRCUS}.prg
//! <fixtures>/c64/prg/MYSTADV2.D64/{EXPERIMENT,WIZARD OF AKYRZ,PERSEUS,INDIANS,WAXWORKS}.prg
//!     extracted from https://ifarchive.org/if-archive/scott-adams/games/c64/mystadv.zip
//! <fixtures>/mysterious-dat/{1_baton,…,B_waxworks}.dat
//!     https://ifarchive.org/if-archive/scott-adams/games/scottfree/mysterious.tar.gz
//! ```
//!
//! where `<fixtures>` is `$SCOTT_DIALECT_FIXTURES` or `stories/scott-dialects`.

use std::path::PathBuf;

use scott::c64::{
    decode_family_b_pictures, identify, image_checksum, prg_image, HeaderShape, RELEASES,
};
use scott::{Database, Options, Vm};

/// The eleven, in the order [`RELEASES`] lists them, paired with the stem of
/// the reference-format conversion of the same title.
const ORACLE: [(&str, &str); 11] = [
    ("BATON", "1_baton"),
    ("TIME MACHINE", "2_timemachine"),
    ("ARROW I", "3_arrow1"),
    ("ARROW II", "4_arrow2"),
    ("PULSAR 7", "5_pulsar7"),
    ("CIRCUS", "6_circus"),
    ("EXPERIMENT", "7_feasibility"),
    ("WIZARD OF AKYRZ", "8_akyrz"),
    ("PERSEUS", "9_perseus"),
    ("INDIANS", "A_tenlittleindians"),
    ("WAXWORKS", "B_waxworks"),
];

/// The five different-release titles, with the overlap against their
/// conversions **as measured**, per table: rooms, items, messages, verbs,
/// nouns. Pinned rather than floored, exactly as `ti994a_specimens` pins its
/// own numbers — "it asserts, per file and per table, the agreement that was
/// measured", because a floor either passes everything or fails an honest
/// release.
///
/// Read them as a description of how far apart the two editions are, not as a
/// quality score. *Escape from Pulsar 7* shares seven room texts out of 45
/// because the Commodore 64 edition names rooms in two words (`Social room`)
/// where the conversion writes sentences (`*I'm in the freighter's social
/// room`) — and it shares **all 146** verb cells, which is the number that
/// could not come out right by accident. *The Wizard of Akyrz* is the most
/// distant edition of the five and still agrees on 64 of 67 verbs and 84 of
/// 86 nouns; *Ten Little Indians* is the closest, differing in one action
/// line's vocabulary word and a handful of texts.
const OVERLAPS: [(&str, usize, usize, usize, usize, usize); 5] = [
    ("TIME MACHINE", 28, 61, 52, 86, 84),
    ("PULSAR 7", 7, 53, 32, 146, 100),
    ("CIRCUS", 33, 58, 48, 96, 93),
    ("WIZARD OF AKYRZ", 5, 18, 20, 64, 84),
    ("INDIANS", 60, 72, 58, 64, 83),
];

/// The six titles whose tables are byte-identical to their conversions, so
/// this suite can demand equality rather than overlap.
const IDENTICAL: [&str; 6] =
    ["BATON", "ARROW I", "ARROW II", "EXPERIMENT", "PERSEUS", "WAXWORKS"];

/// §10.4 and §6.2, pinned: per title, the program file's own 16-bit checksum,
/// its header shape, and the dictionary address §4.1's signature lands on.
/// Nothing here is an input to the loader — the checksum and the shape are,
/// but the dictionary address is derived twice (from the signature and from
/// §6.2's seventh pointer) and pinning it is what would catch a corpus that
/// quietly became a different release.
const PINNED: [(&str, u16, HeaderShape, u16, u16); 11] = [
    ("BATON", 0x01FF, HeaderShape::Mysterious, 0x685F, 0x78EF),
    ("TIME MACHINE", 0xBBDD, HeaderShape::Mysterious, 0x680F, 0x7870),
    ("ARROW I", 0xAFE0, HeaderShape::Mysterious, 0x675F, 0x78E0),
    ("ARROW II", 0x1A8C, HeaderShape::Arrow2, 0x68FF, 0x7CAA),
    ("PULSAR 7", 0x1058, HeaderShape::Early, 0x69E1, 0x7BF2),
    ("CIRCUS", 0x1194, HeaderShape::Mysterious, 0x684F, 0x7912),
    ("EXPERIMENT", 0xDB00, HeaderShape::Early, 0x67C1, 0x7874),
    ("WIZARD OF AKYRZ", 0xA7FE, HeaderShape::Mysterious, 0x6A6F, 0x7BCC),
    ("PERSEUS", 0xE116, HeaderShape::Early, 0x6851, 0x7D8F),
    ("INDIANS", 0x0AD7, HeaderShape::TenLittleIndians, 0x680E, 0x7A44),
    ("WAXWORKS", 0x25CC, HeaderShape::Early, 0x69D1, 0x7F2F),
];

/// Every release loads at `$4000` (§6.2's layout); it is read from the file's
/// first two bytes all the same, since §7.2 is explicit that there is no fixed
/// Commodore 64 base address.
const LOAD: u16 = 0x4000;

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

/// The program file for `name`, wherever the compilation disk it came off put
/// it. The two disks each hold some of the eleven and neither holds all.
fn prg(name: &str) -> Option<Vec<u8>> {
    let root = fixtures()?.join("c64").join("prg");
    ["MYSTADV1.D64", "MYSTADV2.D64"]
        .iter()
        .map(|disk| root.join(disk).join(format!("{name}.prg")))
        .find_map(|p| std::fs::read(p).ok())
}

fn dat(stem: &str) -> Option<Database> {
    let bytes = std::fs::read(fixtures()?.join("mysterious-dat").join(format!("{stem}.dat"))).ok()?;
    Database::parse(&bytes).ok()
}

/// Every specimen that is present, as (name, program file).
fn corpus() -> Vec<(&'static str, Vec<u8>)> {
    ORACLE.iter().filter_map(|(name, _)| Some((*name, prg(name)?))).collect()
}

/// A vacuous skip reads exactly like a pass, so say why.
fn skipped(what: &str) -> bool {
    eprintln!(
        "SKIP: {what} not found. Put the eleven program files under \
         stories/scott-dialects/c64/prg/MYSTADV{{1,2}}.D64/ and the eleven .dat \
         conversions under stories/scott-dialects/mysterious-dat/, or point \
         SCOTT_DIALECT_FIXTURES at a directory holding both."
    );
    true
}

// ── Identification and layout ─────────────────────────────────────────────────

#[test]
fn every_specimen_identifies_as_the_release_the_specification_pins() {
    let files = corpus();
    if files.is_empty() {
        assert!(skipped("the Commodore 64 program files"));
        return;
    }
    for (name, file) in &files {
        let (image, load) = prg_image(file).expect("a program file has a load address");
        assert_eq!(load, LOAD, "{name} loads at $4000");
        let (_, checksum, shape, dictionary, pictures) =
            PINNED.iter().find(|p| p.0 == *name).copied().expect("a pinned row per title");
        assert_eq!(image_checksum(image, load), checksum, "{name}: §7.2 checksum");
        let release = identify(image, load).unwrap_or_else(|| panic!("{name} is catalogued"));
        assert_eq!(release.file_name, *name);
        assert_eq!(release.shape, shape, "{name}: §4.5 header shape");
        // The `AUTO\0GO\0` signature, §4.1, back-off 0 — and it occurs exactly
        // once, which is what makes an unanchored first-hit search safe.
        let hits: Vec<_> = image
            .windows(8)
            .enumerate()
            .filter(|(_, w)| *w == b"AUTO\0GO\0")
            .map(|(i, _)| u16::try_from(i).unwrap() + load)
            .collect();
        assert_eq!(hits, [dictionary], "{name}: §4.1 signature, exactly once");
        // §6.2's seventh pointer, the driver's own dictionary address.
        let seventh = u16::from_le_bytes([
            image[usize::from(0x4917 - load)],
            image[usize::from(0x491C - load)],
        ]);
        assert_eq!(seventh, dictionary, "{name}: §6.2's seventh pointer agrees with §4.1");
        // §6.2's `JMP $4D19` header marker.
        assert_eq!(
            &image[usize::from(0x5DD6 - load)..usize::from(0x5DD9 - load)],
            [0x4C, 0x19, 0x4D],
            "{name}: the header guard"
        );
        let _ = pictures;
    }
    assert_eq!(files.len(), 11, "all eleven specimens present");
}

#[test]
fn the_system_message_block_is_byte_identical_and_first_person_in_all_eleven() {
    let files = corpus();
    if files.is_empty() {
        assert!(skipped("the Commodore 64 program files"));
        return;
    }
    // §6.4: a 790-byte block at $4406, byte-identical across every one of the
    // eleven, holding 44 strings.
    let block = |file: &[u8]| {
        let (image, load) = prg_image(file).unwrap();
        image[usize::from(0x4406 - load)..usize::from(0x4406 - load) + 790].to_vec()
    };
    let first = block(&files[0].1);
    for (name, file) in &files {
        assert_eq!(block(file), first, "{name}: the system-message block differs");
    }
    let text = String::from_utf8_lossy(&first);
    // §6.4's own strings, and §9.3's test read the other way round: none of the
    // ZX second-person set occurs anywhere.
    for present in ["I'm in a ", "I am carrying:", "I'm not carrying it!", "Things I can see:"] {
        assert!(text.contains(present), "§6.4 says {present:?} is in the file");
    }
    for absent in ["You are in a", "You can also see", "You haven't got it", "You are carrying"] {
        for (name, file) in &files {
            assert!(
                !String::from_utf8_lossy(file).contains(absent),
                "{name}: {absent:?} must not occur anywhere — these releases are first-person"
            );
        }
    }
}

#[test]
fn the_direction_nouns_are_already_in_the_dictionary_so_the_5_3_repair_is_a_no_op() {
    // §5.3 says the Commodore 64 Mysterious releases "do not store direction
    // nouns in the dictionary" and that a loader must copy `ANY` and the six
    // direction words into noun cells 0-6. Measured on all eleven, they are
    // already there — so this loader does not apply that repair, and applying
    // it would truncate *The Time Machine*'s stored five-letter NORTH and
    // SOUTH to four characters. This case is the record of that measurement.
    let files = corpus();
    if files.is_empty() {
        assert!(skipped("the Commodore 64 program files"));
        return;
    }
    for (name, file) in &files {
        let db = Database::parse(file).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(db.nouns[0], "ANY", "{name}: noun cell 0");
        for (i, want) in ["NORT", "SOUT", "EAST", "WEST", "UP", "DOWN"].iter().enumerate() {
            assert!(
                db.nouns[i + 1].starts_with(want),
                "{name}: noun cell {} is {:?}, not a direction",
                i + 1,
                db.nouns[i + 1]
            );
        }
    }
}

// ── The oracle ────────────────────────────────────────────────────────────────

/// `a` is `b` up to trailing empty entries — the shape a memory image's
/// vocabulary column has against its conversion's, since the image simply does
/// not spend cells on the blank tail the `.dat` writes out.
fn prefix_with_blank_tail(a: &[String], b: &[String], what: &str, name: &str) {
    assert!(a.len() <= b.len(), "{name}: {what}: image has {} cells, .dat {}", a.len(), b.len());
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        assert_eq!(x, y, "{name}: {what} cell {i}");
    }
    for (i, y) in b[a.len()..].iter().enumerate() {
        assert!(y.is_empty(), "{name}: {what} cell {} is {y:?}, not blank padding", a.len() + i);
    }
}

#[test]
fn the_six_byte_identical_titles_decode_to_their_conversions_table_for_table() {
    let mut ran = 0;
    for (name, stem) in ORACLE {
        if !IDENTICAL.contains(&name) {
            continue;
        }
        let (Some(file), Some(want)) = (prg(name), dat(stem)) else {
            continue;
        };
        let got = Database::parse(&file).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(got.actions, want.actions, "{name}: actions");
        assert_eq!(got.rooms, want.rooms, "{name}: rooms (descriptions and exits)");
        assert_eq!(got.messages, want.messages, "{name}: messages");
        assert_eq!(got.items, want.items, "{name}: items (text, treasure, auto-noun, location)");
        prefix_with_blank_tail(&got.verbs, &want.verbs, "verbs", name);
        prefix_with_blank_tail(&got.nouns, &want.nouns, "nouns", name);
        // …and the header numbers that are not tables.
        assert_eq!(got.max_carry, want.max_carry, "{name}: max carried");
        assert_eq!(got.start_room, want.start_room, "{name}: start room");
        assert_eq!(got.num_treasures, want.num_treasures, "{name}: treasure count");
        assert_eq!(got.word_length, want.word_length, "{name}: word length");
        assert_eq!(got.light_time, want.light_time, "{name}: lamp turns");
        assert_eq!(got.treasure_room, want.treasure_room, "{name}: treasure room");
        eprintln!(
            "{name}: IDENTICAL — {} actions, {} rooms, {} messages, {} items, {}+{} words",
            got.actions.len(),
            got.rooms.len(),
            got.messages.len(),
            got.items.len(),
            got.verbs.len(),
            got.nouns.len()
        );
        ran += 1;
    }
    if ran == 0 {
        assert!(skipped("the program files and their .dat conversions"));
        return;
    }
    assert_eq!(ran, IDENTICAL.len(), "every byte-identical title was compared");
}

#[test]
fn the_other_five_overlap_their_conversions_the_way_different_releases_do() {
    let mut ran = 0;
    for (name, stem) in ORACLE {
        if IDENTICAL.contains(&name) {
            continue;
        }
        let (Some(file), Some(want)) = (prg(name), dat(stem)) else {
            continue;
        };
        let got = Database::parse(&file).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        // Order-independent overlaps, as `ti994a_specimens` does: how much of
        // one release's text the other also has.
        let overlap = |a: &[String], b: &[String]| -> usize {
            let mut b: Vec<&String> = b.iter().filter(|s| !s.is_empty()).collect();
            let mut n = 0;
            for x in a.iter().filter(|s| !s.is_empty()) {
                if let Some(i) = b.iter().position(|y| *y == x) {
                    b.remove(i);
                    n += 1;
                }
            }
            n
        };
        let rooms = overlap(
            &got.rooms.iter().map(|r| r.desc.clone()).collect::<Vec<_>>(),
            &want.rooms.iter().map(|r| r.desc.clone()).collect::<Vec<_>>(),
        );
        let items = overlap(
            &got.items.iter().map(|i| i.text.clone()).collect::<Vec<_>>(),
            &want.items.iter().map(|i| i.text.clone()).collect::<Vec<_>>(),
        );
        let verbs = overlap(&got.verbs, &want.verbs);
        let nouns = overlap(&got.nouns, &want.nouns);
        let messages = overlap(&got.messages, &want.messages);
        eprintln!(
            "{name}: different release — rooms {rooms}/{}, items {items}/{}, \
             messages {messages}/{}, verbs {verbs}/{}, nouns {nouns}/{}",
            got.rooms.len(),
            got.items.len(),
            got.messages.len(),
            got.verbs.len(),
            got.nouns.len()
        );
        let want = OVERLAPS
            .iter()
            .find(|o| o.0 == name)
            .unwrap_or_else(|| panic!("{name} has no pinned overlap row"));
        assert_eq!(
            (rooms, items, messages, verbs, nouns),
            (want.1, want.2, want.3, want.4, want.5),
            "{name}: the measured overlap changed"
        );
        ran += 1;
    }
    if ran == 0 {
        assert!(skipped("the program files and their .dat conversions"));
        return;
    }
    assert_eq!(ran, ORACLE.len() - IDENTICAL.len());
}

// ── The two §5.3 repairs ──────────────────────────────────────────────────────

#[test]
fn pulsar_7s_stored_action_count_is_replaced_and_the_table_then_lands_exactly() {
    let Some(file) = prg("PULSAR 7") else {
        assert!(skipped("PULSAR 7.prg"));
        return;
    };
    let (image, load) = prg_image(&file).unwrap();
    // The header stores 195 at word 2 of the early shape…
    let stored = u16::from_le_bytes([
        image[usize::from(0x5DD7 - load) + 4],
        image[usize::from(0x5DD7 - load) + 5],
    ]);
    assert_eq!(stored, 195, "§5.3: the stored count, which is wrong");
    // …and §5.3's replacement is 190, which the span between the header's end
    // and the dictionary confirms: $69E1 − $5DF1 = 3,056 bytes = 191 records.
    assert_eq!(RELEASES[4].action_count, Some(190));
    assert_eq!((0x69E1 - 0x5DF1) / 16, 191);
    let db = Database::parse(&file).expect("PULSAR 7 loads");
    assert_eq!(db.actions.len(), 191, "action count 190 means 191 records");
    // Every record validates: §5.3's own check.
    for (i, a) in db.actions.iter().enumerate() {
        assert!(u32::from(a.verb) * 150 + u32::from(a.noun) < 150 * 150, "action {i}");
        assert!(a.conditions.iter().all(|c| c.code <= 19), "action {i}");
    }
}

#[test]
fn the_time_machines_sixty_third_item_has_a_location_and_an_empty_description() {
    let Some(file) = prg("TIME MACHINE") else {
        assert!(skipped("TIME MACHINE.prg"));
        return;
    };
    let db = Database::parse(&file).expect("TIME MACHINE loads");
    // §5.3: the header's item count of 62 implies 63 records, the description
    // block holds 62 strings, and the location table is a full 63 bytes.
    assert_eq!(db.items.len(), 63);
    assert_eq!(db.items[62].text, "", "§5.3: an empty description, not a 63rd string");
    assert_eq!(db.items[61].text, "Broken Generator", "…and the 62nd is the last stored one");
    // The control §5.3 names: every other title's block holds exactly
    // (item count + 1) strings.
    for (name, _) in ORACLE {
        if name == "TIME MACHINE" {
            continue;
        }
        let Some(file) = prg(name) else { continue };
        let db = Database::parse(&file).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let blank = db.items.iter().filter(|i| i.text.is_empty()).count();
        assert!(blank <= 1, "{name}: {blank} items with no description");
    }
}

// ── Family B pictures (§8.2) ──────────────────────────────────────────────────

#[test]
fn the_picture_walk_yields_one_image_per_room_and_consumes_each_file_to_its_last_byte() {
    let files = corpus();
    if files.is_empty() {
        assert!(skipped("the Commodore 64 program files"));
        return;
    }
    for (name, file) in &files {
        let (image, load) = prg_image(file).unwrap();
        let db = Database::parse(file).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let pics = decode_family_b_pictures(image, load).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        // §8.6: "room n shows vector image n − 1", so a game with N rooms
        // beyond the copyright slot carries N images.
        assert_eq!(pics.len(), db.rooms.len() - 1, "{name}: one image per room");
        // §8.2's canvas, unconditionally.
        for (i, p) in pics.iter().enumerate() {
            assert_eq!((p.width, p.height), (255, 94), "{name} image {i}");
            assert_eq!(p.pixels.len(), 255 * 94, "{name} image {i}");
            assert_eq!(p.line, if p.background == 0 { 7 } else { 0 }, "{name} image {i}");
        }
        eprintln!("{name}: {} images for {} rooms", pics.len(), db.rooms.len() - 1);
    }
}

#[test]
fn the_picture_block_starts_where_the_specification_pins_it() {
    let files = corpus();
    if files.is_empty() {
        assert!(skipped("the Commodore 64 program files"));
        return;
    }
    for (name, file) in &files {
        let (image, load) = prg_image(file).unwrap();
        let (_, _, _, _, pictures) = PINNED.iter().find(|p| p.0 == *name).copied().unwrap();
        // §6.2: the first $FF at or after the driver's end-of-locations
        // pointer, past a run of 61, 76 or 101 zero bytes.
        let after = u16::from_le_bytes([
            image[usize::from(0x48E6 - load) + 5 * 8 + 1],
            image[usize::from(0x48E6 - load) + 5 * 8 + 5],
        ]);
        let from = usize::from(after - load);
        let gap = image[from..].iter().position(|&b| b == 0xFF).expect("a picture block");
        assert!((61..=101).contains(&gap), "{name}: a {gap}-byte zero run");
        assert_eq!(u16::try_from(from + gap).unwrap() + load, pictures, "{name}: picture address");
    }
}

// ── Robustness ────────────────────────────────────────────────────────────────

#[test]
fn two_hundred_byte_flips_in_a_real_program_file_never_panic() {
    let Some(file) = prg("BATON") else {
        assert!(skipped("BATON.prg"));
        return;
    };
    let mut seed = 0x9E37_79B9u32;
    for _ in 0..200 {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let at = (seed >> 8) as usize % file.len();
        let mut hurt = file.clone();
        hurt[at] ^= 1u8 << (seed % 8);
        // Almost all of these are refused on the checksum, which is the point
        // of §7.2's identity; what this guards is that none of them panics,
        // and that a database that IS produced is structurally sound.
        if let Ok(db) = Database::parse(&hurt) {
            assert!(db.start_room < db.rooms.len());
            for room in &db.rooms {
                assert!(room.exits.iter().all(|&e| e < db.rooms.len()));
            }
            assert_eq!(db.items.len(), db.items.len());
        }
        if let Some((image, load)) = prg_image(&hurt) {
            let _ = decode_family_b_pictures(image, load);
        }
    }
}

#[test]
fn a_loaded_release_plays_and_forces_the_series_lamp_options() {
    let Some(file) = prg("BATON") else {
        assert!(skipped("BATON.prg"));
        return;
    };
    let db = Database::parse(&file).expect("BATON loads");
    assert!(db.mysterious);
    let mut vm = Vm::new_full(db, false, 1, Options::default());
    // §9.2: "every Mysterious Adventures release … forces both on".
    assert!(vm.options().scott_light);
    assert!(vm.options().prehistoric_lamp);
    // And it actually plays: the opening room block names somewhere.
    assert_eq!(vm.step(), scott::StepResult::NeedLine);
    let block = vm.room_block();
    assert!(!block.is_empty(), "the opening room block is empty");
    vm.supply_line("look");
    assert_eq!(vm.step(), scott::StepResult::NeedLine);
    vm.supply_line("inventory");
    assert_eq!(vm.step(), scott::StepResult::NeedLine);
    eprintln!("BATON opening block:\n{block}");
}

// ── Family B at higher resolution (§8.2, SQ-1467) ─────────────────────────────

/// Majority vote over each `scale` x `scale` block of `big`, back onto the
/// native 255 x 94 grid. Ties go to the lowest palette index, which is
/// arbitrary but deterministic; no tie has ever decided a comparison here,
/// because a block that is split evenly is a block the eye reads as an edge
/// either way.
fn majority_downsample(big: &scott::c64::Picture) -> Vec<u8> {
    let s = big.scale as usize;
    let (w, h) = (big.width / s, big.height / s);
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut counts = [0u32; 16];
            for dy in 0..s {
                for dx in 0..s {
                    let p = big.pixels[(y * s + dy) * big.width + x * s + dx];
                    counts[usize::from(p) & 15] += 1;
                }
            }
            let mut best = 0usize;
            for (i, &c) in counts.iter().enumerate() {
                if c > counts[best] {
                    best = i;
                }
            }
            out[y * w + x] = best as u8;
        }
    }
    out
}

/// How far a scaled raster's majority downsample departs from the native one,
/// split into the two things a departure can mean.
#[derive(Default, Debug, Clone, Copy)]
struct Departure {
    /// Every native pixel where the two disagree.
    total: usize,
    /// Those of them that are nowhere near an EDGE: the native pixel's eight
    /// neighbours all agree with it, and its whole scaled block agrees with
    /// itself. A fill that leaked through a seam the supersample opened paints
    /// a whole REGION, so it lands here in the thousands; a staircase that
    /// resolved one step differently is a boundary pixel by construction and
    /// cannot land here at all.
    ///
    /// **Stated by uniformity rather than by the line colour** (SQ-1491). It
    /// used to ask whether the line colour was in the neighbourhood, which
    /// stopped meaning what it says once the colour clash landed: a line
    /// pixel in a cell some fill claimed later is drawn in that fill's ink and
    /// is not the line colour at all, so every moved staircase step in such a
    /// cell was counted as an interior leak. Uniformity is what the doc
    /// comment always meant and does not depend on any pixel's colour.
    interior: usize,
}

fn compare(native: &scott::c64::Picture, big: &scott::c64::Picture) -> Departure {
    let s = big.scale as usize;
    let (w, h) = (native.width, native.height);
    let small = majority_downsample(big);
    let mut d = Departure::default();
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (native.pixels[y * w + x], small[y * w + x]);
            if a == b {
                continue;
            }
            d.total += 1;
            let flat_native = (y.saturating_sub(1)..=(y + 1).min(h - 1))
                .flat_map(|ny| (x.saturating_sub(1)..=(x + 1).min(w - 1)).map(move |nx| (nx, ny)))
                .all(|(nx, ny)| native.pixels[ny * w + nx] == a);
            let flat_block = (0..s).all(|dy| {
                (0..s).all(|dx| big.pixels[(y * s + dy) * big.width + x * s + dx] == b)
            });
            if flat_native && flat_block {
                d.interior += 1;
            }
        }
    }
    d
}

/// **The topology of a picture does not depend on the size it is drawn at.**
///
/// Every image of every release is drawn twice — at §8.2's own 255 x 94 canvas
/// and at 2, 3 and 4 times that — and the large raster is voted back down to
/// the native grid, block by block. What must survive is the *regions*: a
/// scaled line is a finer staircase and its pixels move, but a fill that was
/// sealed at 1x by two lines touching at a corner must not find a seam at 4x
/// and flood the room, which is what a supersampled vector format gets wrong
/// if it gets anything wrong (SQ-1467).
///
/// So the assertion is on [`Departure::interior`] — a disagreement the ink
/// cannot account for — and it is **zero across the whole corpus at every
/// scale**. `Departure::total` is reported rather than asserted: it counts the
/// staircase pixels the finer resolution deliberately moved, which is the
/// feature and not a defect.
#[test]
fn every_picture_keeps_its_regions_at_every_supersample() {
    let files = corpus();
    if files.is_empty() {
        assert!(skipped("the Commodore 64 program files"));
        return;
    }
    for (name, file) in &files {
        let (image, load) = prg_image(file).unwrap();
        let lists = scott::c64::decode_family_b_picture_lists(image, load)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        for scale in [2u32, 3, 4] {
            let mut worst = (0usize, Departure::default());
            let mut totals = Departure::default();
            for (i, list) in lists.iter().enumerate() {
                let native = list.rasterise();
                let big = list.rasterise_at(scale);
                assert_eq!(
                    (big.width, big.height, big.scale),
                    (255 * scale as usize, 94 * scale as usize, scale),
                    "{name} image {i} at {scale}x"
                );
                let d = compare(&native, &big);
                assert_eq!(
                    d.interior, 0,
                    "{name} image {i} at {scale}x: {} pixels disagree with no ink to explain them \
                     — a fill reached somewhere the native raster sealed off",
                    d.interior
                );
                totals.total += d.total;
                totals.interior += d.interior;
                if d.total > worst.1.total {
                    worst = (i, d);
                }
            }
            let px = lists.len() * 255 * 94;
            eprintln!(
                "{name} at {scale}x: {} of {px} native pixels moved ({:.3}%), 0 unexplained; \
                 worst image {} with {}",
                totals.total,
                100.0 * totals.total as f64 / px as f64,
                worst.0,
                worst.1.total
            );
        }
    }
}
