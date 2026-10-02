//! SQ-0859: one browser row per GAME, measured on the real compilations.
//!
//! The defect this pins: the story browser listed a disk image once and opened
//! `MountedDisk::story()` — the format's own tiebreak, in practice the largest
//! file — so a compilation offered exactly one game and the rest were
//! unreachable however long you looked at the list. `INFOCOM6` opened
//! *Sherlock* and there was no way at all to reach *Leather Goddesses of
//! Phobos* beside it; the six Atari ST volumes and the DOS `floppy*.ima` set
//! were the same shape. About thirty games, one row each on paper, one row
//! per *disk* in practice.
//!
//! The rule now: [`app::picker::scan_stories`] mounts each image once and emits
//! one row per story on it, each carrying `meta.disk_entry` — the name the
//! volume stores that story under — which the launch hands back to
//! [`app::hints::load_mounted_story_from`]. A disk holding ONE story keeps the
//! old path exactly: no selector, no second row, nothing to notice. That is
//! most of the corpus and it is guarded first below.
//!
//! `stories/` is gitignored (commercial media), so every case skips vacuously
//! when its fixture is missing and every `ran > 0` guard is gated on
//! [`any_media_present`] — CI has none of it and must not fail on its absence.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use app::storage::{story_key_at, story_key_at_from};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Compilation volumes: one image, several games. Each is a skip when absent.
const COMPILATIONS: &[&str] = &[
    "Infocom Compilation 1 (19xx)(-).st",
    "Infocom Compilation 6 (19xx)(-).st",
    "Infocom Compilation 9 (19xx)(-).st",
    "floppy1.ima",
    "floppy2.ima",
    "Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 6 of 7).2mg",
];

/// Single-title release floppies: one image, one game, and **nothing about them
/// may move**.
const SINGLE_TITLE: &[&str] = &[
    "Zork I - The Great Underground Empire.adf",
    "Zork Zero - The Revenge of Megaboz.adf",
    "Journey - The Quest Begins.adf",
    "Arthur - The Quest for Excalibur.adf",
    "Zork Zero Disk.image",
];

fn any_media_present() -> bool {
    COMPILATIONS.iter().chain(SINGLE_TITLE.iter()).any(|f| stories_dir().join(f).exists())
}

/// A scratch storage base, unique per test process so parallel tests cannot
/// read each other's sidecars.
fn data_base(tag: &str) -> PathBuf {
    app::scratch_dir(&format!("sq0859-{tag}"))
}

/// Every row `scan_stories` produces for one image, by mounting the whole
/// directory once and keeping the rows whose path is that image.
fn rows_for(image: &Path, base: &Path) -> Vec<app::picker::StoryEntry> {
    app::picker::scan_stories(&stories_dir(), &app::data_roots::DataRoots::single(base))
        .into_iter()
        .filter(|e| e.path == image)
        .collect()
}

/// What the mount itself says is on the image — the answer the browser must
/// match, taken from `blorb` rather than from the code under test.
fn stories_on(image: &Path) -> Vec<(String, Vec<u8>)> {
    let Ok(raw) = std::fs::read(image) else { return Vec::new() };
    if blorb::medium::DiskImage::detect(&raw).is_none() {
        return Vec::new();
    }
    let Ok(disk) = blorb::medium::MountedDisk::mount(raw) else { return Vec::new() };
    disk.stories().into_iter().map(|s| (s.name, s.bytes)).collect()
}

fn header_release(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0x02], b[0x03]])
}

fn header_serial(b: &[u8]) -> String {
    b[0x12..0x18].iter().map(|c| char::from(c & 0x7f)).collect()
}

// ── Guard 1: a single-story image is exactly what it was ─────────────────────

/// **The guard that matters most, because it covers most disks.** A release
/// floppy carrying one game must produce one row, with no story selector on it,
/// and that row must be identical to the one the old path produced —
/// `resolve_entry`, which opens `MountedDisk::story()` and knows nothing about
/// any of this.
#[test]
fn a_single_story_floppy_is_one_unchanged_row() {
    let base = data_base("single");
    let mut ran = 0;
    for image in SINGLE_TITLE {
        let path = stories_dir().join(image);
        if !path.is_file() {
            continue;
        }
        let Some(old) = app::picker::resolve_entry(&path, &app::data_roots::DataRoots::single(&base)) else {
            continue; // not launchable on this corpus; nothing to compare
        };
        ran += 1;
        let rows = app::picker::resolve_entries(&path, &app::data_roots::DataRoots::single(&base));
        assert_eq!(rows.len(), 1, "{image}: a single-title floppy is one row");
        assert_eq!(
            rows[0].meta.disk_entry, None,
            "{image}: nothing to choose, so nothing is carried",
        );
        assert_eq!(rows[0], old, "{image}: the row must be byte-for-byte the old answer");
        // …and the key its saves live under is the same one as before, which is
        // the promise no change here may break.
        assert_eq!(rows[0].story_key(), story_key_at(&path), "{image}: save key moved");
    }
    let _ = std::fs::remove_dir_all(&base);
    assert!(ran > 0 || !any_media_present(), "media are present but no single-title floppy ran");
}

// ── Guard: every game on a compilation is offered ────────────────────────────

/// The defect, as a test. Every story the mount finds on a compilation must be
/// **reachable** in the browser's list.
///
/// Reachable, rather than "a row for this image", is the contract since SQ-0844
/// folded the duplicate builds a multi-disk release carries: `Infocom
/// Compilation 9` stores Hitchhiker's and Cutthroats as second copies of the
/// very builds `Compilation 1` and `Compilation 3` already offer — one IFID
/// each — so the list shows them once, off the earlier disk. What must never
/// happen is a game being absent, which is what SQ-0859 fixed, so the assertion
/// is on the IFID: every build on the disk is somewhere in the list.
///
/// FALSIFICATION (measured 2026-08-14, by making `scan_stories` call
/// `resolve_entry` per file again, as it did before SQ-0859):
///
/// ```text
/// thread 'disk_story_rows::every_game_on_a_compilation_is_its_own_row' panicked at
/// crates/app/tests/suites/disk_story_rows.rs:157:9:
/// assertion `left == right` failed: Infocom Compilation 1 (19xx)(-).st: the mount
/// finds 6 stories and the picker offers 1 row(s) — named [] of ["HITCHHI.T",
/// "PLANETF.T", "SEASTAL.T", "STARCRO.T", "STATION.T", "SUSPEND.T"]
///   left: 1
///  right: 6
/// ```
///
/// …and, in the same run, `leather_goddesses_is_reachable_on_infocom6` failed
/// with *"INFOCOM6 holds five games; the list offered 1 row(s)"* — the reported
/// symptom in one line.
#[test]
fn every_game_on_a_compilation_is_its_own_row() {
    let base = data_base("rows");
    let listed = app::picker::scan_stories(&stories_dir(), &app::data_roots::DataRoots::single(&base));
    let mut ran = 0;
    for image in COMPILATIONS {
        let path = stories_dir().join(image);
        let on_disk = stories_on(&path);
        if on_disk.len() < 2 {
            continue; // absent, or not a compilation on this corpus
        }
        ran += 1;
        let rows: Vec<&app::picker::StoryEntry> =
            listed.iter().filter(|e| e.path == path).collect();
        let offered: Vec<String> = rows.iter().filter_map(|e| e.meta.disk_entry.clone()).collect();
        for (name, bytes) in &on_disk {
            let ifid = app::ifid::compute_ifid(bytes);
            // Either this image's own row…
            if offered.contains(name) {
                continue;
            }
            // …or the same build off an earlier volume of the same release,
            // which is the only reason a row may be missing here.
            assert!(
                listed.iter().any(|e| e.meta.ifid == ifid),
                "{image}: {name} ({ifid}) is on the disk and nowhere in the list — \
                 offered {offered:?} of {:?}",
                on_disk.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            );
        }
        // And the rows this image does contribute are all genuinely its own.
        assert!(
            !rows.is_empty(),
            "{image}: a compilation contributed no rows at all",
        );
    }
    let _ = std::fs::remove_dir_all(&base);
    assert!(ran > 0 || !any_media_present(), "media are present but no compilation was scanned");
}

/// The reported case by name: *Leather Goddesses of Phobos* on `INFOCOM6`,
/// which the tiebreak never opened because *Sherlock* is the larger file — and
/// which is also the corpus's one story whose serial is not six ASCII digits
/// (`Blown!`, high-ASCII, SQ-0856), so it is the row most likely to fall out of
/// any identity path.
#[test]
fn leather_goddesses_is_reachable_on_infocom6() {
    let image =
        stories_dir().join("Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 6 of 7).2mg");
    if !image.is_file() {
        return; // gitignored corpus; nothing to say
    }
    let base = data_base("lgop");
    let rows = rows_for(&image, &base);

    // The tiebreak — what the browser used to offer, and all it used to offer.
    let tiebreak = app::picker::resolve_entry(&image, &app::data_roots::DataRoots::single(&base)).expect("the image mounts");
    assert!(
        rows.len() > 1,
        "INFOCOM6 holds five games; the list offered {} row(s)",
        rows.len()
    );

    let lgop = rows
        .iter()
        .find(|e| e.meta.disk_entry.as_deref() == Some("LEATHRGODDESSES"))
        .expect("LEATHRGODDESSES is on INFOCOM6 and must be a row of its own");
    assert_ne!(
        lgop.story_key(),
        tiebreak.story_key(),
        "the row must not be the tiebreak game wearing another name",
    );

    // Opening it opens IT: the bytes the launch path returns carry Leather
    // Goddesses' own header, not Sherlock's.
    let (loaded, disk_image) =
        app::hints::load_mounted_story_from(&image, Some("LEATHRGODDESSES")).expect("opens");
    let bytes = loaded.bytes();
    assert_eq!(disk_image, Some(blorb::medium::DiskImage::ProDos));
    assert_eq!(header_serial(bytes), "Blown!", "the high-ASCII serial reads as text");
    assert_eq!(
        lgop.meta.serial.as_deref(),
        Some(header_serial(bytes).as_str()),
        "the row's identity is the story's own",
    );
    assert_eq!(lgop.meta.release, Some(header_release(bytes)));

    // Its last-resort title is the DISK's name for it, not the box's: without
    // this, five rows off one image would every one of them read *Lost
    // Treasures of Infocom, The (1993)…(Disk 6 of 7)*.
    assert_eq!(lgop.title, "LEATHRGODDESSES", "titled by the disk when no table knows it");
    let _ = std::fs::remove_dir_all(&base);
}

// ── Every row opens its own game, and keeps its own saves ────────────────────

/// A row's identity columns must describe the story the row opens — not the
/// image, and not its disk-mate. Checked by opening every row the way the app
/// does and comparing the header that comes back.
#[test]
fn every_row_opens_the_story_its_columns_describe() {
    let base = data_base("open");
    let mut ran = 0;
    for image in COMPILATIONS {
        let path = stories_dir().join(image);
        if !path.is_file() {
            continue;
        }
        for row in rows_for(&path, &base) {
            ran += 1;
            let (loaded, disk_image) =
                app::hints::load_mounted_story_from(&path, row.meta.disk_entry.as_deref())
                    .unwrap_or_else(|e| panic!("{image}: {:?} will not open: {e}", row.meta.disk_entry));
            let bytes = loaded.bytes();
            assert_eq!(row.meta.release, Some(header_release(bytes)), "{image}: {}", row.title);
            // Read with bit 7 masked, the way every serial reader in the
            // workspace reads it (SQ-0856) — the Apple II press writes its
            // serial in high ASCII.
            assert_eq!(
                row.meta.serial.as_deref().map(str::to_string),
                Some(header_serial(bytes)),
                "{image}: {}",
                row.title,
            );
            assert_eq!(row.meta.disk_image, disk_image, "{image}: {}", row.title);
            assert_eq!(row.meta.story_bytes, bytes.len() as u64, "{image}: {}", row.title);
            // The TYPE column's other half: the Z-machine version is the story's
            // own, so a v3 and a v5 on one disk are not both labelled alike.
            assert_eq!(
                row.meta.version.as_deref(),
                Some(bytes[0].to_string()).as_deref(),
                "{image}: {}",
                row.title,
            );
        }
    }
    let _ = std::fs::remove_dir_all(&base);
    assert!(ran > 0 || !any_media_present(), "media are present but no row was opened");
}

/// **Saves, end to end** (SQ-0850 keys them; this pins that the new path keeps
/// its promise). Every row off one image resolves to its own game directory,
/// and the key the *launch* computes from `(path, disk_entry)` is the key the
/// *browser row* showed — the two halves that must agree or a game's saves
/// vanish the moment it is played.
#[test]
fn each_row_keeps_its_own_saves() {
    let base = data_base("saves");
    let mut ran = 0;
    for image in COMPILATIONS {
        let path = stories_dir().join(image);
        if !path.is_file() {
            continue;
        }
        let rows = rows_for(&path, &base);
        if rows.len() < 2 {
            continue;
        }
        ran += 1;
        let mut dirs: BTreeSet<PathBuf> = BTreeSet::new();
        for row in &rows {
            let dir = row.game_dir(&app::data_roots::DataRoots::single(&base));
            assert!(dirs.insert(dir.clone()), "{image}: two rows share {}", dir.display());
            // The launch does not have the row in hand — it has the path and the
            // selector — so it must arrive at the same key from those alone.
            assert_eq!(
                story_key_at_from(&path, row.meta.disk_entry.as_deref()),
                row.story_key(),
                "{image}: {} keys differently at launch than in the list",
                row.title,
            );
            // …and never at the image's own filename, which is what all of them
            // shared before SQ-0850.
            assert_ne!(
                row.story_key(),
                app::storage::story_key_for(app::storage::StoryOrigin {
                    path: &path,
                    entry: None,
                    build: None,
                }),
            );
        }
        assert_eq!(dirs.len(), rows.len(), "{image}: {} rows, {} directories", rows.len(), dirs.len());
    }
    let _ = std::fs::remove_dir_all(&base);
    assert!(ran > 0 || !any_media_present(), "media are present but no compilation was keyed");
}

/// Rows off one image must be distinguishable **on screen**, which means the
/// title column: five rows reading *Lost Treasures of Infocom (Disk 6 of 7)* is
/// a list that offers five games and shows one. Titles come from the
/// release+serial table where it knows the build, and from the disk's own name
/// where it does not.
#[test]
fn rows_off_one_image_carry_distinct_titles() {
    let base = data_base("titles");
    let mut ran = 0;
    for image in COMPILATIONS {
        let path = stories_dir().join(image);
        if !path.is_file() {
            continue;
        }
        let rows = rows_for(&path, &base);
        if rows.len() < 2 {
            continue;
        }
        ran += 1;
        let titles: BTreeSet<&str> = rows.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(
            titles.len(),
            rows.len(),
            "{image}: {} rows share titles — {:?}",
            rows.len(),
            rows.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(),
        );
        // The image's own stem is the one title none of them may take, since it
        // names the box rather than any game in it.
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        for row in &rows {
            assert_ne!(row.title, stem, "{image}: a row is titled after the box");
        }
    }
    let _ = std::fs::remove_dir_all(&base);
    assert!(ran > 0 || !any_media_present(), "media are present but no titles were compared");
}

/// The browser's whole list grows by exactly the games that were hidden: no
/// story is dropped, and the images that hold one game contribute one row each.
///
/// Counted in **builds**, not in rows, since SQ-0844: a disk contributes one row
/// per story it holds *minus* the builds an earlier volume of its own release
/// already offers. Every story on the disk still has to be somewhere.
#[test]
fn the_scan_gains_a_row_per_hidden_game_and_loses_none() {
    let dir = stories_dir();
    if !dir.is_dir() {
        return;
    }
    let base = data_base("scan");
    let listed = app::picker::scan_stories(&dir, &app::data_roots::DataRoots::single(&base));
    let mut ran = 0;
    for image in COMPILATIONS.iter().chain(SINGLE_TITLE.iter()) {
        let path = dir.join(image);
        if !path.is_file() {
            continue;
        }
        let on_disk = stories_on(&path);
        if on_disk.is_empty() {
            continue; // mountable but carrying nothing launchable
        }
        ran += 1;
        let rows = listed.iter().filter(|e| e.path == path).count();
        let folded = on_disk
            .iter()
            .filter(|(_, b)| {
                let ifid = app::ifid::compute_ifid(b);
                !listed.iter().any(|e| e.path == path && e.meta.ifid == ifid)
            })
            .count();
        assert_eq!(
            rows + folded,
            on_disk.len(),
            "{image}: {} stories on the disk, {rows} rows in the list, {folded} folded",
            on_disk.len(),
        );
        // Nothing may vanish: a folded build is offered by another volume.
        for (name, bytes) in &on_disk {
            let ifid = app::ifid::compute_ifid(bytes);
            assert!(
                listed.iter().any(|e| e.meta.ifid == ifid),
                "{image}: {name} ({ifid}) left the list entirely",
            );
        }
    }
    let _ = std::fs::remove_dir_all(&base);
    assert!(ran > 0 || !any_media_present(), "media are present but none were counted");
}
