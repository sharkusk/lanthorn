//! SQ-1414: the Commodore 64 *Mysterious Adventures* compilation disks open
//! through the one disk seam every front-end shares.
//!
//! `scott::c64::parse_c64_mysterious_prg` (the loader over raw PRG bytes) and
//! `blorb::medium`'s D64 directory walk (`MountedDisk::contents`) landed
//! separately; this pins the app-side wiring that joins them —
//! `app::hints::mounted_stories` and `app::hints::load_mounted_story_from`
//! offering the eleven program files `MountedDisk::stories` (Z-code/Glulx/
//! Blorb only, by that door's own design) never lists, each keyed with its
//! own saves — against the two real compilation disks and the one disk in
//! the corpus that must NOT contribute a row: `QUESTPR1.D64`'s `SHULK.DB` is
//! the US-format *Hulk*, a different Commodore 64 family entirely, and has
//! to be a named refusal rather than a crash.
//!
//! `stories/` is gitignored (commercial media), so every case skips
//! vacuously when its fixture is absent.

use std::path::PathBuf;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/scott-dialects/c64")
}

fn data_base(tag: &str) -> PathBuf {
    app::scratch_dir(&format!("sq1414-{tag}"))
}

/// `MYSTADV1.D64`'s six program files, in disk order (per
/// `crates/blorb/src/d64.rs`'s own `the_mysterious_adventures_disks_list_their_program_files`).
const MYSTADV1_GAMES: &[&str] =
    &["BATON", "TIME MACHINE", "ARROW I", "ARROW II", "PULSAR 7", "CIRCUS"];

/// `MYSTADV2.D64`'s five, likewise.
const MYSTADV2_GAMES: &[&str] =
    &["EXPERIMENT", "WIZARD OF AKYRZ", "PERSEUS", "INDIANS", "WAXWORKS"];

#[test]
fn mystadv1_lists_six_named_rows_in_disk_order_with_no_boot() {
    let path = stories_dir().join("MYSTADV1.D64");
    let Some((_, stories)) = app::hints::mounted_stories(&path) else {
        eprintln!("SKIP: stories/scott-dialects/c64/MYSTADV1.D64 absent (gitignored commercial fixture)");
        return;
    };
    let names: Vec<String> = stories.iter().map(|(s, _)| s.name.clone()).collect();
    assert_eq!(
        names, MYSTADV1_GAMES,
        "MYSTADV1.D64's rows must be exactly the six games, in disk order"
    );
    assert!(!names.iter().any(|n| n == "BOOT"), "the boot loader must never be offered as a game");
}

#[test]
fn mystadv2_lists_five_named_rows_in_disk_order_with_no_boot() {
    let path = stories_dir().join("MYSTADV2.D64");
    let Some((_, stories)) = app::hints::mounted_stories(&path) else {
        eprintln!("SKIP: stories/scott-dialects/c64/MYSTADV2.D64 absent (gitignored commercial fixture)");
        return;
    };
    let names: Vec<String> = stories.iter().map(|(s, _)| s.name.clone()).collect();
    assert_eq!(
        names, MYSTADV2_GAMES,
        "MYSTADV2.D64's rows must be exactly the five games, in disk order"
    );
    assert!(!names.iter().any(|n| n == "BOOT"), "the boot loader must never be offered as a game");
}

/// The picker's own multi-story door lists the same six rows — the existing
/// disk-set picker, with no new UI needed for a Scott-only compilation.
#[test]
fn the_picker_lists_the_same_six_rows_as_the_disk_menu() {
    let path = stories_dir().join("MYSTADV1.D64");
    if !path.is_file() {
        eprintln!("SKIP: stories/scott-dialects/c64/MYSTADV1.D64 absent (gitignored commercial fixture)");
        return;
    }
    let base = data_base("picker-rows");
    let rows = app::picker::resolve_entries(&path, &base);
    assert_eq!(rows.len(), 6, "MYSTADV1.D64 must offer six rows: {:?}", rows.iter().map(|r| &r.meta.disk_entry).collect::<Vec<_>>());
    for row in &rows {
        assert_eq!(row.meta.engine, app::picker::Engine::Scott, "{}: not classified Scott", row.title);
        assert!(
            MYSTADV1_GAMES.contains(&row.meta.disk_entry.as_deref().unwrap_or_default()),
            "unexpected entry: {:?}",
            row.meta.disk_entry
        );
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// `load_mounted_story_from(path, Some("BATON"))` boots to *The Golden
/// Baton*'s first room — Family B's opening SPOOKY Forest, item-for-item the
/// same as `scott-cli`'s own output on this exact disk and entry.
#[test]
fn baton_boots_to_the_golden_batons_first_room() {
    let path = stories_dir().join("MYSTADV1.D64");
    if !path.is_file() {
        eprintln!("SKIP: stories/scott-dialects/c64/MYSTADV1.D64 absent (gitignored commercial fixture)");
        return;
    }
    let (loaded, disk_image) =
        app::hints::load_mounted_story_from(&path, Some("BATON")).expect("BATON opens");
    let app::hints::LoadedStory::Scott(bytes) = loaded else {
        panic!("BATON resolved to a non-Scott engine: {loaded:?}");
    };
    assert_eq!(disk_image, Some(blorb::medium::DiskImage::CommodoreD64));

    let db = scott::Database::parse(&bytes).expect("a valid Scott database");
    assert!(db.mysterious, "the Mysterious Adventures series options must be set");
    let vm =
        scott::Vm::new_full(db, false, scott::Vm::DEFAULT_RNG_SEED, scott::Options::default());
    let block = vm.room_block();
    assert!(
        block.contains("SPOOKY Forest"),
        "the opening room must be the Golden Baton's dense SPOOKY Forest: {block}"
    );
    assert!(block.contains("Old Cloak"), "the opening room carries the old cloak: {block}");

    // The app's own Scott adapter accepts exactly what the disk path handed
    // it — the same cross-check `scott_zip_open.rs` makes for the zip door.
    app::scott_session::ScottSession::new(bytes, None)
        .expect("ScottSession accepts the bytes the disk mount resolved");
}

/// Two games off two different disks must never share a save directory.
/// `story_key_for` keys a disk-mounted Scott entry on its CBM name (no
/// Z-machine header to build a `DiskBuild` from), and the eleven names across
/// both Mysterious disks are all distinct, so every key here must be too.
#[test]
fn the_two_disks_rows_have_distinct_save_keys() {
    let path1 = stories_dir().join("MYSTADV1.D64");
    let path2 = stories_dir().join("MYSTADV2.D64");
    if !path1.is_file() || !path2.is_file() {
        eprintln!("SKIP: stories/scott-dialects/c64/MYSTADV{{1,2}}.D64 absent (gitignored commercial fixture)");
        return;
    }
    let base = data_base("keys");
    let rows1 = app::picker::resolve_entries(&path1, &base);
    let rows2 = app::picker::resolve_entries(&path2, &base);
    assert_eq!(rows1.len(), 6);
    assert_eq!(rows2.len(), 5);

    let mut keys: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut dirs: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
    for row in rows1.iter().chain(rows2.iter()) {
        assert!(
            keys.insert(row.story_key()),
            "duplicate save key {:?} for {:?}",
            row.story_key(),
            row.meta.disk_entry
        );
        assert!(
            dirs.insert(row.game_dir(&base)),
            "duplicate save directory for {:?}",
            row.meta.disk_entry
        );
        // Launch and list must agree, exactly as `disk_story_rows.rs` pins for
        // the Infocom compilations.
        assert_eq!(
            app::storage::story_key_at_from(&row.path, row.meta.disk_entry.as_deref()),
            row.story_key(),
            "{:?} keys differently at launch than in the list",
            row.meta.disk_entry,
        );
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// `QUESTPR1.D64` carries the US-format *Hulk* (`SHULK.DB`), a DIFFERENT
/// Commodore 64 family this loader does not read — `looks_like_scott_bytes`
/// must reject it, so it contributes no Scott row, and neither the mount nor
/// the picker may panic reaching that conclusion.
#[test]
fn questpr1_yields_no_scott_rows_and_no_panic() {
    let path = stories_dir().join("QUESTPR1.D64");
    if !path.is_file() {
        eprintln!("SKIP: stories/scott-dialects/c64/QUESTPR1.D64 absent (gitignored commercial fixture)");
        return;
    }

    // The raw sniff, directly: SHULK.DB (the US Hulk) must not pass it, or a
    // Scott row would follow from `mounted_stories` alone.
    let raw = std::fs::read(&path).expect("QUESTPR1.D64 reads");
    let disk = blorb::medium::MountedDisk::mount(raw).expect("QUESTPR1.D64 mounts");
    let shulk = disk.read_named("SHULK.DB").expect("SHULK.DB is on the disk");
    assert!(
        !scott::looks_like_scott_bytes(&shulk),
        "SHULK.DB is the US-format Hulk, a different C64 family — must not sniff as Mysterious"
    );

    // No panic scanning it at either layer, and no Scott row from either.
    if let Some((_, stories)) = app::hints::mounted_stories(&path) {
        for (story, _) in &stories {
            assert_ne!(story.name, "SHULK.DB", "SHULK.DB must never be offered as a game");
        }
    }
    let base = data_base("questpr1");
    let rows = app::picker::resolve_entries(&path, &base);
    for row in &rows {
        assert_ne!(
            row.meta.engine,
            app::picker::Engine::Scott,
            "QUESTPR1.D64 must not offer a Scott row: {:?}",
            row.meta.disk_entry
        );
    }
    let _ = std::fs::remove_dir_all(&base);
}
