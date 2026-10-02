//! SQ-0962: **one disk is not one game**, and the launch path had been assuming
//! it was for everything that belonged to no multi-disk set.
//!
//! Point lanthorn at `treasures/The Lost Treasures of Infocom - Disk 1 - Beyond
//! Zork, Lurking Horror.dc42` and it started *Beyond Zork* — the format's own
//! tiebreak — with no way to reach the other game on the platter. Same for
//! `stories/InfocomMasterpieces.img`, which holds thirty-three.
//!
//! Nothing was missing underneath. `meta.disk_entry` threads a chosen story
//! through the picker, the launch-options dialog and the save key, and
//! `picker::dedupe_within_a_volume` exists *specifically* for a volume holding
//! several games (SQ-0878). The chooser was simply not reached:
//! `StorySource::of` asked `disk_set::members` and gave up on `None`, so "is
//! this a volume of a set?" was standing in for "is there a choice to make?".
//! A compilation pressed onto a single disc is a shelf too.
//!
//! # What must not move
//!
//! - **A disk with one game still opens it.** `Disk 5 - Zork Zero.dc42` holds
//!   exactly one story, and so does every single-title release floppy; a
//!   one-row browser in front of them would be a regression, not a fix.
//! - **A hybrid disc keeps both machines.** The fold that drops a build an
//!   earlier volume already offered is scoped to *volumes*, and a lone volume
//!   has no earlier one. *Masterpieces* presses Zork I as r88/840726 on the
//!   Macintosh side and again on the DOS side, and telling those apart is the
//!   whole subtlety of SQ-0878's machine key.
//!
//! # Specimens
//!
//! The DiskCopy volumes are **copied into a directory of their own** before
//! being asked, and that is the point of the case rather than hygiene: in
//! `treasures/` they are five volumes of one release (SQ-0961), which would
//! answer through the set rule and prove nothing about this one. A player who
//! downloaded a single disk has exactly the isolated file this builds.
//!
//! Every fixture lives in a gitignored directory, so each case skips vacuously
//! when its own is absent and carries a non-vacuity guard on the shape it needs.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A scratch directory unique to this process and `tag`.
fn scratch(tag: &str) -> PathBuf {
    app::scratch_dir(&format!("sq0962-{tag}"))
}

/// `src` copied into a directory holding nothing else, so no naming rule can
/// group it with anything. `None` when the fixture is absent.
fn isolated(src: &Path, tag: &str) -> Option<(PathBuf, PathBuf)> {
    if !src.exists() {
        return None;
    }
    let dir = scratch(tag);
    let dest = dir.join(src.file_name()?);
    std::fs::copy(src, &dest).expect("the fixture copies");
    Some((dir, dest))
}

/// How many stories `blorb` says are on this volume — the premise each case
/// needs, taken from outside the code under test.
fn stories_on(path: &Path) -> usize {
    let Ok(raw) = std::fs::read(path) else { return 0 };
    if blorb::medium::DiskImage::detect(&raw).is_none() {
        return 0;
    }
    blorb::medium::MountedDisk::mount(raw).map(|d| d.stories().len()).unwrap_or(0)
}

fn builds(rows: &[app::picker::StoryEntry]) -> Vec<(u16, String)> {
    rows.iter().filter_map(|e| Some((e.meta.release?, e.meta.serial.clone()?))).collect()
}

/// **The defect.** A volume that belongs to no set and holds several games
/// offers all of them.
///
/// FALSIFICATION: drop the `holds_several_games` fallback from
/// `StorySource::of` and both rows here report `None` — which is the launch
/// going straight into `MountedDisk::story()`'s tiebreak, *Beyond Zork* on the
/// DiskCopy disk and one of thirty-three on the Masterpieces image.
#[test]
fn a_lone_volume_of_several_games_offers_all_of_them() {
    let base = scratch("offers-base");
    let mut ran = 0;

    let dc42 = repo_root()
        .join("treasures/Mac/The Lost Treasures of Infocom - Disk 1 - Beyond Zork, Lurking Horror.dc42");
    if let Some((_dir, path)) = isolated(&dc42, "dc42-one") {
        ran += 1;
        // Premise: it really is a lone volume, and it really holds several.
        assert!(app::disk_set::members(&path).is_none(), "the copy must be in no set");
        assert!(stories_on(&path) >= 2, "the premise: this platter holds several stories");

        let source = app::picker::StorySource::of(&path, &app::data_roots::DataRoots::single(&base))
            .expect("a disk of games is a source of stories");
        let rows = source.scan(&app::data_roots::DataRoots::single(&base));
        let b = builds(&rows);
        assert!(rows.len() >= 2, "one row for a disk of two games: {:?}", b);
        // Beyond Zork r57/871221 is the tiebreak the launch used to take; The
        // Lurking Horror r203/870506 is the game that was unreachable.
        assert!(b.contains(&(57, "871221".into())), "no Beyond Zork: {b:?}");
        assert!(b.contains(&(203, "870506".into())), "no Lurking Horror: {b:?}");
    }

    let masterpieces = repo_root().join("stories/InfocomMasterpieces.img");
    if masterpieces.exists() {
        ran += 1;
        assert!(app::disk_set::members(&masterpieces).is_none(), "it belongs to no set");
        let source = app::picker::StorySource::of(&masterpieces, &app::data_roots::DataRoots::single(&base))
            .expect("thirty-three games are a source of stories");
        assert_eq!(source.scan(&app::data_roots::DataRoots::single(&base)).len(), 33, "the whole disc, not its tiebreak");
    }

    let _ = std::fs::remove_dir_all(&base);
    assert!(ran > 0 || !(dc42.exists() || masterpieces.exists()), "a fixture is present, none ran");
}

/// …and a disk with **one** game still opens it without a browser in the way.
///
/// `Disk 5 - Zork Zero.dc42` is the sharp case: same press, same naming, one
/// story. A fix that turned it into a one-row list would be a regression the
/// case above cannot see.
#[test]
fn a_lone_volume_of_one_game_still_opens_it() {
    let base = scratch("single-base");
    let mut ran = 0;

    let disk5 = repo_root().join("treasures/Mac/The Lost Treasures of Infocom - Disk 5 - Zork Zero.dc42");
    if let Some((_dir, path)) = isolated(&disk5, "dc42-five") {
        ran += 1;
        assert_eq!(stories_on(&path), 1, "the premise: this platter holds one story");
        assert!(
            app::picker::StorySource::of(&path, &app::data_roots::DataRoots::single(&base)).is_none(),
            "a one-game disk needs no chooser",
        );
    }

    for name in ["Zork I - The Great Underground Empire.adf", "Zork Zero Disk.image"] {
        let path = repo_root().join("stories").join(name);
        if !path.exists() {
            continue;
        }
        ran += 1;
        assert!(
            app::picker::StorySource::of(&path, &app::data_roots::DataRoots::single(&base)).is_none(),
            "{name}: a single-title floppy opens itself",
        );
    }

    let _ = std::fs::remove_dir_all(&base);
    // Same shape as the case above: a fixture that IS present must have run, but a
    // shelf that is absent skips vacuously. `stories/` and `treasures/` are
    // gitignored, so on CI none of the three exist and this case has nothing to say.
    let any = disk5.exists()
        || ["Zork I - The Great Underground Empire.adf", "Zork Zero Disk.image"]
            .iter()
            .any(|n| repo_root().join("stories").join(n).exists());
    assert!(ran > 0 || !any, "a fixture is present, none ran");
}

/// A **hybrid disc** keeps one row per machine for a build both sides carry.
///
/// *Classic Text Adventure Masterpieces* presses twenty-five builds twice over,
/// Macintosh and DOS, and the two are byte-identical for several of them — so
/// the machine, not the build, is what tells the rows apart. This is the case
/// that pins the cross-volume fold to volumes.
///
/// FALSIFICATION: run `dedupe_within_sets` unconditionally in
/// `StorySource::scan` and every DOS row here disappears into its Macintosh
/// twin, halving the disc.
#[test]
fn a_lone_hybrid_disc_keeps_one_row_per_machine() {
    let disc = repo_root()
        .join("masterpieces/Classic Text Adventure Masterpieces of Infocom (USA).bin");
    if !disc.exists() {
        return;
    }
    let base = scratch("hybrid-base");
    let source =
        app::picker::StorySource::of(&disc, &app::data_roots::DataRoots::single(&base)).expect("a hybrid compilation is a source");
    let rows = source.scan(&app::data_roots::DataRoots::single(&base));

    // Zork I release 88 / serial 840726 sits on both sides of this disc.
    let zork1: Vec<&app::picker::StoryEntry> = rows
        .iter()
        .filter(|e| e.meta.release == Some(88) && e.meta.serial.as_deref() == Some("840726"))
        .collect();
    assert_eq!(zork1.len(), 2, "one build, two machines: {:?}", zork1.iter().map(|e| &e.meta.disk_entry).collect::<Vec<_>>());
    assert_ne!(zork1[0].meta.disk_image, zork1[1].meta.disk_image, "the rows name one machine");
    assert_ne!(zork1[0].meta.disk_entry, zork1[1].meta.disk_entry);
    // …and the disc as a whole is not folded in half. Measured 2026-08-21.
    assert_eq!(rows.len(), 66, "the whole disc");

    let _ = std::fs::remove_dir_all(&base);
}
