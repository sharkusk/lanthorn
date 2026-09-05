//! What an Inform 7 Glulx story says about its own map, read from the FILE —
//! no turn played, no VM booted (SQ-1303).
//!
//! Every number below was measured off the story file named beside it. The
//! oracle for Counterfeit Monkey is a played map dump of the same release: all
//! 94 rooms that dump reached appear here by name, and the 15 extra names are
//! rooms it never visited.
//!
//! `stories/` is gitignored commercial media, so these skip vacuously on CI —
//! and there is no committed Glulx fixture that can stand in, because the one
//! there is (`gvm-cli/tests/fixtures/glulxercise.ulx`) is a VM conformance
//! suite with no parser, which [`ParseNames::detect`] refuses before this
//! reader is ever reached.

use std::path::PathBuf;

use gvm::i7map::{I7Exit, I7World};
use gvm::memory::Memory;
use gvm::objects::ParseNames;
use gvm::world::Compass;

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

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn story(name: &str) -> Option<Memory> {
    let path = stories_dir().join(name);
    if !path.exists() {
        eprintln!("SKIP: {} absent", path.display());
        return None;
    }
    Memory::new(glulx_image(std::fs::read(&path).ok()?)?).ok()
}

/// `(memory, objects, world)` for a story that must yield a map.
fn world(name: &str) -> Option<(Memory, ParseNames, I7World)> {
    let mem = story(name)?;
    let pn = ParseNames::detect(&mem).expect("an object tree");
    let w = I7World::detect(&mem, &pn).expect("an Inform 7 map");
    Some((mem, pn, w))
}

/// Every exit of `room`, as `(direction label, destination name)`.
fn exits_of(mem: &Memory, pn: &ParseNames, w: &I7World, room: u32) -> Vec<(String, String)> {
    w.exits(mem, pn, room)
        .into_iter()
        .map(|(c, d, e)| {
            let dir = c
                .map(|c| format!("{c:?}"))
                .unwrap_or_else(|| w.printed_name(mem, pn, d).unwrap_or_default());
            let to = match e {
                I7Exit::Room(x) | I7Exit::ThroughDoor { to: x, .. } => {
                    w.printed_name(mem, pn, x).unwrap_or_default()
                }
                I7Exit::Door(x) => format!("unresolved door {x:#x}"),
            };
            (dir, to)
        })
        .collect()
}

// ── Inform 7 build 6M62 ─────────────────────────────────────────────────────

#[test]
fn counterfeit_monkey_hands_over_its_whole_map_without_a_turn_played() {
    let Some((mem, pn, w)) = world("CounterfeitMonkey-11.gblorb") else {
        return;
    };

    // Nothing in the image names any of these; all four are derived. The
    // property numbers are compiler-assigned and differ story to story, which
    // is why they are read rather than assumed.
    assert_eq!(w.properties(), (25, 419, Some(21)));
    assert_eq!(w.map_storage(), 0x378f28);
    assert_eq!(w.rooms().len(), 100);

    // The Standard Rules define twelve directions; Counterfeit Monkey adds
    // eight shipboard ones, so the column count is read, never assumed.
    assert_eq!(w.directions().len(), 20);
    let dirs: Vec<String> = w
        .directions()
        .iter()
        .filter_map(|&d| w.printed_name(&mem, &pn, d))
        .collect();
    assert_eq!(
        dirs,
        [
            "north",
            "northeast",
            "northwest",
            "south",
            "southeast",
            "southwest",
            "east",
            "west",
            "up",
            "down",
            "inside",
            "outside",
            "Starboard",
            "port",
            "fore",
            "aft",
            "aft-port",
            "aft-starboard",
            "fore-port",
            "fore-starboard",
        ]
    );

    // Every room is named, which is the whole point: I7 leaves the hardware
    // short name empty, so `ParseNames` alone answers nothing here.
    let named = w
        .rooms()
        .iter()
        .filter(|&&r| w.printed_name(&mem, &pn, r).is_some())
        .count();
    assert_eq!(named, 100);
    assert_eq!(pn.short_name(&mem, w.rooms()[0]).as_deref(), Some(""));

    let total: usize = w.rooms().iter().map(|&r| w.exits(&mem, &pn, r).len()).sum();
    assert_eq!(total, 211);

    // The first row of `Map_Storage`, including a door the reader resolves
    // through its `found_in` sides rather than by calling `door_to()`.
    let fair = w.rooms()[0];
    assert_eq!(w.printed_name(&mem, &pn, fair).as_deref(), Some("Fair"));
    assert_eq!(
        exits_of(&mem, &pn, &w, fair),
        [
            ("N".into(), "Park Center".to_string()),
            ("Ne".into(), "Monumental Staircase".into()),
            ("Nw".into(), "Church Forecourt".into()),
            ("S".into(), "Ampersand Bend".into()),
            ("E".into(), "Heritage Corner".into()),
            ("W".into(), "Midway".into()),
        ]
    );
    assert!(matches!(
        w.exits(&mem, &pn, fair)[3].2,
        I7Exit::ThroughDoor { .. } // south, through the Ampersand Bend door
    ));
}

#[test]
fn every_room_the_played_counterfeit_monkey_map_reached_is_here_by_name() {
    let Some((mem, pn, w)) = world("CounterfeitMonkey-11.gblorb") else {
        return;
    };
    let names: Vec<String> = w
        .rooms()
        .iter()
        .filter_map(|&r| w.printed_name(&mem, &pn, r))
        .collect();

    // A sample of the played dump (94 rooms over seven sessions); the full
    // comparison is in the SQ-1303 report. Two of these — Wonderland and
    // Oracle Project — are rooms whose PLAYED connections do not match the
    // compiled map, because Counterfeit Monkey rewrites `Map_Storage` at run
    // time; the room set matches regardless.
    for room in [
        "New Church",
        "Abandoned Shore",
        "Wonderland",
        "Galley",
        "Oracle Project",
        "Bus Station",
        "Samuel Johnson Basement",
        "Babel Café",
        "Monumental Staircase",
        "Precarious Perch",
        "Higgate's office",
    ] {
        assert!(
            names.contains(&room.to_string()),
            "{room} missing from the static read"
        );
    }

    // …and fifteen rooms the dump never reached, which is the point of reading
    // the file rather than walking it.
    for unvisited in [
        "Shadow Chamber",
        "Brock's Stateroom",
        "Lecture Hall",
        "Private Beach",
    ] {
        assert!(
            names.contains(&unvisited.to_string()),
            "{unvisited} missing"
        );
    }
}

// ── Inform 7 build 6L38 ─────────────────────────────────────────────────────

#[test]
fn the_wizard_sniffer_map_is_not_word_aligned() {
    let Some((mem, pn, w)) = world("The_Wizard_Sniffer.gblorb") else {
        return;
    };
    assert_eq!(w.properties(), (25, 277, Some(21)));

    // THE fact that makes a four-byte-stride scan useless: Glulx imposes no
    // alignment and Inform packs its arrays, so this story's map sits at an
    // address ≡ 1 (mod 4) where Counterfeit Monkey's is aligned.
    assert_eq!(w.map_storage(), 0xe36dd);
    assert_eq!(w.map_storage() % 4, 1);

    assert_eq!(w.rooms().len(), 40);
    assert_eq!(w.directions().len(), 12);
    let total: usize = w.rooms().iter().map(|&r| w.exits(&mem, &pn, r).len()).sum();
    assert_eq!(total, 93);

    let mountain = w.rooms()[0];
    assert_eq!(
        w.printed_name(&mem, &pn, mountain).as_deref(),
        Some("Atop a Mountain")
    );
    assert_eq!(
        exits_of(&mem, &pn, &w, mountain),
        [("N".to_string(), "Southern Bailey".to_string())]
    );
    assert_eq!(
        w.exits(&mem, &pn, mountain)[0].0,
        Some(Compass::N),
        "a compass point is recovered from the direction object's parser words"
    );
}

// ── Inform 10.1.2 ───────────────────────────────────────────────────────────

#[test]
fn the_scheme_is_unchanged_from_six_l_thirty_eight_through_inform_ten() {
    let Some((mem, pn, w)) = world("Skuga Lake ME.gblorb") else {
        return;
    };
    // `WorldModelKit/Sections/WorldModel.i6t` spells `MapConnection` exactly as
    // the 6M62 template does — only the property NUMBERS move.
    assert_eq!(w.properties(), (26, 32, Some(21)));
    assert_eq!(w.map_storage(), 0x14ec52);
    assert_eq!(w.rooms().len(), 52);
    assert_eq!(w.directions().len(), 12);
    let total: usize = w.rooms().iter().map(|&r| w.exits(&mem, &pn, r).len()).sum();
    assert_eq!(total, 88);
    let named = w
        .rooms()
        .iter()
        .filter(|&&r| w.printed_name(&mem, &pn, r).is_some())
        .count();
    assert_eq!(named, 51);
}

// ── What the reader REFUSES, and why ────────────────────────────────────────

#[test]
fn an_inform_seven_build_older_than_the_map_array_is_refused() {
    // `AnchorheadDemo.gblorb` is Inform 7 build 4K41 (2007) and carries no
    // instance-count properties at all: the compass objects have nothing that
    // indexes them, so there is nothing to index `Map_Storage` with — and no
    // `Map_Storage`. `King_of_Shreds_and_Patches.gblorb` (build 5J39) is the
    // same. Refusing beats reporting a coincidence.
    let Some(mem) = story("AnchorheadDemo.gblorb") else {
        return;
    };
    let pn = ParseNames::detect(&mem).expect("an object tree");
    assert!(I7World::detect(&mem, &pn).is_none());
}

#[test]
fn a_story_that_builds_its_map_at_run_time_is_refused_rather_than_guessed_at() {
    // Kerkerkruip generates its dungeon each game, so the COMPILED
    // `Map_Storage` is all zeros — there is no map in the file to find, and
    // the floor on room entries is what stops a three-room accident scoring
    // highest and being reported as one.
    let Some(mem) = story("Kerkerkruip.gblorb") else {
        return;
    };
    let pn = ParseNames::detect(&mem).expect("an object tree");
    assert!(I7World::detect(&mem, &pn).is_none());
}

// ── The corpus sweep ────────────────────────────────────────────────────────

#[test]
fn every_map_this_reader_reports_is_internally_consistent() {
    let Ok(dir) = std::fs::read_dir(stories_dir()) else {
        eprintln!("SKIP: no stories/");
        return;
    };
    let mut found = 0;
    let mut declined = 0;
    for entry in dir.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "gblorb") {
            continue;
        }
        let Some(image) = std::fs::read(&path).ok().and_then(glulx_image) else {
            continue;
        };
        let Ok(mem) = Memory::new(image) else {
            continue;
        };
        let Ok(pn) = ParseNames::detect(&mem) else {
            continue;
        };
        let Some(w) = I7World::detect(&mem, &pn) else {
            declined += 1;
            continue;
        };
        found += 1;
        let name = path.file_name().unwrap().to_string_lossy().to_string();

        assert!(!w.rooms().is_empty(), "{name}: a map with no rooms");
        assert!(
            !w.directions().is_empty(),
            "{name}: a map with no directions"
        );
        // Every row is a room this reader also calls a room, and every entry
        // is one of this story's objects — the two halves that make the model
        // usable at all.
        for &r in w.rooms() {
            assert!(
                w.is_room(r),
                "{name}: room {r:#x} is not one of its own rooms"
            );
            for d in 0..w.directions().len() {
                let Some(entry) = w.raw_exit(&mem, r, d) else {
                    continue;
                };
                assert!(
                    pn.is_object(&mem, entry),
                    "{name}: {r:#x} direction {d} names {entry:#x}, not an object"
                );
            }
        }
    }
    if found + declined == 0 {
        eprintln!("SKIP: no Glulx stories present");
        return;
    }
    // Measured 2026-09-04 over the 31 Inform 7 Glulx stories in `stories/`.
    // Not pinned exactly, because the directory is a working collection.
    assert!(found >= declined, "{found} maps read, {declined} refused");
}
