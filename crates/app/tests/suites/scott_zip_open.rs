//! SQ-1460: does an MS-DOS Scott Adams release open straight from its zip?
//!
//! The app already opens zips by CONTENT (SQ-1085/SQ-1414): `hints::load_story`
//! is the function a command-line path or a picker launch both resolve through,
//! and it classifies every zip entry with `hints::extract_story` rather than by
//! extension, so a Scott `.dat` is recognised the same way loose or zipped.
//! `stories/scott-dialects/msdos/The-Hulk_DOS_EN.zip` is the real-world stress
//! case: 71 entries, dozens of `.PAK` picture/sound files and a `START.EXE`
//! sitting AHEAD of `ADVENT.DAT` in archive order, so the scan has to walk past
//! all of them without one being misclassified as a story before it reaches the
//! one entry that actually is.
//!
//! Two halves: the real fixture (`stories/` only, gitignored — skips vacuously)
//! pins the actual release; a hand-built zip pins the SELECTION RULE itself
//! without needing the commercial file, using the freely-redistributable
//! `tiny_cave.dat` already checked into `crates/scott/tests/`.

use std::io::Write as _;
use std::path::{Path, PathBuf};

fn scratch(tag: &str) -> PathBuf {
    app::scratch_dir(tag)
}

fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
    let file = std::fs::File::create(path).expect("a scratch zip");
    let mut zw = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in entries {
        zw.start_file(*name, opts).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap();
}

/// Extract one named entry from a zip with the same `zip` crate the app
/// depends on, independently of `hints`'s own extraction — the reference point
/// each case checks the loader's answer against.
fn extract_entry(zip_path: &Path, entry: &str) -> Vec<u8> {
    let file = std::fs::File::open(zip_path).expect("open the zip");
    let mut zip = zip::ZipArchive::new(file).expect("valid zip");
    let mut e = zip.by_name(entry).unwrap_or_else(|_| panic!("{entry} present in {zip_path:?}"));
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut e, &mut out).unwrap();
    out
}

/// What `scott-cli` would print as the first room description for `bytes`: the
/// exact construction `scott-cli/src/main.rs` and `ScottSession::new_with_options`
/// both use — `Database::parse` into `Vm::new_full` with the default seed and
/// default `Options` — followed by `Vm::room_block()`, which is the "top window"
/// text scott-cli prints for the opening room (main.rs's loop, first pass through
/// `let block = vm.room_block()`).
fn first_room_block(bytes: &[u8]) -> String {
    let db = scott::Database::parse(bytes).expect("a valid Scott database");
    let vm = scott::Vm::new_full(db, false, scott::Vm::DEFAULT_RNG_SEED, scott::Options::default());
    vm.room_block()
}

// ── Real fixture: The Hulk (MS-DOS) ─────────────────────────────────────────

fn hulk_zip() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories/scott-dialects/msdos/The-Hulk_DOS_EN.zip")
}

/// Non-vacuity guard (CLAUDE.md "A frame is a fixture"): a skip here must not
/// read like a pass. Pinned to the archive's own entry count so a truncated or
/// substituted fixture is caught too.
#[test]
fn hulk_zip_fixture_is_the_71_entry_archive_when_present() {
    let zip = hulk_zip();
    let Ok(names) = app::hints::zip_entry_names(&zip) else {
        eprintln!("SKIP: stories/scott-dialects/msdos/The-Hulk_DOS_EN.zip absent (gitignored commercial fixture)");
        return;
    };
    assert_eq!(names.len(), 71, "The-Hulk_DOS_EN.zip's own entry count changed: {names:?}");
    assert!(names.iter().any(|n| n == "ADVENT.DAT"), "no ADVENT.DAT in {names:?}");
    assert!(names.iter().any(|n| n == "START.EXE"), "no START.EXE in {names:?}");
}

/// The defect under test: does the zip open straight through as a Scott game,
/// picking `ADVENT.DAT` out of 70 non-story siblings (dozens of `.PAK`s, a
/// `START.EXE`, a `HULK.BAT`) rather than stopping at the wrong entry or
/// refusing the archive outright?
#[test]
fn hulk_zip_opens_as_scott_with_advent_dat_not_start_exe_or_a_pak() {
    let zip = hulk_zip();
    if app::hints::zip_entry_names(&zip).is_err() {
        eprintln!("SKIP: stories/scott-dialects/msdos/The-Hulk_DOS_EN.zip absent (gitignored commercial fixture)");
        return;
    }

    let advent_bytes = extract_entry(&zip, "ADVENT.DAT");

    // `zipped_stories` is what the picker lists — one row per entry the loader
    // recognises as a story. A single-story archive must be exactly one row,
    // and it must be the one named ADVENT.DAT: any of the 70 siblings coming
    // back too would mean a `.PAK` or `START.EXE` was misclassified.
    let stories = app::hints::zipped_stories(&zip).expect("at least one story in the archive");
    assert_eq!(
        stories.len(),
        1,
        "expected exactly one recognised story (ADVENT.DAT); the loader also \
         picked up: {:?}",
        stories.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
    assert_eq!(stories[0].0, "ADVENT.DAT");
    assert_eq!(stories[0].1, advent_bytes, "the entry the loader read differs from ADVENT.DAT's own bytes");

    // `hints::load_story` is the SAME function a bare command-line path or a
    // picker launch resolves a story through (see `story_identity_sweep.rs`,
    // which sweeps the whole corpus through it) — so this is opening the zip
    // exactly the way `lanthorn stories/.../The-Hulk_DOS_EN.zip` would.
    let loaded = app::hints::load_story(&zip).expect("the zip opens as a story");
    let app::hints::LoadedStory::Scott(zip_bytes) = loaded else {
        panic!("The-Hulk_DOS_EN.zip resolved to a non-Scott engine: {loaded:?}");
    };
    assert_eq!(zip_bytes, advent_bytes, "load_story's bytes differ from ADVENT.DAT extracted independently");

    // Boot it — both through the app's real Scott adapter (proves the engine
    // accepts exactly what the zip path handed it) and through the bare `scott`
    // VM construction `scott-cli` uses (the room-description oracle below).
    app::scott_session::ScottSession::new(zip_bytes.clone(), None)
        .expect("ScottSession accepts the bytes load_story resolved from the zip");

    // The oracle: `scott-cli`'s first room description is `Vm::room_block()`
    // off `Database::parse`+`Vm::new_full` with the default seed — the exact
    // construction reproduced in `first_room_block`. Extracted independently
    // to a scratch dir, then re-read from disk, so this is genuinely "what
    // scott-cli would print for the extracted ADVENT.DAT" and not merely a
    // second in-memory copy of the same bytes.
    let scratch_dir = scratch("sq1460");
    let extracted_path = scratch_dir.join("ADVENT.DAT");
    std::fs::write(&extracted_path, &advent_bytes).unwrap();
    let from_disk = std::fs::read(&extracted_path).unwrap();

    let via_zip = first_room_block(&zip_bytes);
    let via_extracted_file = first_room_block(&from_disk);
    assert_eq!(via_zip, via_extracted_file, "the zip-sourced game opens on a different room than the extracted .dat");
    assert!(!via_zip.trim().is_empty(), "the opening room description must not be blank");

    let _ = std::fs::remove_dir_all(&scratch_dir);
}

// ── Hand-built zip: pin the selection rule without the commercial fixture ──

/// `tiny_cave.dat` — the same freely-redistributable fixture `scott_mapper.rs`
/// drives — packed behind a decoy `README.TXT` and a decoy `.PAK` that must NOT
/// be mistaken for a story: this is the case that runs on CI, where `stories/`
/// does not exist, so it is the one that actually falsifies a regression.
fn tiny_cave() -> Vec<u8> {
    include_bytes!("../../../scott/tests/tiny_cave.dat").to_vec()
}

/// Non-story binary padding — not a Blorb (`FORM` magic), not Glulx (`Glul`
/// magic), not valid UTF-8 (so `looks_like_scott_bytes`'s text path rejects it
/// immediately) and not a TI-99/4A tokenised image, and its first byte (0) is
/// outside the Z-machine's `3..=8` version range.
fn decoy_pak() -> Vec<u8> {
    (0u32..300).map(|i| ((i * 37) % 256) as u8).collect()
}

#[test]
fn hand_built_zip_selects_the_scott_dat_ahead_of_decoys() {
    let dir = scratch("sq1460-synthetic");
    let zip_path = dir.join("compilation.zip");
    // Decoys first, in archive order, mirroring The Hulk's real layout where
    // dozens of `.PAK`s and `START.EXE` precede `ADVENT.DAT`.
    write_zip(
        &zip_path,
        &[
            ("R0001.PAK", &decoy_pak()),
            ("B0100I.PAK", &decoy_pak()),
            ("README.TXT", b"Tiny Cave -- a freely redistributable Scott Adams test fixture.\n"),
            ("tiny_cave.dat", &tiny_cave()),
            ("R0002.PAK", &decoy_pak()),
        ],
    );

    let stories = app::hints::zipped_stories(&zip_path).expect("tiny_cave.dat is a recognised story");
    assert_eq!(
        stories.len(),
        1,
        "a decoy .PAK or the README was misclassified as a story: {:?}",
        stories.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
    assert_eq!(stories[0].0, "tiny_cave.dat");
    assert_eq!(stories[0].1, tiny_cave());

    let loaded = app::hints::load_story(&zip_path).expect("the archive opens as a story");
    match loaded {
        app::hints::LoadedStory::Scott(bytes) => assert_eq!(bytes, tiny_cave()),
        other => panic!("expected Scott, got {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// The falsifying half of the case above: swap the order so `tiny_cave.dat` is
/// the LAST entry, after every decoy — proves the selection is by content, not
/// by "the tests happen to have put it first."
#[test]
fn hand_built_zip_selects_the_scott_dat_when_it_is_the_last_entry() {
    let dir = scratch("sq1460-synthetic-tail");
    let zip_path = dir.join("compilation-tail.zip");
    write_zip(
        &zip_path,
        &[
            ("R0001.PAK", &decoy_pak()),
            ("README.TXT", b"decoy\n"),
            ("B0100I.PAK", &decoy_pak()),
            ("R0002.PAK", &decoy_pak()),
            ("tiny_cave.dat", &tiny_cave()),
        ],
    );

    let loaded = app::hints::load_story(&zip_path).expect("the archive opens as a story");
    match loaded {
        app::hints::LoadedStory::Scott(bytes) => assert_eq!(bytes, tiny_cave()),
        other => panic!("expected Scott, got {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&dir);
}
