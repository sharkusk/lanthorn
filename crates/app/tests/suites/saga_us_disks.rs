//! SQ-1470: the US S.A.G.A. release disks — Atari 8-bit `.atr`, Apple II DOS
//! 3.3 `.dsk`, and the Commodore 64 `SHULK.DB` Questprobe release — open
//! through the one disk seam every front-end shares, the same way the
//! Commodore 64 *Mysterious Adventures* compilations do (`c64_mysterious_disks.rs`).
//!
//! `scott::saga_us` (the loader over raw container bytes) and `blorb`'s
//! ATR/DOS 3.3/D64 readers landed separately; this pins the app-side wiring
//! that joins them: `app::hints::mounted_stories`'s scan over
//! `MountedDisk::contents()` extended with the Atari whole-image door
//! (`blorb::atr::IMAGE_ENTRY` — the database is not a directory entry on that
//! platform), each candidate content-identified by
//! `scott::SagaUs::display_title` and keyed for save purposes on that same
//! identity rather than on a container entry name three different releases
//! can share (`IMAGE` on Atari, `DATABASE` on two different Apple II
//! titles).
//!
//! `stories/` is gitignored (commercial media), so every case skips
//! vacuously when its fixture is absent.

use std::path::PathBuf;

fn atari_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/scott-dialects/atari")
}

fn apple_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/scott-dialects/apple")
}

fn c64_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/scott-dialects/c64")
}

fn data_base(tag: &str) -> PathBuf {
    app::scratch_dir(&format!("sq1470-{tag}"))
}

/// One Atari side A specimen and the title `mounted_stories` must resolve for
/// it (`scott::SagaUs::display_title`, §12.12's per-release table).
const ATARI_SIDE_A: &[(&str, &str)] = &[
    ("SAGA #1 - Adventureland [side A].atr", "Adventureland (Atari 8-bit)"),
    ("SAGA #2 - Pirate Adventure [side A].atr", "Pirate Adventure (Atari 8-bit)"),
    ("SAGA #4 - Voodoo Castle [side A].atr", "Voodoo Castle (Atari 8-bit)"),
    ("SAGA #5 - The Count [side A].atr", "The Count (Atari 8-bit)"),
    ("SAGA #6 - Strange Odyssey [side A].atr", "Strange Odyssey (Atari 8-bit)"),
    (
        "SAGA No. 13 - The Sorcerer of Claymorgue Castle _ side A.atr",
        "The Sorcerer of Claymorgue Castle (Atari 8-bit)",
    ),
];

/// Every Atari side A but the damaged one yields exactly one Scott row, and
/// the row's own content-identified title is the release `display_title`
/// gives — never the container's bare `IMAGE` entry.
#[test]
fn atari_side_a_specimens_yield_one_named_row_each() {
    let dir = atari_dir();
    let mut ran = 0;
    for (file, want_title) in ATARI_SIDE_A {
        let path = dir.join(file);
        if !path.is_file() {
            eprintln!("SKIP: stories/scott-dialects/atari/{file} absent (gitignored commercial fixture)");
            continue;
        }
        ran += 1;
        let base = data_base(&format!("atari-{file}"));
        let rows = app::picker::resolve_entries(&path, &base);
        assert_eq!(rows.len(), 1, "{file}: must offer exactly one row, got {rows:?}");
        assert_eq!(rows[0].meta.engine, app::picker::Engine::Scott, "{file}: not classified Scott");
        assert_eq!(&rows[0].title, want_title, "{file}: wrong content-identified title");
        let _ = std::fs::remove_dir_all(&base);
    }
    if ran == 0 {
        eprintln!("SKIP: no Atari 8-bit S.A.G.A. specimens present");
    }
}

/// Mission Impossible's Atari side A is a damaged specimen — its pointer
/// tables do not resolve (`scott::saga_us::parse_saga_us`'s own doc: "the two
/// item-location tables disagree") — so it must yield zero Scott rows and no
/// panic, never a fabricated row for an image `looks_like_scott_bytes`
/// merely sniffed as plausible.
#[test]
fn mission_impossible_atari_side_a_yields_no_rows_and_no_panic() {
    let path = atari_dir().join("SAGA #3 - Mission Impossible [side A].atr");
    if !path.is_file() {
        eprintln!("SKIP: stories/scott-dialects/atari/SAGA #3 - Mission Impossible [side A].atr absent (gitignored commercial fixture)");
        return;
    }
    // `mounted_stories` must not panic scanning the damaged image, and must
    // not offer it as a Scott row.
    if let Some((_, stories)) = app::hints::mounted_stories(&path) {
        assert!(stories.is_empty(), "a damaged database must not be offered: {stories:?}");
    }
    let base = data_base("mission-impossible-damaged");
    let rows = app::picker::resolve_entries(&path, &base);
    assert!(rows.is_empty(), "a damaged database must not be offered: {rows:?}");
    let _ = std::fs::remove_dir_all(&base);
}

/// Atari side B is the companion PICTURE disk (spec §7.3) and carries no
/// database at all — no rows, no panic.
#[test]
fn atari_side_b_picture_disks_yield_no_scott_rows() {
    let path = atari_dir().join("SAGA #4 - Voodoo Castle [side B].atr");
    if !path.is_file() {
        eprintln!("SKIP: stories/scott-dialects/atari/SAGA #4 - Voodoo Castle [side B].atr absent (gitignored commercial fixture)");
        return;
    }
    assert!(
        app::hints::mounted_stories(&path).is_none(),
        "a picture-only side must offer no story rows at all"
    );
}

/// One Apple II boot side specimen and the title it must resolve to — the
/// three whose database is named `A?.DAT` and the two (scrambled, §8.4) that
/// share the literal name `DATABASE`.
const APPLE_BOOT_SIDES: &[(&str, &str)] = &[
    (
        "Scott Adams Graphic Adventure 1 - Adventureland v2.1-416 (4am crack) side B - boot.dsk",
        "Adventureland (Apple II)",
    ),
    (
        "Scott Adams Graphic Adventure 2 - Pirate Adventure v2.1-408 (4am crack) side B - boot.dsk",
        "Pirate Adventure (Apple II)",
    ),
    (
        "Scott Adams Graphic Adventure 4 - Voodoo Castle v2.1-119 (4am crack) side B (boot).dsk",
        "Voodoo Castle (Apple II)",
    ),
    (
        "Scott Adams Graphic Adventure 5 - The Count v2.1-115 (4am crack) side B - boot.dsk",
        "The Count (Apple II)",
    ),
    (
        "Scott Adams Graphic Adventure 6 - Strange Odyssey v2.1-119 (4am crack) side B - boot.dsk",
        "Strange Odyssey (Apple II)",
    ),
    (
        "Scott Adams Graphic Adventure 13 - The Sorcerer of Claymorgue Castle v2.2-122 (4am crack) side B (boot).dsk",
        "The Sorcerer of Claymorgue Castle (Apple II)",
    ),
];

/// Every Apple II boot side yields exactly one Scott row, content-identified
/// — including *The Count* and *Claymorgue Castle*, whose database files are
/// BOTH named `DATABASE` and must not be confused with each other.
#[test]
fn apple_ii_boot_sides_yield_one_named_row_each() {
    let dir = apple_dir();
    let mut ran = 0;
    for (file, want_title) in APPLE_BOOT_SIDES {
        let path = dir.join(file);
        if !path.is_file() {
            eprintln!("SKIP: stories/scott-dialects/apple/{file} absent (gitignored commercial fixture)");
            continue;
        }
        ran += 1;
        let base = data_base(&format!("apple-{file}"));
        let rows = app::picker::resolve_entries(&path, &base);
        assert_eq!(rows.len(), 1, "{file}: must offer exactly one row, got {rows:?}");
        assert_eq!(rows[0].meta.engine, app::picker::Engine::Scott, "{file}: not classified Scott");
        assert_eq!(&rows[0].title, want_title, "{file}: wrong content-identified title");
        let _ = std::fs::remove_dir_all(&base);
    }
    if ran == 0 {
        eprintln!("SKIP: no Apple II S.A.G.A. specimens present");
    }
}

/// A PLAIN title's Apple II side A is an ordinary DOS 3.3 volume of artwork
/// (spec §7.4/§8.4) — no database, so no Scott rows.
#[test]
fn apple_ii_plain_picture_side_yields_no_scott_rows() {
    let path = apple_dir()
        .join("Scott Adams Graphic Adventure 1 - Adventureland v2.1-416 (4am crack) side A.dsk");
    if !path.is_file() {
        eprintln!("SKIP: Apple II Adventureland side A absent (gitignored commercial fixture)");
        return;
    }
    assert!(
        app::hints::mounted_stories(&path).is_none(),
        "a picture-only side must offer no story rows at all"
    );
}

/// A SCRAMBLED title's Apple II side A is not a DOS 3.3 disk at all (spec
/// §7.4: the picture files sit on the boot disk instead, and side A holds
/// something else) — `DiskImage::detect` must refuse it outright rather than
/// panic or fabricate a row.
#[test]
fn apple_ii_scrambled_side_a_is_not_a_disk_image_and_yields_no_rows() {
    let path = apple_dir()
        .join("Scott Adams Graphic Adventure 4 - Voodoo Castle v2.1-119 (4am crack) side A.dsk");
    if !path.is_file() {
        eprintln!("SKIP: Apple II Voodoo Castle side A absent (gitignored commercial fixture)");
        return;
    }
    assert!(
        app::hints::mounted_stories(&path).is_none(),
        "a scrambled title's side A is not a readable DOS 3.3 disk at all"
    );
}

/// `QUESTPR1.D64`'s `SHULK.DB` is the US S.A.G.A. *Hulk* (a DIFFERENT
/// Commodore 64 family from the *Mysterious Adventures* disks) and, since the
/// loader landed, is recognised and content-identified.
#[test]
fn questpr1_yields_the_hulk_row() {
    let path = c64_dir().join("QUESTPR1.D64");
    if !path.is_file() {
        eprintln!("SKIP: stories/scott-dialects/c64/QUESTPR1.D64 absent (gitignored commercial fixture)");
        return;
    }
    let base = data_base("questpr1");
    let rows = app::picker::resolve_entries(&path, &base);
    assert_eq!(rows.len(), 1, "QUESTPR1.D64 must offer exactly one row: {rows:?}");
    assert_eq!(rows[0].meta.engine, app::picker::Engine::Scott);
    assert_eq!(rows[0].title, "The Hulk (Commodore 64)");
    let _ = std::fs::remove_dir_all(&base);
}

/// The exact same build 119 of adventure 4 is pressed for the Atari 8-bit and
/// the Apple II (§12.12's per-release table: identical version, identical
/// header counts) — `load_mounted_story_from` must boot both to the SAME
/// first room, and that room must be exact against `stories/adv04.dat`
/// booted directly, which is the ScottFree reference conversion of the same
/// release.
#[test]
fn voodoo_castle_boots_identically_off_atari_and_apple_ii_and_matches_the_reference_dat() {
    let atari = atari_dir().join("SAGA #4 - Voodoo Castle [side A].atr");
    let apple = apple_dir()
        .join("Scott Adams Graphic Adventure 4 - Voodoo Castle v2.1-119 (4am crack) side B (boot).dsk");
    let reference = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/adv04.dat");
    if !atari.is_file() || !apple.is_file() {
        eprintln!("SKIP: Atari and/or Apple II Voodoo Castle specimen absent (gitignored commercial fixture)");
        return;
    }
    if !reference.is_file() {
        eprintln!("SKIP: stories/adv04.dat (the ScottFree reference conversion) absent");
        return;
    }

    let room_text = |bytes: &[u8]| -> String {
        let db = scott::Database::parse(bytes).expect("a valid Scott database");
        let vm =
            scott::Vm::new_full(db, false, scott::Vm::DEFAULT_RNG_SEED, scott::Options::default());
        vm.room_block()
    };

    let (atari_loaded, _) = app::hints::load_mounted_story_from(&atari, None).expect("Atari side opens");
    let app::hints::LoadedStory::Scott(atari_bytes) = atari_loaded else {
        panic!("Atari Voodoo Castle resolved to a non-Scott engine");
    };
    let (apple_loaded, _) =
        app::hints::load_mounted_story_from(&apple, None).expect("Apple II side opens");
    let app::hints::LoadedStory::Scott(apple_bytes) = apple_loaded else {
        panic!("Apple II Voodoo Castle resolved to a non-Scott engine");
    };
    let reference_bytes = std::fs::read(&reference).expect("adv04.dat reads");

    let atari_room = room_text(&atari_bytes);
    let apple_room = room_text(&apple_bytes);
    let reference_room = room_text(&reference_bytes);

    assert!(atari_room.contains("I'm in a chapel"), "Atari opening room: {atari_room}");
    assert_eq!(atari_room, apple_room, "the Atari and Apple II releases must boot to the SAME room");
    assert_eq!(atari_room, reference_room, "must be exact against the reference adv04.dat conversion");
}

/// The same title pressed on two different platforms must not share a save
/// key: `story_key_for` keys a disk-sourced Scott entry on its NAME alone
/// (Scott bytes carry no Z-machine header to build a `DiskBuild` from), and
/// the release build 119/4 is identical on Atari and Apple II, so only the
/// platform distinguishes the two saves.
#[test]
fn voodoo_castle_atari_and_apple_ii_have_distinct_save_keys() {
    let atari = atari_dir().join("SAGA #4 - Voodoo Castle [side A].atr");
    let apple = apple_dir()
        .join("Scott Adams Graphic Adventure 4 - Voodoo Castle v2.1-119 (4am crack) side B (boot).dsk");
    if !atari.is_file() || !apple.is_file() {
        eprintln!("SKIP: Atari and/or Apple II Voodoo Castle specimen absent (gitignored commercial fixture)");
        return;
    }
    let base = data_base("voodoo-keys");
    let atari_rows = app::picker::resolve_entries(&atari, &base);
    let apple_rows = app::picker::resolve_entries(&apple, &base);
    assert_eq!(atari_rows.len(), 1);
    assert_eq!(apple_rows.len(), 1);
    assert_ne!(
        atari_rows[0].story_key(),
        apple_rows[0].story_key(),
        "the same release build on two platforms must not share a save key"
    );
    assert_ne!(atari_rows[0].game_dir(&base), apple_rows[0].game_dir(&base));
    let _ = std::fs::remove_dir_all(&base);
}

/// Two DIFFERENT Apple II titles — *The Count* and *Claymorgue Castle* — both
/// name their database file `DATABASE` (spec §7.4), and both boot disks live
/// in this very directory, so a save key built from the container entry name
/// alone would collide.
#[test]
fn the_count_and_claymorgue_castle_do_not_share_a_save_key() {
    let count = apple_dir()
        .join("Scott Adams Graphic Adventure 5 - The Count v2.1-115 (4am crack) side B - boot.dsk");
    let claymorgue = apple_dir().join(
        "Scott Adams Graphic Adventure 13 - The Sorcerer of Claymorgue Castle v2.2-122 (4am crack) side B (boot).dsk",
    );
    if !count.is_file() || !claymorgue.is_file() {
        eprintln!("SKIP: Apple II The Count and/or Claymorgue Castle specimen absent (gitignored commercial fixture)");
        return;
    }
    let base = data_base("database-collision");
    let count_rows = app::picker::resolve_entries(&count, &base);
    let claymorgue_rows = app::picker::resolve_entries(&claymorgue, &base);
    assert_eq!(count_rows.len(), 1);
    assert_eq!(claymorgue_rows.len(), 1);
    assert_ne!(
        count_rows[0].story_key(),
        claymorgue_rows[0].story_key(),
        "two different games named DATABASE must not share a save key"
    );
    assert_ne!(count_rows[0].game_dir(&base), claymorgue_rows[0].game_dir(&base));
    let _ = std::fs::remove_dir_all(&base);
}
