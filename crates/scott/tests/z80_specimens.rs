//! Re-runs [`scott::decompress_z80`] over the real ZX Spectrum snapshot
//! corpus, when it's present (SQ-1452).
//!
//! # What this checks
//!
//! * `every_spectrum_specimen_decompresses_to_one_image` — every 48K
//!   `.z80` under the fixtures dir decompresses to exactly
//!   [`scott::IMAGE_LEN`] bytes; `blizzard.z80` measures out as hardware
//!   mode 3 (128K, per the v2 table — see `decompress_z80`'s module doc)
//!   and must be named as unsupported rather than silently mis-decoded.
//!
//! * `mysterious_adventures_room_text_is_recoverable_after_decompression`
//!   — the real oracle. For each of the eleven Mysterious Adventures
//!   releases, every room description [`scott::Database::parse`] reads out
//!   of the ScottFree `.dat` conversion (reconstructed with its original
//!   leading `*` literal-marker, since the `.dat` loader strips that into
//!   [`scott::Room::literal`]) is searched for verbatim in the
//!   DECOMPRESSED image. This is NOT the oracle SQ-1452's own brief
//!   proposed — see "A deviation from the brief" below — but it is the one
//!   that actually holds, and it proves the same thing: the decompressor
//!   recovers the real game data, not merely 49,152 bytes of the right
//!   shape.
//!
//! # A deviation from the brief
//!
//! SQ-1452 asked for a different oracle: that the `.dat`'s twelve header
//! ints (NumItems, NumActions, ..., TreasureRoom) would turn up as a
//! contiguous little-endian `u16` run somewhere in the decompressed image,
//! since SQ-1414 had found they appear NOWHERE in the raw compressed file.
//! **Checked directly against `m1goldba.z80`/`1_baton.dat` and it does not
//! hold**: that exact 12-`u16` run is absent from the decompressed image
//! too, in every byte order and width tried (u16 LE/BE, byte sequence,
//! forward and reversed). The likely reason: those twelve numbers are
//! genuinely Scott Adams DATABASE fields in the modern reverse-engineered
//! `.dat` convention, but the original 1980s Spectrum machine-code
//! interpreter most plausibly bakes small config constants (MaxCarry,
//! WordLength, LightTime, ...) into the program as immediate operands
//! rather than as a table an interpreter reads at runtime — there is no
//! reason a compiled machine-code game would keep them as data at all. The
//! ROOM, ITEM and MESSAGE text tables are a different matter: the engine
//! has to read THOSE from data to print them, and the room-description
//! oracle above confirms they are there, byte-for-byte, wherever
//! [`scott::decompress_z80`] recovers them.
//!
//! Both fixture sets are commercial game files and not redistributable, so
//! neither is committed and CI never sees either — this is a vacuous-skip
//! suite exactly like `dialect_specimens.rs`.
//!
//! # Getting the fixtures
//!
//! `.z80` corpus: same as `dialect_specimens.rs` —
//! `SCOTT_DIALECT_FIXTURES/spectrum/` or `stories/scott-dialects/spectrum/`,
//! from `https://ifarchive.org/if-archive/games/spectrum/mystsoft.zip`.
//!
//! `.dat` corpus: point `SCOTT_MYSTERIOUS_DATS` at a directory holding the
//! ScottFree `.dat` conversions of the same eleven Mysterious Adventures
//! releases (file names `1_baton.dat` … `B_waxworks.dat`, matching the IF
//! Archive's `if-archive/scott-adams/games/scottfree/mysterious.tar.gz`, or
//! any other ScottFree-format conversion of the same eleven games). Without
//! it, only the "every specimen decompresses to one image" check runs.

use std::path::PathBuf;

use scott::{decompress_z80, Database, IMAGE_LEN};

fn spectrum_fixtures() -> Option<PathBuf> {
    let candidates = [
        std::env::var_os("SCOTT_DIALECT_FIXTURES").map(PathBuf::from),
        Some(PathBuf::from("stories/scott-dialects")),
        Some(PathBuf::from("../../stories/scott-dialects")),
    ];
    candidates
        .into_iter()
        .flatten()
        .map(|d| d.join("spectrum"))
        .find(|p| p.is_dir())
}

fn dat_fixtures() -> Option<PathBuf> {
    std::env::var_os("SCOTT_MYSTERIOUS_DATS")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

fn read(dir: &std::path::Path, name: &str) -> Option<Vec<u8>> {
    std::fs::read(dir.join(name)).ok()
}

#[test]
fn every_spectrum_specimen_decompresses_to_one_image() {
    let Some(dir) = spectrum_fixtures() else {
        eprintln!(
            "SKIP: no ZX Spectrum specimens — set SCOTT_DIALECT_FIXTURES or \
             populate stories/scott-dialects/spectrum/ (see this suite's \
             module doc). This is a vacuous skip, NOT a pass."
        );
        return;
    };
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("fixtures dir readable")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".z80"))
        .collect();
    names.sort();
    assert_eq!(names.len(), 20, "the documented 20-file Spectrum corpus");

    // The one known 128K specimen in this corpus (hardware mode 3 — see
    // `decompress_z80`'s module doc); everything else here is 48K.
    const NOT_48K: &str = "blizzard.z80";

    for name in &names {
        let bytes = read(&dir, name).unwrap_or_else(|| panic!("{name} readable"));
        match decompress_z80(&bytes) {
            Ok(image) => {
                assert_ne!(
                    name, NOT_48K,
                    "{name}: expected an UnsupportedHardwareMode refusal, not a decode"
                );
                assert_eq!(image.len(), IMAGE_LEN, "{name}: wrong image size");
            }
            Err(scott::Z80Error::UnsupportedHardwareMode(m)) => {
                assert_eq!(name, NOT_48K, "{name}: unexpectedly refused as hardware mode {m}");
            }
            Err(e) => panic!("{name} ({} bytes) failed to decompress: {e}", bytes.len()),
        }
    }
}

/// The eleven Mysterious Adventures releases, pairing each Spectrum `.z80`
/// specimen with its ScottFree `.dat` conversion's file name.
const MYSTERIOUS: &[(&str, &str)] = &[
    ("m1goldba.z80", "1_baton.dat"),
    ("m2tmachi.z80", "2_timemachine.dat"),
    ("m3arrow1.z80", "3_arrow1.dat"),
    ("m4arrow2.z80", "4_arrow2.dat"),
    ("m5pulsar.z80", "5_pulsar7.dat"),
    ("m6circus.z80", "6_circus.dat"),
    ("m7feasib.z80", "7_feasibility.dat"),
    ("m8akyrtz.z80", "8_akyrz.dat"),
    ("m9perseu.z80", "9_perseus.dat"),
    ("m10india.z80", "A_tenlittleindians.dat"),
    ("m11waxwo.z80", "B_waxworks.dat"),
];

#[test]
fn mysterious_adventures_room_text_is_recoverable_after_decompression() {
    let (Some(spec_dir), Some(dat_dir)) = (spectrum_fixtures(), dat_fixtures()) else {
        eprintln!(
            "SKIP: needs both the ZX Spectrum corpus (SCOTT_DIALECT_FIXTURES \
             or stories/scott-dialects/spectrum/) and the Mysterious \
             Adventures .dat corpus (SCOTT_MYSTERIOUS_DATS) — see this \
             suite's module doc. This is a vacuous skip, NOT a pass."
        );
        return;
    };

    let mut total_rooms = 0usize;
    let mut total_matched = 0usize;
    for (z80_name, dat_name) in MYSTERIOUS {
        let z80_bytes = read(&spec_dir, z80_name)
            .unwrap_or_else(|| panic!("{z80_name} present in the Spectrum corpus"));
        let dat_bytes =
            read(&dat_dir, dat_name).unwrap_or_else(|| panic!("{dat_name} present in the .dat corpus"));
        let db = Database::parse(&dat_bytes)
            .unwrap_or_else(|e| panic!("{dat_name} failed to parse as a ScottFree .dat: {e:?}"));
        let image = decompress_z80(&z80_bytes).unwrap_or_else(|e| panic!("{z80_name} failed to decompress: {e}"));

        let mut matched = 0usize;
        let mut first_hit: Option<(usize, String)> = None;
        for room in &db.rooms {
            if room.desc.is_empty() {
                continue;
            }
            // Reconstruct the raw source form: the `.dat` loader strips a
            // leading `*` into `literal`, so put it back before searching —
            // that's the form the original interpreter's own data actually
            // stores (confirmed directly against several of these
            // specimens).
            let needle = if room.literal {
                format!("*{}", room.desc)
            } else {
                room.desc.clone()
            };
            if let Some(at) = image
                .windows(needle.len())
                .position(|w| w == needle.as_bytes())
            {
                matched += 1;
                // Prefer a longer match to report — a stray one- or
                // two-character room description is a true hit but a
                // useless example.
                if first_hit.as_ref().is_none_or(|(_, t)| t.len() < needle.len()) {
                    first_hit = Some((at, needle));
                }
            }
        }
        total_rooms += db.rooms.len();
        total_matched += matched;

        let (offset, text) = first_hit.unwrap_or_else(|| {
            panic!(
                "{z80_name}: NONE of {dat_name}'s {} room descriptions were found in the \
                 decompressed image — decompression is likely wrong for this file",
                db.rooms.len()
            )
        });
        eprintln!(
            "{z80_name}: {matched}/{} of {dat_name}'s room descriptions found; first hit \
             {text:?} at decompressed offset {offset} (0x{offset:04x}, address 0x{:04x})",
            db.rooms.len(),
            offset + 0x4000
        );
    }
    eprintln!("total: {total_matched}/{total_rooms} room descriptions recovered across all 11 releases");
    assert!(
        total_matched * 2 >= total_rooms,
        "expected at least half of all room descriptions to be recoverable; got {total_matched}/{total_rooms}"
    );
}

/// Confirms the negative half of the "deviation from the brief" note above:
/// the `.dat`'s twelve header ints, exactly as SQ-1414 quoted them for
/// Golden Baton, are absent from BOTH the raw file and the decompressed
/// image. If this ever starts passing, the module doc above is wrong and
/// needs rewriting, not this assertion.
#[test]
fn dat_header_ints_do_not_appear_as_a_contiguous_run_even_after_decompression() {
    let (Some(spec_dir), Some(dat_dir)) = (spectrum_fixtures(), dat_fixtures()) else {
        eprintln!(
            "SKIP: needs both fixture sets — see this suite's module doc. \
             This is a vacuous skip, NOT a pass."
        );
        return;
    };
    let z80_bytes =
        read(&spec_dir, "m1goldba.z80").expect("m1goldba.z80 present in the Spectrum corpus");
    let dat_bytes = read(&dat_dir, "1_baton.dat").expect("1_baton.dat present in the .dat corpus");
    let db = Database::parse(&dat_bytes).expect("1_baton.dat parses as a ScottFree .dat");

    let header: [i32; 12] = [
        0,
        db.items.len() as i32 - 1,
        db.actions.len() as i32 - 1,
        db.verbs.len() as i32 - 1,
        db.rooms.len() as i32 - 1,
        db.max_carry,
        db.start_room as i32,
        db.num_treasures,
        db.word_length as i32,
        db.light_time,
        db.messages.len() as i32 - 1,
        db.treasure_room as i32,
    ];
    // SQ-1414 quoted this exact array for Golden Baton; confirm it still
    // matches what `Database::parse` reconstructs before using it.
    assert_eq!(header, [0, 48, 166, 78, 31, 6, 1, 0, 4, 200, 99, 0]);

    let needle: Vec<u8> = header.iter().flat_map(|&v| (v as u16).to_le_bytes()).collect();
    assert!(
        !z80_bytes.windows(needle.len()).any(|w| w == needle),
        "unexpectedly found the header run in the RAW file"
    );

    let image = decompress_z80(&z80_bytes).expect("m1goldba.z80 decompresses");
    assert!(
        !image.windows(needle.len()).any(|w| w == needle),
        "unexpectedly found the header run in the DECOMPRESSED image — the module doc's \
         'deviation from the brief' note is now wrong and needs rewriting"
    );
}
